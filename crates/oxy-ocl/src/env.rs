//! Compilation of OCL sources (Complete OCL documents and Ecore annotations) into an
//! [`Environment`]: resolved invariants, definitions and bodies, plus diagnostics.

use std::collections::HashMap;
use std::rc::Rc;

use oxy_ecore::{ClassifierId, ElementRef, FeatureId, Metamodel, OCL_ANNOTATION_SOURCES, OperationId, PackageId};

use crate::check::Checker;
use crate::diagnostic::{Diagnostic, Severity};
use crate::oclinecore::OclInEcore;
use crate::resolution::Analysis;
use crate::span::{LineIndex, Position, Span};
use crate::syntax::ast::*;
use crate::syntax::{parse_document, parse_expression};
use crate::types::{Type, library_type};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(pub u32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceKind {
    /// A Complete OCL document.
    Document,
    /// An expression stored in an Ecore annotation of `element` under `key`; `offset` is where
    /// its `<details>` tag is in the resource's text, if it was read from one.
    Annotation { resource: String, element: String, key: String, offset: Option<usize> },
    /// An ad-hoc expression, e.g. typed into a console.
    Expression,
}

#[derive(Clone, Debug)]
pub struct Source {
    pub name: String,
    pub text: String,
    pub kind: SourceKind,
    line_index: LineIndex,
}

impl Source {
    pub fn new(name: impl Into<String>, text: impl Into<String>, kind: SourceKind) -> Self {
        let text = text.into();
        Source { name: name.into(), line_index: LineIndex::new(&text), text, kind }
    }

    pub fn position(&self, offset: u32) -> Position {
        self.line_index.position(&self.text, offset as usize)
    }

    pub fn offset(&self, pos: Position) -> usize {
        self.line_index.offset(&self.text, pos)
    }

    pub fn snippet(&self, span: Span) -> &str {
        self.text.get(span.range()).unwrap_or("")
    }
}

/// An expression with the source it came from.
#[derive(Clone, Debug)]
pub struct Body {
    pub expr: Rc<Expr>,
    pub source: SourceId,
    /// The package unqualified type names are resolved in first.
    pub package: Option<PackageId>,
}

#[derive(Clone, Debug)]
pub struct Invariant {
    pub name: Option<String>,
    pub context: ClassifierId,
    /// The name `self` is also known by (`context b : Book`).
    pub self_name: Option<String>,
    pub body: Body,
    pub message: Option<Rc<Expr>>,
    /// The whole constraint in its source (for annotation sources: the expression).
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct DefAttribute {
    pub class: ClassifierId,
    pub name: String,
    pub ty: Type,
    pub is_static: bool,
    pub body: Body,
}

#[derive(Clone, Debug)]
pub struct DefOperation {
    pub class: ClassifierId,
    pub name: String,
    pub params: Vec<(String, Type)>,
    pub ty: Type,
    pub is_static: bool,
    pub body: Body,
}

/// A pre- or postcondition of an operation. They are evaluated whenever the operation is
/// called during evaluation, and validation checks them for every object (see
/// [`crate::validate`]).
#[derive(Clone, Debug)]
pub struct OperationConstraint {
    pub kind: ConstraintKind,
    pub name: Option<String>,
    pub class: ClassifierId,
    pub operation: String,
    pub operation_id: OperationId,
    pub params: Vec<(String, Type)>,
    pub result: Type,
    pub body: Body,
    /// Optional message expression: `pre Name('message'): ...`.
    pub message: Option<Rc<Expr>>,
    /// The whole constraint in its source (for annotation sources: the expression).
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct SourcedDiagnostic {
    pub source: SourceId,
    pub diagnostic: Diagnostic,
}

/// Everything the evaluator needs besides the metamodel and the model.
#[derive(Clone, Debug, Default)]
pub struct Environment {
    pub(crate) sources: Vec<Source>,
    pub(crate) invariants: Vec<Invariant>,
    pub(crate) def_attributes: HashMap<(ClassifierId, String), Rc<DefAttribute>>,
    pub(crate) def_operations: HashMap<(ClassifierId, String), Vec<Rc<DefOperation>>>,
    pub(crate) operation_bodies: HashMap<OperationId, Body>,
    pub(crate) derivations: HashMap<FeatureId, Body>,
    pub(crate) initializers: HashMap<FeatureId, Body>,
    pub(crate) operation_constraints: Vec<OperationConstraint>,
    /// Indices into `operation_constraints`, by operation.
    pub(crate) contracts: HashMap<OperationId, Vec<usize>>,
    pub(crate) diagnostics: Vec<SourcedDiagnostic>,
    pub(crate) expr_types: HashMap<SourceId, Vec<(Span, Type)>>,
    pub(crate) analysis: HashMap<SourceId, Analysis>,
}

impl Environment {
    pub fn source(&self, id: SourceId) -> &Source {
        &self.sources[id.0 as usize]
    }

    pub fn sources(&self) -> impl Iterator<Item = (SourceId, &Source)> {
        self.sources.iter().enumerate().map(|(i, s)| (SourceId(i as u32), s))
    }

    pub fn invariants(&self) -> &[Invariant] {
        &self.invariants
    }

    pub fn operation_constraints(&self) -> &[OperationConstraint] {
        &self.operation_constraints
    }

    /// The pre- and postconditions of an operation (indices into
    /// [`Environment::operation_constraints`]).
    pub fn contracts_of(&self, op: OperationId) -> &[usize] {
        self.contracts.get(&op).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn diagnostics(&self) -> &[SourcedDiagnostic] {
        &self.diagnostics
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.diagnostic.is_error())
    }

    pub fn def_attributes(&self) -> impl Iterator<Item = &DefAttribute> {
        self.def_attributes.values().map(|d| d.as_ref())
    }

    pub fn def_operations(&self) -> impl Iterator<Item = &DefOperation> {
        self.def_operations.values().flatten().map(|d| d.as_ref())
    }

    /// Statically inferred types of all checked expressions in a source, with their spans.
    pub fn expr_types(&self, source: SourceId) -> &[(Span, Type)] {
        self.expr_types.get(&source).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The static types of the checked expressions of a source, and how their names and calls
    /// were resolved (by expression id).
    pub fn analysis(&self, source: SourceId) -> Option<&Analysis> {
        self.analysis.get(&source)
    }

    /// Resolves a type written in OCL (e.g. a variable's declared type) like the checker
    /// does; unqualified names are looked up in `package` first. Unknown types are
    /// [`Type::Error`].
    pub fn resolve_type(&self, mm: &Metamodel, package: Option<PackageId>, te: &TypeExpr) -> Type {
        resolve_type(mm, package, te, &mut Vec::new())
    }

    /// The type of the innermost checked expression whose span contains `offset`.
    pub fn type_at(&self, source: SourceId, offset: u32) -> Option<(Span, &Type)> {
        self.expr_types(source)
            .iter()
            .filter(|(span, _)| span.start <= offset && offset <= span.end)
            .min_by_key(|(span, _)| span.end - span.start)
            .map(|(span, t)| (*span, t))
    }

    /// The type of the largest checked expression ending exactly at `offset` (used for
    /// completion after `expr.` or `expr->`).
    pub fn type_ending_at(&self, source: SourceId, offset: u32) -> Option<&Type> {
        self.expr_types(source)
            .iter()
            .filter(|(span, _)| span.end == offset)
            .max_by_key(|(span, _)| span.end - span.start)
            .map(|(_, t)| t)
    }

    /// A `def:` attribute visible on `class` (declared on it or a supertype).
    pub fn find_def_attribute(&self, mm: &Metamodel, class: ClassifierId, name: &str) -> Option<&Rc<DefAttribute>> {
        if self.def_attributes.is_empty() {
            return None;
        }
        std::iter::once(&class)
            .chain(mm.all_super_types(class))
            .find_map(|c| self.def_attributes.get(&(*c, name.to_string())))
    }

    /// `def:` operations named `name` visible on `class`.
    pub fn find_def_operations(&self, mm: &Metamodel, class: ClassifierId, name: &str) -> Vec<&Rc<DefOperation>> {
        if self.def_operations.is_empty() {
            return Vec::new();
        }
        std::iter::once(&class)
            .chain(mm.all_super_types(class))
            .filter_map(|c| self.def_operations.get(&(*c, name.to_string())))
            .flatten()
            .collect()
    }

    pub fn operation_body(&self, op: OperationId) -> Option<&Body> {
        self.operation_bodies.get(&op)
    }

    pub fn derivation(&self, feature: FeatureId) -> Option<&Body> {
        self.derivations.get(&feature)
    }

    pub fn initializer(&self, feature: FeatureId) -> Option<&Body> {
        self.initializers.get(&feature)
    }

    /// Type checks a standalone expression (e.g. typed into a console) against this
    /// environment. `self` is typed by `self_class` when given.
    pub fn check_expression(
        &self,
        mm: &Metamodel,
        expr: &Expr,
        self_class: Option<ClassifierId>,
    ) -> (Type, Vec<Diagnostic>) {
        let mut checker = Checker::new(mm, self, None);
        if let Some(c) = self_class {
            checker.bind_self(Type::Classifier(c), None);
        }
        let t = checker.check(expr);
        (t, checker.take_diagnostics())
    }

    pub(crate) fn add_source(&mut self, source: Source) -> SourceId {
        self.sources.push(source);
        SourceId(self.sources.len() as u32 - 1)
    }
}

/// A name standing for packages, as QVT's model types do (`modeltype UML uses '…'`): with
/// it, `UML::Class` names a classifier of those packages (or of their subpackages:
/// `UML::sub::Class`), and unqualified names are looked up in them first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageAlias {
    pub name: String,
    pub packages: Vec<PackageId>,
}

/// Resolves a classifier name like [`lookup_classifier`], with package aliases.
pub(crate) fn lookup_classifier_in(
    mm: &Metamodel,
    package: Option<PackageId>,
    aliases: &[PackageAlias],
    path: &[&str],
) -> Result<ClassifierId, String> {
    if aliases.is_empty() {
        return lookup_classifier(mm, package, path);
    }
    let (packages, rest): (Vec<PackageId>, &[&str]) = match path {
        [first, rest @ ..] if !rest.is_empty() => match aliases.iter().find(|a| a.name == *first) {
            Some(a) => (a.packages.clone(), rest),
            None => return lookup_classifier(mm, package, path),
        },
        [_] => (aliases.iter().flat_map(|a| a.packages.iter().copied()).collect(), path),
        _ => return lookup_classifier(mm, package, path),
    };
    let mut found: Vec<ClassifierId> = Vec::new();
    for p in packages {
        for c in find_in_package(mm, p, rest) {
            if !found.contains(&c) {
                found.push(c);
            }
        }
    }
    match found.as_slice() {
        [c] => Ok(*c),
        [] if path.len() == 1 => lookup_classifier(mm, package, path),
        [] => Err(format!("unknown type '{}'", path.join("::"))),
        _ => Err(format!(
            "ambiguous type '{}': could be {}",
            path.join("::"),
            found.iter().map(|c| mm.qualified_name(*c)).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// The classifiers `path` names in a package: `[Class]` (directly in it, else in a
/// subpackage), `[sub, Class]`, or the package's own name first (`[pkg, Class]`).
fn find_in_package(mm: &Metamodel, package: PackageId, path: &[&str]) -> Vec<ClassifierId> {
    match path {
        [] => Vec::new(),
        [name] => {
            if let Some(c) = mm.find_classifier_in(package, name) {
                return vec![c];
            }
            mm.package(package).subpackages.iter().flat_map(|s| find_in_package(mm, *s, path)).collect()
        }
        [first, rest @ ..] => {
            let mut out: Vec<ClassifierId> = mm
                .package(package)
                .subpackages
                .iter()
                .filter(|s| mm.package(**s).name == *first)
                .flat_map(|s| find_in_package(mm, *s, rest))
                .collect();
            if out.is_empty() && mm.package(package).name == *first {
                out = find_in_package(mm, package, rest);
            }
            out
        }
    }
}

/// Resolves a classifier name, preferring the given package for unqualified names.
pub(crate) fn lookup_classifier(
    mm: &Metamodel,
    package: Option<PackageId>,
    path: &[&str],
) -> Result<ClassifierId, String> {
    let candidates = if let [name] = path {
        if let Some(c) = package.and_then(|p| mm.find_classifier_in(p, name)) {
            return Ok(c);
        }
        mm.find_classifiers(name)
    } else {
        mm.find_classifiers_qualified(path)
    };
    let user: Vec<ClassifierId> =
        candidates.iter().copied().filter(|c| mm.classifier(*c).package != mm.ecore_package()).collect();
    match (user.len(), candidates.first()) {
        (1, _) => Ok(user[0]),
        (0, Some(c)) => Ok(*c),
        (0, None) => Err(format!("unknown type '{}'", path.join("::"))),
        _ => Err(format!(
            "ambiguous type '{}': could be {}",
            path.join("::"),
            user.iter().map(|c| mm.qualified_name(*c)).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// Resolves a type expression; reports problems to `diags`.
pub(crate) fn resolve_type(
    mm: &Metamodel,
    package: Option<PackageId>,
    te: &TypeExpr,
    diags: &mut Vec<Diagnostic>,
) -> Type {
    resolve_type_in(mm, package, &[], te, diags)
}

/// Resolves a type expression with package aliases; reports problems to `diags`.
pub(crate) fn resolve_type_in(
    mm: &Metamodel,
    package: Option<PackageId>,
    aliases: &[PackageAlias],
    te: &TypeExpr,
    diags: &mut Vec<Diagnostic>,
) -> Type {
    match &te.kind {
        TypeExprKind::Path(path) => {
            if let [single] = path.as_slice()
                && let Some(t) = library_type(&single.name)
            {
                return t;
            }
            let names: Vec<&str> = path.iter().map(|i| i.name.as_str()).collect();
            match lookup_classifier_in(mm, package, aliases, &names) {
                Ok(c) => Type::of_classifier(mm, c),
                Err(e) => {
                    diags.push(Diagnostic::error(te.span, e));
                    Type::Error
                }
            }
        }
        TypeExprKind::Collection(kind, elem) => {
            Type::collection(*kind, resolve_type_in(mm, package, aliases, elem, diags))
        }
        TypeExprKind::Tuple(parts) => Type::tuple(
            parts.iter().map(|(n, t)| (n.name.clone(), resolve_type_in(mm, package, aliases, t, diags))).collect(),
        ),
        TypeExprKind::Error => Type::Error,
    }
}

/// What the type checker should verify for a registered body.
enum Check {
    Invariant(usize),
    DefAttribute(Rc<DefAttribute>),
    DefOperation(Rc<DefOperation>),
    OperationBody(OperationId),
    Derivation(FeatureId),
    Initializer(FeatureId),
    OperationConstraint(usize),
}

/// Collects OCL sources and compiles them into an [`Environment`].
pub struct EnvironmentBuilder<'a> {
    mm: &'a Metamodel,
    env: Environment,
    checks: Vec<(Check, Option<PackageId>)>,
}

impl<'a> EnvironmentBuilder<'a> {
    pub fn new(mm: &'a Metamodel) -> Self {
        EnvironmentBuilder { mm, env: Environment::default(), checks: Vec::new() }
    }

    fn diag(&mut self, source: SourceId, diagnostic: Diagnostic) {
        self.env.diagnostics.push(SourcedDiagnostic { source, diagnostic });
    }

    /// Adds the OCL constraints, operation bodies and derivations embedded in all
    /// user-loaded Ecore packages.
    pub fn add_ecore_constraints(&mut self) {
        let roots: Vec<PackageId> = self.mm.user_root_packages().collect();
        for p in roots {
            self.add_package_constraints(p);
        }
    }

    /// Adds the OCL embedded in one package (and its subpackages).
    pub fn add_package_constraints(&mut self, package: PackageId) {
        let mm = self.mm;
        let resource = mm.resources()[mm.package(package).resource].uri.clone();
        let file = resource.rsplit(['/', '\\']).next().unwrap_or(&resource).to_string();
        for &c in &mm.package(package).classifiers {
            let Some(class) = mm.class_data(c) else { continue };
            let class_name = mm.qualified_name(c);
            for ann in ocl_annotations(&mm.classifier(c).annotations) {
                for (i, (key, value)) in ann.details.iter().enumerate() {
                    let element = format!("{class_name}::{key}");
                    let (src, expr) =
                        self.annotation_source(&file, &resource, &element, key, value, ann.offsets.get(i).copied());
                    let (expr, message) = split_status_tuple(expr);
                    let span = expr.span;
                    self.env.invariants.push(Invariant {
                        name: Some(key.clone()),
                        context: c,
                        self_name: None,
                        body: Body { expr, source: src, package: Some(package) },
                        message,
                        span,
                    });
                    self.checks.push((Check::Invariant(self.env.invariants.len() - 1), Some(package)));
                }
            }
            for &f in &class.features {
                let feature = mm.feature(f);
                for ann in ocl_annotations(&feature.annotations) {
                    for (i, (key, value)) in ann.details.iter().enumerate() {
                        let element = format!("{class_name}::{}", feature.name);
                        let (src, expr) =
                            self.annotation_source(&file, &resource, &element, key, value, ann.offsets.get(i).copied());
                        let body = Body { expr, source: src, package: Some(package) };
                        match key.as_str() {
                            "derivation" => {
                                self.env.derivations.insert(f, body);
                                self.checks.push((Check::Derivation(f), Some(package)));
                            }
                            "initial" => {
                                self.env.initializers.insert(f, body);
                                self.checks.push((Check::Initializer(f), Some(package)));
                            }
                            other => self.diag(
                                src,
                                Diagnostic::warning(
                                    Span::default(),
                                    format!("ignoring unknown OCL annotation key '{other}'"),
                                ),
                            ),
                        }
                    }
                }
            }
            for &o in &class.operations {
                let op = mm.operation(o);
                for ann in ocl_annotations(&op.annotations) {
                    for (i, (key, value)) in ann.details.iter().enumerate() {
                        let element = format!("{class_name}::{}()", op.name);
                        let (src, expr) =
                            self.annotation_source(&file, &resource, &element, key, value, ann.offsets.get(i).copied());
                        match key.as_str() {
                            "body" if is_invariant_operation(mm, o) => {
                                let (expr, message) = split_status_tuple(expr);
                                let span = expr.span;
                                self.env.invariants.push(Invariant {
                                    name: Some(op.name.clone()),
                                    context: c,
                                    self_name: None,
                                    body: Body { expr, source: src, package: Some(package) },
                                    message,
                                    span,
                                });
                                self.checks.push((Check::Invariant(self.env.invariants.len() - 1), Some(package)));
                            }
                            "body" => {
                                self.env.operation_bodies.insert(o, Body { expr, source: src, package: Some(package) });
                                self.checks.push((Check::OperationBody(o), Some(package)));
                            }
                            key if contract_key(key).is_some() => {
                                let (kind, name) = contract_key(key).unwrap();
                                let (expr, message) = split_status_tuple(expr);
                                let span = expr.span;
                                self.push_operation_constraint(
                                    OperationConstraintSpec { kind, name, message, span },
                                    c,
                                    o,
                                    Body { expr, source: src, package: Some(package) },
                                    Some(package),
                                );
                            }
                            other => self.diag(
                                src,
                                Diagnostic::warning(
                                    Span::default(),
                                    format!("ignoring unknown OCL annotation key '{other}'"),
                                ),
                            ),
                        }
                    }
                }
            }
        }
        for sub in mm.package(package).subpackages.clone() {
            self.add_package_constraints(sub);
        }
    }

    fn annotation_source(
        &mut self,
        file: &str,
        resource: &str,
        element: &str,
        key: &str,
        text: &str,
        offset: Option<usize>,
    ) -> (SourceId, Rc<Expr>) {
        let src = self.env.add_source(Source::new(
            format!("{file}: {element}"),
            text,
            SourceKind::Annotation {
                resource: resource.to_string(),
                element: element.to_string(),
                key: key.to_string(),
                offset,
            },
        ));
        let (expr, diags) = parse_expression(text);
        for d in diags {
            self.diag(src, d);
        }
        (src, Rc::new(expr))
    }

    fn push_operation_constraint(
        &mut self,
        spec: OperationConstraintSpec,
        class: ClassifierId,
        op: OperationId,
        body: Body,
        package: Option<PackageId>,
    ) {
        let mm = self.mm;
        let o = mm.operation(op);
        let index = self.env.operation_constraints.len();
        self.env.operation_constraints.push(OperationConstraint {
            kind: spec.kind,
            name: spec.name,
            class,
            operation: o.name.clone(),
            operation_id: op,
            params: o.params.iter().map(|p| (p.name.clone(), Type::of_typed(mm, p.ty, &p.multiplicity))).collect(),
            result: if o.ty.is_some() { Type::of_typed(mm, o.ty, &o.multiplicity) } else { Type::OclVoid },
            body,
            message: spec.message,
            span: spec.span,
        });
        self.env.contracts.entry(op).or_default().push(index);
        self.checks.push((Check::OperationConstraint(index), package));
    }

    /// Registers the OCL of a built OCLinEcore document. Its constraints are type checked
    /// against the document text, so diagnostics point into the document; the document's
    /// syntax and metamodel problems are reported for the same source.
    pub fn add_oclinecore(&mut self, name: &str, text: &str, built: &OclInEcore) -> SourceId {
        let mm = self.mm;
        let src = self.env.add_source(Source::new(name, text, SourceKind::Document));
        for d in &built.diagnostics {
            self.diag(src, d.clone());
        }
        for ec in &built.constraints {
            let c = &ec.constraint;
            let package = Some(ec.package);
            let body = Body { expr: Rc::new(c.expr.clone()), source: src, package };
            match (ec.owner, c.kind) {
                (ElementRef::Classifier(class), _) => {
                    self.env.invariants.push(Invariant {
                        name: c.name.as_ref().map(|n| n.name.clone()),
                        context: class,
                        self_name: None,
                        body,
                        message: c.message.clone().map(Rc::new),
                        span: c.span,
                    });
                    self.checks.push((Check::Invariant(self.env.invariants.len() - 1), package));
                }
                (ElementRef::Feature(f), ConstraintKind::Init) => {
                    self.env.initializers.insert(f, body);
                    self.checks.push((Check::Initializer(f), package));
                }
                (ElementRef::Feature(f), _) => {
                    self.env.derivations.insert(f, body);
                    self.checks.push((Check::Derivation(f), package));
                }
                (ElementRef::Operation(o), ConstraintKind::Body) => {
                    self.env.operation_bodies.insert(o, body);
                    self.checks.push((Check::OperationBody(o), package));
                }
                (ElementRef::Operation(o), kind) => {
                    let spec = OperationConstraintSpec {
                        kind,
                        name: c.name.as_ref().map(|n| n.name.clone()),
                        message: c.message.clone().map(Rc::new),
                        span: c.span,
                    };
                    self.push_operation_constraint(spec, mm.operation(o).owner, o, body, package);
                }
                (ElementRef::Package(_), _) => {}
            }
        }
        src
    }

    /// Parses a Complete OCL document and registers its contents. `name` identifies the
    /// document in diagnostics (typically its path).
    pub fn add_document(&mut self, name: &str, text: &str) -> SourceId {
        let src = self.env.add_source(Source::new(name, text, SourceKind::Document));
        let (doc, diags) = parse_document(text);
        for d in diags {
            self.diag(src, d);
        }
        for pkg in &doc.packages {
            let package = if pkg.path.is_empty() {
                None
            } else {
                let names: Vec<&str> = pkg.path.iter().map(|i| i.name.as_str()).collect();
                let found = self.mm.find_packages_qualified(&names);
                match found.as_slice() {
                    [p] => Some(*p),
                    [] => {
                        let span = pkg.path[0].span.to(pkg.path.last().unwrap().span);
                        self.diag(src, Diagnostic::error(span, format!("unknown package '{}'", names.join("::"))));
                        continue;
                    }
                    [p, ..] => Some(*p),
                }
            };
            for ctx in &pkg.contexts {
                self.add_context(src, package, ctx);
            }
        }
        src
    }

    fn resolve_class(&mut self, src: SourceId, package: Option<PackageId>, path: &[Ident]) -> Option<ClassifierId> {
        let names: Vec<&str> = path.iter().map(|i| i.name.as_str()).collect();
        let span = path[0].span.to(path.last().unwrap().span);
        match lookup_classifier(self.mm, package, &names) {
            Ok(c) if self.mm.is_class(c) => Some(c),
            Ok(_) => {
                self.diag(src, Diagnostic::error(span, format!("'{}' is not a class", names.join("::"))));
                None
            }
            Err(e) => {
                self.diag(src, Diagnostic::error(span, e));
                None
            }
        }
    }

    fn add_context(&mut self, src: SourceId, package: Option<PackageId>, ctx: &ContextDecl) {
        let mm = self.mm;
        match &ctx.kind {
            ContextKind::Classifier { self_name, invariants, definitions } => {
                let Some(class) = self.resolve_class(src, package, &ctx.path) else { return };
                for inv in invariants {
                    self.env.invariants.push(Invariant {
                        name: inv.name.as_ref().map(|n| n.name.clone()),
                        context: class,
                        self_name: self_name.as_ref().map(|n| n.name.clone()),
                        body: Body { expr: Rc::new(inv.expr.clone()), source: src, package },
                        message: inv.message.clone().map(Rc::new),
                        span: inv.span,
                    });
                    self.checks.push((Check::Invariant(self.env.invariants.len() - 1), package));
                }
                for def in definitions {
                    self.add_definition(src, package, class, def);
                }
            }
            ContextKind::Operation { params, result: _, constraints } => {
                let (class_path, op_name) = ctx.path.split_at(ctx.path.len() - 1);
                let op_name = &op_name[0];
                if class_path.is_empty() {
                    self.diag(src, Diagnostic::error(op_name.span, "expected 'Class::operation(...)'"));
                    return;
                }
                let Some(class) = self.resolve_class(src, package, class_path) else { return };
                let candidates = mm.lookup_operations(class, &op_name.name);
                let op = candidates
                    .iter()
                    .copied()
                    .find(|o| mm.operation(*o).params.len() == params.len())
                    .or_else(|| candidates.first().copied());
                let Some(op) = op else {
                    self.diag(
                        src,
                        Diagnostic::error(
                            op_name.span,
                            format!("class '{}' has no operation '{}'", mm.classifier(class).name, op_name.name),
                        ),
                    );
                    return;
                };
                if mm.operation(op).params.len() != params.len() {
                    self.diag(
                        src,
                        Diagnostic::error(
                            op_name.span,
                            format!(
                                "operation '{}' has {} parameter(s), but {} are declared here",
                                op_name.name,
                                mm.operation(op).params.len(),
                                params.len()
                            ),
                        ),
                    );
                }
                for c in constraints {
                    let body = Body { expr: Rc::new(c.expr.clone()), source: src, package };
                    match c.kind {
                        ConstraintKind::Body => {
                            if self.env.operation_bodies.contains_key(&op) {
                                self.diag(
                                    src,
                                    Diagnostic::warning(
                                        c.span,
                                        "this body replaces the one defined in the Ecore model",
                                    ),
                                );
                            }
                            self.env.operation_bodies.insert(op, body);
                            self.checks.push((Check::OperationBody(op), package));
                        }
                        kind => {
                            let spec = OperationConstraintSpec {
                                kind,
                                name: c.name.as_ref().map(|n| n.name.clone()),
                                message: c.message.clone().map(Rc::new),
                                span: c.span,
                            };
                            self.push_operation_constraint(spec, class, op, body, package);
                        }
                    }
                }
            }
            ContextKind::Property { ty: _, constraints } => {
                let (class_path, prop) = ctx.path.split_at(ctx.path.len() - 1);
                let prop = &prop[0];
                let Some(class) = self.resolve_class(src, package, class_path) else { return };
                let Some(feature) = mm.lookup_feature(class, &prop.name) else {
                    self.diag(
                        src,
                        Diagnostic::error(
                            prop.span,
                            format!("class '{}' has no property '{}'", mm.classifier(class).name, prop.name),
                        ),
                    );
                    return;
                };
                for c in constraints {
                    let body = Body { expr: Rc::new(c.expr.clone()), source: src, package };
                    if c.kind == ConstraintKind::Init {
                        self.env.initializers.insert(feature, body);
                        self.checks.push((Check::Initializer(feature), package));
                    } else {
                        self.env.derivations.insert(feature, body);
                        self.checks.push((Check::Derivation(feature), package));
                    }
                }
            }
        }
    }

    fn add_definition(&mut self, src: SourceId, package: Option<PackageId>, class: ClassifierId, def: &Definition) {
        let mm = self.mm;
        let mut diags = Vec::new();
        let ty = def.ty.as_ref().map_or(Type::Error, |t| resolve_type(mm, package, t, &mut diags));
        let body = Body { expr: Rc::new(def.body.clone()), source: src, package };
        let key = (class, def.name.name.clone());
        match &def.params {
            None => {
                if mm.lookup_feature(class, &def.name.name).is_some() || self.env.def_attributes.contains_key(&key) {
                    diags.push(Diagnostic::error(
                        def.name.span,
                        format!("'{}' is already defined for '{}'", def.name.name, mm.classifier(class).name),
                    ));
                }
                let d =
                    Rc::new(DefAttribute { class, name: def.name.name.clone(), ty, is_static: def.is_static, body });
                self.env.def_attributes.insert(key, d.clone());
                self.checks.push((Check::DefAttribute(d), package));
            }
            Some(params) => {
                let params: Vec<(String, Type)> = params
                    .iter()
                    .map(|p| {
                        let t = p.ty.as_ref().map_or(Type::Error, |t| resolve_type(mm, package, t, &mut diags));
                        (p.name.name.clone(), t)
                    })
                    .collect();
                let d = Rc::new(DefOperation {
                    class,
                    name: def.name.name.clone(),
                    params,
                    ty,
                    is_static: def.is_static,
                    body,
                });
                self.env.def_operations.entry(key).or_default().push(d.clone());
                self.checks.push((Check::DefOperation(d), package));
            }
        }
        for d in diags {
            self.diag(src, d);
        }
    }

    /// Type checks everything that was added and returns the environment.
    pub fn build(mut self) -> Environment {
        let mm = self.mm;
        let checks = std::mem::take(&mut self.checks);
        let mut results = Vec::new();
        let mut types: HashMap<SourceId, Vec<(Span, Type)>> = HashMap::new();
        let mut analysis: HashMap<SourceId, Analysis> = HashMap::new();
        {
            let env = &self.env;
            for (check, package) in &checks {
                let (source, mut checker) = match check {
                    Check::Invariant(i) => {
                        let inv = &env.invariants[*i];
                        let mut c = Checker::new(mm, env, *package);
                        c.bind_self(Type::Classifier(inv.context), inv.self_name.as_deref());
                        c.expect(&inv.body.expr, &Type::Boolean, "an invariant");
                        if let Some(m) = &inv.message {
                            c.expect(m, &Type::String, "a constraint message");
                        }
                        (inv.body.source, c)
                    }
                    Check::DefAttribute(d) => {
                        let mut c = Checker::new(mm, env, *package);
                        c.bind_self(Type::Classifier(d.class), None);
                        c.expect(&d.body.expr, &d.ty, &format!("'{}'", d.name));
                        (d.body.source, c)
                    }
                    Check::DefOperation(d) => {
                        let mut c = Checker::new(mm, env, *package);
                        c.bind_self(Type::Classifier(d.class), None);
                        for (n, t) in &d.params {
                            c.bind(n, t.clone());
                        }
                        c.expect(&d.body.expr, &d.ty, &format!("'{}'", d.name));
                        (d.body.source, c)
                    }
                    Check::OperationBody(o) => {
                        let op = mm.operation(*o);
                        let body = &env.operation_bodies[o];
                        let mut c = Checker::new(mm, env, *package);
                        c.bind_self(Type::Classifier(op.owner), None);
                        for p in &op.params {
                            c.bind(&p.name, Type::of_typed(mm, p.ty, &p.multiplicity));
                        }
                        let ty =
                            if op.ty.is_some() { Type::of_typed(mm, op.ty, &op.multiplicity) } else { Type::OclAny };
                        c.expect(&body.expr, &ty, &format!("the body of '{}'", op.name));
                        (body.source, c)
                    }
                    Check::Derivation(f) | Check::Initializer(f) => {
                        let feature = mm.feature(*f);
                        let body = if matches!(check, Check::Derivation(_)) {
                            &env.derivations[f]
                        } else {
                            &env.initializers[f]
                        };
                        let mut c = Checker::new(mm, env, *package);
                        c.bind_self(Type::Classifier(feature.owner), None);
                        let ty = Type::of_typed(mm, feature.ty, &feature.multiplicity);
                        c.expect(&body.expr, &ty, &format!("the value of '{}'", feature.name));
                        (body.source, c)
                    }
                    Check::OperationConstraint(i) => {
                        let oc = &env.operation_constraints[*i];
                        let mut c = Checker::new(mm, env, *package);
                        c.bind_self(Type::Classifier(oc.class), None);
                        for (n, t) in &oc.params {
                            c.bind(n, t.clone());
                        }
                        if oc.kind == ConstraintKind::Post {
                            c.bind("result", oc.result.clone());
                            c.allow_at_pre();
                        }
                        c.expect(&oc.body.expr, &Type::Boolean, &format!("a {}condition", oc.kind.keyword()));
                        if let Some(m) = &oc.message {
                            c.expect(m, &Type::String, "a constraint message");
                        }
                        (oc.body.source, c)
                    }
                };
                for d in checker.take_diagnostics() {
                    results.push(SourcedDiagnostic { source, diagnostic: d });
                }
                types.entry(source).or_default().extend(checker.take_types());
                analysis.entry(source).or_default().extend(checker.take_analysis());
            }
        }
        self.env.diagnostics.extend(results);
        self.env.expr_types = types;
        self.env.analysis = analysis;
        // Stable order: by source, then position, errors first at equal positions.
        self.env.diagnostics.sort_by(|a, b| {
            (a.source, a.diagnostic.span.start, a.diagnostic.severity).cmp(&(
                b.source,
                b.diagnostic.span.start,
                b.diagnostic.severity,
            ))
        });
        self.env.diagnostics.dedup_by(|a, b| a.source == b.source && a.diagnostic == b.diagnostic);
        self.env
    }
}

/// What distinguishes the pre- and postconditions registered by
/// [`EnvironmentBuilder::push_operation_constraint`].
struct OperationConstraintSpec {
    kind: ConstraintKind,
    name: Option<String>,
    message: Option<Rc<Expr>>,
    span: Span,
}

/// Annotation keys of pre- and postconditions: `pre`/`precondition` and `post`/`postcondition`,
/// optionally followed by `_Name` (as written for named constraints when exporting
/// OCLinEcore; a numeric suffix only makes keys unique).
pub fn contract_key(key: &str) -> Option<(ConstraintKind, Option<String>)> {
    let (head, name) = match key.split_once('_') {
        Some((h, n)) => (h, Some(n)),
        None => (key, None),
    };
    let kind = match head {
        "pre" | "precondition" => ConstraintKind::Pre,
        "post" | "postcondition" => ConstraintKind::Post,
        _ => return None,
    };
    let name = name.filter(|n| !n.is_empty() && !n.chars().all(|c| c.is_ascii_digit())).map(str::to_string);
    Some((kind, name))
}

fn ocl_annotations(annotations: &[oxy_ecore::Annotation]) -> impl Iterator<Item = &oxy_ecore::Annotation> {
    annotations.iter().filter(|a| OCL_ANNOTATION_SOURCES.contains(&a.source.as_str()))
}

/// EMF "invariant operations": `op(diagnostics : EDiagnosticChain, context : EMap) : EBoolean`.
pub fn is_invariant_operation(mm: &Metamodel, op: OperationId) -> bool {
    let o = mm.operation(op);
    let type_name = |t: Option<ClassifierId>| t.map(|t| mm.classifier(t).name.as_str());
    o.params.len() == 2
        && type_name(o.params[0].ty) == Some("EDiagnosticChain")
        && type_name(o.params[1].ty) == Some("EMap")
        && type_name(o.ty) == Some("EBoolean")
}

/// OCLinEcore encodes `inv name('message'): expr` as
/// `Tuple{message : String = ..., status : Boolean = expr}.status`. Split such expressions
/// back into the status expression and the message.
pub(crate) fn split_status_tuple(expr: Rc<Expr>) -> (Rc<Expr>, Option<Rc<Expr>>) {
    if let ExprKind::Property { source, name, .. } = &expr.kind
        && name.name == "status"
        && let ExprKind::TupleLit(parts) = &source.kind
    {
        let status = parts.iter().find(|p| p.name.name == "status").and_then(|p| p.init.clone());
        let message = parts.iter().find(|p| p.name.name == "message").and_then(|p| p.init.clone());
        if let Some(status) = status {
            return (Rc::new(status), message.map(Rc::new));
        }
    }
    (expr, None)
}

impl SourcedDiagnostic {
    pub fn severity(&self) -> Severity {
        self.diagnostic.severity
    }
}
