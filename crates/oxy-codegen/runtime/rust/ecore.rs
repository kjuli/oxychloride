//! The reflective half of a generated model: metadata of packages, classes and features,
//! consistent links (containment and opposites), deletion, XMI reading and writing, and
//! multiplicity checks. It works on any [`Store`], which the generated `Model` implements.
//!
//! Copied verbatim into every generated crate; do not edit.

#![allow(dead_code)]

use std::fmt;

use crate::ocl::ObjectId;
use crate::xml;

pub const XMI_NS: &str = "http://www.omg.org/XMI";
pub const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";

/// An `EPackage`.
#[derive(Debug)]
pub struct PackageInfo {
    pub name: &'static str,
    pub ns_uri: &'static str,
    pub ns_prefix: &'static str,
}

/// An `EClass`.
#[derive(Debug)]
pub struct ClassInfo {
    pub name: &'static str,
    /// Index into the packages.
    pub package: usize,
    /// Abstract classes and interfaces cannot be instantiated.
    pub is_abstract: bool,
    /// All supertypes (indices into the classes).
    pub supers: &'static [usize],
    /// All stored features including inherited ones (indices into the features), in EMF's
    /// order.
    pub features: &'static [usize],
}

/// An `EEnum`: the names and serialized literals of its literals.
#[derive(Debug)]
pub struct EnumInfo {
    pub name: &'static str,
    pub names: &'static [&'static str],
    pub literals: &'static [&'static str],
}

/// The kind of the values of a feature.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ValueKind {
    Boolean,
    Integer,
    Real,
    String,
    /// An enumeration (index into the enumerations).
    Enum(usize),
    /// Objects of a class (index into the classes), or of any class.
    Object(Option<usize>),
}

/// A stored structural feature.
#[derive(Debug)]
pub struct FeatureInfo {
    pub name: &'static str,
    /// The class declaring it.
    pub owner: usize,
    pub kind: ValueKind,
    pub many: bool,
    pub lower: i64,
    /// -1 for unbounded.
    pub upper: i64,
    pub unique: bool,
    pub containment: bool,
    /// The opposite of a containment: the object's container (not stored).
    pub container: bool,
    pub opposite: Option<usize>,
    pub serialized: bool,
    pub id: bool,
    /// The serialized default value of a single-valued attribute.
    pub default: Option<&'static str>,
}

impl FeatureInfo {
    pub fn is_reference(&self) -> bool {
        matches!(self.kind, ValueKind::Object(_))
    }
}

/// A value of an attribute, for reflection.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Boolean(bool),
    Integer(i64),
    Real(f64),
    String(String),
    /// A literal of an enumeration, by index.
    Enum(usize),
}

impl Value {
    pub fn as_boolean(&self) -> Option<bool> {
        match self {
            Value::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Value::Integer(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_real(&self) -> Option<f64> {
        match self {
            Value::Real(r) => Some(*r),
            Value::Integer(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub fn as_string(&self) -> Option<String> {
        match self {
            Value::String(s) => Some(s.clone()),
            _ => None,
        }
    }

    pub fn as_enum(&self) -> Option<usize> {
        match self {
            Value::Enum(i) => Some(*i),
            _ => None,
        }
    }
}

/// Inserts `item` into `items` at `index` (or at the end), unless `unique` and present.
pub fn insert<T: PartialEq>(items: &mut Vec<T>, item: T, index: Option<usize>, unique: bool) {
    if unique && items.contains(&item) {
        return;
    }
    match index {
        Some(i) if i <= items.len() => items.insert(i, item),
        _ => items.push(item),
    }
}

/// The storage of a model, implemented by the generated `Model`.
pub trait Store: Default {
    fn packages(&self) -> &'static [PackageInfo];
    fn classes(&self) -> &'static [ClassInfo];
    fn features(&self) -> &'static [FeatureInfo];
    fn enums(&self) -> &'static [EnumInfo];

    /// Creates an object of a concrete class (with default values, without a container).
    fn create_object(&mut self, class: usize) -> ObjectId;
    /// Removes an object (that nothing refers to any more).
    fn remove_object(&mut self, o: ObjectId);
    /// The live objects, in creation order.
    fn object_ids(&self) -> Vec<ObjectId>;
    fn contains_object(&self, o: ObjectId) -> bool;
    fn class_index(&self, o: ObjectId) -> usize;
    /// The container of an object and the containment feature holding it.
    fn container(&self, o: ObjectId) -> Option<(ObjectId, usize)>;
    fn set_container(&mut self, o: ObjectId, container: Option<(ObjectId, usize)>);

    /// The targets of a stored reference.
    fn refs(&self, o: ObjectId, feature: usize) -> Vec<ObjectId>;
    /// Adds a target (many-valued: at `index`, or at the end; single-valued: replaces the
    /// value), without maintaining anything else.
    fn basic_add(&mut self, o: ObjectId, feature: usize, target: ObjectId, index: Option<usize>);
    /// Removes a target, without maintaining anything else.
    fn basic_remove(&mut self, o: ObjectId, feature: usize, target: ObjectId);
    /// The values of an attribute (none for an unset nullable one).
    fn attribute(&self, o: ObjectId, feature: usize) -> Vec<Value>;
    /// Replaces the values of an attribute.
    fn set_attribute(&mut self, o: ObjectId, feature: usize, values: Vec<Value>);
}

/// Whether class `sub` is `sup` or one of its subclasses.
pub fn conforms(classes: &[ClassInfo], sub: usize, sup: usize) -> bool {
    sup == usize::MAX || sub == sup || classes[sub].supers.contains(&sup)
}

/// Adds `target` to reference `feature` of `o` like EMF does: a contained target leaves its
/// old container, and the opposite reference is updated (a single-valued opposite drops its
/// old value). For a single-valued reference the old value is replaced.
pub fn link<S: Store>(s: &mut S, o: ObjectId, feature: usize, target: ObjectId, index: Option<usize>) {
    let f = &s.features()[feature];
    if f.container {
        if let Some(opposite) = f.opposite {
            link(s, target, opposite, o, None);
        }
        return;
    }
    let current = s.refs(o, feature);
    if current.contains(&target) && (f.unique || !f.many) {
        return;
    }
    if !f.many {
        for old in current {
            unlink(s, o, feature, old);
        }
    }
    if f.containment {
        if let Some((c, cf)) = s.container(target) {
            unlink(s, c, cf, target);
        }
        s.set_container(target, Some((o, feature)));
    }
    s.basic_add(o, feature, target, index);
    if let (Some(opposite), false) = (f.opposite, f.containment) {
        if !s.features()[opposite].many {
            for old in s.refs(target, opposite) {
                if old != o {
                    s.basic_remove(old, feature, target);
                }
            }
        }
        s.basic_add(target, opposite, o, None);
    }
}

/// Removes `target` from reference `feature` of `o` (and `o` from the opposite).
pub fn unlink<S: Store>(s: &mut S, o: ObjectId, feature: usize, target: ObjectId) {
    let f = &s.features()[feature];
    if f.container {
        if let Some(opposite) = f.opposite {
            if s.container(o) == Some((target, opposite)) {
                unlink(s, target, opposite, o);
            }
        }
        return;
    }
    if !s.refs(o, feature).contains(&target) {
        return;
    }
    s.basic_remove(o, feature, target);
    if f.containment {
        s.set_container(target, None);
    } else if let Some(opposite) = f.opposite {
        s.basic_remove(target, opposite, o);
    }
}

/// Sets a single-valued reference.
pub fn set_reference<S: Store>(s: &mut S, o: ObjectId, feature: usize, target: Option<ObjectId>) {
    match target {
        Some(t) => link(s, o, feature, t, None),
        None => {
            let f = &s.features()[feature];
            if f.container {
                if let Some((c, cf)) = s.container(o) {
                    if Some(cf) == f.opposite {
                        unlink(s, c, cf, o);
                    }
                }
                return;
            }
            for old in s.refs(o, feature) {
                unlink(s, o, feature, old);
            }
        }
    }
}

/// Replaces the targets of a many-valued reference.
pub fn set_references<S: Store>(s: &mut S, o: ObjectId, feature: usize, targets: Vec<ObjectId>) {
    for old in s.refs(o, feature) {
        if !targets.contains(&old) {
            unlink(s, o, feature, old);
        }
    }
    for t in targets {
        link(s, o, feature, t, None);
    }
}

/// The value of a container reference (`feature`): the container, if it holds the object
/// through the opposite.
pub fn container_through<S: Store>(s: &S, o: ObjectId, feature: usize) -> Option<ObjectId> {
    let opposite = s.features()[feature].opposite?;
    match s.container(o) {
        Some((c, cf)) if cf == opposite => Some(c),
        _ => None,
    }
}

/// The objects `o` contains directly.
pub fn contents<S: Store>(s: &S, o: ObjectId) -> Vec<ObjectId> {
    let class = &s.classes()[s.class_index(o)];
    class.features.iter().filter(|f| s.features()[**f].containment).flat_map(|f| s.refs(o, *f)).collect()
}

/// The objects `o` contains, transitively (depth first).
pub fn all_contents<S: Store>(s: &S, o: ObjectId) -> Vec<ObjectId> {
    let mut out = Vec::new();
    for c in contents(s, o) {
        out.push(c);
        out.extend(all_contents(s, c));
    }
    out
}

/// The objects without a container, in creation order.
pub fn roots<S: Store>(s: &S) -> Vec<ObjectId> {
    s.object_ids().into_iter().filter(|o| s.container(*o).is_none()).collect()
}

/// Deletes an object and everything it contains, removing them from every reference (like
/// EMF's `EcoreUtil.delete`).
pub fn delete<S: Store>(s: &mut S, o: ObjectId) {
    if !s.contains_object(o) {
        return;
    }
    let mut doomed = vec![o];
    doomed.extend(all_contents(s, o));
    if let Some((c, cf)) = s.container(o) {
        unlink(s, c, cf, o);
    }
    for x in s.object_ids() {
        if doomed.contains(&x) {
            continue;
        }
        let class = &s.classes()[s.class_index(x)];
        for &f in class.features {
            let fi = &s.features()[f];
            if fi.is_reference() && !fi.containment && !fi.container {
                for t in s.refs(x, f) {
                    if doomed.contains(&t) {
                        s.basic_remove(x, f, t);
                    }
                }
            }
        }
    }
    for d in doomed {
        s.remove_object(d);
    }
}

/// The value of a feature named `name` of the class of `o`.
fn feature_named<S: Store>(s: &S, class: usize, name: &str) -> Option<usize> {
    s.classes()[class].features.iter().copied().find(|f| s.features()[*f].name == name)
}

/// The id of an object for references: its `iD` attribute's value.
fn id_of<S: Store>(s: &S, o: ObjectId) -> Option<String> {
    let class = &s.classes()[s.class_index(o)];
    class
        .features
        .iter()
        .find(|f| s.features()[**f].id && !s.features()[**f].many)
        .and_then(|f| s.attribute(o, *f).into_iter().next())
        .map(|v| format_value(s, &v))
}

/// EMF's URI fragment of an object: `//@books.0`, or `/1/@books.0` among several roots.
pub fn uri_fragment<S: Store>(s: &S, o: ObjectId) -> String {
    let mut segments = Vec::new();
    let mut cur = o;
    while let Some((c, f)) = s.container(cur) {
        let fi = &s.features()[f];
        if fi.many {
            let i = s.refs(c, f).iter().position(|x| *x == cur).unwrap_or(0);
            segments.push(format!("@{}.{i}", fi.name));
        } else {
            segments.push(format!("@{}", fi.name));
        }
        cur = c;
    }
    segments.reverse();
    let roots = roots(s);
    let root = if roots.len() <= 1 { String::new() } else { roots.iter().position(|r| *r == cur).unwrap_or(0).to_string() };
    let mut out = format!("/{root}");
    if !segments.is_empty() {
        out.push('/');
        out.push_str(&segments.join("/"));
    }
    out
}

/// A human readable label such as `Book 'Dune'` (by the `name` attribute) or
/// `Book //@books.0`.
pub fn label<S: Store>(s: &S, o: ObjectId) -> String {
    let class = s.class_index(o);
    let name = s.classes()[class].name;
    if let Some(f) = feature_named(s, class, "name") {
        if let Some(Value::String(n)) = s.attribute(o, f).into_iter().next() {
            return format!("{name} '{n}'");
        }
    }
    format!("{name} {}", uri_fragment(s, o))
}

fn format_value<S: Store>(s: &S, v: &Value) -> String {
    match v {
        Value::Boolean(b) => b.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Real(r) => crate::ocl::format_real(*r),
        Value::String(t) => t.clone(),
        Value::Enum(_) => {
            let _ = s;
            String::new()
        }
    }
}

fn format_attribute<S: Store>(s: &S, f: &FeatureInfo, v: &Value) -> String {
    match (v, f.kind) {
        (Value::Enum(i), ValueKind::Enum(e)) => s.enums()[e].literals.get(*i).copied().unwrap_or("").to_string(),
        _ => format_value(s, v),
    }
}

fn parse_attribute<S: Store>(s: &S, f: &FeatureInfo, text: &str) -> Result<Value, String> {
    match f.kind {
        ValueKind::Boolean => match text.trim() {
            "true" => Ok(Value::Boolean(true)),
            "false" => Ok(Value::Boolean(false)),
            _ => Err(format!("'{text}' is not a boolean")),
        },
        ValueKind::Integer => text.trim().parse().map(Value::Integer).map_err(|_| format!("'{text}' is not an integer")),
        ValueKind::Real => text.trim().parse().map(Value::Real).map_err(|_| format!("'{text}' is not a number")),
        ValueKind::Enum(e) => {
            let info = &s.enums()[e];
            info.literals
                .iter()
                .position(|l| *l == text)
                .or_else(|| info.names.iter().position(|n| *n == text))
                .map(Value::Enum)
                .ok_or_else(|| format!("'{text}' is not a literal of {}", info.name))
        }
        ValueKind::String | ValueKind::Object(_) => Ok(Value::String(text.to_string())),
    }
}

// ------------------------------------------------------------------ XMI

/// Reads an XMI document. Problems that do not prevent loading (unknown features,
/// unresolved references, bad values) are returned as warnings.
pub fn read_xmi<S: Store>(text: &str) -> Result<(S, Vec<String>), String> {
    let doc = xml::parse(text)?;
    let mut r = Reader { s: S::default(), namespaces: doc.namespaces, warnings: Vec::new(), pending: Vec::new(), ids: Vec::new() };
    let roots: Vec<&xml::Element> = if doc.root.local_name() == "XMI" && r.uri_of(doc.root.prefix()) == Some(XMI_NS) {
        doc.root.children.iter().collect()
    } else {
        vec![&doc.root]
    };
    let mut created = Vec::new();
    for e in roots {
        let class = match e.attribute("xsi:type") {
            Some(t) => r.class_named(t)?,
            None => r.class_named(&e.name)?,
        };
        created.push(r.object(e, class)?);
    }
    r.resolve(&created);
    Ok((r.s, r.warnings))
}

struct Reader<S: Store> {
    s: S,
    namespaces: std::collections::HashMap<String, String>,
    warnings: Vec<String>,
    /// References to resolve once all objects exist: object, feature, the references.
    pending: Vec<(ObjectId, usize, Vec<String>)>,
    /// Objects by `xmi:id` or `iD` attribute.
    ids: Vec<(String, ObjectId)>,
}

impl<S: Store> Reader<S> {
    fn uri_of(&self, prefix: Option<&str>) -> Option<&str> {
        self.namespaces.get(prefix.unwrap_or("")).map(String::as_str)
    }

    fn class_named(&self, qname: &str) -> Result<usize, String> {
        let (prefix, name) = match qname.split_once(':') {
            Some((p, n)) => (Some(p), n),
            None => (None, qname),
        };
        let uri = self.uri_of(prefix).unwrap_or("");
        let package = self.s.packages().iter().position(|p| p.ns_uri == uri);
        self.s
            .classes()
            .iter()
            .position(|c| Some(c.package) == package && c.name == name)
            .ok_or_else(|| format!("unknown class '{qname}'"))
    }

    fn object(&mut self, e: &xml::Element, class: usize) -> Result<ObjectId, String> {
        let info = &self.s.classes()[class];
        if info.is_abstract {
            return Err(format!("cannot instantiate the abstract class {}", info.name));
        }
        let o = self.s.create_object(class);
        for (name, value) in &e.attributes {
            if name.contains(':') || name == "xmlns" {
                if name == "xmi:id" {
                    self.ids.push((value.clone(), o));
                }
                continue;
            }
            let Some(f) = feature_named(&self.s, class, name) else {
                self.warnings.push(format!("{} has no feature '{name}'", info.name));
                continue;
            };
            let fi = &self.s.features()[f];
            if fi.is_reference() {
                self.pending.push((o, f, value.split_whitespace().map(str::to_string).collect()));
            } else {
                self.attribute_value(o, f, value);
            }
        }
        for child in &e.children {
            let name = child.local_name();
            let Some(f) = feature_named(&self.s, class, name) else {
                self.warnings.push(format!("{} has no feature '{name}'", info.name));
                continue;
            };
            let fi = &self.s.features()[f];
            if !fi.is_reference() {
                self.attribute_value(o, f, &child.text);
            } else if let Some(href) = child.attribute("href") {
                self.warnings.push(format!("references to other documents are not supported: '{href}'"));
            } else if fi.containment {
                let class = match (child.attribute("xsi:type"), fi.kind) {
                    (Some(t), _) => self.class_named(t)?,
                    (None, ValueKind::Object(Some(c))) => c,
                    (None, _) => return Err(format!("the class of '{name}' is missing (xsi:type)")),
                };
                let target = self.object(child, class)?;
                link(&mut self.s, o, f, target, None);
            } else {
                self.warnings.push(format!("unexpected element '{name}'"));
            }
        }
        if let Some(id) = id_of(&self.s, o) {
            self.ids.push((id, o));
        }
        Ok(o)
    }

    fn attribute_value(&mut self, o: ObjectId, f: usize, text: &str) {
        let fi = &self.s.features()[f];
        match parse_attribute(&self.s, fi, text) {
            Ok(v) => {
                let mut values = if fi.many { self.s.attribute(o, f) } else { Vec::new() };
                values.push(v);
                self.s.set_attribute(o, f, values);
            }
            Err(e) => self.warnings.push(format!("{}: {e}", fi.name)),
        }
    }

    fn find(&self, roots: &[ObjectId], reference: &str) -> Option<ObjectId> {
        let fragment = reference.split_once('#').map_or(reference, |(_, f)| f);
        if let Some((_, o)) = self.ids.iter().find(|(id, _)| id == fragment) {
            return Some(*o);
        }
        let mut segments = fragment.strip_prefix('/')?.split('/');
        let head = segments.next().unwrap_or("");
        let mut cur = *roots.get(if head.is_empty() { 0 } else { head.parse().ok()? })?;
        for segment in segments.filter(|s| !s.is_empty()) {
            let (name, index) = match segment.strip_prefix('@')?.split_once('.') {
                Some((n, i)) => (n, i.parse().ok()?),
                None => (segment.strip_prefix('@')?, 0),
            };
            let f = feature_named(&self.s, self.s.class_index(cur), name)?;
            cur = *self.s.refs(cur, f).get(index)?;
        }
        Some(cur)
    }

    fn resolve(&mut self, roots: &[ObjectId]) {
        for (o, f, refs) in std::mem::take(&mut self.pending) {
            let fi = &self.s.features()[f];
            if fi.container {
                continue;
            }
            for reference in refs {
                let Some(target) = self.find(roots, &reference) else {
                    self.warnings.push(format!("cannot resolve '{reference}' ({})", fi.name));
                    continue;
                };
                if let ValueKind::Object(Some(c)) = fi.kind {
                    if !conforms(self.s.classes(), self.s.class_index(target), c) {
                        self.warnings.push(format!("'{reference}' is not a {}", self.s.classes()[c].name));
                        continue;
                    }
                }
                link(&mut self.s, o, f, target, None);
            }
        }
    }
}

/// Writes the objects without a container as an XMI document.
pub fn write_xmi<S: Store>(s: &S) -> String {
    let roots = roots(s);
    let mut prefixes: Vec<(String, &'static str)> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    let single = roots.len() == 1;
    for r in &roots {
        write_object(s, *r, None, None, if single { 0 } else { 1 }, &mut prefixes, &mut lines);
    }
    prefixes.sort();
    let mut namespaces = format!(" xmi:version=\"2.0\" xmlns:xmi=\"{XMI_NS}\" xmlns:xsi=\"{XSI_NS}\"");
    for (p, uri) in &prefixes {
        namespaces.push_str(&format!(" xmlns:{p}=\"{}\"", xml::escape(uri)));
    }
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    if single {
        let first = lines[0].replacen("{NS}", &namespaces, 1);
        out.push_str(&first);
        out.push('\n');
        for l in &lines[1..] {
            out.push_str(l);
            out.push('\n');
        }
    } else {
        out.push_str(&format!("<xmi:XMI{namespaces}>\n"));
        for l in &lines {
            out.push_str(&l.replacen("{NS}", "", 1));
            out.push('\n');
        }
        out.push_str("</xmi:XMI>\n");
    }
    out
}

fn qualified_class<S: Store>(s: &S, class: usize, prefixes: &mut Vec<(String, &'static str)>) -> String {
    let info = &s.classes()[class];
    let package = &s.packages()[info.package];
    let prefix = if package.ns_prefix.is_empty() { package.name } else { package.ns_prefix };
    if !prefixes.iter().any(|(p, _)| p == prefix) {
        prefixes.push((prefix.to_string(), package.ns_uri));
    }
    format!("{prefix}:{}", info.name)
}

fn write_object<S: Store>(
    s: &S,
    o: ObjectId,
    tag: Option<&str>,
    declared: Option<usize>,
    depth: usize,
    prefixes: &mut Vec<(String, &'static str)>,
    lines: &mut Vec<String>,
) {
    let class = s.class_index(o);
    let indent = "  ".repeat(depth);
    let mut head = match tag {
        Some(t) => format!("{indent}<{t}"),
        None => format!("{indent}<{}{{NS}}", qualified_class(s, class, prefixes)),
    };
    if tag.is_some() && declared != Some(class) {
        head.push_str(&format!(" xsi:type=\"{}\"", qualified_class(s, class, prefixes)));
    }
    let mut children: Vec<(usize, Option<String>, Option<ObjectId>)> = Vec::new();
    for &f in s.classes()[class].features {
        let fi = &s.features()[f];
        if !fi.serialized || fi.container {
            continue;
        }
        if fi.is_reference() {
            let targets = s.refs(o, f);
            if targets.is_empty() {
                continue;
            }
            if fi.containment {
                children.extend(targets.into_iter().map(|t| (f, None, Some(t))));
            } else {
                let refs: Vec<String> = targets.iter().map(|t| id_of(s, *t).unwrap_or_else(|| uri_fragment(s, *t))).collect();
                head.push_str(&format!(" {}=\"{}\"", fi.name, xml::escape(&refs.join(" "))));
            }
            continue;
        }
        let values = s.attribute(o, f);
        if fi.many {
            children.extend(values.iter().map(|v| (f, Some(format_attribute(s, fi, v)), None)));
        } else if let Some(v) = values.first() {
            let text = format_attribute(s, fi, v);
            if fi.default != Some(text.as_str()) {
                head.push_str(&format!(" {}=\"{}\"", fi.name, xml::escape(&text)));
            }
        }
    }
    if children.is_empty() {
        lines.push(format!("{head}/>"));
        return;
    }
    lines.push(format!("{head}>"));
    for (f, text, target) in children {
        let fi = &s.features()[f];
        match (text, target) {
            (Some(t), _) => lines.push(format!("{indent}  <{}>{}</{}>", fi.name, xml::escape(&t), fi.name)),
            (None, Some(t)) => {
                let declared = match fi.kind {
                    ValueKind::Object(c) => c,
                    _ => None,
                };
                write_object(s, t, Some(fi.name), declared, depth + 1, prefixes, lines);
            }
            (None, None) => {}
        }
    }
    let close = match tag {
        Some(t) => t.to_string(),
        None => qualified_class(s, class, prefixes),
    };
    lines.push(format!("{indent}</{close}>"));
}

// ------------------------------------------------------------------ validation

/// What a [`Diagnostic`] is about.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DiagnosticKind {
    Invariant,
    Multiplicity,
}

/// A violated (or failing) invariant or multiplicity, found by `Model::validate`.
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub object: ObjectId,
    /// The object's label, e.g. `Book 'Dune'`.
    pub label: String,
    /// `Class::Name` of the invariant, or `Class::feature` for multiplicities.
    pub constraint: String,
    pub kind: DiagnosticKind,
    /// False when the invariant could not be evaluated (`message` says why).
    pub violated: bool,
    pub message: Option<String>,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} on {}", self.constraint, if self.violated { "violated" } else { "failed" }, self.label)?;
        if let Some(m) = &self.message {
            write!(f, ": {m}")?;
        }
        Ok(())
    }
}

/// Checks the bounds of the stored features of an object.
pub fn check_multiplicities<S: Store>(s: &S, o: ObjectId, out: &mut Vec<Diagnostic>) {
    let class = s.class_index(o);
    for &f in s.classes()[class].features {
        let fi = &s.features()[f];
        if fi.container {
            continue;
        }
        let count = if fi.is_reference() { s.refs(o, f).len() } else { s.attribute(o, f).len() } as i64;
        let message = if count < fi.lower {
            format!("{} needs at least {} value(s), has {count}", fi.name, fi.lower)
        } else if fi.many && fi.upper >= 0 && count > fi.upper {
            format!("{} allows at most {} value(s), has {count}", fi.name, fi.upper)
        } else {
            continue;
        };
        out.push(Diagnostic {
            object: o,
            label: label(s, o),
            constraint: format!("{}::{}", s.classes()[class].name, fi.name),
            kind: DiagnosticKind::Multiplicity,
            violated: true,
            message: Some(message),
        });
    }
}
