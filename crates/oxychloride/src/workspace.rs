//! The [`Workspace`]: metamodels, models and documents loaded together.
//!
//! Documents come in four languages (see [`Language`]): Complete OCL documents add
//! constraints to loaded metamodels, OCLinEcore documents *are* metamodels (with embedded
//! OCL), and HUTN and XMI documents *are* models. The workspace therefore keeps two layers of
//! each:
//!
//! * the metamodels loaded from `.ecore` files (`base_mm`), and `mm` = those plus the
//!   packages of the OCLinEcore documents;
//! * the models loaded from XMI files (`xmi_model`), and `model` = those plus the objects
//!   of the model documents (HUTN, and XMI edited as a document).
//!
//! When a document changes, the derived layer is rebuilt (`finish`); models typed by
//! classes of an OCLinEcore document are reloaded, since those classes are rebuilt too.

use std::collections::HashMap;
use std::fmt;
use std::ops::Range as ByteRange;
use std::path::Path;

use indexmap::IndexMap;
use oxy_ecore::hutn::{self, HutnLoad};
use oxy_ecore::{ClassifierKind as EKind, LoadError, Metamodel, Model, ObjectId, Problem, XmiDocument};
use oxy_ocl::env::{OperationConstraint, SourceKind};
use oxy_ocl::oclinecore::{self, OclInEcore};
use oxy_ocl::syntax::ast::ConstraintKind as OclConstraintKind;
use oxy_ocl::syntax::{parse_document, parse_ecore_document, parse_expression};
use oxy_ocl::{ContractCheck, Environment, EnvironmentBuilder, Evaluator, LineIndex, SourceId, Status, Type, Value};

use crate::protocol::*;

/// Errors raised by workspace operations (as opposed to diagnostics about OCL sources).
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// A metamodel or model could not be loaded.
    Load(LoadError),
    /// A referenced document, resource or object does not exist.
    NotFound(String),
    /// The request itself is malformed.
    Invalid(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Load(e) => write!(f, "{e}"),
            Error::NotFound(m) | Error::Invalid(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

impl From<LoadError> for Error {
    fn from(e: LoadError) -> Self {
        Error::Load(e)
    }
}

/// Where a metamodel or model was loaded from, so the workspace can be rebuilt.
#[derive(Debug, Clone)]
enum Input {
    File(String),
    Text { uri: String, text: String },
}

#[derive(Debug, Clone)]
struct DocumentEntry {
    text: String,
    language: Language,
    /// Problems resolving `import` statements.
    import_diagnostics: Vec<oxy_ocl::Diagnostic>,
    /// OCLinEcore documents this (OCLinEcore) document imports; they are built first.
    depends_on: Vec<String>,
    line_index: LineIndex,
}

/// Options for [`Workspace::validate`].
#[derive(Debug, Clone)]
pub struct ValidateOptions {
    /// Include the constraints embedded in the metamodels (`.ecore` files and OCLinEcore
    /// documents).
    pub ecore: bool,
    /// Complete OCL documents whose constraints to include (`None`: all open documents).
    pub documents: Option<Vec<String>>,
    /// Only check these objects (`None`: all objects of all loaded models).
    pub objects: Option<Vec<ObjectRef>>,
    /// Report satisfied constraints too (otherwise only violations and errors).
    pub include_satisfied: bool,
    /// Check pre- and postconditions of operations (see `oxy_ocl::validate`).
    pub contracts: bool,
}

impl Default for ValidateOptions {
    fn default() -> Self {
        ValidateOptions { ecore: true, documents: None, objects: None, include_satisfied: false, contracts: true }
    }
}

/// A set of metamodels, instance models and documents (Complete OCL, OCLinEcore, HUTN).
///
/// This is the main entry point of the engine for Rust callers; the JSON-RPC server, the C
/// ABI and the language server are thin layers on top of it.
#[derive(Debug, Clone, Default)]
pub struct Workspace {
    /// Metamodels loaded from `.ecore` files.
    pub(crate) base_mm: Metamodel,
    /// `base_mm` plus the packages of the OCLinEcore documents.
    pub(crate) mm: Metamodel,
    /// Models loaded from XMI files.
    xmi_model: Model,
    /// `xmi_model` plus the objects of the model documents.
    model: Model,
    documents: IndexMap<String, DocumentEntry>,
    /// Built OCLinEcore documents (their elements live in `mm`).
    oclinecore: IndexMap<String, OclInEcore>,
    /// Loaded model documents, HUTN and XMI (their objects live in `model`).
    hutn: IndexMap<String, HutnLoad>,
    /// The document and range defining each object of a model document.
    locations: HashMap<ObjectId, (String, ByteRange<usize>)>,
    /// Classifiers with an id at or above this belong to OCLinEcore documents.
    document_classifiers: usize,
    ecore_inputs: Vec<Input>,
    model_inputs: Vec<Input>,
    /// Problems reloading XMI models after their metamodel changed.
    model_problems: Vec<Diagnostic>,
    /// URI prefix mappings, e.g. `pathmap://PCM_MODELS/` → a local directory.
    pub(crate) uri_mappings: Vec<(String, String)>,
    /// The package registry (see `registry.rs`), with locations made absolute.
    pub(crate) registry: Vec<RegistryEntry>,
    /// The state of each registry entry.
    pub(crate) registry_state: Vec<crate::registry::EntryState>,
    /// Changes whenever `mm` is rebuilt or grows (transformation sessions refer to it).
    pub(crate) generation: u64,
    /// Transformation sessions (see `transform.rs`).
    pub(crate) transformations: crate::transform::Sessions,
    /// `mm` must be rebuilt from `base_mm` and the OCLinEcore documents.
    mm_dirty: bool,
    /// The XMI models must be reloaded (the metamodel changed underneath them).
    xmi_stale: bool,
    /// `model` must be rebuilt from `xmi_model` and the HUTN documents.
    models_dirty: bool,
}

impl Workspace {
    pub fn new() -> Self {
        Workspace::default()
    }

    /// Maps URIs starting with `from` to `to` (typically a directory), like EMF's URI
    /// converter: referenced resources such as `pathmap://PCM_MODELS/Palladio.resourcetype`
    /// are then loaded from the mapped location.
    pub fn map_uri(&mut self, from: &str, to: &str) {
        self.uri_mappings.retain(|(f, _)| f != from);
        self.uri_mappings.push((from.to_string(), to.to_string()));
        // Resolve references that were waiting for a now-mapped resource.
        let pending: Vec<String> = self.xmi_model.unresolved_references().iter().map(|r| r.uri.clone()).collect();
        let mut warnings = Vec::new();
        let mut loaded = Vec::new();
        for uri in pending {
            self.demand_load(&uri, &mut loaded, &mut warnings);
        }
        self.models_dirty = true;
        self.finish();
    }

    /// Loads the model resource `uri` refers to, if it exists and is not loaded yet.
    fn demand_load(&mut self, uri: &str, loaded: &mut Vec<usize>, warnings: &mut Vec<Diagnostic>) {
        if self.xmi_model.resource_of_uri(uri).is_some() {
            return;
        }
        let path = self.locator().locate_prefixed(uri).unwrap_or_else(|| uri.to_string());
        if let Some(r) = self.xmi_model.resource_of_uri(&path) {
            self.xmi_model.add_alias(r, uri);
        } else if Path::new(&path).is_file() {
            let result = XmiDocument::read_file(&path)
                .map_err(Error::from)
                .and_then(|d| self.load_model_recursive(d, loaded, warnings));
            match result {
                Ok(r) => self.xmi_model.add_alias(r, uri),
                Err(e) => warnings.push(Diagnostic { severity: Severity::Warning, ..error_diagnostic(&e) }),
            }
        } else {
            return;
        }
        for r in self.xmi_model.resolve_deferred(&self.mm) {
            warnings.push(Diagnostic {
                severity: Severity::Warning,
                message: format!("cannot resolve reference '{}#{}'", r.uri, r.fragment),
                source: uri.to_string(),
                origin: Origin::File { path: uri.to_string() },
                range: Range::default(),
            });
        }
    }

    pub fn metamodel(&self) -> &Metamodel {
        &self.mm
    }

    pub fn model(&self) -> &Model {
        &self.model
    }

    /// Removes everything (the package registry too).
    pub fn reset(&mut self) {
        *self = Workspace::new();
    }

    /// Removes all models loaded from files, keeping metamodels and documents (HUTN
    /// documents stay too: they are documents).
    pub fn clear_models(&mut self) {
        self.xmi_model = Model::new();
        self.model_inputs.clear();
        self.model_problems.clear();
        self.models_dirty = true;
        self.finish();
    }

    /// Reloads all metamodels and models from their sources (e.g. after files changed on
    /// disk) and re-resolves document imports. Returns the problems encountered.
    pub fn reload(&mut self) -> Vec<Diagnostic> {
        let ecore = std::mem::take(&mut self.ecore_inputs);
        self.base_mm = Metamodel::new();
        self.mm = Metamodel::new();
        self.load_registry();
        let mut problems = Vec::new();
        for input in ecore {
            let r = match &input {
                Input::File(p) => self.load_ecore_file_inner(Path::new(p)),
                Input::Text { uri, text } => self.load_ecore_str_inner(text, uri),
            };
            if let Err(e) = r {
                problems.push(error_diagnostic(&e));
            }
        }
        // The built OCLinEcore documents refer to packages of the old `mm`; rebuild them before
        // imports are resolved against them.
        self.build_metamodel();
        let names: Vec<String> = self.documents.keys().cloned().collect();
        for name in names {
            self.resolve_document_imports(&name);
        }
        self.mm_dirty = true;
        self.xmi_stale = true;
        self.finish();
        problems.extend(self.model_problems.iter().cloned());
        problems
    }

    // ------------------------------------------------------------------ rebuilding

    fn has_oclinecore_documents(&self) -> bool {
        self.documents.values().any(|d| d.language == Language::OclInEcore)
    }

    /// Brings the derived metamodel and models up to date.
    fn finish(&mut self) {
        // Reloading XMI models can load further metamodels, which dirties `mm` again.
        for _ in 0..3 {
            if std::mem::take(&mut self.mm_dirty) {
                let typed_by_documents = self
                    .xmi_model
                    .object_ids()
                    .any(|o| self.xmi_model.class_of(o).index() >= self.document_classifiers);
                self.build_metamodel();
                if typed_by_documents {
                    self.xmi_stale = true;
                }
                self.models_dirty = true;
            }
            if std::mem::take(&mut self.xmi_stale) {
                self.reload_xmi();
                self.models_dirty = true;
            }
            if !self.mm_dirty && !self.xmi_stale {
                break;
            }
        }
        if std::mem::take(&mut self.models_dirty) {
            self.build_models();
        }
    }

    /// Rebuilds `mm` from `base_mm` and the OCLinEcore documents.
    fn build_metamodel(&mut self) {
        let mut mm = self.base_mm.clone();
        self.document_classifiers = mm.classifier_count();
        let mut built = IndexMap::new();
        for name in self.build_order() {
            let text = &self.documents[&name].text;
            let (document, diagnostics) = parse_ecore_document(text);
            built.insert(name.clone(), oclinecore::build(&mut mm, &name, text, document, diagnostics));
        }
        self.mm = mm;
        self.oclinecore = built;
        self.generation += 1;
    }

    /// OCLinEcore documents, each after the documents it imports.
    fn build_order(&self) -> Vec<String> {
        let mut pending: Vec<&String> =
            self.documents.iter().filter(|(_, d)| d.language == Language::OclInEcore).map(|(n, _)| n).collect();
        let mut order: Vec<String> = Vec::new();
        while !pending.is_empty() {
            let ready = pending
                .iter()
                .position(|n| self.documents[*n].depends_on.iter().all(|d| order.contains(d) || !pending.contains(&d)))
                .unwrap_or(0); // a cycle: build in document order
            order.push(pending.remove(ready).clone());
        }
        order
    }

    /// Reloads the XMI models from their inputs, against the current metamodel.
    fn reload_xmi(&mut self) {
        self.xmi_model = Model::new();
        self.model_problems.clear();
        for input in std::mem::take(&mut self.model_inputs) {
            let result = match &input {
                Input::File(p) => {
                    XmiDocument::read_file(p).map_err(Error::from).and_then(|d| self.load_model_document(d))
                }
                Input::Text { uri, text } => XmiDocument::parse(text.as_str(), uri.as_str())
                    .map_err(Error::from)
                    .and_then(|d| self.load_model_document(d)),
            };
            if let Err(e) = result {
                self.model_problems.push(error_diagnostic(&e));
            }
            // Inputs stay registered even if they fail now (e.g. while their metamodel
            // document has errors), so they load again once it is fixed.
            self.model_inputs.push(input);
        }
    }

    /// Rebuilds `model` from the XMI models and the model documents.
    fn build_models(&mut self) {
        let mut model = self.xmi_model.clone();
        let mut loads = IndexMap::new();
        let mut locations = HashMap::new();
        for (name, doc) in &self.documents {
            let load = match doc.language {
                Language::Hutn => hutn::load(&doc.text, name, &self.mm, &mut model),
                Language::Xmi => load_xmi_document(&doc.text, name, &self.mm, &mut model),
                Language::Ocl
                | Language::OclInEcore
                | Language::Henshin
                | Language::HenshinModel
                | Language::Qvto
                | Language::Qvtr => continue,
            };
            for o in &load.objects {
                locations.insert(o.object, (name.clone(), o.span.clone()));
            }
            loads.insert(name.clone(), load);
        }
        self.model = model;
        self.hutn = loads;
        self.locations = locations;
    }

    // ------------------------------------------------------------------ loading

    /// Loads an `.ecore` file and any `.ecore` files it references.
    pub fn load_ecore_file(&mut self, path: impl AsRef<Path>) -> Result<LoadResult, Error> {
        let result = self.load_ecore_file_inner(path.as_ref());
        self.finish();
        result
    }

    fn load_ecore_file_inner(&mut self, path: &Path) -> Result<LoadResult, Error> {
        let load = self.load_metamodel_file(path)?;
        let uri = self.base_mm.resources()[load.resource].uri.clone();
        if !self.ecore_inputs.iter().any(|i| matches!(i, Input::File(p) if *p == uri)) {
            self.ecore_inputs.push(Input::File(uri));
        }
        Ok(self.ecore_load_result(&load))
    }

    /// Loads an `.ecore` file (and the files it refers to, through the package registry) into
    /// `base_mm` and `mm`.
    pub(crate) fn load_metamodel_file(&mut self, path: &Path) -> Result<oxy_ecore::EcoreLoad, LoadError> {
        let locator = self.locator();
        let load = locator.load_file(&mut self.base_mm, path)?;
        let uri = self.base_mm.resources()[load.resource].uri.clone();
        // Keep `mm` usable right away (a model being loaded may need the new packages). A
        // conflict with an OCLinEcore document is reported for the document once `mm` is
        // rebuilt.
        let _ = locator.load_file(&mut self.mm, Path::new(&uri));
        self.generation += 1;
        self.metamodel_grew();
        Ok(load)
    }

    /// Loads `.ecore` content. `uri` identifies the resource (and is used to resolve
    /// relative references to already loaded resources).
    pub fn load_ecore_str(&mut self, text: &str, uri: &str) -> Result<LoadResult, Error> {
        let result = self.load_ecore_str_inner(text, uri);
        self.finish();
        result
    }

    fn load_ecore_str_inner(&mut self, text: &str, uri: &str) -> Result<LoadResult, Error> {
        let locator = self.locator();
        let load = locator.load_str(&mut self.base_mm, text, uri)?;
        self.ecore_inputs.push(Input::Text { uri: uri.to_string(), text: text.to_string() });
        let _ = locator.load_str(&mut self.mm, text, uri);
        self.generation += 1;
        self.metamodel_grew();
        Ok(self.ecore_load_result(&load))
    }

    /// After `.ecore` content was added to both `base_mm` and `mm`.
    fn metamodel_grew(&mut self) {
        if self.has_oclinecore_documents() {
            // `mm` now has packages after the documents' ones; rebuild it in order.
            self.mm_dirty = true;
        } else {
            self.document_classifiers = self.mm.classifier_count();
        }
        // HUTN documents may refer to the new packages.
        self.models_dirty = true;
    }

    fn ecore_load_result(&self, load: &oxy_ecore::EcoreLoad) -> LoadResult {
        let mm = &self.base_mm;
        let mut packages: Vec<String> = load.packages.iter().map(|p| mm.package(*p).ns_uri.clone()).collect();
        for dep in &load.dependencies {
            packages.extend(mm.resources()[*dep].roots.iter().map(|p| mm.package(*p).ns_uri.clone()));
        }
        LoadResult {
            uri: mm.resources()[load.resource].uri.clone(),
            packages,
            objects: None,
            warnings: load.warnings.iter().map(load_diagnostic).collect(),
        }
    }

    /// Loads an XMI model file. Metamodels named in `xsi:schemaLocation` are loaded
    /// automatically when they are not loaded yet.
    pub fn load_model_file(&mut self, path: impl AsRef<Path>) -> Result<LoadResult, Error> {
        let result = XmiDocument::read_file(path).map_err(Error::from).and_then(|doc| self.load_model_document(doc));
        if let Ok(r) = &result
            && !self.model_inputs.iter().any(|i| matches!(i, Input::File(p) if *p == r.uri))
        {
            self.model_inputs.push(Input::File(r.uri.clone()));
        }
        self.models_dirty = true;
        self.finish();
        result
    }

    /// Loads XMI model content; `uri` identifies the resource.
    pub fn load_model_str(&mut self, text: &str, uri: &str) -> Result<LoadResult, Error> {
        let result = XmiDocument::parse(text, uri).map_err(Error::from).and_then(|doc| self.load_model_document(doc));
        if result.is_ok() {
            self.model_inputs.push(Input::Text { uri: uri.to_string(), text: text.to_string() });
        }
        self.models_dirty = true;
        self.finish();
        result
    }

    fn load_model_document(&mut self, doc: XmiDocument) -> Result<LoadResult, Error> {
        let mut loaded = Vec::new();
        let mut warnings = Vec::new();
        let resource = self.load_model_recursive(doc, &mut loaded, &mut warnings)?;
        // Summarize references into resources that could not be loaded (e.g. `pathmap:` URIs).
        let mut unresolved: IndexMap<&str, usize> = IndexMap::new();
        for r in self.xmi_model.unresolved_references() {
            if loaded.contains(&self.xmi_model.object(r.object).resource) {
                *unresolved.entry(r.uri.as_str()).or_default() += 1;
            }
        }
        let uri = self.xmi_model.resources()[resource].uri.clone();
        for (target, count) in unresolved {
            warnings.push(Diagnostic {
                severity: Severity::Warning,
                message: format!(
                    "{count} reference{} into '{target}' cannot be resolved (the resource is not loaded)",
                    if count == 1 { "" } else { "s" }
                ),
                source: uri.clone(),
                origin: Origin::File { path: uri.clone() },
                range: Range::default(),
            });
        }
        let res = &self.xmi_model.resources()[resource];
        let objects = res.roots.iter().map(|r| 1 + self.xmi_model.all_contents(&self.mm, *r).len()).sum();
        Ok(LoadResult { uri, packages: Vec::new(), objects: Some(objects), warnings })
    }

    /// Loads a model and, like EMF's resource sets, the model files it references.
    fn load_model_recursive(
        &mut self,
        doc: XmiDocument,
        loaded: &mut Vec<usize>,
        warnings: &mut Vec<Diagnostic>,
    ) -> Result<usize, Error> {
        let missing: Vec<String> =
            doc.used_ns_uris().into_iter().filter(|ns| self.mm.package_by_ns_uri(ns).is_none()).collect();
        if !missing.is_empty() {
            let locator = self.locator();
            for (ns, location) in doc.schema_locations() {
                let location = locator.locate(&location).unwrap_or(location);
                if missing.contains(&ns) && Path::new(&location).exists() {
                    let r = self.load_ecore_file_inner(Path::new(&location))?;
                    warnings.extend(r.warnings);
                }
            }
        }
        let load = doc.load(&self.mm, &mut self.xmi_model)?;
        loaded.push(load.resource);
        warnings.extend(load.warnings.iter().map(load_diagnostic));
        for uri in doc.referenced_uris() {
            self.demand_load(&uri, loaded, warnings);
        }
        Ok(load.resource)
    }

    // ------------------------------------------------------------------ documents

    /// Adds or replaces a document; its language is derived from the name's extension
    /// (see [`Language::from_path`]). `name` is normally the document's path: relative
    /// `import`s are resolved against it and loaded if needed. Returns the diagnostics of
    /// the document.
    pub fn open_document(&mut self, name: &str, text: &str) -> Vec<Diagnostic> {
        self.open_document_as(name, text, Language::from_path(name))
    }

    /// Adds or replaces a document of the given language.
    pub fn open_document_as(&mut self, name: &str, text: &str, language: Language) -> Vec<Diagnostic> {
        let previous = self.documents.get(name).map(|d| d.language);
        self.documents.insert(
            name.to_string(),
            DocumentEntry {
                text: text.to_string(),
                language,
                import_diagnostics: Vec::new(),
                depends_on: Vec::new(),
                line_index: LineIndex::new(text),
            },
        );
        self.resolve_document_imports(name);
        self.document_changed(language, previous);
        self.finish();
        self.diagnostics(Some(name))
    }

    pub fn close_document(&mut self, name: &str) -> bool {
        match self.documents.shift_remove(name) {
            Some(doc) => {
                self.document_changed(doc.language, None);
                self.finish();
                true
            }
            None => false,
        }
    }

    fn document_changed(&mut self, language: Language, previous: Option<Language>) {
        for l in std::iter::once(language).chain(previous) {
            match l {
                Language::OclInEcore => self.mm_dirty = true,
                Language::Hutn | Language::Xmi => self.models_dirty = true,
                Language::Ocl | Language::Henshin | Language::HenshinModel | Language::Qvto | Language::Qvtr => {}
            }
        }
    }

    pub fn document_names(&self) -> impl Iterator<Item = &str> {
        self.documents.keys().map(String::as_str)
    }

    pub fn document_text(&self, name: &str) -> Option<&str> {
        self.documents.get(name).map(|d| d.text.as_str())
    }

    pub fn document_language(&self, name: &str) -> Option<Language> {
        self.documents.get(name).map(|d| d.language)
    }

    /// The built metamodel of an OCLinEcore document.
    pub fn oclinecore_document(&self, name: &str) -> Option<&OclInEcore> {
        self.oclinecore.get(name)
    }

    /// The loaded objects of a HUTN document.
    pub fn hutn_document(&self, name: &str) -> Option<&HutnLoad> {
        self.hutn.get(name)
    }

    fn resolve_document_imports(&mut self, name: &str) {
        let Some(doc) = self.documents.get(name) else { return };
        let text = doc.text.clone();
        let language = doc.language;
        let (diagnostics, depends_on) = match language {
            Language::Ocl => {
                let (parsed, _) = parse_document(&text);
                self.resolve_imports(name, &parsed.imports)
            }
            Language::OclInEcore => {
                let (parsed, _) = parse_ecore_document(&text);
                self.resolve_imports(name, &parsed.imports)
            }
            Language::Hutn | Language::Henshin => (Vec::new(), Vec::new()),
            Language::HenshinModel => {
                self.load_henshin_imports(name, &text);
                (Vec::new(), Vec::new())
            }
            Language::Xmi => {
                self.load_schema_locations(name, &text);
                (Vec::new(), Vec::new())
            }
            Language::Qvto | Language::Qvtr => {
                self.load_qvt_imports(name, &text, language);
                (Vec::new(), Vec::new())
            }
        };
        if let Some(doc) = self.documents.get_mut(name) {
            doc.import_diagnostics = diagnostics;
            doc.depends_on = depends_on;
        }
    }

    /// Loads the `.ecore` files that an XMI document's `xsi:schemaLocation` names, for the
    /// packages it uses that are not loaded yet (problems are reported when the document loads).
    fn load_schema_locations(&mut self, name: &str, text: &str) {
        let Ok(doc) = XmiDocument::parse(text, name) else { return };
        let missing: Vec<String> =
            doc.used_ns_uris().into_iter().filter(|ns| self.mm.package_by_ns_uri(ns).is_none()).collect();
        let locator = self.locator();
        for (ns, location) in doc.schema_locations() {
            let location = locator.locate(&location).unwrap_or(location);
            if missing.contains(&ns) && location.ends_with(".ecore") && Path::new(&location).is_file() {
                let _ = self.load_ecore_file_inner(Path::new(&location));
            }
        }
    }

    /// Loads the `.ecore` files a document imports. Imports of OCLinEcore documents must
    /// name open documents; those are returned as dependencies.
    fn resolve_imports(
        &mut self,
        name: &str,
        imports: &[oxy_ocl::syntax::ast::Import],
    ) -> (Vec<oxy_ocl::Diagnostic>, Vec<String>) {
        let mut diags = Vec::new();
        let mut depends_on = Vec::new();
        for import in imports {
            let target = import.uri.split('#').next().unwrap_or("");
            if target.is_empty() || target == oxy_ecore::ECORE_NS_URI {
                continue;
            }
            // A loaded package's nsURI, or a URI a loaded file is known by (e.g. a registered one).
            if self.base_mm.package_by_ns_uri(target).is_some() || self.base_mm.resource_of_uri(target).is_some() {
                continue;
            }
            // A package of an OCLinEcore document, by nsURI.
            if let Some((doc, _)) =
                self.oclinecore.iter().find(|(_, b)| b.packages.iter().any(|p| self.mm.package(*p).ns_uri == target))
            {
                depends_on.push(doc.clone());
                continue;
            }
            let path = self
                .locator()
                .locate(target)
                .unwrap_or_else(|| oxy_ecore::ecore_loader::resolve_relative(name, target));
            if Language::from_path(&path) == Language::OclInEcore {
                match self.documents.keys().find(|d| *d == &path || Path::new(d.as_str()) == Path::new(&path)) {
                    Some(d) => depends_on.push(d.clone()),
                    None => diags.push(oxy_ocl::Diagnostic::error(
                        import.span,
                        format!("open '{}' as a document to use its metamodel", import.uri),
                    )),
                }
                continue;
            }
            if self.base_mm.resource_of_uri(&path).is_some() {
                continue;
            }
            if !Path::new(&path).exists() {
                // Maybe a platform: URI or nsURI whose package was loaded by file name.
                let file = target.rsplit('/').next().unwrap_or("");
                let known = self.base_mm.resources().iter().any(|r| r.uri.rsplit(['/', '\\']).next() == Some(file));
                if !known {
                    let hint =
                        if target.contains(':') { ": register a file for it in the package registry" } else { "" };
                    diags.push(oxy_ocl::Diagnostic::error(
                        import.span,
                        format!("cannot find imported metamodel '{}'{hint}", import.uri),
                    ));
                }
                continue;
            }
            match self.load_ecore_file_inner(Path::new(&path)) {
                Ok(r) => {
                    for w in r.warnings {
                        diags.push(oxy_ocl::Diagnostic::warning(import.span, format!("{}: {}", w.source, w.message)));
                    }
                }
                Err(e) => diags.push(oxy_ocl::Diagnostic::error(import.span, e.to_string())),
            }
        }
        (diags, depends_on)
    }

    // ------------------------------------------------------------------ compiling

    /// Whether a package was defined by an OCLinEcore document.
    fn is_document_package(&self, p: oxy_ecore::PackageId) -> bool {
        let resource = self.mm.package(p).resource;
        self.oclinecore.values().any(|b| b.resource == resource)
    }

    /// Compiles OCL sources into an evaluation environment: with `ecore`, the OCL embedded
    /// in the metamodels (`.ecore` files and OCLinEcore documents), and the selected
    /// Complete OCL documents (`None`: all).
    pub fn compile(&self, ecore: bool, documents: Option<&[String]>) -> Compiled {
        let mut builder = EnvironmentBuilder::new(&self.mm);
        let mut document_sources = Vec::new();
        if ecore {
            let roots: Vec<oxy_ecore::PackageId> =
                self.mm.user_root_packages().filter(|p| !self.is_document_package(*p)).collect();
            for p in roots {
                builder.add_package_constraints(p);
            }
            for (name, built) in &self.oclinecore {
                let id = builder.add_oclinecore(name, &self.documents[name].text, built);
                document_sources.push((name.clone(), id));
            }
        }
        for (name, doc) in &self.documents {
            if doc.language != Language::Ocl || documents.is_some_and(|d| !d.iter().any(|n| n == name)) {
                continue;
            }
            let id = builder.add_document(name, &doc.text);
            document_sources.push((name.clone(), id));
        }
        Compiled { env: builder.build(), document_sources }
    }

    /// Diagnostics of one document (`Some(name)`), or of everything: all documents, the
    /// OCL embedded in the loaded metamodels, and models that failed to reload (`None`).
    ///
    /// For HUTN documents these include the constraints (invariants, pre- and
    /// postconditions) that the document's objects violate.
    pub fn diagnostics(&self, document: Option<&str>) -> Vec<Diagnostic> {
        // Embedded OCL is always compiled so that documents can use operation bodies and
        // derived features defined there.
        let compiled = self.compile(true, None);
        let mut out = Vec::new();
        if document.is_none() {
            out.extend(self.model_problems.iter().cloned());
        }
        for (name, doc) in &self.documents {
            if document.is_some_and(|d| d != name) {
                continue;
            }
            if doc.language.is_model() {
                out.extend(self.model_document_diagnostics(name, &compiled));
            } else if doc.language.is_transformation() {
                out.extend(self.henshin_diagnostics(name));
            } else if doc.language.is_qvt() {
                out.extend(self.qvt_diagnostics(name, &compiled));
            } else if let Some(src) = compiled.source_of(name) {
                let source = compiled.env.source(src);
                out.extend(doc.import_diagnostics.iter().map(|d| convert_diagnostic(source, d)));
            }
        }
        for d in compiled.env.diagnostics() {
            let source = compiled.env.source(d.source);
            let include = match (&source.kind, document) {
                (SourceKind::Document, Some(name)) => source.name == name,
                (_, Some(_)) => false,
                (_, None) => true,
            };
            if include {
                out.push(convert_diagnostic(source, &d.diagnostic));
            }
        }
        out
    }

    /// A range in a document, from byte offsets.
    pub(crate) fn document_range(&self, document: &str, span: &ByteRange<usize>) -> Range {
        let Some(doc) = self.documents.get(document) else { return Range::default() };
        let p = |o: usize| {
            let p = doc.line_index.position(&doc.text, o);
            Position { line: p.line, column: p.column }
        };
        Range { start: p(span.start), end: p(span.end) }
    }

    fn document_diagnostic(
        &self,
        name: &str,
        severity: Severity,
        message: String,
        span: &ByteRange<usize>,
    ) -> Diagnostic {
        Diagnostic {
            severity,
            message,
            source: name.to_string(),
            origin: Origin::Document { name: name.to_string() },
            range: self.document_range(name, span),
        }
    }

    /// The problems loading a model document (syntax errors, unknown classes and features,
    /// unresolved references, unset required features), without constraint violations.
    pub fn model_problems(&self, name: &str) -> Vec<Diagnostic> {
        let Some(load) = self.hutn.get(name) else { return Vec::new() };
        load.problems
            .iter()
            .map(|p: &Problem| {
                let severity = if p.is_error() { Severity::Error } else { Severity::Warning };
                self.document_diagnostic(name, severity, p.message.clone(), &p.span)
            })
            .collect()
    }

    /// Load problems of a model document, and the constraints its objects violate.
    fn model_document_diagnostics(&self, name: &str, compiled: &Compiled) -> Vec<Diagnostic> {
        let Some(load) = self.hutn.get(name) else { return Vec::new() };
        let diagnostic = |severity: Severity, message: String, span: &ByteRange<usize>| {
            self.document_diagnostic(name, severity, message, span)
        };
        let mut out = self.model_problems(name);
        // Constraints are only checked once the model itself is free of errors.
        if load.objects.is_empty() || load.problems.iter().any(Problem::is_error) {
            return out;
        }
        let env = &compiled.env;
        let objects: Vec<ObjectId> = load.objects.iter().map(|o| o.object).collect();
        let name_span = |o: ObjectId| load.objects.iter().find(|x| x.object == o).map(|x| x.name.clone());
        let validation = oxy_ocl::validate_model(&self.mm, &self.model, env, &objects, true);
        for r in &validation.invariants {
            let Some(span) = name_span(r.object) else { continue };
            let inv = &env.invariants()[r.invariant];
            let what = format!(
                "invariant {} of {}",
                inv.name.as_deref().map_or_else(|| "<anonymous>".to_string(), |n| format!("'{n}'")),
                self.mm.qualified_name(inv.context)
            );
            match &r.status {
                Status::Satisfied => {}
                Status::Violated => out.push(diagnostic(
                    Severity::Error,
                    format!("{what} is violated{}", r.message.as_ref().map(|m| format!(": {m}")).unwrap_or_default()),
                    &span,
                )),
                Status::Error(m) => {
                    out.push(diagnostic(Severity::Warning, format!("{what} cannot be checked: {m}"), &span))
                }
            }
        }
        for c in &validation.contracts {
            let Some(span) = name_span(c.object) else { continue };
            let oc = &env.operation_constraints()[c.constraint];
            let what = format!(
                "{} {}of {}::{}",
                if oc.kind == OclConstraintKind::Pre { "precondition" } else { "postcondition" },
                oc.name.as_deref().map(|n| format!("'{n}' ")).unwrap_or_default(),
                self.mm.classifier(oc.class).name,
                oc.operation
            );
            let call = self.call_text(oc, &c.args);
            match &c.status {
                Status::Satisfied => {}
                Status::Violated => out.push(diagnostic(
                    Severity::Error,
                    format!(
                        "{what} is violated by {call}{}",
                        c.message.as_ref().map(|m| format!(": {m}")).unwrap_or_default()
                    ),
                    &span,
                )),
                Status::Error(m) => {
                    out.push(diagnostic(Severity::Warning, format!("{what} cannot be checked for {call}: {m}"), &span))
                }
            }
        }
        out
    }

    // ------------------------------------------------------------------ validation

    /// Whether the constraints of a source are selected by `options` (Complete OCL documents
    /// were already selected when compiling).
    fn is_selected(&self, env: &Environment, source: SourceId, options: &ValidateOptions) -> bool {
        let src = env.source(source);
        let is_ocl_document = matches!(src.kind, SourceKind::Document)
            && self.documents.get(&src.name).is_some_and(|d| d.language == Language::Ocl);
        is_ocl_document || options.ecore
    }

    fn constraint_info(
        &self,
        env: &Environment,
        source: SourceId,
        span: oxy_ocl::Span,
    ) -> (String, Origin, Range, String) {
        let source = env.source(source);
        (source.name.clone(), origin(&source.kind, &source.name), range(source, span), source.snippet(span).to_string())
    }

    fn invariant_info(&self, env: &Environment, inv: &oxy_ocl::Invariant) -> ConstraintInfo {
        let (source, origin, range, text) = self.constraint_info(env, inv.body.source, inv.span);
        ConstraintInfo {
            name: inv.name.clone().unwrap_or_else(|| "<anonymous>".into()),
            kind: ConstraintKind::Invariant,
            context: self.mm.qualified_name(inv.context),
            operation: None,
            source,
            origin,
            range,
            text,
        }
    }

    fn contract_info(&self, env: &Environment, oc: &OperationConstraint) -> ConstraintInfo {
        let (source, origin, range, text) = self.constraint_info(env, oc.body.source, oc.span);
        let params: Vec<String> = oc.params.iter().map(|(n, t)| format!("{n} : {}", t.display(&self.mm))).collect();
        let result = match &oc.result {
            Type::OclVoid => String::new(),
            t => format!(" : {}", t.display(&self.mm)),
        };
        ConstraintInfo {
            name: oc.name.clone().unwrap_or_else(|| "<anonymous>".into()),
            kind: if oc.kind == OclConstraintKind::Pre {
                ConstraintKind::Precondition
            } else {
                ConstraintKind::Postcondition
            },
            context: self.mm.qualified_name(oc.class),
            operation: Some(format!("{}({}){result}", oc.operation, params.join(", "))),
            source,
            origin,
            range,
            text,
        }
    }

    /// `operation(arg, ...)` with the arguments rendered.
    fn call_text(&self, oc: &OperationConstraint, args: &[Value]) -> String {
        let args: Vec<String> = args.iter().map(|a| a.render(&self.mm, &self.model)).collect();
        format!("{}({})", oc.operation, args.join(", "))
    }

    /// Evaluates invariants (and pre- and postconditions) on the loaded models.
    pub fn validate(&self, options: &ValidateOptions) -> Result<ValidationReport, Error> {
        if let Some(docs) = &options.documents {
            for d in docs {
                if !self.documents.contains_key(d) {
                    return Err(Error::NotFound(format!("no open document named '{d}'")));
                }
            }
        }
        // Embedded OCL (bodies, derivations) is always needed for evaluation, but its
        // constraints are only checked when requested.
        let compiled = self.compile(true, options.documents.as_deref());
        let env = &compiled.env;
        let objects: Vec<ObjectId> = match &options.objects {
            Some(refs) => refs.iter().map(|r| self.resolve_object(r)).collect::<Result<_, _>>()?,
            None => self.model.object_ids().collect(),
        };
        let mut constraints = Vec::new();
        let mut invariant_index = vec![None; env.invariants().len()];
        for (i, inv) in env.invariants().iter().enumerate() {
            if self.is_selected(env, inv.body.source, options) {
                invariant_index[i] = Some(constraints.len());
                constraints.push(self.invariant_info(env, inv));
            }
        }
        let mut contract_index = vec![None; env.operation_constraints().len()];
        if options.contracts {
            for (i, oc) in env.operation_constraints().iter().enumerate() {
                if self.is_selected(env, oc.body.source, options) {
                    contract_index[i] = Some(constraints.len());
                    constraints.push(self.contract_info(env, oc));
                }
            }
        }
        let mut summary =
            ValidationSummary { constraints: constraints.len(), objects: objects.len(), ..Default::default() };
        let validation = oxy_ocl::validate_model(&self.mm, &self.model, env, &objects, options.contracts);
        let mut results = Vec::new();
        let mut record = |constraint: usize,
                          object: ObjectId,
                          call: Option<String>,
                          status: &Status,
                          message: Option<String>| {
            summary.evaluations += 1;
            let (status, message) = match status {
                Status::Satisfied => {
                    summary.satisfied += 1;
                    if !options.include_satisfied {
                        return;
                    }
                    (ResultStatus::Satisfied, None)
                }
                Status::Violated => {
                    summary.violated += 1;
                    (ResultStatus::Violated, message)
                }
                Status::Error(m) => {
                    summary.errors += 1;
                    (ResultStatus::Error, Some(m.clone()))
                }
            };
            results.push(ValidationResult { constraint, object: self.object_summary(object), call, status, message });
        };
        for r in &validation.invariants {
            if let Some(c) = invariant_index[r.invariant] {
                record(c, r.object, None, &r.status, r.message.clone());
            }
        }
        for c in &validation.contracts {
            if let Some(index) = contract_index[c.constraint] {
                let call = self.call_text(&env.operation_constraints()[c.constraint], &c.args);
                record(index, c.object, Some(call), &c.status, c.message.clone());
            }
        }
        let selected_operations: Vec<oxy_ecore::OperationId> = env
            .operation_constraints()
            .iter()
            .zip(&contract_index)
            .filter(|(_, i)| i.is_some())
            .map(|(oc, _)| oc.operation_id)
            .collect();
        let notes = validation
            .unchecked
            .iter()
            .filter(|u| selected_operations.contains(&u.operation))
            .map(|u| {
                let op = self.mm.operation(u.operation);
                format!("{}::{}(): {}", self.mm.qualified_name(op.owner), op.name, u.reason)
            })
            .collect();
        let mut diagnostics: Vec<Diagnostic> = Vec::new();
        for (name, doc) in &self.documents {
            if let Some(src) = compiled.source_of(name) {
                let source = env.source(src);
                diagnostics.extend(doc.import_diagnostics.iter().map(|d| convert_diagnostic(source, d)));
            }
        }
        diagnostics.extend(
            env.diagnostics()
                .iter()
                .filter(|d| self.is_selected(env, d.source, options))
                .map(|d| convert_diagnostic(env.source(d.source), &d.diagnostic)),
        );
        Ok(ValidationReport { constraints, results, summary, diagnostics, notes })
    }

    // ------------------------------------------------------------------ evaluation

    /// Evaluates an OCL expression, with `self` bound to `context` if given. All open
    /// documents' definitions are available. Pre- and postconditions of the operations the
    /// expression calls are checked; failures are reported in the result.
    pub fn evaluate(&self, expression: &str, context: Option<&ObjectRef>) -> Result<EvaluationResult, Error> {
        let self_object = context.map(|c| self.resolve_object(c)).transpose()?;
        let compiled = self.compile(true, None);
        let env = &compiled.env;
        let (expr, parse_diags) = parse_expression(expression);
        let source = oxy_ocl::Source::new("<expression>", expression, SourceKind::Expression);
        let self_class = self_object.map(|o| self.model.class_of(o));
        let (ty, check_diags) = env.check_expression(&self.mm, &expr, self_class);
        let diagnostics: Vec<Diagnostic> = parse_diags
            .iter()
            .chain(check_diags.iter())
            .map(|d| Diagnostic {
                severity: severity(d.severity),
                message: d.message.clone(),
                source: source.name.clone(),
                origin: Origin::Expression,
                range: range(&source, d.span),
            })
            .collect();
        let mut evaluator = Evaluator::new(&self.mm, &self.model, env);
        let value = evaluator.evaluate(&expr, self_object.map(Value::Object), &[]);
        let contracts = self.contract_results(env, evaluator.take_contract_checks());
        Ok(EvaluationResult {
            display: value.render(&self.mm, &self.model),
            value: self.value_to_json(&value),
            ty: ty.display(&self.mm).to_string(),
            invalid: value.is_invalid(),
            diagnostics,
            contracts,
        })
    }

    fn contract_results(&self, env: &Environment, checks: Vec<ContractCheck>) -> Vec<ContractResult> {
        let mut seen = std::collections::HashSet::new();
        checks
            .into_iter()
            .filter(|c| seen.insert((c.constraint, c.object, c.args.iter().map(Value::key).collect::<Vec<_>>())))
            .map(|c| {
                let oc = &env.operation_constraints()[c.constraint];
                let (status, message) = match c.status {
                    Status::Satisfied => (ResultStatus::Satisfied, None),
                    Status::Violated => (ResultStatus::Violated, c.message),
                    Status::Error(m) => (ResultStatus::Error, Some(m)),
                };
                ContractResult {
                    constraint: self.contract_info(env, oc),
                    object: self.object_summary(c.object),
                    call: self.call_text(oc, &c.args),
                    status,
                    message,
                }
            })
            .collect()
    }

    /// A structured JSON rendering of a runtime value.
    pub fn value_to_json(&self, value: &Value) -> serde_json::Value {
        use serde_json::json;
        match value {
            Value::Null => serde_json::Value::Null,
            Value::Invalid(m) => json!({ "invalid": &**m }),
            Value::Boolean(b) => json!(b),
            Value::Integer(i) => json!(i),
            Value::Real(r) => serde_json::Number::from_f64(*r).map_or_else(|| json!(r.to_string()), Into::into),
            Value::String(s) => json!(&**s),
            Value::Unlimited => json!("*"),
            Value::Object(o) => json!({ "object": self.object_summary(*o) }),
            Value::Enum(e, l) => json!({
                "enum": self.mm.qualified_name(*e),
                "literal": self.mm.enum_literals(*e).get(*l as usize).map(|l| l.name.as_str()),
            }),
            Value::Collection(c) => json!({
                "kind": c.kind.name(),
                "items": c.items.iter().map(|v| self.value_to_json(v)).collect::<Vec<_>>(),
            }),
            Value::Tuple(parts) => json!({
                "tuple": parts.iter().map(|(n, v)| (n.to_string(), self.value_to_json(v))).collect::<serde_json::Map<_, _>>(),
            }),
            Value::Type(t) => json!({ "type": t.display(&self.mm).to_string() }),
            Value::Model(m) => json!({ "model": &**m }),
        }
    }

    // ------------------------------------------------------------------ objects

    /// Finds an object by resource URI (default: first model resource) and fragment.
    pub fn resolve_object(&self, r: &ObjectRef) -> Result<ObjectId, Error> {
        let resource = match &r.resource {
            Some(uri) => self.resource_index(uri)?,
            None if self.model.resources().is_empty() => return Err(Error::NotFound("no model is loaded".into())),
            None => 0,
        };
        self.model
            .resolve_fragment(&self.mm, resource, &r.fragment)
            .ok_or_else(|| Error::NotFound(format!("no object at '{}'", r.fragment)))
    }

    /// A model resource by URI, a path that canonicalizes to it, or its file name.
    pub(crate) fn resource_index(&self, uri: &str) -> Result<usize, Error> {
        self.model
            .resource_of_uri(uri)
            .or_else(|| {
                let canonical = oxy_ecore::ecore_loader::canonical_uri(Path::new(uri));
                self.model.resource_of_uri(&canonical).or_else(|| {
                    self.model.resources().iter().position(|res| res.uri.rsplit(['/', '\\']).next() == Some(uri))
                })
            })
            .ok_or_else(|| Error::NotFound(format!("no model resource '{uri}' is loaded")))
    }

    /// The document and range defining an object of a HUTN document.
    pub fn object_location(&self, o: ObjectId) -> Option<(&str, Range)> {
        let (doc, span) = self.locations.get(&o)?;
        Some((doc.as_str(), self.document_range(doc, span)))
    }

    pub fn object_summary(&self, o: ObjectId) -> ObjectSummary {
        let resource = self.model.object(o).resource;
        ObjectSummary {
            resource: self.model.resources()[resource].uri.clone(),
            fragment: self.model.uri_fragment(&self.mm, o),
            class: self.mm.qualified_name(self.model.class_of(o)),
            label: self.model.label(&self.mm, o),
            range: self.object_location(o).map(|(_, r)| r),
        }
    }

    // ------------------------------------------------------------------ conversion

    /// Exports a document in its interchange format: an OCLinEcore document as an `.ecore`
    /// file, a HUTN document as an XMI file, a Henshin text as a `.henshin` file (and a
    /// `.henshin` file as a Henshin text). `target` is the path the result will be written
    /// to; it is used to make references to `.ecore` files relative.
    pub fn export_document(&self, name: &str, target: Option<&str>) -> Result<ExportResult, Error> {
        let doc =
            self.documents.get(name).ok_or_else(|| Error::NotFound(format!("no open document named '{name}'")))?;
        match doc.language {
            Language::OclInEcore => {
                let built =
                    self.oclinecore.get(name).ok_or_else(|| Error::NotFound(format!("'{name}' is not built")))?;
                Ok(ExportResult { text: self.mm.write_ecore(built.resource), format: "ecore".into() })
            }
            Language::Hutn => {
                let load = self.hutn.get(name).ok_or_else(|| Error::NotFound(format!("'{name}' is not loaded")))?;
                let locations = self.schema_locations(target.unwrap_or(name));
                let text = oxy_ecore::xmi_writer::write_resource(&self.mm, &self.model, load.resource, &locations);
                Ok(ExportResult { text, format: "xmi".into() })
            }
            Language::Ocl => Err(Error::Invalid("Complete OCL documents cannot be exported".into())),
            Language::Henshin | Language::HenshinModel => self.convert_transformation(name, target),
            Language::Xmi => Err(Error::Invalid("XMI documents are XMI already".into())),
            Language::Qvto | Language::Qvtr => Err(Error::Invalid("QVT transformations cannot be exported".into())),
        }
    }

    /// `xsi:schemaLocation`s of the `.ecore` files of the metamodels, relative to `from`.
    pub(crate) fn schema_locations(&self, from: &str) -> Vec<(String, String)> {
        let mut locations = Vec::new();
        for p in self.mm.package_ids() {
            let resource = &self.mm.resources()[self.mm.package(p).resource].uri;
            if resource.ends_with(".ecore") && Path::new(resource).is_absolute() {
                let relative = oxy_ecore::ecore_writer::relative_path(from, resource);
                locations.push((self.mm.package(p).ns_uri.clone(), relative));
            }
        }
        locations
    }

    /// Converts a file to a textual document: an `.ecore` file to OCLinEcore, an XMI model
    /// to HUTN, a `.henshin` file to Henshin's textual syntax (and a Henshin text to a
    /// `.henshin` file). The workspace itself is not changed.
    pub fn convert_file(&self, path: &str, to: Language) -> Result<String, Error> {
        match to {
            Language::OclInEcore => {
                let mut mm = self.base_mm.clone();
                let load = self.locator().load_file(&mut mm, Path::new(path))?;
                Ok(oclinecore::print(&mm, load.resource))
            }
            Language::Hutn => {
                let doc = XmiDocument::read_file(path)?;
                let mut mm = self.mm.clone();
                let locator = self.locator();
                for (ns, location) in doc.schema_locations() {
                    let location = locator.locate(&location).unwrap_or(location);
                    if mm.package_by_ns_uri(&ns).is_none() && Path::new(&location).exists() {
                        locator.load_file(&mut mm, Path::new(&location))?;
                    }
                }
                let mut model = Model::new();
                let load = doc.load(&mm, &mut model)?;
                Ok(hutn::write(&mm, &model, load.resource))
            }
            Language::Henshin | Language::HenshinModel => self.convert_transformation_file(path, to),
            Language::Ocl | Language::Xmi | Language::Qvto | Language::Qvtr => {
                Err(Error::Invalid(format!("files cannot be converted to {}", to.extension())))
            }
        }
    }

    /// XML Schemas for XMI models of the loaded metamodels: one per package with an nsURI
    /// (including Ecore's, which `.ecore` files and Ecore-typed features need), and the XMI
    /// schema. They import each other by file name, so they belong in one directory.
    pub fn schemas(&self) -> Vec<SchemaFile> {
        use oxy_ecore::xsd_writer;
        let mm = &self.mm;
        let mut files = vec![SchemaFile {
            ns_uri: Some(oxy_ecore::xml::XMI_NS.to_string()),
            file_name: xsd_writer::XMI_SCHEMA_FILE.to_string(),
            text: xsd_writer::XMI_SCHEMA.to_string(),
        }];
        for p in mm.package_ids() {
            let ns_uri = &mm.package(p).ns_uri;
            if ns_uri.is_empty() {
                continue;
            }
            files.push(SchemaFile {
                ns_uri: Some(ns_uri.clone()),
                file_name: xsd_writer::schema_file_name(mm, p),
                text: xsd_writer::write_schema(mm, p),
            });
        }
        files
    }

    // ------------------------------------------------------------------ browsing

    /// The loaded metamodels (excluding Ecore itself), including those of OCLinEcore
    /// documents.
    pub fn packages(&self) -> Vec<PackageInfo> {
        let compiled = self.compile(true, Some(&[][..]));
        self.mm.user_root_packages().map(|p| self.package_info(&compiled.env, p)).collect()
    }

    fn package_info(&self, env: &Environment, p: oxy_ecore::PackageId) -> PackageInfo {
        let mm = &self.mm;
        let pkg = mm.package(p);
        let type_name = |ty: Option<oxy_ecore::ClassifierId>, m: &oxy_ecore::Multiplicity| {
            Type::of_typed(mm, ty, m).display(mm).to_string()
        };
        let text = |b: &oxy_ocl::env::Body| env.source(b.source).snippet(b.expr.span).to_string();
        let contracts = |o: oxy_ecore::OperationId, kind: OclConstraintKind| -> Vec<NamedExpression> {
            env.contracts_of(o)
                .iter()
                .map(|i| &env.operation_constraints()[*i])
                .filter(|oc| oc.kind == kind)
                .map(|oc| NamedExpression { name: oc.name.clone().unwrap_or_default(), expression: text(&oc.body) })
                .collect()
        };
        let classifiers = pkg
            .classifiers
            .iter()
            .map(|&c| {
                let cl = mm.classifier(c);
                let (kind, is_abstract, is_interface) = match &cl.kind {
                    EKind::Class(d) => (ClassifierKind::Class, d.is_abstract, d.is_interface),
                    EKind::DataType(_) => (ClassifierKind::DataType, false, false),
                    EKind::Enum(_) => (ClassifierKind::Enum, false, false),
                };
                let data = mm.class_data(c);
                ClassifierInfo {
                    name: cl.name.clone(),
                    qualified_name: mm.qualified_name(c),
                    kind,
                    is_abstract,
                    is_interface,
                    super_types: mm.super_types(c).iter().map(|s| mm.qualified_name(*s)).collect(),
                    features: data
                        .map(|d| {
                            d.features
                                .iter()
                                .map(|&f| {
                                    let feat = mm.feature(f);
                                    FeatureInfo {
                                        name: feat.name.clone(),
                                        ty: type_name(feat.ty, &feat.multiplicity),
                                        is_reference: feat.is_reference(),
                                        containment: feat.is_containment(),
                                        derived: feat.derived,
                                        lower: feat.multiplicity.lower,
                                        upper: feat.multiplicity.upper,
                                        opposite: feat.opposite().map(|o| mm.feature(o).name.clone()),
                                        derivation: env.derivation(f).map(text),
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    operations: data
                        .map(|d| {
                            d.operations
                                .iter()
                                .map(|&o| {
                                    let op = mm.operation(o);
                                    OperationInfo {
                                        name: op.name.clone(),
                                        parameters: op
                                            .params
                                            .iter()
                                            .map(|p| ParameterInfo {
                                                name: p.name.clone(),
                                                ty: type_name(p.ty, &p.multiplicity),
                                            })
                                            .collect(),
                                        ty: if op.ty.is_some() {
                                            type_name(op.ty, &op.multiplicity)
                                        } else {
                                            "OclVoid".into()
                                        },
                                        body: env.operation_body(o).map(text),
                                        preconditions: contracts(o, OclConstraintKind::Pre),
                                        postconditions: contracts(o, OclConstraintKind::Post),
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    literals: mm.enum_literals(c).iter().map(|l| l.name.clone()).collect(),
                    invariants: env
                        .invariants()
                        .iter()
                        .filter(|i| i.context == c)
                        .map(|i| NamedExpression {
                            name: i.name.clone().unwrap_or_default(),
                            expression: text(&i.body),
                        })
                        .collect(),
                }
            })
            .collect();
        PackageInfo {
            name: pkg.name.clone(),
            ns_uri: pkg.ns_uri.clone(),
            ns_prefix: pkg.ns_prefix.clone(),
            resource: mm.resources()[pkg.resource].uri.clone(),
            classifiers,
            subpackages: pkg.subpackages.iter().map(|s| self.package_info(env, *s)).collect(),
        }
    }

    /// The loaded models (XMI files and HUTN documents) as object trees.
    pub fn models(&self) -> Vec<ModelResourceInfo> {
        self.model
            .resources()
            .iter()
            .map(|r| ModelResourceInfo {
                uri: r.uri.clone(),
                roots: r.roots.iter().map(|o| self.object_info(*o)).collect(),
            })
            .collect()
    }

    fn object_info(&self, o: ObjectId) -> ObjectInfo {
        let mm = &self.mm;
        let model = &self.model;
        let mut attributes = Vec::new();
        let mut references = Vec::new();
        let mut children = Vec::new();
        for &f in mm.all_features(model.class_of(o)) {
            let feature = mm.feature(f);
            let values = model.get(o, f);
            if values.is_empty() {
                continue;
            }
            if feature.is_containment() {
                children.extend(values.iter().filter_map(|v| match v {
                    oxy_ecore::ModelValue::Object(c) => Some(self.object_info(*c)),
                    _ => None,
                }));
                continue;
            }
            let rendered: Vec<String> = values
                .iter()
                .map(|v| match v {
                    oxy_ecore::ModelValue::Object(t) => model.label(mm, *t),
                    other => oxy_ocl::eval::convert_model_value(other).render(mm, model),
                })
                .collect();
            let slot = SlotInfo { name: feature.name.clone(), values: rendered };
            if feature.is_reference() {
                // Skip container references: they are implied by the tree.
                if feature.opposite().is_some_and(|op| mm.feature(op).is_containment()) {
                    continue;
                }
                references.push(slot);
            } else {
                attributes.push(slot);
            }
        }
        ObjectInfo {
            fragment: model.uri_fragment(mm, o),
            class: mm.qualified_name(model.class_of(o)),
            label: model.label(mm, o),
            range: self.object_location(o).map(|(_, r)| r),
            attributes,
            references,
            children,
        }
    }
}

/// A compiled set of OCL sources.
pub struct Compiled {
    pub env: Environment,
    document_sources: Vec<(String, SourceId)>,
}

impl Compiled {
    pub fn source_of(&self, document: &str) -> Option<SourceId> {
        self.document_sources.iter().find(|(n, _)| n == document).map(|(_, s)| *s)
    }
}

// ---------------------------------------------------------------------- XMI documents

/// Loads an XMI document into `model` as the resource `name`, with problems located like those
/// of a HUTN document. A document that cannot be loaded at all (malformed XML, an unknown root
/// class) still gets a resource, an empty one.
fn load_xmi_document(text: &str, name: &str, mm: &Metamodel, model: &mut Model) -> HutnLoad {
    // Loading may fail halfway; only keep complete loads.
    let mut loaded = model.clone();
    match XmiDocument::parse(text, name).and_then(|d| d.load(mm, &mut loaded)) {
        Ok(load) => {
            *model = loaded;
            let objects: Vec<hutn::HutnObject> = load
                .objects
                .iter()
                .map(|&(object, offset)| hutn::HutnObject {
                    object,
                    span: start_tag_span(text, offset),
                    name: tag_name_span(text, offset),
                })
                .collect();
            // What EMF would refuse to load is an error here (the loader only warns).
            let mut problems: Vec<Problem> = load
                .warnings
                .iter()
                .filter(|w| w.path == name)
                .map(|w| Problem::error(error_span(text, w), w.message.clone()))
                .collect();
            let headers: Vec<(ObjectId, ByteRange<usize>)> =
                objects.iter().map(|o| (o.object, o.name.clone())).collect();
            problems.extend(hutn::check_multiplicities(mm, model, &headers));
            HutnLoad { resource: load.resource, problems, objects }
        }
        Err(e) => {
            let resource = model.add_resource(name);
            HutnLoad {
                resource,
                problems: vec![Problem::error(error_span(text, &e), e.message.clone())],
                objects: Vec::new(),
            }
        }
    }
}

/// The span of a load error: the tag name at its location, or the character there.
fn error_span(text: &str, e: &LoadError) -> ByteRange<usize> {
    let Some((line, column)) = e.location else { return 0..0 };
    let line_start = text.split_inclusive('\n').take(line.saturating_sub(1)).map(str::len).sum::<usize>();
    let mut offset = (line_start + column.saturating_sub(1)).min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    if text[offset..].starts_with('<') {
        tag_name_span(text, offset)
    } else {
        offset..text[offset..].chars().next().map_or(offset, |c| offset + c.len_utf8())
    }
}

/// The element name of the start tag at `offset` (the `<`).
fn tag_name_span(text: &str, offset: usize) -> ByteRange<usize> {
    let start = (offset + 1).min(text.len());
    let len = text[start..].find(|c: char| c.is_whitespace() || c == '>' || c == '/').unwrap_or(text.len() - start);
    start..start + len
}

/// The start tag at `offset` (the `<`), up to its `>`.
fn start_tag_span(text: &str, offset: usize) -> ByteRange<usize> {
    let mut quote = None;
    for (i, c) in text[offset..].char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return offset..offset + i + 1,
            _ => {}
        }
    }
    offset..text.len()
}

// ---------------------------------------------------------------------- conversions

pub(crate) fn severity(s: oxy_ocl::Severity) -> Severity {
    match s {
        oxy_ocl::Severity::Error => Severity::Error,
        oxy_ocl::Severity::Warning => Severity::Warning,
        oxy_ocl::Severity::Info => Severity::Info,
    }
}

pub(crate) fn origin(kind: &SourceKind, name: &str) -> Origin {
    match kind {
        SourceKind::Document => Origin::Document { name: name.to_string() },
        SourceKind::Annotation { resource, element, key } => {
            Origin::Ecore { resource: resource.clone(), element: element.clone(), key: key.clone() }
        }
        SourceKind::Expression => Origin::Expression,
    }
}

pub(crate) fn range(source: &oxy_ocl::Source, span: oxy_ocl::Span) -> Range {
    let p = |o: u32| {
        let p = source.position(o);
        Position { line: p.line, column: p.column }
    };
    Range { start: p(span.start), end: p(span.end) }
}

fn convert_diagnostic(source: &oxy_ocl::Source, d: &oxy_ocl::Diagnostic) -> Diagnostic {
    Diagnostic {
        severity: severity(d.severity),
        message: d.message.clone(),
        source: source.name.clone(),
        origin: origin(&source.kind, &source.name),
        range: range(source, d.span),
    }
}

pub(crate) fn load_diagnostic(e: &LoadError) -> Diagnostic {
    let pos = e.location.map(|(l, c)| Position { line: l as u32 - 1, column: c as u32 - 1 }).unwrap_or_default();
    Diagnostic {
        severity: Severity::Warning,
        message: e.message.clone(),
        source: e.path.clone(),
        origin: Origin::File { path: e.path.clone() },
        range: Range { start: pos, end: pos },
    }
}

fn error_diagnostic(e: &Error) -> Diagnostic {
    match e {
        Error::Load(l) => Diagnostic { severity: Severity::Error, ..load_diagnostic(l) },
        other => Diagnostic {
            severity: Severity::Error,
            message: other.to_string(),
            source: String::new(),
            origin: Origin::Expression,
            range: Range::default(),
        },
    }
}
