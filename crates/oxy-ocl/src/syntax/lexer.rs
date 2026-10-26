//! Tokenizer for OCL expressions and Complete OCL documents.

use std::fmt;

use crate::diagnostic::Diagnostic;
use crate::span::Span;

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    /// A name; `_'quoted names'` are unquoted. Document keywords such as `context` or `inv`
    /// are identifiers too, since they are only reserved in certain positions.
    Ident(String),
    /// Whether the identifier was written as `_'...'` (and so is never a keyword).
    QuotedIdent(String),
    Int(i64),
    Real(f64),
    Str(String),
    Keyword(Keyword),
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Semi,
    Colon,
    ColonColon,
    Dot,
    DotDot,
    Arrow,
    SafeDot,
    SafeArrow,
    Bar,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    Plus,
    Minus,
    Star,
    Slash,
    At,
    Caret,
    CaretCaret,
    Question,
    /// `#` (opposite references in OCLinEcore).
    Hash,
    /// `!` (negated qualifiers in OCLinEcore, e.g. `!ordered`).
    Bang,
    /// `:=` (assignments of Imperative OCL; QVT dialect only).
    ColonEq,
    /// `+=` (QVT dialect only).
    PlusEq,
    /// `++` (the rest of a QVT Relations collection template; QVT dialect only).
    PlusPlus,
    Eof,
}

/// The variant of the language to tokenize.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Dialect {
    /// OCL as in Complete OCL documents, OCLinEcore and Ecore annotations.
    #[default]
    Ocl,
    /// OCL as QVT (Operational Mappings and Relations) writes it: also `//` comments,
    /// double-quoted strings, `:=`, `+=`, `!=` and `++`.
    Qvt,
}

/// Words that are reserved inside expressions.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Keyword {
    And,
    Or,
    Xor,
    Not,
    Implies,
    If,
    Then,
    Else,
    Elseif,
    Endif,
    Let,
    In,
    True,
    False,
    Null,
    Invalid,
    SelfKw,
}

impl Keyword {
    fn from_word(word: &str) -> Option<Keyword> {
        Some(match word {
            "and" => Keyword::And,
            "or" => Keyword::Or,
            "xor" => Keyword::Xor,
            "not" => Keyword::Not,
            "implies" => Keyword::Implies,
            "if" => Keyword::If,
            "then" => Keyword::Then,
            "else" => Keyword::Else,
            "elseif" => Keyword::Elseif,
            "endif" => Keyword::Endif,
            "let" => Keyword::Let,
            "in" => Keyword::In,
            "true" => Keyword::True,
            "false" => Keyword::False,
            "null" => Keyword::Null,
            "invalid" => Keyword::Invalid,
            "self" => Keyword::SelfKw,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Keyword::And => "and",
            Keyword::Or => "or",
            Keyword::Xor => "xor",
            Keyword::Not => "not",
            Keyword::Implies => "implies",
            Keyword::If => "if",
            Keyword::Then => "then",
            Keyword::Else => "else",
            Keyword::Elseif => "elseif",
            Keyword::Endif => "endif",
            Keyword::Let => "let",
            Keyword::In => "in",
            Keyword::True => "true",
            Keyword::False => "false",
            Keyword::Null => "null",
            Keyword::Invalid => "invalid",
            Keyword::SelfKw => "self",
        }
    }
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            TokenKind::Ident(n) | TokenKind::QuotedIdent(n) => return write!(f, "'{n}'"),
            TokenKind::Int(i) => return write!(f, "{i}"),
            TokenKind::Real(r) => return write!(f, "{r}"),
            TokenKind::Str(_) => "string literal",
            TokenKind::Keyword(k) => return write!(f, "'{}'", k.as_str()),
            TokenKind::LParen => "'('",
            TokenKind::RParen => "')'",
            TokenKind::LBrace => "'{'",
            TokenKind::RBrace => "'}'",
            TokenKind::LBracket => "'['",
            TokenKind::RBracket => "']'",
            TokenKind::Comma => "','",
            TokenKind::Semi => "';'",
            TokenKind::Colon => "':'",
            TokenKind::ColonColon => "'::'",
            TokenKind::Dot => "'.'",
            TokenKind::DotDot => "'..'",
            TokenKind::Arrow => "'->'",
            TokenKind::SafeDot => "'?.'",
            TokenKind::SafeArrow => "'?->'",
            TokenKind::Bar => "'|'",
            TokenKind::Eq => "'='",
            TokenKind::Ne => "'<>'",
            TokenKind::Lt => "'<'",
            TokenKind::Gt => "'>'",
            TokenKind::Le => "'<='",
            TokenKind::Ge => "'>='",
            TokenKind::Plus => "'+'",
            TokenKind::Minus => "'-'",
            TokenKind::Star => "'*'",
            TokenKind::Slash => "'/'",
            TokenKind::At => "'@'",
            TokenKind::Caret => "'^'",
            TokenKind::CaretCaret => "'^^'",
            TokenKind::Question => "'?'",
            TokenKind::Hash => "'#'",
            TokenKind::Bang => "'!'",
            TokenKind::ColonEq => "':='",
            TokenKind::PlusEq => "'+='",
            TokenKind::PlusPlus => "'++'",
            TokenKind::Eof => "end of input",
        };
        f.write_str(s)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

/// Splits `src` into tokens. Lexical errors are reported and the offending characters skipped.
/// The returned vector always ends with an [`TokenKind::Eof`] token.
pub fn tokenize(src: &str) -> (Vec<Token>, Vec<Diagnostic>) {
    tokenize_dialect(src, Dialect::Ocl)
}

/// Splits `src` into tokens of the given dialect (see [`tokenize`]).
pub fn tokenize_dialect(src: &str, dialect: Dialect) -> (Vec<Token>, Vec<Diagnostic>) {
    let mut lexer = Lexer { src, bytes: src.as_bytes(), dialect, pos: 0, tokens: Vec::new(), diags: Vec::new() };
    lexer.run();
    (lexer.tokens, lexer.diags)
}

struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    dialect: Dialect,
    pos: usize,
    tokens: Vec<Token>,
    diags: Vec<Diagnostic>,
}

impl Lexer<'_> {
    fn peek(&self, ahead: usize) -> u8 {
        self.bytes.get(self.pos + ahead).copied().unwrap_or(0)
    }

    fn push(&mut self, kind: TokenKind, start: usize) {
        self.tokens.push(Token { kind, span: Span::new(start, self.pos) });
    }

    fn run(&mut self) {
        loop {
            self.skip_trivia();
            let start = self.pos;
            if self.pos >= self.bytes.len() {
                self.push(TokenKind::Eof, start);
                return;
            }
            let c = self.peek(0);
            if c.is_ascii_digit() {
                self.number();
                continue;
            }
            if c == b'_' && self.peek(1) == b'\'' {
                self.pos += 1;
                let name = self.string_body(start);
                self.push(TokenKind::QuotedIdent(name), start);
                continue;
            }
            if is_ident_start(self.src[self.pos..].chars().next().unwrap_or(' ')) {
                self.word();
                continue;
            }
            if c == b'\'' || (c == b'"' && self.dialect == Dialect::Qvt) {
                self.string();
                continue;
            }
            if self.dialect == Dialect::Qvt {
                let kind = match (c, self.peek(1)) {
                    (b':', b'=') => Some(TokenKind::ColonEq),
                    (b'+', b'=') => Some(TokenKind::PlusEq),
                    (b'+', b'+') => Some(TokenKind::PlusPlus),
                    (b'!', b'=') => Some(TokenKind::Ne),
                    _ => None,
                };
                if let Some(kind) = kind {
                    self.pos += 2;
                    self.push(kind, start);
                    continue;
                }
            }
            let (kind, len) = match (c, self.peek(1), self.peek(2)) {
                (b'?', b'-', b'>') => (TokenKind::SafeArrow, 3),
                (b'?', b'.', _) => (TokenKind::SafeDot, 2),
                (b'-', b'>', _) => (TokenKind::Arrow, 2),
                (b':', b':', _) => (TokenKind::ColonColon, 2),
                (b'.', b'.', _) => (TokenKind::DotDot, 2),
                (b'<', b'>', _) => (TokenKind::Ne, 2),
                (b'<', b'=', _) => (TokenKind::Le, 2),
                (b'>', b'=', _) => (TokenKind::Ge, 2),
                (b'^', b'^', _) => (TokenKind::CaretCaret, 2),
                (b'(', _, _) => (TokenKind::LParen, 1),
                (b')', _, _) => (TokenKind::RParen, 1),
                (b'{', _, _) => (TokenKind::LBrace, 1),
                (b'}', _, _) => (TokenKind::RBrace, 1),
                (b'[', _, _) => (TokenKind::LBracket, 1),
                (b']', _, _) => (TokenKind::RBracket, 1),
                (b',', _, _) => (TokenKind::Comma, 1),
                (b';', _, _) => (TokenKind::Semi, 1),
                (b':', _, _) => (TokenKind::Colon, 1),
                (b'.', _, _) => (TokenKind::Dot, 1),
                (b'|', _, _) => (TokenKind::Bar, 1),
                (b'=', _, _) => (TokenKind::Eq, 1),
                (b'<', _, _) => (TokenKind::Lt, 1),
                (b'>', _, _) => (TokenKind::Gt, 1),
                (b'+', _, _) => (TokenKind::Plus, 1),
                (b'-', _, _) => (TokenKind::Minus, 1),
                (b'*', _, _) => (TokenKind::Star, 1),
                (b'/', _, _) => (TokenKind::Slash, 1),
                (b'@', _, _) => (TokenKind::At, 1),
                (b'^', _, _) => (TokenKind::Caret, 1),
                (b'?', _, _) => (TokenKind::Question, 1),
                (b'#', _, _) => (TokenKind::Hash, 1),
                (b'!', _, _) => (TokenKind::Bang, 1),
                _ => {
                    let ch = self.src[self.pos..].chars().next().unwrap();
                    self.pos += ch.len_utf8();
                    self.diags
                        .push(Diagnostic::error(Span::new(start, self.pos), format!("unexpected character '{ch}'")));
                    continue;
                }
            };
            self.pos += len;
            self.push(kind, start);
        }
    }

    fn skip_trivia(&mut self) {
        loop {
            let c = self.peek(0);
            if c.is_ascii_whitespace() {
                self.pos += 1;
            } else if (c == b'-' && self.peek(1) == b'-')
                || (c == b'/' && self.peek(1) == b'/' && self.dialect == Dialect::Qvt)
            {
                while self.pos < self.bytes.len() && self.peek(0) != b'\n' {
                    self.pos += 1;
                }
            } else if c == b'/' && self.peek(1) == b'*' {
                let start = self.pos;
                self.pos += 2;
                loop {
                    if self.pos >= self.bytes.len() {
                        self.diags.push(Diagnostic::error(Span::new(start, start + 2), "unterminated comment"));
                        break;
                    }
                    if self.peek(0) == b'*' && self.peek(1) == b'/' {
                        self.pos += 2;
                        break;
                    }
                    self.pos += 1;
                }
            } else if c >= 0x80 && self.src[self.pos..].chars().next().is_some_and(char::is_whitespace) {
                self.pos += self.src[self.pos..].chars().next().unwrap().len_utf8();
            } else {
                return;
            }
        }
    }

    fn word(&mut self) {
        let start = self.pos;
        for (i, ch) in self.src[start..].char_indices() {
            if !is_ident_continue(ch) {
                self.pos = start + i;
                break;
            }
            self.pos = start + i + ch.len_utf8();
        }
        let word = &self.src[start..self.pos];
        let kind = match Keyword::from_word(word) {
            Some(k) => TokenKind::Keyword(k),
            None => TokenKind::Ident(word.to_string()),
        };
        self.push(kind, start);
    }

    fn number(&mut self) {
        let start = self.pos;
        while self.peek(0).is_ascii_digit() {
            self.pos += 1;
        }
        let mut is_real = false;
        if self.peek(0) == b'.' && self.peek(1).is_ascii_digit() {
            is_real = true;
            self.pos += 1;
            while self.peek(0).is_ascii_digit() {
                self.pos += 1;
            }
        }
        if matches!(self.peek(0), b'e' | b'E') {
            let sign = usize::from(matches!(self.peek(1), b'+' | b'-'));
            if self.peek(1 + sign).is_ascii_digit() {
                is_real = true;
                self.pos += 1 + sign;
                while self.peek(0).is_ascii_digit() {
                    self.pos += 1;
                }
            }
        }
        let text = &self.src[start..self.pos];
        let kind = if is_real {
            TokenKind::Real(text.parse().unwrap_or(f64::NAN))
        } else {
            match text.parse::<i64>() {
                Ok(i) => TokenKind::Int(i),
                Err(_) => {
                    self.diags.push(Diagnostic::error(
                        Span::new(start, self.pos),
                        "integer literal is too large (integers are 64-bit)",
                    ));
                    TokenKind::Int(i64::MAX)
                }
            }
        };
        self.push(kind, start);
    }

    fn string(&mut self) {
        let start = self.pos;
        let mut value = self.string_body(start);
        // Adjacent string literals are concatenated: 'abc' 'def'.
        loop {
            let save = self.pos;
            self.skip_trivia();
            if self.peek(0) == b'\'' || (self.peek(0) == b'"' && self.dialect == Dialect::Qvt) {
                value.push_str(&self.string_body(self.pos));
            } else {
                self.pos = save;
                break;
            }
        }
        self.push(TokenKind::Str(value), start);
    }

    /// Reads a `'...'` (or `"..."`) literal starting at the opening quote (at `self.pos`).
    fn string_body(&mut self, start: usize) -> String {
        let quote = self.peek(0) as char;
        self.pos += 1; // opening quote
        let mut out = String::new();
        loop {
            let Some(ch) = self.src[self.pos..].chars().next() else {
                self.diags.push(Diagnostic::error(Span::new(start, self.pos), "unterminated string literal"));
                return out;
            };
            self.pos += ch.len_utf8();
            match ch {
                c if c == quote => return out,
                '\\' => {
                    let esc_start = self.pos - 1;
                    let Some(e) = self.src[self.pos..].chars().next() else { continue };
                    self.pos += e.len_utf8();
                    match e {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'r' => out.push('\r'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        '\'' | '"' | '\\' => out.push(e),
                        'u' => {
                            let hex = self.src.get(self.pos..self.pos + 4).unwrap_or("");
                            match u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) {
                                Some(c) if hex.len() == 4 => {
                                    out.push(c);
                                    self.pos += 4;
                                }
                                _ => self.diags.push(Diagnostic::error(
                                    Span::new(esc_start, self.pos),
                                    "invalid unicode escape (expected \\uXXXX)",
                                )),
                            }
                        }
                        other => {
                            self.diags.push(Diagnostic::error(
                                Span::new(esc_start, self.pos),
                                format!("invalid escape sequence '\\{other}'"),
                            ));
                            out.push(other);
                        }
                    }
                }
                c => out.push(c),
            }
        }
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}

fn is_ident_continue(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        let (tokens, diags) = tokenize(src);
        assert!(diags.is_empty(), "{diags:?}");
        tokens.into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn lexes_operators_and_literals() {
        use TokenKind::*;
        assert_eq!(
            kinds("a->size() <= 3.5e1 -- comment\n and Set{1..2}?.x"),
            vec![
                Ident("a".into()),
                Arrow,
                Ident("size".into()),
                LParen,
                RParen,
                Le,
                Real(35.0),
                Keyword(super::Keyword::And),
                Ident("Set".into()),
                LBrace,
                Int(1),
                DotDot,
                Int(2),
                RBrace,
                SafeDot,
                Ident("x".into()),
                Eof
            ]
        );
    }

    #[test]
    fn lexes_strings() {
        assert_eq!(kinds(r"'a\'b' 'c'"), vec![TokenKind::Str("a'bc".into()), TokenKind::Eof]);
        assert_eq!(kinds("_'and'"), vec![TokenKind::QuotedIdent("and".into()), TokenKind::Eof]);
    }
}
