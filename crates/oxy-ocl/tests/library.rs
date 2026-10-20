use oxy_ecore::{Metamodel, Model, XmiDocument};
use oxy_ocl::syntax::parse_expression;
use oxy_ocl::*;

const EXAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/library");

fn setup() -> (Metamodel, Model) {
    let mut mm = Metamodel::new();
    mm.load_ecore_file(format!("{EXAMPLES}/library.ecore")).unwrap();
    let mut model = Model::new();
    XmiDocument::read_file(format!("{EXAMPLES}/library.xmi")).unwrap().load(&mm, &mut model).unwrap();
    (mm, model)
}

fn env(mm: &Metamodel, doc: Option<&str>) -> Environment {
    let mut b = EnvironmentBuilder::new(mm);
    b.add_ecore_constraints();
    if let Some(text) = doc {
        b.add_document("test.ocl", text);
    }
    b.build()
}

fn diagnostics(env: &Environment) -> Vec<String> {
    env.diagnostics()
        .iter()
        .map(|d| {
            let src = env.source(d.source);
            let pos = src.position(d.diagnostic.span.start);
            format!("{}:{}:{}: {}", src.name, pos.line + 1, pos.column + 1, d.diagnostic.message)
        })
        .collect()
}

/// Evaluates `expr` with `self` bound to the first instance of `class` (if given).
fn eval(mm: &Metamodel, model: &Model, env: &Environment, class: Option<&str>, expr: &str) -> String {
    let (e, d) = parse_expression(expr);
    assert!(d.is_empty(), "{expr}: {d:?}");
    let self_value = class.map(|c| {
        let c = mm.find_classifiers(c)[0];
        Value::Object(model.instances_of(mm, c)[0])
    });
    Evaluator::new(mm, model, env).evaluate(&e, self_value, &[]).render(mm, model)
}

#[test]
fn ecore_constraints_check_cleanly() {
    let (mm, _) = setup();
    let env = env(&mm, None);
    assert_eq!(diagnostics(&env), Vec::<String>::new());
    assert_eq!(env.invariants().len(), 2);
}

#[test]
fn example_document_checks_cleanly() {
    let (mm, _) = setup();
    let text = std::fs::read_to_string(format!("{EXAMPLES}/library.ocl")).unwrap();
    let env = env(&mm, Some(&text));
    assert_eq!(diagnostics(&env), Vec::<String>::new());
    assert_eq!(env.invariants().len(), 8);
}

#[test]
fn validates_library() {
    let (mm, model) = setup();
    let text = std::fs::read_to_string(format!("{EXAMPLES}/library.ocl")).unwrap();
    let env = env(&mm, Some(&text));
    let results = validate(&mm, &model, &env);
    let mut failures: Vec<String> = results
        .iter()
        .filter(|r| r.status != Status::Satisfied)
        .map(|r| {
            let inv = &env.invariants()[r.invariant];
            format!(
                "{} on {}: {:?}{}",
                inv.name.as_deref().unwrap_or("?"),
                model.label(&mm, r.object),
                r.status,
                r.message.as_ref().map(|m| format!(" ({m})")).unwrap_or_default()
            )
        })
        .collect();
    failures.sort();
    assert_eq!(
        failures,
        vec![
            "Adult on Member 'Bob': Violated (Member Bob is 16 years old)",
            "HasIsbn on Book 'The Guns of August': Violated (Book 'The Guns of August' has no ISBN)",
            "SufficientCopies on Book 'A Brief History of Time': Violated",
        ]
    );
}

#[test]
fn evaluates_expressions() {
    let (mm, model) = setup();
    let text = std::fs::read_to_string(format!("{EXAMPLES}/library.ocl")).unwrap();
    let env = env(&mm, Some(&text));
    let cases: &[(Option<&str>, &str, &str)] = &[
        (None, "1 + 2 * 3", "7"),
        (None, "7 / 2", "3.5"),
        (None, "7.div(2) + 7.mod(2)", "4"),
        (None, "'abc'.toUpper().substring(2, 3)", "'BC'"),
        (None, "Sequence{3, 1, 2}->sortedBy(x | x)", "Sequence{1, 2, 3}"),
        (None, "Set{1, 2, 2, 3}->size()", "3"),
        (None, "Bag{1, 2, 2}->count(2)", "2"),
        (None, "Sequence{1..4}->select(x | x.mod(2) = 0)", "Sequence{2, 4}"),
        (None, "Sequence{1, 2, 3}->iterate(x; acc : Integer = 0 | acc + x)", "6"),
        (None, "let x = 5 in if x > 3 then 'big' else 'small' endif", "'big'"),
        (None, "Tuple{a = 1, b = 'x'}.b", "'x'"),
        (None, "Sequence{1, 2} = Sequence{1, 2} and Set{1, 2} = Set{2, 1}", "true"),
        (None, "1 = 1.0", "true"),
        (None, "(1 / 0).oclIsInvalid()", "true"),
        (None, "false and (1 / 0 > 1)", "false"),
        (None, "null.oclIsUndefined()", "true"),
        (None, "Library.allInstances()->size()", "1"),
        (None, "Book.allInstances()->collect(copies)->sum()", "3"),
        (None, "Person.allInstances()->size()", "5"),
        (None, "Genre::SCIENCE", "Genre::SCIENCE"),
        (Some("Library"), "books->select(genre = Genre::FICTION).name", "Sequence{'Dune'}"),
        (Some("Library"), "books.name", "Sequence{'Dune', 'A Brief History of Time', 'The Guns of August'}"),
        (Some("Library"), "members->any(age > 50).name", "'Carol'"),
        (Some("Library"), "members->exists(m | m.age < 18)", "true"),
        (Some("Library"), "loans.member->asSet()->size()", "2"),
        (Some("Library"), "books->first().isAvailable()", "true"),
        (Some("Library"), "books->at(2).isAvailable()", "false"),
        (Some("Library"), "books->at(2).loanCount", "2"),
        (Some("Library"), "books->at(2).borrowers.name->sortedBy(n | n)", "Sequence{'Alice', 'Bob'}"),
        (Some("Library"), "books->first().library = self", "true"),
        (Some("Library"), "books->first().oclContainer() = self", "true"),
        (Some("Library"), "self.oclIsKindOf(Library) and not self.oclIsKindOf(Book)", "true"),
        (Some("Library"), "members->first().oclAsType(Person).name", "'Alice'"),
        (Some("Library"), "members->first().oclAsType(Book)", "invalid (cannot cast Member 'Alice' to Book)"),
        (Some("Library"), "eContents()->selectByType(Member)->size()", "4"),
        (Some("Book"), "authors->collect(a | a.name)", "Sequence{'Frank Herbert'}"),
        (Some("Book"), "isbn.size() > 0", "true"),
        (Some("Member"), "loans->collect(book.name)", "Sequence{'Dune', 'A Brief History of Time'}"),
        (Some("Member"), "library", "invalid (unknown name 'library')"),
    ];
    let mut failures = Vec::new();
    for (class, expr, expected) in cases {
        let got = eval(&mm, &model, &env, *class, expr);
        if got != *expected {
            failures.push(format!("{expr}\n    expected: {expected}\n    got:      {got}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn reports_type_errors() {
    let (mm, _) = setup();
    let doc = "package library
context Book
inv A: copies > 'x'
inv B: nonsense > 0
inv C: authors->size()
inv D: library.books->forAll(b | b.titel <> '')
inv E: copies.substring(1, 2) = ''
context Nope inv: true
endpackage";
    let env = env(&mm, Some(doc));
    let diags = diagnostics(&env);
    let expected = [
        "test.ocl:3:8: operator '>' is not defined for Integer and String",
        "test.ocl:4:8: unknown name 'nonsense' (not a variable, a property of Book, or a type)",
        "test.ocl:5:8: an invariant must be of type Boolean, but this expression is of type Integer",
        "test.ocl:6:36: unknown property 'titel' for type Book",
        "test.ocl:7:15: unknown operation 'substring' for type Integer",
        "test.ocl:8:9: unknown type 'Nope'",
    ];
    assert_eq!(diags, expected);
}

#[test]
fn resolves_types_in_package_context() {
    let pkg = |name: &str| {
        format!(
            r#"<ecore:EPackage xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    xmlns:ecore="http://www.eclipse.org/emf/2002/Ecore" name="{name}" nsURI="http://{name}" nsPrefix="{name}">
  <eClassifiers xsi:type="ecore:EClass" name="Item"/>
</ecore:EPackage>"#
        )
    };
    let mut mm = Metamodel::new();
    mm.load_ecore_str(&pkg("a"), "a.ecore").unwrap();
    mm.load_ecore_str(&pkg("b"), "b.ecore").unwrap();
    let mut model = Model::new();
    let xmi = r#"<xmi:XMI xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:a="http://a" xmlns:b="http://b">
  <a:Item/><b:Item/><b:Item/>
</xmi:XMI>"#;
    XmiDocument::parse(xmi, "m.xmi").unwrap().load(&mm, &mut model).unwrap();

    let doc = "package b context Item inv Two: Item.allInstances()->size() = 2 endpackage
               context a::Item inv One: a::Item.allInstances()->size() = 1
               context Item inv: true";
    let env = env(&mm, Some(doc));
    assert_eq!(diagnostics(&env), vec!["test.ocl:3:24: ambiguous type 'Item': could be a::Item, b::Item"]);
    let results = validate(&mm, &model, &env);
    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|r| r.status == Status::Satisfied), "{results:?}");
}

#[test]
fn type_operations_on_collections() {
    let (mm, model) = setup();
    let env = env(&mm, None);
    let cases: &[(&str, &str)] = &[
        // Classic implicit collect: cast/test each element.
        ("members.oclAsType(Person)->collect(name)->size()", "4"),
        ("members.oclIsKindOf(Person)", "Sequence{true, true, true, true}"),
        ("Bag{members, authors}->flatten().oclIsTypeOf(Member)->count(true)", "4"),
        // Collection-typed argument or other OclAny operations: the collection itself.
        ("members.oclIsKindOf(Collection(Member))", "true"),
        ("members.oclIsUndefined()", "false"),
        ("members->select(oclIsTypeOf(Member)).oclAsType(Member)->exists(age < 18)", "true"),
    ];
    let mut failures = Vec::new();
    for (expr, expected) in cases {
        let got = eval(&mm, &model, &env, Some("Library"), expr);
        if got != *expected {
            failures.push(format!("{expr}\n    expected: {expected}\n    got:      {got}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn oclinecore_round_trip() {
    let (mm, _) = setup();
    let resource = mm.package(mm.user_root_packages().next().unwrap()).resource;
    let text = oxy_ocl::oclinecore::print(&mm, resource);

    // Built from text (in a metamodel without the .ecore file), the metamodel is the same.
    let mut copy = Metamodel::new();
    let built = oxy_ocl::oclinecore::load(&mut copy, "library.oclinecore", &text);
    assert_eq!(built.diagnostics, Vec::new(), "{text}");
    assert_eq!(oxy_ocl::oclinecore::print(&copy, built.resource), text);
    let mut b = EnvironmentBuilder::new(&copy);
    b.add_oclinecore("library.oclinecore", &text, &built);
    let env = b.build();
    assert_eq!(diagnostics(&env), Vec::<String>::new());
    assert_eq!(env.invariants().len(), 2);
    let book = copy.find_classifiers("Book")[0];
    let loan_count = copy.lookup_feature(book, "loanCount").unwrap();
    assert!(env.derivation(loan_count).is_some());

    // The model loads into the copy and validates the same way.
    let mut model = Model::new();
    XmiDocument::read_file(format!("{EXAMPLES}/library.xmi")).unwrap().load(&copy, &mut model).unwrap();
    let violated: Vec<_> = validate(&copy, &model, &env).into_iter().filter(|r| r.status == Status::Violated).collect();
    assert_eq!(violated.len(), 1);
}

#[test]
fn oclinecore_reports_problems_in_the_document() {
    let text = "package shop : shop = 'http://shop' {\n  class Item {\n    attribute price : Integer;\n    property owner : Customer;\n    invariant Positive: price > 'zero';\n  }\n}\n";
    let mut mm = Metamodel::new();
    let built = oxy_ocl::oclinecore::load(&mut mm, "shop.oclinecore", text);
    let mut b = EnvironmentBuilder::new(&mm);
    b.add_oclinecore("shop.oclinecore", text, &built);
    let env = b.build();
    assert_eq!(
        diagnostics(&env),
        [
            "shop.oclinecore:4:22: unknown type 'Customer'",
            "shop.oclinecore:5:25: operator '>' is not defined for Integer and String",
        ]
    );
    // The rest of the metamodel is built; the OCL is stored for export.
    let item = mm.find_classifiers("Item")[0];
    let price = mm.lookup_feature(item, "price").unwrap();
    assert_eq!(mm.classifier(mm.feature(price).ty.unwrap()).name, "EBigInteger");
    assert_eq!(mm.feature(price).multiplicity.lower, 1);
    let ecore = mm.write_ecore(built.resource);
    assert!(ecore.contains(r#"<details key="Positive" value="price &gt; 'zero'"/>"#), "{ecore}");
}

#[test]
fn checks_pre_and_postconditions() {
    let (mm, model) = setup();
    let doc = r#"
        context library::Book::isAvailable() : Boolean
        pre Stocked('no copies of ' + name): copies > 0
        post Consistent: result = (loanCount < copies@pre)
        post Wrong: result
    "#;
    let env = env(&mm, Some(doc));
    assert_eq!(diagnostics(&env), Vec::<String>::new());

    // At calls: failed conditions are recorded (the result is not affected).
    let (expr, _) = parse_expression("library::Book.allInstances()->collect(isAvailable())");
    let mut ev = Evaluator::new(&mm, &model, &env);
    let value = ev.evaluate(&expr, None, &[]);
    assert_eq!(value.render(&mm, &model), "Bag{true, false, false}");
    let checks: Vec<String> = ev
        .take_contract_checks()
        .iter()
        .map(|c| {
            let oc = &env.operation_constraints()[c.constraint];
            format!(
                "{} {}: {:?} {}",
                oc.name.as_deref().unwrap(),
                model.label(&mm, c.object),
                c.status,
                c.message.as_deref().unwrap_or("")
            )
        })
        .collect();
    assert_eq!(
        checks,
        [
            "Wrong Book 'A Brief History of Time': Violated ",
            "Stocked Book 'The Guns of August': Violated no copies of The Guns of August",
            "Wrong Book 'The Guns of August': Violated ",
        ]
    );

    // Validation calls the operation on every book; calls whose precondition fails are skipped.
    let objects: Vec<_> = model.object_ids().collect();
    let v = validate_model(&mm, &model, &env, &objects, true);
    let outcomes: Vec<String> = v
        .contracts
        .iter()
        .map(|c| format!("{} {:?}", env.operation_constraints()[c.constraint].name.as_deref().unwrap(), c.status))
        .collect();
    assert_eq!(outcomes, ["Consistent Satisfied", "Wrong Satisfied", "Consistent Satisfied", "Wrong Violated"]);
    assert!(v.unchecked.is_empty());

    // `@pre` belongs in postconditions only.
    let env = self::env(&mm, Some("context library::Book inv: copies@pre >= 0"));
    assert_eq!(diagnostics(&env), ["test.ocl:1:28: '@pre' can only be used in postconditions"]);
}

#[test]
fn reports_operations_whose_contracts_cannot_be_enumerated() {
    let text = r#"package shop : shop = 'http://shop' {
  class Item {
    attribute price : Integer;
    operation discounted(percent : Integer) : Integer {
      precondition: percent >= 0 and percent <= 100;
      body: price - (price * percent).div(100);
      postcondition: result <= price;
    }
    operation free() : Boolean {
      postcondition: result = (price = 0);
    }
  }
}"#;
    let mut mm = Metamodel::new();
    let built = oxy_ocl::oclinecore::load(&mut mm, "shop.oclinecore", text);
    let mut b = EnvironmentBuilder::new(&mm);
    b.add_oclinecore("shop.oclinecore", text, &built);
    let env = b.build();
    assert_eq!(diagnostics(&env), Vec::<String>::new());
    let mut model = Model::new();
    let resource = model.add_resource("items");
    let item = mm.find_classifiers("Item")[0];
    let o = model.create_object(&mm, item, resource, None);
    model.add(&mm, o, mm.lookup_feature(item, "price").unwrap(), oxy_ecore::ModelValue::Integer(80));
    let v = validate_model(&mm, &model, &env, &[o], true);
    let reasons: Vec<&str> = v.unchecked.iter().map(|u| u.reason.as_str()).collect();
    assert_eq!(
        reasons,
        [
            "parameter 'percent' of type Integer cannot be enumerated; its postconditions are checked where it is called",
            "it has no OCL body, so its postconditions cannot be checked",
        ]
    );
    // Called from an expression, both conditions of `discounted` are checked.
    let (expr, _) = parse_expression("discounted(150)");
    let mut ev = Evaluator::new(&mm, &model, &env);
    assert_eq!(ev.evaluate(&expr, Some(Value::Object(o)), &[]).render(&mm, &model), "-40");
    let statuses: Vec<_> =
        ev.take_contract_checks().into_iter().map(|c| env.operation_constraints()[c.constraint].kind).collect();
    assert_eq!(statuses, [oxy_ocl::syntax::ast::ConstraintKind::Pre]);
}
