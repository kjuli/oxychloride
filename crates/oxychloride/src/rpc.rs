//! A JSON-RPC 2.0 interface to a [`Workspace`].
//!
//! The same [`Server`] backs `oxy serve` (newline-delimited JSON over stdio), the C ABI
//! and the language server, so every client sees the same methods. See `docs/PROTOCOL.md`.

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value as Json, json};

use std::collections::HashMap;

use crate::protocol::{
    CodeRequest, ImportEclipseRequest, Language, MetamodelEdit, NewDocumentRequest, NewProjectRequest, ObjectRef,
    PlatformSettings, Position, QvtRunRequest, RegistryEntry, TransformationEdit,
};
use crate::transform::ExploreRequest;
use crate::workspace::{Error, ValidateOptions, Workspace};

pub const PROTOCOL_VERSION: u32 = 1;

/// Method names, as reported by `initialize`.
pub const METHODS: &[&str] = &[
    "initialize",
    "loadEcore",
    "loadModel",
    "mapUri",
    "registry",
    "setRegistry",
    "platform",
    "setPlatform",
    "scanEclipseProjects",
    "importEclipseProjects",
    "reset",
    "clearModels",
    "reload",
    "openDocument",
    "closeDocument",
    "diagnostics",
    "validate",
    "evaluate",
    "packages",
    "models",
    "hover",
    "complete",
    "classAt",
    "exportDocument",
    "convertFile",
    "createObject",
    "containmentOptions",
    "schemas",
    "transformationUnits",
    "startTransformation",
    "transformationState",
    "findMatches",
    "applyTransformation",
    "exportTransformation",
    "exploreStateSpace",
    "stateSpaceState",
    "adoptState",
    "closeTransformation",
    "codeTargets",
    "generateCode",
    "metamodelDiagram",
    "editMetamodel",
    "transformationDiagram",
    "editTransformation",
    "syncTransformation",
    "qvtTransformations",
    "runQvt",
    "newProject",
    "newDocument",
];

pub mod codes {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const LOAD_ERROR: i64 = -32000;
    pub const NOT_FOUND: i64 = -32001;
}

#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        RpcError { code, message: message.into() }
    }
}

impl From<Error> for RpcError {
    fn from(e: Error) -> Self {
        let code = match &e {
            Error::Load(_) => codes::LOAD_ERROR,
            Error::NotFound(_) => codes::NOT_FOUND,
            Error::Invalid(_) => codes::INVALID_PARAMS,
        };
        RpcError::new(code, e.to_string())
    }
}

#[derive(Debug, Default)]
pub struct Server {
    workspace: Workspace,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoadParams {
    path: Option<String>,
    text: Option<String>,
    uri: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MapUriParams {
    from: String,
    to: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FolderParams {
    folder: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegistryParams {
    entries: Vec<RegistryEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DocumentParams {
    name: String,
    text: Option<String>,
    /// Defaults to the language of the name's extension.
    language: Option<Language>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditMetamodelParams {
    name: String,
    text: Option<String>,
    edit: MetamodelEdit,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct DiagnosticsParams {
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ValidateParams {
    ecore: bool,
    documents: Option<Vec<String>>,
    objects: Option<Vec<ObjectRef>>,
    include_satisfied: bool,
    contracts: bool,
}

impl Default for ValidateParams {
    fn default() -> Self {
        ValidateParams { ecore: true, documents: None, objects: None, include_satisfied: false, contracts: true }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EvaluateParams {
    expression: String,
    context: Option<ObjectRef>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct QvtTransformationsParams {
    /// A model resource: which parameters of each transformation it can be bound to.
    model: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportParams {
    name: String,
    /// The path the export will be written to (references to `.ecore` files are made
    /// relative to it).
    target: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConvertParams {
    path: String,
    to: Language,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PositionParams {
    name: String,
    /// Updated document text, if it changed since it was opened.
    text: Option<String>,
    position: Position,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateObjectParams {
    name: String,
    /// Updated document text, if it changed since it was opened.
    text: Option<String>,
    /// Qualified class name.
    class: String,
    container: Option<ContainerParams>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContainerParams {
    /// Fragment of an object of the document.
    fragment: String,
    feature: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ObjectParams {
    object: ObjectRef,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartTransformationParams {
    /// The model resource: URI, path, file name or document name.
    model: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionParams {
    session: u64,
    step: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FindMatchesParams {
    session: u64,
    /// The Henshin document.
    document: String,
    text: Option<String>,
    rule: String,
    #[serde(default)]
    parameters: serde_json::Map<String, Json>,
    step: Option<usize>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApplyParams {
    session: u64,
    document: String,
    text: Option<String>,
    /// A rule or unit.
    unit: String,
    #[serde(default)]
    parameters: serde_json::Map<String, Json>,
    /// For rules: the index of a match of `findMatches` (default: the first match).
    #[serde(rename = "match")]
    match_index: Option<usize>,
    step: Option<usize>,
    /// Seeds the random choices of independent units.
    seed: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportStateParams {
    session: u64,
    step: Option<usize>,
    format: Language,
    target: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExploreParams {
    session: u64,
    document: String,
    text: Option<String>,
    rules: Vec<String>,
    /// Fixed parameter values by rule.
    #[serde(default)]
    parameters: HashMap<String, serde_json::Map<String, Json>>,
    step: Option<usize>,
    max_states: Option<usize>,
    max_depth: Option<usize>,
    validate: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StateParams {
    session: u64,
    state: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditTransformationParams {
    name: String,
    text: Option<String>,
    edit: TransformationEdit,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncTransformationParams {
    /// The document whose meaning the other gets.
    name: String,
    text: Option<String>,
    /// The document of the other syntax, and its current text.
    target: String,
    target_text: String,
}

/// Updates an open document whose text the client changed meanwhile.
fn update_text(ws: &mut Workspace, name: &str, text: Option<&str>) -> Result<(), RpcError> {
    if let Some(text) = text
        && ws.document_text(name) != Some(text)
    {
        let language = ws.document_language(name).unwrap_or_else(|| Language::from_path(name));
        ws.open_document_as(name, text, language);
    }
    if ws.document_text(name).is_none() {
        return Err(RpcError::new(codes::NOT_FOUND, format!("no open document named '{name}'")));
    }
    Ok(())
}

/// The text of a metamodel document: an `.ecore` file's text is passed on as is (it is not a
/// workspace document), an OCLinEcore document is updated first.
fn metamodel_text<'a>(ws: &mut Workspace, name: &str, text: Option<&'a str>) -> Result<Option<&'a str>, RpcError> {
    if crate::diagram::is_ecore(name) {
        return Ok(text);
    }
    update_text(ws, name, text)?;
    Ok(None)
}

fn params<T: DeserializeOwned>(p: Json) -> Result<T, RpcError> {
    let p = if p.is_null() { json!({}) } else { p };
    serde_json::from_value(p).map_err(|e| RpcError::new(codes::INVALID_PARAMS, format!("invalid params: {e}")))
}

fn to_json<T: serde::Serialize>(v: T) -> Result<Json, RpcError> {
    serde_json::to_value(v).map_err(|e| RpcError::new(codes::INVALID_REQUEST, e.to_string()))
}

impl Server {
    pub fn new() -> Self {
        Server::default()
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    pub fn workspace_mut(&mut self) -> &mut Workspace {
        &mut self.workspace
    }

    /// Calls a method directly (without the JSON-RPC envelope).
    pub fn call(&mut self, method: &str, p: Json) -> Result<Json, RpcError> {
        let ws = &mut self.workspace;
        match method {
            "initialize" => Ok(json!({
                "name": "oxychloride",
                "version": env!("CARGO_PKG_VERSION"),
                "protocolVersion": PROTOCOL_VERSION,
                "methods": METHODS,
            })),
            "loadEcore" | "loadModel" => {
                let p: LoadParams = params(p)?;
                let ecore = method == "loadEcore";
                let result = match (p.path, p.text) {
                    (Some(path), None) if ecore => ws.load_ecore_file(&path),
                    (Some(path), None) => ws.load_model_file(&path),
                    (None, Some(text)) => {
                        let uri =
                            p.uri.unwrap_or_else(|| if ecore { "memory.ecore".into() } else { "memory.xmi".into() });
                        if ecore { ws.load_ecore_str(&text, &uri) } else { ws.load_model_str(&text, &uri) }
                    }
                    _ => return Err(RpcError::new(codes::INVALID_PARAMS, "expected either 'path' or 'text'")),
                }?;
                to_json(result)
            }
            "mapUri" => {
                let p: MapUriParams = params(p)?;
                ws.map_uri(&p.from, &p.to);
                Ok(Json::Null)
            }
            "registry" => to_json(ws.registry()),
            "setRegistry" => {
                let p: RegistryParams = params(p)?;
                to_json(ws.set_registry(p.entries))
            }
            "platform" => to_json(ws.platform()),
            "setPlatform" => {
                let p: PlatformSettings = params(p)?;
                to_json(ws.set_platform(p))
            }
            "scanEclipseProjects" => {
                let p: FolderParams = params(p)?;
                to_json(crate::platform::scan(std::path::Path::new(&p.folder)))
            }
            "importEclipseProjects" => {
                let p: ImportEclipseRequest = params(p)?;
                to_json(Workspace::import_eclipse_projects(&p)?)
            }
            "reset" => {
                ws.reset();
                Ok(Json::Null)
            }
            "clearModels" => {
                ws.clear_models();
                Ok(Json::Null)
            }
            "reload" => to_json(json!({ "problems": ws.reload() })),
            "openDocument" => {
                let p: DocumentParams = params(p)?;
                let text = p.text.ok_or_else(|| RpcError::new(codes::INVALID_PARAMS, "missing 'text'"))?;
                let language = p.language.unwrap_or_else(|| Language::from_path(&p.name));
                let diagnostics = ws.open_document_as(&p.name, &text, language);
                to_json(json!({ "diagnostics": diagnostics, "language": language }))
            }
            "closeDocument" => {
                let p: DocumentParams = params(p)?;
                to_json(json!({ "closed": ws.close_document(&p.name) }))
            }
            "diagnostics" => {
                let p: DiagnosticsParams = params(p)?;
                if let Some(n) = &p.name
                    && ws.document_text(n).is_none()
                {
                    return Err(RpcError::new(codes::NOT_FOUND, format!("no open document named '{n}'")));
                }
                to_json(json!({ "diagnostics": ws.diagnostics(p.name.as_deref()) }))
            }
            "validate" => {
                let p: ValidateParams = params(p)?;
                let options = ValidateOptions {
                    ecore: p.ecore,
                    documents: p.documents,
                    objects: p.objects,
                    include_satisfied: p.include_satisfied,
                    contracts: p.contracts,
                };
                to_json(ws.validate(&options)?)
            }
            "evaluate" => {
                let p: EvaluateParams = params(p)?;
                to_json(ws.evaluate(&p.expression, p.context.as_ref())?)
            }
            "packages" => to_json(ws.packages()),
            "models" => to_json(ws.models()),
            "hover" | "complete" | "classAt" => {
                let p: PositionParams = params(p)?;
                update_text(ws, &p.name, p.text.as_deref())?;
                match method {
                    "hover" => to_json(ws.hover(&p.name, p.position)),
                    "complete" => to_json(ws.complete(&p.name, p.position)),
                    _ => to_json(ws.class_at(&p.name, p.position)),
                }
            }
            "exportDocument" => {
                let p: ExportParams = params(p)?;
                to_json(ws.export_document(&p.name, p.target.as_deref())?)
            }
            "convertFile" => {
                let p: ConvertParams = params(p)?;
                to_json(json!({ "text": ws.convert_file(&p.path, p.to)? }))
            }
            "createObject" => {
                let p: CreateObjectParams = params(p)?;
                update_text(ws, &p.name, p.text.as_deref())?;
                let container = p.container.as_ref().map(|c| (c.fragment.as_str(), c.feature.as_str()));
                to_json(ws.create_object(&p.name, &p.class, container)?)
            }
            "containmentOptions" => {
                let p: ObjectParams = params(p)?;
                to_json(ws.containment_options(&p.object)?)
            }
            "schemas" => to_json(ws.schemas()),
            "transformationUnits" => {
                let p: DocumentParams = params(p)?;
                update_text(ws, &p.name, p.text.as_deref())?;
                to_json(ws.transformation_units(&p.name)?)
            }
            "startTransformation" => {
                let p: StartTransformationParams = params(p)?;
                to_json(ws.start_transformation(&p.model)?)
            }
            "transformationState" => {
                let p: SessionParams = params(p)?;
                to_json(ws.transformation_state(p.session, p.step)?)
            }
            "findMatches" => {
                let p: FindMatchesParams = params(p)?;
                update_text(ws, &p.document, p.text.as_deref())?;
                let limit = p.limit.unwrap_or(100);
                to_json(ws.find_matches(p.session, &p.document, &p.rule, &p.parameters, p.step, limit)?)
            }
            "applyTransformation" => {
                let p: ApplyParams = params(p)?;
                update_text(ws, &p.document, p.text.as_deref())?;
                let seed = p.seed.unwrap_or(1);
                to_json(ws.apply_transformation(
                    p.session,
                    &p.document,
                    &p.unit,
                    &p.parameters,
                    p.match_index,
                    p.step,
                    seed,
                )?)
            }
            "exportTransformation" => {
                let p: ExportStateParams = params(p)?;
                to_json(json!({ "text": ws.export_transformation(p.session, p.step, p.format, p.target.as_deref())? }))
            }
            "exploreStateSpace" => {
                let p: ExploreParams = params(p)?;
                update_text(ws, &p.document, p.text.as_deref())?;
                let defaults = ExploreRequest::default();
                let request = ExploreRequest {
                    rules: p.rules,
                    parameters: p.parameters,
                    step: p.step,
                    max_states: p.max_states.unwrap_or(defaults.max_states),
                    max_depth: p.max_depth,
                    validate: p.validate.unwrap_or(defaults.validate),
                };
                to_json(ws.explore_state_space(p.session, &p.document, &request)?)
            }
            "stateSpaceState" => {
                let p: StateParams = params(p)?;
                to_json(ws.state_space_state(p.session, p.state)?)
            }
            "adoptState" => {
                let p: StateParams = params(p)?;
                to_json(ws.adopt_state(p.session, p.state)?)
            }
            "closeTransformation" => {
                let p: SessionParams = params(p)?;
                to_json(json!({ "closed": ws.close_transformation(p.session) }))
            }
            "codeTargets" => to_json(crate::code_targets()),
            "generateCode" => {
                let p: CodeRequest = params(p)?;
                to_json(ws.generate_code(&p)?)
            }
            "metamodelDiagram" => {
                let p: DocumentParams = params(p)?;
                let text = metamodel_text(ws, &p.name, p.text.as_deref())?;
                to_json(ws.metamodel_diagram(&p.name, text)?)
            }
            "editMetamodel" => {
                let p: EditMetamodelParams = params(p)?;
                let text = metamodel_text(ws, &p.name, p.text.as_deref())?;
                to_json(ws.edit_metamodel(&p.name, text, &p.edit)?)
            }
            "transformationDiagram" => {
                let p: DocumentParams = params(p)?;
                update_text(ws, &p.name, p.text.as_deref())?;
                to_json(ws.transformation_diagram(&p.name)?)
            }
            "editTransformation" => {
                let p: EditTransformationParams = params(p)?;
                update_text(ws, &p.name, p.text.as_deref())?;
                to_json(ws.edit_transformation(&p.name, &p.edit)?)
            }
            "syncTransformation" => {
                let p: SyncTransformationParams = params(p)?;
                update_text(ws, &p.name, p.text.as_deref())?;
                to_json(json!({ "edits": ws.sync_transformation(&p.name, &p.target, &p.target_text)? }))
            }
            "qvtTransformations" => {
                let p: QvtTransformationsParams = params(p)?;
                to_json(json!({ "transformations": ws.qvt_transformations(p.model.as_deref())? }))
            }
            "runQvt" => {
                let p: QvtRunRequest = params(p)?;
                to_json(ws.run_qvt(&p)?)
            }
            "newProject" => {
                let p: NewProjectRequest = params(p)?;
                to_json(Workspace::new_project(&p)?)
            }
            "newDocument" => {
                let p: NewDocumentRequest = params(p)?;
                to_json(json!({ "text": ws.new_document(&p)? }))
            }
            other => Err(RpcError::new(codes::METHOD_NOT_FOUND, format!("unknown method '{other}'"))),
        }
    }

    /// Handles a JSON-RPC request (or batch). Returns `None` for notifications.
    pub fn handle(&mut self, request: Json) -> Option<Json> {
        if let Json::Array(batch) = request {
            let responses: Vec<Json> = batch.into_iter().filter_map(|r| self.handle(r)).collect();
            return (!responses.is_empty()).then_some(Json::Array(responses));
        }
        let id = request.get("id").cloned();
        let Some(method) = request.get("method").and_then(Json::as_str).map(str::to_string) else {
            return Some(error_response(
                id.unwrap_or(Json::Null),
                RpcError::new(codes::INVALID_REQUEST, "missing 'method'"),
            ));
        };
        let p = request.get("params").cloned().unwrap_or(Json::Null);
        let result = self.call(&method, p);
        let id = id?;
        Some(match result {
            Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
            Err(e) => error_response(id, e),
        })
    }

    /// Handles a request given as JSON text and returns the response as JSON text.
    pub fn handle_str(&mut self, request: &str) -> Option<String> {
        let response = match serde_json::from_str::<Json>(request) {
            Ok(r) => self.handle(r)?,
            Err(e) => error_response(Json::Null, RpcError::new(codes::PARSE_ERROR, format!("parse error: {e}"))),
        };
        Some(response.to_string())
    }
}

fn error_response(id: Json, e: RpcError) -> Json {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": e.code, "message": e.message } })
}
