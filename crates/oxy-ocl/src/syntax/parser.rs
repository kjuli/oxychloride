//! Recursive-descent parser for OCL expressions and Complete OCL documents.
//!
//! The parser never fails: syntax errors are reported as diagnostics and the affected
//! parts of the tree are replaced by [`ExprKind::Error`] nodes, so later phases can still
//! check the rest of a document (which is what an editor needs).

mod oclinecore;

pub use oclinecore::{
    FEATURE_QUALIFIERS, GENMODEL_SOURCE, OPERATION_QUALIFIERS, PARAMETER_QUALIFIERS, parse_ecore_document,
};

use super::ast::*;
use super::lexer::{Keyword, Token, TokenKind, tokenize};
use crate::diagnostic::Diagnostic;
use crate::span::Span;

/// Parses a standalone OCL expression (as found in Ecore annotations or typed by a user).
pub fn parse_expression(src: &str) -> (Expr, Vec<Diagnostic>) {
    let mut p = Parser::new(src);
    let expr = p.expr();
    if !p.at_eof() {
        let t = p.peek_token().clone();
        p.error(t.span, format!("unexpected {} after end of expression", t.kind));
    }
    (expr, p.diags)
}

/// Parses a Complete OCL document.
pub fn parse_document(src: &str) -> (Document, Vec<Diagnostic>) {
    let mut p = Parser::new(src);
    let doc = p.document();
    (doc, p.diags)
}

/// Words that start a new document-level element; used for error recovery.
const DOCUMENT_WORDS: &[&str] =
    &["import", "package", "endpackage", "context", "inv", "def", "static", "pre", "post", "body", "init", "derive"];

pub(crate) struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    pub(crate) diags: Vec<Diagnostic>,
    next_id: u32,
}

impl Parser {
    pub(crate) fn new(src: &str) -> Self {
        let (tokens, diags) = tokenize(src);
        Parser { tokens, pos: 0, diags, next_id: 0 }
    }

    // ------------------------------------------------------------ token helpers

    fn peek(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    fn peek_at(&self, n: usize) -> &TokenKind {
        &self.tokens[(self.pos + n).min(self.tokens.len() - 1)].kind
    }

    fn peek_token(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn span(&self) -> Span {
        self.tokens[self.pos].span
    }

    fn prev_span(&self) -> Span {
        self.tokens[self.pos.saturating_sub(1)].span
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek(), TokenKind::Eof)
    }

    fn bump(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        t
    }

    /// Compares token kinds, ignoring payloads (except for keywords).
    fn at(&self, kind: &TokenKind) -> bool {
        match (self.peek(), kind) {
            (TokenKind::Keyword(a), TokenKind::Keyword(b)) => a == b,
            (a, b) => std::mem::discriminant(a) == std::mem::discriminant(b),
        }
    }

    fn at_kw(&self, kw: Keyword) -> bool {
        matches!(self.peek(), TokenKind::Keyword(k) if *k == kw)
    }

    /// Whether the current token is the (unquoted) word `w`.
    fn at_word(&self, w: &str) -> bool {
        matches!(self.peek(), TokenKind::Ident(n) if n == w)
    }

    /// Whether the current token starts a document-level element (`inv:`, `context X`, ...).
    /// Document keywords are not reserved, so `body` may still be used as a property name;
    /// the following token disambiguates.
    fn at_document_word(&self) -> bool {
        match self.peek() {
            TokenKind::Ident(w) if DOCUMENT_WORDS.contains(&w.as_str()) => match w.as_str() {
                "endpackage" => true,
                "import" => matches!(self.peek_at(1), TokenKind::Str(_) | TokenKind::Ident(_)),
                _ => matches!(self.peek_at(1), TokenKind::Colon | TokenKind::Ident(_) | TokenKind::QuotedIdent(_)),
            },
            _ => false,
        }
    }

    fn eat(&mut self, kind: &TokenKind) -> bool {
        if self.at(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_kw(&mut self, kw: Keyword) -> bool {
        if self.at_kw(kw) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        // Avoid cascades of errors at the same location.
        if self.diags.last().is_some_and(|d| d.span == span) {
            return;
        }
        self.diags.push(Diagnostic::error(span, message));
    }

    fn expect(&mut self, kind: &TokenKind, context: &str) -> bool {
        if self.eat(kind) {
            return true;
        }
        let t = self.peek_token().clone();
        self.error(t.span, format!("expected {kind} {context}, found {}", t.kind));
        false
    }

    fn ident(&mut self, what: &str) -> Ident {
        match self.peek().clone() {
            TokenKind::Ident(name) | TokenKind::QuotedIdent(name) => {
                let t = self.bump();
                Ident { name, span: t.span }
            }
            other => {
                let span = self.span();
                self.error(span, format!("expected {what}, found {other}"));
                Ident { name: String::new(), span }
            }
        }
    }

    fn at_ident(&self) -> bool {
        matches!(self.peek(), TokenKind::Ident(_) | TokenKind::QuotedIdent(_))
    }

    fn mk(&mut self, kind: ExprKind, span: Span) -> Expr {
        let id = ExprId(self.next_id);
        self.next_id += 1;
        Expr { id, span, kind }
    }

    fn error_expr(&mut self, span: Span) -> Expr {
        self.mk(ExprKind::Error, span)
    }

    // ------------------------------------------------------------ expressions

    pub(crate) fn expr(&mut self) -> Expr {
        self.binary(0)
    }

    fn binary_op(&self) -> Option<(BinOp, u8)> {
        Some(match self.peek() {
            TokenKind::Keyword(Keyword::Implies) => (BinOp::Implies, 1),
            TokenKind::Keyword(Keyword::Xor) => (BinOp::Xor, 2),
            TokenKind::Keyword(Keyword::Or) => (BinOp::Or, 3),
            TokenKind::Keyword(Keyword::And) => (BinOp::And, 4),
            TokenKind::Eq => (BinOp::Eq, 5),
            TokenKind::Ne => (BinOp::Ne, 5),
            TokenKind::Lt => (BinOp::Lt, 6),
            TokenKind::Gt => (BinOp::Gt, 6),
            TokenKind::Le => (BinOp::Le, 6),
            TokenKind::Ge => (BinOp::Ge, 6),
            TokenKind::Plus => (BinOp::Add, 7),
            TokenKind::Minus => (BinOp::Sub, 7),
            TokenKind::Star => (BinOp::Mul, 8),
            TokenKind::Slash => (BinOp::Div, 8),
            _ => return None,
        })
    }

    fn binary(&mut self, min_prec: u8) -> Expr {
        let mut lhs = self.unary();
        while let Some((op, prec)) = self.binary_op() {
            if prec < min_prec {
                break;
            }
            self.bump();
            let rhs = self.binary(prec + 1);
            let span = lhs.span.to(rhs.span);
            lhs = self.mk(ExprKind::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) }, span);
        }
        lhs
    }

    fn unary(&mut self) -> Expr {
        let start = self.span();
        let op = if self.eat_kw(Keyword::Not) {
            UnOp::Not
        } else if self.eat(&TokenKind::Minus) {
            UnOp::Neg
        } else {
            let primary = self.primary();
            return self.postfix(primary);
        };
        let operand = self.unary();
        let span = start.to(operand.span);
        // Fold negative numeric literals.
        let kind = match (&op, &operand.kind) {
            (UnOp::Neg, ExprKind::Int(i)) => ExprKind::Int(-i),
            (UnOp::Neg, ExprKind::Real(r)) => ExprKind::Real(-r),
            _ => ExprKind::Unary { op, operand: Box::new(operand) },
        };
        self.mk(kind, span)
    }

    fn at_pre(&mut self) -> bool {
        if self.at(&TokenKind::At) && matches!(self.peek_at(1), TokenKind::Ident(n) if n == "pre") {
            self.bump();
            self.bump();
            true
        } else {
            false
        }
    }

    fn postfix(&mut self, mut expr: Expr) -> Expr {
        loop {
            let (arrow, safe) = match self.peek() {
                TokenKind::Dot => (false, false),
                TokenKind::SafeDot => (false, true),
                TokenKind::Arrow => (true, false),
                TokenKind::SafeArrow => (true, true),
                _ => return expr,
            };
            self.bump();
            let name = self.ident("a property or operation name");
            if arrow {
                expr = self.arrow_call(expr, name, safe);
                continue;
            }
            if self.at(&TokenKind::LParen) {
                let args = self.args();
                let at_pre = self.at_pre();
                let span = expr.span.to(self.prev_span());
                expr = self
                    .mk(ExprKind::Call { source: Some(Box::new(expr)), name, args, arrow: false, safe, at_pre }, span);
            } else {
                let at_pre = self.at_pre();
                let span = expr.span.to(self.prev_span());
                expr = self.mk(ExprKind::Property { source: Box::new(expr), name, safe, at_pre }, span);
            }
        }
    }

    fn args(&mut self) -> Vec<Expr> {
        self.expect(&TokenKind::LParen, "to start the argument list");
        let mut args = Vec::new();
        if self.eat(&TokenKind::RParen) {
            return args;
        }
        loop {
            args.push(self.expr());
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(&TokenKind::RParen, "to close the argument list");
        args
    }

    fn arrow_call(&mut self, source: Expr, name: Ident, safe: bool) -> Expr {
        if !self.at(&TokenKind::LParen) {
            let span = self.span();
            self.error(span, format!("expected '(' after '->{name}'"));
            let span = source.span.to(name.span);
            return self.mk(
                ExprKind::Call {
                    source: Some(Box::new(source)),
                    name,
                    args: Vec::new(),
                    arrow: true,
                    safe,
                    at_pre: false,
                },
                span,
            );
        }
        if name.name == "iterate" {
            self.bump();
            let iterators = self.iterator_vars(&TokenKind::Semi).unwrap_or_default();
            let acc_start = self.span();
            let acc_name = self.ident("an accumulator variable");
            let acc_ty = if self.eat(&TokenKind::Colon) { Some(self.type_expr()) } else { None };
            self.expect(&TokenKind::Eq, "to initialize the accumulator");
            let init = self.expr();
            let accumulator = VarDecl { name: acc_name, ty: acc_ty, span: acc_start.to(init.span), init: Some(init) };
            self.expect(&TokenKind::Bar, "before the iterate body");
            let body = self.expr();
            self.expect(&TokenKind::RParen, "to close 'iterate'");
            let span = source.span.to(self.prev_span());
            return self.mk(
                ExprKind::Iterate {
                    source: Box::new(source),
                    iterators,
                    accumulator: Box::new(accumulator),
                    body: Box::new(body),
                    safe,
                },
                span,
            );
        }
        if is_iterator_name(&name.name) {
            self.bump();
            let iterators = self.iterator_vars(&TokenKind::Bar).unwrap_or_default();
            let body = self.expr();
            self.expect(&TokenKind::RParen, &format!("to close '{name}'"));
            let span = source.span.to(self.prev_span());
            return self.mk(
                ExprKind::Iterator { source: Box::new(source), name, iterators, body: Box::new(body), safe },
                span,
            );
        }
        let args = self.args();
        let span = source.span.to(self.prev_span());
        self.mk(ExprKind::Call { source: Some(Box::new(source)), name, args, arrow: true, safe, at_pre: false }, span)
    }

    /// Speculatively parses `v1 [: T], v2 [: T] <terminator>`. Restores the position if the
    /// tokens do not have that shape (the argument is then an implicit-iterator body).
    fn iterator_vars(&mut self, terminator: &TokenKind) -> Option<Vec<VarDecl>> {
        let save = self.pos;
        let save_diags = self.diags.len();
        let mut vars = Vec::new();
        loop {
            if !self.at_ident() {
                break;
            }
            let name = self.ident("an iterator variable");
            let ty = if self.at(&TokenKind::Colon) {
                self.bump();
                Some(self.type_expr())
            } else {
                None
            };
            let span = name.span.to(self.prev_span());
            vars.push(VarDecl { name, ty, init: None, span });
            if self.eat(terminator) {
                if self.diags.len() == save_diags {
                    return Some(vars);
                }
                break;
            }
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.pos = save;
        self.diags.truncate(save_diags);
        None
    }

    fn var_decl(&mut self, require_init: bool) -> VarDecl {
        let name = self.ident("a variable name");
        let ty = if self.eat(&TokenKind::Colon) { Some(self.type_expr()) } else { None };
        let init = if require_init {
            self.expect(&TokenKind::Eq, &format!("to initialize '{name}'"));
            Some(self.expr())
        } else {
            None
        };
        let span = name.span.to(self.prev_span());
        VarDecl { name, ty, init, span }
    }

    fn primary(&mut self) -> Expr {
        let start = self.span();
        let kind = self.peek().clone();
        match kind {
            TokenKind::Int(i) => {
                self.bump();
                self.mk(ExprKind::Int(i), start)
            }
            TokenKind::Real(r) => {
                self.bump();
                self.mk(ExprKind::Real(r), start)
            }
            TokenKind::Str(s) => {
                self.bump();
                self.mk(ExprKind::Str(s), start)
            }
            TokenKind::Star => {
                self.bump();
                self.mk(ExprKind::Unlimited, start)
            }
            TokenKind::Keyword(kw) => self.keyword_primary(kw, start),
            TokenKind::LParen => {
                self.bump();
                let mut e = self.expr();
                self.expect(&TokenKind::RParen, "to close the parenthesis");
                e.span = start.to(self.prev_span());
                e
            }
            TokenKind::Ident(_) if self.at_document_word() => {
                self.error(start, "expected an expression");
                self.error_expr(start)
            }
            TokenKind::Ident(name) => self.name_primary(&name, start),
            TokenKind::QuotedIdent(_) => self.path_primary(start),
            other => {
                self.error(start, format!("expected an expression, found {other}"));
                let recoverable = matches!(
                    other,
                    TokenKind::RParen
                        | TokenKind::RBrace
                        | TokenKind::RBracket
                        | TokenKind::Eof
                        | TokenKind::Comma
                        | TokenKind::Semi
                ) || self.at_document_word();
                if !recoverable {
                    self.bump();
                }
                self.error_expr(start)
            }
        }
    }

    fn keyword_primary(&mut self, kw: Keyword, start: Span) -> Expr {
        match kw {
            Keyword::True | Keyword::False => {
                self.bump();
                self.mk(ExprKind::Bool(kw == Keyword::True), start)
            }
            Keyword::Null => {
                self.bump();
                self.mk(ExprKind::Null, start)
            }
            Keyword::Invalid => {
                self.bump();
                self.mk(ExprKind::Invalid, start)
            }
            Keyword::SelfKw => {
                self.bump();
                self.mk(ExprKind::SelfRef, start)
            }
            Keyword::If => {
                self.bump();
                self.if_rest(start)
            }
            Keyword::Let => {
                self.bump();
                let mut vars = vec![self.var_decl(true)];
                while self.eat(&TokenKind::Comma) {
                    vars.push(self.var_decl(true));
                }
                self.expect(&TokenKind::Keyword(Keyword::In), "after the let variables");
                let body = self.expr();
                let span = start.to(body.span);
                self.mk(ExprKind::Let { vars, body: Box::new(body) }, span)
            }
            other => {
                self.error(start, format!("expected an expression, found '{}'", other.as_str()));
                // Leave closing keywords for the enclosing construct.
                if !matches!(other, Keyword::Then | Keyword::Else | Keyword::Elseif | Keyword::Endif | Keyword::In) {
                    self.bump();
                }
                self.error_expr(start)
            }
        }
    }

    /// Parses the remainder of `if c then a [elseif ...] else b endif` after `if`/`elseif`.
    fn if_rest(&mut self, start: Span) -> Expr {
        let cond = self.expr();
        self.expect(&TokenKind::Keyword(Keyword::Then), "after the 'if' condition");
        let then_branch = self.expr();
        let else_branch = if self.at_kw(Keyword::Elseif) {
            let s = self.span();
            self.bump();
            // The nested `if` shares the single closing `endif`.
            let nested = self.if_rest_no_endif(s);
            self.expect(&TokenKind::Keyword(Keyword::Endif), "to close 'if'");
            let span = start.to(self.prev_span());
            return self.mk(
                ExprKind::If {
                    cond: Box::new(cond),
                    then_branch: Box::new(then_branch),
                    else_branch: Box::new(nested),
                },
                span,
            );
        } else {
            self.expect(&TokenKind::Keyword(Keyword::Else), "('if' requires an 'else' branch)");
            self.expr()
        };
        self.expect(&TokenKind::Keyword(Keyword::Endif), "to close 'if'");
        let span = start.to(self.prev_span());
        self.mk(
            ExprKind::If {
                cond: Box::new(cond),
                then_branch: Box::new(then_branch),
                else_branch: Box::new(else_branch),
            },
            span,
        )
    }

    fn if_rest_no_endif(&mut self, start: Span) -> Expr {
        let cond = self.expr();
        self.expect(&TokenKind::Keyword(Keyword::Then), "after the 'elseif' condition");
        let then_branch = self.expr();
        let else_branch = if self.at_kw(Keyword::Elseif) {
            let s = self.span();
            self.bump();
            self.if_rest_no_endif(s)
        } else {
            self.expect(&TokenKind::Keyword(Keyword::Else), "('if' requires an 'else' branch)");
            self.expr()
        };
        let span = start.to(else_branch.span);
        self.mk(
            ExprKind::If {
                cond: Box::new(cond),
                then_branch: Box::new(then_branch),
                else_branch: Box::new(else_branch),
            },
            span,
        )
    }

    fn name_primary(&mut self, name: &str, start: Span) -> Expr {
        if let Some(kind) = CollectionKind::from_name(name) {
            match self.peek_at(1) {
                TokenKind::LBrace => {
                    self.bump();
                    return self.collection_literal(kind, None, start);
                }
                TokenKind::LParen => {
                    let ty = self.type_expr();
                    if self.at(&TokenKind::LBrace) {
                        let TypeExprKind::Collection(_, elem) = ty.kind else { unreachable!() };
                        return self.collection_literal(kind, Some(*elem), start);
                    }
                    let span = ty.span;
                    return self.mk(ExprKind::TypeLit(ty), span);
                }
                _ => {}
            }
        }
        if name == "Tuple" {
            match self.peek_at(1) {
                TokenKind::LBrace => {
                    self.bump();
                    self.bump();
                    let mut parts = vec![self.var_decl(true)];
                    while self.eat(&TokenKind::Comma) {
                        parts.push(self.var_decl(true));
                    }
                    self.expect(&TokenKind::RBrace, "to close the tuple literal");
                    let span = start.to(self.prev_span());
                    return self.mk(ExprKind::TupleLit(parts), span);
                }
                TokenKind::LParen => {
                    let ty = self.type_expr();
                    let span = ty.span;
                    return self.mk(ExprKind::TypeLit(ty), span);
                }
                _ => {}
            }
        }
        self.path_primary(start)
    }

    fn path_primary(&mut self, start: Span) -> Expr {
        let mut path = vec![self.ident("a name")];
        while self.at(&TokenKind::ColonColon) {
            self.bump();
            path.push(self.ident("a name after '::'"));
        }
        if self.at(&TokenKind::LParen) {
            if path.len() > 1 {
                self.error(start.to(self.prev_span()), "qualified operation calls are not supported");
            }
            let name = path.pop().unwrap();
            let args = self.args();
            let at_pre = self.at_pre();
            let span = start.to(self.prev_span());
            return self.mk(ExprKind::Call { source: None, name, args, arrow: false, safe: false, at_pre }, span);
        }
        let at_pre = self.at_pre();
        let span = start.to(self.prev_span());
        self.mk(ExprKind::Name { path, at_pre }, span)
    }

    /// Parses `{ items }` (the current token is `{`).
    fn collection_literal(&mut self, kind: CollectionKind, element_type: Option<TypeExpr>, start: Span) -> Expr {
        self.expect(&TokenKind::LBrace, "to start the collection literal");
        let mut items = Vec::new();
        if !self.at(&TokenKind::RBrace) {
            loop {
                let first = self.expr();
                if self.eat(&TokenKind::DotDot) {
                    let last = self.expr();
                    items.push(CollectionItem::Range(first, last));
                } else {
                    items.push(CollectionItem::Single(first));
                }
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(&TokenKind::RBrace, "to close the collection literal");
        let span = start.to(self.prev_span());
        self.mk(ExprKind::CollectionLit { kind, element_type, items }, span)
    }

    // ------------------------------------------------------------ types

    pub(crate) fn type_expr(&mut self) -> TypeExpr {
        let start = self.span();
        if let TokenKind::Ident(name) = self.peek().clone() {
            if let Some(kind) = CollectionKind::from_name(&name)
                && matches!(self.peek_at(1), TokenKind::LParen)
            {
                self.bump();
                self.bump();
                let elem = self.type_expr();
                self.expect(&TokenKind::RParen, "to close the collection type");
                return TypeExpr {
                    kind: TypeExprKind::Collection(kind, Box::new(elem)),
                    span: start.to(self.prev_span()),
                };
            }
            if name == "Tuple" && matches!(self.peek_at(1), TokenKind::LParen) {
                self.bump();
                self.bump();
                let mut parts = Vec::new();
                if !self.at(&TokenKind::RParen) {
                    loop {
                        let n = self.ident("a tuple part name");
                        self.expect(&TokenKind::Colon, "after the tuple part name");
                        parts.push((n, self.type_expr()));
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(&TokenKind::RParen, "to close the tuple type");
                return TypeExpr { kind: TypeExprKind::Tuple(parts), span: start.to(self.prev_span()) };
            }
        }
        if !self.at_ident() {
            let t = self.peek_token().clone();
            self.error(t.span, format!("expected a type, found {}", t.kind));
            return TypeExpr { kind: TypeExprKind::Error, span: t.span };
        }
        let mut path = vec![self.ident("a type name")];
        while self.eat(&TokenKind::ColonColon) {
            path.push(self.ident("a type name after '::'"));
        }
        TypeExpr { kind: TypeExprKind::Path(path), span: start.to(self.prev_span()) }
    }

    // ------------------------------------------------------------ documents

    fn document(&mut self) -> Document {
        let mut doc = Document::default();
        let mut loose = PackageDecl { path: Vec::new(), contexts: Vec::new(), span: self.span() };
        loop {
            if self.at_eof() {
                break;
            } else if self.at_word("import") {
                let imp = self.import();
                doc.imports.push(imp);
            } else if self.at_word("package") {
                let pkg = self.package();
                doc.packages.push(pkg);
            } else if self.at_word("context") {
                let ctx = self.context();
                loose.contexts.push(ctx);
            } else {
                let t = self.peek_token().clone();
                self.error(t.span, format!("expected 'import', 'package' or 'context', found {}", t.kind));
                self.bump();
                self.recover();
            }
        }
        if !loose.contexts.is_empty() {
            loose.span = loose.contexts[0].span.to(loose.contexts.last().unwrap().span);
            doc.packages.insert(0, loose);
        }
        doc
    }

    /// Skips tokens up to the next document-level keyword.
    fn recover(&mut self) {
        while !self.at_eof() && !self.at_document_word() {
            self.bump();
        }
    }

    fn import(&mut self) -> Import {
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
                self.recover();
                String::new()
            }
        };
        // Optional `::*` suffix of OCLinEcore-style imports.
        if self.eat(&TokenKind::ColonColon) {
            self.eat(&TokenKind::Star);
        }
        Import { alias, uri, span: start.to(self.prev_span()) }
    }

    fn package(&mut self) -> PackageDecl {
        let start = self.bump().span;
        let mut path = vec![self.ident("a package name")];
        while self.eat(&TokenKind::ColonColon) {
            path.push(self.ident("a package name"));
        }
        let mut contexts = Vec::new();
        loop {
            if self.at_word("endpackage") {
                self.bump();
                break;
            }
            if self.at_eof() {
                self.error(start, "missing 'endpackage'");
                break;
            }
            if self.at_word("context") {
                contexts.push(self.context());
            } else if self.at_word("package") || self.at_word("import") {
                let span = self.span();
                self.error(span, "missing 'endpackage' before this");
                break;
            } else {
                let t = self.peek_token().clone();
                self.error(t.span, format!("expected 'context' or 'endpackage', found {}", t.kind));
                self.bump();
                self.recover();
            }
        }
        PackageDecl { path, contexts, span: start.to(self.prev_span()) }
    }

    fn context(&mut self) -> ContextDecl {
        let start = self.bump().span;
        let self_name = if self.at_ident() && matches!(self.peek_at(1), TokenKind::Colon) {
            let n = self.ident("a variable name");
            self.bump();
            Some(n)
        } else {
            None
        };
        let has_self_name = self_name.is_some();
        let mut path = vec![self.ident("a classifier name")];
        while self.eat(&TokenKind::ColonColon) {
            path.push(self.ident("a name after '::'"));
        }
        let kind = if self.at(&TokenKind::LParen) {
            self.bump();
            let mut params = Vec::new();
            if !self.at(&TokenKind::RParen) {
                loop {
                    params.push(self.var_decl(false));
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
            }
            self.expect(&TokenKind::RParen, "to close the parameter list");
            let result = if self.eat(&TokenKind::Colon) { Some(self.type_expr()) } else { None };
            let constraints = self.constraints(&["pre", "post", "body"]);
            ContextKind::Operation { params, result, constraints }
        } else if self.at(&TokenKind::Colon) && path.len() > 1 {
            self.bump();
            let ty = Some(self.type_expr());
            let constraints = self.constraints(&["init", "derive"]);
            ContextKind::Property { ty, constraints }
        } else {
            let mut invariants = Vec::new();
            let mut definitions = Vec::new();
            loop {
                if self.at_word("inv") {
                    invariants.push(self.constraint());
                } else if self.at_word("def") || self.at_word("static") {
                    definitions.push(self.definition());
                } else if !self.end_of_context() {
                    let t = self.peek_token().clone();
                    self.error(t.span, format!("expected 'inv' or 'def', found {}", t.kind));
                    self.bump();
                    self.recover();
                } else {
                    break;
                }
            }
            ContextKind::Classifier { self_name, invariants, definitions }
        };
        if has_self_name && !matches!(kind, ContextKind::Classifier { .. }) {
            self.error(start, "a context variable can only be declared for classifier contexts");
        }
        ContextDecl { path, kind, span: start.to(self.prev_span()) }
    }

    fn end_of_context(&self) -> bool {
        self.at_eof() || ["context", "package", "endpackage", "import"].iter().any(|w| self.at_word(w))
    }

    fn constraints(&mut self, allowed: &[&str]) -> Vec<Constraint> {
        let mut out = Vec::new();
        loop {
            if allowed.iter().any(|w| self.at_word(w)) {
                out.push(self.constraint());
            } else if self.end_of_context() {
                break;
            } else {
                let t = self.peek_token().clone();
                let expected = allowed.iter().map(|w| format!("'{w}'")).collect::<Vec<_>>().join(", ");
                self.error(t.span, format!("expected {expected}, found {}", t.kind));
                self.bump();
                self.recover();
            }
        }
        out
    }

    fn constraint(&mut self) -> Constraint {
        let kw = self.bump();
        let kind = match &kw.kind {
            TokenKind::Ident(w) => match w.as_str() {
                "inv" => ConstraintKind::Inv,
                "pre" => ConstraintKind::Pre,
                "post" => ConstraintKind::Post,
                "body" => ConstraintKind::Body,
                "init" => ConstraintKind::Init,
                _ => ConstraintKind::Derive,
            },
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
        self.expect(&TokenKind::Colon, &format!("after '{}'", kind.keyword()));
        let expr = self.expr();
        self.check_expression_end();
        Constraint { kind, name, message, expr, span: kw.span.to(self.prev_span()) }
    }

    fn definition(&mut self) -> Definition {
        let start = self.span();
        let is_static = self.at_word("static");
        if is_static {
            self.bump();
        }
        if !self.at_word("def") {
            let span = self.span();
            self.error(span, "expected 'def' after 'static'");
        } else {
            self.bump();
        }
        // Optional definition name before the colon: `def myName: ...`.
        if self.at_ident() && matches!(self.peek_at(1), TokenKind::Colon) {
            self.bump();
        }
        self.expect(&TokenKind::Colon, "after 'def'");
        let name = self.ident("the name of the defined attribute or operation");
        let params = if self.at(&TokenKind::LParen) {
            self.bump();
            let mut params = Vec::new();
            if !self.at(&TokenKind::RParen) {
                loop {
                    params.push(self.var_decl(false));
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
            }
            self.expect(&TokenKind::RParen, "to close the parameter list");
            Some(params)
        } else {
            None
        };
        let ty = if self.eat(&TokenKind::Colon) { Some(self.type_expr()) } else { None };
        self.expect(&TokenKind::Eq, &format!("before the body of '{name}'"));
        let body = self.expr();
        self.check_expression_end();
        Definition { is_static, name, params, ty, body, span: start.to(self.prev_span()) }
    }

    /// After a constraint body only a document keyword (or the end) may follow.
    fn check_expression_end(&mut self) {
        if self.at_eof() || self.at_document_word() {
            return;
        }
        let t = self.peek_token().clone();
        self.error(t.span, format!("unexpected {} after the end of the expression", t.kind));
        self.recover();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(src: &str) -> Expr {
        let (e, d) = parse_expression(src);
        assert!(d.is_empty(), "{src}: {d:?}");
        e
    }

    fn sexpr(e: &Expr) -> String {
        match &e.kind {
            ExprKind::Int(i) => i.to_string(),
            ExprKind::Bool(b) => b.to_string(),
            ExprKind::Str(s) => format!("'{s}'"),
            ExprKind::Name { path, .. } => path.iter().map(|i| i.name.clone()).collect::<Vec<_>>().join("::"),
            ExprKind::SelfRef => "self".into(),
            ExprKind::Binary { op, lhs, rhs } => format!("({} {} {})", op.symbol(), sexpr(lhs), sexpr(rhs)),
            ExprKind::Unary { op, operand } => format!("({op:?} {})", sexpr(operand)),
            ExprKind::Property { source, name, .. } => format!("{}.{}", sexpr(source), name),
            ExprKind::Call { source, name, args, arrow, .. } => format!(
                "{}{}{}({})",
                source.as_ref().map(|s| sexpr(s)).unwrap_or_default(),
                if source.is_none() {
                    ""
                } else if *arrow {
                    "->"
                } else {
                    "."
                },
                name,
                args.iter().map(sexpr).collect::<Vec<_>>().join(", ")
            ),
            ExprKind::Iterator { source, name, iterators, body, .. } => format!(
                "{}->{}({}| {})",
                sexpr(source),
                name,
                iterators.iter().map(|v| v.name.name.clone() + " ").collect::<String>(),
                sexpr(body)
            ),
            ExprKind::If { cond, then_branch, else_branch } => {
                format!("(if {} {} {})", sexpr(cond), sexpr(then_branch), sexpr(else_branch))
            }
            ExprKind::Let { vars, body } => format!("(let {} {})", vars[0].name, sexpr(body)),
            other => format!("{other:?}"),
        }
    }

    #[test]
    fn precedence() {
        assert_eq!(
            sexpr(&parse_ok("a + b * c < d and not e or f implies g")),
            "(implies (or (and (< (+ a (* b c)) d) (Not e)) f) g)"
        );
        assert_eq!(sexpr(&parse_ok("1 - 2 - 3")), "(- (- 1 2) 3)");
        assert_eq!(sexpr(&parse_ok("-x.y")), "(Neg x.y)");
    }

    #[test]
    fn navigation_and_iterators() {
        assert_eq!(
            sexpr(&parse_ok("self.books->select(b | b.copies > 0)->size()")),
            "self.books->select(b | (> b.copies 0))->size()"
        );
        assert_eq!(sexpr(&parse_ok("books->forAll(copies > 0)")), "books->forAll(| (> copies 0))");
        assert_eq!(sexpr(&parse_ok("books->includes(x)")), "books->includes(x)");
        assert_eq!(sexpr(&parse_ok("Genre::FICTION")), "Genre::FICTION");
        assert_eq!(sexpr(&parse_ok("isAvailable()")), "isAvailable()");
    }

    #[test]
    fn literals_and_control() {
        parse_ok("Set{1, 2..5}->union(Sequence(Integer){})->asSet()");
        parse_ok("Tuple{a : Integer = 1, b = 'x'}.a");
        parse_ok("let x : Integer = 1, y = 2 in x + y");
        parse_ok("if a then 1 elseif b then 2 else 3 endif");
        parse_ok("x->iterate(e; acc : Integer = 0 | acc + e)");
        parse_ok("x.oclIsKindOf(Set(Integer)) and y.oclAsType(lib::Book).name <> null");
        parse_ok("a?.b?->size()");
    }

    #[test]
    fn reports_errors() {
        let (_, d) = parse_expression("a + ");
        assert_eq!(d.len(), 1);
        let (_, d) = parse_expression("if a then b endif");
        assert_eq!(d.len(), 1, "{d:?}");
        assert!(d[0].message.contains("else"), "{}", d[0].message);
    }

    #[test]
    fn document() {
        let src = r#"
            import 'library.ecore'
            package library
            context Book
            inv Positive('Copies must be positive'): copies > 0
            inv: true
            def: twice : Integer = copies * 2
            def: plus(n : Integer) : Integer = copies + n
            context b : Book inv: b.copies >= 0
            context Book::isAvailable() : Boolean
            pre: true
            body: copies > 0
            context Book::copies : Integer
            init: 1
            endpackage
            context library::Member inv: age > 0
        "#;
        let (doc, d) = parse_document(src);
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(doc.imports[0].uri, "library.ecore");
        assert_eq!(doc.packages.len(), 2);
        let lib = &doc.packages[1];
        assert_eq!(lib.contexts.len(), 4);
        let ContextKind::Classifier { invariants, definitions, .. } = &lib.contexts[0].kind else { panic!() };
        assert_eq!(invariants.len(), 2);
        assert_eq!(definitions.len(), 2);
        assert!(invariants[0].message.is_some());
        assert!(matches!(lib.contexts[2].kind, ContextKind::Operation { .. }));
        assert!(matches!(lib.contexts[3].kind, ContextKind::Property { .. }));
    }

    #[test]
    fn document_recovery() {
        let (doc, d) = parse_document("context A inv: a +\ninv B: b > \ncontext C inv: c");
        assert_eq!(d.len(), 2, "{d:?}");
        assert_eq!(doc.packages[0].contexts.len(), 2);
    }
}
