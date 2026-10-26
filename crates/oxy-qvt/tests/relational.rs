//! QVT Relations: compiling and running `examples/qvt/uml2rdbms.qvtr` and smaller
//! transformations: enforcing, checking, both directions, templates and keys.

mod common;

use common::*;
use oxy_qvt::{Binding, Language, RunOptions, RunResult};

const IMPORTS: &str = "import uml : 'http://www.example.org/qvt/uml';
import rdbms : 'http://www.example.org/qvt/rdbms';
";

fn bind(source: usize, target: usize) -> Vec<Binding> {
    vec![
        Binding { parameter: "source".into(), resource: source },
        Binding { parameter: "target".into(), resource: target },
    ]
}

fn run_with(f: &mut Fixture, c: &oxy_qvt::Compiled, bindings: &[Binding], options: &RunOptions) -> RunResult {
    let r = oxy_qvt::run(c, &f.mm, &f.env, &mut f.model, bindings, options).unwrap();
    assert!(r.problems.is_empty(), "{:#?}", r.problems);
    r
}

#[test]
fn uml_to_rdbms_means_what_the_operational_one_means() {
    let mut f = fixture();
    let c = f.compile(Language::Relational, &read("uml2rdbms.qvtr"));
    no_errors(&c);
    assert_eq!(c.targets(), vec!["target".to_string()]);
    let (result, out) = f.run(&c, &RunOptions::default());
    assert!(result.problems.is_empty(), "{:#?}", result.problems);
    let relational = f.hutn(out);

    let mut g = fixture();
    let c = g.compile(Language::Operational, &read("uml2rdbms.qvto"));
    let (result, out) = g.run(&c, &RunOptions::default());
    assert!(result.problems.is_empty(), "{:#?}", result.problems);
    assert_eq!(relational, g.hutn(out));
}

#[test]
fn enforcing_again_changes_nothing_and_checking_finds_what_is_missing() {
    let mut f = fixture();
    let c = f.compile(Language::Relational, &read("uml2rdbms.qvtr"));
    let out = f.model.add_resource("out.xmi");
    let source = f.source;
    let check = RunOptions { check_only: true, ..RunOptions::default() };

    let r = run_with(&mut f, &c, &bind(source, out), &check);
    assert_eq!(r.violations, vec!["relation PackageToSchema does not hold for p = Package 'company'"]);

    let r = run_with(&mut f, &c, &bind(source, out), &RunOptions::default());
    assert_eq!(r.created, 13);
    let first = f.hutn(out);

    let r = run_with(&mut f, &c, &bind(source, out), &RunOptions::default());
    assert_eq!(r.created, 0, "{}", f.hutn(out));
    assert_eq!(f.hutn(out), first);

    let r = run_with(&mut f, &c, &bind(source, out), &check);
    assert!(r.violations.is_empty(), "{:?}", r.violations);
}

#[test]
fn bidirectional_relations_run_in_either_direction() {
    let mut f = fixture();
    let c = f.compile(
        Language::Relational,
        &format!(
            "{IMPORTS}
transformation sync(source : uml, target : rdbms) {{
    top relation PackageToSchema {{
        pn : String;
        enforce domain source p : uml::Package {{ name = pn }};
        enforce domain target s : rdbms::Schema {{ name = pn }};
    }}
    top relation ClassToTable {{
        cn : String;
        enforce domain source c : uml::Class {{ namespace = p : uml::Package {{}}, name = cn, persistent = true }};
        enforce domain target t : rdbms::Table {{ schema = s : rdbms::Schema {{}}, name = cn }};
        when {{ PackageToSchema(p, s); }}
    }}
}}"
        ),
    );
    no_errors(&c);
    assert_eq!(c.targets(), vec!["source".to_string(), "target".to_string()]);
    let out = f.model.add_resource("out.xmi");
    let source = f.source;
    let err = oxy_qvt::run(&c, &f.mm, &f.env, &mut f.model, &bind(source, out), &RunOptions::default()).unwrap_err();
    assert!(err.contains("choose one"), "{err}");

    let to_rdbms = RunOptions { target: Some("target".into()), ..RunOptions::default() };
    run_with(&mut f, &c, &bind(source, out), &to_rdbms);
    let schema = f.hutn(out);
    assert!(schema.contains("name: \"Person\"") && !schema.contains("Address"), "{schema}");

    let back = f.model.add_resource("back.xmi");
    let to_uml = RunOptions { target: Some("source".into()), ..RunOptions::default() };
    run_with(&mut f, &c, &bind(back, out), &to_uml);
    let classes = f.hutn(back);
    assert!(classes.contains("Package {\n    name: \"company\""), "{classes}");
    assert!(classes.contains("name: \"Company\"\n        persistent: true"), "{classes}");
}

#[test]
fn templates_keys_and_overrides() {
    let mut f = fixture();
    let c = f.compile(
        Language::Relational,
        &format!(
            "{IMPORTS}
transformation t(source : uml, target : rdbms) {{
    key rdbms::Table {{ schema, name }};

    top relation PackageToSchema {{
        pn : String;
        checkonly domain source p : uml::Package {{
            name = pn,
            elements = es : OrderedSet(uml::PackageableElement) {{ e ++ _ }}
        }};
        enforce domain target s : rdbms::Schema {{ name = pn }};
        where {{ ElementToTable(e, s, 'T_'); }}
    }}

    abstract relation ElementToTable {{
        checkonly domain source e : uml::PackageableElement {{}};
        enforce domain target s : rdbms::Schema {{}};
        primitive domain prefix : String;
    }}

    relation ClassToTable overrides ElementToTable {{
        cn : String;
        checkonly domain source e : uml::Class {{ name = cn }};
        enforce domain target s : rdbms::Schema {{ tables = t : rdbms::Table {{ name = prefix + cn }} }};
        primitive domain prefix : String;
    }}

    relation TypeToTable overrides ElementToTable {{
        tn : String;
        checkonly domain source e : uml::PrimitiveType {{ name = tn }};
        enforce domain target s : rdbms::Schema {{ tables = t : rdbms::Table {{ name = prefix + 'type_' + tn }} }};
        primitive domain prefix : String;
    }}

    -- A column per type and attribute name: equal ones are one column (check before enforce).
    top relation TypeUsage {{
        tn, an : String;
        checkonly domain source p : uml::PrimitiveType {{
            name = tn,
            opposite(uml::Attribute.type) = a : uml::Attribute {{ name = an }}
        }};
        enforce domain target c : rdbms::Column {{ name = tn + '.' + an, type = tn }};
    }}
}}"
        ),
    );
    no_errors(&c);
    let (r, out) = f.run(&c, &RunOptions::default());
    assert!(r.problems.is_empty(), "{:#?}", r.problems);
    let text = f.hutn(out);
    for table in ["T_Person", "T_Company", "T_Address", "T_type_String", "T_type_Integer", "T_type_Boolean"] {
        assert!(text.contains(&format!("name: \"{table}\"")), "{table}: {text}");
    }
    for column in ["String.name", "Integer.age", "Boolean.listed", "String.street"] {
        assert_eq!(text.matches(&format!("name: \"{column}\"")).count(), 1, "{column}: {text}");
    }
}

#[test]
fn reports_problems_of_the_transformation() {
    let f = fixture();
    let c = f.compile(
        Language::Relational,
        &format!(
            "{IMPORTS}
import x : 'http://nowhere';
transformation t(source : uml, target : rdbms) {{
    top relation R {{
        n : String;
        checkonly domain source c : uml::Class {{ nam = n, persistent = 'yes' }};
        enforce domain target t : rdbms::Table {{ name = n, columns = k : rdbms::Key {{}} }};
        enforce domain nowhere x : rdbms::Table {{}};
        when {{ S(c); n.size(); }}
    }}
    relation S {{
        checkonly domain source c : uml::Class {{}};
        enforce domain target t : rdbms::Table {{}};
    }}
    query q(i : Integer) : String {{ i + 1 }}
}}"
        ),
    );
    let m: Vec<String> = c.diagnostics.iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect();
    let has = |s: &str| m.iter().any(|x| x.contains(s));
    assert!(has("cannot find the metamodel 'http://nowhere'"), "{m:#?}");
    assert!(has("unknown property 'nam' of Class"), "{m:#?}");
    assert!(has("a value of type String does not fit 'persistent'"), "{m:#?}");
    assert!(has("a Key cannot be a value of 'columns'"), "{m:#?}");
    assert!(has("unknown model parameter 'nowhere'"), "{m:#?}");
    assert!(has("the relation 'S' takes 2 argument(s), but 1 were given"), "{m:#?}");
    assert!(has("a predicate must be of type Boolean"), "{m:#?}");
    assert!(has("the result of 'q' must be of type String"), "{m:#?}");
}

#[test]
fn endless_invocations_end_with_an_error() {
    let mut f = fixture();
    let c = f.compile(
        Language::Relational,
        &format!(
            "{IMPORTS}
transformation t(source : uml, target : rdbms) {{
    top relation Top {{
        checkonly domain source p : uml::Package {{}};
        enforce domain target s : rdbms::Schema {{ name = 'x' }};
        where {{ Deeper(p, s, 0); }}
    }}
    relation Deeper {{
        checkonly domain source p : uml::Package {{}};
        enforce domain target s : rdbms::Schema {{}};
        primitive domain n : Integer;
        where {{ Deeper(p, s, n + 1); }}
    }}
}}"
        ),
    );
    no_errors(&c);
    let (r, _) = f.run(&c, &RunOptions::default());
    assert!(r.problems.iter().any(|p| p.message.contains("too deeply")), "{:#?}", r.problems);
}
