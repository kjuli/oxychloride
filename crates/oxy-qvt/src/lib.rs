//! QVT model transformations over Ecore models, as in Eclipse QVTo and Eclipse QVTd: the
//! two languages of the OMG's *MOF Query/View/Transformation* standard that people write.
//!
//! * **Operational Mappings** (`.qvto`, [`operational`]): imperative transformations —
//!   mappings from objects of input models to objects of output models, with guards, an
//!   `init`, a population and an `end` section, helpers and queries, `resolve` over the trace
//!   of the mappings applied, and Imperative OCL (variables, assignments, loops, `object`).
//! * **Relations** (`.qvtr`, [`relational`]): declarative transformations — relations between
//!   object patterns of the models (domains), with `when` and `where` clauses, run in the
//!   direction of one of the models: what the relations demand of it is created (enforced),
//!   or only checked.
//!
//! Both are compiled against a [`Metamodel`] (their model types and imports name its packages,
//! by nsURI or by the file they were loaded from) — QVTo documents together with the units
//! they import ([`compile_with_imports`]: libraries and other transformations, found through a
//! [`UnitLookup`]) — into a [`Compiled`] transformation with
//! diagnostics, the types of its expressions (for hovers and completion) and its parameters,
//! and [`run`] on a [`Model`]: input parameters are bound to resources of the model, output
//! parameters to (empty) resources the objects are created in. The OCL inside — and the
//! Imperative OCL of QVTo — is checked and evaluated by `oxy-ocl`, through its
//! [`oxy_ocl::CheckHost`] and [`oxy_ocl::Host`] hooks.

use oxy_ecore::{Metamodel, Model, PackageId};
use oxy_ocl::{Diagnostic, Environment, Span, Type};

pub mod library;
pub mod operational;
pub mod relational;

pub use operational::syntax::Direction;

/// The two QVT languages.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    /// QVT Operational Mappings (`.qvto`).
    Operational,
    /// QVT Relations (`.qvtr`).
    Relational,
}

impl Language {
    /// The language of a file, by its extension.
    pub fn from_path(path: &str) -> Option<Language> {
        let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
        match ext.as_str() {
            "qvto" => Some(Language::Operational),
            "qvtr" => Some(Language::Relational),
            _ => None,
        }
    }
}

/// A model parameter of a transformation.
#[derive(Clone, Debug, PartialEq)]
pub struct Parameter {
    pub name: String,
    /// `in`, `out` or `inout` (QVTo); `None` for QVTr, whose parameters are inputs or the
    /// target depending on the direction of a run.
    pub direction: Option<Direction>,
    /// The name of its model type (QVTo) or metamodel import (QVTr).
    pub model_type: String,
    /// The packages of the model type.
    pub packages: Vec<PackageId>,
    pub span: Span,
}

impl Parameter {
    /// Whether objects are created in models bound to it (in QVTr: when it is the target).
    pub fn is_output(&self) -> bool {
        matches!(self.direction, Some(Direction::Out | Direction::InOut))
    }

    /// Whether a model must be bound to it before a run.
    pub fn is_input(&self) -> bool {
        !matches!(self.direction, Some(Direction::Out))
    }
}

/// What can be used without a source in an expression of a transformation: for completion.
#[derive(Clone, Debug)]
pub struct Scope {
    /// The span of the body.
    pub span: Span,
    /// The variables (parameters, `result`, relation variables) and their types.
    pub variables: Vec<(String, Type)>,
    /// The type of `self` (contextual mappings and helpers).
    pub self_type: Option<Type>,
}

/// A declaration of a transformation, for hovers and outlines. Declarations of imported units
/// (QVTo libraries) are included for completion; their spans lie after the document's end.
#[derive(Clone, Debug)]
pub struct Symbol {
    pub name: String,
    /// What it is: `mapping`, `query`, `relation`, `parameter`, ...
    pub kind: &'static str,
    /// Its signature, e.g. `mapping UML::Class::toTable() : RDB::Table`.
    pub detail: String,
    /// The span of its name.
    pub span: Span,
    /// The span of the whole declaration.
    pub range: Span,
}

/// A compiled transformation document of either language.
pub struct Compiled {
    pub language: Language,
    /// The transformation's name (`None` if the document declares none).
    pub name: Option<String>,
    pub parameters: Vec<Parameter>,
    pub diagnostics: Vec<Diagnostic>,
    /// The types of the checked expressions, with their spans.
    pub types: Vec<(Span, Type)>,
    pub scopes: Vec<Scope>,
    pub symbols: Vec<Symbol>,
    /// The URIs the model types (or imports) name.
    pub uris: Vec<(String, Span)>,
    /// Whether the document is a QVTo library (which is not run itself).
    pub library: bool,
    /// The files of the units the document imports, directly or not.
    pub units: Vec<String>,
    inner: Inner,
}

enum Inner {
    Operational(Box<operational::Transformation>),
    Relational(Box<relational::Transformation>),
}

impl Compiled {
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }

    pub fn parameter(&self, name: &str) -> Option<&Parameter> {
        self.parameters.iter().find(|p| p.name == name)
    }

    /// The parameters a QVTr transformation can be run in the direction of (the targets of
    /// enforce domains); for QVTo, its output parameters.
    pub fn targets(&self) -> Vec<String> {
        match &self.inner {
            Inner::Operational(_) => self.parameters.iter().filter(|p| p.is_output()).map(|p| p.name.clone()).collect(),
            Inner::Relational(t) => t.targets(),
        }
    }

    /// The type of the innermost checked expression whose span contains `offset`.
    pub fn type_at(&self, offset: u32) -> Option<(Span, &Type)> {
        self.types
            .iter()
            .filter(|(span, _)| span.start <= offset && offset <= span.end)
            .min_by_key(|(span, _)| span.end - span.start)
            .map(|(span, t)| (*span, t))
    }

    /// The type of the largest checked expression ending exactly at `offset`.
    pub fn type_ending_at(&self, offset: u32) -> Option<&Type> {
        self.types
            .iter()
            .filter(|(span, _)| span.end == offset)
            .max_by_key(|(span, _)| span.end - span.start)
            .map(|(_, t)| t)
    }

    /// The innermost scope containing `offset`.
    pub fn scope_at(&self, offset: u32) -> Option<&Scope> {
        self.scopes
            .iter()
            .filter(|s| s.span.start <= offset && offset <= s.span.end)
            .min_by_key(|s| s.span.end - s.span.start)
    }

    /// The model types (QVTo) or imports (QVTr) and their packages: `UML::Class` names a
    /// class of `UML`'s packages.
    pub fn aliases(&self) -> &[oxy_ocl::PackageAlias] {
        match &self.inner {
            Inner::Operational(t) => &t.aliases,
            Inner::Relational(t) => &t.aliases,
        }
    }

    /// The names of mappings (QVTo) or relations (QVTr), and of helpers and queries.
    pub fn operations(&self) -> Vec<&Symbol> {
        self.symbols.iter().filter(|s| matches!(s.kind, "mapping" | "helper" | "query" | "relation")).collect()
    }
}

/// Finds the packages a URI stands for (a model type's or import's nsURI, or the location of
/// a metamodel file); the caller knows the package registry and where documents are.
pub type PackageLookup<'a> = &'a dyn Fn(&str) -> Vec<PackageId>;

/// The packages of a URI in a metamodel: those with that nsURI, else the roots of the
/// resource loaded from (or registered under) it. A fragment (`file.ecore#/`) is ignored.
pub fn packages_of_uri(mm: &Metamodel, uri: &str) -> Vec<PackageId> {
    let base = uri.split('#').next().unwrap_or(uri);
    if let Some(p) = mm.package_by_ns_uri(base) {
        return vec![p];
    }
    match mm.resource_of_uri(base) {
        Some(r) => mm.resources()[r].roots.clone(),
        None => Vec::new(),
    }
}

/// A unit a QVTo document imports: its file and text.
#[derive(Clone, Debug)]
pub struct Unit {
    pub path: String,
    pub text: String,
}

/// Finds the unit an `import a.b.c` names: called with the file of the importing unit (`None`:
/// the document being compiled) and the parts of the name (`["a", "b", "c"]`). As in Eclipse
/// QVTo, the unit is `a/b/c.qvto` under a source folder; [`find_unit`] looks next to the
/// importing file and in the folders above it.
pub type UnitLookup<'a> = &'a dyn Fn(Option<&str>, &[String]) -> Option<Unit>;

/// Where the unit `name` (`["a", "b", "c"]`) imported by `from` can be: `a/b/c.qvto` in the
/// folder of `from` and in each folder above it.
pub fn unit_candidates(from: &std::path::Path, name: &[String]) -> Vec<std::path::PathBuf> {
    let mut relative = std::path::PathBuf::new();
    for part in name {
        relative.push(part);
    }
    relative.set_extension("qvto");
    from.parent().into_iter().flat_map(|d| d.ancestors()).map(|d| d.join(&relative)).collect()
}

/// Finds an imported unit on disk (see [`unit_candidates`]); `root` is the file of the
/// document being compiled.
pub fn find_unit(root: &std::path::Path, from: Option<&str>, name: &[String]) -> Option<Unit> {
    let from = from.map(std::path::Path::new).unwrap_or(root);
    unit_candidates(from, name).into_iter().find_map(|p| {
        let text = std::fs::read_to_string(&p).ok()?;
        Some(Unit { path: p.to_string_lossy().into_owned(), text })
    })
}

/// The units a QVTo document imports, directly or not (those `units` finds), in the order they
/// are found.
pub fn imported_units(text: &str, units: UnitLookup) -> Vec<Unit> {
    let mut found: Vec<Unit> = Vec::new();
    let mut pending: Vec<(Option<String>, String)> = vec![(None, text.to_string())];
    while let Some((path, text)) = pending.pop() {
        for import in operational::syntax::parse(&text).0.imports {
            let names: Vec<String> = import.path.iter().map(|i| i.name.clone()).collect();
            let Some(unit) = units(path.as_deref(), &names) else { continue };
            if found.len() < 256 && !found.iter().any(|u| u.path == unit.path) {
                pending.push((Some(unit.path.clone()), unit.text.clone()));
                found.push(unit);
            }
        }
    }
    found
}

/// Compiles a transformation document (QVTo: without imports, see [`compile_with_imports`]).
pub fn compile(language: Language, mm: &Metamodel, env: &Environment, text: &str, lookup: PackageLookup) -> Compiled {
    compile_with_imports(language, mm, env, text, lookup, &|_, _| None)
}

/// Compiles a transformation document; the units a QVTo document imports are found with
/// `units`. The URIs of imported units' model types are looked up relative to their files.
pub fn compile_with_imports(
    language: Language,
    mm: &Metamodel,
    env: &Environment,
    text: &str,
    lookup: PackageLookup,
    units: UnitLookup,
) -> Compiled {
    match language {
        Language::Operational => operational::compile(mm, env, text, lookup, units),
        Language::Relational => relational::compile(mm, env, text, lookup),
    }
}

/// A model parameter bound to a resource of the model a transformation runs on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub parameter: String,
    pub resource: usize,
}

/// How to run a transformation.
#[derive(Clone, Debug, Default)]
pub struct RunOptions {
    /// QVTr: the parameter to run the transformation in the direction of (default: the only
    /// one with enforce domains).
    pub target: Option<String>,
    /// QVTr: only check that the target model is related as the relations demand, without
    /// changing it.
    pub check_only: bool,
    /// QVTo: values of configuration properties (in OCL literal syntax: `'text'`, `3`,
    /// `true`).
    pub configuration: Vec<(String, String)>,
}

/// A problem met while running a transformation.
#[derive(Clone, Debug, PartialEq)]
pub struct RunProblem {
    /// Where in the transformation (if known); with `unit`, in that imported unit.
    pub span: Option<Span>,
    pub message: String,
    /// The file of the imported unit the problem is in (`None`: the document).
    pub unit: Option<String>,
}

/// What a run did.
#[derive(Clone, Debug, Default)]
pub struct RunResult {
    /// The messages of `log` (and of failed assertions).
    pub log: Vec<String>,
    pub problems: Vec<RunProblem>,
    /// QVTr in check mode: the bindings of relations that do not hold.
    pub violations: Vec<String>,
    /// How many mappings (QVTo) or relations (QVTr) were applied.
    pub applications: usize,
    /// The number of objects created.
    pub created: usize,
    /// The message of a fatal assertion that ended the transformation.
    pub aborted: Option<String>,
}

/// Runs a compiled transformation on `model`. Every input parameter must be bound to a
/// resource of the model, every output parameter to a resource its objects are created in
/// (typically a new, empty one). Fails (without running) if the transformation has errors or
/// the bindings do not fit.
pub fn run(
    compiled: &Compiled,
    mm: &Metamodel,
    env: &Environment,
    model: &mut Model,
    bindings: &[Binding],
    options: &RunOptions,
) -> Result<RunResult, String> {
    if let Some(d) = compiled.diagnostics.iter().find(|d| d.is_error()) {
        return Err(format!("the transformation has errors: {}", d.message));
    }
    if compiled.library {
        return Err(format!(
            "'{}' is a library: run a transformation that imports it",
            compiled.name.as_deref().unwrap_or_default()
        ));
    }
    for p in &compiled.parameters {
        if !bindings.iter().any(|b| b.parameter == p.name) {
            return Err(format!("no model is bound to the parameter '{}'", p.name));
        }
    }
    for b in bindings {
        if compiled.parameter(&b.parameter).is_none() {
            return Err(format!("the transformation has no parameter '{}'", b.parameter));
        }
        if b.resource >= model.resources().len() {
            return Err(format!("the model bound to '{}' does not exist", b.parameter));
        }
    }
    match &compiled.inner {
        Inner::Operational(t) => Ok(operational::run(t, compiled, mm, env, model, bindings, options)),
        Inner::Relational(t) => relational::run(t, compiled, mm, env, model, bindings, options),
    }
}
