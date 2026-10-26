//! Conformance of the generated code with the engine: every check is a Boolean `def:`
//! attribute that the engine evaluates to true (so the expectation itself is verified), and
//! the code generated for every target must compute true as well. Known gaps of a target
//! are listed in `deviations`; adding a target means adding it here.

mod common;

use std::path::Path;

use common::*;
use oxy_ocl::{Evaluator, Value};

const METAMODEL: &str = r#"
package conf : conf = 'http://www.example.org/conformance' {
  enum Color { literal RED; literal GREEN; literal BLUE; }
  class Root {
    attribute name : String;
    property items : Item[*] { ordered, composes };
  }
  class Item {
    attribute name : String;
    attribute n : Integer;
    attribute r : Real[?];
    attribute color : Color;
    attribute tags : String[*] { ordered, !unique };
    attribute opt : Integer[?];
    attribute label : String[?] { initial: name + '!'; }
    property next : Item[?];
    operation twice() : Integer { body: n * 2; }
  }
  class Special extends Item {
    attribute extra : String[?];
  }
}
"#;

const MODEL: &str = r#"
@Spec { metamodel "conf" { nsUri: "http://www.example.org/conformance" } }
conf {
  Root "root" {
    name: "root"
    items:
      Item "a" { name: "a" n: 1 r: 1.5 color: RED tags: "t1", "t2", "t1" next: Item "b" },
      Item "b" { name: "b" n: 2 opt: 5 color: GREEN label: "set" next: Special "c" },
      Special "c" { name: "c" n: 3 color: BLUE extra: "x" }
  }
}
"#;

/// The checks: each must be true.
const CHECKS: &[&str] = &[
    // numbers
    "7.div(2) = 3",
    "(-7).div(2) = -3",
    "7.mod(3) = 1",
    "(-7).mod(3) = -1",
    "7 / 2 = 3.5",
    "1 + 2 * 3 = 7",
    "(2.5).floor() = 2",
    "(2.5).round() = 3",
    "(-2.5).round() = -2",
    "(-3).abs() = 3",
    "(-2.5).abs() = 2.5",
    "3.max(5) = 5",
    "3.min(2.5) = 2.5",
    "(3.7).toInteger() = 3",
    "3.toReal() = 3.0",
    "1 = 1.0",
    "2 < 2.5",
    "1.div(0).oclIsInvalid()",
    "(1 / 0).oclIsInvalid()",
    "-(3) = -3",
    "1.toString() = '1'",
    "(1.5).toString() = '1.5'",
    "(2.0).toString() = '2.0'",
    "true.toString() = 'true'",
    "(true xor false) and (false implies true) and not false",
    // strings
    "'abc'.size() = 3",
    "'ab'.concat('cd') = 'abcd'",
    "'hello'.substring(2, 4) = 'ell'",
    "'abc'.substring(3, 5).oclIsInvalid()",
    "'42'.toInteger() = 42",
    "' 2.5 '.toReal() = 2.5",
    "'true'.toBoolean()",
    "'aBc'.toUpper() = 'ABC'",
    "'aBc'.toLowerCase() = 'abc'",
    "'  x '.trim() = 'x'",
    "'banana'.indexOf('an') = 2",
    "'banana'.lastIndexOf('an') = 4",
    "'banana'.indexOf('x') = 0",
    "'abc'.at(2) = 'b'",
    "'abc'.characters() = Sequence{'a', 'b', 'c'}",
    "'Abc'.equalsIgnoreCase('aBC')",
    "'abc'.startsWith('ab') and 'abc'.endsWith('bc')",
    "'a-b-c'.substituteAll('-', '+') = 'a+b+c'",
    "'a-b-c'.substituteFirst('-', '+') = 'a+b-c'",
    "'a b  c'.tokenize() = Sequence{'a', 'b', 'c'}",
    "'a,b;c'.tokenize(',;') = Sequence{'a', 'b', 'c'}",
    "'a,b'.tokenize(',', true) = Sequence{'a', ',', 'b'}",
    "'ab' + 'c' = 'abc'",
    "'abc' < 'abd'",
    "'abc123'.matches('[a-z]+[0-9]+')",
    "'a1b2'.replaceAll('[0-9]', '#') = 'a#b#'",
    "'a1b2'.replaceFirst('[0-9]', '#') = 'a#b2'",
    // collections
    "Sequence{1, 2, 3}->size() = 3",
    "Set{1, 2, 2}->size() = 2",
    "Bag{1, 2, 2}->count(2) = 2",
    "Sequence{1, 2}->includes(2) and Sequence{1, 2}->excludes(3)",
    "Sequence{1, 2, 3}->includesAll(Sequence{1, 3})",
    "Sequence{1, 2}->excludesAll(Sequence{3, 4})",
    "Sequence{1}->excluding(1)->isEmpty() and Sequence{1}->notEmpty()",
    "Sequence{1, 2, 3}->sum() = 6",
    "Sequence{1.5, 2.5}->sum() = 4.0",
    "Sequence{3, 1, 2}->max() = 3 and Sequence{3, 1, 2}->min() = 1",
    "Sequence{1, 2, 1}->asSet() = Set{1, 2}",
    "Set{1, 2} = Set{2, 1}",
    "Bag{1, 1, 2} = Bag{1, 2, 1}",
    "Sequence{1, 2} <> Sequence{2, 1}",
    "Sequence{Sequence{1, 2}, Sequence{3}}->flatten() = Sequence{1, 2, 3}",
    "Sequence{1, 2}->including(3) = Sequence{1, 2, 3}",
    "Set{1, 2}->including(2) = Set{1, 2}",
    "Sequence{1, 2, 1}->excluding(1) = Sequence{2}",
    "Sequence{1, 2, 3}->excludingAll(Sequence{1, 3}) = Sequence{2}",
    "Set{1, 2}->union(Set{2, 3}) = Set{1, 2, 3}",
    "Sequence{1, 2}->union(Sequence{2}) = Sequence{1, 2, 2}",
    "Set{1, 2, 3}->intersection(Set{2, 3, 4}) = Set{2, 3}",
    "Bag{1, 1, 2}->intersection(Bag{1, 2, 2}) = Bag{1, 2}",
    "Set{1, 2}->symmetricDifference(Set{2, 3}) = Set{1, 3}",
    "Set{1, 2, 3} - Set{2} = Set{1, 3}",
    "Sequence{1, 2}->append(3) = Sequence{1, 2, 3}",
    "OrderedSet{1, 2}->append(1) = OrderedSet{2, 1}",
    "Sequence{1, 2}->prepend(0) = Sequence{0, 1, 2}",
    "Sequence{1, 3}->insertAt(2, 2) = Sequence{1, 2, 3}",
    "Sequence{4, 5, 6}->at(2) = 5",
    "Sequence{4, 5, 6}->first() = 4 and Sequence{4, 5, 6}->last() = 6",
    "Sequence{4, 5, 6}->indexOf(6) = 3",
    "Sequence{1, 2, 3, 4}->subSequence(2, 3) = Sequence{2, 3}",
    "Sequence{1, 2, 3}->reverse() = Sequence{3, 2, 1}",
    "Sequence{1}->excluding(1)->first().oclIsInvalid()",
    "Sequence{1, 2, 3}->at(5).oclIsInvalid()",
    "Sequence{1..4} = Sequence{1, 2, 3, 4}",
    "Sequence{1..3}->sum() = 6",
    "Sequence{1, 2}->asOrderedSet()->first() = 1",
    // iterators
    "Sequence{1, 2, 3}->select(x | x > 1) = Sequence{2, 3}",
    "Sequence{1, 2, 3}->reject(x | x > 1) = Sequence{1}",
    "Sequence{1, 2, 3}->collect(x | x * 2) = Sequence{2, 4, 6}",
    "Set{1, 2}->collect(x | x.mod(2)) = Bag{1, 0}",
    "Sequence{1, 2, 3}->forAll(x | x > 0) and Sequence{1, 2, 3}->exists(x | x = 2)",
    "Sequence{1, 2, 3}->one(x | x = 2)",
    "Sequence{1, 2, 3}->any(x | x > 1) = 2",
    "Sequence{1, 2}->any(x | x > 5).oclIsUndefined()",
    "not Sequence{1, 2, 3}->isUnique(x | x.mod(2))",
    "Sequence{3, 1, 2}->sortedBy(x | x) = Sequence{1, 2, 3}",
    "Sequence{1, 2}->forAll(a, b | a + b > 1)",
    "Sequence{1, 2, 3}->iterate(x; acc : Integer = 0 | acc + x) = 6",
    "Sequence{1, 2}->iterate(x; acc : Real = 0 | acc + x) = 3.0",
    "Sequence{1, 2, 3}->collectNested(x | Sequence{x})->size() = 3",
    "Sequence{'a', 'b'}->collect(s | s.toUpper()) = Sequence{'A', 'B'}",
    "Sequence{1, 2, 3}->select(x | x > 5)->isEmpty()",
    // the model
    "self.name.size() = 4",
    "items->size() = 3",
    "items.name = Sequence{'a', 'b', 'c'}",
    "items->select(n > 1).name = Sequence{'b', 'c'}",
    "items->collect(n)->sum() = 6",
    "items->sortedBy(n).name->first() = 'a'",
    "items->closure(next)->size() = 2",
    "items->first().next.name = 'b'",
    "items->last().next.oclIsUndefined()",
    "items->last().next = null",
    "items->first().next <> null",
    "items->selectByKind(Special)->size() = 1",
    "items->selectByType(Item)->size() = 2",
    "items->exists(oclIsKindOf(Special))",
    "items->select(oclIsTypeOf(Special))->size() = 1",
    "items->any(i | i.oclIsKindOf(Special)).oclAsType(Special).extra = 'x'",
    "items->first().oclAsType(Special).oclIsInvalid()",
    "Item.allInstances()->size() = 3 and Special.allInstances()->size() = 1",
    "Color.allInstances()->size() = 3",
    "items->first().color = Color::RED",
    "items->collect(color)->includes(Color::BLUE)",
    "Color::RED < Color::BLUE",
    "Color::GREEN.toString() = 'Color::GREEN'",
    "items->first().tags = Sequence{'t1', 't2', 't1'}",
    "items->first().tags->asSet()->size() = 2",
    "items->first().opt.oclIsUndefined()",
    "items->at(2).opt = 5 and items->at(2).opt + 1 = 6",
    "items->first().oclContainer() = self",
    "self.oclContents()->size() = 3 and self.eAllContents()->size() = 3",
    "items->first().r * 2 = 3.0",
    "items->first().n + 0.5 = 1.5",
    "let x = items->first() in x.n + x.n = 2",
    "(if items->isEmpty() then 0 else 1 endif) = 1",
    "items->first()?.next?.name = 'b'",
    "items->last().next?.name = null",
    "items.name->iterate(s; acc : String = '' | acc + s) = 'abc'",
    "items->first().toString() = 'Item \\'a\\''",
    "items->at(2).r.oclIsUndefined()",
    "(items->at(2).r * 2).oclIsInvalid()",
    "items->first().label = 'a!' and items->at(2).label = 'set'",
    // operations and definitions, called explicitly and implicitly
    "items->first().twice() = 2",
    "items->collect(twice())->sum() = 12",
    "items->select(twice() > 2)->size() = 2",
    "items.twice() = Sequence{2, 4, 6}",
    "plus(1) = 2",
    "items->collect(plus(n)) = Sequence{2, 3, 4}",
    "items->collect(i | plus(i.n))->sum() = 9",
    "names()->size() = 3 and names()->includes('b')",
    "items->forAll(i | i.size2 = i.name.size() * 2)",
    // tuples
    "Tuple{a = 1, b = 'x'}.b = 'x'",
    "Tuple{a = 1, b = 'x'} = Tuple{b = 'x', a = 1}",
    "Sequence{Tuple{n = 1}, Tuple{n = 1}}->isUnique(t | t) = false",
    "items->collect(i | Tuple{name = i.name, next = i.next})->select(t | t.next = null)->size() = 1",
    "Set{1, 2}->product(Set{'a'})->size() = 2",
];

/// Definitions the checks use.
const DEFINITIONS: &str = "
context Root
def: plus(x : Integer) : Integer = x + 1
def: names() : Sequence(String) = items.name
context Item
def: size2 : Integer = name.size() + name.size()
";

/// The checks a target does not pass (by a part of their text), with the reason.
fn deviations(target: &str) -> &'static [(&'static str, &'static str)] {
    match target {
        "rust" => &[
            (".matches(", "regular expressions: the generated crate has no dependencies"),
            (".replaceAll(", "regular expressions: the generated crate has no dependencies"),
            (".replaceFirst(", "regular expressions: the generated crate has no dependencies"),
        ],
        _ => &[],
    }
}

fn fixture() -> Fixture {
    let defs: Vec<String> =
        CHECKS.iter().enumerate().map(|(i, c)| format!("def: c{} : Boolean = {c}", i + 1)).collect();
    let ocl = format!("package conf\n{DEFINITIONS}\ncontext Root\n{}\nendpackage\n", defs.join("\n"));
    textual("conformance", METAMODEL, Some(&ocl), MODEL)
}

#[test]
fn the_engine_agrees_with_the_checks() {
    let fixture = fixture();
    let root = fixture.model.object_ids().next().unwrap();
    let mut ev = Evaluator::new(&fixture.mm, &fixture.model, &fixture.env);
    let mut wrong = Vec::new();
    for i in 1..=CHECKS.len() {
        let (expr, _) = oxy_ocl::syntax::parse_expression(&format!("self.c{i}"));
        let v = ev.evaluate(&expr, Some(Value::Object(root)), &[]);
        if !matches!(v, Value::Boolean(true)) {
            wrong.push(format!("c{i} `{}`: {}", CHECKS[i - 1], v.render(&fixture.mm, &fixture.model)));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Parses `name=value` lines and compares them with the checks.
fn compare(target: &str, out: &str) {
    let allowed = deviations(target);
    let mut wrong = Vec::new();
    for (i, check) in CHECKS.iter().enumerate() {
        let name = format!("c{}", i + 1);
        let value = out.lines().find_map(|l| l.strip_prefix(&format!("{name}="))).unwrap_or("missing");
        let deviation = allowed.iter().any(|(text, _)| check.contains(text));
        if (value == "true") == deviation {
            wrong.push(format!("{name} `{check}`: {value}{}", if deviation { " (expected to deviate)" } else { "" }));
        }
    }
    assert!(wrong.is_empty(), "{target}:\n{}", wrong.join("\n"));
}

#[test]
fn python_agrees() {
    let Some(py) = ["python3", "python"].into_iter().find(|p| available(p, "--version")) else {
        return eprintln!("skipped: no Python");
    };
    let fixture = fixture();
    let (dir, output) = generate(&fixture, "python");
    assert!(output.problems.is_empty(), "{:?}", output.problems);
    std::fs::write(
        dir.join("check.py"),
        format!(
            r#"
from conf import load
root = load("model.xmi").contents[0]
for i in range(1, {}):
    try:
        value = getattr(root, f"c{{i}}")
        value = "true" if value is True else repr(value)
    except Exception as e:
        value = f"error: {{type(e).__name__}}: {{e}}"
    print(f"c{{i}}={{value}}")
"#,
            CHECKS.len() + 1
        ),
    )
    .unwrap();
    compare("python", &run(&dir, py, &["check.py"]));
}

#[test]
fn rust_agrees() {
    if !available("cargo", "--version") {
        return eprintln!("skipped: no cargo");
    }
    let fixture = fixture();
    let (dir, output) = generate(&fixture, "rust");
    let unsupported = output.problems.len();
    assert_eq!(unsupported, deviations("rust").len(), "{:?}", output.problems);
    let calls: Vec<String> = (1..=CHECKS.len()).map(|i| format!("(\"c{i}\", root.c{i}(&m)),")).collect();
    let main = format!(
        r#"
use conf::*;

fn main() {{
    let (m, warnings) = Model::load_xmi("model.xmi").unwrap();
    assert!(warnings.is_empty(), "{{warnings:?}}");
    let root: Root = m.cast(m.roots()[0]).unwrap();
    let results: Vec<(&str, Result<bool, Error>)> = vec![
        {}
    ];
    for (name, value) in results {{
        match value {{
            Ok(true) => println!("{{name}}=true"),
            Ok(false) => println!("{{name}}=false"),
            Err(e) => println!("{{name}}=error: {{e}}"),
        }}
    }}
}}
"#,
        calls.join("\n        ")
    );
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"harness\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nconf = { path = \"conf\" }\n\n[workspace]\n")
    .unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.rs"), main).unwrap();
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/codegen-tests/cargo-target");
    let out = run(&dir, "cargo", &["run", "--quiet", "--offline", "--target-dir", &target.to_string_lossy()]);
    compare("rust", &out);
}
