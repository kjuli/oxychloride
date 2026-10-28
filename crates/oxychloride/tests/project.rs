//! Modeling projects: manifests, the documents of a project, new projects and new documents.

use std::path::{Path, PathBuf};

use oxychloride::project::{ProjectFolders, document_language, identifier};
use oxychloride::rpc::Server;
use oxychloride::*;
use serde_json::json;

const EXAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");

/// A new, empty folder for a test.
fn folder(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("projects").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Writes the files of a new project into `dir`.
fn write(dir: &Path, project: &NewProject) {
    for f in &project.files {
        let path = dir.join(&f.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, &f.text).unwrap();
    }
}

fn paths(project: &NewProject) -> Vec<&str> {
    project.files.iter().map(|f| f.path.as_str()).collect()
}

fn errors(diagnostics: &[Diagnostic]) -> Vec<String> {
    diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| format!("{}:{}: {}", d.source, d.range.start.line + 1, d.message))
        .collect()
}

/// Writes a new project, opens it in a workspace and checks that nothing has errors.
fn create(name: &str, request: NewProjectRequest) -> (PathBuf, NewProject, Workspace, ProjectLoad) {
    let created = Workspace::new_project(&request).unwrap();
    let dir = folder(name);
    write(&dir, &created);
    let project = Project::load(&dir).unwrap();
    let mut ws = Workspace::new();
    let load = ws.open_project(&project);
    assert_eq!(load.problems, vec![], "{name}");
    assert_eq!(errors(&ws.diagnostics(None)), Vec::<String>::new(), "{name}");
    (dir, created, ws, load)
}

const ALL: [Language; 7] = [
    Language::Ocl,
    Language::Hutn,
    Language::Xmi,
    Language::Henshin,
    Language::HenshinModel,
    Language::Qvto,
    Language::Qvtr,
];

#[test]
fn new_projects_have_a_manifest_a_metamodel_and_documents_of_it() {
    let request = NewProjectRequest { name: "My Shop".into(), codegen: Some("python".into()), ..Default::default() };
    let (dir, created, ws, load) = create("shop", request);
    assert_eq!(created.package, "my_shop");
    assert_eq!(created.ns_uri, "http://example.org/my_shop");
    assert_eq!(
        paths(&created),
        vec![
            "oxychloride.json",
            "metamodels/my_shop.oclinecore",
            "constraints/my_shop.ocl",
            "models/example.hutn",
            "models/example.xmi",
            "transformations/copy.qvto",
            "transformations/copy.qvtr",
            "transformations/my_shop.henshin",
            "transformations/my_shop.henshin_text",
            "README.md",
            ".vscode/extensions.json",
        ]
    );
    let project = Project::load(dir.join(MANIFEST)).unwrap();
    assert_eq!(project.manifest.name, "My Shop");
    assert_eq!(project.manifest.folders, ProjectFolders::default());
    assert_eq!(project.manifest.codegen.as_ref().map(|c| c.target.as_str()), Some("python"));
    assert_eq!(project.code_folder(), dir.join("src-gen"));
    assert_eq!(load.documents.len(), 8);
    // Metamodels are opened first: the others use them.
    assert!(load.documents[0].ends_with("my_shop.oclinecore"));

    // Both models are loaded, with the same objects, and satisfy every constraint.
    let models = ws.models();
    assert_eq!(models.len(), 2);
    let report = ws.validate(&ValidateOptions::default()).unwrap();
    assert!(report.is_ok(), "{report:#?}");
    assert_eq!(report.summary.objects, 6);
    // The Complete OCL invariant is among the checked constraints.
    assert_eq!(report.summary.constraints, 3);

    let readme = &created.files.iter().find(|f| f.path == "README.md").unwrap().text;
    assert!(readme.starts_with("# My Shop\n"), "{readme}");
    assert!(readme.contains("oxy generate metamodels/my_shop.oclinecore -t python -o src-gen"), "{readme}");
}

/// `examples/shop` is the project `oxy new` creates; after changing the templates, regenerate it with
/// `oxy new examples/shop --name "My Shop" --description "<its description>" --codegen python --force`.
#[test]
fn the_example_project_is_what_new_projects_are() {
    let dir = Path::new(EXAMPLES).join("shop");
    let project = Project::load(&dir).unwrap();
    let request = NewProjectRequest {
        name: project.manifest.name.clone(),
        description: Some(project.manifest.description.clone()),
        codegen: Some("python".into()),
        ..Default::default()
    };
    for f in Workspace::new_project(&request).unwrap().files {
        let text = std::fs::read_to_string(dir.join(&f.path)).unwrap_or_default();
        assert!(text == f.text, "examples/shop/{} differs from the template, regenerate it", f.path);
    }
    let mut ws = Workspace::new();
    let load = ws.open_project(&project);
    assert_eq!((load.documents.len(), load.problems), (8, vec![]));
    assert_eq!(errors(&ws.diagnostics(None)), Vec::<String>::new());
}

#[test]
fn the_models_of_a_project_are_checked_against_all_constraints() {
    let (dir, _, _, _) = create("cyclic", NewProjectRequest { name: "cyclic".into(), ..Default::default() });
    let model = dir.join("models/example.hutn");
    let text = std::fs::read_to_string(&model).unwrap();
    let text = text.replace(
        r#"Element "first" { name: "first" }"#,
        r#"Element "first" { name: "first" dependencies: Element "second" }"#,
    );
    std::fs::write(&model, text).unwrap();
    let mut ws = Workspace::new();
    ws.open_project(&Project::load(&dir).unwrap());
    let errors = errors(&ws.diagnostics(None));
    assert_eq!(errors.len(), 2, "{errors:#?}");
    assert!(errors.iter().all(|e| e.contains("example.hutn") && e.contains("'Acyclic'")), "{errors:#?}");
}

#[test]
fn new_projects_may_have_an_ecore_metamodel_in_their_registry() {
    let request = NewProjectRequest { name: "plant".into(), metamodel: MetamodelFormat::Ecore, ..Default::default() };
    let (dir, created, ws, _) = create("ecore", request);
    assert!(paths(&created).contains(&"metamodels/plant.ecore"));
    assert!(!paths(&created).iter().any(|p| p.ends_with(".oclinecore")));
    let project = Project::load(&dir).unwrap();
    assert_eq!(
        project.registry(),
        vec![RegistryEntry {
            uri: "http://example.org/plant".into(),
            location: dir.join("metamodels/plant.ecore").to_string_lossy().into_owned()
        }]
    );
    let info = ws.registry();
    assert_eq!(info[0].status, RegistryStatus::Loaded, "{info:#?}");
    // The XMI model names the file, as EMF writes it.
    let xmi = &created.files.iter().find(|f| f.path == "models/example.xmi").unwrap().text;
    assert!(xmi.contains(r#"xsi:schemaLocation="http://example.org/plant ../metamodels/plant.ecore""#), "{xmi}");
    // The embedded OCL survives the conversion.
    let ecore = &created.files.iter().find(|f| f.path == "metamodels/plant.ecore").unwrap().text;
    assert!(ecore.contains("UniqueNames") && ecore.contains("closure"), "{ecore}");
}

#[test]
fn new_projects_have_the_documents_asked_for() {
    for language in ALL {
        for format in [MetamodelFormat::OclInEcore, MetamodelFormat::Ecore] {
            let name = format!("only_{}_{format:?}", language.extension()).to_lowercase();
            let request = NewProjectRequest {
                name: name.clone(),
                metamodel: format,
                include: Some(vec![language]),
                ..Default::default()
            };
            let (_, created, _, load) = create(&name, request);
            let documents: Vec<&str> =
                paths(&created).into_iter().filter(|p| document_language(Path::new(p)).is_some()).collect();
            // The metamodel (an `.ecore` file is not a document), and the one asked for.
            let expected = if format == MetamodelFormat::Ecore { 1 } else { 2 };
            assert_eq!(documents.len(), expected, "{documents:?}");
            assert!(documents.iter().any(|p| document_language(Path::new(p)) == Some(language)), "{documents:?}");
            assert_eq!(load.documents.len(), expected);
        }
    }
    let bare = NewProjectRequest { name: "bare".into(), include: Some(vec![]), ..Default::default() };
    let (_, created, _, _) = create("bare", bare);
    assert_eq!(
        paths(&created),
        vec!["oxychloride.json", "metamodels/bare.oclinecore", "README.md", ".vscode/extensions.json"]
    );
}

#[test]
fn the_transformations_of_new_projects_work() {
    let (dir, _, ws, _) = create("transform", NewProjectRequest { name: "transform".into(), ..Default::default() });
    let path = |p: &str| dir.join(p).to_string_lossy().into_owned();
    for (transformation, applications) in [("copy.qvto", 3), ("copy.qvtr", 3)] {
        let request = QvtRunRequest {
            document: path(&format!("transformations/{transformation}")),
            models: vec![QvtModelBinding { parameter: "source".into(), model: path("models/example.hutn") }],
            outputs: vec![QvtOutputPath { parameter: "target".into(), path: path("models/copy.xmi") }],
            ..QvtRunRequest::default()
        };
        let result = ws.run_qvt(&request).unwrap();
        assert_eq!(result.problems, vec![], "{transformation}");
        assert_eq!(result.applications, applications, "{transformation}");
        assert_eq!(result.outputs[0].objects, 3, "{transformation}");
    }

    // The Henshin text and the `.henshin` file mean the same: converting one gives the other.
    let text = ws.export_document(&path("transformations/transform.henshin"), None).unwrap().text;
    let original = std::fs::read_to_string(dir.join("transformations/transform.henshin_text")).unwrap();
    let rules = |t: &str| t.lines().filter(|l| l.starts_with("rule ")).map(str::to_string).collect::<Vec<_>>();
    assert_eq!(rules(&text), rules(&original));
    assert_eq!(rules(&original).len(), 2);
}

#[test]
fn project_names_become_identifiers() {
    assert_eq!(identifier("My Shop"), "my_shop");
    assert_eq!(identifier("  power-grid v2 "), "power_grid_v2");
    assert_eq!(identifier("2024 models"), "m2024_models");
    assert_eq!(identifier("Ünïcode"), "n_code");
    assert_eq!(identifier("---"), "model");
}

#[test]
fn new_projects_refuse_what_cannot_be_written() {
    let refuse = |request: NewProjectRequest| Workspace::new_project(&request).unwrap_err().to_string();
    assert!(refuse(NewProjectRequest { name: " ".into(), ..Default::default() }).contains("needs a name"));
    let quote = NewProjectRequest { name: "a".into(), ns_uri: Some("http://x/'".into()), ..Default::default() };
    assert!(refuse(quote).contains("cannot be an nsURI"));
    let target = NewProjectRequest { name: "a".into(), codegen: Some("cobol".into()), ..Default::default() };
    assert!(refuse(target).contains("unknown code generation target 'cobol'"));
    // A name that is a keyword of one of the languages: the document that cannot use it is named.
    let keyword = NewProjectRequest { name: "rule".into(), ..Default::default() };
    let e = refuse(keyword);
    assert!(e.contains("cannot create a project named 'rule'") && e.contains("rule.henshin_text"), "{e}");
}

#[test]
fn manifests_have_defaults() {
    let dir = folder("defaults");
    let project = Project::parse(r#"{"$schema": "x", "registry": {"pathmap://LIB/": "lib"}}"#, &dir).unwrap();
    assert_eq!(project.manifest.name, "defaults");
    assert_eq!(project.manifest.folders.models, "models");
    assert_eq!(project.manifest.codegen, None);
    let entries = project.registry();
    assert_eq!(entries[0].uri, "pathmap://LIB/");
    assert!(entries[0].location.ends_with(&format!("lib{}", std::path::MAIN_SEPARATOR)), "{entries:?}");
    assert!(Project::parse("{", &dir).is_err());
    assert!(Project::parse(r#"{"folders": {"models": 3}}"#, &dir).is_err());
}

#[test]
fn the_documents_of_a_project_are_below_it_but_not_in_nested_projects_or_generated_code() {
    let dir = folder("layout");
    let files = [
        "oxychloride.json",
        "a.oclinecore",
        "deep/er/b.hutn",
        "notes.txt",
        "src-gen/c.ocl",
        "gen/d.ocl",
        ".hidden/e.ocl",
        "target/f.ocl",
        "nested/oxychloride.json",
        "nested/g.qvto",
    ];
    for f in files {
        let path = dir.join(f);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, if f.ends_with(".json") { "{}" } else { "" }).unwrap();
    }
    let project = Project::load(&dir).unwrap();
    let relative = |p: &Project| -> Vec<String> {
        p.documents().iter().map(|d| d.strip_prefix(&p.root).unwrap().to_string_lossy().replace('\\', "/")).collect()
    };
    assert_eq!(relative(&project), vec!["a.oclinecore", "deep/er/b.hutn", "gen/d.ocl"]);
    // Code generated elsewhere is not part of the project either.
    std::fs::write(dir.join(MANIFEST), r#"{"codegen": {"target": "rust", "outputFolder": "gen"}}"#).unwrap();
    assert_eq!(relative(&Project::load(&dir).unwrap()), vec!["a.oclinecore", "deep/er/b.hutn"]);

    // The project of a file is the nearest one above it.
    let canonical = |p: PathBuf| std::fs::canonicalize(p).unwrap();
    assert_eq!(Project::find(dir.join("deep/er/b.hutn")), Some(canonical(dir.join(MANIFEST))));
    assert_eq!(Project::find(dir.join("nested/g.qvto")), Some(canonical(dir.join("nested").join(MANIFEST))));
}

#[test]
fn registry_problems_of_a_project_are_located_on_its_manifest() {
    let dir = folder("broken");
    std::fs::write(dir.join(MANIFEST), r#"{"registry": {"http://example.org/gone": "gone.ecore"}}"#).unwrap();
    let mut ws = Workspace::new();
    let load = ws.open_project(&Project::load(&dir).unwrap());
    assert_eq!(load.problems.len(), 1, "{load:#?}");
    assert!(load.problems[0].message.contains("'http://example.org/gone'"), "{load:#?}");
    assert!(load.problems[0].source.ends_with(MANIFEST));
}

/// The university metamodel and the QVT example's metamodels.
fn examples() -> Workspace {
    let mut ws = Workspace::new();
    for file in ["university/university.oclinecore", "qvt/uml.oclinecore", "qvt/rdbms.oclinecore"] {
        let path = format!("{EXAMPLES}/{file}");
        ws.open_document(&path, &std::fs::read_to_string(&path).unwrap());
    }
    ws
}

/// Opens a new document in `ws` and returns its errors.
fn new_document(ws: &mut Workspace, file: &str, request: NewDocumentRequest) -> (String, Vec<String>) {
    let text = ws.new_document(&request).unwrap();
    let name = format!("{EXAMPLES}/new/{file}");
    let diagnostics = ws.open_document(&name, &text);
    ws.close_document(&name);
    (text, errors(&diagnostics))
}

#[test]
fn new_documents_are_free_of_errors() {
    let mut ws = examples();
    let request = |language, name: &str, packages: &[&str]| NewDocumentRequest {
        language,
        name: name.into(),
        packages: packages.iter().map(|p| p.to_string()).collect(),
        ns_uri: None,
    };

    let (text, errors) = new_document(&mut ws, "grid.oclinecore", request(Language::OclInEcore, "Power Grid", &[]));
    assert_eq!(errors, Vec::<String>::new());
    assert!(text.contains("package power_grid : power_grid = 'http://example.org/power_grid' {"), "{text}");
    assert!(text.contains("class PowerGrid {"), "{text}");

    let (text, errors) = new_document(&mut ws, "more.ocl", request(Language::Ocl, "", &["university"]));
    assert_eq!(errors, Vec::<String>::new());
    assert!(text.contains("import 'http://www.example.org/university/1.0'"), "{text}");
    assert!(text.contains("package university\n"), "{text}");

    let (text, errors) = new_document(&mut ws, "m.hutn", request(Language::Hutn, "", &["university"]));
    assert_eq!(errors, Vec::<String>::new());
    assert!(text.contains(r#"nsUri: "http://www.example.org/university/1.0""#), "{text}");

    // The root classes are mapped: those no containment holds.
    let (text, errors) = new_document(&mut ws, "t.qvto", request(Language::Qvto, "toSchema", &["uml", "rdbms"]));
    assert_eq!(errors, Vec::<String>::new(), "{text}");
    assert!(text.contains("transformation toSchema(in source : UML, out target : RDBMS);"), "{text}");
    assert!(text.contains("mapping UML::Package::toSchema() : RDBMS::Schema {"), "{text}");

    let (text, errors) = new_document(&mut ws, "t.qvtr", request(Language::Qvtr, "uml2rdbms", &["uml", "rdbms"]));
    assert_eq!(errors, Vec::<String>::new(), "{text}");
    assert!(text.contains("top relation PackageToSchema {"), "{text}");

    // One metamodel: an in-place transformation, or one between models of it.
    let (text, errors) = new_document(&mut ws, "u.qvto", request(Language::Qvto, "u", &["university"]));
    assert_eq!(errors, Vec::<String>::new(), "{text}");
    assert_eq!(text.matches("modeltype").count(), 1, "{text}");
    let (text, errors) = new_document(&mut ws, "u.qvtr", request(Language::Qvtr, "u", &["university"]));
    assert_eq!(errors, Vec::<String>::new(), "{text}");
    assert!(text.contains("top relation UniversityToUniversity {"), "{text}");

    let e = ws.new_document(&request(Language::Ocl, "", &["nowhere"])).unwrap_err();
    assert_eq!(e.to_string(), "no package 'nowhere' is loaded");
    assert!(ws.new_document(&request(Language::Qvtr, "x", &[])).is_err());
}

#[test]
fn projects_over_json_rpc() {
    let mut server = Server::new();
    let r = server
        .handle(json!({"jsonrpc": "2.0", "id": 1, "method": "newProject",
        "params": {"name": "grid", "include": ["hutn", "henshinModel"], "metamodel": "ecore", "codegen": "rust"}}))
        .unwrap();
    let files: Vec<&str> =
        r["result"]["files"].as_array().unwrap().iter().map(|f| f["path"].as_str().unwrap()).collect();
    assert_eq!(
        files,
        vec![
            "oxychloride.json",
            "metamodels/grid.ecore",
            "models/example.hutn",
            "transformations/grid.henshin",
            "README.md",
            ".vscode/extensions.json"
        ]
    );
    assert_eq!(r["result"]["nsUri"], "http://example.org/grid");
    let manifest: serde_json::Value = serde_json::from_str(r["result"]["files"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(manifest["codegen"], json!({"target": "rust"}));

    let r = server.handle(json!({"jsonrpc": "2.0", "id": 2, "method": "newProject", "params": {"name": ""}})).unwrap();
    assert_eq!(r["error"]["code"], rpc::codes::INVALID_PARAMS);

    let r = server
        .handle(json!({"jsonrpc": "2.0", "id": 3, "method": "newDocument",
        "params": {"language": "oclinecore", "name": "grid", "nsUri": "urn:grid"}}))
        .unwrap();
    assert!(r["result"]["text"].as_str().unwrap().contains("package grid : grid = 'urn:grid' {"), "{r}");
}
