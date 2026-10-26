//! Imperative OCL and the expressions of QVT Operational Mappings: what the QVT dialect adds to
//! OCL expressions (the `ImperativeOCL.gi` and `QVTOParser.gi` grammars of Eclipse QVTo).
//!
//! Its keywords (`var`, `object`, `map`, `while`, ...) are not reserved: a word starts an
//! imperative expression only when the token after it fits (`object T {`, `map name(`), so
//! models may still have features with these names.

use super::Parser;
use crate::span::Span;
use crate::syntax::ast::*;
use crate::syntax::lexer::{Keyword, TokenKind};

/// Iterators of Imperative OCL (after `->`), besides OCL's.
pub const QVT_ITERATORS: &[&str] =
    &["xselect", "xcollect", "selectOne", "collectOne", "collectselect", "collectselectOne"];

/// The resolve operations of QVT Operational Mappings.
pub const RESOLVE_OPERATIONS: &[&str] = &[
    "resolve",
    "resolveone",
    "resolveIn",
    "resolveoneIn",
    "invresolve",
    "invresolveone",
    "invresolveIn",
    "invresolveoneIn",
];

fn is_name(t: &TokenKind) -> bool {
    matches!(t, TokenKind::Ident(_) | TokenKind::QuotedIdent(_))
}

impl Parser {
    /// `target := value` and `target += value` after `target` (right-associative).
    pub(super) fn assignment(&mut self, target: Expr) -> Expr {
        let add = match self.peek() {
            TokenKind::ColonEq => false,
            TokenKind::PlusEq => true,
            _ => return target,
        };
        let op = self.bump().span;
        let assignable = match &target.kind {
            ExprKind::Name { path, .. } => path.len() == 1,
            ExprKind::Property { .. } | ExprKind::Error => true,
            _ => false,
        };
        if !assignable {
            self.error(target.span.to(op), "only variables and properties can be assigned");
        }
        let value = self.expr();
        let span = target.span.to(value.span);
        self.mk(ExprKind::Assign { target: Box::new(target), value: Box::new(value), add }, span)
    }

    /// Whether the current word starts an Imperative OCL or QVT expression here.
    pub(super) fn at_imperative_word(&self) -> bool {
        let TokenKind::Ident(w) = self.peek() else { return false };
        let next = self.peek_at(1);
        match w.as_str() {
            "var" => is_name(next) || matches!(next, TokenKind::LParen),
            "object" => is_name(next) || matches!(next, TokenKind::Keyword(Keyword::SelfKw)),
            "map" | "xmap" | "new" => is_name(next),
            "while" | "compute" | "log" => matches!(next, TokenKind::LParen),
            "assert" => matches!(next, TokenKind::LParen) || is_name(next),
            "switch" | "do" | "try" => matches!(next, TokenKind::LBrace),
            "return" => true,
            "break" | "continue" => {
                matches!(next, TokenKind::Semi | TokenKind::RBrace | TokenKind::Eof | TokenKind::Keyword(_))
            }
            "late" => matches!(next, TokenKind::Ident(n) if RESOLVE_OPERATIONS.contains(&n.as_str())),
            "raise" => is_name(next) || matches!(next, TokenKind::Str(_)),
            "Dict" => matches!(next, TokenKind::LBrace),
            w if RESOLVE_OPERATIONS.contains(&w) => matches!(next, TokenKind::LParen),
            _ => false,
        }
    }

    /// An expression starting with an imperative word (see [`Parser::at_imperative_word`]).
    pub(super) fn imperative_primary(&mut self) -> Expr {
        let start = self.span();
        let word = match self.peek() {
            TokenKind::Ident(w) => w.clone(),
            _ => return self.error_expr(start),
        };
        if word == "late" || RESOLVE_OPERATIONS.contains(&word.as_str()) {
            return self.resolve(None, false, start);
        }
        self.bump();
        let kind = match word.as_str() {
            "var" => {
                let paren = self.eat(&TokenKind::LParen);
                let mut vars = vec![self.declarator()];
                while self.eat(&TokenKind::Comma) {
                    vars.push(self.declarator());
                }
                if paren {
                    self.expect(&TokenKind::RParen, "to close the variable declarations");
                }
                ExprKind::VarDecls(vars)
            }
            "object" => return self.object_expr(start),
            "map" | "xmap" => return self.map_call(None, false, word == "xmap", start),
            "new" => {
                let ty = self.plain_type_expr();
                let extent = self.extent();
                let args = self.args();
                ExprKind::New { ty, extent, args }
            }
            "while" => {
                self.expect(&TokenKind::LParen, "after 'while'");
                let init = if self.eat_word("var")
                    || (self.at_ident() && matches!(self.peek_at(1), TokenKind::Colon | TokenKind::ColonEq))
                {
                    let d = self.declarator();
                    self.expect(&TokenKind::Semi, "after the declaration of the loop variable");
                    Some(Box::new(d))
                } else {
                    None
                };
                let cond = self.expr();
                self.expect(&TokenKind::RParen, "to close the condition");
                let body = self.body();
                ExprKind::While { init, cond: Box::new(cond), body: Box::new(body) }
            }
            "compute" => {
                self.expect(&TokenKind::LParen, "after 'compute'");
                let var = self.declarator();
                self.expect(&TokenKind::RParen, "to close the declaration");
                let body = self.body();
                ExprKind::Compute { var: Box::new(var), body: Box::new(body) }
            }
            "log" => {
                let args = self.args();
                let when = if self.eat_word("when") { Some(Box::new(self.binary(0))) } else { None };
                ExprKind::Log { args, when }
            }
            "assert" => {
                let severity = if self.at_ident() { Some(self.ident("a severity")) } else { None };
                self.expect(&TokenKind::LParen, "before the asserted condition");
                let condition = self.expr();
                self.expect(&TokenKind::RParen, "after the asserted condition");
                let log = if self.eat_word("with") {
                    let s = self.span();
                    if self.at_word("log") && matches!(self.peek_at(1), TokenKind::LParen) {
                        Some(Box::new(self.imperative_primary()))
                    } else {
                        self.error(s, "expected 'log(...)' after 'with'");
                        None
                    }
                } else {
                    None
                };
                ExprKind::Assert { severity, condition: Box::new(condition), log }
            }
            "switch" => {
                self.expect(&TokenKind::LBrace, "after 'switch'");
                let mut cases = Vec::new();
                let mut default = None;
                loop {
                    if self.eat_word("case") {
                        self.expect(&TokenKind::LParen, "before the condition");
                        let c = self.expr();
                        self.expect(&TokenKind::RParen, "after the condition");
                        let b = self.case_body();
                        cases.push((c, b));
                    } else if self.eat(&TokenKind::LParen) {
                        let c = self.expr();
                        self.expect(&TokenKind::RParen, "after the condition");
                        self.expect(&TokenKind::Question, "after the condition");
                        let b = self.case_body();
                        cases.push((c, b));
                    } else if self.eat_kw(Keyword::Else) {
                        self.eat(&TokenKind::Question);
                        default = Some(Box::new(self.case_body()));
                    } else {
                        break;
                    }
                }
                self.expect(&TokenKind::RBrace, "to close 'switch'");
                ExprKind::Switch { cases, default }
            }
            "do" => return self.block_from(start),
            "return" => {
                let value = if matches!(self.peek(), TokenKind::Semi | TokenKind::RBrace | TokenKind::Eof) {
                    None
                } else {
                    Some(Box::new(self.expr()))
                };
                ExprKind::Return(value)
            }
            "break" => ExprKind::Break,
            "continue" => ExprKind::Continue,
            "try" => {
                self.block();
                while self.eat_word("except") {
                    if self.eat(&TokenKind::LParen) {
                        self.skip_balanced(&TokenKind::RParen);
                    }
                    self.block();
                }
                self.error(start.to(self.prev_span()), "exception handling ('try') is not supported");
                ExprKind::Error
            }
            "raise" => {
                self.primary();
                if self.at(&TokenKind::LParen) {
                    self.args();
                }
                self.error(start.to(self.prev_span()), "exceptions ('raise') are not supported");
                ExprKind::Error
            }
            "Dict" => {
                self.bump();
                self.skip_balanced(&TokenKind::RBrace);
                self.error(start.to(self.prev_span()), "dictionaries ('Dict') are not supported");
                ExprKind::Error
            }
            _ => ExprKind::Error,
        };
        let span = start.to(self.prev_span());
        self.mk(kind, span)
    }

    /// Skips tokens up to and including `close` (with nested brackets).
    fn skip_balanced(&mut self, close: &TokenKind) {
        let mut depth = 0usize;
        while !self.at_eof() {
            let t = self.bump().kind;
            match t {
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => {
                    if depth == 0 && std::mem::discriminant(&t) == std::mem::discriminant(close) {
                        return;
                    }
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
        }
    }

    /// `name [: T] [(:= | =) init]`.
    fn declarator(&mut self) -> VarDecl {
        let name = self.ident("a variable name");
        let ty = if self.eat(&TokenKind::Colon) { Some(self.type_expr()) } else { None };
        // `::=` (lexed as `::` `=`) initializes too.
        let colons = self.at(&TokenKind::ColonColon) && matches!(self.peek_at(1), TokenKind::Eq);
        if colons {
            self.bump();
        }
        let init = if self.eat(&TokenKind::ColonEq) || self.eat(&TokenKind::Eq) { Some(self.expr()) } else { None };
        let span = name.span.to(self.prev_span());
        VarDecl { name, ty, init, span }
    }

    /// `@extent` after a type.
    fn extent(&mut self) -> Option<Ident> {
        if self.eat(&TokenKind::At) { Some(self.ident("a model parameter after '@'")) } else { None }
    }

    /// `object [v :] [T][@extent] { ... }` after `object`.
    fn object_expr(&mut self, start: Span) -> Expr {
        let var = if self.at_ident() && matches!(self.peek_at(1), TokenKind::Colon) {
            let v = self.ident("a variable name");
            self.bump();
            Some(v)
        } else if self.at_kw(Keyword::SelfKw) && matches!(self.peek_at(1), TokenKind::Colon) {
            let t = self.bump();
            self.bump();
            Some(Ident { name: "self".into(), span: t.span })
        } else {
            None
        };
        let (ty, extent) = if self.at(&TokenKind::LBrace) {
            (None, None)
        } else {
            let t = self.plain_type_expr();
            (Some(t), self.extent())
        };
        let body = self.braced_statements("the object's body");
        let span = start.to(self.prev_span());
        self.mk(ExprKind::Object { var, ty, extent, body }, span)
    }

    /// `name(args)` or `T::name(args)` after `map`/`xmap`.
    fn map_call(&mut self, source: Option<Expr>, arrow: bool, strict: bool, start: Span) -> Expr {
        let mut name = vec![self.ident("a mapping name")];
        while self.eat(&TokenKind::ColonColon) {
            name.push(self.ident("a mapping name"));
        }
        let args = if self.at(&TokenKind::LParen) {
            self.args()
        } else {
            let span = self.span();
            self.error(span, "expected '(' after the mapping name");
            Vec::new()
        };
        let span = start.to(self.prev_span());
        self.mk(ExprKind::MapCall { source: source.map(Box::new), arrow, strict, name, args }, span)
    }

    /// `[late] resolve...(...)` at the current token.
    fn resolve(&mut self, source: Option<Expr>, arrow: bool, start: Span) -> Expr {
        let late = self.eat_word("late");
        let op = self.ident("a resolve operation");
        let one = op.name.contains("one");
        let inverse = op.name.starts_with("inv");
        let within = op.name.ends_with("In");
        self.expect(&TokenKind::LParen, &format!("after '{}'", op.name));
        let mut mapping = None;
        let mut var = None;
        let mut ty = None;
        let mut condition = None;
        let mut more = !self.at(&TokenKind::RParen);
        if within && more {
            let mut path = vec![self.ident("a mapping (Type::name)")];
            while self.eat(&TokenKind::ColonColon) {
                path.push(self.ident("a mapping name"));
            }
            mapping = Some(path);
            more = self.eat(&TokenKind::Comma);
        }
        if more {
            if self.at_ident() && matches!(self.peek_at(1), TokenKind::Colon) {
                var = Some(self.ident("a variable name"));
                self.bump();
            }
            if !self.at(&TokenKind::Bar) {
                ty = Some(self.type_expr());
            }
            if self.eat(&TokenKind::Bar) {
                condition = Some(Box::new(self.expr()));
            }
        }
        if within && mapping.is_none() {
            self.error(op.span, format!("'{}' needs a mapping (Type::name) as its first argument", op.name));
        }
        self.expect(&TokenKind::RParen, &format!("to close '{}'", op.name));
        let span = start.to(self.prev_span());
        self.mk(
            ExprKind::Resolve { source: source.map(Box::new), arrow, one, inverse, late, mapping, var, ty, condition },
            span,
        )
    }

    /// What Imperative OCL and QVT add after `.` and `->`, and the shorthand filters `coll[...]`
    /// and `coll![...]`. Gives the expression back (boxed) if nothing applies.
    pub(super) fn qvt_postfix(&mut self, expr: Expr) -> Result<Expr, Box<Expr>> {
        match self.peek() {
            TokenKind::LBracket => return Ok(self.filter(expr, false)),
            TokenKind::Bang if matches!(self.peek_at(1), TokenKind::LBracket) => {
                self.bump();
                return Ok(self.filter(expr, true));
            }
            TokenKind::Dot | TokenKind::Arrow => {}
            _ => return Err(Box::new(expr)),
        }
        let arrow = matches!(self.peek(), TokenKind::Arrow);
        let TokenKind::Ident(name) = self.peek_at(1).clone() else { return Err(Box::new(expr)) };
        let next = self.peek_at(2).clone();
        let start = expr.span;
        match name.as_str() {
            "map" | "xmap" if is_name(&next) => {
                self.bump();
                self.bump();
                Ok(self.map_call(Some(expr), arrow, name == "xmap", start))
            }
            "late" if matches!(&next, TokenKind::Ident(n) if RESOLVE_OPERATIONS.contains(&n.as_str())) => {
                self.bump();
                Ok(self.resolve(Some(expr), arrow, start))
            }
            n if RESOLVE_OPERATIONS.contains(&n) && matches!(next, TokenKind::LParen) => {
                self.bump();
                Ok(self.resolve(Some(expr), arrow, start))
            }
            "forEach" | "forOne" if arrow && matches!(next, TokenKind::LParen) => {
                self.bump();
                self.bump();
                self.bump();
                Ok(self.for_each(expr, name == "forOne"))
            }
            n if arrow && QVT_ITERATORS.contains(&n) && matches!(next, TokenKind::LParen) => {
                self.bump();
                let name = self.ident("an iterator");
                self.bump();
                if name.name.starts_with("collectselect")
                    && let Some(e) = self.collect_select(expr.clone(), name.clone())
                {
                    return Ok(e);
                }
                let iterators = self.iterator_vars(&TokenKind::Bar).unwrap_or_default();
                let body = self.expr();
                self.expect(&TokenKind::RParen, &format!("to close '{name}'"));
                let span = start.to(self.prev_span());
                Ok(self.mk(
                    ExprKind::Iterator { source: Box::new(expr), name, iterators, body: Box::new(body), safe: false },
                    span,
                ))
            }
            _ => Err(Box::new(expr)),
        }
    }

    /// `v; r = e [| cond])` after `->collectselect(`: an iterator with two variables, the
    /// second initialized by the collected expression; the body is the condition. `None` (and
    /// nothing consumed) for the other forms.
    fn collect_select(&mut self, source: Expr, name: Ident) -> Option<Expr> {
        if !(self.at_ident() && matches!(self.peek_at(1), TokenKind::Semi)) {
            return None;
        }
        let it = self.ident("an iterator variable");
        self.bump();
        let target = self.ident("a variable for the collected value");
        self.expect(&TokenKind::Eq, "after the variable");
        let value = self.expr();
        let condition = if self.eat(&TokenKind::Bar) {
            self.expr()
        } else {
            let span = self.prev_span();
            self.mk(ExprKind::Bool(true), span)
        };
        self.expect(&TokenKind::RParen, &format!("to close '{name}'"));
        let it_span = it.span;
        let target_span = target.span.to(value.span);
        let iterators = vec![
            VarDecl { name: it, ty: None, init: None, span: it_span },
            VarDecl { name: target, ty: None, init: Some(value), span: target_span },
        ];
        let span = source.span.to(self.prev_span());
        Some(self.mk(
            ExprKind::Iterator { source: Box::new(source), name, iterators, body: Box::new(condition), safe: false },
            span,
        ))
    }

    /// `source[v | cond]`, `source[cond]` or `source[T]` (`xselect`; with `!`: `selectOne`).
    fn filter(&mut self, source: Expr, one: bool) -> Expr {
        let open = self.bump().span;
        let iterators = self.iterator_vars(&TokenKind::Bar).unwrap_or_default();
        let body = self.expr();
        self.expect(&TokenKind::RBracket, "to close the filter");
        let name = Ident { name: if one { "selectOne" } else { "xselect" }.to_string(), span: open };
        let span = source.span.to(self.prev_span());
        self.mk(
            ExprKind::Iterator { source: Box::new(source), name, iterators, body: Box::new(body), safe: false },
            span,
        )
    }

    /// `v1, v2 [| cond]) { body }` after `->forEach(`.
    fn for_each(&mut self, source: Expr, one: bool) -> Expr {
        let mut iterators = Vec::new();
        if self.at_ident()
            && matches!(self.peek_at(1), TokenKind::Comma | TokenKind::Bar | TokenKind::RParen | TokenKind::Colon)
        {
            loop {
                let name = self.ident("an iterator variable");
                let ty = if self.eat(&TokenKind::Colon) { Some(self.type_expr()) } else { None };
                let span = name.span.to(self.prev_span());
                iterators.push(VarDecl { name, ty, init: None, span });
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        let condition = if self.eat(&TokenKind::Bar) || (iterators.is_empty() && !self.at(&TokenKind::RParen)) {
            Some(Box::new(self.expr()))
        } else {
            None
        };
        self.expect(&TokenKind::RParen, "to close the iterator variables");
        let body = self.body();
        let span = source.span.to(self.prev_span());
        self.mk(ExprKind::ForEach { source: Box::new(source), iterators, condition, body: Box::new(body), one }, span)
    }

    /// The imperative and the OCL `if` (after `if`): `if (c) { } elif (c) { } else { } [endif]`
    /// and `if c then e [elif c then e] [else e] endif`. Without an `else` it is a `Switch`.
    pub(super) fn qvt_if(&mut self, start: Span) -> Expr {
        let cond = self.binary(0);
        let mut cases = Vec::new();
        let mut default = None;
        if self.eat_kw(Keyword::Then) {
            let then = self.expr();
            cases.push((cond, then));
            while self.at_kw(Keyword::Elseif) || self.at_word("elif") {
                self.bump();
                let c = self.binary(0);
                self.expect(&TokenKind::Keyword(Keyword::Then), "after the 'elif' condition");
                let b = self.expr();
                cases.push((c, b));
            }
            if self.eat_kw(Keyword::Else) {
                default = Some(self.expr());
            }
            self.expect(&TokenKind::Keyword(Keyword::Endif), "to close 'if'");
        } else {
            let then = self.body();
            cases.push((cond, then));
            while self.at_kw(Keyword::Elseif) || self.at_word("elif") {
                self.bump();
                let c = self.binary(0);
                let b = self.body();
                cases.push((c, b));
            }
            if self.eat_kw(Keyword::Else) {
                default = Some(self.body());
            }
            self.eat_kw(Keyword::Endif);
        }
        let span = start.to(self.prev_span());
        if cases.len() == 1
            && default.is_some()
            && let (Some((cond, then_branch)), Some(else_branch)) = (cases.pop(), default.take())
        {
            return self.mk(
                ExprKind::If {
                    cond: Box::new(cond),
                    then_branch: Box::new(then_branch),
                    else_branch: Box::new(else_branch),
                },
                span,
            );
        }
        self.mk(ExprKind::Switch { cases, default: default.map(Box::new) }, span)
    }

    /// A block, or else a single expression (the body of `while`, an `if` branch, ...).
    fn body(&mut self) -> Expr {
        if self.at(&TokenKind::LBrace) { self.block() } else { self.expr() }
    }

    /// The statement of a `case`: a block, or an expression and `;`.
    fn case_body(&mut self) -> Expr {
        if self.at(&TokenKind::LBrace) {
            let b = self.block();
            self.eat(&TokenKind::Semi);
            return b;
        }
        let e = self.expr();
        self.expect(&TokenKind::Semi, "after the alternative");
        e
    }

    /// `{ e1; e2; ... }`.
    pub fn block(&mut self) -> Expr {
        let start = self.span();
        self.block_from(start)
    }

    fn block_from(&mut self, start: Span) -> Expr {
        let items = self.braced_statements("the block");
        let span = start.to(self.prev_span());
        self.mk(ExprKind::Block(items), span)
    }

    /// `{ statements }`.
    pub fn braced_statements(&mut self, what: &str) -> Vec<Expr> {
        if !self.expect(&TokenKind::LBrace, &format!("to start {what}")) {
            return Vec::new();
        }
        let items = self.statements();
        self.expect(&TokenKind::RBrace, &format!("to close {what}"));
        items
    }

    /// Expressions separated by `;` up to a `}` (not consumed), the end, or a document word.
    /// After an expression ending with `}` or `endif` the `;` may be left out.
    pub fn statements(&mut self) -> Vec<Expr> {
        let mut out = Vec::new();
        loop {
            while self.eat(&TokenKind::Semi) {}
            if self.at(&TokenKind::RBrace) || self.at_eof() || self.at_document_word() {
                return out;
            }
            let before = self.pos;
            let e = self.expr();
            let progressed = self.pos != before;
            out.push(e);
            if self.eat(&TokenKind::Semi) || self.at(&TokenKind::RBrace) {
                continue;
            }
            let ended_with_block = matches!(
                self.tokens[self.pos.saturating_sub(1)].kind,
                TokenKind::RBrace | TokenKind::Keyword(Keyword::Endif)
            );
            if ended_with_block && progressed {
                continue;
            }
            if self.at_eof() || self.at_document_word() {
                let span = self.prev_span();
                self.error(span, "expected ';' after the expression");
                return out;
            }
            let t = self.peek_token().clone();
            self.error(t.span, format!("expected ';' after the expression, found {}", t.kind));
            if !progressed {
                self.bump();
            }
            self.skip_imperative_statement();
        }
    }

    /// Skips to the end of the current statement: after the next `;`, or before a `}` that
    /// closes the enclosing block.
    fn skip_imperative_statement(&mut self) {
        let mut depth = 0usize;
        loop {
            match self.peek() {
                TokenKind::Eof => return,
                TokenKind::Semi if depth == 0 => {
                    self.bump();
                    return;
                }
                TokenKind::RBrace if depth == 0 => return,
                TokenKind::LBrace | TokenKind::LParen | TokenKind::LBracket => depth += 1,
                TokenKind::RBrace | TokenKind::RParen | TokenKind::RBracket => depth = depth.saturating_sub(1),
                _ if depth == 0 && self.at_document_word() => return,
                _ => {}
            }
            self.bump();
        }
    }

    /// QVT's types besides OCL's: `List(T)` (a `Sequence`); `Dict(K, V)` is not supported.
    pub(super) fn qvt_type(&mut self, name: &str, start: Span) -> Option<TypeExpr> {
        if !matches!(self.peek_at(1), TokenKind::LParen) {
            return None;
        }
        match name {
            "List" => {
                self.bump();
                self.bump();
                let elem = self.type_expr();
                self.expect(&TokenKind::RParen, "to close the list type");
                let span = start.to(self.prev_span());
                Some(TypeExpr { kind: TypeExprKind::Collection(CollectionKind::Sequence, Box::new(elem)), span })
            }
            "Dict" => {
                self.bump();
                self.bump();
                self.skip_balanced(&TokenKind::RParen);
                let span = start.to(self.prev_span());
                self.error(span, "dictionaries ('Dict') are not supported");
                Some(TypeExpr { kind: TypeExprKind::Error, span })
            }
            _ => None,
        }
    }

    /// A multiplicity after a type (`T[1]`, `T[?]`, `T[*]`, `T[0..*]`, ...): a type with an
    /// upper bound above one is a `Set` of it.
    pub(super) fn type_multiplicity(&mut self, t: TypeExpr) -> TypeExpr {
        if !self.at(&TokenKind::LBracket) {
            return t;
        }
        let mark = self.mark();
        self.bump();
        let many = match (self.peek().clone(), self.peek_at(1).clone()) {
            (TokenKind::Star | TokenKind::Plus, TokenKind::RBracket) => {
                self.bump();
                Some(true)
            }
            (TokenKind::Question, TokenKind::RBracket) => {
                self.bump();
                Some(false)
            }
            (TokenKind::Int(n), TokenKind::RBracket) => {
                self.bump();
                Some(n > 1)
            }
            (TokenKind::Int(_), TokenKind::DotDot) => {
                self.bump();
                self.bump();
                match self.peek().clone() {
                    TokenKind::Star => {
                        self.bump();
                        Some(true)
                    }
                    TokenKind::Int(n) => {
                        self.bump();
                        Some(n > 1)
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        let Some(many) = many.filter(|_| self.eat(&TokenKind::RBracket)) else {
            self.rewind(mark);
            return t;
        };
        if !many || matches!(t.kind, TypeExprKind::Collection(..)) {
            return t;
        }
        let span = t.span.to(self.prev_span());
        TypeExpr { kind: TypeExprKind::Collection(CollectionKind::Set, Box::new(t)), span }
    }
}
