//! Diagram edits of Henshin transformations, made on the syntax tree of the textual syntax: new
//! elements are written the way [`oxy_henshin::print`] writes them, indented like their
//! neighbours; changed elements get a new name, class, action or value in place; deleted ones
//! are removed with their lines (and the edges and `reuse`s of deleted nodes, the calls of
//! deleted rules and units with them). A `.henshin` file is edited through its textual form
//! (see `henshin.rs`).

use std::collections::{HashMap, HashSet};
use std::ops::Range as ByteRange;

use oxy_ecore::Metamodel;
use oxy_henshin::Module;
use oxy_henshin::print::{self, Printer};
use oxy_henshin::syntax::{self, Element, *};
use oxy_henshin::xmi::{self, WriteOptions};

use super::henshin::*;
use super::*;
use crate::textdiff::line_replacements;

/// A transformation text with its syntax tree, which edits are computed for.
pub(crate) struct Doc<'a> {
    text: &'a str,
    imports: &'a [QName],
    ix: Index<'a>,
    printer: Printer,
}

/// The text of a unit of indentation, one level deeper than the line at `offset`.
fn deeper(text: &str, offset: usize) -> String {
    format!("{}{}", indentation(text, offset), indent_unit_or_tab(text))
}

/// The document's unit of indentation (a tab if it does not tell).
fn indent_unit_or_tab(text: &str) -> String {
    if text.lines().any(|l| l.starts_with([' ', '\t'])) { indent_unit(text) } else { "\t".into() }
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

fn check_name(name: &str) -> Result<(), Error> {
    if syntax::is_name(name) { Ok(()) } else { Err(invalid(format!("'{name}' is not a name"))) }
}

fn parse_value(text: &str) -> Result<Expr, Error> {
    let (e, problems) = syntax::parse_expr(text);
    match problems.first() {
        Some(p) => Err(invalid(format!("'{text}' is not an expression: {}", p.message))),
        None => Ok(e),
    }
}

fn parse_qname(text: &str) -> Result<QName, Error> {
    let parts: Vec<&str> = text.split('.').collect();
    if parts.iter().any(|p| !syntax::is_name(p)) {
        return Err(invalid(format!("'{text}' is not a name")));
    }
    Ok(QName { parts: parts.iter().map(|p| Name { text: p.to_string(), span: 0..0 }).collect(), span: 0..0 })
}

fn name(text: &str) -> Name {
    Name { text: text.to_string(), span: 0..0 }
}

fn param_kind(kind: &str) -> Result<ParamKind, Error> {
    Ok(match kind {
        "in" => ParamKind::In,
        "out" => ParamKind::Out,
        "inout" => ParamKind::InOut,
        "var" => ParamKind::Var,
        "unknown" | "" => ParamKind::Unknown,
        other => return Err(invalid(format!("'{other}' is not a kind of parameter"))),
    })
}

/// The last element of a list by its span.
fn last<T>(items: &[T], span: impl Fn(&T) -> ByteRange<usize>) -> Option<ByteRange<usize>> {
    items.last().map(span)
}

/// A condition graph's formula without the graph `name` (`None` if nothing is left).
fn without(l: &Logic, name: &str) -> Option<Logic> {
    match l {
        Logic::Ref(n) if n.text == name => None,
        Logic::Ref(_) | Logic::Error => Some(l.clone()),
        Logic::Not(inner) => without(inner, name).map(|i| Logic::Not(Box::new(i))),
        Logic::And(a, b) | Logic::Or(a, b) | Logic::Xor(a, b) => match (without(a, name), without(b, name)) {
            (Some(a), Some(b)) => Some(match l {
                Logic::And(..) => Logic::And(Box::new(a), Box::new(b)),
                Logic::Or(..) => Logic::Or(Box::new(a), Box::new(b)),
                _ => Logic::Xor(Box::new(a), Box::new(b)),
            }),
            (x, None) | (None, x) => x,
        },
    }
}

/// The names a formula refers to, with their spans.
fn logic_refs<'l>(l: &'l Logic, out: &mut Vec<&'l Name>) {
    match l {
        Logic::Ref(n) => out.push(n),
        Logic::Not(inner) => logic_refs(inner, out),
        Logic::And(a, b) | Logic::Or(a, b) | Logic::Xor(a, b) => {
            logic_refs(a, out);
            logic_refs(b, out);
        }
        Logic::Error => {}
    }
}

impl<'a> Doc<'a> {
    pub(crate) fn new(text: &'a str, doc: &'a Document) -> Self {
        Doc {
            text,
            imports: &doc.imports,
            ix: Index::new(doc),
            printer: Printer::with_indent(indent_unit_or_tab(text)),
        }
    }

    /// Lines of printed text, indented for the line at `at`.
    fn indented(&self, block: &str, indent: &str) -> String {
        block
            .lines()
            .map(|l| if l.is_empty() { String::new() } else { format!("{indent}{l}") })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn removal(&self, span: ByteRange<usize>) -> ByteEdit {
        ByteEdit::delete(removal(self.text, span, Some("//")))
    }

    fn scope(&self, id: &str) -> Result<&ScopeEntry<'a>, Error> {
        self.ix.scopes.get(id).ok_or_else(|| Error::NotFound(format!("no rule, multi-rule or condition graph '{id}'")))
    }

    fn node(&self, id: &str) -> Result<&NodeEntry<'a>, Error> {
        self.ix.nodes.get(id).ok_or_else(|| Error::NotFound(format!("no node '{id}'")))
    }

    /// The names of the nodes of a rule (in any of its scopes).
    fn rule_node_names(&self, rule: &str) -> HashSet<&str> {
        self.ix
            .nodes
            .values()
            .filter(|n| self.ix.scopes.get(&n.scope).is_some_and(|s| s.rule == rule))
            .map(|n| n.decl.name.text.as_str())
            .collect()
    }

    fn callable_taken(&self, name: &str) -> bool {
        self.ix.rules.values().any(|r| r.name.text == name) || self.ix.units.values().any(|u| u.name.text == name)
    }

    pub(crate) fn edit(&self, edit: &TransformationEdit) -> Result<(Vec<ByteEdit>, Option<String>), Error> {
        let mut edits = Vec::new();
        let element = match edit {
            TransformationEdit::SetImports { imports } => {
                edits.extend(self.set_imports(imports)?);
                None
            }
            TransformationEdit::AddRule { name } => {
                check_name(name)?;
                if self.callable_taken(name) {
                    return Err(invalid(format!("there is already a rule or unit named '{name}'")));
                }
                let rule = Element::Rule(RuleDecl {
                    name: self::name(name),
                    params: Vec::new(),
                    params_span: 0..0,
                    items: vec![RuleItem::Graph(GraphDecl { items: Vec::new(), body: 0..0, span: 0..0 })],
                    body: 0..0,
                    span: 0..0,
                });
                edits.push(self.append_element(&rule));
                Some(name.clone())
            }
            TransformationEdit::AddUnit { name } => {
                check_name(name)?;
                if self.callable_taken(name) {
                    return Err(invalid(format!("there is already a rule or unit named '{name}'")));
                }
                let unit = Element::Unit(UnitDecl {
                    name: self::name(name),
                    params: Vec::new(),
                    params_span: 0..0,
                    items: Vec::new(),
                    body: 0..0,
                    span: 0..0,
                });
                edits.push(self.append_element(&unit));
                Some(name.clone())
            }
            TransformationEdit::Rename { element, name } => Some(self.rename(element, name, &mut edits)?),
            TransformationEdit::Delete { elements } => {
                self.delete(elements, &mut edits)?;
                None
            }
            TransformationEdit::SetParameters { element, parameters } => {
                let mut params = Vec::new();
                let mut seen = HashSet::new();
                for p in parameters {
                    check_name(&p.name)?;
                    if !seen.insert(p.name.as_str()) {
                        return Err(invalid(format!("duplicate parameter '{}'", p.name)));
                    }
                    let ty = match p.ty.as_deref().filter(|t| !t.is_empty()) {
                        Some(t) => parse_qname(t)?,
                        None => QName { parts: Vec::new(), span: 0..0 },
                    };
                    params.push(ParamDecl { kind: param_kind(&p.kind)?, name: self::name(&p.name), ty, span: 0..0 });
                }
                let (span, after_name) = match (self.ix.rules.get(element), self.ix.units.get(element)) {
                    (Some(r), _) => (r.params_span.clone(), r.name.span.end),
                    (_, Some(u)) => (u.params_span.clone(), u.name.span.end),
                    _ => return Err(Error::NotFound(format!("no rule or unit '{element}'"))),
                };
                let text = print::params(&params);
                edits.push(if span.is_empty() {
                    ByteEdit::insert(after_name, text)
                } else {
                    ByteEdit::replace(span, text)
                });
                None
            }
            TransformationEdit::SetRuleOptions { element, check_dangling, injective_matching } => {
                let scope = self.scope(element)?;
                if matches!(scope.decl, ScopeDecl::Condition(_)) {
                    return Err(invalid(format!("'{element}' is a condition graph, not a rule")));
                }
                // A multi-rule has its kernel's options unless it says otherwise.
                let (kd, ki) = self.effective_options(scope.parent.as_deref());
                if let Some(v) = check_dangling {
                    self.set_option(scope, "checkDangling", *v, kd, &mut edits);
                }
                if let Some(v) = injective_matching {
                    self.set_option(scope, "injectiveMatching", *v, ki, &mut edits);
                }
                None
            }
            TransformationEdit::SetConditions { element, conditions } => {
                let scope = self.scope(element)?;
                let exprs: Vec<Expr> = conditions
                    .iter()
                    .filter(|c| !c.trim().is_empty())
                    .map(|c| parse_value(c))
                    .collect::<Result<_, _>>()?;
                let existing = scope.decl.rule_items().iter().find_map(|i| match i {
                    RuleItem::Conditions(_, span) => Some(span.clone()),
                    _ => None,
                });
                let line = format!("conditions [{}]", exprs.iter().map(print::expr).collect::<Vec<_>>().join(", "));
                match (existing, exprs.is_empty()) {
                    (Some(span), true) => edits.push(self.removal(span)),
                    (Some(span), false) => edits.push(ByteEdit::replace(span, line)),
                    (None, true) => {}
                    (None, false) => edits.push(self.insert_rule_item(scope, &line, true)?),
                }
                None
            }
            TransformationEdit::AddNode { scope, name, class, action } => {
                check_name(name)?;
                let class = parse_qname(class)?;
                let s = self.scope(scope)?;
                if self.rule_node_names(&s.rule).contains(name.as_str()) || s.names.contains_key(name) {
                    return Err(invalid(format!("there is already a node named '{name}'")));
                }
                let action = match s.decl {
                    ScopeDecl::Condition(_) => None,
                    _ if *action == RuleAction::Preserve => None,
                    _ => Some((syntax_action(*action), 0..0)),
                };
                let decl = NodeDecl {
                    action,
                    name: self::name(name),
                    class,
                    attributes: Vec::new(),
                    attributes_span: None,
                    span: 0..0,
                };
                let mut line = String::new();
                self.printer.node(&mut line, 0, &decl);
                edits.push(self.insert_graph_item(s, line.trim_end(), GraphPlace::AfterNodes)?);
                Some(format!("{scope}::{name}"))
            }
            TransformationEdit::UpdateNode { element, class, action } => {
                let n = self.node(element)?;
                if let Some(c) = class {
                    let q = parse_qname(c)?;
                    edits.push(ByteEdit::replace(n.decl.class.span.clone(), print::qname(&q)));
                }
                if let Some(a) = action
                    && *a != n.action
                {
                    self.change_node_action(element, n, *a, &mut edits)?;
                }
                None
            }
            TransformationEdit::AddEdge { scope, source, target, reference, action } => {
                let s = self.node(source)?;
                let t = self.node(target)?;
                let q = parse_qname(reference)?;
                let scope_id = match scope {
                    Some(s) => s.clone(),
                    // The inner one of the nodes' scopes.
                    None => {
                        if self.ix.scopes.get(&t.scope).is_some_and(|ts| is_within(&self.ix, &ts.parent, &s.scope))
                            || t.scope.len() > s.scope.len()
                        {
                            t.scope.clone()
                        } else {
                            s.scope.clone()
                        }
                    }
                };
                let sc = self.scope(&scope_id)?;
                for (n, id) in [(s, source), (t, target)] {
                    if sc.names.get(&n.decl.name.text) != Some(id) {
                        return Err(invalid(format!("node '{id}' is not visible in '{scope_id}'")));
                    }
                }
                let action = match sc.decl {
                    ScopeDecl::Condition(_) => None,
                    _ if *action == RuleAction::Preserve => None,
                    _ => Some((syntax_action(*action), 0..0)),
                };
                let edge = EdgeDecl {
                    action,
                    source: self::name(&s.decl.name.text),
                    target: self::name(&t.decl.name.text),
                    ty: q,
                    span: 0..0,
                };
                edits.push(self.insert_edge(sc, &edge)?);
                Some(format!("{scope_id}::{}->{}:{}", s.decl.name.text, t.decl.name.text, reference))
            }
            TransformationEdit::UpdateEdge { element, action, reference } => {
                let e = self.ix.edges.get(element).ok_or_else(|| Error::NotFound(format!("no edge '{element}'")))?;
                if let Some(r) = reference {
                    let q = parse_qname(r)?;
                    edits.push(ByteEdit::replace(e.decl.ty.span.clone(), print::qname(&q)));
                }
                if let Some(a) = action
                    && *a != e.action
                {
                    let sc = self.scope(&e.scope)?;
                    if matches!(sc.decl, ScopeDecl::Condition(_)) {
                        return Err(invalid("the action of a condition graph's elements is given by its formula"));
                    }
                    edits.push(self.set_keyword(e.decl.action.as_ref(), e.decl.span.start + 1, *a));
                }
                None
            }
            TransformationEdit::AddAttribute { node, scope, name, value, action, set } => {
                let n = self.node(node)?;
                let scope_id = scope.clone().unwrap_or_else(|| n.scope.clone());
                let sc = self.scope(&scope_id)?;
                let value = parse_value(value)?;
                let own = if matches!(sc.decl, ScopeDecl::Condition(_)) && scope_id != n.scope {
                    sc.polarity
                } else {
                    n.action
                };
                let action = action.filter(|a| *a != own && !matches!(sc.decl, ScopeDecl::Condition(_)));
                let decl = AttributeDecl {
                    action: action.map(|a| (syntax_action(a), 0..0)),
                    set: *set && action.is_none(),
                    name: parse_qname(name)?,
                    value,
                    span: 0..0,
                };
                let line = print::attribute(&decl);
                if scope_id == n.scope {
                    edits.push(self.insert_attribute(AttributeOwner::Node(n.decl), n.decl.class.span.end, &line));
                } else {
                    let reuse =
                        self.ix.reuses.iter().find(|(s, _, id)| *s == scope_id && id.as_deref() == Some(node.as_str()));
                    match reuse {
                        Some((_, r, _)) => {
                            edits.push(self.insert_attribute(AttributeOwner::Reuse(r), r.name.span.end, &line))
                        }
                        None => {
                            let block = format!(
                                "reuse {} {{\n{}{line}\n}}",
                                print::name(&n.decl.name.text),
                                self.printer.indent
                            );
                            edits.push(self.insert_graph_item(sc, &block, GraphPlace::AfterNodes)?);
                        }
                    }
                }
                Some(format!("{scope_id}::{}.{name}", n.decl.name.text))
            }
            TransformationEdit::UpdateAttribute { element, name, value, action, set } => {
                let a = self
                    .ix
                    .attributes
                    .get(element)
                    .ok_or_else(|| Error::NotFound(format!("no attribute '{element}'")))?;
                let sc = self.scope(&a.scope)?;
                let mut decl = a.decl.clone();
                if let Some(n) = name {
                    decl.name = parse_qname(n)?;
                }
                if let Some(v) = value {
                    decl.value = parse_value(v)?;
                }
                if let Some(x) = action {
                    let node_action =
                        a.node.as_ref().and_then(|n| self.ix.nodes.get(n)).map_or(RuleAction::Preserve, |n| n.action);
                    let own = if matches!(sc.decl, ScopeDecl::Condition(_)) { sc.polarity } else { node_action };
                    decl.action = (*x != own).then(|| (syntax_action(*x), 0..0));
                    if decl.action.is_some() {
                        decl.set = false;
                    }
                }
                if let Some(s) = set {
                    decl.set = *s;
                    if *s {
                        decl.action = None;
                    }
                }
                edits.push(ByteEdit::replace(a.decl.span.clone(), print::attribute(&decl)));
                let node = a
                    .node
                    .as_ref()
                    .and_then(|n| self.ix.nodes.get(n))
                    .map_or_else(String::new, |n| n.decl.name.text.clone());
                Some(format!("{}::{node}.{}", a.scope, decl.name.text()))
            }
            TransformationEdit::AddMultiRule { scope, name } => {
                check_name(name)?;
                let sc = self.scope(scope)?;
                if matches!(sc.decl, ScopeDecl::Condition(_)) {
                    return Err(invalid("multi-rules belong to rules and multi-rules"));
                }
                let decl = GraphItem::MultiRule(MultiRuleDecl {
                    name: self::name(name),
                    items: vec![RuleItem::Graph(GraphDecl { items: Vec::new(), body: 0..0, span: 0..0 })],
                    body: 0..0,
                    span: 0..0,
                });
                let mut block = String::new();
                self.printer.graph_item(&mut block, 0, &decl);
                edits.push(self.insert_graph_item(sc, block.trim_end(), GraphPlace::End)?);
                Some(format!("{scope}/{name}"))
            }
            TransformationEdit::SetUnitBody { element, body } => {
                let u = self.ix.units.get(element).ok_or_else(|| Error::NotFound(format!("no unit '{element}'")))?;
                let wrapper = format!("unit u() {{\n{body}\n}}");
                let (parsed, problems) = syntax::parse(&format!("ePackageImport x\n{wrapper}"));
                if let Some(p) = problems.first() {
                    return Err(invalid(format!("the body is not a unit's body: {}", p.message)));
                }
                let Some(Element::Unit(parsed)) = parsed.elements.first() else {
                    return Err(invalid("the body is not a unit's body"));
                };
                let mut printed = String::new();
                self.printer.unit_items(&mut printed, 0, &parsed.items);
                let indent = deeper(self.text, u.span.start);
                let close_indent = indentation(self.text, u.span.start);
                let inner = self.indented(printed.trim_end(), &indent);
                if u.body.is_empty() {
                    return Err(invalid(format!("unit '{element}' has no body")));
                }
                let content =
                    if inner.is_empty() { format!("\n{close_indent}") } else { format!("\n{inner}\n{close_indent}") };
                edits.push(ByteEdit::replace(u.body.start + 1..u.body.end - 1, content));
                None
            }
        };
        Ok((edits, element))
    }

    /// Replaces the `ePackageImport` lines; the first imports go to the top (after the
    /// comments that start the document).
    fn set_imports(&self, imports: &[String]) -> Result<Option<ByteEdit>, Error> {
        let mut lines: Vec<String> = Vec::new();
        for i in imports {
            let q = parse_qname(i).unwrap_or_else(|_| QName { parts: vec![name(i)], span: 0..0 });
            let line = format!("ePackageImport {}", print::import(&q));
            if lines.contains(&line) {
                return Err(invalid(format!("'{i}' is imported twice")));
            }
            lines.push(line);
        }
        let block = lines.join("\n");
        let (Some(first), Some(last)) = (self.imports.first(), self.imports.last()) else {
            if block.is_empty() {
                return Ok(None);
            }
            // After the leading comment lines and blank lines.
            let mut at = 0;
            while at < self.text.len() {
                let end = line_end(self.text, at);
                let line = self.text[at..end].trim();
                if !(line.is_empty() || line.starts_with("//")) {
                    break;
                }
                at = (end + 1).min(self.text.len());
            }
            let rest = &self.text[at..];
            let gap = if rest.trim().is_empty() { "\n" } else { "\n\n" };
            let lead = if at > 0 && !self.text[..at].ends_with("\n\n") && !self.text[..at].trim().is_empty() {
                "\n"
            } else {
                ""
            };
            let tail = if rest.trim().is_empty() { at..self.text.len() } else { at..at };
            return Ok(Some(ByteEdit::replace(tail, format!("{lead}{block}{gap}"))));
        };
        // From the keyword of the first import to the end of the last.
        let keyword = self.text[..first.span.start].trim_end();
        let start = keyword.strip_suffix("ePackageImport").map_or(first.span.start, str::len);
        let span = start..last.span.end;
        if block.is_empty() {
            return Ok(Some(ByteEdit::delete(removal(self.text, span, None))));
        }
        Ok(Some(ByteEdit::replace(span, block)))
    }

    // ------------------------------------------------------------------ inserting

    /// A new rule or unit at the end of the document.
    fn append_element(&self, e: &Element) -> ByteEdit {
        let printed = self.printer.element(e);
        let end = self.text.len();
        let trimmed = self.text.trim_end_matches(['\n', ' ', '\t']).len();
        let lead = if trimmed == 0 { "" } else { "\n\n" };
        ByteEdit::replace(trimmed..end, format!("{lead}{printed}"))
    }

    /// Inserts a line into the body of a rule or multi-rule (before its graph).
    fn insert_rule_item(&self, scope: &ScopeEntry<'a>, line: &str, first: bool) -> Result<ByteEdit, Error> {
        let (start, body) = match scope.decl {
            ScopeDecl::Rule(r) => (r.span.start, r.body.clone()),
            ScopeDecl::Multi(m) => (m.span.start, m.body.clone()),
            ScopeDecl::Condition(_) => return Err(invalid("condition graphs have no options")),
        };
        if body.len() < 2 {
            return Err(invalid("the rule has no body"));
        }
        let items = scope.decl.rule_items();
        let after = if first {
            // After the options already there, before the graph.
            items.iter().filter(|i| !matches!(i, RuleItem::Graph(_))).map(RuleItem::span).next_back()
        } else {
            items.last().map(RuleItem::span)
        };
        Ok(insert_member(self.text, start, body.start + 1, body.end - 1, after, line, false))
    }

    /// Inserts an element into the graph of a scope (a new graph if a rule has none).
    fn insert_graph_item(&self, scope: &ScopeEntry<'a>, block: &str, place: GraphPlace) -> Result<ByteEdit, Error> {
        let (start, body, items): (usize, ByteRange<usize>, Vec<&GraphItem>) = match scope.decl {
            ScopeDecl::Condition(c) => (c.span.start, c.body.clone(), c.items.iter().collect()),
            _ => match scope.decl.graph() {
                Some(g) => (g.span.start, g.body.clone(), g.items.iter().collect()),
                None => {
                    let graph = format!("graph {{\n{}\n}}", self.indented(block, &self.printer.indent));
                    return self.insert_rule_item(scope, &graph, false);
                }
            },
        };
        if body.len() < 2 {
            return Err(invalid("the graph has no body"));
        }
        let after = match place {
            GraphPlace::AfterNodes => items
                .iter()
                .filter(|i| matches!(i, GraphItem::Node(_) | GraphItem::Reuse(_)))
                .map(|i| i.span())
                .next_back(),
            GraphPlace::End => last(&items, |i| i.span()),
        };
        Ok(insert_member(self.text, start, body.start + 1, body.end - 1, after, block, false))
    }

    fn insert_edge(&self, scope: &ScopeEntry<'a>, edge: &EdgeDecl) -> Result<ByteEdit, Error> {
        let lists: Vec<&EdgesDecl> = scope
            .decl
            .graph_items()
            .into_iter()
            .filter_map(|i| if let GraphItem::Edges(e) = i { Some(e) } else { None })
            .collect();
        match lists.last().and_then(|l| l.edges.last()) {
            Some(last_edge) => Ok(ByteEdit::insert(last_edge.span.end, format!(", {}", print::edge(edge)))),
            None => self.insert_graph_item(scope, &print::edges(std::slice::from_ref(edge)), GraphPlace::AfterNodes),
        }
    }

    /// Inserts an attribute into a node's (or `reuse`'s) block, making the block if there is
    /// none (`head_end`: where the node's declaration ends before its block).
    fn insert_attribute(&self, owner: AttributeOwner<'a>, head_end: usize, line: &str) -> ByteEdit {
        match owner.attributes_span() {
            Some(block) if block.len() >= 2 => {
                let after = last(owner.attributes(), |a| a.span.clone());
                insert_member(self.text, owner.span().start, block.start + 1, block.end - 1, after, line, false)
            }
            _ => {
                let indent = indentation(self.text, owner.span().start).to_string();
                let unit = &self.printer.indent;
                ByteEdit::insert(head_end, format!(" {{\n{indent}{unit}{line}\n{indent}}}"))
            }
        }
    }

    // ------------------------------------------------------------------ changing

    /// The options a rule or multi-rule has (for a multi-rule without options of its own).
    fn effective_options(&self, scope: Option<&str>) -> (bool, bool) {
        let Some(id) = scope else { return (true, true) };
        let Some(s) = self.ix.scopes.get(id) else { return (true, true) };
        let (mut d, mut i) = self.effective_options(s.parent.as_deref());
        for item in s.decl.rule_items() {
            match item {
                RuleItem::CheckDangling(v, _) => d = *v,
                RuleItem::InjectiveMatching(v, _) => i = *v,
                _ => {}
            }
        }
        (d, i)
    }

    fn set_option(&self, scope: &ScopeEntry<'a>, keyword: &str, value: bool, default: bool, edits: &mut Vec<ByteEdit>) {
        let existing = scope.decl.rule_items().iter().find_map(|i| match (i, keyword) {
            (RuleItem::CheckDangling(_, span), "checkDangling")
            | (RuleItem::InjectiveMatching(_, span), "injectiveMatching") => Some(span.clone()),
            _ => None,
        });
        let line = format!("{keyword} {value}");
        match existing {
            Some(span) if value == default => edits.push(self.removal(span)),
            Some(span) => edits.push(ByteEdit::replace(span, line)),
            None if value != default => {
                if let Ok(e) = self.insert_rule_item(scope, &line, true) {
                    edits.push(e);
                }
            }
            None => {}
        }
    }

    /// Writes an action keyword (none for `preserve`) where `existing` is, or at `at`.
    fn set_keyword(&self, existing: Option<&(Action, ByteRange<usize>)>, at: usize, action: RuleAction) -> ByteEdit {
        let keyword = syntax_action(action).keyword();
        match (existing, action) {
            (Some((_, span)), RuleAction::Preserve) => {
                let rest = &self.text[span.end..];
                let spaces = rest.len() - rest.trim_start_matches([' ', '\t']).len();
                ByteEdit::delete(span.start..span.end + spaces)
            }
            (Some((_, span)), _) => ByteEdit::replace(span.clone(), keyword),
            (None, RuleAction::Preserve) => ByteEdit::insert(at, ""),
            (None, _) => ByteEdit::insert(at, format!("{keyword} ")),
        }
    }

    /// Changes a node's action, and those of its edges and attributes so that they fit.
    fn change_node_action(
        &self,
        id: &str,
        n: &NodeEntry<'a>,
        action: RuleAction,
        edits: &mut Vec<ByteEdit>,
    ) -> Result<(), Error> {
        let scope = self.scope(&n.scope)?;
        if matches!(scope.decl, ScopeDecl::Condition(_)) {
            return Err(invalid("the action of a condition graph's nodes is given by its formula"));
        }
        edits.push(self.set_keyword(n.decl.action.as_ref(), n.decl.span.start, action));
        let condition = matches!(action, RuleAction::Forbid | RuleAction::Require);
        for e in self.ix.edges.values() {
            if e.source.as_deref() != Some(id) && e.target.as_deref() != Some(id) {
                continue;
            }
            if self.ix.scopes.get(&e.scope).is_some_and(|s| matches!(s.decl, ScopeDecl::Condition(_))) {
                continue;
            }
            let other = if e.source.as_deref() == Some(id) { &e.target } else { &e.source };
            let other_action =
                other.as_ref().and_then(|o| self.ix.nodes.get(o)).map_or(RuleAction::Preserve, |o| o.action);
            let fits = match action {
                RuleAction::Create => RuleAction::Create,
                RuleAction::Delete => RuleAction::Delete,
                RuleAction::Forbid | RuleAction::Require => action,
                RuleAction::Preserve => match (e.action, other_action) {
                    // An edge of a node that was in a condition stays in it only if the other
                    // node still is.
                    (RuleAction::Forbid | RuleAction::Require, o) if o == e.action => e.action,
                    (RuleAction::Forbid | RuleAction::Require, _) if n.action == e.action => RuleAction::Preserve,
                    (a, _) => a,
                },
            };
            if fits != e.action {
                edits.push(self.set_keyword(e.decl.action.as_ref(), e.decl.span.start + 1, fits));
            }
        }
        // Created, deleted and condition nodes only match or create their attributes.
        if matches!(action, RuleAction::Create | RuleAction::Delete) || condition {
            for a in self.ix.attributes.values().filter(|a| a.node.as_deref() == Some(id) && a.scope == n.scope) {
                if a.decl.set || a.decl.action.is_some() {
                    let mut decl = a.decl.clone();
                    decl.set = false;
                    decl.action = None;
                    edits.push(ByteEdit::replace(a.decl.span.clone(), print::attribute(&decl)));
                }
            }
        }
        Ok(())
    }

    fn rename(&self, element: &str, new: &str, edits: &mut Vec<ByteEdit>) -> Result<String, Error> {
        check_name(new)?;
        let escaped = print::name(new);
        if let Some(r) = self
            .ix
            .rules
            .get(element)
            .map(|r| (&r.name, r.span.clone()))
            .or_else(|| self.ix.units.get(element).map(|u| (&u.name, u.span.clone())))
        {
            let (old, _) = r;
            if old.text == new {
                return Ok(element.to_string());
            }
            if self.callable_taken(new) {
                return Err(invalid(format!("there is already a rule or unit named '{new}'")));
            }
            edits.push(ByteEdit::replace(old.span.clone(), escaped.clone()));
            for s in self.ix.steps.values() {
                if let UnitItem::Call { target, .. } = s.item
                    && target.text == old.text
                {
                    edits.push(ByteEdit::replace(target.span.clone(), escaped.clone()));
                }
            }
            return Ok(new.to_string());
        }
        if let Some(s) = self.ix.scopes.get(element) {
            let old = s.decl.name();
            edits.push(ByteEdit::replace(old.span.clone(), escaped.clone()));
            if matches!(s.decl, ScopeDecl::Condition(_)) {
                // The formula's references to the graph.
                for (scope, f, graphs) in &self.ix.formulas {
                    if graphs.iter().any(|g| g == element) && Some(scope) == s.parent.as_ref() {
                        let mut refs = Vec::new();
                        logic_refs(&f.formula, &mut refs);
                        for r in refs.into_iter().filter(|r| r.text == old.text) {
                            edits.push(ByteEdit::replace(r.span.clone(), escaped.clone()));
                        }
                    }
                }
                return Ok(format!("{}#{new}", s.parent.clone().unwrap_or_default()));
            }
            return Ok(format!("{}/{new}", s.parent.clone().unwrap_or_default()));
        }
        let n = self.node(element)?;
        let old = &n.decl.name;
        if old.text == new {
            return Ok(element.to_string());
        }
        let scope = self.scope(&n.scope)?;
        if self.rule_node_names(&scope.rule).contains(new) || scope.names.contains_key(new) {
            return Err(invalid(format!("there is already a node named '{new}'")));
        }
        edits.push(ByteEdit::replace(old.span.clone(), escaped.clone()));
        for e in self.ix.edges.values() {
            if e.source.as_deref() == Some(element) {
                edits.push(ByteEdit::replace(e.decl.source.span.clone(), escaped.clone()));
            }
            if e.target.as_deref() == Some(element) {
                edits.push(ByteEdit::replace(e.decl.target.span.clone(), escaped.clone()));
            }
        }
        for (_, r, node) in &self.ix.reuses {
            if node.as_deref() == Some(element) {
                edits.push(ByteEdit::replace(r.name.span.clone(), escaped.clone()));
            }
        }
        // A node parameter is the node.
        if let ScopeDecl::Rule(r) = scope.decl
            && let Some(p) = r.params.iter().find(|p| p.name.text == old.text)
        {
            edits.push(ByteEdit::replace(p.name.span.clone(), escaped.clone()));
        }
        Ok(format!("{}::{new}", n.scope))
    }

    // ------------------------------------------------------------------ deleting

    fn delete(&self, elements: &[String], edits: &mut Vec<ByteEdit>) -> Result<(), Error> {
        let mut nodes: HashSet<&str> = HashSet::new();
        let mut edges: HashSet<&str> = HashSet::new();
        let mut attributes: HashSet<&str> = HashSet::new();
        let mut steps: HashSet<String> = HashSet::new();
        let mut callables: HashSet<&str> = HashSet::new();
        for id in elements {
            if let Some(r) = self.ix.rules.get(id) {
                edits.push(self.removal(r.span.clone()));
                callables.insert(r.name.text.as_str());
            } else if let Some(u) = self.ix.units.get(id) {
                edits.push(self.removal(u.span.clone()));
                callables.insert(u.name.text.as_str());
            } else if let Some(s) = self.ix.scopes.get(id) {
                match s.decl {
                    ScopeDecl::Multi(m) => edits.push(self.removal(m.span.clone())),
                    ScopeDecl::Condition(c) => self.delete_condition_graph(id, s, c, edits),
                    ScopeDecl::Rule(_) => {}
                }
            } else if self.ix.nodes.contains_key(id) {
                nodes.insert(id);
            } else if self.ix.edges.contains_key(id) {
                edges.insert(id);
            } else if self.ix.attributes.contains_key(id) {
                attributes.insert(id);
            } else if self.ix.steps.contains_key(id) {
                steps.insert(id.clone());
            } else {
                return Err(Error::NotFound(format!("no element '{id}'")));
            }
        }
        // Deleted nodes take their edges and `reuse`s with them.
        for (eid, e) in &self.ix.edges {
            if e.source.as_deref().is_some_and(|s| nodes.contains(s))
                || e.target.as_deref().is_some_and(|t| nodes.contains(t))
            {
                edges.insert(eid);
            }
        }
        for n in &nodes {
            edits.push(self.removal(self.ix.nodes[*n].decl.span.clone()));
        }
        let mut deleted_reuses: HashSet<*const ReuseDecl> = HashSet::new();
        for (_, r, node) in &self.ix.reuses {
            if node.as_deref().is_some_and(|n| nodes.contains(n)) {
                edits.push(self.removal(r.span.clone()));
                deleted_reuses.insert(*r);
            }
        }
        self.delete_edges(&edges, edits);
        self.delete_attributes(&attributes, &nodes, &deleted_reuses, edits);
        // Calls of deleted rules and units.
        for (sid, s) in &self.ix.steps {
            if let UnitItem::Call { target, .. } = s.item
                && callables.contains(target.text.as_str())
                && !callables.contains(self.ix.units.get(&s.unit).map_or("", |u| u.name.text.as_str()))
            {
                steps.insert(sid.clone());
            }
        }
        self.delete_steps(&steps, edits);
        Ok(())
    }

    fn delete_condition_graph(&self, id: &str, s: &ScopeEntry<'a>, c: &ConditionGraphDecl, edits: &mut Vec<ByteEdit>) {
        let Some((_, f, graphs)) = self.ix.formulas.iter().find(|(_, _, graphs)| graphs.iter().any(|g| g == id)) else {
            return;
        };
        let _ = s;
        if graphs.len() == 1 {
            edits.push(self.removal(f.span.clone()));
            return;
        }
        edits.push(self.removal(c.span.clone()));
        match without(&f.formula, &c.name.text) {
            Some(l) if !f.formula_span.is_empty() => {
                edits.push(ByteEdit::replace(f.formula_span.clone(), format!("formula {}", print::logic(&l))))
            }
            Some(_) => {}
            None => edits.push(self.removal(f.span.clone())),
        }
    }

    /// Deletes edges from their lists: a list without edges goes, else the runs of deleted
    /// edges with the commas between them.
    fn delete_edges(&self, edges: &HashSet<&str>, edits: &mut Vec<ByteEdit>) {
        let mut lists: HashMap<*const EdgesDecl, (&EdgesDecl, Vec<usize>)> = HashMap::new();
        for id in edges {
            let e = &self.ix.edges[*id];
            let i = e.list.edges.iter().position(|x| std::ptr::eq(x, e.decl)).unwrap_or(0);
            lists.entry(e.list as *const EdgesDecl).or_insert((e.list, Vec::new())).1.push(i);
        }
        for (list, mut deleted) in lists.into_values() {
            deleted.sort_unstable();
            deleted.dedup();
            if deleted.len() == list.edges.len() {
                edits.push(self.removal(list.span.clone()));
                continue;
            }
            for run in runs(&deleted) {
                let (first, last) = (run[0], run[run.len() - 1]);
                let range = if first > 0 {
                    list.edges[first - 1].span.end..list.edges[last].span.end
                } else {
                    list.edges[first].span.start..list.edges[last + 1].span.start
                };
                edits.push(ByteEdit::delete(range));
            }
        }
    }

    fn delete_attributes(
        &self,
        attributes: &HashSet<&str>,
        nodes: &HashSet<&str>,
        deleted_reuses: &HashSet<*const ReuseDecl>,
        edits: &mut Vec<ByteEdit>,
    ) {
        // Grouped by what declares them.
        let mut owners: HashMap<usize, (AttributeOwner<'a>, Vec<&AttributeDecl>)> = HashMap::new();
        for id in attributes {
            let a = &self.ix.attributes[*id];
            if a.node.as_deref().is_some_and(|n| nodes.contains(n)) {
                continue;
            }
            if let AttributeOwner::Reuse(r) = a.owner
                && deleted_reuses.contains(&(r as *const ReuseDecl))
            {
                continue;
            }
            let key = a.owner.span().start;
            owners.entry(key).or_insert((a.owner, Vec::new())).1.push(a.decl);
        }
        for (owner, deleted) in owners.into_values() {
            let all = owner.attributes();
            if deleted.len() < all.len() {
                for d in deleted {
                    edits.push(self.removal(d.span.clone()));
                }
                continue;
            }
            match owner {
                // A `reuse` without attributes says nothing.
                AttributeOwner::Reuse(r) => edits.push(self.removal(r.span.clone())),
                AttributeOwner::Node(n) => {
                    if let Some(block) = &n.attributes_span {
                        edits.push(ByteEdit::delete(n.class.span.end..block.end));
                    } else {
                        for d in deleted {
                            edits.push(self.removal(d.span.clone()));
                        }
                    }
                }
            }
        }
    }

    /// Deletes elements of units: from a block, with their lines; from a list written on one
    /// line, with the space before them (a choice left empty goes with its comma).
    fn delete_steps(&self, steps: &HashSet<String>, edits: &mut Vec<ByteEdit>) {
        let mut lists: HashMap<*const UnitItem, (&[UnitItem], bool, Vec<usize>)> = HashMap::new();
        for id in steps {
            let s = &self.ix.steps[id];
            let i = s.siblings.iter().position(|x| std::ptr::eq(x, s.item)).unwrap_or(0);
            let key = s.siblings.as_ptr();
            lists.entry(key).or_insert((s.siblings, s.inline, Vec::new())).2.push(i);
        }
        for (list, inline, deleted) in lists.into_values() {
            for i in deleted {
                let span = list[i].span().clone();
                if !inline {
                    edits.push(self.removal(span));
                } else if list.len() > 1 {
                    let start = if i > 0 { list[i - 1].span().end } else { span.start };
                    let end = if i > 0 { span.end } else { list[1].span().start };
                    edits.push(ByteEdit::delete(start..end));
                } else {
                    // The only element of a choice: the choice goes, with a comma.
                    let before = self.text[..span.start].trim_end();
                    let after = self.text[span.end..].trim_start();
                    if before.ends_with(',') {
                        edits.push(ByteEdit::delete(before.len() - 1..span.end));
                    } else if after.starts_with(',') {
                        let end = span.end + (self.text[span.end..].len() - after.len()) + 1;
                        let rest = &self.text[end..];
                        edits.push(ByteEdit::delete(span.start..end + (rest.len() - rest.trim_start().len())));
                    } else {
                        edits.push(ByteEdit::delete(span));
                    }
                }
            }
        }
    }
}

/// Where a new element of a graph goes.
#[derive(Clone, Copy)]
enum GraphPlace {
    /// After the nodes (and `reuse`s).
    AfterNodes,
    /// At the end.
    End,
}

/// Whether `scope` is `outer` or inside it.
fn is_within(ix: &Index, scope: &Option<String>, outer: &str) -> bool {
    let mut cur = scope.clone();
    while let Some(s) = cur {
        if s == outer {
            return true;
        }
        cur = ix.scopes.get(&s).and_then(|e| e.parent.clone());
    }
    false
}

/// Runs of consecutive numbers.
fn runs(sorted: &[usize]) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = Vec::new();
    for &i in sorted {
        match out.last_mut() {
            Some(run) if *run.last().unwrap_or(&usize::MAX) + 1 == i => run.push(i),
            _ => out.push(vec![i]),
        }
    }
    out
}

impl Workspace {
    /// The text edits that make `edit` in a transformation document (of either syntax). The
    /// document is not changed. A `.henshin` file is edited through its textual form, compiled
    /// and written again; the edit fails if that has errors.
    pub fn edit_transformation(
        &self,
        name: &str,
        edit: &TransformationEdit,
    ) -> Result<TransformationEditResult, Error> {
        let (syntax, language) = self.transformation_syntax(name)?;
        let text = self.document_text(name).unwrap_or_default();
        if language == Language::Henshin {
            let doc = Doc::new(text, &syntax.document);
            let (edits, element) = doc.edit(edit)?;
            return Ok(TransformationEditResult { edits: text_edits(text, edits)?, element });
        }
        let mm = self.metamodel();
        if let Some(p) = syntax.problems.iter().find(|p| p.is_error()) {
            return Err(invalid(format!("{name} cannot be read: {}", p.message)));
        }
        let textual = crate::henshin_documents::print(&syntax.document, "");
        let (parsed, _) = syntax::parse(&textual);
        let doc = Doc::new(&textual, &parsed);
        let (edits, element) = doc.edit(edit)?;
        let changed = apply_edits(&textual, edits)?;
        let xml = write_model(mm, name, &changed, syntax.template.as_ref())?;
        let edits = replacements_to_edits(text, line_replacements(text, &xml));
        Ok(TransformationEditResult { edits, element })
    }
}

/// A `.henshin` file of a text that must compile without errors.
fn write_model(mm: &Metamodel, name: &str, text: &str, template: Option<&xmi::Template>) -> Result<String, Error> {
    let module = Module::compile(mm, text);
    if let Some(p) = module.problems.iter().find(|p| p.is_error()) {
        return Err(invalid(format!("the change would make the transformation invalid: {}", p.message)));
    }
    Ok(xmi::write(mm, &module, &WriteOptions { path: name, name: None, template }))
}
