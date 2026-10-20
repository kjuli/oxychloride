//! Writing model resources as XMI documents, as EMF's `XMIResource` does.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::ecore_writer::relative_path;
use crate::metamodel::*;
use crate::model::*;
use crate::xml::{escape_attr, escape_text};

/// Whether a feature's values are written. Derived, transient and container references are
/// implied by other data; of a bidirectional pair of non-containment references only one
/// side is written (loading restores the other).
pub(crate) fn is_persistent(mm: &Metamodel, f: FeatureId) -> bool {
    let feature = mm.feature(f);
    if feature.derived || feature.transient {
        return false;
    }
    match feature.opposite() {
        Some(o) if mm.feature(o).is_containment() => false,
        Some(o) if !feature.is_containment() => {
            let other = mm.feature(o);
            f < o || other.derived || other.transient || !other.changeable
        }
        _ => true,
    }
}

/// The literal text of a data value.
pub(crate) fn value_text(mm: &Metamodel, v: &ModelValue) -> String {
    match v {
        ModelValue::Boolean(b) => b.to_string(),
        ModelValue::Integer(i) => i.to_string(),
        ModelValue::Real(r) if r.is_finite() && r.fract() == 0.0 && r.abs() < 1e15 => format!("{r:.1}"),
        ModelValue::Real(r) => r.to_string(),
        ModelValue::String(s) => s.clone(),
        ModelValue::Enum(e, i) => mm.enum_literals(*e).get(*i).map(|l| l.literal.clone()).unwrap_or_default(),
        ModelValue::Object(o) => format!("@{}", o.0),
    }
}

/// Serializes one resource of `model`. `schema_locations` maps nsURIs to the locations
/// written to `xsi:schemaLocation` (typically relative paths of `.ecore` files).
pub fn write_resource(mm: &Metamodel, model: &Model, resource: usize, schema_locations: &[(String, String)]) -> String {
    let roots = &model.resources()[resource].roots;
    let mut w = Writer { mm, model, resource, prefixes: HashMap::new(), used: Vec::new() };
    // Collect the packages of all objects first: their namespaces are declared on the root.
    for r in roots {
        w.prefix_of(model.class_of(*r));
        for o in model.all_contents(mm, *r) {
            w.prefix_of(model.class_of(o));
        }
    }
    let mut namespaces = String::from(
        r#" xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance""#,
    );
    for (prefix, p) in w.used.clone() {
        let _ = write!(namespaces, " xmlns:{prefix}=\"{}\"", escape_attr(&mm.package(p).ns_uri));
    }
    let locations: Vec<String> = w
        .used
        .iter()
        .filter_map(|(_, p)| {
            let ns = &mm.package(*p).ns_uri;
            schema_locations.iter().find(|(n, _)| n == ns).map(|(n, l)| format!("{n} {l}"))
        })
        .collect();
    if !locations.is_empty() {
        let _ = write!(namespaces, " xsi:schemaLocation=\"{}\"", escape_attr(&locations.join(" ")));
    }
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    if roots.len() == 1 {
        let tag = w.qualified_class(model.class_of(roots[0]));
        w.object(&mut out, roots[0], &tag, None, &namespaces, 0);
    } else {
        let _ = writeln!(out, "<xmi:XMI{namespaces}>");
        for r in roots {
            let tag = w.qualified_class(model.class_of(*r));
            w.object(&mut out, *r, &tag, None, "", 1);
        }
        out.push_str("</xmi:XMI>\n");
    }
    out
}

struct Writer<'a> {
    mm: &'a Metamodel,
    model: &'a Model,
    resource: usize,
    prefixes: HashMap<PackageId, String>,
    used: Vec<(String, PackageId)>,
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

impl Writer<'_> {
    fn prefix_of(&mut self, class: ClassifierId) -> String {
        let package = self.mm.classifier(class).package;
        if let Some(p) = self.prefixes.get(&package) {
            return p.clone();
        }
        let pkg = self.mm.package(package);
        let base = if pkg.ns_prefix.is_empty() { pkg.name.clone() } else { pkg.ns_prefix.clone() };
        let base = if base.is_empty() || base.starts_with("xml") { "p".to_string() } else { base };
        let mut prefix = base.clone();
        let mut n = 1;
        while self.used.iter().any(|(p, _)| *p == prefix) || prefix == "xmi" || prefix == "xsi" {
            prefix = format!("{base}{n}");
            n += 1;
        }
        self.prefixes.insert(package, prefix.clone());
        self.used.push((prefix.clone(), package));
        prefix
    }

    fn qualified_class(&mut self, class: ClassifierId) -> String {
        format!("{}:{}", self.prefix_of(class), self.mm.classifier(class).name)
    }

    /// A reference to `target` from this resource: a fragment, or `uri#fragment` for
    /// objects in other resources.
    fn href(&self, target: ObjectId) -> String {
        let obj = self.model.object(target);
        let fragment = obj.xmi_id.clone().unwrap_or_else(|| self.model.uri_fragment(self.mm, target));
        if obj.resource == self.resource {
            return fragment;
        }
        let own = &self.model.resources()[self.resource].uri;
        let other = &self.model.resources()[obj.resource].uri;
        let location =
            if other.contains(':') && !other.starts_with('/') { other.clone() } else { relative_path(own, other) };
        format!("{location}#{fragment}")
    }

    fn object(
        &mut self,
        out: &mut String,
        o: ObjectId,
        tag: &str,
        xsi_type: Option<String>,
        extra: &str,
        depth: usize,
    ) {
        let mm = self.mm;
        let model = self.model;
        indent(out, depth);
        let _ = write!(out, "<{tag}{extra}");
        if let Some(t) = xsi_type {
            let _ = write!(out, " xsi:type=\"{t}\"");
        }
        if let Some(id) = &model.object(o).xmi_id {
            let _ = write!(out, " xmi:id=\"{}\"", escape_attr(id));
        }
        let mut children = String::new();
        for &f in mm.all_features(model.class_of(o)) {
            let values = model.get(o, f);
            if values.is_empty() || !is_persistent(mm, f) {
                continue;
            }
            let feature = mm.feature(f);
            if feature.is_containment() {
                for v in values {
                    let ModelValue::Object(child) = v else { continue };
                    let class = model.class_of(*child);
                    let xsi = (Some(class) != feature.ty).then(|| self.qualified_class(class));
                    self.object(&mut children, *child, &feature.name, xsi, "", depth + 1);
                }
            } else if feature.is_reference() {
                let mut local = Vec::new();
                for v in values {
                    let ModelValue::Object(t) = v else { continue };
                    let href = self.href(*t);
                    if model.object(*t).resource == self.resource {
                        local.push(href);
                    } else {
                        indent(&mut children, depth + 1);
                        let _ = writeln!(children, "<{} href=\"{}\"/>", feature.name, escape_attr(&href));
                    }
                }
                if !local.is_empty() {
                    let _ = write!(out, " {}=\"{}\"", feature.name, escape_attr(&local.join(" ")));
                }
            } else if feature.is_many() {
                for v in values {
                    indent(&mut children, depth + 1);
                    let _ = writeln!(children, "<{0}>{1}</{0}>", feature.name, escape_text(&value_text(mm, v)));
                }
            } else {
                let _ = write!(out, " {}=\"{}\"", feature.name, escape_attr(&value_text(mm, &values[0])));
            }
        }
        if children.is_empty() {
            out.push_str("/>\n");
        } else {
            out.push_str(">\n");
            out.push_str(&children);
            indent(out, depth);
            let _ = writeln!(out, "</{tag}>");
        }
    }
}
