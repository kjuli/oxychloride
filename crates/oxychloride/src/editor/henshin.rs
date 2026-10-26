//! Completion and hovers for Henshin documents (`.henshin_text`, and hovers for `.henshin`
//! files).
//!
//! Hovers describe the names the compiler resolved (packages, classes, nodes, features,
//! parameters, rules and units); in `.henshin` files at the XML that names them. Completion scans the tokens before the cursor for the block it
//! is in — a rule, a graph, a node's attributes, an edge, a formula, a unit — and proposes what
//! fits there: keywords, the classes of the imported packages, the nodes of the enclosing rule,
//! the references of an edge's source class, attributes, parameters and enumeration literals,
//! rules and units to call.

use oxy_ecore::{ClassifierId, Metamodel};
use oxy_henshin::Module;
use oxy_henshin::module::{feature_text, type_text};
use oxy_henshin::syntax::{self, Tok, Token};

use super::item;
use crate::protocol::*;
use crate::workspace::Workspace;

const TOP_KEYWORDS: &[&str] = &["ePackageImport", "rule", "unit"];
const RULE_KEYWORDS: &[&str] = &["graph", "conditions", "checkDangling", "injectiveMatching", "javaImport"];
const ACTIONS: &[&str] = &["preserve", "create", "delete", "forbid", "require"];
const UNIT_KEYWORDS: &[&str] = &["independent", "priority", "if", "for", "while", "strict", "rollback"];
const PARAM_KINDS: &[&str] = &["IN", "OUT", "INOUT", "VAR"];
/// The Ecore data types of the grammar's parameter types.
const DATA_TYPES: &[&str] = &[
    "EString",
    "EInt",
    "EBoolean",
    "EDouble",
    "EFloat",
    "ELong",
    "EShort",
    "EByte",
    "EChar",
    "EBigInteger",
    "EBigDecimal",
    "EDate",
    "EIntegerObject",
    "EDoubleObject",
    "EBooleanObject",
    "EJavaObject",
];

#[derive(Clone, Debug, PartialEq)]
enum Frame {
    Rule,
    Unit,
    Graph,
    ConditionGraph,
    Formula,
    /// The attributes of a node of this class (by name), or of a reused node.
    Attributes(Attributes),
    Params,
    Edges,
    /// An edge: the tokens after its `(`.
    Edge(usize),
    Conditions,
    UnitList,
    /// The call arguments of a unit step.
    Arguments,
    Other,
}

#[derive(Clone, Debug, PartialEq)]
enum Attributes {
    Class(Vec<String>),
    Node(String),
}

fn is(t: Option<&Token>, word: &str) -> bool {
    t.is_some_and(|t| t.kind.is_ident(word))
}

fn ident(t: Option<&Token>) -> Option<&str> {
    match t.map(|t| &t.kind) {
        Some(Tok::Ident(w)) => Some(w.as_str()),
        _ => None,
    }
}

/// The blocks open at the end of `tokens`, innermost last, each with the index of its opening
/// token.
fn frames(tokens: &[Token]) -> Vec<(Frame, usize)> {
    let mut stack: Vec<(Frame, usize)> = Vec::new();
    let at = |i: usize, back: usize| i.checked_sub(back).and_then(|j| tokens.get(j));
    for (i, t) in tokens.iter().enumerate() {
        let top = stack.last().map(|(f, _)| f.clone());
        match t.kind {
            Tok::LBrace => {
                let frame = match &at(i, 1).map(|t| &t.kind) {
                    Some(Tok::Ident(w)) if w == "graph" => Frame::Graph,
                    Some(Tok::Ident(w)) if w == "matchingFormula" => Frame::Formula,
                    Some(Tok::Ident(w)) if w == "then" || w == "else" || w == "while" => Frame::Unit,
                    Some(Tok::Ident(_)) if is(at(i, 2), "multiRule") => Frame::Rule,
                    Some(Tok::Ident(_)) if is(at(i, 2), "conditionGraph") => Frame::ConditionGraph,
                    Some(Tok::Ident(w)) if is(at(i, 2), "reuse") => Frame::Attributes(Attributes::Node(w.clone())),
                    Some(Tok::RParen) => {
                        // `rule name(...) {`, `unit name(...) {`, `for (n) {`.
                        let mut depth = 0;
                        let mut open = None;
                        for j in (0..i).rev() {
                            match tokens[j].kind {
                                Tok::RParen => depth += 1,
                                Tok::LParen => {
                                    depth -= 1;
                                    if depth == 0 {
                                        open = Some(j);
                                        break;
                                    }
                                }
                                _ => {}
                            }
                        }
                        match open {
                            Some(j) if is(at(j, 2), "rule") => Frame::Rule,
                            Some(j) if is(at(j, 2), "unit") || is(at(j, 1), "for") => Frame::Unit,
                            _ => Frame::Other,
                        }
                    }
                    Some(Tok::Ident(_)) => {
                        // `node name : pkg.Class {`.
                        let mut j = i - 1;
                        let mut parts = vec![ident(tokens.get(j)).unwrap_or("").to_string()];
                        while j >= 2 && tokens[j - 1].kind == Tok::Dot && ident(tokens.get(j - 2)).is_some() {
                            j -= 2;
                            parts.insert(0, ident(tokens.get(j)).unwrap_or("").to_string());
                        }
                        if j >= 1 && tokens[j - 1].kind == Tok::Colon {
                            Frame::Attributes(Attributes::Class(parts))
                        } else if matches!(top, Some(Frame::Unit | Frame::UnitList)) {
                            Frame::Unit
                        } else {
                            Frame::Other
                        }
                    }
                    _ if matches!(top, Some(Frame::Unit | Frame::UnitList)) => Frame::Unit,
                    _ => Frame::Other,
                };
                stack.push((frame, i));
            }
            Tok::LBracket => {
                let frame = match ident(at(i, 1)) {
                    Some("edges") => Frame::Edges,
                    Some("conditions") => Frame::Conditions,
                    Some("independent" | "priority") => Frame::UnitList,
                    _ => Frame::Other,
                };
                stack.push((frame, i));
            }
            Tok::LParen => {
                let frame = if top == Some(Frame::Edges) {
                    Frame::Edge(i)
                } else if ident(at(i, 1)).is_some() && (is(at(i, 2), "rule") || is(at(i, 2), "unit")) {
                    Frame::Params
                } else if is(at(i, 1), "if") {
                    Frame::UnitList
                } else if is(at(i, 1), "for") {
                    Frame::Conditions
                } else if matches!(top, Some(Frame::Unit | Frame::UnitList)) && ident(at(i, 1)).is_some() {
                    Frame::Arguments
                } else if top == Some(Frame::Conditions) {
                    Frame::Conditions
                } else if top == Some(Frame::Formula) {
                    Frame::Formula
                } else {
                    Frame::Other
                };
                stack.push((frame, i));
            }
            Tok::RBrace | Tok::RBracket | Tok::RParen => {
                stack.pop();
            }
            _ => {}
        }
    }
    stack
}

impl Workspace {
    pub(super) fn hover_henshin(&self, name: &str, text: &str, language: Language, offset: usize) -> Option<HoverInfo> {
        let (module, _) = self.compile_transformation(name, text, language);
        let (span, contents) = module.symbol_at(offset)?;
        let range = self.text_range(text, span);
        Some(HoverInfo { range, contents: contents.to_string() })
    }

    pub(super) fn text_range(&self, text: &str, span: &std::ops::Range<usize>) -> Range {
        let index = oxy_ocl::LineIndex::new(text);
        let p = |o: usize| {
            let p = index.position(text, o);
            Position { line: p.line, column: p.column }
        };
        Range { start: p(span.start), end: p(span.end) }
    }

    pub(super) fn complete_henshin(&self, text: &str, word_start: usize) -> Vec<CompletionItem> {
        let mm = self.metamodel();
        let module = Module::compile(mm, text);
        let (mut tokens, _) = syntax::tokenize(&text[..word_start]);
        tokens.pop(); // Eof
        let stack = frames(&tokens);
        let last = tokens.last();
        let before = tokens.len().checked_sub(2).and_then(|i| tokens.get(i));
        let scope = module.scope_at(word_start);
        let mut items = Vec::new();
        let keywords = |items: &mut Vec<CompletionItem>, words: &[&str]| {
            items.extend(words.iter().map(|w| item(w, CompletionKind::Keyword, None)));
        };
        let nodes = |items: &mut Vec<CompletionItem>| {
            for (name, class) in scope.map(|s| s.nodes.as_slice()).unwrap_or(&[]) {
                items.push(item(name, CompletionKind::Variable, Some(mm.classifier(*class).name.clone())));
            }
        };
        let params = |items: &mut Vec<CompletionItem>| {
            for p in scope.map(|s| s.params.as_slice()).unwrap_or(&[]) {
                items.push(item(&p.name, CompletionKind::Variable, Some(p.type_name(mm))));
            }
        };
        // Right after keywords that take a Boolean.
        if is(last, "checkDangling") || is(last, "injectiveMatching") || is(last, "strict") || is(last, "rollback") {
            keywords(&mut items, &["true", "false"]);
            return items;
        }
        let frame = stack.last().map(|(f, _)| f.clone());
        match frame {
            None => {
                if is(last, "ePackageImport") {
                    for p in mm.user_root_packages() {
                        let pkg = mm.package(p);
                        items.push(item(&pkg.name, CompletionKind::Type, Some(pkg.ns_uri.clone())));
                    }
                } else if !is(last, "rule") && !is(last, "unit") {
                    keywords(&mut items, TOP_KEYWORDS);
                }
            }
            Some(Frame::Params) => {
                if last.is_some_and(|t| t.kind == Tok::Colon) {
                    self.henshin_class_items(&module, &mut items);
                    for t in DATA_TYPES {
                        items.push(item(t, CompletionKind::Type, Some("Ecore data type".into())));
                    }
                } else if last.is_some_and(|t| matches!(t.kind, Tok::LParen | Tok::Comma)) {
                    keywords(&mut items, PARAM_KINDS);
                }
            }
            Some(Frame::Rule) => keywords(&mut items, RULE_KEYWORDS),
            Some(Frame::Graph | Frame::ConditionGraph) => {
                let condition = frame == Some(Frame::ConditionGraph);
                if last.is_some_and(|t| t.kind == Tok::Colon) && ident(before).is_some() {
                    self.henshin_class_items(&module, &mut items);
                } else if last.is_some_and(|t| t.kind == Tok::Dot) {
                    // `pkg.` of a qualified class name.
                    self.henshin_class_items(&module, &mut items);
                } else if ident(last).is_some_and(|w| ACTIONS.contains(&w)) {
                    keywords(&mut items, &["node"]);
                } else if is(last, "reuse") {
                    nodes(&mut items);
                } else if is(last, "node") || is(last, "multiRule") || is(last, "conditionGraph") {
                } else {
                    keywords(&mut items, &["node", "edges", "reuse", "matchingFormula"]);
                    if !condition {
                        keywords(&mut items, ACTIONS);
                        keywords(&mut items, &["multiRule"]);
                    }
                }
            }
            Some(Frame::Formula) => {
                let graphs = scope.map(|s| s.graphs.as_slice()).unwrap_or(&[]);
                let expects_graph = is(last, "formula")
                    || last.is_some_and(|t| matches!(t.kind, Tok::Bang | Tok::LParen))
                    || ["AND", "OR", "XOR"].iter().any(|w| is(last, w));
                if expects_graph {
                    for g in graphs {
                        items.push(item(g, CompletionKind::Variable, Some("condition graph".into())));
                    }
                } else if ident(last).is_some_and(|w| graphs.iter().any(|g| g == w))
                    || last.is_some_and(|t| t.kind == Tok::RParen)
                {
                    keywords(&mut items, &["AND", "OR", "XOR", "conditionGraph"]);
                } else if !is(last, "conditionGraph") {
                    keywords(&mut items, &["formula", "conditionGraph"]);
                }
            }
            Some(Frame::Attributes(owner)) => {
                let class = match &owner {
                    Attributes::Class(path) => {
                        let path: Vec<&str> = path.iter().map(String::as_str).collect();
                        mm.find_classifiers_qualified(&path).into_iter().find(|c| mm.is_class(*c))
                    }
                    Attributes::Node(name) => {
                        scope.and_then(|s| s.nodes.iter().find(|(n, _)| n == name)).map(|(_, c)| *c)
                    }
                };
                if last.is_some_and(|t| t.kind == Tok::Assign) {
                    let feature = ident(before).zip(class).and_then(|(name, c)| mm.lookup_feature(c, name));
                    if let Some(ty) = feature.and_then(|f| mm.feature(f).ty) {
                        for l in mm.enum_literals(ty) {
                            items.push(item(
                                &l.name,
                                CompletionKind::EnumLiteral,
                                Some(mm.classifier(ty).name.clone()),
                            ));
                        }
                        if mm.data_type_kind(ty) == Some(oxy_ecore::DataTypeKind::Boolean) {
                            keywords(&mut items, &["true", "false"]);
                        }
                    }
                    params(&mut items);
                } else if let Some(class) = class {
                    let after_name = ident(last).is_some_and(|w| !syntax::is_keyword(w))
                        && before.is_some_and(|t| !matches!(t.kind, Tok::Assign));
                    if !after_name || ident(last).is_some_and(|w| w == "set" || ACTIONS.contains(&w)) {
                        for &f in mm.all_features(class) {
                            let feature = mm.feature(f);
                            if !feature.is_reference() && !feature.derived {
                                items.push(item(
                                    &feature.name,
                                    CompletionKind::Property,
                                    Some(type_text(mm, feature.ty, &feature.multiplicity)),
                                ));
                            }
                        }
                        if !ident(last).is_some_and(|w| w == "set" || ACTIONS.contains(&w)) {
                            keywords(&mut items, &["set"]);
                            keywords(&mut items, ACTIONS);
                        }
                    }
                }
            }
            Some(Frame::Edges) => {}
            Some(Frame::Edge(open)) => {
                let inside = &tokens[open + 1..];
                let arrow = inside.iter().position(|t| t.kind == Tok::Arrow);
                let colon = inside.iter().position(|t| t.kind == Tok::Colon);
                match (arrow, colon) {
                    (None, _) => {
                        if inside.is_empty() {
                            keywords(&mut items, ACTIONS);
                        }
                        if inside.len() <= 1 {
                            nodes(&mut items);
                        }
                    }
                    (Some(a), None) if a + 1 == inside.len() => nodes(&mut items),
                    (Some(a), Some(c)) if c + 1 == inside.len() => {
                        let find = |name: Option<&str>| {
                            scope.and_then(|s| s.nodes.iter().find(|(n, _)| Some(n.as_str()) == name)).map(|(_, c)| *c)
                        };
                        let source = find(ident(a.checked_sub(1).and_then(|i| inside.get(i))));
                        let target = find(ident(inside.get(a + 1)));
                        if let Some(source) = source {
                            self.henshin_reference_items(source, target, &mut items);
                        }
                    }
                    _ => {}
                }
            }
            Some(Frame::Conditions) => {
                params(&mut items);
                keywords(&mut items, &["true", "false"]);
            }
            Some(Frame::Unit | Frame::UnitList) => {
                if is(last, "if") || is(last, "for") {
                } else {
                    for r in &module.rules {
                        items.push(item(&r.name, CompletionKind::Operation, Some(r.signature(mm))));
                    }
                    for u in &module.units {
                        items.push(item(&u.name, CompletionKind::Operation, Some(u.signature(mm))));
                    }
                    keywords(&mut items, UNIT_KEYWORDS);
                    if last.is_some_and(|t| t.kind == Tok::RParen) {
                        keywords(&mut items, &["then"]);
                    }
                    if last.is_some_and(|t| t.kind == Tok::RBrace) {
                        keywords(&mut items, &["else"]);
                    }
                }
            }
            Some(Frame::Arguments) => params(&mut items),
            Some(Frame::Other) => {}
        }
        items
    }

    /// The classes of the imported packages (all classes if none is imported).
    fn henshin_class_items(&self, module: &Module, items: &mut Vec<CompletionItem>) {
        let mm = self.metamodel();
        let packages: Vec<oxy_ecore::PackageId> =
            if module.packages.is_empty() { mm.user_root_packages().collect() } else { module.packages.clone() };
        for p in packages {
            collect_classes(mm, p, items);
        }
    }

    /// The references of `source`, those that can point to `target` first.
    fn henshin_reference_items(
        &self,
        source: ClassifierId,
        target: Option<ClassifierId>,
        items: &mut Vec<CompletionItem>,
    ) {
        let mm = self.metamodel();
        let mut refs: Vec<(bool, CompletionItem)> = mm
            .all_features(source)
            .iter()
            .filter(|f| mm.feature(**f).is_reference())
            .map(|f| {
                let feature = mm.feature(*f);
                let fits = match (target, feature.ty) {
                    (Some(t), Some(ty)) => mm.conforms_to(t, ty),
                    _ => true,
                };
                (fits, item(&feature.name, CompletionKind::Property, Some(feature_text(mm, *f))))
            })
            .collect();
        refs.sort_by_key(|(fits, _)| !*fits);
        items.extend(refs.into_iter().map(|(_, i)| i));
    }
}

fn collect_classes(mm: &Metamodel, p: oxy_ecore::PackageId, items: &mut Vec<CompletionItem>) {
    for c in &mm.package(p).classifiers {
        if mm.is_class(*c) {
            items.push(item(&mm.classifier(*c).name, CompletionKind::Type, Some(mm.package_qualified_name(p))));
        }
    }
    for s in &mm.package(p).subpackages {
        collect_classes(mm, *s, items);
    }
}
