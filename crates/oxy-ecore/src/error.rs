use std::fmt;

/// An error raised while loading an `.ecore` metamodel or an XMI model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadError {
    pub path: String,
    /// 1-based line and column, when the error can be attributed to a location.
    pub location: Option<(usize, usize)>,
    pub message: String,
}

impl LoadError {
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        LoadError { path: path.into(), location: None, message: message.into() }
    }

    pub fn at(path: impl Into<String>, text: &str, offset: usize, message: impl Into<String>) -> Self {
        LoadError { path: path.into(), location: Some(crate::xml::line_col(text, offset)), message: message.into() }
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.location {
            Some((line, col)) => write!(f, "{}:{}:{}: {}", self.path, line, col, self.message),
            None => write!(f, "{}: {}", self.path, self.message),
        }
    }
}

impl std::error::Error for LoadError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProblemSeverity {
    Error,
    Warning,
}

/// A problem located by a byte range in a source text (used by the textual model syntax,
/// where editors need precise ranges rather than line/column pairs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub severity: ProblemSeverity,
    pub message: String,
    pub span: std::ops::Range<usize>,
}

impl Problem {
    pub fn error(span: std::ops::Range<usize>, message: impl Into<String>) -> Self {
        Problem { severity: ProblemSeverity::Error, message: message.into(), span }
    }

    pub fn warning(span: std::ops::Range<usize>, message: impl Into<String>) -> Self {
        Problem { severity: ProblemSeverity::Warning, message: message.into(), span }
    }

    pub fn is_error(&self) -> bool {
        self.severity == ProblemSeverity::Error
    }
}
