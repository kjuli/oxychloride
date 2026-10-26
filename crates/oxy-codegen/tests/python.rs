//! The Python target: generated packages for the examples load their models and find the
//! same violations as the engine.

mod common;

use common::*;

fn python() -> Option<&'static str> {
    ["python3", "python"].into_iter().find(|p| available(p, "--version"))
}

#[test]
fn library_validation() {
    let Some(py) = python() else { return eprintln!("skipped: no Python") };
    let fixture = library();
    let (dir, output) = generate(&fixture, "python");
    assert!(output.problems.is_empty(), "{:?}", output.problems);
    std::fs::write(
        dir.join("check.py"),
        r#"
from library import load, validate, Book, Library, Member, Genre
res = load("model.xmi")
for d in validate(res):
    print(d)
lib = res.contents[0]
print(len(lib.books), lib.books[0].library is lib, lib.books[1].loan_count, lib.books[0].is_available())
print(sorted(m.name for m in lib.books[1].borrowers))
"#,
    )
    .unwrap();
    let out = run(&dir, py, &["check.py"]);
    let expected = "\
Book::SufficientCopies violated on Book 'A Brief History of Time'
Book::HasIsbn violated on Book 'The Guns of August': Book 'The Guns of August' has no ISBN
Member::Adult violated on Member 'Bob': Member Bob is 16 years old
3 True 2 True
['Alice', 'Bob']
";
    assert_eq!(out, expected);
}

#[test]
fn university_contracts_and_round_trip() {
    let Some(py) = python() else { return eprintln!("skipped: no Python") };
    let fixture = university();
    let (dir, output) = generate(&fixture, "python");
    assert!(output.problems.is_empty(), "{:?}", output.problems);
    std::fs::write(
        dir.join("check.py"),
        r#"
from university import (load, loads, dumps, validate, ContractViolation, set_contract_checking,
                        University, Course, Student, Enrolment, Kind)
res = load("model.xmi")
for d in validate(res):
    print(d)
uni = res.contents[0]
for c in uni.courses:
    try:
        c.has_seat()
    except ContractViolation as e:
        print(e)
carla = [s for s in uni.students if s.name == "Carla"][0]
print(carla.credits, [s.name for s in carla.enrolments[0].course.participants()])
try:
    carla.may_enrol(carla.enrolments[0].course)
except ContractViolation as e:
    print(e)
set_contract_checking(False)
print(uni.courses[0].has_seat())
set_contract_checking(True)

# Build a model in code: opposites and containment are kept consistent.
u = University(name="U")
c = Course(code="C1", title="T", kind=Kind.SEMINAR, credits=5, capacity=1)
s = Student(name="S", semester=1)
u.courses.append(c)
u.students.append(s)
e = Enrolment(grade=2)
s.enrolments.append(e)
e.course = c
print(e.student is s, c.enrolments[0] is e, s.credits, c.e_container() is u)
e.course = None
print(len(c.enrolments))
print([d.constraint for d in validate(u)])

# XMI round trip.
again = loads(dumps(res))
print(dumps(again) == dumps(res), len(again.all_contents()), len(validate(again)))
"#,
    )
    .unwrap();
    let out = run(&dir, py, &["check.py"]);
    let expected = "\
Student::PrerequisitesMet violated on Student 'Brian'
Student::CreditLimit violated on Student 'Carla': Student Carla takes 44 credits
Student::PrerequisitesMet violated on Student 'Carla'
postcondition FreeSeat of Course::hasSeat() is violated on Course //@courses.0
postcondition FreeSeat of Course::hasSeat() is violated on Course //@courses.1
44 ['Ada', 'Brian', 'Carla']
precondition NotEnrolled of Student::mayEnrol() is violated on Student 'Carla': already enrolled in IN0001
True
True True 5 True
0
['Student::CreditLimit', 'Enrolment::course']
True 16 3
";
    assert_eq!(out, expected);
}
