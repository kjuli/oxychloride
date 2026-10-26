//! The Oxychloride OCL engine: parsing, type checking and evaluation of OCL over Ecore models.

pub mod diagnostic;
pub mod span;
pub mod syntax;

pub use diagnostic::{Diagnostic, Severity};
pub use span::{LineIndex, Position, Span};

mod check;
pub mod env;
pub mod eval;
pub mod oclinecore;
pub mod resolution;
pub mod types;
pub mod validate;
pub mod value;

pub use env::{Environment, EnvironmentBuilder, Invariant, Source, SourceId, SourceKind, SourcedDiagnostic};
pub use eval::{ContractCheck, Evaluator};
pub use resolution::{Analysis, Resolution};
pub use types::Type;
pub use validate::{ConstraintResult, Status, UncheckedOperation, Validation, validate, validate_model};
pub use value::Value;
