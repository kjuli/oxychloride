//! The Rust target: generated crates for the examples compile, load their models and find
//! the same violations as the engine.

mod common;

use std::path::Path;

use common::*;

/// Builds and runs a binary that uses the generated crate `krate` (in `dir`), with
/// `main.rs` as its source. Returns its output.
fn run_harness(dir: &Path, krate: &str, main: &str) -> String {
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"harness\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n{} = {{ path = \"{krate}\" }}\n\n[workspace]\n",
            krate.replace('_', "-")
        ),
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.rs"), main).unwrap();
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/codegen-tests/cargo-target");
    let target = target.to_string_lossy().into_owned();
    run(dir, "cargo", &["run", "--quiet", "--offline", "--target-dir", &target])
}

fn cargo() -> bool {
    available("cargo", "--version")
}

#[test]
fn library_validation() {
    if !cargo() {
        return eprintln!("skipped: no cargo");
    }
    let fixture = library();
    let (dir, output) = generate(&fixture, "rust");
    assert!(output.problems.is_empty(), "{:?}", output.problems);
    let out = run_harness(
        &dir,
        "library",
        r#"
use library::*;

fn main() {
    let (m, warnings) = Model::load_xmi("model.xmi").unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    for d in m.validate() {
        println!("{d}");
    }
    let lib: Library = m.cast(m.roots()[0]).unwrap();
    let books = lib.books(&m);
    println!(
        "{} {} {} {}",
        books.len(),
        books[0].library(&m) == Some(lib),
        books[1].loan_count(&m).unwrap(),
        books[0].is_available(&m).unwrap()
    );
    let mut names: Vec<String> =
        books[1].borrowers(&m).unwrap().iter().map(|b| b.name(&m).unwrap().to_string()).collect();
    names.sort();
    println!("{names:?}");
}
"#,
    );
    let expected = "\
Book::SufficientCopies violated on Book 'A Brief History of Time'
Book::HasIsbn violated on Book 'The Guns of August': Book 'The Guns of August' has no ISBN
Member::Adult violated on Member 'Bob': Member Bob is 16 years old
3 true 2 true
[\"Alice\", \"Bob\"]
";
    assert_eq!(out, expected);
}

#[test]
fn university_contracts_and_round_trip() {
    if !cargo() {
        return eprintln!("skipped: no cargo");
    }
    let fixture = university();
    let (dir, output) = generate(&fixture, "rust");
    assert!(output.problems.is_empty(), "{:?}", output.problems);
    let out = run_harness(
        &dir,
        "university",
        r#"
use university::*;

fn main() {
    let (m, warnings) = Model::load_xmi("model.xmi").unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    for d in m.validate() {
        println!("{d}");
    }
    let uni: University = m.cast(m.roots()[0]).unwrap();
    for c in uni.courses(&m) {
        if let Err(e) = c.has_seat(&m) {
            println!("{e}");
        }
    }
    let carla = *uni.students(&m).iter().find(|s| s.name(&m) == "Carla").unwrap();
    let course = carla.enrolments(&m)[0].course(&m).unwrap();
    let participants: Vec<&str> = course.participants(&m).unwrap().iter().map(|s| s.name(&m)).collect();
    println!("{} {participants:?}", carla.credits(&m).unwrap());
    if let Err(e) = carla.may_enrol(&m, course) {
        println!("{e}");
    }
    let mut unchecked = m.clone();
    unchecked.check_contracts = false;
    println!("{}", uni.courses(&unchecked)[0].has_seat(&unchecked).unwrap());

    // Build a model in code: opposites and containment are kept consistent.
    let mut m2 = Model::new();
    let u = m2.create_university();
    u.set_name(&mut m2, "U");
    let c = m2.create_course();
    c.set_code(&mut m2, "C1");
    c.set_kind(&mut m2, Kind::Seminar);
    c.set_credits(&mut m2, 5);
    c.set_capacity(&mut m2, 1);
    let s = m2.create_student();
    s.set_name(&mut m2, "S");
    u.add_to_courses(&mut m2, c);
    u.add_to_students(&mut m2, s);
    let e = m2.create_enrolment();
    e.set_grade(&mut m2, Some(2));
    s.add_to_enrolments(&mut m2, e);
    e.set_course(&mut m2, Some(c));
    println!(
        "{} {} {} {}",
        e.student(&m2) == Some(s),
        c.enrolments(&m2)[0] == e,
        s.credits(&m2).unwrap(),
        c.e_container(&m2) == Some(u.e_id())
    );
    e.set_course(&mut m2, None);
    println!("{}", c.enrolments(&m2).len());
    let constraints: Vec<String> = m2.validate().into_iter().map(|d| d.constraint).collect();
    println!("{constraints:?}");

    // XMI round trip.
    let (again, warnings) = Model::from_xmi(&m.to_xmi()).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    println!("{} {} {}", again.to_xmi() == m.to_xmi(), again.objects().len(), again.validate().len());
}
"#,
    );
    let expected = "\
Student::PrerequisitesMet violated on Student 'Brian'
Student::CreditLimit violated on Student 'Carla': Student Carla takes 44 credits
Student::PrerequisitesMet violated on Student 'Carla'
postcondition FreeSeat of Course::hasSeat() is violated on Course //@courses.0
postcondition FreeSeat of Course::hasSeat() is violated on Course //@courses.1
44 [\"Ada\", \"Brian\", \"Carla\"]
precondition NotEnrolled of Student::mayEnrol() is violated on Student 'Carla': already enrolled in IN0001
true
true true 5 true
0
[\"Enrolment::course\"]
true 16 3
";
    // Unlike Python (and OCL), CreditLimit holds after the enrolment lost its course: Rust
    // collections cannot contain null, so `enrolments.course` leaves the null out instead of
    // making `.credits` invalid (see docs/CODEGEN.md).
    assert_eq!(out, expected);
}
