//! Checking invariants and operation contracts against instance models.
//!
//! Invariants are evaluated on every instance of their context class. Pre- and
//! postconditions are checked in two ways:
//!
//! * at every operation call made while evaluating (invariants, bodies, derivations, …):
//!   failed conditions are reported for that call;
//! * for every object, by calling the operation with every combination of arguments taken
//!   from finite domains (Booleans, enumeration literals, the objects of the model). Calls
//!   whose preconditions do not hold are skipped; for the others the body is evaluated and
//!   the postconditions are checked. Operations with parameters of other types (numbers,
//!   strings, collections) are only checked where they are called.

use std::collections::{BTreeMap, HashSet};

use oxy_ecore::{DataTypeKind, Metamodel, Model, ObjectId, OperationId, Parameter};

use crate::env::Environment;
use crate::eval::{ContractCheck, Evaluator, Record};
use crate::syntax::ast::ConstraintKind;
use crate::types::Type;
use crate::value::Value;

/// At most this many argument combinations are checked per object and operation.
pub const MAX_CALLS_PER_OBJECT: usize = 1000;

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Satisfied,
    Violated,
    /// The constraint evaluated to `null` or `invalid`, or to a non-Boolean.
    Error(String),
}

/// The status of a constraint that evaluated to `v`.
pub(crate) fn status_of(v: Value, mm: &Metamodel, model: &Model) -> Status {
    match v {
        Value::Boolean(true) => Status::Satisfied,
        Value::Boolean(false) => Status::Violated,
        Value::Invalid(m) => Status::Error(format!("evaluated to invalid: {m}")),
        Value::Null => Status::Error("evaluated to null".into()),
        other => Status::Error(format!("evaluated to a non-Boolean value: {}", other.render(mm, model))),
    }
}

/// The outcome of evaluating one invariant on one object.
#[derive(Clone, Debug, PartialEq)]
pub struct ConstraintResult {
    /// Index into [`Environment::invariants`].
    pub invariant: usize,
    pub object: ObjectId,
    pub status: Status,
    /// The evaluated constraint message, for violated constraints that declare one.
    pub message: Option<String>,
}

/// An operation whose postconditions could not be checked for every object.
#[derive(Clone, Debug, PartialEq)]
pub struct UncheckedOperation {
    pub operation: OperationId,
    pub reason: String,
}

/// Everything [`validate_model`] found.
#[derive(Clone, Debug, Default)]
pub struct Validation {
    pub invariants: Vec<ConstraintResult>,
    /// Pre- and postcondition outcomes, without duplicates (the same condition failing for
    /// the same call is reported once).
    pub contracts: Vec<ContractCheck>,
    pub unchecked: Vec<UncheckedOperation>,
}

/// Evaluates every invariant on every instance of its context class.
pub fn validate(mm: &Metamodel, model: &Model, env: &Environment) -> Vec<ConstraintResult> {
    let all: Vec<ObjectId> = model.object_ids().collect();
    validate_objects(mm, model, env, &all)
}

/// Evaluates the applicable invariants on the given objects.
pub fn validate_objects(
    mm: &Metamodel,
    model: &Model,
    env: &Environment,
    objects: &[ObjectId],
) -> Vec<ConstraintResult> {
    validate_model(mm, model, env, objects, false).invariants
}

/// Evaluates the invariants on the given objects and, when `contracts` is set, checks the
/// operation contracts of these objects (see the module documentation).
pub fn validate_model(
    mm: &Metamodel,
    model: &Model,
    env: &Environment,
    objects: &[ObjectId],
    contracts: bool,
) -> Validation {
    let mut ev = Evaluator::new(mm, model, env);
    ev.set_check_contracts(contracts);
    let mut out = Validation::default();
    // `allInstances()` ranges over the objects of the object's resource, as in Eclipse OCL,
    // where each model is validated on its own (its extent is its resource's contents).
    for (index, inv) in env.invariants().iter().enumerate() {
        for &object in objects {
            if !mm.conforms_to(model.class_of(object), inv.context) {
                continue;
            }
            ev.set_extent(Some(vec![model.object(object).resource]));
            let self_value = Value::Object(object);
            let vars: Vec<(&str, Value)> =
                inv.self_name.as_deref().map(|n| (n, self_value.clone())).into_iter().collect();
            let value = ev.eval_body(&inv.body, self_value.clone(), &vars);
            let status = status_of(value, mm, model);
            let message = match (&status, &inv.message) {
                (Status::Violated, Some(m)) => {
                    let body = crate::env::Body { expr: m.clone(), ..inv.body.clone() };
                    Some(ev.eval_body(&body, self_value, &vars).to_ocl_string(mm, model))
                }
                _ => None,
            };
            out.invariants.push(ConstraintResult { invariant: index, object, status, message });
        }
    }
    ev.set_extent(None);
    if contracts {
        check_operations(&mut ev, mm, model, env, objects, &mut out.unchecked);
        let mut seen = HashSet::new();
        for c in ev.take_contract_checks() {
            let key = (c.constraint, c.object, c.args.iter().map(Value::key).collect::<Vec<_>>());
            if seen.insert(key) {
                out.contracts.push(c);
            }
        }
    }
    out
}

/// The values a parameter can take, if there are finitely many.
fn domain(mm: &Metamodel, model: &Model, p: &Parameter) -> Option<Vec<Value>> {
    if p.multiplicity.is_many() {
        return None;
    }
    let t = p.ty?;
    if mm.is_class(t) {
        Some(model.instances_of(mm, t).into_iter().map(Value::Object).collect())
    } else if mm.is_enum(t) {
        Some((0..mm.enum_literals(t).len() as u32).map(|i| Value::Enum(t, i)).collect())
    } else if mm.data_type_kind(t) == Some(DataTypeKind::Boolean) {
        Some(vec![Value::Boolean(true), Value::Boolean(false)])
    } else {
        None
    }
}

/// Calls every operation that has postconditions on every applicable object, with every
/// combination of arguments, and checks its contracts. The outcomes are recorded in `ev`.
fn check_operations(
    ev: &mut Evaluator,
    mm: &Metamodel,
    model: &Model,
    env: &Environment,
    objects: &[ObjectId],
    unchecked: &mut Vec<UncheckedOperation>,
) {
    let mut by_operation: BTreeMap<OperationId, Vec<usize>> = BTreeMap::new();
    for (i, oc) in env.operation_constraints().iter().enumerate() {
        by_operation.entry(oc.operation_id).or_default().push(i);
    }
    for (op, constraints) in by_operation {
        let has_post = constraints.iter().any(|i| env.operation_constraints()[*i].kind == ConstraintKind::Post);
        if !has_post {
            continue; // Preconditions alone are checked where the operation is called.
        }
        let operation = mm.operation(op);
        let mut domains = Vec::new();
        let mut not_enumerable = None;
        for p in &operation.params {
            match domain(mm, model, p) {
                Some(d) => domains.push(d),
                None => {
                    let ty = Type::of_typed(mm, p.ty, &p.multiplicity);
                    not_enumerable =
                        Some(format!("parameter '{}' of type {} cannot be enumerated", p.name, ty.display(mm)));
                    break;
                }
            }
        }
        if let Some(reason) = not_enumerable {
            unchecked.push(UncheckedOperation {
                operation: op,
                reason: format!("{reason}; its postconditions are checked where it is called"),
            });
            continue;
        }
        let names: Vec<&str> = operation.params.iter().map(|p| p.name.as_str()).collect();
        let mut truncated = false;
        let mut missing_body = false;
        for &object in objects {
            if !mm.conforms_to(model.class_of(object), operation.owner) {
                continue;
            }
            for (calls, args) in combinations(&domains).enumerate() {
                if calls == MAX_CALLS_PER_OBJECT {
                    truncated = true;
                    break;
                }
                let vars: Vec<(&str, Value)> = names.iter().copied().zip(args.iter().cloned()).collect();
                let pre = ev.check_conditions(&constraints, ConstraintKind::Pre, object, &vars, None, Record::Errors);
                if pre.iter().any(|(_, s)| *s != Status::Satisfied) {
                    continue; // Not a valid call (or the precondition failed to evaluate).
                }
                let Some(result) = ev.operation_result(object, op, &args) else {
                    missing_body = true;
                    break;
                };
                ev.check_conditions(&constraints, ConstraintKind::Post, object, &vars, Some(&result), Record::All);
            }
        }
        if missing_body {
            unchecked.push(UncheckedOperation {
                operation: op,
                reason: "it has no OCL body, so its postconditions cannot be checked".into(),
            });
        } else if truncated {
            unchecked.push(UncheckedOperation {
                operation: op,
                reason: format!("only the first {MAX_CALLS_PER_OBJECT} argument combinations per object were checked"),
            });
        }
    }
}

/// All combinations of one value from each domain (one empty combination for no domains).
fn combinations(domains: &[Vec<Value>]) -> impl Iterator<Item = Vec<Value>> + '_ {
    let mut indices = vec![0usize; domains.len()];
    let mut done = domains.iter().any(Vec::is_empty);
    std::iter::from_fn(move || {
        if done {
            return None;
        }
        let current: Vec<Value> = indices.iter().zip(domains).map(|(i, d)| d[*i].clone()).collect();
        // Advance the odometer.
        let mut pos = domains.len();
        loop {
            if pos == 0 {
                done = true;
                break;
            }
            pos -= 1;
            indices[pos] += 1;
            if indices[pos] < domains[pos].len() {
                break;
            }
            indices[pos] = 0;
        }
        Some(current)
    })
}
