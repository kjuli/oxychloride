//! Static type checking of OCL expressions against a metamodel.
//!
//! The checker reports unknown names, unknown properties and operations, wrong argument
//! counts and type mismatches. It is deliberately lenient where OCL's static typing is
//! weaker than its dynamic semantics: anything typed [`Type::Error`] is accepted silently,
//! so one mistake does not cascade into many diagnostics.

use oxy_ecore::{ClassifierId, Metamodel, PackageId};

use crate::diagnostic::Diagnostic;
use crate::env::{Environment, PackageAlias, lookup_classifier_in, resolve_type_in};
use crate::host::CheckHost;
use crate::resolution::{Analysis, Property, Resolution, UserOperation};
use crate::span::Span;
use crate::syntax::ast::*;
use crate::types::{CollectionKind, Type, library_type};

struct Scope {
    vars: Vec<(String, Type)>,
    /// Element type of an implicit iterator (or the type of `self` in the outermost scope).
    implicit: Option<Type>,
    /// Whether operations can be called on the implicit source without a source (not on an
    /// object being populated, see `Evaluator::with_implicit`).
    operations: bool,
}

/// The type checker of OCL expressions. Languages built on OCL (QVT) use it with a
/// [`CheckHost`] and package aliases ([`Checker::for_language`]): they bind the variables of
/// their operations and check their bodies.
pub struct Checker<'a> {
    mm: &'a Metamodel,
    env: &'a Environment,
    package: Option<PackageId>,
    aliases: &'a [PackageAlias],
    host: Option<&'a dyn CheckHost>,
    scopes: Vec<Scope>,
    diags: Vec<Diagnostic>,
    types: Vec<(Span, Type)>,
    /// Types and resolutions by expression, for tools that translate OCL.
    analysis: Analysis,
    /// Whether `@pre` may be used (only in postconditions).
    at_pre_allowed: bool,
    /// The number of loops around the expression being checked (for `break`).
    loops: u32,
    /// The types of the `return`ed values, with their spans.
    returns: Vec<(Span, Type)>,
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
            aliases: &[],
            host: None,
            scopes: vec![Scope { vars: Vec::new(), implicit: None, operations: true }],
            diags: Vec::new(),
            types: Vec::new(),
            analysis: Analysis::default(),
            at_pre_allowed: false,
            loops: 0,
            returns: Vec::new(),
        }
    }

    /// A checker for the expressions of a language built on OCL: its names, operations and
    /// expressions come from `host`, its type names may use `aliases`.
    pub fn for_language(
        mm: &'a Metamodel,
        env: &'a Environment,
        host: &'a dyn CheckHost,
        aliases: &'a [PackageAlias],
    ) -> Self {
        Checker { aliases, host: Some(host), ..Checker::new(mm, env, None) }
    }

    pub(crate) fn allow_at_pre(&mut self) {
        self.at_pre_allowed = true;
    }

    pub fn metamodel(&self) -> &'a Metamodel {
        self.mm
    }

    /// Binds `self` (and an alias of it) in the outermost scope; `self`'s properties can then be
    /// used without `self.`.
    pub fn bind_self(&mut self, ty: Type, alias: Option<&str>) {
        let scope = &mut self.scopes[0];
        scope.vars.push(("self".into(), ty.clone()));
        if let Some(a) = alias {
            scope.vars.push((a.into(), ty.clone()));
        }
        scope.implicit = Some(ty);
    }

    /// Declares a variable in the innermost scope.
    pub fn bind(&mut self, name: &str, ty: Type) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.vars.push((name.into(), ty));
        }
    }

    /// Opens a scope; with `implicit`, the properties and operations of that type can be used
    /// without a source in it (like those of `self`).
    pub fn push_scope(&mut self, implicit: Option<Type>) {
        self.scopes.push(Scope { vars: Vec::new(), implicit, operations: true });
    }

    /// Opens the scope of an object being populated (`object`, a mapping's population): its
    /// properties can be used without a source, its operations cannot.
    pub fn push_object_scope(&mut self, object: Type) {
        self.scopes.push(Scope { vars: Vec::new(), implicit: Some(object), operations: false });
    }

    pub fn pop_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    pub fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        std::mem::take(&mut self.diags)
    }

    /// The types of the checked expressions, with their spans.
    pub fn take_types(&mut self) -> Vec<(Span, Type)> {
        std::mem::take(&mut self.types)
    }

    pub fn take_analysis(&mut self) -> Analysis {
        std::mem::take(&mut self.analysis)
    }

    /// The types of the values `return`ed since the last call, with their spans.
    pub fn take_returns(&mut self) -> Vec<(Span, Type)> {
        std::mem::take(&mut self.returns)
    }

    /// Resolves a type written in the source, reporting unknown names.
    pub fn resolve_type(&mut self, te: &TypeExpr) -> Type {
        let mut diags = Vec::new();
        let t = resolve_type_in(self.mm, self.package, self.aliases, te, &mut diags);
        self.diags.extend(diags);
        t
    }

    /// Resolves a classifier name (with the package aliases).
    pub fn lookup_classifier(&self, path: &[&str]) -> Result<ClassifierId, String> {
        lookup_classifier_in(self.mm, self.package, self.aliases, path)
    }

    /// Reports an error.
    pub fn report_error(&mut self, span: Span, message: impl Into<String>) {
        self.diags.push(Diagnostic::error(span, message));
    }

    /// Reports a warning.
    pub fn report_warning(&mut self, span: Span, message: impl Into<String>) {
        self.diags.push(Diagnostic::warning(span, message));
    }

    fn resolved(&mut self, e: &Expr, r: Resolution) {
        self.analysis.resolutions.insert(e.id, r);
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diags.push(Diagnostic::error(span, message));
    }

    fn show(&self, t: &Type) -> String {
        t.display(self.mm).to_string()
    }

    /// Checks `e` and reports an error unless its type conforms to `expected`.
    pub fn expect(&mut self, e: &Expr, expected: &Type, what: &str) -> Type {
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

    /// Checks an expression and returns its type.
    pub fn check(&mut self, e: &Expr) -> Type {
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
        let t = stacker::maybe_grow(crate::eval::RED_ZONE, crate::eval::STACK_CHUNK, || self.check_inner(e));
        self.types.push((e.span, t.clone()));
        self.analysis.types.insert(e.id, t.clone());
        t
    }

    /// The type of a variable in scope.
    pub fn lookup_var(&self, name: &str) -> Option<Type> {
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
            ExprKind::Name { path, .. } => {
                let (t, r) = self.resolve_name(path, e.span);
                if let Some(r) = r {
                    self.resolved(e, r);
                }
                t
            }
            ExprKind::TypeLit(te) => {
                let t = self.resolve_type(te);
                Type::Meta(t.into())
            }
            ExprKind::Property { source, name, .. } => {
                let src = self.check(source);
                match self.property(&src, &name.name) {
                    Some((t, property, collect)) => {
                        if let Some(property) = property {
                            self.resolved(e, Resolution::Property { property, collect });
                        }
                        t
                    }
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
                let (t, r) = self.call_type(&src, name, &args, *arrow);
                if let Some(r) = r {
                    self.resolved(e, r);
                }
                t
            }
            ExprKind::Call { source: None, name, args, .. } => {
                let args: Vec<(Type, Span)> = args.iter().map(|a| (self.check(a), a.span)).collect();
                let (t, r) = self.implicit_call_type(name, &args);
                if let Some(r) = r {
                    self.resolved(e, r);
                }
                t
            }
            ExprKind::Iterator { source, name, iterators, body, .. } => {
                let src = self.check(source);
                self.iterator_type(&src, name, iterators, body)
            }
            ExprKind::Iterate { source, iterators, accumulator, body, .. } => {
                let src = self.check(source);
                let elem = self.element_type(&src);
                self.scopes.push(Scope { vars: Vec::new(), implicit: None, operations: true });
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
                self.scopes.push(Scope { vars: Vec::new(), implicit: None, operations: true });
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
                let declared = element_type.as_ref().map(|te| self.resolve_type(te));
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
            _ => self.check_imperative(e),
        }
    }

    fn declared_or(&mut self, v: &VarDecl, fallback: Type) -> Type {
        match &v.ty {
            Some(te) => self.resolve_type(te),
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

    fn resolve_name(&mut self, path: &[Ident], span: Span) -> (Type, Option<Resolution>) {
        let mm = self.mm;
        if let [ident] = path {
            let name = ident.name.as_str();
            if let Some(t) = self.lookup_var(name) {
                return (t, Some(Resolution::Variable));
            }
            if let Some(t) = self.host.and_then(|h| h.variable(name)) {
                return (t, Some(Resolution::Variable));
            }
            let implicit: Vec<Type> = self.scopes.iter().rev().filter_map(|s| s.implicit.clone()).collect();
            for (depth, src) in implicit.iter().enumerate() {
                if let Some((t, property, _)) = self.property(src, name) {
                    return (t, property.map(|property| Resolution::ImplicitProperty { depth, property }));
                }
            }
            if let Some(t) = library_type(name) {
                return (Type::Meta(t.clone().into()), Some(Resolution::Type(t)));
            }
            if let Ok(c) = self.lookup_classifier(&[name]) {
                let t = Type::of_classifier(mm, c);
                return (Type::Meta(t.clone().into()), Some(Resolution::Type(t)));
            }
            let msg = match implicit.first() {
                Some(t) if !t.is_error() => {
                    format!("unknown name '{name}' (not a variable, a property of {}, or a type)", self.show(t))
                }
                Some(_) => return (Type::Error, None),
                None => format!("unknown name '{name}'"),
            };
            self.error(span, msg);
            return (Type::Error, None);
        }
        let names: Vec<&str> = path.iter().map(|i| i.name.as_str()).collect();
        let (last, head) = names.split_last().unwrap();
        if let Ok(c) = self.lookup_classifier(head)
            && mm.is_enum(c)
        {
            if let Some(literal) = mm.enum_literal_index(c, last) {
                return (Type::Classifier(c), Some(Resolution::EnumLiteral { enumeration: c, literal }));
            }
            self.error(span, format!("enumeration '{}' has no literal '{last}'", mm.classifier(c).name));
            return (Type::Error, None);
        }
        match self.lookup_classifier(&names) {
            Ok(c) => {
                let t = Type::of_classifier(mm, c);
                (Type::Meta(t.clone().into()), Some(Resolution::Type(t)))
            }
            Err(e) => {
                self.error(span, e);
                (Type::Error, None)
            }
        }
    }

    // ---------------------------------------------------------------- properties

    /// The type of property `name` of `src`, what it is (`None` when `src` is unknown), and
    /// whether it is collected from the elements of a collection.
    fn property(&self, src: &Type, name: &str) -> Option<(Type, Option<Property>, bool)> {
        let mm = self.mm;
        match src {
            Type::Error => Some((Type::Error, None, false)),
            Type::Classifier(c) if mm.is_class(*c) => {
                if let Some(f) = mm.lookup_feature(*c, name) {
                    let feature = mm.feature(f);
                    return Some((
                        Type::of_typed(mm, feature.ty, &feature.multiplicity),
                        Some(Property::Feature(f)),
                        false,
                    ));
                }
                match self.env.find_def_attribute(mm, *c, name) {
                    Some(d) => Some((
                        d.ty.clone(),
                        Some(Property::DefAttribute { class: d.class, name: d.name.clone() }),
                        false,
                    )),
                    None => self.host?.property(mm, src, name).map(|t| (t, None, false)),
                }
            }
            Type::Tuple(parts) => parts
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, t)| (t.clone(), Some(Property::TuplePart(name.to_string())), false)),
            Type::Meta(_) if name == "name" => Some((Type::String, Some(Property::TypeName), false)),
            Type::Model(_) => self.host?.property(mm, src, name).map(|t| (t, None, false)),
            Type::Collection(k, e) => {
                let (t, property, _) = self.property(e, name)?;
                Some((Type::collection(k.collect_kind(), t.flattened_element()), property, true))
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

    fn call_type(
        &mut self,
        src: &Type,
        name: &Ident,
        args: &[(Type, Span)],
        arrow: bool,
    ) -> (Type, Option<Resolution>) {
        let library = |collect: bool, t: Type| {
            let r = (!t.is_error()).then_some(Resolution::Library { collect });
            (t, r)
        };
        if src.is_error() {
            return match self.oclany_op(src, &name.name, args) {
                Lookup::Found(t) => (t, None),
                _ => (Type::Error, None),
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
            let mut l = self.collection_op(&coll, &name.name, args);
            if matches!(l, Lookup::NotFound)
                && let Some(found) = self.host_op(Some(&coll), &name.name, args)
            {
                l = found;
            }
            return library(false, self.report(l, name, &coll));
        }
        if let Type::Collection(k, e) = src {
            let element_wise = is_element_wise_type_op(&name.name, args.iter().map(|(t, _)| t));
            if !element_wise && let Lookup::Found(t) = self.oclany_op(src, &name.name, args) {
                return library(false, t);
            }
            // Implicit collect: `coll.op()` is `coll->collect(op())` (classic OCL).
            let (mut l, mut operation) = self.user_op(e, &name.name, args);
            if matches!(l, Lookup::NotFound)
                && let Some(found) = self.host_op(Some(e), &name.name, args)
            {
                l = found;
            }
            if matches!(l, Lookup::NotFound) && !e.is_collection() {
                l = self.library_op(e, &name.name, args);
                operation = None;
            }
            if let Lookup::Found(t) = &l {
                let t = Type::collection(k.collect_kind(), t.flattened_element());
                let r = match operation {
                    Some(operation) => Resolution::Operation { operation, implicit: None, collect: true },
                    None => Resolution::Library { collect: true },
                };
                return (t, Some(r));
            }
            if !matches!(self.collection_op(src, &name.name, args), Lookup::NotFound) {
                self.error(name.span, format!("'{}' is a collection operation; use '->{}'", name.name, name.name));
                return (Type::Error, None);
            }
            return (self.report(l, name, src), None);
        }
        let (l, operation) = self.user_op(src, &name.name, args);
        if let Some(operation) = operation {
            let t = self.report(l, name, src);
            return (t, Some(Resolution::Operation { operation, implicit: None, collect: false }));
        }
        if matches!(l, Lookup::NotFound) {
            if let Some(found) = self.host_op(Some(src), &name.name, args) {
                return (self.report(found, name, src), None);
            }
            let l = self.library_op(src, &name.name, args);
            return library(false, self.report(l, name, src));
        }
        (self.report(l, name, src), None)
    }

    /// An operation of the host (see [`CheckHost::operation`]).
    fn host_op(&self, src: Option<&Type>, name: &str, args: &[(Type, Span)]) -> Option<Lookup> {
        let types: Vec<Type> = args.iter().map(|(t, _)| t.clone()).collect();
        Some(match self.host?.operation(self.mm, src, name, &types)? {
            Ok(t) => Lookup::Found(t),
            Err(e) => Lookup::BadArgs(e),
        })
    }

    fn implicit_call_type(&mut self, name: &Ident, args: &[(Type, Span)]) -> (Type, Option<Resolution>) {
        let implicit: Vec<Type> =
            self.scopes.iter().rev().filter(|s| s.operations).filter_map(|s| s.implicit.clone()).collect();
        for (depth, src) in implicit.iter().enumerate() {
            let (l, operation) = self.user_op(src, &name.name, args);
            if !matches!(l, Lookup::NotFound) {
                let r = operation.map(|operation| Resolution::Operation {
                    operation,
                    implicit: Some(depth),
                    collect: false,
                });
                return (self.report(l, name, src), r);
            }
            if let Some(found) = self.host_op(Some(src), &name.name, args) {
                return (self.report(found, name, src), None);
            }
        }
        if let Some(found) = self.host_op(None, &name.name, args) {
            return (self.report(found, name, &Type::Error), None);
        }
        // Library operations apply to the innermost implicit source (iterator element or self).
        match implicit.first() {
            Some(src) => {
                let src = src.clone();
                let l = self.library_op(&src, &name.name, args);
                let t = self.report(l, name, &src);
                let r = (!t.is_error()).then_some(Resolution::Library { collect: false });
                (t, r)
            }
            None => {
                self.error(name.span, format!("unknown operation '{}'", name.name));
                (Type::Error, None)
            }
        }
    }

    /// Operations defined by the metamodel (EOperations) or by `def:`, and which one was
    /// found.
    fn user_op(&self, src: &Type, name: &str, args: &[(Type, Span)]) -> (Lookup, Option<UserOperation>) {
        let mm = self.mm;
        let Type::Classifier(c) = src else { return (Lookup::NotFound, None) };
        if !mm.is_class(*c) {
            return (Lookup::NotFound, None);
        }
        let mut arity_mismatch = None;
        for op in mm.lookup_operations(*c, name) {
            let o = mm.operation(op);
            let params: Vec<Type> = o.params.iter().map(|p| Type::of_typed(mm, p.ty, &p.multiplicity)).collect();
            match self.check_args(args, &params) {
                Ok(()) => {
                    let t = if o.ty.is_some() { Type::of_typed(mm, o.ty, &o.multiplicity) } else { Type::OclVoid };
                    return (Lookup::Found(t), Some(UserOperation::Operation(op)));
                }
                Err(e) => arity_mismatch = Some(e),
            }
        }
        for d in self.env.find_def_operations(mm, *c, name) {
            let params: Vec<Type> = d.params.iter().map(|(_, t)| t.clone()).collect();
            match self.check_args(args, &params) {
                Ok(()) => {
                    let operation = UserOperation::Def { class: d.class, name: d.name.clone(), arity: d.params.len() };
                    return (Lookup::Found(d.ty.clone()), Some(operation));
                }
                Err(e) => arity_mismatch = Some(e),
            }
        }
        (arity_mismatch.map_or(Lookup::NotFound, Lookup::BadArgs), None)
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
            _ => (CollectionKind::Set, Type::Error),
        };
        // `collectselect(i; r = e | cond)`: `r` is collected where `cond` holds.
        if let [it, target] = iterators
            && let Some(init) = &target.init
            && name.name.starts_with("collectselect")
        {
            self.scopes.push(Scope { vars: Vec::new(), implicit: None, operations: true });
            let t = self.declared_or(it, elem.clone());
            self.bind(&it.name.name, t);
            let value = self.check(init);
            self.bind(&target.name.name, value.clone());
            self.expect(body, &Type::Boolean, &format!("the condition of '{}'", name.name));
            self.scopes.pop();
            return if name.name == "collectselect" {
                Type::collection(kind.collect_kind(), value.flattened_element())
            } else {
                value
            };
        }
        self.scopes.push(Scope {
            vars: Vec::new(),
            implicit: iterators.is_empty().then(|| elem.clone()),
            operations: true,
        });
        for v in iterators {
            let t = self.declared_or(v, elem.clone());
            self.bind(&v.name.name, t);
        }
        let n = name.name.as_str();
        let body_ty = match n {
            "select" | "reject" | "forAll" | "exists" | "one" | "any" => {
                self.expect(body, &Type::Boolean, &format!("the body of '{n}'"))
            }
            // A condition, or a type to select the elements of.
            "xselect" | "selectOne" => {
                let t = self.check(body);
                if !matches!(t, Type::Meta(_)) && !t.conforms_to(&Type::Boolean, mm) {
                    let msg = format!("the body of '{n}' must be a Boolean or a type, but is {}", self.show(&t));
                    self.error(body.span, msg);
                }
                t
            }
            _ => self.check(body),
        };
        self.scopes.pop();
        if iterators.len() > 1 && !matches!(n, "forAll" | "exists") {
            self.error(name.span, format!("'{n}' takes a single iterator variable"));
        }
        match n {
            "select" | "reject" => coll,
            "xselect" => match body_ty {
                Type::Meta(t) => Type::collection(kind, (*t).clone()),
                _ => coll,
            },
            "selectOne" => match body_ty {
                Type::Meta(t) => (*t).clone(),
                _ => elem,
            },
            "xcollect" | "collectselect" => Type::collection(kind.collect_kind(), body_ty.flattened_element()),
            "collectOne" | "collectselectOne" => body_ty,
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

// ------------------------------------------------------------------ Imperative OCL

impl Checker<'_> {
    /// Imperative OCL and the expressions of QVT Operational Mappings.
    fn check_imperative(&mut self, e: &Expr) -> Type {
        let mm = self.mm;
        match &e.kind {
            ExprKind::Block(items) => {
                self.push_scope(None);
                let mut t = Type::OclVoid;
                for item in items {
                    t = self.check(item);
                }
                self.pop_scope();
                t
            }
            ExprKind::VarDecls(vars) => {
                let mut t = Type::OclVoid;
                for v in vars {
                    t = self.declare(v);
                }
                t
            }
            ExprKind::Assign { target, value, add } => {
                let value_ty = self.check(value);
                if let Some(target_ty) = self.assign_target(target) {
                    self.check_assignment(&target_ty, &value_ty, *add, value.span);
                }
                value_ty
            }
            ExprKind::While { init, cond, body } => {
                self.push_scope(None);
                let t = init.as_ref().map_or(Type::OclVoid, |v| self.declare(v));
                self.expect(cond, &Type::Boolean, "a 'while' condition");
                self.loops += 1;
                self.check(body);
                self.loops -= 1;
                self.pop_scope();
                t
            }
            ExprKind::ForEach { source, iterators, condition, body, .. } => {
                let src = self.check(source);
                let elem = self.element_type(&src);
                self.push_scope(iterators.is_empty().then(|| elem.clone()));
                for v in iterators {
                    let t = self.declared_or(v, elem.clone());
                    self.bind(&v.name.name, t);
                }
                if let Some(c) = condition {
                    self.expect(c, &Type::Boolean, "the condition of 'forEach'");
                }
                self.loops += 1;
                self.check(body);
                self.loops -= 1;
                self.pop_scope();
                Type::OclVoid
            }
            ExprKind::Switch { cases, default } => {
                let mut result: Option<Type> = None;
                for (c, b) in cases {
                    self.expect(c, &Type::Boolean, "a condition");
                    let t = self.check(b);
                    result = Some(match result {
                        None => t,
                        Some(r) => r.common_supertype(&t, mm),
                    });
                }
                if let Some(d) = default {
                    let t = self.check(d);
                    result = Some(match result {
                        None => t,
                        Some(r) => r.common_supertype(&t, mm),
                    });
                }
                result.unwrap_or(Type::OclVoid)
            }
            ExprKind::Return(value) => {
                let t = value.as_ref().map_or(Type::OclVoid, |v| self.check(v));
                self.returns.push((e.span, t));
                Type::OclVoid
            }
            ExprKind::Break | ExprKind::Continue => {
                if self.loops == 0 {
                    let word = if matches!(e.kind, ExprKind::Break) { "break" } else { "continue" };
                    self.error(e.span, format!("'{word}' outside of a loop"));
                }
                Type::OclVoid
            }
            ExprKind::Compute { var, body } => {
                self.push_scope(None);
                let t = self.declare(var);
                self.check(body);
                self.pop_scope();
                t
            }
            ExprKind::Log { args, when } => {
                if args.is_empty() || args.len() > 3 {
                    self.error(e.span, "'log' takes a message, and optionally a value and a level");
                }
                for a in args {
                    self.check(a);
                }
                if let Some(w) = when {
                    self.expect(w, &Type::Boolean, "the condition of 'log'");
                }
                Type::OclVoid
            }
            ExprKind::Assert { severity, condition, log } => {
                if let Some(s) = severity
                    && !matches!(s.name.as_str(), "warning" | "error" | "fatal")
                {
                    self.error(s.span, format!("unknown severity '{}' (warning, error or fatal)", s.name));
                }
                self.expect(condition, &Type::Boolean, "an assertion");
                if let Some(l) = log {
                    self.check(l);
                }
                Type::OclVoid
            }
            ExprKind::Object { var, ty, extent, body } => {
                let declared = ty.as_ref().map(|t| self.resolve_type(t));
                let existing = var.as_ref().and_then(|v| self.lookup_var(&v.name));
                let t = match (&declared, &existing) {
                    (Some(t), _) => t.clone(),
                    (None, Some(t)) => t.clone(),
                    (None, None) => {
                        let span = var.as_ref().map_or(e.span, |v| v.span);
                        self.error(span, "the type of the object is missing");
                        Type::Error
                    }
                };
                // An object in the variable already is updated, not created.
                let class = if existing.is_some() {
                    match &t {
                        Type::Classifier(c) => Some(*c),
                        _ => None,
                    }
                } else {
                    self.creatable(&t, ty.as_ref().map_or(e.span, |t| t.span), extent.is_none())
                };
                if let (Some(d), Some(x)) = (&declared, &existing)
                    && !d.conforms_to(x, mm)
                {
                    let msg = format!(
                        "an object of type {} cannot be assigned to a variable of type {}",
                        self.show(d),
                        self.show(x)
                    );
                    self.error(e.span, msg);
                }
                if let Some(x) = extent {
                    self.check_extent(x, class);
                }
                if let Some(v) = var
                    && existing.is_none()
                {
                    self.bind(&v.name, t.clone());
                }
                self.push_object_scope(t.clone());
                for item in body {
                    self.check(item);
                }
                self.pop_scope();
                t
            }
            ExprKind::New { ty, extent, args } => {
                let t = self.resolve_type(ty);
                let class = self.creatable(&t, ty.span, extent.is_none());
                if let Some(x) = extent {
                    self.check_extent(x, class);
                }
                for a in args {
                    self.check(a);
                }
                if !args.is_empty() {
                    self.error(e.span, "constructors with arguments are not supported");
                }
                t
            }
            ExprKind::MapCall { source, arrow, name, args, .. } => {
                let src = source.as_ref().map(|s| self.check(s));
                let args: Vec<Type> = args.iter().map(|a| self.check(a)).collect();
                let each = match &src {
                    Some(Type::Collection(_, e)) => Some((**e).clone()),
                    Some(t) if *arrow => Some(t.clone()),
                    other => other.clone(),
                };
                let result = match self.host.map(|h| h.mapping(mm, each.as_ref(), name, &args)) {
                    Some(Ok(t)) => t,
                    Some(Err(msg)) => {
                        let span = name.first().map_or(e.span, |n| n.span.to(name.last().map_or(n.span, |l| l.span)));
                        self.error(span, msg);
                        Type::Error
                    }
                    None => {
                        self.error(e.span, "mappings can only be called in transformations");
                        Type::Error
                    }
                };
                match &src {
                    Some(Type::Collection(k, _)) => Type::collection(k.collect_kind(), result),
                    Some(_) if *arrow => Type::collection(CollectionKind::Sequence, result),
                    _ => result,
                }
            }
            ExprKind::Resolve { source, one, mapping, var, ty, condition, .. } => {
                let src = source.as_ref().map(|s| self.check(s));
                if let Some(m) = mapping
                    && let Some(Err(msg)) = self.host.map(|h| h.mapping_reference(mm, m))
                {
                    let span = m[0].span.to(m[m.len() - 1].span);
                    self.error(span, msg);
                }
                let target = ty.as_ref().map_or(Type::OclAny, |t| self.resolve_type(t));
                if let Some(c) = condition {
                    self.push_scope(None);
                    if let Some(v) = var {
                        self.bind(&v.name, target.clone());
                    }
                    self.expect(c, &Type::Boolean, "the condition of a resolve");
                    self.pop_scope();
                }
                if *one && !matches!(src, Some(Type::Collection(..))) {
                    target
                } else {
                    Type::collection(CollectionKind::Sequence, target)
                }
            }
            _ => Type::Error,
        }
    }

    /// Declares a variable of `var`: its declared type, else the type of its initial value.
    fn declare(&mut self, v: &VarDecl) -> Type {
        let init = v.init.as_ref().map(|i| self.check(i));
        let t = match (&v.ty, &init) {
            (Some(_), _) => self.declared_or(v, Type::Error),
            (None, Some(t)) if *t == Type::OclVoid => Type::OclAny,
            (None, Some(t)) => t.clone(),
            (None, None) => {
                self.error(v.span, format!("the variable '{}' needs a type or an initial value", v.name));
                Type::Error
            }
        };
        if let Some(i) = &init
            && v.ty.is_some()
        {
            self.check_assignment(&t, i, false, v.span);
        }
        self.bind(&v.name.name, t.clone());
        t
    }

    /// The type of what an assignment assigns (`None` if it cannot be assigned, reported).
    fn assign_target(&mut self, target: &Expr) -> Option<Type> {
        match &target.kind {
            ExprKind::Name { path, .. } if path.len() == 1 => {
                let name = path[0].name.as_str();
                if let Some(t) = self.lookup_var(name) {
                    if name == "self" {
                        self.error(target.span, "'self' cannot be assigned");
                        return None;
                    }
                    self.types.push((target.span, t.clone()));
                    return Some(t);
                }
                if let Some(h) = self.host
                    && let Some(t) = h.variable(name)
                {
                    if !h.assignable(name) {
                        self.error(target.span, format!("'{name}' cannot be assigned"));
                        return None;
                    }
                    self.types.push((target.span, t.clone()));
                    return Some(t);
                }
                let implicit: Vec<Type> = self.scopes.iter().rev().filter_map(|s| s.implicit.clone()).collect();
                for src in &implicit {
                    if let Some((t, _, false)) = self.property(src, name) {
                        self.types.push((target.span, t.clone()));
                        return Some(t);
                    }
                }
                self.error(target.span, format!("unknown variable or property '{name}'"));
                None
            }
            ExprKind::Property { source, name, .. } => {
                let src = self.check(source);
                match self.property(&src, &name.name) {
                    Some((t, _, collect)) => {
                        if collect {
                            self.error(name.span, "cannot assign a property of the elements of a collection");
                            return None;
                        }
                        self.types.push((target.span, t.clone()));
                        Some(t)
                    }
                    None => {
                        let msg = self.unknown_property_message(&src, &name.name);
                        self.error(name.span, msg);
                        None
                    }
                }
            }
            ExprKind::Error => None,
            _ => {
                self.error(target.span, "only variables and properties can be assigned");
                None
            }
        }
    }

    /// Reports a value that does not fit what it is assigned to: many-valued targets take
    /// elements and collections of them (`+=` adds them), single-valued ones a single value.
    fn check_assignment(&mut self, target: &Type, value: &Type, add: bool, span: Span) {
        let mm = self.mm;
        let fits = match target {
            Type::Collection(_, elem) => {
                value.conforms_to(elem, mm) || value.element().is_some_and(|v| v.conforms_to(elem, mm))
            }
            Type::String if add => value.conforms_to(&Type::String, mm),
            t => value.conforms_to(t, mm),
        };
        if fits || value.is_error() || target.is_error() {
            return;
        }
        let msg = format!("a value of type {} cannot be assigned to {}", self.show(value), self.show(target));
        // OclAny may well hold a fitting value: the evaluator checks it.
        if *value == Type::OclAny || value.element() == Some(&Type::OclAny) {
            self.report_warning(span, msg);
        } else {
            self.error(span, msg);
        }
    }

    /// The class objects of type `t` are created of; reports types that cannot be created
    /// (`default_extent`: in the model the host chooses).
    fn creatable(&mut self, t: &Type, span: Span, default_extent: bool) -> Option<ClassifierId> {
        match t {
            Type::Classifier(c) if self.mm.is_class(*c) => {
                let data = self.mm.class_data(*c);
                if data.is_some_and(|d| d.is_abstract || d.is_interface) {
                    let msg = format!("{} is abstract: its objects cannot be created", self.mm.classifier(*c).name);
                    self.error(span, msg);
                }
                if default_extent && let Some(Err(msg)) = self.host.map(|h| h.can_create(self.mm, *c)) {
                    self.error(span, msg);
                }
                Some(*c)
            }
            Type::Error => None,
            other => {
                let msg = format!("only objects of classes can be created, not of {}", self.show(other));
                self.error(span, msg);
                None
            }
        }
    }

    fn check_extent(&mut self, extent: &Ident, class: Option<ClassifierId>) {
        if let Some(Err(msg)) = self.host.map(|h| h.extent(&extent.name, class)) {
            self.error(extent.span, msg);
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
