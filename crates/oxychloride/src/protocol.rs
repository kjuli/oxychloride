//! Serializable data exchanged with clients (JSON-RPC, C ABI, the language server).
//!
//! Field names are camelCase on the wire. Lines and columns are 0-based; columns count
//! Unicode scalar values.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

/// The language of an open document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    /// Complete OCL: constraints for existing metamodels (`.ocl`).
    Ocl,
    /// OCLinEcore: a metamodel with embedded OCL (`.oclinecore`).
    #[serde(rename = "oclinecore")]
    OclInEcore,
    /// HUTN: an instance model in the OMG Human-Usable Textual Notation (`.hutn`).
    Hutn,
    /// XMI: an instance model as EMF writes it (`.xmi`, or an extension a bundle registers for
    /// its models, such as `.repository`), edited as text.
    Xmi,
    /// Ecore: a metamodel as EMF writes it (`.ecore`), edited as XML. The document's text
    /// stands for the file: the workspace loads it instead, and the OCL embedded in its
    /// annotations is checked where it is.
    Ecore,
    /// Henshin: graph transformation rules and units in Henshin's textual syntax
    /// (`.henshin_text`).
    Henshin,
    /// A Henshin transformation module as Henshin's graphical and tree editors write it
    /// (`.henshin`, XMI of Henshin's metamodel), edited as text or as rule diagrams. It means
    /// the same as its textual syntax.
    #[serde(rename = "henshinModel")]
    HenshinModel,
    /// QVT Operational Mappings (`.qvto`): imperative model-to-model transformations.
    Qvto,
    /// QVT Relations (`.qvtr`): declarative model-to-model transformations.
    Qvtr,
}

impl Language {
    /// Whether documents of this language are models (their objects are part of the workspace's
    /// models), rather than metamodels or constraints.
    pub fn is_model(self) -> bool {
        matches!(self, Language::Hutn | Language::Xmi)
    }

    /// Whether documents of this language are Henshin transformations (in either syntax).
    pub fn is_transformation(self) -> bool {
        matches!(self, Language::Henshin | Language::HenshinModel)
    }

    /// Whether documents of this language are QVT transformations (of either language).
    pub fn is_qvt(self) -> bool {
        matches!(self, Language::Qvto | Language::Qvtr)
    }

    /// The language of a document, by its file extension (Complete OCL by default).
    pub fn from_path(name: &str) -> Language {
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        match ext.as_str() {
            "oclinecore" => Language::OclInEcore,
            "hutn" => Language::Hutn,
            "xmi" => Language::Xmi,
            "ecore" => Language::Ecore,
            "henshin_text" => Language::Henshin,
            "henshin" => Language::HenshinModel,
            "qvto" => Language::Qvto,
            "qvtr" => Language::Qvtr,
            _ => Language::Ocl,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Language::Ocl => "ocl",
            Language::OclInEcore => "oclinecore",
            Language::Hutn => "hutn",
            Language::Xmi => "xmi",
            Language::Ecore => "ecore",
            Language::Henshin => "henshin_text",
            Language::HenshinModel => "henshin",
            Language::Qvto => "qvto",
            Language::Qvtr => "qvtr",
        }
    }
}

/// Where a piece of OCL comes from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Origin {
    /// A Complete OCL document, identified by the name it was opened with.
    Document { name: String },
    /// An expression embedded in an `.ecore` file. The diagnostic's range is relative to the
    /// expression; `location` is that range in the file (in the `value` of the annotation's
    /// `<details>`), when known.
    Ecore {
        resource: String,
        element: String,
        key: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        location: Option<Range>,
    },
    /// An expression passed to `evaluate`.
    Expression,
    /// A metamodel or model file that failed to load.
    File { path: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    /// Human readable name of the source (document path or `file.ecore: Class::Constraint`).
    pub source: String,
    pub origin: Origin,
    pub range: Range,
}

/// A reference to a model object: the resource URI and the EMF URI fragment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectRef {
    /// URI (path) of the resource. Optional in requests: defaults to the first loaded model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    /// EMF URI fragment, e.g. `//@books.0` (or an `xmi:id`).
    pub fragment: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectSummary {
    pub resource: String,
    pub fragment: String,
    /// Qualified class name.
    pub class: String,
    pub label: String,
    /// For objects defined in a HUTN document (named by `resource`): the range of the
    /// object's definition.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub range: Option<Range>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResultStatus {
    Satisfied,
    Violated,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConstraintKind {
    Invariant,
    Precondition,
    Postcondition,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstraintInfo {
    /// Constraint name (`<anonymous>` if unnamed).
    pub name: String,
    pub kind: ConstraintKind,
    /// Qualified name of the context class.
    pub context: String,
    /// For pre- and postconditions: the operation's signature, e.g. `hasSeat() : Boolean`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub operation: Option<String>,
    pub source: String,
    pub origin: Origin,
    pub range: Range,
    /// The constraint's OCL text.
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationResult {
    /// Index into [`ValidationReport::constraints`].
    pub constraint: usize,
    pub object: ObjectSummary,
    /// For pre- and postconditions: the checked call, e.g. `mayEnrol(Course 'Algorithms')`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub call: Option<String>,
    pub status: ResultStatus,
    /// The constraint's message (for violations) or the error explanation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationSummary {
    pub constraints: usize,
    pub objects: usize,
    pub evaluations: usize,
    pub satisfied: usize,
    pub violated: usize,
    pub errors: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationReport {
    pub constraints: Vec<ConstraintInfo>,
    /// Results; satisfied ones are omitted unless requested.
    pub results: Vec<ValidationResult>,
    pub summary: ValidationSummary,
    /// Problems found while compiling the constraints.
    pub diagnostics: Vec<Diagnostic>,
    /// Remarks about what could not be checked, e.g. operations whose postconditions are
    /// only checked where they are called.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub notes: Vec<String>,
}

impl ValidationReport {
    /// Whether all constraints are satisfied and compiled without errors.
    pub fn is_ok(&self) -> bool {
        self.summary.violated == 0
            && self.summary.errors == 0
            && !self.diagnostics.iter().any(|d| d.severity == Severity::Error)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationResult {
    /// The value in OCL literal syntax, e.g. `Set{1, 2}` or `Book 'Dune'`.
    pub display: String,
    /// A structured JSON rendering of the value (see the protocol documentation).
    pub value: serde_json::Value,
    /// Statically inferred type.
    #[serde(rename = "type")]
    pub ty: String,
    pub invalid: bool,
    pub diagnostics: Vec<Diagnostic>,
    /// Pre- and postconditions that failed in operation calls made by the expression.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub contracts: Vec<ContractResult>,
}

/// The outcome of a pre- or postcondition at one operation call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractResult {
    pub constraint: ConstraintInfo,
    /// The object the operation was called on.
    pub object: ObjectSummary,
    pub call: String,
    pub status: ResultStatus,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub message: Option<String>,
}

// ------------------------------------------------------------------ metamodel

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageInfo {
    pub name: String,
    pub ns_uri: String,
    pub ns_prefix: String,
    pub resource: String,
    pub classifiers: Vec<ClassifierInfo>,
    pub subpackages: Vec<PackageInfo>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClassifierKind {
    Class,
    DataType,
    Enum,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassifierInfo {
    pub name: String,
    pub qualified_name: String,
    pub kind: ClassifierKind,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub is_abstract: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub is_interface: bool,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub super_types: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub features: Vec<FeatureInfo>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub operations: Vec<OperationInfo>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub literals: Vec<String>,
    /// Invariants embedded in the Ecore model: `(name, OCL text)`.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub invariants: Vec<NamedExpression>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedExpression {
    pub name: String,
    pub expression: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub is_reference: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub containment: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub derived: bool,
    pub lower: i32,
    pub upper: i32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub opposite: Option<String>,
    /// OCL derivation, if any.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub derivation: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationInfo {
    pub name: String,
    pub parameters: Vec<ParameterInfo>,
    #[serde(rename = "type")]
    pub ty: String,
    /// OCL body, if any.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub preconditions: Vec<NamedExpression>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub postconditions: Vec<NamedExpression>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
}

// ------------------------------------------------------------------ models

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelResourceInfo {
    pub uri: String,
    pub roots: Vec<ObjectInfo>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectInfo {
    pub fragment: String,
    pub class: String,
    pub label: String,
    /// For objects defined in a HUTN document: the range of the definition.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub range: Option<Range>,
    /// Attribute values rendered in OCL syntax.
    pub attributes: Vec<SlotInfo>,
    /// Non-containment references, as labels of the referenced objects.
    pub references: Vec<SlotInfo>,
    pub children: Vec<ObjectInfo>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotInfo {
    pub name: String,
    pub values: Vec<String>,
}

// ------------------------------------------------------------------ loading

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadResult {
    /// Resolved URI of what was loaded.
    pub uri: String,
    /// For metamodels: nsURIs of the loaded packages (including dependencies).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub packages: Vec<String>,
    /// For models: number of objects in the resource.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub objects: Option<usize>,
    pub warnings: Vec<Diagnostic>,
}

// ------------------------------------------------------------------ package registry

/// An entry of the package registry, like EMF's package registry and URI map: `uri` — a
/// package's nsURI, or any URI that `.ecore` files, imports and models use for a metamodel,
/// reachable or not (`http://example.org/shop`, `platform:/plugin/x/model/shop.ecore`) — stands
/// for the metamodel in the file `location` (an `.ecore` file, or Ecore XML under another
/// extension). A `uri` ending in `/` maps a prefix to the folder `location` instead
/// (`pathmap://MODELS/` → `/path/to/models`); its files are loaded when something refers to them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryEntry {
    pub uri: String,
    /// A path (absolute, or relative to the working directory).
    pub location: String,
}

impl RegistryEntry {
    /// Whether the entry maps a URI prefix to a folder.
    pub fn is_prefix(&self) -> bool {
        self.uri.ends_with('/')
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RegistryStatus {
    /// The file is loaded (its packages are part of the workspace's metamodels).
    Loaded,
    /// A prefix mapped to a folder; files under it are loaded on demand.
    Folder,
    /// The file or folder does not exist.
    Missing,
    /// The file could not be loaded (see `message`).
    Error,
}

/// A package of a registered metamodel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredPackage {
    pub name: String,
    pub ns_uri: String,
}

/// A metamodel resource (an `.ecore` file) loaded for a registry entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredResource {
    /// Its canonical path (or URI).
    pub uri: String,
    /// The other URIs it is known by (registered URIs, URIs other files refer to it by).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub aliases: Vec<String>,
    pub packages: Vec<RegisteredPackage>,
}

/// The state of a registry entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryInfo {
    pub uri: String,
    pub location: String,
    pub status: RegistryStatus,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub message: Option<String>,
    /// The loaded file (status `loaded`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub resource: Option<RegisteredResource>,
    /// The metamodels the file uses, directly or through others (`.ecore` files it refers
    /// to, by path or by a registered URI), without Ecore itself.
    pub imports: Vec<RegisteredResource>,
    /// Problems loading the file, e.g. references that cannot be resolved.
    pub warnings: Vec<Diagnostic>,
}

// ------------------------------------------------------------------ Eclipse projects

/// The Eclipse projects of the workspace and its target platform (see docs/PROJECTS.md,
/// "Eclipse projects"): `platform:/resource` and `platform:/plugin` URIs, the packages bundles
/// register and their URI mappings resolve as in Eclipse.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlatformSettings {
    /// Folders searched for Eclipse projects (folders with a `.project` or a
    /// `META-INF/MANIFEST.MF`), like the projects of an Eclipse workspace.
    pub workspace: Vec<String>,
    /// The target platform: bundles (jars or folders), folders of bundles (an installation's
    /// `plugins`, a bundle pool), installations, or folders with bundles further down. `~/` is
    /// the home folder.
    pub target: Vec<String>,
}

/// A package a bundle registers (`org.eclipse.emf.ecore.generated_package`, `dynamic_package`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EclipsePackageInfo {
    pub ns_uri: String,
    /// Its `.ecore` file (a path, or an `archive:` URI into a jar).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub location: Option<String>,
    /// For workspace projects: `loaded`, or why not; for the target platform's bundles `folder`
    /// (loaded when something refers to it).
    pub status: RegistryStatus,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub message: Option<String>,
}

/// A bundle a bundle requires (`Require-Bundle`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequiredBundle {
    pub name: String,
    pub optional: bool,
    /// Where it is (a workspace project or the target platform), if anywhere.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub location: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UriMappingInfo {
    pub source: String,
    pub target: String,
}

/// An Eclipse project of the workspace, or a bundle of the target platform.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EclipseBundleInfo {
    /// The symbolic name, else the project's name.
    pub name: String,
    /// The project's name (`.project`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub symbolic_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub version: Option<String>,
    /// Its folder or jar.
    pub location: String,
    pub packages: Vec<EclipsePackageInfo>,
    pub requires: Vec<RequiredBundle>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub uri_mappings: Vec<UriMappingInfo>,
    /// File extensions of its models (`extension_parser`), e.g. `repository`.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub model_extensions: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub problems: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingBundle {
    pub name: String,
    pub required_by: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetLocationInfo {
    pub location: String,
    /// How many bundles it has.
    pub bundles: usize,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub problem: Option<String>,
}

/// The state of the Eclipse projects and the target platform (`platform`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformInfo {
    pub settings: PlatformSettings,
    pub projects: Vec<EclipseBundleInfo>,
    /// The bundles of the target platform the projects require, transitively.
    pub required: Vec<EclipseBundleInfo>,
    /// Required bundles that are neither workspace projects nor in the target platform.
    pub missing: Vec<MissingBundle>,
    pub target: Vec<TargetLocationInfo>,
    /// File extensions of models (opened as XMI), from the projects and their requirements.
    pub model_extensions: Vec<String>,
}

/// A target platform found on this machine (`scanEclipseProjects`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetCandidate {
    pub location: String,
    /// What it is (`bundle pool`, `Eclipse installation`, `target platform of the Eclipse workspace …`).
    pub kind: String,
    pub bundles: usize,
    /// How many of the bundles the projects require (and the folder lacks) it has.
    pub provides: usize,
}

/// The Eclipse projects in a folder, for importing them (`scanEclipseProjects`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EclipseScan {
    pub projects: Vec<EclipseBundleInfo>,
    /// The bundles they require that are not among them.
    pub required: Vec<String>,
    /// Target platforms on this machine, those with most of `required` first.
    pub targets: Vec<TargetCandidate>,
}

/// Makes a folder with Eclipse projects a modeling project (`importEclipseProjects`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImportEclipseRequest {
    /// The folder (it becomes the project's folder).
    pub folder: String,
    /// The project's name (default: the Eclipse project's, if there is one, else the folder's).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The target platform (default: none). Paths are written as given.
    pub target_platform: Vec<String>,
}

/// A modeling project made of Eclipse projects: its manifest (not written), and what was found.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedProject {
    /// The manifest's path and text.
    pub manifest: ProjectFile,
    /// The Eclipse projects, their packages and their requirements, with the target platform.
    pub platform: PlatformInfo,
}

// ------------------------------------------------------------------ editor support

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HoverInfo {
    pub range: Range,
    /// Markdown-free plain text, e.g. `Set(Book)`.
    pub contents: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompletionKind {
    Keyword,
    Type,
    Property,
    Operation,
    Iterator,
    Variable,
    EnumLiteral,
    Snippet,
    /// A model object (a reference target in a model document).
    Object,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionItem {
    pub label: String,
    pub kind: CompletionKind,
    /// Type or signature shown next to the label.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub detail: Option<String>,
    /// Text to insert (defaults to the label).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub insert: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionList {
    /// The range of the partially typed word that the completion replaces.
    pub range: Range,
    pub items: Vec<CompletionItem>,
}

/// A change to a document: `range` is replaced by `text` (an insertion if `range` is empty).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextEdit {
    pub range: Range,
    pub text: String,
}

/// Edits that add an object to a HUTN document (see `createObject`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentEdit {
    /// Non-overlapping edits in document order; ranges refer to the text before the edit.
    pub edits: Vec<TextEdit>,
    /// Where the cursor goes in the edited text: inside the new object's braces.
    pub cursor: Position,
}

/// A containment feature of an object, and the classes of the objects it can contain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainmentOption {
    pub feature: String,
    /// Qualified name of the feature's type.
    #[serde(rename = "type")]
    pub ty: String,
    pub many: bool,
    /// Single-valued and already set: nothing can be added.
    pub full: bool,
    /// Qualified names of the concrete classes conforming to the type.
    pub classes: Vec<String>,
}

/// A generated XML Schema (see `schemas`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaFile {
    /// The target namespace: a package's nsURI, or the XMI namespace.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ns_uri: Option<String>,
    /// The file name the other schemas import it by.
    pub file_name: String,
    pub text: String,
}

// ------------------------------------------------------------------ conversion

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub text: String,
    /// `ecore` (for OCLinEcore documents) or `xmi` (for HUTN documents).
    pub format: String,
}

// ------------------------------------------------------------------ transformations

/// A rule or unit of a Henshin document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransformationUnitInfo {
    pub name: String,
    /// `rule` or `unit`.
    pub kind: String,
    pub parameters: Vec<TransformationParameterInfo>,
    /// The range of the rule's or unit's name.
    pub range: Range,
    /// Multi-rules of a rule.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub multi_rules: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransformationParameterInfo {
    pub name: String,
    /// `in`, `out`, `inout`, `var`, or `unknown` (no kind given).
    pub kind: String,
    #[serde(rename = "type")]
    pub ty: String,
    /// Whether the parameter is a node (its type is a class).
    pub node: bool,
    /// For enumeration-typed parameters: the literals.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub literals: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransformationUnits {
    pub units: Vec<TransformationUnitInfo>,
    pub diagnostics: Vec<Diagnostic>,
}

/// An object of a transformed model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNodeInfo {
    /// Stable within a transformation: created objects get new ids, ids are never reused.
    pub id: u32,
    /// Qualified class name.
    pub class: String,
    pub label: String,
    /// Attribute values rendered in OCL syntax.
    pub attributes: Vec<SlotInfo>,
    /// The containing object.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub container: Option<u32>,
}

/// A link between objects. Of a bidirectional pair of references, only one side is listed
/// (the containment, else the reference declared first); containers are implied.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdgeInfo {
    pub source: u32,
    pub target: u32,
    pub feature: String,
    pub containment: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub opposite: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphInfo {
    pub nodes: Vec<GraphNodeInfo>,
    pub edges: Vec<GraphEdgeInfo>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttributeChange {
    pub node: u32,
    pub feature: String,
    pub old: Vec<String>,
    pub new: Vec<String>,
}

/// What a step changed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphChanges {
    pub created_nodes: Vec<u32>,
    /// The deleted objects, as they were.
    pub deleted_nodes: Vec<GraphNodeInfo>,
    pub created_edges: Vec<GraphEdgeInfo>,
    pub deleted_edges: Vec<GraphEdgeInfo>,
    pub changed_attributes: Vec<AttributeChange>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedValue {
    pub name: String,
    pub value: String,
}

/// A rule node and the object it matched.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchedNode {
    pub name: String,
    pub node: u32,
}

/// An application of a rule.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleApplicationInfo {
    pub rule: String,
    /// `rule(param=value, ...)`, or for rules without parameters `rule(node: object, ...)`.
    pub label: String,
    pub parameters: Vec<NamedValue>,
    /// Rule nodes and the objects they matched (nodes of multi-rules as `multiRule.node`).
    pub nodes: Vec<MatchedNode>,
    /// The number of multi-rule matches applied with it.
    #[serde(skip_serializing_if = "is_zero", default)]
    pub instances: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// A match of a rule (see `findMatches`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchInfo {
    /// Pass it as `match` to `applyTransformation`.
    pub index: usize,
    pub label: String,
    pub parameters: Vec<NamedValue>,
    pub nodes: Vec<MatchedNode>,
    /// All objects matched, including those of multi-rule matches.
    pub matched: Vec<u32>,
    #[serde(skip_serializing_if = "is_zero", default)]
    pub instances: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchList {
    pub matches: Vec<MatchInfo>,
    /// Whether there are more matches than were listed.
    pub more: bool,
    /// Problems met while matching (e.g. a parameter without a value).
    pub messages: Vec<String>,
}

/// An invariant that a state of a transformation violates (or cannot evaluate).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateViolation {
    pub constraint: String,
    /// Qualified name of the context class.
    pub context: String,
    pub node: u32,
    pub label: String,
    pub status: ResultStatus,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub message: Option<String>,
}

/// A step of a transformation: the initial model, or the application of a rule or unit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransformationStep {
    pub index: usize,
    /// `initial model`, or what was applied.
    pub label: String,
    /// The rule applications of the step (several for a unit).
    pub applications: Vec<RuleApplicationInfo>,
}

/// A transformation session and one of its states.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransformationState {
    pub session: u64,
    /// The model resource the transformation started from.
    pub model: String,
    pub steps: Vec<TransformationStep>,
    /// The step whose state `graph` is.
    pub step: usize,
    pub graph: GraphInfo,
    /// What the step changed (relative to the step before).
    pub changes: GraphChanges,
    /// The invariants the state violates.
    pub violations: Vec<StateViolation>,
}

/// The outcome of `applyTransformation`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransformationResult {
    /// Whether the rule or unit could be applied (if not, no step was added).
    pub applied: bool,
    pub state: TransformationState,
    pub messages: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateSpaceState {
    pub id: usize,
    /// The length of the shortest path from the initial state.
    pub depth: usize,
    pub nodes: usize,
    /// Whether all transitions leaving the state are known (a limit may have stopped the
    /// exploration).
    pub explored: bool,
    /// No rule is applicable (and the state was explored).
    pub terminal: bool,
    /// The number of invariant violations (when validating).
    pub violations: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateSpaceTransition {
    pub source: usize,
    pub target: usize,
    pub rule: String,
    pub label: String,
    /// The number of matches with this label between the two states.
    pub count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateSpaceInfo {
    pub session: u64,
    /// The step the exploration started from.
    pub step: usize,
    pub rules: Vec<String>,
    /// `states[0]` is the initial state.
    pub states: Vec<StateSpaceState>,
    pub transitions: Vec<StateSpaceTransition>,
    /// Whether every reachable state was explored.
    pub complete: bool,
    pub messages: Vec<String>,
}

/// A state of an explored state space.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateSpaceStateDetails {
    pub id: usize,
    pub graph: GraphInfo,
    pub violations: Vec<StateViolation>,
    /// The transitions (indices) of a shortest path from the initial state.
    pub path: Vec<usize>,
    /// What the last transition of the path changed.
    pub changes: GraphChanges,
}

// ------------------------------------------------------------------ code generation

/// A target language of `generateCode` (see `codeTargets`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeTarget {
    /// The identifier to pass as `target` (`"python"`, `"rust"`).
    pub id: String,
    pub name: String,
    pub description: String,
}

/// What to generate code for (see `generateCode`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CodeRequest {
    /// The target's identifier.
    pub target: String,
    /// The metamodel resource whose packages to generate: an `.ecore` file's URI or an
    /// OCLinEcore document's name, as `PackageInfo::resource` reports it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    /// Qualified names of the packages to generate (instead of `resource`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packages: Option<Vec<String>>,
    /// The name of the generated module, package or crate (default: the first package's).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The Complete OCL documents whose constraints and definitions to include (default:
    /// all open ones). The OCL embedded in the metamodels is always included.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documents: Option<Vec<String>>,
}

/// A file of generated code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeFile {
    /// Relative to the output directory, with `/` separators.
    pub path: String,
    pub text: String,
}

/// Something the generated code does not support (the affected OCL fails at run time).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeProblem {
    pub severity: Severity,
    pub message: String,
    /// Where the OCL comes from, e.g. `library.ocl:16`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub origin: Option<String>,
}

/// The result of `generateCode`. The files are returned, not written.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedCode {
    pub target: String,
    /// Qualified names of the generated packages (the selected ones and those they depend
    /// on).
    pub packages: Vec<String>,
    pub files: Vec<CodeFile>,
    pub problems: Vec<CodeProblem>,
}

// ------------------------------------------------------------------ metamodel diagrams

/// The syntax of a metamodel document shown as a diagram.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MetamodelSyntax {
    /// An `.ecore` file (EMF's XMI).
    Ecore,
    /// An OCLinEcore document.
    #[serde(rename = "oclinecore")]
    OclInEcore,
}

/// The elements of a metamodel document, for drawing it as a class diagram (see
/// `metamodelDiagram`). Elements are identified by ids derived from their names:
/// `pkg::sub` (packages), `pkg::Class` (classifiers), `pkg::Class.feature`,
/// `pkg::Class.operation()` and `pkg::Enum.LITERAL`; a repeated name gets `#2`, `#3`, ….
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetamodelDiagram {
    pub syntax: MetamodelSyntax,
    /// All packages of the document, subpackages included (in document order).
    pub packages: Vec<DiagramPackage>,
    pub classifiers: Vec<DiagramClassifier>,
    /// Classifiers of other metamodels that classifiers of the document extend or refer to.
    pub external: Vec<DiagramExternal>,
    /// The types features, operations and parameters can have, as they are written.
    pub types: Vec<DiagramType>,
    pub problems: Vec<DiagramProblem>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagramPackage {
    pub id: String,
    pub name: String,
    pub ns_uri: String,
    pub ns_prefix: String,
    pub range: Range,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagramClassifier {
    pub id: String,
    pub name: String,
    /// The id of the package.
    pub package: String,
    pub kind: ClassifierKind,
    pub is_abstract: bool,
    pub is_interface: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub instance_class: Option<String>,
    /// Ids of the direct supertypes (classifiers of the document or `external` ones).
    pub super_types: Vec<String>,
    pub attributes: Vec<DiagramFeature>,
    pub references: Vec<DiagramFeature>,
    pub operations: Vec<DiagramOperation>,
    pub literals: Vec<DiagramLiteral>,
    pub invariants: Vec<DiagramInvariant>,
    /// The definition in the document.
    pub range: Range,
}

/// An attribute or reference.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagramFeature {
    pub id: String,
    pub name: String,
    /// The type as written in the document's syntax (`String`, `ecore::EInt`, `EString`).
    pub type_name: String,
    /// The id of the type, if it is resolved.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none", default)]
    pub ty: Option<String>,
    pub lower: i32,
    /// `-1` for unbounded.
    pub upper: i32,
    pub containment: bool,
    /// The id of the opposite reference.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub opposite: Option<String>,
    pub ordered: bool,
    pub unique: bool,
    pub derived: bool,
    pub changeable: bool,
    pub transient: bool,
    pub volatile: bool,
    /// An `iD` attribute.
    pub is_id: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub default_value: Option<String>,
    pub range: Range,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagramOperation {
    pub id: String,
    pub name: String,
    pub parameters: Vec<DiagramParameter>,
    /// The result type as written (`None` for operations without a result).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub type_name: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none", default)]
    pub ty: Option<String>,
    pub lower: i32,
    pub upper: i32,
    /// The operation has an OCL body.
    pub has_body: bool,
    pub range: Range,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagramParameter {
    pub name: String,
    pub type_name: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none", default)]
    pub ty: Option<String>,
    pub lower: i32,
    pub upper: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagramLiteral {
    pub id: String,
    pub name: String,
    pub value: i64,
    pub literal: String,
    pub range: Range,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagramInvariant {
    pub name: String,
    pub range: Range,
}

/// A classifier of another metamodel (e.g. `ecore::EObject` as a supertype).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagramExternal {
    pub id: String,
    pub name: String,
    pub kind: ClassifierKind,
    pub is_abstract: bool,
    pub is_interface: bool,
    /// The URI of the metamodel resource it belongs to.
    pub resource: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagramType {
    pub id: String,
    /// The type as written in the document's syntax.
    pub label: String,
    pub kind: ClassifierKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagramProblem {
    pub severity: Severity,
    pub message: String,
    /// The innermost element whose definition contains the problem.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub element: Option<String>,
    pub range: Range,
}

/// What a new classifier is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NewClassifier {
    Class,
    AbstractClass,
    Interface,
    Enum,
    DataType,
}

/// A parameter of an operation added or changed by an edit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterSpec {
    pub name: String,
    /// The id of the type.
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default = "one")]
    pub lower: i32,
    #[serde(default = "one")]
    pub upper: i32,
}

fn one() -> i32 {
    1
}

/// Properties an `update` edit changes; absent properties stay as they are.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementUpdate {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub is_abstract: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub is_interface: Option<bool>,
    /// The id of the type (of a feature, or the result of an operation).
    #[serde(rename = "type", skip_serializing_if = "Option::is_none", default)]
    pub ty: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub lower: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub upper: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub containment: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ordered: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub unique: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub derived: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub changeable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub transient: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub volatile: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub is_id: Option<bool>,
    /// The parameters of an operation.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub parameters: Option<Vec<ParameterSpec>>,
}

/// A change of a metamodel document, made by editing its text (see `editMetamodel`).
/// Elements and types are named by their diagram ids.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum MetamodelEdit {
    AddClassifier {
        package: String,
        classifier: NewClassifier,
        name: String,
    },
    AddAttribute {
        owner: String,
        name: String,
        #[serde(rename = "type")]
        ty: String,
        #[serde(default)]
        lower: i32,
        #[serde(default = "one")]
        upper: i32,
    },
    /// A reference, and with `opposite` its opposite in the target class.
    AddReference {
        owner: String,
        name: String,
        #[serde(rename = "type")]
        ty: String,
        #[serde(default)]
        lower: i32,
        #[serde(default = "one")]
        upper: i32,
        #[serde(default)]
        containment: bool,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        opposite: Option<String>,
    },
    AddOperation {
        owner: String,
        name: String,
        #[serde(rename = "type", skip_serializing_if = "Option::is_none", default)]
        ty: Option<String>,
        #[serde(default = "one")]
        lower: i32,
        #[serde(default = "one")]
        upper: i32,
        #[serde(default)]
        parameters: Vec<ParameterSpec>,
    },
    AddLiteral {
        owner: String,
        name: String,
    },
    AddSuperType {
        owner: String,
        super_type: String,
    },
    RemoveSuperType {
        owner: String,
        super_type: String,
    },
    /// Changes properties of a package, classifier, feature, operation or literal. Renaming
    /// updates the references to the element in the document (types, supertypes, opposites),
    /// not in its OCL expressions.
    Update {
        element: String,
        properties: ElementUpdate,
    },
    /// Deletes elements, the references typed by deleted classes, and the supertypes and
    /// opposites that refer to deleted elements.
    Delete {
        elements: Vec<String>,
    },
}

/// The text edits that make a [`MetamodelEdit`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetamodelEditResult {
    /// Non-overlapping edits in document order; ranges refer to the text before the edit.
    pub edits: Vec<TextEdit>,
    /// The id of the added or renamed element.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub element: Option<String>,
}

// ------------------------------------------------------------------ transformation diagrams

/// The syntax of a transformation document shown as rule diagrams.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransformationSyntax {
    /// Henshin's textual syntax (`.henshin_text`).
    Text,
    /// Henshin's model (`.henshin`).
    Model,
}

/// What a rule does with a node, edge or attribute: Henshin's actions (drawn as `«create»`
/// and so on, in Henshin's colors).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleAction {
    Preserve,
    Create,
    Delete,
    /// Part of a negative application condition.
    Forbid,
    /// Part of a positive application condition.
    Require,
}

/// The rules and units of a Henshin document (of either syntax), for drawing them in Henshin's
/// notation (see `transformationDiagram`). Elements are identified by ids made of names: a rule
/// `r`, a multi-rule `r/m` (nested: `r/m/n`), a condition graph of a matching formula `r#g`
/// (in a multi-rule `r/m#g`, nested in another `r#g#h`); a node `<scope>::n`, an edge
/// `<scope>::s->t:ref`, an attribute `<scope>::n.a` (the scope it is written in: a multi-rule
/// or a condition graph can add attributes to an enclosing node); a unit `u`, the elements of
/// its body `u@1`, `u@2.1`, …; a repeated id gets `#2`, `#3`, ….
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransformationDiagram {
    pub syntax: TransformationSyntax,
    /// The imported packages, as written.
    pub imports: Vec<String>,
    /// The packages of the workspace that can be imported (top-level packages, as an import
    /// names them).
    pub packages: Vec<String>,
    pub rules: Vec<RuleDiagram>,
    pub units: Vec<UnitDiagram>,
    /// The classes nodes can have: those of the imported packages first.
    pub classes: Vec<NodeClass>,
    /// The types parameters can have, as written.
    pub types: Vec<String>,
    pub problems: Vec<DiagramProblem>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransformationParameterSpec {
    /// `in`, `out`, `inout`, `var`, or `unknown` (no kind given).
    pub kind: String,
    pub name: String,
    /// The type as written (none for parameters without a type).
    #[serde(rename = "type", skip_serializing_if = "Option::is_none", default)]
    pub ty: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleDiagram {
    pub id: String,
    pub name: String,
    pub parameters: Vec<TransformationParameterSpec>,
    pub check_dangling: bool,
    pub injective_matching: bool,
    /// Attribute conditions, as written.
    pub conditions: Vec<String>,
    pub java_imports: Vec<String>,
    /// The multi-rules, nested ones too (their elements are among the rule's nodes and edges).
    pub multi_rules: Vec<MultiRuleDiagram>,
    /// The matching formulas (their condition graphs' elements are among the nodes and edges).
    pub formulas: Vec<FormulaDiagram>,
    pub nodes: Vec<RuleNodeDiagram>,
    pub edges: Vec<RuleEdgeDiagram>,
    pub range: Range,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiRuleDiagram {
    pub id: String,
    pub name: String,
    /// The rule or multi-rule it is a multi-rule of.
    pub kernel: String,
    pub check_dangling: bool,
    pub injective_matching: bool,
    pub conditions: Vec<String>,
    pub range: Range,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormulaDiagram {
    /// The rule, multi-rule or condition graph the formula belongs to.
    pub scope: String,
    /// The formula as written (`!a AND (b OR c)`).
    pub formula: String,
    /// The ids of its condition graphs.
    pub graphs: Vec<String>,
    pub range: Range,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleNodeDiagram {
    pub id: String,
    pub name: String,
    /// The class as written.
    pub class: String,
    /// The qualified name of the class, if it is resolved.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub class_id: Option<String>,
    pub action: RuleAction,
    /// The multi-rule the node belongs to (its id); none for nodes of the rule itself.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub multi: Option<String>,
    /// The condition graph the node belongs to (its id).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub condition: Option<String>,
    pub attributes: Vec<RuleAttributeDiagram>,
    /// The node is a parameter of the rule.
    pub parameter: bool,
    pub range: Range,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleAttributeDiagram {
    pub id: String,
    pub name: String,
    /// The value as written.
    pub value: String,
    pub action: RuleAction,
    /// `set name=value`: the value the attribute gets.
    pub set: bool,
    /// The multi-rule or condition graph the attribute is written in, if not the node's own.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub scope: Option<String>,
    pub range: Range,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleEdgeDiagram {
    pub id: String,
    /// Node ids.
    pub source: String,
    pub target: String,
    pub reference: String,
    pub action: RuleAction,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub multi: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub condition: Option<String>,
    pub range: Range,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitDiagram {
    pub id: String,
    pub name: String,
    pub parameters: Vec<TransformationParameterSpec>,
    pub body: Vec<UnitStepDiagram>,
    /// The body as written in the textual syntax (one element per line).
    pub text: String,
    pub range: Range,
}

/// An element of a unit's body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitStepDiagram {
    pub id: String,
    /// `call`, `sequence`, `independent`, `priority`, `if`, `for`, `while`, `strict` or
    /// `rollback`.
    pub kind: String,
    /// The rule or unit a call calls.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub arguments: Vec<String>,
    /// The number of iterations of `for`, as written.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub count: Option<String>,
    /// The value of `strict` and `rollback`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub value: Option<bool>,
    /// The nested lists: a sequence's elements; the choices of `independent` and `priority`;
    /// the condition, `then` and `else` of `if`; the body of `for` and `while`.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub children: Vec<Vec<UnitStepDiagram>>,
    pub range: Range,
}

/// A class that nodes can have.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeClass {
    /// The qualified name (`pkg::Class`).
    pub id: String,
    /// The name as rules write it.
    pub name: String,
    pub is_abstract: bool,
    /// The class and its supertypes (ids), for telling which references fit.
    pub conforms_to: Vec<String>,
    /// Attributes, inherited ones too.
    pub attributes: Vec<NodeClassAttribute>,
    /// References, inherited ones too.
    pub references: Vec<NodeClassReference>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeClassAttribute {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    /// The literals of an enumeration-typed attribute.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub literals: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeClassReference {
    pub name: String,
    /// The id of the target class.
    pub target: String,
    pub containment: bool,
    pub many: bool,
}

/// A change of a transformation document, made by editing its text (see
/// `editTransformation`). Elements are named by their diagram ids; classes, types and values
/// are written as in the textual syntax.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum TransformationEdit {
    /// Replaces the imported packages (names as written after `ePackageImport`).
    SetImports {
        imports: Vec<String>,
    },
    AddRule {
        name: String,
    },
    AddUnit {
        name: String,
    },
    /// Renames a rule, unit, multi-rule, condition graph or node, and what refers to it in the
    /// document (calls, edges, `reuse`, formulas, node parameters).
    Rename {
        element: String,
        name: String,
    },
    /// Deletes rules, units, multi-rules, nodes (with their edges), edges, attributes and the
    /// elements of units; deleting a rule or unit deletes its calls.
    Delete {
        elements: Vec<String>,
    },
    /// Replaces the parameters of a rule or unit.
    SetParameters {
        element: String,
        parameters: Vec<TransformationParameterSpec>,
    },
    /// Changes the matching options of a rule or multi-rule.
    SetRuleOptions {
        element: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        check_dangling: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        injective_matching: Option<bool>,
    },
    /// Replaces the attribute conditions of a rule or multi-rule.
    SetConditions {
        element: String,
        conditions: Vec<String>,
    },
    /// Adds a node to a rule, multi-rule or condition graph.
    AddNode {
        scope: String,
        name: String,
        class: String,
        action: RuleAction,
    },
    /// Changes a node's class or action. Changing the action changes the actions of its
    /// edges and attributes so that they fit (an edge of a created node is created, …).
    UpdateNode {
        element: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        class: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        action: Option<RuleAction>,
    },
    /// Adds an edge between two nodes, to the scope given (default: the innermost scope of
    /// the two nodes).
    AddEdge {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        scope: Option<String>,
        source: String,
        target: String,
        reference: String,
        action: RuleAction,
    },
    UpdateEdge {
        element: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        action: Option<RuleAction>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        reference: Option<String>,
    },
    /// Adds an attribute to a node, written in `scope` (default: the node's own; a multi-rule
    /// or condition graph adds it with `reuse`).
    AddAttribute {
        node: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        scope: Option<String>,
        name: String,
        value: String,
        /// An action different from the node's (`create`, `delete`, `forbid`, `require`).
        #[serde(skip_serializing_if = "Option::is_none", default)]
        action: Option<RuleAction>,
        /// `set name=value`.
        #[serde(default)]
        set: bool,
    },
    UpdateAttribute {
        element: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        value: Option<String>,
        /// The action (the node's own action is written without a keyword).
        #[serde(skip_serializing_if = "Option::is_none", default)]
        action: Option<RuleAction>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        set: Option<bool>,
    },
    /// Adds a multi-rule to a rule or multi-rule.
    AddMultiRule {
        scope: String,
        name: String,
    },
    /// Replaces the body of a unit with elements written in the textual syntax.
    SetUnitBody {
        element: String,
        body: String,
    },
}

/// The text edits that make a [`TransformationEdit`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransformationEditResult {
    /// Non-overlapping edits in document order; ranges refer to the text before the edit.
    pub edits: Vec<TextEdit>,
    /// The id of the added or renamed element.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub element: Option<String>,
}

// ---------------------------------------------------------------------- QVT

/// A QVT transformation of an open document (`qvtTransformations`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QvtTransformationInfo {
    /// The document's name.
    pub document: String,
    /// `qvto` or `qvtr`.
    pub language: Language,
    pub name: String,
    pub parameters: Vec<QvtParameterInfo>,
    /// The parameters a run can create or change objects in: QVTo's `out` and `inout`
    /// parameters, the parameters QVTr relations enforce (the directions it can run in).
    pub targets: Vec<String>,
    /// The number of errors of the document (a transformation with errors cannot run).
    pub errors: usize,
    /// For `qvtTransformations` with a model: the parameters the model can be bound to (its
    /// objects are of the parameter's metamodel). Empty without a model.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub applicable: Vec<String>,
}

/// A model parameter of a QVT transformation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QvtParameterInfo {
    pub name: String,
    /// `in`, `out` or `inout` (QVTo); none for QVTr, whose parameters are inputs or the target
    /// depending on the direction of a run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    /// The model type (QVTo) or the imported metamodels (QVTr).
    pub model_type: String,
    /// The nsURIs of the packages of its metamodel.
    pub metamodels: Vec<String>,
}

/// A model bound to a parameter of a QVT transformation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QvtModelBinding {
    pub parameter: String,
    /// A model resource of the workspace: the URI of a loaded XMI model, or the name of an
    /// open model document (HUTN or XMI), as `models` reports them.
    pub model: String,
}

/// Where the model of an output parameter is written to (for its references to other
/// models and its schema locations; the engine does not write files).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QvtOutputPath {
    pub parameter: String,
    pub path: String,
}

/// What `runQvt` runs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QvtRunRequest {
    /// The QVT document (open in the workspace).
    pub document: String,
    /// The models of the input parameters (QVTo: `in` and `inout`; QVTr: all but the target,
    /// and the target too when an existing model is to be updated or checked).
    pub models: Vec<QvtModelBinding>,
    /// The paths the models of the output parameters will be written to (default:
    /// `<parameter>.xmi` next to the document).
    pub outputs: Vec<QvtOutputPath>,
    /// QVTr: the parameter to run in the direction of (needed when relations enforce more than
    /// one).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// QVTr: only check the target model against the relations.
    pub check: bool,
    /// `xmi` (default) or `hutn`: the format of the output models' text (by default, the
    /// output path's extension decides).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// QVTo: values of configuration properties, in OCL syntax (`'text'`, `3`, `true`).
    pub configuration: std::collections::BTreeMap<String, String>,
}

/// A model a QVT transformation produced or changed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QvtOutput {
    pub parameter: String,
    pub path: String,
    /// `xmi` or `hutn`.
    pub format: String,
    pub text: String,
    /// The number of objects in it.
    pub objects: usize,
}

/// The outcome of `runQvt`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QvtRunResult {
    pub outputs: Vec<QvtOutput>,
    /// The messages of `log` (and of failed assertions).
    pub log: Vec<String>,
    /// Problems met while running, located in the transformation.
    pub problems: Vec<Diagnostic>,
    /// QVTr in check mode: the bindings of relations that do not hold.
    pub violations: Vec<String>,
    /// The invariants the output models violate (`Class::Invariant on Object: message`).
    pub invariants: Vec<String>,
    /// How many mappings (QVTo) or relations (QVTr) were applied.
    pub applications: usize,
    /// How many objects were created.
    pub created: usize,
    /// The message of a fatal assertion that ended the transformation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aborted: Option<String>,
}

// ---------------------------------------------------------------------- projects

/// How the metamodel of a new project is written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetamodelFormat {
    /// An OCLinEcore document (`.oclinecore`): Ecore with embedded OCL, as text.
    #[default]
    #[serde(rename = "oclinecore")]
    OclInEcore,
    /// An `.ecore` file (Ecore's XML), registered in the project's package registry.
    Ecore,
}

/// What `newProject` creates: a modeling project around one metamodel.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewProjectRequest {
    /// The project's name; the metamodel's package is named after it (`My Shop` → `my_shop`).
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The nsURI of the metamodel's package (default: `http://example.org/<package>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ns_uri: Option<String>,
    #[serde(default)]
    pub metamodel: MetamodelFormat,
    /// The documents to create besides the metamodel, by language: `ocl` (constraints),
    /// `hutn` and `xmi` (a model), `henshin` and `henshinModel` (a Henshin transformation as
    /// text and as a `.henshin` file; both: a pair kept equivalent), `qvto` and `qvtr` (a QVT
    /// transformation). Default: all of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<Language>>,
    /// The code generation target of the manifest's `codegen` (`python`, `rust`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codegen: Option<String>,
}

/// A file of a new project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectFile {
    /// Relative to the project folder, with `/` separators.
    pub path: String,
    pub text: String,
}

/// The result of `newProject`. The files are returned, not written.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewProject {
    /// The name of the metamodel's package.
    pub package: String,
    pub ns_uri: String,
    /// The manifest (`oxychloride.json`) first, then the documents, then the rest.
    pub files: Vec<ProjectFile>,
}

/// What `newDocument` writes: the starting point of a document about loaded metamodels.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewDocumentRequest {
    /// `oclinecore`, `ocl`, `hutn`, `qvto` or `qvtr`.
    pub language: Language,
    /// The name of the new metamodel's package (OCLinEcore) or of the transformation (QVT);
    /// other documents mention it in their comment.
    pub name: String,
    /// Qualified names of loaded packages: those the Complete OCL document constrains, the
    /// metamodel of the model (HUTN: the first), the metamodels a QVT transformation
    /// transforms (from the first to the last).
    #[serde(default)]
    pub packages: Vec<String>,
    /// OCLinEcore: the new package's nsURI (default: `http://example.org/<name>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ns_uri: Option<String>,
}
