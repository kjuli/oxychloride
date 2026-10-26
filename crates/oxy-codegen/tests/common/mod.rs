//! Fixtures for the code generation tests: the example metamodels with their OCL, and
//! helpers that generate code into `target/codegen-tests` and run it.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use oxy_codegen::{Options, Output};
use oxy_ecore::{Metamodel, Model, PackageId};
use oxy_ocl::{Environment, EnvironmentBuilder};

pub struct Fixture {
    pub name: &'static str,
    pub mm: Metamodel,
    pub env: Environment,
    pub packages: Vec<PackageId>,
    /// An XMI model of the metamodel.
    pub xmi: String,
    /// The same model, loaded.
    pub model: Model,
}

pub fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// `library.ecore` with `library.ocl`, and `library.xmi`.
pub fn library() -> Fixture {
    let dir = examples().join("library");
    let mut mm = Metamodel::new();
    let load = mm.load_ecore_file(dir.join("library.ecore")).unwrap();
    let packages = mm.resources()[load.resource].roots.clone();
    let mut b = EnvironmentBuilder::new(&mm);
    b.add_ecore_constraints();
    b.add_document("library.ocl", &read(&dir.join("library.ocl")));
    let env = b.build();
    assert!(!env.has_errors(), "{:?}", env.diagnostics());
    let xmi = read(&dir.join("library.xmi"));
    let mut model = Model::new();
    oxy_ecore::XmiDocument::parse(&xmi, "library.xmi").unwrap().load(&mm, &mut model).unwrap();
    Fixture { name: "library", mm, env, packages, xmi, model }
}

/// An OCLinEcore metamodel (with an optional Complete OCL document) and a HUTN model of it.
pub fn textual(name: &'static str, oclinecore: &str, ocl: Option<&str>, hutn: &str) -> Fixture {
    let mut mm = Metamodel::new();
    let built = oxy_ocl::oclinecore::load(&mut mm, &format!("{name}.oclinecore"), oclinecore);
    let packages = built.packages.clone();
    let mut b = EnvironmentBuilder::new(&mm);
    b.add_oclinecore(&format!("{name}.oclinecore"), oclinecore, &built);
    if let Some(ocl) = ocl {
        b.add_document(&format!("{name}.ocl"), ocl);
    }
    let env = b.build();
    let errors: Vec<_> = env.diagnostics().iter().filter(|d| d.diagnostic.is_error()).collect();
    assert!(errors.is_empty(), "{errors:?}");
    let mut model = Model::new();
    let load = oxy_ecore::hutn::load(hutn, &format!("{name}.hutn"), &mm, &mut model);
    assert!(load.problems.iter().all(|p| p.severity != oxy_ecore::ProblemSeverity::Error), "{:?}", load.problems);
    let xmi = oxy_ecore::xmi_writer::write_resource(&mm, &model, load.resource, &[]);
    Fixture { name, mm, env, packages, xmi, model }
}

/// `university.oclinecore` and `university.hutn`.
pub fn university() -> Fixture {
    let dir = examples().join("university");
    textual("university", &read(&dir.join("university.oclinecore")), None, &read(&dir.join("university.hutn")))
}

/// Generates code for a fixture into `target/codegen-tests/<fixture>-<target>` (emptied
/// first) and writes the model there as `model.xmi`.
pub fn generate(fixture: &Fixture, target: &str) -> (PathBuf, Output) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/codegen-tests")
        .join(format!("{}-{target}", fixture.name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let output = oxy_codegen::generate(&fixture.mm, &fixture.env, &fixture.packages, target, &Options::default())
        .expect("generation succeeds");
    for f in &output.files {
        let path = dir.join(&f.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &f.text).unwrap();
    }
    std::fs::write(dir.join("model.xmi"), &fixture.xmi).unwrap();
    (dir, output)
}

/// Whether a program is available (tests that need it are skipped otherwise).
pub fn available(program: &str, arg: &str) -> bool {
    Command::new(program).arg(arg).output().is_ok_and(|o| o.status.success())
}

/// Runs a command in `dir`; panics with its output if it fails.
pub fn run(dir: &Path, program: &str, args: &[&str]) -> String {
    let out = Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("cannot run {program}: {e}"));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "{program} {args:?} failed in {}:\n{stdout}\n{stderr}", dir.display());
    stdout
}
