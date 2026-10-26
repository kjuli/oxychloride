//! A text buffer for generated code, with indentation.

use std::fmt::Write as _;

#[derive(Default)]
pub struct Writer {
    out: String,
    indent: usize,
    /// The indentation unit (four spaces by default).
    unit: &'static str,
}

impl Writer {
    pub fn new() -> Writer {
        Writer { out: String::new(), indent: 0, unit: "    " }
    }

    /// Writes one line at the current indentation (empty lines without indentation).
    pub fn line(&mut self, text: impl AsRef<str>) {
        let text = text.as_ref();
        if !text.is_empty() {
            for _ in 0..self.indent {
                self.out.push_str(self.unit);
            }
            self.out.push_str(text);
        }
        self.out.push('\n');
    }

    /// Writes several lines (separated by `\n`) at the current indentation.
    pub fn lines(&mut self, text: impl AsRef<str>) {
        for l in text.as_ref().lines() {
            self.line(l);
        }
    }

    /// Ends the text with (at least) one empty line, unless it is empty.
    pub fn blank(&mut self) {
        self.blank_lines(1);
    }

    /// Ends the text with (at least) `n` empty lines, unless it is empty.
    pub fn blank_lines(&mut self, n: usize) {
        if self.out.is_empty() {
            return;
        }
        let have = self.out.len() - self.out.trim_end_matches('\n').len();
        for _ in have..n + 1 {
            self.out.push('\n');
        }
    }

    pub fn indent(&mut self) {
        self.indent += 1;
    }

    pub fn dedent(&mut self) {
        self.indent = self.indent.saturating_sub(1);
    }

    /// Writes `head`, then the lines of `body` indented one level.
    pub fn block(&mut self, head: impl AsRef<str>, body: impl FnOnce(&mut Writer)) {
        self.line(head);
        self.indent();
        body(self);
        self.dedent();
    }

    /// Writes a line built with `format_args!`.
    pub fn fmt(&mut self, args: std::fmt::Arguments<'_>) {
        let mut s = String::new();
        let _ = s.write_fmt(args);
        self.line(s);
    }

    pub fn finish(self) -> String {
        self.out
    }
}

/// Comment lines for a (possibly multi-line) text, each prefixed with `prefix`.
pub fn comment_lines(prefix: &str, text: &str) -> Vec<String> {
    text.lines()
        .map(|l| if l.trim().is_empty() { prefix.trim_end().to_string() } else { format!("{prefix}{}", l.trim_end()) })
        .collect()
}
