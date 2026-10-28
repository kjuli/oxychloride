//! In-memory representation of Ecore metamodels.
//!
//! All metamodel elements live in arenas owned by [`Metamodel`] and are referred to by
//! small copyable ids. The metamodel only ever grows, so ids stay valid for its lifetime;
//! instance models ([`crate::Model`]) and compiled OCL refer to elements by these ids.

use std::collections::HashMap;

use indexmap::IndexMap;

macro_rules! id_type {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub u32);

        impl $name {
            pub fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

id_type!(/// An `EPackage`.
    PackageId);
id_type!(/// An `EClassifier`: `EClass`, `EDataType` or `EEnum`.
    ClassifierId);
id_type!(/// An `EStructuralFeature`: `EAttribute` or `EReference`.
    FeatureId);
id_type!(/// An `EOperation`.
    OperationId);

/// Unbounded upper bound (`*`), as in Ecore.
pub const UNBOUNDED: i32 = -1;
/// Unspecified upper bound (`?`), as in Ecore. Treated like [`UNBOUNDED`].
pub const UNSPECIFIED: i32 = -2;

pub const ECORE_NS_URI: &str = "http://www.eclipse.org/emf/2002/Ecore";

/// The URI of `Ecore.ecore` in an Eclipse installation (the built-in Ecore resource is known by it).
pub const ECORE_PLATFORM_URI: &str = "platform:/plugin/org.eclipse.emf.ecore/model/Ecore.ecore";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Annotation {
    pub source: String,
    pub details: Vec<(String, String)>,
    /// Where each detail is in the `.ecore` text it was loaded from: the byte offset of its
    /// `<details>` tag (empty for annotations not read from XML).
    pub offsets: Vec<usize>,
}

impl Annotation {
    pub fn detail(&self, key: &str) -> Option<&str> {
        self.details.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    pub ns_uri: String,
    pub ns_prefix: String,
    pub super_package: Option<PackageId>,
    pub subpackages: Vec<PackageId>,
    pub classifiers: Vec<ClassifierId>,
    pub annotations: Vec<Annotation>,
    /// Index into [`Metamodel::resources`] of the file this package was loaded from.
    pub resource: usize,
}

#[derive(Debug, Clone)]
pub struct Classifier {
    pub name: String,
    pub package: PackageId,
    pub instance_class_name: Option<String>,
    pub annotations: Vec<Annotation>,
    pub kind: ClassifierKind,
}

#[derive(Debug, Clone)]
pub enum ClassifierKind {
    Class(ClassData),
    DataType(DataTypeKind),
    Enum(Vec<EnumLiteral>),
}

#[derive(Debug, Clone, Default)]
pub struct ClassData {
    pub is_abstract: bool,
    pub is_interface: bool,
    pub super_types: Vec<ClassifierId>,
    pub features: Vec<FeatureId>,
    pub operations: Vec<OperationId>,
}

/// How values of an `EDataType` are represented, derived from its instance class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataTypeKind {
    Boolean,
    Integer,
    Real,
    String,
    /// Anything else (dates, Java objects, custom data types): values are kept as strings.
    Other,
}

impl DataTypeKind {
    pub fn from_instance_class(name: Option<&str>) -> DataTypeKind {
        match name.unwrap_or("") {
            "boolean" | "java.lang.Boolean" => DataTypeKind::Boolean,
            "int"
            | "long"
            | "short"
            | "byte"
            | "java.lang.Integer"
            | "java.lang.Long"
            | "java.lang.Short"
            | "java.lang.Byte"
            | "java.math.BigInteger" => DataTypeKind::Integer,
            "double" | "float" | "java.lang.Double" | "java.lang.Float" | "java.math.BigDecimal" => DataTypeKind::Real,
            "java.lang.String" | "char" | "java.lang.Character" => DataTypeKind::String,
            _ => DataTypeKind::Other,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EnumLiteral {
    pub name: String,
    pub value: i64,
    pub literal: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureKind {
    Attribute { is_id: bool },
    Reference { containment: bool, opposite: Option<FeatureId> },
}

/// Multiplicity and collection semantics shared by features, operations and parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Multiplicity {
    pub lower: i32,
    pub upper: i32,
    pub ordered: bool,
    pub unique: bool,
}

impl Default for Multiplicity {
    fn default() -> Self {
        Multiplicity { lower: 0, upper: 1, ordered: true, unique: true }
    }
}

impl Multiplicity {
    pub fn is_many(&self) -> bool {
        self.upper > 1 || self.upper == UNBOUNDED || self.upper == UNSPECIFIED
    }

    pub fn is_required(&self) -> bool {
        self.lower >= 1
    }
}

#[derive(Debug, Clone)]
pub struct Feature {
    pub name: String,
    pub owner: ClassifierId,
    pub kind: FeatureKind,
    /// `None` when the type is missing or could not be resolved (treated as `OclAny`).
    pub ty: Option<ClassifierId>,
    pub multiplicity: Multiplicity,
    pub derived: bool,
    pub transient: bool,
    pub volatile: bool,
    pub changeable: bool,
    pub default_value_literal: Option<String>,
    pub annotations: Vec<Annotation>,
}

impl Feature {
    pub fn is_reference(&self) -> bool {
        matches!(self.kind, FeatureKind::Reference { .. })
    }

    pub fn is_containment(&self) -> bool {
        matches!(self.kind, FeatureKind::Reference { containment: true, .. })
    }

    pub fn opposite(&self) -> Option<FeatureId> {
        match self.kind {
            FeatureKind::Reference { opposite, .. } => opposite,
            FeatureKind::Attribute { .. } => None,
        }
    }

    pub fn is_many(&self) -> bool {
        self.multiplicity.is_many()
    }
}

#[derive(Debug, Clone)]
pub struct Parameter {
    pub name: String,
    pub ty: Option<ClassifierId>,
    pub multiplicity: Multiplicity,
}

#[derive(Debug, Clone)]
pub struct Operation {
    pub name: String,
    pub owner: ClassifierId,
    /// `None` for `void` operations or unresolved types.
    pub ty: Option<ClassifierId>,
    pub multiplicity: Multiplicity,
    pub params: Vec<Parameter>,
    pub annotations: Vec<Annotation>,
}

/// A loaded `.ecore` file.
#[derive(Debug, Clone)]
pub struct EcoreResource {
    /// Canonical path or URI the resource was loaded from.
    pub uri: String,
    /// Other URIs the resource is known by, e.g. a placeholder URI that a package registry
    /// maps to the file (`http://example.org/shop.ecore` → `/models/shop.ecore`).
    pub aliases: Vec<String>,
    pub roots: Vec<PackageId>,
}

impl EcoreResource {
    /// Whether the resource is known by `uri` (its own URI or an alias).
    pub fn is_named(&self, uri: &str) -> bool {
        self.uri == uri || self.aliases.iter().any(|a| a == uri)
    }
}

/// Any addressable metamodel element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ElementRef {
    Package(PackageId),
    Classifier(ClassifierId),
    Feature(FeatureId),
    Operation(OperationId),
}

#[derive(Debug, Clone, Default)]
struct ClassCache {
    /// All proper supertypes, nearest first.
    all_super_types: Vec<ClassifierId>,
    /// All features including inherited ones, in EMF order (inherited first).
    all_features: Vec<FeatureId>,
    feature_by_name: HashMap<String, FeatureId>,
    all_operations: Vec<OperationId>,
}

/// A registry of Ecore packages. Always contains the Ecore package itself.
#[derive(Debug, Clone)]
pub struct Metamodel {
    pub(crate) packages: Vec<Package>,
    pub(crate) classifiers: Vec<Classifier>,
    pub(crate) features: Vec<Feature>,
    pub(crate) operations: Vec<Operation>,
    pub(crate) resources: Vec<EcoreResource>,
    by_ns_uri: IndexMap<String, PackageId>,
    caches: Vec<Option<ClassCache>>,
    eobject: Option<ClassifierId>,
}

impl Default for Metamodel {
    fn default() -> Self {
        Self::new()
    }
}

impl Metamodel {
    /// Creates a registry containing only the Ecore metamodel.
    pub fn new() -> Self {
        let mut mm = Metamodel::empty();
        crate::ecore_loader::load_str(&mut mm, crate::builtin::ECORE_ECORE, ECORE_NS_URI)
            .expect("the built-in Ecore metamodel must load");
        // Where `.ecore` files find Ecore, as in an Eclipse installation.
        let ecore = mm.package(mm.ecore_package()).resource;
        mm.add_resource_alias(ecore, ECORE_PLATFORM_URI);
        mm.eobject = mm.find_classifier_in(mm.ecore_package(), "EObject");
        mm
    }

    pub(crate) fn empty() -> Self {
        Metamodel {
            packages: Vec::new(),
            classifiers: Vec::new(),
            features: Vec::new(),
            operations: Vec::new(),
            resources: Vec::new(),
            by_ns_uri: IndexMap::new(),
            caches: Vec::new(),
            eobject: None,
        }
    }

    // ---------------------------------------------------------------- access

    pub fn package(&self, id: PackageId) -> &Package {
        &self.packages[id.index()]
    }

    pub fn classifier(&self, id: ClassifierId) -> &Classifier {
        &self.classifiers[id.index()]
    }

    pub fn feature(&self, id: FeatureId) -> &Feature {
        &self.features[id.index()]
    }

    pub fn operation(&self, id: OperationId) -> &Operation {
        &self.operations[id.index()]
    }

    pub fn resources(&self) -> &[EcoreResource] {
        &self.resources
    }

    pub fn package_ids(&self) -> impl Iterator<Item = PackageId> + '_ {
        (0..self.packages.len() as u32).map(PackageId)
    }

    /// The number of classifiers. Classifiers added later get ids at or above this count.
    pub fn classifier_count(&self) -> usize {
        self.classifiers.len()
    }

    pub fn classifier_ids(&self) -> impl Iterator<Item = ClassifierId> + '_ {
        (0..self.classifiers.len() as u32).map(ClassifierId)
    }

    /// Top-level packages, in load order (the Ecore package comes first).
    pub fn root_packages(&self) -> impl Iterator<Item = PackageId> + '_ {
        self.package_ids().filter(|p| self.package(*p).super_package.is_none())
    }

    /// Top-level packages that were loaded by the user (everything but Ecore itself).
    pub fn user_root_packages(&self) -> impl Iterator<Item = PackageId> + '_ {
        self.root_packages().filter(|p| self.package(*p).ns_uri != ECORE_NS_URI)
    }

    /// The packages of a resource, subpackages included.
    pub fn packages_of_resource(&self, resource: usize) -> impl Iterator<Item = PackageId> + '_ {
        (0..self.packages.len() as u32).map(PackageId).filter(move |p| self.package(*p).resource == resource)
    }

    pub fn ecore_package(&self) -> PackageId {
        PackageId(0)
    }

    /// `EObject`, the implicit supertype of every class.
    pub fn eobject(&self) -> ClassifierId {
        self.eobject.expect("Ecore is always loaded")
    }

    pub fn package_by_ns_uri(&self, ns_uri: &str) -> Option<PackageId> {
        self.by_ns_uri.get(ns_uri).copied()
    }

    pub fn resource_of_uri(&self, uri: &str) -> Option<usize> {
        self.resources.iter().position(|r| r.is_named(uri))
    }

    /// Makes a loaded resource known by another URI too (see [`EcoreResource::aliases`]).
    pub fn add_resource_alias(&mut self, resource: usize, uri: &str) {
        let r = &mut self.resources[resource];
        if !r.is_named(uri) {
            r.aliases.push(uri.to_string());
        }
    }

    /// The other resources whose classifiers the elements of `resource` refer to (as types and
    /// supertypes), in order of first use. Ecore's own resource is included when used.
    pub fn referenced_resources(&self, resource: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut note = |c: Option<ClassifierId>| {
            if let Some(c) = c {
                let r = self.package(self.classifier(c).package).resource;
                if r != resource && !out.contains(&r) {
                    out.push(r);
                }
            }
        };
        let mut packages = self.resources[resource].roots.clone();
        while let Some(p) = packages.pop() {
            let pkg = self.package(p);
            packages.extend(&pkg.subpackages);
            for &c in &pkg.classifiers {
                let Some(data) = self.class_data(c) else { continue };
                data.super_types.iter().for_each(|s| note(Some(*s)));
                for &f in &data.features {
                    note(self.feature(f).ty);
                }
                for &o in &data.operations {
                    let op = self.operation(o);
                    note(op.ty);
                    op.params.iter().for_each(|p| note(p.ty));
                }
            }
        }
        out
    }

    /// Resolves an Ecore URI fragment such as `//Library/books` inside a loaded resource.
    /// The first segment selects the root package (`""` for the first, or an index).
    pub fn resolve_fragment(&self, resource: usize, fragment: &str) -> Option<ElementRef> {
        let rest = fragment.strip_prefix('/')?;
        let (root, path) = rest.split_once('/').unwrap_or((rest, ""));
        let index = if root.is_empty() { 0 } else { root.parse().ok()? };
        let package = *self.resources.get(resource)?.roots.get(index)?;
        self.resolve_in_package(package, path)
    }

    /// Resolves a name path such as `sub/Thing/feature` relative to a package.
    pub fn resolve_in_package(&self, package: PackageId, path: &str) -> Option<ElementRef> {
        let mut current = ElementRef::Package(package);
        for segment in path.split('/').filter(|s| !s.is_empty()) {
            current = match current {
                ElementRef::Package(p) => {
                    let pkg = self.package(p);
                    if let Some(sub) = pkg.subpackages.iter().find(|s| self.package(**s).name == segment) {
                        ElementRef::Package(*sub)
                    } else {
                        ElementRef::Classifier(self.find_classifier_in(p, segment)?)
                    }
                }
                ElementRef::Classifier(c) => {
                    let data = self.class_data(c)?;
                    if let Some(f) = data.features.iter().find(|f| self.feature(**f).name == segment) {
                        ElementRef::Feature(*f)
                    } else {
                        ElementRef::Operation(*data.operations.iter().find(|o| self.operation(**o).name == segment)?)
                    }
                }
                ElementRef::Feature(_) | ElementRef::Operation(_) => return None,
            };
        }
        Some(current)
    }

    pub fn is_class(&self, id: ClassifierId) -> bool {
        matches!(self.classifier(id).kind, ClassifierKind::Class(_))
    }

    pub fn is_enum(&self, id: ClassifierId) -> bool {
        matches!(self.classifier(id).kind, ClassifierKind::Enum(_))
    }

    pub fn class_data(&self, id: ClassifierId) -> Option<&ClassData> {
        match &self.classifier(id).kind {
            ClassifierKind::Class(c) => Some(c),
            _ => None,
        }
    }

    pub fn enum_literals(&self, id: ClassifierId) -> &[EnumLiteral] {
        match &self.classifier(id).kind {
            ClassifierKind::Enum(lits) => lits,
            _ => &[],
        }
    }

    pub fn enum_literal_index(&self, id: ClassifierId, name: &str) -> Option<usize> {
        self.enum_literals(id).iter().position(|l| l.name == name)
    }

    /// The value representation of a data type; `None` for classes and enums.
    pub fn data_type_kind(&self, id: ClassifierId) -> Option<DataTypeKind> {
        match &self.classifier(id).kind {
            ClassifierKind::DataType(k) => Some(*k),
            _ => None,
        }
    }

    // ---------------------------------------------------------------- names

    /// `pkg::sub::Name`.
    pub fn qualified_name(&self, id: ClassifierId) -> String {
        let c = self.classifier(id);
        format!("{}::{}", self.package_qualified_name(c.package), c.name)
    }

    pub fn package_qualified_name(&self, id: PackageId) -> String {
        let mut parts = vec![self.package(id).name.as_str()];
        let mut cur = self.package(id).super_package;
        while let Some(p) = cur {
            parts.push(&self.package(p).name);
            cur = self.package(p).super_package;
        }
        parts.reverse();
        parts.join("::")
    }

    pub fn feature_qualified_name(&self, id: FeatureId) -> String {
        let f = self.feature(id);
        format!("{}::{}", self.qualified_name(f.owner), f.name)
    }

    pub fn find_classifier_in(&self, package: PackageId, name: &str) -> Option<ClassifierId> {
        self.package(package).classifiers.iter().copied().find(|c| self.classifier(*c).name == name)
    }

    /// All classifiers with the given simple name; user packages come before Ecore.
    pub fn find_classifiers(&self, name: &str) -> Vec<ClassifierId> {
        let mut found: Vec<ClassifierId> = self.classifier_ids().filter(|c| self.classifier(*c).name == name).collect();
        found.sort_by_key(|c| self.classifier(*c).package == self.ecore_package());
        found
    }

    /// Resolves a (possibly partially) qualified classifier name such as `["library", "Book"]`.
    /// The path must match a suffix of the classifier's package path.
    pub fn find_classifiers_qualified(&self, path: &[&str]) -> Vec<ClassifierId> {
        let Some((last, pkgs)) = path.split_last() else { return Vec::new() };
        self.find_classifiers(last)
            .into_iter()
            .filter(|c| self.package_path_ends_with(self.classifier(*c).package, pkgs))
            .collect()
    }

    /// Resolves a (possibly partially) qualified package name.
    pub fn find_packages_qualified(&self, path: &[&str]) -> Vec<PackageId> {
        let Some((last, parents)) = path.split_last() else { return Vec::new() };
        self.package_ids()
            .filter(|p| self.package(*p).name == *last)
            .filter(|p| match self.package(*p).super_package {
                Some(sp) => self.package_path_ends_with(sp, parents),
                None => parents.is_empty(),
            })
            .collect()
    }

    fn package_path_ends_with(&self, package: PackageId, suffix: &[&str]) -> bool {
        let mut cur = Some(package);
        for name in suffix.iter().rev() {
            match cur {
                Some(p) if self.package(p).name == *name => cur = self.package(p).super_package,
                _ => return false,
            }
        }
        true
    }

    // ---------------------------------------------------------------- hierarchy

    pub fn super_types(&self, id: ClassifierId) -> &[ClassifierId] {
        self.class_data(id).map(|c| c.super_types.as_slice()).unwrap_or(&[])
    }

    /// All proper supertypes of a class (transitively), nearest first. Excludes `EObject`
    /// unless it is declared explicitly.
    pub fn all_super_types(&self, id: ClassifierId) -> &[ClassifierId] {
        self.cache(id).map(|c| c.all_super_types.as_slice()).unwrap_or(&[])
    }

    /// Whether `sub` is `sup` or a subtype of it. Every class conforms to `EObject`.
    pub fn conforms_to(&self, sub: ClassifierId, sup: ClassifierId) -> bool {
        sub == sup || (self.is_class(sub) && Some(sup) == self.eobject) || self.all_super_types(sub).contains(&sup)
    }

    /// Direct and indirect subclasses of a class (excluding itself).
    pub fn all_sub_types(&self, id: ClassifierId) -> Vec<ClassifierId> {
        self.classifier_ids().filter(|c| *c != id && self.is_class(*c) && self.conforms_to(*c, id)).collect()
    }

    /// All structural features of a class including inherited ones.
    pub fn all_features(&self, id: ClassifierId) -> &[FeatureId] {
        self.cache(id).map(|c| c.all_features.as_slice()).unwrap_or(&[])
    }

    pub fn lookup_feature(&self, class: ClassifierId, name: &str) -> Option<FeatureId> {
        self.cache(class)?.feature_by_name.get(name).copied()
    }

    /// All operations of a class including inherited ones (own operations first).
    pub fn all_operations(&self, id: ClassifierId) -> &[OperationId] {
        self.cache(id).map(|c| c.all_operations.as_slice()).unwrap_or(&[])
    }

    pub fn lookup_operations(&self, class: ClassifierId, name: &str) -> Vec<OperationId> {
        self.all_operations(class).iter().copied().filter(|o| self.operation(*o).name == name).collect()
    }

    fn cache(&self, id: ClassifierId) -> Option<&ClassCache> {
        self.caches.get(id.index()).and_then(Option::as_ref)
    }

    // ---------------------------------------------------------------- building
    //
    // Metamodels are normally built by the `.ecore` loader, but other front ends (such as a
    // textual metamodel syntax) build them through the same calls: add a resource, add its
    // packages, classifiers, features and operations, then call [`Metamodel::finish_resource`].

    /// Adds an (empty) resource and returns its index. Its root packages are set by
    /// [`Metamodel::finish_resource`].
    pub fn add_resource(&mut self, uri: impl Into<String>) -> usize {
        self.resources.push(EcoreResource { uri: uri.into(), aliases: Vec::new(), roots: Vec::new() });
        self.resources.len() - 1
    }

    /// Completes a resource built element by element: records its root packages, registers
    /// their nsURIs and recomputes the inheritance caches.
    pub fn finish_resource(&mut self, resource: usize, roots: Vec<PackageId>) {
        for p in &roots {
            self.register_tree(*p);
        }
        self.resources[resource].roots = roots;
        self.rebuild_caches();
    }

    pub(crate) fn register_tree(&mut self, p: PackageId) {
        self.register_ns_uri(p);
        for sub in self.package(p).subpackages.clone() {
            self.register_tree(sub);
        }
    }

    /// Adds a package (as a subpackage of `pkg.super_package`, if set).
    pub fn add_package(&mut self, pkg: Package) -> PackageId {
        let id = PackageId(self.packages.len() as u32);
        if let Some(parent) = pkg.super_package {
            self.packages[parent.index()].subpackages.push(id);
        }
        self.packages.push(pkg);
        id
    }

    pub(crate) fn register_ns_uri(&mut self, id: PackageId) {
        let uri = self.packages[id.index()].ns_uri.clone();
        if !uri.is_empty() {
            self.by_ns_uri.entry(uri).or_insert(id);
        }
    }

    /// Adds a classifier to its package.
    pub fn add_classifier(&mut self, c: Classifier) -> ClassifierId {
        let id = ClassifierId(self.classifiers.len() as u32);
        self.packages[c.package.index()].classifiers.push(id);
        self.classifiers.push(c);
        id
    }

    /// Adds a structural feature to its owning class.
    pub fn add_feature(&mut self, f: Feature) -> FeatureId {
        let id = FeatureId(self.features.len() as u32);
        if let ClassifierKind::Class(c) = &mut self.classifiers[f.owner.index()].kind {
            c.features.push(id);
        }
        self.features.push(f);
        id
    }

    /// Adds an operation to its owning class.
    pub fn add_operation(&mut self, o: Operation) -> OperationId {
        let id = OperationId(self.operations.len() as u32);
        if let ClassifierKind::Class(c) = &mut self.classifiers[o.owner.index()].kind {
            c.operations.push(id);
        }
        self.operations.push(o);
        id
    }

    pub fn package_mut(&mut self, id: PackageId) -> &mut Package {
        &mut self.packages[id.index()]
    }

    pub fn classifier_mut(&mut self, id: ClassifierId) -> &mut Classifier {
        &mut self.classifiers[id.index()]
    }

    pub fn feature_mut(&mut self, id: FeatureId) -> &mut Feature {
        &mut self.features[id.index()]
    }

    pub fn operation_mut(&mut self, id: OperationId) -> &mut Operation {
        &mut self.operations[id.index()]
    }

    /// The annotations of any metamodel element.
    pub fn annotations(&self, element: ElementRef) -> &[Annotation] {
        match element {
            ElementRef::Package(p) => &self.package(p).annotations,
            ElementRef::Classifier(c) => &self.classifier(c).annotations,
            ElementRef::Feature(f) => &self.feature(f).annotations,
            ElementRef::Operation(o) => &self.operation(o).annotations,
        }
    }

    pub fn annotations_mut(&mut self, element: ElementRef) -> &mut Vec<Annotation> {
        match element {
            ElementRef::Package(p) => &mut self.package_mut(p).annotations,
            ElementRef::Classifier(c) => &mut self.classifier_mut(c).annotations,
            ElementRef::Feature(f) => &mut self.feature_mut(f).annotations,
            ElementRef::Operation(o) => &mut self.operation_mut(o).annotations,
        }
    }

    /// Recomputes inheritance caches. Called after every load.
    pub(crate) fn rebuild_caches(&mut self) {
        let mut caches = vec![None; self.classifiers.len()];
        for (i, slot) in caches.iter_mut().enumerate() {
            let id = ClassifierId(i as u32);
            if !self.is_class(id) {
                continue;
            }
            // Breadth-first supertype closure, cycle safe.
            let mut supers: Vec<ClassifierId> = Vec::new();
            let mut queue: std::collections::VecDeque<ClassifierId> = self.super_types(id).iter().copied().collect();
            while let Some(s) = queue.pop_front() {
                if s == id || supers.contains(&s) {
                    continue;
                }
                supers.push(s);
                queue.extend(self.super_types(s).iter().copied());
            }
            // Features: farthest ancestors first, as EMF does.
            let mut all_features = Vec::new();
            for s in supers.iter().rev().chain(std::iter::once(&id)) {
                if let Some(cd) = self.class_data(*s) {
                    for f in &cd.features {
                        if !all_features.contains(f) {
                            all_features.push(*f);
                        }
                    }
                }
            }
            let mut feature_by_name = HashMap::new();
            // Later (more specific) features shadow inherited ones of the same name.
            for f in &all_features {
                feature_by_name.insert(self.feature(*f).name.clone(), *f);
            }
            let mut all_operations = Vec::new();
            for s in std::iter::once(&id).chain(supers.iter()) {
                if let Some(cd) = self.class_data(*s) {
                    all_operations.extend(cd.operations.iter().copied());
                }
            }
            *slot = Some(ClassCache { all_super_types: supers, all_features, feature_by_name, all_operations });
        }
        self.caches = caches;
    }
}
