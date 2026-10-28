//! Metamodel documents as class diagrams, and changing them from a diagram.
//!
//! A diagram shows the classifiers of one metamodel document: an OCLinEcore document of the
//! workspace, or an `.ecore` file given as text (loaded into a metamodel of its own, so that it
//! does not clash with the workspace's metamodels; the workspace is not changed). Elements are
//! identified by ids made of their names (see [`MetamodelDiagram`]).
//!
//! The text stays the one source of truth: an edit made in a diagram ([`MetamodelEdit`])
//! becomes small text edits of the document, written in its syntax (`oclinecore.rs`,
//! `ecore.rs`) and leaving the rest of the text — layout, comments, OCL — as it is. The client
//! applies them to the document and draws the diagram again from the new text.

use std::collections::{HashMap, HashSet};
use std::ops::Range as ByteRange;

use indexmap::{IndexMap, IndexSet};
use oxy_ecore::{
    ClassifierId, ClassifierKind as EKind, FeatureId, FeatureKind, Metamodel, OCL_ANNOTATION_SOURCES, OperationId,
    PackageId, UNBOUNDED,
};
use oxy_ocl::LineIndex;
use oxy_ocl::env::is_invariant_operation;

use crate::protocol::*;
use crate::workspace::{Error, Workspace};

mod ecore;
mod henshin;
mod henshin_edit;
mod oclinecore;

/// Whether a document is an `.ecore` file (rather than an OCLinEcore document).
pub(crate) fn is_ecore(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".ecore")
}

impl Workspace {
    /// The elements of a metamodel document for drawing it as a class diagram: an OCLinEcore
    /// document, or an `.ecore` file (`text`, else read from disk).
    pub fn metamodel_diagram(&self, name: &str, text: Option<&str>) -> Result<MetamodelDiagram, Error> {
        if is_ecore(name) {
            let text = ecore_text(name, text)?;
            let file = ecore::File::load(name, &text, &self.locator())?;
            let doc = ecore::Doc::new(&text, &file.root);
            let cx = Context::new(&file.mm, file.resource, &text);
            let problems = file
                .warnings
                .iter()
                .map(|w| {
                    let offset =
                        w.location.map_or(0, |(l, c)| line_offset(&text, l.saturating_sub(1)) + c.saturating_sub(1));
                    (Severity::Warning, w.message.clone(), offset..offset)
                })
                .collect();
            let visible: Vec<usize> = (0..file.mm.resources().len()).collect();
            Ok(cx.diagram(&doc.source_map(), MetamodelSyntax::Ecore, &visible, problems))
        } else {
            let built = self.oclinecore_document(name).ok_or_else(|| not_a_metamodel(name))?;
            let text = self.document_text(name).unwrap_or_default();
            let cx = Context::new(&self.mm, built.resource, text);
            let doc = oclinecore::Doc::new(text, &built.document);
            let index = LineIndex::new(text);
            let problems = self
                .diagnostics(Some(name))
                .into_iter()
                .map(|d| {
                    let at = |p: Position| index.offset(text, oxy_ocl::Position { line: p.line, column: p.column });
                    (d.severity, d.message, at(d.range.start)..at(d.range.end))
                })
                .collect();
            let visible = doc.visible_resources(&self.mm, name, built.resource);
            Ok(cx.diagram(&doc.source_map(), MetamodelSyntax::OclInEcore, &visible, problems))
        }
    }

    /// The text edits that make `edit` in a metamodel document (an OCLinEcore document, or an
    /// `.ecore` file given as `text`, else read from disk). The document is not changed.
    pub fn edit_metamodel(
        &self,
        name: &str,
        text: Option<&str>,
        edit: &MetamodelEdit,
    ) -> Result<MetamodelEditResult, Error> {
        let (edits, element, text) = if is_ecore(name) {
            let text = ecore_text(name, text)?;
            let file = ecore::File::load(name, &text, &self.locator())?;
            let doc = ecore::Doc::new(&text, &file.root);
            let cx = Context::new(&file.mm, file.resource, &text);
            let (edits, element) = doc.edit(&cx, edit)?;
            (ecore::emf_layout(&text, edits)?, element, text)
        } else {
            let built = self.oclinecore_document(name).ok_or_else(|| not_a_metamodel(name))?;
            let text = self.document_text(name).unwrap_or_default();
            let cx = Context::new(&self.mm, built.resource, text);
            let doc = oclinecore::Doc::new(text, &built.document);
            let (edits, element) = doc.edit(&cx, edit)?;
            (edits, element, text.to_string())
        };
        Ok(MetamodelEditResult { edits: text_edits(&text, edits)?, element })
    }
}

fn not_a_metamodel(name: &str) -> Error {
    Error::NotFound(format!("no OCLinEcore document or .ecore file named '{name}'"))
}

fn ecore_text(name: &str, text: Option<&str>) -> Result<String, Error> {
    match text {
        Some(t) => Ok(t.to_string()),
        None => std::fs::read_to_string(name).map_err(|e| Error::NotFound(format!("cannot read '{name}': {e}"))),
    }
}

// ------------------------------------------------------------------ elements and ids

/// A metamodel element a diagram shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Element {
    Package(PackageId),
    Classifier(ClassifierId),
    Feature(FeatureId),
    Operation(OperationId),
    /// An enumeration literal, by index.
    Literal(ClassifierId, usize),
}

/// Hands out ids in document order: a repeated id gets `#2`, `#3`, …. The metamodel and the
/// syntax trees are walked in the same order, so they agree on the ids.
#[derive(Default)]
pub(crate) struct IdAllocator(HashMap<String, usize>);

impl IdAllocator {
    pub(crate) fn next(&mut self, base: String) -> String {
        let n = self.0.entry(base.clone()).or_insert(0);
        *n += 1;
        if *n == 1 { base } else { format!("{base}#{n}") }
    }
}

pub(crate) fn classifier_id(package: &str, name: &str) -> String {
    format!("{package}::{name}")
}

pub(crate) fn member_id(owner: &str, name: &str) -> String {
    format!("{owner}.{name}")
}

pub(crate) fn operation_id(owner: &str, name: &str) -> String {
    format!("{owner}.{name}()")
}

/// The ids of the elements of a metamodel resource.
pub(crate) struct Ids {
    by_id: IndexMap<String, Element>,
    of: HashMap<Element, String>,
}

impl Ids {
    fn new(mm: &Metamodel, resource: usize) -> Ids {
        let mut ids = Ids { by_id: IndexMap::new(), of: HashMap::new() };
        let mut alloc = IdAllocator::default();
        for root in &mm.resources()[resource].roots {
            ids.package(mm, *root, "", &mut alloc);
        }
        ids
    }

    fn insert(&mut self, id: String, e: Element) {
        self.of.insert(e, id.clone());
        self.by_id.insert(id, e);
    }

    fn package(&mut self, mm: &Metamodel, p: PackageId, prefix: &str, alloc: &mut IdAllocator) {
        let pkg = mm.package(p);
        let id = alloc.next(format!("{prefix}{}", pkg.name));
        self.insert(id.clone(), Element::Package(p));
        for c in &pkg.classifiers {
            let cid = alloc.next(classifier_id(&id, &mm.classifier(*c).name));
            self.insert(cid.clone(), Element::Classifier(*c));
            match &mm.classifier(*c).kind {
                EKind::Class(data) => {
                    for f in &data.features {
                        self.insert(alloc.next(member_id(&cid, &mm.feature(*f).name)), Element::Feature(*f));
                    }
                    for o in &data.operations {
                        self.insert(alloc.next(operation_id(&cid, &mm.operation(*o).name)), Element::Operation(*o));
                    }
                }
                EKind::Enum(literals) => {
                    for (i, l) in literals.iter().enumerate() {
                        self.insert(alloc.next(member_id(&cid, &l.name)), Element::Literal(*c, i));
                    }
                }
                EKind::DataType(_) => {}
            }
        }
        for s in &pkg.subpackages {
            self.package(mm, *s, &format!("{id}::"), alloc);
        }
    }
}

/// A metamodel resource and its document text, which edits are computed for.
pub(crate) struct Context<'a> {
    pub(crate) mm: &'a Metamodel,
    pub(crate) resource: usize,
    pub(crate) text: &'a str,
    ids: Ids,
}

impl<'a> Context<'a> {
    fn new(mm: &'a Metamodel, resource: usize, text: &'a str) -> Self {
        Context { mm, resource, text, ids: Ids::new(mm, resource) }
    }

    pub(crate) fn element(&self, id: &str) -> Result<Element, Error> {
        self.ids.by_id.get(id).copied().ok_or_else(|| Error::NotFound(format!("no element '{id}' in the metamodel")))
    }

    pub(crate) fn id(&self, e: Element) -> Option<&str> {
        self.ids.of.get(&e).map(String::as_str)
    }

    pub(crate) fn classifier(&self, id: &str) -> Result<ClassifierId, Error> {
        match self.element(id)? {
            Element::Classifier(c) => Ok(c),
            _ => Err(Error::Invalid(format!("'{id}' is not a classifier"))),
        }
    }

    pub(crate) fn class(&self, id: &str) -> Result<ClassifierId, Error> {
        let c = self.classifier(id)?;
        if self.mm.is_class(c) { Ok(c) } else { Err(Error::Invalid(format!("'{id}' is not a class"))) }
    }

    pub(crate) fn enumeration(&self, id: &str) -> Result<ClassifierId, Error> {
        let c = self.classifier(id)?;
        if self.mm.is_enum(c) { Ok(c) } else { Err(Error::Invalid(format!("'{id}' is not an enumeration"))) }
    }

    pub(crate) fn in_resource(&self, c: ClassifierId) -> bool {
        let mut p = self.mm.classifier(c).package;
        while let Some(sp) = self.mm.package(p).super_package {
            p = sp;
        }
        self.mm.package(p).resource == self.resource
    }

    /// The id of any classifier: its diagram id, or its qualified name in another metamodel.
    pub(crate) fn type_id(&self, c: ClassifierId) -> String {
        self.id(Element::Classifier(c)).map(str::to_string).unwrap_or_else(|| self.mm.qualified_name(c))
    }

    /// The classifier a type id names (see [`Context::type_id`]).
    pub(crate) fn resolve_type(&self, id: &str) -> Result<ClassifierId, Error> {
        if let Some(Element::Classifier(c)) = self.ids.by_id.get(id) {
            return Ok(*c);
        }
        let path: Vec<&str> = id.split("::").collect();
        let found = self.mm.find_classifiers_qualified(&path);
        found.first().copied().ok_or_else(|| Error::NotFound(format!("no type '{id}'")))
    }

    /// A feature type: data types and enumerations for attributes, classes for references.
    pub(crate) fn feature_type(&self, id: &str, reference: bool) -> Result<ClassifierId, Error> {
        let t = self.resolve_type(id)?;
        match (reference, self.mm.is_class(t)) {
            (true, false) => Err(Error::Invalid(format!("the type of a reference must be a class, not '{id}'"))),
            (false, true) => {
                Err(Error::Invalid(format!("the type of an attribute must be a data type or enumeration, not '{id}'")))
            }
            _ => Ok(t),
        }
    }

    /// Features of the resource (in id order).
    pub(crate) fn features(&self) -> impl Iterator<Item = FeatureId> + '_ {
        self.ids.by_id.values().filter_map(|e| match e {
            Element::Feature(f) => Some(*f),
            _ => None,
        })
    }

    /// Classes of the resource (in id order).
    pub(crate) fn classes(&self) -> impl Iterator<Item = ClassifierId> + '_ {
        self.ids.by_id.values().filter_map(|e| match e {
            Element::Classifier(c) if self.mm.is_class(*c) => Some(*c),
            _ => None,
        })
    }

    /// Fails unless `name` can name an element.
    pub(crate) fn check_name(&self, name: &str) -> Result<(), Error> {
        if name.is_empty() || name.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(Error::Invalid(format!("'{name}' is not a valid name")));
        }
        Ok(())
    }

    /// Fails if the package already has a classifier named `name`.
    pub(crate) fn check_new_classifier(&self, package: PackageId, name: &str) -> Result<(), Error> {
        self.check_name(name)?;
        if self.mm.package(package).classifiers.iter().any(|c| self.mm.classifier(*c).name == name) {
            return Err(Error::Invalid(format!("the package already has a classifier named '{name}'")));
        }
        Ok(())
    }

    /// Fails if the class already has (or inherits) a feature named `name`.
    pub(crate) fn check_new_feature(&self, class: ClassifierId, name: &str) -> Result<(), Error> {
        self.check_name(name)?;
        if self.mm.lookup_feature(class, name).is_some() {
            return Err(Error::Invalid(format!(
                "'{}' already has a feature named '{name}'",
                self.mm.classifier(class).name
            )));
        }
        Ok(())
    }

    /// The elements an edit deletes, with the references typed by deleted classes.
    pub(crate) fn deleted(&self, ids: &[String]) -> Result<HashSet<Element>, Error> {
        let mut deleted = HashSet::new();
        for id in ids {
            let e = self.element(id)?;
            if let Element::Package(p) = e
                && self.mm.package(p).super_package.is_none()
            {
                return Err(Error::Invalid(format!("the root package '{id}' cannot be deleted")));
            }
            deleted.insert(e);
        }
        let deleted_classifier = |c: ClassifierId, deleted: &HashSet<Element>| {
            deleted.contains(&Element::Classifier(c)) || {
                let mut p = Some(self.mm.classifier(c).package);
                std::iter::from_fn(|| {
                    let cur = p?;
                    p = self.mm.package(cur).super_package;
                    Some(cur)
                })
                .any(|p| deleted.contains(&Element::Package(p)))
            }
        };
        let typed: Vec<FeatureId> = self
            .features()
            .filter(|f| {
                let feature = self.mm.feature(*f);
                feature.is_reference() && feature.ty.is_some_and(|t| deleted_classifier(t, &deleted))
            })
            .collect();
        deleted.extend(typed.into_iter().map(Element::Feature));
        // Classifiers inside deleted packages are deleted too.
        let nested: Vec<ClassifierId> = self
            .ids
            .by_id
            .values()
            .filter_map(|e| if let Element::Classifier(c) = e { Some(*c) } else { None })
            .collect();
        for c in nested {
            if deleted_classifier(c, &deleted) {
                deleted.insert(Element::Classifier(c));
            }
        }
        Ok(deleted)
    }

    /// Whether a deleted element contains (or is) the element.
    pub(crate) fn is_deleted(&self, e: Element, deleted: &HashSet<Element>) -> bool {
        match e {
            _ if deleted.contains(&e) => true,
            Element::Feature(f) => deleted.contains(&Element::Classifier(self.mm.feature(f).owner)),
            Element::Operation(o) => deleted.contains(&Element::Classifier(self.mm.operation(o).owner)),
            Element::Literal(c, _) => deleted.contains(&Element::Classifier(c)),
            Element::Classifier(_) | Element::Package(_) => false,
        }
    }

    // -------------------------------------------------------------- the diagram

    fn diagram(
        &self,
        map: &SourceMap,
        syntax: MetamodelSyntax,
        visible: &[usize],
        problems: Vec<(Severity, String, ByteRange<usize>)>,
    ) -> MetamodelDiagram {
        let mm = self.mm;
        let index = LineIndex::new(self.text);
        let range = |s: &ByteRange<usize>| byte_range(self.text, &index, s.clone());
        let span = |id: &str| range(map.spans.get(id).unwrap_or(&(0..0)));
        let label = |t: ClassifierId, package: PackageId| match syntax {
            MetamodelSyntax::OclInEcore => oxy_ocl::oclinecore::type_name(mm, t, package),
            MetamodelSyntax::Ecore => mm.classifier(t).name.clone(),
        };
        let mut external: IndexSet<ClassifierId> = IndexSet::new();
        let mut packages = Vec::new();
        let mut classifiers = Vec::new();
        for (id, e) in &self.ids.by_id {
            match *e {
                Element::Package(p) => {
                    let pkg = mm.package(p);
                    packages.push(DiagramPackage {
                        id: id.clone(),
                        name: pkg.name.clone(),
                        ns_uri: pkg.ns_uri.clone(),
                        ns_prefix: pkg.ns_prefix.clone(),
                        range: span(id),
                    });
                }
                Element::Classifier(c) => {
                    let cl = mm.classifier(c);
                    let package = self.id(Element::Package(cl.package)).unwrap_or_default().to_string();
                    let mut d = DiagramClassifier {
                        id: id.clone(),
                        name: cl.name.clone(),
                        package,
                        kind: ClassifierKind::DataType,
                        is_abstract: false,
                        is_interface: false,
                        instance_class: cl.instance_class_name.clone(),
                        super_types: Vec::new(),
                        attributes: Vec::new(),
                        references: Vec::new(),
                        operations: Vec::new(),
                        literals: Vec::new(),
                        invariants: Vec::new(),
                        range: span(id),
                    };
                    match &cl.kind {
                        EKind::Class(data) => {
                            d.kind = ClassifierKind::Class;
                            d.is_abstract = data.is_abstract;
                            d.is_interface = data.is_interface;
                            for s in &data.super_types {
                                if !self.in_resource(*s) {
                                    external.insert(*s);
                                }
                                d.super_types.push(self.type_id(*s));
                            }
                            for f in &data.features {
                                let Some(fid) = self.id(Element::Feature(*f)) else { continue };
                                let feature = mm.feature(*f);
                                let m = feature.multiplicity;
                                let df = DiagramFeature {
                                    id: fid.to_string(),
                                    name: feature.name.clone(),
                                    type_name: feature
                                        .ty
                                        .map(|t| label(t, cl.package))
                                        .or_else(|| map.type_texts.get(fid).cloned())
                                        .unwrap_or_default(),
                                    ty: feature.ty.map(|t| self.type_id(t)),
                                    lower: m.lower,
                                    upper: if m.upper < 0 { UNBOUNDED } else { m.upper },
                                    containment: feature.is_containment(),
                                    opposite: feature
                                        .opposite()
                                        .and_then(|o| self.id(Element::Feature(o)))
                                        .map(str::to_string),
                                    ordered: m.ordered,
                                    unique: m.unique,
                                    derived: feature.derived,
                                    changeable: feature.changeable,
                                    transient: feature.transient,
                                    volatile: feature.volatile,
                                    is_id: matches!(feature.kind, FeatureKind::Attribute { is_id: true }),
                                    default_value: feature.default_value_literal.clone(),
                                    range: span(fid),
                                };
                                if feature.is_reference() {
                                    if let Some(t) = feature.ty
                                        && !self.in_resource(t)
                                    {
                                        external.insert(t);
                                    }
                                    d.references.push(df);
                                } else {
                                    d.attributes.push(df);
                                }
                            }
                            for o in &data.operations {
                                let Some(oid) = self.id(Element::Operation(*o)) else { continue };
                                if is_invariant_operation(mm, *o) {
                                    continue;
                                }
                                let op = mm.operation(*o);
                                d.operations.push(DiagramOperation {
                                    id: oid.to_string(),
                                    name: op.name.clone(),
                                    parameters: op
                                        .params
                                        .iter()
                                        .map(|p| DiagramParameter {
                                            name: p.name.clone(),
                                            type_name: p.ty.map(|t| label(t, cl.package)).unwrap_or_default(),
                                            ty: p.ty.map(|t| self.type_id(t)),
                                            lower: p.multiplicity.lower,
                                            upper: p.multiplicity.upper.max(UNBOUNDED),
                                        })
                                        .collect(),
                                    type_name: op
                                        .ty
                                        .map(|t| label(t, cl.package))
                                        .or_else(|| map.type_texts.get(oid).cloned()),
                                    ty: op.ty.map(|t| self.type_id(t)),
                                    lower: op.multiplicity.lower,
                                    upper: op.multiplicity.upper.max(UNBOUNDED),
                                    has_body: ocl_keys(&op.annotations).any(|k| k == "body"),
                                    range: span(oid),
                                });
                            }
                            d.invariants = match map.invariants.get(id) {
                                Some(list) => list
                                    .iter()
                                    .map(|(name, s)| DiagramInvariant { name: name.clone(), range: range(s) })
                                    .collect(),
                                None => invariant_names(mm, c)
                                    .into_iter()
                                    .map(|name| DiagramInvariant { name, range: d.range })
                                    .collect(),
                            };
                        }
                        EKind::Enum(literals) => {
                            d.kind = ClassifierKind::Enum;
                            for (i, l) in literals.iter().enumerate() {
                                let Some(lid) = self.id(Element::Literal(c, i)) else { continue };
                                d.literals.push(DiagramLiteral {
                                    id: lid.to_string(),
                                    name: l.name.clone(),
                                    value: l.value,
                                    literal: l.literal.clone(),
                                    range: span(lid),
                                });
                            }
                        }
                        EKind::DataType(_) => {}
                    }
                    classifiers.push(d);
                }
                _ => {}
            }
        }
        let external = external
            .into_iter()
            .map(|c| {
                let cl = mm.classifier(c);
                let data = mm.class_data(c);
                let mut root = cl.package;
                while let Some(p) = mm.package(root).super_package {
                    root = p;
                }
                DiagramExternal {
                    id: self.type_id(c),
                    name: cl.name.clone(),
                    kind: kind_of(&cl.kind),
                    is_abstract: data.is_some_and(|d| d.is_abstract),
                    is_interface: data.is_some_and(|d| d.is_interface),
                    resource: mm.resources()[mm.package(root).resource].uri.clone(),
                }
            })
            .collect();
        let first_package = mm.resources()[self.resource].roots.first().copied().unwrap_or(mm.ecore_package());
        let types = self.types(visible, |t| label(t, first_package));
        let problems = problems
            .into_iter()
            .map(|(severity, message, s)| DiagramProblem {
                severity,
                message,
                element: map.innermost(s.start).map(str::to_string),
                range: range(&s),
            })
            .collect();
        MetamodelDiagram { syntax, packages, classifiers, external, types, problems }
    }

    /// The types a feature can have: the classifiers of the visible resources (this one first),
    /// and Ecore's data types and `EObject`.
    fn types(&self, visible: &[usize], label: impl Fn(ClassifierId) -> String) -> Vec<DiagramType> {
        let mm = self.mm;
        let ecore = mm.package(mm.ecore_package()).resource;
        let mut resources: Vec<usize> = vec![self.resource];
        resources.extend(visible.iter().copied().filter(|r| *r != self.resource && *r != ecore));
        resources.push(ecore);
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for r in resources {
            let mut packages: Vec<PackageId> = mm.resources()[r].roots.clone();
            while let Some(p) = packages.pop() {
                packages.extend(mm.package(p).subpackages.iter().rev());
                for c in &mm.package(p).classifiers {
                    let cl = mm.classifier(*c);
                    if r == ecore && mm.is_class(*c) && *c != mm.eobject() {
                        continue;
                    }
                    if r == ecore && ["EDiagnosticChain", "EMap", "EEList", "ETreeIterator"].contains(&cl.name.as_str())
                    {
                        continue;
                    }
                    if seen.insert(*c) {
                        out.push(DiagramType { id: self.type_id(*c), label: label(*c), kind: kind_of(&cl.kind) });
                    }
                }
            }
        }
        out
    }
}

fn kind_of(kind: &EKind) -> ClassifierKind {
    match kind {
        EKind::Class(_) => ClassifierKind::Class,
        EKind::DataType(_) => ClassifierKind::DataType,
        EKind::Enum(_) => ClassifierKind::Enum,
    }
}

fn ocl_keys(annotations: &[oxy_ecore::Annotation]) -> impl Iterator<Item = &str> {
    annotations
        .iter()
        .filter(|a| OCL_ANNOTATION_SOURCES.contains(&a.source.as_str()))
        .flat_map(|a| a.details.iter().map(|(k, _)| k.as_str()))
}

/// The invariants of a class: the OCL in its annotations, and invariant operations.
fn invariant_names(mm: &Metamodel, c: ClassifierId) -> Vec<String> {
    let cl = mm.classifier(c);
    let mut names: Vec<String> = ocl_keys(&cl.annotations).map(str::to_string).collect();
    if let Some(data) = mm.class_data(c) {
        names.extend(
            data.operations.iter().filter(|o| is_invariant_operation(mm, **o)).map(|o| mm.operation(*o).name.clone()),
        );
    }
    names
}

/// Where the elements of a document are defined.
#[derive(Default)]
pub(crate) struct SourceMap {
    pub(crate) spans: HashMap<String, ByteRange<usize>>,
    /// The invariants of classes (by class id) with their definitions, where the syntax has them.
    pub(crate) invariants: HashMap<String, Vec<(String, ByteRange<usize>)>>,
    /// Types as written, of features and operations whose type is not resolved.
    pub(crate) type_texts: HashMap<String, String>,
}

impl SourceMap {
    /// The innermost element whose definition contains `offset`.
    fn innermost(&self, offset: usize) -> Option<&str> {
        self.spans
            .iter()
            .filter(|(_, s)| s.start <= offset && offset <= s.end)
            .min_by_key(|(_, s)| s.end - s.start)
            .map(|(id, _)| id.as_str())
    }
}

// ------------------------------------------------------------------ text edits

/// A text edit by byte offsets.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ByteEdit {
    pub(crate) range: ByteRange<usize>,
    pub(crate) text: String,
}

impl ByteEdit {
    pub(crate) fn insert(at: usize, text: impl Into<String>) -> Self {
        ByteEdit { range: at..at, text: text.into() }
    }

    pub(crate) fn replace(range: ByteRange<usize>, text: impl Into<String>) -> Self {
        ByteEdit { range, text: text.into() }
    }

    pub(crate) fn delete(range: ByteRange<usize>) -> Self {
        ByteEdit { range, text: String::new() }
    }

    fn is_deletion(&self) -> bool {
        self.text.is_empty() && !self.range.is_empty()
    }
}

/// Orders the edits and makes them non-overlapping (see [`normalize_edits`]), as text edits.
fn text_edits(text: &str, edits: Vec<ByteEdit>) -> Result<Vec<TextEdit>, Error> {
    let out = normalize_edits(edits)?;
    let index = LineIndex::new(text);
    Ok(out.into_iter().map(|e| TextEdit { range: byte_range(text, &index, e.range), text: e.text }).collect())
}

/// The text with the edits made.
pub(crate) fn apply_edits(text: &str, edits: Vec<ByteEdit>) -> Result<String, Error> {
    let mut out = text.to_string();
    for e in normalize_edits(edits)?.into_iter().rev() {
        out.replace_range(e.range, &e.text);
    }
    Ok(out)
}

/// Orders the edits and makes them non-overlapping: insertions at the same offset are joined
/// (in the order given), deletions that overlap are merged, and edits inside a deletion are
/// dropped (they change text that is deleted anyway).
pub(crate) fn normalize_edits(mut edits: Vec<ByteEdit>) -> Result<Vec<ByteEdit>, Error> {
    // A stable sort keeps insertions at one offset in their order.
    edits.sort_by_key(|e| (e.range.start, e.range.end));
    let mut out: Vec<ByteEdit> = Vec::new();
    for e in edits {
        if let Some(last) = out.last_mut() {
            if last.is_deletion() && e.is_deletion() && e.range.start <= last.range.end {
                last.range.end = last.range.end.max(e.range.end);
                continue;
            }
            // Insertions at the ends of a deletion are kept.
            let inside = if e.range.is_empty() {
                last.range.start < e.range.start && e.range.start < last.range.end
            } else {
                last.range.start <= e.range.start && e.range.end <= last.range.end
            };
            if last.is_deletion() && inside {
                continue;
            }
            if last.range.is_empty() && e.range.is_empty() && last.range.start == e.range.start {
                last.text.push_str(&e.text);
                continue;
            }
            if e.range.start < last.range.end {
                if e.range == last.range && e.text == last.text {
                    continue;
                }
                return Err(Error::Invalid("the edit changes overlapping parts of the document".into()));
            }
        }
        out.push(e);
    }
    Ok(out)
}

/// Text edits from replacements of `text`'s bytes (in order, not overlapping).
pub(crate) fn replacements_to_edits(text: &str, replacements: Vec<crate::textdiff::Replacement>) -> Vec<TextEdit> {
    let index = LineIndex::new(text);
    replacements.into_iter().map(|r| TextEdit { range: byte_range(text, &index, r.range), text: r.text }).collect()
}

pub(crate) fn byte_range(text: &str, index: &LineIndex, r: ByteRange<usize>) -> Range {
    let p = |o: usize| {
        let p = index.position(text, o);
        Position { line: p.line, column: p.column }
    };
    Range { start: p(r.start), end: p(r.end) }
}

/// The byte offset at which 0-based `line` starts.
fn line_offset(text: &str, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    text.match_indices('\n').nth(line - 1).map_or(text.len(), |(i, _)| i + 1)
}

/// The offset of the start of the line containing `offset`.
pub(crate) fn line_start(text: &str, offset: usize) -> usize {
    text[..offset].rfind('\n').map_or(0, |i| i + 1)
}

/// The offset of the line break ending the line containing `offset` (or the end of the text).
pub(crate) fn line_end(text: &str, offset: usize) -> usize {
    text[offset..].find('\n').map_or(text.len(), |i| offset + i)
}

/// The whitespace the line containing `offset` starts with.
pub(crate) fn indentation(text: &str, offset: usize) -> &str {
    let start = line_start(text, offset);
    let line = &text[start..line_end(text, start)];
    &line[..line.len() - line.trim_start().len()]
}

/// Whether only whitespace precedes `offset` on its line.
pub(crate) fn starts_line(text: &str, offset: usize) -> bool {
    text[line_start(text, offset)..offset].trim().is_empty()
}

/// One level of indentation as the document writes it (two spaces if it does not tell).
pub(crate) fn indent_unit(text: &str) -> String {
    let mut levels: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| &l[..l.len() - l.trim_start().len()])
        .filter(|w| !w.is_empty())
        .collect();
    levels.sort_by_key(|w| w.len());
    match levels.first() {
        Some(w) if w.starts_with('\t') => "\t".into(),
        Some(w) if w.len() <= 8 => w.to_string(),
        _ => "  ".into(),
    }
}

/// The text to delete for removing the element at `span`: whole lines when the element has its
/// lines to itself, with the comment lines right above it (lines starting with `comment`), and
/// one of two blank lines that would end up next to each other.
pub(crate) fn removal(text: &str, span: ByteRange<usize>, comment: Option<&str>) -> ByteRange<usize> {
    let (start, end) = (span.start, span.end.min(text.len()));
    let ls = line_start(text, start);
    let le = line_end(text, end);
    let rest = text[end..le].trim();
    let rest_is_comment = comment.is_some_and(|c| rest.starts_with(c));
    if !text[ls..start].trim().is_empty() || !(rest.is_empty() || rest_is_comment) {
        // Shares its line: remove the element and the spaces before it.
        let before = text[..start].trim_end_matches([' ', '\t']).len();
        return if text[..start].ends_with([' ', '\t']) && before > ls { before..end } else { start..end };
    }
    let mut s = ls;
    let e = (le + 1).min(text.len());
    if let Some(c) = comment {
        while s > 0 {
            let prev = line_start(text, s - 1);
            if text[prev..s - 1].trim_start().starts_with(c) {
                s = prev;
            } else {
                break;
            }
        }
    }
    if s > 0 {
        let prev = line_start(text, s - 1);
        let next = &text[e..line_end(text, e)];
        let next_blank = e >= text.len() || next.trim().is_empty();
        let next_closes = next.trim_start().starts_with('}') || next.trim_start().starts_with("</");
        if text[prev..s - 1].trim().is_empty() && (next_blank || next_closes) {
            s = prev;
        }
    }
    s..e
}

/// Inserts `member` (lines without indentation) into an element's body: after the
/// `after` sibling, else at the start of the body. `body` is the span of the body including its
/// delimiters, `open_end` the offset after the opening delimiter and `close` the offset of the
/// closing one. With `blank_line`, the member is separated from its predecessor by an empty
/// line.
pub(crate) fn insert_member(
    text: &str,
    parent_start: usize,
    open_end: usize,
    close: usize,
    after: Option<ByteRange<usize>>,
    member: &str,
    blank_line: bool,
) -> ByteEdit {
    let parent_indent = indentation(text, parent_start);
    let unit = indent_unit(text);
    let blank = if blank_line { "\n" } else { "" };
    let block = |indent: &str| member.lines().map(|l| format!("{indent}{l}")).collect::<Vec<_>>().join("\n");
    match after {
        Some(s) => {
            let indent = if starts_line(text, s.start) {
                indentation(text, s.start).to_string()
            } else {
                format!("{parent_indent}{unit}")
            };
            let le = line_end(text, s.end);
            let rest = text[s.end..le].trim();
            if (rest.is_empty() || rest.starts_with("--") || rest.starts_with("<!--")) && le < text.len() {
                ByteEdit::insert(le + 1, format!("{blank}{}\n", block(&indent)))
            } else {
                ByteEdit::insert(s.end, format!("\n{blank}{}", block(&indent)))
            }
        }
        None => {
            let indent = format!("{parent_indent}{unit}");
            if text[open_end..close].trim().is_empty() {
                return ByteEdit::replace(open_end..close, format!("\n{}\n{parent_indent}", block(&indent)));
            }
            let le = line_end(text, open_end);
            if text[open_end..le].trim().is_empty() && le < text.len() {
                let more = if blank_line { "\n" } else { "" };
                ByteEdit::insert(le + 1, format!("{}\n{more}", block(&indent)))
            } else {
                ByteEdit::insert(open_end, format!("\n{}", block(&indent)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_whole_lines_with_their_comments() {
        let text = "a {\n  x;\n\n  -- about y\n  y;\n\n  z;\n}\n";
        let y = text.find("y;").unwrap();
        let r = removal(text, y..y + 2, Some("--"));
        assert_eq!(format!("{}{}", &text[..r.start], &text[r.end..]), "a {\n  x;\n\n  z;\n}\n");
        let x = text.find("x;").unwrap();
        let r = removal(text, x..x + 2, Some("--"));
        assert_eq!(&text[r], "  x;\n");
        let inline = "a { x; y; }";
        let y = inline.find("y;").unwrap();
        assert_eq!(&inline[removal(inline, y..y + 2, None)], " y;");
    }

    #[test]
    fn joins_and_checks_edits() {
        let text = "abcdef";
        let edits = vec![
            ByteEdit::insert(2, "1"),
            ByteEdit::delete(1..4),
            ByteEdit::insert(1, "x"),
            ByteEdit::insert(1, "y"),
            ByteEdit::delete(3..5),
        ];
        let out = text_edits(text, edits).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].text, "xy");
        assert_eq!((out[1].range.start.column, out[1].range.end.column), (1, 5));
        assert!(text_edits(text, vec![ByteEdit::replace(1..3, "a"), ByteEdit::replace(2..4, "b")]).is_err());
    }
}
