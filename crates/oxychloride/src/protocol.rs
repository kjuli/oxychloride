//! Serializable data exchanged with clients (JSON-RPC, C ABI, the desktop app).
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
    /// XMI: an instance model as EMF writes it (`.xmi`), edited as text.
    Xmi,
}

impl Language {
    /// Whether documents of this language are models (their objects are part of the workspace's
    /// models), rather than metamodels or constraints.
    pub fn is_model(self) -> bool {
        matches!(self, Language::Hutn | Language::Xmi)
    }

    /// The language of a document, by its file extension (Complete OCL by default).
    pub fn from_path(name: &str) -> Language {
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        match ext.as_str() {
            "oclinecore" => Language::OclInEcore,
            "hutn" => Language::Hutn,
            "xmi" => Language::Xmi,
            _ => Language::Ocl,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Language::Ocl => "ocl",
            Language::OclInEcore => "oclinecore",
            Language::Hutn => "hutn",
            Language::Xmi => "xmi",
        }
    }
}

/// Where a piece of OCL comes from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Origin {
    /// A Complete OCL document, identified by the name it was opened with.
    Document { name: String },
    /// An expression embedded in an `.ecore` file.
    Ecore { resource: String, element: String, key: String },
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
