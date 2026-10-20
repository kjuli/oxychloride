//! Static type checking of OCL expressions against a metamodel.
//!
//! The checker reports unknown names, unknown properties and operations, wrong argument
//! counts and type mismatches. It is deliberately lenient where OCL's static typing is
//! weaker than its dynamic semantics: anything typed [`Type::Error`] is accepted silently,
//! so one mistake does not cascade into many diagnostics.

use oxy_ecore::{Metamodel, PackageId};

use crate::diagnostic::Diagnostic;
use crate::env::{Environment, lookup_classifier, resolve_type};
use crate::span::Span;
use crate::syntax::ast::*;
use crate::types::{CollectionKind, Type, library_type};

struct Scope {
    vars: Vec<(String, Type)>,
    /// Element type of an implicit iterator (or the type of `self` in the outermost scope).
    implicit: Option<Type>,
}

pub(crate) struct Checker<'a> {
    mm: &'a Metamodel,
    env: &'a Environment,
    package: Option<PackageId>,
    scopes: Vec<Scope>,
    diags: Vec<Diagnostic>,
    types: Vec<(Span, Type)>,
    /// Whether `@pre` may be used (only in postconditions).
    at_pre_allowed: bool,
}

/// Outcome of looking up a library operation.
enum Lookup {
    Found(Type),
    BadArgs(String),
    NotFound,
}

impl<'a> Checker<'a> {
    pub(crate) fn new(mm: &'a Metamodel, env: &'a Environment, package: Option<PackageId>) -> Self {
        Checker {
            mm,
            env,
            package,
            scopes: vec![Scope { vars: Vec::new(), implicit: None }],
            diags: Vec::new(),
            types: Vec::new(),
            at_pre_allowed: false,
        }
    }

    pub(crate) fn allow_at_pre(&mut self) {
        self.at_pre_allowed = true;
    }

    pub(crate) fn bind_self(&mut self, ty: Type, alias: Option<&str>) {
        let scope = &mut self.scopes[0];
        scope.vars.push(("self".into(), ty.clone()));
        if let Some(a) = alias {
            scope.vars.push((a.into(), ty.clone()));
        }
        scope.implicit = Some(ty);
    }

    pub(crate) fn bind(&mut self, name: &str, ty: Type) {
        self.scopes.last_mut().unwrap().vars.push((name.into(), ty));
    }

    pub(crate) fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        std::mem::take(&mut self.diags)
    }

    pub(crate) fn take_types(&mut self) -> Vec<(Span, Type)> {
        std::mem::take(&mut self.types)
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diags.push(Diagnostic::error(span, message));
    }

    fn show(&self, t: &Type) -> String {
        t.display(self.mm).to_string()
    }

    /// Checks `e` and reports an error unless its type conforms to `expected`.
    pub(crate) fn expect(&mut self, e: &Expr, expected: &Type, what: &str) -> Type {
        let t = self.check(e);
        if !t.conforms_to(expected, self.mm) && !matches!(e.kind, ExprKind::Error) {
            let msg = format!(
                "{what} must be of type {}, but this expression is of type {}",
                self.show(expected),
                self.show(&t)
            );
            self.error(e.span, msg);
        }
        t
    }

    pub(crate) fn check(&mut self, e: &Expr) -> Type {
        if !self.at_pre_allowed
            && matches!(
                e.kind,
                ExprKind::Name { at_pre: true, .. }
                    | ExprKind::Property { at_pre: true, .. }
                    | ExprKind::Call { at_pre: true, .. }
            )
        {
            self.error(e.span, "'@pre' can only be used in postconditions");
        }
        let t = self.check_inner(e);
        self.types.push((e.span, t.clone()));
        t
    }

    fn lookup_var(&self, name: &str) -> Option<Type> {
        self.scopes.iter().rev().find_map(|s| s.vars.iter().rev().find(|(n, _)| n == name).map(|(_, t)| t.clone()))
    }

    fn check_inner(&mut self, e: &Expr) -> Type {
        match &e.kind {
            ExprKind::Bool(_) => Type::Boolean,
            ExprKind::Int(_) => Type::Integer,
            ExprKind::Real(_) => Type::Real,
            ExprKind::Str(_) => Type::String,
            ExprKind::Null => Type::OclVoid,
            ExprKind::Invalid => Type::OclInvalid,
            ExprKind::Unlimited => Type::UnlimitedNatural,
            ExprKind::Error => Type::Error,
            ExprKind::SelfRef => self.lookup_var("self").unwrap_or_else(|| {
                self.error(e.span, "'self' is not defined here (no context class)");
                Type::Error
            }),
            ExprKind::Name { path, .. } => self.resolve_name(path, e.span),
            ExprKind::TypeLit(te) => {
                let mut diags = Vec::new();
                let t = resolve_type(self.mm, self.package, te, &mut diags);
                self.diags.extend(diags);
                Type::Meta(t.into())
            }
            ExprKind::Property { source, name, .. } => {
                let src = self.check(source);
                match self.property_type(&src, &name.name) {
                    Some(t) => t,
                    None => {
                        let msg = self.unknown_property_message(&src, &name.name);
                        self.error(name.span, msg);
                        Type::Error
                    }
                }
            }
            ExprKind::Call { source: Some(source), name, args, arrow, .. } => {
                let src = self.check(source);
                let args: Vec<(Type, Span)> = args.iter().map(|a| (self.check(a), a.span)).collect();
                self.call_type(&src, name, &args, *arrow)
            }
            ExprKind::Call { source: None, name, args, .. } => {
                let args: Vec<(Type, Span)> = args.iter().map(|a| (self.check(a), a.span)).collect();
                self.implicit_call_type(name, &args)
            }
            ExprKind::Iterator { source, name, iterators, body, .. } => {
                let src = self.check(source);
                self.iterator_type(&src, name, iterators, body)
            }
            ExprKind::Iterate { source, iterators, accumulator, body, .. } => {
                let src = self.check(source);
                let elem = self.element_type(&src);
                self.scopes.push(Scope { vars: Vec::new(), implicit: None });
                for v in iterators {
                    let t = self.declared_or(v, elem.clone());
                    self.bind(&v.name.name, t);
                }
                let init = accumulator.init.as_ref().map_or(Type::Error, |i| self.check(i));
                let acc_ty = self.declared_or(accumulator, init.clone());
                if !init.conforms_to(&acc_ty, self.mm) {
                    let msg = format!(
                        "accumulator initializer of type {} does not conform to {}",
                        self.show(&init),
                        self.show(&acc_ty)
                    );
                    self.error(accumulator.span, msg);
                }
                self.bind(&accumulator.name.name, acc_ty.clone());
                self.expect(body, &acc_ty, "the iterate body");
                self.scopes.pop();
                acc_ty
            }
            ExprKind::Binary { op, lhs, rhs } => self.binary_type(*op, lhs, rhs, e.span),
            ExprKind::Unary { op, operand } => {
                let t = self.check(operand);
                match op {
                    UnOp::Not => {
                        if !t.conforms_to(&Type::Boolean, self.mm) {
                            let msg = format!("'not' expects a Boolean, found {}", self.show(&t));
                            self.error(operand.span, msg);
                        }
                        Type::Boolean
                    }
                    UnOp::Neg if t.is_numeric() || t.is_error() => {
                        if t == Type::UnlimitedNatural {
                            Type::Integer
                        } else {
                            t
                        }
                    }
                    UnOp::Neg => {
                        let msg = format!("unary '-' expects a number, found {}", self.show(&t));
                        self.error(operand.span, msg);
                        Type::Error
                    }
                }
            }
            ExprKind::If { cond, then_branch, else_branch } => {
                self.expect(cond, &Type::Boolean, "an 'if' condition");
                let a = self.check(then_branch);
                let b = self.check(else_branch);
                a.common_supertype(&b, self.mm)
            }
            ExprKind::Let { vars, body } => {
                self.scopes.push(Scope { vars: Vec::new(), implicit: None });
                for v in vars {
                    let init = v.init.as_ref().map_or(Type::Error, |i| self.check(i));
                    let t = self.declared_or(v, init.clone());
                    if !init.conforms_to(&t, self.mm) {
                        let msg = format!(
                            "initializer of '{}' has type {}, which does not conform to {}",
                            v.name,
                            self.show(&init),
                            self.show(&t)
                        );
                        self.error(v.span, msg);
                    }
                    self.bind(&v.name.name, t);
                }
                let t = self.check(body);
                self.scopes.pop();
                t
            }
            ExprKind::CollectionLit { kind, element_type, items } => {
                let declared = element_type.as_ref().map(|te| {
                    let mut diags = Vec::new();
                    let t = resolve_type(self.mm, self.package, te, &mut diags);
                    self.diags.extend(diags);
                    t
                });
                let mut elem: Option<Type> = None;
                for item in items {
                    let t = match item {
                        CollectionItem::Single(x) => self.check(x),
                        CollectionItem::Range(a, b) => {
                            self.expect(a, &Type::Integer, "a range bound");
                            self.expect(b, &Type::Integer, "a range bound");
                            Type::Integer
                        }
                    };
                    if let Some(d) = &declared
                        && !t.conforms_to(d, self.mm)
                    {
                        let span = match item {
                            CollectionItem::Single(x) => x.span,
                            CollectionItem::Range(a, b) => a.span.to(b.span),
                        };
                        let msg = format!("element of type {} does not conform to {}", self.show(&t), self.show(d));
                        self.error(span, msg);
                    }
                    elem = Some(match elem {
                        None => t,
                        Some(prev) => prev.common_supertype(&t, self.mm),
                    });
                }
                let elem = declared.or(elem).unwrap_or(Type::OclVoid);
                Type::collection(*kind, elem)
            }
            ExprKind::TupleLit(parts) => {
                let mut out = Vec::new();
                for p in parts {
                    let init = p.init.as_ref().map_or(Type::Error, |i| self.check(i));
                    out.push((p.name.name.clone(), self.declared_or(p, init)));
                }
                Type::tuple(out)
            }
        }
    }

    fn declared_or(&mut self, v: &VarDecl, fallback: Type) -> Type {
        match &v.ty {
            Some(te) => {
                let mut diags = Vec::new();
                let t = resolve_type(self.mm, self.package, te, &mut diags);
                self.diags.extend(diags);
                t
            }
            None => fallback,
        }
    }

    fn element_type(&self, src: &Type) -> Type {
        match src {
            Type::Collection(_, e) => (**e).clone(),
            other => other.clone(),
        }
    }

    // ---------------------------------------------------------------- names

    fn resolve_name(&mut self, path: &[Ident], span: Span) -> Type {
        let mm = self.mm;
        if let [ident] = path {
            let name = ident.name.as_str();
            if let Some(t) = self.lookup_var(name) {
                return t;
            }
            let implicit: Vec<Type> = self.scopes.iter().rev().filter_map(|s| s.implicit.clone()).collect();
            for src in &implicit {
                if let Some(t) = self.property_type(src, name) {
                    return t;
                }
            }
            if let Some(t) = library_type(name) {
                return Type::Meta(t.into());
            }
            if let Ok(c) = lookup_classifier(mm, self.package, &[name]) {
                return Type::Meta(Type::of_classifier(mm, c).into());
            }
            let msg = match implicit.first() {
                Some(t) if !t.is_error() => {
                    format!("unknown name '{name}' (not a variable, a property of {}, or a type)", self.show(t))
                }
                Some(_) => return Type::Error,
                None => format!("unknown name '{name}'"),
            };
            self.error(span, msg);
            return Type::Error;
        }
        let names: Vec<&str> = path.iter().map(|i| i.name.as_str()).collect();
        let (last, head) = names.split_last().unwrap();
        if let Ok(c) = lookup_classifier(mm, self.package, head)
            && mm.is_enum(c)
        {
            if mm.enum_literal_index(c, last).is_some() {
                return Type::Classifier(c);
            }
            self.error(span, format!("enumeration '{}' has no literal '{last}'", mm.classifier(c).name));
            return Type::Error;
        }
        match lookup_classifier(mm, self.package, &names) {
            Ok(c) => Type::Meta(Type::of_classifier(mm, c).into()),
            Err(e) => {
                self.error(span, e);
                Type::Error
            }
        }
    }

    // ---------------------------------------------------------------- properties

    pub(crate) fn property_type(&self, src: &Type, name: &str) -> Option<Type> {
        let mm = self.mm;
        match src {
            Type::Error => Some(Type::Error),
            Type::Classifier(c) if mm.is_class(*c) => {
                if let Some(f) = mm.lookup_feature(*c, name) {
                    let f = mm.feature(f);
                    return Some(Type::of_typed(mm, f.ty, &f.multiplicity));
                }
                self.env.find_def_attribute(mm, *c, name).map(|d| d.ty.clone())
            }
            Type::Tuple(parts) => parts.iter().find(|(n, _)| n == name).map(|(_, t)| t.clone()),
            Type::Meta(_) if name == "name" => Some(Type::String),
            Type::Collection(k, e) => {
                let t = self.property_type(e, name)?;
                Some(Type::collection(k.collect_kind(), t.flattened_element()))
            }
            _ => None,
        }
    }

    fn unknown_property_message(&self, src: &Type, name: &str) -> String {
        match src {
            Type::OclVoid => format!("cannot access '{name}' on null"),
            Type::Collection(_, e) if matches!(**e, Type::Classifier(_)) => {
                format!("unknown property '{name}' for elements of type {}", self.show(e))
            }
            _ => format!("unknown property '{name}' for type {}", self.show(src)),
        }
    }

    // ---------------------------------------------------------------- calls

    fn report(&mut self, lookup: Lookup, name: &Ident, src: &Type) -> Type {
        match lookup {
            Lookup::Found(t) => t,
            Lookup::BadArgs(msg) => {
                self.error(name.span, format!("'{}': {msg}", name.name));
                Type::Error
            }
            Lookup::NotFound => {
                let msg = format!("unknown operation '{}' for type {}", name.name, self.show(src));
                self.error(name.span, msg);
                Type::Error
            }
        }
    }

    fn call_type(&mut self, src: &Type, name: &Ident, args: &[(Type, Span)], arrow: bool) -> Type {
        if src.is_error() {
            return match self.oclany_op(src, &name.name, args) {
                Lookup::Found(t) => t,
                _ => Type::Error,
            };
        }
        if arrow {
            let coll = if src.is_collection() {
                src.clone()
            } else if *src == Type::OclVoid {
                Type::collection(CollectionKind::Set, Type::OclVoid)
            } else {
                Type::collection(CollectionKind::Set, src.clone())
            };
            let l = self.collection_op(&coll, &name.name, args);
            return self.report(l, name, &coll);
        }
        if let Type::Collection(k, e) = src {
            let element_wise = is_element_wise_type_op(&name.name, args.iter().map(|(t, _)| t));
            if !element_wise && let Lookup::Found(t) = self.oclany_op(src, &name.name, args) {
                return t;
            }
            // Implicit collect: `coll.op()` is `coll->collect(op())` (classic OCL).
            let mut l = self.user_op(e, &name.name, args);
            if matches!(l, Lookup::NotFound) && !e.is_collection() {
                l = self.library_op(e, &name.name, args);
            }
            if let Lookup::Found(t) = &l {
                return Type::collection(k.collect_kind(), t.flattened_element());
            }
            if !matches!(self.collection_op(src, &name.name, args), Lookup::NotFound) {
                self.error(name.span, format!("'{}' is a collection operation; use '->{}'", name.name, name.name));
                return Type::Error;
            }
            return self.report(l, name, src);
        }
        let mut l = self.user_op(src, &name.name, args);
        if matches!(l, Lookup::NotFound) {
            l = self.library_op(src, &name.name, args);
        }
        self.report(l, name, src)
    }

    fn implicit_call_type(&mut self, name: &Ident, args: &[(Type, Span)]) -> Type {
        let implicit: Vec<Type> = self.scopes.iter().rev().filter_map(|s| s.implicit.clone()).collect();
        for src in &implicit {
            let l = self.user_op(src, &name.name, args);
            if !matches!(l, Lookup::NotFound) {
                return self.report(l, name, src);
            }
        }
        // Library operations apply to the innermost implicit source (iterator element or self).
        match implicit.first() {
            Some(src) => {
                let src = src.clone();
                let l = self.library_op(&src, &name.name, args);
                self.report(l, name, &src)
            }
            None => {
                self.error(name.span, format!("unknown operation '{}'", name.name));
                Type::Error
            }
        }
    }

    /// Operations defined by the metamodel (EOperations) or by `def:`.
    fn user_op(&self, src: &Type, name: &str, args: &[(Type, Span)]) -> Lookup {
        let mm = self.mm;
        let Type::Classifier(c) = src else { return Lookup::NotFound };
        if !mm.is_class(*c) {
            return Lookup::NotFound;
        }
        let mut arity_mismatch = None;
        for op in mm.lookup_operations(*c, name) {
            let o = mm.operation(op);
            let params: Vec<Type> = o.params.iter().map(|p| Type::of_typed(mm, p.ty, &p.multiplicity)).collect();
            match self.check_args(args, &params) {
                Ok(()) => {
                    return Lookup::Found(if o.ty.is_some() {
                        Type::of_typed(mm, o.ty, &o.multiplicity)
                    } else {
                        Type::OclVoid
                    });
                }
                Err(e) => arity_mismatch = Some(e),
            }
        }
        for d in self.env.find_def_operations(mm, *c, name) {
            let params: Vec<Type> = d.params.iter().map(|(_, t)| t.clone()).collect();
            match self.check_args(args, &params) {
                Ok(()) => return Lookup::Found(d.ty.clone()),
                Err(e) => arity_mismatch = Some(e),
            }
        }
        arity_mismatch.map_or(Lookup::NotFound, Lookup::BadArgs)
    }

    fn check_args(&self, args: &[(Type, Span)], params: &[Type]) -> Result<(), String> {
        if args.len() != params.len() {
            return Err(format!("expects {} argument(s), but {} were given", params.len(), args.len()));
        }
        for (i, ((a, _), p)) in args.iter().zip(params).enumerate() {
            if !a.conforms_to(p, self.mm) {
                return Err(format!("argument {} has type {}, but {} is expected", i + 1, self.show(a), self.show(p)));
            }
        }
        Ok(())
    }

    fn library_op(&self, src: &Type, name: &str, args: &[(Type, Span)]) -> Lookup {
        let l = match src {
            Type::Integer | Type::Real | Type::UnlimitedNatural => self.numeric_op(src, name, args),
            Type::String => self.string_op(name, args),
            Type::Meta(t) => self.meta_op(t, name, args),
            _ => Lookup::NotFound,
        };
        match l {
            Lookup::NotFound => self.oclany_op(src, name, args),
            other => other,
        }
    }

    fn arity(&self, args: &[(Type, Span)], n: usize, result: Type) -> Lookup {
        if args.len() == n {
            Lookup::Found(result)
        } else {
            Lookup::BadArgs(format!("expects {n} argument(s), but {} were given", args.len()))
        }
    }

    fn typed(&self, args: &[(Type, Span)], params: &[Type], result: Type) -> Lookup {
        match self.check_args(args, params) {
            Ok(()) => Lookup::Found(result),
            Err(e) => Lookup::BadArgs(e),
        }
    }

    fn oclany_op(&self, src: &Type, name: &str, args: &[(Type, Span)]) -> Lookup {
        match name {
            "oclIsUndefined" | "oclIsInvalid" | "oclIsNew" => self.arity(args, 0, Type::Boolean),
            "oclIsKindOf" | "oclIsTypeOf" => match args {
                [(Type::Meta(_) | Type::Error, _)] => Lookup::Found(Type::Boolean),
                [_] => Lookup::BadArgs("expects a type argument".into()),
                _ => self.arity(args, 1, Type::Boolean),
            },
            "oclAsType" => match args {
                [(Type::Meta(t), _)] => Lookup::Found((**t).clone()),
                [(Type::Error, _)] => Lookup::Found(Type::Error),
                [_] => Lookup::BadArgs("expects a type argument".into()),
                _ => self.arity(args, 1, Type::Error),
            },
            "oclType" | "eClass" => self.arity(args, 0, Type::Meta(src.clone().into())),
            "oclAsSet" => self.arity(args, 0, Type::collection(CollectionKind::Set, src.clone())),
            "toString" => self.arity(args, 0, Type::String),
            "oclContainer" | "eContainer" => self.arity(args, 0, Type::OclAny),
            "oclContents" | "eContents" | "eAllContents" => {
                self.arity(args, 0, Type::collection(CollectionKind::Set, Type::OclAny))
            }
            _ => Lookup::NotFound,
        }
    }

    fn numeric_op(&self, src: &Type, name: &str, args: &[(Type, Span)]) -> Lookup {
        let num_result = |other: &Type| {
            if matches!((src, other), (Type::Integer | Type::UnlimitedNatural, Type::Integer | Type::UnlimitedNatural))
            {
                Type::Integer
            } else {
                Type::Real
            }
        };
        match name {
            "abs" => self.arity(args, 0, if *src == Type::Real { Type::Real } else { Type::Integer }),
            "floor" | "round" => self.arity(args, 0, Type::Integer),
            "max" | "min" => match args {
                [(t, _)] if t.is_numeric() || t.is_error() => Lookup::Found(num_result(t)),
                [_] => Lookup::BadArgs("expects a number".into()),
                _ => self.arity(args, 1, Type::Real),
            },
            "div" | "mod" => self.typed(args, &[Type::Integer], Type::Integer),
            "toInteger" => self.arity(args, 0, Type::Integer),
            "toReal" => self.arity(args, 0, Type::Real),
            _ => Lookup::NotFound,
        }
    }

    fn string_op(&self, name: &str, args: &[(Type, Span)]) -> Lookup {
        let s = Type::String;
        let i = Type::Integer;
        let seq = Type::collection(CollectionKind::Sequence, Type::String);
        match name {
            "size" => self.arity(args, 0, i),
            "concat" => self.typed(args, &[Type::String], s),
            "substring" => self.typed(args, &[i.clone(), i], s),
            "toInteger" => self.arity(args, 0, i),
            "toReal" => self.arity(args, 0, Type::Real),
            "toBoolean" => self.arity(args, 0, Type::Boolean),
            "toUpper" | "toUpperCase" | "toLower" | "toLowerCase" | "trim" => self.arity(args, 0, s),
            "indexOf" | "lastIndexOf" => self.typed(args, &[s], i),
            "at" => self.typed(args, &[i], s),
            "characters" => self.arity(args, 0, seq),
            "equalsIgnoreCase" | "startsWith" | "endsWith" | "matches" => self.typed(args, &[s], Type::Boolean),
            "replaceAll" | "replaceFirst" | "substituteAll" | "substituteFirst" => {
                self.typed(args, &[s.clone(), s.clone()], s)
            }
            "tokenize" => match args.len() {
                0 => Lookup::Found(seq),
                1 => self.typed(args, &[s], seq),
                _ => self.typed(args, &[s, Type::Boolean], seq),
            },
            _ => Lookup::NotFound,
        }
    }

    fn meta_op(&self, t: &Type, name: &str, args: &[(Type, Span)]) -> Lookup {
        match name {
            "allInstances" => self.arity(args, 0, Type::collection(CollectionKind::Set, t.clone())),
            _ => Lookup::NotFound,
        }
    }

    fn collection_op(&self, coll: &Type, name: &str, args: &[(Type, Span)]) -> Lookup {
        let mm = self.mm;
        let Type::Collection(kind, elem) = coll else { return Lookup::NotFound };
        let kind = *kind;
        let elem = (**elem).clone();
        let same = coll.clone();
        let any_coll = Type::collection(CollectionKind::Collection, Type::OclAny);
        match name {
            "size" => self.arity(args, 0, Type::Integer),
            "count" | "indexOf" => self.arity(args, 1, Type::Integer),
            "includes" | "excludes" => self.arity(args, 1, Type::Boolean),
            "includesAll" | "excludesAll" => self.typed(args, &[any_coll], Type::Boolean),
            "isEmpty" | "notEmpty" => self.arity(args, 0, Type::Boolean),
            "sum" | "max" | "min" => {
                if !(elem.is_numeric()
                    || elem.is_error()
                    || (name != "sum" && elem == Type::String)
                    || elem == Type::OclVoid)
                {
                    return Lookup::BadArgs(format!(
                        "requires numeric elements, but elements are {}",
                        self.show(&elem)
                    ));
                }
                self.arity(args, 0, elem)
            }
            "asSet" => self.arity(args, 0, Type::collection(CollectionKind::Set, elem)),
            "asOrderedSet" => self.arity(args, 0, Type::collection(CollectionKind::OrderedSet, elem)),
            "asSequence" => self.arity(args, 0, Type::collection(CollectionKind::Sequence, elem)),
            "asBag" => self.arity(args, 0, Type::collection(CollectionKind::Bag, elem)),
            "flatten" => self.arity(args, 0, Type::collection(kind, elem.flattened_element())),
            "including" | "append" | "prepend" => match args {
                [(t, _)] => Lookup::Found(Type::collection(kind, elem.common_supertype(t, mm))),
                _ => self.arity(args, 1, same),
            },
            "insertAt" => match args {
                [(i, _), (t, _)] if i.conforms_to(&Type::Integer, mm) => {
                    Lookup::Found(Type::collection(kind, elem.common_supertype(t, mm)))
                }
                [_, _] => Lookup::BadArgs("expects an Integer index".into()),
                _ => self.arity(args, 2, same),
            },
            "excluding" => self.arity(args, 1, same),
            "includingAll" | "union" => match args {
                [(Type::Collection(k2, e2), _)] => {
                    let result_kind = if name == "includingAll" { kind } else { union_kind(kind, *k2) };
                    Lookup::Found(Type::collection(result_kind, elem.common_supertype(e2, mm)))
                }
                [(Type::Error, _)] => Lookup::Found(Type::Error),
                [_] => Lookup::BadArgs("expects a collection".into()),
                _ => self.arity(args, 1, same),
            },
            "excludingAll" => self.typed(args, &[Type::collection(CollectionKind::Collection, Type::OclAny)], same),
            "intersection" => match args {
                [(Type::Collection(k2, _), _)] => {
                    let k = if kind.is_unique() || k2.is_unique() { CollectionKind::Set } else { CollectionKind::Bag };
                    Lookup::Found(Type::collection(k, elem))
                }
                [(Type::Error, _)] => Lookup::Found(Type::Error),
                [_] => Lookup::BadArgs("expects a collection".into()),
                _ => self.arity(args, 1, same),
            },
            "symmetricDifference" => self.arity(args, 1, Type::collection(CollectionKind::Set, elem)),
            "at" => self.typed(args, &[Type::Integer], elem),
            "first" | "last" | "any" => self.arity(args, 0, elem),
            "subSequence" | "subOrderedSet" => self.typed(args, &[Type::Integer, Type::Integer], same),
            "reverse" => self.arity(args, 0, same),
            "selectByKind" | "selectByType" => match args {
                [(Type::Meta(t), _)] => Lookup::Found(Type::collection(kind, (**t).clone())),
                [(Type::Error, _)] => Lookup::Found(Type::Error),
                [_] => Lookup::BadArgs("expects a type argument".into()),
                _ => self.arity(args, 1, same),
            },
            "product" => match args {
                [(Type::Collection(_, e2), _)] => Lookup::Found(Type::collection(
                    CollectionKind::Set,
                    Type::tuple(vec![("first".into(), elem), ("second".into(), (**e2).clone())]),
                )),
                _ => self.arity(args, 1, Type::Error),
            },
            "oclIsUndefined" | "oclIsInvalid" | "oclType" | "oclAsType" | "oclIsKindOf" | "oclIsTypeOf"
            | "toString" => self.oclany_op(coll, name, args),
            _ => Lookup::NotFound,
        }
    }

    // ---------------------------------------------------------------- iterators

    fn iterator_type(&mut self, src: &Type, name: &Ident, iterators: &[VarDecl], body: &Expr) -> Type {
        let mm = self.mm;
        let coll = match src {
            Type::Collection(..) => src.clone(),
            Type::Error => Type::collection(CollectionKind::Set, Type::Error),
            other => Type::collection(CollectionKind::Set, other.clone()),
        };
        let (kind, elem) = match &coll {
            Type::Collection(k, e) => (*k, (**e).clone()),
            _ => unreachable!(),
        };
        self.scopes.push(Scope { vars: Vec::new(), implicit: iterators.is_empty().then(|| elem.clone()) });
        for v in iterators {
            let t = self.declared_or(v, elem.clone());
            self.bind(&v.name.name, t);
        }
        let n = name.name.as_str();
        let body_ty = match n {
            "select" | "reject" | "forAll" | "exists" | "one" | "any" => {
                self.expect(body, &Type::Boolean, &format!("the body of '{n}'"))
            }
            _ => self.check(body),
        };
        self.scopes.pop();
        if iterators.len() > 1 && !matches!(n, "forAll" | "exists") {
            self.error(name.span, format!("'{n}' takes a single iterator variable"));
        }
        match n {
            "select" | "reject" => coll,
            "collect" => Type::collection(kind.collect_kind(), body_ty.flattened_element()),
            "collectNested" => Type::collection(kind.collect_kind(), body_ty),
            "forAll" | "exists" | "one" | "isUnique" => Type::Boolean,
            "any" => elem,
            "sortedBy" => {
                let k = if kind.is_unique() { CollectionKind::OrderedSet } else { CollectionKind::Sequence };
                Type::collection(k, elem)
            }
            "closure" => {
                let k = if kind.is_ordered() { CollectionKind::OrderedSet } else { CollectionKind::Set };
                let reached = body_ty.element().cloned().unwrap_or(body_ty);
                Type::collection(k, elem.common_supertype(&reached, mm))
            }
            _ => Type::Error,
        }
    }

    // ---------------------------------------------------------------- operators

    fn binary_type(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr, span: Span) -> Type {
        let mm = self.mm;
        match op {
            BinOp::And | BinOp::Or | BinOp::Xor | BinOp::Implies => {
                let what = format!("an operand of '{}'", op.symbol());
                self.expect(lhs, &Type::Boolean, &what);
                self.expect(rhs, &Type::Boolean, &what);
                Type::Boolean
            }
            BinOp::Eq | BinOp::Ne => {
                self.check(lhs);
                self.check(rhs);
                Type::Boolean
            }
            _ => {
                let a = self.check(lhs);
                let b = self.check(rhs);
                if a.is_error() || b.is_error() {
                    return match op {
                        BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => Type::Boolean,
                        _ => Type::Error,
                    };
                }
                let numeric = a.is_numeric() && b.is_numeric();
                let both_int = matches!(a, Type::Integer | Type::UnlimitedNatural)
                    && matches!(b, Type::Integer | Type::UnlimitedNatural);
                let result = match op {
                    BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                        let comparable = numeric
                            || (a == Type::String && b == Type::String)
                            || matches!((&a, &b), (Type::Classifier(x), Type::Classifier(y)) if x == y && mm.is_enum(*x));
                        comparable.then_some(Type::Boolean)
                    }
                    BinOp::Add if a == Type::String && b == Type::String => Some(Type::String),
                    BinOp::Add | BinOp::Mul if numeric => Some(if both_int { Type::Integer } else { Type::Real }),
                    BinOp::Sub if numeric => Some(if both_int { Type::Integer } else { Type::Real }),
                    BinOp::Sub if a.is_collection() && b.is_collection() => Some(a.clone()),
                    BinOp::Div if numeric => Some(Type::Real),
                    _ => None,
                };
                result.unwrap_or_else(|| {
                    let msg = format!(
                        "operator '{}' is not defined for {} and {}",
                        op.symbol(),
                        self.show(&a),
                        self.show(&b)
                    );
                    self.error(span, msg);
                    Type::Error
                })
            }
        }
    }
}

/// `coll.oclAsType(T)` / `oclIsKindOf(T)` / `oclIsTypeOf(T)` with a non-collection `T` is
/// applied to each element (classic Eclipse OCL semantics, relied upon by many metamodels);
/// other `OclAny` operations called with `.` on a collection apply to the collection itself.
pub(crate) fn is_element_wise_type_op<'t>(name: &str, mut args: impl Iterator<Item = &'t Type>) -> bool {
    matches!(name, "oclAsType" | "oclIsKindOf" | "oclIsTypeOf")
        && matches!(args.next(), Some(Type::Meta(t)) if !t.is_collection())
}

fn union_kind(a: CollectionKind, b: CollectionKind) -> CollectionKind {
    use CollectionKind::*;
    match (a, b) {
        (Set, Set) => Set,
        (OrderedSet, OrderedSet) => OrderedSet,
        (Sequence | OrderedSet, Sequence | OrderedSet) => Sequence,
        _ => Bag,
    }
}
