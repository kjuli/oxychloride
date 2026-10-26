//! Henshin's attribute expressions as JavaScript.
//!
//! In `.henshin` files, attribute values, attribute conditions and iteration counts are
//! JavaScript expressions (Henshin evaluates them with a script engine). The textual syntax has
//! a small expression language instead; this module translates between the two:
//!
//! * [`parse`] reads the part of JavaScript the textual syntax can express — literals,
//!   parameters, dotted names and calls (Java attributes and calls), `!`, unary `-`,
//!   `* / + -`, comparisons, `==`/`!=` (also `===`/`!==`), `&&` and `||` — into a
//!   [`syntax::Expr`](crate::syntax::Expr);
//! * [`print`] and [`print_compiled`] write expressions as JavaScript, the way Henshin's own
//!   translation of the textual syntax does (`x>amount`, `a && b`).

use std::ops::Range;

use oxy_ecore::Metamodel;

use crate::module::Param;
use crate::syntax::{BinOp, Expr, ExprKind};
use crate::value::{CExpr, Value};

/// The function that holds JavaScript the textual syntax cannot express:
/// `javascript("a ? b : c")` in a text is `a ? b : c` in a `.henshin` file.
pub const SCRIPT: &str = "javascript";

/// The script of a `javascript("…")` expression.
pub fn script(e: &Expr) -> Option<&str> {
    match &e.kind {
        ExprKind::Call(f, args) if f == SCRIPT && args.len() == 1 => match &args[0].kind {
            ExprKind::Str(s) => Some(s),
            _ => None,
        },
        _ => None,
    }
}

// ---------------------------------------------------------------------- printing

/// The precedence of an operator in JavaScript (and in the textual syntax, which agrees).
fn precedence(op: BinOp) -> u8 {
    match op {
        BinOp::Or => 1,
        BinOp::And => 2,
        BinOp::Eq | BinOp::Ne => 3,
        BinOp::Ge | BinOp::Le | BinOp::Gt | BinOp::Lt => 4,
        BinOp::Add | BinOp::Sub => 5,
        BinOp::Mul | BinOp::Div => 6,
    }
}

/// The precedence of unary operators and operands.
const UNARY: u8 = 7;

fn js_operator(op: BinOp) -> &'static str {
    match op {
        BinOp::Or => " || ",
        BinOp::And => " && ",
        other => other.symbol(),
    }
}

/// A JavaScript string literal.
pub fn string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn real_literal(r: f64) -> String {
    crate::value::format_real(r)
}

/// Joins a binary expression, parenthesizing operands that bind less tightly (and right
/// operands that bind equally: the operators are left-associative).
fn binary(op: BinOp, (l, lp): (String, u8), (r, rp): (String, u8)) -> (String, u8) {
    let p = precedence(op);
    let l = if lp < p { format!("({l})") } else { l };
    let r = if rp <= p { format!("({r})") } else { r };
    (format!("{l}{}{r}", js_operator(op)), p)
}

fn unary(prefix: &str, (e, p): (String, u8)) -> (String, u8) {
    (if p < UNARY { format!("{prefix}({e})") } else { format!("{prefix}{e}") }, UNARY)
}

/// An expression of the textual syntax as JavaScript (empty for a syntax error).
pub fn print(e: &Expr) -> String {
    fn go(e: &Expr) -> (String, u8) {
        if let Some(s) = script(e) {
            return (s.to_string(), 0);
        }
        match &e.kind {
            ExprKind::Name(n) => (n.clone(), UNARY),
            ExprKind::Qualified(parts) => (parts.join("."), UNARY),
            ExprKind::Call(name, args) => {
                let args: Vec<String> = args.iter().map(|a| go(a).0).collect();
                (format!("{name}({})", args.join(",")), UNARY)
            }
            ExprKind::Str(s) => (string_literal(s), UNARY),
            ExprKind::Int(i) => (i.to_string(), if *i < 0 { UNARY - 1 } else { UNARY }),
            ExprKind::Real(r) => (real_literal(*r), if *r < 0.0 { UNARY - 1 } else { UNARY }),
            ExprKind::Bool(b) => (b.to_string(), UNARY),
            ExprKind::Not(inner) => unary("!", go(inner)),
            ExprKind::Neg(inner) => unary("-", go(inner)),
            ExprKind::Binary(op, l, r) => binary(*op, go(l), go(r)),
            ExprKind::Error => (String::new(), UNARY),
        }
    }
    go(e).0
}

/// A compiled expression as JavaScript: parameters by name, enumeration literals as their
/// names in a string (Henshin converts strings to literals of enumeration-typed attributes).
pub fn print_compiled(mm: &Metamodel, e: &CExpr, params: &[Param]) -> String {
    fn go(mm: &Metamodel, e: &CExpr, params: &[Param]) -> (String, u8) {
        match e {
            CExpr::Param(i) => (params.get(*i).map_or_else(|| "?".to_string(), |p| p.name.clone()), UNARY),
            CExpr::Literal(v) => match v {
                Value::Null => ("null".into(), UNARY),
                Value::Bool(b) => (b.to_string(), UNARY),
                Value::Int(i) => (i.to_string(), if *i < 0 { UNARY - 1 } else { UNARY }),
                Value::Real(r) => (real_literal(*r), if *r < 0.0 { UNARY - 1 } else { UNARY }),
                Value::Str(s) => (string_literal(s), UNARY),
                Value::Enum(c, l) => {
                    let name = mm.enum_literals(*c).get(*l).map_or("", |l| l.name.as_str());
                    (string_literal(name), UNARY)
                }
                Value::Node(n) => (format!("#{}", n.0), UNARY),
            },
            CExpr::Not(inner) => unary("!", go(mm, inner, params)),
            CExpr::Neg(inner) => unary("-", go(mm, inner, params)),
            CExpr::Binary(op, l, r) => binary(*op, go(mm, l, params), go(mm, r, params)),
            CExpr::Unsupported { source, .. } => (source.clone(), if source.is_empty() { UNARY } else { 0 }),
        }
    }
    go(mm, e, params).0
}

// ---------------------------------------------------------------------- parsing

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Int(i64),
    Real(f64),
    Str(String),
    Punct(&'static str),
    Eof,
}

/// Operators and punctuation, longest first.
const PUNCTUATION: &[&str] = &[
    "===", "!==", "==", "!=", ">=", "<=", "&&", "||", "(", ")", ",", ".", "!", "-", "+", "*", "/", "%", "<", ">", "?",
    ":", "[", "]",
];

/// A token or a problem, with its span.
type Spanned<T> = (T, Range<usize>);

fn tokenize(src: &str) -> Result<Vec<Spanned<Tok>>, Spanned<String>> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < src.len() {
        let c = src[pos..].chars().next().unwrap_or(' ');
        if c.is_whitespace() {
            pos += c.len_utf8();
            continue;
        }
        let start = pos;
        if c.is_alphabetic() || c == '_' || c == '$' {
            let len = src[pos..]
                .find(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '$'))
                .unwrap_or(src.len() - pos);
            pos += len;
            out.push((Tok::Ident(src[start..pos].to_string()), start..pos));
        } else if c.is_ascii_digit() {
            while pos < src.len() && bytes[pos].is_ascii_digit() {
                pos += 1;
            }
            let mut real = false;
            if pos + 1 < src.len() && bytes[pos] == b'.' && bytes[pos + 1].is_ascii_digit() {
                real = true;
                pos += 1;
                while pos < src.len() && bytes[pos].is_ascii_digit() {
                    pos += 1;
                }
            }
            if pos < src.len() && (bytes[pos] == b'e' || bytes[pos] == b'E') {
                let mut e = pos + 1;
                if e < src.len() && (bytes[e] == b'+' || bytes[e] == b'-') {
                    e += 1;
                }
                if e < src.len() && bytes[e].is_ascii_digit() {
                    real = true;
                    pos = e;
                    while pos < src.len() && bytes[pos].is_ascii_digit() {
                        pos += 1;
                    }
                }
            }
            let text = &src[start..pos];
            let tok = if real {
                Tok::Real(text.parse().map_err(|_| (format!("'{text}' is not a number"), start..pos))?)
            } else {
                match text.parse() {
                    Ok(i) => Tok::Int(i),
                    Err(_) => Tok::Real(text.parse().map_err(|_| (format!("'{text}' is not a number"), start..pos))?),
                }
            };
            out.push((tok, start..pos));
        } else if c == '"' || c == '\'' {
            let mut value = String::new();
            let mut chars = src[pos + 1..].char_indices();
            let mut end = None;
            while let Some((i, ch)) = chars.next() {
                match ch {
                    ch if ch == c => {
                        end = Some(pos + 1 + i + 1);
                        break;
                    }
                    '\\' => match chars.next() {
                        Some((_, 'n')) => value.push('\n'),
                        Some((_, 't')) => value.push('\t'),
                        Some((_, 'r')) => value.push('\r'),
                        Some((_, e)) => value.push(e),
                        None => break,
                    },
                    ch => value.push(ch),
                }
            }
            let Some(end) = end else { return Err(("unterminated string".into(), start..src.len())) };
            pos = end;
            out.push((Tok::Str(value), start..pos));
        } else if let Some(p) = PUNCTUATION.iter().find(|p| src[pos..].starts_with(**p)) {
            pos += p.len();
            out.push((Tok::Punct(p), start..pos));
        } else {
            return Err((format!("unexpected character '{c}'"), start..start + c.len_utf8()));
        }
    }
    out.push((Tok::Eof, src.len()..src.len()));
    Ok(out)
}

struct Parser<'a> {
    tokens: Vec<(Tok, Range<usize>)>,
    pos: usize,
    /// Maps offsets in the expression to spans in the document.
    map: &'a dyn Fn(Range<usize>) -> Range<usize>,
}

type Parsed<T> = Result<T, (String, Range<usize>)>;

impl Parser<'_> {
    fn peek(&self) -> &Tok {
        &self.tokens[self.pos].0
    }

    fn at(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::Punct(q) if *q == p)
    }

    fn bump(&mut self) -> (Tok, Range<usize>) {
        let t = self.tokens[self.pos].clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn start(&self) -> usize {
        self.tokens[self.pos].1.start
    }

    fn end(&self) -> usize {
        if self.pos == 0 { 0 } else { self.tokens[self.pos - 1].1.end }
    }

    fn expr(&mut self, start: usize, kind: ExprKind) -> Expr {
        Expr { kind, span: (self.map)(start..self.end()) }
    }

    fn unsupported<T>(&self, what: &str) -> Parsed<T> {
        let span = self.tokens[self.pos].1.clone();
        Err((format!("{what} cannot be written in Henshin's textual syntax"), span))
    }

    fn binary(&mut self, level: usize) -> Parsed<Expr> {
        const LEVELS: &[&[(&str, BinOp)]] = &[
            &[("||", BinOp::Or)],
            &[("&&", BinOp::And)],
            &[("===", BinOp::Eq), ("!==", BinOp::Ne), ("==", BinOp::Eq), ("!=", BinOp::Ne)],
            &[(">=", BinOp::Ge), ("<=", BinOp::Le), (">", BinOp::Gt), ("<", BinOp::Lt)],
            &[("+", BinOp::Add), ("-", BinOp::Sub)],
            &[("*", BinOp::Mul), ("/", BinOp::Div)],
        ];
        if level == LEVELS.len() {
            return self.unary();
        }
        let start = self.start();
        let mut left = self.binary(level + 1)?;
        'outer: loop {
            for (p, op) in LEVELS[level] {
                if self.at(p) {
                    self.bump();
                    let right = self.binary(level + 1)?;
                    left = self.expr(start, ExprKind::Binary(*op, Box::new(left), Box::new(right)));
                    continue 'outer;
                }
            }
            if self.at("%") {
                return self.unsupported("'%'");
            }
            return Ok(left);
        }
    }

    fn unary(&mut self) -> Parsed<Expr> {
        let start = self.start();
        if self.at("!") {
            self.bump();
            let inner = self.unary()?;
            return Ok(self.expr(start, ExprKind::Not(Box::new(inner))));
        }
        if self.at("-") {
            self.bump();
            let inner = self.unary()?;
            // Negative literals are literals, as in the textual syntax's compiler.
            let kind = match inner.kind {
                ExprKind::Int(i) => ExprKind::Int(-i),
                ExprKind::Real(r) => ExprKind::Real(-r),
                other => ExprKind::Neg(Box::new(Expr { kind: other, span: inner.span })),
            };
            return Ok(self.expr(start, kind));
        }
        if self.at("+") {
            return self.unsupported("unary '+'");
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Parsed<Expr> {
        let start = self.start();
        let (tok, _) = self.bump();
        let kind = match tok {
            Tok::Int(i) => ExprKind::Int(i),
            Tok::Real(r) => ExprKind::Real(r),
            Tok::Str(s) => ExprKind::Str(s),
            Tok::Ident(w) if w == "true" || w == "false" => ExprKind::Bool(w == "true"),
            Tok::Ident(w) if w == "null" || w == "undefined" || w == "new" || w == "function" => {
                self.pos -= 1;
                return self.unsupported(&format!("'{w}'"));
            }
            Tok::Ident(first) => {
                let mut parts = vec![first];
                while self.at(".") {
                    self.bump();
                    match self.bump() {
                        (Tok::Ident(w), _) => parts.push(w),
                        _ => {
                            self.pos -= 1;
                            return self.unsupported("this member access");
                        }
                    }
                }
                if self.at("(") {
                    self.bump();
                    let mut args = Vec::new();
                    if !self.at(")") {
                        loop {
                            args.push(self.binary(0)?);
                            if !self.at(",") {
                                break;
                            }
                            self.bump();
                        }
                    }
                    if !self.at(")") {
                        return self.unsupported("this call");
                    }
                    self.bump();
                    ExprKind::Call(parts.join("."), args)
                } else if parts.len() == 1 {
                    ExprKind::Name(parts.pop().unwrap_or_default())
                } else {
                    ExprKind::Qualified(parts)
                }
            }
            Tok::Punct("(") => {
                let inner = self.binary(0)?;
                if !self.at(")") {
                    return self.unsupported("this expression");
                }
                self.bump();
                inner.kind
            }
            Tok::Eof => {
                return Err(("expected an expression".into(), (self.map)(self.start()..self.start())));
            }
            Tok::Punct(p) => {
                self.pos -= 1;
                return self.unsupported(&format!("'{p}'"));
            }
        };
        if self.at(".") || self.at("(") || self.at("[") {
            return self.unsupported("a member access on this value");
        }
        Ok(self.expr(start, kind))
    }
}

/// Parses a JavaScript expression of a `.henshin` file. `map` turns offsets in `src` into
/// spans of the document. Expressions the textual syntax cannot express (`a ? b : c`, `null`,
/// `%`, …) are an error with the span of the offending part.
pub fn parse(src: &str, map: &dyn Fn(Range<usize>) -> Range<usize>) -> Result<Expr, (String, Range<usize>)> {
    let tokens = tokenize(src).map_err(|(m, s)| (m, map(s)))?;
    let mut p = Parser { tokens, pos: 0, map };
    let e = p.binary(0).map_err(|(m, s)| (m, if s.end <= src.len() { map(s) } else { s }))?;
    if p.peek() != &Tok::Eof {
        let what = if p.at("?") { "'?:'".to_string() } else { "this expression".to_string() };
        return p.unsupported(&what).map_err(|(m, s)| (m, map(s)));
    }
    Ok(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(src: &str) -> String {
        print(&parse(src, &|s| s).unwrap_or_else(|e| panic!("{src}: {e:?}")))
    }

    #[test]
    fn parses_and_prints_javascript() {
        assert_eq!(round_trip("x>amount"), "x>amount");
        assert_eq!(round_trip("x - amount"), "x-amount");
        assert_eq!(round_trip("(a+b)*c"), "(a+b)*c");
        assert_eq!(round_trip("a-(b-c)"), "a-(b-c)");
        assert_eq!(round_trip("a === 1 && !(b || c)"), "a==1 && !(b || c)");
        assert_eq!(round_trip("'it\\'s'"), "\"it's\"");
        assert_eq!(round_trip("-3"), "-3");
        assert_eq!(round_trip("Math.max(a, 2.5)"), "Math.max(a,2.5)");
        assert_eq!(round_trip("java.lang.Integer.MAX_VALUE"), "java.lang.Integer.MAX_VALUE");
    }

    #[test]
    fn rejects_what_the_textual_syntax_cannot_express() {
        for src in ["a ? b : c", "null", "a % 2", "x.y()[0]", "(a).b", "1 +"] {
            assert!(parse(src, &|s| s).is_err(), "{src}");
        }
        let (_, span) = parse("a ? b : c", &|s| s.start + 10..s.end + 10).unwrap_err();
        assert_eq!(span, 12..13);
    }
}
