//! Parser for OCLinEcore documents: Ecore metamodels written as text, with embedded OCL
//! (the syntax of Eclipse OCL's OCLinEcore editor).
//!
//! Embedded expressions are parsed by the OCL expression parser on the same token stream,
//! so all spans are offsets into the document. Like the Complete OCL parser, this parser
//! never fails: errors are reported and parsing resumes at the next statement.

use super::Parser;
use crate::diagnostic::Diagnostic;
use crate::span::Span;
use crate::syntax::ast::*;
use crate::syntax::lexer::TokenKind;

/// The annotation source under which EMF stores documentation.
pub const GENMODEL_SOURCE: &str = "http://www.eclipse.org/emf/2002/GenModel";

/// Parses an OCLinEcore document.
pub fn parse_ecore_document(src: &str) -> (EcoreDocument, Vec<Diagnostic>) {
    let mut p = Parser::new(src);
    let doc = p.ecore_document();
    (doc, p.diags)
}

/// Qualifiers allowed on attributes and references.
pub const FEATURE_QUALIFIERS: &[&str] = &[
    "composes",
    "derived",
    "id",
    "ordered",
    "readonly",
    "resolve",
    "transient",
    "unique",
    "unsettable",
    "volatile",
    "static",
];
/// Qualifiers allowed on operations.
pub const OPERATION_QUALIFIERS: &[&str] = &["derived", "ordered", "transient", "unique", "static"];
/// Qualifiers allowed on parameters.
pub const PARAMETER_QUALIFIERS: &[&str] = &["ordered", "unique"];

impl Parser {
    fn ecore_document(&mut self) -> EcoreDocument {
        let mut doc = EcoreDocument::default();
        if self.at_word("module") {
            self.bump();
            self.ident("a module name");
            self.eat(&TokenKind::Semi);
        }
        loop {
            if self.at_eof() {
                return doc;
            }
            if self.at_word("import") || self.at_word("library") {
                let imp = self.ecore_import();
                doc.imports.push(imp);
            } else if self.at_word("package") {
                let pkg = self.package_def();
                doc.packages.push(pkg);
            } else {
                let t = self.peek_token().clone();
                self.error(t.span, format!("expected 'import' or 'package', found {}", t.kind));
                self.bump();
                self.skip_statement();
            }
        }
    }

    /// Skips the rest of a statement: up to and including `;`, or up to a `}` closing the
    /// enclosing element. Nested braces are skipped as a whole.
    fn skip_statement(&mut self) {
        let mut depth = 0usize;
        loop {
            match self.peek() {
                TokenKind::Eof => return,
                TokenKind::Semi if depth == 0 => {
                    self.bump();
                    return;
                }
                TokenKind::LBrace => depth += 1,
                TokenKind::RBrace if depth == 0 => return,
                TokenKind::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        self.bump();
                        self.eat(&TokenKind::Semi);
                        return;
                    }
                }
                _ => {}
            }
            self.bump();
        }
    }

    /// Expects the `;` ending a statement.
    fn end_statement(&mut self, what: &str) {
        if self.eat(&TokenKind::Semi) {
            return;
        }
        let t = self.peek_token().clone();
        self.error(t.span, format!("expected ';' after {what}, found {}", t.kind));
        if !matches!(t.kind, TokenKind::RBrace) {
            self.skip_statement();
        }
    }

    fn ecore_import(&mut self) -> Import {
        let start = self.bump().span;
        let alias = if self.at_ident() && matches!(self.peek_at(1), TokenKind::Colon) {
            let a = self.ident("an import alias");
            self.bump();
            Some(a)
        } else {
            None
        };
        let uri = match self.peek().clone() {
            TokenKind::Str(s) => {
                self.bump();
                s
            }
            other => {
                let span = self.span();
                self.error(span, format!("expected the imported URI as a string, found {other}"));
                String::new()
            }
        };
        if self.eat(&TokenKind::ColonColon) {
            self.eat(&TokenKind::Star);
        }
        let span = start.to(self.prev_span());
        self.end_statement("the import");
        Import { alias, uri, span }
    }

    fn package_def(&mut self) -> PackageDef {
        let start = self.bump().span;
        let name = self.ident("a package name");
        let mut ns_prefix = None;
        let mut ns_uri = None;
        if self.eat(&TokenKind::Colon) {
            ns_prefix = Some(self.ident("the package's nsPrefix"));
        }
        if self.eat(&TokenKind::Eq) {
            match self.peek().clone() {
                TokenKind::Str(s) => {
                    let span = self.bump().span;
                    ns_uri = Some((s, span));
                }
                other => {
                    let span = self.span();
                    self.error(span, format!("expected the nsURI as a string, found {other}"));
                }
            }
        }
        let mut pkg = PackageDef {
            name,
            ns_prefix,
            ns_uri,
            annotations: Vec::new(),
            packages: Vec::new(),
            classifiers: Vec::new(),
            span: start,
            body: None,
        };
        if self.eat(&TokenKind::Semi) {
            pkg.span = start.to(self.prev_span());
            return pkg;
        }
        let open = self.span();
        if !self.expect(&TokenKind::LBrace, "to start the package") {
            self.skip_statement();
            pkg.span = start.to(self.prev_span());
            return pkg;
        }
        loop {
            if self.eat(&TokenKind::RBrace) {
                pkg.body = Some(open.to(self.prev_span()));
                break;
            }
            if self.at_eof() {
                self.error(pkg.name.span, format!("missing '}}' to close package '{}'", pkg.name));
                break;
            }
            if self.at_word("annotation") || self.at_word("documentation") {
                let a = self.annotation_def();
                pkg.annotations.push(a);
            } else if self.at_word("package") {
                let p = self.package_def();
                pkg.packages.push(p);
            } else if self.at_classifier_start() {
                let c = self.classifier_def();
                pkg.classifiers.push(c);
            } else {
                let t = self.peek_token().clone();
                self.error(t.span, format!("expected a class, data type, enumeration or package, found {}", t.kind));
                self.bump();
                self.skip_statement();
            }
        }
        pkg.span = start.to(self.prev_span());
        pkg
    }

    fn at_classifier_start(&self) -> bool {
        ["abstract", "class", "interface", "datatype", "primitive", "enum"].iter().any(|w| self.at_word(w))
    }

    /// Whether the `{` at the current position opens a qualifier list such as `{ ordered }`
    /// (rather than a body). An empty `{}` is a qualifier list only if a body follows.
    fn at_qualifier_braces(&self, allowed: &[&str]) -> bool {
        if !matches!(self.peek(), TokenKind::LBrace) {
            return false;
        }
        let mut i = 1;
        loop {
            match self.peek_at(i) {
                TokenKind::RBrace => return i > 1 || matches!(self.peek_at(i + 1), TokenKind::LBrace),
                TokenKind::Comma | TokenKind::Bang => i += 1,
                TokenKind::Ident(w) if allowed.contains(&w.as_str()) => i += 1,
                _ => return false,
            }
        }
    }

    /// An optional qualifier list and the span of its braces.
    fn qualifier_list(&mut self, allowed: &[&str]) -> (Vec<Qualifier>, Option<Span>) {
        if !self.at_qualifier_braces(allowed) {
            return (Vec::new(), None);
        }
        let open = self.span();
        let list = self.qualifiers();
        (list, Some(open.to(self.prev_span())))
    }

    fn qualifiers(&mut self) -> Vec<Qualifier> {
        let mut out = Vec::new();
        self.bump(); // '{'
        loop {
            let start = self.span();
            match self.peek().clone() {
                TokenKind::RBrace => {
                    self.bump();
                    return out;
                }
                TokenKind::Comma => {
                    self.bump();
                }
                TokenKind::Bang => {
                    self.bump();
                    let name = self.ident("a qualifier");
                    out.push(Qualifier { name: name.name, negated: true, span: start.to(name.span) });
                }
                TokenKind::Ident(name) => {
                    self.bump();
                    out.push(Qualifier { name, negated: false, span: start });
                }
                other => {
                    self.error(start, format!("expected a qualifier or '}}', found {other}"));
                    return out;
                }
            }
        }
    }

    fn classifier_def(&mut self) -> ClassifierDef {
        let start = self.span();
        let mut is_abstract = false;
        let mut is_interface = false;
        loop {
            if self.at_word("abstract") {
                is_abstract = true;
            } else if !self.at_word("primitive") {
                break;
            }
            self.bump();
        }
        let keyword = self.bump();
        let keyword_span = keyword.span;
        let word = match &keyword.kind {
            TokenKind::Ident(w) => w.clone(),
            other => {
                self.error(keyword.span, format!("expected 'class', 'datatype' or 'enum', found {other}"));
                String::new()
            }
        };
        if word == "interface" {
            is_interface = true;
        }
        let name = self.ident("a name");
        if self.at(&TokenKind::Lt) {
            let span = self.span();
            self.error(span, "generic type parameters are not supported");
            while !self.at_eof() && !self.eat(&TokenKind::Gt) {
                self.bump();
            }
        }
        let is_class = word == "class" || word == "interface";
        let mut supers = Vec::new();
        let mut extends = None;
        if is_class && self.at_word("extends") {
            let extends_start = self.bump().span;
            loop {
                supers.push(self.type_ref());
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
            extends = Some(extends_start.to(self.prev_span()));
        }
        let instance_class = if self.eat(&TokenKind::Colon) {
            match self.peek().clone() {
                TokenKind::Str(s) => {
                    self.bump();
                    Some(s)
                }
                other => {
                    let span = self.span();
                    self.error(span, format!("expected the instance class name as a string, found {other}"));
                    None
                }
            }
        } else {
            None
        };
        let mut qualifiers = None;
        if self.at_qualifier_braces(&["interface", "serializable"]) {
            let open = self.span();
            let list = self.qualifiers();
            qualifiers = Some(open.to(self.prev_span()));
            for q in list {
                if q.name == "interface" && is_class {
                    is_interface = !q.negated;
                } else if q.name != "serializable" {
                    self.error(q.span, format!("'{}' is not a qualifier of this element", q.name));
                }
            }
        }
        let mut annotations = Vec::new();
        let mut features = Vec::new();
        let mut operations = Vec::new();
        let mut invariants = Vec::new();
        let mut literals = Vec::new();
        let mut body = None;
        if !self.eat(&TokenKind::Semi) {
            let open = self.span();
            if self.expect(&TokenKind::LBrace, &format!("to start the body of '{name}'")) {
                loop {
                    if self.eat(&TokenKind::RBrace) {
                        body = Some(open.to(self.prev_span()));
                        break;
                    }
                    if self.at_eof() {
                        self.error(name.span, format!("missing '}}' to close '{name}'"));
                        break;
                    }
                    let s = self.span();
                    if self.at_word("annotation") || self.at_word("documentation") {
                        annotations.push(self.annotation_def());
                    } else if is_class && self.at_feature_start() {
                        features.push(self.feature_def());
                    } else if is_class && self.at_operation_start() {
                        operations.push(self.operation_def());
                    } else if self.at_word("invariant") || self.at_word("inv") || self.at_word("callable") {
                        if let Some(c) = self.ecore_constraint() {
                            if is_class {
                                invariants.push(c);
                            } else {
                                self.error(c.span, "invariants are only supported on classes");
                            }
                        }
                    } else if word == "enum" && (self.at_word("literal") || self.at_ident()) {
                        literals.push(self.literal_def());
                    } else {
                        let t = self.peek_token().clone();
                        let expected = if is_class {
                            "'attribute', 'property', 'operation' or 'invariant'"
                        } else if word == "enum" {
                            "an enumeration literal"
                        } else {
                            "an annotation"
                        };
                        self.error(s, format!("expected {expected}, found {}", t.kind));
                        self.bump();
                        self.skip_statement();
                    }
                }
            } else {
                self.skip_statement();
            }
        }
        let kind = match word.as_str() {
            "class" | "interface" => {
                ClassifierDefKind::Class { is_abstract, is_interface, supers, features, operations, invariants }
            }
            "enum" => ClassifierDefKind::Enum { literals },
            "datatype" => ClassifierDefKind::DataType,
            other => {
                if !other.is_empty() {
                    self.error(keyword.span, format!("expected 'class', 'datatype' or 'enum', found '{other}'"));
                }
                ClassifierDefKind::DataType
            }
        };
        ClassifierDef {
            name,
            instance_class,
            annotations,
            kind,
            span: start.to(self.prev_span()),
            keyword: keyword_span,
            extends,
            qualifiers,
            body,
        }
    }

    /// `static`/`definition` modifiers followed by `attribute` or `property`.
    fn at_feature_start(&self) -> bool {
        let mut i = 0;
        while matches!(self.peek_at(i), TokenKind::Ident(w) if w == "static" || w == "definition") {
            i += 1;
        }
        matches!(self.peek_at(i), TokenKind::Ident(w) if w == "attribute" || w == "property")
    }

    fn at_operation_start(&self) -> bool {
        let mut i = 0;
        while matches!(self.peek_at(i), TokenKind::Ident(w) if w == "static" || w == "definition") {
            i += 1;
        }
        matches!(self.peek_at(i), TokenKind::Ident(w) if w == "operation")
    }

    fn skip_modifiers(&mut self) {
        while self.at_word("static") || self.at_word("definition") {
            let span = self.bump().span;
            self.diags.push(Diagnostic::warning(span, "'static' and 'definition' are ignored"));
        }
    }

    fn literal_def(&mut self) -> LiteralDef {
        let start = self.span();
        if self.at_word("literal") && matches!(self.peek_at(1), TokenKind::Ident(_) | TokenKind::QuotedIdent(_)) {
            self.bump();
        }
        let name = self.ident("a literal name");
        let mut literal = None;
        let mut value = None;
        if self.eat(&TokenKind::Colon) {
            match self.peek().clone() {
                TokenKind::Str(s) => {
                    self.bump();
                    literal = Some(s);
                }
                other => {
                    let span = self.span();
                    self.error(span, format!("expected the literal string, found {other}"));
                }
            }
        }
        if self.eat(&TokenKind::Eq) {
            let negative = self.eat(&TokenKind::Minus);
            match self.peek().clone() {
                TokenKind::Int(i) => {
                    self.bump();
                    value = Some(if negative { -i } else { i });
                }
                other => {
                    let span = self.span();
                    self.error(span, format!("expected an integer value, found {other}"));
                }
            }
        }
        let span = start.to(self.prev_span());
        if self.at(&TokenKind::LBrace) {
            self.skip_braces();
        } else {
            self.end_statement("the literal");
        }
        LiteralDef { name, value, literal, span }
    }

    /// Skips a balanced `{ ... }` (and a following `;`).
    fn skip_braces(&mut self) {
        let mut depth = 0usize;
        loop {
            match self.peek() {
                TokenKind::Eof => return,
                TokenKind::LBrace => depth += 1,
                TokenKind::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        self.bump();
                        self.eat(&TokenKind::Semi);
                        return;
                    }
                }
                _ => {}
            }
            self.bump();
        }
    }

    /// `Name`, `pkg::Name`, or a collection type `Set(Name)`.
    fn type_ref(&mut self) -> TypeRef {
        let start = self.span();
        if let TokenKind::Ident(name) = self.peek().clone()
            && let Some(kind) = CollectionKind::from_name(&name)
            && matches!(self.peek_at(1), TokenKind::LParen)
        {
            self.bump();
            self.bump();
            let inner = self.type_ref();
            self.expect(&TokenKind::RParen, "to close the collection type");
            return TypeRef { path: inner.path, collection: Some(kind), span: start.to(self.prev_span()) };
        }
        let mut path = vec![self.ident("a type name")];
        while self.eat(&TokenKind::ColonColon) {
            path.push(self.ident("a type name after '::'"));
        }
        if self.at(&TokenKind::Lt) {
            let span = self.span();
            self.error(span, "generic type arguments are not supported");
            while !self.at_eof() && !self.eat(&TokenKind::Gt) {
                self.bump();
            }
        }
        TypeRef { path, collection: None, span: start.to(self.prev_span()) }
    }

    /// An optional multiplicity: `[*]`, `[+]`, `[?]`, `[n]`, `[n..m]`, `[n..*]`, possibly with
    /// a null-free marker (`|?` or `|1`), which is accepted and ignored.
    fn multiplicity(&mut self) -> Option<MultiplicityDef> {
        if !self.at(&TokenKind::LBracket) {
            return None;
        }
        let start = self.bump().span;
        let bound = |p: &mut Parser| -> Option<i32> {
            match p.peek().clone() {
                TokenKind::Int(i) => {
                    p.bump();
                    Some(i.clamp(0, i32::MAX as i64) as i32)
                }
                TokenKind::Star => {
                    p.bump();
                    Some(-1)
                }
                _ => None,
            }
        };
        let (lower, upper) = match self.peek().clone() {
            TokenKind::Star => {
                self.bump();
                (0, -1)
            }
            TokenKind::Plus => {
                self.bump();
                (1, -1)
            }
            TokenKind::Question => {
                self.bump();
                (0, 1)
            }
            TokenKind::Int(_) => {
                let lower = bound(self).unwrap_or(0);
                if self.eat(&TokenKind::DotDot) {
                    match bound(self) {
                        Some(u) => (lower, u),
                        None => {
                            let span = self.span();
                            self.error(span, "expected an upper bound (a number or '*')");
                            (lower, -1)
                        }
                    }
                } else {
                    (lower, lower)
                }
            }
            other => {
                let span = self.span();
                self.error(span, format!("expected a multiplicity such as '*', '?', '1' or '0..*', found {other}"));
                (0, 1)
            }
        };
        if self.eat(&TokenKind::Bar) && !self.eat(&TokenKind::Question) {
            self.eat(&TokenKind::Int(1));
        }
        self.expect(&TokenKind::RBracket, "to close the multiplicity");
        let span = start.to(self.prev_span());
        if upper != -1 && upper < lower {
            self.error(span, "the upper bound is smaller than the lower bound");
        }
        Some(MultiplicityDef { lower, upper, span })
    }

    fn feature_def(&mut self) -> FeatureDef {
        let start = self.span();
        self.skip_modifiers();
        let keyword = if self.at_word("attribute") { FeatureKeyword::Attribute } else { FeatureKeyword::Property };
        self.bump();
        let name = self.ident("a feature name");
        let opposite =
            if self.eat(&TokenKind::Hash) { Some(self.ident("the name of the opposite property")) } else { None };
        let (ty, multiplicity) = if self.eat(&TokenKind::Colon) {
            let t = self.type_ref();
            (Some(t), self.multiplicity())
        } else {
            (None, None)
        };
        let default = if self.eat(&TokenKind::Eq) {
            match self.peek().clone() {
                TokenKind::Str(s) => {
                    self.bump();
                    Some(s)
                }
                other => {
                    let span = self.span();
                    self.error(span, format!("expected the default value as a string, found {other}"));
                    None
                }
            }
        } else {
            None
        };
        let (qualifiers, qualifier_span) = self.qualifier_list(FEATURE_QUALIFIERS);
        let mut constraints = Vec::new();
        let mut annotations = Vec::new();
        let mut body = None;
        if self.at(&TokenKind::LBrace) {
            let open = self.bump().span;
            loop {
                if self.eat(&TokenKind::RBrace) {
                    body = Some(open.to(self.prev_span()));
                    break;
                }
                if self.at_eof() {
                    self.error(name.span, format!("missing '}}' to close '{name}'"));
                    break;
                }
                if ["initial", "init", "derivation", "derive"].iter().any(|w| self.at_word(w)) {
                    if let Some(c) = self.ecore_constraint() {
                        constraints.push(c);
                    }
                } else if self.at_word("annotation") || self.at_word("documentation") {
                    annotations.push(self.annotation_def());
                } else if self.at_word("key") {
                    self.bump();
                    self.skip_statement();
                } else {
                    let t = self.peek_token().clone();
                    self.error(t.span, format!("expected 'initial', 'derivation' or '}}', found {}", t.kind));
                    self.bump();
                    self.skip_statement();
                }
            }
            self.eat(&TokenKind::Semi);
        } else {
            self.end_statement(&format!("'{name}'"));
        }
        FeatureDef {
            keyword,
            name,
            opposite,
            ty,
            multiplicity,
            default,
            qualifiers,
            constraints,
            annotations,
            span: start.to(self.prev_span()),
            qualifier_span,
            body,
        }
    }

    fn operation_def(&mut self) -> OperationDef {
        let start = self.span();
        self.skip_modifiers();
        self.bump(); // 'operation'
        let name = self.ident("an operation name");
        let mut params = Vec::new();
        let params_start = self.span();
        if self.expect(&TokenKind::LParen, "to start the parameter list") {
            if !self.at(&TokenKind::RParen) {
                loop {
                    let p_start = self.span();
                    let p_name = self.ident("a parameter name");
                    let (ty, multiplicity) = if self.eat(&TokenKind::Colon) {
                        let t = self.type_ref();
                        (Some(t), self.multiplicity())
                    } else {
                        (None, None)
                    };
                    let qualifiers =
                        if self.at_qualifier_braces(PARAMETER_QUALIFIERS) { self.qualifiers() } else { Vec::new() };
                    if self.at(&TokenKind::LBrace) {
                        self.skip_braces();
                    }
                    params.push(ParamDef {
                        name: p_name,
                        ty,
                        multiplicity,
                        qualifiers,
                        span: p_start.to(self.prev_span()),
                    });
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
            }
            self.expect(&TokenKind::RParen, "to close the parameter list");
        }
        let params_span = params_start.to(self.prev_span());
        let (ty, multiplicity) = if self.eat(&TokenKind::Colon) {
            let t = self.type_ref();
            (Some(t), self.multiplicity())
        } else {
            (None, None)
        };
        let mut throws = Vec::new();
        if self.at_word("throws") {
            self.bump();
            loop {
                throws.push(self.type_ref());
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        let (qualifiers, qualifier_span) = self.qualifier_list(OPERATION_QUALIFIERS);
        let mut constraints = Vec::new();
        let mut annotations = Vec::new();
        let mut body = None;
        if self.at(&TokenKind::LBrace) {
            let open = self.bump().span;
            loop {
                if self.eat(&TokenKind::RBrace) {
                    body = Some(open.to(self.prev_span()));
                    break;
                }
                if self.at_eof() {
                    self.error(name.span, format!("missing '}}' to close '{name}'"));
                    break;
                }
                if ["precondition", "pre", "postcondition", "post", "body"].iter().any(|w| self.at_word(w)) {
                    if let Some(c) = self.ecore_constraint() {
                        constraints.push(c);
                    }
                } else if self.at_word("annotation") || self.at_word("documentation") {
                    annotations.push(self.annotation_def());
                } else {
                    let t = self.peek_token().clone();
                    self.error(
                        t.span,
                        format!("expected 'precondition', 'postcondition', 'body' or '}}', found {}", t.kind),
                    );
                    self.bump();
                    self.skip_statement();
                }
            }
            self.eat(&TokenKind::Semi);
        } else {
            self.end_statement(&format!("operation '{name}'"));
        }
        OperationDef {
            name,
            params,
            params_span,
            ty,
            multiplicity,
            throws,
            qualifiers,
            constraints,
            annotations,
            span: start.to(self.prev_span()),
            qualifier_span,
            body,
        }
    }

    /// `invariant Name('message'): expr;` and the other constraint forms. Returns `None` for
    /// constraints without an expression (`body: ;`).
    fn ecore_constraint(&mut self) -> Option<Constraint> {
        let start = self.span();
        if self.at_word("callable") {
            self.bump();
            if !self.at_word("invariant") && !self.at_word("inv") {
                let span = self.span();
                self.error(span, "expected 'invariant' after 'callable'");
            }
        }
        let kw = self.bump();
        let word = match &kw.kind {
            TokenKind::Ident(w) => w.as_str(),
            _ => "",
        };
        let kind = match word {
            "precondition" | "pre" => ConstraintKind::Pre,
            "postcondition" | "post" => ConstraintKind::Post,
            "body" => ConstraintKind::Body,
            "initial" | "init" => ConstraintKind::Init,
            "derivation" | "derive" => ConstraintKind::Derive,
            _ => ConstraintKind::Inv,
        };
        let name = if self.at_ident() { Some(self.ident("a constraint name")) } else { None };
        let message = if self.at(&TokenKind::LParen) {
            self.bump();
            let m = self.expr();
            self.expect(&TokenKind::RParen, "to close the constraint message");
            Some(m)
        } else {
            None
        };
        if kind == ConstraintKind::Inv && self.eat(&TokenKind::Semi) {
            return None;
        }
        if !self.expect(&TokenKind::Colon, &format!("after '{word}'")) {
            self.skip_statement();
            return None;
        }
        if self.eat(&TokenKind::Semi) {
            return None;
        }
        let expr = self.expr();
        let span = start.to(self.prev_span());
        self.end_statement("the expression");
        Some(Constraint { kind, name, message, expr, span })
    }

    fn annotation_def(&mut self) -> AnnotationDef {
        let start = self.bump();
        let is_documentation = matches!(&start.kind, TokenKind::Ident(w) if w == "documentation");
        let mut source = String::new();
        let mut details = Vec::new();
        if is_documentation {
            source = GENMODEL_SOURCE.to_string();
            if let TokenKind::Str(s) = self.peek().clone() {
                self.bump();
                details.push(("documentation".to_string(), s));
            }
        } else {
            match self.peek().clone() {
                TokenKind::Str(s) => {
                    self.bump();
                    source = s;
                }
                TokenKind::Ident(s) | TokenKind::QuotedIdent(s) => {
                    self.bump();
                    source = s;
                }
                _ => {}
            }
        }
        if self.eat(&TokenKind::LParen) {
            loop {
                if self.at(&TokenKind::RParen) {
                    break;
                }
                let key = match self.peek().clone() {
                    TokenKind::Str(s) | TokenKind::Ident(s) | TokenKind::QuotedIdent(s) => {
                        self.bump();
                        s
                    }
                    other => {
                        let span = self.span();
                        self.error(span, format!("expected a detail key, found {other}"));
                        break;
                    }
                };
                let mut value = String::new();
                if self.eat(&TokenKind::Eq)
                    && let TokenKind::Str(s) = self.peek().clone()
                {
                    self.bump();
                    value = s;
                }
                details.push((key, value));
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect(&TokenKind::RParen, "to close the annotation details");
        }
        let span = start.span.to(self.prev_span());
        if self.at(&TokenKind::LBrace) {
            // Nested annotations and references are not supported; skip them.
            self.skip_braces();
        } else {
            self.end_statement(if is_documentation { "the documentation" } else { "the annotation" });
        }
        AnnotationDef { source, details, span }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_metamodel() {
        let src = r#"
            module library;
            import ecore : 'http://www.eclipse.org/emf/2002/Ecore#/';
            package library : lib = 'http://example.org/library' {
              documentation 'A library.';
              class Library {
                attribute name : String[?];
                property books#library : Book[*] { ordered, composes };
              }
              abstract class Item extends Named, ecore::EObject : 'org.Item' { interface } {
                attribute tags : String[0..*] { !unique };
                property owner : Library = 'x' { derived volatile } {
                  derivation: oclContainer().oclAsType(Library);
                }
                operation rename(newName : String[1], flag : Boolean) : Boolean[?] throws Error {
                  precondition NotEmpty('name must not be empty'): newName.size() > 0;
                  post: result = true;
                  body: true;
                }
                invariant HasName('needs a name'): name <> null;
                invariant: true;
              }
              enum Genre { literal FICTION; SCIENCE = 3; HISTORY : 'history'; }
              datatype Date : 'java.util.Date' { serializable };
              package sub : sub = 'http://example.org/library/sub';
            }
        "#;
        let (doc, diags) = parse_ecore_document(src);
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(doc.imports[0].alias.as_ref().unwrap().name, "ecore");
        let pkg = &doc.packages[0];
        assert_eq!(pkg.ns_prefix.as_ref().unwrap().name, "lib");
        assert_eq!(pkg.ns_uri.as_ref().unwrap().0, "http://example.org/library");
        assert_eq!(pkg.annotations[0].details[0].1, "A library.");
        assert_eq!(pkg.classifiers.len(), 4);
        assert_eq!(pkg.packages.len(), 1);
        let ClassifierDefKind::Class { features, .. } = &pkg.classifiers[0].kind else { panic!() };
        assert_eq!(features[1].opposite.as_ref().unwrap().name, "library");
        assert_eq!(features[1].multiplicity.unwrap().upper, -1);
        assert_eq!(features[1].qualifiers.len(), 2);
        let ClassifierDefKind::Class { is_abstract, is_interface, supers, features, operations, invariants } =
            &pkg.classifiers[1].kind
        else {
            panic!()
        };
        assert!(*is_abstract && *is_interface);
        assert_eq!(supers[1].path.len(), 2);
        assert_eq!(pkg.classifiers[1].instance_class.as_deref(), Some("org.Item"));
        assert!(features[0].qualifiers[0].negated);
        assert_eq!(features[1].default.as_deref(), Some("x"));
        assert_eq!(features[1].constraints[0].kind, ConstraintKind::Derive);
        let op = &operations[0];
        assert_eq!(op.params.len(), 2);
        assert_eq!(op.multiplicity.unwrap().upper, 1);
        let kinds: Vec<ConstraintKind> = op.constraints.iter().map(|c| c.kind).collect();
        assert_eq!(kinds, [ConstraintKind::Pre, ConstraintKind::Post, ConstraintKind::Body]);
        assert!(op.constraints[0].message.is_some());
        assert_eq!(invariants.len(), 2);
        let ClassifierDefKind::Enum { literals } = &pkg.classifiers[2].kind else { panic!() };
        assert_eq!(
            literals.iter().map(|l| l.name.name.as_str()).collect::<Vec<_>>(),
            ["FICTION", "SCIENCE", "HISTORY"]
        );
        assert_eq!(literals[1].value, Some(3));
        assert_eq!(literals[2].literal.as_deref(), Some("history"));
    }

    #[test]
    fn records_the_spans_of_bodies_and_clauses() {
        let src = "package p : p = 'u' {\n  abstract class A extends B, C { interface } {\n    property x : B[*] { composes } { derivation: null; }\n    operation f(a : B) : B { ordered };\n  }\n  class B;\n}";
        let (doc, diags) = parse_ecore_document(src);
        assert!(diags.is_empty(), "{diags:?}");
        let slice = |s: Span| &src[s.range()];
        let pkg = &doc.packages[0];
        assert!(slice(pkg.body.unwrap()).starts_with("{\n  abstract") && slice(pkg.body.unwrap()).ends_with('}'));
        let a = &pkg.classifiers[0];
        assert_eq!(slice(a.keyword), "class");
        assert_eq!(slice(a.extends.unwrap()), "extends B, C");
        assert_eq!(slice(a.qualifiers.unwrap()), "{ interface }");
        assert!(slice(a.body.unwrap()).starts_with("{\n    property"));
        let ClassifierDefKind::Class { features, operations, .. } = &a.kind else { panic!() };
        assert_eq!(slice(features[0].qualifier_span.unwrap()), "{ composes }");
        assert_eq!(slice(features[0].body.unwrap()), "{ derivation: null; }");
        assert_eq!(slice(operations[0].params_span), "(a : B)");
        assert_eq!(slice(operations[0].qualifier_span.unwrap()), "{ ordered }");
        assert!(operations[0].body.is_none());
        assert!(pkg.classifiers[1].body.is_none());
    }

    #[test]
    fn recovers_from_errors() {
        let src = "package p : p = 'u' {\n class A {\n attribute x : ;\n invariant I: x > ;\n attribute y : String;\n }\n class B { oops }\n}";
        let (doc, diags) = parse_ecore_document(src);
        assert!(diags.len() >= 3, "{diags:?}");
        let pkg = &doc.packages[0];
        assert_eq!(pkg.classifiers.len(), 2);
        let ClassifierDefKind::Class { features, .. } = &pkg.classifiers[0].kind else { panic!() };
        assert_eq!(features.last().unwrap().name.name, "y");
    }
}
