//! Graph transformations with Henshin documents (`.henshin_text`, and `.henshin` files, which
//! mean the same): diagnostics, editor support, rule and unit application, state space
//! exploration, conversion between the two syntaxes and keeping a pair of them in sync.

use oxychloride::*;
use serde_json::{Map, Value as Json, json};

const EXAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");

fn open(ws: &mut Workspace, file: &str) -> (String, Vec<Diagnostic>) {
    let path = format!("{EXAMPLES}/{file}");
    let text = std::fs::read_to_string(&path).unwrap();
    let diagnostics = ws.open_document(&path, &text);
    (path, diagnostics)
}

/// The university metamodel, model and transformation.
fn university() -> (Workspace, String, String) {
    let mut ws = Workspace::new();
    let (_, d) = open(&mut ws, "university/university.oclinecore");
    assert!(d.is_empty(), "{d:#?}");
    let (model, _) = open(&mut ws, "university/university.hutn");
    let (rules, d) = open(&mut ws, "university/university.henshin_text");
    assert!(d.is_empty(), "{d:#?}");
    (ws, model, rules)
}

fn philosophers() -> (Workspace, String, String) {
    let mut ws = Workspace::new();
    let (_, d) = open(&mut ws, "philosophers/philosophers.oclinecore");
    assert!(d.is_empty(), "{d:#?}");
    let (model, d) = open(&mut ws, "philosophers/philosophers.hutn");
    assert!(d.is_empty(), "{d:#?}");
    let (rules, d) = open(&mut ws, "philosophers/philosophers.henshin_text");
    assert!(d.is_empty(), "{d:#?}");
    (ws, model, rules)
}

fn args(v: Json) -> Map<String, Json> {
    v.as_object().cloned().unwrap_or_default()
}

/// The id of the node with this label.
fn node(state: &TransformationState, label: &str) -> u32 {
    state.graph.nodes.iter().find(|n| n.label == label).unwrap_or_else(|| panic!("no node {label}")).id
}

fn count(state: &TransformationState, class: &str) -> usize {
    state.graph.nodes.iter().filter(|n| n.class.ends_with(&format!("::{class}"))).count()
}

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
fn lists_rules_and_units() {
    let (ws, _, rules) = university();
    assert_eq!(ws.document_language(&rules), Some(Language::Henshin));
    let units = ws.transformation_units(&rules).unwrap();
    let names: Vec<(&str, &str)> = units.units.iter().map(|u| (u.name.as_str(), u.kind.as_str())).collect();
    assert_eq!(
        names,
        [
            ("enrol", "rule"),
            ("drop", "rule"),
            ("grade", "rule"),
            ("createSeminar", "rule"),
            ("addSeats", "rule"),
            ("closeCourse", "rule"),
            ("enrolEverywhere", "unit"),
            ("gradeOrEnrol", "unit"),
            ("replaceBySeminar", "unit"),
            ("growCourse", "unit"),
        ]
    );
    let enrol = &units.units[0];
    assert_eq!(enrol.parameters[0].name, "student");
    assert!(enrol.parameters[0].node);
    assert_eq!(enrol.parameters[0].kind, "in");
    assert_eq!(units.units[5].multi_rules, ["dropEnrolments"]);
}

#[test]
fn reports_problems_with_ranges() {
    let (mut ws, _, _) = university();
    let text = "ePackageImport university\n\
        rule r(IN x:EInt) {\n\
        \tgraph {\n\
        \t\tnode s:Student { age=x }\n\
        \t\tnode c:Cours\n\
        \t\tcreate node e:Enrolment\n\
        \t\tedges [(s->e:enrolments), (create e->s:course), (s->q:enrolments)]\n\
        \t}\n\
        }\n";
    let diagnostics = ws.open_document("/tmp/bad.henshin_text", text);
    let messages: Vec<(u32, &str)> = diagnostics.iter().map(|d| (d.range.start.line, d.message.as_str())).collect();
    assert!(messages.contains(&(3, "class Student has no attribute 'age'")), "{messages:#?}");
    assert!(messages.contains(&(4, "unknown class 'Cours'")), "{messages:#?}");
    assert!(messages.iter().any(|(l, m)| *l == 6 && m.contains("created node must be created too")), "{messages:#?}");
    assert!(
        messages.iter().any(|(l, m)| *l == 6 && m.contains("'s' is a Student, but 'course' refers to Course")),
        "{messages:#?}"
    );
    assert!(messages.contains(&(6, "unknown node 'q'")), "{messages:#?}");
    // Syntax errors.
    let diagnostics =
        ws.open_document("/tmp/bad.henshin_text", "ePackageImport university rule r() { graph { node } }");
    assert!(diagnostics.iter().any(|d| d.message.starts_with("expected a node name")), "{diagnostics:#?}");
    // Unknown packages.
    let diagnostics = ws.open_document("/tmp/bad.henshin_text", "ePackageImport nowhere");
    assert!(diagnostics[0].message.starts_with("unknown package 'nowhere'"), "{diagnostics:#?}");
    // They are part of the workspace's diagnostics.
    assert!(ws.diagnostics(None).iter().any(|d| d.source == "/tmp/bad.henshin_text"));
}

#[test]
fn hovers_describe_names() {
    let (ws, _, rules) = university();
    let text = ws.document_text(&rules).unwrap().to_string();
    let hover = |marker: &str| ws.hover(&rules, position_after(&text, marker)).map(|h| h.contents);
    assert_eq!(hover("create node enrol").as_deref(), Some("create node enrolment : university::Enrolment"));
    assert_eq!(hover("create node enrolment:Enrol").as_deref(), Some("class university::Enrolment"));
    assert_eq!(
        hover("(create student->enrolment:enrol").as_deref(),
        Some("reference Student::enrolments : Enrolment[*] (containment, opposite student)")
    );
    assert_eq!(hover("kind=Kind.SEM").as_deref(), Some("literal university::Kind::SEMINAR"));
    assert_eq!(
        hover("priority [gra").as_deref(),
        Some("rule grade(IN student : Student, IN course : Course, IN mark : EInt)")
    );
    assert_eq!(
        hover("ePackageImport univ").as_deref(),
        Some("package university ('http://www.example.org/university/1.0')")
    );
}

#[test]
fn completes_from_the_metamodel_and_the_rule() {
    let (mut ws, _, _) = university();
    let name = "/tmp/c.henshin_text";
    let complete = |ws: &mut Workspace, text: &str, marker: &str| {
        ws.open_document(name, text);
        let list = ws.complete(name, position_after(text, marker)).unwrap();
        labels(&list).into_iter().map(str::to_string).collect::<Vec<_>>()
    };
    let base =
        "ePackageImport university\nrule r(IN code:EString) {\n  graph {\n    node s:Student\n    node c:Course { ";
    // Classes after `node x:`.
    let l = complete(&mut ws, "ePackageImport university\nrule r() { graph { node s: } }", "node s:");
    assert!(l.contains(&"Student".to_string()) && l.contains(&"Course".to_string()), "{l:?}");
    // Attributes in a node's braces, values after `=`.
    let l = complete(&mut ws, &format!("{base} }}\n  }}\n}}"), "node c:Course { ");
    assert!(
        l.contains(&"code".to_string()) && l.contains(&"capacity".to_string()) && l.contains(&"set".to_string()),
        "{l:?}"
    );
    assert!(!l.contains(&"enrolments".to_string()), "{l:?}");
    let l = complete(&mut ws, &format!("{base}kind= }}\n  }}\n}}"), "kind=");
    assert!(l.contains(&"LECTURE".to_string()) && l.contains(&"code".to_string()), "{l:?}");
    // Node names and references in edges.
    let text = "ePackageImport university\nrule r() { graph { node s:Student node e:Enrolment edges [(s->e: )] } }";
    let l = complete(&mut ws, text, "(s->e:");
    assert_eq!(l.first().map(String::as_str), Some("enrolments"), "{l:?}");
    let l = complete(&mut ws, text, "edges [(");
    assert!(l.contains(&"s".to_string()) && l.contains(&"create".to_string()), "{l:?}");
    let l = complete(&mut ws, text, "(s->");
    assert!(l.contains(&"e".to_string()) && !l.contains(&"create".to_string()), "{l:?}");
    // Keywords of graphs and rules, rules to call in units.
    let l = complete(&mut ws, "ePackageImport university\nrule r() { graph {  } }", "graph { ");
    assert!(
        l.contains(&"node".to_string()) && l.contains(&"forbid".to_string()) && l.contains(&"edges".to_string()),
        "{l:?}"
    );
    let l = complete(&mut ws, "ePackageImport university\nrule r() {  }", "rule r() { ");
    assert!(l.contains(&"graph".to_string()) && l.contains(&"checkDangling".to_string()), "{l:?}");
    let text = "ePackageImport university\nrule r() { graph { node s:Student } }\nunit u() {  }";
    let l = complete(&mut ws, text, "unit u() { ");
    assert!(l.contains(&"r".to_string()) && l.contains(&"while".to_string()), "{l:?}");
    let l = complete(&mut ws, "ePackageImport ", "ePackageImport ");
    assert!(l.contains(&"university".to_string()), "{l:?}");
}

#[test]
fn applies_rules_step_by_step() {
    let (mut ws, model, rules) = university();
    let state = ws.start_transformation(&model).unwrap();
    assert_eq!(state.step, 0);
    assert_eq!(count(&state, "Enrolment"), 8);
    let session = state.session;
    let brian = node(&state, "Student 'Brian'");
    let algo = state.graph.nodes.iter().find(|n| n.attributes.iter().any(|a| a.values == ["'IN0007'"])).unwrap().id;
    // The initial state violates the model's invariants.
    assert!(state.violations.iter().any(|v| v.constraint == "CreditLimit"), "{:#?}", state.violations);

    // Brian is enrolled in two courses: enrol has two matches for him.
    let matches = ws.find_matches(session, &rules, "enrol", &args(json!({ "student": brian })), None, 10).unwrap();
    assert_eq!(matches.matches.len(), 2, "{matches:#?}");
    assert!(matches.matches.iter().all(|m| m.label.starts_with("enrol(student=Student 'Brian', course=Course")));
    // In Algorithms: one match.
    let given = args(json!({ "student": brian, "course": algo }));
    let matches = ws.find_matches(session, &rules, "enrol", &given, None, 10).unwrap();
    assert_eq!(matches.matches.len(), 1);
    let result = ws.apply_transformation(session, &rules, "enrol", &given, Some(0), None, 1).unwrap();
    assert!(result.applied, "{:?}", result.messages);
    let state = result.state;
    assert_eq!(state.step, 1);
    assert_eq!(count(&state, "Enrolment"), 9);
    assert_eq!(state.changes.created_nodes.len(), 1);
    let created = state.changes.created_nodes[0];
    let edges: Vec<(&str, u32, u32)> =
        state.changes.created_edges.iter().map(|e| (e.feature.as_str(), e.source, e.target)).collect();
    assert!(edges.contains(&("enrolments", brian, created)), "{edges:?}");
    assert!(edges.contains(&("enrolments", algo, created)), "{edges:?}");
    // Now the NAC forbids enrolling again.
    let again = ws.apply_transformation(session, &rules, "enrol", &given, None, None, 1).unwrap();
    assert!(!again.applied);
    assert_eq!(again.state.step, 1);

    // grade: `set` changes an attribute; the condition checks the mark.
    let bad = ws.apply_transformation(
        session,
        &rules,
        "grade",
        &args(json!({ "student": brian, "course": algo, "mark": 7 })),
        None,
        None,
        1,
    );
    assert!(!bad.unwrap().applied);
    let graded = ws
        .apply_transformation(
            session,
            &rules,
            "grade",
            &args(json!({ "student": brian, "course": algo, "mark": 2 })),
            None,
            None,
            1,
        )
        .unwrap();
    assert!(graded.applied);
    assert_eq!(graded.state.changes.changed_attributes[0].feature, "grade");
    assert_eq!(graded.state.changes.changed_attributes[0].new, ["2"]);

    // drop deletes the enrolment again.
    let dropped = ws
        .apply_transformation(
            session,
            &rules,
            "drop",
            &args(json!({ "student": brian, "course": algo })),
            None,
            None,
            1,
        )
        .unwrap();
    assert!(dropped.applied);
    assert_eq!(dropped.state.changes.deleted_nodes.len(), 1);
    assert_eq!(dropped.state.changes.deleted_nodes[0].id, created);
    assert_eq!(dropped.state.steps.len(), 4);

    // Going back: applying on step 1 discards the later steps.
    let from_1 = ws
        .apply_transformation(
            session,
            &rules,
            "drop",
            &args(json!({ "student": brian, "course": algo })),
            None,
            Some(1),
            1,
        )
        .unwrap();
    assert_eq!(from_1.state.steps.len(), 3);
    assert_eq!(ws.transformation_state(session, Some(0)).unwrap().graph.nodes.len(), state.graph.nodes.len() - 1);

    // The model itself is unchanged; states export as HUTN and XMI.
    assert_eq!(ws.model().len(), 16);
    let hutn = ws.export_transformation(session, Some(1), Language::Hutn, None).unwrap();
    assert_eq!(hutn.lines().filter(|l| l.trim_start().starts_with("Enrolment \"")).count(), 9, "{hutn}");
    let xmi = ws.export_transformation(session, Some(1), Language::Xmi, None).unwrap();
    assert!(xmi.contains("<uni:University"), "{xmi}");
    assert!(ws.close_transformation(session));
    assert!(ws.transformation_state(session, None).is_err());
}

#[test]
fn creates_with_attributes_and_binds_out_parameters() {
    let (mut ws, model, rules) = university();
    let session = ws.start_transformation(&model).unwrap().session;
    let given = args(json!({ "code": "IN2000", "title": "Seminar", "credits": "5", "capacity": 10 }));
    let result = ws.apply_transformation(session, &rules, "createSeminar", &given, None, None, 1).unwrap();
    assert!(result.applied, "{:?}", result.messages);
    let course = result.state.changes.created_nodes[0];
    let info = result.state.graph.nodes.iter().find(|n| n.id == course).unwrap();
    let attr = |name: &str| info.attributes.iter().find(|a| a.name == name).map(|a| a.values.join(","));
    assert_eq!(attr("code").as_deref(), Some("'IN2000'"));
    assert_eq!(attr("kind").as_deref(), Some("Kind::SEMINAR"));
    assert_eq!(attr("credits").as_deref(), Some("5"));
    let app = &result.state.steps[1].applications[0];
    assert!(app.parameters.iter().any(|p| p.name == "course" && p.value == format!("Course #{course}")), "{app:#?}");
    // The code is taken now.
    let again = ws.apply_transformation(session, &rules, "createSeminar", &given, None, None, 1).unwrap();
    assert!(!again.applied);
    // Missing parameters are reported.
    let missing = ws
        .apply_transformation(session, &rules, "createSeminar", &args(json!({ "code": "X" })), None, None, 1)
        .unwrap();
    assert!(!missing.applied);
    assert!(missing.messages.iter().any(|m| m.contains("has no value")), "{:?}", missing.messages);
    // Bad parameter values are errors.
    assert!(
        ws.apply_transformation(session, &rules, "createSeminar", &args(json!({ "credits": "many" })), None, None, 1)
            .is_err()
    );
    assert!(
        ws.apply_transformation(session, &rules, "createSeminar", &args(json!({ "nope": 1 })), None, None, 1).is_err()
    );
}

#[test]
fn matches_and_changes_attribute_values() {
    let (mut ws, model, rules) = university();
    let session = ws.start_transformation(&model).unwrap().session;
    let result = ws
        .apply_transformation(
            session,
            &rules,
            "addSeats",
            &args(json!({ "code": "IN0001", "extra": 2 })),
            None,
            None,
            1,
        )
        .unwrap();
    assert!(result.applied, "{:?}", result.messages);
    let change = &result.state.changes.changed_attributes[0];
    assert_eq!(
        (change.feature.as_str(), change.old.clone(), change.new.clone()),
        ("capacity", vec!["3".into()], vec!["5".into()])
    );
    // The prog1 course is no longer full: FreeSeat's postcondition does not matter for invariants,
    // but NotOverbooked holds still.
    assert!(!result.state.violations.iter().any(|v| v.constraint == "NotOverbooked"));
    // A unit iterating it.
    let grown = ws
        .apply_transformation(
            session,
            &rules,
            "growCourse",
            &args(json!({ "code": "IN0001", "extra": 1 })),
            None,
            None,
            1,
        )
        .unwrap();
    assert!(grown.applied);
    assert_eq!(grown.state.steps[2].applications.len(), 3);
    assert_eq!(grown.state.changes.changed_attributes[0].new, ["8"]);
}

#[test]
fn amalgamates_multi_rules() {
    let (mut ws, model, rules) = university();
    let state = ws.start_transformation(&model).unwrap();
    let session = state.session;
    // prog1 is a prerequisite: the NAC forbids closing it.
    let closed = ws
        .apply_transformation(session, &rules, "closeCourse", &args(json!({ "code": "IN0001" })), None, None, 1)
        .unwrap();
    assert!(!closed.applied);
    // The thesis is not: it is closed with Carla's enrolment in it.
    let matches =
        ws.find_matches(session, &rules, "closeCourse", &args(json!({ "code": "IN9999" })), None, 10).unwrap();
    assert_eq!(matches.matches.len(), 1);
    assert_eq!(matches.matches[0].instances, 1);
    let closed = ws
        .apply_transformation(session, &rules, "closeCourse", &args(json!({ "code": "IN9999" })), None, None, 1)
        .unwrap();
    assert!(closed.applied, "{:?}", closed.messages);
    let deleted: Vec<&str> = closed.state.changes.deleted_nodes.iter().map(|n| n.class.as_str()).collect();
    assert_eq!(deleted, ["university::Course", "university::Enrolment"]);
    assert_eq!(count(&closed.state, "Enrolment"), 7);
    // Carla is within the credit limit now.
    assert!(!closed.state.violations.iter().any(|v| v.constraint == "CreditLimit"), "{:#?}", closed.state.violations);
    // Algorithms is required by nothing now (the thesis is gone): with its enrolments, it can go.
    let algo = ws.find_matches(session, &rules, "closeCourse", &args(json!({ "code": "IN0007" })), None, 10).unwrap();
    assert_eq!(algo.matches[0].instances, 2);
}

#[test]
fn executes_units() {
    let (mut ws, model, rules) = university();
    let state = ws.start_transformation(&model).unwrap();
    let session = state.session;
    let brian = node(&state, "Student 'Brian'");
    // A loop: Brian is enrolled in the two courses he is not in yet.
    let result = ws
        .apply_transformation(session, &rules, "enrolEverywhere", &args(json!({ "student": brian })), None, None, 1)
        .unwrap();
    assert!(result.applied, "{:?}", result.messages);
    assert_eq!(result.state.steps[1].applications.len(), 2);
    assert_eq!(result.state.changes.created_nodes.len(), 2);
    assert!(
        result.state.steps[1].label.starts_with("enrolEverywhere(student=Student 'Brian')"),
        "{}",
        result.state.steps[1].label
    );
    // Priority: Brian is enrolled in everything, so he is graded.
    let prog1 = state.graph.nodes.iter().find(|n| n.attributes.iter().any(|a| a.values == ["'IN0001'"])).unwrap().id;
    let given = args(json!({ "student": brian, "course": prog1, "mark": 3 }));
    let result = ws.apply_transformation(session, &rules, "gradeOrEnrol", &given, None, None, 1).unwrap();
    assert_eq!(result.state.steps[2].applications[0].rule, "grade");
    // A sequence that fails rolls back: closing prog1 is not possible.
    let given = args(json!({ "old": "IN0001", "code": "S1", "title": "S", "credits": 1, "capacity": 1 }));
    let result = ws.apply_transformation(session, &rules, "replaceBySeminar", &given, None, None, 1).unwrap();
    assert!(!result.applied);
    assert_eq!(result.state.steps.len(), 3);
    let given = args(json!({ "old": "IN9999", "code": "S1", "title": "S", "credits": 1, "capacity": 1 }));
    let result = ws.apply_transformation(session, &rules, "replaceBySeminar", &given, None, None, 1).unwrap();
    assert!(result.applied, "{:?}", result.messages);
    let rules_applied: Vec<&str> = result.state.steps[3].applications.iter().map(|a| a.rule.as_str()).collect();
    assert_eq!(rules_applied, ["closeCourse", "createSeminar"]);
}

#[test]
fn explores_the_state_space_of_the_dining_philosophers() {
    let (mut ws, model, rules) = philosophers();
    let session = ws.start_transformation(&model).unwrap().session;
    let request =
        ExploreRequest { rules: vec!["takeLeft".into(), "takeRight".into(), "release".into()], ..Default::default() };
    let space = ws.explore_state_space(session, &rules, &request).unwrap();
    assert!(space.complete, "{:?}", space.messages);
    assert!(space.messages.is_empty(), "{:?}", space.messages);
    // Each philosopher thinks, holds the left fork, or eats; neighbours share forks.
    // Thinking/left combinations: 2^3 = 8; eating: one at a time, with the others thinking
    // or (the one to the left of the eater) holding a left fork: 3 * 2 = 6.
    assert_eq!(space.states.len(), 14, "{space:#?}");
    let terminal: Vec<&StateSpaceState> = space.states.iter().filter(|s| s.terminal).collect();
    assert_eq!(terminal.len(), 1, "the deadlock");
    assert_eq!(terminal[0].violations, 1, "NoDeadlock");
    assert_eq!(space.states.iter().filter(|s| s.violations > 0).count(), 1);
    assert!(space.transitions.iter().any(|t| t.rule == "release"));
    // The deadlock is three takeLeft steps away.
    let details = ws.state_space_state(session, terminal[0].id).unwrap();
    assert_eq!(details.path.len(), 3);
    assert!(details.path.iter().all(|t| space.transitions[*t].rule == "takeLeft"));
    assert_eq!(details.violations[0].constraint, "NoDeadlock");
    assert_eq!(details.changes.created_edges.len(), 1);
    // Adopting the state replays the path as steps.
    let state = ws.adopt_state(session, terminal[0].id).unwrap();
    assert_eq!(state.steps.len(), 4);
    assert!(state.steps[1].label.starts_with("takeLeft"));
    assert_eq!(state.violations.len(), 1);
    // Limits.
    let limited = ws
        .explore_state_space(session, &rules, &ExploreRequest { step: Some(0), max_states: 5, ..request.clone() })
        .unwrap();
    assert_eq!(limited.states.len(), 5);
    assert!(!limited.complete);
    let shallow = ws
        .explore_state_space(session, &rules, &ExploreRequest { step: Some(0), max_depth: Some(1), ..request })
        .unwrap();
    assert_eq!(shallow.states.len(), 4);
    assert!(!shallow.complete);
}

#[test]
fn merges_isomorphic_states() {
    // Anonymous philosophers: the rotations of a state are isomorphic.
    let (mut ws, _, rules) = philosophers();
    let text = "@Spec { metamodel \"philosophers\" { nsUri: \"http://www.example.org/philosophers/1.0\" } }\n\
        philosophers { Table { forks: Fork \"a\" {}, Fork \"b\" {}, Fork \"c\" {}\n\
        philosophers: Philosopher { left: Fork \"a\" right: Fork \"b\" }, Philosopher { left: Fork \"b\" right: Fork \"c\" },\n\
        Philosopher { left: Fork \"c\" right: Fork \"a\" } } }";
    let d = ws.open_document("/tmp/anonymous.hutn", text);
    assert!(!d.iter().any(|d| d.severity == Severity::Error), "{d:#?}");
    let session = ws.start_transformation("/tmp/anonymous.hutn").unwrap().session;
    let request =
        ExploreRequest { rules: vec!["takeLeft".into(), "takeRight".into(), "release".into()], ..Default::default() };
    let space = ws.explore_state_space(session, &rules, &request).unwrap();
    assert!(space.complete);
    // Up to rotation: 8 thinking/left combinations are 4 (none, one, two, three hold their
    // left fork); with an eater: the one to its left thinks or holds its left fork: 2.
    assert_eq!(space.states.len(), 6, "{space:#?}");
    // From the initial state, three takeLeft matches lead to the same state.
    assert_eq!(space.transitions.iter().filter(|t| t.source == 0).map(|t| t.count).sum::<usize>(), 3);
}

#[test]
fn works_through_json_rpc() {
    let mut server = rpc::Server::new();
    for file in ["university/university.oclinecore", "university/university.hutn", "university/university.henshin_text"]
    {
        let path = format!("{EXAMPLES}/{file}");
        let text = std::fs::read_to_string(&path).unwrap();
        server.call("openDocument", json!({ "name": path, "text": text })).unwrap();
    }
    let rules = format!("{EXAMPLES}/university/university.henshin_text");
    let units = server.call("transformationUnits", json!({ "name": rules })).unwrap();
    assert_eq!(units["units"].as_array().unwrap().len(), 10);
    let state = server.call("startTransformation", json!({ "model": "university.hutn" })).unwrap();
    let session = state["session"].as_u64().unwrap();
    let matches =
        server.call("findMatches", json!({ "session": session, "document": rules, "rule": "enrol" })).unwrap();
    assert_eq!(matches["matches"].as_array().unwrap().len(), 4);
    let applied = server
        .call("applyTransformation", json!({ "session": session, "document": rules, "unit": "enrol", "match": 3 }))
        .unwrap();
    assert_eq!(applied["applied"], true);
    assert_eq!(applied["state"]["step"], 1);
    let space = server
        .call("exploreStateSpace", json!({ "session": session, "document": rules, "rules": ["enrol"], "step": 0 }))
        .unwrap();
    // The four missing enrolments, each made or not.
    assert_eq!(space["states"].as_array().unwrap().len(), 16);
    let state = server.call("stateSpaceState", json!({ "session": session, "state": 15 })).unwrap();
    assert_eq!(state["path"].as_array().unwrap().len(), 4);
    let text = server.call("exportTransformation", json!({ "session": session, "format": "hutn" })).unwrap();
    assert!(text["text"].as_str().unwrap().contains("University"));
    let err = server.call("findMatches", json!({ "session": 99, "document": rules, "rule": "enrol" })).unwrap_err();
    assert_eq!(err.code, rpc::codes::NOT_FOUND);
    assert!(rpc::METHODS.contains(&"exploreStateSpace"));
}

#[test]
fn sessions_survive_metamodel_changes() {
    let (mut ws, model, rules) = university();
    let session = ws.start_transformation(&model).unwrap().session;
    let applied = ws.apply_transformation(session, &rules, "enrol", &Map::new(), None, None, 1).unwrap();
    assert!(applied.applied);
    // Editing the metamodel rebuilds it; the session's states are mapped onto the new one.
    let mm = format!("{EXAMPLES}/university/university.oclinecore");
    let text = ws
        .document_text(&mm)
        .unwrap()
        .replace("class University {", "class University {\n    attribute city : String[?];");
    ws.open_document(&mm, &text);
    let state = ws.transformation_state(session, None).unwrap();
    assert_eq!(count(&state, "Enrolment"), 9);
    let again = ws.apply_transformation(session, &rules, "enrol", &Map::new(), None, None, 1).unwrap();
    assert!(again.applied, "{:?}", again.messages);
}

#[test]
fn survives_every_prefix_of_a_document() {
    let (mut ws, _, _) = university();
    for file in ["university/university.henshin_text", "philosophers/philosophers.henshin_text"] {
        let text = std::fs::read_to_string(format!("{EXAMPLES}/{file}")).unwrap();
        let name = "/tmp/prefix.henshin_text";
        let ends: Vec<usize> = text.char_indices().map(|(i, _)| i).step_by(5).chain([text.len()]).collect();
        for end in ends {
            let prefix = &text[..end];
            ws.open_document(name, prefix);
            let end_position = Position {
                line: prefix.matches('\n').count() as u32,
                column: prefix[prefix.rfind('\n').map_or(0, |i| i + 1)..].chars().count() as u32,
            };
            ws.complete(name, end_position);
            ws.hover(name, end_position);
            let _ = ws.transformation_units(name).unwrap();
        }
    }
}

// ---------------------------------------------------------------------- `.henshin` files

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

fn unit_signatures(ws: &Workspace, document: &str) -> Vec<String> {
    let units = ws.transformation_units(document).unwrap();
    assert!(!units.diagnostics.iter().any(|d| d.severity == Severity::Error), "{:#?}", units.diagnostics);
    units
        .units
        .iter()
        .map(|u| {
            let params: Vec<String> = u.parameters.iter().map(|p| format!("{} {}:{}", p.kind, p.name, p.ty)).collect();
            format!("{} {}({}) {:?}", u.kind, u.name, params.join(", "), u.multi_rules)
        })
        .collect()
}

#[test]
fn henshin_models_mean_what_their_texts_mean() {
    for (example, rules) in
        [("university", vec!["enrol", "drop"]), ("philosophers", vec!["takeLeft", "takeRight", "release"])]
    {
        let mut ws = Workspace::new();
        let (_, d) = open(&mut ws, &format!("{example}/{example}.oclinecore"));
        assert!(d.is_empty(), "{d:#?}");
        let (model, _) = open(&mut ws, &format!("{example}/{example}.hutn"));
        let (text, d) = open(&mut ws, &format!("{example}/{example}.henshin_text"));
        assert!(d.is_empty(), "{d:#?}");
        let (xmi, d) = open(&mut ws, &format!("{example}/{example}.henshin"));
        assert!(d.is_empty(), "{example}.henshin: {d:#?}");
        assert_eq!(ws.document_language(&xmi), Some(Language::HenshinModel));
        // The same rules and units, …
        assert_eq!(unit_signatures(&ws, &xmi), unit_signatures(&ws, &text));
        // … the same state space, …
        let session = ws.start_transformation(&model).unwrap().session;
        let request = ExploreRequest {
            rules: rules.iter().map(|r| r.to_string()).collect(),
            max_states: 200,
            ..Default::default()
        };
        let from_text = ws.explore_state_space(session, &text, &request).unwrap();
        let from_model = ws.explore_state_space(session, &xmi, &request).unwrap();
        assert_eq!(from_text.states.len(), from_model.states.len());
        assert_eq!(from_text.transitions.len(), from_model.transitions.len());
        assert_eq!(from_text.complete, from_model.complete);
        // … and the example files are in sync.
        let model_text = ws.document_text(&xmi).unwrap().to_string();
        assert_eq!(
            ws.sync_transformation(&text, &xmi, &model_text).unwrap(),
            vec![],
            "{example}.henshin is not in sync"
        );
        let rules_text = ws.document_text(&text).unwrap().to_string();
        assert_eq!(
            ws.sync_transformation(&xmi, &text, &rules_text).unwrap(),
            vec![],
            "{example}.henshin_text is not in sync"
        );
    }
}

#[test]
fn applies_rules_of_henshin_models() {
    let (mut ws, model, _) = university();
    let (xmi, d) = open(&mut ws, "university/university.henshin");
    assert!(d.is_empty(), "{d:#?}");
    let start = ws.start_transformation(&model).unwrap();
    let student = node(&start, "Student 'Brian'");
    let course = start.graph.nodes.iter().find(|n| n.attributes.iter().any(|a| a.values == ["'IN0007'"])).unwrap().id;
    let before = count(&start, "Enrolment");
    let state = ws
        .apply_transformation(
            start.session,
            &xmi,
            "enrol",
            &args(json!({ "student": student, "course": course })),
            None,
            None,
            1,
        )
        .unwrap()
        .state;
    assert_eq!(count(&state, "Enrolment"), before + 1);
}

#[test]
fn converts_between_the_syntaxes() {
    let (mut ws, _, text) = university();
    let model = ws.export_document(&text, Some("/tmp/university.henshin")).unwrap();
    assert_eq!(model.format, "henshin");
    assert!(model.text.contains("<henshin:Module") && model.text.contains("name=\"enrol\""));
    let d = ws.open_document("/tmp/university.henshin", &model.text);
    assert!(d.is_empty(), "{d:#?}");
    let back = ws.export_document("/tmp/university.henshin", None).unwrap();
    assert_eq!(back.format, "henshin_text");
    assert!(
        back.text.starts_with("ePackageImport university\n\nrule enrol(IN student:Student, IN course:Course) {"),
        "{}",
        back.text
    );
    // A text with errors is not converted.
    ws.open_document("/tmp/broken.henshin_text", "ePackageImport university\nrule r() { graph { node x:Nope } }");
    let e = ws.export_document("/tmp/broken.henshin_text", None).unwrap_err().to_string();
    assert!(e.contains("unknown class 'Nope'"), "{e}");
}

#[test]
fn keeps_pairs_of_documents_in_sync() {
    let (mut ws, _, text) = university();
    let (xmi, _) = open(&mut ws, "university/university.henshin");
    let rules = ws.document_text(&text).unwrap().to_string();
    let model = ws.document_text(&xmi).unwrap().to_string();
    // A change of the text goes to the model as a few line edits …
    let changed = rules.replace("rule drop(", "rule leave(").replace("\tdrop(", "\tleave(");
    ws.open_document(&text, &changed);
    let edits = ws.sync_transformation(&text, &xmi, &model).unwrap();
    assert!(!edits.is_empty() && edits.len() <= 2, "{edits:#?}");
    let new_model = apply(&model, &edits);
    assert!(new_model.contains("name=\"leave\"") && !new_model.contains("name=\"drop\""));
    // … keeping the ids of the elements that are still there.
    let id_of = |text: &str, name: &str| {
        let at = text.find(&format!("name=\"{name}\"")).unwrap();
        text[text[..at].rfind("xmi:id=\"").unwrap()..at].to_string()
    };
    assert_eq!(id_of(&new_model, "leave"), id_of(&model, "drop"));
    assert_eq!(id_of(&new_model, "grade"), id_of(&model, "grade"));
    // A change of the model goes to the text: only the changed rule is written anew, the others
    // keep their comments.
    ws.open_document(&xmi, &new_model.replace("name=\"addSeats\"", "name=\"moreSeats\""));
    let edits = ws.sync_transformation(&xmi, &text, &changed).unwrap();
    let new_text = apply(&changed, &edits);
    assert!(new_text.contains("rule moreSeats(IN code:EString, IN extra:EInt, VAR seats:EInt) {"), "{new_text}");
    assert!(new_text.contains("// Enrols a student in a course they are not enrolled in yet"));
    assert!(new_text.contains("\t\tforbid node existing:Enrolment\n\t\tedges [(forbid student->existing:enrolments)"));
    // Calls refer to units by id in a model: they are renamed too.
    assert!(new_text.contains("// Adds `extra` seats three times (an iterated unit).\nunit growCourse("), "{new_text}");
    assert!(new_text.contains("\t\tmoreSeats(code, extra, seats)"), "{new_text}");
    // In sync again: nothing to do.
    ws.open_document(&text, &new_text);
    let model_now = ws.document_text(&xmi).unwrap().to_string();
    assert_eq!(ws.sync_transformation(&text, &xmi, &model_now).unwrap(), vec![]);
    // A document with errors is not synced.
    ws.open_document(&text, "ePackageImport university\nrule r() { graph { node x:Nope } }");
    assert!(ws.sync_transformation(&text, &xmi, &model_now).is_err());
}

#[test]
fn locates_problems_and_hovers_in_henshin_models() {
    let (mut ws, _, _) = university();
    let (xmi, _) = open(&mut ws, "university/university.henshin");
    let model = ws.document_text(&xmi).unwrap().to_string();
    let broken = model.replacen("#//Enrolment\"", "#//Enrollment\"", 1);
    let d = ws.open_document(&xmi, &broken);
    let error = d.iter().find(|d| d.severity == Severity::Error).unwrap_or_else(|| panic!("{d:#?}"));
    assert!(error.message.contains("unknown class 'Enrollment'"), "{error:?}");
    let line = broken.lines().nth(error.range.start.line as usize).unwrap();
    assert!(line.contains("href=\"http://www.example.org/university/1.0#//Enrollment\""), "{line}");
    ws.open_document(&xmi, &model);
    let at = position_after(&model, "<type href=\"http://www.example.org/university/1.0#//Cou");
    let hover = ws.hover(&xmi, at).unwrap();
    assert!(hover.contents.contains("class university::Course"), "{hover:?}");
}
