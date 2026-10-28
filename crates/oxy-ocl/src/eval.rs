//! Tree-walking evaluation of OCL expressions over an instance model.
//!
//! Evaluation never fails: errors produce `invalid` values carrying an explanation, which
//! propagate according to OCL's strictness rules (with the usual exceptions: Boolean
//! operators, `oclIsInvalid()`, `oclIsUndefined()`).

use std::cmp::Ordering;
use std::collections::HashSet;
use std::rc::Rc;

use oxy_ecore::{DataTypeKind, Metamodel, Model, ModelValue, ObjectId, OperationId, PackageId, default_value};

use crate::env::{Body, Environment, PackageAlias, lookup_classifier_in, resolve_type_in};
use crate::host::{Host, ResolveRequest};
use crate::span::Span;
use crate::syntax::ast::*;
use crate::types::{CollectionKind, Type, library_type};
use crate::validate::{Status, status_of};
use crate::value::{Collection, Value};

/// Maximum nesting of operation/definition calls before evaluation is aborted.
const MAX_DEPTH: u32 = 256;

/// When less stack than this is left, evaluation continues on a new stack segment...
pub(crate) const RED_ZONE: usize = 128 * 1024;
/// ... of this size.
pub(crate) const STACK_CHUNK: usize = 4 * 1024 * 1024;

/// Most iterations of one loop (`while`, `forEach`) before it is stopped.
const MAX_ITERATIONS: u64 = 10_000_000;

#[derive(Clone, Debug)]
struct Frame {
    vars: Vec<(Rc<str>, Value)>,
    /// The current element of an implicit iterator (or `self` in the outermost frame, or the
    /// object being populated by `object`).
    implicit: Option<Value>,
    /// Whether operations can be called on `implicit` without a source (not on the object being
    /// populated: it is the implicit source of properties only, as in Eclipse QVTo).
    operations: bool,
    /// The declared types of the variables that have one (`var x : T`).
    types: Vec<(Rc<str>, Type)>,
}

/// A transfer of control of Imperative OCL, pending until a block, loop or operation takes it.
#[derive(Clone, Debug)]
enum Control {
    Return(Value),
    Break,
    Continue,
    /// A fatal assertion failed: the transformation ends.
    Abort(String),
}

/// The model an evaluator reads (and, for Imperative OCL, changes).
enum ModelRef<'a> {
    Shared(&'a Model),
    Exclusive(&'a mut Model),
}

impl std::ops::Deref for ModelRef<'_> {
    type Target = Model;

    fn deref(&self) -> &Model {
        match self {
            ModelRef::Shared(m) => m,
            ModelRef::Exclusive(m) => m,
        }
    }
}

/// An assignment of a `late resolve`, done at the end of a transformation (see
/// [`Host::defer`] and [`Evaluator::complete_late`]).
#[derive(Clone, Debug)]
pub struct LateResolve {
    /// The object whose property is assigned (`None`: a variable, which cannot be late).
    pub target: ObjectId,
    pub property: String,
    pub add: bool,
    pub request: ResolveRequest,
    one: bool,
    var: Option<Rc<str>>,
    ty: Option<Type>,
    condition: Option<Rc<Expr>>,
    frames: Vec<Frame>,
    span: Span,
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
    model: ModelRef<'a>,
    env: &'a Environment,
    host: Option<&'a dyn Host>,
    aliases: Rc<[PackageAlias]>,
    control: Option<Control>,
    /// Problems of Imperative OCL statements (failed assignments, invalid results).
    errors: Vec<(Span, String)>,
    frames: Vec<Frame>,
    depth: u32,
    /// Package of the body being evaluated, for resolving unqualified type names.
    package: Option<PackageId>,
    /// Whether operation calls evaluate the operation's pre- and postconditions.
    check_contracts: bool,
    contract_checks: Vec<ContractCheck>,
    /// The resources `allInstances()` ranges over (`None`: all).
    extent: Option<Vec<usize>>,
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
        Evaluator::with_model(mm, ModelRef::Shared(model), env)
    }

    /// An evaluator that may change the model (for Imperative OCL: assignments, `object`).
    pub fn new_mut(mm: &'a Metamodel, model: &'a mut Model, env: &'a Environment) -> Self {
        Evaluator::with_model(mm, ModelRef::Exclusive(model), env)
    }

    fn with_model(mm: &'a Metamodel, model: ModelRef<'a>, env: &'a Environment) -> Self {
        Evaluator {
            mm,
            model,
            env,
            host: None,
            aliases: Rc::from(Vec::new()),
            control: None,
            errors: Vec::new(),
            frames: vec![Frame { vars: Vec::new(), implicit: None, operations: true, types: Vec::new() }],
            depth: 0,
            package: None,
            check_contracts: true,
            contract_checks: Vec::new(),
            extent: None,
        }
    }

    /// Evaluates with the names, operations and expressions of a language built on OCL.
    pub fn with_host(mut self, host: &'a dyn Host) -> Self {
        self.host = Some(host);
        self
    }

    /// Resolves type names with package aliases (QVT's model types).
    pub fn with_aliases(mut self, aliases: Rc<[PackageAlias]>) -> Self {
        self.aliases = aliases;
        self
    }

    /// Replaces the package aliases (for code of another module, e.g. a QVTo library) and
    /// returns the previous ones.
    pub fn set_aliases(&mut self, aliases: Rc<[PackageAlias]>) -> Rc<[PackageAlias]> {
        std::mem::replace(&mut self.aliases, aliases)
    }

    pub fn metamodel(&self) -> &'a Metamodel {
        self.mm
    }

    pub fn model(&self) -> &Model {
        &self.model
    }

    /// The model, if this evaluator may change it ([`Evaluator::new_mut`]).
    pub fn model_mut(&mut self) -> Option<&mut Model> {
        match &mut self.model {
            ModelRef::Exclusive(m) => Some(m),
            ModelRef::Shared(_) => None,
        }
    }

    pub fn environment(&self) -> &'a Environment {
        self.env
    }

    /// The problems of Imperative OCL statements since the last call: assignments that failed
    /// and statements whose value is invalid.
    pub fn take_errors(&mut self) -> Vec<(Span, String)> {
        std::mem::take(&mut self.errors)
    }

    /// Records a problem of a statement.
    pub fn report(&mut self, span: Span, message: impl Into<String>) {
        let message = message.into();
        if !self.errors.iter().any(|(s, m)| *s == span && *m == message) {
            self.errors.push((span, message));
        }
    }

    /// The message of a failed fatal assertion, which ended the evaluation.
    pub fn aborted(&self) -> Option<&str> {
        match &self.control {
            Some(Control::Abort(m)) => Some(m),
            _ => None,
        }
    }

    /// Turns the evaluation of pre- and postconditions at operation calls on or off (it is on
    /// by default).
    /// Restricts `allInstances()` to the objects of some resources (`None`: all), like Eclipse
    /// OCL, where it ranges over the resource of the object being validated.
    pub fn set_extent(&mut self, resources: Option<Vec<usize>>) {
        self.extent = resources;
    }

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
            let status = status_of(value, self.mm, &self.model);
            let keep = match record {
                Record::All => true,
                Record::Failures => status != Status::Satisfied,
                Record::Errors => matches!(status, Status::Error(_)),
            };
            if keep {
                let message = match (&status, &oc.message) {
                    (Status::Violated, Some(m)) => {
                        let body = Body { expr: m.clone(), ..oc.body.clone() };
                        Some(self.eval_body(&body, Value::Object(object), &bound).to_ocl_string(self.mm, &self.model))
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
        let mut frame = Frame { vars: Vec::new(), implicit: None, operations: true, types: Vec::new() };
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

    /// The value of a variable in scope.
    pub fn lookup_var(&self, name: &str) -> Option<Value> {
        self.frames.iter().rev().find_map(|f| f.vars.iter().rev().find(|(n, _)| &**n == name).map(|(_, v)| v.clone()))
    }

    /// Assigns a variable in scope; `false` if there is none of that name.
    pub fn set_var(&mut self, name: &str, value: Value) -> bool {
        for f in self.frames.iter_mut().rev() {
            if let Some(slot) = f.vars.iter_mut().rev().find(|(n, _)| &**n == name) {
                slot.1 = value;
                return true;
            }
        }
        false
    }

    /// Declares a variable in the innermost scope.
    pub fn declare_var(&mut self, name: &str, value: Value) {
        if let Some(f) = self.frames.last_mut() {
            f.vars.push((name.into(), value));
        }
    }

    fn with_frame<T>(&mut self, frame: Frame, f: impl FnOnce(&mut Self) -> T) -> T {
        self.frames.push(frame);
        let r = f(self);
        self.frames.pop();
        r
    }

    /// Runs `f` in a fresh scope (like an operation body): `self` bound to `self_value` (and its
    /// properties usable without `self.`) and the given variables. `None` when calls nest too
    /// deeply (endless recursion).
    pub fn call_frame<T>(
        &mut self,
        self_value: Option<Value>,
        vars: &[(&str, Value)],
        f: impl FnOnce(&mut Self) -> T,
    ) -> Option<T> {
        if self.depth >= MAX_DEPTH {
            return None;
        }
        let mut frame = Frame { vars: Vec::new(), implicit: None, operations: true, types: Vec::new() };
        if let Some(s) = self_value {
            frame.vars.push(("self".into(), s.clone()));
            frame.implicit = Some(s);
        }
        for (n, v) in vars {
            frame.vars.push(((*n).into(), v.clone()));
        }
        self.depth += 1;
        let saved = std::mem::replace(&mut self.frames, vec![frame]);
        let r = f(self);
        self.frames = saved;
        self.depth -= 1;
        Some(r)
    }

    /// Runs `f` in a nested scope in which the properties of `implicit` can be used without a
    /// source (as in the body of `object`; its operations need a source).
    pub fn with_implicit<T>(&mut self, implicit: Value, f: impl FnOnce(&mut Self) -> T) -> T {
        self.with_frame(Frame { vars: Vec::new(), implicit: Some(implicit), operations: false, types: Vec::new() }, f)
    }

    /// The value of a `return` that ended the statements evaluated last (and clears it, as
    /// well as `break` and `continue` outside of loops).
    pub fn take_return(&mut self) -> Option<Value> {
        match self.control.take() {
            Some(Control::Return(v)) => Some(v),
            Some(abort @ Control::Abort(_)) => {
                self.control = Some(abort);
                None
            }
            _ => None,
        }
    }

    /// Evaluates statements in order (until a `return`, `break` or `continue`); the value is
    /// the last one's. Invalid values of the others are reported.
    pub fn eval_statements(&mut self, items: &[Expr]) -> Value {
        let mut last = Value::Null;
        for (i, item) in items.iter().enumerate() {
            last = self.eval(item);
            if self.control.is_some() {
                break;
            }
            if i + 1 < items.len()
                && let Value::Invalid(m) = &last
            {
                let m = m.to_string();
                self.report(item.span, m);
            }
        }
        last
    }

    pub fn eval(&mut self, e: &Expr) -> Value {
        if self.control.is_some() {
            return Value::Null;
        }
        // Recursion (operations, QVT mappings) is bounded by `MAX_DEPTH`, not by the thread's stack.
        stacker::maybe_grow(RED_ZONE, STACK_CHUNK, || self.eval_kind(e))
    }

    fn eval_kind(&mut self, e: &Expr) -> Value {
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
            ExprKind::TypeLit(te) => match self.resolve_type(te) {
                Ok(t) => Value::Type(Rc::new(t)),
                Err(m) => Value::invalid(m),
            },
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
                // Imperative OCL's mutable lists: `list->add(x)` adds to the variable or property.
                if name.name == "add"
                    && *arrow
                    && let ([arg], Value::Collection(_)) = (args.as_slice(), &src)
                    && matches!(source.kind, ExprKind::Name { .. } | ExprKind::Property { .. })
                {
                    let arg = strict!(arg.clone());
                    if let Err(m) = self.assign(source, arg, true) {
                        return Value::invalid(m);
                    }
                    return Value::Null;
                }
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
                let implicit: Vec<Value> =
                    self.frames.iter().rev().filter(|f| f.operations).filter_map(|f| f.implicit.clone()).collect();
                for v in &implicit {
                    if let Value::Object(o) = v
                        && self.has_user_op(*o, &name.name, args.len())
                    {
                        return self.call(v.clone(), &name.name, &args);
                    }
                    if let Some(h) = self.host
                        && let Some(r) = h.operation(self, Some(v), &name.name, &args)
                    {
                        return r;
                    }
                }
                if let Some(h) = self.host
                    && let Some(r) = h.operation(self, None, &name.name, &args)
                {
                    return r;
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
                    let mut frame = Frame { vars: Vec::new(), implicit: None, operations: true, types: Vec::new() };
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
                self.frames.push(Frame { vars: Vec::new(), implicit: None, operations: true, types: Vec::new() });
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
            _ => self.eval_imperative(e),
        }
    }

    /// Resolves a type written in an expression (with the package aliases).
    pub fn resolve_type(&self, te: &TypeExpr) -> Result<Type, String> {
        let mut diags = Vec::new();
        let t = resolve_type_in(self.mm, self.package, &self.aliases, te, &mut diags);
        match diags.into_iter().next() {
            Some(d) => match (&te.kind, self.host) {
                (TypeExprKind::Path(path), Some(h)) => h.named_type(path).ok_or(d.message),
                _ => Err(d.message),
            },
            None => Ok(t),
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
            if let Some(h) = self.host
                && let Some(v) = h.variable(self, name)
            {
                return v;
            }
            let implicit: Vec<Value> = self.frames.iter().rev().filter_map(|f| f.implicit.clone()).collect();
            for v in &implicit {
                if self.has_property(v, name) {
                    return self.navigate(v, name);
                }
                if let Some(h) = self.host
                    && let Some(r) = h.property(self, v, name)
                {
                    return r;
                }
            }
            if let Some(t) = library_type(name) {
                return Value::Type(Rc::new(t));
            }
            if let Ok(c) = lookup_classifier_in(mm, self.package, &self.aliases, &[name]) {
                return Value::Type(Rc::new(Type::of_classifier(mm, c)));
            }
            return Value::invalid(format!("unknown name '{name}'"));
        }
        let names: Vec<&str> = path.iter().map(|i| i.name.as_str()).collect();
        let Some((last, head)) = names.split_last() else { return Value::invalid("empty name") };
        if let Ok(c) = lookup_classifier_in(mm, self.package, &self.aliases, head)
            && mm.is_enum(c)
        {
            return match mm.enum_literal_index(c, last) {
                Some(i) => Value::Enum(c, i as u32),
                None => Value::invalid(format!("enumeration '{}' has no literal '{last}'", mm.classifier(c).name)),
            };
        }
        match lookup_classifier_in(mm, self.package, &self.aliases, &names) {
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
            Value::Model(_) => match self.host {
                Some(h) => {
                    h.property(self, src, name).unwrap_or_else(|| Value::invalid(format!("unknown property '{name}'")))
                }
                None => Value::invalid(format!("unknown property '{name}'")),
            },
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
        if let Some(h) = self.host
            && let Some(v) = h.property(self, &Value::Object(o), name)
        {
            return v;
        }
        Value::invalid(format!("unknown property '{name}' of class {}", mm.classifier(class).name))
    }

    // ---------------------------------------------------------------- calls

    /// Calls an operation on a value (`src.name(args)`): a metamodel or `def:` operation, the
    /// host's, or OCL's.
    pub fn call(&mut self, src: Value, name: &str, args: &[Value]) -> Value {
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
        if !matches!(src, Value::Collection(_))
            && let Some(h) = self.host
            && let Some(v) = h.operation(self, Some(&src), name, args)
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
        Value::invalid(format!("unknown operation '{name}' for {}", src.render(self.mm, &self.model)))
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
        let model = &*self.model;
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
    pub fn conforms(&self, v: &Value, t: &Type, exact: bool) -> bool {
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
            _ => v.type_of(&self.model) == *t,
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
            Type::Classifier(c) if mm.is_class(*c) => {
                let mut instances = self.model.instances_of(mm, *c);
                if let Some(extent) = &self.extent {
                    instances.retain(|o| extent.contains(&self.model.object(*o).resource));
                }
                Value::collection(CollectionKind::Set, instances.into_iter().map(Value::Object).collect())
            }
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
                if let Some(v) = self.oclany_op(&src, name, args) {
                    return v;
                }
                if let Some(h) = self.host
                    && let Some(v) = h.operation(self, Some(&src), name, args)
                {
                    return v;
                }
                Value::invalid(format!("unknown collection operation '{name}'"))
            }
        }
    }

    // ---------------------------------------------------------------- iterators

    fn bind_and_eval(&mut self, iterators: &[VarDecl], values: &[Value], body: &Expr) -> Value {
        let mut frame = Frame { vars: Vec::new(), implicit: None, operations: true, types: Vec::new() };
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
        // `collectselect(i; r = e | cond)`: `r` is collected where `cond` holds.
        if let [it, target] = iterators
            && let Some(init) = &target.init
            && name.starts_with("collectselect")
        {
            let mut out = Vec::new();
            for item in items {
                let frame = Frame {
                    vars: vec![(it.name.name.as_str().into(), item)],
                    implicit: None,
                    operations: true,
                    types: Vec::new(),
                };
                let (value, keep) = self.with_frame(frame, |ev| {
                    let value = ev.eval(init);
                    ev.declare_var(&target.name.name, value.clone());
                    (value, ev.eval(body))
                });
                match keep {
                    Value::Boolean(true) if !value.is_null() => {
                        if name == "collectselectOne" {
                            return value;
                        }
                        push_flattened(&mut out, strict!(value));
                    }
                    Value::Invalid(m) => return Value::Invalid(m),
                    _ => {}
                }
            }
            return if name == "collectselectOne" { Value::Null } else { Value::collection(kind.collect_kind(), out) };
        }
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
            "xselect" | "selectOne" => {
                let mut out = Vec::new();
                for item in items {
                    let keep = match one(self, &item) {
                        Value::Boolean(b) => b,
                        Value::Type(t) => !item.is_null() && self.conforms(&item, &t, false),
                        Value::Null => false,
                        Value::Invalid(m) => return Value::Invalid(m),
                        other => return Value::invalid(format!("{name} body is not a Boolean: {other}")),
                    };
                    if keep {
                        if name == "selectOne" {
                            return item;
                        }
                        out.push(item);
                    }
                }
                if name == "selectOne" { Value::Null } else { Value::collection(kind, out) }
            }
            "xcollect" | "collectselect" => {
                let mut out = Vec::new();
                for item in &items {
                    let v = strict!(one(self, item));
                    push_flattened(&mut out, v);
                }
                out.retain(|v| !v.is_null());
                Value::collection(kind.collect_kind(), out)
            }
            "collectOne" | "collectselectOne" => {
                for item in &items {
                    let v = strict!(one(self, item));
                    if !v.is_null() {
                        return v;
                    }
                }
                Value::Null
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

// ------------------------------------------------------------------ Imperative OCL

impl Evaluator<'_> {
    /// Imperative OCL and the expressions of QVT Operational Mappings.
    fn eval_imperative(&mut self, e: &Expr) -> Value {
        match &e.kind {
            ExprKind::Block(items) => self
                .with_frame(Frame { vars: Vec::new(), implicit: None, operations: true, types: Vec::new() }, |ev| {
                    ev.eval_statements(items)
                }),
            // The value is the last variable's (`var a := var b := 1`).
            ExprKind::VarDecls(vars) => {
                let mut last = Value::Null;
                for v in vars {
                    let value = self.initial_value(v);
                    if let Value::Invalid(m) = &value {
                        let m = m.to_string();
                        self.report(v.span, m);
                    }
                    self.declare_var(&v.name.name, value.clone());
                    if let Some(Ok(t)) = v.ty.as_ref().map(|t| self.resolve_type(t))
                        && let Some(f) = self.frames.last_mut()
                    {
                        f.types.push((v.name.name.as_str().into(), t));
                    }
                    // Reported already: not again as the value of a statement.
                    last = if value.is_invalid() { Value::Null } else { value };
                }
                last
            }
            ExprKind::Assign { target, value, add } => {
                if let ExprKind::Resolve { late: true, .. } = &value.kind {
                    return self.late_assign(e.span, target, value, *add);
                }
                let v = self.eval(value);
                if self.control.is_some() {
                    return Value::Null;
                }
                if let Value::Invalid(m) = &v {
                    let m = m.to_string();
                    self.report(e.span, m);
                    return Value::Null;
                }
                match self.assign(target, v.clone(), *add) {
                    Ok(()) => v,
                    Err(m) => {
                        self.report(e.span, m);
                        Value::Null
                    }
                }
            }
            ExprKind::While { init, cond, body } => {
                self.with_frame(Frame { vars: Vec::new(), implicit: None, operations: true, types: Vec::new() }, |ev| {
                    if let Some(v) = init {
                        let value = ev.initial_value(v);
                        ev.declare_var(&v.name.name, value);
                    }
                    let mut n = 0u64;
                    loop {
                        match ev.eval(cond) {
                            Value::Boolean(true) => {}
                            Value::Boolean(false) => break,
                            other => {
                                if ev.control.is_none() {
                                    let m = match other {
                                        Value::Invalid(m) => m.to_string(),
                                        other => format!("'while' condition is not a Boolean: {other}"),
                                    };
                                    ev.report(cond.span, m);
                                }
                                break;
                            }
                        }
                        ev.eval(body);
                        if !ev.loop_continues() {
                            break;
                        }
                        n += 1;
                        if n >= MAX_ITERATIONS {
                            ev.report(e.span, "the loop was stopped after too many iterations");
                            break;
                        }
                    }
                    // With a declaration, the value is the variable's.
                    init.as_ref().and_then(|v| ev.lookup_var(&v.name.name)).unwrap_or(Value::Null)
                })
            }
            ExprKind::ForEach { source, iterators, condition, body, one } => {
                let items = match to_collection(self.eval(source)) {
                    Ok(c) => c,
                    Err(Value::Invalid(m)) => {
                        let m = m.to_string();
                        self.report(source.span, m);
                        return Value::Null;
                    }
                    Err(_) => return Value::Null,
                };
                for item in items {
                    let mut frame = Frame { vars: Vec::new(), implicit: None, operations: true, types: Vec::new() };
                    match iterators.first() {
                        Some(v) => frame.vars.push((v.name.name.as_str().into(), item)),
                        None => frame.implicit = Some(item),
                    }
                    let mut ran = false;
                    let go_on = self.with_frame(frame, |ev| {
                        if let Some(c) = condition {
                            match ev.eval(c) {
                                Value::Boolean(true) => {}
                                Value::Invalid(m) => {
                                    let m = m.to_string();
                                    ev.report(c.span, m);
                                    return true;
                                }
                                _ => return true,
                            }
                        }
                        ran = true;
                        ev.eval(body);
                        ev.loop_continues()
                    });
                    if !go_on || (*one && ran) {
                        break;
                    }
                }
                Value::Null
            }
            ExprKind::Switch { cases, default } => {
                for (c, b) in cases {
                    match self.eval(c) {
                        Value::Boolean(true) => return self.eval(b),
                        Value::Boolean(false) | Value::Null => {}
                        Value::Invalid(m) => return Value::Invalid(m),
                        other => return Value::invalid(format!("condition is not a Boolean: {other}")),
                    }
                }
                match default {
                    Some(d) => self.eval(d),
                    None => Value::Null,
                }
            }
            ExprKind::Return(value) => {
                let v = value.as_ref().map_or(Value::Null, |v| self.eval(v));
                if self.control.is_none() {
                    self.control = Some(Control::Return(v));
                }
                Value::Null
            }
            ExprKind::Break => {
                self.control = Some(Control::Break);
                Value::Null
            }
            ExprKind::Continue => {
                self.control = Some(Control::Continue);
                Value::Null
            }
            ExprKind::Compute { var, body } => {
                self.with_frame(Frame { vars: Vec::new(), implicit: None, operations: true, types: Vec::new() }, |ev| {
                    let value = ev.initial_value(var);
                    ev.declare_var(&var.name.name, value);
                    ev.eval(body);
                    ev.lookup_var(&var.name.name).unwrap_or(Value::Null)
                })
            }
            ExprKind::Log { args, when } => {
                if let Some(w) = when
                    && !matches!(self.eval(w), Value::Boolean(true))
                {
                    return Value::Null;
                }
                let message = self.log_message(args);
                if let Some(h) = self.host {
                    h.log(self, message);
                }
                Value::Null
            }
            ExprKind::Assert { severity, condition, log } => {
                let holds = self.eval(condition);
                if matches!(holds, Value::Boolean(true)) {
                    return Value::Null;
                }
                let level = severity.as_ref().map_or("error", |s| s.name.as_str());
                let detail = match (&holds, log.as_deref()) {
                    (_, Some(Expr { kind: ExprKind::Log { args, .. }, .. })) => self.log_message(args),
                    (Value::Invalid(m), None) => m.to_string(),
                    _ => format!("'{}' does not hold", condition_text(condition)),
                };
                let message = format!("assertion failed ({level}): {detail}");
                if level == "fatal" {
                    self.control = Some(Control::Abort(message));
                } else if let Some(h) = self.host {
                    h.log(self, message.clone());
                    self.report(e.span, message);
                } else {
                    self.report(e.span, message);
                }
                Value::Null
            }
            ExprKind::Object { var, ty, extent, body } => {
                let existing = var.as_ref().and_then(|v| self.lookup_var(&v.name));
                let object = match (&existing, ty) {
                    (Some(Value::Object(o)), _) => *o,
                    (_, Some(te)) => match self.create(te, extent.as_ref()) {
                        Ok(o) => o,
                        Err(m) => return Value::invalid(m),
                    },
                    // `object x : { … }` with `x` null: an object of `x`'s declared type.
                    _ => match var.as_ref().and_then(|v| self.declared_type(&v.name)) {
                        Some(Type::Classifier(c)) => match self.create_of(c, extent.as_ref()) {
                            Ok(o) => o,
                            Err(m) => return Value::invalid(m),
                        },
                        _ => return Value::invalid("the type of the object is not known"),
                    },
                };
                if let Some(v) = var
                    && !matches!(existing, Some(Value::Object(o)) if o == object)
                    && !self.set_var(&v.name, Value::Object(object))
                {
                    self.declare_var(&v.name, Value::Object(object));
                }
                self.with_implicit(Value::Object(object), |ev| ev.eval_statements(body));
                Value::Object(object)
            }
            ExprKind::New { ty, extent: None, args } if let Some(h) = self.host => {
                let mut values = Vec::new();
                for a in args {
                    values.push(strict!(self.eval(a)));
                }
                if let Some(v) = h.instantiate(self, ty, values) {
                    return v;
                }
                match self.create(ty, None) {
                    Ok(o) => Value::Object(o),
                    Err(m) => Value::invalid(m),
                }
            }
            ExprKind::New { ty, extent, .. } => match self.create(ty, extent.as_ref()) {
                Ok(o) => Value::Object(o),
                Err(m) => Value::invalid(m),
            },
            ExprKind::MapCall { source, strict, name, args, arrow } => {
                let src = source.as_ref().map(|s| self.eval(s));
                let mut values = Vec::new();
                for a in args {
                    values.push(strict!(self.eval(a)));
                }
                let Some(h) = self.host else {
                    return Value::invalid("mappings can only be called in transformations");
                };
                match src {
                    None => h.map(self, None, *strict, name, values),
                    Some(Value::Invalid(m)) => Value::Invalid(m),
                    Some(Value::Collection(c)) => {
                        let mut out = Vec::new();
                        for item in c.items.iter().filter(|i| !i.is_null()) {
                            match h.map(self, Some(item.clone()), *strict, name, values.clone()) {
                                Value::Null => {}
                                Value::Invalid(m) => return Value::Invalid(m),
                                v => out.push(v),
                            }
                        }
                        Value::collection(c.kind.collect_kind(), out)
                    }
                    Some(Value::Null) if *arrow => Value::collection(CollectionKind::Sequence, Vec::new()),
                    Some(Value::Null) => Value::Null,
                    Some(v) if *arrow => match h.map(self, Some(v), *strict, name, values) {
                        Value::Null => Value::collection(CollectionKind::Sequence, Vec::new()),
                        Value::Invalid(m) => Value::Invalid(m),
                        r => Value::collection(CollectionKind::Sequence, vec![r]),
                    },
                    Some(v) => h.map(self, Some(v), *strict, name, values),
                }
            }
            ExprKind::Resolve { source, one, inverse, mapping, var, ty, condition, .. } => {
                let src = match source {
                    Some(s) => Some(strict!(self.eval(s))),
                    None => None,
                };
                let ty = match ty.as_ref().map(|t| self.resolve_type(t)) {
                    Some(Err(m)) => return Value::invalid(m),
                    Some(Ok(t)) => Some(t),
                    None => None,
                };
                let request = ResolveRequest {
                    source: src,
                    inverse: *inverse,
                    mapping: mapping.as_ref().map(|m| m.iter().map(|i| i.name.clone()).collect()),
                };
                let var = var.as_ref().map(|v| Rc::from(v.name.as_str()));
                self.resolve_values(&request, *one, var.as_ref(), ty.as_ref(), condition.as_deref())
            }
            _ => Value::invalid("unsupported expression"),
        }
    }

    /// After the body of a loop: whether to go on (takes a `break` or `continue`).
    fn loop_continues(&mut self) -> bool {
        match self.control {
            Some(Control::Break) => {
                self.control = None;
                false
            }
            Some(Control::Continue) => {
                self.control = None;
                true
            }
            Some(_) => false,
            None => true,
        }
    }

    /// The initial value of a declared variable: its initializer, else an empty collection for
    /// collection types, else null.
    fn initial_value(&mut self, v: &VarDecl) -> Value {
        if let Some(i) = &v.init {
            return self.eval(i);
        }
        match v.ty.as_ref().map(|t| self.resolve_type(t)) {
            Some(Ok(Type::Collection(k, _))) => Value::collection(k, Vec::new()),
            _ => Value::Null,
        }
    }

    fn log_message(&mut self, args: &[Expr]) -> String {
        let mut parts = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let v = self.eval(a);
            let text = match (i, &v) {
                (0, _) => v.to_ocl_string(self.mm, &self.model),
                (1, _) => v.render(self.mm, &self.model),
                _ => continue,
            };
            parts.push(text);
        }
        parts.join(": ")
    }

    /// The declared type of a variable in scope (`var x : T`), if it has one.
    fn declared_type(&self, name: &str) -> Option<Type> {
        self.frames.iter().rev().find_map(|f| f.types.iter().rev().find(|(n, _)| &**n == name).map(|(_, t)| t.clone()))
    }

    /// Creates an object of the class `te` names (through the host).
    fn create(&mut self, te: &TypeExpr, extent: Option<&Ident>) -> Result<ObjectId, String> {
        match self.resolve_type(te)? {
            Type::Classifier(c) if self.mm.is_class(c) => self.create_of(c, extent),
            other => Err(format!("cannot create an object of type {}", other.display(self.mm))),
        }
    }

    /// Creates an object of a class (through the host).
    fn create_of(&mut self, class: oxy_ecore::ClassifierId, extent: Option<&Ident>) -> Result<ObjectId, String> {
        if self.mm.class_data(class).is_some_and(|d| d.is_abstract || d.is_interface) {
            return Err(format!("{} is abstract: its objects cannot be created", self.mm.classifier(class).name));
        }
        let Some(h) = self.host else { return Err("objects can only be created in transformations".into()) };
        h.create(self, class, extent.map(|x| x.name.as_str()))
    }

    /// Assigns (`add`: adds) a value to a variable or property.
    fn assign(&mut self, target: &Expr, value: Value, add: bool) -> Result<(), String> {
        match &target.kind {
            ExprKind::Name { path, .. } if path.len() == 1 => {
                let name = path[0].name.as_str();
                if let Some(current) = self.lookup_var(name) {
                    let new = if add { added(current, value) } else { value };
                    self.set_var(name, new);
                    return Ok(());
                }
                if let Some(h) = self.host
                    && let Some(current) = h.variable(self, name)
                {
                    let new = if add { added(current, value) } else { value };
                    if h.assign_variable(self, name, new) {
                        return Ok(());
                    }
                    return Err(format!("'{name}' cannot be assigned"));
                }
                let implicit: Vec<Value> = self.frames.iter().rev().filter_map(|f| f.implicit.clone()).collect();
                for v in &implicit {
                    if self.has_property(v, name) || self.host.is_some_and(|h| h.property(self, v, name).is_some()) {
                        return self.assign_property(v, name, value, add);
                    }
                }
                Err(format!("unknown variable or property '{name}'"))
            }
            ExprKind::Property { source, name, .. } => match self.eval(source) {
                Value::Invalid(m) => Err(m.to_string()),
                Value::Null => Err(format!("cannot assign '{}' of null", name.name)),
                src => self.assign_property(&src, &name.name, value, add),
            },
            _ => Err("only variables and properties can be assigned".into()),
        }
    }

    /// Assigns (`add`: adds) a value to a property of an object (or a property the host
    /// defines), as EMF does: containment and opposites stay consistent.
    pub fn assign_property(&mut self, target: &Value, name: &str, value: Value, add: bool) -> Result<(), String> {
        let mm = self.mm;
        let feature = match target {
            Value::Object(o) => mm.lookup_feature(self.model.class_of(*o), name),
            _ => None,
        };
        let (Value::Object(o), Some(f)) = (target, feature) else {
            if let Some(h) = self.host
                && let Some(r) = h.assign_property(self, target, name, value, add)
            {
                return r;
            }
            return Err(match target {
                Value::Object(o) => {
                    format!("unknown property '{name}' of class {}", mm.classifier(self.model.class_of(*o)).name)
                }
                other => format!("cannot assign property '{name}' of {other}"),
            });
        };
        let o = *o;
        if let Some(h) = self.host {
            h.writable(self, o)?;
        }
        let feat = mm.feature(f);
        if feat.derived {
            return Err(format!("'{name}' is derived: it cannot be assigned"));
        }
        let values: Vec<ModelValue> = match &value {
            Value::Invalid(m) => return Err(m.to_string()),
            Value::Null => Vec::new(),
            Value::Collection(c) => {
                c.items.iter().filter(|v| !v.is_null()).map(|v| self.to_model_value(f, v)).collect::<Result<_, _>>()?
            }
            v => vec![self.to_model_value(f, v)?],
        };
        if !feat.is_many() && values.len() > 1 {
            return Err(format!("cannot assign {} values to the single-valued property '{name}'", values.len()));
        }
        let Some(model) = self.model_mut() else { return Err("the model cannot be changed here".into()) };
        if feat.is_many() {
            if add {
                for v in values {
                    model.insert(mm, o, f, v);
                }
            } else {
                model.set(mm, o, f, values);
            }
        } else {
            match values.into_iter().next() {
                Some(v) => model.insert(mm, o, f, v),
                None => model.clear(mm, o, f),
            }
        }
        Ok(())
    }

    /// The value a feature stores for an OCL value.
    fn to_model_value(&self, feature: oxy_ecore::FeatureId, v: &Value) -> Result<ModelValue, String> {
        let mm = self.mm;
        let f = mm.feature(feature);
        let kind = f.ty.and_then(|t| mm.data_type_kind(t));
        let mismatch = || {
            let ty = f.ty.map_or("?".to_string(), |t| mm.classifier(t).name.clone());
            format!("{} does not fit the property '{}' of type {ty}", v.render(mm, &self.model), f.name)
        };
        Ok(match v {
            Value::Object(o) if f.is_reference() => {
                if f.ty.is_some_and(|t| !mm.conforms_to(self.model.class_of(*o), t)) {
                    return Err(mismatch());
                }
                ModelValue::Object(*o)
            }
            Value::Enum(e, l) if f.ty == Some(*e) => ModelValue::Enum(*e, *l as usize),
            Value::Boolean(b) if kind == Some(DataTypeKind::Boolean) => ModelValue::Boolean(*b),
            Value::Integer(i) if kind == Some(DataTypeKind::Real) => ModelValue::Real(*i as f64),
            Value::Integer(i) if kind == Some(DataTypeKind::Integer) => ModelValue::Integer(*i),
            Value::Unlimited if kind == Some(DataTypeKind::Integer) => ModelValue::Integer(-1),
            Value::Real(r) if kind == Some(DataTypeKind::Real) => ModelValue::Real(*r),
            Value::Real(r) if kind == Some(DataTypeKind::Integer) && r.fract() == 0.0 => ModelValue::Integer(*r as i64),
            Value::String(s) if matches!(kind, Some(DataTypeKind::String | DataTypeKind::Other)) => {
                ModelValue::String(s.to_string())
            }
            Value::Boolean(b) if kind == Some(DataTypeKind::Other) => ModelValue::String(b.to_string()),
            Value::Integer(i) if kind == Some(DataTypeKind::Other) => ModelValue::String(i.to_string()),
            _ => return Err(mismatch()),
        })
    }

    /// `target := late resolve...`: remembered by the host until the end of the
    /// transformation (or resolved now if it cannot).
    fn late_assign(&mut self, span: Span, target: &Expr, value: &Expr, add: bool) -> Value {
        let ExprKind::Resolve { source, one, inverse, mapping, var, ty, condition, .. } = &value.kind else {
            return Value::Null;
        };
        let (object, property) = match &target.kind {
            ExprKind::Property { source, name, .. } => match self.eval(source) {
                Value::Object(o) => (o, name.name.clone()),
                other => {
                    self.report(
                        span,
                        format!("a late resolve must be assigned to a property of an object, not of {other}"),
                    );
                    return Value::Null;
                }
            },
            ExprKind::Name { path, .. } if path.len() == 1 && self.lookup_var(&path[0].name).is_none() => {
                let name = path[0].name.as_str();
                let implicit: Vec<Value> = self.frames.iter().rev().filter_map(|f| f.implicit.clone()).collect();
                match implicit.into_iter().find(|v| self.has_property(v, name)) {
                    Some(Value::Object(o)) => (o, name.to_string()),
                    _ => {
                        self.report(span, format!("unknown property '{name}'"));
                        return Value::Null;
                    }
                }
            }
            _ => {
                self.report(span, "a late resolve must be assigned to a property");
                return Value::Null;
            }
        };
        let src = match source {
            Some(s) => match self.eval(s) {
                Value::Invalid(m) => {
                    self.report(span, m.to_string());
                    return Value::Null;
                }
                v => Some(v),
            },
            None => None,
        };
        let ty = match ty.as_ref().map(|t| self.resolve_type(t)) {
            Some(Err(m)) => {
                self.report(span, m);
                return Value::Null;
            }
            Some(Ok(t)) => Some(t),
            None => None,
        };
        let late = LateResolve {
            target: object,
            property,
            add,
            request: ResolveRequest {
                source: src,
                inverse: *inverse,
                mapping: mapping.as_ref().map(|m| m.iter().map(|i| i.name.clone()).collect()),
            },
            one: *one,
            var: var.as_ref().map(|v| Rc::from(v.name.as_str())),
            ty,
            condition: condition.as_ref().map(|c| Rc::new((**c).clone())),
            frames: self.frames.clone(),
            span,
        };
        let deferred = match self.host {
            Some(h) => h.defer(self, late.clone()),
            None => false,
        };
        if !deferred {
            self.complete_late(late);
        }
        Value::Null
    }

    /// Does the assignment of a `late resolve` (at the end of a transformation).
    pub fn complete_late(&mut self, late: LateResolve) {
        let saved = std::mem::replace(&mut self.frames, late.frames.clone());
        let value = self.resolve_values(
            &late.request,
            late.one,
            late.var.as_ref(),
            late.ty.as_ref(),
            late.condition.as_deref(),
        );
        self.frames = saved;
        let result = match value {
            Value::Invalid(m) => Err(m.to_string()),
            v => self.assign_property(&Value::Object(late.target), &late.property, v, late.add),
        };
        if let Err(m) = result {
            self.report(late.span, m);
        }
    }

    /// The objects a resolve yields: those the host offers for each source object, of the
    /// type and satisfying the condition (`one`: the first).
    fn resolve_values(
        &mut self,
        request: &ResolveRequest,
        one: bool,
        var: Option<&Rc<str>>,
        ty: Option<&Type>,
        condition: Option<&Expr>,
    ) -> Value {
        if let Some(Value::Collection(c)) = &request.source {
            let mut out = Vec::new();
            for item in c.items.iter().filter(|i| !i.is_null()) {
                let r = ResolveRequest { source: Some(item.clone()), ..request.clone() };
                match self.resolve_values(&r, one, var, ty, condition) {
                    Value::Invalid(m) => return Value::Invalid(m),
                    Value::Null => {}
                    v => push_flattened(&mut out, v),
                }
            }
            return Value::collection(CollectionKind::Sequence, out);
        }
        if matches!(request.source, Some(Value::Null)) {
            return if one { Value::Null } else { Value::collection(CollectionKind::Sequence, Vec::new()) };
        }
        let Some(h) = self.host else { return Value::invalid("resolve can only be used in transformations") };
        let candidates = h.resolve(self, request);
        let mut out = Vec::new();
        for c in candidates {
            if let Some(t) = ty
                && !self.conforms(&c, t, false)
            {
                continue;
            }
            if let Some(cond) = condition {
                let mut frame = Frame { vars: Vec::new(), implicit: None, operations: true, types: Vec::new() };
                match var {
                    Some(v) => frame.vars.push((v.clone(), c.clone())),
                    None => frame.implicit = Some(c.clone()),
                }
                match self.with_frame(frame, |ev| ev.eval(cond)) {
                    Value::Boolean(true) => {}
                    Value::Invalid(m) => return Value::Invalid(m),
                    _ => continue,
                }
            }
            if one {
                return c;
            }
            out.push(c);
        }
        if one { Value::Null } else { Value::collection(CollectionKind::Sequence, out) }
    }
}

/// `current += value` for variables: collections get the value (or its elements) added,
/// strings it appended; other values are replaced.
fn added(current: Value, value: Value) -> Value {
    match (current, value) {
        (Value::Collection(c), Value::Collection(more)) => {
            let mut items = c.items.clone();
            items.extend(more.items.iter().cloned());
            Value::collection(c.kind, items)
        }
        (Value::Collection(c), v) => {
            let mut items = c.items.clone();
            if !v.is_null() {
                items.push(v);
            }
            Value::collection(c.kind, items)
        }
        (Value::String(a), Value::String(b)) => Value::string(format!("{a}{b}")),
        (_, v) => v,
    }
}

/// A short text for an expression in messages.
fn condition_text(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Name { path, .. } => path.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join("::"),
        _ => "the condition".to_string(),
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
