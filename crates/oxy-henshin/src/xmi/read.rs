//! Reading `.henshin` files into syntax trees (see the [module documentation](super)).
//!
//! The tree's spans are in the XML: names at their `name` attributes, types and features at
//! their `href`s, values at their `value` attributes, calls at their ids in `subUnits`, and
//! every element at its XML element. Nodes without names (common in files made with Henshin's
//! graphical editor) get names made from their class (`account`, `account2`, …).

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

use oxy_ecore::xml::{self, Element as XmlElement};
use oxy_ecore::{ClassifierId, ElementRef, FeatureId, Metamodel, PackageId, Problem};

use super::{Extra, Keys, NESTED, Template, unit_key};
use crate::js;
use crate::syntax::*;

/// A `.henshin` file read as a syntax tree.
#[derive(Clone, Debug, Default)]
pub struct Read {
    pub document: Document,
    /// Problems of the file as a model (the compiler reports the rest).
    pub problems: Vec<Problem>,
    /// What [`write`](super::write) keeps when it replaces the file.
    pub template: Template,
}

/// Reads a `.henshin` file. `path` is where it is, for resolving references to `.ecore` files.
/// Never fails: problems are reported with spans in `text`. An empty file is an empty module.
pub fn read(mm: &Metamodel, text: &str, path: &str) -> Read {
    if text.trim().is_empty() {
        return Read::default();
    }
    let root = match xml::parse(text, path) {
        Ok(root) => root,
        Err(e) => {
            let at = e.location.map_or(0, |(line, column)| {
                let start: usize = text.split_inclusive('\n').take(line.saturating_sub(1)).map(str::len).sum();
                (start + column.saturating_sub(1)).min(text.len())
            });
            return Read { problems: vec![Problem::error(at..at, e.message)], ..Read::default() };
        }
    };
    let mut r = Reader {
        mm,
        text,
        dir: Path::new(path).parent().map(Path::to_path_buf).unwrap_or_default(),
        root: &root,
        by_id: HashMap::new(),
        problems: Vec::new(),
        template: Template::default(),
        keys: Keys::default(),
        packages: Vec::new(),
        unit_names: HashMap::new(),
        nested: HashMap::new(),
        unbound: HashMap::new(),
    };
    if root.local_name() != "Module" {
        let span = tag_span(&root);
        r.problems.push(Problem::error(span, "not a Henshin module: the root element must be a 'henshin:Module'"));
        return Read { problems: r.problems, ..Read::default() };
    }
    r.index(&root);
    let document = r.document();
    Read { document, problems: r.problems, template: r.template }
}

// ---------------------------------------------------------------------- XML helpers

/// The element's tag name.
fn tag_span(e: &XmlElement) -> Range<usize> {
    e.offset + 1..e.offset + 1 + e.name.len()
}

fn whole(e: &XmlElement) -> Range<usize> {
    e.offset..e.end
}

fn xmi_id(e: &XmlElement) -> Option<&str> {
    e.attr_ns(xml::XMI_NS, "id").or_else(|| e.attr("xmi:id"))
}

/// The local name of the element's `xsi:type` (`Rule`, `NestedCondition`, …).
fn xsi_type(e: &XmlElement) -> Option<&str> {
    let t = e.attr_ns(xml::XSI_NS, "type").or_else(|| e.attr("xsi:type"))?;
    Some(xml::split_qname(t).1)
}

/// The span of an attribute's value (without the quotes), from the start tag's text.
fn attr_span(text: &str, e: &XmlElement, name: &str) -> Option<Range<usize>> {
    let bytes = text.as_bytes();
    let mut i = e.offset + 1 + e.name.len();
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] == b'>' || bytes[i] == b'/' {
            return None;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' && bytes[i] != b'>' {
            i += 1;
        }
        let attr = &text[start..i];
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'=') {
            i += 1;
        }
        let quote = *bytes.get(i)?;
        if quote != b'"' && quote != b'\'' {
            return None;
        }
        let value_start = i + 1;
        let value_end = value_start + text[value_start..].find(quote as char)?;
        if attr == name {
            return Some(value_start..value_end);
        }
        i = value_end + 1;
    }
}

/// Element-local names of what writing produces; the rest is kept in the template.
const MANAGED_CHILDREN: &[&str] = &[
    "imports",
    "units",
    "parameters",
    "parameterMappings",
    "lhs",
    "rhs",
    "attributeConditions",
    "mappings",
    "multiRules",
    "multiMappings",
    "javaImports",
    "nodes",
    "edges",
    "formula",
    "attributes",
    "type",
    "child",
    "left",
    "right",
    "conclusion",
];

const MANAGED_ATTRIBUTES: &[&str] = &[
    "xmi:id",
    "xsi:type",
    "xmi:version",
    "name",
    "incoming",
    "outgoing",
    "source",
    "target",
    "origin",
    "image",
    "value",
    "kind",
    "checkDangling",
    "injectiveMatching",
    "subUnits",
    "subUnit",
    "if",
    "then",
    "else",
    "iterations",
    "strict",
    "rollback",
    "conditionText",
    "key",
    "href",
];

/// What a node of a rule is, before it becomes a declaration.
#[derive(Clone)]
struct RuleNodeInfo<'x> {
    name: String,
    class: QName,
    kind: Action,
    lhs: Option<&'x XmlElement>,
    rhs: Option<&'x XmlElement>,
    /// For copies of kernel nodes (in multi-rules): the kernel's node.
    kernel: Option<usize>,
}

impl RuleScope<'_> {
    /// The names a made-up node name must not be: the nodes' and the parameters'.
    fn taken(&self) -> HashSet<String> {
        self.names.union(&self.params).cloned().collect()
    }
}

/// An edge of one side of a rule, with its ends by node index.
struct EdgeInfo<'x> {
    element: &'x XmlElement,
    source: usize,
    target: usize,
    feature: String,
    source_span: Range<usize>,
    target_span: Range<usize>,
    feature_span: Range<usize>,
}

/// A rule read so far: its nodes (by element id) and names, for its multi-rules.
#[derive(Default, Clone)]
struct RuleScope<'x> {
    nodes: Vec<RuleNodeInfo<'x>>,
    /// Node index by XML id (left- and right-hand side elements).
    by_element: HashMap<String, usize>,
    /// The names of the nodes.
    names: HashSet<String>,
    /// The names of the rule's parameters: names made up for nodes avoid them (a node named
    /// like a parameter is a parameter).
    params: HashSet<String>,
    /// The edges, as `(source, feature, target)` by node index and side.
    lhs_edges: Vec<(usize, String, usize)>,
    rhs_edges: Vec<(usize, String, usize)>,
}

struct Reader<'a, 'x> {
    mm: &'a Metamodel,
    text: &'a str,
    dir: PathBuf,
    root: &'x XmlElement,
    by_id: HashMap<String, &'x XmlElement>,
    problems: Vec<Problem>,
    template: Template,
    keys: Keys,
    /// The imported packages.
    packages: Vec<PackageId>,
    /// Names of rules and units, by XML id.
    unit_names: HashMap<String, String>,
    /// Units made for nested blocks, by XML id, with their role.
    nested: HashMap<String, String>,
    /// Variables of units for the parameters of the units they call that they map nothing to,
    /// by the unit's XML id.
    unbound: HashMap<String, Vec<Unbound>>,
}

/// The parameter of an [`Unbound`] for the variables of the called unit.
const INHERITED: &str = "*";

/// A parameter of a called unit without a mapping from the calling unit: Henshin leaves it
/// unbound, the text passes a variable of the calling unit.
struct Unbound {
    /// The XML id of the called unit.
    callee: String,
    /// The parameter (of the called unit) by name.
    param: String,
    /// The calling unit's variable.
    name: String,
    ty: QName,
}

impl<'x> Reader<'_, 'x> {
    fn error(&mut self, span: Range<usize>, message: impl Into<String>) {
        self.problems.push(Problem::error(span, message));
    }

    fn warning(&mut self, span: Range<usize>, message: impl Into<String>) {
        self.problems.push(Problem::warning(span, message));
    }

    fn index(&mut self, e: &'x XmlElement) {
        if let Some(id) = xmi_id(e) {
            self.by_id.insert(id.to_string(), e);
        }
        for c in &e.children {
            self.index(c);
        }
    }

    /// Resolves a reference: an id, or a fragment path (`//@units.0/@lhs/@nodes.1`).
    fn resolve(&self, reference: &str) -> Option<&'x XmlElement> {
        if let Some(e) = self.by_id.get(reference) {
            return Some(e);
        }
        let path = reference.strip_prefix('#').unwrap_or(reference).strip_prefix('/')?;
        let mut current = self.root;
        for segment in path.split('/').filter(|s| !s.is_empty()) {
            let segment = segment.strip_prefix('@')?;
            let (feature, index) = match segment.split_once('.') {
                Some((f, i)) => (f, i.parse().ok()?),
                None => (segment, 0),
            };
            current = current.children.iter().filter(|c| c.local_name() == feature).nth(index)?;
        }
        Some(current)
    }

    /// The references in an attribute (a list of ids), with their spans.
    fn references(&self, e: &XmlElement, attr: &str) -> Vec<(String, Range<usize>)> {
        let mut out = Vec::new();
        if let Some(value) = e.attr(attr) {
            let base = attr_span(self.text, e, attr).map_or(e.offset, |s| s.start);
            let raw = attr_span(self.text, e, attr).map(|s| &self.text[s]).unwrap_or(value);
            let exact = raw == value;
            let mut at = 0;
            for part in value.split_whitespace() {
                let i = value[at..].find(part).map_or(at, |i| at + i);
                at = i + part.len();
                let span = if exact { base + i..base + i + part.len() } else { base..base + raw.len() };
                out.push((part.to_string(), span));
            }
        }
        // References to other files are elements with an `href`.
        for c in e.children_named(attr) {
            if let Some(href) = c.attr("href") {
                let span = attr_span(self.text, c, "href").unwrap_or_else(|| whole(c));
                out.push((href.to_string(), span));
            }
        }
        out
    }

    /// The one element a reference attribute names.
    fn reference(&mut self, e: &XmlElement, attr: &str) -> Option<(&'x XmlElement, Range<usize>)> {
        let (r, span) = self.references(e, attr).into_iter().next()?;
        match self.resolve(&r) {
            Some(target) => Some((target, span)),
            None => {
                let what =
                    if r.contains('#') { "elements of other files are not supported" } else { "no such element" };
                self.error(span, format!("cannot resolve '{r}': {what}"));
                None
            }
        }
    }

    /// The span of a name: its `name` attribute, else the element's tag.
    fn name_span(&self, e: &XmlElement) -> Range<usize> {
        attr_span(self.text, e, "name").unwrap_or_else(|| tag_span(e))
    }

    /// Records an element's id and the content writing does not produce. `name_is_extra`: the
    /// element's name is kept (graphs, the module, attribute conditions).
    fn record(&mut self, key: &str, e: &XmlElement, name_is_extra: bool) {
        if let Some(id) = xmi_id(e) {
            self.template.ids.insert(key.to_string(), id.to_string());
        }
        let mut extra = Extra::default();
        for (k, v) in &e.attrs {
            let managed = (MANAGED_ATTRIBUTES.contains(&k.as_str()) && !(name_is_extra && k == "name"))
                || XmlElement::is_xmlns_attr(k);
            if !managed {
                extra.attributes.push((k.clone(), v.clone()));
            }
        }
        for c in &e.children {
            let ours = c.local_name() == "annotations" && c.attr("key") == Some(NESTED);
            if !MANAGED_CHILDREN.contains(&c.local_name()) && !ours {
                extra.children.push(self.text[c.offset..c.end].to_string());
            }
        }
        if !extra.attributes.is_empty() || !extra.children.is_empty() {
            self.template.extras.insert(key.to_string(), extra);
        }
    }

    /// Keeps an element's name in the file when the text names it otherwise.
    fn keep_name(&mut self, key: &str, e: &XmlElement, chosen: &str) {
        let original = e.attr("name");
        if original != Some(chosen) {
            self.template.extras.entry(key.to_string()).or_default().name = Some(original.map(str::to_string));
        }
    }

    // ------------------------------------------------------------------ metamodel references

    /// The packages a resource reference (the part of an `href` before `#`) names.
    fn resource_roots(&self, uri: &str) -> Vec<PackageId> {
        let mm = self.mm;
        if let Some(p) = mm.package_by_ns_uri(uri) {
            return vec![p];
        }
        let resolved = self.dir.join(uri);
        let normalized = normalize(&resolved);
        let file_name = Path::new(uri).file_name();
        let by_path = mm.resources().iter().find(|r| {
            let p = Path::new(&r.uri);
            p == resolved || normalize(p) == normalized || r.is_named(uri)
        });
        let by_name =
            || mm.resources().iter().find(|r| file_name.is_some() && Path::new(&r.uri).file_name() == file_name);
        if let Some(r) = by_path.or_else(by_name) {
            return r.roots.clone();
        }
        // An `.ecore` file whose package is loaded from elsewhere (e.g. an OCLinEcore document).
        if let Ok(text) = std::fs::read_to_string(&normalized)
            && let Ok(e) = xml::parse(&text, uri)
            && let Some(p) = e.attr("nsURI").and_then(|ns| mm.package_by_ns_uri(ns))
        {
            return vec![p];
        }
        Vec::new()
    }

    /// The metamodel element an `href` names.
    fn resolve_href(&self, href: &str) -> Option<ElementRef> {
        let (uri, fragment) = href.split_once('#')?;
        let roots = self.resource_roots(uri);
        let rest = fragment.strip_prefix('/')?;
        let (root, path) = rest.split_once('/').unwrap_or((rest, ""));
        let index: usize = if root.is_empty() { 0 } else { root.parse().ok()? };
        let package = *roots.get(index)?;
        self.mm.resolve_in_package(package, path)
    }

    /// The `href` of an element's `type` child (or `type` attribute), with its span.
    fn type_href(&self, e: &XmlElement) -> Option<(String, Range<usize>)> {
        if let Some(t) = e.children_named("type").next() {
            let href = t.attr("href")?;
            return Some((href.to_string(), attr_span(self.text, t, "href").unwrap_or_else(|| whole(t))));
        }
        let t = e.attr("type")?;
        Some((t.to_string(), attr_span(self.text, e, "type").unwrap_or_else(|| tag_span(e))))
    }

    /// How a classifier is named in the text: by its name, qualified if the name is
    /// ambiguous.
    fn classifier_name(&self, c: ClassifierId) -> String {
        let mm = self.mm;
        let name = &mm.classifier(c).name;
        let found = mm.find_classifiers(name);
        let imported = |x: &ClassifierId| self.is_imported(mm.classifier(*x).package);
        let unique = found.len() == 1
            || (imported(&c) && found.iter().filter(|x| imported(x)).count() == 1)
            || mm.classifier(c).package == mm.ecore_package();
        if unique { name.clone() } else { mm.qualified_name(c).replace("::", ".") }
    }

    fn is_imported(&self, mut p: PackageId) -> bool {
        loop {
            if self.packages.contains(&p) {
                return true;
            }
            match self.mm.package(p).super_package {
                Some(sp) => p = sp,
                None => return false,
            }
        }
    }

    /// A dotted name with one span for all parts.
    fn qname(text: &str, span: Range<usize>) -> QName {
        QName { parts: text.split('.').map(|p| Name { text: p.to_string(), span: span.clone() }).collect(), span }
    }

    /// The type of a node, parameter or the like, as a name.
    fn type_name(&mut self, e: &XmlElement) -> (QName, Option<ElementRef>) {
        let Some((href, span)) = self.type_href(e) else {
            let span = tag_span(e);
            self.error(span.clone(), "the element has no type");
            return (Self::qname("", span), None);
        };
        let target = self.resolve_href(&href);
        let text = match target {
            Some(ElementRef::Classifier(c)) => self.classifier_name(c),
            _ => href.rsplit(['/', '#']).next().unwrap_or("").to_string(),
        };
        (Self::qname(&text, span), target)
    }

    /// The feature an attribute or edge has as its type.
    fn feature(&self, e: &XmlElement) -> (String, Range<usize>, Option<FeatureId>) {
        match self.type_href(e) {
            Some((href, span)) => {
                let f = match self.resolve_href(&href) {
                    Some(ElementRef::Feature(f)) => Some(f),
                    _ => None,
                };
                let name = f.map_or_else(
                    || href.rsplit(['/', '#']).next().unwrap_or("").to_string(),
                    |f| self.mm.feature(f).name.clone(),
                );
                (name, span, f)
            }
            None => (String::new(), tag_span(e), None),
        }
    }

    // ------------------------------------------------------------------ values

    /// An attribute value or condition: JavaScript, read as an expression of the textual
    /// syntax. String values of enumeration-typed attributes are literals; JavaScript the
    /// textual syntax cannot express is `javascript("…")`.
    fn value(&mut self, e: &XmlElement, attr: &str, feature: Option<FeatureId>) -> Expr {
        let raw = e.attr(attr).unwrap_or("").to_string();
        let span = attr_span(self.text, e, attr).unwrap_or_else(|| tag_span(e));
        let exact = self.text.get(span.clone()) == Some(raw.as_str());
        let base = span.start;
        let whole_span = span.clone();
        let map = move |s: Range<usize>| if exact { base + s.start..base + s.end } else { whole_span.clone() };
        match js::parse(&raw, &map) {
            Ok(mut expr) => {
                if let (ExprKind::Str(s), Some(f)) = (&expr.kind, feature)
                    && let Some(ty) = self.mm.feature(f).ty.filter(|t| self.mm.is_enum(*t))
                    && self.mm.enum_literal_index(ty, s).is_some()
                {
                    expr.kind = ExprKind::Qualified(vec![self.classifier_name(ty), s.clone()]);
                }
                expr
            }
            Err(_) => {
                let string = Expr { kind: ExprKind::Str(raw), span: span.clone() };
                Expr { kind: ExprKind::Call(js::SCRIPT.into(), vec![string]), span }
            }
        }
    }

    // ------------------------------------------------------------------ the module

    fn document(&mut self) -> Document {
        let root = self.root;
        self.record("module", root, true);
        let mut doc = Document::default();
        for i in root.children_named("imports") {
            let Some(href) = i.attr("href") else { continue };
            let span = attr_span(self.text, i, "href").unwrap_or_else(|| whole(i));
            let (name, p) = match self.resolve_href(href) {
                Some(ElementRef::Package(p)) => (self.mm.package_qualified_name(p).replace("::", "."), Some(p)),
                _ => (href.split('#').next().unwrap_or(href).to_string(), None),
            };
            if let Some(p) = p {
                self.packages.push(p);
            }
            doc.imports.push(QName { parts: vec![Name { text: name, span: span.clone() }], span });
        }
        for s in root.children_named("subModules") {
            self.warning(tag_span(s), "sub-modules are not supported: their rules and units are ignored");
        }
        // Names of rules and units, and the units made for nested blocks.
        let mut taken = HashSet::new();
        let units: Vec<&'x XmlElement> = root.children_named("units").collect();
        for (i, u) in units.iter().enumerate() {
            let mut name = text_name(u.attr("name").unwrap_or(""));
            if name.is_empty() || taken.contains(&name) {
                let base = if xsi_type(u) == Some("Rule") { "rule" } else { "unit" };
                name = fresh(&format!("{base}{}", i + 1), &taken);
            }
            taken.insert(name.clone());
            if let Some(id) = xmi_id(u) {
                self.unit_names.insert(id.to_string(), name);
                if let Some(a) = u.children_named("annotations").find(|a| a.attr("key") == Some(NESTED)) {
                    self.nested.insert(id.to_string(), a.attr("value").unwrap_or("inline").to_string());
                }
            }
        }
        self.find_unbound(&units);
        for (i, u) in units.iter().enumerate() {
            let name = xmi_id(u)
                .and_then(|id| self.unit_names.get(id).cloned())
                .unwrap_or_else(|| fresh(&format!("unit{}", i + 1), &taken));
            if xsi_type(u) == Some("Rule") {
                self.template.units.push(name.clone());
                doc.elements.push(Element::Rule(self.rule(u, name)));
            } else if !xmi_id(u).is_some_and(|id| self.nested.contains_key(id)) {
                self.template.units.push(name.clone());
                doc.elements.push(Element::Unit(self.unit(u, name)));
            }
        }
        doc
    }

    fn params(&mut self, e: &XmlElement, key: &str) -> Vec<ParamDecl> {
        let mut out = Vec::new();
        for p in e.children_named("parameters") {
            let name = text_name(p.attr("name").unwrap_or(""));
            let pkey = format!("{key}/param:{name}");
            self.record(&pkey, p, false);
            self.keep_name(&pkey, p, &name);
            let kind = match p.attr("kind") {
                Some("IN") => ParamKind::In,
                Some("OUT") => ParamKind::Out,
                Some("INOUT") => ParamKind::InOut,
                Some("VAR") => ParamKind::Var,
                _ => ParamKind::Unknown,
            };
            // Henshin's parameters may have no type.
            let ty = if self.type_href(p).is_some() {
                self.type_name(p).0
            } else {
                let at = self.name_span(p).end;
                QName { parts: Vec::new(), span: at..at }
            };
            out.push(ParamDecl { kind, name: Name { text: name, span: self.name_span(p) }, ty, span: whole(p) });
        }
        out
    }

    // ------------------------------------------------------------------ rules

    fn rule(&mut self, e: &'x XmlElement, name: String) -> RuleDecl {
        let key = unit_key(&name);
        self.record(&key, e, false);
        self.keep_name(&key, e, &name);
        let params = self.params(e, &key);
        let (items, scope) = self.rule_items(e, &key, &params, None);
        self.template.nodes.insert(name.clone(), scope.nodes.iter().map(|n| n.name.clone()).collect());
        RuleDecl {
            name: Name { text: name, span: self.name_span(e) },
            params,
            params_span: tag_span(e),
            items,
            body: whole(e),
            span: whole(e),
        }
    }

    /// The elements of a rule (or multi-rule, with its kernel's scope and flags).
    fn rule_items(
        &mut self,
        e: &'x XmlElement,
        key: &str,
        params: &[ParamDecl],
        kernel: Option<(&RuleScope<'x>, bool, bool)>,
    ) -> (Vec<RuleItem>, RuleScope<'x>) {
        let mut items = Vec::new();
        let flag = |attr: &str| e.attr(attr).map(|v| v != "false");
        let (kernel_dangling, kernel_injective) = kernel.map_or((true, true), |(_, d, i)| (d, i));
        let dangling = flag("checkDangling").unwrap_or(kernel_dangling);
        let injective = flag("injectiveMatching").unwrap_or(kernel_injective);
        if dangling != kernel_dangling {
            items.push(RuleItem::CheckDangling(
                dangling,
                attr_span(self.text, e, "checkDangling").unwrap_or_else(|| tag_span(e)),
            ));
        }
        if injective != kernel_injective {
            let span = attr_span(self.text, e, "injectiveMatching").unwrap_or_else(|| tag_span(e));
            items.push(RuleItem::InjectiveMatching(injective, span));
        }
        for j in e.children_named("javaImports") {
            let span = whole(j);
            items.push(RuleItem::JavaImport(Self::qname(j.text.trim(), span.clone()), span));
        }
        let mut conditions = Vec::new();
        for (i, c) in e.children_named("attributeConditions").enumerate() {
            self.record(&format!("{key}/condition:{i}"), c, true);
            conditions.push(self.value(c, "conditionText", None));
        }
        if let Some(first) = conditions.first() {
            let span = first.span.clone();
            items.push(RuleItem::Conditions(conditions, span));
        }
        let (graph_items, scope) = self.graph(e, key, params, kernel.map(|(k, _, _)| k));
        let span = e.children_named("lhs").next().map_or_else(|| whole(e), whole);
        items.push(RuleItem::Graph(GraphDecl { items: graph_items, body: span.clone(), span }));
        (items, scope)
    }

    /// The graph of a rule: its nodes and edges with their actions, its application
    /// condition, and its multi-rules.
    fn graph(
        &mut self,
        e: &'x XmlElement,
        key: &str,
        params: &[ParamDecl],
        kernel: Option<&RuleScope<'x>>,
    ) -> (Vec<GraphItem>, RuleScope<'x>) {
        let lhs = e.children_named("lhs").next();
        let rhs = e.children_named("rhs").next();
        let lhs_key = format!("{key}/lhs");
        let rhs_key = format!("{key}/rhs");
        if let Some(l) = lhs {
            self.record(&lhs_key, l, true);
        }
        if let Some(r) = rhs {
            self.record(&rhs_key, r, true);
        }
        let lhs_nodes: Vec<&XmlElement> = lhs.map(|g| g.children_named("nodes").collect()).unwrap_or_default();
        let rhs_nodes: Vec<&XmlElement> = rhs.map(|g| g.children_named("nodes").collect()).unwrap_or_default();
        // Preserved nodes are mapped from the left- to the right-hand side.
        let mut image: HashMap<String, &XmlElement> = HashMap::new();
        let mut has_origin: HashSet<String> = HashSet::new();
        for m in e.children_named("mappings") {
            let (Some((o, _)), Some((i, _))) = (self.reference(m, "origin"), self.reference(m, "image")) else {
                continue;
            };
            if let (Some(oid), Some(iid)) = (xmi_id(o), xmi_id(i)) {
                image.insert(oid.to_string(), i);
                has_origin.insert(iid.to_string());
            }
        }
        // Copies of kernel nodes (multi-rules).
        let mut kernel_of: HashMap<String, usize> = HashMap::new();
        if let Some(k) = kernel {
            for m in e.children_named("multiMappings") {
                let (Some((o, _)), Some((i, _))) = (self.reference(m, "origin"), self.reference(m, "image")) else {
                    continue;
                };
                if let (Some(oid), Some(iid)) = (xmi_id(o), xmi_id(i))
                    && let Some(n) = k.by_element.get(oid)
                {
                    kernel_of.insert(iid.to_string(), *n);
                }
            }
        }
        let mut scope = RuleScope::default();
        scope.params.extend(params.iter().map(|p| p.name.text.clone()));
        if let Some(k) = kernel {
            scope.names.extend(k.names.iter().cloned());
        }
        let id = |n: &XmlElement| xmi_id(n).unwrap_or("").to_string();
        // Kernel copies first (they keep their kernel names), then the rule's own nodes.
        let mut infos: Vec<RuleNodeInfo<'x>> = Vec::new();
        for l in &lhs_nodes {
            let r = image.get(&id(l)).copied();
            let copy = kernel_of.get(&id(l)).or_else(|| r.and_then(|r| kernel_of.get(&id(r)))).copied();
            let (class, _) = self.type_name(l);
            let kind = if r.is_some() { Action::Preserve } else { Action::Delete };
            infos.push(RuleNodeInfo { name: String::new(), class, kind, lhs: Some(l), rhs: r, kernel: copy });
        }
        for r in &rhs_nodes {
            if has_origin.contains(&id(r)) {
                continue;
            }
            let (class, _) = self.type_name(r);
            let copy = kernel_of.get(&id(r)).copied();
            infos.push(RuleNodeInfo {
                name: String::new(),
                class,
                kind: Action::Create,
                lhs: None,
                rhs: Some(r),
                kernel: copy,
            });
        }
        for info in &mut infos {
            if let (Some(k), Some(kernel)) = (info.kernel, kernel) {
                info.name = kernel.nodes[k].name.clone();
            }
        }
        for info in &mut infos {
            if info.kernel.is_some() {
                continue;
            }
            let given = text_name(
                info.lhs.and_then(|n| n.attr("name")).or_else(|| info.rhs.and_then(|n| n.attr("name"))).unwrap_or(""),
            );
            info.name = if !given.is_empty() && !scope.names.contains(&given) {
                given
            } else {
                fresh(&lower_first(info.class.last()), &scope.taken())
            };
            scope.names.insert(info.name.clone());
        }
        for (i, info) in infos.iter().enumerate() {
            for n in [info.lhs, info.rhs].into_iter().flatten() {
                scope.by_element.insert(id(n), i);
            }
        }
        // Declarations.
        let mut items = Vec::new();
        for info in &infos {
            let node_key_l = format!("{lhs_key}/node:{}", info.name);
            let node_key_r = format!("{rhs_key}/node:{}", info.name);
            if let Some(l) = info.lhs {
                let k = self.keys.next(node_key_l.clone());
                self.record(&k, l, false);
                self.keep_name(&k, l, &info.name);
            }
            if let Some(r) = info.rhs {
                let k = self.keys.next(node_key_r.clone());
                self.record(&k, r, false);
                self.keep_name(&k, r, &info.name);
            }
            let attributes = match (info.kernel, kernel) {
                (Some(k), Some(kernel)) => {
                    let kn = &kernel.nodes[k];
                    self.attributes(info, &node_key_l, &node_key_r, Some((kn.lhs, kn.rhs)))
                }
                _ => self.attributes(info, &node_key_l, &node_key_r, None),
            };
            let decl_el = info.lhs.or(info.rhs).expect("a node is in one of the sides");
            let name = Name { text: info.name.clone(), span: self.name_span(decl_el) };
            if info.kernel.is_some() {
                if !attributes.is_empty() {
                    items.push(GraphItem::Reuse(ReuseDecl {
                        name,
                        attributes,
                        attributes_span: None,
                        span: whole(decl_el),
                    }));
                }
            } else {
                let action = match info.kind {
                    Action::Preserve => None,
                    other => Some((other, tag_span(decl_el))),
                };
                items.push(GraphItem::Node(NodeDecl {
                    action,
                    name,
                    class: info.class.clone(),
                    attributes,
                    attributes_span: None,
                    span: whole(decl_el),
                }));
            }
        }
        scope.nodes = infos;
        // Edges: preserved ones are in both sides.
        let side_edges = |g: Option<&'x XmlElement>| -> Vec<&'x XmlElement> {
            g.map(|g| g.children_named("edges").collect()).unwrap_or_default()
        };
        let lhs_edges: Vec<EdgeInfo<'x>> =
            side_edges(lhs).into_iter().filter_map(|e| self.edge_info(e, &scope.by_element)).collect();
        let rhs_edges: Vec<EdgeInfo<'x>> =
            side_edges(rhs).into_iter().filter_map(|e| self.edge_info(e, &scope.by_element)).collect();
        for (side_key, list) in [(&lhs_key, &lhs_edges), (&rhs_key, &rhs_edges)] {
            for e in list {
                let k = self.keys.next(format!(
                    "{side_key}/edge:{}->{}:{}",
                    scope.nodes[e.source].name, scope.nodes[e.target].name, e.feature
                ));
                self.record(&k, e.element, false);
            }
        }
        scope.lhs_edges = lhs_edges.iter().map(|e| (e.source, e.feature.clone(), e.target)).collect();
        scope.rhs_edges = rhs_edges.iter().map(|e| (e.source, e.feature.clone(), e.target)).collect();
        // Copies of kernel edges are not part of a multi-rule's text.
        let mut kernel_lhs: Vec<(usize, String, usize)> = kernel.map(|k| k.lhs_edges.clone()).unwrap_or_default();
        let mut kernel_rhs: Vec<(usize, String, usize)> = kernel.map(|k| k.rhs_edges.clone()).unwrap_or_default();
        let is_copy = |e: &EdgeInfo, kernel_edges: &mut Vec<(usize, String, usize)>| {
            let (Some(ks), Some(kt)) = (scope.nodes[e.source].kernel, scope.nodes[e.target].kernel) else {
                return false;
            };
            match kernel_edges.iter().position(|(a, f, b)| *a == ks && *b == kt && *f == e.feature) {
                Some(i) => {
                    kernel_edges.remove(i);
                    true
                }
                None => false,
            }
        };
        let mut rhs_used = vec![false; rhs_edges.len()];
        let mut edges = Vec::new();
        let decl = |e: &EdgeInfo, action: Option<Action>, scope: &RuleScope| EdgeDecl {
            action: action.map(|a| (a, tag_span(e.element))),
            source: Name { text: scope.nodes[e.source].name.clone(), span: e.source_span.clone() },
            target: Name { text: scope.nodes[e.target].name.clone(), span: e.target_span.clone() },
            ty: Self::qname(&e.feature, e.feature_span.clone()),
            span: whole(e.element),
        };
        for le in &lhs_edges {
            let partner = rhs_edges.iter().enumerate().position(|(j, re)| {
                !rhs_used[j]
                    && scope.nodes[le.source].rhs.is_some()
                    && re.source == le.source
                    && re.target == le.target
                    && re.feature == le.feature
            });
            if let Some(j) = partner {
                rhs_used[j] = true;
                if is_copy(&rhs_edges[j], &mut kernel_rhs) {
                    is_copy(le, &mut kernel_lhs);
                    continue;
                }
            }
            if is_copy(le, &mut kernel_lhs) {
                continue;
            }
            edges.push(decl(le, if partner.is_some() { None } else { Some(Action::Delete) }, &scope));
        }
        for (j, re) in rhs_edges.iter().enumerate() {
            if rhs_used[j] || is_copy(re, &mut kernel_rhs) {
                continue;
            }
            edges.push(decl(re, Some(Action::Create), &scope));
        }
        // Mappings.
        for info in &scope.nodes {
            if let (Some(l), Some(_)) = (info.lhs, info.rhs)
                && let Some(m) = e.children_named("mappings").find(|m| m.attr("origin") == xmi_id(l))
            {
                let k = self.keys.next(format!("{key}/mapping:{}", info.name));
                self.record(&k, m, false);
            }
        }
        // The application condition, and the elements it adds to the graph.
        let mut condition_items = Vec::new();
        if let Some(l) = lhs
            && let Some(f) = l.children_named("formula").next()
        {
            let host_key = lhs_key.clone();
            self.lhs_condition(f, &host_key, &mut scope, &mut items, &mut edges, &mut condition_items);
        }
        if !edges.is_empty() {
            let span = edges.first().map_or(0..0, |e| e.span.clone());
            items.push(GraphItem::Edges(EdgesDecl { edges, span }));
        }
        items.extend(condition_items);
        // Multi-rules.
        let dangling = e.attr("checkDangling").map(|v| v != "false").unwrap_or(true);
        let injective = e.attr("injectiveMatching").map(|v| v != "false").unwrap_or(true);
        let mut multi_names = scope.taken();
        for m in e.children_named("multiRules") {
            let given = text_name(m.attr("name").unwrap_or(""));
            let name =
                if given.is_empty() || multi_names.contains(&given) { fresh("multi", &multi_names) } else { given };
            multi_names.insert(name.clone());
            let mkey = format!("{key}/multi:{name}");
            self.record(&mkey, m, false);
            self.keep_name(&mkey, m, &name);
            for p in m.children_named("parameters") {
                let pname = p.attr("name").unwrap_or("");
                self.record(&format!("{mkey}/param:{pname}"), p, false);
            }
            let (multi_items, multi_scope) = self.rule_items(m, &mkey, params, Some((&scope, dangling, injective)));
            // Multi-mappings, by the side of the kernel node.
            for mm in m.children_named("multiMappings") {
                let Some(origin) = mm.attr("origin") else { continue };
                let Some(image_id) = mm.attr("image") else { continue };
                let Some(&k) = scope.by_element.get(origin) else { continue };
                let side = if scope.nodes[k].lhs.and_then(xmi_id) == Some(origin) { "lhs" } else { "rhs" };
                if multi_scope.by_element.contains_key(image_id) {
                    let mk = self.keys.next(format!("{mkey}/multiMapping:{side}:{}", scope.nodes[k].name));
                    self.record(&mk, mm, false);
                }
            }
            let span = whole(m);
            items.push(GraphItem::MultiRule(MultiRuleDecl {
                name: Name { text: name, span: self.name_span(m) },
                items: multi_items,
                body: span.clone(),
                span,
            }));
        }
        (items, scope)
    }

    fn edge_info(&mut self, e: &'x XmlElement, nodes: &HashMap<String, usize>) -> Option<EdgeInfo<'x>> {
        let (src, source_span) = self.reference(e, "source")?;
        let (tgt, target_span) = self.reference(e, "target")?;
        let (Some(source), Some(target)) =
            (xmi_id(src).and_then(|i| nodes.get(i)), xmi_id(tgt).and_then(|i| nodes.get(i)))
        else {
            self.error(whole(e), "the edge connects nodes of another graph");
            return None;
        };
        let (feature, feature_span, _) = self.feature(e);
        Some(EdgeInfo { element: e, source: *source, target: *target, feature, source_span, target_span, feature_span })
    }

    /// The attributes of a node as declarations: matched, set, deleted. For copies of kernel
    /// nodes, the kernel's attributes are left out.
    fn attributes(
        &mut self,
        info: &RuleNodeInfo<'x>,
        lhs_key: &str,
        rhs_key: &str,
        kernel: Option<(Option<&'x XmlElement>, Option<&'x XmlElement>)>,
    ) -> Vec<AttributeDecl> {
        let attrs = |n: Option<&'x XmlElement>| -> Vec<&'x XmlElement> {
            n.map(|n| n.children_named("attributes").collect()).unwrap_or_default()
        };
        let value_text = |a: &XmlElement| a.attr("value").unwrap_or("").trim().to_string();
        let mut lhs = attrs(info.lhs);
        let mut rhs = attrs(info.rhs);
        let feature_of = |s: &Self, a: &XmlElement| s.feature(a).0;
        if let Some((kl, kr)) = kernel {
            let same = |s: &Self, a: &XmlElement, k: &[&XmlElement]| {
                k.iter().any(|b| feature_of(s, a) == feature_of(s, b) && value_text(a) == value_text(b))
            };
            let kl = attrs(kl);
            let kr = attrs(kr);
            lhs.retain(|a| !same(self, a, &kl));
            rhs.retain(|a| !same(self, a, &kr));
        }
        for (side_key, list) in [(lhs_key, &lhs), (rhs_key, &rhs)] {
            for a in list {
                let k = self.keys.next(format!("{side_key}/attr:{}", feature_of(self, a)));
                self.record(&k, a, false);
            }
        }
        let mut out = Vec::new();
        let decl = |s: &mut Self, a: &XmlElement, action: Option<Action>, set: bool| {
            let (name, span, f) = s.feature(a);
            let value = s.value(a, "value", f);
            AttributeDecl {
                action: action.map(|x| (x, tag_span(a))),
                set,
                name: Self::qname(&name, span),
                value,
                span: whole(a),
            }
        };
        match info.kind {
            Action::Create => out.extend(rhs.iter().map(|a| decl(self, a, None, false))),
            Action::Delete => out.extend(lhs.iter().map(|a| decl(self, a, None, false))),
            _ => {
                let mut rhs_used = vec![false; rhs.len()];
                let mut sets = Vec::new();
                for a in &lhs {
                    let f = feature_of(self, a);
                    match rhs.iter().enumerate().find(|(j, b)| !rhs_used[*j] && feature_of(self, b) == f) {
                        Some((j, b)) => {
                            rhs_used[j] = true;
                            out.push(decl(self, a, None, false));
                            if value_text(a) != value_text(b) {
                                sets.push(decl(self, b, None, true));
                            }
                        }
                        None => out.push(decl(self, a, Some(Action::Delete), false)),
                    }
                }
                for (j, b) in rhs.iter().enumerate() {
                    if !rhs_used[j] {
                        sets.push(decl(self, b, None, true));
                    }
                }
                out.extend(sets);
            }
        }
        out
    }

    // ------------------------------------------------------------------ conditions

    /// The left-hand side's formula: the negative condition `forbid` and the positive one
    /// `require` become elements of the graph with these actions, the rest a matching formula.
    fn lhs_condition(
        &mut self,
        formula: &'x XmlElement,
        host_key: &str,
        scope: &mut RuleScope<'x>,
        items: &mut Vec<GraphItem>,
        edges: &mut Vec<EdgeDecl>,
        condition_items: &mut Vec<GraphItem>,
    ) {
        let mut terms = Vec::new();
        conjuncts(formula, &format!("{host_key}/formula"), &mut terms);
        // The nested condition of a term that can be written with actions: `forbid` for a
        // negated one, `require` for a positive one, without a formula of its own.
        let simple = |term: &'x XmlElement| -> Option<(Action, &'x XmlElement)> {
            match xsi_type(term) {
                Some("Not") => term
                    .children_named("child")
                    .next()
                    .filter(|c| xsi_type(c) == Some("NestedCondition") && !has_formula(c))
                    .map(|c| (Action::Forbid, c)),
                Some("NestedCondition") if !has_formula(term) => Some((Action::Require, term)),
                _ => None,
            }
        };
        // Written by this writer the conditions are named `forbid` and `require`; Henshin's
        // editors leave them unnamed (or call them `default`).
        let named = |action: Action| {
            terms
                .iter()
                .any(|(t, _)| simple(t).is_some_and(|(a, nc)| a == action && conclusion_name(nc) == action.keyword()))
        };
        let (forbid_named, require_named) = (named(Action::Forbid), named(Action::Require));
        let mut rest: Vec<(&'x XmlElement, String)> = Vec::new();
        let (mut forbid, mut require) = (false, false);
        for (term, path) in terms.iter().cloned() {
            if xsi_type(term) == Some("True") {
                continue;
            }
            let Some((action, nc)) = simple(term) else {
                rest.push((term, path));
                continue;
            };
            let (done, by_name) =
                if action == Action::Forbid { (&mut forbid, forbid_named) } else { (&mut require, require_named) };
            let name = conclusion_name(nc);
            let fits = if by_name { name == action.keyword() } else { name.is_empty() || name == "default" };
            if *done || !fits {
                rest.push((term, path));
                continue;
            }
            *done = true;
            if action == Action::Forbid {
                self.record(&path, term, false);
            }
            self.action_condition(nc, host_key, action, scope, items, edges);
        }
        if rest.is_empty() {
            return;
        }
        let outer: Vec<(String, String)> = scope
            .nodes
            .iter()
            .filter(|n| n.lhs.is_some())
            .filter_map(|n| Some((xmi_id(n.lhs?)?.to_string(), n.name.clone())))
            .collect();
        let decl = self.formula_decl(&rest, host_key, &outer, &scope.taken());
        condition_items.push(GraphItem::Formula(decl));
    }

    /// A negative (`forbid`) or positive (`require`) condition as elements of the rule's graph.
    fn action_condition(
        &mut self,
        nc: &'x XmlElement,
        host_key: &str,
        action: Action,
        scope: &mut RuleScope<'x>,
        items: &mut Vec<GraphItem>,
        edges: &mut Vec<EdgeDecl>,
    ) {
        let name = action.keyword();
        let key = self.keys.next(format!("{host_key}/nc:{name}"));
        self.record(&key, nc, false);
        let Some(conclusion) = nc.children_named("conclusion").next() else { return };
        let conclusion_key = format!("{key}/conclusion");
        self.record(&conclusion_key, conclusion, false);
        self.keep_name(&conclusion_key, conclusion, name);
        // Copies of the rule's nodes.
        let mut copy_of: HashMap<String, usize> = HashMap::new();
        for m in nc.children_named("mappings") {
            let (Some((o, _)), Some((i, _))) = (self.reference(m, "origin"), self.reference(m, "image")) else {
                continue;
            };
            if let (Some(oid), Some(iid)) = (xmi_id(o), xmi_id(i))
                && let Some(n) = scope.by_element.get(oid)
            {
                copy_of.insert(iid.to_string(), *n);
                let mk = self.keys.next(format!("{key}/mapping:{}", scope.nodes[*n].name));
                self.record(&mk, m, false);
            }
        }
        let mut names: HashMap<String, String> = HashMap::new();
        for n in conclusion.children_named("nodes") {
            let id = xmi_id(n).unwrap_or("").to_string();
            match copy_of.get(&id) {
                Some(&outer) => {
                    let outer_name = scope.nodes[outer].name.clone();
                    let nk = self.keys.next(format!("{conclusion_key}/node:{outer_name}"));
                    self.record(&nk, n, false);
                    self.keep_name(&nk, n, &outer_name);
                    names.insert(id, outer_name.clone());
                    // Attributes the copy adds to the node: `forbid name=value`.
                    let origin_attrs: Vec<(String, String)> = scope.nodes[outer]
                        .lhs
                        .map(|l| {
                            l.children_named("attributes")
                                .map(|a| (self.feature(a).0, a.attr("value").unwrap_or("").trim().to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                    let mut added = Vec::new();
                    for a in n.children_named("attributes") {
                        let (fname, fspan, f) = self.feature(a);
                        let ak = self.keys.next(format!("{nk}/attr:{fname}"));
                        self.record(&ak, a, false);
                        if origin_attrs.contains(&(fname.clone(), a.attr("value").unwrap_or("").trim().to_string())) {
                            continue;
                        }
                        let value = self.value(a, "value", f);
                        added.push(AttributeDecl {
                            action: Some((action, tag_span(a))),
                            set: false,
                            name: Self::qname(&fname, fspan),
                            value,
                            span: whole(a),
                        });
                    }
                    if !added.is_empty() {
                        let node_name = scope.nodes[outer].name.clone();
                        add_attributes(items, &node_name, added, n);
                    }
                }
                None => {
                    let given = text_name(n.attr("name").unwrap_or(""));
                    let (class, _) = self.type_name(n);
                    let node_name = if !given.is_empty() && !scope.names.contains(&given) {
                        given
                    } else {
                        fresh(&lower_first(class.last()), &scope.taken())
                    };
                    scope.names.insert(node_name.clone());
                    let nk = self.keys.next(format!("{conclusion_key}/node:{node_name}"));
                    self.record(&nk, n, false);
                    self.keep_name(&nk, n, &node_name);
                    names.insert(id, node_name.clone());
                    let attributes = n
                        .children_named("attributes")
                        .map(|a| {
                            let (fname, fspan, f) = self.feature(a);
                            let ak = self.keys.next(format!("{nk}/attr:{fname}"));
                            self.record(&ak, a, false);
                            let value = self.value(a, "value", f);
                            AttributeDecl {
                                action: None,
                                set: false,
                                name: Self::qname(&fname, fspan),
                                value,
                                span: whole(a),
                            }
                        })
                        .collect();
                    items.push(GraphItem::Node(NodeDecl {
                        action: Some((action, tag_span(n))),
                        name: Name { text: node_name, span: self.name_span(n) },
                        class,
                        attributes,
                        attributes_span: None,
                        span: whole(n),
                    }));
                }
            }
        }
        for edge in conclusion.children_named("edges") {
            let Some((s, sspan)) = self.reference(edge, "source") else { continue };
            let Some((t, tspan)) = self.reference(edge, "target") else { continue };
            let (Some(sn), Some(tn)) = (xmi_id(s).and_then(|i| names.get(i)), xmi_id(t).and_then(|i| names.get(i)))
            else {
                continue;
            };
            let (feature, fspan, _) = self.feature(edge);
            let ek = self.keys.next(format!("{conclusion_key}/edge:{sn}->{tn}:{feature}"));
            self.record(&ek, edge, false);
            edges.push(EdgeDecl {
                action: Some((action, tag_span(edge))),
                source: Name { text: sn.clone(), span: sspan },
                target: Name { text: tn.clone(), span: tspan },
                ty: Self::qname(&feature, fspan),
                span: whole(edge),
            });
        }
    }

    /// A matching formula of terms (formulas over nested conditions, conjoined). `outer` are the
    /// host graph's nodes (XML id, name); `taken` the names in use.
    fn formula_decl(
        &mut self,
        terms: &[(&'x XmlElement, String)],
        host_key: &str,
        outer: &[(String, String)],
        taken: &HashSet<String>,
    ) -> FormulaDecl {
        let mut graphs = Vec::new();
        let mut graph_names: HashSet<String> = HashSet::new();
        let mut logic: Option<Logic> = None;
        let span = terms.first().map_or(0..0, |(t, _)| whole(t));
        for (term, path) in terms {
            let l = self.logic(term, path, host_key, outer, taken, &mut graphs, &mut graph_names);
            logic = Some(match logic {
                None => l,
                Some(prev) => Logic::And(Box::new(prev), Box::new(l)),
            });
        }
        FormulaDecl {
            formula: logic.unwrap_or(Logic::Error),
            formula_span: span.clone(),
            graphs,
            body: span.clone(),
            span,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn logic(
        &mut self,
        f: &'x XmlElement,
        path: &str,
        host_key: &str,
        outer: &[(String, String)],
        taken: &HashSet<String>,
        graphs: &mut Vec<ConditionGraphDecl>,
        graph_names: &mut HashSet<String>,
    ) -> Logic {
        let span = tag_span(f);
        match xsi_type(f) {
            Some("Not") => {
                self.record(path, f, false);
                match f.children_named("child").next() {
                    Some(c) => Logic::Not(Box::new(self.logic(
                        c,
                        &format!("{path}/child"),
                        host_key,
                        outer,
                        taken,
                        graphs,
                        graph_names,
                    ))),
                    None => Logic::Error,
                }
            }
            Some(t @ ("And" | "Or" | "Xor")) => {
                self.record(path, f, false);
                let left = f.children_named("left").next();
                let right = f.children_named("right").next();
                let (Some(l), Some(r)) = (left, right) else { return Logic::Error };
                let l = Box::new(self.logic(l, &format!("{path}/left"), host_key, outer, taken, graphs, graph_names));
                let r = Box::new(self.logic(r, &format!("{path}/right"), host_key, outer, taken, graphs, graph_names));
                match t {
                    "And" => Logic::And(l, r),
                    "Or" => Logic::Or(l, r),
                    _ => Logic::Xor(l, r),
                }
            }
            Some("NestedCondition") => {
                let given = text_name(conclusion_name(f));
                let name = if given.is_empty() || graph_names.contains(&given) || is_keyword(&given) {
                    fresh("condition", graph_names)
                } else {
                    given
                };
                graph_names.insert(name.clone());
                let decl = self.condition_graph(f, &name, host_key, outer, taken);
                graphs.push(decl);
                Logic::Ref(Name { text: name, span })
            }
            Some("True") => {
                self.warning(span.clone(), "'true' cannot be written in a matching formula; it is left out");
                Logic::Error
            }
            other => {
                self.error(span, format!("unknown formula '{}'", other.unwrap_or("?")));
                Logic::Error
            }
        }
    }

    /// A nested condition as a condition graph: its own nodes, copies of the host's nodes with
    /// further attributes as `reuse`, its edges and its own formula.
    fn condition_graph(
        &mut self,
        nc: &'x XmlElement,
        name: &str,
        host_key: &str,
        outer: &[(String, String)],
        taken: &HashSet<String>,
    ) -> ConditionGraphDecl {
        let key = self.keys.next(format!("{host_key}/nc:{name}"));
        self.record(&key, nc, false);
        let mut items = Vec::new();
        let Some(conclusion) = nc.children_named("conclusion").next() else {
            return ConditionGraphDecl {
                name: Name { text: name.into(), span: tag_span(nc) },
                items,
                body: whole(nc),
                span: whole(nc),
            };
        };
        let conclusion_key = format!("{key}/conclusion");
        self.record(&conclusion_key, conclusion, false);
        self.keep_name(&conclusion_key, conclusion, name);
        let outer_names: HashMap<&str, &str> = outer.iter().map(|(i, n)| (i.as_str(), n.as_str())).collect();
        let mut copy_of: HashMap<String, String> = HashMap::new();
        for m in nc.children_named("mappings") {
            let (Some(o), Some(i)) = (m.attr("origin"), m.attr("image")) else { continue };
            match outer_names.get(o) {
                Some(n) => {
                    copy_of.insert(i.to_string(), n.to_string());
                    let mk = self.keys.next(format!("{key}/mapping:{n}"));
                    self.record(&mk, m, false);
                }
                None => {
                    let span = attr_span(self.text, m, "origin").unwrap_or_else(|| tag_span(m));
                    self.error(span, "the origin of this mapping is not a node of the enclosing graph");
                }
            }
        }
        let mut names: HashMap<String, String> = HashMap::new();
        let mut scope: HashSet<String> = taken.clone();
        let mut inner_outer: Vec<(String, String)> = Vec::new();
        for n in conclusion.children_named("nodes") {
            let id = xmi_id(n).unwrap_or("").to_string();
            let attrs: Vec<&XmlElement> = n.children_named("attributes").collect();
            let (node_name, own) = match copy_of.get(&id) {
                Some(outer_name) => (outer_name.clone(), false),
                None => {
                    let given = text_name(n.attr("name").unwrap_or(""));
                    let (class, _) = self.type_name(n);
                    let nm = if !given.is_empty() && !scope.contains(&given) {
                        given
                    } else {
                        fresh(&lower_first(class.last()), &scope)
                    };
                    (nm, true)
                }
            };
            scope.insert(node_name.clone());
            names.insert(id.clone(), node_name.clone());
            inner_outer.push((id, node_name.clone()));
            let nk = self.keys.next(format!("{conclusion_key}/node:{node_name}"));
            self.record(&nk, n, false);
            self.keep_name(&nk, n, &node_name);
            let attributes: Vec<AttributeDecl> = attrs
                .iter()
                .map(|a| {
                    let (fname, fspan, f) = self.feature(a);
                    let ak = self.keys.next(format!("{nk}/attr:{fname}"));
                    self.record(&ak, a, false);
                    let value = self.value(a, "value", f);
                    AttributeDecl { action: None, set: false, name: Self::qname(&fname, fspan), value, span: whole(a) }
                })
                .collect();
            if own {
                let (class, _) = self.type_name(n);
                items.push(GraphItem::Node(NodeDecl {
                    action: None,
                    name: Name { text: node_name, span: self.name_span(n) },
                    class,
                    attributes,
                    attributes_span: None,
                    span: whole(n),
                }));
            } else if !attributes.is_empty() {
                items.push(GraphItem::Reuse(ReuseDecl {
                    name: Name { text: node_name, span: self.name_span(n) },
                    attributes,
                    attributes_span: None,
                    span: whole(n),
                }));
            }
        }
        let mut edges = Vec::new();
        for edge in conclusion.children_named("edges") {
            let (Some(s), Some(t)) = (edge.attr("source"), edge.attr("target")) else { continue };
            let (Some(sn), Some(tn)) = (names.get(s), names.get(t)) else {
                self.error(whole(edge), "the edge connects nodes of another graph");
                continue;
            };
            let (feature, fspan, _) = self.feature(edge);
            let ek = self.keys.next(format!("{conclusion_key}/edge:{sn}->{tn}:{feature}"));
            self.record(&ek, edge, false);
            edges.push(EdgeDecl {
                action: None,
                source: Name {
                    text: sn.clone(),
                    span: attr_span(self.text, edge, "source").unwrap_or_else(|| whole(edge)),
                },
                target: Name {
                    text: tn.clone(),
                    span: attr_span(self.text, edge, "target").unwrap_or_else(|| whole(edge)),
                },
                ty: Self::qname(&feature, fspan),
                span: whole(edge),
            });
        }
        if !edges.is_empty() {
            let span = edges[0].span.clone();
            items.push(GraphItem::Edges(EdgesDecl { edges, span }));
        }
        if let Some(f) = conclusion.children_named("formula").next() {
            let mut terms = Vec::new();
            conjuncts(f, &format!("{conclusion_key}/formula"), &mut terms);
            let terms: Vec<_> = terms.into_iter().filter(|(t, _)| xsi_type(t) != Some("True")).collect();
            if !terms.is_empty() {
                let decl = self.formula_decl(&terms, &conclusion_key, &inner_outer, &scope);
                items.push(GraphItem::Formula(decl));
            }
        }
        ConditionGraphDecl {
            name: Name {
                text: name.to_string(),
                span: attr_span(self.text, conclusion, "name").unwrap_or_else(|| tag_span(nc)),
            },
            items,
            body: whole(nc),
            span: whole(nc),
        }
    }

    // ------------------------------------------------------------------ units

    fn unit(&mut self, e: &'x XmlElement, name: String) -> UnitDecl {
        let key = unit_key(&name);
        self.record(&key, e, false);
        self.keep_name(&key, e, &name);
        let mut params = self.params(e, &key);
        let items = self.unit_body(e);
        for u in xmi_id(e).and_then(|id| self.unbound.get(id)).into_iter().flatten() {
            let span = tag_span(e);
            params.push(ParamDecl {
                kind: ParamKind::Var,
                name: Name { text: u.name.clone(), span: span.clone() },
                ty: u.ty.clone(),
                span,
            });
        }
        UnitDecl {
            name: Name { text: name, span: self.name_span(e) },
            params,
            params_span: tag_span(e),
            items,
            body: whole(e),
            span: whole(e),
        }
    }

    /// The elements of a unit.
    fn unit_body(&mut self, e: &'x XmlElement) -> Vec<UnitItem> {
        let span = tag_span(e);
        let name = xmi_id(e).and_then(|id| self.unit_names.get(id).cloned()).unwrap_or_default();
        let key = unit_key(&name);
        if let Some(a) = e.children_named("annotations").find(|a| a.attr("key") == Some(NESTED)) {
            self.record(&format!("{key}/annotation:nested"), a, false);
        }
        for m in e.children_named("parameterMappings") {
            let describe = |s: &Self, attr: &str| -> Option<String> {
                let p = s.resolve(m.attr(attr)?)?;
                let unit = s.parent_unit(p)?;
                Some(format!("{unit}.{}", p.attr("name").unwrap_or("")))
            };
            if let (Some(src), Some(tgt)) = (describe(self, "source"), describe(self, "target")) {
                let k = self.keys.next(format!("{key}/mapping:{src}->{tgt}"));
                self.record(&k, m, false);
            }
        }
        let mut items = Vec::new();
        let flag = |attr: &str, default: bool| e.attr(attr).map_or(default, |v| v != "false");
        match xsi_type(e) {
            Some("SequentialUnit") => {
                if !flag("strict", true) {
                    items.push(UnitItem::Strict(false, attr_span(self.text, e, "strict").unwrap_or(span.clone())));
                }
                if !flag("rollback", true) {
                    items.push(UnitItem::Rollback(false, attr_span(self.text, e, "rollback").unwrap_or(span.clone())));
                }
                for (r, rspan) in self.references(e, "subUnits") {
                    self.item(e, &r, rspan, &mut items);
                }
            }
            Some(t @ ("IndependentUnit" | "PriorityUnit")) => {
                let lists: Vec<Vec<UnitItem>> =
                    self.references(e, "subUnits").into_iter().map(|(r, rspan)| self.list(e, &r, rspan)).collect();
                items.push(if t == "IndependentUnit" {
                    UnitItem::Independent(lists, span)
                } else {
                    UnitItem::Priority(lists, span)
                });
            }
            Some("ConditionalUnit") => {
                let part = |s: &mut Self, attr: &str| -> Option<Vec<UnitItem>> {
                    let (r, rspan) = s.references(e, attr).into_iter().next()?;
                    Some(s.list(e, &r, rspan))
                };
                let condition = part(self, "if").unwrap_or_default();
                let then = part(self, "then").unwrap_or_default();
                let otherwise = part(self, "else");
                items.push(UnitItem::If { condition, then, otherwise, span });
            }
            Some("IteratedUnit") => {
                if flag("strict", false) {
                    self.warning(
                        span.clone(),
                        "strict iterated units cannot be written in the textual syntax: 'strict' is left out",
                    );
                }
                let count = self.value(e, "iterations", None);
                let body = match self.references(e, "subUnit").into_iter().next() {
                    Some((r, rspan)) => self.list(e, &r, rspan),
                    None => Vec::new(),
                };
                items.push(UnitItem::For { count, body, span });
            }
            Some("LoopUnit") => {
                let body = match self.references(e, "subUnit").into_iter().next() {
                    Some((r, rspan)) => self.list(e, &r, rspan),
                    None => Vec::new(),
                };
                items.push(UnitItem::While { body, span });
            }
            other => {
                self.error(span, format!("unknown kind of unit '{}'", other.unwrap_or("?")));
            }
        }
        items
    }

    /// The name of the unit or rule a parameter belongs to.
    fn parent_unit(&self, param: &XmlElement) -> Option<String> {
        fn find<'e>(e: &'e XmlElement, target: &XmlElement, unit: Option<&'e XmlElement>) -> Option<&'e XmlElement> {
            let unit = if matches!(e.local_name(), "units" | "multiRules") { Some(e) } else { unit };
            if std::ptr::eq(e, target) {
                return unit;
            }
            e.children.iter().find_map(|c| find(c, target, unit))
        }
        let u = find(self.root, param, None)?;
        if u.local_name() == "multiRules" {
            return None;
        }
        self.unit_names.get(xmi_id(u)?).cloned()
    }

    /// The elements of a branch (of a choice, a condition or a loop): the elements of the unit
    /// made for it, or the one element.
    fn list(&mut self, owner: &'x XmlElement, reference: &str, span: Range<usize>) -> Vec<UnitItem> {
        if let Some(target) = self.resolve(reference)
            && let Some(role) = xmi_id(target).and_then(|id| self.nested.get(id)).cloned()
            && role != "block"
            && xsi_type(target) == Some("SequentialUnit")
        {
            return self.unit_body(target);
        }
        let mut out = Vec::new();
        self.item(owner, reference, span, &mut out);
        out
    }

    /// An element of a sequence: a call, or the elements of a unit made for a nested block.
    fn item(&mut self, owner: &'x XmlElement, reference: &str, span: Range<usize>, out: &mut Vec<UnitItem>) {
        let Some(target) = self.resolve(reference) else {
            let what = if reference.contains('#') { "units of other files are not supported" } else { "no such unit" };
            self.error(span, format!("cannot resolve '{reference}': {what}"));
            return;
        };
        let id = xmi_id(target).unwrap_or("").to_string();
        if let Some(role) = self.nested.get(&id).cloned() {
            let body = self.unit_body(target);
            if role == "block" {
                out.push(UnitItem::Sequence(body, whole(target)));
            } else {
                out.extend(body);
            }
            return;
        }
        let Some(name) = self.unit_names.get(&id).cloned() else {
            self.error(span, format!("'{reference}' is not a rule or unit"));
            return;
        };
        let args = self.arguments(owner, target, &span);
        out.push(UnitItem::Call { target: Name { text: name, span: span.clone() }, args, span });
    }

    /// The arguments of a call: for every parameter of the called unit, the parameter of the
    /// calling unit mapped to (or from) it, else the variable [`find_unbound`](Self::find_unbound)
    /// made for it.
    fn arguments(&self, owner: &XmlElement, callee: &XmlElement, span: &Range<usize>) -> Vec<Name> {
        let owner_id = xmi_id(owner).unwrap_or("");
        let callee_id = xmi_id(callee).unwrap_or("");
        let variable = |param: &str| {
            self.unbound
                .get(owner_id)
                .and_then(|list| list.iter().find(|u| u.callee == callee_id && u.param == param))
                .map_or_else(|| param.to_string(), |u| u.name.clone())
        };
        let mut args = Vec::new();
        for p in callee.children_named("parameters") {
            let Some(pid) = xmi_id(p) else { continue };
            let name = mapped(owner, pid).unwrap_or_else(|| variable(&text_name(p.attr("name").unwrap_or(""))));
            args.push(Name { text: name, span: span.clone() });
        }
        for _ in self.unbound.get(callee_id).into_iter().flatten() {
            args.push(Name { text: variable(INHERITED), span: span.clone() });
        }
        args
    }

    /// Finds the parameters of called units that their callers map nothing to, and gives the
    /// callers variables for them (until none are missing: a caller's new variable is missing
    /// in its own callers).
    fn find_unbound(&mut self, units: &[&'x XmlElement]) {
        let units: Vec<&'x XmlElement> = units
            .iter()
            .copied()
            .filter(|u| xsi_type(u) != Some("Rule") && !xmi_id(u).is_some_and(|id| self.nested.contains_key(id)))
            .collect();
        loop {
            let mut changed = false;
            for u in &units {
                let Some(uid) = xmi_id(u) else { continue };
                for attr in ["subUnits", "subUnit", "if", "then", "else"] {
                    for (r, _) in self.references(u, attr) {
                        let Some(c) = self.resolve(&r) else { continue };
                        let Some(cid) = xmi_id(c) else { continue };
                        if self.nested.contains_key(cid) {
                            continue;
                        }
                        let mut needs: Vec<(String, QName)> = Vec::new();
                        for p in c.children_named("parameters") {
                            if xmi_id(p).is_some_and(|pid| mapped(u, pid).is_none()) {
                                let ty = if self.type_href(p).is_some() {
                                    self.type_name(p).0
                                } else {
                                    QName { parts: Vec::new(), span: 0..0 }
                                };
                                needs.push((text_name(p.attr("name").unwrap_or("")), ty));
                            }
                        }
                        // The callee's own variables get one variable of the caller (so that
                        // recursion ends).
                        if let Some(first) = self.unbound.get(cid).and_then(|l| l.first()) {
                            needs.push((INHERITED.to_string(), first.ty.clone()));
                        }
                        for (param, ty) in needs {
                            let list = self.unbound.entry(uid.to_string()).or_default();
                            if list.iter().any(|x| x.callee == cid && x.param == param) {
                                continue;
                            }
                            let mut taken: HashSet<String> = u
                                .children_named("parameters")
                                .map(|p| text_name(p.attr("name").unwrap_or("")))
                                .collect();
                            taken.extend(list.iter().map(|x| x.name.clone()));
                            let name = fresh(if param == INHERITED { "unbound" } else { &param }, &taken);
                            list.push(Unbound { callee: cid.to_string(), param, name, ty });
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }
}

/// The parameter of `unit` mapped to or from the parameter `param` (of a unit it calls).
fn mapped(unit: &XmlElement, param: &str) -> Option<String> {
    let own: Vec<(&str, &str)> =
        unit.children_named("parameters").filter_map(|p| Some((xmi_id(p)?, p.attr("name")?))).collect();
    let name_of = |id: &str| own.iter().find(|(i, _)| *i == id).map(|(_, n)| text_name(n));
    let mappings: Vec<(&str, &str)> =
        unit.children_named("parameterMappings").filter_map(|m| Some((m.attr("source")?, m.attr("target")?))).collect();
    mappings
        .iter()
        .find(|(s, t)| *t == param && name_of(s).is_some())
        .and_then(|(s, _)| name_of(s))
        .or_else(|| mappings.iter().find(|(s, t)| *s == param && name_of(t).is_some()).and_then(|(_, t)| name_of(t)))
}

/// The terms of a conjunction (with their key paths), nested `And`s flattened.
fn conjuncts<'x>(f: &'x XmlElement, path: &str, out: &mut Vec<(&'x XmlElement, String)>) {
    if xsi_type(f) == Some("And")
        && let (Some(l), Some(r)) = (f.children_named("left").next(), f.children_named("right").next())
    {
        conjuncts(l, &format!("{path}/left"), out);
        conjuncts(r, &format!("{path}/right"), out);
        return;
    }
    out.push((f, path.to_string()));
}

fn conclusion_name(nc: &XmlElement) -> &str {
    nc.children_named("conclusion").next().and_then(|c| c.attr("name")).unwrap_or("")
}

fn has_formula(nc: &XmlElement) -> bool {
    nc.children_named("conclusion").next().is_some_and(|c| c.children_named("formula").next().is_some())
}

/// Adds attributes to the declaration of a node (or a `reuse` of it) among `items`.
fn add_attributes(items: &mut Vec<GraphItem>, node: &str, attributes: Vec<AttributeDecl>, at: &XmlElement) {
    for item in items.iter_mut() {
        match item {
            GraphItem::Node(n) if n.name.text == node => {
                n.attributes.extend(attributes);
                return;
            }
            GraphItem::Reuse(r) if r.name.text == node => {
                r.attributes.extend(attributes);
                return;
            }
            _ => {}
        }
    }
    // A kernel node without a `reuse` yet.
    items.push(GraphItem::Reuse(ReuseDecl {
        name: Name { text: node.to_string(), span: tag_span(at) },
        attributes,
        attributes_span: None,
        span: whole(at),
    }));
}

/// A name of the textual syntax for a name of the model: characters that cannot be part of a
/// name become `_` (`a-b` → `a_b`, `2nd` → `_2nd`).
fn text_name(name: &str) -> String {
    let mut out: String = name.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if out.chars().next().is_some_and(|c| !(c.is_alphabetic() || c == '_')) {
        out.insert(0, '_');
    }
    out
}

/// `Account` → `account`.
fn lower_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_lowercase().collect::<String>() + chars.as_str(),
        None => "node".into(),
    }
}

/// `base`, else `base2`, `base3`, … : the first that is free and not a keyword.
fn fresh(base: &str, taken: &HashSet<String>) -> String {
    let base = if base.is_empty() { "x" } else { base };
    if !taken.contains(base) && !is_keyword(base) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base}{i}")).find(|n| !taken.contains(n)).unwrap_or_default()
}

/// A path without `.` and `..` components.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}
