//! Tree-walking evaluation of OCL expressions over an instance model.
//!
//! Evaluation never fails: errors produce `invalid` values carrying an explanation, which
//! propagate according to OCL's strictness rules (with the usual exceptions: Boolean
//! operators, `oclIsInvalid()`, `oclIsUndefined()`).

use std::cmp::Ordering;
use std::collections::HashSet;
use std::rc::Rc;

use oxy_ecore::{Metamodel, Model, ModelValue, ObjectId, OperationId, PackageId, default_value};

use crate::env::{Body, Environment, lookup_classifier, resolve_type};
use crate::syntax::ast::*;
use crate::types::{CollectionKind, Type, library_type};
use crate::validate::{Status, status_of};
use crate::value::{Collection, Value};

/// Maximum nesting of operation/definition calls before evaluation is aborted.
const MAX_DEPTH: u32 = 256;

struct Frame {
    vars: Vec<(Rc<str>, Value)>,
    /// The current element of an implicit iterator (or `self` in the outermost frame).
    implicit: Option<Value>,
}

/// A pre- or postcondition evaluated for an operation call.
#[derive(Clone, Debug)]
pub struct ContractCheck {
    /// Index into [`Environment::operation_constraints`].
    pub constraint: usize,
    /// The object the operation was called on.
    pub object: ObjectId,
    pub args: Vec<Value>,
    pub status: Status,
    /// The evaluated constraint message, for violated constraints that declare one.
    pub message: Option<String>,
}

/// Which outcomes of evaluating pre- and postconditions are recorded.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Record {
    /// Violations and errors (operation calls during ordinary evaluation).
    Failures,
    /// Errors only (preconditions that select the calls to check).
    Errors,
    /// Everything, including satisfied conditions.
    All,
}

pub struct Evaluator<'a> {
    mm: &'a Metamodel,
    model: &'a Model,
    env: &'a Environment,
    frames: Vec<Frame>,
    depth: u32,
    /// Package of the body being evaluated, for resolving unqualified type names.
    package: Option<PackageId>,
    /// Whether operation calls evaluate the operation's pre- and postconditions.
    check_contracts: bool,
    contract_checks: Vec<ContractCheck>,
}

macro_rules! strict {
    ($v:expr) => {{
        let v = $v;
        if v.is_invalid() {
            return v;
        }
        v
    }};
}

impl<'a> Evaluator<'a> {
    pub fn new(mm: &'a Metamodel, model: &'a Model, env: &'a Environment) -> Self {
        Evaluator {
            mm,
            model,
            env,
            frames: vec![Frame { vars: Vec::new(), implicit: None }],
            depth: 0,
            package: None,
            check_contracts: true,
            contract_checks: Vec::new(),
        }
    }

    /// Turns the evaluation of pre- and postconditions at operation calls on or off (it is on
    /// by default).
    pub fn set_check_contracts(&mut self, on: bool) {
        self.check_contracts = on;
    }

    /// The pre- and postconditions that failed at operation calls since the last call (and
    /// the outcomes recorded by [`Evaluator::check_conditions`]).
    pub fn take_contract_checks(&mut self) -> Vec<ContractCheck> {
        std::mem::take(&mut self.contract_checks)
    }

    /// Evaluates the conditions of kind `kind` among `constraints` (indices into
    /// [`Environment::operation_constraints`]) for a call on `object` with the parameters
    /// bound in `vars`; `result` is bound for postconditions. Outcomes are recorded as
    /// selected by `record` and returned.
    pub fn check_conditions(
        &mut self,
        constraints: &[usize],
        kind: ConstraintKind,
        object: ObjectId,
        vars: &[(&str, Value)],
        result: Option<&Value>,
        record: Record,
    ) -> Vec<(usize, Status)> {
        let mut out = Vec::new();
        for &index in constraints {
            let oc = &self.env.operation_constraints()[index];
            if oc.kind != kind {
                continue;
            }
            let mut bound: Vec<(&str, Value)> = vars.to_vec();
            if let Some(r) = result {
                bound.push(("result", r.clone()));
            }
            let body = oc.body.clone();
            let value = self.eval_body(&body, Value::Object(object), &bound);
            let status = status_of(value, self.mm, self.model);
            let keep = match record {
                Record::All => true,
                Record::Failures => status != Status::Satisfied,
                Record::Errors => matches!(status, Status::Error(_)),
            };
            if keep {
                let message = match (&status, &oc.message) {
                    (Status::Violated, Some(m)) => {
                        let body = Body { expr: m.clone(), ..oc.body.clone() };
                        Some(self.eval_body(&body, Value::Object(object), &bound).to_ocl_string(self.mm, self.model))
                    }
                    _ => None,
                };
                let args = vars.iter().map(|(_, v)| v.clone()).collect();
                self.contract_checks.push(ContractCheck {
                    constraint: index,
                    object,
                    args,
                    status: status.clone(),
                    message,
                });
            }
            out.push((index, status));
        }
        out
    }

    /// Evaluates the body of operation `op` (or of the operation overriding it in the class
    /// of `object`) without checking its contracts. `None` if no OCL body is defined.
    pub fn operation_result(&mut self, object: ObjectId, op: OperationId, args: &[Value]) -> Option<Value> {
        let mm = self.mm;
        let operation = mm.operation(op);
        let class = self.model.class_of(object);
        let body = mm
            .lookup_operations(class, &operation.name)
            .into_iter()
            .filter(|o| mm.operation(*o).params.len() == args.len())
            .find_map(|o| self.env.operation_body(o))?
            .clone();
        let names: Vec<&str> = operation.params.iter().map(|p| p.name.as_str()).collect();
        let vars: Vec<(&str, Value)> = names.into_iter().zip(args.iter().cloned()).collect();
        Some(self.eval_body(&body, Value::Object(object), &vars))
    }

    /// Evaluates `expr` with `self` bound to `self_value` (if any) and additional variables.
    pub fn evaluate(&mut self, expr: &Expr, self_value: Option<Value>, vars: &[(&str, Value)]) -> Value {
        let mut frame = Frame { vars: Vec::new(), implicit: None };
        if let Some(s) = self_value {
            frame.vars.push(("self".into(), s.clone()));
            frame.implicit = Some(s);
        }
        for (n, v) in vars {
            frame.vars.push(((*n).into(), v.clone()));
        }
        let saved = std::mem::replace(&mut self.frames, vec![frame]);
        let v = self.eval(expr);
        self.frames = saved;
        v
    }

    /// Evaluates a registered body (invariant, definition, operation body, derivation).
    pub fn eval_body(&mut self, body: &Body, self_value: Value, vars: &[(&str, Value)]) -> Value {
        if self.depth >= MAX_DEPTH {
            return Value::invalid("maximum call depth exceeded (infinite recursion?)");
        }
        self.depth += 1;
        let saved = std::mem::replace(&mut self.package, body.package);
        let v = self.evaluate(&body.expr, Some(self_value), vars);
        self.package = saved;
        self.depth -= 1;
        v
    }

    fn lookup_var(&self, name: &str) -> Option<Value> {
        self.frames.iter().rev().find_map(|f| f.vars.iter().rev().find(|(n, _)| &**n == name).map(|(_, v)| v.clone()))
    }

    fn with_frame<T>(&mut self, frame: Frame, f: impl FnOnce(&mut Self) -> T) -> T {
        self.frames.push(frame);
        let r = f(self);
        self.frames.pop();
        r
    }

    pub fn eval(&mut self, e: &Expr) -> Value {
        match &e.kind {
            ExprKind::Bool(b) => Value::Boolean(*b),
            ExprKind::Int(i) => Value::Integer(*i),
            ExprKind::Real(r) => Value::Real(*r),
            ExprKind::Str(s) => Value::string(s),
            ExprKind::Null => Value::Null,
            ExprKind::Invalid => Value::invalid("invalid literal"),
            ExprKind::Unlimited => Value::Unlimited,
            ExprKind::Error => Value::invalid("the expression contains syntax errors"),
            ExprKind::SelfRef => self.lookup_var("self").unwrap_or_else(|| Value::invalid("'self' is not defined")),
            ExprKind::Name { path, .. } => self.name(path),
            ExprKind::TypeLit(te) => {
                let mut diags = Vec::new();
                let t = resolve_type(self.mm, self.package, te, &mut diags);
                if diags.is_empty() { Value::Type(Rc::new(t)) } else { Value::invalid(diags[0].message.clone()) }
            }
            ExprKind::Property { source, name, safe, .. } => {
                let src = self.eval(source);
                if *safe {
                    match &src {
                        Value::Null => return Value::Null,
                        Value::Collection(c) => {
                            let kept = c.items.iter().filter(|v| !v.is_null()).cloned().collect();
                            return self.navigate(&Value::collection(c.kind, kept), &name.name);
                        }
                        _ => {}
                    }
                }
                self.navigate(&src, &name.name)
            }
            ExprKind::Call { source: Some(source), name, args, arrow, safe, .. } => {
                let src = self.eval(source);
                let args: Vec<Value> = args.iter().map(|a| self.eval(a)).collect();
                if *arrow {
                    let items = match to_collection(src) {
                        Ok(c) => c,
                        Err(inv) => return inv,
                    };
                    let items = if *safe { items.filter_nulls() } else { items };
                    return self.collection_op(items, &name.name, &args);
                }
                if *safe && src.is_null() {
                    return Value::Null;
                }
                self.call(src, &name.name, &args)
            }
            ExprKind::Call { source: None, name, args, .. } => {
                let args: Vec<Value> = args.iter().map(|a| self.eval(a)).collect();
                let implicit: Vec<Value> = self.frames.iter().rev().filter_map(|f| f.implicit.clone()).collect();
                for v in &implicit {
                    if let Value::Object(o) = v
                        && self.has_user_op(*o, &name.name, args.len())
                    {
                        return self.call(v.clone(), &name.name, &args);
                    }
                }
                // Library operations apply to the innermost implicit source.
                match implicit.first() {
                    Some(s) => self.call(s.clone(), &name.name, &args),
                    None => Value::invalid(format!("unknown operation '{}'", name.name)),
                }
            }
            ExprKind::Iterator { source, name, iterators, body, safe } => {
                let items = match to_collection(self.eval(source)) {
                    Ok(c) => c,
                    Err(inv) => return inv,
                };
                let items = if *safe { items.filter_nulls() } else { items };
                self.iterator(items, &name.name, iterators, body)
            }
            ExprKind::Iterate { source, iterators, accumulator, body, safe } => {
                let src = self.eval(source);
                let items = match to_collection(src) {
                    Ok(c) => c,
                    Err(inv) => return inv,
                };
                let items = if *safe { items.filter_nulls() } else { items };
                let mut acc = match &accumulator.init {
                    Some(i) => self.eval(i),
                    None => Value::Null,
                };
                let acc_name: Rc<str> = accumulator.name.name.as_str().into();
                for item in items {
                    let mut frame = Frame { vars: Vec::new(), implicit: None };
                    match iterators.first() {
                        Some(v) => frame.vars.push((v.name.name.as_str().into(), item)),
                        None => frame.implicit = Some(item),
                    }
                    frame.vars.push((acc_name.clone(), acc));
                    acc = self.with_frame(frame, |ev| ev.eval(body));
                }
                acc
            }
            ExprKind::Binary { op, lhs, rhs } => self.binary(*op, lhs, rhs),
            ExprKind::Unary { op, operand } => {
                let v = strict!(self.eval(operand));
                match (op, v) {
                    (UnOp::Not, Value::Boolean(b)) => Value::Boolean(!b),
                    (UnOp::Neg, Value::Integer(i)) => {
                        i.checked_neg().map_or_else(|| Value::invalid("integer overflow"), Value::Integer)
                    }
                    (UnOp::Neg, Value::Real(r)) => Value::Real(-r),
                    (UnOp::Not, Value::Null) | (UnOp::Neg, Value::Null) => Value::invalid("operand is null"),
                    (op, v) => Value::invalid(format!(
                        "'{}' is not applicable to {v}",
                        if *op == UnOp::Not { "not" } else { "-" }
                    )),
                }
            }
            ExprKind::If { cond, then_branch, else_branch } => match self.eval(cond) {
                Value::Boolean(true) => self.eval(then_branch),
                Value::Boolean(false) => self.eval(else_branch),
                Value::Invalid(m) => Value::Invalid(m),
                Value::Null => Value::invalid("'if' condition is null"),
                other => Value::invalid(format!("'if' condition is not a Boolean: {other}")),
            },
            ExprKind::Let { vars, body } => {
                self.frames.push(Frame { vars: Vec::new(), implicit: None });
                for v in vars {
                    let value = v.init.as_ref().map_or(Value::Null, |i| self.eval(i));
                    self.frames.last_mut().unwrap().vars.push((v.name.name.as_str().into(), value));
                }
                let r = self.eval(body);
                self.frames.pop();
                r
            }
            ExprKind::CollectionLit { kind, items, .. } => {
                let mut out = Vec::new();
                for item in items {
                    match item {
                        CollectionItem::Single(x) => out.push(strict!(self.eval(x))),
                        CollectionItem::Range(a, b) => {
                            let (lo, hi) = match (self.eval(a), self.eval(b)) {
                                (Value::Integer(lo), Value::Integer(hi)) => (lo, hi),
                                (Value::Invalid(m), _) | (_, Value::Invalid(m)) => return Value::Invalid(m),
                                _ => return Value::invalid("range bounds must be integers"),
                            };
                            if hi.saturating_sub(lo) > 10_000_000 {
                                return Value::invalid("range is too large");
                            }
                            out.extend((lo..=hi).map(Value::Integer));
                        }
                    }
                }
                Value::collection(*kind, out)
            }
            ExprKind::TupleLit(parts) => {
                let mut out = Vec::new();
                for p in parts {
                    let v = p.init.as_ref().map_or(Value::Null, |i| self.eval(i));
                    out.push((Rc::from(p.name.name.as_str()), strict!(v)));
                }
                Value::tuple(out)
            }
        }
    }

    // ---------------------------------------------------------------- names & properties

    fn name(&mut self, path: &[Ident]) -> Value {
        let mm = self.mm;
        if let [ident] = path {
            let name = ident.name.as_str();
            if let Some(v) = self.lookup_var(name) {
                return v;
            }
            let implicit: Vec<Value> = self.frames.iter().rev().filter_map(|f| f.implicit.clone()).collect();
            for v in &implicit {
                if self.has_property(v, name) {
                    return self.navigate(v, name);
                }
            }
            if let Some(t) = library_type(name) {
                return Value::Type(Rc::new(t));
            }
            if let Ok(c) = lookup_classifier(mm, self.package, &[name]) {
                return Value::Type(Rc::new(Type::of_classifier(mm, c)));
            }
            return Value::invalid(format!("unknown name '{name}'"));
        }
        let names: Vec<&str> = path.iter().map(|i| i.name.as_str()).collect();
        let (last, head) = names.split_last().unwrap();
        if let Ok(c) = lookup_classifier(mm, self.package, head)
            && mm.is_enum(c)
        {
            return match mm.enum_literal_index(c, last) {
                Some(i) => Value::Enum(c, i as u32),
                None => Value::invalid(format!("enumeration '{}' has no literal '{last}'", mm.classifier(c).name)),
            };
        }
        match lookup_classifier(mm, self.package, &names) {
            Ok(c) => Value::Type(Rc::new(Type::of_classifier(mm, c))),
            Err(e) => Value::invalid(e),
        }
    }

    fn has_property(&self, v: &Value, name: &str) -> bool {
        match v {
            Value::Object(o) => {
                let class = self.model.class_of(*o);
                self.mm.lookup_feature(class, name).is_some()
                    || self.env.find_def_attribute(self.mm, class, name).is_some()
            }
            Value::Tuple(parts) => parts.iter().any(|(n, _)| &**n == name),
            _ => false,
        }
    }

    fn has_user_op(&self, o: ObjectId, name: &str, arity: usize) -> bool {
        let class = self.model.class_of(o);
        self.mm.lookup_operations(class, name).iter().any(|op| self.mm.operation(*op).params.len() == arity)
            || self.env.find_def_operations(self.mm, class, name).iter().any(|d| d.params.len() == arity)
    }

    /// `source.name`, including implicit collect over collections.
    pub fn navigate(&mut self, src: &Value, name: &str) -> Value {
        match src {
            Value::Invalid(_) => src.clone(),
            Value::Null => Value::invalid(format!("cannot navigate '{name}' from null")),
            Value::Object(o) => self.object_property(*o, name),
            Value::Tuple(parts) => parts
                .iter()
                .find(|(n, _)| &**n == name)
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| Value::invalid(format!("tuple has no part '{name}'"))),
            Value::Type(t) if name == "name" => Value::string(t.display(self.mm).to_string()),
            Value::Collection(c) => {
                let mut out = Vec::new();
                for item in &c.items {
                    let v = strict!(self.navigate(item, name));
                    push_flattened(&mut out, v);
                }
                Value::collection(c.kind.collect_kind(), out)
            }
            other => Value::invalid(format!("{other} has no property '{name}'")),
        }
    }

    fn object_property(&mut self, o: ObjectId, name: &str) -> Value {
        let mm = self.mm;
        let class = self.model.class_of(o);
        if let Some(f) = mm.lookup_feature(class, name) {
            if let Some(body) = self.env.derivation(f) {
                let body = body.clone();
                return self.eval_body(&body, Value::Object(o), &[]);
            }
            let feature = mm.feature(f);
            let values = self.model.get(o, f);
            if feature.is_many() {
                let kind = CollectionKind::from_flags(feature.multiplicity.ordered, feature.multiplicity.unique);
                return Value::collection(kind, values.iter().map(convert_model_value).collect());
            }
            if let Some(v) = values.first() {
                return convert_model_value(v);
            }
            if let Some(body) = self.env.initializer(f) {
                let body = body.clone();
                return self.eval_body(&body, Value::Object(o), &[]);
            }
            return default_value(mm, f).as_ref().map_or(Value::Null, convert_model_value);
        }
        if let Some(def) = self.env.find_def_attribute(mm, class, name) {
            let body = def.body.clone();
            return self.eval_body(&body, Value::Object(o), &[]);
        }
        Value::invalid(format!("unknown property '{name}' of class {}", mm.classifier(class).name))
    }

    // ---------------------------------------------------------------- calls

    fn call(&mut self, src: Value, name: &str, args: &[Value]) -> Value {
        match name {
            "oclIsInvalid" if args.is_empty() => return Value::Boolean(src.is_invalid()),
            "oclIsUndefined" if args.is_empty() => return Value::Boolean(src.is_undefined()),
            _ => {}
        }
        let src = strict!(src);
        for a in args {
            if a.is_invalid() {
                return a.clone();
            }
        }
        if let Value::Object(o) = &src
            && let Some(v) = self.user_op(*o, name, args)
        {
            return v;
        }
        if let Value::Collection(c) = &src {
            let element_wise = matches!(name, "oclAsType" | "oclIsKindOf" | "oclIsTypeOf")
                && matches!(args, [Value::Type(t)] if !t.is_collection());
            if !element_wise && let Some(v) = self.oclany_op(&src, name, args) {
                return v;
            }
            // Implicit collect of an operation call over the elements.
            let mut out = Vec::new();
            for item in &c.items {
                let v = strict!(self.call(item.clone(), name, args));
                push_flattened(&mut out, v);
            }
            return Value::collection(c.kind.collect_kind(), out);
        }
        let library = match &src {
            Value::Integer(_) | Value::Real(_) | Value::Unlimited => self.numeric_op(&src, name, args),
            Value::String(s) => string_op(s, name, args),
            Value::Type(t) => self.type_op(t, name, args),
            _ => None,
        };
        if let Some(v) = library {
            return v;
        }
        if let Some(v) = self.oclany_op(&src, name, args) {
            return v;
        }
        if src.is_null() {
            return Value::invalid(format!("cannot call '{name}' on null"));
        }
        Value::invalid(format!("unknown operation '{name}' for {}", src.render(self.mm, self.model)))
    }

    fn user_op(&mut self, o: ObjectId, name: &str, args: &[Value]) -> Option<Value> {
        let mm = self.mm;
        let class = self.model.class_of(o);
        // The operation and the ones it overrides: all of their contracts apply.
        let candidates: Vec<OperationId> = mm
            .lookup_operations(class, name)
            .into_iter()
            .filter(|op| mm.operation(*op).params.len() == args.len())
            .collect();
        if let Some(&op) = candidates.first() {
            let names: Vec<String> = mm.operation(op).params.iter().map(|p| p.name.clone()).collect();
            let vars: Vec<(&str, Value)> = names.iter().map(String::as_str).zip(args.iter().cloned()).collect();
            let contracts: Vec<usize> = if self.check_contracts {
                candidates.iter().flat_map(|c| self.env.contracts_of(*c).iter().copied()).collect()
            } else {
                Vec::new()
            };
            self.check_conditions(&contracts, ConstraintKind::Pre, o, &vars, None, Record::Failures);
            let Some(body) = candidates.iter().find_map(|c| self.env.operation_body(*c)) else {
                return Some(Value::invalid(format!("operation '{name}' has no OCL body")));
            };
            let body = body.clone();
            let result = self.eval_body(&body, Value::Object(o), &vars);
            self.check_conditions(&contracts, ConstraintKind::Post, o, &vars, Some(&result), Record::Failures);
            return Some(result);
        }
        let defs = self.env.find_def_operations(mm, class, name);
        let def = defs.into_iter().find(|d| d.params.len() == args.len())?.clone();
        let vars: Vec<(&str, Value)> = def.params.iter().map(|(n, _)| n.as_str()).zip(args.iter().cloned()).collect();
        Some(self.eval_body(&def.body, Value::Object(o), &vars))
    }

    fn oclany_op(&mut self, src: &Value, name: &str, args: &[Value]) -> Option<Value> {
        let mm = self.mm;
        let model = self.model;
        Some(match (name, args) {
            ("oclIsKindOf", [Value::Type(t)]) => Value::Boolean(match src {
                Value::Null => matches!(**t, Type::OclVoid | Type::OclAny),
                _ => self.conforms(src, t, false),
            }),
            ("oclIsTypeOf", [Value::Type(t)]) => Value::Boolean(match src {
                Value::Null => **t == Type::OclVoid,
                _ => self.conforms(src, t, true),
            }),
            ("oclAsType", [Value::Type(t)]) => match (src, &**t) {
                (Value::Null, _) => Value::Null,
                (Value::Integer(i), Type::Real) => Value::Real(*i as f64),
                _ if self.conforms(src, t, false) => src.clone(),
                _ => Value::invalid(format!("cannot cast {} to {}", src.render(mm, model), t.display(mm))),
            },
            ("oclType" | "eClass", []) => match src {
                Value::Collection(c) => Value::Type(Rc::new(Type::collection(c.kind, Type::OclAny))),
                _ => Value::Type(Rc::new(src.type_of(model))),
            },
            ("oclAsSet", []) => match src {
                Value::Null => Value::collection(CollectionKind::Set, Vec::new()),
                v => Value::collection(CollectionKind::Set, vec![v.clone()]),
            },
            ("toString", []) => Value::string(src.to_ocl_string(mm, model)),
            ("oclIsNew", []) => Value::Boolean(false),
            ("oclContainer" | "eContainer", []) => match src {
                Value::Object(o) => model.container(*o).map_or(Value::Null, Value::Object),
                _ => Value::Null,
            },
            ("oclContents" | "eContents", []) => match src {
                Value::Object(o) => Value::collection(
                    CollectionKind::OrderedSet,
                    model.contents(mm, *o).into_iter().map(Value::Object).collect(),
                ),
                _ => Value::collection(CollectionKind::OrderedSet, Vec::new()),
            },
            ("eAllContents", []) => match src {
                Value::Object(o) => Value::collection(
                    CollectionKind::OrderedSet,
                    model.all_contents(mm, *o).into_iter().map(Value::Object).collect(),
                ),
                _ => Value::collection(CollectionKind::OrderedSet, Vec::new()),
            },
            _ => return None,
        })
    }

    /// Whether a runtime value conforms to `t` (exactly, when `exact`).
    fn conforms(&self, v: &Value, t: &Type, exact: bool) -> bool {
        let mm = self.mm;
        if !exact && *t == Type::OclAny {
            return !matches!(v, Value::Invalid(_));
        }
        match (v, t) {
            (Value::Object(o), Type::Classifier(c)) => {
                let class = self.model.class_of(*o);
                if exact { class == *c } else { mm.conforms_to(class, *c) }
            }
            (Value::Integer(_), Type::Real) => !exact,
            (Value::Collection(c), Type::Collection(k, elem)) => {
                let kind_ok = if exact { c.kind == *k } else { c.kind == *k || *k == CollectionKind::Collection };
                kind_ok && (**elem == Type::OclAny || c.items.iter().all(|i| self.conforms(i, elem, false)))
            }
            _ => v.type_of(self.model) == *t,
        }
    }

    fn numeric_op(&self, src: &Value, name: &str, args: &[Value]) -> Option<Value> {
        Some(match (name, src, args) {
            ("abs", Value::Integer(i), []) => {
                i.checked_abs().map_or_else(|| Value::invalid("integer overflow"), Value::Integer)
            }
            ("abs", Value::Real(r), []) => Value::Real(r.abs()),
            ("floor", Value::Real(r), []) => real_to_int(r.floor()),
            ("round", Value::Real(r), []) => real_to_int((r + 0.5).floor()),
            ("floor" | "round", Value::Integer(i), []) => Value::Integer(*i),
            ("max", a, [b]) => match a.ocl_compare(b) {
                Some(Ordering::Less) => promote(b, a),
                Some(_) => promote(a, b),
                None => Value::invalid("max() expects a number"),
            },
            ("min", a, [b]) => match a.ocl_compare(b) {
                Some(Ordering::Greater) => promote(b, a),
                Some(_) => promote(a, b),
                None => Value::invalid("min() expects a number"),
            },
            ("div", Value::Integer(a), [Value::Integer(b)]) => {
                if *b == 0 {
                    Value::invalid("division by zero")
                } else {
                    Value::Integer(a.wrapping_div(*b))
                }
            }
            ("mod", Value::Integer(a), [Value::Integer(b)]) => {
                if *b == 0 {
                    Value::invalid("division by zero")
                } else {
                    Value::Integer(a.wrapping_rem(*b))
                }
            }
            ("toInteger", Value::Integer(i), []) => Value::Integer(*i),
            ("toInteger", Value::Real(r), []) => real_to_int(r.trunc()),
            ("toReal", v, []) => v.as_f64().map_or_else(|| Value::invalid("not a number"), Value::Real),
            _ => return None,
        })
    }

    fn type_op(&self, t: &Type, name: &str, args: &[Value]) -> Option<Value> {
        let mm = self.mm;
        if name != "allInstances" || !args.is_empty() {
            return None;
        }
        Some(match t {
            Type::Classifier(c) if mm.is_class(*c) => Value::collection(
                CollectionKind::Set,
                self.model.instances_of(mm, *c).into_iter().map(Value::Object).collect(),
            ),
            Type::Classifier(c) if mm.is_enum(*c) => Value::collection(
                CollectionKind::Set,
                (0..mm.enum_literals(*c).len() as u32).map(|i| Value::Enum(*c, i)).collect(),
            ),
            Type::Boolean => Value::collection(CollectionKind::Set, vec![Value::Boolean(true), Value::Boolean(false)]),
            Type::OclVoid => Value::collection(CollectionKind::Set, vec![Value::Null]),
            other => Value::invalid(format!("allInstances() is not defined for {}", other.display(mm))),
        })
    }

    // ---------------------------------------------------------------- collections

    fn collection_op(&mut self, items: CollectionItems, name: &str, args: &[Value]) -> Value {
        for a in args {
            if a.is_invalid() {
                return a.clone();
            }
        }
        let (kind, items) = items.parts();
        let coll = |k: CollectionKind, v: Vec<Value>| Value::collection(k, v);
        let arg_items = |i: usize| -> Option<(CollectionKind, Vec<Value>)> {
            match args.get(i)? {
                Value::Collection(c) => Some((c.kind, c.items.clone())),
                _ => None,
            }
        };
        let contains = |items: &[Value], v: &Value| items.iter().any(|x| x.ocl_equals(v));
        let index = |v: &Value, len: usize| -> Option<usize> {
            match v {
                Value::Integer(i) if *i >= 1 && (*i as usize) <= len => Some(*i as usize - 1),
                _ => None,
            }
        };
        match (name, args) {
            ("size", []) => Value::Integer(items.len() as i64),
            ("count", [x]) => Value::Integer(items.iter().filter(|i| i.ocl_equals(x)).count() as i64),
            ("includes", [x]) => Value::Boolean(contains(&items, x)),
            ("excludes", [x]) => Value::Boolean(!contains(&items, x)),
            ("includesAll", [_]) => match arg_items(0) {
                Some((_, other)) => Value::Boolean(other.iter().all(|o| contains(&items, o))),
                None => Value::invalid("includesAll() expects a collection"),
            },
            ("excludesAll", [_]) => match arg_items(0) {
                Some((_, other)) => Value::Boolean(other.iter().all(|o| !contains(&items, o))),
                None => Value::invalid("excludesAll() expects a collection"),
            },
            ("isEmpty", []) => Value::Boolean(items.is_empty()),
            ("notEmpty", []) => Value::Boolean(!items.is_empty()),
            ("sum", []) => {
                let mut acc = Value::Integer(0);
                for i in &items {
                    acc = add(&acc, i);
                    if acc.is_invalid() {
                        return acc;
                    }
                }
                acc
            }
            ("max" | "min", []) => {
                let mut best: Option<Value> = None;
                for i in items {
                    best = Some(match best {
                        None => i,
                        Some(b) => match i.ocl_compare(&b) {
                            Some(Ordering::Greater) if name == "max" => i,
                            Some(Ordering::Less) if name == "min" => i,
                            Some(_) => b,
                            None => return Value::invalid(format!("{name}() requires comparable elements")),
                        },
                    });
                }
                best.unwrap_or_else(|| Value::invalid(format!("{name}() of an empty collection")))
            }
            ("asSet", []) => coll(CollectionKind::Set, items),
            ("asOrderedSet", []) => coll(CollectionKind::OrderedSet, items),
            ("asSequence", []) => coll(CollectionKind::Sequence, items),
            ("asBag", []) => coll(CollectionKind::Bag, items),
            ("flatten", []) => {
                let mut out = Vec::new();
                for i in items {
                    push_flattened(&mut out, i);
                }
                coll(kind, out)
            }
            ("including", [x]) => {
                let mut items = items;
                items.push(x.clone());
                coll(kind, items)
            }
            ("excluding", [x]) => coll(kind, items.into_iter().filter(|i| !i.ocl_equals(x)).collect()),
            ("includingAll", [_]) | ("union", [_]) => match arg_items(0) {
                Some((k2, other)) => {
                    let result_kind = if name == "union" { union_kind(kind, k2) } else { kind };
                    let mut items = items;
                    items.extend(other);
                    coll(result_kind, items)
                }
                None => Value::invalid(format!("{name}() expects a collection")),
            },
            ("excludingAll", [_]) => match arg_items(0) {
                Some((_, other)) => coll(kind, items.into_iter().filter(|i| !contains(&other, i)).collect()),
                None => Value::invalid("excludingAll() expects a collection"),
            },
            ("intersection", [_]) => match arg_items(0) {
                Some((k2, other)) => {
                    let result_kind =
                        if kind.is_unique() || k2.is_unique() { CollectionKind::Set } else { CollectionKind::Bag };
                    if result_kind == CollectionKind::Set {
                        coll(result_kind, items.into_iter().filter(|i| contains(&other, i)).collect())
                    } else {
                        // Bag intersection: minimum multiplicity.
                        let mut remaining = other;
                        let mut out = Vec::new();
                        for i in items {
                            if let Some(pos) = remaining.iter().position(|o| o.ocl_equals(&i)) {
                                remaining.remove(pos);
                                out.push(i);
                            }
                        }
                        coll(result_kind, out)
                    }
                }
                None => Value::invalid("intersection() expects a collection"),
            },
            ("symmetricDifference", [_]) => match arg_items(0) {
                Some((_, other)) => {
                    let mut out: Vec<Value> = items.iter().filter(|i| !contains(&other, i)).cloned().collect();
                    out.extend(other.iter().filter(|o| !contains(&items, o)).cloned());
                    coll(CollectionKind::Set, out)
                }
                None => Value::invalid("symmetricDifference() expects a collection"),
            },
            ("append", [x]) => {
                let mut items: Vec<Value> =
                    if kind.is_unique() { items.into_iter().filter(|i| !i.ocl_equals(x)).collect() } else { items };
                items.push(x.clone());
                coll(kind, items)
            }
            ("prepend", [x]) => {
                let mut items: Vec<Value> =
                    if kind.is_unique() { items.into_iter().filter(|i| !i.ocl_equals(x)).collect() } else { items };
                items.insert(0, x.clone());
                coll(kind, items)
            }
            ("insertAt", [i, x]) => {
                let mut items: Vec<Value> =
                    if kind.is_unique() { items.into_iter().filter(|e| !e.ocl_equals(x)).collect() } else { items };
                match i {
                    Value::Integer(n) if *n >= 1 && (*n as usize) <= items.len() + 1 => {
                        items.insert(*n as usize - 1, x.clone());
                        coll(kind, items)
                    }
                    _ => Value::invalid(format!("index {i} out of range")),
                }
            }
            ("at", [i]) => match index(i, items.len()) {
                Some(n) => items[n].clone(),
                None => Value::invalid(format!("index {i} out of range 1..{}", items.len())),
            },
            ("first", []) => items.first().cloned().unwrap_or_else(|| Value::invalid("first() of an empty collection")),
            ("last", []) => items.last().cloned().unwrap_or_else(|| Value::invalid("last() of an empty collection")),
            ("indexOf", [x]) => match items.iter().position(|i| i.ocl_equals(x)) {
                Some(p) => Value::Integer(p as i64 + 1),
                None => Value::invalid("indexOf(): element not found"),
            },
            ("subSequence" | "subOrderedSet", [a, b]) => match (index(a, items.len()), index(b, items.len())) {
                (Some(lo), Some(hi)) if lo <= hi => coll(kind, items[lo..=hi].to_vec()),
                _ => Value::invalid(format!("{name}(): indices out of range")),
            },
            ("reverse", []) => coll(kind, items.into_iter().rev().collect()),
            ("selectByKind" | "selectByType", [Value::Type(t)]) => {
                let exact = name == "selectByType";
                coll(kind, items.into_iter().filter(|i| !i.is_null() && self.conforms(i, t, exact)).collect())
            }
            ("product", [_]) => match arg_items(0) {
                Some((_, other)) => {
                    let mut out = Vec::new();
                    for a in &items {
                        for b in &other {
                            out.push(Value::tuple(vec![("first".into(), a.clone()), ("second".into(), b.clone())]));
                        }
                    }
                    coll(CollectionKind::Set, out)
                }
                None => Value::invalid("product() expects a collection"),
            },
            _ => {
                let src = coll(kind, items);
                self.oclany_op(&src, name, args)
                    .unwrap_or_else(|| Value::invalid(format!("unknown collection operation '{name}'")))
            }
        }
    }

    // ---------------------------------------------------------------- iterators

    fn bind_and_eval(&mut self, iterators: &[VarDecl], values: &[Value], body: &Expr) -> Value {
        let mut frame = Frame { vars: Vec::new(), implicit: None };
        if iterators.is_empty() {
            frame.implicit = values.first().cloned();
        } else {
            for (v, val) in iterators.iter().zip(values) {
                frame.vars.push((v.name.name.as_str().into(), val.clone()));
            }
        }
        self.with_frame(frame, |ev| ev.eval(body))
    }

    fn iterator(&mut self, items: CollectionItems, name: &str, iterators: &[VarDecl], body: &Expr) -> Value {
        let (kind, items) = items.parts();
        let one = |ev: &mut Self, item: &Value| ev.bind_and_eval(iterators, std::slice::from_ref(item), body);
        match name {
            "select" | "reject" => {
                let keep = name == "select";
                let mut out = Vec::new();
                for item in items {
                    match one(self, &item) {
                        Value::Boolean(b) => {
                            if b == keep {
                                out.push(item);
                            }
                        }
                        Value::Invalid(m) => return Value::Invalid(m),
                        other => return Value::invalid(format!("{name} body is not a Boolean: {other}")),
                    }
                }
                Value::collection(kind, out)
            }
            "collect" | "collectNested" => {
                let mut out = Vec::new();
                for item in &items {
                    let v = strict!(one(self, item));
                    if name == "collect" {
                        push_flattened(&mut out, v);
                    } else {
                        out.push(v);
                    }
                }
                Value::collection(kind.collect_kind(), out)
            }
            "forAll" | "exists" => {
                let decisive = name == "exists";
                let mut undefined: Option<Value> = None;
                let n = iterators.len().max(1);
                let mut indices = vec![0usize; n];
                if items.is_empty() {
                    return Value::Boolean(!decisive);
                }
                loop {
                    let values: Vec<Value> = indices.iter().map(|i| items[*i].clone()).collect();
                    match self.bind_and_eval(iterators, &values, body) {
                        Value::Boolean(b) if b == decisive => return Value::Boolean(decisive),
                        Value::Boolean(_) => {}
                        Value::Invalid(m) => undefined = Some(Value::Invalid(m)),
                        Value::Null => {
                            undefined.get_or_insert(Value::invalid(format!("{name} body evaluated to null")));
                        }
                        other => undefined = Some(Value::invalid(format!("{name} body is not a Boolean: {other}"))),
                    }
                    // Advance the odometer over all iterator combinations.
                    let mut pos = n;
                    loop {
                        if pos == 0 {
                            return undefined.unwrap_or(Value::Boolean(!decisive));
                        }
                        pos -= 1;
                        indices[pos] += 1;
                        if indices[pos] < items.len() {
                            break;
                        }
                        indices[pos] = 0;
                    }
                }
            }
            "one" => {
                let mut count = 0;
                for item in &items {
                    match one(self, item) {
                        Value::Boolean(true) => count += 1,
                        Value::Boolean(false) => {}
                        Value::Invalid(m) => return Value::Invalid(m),
                        other => return Value::invalid(format!("one body is not a Boolean: {other}")),
                    }
                }
                Value::Boolean(count == 1)
            }
            "any" => {
                for item in items {
                    match one(self, &item) {
                        Value::Boolean(true) => return item,
                        Value::Boolean(false) => {}
                        Value::Invalid(m) => return Value::Invalid(m),
                        other => return Value::invalid(format!("any body is not a Boolean: {other}")),
                    }
                }
                Value::Null
            }
            "isUnique" => {
                let mut seen = HashSet::new();
                for item in &items {
                    let v = strict!(one(self, item));
                    if !seen.insert(v.key()) {
                        return Value::Boolean(false);
                    }
                }
                Value::Boolean(true)
            }
            "sortedBy" => {
                let mut keyed = Vec::new();
                for item in items {
                    let k = strict!(one(self, &item));
                    keyed.push((k, item));
                }
                let mut failed = false;
                keyed.sort_by(|a, b| {
                    a.0.ocl_compare(&b.0).unwrap_or_else(|| {
                        failed = true;
                        Ordering::Equal
                    })
                });
                if failed {
                    return Value::invalid("sortedBy: keys are not comparable");
                }
                let k = if kind.is_unique() { CollectionKind::OrderedSet } else { CollectionKind::Sequence };
                Value::collection(k, keyed.into_iter().map(|(_, v)| v).collect())
            }
            "closure" => {
                let mut result: Vec<Value> = Vec::new();
                let mut seen = HashSet::new();
                let mut queue: std::collections::VecDeque<Value> = items.into_iter().collect();
                while let Some(item) = queue.pop_front() {
                    let v = strict!(one(self, &item));
                    let next = match v {
                        Value::Collection(c) => c.items.clone(),
                        Value::Null => Vec::new(),
                        single => vec![single],
                    };
                    for n in next {
                        if !n.is_null() && seen.insert(n.key()) {
                            result.push(n.clone());
                            queue.push_back(n);
                        }
                    }
                }
                let k = if kind.is_ordered() { CollectionKind::OrderedSet } else { CollectionKind::Set };
                Value::collection(k, result)
            }
            other => Value::invalid(format!("unknown iterator '{other}'")),
        }
    }

    // ---------------------------------------------------------------- operators

    fn binary(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr) -> Value {
        match op {
            BinOp::And | BinOp::Or | BinOp::Implies => {
                let l = self.eval(lhs);
                let decisive = |v: &Value| match op {
                    BinOp::And => matches!(v, Value::Boolean(false)),
                    BinOp::Or => matches!(v, Value::Boolean(true)),
                    _ => false,
                };
                if decisive(&l) {
                    return l;
                }
                if op == BinOp::Implies && matches!(l, Value::Boolean(false)) {
                    return Value::Boolean(true);
                }
                let r = self.eval(rhs);
                if decisive(&r) || (op == BinOp::Implies && matches!(r, Value::Boolean(true))) {
                    return r;
                }
                match (&l, &r) {
                    (Value::Invalid(_), _) => l,
                    (_, Value::Invalid(_)) => r,
                    (Value::Boolean(a), Value::Boolean(b)) => Value::Boolean(match op {
                        BinOp::And => *a && *b,
                        BinOp::Or => *a || *b,
                        _ => !*a || *b,
                    }),
                    (Value::Null, _) | (_, Value::Null) => {
                        Value::invalid(format!("operand of '{}' is null", op.symbol()))
                    }
                    _ => Value::invalid(format!("operands of '{}' must be Booleans", op.symbol())),
                }
            }
            _ => {
                let l = strict!(self.eval(lhs));
                let r = strict!(self.eval(rhs));
                binary_values(op, &l, &r)
            }
        }
    }
}

/// Items of a collection operand, remembering whether the source was a real collection.
enum CollectionItems {
    Of(CollectionKind, Vec<Value>),
    /// A single value (or null) used with `->`: behaves like a `Set`.
    Single(Vec<Value>),
}

impl IntoIterator for CollectionItems {
    type Item = Value;
    type IntoIter = std::vec::IntoIter<Value>;

    fn into_iter(self) -> Self::IntoIter {
        match self {
            CollectionItems::Of(_, v) | CollectionItems::Single(v) => v.into_iter(),
        }
    }
}

fn to_collection(v: Value) -> Result<CollectionItems, Value> {
    match v {
        Value::Invalid(_) => Err(v),
        Value::Null => Ok(CollectionItems::Single(Vec::new())),
        Value::Collection(c) => {
            let Collection { kind, items } = Rc::unwrap_or_clone(c);
            Ok(CollectionItems::Of(kind, items))
        }
        other => Ok(CollectionItems::Single(vec![other])),
    }
}

impl CollectionItems {
    fn parts(self) -> (CollectionKind, Vec<Value>) {
        match self {
            CollectionItems::Of(k, v) => (k, v),
            CollectionItems::Single(v) => (CollectionKind::Set, v),
        }
    }

    fn filter_nulls(self) -> Self {
        match self {
            CollectionItems::Of(k, v) => CollectionItems::Of(k, v.into_iter().filter(|x| !x.is_null()).collect()),
            CollectionItems::Single(v) => CollectionItems::Single(v.into_iter().filter(|x| !x.is_null()).collect()),
        }
    }
}

fn push_flattened(out: &mut Vec<Value>, v: Value) {
    match v {
        Value::Collection(c) => {
            for i in c.items.iter() {
                push_flattened(out, i.clone());
            }
        }
        other => out.push(other),
    }
}

/// Converts a stored model value into an OCL value.
pub fn convert_model_value(v: &ModelValue) -> Value {
    match v {
        ModelValue::Boolean(b) => Value::Boolean(*b),
        ModelValue::Integer(i) => Value::Integer(*i),
        ModelValue::Real(r) => Value::Real(*r),
        ModelValue::String(s) => Value::string(s),
        ModelValue::Enum(e, i) => Value::Enum(*e, *i as u32),
        ModelValue::Object(o) => Value::Object(*o),
    }
}

fn real_to_int(r: f64) -> Value {
    if r.is_finite() && r.abs() < 9.2e18 {
        Value::Integer(r as i64)
    } else {
        Value::invalid("number out of Integer range")
    }
}

/// `max`/`min` result: Real if either operand is Real.
fn promote(chosen: &Value, other: &Value) -> Value {
    match (chosen, other) {
        (Value::Integer(i), Value::Real(_)) => Value::Real(*i as f64),
        _ => chosen.clone(),
    }
}

fn union_kind(a: CollectionKind, b: CollectionKind) -> CollectionKind {
    use CollectionKind::*;
    match (a, b) {
        (Set, Set) => Set,
        (OrderedSet, OrderedSet) => OrderedSet,
        (Sequence | OrderedSet, Sequence | OrderedSet) => Sequence,
        _ => Bag,
    }
}

fn add(a: &Value, b: &Value) -> Value {
    binary_values(BinOp::Add, a, b)
}

fn binary_values(op: BinOp, l: &Value, r: &Value) -> Value {
    use Value::*;
    match op {
        BinOp::Eq => return Boolean(l.ocl_equals(r)),
        BinOp::Ne => return Boolean(!l.ocl_equals(r)),
        BinOp::Xor => {
            return match (l, r) {
                (Boolean(a), Boolean(b)) => Boolean(a ^ b),
                _ => Value::invalid("operands of 'xor' must be Booleans"),
            };
        }
        BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
            let Some(ord) = l.ocl_compare(r) else {
                return Value::invalid(format!("cannot compare {l} and {r} with '{}'", op.symbol()));
            };
            return Boolean(match op {
                BinOp::Lt => ord == Ordering::Less,
                BinOp::Gt => ord == Ordering::Greater,
                BinOp::Le => ord != Ordering::Greater,
                _ => ord != Ordering::Less,
            });
        }
        _ => {}
    }
    let overflow = || Value::invalid("integer overflow");
    match (op, l, r) {
        (BinOp::Add, Integer(a), Integer(b)) => a.checked_add(*b).map_or_else(overflow, Integer),
        (BinOp::Sub, Integer(a), Integer(b)) => a.checked_sub(*b).map_or_else(overflow, Integer),
        (BinOp::Mul, Integer(a), Integer(b)) => a.checked_mul(*b).map_or_else(overflow, Integer),
        (BinOp::Add, String(a), String(b)) => Value::string(format!("{a}{b}")),
        (BinOp::Sub, Collection(a), Collection(b)) => Value::collection(
            a.kind,
            a.items.iter().filter(|x| !b.items.iter().any(|y| y.ocl_equals(x))).cloned().collect(),
        ),
        (BinOp::Div, a, b) => match (a.as_f64(), b.as_f64()) {
            (Some(_), Some(0.0)) => Value::invalid("division by zero"),
            (Some(x), Some(y)) => Real(x / y),
            _ => Value::invalid(format!("cannot divide {a} by {b}")),
        },
        (op, a, b) => match (a.as_f64(), b.as_f64()) {
            (Some(x), Some(y)) => Real(match op {
                BinOp::Add => x + y,
                BinOp::Sub => x - y,
                _ => x * y,
            }),
            _ if a.is_null() || b.is_null() => Value::invalid(format!("operand of '{}' is null", op.symbol())),
            _ => Value::invalid(format!("operator '{}' is not defined for {a} and {b}", op.symbol())),
        },
    }
}

fn string_op(s: &str, name: &str, args: &[Value]) -> Option<Value> {
    use Value::*;
    let chars = || s.chars().collect::<Vec<char>>();
    Some(match (name, args) {
        ("size", []) => Integer(s.chars().count() as i64),
        ("concat", [String(b)]) => Value::string(format!("{s}{b}")),
        ("substring", [Integer(lo), Integer(hi)]) => {
            let cs = chars();
            if *lo < 1 || *hi < *lo - 1 || *hi as usize > cs.len() {
                Value::invalid(format!("substring({lo}, {hi}) out of range for a string of size {}", cs.len()))
            } else {
                Value::string(cs[*lo as usize - 1..*hi as usize].iter().collect::<std::string::String>())
            }
        }
        ("toInteger", []) => {
            s.trim().parse().map(Integer).unwrap_or_else(|_| Value::invalid(format!("'{s}' is not an integer")))
        }
        ("toReal", []) => {
            s.trim().parse().map(Real).unwrap_or_else(|_| Value::invalid(format!("'{s}' is not a number")))
        }
        ("toBoolean", []) => match s.trim() {
            "true" => Boolean(true),
            "false" => Boolean(false),
            _ => Value::invalid(format!("'{s}' is not a Boolean")),
        },
        ("toUpper" | "toUpperCase", []) => Value::string(s.to_uppercase()),
        ("toLower" | "toLowerCase", []) => Value::string(s.to_lowercase()),
        ("trim", []) => Value::string(s.trim()),
        ("indexOf", [String(sub)]) => Integer(match s.find(&**sub) {
            Some(b) => s[..b].chars().count() as i64 + 1,
            None => 0,
        }),
        ("lastIndexOf", [String(sub)]) => Integer(match s.rfind(&**sub) {
            Some(b) => s[..b].chars().count() as i64 + 1,
            None => 0,
        }),
        ("at", [Integer(i)]) => match chars().get((*i as usize).wrapping_sub(1)) {
            Some(c) if *i >= 1 => Value::string(c.to_string()),
            _ => Value::invalid(format!("index {i} out of range")),
        },
        ("characters", []) => {
            Value::collection(CollectionKind::Sequence, s.chars().map(|c| Value::string(c.to_string())).collect())
        }
        ("equalsIgnoreCase", [String(b)]) => Boolean(s.to_lowercase() == b.to_lowercase()),
        ("startsWith", [String(b)]) => Boolean(s.starts_with(&**b)),
        ("endsWith", [String(b)]) => Boolean(s.ends_with(&**b)),
        ("matches", [String(re)]) => match regex::Regex::new(&format!("^(?:{re})$")) {
            Ok(r) => Boolean(r.is_match(s)),
            Err(e) => Value::invalid(format!("invalid regular expression: {e}")),
        },
        ("replaceAll" | "replaceFirst", [String(re), String(rep)]) => match regex::Regex::new(re) {
            // Java-style `$1` group references work as-is with the regex crate.
            Ok(r) if name == "replaceAll" => Value::string(r.replace_all(s, &**rep)),
            Ok(r) => Value::string(r.replace(s, &**rep)),
            Err(e) => Value::invalid(format!("invalid regular expression: {e}")),
        },
        ("substituteAll", [String(a), String(b)]) => Value::string(s.replace(&**a, b)),
        ("substituteFirst", [String(a), String(b)]) => Value::string(s.replacen(&**a, b, 1)),
        ("tokenize", []) => tokenize(s, " \t\n\r\u{c}", false),
        ("tokenize", [String(d)]) => tokenize(s, d, false),
        ("tokenize", [String(d), Boolean(keep)]) => tokenize(s, d, *keep),
        _ => return None,
    })
}

fn tokenize(s: &str, delims: &str, keep: bool) -> Value {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        if delims.contains(c) {
            if !cur.is_empty() {
                out.push(Value::string(std::mem::take(&mut cur)));
            }
            if keep {
                out.push(Value::string(c.to_string()));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(Value::string(cur));
    }
    Value::collection(CollectionKind::Sequence, out)
}
