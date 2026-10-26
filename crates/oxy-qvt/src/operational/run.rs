//! Running a QVTo transformation: the [`Runner`] is the OCL evaluator's [`Host`]. It binds
//! the model parameters to resources of the model, calls mappings (guard, `init`, implicit
//! creation of the result, the trace record, `population`, `end`), helpers and queries, keeps
//! the trace that `resolve` searches and the assignments of `late resolve` that are done at the
//! end, and the values of properties.
//!
//! As in Eclipse QVTo, a mapping called again with the same source and arguments returns the
//! result of the first call; objects are created in the output model whose model type has
//! their class (or the one named with `@`); objects of input models cannot be changed.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use oxy_ecore::{ClassifierId, Metamodel, Model, ObjectId};
use oxy_ocl::syntax::ast::Ident;
use oxy_ocl::syntax::parse_expression;
use oxy_ocl::types::CollectionKind;
use oxy_ocl::{Environment, Evaluator, Host, LateResolve, ResolveRequest, Type, Value};

use super::compile::Transformation;
use super::syntax::{Direction, HelperBody, PropertyKind};
use crate::{Binding, Compiled, RunOptions, RunProblem, RunResult, library};

/// A mapping application: what it was called with and what it produced.
struct Trace {
    mapping: usize,
    source: Option<Value>,
    args: Vec<Value>,
    result: Value,
}

/// A model parameter bound to a resource.
struct Extent {
    name: String,
    resource: usize,
    direction: Direction,
}

struct Runner<'t> {
    t: &'t Transformation,
    extents: Vec<Extent>,
    traces: RefCell<Vec<Trace>>,
    /// Values of the transformation's properties (and configuration properties).
    properties: RefCell<HashMap<String, Value>>,
    /// Values of intermediate properties, by object.
    intermediate: RefCell<HashMap<(ObjectId, String), Value>>,
    late: RefCell<Vec<LateResolve>>,
    log: RefCell<Vec<String>>,
    problems: RefCell<Vec<RunProblem>>,
    applications: Cell<usize>,
    created: Cell<usize>,
}

pub(crate) fn run(
    t: &Transformation,
    _compiled: &Compiled,
    mm: &Metamodel,
    env: &Environment,
    model: &mut Model,
    bindings: &[Binding],
    options: &RunOptions,
) -> RunResult {
    let extents = t
        .params
        .iter()
        .filter_map(|p| {
            let b = bindings.iter().find(|b| b.parameter == p.name)?;
            Some(Extent { name: p.name.clone(), resource: b.resource, direction: p.direction })
        })
        .collect();
    let runner = Runner {
        t,
        extents,
        traces: RefCell::new(Vec::new()),
        properties: RefCell::new(HashMap::new()),
        intermediate: RefCell::new(HashMap::new()),
        late: RefCell::new(Vec::new()),
        log: RefCell::new(Vec::new()),
        problems: RefCell::new(Vec::new()),
        applications: Cell::new(0),
        created: Cell::new(0),
    };
    let aborted = {
        let mut ev = Evaluator::new_mut(mm, model, env).with_host(&runner).with_aliases(&t.aliases);
        runner.init_properties(&mut ev, options);
        if let Some(main) = &t.main {
            let args = runner.entry_args(&ev, &t.main_params);
            let vars: Vec<(&str, Value)> = t.main_params.iter().map(|p| p.name.as_str()).zip(args).collect();
            ev.call_frame(None, &vars, |ev| {
                if let (Value::Invalid(m), Some(last)) = (ev.eval_statements(&main.body), main.body.last()) {
                    ev.report(last.span, m.to_string());
                }
                ev.take_return();
            });
        } else if let Some(i) = t.entry_mapping {
            let args = runner.entry_args(&ev, &t.mappings[i].1.params);
            let result = runner.invoke(&mut ev, i, None, args, false);
            if let Value::Invalid(m) = &result {
                runner.problem(Some(t.mappings[i].0.name.span), m.to_string());
            }
            // Old style: the output is the result of `main`.
            if t.implicit_output
                && let Some(e) = runner.extent("result")
                && let Some(model) = ev.model_mut()
            {
                let roots = match result {
                    Value::Object(o) => vec![o],
                    Value::Collection(c) => {
                        c.items.iter().filter_map(|v| if let Value::Object(o) = v { Some(*o) } else { None }).collect()
                    }
                    _ => Vec::new(),
                };
                let roots = roots.into_iter().filter(|o| model.container(*o).is_none()).collect();
                model.set_roots(e.resource, roots);
            }
        }
        // `late resolve`s, in order (they may defer further ones).
        for _ in 0..16 {
            let pending = std::mem::take(&mut *runner.late.borrow_mut());
            if pending.is_empty() {
                break;
            }
            for l in pending {
                ev.complete_late(l);
            }
        }
        let errors = ev.take_errors();
        runner
            .problems
            .borrow_mut()
            .extend(errors.into_iter().map(|(span, message)| RunProblem { span: Some(span), message }));
        ev.aborted().map(str::to_string)
    };
    RunResult {
        log: runner.log.take(),
        problems: runner.problems.take(),
        violations: Vec::new(),
        applications: runner.applications.get(),
        created: runner.created.get(),
        aborted,
    }
}

impl Runner<'_> {
    /// `this`: the transformation.
    fn is_this(&self, v: &Value) -> bool {
        matches!(v, Value::Model(m) if **m == *self.t.name)
    }

    /// The arguments of an entry with parameters (`main(in p : T)`): for each, the first root
    /// object of the input models of its type that no earlier parameter took.
    fn entry_args(&self, ev: &Evaluator<'_>, params: &[super::compile::Sig]) -> Vec<Value> {
        let model = ev.model();
        let mut used: Vec<Value> = Vec::new();
        params
            .iter()
            .map(|p| {
                let found = self
                    .extents
                    .iter()
                    .filter(|e| e.direction != Direction::Out)
                    .flat_map(|e| model.resources()[e.resource].roots.iter().map(|o| Value::Object(*o)))
                    .find(|v| ev.conforms(v, &p.ty, false) && !used.iter().any(|u| u.ocl_equals(v)))
                    .unwrap_or(Value::Null);
                used.push(found.clone());
                found
            })
            .collect()
    }

    fn extent(&self, name: &str) -> Option<&Extent> {
        self.extents.iter().find(|e| e.name == name)
    }

    fn problem(&self, span: Option<oxy_ocl::Span>, message: impl Into<String>) {
        self.problems.borrow_mut().push(RunProblem { span, message: message.into() });
    }

    /// The values of the properties: configuration properties from the options (else their
    /// value), the others from their value.
    fn init_properties(&self, ev: &mut Evaluator<'_>, options: &RunOptions) {
        for (p, s) in &self.t.properties {
            if p.kind == PropertyKind::Intermediate {
                continue;
            }
            let configured = options.configuration.iter().find(|(n, _)| *n == p.name.name).map(|(_, v)| v.clone());
            let value = match (configured, &p.init) {
                (Some(text), _) => {
                    let (e, diags) = parse_expression(&text);
                    let v = if diags.is_empty() { ev.evaluate(&e, None, &[]) } else { Value::string(&text) };
                    match (&s.ty, v) {
                        (Type::String, Value::Invalid(_)) => Value::string(&text),
                        (_, v) => v,
                    }
                }
                (None, Some(init)) => ev.call_frame(None, &[], |ev| ev.eval(init)).unwrap_or(Value::Null),
                (None, None) => match &s.ty {
                    Type::Collection(k, _) => Value::collection(*k, Vec::new()),
                    _ => Value::Null,
                },
            };
            if let Value::Invalid(m) = &value {
                self.problem(Some(p.span), format!("the value of '{}': {m}", p.name.name));
            }
            self.properties.borrow_mut().insert(p.name.name.clone(), value);
        }
    }

    fn call_helper(&self, ev: &mut Evaluator<'_>, i: usize, source: Option<&Value>, args: &[Value]) -> Value {
        let (h, s) = &self.t.helpers[i];
        let vars: Vec<(&str, Value)> = s.params.iter().map(|p| p.name.as_str()).zip(args.iter().cloned()).collect();
        let self_value = if s.context.is_some() { source.cloned() } else { None };
        ev.call_frame(self_value, &vars, |ev| match &h.body {
            HelperBody::Expr(e) => ev.eval(e),
            HelperBody::Block(items, _) => {
                let v = ev.eval_statements(items);
                ev.take_return().unwrap_or(v)
            }
            HelperBody::None => Value::invalid(format!("'{}' has no body", h.name.name)),
        })
        .unwrap_or_else(|| Value::invalid("maximum call depth exceeded (infinite recursion?)"))
    }

    /// The recorded result of an earlier call of the mapping with these values.
    fn traced(&self, mapping: usize, source: &Option<Value>, args: &[Value]) -> Option<Value> {
        let same = |a: &Option<Value>, b: &Option<Value>| match (a, b) {
            (Some(x), Some(y)) => x.ocl_equals(y),
            (None, None) => true,
            _ => false,
        };
        self.traces
            .borrow()
            .iter()
            .find(|t| {
                t.mapping == mapping
                    && same(&t.source, source)
                    && t.args.len() == args.len()
                    && t.args.iter().zip(args).all(|(a, b)| a.ocl_equals(b))
            })
            .map(|t| t.result.clone())
    }

    /// Whether the guard of a mapping holds for these values.
    fn guard_holds(&self, ev: &mut Evaluator<'_>, i: usize, source: &Option<Value>, args: &[Value]) -> bool {
        let (m, s) = &self.t.mappings[i];
        if let (Some(ctx), Some(src)) = (&s.context, source)
            && !ev.conforms(src, ctx, false)
        {
            return false;
        }
        let Some(when) = &m.when else { return true };
        let vars: Vec<(&str, Value)> = s.params.iter().map(|p| p.name.as_str()).zip(args.iter().cloned()).collect();
        let self_value = if s.context.is_some() { source.clone() } else { None };
        ev.call_frame(self_value, &vars, |ev| when.iter().all(|c| matches!(ev.eval(c), Value::Boolean(true))))
            .unwrap_or(false)
    }

    /// Calls mapping `i`: through its disjuncts, from the trace, or by executing it.
    fn invoke(&self, ev: &mut Evaluator<'_>, i: usize, source: Option<Value>, args: Vec<Value>, strict: bool) -> Value {
        let (m, s) = &self.t.mappings[i];
        if !s.disjuncts.is_empty() {
            for &d in &s.disjuncts {
                if self.t.mappings[d].1.params.len() == args.len() && self.guard_holds(ev, d, &source, &args) {
                    return self.invoke(ev, d, source, args, strict);
                }
            }
            return if strict {
                Value::invalid(format!("no mapping that '{}' disjoins applies", m.name.name))
            } else {
                Value::Null
            };
        }
        if m.body.is_none() {
            return Value::invalid(format!("the abstract mapping '{}' cannot be called", m.name.name));
        }
        if let Some(r) = self.traced(i, &source, &args) {
            return r;
        }
        let mut vars: Vec<(&str, Value)> = s.params.iter().map(|p| p.name.as_str()).zip(args.iter().cloned()).collect();
        if !s.results.is_empty() {
            vars.push(("result", Value::Null));
            for r in &s.results {
                if let Some(n) = &r.name {
                    vars.push((n.as_str(), Value::Null));
                }
            }
        }
        let self_value = if s.context.is_some() { source.clone() } else { None };
        ev.call_frame(self_value, &vars, |ev| self.execute(ev, i, source, args, strict))
            .unwrap_or_else(|| Value::invalid("maximum call depth exceeded (infinite recursion?)"))
    }

    /// The sections of a mapping, in its call frame.
    fn execute(
        &self,
        ev: &mut Evaluator<'_>,
        i: usize,
        source: Option<Value>,
        args: Vec<Value>,
        strict: bool,
    ) -> Value {
        let (m, s) = &self.t.mappings[i];
        // As in Eclipse QVTo, a guard that is not `true` (also `null` or `invalid`) makes the
        // mapping not applicable.
        if let Some(when) = &m.when {
            for c in when {
                match ev.eval(c) {
                    Value::Boolean(true) => {}
                    Value::Invalid(msg) if strict => {
                        return Value::invalid(format!("the guard of '{}' is invalid (xmap): {msg}", m.name.name));
                    }
                    _ if strict => {
                        return Value::invalid(format!("the guard of '{}' does not hold (xmap)", m.name.name));
                    }
                    _ => return Value::Null,
                }
            }
        }
        self.applications.set(self.applications.get() + 1);
        if let Some(init) = &m.init {
            ev.eval_statements(init);
            if let Some(v) = ev.take_return() {
                return v;
            }
        }
        if let Err(e) = self.instantiate(ev, i) {
            return Value::invalid(e);
        }
        let result = self.result(ev, i);
        let trace = self.traces.borrow().len();
        self.traces.borrow_mut().push(Trace {
            mapping: i,
            source: source.clone(),
            args: args.clone(),
            result: result.clone(),
        });
        for &h in &s.inherits {
            self.inherit(ev, h, &result);
        }
        self.populate(ev, i, &result);
        if let Some(end) = &m.end {
            ev.eval_statements(end);
        }
        if let Some(v) = ev.take_return() {
            self.set_result(ev, i, v);
        }
        let result = self.result(ev, i);
        if let Some(t) = self.traces.borrow_mut().get_mut(trace) {
            t.result = result.clone();
        }
        for &g in &s.merges {
            self.invoke(ev, g, source.clone(), args.clone(), false);
        }
        if let Some(conditions) = &m.where_ {
            for c in conditions {
                if !matches!(ev.eval(c), Value::Boolean(true)) {
                    ev.report(c.span, format!("the postcondition (where) of '{}' does not hold", m.name.name));
                }
            }
        }
        result
    }

    /// Creates the objects of the results that are still null.
    fn instantiate(&self, ev: &mut Evaluator<'_>, i: usize) -> Result<(), String> {
        let s = &self.t.mappings[i].1;
        let mm = ev.metamodel();
        for r in &s.results {
            let var = r.name.as_deref().unwrap_or("result");
            if !matches!(ev.lookup_var(var), Some(Value::Null) | None) {
                continue;
            }
            let Type::Classifier(class) = r.ty else { continue };
            if !mm.is_class(class) || mm.class_data(class).is_some_and(|d| d.is_abstract || d.is_interface) {
                continue;
            }
            let o = self.create(ev, class, r.extent.as_deref())?;
            ev.set_var(var, Value::Object(o));
            ev.set_var("result", Value::Object(o));
        }
        Ok(())
    }

    fn result(&self, ev: &Evaluator<'_>, i: usize) -> Value {
        let s = &self.t.mappings[i].1;
        match s.results.as_slice() {
            [] => Value::Null,
            [r] => ev.lookup_var(r.name.as_deref().unwrap_or("result")).unwrap_or(Value::Null),
            rs => Value::tuple(
                rs.iter()
                    .enumerate()
                    .map(|(n, r)| {
                        let name = r.name.clone().unwrap_or_else(|| format!("r{n}"));
                        let v = ev.lookup_var(&name).unwrap_or(Value::Null);
                        (name.into(), v)
                    })
                    .collect(),
            ),
        }
    }

    fn set_result(&self, ev: &mut Evaluator<'_>, i: usize, v: Value) {
        let s = &self.t.mappings[i].1;
        if let [r] = s.results.as_slice()
            && let Some(n) = &r.name
        {
            ev.set_var(n, v.clone());
        }
        ev.set_var("result", v);
    }

    /// The population section: with the result object (or, for `inout` mappings without a
    /// result, `self`) as the implicit source of its names — unless it is written with the
    /// `population` keyword.
    fn populate(&self, ev: &mut Evaluator<'_>, i: usize, result: &Value) {
        let (m, s) = &self.t.mappings[i];
        let target = match result {
            _ if m.population_keyword => None,
            Value::Object(_) => Some(result.clone()),
            _ if s.results.is_empty() && m.direction == Some(Direction::InOut) => ev.lookup_var("self"),
            _ => None,
        };
        match target {
            Some(t) => {
                ev.with_implicit(t, |ev| ev.eval_statements(&m.population));
            }
            None => {
                ev.eval_statements(&m.population);
            }
        }
    }

    /// Runs the population and end sections of an inherited mapping on the same result.
    fn inherit(&self, ev: &mut Evaluator<'_>, h: usize, result: &Value) {
        let (m, s) = &self.t.mappings[h];
        let self_value = ev.lookup_var("self");
        let mut vars: Vec<(String, Value)> =
            s.params.iter().map(|p| (p.name.clone(), ev.lookup_var(&p.name).unwrap_or(Value::Null))).collect();
        vars.push(("result".into(), result.clone()));
        let vars: Vec<(&str, Value)> = vars.iter().map(|(n, v)| (n.as_str(), v.clone())).collect();
        ev.call_frame(self_value, &vars, |ev| {
            if let Some(init) = &m.init {
                ev.eval_statements(init);
            }
            self.populate(ev, h, result);
            if let Some(end) = &m.end {
                ev.eval_statements(end);
            }
            ev.take_return();
        });
    }

    fn create(&self, ev: &mut Evaluator<'_>, class: ClassifierId, extent: Option<&str>) -> Result<ObjectId, String> {
        let mm = ev.metamodel();
        let resource = match extent {
            Some(name) => match self.extent(name) {
                Some(e) if e.direction != Direction::In => e.resource,
                Some(_) => return Err(format!("objects cannot be created in the input model '{name}'")),
                None => return Err(format!("unknown model parameter '{name}'")),
            },
            None => match self.t.default_extent(mm, class).and_then(|p| self.extent(&p.name)) {
                Some(e) => e.resource,
                None => {
                    return Err(format!("no output model to create {} in", mm.classifier(class).name));
                }
            },
        };
        let Some(model) = ev.model_mut() else { return Err("the model cannot be changed".into()) };
        let o = model.create_object(mm, class, resource, None);
        self.created.set(self.created.get() + 1);
        Ok(o)
    }

    /// `uml.rootObjects()`, `uml.objectsOfKind(T)`, ...
    fn model_operation(&self, ev: &mut Evaluator<'_>, name: &str, op: &str, args: &[Value]) -> Option<Value> {
        let extent = self.extent(name)?;
        let resource = extent.resource;
        let mm = ev.metamodel();
        let model = ev.model();
        let set = |items: Vec<Value>| Value::collection(CollectionKind::Set, items);
        let objects = || -> Vec<ObjectId> {
            model.object_ids().filter(|o| !model.is_deleted(*o) && model.object(*o).resource == resource).collect()
        };
        Some(match (op, args) {
            ("rootObjects", []) => set(model.resources()[resource].roots.iter().map(|o| Value::Object(*o)).collect()),
            ("objects", []) => set(objects().into_iter().map(Value::Object).collect()),
            ("objectsOfType" | "objectsOfKind", [Value::Type(t)]) => {
                let exact = op == "objectsOfType";
                let items: Vec<Value> = objects().into_iter().map(Value::Object).collect();
                set(items.into_iter().filter(|v| ev.conforms(v, t, exact)).collect())
            }
            ("removeElement", [Value::Object(o)]) => {
                if let Err(e) = self.writable(ev, *o) {
                    return Some(Value::invalid(e));
                }
                if let Some(model) = ev.model_mut() {
                    model.delete(mm, *o);
                }
                Value::Null
            }
            _ => return None,
        })
    }

    /// The intermediate property `name` of objects of `class`, if there is one.
    fn intermediate_property(&self, mm: &Metamodel, class: ClassifierId, name: &str) -> Option<usize> {
        self.t.properties.iter().position(|(p, s)| {
            p.kind == PropertyKind::Intermediate
                && p.name.name == name
                && s.context.is_some_and(|c| mm.conforms_to(class, c))
        })
    }
}

impl Host for Runner<'_> {
    fn variable(&self, _ev: &mut Evaluator<'_>, name: &str) -> Option<Value> {
        if name == "this" {
            return Some(Value::Model(self.t.name.as_str().into()));
        }
        if self.extent(name).is_some() {
            return Some(Value::Model(name.into()));
        }
        self.properties.borrow().get(name).cloned()
    }

    fn assign_variable(&self, _ev: &mut Evaluator<'_>, name: &str, value: Value) -> bool {
        let module = self.t.properties.iter().any(|(p, _)| p.kind == PropertyKind::Module && p.name.name == name);
        if module {
            self.properties.borrow_mut().insert(name.to_string(), value);
        }
        module
    }

    fn operation(&self, ev: &mut Evaluator<'_>, source: Option<&Value>, name: &str, args: &[Value]) -> Option<Value> {
        let source = source.filter(|v| !self.is_this(v));
        // Contextual helpers are not applied to null (the call is invalid, as in OCL).
        if source.is_some_and(Value::is_null) {
            return None;
        }
        let src_type = source.map(|v| v.type_of(ev.model()));
        let arg_types: Vec<Type> = args.iter().map(|a| a.type_of(ev.model())).collect();
        if let Some(Ok(i)) = self.t.select_helper(ev.metamodel(), src_type.as_ref(), name, &arg_types) {
            return Some(self.call_helper(ev, i, source, args));
        }
        if let Some(Value::Model(m)) = source {
            return self.model_operation(ev, m, name, args);
        }
        // Copies are made in the output models (not in the original's, which may be an input).
        if let (Some(Value::Object(o)), "deepclone" | "clone", []) = (source, name, args) {
            let mut create = |ev: &mut Evaluator<'_>, class| self.create(ev, class, None);
            return Some(match library::clone_object(ev, *o, name == "deepclone", &mut create) {
                Ok(c) => Value::Object(c),
                Err(e) => Value::invalid(e),
            });
        }
        if let Some(v) = library::call_operation(ev, source, name, args) {
            return Some(v);
        }
        // A mapping called like an operation (`self.toTable()`): `map`.
        let ident = [Ident { name: name.to_string(), span: oxy_ocl::Span::new(0, 0) }];
        let i = self.t.select_mapping(ev.metamodel(), src_type.as_ref(), &ident, &arg_types).ok()?;
        Some(self.invoke(ev, i, source.cloned(), args.to_vec(), false))
    }

    fn property(&self, ev: &mut Evaluator<'_>, source: &Value, name: &str) -> Option<Value> {
        match source {
            Value::Model(m) if **m == *self.t.name => self.properties.borrow().get(name).cloned(),
            Value::Object(o) => {
                let class = ev.model().class_of(*o);
                let Some(i) = self.intermediate_property(ev.metamodel(), class, name) else {
                    // `_abstract`: a feature named like a keyword of QVTo.
                    let real = super::compile::escaped(name)?;
                    ev.metamodel().lookup_feature(class, real)?;
                    return Some(ev.navigate(source, real));
                };
                if let Some(v) = self.intermediate.borrow().get(&(*o, name.to_string())) {
                    return Some(v.clone());
                }
                let (p, s) = &self.t.properties[i];
                let v = match (&p.init, &s.ty) {
                    (Some(init), _) => {
                        ev.call_frame(Some(source.clone()), &[], |ev| ev.eval(init)).unwrap_or(Value::Null)
                    }
                    (None, Type::Collection(k, _)) => Value::collection(*k, Vec::new()),
                    (None, _) => Value::Null,
                };
                self.intermediate.borrow_mut().insert((*o, name.to_string()), v.clone());
                Some(v)
            }
            _ => None,
        }
    }

    fn assign_property(
        &self,
        ev: &mut Evaluator<'_>,
        target: &Value,
        name: &str,
        value: Value,
        add: bool,
    ) -> Option<Result<(), String>> {
        if let Value::Object(o) = target
            && let Some(real) = super::compile::escaped(name)
            && self.intermediate_property(ev.metamodel(), ev.model().class_of(*o), name).is_none()
            && ev.metamodel().lookup_feature(ev.model().class_of(*o), real).is_some()
        {
            return Some(ev.assign_property(target, real, value, add));
        }
        let current = self.property(ev, target, name)?;
        let new = if add {
            match (current, value) {
                (Value::Collection(c), Value::Collection(more)) => {
                    Value::collection(c.kind, c.items.iter().chain(more.items.iter()).cloned().collect())
                }
                (Value::Collection(c), v) => {
                    Value::collection(c.kind, c.items.iter().cloned().chain(std::iter::once(v)).collect())
                }
                (_, v) => v,
            }
        } else {
            value
        };
        match target {
            Value::Model(_) => {
                if self.t.properties.iter().any(|(p, _)| p.kind == PropertyKind::Configuration && p.name.name == name) {
                    return Some(Err(format!("the configuration property '{name}' cannot be assigned")));
                }
                self.properties.borrow_mut().insert(name.to_string(), new);
            }
            Value::Object(o) => {
                self.intermediate.borrow_mut().insert((*o, name.to_string()), new);
            }
            _ => return None,
        }
        Some(Ok(()))
    }

    fn map(
        &self,
        ev: &mut Evaluator<'_>,
        source: Option<Value>,
        strict: bool,
        name: &[Ident],
        args: Vec<Value>,
    ) -> Value {
        let source = source.filter(|v| !self.is_this(v));
        let src_type = source.as_ref().map(|v| v.type_of(ev.model()));
        let arg_types: Vec<Type> = args.iter().map(|a| a.type_of(ev.model())).collect();
        match self.t.select_mapping(ev.metamodel(), src_type.as_ref(), name, &arg_types) {
            Ok(i) => self.invoke(ev, i, source, args, strict),
            Err(e) if strict || source.is_none() => Value::invalid(e),
            // Not defined for objects of this class: not applicable.
            Err(_) => Value::Null,
        }
    }

    fn create(&self, ev: &mut Evaluator<'_>, class: ClassifierId, extent: Option<&str>) -> Result<ObjectId, String> {
        Runner::create(self, ev, class, extent)
    }

    fn resolve(&self, ev: &mut Evaluator<'_>, request: &ResolveRequest) -> Vec<Value> {
        let mm = ev.metamodel();
        let applies = |mapping: usize| match &request.mapping {
            None => true,
            Some(path) => {
                let (m, s) = &self.t.mappings[mapping];
                let Some((last, qualifier)) = path.split_last() else { return false };
                m.name.name == *last
                    && match (qualifier.last(), &s.context) {
                        (None, _) => true,
                        (Some(q), Some(Type::Classifier(c))) => mm.classifier(*c).name == *q,
                        _ => false,
                    }
            }
        };
        let mut out = Vec::new();
        let push = |out: &mut Vec<Value>, v: &Value| match v {
            Value::Null => {}
            Value::Collection(c) => out.extend(c.items.iter().filter(|i| !i.is_null()).cloned()),
            Value::Tuple(parts) => out.extend(parts.iter().map(|(_, v)| v.clone()).filter(|v| !v.is_null())),
            v => out.push(v.clone()),
        };
        for t in self.traces.borrow().iter().filter(|t| applies(t.mapping)) {
            // The source of a mapping without a context is its first argument.
            let source = t.source.as_ref().or(t.args.first());
            if request.inverse {
                let produced = |x: &Value| match &t.result {
                    Value::Collection(c) => c.items.iter().any(|i| i.ocl_equals(x)),
                    r => r.ocl_equals(x),
                };
                match (&request.source, source) {
                    (Some(x), Some(s)) if produced(x) => out.push(s.clone()),
                    (None, Some(s)) => out.push(s.clone()),
                    _ => {}
                }
            } else {
                match (&request.source, source) {
                    (Some(x), Some(s)) if s.ocl_equals(x) => push(&mut out, &t.result),
                    (None, _) => push(&mut out, &t.result),
                    _ => {}
                }
            }
        }
        let mut seen = std::collections::HashSet::new();
        out.retain(|v| seen.insert(v.key()));
        out
    }

    fn defer(&self, _ev: &mut Evaluator<'_>, late: LateResolve) -> bool {
        self.late.borrow_mut().push(late);
        true
    }

    fn log(&self, _ev: &mut Evaluator<'_>, message: String) {
        self.log.borrow_mut().push(message);
    }

    fn writable(&self, ev: &Evaluator<'_>, object: ObjectId) -> Result<(), String> {
        let resource = ev.model().object(object).resource;
        match self.extents.iter().find(|e| e.resource == resource) {
            Some(e) if e.direction == Direction::In => {
                Err(format!("objects of the input model '{}' cannot be changed", e.name))
            }
            _ => Ok(()),
        }
    }
}
