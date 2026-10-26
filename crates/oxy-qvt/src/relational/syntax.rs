//! The syntax of QVT Relations (`.qvtr`), after Eclipse QVTd's grammar (`QVTrelation.xtext`):
//! metamodel imports, and transformations with keys, relations (variables, domains with
//! object and collection templates, `when` and `where` clauses) and queries — at the top level
//! or in `package` blocks.
//!
//! Expressions are parsed by the OCL parser's QVT dialect on the same token stream (spans are
//! document offsets); the parser never fails and resumes at the next element after an error.

use oxy_ocl::syntax::ast::{Expr, ExprKind, Ident, TypeExpr, TypeExprKind};
use oxy_ocl::syntax::lexer::TokenKind;
use oxy_ocl::syntax::{Dialect, Parser};
use oxy_ocl::{Diagnostic, Span};

#[derive(Clone, Debug, Default)]
pub struct Document {
    pub imports: Vec<Import>,
    pub transformations: Vec<Transformation>,
}

/// `import alias : 'uri'[::package...];`
#[derive(Clone, Debug)]
pub struct Import {
    pub alias: Option<Ident>,
    pub uri: String,
    pub uri_span: Span,
    /// Packages inside the imported resource (`'file.ecore'::pkg::sub`).
    pub path: Vec<Ident>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Transformation {
    pub name: Ident,
    pub params: Vec<ModelDecl>,
    pub keys: Vec<Key>,
    pub relations: Vec<Relation>,
    pub queries: Vec<Query>,
    pub span: Span,
}

/// `uml : SimpleUML` or `uml : { a, b }`.
#[derive(Clone, Debug)]
pub struct ModelDecl {
    pub name: Ident,
    pub metamodels: Vec<Ident>,
    pub span: Span,
}

/// `key Class { property, opposite(Other.property) };`
#[derive(Clone, Debug)]
pub struct Key {
    pub class: TypeExpr,
    pub properties: Vec<Ident>,
    pub opposites: Vec<(TypeExpr, Ident)>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Relation {
    pub is_top: bool,
    pub is_abstract: bool,
    pub name: Ident,
    pub overrides: Option<Ident>,
    pub variables: Vec<VarDecl>,
    pub domains: Vec<Domain>,
    pub when: Option<Vec<Expr>>,
    pub where_: Option<Vec<Expr>>,
    pub span: Span,
}

/// `a, b : T [= init];`
#[derive(Clone, Debug)]
pub struct VarDecl {
    pub names: Vec<Ident>,
    pub ty: Option<TypeExpr>,
    pub init: Option<Expr>,
    pub span: Span,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DomainKind {
    Checkonly,
    Enforce,
    /// `replace` (treated like `enforce`).
    Replace,
    /// Neither: only checked.
    Plain,
    /// `primitive domain x : T`: a parameter of the relation.
    Primitive,
}

#[derive(Clone, Debug)]
pub struct Domain {
    pub kind: DomainKind,
    /// The model parameter (`None` for primitive domains).
    pub model: Option<Ident>,
    pub patterns: Vec<Template>,
    /// The variables of a primitive domain.
    pub primitives: Vec<(Ident, TypeExpr)>,
    /// `default_values { x = e; }`: values of unbound variables when the domain is enforced.
    pub default_values: Vec<(Ident, Expr)>,
    pub span: Span,
}

impl Domain {
    pub fn is_enforceable(&self) -> bool {
        matches!(self.kind, DomainKind::Enforce | DomainKind::Replace)
    }
}

#[derive(Clone, Debug)]
pub enum Template {
    Object(ObjectTemplate),
    Collection(CollectionTemplate),
}

impl Template {
    pub fn var(&self) -> Option<&Ident> {
        match self {
            Template::Object(o) => o.var.as_ref(),
            Template::Collection(c) => c.var.as_ref(),
        }
    }

    pub fn ty(&self) -> &TypeExpr {
        match self {
            Template::Object(o) => &o.ty,
            Template::Collection(c) => &c.ty,
        }
    }

    pub fn span(&self) -> Span {
        match self {
            Template::Object(o) => o.span,
            Template::Collection(c) => c.span,
        }
    }

    pub fn guard(&self) -> Option<&Expr> {
        match self {
            Template::Object(o) => o.guard.as_ref(),
            Template::Collection(c) => c.guard.as_ref(),
        }
    }
}

/// `v : T { property = value, ... } [{ guard }]`.
#[derive(Clone, Debug)]
pub struct ObjectTemplate {
    pub var: Option<Ident>,
    pub ty: TypeExpr,
    pub properties: Vec<PropertyTemplate>,
    pub guard: Option<Expr>,
    pub span: Span,
}

/// `property = value` or `opposite(Class.property) = value`.
#[derive(Clone, Debug)]
pub struct PropertyTemplate {
    pub property: Ident,
    /// For `opposite(Class.property)`: the class.
    pub opposite: Option<TypeExpr>,
    pub value: TemplateValue,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum TemplateValue {
    Expr(Expr),
    Template(Template),
}

/// `v : Set(T) { member, ... ++ rest }`.
#[derive(Clone, Debug)]
pub struct CollectionTemplate {
    pub var: Option<Ident>,
    pub ty: TypeExpr,
    pub members: Vec<Member>,
    /// The rest of the collection (`_`: ignored).
    pub rest: Option<Ident>,
    pub guard: Option<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Member {
    Var(Ident),
    Template(Box<Template>),
}

/// `query name(params) : T { expression }`.
#[derive(Clone, Debug)]
pub struct Query {
    pub name: Ident,
    pub params: Vec<(Ident, TypeExpr)>,
    pub ty: TypeExpr,
    pub body: Option<Expr>,
    pub span: Span,
}

/// Parses a `.qvtr` document.
pub fn parse(src: &str) -> (Document, Vec<Diagnostic>) {
    let mut p = QvtrParser { p: Parser::with_dialect(src, Dialect::Qvt) };
    let doc = p.document();
    (doc, p.p.into_diagnostics())
}

struct QvtrParser {
    p: Parser,
}

/// Words that start an element of a transformation.
const ELEMENT_WORDS: &[&str] =
    &["top", "abstract", "relation", "query", "transient", "key", "target", "attribute", "property", "operation"];

impl QvtrParser {
    fn at_element(&self) -> bool {
        matches!(self.p.peek(), TokenKind::Ident(w) if ELEMENT_WORDS.contains(&w.as_str()) || w == "transformation" || w == "import" || w == "package")
            && matches!(self.p.peek_at(1), TokenKind::Ident(_) | TokenKind::QuotedIdent(_) | TokenKind::Str(_))
    }

    /// Skips to the next element, or to a `}` that closes the enclosing block.
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
        self.elements(&mut doc, false);
        doc
    }

    /// Imports, packages and transformations (up to a `}` when `nested`).
    fn elements(&mut self, doc: &mut Document, nested: bool) {
        loop {
            while self.p.eat(&TokenKind::Semi) {}
            if self.p.at_eof() || (nested && self.p.at(&TokenKind::RBrace)) {
                return;
            }
            if self.p.at_word("import") {
                let i = self.import();
                doc.imports.push(i);
            } else if self.p.at_word("transformation") {
                let t = self.transformation();
                doc.transformations.push(t);
            } else if self.p.at_word("package") {
                let start = self.p.bump().span;
                while !self.p.at_eof() && !self.p.at(&TokenKind::LBrace) && !self.p.at(&TokenKind::Semi) {
                    self.p.bump();
                }
                if self.p.eat(&TokenKind::LBrace) {
                    self.elements(doc, true);
                    if !self.p.eat(&TokenKind::RBrace) {
                        self.p.error(start, "missing '}' at the end of the package");
                    }
                }
            } else {
                let t = self.p.peek_token().clone();
                self.p.error(t.span, format!("expected 'import', 'package' or 'transformation', found {}", t.kind));
                self.p.bump();
                self.recover();
                if !nested && self.p.at(&TokenKind::RBrace) {
                    self.p.bump();
                }
            }
        }
    }

    fn import(&mut self) -> Import {
        let start = self.p.bump().span;
        let alias = if self.p.at_ident() && matches!(self.p.peek_at(1), TokenKind::Colon) {
            let a = self.p.ident("an alias");
            self.p.bump();
            Some(a)
        } else {
            None
        };
        let uri_span = self.p.span();
        let uri = match self.p.peek().clone() {
            TokenKind::Str(s) => {
                self.p.bump();
                s
            }
            other => {
                self.p.error(uri_span, format!("expected the metamodel's URI as a string, found {other}"));
                String::new()
            }
        };
        let mut path = Vec::new();
        while self.p.eat(&TokenKind::ColonColon) {
            if self.p.eat(&TokenKind::Star) {
                break;
            }
            path.push(self.p.ident("a package name"));
        }
        self.p.expect(&TokenKind::Semi, "after the import");
        Import { alias, uri, uri_span, path, span: start.to(self.p.prev_span()) }
    }

    fn transformation(&mut self) -> Transformation {
        let start = self.p.bump().span;
        let mut name = self.p.ident("the transformation's name");
        while self.p.eat(&TokenKind::ColonColon) {
            name = self.p.ident("the transformation's name");
        }
        let mut params = Vec::new();
        if self.p.expect(&TokenKind::LParen, "to start the transformation's parameters") {
            loop {
                let s = self.p.span();
                let pname = self.p.ident("a model parameter");
                self.p.expect(&TokenKind::Colon, "after the parameter's name");
                let mut metamodels = Vec::new();
                if self.p.eat(&TokenKind::LBrace) {
                    loop {
                        metamodels.push(self.p.ident("a metamodel"));
                        if !self.p.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                    self.p.expect(&TokenKind::RBrace, "after the metamodels");
                } else {
                    metamodels.push(self.p.ident("a metamodel"));
                }
                for word in ["dependsOn", "iterates"] {
                    if self.p.at_word(word) {
                        let w = self.p.bump().span;
                        if self.p.eat(&TokenKind::LBrace) {
                            while !self.p.at_eof() && !self.p.eat(&TokenKind::RBrace) {
                                self.p.bump();
                            }
                        } else {
                            self.p.ident("a model parameter");
                        }
                        self.p.warning(w.to(self.p.prev_span()), format!("'{word}' is ignored"));
                    }
                }
                params.push(ModelDecl { name: pname, metamodels, span: s.to(self.p.prev_span()) });
                if !self.p.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.p.expect(&TokenKind::RParen, "to close the transformation's parameters");
        }
        if self.p.at_word("extends") {
            let s = self.p.bump().span;
            loop {
                self.p.ident("a transformation");
                if !self.p.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.p.error(s.to(self.p.prev_span()), "extending other transformations is not supported");
        }
        let mut t =
            Transformation { name, params, keys: Vec::new(), relations: Vec::new(), queries: Vec::new(), span: start };
        if !self.p.expect(&TokenKind::LBrace, "to start the transformation's body") {
            self.recover();
        }
        loop {
            while self.p.eat(&TokenKind::Semi) {}
            if self.p.at_eof() {
                self.p.error(start, "missing '}' at the end of the transformation");
                break;
            }
            if self.p.eat(&TokenKind::RBrace) {
                break;
            }
            let s = self.p.span();
            if self.p.at_word("key") {
                let k = self.key();
                t.keys.push(k);
            } else if self.p.at_word("top") || self.p.at_word("abstract") || self.p.at_word("relation") {
                let r = self.relation();
                t.relations.push(r);
            } else if self.p.at_word("query") || self.p.at_word("transient") {
                let q = self.query();
                t.queries.push(q);
            } else if self.p.at_word("target") {
                self.p.bump();
                self.p.ident("a target name");
                if self.p.at(&TokenKind::LBrace) {
                    self.skip_braces();
                }
                self.p.eat(&TokenKind::Semi);
            } else if self.p.at_word("attribute") || self.p.at_word("property") || self.p.at_word("operation") {
                self.p.error(s, "properties and operations of transformations are not supported");
                self.p.bump();
                self.skip_definition();
            } else {
                let tk = self.p.peek_token().clone();
                self.p.error(tk.span, format!("expected a relation, query or key, found {}", tk.kind));
                self.p.bump();
                self.recover();
            }
        }
        t.span = start.to(self.p.prev_span());
        t
    }

    fn skip_braces(&mut self) {
        let mut depth = 0usize;
        while !self.p.at_eof() {
            match self.p.bump().kind {
                TokenKind::LBrace => depth += 1,
                TokenKind::RBrace => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    fn skip_definition(&mut self) {
        loop {
            match self.p.peek() {
                TokenKind::Eof | TokenKind::RBrace => return,
                TokenKind::Semi => {
                    self.p.bump();
                    return;
                }
                TokenKind::LBrace => {
                    self.skip_braces();
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

    /// `A::B` (a type path).
    fn path_type(&mut self) -> TypeExpr {
        let start = self.p.span();
        let mut path = vec![self.p.ident("a class")];
        while self.p.eat(&TokenKind::ColonColon) {
            path.push(self.p.ident("a class"));
        }
        TypeExpr { kind: TypeExprKind::Path(path), span: start.to(self.p.prev_span()) }
    }

    fn key(&mut self) -> Key {
        let start = self.p.bump().span;
        let class = self.path_type();
        let mut properties = Vec::new();
        let mut opposites = Vec::new();
        if self.p.expect(&TokenKind::LBrace, "to start the key's properties") {
            loop {
                if self.p.eat_word("opposite") {
                    self.p.expect(&TokenKind::LParen, "after 'opposite'");
                    let (c, prop) = self.opposite_path();
                    opposites.push((c, prop));
                    self.p.expect(&TokenKind::RParen, "to close 'opposite'");
                } else {
                    properties.push(self.p.ident("a property"));
                }
                if !self.p.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.p.expect(&TokenKind::RBrace, "to close the key's properties");
        }
        self.p.expect(&TokenKind::Semi, "after the key");
        Key { class, properties, opposites, span: start.to(self.p.prev_span()) }
    }

    /// `Class.property` (or `pkg::Class.property`) inside `opposite(...)`.
    fn opposite_path(&mut self) -> (TypeExpr, Ident) {
        let start = self.p.span();
        let mut path = vec![self.p.ident("a class")];
        while self.p.eat(&TokenKind::ColonColon) {
            path.push(self.p.ident("a class"));
        }
        let property = if self.p.eat(&TokenKind::Dot) {
            self.p.ident("a property")
        } else {
            // `pkg::Class::property`.
            let last = path.pop();
            last.unwrap_or_else(|| Ident { name: String::new(), span: start })
        };
        let class = TypeExpr { kind: TypeExprKind::Path(path), span: start.to(self.p.prev_span()) };
        (class, property)
    }

    fn query(&mut self) -> Query {
        let start = self.p.span();
        self.p.eat_word("transient");
        self.p.expect_word("query", "");
        let name = self.p.ident("the query's name");
        let mut params = Vec::new();
        if self.p.expect(&TokenKind::LParen, "to start the parameters") {
            if !self.p.at(&TokenKind::RParen) {
                loop {
                    let n = self.p.ident("a parameter name");
                    self.p.expect(&TokenKind::Colon, "after the parameter's name");
                    let t = self.p.type_expr();
                    params.push((n, t));
                    if !self.p.eat(&TokenKind::Comma) {
                        break;
                    }
                }
            }
            self.p.expect(&TokenKind::RParen, "to close the parameters");
        }
        self.p.expect(&TokenKind::Colon, "before the query's type");
        let ty = self.p.type_expr();
        let body = if self.p.eat(&TokenKind::LBrace) {
            let e = self.p.expr();
            self.p.expect(&TokenKind::RBrace, "to close the query's body");
            Some(e)
        } else {
            if self.p.at_word("implementedby") {
                let s = self.p.bump().span;
                self.p.bump();
                self.p.error(s.to(self.p.prev_span()), "queries implemented in Java are not supported");
            }
            self.p.expect(&TokenKind::Semi, "or a body after the query's type");
            None
        };
        Query { name, params, ty, body, span: start.to(self.p.prev_span()) }
    }

    fn relation(&mut self) -> Relation {
        let start = self.p.span();
        let mut is_top = false;
        let mut is_abstract = false;
        loop {
            if self.p.eat_word("top") {
                is_top = true;
            } else if self.p.eat_word("abstract") {
                is_abstract = true;
            } else {
                break;
            }
        }
        self.p.expect_word("relation", "");
        let name = self.p.ident("the relation's name");
        let overrides = if self.p.eat_word("overrides") { Some(self.p.ident("a relation")) } else { None };
        let mut r = Relation {
            is_top,
            is_abstract,
            name,
            overrides,
            variables: Vec::new(),
            domains: Vec::new(),
            when: None,
            where_: None,
            span: start,
        };
        if !self.p.expect(&TokenKind::LBrace, "to start the relation's body") {
            self.recover();
            r.span = start.to(self.p.prev_span());
            return r;
        }
        loop {
            while self.p.eat(&TokenKind::Semi) {}
            if self.p.at_eof() || self.p.eat(&TokenKind::RBrace) {
                break;
            }
            let s = self.p.span();
            let kind = if self.p.at_word("checkonly") {
                Some(DomainKind::Checkonly)
            } else if self.p.at_word("enforce") {
                Some(DomainKind::Enforce)
            } else if self.p.at_word("replace") {
                Some(DomainKind::Replace)
            } else if self.p.at_word("domain") && !matches!(self.p.peek_at(1), TokenKind::Colon | TokenKind::Comma) {
                Some(DomainKind::Plain)
            } else if self.p.at_word("primitive") && self.p.at_word_ahead(1, "domain") {
                Some(DomainKind::Primitive)
            } else {
                None
            };
            if let Some(kind) = kind {
                let d = self.domain(kind, s);
                r.domains.push(d);
            } else if self.p.at_word("when") && matches!(self.p.peek_at(1), TokenKind::LBrace) {
                self.p.bump();
                r.when = Some(self.predicates("the when clause"));
            } else if self.p.at_word("where") && matches!(self.p.peek_at(1), TokenKind::LBrace) {
                self.p.bump();
                r.where_ = Some(self.predicates("the where clause"));
            } else if self.p.at_ident() {
                let v = self.var_decl();
                r.variables.push(v);
            } else {
                let t = self.p.peek_token().clone();
                self.p.error(t.span, format!("expected a variable, a domain, 'when' or 'where', found {}", t.kind));
                self.p.bump();
                self.recover();
                if self.p.at(&TokenKind::RBrace) {
                    self.p.bump();
                    break;
                }
            }
        }
        r.span = start.to(self.p.prev_span());
        r
    }

    fn var_decl(&mut self) -> VarDecl {
        let start = self.p.span();
        let mut names = vec![self.p.ident("a variable")];
        while self.p.eat(&TokenKind::Comma) {
            names.push(self.p.ident("a variable"));
        }
        let ty = if self.p.eat(&TokenKind::Colon) { Some(self.p.type_expr()) } else { None };
        let init = if self.p.eat(&TokenKind::Eq) { Some(self.p.expr()) } else { None };
        if ty.is_none() && init.is_none() {
            self.p.error(start.to(self.p.prev_span()), "a variable needs a type");
        }
        self.p.expect(&TokenKind::Semi, "after the variable");
        VarDecl { names, ty, init, span: start.to(self.p.prev_span()) }
    }

    /// `{ predicate; ... }`.
    fn predicates(&mut self, what: &str) -> Vec<Expr> {
        let mut out = Vec::new();
        if !self.p.expect(&TokenKind::LBrace, &format!("to start {what}")) {
            return out;
        }
        loop {
            while self.p.eat(&TokenKind::Semi) {}
            if self.p.at_eof() || self.p.eat(&TokenKind::RBrace) {
                return out;
            }
            let before = self.p.span();
            let e = self.p.expr();
            out.push(e);
            if !self.p.eat(&TokenKind::Semi) && !self.p.at(&TokenKind::RBrace) {
                let t = self.p.peek_token().clone();
                self.p.error(t.span, format!("expected ';' after the predicate, found {}", t.kind));
                if self.p.span() == before {
                    self.p.bump();
                }
                while !self.p.at_eof() && !self.p.at(&TokenKind::Semi) && !self.p.at(&TokenKind::RBrace) {
                    self.p.bump();
                }
            }
        }
    }

    fn domain(&mut self, kind: DomainKind, start: Span) -> Domain {
        if kind != DomainKind::Plain {
            self.p.bump();
        }
        self.p.expect_word("domain", "");
        let mut d = Domain {
            kind,
            model: None,
            patterns: Vec::new(),
            primitives: Vec::new(),
            default_values: Vec::new(),
            span: start,
        };
        if kind == DomainKind::Primitive {
            loop {
                let n = self.p.ident("a variable");
                self.p.expect(&TokenKind::Colon, "after the variable");
                let t = self.p.type_expr();
                d.primitives.push((n, t));
                if !self.p.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.p.expect(&TokenKind::Semi, "after the primitive domain");
            d.span = start.to(self.p.prev_span());
            return d;
        }
        d.model = Some(self.p.ident("a model parameter"));
        if !self.p.at(&TokenKind::Semi) {
            loop {
                let t = self.template();
                d.patterns.push(t);
                if !self.p.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        if self.p.at_word("implementedby") {
            let s = self.p.bump().span;
            self.p.ident("an implementation");
            self.p.error(s.to(self.p.prev_span()), "domains implemented in Java are not supported");
        }
        if self.p.eat_word("default_values") && self.p.expect(&TokenKind::LBrace, "after 'default_values'") {
            loop {
                if self.p.at_eof() || self.p.eat(&TokenKind::RBrace) {
                    break;
                }
                let v = self.p.ident("a variable");
                self.p.expect(&TokenKind::Eq, "after the variable");
                let e = self.p.expr();
                self.p.expect(&TokenKind::Semi, "after the default value");
                d.default_values.push((v, e));
            }
        }
        self.p.expect(&TokenKind::Semi, "after the domain");
        d.span = start.to(self.p.prev_span());
        d
    }

    /// Whether a template starts here: `v : T` or `: T`.
    fn at_template(&self) -> bool {
        self.p.at(&TokenKind::Colon) || (self.p.at_ident() && matches!(self.p.peek_at(1), TokenKind::Colon))
    }

    fn template(&mut self) -> Template {
        let start = self.p.span();
        let var = if self.p.at_ident() {
            let v = self.p.ident("a variable");
            Some(v).filter(|v| v.name != "_")
        } else {
            None
        };
        self.p.expect(&TokenKind::Colon, "after the template's variable");
        let ty = self.p.type_expr();
        if matches!(ty.kind, TypeExprKind::Collection(..)) {
            let mut members = Vec::new();
            let mut rest = None;
            if self.p.eat(&TokenKind::LBrace) {
                if !self.p.at(&TokenKind::RBrace) {
                    loop {
                        if self.at_template() {
                            members.push(Member::Template(Box::new(self.template())));
                        } else {
                            members.push(Member::Var(self.p.ident("a variable")));
                        }
                        if self.p.eat(&TokenKind::PlusPlus) {
                            let r = self.p.ident("the rest's variable");
                            rest = Some(r);
                            break;
                        }
                        if !self.p.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.p.expect(&TokenKind::RBrace, "to close the collection template");
            }
            let guard = self.guard();
            return Template::Collection(CollectionTemplate {
                var,
                ty,
                members,
                rest,
                guard,
                span: start.to(self.p.prev_span()),
            });
        }
        let mut properties = Vec::new();
        if self.p.eat(&TokenKind::LBrace) {
            if !self.p.at(&TokenKind::RBrace) {
                loop {
                    let s = self.p.span();
                    let (property, opposite) = if self.p.eat_word("opposite") {
                        self.p.expect(&TokenKind::LParen, "after 'opposite'");
                        let (class, prop) = self.opposite_path();
                        self.p.expect(&TokenKind::RParen, "to close 'opposite'");
                        (prop, Some(class))
                    } else {
                        (self.p.ident("a property"), None)
                    };
                    self.p.expect(&TokenKind::Eq, "after the property");
                    let value = if self.at_template() {
                        TemplateValue::Template(self.template())
                    } else {
                        TemplateValue::Expr(self.p.expr())
                    };
                    properties.push(PropertyTemplate { property, opposite, value, span: s.to(self.p.prev_span()) });
                    if !self.p.eat(&TokenKind::Comma) {
                        break;
                    }
                }
            }
            self.p.expect(&TokenKind::RBrace, "to close the template");
        }
        let guard = self.guard();
        Template::Object(ObjectTemplate { var, ty, properties, guard, span: start.to(self.p.prev_span()) })
    }

    /// `{ condition }` after a template.
    fn guard(&mut self) -> Option<Expr> {
        if !self.p.at(&TokenKind::LBrace) {
            return None;
        }
        self.p.bump();
        let e = self.p.expr();
        self.p.expect(&TokenKind::RBrace, "to close the template's condition");
        Some(e)
    }
}

/// A predicate that calls a relation: `R(args)` (or `not R(args)`).
pub fn relation_call(e: &Expr) -> Option<(&Ident, &[Expr], bool)> {
    match &e.kind {
        ExprKind::Call { source: None, name, args, .. } => Some((name, args, false)),
        ExprKind::Unary { op: oxy_ocl::syntax::ast::UnOp::Not, operand } => match &operand.kind {
            ExprKind::Call { source: None, name, args, .. } => Some((name, args, true)),
            _ => None,
        },
        _ => None,
    }
}
