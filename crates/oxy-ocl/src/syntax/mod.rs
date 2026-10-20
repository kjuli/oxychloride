//! Lexing and parsing.

pub mod ast;
pub mod lexer;
mod parser;

pub use parser::{
    FEATURE_QUALIFIERS, GENMODEL_SOURCE, OPERATION_QUALIFIERS, PARAMETER_QUALIFIERS, parse_document,
    parse_ecore_document, parse_expression,
};
