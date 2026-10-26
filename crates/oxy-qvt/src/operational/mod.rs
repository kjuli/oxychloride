//! QVT Operational Mappings (`.qvto`): parsing ([`syntax`]), compiling (model types, signatures,
//! type checking of all bodies) and running.

mod compile;
mod run;
pub mod syntax;

pub use compile::Transformation;
pub(crate) use compile::compile;
pub(crate) use run::run;
