//! Applying rules and units to graphs.
//!
//! Matching is a backtracking search over the left-hand side of a rule (preserved and deleted
//! nodes), extending from nodes already matched along links. A match must satisfy the
//! attribute values of its nodes (an unbound parameter used as a value is bound by the match),
//! the rule's attribute conditions, its application condition and — with `checkDangling` — must
//! not leave links of deleted nodes behind. Matching is injective unless the rule says
//! `injectiveMatching false`.
//!
//! A rule with multi-rules is applied with all matches of each multi-rule that extend the
//! kernel match (amalgamation), as one step.
//!
//! Units follow Henshin's semantics: a sequence applies its steps in order (strict: fails, and
//! with rollback undoes everything, when one fails); `independent` applies one of its steps
//! chosen at random, `priority` the first applicable one; `if`, `for` and `while` are as
//! expected. Calls pass the calling unit's parameters by position; the values of `OUT` and
//! `INOUT` parameters are passed back.

use std::cell::RefCell;
use std::collections::HashSet;

use oxy_ecore::{ClassifierId, FeatureId, Metamodel, ModelValue};

use crate::graph::{Graph, Link, NodeId};
use crate::module::{Callable, Condition, Module, NodeKind, Rule, Step};
use crate::syntax::ParamKind;
use crate::value::{CExpr, EvalError, Value};

/// At most this many rule applications per unit execution (so `while` loops end).
pub const MAX_APPLICATIONS: usize = 10_000;
const MAX_CALL_DEPTH: usize = 200;

/// A match of a rule.
#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    /// The node matched by each node of the rule (`None` for created nodes).
    pub nodes: Vec<Option<NodeId>>,
    /// The parameter values (given, or bound by the match).
    pub params: Vec<Option<Value>>,
    /// For each multi-rule, all its matches extending this match.
    pub multi: Vec<Vec<Match>>,
}

impl Match {
    /// The nodes matched by the rule and its multi-rules.
    pub fn matched_nodes(&self) -> Vec<NodeId> {
        let mut out: Vec<NodeId> = self.nodes.iter().flatten().copied().collect();
        for m in self.multi.iter().flatten() {
            out.extend(m.matched_nodes().into_iter().filter(|n| !out.contains(n)).collect::<Vec<_>>());
        }
        out
    }
}

/// One application of a rule, described before it changed the graph.
#[derive(Clone, Debug, PartialEq)]
pub struct Application {
    pub rule: String,
    /// Parameters with values (except `VAR`s), rendered.
    pub params: Vec<(String, String)>,
    /// Rule nodes and the nodes they matched (multi-rule nodes as `multiRule.node`).
    pub nodes: Vec<(String, NodeId)>,
    /// The kernel rule's matched nodes, rendered: rule node, label.
    pub matched: Vec<(String, String)>,
    /// The number of multi-rule matches applied with it.
    pub instances: usize,
}

impl Application {
    /// `rule(param=value, ...)`.
    pub fn label(&self) -> String {
        if self.params.is_empty() {
            return self.rule.clone();
        }
        let params: Vec<String> = self.params.iter().map(|(n, v)| format!("{n}={v}")).collect();
        format!("{}({})", self.rule, params.join(", "))
    }

    /// The label, or for rules without parameters the rule and the objects it matched:
    /// `rule(node: Label, ...)`.
    pub fn detailed_label(&self) -> String {
        if !self.params.is_empty() || self.matched.is_empty() {
            return self.label();
        }
        let nodes: Vec<String> = self.matched.iter().map(|(n, l)| format!("{n}: {l}")).collect();
        format!("{}({})", self.rule, nodes.join(", "))
    }
}

/// The outcome of executing a rule or unit.
#[derive(Clone, Debug, PartialEq)]
pub struct Execution {
    pub success: bool,
    /// The parameter values afterwards.
    pub params: Vec<Option<Value>>,
    /// The rule applications, in order (only those that took effect).
    pub trace: Vec<Application>,
}

/// Where a change of the plan applies.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    Old(NodeId),
    New(usize),
}

/// The changes a match makes.
#[derive(Default, Debug)]
struct Plan {
    delete_edges: Vec<Link>,
    delete_nodes: Vec<NodeId>,
    create_nodes: Vec<ClassifierId>,
    attributes: Vec<(Target, FeatureId, Option<ModelValue>)>,
    create_edges: Vec<(Target, FeatureId, Target)>,
    /// Parameters naming created nodes of the (top-level) rule: parameter, created node.
    created_params: Vec<(usize, usize)>,
}

/// Nodes to match, with the links and attribute values they must have.
struct Pattern<'r> {
    nodes: Vec<PatternNode<'r>>,
    edges: Vec<(usize, FeatureId, usize)>,
    /// Attribute values required of slots matched before.
    checks: Vec<(usize, FeatureId, &'r CExpr)>,
}

struct PatternNode<'r> {
    slot: usize,
    class: ClassifierId,
    matches: &'r [(FeatureId, CExpr)],
    param: Option<usize>,
}

/// A partial match.
#[derive(Clone)]
struct State {
    slots: Vec<Option<NodeId>>,
    params: Vec<Option<Value>>,
}

enum Check {
    Holds,
    Fails,
    /// Needs parameters that are not bound yet.
    Pending,
}

/// Applies the rules and units of a module.
pub struct Engine<'a> {
    pub mm: &'a Metamodel,
    pub module: &'a Module,
    /// Problems met while matching or applying (e.g. an expression that failed), each once.
    messages: RefCell<Vec<String>>,
}

impl<'a> Engine<'a> {
    pub fn new(mm: &'a Metamodel, module: &'a Module) -> Self {
        Engine { mm, module, messages: RefCell::new(Vec::new()) }
    }

    /// The problems met so far.
    pub fn messages(&self) -> Vec<String> {
        self.messages.borrow().clone()
    }

    fn note(&self, rule: &Rule, message: impl AsRef<str>) {
        let m = format!("rule {}: {}", rule.name, message.as_ref());
        let mut messages = self.messages.borrow_mut();
        if !messages.contains(&m) {
            messages.push(m);
        }
    }

    // ------------------------------------------------------------------ matching

    /// Matches of a rule (at most `limit`), with the given parameter values (`None`: to be
    /// bound by matching). `params` has one entry per parameter of the rule.
    pub fn matches(&self, graph: &Graph, rule: usize, params: &[Option<Value>], limit: usize) -> Vec<Match> {
        let Some(r) = self.module.rules.get(rule) else { return Vec::new() };
        let mut params = params.to_vec();
        params.resize(r.params.len(), None);
        let mut state = State { slots: vec![None; r.slots], params };
        let mut out = Vec::new();
        if limit == 0 {
            return out;
        }
        self.rule_matches(graph, r, &mut state, &mut |m| {
            out.push(m);
            out.len() >= limit
        });
        out
    }

    /// Finds the matches of `rule` extending `state`; `found` returns `true` to stop.
    fn rule_matches(
        &self,
        graph: &Graph,
        rule: &Rule,
        state: &mut State,
        found: &mut dyn FnMut(Match) -> bool,
    ) -> bool {
        let pattern = Pattern {
            nodes: rule
                .nodes
                .iter()
                .enumerate()
                .skip(rule.inherited)
                .filter(|(_, n)| n.kind.is_lhs())
                .map(|(i, n)| PatternNode { slot: i, class: n.class, matches: &n.matches, param: n.param })
                .collect(),
            edges: rule.edges.iter().filter(|e| e.kind.is_lhs()).map(|e| (e.source, e.feature, e.target)).collect(),
            checks: rule.nodes[..rule.inherited]
                .iter()
                .enumerate()
                .filter(|(_, n)| n.kind.is_lhs())
                .flat_map(|(i, n)| n.matches.iter().map(move |(f, e)| (i, *f, e)))
                .collect(),
        };
        self.search(graph, &pattern, rule.injective, state, &mut |st| {
            if !self.conditions_hold(rule, st) || !self.holds(graph, rule, &rule.condition, st) {
                return false;
            }
            let mut multi = Vec::new();
            for m in &rule.multi_rules {
                let mut sub = State { slots: vec![None; m.slots], params: st.params.clone() };
                for (i, slot) in st.slots.iter().take(rule.nodes.len()).enumerate() {
                    sub.slots[i] = *slot;
                }
                let mut matches = Vec::new();
                self.rule_matches(graph, m, &mut sub, &mut |mm| {
                    matches.push(mm);
                    false
                });
                multi.push(matches);
            }
            let m = Match { nodes: st.slots[..rule.nodes.len()].to_vec(), params: st.params.clone(), multi };
            // Only complete matches (kernel rules, after amalgamation) are checked for dangling
            // links and evaluated.
            if rule.inherited == 0 {
                let mut plan = Plan::default();
                if let Err(e) = self.plan(rule, &m, &[], &mut plan, true) {
                    self.note(rule, e);
                    return false;
                }
                if rule.check_dangling && self.dangling(graph, &plan) {
                    return false;
                }
            }
            found(m)
        })
    }

    /// Whether the attribute conditions of a rule hold.
    fn conditions_hold(&self, rule: &Rule, st: &State) -> bool {
        for c in &rule.conditions {
            match c.eval(self.mm, &st.params) {
                Ok(Value::Bool(true)) => {}
                Ok(Value::Bool(false)) => return false,
                Ok(v) => {
                    self.note(rule, format!("a condition evaluated to {}, not a Boolean", v.to_text(self.mm)));
                    return false;
                }
                Err(EvalError::Unbound(p)) => {
                    self.note(rule, format!("parameter '{}' has no value", rule.params[p].name));
                    return false;
                }
                Err(EvalError::Failed(m)) => {
                    self.note(rule, m);
                    return false;
                }
            }
        }
        true
    }

    fn holds(&self, graph: &Graph, rule: &Rule, c: &Condition, st: &mut State) -> bool {
        match c {
            Condition::True => true,
            Condition::Graph(g) => {
                let pattern = Pattern {
                    nodes: g
                        .nodes
                        .iter()
                        .map(|n| PatternNode { slot: n.slot, class: n.class, matches: &n.matches, param: None })
                        .collect(),
                    edges: g.edges.clone(),
                    checks: g.checks.iter().map(|(s, f, e)| (*s, *f, e)).collect(),
                };
                self.search(graph, &pattern, rule.injective, st, &mut |st| self.holds(graph, rule, &g.condition, st))
            }
            Condition::Not(c) => !self.holds(graph, rule, c, st),
            Condition::And(cs) => cs.iter().all(|c| self.holds(graph, rule, c, st)),
            Condition::Or(a, b) => self.holds(graph, rule, a, st) || self.holds(graph, rule, b, st),
            Condition::Xor(a, b) => self.holds(graph, rule, a, st) != self.holds(graph, rule, b, st),
        }
    }

    /// Searches the extensions of `state` that match `pattern`; `accept` returns `true` to stop
    /// (then `search` returns `true`). `state` is restored afterwards.
    fn search(
        &self,
        graph: &Graph,
        pattern: &Pattern,
        injective: bool,
        state: &mut State,
        accept: &mut dyn FnMut(&mut State) -> bool,
    ) -> bool {
        let mut bound = Vec::new();
        let mut pending = Vec::new();
        let ok = pattern.edges.iter().all(|(s, f, t)| match (state.slots[*s], state.slots[*t]) {
            (Some(a), Some(b)) => graph.has_link(a, *f, b),
            _ => true,
        }) && pattern.checks.iter().all(|(slot, f, e)| match state.slots[*slot] {
            Some(n) => match self.check(graph, state, n, *f, e, &mut bound) {
                Check::Holds => true,
                Check::Fails => false,
                Check::Pending => {
                    pending.push((n, *f, *e));
                    true
                }
            },
            None => true,
        });
        let stopped = ok && {
            let order = self.order(graph, pattern, state);
            self.extend(graph, pattern, &order, 0, injective, state, &mut pending, accept)
        };
        for p in bound {
            state.params[p] = None;
        }
        stopped
    }

    /// The order in which to match the pattern's nodes: each next node connected to nodes
    /// matched before, if possible.
    fn order(&self, graph: &Graph, pattern: &Pattern, state: &State) -> Vec<usize> {
        let mut assigned: HashSet<usize> = (0..state.slots.len()).filter(|s| state.slots[*s].is_some()).collect();
        let mut remaining: Vec<usize> = (0..pattern.nodes.len()).collect();
        let mut order = Vec::new();
        let count = |c: ClassifierId| {
            graph.node_ids().filter(|n| graph.class_of(*n).is_some_and(|x| self.mm.conforms_to(x, c))).count()
        };
        while !remaining.is_empty() {
            let score = |i: usize| {
                let n = &pattern.nodes[i];
                if n.param.is_some_and(|p| matches!(state.params[p], Some(Value::Node(_)))) {
                    return (0, 0);
                }
                let forward = pattern.edges.iter().any(|(s, _, t)| *t == n.slot && assigned.contains(s));
                let backward_opposite = pattern
                    .edges
                    .iter()
                    .any(|(s, f, t)| *s == n.slot && assigned.contains(t) && self.mm.feature(*f).opposite().is_some());
                let backward = pattern.edges.iter().any(|(s, _, t)| *s == n.slot && assigned.contains(t));
                let rank = if forward {
                    1
                } else if backward_opposite {
                    2
                } else if backward {
                    3
                } else {
                    4
                };
                (rank, count(n.class))
            };
            let (pos, _) = remaining.iter().enumerate().min_by_key(|(_, i)| score(**i)).expect("not empty");
            let i = remaining.remove(pos);
            assigned.insert(pattern.nodes[i].slot);
            order.push(i);
        }
        order
    }

    fn candidates(&self, graph: &Graph, pattern: &Pattern, node: &PatternNode, state: &State) -> Vec<NodeId> {
        if let Some(Some(Value::Node(n))) = node.param.map(|p| &state.params[p]) {
            return vec![*n];
        }
        for (s, f, t) in &pattern.edges {
            if *t == node.slot
                && *s != node.slot
                && let Some(source) = state.slots[*s]
            {
                return graph.targets(source, *f).to_vec();
            }
        }
        for (s, f, t) in &pattern.edges {
            if *s == node.slot
                && *t != node.slot
                && let Some(target) = state.slots[*t]
                && let Some(op) = self.mm.feature(*f).opposite()
            {
                return graph.targets(target, op).to_vec();
            }
        }
        graph.node_ids().collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn extend<'p>(
        &self,
        graph: &Graph,
        pattern: &'p Pattern,
        order: &[usize],
        i: usize,
        injective: bool,
        state: &mut State,
        pending: &mut Vec<(NodeId, FeatureId, &'p CExpr)>,
        accept: &mut dyn FnMut(&mut State) -> bool,
    ) -> bool {
        let Some(&index) = order.get(i) else {
            return self.finish(graph, state, pending, accept);
        };
        let node = &pattern.nodes[index];
        let mut seen = HashSet::new();
        for c in self.candidates(graph, pattern, node, state) {
            if !seen.insert(c) || !graph.class_of(c).is_some_and(|class| self.mm.conforms_to(class, node.class)) {
                continue;
            }
            if injective && state.slots.contains(&Some(c)) {
                continue;
            }
            let mut bound = Vec::new();
            if let Some(p) = node.param {
                match &state.params[p] {
                    Some(Value::Node(n)) if *n == c => {}
                    Some(_) => continue,
                    None => {
                        state.params[p] = Some(Value::Node(c));
                        bound.push(p);
                    }
                }
            }
            state.slots[node.slot] = Some(c);
            let linked = pattern.edges.iter().all(|(s, f, t)| {
                if *s != node.slot && *t != node.slot {
                    return true;
                }
                match (state.slots[*s], state.slots[*t]) {
                    (Some(a), Some(b)) => graph.has_link(a, *f, b),
                    _ => true,
                }
            });
            let pending_before = pending.len();
            let mut ok = linked;
            if ok {
                for (f, e) in node.matches {
                    match self.check(graph, state, c, *f, e, &mut bound) {
                        Check::Holds => {}
                        Check::Fails => {
                            ok = false;
                            break;
                        }
                        Check::Pending => pending.push((c, *f, e)),
                    }
                }
            }
            let stopped = ok && self.extend(graph, pattern, order, i + 1, injective, state, pending, accept);
            pending.truncate(pending_before);
            state.slots[node.slot] = None;
            for p in bound {
                state.params[p] = None;
            }
            if stopped {
                return true;
            }
        }
        false
    }

    /// Checks the attribute values that needed parameters bound later, then accepts.
    fn finish(
        &self,
        graph: &Graph,
        state: &mut State,
        pending: &[(NodeId, FeatureId, &CExpr)],
        accept: &mut dyn FnMut(&mut State) -> bool,
    ) -> bool {
        let mut bound = Vec::new();
        let mut left: Vec<(NodeId, FeatureId, &CExpr)> = pending.to_vec();
        let mut ok = true;
        while ok && !left.is_empty() {
            let before = left.len();
            let mut next = Vec::new();
            for (n, f, e) in left {
                match self.check(graph, state, n, f, e, &mut bound) {
                    Check::Holds => {}
                    Check::Fails => {
                        ok = false;
                        break;
                    }
                    Check::Pending => next.push((n, f, e)),
                }
            }
            if next.len() == before {
                // Parameters nothing binds.
                ok = false;
            }
            left = next;
        }
        let stopped = ok && accept(state);
        for p in bound {
            state.params[p] = None;
        }
        stopped
    }

    /// Whether node `n` has the value `e` in attribute `f`. A lone unbound parameter is bound
    /// to the node's value (and recorded in `bound`).
    fn check(
        &self,
        graph: &Graph,
        state: &mut State,
        n: NodeId,
        f: FeatureId,
        e: &CExpr,
        bound: &mut Vec<usize>,
    ) -> Check {
        let actual = Value::of_attribute(self.mm, graph, n, f);
        if let Some(p) = e.as_param()
            && state.params[p].is_none()
        {
            state.params[p] = Some(actual);
            bound.push(p);
            return Check::Holds;
        }
        match e.eval(self.mm, &state.params) {
            Ok(v) if v.equals(&actual) => Check::Holds,
            Ok(Value::Str(s)) if matches!(actual, Value::Enum(..)) => {
                if actual.to_text(self.mm) == s {
                    Check::Holds
                } else {
                    Check::Fails
                }
            }
            Ok(_) => Check::Fails,
            Err(EvalError::Unbound(_)) => Check::Pending,
            Err(EvalError::Failed(m)) => {
                let mut messages = self.messages.borrow_mut();
                if !messages.contains(&m) {
                    messages.push(m);
                }
                Check::Fails
            }
        }
    }

    // ------------------------------------------------------------------ applying

    /// Collects the changes of a match. `created` gives the created node of each inherited
    /// node (multi-rules).
    fn plan(
        &self,
        rule: &Rule,
        m: &Match,
        created: &[Option<usize>],
        plan: &mut Plan,
        top: bool,
    ) -> Result<(), String> {
        let mut new: Vec<Option<usize>> = vec![None; rule.nodes.len()];
        new[..created.len().min(rule.nodes.len())].copy_from_slice(&created[..created.len().min(rule.nodes.len())]);
        for (i, n) in rule.nodes.iter().enumerate().skip(rule.inherited) {
            match n.kind {
                NodeKind::Create => {
                    plan.create_nodes.push(n.class);
                    new[i] = Some(plan.create_nodes.len() - 1);
                    if top && let Some(p) = n.param {
                        plan.created_params.push((p, plan.create_nodes.len() - 1));
                    }
                }
                NodeKind::Delete => plan.delete_nodes.extend(m.nodes[i]),
                NodeKind::Preserve => {}
            }
        }
        let target = |i: usize| -> Result<Target, String> {
            match (new[i], m.nodes.get(i).copied().flatten()) {
                (Some(k), _) => Ok(Target::New(k)),
                (None, Some(n)) => Ok(Target::Old(n)),
                (None, None) => Err(format!("node '{}' is not matched", rule.nodes[i].name)),
            }
        };
        for (i, n) in rule.nodes.iter().enumerate() {
            for (f, e) in &n.sets {
                let value = match e.eval(self.mm, &m.params) {
                    Ok(v) => v,
                    Err(EvalError::Unbound(p)) => {
                        return Err(format!("parameter '{}' has no value", rule.params[p].name));
                    }
                    Err(EvalError::Failed(msg)) => return Err(msg),
                };
                plan.attributes.push((target(i)?, *f, value.to_model(self.mm, *f)?));
            }
            for f in &n.unsets {
                plan.attributes.push((target(i)?, *f, None));
            }
        }
        for e in &rule.edges {
            match e.kind {
                NodeKind::Delete => {
                    if let (Some(s), Some(t)) = (m.nodes[e.source], m.nodes[e.target]) {
                        plan.delete_edges.push((s, e.feature, t));
                    }
                }
                NodeKind::Create => plan.create_edges.push((target(e.source)?, e.feature, target(e.target)?)),
                NodeKind::Preserve => {}
            }
        }
        for (k, multi) in rule.multi_rules.iter().enumerate() {
            for mm in m.multi.get(k).into_iter().flatten() {
                self.plan(multi, mm, &new, plan, false)?;
            }
        }
        Ok(())
    }

    /// Whether deleting the plan's nodes would leave links that the plan does not delete.
    fn dangling(&self, graph: &Graph, plan: &Plan) -> bool {
        let covered = |s: NodeId, f: FeatureId, t: NodeId| {
            plan.delete_edges.contains(&(s, f, t))
                || self.mm.feature(f).opposite().is_some_and(|op| plan.delete_edges.contains(&(t, op, s)))
        };
        let deleted: HashSet<NodeId> = plan.delete_nodes.iter().copied().collect();
        for &n in &deleted {
            let Some(node) = graph.node(n) else { continue };
            for (f, targets) in node.references() {
                if targets.iter().any(|t| !covered(n, f, *t)) {
                    return true;
                }
            }
            if graph.incoming(n).iter().any(|(s, f)| !covered(*s, *f, n)) {
                return true;
            }
        }
        false
    }

    /// Applies a match of a rule (from [`Engine::matches`] on this graph). Returns the
    /// parameter values afterwards (parameters naming created nodes are bound to them).
    pub fn apply_match(&self, graph: &mut Graph, rule: usize, m: &Match) -> Result<Vec<Option<Value>>, String> {
        let r = self.module.rules.get(rule).ok_or("no such rule")?;
        let mut plan = Plan::default();
        self.plan(r, m, &[], &mut plan, true)?;
        let mm = self.mm;
        for (s, f, t) in &plan.delete_edges {
            graph.remove_link(mm, *s, *f, *t);
        }
        let mut deleted = HashSet::new();
        for n in &plan.delete_nodes {
            if deleted.insert(*n) {
                graph.remove_node(mm, *n);
            }
        }
        let created: Vec<NodeId> = plan.create_nodes.iter().map(|c| graph.add_node(*c)).collect();
        let resolve = |t: Target| match t {
            Target::Old(n) => n,
            Target::New(k) => created[k],
        };
        for (t, f, v) in &plan.attributes {
            graph.set_attribute(resolve(*t), *f, v.iter().cloned().collect());
        }
        for (s, f, t) in &plan.create_edges {
            let (s, t) = (resolve(*s), resolve(*t));
            if !graph.add_link(mm, s, *f, t) {
                return Err(format!(
                    "cannot add '{}' from {} to {}: it would contain a node in itself",
                    mm.feature(*f).name,
                    graph.label(mm, s),
                    graph.label(mm, t)
                ));
            }
        }
        let mut params = m.params.clone();
        for (p, k) in &plan.created_params {
            params[*p] = Some(Value::Node(created[*k]));
        }
        Ok(params)
    }

    /// Describes a match, before applying it.
    pub fn describe(&self, graph: &Graph, rule: usize, m: &Match) -> Application {
        let r = &self.module.rules[rule];
        let params = r
            .params
            .iter()
            .zip(&m.params)
            .filter(|(p, _)| p.kind != ParamKind::Var)
            .filter_map(|(p, v)| Some((p.name.clone(), v.as_ref()?.render(self.mm, graph))))
            .collect();
        let mut nodes: Vec<(String, NodeId)> =
            r.nodes.iter().zip(&m.nodes).filter_map(|(n, id)| Some((n.name.clone(), (*id)?))).collect();
        let matched = nodes.iter().map(|(n, id)| (n.clone(), graph.label(self.mm, *id))).collect();
        multi_nodes(r, m, "", &mut nodes);
        Application { rule: r.name.clone(), params, nodes, matched, instances: m.multi.iter().map(Vec::len).sum() }
    }

    /// Adds the parameters an application bound (e.g. `OUT` parameters naming created nodes)
    /// to its description, rendered in the graph after the application.
    pub fn describe_outcome(&self, graph: &Graph, rule: usize, params: &[Option<Value>], app: &mut Application) {
        let r = &self.module.rules[rule];
        for (p, v) in r.params.iter().zip(params) {
            if p.kind != ParamKind::Var
                && !app.params.iter().any(|(n, _)| *n == p.name)
                && let Some(v) = v
            {
                app.params.push((p.name.clone(), v.render(self.mm, graph)));
            }
        }
    }

    // ------------------------------------------------------------------ units

    /// Executes a rule (at its first match) or a unit. `params` has one entry per parameter.
    /// The graph is changed only by the steps that took effect (failed sequences roll back).
    pub fn execute(
        &self,
        graph: &mut Graph,
        callable: Callable,
        params: Vec<Option<Value>>,
        seed: u64,
    ) -> Result<Execution, String> {
        let mut run = Run { engine: self, trace: Vec::new(), rng: seed | 1, applications: 0, depth: 0 };
        let mut params = params;
        params.resize(self.module.params(callable).len(), None);
        let success = run.call(graph, callable, &mut params)?;
        Ok(Execution { success, params, trace: run.trace })
    }
}

/// The nodes matched by the multi-rules of a match (not the kernel's), named
/// `multiRule.node`.
fn multi_nodes(rule: &Rule, m: &Match, prefix: &str, out: &mut Vec<(String, NodeId)>) {
    for (multi, matches) in rule.multi_rules.iter().zip(&m.multi) {
        let prefix = format!("{prefix}{}.", multi.name);
        for mm in matches {
            for (n, id) in multi.nodes.iter().zip(&mm.nodes).skip(multi.inherited) {
                if let Some(id) = id {
                    out.push((format!("{prefix}{}", n.name), *id));
                }
            }
            multi_nodes(multi, mm, &prefix, out);
        }
    }
}

struct Run<'e, 'a> {
    engine: &'e Engine<'a>,
    trace: Vec<Application>,
    rng: u64,
    applications: usize,
    depth: usize,
}

impl Run<'_, '_> {
    fn random(&mut self, n: usize) -> usize {
        // xorshift64
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng % n.max(1) as u64) as usize
    }

    fn call(&mut self, graph: &mut Graph, callable: Callable, params: &mut Vec<Option<Value>>) -> Result<bool, String> {
        let engine = self.engine;
        match callable {
            Callable::Rule(i) => {
                let Some(m) = engine.matches(graph, i, params, 1).pop() else { return Ok(false) };
                self.applications += 1;
                if self.applications > MAX_APPLICATIONS {
                    return Err(format!("stopped after {MAX_APPLICATIONS} rule applications (an endless loop?)"));
                }
                let mut application = engine.describe(graph, i, &m);
                *params = engine.apply_match(graph, i, &m)?;
                engine.describe_outcome(graph, i, params, &mut application);
                self.trace.push(application);
                Ok(true)
            }
            Callable::Unit(i) => {
                if self.depth >= MAX_CALL_DEPTH {
                    return Err(format!("units call each other more than {MAX_CALL_DEPTH} levels deep"));
                }
                self.depth += 1;
                let unit = &engine.module.units[i];
                let result = self.exec(graph, &unit.body, params);
                self.depth -= 1;
                result
            }
        }
    }

    fn exec(&mut self, graph: &mut Graph, step: &Step, params: &mut Vec<Option<Value>>) -> Result<bool, String> {
        match step {
            Step::Call { target, args, .. } => {
                let callee = self.engine.module.params(*target);
                let mut values: Vec<Option<Value>> = vec![None; callee.len()];
                for (i, a) in args.iter().enumerate() {
                    if callee.get(i).is_some_and(|p| p.kind != ParamKind::Var) {
                        values[i] = params.get(*a).cloned().flatten();
                    }
                }
                let ok = self.call(graph, *target, &mut values)?;
                if ok {
                    for (i, a) in args.iter().enumerate() {
                        let passes_back = callee
                            .get(i)
                            .is_some_and(|p| matches!(p.kind, ParamKind::Out | ParamKind::InOut | ParamKind::Unknown));
                        if passes_back && let Some(v) = values[i].clone() {
                            params[*a] = Some(v);
                        }
                    }
                }
                Ok(ok)
            }
            Step::Sequence { steps, strict, rollback } => {
                let snapshot = (*strict && *rollback).then(|| (graph.clone(), params.clone(), self.trace.len()));
                for s in steps {
                    if !self.exec(graph, s, params)? && *strict {
                        if let Some((g, p, t)) = snapshot {
                            *graph = g;
                            *params = p;
                            self.trace.truncate(t);
                        }
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Step::Independent(steps) => {
                let mut order: Vec<usize> = (0..steps.len()).collect();
                for i in (1..order.len()).rev() {
                    let j = self.random(i + 1);
                    order.swap(i, j);
                }
                for i in order {
                    if self.exec(graph, &steps[i], params)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Step::Priority(steps) => {
                for s in steps {
                    if self.exec(graph, s, params)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Step::If { condition, then, otherwise } => {
                if self.exec(graph, condition, params)? {
                    self.exec(graph, then, params)
                } else if let Some(o) = otherwise {
                    self.exec(graph, o, params)
                } else {
                    Ok(true)
                }
            }
            Step::For { count, body } => {
                let n = match count.eval(self.engine.mm, params) {
                    Ok(Value::Int(n)) => n,
                    Ok(v) => {
                        return Err(format!("'for' needs a number of iterations, not {}", v.to_text(self.engine.mm)));
                    }
                    Err(EvalError::Unbound(_)) => {
                        return Err("the number of iterations of 'for' uses a parameter that has no value".into());
                    }
                    Err(EvalError::Failed(m)) => return Err(m),
                };
                let snapshot = (graph.clone(), params.clone(), self.trace.len());
                for _ in 0..n.max(0) {
                    let before = self.applications;
                    let ok = self.exec(graph, body, params)?;
                    if ok && self.applications == before {
                        // Nothing changed: the remaining iterations would not change anything either.
                        break;
                    }
                    if !ok {
                        (*graph, *params) = (snapshot.0, snapshot.1);
                        self.trace.truncate(snapshot.2);
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Step::While(body) => {
                loop {
                    let before = self.applications;
                    if !self.exec(graph, body, params)? {
                        break;
                    }
                    if self.applications == before {
                        // The body succeeds without applying anything: it would loop forever.
                        break;
                    }
                }
                Ok(true)
            }
        }
    }
}
