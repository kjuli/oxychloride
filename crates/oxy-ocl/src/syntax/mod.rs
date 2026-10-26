//! Lexing and parsing.

pub mod ast;
pub mod lexer;
mod parser;

pub use lexer::Dialect;
pub use parser::{
    FEATURE_QUALIFIERS, GENMODEL_SOURCE, Mark, OPERATION_QUALIFIERS, PARAMETER_QUALIFIERS, Parser, QVT_ITERATORS,
    RESOLVE_OPERATIONS, parse_document, parse_ecore_document, parse_expression,
};
