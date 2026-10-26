//! QVTo imports: libraries (imported, `access`ed, `extends`), transformations that extend
//! others and transformations that run accessed ones (`new T(m).transform()`), and the
//! problems of imported units.

mod common;

use common::*;
use oxy_qvt::{Binding, RunOptions};

const MODELTYPES: &str = "modeltype UML uses 'http://www.example.org/qvt/uml';
modeltype RDBMS uses 'http://www.example.org/qvt/rdbms';
";

/// A library with its own names for the model types.
const NAMING: &str = "library Naming;
modeltype U uses 'http://www.example.org/qvt/uml';
modeltype R uses 'http://www.example.org/qvt/rdbms';

property prefix : String = 'T_';
property calls : Integer = 0;

query U::Class::tableName() : String {
    calls := calls + 1;
    return prefix + self.name.toUpper();
}

query label(s : String) : String = 'lib:' + s;

mapping U::Attribute::toColumn() : R::Column {
    name := self.name;
    type := 'VARCHAR';
}
";

fn messages(c: &oxy_qvt::Compiled) -> Vec<String> {
    c.diagnostics.iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect()
}

#[test]
fn libraries_are_used_by_importing_them() {
    let mut f = fixture();
    let text = format!(
        "import lib.Naming;
{MODELTYPES}
transformation t(in source : UML, out target : RDBMS);

main() {{
    source.objectsOfKind(UML::Class)->select(persistent)->map toTable();
    log(label('x'));
    log(Naming::label('x'));
    log(this.Naming::label('y'));
    log(calls.toString());
}}

-- Overrides the library's query; `Naming::label` still calls the library's.
query label(s : String) : String = 'own:' + s;

mapping UML::Class::toTable() : RDBMS::Table {{
    name := self.tableName();
    columns := self.attributes->map toColumn();
}}
"
    );
    let c = f.compile_with_units(&text, &[("lib.Naming", NAMING)]);
    no_errors(&c);
    assert_eq!(c.units, vec!["lib/Naming.qvto"]);
    // The library's operations can be completed in the document.
    assert!(c.operations().iter().any(|s| s.name == "toColumn"));
    let (result, out) = f.run(&c, &RunOptions::default());
    assert!(result.problems.is_empty(), "{:#?}", result.problems);
    assert_eq!(result.log, vec!["own:x", "lib:x", "lib:y", "2"]);
    let hutn = f.hutn(out);
    assert!(hutn.contains("name: \"T_PERSON\""), "{hutn}");
    assert!(hutn.contains("type: \"VARCHAR\""), "{hutn}");
}

#[test]
fn extending_and_accessing_transformations() {
    let mut f = fixture();
    // A base transformation with other names for its parameters.
    let base = format!(
        "{MODELTYPES}
transformation Base(in uml : UML, out rdb : RDBMS);
property schemaName : String = 'base';
main() {{
    object RDBMS::Schema@rdb {{ name := schemaName + ':' + uml.rootObjects()->size().toString(); }};
}}
helper describe() : String {{ return 'base'; }}
"
    );
    // Accessed: run on models of the accessing transformation.
    let tables = format!(
        "{MODELTYPES}
transformation Tables(in src : UML, out dst : RDBMS);
configuration property suffix : String;
main() {{
    src.objectsOfKind(UML::Class)->forEach(c) {{
        object RDBMS::Table@dst {{ name := c.name + suffix; }};
    }};
    log('tables done');
}}
"
    );
    let text = format!(
        "import Base;
import Tables;
{MODELTYPES}
transformation t(in source : UML, out target : RDBMS) extends Base access transformation Tables;
-- Overrides the base's helper; the base's `main` is inherited.
helper describe() : String {{ return 'derived'; }}
"
    );
    let c = f.compile_with_units(&text, &[("Base", &base), ("Tables", &tables)]);
    no_errors(&c);
    let (result, out) = f.run(&c, &RunOptions::default());
    assert!(result.problems.is_empty(), "{:#?}", result.problems);
    let hutn = f.hutn(out);
    assert!(hutn.contains("name: \"base:1\""), "{hutn}");

    let text = format!(
        "import Base;
import Tables;
{MODELTYPES}
transformation t(in source : UML, out target : RDBMS) extends Base access transformation Tables;
main() {{
    log(describe());
    log(Base::describe());
    var tables : Tables := new Tables(source, target);
    tables.suffix := '_T';
    var status : Status := tables.transform();
    log(status.succeeded().toString());
}}
helper describe() : String {{ return 'derived'; }}
"
    );
    let c = f.compile_with_units(&text, &[("Base", &base), ("Tables", &tables)]);
    no_errors(&c);
    let (result, out) = f.run(&c, &RunOptions::default());
    assert!(result.problems.is_empty(), "{:#?}", result.problems);
    assert_eq!(result.log, vec!["derived", "base", "tables done", "true"]);
    let hutn = f.hutn(out);
    assert!(hutn.contains("name: \"Person_T\""), "{hutn}");
}

#[test]
fn problems_of_imports() {
    let f = fixture();
    // A unit that is missing, one with errors (reported at its import), an unknown module.
    let broken = "library Broken;\nquery q() : Integer = 'text';\n";
    let text = format!(
        "import lib.Missing;
import Broken;
{MODELTYPES}
transformation t(in source : UML, out target : RDBMS) access library Nothing;
main() {{}}
"
    );
    let c = f.compile_with_units(&text, &[("Broken", broken)]);
    let m = messages(&c);
    let has = |s: &str| m.iter().any(|x| x.contains(s));
    assert!(has("cannot find the unit 'lib.Missing': lib/Missing.qvto"), "{m:#?}");
    assert!(has("the imported unit has errors: Broken.qvto:2:"), "{m:#?}");
    assert!(has("unknown module 'Nothing'"), "{m:#?}");
    let errors: Vec<_> = c.diagnostics.iter().filter(|d| d.is_error()).collect();
    assert!(errors.iter().all(|d| (d.span.end as usize) <= text.len()), "{errors:#?}");

    // Libraries that import each other.
    let a = "library A;\nimport B;\nquery a() : String = 'a' + b0();\nquery a0() : String = 'a0';\n";
    let b = "library B;\nimport A;\nquery b() : String = 'b' + a0();\nquery b0() : String = 'b0';\n";
    let text = format!(
        "import A;\n{MODELTYPES}transformation t(in source : UML, out target : RDBMS);\nmain() {{ log(a() + b()); }}\n"
    );
    let c = f.compile_with_units(&text, &[("A", a), ("B", b)]);
    no_errors(&c);

    // A library is not run itself.
    let c = f.compile_with_units(NAMING, &[]);
    no_errors(&c);
    assert!(c.library);
    let mut model = f.model.clone();
    let err = oxy_qvt::run(&c, &f.mm, &f.env, &mut model, &[], &RunOptions::default()).err().unwrap();
    assert!(err.contains("is a library"), "{err}");
}

#[test]
fn run_problems_in_imported_units_are_located_there() {
    let mut f = fixture();
    let lib = "library L;\nmodeltype U uses 'http://www.example.org/qvt/uml';\nquery U::Class::bad() : String {\n    return self.attributes->first().nothing;\n}\n";
    let text = format!(
        "import L;
{MODELTYPES}
transformation t(in source : UML, out target : RDBMS);
main() {{ var x := source.objectsOfKind(UML::Class)->any(true).bad(); }}
"
    );
    let c = f.compile_with_units(&text, &[("L", lib)]);
    // `nothing` is unknown: the library has errors.
    assert!(messages(&c).iter().any(|m| m.contains("L.qvto:4")), "{:#?}", c.diagnostics);

    // A statement of the library that fails.
    let lib = "library L;\nmodeltype U uses 'http://www.example.org/qvt/uml';\nquery U::Class::bad() : String {\n    var n := self.attributes->at(99).name;\n    return 'x';\n}\n";
    let c = f.compile_with_units(&text, &[("L", lib)]);
    no_errors(&c);
    let out = f.model.add_resource("out.xmi");
    let bindings = vec![
        Binding { parameter: "source".into(), resource: f.source },
        Binding { parameter: "target".into(), resource: out },
    ];
    let result = oxy_qvt::run(&c, &f.mm, &f.env, &mut f.model, &bindings, &RunOptions::default()).unwrap();
    let p = result.problems.iter().find(|p| p.unit.is_some()).expect("a problem in the library");
    assert_eq!(p.unit.as_deref(), Some("L.qvto"));
    let span = p.span.unwrap();
    assert_eq!(&lib[span.start as usize..span.end as usize], "n := self.attributes->at(99).name");
}
