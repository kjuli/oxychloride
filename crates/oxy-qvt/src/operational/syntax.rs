//! The syntax of QVT Operational Mappings (`.qvto`), after Eclipse QVTo's grammar
//! (`QVTOParser.gi`): model types, a transformation header, and mappings, helpers, queries,
//! `main` and properties — at the top level or in the braces of the transformation.
//!
//! The expressions are OCL with Imperative OCL, parsed by the OCL parser's QVT dialect on the
//! same token stream, so all spans are offsets into the document. Like the other parsers of
//! the engine, this one never fails: problems are reported and parsing resumes at the next
//! element.

use oxy_ocl::Diagnostic;
use oxy_ocl::Span;
use oxy_ocl::syntax::ast::{Expr, Ident, TypeExpr};
use oxy_ocl::syntax::lexer::{Keyword, TokenKind};
use oxy_ocl::syntax::{Dialect, Parser};

/// A `.qvto` document.
#[derive(Clone, Debug, Default)]
pub struct Document {
    pub modeltypes: Vec<ModelType>,
    pub transformation: Option<Header>,
    pub elements: Vec<Element>,
    /// `import` statements (other transformations and libraries are not supported).
    pub imports: Vec<Span>,
}

/// `modeltype UML uses 'http://...';`
#[derive(Clone, Debug)]
pub struct ModelType {
    pub name: Ident,
    /// `'strict'` or `'effective'`.
    pub compliance: Option<String>,
    pub packages: Vec<PackageRef>,
    pub span: Span,
}

/// A package of a model type: by nsURI or file (`'http://...'`), by name (`simpleuml`), or both
/// (`simpleuml('http://...')`).
#[derive(Clone, Debug)]
pub struct PackageRef {
    pub name: Option<Vec<Ident>>,
    pub uri: Option<(String, Span)>,
    pub span: Span,
}

/// `transformation Name(in uml : UML, out rdb : RDB)`.
#[derive(Clone, Debug)]
pub struct Header {
    pub name: Ident,
    pub params: Vec<ModelParam>,
    /// `extends` and `access` (not supported).
    pub uses: Vec<Ident>,
    pub span: Span,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    In,
    Out,
    InOut,
}

impl Direction {
    pub fn keyword(self) -> &'static str {
        match self {
            Direction::In => "in",
            Direction::Out => "out",
            Direction::InOut => "inout",
        }
    }
}

/// A model parameter of the transformation: `in uml : UML` or `out RDB`.
#[derive(Clone, Debug)]
pub struct ModelParam {
    pub direction: Direction,
    pub name: Option<Ident>,
    pub model_type: Ident,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Element {
    Mapping(Mapping),
    Helper(Helper),
    Main(Main),
    Property(Property),
}

/// A parameter of a mapping or helper: `[in|out|inout] [name :] T[@extent]`.
#[derive(Clone, Debug)]
pub struct Param {
    pub direction: Option<Direction>,
    pub name: Option<Ident>,
    pub ty: TypeExpr,
    pub extent: Option<Ident>,
    pub span: Span,
}

/// `mapping [inout] Ctx::name(params) : results [inherits|merges|disjuncts ...] [when {...}]
/// [where {...}] { [init {...}] [population {...} | ...] [end {...}] }`.
#[derive(Clone, Debug)]
pub struct Mapping {
    pub is_abstract: bool,
    pub direction: Option<Direction>,
    pub context: Option<TypeExpr>,
    pub name: Ident,
    pub params: Vec<Param>,
    pub results: Vec<Param>,
    pub inherits: Vec<Vec<Ident>>,
    pub merges: Vec<Vec<Ident>>,
    pub disjuncts: Vec<Vec<Ident>>,
    pub when: Option<Vec<Expr>>,
    pub where_: Option<Vec<Expr>>,
    pub init: Option<Vec<Expr>>,
    pub population: Vec<Expr>,
    /// Whether the population is written with the `population` keyword: then (as in Eclipse
    /// QVTo, after §8.2.1.19 of the standard) the result is not its implicit object.
    pub population_keyword: bool,
    pub end: Option<Vec<Expr>>,
    /// The braces of the body (`None` for a declaration ending with `;`).
    pub body: Option<Span>,
    pub span: Span,
}

/// `helper`/`query [Ctx::]name(params) : T` with a body `= e;` or `{ ... }`.
#[derive(Clone, Debug)]
pub struct Helper {
    pub is_query: bool,
    pub context: Option<TypeExpr>,
    pub name: Ident,
    pub params: Vec<Param>,
    pub results: Vec<Param>,
    pub body: HelperBody,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum HelperBody {
    /// Only declared (`;`).
    None,
    /// `= expression;`
    Expr(Expr),
    /// `{ statements }`
    Block(Vec<Expr>, Span),
}

/// `main() { ... }`.
#[derive(Clone, Debug)]
pub struct Main {
    /// Parameters bound to root objects of the input models (an old style of QVTo).
    pub params: Vec<Param>,
    pub body: Vec<Expr>,
    pub block: Span,
    pub span: Span,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PropertyKind {
    /// `property x : T = e;` of the transformation.
    Module,
    /// `configuration property x : T;`: set when the transformation is run.
    Configuration,
    /// `intermediate property Ctx::x : T;`: a property added to the objects of a class.
    Intermediate,
}

#[derive(Clone, Debug)]
pub struct Property {
    pub kind: PropertyKind,
    pub context: Option<TypeExpr>,
    pub name: Ident,
    pub ty: Option<TypeExpr>,
    pub init: Option<Expr>,
    pub span: Span,
}

/// Words that start an element (for recovering from errors).
const ELEMENT_WORDS: &[&str] = &[
    "mapping",
    "query",
    "helper",
    "main",
    "property",
    "configuration",
    "intermediate",
    "modeltype",
    "metamodel",
    "transformation",
    "library",
    "import",
    "abstract",
    "blackbox",
    "static",
    "constructor",
    "tag",
    "exception",
];

/// Parses a `.qvto` document.
pub fn parse(src: &str) -> (Document, Vec<Diagnostic>) {
    let mut p = QvtoParser { p: Parser::with_dialect(src, Dialect::Qvt) };
    let doc = p.document();
    (doc, p.p.into_diagnostics())
}

struct QvtoParser {
    p: Parser,
}

impl QvtoParser {
    fn at_element(&self) -> bool {
        let p = &self.p;
        match p.peek() {
            TokenKind::Ident(w) if ELEMENT_WORDS.contains(&w.as_str()) => match w.as_str() {
                "main" => matches!(p.peek_at(1), TokenKind::LParen),
                _ => matches!(p.peek_at(1), TokenKind::Ident(_) | TokenKind::QuotedIdent(_) | TokenKind::Str(_)),
            },
            _ => false,
        }
    }

    /// Skips to the next element (or a closing brace at the same depth).
    fn recover(&mut self) {
        let mut depth = 0usize;
        loop {
            match self.p.peek() {
                TokenKind::Eof => return,
                TokenKind::LBrace => depth += 1,
                TokenKind::RBrace => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                }
                _ if depth == 0 && self.at_element() => return,
                _ => {}
            }
            self.p.bump();
        }
    }

    fn document(&mut self) -> Document {
        let mut doc = Document::default();
        loop {
            while self.p.eat(&TokenKind::Semi) {}
            if self.p.at_eof() {
                return doc;
            }
            if self.p.at(&TokenKind::RBrace) {
                let span = self.p.span();
                self.p.error(span, "unexpected '}'");
                self.p.bump();
                continue;
            }
            self.element(&mut doc, true);
        }
    }

    /// One element of the document (`top`) or of the braces of the transformation.
    fn element(&mut self, doc: &mut Document, top: bool) {
        let start = self.p.span();
        let mut is_abstract = false;
        loop {
            if self.p.at_word("abstract") {
                is_abstract = true;
            } else if !(self.p.at_word("blackbox") || self.p.at_word("static")) {
                break;
            }
            self.p.bump();
        }
        let word = match self.p.peek() {
            TokenKind::Ident(w) => w.clone(),
            other => {
                let other = other.clone();
                self.p.error(start, format!("expected a mapping, query, helper, property or 'main', found {other}"));
                self.p.bump();
                self.recover();
                return;
            }
        };
        match word.as_str() {
            "import" => {
                self.p.bump();
                while !self.p.at_eof() && !self.p.at(&TokenKind::Semi) && !self.at_element() {
                    self.p.bump();
                }
                self.p.eat(&TokenKind::Semi);
                doc.imports.push(start.to(self.p.prev_span()));
            }
            "modeltype" | "metamodel" if top => {
                let m = self.modeltype();
                doc.modeltypes.push(m);
            }
            "transformation" if top => {
                if doc.transformation.is_some() {
                    self.p.error(start, "only one transformation per document is supported");
                }
                let header = self.header(start);
                let braced = self.p.at(&TokenKind::LBrace);
                if doc.transformation.is_none() {
                    doc.transformation = Some(header);
                }
                if braced {
                    self.p.bump();
                    loop {
                        while self.p.eat(&TokenKind::Semi) {}
                        if self.p.at_eof() {
                            self.p.error(start, "missing '}' at the end of the transformation");
                            break;
                        }
                        if self.p.eat(&TokenKind::RBrace) {
                            break;
                        }
                        self.element(doc, false);
                    }
                } else {
                    self.p.expect(&TokenKind::Semi, "after the transformation's signature");
                }
            }
            "library" => {
                self.p.bump();
                self.p.error(start, "libraries are not supported");
                self.recover();
                if self.p.at(&TokenKind::RBrace) {
                    self.p.bump();
                }
            }
            "mapping" => {
                let m = self.mapping(start, is_abstract);
                doc.elements.push(Element::Mapping(m));
            }
            "helper" | "query" => {
                let h = self.helper(start, word == "query");
                doc.elements.push(Element::Helper(h));
            }
            "main" => {
                self.p.bump();
                self.p.expect(&TokenKind::LParen, "after 'main'");
                let params = if self.p.at(&TokenKind::RParen) { Vec::new() } else { self.params() };
                self.p.expect(&TokenKind::RParen, "to close the parameters of 'main'");
                let block_start = self.p.span();
                let body = self.p.braced_statements("the body of 'main'");
                let block = block_start.to(self.p.prev_span());
                doc.elements.push(Element::Main(Main { params, body, block, span: start.to(self.p.prev_span()) }));
            }
            "property" | "configuration" => {
                let kind = if word == "configuration" {
                    self.p.bump();
                    PropertyKind::Configuration
                } else {
                    PropertyKind::Module
                };
                self.p.expect_word("property", "");
                let prop = self.property(start, kind);
                doc.elements.push(Element::Property(prop));
            }
            "intermediate" => {
                self.p.bump();
                if self.p.eat_word("property") {
                    let prop = self.property(start, PropertyKind::Intermediate);
                    doc.elements.push(Element::Property(prop));
                } else {
                    self.p.error(start.to(self.p.span()), "intermediate classes are not supported");
                    self.skip_definition();
                }
            }
            "constructor" | "tag" | "exception" | "typedef" | "rename" => {
                self.p.bump();
                self.p.error(start, format!("'{word}' is not supported"));
                self.skip_definition();
            }
            "modeltype" | "metamodel" | "transformation" => {
                self.p.error(start, format!("'{word}' must be at the top of the document"));
                self.p.bump();
                self.recover();
            }
            _ => {
                let t = self.p.peek_token().clone();
                self.p
                    .error(t.span, format!("expected a mapping, query, helper, property or 'main', found {}", t.kind));
                self.p.bump();
                self.recover();
            }
        }
    }

    /// Skips an unsupported definition: up to a `;` or past a braced body.
    fn skip_definition(&mut self) {
        loop {
            match self.p.peek() {
                TokenKind::Eof | TokenKind::RBrace => return,
                TokenKind::Semi => {
                    self.p.bump();
                    return;
                }
                TokenKind::LBrace => {
                    self.p.braced_statements("the definition");
                    self.p.eat(&TokenKind::Semi);
                    return;
                }
                _ if self.at_element() => return,
                _ => {
                    self.p.bump();
                }
            }
        }
    }

    fn modeltype(&mut self) -> ModelType {
        let start = self.p.bump().span;
        let name = self.p.ident("a model type name");
        let compliance = match self.p.peek().clone() {
            TokenKind::Str(s) => {
                self.p.bump();
                Some(s)
            }
            _ => None,
        };
        self.p.expect_word("uses", "after the model type's name");
        let mut packages = Vec::new();
        loop {
            let s = self.p.span();
            let package = match self.p.peek().clone() {
                TokenKind::Str(uri) => {
                    self.p.bump();
                    PackageRef { name: None, uri: Some((uri, s)), span: s }
                }
                _ => {
                    let mut path = vec![self.p.ident("a package name or URI")];
                    while self.p.eat(&TokenKind::ColonColon) || self.p.eat(&TokenKind::Dot) {
                        path.push(self.p.ident("a package name"));
                    }
                    let uri = if self.p.eat(&TokenKind::LParen) {
                        let us = self.p.span();
                        let uri = match self.p.peek().clone() {
                            TokenKind::Str(u) => {
                                self.p.bump();
                                Some((u, us))
                            }
                            other => {
                                self.p.error(us, format!("expected the package's URI as a string, found {other}"));
                                None
                            }
                        };
                        self.p.expect(&TokenKind::RParen, "after the URI");
                        uri
                    } else {
                        None
                    };
                    PackageRef { name: Some(path), uri, span: s.to(self.p.prev_span()) }
                }
            };
            packages.push(package);
            if !self.p.eat(&TokenKind::Comma) {
                break;
            }
        }
        if self.p.at_word("where") && matches!(self.p.peek_at(1), TokenKind::LBrace) {
            let s = self.p.bump().span;
            self.p.braced_statements("the conditions of the model type");
            self.p.warning(s.to(self.p.prev_span()), "the conditions of model types are not checked");
        }
        self.p.expect(&TokenKind::Semi, "after the model type");
        ModelType { name, compliance, packages, span: start.to(self.p.prev_span()) }
    }

    fn header(&mut self, start: Span) -> Header {
        self.p.bump();
        let mut name = self.p.ident("the transformation's name");
        while self.p.eat(&TokenKind::Dot) {
            name = self.p.ident("the transformation's name");
        }
        let mut params = Vec::new();
        if self.p.eat(&TokenKind::LParen) {
            if !self.p.at(&TokenKind::RParen) {
                loop {
                    let s = self.p.span();
                    let direction = self.direction().unwrap_or(Direction::In);
                    let (name, model_type) = if self.p.at_ident() && matches!(self.p.peek_at(1), TokenKind::Colon) {
                        let n = self.p.ident("a parameter name");
                        self.p.bump();
                        (Some(n), self.p.ident("a model type"))
                    } else {
                        (None, self.p.ident("a model type"))
                    };
                    params.push(ModelParam { direction, name, model_type, span: s.to(self.p.prev_span()) });
                    if !self.p.eat(&TokenKind::Comma) {
                        break;
                    }
                }
            }
            self.p.expect(&TokenKind::RParen, "to close the transformation's parameters");
        }
        let mut uses = Vec::new();
        while self.p.at_word("extends") || self.p.at_word("access") || self.p.at_word("refines") {
            let s = self.p.bump().span;
            if self.p.at_word("transformation") || self.p.at_word("library") {
                self.p.bump();
            }
            loop {
                uses.push(self.p.ident("a transformation or library"));
                if self.p.eat(&TokenKind::LParen) {
                    let mut depth = 0usize;
                    while !self.p.at_eof() {
                        match self.p.bump().kind {
                            TokenKind::LParen => depth += 1,
                            TokenKind::RParen if depth == 0 => break,
                            TokenKind::RParen => depth -= 1,
                            _ => {}
                        }
                    }
                }
                if !self.p.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.p.error(s.to(self.p.prev_span()), "extending or accessing other transformations is not supported");
        }
        Header { name, params, uses, span: start.to(self.p.prev_span()) }
    }

    fn direction(&mut self) -> Option<Direction> {
        let d = match self.p.peek() {
            TokenKind::Keyword(Keyword::In) => Direction::In,
            TokenKind::Ident(w) if w == "out" => Direction::Out,
            TokenKind::Ident(w) if w == "inout" => Direction::InOut,
            _ => return None,
        };
        // `out` and `inout` may be parameter names: `out : T`.
        if matches!(self.p.peek_at(1), TokenKind::Colon | TokenKind::Comma | TokenKind::RParen) {
            return None;
        }
        self.p.bump();
        Some(d)
    }

    /// `[dir] [name :] T[@extent]`, separated by commas.
    fn params(&mut self) -> Vec<Param> {
        let mut out = Vec::new();
        loop {
            let s = self.p.span();
            let direction = self.direction();
            let name = if self.p.at_ident() && matches!(self.p.peek_at(1), TokenKind::Colon) {
                let n = self.p.ident("a parameter name");
                self.p.bump();
                Some(n)
            } else {
                None
            };
            let ty = self.p.type_expr();
            let extent = if self.p.eat(&TokenKind::At) { Some(self.p.ident("a model parameter")) } else { None };
            out.push(Param { direction, name, ty, extent, span: s.to(self.p.prev_span()) });
            if !self.p.eat(&TokenKind::Comma) {
                return out;
            }
        }
    }

    /// `Ctx::name` or `name`: the context type and the name.
    fn scoped_name(&mut self, what: &str) -> (Option<TypeExpr>, Ident) {
        let start = self.p.span();
        if let TokenKind::Ident(w) = self.p.peek().clone()
            && matches!(w.as_str(), "Set" | "OrderedSet" | "Bag" | "Sequence" | "Collection" | "List" | "Tuple")
            && matches!(self.p.peek_at(1), TokenKind::LParen)
        {
            let t = self.p.type_expr();
            self.p.expect(&TokenKind::ColonColon, &format!("before the {what}'s name"));
            return (Some(t), self.p.ident(what));
        }
        let mut path = vec![self.p.ident(what)];
        while self.p.eat(&TokenKind::ColonColon) {
            path.push(self.p.ident(what));
        }
        let name = path.pop().unwrap_or_else(|| Ident { name: String::new(), span: start });
        if path.is_empty() {
            return (None, name);
        }
        let span = path[0].span.to(path[path.len() - 1].span);
        (Some(TypeExpr { kind: oxy_ocl::syntax::ast::TypeExprKind::Path(path), span }), name)
    }

    /// `(params) [: results]`.
    fn signature(&mut self) -> (Vec<Param>, Vec<Param>) {
        let mut params = Vec::new();
        if self.p.expect(&TokenKind::LParen, "to start the parameters") {
            if !self.p.at(&TokenKind::RParen) {
                params = self.params();
            }
            self.p.expect(&TokenKind::RParen, "to close the parameters");
        }
        let results = if self.p.eat(&TokenKind::Colon) { self.params() } else { Vec::new() };
        (params, results)
    }

    fn mapping_refs(&mut self) -> Vec<Vec<Ident>> {
        let mut out = Vec::new();
        loop {
            let mut path = vec![self.p.ident("a mapping")];
            while self.p.eat(&TokenKind::ColonColon) {
                path.push(self.p.ident("a mapping"));
            }
            out.push(path);
            if !self.p.eat(&TokenKind::Comma) {
                return out;
            }
        }
    }

    fn mapping(&mut self, start: Span, is_abstract: bool) -> Mapping {
        self.p.bump();
        let direction = self.direction();
        let (context, name) = self.scoped_name("mapping");
        let (params, results) = self.signature();
        let mut m = Mapping {
            is_abstract,
            direction,
            context,
            name,
            params,
            results,
            inherits: Vec::new(),
            merges: Vec::new(),
            disjuncts: Vec::new(),
            when: None,
            where_: None,
            init: None,
            population: Vec::new(),
            population_keyword: false,
            end: None,
            body: None,
            span: start,
        };
        // `inherits`, `merges` and `disjuncts` come before `when` and `where`; any order is
        // accepted.
        loop {
            if self.p.eat_word("inherits") {
                m.inherits.extend(self.mapping_refs());
            } else if self.p.eat_word("merges") {
                m.merges.extend(self.mapping_refs());
            } else if self.p.eat_word("disjuncts") {
                m.disjuncts.extend(self.mapping_refs());
            } else if self.p.at_word("refines") {
                let s = self.p.bump().span;
                self.mapping_refs();
                self.p.error(s.to(self.p.prev_span()), "'refines' is not supported");
            } else if self.p.at_word("when") && matches!(self.p.peek_at(1), TokenKind::LBrace) {
                self.p.bump();
                m.when = Some(self.p.braced_statements("the guard"));
            } else if self.p.at_word("where") && matches!(self.p.peek_at(1), TokenKind::LBrace) {
                self.p.bump();
                m.where_ = Some(self.p.braced_statements("the postcondition"));
            } else {
                break;
            }
        }
        if self.p.eat(&TokenKind::Semi) {
            m.span = start.to(self.p.prev_span());
            return m;
        }
        let body_start = self.p.span();
        if self.p.expect(&TokenKind::LBrace, "to start the mapping's body (or ';')") {
            if self.p.at_word("init") && matches!(self.p.peek_at(1), TokenKind::LBrace) {
                self.p.bump();
                m.init = Some(self.p.braced_statements("the init section"));
            }
            if self.p.at_word("population") && matches!(self.p.peek_at(1), TokenKind::LBrace) {
                self.p.bump();
                m.population = self.p.braced_statements("the population section");
                m.population_keyword = true;
            } else {
                m.population = self.p.statements();
            }
            if self.p.at_word("end") && matches!(self.p.peek_at(1), TokenKind::LBrace) {
                self.p.bump();
                m.end = Some(self.p.braced_statements("the end section"));
            }
            if !self.p.eat(&TokenKind::RBrace) {
                let t = self.p.peek_token().clone();
                self.p.error(t.span, format!("expected '}}' to close the mapping, found {}", t.kind));
                self.recover();
                self.p.eat(&TokenKind::RBrace);
            }
            m.body = Some(body_start.to(self.p.prev_span()));
        } else {
            self.recover();
        }
        m.span = start.to(self.p.prev_span());
        m
    }

    fn helper(&mut self, start: Span, is_query: bool) -> Helper {
        self.p.bump();
        let (context, name) = self.scoped_name(if is_query { "query" } else { "helper" });
        let (params, results) = self.signature();
        let body = if self.p.eat(&TokenKind::Eq) {
            let e = self.p.expr();
            self.p.expect(&TokenKind::Semi, "after the body");
            HelperBody::Expr(e)
        } else if self.p.at(&TokenKind::LBrace) {
            let s = self.p.span();
            let items = self.p.braced_statements("the body");
            HelperBody::Block(items, s.to(self.p.prev_span()))
        } else {
            self.p.expect(&TokenKind::Semi, "or a body after the signature");
            HelperBody::None
        };
        Helper { is_query, context, name, params, results, body, span: start.to(self.p.prev_span()) }
    }

    fn property(&mut self, start: Span, kind: PropertyKind) -> Property {
        let (context, name) = if kind == PropertyKind::Intermediate {
            self.scoped_name("property")
        } else {
            (None, self.p.ident("a property name"))
        };
        let ty = if self.p.eat(&TokenKind::Colon) { Some(self.p.type_expr()) } else { None };
        let init =
            if self.p.eat(&TokenKind::Eq) || self.p.eat(&TokenKind::ColonEq) { Some(self.p.expr()) } else { None };
        self.p.expect(&TokenKind::Semi, "after the property");
        if ty.is_none() && init.is_none() {
            self.p.error(name.span, format!("the property '{}' needs a type", name.name));
        }
        if kind == PropertyKind::Intermediate && context.is_none() {
            self.p.error(name.span, "an intermediate property needs a class (Class::name)");
        }
        Property { kind, context, name, ty, init, span: start.to(self.p.prev_span()) }
    }
}
