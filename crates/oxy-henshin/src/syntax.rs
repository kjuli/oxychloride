//! The textual syntax of Henshin (`.henshin_text`, the Xtext grammar of
//! `org.eclipse.emf.henshin.text`): lexer, AST and a recovering parser.
//!
//! ```text
//! ePackageImport bank
//!
//! rule createAccount(IN client:EString, IN accountId:EInt) {
//!     graph {
//!         node bank:Bank
//!         node client:Client { name=client }
//!         create node account:Account { id=accountId }
//!         forbid node existing:Account { id=accountId }
//!         edges [(bank->client:clients), (create client->account:accounts),
//!                (forbid bank->existing:accounts)]
//!     }
//! }
//!
//! unit openAccounts(IN client:EString, IN id:EInt) {
//!     createAccount(client, id)
//! }
//! ```
//!
//! The parser never fails: problems are reported with byte ranges and parsing resumes at the
//! next element it recognizes. Keywords are those of the Xtext grammar; they cannot be used as
//! names.

use std::ops::Range;

use oxy_ecore::Problem;

// ---------------------------------------------------------------------- lexer

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    Int(i64),
    Real(f64),
    Str(String),
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Colon,
    Dot,
    /// `=`
    Assign,
    /// `->`
    Arrow,
    EqEq,
    NotEq,
    Ge,
    Le,
    Gt,
    Lt,
    Plus,
    Minus,
    Star,
    Slash,
    Bang,
    Eof,
}

impl Tok {
    pub fn is_ident(&self, word: &str) -> bool {
        matches!(self, Tok::Ident(w) if w == word)
    }

    fn describe(&self) -> String {
        match self {
            Tok::Ident(w) => format!("'{w}'"),
            Tok::Int(i) => i.to_string(),
            Tok::Real(r) => r.to_string(),
            Tok::Str(_) => "a string".into(),
            Tok::Eof => "the end of the document".into(),
            other => format!("'{}'", punctuation(other)),
        }
    }
}

fn punctuation(t: &Tok) -> &'static str {
    match t {
        Tok::LBrace => "{",
        Tok::RBrace => "}",
        Tok::LParen => "(",
        Tok::RParen => ")",
        Tok::LBracket => "[",
        Tok::RBracket => "]",
        Tok::Comma => ",",
        Tok::Colon => ":",
        Tok::Dot => ".",
        Tok::Assign => "=",
        Tok::Arrow => "->",
        Tok::EqEq => "==",
        Tok::NotEq => "!=",
        Tok::Ge => ">=",
        Tok::Le => "<=",
        Tok::Gt => ">",
        Tok::Lt => "<",
        Tok::Plus => "+",
        Tok::Minus => "-",
        Tok::Star => "*",
        Tok::Slash => "/",
        Tok::Bang => "!",
        _ => "?",
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: Tok,
    pub span: Range<usize>,
}

/// The keywords of the grammar (they cannot be names).
pub const KEYWORDS: &[&str] = &[
    "ePackageImport",
    "rule",
    "unit",
    "javaImport",
    "checkDangling",
    "injectiveMatching",
    "conditions",
    "graph",
    "preserve",
    "create",
    "delete",
    "forbid",
    "require",
    "edges",
    "node",
    "reuse",
    "set",
    "multiRule",
    "matchingFormula",
    "formula",
    "conditionGraph",
    "AND",
    "OR",
    "XOR",
    "strict",
    "rollback",
    "independent",
    "priority",
    "if",
    "then",
    "else",
    "for",
    "while",
    "IN",
    "OUT",
    "INOUT",
    "VAR",
    "true",
    "false",
];

pub fn is_keyword(word: &str) -> bool {
    KEYWORDS.contains(&word)
}

/// Splits a text into tokens (always ending with [`Tok::Eof`]). Comments (`//`, `/* */`) are
/// skipped.
pub fn tokenize(text: &str) -> (Vec<Token>, Vec<Problem>) {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut problems = Vec::new();
    let mut pos = 0;
    let peek = |i: usize| bytes.get(i).copied().unwrap_or(0);
    loop {
        loop {
            match (peek(pos), peek(pos + 1)) {
                (c, _) if c.is_ascii_whitespace() => pos += 1,
                (b'/', b'/') => {
                    while pos < bytes.len() && bytes[pos] != b'\n' {
                        pos += 1;
                    }
                }
                (b'/', b'*') => {
                    let start = pos;
                    pos += 2;
                    while pos < bytes.len() && !(bytes[pos] == b'*' && peek(pos + 1) == b'/') {
                        pos += 1;
                    }
                    if pos >= bytes.len() {
                        problems.push(Problem::error(start..start + 2, "unterminated comment"));
                    } else {
                        pos += 2;
                    }
                }
                (c, _) if c >= 0x80 && text[pos..].chars().next().is_some_and(char::is_whitespace) => {
                    pos += text[pos..].chars().next().map_or(1, char::len_utf8);
                }
                _ => break,
            }
        }
        let start = pos;
        if pos >= bytes.len() {
            tokens.push(Token { kind: Tok::Eof, span: pos..pos });
            return (tokens, problems);
        }
        let c = text[pos..].chars().next().unwrap_or('\0');
        let kind = if c.is_alphabetic() || c == '_' || c == '^' {
            // `^name` escapes a keyword in Xtext: the token keeps the `^` (so it is not a
            // keyword), names drop it.
            let skip = usize::from(c == '^');
            let len = text[pos + skip..]
                .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
                .unwrap_or(text.len() - pos - skip);
            pos += skip + len;
            Tok::Ident(text[start..pos].to_string())
        } else if c.is_ascii_digit() {
            while peek(pos).is_ascii_digit() {
                pos += 1;
            }
            let real = peek(pos) == b'.' && peek(pos + 1).is_ascii_digit();
            if real {
                pos += 1;
                while peek(pos).is_ascii_digit() {
                    pos += 1;
                }
            }
            let s = &text[start..pos];
            if real {
                Tok::Real(s.parse().unwrap_or(f64::NAN))
            } else {
                match s.parse() {
                    Ok(i) => Tok::Int(i),
                    Err(_) => {
                        problems.push(Problem::error(start..pos, "integer literal is too large"));
                        Tok::Int(0)
                    }
                }
            }
        } else if c == '"' || c == '\'' {
            let (value, end, ok) = string_literal(text, pos, c);
            if !ok {
                problems.push(Problem::error(start..end, "unterminated string literal"));
            }
            pos = end;
            Tok::Str(value)
        } else {
            let (kind, len) = match (c, peek(pos + 1)) {
                ('-', b'>') => (Tok::Arrow, 2),
                ('=', b'=') => (Tok::EqEq, 2),
                ('!', b'=') => (Tok::NotEq, 2),
                ('>', b'=') => (Tok::Ge, 2),
                ('<', b'=') => (Tok::Le, 2),
                ('{', _) => (Tok::LBrace, 1),
                ('}', _) => (Tok::RBrace, 1),
                ('(', _) => (Tok::LParen, 1),
                (')', _) => (Tok::RParen, 1),
                ('[', _) => (Tok::LBracket, 1),
                (']', _) => (Tok::RBracket, 1),
                (',', _) => (Tok::Comma, 1),
                (':', _) => (Tok::Colon, 1),
                ('.', _) => (Tok::Dot, 1),
                ('=', _) => (Tok::Assign, 1),
                ('>', _) => (Tok::Gt, 1),
                ('<', _) => (Tok::Lt, 1),
                ('+', _) => (Tok::Plus, 1),
                ('-', _) => (Tok::Minus, 1),
                ('*', _) => (Tok::Star, 1),
                ('/', _) => (Tok::Slash, 1),
                ('!', _) => (Tok::Bang, 1),
                _ => {
                    pos += c.len_utf8();
                    problems.push(Problem::error(start..pos, format!("unexpected character '{c}'")));
                    continue;
                }
            };
            pos += len;
            kind
        };
        tokens.push(Token { kind, span: start..pos });
    }
}

/// Reads a string literal starting at the opening quote (Xtext's `STRING`: either quote,
/// backslash escapes).
fn string_literal(text: &str, start: usize, quote: char) -> (String, usize, bool) {
    let mut out = String::new();
    let mut chars = text[start + 1..].char_indices();
    while let Some((i, c)) = chars.next() {
        let at = start + 1 + i;
        match c {
            c if c == quote => return (out, at + 1, true),
            '\n' => return (out, at, false),
            '\\' => match chars.next() {
                Some((_, 'n')) => out.push('\n'),
                Some((_, 't')) => out.push('\t'),
                Some((_, 'r')) => out.push('\r'),
                Some((_, e)) => out.push(e),
                None => break,
            },
            c => out.push(c),
        }
    }
    (out, text.len(), false)
}

// ---------------------------------------------------------------------- AST

#[derive(Clone, Debug, PartialEq)]
pub struct Name {
    pub text: String,
    pub span: Range<usize>,
}

/// A dotted name (`EString` in the grammar), e.g. `bank.Account`.
#[derive(Clone, Debug, PartialEq)]
pub struct QName {
    pub parts: Vec<Name>,
    pub span: Range<usize>,
}

impl QName {
    pub fn text(&self) -> String {
        self.parts.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join(".")
    }

    pub fn segments(&self) -> Vec<&str> {
        self.parts.iter().map(|p| p.text.as_str()).collect()
    }

    pub fn last(&self) -> &str {
        self.parts.last().map_or("", |p| p.text.as_str())
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document {
    pub imports: Vec<QName>,
    pub elements: Vec<Element>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Element {
    Rule(RuleDecl),
    Unit(UnitDecl),
}

impl Element {
    pub fn name(&self) -> &Name {
        match self {
            Element::Rule(r) => &r.name,
            Element::Unit(u) => &u.name,
        }
    }

    pub fn span(&self) -> &Range<usize> {
        match self {
            Element::Rule(r) => &r.span,
            Element::Unit(u) => &u.span,
        }
    }

    pub fn params(&self) -> &[ParamDecl] {
        match self {
            Element::Rule(r) => &r.params,
            Element::Unit(u) => &u.params,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParamKind {
    In,
    Out,
    InOut,
    Var,
    /// No kind given (`UNKNOWN` in Henshin).
    Unknown,
}

impl ParamKind {
    pub fn keyword(self) -> &'static str {
        match self {
            ParamKind::In => "IN",
            ParamKind::Out => "OUT",
            ParamKind::InOut => "INOUT",
            ParamKind::Var => "VAR",
            ParamKind::Unknown => "",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParamDecl {
    pub kind: ParamKind,
    pub name: Name,
    pub ty: QName,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RuleDecl {
    pub name: Name,
    pub params: Vec<ParamDecl>,
    /// The parameter list with its parentheses (empty, after the name, if there is none).
    pub params_span: Range<usize>,
    pub items: Vec<RuleItem>,
    /// The body with its braces (empty if there is none).
    pub body: Range<usize>,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RuleItem {
    /// `javaImport pkg`: the package and the span of the whole item.
    JavaImport(QName, Range<usize>),
    CheckDangling(bool, Range<usize>),
    InjectiveMatching(bool, Range<usize>),
    /// `conditions [...]`: the attribute conditions and the span of the whole item.
    Conditions(Vec<Expr>, Range<usize>),
    Graph(GraphDecl),
}

impl RuleItem {
    pub fn span(&self) -> Range<usize> {
        match self {
            RuleItem::JavaImport(_, span)
            | RuleItem::CheckDangling(_, span)
            | RuleItem::InjectiveMatching(_, span)
            | RuleItem::Conditions(_, span) => span.clone(),
            RuleItem::Graph(g) => g.span.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Preserve,
    Create,
    Delete,
    Forbid,
    Require,
}

impl Action {
    pub fn from_keyword(word: &str) -> Option<Action> {
        Some(match word {
            "preserve" => Action::Preserve,
            "create" => Action::Create,
            "delete" => Action::Delete,
            "forbid" => Action::Forbid,
            "require" => Action::Require,
            _ => return None,
        })
    }

    pub fn keyword(self) -> &'static str {
        match self {
            Action::Preserve => "preserve",
            Action::Create => "create",
            Action::Delete => "delete",
            Action::Forbid => "forbid",
            Action::Require => "require",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GraphDecl {
    pub items: Vec<GraphItem>,
    /// The body with its braces.
    pub body: Range<usize>,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GraphItem {
    Node(NodeDecl),
    Reuse(ReuseDecl),
    Edges(EdgesDecl),
    Formula(FormulaDecl),
    MultiRule(MultiRuleDecl),
}

impl GraphItem {
    pub fn span(&self) -> Range<usize> {
        match self {
            GraphItem::Node(n) => n.span.clone(),
            GraphItem::Reuse(r) => r.span.clone(),
            GraphItem::Edges(e) => e.span.clone(),
            GraphItem::Formula(f) => f.span.clone(),
            GraphItem::MultiRule(m) => m.span.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeDecl {
    pub action: Option<(Action, Range<usize>)>,
    pub name: Name,
    pub class: QName,
    pub attributes: Vec<AttributeDecl>,
    /// The attribute block with its braces, if there is one.
    pub attributes_span: Option<Range<usize>>,
    pub span: Range<usize>,
}

/// `reuse name { ... }`: a node of the enclosing rule (or graph), with further attributes.
#[derive(Clone, Debug, PartialEq)]
pub struct ReuseDecl {
    pub name: Name,
    pub attributes: Vec<AttributeDecl>,
    pub attributes_span: Option<Range<usize>>,
    pub span: Range<usize>,
}

/// `edges [...]`.
#[derive(Clone, Debug, PartialEq)]
pub struct EdgesDecl {
    pub edges: Vec<EdgeDecl>,
    /// From `edges` to the closing bracket.
    pub span: Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AttributeDecl {
    pub action: Option<(Action, Range<usize>)>,
    /// `set name = value`: the attribute is changed.
    pub set: bool,
    pub name: QName,
    pub value: Expr,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EdgeDecl {
    pub action: Option<(Action, Range<usize>)>,
    pub source: Name,
    pub target: Name,
    pub ty: QName,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MultiRuleDecl {
    pub name: Name,
    pub items: Vec<RuleItem>,
    /// The body with its braces.
    pub body: Range<usize>,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FormulaDecl {
    pub formula: Logic,
    /// `formula ...` (empty if missing).
    pub formula_span: Range<usize>,
    pub graphs: Vec<ConditionGraphDecl>,
    /// The body with its braces.
    pub body: Range<usize>,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Logic {
    Ref(Name),
    Not(Box<Logic>),
    And(Box<Logic>, Box<Logic>),
    Or(Box<Logic>, Box<Logic>),
    Xor(Box<Logic>, Box<Logic>),
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConditionGraphDecl {
    pub name: Name,
    pub items: Vec<GraphItem>,
    /// The body with its braces.
    pub body: Range<usize>,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UnitDecl {
    pub name: Name,
    pub params: Vec<ParamDecl>,
    /// The parameter list with its parentheses (empty, after the name, if there is none).
    pub params_span: Range<usize>,
    pub items: Vec<UnitItem>,
    /// The body with its braces (empty if there is none).
    pub body: Range<usize>,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum UnitItem {
    /// `rule(param, ...)`: a call of a rule or unit, passing parameters of the calling unit.
    Call {
        target: Name,
        args: Vec<Name>,
        span: Range<usize>,
    },
    /// `{ ... }`: a nested sequence.
    Sequence(Vec<UnitItem>, Range<usize>),
    Strict(bool, Range<usize>),
    Rollback(bool, Range<usize>),
    Independent(Vec<Vec<UnitItem>>, Range<usize>),
    Priority(Vec<Vec<UnitItem>>, Range<usize>),
    If {
        condition: Vec<UnitItem>,
        then: Vec<UnitItem>,
        otherwise: Option<Vec<UnitItem>>,
        span: Range<usize>,
    },
    For {
        count: Expr,
        body: Vec<UnitItem>,
        span: Range<usize>,
    },
    While {
        body: Vec<UnitItem>,
        span: Range<usize>,
    },
}

impl UnitItem {
    pub fn span(&self) -> &Range<usize> {
        match self {
            UnitItem::Call { span, .. }
            | UnitItem::Sequence(_, span)
            | UnitItem::Strict(_, span)
            | UnitItem::Rollback(_, span)
            | UnitItem::Independent(_, span)
            | UnitItem::Priority(_, span)
            | UnitItem::If { span, .. }
            | UnitItem::For { span, .. }
            | UnitItem::While { span, .. } => span,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Or,
    And,
    Eq,
    Ne,
    Ge,
    Le,
    Gt,
    Lt,
    Add,
    Sub,
    Mul,
    Div,
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Or => "OR",
            BinOp::And => "AND",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Ge => ">=",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Lt => "<",
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    /// A parameter (or, for enumeration-typed attributes, a literal).
    Name(String),
    /// `Type.NAME`: an enumeration literal (a Java attribute in Henshin).
    Qualified(Vec<String>),
    /// `name(args)`: a Java call (not supported by the engine).
    Call(String, Vec<Expr>),
    Str(String),
    Int(i64),
    Real(f64),
    Bool(bool),
    Not(Box<Expr>),
    Neg(Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Error,
}

impl Expr {
    /// Calls `f` on the names this expression uses.
    pub fn visit_names<'a>(&'a self, f: &mut impl FnMut(&'a str, &'a Range<usize>)) {
        match &self.kind {
            ExprKind::Name(n) => f(n, &self.span),
            ExprKind::Call(_, args) => args.iter().for_each(|a| a.visit_names(f)),
            ExprKind::Not(e) | ExprKind::Neg(e) => e.visit_names(f),
            ExprKind::Binary(_, l, r) => {
                l.visit_names(f);
                r.visit_names(f);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------- parser

/// Parses a Henshin text document. Never fails; problems are returned with byte ranges.
pub fn parse(text: &str) -> (Document, Vec<Problem>) {
    let (tokens, problems) = tokenize(text);
    let mut p = Parser { tokens, pos: 0, problems };
    let doc = p.document();
    (doc, p.problems)
}

/// Parses one expression (an attribute value, a condition). Never fails; problems are returned
/// with byte ranges.
pub fn parse_expr(text: &str) -> (Expr, Vec<Problem>) {
    let (tokens, problems) = tokenize(text);
    let mut p = Parser { tokens, pos: 0, problems };
    let e = p.expr();
    if !p.at_eof() {
        p.error_here("the end of the expression");
    }
    (e, p.problems)
}

/// Whether a text is a name of the textual syntax (keywords are names when escaped with `^`).
pub fn is_name(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_') && chars.all(|c| c.is_alphanumeric() || c == '_')
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    problems: Vec<Problem>,
}

/// Words that start a top-level element.
const TOP_LEVEL: &[&str] = &["ePackageImport", "rule", "unit"];

impl Parser {
    fn peek(&self) -> &Tok {
        &self.tokens[self.pos.min(self.tokens.len() - 1)].kind
    }

    fn peek_at(&self, n: usize) -> &Tok {
        &self.tokens[(self.pos + n).min(self.tokens.len() - 1)].kind
    }

    fn span(&self) -> Range<usize> {
        self.tokens[self.pos.min(self.tokens.len() - 1)].span.clone()
    }

    /// The end of the last consumed token.
    fn last_end(&self) -> usize {
        if self.pos == 0 { 0 } else { self.tokens[self.pos - 1].span.end }
    }

    fn bump(&mut self) -> Token {
        let t = self.tokens[self.pos.min(self.tokens.len() - 1)].clone();
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek(), Tok::Eof)
    }

    fn at_word(&self, word: &str) -> bool {
        self.peek().is_ident(word)
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == t {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_word(&mut self, word: &str) -> bool {
        if self.at_word(word) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn error_here(&mut self, expected: &str) {
        let found = self.peek().describe();
        let span = self.span();
        // One error per position is enough.
        if self.problems.last().is_some_and(|p| p.span == span) {
            return;
        }
        self.problems.push(Problem::error(span, format!("expected {expected}, found {found}")));
    }

    fn expect(&mut self, t: Tok) -> bool {
        if self.eat(&t) {
            true
        } else {
            self.error_here(&format!("'{}'", punctuation(&t)));
            false
        }
    }

    fn expect_word(&mut self, word: &str) -> bool {
        if self.eat_word(word) {
            true
        } else {
            self.error_here(&format!("'{word}'"));
            false
        }
    }

    /// A name that is not a keyword. On error, an empty name at the current position.
    fn name(&mut self, what: &str) -> Name {
        match self.peek().clone() {
            Tok::Ident(w) if !is_keyword(&w) => {
                let t = self.bump();
                let text = w.strip_prefix('^').map_or(w.clone(), str::to_string);
                Name { text, span: t.span }
            }
            _ => {
                self.error_here(what);
                let at = self.span().start;
                Name { text: String::new(), span: at..at }
            }
        }
    }

    fn qname(&mut self, what: &str) -> QName {
        let first = self.name(what);
        let start = first.span.start;
        let mut parts = vec![first];
        while matches!(self.peek(), Tok::Dot) && matches!(self.peek_at(1), Tok::Ident(_)) {
            self.bump();
            parts.push(self.name(what));
        }
        let end = parts.last().map_or(start, |p| p.span.end);
        QName { parts, span: start..end }
    }

    fn boolean(&mut self) -> bool {
        match self.peek() {
            t if t.is_ident("true") => {
                self.bump();
                true
            }
            t if t.is_ident("false") => {
                self.bump();
                false
            }
            _ => {
                self.error_here("'true' or 'false'");
                true
            }
        }
    }

    /// Skips a token; a skipped `{`, `(` or `[` skips up to its partner.
    fn skip(&mut self) {
        let open = self.bump().kind;
        let close = match open {
            Tok::LBrace => Tok::RBrace,
            Tok::LParen => Tok::RParen,
            Tok::LBracket => Tok::RBracket,
            _ => return,
        };
        let mut depth = 1;
        while !self.at_eof() {
            let t = self.bump().kind;
            if t == open {
                depth += 1;
            } else if t == close {
                depth -= 1;
                if depth == 0 {
                    return;
                }
            }
        }
    }

    // ------------------------------------------------------------------ elements

    fn document(&mut self) -> Document {
        let mut doc = Document::default();
        loop {
            match self.peek().clone() {
                Tok::Eof => break,
                t if t.is_ident("ePackageImport") => {
                    self.bump();
                    if let Tok::Str(s) = self.peek().clone() {
                        let span = self.bump().span;
                        doc.imports.push(QName { parts: vec![Name { text: s, span: span.clone() }], span });
                    } else {
                        doc.imports.push(self.qname("the name of a package"));
                    }
                }
                t if t.is_ident("rule") => doc.elements.push(Element::Rule(self.rule())),
                t if t.is_ident("unit") => doc.elements.push(Element::Unit(self.unit())),
                _ => {
                    self.error_here("'rule', 'unit' or 'ePackageImport'");
                    self.skip();
                    while !self.at_eof() && !TOP_LEVEL.iter().any(|w| self.at_word(w)) {
                        self.skip();
                    }
                }
            }
        }
        if doc.imports.is_empty() && !doc.elements.is_empty() {
            let at = doc.elements[0].span().start;
            self.problems.push(Problem::error(at..at, "a Henshin text starts with 'ePackageImport <package>'"));
        }
        doc
    }

    /// A parameter list, and its span with the parentheses.
    fn params(&mut self) -> (Vec<ParamDecl>, Range<usize>) {
        let mut params = Vec::new();
        let start = self.span().start;
        if !self.expect(Tok::LParen) {
            let at = self.last_end();
            return (params, at..at);
        }
        if self.eat(&Tok::RParen) {
            return (params, start..self.last_end());
        }
        loop {
            let start = self.span().start;
            let kind = match self.peek() {
                t if t.is_ident("IN") => ParamKind::In,
                t if t.is_ident("OUT") => ParamKind::Out,
                t if t.is_ident("INOUT") => ParamKind::InOut,
                t if t.is_ident("VAR") => ParamKind::Var,
                _ => ParamKind::Unknown,
            };
            if kind != ParamKind::Unknown {
                self.bump();
            }
            let name = self.name("a parameter name");
            // Henshin's parameters may have no type (the textual syntax of Henshin requires
            // one; models made with its editors do not).
            let ty = if self.eat(&Tok::Colon) {
                self.qname("a parameter type")
            } else {
                let at = self.last_end();
                QName { parts: Vec::new(), span: at..at }
            };
            params.push(ParamDecl { kind, name, ty, span: start..self.last_end() });
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen);
        (params, start..self.last_end())
    }

    /// A `{ ... }` block whose contents `inner` parses up to and including the closing brace;
    /// returns the contents and the span of the block (empty, here, if there is no block).
    fn block<T: Default>(&mut self, inner: impl FnOnce(&mut Self) -> T) -> (T, Range<usize>) {
        let start = self.span().start;
        if self.expect(Tok::LBrace) {
            let contents = inner(self);
            (contents, start..self.last_end())
        } else {
            let at = self.last_end();
            (T::default(), at..at)
        }
    }

    fn rule(&mut self) -> RuleDecl {
        let start = self.bump().span.start;
        let name = self.name("a rule name");
        let (params, params_span) = self.params();
        let (items, body) = self.block(Self::rule_items);
        RuleDecl { name, params, params_span, items, body, span: start..self.last_end() }
    }

    /// Rule elements up to and including the closing `}`.
    fn rule_items(&mut self) -> Vec<RuleItem> {
        let mut items = Vec::new();
        loop {
            match self.peek().clone() {
                Tok::RBrace => {
                    self.bump();
                    break;
                }
                Tok::Eof => {
                    self.error_here("'}'");
                    break;
                }
                t if t.is_ident("javaImport") => {
                    let start = self.bump().span.start;
                    let package = self.qname("a Java package");
                    items.push(RuleItem::JavaImport(package, start..self.last_end()));
                }
                t if t.is_ident("checkDangling") => {
                    let start = self.bump().span.start;
                    let value = self.boolean();
                    items.push(RuleItem::CheckDangling(value, start..self.last_end()));
                }
                t if t.is_ident("injectiveMatching") => {
                    let start = self.bump().span.start;
                    let value = self.boolean();
                    items.push(RuleItem::InjectiveMatching(value, start..self.last_end()));
                }
                t if t.is_ident("conditions") => {
                    let start = self.bump().span.start;
                    let mut conditions = Vec::new();
                    if self.expect(Tok::LBracket) {
                        loop {
                            conditions.push(self.expr());
                            if !self.eat(&Tok::Comma) {
                                break;
                            }
                        }
                        self.expect(Tok::RBracket);
                    }
                    items.push(RuleItem::Conditions(conditions, start..self.last_end()));
                }
                t if t.is_ident("graph") => {
                    let start = self.bump().span.start;
                    let (items_, body) = self.block(Self::graph_items);
                    items.push(RuleItem::Graph(GraphDecl { items: items_, body, span: start..self.last_end() }));
                }
                t if TOP_LEVEL.iter().any(|w| t.is_ident(w)) => {
                    self.error_here("'}'");
                    break;
                }
                _ => {
                    self.error_here("'graph', 'conditions', 'checkDangling', 'injectiveMatching' or 'javaImport'");
                    self.skip();
                }
            }
        }
        items
    }

    /// Graph elements up to and including the closing `}`.
    fn graph_items(&mut self) -> Vec<GraphItem> {
        let mut items = Vec::new();
        loop {
            let start = self.span().start;
            match self.peek().clone() {
                Tok::RBrace => {
                    self.bump();
                    break;
                }
                Tok::Eof => {
                    self.error_here("'}'");
                    break;
                }
                Tok::Ident(w) if w == "node" || Action::from_keyword(&w).is_some() => {
                    let action = Action::from_keyword(&w).map(|a| (a, self.bump().span));
                    if !self.expect_word("node") {
                        // e.g. `create x:...`: recover as if `node` were there.
                        if !matches!(self.peek(), Tok::Ident(_)) {
                            continue;
                        }
                    }
                    let name = self.name("a node name");
                    self.expect(Tok::Colon);
                    let class = self.qname("a class name");
                    let (attributes, attributes_span) = self.attributes();
                    items.push(GraphItem::Node(NodeDecl {
                        action,
                        name,
                        class,
                        attributes,
                        attributes_span,
                        span: start..self.last_end(),
                    }));
                }
                t if t.is_ident("reuse") => {
                    self.bump();
                    let name = self.name("the name of a node");
                    let (attributes, attributes_span) = self.attributes();
                    items.push(GraphItem::Reuse(ReuseDecl {
                        name,
                        attributes,
                        attributes_span,
                        span: start..self.last_end(),
                    }));
                }
                t if t.is_ident("edges") => {
                    self.bump();
                    let edges = self.edges();
                    items.push(GraphItem::Edges(EdgesDecl { edges, span: start..self.last_end() }));
                }
                t if t.is_ident("multiRule") => {
                    self.bump();
                    let name = self.name("a multi-rule name");
                    let (items_, body) = self.block(Self::rule_items);
                    items.push(GraphItem::MultiRule(MultiRuleDecl {
                        name,
                        items: items_,
                        body,
                        span: start..self.last_end(),
                    }));
                }
                t if t.is_ident("matchingFormula") => {
                    self.bump();
                    items.push(GraphItem::Formula(self.formula(start)));
                }
                t if TOP_LEVEL.iter().any(|w| t.is_ident(w)) => {
                    self.error_here("'}'");
                    break;
                }
                _ => {
                    self.error_here("'node', 'edges', 'reuse', 'multiRule' or 'matchingFormula'");
                    self.skip();
                }
            }
        }
        items
    }

    /// An optional `{ attribute* }` block, and its span with the braces.
    fn attributes(&mut self) -> (Vec<AttributeDecl>, Option<Range<usize>>) {
        let mut attributes = Vec::new();
        let start = self.span().start;
        if !self.eat(&Tok::LBrace) {
            return (attributes, None);
        }
        loop {
            let start = self.span().start;
            match self.peek().clone() {
                Tok::RBrace => {
                    self.bump();
                    break;
                }
                Tok::Eof => {
                    self.error_here("'}'");
                    break;
                }
                Tok::Ident(w) if w == "set" || Action::from_keyword(&w).is_some() || !is_keyword(&w) => {
                    let set = w == "set";
                    let action = Action::from_keyword(&w).map(|a| (a, self.span()));
                    if set || action.is_some() {
                        self.bump();
                    }
                    let name = self.qname("an attribute name");
                    self.expect(Tok::Assign);
                    let value = self.expr();
                    attributes.push(AttributeDecl { action, set, name, value, span: start..self.last_end() });
                    // Commas between attributes are tolerated.
                    self.eat(&Tok::Comma);
                }
                _ => {
                    self.error_here("an attribute or '}'");
                    if matches!(self.peek(), Tok::Ident(_)) {
                        break;
                    }
                    self.skip();
                }
            }
        }
        (attributes, Some(start..self.last_end()))
    }

    fn edges(&mut self) -> Vec<EdgeDecl> {
        let mut edges = Vec::new();
        if !self.expect(Tok::LBracket) {
            return edges;
        }
        loop {
            let start = self.span().start;
            if !self.eat(&Tok::LParen) {
                self.error_here("'(' starting an edge");
                break;
            }
            let action = match self.peek().clone() {
                Tok::Ident(w) => Action::from_keyword(&w).map(|a| (a, self.bump().span)),
                _ => None,
            };
            let source = self.name("the name of the source node");
            self.expect(Tok::Arrow);
            let target = self.name("the name of the target node");
            self.expect(Tok::Colon);
            let ty = self.qname("the name of a reference");
            self.expect(Tok::RParen);
            edges.push(EdgeDecl { action, source, target, ty, span: start..self.last_end() });
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        if !self.expect(Tok::RBracket) {
            // Resynchronize after the list.
            while !matches!(self.peek(), Tok::RBracket | Tok::RBrace | Tok::Eof) && !self.at_word("node") {
                self.skip();
            }
            self.eat(&Tok::RBracket);
        }
        edges
    }

    fn formula(&mut self, start: usize) -> FormulaDecl {
        let mut graphs = Vec::new();
        let mut formula = Logic::Error;
        let body_start = self.span().start;
        let mut formula_span = body_start..body_start;
        let mut body = self.last_end()..self.last_end();
        if self.expect(Tok::LBrace) {
            let formula_start = self.span().start;
            if self.expect_word("formula") {
                formula = self.logic_or();
                formula_span = formula_start..self.last_end();
            }
            loop {
                let gstart = self.span().start;
                match self.peek().clone() {
                    Tok::RBrace => {
                        self.bump();
                        break;
                    }
                    Tok::Eof => {
                        self.error_here("'}'");
                        break;
                    }
                    t if t.is_ident("conditionGraph") => {
                        self.bump();
                        let name = self.name("a condition graph name");
                        let (items, body) = self.block(Self::graph_items);
                        graphs.push(ConditionGraphDecl { name, items, body, span: gstart..self.last_end() });
                    }
                    _ => {
                        self.error_here("'conditionGraph' or '}'");
                        if TOP_LEVEL.iter().any(|w| self.at_word(w)) {
                            break;
                        }
                        self.skip();
                    }
                }
            }
            body = body_start..self.last_end();
        }
        FormulaDecl { formula, formula_span, graphs, body, span: start..self.last_end() }
    }

    fn logic_or(&mut self) -> Logic {
        let mut left = self.logic_and();
        loop {
            if self.eat_word("OR") {
                left = Logic::Or(Box::new(left), Box::new(self.logic_and()));
            } else if self.eat_word("XOR") {
                left = Logic::Xor(Box::new(left), Box::new(self.logic_and()));
            } else {
                return left;
            }
        }
    }

    fn logic_and(&mut self) -> Logic {
        let mut left = self.logic_primary();
        while self.eat_word("AND") {
            left = Logic::And(Box::new(left), Box::new(self.logic_primary()));
        }
        left
    }

    fn logic_primary(&mut self) -> Logic {
        if self.eat(&Tok::LParen) {
            let l = self.logic_or();
            self.expect(Tok::RParen);
            l
        } else if self.eat(&Tok::Bang) {
            Logic::Not(Box::new(self.logic_primary()))
        } else {
            match self.peek().clone() {
                Tok::Ident(w) if !is_keyword(&w) => Logic::Ref(self.name("")),
                _ => {
                    self.error_here("the name of a condition graph");
                    Logic::Error
                }
            }
        }
    }

    // ------------------------------------------------------------------ units

    fn unit(&mut self) -> UnitDecl {
        let start = self.bump().span.start;
        let name = self.name("a unit name");
        let (params, params_span) = self.params();
        let (items, body) = self.block(Self::unit_block);
        UnitDecl { name, params, params_span, items, body, span: start..self.last_end() }
    }

    /// Unit elements up to and including the closing `}`.
    fn unit_block(&mut self) -> Vec<UnitItem> {
        let mut items = Vec::new();
        loop {
            match self.peek() {
                Tok::RBrace => {
                    self.bump();
                    break;
                }
                Tok::Eof => {
                    self.error_here("'}'");
                    break;
                }
                t if TOP_LEVEL.iter().any(|w| t.is_ident(w)) => {
                    self.error_here("'}'");
                    break;
                }
                _ => match self.unit_item() {
                    Some(item) => items.push(item),
                    None => self.skip(),
                },
            }
        }
        items
    }

    /// Unit elements until one of the `stops` (not consumed).
    fn unit_list(&mut self, stops: &[Tok]) -> Vec<UnitItem> {
        let mut items = Vec::new();
        while !stops.contains(self.peek()) && !self.at_eof() && !TOP_LEVEL.iter().any(|w| self.at_word(w)) {
            match self.unit_item() {
                Some(item) => items.push(item),
                None => self.skip(),
            }
        }
        items
    }

    /// One unit element, or `None` (with an error) if there is none here.
    fn unit_item(&mut self) -> Option<UnitItem> {
        let start = self.span().start;
        let item = match self.peek().clone() {
            Tok::LBrace => {
                self.bump();
                let items = self.unit_block();
                UnitItem::Sequence(items, start..self.last_end())
            }
            t if t.is_ident("strict") => {
                self.bump();
                let v = self.boolean();
                UnitItem::Strict(v, start..self.last_end())
            }
            t if t.is_ident("rollback") => {
                self.bump();
                let v = self.boolean();
                UnitItem::Rollback(v, start..self.last_end())
            }
            t if t.is_ident("independent") || t.is_ident("priority") => {
                let independent = t.is_ident("independent");
                self.bump();
                let mut lists = Vec::new();
                if self.expect(Tok::LBracket) {
                    loop {
                        lists.push(self.unit_list(&[Tok::Comma, Tok::RBracket, Tok::RBrace]));
                        if !self.eat(&Tok::Comma) {
                            break;
                        }
                    }
                    self.expect(Tok::RBracket);
                }
                let span = start..self.last_end();
                if independent { UnitItem::Independent(lists, span) } else { UnitItem::Priority(lists, span) }
            }
            t if t.is_ident("if") => {
                self.bump();
                let mut condition = Vec::new();
                if self.expect(Tok::LParen) {
                    condition = self.unit_list(&[Tok::RParen, Tok::RBrace]);
                    self.expect(Tok::RParen);
                }
                self.expect_word("then");
                let then = if self.expect(Tok::LBrace) { self.unit_block() } else { Vec::new() };
                let otherwise = if self.eat_word("else") {
                    Some(if self.expect(Tok::LBrace) { self.unit_block() } else { Vec::new() })
                } else {
                    None
                };
                UnitItem::If { condition, then, otherwise, span: start..self.last_end() }
            }
            t if t.is_ident("for") => {
                self.bump();
                let mut count = Expr { kind: ExprKind::Error, span: start..start };
                if self.expect(Tok::LParen) {
                    count = self.expr();
                    self.expect(Tok::RParen);
                }
                let body = if self.expect(Tok::LBrace) { self.unit_block() } else { Vec::new() };
                UnitItem::For { count, body, span: start..self.last_end() }
            }
            t if t.is_ident("while") => {
                self.bump();
                let body = if self.expect(Tok::LBrace) { self.unit_block() } else { Vec::new() };
                UnitItem::While { body, span: start..self.last_end() }
            }
            Tok::Ident(w) if !is_keyword(&w) => {
                let target = self.name("");
                let mut args = Vec::new();
                if self.expect(Tok::LParen) && !self.eat(&Tok::RParen) {
                    loop {
                        args.push(self.name("a parameter of the unit"));
                        if !self.eat(&Tok::Comma) {
                            break;
                        }
                    }
                    self.expect(Tok::RParen);
                }
                UnitItem::Call { target, args, span: start..self.last_end() }
            }
            _ => {
                self.error_here(
                    "a call of a rule or unit, or a unit ('independent', 'priority', 'if', 'for', 'while', '{')",
                );
                return None;
            }
        };
        Some(item)
    }

    // ------------------------------------------------------------------ expressions

    fn expr(&mut self) -> Expr {
        self.binary(0)
    }

    fn binary(&mut self, level: usize) -> Expr {
        const LEVELS: &[&[(Option<&str>, Tok, BinOp)]] = &[
            &[(Some("OR"), Tok::Eof, BinOp::Or)],
            &[(Some("AND"), Tok::Eof, BinOp::And)],
            &[(None, Tok::EqEq, BinOp::Eq), (None, Tok::NotEq, BinOp::Ne)],
            &[
                (None, Tok::Ge, BinOp::Ge),
                (None, Tok::Le, BinOp::Le),
                (None, Tok::Gt, BinOp::Gt),
                (None, Tok::Lt, BinOp::Lt),
            ],
            &[(None, Tok::Plus, BinOp::Add), (None, Tok::Minus, BinOp::Sub)],
            &[(None, Tok::Star, BinOp::Mul), (None, Tok::Slash, BinOp::Div)],
        ];
        if level == LEVELS.len() {
            return self.primary();
        }
        let mut left = self.binary(level + 1);
        'outer: loop {
            for (word, tok, op) in LEVELS[level] {
                let matched = match word {
                    Some(w) => self.at_word(w),
                    None => self.peek() == tok,
                };
                if matched {
                    self.bump();
                    let right = self.binary(level + 1);
                    let span = left.span.start..right.span.end.max(left.span.end);
                    left = Expr { kind: ExprKind::Binary(*op, Box::new(left), Box::new(right)), span };
                    continue 'outer;
                }
            }
            return left;
        }
    }

    fn primary(&mut self) -> Expr {
        let start = self.span().start;
        let kind = match self.peek().clone() {
            Tok::LParen => {
                self.bump();
                let e = self.expr();
                self.expect(Tok::RParen);
                return Expr { kind: e.kind, span: start..self.last_end() };
            }
            Tok::Bang => {
                self.bump();
                ExprKind::Not(Box::new(self.primary()))
            }
            Tok::Minus => {
                self.bump();
                ExprKind::Neg(Box::new(self.primary()))
            }
            Tok::Str(s) => {
                self.bump();
                ExprKind::Str(s)
            }
            Tok::Int(i) => {
                self.bump();
                ExprKind::Int(i)
            }
            Tok::Real(r) => {
                self.bump();
                ExprKind::Real(r)
            }
            t if t.is_ident("true") || t.is_ident("false") => ExprKind::Bool(self.boolean()),
            Tok::Ident(w) if !is_keyword(&w) => {
                let q = self.qname("");
                if matches!(self.peek(), Tok::LParen) {
                    self.bump();
                    let mut args = Vec::new();
                    if !self.eat(&Tok::RParen) {
                        loop {
                            args.push(self.expr());
                            if !self.eat(&Tok::Comma) {
                                break;
                            }
                        }
                        self.expect(Tok::RParen);
                    }
                    ExprKind::Call(q.text(), args)
                } else if q.parts.len() == 1 {
                    ExprKind::Name(q.text())
                } else {
                    ExprKind::Qualified(q.parts.into_iter().map(|p| p.text).collect())
                }
            }
            _ => {
                self.error_here("an expression");
                let span = self.span();
                return Expr { kind: ExprKind::Error, span: span.start..span.start };
            }
        };
        Expr { kind, span: start..self.last_end() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_bank_example() {
        let text = r#"
ePackageImport bank
rule createAccount(IN client:EString,IN accountId:EInt){
	graph{
		node bank:Bank
		preserve node manager:Manager
		create node newAccount:Account{
			create id=accountId
		}
		forbid node account:Account{
			id=accountId
		}
		edges[(bank->manager:managers),(forbid bank->account:accounts)]
		edges[(create bank-> newAccount:accounts)]
	}
}
unit planCourseOrIncrement(IN currentHour:EInt){
	if (planOneCorse(currentHour)) then {
		planUnscheduledInterestingCourses(currentHour)
	}else {
		incrementIfPossible(currentHour)
	}
	for(2){ a() }
	independent [ a() b(), c() ]
}
"#;
        let (doc, problems) = parse(text);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(doc.imports[0].text(), "bank");
        let Element::Rule(rule) = &doc.elements[0] else { panic!() };
        assert_eq!(rule.params.len(), 2);
        let RuleItem::Graph(g) = &rule.items[0] else { panic!() };
        assert_eq!(g.items.len(), 6);
        let Element::Unit(unit) = &doc.elements[1] else { panic!() };
        assert_eq!(unit.items.len(), 3);
        let UnitItem::Independent(lists, _) = &unit.items[2] else { panic!() };
        assert_eq!(lists.iter().map(Vec::len).collect::<Vec<_>>(), [2, 1]);
    }

    #[test]
    fn recovers_from_errors() {
        let (doc, problems) = parse("ePackageImport x\nrule a() { graph { node : } }\nrule b() { graph { node n:N } }");
        assert!(!problems.is_empty());
        assert_eq!(doc.elements.len(), 2);
        let (_, problems) = parse("ePackageImport x\nrule a() { conditions [x > ] }");
        assert_eq!(problems.len(), 1, "{problems:?}");
    }

    #[test]
    fn parses_expressions_with_precedence() {
        let (doc, problems) = parse("ePackageImport x rule r() { conditions [a + 2 * b >= 3 AND !c OR d == 'x'] }");
        assert!(problems.is_empty(), "{problems:?}");
        let Element::Rule(r) = &doc.elements[0] else { panic!() };
        let RuleItem::Conditions(c, _) = &r.items[0] else { panic!() };
        let ExprKind::Binary(BinOp::Or, l, _) = &c[0].kind else { panic!("{:?}", c[0]) };
        assert!(matches!(l.kind, ExprKind::Binary(BinOp::And, _, _)));
    }
}
