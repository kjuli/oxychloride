//! The language server, driven over an in-memory connection like an editor would.

use std::time::Duration;

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use oxy_lsp::path_to_uri;
use serde_json::{Value as Json, json};

const EXAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");

struct Client {
    conn: Connection,
    next_id: i32,
    server: Option<std::thread::JoinHandle<()>>,
}

impl Client {
    /// Starts a server with `folder` as the workspace folder.
    fn start(folder: &str) -> Client {
        let (client, server) = Connection::memory();
        let handle = std::thread::spawn(move || oxy_lsp::serve(&server).unwrap());
        let mut c = Client { conn: client, next_id: 0, server: Some(handle) };
        let folders = json!([{ "uri": path_to_uri(folder), "name": "examples" }]);
        c.request("initialize", json!({ "capabilities": {}, "workspaceFolders": folders }));
        c.notify("initialized", json!({}));
        c
    }

    fn notify(&self, method: &str, params: Json) {
        self.conn.sender.send(Message::Notification(Notification::new(method.into(), params))).unwrap();
    }

    /// Sends a request and returns its response, keeping notifications for later.
    fn request(&mut self, method: &str, params: Json) -> Response {
        self.next_id += 1;
        let id = RequestId::from(self.next_id);
        self.conn.sender.send(Message::Request(Request::new(id.clone(), method.into(), params))).unwrap();
        loop {
            match self.conn.receiver.recv_timeout(Duration::from_secs(20)).expect("a response") {
                Message::Response(r) if r.id == id => return r,
                _ => {}
            }
        }
    }

    fn rpc(&mut self, method: &str, params: Json) -> Json {
        let r = self.request("oxychloride/rpc", json!({ "method": method, "params": params }));
        r.result.unwrap_or_else(|| panic!("{method}: {:?}", r.error))
    }

    /// Waits for the diagnostics of `uri` that satisfy `accept`.
    fn diagnostics(&self, uri: &str, accept: impl Fn(&[Json]) -> bool) -> Vec<Json> {
        loop {
            let Message::Notification(n) =
                self.conn.receiver.recv_timeout(Duration::from_secs(20)).expect("diagnostics")
            else {
                continue;
            };
            if n.method == "textDocument/publishDiagnostics" && n.params["uri"] == uri {
                let diagnostics = n.params["diagnostics"].as_array().unwrap().clone();
                if accept(&diagnostics) {
                    return diagnostics;
                }
            }
        }
    }

    fn open(&self, path: &str, language: &str) -> (String, String) {
        let text = std::fs::read_to_string(path).unwrap();
        let uri = path_to_uri(path);
        let doc = json!({ "uri": uri, "languageId": language, "version": 1, "text": text });
        self.notify("textDocument/didOpen", json!({ "textDocument": doc }));
        (uri, text)
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.request("shutdown", Json::Null);
        self.notify("exit", Json::Null);
        if let Some(h) = self.server.take() {
            h.join().unwrap();
        }
    }
}

#[test]
fn checks_models_against_metamodels_of_the_workspace() {
    let folder = format!("{EXAMPLES}/university");
    let mut client = Client::start(&folder);
    // The metamodel is not open: it is loaded from the workspace folder.
    let (uri, text) = client.open(&format!("{folder}/university.hutn"), "hutn");
    let diagnostics = client.diagnostics(&uri, |d| !d.is_empty());
    assert_eq!(diagnostics.len(), 5, "{diagnostics:#?}");
    assert!(diagnostics.iter().all(|d| d["severity"] == 1 && d["source"] == "ocl"));

    // Edits are checked live: fix Carla's credit limit by dropping the thesis enrolment.
    let edited = text
        .replace("          Enrolment { course: Course \"thesis\" }\n", "")
        .replace("Enrolment { course: Course \"algo\" grade: 1 },", "Enrolment { course: Course \"algo\" grade: 1 }");
    let changes = json!([{ "text": edited }]);
    client.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": uri, "version": 2 }, "contentChanges": changes }),
    );
    let diagnostics = client.diagnostics(&uri, |d| d.len() != 5);
    assert!(!diagnostics.iter().any(|d| d["message"].as_str().unwrap().contains("CreditLimit")), "{diagnostics:#?}");

    // Hover and completion, with LSP positions.
    let line = edited.lines().position(|l| l.contains("semester: 3")).unwrap();
    let position = json!({ "line": line, "character": 10 });
    let hover = client.request("textDocument/hover", json!({ "textDocument": { "uri": uri }, "position": position }));
    let contents = hover.result.unwrap()["contents"]["value"].as_str().unwrap().to_string();
    assert!(contents.contains("attribute Student::semester : Integer"), "{contents}");
    let position = json!({ "line": line, "character": 8 });
    let completion =
        client.request("textDocument/completion", json!({ "textDocument": { "uri": uri }, "position": position }));
    let result = completion.result.unwrap();
    assert!(result["items"].as_array().unwrap().iter().any(|i| i["label"] == "semester"), "{result}");

    // The engine's methods: evaluation (the console) and the browsing views.
    let r = client.rpc(
        "evaluate",
        json!({ "expression": "Student.allInstances()->size()", "context": { "resource": format!("{folder}/university.hutn"), "fragment": "ada" } }),
    );
    assert_eq!(r["display"], "3");
    let packages = client.rpc("packages", Json::Null);
    assert_eq!(packages[0]["name"], "university");
    let r = client.request("oxychloride/rpc", json!({ "method": "evaluate", "params": {} }));
    assert!(r.error.is_some());
}

#[test]
fn checks_xmi_documents() {
    let folder = format!("{EXAMPLES}/library");
    let client = Client::start(&folder);
    let (uri, _) = client.open(&format!("{folder}/library.xmi"), "xml");
    // Violations of the embedded constraints and of library.ocl (loaded from the folder).
    let diagnostics = client.diagnostics(&uri, |d| !d.is_empty());
    assert_eq!(diagnostics.len(), 3, "{diagnostics:#?}");
}

#[test]
fn transforms_models_with_henshin_documents() {
    let folder = format!("{EXAMPLES}/philosophers");
    let mut client = Client::start(&folder);
    let model = format!("{folder}/philosophers.hutn");
    let rules = format!("{folder}/philosophers.henshin_text");
    // The rules are loaded from the folder; open, they are checked like the other documents.
    let (uri, text) = client.open(&rules, "henshin");
    let diagnostics = client.diagnostics(&uri, |_| true);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    let line = text.lines().position(|l| l.contains("node fork:Fork")).unwrap();
    let position = json!({ "line": line, "character": 17 });
    let hover = client.request("textDocument/hover", json!({ "textDocument": { "uri": uri }, "position": position }));
    let contents = hover.result.unwrap()["contents"]["value"].as_str().unwrap().to_string();
    assert!(contents.contains("class philosophers::Fork"), "{contents}");

    client.open(&model, "hutn");
    let state = client.rpc("startTransformation", json!({ "model": model }));
    let session = state["session"].clone();
    let applied =
        client.rpc("applyTransformation", json!({ "session": session, "document": rules, "unit": "takeLeft" }));
    assert_eq!(applied["applied"], true, "{applied}");
    assert_eq!(applied["state"]["changes"]["createdEdges"][0]["feature"], "holds");
    let space = client.rpc(
        "exploreStateSpace",
        json!({ "session": session, "document": rules, "rules": ["takeLeft", "takeRight", "release"], "step": 0 }),
    );
    assert_eq!(space["states"].as_array().unwrap().len(), 14);
}
