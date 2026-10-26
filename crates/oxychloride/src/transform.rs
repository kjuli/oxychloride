//! Graph transformations of the workspace's models with the rules and units of Henshin
//! documents (`.henshin_text` or `.henshin`, which mean the same).
//!
//! A *transformation session* starts from a model resource (an XMI file or a HUTN/XMI
//! document) and keeps the states it went through: step 0 is the model, every applied rule or
//! unit adds a step. The model itself is not changed; states can be exported as HUTN or XMI.
//! Rules are taken from the Henshin document at every request, so editing the rules between
//! steps works. A session can also explore the state space of a set of rules from one of its
//! steps, and adopt a path of the state space as its steps.
//!
//! Sessions keep a copy of the metamodel they were started with; when the workspace's
//! metamodel changes, their graphs are mapped onto the new one by qualified names.

use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;
use oxy_ecore::{Metamodel, ObjectId};
use oxy_henshin::module::ParamType;
use oxy_henshin::syntax::ParamKind;
use oxy_henshin::{
    Application, Callable, Engine, ExploreOptions, Graph, Module, NodeId, Param, Selected, StateSpace, Value,
};
use oxy_ocl::Status;
use serde_json::{Map, Value as Json};

use crate::protocol::*;
use crate::workspace::{Compiled, Error, Workspace};

/// The transformation sessions of a workspace.
#[derive(Clone, Debug, Default)]
pub(crate) struct Sessions {
    next: u64,
    sessions: IndexMap<u64, Session>,
}

#[derive(Clone, Debug)]
struct Session {
    model: String,
    /// The metamodel the graphs refer to.
    mm: Metamodel,
    /// The workspace's metamodel generation `mm` is a copy of.
    generation: u64,
    steps: Vec<Step>,
    space: Option<Explored>,
}

#[derive(Clone, Debug)]
struct Step {
    graph: Graph,
    label: String,
    applications: Vec<RuleApplicationInfo>,
}

#[derive(Clone, Debug)]
struct Explored {
    /// The step the exploration started from.
    step: usize,
    space: StateSpace,
}

/// Options of [`Workspace::explore_state_space`].
#[derive(Clone, Debug)]
pub struct ExploreRequest {
    /// The rules to apply (names).
    pub rules: Vec<String>,
    /// Fixed parameter values per rule.
    pub parameters: HashMap<String, Map<String, Json>>,
    /// The step to start from (default: the last).
    pub step: Option<usize>,
    pub max_states: usize,
    pub max_depth: Option<usize>,
    /// Count the invariant violations of every state.
    pub validate: bool,
}

impl Default for ExploreRequest {
    fn default() -> Self {
        ExploreRequest {
            rules: Vec::new(),
            parameters: HashMap::new(),
            step: None,
            max_states: ExploreOptions::default().max_states,
            max_depth: None,
            validate: true,
        }
    }
}

fn kind_name(kind: ParamKind) -> &'static str {
    match kind {
        ParamKind::In => "in",
        ParamKind::Out => "out",
        ParamKind::InOut => "inout",
        ParamKind::Var => "var",
        ParamKind::Unknown => "unknown",
    }
}

fn application_info(app: &Application) -> RuleApplicationInfo {
    RuleApplicationInfo {
        rule: app.rule.clone(),
        label: app.detailed_label(),
        parameters: app.params.iter().map(|(n, v)| NamedValue { name: n.clone(), value: v.clone() }).collect(),
        nodes: app.nodes.iter().map(|(n, id)| MatchedNode { name: n.clone(), node: id.0 }).collect(),
        instances: app.instances,
    }
}

impl Workspace {
    // ------------------------------------------------------------------ documents

    /// The compiled Henshin document `name`, of either syntax (against the current metamodel).
    pub(crate) fn henshin_module(&self, name: &str) -> Result<Module, Error> {
        let text =
            self.document_text(name).ok_or_else(|| Error::NotFound(format!("no open document named '{name}'")))?;
        let language = self.document_language(name).filter(|l| l.is_transformation());
        let language = language.ok_or_else(|| Error::Invalid(format!("'{name}' is not a Henshin document")))?;
        Ok(self.compile_transformation(name, text, language).0)
    }

    /// The compiled module of a document that must not have errors.
    fn runnable_module(&self, name: &str) -> Result<Module, Error> {
        let module = self.henshin_module(name)?;
        if let Some(p) = module.problems.iter().find(|p| p.is_error()) {
            let at = self.document_range(name, &p.span).start;
            return Err(Error::Invalid(format!(
                "{name} has errors (line {}: {}); fix them first",
                at.line + 1,
                p.message
            )));
        }
        Ok(module)
    }

    /// The problems of a Henshin document.
    pub(crate) fn henshin_diagnostics(&self, name: &str) -> Vec<Diagnostic> {
        let Ok(module) = self.henshin_module(name) else { return Vec::new() };
        module
            .problems
            .iter()
            .map(|p| Diagnostic {
                severity: if p.is_error() { Severity::Error } else { Severity::Warning },
                message: p.message.clone(),
                source: name.to_string(),
                origin: Origin::Document { name: name.to_string() },
                range: self.document_range(name, &p.span),
            })
            .collect()
    }

    /// The rules and units of a Henshin document.
    pub fn transformation_units(&self, document: &str) -> Result<TransformationUnits, Error> {
        let module = self.henshin_module(document)?;
        let mm = self.metamodel();
        let params = |params: &[Param]| -> Vec<TransformationParameterInfo> {
            params
                .iter()
                .map(|p| TransformationParameterInfo {
                    name: p.name.clone(),
                    kind: kind_name(p.kind).into(),
                    ty: p.type_name(mm),
                    node: matches!(p.ty, ParamType::Class(_)),
                    literals: match &p.ty {
                        ParamType::Data(c) => mm.enum_literals(*c).iter().map(|l| l.name.clone()).collect(),
                        _ => Vec::new(),
                    },
                })
                .collect()
        };
        let mut units: Vec<TransformationUnitInfo> = module
            .rules
            .iter()
            .map(|r| TransformationUnitInfo {
                name: r.name.clone(),
                kind: "rule".into(),
                parameters: params(&r.params),
                range: self.document_range(document, &r.name_span),
                multi_rules: r.multi_rules.iter().map(|m| m.name.clone()).collect(),
            })
            .collect();
        units.extend(module.units.iter().map(|u| TransformationUnitInfo {
            name: u.name.clone(),
            kind: "unit".into(),
            parameters: params(&u.params),
            range: self.document_range(document, &u.name_span),
            multi_rules: Vec::new(),
        }));
        units.sort_by_key(|u| (u.range.start.line, u.range.start.column));
        Ok(TransformationUnits { units, diagnostics: self.henshin_diagnostics(document) })
    }

    // ------------------------------------------------------------------ sessions

    /// Starts transforming a model resource (by URI, path, file name or document name).
    pub fn start_transformation(&mut self, model: &str) -> Result<TransformationState, Error> {
        let resource = self.resource_index(model)?;
        let (graph, _) = Graph::from_model(self.metamodel(), self.model(), resource);
        let uri = self.model().resources()[resource].uri.clone();
        let session = Session {
            model: uri,
            mm: self.metamodel().clone(),
            generation: self.generation,
            steps: vec![Step { graph, label: "initial model".into(), applications: Vec::new() }],
            space: None,
        };
        let id = self.transformations.next;
        self.transformations.next += 1;
        self.transformations.sessions.insert(id, session);
        self.transformation_state(id, None)
    }

    pub fn close_transformation(&mut self, session: u64) -> bool {
        self.transformations.sessions.shift_remove(&session).is_some()
    }

    /// The session, its graphs mapped onto the current metamodel.
    fn session(&mut self, id: u64) -> Result<&mut Session, Error> {
        let generation = self.generation;
        let mm = &self.mm;
        let s = self
            .transformations
            .sessions
            .get_mut(&id)
            .ok_or_else(|| Error::NotFound(format!("no transformation session {id}")))?;
        if s.generation != generation {
            for step in &mut s.steps {
                step.graph = step.graph.migrate(&s.mm, mm);
            }
            s.space = None;
            s.mm = mm.clone();
            s.generation = generation;
        }
        Ok(s)
    }

    fn step_index(s: &Session, step: Option<usize>) -> Result<usize, Error> {
        let last = s.steps.len() - 1;
        match step {
            Some(i) if i > last => Err(Error::NotFound(format!("the transformation has no step {i}"))),
            Some(i) => Ok(i),
            None => Ok(last),
        }
    }

    /// A step of a session (default: the last).
    pub fn transformation_state(&mut self, session: u64, step: Option<usize>) -> Result<TransformationState, Error> {
        let s = self.session(session)?;
        let index = Self::step_index(s, step)?;
        let model = s.model.clone();
        let steps: Vec<TransformationStep> = s
            .steps
            .iter()
            .enumerate()
            .map(|(i, st)| TransformationStep {
                index: i,
                label: st.label.clone(),
                applications: st.applications.clone(),
            })
            .collect();
        let current = s.steps[index].graph.clone();
        let previous = index.checked_sub(1).map(|i| s.steps[i].graph.clone());
        let changes = previous.map_or_else(GraphChanges::default, |p| self.changes(&p, &current));
        let compiled = self.compile(true, None);
        Ok(TransformationState {
            session,
            model,
            steps,
            step: index,
            graph: self.graph_info(&current),
            changes,
            violations: self.violations(&current, &compiled),
        })
    }

    /// Parameter values from JSON (`{name: value}`): strings, numbers and Booleans for data
    /// types (enumeration literals by name), node ids for nodes; `null` or missing: unbound.
    fn param_values(
        &self,
        params: &[Param],
        given: &Map<String, Json>,
        graph: &Graph,
    ) -> Result<Vec<Option<Value>>, Error> {
        let mm = self.metamodel();
        for name in given.keys() {
            if !params.iter().any(|p| p.name == *name) {
                return Err(Error::Invalid(format!("there is no parameter '{name}'")));
            }
        }
        let invalid = |p: &Param, v: &Json| {
            Error::Invalid(format!("{v} is not a value of parameter '{}' : {}", p.name, p.type_name(mm)))
        };
        params
            .iter()
            .map(|p| {
                let Some(v) = given.get(&p.name).filter(|v| !v.is_null()) else { return Ok(None) };
                Ok(Some(match &p.ty {
                    ParamType::Class(c) => {
                        let id =
                            v.as_u64().or_else(|| v.get("node").and_then(Json::as_u64)).ok_or_else(|| invalid(p, v))?;
                        let node = NodeId(id as u32);
                        match graph.class_of(node) {
                            Some(class) if mm.conforms_to(class, *c) => Value::Node(node),
                            Some(_) => {
                                return Err(Error::Invalid(format!(
                                    "{} is not a {}",
                                    graph.label(mm, node),
                                    mm.classifier(*c).name
                                )));
                            }
                            None => return Err(Error::NotFound(format!("there is no object #{id}"))),
                        }
                    }
                    ParamType::Data(c) if mm.is_enum(*c) => {
                        let name = v.as_str().ok_or_else(|| invalid(p, v))?;
                        if name.is_empty() {
                            return Ok(None);
                        }
                        Value::Enum(*c, mm.enum_literal_index(*c, name).ok_or_else(|| invalid(p, v))?)
                    }
                    ParamType::Data(c) => {
                        use oxy_ecore::DataTypeKind as K;
                        let kind = mm.data_type_kind(*c).unwrap_or(K::Other);
                        match (kind, v) {
                            (K::String | K::Other, Json::String(s)) => Value::Str(s.clone()),
                            (K::String | K::Other, v) => Value::Str(v.to_string()),
                            (_, Json::String(s)) if s.trim().is_empty() => return Ok(None),
                            (K::Boolean, Json::Bool(b)) => Value::Bool(*b),
                            (K::Boolean, Json::String(s)) => match s.trim() {
                                "true" => Value::Bool(true),
                                "false" => Value::Bool(false),
                                _ => return Err(invalid(p, v)),
                            },
                            (K::Integer, v) => {
                                let i = v.as_i64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()));
                                Value::Int(i.ok_or_else(|| invalid(p, v))?)
                            }
                            (K::Real, v) => {
                                let r = v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()));
                                Value::Real(r.ok_or_else(|| invalid(p, v))?)
                            }
                            _ => return Err(invalid(p, v)),
                        }
                    }
                    ParamType::Unknown(_) => match v {
                        Json::Bool(b) => Value::Bool(*b),
                        Json::Number(n) => {
                            n.as_i64().map_or_else(|| Value::Real(n.as_f64().unwrap_or(0.0)), Value::Int)
                        }
                        Json::String(s) => Value::Str(s.clone()),
                        _ => return Err(invalid(p, v)),
                    },
                }))
            })
            .collect()
    }

    /// The matches of a rule in a step of a session (default: the last).
    pub fn find_matches(
        &mut self,
        session: u64,
        document: &str,
        rule: &str,
        parameters: &Map<String, Json>,
        step: Option<usize>,
        limit: usize,
    ) -> Result<MatchList, Error> {
        let module = self.runnable_module(document)?;
        let Some(Callable::Rule(r)) = module.callable(rule) else {
            return Err(Error::NotFound(format!("{document} has no rule named '{rule}'")));
        };
        let s = self.session(session)?;
        let index = Self::step_index(s, step)?;
        let graph = s.steps[index].graph.clone();
        let params = self.param_values(&module.rules[r].params, parameters, &graph)?;
        let engine = Engine::new(self.metamodel(), &module);
        let mut matches = engine.matches(&graph, r, &params, limit.saturating_add(1));
        let more = matches.len() > limit;
        matches.truncate(limit);
        let matches = matches
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let app = engine.describe(&graph, r, m);
                MatchInfo {
                    index: i,
                    label: app.detailed_label(),
                    parameters: app
                        .params
                        .iter()
                        .map(|(n, v)| NamedValue { name: n.clone(), value: v.clone() })
                        .collect(),
                    nodes: app.nodes.iter().map(|(n, id)| MatchedNode { name: n.clone(), node: id.0 }).collect(),
                    matched: m.matched_nodes().iter().map(|n| n.0).collect(),
                    instances: app.instances,
                }
            })
            .collect();
        Ok(MatchList { matches, more, messages: engine.messages() })
    }

    /// Applies a rule (at match `match_index` of `find_matches`, else at its first match) or
    /// executes a unit on a step of a session (default: the last). If it applies, the steps
    /// after that step are discarded and the result becomes the last step.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_transformation(
        &mut self,
        session: u64,
        document: &str,
        unit: &str,
        parameters: &Map<String, Json>,
        match_index: Option<usize>,
        step: Option<usize>,
        seed: u64,
    ) -> Result<TransformationResult, Error> {
        let module = self.runnable_module(document)?;
        let callable = module
            .callable(unit)
            .ok_or_else(|| Error::NotFound(format!("{document} has no rule or unit named '{unit}'")))?;
        let s = self.session(session)?;
        let base = Self::step_index(s, step)?;
        let before = s.steps[base].graph.clone();
        let mut graph = before.clone();
        let params = self.param_values(module.params(callable), parameters, &graph)?;
        let mm = self.metamodel();
        let engine = Engine::new(mm, &module);
        let mut messages = Vec::new();
        let applied = match (callable, match_index) {
            (Callable::Rule(r), Some(i)) => {
                let m = engine
                    .matches(&graph, r, &params, i.saturating_add(1))
                    .into_iter()
                    .nth(i)
                    .ok_or_else(|| Error::Invalid(format!("rule '{unit}' has no match {i} (any more)")))?;
                let mut app = engine.describe(&graph, r, &m);
                let after = engine.apply_match(&mut graph, r, &m).map_err(Error::Invalid)?;
                engine.describe_outcome(&graph, r, &after, &mut app);
                Some((app.detailed_label(), vec![application_info(&app)]))
            }
            (Callable::Unit(_), Some(_)) => {
                return Err(Error::Invalid(format!("'{unit}' is a unit: it has no matches")));
            }
            (_, None) => match engine.execute(&mut graph, callable, params.clone(), seed) {
                Ok(e) if e.success => {
                    let label = match callable {
                        Callable::Rule(_) => {
                            e.trace.last().map(Application::detailed_label).unwrap_or_else(|| unit.to_string())
                        }
                        Callable::Unit(_) => {
                            let given: Vec<String> = module
                                .params(callable)
                                .iter()
                                .zip(&params)
                                .filter_map(|(p, v)| Some(format!("{}={}", p.name, v.as_ref()?.render(mm, &before))))
                                .collect();
                            if given.is_empty() { unit.to_string() } else { format!("{unit}({})", given.join(", ")) }
                        }
                    };
                    Some((label, e.trace.iter().map(application_info).collect()))
                }
                Ok(_) => None,
                Err(m) => {
                    messages.push(m);
                    None
                }
            },
        };
        messages.extend(engine.messages());
        let index = match applied {
            Some((label, applications)) => {
                let s = self.session(session)?;
                s.steps.truncate(base + 1);
                s.steps.push(Step { graph, label, applications });
                if s.space.as_ref().is_some_and(|e| e.step > base) {
                    s.space = None;
                }
                s.steps.len() - 1
            }
            None => {
                if messages.is_empty() {
                    messages.push(format!("'{unit}' is not applicable"));
                }
                base
            }
        };
        let state = self.transformation_state(session, Some(index))?;
        Ok(TransformationResult { applied: index != base, state, messages })
    }

    /// A step of a session (default: the last) as a HUTN or XMI document. `target` is where
    /// it will be written (for relative `xsi:schemaLocation`s in XMI).
    pub fn export_transformation(
        &mut self,
        session: u64,
        step: Option<usize>,
        format: Language,
        target: Option<&str>,
    ) -> Result<String, Error> {
        let s = self.session(session)?;
        let index = Self::step_index(s, step)?;
        let graph = s.steps[index].graph.clone();
        let uri = target.map_or_else(|| s.model.clone(), str::to_string);
        let mm = self.metamodel();
        let (model, _) = graph.to_model(mm, &uri);
        match format {
            Language::Hutn => Ok(oxy_ecore::hutn::write(mm, &model, 0)),
            Language::Xmi => {
                let locations = self.schema_locations(&uri);
                Ok(oxy_ecore::xmi_writer::write_resource(mm, &model, 0, &locations))
            }
            other => Err(Error::Invalid(format!("states can be exported as HUTN or XMI, not {}", other.extension()))),
        }
    }

    // ------------------------------------------------------------------ state spaces

    /// Explores the states that the given rules reach from a step of a session.
    pub fn explore_state_space(
        &mut self,
        session: u64,
        document: &str,
        request: &ExploreRequest,
    ) -> Result<StateSpaceInfo, Error> {
        let module = self.runnable_module(document)?;
        let s = self.session(session)?;
        let base = Self::step_index(s, request.step)?;
        let initial = s.steps[base].graph.clone();
        let mut selected = Vec::new();
        let mut names = Vec::new();
        for name in &request.rules {
            match module.callable(name) {
                Some(Callable::Rule(r)) => {
                    let empty = Map::new();
                    let given = request.parameters.get(name).unwrap_or(&empty);
                    let params = self.param_values(&module.rules[r].params, given, &initial)?;
                    selected.push(Selected { rule: r, params });
                    names.push(name.clone());
                }
                Some(Callable::Unit(_)) => {
                    return Err(Error::Invalid(format!("'{name}' is a unit: state spaces are explored with rules")));
                }
                None => return Err(Error::NotFound(format!("{document} has no rule named '{name}'"))),
            }
        }
        if selected.is_empty() {
            return Err(Error::Invalid("select at least one rule".into()));
        }
        let engine = Engine::new(self.metamodel(), &module);
        let options = ExploreOptions { max_states: request.max_states.max(1), max_depth: request.max_depth };
        let space = oxy_henshin::explore(&engine, initial, &selected, &options);
        let compiled = request.validate.then(|| self.compile(true, None));
        let info = StateSpaceInfo {
            session,
            step: base,
            rules: names.clone(),
            states: space
                .states
                .iter()
                .enumerate()
                .map(|(i, st)| StateSpaceState {
                    id: i,
                    depth: st.depth,
                    nodes: st.graph.len(),
                    explored: st.explored,
                    terminal: st.explored && space.outgoing(i).next().is_none(),
                    violations: compiled.as_ref().map_or(0, |c| self.violations(&st.graph, c).len()),
                })
                .collect(),
            transitions: space
                .transitions
                .iter()
                .map(|t| StateSpaceTransition {
                    source: t.source,
                    target: t.target,
                    rule: t.application.rule.clone(),
                    label: t.application.detailed_label(),
                    count: t.count,
                })
                .collect(),
            complete: space.complete,
            messages: space.messages.clone(),
        };
        self.session(session)?.space = Some(Explored { step: base, space });
        Ok(info)
    }

    fn explored(&mut self, session: u64) -> Result<Explored, Error> {
        self.session(session)?
            .space
            .clone()
            .ok_or_else(|| Error::NotFound("the state space has not been explored (or the model changed)".into()))
    }

    /// A state of the session's state space.
    pub fn state_space_state(&mut self, session: u64, state: usize) -> Result<StateSpaceStateDetails, Error> {
        let explored = self.explored(session)?;
        let space = &explored.space;
        let st =
            space.states.get(state).ok_or_else(|| Error::NotFound(format!("the state space has no state {state}")))?;
        let changes = match st.parent {
            Some(t) => self.changes(&space.states[space.transitions[t].source].graph, &st.graph),
            None => GraphChanges::default(),
        };
        let compiled = self.compile(true, None);
        Ok(StateSpaceStateDetails {
            id: state,
            graph: self.graph_info(&st.graph),
            violations: self.violations(&st.graph, &compiled),
            path: space.path(state),
            changes,
        })
    }

    /// Replaces the steps after the step the state space was explored from by the shortest path
    /// to one of its states.
    pub fn adopt_state(&mut self, session: u64, state: usize) -> Result<TransformationState, Error> {
        let explored = self.explored(session)?;
        let space = &explored.space;
        if state >= space.states.len() {
            return Err(Error::NotFound(format!("the state space has no state {state}")));
        }
        let path = space.path(state);
        let s = self.session(session)?;
        s.steps.truncate(explored.step + 1);
        for t in path {
            let tr = &space.transitions[t];
            s.steps.push(Step {
                graph: space.states[tr.target].graph.clone(),
                label: tr.application.detailed_label(),
                applications: vec![application_info(&tr.application)],
            });
        }
        self.transformation_state(session, None)
    }

    // ------------------------------------------------------------------ rendering

    /// Whether a link is listed in [`GraphInfo`] (one side of each bidirectional pair).
    fn listed(mm: &Metamodel, feature: oxy_ecore::FeatureId, source: NodeId, target: NodeId) -> bool {
        let f = mm.feature(feature);
        match f.opposite() {
            None => true,
            Some(op) if op == feature => source <= target,
            Some(op) => {
                let o = mm.feature(op);
                !o.is_containment() && (f.is_containment() || feature.0 < op.0)
            }
        }
    }

    fn node_info(&self, graph: &Graph, n: NodeId) -> GraphNodeInfo {
        let mm = self.metamodel();
        let empty = oxy_ecore::Model::new();
        let node = graph.node(n);
        GraphNodeInfo {
            id: n.0,
            class: node.map(|x| mm.qualified_name(x.class)).unwrap_or_default(),
            label: graph.label(mm, n),
            attributes: node
                .map(|x| {
                    x.attributes()
                        .map(|(f, values)| SlotInfo {
                            name: mm.feature(f).name.clone(),
                            values: values
                                .iter()
                                .map(|v| oxy_ocl::eval::convert_model_value(v).render(mm, &empty))
                                .collect(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            container: node.and_then(|x| x.container.map(|(c, _)| c.0)),
        }
    }

    fn edges(&self, graph: &Graph) -> Vec<GraphEdgeInfo> {
        let mm = self.metamodel();
        graph
            .links()
            .into_iter()
            .filter(|(s, f, t)| Self::listed(mm, *f, *s, *t))
            .map(|(s, f, t)| {
                let feature = mm.feature(f);
                GraphEdgeInfo {
                    source: s.0,
                    target: t.0,
                    feature: feature.name.clone(),
                    containment: feature.is_containment(),
                    opposite: feature.opposite().map(|o| mm.feature(o).name.clone()),
                }
            })
            .collect()
    }

    pub(crate) fn graph_info(&self, graph: &Graph) -> GraphInfo {
        GraphInfo { nodes: graph.node_ids().map(|n| self.node_info(graph, n)).collect(), edges: self.edges(graph) }
    }

    /// What changed from `before` to `after` (nodes are compared by id).
    fn changes(&self, before: &Graph, after: &Graph) -> GraphChanges {
        let old_edges: HashSet<GraphEdgeInfo> = self.edges(before).into_iter().collect();
        let new_edges: HashSet<GraphEdgeInfo> = self.edges(after).into_iter().collect();
        let order = |mut v: Vec<GraphEdgeInfo>| {
            v.sort_by(|a, b| (a.source, a.target, &a.feature).cmp(&(b.source, b.target, &b.feature)));
            v
        };
        let mut changed_attributes = Vec::new();
        for n in after.node_ids().filter(|n| before.contains(*n)) {
            let (old, new) = (self.node_info(before, n), self.node_info(after, n));
            let names: Vec<&String> = old
                .attributes
                .iter()
                .map(|s| &s.name)
                .chain(new.attributes.iter().map(|s| &s.name))
                .collect::<indexmap::IndexSet<_>>()
                .into_iter()
                .collect();
            for name in names {
                let values = |info: &GraphNodeInfo| {
                    info.attributes.iter().find(|s| s.name == *name).map(|s| s.values.clone()).unwrap_or_default()
                };
                let (o, v) = (values(&old), values(&new));
                if o != v {
                    changed_attributes.push(AttributeChange { node: n.0, feature: name.clone(), old: o, new: v });
                }
            }
        }
        GraphChanges {
            created_nodes: after.node_ids().filter(|n| !before.contains(*n)).map(|n| n.0).collect(),
            deleted_nodes: before
                .node_ids()
                .filter(|n| !after.contains(*n))
                .map(|n| self.node_info(before, n))
                .collect(),
            created_edges: order(new_edges.difference(&old_edges).cloned().collect()),
            deleted_edges: order(old_edges.difference(&new_edges).cloned().collect()),
            changed_attributes,
        }
    }

    /// The invariants a state violates or cannot evaluate.
    fn violations(&self, graph: &Graph, compiled: &Compiled) -> Vec<StateViolation> {
        let mm = self.metamodel();
        if compiled.env.invariants().is_empty() {
            return Vec::new();
        }
        let (model, map) = graph.to_model(mm, "state");
        let nodes: HashMap<ObjectId, NodeId> = map.iter().map(|(n, o)| (*o, *n)).collect();
        let objects: Vec<ObjectId> = model.object_ids().collect();
        let validation = oxy_ocl::validate_model(mm, &model, &compiled.env, &objects, false);
        validation
            .invariants
            .iter()
            .filter(|r| r.status != Status::Satisfied)
            .filter_map(|r| {
                let inv = &compiled.env.invariants()[r.invariant];
                let node = *nodes.get(&r.object)?;
                let (status, message) = match &r.status {
                    Status::Error(m) => (ResultStatus::Error, Some(m.clone())),
                    _ => (ResultStatus::Violated, r.message.clone()),
                };
                Some(StateViolation {
                    constraint: inv.name.clone().unwrap_or_else(|| "<anonymous>".into()),
                    context: mm.qualified_name(inv.context),
                    node: node.0,
                    label: graph.label(mm, node),
                    status,
                    message,
                })
            })
            .collect()
    }
}
