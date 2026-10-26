//! Lowering OCL syntax trees into code model expressions.
//!
//! The type checker has already decided what every name and call means and recorded it in
//! the source's [`Analysis`]; lowering makes that explicit: implicit `self` and implicit
//! iterator variables become references, `coll.name` becomes a `collect`, a single value used
//! with `->` becomes `oclAsSet()`, library operation names become canonical, and variables
//! get names that are unique within the body. Whatever cannot be translated becomes an
//! [`ExprKind::Invalid`] node, with a warning.

use std::collections::HashSet;

use oxy_ecore::{Metamodel, PackageId as EcorePackageId};
use oxy_ocl::resolution::{Property, Resolution, UserOperation};
use oxy_ocl::syntax::ast::{self, CollectionKind};
use oxy_ocl::{Analysis, Environment};

use crate::build::Index;
use crate::library::canonical_name;
use crate::model::*;
use crate::{Problem, Severity};

/// What an OCL variable name stands for.
#[derive(Clone, Debug)]
pub(crate) enum Binding {
    SelfRef,
    Result,
    Parameter(String),
    Var(String),
}

pub(crate) struct Lowerer<'a> {
    mm: &'a Metamodel,
    env: &'a Environment,
    index: &'a Index,
    analysis: Option<&'a Analysis>,
    /// The package unqualified type names are resolved in first.
    package: Option<EcorePackageId>,
    /// What is being lowered, for messages.
    what: &'a str,
    origin: &'a str,
    problems: &'a mut Vec<Problem>,
    /// Visible variables, innermost last.
    scope: Vec<(String, Binding)>,
    /// Implicit sources (`self`, iterators without declared variables), innermost last.
    implicit: Vec<(Binding, Ty)>,
    /// Names given to variables so far.
    used: HashSet<String>,
    counter: usize,
    reported: bool,
}

impl<'a> Lowerer<'a> {
    pub(crate) fn new(
        mm: &'a Metamodel,
        env: &'a Environment,
        index: &'a Index,
        body: &oxy_ocl::env::Body,
        what: &'a str,
        origin: &'a str,
        problems: &'a mut Vec<Problem>,
    ) -> Self {
        Lowerer {
            mm,
            env,
            index,
            analysis: env.analysis(body.source),
            package: body.package,
            what,
            origin,
            problems,
            scope: Vec::new(),
            implicit: Vec::new(),
            used: HashSet::new(),
            counter: 0,
            reported: false,
        }
    }

    pub(crate) fn bind_self(&mut self, ty: Ty) {
        self.scope.push(("self".into(), Binding::SelfRef));
        self.implicit.push((Binding::SelfRef, ty));
    }

    pub(crate) fn bind(&mut self, name: &str, binding: Binding) {
        if let Binding::Parameter(p) | Binding::Var(p) = &binding {
            self.used.insert(p.clone());
        }
        self.scope.push((name.to_string(), binding));
    }

    /// Reports a construct that cannot be translated and returns an `invalid` node.
    fn unsupported(&mut self, reason: impl Into<String>) -> Expr {
        let reason = reason.into();
        // One warning per body is enough: the body evaluates to invalid anyway.
        if !self.reported {
            self.reported = true;
            self.problems.push(Problem {
                severity: Severity::Warning,
                message: format!("{}: {reason}; this part evaluates to invalid", self.what),
                origin: Some(self.origin.to_string()),
            });
        }
        Expr::invalid(reason)
    }

    fn ty_of(&self, e: &ast::Expr) -> Ty {
        match self.analysis.and_then(|a| a.type_of(e.id)) {
            Some(t) => self.index.ocl_ty(self.mm, t),
            None => Ty::Invalid,
        }
    }

    fn resolution(&self, e: &ast::Expr) -> Option<&'a Resolution> {
        self.analysis.and_then(|a| a.resolution(e.id))
    }

    /// A fresh, unique variable name based on `name` (`_` for implicit iterators).
    fn fresh(&mut self, name: &str) -> String {
        let candidate = if name == "_" {
            loop {
                self.counter += 1;
                let n = format!("_{}", self.counter);
                if !self.used.contains(&n) {
                    break n;
                }
            }
        } else if !self.used.contains(name) && name != "self" && name != "result" {
            name.to_string()
        } else {
            (2..).map(|i| format!("{name}_{i}")).find(|n| !self.used.contains(n)).unwrap()
        };
        self.used.insert(candidate.clone());
        candidate
    }

    fn reference(&self, binding: &Binding, ty: Ty) -> Expr {
        let kind = match binding {
            Binding::SelfRef => ExprKind::SelfRef,
            Binding::Result => ExprKind::Result,
            Binding::Parameter(p) => ExprKind::Parameter(p.clone()),
            Binding::Var(v) => ExprKind::Var(v.clone()),
        };
        Expr::new(kind, ty)
    }

    /// The implicit source of the given depth (0: innermost).
    fn implicit_source(&mut self, depth: usize) -> Expr {
        match self.implicit.len().checked_sub(depth + 1).map(|i| self.implicit[i].clone()) {
            Some((binding, ty)) => self.reference(&binding, ty),
            None => self.unsupported("an implicit source is missing"),
        }
    }

    fn lookup(&self, name: &str) -> Option<Binding> {
        self.scope.iter().rev().find(|(n, _)| n == name).map(|(_, b)| b.clone())
    }

    /// Binds a variable declared in OCL as `name` and returns its unique name.
    fn declare(&mut self, name: &str) -> String {
        let unique = self.fresh(name);
        self.scope.push((name.to_string(), Binding::Var(unique.clone())));
        unique
    }

    pub(crate) fn lower(&mut self, e: &ast::Expr) -> Expr {
        use ast::ExprKind as K;
        let ty = self.ty_of(e);
        match &e.kind {
            K::Bool(b) => Expr::new(ExprKind::Boolean(*b), Ty::Boolean),
            K::Int(i) => Expr::new(ExprKind::Integer(*i), Ty::Integer),
            K::Real(r) => Expr::new(ExprKind::Real(*r), Ty::Real),
            K::Str(s) => Expr::new(ExprKind::String(s.clone()), Ty::String),
            K::Null => Expr::new(ExprKind::Null, Ty::Void),
            K::Unlimited => Expr::new(ExprKind::Unlimited, Ty::UnlimitedNatural),
            K::Invalid => Expr::new(ExprKind::Invalid("invalid".into()), Ty::Invalid),
            K::Error => self.unsupported("the expression has syntax errors"),
            K::SelfRef => Expr::new(ExprKind::SelfRef, ty),
            K::TypeLit(_) => match ty {
                Ty::Type(t) => Expr::new(ExprKind::Type((*t).clone()), Ty::Type(t)),
                _ => self.unsupported("unknown type"),
            },
            K::Name { path, .. } => self.name(e, path, ty),
            K::Property { source, name, safe, .. } => {
                let Some(Resolution::Property { property, collect }) = self.resolution(e) else {
                    return self.unsupported(format!("unknown property '{}'", name.name));
                };
                let src = self.lower(source);
                if *collect {
                    let src = if *safe { self.excluding_null(src) } else { src };
                    self.collect(src, ty, |l, var| l.property(var, property, false))
                } else {
                    self.property(src, property, *safe)
                }
            }
            K::Call { source, name, args, arrow, safe, .. } => {
                self.call(e, source.as_deref(), name, args, *arrow, *safe, ty)
            }
            K::Iterator { source, name, iterators, body, safe } => {
                let src = self.lower(source);
                let src = self.as_collection(src);
                let src = if *safe { self.excluding_null(src) } else { src };
                let Some(kind) = IteratorKind::from_name(&name.name) else {
                    return self.unsupported(format!("unknown iterator '{}'", name.name));
                };
                let element = src.ty.element().cloned().unwrap_or(Ty::Invalid);
                let mark = self.scope.len();
                let implicit = iterators.is_empty();
                let vars: Vec<Var> = if implicit {
                    let n = self.fresh("_");
                    self.implicit.push((Binding::Var(n.clone()), element.clone()));
                    vec![Var { name: n, ty: element.clone() }]
                } else {
                    iterators
                        .iter()
                        .map(|v| {
                            let t = self.declared_ty(v).unwrap_or_else(|| element.clone());
                            Var { name: self.declare(&v.name.name), ty: t }
                        })
                        .collect()
                };
                let body = self.lower(body);
                self.scope.truncate(mark);
                if implicit {
                    self.implicit.pop();
                }
                Expr::new(ExprKind::Iterator { kind, source: Box::new(src), vars, body: Box::new(body) }, ty)
            }
            K::Iterate { source, iterators, accumulator, body, safe } => {
                let src = self.lower(source);
                let src = self.as_collection(src);
                let src = if *safe { self.excluding_null(src) } else { src };
                let element = src.ty.element().cloned().unwrap_or(Ty::Invalid);
                let mark = self.scope.len();
                let var = match iterators.first() {
                    Some(v) => {
                        let t = self.declared_ty(v).unwrap_or_else(|| element.clone());
                        Var { name: self.declare(&v.name.name), ty: t }
                    }
                    None => Var { name: self.fresh("_"), ty: element },
                };
                let init = match &accumulator.init {
                    Some(i) => self.lower(i),
                    None => Expr::new(ExprKind::Null, Ty::Void),
                };
                let acc_ty = self.declared_ty(accumulator).unwrap_or_else(|| init.ty.clone());
                let acc = Var { name: self.declare(&accumulator.name.name), ty: acc_ty };
                let body = self.lower(body);
                self.scope.truncate(mark);
                Expr::new(
                    ExprKind::Iterate { source: Box::new(src), var, acc, init: Box::new(init), body: Box::new(body) },
                    ty,
                )
            }
            K::Binary { op, lhs, rhs } => {
                let (l, r) = (self.lower(lhs), self.lower(rhs));
                Expr::new(ExprKind::Binary { op: *op, lhs: Box::new(l), rhs: Box::new(r) }, ty)
            }
            K::Unary { op, operand } => {
                let o = self.lower(operand);
                Expr::new(ExprKind::Unary { op: *op, operand: Box::new(o) }, ty)
            }
            K::If { cond, then_branch, else_branch } => {
                let c = self.lower(cond);
                let t = self.lower(then_branch);
                let f = self.lower(else_branch);
                Expr::new(ExprKind::If { cond: Box::new(c), then_branch: Box::new(t), else_branch: Box::new(f) }, ty)
            }
            K::Let { vars, body } => {
                let mark = self.scope.len();
                let mut lets = Vec::new();
                for v in vars {
                    let init = match &v.init {
                        Some(i) => self.lower(i),
                        None => Expr::new(ExprKind::Null, Ty::Void),
                    };
                    let t = self.declared_ty(v).unwrap_or_else(|| init.ty.clone());
                    lets.push((Var { name: self.declare(&v.name.name), ty: t }, init));
                }
                let mut out = self.lower(body);
                self.scope.truncate(mark);
                for (var, init) in lets.into_iter().rev() {
                    let t = out.ty.clone();
                    out = Expr::new(ExprKind::Let { var, init: Box::new(init), body: Box::new(out) }, t);
                }
                out
            }
            K::CollectionLit { kind, items, .. } => {
                let items = items
                    .iter()
                    .map(|i| match i {
                        ast::CollectionItem::Single(x) => CollectionItem::Single(self.lower(x)),
                        ast::CollectionItem::Range(a, b) => CollectionItem::Range(self.lower(a), self.lower(b)),
                    })
                    .collect();
                Expr::new(ExprKind::CollectionLiteral { kind: *kind, items }, ty)
            }
            K::TupleLit(parts) => {
                let mut out: Vec<(String, Expr)> = parts
                    .iter()
                    .map(|p| {
                        let v = match &p.init {
                            Some(i) => self.lower(i),
                            None => Expr::new(ExprKind::Null, Ty::Void),
                        };
                        (p.name.name.clone(), v)
                    })
                    .collect();
                out.sort_by(|a, b| a.0.cmp(&b.0));
                Expr::new(ExprKind::TupleLiteral(out), ty)
            }
            // Imperative OCL appears in QVT transformations only, never in metamodels.
            _ => self.unsupported("Imperative OCL"),
        }
    }

    /// The declared type of a variable, if it has one.
    fn declared_ty(&self, v: &ast::VarDecl) -> Option<Ty> {
        let te = v.ty.as_ref()?;
        let t = self.env.resolve_type(self.mm, self.package, te);
        Some(self.index.ocl_ty(self.mm, &t))
    }

    fn name(&mut self, e: &ast::Expr, path: &[ast::Ident], ty: Ty) -> Expr {
        match self.resolution(e) {
            Some(Resolution::Variable) => {
                let name = &path[0].name;
                match self.lookup(name) {
                    Some(b) => self.reference(&b, ty),
                    None => self.unsupported(format!("unknown variable '{name}'")),
                }
            }
            Some(Resolution::ImplicitProperty { depth, property }) => {
                let src = self.implicit_source(*depth);
                self.property(src, property, false)
            }
            Some(Resolution::Type(t)) => {
                let t = self.index.ocl_ty(self.mm, t);
                Expr::new(ExprKind::Type(t.clone()), Ty::Type(Box::new(t)))
            }
            Some(Resolution::EnumLiteral { enumeration, literal }) => match self.index.enums.get(enumeration) {
                Some(id) => Expr::new(ExprKind::EnumLiteral(*id, *literal), Ty::Enum(*id)),
                None => self.unsupported("an enumeration that is not generated"),
            },
            _ => self.unsupported(format!(
                "unknown name '{}'",
                path.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join("::")
            )),
        }
    }

    fn property(&mut self, src: Expr, property: &Property, safe: bool) -> Expr {
        let mm = self.mm;
        match property {
            Property::Feature(f) => match self.index.features.get(f) {
                Some(id) => {
                    let feature = mm.feature(*f);
                    let ty = oxy_ocl::Type::of_typed(mm, feature.ty, &feature.multiplicity);
                    let ty = self.index.ocl_ty(mm, &ty);
                    Expr::new(ExprKind::Feature { source: Box::new(src), feature: *id, safe }, ty)
                }
                None => {
                    self.unsupported(format!("the feature '{}' is not generated", self.mm.feature_qualified_name(*f)))
                }
            },
            Property::DefAttribute { class, name } => match self.index.classes.get(class) {
                Some(id) => {
                    let ty = self.def_attribute_ty(*class, name);
                    Expr::new(
                        ExprKind::DefAttribute { source: Box::new(src), class: *id, name: name.clone(), safe },
                        ty,
                    )
                }
                None => self.unsupported(format!("the definition '{name}' belongs to a class that is not generated")),
            },
            Property::TuplePart(name) => {
                let ty = match &src.ty {
                    Ty::Tuple(parts) => parts.iter().find(|(n, _)| n == name).map(|(_, t)| t.clone()),
                    _ => None,
                };
                Expr::new(ExprKind::TuplePart { source: Box::new(src), name: name.clone() }, ty.unwrap_or(Ty::Invalid))
            }
            Property::TypeName => match &src.kind {
                ExprKind::Type(t) => {
                    let name = match t {
                        Ty::Class(c) => self.class_name(*c),
                        Ty::Enum(e) => {
                            let ecore = self.index.enums.iter().find(|(_, v)| **v == *e).map(|(k, _)| *k);
                            ecore.map(|c| self.mm.classifier(c).name.clone()).unwrap_or_default()
                        }
                        other => ocl_type_name(other),
                    };
                    Expr::new(ExprKind::String(name), Ty::String)
                }
                _ => self.unsupported("the name of a type that is computed at run time"),
            },
        }
    }

    fn class_name(&self, c: ClassId) -> String {
        let ecore = self.index.classes.iter().find(|(_, v)| **v == c).map(|(k, _)| *k);
        ecore.map(|c| self.mm.classifier(c).name.clone()).unwrap_or_default()
    }

    fn def_attribute_ty(&self, class: oxy_ecore::ClassifierId, name: &str) -> Ty {
        match self.env.find_def_attribute(self.mm, class, name) {
            Some(d) => self.index.ocl_ty(self.mm, &d.ty),
            None => Ty::Invalid,
        }
    }

    /// The type of a call of a user-defined operation.
    fn operation_ty(&self, operation: &UserOperation) -> Ty {
        let mm = self.mm;
        match operation {
            UserOperation::Operation(op) => {
                let o = mm.operation(*op);
                match o.ty {
                    Some(_) => self.index.ocl_ty(mm, &oxy_ocl::Type::of_typed(mm, o.ty, &o.multiplicity)),
                    None => Ty::Void,
                }
            }
            UserOperation::Def { class, name, arity } => self
                .env
                .find_def_operations(mm, *class, name)
                .into_iter()
                .find(|d| d.params.len() == *arity)
                .map_or(Ty::Invalid, |d| self.index.ocl_ty(mm, &d.ty)),
        }
    }

    /// `src->collect(v | f(v))` typed `ty`.
    fn collect(&mut self, src: Expr, ty: Ty, f: impl FnOnce(&mut Self, Expr) -> Expr) -> Expr {
        let element = src.ty.element().cloned().unwrap_or(Ty::Invalid);
        let name = self.fresh("_");
        let var = Var { name: name.clone(), ty: element.clone() };
        let body = f(self, Expr::new(ExprKind::Var(name), element));
        Expr::new(
            ExprKind::Iterator {
                kind: IteratorKind::Collect,
                source: Box::new(src),
                vars: vec![var],
                body: Box::new(body),
            },
            ty,
        )
    }

    /// A single value used as a collection: `x->op()` is `x.oclAsSet()->op()`.
    fn as_collection(&mut self, src: Expr) -> Expr {
        if src.ty.is_collection() {
            return src;
        }
        let element = if src.ty == Ty::Void { Ty::Void } else { src.ty.clone() };
        Expr::library("oclAsSet", src, Vec::new(), Ty::collection(CollectionKind::Set, element))
    }

    /// `src->excluding(null)`, for `?.` and `?->`.
    fn excluding_null(&mut self, src: Expr) -> Expr {
        let ty = src.ty.clone();
        Expr::new(
            ExprKind::Library {
                name: "excluding".into(),
                source: Box::new(src),
                args: vec![Expr::new(ExprKind::Null, Ty::Void)],
                safe: false,
            },
            ty,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn call(
        &mut self,
        e: &ast::Expr,
        source: Option<&ast::Expr>,
        name: &ast::Ident,
        args: &[ast::Expr],
        arrow: bool,
        safe: bool,
        ty: Ty,
    ) -> Expr {
        let Some(resolution) = self.resolution(e) else {
            return self.unsupported(format!("unknown operation '{}'", name.name));
        };
        match resolution {
            Resolution::Operation { operation, implicit, collect } => {
                let src = match (source, implicit) {
                    (Some(s), _) => self.lower(s),
                    (None, Some(depth)) => self.implicit_source(*depth),
                    (None, None) => self.implicit_source(0),
                };
                let args: Vec<Expr> = args.iter().map(|a| self.lower(a)).collect();
                if *collect {
                    let src = if safe { self.excluding_null(src) } else { src };
                    let call_ty = self.operation_ty(operation);
                    self.collect(src, ty, |l, var| l.user_call(var, operation, args, false, call_ty))
                } else {
                    self.user_call(src, operation, args, safe, ty)
                }
            }
            Resolution::Library { collect } => {
                let canonical = canonical_name(&name.name).to_string();
                let src = match source {
                    Some(s) => self.lower(s),
                    None => self.implicit_source(0),
                };
                let args: Vec<Expr> = args.iter().map(|a| self.lower(a)).collect();
                if canonical == "allInstances" {
                    return match &src.kind {
                        ExprKind::Type(t @ (Ty::Class(_) | Ty::Enum(_) | Ty::Boolean)) => {
                            Expr::new(ExprKind::AllInstances(t.clone()), ty)
                        }
                        _ => self.unsupported("allInstances() of this type"),
                    };
                }
                if canonical == "oclIsNew" {
                    return Expr::new(ExprKind::Boolean(false), Ty::Boolean);
                }
                if *collect {
                    let src = if safe { self.excluding_null(src) } else { src };
                    let element_ty = match (&ty, canonical.as_str()) {
                        (_, "characters" | "tokenize") => Ty::collection(CollectionKind::Sequence, Ty::String),
                        (Ty::Collection(_, t), _) => (**t).clone(),
                        _ => Ty::Invalid,
                    };
                    return self.collect(src, ty, |_, var| {
                        Expr::new(
                            ExprKind::Library { name: canonical, source: Box::new(var), args, safe: false },
                            element_ty,
                        )
                    });
                }
                let src = if arrow {
                    let src = self.as_collection(src);
                    if safe { self.excluding_null(src) } else { src }
                } else {
                    src
                };
                let safe = safe && !arrow;
                // The checker types containers and contents as OclAny: they are objects.
                let ty = match (canonical.as_str(), ty) {
                    ("oclContainer", Ty::Any) => Ty::Object,
                    ("oclContents" | "eAllContents", Ty::Collection(k, _)) => Ty::collection(k, Ty::Object),
                    (_, ty) => ty,
                };
                Expr::new(ExprKind::Library { name: canonical, source: Box::new(src), args, safe }, ty)
            }
            _ => self.unsupported(format!("unknown operation '{}'", name.name)),
        }
    }

    fn user_call(&mut self, src: Expr, operation: &UserOperation, args: Vec<Expr>, safe: bool, ty: Ty) -> Expr {
        match operation {
            UserOperation::Operation(op) => match self.index.operations.get(op) {
                Some(id) => Expr::new(ExprKind::Call { source: Box::new(src), operation: *id, args, safe }, ty),
                None => self.unsupported(format!(
                    "the operation '{}' belongs to a class that is not generated",
                    self.mm.operation(*op).name
                )),
            },
            UserOperation::Def { class, name, .. } => match self.index.classes.get(class) {
                Some(id) => Expr::new(
                    ExprKind::DefCall { source: Box::new(src), class: *id, name: name.clone(), args, safe },
                    ty,
                ),
                None => self.unsupported(format!("the definition '{name}' belongs to a class that is not generated")),
            },
        }
    }
}

fn ocl_type_name(t: &Ty) -> String {
    match t {
        Ty::Boolean => "Boolean",
        Ty::Integer => "Integer",
        Ty::Real => "Real",
        Ty::String => "String",
        Ty::UnlimitedNatural => "UnlimitedNatural",
        Ty::Void => "OclVoid",
        Ty::Invalid => "OclInvalid",
        _ => "OclAny",
    }
    .to_string()
}
