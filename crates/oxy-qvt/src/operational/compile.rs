//! Compiling a QVTo document: the units it imports are loaded (each parsed with its own span
//! offset, so that every span tells its unit); the model types of each unit become its package
//! aliases (`UML::Class`); the module's mappings, helpers and properties — with those of the
//! libraries it imports or uses and the transformations it extends, which it overrides — get
//! typed signatures; transformations it accesses are compiled on their own (for `new T(...)`);
//! and every body is type checked by OCL's checker with the transformation as its
//! [`CheckHost`] (which knows the model parameters, `this`, the properties, helpers and
//! mappings).

use std::collections::HashMap;
use std::rc::Rc;

use oxy_ecore::{ClassifierId, Metamodel, PackageId};
use oxy_ocl::syntax::ast::{Expr, Ident, TypeExpr, TypeExprKind};
use oxy_ocl::{CheckHost, Checker, Diagnostic, Environment, PackageAlias, Span, Type};

use super::syntax::{
    self, Direction, Document, Element, Helper, HelperBody, Main, Mapping, ModuleKind, Property, PropertyKind,
    UsageKind,
};
use crate::{Compiled, Inner, Language, PackageLookup, Parameter, Scope, Symbol, UnitLookup, library};

/// At most this many units are loaded for one document.
const MAX_UNITS: usize = 256;
/// Transformations accessing transformations: at most this deep.
const MAX_ACCESS_DEPTH: usize = 8;

/// A unit of a compilation: the document compiled (the first) or one it imports.
pub(crate) struct Unit {
    /// Its file (`None`: the document).
    pub path: Option<String>,
    /// Where its spans start: the spans of the units do not overlap.
    pub offset: u32,
    pub text: String,
    /// The transformation or library it defines.
    pub module: Option<(ModuleKind, String)>,
    pub aliases: Rc<[PackageAlias]>,
}

impl Unit {
    fn contains(&self, span: Span) -> bool {
        self.offset <= span.start && span.start <= self.offset + self.text.len() as u32
    }

    /// The line (from 1) of an offset of the unit.
    fn line(&self, offset: u32) -> usize {
        let local = (offset.saturating_sub(self.offset) as usize).min(self.text.len());
        self.text[..local].matches('\n').count() + 1
    }
}

/// The units of a compilation.
pub(crate) struct Units(pub Vec<Unit>);

impl Units {
    /// The unit a span is in.
    pub fn of(&self, span: Span) -> usize {
        self.0.iter().rposition(|u| u.contains(span)).unwrap_or(0)
    }

    /// The span relative to the start of its unit, and the unit's file (`None`: the document).
    pub fn locate(&self, span: Span) -> (Span, Option<String>) {
        let u = &self.0[self.of(span)];
        (Span { start: span.start - u.offset, end: span.end - u.offset }, u.path.clone())
    }
}

/// A compiled QVTo transformation (or library).
pub struct Transformation {
    pub name: String,
    /// Whether it is a library: its objects are created in the models of the transformations
    /// that use it.
    pub(crate) library: bool,
    /// The units of the compilation (shared with the transformations it accesses).
    pub(crate) units: Rc<Units>,
    /// The aliases of its unit.
    pub(crate) aliases: Rc<[PackageAlias]>,
    /// The names of the modules its operations come from (its own, its libraries, the
    /// transformations it extends): `this` and `Lib::op()` name them.
    pub(crate) modules: Vec<String>,
    /// The transformations it accesses (`access transformation T`), for `new T(...)`.
    pub(crate) accessed: Vec<Transformation>,
    /// The model parameters of the transformations it extends, by their names there: the
    /// parameters they stand for here (by position).
    pub(crate) param_aliases: Vec<(String, String)>,
    pub(crate) params: Vec<ModelParamInfo>,
    pub(crate) mappings: Vec<(Mapping, MappingSig)>,
    pub(crate) helpers: Vec<(Helper, HelperSig)>,
    pub(crate) properties: Vec<(Property, PropertySig)>,
    pub(crate) main: Option<Main>,
    /// The unit of `main` (an extended transformation's).
    pub(crate) main_unit: usize,
    /// The parameters of `main`, bound to root objects of the input models.
    pub(crate) main_params: Vec<Sig>,
    /// Without `main`: a mapping `main` without a context, the entry of old-style QVTo.
    pub(crate) entry_mapping: Option<usize>,
    /// Without `main` and an entry mapping: a query `main` (old style).
    pub(crate) entry_helper: Option<usize>,
    /// Old style (no model parameters): the output model is the result of the entry mapping
    /// (the parameter `result`), not everything created.
    pub(crate) implicit_output: bool,
}

pub(crate) struct ModelParamInfo {
    pub name: String,
    pub direction: Direction,
    pub model_type: String,
    pub packages: Vec<PackageId>,
}

/// A parameter of a mapping or helper.
#[derive(Clone, Debug)]
pub(crate) struct Sig {
    pub name: String,
    pub ty: Type,
}

/// A result of a mapping: `result` (or its name), its type and the model it is created in.
#[derive(Clone, Debug)]
pub(crate) struct ResultSig {
    pub name: Option<String>,
    pub ty: Type,
    pub extent: Option<String>,
}

pub(crate) struct MappingSig {
    /// The unit it is declared in.
    pub unit: usize,
    /// Whether a mapping with the same signature of a module closer to the transformation
    /// overrides it (it is only called qualified, `Lib::m`).
    pub overridden: bool,
    pub context: Option<Type>,
    pub params: Vec<Sig>,
    pub results: Vec<ResultSig>,
    pub inherits: Vec<usize>,
    pub merges: Vec<usize>,
    pub disjuncts: Vec<usize>,
}

pub(crate) struct HelperSig {
    pub unit: usize,
    pub overridden: bool,
    pub context: Option<Type>,
    pub params: Vec<Sig>,
    pub result: Type,
}

pub(crate) struct PropertySig {
    pub unit: usize,
    pub overridden: bool,
    pub ty: Type,
    pub context: Option<ClassifierId>,
}

impl MappingSig {
    /// The type a call returns: its result, a tuple of its results, or nothing.
    pub fn result_type(&self) -> Type {
        match self.results.as_slice() {
            [] => Type::OclVoid,
            [r] => r.ty.clone(),
            rs => Type::tuple(
                rs.iter()
                    .enumerate()
                    .map(|(i, r)| (r.name.clone().unwrap_or_else(|| format!("r{i}")), r.ty.clone()))
                    .collect(),
            ),
        }
    }
}

/// Whether `t` names (a part of) the classifier `c` (`UML::Class` names `Class`).
fn names_context(mm: &Metamodel, context: &Option<Type>, qualifier: &[Ident]) -> bool {
    match (context, qualifier.last()) {
        (_, None) => true,
        (Some(Type::Classifier(c)), Some(last)) => mm.classifier(*c).name == last.name,
        _ => false,
    }
}

/// Whether a value of type `arg` can be passed for a parameter of type `param` (leniently:
/// the element types of collections are not known at run time).
pub(crate) fn fits(mm: &Metamodel, arg: &Type, param: &Type) -> bool {
    match (arg, param) {
        (Type::Error | Type::OclVoid | Type::OclInvalid, _) | (_, Type::Error) => true,
        (Type::Collection(k1, e1), Type::Collection(k2, e2)) => {
            (k1 == k2 || matches!(k2, oxy_ocl::types::CollectionKind::Collection))
                && (matches!(**e1, Type::OclAny) || fits(mm, e1, e2))
        }
        _ => arg.conforms_to(param, mm),
    }
}

/// The candidates (helpers or mappings) with as many parameters as there are arguments whose
/// types fit; if none fits, all of that arity (so that the mismatch is reported).
fn overloads<'p>(
    mm: &Metamodel,
    candidates: &[usize],
    args: &[Type],
    params: impl Fn(usize) -> &'p [Sig],
) -> Vec<usize> {
    let arity: Vec<usize> = candidates.iter().copied().filter(|i| params(*i).len() == args.len()).collect();
    let typed: Vec<usize> =
        arity.iter().copied().filter(|i| params(*i).iter().zip(args).all(|(p, a)| fits(mm, a, &p.ty))).collect();
    if typed.is_empty() { arity } else { typed }
}

/// Of overloads, the one whose parameter types conform to those of all others (the first if
/// none does).
fn most_specific_params<'p>(
    mm: &Metamodel,
    candidates: &[usize],
    params: impl Fn(usize) -> &'p [Sig],
) -> Option<usize> {
    candidates
        .iter()
        .copied()
        .find(|i| {
            candidates.iter().all(|j| params(*i).iter().zip(params(*j)).all(|(a, b)| a.ty.conforms_to(&b.ty, mm)))
        })
        .or_else(|| candidates.first().copied())
}

/// The most specific of the candidates whose context `src` conforms to.
fn most_specific<'a>(
    mm: &Metamodel,
    src: &Type,
    candidates: impl Iterator<Item = (usize, &'a Option<Type>)>,
) -> Option<usize> {
    let mut best: Option<(usize, &Type)> = None;
    for (i, ctx) in candidates {
        let Some(ctx) = ctx else { continue };
        if !src.conforms_to(ctx, mm) {
            continue;
        }
        match best {
            Some((_, b)) if !ctx.conforms_to(b, mm) || ctx == b => {}
            _ => best = Some((i, ctx)),
        }
    }
    best.map(|(i, _)| i)
}

impl Transformation {
    /// `this.m()`: the transformation as the source of its global helpers and mappings.
    pub(crate) fn without_this<'t>(&self, source: Option<&'t Type>) -> Option<&'t Type> {
        source.filter(|t| !matches!(t, Type::Model(m) if self.is_module(m)))
    }

    /// Whether `name` is the transformation or a module its operations come from.
    pub(crate) fn is_module(&self, name: &str) -> bool {
        name == self.name || self.modules.iter().any(|m| m == name)
    }

    /// The aliases of a unit (for the code declared in it).
    pub(crate) fn aliases_of(&self, unit: usize) -> Rc<[PackageAlias]> {
        self.units.0[unit].aliases.clone()
    }

    /// The module a unit defines.
    fn module_of(&self, unit: usize) -> Option<&str> {
        self.units.0[unit].module.as_ref().map(|(_, n)| n.as_str())
    }

    /// The transformation `new T(...)` names.
    pub(crate) fn accessed(&self, name: &str) -> Option<usize> {
        self.accessed.iter().position(|t| t.name == name)
    }

    pub(crate) fn param(&self, name: &str) -> Option<&ModelParamInfo> {
        let name = self.param_aliases.iter().find(|(a, _)| a == name).map_or(name, |(_, real)| real.as_str());
        self.params.iter().find(|p| p.name == name)
    }

    /// The types QVTo names besides classifiers: accessed transformations, `Status` and
    /// `Transformation` (any of them).
    pub(crate) fn named_type(&self, path: &[Ident]) -> Option<Type> {
        let [name] = path else { return None };
        match name.name.as_str() {
            n if self.accessed(n).is_some() => Some(Type::Model(n.into())),
            STATUS => Some(Type::Model(STATUS.into())),
            "Transformation" => Some(Type::OclAny),
            _ => None,
        }
    }

    /// The helper a call selects: contextual ones by the (static or runtime) type of the
    /// source, the most specific; global ones without a source. `Err` if there are helpers of
    /// that name but none fits.
    pub(crate) fn select_helper(
        &self,
        mm: &Metamodel,
        source: Option<&Type>,
        name: &str,
        args: &[Type],
    ) -> Option<Result<usize, String>> {
        let arity = args.len();
        // `Lib::op`: the operation of that module, even if overridden.
        let (module, name) = match name.rsplit_once("::") {
            Some((m, n)) => (Some(m.rsplit("::").next().unwrap_or(m)), n),
            None => (None, name),
        };
        let named: Vec<usize> = self
            .helpers
            .iter()
            .enumerate()
            .filter(|(_, (h, s))| {
                h.name.name == name
                    && match module {
                        Some(m) => self.module_of(s.unit) == Some(m),
                        None => !s.overridden,
                    }
            })
            .map(|(i, _)| i)
            .collect();
        if named.is_empty() {
            return None;
        }
        let fitting = overloads(mm, &named, args, |i| &self.helpers[i].1.params);
        let found = match source {
            None => {
                let global: Vec<usize> =
                    fitting.iter().copied().filter(|i| self.helpers[*i].1.context.is_none()).collect();
                most_specific_params(mm, &global, |i| &self.helpers[i].1.params)
            }
            Some(src) => most_specific(mm, src, fitting.iter().map(|i| (*i, &self.helpers[*i].1.context))),
        };
        match found {
            Some(i) => Some(Ok(i)),
            None => {
                let contextual = named.iter().any(|i| self.helpers[*i].1.context.is_some() == source.is_some());
                if !contextual {
                    return None;
                }
                let arities = named.iter().any(|i| self.helpers[*i].1.params.len() == arity);
                Some(Err(if arities {
                    match source {
                        Some(src) => format!("'{name}' is not defined for {}", src.display(mm)),
                        None => format!("'{name}' needs an object to be called on"),
                    }
                } else {
                    let n = self.helpers[named[0]].1.params.len();
                    format!("'{name}' expects {n} argument(s), but {arity} were given")
                }))
            }
        }
    }

    /// The mapping a call selects (like [`Transformation::select_helper`]); `name` may be
    /// qualified by the context type.
    pub(crate) fn select_mapping(
        &self,
        mm: &Metamodel,
        source: Option<&Type>,
        name: &[Ident],
        args: &[Type],
    ) -> Result<usize, String> {
        let arity = args.len();
        let Some((last, qualifier)) = name.split_last() else { return Err("missing mapping name".into()) };
        let named: Vec<usize> = self
            .mappings
            .iter()
            .enumerate()
            .filter(|(_, (m, s))| m.name.name == last.name && !s.overridden && names_context(mm, &s.context, qualifier))
            .map(|(i, _)| i)
            .collect();
        if named.is_empty() {
            return Err(format!("unknown mapping '{}'", oxy_ocl::host::path_text(name)));
        }
        let fitting = overloads(mm, &named, args, |i| &self.mappings[i].1.params);
        if fitting.is_empty() {
            let n = self.mappings[named[0]].1.params.len();
            return Err(format!("mapping '{}' expects {n} argument(s), but {arity} were given", last.name));
        }
        let found = match source {
            None => fitting.iter().copied().find(|i| self.mappings[*i].1.context.is_none()),
            // Statically unknown objects: whichever applies at run time.
            Some(Type::OclAny | Type::Error) => fitting.iter().copied().find(|i| self.mappings[*i].1.context.is_some()),
            Some(src) => most_specific(mm, src, fitting.iter().map(|i| (*i, &self.mappings[*i].1.context))),
        };
        found.ok_or_else(|| match source {
            Some(src) => format!("mapping '{}' is not defined for {}", last.name, src.display(mm)),
            None => format!("mapping '{}' needs an object to be called on", last.name),
        })
    }

    /// The model parameter objects of `class` are created in by default: the first output
    /// whose model type has the class's package, else the first output.
    pub(crate) fn default_extent(&self, mm: &Metamodel, class: ClassifierId) -> Option<&ModelParamInfo> {
        let mut package = Some(mm.classifier(class).package);
        let mut packages = Vec::new();
        while let Some(p) = package {
            packages.push(p);
            package = mm.package(p).super_package;
        }
        let outputs = || self.params.iter().filter(|p| p.direction != Direction::In);
        outputs().find(|p| p.packages.iter().any(|x| packages.contains(x))).or_else(|| outputs().next())
    }
}

impl Transformation {
    /// Whether the model type of a parameter has the package of a class.
    fn output_has(&self, mm: &Metamodel, p: &ModelParamInfo, class: ClassifierId) -> bool {
        let mut package = Some(mm.classifier(class).package);
        while let Some(x) = package {
            if p.packages.contains(&x) {
                return true;
            }
            package = mm.package(x).super_package;
        }
        false
    }
}

impl CheckHost for Transformation {
    fn variable(&self, name: &str) -> Option<Type> {
        if name == "this" {
            return Some(Type::Model(self.name.as_str().into()));
        }
        if let Some(p) = self.param(name) {
            return Some(Type::Model(p.model_type.as_str().into()));
        }
        self.properties
            .iter()
            .find(|(p, s)| p.kind != PropertyKind::Intermediate && p.name.name == name && !s.overridden)
            .map(|(_, s)| s.ty.clone())
    }

    /// Properties of the transformation, also configuration properties (as in Eclipse QVTo,
    /// a transformation may change what it was configured with).
    fn assignable(&self, name: &str) -> bool {
        self.properties
            .iter()
            .any(|(p, s)| p.kind != PropertyKind::Intermediate && p.name.name == name && !s.overridden)
    }

    fn operation(
        &self,
        mm: &Metamodel,
        source: Option<&Type>,
        name: &str,
        args: &[Type],
    ) -> Option<Result<Type, String>> {
        let source = self.without_this(source);
        match (source, name, args) {
            // `new T(...).transform()`: runs an accessed transformation.
            (Some(Type::Model(m)), "transform", []) if self.accessed(m).is_some() => {
                return Some(Ok(Type::Model(STATUS.into())));
            }
            // A `Transformation` (any accessed one).
            (Some(Type::OclAny), "transform", []) if !self.accessed.is_empty() => {
                return Some(Ok(Type::Model(STATUS.into())));
            }
            (Some(Type::Model(m)), "succeeded" | "failed", []) if **m == *STATUS => return Some(Ok(Type::Boolean)),
            // `dump(x)`: logs a value (Eclipse QVTo's standard library).
            (None, "dump", [_]) => return Some(Ok(Type::OclVoid)),
            (Some(Type::Model(m)), "raisedException", []) if **m == *STATUS => return Some(Ok(Type::OclAny)),
            _ => {}
        }
        match self.select_helper(mm, source, name, args) {
            Some(Ok(i)) => {
                let sig = &self.helpers[i].1;
                for (n, (a, p)) in args.iter().zip(&sig.params).enumerate() {
                    if !a.conforms_to(&p.ty, mm) {
                        return Some(Err(format!(
                            "argument {} has type {}, but {} is expected",
                            n + 1,
                            a.display(mm),
                            p.ty.display(mm)
                        )));
                    }
                }
                Some(Ok(sig.result.clone()))
            }
            Some(Err(e)) => library::check_operation(mm, source, name, args).or(Some(Err(e))),
            None => library::check_operation(mm, source, name, args).or_else(|| {
                // A mapping called like an operation (`self.toTable()`): `map`.
                let name = [Ident { name: name.to_string(), span: Span::new(0, 0) }];
                let i = self.select_mapping(mm, source, &name, args).ok()?;
                Some(Ok(self.mappings[i].1.result_type()))
            }),
        }
    }

    fn property(&self, mm: &Metamodel, source: &Type, name: &str) -> Option<Type> {
        match source {
            // The properties of an instance of an accessed transformation.
            Type::Model(m) if !self.is_module(m) && self.accessed(m).is_some() => {
                let t = &self.accessed[self.accessed(m)?];
                t.properties
                    .iter()
                    .find(|(p, s)| p.kind != PropertyKind::Intermediate && p.name.name == name && !s.overridden)
                    .map(|(_, s)| s.ty.clone())
            }
            Type::Model(m) if self.is_module(m) => self
                .properties
                .iter()
                .find(|(p, s)| p.kind != PropertyKind::Intermediate && p.name.name == name && !s.overridden)
                .map(|(_, s)| s.ty.clone()),
            Type::Classifier(c) => self
                .properties
                .iter()
                .find(|(p, s)| {
                    p.kind == PropertyKind::Intermediate
                        && p.name.name == name
                        && s.context.is_some_and(|ctx| mm.conforms_to(*c, ctx))
                })
                .map(|(_, s)| s.ty.clone())
                .or_else(|| {
                    // `_abstract`: a feature named like a keyword of QVTo.
                    let f = mm.lookup_feature(*c, escaped(name)?)?;
                    let feature = mm.feature(f);
                    Some(Type::of_typed(mm, feature.ty, &feature.multiplicity))
                }),
            _ => None,
        }
    }

    fn mapping(&self, mm: &Metamodel, source: Option<&Type>, name: &[Ident], args: &[Type]) -> Result<Type, String> {
        let i = self.select_mapping(mm, self.without_this(source), name, args)?;
        let sig = &self.mappings[i].1;
        for (n, (a, p)) in args.iter().zip(&sig.params).enumerate() {
            if !a.conforms_to(&p.ty, mm) {
                return Err(format!(
                    "argument {} has type {}, but {} is expected",
                    n + 1,
                    a.display(mm),
                    p.ty.display(mm)
                ));
            }
        }
        Ok(sig.result_type())
    }

    fn mapping_reference(&self, mm: &Metamodel, name: &[Ident]) -> Result<(), String> {
        let Some((last, qualifier)) = name.split_last() else { return Err("missing mapping name".into()) };
        if self.mappings.iter().any(|(m, s)| m.name.name == last.name && names_context(mm, &s.context, qualifier)) {
            Ok(())
        } else {
            Err(format!("unknown mapping '{}'", oxy_ocl::host::path_text(name)))
        }
    }

    fn extent(&self, name: &str, _class: Option<ClassifierId>) -> Result<(), String> {
        match self.param(name) {
            Some(p) if p.direction == Direction::In => {
                Err(format!("objects cannot be created in the input model '{name}'"))
            }
            Some(_) => Ok(()),
            None => Err(format!("unknown model parameter '{name}'")),
        }
    }

    fn instantiate(&self, _mm: &Metamodel, ty: &TypeExpr, args: &[Type]) -> Option<Result<Type, String>> {
        let TypeExprKind::Path(path) = &ty.kind else { return None };
        let name = &path.last()?.name;
        let t = &self.accessed[self.accessed(name)?];
        if args.len() != t.params.len() {
            return Some(Err(format!(
                "'{name}' has {} model parameter(s), but {} model(s) were given",
                t.params.len(),
                args.len()
            )));
        }
        if let Some(n) = args.iter().position(|a| !matches!(a, Type::Model(_) | Type::Error)) {
            return Some(Err(format!("argument {} of '{name}' must be a model parameter", n + 1)));
        }
        Some(Ok(Type::Model(name.as_str().into())))
    }

    fn named_type(&self, path: &[Ident]) -> Option<Type> {
        Transformation::named_type(self, path)
    }

    fn can_create(&self, mm: &Metamodel, class: ClassifierId) -> Result<(), String> {
        if self.library {
            return Ok(());
        }
        match self.default_extent(mm, class) {
            Some(p) if p.packages.is_empty() || self.output_has(mm, p, class) => Ok(()),
            Some(_) => Err(match self.params.iter().find(|p| self.output_has(mm, p, class)) {
                Some(input) => format!("objects cannot be created in the input model '{}'", input.name),
                None => format!(
                    "no output model has {}'s metamodel: name the model to create it in (T@model)",
                    mm.classifier(class).name
                ),
            }),
            None => Err("the transformation has no output model to create objects in".into()),
        }
    }
}

/// The type and value of what `transform()` returns (Eclipse QVTo's `Status`).
pub(crate) const STATUS: &str = "Status";

/// A checker for the bodies of `t` declared in `unit`.
fn checker<'a>(mm: &'a Metamodel, env: &'a Environment, t: &'a Transformation, unit: usize) -> Checker<'a> {
    Checker::for_language(mm, env, t, &t.units.0[unit].aliases)
}

/// What checking the bodies collects.
#[derive(Default)]
struct Output {
    diags: Vec<Diagnostic>,
    types: Vec<(Span, Type)>,
    scopes: Vec<Scope>,
}

impl Output {
    fn take(&mut self, c: &mut Checker<'_>) {
        self.diags.extend(c.take_diagnostics());
        self.types.extend(c.take_types());
    }
}

fn show(mm: &Metamodel, t: &Type) -> String {
    t.display(mm).to_string()
}

/// A loaded unit: its syntax and the units its imports name.
struct Loaded {
    doc: Document,
    /// For each import of the document, the unit it names (if found).
    imports: Vec<Option<usize>>,
    /// The import of the document compiled through which the unit was loaded.
    via: Option<usize>,
}

/// Loads the document and the units it imports, transitively.
struct Loader<'a> {
    units: UnitLookup<'a>,
    loaded: Vec<Loaded>,
    infos: Vec<Unit>,
    by_path: HashMap<String, usize>,
    next_offset: u32,
    diags: Vec<Diagnostic>,
}

impl Loader<'_> {
    fn load(&mut self, path: Option<String>, text: String, via: Option<usize>) -> usize {
        let offset = self.next_offset;
        self.next_offset = offset.saturating_add(text.len() as u32 + 1);
        let (doc, diags) = syntax::parse_at(&text, offset);
        self.diags.extend(diags);
        let index = self.loaded.len();
        if let Some(p) = &path {
            self.by_path.insert(p.clone(), index);
        }
        let module = doc.transformation.as_ref().map(|h| (h.kind, h.name.name.clone()));
        let imports = doc.imports.clone();
        self.infos.push(Unit { path: path.clone(), offset, text, module, aliases: Rc::from(Vec::new()) });
        self.loaded.push(Loaded { doc, imports: Vec::new(), via });
        for (n, import) in imports.iter().enumerate() {
            let via = via.or((index == 0).then_some(n));
            let names: Vec<String> = import.path.iter().map(|i| i.name.clone()).collect();
            let target = match (self.units)(path.as_deref(), &names) {
                Some(unit) => match self.by_path.get(&unit.path) {
                    Some(i) => Some(*i),
                    None if self.loaded.len() < MAX_UNITS => Some(self.load(Some(unit.path), unit.text, via)),
                    None => {
                        self.diags.push(Diagnostic::error(import.span, "too many imported units"));
                        None
                    }
                },
                // Eclipse QVTo's old built-in library: its operations are String's now.
                None if names == ["Strings"] => None,
                None => {
                    let file = format!("{}.qvto", names.join("/"));
                    self.diags.push(Diagnostic::error(
                        import.span,
                        format!(
                            "cannot find the unit '{}': {file} next to the importing file or in a folder above it",
                            import.name()
                        ),
                    ));
                    None
                }
            };
            self.loaded[index].imports.push(target);
        }
        index
    }
}

/// A URI of an imported unit's model type: relative ones are relative to its file.
fn unit_uri(unit: &Unit, uri: &str) -> String {
    let relative = !uri.contains(':') && !uri.starts_with('/');
    match (&unit.path, relative) {
        (Some(path), true) => match std::path::Path::new(path).parent() {
            Some(dir) => dir.join(uri).to_string_lossy().into_owned(),
            None => uri.to_string(),
        },
        _ => uri.to_string(),
    }
}

/// The package aliases of a unit's model types.
fn model_types(
    mm: &Metamodel,
    unit: &Unit,
    doc: &Document,
    lookup: PackageLookup,
    diags: &mut Vec<Diagnostic>,
    symbols: &mut Vec<Symbol>,
    uris: &mut Vec<(String, Span)>,
) -> Vec<PackageAlias> {
    let mut aliases: Vec<PackageAlias> = Vec::new();
    for mt in &doc.modeltypes {
        let mut packages: Vec<PackageId> = Vec::new();
        for r in &mt.packages {
            let found = match (&r.uri, &r.name) {
                (Some((uri, span)), _) => {
                    uris.push((uri.clone(), *span));
                    let found = lookup(&unit_uri(unit, uri));
                    if found.is_empty() {
                        diags.push(Diagnostic::error(
                            *span,
                            format!(
                                "cannot find the metamodel '{uri}': open it, or register its file in the package registry"
                            ),
                        ));
                    }
                    found
                }
                (None, Some(path)) => {
                    let names: Vec<&str> = path.iter().map(|i| i.name.as_str()).collect();
                    let found = mm.find_packages_qualified(&names);
                    if found.is_empty() {
                        diags.push(Diagnostic::error(r.span, format!("unknown package '{}'", names.join("::"))));
                    }
                    found
                }
                (None, None) => Vec::new(),
            };
            for p in found {
                if !packages.contains(&p) {
                    packages.push(p);
                }
            }
        }
        if aliases.iter().any(|a| a.name == mt.name.name) {
            diags.push(Diagnostic::error(mt.name.span, format!("duplicate model type '{}'", mt.name.name)));
            continue;
        }
        let uses: Vec<String> = mt
            .packages
            .iter()
            .map(|r| match (&r.uri, &r.name) {
                (Some((u, _)), _) => format!("'{u}'"),
                (None, Some(p)) => oxy_ocl::host::path_text(p),
                _ => String::new(),
            })
            .collect();
        symbols.push(Symbol {
            name: mt.name.name.clone(),
            kind: "modeltype",
            detail: format!("modeltype {} uses {}", mt.name.name, uses.join(", ")),
            span: mt.name.span,
            range: mt.span,
        });
        aliases.push(PackageAlias { name: mt.name.name.clone(), packages });
    }
    aliases
}

/// What building the modules of a compilation shares.
struct Context<'a> {
    mm: &'a Metamodel,
    env: &'a Environment,
    loaded: &'a [Loaded],
    units: Rc<Units>,
}

impl Context<'_> {
    fn kind(&self, unit: usize) -> Option<ModuleKind> {
        self.units.0[unit].module.as_ref().map(|(k, _)| *k)
    }

    /// Whether importing the unit makes its elements available: a library, or an old-style
    /// transformation without model parameters (which Eclipse QVTo treats like one).
    fn used_by_import(&self, unit: usize) -> bool {
        match &self.loaded[unit].doc.transformation {
            Some(h) => h.kind == ModuleKind::Library || h.params.is_empty(),
            None => false,
        }
    }

    /// The unit of the module a usage in `unit` names: one it imports, else any loaded one.
    fn find_module(&self, unit: usize, path: &[Ident]) -> Option<usize> {
        let name = &path.last()?.name;
        let named = |u: &usize| self.units.0[*u].module.as_ref().is_some_and(|(_, n)| n == name);
        self.loaded[unit].imports.iter().flatten().copied().find(named).or_else(|| (0..self.loaded.len()).find(named))
    }
}

pub(crate) fn compile(
    mm: &Metamodel,
    env: &Environment,
    text: &str,
    lookup: PackageLookup,
    unit_lookup: UnitLookup,
) -> Compiled {
    let mut loader = Loader {
        units: unit_lookup,
        loaded: Vec::new(),
        infos: Vec::new(),
        by_path: HashMap::new(),
        next_offset: 0,
        diags: Vec::new(),
    };
    loader.load(None, text.to_string(), None);
    let Loader { loaded, mut infos, mut diags, .. } = loader;

    // Model types of every unit (only the document's are symbols).
    let mut symbols = Vec::new();
    let mut uris = Vec::new();
    for (i, l) in loaded.iter().enumerate() {
        let (mut s, mut u) = (Vec::new(), Vec::new());
        let aliases = model_types(mm, &infos[i], &l.doc, lookup, &mut diags, &mut s, &mut u);
        infos[i].aliases = Rc::from(aliases);
        if i == 0 {
            symbols = s;
            uris = u;
        }
    }
    for (i, l) in loaded.iter().enumerate().skip(1) {
        if l.doc.transformation.is_none() {
            let via = l.via.map_or(Span::new(0, 0), |v| loaded[0].doc.imports[v].span);
            let path = infos[i].path.clone().unwrap_or_default();
            diags.push(Diagnostic::error(via, format!("{path} defines no transformation or library")));
        }
    }
    let unit_paths: Vec<String> = infos.iter().filter_map(|u| u.path.clone()).collect();
    let cx = Context { mm, env, loaded: &loaded, units: Rc::new(Units(infos)) };
    let library = cx.kind(0) == Some(ModuleKind::Library);
    let mut parameters = Vec::new();
    let mut out = Output::default();
    let t = build(&cx, 0, &mut vec![0], &mut diags, &mut symbols, &mut parameters, &mut out);
    if loaded[0].doc.transformation.is_none() && !text.trim().is_empty() {
        diags.push(Diagnostic::error(Span::new(0, 0), "missing 'transformation' declaration"));
    }

    // Problems of imported units are reported at the import that brought them in.
    let root_len = text.len() as u32;
    let mut summarized: Vec<usize> = Vec::new();
    let mut kept = Vec::new();
    diags.sort_by_key(|d| (d.span.start, d.span.end));
    diags.dedup_by(|a, b| a.span == b.span && a.message == b.message);
    for d in diags {
        let unit = cx.units.of(d.span);
        if unit == 0 {
            kept.push(d);
            continue;
        }
        if !d.is_error() || summarized.contains(&unit) {
            continue;
        }
        summarized.push(unit);
        let u = &cx.units.0[unit];
        let via = loaded[unit].via.map_or(Span::new(0, 0), |v| loaded[0].doc.imports[v].span);
        let more = format!("{}:{}: {}", u.path.as_deref().unwrap_or_default(), u.line(d.span.start), d.message);
        kept.push(Diagnostic::error(via, format!("the imported unit has errors: {more}")));
    }
    kept.sort_by_key(|d| (d.span.start, d.span.end));
    out.types.retain(|(s, _)| s.end <= root_len);
    out.scopes.retain(|s| s.span.end <= root_len);

    let name = t.name.clone();
    Compiled {
        language: Language::Operational,
        name: (!name.is_empty()).then_some(name),
        parameters,
        diagnostics: kept,
        types: out.types,
        scopes: out.scopes,
        symbols,
        uris,
        library,
        units: unit_paths,
        inner: Inner::Operational(Box::new(t)),
    }
}

/// Builds the module of `unit`: its own elements, those of the libraries it imports or uses and
/// of the transformations it extends (which it overrides), and the transformations it accesses.
/// `chain` holds the modules being built (to stop cyclic access).
fn build(
    cx: &Context<'_>,
    unit: usize,
    chain: &mut Vec<usize>,
    diags: &mut Vec<Diagnostic>,
    symbols: &mut Vec<Symbol>,
    parameters: &mut Vec<Parameter>,
    out: &mut Output,
) -> Transformation {
    let mm = cx.mm;
    let env = cx.env;
    let doc = &cx.loaded[unit].doc;
    let unit_aliases = cx.units.0[unit].aliases.clone();

    // The transformation's parameters.
    let mut params: Vec<ModelParamInfo> = Vec::new();
    let name = match &doc.transformation {
        Some(h) => {
            for p in &h.params {
                let Some(alias) = unit_aliases.iter().find(|a| a.name == p.model_type.name) else {
                    diags.push(Diagnostic::error(
                        p.model_type.span,
                        format!("unknown model type '{}' (declare it with 'modeltype')", p.model_type.name),
                    ));
                    continue;
                };
                if h.kind == ModuleKind::Library {
                    continue;
                }
                let name = p.name.as_ref().map_or(p.model_type.name.clone(), |n| n.name.clone());
                if params.iter().any(|x| x.name == name) {
                    diags.push(Diagnostic::error(p.span, format!("duplicate parameter '{name}'")));
                    continue;
                }
                symbols.push(Symbol {
                    name: name.clone(),
                    kind: "parameter",
                    detail: format!("{} {name} : {}", p.direction.keyword(), p.model_type.name),
                    span: p.name.as_ref().map_or(p.model_type.span, |n| n.span),
                    range: p.span,
                });
                params.push(ModelParamInfo {
                    name: name.clone(),
                    direction: p.direction,
                    model_type: p.model_type.name.clone(),
                    packages: alias.packages.clone(),
                });
                parameters.push(Parameter {
                    name,
                    direction: Some(p.direction),
                    model_type: p.model_type.name.clone(),
                    packages: alias.packages.clone(),
                    span: p.span,
                });
            }
            let kind = if h.kind == ModuleKind::Library { "library" } else { "transformation" };
            symbols.push(Symbol {
                name: h.name.name.clone(),
                kind,
                detail: format!(
                    "{kind} {}({})",
                    h.name.name,
                    h.params
                        .iter()
                        .map(|p| match (&p.name, h.kind) {
                            (_, ModuleKind::Library) => p.model_type.name.clone(),
                            (Some(n), _) => format!("{} {} : {}", p.direction.keyword(), n.name, p.model_type.name),
                            (None, _) => format!("{} {}", p.direction.keyword(), p.model_type.name),
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                span: h.name.span,
                range: h.span,
            });
            h.name.name.clone()
        }
        None => String::new(),
    };

    // The modules whose elements become the module's (the closest first) and the
    // transformations it accesses.
    let mut members = vec![unit];
    let mut accessed_units: Vec<usize> = Vec::new();
    let mut next = 0;
    while next < members.len() {
        let x = members[next];
        next += 1;
        // Imported libraries can be used without `access` (as in Eclipse QVTo), imported
        // transformations instantiated (`new T(...)`) — unless the module accesses them.
        let explicitly_accessed: Vec<usize> = cx.loaded[x]
            .doc
            .transformation
            .iter()
            .flat_map(|h| &h.uses)
            .filter(|u| u.kind == UsageKind::Access)
            .filter_map(|u| cx.find_module(x, &u.path))
            .collect();
        for target in cx.loaded[x].imports.iter().flatten() {
            if explicitly_accessed.contains(target) || *target == unit {
                continue;
            }
            if cx.used_by_import(*target) {
                if !members.contains(target) {
                    members.push(*target);
                }
            } else if x == unit
                && cx.kind(*target) == Some(ModuleKind::Transformation)
                && !accessed_units.contains(target)
            {
                accessed_units.push(*target);
            }
        }
        let Some(h) = &cx.loaded[x].doc.transformation else { continue };
        for usage in &h.uses {
            let Some(m) = cx.find_module(x, &usage.path) else {
                let name = &usage.name().name;
                diags.push(Diagnostic::error(usage.span, format!("unknown module '{name}' (import its unit)")));
                continue;
            };
            let kind = cx.kind(m);
            if let (Some(written), Some(actual)) = (usage.module_kind, kind)
                && written != actual
            {
                let what = |k| if k == ModuleKind::Library { "library" } else { "transformation" };
                let msg = format!("'{}' is a {}, not a {}", usage.name().name, what(actual), what(written));
                diags.push(Diagnostic::error(usage.span, msg));
            }
            if m == unit || chain.contains(&m) && usage.kind == UsageKind::Extends {
                diags.push(Diagnostic::error(usage.span, format!("'{}' cannot use itself", usage.name().name)));
                continue;
            }
            match (usage.kind, kind) {
                (UsageKind::Extends, _) => {
                    accessed_units.retain(|a| *a != m);
                    if !members.contains(&m) {
                        members.push(m);
                    }
                }
                (UsageKind::Access, Some(ModuleKind::Transformation)) => {
                    if !accessed_units.contains(&m) {
                        accessed_units.push(m);
                    }
                }
                _ => {
                    if !members.contains(&m) {
                        members.push(m);
                    }
                }
            }
        }
    }
    let modules: Vec<String> = members
        .iter()
        .filter_map(|u| cx.units.0[*u].module.as_ref().map(|(_, n)| n.clone()))
        .filter(|n| *n != name)
        .collect();

    let mut t = Transformation {
        name: name.clone(),
        library: cx.kind(unit) == Some(ModuleKind::Library),
        units: cx.units.clone(),
        aliases: unit_aliases,
        modules,
        accessed: Vec::new(),
        param_aliases: Vec::new(),
        params,
        mappings: Vec::new(),
        helpers: Vec::new(),
        properties: Vec::new(),
        main: None,
        main_unit: unit,
        main_params: Vec::new(),
        entry_mapping: None,
        entry_helper: None,
        implicit_output: false,
    };

    // The model parameters of extended transformations stand for the module's (by position).
    for &x in members.iter().skip(1) {
        let Some(h) = &cx.loaded[x].doc.transformation else { continue };
        if h.kind != ModuleKind::Transformation {
            continue;
        }
        for (p, own) in h.params.iter().zip(&t.params) {
            let alias = p.name.as_ref().map_or(&p.model_type.name, |n| &n.name);
            if *alias != own.name && t.params.iter().all(|q| q.name != *alias) {
                t.param_aliases.push((alias.clone(), own.name.clone()));
            }
        }
    }

    // Signatures (types are resolved with a checker that only needs the unit's aliases).
    for &x in &members {
        let aliases = cx.units.0[x].aliases.clone();
        let mut resolver = Checker::for_language(mm, env, &NoHost, &aliases);
        let sig = |params: &[syntax::Param], resolver: &mut Checker<'_>| -> Vec<Sig> {
            params
                .iter()
                .enumerate()
                .map(|(i, p)| Sig {
                    name: p.name.as_ref().map_or_else(|| format!("p{i}"), |n| n.name.clone()),
                    ty: resolver.resolve_type(&p.ty),
                })
                .collect()
        };
        let library_unit = cx.kind(x) == Some(ModuleKind::Library);
        for e in &cx.loaded[x].doc.elements {
            match e {
                Element::Mapping(m) => {
                    let context = m.context.as_ref().map(|c| resolver.resolve_type(c));
                    let params = sig(&m.params, &mut resolver);
                    let results = m
                        .results
                        .iter()
                        .map(|r| ResultSig {
                            name: r.name.as_ref().map(|n| n.name.clone()),
                            ty: resolver.resolve_type(&r.ty),
                            extent: r.extent.as_ref().map(|x| x.name.clone()),
                        })
                        .collect();
                    t.mappings.push((
                        m.clone(),
                        MappingSig {
                            unit: x,
                            overridden: false,
                            context,
                            params,
                            results,
                            inherits: Vec::new(),
                            merges: Vec::new(),
                            disjuncts: Vec::new(),
                        },
                    ));
                }
                Element::Helper(h) => {
                    let context = h.context.as_ref().map(|c| resolver.resolve_type(c));
                    let params = sig(&h.params, &mut resolver);
                    let result = match h.results.as_slice() {
                        [] => Type::OclVoid,
                        [r] => resolver.resolve_type(&r.ty),
                        rs => Type::tuple(
                            rs.iter()
                                .enumerate()
                                .map(|(i, r)| {
                                    (
                                        r.name.as_ref().map_or_else(|| format!("r{i}"), |n| n.name.clone()),
                                        resolver.resolve_type(&r.ty),
                                    )
                                })
                                .collect(),
                        ),
                    };
                    t.helpers.push((h.clone(), HelperSig { unit: x, overridden: false, context, params, result }));
                }
                Element::Property(p) => {
                    let ty = p.ty.as_ref().map(|t| resolver.resolve_type(t));
                    let context = match p.context.as_ref().map(|c| resolver.resolve_type(c)) {
                        Some(Type::Classifier(c)) if mm.is_class(c) => Some(c),
                        Some(Type::Error) | None => None,
                        Some(other) => {
                            let span = p.context.as_ref().map_or(p.span, |c| c.span);
                            resolver.report_error(
                                span,
                                format!("intermediate properties belong to classes, not to {}", show(mm, &other)),
                            );
                            None
                        }
                    };
                    t.properties.push((
                        p.clone(),
                        PropertySig { unit: x, overridden: false, ty: ty.unwrap_or(Type::Error), context },
                    ));
                }
                Element::Main(m) => {
                    if library_unit {
                        diags.push(Diagnostic::error(m.span, "a library has no 'main'"));
                    } else if x == unit && t.main.is_some() {
                        diags.push(Diagnostic::error(m.span, "duplicate 'main'"));
                    } else if t.main.is_none() {
                        // Its own, else that of a transformation it extends.
                        t.main = Some(m.clone());
                        t.main_unit = x;
                        t.main_params = sig(&m.params, &mut resolver);
                    }
                }
            }
        }
        diags.extend(resolver.take_diagnostics());
    }

    // Overriding: of elements with the same signature, the one of the closest module counts.
    for i in 0..t.mappings.len() {
        let (m, s) = &t.mappings[i];
        let hidden = t.mappings[..i].iter().any(|(o, os)| {
            os.unit != s.unit
                && o.name.name == m.name.name
                && os.context == s.context
                && same_params(&os.params, &s.params)
        });
        t.mappings[i].1.overridden = hidden;
    }
    for i in 0..t.helpers.len() {
        let (h, s) = &t.helpers[i];
        let hidden = t.helpers[..i].iter().any(|(o, os)| {
            os.unit != s.unit
                && o.name.name == h.name.name
                && os.context == s.context
                && same_params(&os.params, &s.params)
        });
        t.helpers[i].1.overridden = hidden;
    }
    for i in 0..t.properties.len() {
        let (p, s) = &t.properties[i];
        let hidden = t.properties[..i]
            .iter()
            .any(|(o, os)| os.unit != s.unit && o.name.name == p.name.name && os.context == s.context);
        t.properties[i].1.overridden = hidden;
    }

    if t.main.is_none() {
        t.entry_mapping =
            t.mappings.iter().position(|(m, s)| m.name.name == "main" && s.context.is_none() && !s.overridden);
        if t.entry_mapping.is_none() {
            t.entry_helper =
                t.helpers.iter().position(|(h, s)| h.name.name == "main" && s.context.is_none() && !s.overridden);
        }
    }
    // Old-style transformations without model parameters (`transformation t;`): the inputs are
    // the parameters of `main`, the output its result.
    let is_transformation = doc.transformation.as_ref().is_some_and(|h| h.kind == ModuleKind::Transformation);
    if t.params.is_empty() && is_transformation {
        let (inputs, output, span): (Vec<Sig>, Option<Type>, Span) = match (&t.main, t.entry_mapping, t.entry_helper) {
            (Some(m), _, _) => (t.main_params.clone(), None, m.span),
            (None, Some(i), _) => {
                (t.mappings[i].1.params.clone(), Some(t.mappings[i].1.result_type()), t.mappings[i].0.span)
            }
            (None, None, Some(i)) => {
                (t.helpers[i].1.params.clone(), Some(t.helpers[i].1.result.clone()), t.helpers[i].0.span)
            }
            (None, None, None) => (Vec::new(), None, Span::new(0, 0)),
        };
        let model_type = |ty: &Type| -> Option<&PackageAlias> {
            let Type::Classifier(c) = ty else { return None };
            let mut p = Some(mm.classifier(*c).package);
            let mut packages = Vec::new();
            while let Some(x) = p {
                packages.push(x);
                p = mm.package(x).super_package;
            }
            t.aliases.iter().find(|a| a.packages.iter().any(|x| packages.contains(x)))
        };
        let mut implicit = Vec::new();
        for p in &inputs {
            if let Some(a) = model_type(&p.ty) {
                implicit.push((p.name.clone(), Direction::In, a.clone()));
            }
        }
        if let Some(a) = output.as_ref().and_then(model_type) {
            implicit.push(("result".to_string(), Direction::Out, a.clone()));
            t.implicit_output = true;
        }
        for (name, direction, alias) in implicit {
            parameters.push(Parameter {
                name: name.clone(),
                direction: Some(direction),
                model_type: alias.name.clone(),
                packages: alias.packages.clone(),
                span,
            });
            t.params.push(ModelParamInfo { name, direction, model_type: alias.name, packages: alias.packages });
        }
    }

    // The transformations it accesses, compiled on their own.
    for m in accessed_units {
        if chain.contains(&m) || chain.len() >= MAX_ACCESS_DEPTH {
            continue;
        }
        chain.push(m);
        let (mut s, mut p, mut o) = (Vec::new(), Vec::new(), Output::default());
        let accessed = build(cx, m, chain, diags, &mut s, &mut p, &mut o);
        diags.extend(o.diags);
        chain.pop();
        t.accessed.push(accessed);
    }

    // Mapping references, duplicates, symbols.
    let references = |t: &Transformation, refs: &[Vec<Ident>], diags: &mut Vec<Diagnostic>| -> Vec<usize> {
        refs.iter()
            .filter_map(|path| {
                let (last, qualifier) = path.split_last()?;
                let found = t.mappings.iter().position(|(m, s)| {
                    m.name.name == last.name && !s.overridden && names_context(mm, &s.context, qualifier)
                });
                if found.is_none() {
                    let span = path[0].span.to(last.span);
                    diags
                        .push(Diagnostic::error(span, format!("unknown mapping '{}'", oxy_ocl::host::path_text(path))));
                }
                found
            })
            .collect()
    };
    for i in 0..t.mappings.len() {
        let (inherits, merges, disjuncts) = {
            let m = &t.mappings[i].0;
            (references(&t, &m.inherits, diags), references(&t, &m.merges, diags), references(&t, &m.disjuncts, diags))
        };
        let (m, s) = &t.mappings[i];
        if t.mappings[..i].iter().any(|(o, os)| {
            os.unit == s.unit
                && o.name.name == m.name.name
                && os.context == s.context
                && same_params(&os.params, &s.params)
        }) {
            diags.push(Diagnostic::error(m.name.span, format!("duplicate mapping '{}'", m.name.name)));
        }
        if m.body.is_none() && !m.is_abstract && disjuncts.is_empty() {
            diags.push(Diagnostic::error(m.name.span, format!("the mapping '{}' has no body", m.name.name)));
        }
        if m.results.len() > 1 {
            diags.push(Diagnostic::error(m.name.span, "mappings with several results are not supported"));
        }
        for r in &s.results {
            if let Some(x) = &r.extent
                && let Err(e) = t.extent(x, None)
                && s.unit == unit
            {
                let span = m
                    .results
                    .iter()
                    .find_map(|p| p.extent.as_ref().filter(|e| e.name == *x))
                    .map_or(m.span, |e| e.span);
                diags.push(Diagnostic::error(span, e));
            }
        }
        if !s.overridden {
            symbols.push(Symbol {
                name: m.name.name.clone(),
                kind: "mapping",
                detail: signature(mm, "mapping", &m.name.name, &s.context, &s.params, &s.result_type()),
                span: m.name.span,
                range: m.span,
            });
        }
        let sig = &mut t.mappings[i].1;
        sig.inherits = inherits;
        sig.merges = merges;
        sig.disjuncts = disjuncts;
    }
    for (i, (h, s)) in t.helpers.iter().enumerate() {
        if t.helpers[..i].iter().any(|(o, os)| {
            os.unit == s.unit
                && o.name.name == h.name.name
                && os.context == s.context
                && same_params(&os.params, &s.params)
        }) {
            diags.push(Diagnostic::error(h.name.span, format!("duplicate {} '{}'", kind_of(h), h.name.name)));
        }
        if matches!(h.body, HelperBody::None) {
            diags.push(Diagnostic::error(h.name.span, format!("the {} '{}' has no body", kind_of(h), h.name.name)));
        }
        if !s.overridden || s.unit == unit {
            symbols.push(Symbol {
                name: h.name.name.clone(),
                kind: kind_of(h),
                detail: signature(mm, kind_of(h), &h.name.name, &s.context, &s.params, &s.result),
                span: h.name.span,
                range: h.span,
            });
        }
    }
    for (i, (p, s)) in t.properties.iter().enumerate() {
        if t.properties[..i]
            .iter()
            .any(|(o, os)| os.unit == s.unit && o.name.name == p.name.name && os.context == s.context)
        {
            diags.push(Diagnostic::error(p.name.span, format!("duplicate property '{}'", p.name.name)));
        }
        let kind = match p.kind {
            PropertyKind::Module => "property",
            PropertyKind::Configuration => "configuration property",
            PropertyKind::Intermediate => "intermediate property",
        };
        let owner = s.context.map(|c| format!("{}::", mm.classifier(c).name)).unwrap_or_default();
        if !s.overridden {
            symbols.push(Symbol {
                name: p.name.name.clone(),
                kind: "property",
                detail: format!("{kind} {owner}{} : {}", p.name.name, show(mm, &s.ty)),
                span: p.name.span,
                range: p.span,
            });
        }
    }
    // (Accessed transformations need none: they may be used for their operations.)
    let entry = t.main.is_some() || t.entry_mapping.is_some() || t.entry_helper.is_some();
    if !entry && is_transformation && unit == 0 {
        let span = doc.transformation.as_ref().map_or(Span::new(0, 0), |h| h.name.span);
        diags.push(Diagnostic::error(span, "the transformation has no 'main'"));
    }

    // Properties without a type have the type of their value.
    let inferred: Vec<(usize, Type)> = t
        .properties
        .iter()
        .enumerate()
        .filter(|(_, (p, _))| p.ty.is_none())
        .filter_map(|(i, (p, s))| {
            let init = p.init.as_ref()?;
            let mut c = checker(mm, env, &t, s.unit);
            Some((i, c.check(init)))
        })
        .collect();
    for (i, ty) in inferred {
        t.properties[i].1.ty = ty;
    }

    // Bodies.
    check_bodies(mm, env, &t, out);
    diags.append(&mut out.diags);
    t
}

fn same_params(a: &[Sig], b: &[Sig]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.ty == y.ty)
}

fn kind_of(h: &Helper) -> &'static str {
    if h.is_query { "query" } else { "helper" }
}

fn signature(mm: &Metamodel, kind: &str, name: &str, context: &Option<Type>, params: &[Sig], result: &Type) -> String {
    let ctx = context.as_ref().map(|c| format!("{}::", show(mm, c))).unwrap_or_default();
    let params: Vec<String> = params.iter().map(|p| format!("{} : {}", p.name, show(mm, &p.ty))).collect();
    let result = if *result == Type::OclVoid { String::new() } else { format!(" : {}", show(mm, result)) };
    format!("{kind} {ctx}{name}({}){result}", params.join(", "))
}

/// `_name` names `name` (QVTo's escape of names that are keywords).
pub(crate) fn escaped(name: &str) -> Option<&str> {
    name.strip_prefix('_').filter(|n| !n.is_empty())
}

/// A host that knows nothing (for resolving types).
struct NoHost;

impl CheckHost for NoHost {}

fn check_statements(c: &mut Checker<'_>, items: &[Expr]) {
    for e in items {
        c.check(e);
    }
}

fn check_conditions(c: &mut Checker<'_>, items: &[Expr], what: &str) {
    for e in items {
        c.expect(e, &Type::Boolean, what);
    }
}

fn bind_params(c: &mut Checker<'_>, context: &Option<Type>, params: &[Sig]) -> Vec<(String, Type)> {
    let mut vars = Vec::new();
    if let Some(ctx) = context {
        c.bind_self(ctx.clone(), None);
        vars.push(("self".to_string(), ctx.clone()));
    }
    for p in params {
        c.bind(&p.name, p.ty.clone());
        vars.push((p.name.clone(), p.ty.clone()));
    }
    vars
}

fn check_bodies(mm: &Metamodel, env: &Environment, t: &Transformation, out: &mut Output) {
    for (m, s) in &t.mappings {
        let mut c = checker(mm, env, t, s.unit);
        let mut vars = bind_params(&mut c, &s.context, &s.params);
        let result = s.result_type();
        if !s.results.is_empty() {
            c.bind("result", result.clone());
            vars.push(("result".into(), result.clone()));
            for r in &s.results {
                if let Some(n) = &r.name {
                    c.bind(n, r.ty.clone());
                    vars.push((n.clone(), r.ty.clone()));
                }
            }
        }
        if let Some(when) = &m.when {
            check_conditions(&mut c, when, "a guard (when)");
        }
        if let Some(init) = &m.init {
            check_statements(&mut c, init);
        }
        let populated = match s.results.as_slice() {
            _ if m.population_keyword => None,
            [r] if matches!(r.ty, Type::Classifier(cl) if mm.is_class(cl)) => Some(r.ty.clone()),
            [] => s.context.clone().filter(|_| m.direction == Some(Direction::InOut)),
            _ => None,
        };
        if let Some(p) = populated {
            c.push_object_scope(p);
            check_statements(&mut c, &m.population);
            c.pop_scope();
        } else {
            check_statements(&mut c, &m.population);
        }
        if let Some(end) = &m.end {
            check_statements(&mut c, end);
        }
        if let Some(w) = &m.where_ {
            check_conditions(&mut c, w, "a postcondition (where)");
        }
        if let Some(d) = s.disjuncts.iter().find(|d| t.mappings[**d].1.params.len() != s.params.len()) {
            let msg = format!("'{}' has other parameters than this mapping", t.mappings[*d].0.name.name);
            c.report_error(m.name.span, msg);
        }
        c.take_returns();
        out.take(&mut c);
        out.scopes.push(Scope { span: m.span, variables: vars, self_type: s.context.clone() });
    }
    for (h, s) in &t.helpers {
        let mut c = checker(mm, env, t, s.unit);
        let vars = bind_params(&mut c, &s.context, &s.params);
        let what = format!("the result of '{}'", h.name.name);
        match &h.body {
            HelperBody::Expr(e) => {
                c.expect(e, &s.result, &what);
            }
            HelperBody::Block(items, _) => {
                check_statements(&mut c, items);
                for (span, rt) in c.take_returns() {
                    if !rt.conforms_to(&s.result, mm) && !rt.is_error() {
                        let msg = format!(
                            "{what} must be of type {}, but this returns {}",
                            show(mm, &s.result),
                            show(mm, &rt)
                        );
                        c.report_error(span, msg);
                    }
                }
            }
            HelperBody::None => {}
        }
        out.take(&mut c);
        out.scopes.push(Scope { span: h.span, variables: vars, self_type: s.context.clone() });
    }
    for (p, s) in &t.properties {
        if let Some(init) = &p.init {
            let mut c = checker(mm, env, t, s.unit);
            if let Some(ctx) = s.context {
                c.bind_self(Type::Classifier(ctx), None);
            }
            if p.ty.is_some() {
                c.expect(init, &s.ty, &format!("the value of '{}'", p.name.name));
            } else {
                c.check(init);
            }
            out.take(&mut c);
        }
    }
    if let Some(main) = &t.main {
        let mut c = checker(mm, env, t, t.main_unit);
        let vars = bind_params(&mut c, &None, &t.main_params);
        check_statements(&mut c, &main.body);
        c.take_returns();
        out.take(&mut c);
        out.scopes.push(Scope { span: main.span, variables: vars, self_type: None });
    }
}
