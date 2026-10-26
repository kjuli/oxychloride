//! Compiling a QVTo document: its model types become package aliases (`UML::Class`), its
//! mappings, helpers and properties get typed signatures, and every body is type checked by
//! OCL's checker with the transformation as its [`CheckHost`] (which knows the model
//! parameters, `this`, the properties, helpers and mappings).

use oxy_ecore::{ClassifierId, Metamodel, PackageId};
use oxy_ocl::syntax::ast::{Expr, Ident};
use oxy_ocl::{CheckHost, Checker, Diagnostic, Environment, PackageAlias, Span, Type};

use super::syntax::{self, Direction, Element, Helper, HelperBody, Main, Mapping, Property, PropertyKind};
use crate::{Compiled, Inner, Language, PackageLookup, Parameter, Scope, Symbol, library};

/// A compiled QVTo transformation.
pub struct Transformation {
    pub name: String,
    pub(crate) aliases: Vec<PackageAlias>,
    pub(crate) params: Vec<ModelParamInfo>,
    pub(crate) mappings: Vec<(Mapping, MappingSig)>,
    pub(crate) helpers: Vec<(Helper, HelperSig)>,
    pub(crate) properties: Vec<(Property, PropertySig)>,
    pub(crate) main: Option<Main>,
    /// The parameters of `main`, bound to root objects of the input models.
    pub(crate) main_params: Vec<Sig>,
    /// Without `main`: a mapping `main` without a context, the entry of old-style QVTo.
    pub(crate) entry_mapping: Option<usize>,
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
    pub context: Option<Type>,
    pub params: Vec<Sig>,
    pub results: Vec<ResultSig>,
    pub inherits: Vec<usize>,
    pub merges: Vec<usize>,
    pub disjuncts: Vec<usize>,
}

pub(crate) struct HelperSig {
    pub context: Option<Type>,
    pub params: Vec<Sig>,
    pub result: Type,
}

pub(crate) struct PropertySig {
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
        source.filter(|t| !matches!(t, Type::Model(m) if **m == *self.name))
    }

    pub(crate) fn param(&self, name: &str) -> Option<&ModelParamInfo> {
        self.params.iter().find(|p| p.name == name)
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
        let named: Vec<usize> =
            self.helpers.iter().enumerate().filter(|(_, (h, _))| h.name.name == name).map(|(i, _)| i).collect();
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
            .filter(|(_, (m, s))| m.name.name == last.name && names_context(mm, &s.context, qualifier))
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
            .find(|(p, _)| p.kind != PropertyKind::Intermediate && p.name.name == name)
            .map(|(_, s)| s.ty.clone())
    }

    fn assignable(&self, name: &str) -> bool {
        self.properties.iter().any(|(p, _)| p.kind == PropertyKind::Module && p.name.name == name)
    }

    fn operation(
        &self,
        mm: &Metamodel,
        source: Option<&Type>,
        name: &str,
        args: &[Type],
    ) -> Option<Result<Type, String>> {
        let source = self.without_this(source);
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
            Type::Model(m) if **m == *self.name => self
                .properties
                .iter()
                .find(|(p, _)| p.kind != PropertyKind::Intermediate && p.name.name == name)
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

    fn can_create(&self, mm: &Metamodel, class: ClassifierId) -> Result<(), String> {
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

/// A checker for the bodies of `t`.
fn checker<'a>(mm: &'a Metamodel, env: &'a Environment, t: &'a Transformation) -> Checker<'a> {
    Checker::for_language(mm, env, t, &t.aliases)
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

pub(crate) fn compile(mm: &Metamodel, env: &Environment, text: &str, lookup: PackageLookup) -> Compiled {
    let (doc, mut diags) = syntax::parse(text);
    for s in &doc.imports {
        diags.push(Diagnostic::error(*s, "importing other transformations and libraries is not supported"));
    }
    let mut symbols = Vec::new();
    let mut uris = Vec::new();

    // Model types.
    let mut aliases: Vec<PackageAlias> = Vec::new();
    for mt in &doc.modeltypes {
        let mut packages: Vec<PackageId> = Vec::new();
        for r in &mt.packages {
            let found = match (&r.uri, &r.name) {
                (Some((uri, span)), _) => {
                    uris.push((uri.clone(), *span));
                    let found = lookup(uri);
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

    // The transformation's parameters.
    let mut params: Vec<ModelParamInfo> = Vec::new();
    let mut parameters: Vec<Parameter> = Vec::new();
    let name = match &doc.transformation {
        Some(h) => {
            for p in &h.params {
                let Some(alias) = aliases.iter().find(|a| a.name == p.model_type.name) else {
                    diags.push(Diagnostic::error(
                        p.model_type.span,
                        format!("unknown model type '{}' (declare it with 'modeltype')", p.model_type.name),
                    ));
                    continue;
                };
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
            symbols.push(Symbol {
                name: h.name.name.clone(),
                kind: "transformation",
                detail: format!(
                    "transformation {}({})",
                    h.name.name,
                    h.params
                        .iter()
                        .map(|p| match &p.name {
                            Some(n) => format!("{} {} : {}", p.direction.keyword(), n.name, p.model_type.name),
                            None => format!("{} {}", p.direction.keyword(), p.model_type.name),
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                span: h.name.span,
                range: h.span,
            });
            h.name.name.clone()
        }
        None => {
            if !text.trim().is_empty() {
                diags.push(Diagnostic::error(Span::new(0, 0), "missing 'transformation' declaration"));
            }
            String::new()
        }
    };

    let mut t = Transformation {
        name: name.clone(),
        aliases,
        params,
        mappings: Vec::new(),
        helpers: Vec::new(),
        properties: Vec::new(),
        main: None,
        main_params: Vec::new(),
        entry_mapping: None,
        implicit_output: false,
    };

    // Signatures (types are resolved with a checker that only needs the aliases).
    let mut mapping_syntax = Vec::new();
    let mut helper_syntax = Vec::new();
    let mut property_syntax = Vec::new();
    for e in doc.elements {
        match e {
            Element::Mapping(m) => mapping_syntax.push(m),
            Element::Helper(h) => helper_syntax.push(h),
            Element::Property(p) => property_syntax.push(p),
            Element::Main(m) => {
                if t.main.is_some() {
                    diags.push(Diagnostic::error(m.span, "duplicate 'main'"));
                } else {
                    t.main = Some(m);
                }
            }
        }
    }
    {
        let mut resolver = Checker::for_language(mm, env, &NoHost, &t.aliases);
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
        let mut mappings = Vec::new();
        for m in mapping_syntax {
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
            mappings.push((
                m,
                MappingSig {
                    context,
                    params,
                    results,
                    inherits: Vec::new(),
                    merges: Vec::new(),
                    disjuncts: Vec::new(),
                },
            ));
        }
        let mut helpers = Vec::new();
        for h in helper_syntax {
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
            helpers.push((h, HelperSig { context, params, result }));
        }
        let mut properties = Vec::new();
        for p in property_syntax {
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
            properties.push((p, PropertySig { ty: ty.unwrap_or(Type::Error), context }));
        }
        if let Some(main) = &t.main {
            t.main_params = sig(&main.params, &mut resolver);
        }
        diags.extend(resolver.take_diagnostics());
        t.mappings = mappings;
        t.helpers = helpers;
        t.properties = properties;
    }
    if t.main.is_none() {
        t.entry_mapping = t.mappings.iter().position(|(m, s)| m.name.name == "main" && s.context.is_none());
    }
    // Old-style transformations without model parameters (`transformation t;`): the inputs are
    // the parameters of `main`, the output its result.
    if t.params.is_empty() && doc.transformation.is_some() {
        let (inputs, output, span): (Vec<Sig>, Option<Type>, Span) = match (&t.main, t.entry_mapping) {
            (Some(m), _) => (t.main_params.clone(), None, m.span),
            (None, Some(i)) => {
                (t.mappings[i].1.params.clone(), Some(t.mappings[i].1.result_type()), t.mappings[i].0.span)
            }
            (None, None) => (Vec::new(), None, Span::new(0, 0)),
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

    // Mapping references, duplicates, symbols.
    let references = |t: &Transformation, refs: &[Vec<Ident>], diags: &mut Vec<Diagnostic>| -> Vec<usize> {
        refs.iter()
            .filter_map(|path| {
                let (last, qualifier) = path.split_last()?;
                let found = t
                    .mappings
                    .iter()
                    .position(|(m, s)| m.name.name == last.name && names_context(mm, &s.context, qualifier));
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
            (
                references(&t, &m.inherits, &mut diags),
                references(&t, &m.merges, &mut diags),
                references(&t, &m.disjuncts, &mut diags),
            )
        };
        let (m, s) = &t.mappings[i];
        if t.mappings[..i]
            .iter()
            .any(|(o, os)| o.name.name == m.name.name && os.context == s.context && same_params(&os.params, &s.params))
        {
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
            {
                let span = m
                    .results
                    .iter()
                    .find_map(|p| p.extent.as_ref().filter(|e| e.name == *x))
                    .map_or(m.span, |e| e.span);
                diags.push(Diagnostic::error(span, e));
            }
        }
        symbols.push(Symbol {
            name: m.name.name.clone(),
            kind: "mapping",
            detail: signature(mm, "mapping", &m.name.name, &s.context, &s.params, &s.result_type()),
            span: m.name.span,
            range: m.span,
        });
        let sig = &mut t.mappings[i].1;
        sig.inherits = inherits;
        sig.merges = merges;
        sig.disjuncts = disjuncts;
    }
    for (i, (h, s)) in t.helpers.iter().enumerate() {
        if t.helpers[..i]
            .iter()
            .any(|(o, os)| o.name.name == h.name.name && os.context == s.context && same_params(&os.params, &s.params))
        {
            diags.push(Diagnostic::error(h.name.span, format!("duplicate {} '{}'", kind_of(h), h.name.name)));
        }
        if matches!(h.body, HelperBody::None) {
            diags.push(Diagnostic::error(h.name.span, format!("the {} '{}' has no body", kind_of(h), h.name.name)));
        }
        symbols.push(Symbol {
            name: h.name.name.clone(),
            kind: kind_of(h),
            detail: signature(mm, kind_of(h), &h.name.name, &s.context, &s.params, &s.result),
            span: h.name.span,
            range: h.span,
        });
    }
    for (i, (p, s)) in t.properties.iter().enumerate() {
        if t.properties[..i].iter().any(|(o, os)| o.name.name == p.name.name && os.context == s.context) {
            diags.push(Diagnostic::error(p.name.span, format!("duplicate property '{}'", p.name.name)));
        }
        let kind = match p.kind {
            PropertyKind::Module => "property",
            PropertyKind::Configuration => "configuration property",
            PropertyKind::Intermediate => "intermediate property",
        };
        let owner = s.context.map(|c| format!("{}::", mm.classifier(c).name)).unwrap_or_default();
        symbols.push(Symbol {
            name: p.name.name.clone(),
            kind: "property",
            detail: format!("{kind} {owner}{} : {}", p.name.name, show(mm, &s.ty)),
            span: p.name.span,
            range: p.span,
        });
    }
    if t.main.is_none() && t.entry_mapping.is_none() && doc.transformation.is_some() {
        let span = doc.transformation.as_ref().map_or(Span::new(0, 0), |h| h.name.span);
        diags.push(Diagnostic::error(span, "the transformation has no 'main'"));
    }

    // Properties without a type have the type of their value.
    let inferred: Vec<(usize, Type)> = t
        .properties
        .iter()
        .enumerate()
        .filter(|(_, (p, _))| p.ty.is_none())
        .filter_map(|(i, (p, _))| {
            let init = p.init.as_ref()?;
            let mut c = checker(mm, env, &t);
            Some((i, c.check(init)))
        })
        .collect();
    for (i, ty) in inferred {
        t.properties[i].1.ty = ty;
    }

    // Bodies.
    let mut out = Output::default();
    check_bodies(mm, env, &t, &mut out);
    diags.extend(out.diags);
    diags.sort_by_key(|d| (d.span.start, d.span.end));
    diags.dedup_by(|a, b| a.span == b.span && a.message == b.message);

    Compiled {
        language: Language::Operational,
        name: (!name.is_empty()).then_some(name),
        parameters,
        diagnostics: diags,
        types: out.types,
        scopes: out.scopes,
        symbols,
        uris,
        inner: Inner::Operational(Box::new(t)),
    }
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
        let mut c = checker(mm, env, t);
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
        let mut c = checker(mm, env, t);
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
            let mut c = checker(mm, env, t);
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
        let mut c = checker(mm, env, t);
        let vars = bind_params(&mut c, &None, &t.main_params);
        check_statements(&mut c, &main.body);
        c.take_returns();
        out.take(&mut c);
        out.scopes.push(Scope { span: main.span, variables: vars, self_type: None });
    }
}
