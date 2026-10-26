//! Compiled transformation modules: the rules and units of a Henshin text, resolved against a
//! metamodel.
//!
//! A rule's nodes are its left- and right-hand side: *preserved* and *deleted* nodes are
//! matched, *created* nodes are added. `forbid` and `require` elements become one negative and
//! one positive application condition (as in Henshin's textual syntax); a `matchingFormula`
//! combines named condition graphs with `AND`, `OR`, `XOR` and `!`. Multi-rules (amalgamation)
//! start with copies of their kernel rule's nodes, so the nodes of a kernel and a multi-rule
//! share indices.

use std::collections::HashMap;
use std::ops::Range;

use oxy_ecore::{ClassifierId, FeatureId, Metamodel, Multiplicity, PackageId, Problem};

use crate::syntax::{self, Action, Element, Expr, ExprKind, GraphItem, Logic, ParamKind, QName, RuleItem, UnitItem};
use crate::value::{CExpr, Value};

/// A rule or unit of a module.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Callable {
    Rule(usize),
    Unit(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ParamType {
    /// An Ecore data type (or enumeration).
    Data(ClassifierId),
    /// A class: the parameter is a node.
    Class(ClassifierId),
    /// A type that could not be resolved.
    Unknown(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub kind: ParamKind,
    pub ty: ParamType,
    pub span: Range<usize>,
}

impl Param {
    pub fn type_name(&self, mm: &Metamodel) -> String {
        match &self.ty {
            ParamType::Data(c) | ParamType::Class(c) => mm.classifier(*c).name.clone(),
            ParamType::Unknown(name) => name.clone(),
        }
    }

    /// `IN name : Type` (`IN name` for a parameter without a type).
    pub fn signature(&self, mm: &Metamodel) -> String {
        let kind = self.kind.keyword();
        let ty = self.type_name(mm);
        let ty = if ty.is_empty() { String::new() } else { format!(" : {ty}") };
        format!("{kind}{}{}{ty}", if kind.is_empty() { "" } else { " " }, self.name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeKind {
    Preserve,
    Delete,
    Create,
}

impl NodeKind {
    pub fn is_lhs(self) -> bool {
        self != NodeKind::Create
    }

    pub fn keyword(self) -> &'static str {
        match self {
            NodeKind::Preserve => "preserve",
            NodeKind::Delete => "delete",
            NodeKind::Create => "create",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RuleNode {
    pub name: String,
    pub class: ClassifierId,
    pub kind: NodeKind,
    /// Attribute values the matched object must have (left-hand side).
    pub matches: Vec<(FeatureId, CExpr)>,
    /// Attribute values set by the rule (right-hand side).
    pub sets: Vec<(FeatureId, CExpr)>,
    /// Attributes deleted (unset) by the rule.
    pub unsets: Vec<FeatureId>,
    /// The parameter naming this node.
    pub param: Option<usize>,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RuleEdge {
    /// Node indices.
    pub source: usize,
    pub feature: FeatureId,
    pub target: usize,
    pub kind: NodeKind,
    pub span: Range<usize>,
}

/// A node of a condition graph, matched in a match slot.
#[derive(Clone, Debug, PartialEq)]
pub struct PatternNode {
    pub name: String,
    pub slot: usize,
    pub class: ClassifierId,
    pub matches: Vec<(FeatureId, CExpr)>,
}

/// A graph that must (or, negated, must not) extend a match.
#[derive(Clone, Debug, PartialEq)]
pub struct ConditionGraph {
    pub name: String,
    pub nodes: Vec<PatternNode>,
    /// Attribute values required of nodes matched before (by slot).
    pub checks: Vec<(usize, FeatureId, CExpr)>,
    /// Links between slots.
    pub edges: Vec<(usize, FeatureId, usize)>,
    /// A nested condition on the extended match.
    pub condition: Condition,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
    True,
    Graph(Box<ConditionGraph>),
    Not(Box<Condition>),
    And(Vec<Condition>),
    Or(Box<Condition>, Box<Condition>),
    Xor(Box<Condition>, Box<Condition>),
}

impl Condition {
    fn and(conditions: Vec<Condition>) -> Condition {
        let mut c: Vec<Condition> = conditions.into_iter().filter(|c| *c != Condition::True).collect();
        match c.len() {
            0 => Condition::True,
            1 => c.pop().unwrap_or(Condition::True),
            _ => Condition::And(c),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub name: String,
    pub span: Range<usize>,
    pub name_span: Range<usize>,
    /// Shared by a kernel rule and its multi-rules.
    pub params: Vec<Param>,
    pub check_dangling: bool,
    pub injective: bool,
    /// Java packages imported for attribute expressions (`javaImport`).
    pub java_imports: Vec<String>,
    /// Match slot `i` holds the object matched by node `i`.
    pub nodes: Vec<RuleNode>,
    /// `nodes[..inherited]` are copies of the kernel rule's nodes (multi-rules only).
    pub inherited: usize,
    pub edges: Vec<RuleEdge>,
    /// Attribute conditions (`conditions [...]`).
    pub conditions: Vec<CExpr>,
    pub condition: Condition,
    /// The number of match slots: nodes and condition graph nodes.
    pub slots: usize,
    pub multi_rules: Vec<Rule>,
}

impl Rule {
    pub fn signature(&self, mm: &Metamodel) -> String {
        let params: Vec<String> = self.params.iter().map(|p| p.signature(mm)).collect();
        format!("rule {}({})", self.name, params.join(", "))
    }

    /// The node with this name.
    pub fn node(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.name == name)
    }
}

/// A unit's control flow.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Calls a rule or unit; `args` are parameters of the calling unit, passed by position.
    Call {
        target: Callable,
        args: Vec<usize>,
        span: Range<usize>,
    },
    Sequence {
        steps: Vec<Step>,
        strict: bool,
        rollback: bool,
    },
    /// Applies one of the steps, chosen at random.
    Independent(Vec<Step>),
    /// Applies the first step that can be applied.
    Priority(Vec<Step>),
    If {
        condition: Box<Step>,
        then: Box<Step>,
        otherwise: Option<Box<Step>>,
    },
    For {
        count: CExpr,
        body: Box<Step>,
    },
    While(Box<Step>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Unit {
    pub name: String,
    pub span: Range<usize>,
    pub name_span: Range<usize>,
    pub params: Vec<Param>,
    pub body: Step,
}

impl Unit {
    pub fn signature(&self, mm: &Metamodel) -> String {
        let params: Vec<String> = self.params.iter().map(|p| p.signature(mm)).collect();
        format!("unit {}({})", self.name, params.join(", "))
    }
}

/// The names visible in a region of the document (for completion).
#[derive(Clone, Debug, PartialEq)]
pub struct Scope {
    pub span: Range<usize>,
    /// Nodes by name, with their class.
    pub nodes: Vec<(String, ClassifierId)>,
    /// Parameters of the enclosing rule or unit.
    pub params: Vec<Param>,
    /// Condition graphs of the enclosing formula.
    pub graphs: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Module {
    /// The imported packages.
    pub packages: Vec<PackageId>,
    pub rules: Vec<Rule>,
    pub units: Vec<Unit>,
    pub problems: Vec<Problem>,
    /// Descriptions of the names in the document, for hovers.
    pub symbols: Vec<(Range<usize>, String)>,
    pub scopes: Vec<Scope>,
}

impl Module {
    /// Parses and resolves a Henshin text.
    pub fn compile(mm: &Metamodel, text: &str) -> Module {
        let (doc, problems) = syntax::parse(text);
        Module::from_document(mm, &doc, problems)
    }

    pub fn from_document(mm: &Metamodel, doc: &syntax::Document, problems: Vec<Problem>) -> Module {
        let mut c = Compiler { mm, module: Module { problems, ..Module::default() }, callables: HashMap::new() };
        c.imports(&doc.imports);
        c.declare(doc);
        for element in &doc.elements {
            match element {
                Element::Rule(r) => {
                    let rule = c.rule(r);
                    c.module.rules.push(rule);
                }
                Element::Unit(u) => {
                    let unit = c.unit(u);
                    c.module.units.push(unit);
                }
            }
        }
        c.module
    }

    pub fn callable(&self, name: &str) -> Option<Callable> {
        self.rules
            .iter()
            .position(|r| r.name == name)
            .map(Callable::Rule)
            .or_else(|| self.units.iter().position(|u| u.name == name).map(Callable::Unit))
    }

    pub fn name_of(&self, c: Callable) -> &str {
        match c {
            Callable::Rule(i) => &self.rules[i].name,
            Callable::Unit(i) => &self.units[i].name,
        }
    }

    pub fn params(&self, c: Callable) -> &[Param] {
        match c {
            Callable::Rule(i) => &self.rules[i].params,
            Callable::Unit(i) => &self.units[i].params,
        }
    }

    pub fn has_errors(&self) -> bool {
        self.problems.iter().any(Problem::is_error)
    }

    /// The description of the innermost name at `offset`.
    pub fn symbol_at(&self, offset: usize) -> Option<(&Range<usize>, &str)> {
        self.symbols
            .iter()
            .filter(|(span, _)| span.start <= offset && offset <= span.end && span.start < span.end)
            .min_by_key(|(span, _)| span.end - span.start)
            .map(|(span, text)| (span, text.as_str()))
    }

    /// The innermost scope at `offset`.
    pub fn scope_at(&self, offset: usize) -> Option<&Scope> {
        self.scopes
            .iter()
            .filter(|s| s.span.start <= offset && offset <= s.span.end)
            .min_by_key(|s| s.span.end - s.span.start)
    }
}

/// `Type[*]`, `Type[0..1]` etc.
pub fn type_text(mm: &Metamodel, ty: Option<ClassifierId>, m: &Multiplicity) -> String {
    let name = ty.map_or_else(|| "?".to_string(), |t| mm.classifier(t).name.clone());
    let upper = |u: i32| if u < 0 { "*".to_string() } else { u.to_string() };
    match (m.lower, m.upper) {
        (1, 1) => name,
        (0, 1) => format!("{name}[0..1]"),
        (0, u) if u < 0 => format!("{name}[*]"),
        (l, u) => format!("{name}[{l}..{}]", upper(u)),
    }
}

/// A description of a structural feature, for hovers.
pub fn feature_text(mm: &Metamodel, f: FeatureId) -> String {
    let feature = mm.feature(f);
    let kind = if feature.is_reference() { "reference" } else { "attribute" };
    let mut text = format!(
        "{kind} {}::{} : {}",
        mm.classifier(feature.owner).name,
        feature.name,
        type_text(mm, feature.ty, &feature.multiplicity)
    );
    let mut notes = Vec::new();
    if feature.is_containment() {
        notes.push("containment".to_string());
    }
    if let Some(op) = feature.opposite() {
        notes.push(format!("opposite {}", mm.feature(op).name));
    }
    if !notes.is_empty() {
        text.push_str(&format!(" ({})", notes.join(", ")));
    }
    text
}

/// What a name in a rule's graph refers to while compiling.
#[derive(Clone, Copy, Debug, PartialEq)]
enum NodeRef {
    /// A node of the rule (left- or right-hand side).
    Rule(usize),
    /// A node of the negative (`forbid`) or positive (`require`) application condition.
    Nac(usize),
    Pac(usize),
}

/// A node of an application condition being compiled.
struct ConditionNode {
    name: String,
    class: ClassifierId,
    matches: Vec<(FeatureId, CExpr)>,
}

#[derive(Default)]
struct ActionCondition {
    nodes: Vec<ConditionNode>,
    checks: Vec<(usize, FeatureId, CExpr)>,
    /// Endpoints: rule nodes or condition nodes.
    edges: Vec<(NodeRef, FeatureId, NodeRef)>,
}

impl ActionCondition {
    fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.checks.is_empty() && self.edges.is_empty()
    }
}

struct Compiler<'a> {
    mm: &'a Metamodel,
    module: Module,
    /// Rules and units by name, with their parameter counts.
    callables: HashMap<String, (Callable, usize)>,
}

impl Compiler<'_> {
    fn error(&mut self, span: &Range<usize>, message: impl Into<String>) {
        self.module.problems.push(Problem::error(span.clone(), message));
    }

    fn warning(&mut self, span: &Range<usize>, message: impl Into<String>) {
        self.module.problems.push(Problem::warning(span.clone(), message));
    }

    fn symbol(&mut self, span: &Range<usize>, text: impl Into<String>) {
        if span.start < span.end {
            self.module.symbols.push((span.clone(), text.into()));
        }
    }

    // ------------------------------------------------------------------ names

    fn imports(&mut self, imports: &[QName]) {
        for import in imports {
            let text = import.text();
            let by_uri = self.mm.package_by_ns_uri(&text);
            let found = by_uri.map(|p| vec![p]).unwrap_or_else(|| {
                let path = import.segments();
                let mut found = self.mm.find_packages_qualified(&path);
                if found.is_empty() {
                    // Also by nsPrefix.
                    found = self.mm.package_ids().filter(|p| self.mm.package(*p).ns_prefix == text).collect();
                }
                found
            });
            match found.first() {
                Some(p) => {
                    let pkg = self.mm.package(*p);
                    let text = format!("package {} ('{}')", self.mm.package_qualified_name(*p), pkg.ns_uri);
                    self.symbol(&import.span, text);
                    self.module.packages.push(*p);
                }
                None if text.is_empty() => {}
                None => self.error(
                    &import.span,
                    format!("unknown package '{text}': open or load its metamodel (.oclinecore or .ecore)"),
                ),
            }
        }
    }

    fn declare(&mut self, doc: &syntax::Document) {
        let mut rules = 0;
        let mut units = 0;
        for element in &doc.elements {
            let name = element.name();
            if name.text.is_empty() {
                continue;
            }
            let callable = match element {
                Element::Rule(_) => {
                    rules += 1;
                    Callable::Rule(rules - 1)
                }
                Element::Unit(_) => {
                    units += 1;
                    Callable::Unit(units - 1)
                }
            };
            if self.callables.contains_key(&name.text) {
                let span = name.span.clone();
                self.error(&span, format!("there is already a rule or unit named '{}'", name.text));
            } else {
                self.callables.insert(name.text.clone(), (callable, element.params().len()));
            }
        }
    }

    /// Whether a package is (in) one of the imported packages.
    fn is_imported(&self, mut p: PackageId) -> bool {
        loop {
            if self.module.packages.contains(&p) {
                return true;
            }
            match self.mm.package(p).super_package {
                Some(sp) => p = sp,
                None => return false,
            }
        }
    }

    /// A classifier by (dotted) name; imported packages first.
    fn classifier(&self, name: &QName, filter: impl Fn(ClassifierId) -> bool) -> Vec<ClassifierId> {
        let segments = name.segments();
        let mut found: Vec<ClassifierId> =
            self.mm.find_classifiers_qualified(&segments).into_iter().filter(|c| filter(*c)).collect();
        found.sort_by_key(|c| !self.is_imported(self.mm.classifier(*c).package));
        found
    }

    fn class(&mut self, name: &QName) -> Option<ClassifierId> {
        if name.last().is_empty() {
            return None;
        }
        let mm = self.mm;
        let found = self.classifier(name, |c| mm.is_class(c));
        match found.first().copied() {
            Some(c) => {
                if !self.is_imported(mm.classifier(c).package) && mm.classifier(c).package != mm.ecore_package() {
                    let package = mm.package_qualified_name(mm.classifier(c).package);
                    self.warning(
                        &name.span,
                        format!("class '{}' is in package '{package}', which is not imported", name.text()),
                    );
                }
                let data = mm.class_data(c);
                let what = if data.is_some_and(|d| d.is_interface) {
                    "interface"
                } else if data.is_some_and(|d| d.is_abstract) {
                    "abstract class"
                } else {
                    "class"
                };
                let supers: Vec<String> = mm.super_types(c).iter().map(|s| mm.classifier(*s).name.clone()).collect();
                let extends = if supers.is_empty() { String::new() } else { format!(" extends {}", supers.join(", ")) };
                let text = format!("{what} {}{extends}", mm.qualified_name(c));
                self.symbol(&name.span, text);
                Some(c)
            }
            None => {
                self.error(&name.span, format!("unknown class '{}'", name.text()));
                None
            }
        }
    }

    fn params(&mut self, decls: &[syntax::ParamDecl]) -> Vec<Param> {
        let mut params: Vec<Param> = Vec::new();
        for d in decls {
            if params.iter().any(|p| p.name == d.name.text) {
                self.error(&d.name.span, format!("duplicate parameter '{}'", d.name.text));
            }
            let mm = self.mm;
            let ecore = (d.ty.parts.len() == 1)
                .then(|| mm.find_classifier_in(mm.ecore_package(), d.ty.last()))
                .flatten()
                .filter(|c| !mm.is_class(*c));
            let ty = match ecore {
                Some(c) => ParamType::Data(c),
                None => match self.classifier(&d.ty, |_| true).first().copied() {
                    Some(c) if mm.is_class(c) => ParamType::Class(c),
                    Some(c) => ParamType::Data(c),
                    None => {
                        if !d.ty.last().is_empty() {
                            self.error(&d.ty.span, format!("unknown type '{}'", d.ty.text()));
                        }
                        ParamType::Unknown(d.ty.text())
                    }
                },
            };
            let param = Param { name: d.name.text.clone(), kind: d.kind, ty, span: d.span.clone() };
            let text = format!("parameter {}", param.signature(mm));
            self.symbol(&d.name.span, text);
            if let ParamType::Data(c) | ParamType::Class(c) = &param.ty {
                let text = format!("type {}", mm.qualified_name(*c));
                self.symbol(&d.ty.span, text);
            }
            params.push(param);
        }
        params
    }

    // ------------------------------------------------------------------ expressions

    /// Resolves an expression; `expected` is the type of the attribute it is the value of.
    fn expr(&mut self, e: &Expr, params: &[Param], expected: Option<ClassifierId>) -> CExpr {
        let mm = self.mm;
        match &e.kind {
            ExprKind::Name(name) => {
                if let Some(i) = params.iter().position(|p| p.name == *name) {
                    let text = format!("parameter {}", params[i].signature(mm));
                    self.symbol(&e.span, text);
                    return CExpr::Param(i);
                }
                if let Some(ty) = expected.filter(|t| mm.is_enum(*t))
                    && let Some(l) = mm.enum_literal_index(ty, name)
                {
                    let text = format!("literal {}::{name}", mm.qualified_name(ty));
                    self.symbol(&e.span, text);
                    return CExpr::Literal(Value::Enum(ty, l));
                }
                self.error(&e.span, format!("unknown parameter '{name}'"));
                CExpr::unsupported(format!("unknown parameter '{name}'"), e)
            }
            ExprKind::Qualified(parts) => {
                let Some((literal, path)) = parts.split_last() else { return CExpr::unsupported("empty name", e) };
                let path: Vec<&str> = path.iter().map(String::as_str).collect();
                let enums = mm.find_classifiers_qualified(&path);
                if let Some((ty, l)) = enums
                    .iter()
                    .filter(|c| mm.is_enum(**c))
                    .find_map(|c| Some((*c, mm.enum_literal_index(*c, literal)?)))
                {
                    let text = format!("literal {}::{literal}", mm.qualified_name(ty));
                    self.symbol(&e.span, text);
                    return CExpr::Literal(Value::Enum(ty, l));
                }
                let message = format!("'{}' is a Java attribute, which this engine cannot evaluate", parts.join("."));
                self.warning(&e.span, message.clone());
                CExpr::unsupported(message, e)
            }
            ExprKind::Call(..) if crate::js::script(e).is_some() => {
                let message = "JavaScript expressions of .henshin files cannot be evaluated by this engine";
                self.warning(&e.span, message);
                CExpr::unsupported(message, e)
            }
            ExprKind::Call(name, _) => {
                let message = format!("'{name}(...)' is a Java call, which this engine cannot evaluate");
                self.warning(&e.span, message.clone());
                CExpr::unsupported(message, e)
            }
            ExprKind::Str(s) => CExpr::Literal(Value::Str(s.clone())),
            ExprKind::Int(i) => CExpr::Literal(Value::Int(*i)),
            ExprKind::Real(r) => CExpr::Literal(Value::Real(*r)),
            ExprKind::Bool(b) => CExpr::Literal(Value::Bool(*b)),
            ExprKind::Not(inner) => CExpr::Not(Box::new(self.expr(inner, params, None))),
            ExprKind::Neg(inner) => match self.expr(inner, params, None) {
                CExpr::Literal(Value::Int(i)) => CExpr::Literal(Value::Int(-i)),
                CExpr::Literal(Value::Real(r)) => CExpr::Literal(Value::Real(-r)),
                other => CExpr::Neg(Box::new(other)),
            },
            ExprKind::Binary(op, l, r) => {
                // Literals compared with an enumeration-typed side are literals of that enum.
                let l = self.expr(l, params, expected);
                let r = self.expr(r, params, expected);
                CExpr::Binary(*op, Box::new(l), Box::new(r))
            }
            ExprKind::Error => CExpr::unsupported("syntax error", e),
        }
    }

    // ------------------------------------------------------------------ rules

    fn rule(&mut self, decl: &syntax::RuleDecl) -> Rule {
        let params = self.params(&decl.params);
        let mut rule = Rule {
            name: decl.name.text.clone(),
            span: decl.span.clone(),
            name_span: decl.name.span.clone(),
            params,
            check_dangling: true,
            injective: true,
            java_imports: Vec::new(),
            nodes: Vec::new(),
            inherited: 0,
            edges: Vec::new(),
            conditions: Vec::new(),
            condition: Condition::True,
            slots: 0,
            multi_rules: Vec::new(),
        };
        let signature = rule.signature(self.mm);
        self.symbol(&decl.name.span, signature);
        self.rule_body(&mut rule, &decl.items, &decl.span);
        for (i, p) in rule.params.iter().enumerate() {
            if !p.name.is_empty() && !self.param_used(&rule, i) && p.kind != ParamKind::Var {
                let span = p.span.clone();
                self.warning(&span, format!("parameter '{}' is not used by the rule", p.name));
            }
        }
        rule
    }

    fn param_used(&self, rule: &Rule, p: usize) -> bool {
        let uses = |e: &CExpr| {
            let mut out = Vec::new();
            e.params(&mut out);
            out.contains(&p)
        };
        let in_condition = |c: &Condition| condition_uses(c, &uses);
        rule.nodes.iter().any(|n| {
            n.param == Some(p) || n.matches.iter().any(|(_, e)| uses(e)) || n.sets.iter().any(|(_, e)| uses(e))
        }) || rule.conditions.iter().any(uses)
            || in_condition(&rule.condition)
            || rule.multi_rules.iter().any(|m| self.param_used(m, p))
    }

    /// Compiles the elements of a rule (or multi-rule) into `rule`, whose inherited nodes (and
    /// parameters) are set already.
    fn rule_body(&mut self, rule: &mut Rule, items: &[RuleItem], span: &Range<usize>) {
        let mut graph_items: Vec<&GraphItem> = Vec::new();
        let mut condition_exprs: Vec<&Expr> = Vec::new();
        for item in items {
            match item {
                RuleItem::JavaImport(package, _) => rule.java_imports.push(package.text()),
                RuleItem::CheckDangling(v, _) => rule.check_dangling = *v,
                RuleItem::InjectiveMatching(v, _) => rule.injective = *v,
                RuleItem::Conditions(c, _) => condition_exprs.extend(c),
                RuleItem::Graph(g) => graph_items.extend(&g.items),
            }
        }
        let mm = self.mm;
        let mut names: HashMap<String, NodeRef> =
            rule.nodes.iter().enumerate().map(|(i, n)| (n.name.clone(), NodeRef::Rule(i))).collect();
        let mut nac = ActionCondition::default();
        let mut pac = ActionCondition::default();
        // Nodes, then (once all names are known) their attributes, then edges.
        let mut node_decls: Vec<(NodeRef, &syntax::NodeDecl, Option<ClassifierId>)> = Vec::new();
        for item in &graph_items {
            let GraphItem::Node(n) = item else { continue };
            let class = self.class(&n.class);
            if n.name.text.is_empty() {
                continue;
            }
            if names.contains_key(&n.name.text) {
                self.error(&n.name.span, format!("there is already a node named '{}'", n.name.text));
                continue;
            }
            let Some(class) = class else { continue };
            let action = n.action.as_ref().map_or(Action::Preserve, |(a, _)| *a);
            let r = match action {
                Action::Forbid | Action::Require => {
                    let target = if action == Action::Forbid { &mut nac } else { &mut pac };
                    target.nodes.push(ConditionNode { name: n.name.text.clone(), class, matches: Vec::new() });
                    if action == Action::Forbid {
                        NodeRef::Nac(nac.nodes.len() - 1)
                    } else {
                        NodeRef::Pac(pac.nodes.len() - 1)
                    }
                }
                _ => {
                    let kind = match action {
                        Action::Create => NodeKind::Create,
                        Action::Delete => NodeKind::Delete,
                        _ => NodeKind::Preserve,
                    };
                    if kind == NodeKind::Create && mm.class_data(class).is_some_and(|d| d.is_abstract || d.is_interface)
                    {
                        self.error(&n.class.span, format!("'{}' is abstract: it cannot be created", n.class.text()));
                    }
                    rule.nodes.push(RuleNode {
                        name: n.name.text.clone(),
                        class,
                        kind,
                        matches: Vec::new(),
                        sets: Vec::new(),
                        unsets: Vec::new(),
                        param: None,
                        span: n.span.clone(),
                    });
                    NodeRef::Rule(rule.nodes.len() - 1)
                }
            };
            let text = format!("{} node {} : {}", action.keyword(), n.name.text, mm.qualified_name(class));
            self.symbol(&n.name.span, text);
            names.insert(n.name.text.clone(), r);
            node_decls.push((r, n, Some(class)));
        }
        for (r, decl, class) in &node_decls {
            let Some(class) = *class else { continue };
            let node_action = decl.action.as_ref().map_or(Action::Preserve, |(a, _)| *a);
            for attr in &decl.attributes {
                self.attribute(rule, &mut nac, &mut pac, *r, class, node_action, attr);
            }
        }
        for item in &graph_items {
            let GraphItem::Reuse(reuse) = item else { continue };
            match names.get(&reuse.name.text) {
                Some(NodeRef::Rule(i)) if *i < rule.inherited => {
                    let (class, action) = (rule.nodes[*i].class, rule.nodes[*i].kind);
                    let text = format!(
                        "{} node {} : {} (of the kernel rule)",
                        action.keyword(),
                        reuse.name.text,
                        mm.qualified_name(class)
                    );
                    self.symbol(&reuse.name.span, text);
                    let action = match action {
                        NodeKind::Preserve => Action::Preserve,
                        NodeKind::Delete => Action::Delete,
                        NodeKind::Create => Action::Create,
                    };
                    for attr in &reuse.attributes {
                        self.attribute(rule, &mut nac, &mut pac, NodeRef::Rule(*i), class, action, attr);
                    }
                }
                _ if rule.inherited == 0 => self.error(
                    &reuse.span,
                    "'reuse' refers to a node of the kernel rule: it is only allowed in multi-rules",
                ),
                _ => self.error(&reuse.name.span, format!("the kernel rule has no node named '{}'", reuse.name.text)),
            }
        }
        for item in &graph_items {
            let GraphItem::Edges(edges) = item else { continue };
            for edge in &edges.edges {
                self.edge(rule, &mut nac, &mut pac, &names, edge);
            }
        }
        // Node parameters: a parameter named like a node is that node.
        for (i, p) in rule.params.iter().enumerate() {
            let Some(n) = rule.nodes.iter().position(|n| n.name == p.name) else { continue };
            if n < rule.inherited {
                continue;
            }
            match &p.ty {
                ParamType::Class(c)
                    if mm.conforms_to(rule.nodes[n].class, *c) || mm.conforms_to(*c, rule.nodes[n].class) =>
                {
                    rule.nodes[n].param = Some(i);
                }
                ParamType::Unknown(_) => rule.nodes[n].param = Some(i),
                _ => {
                    let span = p.span.clone();
                    let class = mm.classifier(rule.nodes[n].class).name.clone();
                    self.error(
                        &span,
                        format!("parameter '{}' names a node of class {class}, so it must have that type", p.name),
                    );
                }
            }
        }
        for e in condition_exprs {
            let c = self.expr(e, &rule.params, None);
            rule.conditions.push(c);
        }
        // Application conditions.
        rule.slots = rule.nodes.len();
        let mut conditions = Vec::new();
        if !nac.is_empty() {
            conditions
                .push(Condition::Not(Box::new(Condition::Graph(Box::new(self.action_graph(rule, nac, "forbid"))))));
        }
        if !pac.is_empty() {
            conditions.push(Condition::Graph(Box::new(self.action_graph(rule, pac, "require"))));
        }
        let outer: HashMap<String, (usize, ClassifierId)> = rule
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.kind.is_lhs())
            .map(|(i, n)| (n.name.clone(), (i, n.class)))
            .collect();
        for item in &graph_items {
            if let GraphItem::Formula(f) = item {
                let c = self.formula(rule, &outer, f);
                conditions.push(c);
            }
        }
        rule.condition = Condition::and(conditions);
        let mut scope_nodes: Vec<(String, ClassifierId)> =
            rule.nodes.iter().map(|n| (n.name.clone(), n.class)).collect();
        for (name, r) in &names {
            if let NodeRef::Nac(_) | NodeRef::Pac(_) = r
                && let Some((_, _, Some(class))) = node_decls.iter().find(|(x, _, _)| x == r)
            {
                scope_nodes.push((name.clone(), *class));
            }
        }
        self.module.scopes.push(Scope {
            span: span.clone(),
            nodes: scope_nodes,
            params: rule.params.clone(),
            graphs: Vec::new(),
        });
        // Multi-rules, with copies of this rule's nodes.
        for item in &graph_items {
            let GraphItem::MultiRule(m) = item else { continue };
            let mut multi = Rule {
                name: m.name.text.clone(),
                span: m.span.clone(),
                name_span: m.name.span.clone(),
                params: rule.params.clone(),
                check_dangling: rule.check_dangling,
                injective: rule.injective,
                java_imports: rule.java_imports.clone(),
                nodes: rule
                    .nodes
                    .iter()
                    .map(|n| RuleNode {
                        matches: Vec::new(),
                        sets: Vec::new(),
                        unsets: Vec::new(),
                        param: None,
                        ..n.clone()
                    })
                    .collect(),
                inherited: rule.nodes.len(),
                edges: Vec::new(),
                conditions: Vec::new(),
                condition: Condition::True,
                slots: 0,
                multi_rules: Vec::new(),
            };
            let text = format!("multi-rule {} of rule {}", m.name.text, rule.name);
            self.symbol(&m.name.span, text);
            self.rule_body(&mut multi, &m.items, &m.span);
            rule.multi_rules.push(multi);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn attribute(
        &mut self,
        rule: &mut Rule,
        nac: &mut ActionCondition,
        pac: &mut ActionCondition,
        node: NodeRef,
        class: ClassifierId,
        node_action: Action,
        attr: &syntax::AttributeDecl,
    ) {
        let mm = self.mm;
        let name = attr.name.last();
        if name.is_empty() {
            return;
        }
        let Some(feature) = mm.lookup_feature(class, name) else {
            self.error(&attr.name.span, format!("class {} has no attribute '{name}'", mm.classifier(class).name));
            return;
        };
        if mm.feature(feature).is_reference() {
            self.error(&attr.name.span, format!("'{name}' is a reference: use an edge (in 'edges [...]')"));
            return;
        }
        self.symbol(&attr.name.span, feature_text(mm, feature));
        let value = self.expr(&attr.value, &rule.params, mm.feature(feature).ty);
        let action = attr.action.as_ref().map(|(a, _)| *a);
        let bad = |c: &mut Self, allowed: &str| {
            let span = attr.action.as_ref().map_or(attr.span.clone(), |(_, s)| s.clone());
            c.error(&span, format!("attributes of {} nodes can only be {allowed}", node_action.keyword()));
        };
        match node {
            NodeRef::Rule(i) => {
                let kind = rule.nodes[i].kind;
                match (kind, attr.set, action) {
                    (NodeKind::Create, _, None | Some(Action::Create)) | (NodeKind::Preserve, true, _) => {
                        rule.nodes[i].sets.push((feature, value))
                    }
                    (NodeKind::Preserve, false, None | Some(Action::Preserve)) => {
                        rule.nodes[i].matches.push((feature, value))
                    }
                    (NodeKind::Preserve, false, Some(Action::Create)) => rule.nodes[i].sets.push((feature, value)),
                    (NodeKind::Preserve, false, Some(Action::Delete)) => {
                        rule.nodes[i].matches.push((feature, value));
                        rule.nodes[i].unsets.push(feature);
                    }
                    (NodeKind::Preserve | NodeKind::Delete, false, Some(Action::Forbid)) => {
                        nac.checks.push((i, feature, value))
                    }
                    (NodeKind::Preserve | NodeKind::Delete, false, Some(Action::Require)) => {
                        pac.checks.push((i, feature, value))
                    }
                    (NodeKind::Delete, false, None | Some(Action::Delete | Action::Preserve)) => {
                        rule.nodes[i].matches.push((feature, value))
                    }
                    (NodeKind::Create, _, _) => bad(self, "created"),
                    (NodeKind::Delete, _, _) => bad(self, "matched ('set' and 'create' would be lost)"),
                }
            }
            NodeRef::Nac(i) | NodeRef::Pac(i) => {
                let own = if matches!(node, NodeRef::Nac(_)) { Action::Forbid } else { Action::Require };
                if attr.set || action.is_some_and(|a| a != own) {
                    bad(self, "matched");
                    return;
                }
                let target = if own == Action::Forbid { nac } else { pac };
                target.nodes[i].matches.push((feature, value));
            }
        }
    }

    fn edge(
        &mut self,
        rule: &mut Rule,
        nac: &mut ActionCondition,
        pac: &mut ActionCondition,
        names: &HashMap<String, NodeRef>,
        edge: &syntax::EdgeDecl,
    ) {
        let mm = self.mm;
        let endpoint = |c: &mut Self, name: &syntax::Name| -> Option<(NodeRef, ClassifierId)> {
            if name.text.is_empty() {
                return None;
            }
            let Some(r) = names.get(&name.text).copied() else {
                c.error(&name.span, format!("unknown node '{}'", name.text));
                return None;
            };
            let class = match r {
                NodeRef::Rule(i) => rule.nodes[i].class,
                NodeRef::Nac(i) => nac.nodes[i].class,
                NodeRef::Pac(i) => pac.nodes[i].class,
            };
            let what = match r {
                NodeRef::Rule(i) => rule.nodes[i].kind.keyword(),
                NodeRef::Nac(_) => "forbid",
                NodeRef::Pac(_) => "require",
            };
            c.symbol(&name.span, format!("{what} node {} : {}", name.text, mm.qualified_name(class)));
            Some((r, class))
        };
        let source = endpoint(self, &edge.source);
        let target = endpoint(self, &edge.target);
        let (Some((s, sc)), Some((t, tc))) = (source, target) else { return };
        let name = edge.ty.last();
        if name.is_empty() {
            return;
        }
        let Some(feature) = mm.lookup_feature(sc, name) else {
            self.error(&edge.ty.span, format!("class {} has no reference '{name}'", mm.classifier(sc).name));
            return;
        };
        let f = mm.feature(feature);
        if !f.is_reference() {
            self.error(&edge.ty.span, format!("'{name}' is an attribute: set it in the node's '{{ ... }}'"));
            return;
        }
        self.symbol(&edge.ty.span, feature_text(mm, feature));
        if let Some(ty) = f.ty
            && !mm.conforms_to(tc, ty)
            && !mm.conforms_to(ty, tc)
        {
            self.error(
                &edge.target.span,
                format!(
                    "'{}' is a {}, but '{name}' refers to {}",
                    edge.target.text,
                    mm.classifier(tc).name,
                    mm.classifier(ty).name
                ),
            );
            return;
        }
        let action = edge.action.as_ref().map_or(Action::Preserve, |(a, _)| *a);
        let kind_of = |r: NodeRef| match r {
            NodeRef::Rule(i) => Some(rule.nodes[i].kind),
            _ => None,
        };
        let span = edge.span.clone();
        match action {
            Action::Preserve | Action::Delete | Action::Create => {
                let (Some(sk), Some(tk)) = (kind_of(s), kind_of(t)) else {
                    self.error(&span, "an edge to a 'forbid' or 'require' node must have the same action");
                    return;
                };
                let kind = match action {
                    Action::Create => NodeKind::Create,
                    Action::Delete => NodeKind::Delete,
                    _ => NodeKind::Preserve,
                };
                if kind == NodeKind::Create && (sk == NodeKind::Delete || tk == NodeKind::Delete) {
                    self.error(&span, "an edge of a deleted node cannot be created");
                    return;
                }
                if kind != NodeKind::Create && (sk == NodeKind::Create || tk == NodeKind::Create) {
                    self.error(
                        &span,
                        format!("an edge of a created node must be created too ('create {}->...')", edge.source.text),
                    );
                    return;
                }
                if kind == NodeKind::Preserve && (sk == NodeKind::Delete || tk == NodeKind::Delete) {
                    self.error(&span, "an edge of a deleted node is deleted too: write 'delete'");
                    return;
                }
                if !f.changeable && kind != NodeKind::Preserve {
                    self.warning(&edge.ty.span, format!("reference '{name}' is not changeable"));
                }
                let (NodeRef::Rule(source), NodeRef::Rule(target)) = (s, t) else { return };
                rule.edges.push(RuleEdge { source, feature, target, kind, span });
            }
            Action::Forbid | Action::Require => {
                let forbid = action == Action::Forbid;
                let ok = |r: NodeRef| match r {
                    NodeRef::Rule(i) => rule.nodes[i].kind.is_lhs(),
                    NodeRef::Nac(_) => forbid,
                    NodeRef::Pac(_) => !forbid,
                };
                if !ok(s) || !ok(t) {
                    self.error(
                        &span,
                        format!(
                            "a '{}' edge connects matched nodes and '{}' nodes",
                            action.keyword(),
                            action.keyword()
                        ),
                    );
                    return;
                }
                let target = if forbid { nac } else { pac };
                target.edges.push((s, feature, t));
            }
        }
    }

    /// The condition graph of the `forbid` (or `require`) elements of a rule.
    fn action_graph(&mut self, rule: &mut Rule, condition: ActionCondition, name: &str) -> ConditionGraph {
        let base = rule.slots;
        rule.slots += condition.nodes.len();
        let slot = |r: NodeRef| match r {
            NodeRef::Rule(i) => i,
            NodeRef::Nac(i) | NodeRef::Pac(i) => base + i,
        };
        ConditionGraph {
            name: name.to_string(),
            nodes: condition
                .nodes
                .into_iter()
                .enumerate()
                .map(|(i, n)| PatternNode { name: n.name, slot: base + i, class: n.class, matches: n.matches })
                .collect(),
            checks: condition.checks,
            edges: condition.edges.iter().map(|(s, f, t)| (slot(*s), *f, slot(*t))).collect(),
            condition: Condition::True,
        }
    }

    /// A `matchingFormula`: its condition graphs combined by its formula.
    fn formula(
        &mut self,
        rule: &mut Rule,
        outer: &HashMap<String, (usize, ClassifierId)>,
        f: &syntax::FormulaDecl,
    ) -> Condition {
        let mut graphs: HashMap<String, ConditionGraph> = HashMap::new();
        let names: Vec<String> = f.graphs.iter().map(|g| g.name.text.clone()).collect();
        for g in &f.graphs {
            if graphs.contains_key(&g.name.text) {
                self.error(&g.name.span, format!("duplicate condition graph '{}'", g.name.text));
                continue;
            }
            let compiled = self.condition_graph(rule, outer, g);
            graphs.insert(g.name.text.clone(), compiled);
        }
        let mut used = Vec::new();
        let c = self.logic(&f.formula, &graphs, &mut used);
        for g in &f.graphs {
            if !used.contains(&g.name.text) && !g.name.text.is_empty() {
                self.warning(&g.name.span, format!("condition graph '{}' is not used in the formula", g.name.text));
            }
        }
        self.module.scopes.push(Scope {
            span: f.span.clone(),
            nodes: outer.iter().map(|(n, (_, c))| (n.clone(), *c)).collect(),
            params: rule.params.clone(),
            graphs: names,
        });
        c
    }

    fn logic(&mut self, l: &Logic, graphs: &HashMap<String, ConditionGraph>, used: &mut Vec<String>) -> Condition {
        match l {
            Logic::Ref(name) => match graphs.get(&name.text) {
                Some(g) => {
                    used.push(name.text.clone());
                    self.symbol(&name.span, format!("condition graph {}", name.text));
                    Condition::Graph(Box::new(g.clone()))
                }
                None => {
                    self.error(&name.span, format!("unknown condition graph '{}'", name.text));
                    Condition::True
                }
            },
            Logic::Not(inner) => Condition::Not(Box::new(self.logic(inner, graphs, used))),
            Logic::And(a, b) => Condition::and(vec![self.logic(a, graphs, used), self.logic(b, graphs, used)]),
            Logic::Or(a, b) => {
                Condition::Or(Box::new(self.logic(a, graphs, used)), Box::new(self.logic(b, graphs, used)))
            }
            Logic::Xor(a, b) => {
                Condition::Xor(Box::new(self.logic(a, graphs, used)), Box::new(self.logic(b, graphs, used)))
            }
            Logic::Error => Condition::True,
        }
    }

    fn condition_graph(
        &mut self,
        rule: &mut Rule,
        outer: &HashMap<String, (usize, ClassifierId)>,
        g: &syntax::ConditionGraphDecl,
    ) -> ConditionGraph {
        let mm = self.mm;
        let mut names = outer.clone();
        let mut graph = ConditionGraph {
            name: g.name.text.clone(),
            nodes: Vec::new(),
            checks: Vec::new(),
            edges: Vec::new(),
            condition: Condition::True,
        };
        self.symbol(&g.name.span, format!("condition graph {}", g.name.text));
        for item in &g.items {
            let GraphItem::Node(n) = item else { continue };
            if let Some((_, span)) = &n.action {
                self.error(span, "nodes of condition graphs have no action");
            }
            let Some(class) = self.class(&n.class) else { continue };
            if names.contains_key(&n.name.text) {
                self.error(&n.name.span, format!("there is already a node named '{}'", n.name.text));
                continue;
            }
            let slot = rule.slots;
            rule.slots += 1;
            names.insert(n.name.text.clone(), (slot, class));
            self.symbol(
                &n.name.span,
                format!("node {} : {} (condition graph {})", n.name.text, mm.qualified_name(class), g.name.text),
            );
            let mut matches = Vec::new();
            for attr in &n.attributes {
                if let Some(m) = self.condition_attribute(rule, class, attr) {
                    matches.push(m);
                }
            }
            graph.nodes.push(PatternNode { name: n.name.text.clone(), slot, class, matches });
        }
        for item in &g.items {
            match item {
                GraphItem::Node(_) => {}
                GraphItem::Reuse(r) => match names.get(&r.name.text).copied() {
                    Some((slot, class)) => {
                        self.symbol(&r.name.span, format!("node {} : {}", r.name.text, mm.qualified_name(class)));
                        for attr in &r.attributes {
                            if let Some((f, e)) = self.condition_attribute(rule, class, attr) {
                                graph.checks.push((slot, f, e));
                            }
                        }
                    }
                    None => self.error(&r.name.span, format!("unknown node '{}'", r.name.text)),
                },
                GraphItem::Edges(edges) => {
                    for e in &edges.edges {
                        if let Some((_, span)) = &e.action {
                            self.error(span, "edges of condition graphs have no action");
                        }
                        let s = names.get(&e.source.text).copied();
                        let t = names.get(&e.target.text).copied();
                        for (name, found) in [(&e.source, s), (&e.target, t)] {
                            match found {
                                Some((_, class)) => self
                                    .symbol(&name.span, format!("node {} : {}", name.text, mm.qualified_name(class))),
                                None if name.text.is_empty() => {}
                                None => self.error(&name.span, format!("unknown node '{}'", name.text)),
                            }
                        }
                        let (Some((s, sc)), Some((t, _))) = (s, t) else { continue };
                        match mm.lookup_feature(sc, e.ty.last()).filter(|f| mm.feature(*f).is_reference()) {
                            Some(f) => {
                                self.symbol(&e.ty.span, feature_text(mm, f));
                                graph.edges.push((s, f, t));
                            }
                            None if e.ty.last().is_empty() => {}
                            None => self.error(
                                &e.ty.span,
                                format!("class {} has no reference '{}'", mm.classifier(sc).name, e.ty.last()),
                            ),
                        }
                    }
                }
                GraphItem::Formula(f) => {
                    let c = self.formula(rule, &names, f);
                    graph.condition = Condition::and(vec![std::mem::replace(&mut graph.condition, Condition::True), c]);
                }
                GraphItem::MultiRule(m) => self.error(&m.span, "multi-rules belong in the graph of a rule"),
            }
        }
        self.module.scopes.push(Scope {
            span: g.span.clone(),
            nodes: names.iter().map(|(n, (_, c))| (n.clone(), *c)).collect(),
            params: rule.params.clone(),
            graphs: Vec::new(),
        });
        graph
    }

    fn condition_attribute(
        &mut self,
        rule: &Rule,
        class: ClassifierId,
        attr: &syntax::AttributeDecl,
    ) -> Option<(FeatureId, CExpr)> {
        let mm = self.mm;
        if attr.set || attr.action.is_some() {
            self.error(&attr.span, "attributes of condition graphs can only be matched");
        }
        let name = attr.name.last();
        let feature = mm.lookup_feature(class, name).filter(|f| !mm.feature(*f).is_reference());
        let Some(feature) = feature else {
            if !name.is_empty() {
                self.error(&attr.name.span, format!("class {} has no attribute '{name}'", mm.classifier(class).name));
            }
            return None;
        };
        self.symbol(&attr.name.span, feature_text(mm, feature));
        Some((feature, self.expr(&attr.value, &rule.params, mm.feature(feature).ty)))
    }

    // ------------------------------------------------------------------ units

    fn unit(&mut self, decl: &syntax::UnitDecl) -> Unit {
        let params = self.params(&decl.params);
        let mut unit = Unit {
            name: decl.name.text.clone(),
            span: decl.span.clone(),
            name_span: decl.name.span.clone(),
            params,
            body: Step::Sequence { steps: Vec::new(), strict: true, rollback: true },
        };
        let signature = unit.signature(self.mm);
        self.symbol(&decl.name.span, signature);
        if decl.items.is_empty() {
            self.warning(&decl.name.span, format!("unit '{}' does nothing", decl.name.text));
        }
        unit.body = self.sequence(&unit.params, &decl.items);
        self.module.scopes.push(Scope {
            span: decl.span.clone(),
            nodes: Vec::new(),
            params: unit.params.clone(),
            graphs: Vec::new(),
        });
        unit
    }

    fn sequence(&mut self, params: &[Param], items: &[UnitItem]) -> Step {
        let mut steps = Vec::new();
        let mut strict = true;
        let mut rollback = true;
        for item in items {
            match item {
                UnitItem::Strict(v, _) => strict = *v,
                UnitItem::Rollback(v, _) => rollback = *v,
                other => {
                    if let Some(s) = self.step(params, other) {
                        steps.push(s);
                    }
                }
            }
        }
        Step::Sequence { steps, strict, rollback }
    }

    fn step(&mut self, params: &[Param], item: &UnitItem) -> Option<Step> {
        Some(match item {
            UnitItem::Call { target, args, span } => {
                if target.text.is_empty() {
                    return None;
                }
                let Some((callable, arity)) = self.callables.get(&target.text).copied() else {
                    self.error(&target.span, format!("unknown rule or unit '{}'", target.text));
                    return None;
                };
                let signature = match callable {
                    // Rules and units declared later are not compiled yet; describe by name.
                    Callable::Rule(i) => self.module.rules.get(i).map(|r| r.signature(self.mm)),
                    Callable::Unit(i) => self.module.units.get(i).map(|u| u.signature(self.mm)),
                }
                .unwrap_or_else(|| {
                    format!("{} {}", if matches!(callable, Callable::Rule(_)) { "rule" } else { "unit" }, target.text)
                });
                self.symbol(&target.span, signature);
                if args.len() != arity {
                    self.error(
                        span,
                        format!(
                            "'{}' has {arity} parameter{}, not {}",
                            target.text,
                            if arity == 1 { "" } else { "s" },
                            args.len()
                        ),
                    );
                    return None;
                }
                let mut indices = Vec::new();
                for a in args {
                    match params.iter().position(|p| p.name == a.text) {
                        Some(i) => {
                            let text = format!("parameter {}", params[i].signature(self.mm));
                            self.symbol(&a.span, text);
                            indices.push(i);
                        }
                        None => {
                            if !a.text.is_empty() {
                                self.error(&a.span, format!("'{}' is not a parameter of the unit", a.text));
                            }
                            return None;
                        }
                    }
                }
                Step::Call { target: callable, args: indices, span: span.clone() }
            }
            UnitItem::Sequence(items, _) => self.sequence(params, items),
            UnitItem::Strict(..) | UnitItem::Rollback(..) => return None,
            UnitItem::Independent(lists, _) => {
                Step::Independent(lists.iter().map(|l| self.sequence(params, l)).collect())
            }
            UnitItem::Priority(lists, _) => Step::Priority(lists.iter().map(|l| self.sequence(params, l)).collect()),
            UnitItem::If { condition, then, otherwise, .. } => Step::If {
                condition: Box::new(self.sequence(params, condition)),
                then: Box::new(self.sequence(params, then)),
                otherwise: otherwise.as_ref().map(|o| Box::new(self.sequence(params, o))),
            },
            UnitItem::For { count, body, .. } => {
                Step::For { count: self.expr(count, params, None), body: Box::new(self.sequence(params, body)) }
            }
            UnitItem::While { body, .. } => Step::While(Box::new(self.sequence(params, body))),
        })
    }
}

fn condition_uses(c: &Condition, uses: &impl Fn(&CExpr) -> bool) -> bool {
    match c {
        Condition::True => false,
        Condition::Graph(g) => {
            g.nodes.iter().any(|n| n.matches.iter().any(|(_, e)| uses(e)))
                || g.checks.iter().any(|(_, _, e)| uses(e))
                || condition_uses(&g.condition, uses)
        }
        Condition::Not(c) => condition_uses(c, uses),
        Condition::And(cs) => cs.iter().any(|c| condition_uses(c, uses)),
        Condition::Or(a, b) | Condition::Xor(a, b) => condition_uses(a, uses) || condition_uses(b, uses),
    }
}
