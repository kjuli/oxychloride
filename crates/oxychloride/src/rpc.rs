//! A JSON-RPC 2.0 interface to a [`Workspace`].
//!
//! The same [`Server`] backs `oxy serve` (newline-delimited JSON over stdio), the C ABI
//! and the desktop app, so every client sees the same methods. See `docs/PROTOCOL.md`.

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value as Json, json};

use crate::protocol::{Language, ObjectRef, Position};
use crate::workspace::{Error, ValidateOptions, Workspace};

pub const PROTOCOL_VERSION: u32 = 1;

/// Method names, as reported by `initialize`.
pub const METHODS: &[&str] = &[
    "initialize",
    "loadEcore",
    "loadModel",
    "mapUri",
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
    "exportDocument",
    "convertFile",
    "createObject",
    "containmentOptions",
    "schemas",
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
struct DocumentParams {
    name: String,
    text: Option<String>,
    /// Defaults to the language of the name's extension.
    language: Option<Language>,
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
            "hover" | "complete" => {
                let p: PositionParams = params(p)?;
                update_text(ws, &p.name, p.text.as_deref())?;
                if method == "hover" {
                    to_json(ws.hover(&p.name, p.position))
                } else {
                    to_json(ws.complete(&p.name, p.position))
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
