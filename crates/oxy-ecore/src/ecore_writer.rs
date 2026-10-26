//! Writing the packages of a [`Metamodel`] resource as an `.ecore` document (EMF's XMI
//! serialization of `EPackage`s), so metamodels built in other ways can be exported.

use std::fmt::Write as _;
use std::path::{Component, Path};

use crate::metamodel::*;
use crate::xml::escape_attr;

const HEADER: &str = r#"xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    xmlns:ecore="http://www.eclipse.org/emf/2002/Ecore""#;

/// Serializes the root packages of a resource. References to elements of other resources
/// use a path relative to the resource when both are files, and the target package's nsURI
/// otherwise.
pub fn write_resource(mm: &Metamodel, resource: usize) -> String {
    let roots = &mm.resources()[resource].roots;
    let w = Writer { mm, resource };
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    if roots.len() == 1 {
        w.package(&mut out, roots[0], 0, true);
    } else {
        let _ = writeln!(out, "<xmi:XMI {HEADER}>");
        for p in roots {
            w.package(&mut out, *p, 1, true);
        }
        out.push_str("</xmi:XMI>\n");
    }
    out
}

struct Writer<'a> {
    mm: &'a Metamodel,
    resource: usize,
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

/// Appends ` name="value"`.
fn attr(out: &mut String, name: &str, value: &str) {
    let _ = write!(out, " {name}=\"{}\"", escape_attr(value));
}

fn flag(out: &mut String, name: &str, value: bool, default: bool) {
    if value != default {
        attr(out, name, if value { "true" } else { "false" });
    }
}

impl Writer<'_> {
    fn package(&self, out: &mut String, p: PackageId, depth: usize, root: bool) {
        let mm = self.mm;
        let pkg = mm.package(p);
        indent(out, depth);
        if root {
            out.push_str("<ecore:EPackage ");
            out.push_str(HEADER);
        } else {
            out.push_str("<eSubpackages");
        }
        attr(out, "name", &pkg.name);
        attr(out, "nsURI", &pkg.ns_uri);
        attr(out, "nsPrefix", &pkg.ns_prefix);
        let tag = if root { "ecore:EPackage" } else { "eSubpackages" };
        if pkg.annotations.is_empty() && pkg.classifiers.is_empty() && pkg.subpackages.is_empty() {
            out.push_str("/>\n");
            return;
        }
        out.push_str(">\n");
        self.annotations(out, &pkg.annotations, depth + 1);
        for c in &pkg.classifiers {
            self.classifier(out, *c, depth + 1);
        }
        for s in &pkg.subpackages {
            self.package(out, *s, depth + 1, false);
        }
        indent(out, depth);
        let _ = writeln!(out, "</{tag}>");
    }

    fn annotations(&self, out: &mut String, annotations: &[Annotation], depth: usize) {
        for a in annotations {
            indent(out, depth);
            out.push_str("<eAnnotations");
            attr(out, "source", &a.source);
            if a.details.is_empty() {
                out.push_str("/>\n");
                continue;
            }
            out.push_str(">\n");
            for (k, v) in &a.details {
                indent(out, depth + 1);
                out.push_str("<details");
                attr(out, "key", k);
                attr(out, "value", v);
                out.push_str("/>\n");
            }
            indent(out, depth);
            out.push_str("</eAnnotations>\n");
        }
    }

    fn classifier(&self, out: &mut String, c: ClassifierId, depth: usize) {
        let mm = self.mm;
        let cl = mm.classifier(c);
        indent(out, depth);
        out.push_str("<eClassifiers");
        let kind = match &cl.kind {
            ClassifierKind::Class(_) => "ecore:EClass",
            ClassifierKind::DataType(_) => "ecore:EDataType",
            ClassifierKind::Enum(_) => "ecore:EEnum",
        };
        attr(out, "xsi:type", kind);
        attr(out, "name", &cl.name);
        if let Some(icn) = &cl.instance_class_name {
            attr(out, "instanceClassName", icn);
        }
        let mut children = String::new();
        self.annotations(&mut children, &cl.annotations, depth + 1);
        match &cl.kind {
            ClassifierKind::Class(data) => {
                flag(out, "abstract", data.is_abstract, false);
                flag(out, "interface", data.is_interface, false);
                if !data.super_types.is_empty() {
                    let supers: Vec<String> =
                        data.super_types.iter().map(|s| self.href(ElementRef::Classifier(*s))).collect();
                    attr(out, "eSuperTypes", &supers.join(" "));
                }
                for o in &data.operations {
                    self.operation(&mut children, *o, depth + 1);
                }
                for f in &data.features {
                    self.feature(&mut children, *f, depth + 1);
                }
            }
            ClassifierKind::Enum(literals) => {
                for (i, l) in literals.iter().enumerate() {
                    indent(&mut children, depth + 1);
                    children.push_str("<eLiterals");
                    attr(&mut children, "name", &l.name);
                    if l.value != i as i64 {
                        attr(&mut children, "value", &l.value.to_string());
                    }
                    if l.literal != l.name {
                        attr(&mut children, "literal", &l.literal);
                    }
                    children.push_str("/>\n");
                }
            }
            ClassifierKind::DataType(_) => {}
        }
        if children.is_empty() {
            out.push_str("/>\n");
        } else {
            out.push_str(">\n");
            out.push_str(&children);
            indent(out, depth);
            out.push_str("</eClassifiers>\n");
        }
    }

    fn typed(&self, out: &mut String, ty: Option<ClassifierId>, m: &Multiplicity) {
        flag(out, "ordered", m.ordered, true);
        flag(out, "unique", m.unique, true);
        if m.lower != 0 {
            attr(out, "lowerBound", &m.lower.to_string());
        }
        if m.upper != 1 {
            attr(out, "upperBound", &m.upper.to_string());
        }
        if let Some(t) = ty {
            attr(out, "eType", &self.type_href(t));
        }
    }

    fn feature(&self, out: &mut String, f: FeatureId, depth: usize) {
        let feature = self.mm.feature(f);
        indent(out, depth);
        out.push_str("<eStructuralFeatures");
        let is_ref = feature.is_reference();
        attr(out, "xsi:type", if is_ref { "ecore:EReference" } else { "ecore:EAttribute" });
        attr(out, "name", &feature.name);
        self.typed(out, feature.ty, &feature.multiplicity);
        flag(out, "changeable", feature.changeable, true);
        flag(out, "volatile", feature.volatile, false);
        flag(out, "transient", feature.transient, false);
        if let Some(d) = &feature.default_value_literal {
            attr(out, "defaultValueLiteral", d);
        }
        flag(out, "derived", feature.derived, false);
        match feature.kind {
            FeatureKind::Attribute { is_id } => flag(out, "iD", is_id, false),
            FeatureKind::Reference { containment, opposite } => {
                flag(out, "containment", containment, false);
                if let Some(o) = opposite {
                    attr(out, "eOpposite", &self.href(ElementRef::Feature(o)));
                }
            }
        }
        if feature.annotations.is_empty() {
            out.push_str("/>\n");
        } else {
            out.push_str(">\n");
            self.annotations(out, &feature.annotations, depth + 1);
            indent(out, depth);
            out.push_str("</eStructuralFeatures>\n");
        }
    }

    fn operation(&self, out: &mut String, o: OperationId, depth: usize) {
        let op = self.mm.operation(o);
        indent(out, depth);
        out.push_str("<eOperations");
        attr(out, "name", &op.name);
        self.typed(out, op.ty, &op.multiplicity);
        if op.annotations.is_empty() && op.params.is_empty() {
            out.push_str("/>\n");
            return;
        }
        out.push_str(">\n");
        self.annotations(out, &op.annotations, depth + 1);
        for p in &op.params {
            indent(out, depth + 1);
            out.push_str("<eParameters");
            attr(out, "name", &p.name);
            self.typed(out, p.ty, &p.multiplicity);
            out.push_str("/>\n");
        }
        indent(out, depth);
        out.push_str("</eOperations>\n");
    }

    fn type_href(&self, t: ClassifierId) -> String {
        type_href(self.mm, self.resource, t)
    }

    fn href(&self, target: ElementRef) -> String {
        href(self.mm, self.resource, target)
    }
}

/// An `eType` value in `resource`: cross-document references carry a type hint, as EMF writes
/// them (`ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EString`).
pub fn type_href(mm: &Metamodel, resource: usize, t: ClassifierId) -> String {
    let href = href(mm, resource, ElementRef::Classifier(t));
    if href.starts_with('#') {
        return href;
    }
    let hint = match mm.classifier(t).kind {
        ClassifierKind::Class(_) => "ecore:EClass",
        ClassifierKind::DataType(_) => "ecore:EDataType",
        ClassifierKind::Enum(_) => "ecore:EEnum",
    };
    format!("{hint} {href}")
}

/// A reference from `resource` to a classifier or feature: `#//Class/feature` within the
/// resource, else a relative path or the package's nsURI before the `#`.
pub fn href(mm: &Metamodel, resource: usize, target: ElementRef) -> String {
    let (package, mut path) = match target {
        ElementRef::Classifier(c) => (mm.classifier(c).package, vec![mm.classifier(c).name.clone()]),
        ElementRef::Feature(f) => {
            let feature = mm.feature(f);
            let owner = mm.classifier(feature.owner);
            (owner.package, vec![owner.name.clone(), feature.name.clone()])
        }
        ElementRef::Operation(o) => {
            let op = mm.operation(o);
            let owner = mm.classifier(op.owner);
            (owner.package, vec![owner.name.clone(), op.name.clone()])
        }
        ElementRef::Package(p) => (p, Vec::new()),
    };
    let mut root = package;
    while let Some(parent) = mm.package(root).super_package {
        path.insert(0, mm.package(root).name.clone());
        root = parent;
    }
    let target_resource = mm.package(root).resource;
    let index = mm.resources()[target_resource].roots.iter().position(|r| *r == root).unwrap_or(0);
    let fragment = format!("/{}/{}", if index == 0 { String::new() } else { index.to_string() }, path.join("/"));
    if target_resource == resource {
        return format!("#{fragment}");
    }
    let ns_uri = &mm.package(root).ns_uri;
    let target_uri = &mm.resources()[target_resource].uri;
    let own_uri = &mm.resources()[resource].uri;
    if is_ecore_file(target_uri) && is_file(own_uri) && ns_uri != ECORE_NS_URI {
        return format!("{}#{fragment}", relative_path(own_uri, target_uri));
    }
    format!("{ns_uri}#{fragment}")
}

fn is_file(uri: &str) -> bool {
    !uri.contains("://") && !uri.starts_with("platform:")
}

fn is_ecore_file(uri: &str) -> bool {
    is_file(uri) && uri.ends_with(".ecore")
}

/// `to` relative to the directory of `from` (both file paths), with `/` separators.
pub fn relative_path(from: &str, to: &str) -> String {
    let from_dir: Vec<Component> = Path::new(from).parent().map(|p| p.components().collect()).unwrap_or_default();
    let to_parts: Vec<Component> = Path::new(to).components().collect();
    let common = from_dir.iter().zip(&to_parts).take_while(|(a, b)| a == b).count();
    if common == 0 && Path::new(to).is_absolute() {
        return to.replace('\\', "/");
    }
    let mut parts: Vec<String> = vec!["..".to_string(); from_dir.len() - common];
    parts.extend(to_parts[common..].iter().map(|c| c.as_os_str().to_string_lossy().into_owned()));
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths() {
        assert_eq!(relative_path("/a/b/x.ecore", "/a/b/y.ecore"), "y.ecore");
        assert_eq!(relative_path("/a/b/x.ecore", "/a/c/y.ecore"), "../c/y.ecore");
        assert_eq!(relative_path("x.ecore", "y.ecore"), "y.ecore");
    }
}
