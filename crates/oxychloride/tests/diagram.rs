//! Metamodel documents as diagrams (`metamodelDiagram`) and diagram edits as text edits
//! (`editMetamodel`), for OCLinEcore documents and `.ecore` files.

use oxychloride::*;
use serde_json::json;

const EXAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");

fn example(file: &str) -> (String, String) {
    let path = format!("{EXAMPLES}/{file}");
    let text = std::fs::read_to_string(&path).unwrap();
    (path, text)
}

/// Applies edits whose ranges refer to `text`.
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

fn edit(value: serde_json::Value) -> MetamodelEdit {
    serde_json::from_value(value).unwrap()
}

/// An OCLinEcore document being edited through diagram edits.
struct TextualMetamodel {
    ws: Workspace,
    name: String,
}

impl TextualMetamodel {
    fn university() -> Self {
        let (name, text) = example("university/university.oclinecore");
        let mut ws = Workspace::new();
        assert!(ws.open_document(&name, &text).is_empty());
        TextualMetamodel { ws, name }
    }

    fn text(&self) -> &str {
        self.ws.document_text(&self.name).unwrap()
    }

    /// Makes an edit and opens the edited text; returns the id the edit names.
    fn edit(&mut self, e: serde_json::Value) -> Option<String> {
        let result = self.ws.edit_metamodel(&self.name, None, &edit(e)).unwrap();
        let text = apply(self.text(), &result.edits);
        self.ws.open_document(&self.name.clone(), &text);
        result.element
    }

    fn error(&self, e: serde_json::Value) -> String {
        self.ws.edit_metamodel(&self.name, None, &edit(e)).unwrap_err().to_string()
    }

    fn diagram(&self) -> MetamodelDiagram {
        self.ws.metamodel_diagram(&self.name, None).unwrap()
    }

    fn errors(&self) -> Vec<String> {
        let diags = self.ws.diagnostics(Some(&self.name));
        diags.into_iter().filter(|d| d.severity == Severity::Error).map(|d| d.message).collect()
    }
}

fn classifier<'a>(d: &'a MetamodelDiagram, id: &str) -> &'a DiagramClassifier {
    d.classifiers.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("no classifier {id}"))
}

fn names(features: &[DiagramFeature]) -> Vec<&str> {
    features.iter().map(|f| f.name.as_str()).collect()
}

#[test]
fn draws_an_oclinecore_metamodel() {
    let m = TextualMetamodel::university();
    let d = m.diagram();
    assert_eq!(d.syntax, MetamodelSyntax::OclInEcore);
    assert_eq!(d.packages.len(), 1);
    assert_eq!(d.packages[0].ns_uri, "http://www.example.org/university/1.0");
    let ids: Vec<&str> = d.classifiers.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "university::University",
            "university::Kind",
            "university::Course",
            "university::Student",
            "university::Enrolment"
        ]
    );
    let course = classifier(&d, "university::Course");
    assert_eq!(names(&course.attributes), ["code", "title", "kind", "credits", "capacity"]);
    assert_eq!(names(&course.references), ["prerequisites", "enrolments"]);
    assert_eq!(course.attributes[2].type_name, "Kind");
    assert_eq!(course.attributes[2].ty.as_deref(), Some("university::Kind"));
    assert!(course.attributes[0].is_id);
    let enrolments = &course.references[1];
    assert_eq!((enrolments.lower, enrolments.upper), (0, -1));
    assert_eq!(enrolments.opposite.as_deref(), Some("university::Enrolment.course"));
    assert_eq!(course.operations.iter().map(|o| o.name.as_str()).collect::<Vec<_>>(), ["participants", "hasSeat"]);
    assert_eq!(course.operations[0].type_name.as_deref(), Some("Student"));
    assert!(course.operations[0].has_body);
    assert_eq!(course.invariants.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["NotOverbooked"]);
    // The class's definition, from `class Course {` to its closing brace.
    assert_eq!(course.range.start, Position { line: 21, column: 2 });
    let university = classifier(&d, "university::University");
    assert!(university.references.iter().all(|r| r.containment));
    let student = classifier(&d, "university::Student");
    let credits = student.attributes.iter().find(|a| a.name == "credits").unwrap();
    assert!(credits.derived && !credits.changeable && credits.volatile && credits.transient);
    let kind = classifier(&d, "university::Kind");
    assert_eq!(kind.kind, ClassifierKind::Enum);
    assert_eq!(
        kind.literals.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(),
        ["university::Kind.LECTURE", "university::Kind.SEMINAR", "university::Kind.PROJECT"]
    );
    assert!(d.external.is_empty());
    assert!(d.types.iter().any(|t| t.id == "ecore::EString" && t.label == "String"));
    assert!(d.types.iter().any(|t| t.id == "ecore::EInt" && t.label == "ecore::EInt"));
    assert!(d.types.iter().any(|t| t.id == "university::Course" && t.label == "Course"));
    assert!(d.problems.is_empty(), "{:?}", d.problems);
}

#[test]
fn reports_problems_on_the_elements() {
    let mut m = TextualMetamodel::university();
    let text = m.text().replace("attribute title : String;", "attribute title : Strin;");
    m.ws.open_document(&m.name.clone(), &text);
    let d = m.diagram();
    let p = d.problems.iter().find(|p| p.severity == Severity::Error).unwrap();
    assert_eq!(p.element.as_deref(), Some("university::Course.title"), "{p:?}");
    let title = &classifier(&d, "university::Course").attributes[1];
    assert_eq!((title.type_name.as_str(), title.ty.as_deref()), ("Strin", None));
}

#[test]
fn adds_elements_to_an_oclinecore_metamodel() {
    let mut m = TextualMetamodel::university();
    assert_eq!(
        m.edit(json!({"kind": "addClassifier", "package": "university", "classifier": "class", "name": "Room"})),
        Some("university::Room".into())
    );
    assert!(m.text().ends_with(
        "    invariant ValidGrade: grade <> null implies (grade >= 1 and grade <= 5);\n  }\n\n  class Room;\n}\n"
    ));
    m.edit(json!({"kind": "addAttribute", "owner": "university::Room", "name": "seats", "type": "ecore::EBigInteger", "lower": 0, "upper": 1}));
    assert!(m.text().contains("\n  class Room {\n    attribute seats : Integer[?];\n  }\n}\n"), "{}", m.text());
    m.edit(json!({"kind": "addReference", "owner": "university::Course", "name": "room", "type": "university::Room", "lower": 0, "upper": 1, "opposite": "courses"}));
    assert!(
        m.text().contains("    property enrolments#course : Enrolment[*];\n    property room#courses : Room[?];\n")
    );
    assert!(m.text().contains("    attribute seats : Integer[?];\n    property courses#room : Course[?];\n  }"));
    m.edit(json!({"kind": "addReference", "owner": "university::University", "name": "rooms", "type": "university::Room", "upper": -1, "containment": true}));
    assert!(m.text().contains("    property rooms : Room[*] { composes };\n  }"));
    m.edit(json!({"kind": "addOperation", "owner": "university::Room", "name": "fits", "type": "ecore::EBoolean",
                  "parameters": [{"name": "course", "type": "university::Course"}, {"name": "extra", "type": "ecore::EBigInteger", "lower": 0, "upper": -1}]}));
    assert!(m.text().contains("    operation fits(course : Course, extra : Integer[*]) : Boolean;\n  }"));
    m.edit(json!({"kind": "addLiteral", "owner": "university::Kind", "name": "WORKSHOP"}));
    assert!(m.text().contains("    literal PROJECT;\n    literal WORKSHOP;\n  }"));
    m.edit(json!({"kind": "addClassifier", "package": "university", "classifier": "abstractClass", "name": "Person"}));
    m.edit(json!({"kind": "addSuperType", "owner": "university::Student", "superType": "university::Person"}));
    assert!(m.text().contains("  class Student extends Person {\n"));
    m.edit(json!({"kind": "addClassifier", "package": "university", "classifier": "interface", "name": "Named"}));
    m.edit(json!({"kind": "addSuperType", "owner": "university::Student", "superType": "university::Named"}));
    assert!(m.text().contains("  class Student extends Person, Named {\n"));
    assert!(m.text().contains("  abstract class Person;\n\n  interface Named;\n}"));
    assert!(m.errors().is_empty(), "{:?}", m.errors());

    let d = m.diagram();
    let room = classifier(&d, "university::Room");
    assert_eq!(names(&room.attributes), ["seats"]);
    assert_eq!(room.references[0].opposite.as_deref(), Some("university::Course.room"));
    assert_eq!(room.operations[0].parameters.len(), 2);
    assert_eq!(classifier(&d, "university::Student").super_types, ["university::Person", "university::Named"]);
    assert!(classifier(&d, "university::Named").is_interface);

    assert!(
        m.error(json!({"kind": "addClassifier", "package": "university", "classifier": "class", "name": "Room"}))
            .contains("already")
    );
    assert!(
        m.error(
            json!({"kind": "addAttribute", "owner": "university::Room", "name": "x", "type": "university::Course"})
        )
        .contains("data type")
    );
    assert!(
        m.error(json!({"kind": "addSuperType", "owner": "university::Person", "superType": "university::Student"}))
            .contains("cyclic")
    );
    assert!(
        m.error(
            json!({"kind": "addAttribute", "owner": "university::Student", "name": "name", "type": "ecore::EString"})
        )
        .contains("already")
    );
}

#[test]
fn changes_elements_of_an_oclinecore_metamodel() {
    let mut m = TextualMetamodel::university();
    let renamed = m.edit(json!({"kind": "update", "element": "university::Course", "properties": {"name": "Lecture"}}));
    assert_eq!(renamed.as_deref(), Some("university::Lecture"));
    let text = m.text().to_string();
    assert!(text.contains("  class Lecture {\n"));
    assert!(text.contains("property courses : Lecture[*] { ordered, composes };"));
    assert!(text.contains("property prerequisites : Lecture[*];"));
    assert!(text.contains("operation passed(course : Lecture) : Boolean {"));
    assert!(text.contains("property course#enrolments : Lecture;"));
    assert!(!text.contains(": Course"));
    assert!(m.errors().is_empty(), "{:?}", m.errors());

    m.edit(json!({"kind": "update", "element": "university::Enrolment.student", "properties": {"name": "attendee"}}));
    assert!(m.text().contains("property enrolments#attendee : Enrolment[*] { composes };"));
    assert!(m.text().contains("property attendee#enrolments : Student;"));

    m.edit(json!({"kind": "update", "element": "university::Lecture.prerequisites",
                  "properties": {"lower": 0, "upper": 3, "unique": false, "ordered": true}}));
    assert!(m.text().contains("property prerequisites : Lecture[0..3] { ordered, !unique };"), "{}", m.text());
    m.edit(json!({"kind": "update", "element": "university::Lecture.prerequisites", "properties": {"upper": 1, "unique": true, "ordered": false}}));
    assert!(m.text().contains("property prerequisites : Lecture[?];"), "{}", m.text());
    m.edit(json!({"kind": "update", "element": "university::Student.credits", "properties": {"derived": false, "changeable": true, "transient": false, "volatile": false}}));
    assert!(m.text().contains("property credits : Integer {\n      derivation:"), "{}", m.text());
    m.edit(json!({"kind": "update", "element": "university::Enrolment.grade", "properties": {"type": "ecore::EString", "lower": 1, "upper": 1}}));
    assert!(m.text().contains("attribute grade : String;"));
    m.edit(json!({"kind": "update", "element": "university::University", "properties": {"isAbstract": true}}));
    assert!(m.text().contains("  abstract class University {"));
    m.edit(json!({"kind": "update", "element": "university::University", "properties": {"isAbstract": false, "isInterface": true}}));
    assert!(m.text().contains("  interface University {"));
    m.edit(json!({"kind": "update", "element": "university::Lecture.hasSeat()", "properties": {"type": "", "parameters": [{"name": "n", "type": "ecore::EBigInteger"}]}}));
    assert!(m.text().contains("    operation hasSeat(n : Integer) {"), "{}", m.text());
    m.edit(json!({"kind": "update", "element": "university::Kind.SEMINAR", "properties": {"name": "TUTORIAL"}}));
    assert!(m.text().contains("    literal TUTORIAL;"));
    let d = m.diagram();
    assert!(classifier(&d, "university::University").is_interface);
    assert!(classifier(&d, "university::Lecture").operations[1].type_name.is_none());
}

#[test]
fn deletes_elements_of_an_oclinecore_metamodel() {
    let mut m = TextualMetamodel::university();
    m.edit(json!({"kind": "delete", "elements": ["university::Enrolment"]}));
    let text = m.text().to_string();
    assert!(!text.contains("class Enrolment"));
    assert!(!text.contains(": Enrolment"));
    assert!(text.trim_end().ends_with("    invariant PrerequisitesMet:\n      enrolments->forAll(e | e.grade <> null or e.course.prerequisites->forAll(p | passed(p)));\n  }\n}"), "{text}");
    let d = m.diagram();
    assert_eq!(names(&classifier(&d, "university::Course").references), ["prerequisites"]);

    // A commented operation goes with its comment.
    let mut m = TextualMetamodel::university();
    m.edit(json!({"kind": "delete", "elements": ["university::Course.hasSeat()"]}));
    assert!(!m.text().contains("Deliberately off by one"));
    assert!(m.text().contains("    }\n\n    invariant NotOverbooked"), "{}", m.text());

    // Supertypes and opposites that refer to deleted elements are removed.
    let mut m = TextualMetamodel::university();
    m.edit(json!({"kind": "addClassifier", "package": "university", "classifier": "class", "name": "Person"}));
    m.edit(json!({"kind": "addSuperType", "owner": "university::Student", "superType": "university::Person"}));
    m.edit(json!({"kind": "delete", "elements": ["university::Person", "university::Enrolment.course"]}));
    assert!(m.text().contains("  class Student {\n"));
    assert!(m.text().contains("    property enrolments : Enrolment[*];\n"), "{}", m.text());
    assert!(!m.text().contains("Person"));
    m.edit(json!({"kind": "delete", "elements": ["university::Kind.SEMINAR"]}));
    assert!(m.text().contains("    literal LECTURE;\n    literal PROJECT;\n"));
    assert!(m.error(json!({"kind": "delete", "elements": ["university"]})).contains("root package"));
}

// ------------------------------------------------------------------ .ecore files

/// An `.ecore` file being edited through diagram edits (in memory).
struct EcoreFile {
    ws: Workspace,
    name: String,
    text: String,
}

impl EcoreFile {
    fn library() -> Self {
        let (name, text) = example("library/library.ecore");
        EcoreFile { ws: Workspace::new(), name, text }
    }

    fn edit(&mut self, e: serde_json::Value) -> Option<String> {
        let result = self.ws.edit_metamodel(&self.name, Some(&self.text), &edit(e)).unwrap();
        self.text = apply(&self.text, &result.edits);
        result.element
    }

    fn diagram(&self) -> MetamodelDiagram {
        self.ws.metamodel_diagram(&self.name, Some(&self.text)).unwrap()
    }
}

#[test]
fn draws_an_ecore_file() {
    let f = EcoreFile::library();
    let d = f.diagram();
    assert_eq!(d.syntax, MetamodelSyntax::Ecore);
    let book = classifier(&d, "library::Book");
    assert_eq!(names(&book.attributes), ["name", "isbn", "copies", "genre", "loanCount"]);
    assert_eq!(book.attributes[0].type_name, "EString");
    assert_eq!(names(&book.references), ["authors", "library"]);
    assert_eq!(book.references[1].opposite.as_deref(), Some("library::Library.books"));
    assert_eq!(book.invariants.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["SufficientCopies"]);
    assert_eq!(book.operations[0].name, "isAvailable");
    assert_eq!(classifier(&d, "library::Member").super_types, ["library::Person"]);
    assert!(classifier(&d, "library::Person").is_abstract);
    assert_eq!(classifier(&d, "library::Genre").literals.len(), 3);
    assert!(d.types.iter().any(|t| t.id == "ecore::EString" && t.label == "EString"));
    // The workspace is not changed.
    assert!(f.ws.packages().is_empty());
}

#[test]
fn edits_an_ecore_file() {
    let mut f = EcoreFile::library();
    f.edit(json!({"kind": "addClassifier", "package": "library", "classifier": "class", "name": "Shelf"}));
    assert!(f.text.contains("    <eLiterals name=\"HISTORY\" value=\"2\"/>\n  </eClassifiers>\n  <eClassifiers xsi:type=\"ecore:EClass\" name=\"Shelf\"/>\n</ecore:EPackage>"));
    f.edit(json!({"kind": "addAttribute", "owner": "library::Shelf", "name": "label", "type": "ecore::EString"}));
    assert!(f.text.contains("  <eClassifiers xsi:type=\"ecore:EClass\" name=\"Shelf\">\n    <eStructuralFeatures xsi:type=\"ecore:EAttribute\" name=\"label\" eType=\"ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EString\"/>\n  </eClassifiers>"), "{}", f.text);
    f.edit(json!({"kind": "addReference", "owner": "library::Library", "name": "shelves", "type": "library::Shelf", "upper": -1, "containment": true, "opposite": "library"}));
    assert!(f.text.contains("<eStructuralFeatures xsi:type=\"ecore:EReference\" name=\"shelves\" upperBound=\"-1\" eType=\"#//Shelf\" containment=\"true\" eOpposite=\"#//Shelf/library\"/>\n  </eClassifiers>"));
    assert!(f.text.contains("<eStructuralFeatures xsi:type=\"ecore:EReference\" name=\"library\" eType=\"#//Library\" eOpposite=\"#//Library/shelves\"/>"));
    f.edit(json!({"kind": "addSuperType", "owner": "library::Shelf", "superType": "library::Person"}));
    assert!(f.text.contains("name=\"Shelf\" eSuperTypes=\"#//Person\">"));
    f.edit(json!({"kind": "addLiteral", "owner": "library::Genre", "name": "POETRY"}));
    assert!(f.text.contains("<eLiterals name=\"HISTORY\" value=\"2\"/>\n    <eLiterals name=\"POETRY\" value=\"3\"/>"));
    f.edit(json!({"kind": "addOperation", "owner": "library::Shelf", "name": "holds", "type": "ecore::EBoolean", "lower": 0, "upper": 1,
                  "parameters": [{"name": "book", "type": "library::Book", "lower": 0}]}));
    assert!(f.text.contains("    <eOperations name=\"holds\" eType=\"ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EBoolean\">\n      <eParameters name=\"book\" eType=\"#//Book\"/>\n    </eOperations>\n    <eStructuralFeatures xsi:type=\"ecore:EAttribute\" name=\"label\""), "{}", f.text);
    let d = f.diagram();
    let shelf = classifier(&d, "library::Shelf");
    assert_eq!(names(&shelf.attributes), ["label"]);
    assert_eq!(shelf.super_types, ["library::Person"]);
    assert_eq!(shelf.references[0].opposite.as_deref(), Some("library::Library.shelves"));

    // Renaming rewrites the references to the element.
    f.edit(json!({"kind": "update", "element": "library::Book", "properties": {"name": "Title"}}));
    assert!(!f.text.contains("#//Book"));
    assert!(f.text.contains("eType=\"#//Title\" containment=\"true\" eOpposite=\"#//Title/library\""));
    f.edit(json!({"kind": "update", "element": "library::Title.library", "properties": {"name": "owner"}}));
    assert!(f.text.contains("eOpposite=\"#//Title/owner\""));
    f.edit(json!({"kind": "update", "element": "library::Loan.book", "properties": {"lower": 0, "upper": -1, "ordered": false, "containment": true}}));
    assert!(f.text.contains("<eStructuralFeatures xsi:type=\"ecore:EReference\" name=\"book\" ordered=\"false\" upperBound=\"-1\" eType=\"#//Title\" containment=\"true\"/>"), "{}", f.text);
    f.edit(json!({"kind": "update", "element": "library::Person", "properties": {"isAbstract": false}}));
    assert!(f.text.contains("<eClassifiers xsi:type=\"ecore:EClass\" name=\"Person\">"));
    let d = f.diagram();
    assert!(d.problems.is_empty(), "{:?}", d.problems);
    assert_eq!(classifier(&d, "library::Library").references[0].ty.as_deref(), Some("library::Title"));

    // Deleting a class deletes the references to it and removes it from supertypes.
    f.edit(json!({"kind": "delete", "elements": ["library::Person"]}));
    assert!(!f.text.contains("Person"));
    assert!(f.text.contains("<eClassifiers xsi:type=\"ecore:EClass\" name=\"Author\"/>"));
    assert!(f.text.contains("<eClassifiers xsi:type=\"ecore:EClass\" name=\"Member\">"));
    f.edit(json!({"kind": "delete", "elements": ["library::Title"]}));
    assert!(!f.text.contains("name=\"Title\"") && !f.text.contains("Loan/book"), "{}", f.text);
    assert!(f.text.contains("name=\"Library\">\n    <eStructuralFeatures xsi:type=\"ecore:EAttribute\" name=\"name\""));
    let d = f.diagram();
    assert_eq!(names(&classifier(&d, "library::Library").references), ["members", "loans", "authors", "shelves"]);
    // Parameters keep their (now unknown) type, which is reported on the operation.
    let problem = &d.problems[0];
    assert!(problem.message.contains("cannot resolve reference '#//Title'"), "{problem:?}");
    assert_eq!(problem.element.as_deref(), Some("library::Shelf.holds()"));
}

#[test]
fn serves_diagrams_over_json_rpc() {
    let (name, text) = example("university/university.oclinecore");
    let mut server = rpc::Server::new();
    let d = server.call("metamodelDiagram", json!({"name": name, "text": text})).unwrap();
    assert_eq!(d["syntax"], "oclinecore");
    assert_eq!(d["classifiers"][2]["references"][1]["opposite"], "university::Enrolment.course");
    let r = server
        .call(
            "editMetamodel",
            json!({"name": name, "edit": {"kind": "addClassifier", "package": "university", "classifier": "enum", "name": "Level"}}),
        )
        .unwrap();
    assert_eq!(r["element"], "university::Level");
    assert_eq!(r["edits"][0]["text"], "\n  enum Level;\n");
    let (ecore, ecore_text) = example("library/library.ecore");
    let d = server.call("metamodelDiagram", json!({"name": ecore, "text": ecore_text})).unwrap();
    assert_eq!(d["syntax"], "ecore");
    let e = server.call("metamodelDiagram", json!({"name": ecore, "text": "<ecore:EPackage"})).unwrap_err();
    assert_eq!(e.code, rpc::codes::LOAD_ERROR);
}
