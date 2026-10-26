//! Metamodels written in OCLinEcore and models written in HUTN, validated with OCL.

use oxychloride::*;
use serde_json::json;

const EXAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");

fn example(file: &str) -> (String, String) {
    let path = format!("{EXAMPLES}/{file}");
    let text = std::fs::read_to_string(&path).unwrap();
    (path, text)
}

/// The university example: a metamodel in OCLinEcore and a model in HUTN.
fn university() -> (Workspace, String, String) {
    let mut ws = Workspace::new();
    let (mm_path, mm_text) = example("university/university.oclinecore");
    let diags = ws.open_document(&mm_path, &mm_text);
    assert!(diags.is_empty(), "{diags:#?}");
    let (model_path, model_text) = example("university/university.hutn");
    ws.open_document(&model_path, &model_text);
    (ws, mm_path, model_path)
}

/// The position of the end of the first occurrence of `marker` in `text`.
fn position_after(text: &str, marker: &str) -> Position {
    let offset = text.find(marker).unwrap_or_else(|| panic!("no '{marker}'")) + marker.len();
    let before = &text[..offset];
    let line = before.matches('\n').count() as u32;
    let column = before[before.rfind('\n').map_or(0, |i| i + 1)..].chars().count() as u32;
    Position { line, column }
}

fn labels(list: &CompletionList) -> Vec<&str> {
    list.items.iter().map(|i| i.label.as_str()).collect()
}

#[test]
fn validates_a_textual_model_against_a_textual_metamodel() {
    let (ws, mm_path, model_path) = university();
    assert_eq!(ws.document_language(&mm_path), Some(Language::OclInEcore));
    assert_eq!(ws.document_language(&model_path), Some(Language::Hutn));

    let report = ws.validate(&ValidateOptions::default()).unwrap();
    let failures: Vec<String> = report
        .results
        .iter()
        .map(|r| {
            let c = &report.constraints[r.constraint];
            format!("{} {} {}", c.name, r.object.label, r.call.as_deref().unwrap_or("-"))
        })
        .collect();
    assert_eq!(
        failures,
        [
            "CreditLimit Student 'Carla' -",
            "PrerequisitesMet Student 'Brian' -",
            "PrerequisitesMet Student 'Carla' -",
            "FreeSeat Course #prog1 hasSeat()",
            "FreeSeat Course #prog2 hasSeat()",
        ]
    );
    assert_eq!(report.summary.violated, 5);
    assert_eq!(report.summary.errors, 0);
    assert!(report.diagnostics.is_empty(), "{:#?}", report.diagnostics);
    // Every postcondition is checked: mayEnrol is called with every course on every student.
    let may_enrol = report.constraints.iter().position(|c| c.name == "PrerequisitesPassed").unwrap();
    assert_eq!(report.constraints[may_enrol].kind, ConstraintKind::Postcondition);
    let options = ValidateOptions { include_satisfied: true, ..Default::default() };
    let all = ws.validate(&options).unwrap();
    // 3 students x 4 courses, minus the 8 calls whose precondition (not enrolled) fails.
    assert_eq!(all.results.iter().filter(|r| r.constraint == may_enrol).count(), 12 - 8);

    // Objects are located in the HUTN document.
    let carla = &report.results[0].object;
    assert_eq!(carla.resource, model_path);
    assert_eq!(carla.range.unwrap().start.line, 58);
}

#[test]
fn reports_violations_in_the_model_document() {
    let (ws, mm_path, model_path) = university();
    let diags = ws.diagnostics(Some(&model_path));
    let messages: Vec<(u32, &str)> = diags.iter().map(|d| (d.range.start.line, d.message.as_str())).collect();
    assert_eq!(messages.len(), 5, "{messages:#?}");
    assert!(
        messages.contains(&(
            58,
            "invariant 'CreditLimit' of university::Student is violated: Student Carla takes 44 credits"
        ))
    );
    assert!(messages.contains(&(17, "postcondition 'FreeSeat' of Course::hasSeat is violated by hasSeat()")));
    assert!(
        diags
            .iter()
            .all(|d| d.severity == Severity::Error && d.origin == Origin::Document { name: model_path.clone() })
    );
    assert!(ws.diagnostics(Some(&mm_path)).is_empty());
}

#[test]
fn reports_problems_in_the_metamodel_document() {
    let (mut ws, mm_path, _) = university();
    let (_, text) = example("university/university.oclinecore");
    let broken = text
        .replace("credits <= 30;", "credits <= 'thirty';")
        .replace("property prerequisites : Course[*];", "property prerequisites : Lecture[*];");
    let diags = ws.open_document(&mm_path, &broken);
    let messages: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
    assert!(messages.contains(&"unknown type 'Lecture'"), "{messages:#?}");
    assert!(
        messages.iter().any(|m| m.contains("operator '<=' is not defined for Integer and String")),
        "{messages:#?}"
    );
    let line = broken.lines().position(|l| l.contains("'thirty'")).unwrap() as u32;
    assert!(diags.iter().any(|d| d.range.start.line == line), "diagnostics point into the document");
}

#[test]
fn models_follow_metamodel_edits() {
    let (mut ws, mm_path, model_path) = university();
    let (_, text) = example("university/university.oclinecore");
    // Rename a feature: the model now uses an unknown feature.
    let renamed = text.replace("attribute semester : Integer;", "attribute term : Integer;");
    ws.open_document(&mm_path, &renamed);
    let diags = ws.diagnostics(Some(&model_path));
    assert!(diags.iter().any(|d| d.message == "class 'Student' has no feature 'semester'"), "{diags:#?}");
    // Constraints are not checked while the model has errors.
    assert!(!diags.iter().any(|d| d.message.contains("violated")));

    // Back to the original: the model is valid again (apart from its violations).
    ws.open_document(&mm_path, &text);
    assert_eq!(ws.diagnostics(Some(&model_path)).len(), 5);

    // Closing the metamodel leaves the model without classes.
    ws.close_document(&mm_path);
    let diags = ws.diagnostics(Some(&model_path));
    assert!(diags.iter().any(|d| d.message.contains("no metamodel with nsURI")), "{diags:#?}");
}

#[test]
fn evaluates_with_contracts() {
    let (ws, _, model_path) = university();
    let ada = ObjectRef { resource: Some(model_path), fragment: "ada".into() };
    let r = ws.evaluate("self.mayEnrol(Course.allInstances()->any(code = 'IN0001'))", Some(&ada)).unwrap();
    assert_eq!(r.display, "true");
    let failed: Vec<(&str, &str, ResultStatus)> =
        r.contracts.iter().map(|c| (c.constraint.name.as_str(), c.call.as_str(), c.status)).collect();
    assert_eq!(
        failed,
        [
            ("NotEnrolled", "mayEnrol(Course #prog1)", ResultStatus::Violated),
            ("FreeSeat", "hasSeat()", ResultStatus::Violated),
        ]
    );
    assert_eq!(r.contracts[0].message.as_deref(), Some("already enrolled in IN0001"));
    let r = ws.evaluate("self.credits", Some(&ObjectRef { resource: None, fragment: "carla".into() })).unwrap();
    assert_eq!((r.display.as_str(), r.ty.as_str()), ("44", "Integer"));
}

#[test]
fn browses_textual_metamodels_and_models() {
    let (ws, mm_path, model_path) = university();
    let packages = ws.packages();
    let university = packages.iter().find(|p| p.name == "university").unwrap();
    assert_eq!(university.resource, mm_path);
    let course = university.classifiers.iter().find(|c| c.name == "Course").unwrap();
    let has_seat = course.operations.iter().find(|o| o.name == "hasSeat").unwrap();
    assert_eq!(has_seat.body.as_deref(), Some("enrolments->size() <= capacity"));
    assert_eq!(has_seat.postconditions[0].name, "FreeSeat");
    let student = university.classifiers.iter().find(|c| c.name == "Student").unwrap();
    let credits = student.features.iter().find(|f| f.name == "credits").unwrap();
    assert_eq!(credits.derivation.as_deref(), Some("enrolments.course.credits->sum()"));

    let models = ws.models();
    let model = models.iter().find(|m| m.uri == model_path).unwrap();
    assert_eq!(model.roots[0].label, "University 'Technical University'");
    assert_eq!(model.roots[0].range.unwrap().start.line, 14);
}

#[test]
fn completes_oclinecore() {
    let (mut ws, mm_path, _) = university();
    let (_, text) = example("university/university.oclinecore");

    // Inside an expression: features of the enclosing class, and the operation's parameters.
    let edited = text.replace("body: course.prerequisites", "body: course.prerequisites->forAll(p | p.");
    ws.open_document(&mm_path, &edited);
    let list = ws.complete(&mm_path, position_after(&edited, "forAll(p | p.")).unwrap();
    assert!(labels(&list).contains(&"capacity"), "{:?}", labels(&list));
    let edited = text.replace("body: course.prerequisites", "body: cour");
    ws.open_document(&mm_path, &edited);
    let list = ws.complete(&mm_path, position_after(&edited, "body: cour")).unwrap();
    assert!(labels(&list).contains(&"course"), "{:?}", labels(&list));
    let edited = text.replace("result implies course", "result implies res");
    ws.open_document(&mm_path, &edited);
    let list = ws.complete(&mm_path, position_after(&edited, "implies res")).unwrap();
    assert_eq!(labels(&list), ["result"]);

    // In the structure: member keywords, type names and qualifiers.
    let edited = text.replace("    attribute semester : Integer;\n", "    attribute semester : Integer;\n    at\n");
    ws.open_document(&mm_path, &edited);
    let list = ws.complete(&mm_path, position_after(&edited, "    at")).unwrap();
    assert_eq!(labels(&list), ["attribute"]);
    let edited = text.replace("attribute semester : Integer;", "attribute semester : ");
    ws.open_document(&mm_path, &edited);
    let list = ws.complete(&mm_path, position_after(&edited, "semester : ")).unwrap();
    for expected in ["Integer", "String", "Course", "Kind", "EInt", "OrderedSet"] {
        assert!(labels(&list).contains(&expected), "{expected} in {:?}", labels(&list));
    }
    let edited = text.replace("property prerequisites : Course[*];", "property prerequisites : Course[*] { ord");
    ws.open_document(&mm_path, &edited);
    let list = ws.complete(&mm_path, position_after(&edited, "{ ord")).unwrap();
    assert_eq!(labels(&list), ["ordered"]);
}

#[test]
fn completes_hutn() {
    let (mut ws, _, model_path) = university();
    let (_, text) = example("university/university.hutn");

    // A slot name in an object: the class's features not set yet.
    let edited = text.replace("name: \"Brian\"", "name: \"Brian\"\n        se");
    ws.open_document(&model_path, &edited);
    let list = ws.complete(&model_path, position_after(&edited, "\n        se")).unwrap();
    assert_eq!(labels(&list), ["semester"]);
    assert_eq!(list.items[0].insert.as_deref(), Some("semester: "));

    // Values: references to identified objects of the feature's type, and enum literals.
    let edited = text.replace("prerequisites: Course \"prog1\"\n      },", "prerequisites: \n      },");
    ws.open_document(&model_path, &edited);
    let list = ws.complete(&model_path, position_after(&edited, "prerequisites: ")).unwrap();
    assert_eq!(labels(&list), ["Course \"prog1\"", "Course \"prog2\"", "Course \"algo\"", "Course \"thesis\""]);
    let edited = text.replace("kind: PROJECT", "kind: ");
    ws.open_document(&model_path, &edited);
    let list = ws.complete(&model_path, position_after(&edited, "kind: ")).unwrap();
    assert_eq!(labels(&list), ["LECTURE", "SEMINAR", "PROJECT"]);

    // Containment: classes that can be nested; the identifier inside a reference.
    let edited = text.replace("Enrolment { course: Course \"algo\" }", "Enr");
    ws.open_document(&model_path, &edited);
    let list = ws.complete(&model_path, position_after(&edited, "\"prog2\" },\n          Enr")).unwrap();
    assert_eq!(labels(&list), ["Enrolment"]);
    let edited = text.replace("prerequisites: Course \"prog1\"\n      },", "prerequisites: Course \"pr\n      },");
    ws.open_document(&model_path, &edited);
    let list = ws.complete(&model_path, position_after(&edited, "Course \"pr")).unwrap();
    assert_eq!(labels(&list), ["prog1", "prog2"]);

    // A package block: its concrete classes.
    let edited = text.replace("university {\n", "university {\n  Stu\n");
    ws.open_document(&model_path, &edited);
    let list = ws.complete(&model_path, position_after(&edited, "  Stu")).unwrap();
    assert_eq!(labels(&list), ["Student"]);
}

#[test]
fn hovers() {
    let (ws, mm_path, model_path) = university();
    let (_, model) = example("university/university.hutn");
    let hover = ws.hover(&model_path, position_after(&model, "semester")).unwrap();
    assert_eq!(hover.contents, "attribute Student::semester : Integer");
    let hover = ws.hover(&model_path, position_after(&model, "prerequisites: Course \"pr")).unwrap();
    assert_eq!(hover.contents, "→ Course #prog1");
    let hover = ws.hover(&model_path, position_after(&model, "Student \"ad")).unwrap();
    assert_eq!(hover.contents, "class university::Student");

    let (_, metamodel) = example("university/university.oclinecore");
    let hover = ws.hover(&mm_path, position_after(&metamodel, "derivation: enrolments.course.cred")).unwrap();
    assert_eq!(hover.contents, "Bag(Integer)");
    let hover = ws.hover(&mm_path, position_after(&metamodel, "operation hasSe")).unwrap();
    assert_eq!(hover.contents, "operation Course::hasSeat() : Boolean");
    let hover = ws.hover(&mm_path, position_after(&metamodel, "property prerequisites : Cou")).unwrap();
    assert_eq!(hover.contents, "class university::Course");
}

#[test]
fn finds_the_class_at_a_position() {
    let (ws, mm_path, model_path) = university();
    let (_, metamodel) = example("university/university.oclinecore");
    let class_at = |ws: &Workspace, marker: &str| ws.class_at(&mm_path, position_after(&metamodel, marker));
    assert_eq!(class_at(&ws, "class Univ").as_deref(), Some("university::University"));
    // In a feature, an operation body and a constraint of the class.
    assert_eq!(class_at(&ws, "attribute semes").as_deref(), Some("university::Student"));
    assert_eq!(class_at(&ws, "body: enrolments->size() <=").as_deref(), Some("university::Course"));
    assert_eq!(class_at(&ws, "invariant NotOverbooked(").as_deref(), Some("university::Course"));
    // Enumerations are not classes; nor is the package header or a comment before it.
    assert_eq!(class_at(&ws, "literal SEMI"), None);
    assert_eq!(class_at(&ws, "package univ"), None);
    assert_eq!(class_at(&ws, "-- A course"), None);
    // Only OCLinEcore documents define classes.
    assert_eq!(ws.class_at(&model_path, Position { line: 0, column: 0 }), None);

    // Through the protocol, with an updated text.
    let mut server = rpc::Server::new();
    server.call("openDocument", json!({ "name": mm_path, "text": metamodel })).unwrap();
    let edited =
        metamodel.replace("class Student {", "class Person {\n    attribute born : Integer;\n  }\n\n  class Student {");
    let position = position_after(&edited, "attribute bo");
    let class = server.call("classAt", json!({ "name": mm_path, "text": edited, "position": position })).unwrap();
    assert_eq!(class, json!("university::Person"));
    let outside = server.call("classAt", json!({ "name": mm_path, "position": { "line": 0, "column": 0 } })).unwrap();
    assert_eq!(outside, json!(null));
}

#[test]
fn exports_and_converts() {
    let (ws, mm_path, model_path) = university();
    let ecore = ws.export_document(&mm_path, None).unwrap();
    assert_eq!(ecore.format, "ecore");
    let xmi = ws.export_document(&model_path, None).unwrap();
    assert_eq!(xmi.format, "xmi");

    // The exports load into a fresh workspace and validate the same.
    let mut copy = Workspace::new();
    copy.load_ecore_str(&ecore.text, "university.ecore").unwrap();
    copy.load_model_str(&xmi.text, "university.xmi").unwrap();
    let report = copy.validate(&ValidateOptions::default()).unwrap();
    assert_eq!(report.summary.violated, 5);

    // An XMI model typed by an OCLinEcore document is reloaded when the document changes.
    let mut ws = ws;
    ws.load_model_str(&xmi.text, "copy.xmi").unwrap();
    let (_, text) = example("university/university.oclinecore");
    ws.open_document(&mm_path, &format!("{text}\n"));
    let report = ws.validate(&ValidateOptions::default()).unwrap();
    assert_eq!(report.summary.violated, 10);

    // .ecore files and XMI models convert to text.
    let (library_ecore, _) = example("library/library.ecore");
    let text = ws.convert_file(&library_ecore, Language::OclInEcore).unwrap();
    assert!(
        text.contains("invariant SufficientCopies: library.loans->select(book = self)->size() <= copies;"),
        "{text}"
    );
    let mut fresh = Workspace::new();
    assert_eq!(fresh.open_document("library.oclinecore", &text), Vec::<Diagnostic>::new());
    let (library_xmi, _) = example("library/library.xmi");
    let hutn = fresh.convert_file(&library_xmi, Language::Hutn).unwrap();
    assert!(hutn.contains("Book \"Dune\" {"), "{hutn}");
    assert!(fresh.open_document("library.hutn", &hutn).iter().all(|d| d.message.contains("violated")));
    assert_eq!(fresh.validate(&ValidateOptions::default()).unwrap().summary.violated, 1);
}

#[test]
fn json_rpc_for_textual_documents() {
    let mut server = rpc::Server::new();
    let (mm_path, mm_text) = example("university/university.oclinecore");
    let r = server
        .call("openDocument", json!({ "name": "metamodel.txt", "text": mm_text, "language": "oclinecore" }))
        .unwrap();
    assert_eq!(r["language"], "oclinecore");
    assert_eq!(r["diagnostics"], json!([]));
    let (_, model_text) = example("university/university.hutn");
    let r = server.call("openDocument", json!({ "name": "model.hutn", "text": model_text })).unwrap();
    assert_eq!(r["language"], "hutn");
    assert_eq!(r["diagnostics"].as_array().unwrap().len(), 5);
    let report = server.call("validate", json!({ "contracts": false })).unwrap();
    assert_eq!(report["summary"]["violated"], 3);
    let report = server.call("validate", json!({})).unwrap();
    let post = report["constraints"].as_array().unwrap().iter().find(|c| c["name"] == "FreeSeat").unwrap();
    assert_eq!(post["kind"], "postcondition");
    assert_eq!(post["operation"], "hasSeat() : Boolean");
    let exported = server.call("exportDocument", json!({ "name": "metamodel.txt" })).unwrap();
    assert!(exported["text"].as_str().unwrap().contains("nsURI=\"http://www.example.org/university/1.0\""));
    let converted =
        server.call("convertFile", json!({ "path": mm_path.replace(".oclinecore", ".hutn"), "to": "hutn" }));
    assert!(converted.is_err(), "a HUTN file is not XMI");
    let completion =
        server.call("complete", json!({ "name": "model.hutn", "position": { "line": 25, "column": 8 } })).unwrap();
    assert!(completion["items"].is_array());
}

/// The byte offset of a position.
fn offset_of(text: &str, p: Position) -> usize {
    let line_start = text.split_inclusive('\n').take(p.line as usize).map(str::len).sum::<usize>();
    line_start + text[line_start..].chars().take(p.column as usize).map(char::len_utf8).sum::<usize>()
}

/// Applies the edits of `createObject` and returns the new text and the cursor offset.
fn apply(text: &str, edit: &DocumentEdit) -> (String, usize) {
    let mut out = text.to_string();
    for e in edit.edits.iter().rev() {
        out.replace_range(offset_of(text, e.range.start)..offset_of(text, e.range.end), &e.text);
    }
    let cursor = offset_of(&out, edit.cursor);
    (out, cursor)
}

fn roots(ws: &Workspace, resource: &str) -> Vec<ObjectInfo> {
    ws.models().into_iter().find(|m| m.uri == resource).map(|m| m.roots).unwrap_or_default()
}

#[test]
fn creates_root_objects() {
    let (mut ws, _, model_path) = university();
    let text = ws.document_text(&model_path).unwrap().to_string();
    let edit = ws.create_object(&model_path, "university::Course", None).unwrap();
    let (new, cursor) = apply(&text, &edit);
    assert!(new.ends_with("      }\n  }\n  Course {  }\n}\n"), "{new}");
    assert_eq!(&new[cursor - 2..cursor + 2], "{  }");
    ws.open_document(&model_path, &new);
    let classes: Vec<String> = roots(&ws, &model_path).into_iter().map(|o| o.class).collect();
    assert_eq!(classes, ["university::University", "university::Course"]);

    // A new document gets a @Spec entry and a package block.
    let name = "new.hutn";
    ws.open_document(name, "// A model.\n");
    let edit = ws.create_object(name, "university::University", None).unwrap();
    let (new, cursor) = apply("// A model.\n", &edit);
    assert_eq!(
        new,
        "// A model.\n\n@Spec {\n  metamodel \"university\" {\n    nsUri: \"http://www.example.org/university/1.0\"\n  }\n}\n\nuniversity {\n  University {  }\n}\n"
    );
    assert_eq!(&new[cursor - 1..cursor + 1], "  ");
    ws.open_document(name, &new);
    assert_eq!(roots(&ws, name).len(), 1);

    // The blank line of an empty package block is replaced.
    let template = "university {\n  \n}\n";
    ws.open_document(name, template);
    let (new, _) = apply(template, &ws.create_object(name, "university::University", None).unwrap());
    assert_eq!(new, "university {\n  University {  }\n}\n");

    assert!(matches!(ws.create_object(name, "university::Nothing", None), Err(Error::NotFound(_))));
    assert!(matches!(ws.create_object(name, "university::Kind", None), Err(Error::NotFound(_))));
}

#[test]
fn creates_contained_objects() {
    let (mut ws, _, model_path) = university();
    let tum = roots(&ws, &model_path)[0].fragment.clone();
    let options =
        ws.containment_options(&ObjectRef { resource: Some(model_path.clone()), fragment: tum.clone() }).unwrap();
    let features: Vec<(&str, &[String])> = options.iter().map(|o| (o.feature.as_str(), &o.classes[..])).collect();
    assert_eq!(
        features,
        [("courses", &["university::Course".to_string()][..]), ("students", &["university::Student".to_string()][..])]
    );
    assert!(options.iter().all(|o| o.many && !o.full));

    // Another value of a slot whose values are on lines of their own.
    let text = ws.document_text(&model_path).unwrap().to_string();
    let edit = ws.create_object(&model_path, "university::Course", Some((&tum, "courses"))).unwrap();
    let (new, cursor) = apply(&text, &edit);
    assert!(
        new.contains("prerequisites: Course \"algo\", Course \"prog2\"\n      },\n      Course {  }\n    students:"),
        "{new}"
    );
    assert_eq!(&new[cursor - 2..cursor + 2], "{  }");
    ws.open_document(&model_path, &new);
    let tum_info = &roots(&ws, &model_path)[0];
    assert_eq!(tum_info.children.iter().filter(|c| c.class == "university::Course").count(), 5);

    // A new slot in a one-line object.
    let name = "new.hutn";
    let text = "university {\n  University \"u\" { name: \"U\" }\n}\n";
    ws.open_document(name, text);
    let u = roots(&ws, name)[0].fragment.clone();
    let (new, _) = apply(text, &ws.create_object(name, "university::Student", Some((&u, "students"))).unwrap());
    assert_eq!(new, "university {\n  University \"u\" { name: \"U\" students: Student {  } }\n}\n");
    ws.open_document(name, &new);
    assert_eq!(roots(&ws, name)[0].children.len(), 1);
    // Further values on the same line.
    let (new2, _) = apply(&new, &ws.create_object(name, "university::Student", Some((&u, "students"))).unwrap());
    assert!(new2.contains("students: Student {  }, Student {  } }"), "{new2}");

    // Only containment features of the right type.
    let err = ws.create_object(name, "university::Course", Some((&u, "students")));
    assert!(matches!(err, Err(Error::Invalid(_))), "{err:?}");
    let err = ws.create_object(name, "university::Course", Some((&u, "name")));
    assert!(matches!(err, Err(Error::Invalid(_))), "{err:?}");

    // Over JSON-RPC.
    let mut server = rpc::Server::new();
    let (_, mm_text) = example("university/university.oclinecore");
    server.call("openDocument", json!({ "name": "mm.oclinecore", "text": mm_text })).unwrap();
    server.call("openDocument", json!({ "name": "m.hutn", "text": "" })).unwrap();
    let r = server
        .call("createObject", json!({ "name": "m.hutn", "text": text, "class": "university::University" }))
        .unwrap();
    assert_eq!(r["edits"].as_array().unwrap().len(), 1);
    assert_eq!(r["cursor"], json!({ "line": 2, "column": 15 }));
    let r = server.call("containmentOptions", json!({ "object": { "resource": "m.hutn", "fragment": u } })).unwrap();
    assert_eq!(r[0]["type"], "university::Course");
}

#[test]
fn xmi_documents_are_checked_live() {
    let mut ws = Workspace::new();
    let (ocl_path, ocl_text) = example("library/library.ocl");
    ws.open_document(&ocl_path, &ocl_text);
    let (xmi_path, xmi_text) = example("library/library.xmi");
    // The metamodel comes from xsi:schemaLocation; the constraints from the embedded OCL and
    // the Complete OCL document.
    let diags = ws.open_document(&xmi_path, &xmi_text);
    assert_eq!(ws.document_language(&xmi_path), Some(Language::Xmi));
    let violations: Vec<(&str, u32)> = diags.iter().map(|d| (d.message.as_str(), d.range.start.line)).collect();
    assert_eq!(violations.len(), 3, "{violations:#?}");
    assert!(
        diags.iter().all(|d| d.severity == Severity::Error && d.origin == Origin::Document { name: xmi_path.clone() })
    );
    // Located on the violating element's tag name.
    let bob = xmi_text.lines().position(|l| l.contains("name=\"Bob\"")).unwrap() as u32;
    let d = diags.iter().find(|d| d.range.start.line == bob).expect("a violation on Bob");
    let line = xmi_text.lines().nth(bob as usize).unwrap();
    let name = &line[d.range.start.column as usize..d.range.end.column as usize];
    assert_eq!(name, "members");
    // Objects of XMI documents are part of the models, with their definitions.
    let objects = roots(&ws, &xmi_path);
    assert_eq!(objects.len(), 1);
    assert!(objects[0].range.is_some());

    // Edits: a load problem replaces the violations; a malformed document reports one error.
    let broken = xmi_text.replace("copies=\"2\"", "copies=\"two\"");
    let diags = ws.open_document(&xmi_path, &broken);
    assert_eq!(diags.len(), 1, "{diags:#?}");
    assert!(diags[0].message.contains("copies"), "{}", diags[0].message);
    let diags = ws.open_document(&xmi_path, &xmi_text.replace("</library:Library>", ""));
    assert_eq!(diags.len(), 1, "{diags:#?}");
    assert!(roots(&ws, &xmi_path).is_empty());
    let diags = ws.open_document(&xmi_path, &xmi_text.replace("name=\"Carol\"", "name=\"Carol\" nickname=\"C\""));
    assert!(diags.iter().any(|d| d.message.contains("no feature 'nickname'")), "{diags:#?}");
}

#[test]
fn reloads_documents_of_several_metamodels() {
    let mut ws = Workspace::new();
    let (mm_path, mm_text) = example("university/university.oclinecore");
    ws.open_document(&mm_path, &mm_text);
    let (ocl_path, ocl_text) = example("library/library.ocl");
    ws.open_document(&ocl_path, &ocl_text);
    assert!(ws.reload().is_empty());
    assert_eq!(ws.packages().len(), 2);
}

#[test]
fn generates_xml_schemas() {
    let (ws, _, _) = university();
    let schemas = ws.schemas();
    let names: Vec<&str> = schemas.iter().map(|s| s.file_name.as_str()).collect();
    assert!(
        names.contains(&"XMI.xsd") && names.contains(&"university.xsd") && names.contains(&"ecore.xsd"),
        "{names:?}"
    );
    let university = schemas.iter().find(|s| s.file_name == "university.xsd").unwrap();
    assert_eq!(university.ns_uri.as_deref(), Some("http://www.example.org/university/1.0"));
    assert!(university.text.contains("name=\"Course\""));
}

#[test]
fn generates_code_for_a_textual_metamodel() {
    let (mut ws, mm_path, _) = university();
    // A Complete OCL document adds a definition and an invariant.
    ws.open_document("extra.ocl", "package university context Student def: initial : String = name.substring(1, 1) inv Named: name.size() > 0 endpackage");
    let request = CodeRequest { target: "python".into(), resource: Some(mm_path.clone()), ..Default::default() };
    let code = ws.generate_code(&request).unwrap();
    assert_eq!(code.packages, ["university"]);
    assert!(code.problems.is_empty(), "{:?}", code.problems);
    let paths: Vec<&str> = code.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        ["university/_runtime.py", "university/_model.py", "university/university.py", "university/__init__.py"]
    );
    let model = &code.files[1].text;
    for expected in ["class Student(_rt.EObject):", "def check_named(self) -> bool:", "def initial(self) -> str:"] {
        assert!(model.contains(expected), "{expected} missing");
    }
    // Without the document's definitions.
    let request = CodeRequest { documents: Some(Vec::new()), ..request };
    let code = ws.generate_code(&request).unwrap();
    assert!(!code.files[1].text.contains("check_named"));

    // Through the protocol: the targets, a crate, and errors.
    let mut server = rpc::Server::new();
    let targets = server.call("codeTargets", json!(null)).unwrap();
    let ids: Vec<&str> = targets.as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["python", "rust"]);
    let (_, text) = example("university/university.oclinecore");
    server.call("openDocument", json!({ "name": "u.oclinecore", "text": text })).unwrap();
    let code =
        server.call("generateCode", json!({ "target": "rust", "resource": "u.oclinecore", "name": "uni" })).unwrap();
    let files: Vec<&str> = code["files"].as_array().unwrap().iter().map(|f| f["path"].as_str().unwrap()).collect();
    assert!(files.contains(&"uni/Cargo.toml") && files.contains(&"uni/src/university.rs"), "{files:?}");
    let error = server.call("generateCode", json!({ "target": "cobol", "resource": "u.oclinecore" })).unwrap_err();
    assert_eq!(error.code, rpc::codes::INVALID_PARAMS);
    let error = server.call("generateCode", json!({ "target": "rust", "resource": "nope.ecore" })).unwrap_err();
    assert_eq!(error.code, rpc::codes::NOT_FOUND);
}
