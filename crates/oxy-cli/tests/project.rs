//! `oxy new` and `oxy check` of a project.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn oxy(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_oxy")).args(args).arg("--no-color").current_dir(dir).output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn folder(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn new_projects_check_without_problems() {
    let dir = folder("new");
    let out = oxy(&dir, &["new", "shop", "--codegen", "rust"]);
    assert!(out.status.success(), "{out:?}");
    assert!(stdout(&out).contains("created shop (11 files; package shop, nsURI http://example.org/shop)"), "{out:?}");

    // In the project, or in a folder of it: the project is found.
    let out = oxy(&dir.join("shop/models"), &["check"]);
    assert!(out.status.success(), "{out:?}");
    assert!(stdout(&out).contains("8 documents checked, no problems"), "{out:?}");

    // Files are not replaced unless asked to.
    let out = oxy(&dir, &["new", "shop", "--bare"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("exists (use --force to replace it)"), "{out:?}");

    // A violated constraint in a model is a problem of the project.
    let model = dir.join("shop/models/example.hutn");
    let text = std::fs::read_to_string(&model).unwrap().replace(r#"name: "second""#, r#"name: "first""#);
    std::fs::write(&model, text).unwrap();
    let out = oxy(&dir, &["check", "--project", "shop"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(stdout(&out).contains("invariant 'UniqueNames' of shop::Model is violated"), "{out:?}");
    assert!(stdout(&out).contains("shop/models/example.hutn"), "{out:?}");
}

#[test]
fn new_projects_take_their_parts_and_format() {
    let dir = folder("parts");
    let out = oxy(&dir, &["new", "grid", "--ecore", "--with", "hutn,qvto", "--ns-uri", "urn:grid", "--format", "json"]);
    assert!(out.status.success(), "{out:?}");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["nsUri"], "urn:grid");
    assert_eq!(json["files"].as_array().unwrap().len(), 6, "{json}");
    assert!(dir.join("grid/metamodels/grid.ecore").is_file());
    let manifest = std::fs::read_to_string(dir.join("grid/oxychloride.json")).unwrap();
    assert!(manifest.contains(r#""urn:grid": "metamodels/grid.ecore""#), "{manifest}");
    let out = oxy(&dir.join("grid"), &["check"]);
    assert!(stdout(&out).contains("3 documents checked, no problems"), "{out:?}");
}

#[test]
fn check_without_documents_needs_a_project() {
    let dir = folder("none");
    let out = oxy(&dir, &["check"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no oxychloride.json in this folder or above"), "{out:?}");
}
