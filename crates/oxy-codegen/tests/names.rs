//! Hostile metamodels: names that are keywords or clash with generated names, the same class
//! name in two packages, data types, operations without bodies, and OCL with errors. The
//! generated code must still load (Python) and compile (Rust).

mod common;

use std::path::Path;

use common::*;
use oxy_ecore::{Metamodel, Model};
use oxy_ocl::EnvironmentBuilder;

const ECORE: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<ecore:EPackage xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    xmlns:ecore="http://www.eclipse.org/emf/2002/Ecore" name="weird" nsURI="http://example.org/weird" nsPrefix="weird">
  <eClassifiers xsi:type="ecore:EClass" name="class" abstract="true">
    <eAnnotations source="http://www.eclipse.org/emf/2002/Ecore/OCL/Pivot">
      <details key="lambda" value="type &lt;&gt; null implies type.size() &gt; 0"/>
      <details key="broken" value="self.nope &gt; 0"/>
    </eAnnotations>
    <eOperations name="cast" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EInt">
      <eParameters name="in" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EInt"/>
    </eOperations>
    <eOperations name="result" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EString">
      <eAnnotations source="http://www.eclipse.org/emf/2002/Ecore/OCL/Pivot">
        <details key="body" value="if self.match > 0 then 'positive' else type endif"/>
      </eAnnotations>
    </eOperations>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="type" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EString"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="match" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EInt"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="self" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EBoolean"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="eContainer" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EString"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="values" upperBound="-1" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EInt"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="when" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EDate"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="none" eType="#//Option"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="Model" eSuperTypes="#//class">
    <eStructuralFeatures xsi:type="ecore:EReference" name="children" upperBound="-1" eType="#//Model" containment="true" eOpposite="#//Model/parent"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="parent" eType="#//Model" eOpposite="#//Model/children"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="data" eType="#//Data"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="String" eSuperTypes="#//Model #//Data"/>
  <eClassifiers xsi:type="ecore:EClass" name="Data" interface="true" abstract="true"/>
  <eClassifiers xsi:type="ecore:EEnum" name="Option">
    <eLiterals name="None"/>
    <eLiterals name="Some" value="1"/>
    <eLiterals name="self" value="2"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EDataType" name="Timestamp" instanceClassName="java.util.Date"/>
  <eSubpackages name="sub" nsURI="http://example.org/weird/sub" nsPrefix="sub">
    <eClassifiers xsi:type="ecore:EClass" name="Model">
      <eStructuralFeatures xsi:type="ecore:EAttribute" name="stamp" eType="#//Timestamp"/>
      <eStructuralFeatures xsi:type="ecore:EReference" name="outer" eType="#//Model"/>
    </eClassifiers>
  </eSubpackages>
</ecore:EPackage>
"##;

fn fixture() -> Fixture {
    let mut mm = Metamodel::new();
    let load = mm.load_ecore_str(ECORE, "weird.ecore").unwrap();
    let packages = mm.resources()[load.resource].roots.clone();
    let mut b = EnvironmentBuilder::new(&mm);
    b.add_ecore_constraints();
    let env = b.build();
    assert!(env.has_errors(), "the broken invariant is an error");
    let xmi = r#"<?xml version="1.0" encoding="UTF-8"?>
<weird:String xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:weird="http://example.org/weird" type="t" match="3" none="Some">
  <children type="child"/>
</weird:String>
"#
    .to_string();
    Fixture { name: "names", mm, env, packages, xmi, model: Model::new() }
}

#[test]
fn problems_are_reported_not_fatal() {
    let fixture = fixture();
    for target in ["python", "rust"] {
        let (_, output) = generate(&fixture, target);
        assert!(
            output.problems.iter().any(|p| p.message.contains("broken") && p.message.contains("has errors")),
            "{target}: {:?}",
            output.problems
        );
    }
}

#[test]
fn python_loads() {
    let Some(py) = ["python3", "python"].into_iter().find(|p| available(p, "--version")) else {
        return eprintln!("skipped: no Python");
    };
    let (dir, _) = generate(&fixture(), "python");
    std::fs::write(
        dir.join("check.py"),
        r#"
import weird
from weird import load, validate, OclInvalid
from weird.weird import String, Model, Option
from weird.weird_sub import Model as SubModel
s = load("model.xmi").contents[0]
print(type(s).__name__, s.type, s.match, s.none, s.children[0].parent is s, s.result_())
print(sorted(d.constraint for d in validate(s)))
try:
    s.cast(1)
except NotImplementedError as e:
    print("cast:", e)
print(SubModel(outer=s).outer is s)
"#,
    )
    .unwrap();
    let out = run(&dir, py, &["check.py"]);
    assert_eq!(
        out,
        "String t 3 Option::Some True positive\n\
         ['class::broken', 'class::broken']\n\
         cast: class::cast() has no OCL body\n\
         True\n"
    );
}

#[test]
fn rust_compiles() {
    if !available("cargo", "--version") {
        return eprintln!("skipped: no cargo");
    }
    let (dir, _) = generate(&fixture(), "rust");
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"harness\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nnames = { path = \"weird\", package = \"weird\" }\n\n[workspace]\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("src/main.rs"),
        r#"
use names::weird::*;

fn main() {
    let (m, warnings) = names::Model::load_xmi("model.xmi").unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let s: String_ = m.cast(m.roots()[0]).unwrap();
    println!("{:?} {} {:?} {}", s.type_(&m), s.match_(&m), s.none(&m), s.result_(&m).unwrap().unwrap());
    let mut constraints: Vec<std::string::String> = m.validate().into_iter().map(|d| d.constraint).collect();
    constraints.sort();
    println!("{constraints:?}");
    println!("{}", s.cast_2(&m, 1).unwrap_err());
}
"#,
    )
    .unwrap();
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/codegen-tests/cargo-target");
    let out = run(&dir, "cargo", &["run", "--quiet", "--offline", "--target-dir", &target.to_string_lossy()]);
    assert_eq!(
        out,
        "Some(\"t\") 3 Some positive\n\
         [\"class::broken\", \"class::broken\"]\n\
         invalid: class::cast() has no OCL body\n"
    );
}
