//! HUTN, the OMG *Human-Usable Textual Notation*: a generic text syntax for instance models
//! of any metamodel (the notation Epsilon implements for EMF).
//!
//! ```text
//! @Spec {
//!   metamodel "library" { nsUri: "http://www.example.org/library/1.0" }
//! }
//!
//! library {
//!   Library "city" {
//!     name: "City Library"
//!     books: Book "dune" { name: "Dune" copies: 2 genre: FICTION authors: Author "herbert" }
//!     authors: Author "herbert" { name: "Frank Herbert" }
//!   }
//! }
//! ```
//!
//! * A *package block* (`library { ... }`) holds root objects. It names a package by its name,
//!   its nsPrefix, or the name given to it in `@Spec`. Objects may also appear at the top level.
//! * An *object* is `[adjectives] Class ["identifier"] { slots }`. Identifiers name objects for
//!   references; they must be unique in a document and become the objects' `xmi:id`s.
//! * A *slot* is `feature: value, value, ...`. Values are strings (`"..."`), numbers, `true`,
//!   `false`, `null`, enumeration literals, nested objects (for containment features) and
//!   references `Class "identifier"` (or just `"identifier"`) to objects defined anywhere in
//!   the document.
//! * *Adjectives* set Boolean attributes: `nuclear Family "x" {}` sets `nuclear` to `true`,
//!   `~nuclear` to `false`.
//! * Comments are `// ...` and `/* ... */`.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::ops::Range;

use crate::error::Problem;
use crate::metamodel::*;
use crate::model::*;
use crate::xmi_writer::is_persistent;

// ------------------------------------------------------------------ lexer

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    Str(String),
    Int(i64),
    Real(f64),
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Colon,
    ColonColon,
    Comma,
    Semi,
    At,
    Tilde,
    Eof,
}

impl Tok {
    fn describe(&self) -> String {
        match self {
            Tok::Ident(n) => format!("'{n}'"),
            Tok::Str(_) => "a string".into(),
            Tok::Int(_) | Tok::Real(_) => "a number".into(),
            Tok::LBrace => "'{'".into(),
            Tok::RBrace => "'}'".into(),
            Tok::LBracket => "'['".into(),
            Tok::RBracket => "']'".into(),
            Tok::Colon => "':'".into(),
            Tok::ColonColon => "'::'".into(),
            Tok::Comma => "','".into(),
            Tok::Semi => "';'".into(),
            Tok::At => "'@'".into(),
            Tok::Tilde => "'~'".into(),
            Tok::Eof => "end of input".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: Tok,
    pub span: Range<usize>,
}

/// Splits a HUTN text into tokens (always ending with [`Tok::Eof`]). Comments are skipped.
pub fn tokenize(text: &str) -> (Vec<Token>, Vec<Problem>) {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut problems = Vec::new();
    let mut pos = 0;
    let peek = |i: usize| bytes.get(i).copied().unwrap_or(0);
    loop {
        // Whitespace and comments.
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
                    pos += text[pos..].chars().next().unwrap().len_utf8();
                }
                _ => break,
            }
        }
        let start = pos;
        if pos >= bytes.len() {
            tokens.push(Token { kind: Tok::Eof, span: pos..pos });
            return (tokens, problems);
        }
        let c = text[pos..].chars().next().unwrap();
        let kind = if c.is_alphabetic() || c == '_' {
            let len = text[pos..].find(|ch: char| !(ch.is_alphanumeric() || ch == '_')).unwrap_or(text.len() - pos);
            pos += len;
            Tok::Ident(text[start..pos].to_string())
        } else if c.is_ascii_digit() || (c == '-' && peek(pos + 1).is_ascii_digit()) {
            pos += 1;
            while peek(pos).is_ascii_digit() {
                pos += 1;
            }
            let mut real = false;
            if peek(pos) == b'.' && peek(pos + 1).is_ascii_digit() {
                real = true;
                pos += 1;
                while peek(pos).is_ascii_digit() {
                    pos += 1;
                }
            }
            if matches!(peek(pos), b'e' | b'E') {
                let sign = usize::from(matches!(peek(pos + 1), b'+' | b'-'));
                if peek(pos + 1 + sign).is_ascii_digit() {
                    real = true;
                    pos += 1 + sign;
                    while peek(pos).is_ascii_digit() {
                        pos += 1;
                    }
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
                (':', b':') => (Tok::ColonColon, 2),
                ('{', _) => (Tok::LBrace, 1),
                ('}', _) => (Tok::RBrace, 1),
                ('[', _) => (Tok::LBracket, 1),
                (']', _) => (Tok::RBracket, 1),
                (':', _) => (Tok::Colon, 1),
                (',', _) => (Tok::Comma, 1),
                (';', _) => (Tok::Semi, 1),
                ('@', _) => (Tok::At, 1),
                ('~', _) => (Tok::Tilde, 1),
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

/// Reads a string literal starting at the opening quote. Strings end at the closing quote
/// or (unterminated) at the end of the line. Returns the value, the end offset and whether
/// the literal was terminated.
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

/// Writes a string as a HUTN literal.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ------------------------------------------------------------------ syntax

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Document {
    /// Metamodels declared in `@Spec`.
    pub spec: Vec<MetamodelSpec>,
    /// Top-level blocks: package blocks or root objects.
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MetamodelSpec {
    pub name: String,
    pub ns_uri: Option<String>,
    pub span: Range<usize>,
}

/// A possibly qualified name, `Class` or `pkg::Class`.
#[derive(Clone, Debug, PartialEq)]
pub struct Name {
    pub path: Vec<String>,
    pub span: Range<usize>,
}

impl Name {
    pub fn text(&self) -> String {
        self.path.join("::")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Adjective {
    pub name: String,
    pub negated: bool,
    pub span: Range<usize>,
}

/// `[adjectives] Name ["identifier"] [{ items }]`: an object, or (at the top level) a
/// package block.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub adjectives: Vec<Adjective>,
    pub name: Name,
    pub identifier: Option<(String, Range<usize>)>,
    pub items: Vec<Item>,
    /// From the first adjective to the closing brace.
    pub span: Range<usize>,
    /// The braces, if the block has a body.
    pub body: Option<Range<usize>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Slot(Slot),
    Block(Block),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub feature: String,
    pub feature_span: Range<usize>,
    pub values: Vec<Value>,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Value {
    pub kind: ValueKind,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ValueKind {
    Str(String),
    Int(i64),
    Real(f64),
    Bool(bool),
    Null,
    /// A bare name: an enumeration literal.
    Name(Name),
    /// A nested object (with a body).
    Object(Block),
    /// `Class "identifier"`: a reference to an object defined elsewhere.
    Reference {
        class: Name,
        identifier: String,
    },
}

/// Parses a HUTN document. Never fails: problems are reported and the parser recovers.
pub fn parse(text: &str) -> (Document, Vec<Problem>) {
    let (tokens, problems) = tokenize(text);
    let mut p = Parser { tokens, pos: 0, problems };
    let doc = p.document();
    (doc, p.problems)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    problems: Vec<Problem>,
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.tokens[self.pos].kind
    }

    fn peek_at(&self, n: usize) -> &Tok {
        &self.tokens[(self.pos + n).min(self.tokens.len() - 1)].kind
    }

    fn span(&self) -> Range<usize> {
        self.tokens[self.pos].span.clone()
    }

    fn prev_end(&self) -> usize {
        self.tokens[self.pos.saturating_sub(1)].span.end
    }

    fn bump(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, kind: &Tok) -> bool {
        if std::mem::discriminant(self.peek()) == std::mem::discriminant(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn error(&mut self, span: Range<usize>, message: impl Into<String>) {
        if self.problems.last().is_some_and(|p| p.span == span) {
            return;
        }
        self.problems.push(Problem::error(span, message));
    }

    fn unexpected(&mut self, what: &str) {
        let span = self.span();
        let found = self.peek().describe();
        self.error(span, format!("expected {what}, found {found}"));
    }

    /// `Ident :` (but not `Ident ::`) starts a slot.
    fn at_slot(&self) -> bool {
        matches!(self.peek(), Tok::Ident(_)) && matches!(self.peek_at(1), Tok::Colon)
    }

    fn at_block_start(&self) -> bool {
        matches!(self.peek(), Tok::Ident(_) | Tok::Tilde)
    }

    fn document(&mut self) -> Document {
        let mut doc = Document::default();
        loop {
            match self.peek() {
                Tok::Eof => return doc,
                Tok::At => self.spec(&mut doc),
                Tok::Semi | Tok::Comma => {
                    self.bump();
                }
                _ if self.at_slot() => {
                    // A slot outside any object: report it and skip it.
                    let span = self.span();
                    self.error(span, "slots must appear inside an object's braces");
                    self.slot();
                }
                _ if self.at_block_start() => {
                    if let Some(b) = self.block() {
                        doc.blocks.push(b);
                    }
                }
                _ => {
                    self.unexpected("a package block or an object");
                    self.bump();
                }
            }
        }
    }

    fn spec(&mut self, doc: &mut Document) {
        let start = self.bump().span.start;
        match self.peek().clone() {
            Tok::Ident(n) if n == "Spec" => {
                self.bump();
            }
            _ => self.unexpected("'Spec' after '@'"),
        }
        if !self.eat(&Tok::LBrace) {
            self.unexpected("'{'");
            return;
        }
        loop {
            match self.peek().clone() {
                Tok::RBrace => {
                    self.bump();
                    return;
                }
                Tok::Eof => {
                    self.error(start..start + 1, "missing '}' to close '@Spec'");
                    return;
                }
                Tok::Ident(w) if w == "metamodel" => {
                    let s = self.bump().span.start;
                    let name = match self.peek().clone() {
                        Tok::Str(n) => {
                            self.bump();
                            n
                        }
                        _ => {
                            self.unexpected("the metamodel name as a string");
                            String::new()
                        }
                    };
                    let mut ns_uri = None;
                    if self.eat(&Tok::LBrace) {
                        while !matches!(self.peek(), Tok::RBrace | Tok::Eof) {
                            if self.at_slot() {
                                let slot = self.slot();
                                if slot.feature == "nsUri"
                                    && let Some(Value { kind: ValueKind::Str(u), .. }) = slot.values.first()
                                {
                                    ns_uri = Some(u.clone());
                                }
                            } else {
                                self.unexpected("'nsUri: \"...\"'");
                                self.bump();
                            }
                        }
                        if !self.eat(&Tok::RBrace) {
                            self.error(s..s + 9, "missing '}' to close 'metamodel'");
                        }
                    }
                    doc.spec.push(MetamodelSpec { name, ns_uri, span: s..self.prev_end() });
                }
                _ => {
                    self.unexpected("'metamodel' or '}'");
                    self.bump();
                }
            }
        }
    }

    /// Parses `[~]adjective* Name ["identifier"] [{ ... }]`.
    fn block(&mut self) -> Option<Block> {
        let start = self.span().start;
        let mut words: Vec<(bool, Name)> = Vec::new();
        while self.at_block_start() && !self.at_slot() {
            let negated = self.eat(&Tok::Tilde);
            let Tok::Ident(first) = self.peek().clone() else {
                self.unexpected("a name after '~'");
                break;
            };
            let s = self.bump().span.start;
            let mut path = vec![first];
            while matches!(self.peek(), Tok::ColonColon) {
                self.bump();
                match self.peek().clone() {
                    Tok::Ident(n) => {
                        self.bump();
                        path.push(n);
                    }
                    _ => {
                        self.unexpected("a name after '::'");
                        break;
                    }
                }
            }
            words.push((negated, Name { path, span: s..self.prev_end() }));
        }
        let (negated, name) = words.pop()?;
        if negated {
            self.error(name.span.clone(), "'~' can only negate an adjective, not a class name");
        }
        let adjectives = words
            .into_iter()
            .map(|(negated, n)| {
                if n.path.len() > 1 {
                    self.error(n.span.clone(), "an adjective must be a simple name");
                }
                Adjective { name: n.text(), negated, span: n.span }
            })
            .collect();
        let identifier = match self.peek().clone() {
            Tok::Str(s) => {
                let span = self.bump().span;
                Some((s, span))
            }
            _ => None,
        };
        let mut items = Vec::new();
        let mut body = None;
        if matches!(self.peek(), Tok::LBrace) {
            let open = self.bump().span.start;
            loop {
                match self.peek() {
                    Tok::RBrace => {
                        self.bump();
                        break;
                    }
                    Tok::Eof => {
                        self.error(name.span.clone(), format!("missing '}}' to close '{}'", name.text()));
                        break;
                    }
                    Tok::Semi | Tok::Comma => {
                        self.bump();
                    }
                    _ if self.at_slot() => items.push(Item::Slot(self.slot())),
                    _ if self.at_block_start() => {
                        if let Some(b) = self.block() {
                            items.push(Item::Block(b));
                        }
                    }
                    _ => {
                        self.unexpected("a slot ('feature: value') or '}'");
                        self.bump();
                    }
                }
            }
            body = Some(open..self.prev_end());
        }
        Some(Block { adjectives, name, identifier, items, span: start..self.prev_end(), body })
    }

    fn slot(&mut self) -> Slot {
        let Tok::Ident(feature) = self.peek().clone() else { unreachable!("at_slot was checked") };
        let feature_span = self.bump().span;
        self.bump(); // ':'
        let mut values = Vec::new();
        loop {
            match self.value() {
                Some(v) => values.push(v),
                None => {
                    self.unexpected("a value");
                    break;
                }
            }
            // A comma followed by the next slot separates slots, which is tolerated.
            if !self.eat(&Tok::Comma) || self.at_slot() {
                break;
            }
        }
        self.eat(&Tok::Semi);
        let span = feature_span.start..self.prev_end();
        Slot { feature, feature_span, values, span }
    }

    fn value(&mut self) -> Option<Value> {
        let span = self.span();
        let kind = match self.peek().clone() {
            Tok::Str(s) => {
                self.bump();
                ValueKind::Str(s)
            }
            Tok::Int(i) => {
                self.bump();
                ValueKind::Int(i)
            }
            Tok::Real(r) => {
                self.bump();
                ValueKind::Real(r)
            }
            Tok::Ident(_) | Tok::Tilde if !self.at_slot() => {
                let block = self.block()?;
                let span = block.span.clone();
                let kind = if block.body.is_some() {
                    ValueKind::Object(block)
                } else if let Some((identifier, _)) = block.identifier {
                    if let Some(a) = block.adjectives.first() {
                        self.error(a.span.clone(), "adjectives are only allowed on object definitions");
                    }
                    ValueKind::Reference { class: block.name, identifier }
                } else {
                    if let Some(a) = block.adjectives.first() {
                        self.error(a.span.start..block.name.span.end, "expected a single value");
                    }
                    match block.name.text().as_str() {
                        "true" => ValueKind::Bool(true),
                        "false" => ValueKind::Bool(false),
                        "null" => ValueKind::Null,
                        _ => ValueKind::Name(block.name),
                    }
                };
                return Some(Value { kind, span });
            }
            _ => return None,
        };
        Some(Value { kind, span })
    }
}

// ------------------------------------------------------------------ loading

/// The outcome of loading a HUTN document.
#[derive(Debug, Clone)]
pub struct HutnLoad {
    /// Index of the new resource in [`Model::resources`].
    pub resource: usize,
    pub problems: Vec<Problem>,
    /// Every object created, in document order.
    pub objects: Vec<HutnObject>,
}

/// An object defined in a HUTN document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HutnObject {
    pub object: ObjectId,
    /// The whole definition, from the first adjective to the closing brace.
    pub span: Range<usize>,
    /// The class name, where problems with the object are shown.
    pub name: Range<usize>,
}

/// Parses `text` and adds its objects to `model` as a new resource named `uri`. Classes are
/// resolved in `mm`. Problems (syntax errors, unknown classes or features, type mismatches,
/// unresolved references, unset required features) are reported; loading always succeeds.
pub fn load(text: &str, uri: &str, mm: &Metamodel, model: &mut Model) -> HutnLoad {
    let (doc, problems) = parse(text);
    load_document(&doc, problems, uri, mm, model)
}

/// Loads an already parsed document (see [`load`]).
pub fn load_document(doc: &Document, problems: Vec<Problem>, uri: &str, mm: &Metamodel, model: &mut Model) -> HutnLoad {
    let resource = model.add_resource(uri);
    let mut l = Loader {
        mm,
        model,
        resource,
        problems,
        aliases: HashMap::new(),
        ids: HashMap::new(),
        pending: Vec::new(),
        objects: Vec::new(),
    };
    for s in &doc.spec {
        let Some(ns) = &s.ns_uri else { continue };
        match mm.package_by_ns_uri(ns) {
            Some(p) => {
                l.aliases.insert(s.name.clone(), p);
            }
            None => l.problems.push(Problem::error(
                s.span.clone(),
                format!("no metamodel with nsURI '{ns}' is loaded (load its .ecore or OCLinEcore document first)"),
            )),
        }
    }
    for b in &doc.blocks {
        match package_block(mm, &l.aliases, b) {
            Some(p) => {
                for item in &b.items {
                    match item {
                        Item::Block(o) => {
                            l.object(o, Some(p), None);
                        }
                        Item::Slot(s) => l
                            .problems
                            .push(Problem::error(s.span.clone(), "a package block contains objects, not slots")),
                    }
                }
            }
            None => {
                l.object(b, None, None);
            }
        }
    }
    l.resolve_references();
    l.check_multiplicities();
    let Loader { problems, objects, .. } = l;
    HutnLoad { resource, problems, objects }
}

struct PendingRef {
    object: ObjectId,
    feature: FeatureId,
    class: Option<Name>,
    package: Option<PackageId>,
    identifier: String,
    span: Range<usize>,
}

struct Loader<'a> {
    mm: &'a Metamodel,
    model: &'a mut Model,
    resource: usize,
    problems: Vec<Problem>,
    aliases: HashMap<String, PackageId>,
    ids: HashMap<String, ObjectId>,
    pending: Vec<PendingRef>,
    objects: Vec<HutnObject>,
}

impl Loader<'_> {
    fn error(&mut self, span: Range<usize>, message: impl Into<String>) {
        self.problems.push(Problem::error(span, message));
    }

    fn class_label(&self, o: ObjectId) -> String {
        let class = &self.mm.classifier(self.model.class_of(o)).name;
        match &self.model.object(o).xmi_id {
            Some(id) => format!("{class} {}", quote(id)),
            None => class.clone(),
        }
    }

    fn object(
        &mut self,
        b: &Block,
        package: Option<PackageId>,
        container: Option<(ObjectId, FeatureId)>,
    ) -> Option<ObjectId> {
        let mm = self.mm;
        let class = match resolve_class(mm, &self.aliases, package, &b.name.path) {
            Ok(c) => c,
            Err(e) => {
                self.error(b.name.span.clone(), e);
                return None;
            }
        };
        if let Some((_, f)) = container
            && let Some(ty) = mm.feature(f).ty
            && !mm.conforms_to(class, ty)
        {
            self.error(
                b.name.span.clone(),
                format!(
                    "'{}' is not a {}, so it cannot be a value of '{}'",
                    mm.classifier(class).name,
                    mm.classifier(ty).name,
                    mm.feature(f).name
                ),
            );
            return None;
        }
        if let Some(data) = mm.class_data(class)
            && (data.is_abstract || data.is_interface)
        {
            let what = if data.is_interface { "an interface" } else { "abstract" };
            self.error(
                b.name.span.clone(),
                format!("'{}' is {what} and cannot be instantiated", mm.classifier(class).name),
            );
        }
        let o = self.model.create_object(mm, class, self.resource, container);
        self.objects.push(HutnObject { object: o, span: b.span.clone(), name: b.name.span.clone() });
        if let Some((id, span)) = &b.identifier {
            if self.ids.contains_key(id) {
                self.error(span.clone(), format!("duplicate identifier {}", quote(id)));
            } else {
                self.ids.insert(id.clone(), o);
                self.model.set_xmi_id(o, id.clone());
            }
        }
        for a in &b.adjectives {
            match mm.lookup_feature(class, &a.name) {
                Some(f) if mm.feature(f).ty.and_then(|t| mm.data_type_kind(t)) == Some(DataTypeKind::Boolean) => {
                    self.model.add(mm, o, f, ModelValue::Boolean(!a.negated));
                }
                Some(_) => self.error(a.span.clone(), format!("adjective '{}' is not a Boolean attribute", a.name)),
                None => self.error(
                    a.span.clone(),
                    format!("class '{}' has no Boolean attribute '{}'", mm.classifier(class).name, a.name),
                ),
            }
        }
        let mut seen = std::collections::HashSet::new();
        for item in &b.items {
            match item {
                Item::Slot(s) => {
                    if let Some(f) = mm.lookup_feature(class, &s.feature)
                        && !seen.insert(f)
                        && !mm.feature(f).is_many()
                    {
                        self.error(s.feature_span.clone(), format!("feature '{}' is already set", s.feature));
                        continue;
                    }
                    self.slot(o, class, package, s);
                }
                Item::Block(nested) => self.error(
                    nested.name.span.clone(),
                    format!(
                        "a nested object must be the value of a containment feature, e.g. 'feature: {} {{ ... }}'",
                        nested.name.text()
                    ),
                ),
            }
        }
        Some(o)
    }

    fn slot(&mut self, o: ObjectId, class: ClassifierId, package: Option<PackageId>, s: &Slot) {
        let mm = self.mm;
        let Some(f) = mm.lookup_feature(class, &s.feature) else {
            self.error(
                s.feature_span.clone(),
                format!("class '{}' has no feature '{}'", mm.classifier(class).name, s.feature),
            );
            return;
        };
        let feature = mm.feature(f);
        if feature.derived || !feature.changeable {
            let why = if feature.derived { "derived" } else { "not changeable" };
            self.error(s.feature_span.clone(), format!("feature '{}' is {why} and cannot be set", s.feature));
            return;
        }
        if feature.opposite().is_some_and(|op| mm.feature(op).is_containment()) {
            self.error(
                s.feature_span.clone(),
                format!("'{}' refers to the container; it is set by nesting the object in its container", s.feature),
            );
            return;
        }
        if !feature.is_many() && s.values.len() > 1 {
            self.error(s.span.clone(), format!("feature '{}' takes a single value", s.feature));
        }
        for v in &s.values {
            self.value(o, f, package, v);
        }
    }

    fn value(&mut self, o: ObjectId, f: FeatureId, package: Option<PackageId>, v: &Value) {
        let mm = self.mm;
        let feature = mm.feature(f);
        let name = feature.name.clone();
        if matches!(v.kind, ValueKind::Null) {
            return;
        }
        if feature.is_containment() {
            match &v.kind {
                ValueKind::Object(b) => {
                    self.object(b, package, Some((o, f)));
                }
                ValueKind::Reference { class, .. } => self.error(
                    v.span.clone(),
                    format!(
                        "'{name}' is a containment feature: define the object here, e.g. '{} \"...\" {{ ... }}'",
                        class.text()
                    ),
                ),
                _ => self.error(v.span.clone(), format!("'{name}' expects a nested object, e.g. 'Class {{ ... }}'")),
            }
            return;
        }
        if feature.is_reference() {
            match &v.kind {
                ValueKind::Reference { class, identifier } => self.pending.push(PendingRef {
                    object: o,
                    feature: f,
                    class: Some(class.clone()),
                    package,
                    identifier: identifier.clone(),
                    span: v.span.clone(),
                }),
                ValueKind::Str(identifier) => self.pending.push(PendingRef {
                    object: o,
                    feature: f,
                    class: None,
                    package,
                    identifier: identifier.clone(),
                    span: v.span.clone(),
                }),
                ValueKind::Object(b) => self.error(
                    b.name.span.clone(),
                    format!(
                        "'{name}' is not a containment feature: define the object elsewhere and refer to it, e.g. '{} \"id\"'",
                        b.name.text()
                    ),
                ),
                _ => self.error(v.span.clone(), format!("'{name}' expects a reference such as 'Class \"identifier\"'")),
            }
            return;
        }
        let Some(ty) = feature.ty else {
            // Unresolved attribute type: keep the text.
            let text = match &v.kind {
                ValueKind::Str(s) => s.clone(),
                ValueKind::Int(i) => i.to_string(),
                ValueKind::Real(r) => r.to_string(),
                ValueKind::Bool(b) => b.to_string(),
                ValueKind::Name(n) => n.text(),
                _ => return self.error(v.span.clone(), format!("'{name}' expects a data value")),
            };
            self.model.add(mm, o, f, ModelValue::String(text));
            return;
        };
        match data_value(mm, ty, &v.kind) {
            Ok(value) => {
                if matches!(feature.kind, FeatureKind::Attribute { is_id: true })
                    && let ModelValue::String(s) = &value
                {
                    self.model.register_id(o, s.clone());
                }
                self.model.add(mm, o, f, value);
            }
            Err(e) => self.error(v.span.clone(), format!("'{name}': {e}")),
        }
    }

    fn resolve_references(&mut self) {
        let mm = self.mm;
        for p in std::mem::take(&mut self.pending) {
            let Some(&target) = self.ids.get(&p.identifier) else {
                let mut message = format!("no object with identifier {} in this document", quote(&p.identifier));
                if let Some(similar) = self.ids.keys().find(|k| k.eq_ignore_ascii_case(&p.identifier)) {
                    let _ = write!(message, " (did you mean {}?)", quote(similar));
                }
                self.error(p.span, message);
                continue;
            };
            let target_class = self.model.class_of(target);
            if let Some(class) = &p.class {
                match resolve_class(mm, &self.aliases, p.package, &class.path) {
                    Ok(c) if mm.conforms_to(target_class, c) => {}
                    Ok(c) => {
                        let label = self.class_label(target);
                        self.error(p.span, format!("{label} is not a {}", mm.classifier(c).name));
                        continue;
                    }
                    Err(e) => {
                        self.error(class.span.clone(), e);
                        continue;
                    }
                }
            }
            let feature = mm.feature(p.feature);
            if let Some(ty) = feature.ty
                && !mm.conforms_to(target_class, ty)
            {
                let label = self.class_label(target);
                self.error(
                    p.span,
                    format!(
                        "'{}' expects a {}, but {label} is a {}",
                        feature.name,
                        mm.classifier(ty).name,
                        mm.classifier(target_class).name
                    ),
                );
                continue;
            }
            self.model.add(mm, p.object, p.feature, ModelValue::Object(target));
        }
    }

    /// Reports features whose number of values is outside their bounds.
    fn check_multiplicities(&mut self) {
        let objects: Vec<(ObjectId, Range<usize>)> = self.objects.iter().map(|o| (o.object, o.name.clone())).collect();
        let problems = check_multiplicities(self.mm, self.model, &objects);
        self.problems.extend(problems);
    }
}

/// Problems with the number of values of the features of `objects` (each with the span the
/// problems are reported at): unset required features are warnings, too many values errors.
pub fn check_multiplicities(mm: &Metamodel, model: &Model, objects: &[(ObjectId, Range<usize>)]) -> Vec<Problem> {
    let label = |o: ObjectId| {
        let class = &mm.classifier(model.class_of(o)).name;
        match &model.object(o).xmi_id {
            Some(id) => format!("{class} {}", quote(id)),
            None => class.clone(),
        }
    };
    let mut problems = Vec::new();
    for (o, span) in objects {
        for &f in mm.all_features(model.class_of(*o)) {
            let feature = mm.feature(f);
            if feature.derived || !feature.changeable {
                continue;
            }
            let count = model.get(*o, f).len() as i32;
            let m = feature.multiplicity;
            if count < m.lower {
                problems.push(Problem::warning(
                    span.clone(),
                    if m.lower == 1 {
                        format!("required feature '{}' of {} is not set", feature.name, label(*o))
                    } else {
                        format!(
                            "feature '{}' of {} needs at least {} values, but has {count}",
                            feature.name,
                            label(*o),
                            m.lower
                        )
                    },
                ));
            }
            if m.upper > 1 && count > m.upper {
                problems.push(Problem::error(
                    span.clone(),
                    format!(
                        "feature '{}' of {} takes at most {} values, but has {count}",
                        feature.name,
                        label(*o),
                        m.upper
                    ),
                ));
            }
        }
    }
    problems
}

/// The package a top-level block names, if it is a package block (a bare name with a body).
pub fn package_block(mm: &Metamodel, aliases: &HashMap<String, PackageId>, b: &Block) -> Option<PackageId> {
    if !b.adjectives.is_empty() || b.identifier.is_some() || b.body.is_none() {
        return None;
    }
    resolve_package(mm, aliases, &b.name.path)
}

/// Resolves the name of a package block: a `@Spec` name, a (qualified) package name, or an
/// nsPrefix.
pub fn resolve_package(mm: &Metamodel, aliases: &HashMap<String, PackageId>, path: &[String]) -> Option<PackageId> {
    if let [single] = path
        && let Some(p) = aliases.get(single)
    {
        return Some(*p);
    }
    let names: Vec<&str> = path.iter().map(String::as_str).collect();
    let user = |p: &PackageId| mm.package(*p).ns_uri != ECORE_NS_URI;
    let mut found: Vec<PackageId> = mm.find_packages_qualified(&names);
    found.sort_by_key(|p| !user(p));
    if let Some(p) = found.first() {
        return Some(*p);
    }
    if let [single] = path {
        return mm.package_ids().filter(user).find(|p| mm.package(*p).ns_prefix == *single);
    }
    None
}

/// Resolves a class name: first in `package` (and its subpackages), then in all loaded
/// packages (user packages before Ecore).
pub fn resolve_class(
    mm: &Metamodel,
    aliases: &HashMap<String, PackageId>,
    package: Option<PackageId>,
    path: &[String],
) -> Result<ClassifierId, String> {
    let text = path.join("::");
    let check = |c: ClassifierId| {
        if mm.is_class(c) { Ok(c) } else { Err(format!("'{text}' is not a class")) }
    };
    if let [single] = path
        && let Some(p) = package
        && let Some(c) = find_in_tree(mm, p, single)
    {
        return check(c);
    }
    if let [pkg, name] = path
        && let Some(p) = aliases.get(pkg)
        && let Some(c) = mm.find_classifier_in(*p, name)
    {
        return check(c);
    }
    let names: Vec<&str> = path.iter().map(String::as_str).collect();
    let candidates = mm.find_classifiers_qualified(&names);
    let user: Vec<ClassifierId> =
        candidates.iter().copied().filter(|c| mm.classifier(*c).package != mm.ecore_package()).collect();
    match (user.as_slice(), candidates.first()) {
        ([c], _) => check(*c),
        ([], Some(c)) => check(*c),
        ([], None) => Err(format!("unknown class '{text}'")),
        (many, _) => Err(format!(
            "ambiguous class '{text}': could be {} (qualify it with its package)",
            many.iter().map(|c| mm.qualified_name(*c)).collect::<Vec<_>>().join(", ")
        )),
    }
}

fn find_in_tree(mm: &Metamodel, package: PackageId, name: &str) -> Option<ClassifierId> {
    mm.find_classifier_in(package, name)
        .or_else(|| mm.package(package).subpackages.iter().find_map(|s| find_in_tree(mm, *s, name)))
}

/// Converts a literal to a value of data type or enumeration `ty`.
fn data_value(mm: &Metamodel, ty: ClassifierId, v: &ValueKind) -> Result<ModelValue, String> {
    let type_name = &mm.classifier(ty).name;
    if mm.is_enum(ty) {
        let literal = match v {
            ValueKind::Name(n) if n.path.len() == 1 => &n.path[0],
            ValueKind::Name(n) if n.path.len() == 2 && n.path[0] == *type_name => &n.path[1],
            ValueKind::Str(s) => s,
            _ => return Err(format!("expected a literal of enumeration {type_name}")),
        };
        return parse_value(mm, ty, literal).map_err(|_| {
            let literals: Vec<&str> = mm.enum_literals(ty).iter().map(|l| l.name.as_str()).collect();
            format!("'{literal}' is not a literal of {type_name} (expected one of {})", literals.join(", "))
        });
    }
    let kind = mm.data_type_kind(ty).unwrap_or(DataTypeKind::Other);
    match (kind, v) {
        (DataTypeKind::Boolean, ValueKind::Bool(b)) => Ok(ModelValue::Boolean(*b)),
        (DataTypeKind::Integer, ValueKind::Int(i)) => Ok(ModelValue::Integer(*i)),
        (DataTypeKind::Real, ValueKind::Int(i)) => Ok(ModelValue::Real(*i as f64)),
        (DataTypeKind::Real, ValueKind::Real(r)) => Ok(ModelValue::Real(*r)),
        (DataTypeKind::String, ValueKind::Str(s)) => Ok(ModelValue::String(s.clone())),
        (DataTypeKind::Other, ValueKind::Str(s)) => Ok(ModelValue::String(s.clone())),
        (DataTypeKind::Other, ValueKind::Int(i)) => Ok(ModelValue::String(i.to_string())),
        (DataTypeKind::Other, ValueKind::Real(r)) => Ok(ModelValue::String(r.to_string())),
        (DataTypeKind::Other, ValueKind::Bool(b)) => Ok(ModelValue::String(b.to_string())),
        (DataTypeKind::Boolean, _) => Err("expected true or false".into()),
        (DataTypeKind::Integer, _) => Err(format!("expected an integer ({type_name})")),
        (DataTypeKind::Real, _) => Err(format!("expected a number ({type_name})")),
        (DataTypeKind::String, _) => Err("expected a string in double quotes".into()),
        (DataTypeKind::Other, _) => Err(format!("expected a {type_name} value")),
    }
}

// ------------------------------------------------------------------ writing

/// Writes one model resource as a HUTN document. Objects that other objects refer to get
/// identifiers (their `xmi:id`, else their name, else a generated one). References into other
/// resources cannot be expressed in HUTN and are written as comments.
pub fn write(mm: &Metamodel, model: &Model, resource: usize) -> String {
    let roots = &model.resources()[resource].roots;
    let mut objects = Vec::new();
    for r in roots {
        objects.push(*r);
        objects.extend(model.all_contents(mm, *r));
    }
    // Identifiers: objects with an xmi:id, and targets of references.
    let mut needs_id: Vec<ObjectId> = objects.iter().copied().filter(|o| model.object(*o).xmi_id.is_some()).collect();
    for o in &objects {
        for &f in mm.all_features(model.class_of(*o)) {
            let feature = mm.feature(f);
            if !feature.is_reference() || feature.is_containment() || !is_persistent(mm, f) {
                continue;
            }
            for v in model.get(*o, f) {
                if let ModelValue::Object(t) = v
                    && model.object(*t).resource == resource
                    && !needs_id.contains(t)
                {
                    needs_id.push(*t);
                }
            }
        }
    }
    let mut ids: HashMap<ObjectId, String> = HashMap::new();
    let mut taken: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut counters: HashMap<ClassifierId, usize> = HashMap::new();
    // Explicit ids first, so generated ones never collide with them.
    needs_id.sort_by_key(|o| model.object(*o).xmi_id.is_none());
    for o in needs_id {
        let candidate = model.object(o).xmi_id.clone().or_else(|| model.name_of(mm, o)).filter(|n| !n.is_empty());
        let id = match candidate {
            Some(c) if !taken.contains(&c) => c,
            _ => loop {
                let class = model.class_of(o);
                let n = counters.entry(class).or_insert(0);
                *n += 1;
                let name = &mm.classifier(class).name;
                let generated = format!("{}{n}", name[..1].to_lowercase() + &name[1..]);
                if !taken.contains(&generated) {
                    break generated;
                }
            },
        };
        taken.insert(id.clone());
        ids.insert(o, id);
    }

    let mut w = HutnWriter { mm, model, resource, ids, out: String::new() };
    // Root packages of the roots' classes, in order of first use.
    let mut packages: Vec<PackageId> = Vec::new();
    for r in roots {
        let p = mm.classifier(model.class_of(*r)).package;
        if !packages.contains(&p) {
            packages.push(p);
        }
    }
    if !packages.is_empty() {
        w.out.push_str("@Spec {\n");
        for p in &packages {
            let pkg = mm.package(*p);
            let _ = writeln!(w.out, "  metamodel {} {{\n    nsUri: {}\n  }}", quote(&pkg.name), quote(&pkg.ns_uri));
        }
        w.out.push_str("}\n");
    }
    let mut i = 0;
    while i < roots.len() {
        let package = mm.classifier(model.class_of(roots[i])).package;
        let _ = writeln!(w.out, "\n{} {{", mm.package(package).name);
        while i < roots.len() && mm.classifier(model.class_of(roots[i])).package == package {
            w.object(roots[i], Some(package), 1, true);
            i += 1;
        }
        w.out.push_str("}\n");
    }
    w.out
}

struct HutnWriter<'a> {
    mm: &'a Metamodel,
    model: &'a Model,
    resource: usize,
    ids: HashMap<ObjectId, String>,
    out: String,
}

impl HutnWriter<'_> {
    fn indent(&mut self, depth: usize) {
        for _ in 0..depth {
            self.out.push_str("  ");
        }
    }

    fn class_name(&self, class: ClassifierId, package: Option<PackageId>) -> String {
        let c = self.mm.classifier(class);
        if Some(c.package) == package { c.name.clone() } else { self.mm.qualified_name(class) }
    }

    fn header(&self, o: ObjectId, package: Option<PackageId>) -> String {
        let class = self.class_name(self.model.class_of(o), package);
        match self.ids.get(&o) {
            Some(id) => format!("{class} {}", quote(id)),
            None => class,
        }
    }

    /// Writes an object; `own_line` is false when it continues a `feature:` line.
    fn object(&mut self, o: ObjectId, package: Option<PackageId>, depth: usize, own_line: bool) {
        let mm = self.mm;
        let model = self.model;
        if own_line {
            self.indent(depth);
        }
        let header = self.header(o, package);
        self.out.push_str(&header);
        let features: Vec<FeatureId> = mm
            .all_features(model.class_of(o))
            .iter()
            .copied()
            .filter(|f| !model.get(o, *f).is_empty() && is_persistent(mm, *f))
            .collect();
        if features.is_empty() {
            self.out.push_str(" {}");
            if own_line {
                self.out.push('\n');
            }
            return;
        }
        self.out.push_str(" {\n");
        for f in features {
            let feature = mm.feature(f);
            let values = model.get(o, f);
            self.indent(depth + 1);
            let _ = write!(self.out, "{}:", feature.name);
            if feature.is_containment() {
                let children: Vec<ObjectId> = values
                    .iter()
                    .filter_map(|v| match v {
                        ModelValue::Object(c) => Some(*c),
                        _ => None,
                    })
                    .collect();
                if let [single] = children.as_slice() {
                    self.out.push(' ');
                    self.object(*single, package, depth + 1, false);
                    self.out.push('\n');
                } else {
                    self.out.push('\n');
                    for (i, c) in children.iter().enumerate() {
                        self.object(*c, package, depth + 2, true);
                        if i + 1 < children.len() {
                            // Turn the line break after the object into a separator.
                            self.out.pop();
                            self.out.push_str(",\n");
                        }
                    }
                }
                continue;
            }
            let mut rendered = Vec::new();
            let mut external = Vec::new();
            for v in values {
                match v {
                    ModelValue::Object(t) if model.object(*t).resource != self.resource => {
                        let res = &model.resources()[model.object(*t).resource].uri;
                        external.push(format!("{res}#{}", model.uri_fragment(mm, *t)));
                    }
                    ModelValue::Object(t) => rendered.push(self.header(*t, package)),
                    ModelValue::String(s) => rendered.push(quote(s)),
                    ModelValue::Enum(e, i) => {
                        rendered.push(mm.enum_literals(*e).get(*i).map(|l| l.name.clone()).unwrap_or_default())
                    }
                    other => rendered.push(crate::xmi_writer::value_text(mm, other)),
                }
            }
            if rendered.is_empty() {
                // Only references into other resources: keep the slot as a comment.
                self.out.pop();
                while self.out.ends_with(' ') {
                    self.out.pop();
                }
                let _ = write!(self.out, "// {}: (references into other resources)", feature.name);
            } else {
                let _ = write!(self.out, " {}", rendered.join(", "));
            }
            if !external.is_empty() {
                let _ = write!(self.out, " // not expressible in HUTN: {}", external.join(", "));
            }
            self.out.push('\n');
        }
        self.indent(depth);
        self.out.push('}');
        if own_line {
            self.out.push('\n');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_blocks_slots_and_values() {
        let (doc, problems) = parse(
            r#"@Spec { metamodel "lib" { nsUri: "http://x" } }
               lib {
                 // a comment
                 nice ~old Library "city" {
                   name: "City", count: 3 ratio: -1.5e2
                   books: Book "a" { genre: FICTION }, Book { }
                   favourite: Book "a"; tags: "x", "y"
                   flag: true other: null
                 }
               }"#,
        );
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(doc.spec[0].ns_uri.as_deref(), Some("http://x"));
        let lib = &doc.blocks[0];
        assert_eq!(lib.name.path, vec!["lib"]);
        let Item::Block(library) = &lib.items[0] else { panic!() };
        assert_eq!(library.adjectives.len(), 2);
        assert!(library.adjectives[1].negated);
        assert_eq!(library.identifier.as_ref().unwrap().0, "city");
        let slots: Vec<&Slot> = library
            .items
            .iter()
            .map(|i| match i {
                Item::Slot(s) => s,
                Item::Block(_) => panic!(),
            })
            .collect();
        let names: Vec<&str> = slots.iter().map(|s| s.feature.as_str()).collect();
        assert_eq!(names, ["name", "count", "ratio", "books", "favourite", "tags", "flag", "other"]);
        assert_eq!(slots[2].values[0].kind, ValueKind::Real(-150.0));
        assert!(matches!(&slots[3].values[0].kind, ValueKind::Object(b) if b.identifier.is_some()));
        assert!(matches!(&slots[3].values[1].kind, ValueKind::Object(b) if b.identifier.is_none()));
        assert!(matches!(&slots[4].values[0].kind, ValueKind::Reference { identifier, .. } if identifier == "a"));
        assert_eq!(slots[5].values.len(), 2);
        assert_eq!(slots[6].values[0].kind, ValueKind::Bool(true));
        assert_eq!(slots[7].values[0].kind, ValueKind::Null);
    }

    #[test]
    fn recovers_from_errors() {
        let (doc, problems) = parse("lib { Library \"x\" { name: } Book \"b\" { name: \"ok\" ");
        assert!(problems.len() >= 2, "{problems:?}");
        assert_eq!(doc.blocks.len(), 1);
        let (_, problems) = parse("lib { Library { name: \"unterminated\n } }");
        assert!(problems.iter().any(|p| p.message.contains("unterminated")), "{problems:?}");
    }
}
