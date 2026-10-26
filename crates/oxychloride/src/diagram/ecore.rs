//! Diagram edits of `.ecore` files, made on the XML: new elements are written the way EMF (and
//! `oxy_ecore::ecore_writer`) writes them, changed elements get new attribute values in place,
//! and same-file references to renamed elements (`#//Class/feature`) are rewritten.

use std::collections::{HashMap, HashSet};
use std::ops::Range as ByteRange;

use oxy_ecore::ecore_writer::{href, type_href};
use oxy_ecore::xml::{self, Element as Xml, XSI_NS, escape_attr};
use oxy_ecore::{ClassifierId, ElementRef, LoadError, Metamodel};

use crate::registry::Locator;

use super::oclinecore::check_super_type;
use super::*;

/// An `.ecore` file loaded on its own.
pub(crate) struct File {
    pub(crate) mm: Metamodel,
    pub(crate) resource: usize,
    pub(crate) root: Xml,
    pub(crate) warnings: Vec<LoadError>,
}

impl File {
    /// Loads the file's text (named by its path: referenced files are loaded relative to it, or
    /// from where the package registry says).
    pub(crate) fn load(name: &str, text: &str, locator: &Locator) -> Result<File, Error> {
        let root = xml::parse(text, name)?;
        let mut mm = Metamodel::new();
        let load = locator.load_str(&mut mm, text, name)?;
        Ok(File { mm, resource: load.resource, root, warnings: load.warnings })
    }
}

/// Attributes whose values are references (`#//Class`, `other.ecore#//Class`, …).
const REFERENCE_ATTRIBUTES: &[&str] =
    &["eType", "eSuperTypes", "eOpposite", "eClassifier", "eKeys", "eExceptions", "references"];

/// The order EMF writes attributes in (by local name); others go first.
const ATTRIBUTE_ORDER: &[&str] = &[
    "type",
    "name",
    "nsURI",
    "nsPrefix",
    "instanceClassName",
    "literal",
    "value",
    "abstract",
    "interface",
    "eSuperTypes",
    "ordered",
    "unique",
    "lowerBound",
    "upperBound",
    "eType",
    "changeable",
    "volatile",
    "transient",
    "defaultValueLiteral",
    "unsettable",
    "derived",
    "iD",
    "containment",
    "resolveProxies",
    "eOpposite",
    "eKeys",
    "eExceptions",
];

fn attribute_rank(name: &str) -> usize {
    let local = xml::split_qname(name).1;
    ATTRIBUTE_ORDER.iter().position(|a| *a == local).map_or(0, |i| i + 1)
}

/// An attribute of a start tag, by byte offsets.
struct Attr {
    name: String,
    start: usize,
    value: ByteRange<usize>,
    end: usize,
}

/// A start tag, by byte offsets.
struct Tag {
    attrs: Vec<Attr>,
    /// The end of the element name.
    name_end: usize,
    /// The end of the tag (after `>`).
    end: usize,
    /// `<x/>`.
    empty: bool,
}

fn tag(text: &str, el: &Xml) -> Tag {
    let b = text.as_bytes();
    let mut i = el.offset + 1;
    while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'/' && b[i] != b'>' {
        i += 1;
    }
    let name_end = i;
    let mut attrs = Vec::new();
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            return Tag { attrs, name_end, end: b.len(), empty: false };
        }
        if b[i] == b'/' && b.get(i + 1) == Some(&b'>') {
            return Tag { attrs, name_end, end: i + 2, empty: true };
        }
        if b[i] == b'>' {
            return Tag { attrs, name_end, end: i + 1, empty: false };
        }
        let start = i;
        while i < b.len() && b[i] != b'=' && !b[i].is_ascii_whitespace() && b[i] != b'>' {
            i += 1;
        }
        let name = text[start..i].to_string();
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'=') {
            i += 1;
        }
        let Some(&quote) = b.get(i).filter(|q| **q == b'"' || **q == b'\'') else {
            return Tag { attrs, name_end, end: i.min(b.len()), empty: false };
        };
        i += 1;
        let value_start = i;
        while i < b.len() && b[i] != quote {
            i += 1;
        }
        let value = value_start..i;
        i = (i + 1).min(b.len());
        attrs.push(Attr { name, start, value, end: i });
    }
}

/// `<name a="v" .../>`.
fn empty_element(name: &str, attrs: &[(&str, String)]) -> String {
    let mut out = format!("<{name}");
    for (k, v) in attrs {
        out.push_str(&format!(" {k}=\"{}\"", escape_attr(v)));
    }
    out.push_str("/>");
    out
}

/// `lowerBound` and `upperBound` unless they are Ecore's defaults (0 and 1).
fn bounds(lower: i32, upper: i32) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if lower != 0 {
        out.push(("lowerBound", lower.to_string()));
    }
    if upper != 1 {
        out.push(("upperBound", upper.to_string()));
    }
    out
}

#[derive(Clone, Copy)]
enum Node<'a> {
    Package(&'a Xml),
    Classifier(&'a Xml),
    Member(&'a Xml),
}

impl<'a> Node<'a> {
    fn el(self) -> &'a Xml {
        match self {
            Node::Package(e) | Node::Classifier(e) | Node::Member(e) => e,
        }
    }
}

/// An `.ecore` document with the ids of the elements it defines.
pub(crate) struct Doc<'a> {
    text: &'a str,
    root: &'a Xml,
    nodes: HashMap<String, Node<'a>>,
    /// The prefixes of the XML Schema instance and Ecore namespaces.
    xsi: String,
    ecore: String,
}

impl<'a> Doc<'a> {
    pub(crate) fn new(text: &'a str, root: &'a Xml) -> Self {
        let prefix = |ns: &str, default: &str| {
            root.namespaces.iter().find(|(_, u)| *u == ns).map_or(default.to_string(), |(p, _)| p.clone())
        };
        let mut d = Doc {
            text,
            root,
            nodes: HashMap::new(),
            xsi: prefix(XSI_NS, "xsi"),
            ecore: prefix(oxy_ecore::ECORE_NS_URI, "ecore"),
        };
        let packages: Vec<&Xml> =
            if root.local_name() == "XMI" { root.children_named("EPackage").collect() } else { vec![root] };
        let mut alloc = IdAllocator::default();
        for p in packages {
            d.package(p, "", &mut alloc);
        }
        d
    }

    fn package(&mut self, p: &'a Xml, prefix: &str, alloc: &mut IdAllocator) {
        let id = alloc.next(format!("{prefix}{}", p.attr("name").unwrap_or_default()));
        self.nodes.insert(id.clone(), Node::Package(p));
        for c in p.children_named("eClassifiers") {
            let cid = alloc.next(classifier_id(&id, c.attr("name").unwrap_or_default()));
            self.nodes.insert(cid.clone(), Node::Classifier(c));
            for child in &c.children {
                let name = child.attr("name").unwrap_or_default();
                let member = match child.local_name() {
                    "eStructuralFeatures" | "eLiterals" => member_id(&cid, name),
                    "eOperations" => operation_id(&cid, name),
                    _ => continue,
                };
                self.nodes.insert(alloc.next(member), Node::Member(child));
            }
        }
        for s in p.children_named("eSubpackages") {
            self.package(s, &format!("{id}::"), alloc);
        }
    }

    pub(crate) fn source_map(&self) -> SourceMap {
        let mut map = SourceMap::default();
        for (id, node) in &self.nodes {
            let el = node.el();
            map.spans.insert(id.clone(), el.offset..el.end);
            if let Node::Member(m) = node
                && let Some(t) =
                    m.attr("eType").or_else(|| m.children_named("eGenericType").find_map(|g| g.attr("eClassifier")))
            {
                let name = t.rsplit(['/', '#']).next().unwrap_or(t);
                map.type_texts.insert(id.clone(), name.to_string());
            }
        }
        map
    }

    fn node(&self, id: &str) -> Result<Node<'a>, Error> {
        self.nodes.get(id).copied().ok_or_else(|| Error::NotFound(format!("no element '{id}' in the document")))
    }

    fn el_of(&self, cx: &Context, e: Element) -> Result<&'a Xml, Error> {
        let id = cx.id(e).ok_or_else(|| Error::NotFound("the element is not defined in the document".into()))?;
        Ok(self.node(id)?.el())
    }

    fn xsi_type(&self, ty: &str) -> (String, String) {
        (format!("{}:type", self.xsi), format!("{}:{ty}", self.ecore))
    }

    /// An `eType` value, with the document's prefix for the type hint.
    fn type_value(&self, cx: &Context, t: ClassifierId) -> String {
        let value = type_href(cx.mm, cx.resource, t);
        match value.strip_prefix("ecore:") {
            Some(rest) if self.ecore != "ecore" => format!("{}:{rest}", self.ecore),
            _ => value,
        }
    }

    fn last_child(&self, el: &'a Xml, names: &[&str]) -> Option<&'a Xml> {
        names.iter().find_map(|n| el.children.iter().rfind(|c| c.local_name() == *n))
    }

    /// Inserts child elements (lines without indentation) after `after`, else at the start.
    fn insert_child(&self, parent: &Xml, after: Option<&Xml>, child: &str) -> ByteEdit {
        let text = self.text;
        let t = tag(text, parent);
        if t.empty {
            let indent = indentation(text, parent.offset);
            let unit = indent_unit(text);
            let lines: Vec<String> = child.lines().map(|l| format!("{indent}{unit}{l}")).collect();
            let start = text[..t.end - 2].trim_end().len();
            return ByteEdit::replace(start..t.end, format!(">\n{}\n{indent}</{}>", lines.join("\n"), parent.name));
        }
        let close = text[..parent.end].rfind("</").unwrap_or(parent.end);
        insert_member(text, parent.offset, t.end, close, after.map(|a| a.offset..a.end), child, false)
    }

    /// Sets (`Some`) or removes (`None`) an attribute of a start tag.
    fn set_attr(&self, el: &Xml, name: &str, value: Option<&str>) -> Option<ByteEdit> {
        let t = tag(self.text, el);
        match (t.attrs.iter().find(|a| a.name == name), value) {
            (Some(a), Some(v)) => {
                let escaped = escape_attr(v);
                (self.text[a.value.clone()] != escaped).then(|| ByteEdit::replace(a.value.clone(), escaped))
            }
            (Some(a), None) => Some(ByteEdit::delete(self.text[..a.start].trim_end().len()..a.end)),
            (None, Some(v)) => {
                let rank = attribute_rank(name);
                let i = t.attrs.iter().position(|a| attribute_rank(&a.name) > rank).unwrap_or(t.attrs.len());
                let at = if i == 0 { t.name_end } else { t.attrs[i - 1].end };
                Some(ByteEdit::insert(at, format!(" {name}=\"{}\"", escape_attr(v))))
            }
            (None, None) => None,
        }
    }

    fn flag(&self, el: &Xml, name: &str, value: bool, default: bool) -> Option<ByteEdit> {
        self.set_attr(el, name, (value != default).then_some(if value { "true" } else { "false" }))
    }

    // -------------------------------------------------------------- edits

    pub(crate) fn edit(&self, cx: &Context, edit: &MetamodelEdit) -> Result<(Vec<ByteEdit>, Option<String>), Error> {
        let mm = cx.mm;
        match edit {
            MetamodelEdit::AddClassifier { package, classifier, name } => {
                let Element::Package(p) = cx.element(package)? else {
                    return Err(Error::Invalid(format!("'{package}' is not a package")));
                };
                cx.check_new_classifier(p, name)?;
                let el = self.node(package)?.el();
                let kind = match classifier {
                    NewClassifier::Class | NewClassifier::AbstractClass | NewClassifier::Interface => "EClass",
                    NewClassifier::Enum => "EEnum",
                    NewClassifier::DataType => "EDataType",
                };
                let (k, v) = self.xsi_type(kind);
                let mut attrs = vec![(k.as_str(), v), ("name", name.clone())];
                match classifier {
                    NewClassifier::AbstractClass => attrs.push(("abstract", "true".into())),
                    NewClassifier::Interface => {
                        attrs.push(("abstract", "true".into()));
                        attrs.push(("interface", "true".into()));
                    }
                    NewClassifier::DataType => attrs.push(("instanceClassName", "java.lang.String".into())),
                    NewClassifier::Class | NewClassifier::Enum => {}
                }
                let after = self.last_child(el, &["eClassifiers", "eAnnotations"]);
                let edit = self.insert_child(el, after, &empty_element("eClassifiers", &attrs));
                Ok((vec![edit], Some(classifier_id(package, name))))
            }
            MetamodelEdit::AddAttribute { owner, name, ty, lower, upper } => {
                let c = cx.class(owner)?;
                cx.check_new_feature(c, name)?;
                let t = cx.feature_type(ty, false)?;
                let (k, v) = self.xsi_type("EAttribute");
                let mut attrs = vec![(k.as_str(), v), ("name", name.clone())];
                attrs.extend(bounds(*lower, *upper));
                attrs.push(("eType", self.type_value(cx, t)));
                let el = self.el_of(cx, Element::Classifier(c))?;
                let edit =
                    self.insert_child(el, self.feature_anchor(el), &empty_element("eStructuralFeatures", &attrs));
                Ok((vec![edit], Some(member_id(owner, name))))
            }
            MetamodelEdit::AddReference { owner, name, ty, lower, upper, containment, opposite } => {
                let c = cx.class(owner)?;
                cx.check_new_feature(c, name)?;
                let t = cx.feature_type(ty, true)?;
                let (k, v) = self.xsi_type("EReference");
                let mut attrs = vec![(k.as_str(), v.clone()), ("name", name.clone())];
                attrs.extend(bounds(*lower, *upper));
                attrs.push(("eType", self.type_value(cx, t)));
                if *containment {
                    attrs.push(("containment", "true".into()));
                }
                let mut members = Vec::new();
                if let Some(o) = opposite {
                    if !cx.in_resource(t) {
                        return Err(Error::Invalid("the opposite would have to be added to another metamodel".into()));
                    }
                    cx.check_new_feature(t, o)?;
                    if t == c && o == name {
                        return Err(Error::Invalid("a reference cannot be its own opposite".into()));
                    }
                    attrs.push(("eOpposite", format!("{}/{o}", href(mm, cx.resource, ElementRef::Classifier(t)))));
                    let back = vec![
                        (k.as_str(), v),
                        ("name", o.clone()),
                        ("eType", self.type_value(cx, c)),
                        ("eOpposite", format!("{}/{name}", href(mm, cx.resource, ElementRef::Classifier(c)))),
                    ];
                    members.push((t, empty_element("eStructuralFeatures", &back)));
                }
                members.insert(0, (c, empty_element("eStructuralFeatures", &attrs)));
                let edits = if members.len() == 2 && members[0].0 == members[1].0 {
                    let el = self.el_of(cx, Element::Classifier(c))?;
                    vec![self.insert_child(el, self.feature_anchor(el), &format!("{}\n{}", members[0].1, members[1].1))]
                } else {
                    members
                        .iter()
                        .map(|(class, m)| {
                            let el = self.el_of(cx, Element::Classifier(*class))?;
                            Ok(self.insert_child(el, self.feature_anchor(el), m))
                        })
                        .collect::<Result<_, Error>>()?
                };
                Ok((edits, Some(member_id(owner, name))))
            }
            MetamodelEdit::AddOperation { owner, name, ty, lower, upper, parameters } => {
                let c = cx.class(owner)?;
                cx.check_name(name)?;
                let mut attrs = vec![("name", name.clone())];
                if let Some(t) = ty.as_deref().filter(|t| !t.is_empty()) {
                    let t = cx.resolve_type(t)?;
                    attrs.extend(bounds(*lower, *upper));
                    attrs.push(("eType", self.type_value(cx, t)));
                }
                let element = empty_element("eOperations", &attrs);
                let params = self.parameters(cx, parameters)?;
                let text = if params.is_empty() {
                    element
                } else {
                    let unit = indent_unit(self.text);
                    let inner: Vec<String> = params.iter().map(|p| format!("{unit}{p}")).collect();
                    format!("{}>\n{}\n</eOperations>", element.trim_end_matches("/>"), inner.join("\n"))
                };
                let el = self.el_of(cx, Element::Classifier(c))?;
                let after =
                    self.last_child(el, &["eOperations", "eGenericSuperTypes", "eTypeParameters", "eAnnotations"]);
                Ok((vec![self.insert_child(el, after, &text)], Some(operation_id(owner, name))))
            }
            MetamodelEdit::AddLiteral { owner, name } => {
                let e = cx.enumeration(owner)?;
                cx.check_name(name)?;
                if mm.enum_literal_index(e, name).is_some() {
                    return Err(Error::Invalid(format!(
                        "'{}' already has a literal named '{name}'",
                        mm.classifier(e).name
                    )));
                }
                let value = mm.enum_literals(e).iter().map(|l| l.value + 1).max().unwrap_or(0);
                let mut attrs = vec![("name", name.clone())];
                if value != 0 {
                    attrs.push(("value", value.to_string()));
                }
                let el = self.el_of(cx, Element::Classifier(e))?;
                let after = self.last_child(el, &["eLiterals", "eAnnotations"]);
                Ok((
                    vec![self.insert_child(el, after, &empty_element("eLiterals", &attrs))],
                    Some(member_id(owner, name)),
                ))
            }
            MetamodelEdit::AddSuperType { owner, super_type } => {
                let c = cx.class(owner)?;
                let s = cx.resolve_type(super_type)?;
                check_super_type(cx, c, s)?;
                let el = self.el_of(cx, Element::Classifier(c))?;
                let reference = href(mm, cx.resource, ElementRef::Classifier(s));
                let generic: Vec<&Xml> = el.children_named("eGenericSuperTypes").collect();
                let edit = if let Some(last) = generic.last() {
                    self.insert_child(
                        el,
                        Some(last),
                        &empty_element("eGenericSuperTypes", &[("eClassifier", reference)]),
                    )
                } else {
                    let value = match el.attr("eSuperTypes").filter(|v| !v.trim().is_empty()) {
                        Some(v) => format!("{} {reference}", v.trim()),
                        None => reference,
                    };
                    self.set_attr(el, "eSuperTypes", Some(&value)).expect("the value changes")
                };
                Ok((vec![edit], None))
            }
            MetamodelEdit::RemoveSuperType { owner, super_type } => {
                let c = cx.class(owner)?;
                let s = cx.resolve_type(super_type)?;
                let edits = self.remove_supers(cx, c, &HashSet::from([s]))?;
                if edits.is_empty() {
                    return Err(Error::NotFound(format!("'{super_type}' is not a supertype of '{owner}'")));
                }
                Ok((edits, None))
            }
            MetamodelEdit::Update { element, properties } => self.update(cx, element, properties),
            MetamodelEdit::Delete { elements } => {
                let deleted = cx.deleted(elements)?;
                let mut edits = Vec::new();
                for e in &deleted {
                    if let Some(id) = cx.id(*e) {
                        let el = self.node(id)?.el();
                        edits.push(ByteEdit::delete(removal(self.text, el.offset..el.end, None)));
                    }
                }
                for c in cx.classes() {
                    if cx.is_deleted(Element::Classifier(c), &deleted) {
                        continue;
                    }
                    let gone: HashSet<ClassifierId> = mm
                        .super_types(c)
                        .iter()
                        .copied()
                        .filter(|s| deleted.contains(&Element::Classifier(*s)))
                        .collect();
                    if !gone.is_empty() {
                        edits.extend(self.remove_supers(cx, c, &gone)?);
                    }
                }
                for f in cx.features() {
                    if cx.is_deleted(Element::Feature(f), &deleted) {
                        continue;
                    }
                    if let Some(o) = mm.feature(f).opposite()
                        && deleted.contains(&Element::Feature(o))
                    {
                        edits.extend(self.set_attr(self.el_of(cx, Element::Feature(f))?, "eOpposite", None));
                    }
                }
                Ok((edits, None))
            }
        }
    }

    /// Where new features go: after the last feature, else after operations, supertypes and
    /// annotations.
    fn feature_anchor(&self, class: &'a Xml) -> Option<&'a Xml> {
        self.last_child(
            class,
            &["eStructuralFeatures", "eOperations", "eGenericSuperTypes", "eTypeParameters", "eAnnotations"],
        )
    }

    fn parameters(&self, cx: &Context, params: &[ParameterSpec]) -> Result<Vec<String>, Error> {
        let mut out = Vec::new();
        for p in params {
            cx.check_name(&p.name)?;
            let t = cx.resolve_type(&p.ty)?;
            let mut attrs = vec![("name", p.name.clone())];
            attrs.extend(bounds(p.lower, p.upper));
            attrs.push(("eType", self.type_value(cx, t)));
            out.push(empty_element("eParameters", &attrs));
        }
        Ok(out)
    }

    /// The edits that remove supertypes of class `c`.
    fn remove_supers(
        &self,
        cx: &Context,
        c: ClassifierId,
        remove: &HashSet<ClassifierId>,
    ) -> Result<Vec<ByteEdit>, Error> {
        let el = self.el_of(cx, Element::Classifier(c))?;
        let matches = |token: &str| remove.iter().any(|r| self.names(cx, token, *r));
        let mut edits = Vec::new();
        if let Some(value) = el.attr("eSuperTypes") {
            let keep: Vec<&str> = value.split_whitespace().filter(|t| !matches(t)).collect();
            if keep.len() != value.split_whitespace().count() {
                let value = keep.join(" ");
                edits.extend(self.set_attr(el, "eSuperTypes", (!value.is_empty()).then_some(value.as_str())));
            }
        }
        for g in el.children_named("eGenericSuperTypes") {
            if g.attr("eClassifier").is_some_and(matches) {
                edits.push(ByteEdit::delete(removal(self.text, g.offset..g.end, None)));
            }
        }
        Ok(edits)
    }

    /// Whether a reference (`#//A`, `other.ecore#//A`, possibly after a type hint) names `c`.
    fn names(&self, cx: &Context, token: &str, c: ClassifierId) -> bool {
        let target = href(cx.mm, cx.resource, ElementRef::Classifier(c));
        let fragment = |h: &str| h.split_once('#').map(|(_, f)| f.to_string());
        let (own, other) = (token.split('#').next().unwrap_or(""), target.split('#').next().unwrap_or(""));
        fragment(token) == fragment(&target) && (own.is_empty() == other.is_empty())
    }

    /// The edits that rewrite same-file references into an element that is renamed.
    fn rename_references(&self, cx: &Context, target: Element, name: &str) -> Vec<ByteEdit> {
        let Some((root, path)) = fragment_path(cx.mm, cx.resource, target) else { return Vec::new() };
        let mut edits = Vec::new();
        let mut stack = vec![self.root];
        while let Some(el) = stack.pop() {
            stack.extend(el.children.iter());
            if !el.attrs.iter().any(|(k, _)| REFERENCE_ATTRIBUTES.contains(&xml::split_qname(k).1)) {
                continue;
            }
            for a in tag(self.text, el).attrs {
                if !REFERENCE_ATTRIBUTES.contains(&xml::split_qname(&a.name).1) {
                    continue;
                }
                let value = &self.text[a.value.clone()];
                let mut changed = false;
                let tokens: Vec<String> = value
                    .split_whitespace()
                    .map(|t| match rename_in_fragment(t, root, &path, name) {
                        Some(new) => {
                            changed = true;
                            new
                        }
                        None => t.to_string(),
                    })
                    .collect();
                if changed {
                    edits.push(ByteEdit::replace(a.value.clone(), escape_attr(&tokens.join(" "))));
                }
            }
        }
        edits
    }

    fn update(&self, cx: &Context, element: &str, p: &ElementUpdate) -> Result<(Vec<ByteEdit>, Option<String>), Error> {
        let mm = cx.mm;
        let e = cx.element(element)?;
        let el = self.node(element)?.el();
        let mut edits = Vec::new();
        let mut renamed = None;
        let current = el.attr("name").unwrap_or_default();
        if let Some(name) = p.name.as_ref().filter(|n| *n != current) {
            match e {
                Element::Package(_) => cx.check_name(name)?,
                Element::Classifier(c) => cx.check_new_classifier(mm.classifier(c).package, name)?,
                Element::Feature(f) => cx.check_new_feature(mm.feature(f).owner, name)?,
                Element::Operation(_) => cx.check_name(name)?,
                Element::Literal(c, _) => {
                    cx.check_name(name)?;
                    if mm.enum_literal_index(c, name).is_some() {
                        return Err(Error::Invalid(format!(
                            "'{}' already has a literal named '{name}'",
                            mm.classifier(c).name
                        )));
                    }
                }
            }
            edits.extend(self.set_attr(el, "name", Some(name)));
            edits.extend(self.rename_references(cx, e, name));
            renamed = Some(match e {
                Element::Operation(_) => format!("{}.{name}()", element.rsplit_once('.').map_or("", |(o, _)| o)),
                Element::Feature(_) | Element::Literal(..) => {
                    member_id(element.rsplit_once('.').map_or("", |(o, _)| o), name)
                }
                Element::Classifier(_) | Element::Package(_) => match element.rsplit_once("::") {
                    Some((parent, _)) => classifier_id(parent, name),
                    None => name.clone(),
                },
            });
        }
        let class_only = |what: &str| match e {
            Element::Classifier(c) if mm.is_class(c) => Ok(()),
            _ => Err(Error::Invalid(format!("only classes can be {what}"))),
        };
        if let Some(v) = p.is_abstract {
            class_only("abstract")?;
            edits.extend(self.flag(el, "abstract", v, false));
        }
        if let Some(v) = p.is_interface {
            class_only("interfaces")?;
            edits.extend(self.flag(el, "interface", v, false));
        }
        let (reference, attribute, typed) = match e {
            Element::Feature(f) => (mm.feature(f).is_reference(), !mm.feature(f).is_reference(), true),
            Element::Operation(_) => (false, false, true),
            _ => (false, false, false),
        };
        if let Some(t) = &p.ty {
            if !typed {
                return Err(Error::Invalid(format!("'{element}' has no type")));
            }
            let value = if t.is_empty() {
                if !matches!(e, Element::Operation(_)) {
                    return Err(Error::Invalid("a feature needs a type".into()));
                }
                None
            } else if let Element::Feature(_) = e {
                Some(self.type_value(cx, cx.feature_type(t, reference)?))
            } else {
                Some(self.type_value(cx, cx.resolve_type(t)?))
            };
            match el.children_named("eGenericType").next() {
                Some(g) if el.attr("eType").is_none() => match &value {
                    Some(v) => edits.extend(self.set_attr(g, "eClassifier", Some(v))),
                    None => edits.push(ByteEdit::delete(removal(self.text, g.offset..g.end, None))),
                },
                _ => edits.extend(self.set_attr(el, "eType", value.as_deref())),
            }
        }
        if p.lower.is_some() || p.upper.is_some() {
            if !typed {
                return Err(Error::Invalid(format!("'{element}' has no multiplicity")));
            }
            if let Some(l) = p.lower {
                edits.extend(self.set_attr(el, "lowerBound", (l != 0).then(|| l.to_string()).as_deref()));
            }
            if let Some(u) = p.upper {
                edits.extend(self.set_attr(el, "upperBound", (u != 1).then(|| u.to_string()).as_deref()));
            }
        }
        let flags: [(&str, Option<bool>, bool, bool); 8] = [
            ("containment", p.containment, false, reference),
            ("ordered", p.ordered, true, typed),
            ("unique", p.unique, true, typed),
            ("derived", p.derived, false, reference || attribute),
            ("changeable", p.changeable, true, reference || attribute),
            ("transient", p.transient, false, reference || attribute),
            ("volatile", p.volatile, false, reference || attribute),
            ("iD", p.is_id, false, attribute),
        ];
        for (name, value, default, allowed) in flags {
            if let Some(v) = value {
                if !allowed {
                    return Err(Error::Invalid(format!("'{name}' does not apply to '{element}'")));
                }
                edits.extend(self.flag(el, name, v, default));
            }
        }
        if let Some(params) = &p.parameters {
            if !matches!(e, Element::Operation(_)) {
                return Err(Error::Invalid(format!("'{element}' is not an operation")));
            }
            for old in el.children_named("eParameters") {
                edits.push(ByteEdit::delete(removal(self.text, old.offset..old.end, None)));
            }
            let new = self.parameters(cx, params)?;
            if !new.is_empty() {
                let after = self.last_child(el, &["eTypeParameters", "eAnnotations"]);
                edits.push(self.insert_child(el, after, &new.join("\n")));
            }
        }
        Ok((edits, renamed))
    }
}

/// The fragment of an element, as the index of its root package and the names below it
/// (`None` for root packages, which fragments do not name).
fn fragment_path(mm: &Metamodel, resource: usize, e: Element) -> Option<(usize, Vec<String>)> {
    let package_path = |p: PackageId| {
        let mut names = Vec::new();
        let mut cur = p;
        while let Some(parent) = mm.package(cur).super_package {
            names.insert(0, mm.package(cur).name.clone());
            cur = parent;
        }
        let root = mm.resources()[resource].roots.iter().position(|r| *r == cur).unwrap_or(0);
        (root, names)
    };
    let in_class = |c: ClassifierId, member: Option<&str>| {
        let (root, mut names) = package_path(mm.classifier(c).package);
        names.push(mm.classifier(c).name.clone());
        names.extend(member.map(str::to_string));
        (root, names)
    };
    Some(match e {
        Element::Package(p) => {
            let (root, names) = package_path(p);
            if names.is_empty() {
                return None;
            }
            (root, names)
        }
        Element::Classifier(c) => in_class(c, None),
        Element::Feature(f) => in_class(mm.feature(f).owner, Some(&mm.feature(f).name)),
        Element::Operation(o) => in_class(mm.operation(o).owner, Some(&mm.operation(o).name)),
        Element::Literal(c, i) => in_class(c, Some(&mm.enum_literals(c)[i].name)),
    })
}

/// A same-file reference token (`#//A/b`) with the element at `path` renamed, if it goes
/// through that element.
fn rename_in_fragment(token: &str, root: usize, path: &[String], name: &str) -> Option<String> {
    let fragment = token.strip_prefix('#')?.strip_prefix('/')?;
    let (index, rest) = fragment.split_once('/')?;
    let index = if index.is_empty() { 0 } else { index.parse().ok()? };
    let mut segments: Vec<&str> = rest.split('/').collect();
    if index != root || segments.len() < path.len() || segments.iter().zip(path).any(|(s, p)| s != p) {
        return None;
    }
    segments[path.len() - 1] = name;
    Some(format!("#/{}/{}", if index == 0 { String::new() } else { index.to_string() }, segments.join("/")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames_fragments() {
        let path = vec!["Book".to_string()];
        assert_eq!(rename_in_fragment("#//Book/library", 0, &path, "Title").as_deref(), Some("#//Title/library"));
        assert_eq!(rename_in_fragment("#//Book", 0, &path, "Title").as_deref(), Some("#//Title"));
        assert_eq!(rename_in_fragment("#//Booking", 0, &path, "Title"), None);
        assert_eq!(rename_in_fragment("other.ecore#//Book", 0, &path, "Title"), None);
        assert_eq!(rename_in_fragment("#/1/Book", 0, &path, "Title"), None);
    }

    #[test]
    fn scans_start_tags() {
        let text = "<a x=\"1\"  y='2'/>";
        let root = xml::parse(text, "t.xml").unwrap();
        let t = tag(text, &root);
        assert!(t.empty);
        assert_eq!(t.attrs.len(), 2);
        assert_eq!(&text[t.attrs[1].value.clone()], "2");
        assert_eq!(t.end, text.len());
    }
}
