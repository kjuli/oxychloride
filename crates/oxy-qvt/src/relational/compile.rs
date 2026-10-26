//! Compiling a QVTr document: imports become package aliases, the templates of the domains
//! are resolved against the metamodel (classes, features, the types of their variables), and
//! every expression and predicate is type checked with the relation's variables bound and the
//! transformation (its queries and relations) as the checker's [`CheckHost`].

use oxy_ecore::{ClassifierId, FeatureId, Metamodel, PackageId};
use oxy_ocl::syntax::ast::{Expr, Ident, TypeExpr};
use oxy_ocl::types::CollectionKind;
use oxy_ocl::{CheckHost, Checker, Diagnostic, Environment, PackageAlias, Span, Type};

use super::syntax::{
    self, CollectionTemplate, DomainKind, Member, ObjectTemplate, Query, Relation, Template, TemplateValue,
    relation_call,
};
use crate::{Compiled, Inner, Language, PackageLookup, Parameter, Scope, Symbol, library};

/// A compiled QVTr transformation.
pub struct Transformation {
    pub name: String,
    pub(crate) aliases: Vec<PackageAlias>,
    pub(crate) params: Vec<ParamInfo>,
    pub(crate) keys: Vec<KeyInfo>,
    pub(crate) relations: Vec<RelationInfo>,
    pub(crate) queries: Vec<QueryInfo>,
}

pub(crate) struct ParamInfo {
    pub name: String,
    pub packages: Vec<PackageId>,
}

/// `key C { a, b, opposite(D.c) }`: objects of `C` with the same values of these properties
/// are the same object (enforcing reuses them).
pub(crate) struct KeyInfo {
    pub class: ClassifierId,
    pub features: Vec<FeatureId>,
    /// Features of other classes that refer to the object.
    pub opposites: Vec<FeatureId>,
}

pub(crate) struct QueryInfo {
    pub syntax: Query,
    pub params: Vec<(String, Type)>,
    pub result: Type,
}

pub(crate) struct RelationInfo {
    pub syntax: Relation,
    /// All variables: declared ones, those of templates and of primitive domains.
    pub vars: Vec<(String, Type)>,
    /// Initial values of declared variables.
    pub inits: Vec<(String, Expr)>,
    pub domains: Vec<DomainInfo>,
    /// The variables an invocation binds, in order: the roots of the domains' patterns and
    /// the variables of primitive domains.
    pub roots: Vec<String>,
    pub overrides: Option<usize>,
}

pub(crate) struct DomainInfo {
    pub kind: DomainKind,
    /// The model parameter (`None`: a primitive domain).
    pub param: Option<usize>,
    pub patterns: Vec<CTemplate>,
    pub default_values: Vec<(String, Expr)>,
}

/// A template resolved against the metamodel.
#[derive(Clone, Debug)]
pub(crate) enum CTemplate {
    Object(CObject),
    Collection(CCollection),
}

impl CTemplate {
    pub fn var(&self) -> &str {
        match self {
            CTemplate::Object(o) => &o.var,
            CTemplate::Collection(c) => &c.var,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CObject {
    pub var: String,
    pub class: Option<ClassifierId>,
    pub props: Vec<CProp>,
    pub guard: Option<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub(crate) struct CProp {
    pub feature: Option<FeatureId>,
    /// `opposite(C.f)`: the values are the objects of `C` whose `f` holds this object.
    pub opposite: bool,
    pub value: CValue,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub(crate) enum CValue {
    Expr(Expr),
    Template(CTemplate),
}

#[derive(Clone, Debug)]
pub(crate) struct CCollection {
    pub var: String,
    pub kind: CollectionKind,
    pub members: Vec<CMember>,
    pub rest: Option<String>,
    pub guard: Option<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub(crate) enum CMember {
    Var(String),
    Template(Box<CTemplate>),
}

impl Transformation {
    /// The parameters some relation enforces.
    pub(crate) fn targets(&self) -> Vec<String> {
        let mut out = Vec::new();
        for r in &self.relations {
            for d in &r.domains {
                if matches!(d.kind, DomainKind::Enforce | DomainKind::Replace)
                    && let Some(p) = d.param
                    && !out.contains(&self.params[p].name)
                {
                    out.push(self.params[p].name.clone());
                }
            }
        }
        out
    }

    pub(crate) fn relation(&self, name: &str) -> Option<usize> {
        self.relations.iter().position(|r| r.syntax.name.name == name)
    }

    pub(crate) fn query(&self, name: &str, arity: usize) -> Option<usize> {
        self.queries.iter().position(|q| q.syntax.name.name == name && q.params.len() == arity)
    }

    /// The relations an invocation of `r` may apply: `r` itself unless abstract, and the
    /// relations overriding it (transitively), the most specific first.
    pub(crate) fn implementations(&self, r: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stack = vec![r];
        while let Some(x) = stack.pop() {
            for (i, o) in self.relations.iter().enumerate() {
                if o.overrides == Some(x) && !out.contains(&i) {
                    stack.push(i);
                }
            }
            if !self.relations[x].syntax.is_abstract {
                out.push(x);
            }
        }
        out.reverse();
        out
    }
}

impl CheckHost for Transformation {
    fn operation(
        &self,
        mm: &Metamodel,
        source: Option<&Type>,
        name: &str,
        args: &[Type],
    ) -> Option<Result<Type, String>> {
        if source.is_none() {
            if let Some(q) = self.query(name, args.len()) {
                let q = &self.queries[q];
                for (i, (a, (_, p))) in args.iter().zip(&q.params).enumerate() {
                    if !a.conforms_to(p, mm) {
                        return Some(Err(format!(
                            "argument {} has type {}, but {} is expected",
                            i + 1,
                            a.display(mm),
                            p.display(mm)
                        )));
                    }
                }
                return Some(Ok(q.result.clone()));
            }
            if let Some(r) = self.relation(name) {
                let n = self.relations[r].roots.len();
                return Some(if n == args.len() {
                    Ok(Type::Boolean)
                } else {
                    Err(format!("the relation '{name}' takes {n} argument(s), but {} were given", args.len()))
                });
            }
            if self.queries.iter().any(|q| q.syntax.name.name == name) {
                return Some(Err(format!("the query '{name}' takes other arguments")));
            }
        }
        library::check_operation(mm, source, name, args)
    }
}

fn show(mm: &Metamodel, t: &Type) -> String {
    t.display(mm).to_string()
}

/// The type of the values a feature holds (its element type for many-valued ones).
fn feature_element(mm: &Metamodel, f: FeatureId) -> Type {
    let feature = mm.feature(f);
    feature.ty.map_or(Type::OclAny, |t| Type::of_classifier(mm, t))
}

/// Collects the variables of a template, resolving it: unnamed templates get fresh names.
struct TemplateResolver<'a> {
    mm: &'a Metamodel,
    aliases: &'a [PackageAlias],
    env: &'a Environment,
    vars: Vec<(String, Type)>,
    diags: Vec<Diagnostic>,
    fresh: usize,
}

impl TemplateResolver<'_> {
    fn resolve_type(&mut self, te: &TypeExpr) -> Type {
        let mut c = Checker::for_language(self.mm, self.env, &NoHost, self.aliases);
        let t = c.resolve_type(te);
        self.diags.extend(c.take_diagnostics());
        t
    }

    fn declare(&mut self, var: Option<&Ident>, ty: Type) -> String {
        match var {
            Some(v) => {
                match self.vars.iter().find(|(n, _)| *n == v.name) {
                    Some((_, existing))
                        if !ty.conforms_to(existing, self.mm) && !existing.conforms_to(&ty, self.mm) =>
                    {
                        let msg = format!(
                            "'{}' is declared as {} already, not {}",
                            v.name,
                            show(self.mm, existing),
                            show(self.mm, &ty)
                        );
                        self.diags.push(Diagnostic::error(v.span, msg));
                    }
                    Some(_) => {}
                    None => self.vars.push((v.name.clone(), ty)),
                }
                v.name.clone()
            }
            None => {
                self.fresh += 1;
                let name = format!("_{}", self.fresh);
                self.vars.push((name.clone(), ty));
                name
            }
        }
    }

    fn template(&mut self, t: &Template) -> CTemplate {
        match t {
            Template::Object(o) => CTemplate::Object(self.object(o)),
            Template::Collection(c) => CTemplate::Collection(self.collection(c)),
        }
    }

    fn object(&mut self, o: &ObjectTemplate) -> CObject {
        let mm = self.mm;
        let ty = self.resolve_type(&o.ty);
        let class = match &ty {
            Type::Classifier(c) if mm.is_class(*c) => Some(*c),
            Type::Error => None,
            other => {
                let msg = format!("object templates need a class, not {}", show(mm, other));
                self.diags.push(Diagnostic::error(o.ty.span, msg));
                None
            }
        };
        let var = self.declare(o.var.as_ref(), ty);
        let mut props = Vec::new();
        for p in &o.properties {
            let (feature, opposite) = match (&p.opposite, class) {
                (Some(owner), Some(c)) => {
                    let owner_ty = self.resolve_type(owner);
                    match owner_ty {
                        Type::Classifier(oc) if mm.is_class(oc) => match mm.lookup_feature(oc, &p.property.name) {
                            Some(f)
                                if mm.feature(f).ty.is_some_and(|t| mm.conforms_to(c, t) || mm.conforms_to(t, c)) =>
                            {
                                (Some(f), true)
                            }
                            Some(_) => {
                                let msg = format!("'{}' does not refer to {}", p.property.name, mm.classifier(c).name);
                                self.diags.push(Diagnostic::error(p.property.span, msg));
                                (None, true)
                            }
                            None => {
                                let msg =
                                    format!("unknown property '{}' of {}", p.property.name, mm.classifier(oc).name);
                                self.diags.push(Diagnostic::error(p.property.span, msg));
                                (None, true)
                            }
                        },
                        _ => (None, true),
                    }
                }
                (None, Some(c)) => match mm.lookup_feature(c, &p.property.name) {
                    Some(f) => (Some(f), false),
                    None => {
                        let msg = format!("unknown property '{}' of {}", p.property.name, mm.classifier(c).name);
                        self.diags.push(Diagnostic::error(p.property.span, msg));
                        (None, false)
                    }
                },
                _ => (None, false),
            };
            let value = match &p.value {
                TemplateValue::Expr(e) => CValue::Expr(e.clone()),
                TemplateValue::Template(t) => {
                    let nested = self.template(t);
                    if let (Some(f), CTemplate::Object(n)) = (feature, &nested)
                        && let Some(nc) = n.class
                    {
                        let expected = if opposite { Some(mm.feature(f).owner) } else { mm.feature(f).ty };
                        if let Some(e) = expected
                            && !mm.conforms_to(nc, e)
                        {
                            let msg = format!(
                                "a {} cannot be a value of '{}' (of type {})",
                                mm.classifier(nc).name,
                                p.property.name,
                                mm.classifier(e).name
                            );
                            self.diags.push(Diagnostic::error(t.ty().span, msg));
                        }
                    }
                    CValue::Template(nested)
                }
            };
            props.push(CProp { feature, opposite, value, span: p.span });
        }
        CObject { var, class, props, guard: o.guard.clone(), span: o.span }
    }

    fn collection(&mut self, c: &CollectionTemplate) -> CCollection {
        let ty = self.resolve_type(&c.ty);
        let (kind, elem) = match &ty {
            Type::Collection(k, e) => (*k, (**e).clone()),
            _ => (CollectionKind::Set, Type::Error),
        };
        let var = self.declare(c.var.as_ref(), ty);
        let members = c
            .members
            .iter()
            .map(|m| match m {
                Member::Var(v) => {
                    if v.name == "_" {
                        CMember::Var(self.declare(None, elem.clone()))
                    } else {
                        CMember::Var(self.declare(Some(v), elem.clone()))
                    }
                }
                Member::Template(t) => CMember::Template(Box::new(self.template(t))),
            })
            .collect();
        let rest = c.rest.as_ref().filter(|r| r.name != "_").map(|r| self.declare(Some(r), ty_of(kind, &elem)));
        CCollection { var, kind, members, rest, guard: c.guard.clone(), span: c.span }
    }
}

fn ty_of(kind: CollectionKind, elem: &Type) -> Type {
    Type::collection(kind, elem.clone())
}

/// A host that knows nothing (for resolving types).
struct NoHost;

impl CheckHost for NoHost {}

pub(crate) fn compile(mm: &Metamodel, env: &Environment, text: &str, lookup: PackageLookup) -> Compiled {
    let (doc, mut diags) = syntax::parse(text);
    let mut symbols = Vec::new();
    let mut uris = Vec::new();

    // Imports.
    let mut aliases: Vec<PackageAlias> = Vec::new();
    for i in &doc.imports {
        uris.push((i.uri.clone(), i.uri_span));
        let mut packages = lookup(&i.uri);
        if packages.is_empty() {
            diags.push(Diagnostic::error(
                i.uri_span,
                format!("cannot find the metamodel '{}': open it, or register its file in the package registry", i.uri),
            ));
        }
        for segment in &i.path {
            let inner: Vec<PackageId> = packages
                .iter()
                .flat_map(|p| {
                    if mm.package(*p).name == segment.name {
                        vec![*p]
                    } else {
                        mm.package(*p)
                            .subpackages
                            .iter()
                            .copied()
                            .filter(|s| mm.package(*s).name == segment.name)
                            .collect()
                    }
                })
                .collect();
            if inner.is_empty() && !packages.is_empty() {
                diags.push(Diagnostic::error(segment.span, format!("unknown package '{}'", segment.name)));
            }
            packages = inner;
        }
        let name = match (&i.alias, i.path.last(), packages.first()) {
            (Some(a), _, _) => a.name.clone(),
            (None, Some(p), _) => p.name.clone(),
            (None, None, Some(p)) => mm.package(*p).name.clone(),
            (None, None, None) => continue,
        };
        symbols.push(Symbol {
            name: name.clone(),
            kind: "import",
            detail: format!("import {name} : '{}'", i.uri),
            span: i.alias.as_ref().map_or(i.uri_span, |a| a.span),
            range: i.span,
        });
        aliases.push(PackageAlias { name, packages });
    }

    let Some(decl) = doc.transformations.first() else {
        if !text.trim().is_empty() {
            diags.push(Diagnostic::error(Span::new(0, 0), "missing 'transformation' declaration"));
        }
        return Compiled {
            language: Language::Relational,
            name: None,
            parameters: Vec::new(),
            diagnostics: diags,
            types: Vec::new(),
            scopes: Vec::new(),
            symbols,
            uris,
            library: false,
            units: Vec::new(),
            inner: Inner::Relational(Box::new(Transformation {
                name: String::new(),
                aliases,
                params: Vec::new(),
                keys: Vec::new(),
                relations: Vec::new(),
                queries: Vec::new(),
            })),
        };
    };
    for extra in &doc.transformations[1..] {
        diags.push(Diagnostic::error(extra.name.span, "only one transformation per document is supported"));
    }

    // Parameters.
    let mut params = Vec::new();
    let mut parameters = Vec::new();
    for p in &decl.params {
        let mut packages = Vec::new();
        for m in &p.metamodels {
            match aliases.iter().find(|a| a.name == m.name) {
                Some(a) => packages.extend(a.packages.iter().copied()),
                None => {
                    let found = mm.find_packages_qualified(&[m.name.as_str()]);
                    if found.is_empty() {
                        diags.push(Diagnostic::error(m.span, format!("unknown metamodel '{}' (import it)", m.name)));
                    }
                    packages.extend(found);
                }
            }
        }
        if params.iter().any(|x: &ParamInfo| x.name == p.name.name) {
            diags.push(Diagnostic::error(p.name.span, format!("duplicate parameter '{}'", p.name.name)));
            continue;
        }
        let model_type = p.metamodels.iter().map(|m| m.name.clone()).collect::<Vec<_>>().join(", ");
        symbols.push(Symbol {
            name: p.name.name.clone(),
            kind: "parameter",
            detail: format!("{} : {model_type}", p.name.name),
            span: p.name.span,
            range: p.span,
        });
        parameters.push(Parameter {
            name: p.name.name.clone(),
            direction: None,
            model_type,
            packages: packages.clone(),
            span: p.span,
        });
        params.push(ParamInfo { name: p.name.name.clone(), packages });
    }
    symbols.push(Symbol {
        name: decl.name.name.clone(),
        kind: "transformation",
        detail: format!(
            "transformation {}({})",
            decl.name.name,
            decl.params
                .iter()
                .map(|p| format!(
                    "{} : {}",
                    p.name.name,
                    p.metamodels.iter().map(|m| m.name.clone()).collect::<Vec<_>>().join(", ")
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        span: decl.name.span,
        range: decl.span,
    });

    let mut t = Transformation {
        name: decl.name.name.clone(),
        aliases,
        params,
        keys: Vec::new(),
        relations: Vec::new(),
        queries: Vec::new(),
    };

    // Keys.
    for k in &decl.keys {
        let mut c = Checker::for_language(mm, env, &NoHost, &t.aliases);
        let class = match c.resolve_type(&k.class) {
            Type::Classifier(cl) if mm.is_class(cl) => Some(cl),
            Type::Error => None,
            other => {
                c.report_error(k.class.span, format!("keys are defined for classes, not for {}", show(mm, &other)));
                None
            }
        };
        diags.extend(c.take_diagnostics());
        let Some(class) = class else { continue };
        let mut features = Vec::new();
        for p in &k.properties {
            match mm.lookup_feature(class, &p.name) {
                Some(f) => features.push(f),
                None => diags.push(Diagnostic::error(
                    p.span,
                    format!("unknown property '{}' of {}", p.name, mm.classifier(class).name),
                )),
            }
        }
        let mut opposites = Vec::new();
        for (owner, p) in &k.opposites {
            let mut c = Checker::for_language(mm, env, &NoHost, &t.aliases);
            if let Type::Classifier(oc) = c.resolve_type(owner)
                && let Some(f) = mm.lookup_feature(oc, &p.name)
            {
                opposites.push(f);
            } else {
                diags.push(Diagnostic::error(p.span, format!("unknown property '{}'", p.name)));
            }
            diags.extend(c.take_diagnostics());
        }
        t.keys.push(KeyInfo { class, features, opposites });
    }

    // Queries (signatures first: bodies may call each other).
    for q in &decl.queries {
        let mut c = Checker::for_language(mm, env, &NoHost, &t.aliases);
        let params = q.params.iter().map(|(n, ty)| (n.name.clone(), c.resolve_type(ty))).collect::<Vec<_>>();
        let result = c.resolve_type(&q.ty);
        diags.extend(c.take_diagnostics());
        if t.queries.iter().any(|o| o.syntax.name.name == q.name.name && o.params.len() == params.len()) {
            diags.push(Diagnostic::error(q.name.span, format!("duplicate query '{}'", q.name.name)));
        }
        if q.body.is_none() {
            diags.push(Diagnostic::error(q.name.span, format!("the query '{}' has no body", q.name.name)));
        }
        symbols.push(Symbol {
            name: q.name.name.clone(),
            kind: "query",
            detail: format!(
                "query {}({}) : {}",
                q.name.name,
                params.iter().map(|(n, ty)| format!("{n} : {}", show(mm, ty))).collect::<Vec<_>>().join(", "),
                show(mm, &result)
            ),
            span: q.name.span,
            range: q.span,
        });
        t.queries.push(QueryInfo { syntax: q.clone(), params, result });
    }

    // Relations: templates and variables.
    for r in &decl.relations {
        let mut resolver =
            TemplateResolver { mm, aliases: &t.aliases, env, vars: Vec::new(), diags: Vec::new(), fresh: 0 };
        let mut inits = Vec::new();
        for v in &r.variables {
            let ty = match &v.ty {
                Some(te) => resolver.resolve_type(te),
                None => Type::OclAny,
            };
            for n in &v.names {
                if resolver.vars.iter().any(|(x, _)| *x == n.name) {
                    resolver.diags.push(Diagnostic::error(n.span, format!("duplicate variable '{}'", n.name)));
                } else {
                    resolver.vars.push((n.name.clone(), ty.clone()));
                }
                if let Some(init) = &v.init {
                    inits.push((n.name.clone(), init.clone()));
                }
            }
        }
        let mut domains = Vec::new();
        let mut roots = Vec::new();
        for d in &r.domains {
            if d.kind == DomainKind::Primitive {
                for (n, te) in &d.primitives {
                    let ty = resolver.resolve_type(te);
                    let name = resolver.declare(Some(n), ty);
                    roots.push(name);
                }
                domains.push(DomainInfo {
                    kind: d.kind,
                    param: None,
                    patterns: Vec::new(),
                    default_values: Vec::new(),
                });
                continue;
            }
            let param = d.model.as_ref().and_then(|m| {
                let p = t.params.iter().position(|p| p.name == m.name);
                if p.is_none() {
                    resolver.diags.push(Diagnostic::error(m.span, format!("unknown model parameter '{}'", m.name)));
                }
                p
            });
            let patterns: Vec<CTemplate> = d.patterns.iter().map(|p| resolver.template(p)).collect();
            for (p, syn) in patterns.iter().zip(&d.patterns) {
                roots.push(p.var().to_string());
                if let (Some(pi), CTemplate::Object(o)) = (param, p)
                    && let Some(c) = o.class
                    && !t.params[pi].packages.is_empty()
                    && !in_packages(mm, c, &t.params[pi].packages)
                {
                    let msg =
                        format!("{} is not a class of the metamodel of '{}'", mm.classifier(c).name, t.params[pi].name);
                    resolver.diags.push(Diagnostic::error(syn.ty().span, msg));
                }
            }
            let default_values = d.default_values.iter().map(|(v, e)| (v.name.clone(), e.clone())).collect();
            domains.push(DomainInfo { kind: d.kind, param, patterns, default_values });
        }
        if !domains.iter().any(|d| d.param.is_some()) {
            resolver
                .diags
                .push(Diagnostic::error(r.name.span, format!("the relation '{}' has no domain", r.name.name)));
        }
        diags.extend(resolver.diags);
        if t.relations.iter().any(|o| o.syntax.name.name == r.name.name) {
            diags.push(Diagnostic::error(r.name.span, format!("duplicate relation '{}'", r.name.name)));
        }
        let detail = format!(
            "{}{}relation {}({})",
            if r.is_top { "top " } else { "" },
            if r.is_abstract { "abstract " } else { "" },
            r.name.name,
            roots
                .iter()
                .map(|n| {
                    let ty = resolver.vars.iter().find(|(x, _)| x == n).map_or(Type::Error, |(_, t)| t.clone());
                    format!("{n} : {}", show(mm, &ty))
                })
                .collect::<Vec<_>>()
                .join(", ")
        );
        symbols.push(Symbol { name: r.name.name.clone(), kind: "relation", detail, span: r.name.span, range: r.span });
        t.relations.push(RelationInfo {
            syntax: r.clone(),
            vars: resolver.vars,
            inits,
            domains,
            roots,
            overrides: None,
        });
    }
    for i in 0..t.relations.len() {
        let Some(o) = t.relations[i].syntax.overrides.clone() else { continue };
        match t.relation(&o.name) {
            Some(j) if j != i => {
                if t.relations[j].roots.len() != t.relations[i].roots.len() {
                    diags.push(Diagnostic::error(o.span, format!("'{}' has other domains than this relation", o.name)));
                }
                t.relations[i].overrides = Some(j);
            }
            _ => diags.push(Diagnostic::error(o.span, format!("unknown relation '{}'", o.name))),
        }
    }
    if !t.relations.iter().any(|r| r.syntax.is_top) && !t.relations.is_empty() {
        diags.push(Diagnostic::warning(
            decl.name.span,
            "the transformation has no top relation: running it does nothing",
        ));
    }

    // Expressions.
    let mut types = Vec::new();
    let mut scopes = Vec::new();
    for q in &t.queries {
        let Some(body) = &q.syntax.body else { continue };
        let mut c = Checker::for_language(mm, env, &t, &t.aliases);
        for (n, ty) in &q.params {
            c.bind(n, ty.clone());
        }
        c.expect(body, &q.result, &format!("the result of '{}'", q.syntax.name.name));
        diags.extend(c.take_diagnostics());
        types.extend(c.take_types());
        scopes.push(Scope { span: q.syntax.span, variables: q.params.clone(), self_type: None });
    }
    for r in &t.relations {
        let mut c = Checker::for_language(mm, env, &t, &t.aliases);
        for (n, ty) in &r.vars {
            c.bind(n, ty.clone());
        }
        for (name, init) in &r.inits {
            let ty = r.vars.iter().find(|(n, _)| n == name).map_or(Type::Error, |(_, t)| t.clone());
            c.expect(init, &ty, &format!("the value of '{name}'"));
        }
        for d in &r.domains {
            for p in &d.patterns {
                check_template(&mut c, mm, p);
            }
            for (v, e) in &d.default_values {
                match r.vars.iter().find(|(n, _)| n == v) {
                    Some((_, ty)) => {
                        c.expect(e, ty, &format!("the default value of '{v}'"));
                    }
                    None => {
                        c.check(e);
                        c.report_error(e.span, format!("unknown variable '{v}'"));
                    }
                }
            }
        }
        for preds in [&r.syntax.when, &r.syntax.where_].into_iter().flatten() {
            for p in preds {
                check_predicate(&mut c, mm, &t, p);
            }
        }
        diags.extend(c.take_diagnostics());
        types.extend(c.take_types());
        scopes.push(Scope { span: r.syntax.span, variables: r.vars.clone(), self_type: None });
    }
    diags.sort_by_key(|d| (d.span.start, d.span.end));
    diags.dedup_by(|a, b| a.span == b.span && a.message == b.message);

    Compiled {
        language: Language::Relational,
        name: Some(t.name.clone()),
        parameters,
        diagnostics: diags,
        types,
        scopes,
        symbols,
        uris,
        library: false,
        units: Vec::new(),
        inner: Inner::Relational(Box::new(t)),
    }
}

/// Whether a class belongs to one of the packages (or their subpackages).
fn in_packages(mm: &Metamodel, class: ClassifierId, packages: &[PackageId]) -> bool {
    let mut p = Some(mm.classifier(class).package);
    while let Some(x) = p {
        if packages.contains(&x) {
            return true;
        }
        p = mm.package(x).super_package;
    }
    false
}

fn check_template(c: &mut Checker<'_>, mm: &Metamodel, t: &CTemplate) {
    match t {
        CTemplate::Object(o) => {
            for p in &o.props {
                match &p.value {
                    CValue::Template(n) => check_template(c, mm, n),
                    CValue::Expr(e) => {
                        let ty = c.check(e);
                        let Some(f) = p.feature else { continue };
                        let feature = mm.feature(f);
                        let expected =
                            if p.opposite { Type::Classifier(feature.owner) } else { feature_element(mm, f) };
                        let many = p.opposite || feature.is_many();
                        let fits = ty.conforms_to(&expected, mm)
                            || (many && ty.element().is_some_and(|e| e.conforms_to(&expected, mm)))
                            || expected.conforms_to(&ty, mm);
                        if !fits && !ty.is_error() {
                            let msg = format!(
                                "a value of type {} does not fit '{}' (of type {})",
                                show(mm, &ty),
                                feature.name,
                                show(mm, &expected)
                            );
                            c.report_error(e.span, msg);
                        }
                    }
                }
            }
            if let Some(g) = &o.guard {
                c.expect(g, &Type::Boolean, "the condition of a template");
            }
        }
        CTemplate::Collection(col) => {
            for m in &col.members {
                if let CMember::Template(t) = m {
                    check_template(c, mm, t);
                }
            }
            if let Some(g) = &col.guard {
                c.expect(g, &Type::Boolean, "the condition of a template");
            }
        }
    }
}

fn check_predicate(c: &mut Checker<'_>, mm: &Metamodel, t: &Transformation, p: &Expr) {
    if let Some((name, args, _)) = relation_call(p)
        && let Some(r) = t.relation(&name.name)
    {
        let rel = &t.relations[r];
        if args.len() != rel.roots.len() {
            let msg = format!(
                "the relation '{}' takes {} argument(s), but {} were given",
                name.name,
                rel.roots.len(),
                args.len()
            );
            c.report_error(name.span, msg);
        }
        for (a, root) in args.iter().zip(&rel.roots) {
            let ty = c.check(a);
            let expected = rel.vars.iter().find(|(n, _)| n == root).map_or(Type::Error, |(_, t)| t.clone());
            if !ty.conforms_to(&expected, mm) && !expected.conforms_to(&ty, mm) {
                let msg = format!(
                    "a value of type {} does not fit '{root}' (of type {})",
                    show(mm, &ty),
                    show(mm, &expected)
                );
                c.report_error(a.span, msg);
            }
        }
        return;
    }
    c.expect(p, &Type::Boolean, "a predicate");
}

/// Free names of an expression (single names, not paths), for deciding whether it can be
/// evaluated with the variables bound so far.
pub(crate) fn names_in(e: &Expr, out: &mut Vec<String>) {
    e.walk(&mut |x| {
        if let oxy_ocl::syntax::ast::ExprKind::Name { path, .. } = &x.kind
            && let [single] = path.as_slice()
            && !out.contains(&single.name)
        {
            out.push(single.name.clone());
        }
    });
}
