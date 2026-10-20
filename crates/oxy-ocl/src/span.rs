/// A byte range in a source text.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Span { start: start as u32, end: end as u32 }
    }

    pub fn to(self, other: Span) -> Span {
        Span { start: self.start.min(other.start), end: self.end.max(other.end) }
    }

    pub fn range(self) -> std::ops::Range<usize> {
        self.start as usize..self.end as usize
    }
}

/// A 0-based line and a 0-based column counted in characters (Unicode scalar values).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

/// Maps byte offsets to line/column positions.
#[derive(Debug, Clone)]
pub struct LineIndex {
    line_starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        LineIndex { line_starts }
    }

    pub fn position(&self, text: &str, offset: usize) -> Position {
        let offset = offset.min(text.len());
        let line = self.line_starts.partition_point(|s| *s <= offset) - 1;
        let start = self.line_starts[line];
        let column = text.get(start..offset).map_or(offset - start, |s| s.chars().count());
        Position { line: line as u32, column: column as u32 }
    }

    /// Converts a position back to a byte offset (clamped to the text).
    pub fn offset(&self, text: &str, pos: Position) -> usize {
        let Some(start) = self.line_starts.get(pos.line as usize).copied() else { return text.len() };
        let line = &text[start..];
        let line = line.split('\n').next().unwrap_or("");
        start + line.char_indices().nth(pos.column as usize).map_or(line.len(), |(i, _)| i)
    }
}
