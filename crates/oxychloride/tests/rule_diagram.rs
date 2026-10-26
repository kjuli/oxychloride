//! Henshin transformations as rule diagrams (`transformationDiagram`) and diagram edits as
//! text edits (`editTransformation`), for Henshin texts and `.henshin` files.

use oxychloride::*;
use serde_json::json;

const EXAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");

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

/// A transformation document of an example, edited through diagram edits.
struct Rules {
    ws: Workspace,
    name: String,
    /// How many lines the last edit replaced or inserted.
    changed: usize,
}

impl Rules {
    fn open(example: &str, file: &str) -> Self {
        let mut ws = Workspace::new();
        let mm = format!("{EXAMPLES}/{example}/{example}.oclinecore");
        assert!(ws.open_document(&mm, &std::fs::read_to_string(&mm).unwrap()).is_empty());
        let name = format!("{EXAMPLES}/{example}/{file}");
        let d = ws.open_document(&name, &std::fs::read_to_string(&name).unwrap());
        assert!(!d.iter().any(|d| d.severity == Severity::Error), "{d:#?}");
        Rules { ws, name, changed: 0 }
    }

    fn text(&self) -> &str {
        self.ws.document_text(&self.name).unwrap()
    }

    fn diagram(&self) -> TransformationDiagram {
        self.ws.transformation_diagram(&self.name).unwrap()
    }

    /// Makes an edit and opens the edited text; returns the id the edit names.
    fn edit(&mut self, e: serde_json::Value) -> Option<String> {
        let edit: TransformationEdit = serde_json::from_value(e.clone()).unwrap();
        let result = self.ws.edit_transformation(&self.name, &edit).unwrap_or_else(|err| panic!("{e}: {err}"));
        let text = apply(self.text(), &result.edits);
        self.changed = result
            .edits
            .iter()
            .map(|e| ((e.range.end.line - e.range.start.line) as usize).max(e.text.matches('\n').count()))
            .sum();
        let name = self.name.clone();
        let d = self.ws.open_document(&name, &text);
        assert!(!d.iter().any(|d| d.severity == Severity::Error), "{e}:\n{d:#?}\n{text}");
        result.element
    }

    fn error(&self, e: serde_json::Value) -> String {
        let edit: TransformationEdit = serde_json::from_value(e).unwrap();
        self.ws.edit_transformation(&self.name, &edit).unwrap_err().to_string()
    }

    fn rule(&self, id: &str) -> RuleDiagram {
        self.diagram().rules.into_iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no rule {id}"))
    }
}

fn node<'d>(rule: &'d RuleDiagram, id: &str) -> &'d RuleNodeDiagram {
    rule.nodes.iter().find(|n| n.id == id).unwrap_or_else(|| panic!("no node {id} in {:#?}", rule.nodes))
}

fn edge<'d>(rule: &'d RuleDiagram, id: &str) -> &'d RuleEdgeDiagram {
    rule.edges.iter().find(|e| e.id == id).unwrap_or_else(|| panic!("no edge {id} in {:#?}", rule.edges))
}

/// What a diagram shows, without where it is written: for comparing the diagrams of a text and
/// a `.henshin` file.
fn shape(d: &TransformationDiagram) -> Vec<String> {
    let mut out = Vec::new();
    for r in &d.rules {
        let params: Vec<String> = r.parameters.iter().map(|p| format!("{} {}:{:?}", p.kind, p.name, p.ty)).collect();
        out.push(format!(
            "rule {} ({}) {} {} {:?}",
            r.id,
            params.join(", "),
            r.check_dangling,
            r.injective_matching,
            r.conditions
        ));
        // (A model has the created nodes after the matched ones.)
        let mut nodes = Vec::new();
        for n in &r.nodes {
            let mut attrs: Vec<String> = n
                .attributes
                .iter()
                .map(|a| format!("{} {:?} {}={} set={}", a.id, a.action, a.name, a.value, a.set))
                .collect();
            attrs.sort();
            nodes.push(format!(
                "  node {} {:?} {} {:?} {:?} param={} {attrs:?}",
                n.id, n.action, n.class, n.multi, n.condition, n.parameter
            ));
        }
        nodes.sort();
        out.extend(nodes);
        let mut edges: Vec<String> = r
            .edges
            .iter()
            .map(|e| {
                format!("  edge {} {:?} {}->{} {:?} {:?}", e.id, e.action, e.source, e.target, e.multi, e.condition)
            })
            .collect();
        edges.sort();
        out.extend(edges);
        for m in &r.multi_rules {
            out.push(format!("  multi {} of {} {} {}", m.id, m.kernel, m.check_dangling, m.injective_matching));
        }
        for f in &r.formulas {
            out.push(format!("  formula {} {} {:?}", f.scope, f.formula, f.graphs));
        }
    }
    for u in &d.units {
        out.push(format!("unit {} {}", u.id, u.text));
    }
    out
}

#[test]
fn draws_rules_in_henshins_notation() {
    let rules = Rules::open("university", "university.henshin_text");
    let d = rules.diagram();
    assert_eq!(d.syntax, TransformationSyntax::Text);
    assert_eq!(d.imports, ["university"]);
    let names: Vec<&str> = d.rules.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["enrol", "drop", "grade", "createSeminar", "addSeats", "closeCourse"]);
    // enrol: preserved parameter nodes, a created node and a forbidden one.
    let enrol = &d.rules[0];
    let student = node(enrol, "enrol::student");
    assert_eq!((student.action, student.parameter, student.class.as_str()), (RuleAction::Preserve, true, "Student"));
    assert_eq!(student.class_id.as_deref(), Some("university::Student"));
    assert_eq!(node(enrol, "enrol::enrolment").action, RuleAction::Create);
    assert_eq!(node(enrol, "enrol::existing").action, RuleAction::Forbid);
    let e = edge(enrol, "enrol::student->enrolment:enrolments");
    assert_eq!(
        (e.action, e.source.as_str(), e.target.as_str()),
        (RuleAction::Create, "enrol::student", "enrol::enrolment")
    );
    assert_eq!(edge(enrol, "enrol::existing->course:course").action, RuleAction::Forbid);
    // The range selects the node's declaration.
    let text = rules.text();
    let line = text.lines().nth(node(enrol, "enrol::enrolment").range.start.line as usize).unwrap();
    assert_eq!(line.trim(), "create node enrolment:Enrolment");
    // Attributes: matched, set, created.
    let add_seats = rules.rule("addSeats");
    let course = node(&add_seats, "addSeats::course");
    let attrs: Vec<(&str, &str, bool)> =
        course.attributes.iter().map(|a| (a.name.as_str(), a.value.as_str(), a.set)).collect();
    assert_eq!(attrs, [("code", "code", false), ("capacity", "seats", false), ("capacity", "seats + extra", true)]);
    assert_eq!(course.attributes[2].id, "addSeats::course.capacity#2");
    let create_seminar = rules.rule("createSeminar");
    let seminar = node(&create_seminar, "createSeminar::course");
    assert_eq!(seminar.attributes.iter().find(|a| a.name == "kind").unwrap().value, "Kind.SEMINAR");
    assert!(seminar.attributes.iter().all(|a| a.action == RuleAction::Create));
    // A multi-rule's elements, with the kernel's nodes they connect to.
    let close = rules.rule("closeCourse");
    assert!(!close.check_dangling);
    assert_eq!(close.multi_rules.len(), 1);
    assert_eq!(
        (close.multi_rules[0].id.as_str(), close.multi_rules[0].kernel.as_str()),
        ("closeCourse/dropEnrolments", "closeCourse")
    );
    assert!(!close.multi_rules[0].check_dangling, "inherited from the kernel");
    let dropped = node(&close, "closeCourse/dropEnrolments::enrolment");
    assert_eq!((dropped.action, dropped.multi.as_deref()), (RuleAction::Delete, Some("closeCourse/dropEnrolments")));
    let e = edge(&close, "closeCourse/dropEnrolments::enrolment->course:course");
    assert_eq!(e.target, "closeCourse::course");
    // Units.
    let units: Vec<&str> = d.units.iter().map(|u| u.name.as_str()).collect();
    assert_eq!(units, ["enrolEverywhere", "gradeOrEnrol", "replaceBySeminar", "growCourse"]);
    let body = &d.units[0].body;
    assert_eq!(body[0].kind, "while");
    assert_eq!(body[0].children[0][0].target.as_deref(), Some("enrol"));
    assert_eq!(body[0].children[0][0].arguments, ["student", "course"]);
    assert_eq!(body[0].children[0][0].id, "enrolEverywhere@1.1.1");
    assert_eq!(d.units[1].body[0].kind, "priority");
    assert_eq!(d.units[1].body[0].children.len(), 2);
    assert_eq!(d.units[3].body[0].count.as_deref(), Some("3"));
    assert_eq!(d.units[3].text, "for (3) {\n\taddSeats(code, extra, seats)\n}\n");
    // What nodes can be.
    let enrolment = d.classes.iter().find(|c| c.id == "university::Enrolment").unwrap();
    assert!(enrolment.references.iter().any(|r| r.name == "course" && r.target == "university::Course"));
    assert!(enrolment.attributes.iter().any(|a| a.name == "grade"));
    assert!(d.types.iter().any(|t| t == "EInt") && d.types.iter().any(|t| t == "Kind"));
    assert!(d.problems.iter().all(|p| p.severity != Severity::Error));
}

#[test]
fn draws_matching_formulas_as_conditions() {
    let rules = Rules::open("philosophers", "philosophers.henshin_text");
    let take_left = rules.rule("takeLeft");
    assert_eq!(take_left.formulas.len(), 1);
    assert_eq!(take_left.formulas[0].formula, "!heldByOther AND !heldByMe");
    assert_eq!(take_left.formulas[0].graphs, ["takeLeft#heldByOther", "takeLeft#heldByMe"]);
    // Negated graphs of a conjunction are forbidden, as Henshin draws them.
    let other = node(&take_left, "takeLeft#heldByOther::other");
    assert_eq!((other.action, other.condition.as_deref()), (RuleAction::Forbid, Some("takeLeft#heldByOther")));
    let held = edge(&take_left, "takeLeft#heldByMe::p->fork:holds");
    assert_eq!((held.action, held.source.as_str()), (RuleAction::Forbid, "takeLeft::p"));
    let take_right = rules.rule("takeRight");
    assert!(!take_right.injective_matching);
    assert_eq!(node(&take_right, "takeRight::holder").action, RuleAction::Forbid);
}

#[test]
fn draws_henshin_models_like_their_texts() {
    for example in ["university", "philosophers"] {
        let text = Rules::open(example, &format!("{example}.henshin_text"));
        let model = Rules::open(example, &format!("{example}.henshin"));
        let d = model.diagram();
        assert_eq!(d.syntax, TransformationSyntax::Model);
        assert_eq!(shape(&d), shape(&text.diagram()));
        // Ranges select the XML.
        let r = &d.rules[0];
        let line = model.text().lines().nth(r.nodes[0].range.start.line as usize).unwrap();
        assert!(line.trim_start().starts_with("<nodes "), "{line}");
    }
}

/// Edits that change every kind of element, with what the text then contains.
fn edits() -> Vec<(serde_json::Value, Option<&'static str>)> {
    vec![
        (
            json!({ "kind": "addNode", "scope": "drop", "name": "university", "class": "University", "action": "preserve" }),
            Some("\t\tnode university:University\n"),
        ),
        (
            json!({ "kind": "addEdge", "source": "drop::university", "target": "drop::student", "reference": "students", "action": "preserve" }),
            Some("(university->student:students)"),
        ),
        (
            json!({ "kind": "addAttribute", "node": "drop::enrolment", "name": "grade", "value": "1" }),
            Some("\t\tdelete node enrolment:Enrolment {\n\t\t\tgrade=1\n\t\t}"),
        ),
        (
            json!({ "kind": "updateAttribute", "element": "drop::enrolment.grade", "value": "2" }),
            Some("\t\t\tgrade=2\n"),
        ),
        (
            json!({ "kind": "updateNode", "element": "drop::enrolment", "action": "preserve" }),
            Some("(student->enrolment:enrolments)"),
        ),
        (
            json!({ "kind": "updateNode", "element": "drop::enrolment", "action": "create" }),
            Some("(create enrolment->course:course)"),
        ),
        (
            json!({ "kind": "updateEdge", "element": "drop::university->student:students", "action": "forbid" }),
            Some("(forbid university->student:students)"),
        ),
        (
            json!({ "kind": "rename", "element": "drop::student", "name": "learner" }),
            Some("(create learner->enrolment:enrolments)"),
        ),
        (
            json!({ "kind": "setParameters", "element": "drop", "parameters": [{ "kind": "in", "name": "learner", "type": "Student" }, { "kind": "in", "name": "course", "type": "Course" }] }),
            Some("rule drop(IN learner:Student, IN course:Course) {"),
        ),
        (
            json!({ "kind": "setConditions", "element": "grade", "conditions": ["mark >= 2", "mark <= 5"] }),
            Some("\tconditions [mark >= 2, mark <= 5]\n"),
        ),
        (
            json!({ "kind": "setRuleOptions", "element": "grade", "checkDangling": false }),
            Some("\tcheckDangling false\n"),
        ),
        (json!({ "kind": "setRuleOptions", "element": "closeCourse", "checkDangling": true }), None),
        (
            json!({ "kind": "addMultiRule", "scope": "drop", "name": "others" }),
            Some("\t\tmultiRule others {\n\t\t\tgraph {\n\t\t\t}\n\t\t}"),
        ),
        (
            json!({ "kind": "addNode", "scope": "drop/others", "name": "other", "class": "Enrolment", "action": "delete" }),
            Some("\t\t\t\tdelete node other:Enrolment\n"),
        ),
        (
            json!({ "kind": "addEdge", "source": "drop::learner", "target": "drop/others::other", "reference": "enrolments", "action": "delete" }),
            Some("\t\t\t\tedges [(delete learner->other:enrolments)]"),
        ),
        (
            json!({ "kind": "addAttribute", "node": "drop::course", "scope": "drop/others", "name": "credits", "value": "5", "action": "preserve" }),
            Some("reuse course {\n\t\t\t\t\tcredits=5\n\t\t\t\t}"),
        ),
        (json!({ "kind": "delete", "elements": ["drop::university"] }), None),
        (json!({ "kind": "addRule", "name": "nothing" }), Some("\n\nrule nothing() {\n\tgraph {\n\t}\n}\n")),
        (json!({ "kind": "addUnit", "name": "twice" }), Some("unit twice() {\n}")),
        (
            json!({ "kind": "setParameters", "element": "twice", "parameters": [{ "kind": "in", "name": "s", "type": "Student" }, { "kind": "in", "name": "c", "type": "Course" }] }),
            None,
        ),
        (
            json!({ "kind": "setUnitBody", "element": "twice", "body": "drop(s, c)\nnothing()" }),
            Some("unit twice(IN s:Student, IN c:Course) {\n\tdrop(s, c)\n\tnothing()\n}"),
        ),
        (json!({ "kind": "rename", "element": "nothing", "name": "idle" }), Some("\tidle()\n")),
        (json!({ "kind": "delete", "elements": ["idle"] }), None),
        (json!({ "kind": "delete", "elements": ["drop/others"] }), None),
        (json!({ "kind": "delete", "elements": ["drop::enrolment.grade", "addSeats::course.capacity#2"] }), None),
    ]
}

#[test]
fn edits_henshin_texts() {
    let mut rules = Rules::open("university", "university.henshin_text");
    let before = rules.text().to_string();
    for (e, expected) in edits() {
        rules.edit(e.clone());
        if let Some(part) = expected {
            assert!(rules.text().contains(part), "after {e}: missing {part:?} in\n{}", rules.text());
        }
    }
    let text = rules.text();
    // Deleted: the node with its edge, the rule with its call, the multi-rule, attributes.
    let drop = &text[text.find("rule drop(").unwrap()..text.find("rule grade(").unwrap()];
    assert!(!drop.contains("university:University") && !drop.contains("university->learner"), "{drop}");
    assert!(!text.contains("rule idle") && !text.contains("idle()"), "{text}");
    assert!(!text.contains("multiRule others"));
    assert!(text.contains("\t\tcreate node enrolment:Enrolment\n"), "the block of the last attribute goes");
    assert!(text.contains("\t\t\tcapacity=seats\n\t\t}"), "{text}");
    // closeCourse no longer turns the dangling check off.
    assert!(!text.contains("checkDangling false\n\tgraph {\n\t\tnode university:University\n\t\tdelete node course"));
    // The rest of the text, comments included, is as it was.
    for line in before.lines().filter(|l| l.starts_with("//")) {
        assert!(text.contains(line), "lost {line:?}");
    }
    let d = rules.diagram();
    let drop = d.rules.iter().find(|r| r.id == "drop").unwrap();
    assert_eq!(node(drop, "drop::enrolment").action, RuleAction::Create);
    assert!(d.problems.iter().all(|p| p.severity != Severity::Error), "{:#?}", d.problems);
}

#[test]
fn edits_henshin_models_like_texts() {
    let mut text = Rules::open("university", "university.henshin_text");
    let mut model = Rules::open("university", "university.henshin");
    for (e, _) in edits() {
        text.edit(e.clone());
        model.edit(e.clone());
        assert_eq!(shape(&model.diagram()), shape(&text.diagram()), "after {e}");
        // A few lines change, not the file.
        // (A multi-rule has copies of all its kernel's nodes and edges.)
        assert!(model.changed < 80, "after {e}: {} lines changed", model.changed);
    }
    // The text and the model are still equivalent.
    let model_text = model.text().to_string();
    let edits = text.ws.open_document(&model.name.clone(), &model_text);
    assert!(edits.iter().all(|d| d.severity != Severity::Error));
    assert_eq!(text.ws.sync_transformation(&text.name, &model.name, &model_text).unwrap(), vec![]);
}

#[test]
fn rejects_edits_that_do_not_fit() {
    let rules = Rules::open("university", "university.henshin_text");
    let e = rules.error(
        json!({ "kind": "addNode", "scope": "enrol", "name": "student", "class": "Student", "action": "create" }),
    );
    assert!(e.contains("already a node named 'student'"), "{e}");
    let e = rules.error(json!({ "kind": "addRule", "name": "enrol" }));
    assert!(e.contains("already a rule or unit"), "{e}");
    let e = rules.error(json!({ "kind": "rename", "element": "enrol::student", "name": "a b" }));
    assert!(e.contains("not a name"), "{e}");
    let e = rules.error(json!({ "kind": "addAttribute", "node": "enrol::student", "name": "name", "value": "1 +" }));
    assert!(e.contains("not an expression"), "{e}");
    let e = rules.error(json!({ "kind": "delete", "elements": ["nope"] }));
    assert!(e.contains("no element 'nope'"), "{e}");
    // A model is only changed into a valid model.
    let model = Rules::open("university", "university.henshin");
    let e =
        model.error(json!({ "kind": "addNode", "scope": "enrol", "name": "x", "class": "Nope", "action": "create" }));
    assert!(e.contains("unknown class 'Nope'"), "{e}");
}

#[test]
fn works_through_json_rpc() {
    let mut server = rpc::Server::new();
    let mm = format!("{EXAMPLES}/university/university.oclinecore");
    let rules = format!("{EXAMPLES}/university/university.henshin_text");
    for (name, text) in
        [(&mm, std::fs::read_to_string(&mm).unwrap()), (&rules, std::fs::read_to_string(&rules).unwrap())]
    {
        server.call("openDocument", json!({ "name": name, "text": text })).unwrap();
    }
    let d = server.call("transformationDiagram", json!({ "name": rules })).unwrap();
    assert_eq!(d["rules"][0]["nodes"][2]["action"], "create");
    let r = server
        .call("editTransformation", json!({ "name": rules, "edit": { "kind": "addRule", "name": "fresh" } }))
        .unwrap();
    assert_eq!(r["element"], "fresh");
    assert_eq!(r["edits"].as_array().unwrap().len(), 1);
    let target = format!("{EXAMPLES}/university/university.henshin");
    let r = server
        .call(
            "syncTransformation",
            json!({ "name": rules, "target": target, "targetText": std::fs::read_to_string(&target).unwrap() }),
        )
        .unwrap();
    assert_eq!(r["edits"], json!([]));
}
