//! Running a QVTr transformation in the direction of one parameter (the target):
//!
//! * the top relations are applied until nothing changes: for every binding of a relation's
//!   variables that matches the patterns of its source domains (the other parameters') and
//!   satisfies its `when` clause — whose relation calls look up the bindings recorded for the
//!   relations that hold —,
//! * the pattern of the target domain is matched in the target model with these bindings
//!   ("check before enforce"); if it does not match, it is enforced: objects bound already
//!   are changed, objects identified by a `key` are reused, the others created, and their
//!   properties set as the templates say (in check mode, the binding is reported instead);
//! * the binding is recorded (the trace), and the `where` clause applied: its relation calls
//!   apply those relations with the given roots, its `x = e` predicates bind variables
//!   (those that only need the source bindings are bound before the target is enforced).
//!
//! Matching a template binds its variable to each candidate object (all objects of the model
//! of its class, or the values of the property it is the value of), checks or binds its
//! properties (a property with a single unbound variable as its value binds it, to each
//! element of a many-valued property), and its condition. The OCL inside is evaluated by
//! `oxy-ocl` with the run as its host (for queries and relation calls in expressions).

use std::cell::{Cell, RefCell};

use oxy_ecore::{ClassifierId, FeatureId, Metamodel, Model, ModelValue, ObjectId};
use oxy_ocl::syntax::ast::{BinOp, Expr, ExprKind};
use oxy_ocl::{Environment, Evaluator, Host, Value};

use super::compile::{CCollection, CMember, CObject, CProp, CTemplate, CValue, Transformation, names_in};
use super::syntax::relation_call;
use crate::{Binding, Compiled, RunOptions, RunProblem, RunResult, library};

/// Variables and their values.
type Bindings = Vec<(String, Value)>;

fn get<'b>(b: &'b Bindings, name: &str) -> Option<&'b Value> {
    b.iter().find(|(n, _)| n == name).map(|(_, v)| v)
}

fn with(b: &Bindings, name: &str, v: Value) -> Bindings {
    let mut out = b.clone();
    out.push((name.to_string(), v));
    out
}

/// How often the top relations are applied at most (each round must add a binding).
const MAX_ROUNDS: usize = 1000;
/// How deeply relations may invoke each other.
const MAX_INVOCATIONS: usize = 200;

struct Run<'t> {
    t: &'t Transformation,
    /// The target parameter.
    target: usize,
    /// The resource of each parameter.
    extents: Vec<usize>,
    check_only: bool,
    /// The bindings of the relations that hold, by relation.
    traces: RefCell<Vec<Vec<Bindings>>>,
    /// Invocations in progress (against endless recursion).
    active: RefCell<Vec<(usize, Bindings)>>,
    violations: RefCell<Vec<String>>,
    problems: RefCell<Vec<RunProblem>>,
    applications: Cell<usize>,
    created: Cell<usize>,
}

pub(crate) fn run(
    t: &Transformation,
    compiled: &Compiled,
    mm: &Metamodel,
    env: &Environment,
    model: &mut Model,
    bindings: &[Binding],
    options: &RunOptions,
) -> Result<RunResult, String> {
    let targets = compiled.targets();
    let target_name = match (&options.target, targets.as_slice()) {
        (Some(t), _) => t.clone(),
        (None, [only]) => only.clone(),
        (None, []) => return Err("no relation enforces a model: name the target to check".into()),
        (None, _) => {
            return Err(format!(
                "the transformation can run in the direction of {}: choose one",
                targets.iter().map(|t| format!("'{t}'")).collect::<Vec<_>>().join(" or ")
            ));
        }
    };
    let Some(target) = t.params.iter().position(|p| p.name == target_name) else {
        return Err(format!("the transformation has no parameter '{target_name}'"));
    };
    if !options.check_only && !targets.contains(&target_name) {
        return Err(format!("no relation enforces '{target_name}': it can only be checked"));
    }
    let mut extents = Vec::new();
    for p in &t.params {
        match bindings.iter().find(|b| b.parameter == p.name) {
            Some(b) => extents.push(b.resource),
            None => return Err(format!("no model is bound to the parameter '{}'", p.name)),
        }
    }
    let run = Run {
        t,
        target,
        extents,
        check_only: options.check_only,
        traces: RefCell::new(vec![Vec::new(); t.relations.len()]),
        active: RefCell::new(Vec::new()),
        violations: RefCell::new(Vec::new()),
        problems: RefCell::new(Vec::new()),
        applications: Cell::new(0),
        created: Cell::new(0),
    };
    {
        let mut ev = Evaluator::new_mut(mm, model, env).with_host(&run).with_aliases(t.aliases.as_slice().into());
        let mut rounds = 0;
        loop {
            let before = run.traced();
            for (r, rel) in t.relations.iter().enumerate() {
                if !rel.syntax.is_top || rel.syntax.is_abstract || !run.touches_target(r) {
                    continue;
                }
                for m in run.match_sources(&mut ev, r, Vec::new()) {
                    run.enforce(&mut ev, r, m);
                }
            }
            rounds += 1;
            if run.traced() == before {
                break;
            }
            if rounds >= MAX_ROUNDS {
                run.problem(None, "the relations were applied too often; stopped");
                break;
            }
        }
        let errors = ev.take_errors();
        run.problems.borrow_mut().extend(errors.into_iter().map(|(span, message)| RunProblem {
            span: Some(span),
            message,
            unit: None,
        }));
    }
    Ok(RunResult {
        log: Vec::new(),
        problems: run.problems.take(),
        violations: run.violations.take(),
        applications: run.applications.get(),
        created: run.created.get(),
        aborted: None,
    })
}

impl Run<'_> {
    fn traced(&self) -> usize {
        self.traces.borrow().iter().map(Vec::len).sum()
    }

    fn problem(&self, span: Option<oxy_ocl::Span>, message: impl Into<String>) {
        let message = message.into();
        let mut problems = self.problems.borrow_mut();
        if !problems.iter().any(|p| p.span == span && p.message == message) {
            problems.push(RunProblem { span, message, unit: None });
        }
    }

    fn touches_target(&self, r: usize) -> bool {
        self.t.relations[r].domains.iter().any(|d| d.param == Some(self.target))
    }

    fn is_var(&self, r: usize, name: &str) -> bool {
        self.t.relations[r].vars.iter().any(|(n, _)| n == name)
    }

    /// Whether all variables of the relation used in `e` are bound.
    fn evaluable(&self, r: usize, e: &Expr, b: &Bindings) -> bool {
        let mut names = Vec::new();
        names_in(e, &mut names);
        names.iter().all(|n| !self.is_var(r, n) || get(b, n).is_some())
    }

    /// `e` if it is a single variable of the relation that is not bound yet.
    fn unbound_var<'e>(&self, r: usize, e: &'e Expr, b: &Bindings) -> Option<&'e str> {
        match &e.kind {
            ExprKind::Name { path, .. } if path.len() == 1 => {
                let n = path[0].name.as_str();
                (self.is_var(r, n) && get(b, n).is_none()).then_some(n)
            }
            _ => None,
        }
    }

    fn eval(&self, ev: &mut Evaluator<'_>, e: &Expr, b: &Bindings) -> Value {
        let vars: Vec<(&str, Value)> = b.iter().map(|(n, v)| (n.as_str(), v.clone())).collect();
        ev.evaluate(e, None, &vars)
    }

    fn render(&self, ev: &Evaluator<'_>, v: &Value) -> String {
        v.render(ev.metamodel(), ev.model())
    }

    // ---------------------------------------------------------------- matching

    /// The objects of a parameter's model of a class.
    fn candidates(&self, ev: &Evaluator<'_>, param: usize, class: Option<ClassifierId>) -> Vec<Value> {
        let mm = ev.metamodel();
        let model = ev.model();
        let resource = self.extents[param];
        model
            .object_ids()
            .filter(|o| {
                !model.is_deleted(*o)
                    && model.object(*o).resource == resource
                    && class.is_none_or(|c| mm.conforms_to(model.class_of(*o), c))
            })
            .map(Value::Object)
            .collect()
    }

    /// The values of a property of an object (for `opposite(C.f)`, the objects of `C` whose
    /// `f` holds it).
    fn values(&self, ev: &mut Evaluator<'_>, obj: ObjectId, p: &CProp) -> Vec<Value> {
        let Some(f) = p.feature else { return Vec::new() };
        let mm = ev.metamodel();
        if p.opposite {
            let model = ev.model();
            let owner = mm.feature(f).owner;
            return model
                .instances_of(mm, owner)
                .into_iter()
                .filter(|o| model.get(*o, f).contains(&ModelValue::Object(obj)))
                .map(Value::Object)
                .collect();
        }
        let name = mm.feature(f).name.clone();
        match ev.navigate(&Value::Object(obj), &name) {
            Value::Collection(c) => c.items.clone(),
            Value::Null | Value::Invalid(_) => Vec::new(),
            v => vec![v],
        }
    }

    fn is_many(&self, mm: &Metamodel, p: &CProp) -> bool {
        p.opposite || p.feature.is_some_and(|f| mm.feature(f).is_many())
    }

    /// The bindings extending `b` under which `o` matches one of the candidates.
    fn match_object(
        &self,
        ev: &mut Evaluator<'_>,
        r: usize,
        o: &CObject,
        candidates: Vec<Value>,
        b: &Bindings,
    ) -> Vec<Bindings> {
        let mm = ev.metamodel();
        let bound = get(b, &o.var).cloned();
        let mut out = Vec::new();
        for cand in candidates {
            if let Some(v) = &bound
                && !v.ocl_equals(&cand)
            {
                continue;
            }
            let Value::Object(obj) = cand else { continue };
            if o.class.is_some_and(|c| !mm.conforms_to(ev.model().class_of(obj), c)) {
                continue;
            }
            let b1 = if bound.is_some() { b.clone() } else { with(b, &o.var, cand.clone()) };
            let order: Vec<&CProp> = o.props.iter().collect();
            for b2 in self.match_props(ev, r, obj, &order, b1) {
                if let Some(g) = &o.guard
                    && !matches!(self.eval(ev, g, &b2), Value::Boolean(true))
                {
                    continue;
                }
                out.push(b2);
            }
        }
        out
    }

    /// Matches the properties, those that bind variables first.
    fn match_props(
        &self,
        ev: &mut Evaluator<'_>,
        r: usize,
        obj: ObjectId,
        props: &[&CProp],
        b: Bindings,
    ) -> Vec<Bindings> {
        if props.is_empty() {
            return vec![b];
        }
        let ready = |p: &CProp| match &p.value {
            CValue::Template(_) => true,
            CValue::Expr(e) => self.unbound_var(r, e, &b).is_some() || self.evaluable(r, e, &b),
        };
        let Some(i) = props.iter().position(|p| ready(p)) else {
            // Expressions over variables nothing binds: no match.
            return Vec::new();
        };
        let p = props[i];
        let rest: Vec<&CProp> = props.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, p)| *p).collect();
        let mut out = Vec::new();
        for b1 in self.match_prop(ev, r, obj, p, &b) {
            out.extend(self.match_props(ev, r, obj, &rest, b1));
        }
        out
    }

    fn match_prop(&self, ev: &mut Evaluator<'_>, r: usize, obj: ObjectId, p: &CProp, b: &Bindings) -> Vec<Bindings> {
        let mm = ev.metamodel();
        let values = self.values(ev, obj, p);
        let many = self.is_many(mm, p);
        match &p.value {
            CValue::Template(CTemplate::Object(n)) => self.match_object(ev, r, n, values, b),
            CValue::Template(CTemplate::Collection(c)) => {
                let kind = c.kind;
                self.match_collection(ev, r, c, Value::collection(kind, values), b)
            }
            CValue::Expr(e) => {
                if let Some(var) = self.unbound_var(r, e, b) {
                    if many {
                        return values.into_iter().map(|v| with(b, var, v)).collect();
                    }
                    let v = values.into_iter().next().unwrap_or(Value::Null);
                    return vec![with(b, var, v)];
                }
                let v = self.eval(ev, e, b);
                let holds = match (&v, many) {
                    (Value::Invalid(_), _) => false,
                    (Value::Collection(c), true) => c.items.iter().all(|i| values.iter().any(|x| x.ocl_equals(i))),
                    (v, true) => values.iter().any(|x| x.ocl_equals(v)),
                    (v, false) => values.first().unwrap_or(&Value::Null).ocl_equals(v),
                };
                if holds { vec![b.clone()] } else { Vec::new() }
            }
        }
    }

    fn match_collection(
        &self,
        ev: &mut Evaluator<'_>,
        r: usize,
        c: &CCollection,
        value: Value,
        b: &Bindings,
    ) -> Vec<Bindings> {
        let b0 = match get(b, &c.var) {
            Some(v) if !v.ocl_equals(&value) => return Vec::new(),
            Some(_) => b.clone(),
            None => with(b, &c.var, value.clone()),
        };
        let items = value.as_collection().map(|c| c.items.clone()).unwrap_or_default();
        let mut out = Vec::new();
        self.match_members(ev, r, c, &c.members, items, b0, &mut out);
        if let Some(g) = &c.guard {
            out.retain(|b| matches!(self.eval(ev, g, b), Value::Boolean(true)));
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn match_members(
        &self,
        ev: &mut Evaluator<'_>,
        r: usize,
        c: &CCollection,
        members: &[CMember],
        remaining: Vec<Value>,
        b: Bindings,
        out: &mut Vec<Bindings>,
    ) {
        let Some((first, rest)) = members.split_first() else {
            let b = match &c.rest {
                Some(name) => match get(&b, name) {
                    Some(v) if !v.ocl_equals(&Value::collection(c.kind, remaining.clone())) => return,
                    Some(_) => b,
                    None => with(&b, name, Value::collection(c.kind, remaining)),
                },
                None => b,
            };
            out.push(b);
            return;
        };
        for (i, item) in remaining.iter().enumerate() {
            let others: Vec<Value> =
                remaining.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, v)| v.clone()).collect();
            let extended = match first {
                CMember::Var(v) => match get(&b, v) {
                    Some(x) if !x.ocl_equals(item) => continue,
                    Some(_) => vec![b.clone()],
                    None => vec![with(&b, v, item.clone())],
                },
                CMember::Template(t) => match &**t {
                    CTemplate::Object(o) => self.match_object(ev, r, o, vec![item.clone()], &b),
                    CTemplate::Collection(n) => self.match_collection(ev, r, n, item.clone(), &b),
                },
            };
            for b1 in extended {
                self.match_members(ev, r, c, rest, others.clone(), b1, out);
            }
        }
    }

    /// The bindings under which the patterns of the domains of these parameters match.
    fn match_domains(
        &self,
        ev: &mut Evaluator<'_>,
        r: usize,
        target: bool,
        mut states: Vec<Bindings>,
    ) -> Vec<Bindings> {
        let rel = &self.t.relations[r];
        for d in &rel.domains {
            let Some(param) = d.param else { continue };
            if (param == self.target) != target {
                continue;
            }
            for p in &d.patterns {
                let mut next = Vec::new();
                for b in states {
                    match p {
                        CTemplate::Object(o) => {
                            let candidates = match get(&b, &o.var) {
                                Some(v) => vec![v.clone()],
                                None => self.candidates(ev, param, o.class),
                            };
                            next.extend(self.match_object(ev, r, o, candidates, &b));
                        }
                        CTemplate::Collection(c) => match get(&b, &c.var).cloned() {
                            Some(v) => next.extend(self.match_collection(ev, r, c, v, &b)),
                            None => {
                                let all = self.candidates(ev, param, None);
                                next.extend(self.match_collection(ev, r, c, Value::collection(c.kind, all), &b));
                            }
                        },
                    }
                }
                states = next;
                if states.is_empty() {
                    return states;
                }
            }
        }
        states
    }

    /// The bindings of the source domains and the `when` clause.
    fn match_sources(&self, ev: &mut Evaluator<'_>, r: usize, b: Bindings) -> Vec<Bindings> {
        let mut b = b;
        self.initialize(ev, r, &mut b);
        let states = self.match_domains(ev, r, false, vec![b]);
        match &self.t.relations[r].syntax.when {
            Some(when) => states.into_iter().flat_map(|b| self.predicates(ev, r, when, b)).collect(),
            None => states,
        }
    }

    /// Binds declared variables with an initial value.
    fn initialize(&self, ev: &mut Evaluator<'_>, r: usize, b: &mut Bindings) {
        for (name, init) in &self.t.relations[r].inits {
            if get(b, name).is_none() && self.evaluable(r, init, b) {
                let v = self.eval(ev, init, b);
                b.push((name.clone(), v));
            }
        }
    }

    fn predicates(&self, ev: &mut Evaluator<'_>, r: usize, preds: &[Expr], b: Bindings) -> Vec<Bindings> {
        let mut states = vec![b];
        for p in preds {
            let mut next = Vec::new();
            for b in states {
                next.extend(self.predicate(ev, r, p, b));
            }
            states = next;
            if states.is_empty() {
                break;
            }
        }
        states
    }

    /// `x = e` with `x` an unbound variable: the variable and the expression.
    fn assignment<'e>(&self, r: usize, p: &'e Expr, b: &Bindings) -> Option<(&'e str, &'e Expr)> {
        let ExprKind::Binary { op: BinOp::Eq, lhs, rhs } = &p.kind else { return None };
        if let Some(v) = self.unbound_var(r, lhs, b) {
            return Some((v, rhs));
        }
        self.unbound_var(r, rhs, b).map(|v| (v, &**lhs))
    }

    /// A predicate of a `when` clause: a relation call (looked up in the traces), an
    /// assignment of an unbound variable, or a condition.
    fn predicate(&self, ev: &mut Evaluator<'_>, r: usize, p: &Expr, b: Bindings) -> Vec<Bindings> {
        if let Some((name, args, negated)) = relation_call(p)
            && let Some(called) = self.t.relation(&name.name)
        {
            let found = self.unify(ev, r, called, args, &b);
            return match (negated, found.is_empty()) {
                (false, _) => found,
                (true, true) => vec![b],
                (true, false) => Vec::new(),
            };
        }
        if let Some((var, e)) = self.assignment(r, p, &b)
            && self.evaluable(r, e, &b)
        {
            return match self.eval(ev, e, &b) {
                Value::Invalid(_) => Vec::new(),
                v => vec![with(&b, var, v)],
            };
        }
        if !self.evaluable(r, p, &b) {
            self.problem(Some(p.span), "the predicate uses variables nothing binds");
            return Vec::new();
        }
        match self.eval(ev, p, &b) {
            Value::Boolean(true) => vec![b],
            Value::Invalid(m) => {
                self.problem(Some(p.span), m.to_string());
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// The bindings under which a call of relation `called` matches a recorded binding of it
    /// (or of a relation overriding it).
    fn unify(&self, ev: &mut Evaluator<'_>, r: usize, called: usize, args: &[Expr], b: &Bindings) -> Vec<Bindings> {
        let mut out = Vec::new();
        let mut implementations = self.t.implementations(called);
        if !implementations.contains(&called) {
            implementations.push(called);
        }
        for i in implementations {
            let roots = &self.t.relations[i].roots;
            let records: Vec<Bindings> = self.traces.borrow()[i].clone();
            'records: for rec in records {
                let mut b1 = b.clone();
                for (arg, root) in args.iter().zip(roots) {
                    let Some(value) = get(&rec, root).cloned() else { continue 'records };
                    if let Some(var) = self.unbound_var(r, arg, &b1) {
                        b1.push((var.to_string(), value));
                    } else if self.evaluable(r, arg, &b1) {
                        if !self.eval(ev, arg, &b1).ocl_equals(&value) {
                            continue 'records;
                        }
                    } else {
                        continue 'records;
                    }
                }
                out.push(b1);
            }
        }
        out
    }

    // ---------------------------------------------------------------- enforcing

    /// Makes the target domain hold for a binding of the source domains (or, in check mode,
    /// reports that it does not), records it and applies the `where` clause.
    fn enforce(&self, ev: &mut Evaluator<'_>, r: usize, b: Bindings) {
        let rel = &self.t.relations[r];
        // Applied for this binding already: the `where` clause again (relations it calls may
        // apply now that others hold), nothing else.
        let known: Option<Bindings> = self.traces.borrow()[r]
            .iter()
            .find(|rec| b.iter().all(|(n, v)| get(rec, n).is_some_and(|x| x.ocl_equals(v))))
            .cloned();
        if let Some(rec) = known {
            self.where_clause(ev, r, rec);
            return;
        }
        let mut b = b;
        self.pre_assign(ev, r, &mut b);
        for d in rel.domains.iter().filter(|d| d.param == Some(self.target)) {
            for (v, e) in &d.default_values {
                if get(&b, v).is_none() && self.evaluable(r, e, &b) {
                    let value = self.eval(ev, e, &b);
                    b.push((v.clone(), value));
                }
            }
        }
        let matched = self.match_domains(ev, r, true, vec![b.clone()]).into_iter().next();
        let b = match matched {
            Some(m) => m,
            None if self.check_only => {
                self.violation(ev, r, &b);
                return;
            }
            None => {
                let mut b = b;
                let resource = self.extents[self.target];
                for d in rel.domains.iter().filter(|d| d.param == Some(self.target)) {
                    for p in &d.patterns {
                        if let Err((span, e)) = self.enforce_template(ev, r, p, &mut b, resource, None) {
                            self.problem(Some(span), format!("{}: {e}", rel.syntax.name.name));
                            return;
                        }
                    }
                }
                b
            }
        };
        self.applications.set(self.applications.get() + 1);
        self.traces.borrow_mut()[r].push(b.clone());
        self.where_clause(ev, r, b);
    }

    fn violation(&self, ev: &Evaluator<'_>, r: usize, b: &Bindings) {
        let rel = &self.t.relations[r];
        let roots: Vec<String> =
            rel.roots.iter().filter_map(|n| get(b, n).map(|v| format!("{n} = {}", self.render(ev, v)))).collect();
        let message = format!("relation {} does not hold for {}", rel.syntax.name.name, roots.join(", "));
        let mut violations = self.violations.borrow_mut();
        if !violations.contains(&message) {
            violations.push(message);
        }
    }

    /// Binds the variables `where` assigns from what is bound already (before the target is
    /// enforced, since its templates may use them).
    fn pre_assign(&self, ev: &mut Evaluator<'_>, r: usize, b: &mut Bindings) {
        let Some(preds) = &self.t.relations[r].syntax.where_ else { return };
        loop {
            let mut changed = false;
            for p in preds {
                if relation_call(p).is_some_and(|(n, _, _)| self.t.relation(&n.name).is_some()) {
                    continue;
                }
                if let Some((var, e)) = self.assignment(r, p, b)
                    && self.evaluable(r, e, b)
                {
                    let v = self.eval(ev, e, b);
                    if !v.is_invalid() {
                        b.push((var.to_string(), v));
                        changed = true;
                    }
                }
            }
            if !changed {
                return;
            }
        }
    }

    /// The `where` clause: relation calls are applied, assignments bind, conditions are
    /// checked.
    fn where_clause(&self, ev: &mut Evaluator<'_>, r: usize, mut b: Bindings) {
        let Some(preds) = &self.t.relations[r].syntax.where_ else { return };
        let name = &self.t.relations[r].syntax.name.name;
        for p in preds {
            if let Some((called, args, negated)) = relation_call(p)
                && let Some(c) = self.t.relation(&called.name)
            {
                if negated {
                    self.problem(Some(p.span), "a negated relation call cannot be enforced");
                    continue;
                }
                let mut values = Vec::new();
                for a in args {
                    if !self.evaluable(r, a, &b) {
                        self.problem(Some(a.span), format!("{name}: the argument uses variables nothing binds"));
                        return;
                    }
                    values.push(self.eval(ev, a, &b));
                }
                self.invoke(ev, c, values);
                continue;
            }
            if let Some((var, e)) = self.assignment(r, p, &b)
                && self.evaluable(r, e, &b)
            {
                let v = self.eval(ev, e, &b);
                b.push((var.to_string(), v));
                continue;
            }
            if !self.evaluable(r, p, &b) {
                self.problem(Some(p.span), format!("{name}: the predicate uses variables nothing binds"));
                continue;
            }
            if !matches!(self.eval(ev, p, &b), Value::Boolean(true)) {
                self.problem(Some(p.span), format!("{name}: the where clause does not hold"));
            }
        }
    }

    /// Applies a relation to the given values of its roots (from a `where` clause): the
    /// first of its implementations (it, or the relations overriding it) that matches.
    fn invoke(&self, ev: &mut Evaluator<'_>, called: usize, args: Vec<Value>) {
        // Invocations nest up to `MAX_INVOCATIONS` deep, whatever the thread's stack.
        stacker::maybe_grow(128 * 1024, 4 * 1024 * 1024, || self.invoke_inner(ev, called, args));
    }

    fn invoke_inner(&self, ev: &mut Evaluator<'_>, called: usize, args: Vec<Value>) {
        for i in self.t.implementations(called) {
            let rel = &self.t.relations[i];
            if rel.roots.len() != args.len() || !self.roots_fit(ev, i, &args) {
                continue;
            }
            let b: Bindings = rel.roots.iter().cloned().zip(args.iter().cloned()).collect();
            {
                let mut active = self.active.borrow_mut();
                if active.iter().any(|(x, ab)| {
                    *x == i && ab.len() == b.len() && ab.iter().zip(&b).all(|(p, q)| p.1.ocl_equals(&q.1))
                }) {
                    return;
                }
                if active.len() >= MAX_INVOCATIONS {
                    drop(active);
                    self.problem(Some(rel.syntax.name.span), "relations invoke each other too deeply");
                    return;
                }
                active.push((i, b.clone()));
            }
            let matches = self.match_sources(ev, i, b);
            let applies = !matches.is_empty();
            for m in matches {
                self.enforce(ev, i, m);
            }
            self.active.borrow_mut().pop();
            if applies {
                return;
            }
        }
    }

    /// Whether the values fit the types of the relation's root templates.
    fn roots_fit(&self, ev: &Evaluator<'_>, r: usize, args: &[Value]) -> bool {
        let rel = &self.t.relations[r];
        rel.roots.iter().zip(args).all(|(root, v)| match rel.vars.iter().find(|(n, _)| n == root) {
            Some((_, ty)) => v.is_null() || ev.conforms(v, ty, false),
            None => true,
        })
    }

    /// Makes a template hold: the object bound to its variable, one a key identifies, or a
    /// new one, with its properties set.
    fn enforce_template(
        &self,
        ev: &mut Evaluator<'_>,
        r: usize,
        t: &CTemplate,
        b: &mut Bindings,
        resource: usize,
        parent: Option<(ObjectId, FeatureId)>,
    ) -> Result<Value, (oxy_ocl::Span, String)> {
        let mm = ev.metamodel();
        match t {
            CTemplate::Object(o) => {
                let obj = match get(b, &o.var) {
                    Some(Value::Object(x)) => *x,
                    Some(Value::Null) | None => {
                        let found = self.key_lookup(ev, r, o, b, parent);
                        let obj = match found {
                            Some(x) => x,
                            None => self.create(ev, o, resource).map_err(|e| (o.span, e))?,
                        };
                        b.retain(|(n, _)| *n != o.var);
                        b.push((o.var.clone(), Value::Object(obj)));
                        obj
                    }
                    Some(other) => {
                        return Err((o.span, format!("'{}' is {}, not an object", o.var, self.render(ev, other))));
                    }
                };
                for p in &o.props {
                    let Some(f) = p.feature else { continue };
                    let value = match &p.value {
                        CValue::Template(n) => {
                            let link = if p.opposite { None } else { Some((obj, f)) };
                            self.enforce_template(ev, r, n, b, resource, link)?
                        }
                        CValue::Expr(e) => {
                            if let Some(var) = self.unbound_var(r, e, b) {
                                // Already there (an object that was reused): bind it.
                                match self.values(ev, obj, p).into_iter().next() {
                                    Some(v) => {
                                        b.push((var.to_string(), v));
                                        continue;
                                    }
                                    None => {
                                        return Err((
                                            p.span,
                                            format!("nothing binds '{var}' to set '{}'", mm.feature(f).name),
                                        ));
                                    }
                                }
                            }
                            if !self.evaluable(r, e, b) {
                                return Err((
                                    p.span,
                                    format!("the value of '{}' uses variables nothing binds", mm.feature(f).name),
                                ));
                            }
                            match self.eval(ev, e, b) {
                                Value::Invalid(m) => return Err((p.span, m.to_string())),
                                v => v,
                            }
                        }
                    };
                    self.set(ev, obj, p, value).map_err(|e| (p.span, e))?;
                }
                if let Some(g) = &o.guard
                    && !matches!(self.eval(ev, g, b), Value::Boolean(true))
                {
                    return Err((o.span, format!("the condition of '{}' does not hold", o.var)));
                }
                Ok(Value::Object(obj))
            }
            CTemplate::Collection(c) => {
                let mut items = Vec::new();
                for m in &c.members {
                    match m {
                        CMember::Var(v) => match get(b, v) {
                            Some(x) => items.push(x.clone()),
                            None => return Err((c.span, format!("nothing binds '{v}'"))),
                        },
                        CMember::Template(t) => items.push(self.enforce_template(ev, r, t, b, resource, None)?),
                    }
                }
                if let Some(rest) = &c.rest
                    && let Some(Value::Collection(more)) = get(b, rest)
                {
                    items.extend(more.items.iter().cloned());
                }
                let v = Value::collection(c.kind, items);
                if get(b, &c.var).is_none() {
                    b.push((c.var.clone(), v.clone()));
                }
                Ok(v)
            }
        }
    }

    fn create(&self, ev: &mut Evaluator<'_>, o: &CObject, resource: usize) -> Result<ObjectId, String> {
        let mm = ev.metamodel();
        let Some(class) = o.class else { return Err(format!("the class of '{}' is not known", o.var)) };
        if mm.class_data(class).is_some_and(|d| d.is_abstract || d.is_interface) {
            return Err(format!("{} is abstract: '{}' cannot be created", mm.classifier(class).name, o.var));
        }
        let Some(model) = ev.model_mut() else { return Err("the model cannot be changed".into()) };
        self.created.set(self.created.get() + 1);
        Ok(model.create_object(mm, class, resource, None))
    }

    /// Sets (or, for many-valued properties, adds) the value of a property.
    fn set(&self, ev: &mut Evaluator<'_>, obj: ObjectId, p: &CProp, value: Value) -> Result<(), String> {
        let mm = ev.metamodel();
        let Some(f) = p.feature else { return Ok(()) };
        let name = mm.feature(f).name.clone();
        if p.opposite {
            let holders = match value {
                Value::Collection(c) => c.items.clone(),
                Value::Null => Vec::new(),
                v => vec![v],
            };
            for h in holders {
                ev.assign_property(&h, &name, Value::Object(obj), true)?;
            }
            return Ok(());
        }
        let current = self.values(ev, obj, p);
        if mm.feature(f).is_many() {
            let missing: Vec<Value> = match value {
                Value::Collection(c) => {
                    c.items.iter().filter(|i| !current.iter().any(|x| x.ocl_equals(i))).cloned().collect()
                }
                Value::Null => Vec::new(),
                v if current.iter().any(|x| x.ocl_equals(&v)) => Vec::new(),
                v => vec![v],
            };
            if missing.is_empty() {
                return Ok(());
            }
            return ev.assign_property(
                &Value::Object(obj),
                &name,
                Value::collection(oxy_ocl::types::CollectionKind::Sequence, missing),
                true,
            );
        }
        if current.first().is_some_and(|c| c.ocl_equals(&value)) {
            return Ok(());
        }
        ev.assign_property(&Value::Object(obj), &name, value, false)
    }

    /// The object of the target model a key identifies for a template: one of the key's class
    /// whose key properties have the values the template gives them (or its parent, for the
    /// property opposite to the one the template is the value of).
    fn key_lookup(
        &self,
        ev: &mut Evaluator<'_>,
        r: usize,
        o: &CObject,
        b: &Bindings,
        parent: Option<(ObjectId, FeatureId)>,
    ) -> Option<ObjectId> {
        let mm = ev.metamodel();
        let class = o.class?;
        let key = self.t.keys.iter().find(|k| mm.conforms_to(class, k.class))?;
        let mut wanted: Vec<(FeatureId, Value)> = Vec::new();
        for &kf in &key.features {
            let from_parent =
                parent.filter(|(_, pf)| mm.feature(*pf).opposite() == Some(kf)).map(|(po, _)| Value::Object(po));
            let from_template =
                o.props.iter().find(|p| p.feature == Some(kf) && !p.opposite).and_then(|p| match &p.value {
                    CValue::Expr(e) if self.evaluable(r, e, b) => Some(self.eval(ev, e, b)),
                    CValue::Template(t) => get(b, t.var()).cloned(),
                    CValue::Expr(_) => None,
                });
            wanted.push((kf, from_template.or(from_parent)?));
        }
        let mut holders: Vec<(FeatureId, Value)> = Vec::new();
        for &of in &key.opposites {
            let from_parent = parent.filter(|(_, pf)| *pf == of).map(|(po, _)| Value::Object(po));
            let from_template =
                o.props.iter().find(|p| p.feature == Some(of) && p.opposite).and_then(|p| match &p.value {
                    CValue::Expr(e) if self.evaluable(r, e, b) => Some(self.eval(ev, e, b)),
                    CValue::Template(t) => get(b, t.var()).cloned(),
                    CValue::Expr(_) => None,
                });
            holders.push((of, from_template.or(from_parent)?));
        }
        let target = self.target;
        self.candidates(ev, target, Some(key.class)).into_iter().find_map(|cand| {
            let Value::Object(x) = cand else { return None };
            let fits = wanted.iter().all(|(f, v)| {
                let name = mm.feature(*f).name.clone();
                match ev.navigate(&Value::Object(x), &name) {
                    Value::Collection(c) => match v {
                        Value::Collection(w) => w.items.iter().all(|i| c.items.iter().any(|y| y.ocl_equals(i))),
                        v => c.items.iter().any(|y| y.ocl_equals(v)),
                    },
                    current => current.ocl_equals(v),
                }
            }) && holders.iter().all(|(f, h)| match h {
                Value::Object(ho) => ev.model().get(*ho, *f).contains(&ModelValue::Object(x)),
                _ => false,
            });
            fits.then_some(x)
        })
    }
}

impl Host for Run<'_> {
    fn operation(&self, ev: &mut Evaluator<'_>, source: Option<&Value>, name: &str, args: &[Value]) -> Option<Value> {
        if source.is_none() {
            if let Some(q) = self.t.query(name, args.len()) {
                let q = &self.t.queries[q];
                let body = q.syntax.body.as_ref()?;
                let vars: Vec<(&str, Value)> =
                    q.params.iter().map(|(n, _)| n.as_str()).zip(args.iter().cloned()).collect();
                return Some(
                    ev.call_frame(None, &vars, |ev| ev.eval(body))
                        .unwrap_or_else(|| Value::invalid("maximum call depth exceeded (infinite recursion?)")),
                );
            }
            if let Some(r) = self.t.relation(name) {
                // A relation used as a condition: whether it holds for these values.
                let mut implementations = self.t.implementations(r);
                implementations.push(r);
                let traces = self.traces.borrow();
                let holds = implementations.iter().any(|i| {
                    let roots = &self.t.relations[*i].roots;
                    traces[*i].iter().any(|rec| {
                        roots.len() == args.len()
                            && roots.iter().zip(args).all(|(n, v)| get(rec, n).is_some_and(|x| x.ocl_equals(v)))
                    })
                });
                return Some(Value::Boolean(holds));
            }
        }
        library::call_operation(ev, source, name, args)
    }
}
