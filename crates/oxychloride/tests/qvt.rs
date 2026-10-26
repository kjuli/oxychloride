//! QVT transformations in the workspace: diagnostics, hovers and completion of `.qvto` and
//! `.qvtr` documents, finding the transformations a model fits, and running them
//! (`examples/qvt`).

use oxychloride::rpc::Server;
use oxychloride::*;
use serde_json::json;

const EXAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/qvt");
const REGISTRY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/registry");

fn path(file: &str) -> String {
    format!("{EXAMPLES}/{file}")
}

fn open(ws: &mut Workspace, file: &str) -> Vec<Diagnostic> {
    let p = path(file);
    let text = std::fs::read_to_string(&p).unwrap();
    ws.open_document(&p, &text)
}

/// The example: both metamodels, the model, and both transformations.
fn workspace() -> Workspace {
    let mut ws = Workspace::new();
    for f in ["uml.oclinecore", "rdbms.oclinecore", "company.hutn", "uml2rdbms.qvto", "uml2rdbms.qvtr"] {
        let d = open(&mut ws, f);
        assert!(d.iter().all(|d| d.severity != Severity::Error), "{f}: {d:#?}");
    }
    ws
}

fn request(document: &str) -> QvtRunRequest {
    QvtRunRequest {
        document: path(document),
        models: vec![QvtModelBinding { parameter: "source".into(), model: path("company.hutn") }],
        outputs: vec![QvtOutputPath { parameter: "target".into(), path: path("company.xmi") }],
        ..QvtRunRequest::default()
    }
}

#[test]
fn transformations_that_fit_a_model() {
    let ws = workspace();
    let all = ws.qvt_transformations(Some(&path("company.hutn"))).unwrap();
    assert_eq!(all.len(), 2);
    let qvto = all.iter().find(|t| t.language == Language::Qvto).unwrap();
    assert_eq!(qvto.name, "uml2rdbms");
    assert_eq!(qvto.applicable, vec!["source"]);
    assert_eq!(qvto.targets, vec!["target"]);
    assert_eq!(qvto.errors, 0);
    assert_eq!(qvto.parameters[0].direction.as_deref(), Some("in"));
    assert_eq!(qvto.parameters[0].metamodels, vec!["http://www.example.org/qvt/uml"]);
    let qvtr = all.iter().find(|t| t.language == Language::Qvtr).unwrap();
    assert_eq!(qvtr.applicable, vec!["source"]);
    assert_eq!(qvtr.parameters[1].direction, None);
    assert_eq!(qvtr.targets, vec!["target"]);
}

#[test]
fn running_both_languages_gives_the_same_schema() {
    let ws = workspace();
    let qvto = ws.run_qvt(&request("uml2rdbms.qvto")).unwrap();
    assert!(qvto.problems.is_empty(), "{:#?}", qvto.problems);
    assert!(qvto.invariants.is_empty(), "{:#?}", qvto.invariants);
    assert_eq!(qvto.outputs.len(), 1);
    let out = &qvto.outputs[0];
    assert_eq!((out.parameter.as_str(), out.format.as_str(), out.objects), ("target", "xmi", 13));
    assert!(out.text.contains("<rdbms:Schema"), "{}", out.text);
    assert!(out.text.contains("xsi:schemaLocation") || !out.text.contains(".ecore"), "{}", out.text);

    let qvtr = ws.run_qvt(&request("uml2rdbms.qvtr")).unwrap();
    assert!(qvtr.problems.is_empty(), "{:#?}", qvtr.problems);
    assert_eq!(qvtr.outputs[0].text, out.text);

    // The workspace is not changed.
    assert!(ws.models().iter().all(|m| !m.uri.ends_with("company.xmi")));
}

#[test]
fn outputs_as_hutn_and_checking_an_existing_model() {
    let mut ws = workspace();
    let mut r = request("uml2rdbms.qvto");
    r.format = Some("hutn".into());
    let result = ws.run_qvt(&r).unwrap();
    let schema = &result.outputs[0];
    assert_eq!(schema.format, "hutn");
    assert!(schema.text.contains("Schema {"), "{}", schema.text);

    // The produced schema satisfies the relations; one without Company's table does not.
    ws.open_document(&path("schema.hutn"), &schema.text);
    let check = |ws: &Workspace| {
        let mut r = request("uml2rdbms.qvtr");
        r.models.push(QvtModelBinding { parameter: "target".into(), model: path("schema.hutn") });
        r.check = true;
        ws.run_qvt(&r).unwrap()
    };
    let ok = check(&ws);
    assert!(ok.violations.is_empty() && ok.outputs.is_empty(), "{ok:#?}");
    let without_company = schema.text.replacen("name: \"Company\"", "name: \"Firm\"", 1);
    ws.open_document(&path("schema.hutn"), &without_company);
    let bad = check(&ws);
    assert_eq!(bad.violations, vec!["relation ClassToTable does not hold for c = Class 'Company'"]);
}

#[test]
fn problems_of_runs_and_documents() {
    let mut ws = workspace();
    // A transformation with errors does not run.
    let broken =
        "modeltype UML uses 'http://www.example.org/qvt/uml';\ntransformation t(in s : UML);\nmain() { s.nothing(); }";
    let d = ws.open_document(&path("broken.qvto"), broken);
    assert!(d.iter().any(|d| d.message.contains("unknown operation 'nothing'") && d.range.start.line == 2), "{d:#?}");
    let mut r = request("broken.qvto");
    r.models[0].parameter = "s".into();
    let e = ws.run_qvt(&r).unwrap_err().to_string();
    assert!(e.contains("line 3"), "{e}");

    // Problems while running are located in the transformation.
    let failing = "modeltype UML uses 'http://www.example.org/qvt/uml';\ntransformation t(in s : UML);\nmain() {\n  s.objectsOfKind(UML::Class)->forEach(c) { c.name := 'x'; };\n  log('done');\n}";
    ws.open_document(&path("failing.qvto"), failing);
    let mut r = request("failing.qvto");
    r.models[0].parameter = "s".into();
    r.outputs.clear();
    let result = ws.run_qvt(&r).unwrap();
    assert_eq!(result.log, vec!["done"]);
    assert_eq!(result.problems.len(), 1, "{:#?}", result.problems);
    assert!(result.problems[0].message.contains("cannot be changed"));
    assert_eq!(result.problems[0].range.start.line, 3);

    // A missing input.
    let mut r = request("uml2rdbms.qvto");
    r.models.clear();
    assert!(ws.run_qvt(&r).unwrap_err().to_string().contains("no model is given for the parameter 'source'"));
}

#[test]
fn imported_libraries() {
    // `uml2rdbms.qvto` imports `sql.Types` from the file next to it (`workspace` checks that it
    // has no errors); libraries are no transformations to run.
    let mut ws = workspace();
    let library = path("sql/Types.qvto");
    let d = open(&mut ws, "sql/Types.qvto");
    assert!(d.iter().all(|d| d.severity != Severity::Error), "{d:#?}");
    let names: Vec<String> = ws.qvt_transformations(None).unwrap().into_iter().map(|t| t.name).collect();
    assert_eq!(names, vec!["uml2rdbms", "uml2rdbms"]);
    let e = ws.run_qvt(&QvtRunRequest { document: library.clone(), ..QvtRunRequest::default() }).unwrap_err();
    assert!(e.to_string().contains("is a library"), "{e}");

    // The open document is imported, not the file: its errors are reported at the import.
    let text = std::fs::read_to_string(&library).unwrap();
    ws.open_document(&library, &text.replace("'NUMBER';", "3;"));
    let d = ws.diagnostics(Some(&path("uml2rdbms.qvto")));
    let error = d.iter().find(|d| d.severity == Severity::Error).expect("an error at the import");
    assert!(
        error.message.contains("the imported unit has errors") && error.message.contains("Types.qvto:20"),
        "{d:#?}"
    );
    assert_eq!(error.range.start.line, 9);

    // Problems while running the library's code are located in the library.
    ws.open_document(&library, &text.replace("className + '_tid'", "className.substring(5, 1)"));
    let result = ws.run_qvt(&request("uml2rdbms.qvto")).unwrap();
    let p = result.problems.iter().find(|p| p.source == library).expect("a problem in the library");
    assert_eq!(p.range.start.line, 18, "{p:#?}");

    // Hover and completion know the library's operations.
    let qvto = path("uml2rdbms.qvto");
    let source = std::fs::read_to_string(&qvto).unwrap();
    let offset = source.find("idColumn(self.name)").unwrap() + 2;
    let line = source[..offset].matches('\n').count() as u32;
    let column = source[..offset].rsplit('\n').next().unwrap().chars().count() as u32;
    let hover = ws.hover(&qvto, Position { line, column }).unwrap();
    assert!(hover.contents.contains("helper idColumn(className : String) : Column"), "{}", hover.contents);
}

#[test]
fn model_types_are_found_through_the_package_registry() {
    let mut ws = Workspace::new();
    ws.set_registry(vec![RegistryEntry { uri: "urn:shop".into(), location: format!("{REGISTRY}/lib/shop.ecore") }]);
    let text = "modeltype SHOP uses 'urn:shop';\ntransformation t(in a : SHOP, out b : SHOP);\nmain() { a.rootObjects()[SHOP::Item]->map copy(); }\nmapping SHOP::Item::copy() : SHOP::Item { name := self.name; price := self.price * 2; }";
    let d = ws.open_document("/tmp/shop.qvto", text);
    assert!(d.is_empty(), "{d:#?}");
    // A metamodel file next to the document, by its relative path (QVTr's usual imports).
    let text = format!(
        "import shop : '{REGISTRY}/lib/shop.ecore';\ntransformation t(a : shop, b : shop) {{\n  top relation R {{ n : String; checkonly domain a x : shop::Item {{ name = n }}; enforce domain b y : shop::Item {{ name = n }}; }}\n}}"
    );
    let d = ws.open_document("/tmp/shop.qvtr", &text);
    assert!(d.is_empty(), "{d:#?}");
}

#[test]
fn hovers_and_completion() {
    let ws = workspace();
    let name = path("uml2rdbms.qvto");
    let text = ws.document_text(&name).unwrap().to_string();
    let at = |needle: &str, delta: usize| {
        let offset = text.find(needle).unwrap() + delta;
        let line = text[..offset].matches('\n').count() as u32;
        let column = text[..offset].rsplit('\n').next().unwrap().chars().count() as u32;
        Position { line, column }
    };
    let hover = ws.hover(&name, at("mapping UML::Class::classToTable", 22)).unwrap();
    assert_eq!(hover.contents, "mapping Class::classToTable() : Table");
    let hover = ws.hover(&name, at("map classToTable", 6)).unwrap();
    assert_eq!(hover.contents, "mapping Class::classToTable() : Table");
    let hover = ws.hover(&name, at("self.persistent", 7)).unwrap();
    assert_eq!(hover.contents, "Boolean");

    let completion = ws.complete(&name, at("name := self.name;\n\ttables", 13)).unwrap();
    let labels: Vec<&str> = completion.items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"elements") && labels.contains(&"name"), "{labels:?}");
    let completion = ws.complete(&name, at("->map packageToSchema", 6)).unwrap();
    let labels: Vec<&str> = completion.items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"packageToSchema") && labels.contains(&"classToTable"), "{labels:?}");
}

#[test]
fn json_rpc() {
    let mut server = Server::new();
    for f in ["uml.oclinecore", "rdbms.oclinecore", "company.hutn", "uml2rdbms.qvtr"] {
        let p = path(f);
        let text = std::fs::read_to_string(&p).unwrap();
        server.call("openDocument", json!({ "name": p, "text": text })).unwrap();
    }
    let list = server.call("qvtTransformations", json!({ "model": path("company.hutn") })).unwrap();
    assert_eq!(list["transformations"][0]["language"], "qvtr");
    assert_eq!(list["transformations"][0]["applicable"], json!(["source"]));
    let result = server
        .call(
            "runQvt",
            json!({
                "document": path("uml2rdbms.qvtr"),
                "models": [{ "parameter": "source", "model": path("company.hutn") }],
                "outputs": [{ "parameter": "target", "path": path("company.xmi") }],
            }),
        )
        .unwrap();
    assert_eq!(result["outputs"][0]["objects"], 13);
    // PackageToSchema, ClassToTable twice, AttributeToColumn four times, AttributeToForeignKey.
    assert_eq!(result["applications"], 8);
}
