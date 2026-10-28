//! Eclipse projects and their target platform: `platform:` URIs, the packages, URI mappings
//! and model extensions of `plugin.xml`, bundles in jars, OCL in `.ecore` annotations, and
//! `.ecore` files edited as EMF saves them.
//!
//! `tests/fixtures/eclipse/repo` has one plug-in project, `org.example.shop`: its metamodel's
//! classes extend `Named` of `org.example.base`, a bundle of the target platform
//! (`target/plugins`, two versions as jars, only the newer with `Named`). Its models are
//! `.shop` files; `instances/my.shop` refers to the library through `pathmap://SHOP_LIBRARY/`.

use std::path::Path;

use oxychloride::*;
use serde_json::json;

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/eclipse");

fn settings(target: bool) -> PlatformSettings {
    PlatformSettings {
        workspace: vec![format!("{DIR}/repo")],
        target: if target { vec![format!("{DIR}/target/plugins")] } else { Vec::new() },
    }
}

fn shop_ecore() -> String {
    format!("{DIR}/repo/org.example.shop/model/shop.ecore")
}

fn errors(diagnostics: &[Diagnostic]) -> Vec<String> {
    diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| format!("{}: {}", d.source, d.message))
        .collect()
}

/// The project as `oxy import` makes it (its manifest is not written).
fn imported() -> (Project, ImportedProject) {
    let request = ImportEclipseRequest {
        folder: format!("{DIR}/repo"),
        name: None,
        target_platform: vec!["../target/plugins".into()],
    };
    let imported = Workspace::import_eclipse_projects(&request).unwrap();
    (Project::parse(&imported.manifest.text, format!("{DIR}/repo")).unwrap(), imported)
}

fn apply(text: &str, edits: &[TextEdit]) -> String {
    let offset = |p: Position| {
        let line_start: usize = text.split_inclusive('\n').take(p.line as usize).map(str::len).sum();
        let line = &text[line_start..];
        line_start + line.char_indices().nth(p.column as usize).map_or(line.len(), |(i, _)| i)
    };
    let mut out = text.to_string();
    for e in edits.iter().rev() {
        out.replace_range(offset(e.range.start)..offset(e.range.end), &e.text);
    }
    out
}

#[test]
fn projects_find_the_bundles_they_require_in_the_target_platform() {
    let mut ws = Workspace::new();
    let info = ws.set_platform(settings(true));
    let shop = &info.projects[0];
    assert_eq!((shop.name.as_str(), shop.project.as_deref()), ("org.example.shop", Some("org.example.shop")));
    assert_eq!(shop.version.as_deref(), Some("1.2.0.qualifier"));
    assert_eq!(shop.packages[0].ns_uri, "http://example.org/shop/1.0");
    assert_eq!(shop.packages[0].status, RegistryStatus::Loaded, "{:?}", shop.packages[0]);
    assert_eq!(shop.model_extensions, ["shop"]);
    assert_eq!(shop.uri_mappings[0].source, "pathmap://SHOP_LIBRARY/");
    // The newest version of the required bundle; EMF's own bundles are built in, optional ones
    // may be missing.
    assert_eq!(info.required.len(), 1);
    assert_eq!(info.required[0].version.as_deref(), Some("1.0.0.v2024"));
    assert!(info.missing.is_empty(), "{:?}", info.missing);
    assert_eq!(info.model_extensions, ["shop"]);

    // Item extends Named from the jar (`platform:/plugin/org.example.base/…`).
    let mm = ws.metamodel();
    let named = mm.find_classifiers("Named");
    assert_eq!(named.len(), 1);
    let item = mm.find_classifiers("Item")[0];
    assert!(mm.all_super_types(item).contains(&named[0]));
    assert!(ws.diagnostics(None).is_empty(), "{:#?}", ws.diagnostics(None));
}

#[test]
fn without_the_target_platform_required_bundles_are_missing() {
    let mut ws = Workspace::new();
    let info = ws.set_platform(settings(false));
    assert_eq!(info.missing.len(), 1);
    assert_eq!(
        (info.missing[0].name.as_str(), info.missing[0].required_by.as_str()),
        ("org.example.base", "org.example.shop")
    );
    assert!(ws.metamodel().find_classifiers("Named").is_empty());
}

#[test]
fn imported_projects_check_their_models_with_the_ocl_of_their_metamodels() {
    let (project, imported) = imported();
    assert_eq!(project.manifest.name, "org.example.shop");
    assert_eq!(project.manifest.target_platform, ["../target/plugins"]);
    assert_eq!(imported.platform.projects.len(), 1);
    let mut ws = Workspace::new();
    let load = ws.open_project(&project);
    assert!(load.problems.is_empty(), "{:#?}", load.problems);
    // The metamodel first, then the models of the bundle's file extension.
    let documents: Vec<&str> =
        load.documents.iter().map(|d| d.strip_prefix(&format!("{DIR}/repo/")).unwrap_or(d)).collect();
    assert_eq!(
        documents,
        [
            "org.example.shop/model/shop.ecore",
            "org.example.shop/instances/my.shop",
            "org.example.shop/library/standard.shop"
        ]
    );
    assert_eq!(ws.document_language(&load.documents[1]), Some(Language::Xmi));

    // `positivePrice` (an LPG annotation) fails for "free" only: `allInstances()` are the
    // objects of the model's own resource, as in Eclipse OCL, so the library's "bolt" is not a
    // duplicate. The reference into the library (a `pathmap:` URI) resolves.
    let errors = errors(&ws.diagnostics(None));
    assert_eq!(errors.len(), 1, "{errors:#?}");
    assert!(
        errors[0].contains("my.shop") && errors[0].contains("invariant 'positivePrice' of shop::Item"),
        "{errors:#?}"
    );
    let nut = ws.evaluate("shop::Item.allInstances()->any(name = 'nut').related->collect(name)", None);
    assert_eq!(serde_json::to_value(nut.unwrap().value).unwrap(), json!({"kind": "Sequence", "items": ["bolt"]}));
}

#[test]
fn ocl_in_ecore_annotations_is_checked_where_it_is_in_the_file() {
    let mut ws = Workspace::new();
    ws.set_platform(settings(true));
    let name = shop_ecore();
    let text = std::fs::read_to_string(&name).unwrap().replace("self.price &gt; 0", "self.prize &gt; 0");
    ws.open_document_as(&name, &text, Language::Ecore);
    let diagnostics = ws.diagnostics(Some(&name));
    let d = diagnostics.iter().find(|d| d.message.contains("prize")).expect("the error of the annotation");
    assert_eq!(d.source, name);
    // `prize` in `<details key="positivePrice" value="self.prize &gt; 0 …"/>` (line 13).
    let line = text.lines().nth(d.range.start.line as usize).unwrap();
    let column = d.range.start.column as usize;
    assert_eq!(&line[column..column + 5], "prize", "{d:?}");
    // The text in the editor stands for the file: the metamodel is the edited one.
    ws.open_document_as(&name, &text.replace("name=\"Catalog\"", "name=\"Shelf\""), Language::Ecore);
    assert!(!ws.metamodel().find_classifiers("Shelf").is_empty());
    assert!(ws.metamodel().find_classifiers("Catalog").is_empty());
}

#[test]
fn ecore_files_are_edited_as_emf_saves_them() {
    let mut ws = Workspace::new();
    ws.set_platform(settings(true));
    let name = shop_ecore();
    let mut text = std::fs::read_to_string(&name).unwrap();
    assert!(text.contains("\r\n"));
    ws.open_document_as(&name, &text, Language::Ecore);
    let edit = |ws: &mut Workspace, text: &mut String, e: serde_json::Value| {
        let result = ws.edit_metamodel(&name, Some(text), &serde_json::from_value(e).unwrap()).unwrap();
        *text = apply(text, &result.edits);
        ws.open_document_as(&name, text, Language::Ecore);
    };
    edit(
        &mut ws,
        &mut text,
        json!({"kind": "addClassifier", "package": "shop", "classifier": "class", "name": "Supplier"}),
    );
    edit(&mut ws, &mut text, json!({"kind": "addSuperType", "owner": "shop::Supplier", "superType": "base::Named"}));
    edit(
        &mut ws,
        &mut text,
        json!({"kind": "addReference", "owner": "shop::Supplier", "name": "suppliedItems", "type": "shop::Item", "upper": -1}),
    );
    // The file's line breaks, a reference into the bundle as EMF writes it (not where the jar
    // is), and the start tag wrapped after column 80, its continuation two levels deeper.
    assert!(!text.replace("\r\n", "").contains('\n'), "{text}");
    assert!(
        text.contains(
            "  <eClassifiers xsi:type=\"ecore:EClass\" name=\"Supplier\" eSuperTypes=\"platform:/plugin/org.example.base/model/base.ecore#//Named\">\r\n    <eStructuralFeatures xsi:type=\"ecore:EReference\" name=\"suppliedItems\" upperBound=\"-1\"\r\n        eType=\"#//Item\"/>\r\n  </eClassifiers>\r\n</ecore:EPackage>"
        ),
        "{text}"
    );
    assert!(errors(&ws.diagnostics(None)).is_empty(), "{:#?}", ws.diagnostics(None));
    assert!(Path::new(&name).is_file());
}
