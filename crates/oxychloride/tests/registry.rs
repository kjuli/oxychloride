//! The package registry: URIs standing for metamodel files.

use oxychloride::rpc::Server;
use oxychloride::*;
use serde_json::json;

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/registry");

fn entry(uri: &str, location: &str) -> RegistryEntry {
    RegistryEntry { uri: uri.into(), location: format!("{DIR}/{location}") }
}

/// `orders.xml` refers to `shop.ecore` by its nsURI and by a `platform:` URI.
fn registry() -> Vec<RegistryEntry> {
    vec![
        entry("http://example.org/orders", "orders.xml"),
        entry("http://example.org/shop", "lib/shop.ecore"),
        entry("platform:/plugin/org.example.shop/model/shop.ecore", "lib/shop.ecore"),
    ]
}

fn messages(diagnostics: &[Diagnostic]) -> Vec<&str> {
    diagnostics.iter().map(|d| d.message.as_str()).collect()
}

#[test]
fn unregistered_uris_cannot_be_resolved() {
    let mut ws = Workspace::new();
    let r = ws.load_ecore_file(format!("{DIR}/orders.xml")).unwrap();
    assert_eq!(
        messages(&r.warnings),
        vec![
            "cannot resolve reference 'platform:/plugin/org.example.shop/model/shop.ecore#//Named'",
            "cannot resolve reference 'http://example.org/shop#//Item'",
        ]
    );
}

#[test]
fn registered_metamodels_are_loaded_with_what_they_import() {
    let mut ws = Workspace::new();
    let infos = ws.set_registry(registry());
    assert!(infos.iter().all(|i| i.status == RegistryStatus::Loaded && i.warnings.is_empty()), "{infos:#?}");
    // Order is loaded first and pulls in shop.ecore through the registry.
    let orders = &infos[0];
    assert_eq!(orders.resource.as_ref().unwrap().packages[0].ns_uri, "http://example.org/orders");
    assert_eq!(orders.imports.len(), 1);
    let shop = &orders.imports[0];
    assert!(shop.uri.ends_with("lib/shop.ecore"));
    assert_eq!(
        shop.packages,
        vec![RegisteredPackage { name: "shop".into(), ns_uri: "http://example.org/shop".into() }]
    );
    // The file is known by the URIs that stand for it.
    assert!(shop.aliases.contains(&"platform:/plugin/org.example.shop/model/shop.ecore".to_string()));
    assert_eq!(infos[1].resource.as_ref().unwrap().uri, shop.uri);
    assert!(infos[1].imports.is_empty());

    // Both references resolved: Order is Named and contains Items.
    let order = ws.metamodel().find_classifiers("Order")[0];
    let named = ws.metamodel().find_classifiers("Named")[0];
    assert!(ws.metamodel().conforms_to(order, named));
    let packages: Vec<String> = ws.packages().into_iter().map(|p| p.ns_uri).collect();
    assert_eq!(packages, vec!["http://example.org/orders", "http://example.org/shop"]);
}

#[test]
fn documents_and_models_find_registered_metamodels() {
    let mut ws = Workspace::new();
    ws.set_registry(registry());

    // A Complete OCL document importing by nsURI, and by the placeholder URI of the file.
    let ocl = format!("{DIR}/orders.ocl");
    let diags = ws.open_document(
        &ocl,
        "import 'http://example.org/orders'\nimport 'platform:/plugin/org.example.shop/model/shop.ecore'\n\
         context orders::Order inv NotEmpty: items->notEmpty()\n\
         context shop::Item inv Priced: price > 0",
    );
    assert!(diags.is_empty(), "{diags:?}");
    let diags = ws.open_document(&format!("{DIR}/other.ocl"), "import 'http://example.org/nothing'");
    assert_eq!(
        messages(&diags),
        vec![
            "cannot find imported metamodel 'http://example.org/nothing': register a file for it in the package registry"
        ]
    );

    // A HUTN model of the registered nsURI.
    let hutn = format!("{DIR}/orders.hutn");
    let text = "@Spec { metamodel \"orders\" { nsUri: \"http://example.org/orders\" } }\n\
                orders { Order \"o\" { name: \"first\" items: Item \"i\" { name: \"pen\" price: 0 } } }";
    let diags = ws.open_document(&hutn, text);
    assert_eq!(messages(&diags), vec!["invariant 'Priced' of shop::Item is violated"]);

    // An XMI model whose schema location is a placeholder.
    let xmi = format!("{DIR}/orders.xmi");
    let diags = ws.open_document(
        &xmi,
        r#"<orders:Order xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    xmlns:orders="http://example.org/orders" xsi:schemaLocation="http://example.org/orders http://example.org/orders.ecore"
    name="second"/>"#,
    );
    assert_eq!(messages(&diags), vec!["invariant 'NotEmpty' of orders::Order is violated"]);

    // Without the registry, the models lose their metamodel.
    ws.set_registry(Vec::new());
    assert!(ws.metamodel().package_by_ns_uri("http://example.org/orders").is_none());
    assert!(!ws.diagnostics(Some(&hutn)).is_empty());
    let errors = messages(&ws.diagnostics(Some(&ocl))).into_iter().filter(|m| m.contains("cannot find")).count();
    assert_eq!(errors, 2);
}

#[test]
fn oclinecore_documents_import_registered_metamodels() {
    let mut ws = Workspace::new();
    ws.set_registry(registry());
    let doc = format!("{DIR}/catalog.oclinecore");
    let diags = ws.open_document(
        &doc,
        "import shop : 'http://example.org/shop';\n\
         package catalog : catalog = 'http://example.org/catalog' {\n\
           class Catalog extends shop::Named { property items : shop::Item[*] { composes }; }\n\
         }",
    );
    assert!(diags.is_empty(), "{diags:?}");
    let catalog = ws.metamodel().find_classifiers("Catalog")[0];
    let named = ws.metamodel().find_classifiers("Named")[0];
    assert!(ws.metamodel().conforms_to(catalog, named));
}

#[test]
fn missing_files_folders_and_prefixes() {
    let mut ws = Workspace::new();
    let infos = ws.set_registry(vec![
        entry("http://example.org/gone", "gone.ecore"),
        entry("pathmap://SHOP/", "lib"),
        entry("http://example.org/orders", "orders.xml"),
    ]);
    let statuses: Vec<RegistryStatus> = infos.iter().map(|i| i.status).collect();
    assert_eq!(statuses, vec![RegistryStatus::Missing, RegistryStatus::Folder, RegistryStatus::Loaded]);
    assert!(infos[0].message.as_deref().unwrap().starts_with("there is no file"));
    // orders.xml's references are not covered by the prefix.
    assert_eq!(infos[2].warnings.len(), 2);

    // A file under the prefix is loaded when something refers to it.
    let diags = ws
        .open_document(&format!("{DIR}/items.ocl"), "import 'pathmap://SHOP/shop.ecore'\ncontext shop::Item inv: true");
    assert!(diags.is_empty(), "{diags:?}");
    assert!(ws.metamodel().package_by_ns_uri("http://example.org/shop").is_some());

    // Unregistering.
    assert!(ws.unregister("http://example.org/orders"));
    assert!(!ws.unregister("http://example.org/orders"));
    assert_eq!(ws.registry().len(), 2);
    assert!(ws.metamodel().package_by_ns_uri("http://example.org/orders").is_none());
}

#[test]
fn the_registry_over_rpc() {
    let mut server = Server::new();
    let entries: Vec<_> = registry().into_iter().map(|e| json!({"uri": e.uri, "location": e.location})).collect();
    let infos = server.call("setRegistry", json!({ "entries": entries })).unwrap();
    assert_eq!(infos.as_array().unwrap().len(), 3);
    assert_eq!(infos[0]["status"], "loaded");
    assert_eq!(infos[0]["imports"][0]["packages"][0]["nsUri"], "http://example.org/shop");
    assert_eq!(server.call("registry", json!(null)).unwrap(), infos);
    let packages = server.call("packages", json!(null)).unwrap();
    assert_eq!(packages.as_array().unwrap().len(), 2);
}
