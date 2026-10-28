//! OCLinEcore: Ecore metamodels written as text, with embedded OCL.
//!
//! [`build`] turns a parsed document into packages of a [`Metamodel`] and lists the embedded
//! constraints with their positions in the document, which
//! [`crate::EnvironmentBuilder::add_oclinecore`] type checks. The OCL is also stored in the
//! metamodel as Pivot annotations, the way Eclipse OCL stores it in `.ecore` files, so a
//! built document can be exported with [`Metamodel::write_ecore`]. [`print`] goes the other
//! way: it renders a loaded resource as OCLinEcore text.
//!
//! Defaults follow Eclipse OCL: a missing multiplicity means `[1]`; multi-valued features
//! are unordered and unique unless qualified with `ordered`/`!unique` (or typed with a
//! collection type such as `OrderedSet(Book)`); `Integer` and `Real` are mapped to
//! `EBigInteger` and `EBigDecimal`.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;
use std::rc::Rc;

use oxy_ecore::{
    Annotation, ClassData, Classifier, ClassifierId, ClassifierKind, DataTypeKind, ECORE_ANNOTATION_SOURCE,
    ECORE_NS_URI, ElementRef, EnumLiteral, Feature, FeatureId, FeatureKind, Metamodel, Multiplicity,
    OCL_ANNOTATION_SOURCES, Operation, OperationId, Package, PackageId, Parameter, UNBOUNDED, parse_value,
};

use crate::diagnostic::Diagnostic;
use crate::env::{contract_key, is_invariant_operation, lookup_classifier, split_status_tuple};
use crate::span::Span;
use crate::syntax::ast::*;
use crate::syntax::{GENMODEL_SOURCE, parse_ecore_document, parse_expression};

/// The annotation source the OCL of built documents is stored under.
pub const PIVOT_SOURCE: &str = OCL_ANNOTATION_SOURCES[0];

/// OCL primitive types and the Ecore data types that represent them.
const PRIMITIVES: &[(&str, &str)] = &[
    ("Boolean", "EBoolean"),
    ("Integer", "EBigInteger"),
    ("Real", "EBigDecimal"),
    ("String", "EString"),
    ("UnlimitedNatural", "EBigInteger"),
];

/// A constraint of a document and the element it belongs to: a class (invariants), a
/// feature (`initial`, `derivation`) or an operation (`precondition`, `postcondition`,
/// `body`).
#[derive(Clone, Debug)]
pub struct EmbeddedConstraint {
    pub owner: ElementRef,
    /// The package unqualified type names in the constraint are resolved in.
    pub package: PackageId,
    pub constraint: Constraint,
}

/// A built OCLinEcore document.
#[derive(Clone, Debug)]
pub struct OclInEcore {
    pub document: EcoreDocument,
    /// The metamodel resource holding the document's packages.
    pub resource: usize,
    pub packages: Vec<PackageId>,
    pub constraints: Vec<EmbeddedConstraint>,
    /// Every element defined by the document, with the span of its definition.
    pub elements: Vec<(ElementRef, Span)>,
    /// Syntax errors and problems building the metamodel.
    pub diagnostics: Vec<Diagnostic>,
}

impl OclInEcore {
    /// The innermost element whose definition contains `offset`.
    pub fn element_at(&self, offset: u32) -> Option<ElementRef> {
        self.elements
            .iter()
            .filter(|(_, s)| s.start <= offset && offset <= s.end)
            .min_by_key(|(_, s)| s.end - s.start)
            .map(|(e, _)| *e)
    }
}

/// Parses `text` and builds it into `mm`, loading imported `.ecore` files (relative to
/// `uri`) that are not loaded yet.
pub fn load(mm: &mut Metamodel, uri: &str, text: &str) -> OclInEcore {
    let (document, mut diagnostics) = parse_ecore_document(text);
    for import in &document.imports {
        let target = import.uri.split('#').next().unwrap_or("");
        if target.is_empty() || target == ECORE_NS_URI || mm.package_by_ns_uri(target).is_some() {
            continue;
        }
        let path = oxy_ecore::ecore_loader::resolve_relative(uri, target);
        if mm.resource_of_uri(&path).is_some() {
            continue;
        }
        if !Path::new(&path).exists() {
            diagnostics
                .push(Diagnostic::error(import.span, format!("cannot find imported metamodel '{}'", import.uri)));
            continue;
        }
        if let Err(e) = mm.load_ecore_file(&path) {
            diagnostics.push(Diagnostic::error(import.span, e.to_string()));
        }
    }
    build(mm, uri, text, document, diagnostics)
}

/// Builds a parsed document into `mm` as a new resource named `uri`. Imports must already
/// be loaded; `diagnostics` (the syntax errors) are passed through. Problems such as
/// unknown types are reported; the rest of the document is still built.
pub fn build(
    mm: &mut Metamodel,
    uri: &str,
    text: &str,
    document: EcoreDocument,
    diagnostics: Vec<Diagnostic>,
) -> OclInEcore {
    let resource = mm.add_resource(uri);
    let mut b = Builder {
        mm,
        uri,
        text,
        resource,
        diags: diagnostics,
        aliases: HashMap::new(),
        constraints: Vec::new(),
        elements: Vec::new(),
        classes: Vec::new(),
        features: Vec::new(),
        ns_uris: HashSet::new(),
    };
    b.resolve_aliases(&document.imports);
    let mut roots = Vec::new();
    for p in &document.packages {
        if let Some(id) = b.declare_package(p, None) {
            roots.push(id);
        }
    }
    for (c, def, package) in b.classes.clone() {
        b.class_members(c, def, package);
    }
    b.mm.finish_resource(resource, roots.clone());
    for (f, def) in b.features.clone() {
        if let Some(opposite) = &def.opposite {
            b.opposite(f, opposite);
        }
    }
    b.store_annotations();
    let Builder { diags, constraints, elements, .. } = b;
    OclInEcore { document, resource, packages: roots, constraints, elements, diagnostics: diags }
}

struct Builder<'a, 'd> {
    mm: &'a mut Metamodel,
    uri: &'a str,
    text: &'a str,
    resource: usize,
    diags: Vec<Diagnostic>,
    aliases: HashMap<String, PackageId>,
    constraints: Vec<EmbeddedConstraint>,
    elements: Vec<(ElementRef, Span)>,
    classes: Vec<(ClassifierId, &'d ClassifierDef, PackageId)>,
    features: Vec<(FeatureId, &'d FeatureDef)>,
    ns_uris: HashSet<String>,
}

impl<'d> Builder<'_, 'd> {
    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diags.push(Diagnostic::error(span, message));
    }

    fn resolve_aliases(&mut self, imports: &[Import]) {
        for import in imports {
            let Some(alias) = &import.alias else { continue };
            let target = import.uri.split('#').next().unwrap_or("");
            let package = self.mm.package_by_ns_uri(target).or_else(|| {
                let path = oxy_ecore::ecore_loader::resolve_relative(self.uri, target);
                let resource = self.mm.resource_of_uri(&path)?;
                self.mm.resources()[resource].roots.first().copied()
            });
            if let Some(p) = package {
                self.aliases.insert(alias.name.clone(), p);
            }
        }
    }

    // ------------------------------------------------------------ pass 1: declarations

    fn declare_package(&mut self, def: &'d PackageDef, parent: Option<PackageId>) -> Option<PackageId> {
        let ns_uri = def.ns_uri.as_ref().map(|(u, _)| u.clone()).unwrap_or_default();
        if !ns_uri.is_empty() {
            let span = def.ns_uri.as_ref().map(|(_, s)| *s).unwrap_or(def.name.span);
            if let Some(existing) = self.mm.package_by_ns_uri(&ns_uri) {
                let from = self.mm.resources()[self.mm.package(existing).resource].uri.clone();
                self.error(span, format!("a package with nsURI '{ns_uri}' is already loaded (from '{from}')"));
                return None;
            }
            if !self.ns_uris.insert(ns_uri.clone()) {
                self.error(span, format!("another package of this document has nsURI '{ns_uri}'"));
                return None;
            }
        }
        let annotations = def.annotations.iter().map(annotation).collect();
        let id = self.mm.add_package(Package {
            name: def.name.name.clone(),
            ns_uri,
            ns_prefix: def.ns_prefix.as_ref().map_or_else(|| def.name.name.clone(), |p| p.name.clone()),
            super_package: parent,
            subpackages: Vec::new(),
            classifiers: Vec::new(),
            annotations,
            resource: self.resource,
        });
        self.elements.push((ElementRef::Package(id), def.span));
        let mut names = HashSet::new();
        for c in &def.classifiers {
            if !names.insert(c.name.name.clone()) {
                self.error(c.name.span, format!("package '{}' already defines '{}'", def.name, c.name));
                continue;
            }
            let kind = match &c.kind {
                ClassifierDefKind::Class { is_abstract, is_interface, .. } => ClassifierKind::Class(ClassData {
                    is_abstract: *is_abstract,
                    is_interface: *is_interface,
                    ..ClassData::default()
                }),
                ClassifierDefKind::DataType => {
                    ClassifierKind::DataType(DataTypeKind::from_instance_class(c.instance_class.as_deref()))
                }
                ClassifierDefKind::Enum { literals } => {
                    let mut seen = HashSet::new();
                    let mut out = Vec::new();
                    for (i, l) in literals.iter().enumerate() {
                        if !seen.insert(l.name.name.clone()) {
                            self.error(l.name.span, format!("duplicate literal '{}'", l.name));
                            continue;
                        }
                        out.push(EnumLiteral {
                            name: l.name.name.clone(),
                            value: l.value.unwrap_or(i as i64),
                            literal: l.literal.clone().unwrap_or_else(|| l.name.name.clone()),
                        });
                    }
                    ClassifierKind::Enum(out)
                }
            };
            let cid = self.mm.add_classifier(Classifier {
                name: c.name.name.clone(),
                package: id,
                instance_class_name: c.instance_class.clone(),
                annotations: c.annotations.iter().map(annotation).collect(),
                kind,
            });
            self.elements.push((ElementRef::Classifier(cid), c.span));
            if matches!(c.kind, ClassifierDefKind::Class { .. }) {
                self.classes.push((cid, c, id));
            }
        }
        for sub in &def.packages {
            self.declare_package(sub, Some(id));
        }
        Some(id)
    }

    // ------------------------------------------------------------ pass 2: members

    /// Resolves a type name: primitive types, then the enclosing packages, then import
    /// aliases, then all loaded packages.
    fn resolve_type(&mut self, t: &TypeRef, package: PackageId) -> Option<ClassifierId> {
        let names: Vec<&str> = t.path.iter().map(|i| i.name.as_str()).collect();
        if names.iter().any(|n| n.is_empty()) {
            return None; // a syntax error was reported
        }
        if let [single] = names.as_slice() {
            if let Some((_, ecore)) = PRIMITIVES.iter().find(|(p, _)| p == single) {
                return self.mm.find_classifier_in(self.mm.ecore_package(), ecore);
            }
            if matches!(*single, "OclAny" | "OclVoid" | "OclInvalid" | "OclElement") {
                self.error(t.span, format!("'{single}' cannot be used as the type of an Ecore element"));
                return None;
            }
            let mut scope = Some(package);
            while let Some(p) = scope {
                if let Some(c) = self.mm.find_classifier_in(p, single) {
                    return Some(c);
                }
                scope = self.mm.package(p).super_package;
            }
        }
        if let [alias, rest @ ..] = names.as_slice()
            && !rest.is_empty()
            && let Some(p) = self.aliases.get(*alias)
            && let Some(ElementRef::Classifier(c)) = self.mm.resolve_in_package(*p, &rest.join("/"))
        {
            return Some(c);
        }
        match lookup_classifier(self.mm, Some(package), &names) {
            Ok(c) => Some(c),
            Err(e) => {
                self.error(t.span, e);
                None
            }
        }
    }

    /// The multiplicity of a typed element: explicit, implied by a collection type, or `[1]`.
    fn multiplicity(
        &mut self,
        ty: Option<&TypeRef>,
        m: Option<MultiplicityDef>,
        qualifiers: &[Qualifier],
    ) -> Multiplicity {
        let collection = ty.and_then(|t| t.collection);
        let (lower, upper) = match (m, collection) {
            (Some(m), _) => (m.lower, if m.upper < 0 { UNBOUNDED } else { m.upper }),
            (None, Some(_)) => (0, UNBOUNDED),
            (None, None) => (1, 1),
        };
        let mut out = Multiplicity { lower, upper, ordered: true, unique: true };
        if out.is_many() {
            out.ordered = collection.is_some_and(|k| k.is_ordered());
            out.unique = collection.is_none_or(|k| k.is_unique() || k == CollectionKind::Collection);
        }
        for q in qualifiers {
            match q.name.as_str() {
                "ordered" => out.ordered = !q.negated,
                "unique" => out.unique = !q.negated,
                _ => {}
            }
        }
        out
    }

    fn class_members(&mut self, class: ClassifierId, def: &'d ClassifierDef, package: PackageId) {
        let ClassifierDefKind::Class { supers, features, operations, invariants, .. } = &def.kind else { return };
        let mut super_types = Vec::new();
        for s in supers {
            let Some(t) = self.resolve_type(s, package) else { continue };
            if !self.mm.is_class(t) {
                let name = self.mm.classifier(t).name.clone();
                self.error(s.span, format!("'{name}' is not a class and cannot be a supertype"));
            } else if t == class {
                self.error(s.span, "a class cannot extend itself");
            } else if !super_types.contains(&t) {
                super_types.push(t);
            }
        }
        if let ClassifierKind::Class(data) = &mut self.mm.classifier_mut(class).kind {
            data.super_types = super_types;
        }
        let mut names = HashSet::new();
        for f in features {
            if !names.insert(f.name.name.clone()) {
                self.error(f.name.span, format!("'{}' is already defined in this class", f.name));
                continue;
            }
            self.feature(class, f, package);
        }
        for o in operations {
            self.operation(class, o, package);
        }
        for inv in invariants {
            self.constraints.push(EmbeddedConstraint {
                owner: ElementRef::Classifier(class),
                package,
                constraint: inv.clone(),
            });
        }
    }

    fn feature(&mut self, class: ClassifierId, def: &'d FeatureDef, package: PackageId) {
        let ty = def.ty.as_ref().and_then(|t| self.resolve_type(t, package));
        let is_reference = match ty {
            Some(t) => self.mm.is_class(t),
            None => def.keyword == FeatureKeyword::Property,
        };
        if is_reference && def.keyword == FeatureKeyword::Attribute && ty.is_some() {
            self.diags.push(Diagnostic::warning(
                def.name.span,
                format!("'{}' has a class type, so it is a reference; declare it with 'property'", def.name),
            ));
        }
        let mut flags = HashMap::new();
        for q in &def.qualifiers {
            let applicable = match q.name.as_str() {
                "composes" | "resolve" => is_reference,
                "id" => !is_reference,
                n => crate::syntax::FEATURE_QUALIFIERS.contains(&n),
            };
            if !applicable {
                self.error(
                    q.span,
                    format!(
                        "'{}' is not a qualifier of {}",
                        q.name,
                        if is_reference { "references" } else { "attributes" }
                    ),
                );
                continue;
            }
            if q.name == "static" {
                self.diags.push(Diagnostic::warning(q.span, "'static' is ignored"));
            }
            flags.insert(q.name.as_str(), !q.negated);
        }
        let flag = |n: &str| flags.get(n).copied().unwrap_or(false);
        let kind = if is_reference {
            FeatureKind::Reference { containment: flag("composes"), opposite: None }
        } else {
            FeatureKind::Attribute { is_id: flag("id") }
        };
        if let (Some(d), Some(t)) = (&def.default, ty)
            && !self.mm.is_class(t)
            && let Err(e) = parse_value(self.mm, t, d)
        {
            self.error(def.span, format!("invalid default value: {e}"));
        }
        let multiplicity = self.multiplicity(def.ty.as_ref(), def.multiplicity, &def.qualifiers);
        let f = self.mm.add_feature(Feature {
            name: def.name.name.clone(),
            owner: class,
            kind,
            ty,
            multiplicity,
            derived: flag("derived"),
            transient: flag("transient"),
            volatile: flag("volatile"),
            changeable: !flag("readonly"),
            default_value_literal: def.default.clone(),
            annotations: def.annotations.iter().map(annotation).collect(),
        });
        self.elements.push((ElementRef::Feature(f), def.span));
        self.features.push((f, def));
        let mut seen = HashSet::new();
        for c in &def.constraints {
            if !seen.insert(c.kind) {
                self.error(c.span, format!("'{}' already has {} expression", def.name, c.kind.keyword()));
                continue;
            }
            self.constraints.push(EmbeddedConstraint { owner: ElementRef::Feature(f), package, constraint: c.clone() });
        }
    }

    fn operation(&mut self, class: ClassifierId, def: &'d OperationDef, package: PackageId) {
        let mut params = Vec::new();
        let mut names = HashSet::new();
        for p in &def.params {
            if !names.insert(p.name.name.clone()) {
                self.error(p.name.span, format!("duplicate parameter '{}'", p.name));
            }
            let ty = match &p.ty {
                Some(t) => self.resolve_type(t, package),
                None => {
                    self.error(p.name.span, format!("parameter '{}' needs a type", p.name));
                    None
                }
            };
            for q in &p.qualifiers {
                if !crate::syntax::PARAMETER_QUALIFIERS.contains(&q.name.as_str()) {
                    self.error(q.span, format!("'{}' is not a qualifier of parameters", q.name));
                }
            }
            let multiplicity = self.multiplicity(p.ty.as_ref(), p.multiplicity, &p.qualifiers);
            params.push(Parameter { name: p.name.name.clone(), ty, multiplicity });
        }
        let ty = def.ty.as_ref().and_then(|t| self.resolve_type(t, package));
        for q in &def.qualifiers {
            if !crate::syntax::OPERATION_QUALIFIERS.contains(&q.name.as_str()) {
                self.error(q.span, format!("'{}' is not a qualifier of operations", q.name));
            }
        }
        let multiplicity = if def.ty.is_some() {
            self.multiplicity(def.ty.as_ref(), def.multiplicity, &def.qualifiers)
        } else {
            Multiplicity::default()
        };
        let o = self.mm.add_operation(Operation {
            name: def.name.name.clone(),
            owner: class,
            ty,
            multiplicity,
            params,
            annotations: def.annotations.iter().map(annotation).collect(),
        });
        self.elements.push((ElementRef::Operation(o), def.span));
        let mut has_body = false;
        for c in &def.constraints {
            if c.kind == ConstraintKind::Body {
                if has_body {
                    self.error(c.span, format!("operation '{}' already has a body", def.name));
                    continue;
                }
                has_body = true;
            }
            self.constraints.push(EmbeddedConstraint {
                owner: ElementRef::Operation(o),
                package,
                constraint: c.clone(),
            });
        }
    }

    // ------------------------------------------------------------ pass 3: opposites

    fn opposite(&mut self, f: FeatureId, name: &Ident) {
        let o = match self.check_opposite(f, &name.name) {
            Ok(Some(o)) => o,
            Ok(None) => return,
            Err(problem) => {
                self.error(name.span, problem);
                return;
            }
        };
        let set = |kind: &mut FeatureKind, to: FeatureId| {
            if let FeatureKind::Reference { opposite, .. } = kind {
                *opposite = Some(to);
            }
        };
        set(&mut self.mm.feature_mut(f).kind, o);
        set(&mut self.mm.feature_mut(o).kind, f);
    }

    /// The feature `name` names as the opposite of `f` (`None` if `f`'s type is unknown).
    fn check_opposite(&self, f: FeatureId, name: &str) -> Result<Option<FeatureId>, String> {
        let mm = &*self.mm;
        let feature = mm.feature(f);
        let Some(target) = feature.ty else { return Ok(None) };
        if !feature.is_reference() {
            return Err("only references can have an opposite".into());
        }
        let Some(o) = mm.lookup_feature(target, name) else {
            return Err(format!("class '{}' has no property '{name}'", mm.classifier(target).name));
        };
        let other = mm.feature(o);
        let owner = feature.owner;
        if !other.is_reference() {
            Err(format!("'{name}' is not a reference"))
        } else if let Some(t) = other.ty.filter(|t| !mm.conforms_to(owner, *t)) {
            Err(format!("'{name}' refers to {}, not to {}", mm.classifier(t).name, mm.classifier(owner).name))
        } else if other.opposite().is_some_and(|x| x != f) {
            Err(format!("'{name}' already has a different opposite"))
        } else if feature.is_containment() && other.is_containment() {
            Err("a reference and its opposite cannot both be containments".into())
        } else {
            Ok(Some(o))
        }
    }

    // ------------------------------------------------------------ pass 4: annotations

    /// Stores the constraints as Ecore/Pivot annotations, as Eclipse OCL does, so the
    /// metamodel can be exported as an `.ecore` file with its OCL.
    fn store_annotations(&mut self) {
        let mut unnamed: HashMap<(ElementRef, ConstraintKind), usize> = HashMap::new();
        let mut invariant_names: HashMap<ClassifierId, Vec<String>> = HashMap::new();
        let mut packages: Vec<PackageId> = Vec::new();
        for ec in &self.constraints {
            let c = &ec.constraint;
            let text = |s: Span| self.text.get(s.range()).unwrap_or("").trim().to_string();
            let expr = text(c.expr.span);
            let value = match &c.message {
                Some(m) => format!(
                    "Tuple {{\n\tmessage : String = {},\n\tstatus : Boolean = \n\t\t\t{expr}\n}}.status",
                    text(m.span)
                ),
                None => expr,
            };
            let n = unnamed.entry((ec.owner, c.kind)).or_insert(0);
            *n += 1;
            let suffix = |base: &str| match &c.name {
                Some(name) => format!("{base}_{}", name.name),
                None if *n == 1 => base.to_string(),
                None => format!("{base}_{n}"),
            };
            let key = match c.kind {
                ConstraintKind::Inv => {
                    let name = c.name.as_ref().map_or_else(|| format!("invariant{n}"), |i| i.name.clone());
                    if let ElementRef::Classifier(class) = ec.owner {
                        invariant_names.entry(class).or_default().push(name.clone());
                    }
                    name
                }
                ConstraintKind::Pre => suffix("precondition"),
                ConstraintKind::Post => suffix("postcondition"),
                ConstraintKind::Body => "body".to_string(),
                ConstraintKind::Init => "initial".to_string(),
                ConstraintKind::Derive => "derivation".to_string(),
            };
            let annotations = self.mm.annotations_mut(ec.owner);
            detail(annotations, PIVOT_SOURCE, key, value);
            if !packages.contains(&ec.package) {
                packages.push(ec.package);
            }
        }
        for (class, names) in invariant_names {
            let annotations = &mut self.mm.classifier_mut(class).annotations;
            let ecore = annotation_mut(annotations, ECORE_ANNOTATION_SOURCE, true);
            ecore.details.retain(|(k, _)| k != "constraints");
            ecore.details.push(("constraints".into(), names.join(" ")));
        }
        for p in packages {
            let annotations = &mut self.mm.package_mut(p).annotations;
            let ecore = annotation_mut(annotations, ECORE_ANNOTATION_SOURCE, true);
            for key in ["invocationDelegates", "settingDelegates", "validationDelegates"] {
                if ecore.detail(key).is_none() {
                    ecore.details.push((key.into(), PIVOT_SOURCE.into()));
                }
            }
        }
    }
}

fn annotation(def: &AnnotationDef) -> Annotation {
    Annotation { source: def.source.clone(), details: def.details.clone(), offsets: Vec::new() }
}

/// The annotation with `source`, created if missing (at the front when `first`).
fn annotation_mut<'a>(annotations: &'a mut Vec<Annotation>, source: &str, first: bool) -> &'a mut Annotation {
    let pos = match annotations.iter().position(|a| a.source == source) {
        Some(p) => p,
        None => {
            let a = Annotation { source: source.to_string(), ..Annotation::default() };
            if first {
                annotations.insert(0, a);
                0
            } else {
                annotations.push(a);
                annotations.len() - 1
            }
        }
    };
    &mut annotations[pos]
}

fn detail(annotations: &mut Vec<Annotation>, source: &str, key: String, value: String) {
    let a = annotation_mut(annotations, source, false);
    a.details.retain(|(k, _)| *k != key);
    a.details.push((key, value));
}

// ------------------------------------------------------------------ printing

/// Renders a metamodel resource as an OCLinEcore document, including the OCL stored in its
/// annotations.
pub fn print(mm: &Metamodel, resource: usize) -> String {
    let p = Printer { mm, resource };
    let mut out = String::new();
    let imports = p.imports();
    for i in &imports {
        let _ = writeln!(out, "import {};", string(i));
    }
    for (i, root) in mm.resources()[resource].roots.iter().enumerate() {
        if i > 0 || !imports.is_empty() {
            out.push('\n');
        }
        p.package(&mut out, *root, 0);
    }
    out
}

/// How a type is written in a document's package `package`: OCL primitive types for the Ecore
/// types that represent them, `ecore::EInt` for other Ecore types, simple names in the same
/// package and qualified names elsewhere.
pub fn type_name(mm: &Metamodel, t: ClassifierId, package: PackageId) -> String {
    let c = mm.classifier(t);
    if c.package == mm.ecore_package() {
        if let Some((ocl, _)) = PRIMITIVES.iter().find(|(o, e)| *e == c.name && *o != "UnlimitedNatural") {
            return ocl.to_string();
        }
        return format!("ecore::{}", name(&c.name));
    }
    if c.package == package {
        return name(&c.name);
    }
    mm.qualified_name(t).split("::").map(name).collect::<Vec<_>>().join("::")
}

struct Printer<'a> {
    mm: &'a Metamodel,
    resource: usize,
}

const OCL_KEYWORDS: &[&str] = &[
    "and", "or", "xor", "not", "implies", "if", "then", "else", "elseif", "endif", "let", "in", "true", "false",
    "null", "invalid", "self",
];

/// A name, quoted as `_'...'` unless it is a plain identifier.
pub fn name(n: &str) -> String {
    let mut chars = n.chars();
    let plain = chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
        && !OCL_KEYWORDS.contains(&n);
    if plain { n.to_string() } else { format!("_{}", string(n)) }
}

/// A single-quoted string literal.
pub fn string(s: &str) -> String {
    let mut out = String::from("'");
    for c in s.chars() {
        match c {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

/// A multiplicity as written after a type: `[?]`, `[*]`, `[+]`, `[n]`, `[n..m]`, or nothing for `[1]`.
pub fn multiplicity(m: &Multiplicity) -> String {
    let upper = |u: i32| if u < 0 { "*".to_string() } else { u.to_string() };
    match (m.lower, m.upper) {
        (1, 1) => String::new(),
        (0, 1) => "[?]".into(),
        (0, u) if u < 0 => "[*]".into(),
        (1, u) if u < 0 => "[+]".into(),
        (l, u) if l == u => format!("[{l}]"),
        (l, u) => format!("[{l}..{}]", upper(u)),
    }
}

/// OCL constraints stored in an element's annotations: `(key, expression text)`.
fn ocl_details(annotations: &[Annotation]) -> Vec<(&str, &str)> {
    annotations
        .iter()
        .filter(|a| OCL_ANNOTATION_SOURCES.contains(&a.source.as_str()))
        .flat_map(|a| a.details.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .collect()
}

/// Splits an annotation expression into the status expression and the message (for
/// OCLinEcore's `Tuple{message, status}.status` encoding), both as text.
fn split_message(text: &str) -> (String, Option<String>) {
    let (expr, diags) = parse_expression(text);
    if !diags.is_empty() {
        return (text.trim().to_string(), None);
    }
    let (status, message) = split_status_tuple(Rc::new(expr));
    if let Some(m) = message {
        let slice = |s: Span| text.get(s.range()).unwrap_or("").trim().to_string();
        return (slice(status.span), Some(slice(m.span)));
    }
    (text.trim().to_string(), None)
}

impl Printer<'_> {
    /// Resources (other than Ecore and this one) whose classifiers are used, as import URIs.
    fn imports(&self) -> Vec<String> {
        let mm = self.mm;
        let mut used: Vec<usize> = Vec::new();
        let mut note = |c: ClassifierId| {
            let mut p = mm.classifier(c).package;
            while let Some(sp) = mm.package(p).super_package {
                p = sp;
            }
            let r = mm.package(p).resource;
            if r != self.resource && mm.package(p).ns_uri != ECORE_NS_URI && !used.contains(&r) {
                used.push(r);
            }
        };
        for c in mm.classifier_ids().filter(|c| self.in_resource(*c)) {
            for s in mm.super_types(c) {
                note(*s);
            }
            if let Some(data) = mm.class_data(c) {
                for f in &data.features {
                    if let Some(t) = mm.feature(*f).ty {
                        note(t);
                    }
                }
                for o in &data.operations {
                    let op = mm.operation(*o);
                    for t in op.ty.iter().chain(op.params.iter().filter_map(|p| p.ty.as_ref())) {
                        note(*t);
                    }
                }
            }
        }
        let own = &mm.resources()[self.resource].uri;
        used.iter()
            .map(|r| {
                let uri = &mm.resources()[*r].uri;
                if uri.ends_with(".ecore") && !uri.contains("://") {
                    oxy_ecore::ecore_writer::relative_path(own, uri)
                } else {
                    let root = mm.resources()[*r].roots[0];
                    mm.package(root).ns_uri.clone()
                }
            })
            .collect()
    }

    fn in_resource(&self, c: ClassifierId) -> bool {
        let mut p = self.mm.classifier(c).package;
        while let Some(sp) = self.mm.package(p).super_package {
            p = sp;
        }
        self.mm.package(p).resource == self.resource
    }

    fn type_name(&self, t: ClassifierId, package: PackageId) -> String {
        type_name(self.mm, t, package)
    }

    fn typed(&self, ty: Option<ClassifierId>, m: &Multiplicity, package: PackageId) -> String {
        match ty {
            Some(t) => format!(" : {}{}", self.type_name(t, package), multiplicity(m)),
            None => String::new(),
        }
    }

    fn annotations(&self, out: &mut String, annotations: &[Annotation], depth: usize) {
        for a in annotations {
            if OCL_ANNOTATION_SOURCES.contains(&a.source.as_str()) {
                continue;
            }
            if a.source == ECORE_ANNOTATION_SOURCE {
                let rest: Vec<&(String, String)> = a
                    .details
                    .iter()
                    .filter(|(k, _)| {
                        !matches!(
                            k.as_str(),
                            "constraints" | "invocationDelegates" | "settingDelegates" | "validationDelegates"
                        )
                    })
                    .collect();
                if rest.is_empty() {
                    continue;
                }
            }
            indent(out, depth);
            if a.source == GENMODEL_SOURCE
                && let [(k, v)] = a.details.as_slice()
                && k == "documentation"
            {
                let _ = writeln!(out, "documentation {};", string(v));
                continue;
            }
            let _ = write!(out, "annotation {}", string(&a.source));
            if !a.details.is_empty() {
                let details: Vec<String> =
                    a.details.iter().map(|(k, v)| format!("{} = {}", string(k), string(v))).collect();
                let _ = write!(out, "({})", details.join(", "));
            }
            out.push_str(";\n");
        }
    }

    fn package(&self, out: &mut String, p: PackageId, depth: usize) {
        let mm = self.mm;
        let pkg = mm.package(p);
        indent(out, depth);
        let _ = write!(
            out,
            "package {} : {}",
            name(&pkg.name),
            name(if pkg.ns_prefix.is_empty() { &pkg.name } else { &pkg.ns_prefix })
        );
        if !pkg.ns_uri.is_empty() {
            let _ = write!(out, " = {}", string(&pkg.ns_uri));
        }
        out.push_str(" {\n");
        self.annotations(out, &pkg.annotations, depth + 1);
        for (i, c) in pkg.classifiers.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            self.classifier(out, *c, p, depth + 1);
        }
        for s in &pkg.subpackages {
            out.push('\n');
            self.package(out, *s, depth + 1);
        }
        indent(out, depth);
        out.push_str("}\n");
    }

    fn classifier(&self, out: &mut String, c: ClassifierId, package: PackageId, depth: usize) {
        let mm = self.mm;
        let cl = mm.classifier(c);
        indent(out, depth);
        let instance = cl.instance_class_name.as_ref().map(|i| format!(" : {}", string(i))).unwrap_or_default();
        match &cl.kind {
            ClassifierKind::DataType(_) => {
                let _ = write!(out, "datatype {}{instance}", name(&cl.name));
                self.annotated_end(out, &cl.annotations, depth);
            }
            ClassifierKind::Enum(literals) => {
                let _ = writeln!(out, "enum {}{instance} {{", name(&cl.name));
                self.annotations(out, &cl.annotations, depth + 1);
                for (i, l) in literals.iter().enumerate() {
                    indent(out, depth + 1);
                    let _ = write!(out, "literal {}", name(&l.name));
                    if l.literal != l.name {
                        let _ = write!(out, " : {}", string(&l.literal));
                    }
                    if l.value != i as i64 {
                        let _ = write!(out, " = {}", l.value);
                    }
                    out.push_str(";\n");
                }
                indent(out, depth);
                out.push_str("}\n");
            }
            ClassifierKind::Class(data) => {
                if data.is_abstract {
                    out.push_str("abstract ");
                }
                let _ = write!(out, "class {}", name(&cl.name));
                if !data.super_types.is_empty() {
                    let supers: Vec<String> = data.super_types.iter().map(|s| self.type_name(*s, package)).collect();
                    let _ = write!(out, " extends {}", supers.join(", "));
                }
                out.push_str(&instance);
                if data.is_interface {
                    out.push_str(" { interface }");
                }
                let mut body = String::new();
                self.annotations(&mut body, &cl.annotations, depth + 1);
                let invariant_ops: Vec<OperationId> =
                    data.operations.iter().copied().filter(|o| is_invariant_operation(mm, *o)).collect();
                for f in &data.features {
                    self.feature(&mut body, *f, package, depth + 1);
                }
                for o in &data.operations {
                    if !invariant_ops.contains(o) {
                        self.operation(&mut body, *o, package, depth + 1);
                    }
                }
                for (key, text) in ocl_details(&cl.annotations) {
                    self.constraint(&mut body, "invariant", Some(key), text, depth + 1);
                }
                for o in invariant_ops {
                    let op = mm.operation(o);
                    for (key, text) in ocl_details(&op.annotations) {
                        if key == "body" {
                            self.constraint(&mut body, "invariant", Some(&op.name), text, depth + 1);
                        }
                    }
                }
                if body.is_empty() {
                    out.push_str(";\n");
                } else {
                    out.push_str(" {\n");
                    out.push_str(&body);
                    indent(out, depth);
                    out.push_str("}\n");
                }
            }
        }
    }

    /// Ends an element: `;`, or a body with its annotations.
    fn annotated_end(&self, out: &mut String, annotations: &[Annotation], depth: usize) {
        let mut body = String::new();
        self.annotations(&mut body, annotations, depth + 1);
        if body.is_empty() {
            out.push_str(";\n");
        } else {
            out.push_str(" {\n");
            out.push_str(&body);
            indent(out, depth);
            out.push_str("}\n");
        }
    }

    fn constraint(&self, out: &mut String, keyword: &str, constraint_name: Option<&str>, text: &str, depth: usize) {
        let (expr, message) = split_message(text);
        indent(out, depth);
        out.push_str(keyword);
        if let Some(n) = constraint_name {
            let _ = write!(out, " {}", name(n));
        }
        if let Some(m) = message {
            if constraint_name.is_none() {
                out.push_str(" Unnamed");
            }
            let _ = write!(out, "({m})");
        }
        let _ = writeln!(out, ": {expr};");
    }

    fn feature(&self, out: &mut String, f: FeatureId, package: PackageId, depth: usize) {
        let feature = self.mm.feature(f);
        indent(out, depth);
        let keyword = if feature.is_reference() { "property" } else { "attribute" };
        let _ = write!(out, "{keyword} {}", name(&feature.name));
        if let Some(o) = feature.opposite() {
            let _ = write!(out, "#{}", name(&self.mm.feature(o).name));
        }
        out.push_str(&self.typed(feature.ty, &feature.multiplicity, package));
        if let Some(d) = &feature.default_value_literal {
            let _ = write!(out, " = {}", string(d));
        }
        let m = &feature.multiplicity;
        let mut qualifiers = Vec::new();
        if feature.is_containment() {
            qualifiers.push("composes");
        }
        if matches!(feature.kind, FeatureKind::Attribute { is_id: true }) {
            qualifiers.push("id");
        }
        if feature.derived {
            qualifiers.push("derived");
        }
        if m.is_many() && m.ordered {
            qualifiers.push("ordered");
        }
        if !feature.changeable {
            qualifiers.push("readonly");
        }
        if feature.transient {
            qualifiers.push("transient");
        }
        if m.is_many() && !m.unique {
            qualifiers.push("!unique");
        }
        if feature.volatile {
            qualifiers.push("volatile");
        }
        if !qualifiers.is_empty() {
            let _ = write!(out, " {{ {} }}", qualifiers.join(", "));
        }
        let mut body = String::new();
        self.annotations(&mut body, &feature.annotations, depth + 1);
        for (key, text) in ocl_details(&feature.annotations) {
            match key {
                "derivation" => self.constraint(&mut body, "derivation", None, text, depth + 1),
                "initial" => self.constraint(&mut body, "initial", None, text, depth + 1),
                _ => {}
            }
        }
        if body.is_empty() {
            out.push_str(";\n");
        } else {
            out.push_str(" {\n");
            out.push_str(&body);
            indent(out, depth);
            out.push_str("}\n");
        }
    }

    fn operation(&self, out: &mut String, o: OperationId, package: PackageId, depth: usize) {
        let op = self.mm.operation(o);
        indent(out, depth);
        let params: Vec<String> = op
            .params
            .iter()
            .map(|p| {
                let mut s = format!("{}{}", name(&p.name), self.typed(p.ty, &p.multiplicity, package));
                if p.multiplicity.is_many() && p.multiplicity.ordered {
                    s.push_str(" { ordered }");
                }
                s
            })
            .collect();
        let _ = write!(out, "operation {}({})", name(&op.name), params.join(", "));
        out.push_str(&self.typed(op.ty, &op.multiplicity, package));
        if op.ty.is_some() && op.multiplicity.is_many() && op.multiplicity.ordered {
            out.push_str(" { ordered }");
        }
        let mut body = String::new();
        self.annotations(&mut body, &op.annotations, depth + 1);
        let details = ocl_details(&op.annotations);
        for (key, text) in &details {
            if let Some((kind, constraint_name)) = contract_key(key)
                && kind == ConstraintKind::Pre
            {
                self.constraint(&mut body, "precondition", constraint_name.as_deref(), text, depth + 1);
            }
        }
        for (key, text) in &details {
            if *key == "body" {
                self.constraint(&mut body, "body", None, text, depth + 1);
            }
        }
        for (key, text) in &details {
            if let Some((kind, constraint_name)) = contract_key(key)
                && kind == ConstraintKind::Post
            {
                self.constraint(&mut body, "postcondition", constraint_name.as_deref(), text, depth + 1);
            }
        }
        if body.is_empty() {
            out.push_str(";\n");
        } else {
            out.push_str(" {\n");
            out.push_str(&body);
            indent(out, depth);
            out.push_str("}\n");
        }
    }
}
