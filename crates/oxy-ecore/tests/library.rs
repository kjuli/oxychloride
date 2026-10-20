use oxy_ecore::*;

const EXAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/library");

fn load() -> (Metamodel, Model, XmiLoad) {
    let mut mm = Metamodel::new();
    let load = mm.load_ecore_file(format!("{EXAMPLES}/library.ecore")).unwrap();
    assert!(load.warnings.is_empty(), "{:?}", load.warnings);
    let doc = XmiDocument::read_file(format!("{EXAMPLES}/library.xmi")).unwrap();
    let mut model = Model::new();
    let xmi = doc.load(&mm, &mut model).unwrap();
    assert!(xmi.warnings.is_empty(), "{:?}", xmi.warnings);
    (mm, model, xmi)
}

#[test]
fn builtin_ecore_is_complete() {
    let mm = Metamodel::new();
    let ecore = mm.ecore_package();
    let eclass = mm.find_classifier_in(ecore, "EClass").unwrap();
    let enamed = mm.find_classifier_in(ecore, "ENamedElement").unwrap();
    assert!(mm.conforms_to(eclass, enamed));
    assert!(mm.lookup_feature(eclass, "name").is_some());
    assert!(mm.lookup_feature(eclass, "eAnnotations").is_some());
    let estring = mm.find_classifier_in(ecore, "EString").unwrap();
    assert_eq!(mm.data_type_kind(estring), Some(DataTypeKind::String));
}

#[test]
fn loads_library_metamodel() {
    let (mm, _, _) = load();
    let lib = mm.package_by_ns_uri("http://www.example.org/library/1.0").unwrap();
    let book = mm.find_classifier_in(lib, "Book").unwrap();
    let copies = mm.lookup_feature(book, "copies").unwrap();
    assert_eq!(mm.classifier(mm.feature(copies).ty.unwrap()).name, "EInt");
    let library = mm.lookup_feature(book, "library").unwrap();
    let books = mm.feature(library).opposite().unwrap();
    assert_eq!(mm.feature(books).name, "books");
    assert!(mm.feature(books).is_containment());

    let member = mm.find_classifier_in(lib, "Member").unwrap();
    let person = mm.find_classifier_in(lib, "Person").unwrap();
    assert!(mm.conforms_to(member, person));
    assert!(mm.conforms_to(member, mm.eobject()));
    assert_eq!(mm.lookup_feature(member, "age").map(|f| mm.feature(f).owner), Some(person));
    assert_eq!(mm.qualified_name(member), "library::Member");
    assert_eq!(mm.find_classifiers_qualified(&["library", "Book"]), vec![book]);

    let genre = mm.find_classifier_in(lib, "Genre").unwrap();
    assert_eq!(mm.enum_literal_index(genre, "HISTORY"), Some(2));
    assert_eq!(mm.lookup_operations(book, "isAvailable").len(), 1);
}

#[test]
fn loads_library_model() {
    let (mm, model, xmi) = load();
    assert_eq!(xmi.roots.len(), 1);
    let lib = xmi.roots[0];
    assert_eq!(model.label(&mm, lib), "Library 'City Library'");
    let pkg = mm.package_by_ns_uri("http://www.example.org/library/1.0").unwrap();
    let book = mm.find_classifier_in(pkg, "Book").unwrap();
    let books = model.instances_of(&mm, book);
    assert_eq!(books.len(), 3);
    assert_eq!(model.uri_fragment(&mm, books[1]), "//@books.1");
    assert_eq!(model.resolve_fragment(&mm, 0, "//@books.1"), Some(books[1]));
    assert_eq!(model.container(books[0]), Some(lib));

    // The opposite of the containment reference is filled in.
    let library = mm.lookup_feature(book, "library").unwrap();
    assert_eq!(model.get(books[0], library), &[ModelValue::Object(lib)]);

    // Member.loans <-> Loan.member are kept consistent.
    let member = mm.find_classifier_in(pkg, "Member").unwrap();
    let alice = model.instances_of(&mm, member)[0];
    let loans = mm.lookup_feature(member, "loans").unwrap();
    assert_eq!(model.get(alice, loans).len(), 2);

    let copies = mm.lookup_feature(book, "copies").unwrap();
    assert_eq!(model.get(books[0], copies), &[ModelValue::Integer(2)]);
    let isbn = mm.lookup_feature(book, "isbn").unwrap();
    assert!(model.get_or_default(&mm, books[2], isbn).is_empty());
}

#[test]
fn reports_unknown_metamodel() {
    let mm = Metamodel::new();
    let doc = XmiDocument::read_file(format!("{EXAMPLES}/library.xmi")).unwrap();
    assert_eq!(doc.used_ns_uris(), vec!["http://www.example.org/library/1.0".to_string()]);
    let locations = doc.schema_locations();
    assert!(locations[0].1.ends_with("library.ecore"));
    let err = doc.load(&mm, &mut Model::new()).unwrap_err();
    assert!(err.message.contains("unknown package nsURI"), "{err}");
}

/// Renders every object of a resource with its feature values, for comparing models that
/// were loaded or written in different ways.
fn dump(mm: &Metamodel, model: &Model, resource: usize) -> Vec<String> {
    let mut out = Vec::new();
    for r in &model.resources()[resource].roots {
        for o in std::iter::once(*r).chain(model.all_contents(mm, *r)) {
            let mut line = format!("{} {}:", mm.classifier(model.class_of(o)).name, model.uri_fragment(mm, o));
            for &f in mm.all_features(model.class_of(o)) {
                let values: Vec<String> = model
                    .get(o, f)
                    .iter()
                    .map(|v| match v {
                        ModelValue::Object(t) => model.uri_fragment(mm, *t),
                        other => other.to_string(),
                    })
                    .collect();
                if !values.is_empty() {
                    line.push_str(&format!(" {}={}", mm.feature(f).name, values.join(",")));
                }
            }
            out.push(line);
        }
    }
    out
}

#[test]
fn hutn_model_matches_xmi_model() {
    let (mm, xmi_model, xmi) = load();
    let text = std::fs::read_to_string(format!("{EXAMPLES}/library.hutn")).unwrap();
    let mut model = Model::new();
    let load = hutn::load(&text, "library.hutn", &mm, &mut model);
    assert!(load.problems.is_empty(), "{:?}", load.problems);
    assert_eq!(load.objects.len(), 12);
    assert_eq!(dump(&mm, &model, load.resource), dump(&mm, &xmi_model, xmi.resource));
    // Identifiers become xmi:ids.
    assert_eq!(model.resolve_fragment(&mm, load.resource, "dune"), Some(load.objects[1].object));
}

#[test]
fn hutn_reports_problems_with_ranges() {
    let (mm, _, _) = load();
    let text = r#"library {
  Library "l" {
    name: 42
    books: Book "a" { copies: "two" genre: POETRY }, Author "x" {}
    members: Member "m" { loans: Loan "nope" }
    colour: "red"
  }
  Book "b" { library: Library "l" }
  Person "p" {}
}"#;
    let mut model = Model::new();
    let load = hutn::load(text, "bad.hutn", &mm, &mut model);
    let messages: Vec<(&str, &str)> =
        load.problems.iter().map(|p| (&text[p.span.clone()], p.message.as_str())).collect();
    let expect = |snippet: &str, fragment: &str| {
        assert!(
            messages.iter().any(|(s, m)| *s == snippet && m.contains(fragment)),
            "no problem '{fragment}' at '{snippet}' in {messages:#?}"
        );
    };
    expect("42", "expected a string");
    expect("\"two\"", "expected an integer");
    expect("POETRY", "not a literal of Genre");
    expect("Person", "abstract");
    expect("Author", "is not a Book");
    expect("Loan \"nope\"", "no object with identifier \"nope\"");
    expect("colour", "has no feature 'colour'");
    expect("library", "set by nesting");
}

#[test]
fn ecore_round_trip() {
    let (mm, _, _) = load();
    let resource = mm
        .resource_of_uri(&std::fs::canonicalize(format!("{EXAMPLES}/library.ecore")).unwrap().to_string_lossy())
        .unwrap();
    let text = mm.write_ecore(resource);
    let mut copy = Metamodel::new();
    let load = copy.load_ecore_str(&text, "copy.ecore").unwrap();
    assert!(load.warnings.is_empty(), "{:?}", load.warnings);
    assert_eq!(copy.write_ecore(load.resource), text);
    let pkg = copy.package_by_ns_uri("http://www.example.org/library/1.0").unwrap();
    let book = copy.find_classifier_in(pkg, "Book").unwrap();
    let library = copy.lookup_feature(book, "library").unwrap();
    assert_eq!(copy.feature(copy.feature(library).opposite().unwrap()).name, "books");
    assert_eq!(copy.classifier(book).annotations, mm.classifier(mm.find_classifiers("Book")[0]).annotations);
}

#[test]
fn xmi_and_hutn_round_trips() {
    let (mm, model, xmi) = load();
    let expected = dump(&mm, &model, xmi.resource);

    let text = xmi_writer::write_resource(&mm, &model, xmi.resource, &[]);
    let mut copy = Model::new();
    let reloaded = XmiDocument::parse(text, "copy.xmi").unwrap().load(&mm, &mut copy).unwrap();
    assert!(reloaded.warnings.is_empty(), "{:?}", reloaded.warnings);
    assert_eq!(dump(&mm, &copy, reloaded.resource), expected);

    let text = hutn::write(&mm, &model, xmi.resource);
    let mut copy = Model::new();
    let reloaded = hutn::load(&text, "copy.hutn", &mm, &mut copy);
    assert!(reloaded.problems.is_empty(), "{:?}\n{text}", reloaded.problems);
    assert_eq!(dump(&mm, &copy, reloaded.resource), expected, "{text}");
    assert_eq!(hutn::write(&mm, &copy, reloaded.resource), text);
}
