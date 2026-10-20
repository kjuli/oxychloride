//! XML Schemas generated from Ecore packages (`xsd_writer`).

use oxy_ecore::xml::{self, Element};
use oxy_ecore::xsd_writer::{XMI_SCHEMA, XMI_SCHEMA_FILE, schema_file_name, write_schema};
use oxy_ecore::*;

const EXAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/library");
const LIBRARY_NS: &str = "http://www.example.org/library/1.0";

/// A base package referenced by nsURI from the zoo package.
const BASE_ECORE: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<ecore:EPackage xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    xmlns:ecore="http://www.eclipse.org/emf/2002/Ecore" name="base" nsURI="http://example.org/base" nsPrefix="base">
  <eClassifiers xsi:type="ecore:EClass" name="Named" abstract="true">
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="name" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EString"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EEnum" name="Color">
    <eLiterals name="RED" literal="red"/>
    <eLiterals name="GREEN" value="1" literal="green"/>
  </eClassifiers>
</ecore:EPackage>
"##;

/// Inheritance chains, an abstract class, an interface with a second-supertype use, enums,
/// many-valued attributes, cross-package types, a subpackage and Ecore-typed features.
const ZOO_ECORE: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<ecore:EPackage xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    xmlns:ecore="http://www.eclipse.org/emf/2002/Ecore" name="zoo" nsURI="http://example.org/zoo" nsPrefix="zoo">
  <eClassifiers xsi:type="ecore:EClass" name="Zoo" eSuperTypes="http://example.org/base#//Named">
    <eAnnotations source="http://www.eclipse.org/emf/2002/GenModel">
      <details key="documentation" value="A zoo &amp; its animals."/>
    </eAnnotations>
    <eStructuralFeatures xsi:type="ecore:EReference" name="animals" upperBound="-1" eType="#//Animal" containment="true" eOpposite="#//Animal/zoo"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="keepers" upperBound="-1" eType="#//Keeper" containment="true"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="vets" upperBound="-1" eType="#//care/Vet" containment="true"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="favourite" eType="#//Animal"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="extra" eType="ecore:EClass http://www.eclipse.org/emf/2002/Ecore#//EObject" containment="true"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="tags" upperBound="-1" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EString"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="open" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EBoolean"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="Animal" abstract="true" eSuperTypes="http://example.org/base#//Named">
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="weight" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EDouble"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="color" eType="http://example.org/base#//Color"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="legs" upperBound="-1" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EInt"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="zoo" eType="#//Zoo" eOpposite="#//Zoo/animals"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="age" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EInt" derived="true" transient="true" volatile="true"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="cache" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EString" transient="true"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="label" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EString" volatile="true"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="Mammal" abstract="true" eSuperTypes="#//Animal">
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="fur" eType="http://example.org/base#//Color"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="Pet" abstract="true" interface="true">
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="nickname" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EString"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="owner" eType="#//Keeper" eOpposite="#//Keeper/pets"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="Dog" eSuperTypes="#//Mammal #//Pet">
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="tricks" upperBound="-1" eType="#//Trick"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="Fish" eSuperTypes="#//Animal">
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="water" eType="#//Water"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="Keeper" eSuperTypes="http://example.org/base#//Named">
    <eStructuralFeatures xsi:type="ecore:EReference" name="pets" upperBound="-1" eType="#//Pet" eOpposite="#//Pet/owner"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="badge" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EBigInteger"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="href" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EString"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="Exhibit" eSuperTypes="ecore:EClass http://www.eclipse.org/emf/2002/Ecore#//ENamedElement"/>
  <eClassifiers xsi:type="ecore:EEnum" name="Water">
    <eLiterals name="FRESH"/>
    <eLiterals name="SALT" value="1"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EDataType" name="Trick" instanceClassName="java.lang.String"/>
  <eSubpackages name="care" nsURI="http://example.org/zoo/care" nsPrefix="care">
    <eClassifiers xsi:type="ecore:EClass" name="Vet" eSuperTypes="#//Keeper">
      <eStructuralFeatures xsi:type="ecore:EAttribute" name="licensed" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EBoolean"/>
      <eStructuralFeatures xsi:type="ecore:EReference" name="patients" upperBound="-1" eType="#//Animal"/>
    </eClassifiers>
  </eSubpackages>
</ecore:EPackage>
"##;

fn library() -> (Metamodel, PackageId) {
    let mut mm = Metamodel::new();
    mm.load_ecore_file(format!("{EXAMPLES}/library.ecore")).unwrap();
    let p = mm.package_by_ns_uri(LIBRARY_NS).unwrap();
    (mm, p)
}

fn zoo() -> Metamodel {
    let mut mm = Metamodel::new();
    let load = mm.load_ecore_str(BASE_ECORE, "/virtual/base.ecore").unwrap();
    assert!(load.warnings.is_empty(), "{:?}", load.warnings);
    let load = mm.load_ecore_str(ZOO_ECORE, "/virtual/zoo.ecore").unwrap();
    assert!(load.warnings.is_empty(), "{:?}", load.warnings);
    mm
}

fn schema(mm: &Metamodel, ns_uri: &str) -> Element {
    let p = mm.package_by_ns_uri(ns_uri).unwrap();
    let text = write_schema(mm, p);
    xml::parse(&text, "test.xsd").unwrap_or_else(|e| panic!("{e:?}\n{text}"))
}

fn named<'a>(schema: &'a Element, kind: &str, name: &str) -> Option<&'a Element> {
    schema.children.iter().find(|c| c.local_name() == kind && c.attr("name") == Some(name))
}

fn complex_type<'a>(schema: &'a Element, name: &str) -> &'a Element {
    named(schema, "complexType", name).unwrap_or_else(|| panic!("no complexType {name}"))
}

fn descendants<'a>(el: &'a Element, out: &mut Vec<&'a Element>) {
    for c in &el.children {
        out.push(c);
        descendants(c, out);
    }
}

/// `(name, type)` of the local element or attribute declarations of a complex type
/// (not including its base type's).
fn declared(ty: &Element, kind: &str) -> Vec<(String, String)> {
    let mut all = Vec::new();
    descendants(ty, &mut all);
    all.into_iter()
        .filter(|e| e.local_name() == kind && e.attr("name").is_some())
        .map(|e| (e.attr("name").unwrap().to_string(), e.attr("type").unwrap_or("").to_string()))
        .collect()
}

fn decl_type(ty: &Element, kind: &str, name: &str) -> Option<String> {
    declared(ty, kind).into_iter().find(|(n, _)| n == name).map(|(_, t)| t)
}

fn base_of(ty: &Element) -> Option<&str> {
    let mut all = Vec::new();
    descendants(ty, &mut all);
    all.into_iter().find(|e| e.local_name() == "extension").and_then(|e| e.attr("base"))
}

/// Looks up a declaration in a type and its base types (within one schema).
fn lookup(schema: &Element, ty: &str, kind: &str, name: &str) -> Option<String> {
    let mut current = Some(ty.rsplit(':').next().unwrap().to_string());
    while let Some(t) = current {
        let ct = complex_type(schema, &t);
        if let Some(found) = decl_type(ct, kind, name) {
            return Some(found);
        }
        current = base_of(ct).map(|b| b.rsplit(':').next().unwrap().to_string());
    }
    None
}

/// Checks the structure of an XMI element against the schema: every (unqualified) attribute
/// and child element must be declared by its type.
fn check_object(schema: &Element, el: &Element, ty: &str) {
    let ty = el.attr_ns(xml::XSI_NS, "type").unwrap_or(ty);
    for (k, _) in &el.attrs {
        if k.contains(':') || k == "xmlns" {
            continue;
        }
        assert!(lookup(schema, ty, "attribute", k).is_some(), "attribute {k} not declared for {ty}");
    }
    for c in &el.children {
        let child_ty = lookup(schema, ty, "element", c.local_name())
            .unwrap_or_else(|| panic!("element {} not declared for {ty}", c.local_name()));
        check_object(schema, c, &child_ty);
    }
}

#[test]
fn xmi_schema_declares_emf_attributes_and_wrapper() {
    let s = xml::parse(XMI_SCHEMA, XMI_SCHEMA_FILE).unwrap();
    assert_eq!(s.attr("targetNamespace"), Some(xml::XMI_NS));
    for a in ["id", "uuid", "version", "type", "idref", "label"] {
        assert!(named(&s, "attribute", a).is_some(), "xmi:{a}");
    }
    assert!(named(&s, "attributeGroup", "ObjectAttribs").is_some());
    assert!(named(&s, "element", "XMI").is_some());
    assert!(named(&s, "element", "Extension").is_some());
}

#[test]
fn library_schema_file_name_and_header() {
    let (mm, p) = library();
    assert_eq!(schema_file_name(&mm, p), "library.xsd");
    let s = schema(&mm, LIBRARY_NS);
    assert_eq!(s.local_name(), "schema");
    assert_eq!(s.attr("targetNamespace"), Some(LIBRARY_NS));
    assert_eq!(s.attr("xmlns:library"), Some(LIBRARY_NS));
    assert_eq!(s.attr("elementFormDefault"), Some("unqualified"));
    assert_eq!(s.attr("attributeFormDefault"), Some("unqualified"));
    let imports: Vec<_> = s.children_named("import").collect();
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].attr("namespace"), Some(xml::XMI_NS));
    assert_eq!(imports[0].attr("schemaLocation"), Some(XMI_SCHEMA_FILE));
}

#[test]
fn library_classes_become_complex_types() {
    let (mm, _) = library();
    let s = schema(&mm, LIBRARY_NS);

    let library = complex_type(&s, "Library");
    assert_eq!(decl_type(library, "element", "books").as_deref(), Some("library:Book"));
    assert_eq!(decl_type(library, "attribute", "name").as_deref(), Some("xsd:string"));
    assert!(decl_type(library, "attribute", "books").is_none(), "containments are elements only");

    let book = complex_type(&s, "Book");
    assert_eq!(decl_type(book, "attribute", "copies").as_deref(), Some("xsd:int"));
    assert_eq!(decl_type(book, "attribute", "genre").as_deref(), Some("library:Genre"));
    // Non-containment references: fragment attribute or element with href.
    assert_eq!(decl_type(book, "attribute", "authors").as_deref(), Some("xsd:string"));
    assert_eq!(decl_type(book, "element", "authors").as_deref(), Some("library:Author"));
    // Derived and container references are not serialized.
    assert!(decl_type(book, "attribute", "loanCount").is_none());
    assert!(decl_type(book, "attribute", "library").is_none());
    assert!(decl_type(book, "element", "library").is_none());

    let person = complex_type(&s, "Person");
    assert_eq!(person.attr("abstract"), Some("true"));
    assert_eq!(base_of(complex_type(&s, "Author")), Some("library:Person"));
    let member = complex_type(&s, "Member");
    assert_eq!(base_of(member), Some("library:Person"));
    assert!(decl_type(member, "attribute", "age").is_none(), "inherited from the base type");
    assert_eq!(decl_type(member, "attribute", "loans").as_deref(), Some("xsd:string"));
    // Both sides of a bidirectional non-containment pair are accepted.
    assert!(decl_type(complex_type(&s, "Loan"), "attribute", "member").is_some());

    // Root types carry the XMI object attributes and xmi:Extension.
    let mut all = Vec::new();
    descendants(library, &mut all);
    assert!(all.iter().any(|e| e.local_name() == "attributeGroup" && e.attr("ref") == Some("xmi:ObjectAttribs")));
    assert!(all.iter().any(|e| e.local_name() == "element" && e.attr("ref") == Some("xmi:Extension")));
    let choice = all.iter().find(|e| e.local_name() == "choice").unwrap();
    assert_eq!(choice.attr("minOccurs"), Some("0"));
    assert_eq!(choice.attr("maxOccurs"), Some("unbounded"));

    let genre = named(&s, "simpleType", "Genre").unwrap();
    let mut values = Vec::new();
    descendants(genre, &mut values);
    let values: Vec<&str> = values.iter().filter_map(|e| e.attr("value")).collect();
    assert_eq!(values, ["FICTION", "SCIENCE", "HISTORY"]);

    // Global elements for concrete classes only.
    let globals: Vec<&str> = s.children_named("element").filter_map(|e| e.attr("name")).collect();
    assert_eq!(globals, ["Library", "Book", "Author", "Member", "Loan"]);
    assert_eq!(named(&s, "element", "Library").unwrap().attr("type"), Some("library:Library"));
}

#[test]
fn library_model_matches_schema() {
    let (mm, _) = library();
    let s = schema(&mm, LIBRARY_NS);
    let text = std::fs::read_to_string(format!("{EXAMPLES}/library.xmi")).unwrap();
    let root = xml::parse(&text, "library.xmi").unwrap();
    let global = named(&s, "element", root.local_name()).unwrap();
    check_object(&s, &root, global.attr("type").unwrap());
}

#[test]
fn written_xmi_matches_schema() {
    let (mm, _) = library();
    let doc = XmiDocument::read_file(format!("{EXAMPLES}/library.xmi")).unwrap();
    let mut model = Model::new();
    let load = doc.load(&mm, &mut model).unwrap();
    let text = xmi_writer::write_resource(&mm, &model, load.resource, &[]);
    let root = xml::parse(&text, "written.xmi").unwrap();
    let s = schema(&mm, LIBRARY_NS);
    check_object(&s, &root, "library:Library");
}

#[test]
fn zoo_inheritance_and_cross_package_types() {
    let mm = zoo();
    let zoo_pkg = mm.package_by_ns_uri("http://example.org/zoo").unwrap();
    let care = mm.package_by_ns_uri("http://example.org/zoo/care").unwrap();
    let base = mm.package_by_ns_uri("http://example.org/base").unwrap();
    assert_eq!(schema_file_name(&mm, zoo_pkg), "zoo.xsd");
    assert_eq!(schema_file_name(&mm, care), "zoo.care.xsd");
    assert_eq!(schema_file_name(&mm, base), "base.xsd");

    let s = schema(&mm, "http://example.org/zoo");
    let imports: Vec<(&str, &str)> =
        s.children_named("import").map(|i| (i.attr("namespace").unwrap(), i.attr("schemaLocation").unwrap())).collect();
    assert!(imports.contains(&("http://example.org/base", "base.xsd")), "{imports:?}");
    assert!(imports.contains(&("http://example.org/zoo/care", "zoo.care.xsd")), "{imports:?}");
    assert!(imports.contains(&(xml::XMI_NS, XMI_SCHEMA_FILE)));
    assert!(!imports.iter().any(|(ns, _)| *ns == ECORE_NS_URI), "Ecore is not imported");
    assert_eq!(s.attr("xmlns:base"), Some("http://example.org/base"));

    // Cross-package supertype and enum.
    let zoo = complex_type(&s, "Zoo");
    assert_eq!(base_of(zoo), Some("base:Named"));
    assert_eq!(decl_type(zoo, "element", "vets").as_deref(), Some("care:Vet"));
    assert_eq!(decl_type(zoo, "element", "tags").as_deref(), Some("xsd:string"), "many-valued attribute");
    assert!(decl_type(zoo, "attribute", "tags").is_none());
    assert_eq!(decl_type(zoo, "attribute", "open").as_deref(), Some("xsd:boolean"));
    assert_eq!(decl_type(zoo, "element", "extra").as_deref(), Some("xsd:anyType"), "typed by EObject");
    let mut docs = Vec::new();
    descendants(zoo, &mut docs);
    assert!(docs.iter().any(|e| e.local_name() == "documentation" && e.text == "A zoo & its animals."));

    let animal = complex_type(&s, "Animal");
    assert_eq!(animal.attr("abstract"), Some("true"));
    assert_eq!(decl_type(animal, "attribute", "weight").as_deref(), Some("xsd:double"));
    assert_eq!(decl_type(animal, "attribute", "color").as_deref(), Some("base:Color"));
    assert_eq!(decl_type(animal, "element", "legs").as_deref(), Some("xsd:int"));
    assert_eq!(decl_type(animal, "attribute", "label").as_deref(), Some("xsd:string"), "volatile, not transient");
    for skipped in ["age", "cache", "zoo"] {
        assert!(decl_type(animal, "attribute", skipped).is_none(), "{skipped}");
        assert!(decl_type(animal, "element", skipped).is_none(), "{skipped}");
    }

    // Inheritance chain Dog -> Mammal -> Animal -> base::Named; Pet's features are copied.
    assert_eq!(base_of(complex_type(&s, "Mammal")), Some("zoo:Animal"));
    let dog = complex_type(&s, "Dog");
    assert_eq!(base_of(dog), Some("zoo:Mammal"));
    assert_eq!(decl_type(dog, "attribute", "nickname").as_deref(), Some("xsd:string"));
    assert_eq!(decl_type(dog, "element", "owner").as_deref(), Some("zoo:Keeper"));
    assert_eq!(decl_type(dog, "element", "tricks").as_deref(), Some("xsd:string"), "custom data type");
    assert!(decl_type(dog, "attribute", "fur").is_none(), "inherited from Mammal");
    assert_eq!(complex_type(&s, "Pet").attr("abstract"), Some("true"), "interface");

    let keeper = complex_type(&s, "Keeper");
    assert_eq!(decl_type(keeper, "attribute", "badge").as_deref(), Some("xsd:integer"));
    assert!(decl_type(keeper, "attribute", "href").is_none(), "covered by xmi:LinkAttribs");
    assert_eq!(decl_type(complex_type(&s, "Fish"), "attribute", "water").as_deref(), Some("zoo:Water"));

    // Features inherited from Ecore classes are copied into a root type.
    let exhibit = complex_type(&s, "Exhibit");
    assert!(base_of(exhibit).is_none());
    assert_eq!(decl_type(exhibit, "attribute", "name").as_deref(), Some("xsd:string"));
    assert_eq!(decl_type(exhibit, "element", "eAnnotations").as_deref(), Some("xsd:anyType"));

    let globals: Vec<&str> = s.children_named("element").filter_map(|e| e.attr("name")).collect();
    assert_eq!(globals, ["Zoo", "Dog", "Fish", "Keeper", "Exhibit"]);

    // Enumerations accept literal strings and names.
    let b = schema(&mm, "http://example.org/base");
    let mut values = Vec::new();
    descendants(named(&b, "simpleType", "Color").unwrap(), &mut values);
    let values: Vec<&str> = values.iter().filter_map(|e| e.attr("value")).collect();
    assert_eq!(values, ["red", "RED", "green", "GREEN"]);
    assert!(named(&b, "element", "Named").is_none(), "abstract");

    // The subpackage has its own schema importing its parent's.
    let c = schema(&mm, "http://example.org/zoo/care");
    assert_eq!(base_of(complex_type(&c, "Vet")), Some("zoo:Keeper"));
    assert!(c.children_named("import").any(|i| i.attr("schemaLocation") == Some("zoo.xsd")));
    assert!(named(&c, "complexType", "Keeper").is_none(), "not repeated in the subpackage");
}

#[test]
fn ecore_package_schema_describes_ecore_files() {
    let (mm, _) = library();
    let s = schema(&mm, ECORE_NS_URI);
    assert_eq!(schema_file_name(&mm, mm.ecore_package()), "ecore.xsd");
    let eclass = complex_type(&s, "EClass");
    assert_eq!(base_of(eclass), Some("ecore:EClassifier"));
    assert_eq!(decl_type(eclass, "element", "eStructuralFeatures").as_deref(), Some("ecore:EStructuralFeature"));
    assert!(lookup(&s, "ecore:EAttribute", "attribute", "eType").is_some(), "volatile but serialized");
    // The library metamodel itself conforms structurally.
    let text = std::fs::read_to_string(format!("{EXAMPLES}/library.ecore")).unwrap();
    let root = xml::parse(&text, "library.ecore").unwrap();
    check_object(&s, &root, "ecore:EPackage");
}

#[test]
fn file_names_are_safe_and_unique() {
    let mut mm = Metamodel::new();
    let pkg = |name: &str, uri: &str| {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<ecore:EPackage xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:ecore="http://www.eclipse.org/emf/2002/Ecore"
    name="{name}" nsURI="{uri}" nsPrefix="p"/>"#
        )
    };
    mm.load_ecore_str(&pkg("shapes", "http://a/1"), "/v/a.ecore").unwrap();
    mm.load_ecore_str(&pkg("shapes", "http://a/2"), "/v/b.ecore").unwrap();
    mm.load_ecore_str(&pkg("XMI", "http://a/3"), "/v/c.ecore").unwrap();
    mm.load_ecore_str(&pkg("we ird/name", "http://a/4"), "/v/d.ecore").unwrap();
    let names: Vec<String> = mm.user_root_packages().map(|p| schema_file_name(&mm, p)).collect();
    assert_eq!(names, ["shapes.xsd", "shapes-2.xsd", "_XMI.xsd", "we_ird_name.xsd"]);
    for p in mm.user_root_packages() {
        // Empty packages still give well-formed schemas.
        xml::parse(&write_schema(&mm, p), "empty.xsd").unwrap();
    }
}

#[test]
fn packages_without_ns_uri_are_imported_without_namespace() {
    let mut mm = Metamodel::new();
    let plain = r#"<?xml version="1.0" encoding="UTF-8"?>
<ecore:EPackage xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    xmlns:ecore="http://www.eclipse.org/emf/2002/Ecore" name="plain">
  <eClassifiers xsi:type="ecore:EClass" name="Thing"/>
</ecore:EPackage>"#;
    let user = r#"<?xml version="1.0" encoding="UTF-8"?>
<ecore:EPackage xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    xmlns:ecore="http://www.eclipse.org/emf/2002/Ecore" name="user" nsURI="http://u" nsPrefix="u">
  <eClassifiers xsi:type="ecore:EClass" name="Special" eSuperTypes="plain.ecore#//Thing"/>
</ecore:EPackage>"#;
    mm.load_ecore_str(plain, "/v/plain.ecore").unwrap();
    let load = mm.load_ecore_str(user, "/v/user.ecore").unwrap();
    assert!(load.warnings.is_empty(), "{:?}", load.warnings);
    let s = schema(&mm, "http://u");
    let import = s.children_named("import").find(|i| i.attr("schemaLocation") == Some("plain.xsd")).unwrap();
    assert_eq!(import.attr("namespace"), None);
    assert_eq!(base_of(complex_type(&s, "Special")), Some("Thing"));
    let plain_pkg = mm.user_root_packages().find(|p| mm.package(*p).name == "plain").unwrap();
    let p = xml::parse(&write_schema(&mm, plain_pkg), "plain.xsd").unwrap();
    assert_eq!(p.attr("targetNamespace"), None);
    assert_eq!(named(&p, "element", "Thing").unwrap().attr("type"), Some("Thing"));
}
