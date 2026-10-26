//! Code generation from Ecore metamodels and their OCL: native domain models with
//! validation, in target languages such as Python and Rust.
//!
//! Like EMF's generator, a target turns the classes of a metamodel into classes (or structs)
//! with typed features, containment and bidirectional references kept consistent, and
//! loading and saving of XMI documents. Unlike EMF's, the OCL is translated too: invariants
//! become validation methods, pre- and postconditions are checked when operations are
//! called, operation bodies, derived features and `def:`s become ordinary code. The
//! generated code does not use this engine: each target copies a small runtime (the OCL
//! library, collections, XMI) into the output.
//!
//! # Pipeline
//!
//! 1. [`build()`] turns a [`Metamodel`] and a compiled OCL [`Environment`] into a
//!    [`CodeModel`]: the selected packages and those they depend on, with every OCL
//!    expression lowered to a resolved, typed [`model::Expr`] (see [`lower`]).
//! 2. A [`Target`] turns the code model into [`GeneratedFile`]s.
//!
//! [`generate`] does both. Problems (OCL that cannot be translated, constructs a target does
//! not support) are reported as [`Problem`]s; the code is generated anyway, and the affected
//! expressions fail at run time with an explanation.
//!
//! # Adding a target language
//!
//! See `docs/CODEGEN.md` ("Adding a target language"): implement [`Target`] in a module
//! next to [`python`] and [`rust`], register it in [`TARGETS`], and add it to the
//! conformance tests (`tests/conformance.rs`).

pub mod build;
pub mod library;
mod lower;
pub mod model;
pub mod naming;
pub mod python;
pub mod rust;
pub mod writer;

use oxy_ecore::{Metamodel, PackageId};
use oxy_ocl::Environment;

pub use build::build;
pub use model::CodeModel;

/// A code generator for one target language.
///
/// Targets are stateless; the options of one generation are passed to [`Target::generate`].
pub trait Target: Sync {
    /// The identifier used in options, the protocol and the command line (`python`).
    fn id(&self) -> &'static str;
    /// The name shown to users (`Python`).
    fn name(&self) -> &'static str;
    /// One sentence about what is generated.
    fn description(&self) -> &'static str;
    /// Generates the files for a code model. Paths are relative to the output directory
    /// chosen by the user and use `/`.
    fn generate(&self, model: &CodeModel, options: &Options, output: &mut Output);
}

/// The registered targets.
pub static TARGETS: &[&dyn Target] = &[&python::Python, &rust::Rust];

/// The target with the given identifier.
pub fn target(id: &str) -> Option<&'static dyn Target> {
    TARGETS.iter().copied().find(|t| t.id() == id)
}

/// Options of a generation.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// The name of the generated module, package or crate (default: the first package's
    /// name, in lower case).
    pub name: Option<String>,
}

/// A generated file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneratedFile {
    /// The path relative to the output directory, with `/` separators.
    pub path: String,
    pub text: String,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

/// Something the generated code does not (fully) support.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub severity: Severity,
    pub message: String,
    /// Where the problem comes from (e.g. `library.ocl:16`), if known.
    pub origin: Option<String>,
}

impl Problem {
    pub fn warning(message: impl Into<String>) -> Problem {
        Problem { severity: Severity::Warning, message: message.into(), origin: None }
    }
}

/// The result of a generation.
#[derive(Clone, Debug, Default)]
pub struct Output {
    pub files: Vec<GeneratedFile>,
    pub problems: Vec<Problem>,
}

impl Output {
    pub fn file(&mut self, path: impl Into<String>, text: impl Into<String>) {
        self.files.push(GeneratedFile { path: path.into(), text: text.into() });
    }

    pub fn warn(&mut self, message: impl Into<String>, origin: Option<&str>) {
        let problem =
            Problem { severity: Severity::Warning, message: message.into(), origin: origin.map(str::to_string) };
        if !self.problems.contains(&problem) {
            self.problems.push(problem);
        }
    }
}

/// Generates code for `packages` (and the packages they depend on) with the target `target`.
pub fn generate(
    mm: &Metamodel,
    env: &Environment,
    packages: &[PackageId],
    target: &str,
    options: &Options,
) -> Result<Output, String> {
    let t = self::target(target).ok_or_else(|| {
        let known: Vec<&str> = TARGETS.iter().map(|t| t.id()).collect();
        format!("unknown target '{target}' (known: {})", known.join(", "))
    })?;
    if packages.is_empty() {
        return Err("no packages to generate".into());
    }
    let (model, problems) = build(mm, env, packages, options.name.as_deref());
    let mut output = Output { files: Vec::new(), problems };
    t.generate(&model, options, &mut output);
    Ok(output)
}
