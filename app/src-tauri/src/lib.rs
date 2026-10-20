//! Tauri backend of the Oxychloride editor: a thin bridge from the web UI to
//! [`oxychloride::rpc::Server`], plus plain file I/O for the editor tabs.

use std::sync::Mutex;

use oxychloride::rpc::Server;
use serde_json::{Value, json};

/// The engine, shared by all commands.
struct Engine(Mutex<Server>);

/// Forwards a JSON-RPC method call to the engine. Errors are returned as
/// `{ "code": <i64>, "message": <string> }`.
///
/// The commands are `async` so that they run on Tauri's worker pool rather than the main
/// (UI) thread; the engine itself is synchronous and guarded by a mutex.
#[tauri::command]
async fn rpc(state: tauri::State<'_, Engine>, method: String, params: Value) -> Result<Value, Value> {
    let mut server = state.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    server.call(&method, params).map_err(|e| json!({ "code": e.code, "message": e.message }))
}

#[tauri::command]
async fn read_text_file(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| format!("cannot read {path}: {e}"))
}

#[tauri::command]
async fn write_text_file(path: String, contents: String) -> Result<(), String> {
    std::fs::write(&path, contents).map_err(|e| format!("cannot write {path}: {e}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Engine(Mutex::new(Server::new())))
        .invoke_handler(tauri::generate_handler![rpc, read_text_file, write_text_file])
        .run(tauri::generate_context!())
        .expect("error while running the Oxychloride editor");
}

#[cfg(test)]
mod tests {
    //! Checks the JSON shapes the web UI relies on (see `app/src/api.ts`) against the
    //! library example.

    use super::*;

    fn example(file: &str) -> String {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/library");
        dir.join(file).canonicalize().unwrap().to_string_lossy().into_owned()
    }

    #[test]
    fn editor_flow_over_rpc() {
        let mut s = Server::new();
        let ocl_path = example("library.ocl");
        let text = std::fs::read_to_string(&ocl_path).unwrap();

        // Opening the document resolves `import 'library.ecore'` relative to its path.
        let opened = s.call("openDocument", json!({ "name": ocl_path, "text": text })).unwrap();
        assert!(opened["diagnostics"].is_array(), "{opened}");
        let packages = s.call("packages", Value::Null).unwrap();
        assert!(packages.as_array().is_some_and(|p| !p.is_empty()), "import did not load the metamodel");
        assert!(packages[0]["classifiers"].is_array());
        assert!(packages[0]["nsUri"].is_string());

        let load = s.call("loadModel", json!({ "path": example("library.xmi") })).unwrap();
        assert!(load["uri"].is_string() && load["warnings"].is_array(), "{load}");
        let models = s.call("models", Value::Null).unwrap();
        let root = &models[0]["roots"][0];
        assert!(root["fragment"].is_string() && root["children"].is_array() && root["attributes"].is_array());

        let report = s.call("validate", json!({ "ecore": true, "includeSatisfied": true })).unwrap();
        assert!(report["summary"]["evaluations"].as_u64().unwrap() > 0, "{report}");
        let first = &report["results"][0];
        assert!(first["constraint"].is_u64() && first["object"]["label"].is_string());
        assert!(["satisfied", "violated", "error"].contains(&first["status"].as_str().unwrap()));
        let constraints = report["constraints"].as_array().unwrap();
        assert!(
            constraints.iter().any(|c| c["origin"]["kind"] == "document" && c["origin"]["name"] == ocl_path.as_str()),
            "document constraints are named by the document path"
        );

        let context = json!({ "resource": models[0]["uri"], "fragment": root["fragment"] });
        let eval = s.call("evaluate", json!({ "expression": "self.books->size()", "context": context })).unwrap();
        assert!(eval["display"].is_string() && eval["type"].is_string() && eval["diagnostics"].is_array(), "{eval}");

        // `context Book` at line 13 (0-based 12): hover on `Book`, complete after `self.`.
        let hover = s.call("hover", json!({ "name": ocl_path, "position": { "line": 12, "column": 9 } })).unwrap();
        assert!(hover.is_null() || hover["contents"].is_string(), "{hover}");
        let edited = text.replace("inv NonNegativeCopies: copies >= 0", "inv NonNegativeCopies: self.");
        let line = edited.lines().position(|l| l.ends_with("self.")).unwrap() as u32;
        let column = edited.lines().nth(line as usize).unwrap().chars().count() as u32;
        let completions = s
            .call(
                "complete",
                json!({ "name": ocl_path, "text": edited, "position": { "line": line, "column": column } }),
            )
            .unwrap();
        let items = completions["items"].as_array().expect("completion items");
        assert!(items.iter().any(|i| i["label"] == "copies"), "{completions}");
        assert!(completions["range"]["start"]["line"].is_u64());

        let all = s.call("diagnostics", json!({})).unwrap();
        assert!(all["diagnostics"].is_array());
        assert_eq!(s.call("closeDocument", json!({ "name": ocl_path })).unwrap()["closed"], true);
        let err = s.call("loadModel", json!({ "path": "/nonexistent.xmi" })).unwrap_err();
        assert!(!err.message.is_empty());
    }

    fn university(file: &str) -> String {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/university");
        dir.join(file).canonicalize().unwrap().to_string_lossy().into_owned()
    }

    #[test]
    fn textual_metamodels_and_models_over_rpc() {
        let mut s = Server::new();
        let metamodel = university("university.oclinecore");
        let model = university("university.hutn");
        let text = |p: &str| std::fs::read_to_string(p).unwrap();

        let opened = s
            .call("openDocument", json!({ "name": metamodel, "text": text(&metamodel), "language": "oclinecore" }))
            .unwrap();
        assert_eq!(opened["language"], "oclinecore");
        let opened = s.call("openDocument", json!({ "name": model, "text": text(&model) })).unwrap();
        assert_eq!(opened["language"], "hutn", "the language follows the extension");
        let first = &opened["diagnostics"][0];
        assert!(first["origin"]["kind"] == "document" && first["range"]["start"]["line"].is_u64(), "{opened}");

        let packages = s.call("packages", Value::Null).unwrap();
        let course =
            &packages[0]["classifiers"].as_array().unwrap().iter().find(|c| c["name"] == "Course").unwrap().clone();
        let has_seat =
            course["operations"].as_array().unwrap().iter().find(|o| o["name"] == "hasSeat").unwrap().clone();
        assert!(has_seat["postconditions"][0]["expression"].is_string(), "{has_seat}");
        let models = s.call("models", Value::Null).unwrap();
        assert!(models[0]["roots"][0]["range"]["start"]["line"].is_u64(), "objects of HUTN documents have ranges");

        let report = s.call("validate", json!({ "ecore": true, "contracts": true })).unwrap();
        let constraints = report["constraints"].as_array().unwrap();
        assert!(
            constraints
                .iter()
                .all(|c| ["invariant", "precondition", "postcondition"].contains(&c["kind"].as_str().unwrap()))
        );
        let post = report["results"].as_array().unwrap().iter().find(|r| r["call"].is_string()).unwrap();
        assert_eq!(constraints[post["constraint"].as_u64().unwrap() as usize]["operation"], "hasSeat() : Boolean");
        assert!(post["object"]["range"].is_object());

        let context = json!({ "resource": model, "fragment": "ada" });
        let call = "self.mayEnrol(Course.allInstances()->any(code = 'IN0001'))";
        let eval = s.call("evaluate", json!({ "expression": call, "context": context })).unwrap();
        let contract = &eval["contracts"][0];
        assert!(contract["constraint"]["kind"] == "precondition" && contract["call"].is_string(), "{eval}");

        let exported = s.call("exportDocument", json!({ "name": model, "target": "/tmp/university.xmi" })).unwrap();
        assert_eq!(exported["format"], "xmi");
        assert!(exported["text"].as_str().unwrap().starts_with("<?xml"));
        let converted = s.call("convertFile", json!({ "path": example("library.ecore"), "to": "oclinecore" })).unwrap();
        assert!(converted["text"].as_str().unwrap().starts_with("package library"));
        let completions =
            s.call("complete", json!({ "name": model, "position": { "line": 52, "column": 8 } })).unwrap();
        assert!(
            completions["items"].as_array().is_some_and(|i| i.iter().any(|i| i["kind"] == "property")),
            "{completions}"
        );

        // The sidebar's context menu: containment options and new objects.
        let tum = models[0]["roots"][0]["fragment"].clone();
        let options =
            s.call("containmentOptions", json!({ "object": { "resource": model, "fragment": tum } })).unwrap();
        assert!(options[0]["classes"][0] == "university::Course" && options[0]["full"] == false, "{options}");
        let container = json!({ "fragment": tum, "feature": "courses" });
        let edit = s
            .call("createObject", json!({ "name": model, "class": "university::Course", "container": container }))
            .unwrap();
        assert!(edit["edits"][0]["range"]["start"]["line"].is_u64() && edit["edits"][0]["text"].is_string(), "{edit}");
        assert!(edit["cursor"]["column"].is_u64(), "{edit}");
    }
}
