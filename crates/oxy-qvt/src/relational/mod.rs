//! QVT Relations (`.qvtr`): parsing ([`syntax`]), compiling (imports, templates resolved
//! against the metamodel, type checking) and running in the direction of a parameter.

mod compile;
mod run;
pub mod syntax;

pub use compile::Transformation;
pub(crate) use compile::compile;
pub(crate) use run::run;
