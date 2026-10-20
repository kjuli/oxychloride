use std::fmt;

use crate::span::Span;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        })
    }
}

/// A problem found while parsing or checking OCL, located in some source text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub span: Span,
}

impl Diagnostic {
    pub fn error(span: Span, message: impl Into<String>) -> Self {
        Diagnostic { severity: Severity::Error, message: message.into(), span }
    }

    pub fn warning(span: Span, message: impl Into<String>) -> Self {
        Diagnostic { severity: Severity::Warning, message: message.into(), span }
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}
