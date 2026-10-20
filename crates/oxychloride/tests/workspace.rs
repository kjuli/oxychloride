use oxychloride::rpc::Server;
use oxychloride::*;
use serde_json::json;

const EXAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/library");

fn library() -> Workspace {
    let mut ws = Workspace::new();
    // Loading the model pulls in the metamodel through xsi:schemaLocation.
    let r = ws.load_model_file(format!("{EXAMPLES}/library.xmi")).unwrap();
    assert_eq!(r.objects, Some(12));
    let doc = format!("{EXAMPLES}/library.ocl");
    let text = std::fs::read_to_string(&doc).unwrap();
    let diags = ws.open_document(&doc, &text);
    assert!(diags.is_empty(), "{diags:?}");
    ws
}

#[test]
fn validates_with_and_without_ecore_constraints() {
    let ws = library();
    let report = ws.validate(&ValidateOptions::default()).unwrap();
    assert_eq!(report.summary.violated, 3);
    assert_eq!(report.summary.errors, 0);
    // 8 invariants, and the postcondition of Book::isAvailable() (checked on every book).
    assert_eq!(report.constraints.len(), 9);
    let post = &report.constraints[8];
    assert_eq!(
        (post.kind, post.operation.as_deref()),
        (ConstraintKind::Postcondition, Some("isAvailable() : Boolean"))
    );
    assert_eq!(report.summary.evaluations, 23 + 3);
    assert!(!report.is_ok());

    let options = ValidateOptions { contracts: false, ..Default::default() };
    assert_eq!(ws.validate(&options).unwrap().constraints.len(), 8);

    let options = ValidateOptions { ecore: false, ..Default::default() };
    let report = ws.validate(&options).unwrap();
    assert_eq!(report.summary.violated, 2);
    assert_eq!(report.constraints.len(), 7);

    let options = ValidateOptions {
        objects: Some(vec![ObjectRef { resource: Some("library.xmi".into()), fragment: "//@members.1".into() }]),
        include_satisfied: true,
        ..Default::default()
    };
    let report = ws.validate(&options).unwrap();
    let statuses: Vec<_> =
        report.results.iter().map(|r| (report.constraints[r.constraint].name.as_str(), r.status)).collect();
    assert_eq!(
        statuses,
        vec![
            ("AtMostThreeLoans", ResultStatus::Satisfied),
            ("Adult", ResultStatus::Violated),
            ("NoDuplicateLoans", ResultStatus::Satisfied)
        ]
    );
}

#[test]
fn evaluates_with_context_and_definitions() {
    let ws = library();
    let ctx = ObjectRef { resource: None, fragment: "//@books.1".into() };
    let r = ws.evaluate("borrowers->collect(name)->sortedBy(n | n)", Some(&ctx)).unwrap();
    assert_eq!(r.display, "Sequence{'Alice', 'Bob'}");
    assert_eq!(r.ty, "Sequence(String)");
    assert_eq!(r.value, json!({"kind": "Sequence", "items": ["Alice", "Bob"]}));

    let r = ws.evaluate("self", Some(&ctx)).unwrap();
    assert_eq!(r.value["object"]["label"], "Book 'A Brief History of Time'");
    assert_eq!(r.value["object"]["class"], "library::Book");

    let r = ws.evaluate("copies.foo", Some(&ctx)).unwrap();
    assert!(r.invalid);
    assert_eq!(r.diagnostics[0].message, "unknown property 'foo' for type Integer");
    assert_eq!(r.diagnostics[0].range.start, Position { line: 0, column: 7 });

    let missing = ObjectRef { resource: None, fragment: "//@books.9".into() };
    assert!(matches!(ws.evaluate("self", Some(&missing)), Err(Error::NotFound(_))));
}

#[test]
fn reports_import_and_ecore_problems() {
    let mut ws = Workspace::new();
    let diags = ws.open_document("/nowhere/x.ocl", "import 'missing.ecore'\ncontext X inv: true");
    let messages: Vec<_> = diags.iter().map(|d| d.message.as_str()).collect();
    assert_eq!(messages, vec!["cannot find imported metamodel 'missing.ecore'", "unknown type 'X'"]);

    let ecore = std::fs::read_to_string(format!("{EXAMPLES}/library.ecore"))
        .unwrap()
        .replace("loans->size() &lt;= 3", "loans->siz() &lt;= 3");
    ws.load_ecore_str(&ecore, "broken.ecore").unwrap();
    let diags = ws.diagnostics(None);
    let ecore_diag = diags.iter().find(|d| matches!(d.origin, Origin::Ecore { .. })).unwrap();
    assert_eq!(ecore_diag.message, "unknown operation 'siz' for type OrderedSet(Loan)");
    assert_eq!(ecore_diag.source, "broken.ecore: library::Member::AtMostThreeLoans");
}

#[test]
fn hover_and_completion() {
    let mut ws = library();
    let text = "import 'library.ecore'\npackage library\ncontext Book\ninv: library.books->select(b | b.cop";
    let name = format!("{EXAMPLES}/scratch.ocl");
    ws.open_document(&name, text);

    // Hover over `library` (line 3, column 6).
    let hover = ws.hover(&name, Position { line: 3, column: 6 }).unwrap();
    assert_eq!(hover.contents, "Library");

    let labels = |list: CompletionList| list.items.into_iter().map(|i| i.label).collect::<Vec<_>>();
    let end = Position { line: 3, column: 36 };
    assert_eq!(labels(ws.complete(&name, end).unwrap()), vec!["copies"]);

    let text = "package library\ncontext Book\ninv: library.books->";
    ws.open_document(&name, text);
    let items = labels(ws.complete(&name, Position { line: 2, column: 20 }).unwrap());
    assert!(items.contains(&"forAll".to_string()) && items.contains(&"size".to_string()), "{items:?}");

    let text = "package library\ncontext Book\ninv: library.";
    ws.open_document(&name, text);
    let items = labels(ws.complete(&name, Position { line: 2, column: 13 }).unwrap());
    assert!(items.contains(&"books".to_string()) && items.contains(&"members".to_string()), "{items:?}");

    let text = "package library\ncontext Book\ninv: genre = Genre::";
    ws.open_document(&name, text);
    let items = labels(ws.complete(&name, Position { line: 2, column: 20 }).unwrap());
    assert_eq!(items, vec!["FICTION", "SCIENCE", "HISTORY"]);

    let text = "package library\ncontext Book\ninv: is";
    ws.open_document(&name, text);
    let items = labels(ws.complete(&name, Position { line: 2, column: 7 }).unwrap());
    assert!(items.contains(&"isbn".to_string()) && items.contains(&"isAvailable".to_string()), "{items:?}");
}

#[test]
fn json_rpc_session() {
    let mut server = Server::new();
    let mut call = |method: &str, params: serde_json::Value| -> serde_json::Value {
        let req = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        server.handle(req).unwrap()
    };
    let r = call("initialize", json!(null));
    assert_eq!(r["result"]["protocolVersion"], 1);
    let r = call("loadEcore", json!({ "path": format!("{EXAMPLES}/library.ecore") }));
    assert_eq!(r["result"]["packages"][0], "http://www.example.org/library/1.0");
    let r = call("loadModel", json!({ "path": format!("{EXAMPLES}/library.xmi") }));
    assert_eq!(r["result"]["objects"], 12);
    let r = call("openDocument", json!({ "name": "rules.ocl", "text": "context library::Book inv Many: copies > 1" }));
    assert_eq!(r["result"]["diagnostics"], json!([]));
    let r = call("validate", json!({ "ecore": false }));
    assert_eq!(r["result"]["summary"]["violated"], 2);
    assert_eq!(r["result"]["results"][0]["object"]["label"], "Book 'A Brief History of Time'");
    let r = call("evaluate", json!({ "expression": "Member.allInstances()->size()" }));
    assert_eq!(r["result"]["display"], "4");
    let r = call("packages", json!({}));
    assert_eq!(r["result"][0]["classifiers"][1]["name"], "Book");
    let r = call("models", json!({}));
    assert_eq!(r["result"][0]["roots"][0]["label"], "Library 'City Library'");
    let r = call("loadModel", json!({ "path": "/does/not/exist.xmi" }));
    assert_eq!(r["error"]["code"], -32000);
    let r = call("evaluate", json!({}));
    assert_eq!(r["error"]["code"], -32602);
    let r = call("frobnicate", json!({}));
    assert_eq!(r["error"]["code"], -32601);

    assert!(server.handle_str("{nope").unwrap().contains("-32700"));
    // Notifications get no response.
    assert!(server.handle(json!({ "jsonrpc": "2.0", "method": "reset" })).is_none());
    let r = server.handle(json!({ "jsonrpc": "2.0", "id": 2, "method": "models" })).unwrap();
    assert_eq!(r["result"], json!([]));
}

#[test]
fn reload_replays_inputs() {
    let mut ws = library();
    let problems = ws.reload();
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(ws.model().len(), 12);
    assert_eq!(ws.document_names().count(), 1);
    assert_eq!(ws.validate(&ValidateOptions::default()).unwrap().summary.violated, 3);
}

#[test]
fn cross_resource_references_ids_and_uri_mappings() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/net");
    let mut ws = Workspace::new();
    // a.xmi references b.xmi (which references a.xmi back) and a pathmap resource.
    let r = ws.load_model_file(format!("{dir}/a.xmi")).unwrap();
    let messages: Vec<_> = r.warnings.iter().map(|w| w.message.as_str()).collect();
    assert_eq!(
        messages,
        vec!["1 reference into 'pathmap://LIB/lib.xmi' cannot be resolved (the resource is not loaded)"]
    );
    let eval = |ws: &Workspace, e: &str| {
        ws.evaluate(e, Some(&ObjectRef { resource: None, fragment: "A1".into() })).unwrap().display
    };
    assert_eq!(eval(&ws, "next.name"), "Sequence{'a2', 'b1'}");
    assert_eq!(eval(&ws, "next->any(name = 'b1').next.name"), "Sequence{'a1'}");

    ws.map_uri("pathmap://LIB/", &format!("{dir}/lib"));
    assert_eq!(eval(&ws, "next.name"), "Sequence{'a2', 'b1', 'lib'}");
    assert_eq!(ws.model().resources().len(), 3);
    assert!(ws.model().unresolved_references().is_empty());
}
