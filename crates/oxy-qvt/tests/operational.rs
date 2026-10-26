//! QVT Operational Mappings: compiling and running `examples/qvt/uml2rdbms.qvto` and smaller
//! transformations over the example's metamodels.

mod common;

use common::*;
use oxy_qvt::{Binding, Language, RunOptions};

const HEADER: &str = "modeltype UML uses 'http://www.example.org/qvt/uml';
modeltype RDBMS uses 'http://www.example.org/qvt/rdbms';
transformation t(in source : UML, out target : RDBMS);
";

fn messages(c: &oxy_qvt::Compiled) -> Vec<String> {
    c.diagnostics.iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect()
}

/// Runs a transformation (source → target) and returns the fixture, the log and the target.
fn run(text: &str, options: &RunOptions) -> (Fixture, oxy_qvt::RunResult, usize) {
    let mut f = fixture();
    let c = f.compile(Language::Operational, text);
    no_errors(&c);
    let (result, out) = f.run(&c, options);
    (f, result, out)
}

#[test]
fn uml_to_rdbms() {
    let (f, result, out) = run(&read("uml2rdbms.qvto"), &RunOptions::default());
    assert!(result.problems.is_empty(), "{:#?}", result.problems);
    let text = f.hutn(out);
    assert!(text.contains("Table {\n        name: \"Person\""), "{text}");
    assert!(text.contains("refersTo: Key \"Company_pk\""), "{text}");
    assert!(!text.contains("Address"), "{text}");
    // A schema, 2 tables, 2 keys, 7 columns (2 ids, 4 attributes, the foreign key's), 1 foreign key.
    assert_eq!(result.created, 13);
}

#[test]
fn reports_problems_of_the_transformation() {
    let f = fixture();
    let text = format!(
        "{HEADER}
main() {{ source.rootObjects()[UML::Package]->map nothing(); }}
mapping UML::Package::p() : RDBMS::Schema {{ name := self.nam; tables := 3; }}
query UML::Class::q() : Integer {{ return 'x'; }}
helper h() : Integer {{ break; return 1; }}
mapping UML::Class::c() : RDBMS::Table {{ object UML::Class {{}}; }}
"
    );
    let c = f.compile(Language::Operational, &text);
    let m = messages(&c);
    let has = |s: &str| m.iter().any(|x| x.contains(s));
    assert!(has("unknown mapping 'nothing'"), "{m:#?}");
    assert!(has("unknown property 'nam'"), "{m:#?}");
    assert!(has("cannot be assigned to OrderedSet(Table)"), "{m:#?}");
    assert!(has("must be of type Integer, but this returns String"), "{m:#?}");
    assert!(has("'break' outside of a loop"), "{m:#?}");
    assert!(has("objects cannot be created in the input model 'source'"), "{m:#?}");

    let c = f.compile(
        Language::Operational,
        "modeltype X uses 'http://nowhere'; transformation t(in a : X, out b : Y); main() {}",
    );
    let m = messages(&c);
    assert!(m.iter().any(|x| x.contains("cannot find the metamodel 'http://nowhere'")), "{m:#?}");
    assert!(m.iter().any(|x| x.contains("unknown model type 'Y'")), "{m:#?}");
}

#[test]
fn imperative_ocl() {
    let text = format!(
        "{HEADER}
main() {{
    var total := 0;
    var names : Sequence(String) := Sequence{{}};
    source.objectsOfKind(UML::Class)->forEach(c) {{
        total := total + c.attributes->size();
        names += c.name;
    }};
    log('attributes', total);
    var i := 0;
    while (i < 4) {{
        i := i + 1;
        if (i = 2) {{ continue; }} elif (i = 4) {{ break; }};
        log('i = ' + i.toString());
    }};
    log(switch {{ case (total > 100) 'many'; else 'few'; }});
    log(compute (s : String := '') {{ names->forEach(n) {{ s := s + n.substring(1, 1); }} }});
    var l : List(Integer) := List{{}};
    l->add(3);
    l += 4;
    log(l->sum().toString());
    log(names->xcollect(n | n.toLower())->first());
    log(names->selectOne(n | n.startsWith('C')));
    log(names[n | n.size() > 6]->size().repr());
    log(sum(names->size(), 2).toString());
    log(this.sum(1, 1).toString());
    log('%s and %s'.format(Sequence{{'a', 'b'}}));
    assert warning (total > 100) with log('few attributes');
}}
query sum(a : Integer, b : Integer) : Integer {{
    var r := a;
    r := r + b;
    return r;
}}
"
    );
    let (_, result, _) = run(&text, &RunOptions::default());
    assert!(result.problems.iter().all(|p| p.message.contains("assertion failed")), "{:#?}", result.problems);
    assert_eq!(
        result.log,
        vec![
            "attributes: 7",
            "i = 1",
            "i = 3",
            "few",
            "PCA",
            "7",
            "person",
            "Company",
            "2",
            "5",
            "2",
            "a and b",
            "assertion failed (warning): few attributes",
        ]
    );
}

#[test]
fn mappings_disjuncts_inheritance_resolve_and_properties() {
    let text = format!(
        "{HEADER}
configuration property prefix : String;
property count : Integer = 0;
intermediate property UML::Class::tableName : String;

main() {{
    source.objectsOfKind(UML::Class)->forEach(c) {{ c.tableName := prefix + c.name.toUpper(); }};
    var schema := object RDBMS::Schema {{ name := 's'; }};
    schema.tables := source.objectsOfKind(UML::Class)->map table();
    log(this.count.toString());
    var person := source.objectsOfKind(UML::Class)->any(name = 'Person');
    log(person.resolveone(RDBMS::Table).name);
    log(person.resolveIn(UML::Class::persistentTable, t : RDBMS::Table | t.columns->notEmpty())->size().toString());
    log(schema.tables->first().invresolveone(UML::Class).name);
    log(person.xmap plainTable().oclIsInvalid().toString());
}}

mapping UML::Class::table() : RDBMS::Table disjuncts UML::Class::persistentTable, UML::Class::plainTable;

mapping UML::Class::persistentTable() : RDBMS::Table
    when {{ self.persistent }}
    inherits UML::Class::named
{{
    count := count + 1;
    columns += object RDBMS::Column {{ name := 'id'; type := 'NUMBER'; }};
}}

mapping UML::Class::plainTable() : RDBMS::Table
    when {{ not self.persistent }}
    inherits UML::Class::named
{{
}}

abstract mapping UML::Class::named() : RDBMS::Table {{
    name := self.tableName;
}}
"
    );
    let options = RunOptions { configuration: vec![("prefix".into(), "'T_'".into())], ..RunOptions::default() };
    let (f, result, out) = run(&text, &options);
    assert!(result.problems.is_empty(), "{:#?}", result.problems);
    assert_eq!(result.log, vec!["2", "T_PERSON", "1", "Person", "true"]);
    let hutn = f.hutn(out);
    for name in ["T_PERSON", "T_COMPANY", "T_ADDRESS"] {
        assert!(hutn.contains(&format!("name: \"{name}\"")), "{hutn}");
    }
}

#[test]
fn inout_models_are_changed_in_place() {
    let mut f = fixture();
    let c = f.compile(
        Language::Operational,
        "modeltype UML uses 'http://www.example.org/qvt/uml';
transformation rename(inout model : UML);
main() {
    model.objectsOfKind(UML::Class)->forEach(c) { c.name := c.name + 'Entity'; };
    model.objectsOfType(UML::Class)->select(not persistent)->forEach(c) { model.removeElement(c); };
}",
    );
    no_errors(&c);
    let bindings = vec![Binding { parameter: "model".into(), resource: f.source }];
    let result = oxy_qvt::run(&c, &f.mm, &f.env, &mut f.model, &bindings, &RunOptions::default()).unwrap();
    assert!(result.problems.is_empty(), "{:#?}", result.problems);
    let text = f.hutn(f.source);
    assert!(text.contains("name: \"PersonEntity\""), "{text}");
    assert!(!text.contains("Address"), "{text}");
}

#[test]
fn input_models_are_read_only_and_fatal_assertions_end_the_run() {
    let text = format!(
        "{HEADER}
main() {{
    source.objectsOfKind(UML::Class)->forEach(c) {{ c.name := 'x'; }};
    assert fatal (false) with log('stop');
    log('not reached');
}}"
    );
    let (_, result, _) = run(&text, &RunOptions::default());
    assert!(
        result.problems.iter().any(|p| p.message.contains("objects of the input model 'source' cannot be changed")),
        "{:#?}",
        result.problems
    );
    assert_eq!(result.aborted.as_deref(), Some("assertion failed (fatal): stop"));
    assert!(result.log.is_empty(), "{:?}", result.log);
}

#[test]
fn objects_and_clones() {
    let text = format!(
        "{HEADER}
main() {{
    var t := object RDBMS::Table {{
        name := 'a';
        columns += new RDBMS::Column();
        columns->first().name := 'c';
    }};
    var copy := t.deepclone();
    copy.name := 'b';
    log(copy.columns->first().name + ' ' + (copy.columns->first() <> t.columns->first()).toString());
    log(target.rootObjects()->size().toString() + ' ' + target.objectsOfKind(RDBMS::Column)->size().toString());
    log(t.metaClassName() + ' ' + t.columns->first().container().oclAsType(RDBMS::Table).name);
}}"
    );
    let (_, result, _) = run(&text, &RunOptions::default());
    assert!(result.problems.is_empty(), "{:#?}", result.problems);
    assert_eq!(result.log, vec!["c true", "2 2", "Table a"]);
}

#[test]
fn endless_recursion_ends_with_an_error() {
    let text = format!(
        "{HEADER}
main() {{
    log(depth(100000).toString());
    source.rootObjects()[UML::Package]->map loop();
}}
query depth(n : Integer) : Integer {{
    return if n = 0 then 0 else depth(n - 1) + 1 endif;
}}
mapping UML::Package::loop() : RDBMS::Schema {{
    init {{ var x := self.map again(); }}
}}
mapping UML::Package::again() : RDBMS::Schema {{
    init {{ var x := self.map loop(); }}
}}
"
    );
    let (_, result, _) = run(&text, &RunOptions::default());
    assert!(result.problems.iter().any(|p| p.message.contains("maximum call depth")), "{:#?}", result.problems);
}

#[test]
fn calls_as_in_eclipse_qvto() {
    let text = format!(
        "{HEADER}
main() {{
    var person := source.objectsOfKind(UML::Class)->any(name = 'Person');
    log(kind(person));
    log(kind(person.attributes->first()));
    log(kind(null));
    // A mapping called like an operation, and a feature name escaped with `_`.
    log(person.toTable()._name);
    var none : UML::Class := null;
    log(none.label().oclIsUndefined().toString());
    var table : RDBMS::Table := null;
    object table : {{ name := 'created'; }};
    log(table.name);
    // A copy of an input object is made in the output model, where it can be changed.
    var copy := person.deepclone().oclAsType(UML::Class);
    copy.name := 'Copy';
    log(copy.attributes->size().toString() + ' ' + person.name);
}}
helper kind(e : UML::NamedElement) : String {{ return 'element'; }}
helper kind(c : UML::Class) : String {{ return 'class'; }}
query UML::Class::label() : String {{ return self.name; }}
mapping UML::Class::toTable() : RDBMS::Table {{ name := self.name; }}
"
    );
    let (_, result, _) = run(&text, &RunOptions::default());
    assert!(result.problems.is_empty(), "{:#?}", result.problems);
    assert_eq!(result.log, vec!["class", "element", "class", "Person", "true", "created", "4 Person"]);
}
