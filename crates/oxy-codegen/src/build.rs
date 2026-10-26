//! Building a [`CodeModel`] from a metamodel and a compiled OCL environment.
//!
//! The packages to generate are the selected ones, their subpackages, and every package
//! they depend on (supertypes, types of features and operations), so the generated code is
//! complete on its own. The OCL comes from the environment: invariants, derivations,
//! initial values, operation bodies, pre- and postconditions and `def:`s of the generated
//! classes, whatever source (Ecore annotations, OCLinEcore or Complete OCL documents) they
//! were defined in.

use std::collections::HashMap;
use std::rc::Rc;

use oxy_ecore as ecore;
use oxy_ecore::{ClassifierKind, DataTypeKind, Metamodel};
use oxy_ocl::env::Body as OclBody;
use oxy_ocl::syntax::ast::ConstraintKind;
use oxy_ocl::{Environment, SourceKind};

use crate::lower::{Binding, Lowerer};
use crate::model::*;
use crate::{Problem, Severity};

const GENMODEL_SOURCE: &str = "http://www.eclipse.org/emf/2002/GenModel";

/// Maps metamodel elements to code model elements.
#[derive(Default)]
pub(crate) struct Index {
    pub packages: HashMap<ecore::PackageId, PackageId>,
    pub classes: HashMap<ecore::ClassifierId, ClassId>,
    pub enums: HashMap<ecore::ClassifierId, EnumId>,
    pub data_types: HashMap<ecore::ClassifierId, DataTypeId>,
    pub features: HashMap<ecore::FeatureId, FeatureId>,
    pub operations: HashMap<ecore::OperationId, OperationId>,
    pub eobject: Option<ecore::ClassifierId>,
}

impl Index {
    /// The type of one value of an Ecore classifier.
    pub fn ty(&self, mm: &Metamodel, c: ecore::ClassifierId) -> Ty {
        if let Some(id) = self.classes.get(&c) {
            return Ty::Class(*id);
        }
        if let Some(id) = self.enums.get(&c) {
            return Ty::Enum(*id);
        }
        if let Some(id) = self.data_types.get(&c) {
            return Ty::DataType(*id);
        }
        match &mm.classifier(c).kind {
            ClassifierKind::DataType(DataTypeKind::Boolean) => Ty::Boolean,
            ClassifierKind::DataType(DataTypeKind::Integer) => Ty::Integer,
            ClassifierKind::DataType(DataTypeKind::Real) => Ty::Real,
            // Other data types (of Ecore, e.g. EDate) are kept as strings.
            ClassifierKind::DataType(_) => Ty::String,
            ClassifierKind::Class(_) => Ty::Object,
            ClassifierKind::Enum(_) => Ty::Invalid,
        }
    }

    /// The code model type of an OCL type.
    pub fn ocl_ty(&self, mm: &Metamodel, t: &oxy_ocl::Type) -> Ty {
        use oxy_ocl::Type as T;
        match t {
            T::Boolean => Ty::Boolean,
            T::Integer => Ty::Integer,
            T::Real => Ty::Real,
            T::String => Ty::String,
            T::UnlimitedNatural => Ty::UnlimitedNatural,
            T::OclAny => Ty::Any,
            T::OclVoid => Ty::Void,
            T::OclInvalid | T::Error | T::Model(_) => Ty::Invalid,
            T::Classifier(c) => self.ty(mm, *c),
            T::Collection(k, e) => Ty::collection(*k, self.ocl_ty(mm, e)),
            T::Tuple(parts) => Ty::Tuple(parts.iter().map(|(n, t)| (n.clone(), self.ocl_ty(mm, t))).collect()),
            T::Meta(t) => Ty::Type(Box::new(self.ocl_ty(mm, t))),
        }
    }
}

/// Builds the code model for `selected` packages (and the packages they depend on).
pub fn build(
    mm: &Metamodel,
    env: &Environment,
    selected: &[ecore::PackageId],
    name: Option<&str>,
) -> (CodeModel, Vec<Problem>) {
    let mut problems = Vec::new();
    let packages = package_closure(mm, selected, &mut problems);
    let mut index = Index { eobject: Some(mm.eobject()), ..Index::default() };
    let mut model = CodeModel {
        name: name.map(str::to_string).unwrap_or_else(|| {
            selected.first().map(|p| mm.package(*p).name.to_lowercase()).unwrap_or_else(|| "model".into())
        }),
        source: sources(mm, &packages),
        packages: Vec::new(),
        classes: Vec::new(),
        enums: Vec::new(),
        data_types: Vec::new(),
        features: Vec::new(),
        operations: Vec::new(),
    };

    // Pass 1: packages and classifiers, so that types resolve.
    for &p in &packages {
        let pkg = mm.package(p);
        let id = PackageId(model.packages.len());
        index.packages.insert(p, id);
        let mut path = Vec::new();
        let mut cur = Some(p);
        while let Some(c) = cur {
            path.push(mm.package(c).name.clone());
            cur = mm.package(c).super_package;
        }
        path.reverse();
        model.packages.push(Package {
            name: pkg.name.clone(),
            path,
            ns_uri: pkg.ns_uri.clone(),
            ns_prefix: pkg.ns_prefix.clone(),
            super_package: None,
            classes: Vec::new(),
            enums: Vec::new(),
            data_types: Vec::new(),
            doc: documentation(&pkg.annotations),
        });
        for &c in &pkg.classifiers {
            let classifier = mm.classifier(c);
            let doc = documentation(&classifier.annotations);
            match &classifier.kind {
                ClassifierKind::Class(data) => {
                    let cid = ClassId(model.classes.len());
                    index.classes.insert(c, cid);
                    model.packages[id.0].classes.push(cid);
                    model.classes.push(Class {
                        name: classifier.name.clone(),
                        package: id,
                        is_abstract: data.is_abstract,
                        is_interface: data.is_interface,
                        doc,
                        supers: Vec::new(),
                        all_supers: Vec::new(),
                        features: Vec::new(),
                        all_features: Vec::new(),
                        operations: Vec::new(),
                        invariants: Vec::new(),
                        def_attributes: Vec::new(),
                        def_operations: Vec::new(),
                    });
                }
                ClassifierKind::Enum(literals) => {
                    let eid = EnumId(model.enums.len());
                    index.enums.insert(c, eid);
                    model.packages[id.0].enums.push(eid);
                    model.enums.push(Enum {
                        name: classifier.name.clone(),
                        package: id,
                        literals: literals
                            .iter()
                            .map(|l| EnumLiteral {
                                name: l.name.clone(),
                                value: l.value,
                                literal: if l.literal.is_empty() { l.name.clone() } else { l.literal.clone() },
                            })
                            .collect(),
                        doc,
                    });
                }
                ClassifierKind::DataType(kind) => {
                    if *kind == DataTypeKind::Other {
                        let did = DataTypeId(model.data_types.len());
                        index.data_types.insert(c, did);
                        model.packages[id.0].data_types.push(did);
                        model.data_types.push(DataType {
                            name: classifier.name.clone(),
                            package: Some(id),
                            instance_class_name: classifier.instance_class_name.clone(),
                        });
                    }
                }
            }
        }
    }
    for &p in &packages {
        let id = index.packages[&p];
        model.packages[id.0].super_package = mm.package(p).super_package.and_then(|s| index.packages.get(&s).copied());
    }

    // Pass 2: hierarchy, features and operations.
    let class_ids: Vec<(ecore::ClassifierId, ClassId)> = {
        let mut v: Vec<_> = index.classes.iter().map(|(k, v)| (*k, *v)).collect();
        v.sort_by_key(|(_, v)| *v);
        v
    };
    for &(c, cid) in &class_ids {
        let eobject = index.eobject;
        let supers = |ids: &[ecore::ClassifierId]| -> Vec<ClassId> {
            ids.iter().filter(|s| Some(**s) != eobject).filter_map(|s| index.classes.get(s).copied()).collect()
        };
        model.classes[cid.0].supers = supers(mm.super_types(c));
        model.classes[cid.0].all_supers = supers(mm.all_super_types(c));
        let data = mm.class_data(c).expect("indexed classes are classes");
        for &f in &data.features {
            let fid = FeatureId(model.features.len());
            index.features.insert(f, fid);
            model.classes[cid.0].features.push(fid);
            let feature = feature(mm, &index, f, cid, &model.enums, &mut problems);
            model.features.push(feature);
        }
        for &o in &data.operations {
            let oid = OperationId(model.operations.len());
            index.operations.insert(o, oid);
            model.classes[cid.0].operations.push(oid);
            model.operations.push(operation(mm, &index, o, cid, &mut problems));
        }
    }
    for &(c, cid) in &class_ids {
        model.classes[cid.0].all_features =
            mm.all_features(c).iter().filter_map(|f| index.features.get(f).copied()).collect();
    }
    // Opposites (their features exist now).
    for (&f, &fid) in &index.features {
        if let FeatureKind::Reference { opposite, .. } = &mut model.features[fid.0].kind {
            *opposite = mm.feature(f).opposite().and_then(|o| index.features.get(&o).copied());
        }
    }
    for &(_, cid) in &class_ids {
        for oid in model.classes[cid.0].operations.clone() {
            let op = &model.operations[oid.0];
            let overrides: Vec<OperationId> = model.classes[cid.0]
                .all_supers
                .iter()
                .flat_map(|s| model.classes[s.0].operations.iter().copied())
                .filter(|o| {
                    let x = &model.operations[o.0];
                    x.name == op.name && x.params.len() == op.params.len()
                })
                .collect();
            model.operations[oid.0].overrides = overrides;
        }
    }

    // Pass 3: OCL.
    let mut ocl = Ocl { mm, env, index: &index, problems: &mut problems };
    ocl.add_invariants(&mut model);
    ocl.add_features(&mut model);
    ocl.add_operations(&mut model);
    ocl.add_definitions(&mut model);
    (model, problems)
}

/// The selected packages, their subpackages and the packages they depend on (excluding
/// Ecore), in metamodel order.
fn package_closure(
    mm: &Metamodel,
    selected: &[ecore::PackageId],
    problems: &mut Vec<Problem>,
) -> Vec<ecore::PackageId> {
    let ecore_package = mm.ecore_package();
    let mut included: Vec<ecore::PackageId> = Vec::new();
    let mut queue: Vec<ecore::PackageId> = selected.to_vec();
    let mut warned: Vec<ecore::ClassifierId> = Vec::new();
    while let Some(p) = queue.pop() {
        if p == ecore_package || included.contains(&p) {
            continue;
        }
        included.push(p);
        let pkg = mm.package(p);
        queue.extend(pkg.subpackages.iter().copied());
        let mut referenced: Vec<ecore::ClassifierId> = Vec::new();
        for &c in &pkg.classifiers {
            let Some(data) = mm.class_data(c) else { continue };
            referenced.extend(data.super_types.iter().copied());
            referenced.extend(data.features.iter().filter_map(|f| mm.feature(*f).ty));
            for &o in &data.operations {
                let op = mm.operation(o);
                referenced.extend(op.ty);
                referenced.extend(op.params.iter().filter_map(|p| p.ty));
            }
        }
        for c in referenced {
            let owner = mm.classifier(c).package;
            if owner != ecore_package {
                queue.push(owner);
            } else if mm.is_class(c) && c != mm.eobject() && !warned.contains(&c) {
                warned.push(c);
                problems.push(Problem::warning(format!(
                    "the Ecore class '{}' is used as a type: its values are treated as untyped objects",
                    mm.classifier(c).name
                )));
            }
        }
    }
    included.sort();
    included
}

/// The file names of the resources the packages come from.
fn sources(mm: &Metamodel, packages: &[ecore::PackageId]) -> String {
    let mut names: Vec<String> = Vec::new();
    for p in packages {
        let uri = &mm.resources()[mm.package(*p).resource].uri;
        let name = uri.rsplit(['/', '\\']).next().unwrap_or(uri).to_string();
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names.join(", ")
}

fn documentation(annotations: &[ecore::Annotation]) -> Option<String> {
    annotations
        .iter()
        .filter(|a| a.source == GENMODEL_SOURCE)
        .find_map(|a| a.detail("documentation"))
        .map(str::to_string)
}

/// Whether single values of a type have a non-null default (Java primitive types and
/// enumerations), as in EMF.
fn has_primitive_default(mm: &Metamodel, ty: Option<ecore::ClassifierId>) -> bool {
    let Some(ty) = ty else { return false };
    mm.is_enum(ty)
        || matches!(
            mm.classifier(ty).instance_class_name.as_deref(),
            Some("boolean" | "int" | "long" | "short" | "byte" | "double" | "float" | "char")
        )
}

/// The type of a feature, parameter or operation result. Single-valued references
/// (`reference`) are always nullable; other single values when their lower bound is 0 and
/// their type has no primitive default.
fn typed(
    mm: &Metamodel,
    index: &Index,
    ty: Option<ecore::ClassifierId>,
    m: &ecore::Multiplicity,
    reference: bool,
) -> Typed {
    let many = m.is_many();
    let nullable = !many && (reference || (m.lower == 0 && !has_primitive_default(mm, ty)));
    Typed {
        // Without a (resolvable) type: any object, or a value kept as a string.
        ty: ty.map_or(if reference { Ty::Object } else { Ty::String }, |c| index.ty(mm, c)),
        lower: m.lower.max(0) as i64,
        upper: if m.upper < 0 { -1 } else { m.upper as i64 },
        many,
        ordered: m.ordered,
        unique: m.unique,
        nullable,
    }
}

/// The default value of a non-nullable type without a default literal.
fn type_default(ty: &Ty, enums: &[Enum]) -> Option<Literal> {
    match ty {
        Ty::Boolean => Some(Literal::Boolean(false)),
        Ty::Integer | Ty::UnlimitedNatural => Some(Literal::Integer(0)),
        Ty::Real => Some(Literal::Real(0.0)),
        Ty::String | Ty::DataType(_) => Some(Literal::String(String::new())),
        Ty::Enum(e) if !enums[e.0].literals.is_empty() => Some(Literal::Enum(*e, 0)),
        _ => None,
    }
}

fn feature(
    mm: &Metamodel,
    index: &Index,
    f: ecore::FeatureId,
    owner: ClassId,
    enums: &[Enum],
    problems: &mut Vec<Problem>,
) -> Feature {
    let feature = mm.feature(f);
    let kind = match feature.kind {
        ecore::FeatureKind::Attribute { is_id } => FeatureKind::Attribute { id: is_id },
        ecore::FeatureKind::Reference { containment, opposite } => FeatureKind::Reference {
            containment,
            container: opposite.is_some_and(|o| mm.feature(o).is_containment()),
            opposite: None,
        },
    };
    let typed = typed(mm, index, feature.ty, &feature.multiplicity, feature.is_reference());
    let mut default = None;
    if !feature.is_reference() && !typed.many {
        if let (Some(lit), Some(ty)) = (&feature.default_value_literal, feature.ty) {
            match ecore::parse_value(mm, ty, lit) {
                Ok(v) => default = literal(index, &v),
                Err(e) => {
                    problems.push(Problem::warning(format!("default value of '{}': {e}", mm.feature_qualified_name(f))))
                }
            }
        }
        if default.is_none() && !typed.nullable {
            default = type_default(&typed.ty, enums);
        }
    }
    Feature {
        name: feature.name.clone(),
        owner,
        typed,
        kind,
        changeable: feature.changeable,
        derived: feature.derived,
        transient: feature.transient,
        volatile: feature.volatile,
        default,
        derivation: None,
        initial: None,
        doc: documentation(&feature.annotations),
    }
}

fn literal(index: &Index, v: &ecore::ModelValue) -> Option<Literal> {
    Some(match v {
        ecore::ModelValue::Boolean(b) => Literal::Boolean(*b),
        ecore::ModelValue::Integer(i) => Literal::Integer(*i),
        ecore::ModelValue::Real(r) => Literal::Real(*r),
        ecore::ModelValue::String(s) => Literal::String(s.clone()),
        ecore::ModelValue::Enum(e, i) => Literal::Enum(*index.enums.get(e)?, *i),
        ecore::ModelValue::Object(_) => return None,
    })
}

fn operation(
    mm: &Metamodel,
    index: &Index,
    o: ecore::OperationId,
    owner: ClassId,
    _problems: &mut Vec<Problem>,
) -> Operation {
    let op = mm.operation(o);
    Operation {
        name: op.name.clone(),
        owner,
        params: op
            .params
            .iter()
            .map(|p| Parameter { name: p.name.clone(), typed: typed(mm, index, p.ty, &p.multiplicity, false) })
            .collect(),
        result: op.ty.map(|t| typed(mm, index, Some(t), &op.multiplicity, false)),
        body: None,
        preconditions: Vec::new(),
        postconditions: Vec::new(),
        overrides: Vec::new(),
        doc: documentation(&op.annotations),
    }
}

/// Lowers the OCL of the environment into the code model.
struct Ocl<'a> {
    mm: &'a Metamodel,
    env: &'a Environment,
    index: &'a Index,
    problems: &'a mut Vec<Problem>,
}

impl Ocl<'_> {
    /// Where a body is defined: `file:line` for documents, the annotation's element for
    /// `.ecore` files.
    fn origin(&self, body: &OclBody) -> String {
        let source = self.env.source(body.source);
        let file = source.name.rsplit(['/', '\\']).next().unwrap_or(&source.name).to_string();
        match source.kind {
            SourceKind::Document => format!("{file}:{}", source.position(body.expr.span.start).line + 1),
            _ => file,
        }
    }

    /// Lowers a body. Bodies whose source has errors at the body's location are replaced by
    /// an `invalid` expression (with a warning): the code would not mean anything.
    fn lower(&mut self, what: &str, body: &OclBody, bindings: &[(String, Binding)], self_ty: Ty) -> Body {
        let source = self.env.source(body.source);
        let text = source.snippet(body.expr.span).to_string();
        let origin = self.origin(body);
        let span = body.expr.span;
        let error = self.env.diagnostics().iter().find(|d| {
            d.source == body.source
                && d.diagnostic.is_error()
                && d.diagnostic.span.start >= span.start
                && d.diagnostic.span.end <= span.end
        });
        let expr = if let Some(e) = error {
            self.problems.push(Problem {
                severity: Severity::Warning,
                message: format!("{what} has errors ({}); it always evaluates to invalid", e.diagnostic.message),
                origin: Some(origin.clone()),
            });
            Expr::invalid(format!("{what} has errors: {}", e.diagnostic.message))
        } else {
            let mut lowerer = Lowerer::new(self.mm, self.env, self.index, body, what, &origin, self.problems);
            lowerer.bind_self(self_ty);
            for (name, b) in bindings {
                lowerer.bind(name, b.clone());
            }
            lowerer.lower(&body.expr)
        };
        Body { expr, text, origin }
    }

    fn lower_message(
        &mut self,
        what: &str,
        message: &Rc<oxy_ocl::syntax::ast::Expr>,
        body: &OclBody,
        bindings: &[(String, Binding)],
        self_ty: Ty,
    ) -> Expr {
        let message_body = OclBody { expr: message.clone(), ..body.clone() };
        self.lower(&format!("the message of {what}"), &message_body, bindings, self_ty).expr
    }

    fn add_invariants(&mut self, model: &mut CodeModel) {
        let env = self.env;
        for (n, inv) in env.invariants().iter().enumerate() {
            let Some(&class) = self.index.classes.get(&inv.context) else { continue };
            let name = inv.name.clone().unwrap_or_else(|| format!("inv{}", n + 1));
            let what = format!("invariant {}::{name}", model.class(class).name);
            let mut bindings = Vec::new();
            if let Some(alias) = &inv.self_name {
                bindings.push((alias.clone(), Binding::SelfRef));
            }
            let body = self.lower(&what, &inv.body, &bindings, Ty::Class(class));
            let message =
                inv.message.as_ref().map(|m| self.lower_message(&what, m, &inv.body, &bindings, Ty::Class(class)));
            model.classes[class.0].invariants.push(Constraint { name, body, message });
        }
    }

    fn add_features(&mut self, model: &mut CodeModel) {
        let env = self.env;
        let mut features: Vec<(ecore::FeatureId, FeatureId)> =
            self.index.features.iter().map(|(k, v)| (*k, *v)).collect();
        features.sort_by_key(|(_, v)| *v);
        for (f, fid) in features {
            let owner = model.features[fid.0].owner;
            let what = format!("{}::{}", model.class(owner).name, model.features[fid.0].name);
            if let Some(body) = env.derivation(f) {
                let lowered = self.lower(&format!("the derivation of {what}"), body, &[], Ty::Class(owner));
                model.features[fid.0].derivation = Some(lowered);
            }
            if let Some(body) = env.initializer(f) {
                let lowered = self.lower(&format!("the initial value of {what}"), body, &[], Ty::Class(owner));
                model.features[fid.0].initial = Some(lowered);
            }
        }
    }

    fn add_operations(&mut self, model: &mut CodeModel) {
        let env = self.env;
        let mut operations: Vec<(ecore::OperationId, OperationId)> =
            self.index.operations.iter().map(|(k, v)| (*k, *v)).collect();
        operations.sort_by_key(|(_, v)| *v);
        for (o, oid) in operations {
            let owner = model.operations[oid.0].owner;
            let what = format!("{}::{}()", model.class(owner).name, model.operations[oid.0].name);
            let params: Vec<(String, Binding)> = model.operations[oid.0]
                .params
                .iter()
                .map(|p| (p.name.clone(), Binding::Parameter(p.name.clone())))
                .collect();
            if let Some(body) = env.operation_body(o) {
                let lowered = self.lower(&format!("the body of {what}"), body, &params, Ty::Class(owner));
                model.operations[oid.0].body = Some(lowered);
            }
            let (mut pre_count, mut post_count) = (0, 0);
            for &i in env.contracts_of(o) {
                let oc = &env.operation_constraints()[i];
                let post = oc.kind == ConstraintKind::Post;
                let n = if post {
                    post_count += 1;
                    post_count
                } else {
                    pre_count += 1;
                    pre_count
                };
                let name = oc.name.clone().unwrap_or_else(|| format!("{}{n}", if post { "post" } else { "pre" }));
                let what = format!("{} {name} of {what}", if post { "postcondition" } else { "precondition" });
                let mut bindings = params.clone();
                if post {
                    bindings.push(("result".into(), Binding::Result));
                }
                let body = self.lower(&what, &oc.body, &bindings, Ty::Class(owner));
                let message =
                    oc.message.as_ref().map(|m| self.lower_message(&what, m, &oc.body, &bindings, Ty::Class(owner)));
                let constraint = Constraint { name, body, message };
                if post {
                    model.operations[oid.0].postconditions.push(constraint);
                } else {
                    model.operations[oid.0].preconditions.push(constraint);
                }
            }
        }
    }

    fn add_definitions(&mut self, model: &mut CodeModel) {
        let env = self.env;
        let mm = self.mm;
        let mut attributes: Vec<_> =
            env.def_attributes().filter(|d| self.index.classes.contains_key(&d.class)).collect();
        attributes.sort_by(|a, b| (a.class, &a.name).cmp(&(b.class, &b.name)));
        for d in attributes {
            let class = self.index.classes[&d.class];
            let what = format!("the definition of {}::{}", model.class(class).name, d.name);
            let body = self.lower(&what, &d.body, &[], Ty::Class(class));
            let typed = Typed::of_ty(&self.index.ocl_ty(mm, &d.ty));
            model.classes[class.0].def_attributes.push(DefAttribute { name: d.name.clone(), typed, body });
        }
        let mut operations: Vec<_> =
            env.def_operations().filter(|d| self.index.classes.contains_key(&d.class)).collect();
        operations.sort_by(|a, b| (a.class, &a.name, a.params.len()).cmp(&(b.class, &b.name, b.params.len())));
        for d in operations {
            let class = self.index.classes[&d.class];
            let what = format!("the definition of {}::{}()", model.class(class).name, d.name);
            let params: Vec<Parameter> = d
                .params
                .iter()
                .map(|(n, t)| Parameter { name: n.clone(), typed: Typed::of_ty(&self.index.ocl_ty(mm, t)) })
                .collect();
            let bindings: Vec<(String, Binding)> =
                params.iter().map(|p| (p.name.clone(), Binding::Parameter(p.name.clone()))).collect();
            let body = self.lower(&what, &d.body, &bindings, Ty::Class(class));
            let typed = Typed::of_ty(&self.index.ocl_ty(mm, &d.ty));
            model.classes[class.0].def_operations.push(DefOperation { name: d.name.clone(), params, typed, body });
        }
    }
}
