//! Henshin transformations as rule diagrams (in Henshin's notation), and changing them from a
//! diagram.
//!
//! A diagram shows the rules of a transformation document of either syntax — the nodes, edges
//! and attributes of a rule with their actions, the elements of its multi-rules and of the
//! condition graphs of its matching formulas — and its units. It is drawn from the syntax tree
//! (for a `.henshin` file, the one [`xmi::read`](oxy_henshin::xmi::read) makes, whose spans
//! are in the XML), so its ranges select the elements in the document's text.
//!
//! Edits ([`TransformationEdit`]) become small text edits (see `henshin_edit.rs`), as for
//! metamodel diagrams: in a text, on its syntax tree; in a `.henshin` file, the edit is made on
//! the file's textual form, which is then compiled and written back into the file (keeping its
//! ids and what the text cannot express), and the difference becomes line edits.

use std::collections::HashMap;
use std::ops::Range as ByteRange;

use indexmap::IndexMap;
use oxy_ecore::{ClassifierId, FeatureKind, Metamodel, PackageId};
use oxy_henshin::Module;
use oxy_henshin::print;
use oxy_henshin::syntax::*;
use oxy_ocl::LineIndex;

use super::{IdAllocator, byte_range};
use crate::henshin_documents::Syntax;
use crate::protocol::*;
use crate::workspace::{Error, Workspace};

/// The kinds of parameters, as the protocol names them.
pub(crate) fn kind_name(kind: ParamKind) -> &'static str {
    match kind {
        ParamKind::In => "in",
        ParamKind::Out => "out",
        ParamKind::InOut => "inout",
        ParamKind::Var => "var",
        ParamKind::Unknown => "unknown",
    }
}

pub(crate) fn rule_action(a: Action) -> RuleAction {
    match a {
        Action::Preserve => RuleAction::Preserve,
        Action::Create => RuleAction::Create,
        Action::Delete => RuleAction::Delete,
        Action::Forbid => RuleAction::Forbid,
        Action::Require => RuleAction::Require,
    }
}

pub(crate) fn syntax_action(a: RuleAction) -> Action {
    match a {
        RuleAction::Preserve => Action::Preserve,
        RuleAction::Create => Action::Create,
        RuleAction::Delete => Action::Delete,
        RuleAction::Forbid => Action::Forbid,
        RuleAction::Require => Action::Require,
    }
}

// ---------------------------------------------------------------------- the index

/// Where a scope of a rule is written: the rule, a multi-rule or a condition graph.
#[derive(Clone, Copy)]
pub(crate) enum ScopeDecl<'a> {
    Rule(&'a RuleDecl),
    Multi(&'a MultiRuleDecl),
    Condition(&'a ConditionGraphDecl),
}

impl<'a> ScopeDecl<'a> {
    pub(crate) fn name(&self) -> &'a Name {
        match self {
            ScopeDecl::Rule(r) => &r.name,
            ScopeDecl::Multi(m) => &m.name,
            ScopeDecl::Condition(c) => &c.name,
        }
    }

    pub(crate) fn span(&self) -> ByteRange<usize> {
        match self {
            ScopeDecl::Rule(r) => r.span.clone(),
            ScopeDecl::Multi(m) => m.span.clone(),
            ScopeDecl::Condition(c) => c.span.clone(),
        }
    }

    /// The rule items (of rules and multi-rules).
    pub(crate) fn rule_items(&self) -> &'a [RuleItem] {
        match self {
            ScopeDecl::Rule(r) => &r.items,
            ScopeDecl::Multi(m) => &m.items,
            ScopeDecl::Condition(_) => &[],
        }
    }

    /// The graph elements of the scope.
    pub(crate) fn graph_items(&self) -> Vec<&'a GraphItem> {
        match self {
            ScopeDecl::Condition(c) => c.items.iter().collect(),
            _ => self
                .rule_items()
                .iter()
                .filter_map(|i| if let RuleItem::Graph(g) = i { Some(g) } else { None })
                .flat_map(|g| g.items.iter())
                .collect(),
        }
    }

    /// The (first) `graph { ... }` of a rule or multi-rule.
    pub(crate) fn graph(&self) -> Option<&'a GraphDecl> {
        self.rule_items().iter().find_map(|i| if let RuleItem::Graph(g) = i { Some(g) } else { None })
    }
}

pub(crate) struct ScopeEntry<'a> {
    pub(crate) decl: ScopeDecl<'a>,
    /// The id of the rule the scope belongs to.
    pub(crate) rule: String,
    /// The enclosing scope.
    pub(crate) parent: Option<String>,
    /// The names of the nodes visible in the scope (its own and the enclosing scopes'), with
    /// their ids.
    pub(crate) names: HashMap<String, String>,
    /// The multi-rule the scope is (or is in).
    pub(crate) multi: Option<String>,
    /// The condition graph the scope is.
    pub(crate) condition: Option<String>,
    /// The action of the elements of a condition graph (`forbid` for one that is negated in a
    /// conjunction, else `require`); `preserve` for rules and multi-rules.
    pub(crate) polarity: RuleAction,
}

pub(crate) struct NodeEntry<'a> {
    pub(crate) decl: &'a NodeDecl,
    pub(crate) scope: String,
    pub(crate) action: RuleAction,
}

/// What declares an attribute: the node, or a `reuse` of it.
#[derive(Clone, Copy)]
pub(crate) enum AttributeOwner<'a> {
    Node(&'a NodeDecl),
    Reuse(&'a ReuseDecl),
}

impl<'a> AttributeOwner<'a> {
    pub(crate) fn attributes(&self) -> &'a [AttributeDecl] {
        match self {
            AttributeOwner::Node(n) => &n.attributes,
            AttributeOwner::Reuse(r) => &r.attributes,
        }
    }

    pub(crate) fn attributes_span(&self) -> Option<&'a ByteRange<usize>> {
        match self {
            AttributeOwner::Node(n) => n.attributes_span.as_ref(),
            AttributeOwner::Reuse(r) => r.attributes_span.as_ref(),
        }
    }

    pub(crate) fn span(&self) -> ByteRange<usize> {
        match self {
            AttributeOwner::Node(n) => n.span.clone(),
            AttributeOwner::Reuse(r) => r.span.clone(),
        }
    }
}

pub(crate) struct AttributeEntry<'a> {
    pub(crate) decl: &'a AttributeDecl,
    pub(crate) owner: AttributeOwner<'a>,
    /// The node it is an attribute of.
    pub(crate) node: Option<String>,
    pub(crate) scope: String,
    pub(crate) action: RuleAction,
}

pub(crate) struct EdgeEntry<'a> {
    pub(crate) decl: &'a EdgeDecl,
    pub(crate) list: &'a EdgesDecl,
    pub(crate) scope: String,
    pub(crate) source: Option<String>,
    pub(crate) target: Option<String>,
    pub(crate) action: RuleAction,
}

/// An element of a unit's body.
pub(crate) struct StepEntry<'a> {
    pub(crate) item: &'a UnitItem,
    pub(crate) unit: String,
    /// The list it is in, and whether that list is written on one line (`independent [...]`).
    pub(crate) siblings: &'a [UnitItem],
    pub(crate) inline: bool,
}

/// The elements of a transformation document by their diagram ids.
pub(crate) struct Index<'a> {
    pub(crate) rules: IndexMap<String, &'a RuleDecl>,
    pub(crate) units: IndexMap<String, &'a UnitDecl>,
    pub(crate) scopes: IndexMap<String, ScopeEntry<'a>>,
    pub(crate) nodes: IndexMap<String, NodeEntry<'a>>,
    pub(crate) attributes: IndexMap<String, AttributeEntry<'a>>,
    pub(crate) edges: IndexMap<String, EdgeEntry<'a>>,
    pub(crate) steps: IndexMap<String, StepEntry<'a>>,
    /// The `reuse`s, with their scope and the node they reuse.
    pub(crate) reuses: Vec<(String, &'a ReuseDecl, Option<String>)>,
    /// Matching formulas with the scope they are in and the ids of their condition graphs.
    pub(crate) formulas: Vec<(String, &'a FormulaDecl, Vec<String>)>,
    alloc: IdAllocator,
}

/// The action of each condition graph a formula names: `forbid` where it is negated in a
/// conjunction, `require` otherwise (Henshin draws more complex formulas' graphs as `require`).
fn polarities(l: &Logic) -> HashMap<String, RuleAction> {
    fn walk(l: &Logic, negated: bool, pure: bool, out: &mut HashMap<String, RuleAction>) {
        match l {
            Logic::Ref(n) => {
                let a = if negated && pure { RuleAction::Forbid } else { RuleAction::Require };
                out.entry(n.text.clone()).or_insert(a);
            }
            Logic::Not(inner) => walk(inner, !negated, pure, out),
            Logic::And(a, b) => {
                walk(a, negated, pure && !negated, out);
                walk(b, negated, pure && !negated, out);
            }
            Logic::Or(a, b) | Logic::Xor(a, b) => {
                walk(a, negated, false, out);
                walk(b, negated, false, out);
            }
            Logic::Error => {}
        }
    }
    let mut out = HashMap::new();
    walk(l, false, true, &mut out);
    out
}

impl<'a> Index<'a> {
    pub(crate) fn new(doc: &'a Document) -> Self {
        let mut ix = Index {
            rules: IndexMap::new(),
            units: IndexMap::new(),
            scopes: IndexMap::new(),
            nodes: IndexMap::new(),
            attributes: IndexMap::new(),
            edges: IndexMap::new(),
            steps: IndexMap::new(),
            reuses: Vec::new(),
            formulas: Vec::new(),
            alloc: IdAllocator::default(),
        };
        for e in &doc.elements {
            match e {
                Element::Rule(r) => {
                    let id = ix.alloc.next(r.name.text.clone());
                    ix.rules.insert(id.clone(), r);
                    ix.scope(
                        id.clone(),
                        ScopeDecl::Rule(r),
                        &id,
                        None,
                        HashMap::new(),
                        None,
                        None,
                        RuleAction::Preserve,
                    );
                }
                Element::Unit(u) => {
                    let id = ix.alloc.next(u.name.text.clone());
                    ix.units.insert(id.clone(), u);
                    ix.steps(&id, &format!("{id}@"), &u.items, false);
                }
            }
        }
        ix
    }

    #[allow(clippy::too_many_arguments)]
    fn scope(
        &mut self,
        id: String,
        decl: ScopeDecl<'a>,
        rule: &str,
        parent: Option<String>,
        inherited: HashMap<String, String>,
        multi: Option<String>,
        condition: Option<String>,
        polarity: RuleAction,
    ) {
        let items = decl.graph_items();
        let mut names = inherited;
        let default_action = |n: &NodeDecl| n.action.as_ref().map_or(polarity, |(a, _)| rule_action(*a));
        let mut own = Vec::new();
        for item in &items {
            if let GraphItem::Node(n) = item {
                let nid = self.alloc.next(format!("{id}::{}", n.name.text));
                names.insert(n.name.text.clone(), nid.clone());
                self.nodes.insert(nid.clone(), NodeEntry { decl: n, scope: id.clone(), action: default_action(n) });
                own.push((nid, n));
            }
        }
        for (nid, n) in own {
            let n: &'a NodeDecl = n;
            let node_action = default_action(n);
            for a in &n.attributes {
                let aid = self.alloc.next(format!("{id}::{}.{}", n.name.text, a.name.text()));
                let action = a.action.as_ref().map_or(node_action, |(x, _)| rule_action(*x));
                self.attributes.insert(
                    aid,
                    AttributeEntry {
                        decl: a,
                        owner: AttributeOwner::Node(n),
                        node: Some(nid.clone()),
                        scope: id.clone(),
                        action,
                    },
                );
            }
        }
        for item in &items {
            if let GraphItem::Reuse(r) = item {
                let node = names.get(&r.name.text).cloned();
                let node_action = node.as_ref().and_then(|n| self.nodes.get(n)).map_or(polarity, |n| n.action);
                for a in &r.attributes {
                    let aid = self.alloc.next(format!("{id}::{}.{}", r.name.text, a.name.text()));
                    let own_action = if condition.is_some() { polarity } else { node_action };
                    let action = a.action.as_ref().map_or(own_action, |(x, _)| rule_action(*x));
                    self.attributes.insert(
                        aid,
                        AttributeEntry {
                            decl: a,
                            owner: AttributeOwner::Reuse(r),
                            node: node.clone(),
                            scope: id.clone(),
                            action,
                        },
                    );
                }
                self.reuses.push((id.clone(), r, node));
            }
        }
        for item in &items {
            if let GraphItem::Edges(list) = item {
                for e in &list.edges {
                    let eid = self.alloc.next(format!("{id}::{}->{}:{}", e.source.text, e.target.text, e.ty.text()));
                    let action = e.action.as_ref().map_or(polarity, |(a, _)| rule_action(*a));
                    self.edges.insert(
                        eid,
                        EdgeEntry {
                            decl: e,
                            list,
                            scope: id.clone(),
                            source: names.get(&e.source.text).cloned(),
                            target: names.get(&e.target.text).cloned(),
                            action,
                        },
                    );
                }
            }
        }
        self.scopes.insert(
            id.clone(),
            ScopeEntry {
                decl,
                rule: rule.to_string(),
                parent: parent.clone(),
                names: names.clone(),
                multi: multi.clone(),
                condition: condition.clone(),
                polarity,
            },
        );
        for item in &items {
            match item {
                GraphItem::MultiRule(m) => {
                    let mid = self.alloc.next(format!("{id}/{}", m.name.text));
                    let scope = ScopeDecl::Multi(m);
                    self.scope(
                        mid.clone(),
                        scope,
                        rule,
                        Some(id.clone()),
                        names.clone(),
                        Some(mid),
                        None,
                        RuleAction::Preserve,
                    );
                }
                GraphItem::Formula(f) => {
                    let actions = polarities(&f.formula);
                    let mut graphs = Vec::new();
                    for g in &f.graphs {
                        let gid = self.alloc.next(format!("{id}#{}", g.name.text));
                        let action = actions.get(&g.name.text).copied().unwrap_or(RuleAction::Require);
                        graphs.push(gid.clone());
                        let scope = ScopeDecl::Condition(g);
                        self.scope(
                            gid.clone(),
                            scope,
                            rule,
                            Some(id.clone()),
                            names.clone(),
                            multi.clone(),
                            Some(gid),
                            action,
                        );
                    }
                    self.formulas.push((id.clone(), f, graphs));
                }
                _ => {}
            }
        }
    }

    fn steps(&mut self, unit: &str, prefix: &str, items: &'a [UnitItem], inline: bool) {
        for (i, item) in items.iter().enumerate() {
            let id = self.alloc.next(format!("{prefix}{}", i + 1));
            self.steps.insert(id.clone(), StepEntry { item, unit: unit.to_string(), siblings: items, inline });
            for (j, (list, inline)) in step_lists(item).into_iter().enumerate() {
                self.steps(unit, &format!("{id}.{}.", j + 1), list, inline);
            }
        }
    }

    /// The innermost element whose definition contains `offset`.
    pub(crate) fn element_at(&self, offset: usize) -> Option<String> {
        let spans = self
            .rules
            .iter()
            .map(|(id, r)| (id, r.span.clone()))
            .chain(self.scopes.iter().map(|(id, s)| (id, s.decl.span())))
            .chain(self.nodes.iter().map(|(id, n)| (id, n.decl.span.clone())))
            .chain(self.attributes.iter().map(|(id, a)| (id, a.decl.span.clone())))
            .chain(self.edges.iter().map(|(id, e)| (id, e.decl.span.clone())))
            .chain(self.units.iter().map(|(id, u)| (id, u.span.clone())))
            .chain(self.steps.iter().map(|(id, s)| (id, s.item.span().clone())));
        spans
            .filter(|(_, s)| s.start <= offset && offset <= s.end)
            .min_by_key(|(_, s)| s.end - s.start)
            .map(|(id, _)| id.clone())
    }
}

/// The lists of unit elements nested in an element, and whether each is written on one line.
pub(crate) fn step_lists(item: &UnitItem) -> Vec<(&[UnitItem], bool)> {
    match item {
        UnitItem::Sequence(items, _) => vec![(items.as_slice(), false)],
        UnitItem::Independent(lists, _) | UnitItem::Priority(lists, _) => {
            lists.iter().map(|l| (l.as_slice(), true)).collect()
        }
        UnitItem::If { condition, then, otherwise, .. } => {
            let mut out = vec![(condition.as_slice(), true), (then.as_slice(), false)];
            if let Some(o) = otherwise {
                out.push((o.as_slice(), false));
            }
            out
        }
        UnitItem::For { body, .. } | UnitItem::While { body, .. } => vec![(body.as_slice(), false)],
        UnitItem::Call { .. } | UnitItem::Strict(..) | UnitItem::Rollback(..) => Vec::new(),
    }
}

// ---------------------------------------------------------------------- the diagram

/// Ranges in a document's text.
pub(crate) struct Ranges<'t> {
    text: &'t str,
    index: LineIndex,
}

impl<'t> Ranges<'t> {
    pub(crate) fn new(text: &'t str) -> Self {
        Ranges { text, index: LineIndex::new(text) }
    }

    pub(crate) fn of(&self, span: &ByteRange<usize>) -> Range {
        let end = span.end.min(self.text.len());
        byte_range(self.text, &self.index, span.start.min(end)..end)
    }
}

/// The class a node's class name resolves to (imported packages first), as the compiler
/// resolves it.
pub(crate) fn resolve_class(mm: &Metamodel, packages: &[PackageId], name: &QName) -> Option<ClassifierId> {
    let segments = name.segments();
    if segments.iter().any(|s| s.is_empty()) || segments.is_empty() {
        return None;
    }
    let mut found: Vec<ClassifierId> =
        mm.find_classifiers_qualified(&segments).into_iter().filter(|c| mm.is_class(*c)).collect();
    found.sort_by_key(|c| !is_imported(mm, packages, mm.classifier(*c).package));
    found.first().copied()
}

fn is_imported(mm: &Metamodel, packages: &[PackageId], mut p: PackageId) -> bool {
    loop {
        if packages.contains(&p) {
            return true;
        }
        match mm.package(p).super_package {
            Some(sp) => p = sp,
            None => return false,
        }
    }
}

/// How rules write a class: by its name, qualified (with dots) if the name is ambiguous.
pub(crate) fn class_label(mm: &Metamodel, packages: &[PackageId], c: ClassifierId) -> String {
    let name = &mm.classifier(c).name;
    let found = mm.find_classifiers(name);
    let imported: Vec<&ClassifierId> =
        found.iter().filter(|x| is_imported(mm, packages, mm.classifier(**x).package)).collect();
    if found.len() == 1 || (imported.len() == 1 && *imported[0] == c) {
        name.clone()
    } else {
        mm.qualified_name(c).replace("::", ".")
    }
}

/// The Ecore data types parameters usually have.
const DATA_TYPES: &[&str] =
    &["EString", "EInt", "EBoolean", "EDouble", "EFloat", "ELong", "EShort", "EByte", "EChar", "EDate"];

fn parameters(params: &[ParamDecl]) -> Vec<TransformationParameterSpec> {
    params
        .iter()
        .map(|p| TransformationParameterSpec {
            kind: kind_name(p.kind).into(),
            name: p.name.text.clone(),
            ty: (!p.ty.parts.is_empty()).then(|| p.ty.text()),
        })
        .collect()
}

/// The options of a rule or multi-rule: `checkDangling`, `injectiveMatching` (with the
/// defaults given), and the attribute conditions.
fn options(items: &[RuleItem], dangling: bool, injective: bool) -> (bool, bool, Vec<String>, Vec<String>) {
    let (mut d, mut i, mut conditions, mut imports) = (dangling, injective, Vec::new(), Vec::new());
    for item in items {
        match item {
            RuleItem::CheckDangling(v, _) => d = *v,
            RuleItem::InjectiveMatching(v, _) => i = *v,
            RuleItem::Conditions(c, _) => conditions.extend(c.iter().map(print::expr)),
            RuleItem::JavaImport(q, _) => imports.push(q.text()),
            RuleItem::Graph(_) => {}
        }
    }
    (d, i, conditions, imports)
}

impl Workspace {
    /// The rules and units of a transformation document (of either syntax), for drawing them as
    /// diagrams in Henshin's notation.
    pub fn transformation_diagram(&self, name: &str) -> Result<TransformationDiagram, Error> {
        let (syntax, language) = self.transformation_syntax(name)?;
        let text = self.document_text(name).unwrap_or_default();
        Ok(self.draw(&syntax, language, text))
    }

    fn draw(&self, syntax: &Syntax, language: Language, text: &str) -> TransformationDiagram {
        let mm = self.metamodel();
        let module = Module::from_document(mm, &syntax.document, syntax.problems.clone());
        let ix = Index::new(&syntax.document);
        let ranges = Ranges::new(text);
        let packages = &module.packages;
        let mut rules = Vec::new();
        for (rid, r) in &ix.rules {
            let (check_dangling, injective_matching, conditions, java_imports) = options(&r.items, true, true);
            let params: Vec<&str> = r.params.iter().map(|p| p.name.text.as_str()).collect();
            let mut multi_rules = Vec::new();
            let mut flags: HashMap<&str, (bool, bool)> =
                HashMap::from([(rid.as_str(), (check_dangling, injective_matching))]);
            for (sid, s) in ix.scopes.iter().filter(|(_, s)| s.rule == *rid) {
                if let ScopeDecl::Multi(m) = s.decl {
                    let kernel = s.parent.clone().unwrap_or_default();
                    let (kd, ki) = flags.get(kernel.as_str()).copied().unwrap_or((true, true));
                    let (d, i, conditions, _) = options(&m.items, kd, ki);
                    flags.insert(sid.as_str(), (d, i));
                    multi_rules.push(MultiRuleDiagram {
                        id: sid.clone(),
                        name: m.name.text.clone(),
                        kernel,
                        check_dangling: d,
                        injective_matching: i,
                        conditions,
                        range: ranges.of(&m.span),
                    });
                }
            }
            let formulas = ix
                .formulas
                .iter()
                .filter(|(scope, _, _)| ix.scopes.get(scope).is_some_and(|s| s.rule == *rid))
                .map(|(scope, f, graphs)| FormulaDiagram {
                    scope: scope.clone(),
                    formula: print::logic(&f.formula),
                    graphs: graphs.clone(),
                    range: ranges.of(&f.span),
                })
                .collect();
            let mut nodes = Vec::new();
            for (nid, n) in ix.nodes.iter().filter(|(_, n)| ix.scopes.get(&n.scope).is_some_and(|s| s.rule == *rid)) {
                let scope = &ix.scopes[&n.scope];
                let class = resolve_class(mm, packages, &n.decl.class);
                let attributes = ix
                    .attributes
                    .iter()
                    .filter(|(_, a)| a.node.as_deref() == Some(nid.as_str()))
                    .map(|(aid, a)| RuleAttributeDiagram {
                        id: aid.clone(),
                        name: a.decl.name.text(),
                        value: print::expr(&a.decl.value),
                        action: a.action,
                        set: a.decl.set,
                        scope: (a.scope != n.scope).then(|| a.scope.clone()),
                        range: ranges.of(&a.decl.span),
                    })
                    .collect();
                nodes.push(RuleNodeDiagram {
                    id: nid.clone(),
                    name: n.decl.name.text.clone(),
                    class: n.decl.class.text(),
                    class_id: class.map(|c| mm.qualified_name(c)),
                    action: n.action,
                    multi: scope.multi.clone(),
                    condition: scope.condition.clone(),
                    attributes,
                    parameter: n.scope == *rid && params.contains(&n.decl.name.text.as_str()),
                    range: ranges.of(&n.decl.span),
                });
            }
            let edges = ix
                .edges
                .iter()
                .filter(|(_, e)| ix.scopes.get(&e.scope).is_some_and(|s| s.rule == *rid))
                .filter_map(|(eid, e)| {
                    let scope = &ix.scopes[&e.scope];
                    Some(RuleEdgeDiagram {
                        id: eid.clone(),
                        source: e.source.clone()?,
                        target: e.target.clone()?,
                        reference: e.decl.ty.text(),
                        action: e.action,
                        multi: scope.multi.clone(),
                        condition: scope.condition.clone(),
                        range: ranges.of(&e.decl.span),
                    })
                })
                .collect();
            rules.push(RuleDiagram {
                id: rid.clone(),
                name: r.name.text.clone(),
                parameters: parameters(&r.params),
                check_dangling,
                injective_matching,
                conditions,
                java_imports,
                multi_rules,
                formulas,
                nodes,
                edges,
                range: ranges.of(&r.span),
            });
        }
        let units = ix
            .units
            .iter()
            .map(|(uid, u)| {
                let mut text = String::new();
                print::Printer::with_indent("\t").unit_items(&mut text, 0, &u.items);
                UnitDiagram {
                    id: uid.clone(),
                    name: u.name.text.clone(),
                    parameters: parameters(&u.params),
                    body: self.steps_diagram(&ix, &format!("{uid}@"), &u.items, &ranges),
                    text,
                    range: ranges.of(&u.span),
                }
            })
            .collect();
        let problems = module
            .problems
            .iter()
            .map(|p| DiagramProblem {
                severity: if p.is_error() { Severity::Error } else { Severity::Warning },
                message: p.message.clone(),
                element: ix.element_at(p.span.start),
                range: ranges.of(&p.span),
            })
            .collect();
        TransformationDiagram {
            syntax: if language == Language::HenshinModel {
                TransformationSyntax::Model
            } else {
                TransformationSyntax::Text
            },
            imports: syntax.document.imports.iter().map(print::import).collect(),
            packages: importable_packages(mm),
            rules,
            units,
            classes: node_classes(mm, packages),
            types: parameter_types(mm, packages),
            problems,
        }
    }

    fn steps_diagram(&self, ix: &Index, prefix: &str, items: &[UnitItem], ranges: &Ranges) -> Vec<UnitStepDiagram> {
        items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let id = format!("{prefix}{}", i + 1);
                // The index allocated ids in the same order: find this item's.
                let id = ix.steps.iter().find(|(_, s)| std::ptr::eq(s.item, item)).map_or(id, |(k, _)| k.clone());
                let children = step_lists(item)
                    .into_iter()
                    .enumerate()
                    .map(|(j, (list, _))| self.steps_diagram(ix, &format!("{id}.{}.", j + 1), list, ranges))
                    .collect();
                let mut step = UnitStepDiagram {
                    id,
                    kind: String::new(),
                    target: None,
                    arguments: Vec::new(),
                    count: None,
                    value: None,
                    children,
                    range: ranges.of(item.span()),
                };
                step.kind = match item {
                    UnitItem::Call { target, args, .. } => {
                        step.target = Some(target.text.clone());
                        step.arguments = args.iter().map(|a| a.text.clone()).collect();
                        "call"
                    }
                    UnitItem::Sequence(..) => "sequence",
                    UnitItem::Independent(..) => "independent",
                    UnitItem::Priority(..) => "priority",
                    UnitItem::If { .. } => "if",
                    UnitItem::For { count, .. } => {
                        step.count = Some(print::expr(count));
                        "for"
                    }
                    UnitItem::While { .. } => "while",
                    UnitItem::Strict(v, _) => {
                        step.value = Some(*v);
                        "strict"
                    }
                    UnitItem::Rollback(v, _) => {
                        step.value = Some(*v);
                        "rollback"
                    }
                }
                .to_string();
                step
            })
            .collect()
    }
}

/// The classes of the metamodel's user packages (the imported ones first), with their
/// features.
/// The names of the top-level packages, the user's first (Ecore's last), each once.
fn importable_packages(mm: &Metamodel) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for p in mm.user_root_packages().chain([mm.ecore_package()]) {
        let name = &mm.package(p).name;
        if !names.contains(name) {
            names.push(name.clone());
        }
    }
    names
}

fn node_classes(mm: &Metamodel, packages: &[PackageId]) -> Vec<NodeClass> {
    let mut classes: Vec<ClassifierId> =
        mm.classifier_ids().filter(|c| mm.is_class(*c) && mm.classifier(*c).package != mm.ecore_package()).collect();
    classes.sort_by_key(|c| !is_imported(mm, packages, mm.classifier(*c).package));
    classes
        .into_iter()
        .map(|c| {
            let data = mm.class_data(c);
            let mut conforms_to = vec![mm.qualified_name(c)];
            conforms_to.extend(mm.all_super_types(c).iter().map(|s| mm.qualified_name(*s)));
            let mut attributes = Vec::new();
            let mut references = Vec::new();
            for f in mm.all_features(c) {
                let feature = mm.feature(*f);
                match feature.kind {
                    FeatureKind::Reference { .. } => {
                        if let Some(t) = feature.ty {
                            references.push(NodeClassReference {
                                name: feature.name.clone(),
                                target: mm.qualified_name(t),
                                containment: feature.is_containment(),
                                many: feature.multiplicity.upper != 1,
                            });
                        }
                    }
                    FeatureKind::Attribute { .. } => attributes.push(NodeClassAttribute {
                        name: feature.name.clone(),
                        ty: feature.ty.map_or_else(String::new, |t| mm.classifier(t).name.clone()),
                        literals: feature
                            .ty
                            .map(|t| mm.enum_literals(t).iter().map(|l| l.name.clone()).collect())
                            .unwrap_or_default(),
                    }),
                }
            }
            NodeClass {
                id: mm.qualified_name(c),
                name: class_label(mm, packages, c),
                is_abstract: data.is_some_and(|d| d.is_abstract || d.is_interface),
                conforms_to,
                attributes,
                references,
            }
        })
        .collect()
}

/// The types parameters can have: Ecore's usual data types, then the enumerations and classes
/// of the imported packages.
fn parameter_types(mm: &Metamodel, packages: &[PackageId]) -> Vec<String> {
    let mut out: Vec<String> = DATA_TYPES.iter().map(|s| s.to_string()).collect();
    for c in mm.classifier_ids() {
        if mm.classifier(c).package != mm.ecore_package() && is_imported(mm, packages, mm.classifier(c).package) {
            out.push(class_label(mm, packages, c));
        }
    }
    out
}
