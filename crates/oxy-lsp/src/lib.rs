//! A language server (LSP over stdio) for Complete OCL, OCLinEcore, HUTN, XMI and Henshin
//! documents.
//!
//! The server keeps an [`rpc::Server`] — the one behind `oxy serve` and the C ABI — and maps
//! the standard LSP features onto its workspace:
//!
//! * documents open in the editor are workspace documents, named by their path (or, if unsaved,
//!   their URI); the language comes from the editor's language id. The OCLinEcore, Complete OCL
//!   and Henshin files (texts and `.henshin` models) of the workspace folders are loaded as well
//!   (from disk while they are not open), so models find their metamodels and constraints, and
//!   transformations their rules, without opening them. A changed `.ecore`
//!   file that is loaded (or registered) reloads the metamodels. The package registry
//!   (`setRegistry`) can be given as `initializationOptions.registry`, so that it is there
//!   before the workspace folders are loaded;
//! * once changes settle, the diagnostics of all documents are published: models are checked
//!   against the constraints (invariants, pre- and postconditions) as they are edited;
//! * hover and completion;
//! * the request `oxychloride/rpc` (`{method, params}`) calls any engine method (`evaluate`,
//!   `validate`, `packages`, `models`, `createObject`, `schemas`, the transformation methods, …,
//!   see `docs/PROTOCOL.md`).
//!   Positions in its parameters and results count Unicode scalar values, like the engine's;
//! * the notification `oxychloride/workspaceChanged` tells the client that the packages,
//!   models or the state of the package registry changed (e.g. to refresh views).
//!
//! LSP positions count UTF-16 code units; they are converted using the document text.

use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::time::Duration;

use crossbeam_channel::{RecvTimeoutError, Sender};
use lsp_server::{Connection, ErrorCode, Message, Notification, Request, Response};
use oxychloride::rpc;
use oxychloride::{CompletionKind, Diagnostic, Language, Origin, Position, Range, Severity, Workspace};
use serde_json::{Value as Json, json};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// How long changes must settle before diagnostics are recomputed.
const SETTLE: Duration = Duration::from_millis(200);

/// Directories not searched for metamodels and constraints (besides hidden ones).
const SKIPPED_DIRS: &[&str] = &["node_modules", "target", "out", "dist", "build"];
const MAX_DEPTH: usize = 12;

/// Engine methods that change the workspace (diagnostics are published again after them).
const MUTATING_METHODS: &[&str] = &[
    "loadEcore",
    "loadModel",
    "mapUri",
    "setRegistry",
    "reset",
    "clearModels",
    "reload",
    "openDocument",
    "closeDocument",
];

/// Serves LSP on stdin/stdout until the client exits.
pub fn run_stdio() -> Result<()> {
    let (connection, io) = Connection::stdio();
    serve(&connection)?;
    drop(connection);
    io.join()?;
    Ok(())
}

/// Serves LSP on a connection, from `initialize` to `shutdown`.
pub fn serve(connection: &Connection) -> Result<()> {
    let params = connection.initialize(capabilities())?;
    let mut server = LspServer::new(connection.sender.clone());
    // The package registry first, so that the workspace files find registered metamodels.
    if let Some(entries) = params.pointer("/initializationOptions/registry") {
        let entries = serde_json::from_value(entries.clone()).unwrap_or_default();
        server.ws().set_registry(entries);
    }
    server.index(&workspace_folders(&params));
    loop {
        let message = if server.dirty {
            match connection.receiver.recv_timeout(SETTLE) {
                Ok(m) => m,
                Err(RecvTimeoutError::Timeout) => {
                    server.guarded("publishing diagnostics", |s| s.publish());
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match connection.receiver.recv() {
                Ok(m) => m,
                Err(_) => break,
            }
        };
        match message {
            Message::Request(request) => {
                if connection.handle_shutdown(&request)? {
                    break;
                }
                let id = request.id.clone();
                let method = request.method.clone();
                let response = server.guarded(&method, |s| s.request(request)).unwrap_or_else(|| {
                    Response::new_err(id, ErrorCode::InternalError as i32, format!("the engine failed on '{method}'"))
                });
                connection.sender.send(Message::Response(response))?;
            }
            Message::Notification(n) => {
                let method = n.method.clone();
                server.guarded(&method, |s| s.notification(n));
            }
            Message::Response(_) => {}
        }
    }
    Ok(())
}

fn capabilities() -> Json {
    json!({
        "textDocumentSync": { "openClose": true, "change": 1 },
        "hoverProvider": true,
        "completionProvider": { "triggerCharacters": [".", ">", ":", "\"", ","] },
        "workspace": { "workspaceFolders": { "supported": true, "changeNotifications": true } },
        "experimental": { "oxychloride": { "protocolVersion": rpc::PROTOCOL_VERSION } },
    })
}

/// The paths of the workspace folders in `initialize` parameters.
fn workspace_folders(params: &Json) -> Vec<String> {
    match params.get("workspaceFolders").and_then(Json::as_array) {
        Some(folders) => folders.iter().filter_map(|f| uri_to_path(f.get("uri")?.as_str()?)).collect(),
        None => params.get("rootUri").and_then(Json::as_str).and_then(uri_to_path).into_iter().collect(),
    }
}

/// The language of a document the server handles, by its extension.
fn language_of(path: &str) -> Option<Language> {
    let ext = Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "ocl" => Some(Language::Ocl),
        "oclinecore" => Some(Language::OclInEcore),
        "hutn" => Some(Language::Hutn),
        "xmi" => Some(Language::Xmi),
        "henshin_text" => Some(Language::Henshin),
        "henshin" => Some(Language::HenshinModel),
        _ => None,
    }
}

/// The language of a document the client opened with `language_id`.
fn editor_language(name: &str, language_id: &str) -> Option<Language> {
    match language_id {
        "ocl" => Some(Language::Ocl),
        "oclinecore" => Some(Language::OclInEcore),
        "hutn" => Some(Language::Hutn),
        "henshin" => Some(Language::Henshin),
        // Of all XML documents, `.xmi` files are models and `.henshin` files transformations.
        _ => language_of(name),
    }
}

/// The name of the document at `uri` in the engine: its path, or the URI itself for unsaved
/// documents (`untitled:`).
pub fn document_name(uri: &str) -> String {
    uri_to_path(uri).unwrap_or_else(|| uri.to_string())
}

/// Whether files of a language are loaded from the workspace folders even when not open.
fn is_indexed(language: Language) -> bool {
    matches!(language, Language::Ocl | Language::OclInEcore | Language::Henshin | Language::HenshinModel)
}

struct LspServer {
    sender: Sender<Message>,
    rpc: rpc::Server,
    /// Documents open in the editor (by name), with their URI as the client wrote it.
    open: HashMap<String, String>,
    /// The languages of the documents open in the editor.
    languages: HashMap<String, Language>,
    /// Texts of open documents not given to the engine yet.
    pending: HashMap<String, String>,
    /// Workspace files loaded from disk because they are not open.
    indexed: HashSet<String>,
    /// The diagnostics last published, by URI.
    published: HashMap<String, Json>,
    /// The packages, models and registry entries last announced (as JSON text).
    browsed: String,
    /// Diagnostics must be published again.
    dirty: bool,
}

impl LspServer {
    fn new(sender: Sender<Message>) -> Self {
        LspServer {
            sender,
            rpc: rpc::Server::new(),
            open: HashMap::new(),
            languages: HashMap::new(),
            pending: HashMap::new(),
            indexed: HashSet::new(),
            published: HashMap::new(),
            browsed: String::new(),
            dirty: true,
        }
    }

    /// Runs `f`, surviving a panic of the engine (a bug: it must not panic on any input). The
    /// workspace is then rebuilt from the documents, as it may be inconsistent.
    fn guarded<T>(&mut self, what: &str, f: impl FnOnce(&mut Self) -> T) -> Option<T> {
        match catch_unwind(AssertUnwindSafe(|| f(self))) {
            Ok(value) => Some(value),
            Err(_) => {
                eprintln!("oxy lsp: the engine panicked while handling {what}; rebuilding the workspace");
                let documents: Vec<(String, String, Language)> = self
                    .rpc
                    .workspace()
                    .document_names()
                    .filter_map(|n| {
                        let ws = self.rpc.workspace();
                        Some((n.to_string(), ws.document_text(n)?.to_string(), ws.document_language(n)?))
                    })
                    .collect();
                self.rpc = rpc::Server::new();
                for (name, text, language) in documents {
                    let _ = catch_unwind(AssertUnwindSafe(|| self.ws().open_document_as(&name, &text, language)));
                }
                self.dirty = true;
                None
            }
        }
    }

    fn ws(&mut self) -> &mut Workspace {
        self.rpc.workspace_mut()
    }

    // ------------------------------------------------------------------ documents

    /// Loads the metamodels and constraints of workspace folders.
    fn index(&mut self, folders: &[String]) {
        let mut files = Vec::new();
        for folder in folders {
            collect_files(Path::new(folder), 0, &mut files);
        }
        for path in files {
            self.load_from_disk(&path);
        }
        // Imports between documents resolve once all are there.
        self.ws().reload();
        self.dirty = true;
    }

    /// Loads (or reloads) a workspace file that is not open in the editor.
    fn load_from_disk(&mut self, path: &str) {
        if self.open.contains_key(path) {
            return;
        }
        let Some(language) = language_of(path).filter(|l| is_indexed(*l)) else { return };
        match std::fs::read_to_string(path) {
            Ok(text) => {
                self.ws().open_document_as(path, &text, language);
                self.indexed.insert(path.to_string());
            }
            Err(_) => self.forget(path),
        }
    }

    /// Closes a document that is neither open nor (any more) on disk.
    fn forget(&mut self, path: &str) {
        self.indexed.remove(path);
        self.ws().close_document(path);
    }

    /// Gives the engine the latest text of every changed document.
    fn flush(&mut self) {
        for (name, text) in std::mem::take(&mut self.pending) {
            if let Some(language) = self.languages.get(&name).copied() {
                self.ws().open_document_as(&name, &text, language);
            }
        }
    }

    fn uri_of(&self, path: &str) -> String {
        self.open.get(path).cloned().unwrap_or_else(|| path_to_uri(path))
    }

    // ------------------------------------------------------------------ notifications

    fn notification(&mut self, n: Notification) {
        let p = &n.params;
        let uri = p.pointer("/textDocument/uri").and_then(Json::as_str).unwrap_or_default().to_string();
        let name = document_name(&uri);
        if n.method == "textDocument/didOpen" {
            let language_id = p.pointer("/textDocument/languageId").and_then(Json::as_str).unwrap_or_default();
            let Some(language) = editor_language(&name, language_id) else { return };
            let text = p.pointer("/textDocument/text").and_then(Json::as_str).unwrap_or_default();
            self.open.insert(name.clone(), uri);
            self.languages.insert(name.clone(), language);
            self.indexed.remove(&name);
            self.pending.insert(name, text.to_string());
            self.dirty = true;
            return;
        }
        let known = self.open.contains_key(&name).then_some(name);
        match (n.method.as_str(), known) {
            ("textDocument/didChange", Some(path)) => {
                let changes = p.get("contentChanges").and_then(Json::as_array);
                if let Some(text) = changes.and_then(|c| c.last()).and_then(|c| c.get("text")).and_then(Json::as_str) {
                    self.pending.insert(path, text.to_string());
                    self.dirty = true;
                }
            }
            ("textDocument/didClose", Some(path)) => {
                self.open.remove(&path);
                self.languages.remove(&path);
                self.pending.remove(&path);
                if language_of(&path).is_some_and(is_indexed) && Path::new(&path).is_file() {
                    self.load_from_disk(&path);
                } else {
                    self.ws().close_document(&path);
                }
                self.dirty = true;
            }
            ("workspace/didChangeWatchedFiles", _) => {
                let mut reload = false;
                for change in p.get("changes").and_then(Json::as_array).into_iter().flatten() {
                    let Some(path) = change.get("uri").and_then(Json::as_str).and_then(uri_to_path) else { continue };
                    let ws = self.rpc.workspace();
                    let registered = ws.registry_entries().iter().any(|e| Path::new(&e.location) == Path::new(&path));
                    if path.to_ascii_lowercase().ends_with(".ecore") || registered {
                        // Only metamodels in use (loaded or registered); not e.g. one generated
                        // from an OCLinEcore document.
                        reload |= registered || ws.metamodel().resource_of_uri(&path).is_some();
                    } else if change.get("type").and_then(Json::as_u64) == Some(3) {
                        if self.indexed.contains(&path) {
                            self.forget(&path);
                        }
                    } else {
                        self.load_from_disk(&path);
                    }
                }
                if reload {
                    self.ws().reload();
                }
                self.dirty = true;
            }
            ("workspace/didChangeWorkspaceFolders", _) => {
                let added = p.pointer("/event/added").and_then(Json::as_array).into_iter().flatten();
                let folders: Vec<String> =
                    added.filter_map(|f| uri_to_path(f.get("uri")?.as_str()?)).collect::<Vec<_>>();
                self.index(&folders);
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------ requests

    fn request(&mut self, r: Request) -> Response {
        self.flush();
        let result = match r.method.as_str() {
            "textDocument/hover" => Ok(self.hover(&r.params)),
            "textDocument/completion" => Ok(self.completion(&r.params)),
            "oxychloride/rpc" => {
                let method = r.params.get("method").and_then(Json::as_str).unwrap_or_default().to_string();
                let params = r.params.get("params").cloned().unwrap_or(Json::Null);
                let result = self.rpc.call(&method, params).map_err(|e| (e.code as i32, e.message));
                if MUTATING_METHODS.contains(&method.as_str()) {
                    self.dirty = true;
                }
                result
            }
            other => Err((ErrorCode::MethodNotFound as i32, format!("unknown method '{other}'"))),
        };
        match result {
            Ok(value) => Response::new_ok(r.id, value),
            Err((code, message)) => Response::new_err(r.id, code, message),
        }
    }

    /// The document path, its text and the engine position of a text document position request.
    fn position(&self, params: &Json) -> Option<(String, String, Position)> {
        let path = document_name(params.pointer("/textDocument/uri")?.as_str()?);
        let text = self.rpc.workspace().document_text(&path)?.to_string();
        let line = params.pointer("/position/line")?.as_u64()? as u32;
        let character = params.pointer("/position/character")?.as_u64()? as usize;
        let column = scalar_column(line_text(&text, line), character);
        Some((path, text, Position { line, column }))
    }

    fn hover(&self, params: &Json) -> Json {
        let Some((path, text, position)) = self.position(params) else { return Json::Null };
        let Some(info) = self.rpc.workspace().hover(&path, position) else { return Json::Null };
        // The code block's language is the editor's language id.
        let language = match self.rpc.workspace().document_language(&path) {
            Some(Language::Henshin | Language::HenshinModel) => "henshin",
            Some(l) => l.extension(),
            None => "ocl",
        };
        json!({
            "contents": { "kind": "markdown", "value": format!("```{language}\n{}\n```", info.contents) },
            "range": lsp_range(Some(&text), info.range),
        })
    }

    fn completion(&self, params: &Json) -> Json {
        let Some((path, text, position)) = self.position(params) else { return Json::Null };
        let Some(list) = self.rpc.workspace().complete(&path, position) else { return Json::Null };
        let range = lsp_range(Some(&text), list.range);
        let items: Vec<Json> = list
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                json!({
                    "label": item.label,
                    "kind": completion_kind(item.kind),
                    "detail": item.detail,
                    "sortText": format!("{i:05}"),
                    "filterText": item.label,
                    "textEdit": { "range": range, "newText": item.insert.as_deref().unwrap_or(&item.label) },
                })
            })
            .collect();
        json!({ "isIncomplete": false, "items": items })
    }

    // ------------------------------------------------------------------ publishing

    /// Publishes the diagnostics that changed, and announces changed packages or models.
    fn publish(&mut self) {
        self.flush();
        self.dirty = false;
        let ws = self.rpc.workspace();
        let mut by_path: HashMap<String, Vec<Json>> = HashMap::new();
        for name in ws.document_names() {
            by_path.entry(name.to_string()).or_default();
        }
        let mut files: HashMap<String, Option<String>> = HashMap::new();
        for d in ws.diagnostics(None) {
            let (path, d) = match &d.origin {
                Origin::Document { name } => (name.clone(), d),
                Origin::File { path } => (path.clone(), d),
                // The range is relative to the annotation's text: show it on the file.
                Origin::Ecore { resource, .. } => (
                    resource.clone(),
                    Diagnostic { message: format!("{}: {}", d.source, d.message), range: Range::default(), ..d },
                ),
                Origin::Expression => continue,
            };
            let text = match ws.document_text(&path) {
                Some(t) => Some(t.to_string()),
                None => files.entry(path.clone()).or_insert_with(|| std::fs::read_to_string(&path).ok()).clone(),
            };
            by_path.entry(path).or_default().push(lsp_diagnostic(text.as_deref(), &d));
        }
        let mut next = HashMap::new();
        for (path, diagnostics) in by_path {
            next.insert(self.uri_of(&path), Json::Array(diagnostics));
        }
        for uri in self.published.keys() {
            if !next.contains_key(uri) {
                self.notify("textDocument/publishDiagnostics", json!({ "uri": uri, "diagnostics": [] }));
            }
        }
        for (uri, diagnostics) in &next {
            if self.published.get(uri) != Some(diagnostics) {
                self.notify("textDocument/publishDiagnostics", json!({ "uri": uri, "diagnostics": diagnostics }));
            }
        }
        self.published = next;
        let ws = self.rpc.workspace();
        let browsed = json!([ws.packages(), ws.models(), ws.registry()]).to_string();
        if browsed != self.browsed {
            self.browsed = browsed;
            self.notify("oxychloride/workspaceChanged", Json::Null);
        }
    }

    fn notify(&self, method: &str, params: Json) {
        let _ = self.sender.send(Message::Notification(Notification::new(method.to_string(), params)));
    }
}

fn lsp_diagnostic(text: Option<&str>, d: &Diagnostic) -> Json {
    let severity = match d.severity {
        Severity::Error => 1,
        Severity::Warning => 2,
        Severity::Info => 3,
    };
    json!({ "range": lsp_range(text, d.range), "severity": severity, "source": "ocl", "message": d.message })
}

fn completion_kind(kind: CompletionKind) -> u32 {
    match kind {
        CompletionKind::Keyword => 14,
        CompletionKind::Type => 7,
        CompletionKind::Property => 10,
        CompletionKind::Operation => 2,
        CompletionKind::Iterator => 3,
        CompletionKind::Variable => 6,
        CompletionKind::EnumLiteral => 20,
        CompletionKind::Snippet => 15,
        CompletionKind::Object => 18,
    }
}

/// Files below `dir` that are loaded even when not open.
fn collect_files(dir: &Path, depth: usize, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            if depth < MAX_DEPTH && !name.starts_with('.') && !SKIPPED_DIRS.contains(&name.as_str()) {
                collect_files(&path, depth + 1, out);
            }
        } else if let Some(path) = path.to_str()
            && language_of(path).is_some_and(is_indexed)
        {
            out.push(path.to_string());
        }
    }
}

// ---------------------------------------------------------------------- positions

fn line_text(text: &str, line: u32) -> &str {
    let line = text.split('\n').nth(line as usize).unwrap_or("");
    line.strip_suffix('\r').unwrap_or(line)
}

/// The number of scalar values in the first `utf16` code units of `line`.
fn scalar_column(line: &str, utf16: usize) -> u32 {
    let mut units = 0;
    let mut scalars = 0;
    for c in line.chars() {
        if units >= utf16 {
            break;
        }
        units += c.len_utf16();
        scalars += 1;
    }
    scalars
}

/// The number of UTF-16 code units in the first `column` scalar values of `line`.
fn utf16_column(line: &str, column: u32) -> usize {
    line.chars().take(column as usize).map(char::len_utf16).sum()
}

fn lsp_position(text: Option<&str>, p: Position) -> Json {
    let character = match text {
        Some(text) => utf16_column(line_text(text, p.line), p.column),
        None => p.column as usize,
    };
    json!({ "line": p.line, "character": character })
}

fn lsp_range(text: Option<&str>, r: Range) -> Json {
    json!({ "start": lsp_position(text, r.start), "end": lsp_position(text, r.end) })
}

// ---------------------------------------------------------------------- URIs

/// The path of a `file:` URI.
pub fn uri_to_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    // The authority (usually empty) ends at the path.
    let path = &rest[rest.find('/')?..];
    let mut bytes = Vec::with_capacity(path.len());
    let raw = path.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (raw[i], raw.get(i + 1).copied().and_then(hex), raw.get(i + 2).copied().and_then(hex)) {
            (b'%', Some(h), Some(l)) => {
                bytes.push((h * 16 + l) as u8);
                i += 3;
            }
            (b, _, _) => {
                bytes.push(b);
                i += 1;
            }
        }
    }
    let path = String::from_utf8(bytes).ok()?;
    // `/c:/dir` is a Windows path.
    let windows = path.len() >= 3 && path.as_bytes()[2] == b':' && path.as_bytes()[1].is_ascii_alphabetic();
    Some(if windows { path[1..].to_string() } else { path })
}

/// The `file:` URI of a path.
pub fn path_to_uri(path: &str) -> String {
    let path = path.replace('\\', "/");
    let mut uri = String::from(if path.starts_with('/') { "file://" } else { "file:///" });
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_uris() {
        assert_eq!(uri_to_path("file:///Users/a%20b/m.hutn").as_deref(), Some("/Users/a b/m.hutn"));
        assert_eq!(uri_to_path("file:///c%3A/models/m.xmi").as_deref(), Some("c:/models/m.xmi"));
        assert_eq!(uri_to_path("untitled:Untitled-1"), None);
        assert_eq!(path_to_uri("/Users/a b/m.hutn"), "file:///Users/a%20b/m.hutn");
        assert_eq!(path_to_uri("C:\\m\\x.ocl"), "file:///C%3A/m/x.ocl");
        let p = "/tmp/ü x/ä.ocl";
        assert_eq!(uri_to_path(&path_to_uri(p)).as_deref(), Some(p));
    }

    #[test]
    fn converts_columns() {
        let line = "a😀b";
        assert_eq!(utf16_column(line, 2), 3);
        assert_eq!(scalar_column(line, 3), 2);
        assert_eq!(line_text("x\r\ny", 0), "x");
    }
}
