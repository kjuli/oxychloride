//! Loading `.ecore` files (EMF's XMI serialization of `EPackage`s) into a [`Metamodel`].

use std::path::{Component, Path, PathBuf};

use crate::error::LoadError;
use crate::metamodel::*;
use crate::xml::{self, Element, XSI_NS};

/// The outcome of loading an `.ecore` resource.
#[derive(Debug, Clone)]
pub struct EcoreLoad {
    /// Index of the resource in [`Metamodel::resources`].
    pub resource: usize,
    /// Root packages of the resource.
    pub packages: Vec<PackageId>,
    /// Non-fatal problems, e.g. references that could not be resolved.
    pub warnings: Vec<LoadError>,
    /// Further resources that were loaded to resolve cross-file references.
    pub dependencies: Vec<usize>,
}

/// Fetches a referenced resource, given its URI (resolved against the referencing resource):
/// returns the URI to load it as (normally its canonical path) and its text. A fetch may map
/// URIs to other locations, like EMF's URI converter or package registry; the resource is then
/// known by both (see [`EcoreResource::aliases`]).
pub type Fetch<'a> = dyn FnMut(&str) -> Option<(String, String)> + 'a;

/// Reads referenced resources from disk, by path.
pub fn read_from_disk(uri: &str) -> Option<(String, String)> {
    std::fs::read_to_string(uri).ok().map(|text| (uri.to_string(), text))
}

/// Loads an `.ecore` document from a string. Cross-file references can only be resolved
/// against resources that are already loaded.
pub fn load_str(mm: &mut Metamodel, text: &str, uri: &str) -> Result<EcoreLoad, LoadError> {
    load_with(mm, text, uri, &mut |_| None)
}

/// Loads an `.ecore` file, loading referenced `.ecore` files from disk as needed.
pub fn load_file(mm: &mut Metamodel, path: impl AsRef<Path>) -> Result<EcoreLoad, LoadError> {
    load_file_with(mm, path, &mut read_from_disk)
}

/// Loads an `.ecore` file, using `fetch` to obtain referenced resources.
pub fn load_file_with(
    mm: &mut Metamodel,
    path: impl AsRef<Path>,
    fetch: &mut Fetch<'_>,
) -> Result<EcoreLoad, LoadError> {
    let uri = canonical_uri(path.as_ref());
    if let Some(resource) = mm.resource_of_uri(&uri) {
        return Ok(EcoreLoad {
            resource,
            packages: mm.resources[resource].roots.clone(),
            warnings: Vec::new(),
            dependencies: Vec::new(),
        });
    }
    let text = std::fs::read_to_string(&uri).map_err(|e| LoadError::new(&uri, format!("cannot read file: {e}")))?;
    load_with(mm, &text, &uri, fetch)
}

/// Loads an `.ecore` document, using `fetch` to obtain referenced resources.
pub fn load_with(mm: &mut Metamodel, text: &str, uri: &str, fetch: &mut Fetch<'_>) -> Result<EcoreLoad, LoadError> {
    load_as(mm, text, uri, &[], fetch)
}

/// Loads an `.ecore` file that is also known by the (logical) URIs `logical`, e.g.
/// `platform:/resource/project/model/a.ecore`, using `fetch` to obtain referenced resources.
pub fn load_file_as(
    mm: &mut Metamodel,
    path: impl AsRef<Path>,
    logical: &[String],
    fetch: &mut Fetch<'_>,
) -> Result<EcoreLoad, LoadError> {
    let uri = canonical_uri(path.as_ref());
    if mm.resource_of_uri(&uri).is_none() {
        let text = std::fs::read_to_string(&uri).map_err(|e| LoadError::new(&uri, format!("cannot read file: {e}")))?;
        return load_as(mm, &text, &uri, logical, fetch);
    }
    load_as(mm, "", &uri, logical, fetch)
}

/// Loads an `.ecore` document read from `uri` that is also known by the (logical) URIs
/// `logical` — those its references were made with, e.g. `platform:/plugin/a/model/a.ecore`
/// for a file read from a jar or a folder. Like EMF, which resolves the relative references of
/// a resource against the URI it was loaded by, relative references are resolved against the
/// logical URIs first, then against `uri`.
pub fn load_as(
    mm: &mut Metamodel,
    text: &str,
    uri: &str,
    logical: &[String],
    fetch: &mut Fetch<'_>,
) -> Result<EcoreLoad, LoadError> {
    if let Some(resource) = mm.resource_of_uri(uri) {
        for alias in logical {
            mm.add_resource_alias(resource, alias);
        }
        return Ok(EcoreLoad {
            resource,
            packages: mm.resources[resource].roots.clone(),
            warnings: Vec::new(),
            dependencies: Vec::new(),
        });
    }
    let root = xml::parse(text, uri)?;
    let package_elements: Vec<&Element> = match root.local_name() {
        "EPackage" => vec![&root],
        "XMI" => root.children_named("EPackage").collect(),
        other => {
            return Err(LoadError::at(uri, text, root.offset, format!("expected an EPackage, found <{other}>")));
        }
    };
    if package_elements.is_empty() {
        return Err(LoadError::at(uri, text, root.offset, "document contains no EPackage"));
    }

    let resource = mm.add_resource(uri);
    for alias in logical {
        mm.add_resource_alias(resource, alias);
    }

    let mut builder = Builder { mm, text, uri, resource, pending: Vec::new(), warnings: Vec::new() };
    let mut roots = Vec::new();
    for el in package_elements {
        roots.push(builder.package(el, None)?);
    }
    let Builder { pending, mut warnings, .. } = builder;
    mm.resources[resource].roots = roots.clone();
    for p in &roots {
        mm.register_tree(*p);
    }

    let mut dependencies = Vec::new();
    for p in pending {
        resolve(mm, text, uri, resource, p, fetch, &mut warnings, &mut dependencies);
    }
    mm.rebuild_caches();
    Ok(EcoreLoad { resource, packages: roots, warnings, dependencies })
}

/// A reference that is resolved once the whole resource has been built.
struct Pending {
    target: PendingTarget,
    value: String,
    offset: usize,
}

enum PendingTarget {
    FeatureType(FeatureId),
    Opposite(FeatureId),
    OperationType(OperationId),
    ParameterType(OperationId, usize),
    SuperTypes(ClassifierId),
}

struct Builder<'a> {
    mm: &'a mut Metamodel,
    text: &'a str,
    uri: &'a str,
    resource: usize,
    pending: Vec<Pending>,
    warnings: Vec<LoadError>,
}

impl Builder<'_> {
    fn warn(&mut self, offset: usize, message: impl Into<String>) {
        self.warnings.push(LoadError::at(self.uri, self.text, offset, message));
    }

    fn package(&mut self, el: &Element, parent: Option<PackageId>) -> Result<PackageId, LoadError> {
        let name = el.attr("name").unwrap_or_default().to_string();
        let ns_uri = el.attr("nsURI").unwrap_or_default().to_string();
        if !ns_uri.is_empty()
            && let Some(existing) = self.mm.package_by_ns_uri(&ns_uri)
        {
            let other = &self.mm.resources[self.mm.package(existing).resource].uri;
            return Err(LoadError::at(
                self.uri,
                self.text,
                el.offset,
                format!("a package with nsURI '{ns_uri}' is already loaded from '{other}'"),
            ));
        }
        let id = self.mm.add_package(Package {
            name,
            ns_uri,
            ns_prefix: el.attr("nsPrefix").unwrap_or_default().to_string(),
            super_package: parent,
            subpackages: Vec::new(),
            classifiers: Vec::new(),
            annotations: annotations(el),
            resource: self.resource,
        });
        for child in &el.children {
            match child.local_name() {
                "eClassifiers" => self.classifier(child, id),
                "eSubpackages" => {
                    self.package(child, Some(id))?;
                }
                _ => {}
            }
        }
        Ok(id)
    }

    fn classifier(&mut self, el: &Element, package: PackageId) {
        let ty = el.attr_ns(XSI_NS, "type").map(|t| xml::split_qname(t).1).unwrap_or("EClass");
        let name = el.attr("name").unwrap_or_default().to_string();
        let instance_class_name = el.attr("instanceClassName").map(str::to_string);
        let kind = match ty {
            "EClass" => ClassifierKind::Class(ClassData {
                is_abstract: bool_attr(el, "abstract", false),
                is_interface: bool_attr(el, "interface", false),
                ..ClassData::default()
            }),
            "EDataType" => ClassifierKind::DataType(DataTypeKind::from_instance_class(instance_class_name.as_deref())),
            "EEnum" => {
                let literals = el
                    .children_named("eLiterals")
                    .enumerate()
                    .map(|(i, l)| {
                        let name = l.attr("name").unwrap_or_default().to_string();
                        EnumLiteral {
                            literal: l.attr("literal").map(str::to_string).unwrap_or_else(|| name.clone()),
                            value: l.attr("value").and_then(|v| v.parse().ok()).unwrap_or(i as i64),
                            name,
                        }
                    })
                    .collect();
                ClassifierKind::Enum(literals)
            }
            other => {
                self.warn(el.offset, format!("unknown classifier type '{other}', treating '{name}' as a class"));
                ClassifierKind::Class(ClassData::default())
            }
        };
        let is_class = matches!(kind, ClassifierKind::Class(_));
        let id = self.mm.add_classifier(Classifier {
            name,
            package,
            instance_class_name,
            annotations: annotations(el),
            kind,
        });
        if !is_class {
            return;
        }
        if let Some(supers) = el.attr("eSuperTypes") {
            self.pending.push(Pending {
                target: PendingTarget::SuperTypes(id),
                value: supers.to_string(),
                offset: el.offset,
            });
        }
        for generic in el.children_named("eGenericSuperTypes") {
            if let Some(c) = generic.attr("eClassifier") {
                self.pending.push(Pending {
                    target: PendingTarget::SuperTypes(id),
                    value: c.to_string(),
                    offset: generic.offset,
                });
            }
        }
        for child in &el.children {
            match child.local_name() {
                "eStructuralFeatures" => self.feature(child, id),
                "eOperations" => self.operation(child, id),
                _ => {}
            }
        }
    }

    fn feature(&mut self, el: &Element, owner: ClassifierId) {
        let ty = el.attr_ns(XSI_NS, "type").map(|t| xml::split_qname(t).1).unwrap_or("EAttribute");
        let kind = if ty == "EReference" {
            FeatureKind::Reference { containment: bool_attr(el, "containment", false), opposite: None }
        } else {
            FeatureKind::Attribute { is_id: bool_attr(el, "iD", false) }
        };
        let id = self.mm.add_feature(Feature {
            name: el.attr("name").unwrap_or_default().to_string(),
            owner,
            kind,
            ty: None,
            multiplicity: multiplicity(el),
            derived: bool_attr(el, "derived", false),
            transient: bool_attr(el, "transient", false),
            volatile: bool_attr(el, "volatile", false),
            changeable: bool_attr(el, "changeable", true),
            default_value_literal: el.attr("defaultValueLiteral").map(str::to_string),
            annotations: annotations(el),
        });
        if let Some(t) = type_ref(el) {
            self.pending.push(Pending { target: PendingTarget::FeatureType(id), value: t, offset: el.offset });
        }
        if let Some(o) = el.attr("eOpposite") {
            self.pending.push(Pending { target: PendingTarget::Opposite(id), value: o.to_string(), offset: el.offset });
        }
    }

    fn operation(&mut self, el: &Element, owner: ClassifierId) {
        let params: Vec<Parameter> = el
            .children_named("eParameters")
            .map(|p| Parameter {
                name: p.attr("name").unwrap_or_default().to_string(),
                ty: None,
                multiplicity: multiplicity(p),
            })
            .collect();
        let id = self.mm.add_operation(Operation {
            name: el.attr("name").unwrap_or_default().to_string(),
            owner,
            ty: None,
            multiplicity: multiplicity(el),
            params,
            annotations: annotations(el),
        });
        if let Some(t) = type_ref(el) {
            self.pending.push(Pending { target: PendingTarget::OperationType(id), value: t, offset: el.offset });
        }
        for (i, p) in el.children_named("eParameters").enumerate() {
            if let Some(t) = type_ref(p) {
                self.pending.push(Pending { target: PendingTarget::ParameterType(id, i), value: t, offset: p.offset });
            }
        }
    }
}

fn bool_attr(el: &Element, name: &str, default: bool) -> bool {
    el.attr(name).map_or(default, |v| v == "true")
}

fn multiplicity(el: &Element) -> Multiplicity {
    Multiplicity {
        lower: el.attr("lowerBound").and_then(|v| v.parse().ok()).unwrap_or(0),
        upper: el.attr("upperBound").and_then(|v| v.parse().ok()).unwrap_or(1),
        ordered: bool_attr(el, "ordered", true),
        unique: bool_attr(el, "unique", true),
    }
}

/// The `eType` reference, either as an attribute or via an `eGenericType` child.
fn type_ref(el: &Element) -> Option<String> {
    if let Some(t) = el.attr("eType") {
        return Some(t.to_string());
    }
    el.children_named("eGenericType").find_map(|g| g.attr("eClassifier")).map(str::to_string)
}

fn annotations(el: &Element) -> Vec<Annotation> {
    el.children_named("eAnnotations")
        .map(|a| Annotation {
            source: a.attr("source").unwrap_or_default().to_string(),
            details: a
                .children_named("details")
                .map(|d| {
                    let value = d
                        .attr("value")
                        .map(str::to_string)
                        .or_else(|| d.children_named("value").next().map(|v| v.text.clone()))
                        .unwrap_or_default();
                    (d.attr("key").unwrap_or_default().to_string(), value)
                })
                .collect(),
            offsets: a.children_named("details").map(|d| d.offset).collect(),
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn resolve(
    mm: &mut Metamodel,
    text: &str,
    uri: &str,
    resource: usize,
    pending: Pending,
    fetch: &mut Fetch<'_>,
    warnings: &mut Vec<LoadError>,
    dependencies: &mut Vec<usize>,
) {
    let mut targets = Vec::new();
    for href in reference_tokens(&pending.value) {
        match resolve_href(mm, uri, resource, href, fetch, dependencies) {
            Some(t) => targets.push(t),
            None => {
                warnings.push(LoadError::at(uri, text, pending.offset, format!("cannot resolve reference '{href}'")))
            }
        }
    }
    let classifier = |t: &ElementRef| match t {
        ElementRef::Classifier(c) => Some(*c),
        _ => None,
    };
    match pending.target {
        PendingTarget::FeatureType(f) => mm.feature_mut(f).ty = targets.first().and_then(classifier),
        PendingTarget::OperationType(o) => mm.operation_mut(o).ty = targets.first().and_then(classifier),
        PendingTarget::ParameterType(o, i) => mm.operation_mut(o).params[i].ty = targets.first().and_then(classifier),
        PendingTarget::SuperTypes(c) => {
            let supers: Vec<ClassifierId> = targets.iter().filter_map(classifier).collect();
            if let ClassifierKind::Class(data) = &mut mm.classifier_mut(c).kind {
                data.super_types.extend(supers);
            }
        }
        PendingTarget::Opposite(f) => {
            if let Some(ElementRef::Feature(o)) = targets.first()
                && let FeatureKind::Reference { opposite, .. } = &mut mm.feature_mut(f).kind
            {
                *opposite = Some(*o);
            }
        }
    }
}

/// Splits a reference attribute value into its hrefs, skipping type hints such as
/// `ecore:EDataType` in `ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EString`.
fn reference_tokens(value: &str) -> impl Iterator<Item = &str> {
    value.split_whitespace().filter(|t| t.contains('#') || t.starts_with('/'))
}

fn resolve_href(
    mm: &mut Metamodel,
    base: &str,
    resource: usize,
    href: &str,
    fetch: &mut Fetch<'_>,
    dependencies: &mut Vec<usize>,
) -> Option<ElementRef> {
    let (target_uri, fragment) = href.split_once('#').unwrap_or(("", href));
    if target_uri.is_empty() {
        return mm.resolve_fragment(resource, fragment);
    }
    // A registered package nsURI (e.g. the Ecore package): the fragment is relative to that package.
    if let Some(p) = mm.package_by_ns_uri(target_uri) {
        let path = fragment.trim_start_matches('/');
        return mm.resolve_in_package(p, path);
    }
    // EMF resolves a relative reference against the URI the resource was loaded by (e.g.
    // `../../b/model/b.ecore` in `platform:/plugin/a/model/a.ecore`), whatever file it was read from.
    if !has_scheme(target_uri) && !Path::new(target_uri).is_absolute() {
        for logical in logical_uris(mm, resource) {
            let resolved = resolve_uri(&logical, target_uri);
            if let Some(r) =
                mm.resource_of_uri(&resolved).or_else(|| fetch_resource(mm, &resolved, fetch, dependencies))
            {
                return mm.resolve_fragment(r, fragment);
            }
        }
    }
    let resolved = resolve_relative(base, target_uri);
    let target_resource = match mm.resource_of_uri(&resolved) {
        Some(r) => r,
        None => {
            match fetch_resource(mm, &resolved, fetch, dependencies).or_else(|| find_by_file_name(mm, target_uri)) {
                Some(r) => r,
                None => {
                    // `platform:/plugin/...` and `platform:/resource/...` URIs cannot be resolved
                    // without an Eclipse installation (unless `fetch` maps them); fall back to a file
                    // with the same name next to the referencing file (the usual layout when
                    // metamodels are copied together).
                    if !target_uri.starts_with("platform:") {
                        return None;
                    }
                    let file = target_uri.rsplit('/').next().unwrap_or(target_uri);
                    let (location, text) = fetch(&resolve_relative(base, file))?;
                    let r = match mm.resource_of_uri(&location) {
                        Some(r) => r,
                        None => {
                            let loaded = load_as(mm, &text, &location, std::slice::from_ref(&resolved), fetch).ok()?;
                            dependencies.push(loaded.resource);
                            dependencies.extend(loaded.dependencies);
                            loaded.resource
                        }
                    };
                    mm.add_resource_alias(r, &resolved);
                    r
                }
            }
        }
    };
    // The nsURI of a package the fetched resource defines.
    if let Some(p) = mm.package_by_ns_uri(target_uri) {
        return mm.resolve_in_package(p, fragment.trim_start_matches('/'));
    }
    mm.resolve_fragment(target_resource, fragment)
}

/// Fetches and loads the resource `uri` refers to (known by `uri` afterwards), or finds it
/// among the loaded ones by the location it is read from.
fn fetch_resource(
    mm: &mut Metamodel,
    uri: &str,
    fetch: &mut Fetch<'_>,
    dependencies: &mut Vec<usize>,
) -> Option<usize> {
    let (location, text) = fetch(uri)?;
    let logical: Vec<String> = (location != uri).then(|| uri.to_string()).into_iter().collect();
    match mm.resource_of_uri(&location) {
        Some(r) => {
            mm.add_resource_alias(r, uri);
            Some(r)
        }
        None => {
            let loaded = load_as(mm, &text, &location, &logical, fetch).ok()?;
            dependencies.push(loaded.resource);
            dependencies.extend(loaded.dependencies);
            Some(loaded.resource)
        }
    }
}

/// The URIs a resource is known by that stand for its location (`platform:/…`, a placeholder
/// URI of a package registry; not the nsURIs of its packages), against which relative
/// references are resolved before its file.
fn logical_uris(mm: &Metamodel, resource: usize) -> Vec<String> {
    let r = &mm.resources[resource];
    let ns_uri = |a: &str| mm.packages_of_resource(resource).any(|p| mm.package(p).ns_uri == a);
    r.aliases.iter().filter(|a| has_scheme(a) && !ns_uri(a)).cloned().collect()
}

/// Whether a URI starts with a scheme (`platform:`, `http:`, `archive:`; not a drive letter).
pub fn has_scheme(uri: &str) -> bool {
    match uri.split_once(':') {
        Some((scheme, _)) => {
            scheme.len() > 1
                && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        }
        None => false,
    }
}

/// Resolves a relative URI reference against a hierarchical URI (`platform:/plugin/a/model/a.ecore`,
/// `archive:file:/x.jar!/model/a.ecore`, `http://host/a/b.ecore`), removing `.` and `..`
/// segments. The part of the base before its path (the scheme and authority, an archive's
/// location up to `!/`) is kept.
pub fn resolve_uri(base: &str, relative: &str) -> String {
    if has_scheme(relative) {
        return relative.to_string();
    }
    // The part that stays: an archive's location, else the scheme and the authority.
    let root_end = if let Some(i) = base.find("!/") {
        i + 2
    } else if let Some(i) = base.find("://") {
        base[i + 3..].find('/').map_or(base.len(), |j| i + 3 + j + 1)
    } else {
        base.find(":/").map_or(0, |i| i + 2)
    };
    let (root, path) = base.split_at(root_end.min(base.len()));
    let mut segments: Vec<&str> = path.split('/').collect();
    segments.pop(); // the base's own name
    if relative.starts_with('/') {
        segments.clear();
    }
    for segment in relative.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            s => segments.push(s),
        }
    }
    let segments: Vec<&str> = segments.into_iter().filter(|s| !s.is_empty()).collect();
    format!("{root}{}", segments.join("/"))
}

/// Matches `platform:/...` style URIs against already loaded resources by file name.
fn find_by_file_name(mm: &Metamodel, uri: &str) -> Option<usize> {
    if !uri.contains(':') {
        return None;
    }
    let name = uri.rsplit('/').next()?;
    if name == "Ecore.ecore" {
        return Some(mm.package(mm.ecore_package()).resource);
    }
    mm.resources.iter().position(|r| r.uri.rsplit(['/', '\\']).next() == Some(name))
}

/// Resolves `relative` against the location of `base`. URIs with a scheme are returned as is.
pub fn resolve_relative(base: &str, relative: &str) -> String {
    if has_scheme(relative) || Path::new(relative).is_absolute() {
        return relative.to_string();
    }
    if has_scheme(base) {
        return resolve_uri(base, relative);
    }
    let joined = Path::new(base).parent().unwrap_or(Path::new("")).join(relative);
    canonical_uri(&joined)
}

/// Canonicalizes a path if it exists, otherwise normalizes it lexically.
pub fn canonical_uri(path: &Path) -> String {
    if let Ok(p) = std::fs::canonicalize(path) {
        return p.to_string_lossy().into_owned();
    }
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out.to_string_lossy().into_owned()
}
