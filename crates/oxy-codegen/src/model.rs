//! The code model: everything a target generates code from, independent of the target
//! language.
//!
//! A [`CodeModel`] is built from a metamodel and a compiled OCL environment by
//! [`crate::build`]. It is self-contained: classifiers, features and operations refer to each
//! other by index, types are [`Ty`]s, and OCL is already lowered to [`Expr`] trees in which
//! every name and call is resolved and typed (implicit `self`, implicit iterators and implicit
//! collects are explicit). A target only reads this module and [`crate::library`].

use std::fmt;

pub use oxy_ocl::syntax::ast::{BinOp, CollectionKind, UnOp};

macro_rules! index_type {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub usize);
    };
}

index_type!(/// Index into [`CodeModel::packages`].
    PackageId);
index_type!(/// Index into [`CodeModel::classes`].
    ClassId);
index_type!(/// Index into [`CodeModel::enums`].
    EnumId);
index_type!(/// Index into [`CodeModel::data_types`].
    DataTypeId);
index_type!(/// Index into [`CodeModel::features`].
    FeatureId);
index_type!(/// Index into [`CodeModel::operations`].
    OperationId);

/// The static type of a value.
#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    Boolean,
    /// Integers (Ecore's `EInt`, `ELong`, `EBigInteger`, …): 64 bit in the generated code.
    Integer,
    Real,
    String,
    /// `UnlimitedNatural`: a non-negative integer or `*` (represented as -1).
    UnlimitedNatural,
    Enum(EnumId),
    Class(ClassId),
    /// Any model object: `EObject`, `oclContainer()`.
    Object,
    /// A data type without an OCL primitive equivalent (dates, Java types): values are the
    /// strings of their serialized form.
    DataType(DataTypeId),
    Collection(CollectionKind, Box<Ty>),
    /// Tuple parts, sorted by name.
    Tuple(Vec<(String, Ty)>),
    /// A type used as a value: the argument of `oclIsKindOf`, the source of `allInstances`.
    Type(Box<Ty>),
    /// `OclAny`: any value.
    Any,
    /// The type of `null`.
    Void,
    /// The type of `invalid`, and of expressions that could not be translated.
    Invalid,
}

impl Ty {
    pub fn collection(kind: CollectionKind, element: Ty) -> Ty {
        Ty::Collection(kind, Box::new(element))
    }

    pub fn is_collection(&self) -> bool {
        matches!(self, Ty::Collection(..))
    }

    pub fn is_numeric(&self) -> bool {
        matches!(self, Ty::Integer | Ty::Real | Ty::UnlimitedNatural)
    }

    /// Whether values are model objects (of a class or any class).
    pub fn is_object(&self) -> bool {
        matches!(self, Ty::Class(_) | Ty::Object)
    }

    /// The element type of a collection type.
    pub fn element(&self) -> Option<&Ty> {
        match self {
            Ty::Collection(_, e) => Some(e),
            _ => None,
        }
    }

    pub fn collection_kind(&self) -> Option<CollectionKind> {
        match self {
            Ty::Collection(k, _) => Some(*k),
            _ => None,
        }
    }
}

/// The type and multiplicity of a feature, parameter or operation result.
#[derive(Clone, Debug, PartialEq)]
pub struct Typed {
    /// The type of one value (never a collection type for metamodel elements).
    pub ty: Ty,
    pub lower: i64,
    /// -1 for unbounded.
    pub upper: i64,
    /// More than one value: the value is a collection of `ty`.
    pub many: bool,
    pub ordered: bool,
    pub unique: bool,
    /// A single value that may be absent (`null`). Collections are never null.
    ///
    /// Values of object types (classes, `String`, `EBigInteger`, data types) are nullable
    /// when their lower bound is 0. Values of Java primitive types (`EInt`, `EBoolean`,
    /// `EDouble`, …) and enumerations always exist (attributes have a default).
    /// Single-valued references are always nullable: they are unset until linked, whatever
    /// the lower bound (which validation checks).
    pub nullable: bool,
}

impl Typed {
    /// The collection kind of a many-valued element.
    pub fn kind(&self) -> CollectionKind {
        CollectionKind::from_flags(self.ordered, self.unique)
    }

    /// The OCL type of the whole value: `ty`, or a collection of it.
    pub fn value_ty(&self) -> Ty {
        if self.many { Ty::collection(self.kind(), self.ty.clone()) } else { self.ty.clone() }
    }

    /// A single, required value of type `ty` (or a collection, for a collection type), as
    /// for the types of OCL definitions and variables.
    pub fn of_ty(ty: &Ty) -> Typed {
        match ty {
            Ty::Collection(kind, element) => Typed {
                ty: (**element).clone(),
                lower: 0,
                upper: -1,
                many: true,
                ordered: kind.is_ordered(),
                unique: kind.is_unique(),
                nullable: false,
            },
            other => Typed {
                ty: other.clone(),
                lower: 1,
                upper: 1,
                many: false,
                ordered: true,
                unique: true,
                nullable: other.is_object() || matches!(other, Ty::Any | Ty::Void),
            },
        }
    }
}

/// A literal value: the default value of an attribute.
#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    Boolean(bool),
    Integer(i64),
    Real(f64),
    String(String),
    /// A literal of an enumeration, by index.
    Enum(EnumId, usize),
}

#[derive(Clone, Debug)]
pub struct CodeModel {
    /// A name for the generated module, package or crate (lower case, from the first
    /// generated package unless given in the options).
    pub name: String,
    /// What the model was generated from, for headers (file names of the metamodels).
    pub source: String,
    pub packages: Vec<Package>,
    pub classes: Vec<Class>,
    pub enums: Vec<Enum>,
    pub data_types: Vec<DataType>,
    pub features: Vec<Feature>,
    pub operations: Vec<Operation>,
}

#[derive(Clone, Debug)]
pub struct Package {
    pub name: String,
    /// Names from the root package down to this one.
    pub path: Vec<String>,
    pub ns_uri: String,
    pub ns_prefix: String,
    pub super_package: Option<PackageId>,
    pub classes: Vec<ClassId>,
    pub enums: Vec<EnumId>,
    pub data_types: Vec<DataTypeId>,
    pub doc: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Class {
    pub name: String,
    pub package: PackageId,
    pub is_abstract: bool,
    pub is_interface: bool,
    pub doc: Option<String>,
    /// Direct supertypes.
    pub supers: Vec<ClassId>,
    /// All supertypes, nearest first.
    pub all_supers: Vec<ClassId>,
    /// Features declared by this class.
    pub features: Vec<FeatureId>,
    /// All features including inherited ones, inherited first (EMF's order).
    pub all_features: Vec<FeatureId>,
    /// Operations declared by this class.
    pub operations: Vec<OperationId>,
    /// Invariants whose context is this class (they apply to subclasses too).
    pub invariants: Vec<Constraint>,
    /// `def:` attributes of Complete OCL documents for this class.
    pub def_attributes: Vec<DefAttribute>,
    /// `def:` operations of Complete OCL documents for this class.
    pub def_operations: Vec<DefOperation>,
}

impl Class {
    /// Whether objects of exactly this class can exist.
    pub fn is_concrete(&self) -> bool {
        !self.is_abstract && !self.is_interface
    }
}

#[derive(Clone, Debug)]
pub struct Enum {
    pub name: String,
    pub package: PackageId,
    pub literals: Vec<EnumLiteral>,
    pub doc: Option<String>,
}

#[derive(Clone, Debug)]
pub struct EnumLiteral {
    pub name: String,
    pub value: i64,
    /// The serialized form (XMI uses it).
    pub literal: String,
}

/// A data type whose values are kept as strings.
#[derive(Clone, Debug)]
pub struct DataType {
    pub name: String,
    /// `None` for Ecore's own data types (`EDate`, `EJavaObject`, …).
    pub package: Option<PackageId>,
    pub instance_class_name: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Feature {
    pub name: String,
    pub owner: ClassId,
    pub typed: Typed,
    pub kind: FeatureKind,
    pub changeable: bool,
    pub derived: bool,
    pub transient: bool,
    pub volatile: bool,
    /// The default value of an attribute (its default literal, else the default of its
    /// type when it is not nullable).
    pub default: Option<Literal>,
    /// The OCL derivation: the value is computed, never stored.
    pub derivation: Option<Body>,
    /// The OCL initial value: used while the feature is unset.
    pub initial: Option<Body>,
    pub doc: Option<String>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FeatureKind {
    Attribute {
        /// An `iD` attribute: XMI documents may refer to objects by its value.
        id: bool,
    },
    Reference {
        /// The values are owned (contained) by the object.
        containment: bool,
        /// The opposite is a containment: the value is the object's container. Such
        /// references are not stored separately and not serialized.
        container: bool,
        opposite: Option<FeatureId>,
    },
}

impl Feature {
    pub fn is_reference(&self) -> bool {
        matches!(self.kind, FeatureKind::Reference { .. })
    }

    pub fn is_containment(&self) -> bool {
        matches!(self.kind, FeatureKind::Reference { containment: true, .. })
    }

    pub fn is_container(&self) -> bool {
        matches!(self.kind, FeatureKind::Reference { container: true, .. })
    }

    pub fn opposite(&self) -> Option<FeatureId> {
        match self.kind {
            FeatureKind::Reference { opposite, .. } => opposite,
            FeatureKind::Attribute { .. } => None,
        }
    }

    /// Whether objects store a value for this feature. Features with an OCL derivation are
    /// computed, container references are the object's container, and other volatile
    /// features have no value (EMF expects hand-written code for them).
    pub fn is_stored(&self) -> bool {
        self.derivation.is_none() && !self.is_container() && !self.volatile
    }

    /// Whether the feature is written to XMI documents.
    pub fn is_serialized(&self) -> bool {
        self.is_stored() && !self.transient && !self.derived
    }
}

#[derive(Clone, Debug)]
pub struct Parameter {
    pub name: String,
    pub typed: Typed,
}

#[derive(Clone, Debug)]
pub struct Operation {
    pub name: String,
    pub owner: ClassId,
    pub params: Vec<Parameter>,
    /// `None` for operations without a result.
    pub result: Option<Typed>,
    /// The OCL body; operations without one cannot be computed by the generated code.
    pub body: Option<Body>,
    pub preconditions: Vec<Constraint>,
    pub postconditions: Vec<Constraint>,
    /// Operations of supertypes with the same name and number of parameters: this one
    /// overrides them, and their pre- and postconditions apply to it too.
    pub overrides: Vec<OperationId>,
    pub doc: Option<String>,
}

/// An OCL expression with its origin.
#[derive(Clone, Debug)]
pub struct Body {
    pub expr: Expr,
    /// The OCL text, for documentation comments.
    pub text: String,
    /// Where it is defined, e.g. `library.ocl:16` or `library.ecore`.
    pub origin: String,
}

/// An invariant, precondition or postcondition.
#[derive(Clone, Debug)]
pub struct Constraint {
    /// The constraint's name; unnamed constraints are numbered (`inv1`, `post2`, …).
    pub name: String,
    pub body: Body,
    /// The message to report when the constraint is violated (a String expression).
    pub message: Option<Expr>,
}

/// A `def:` attribute: a computed property.
#[derive(Clone, Debug)]
pub struct DefAttribute {
    pub name: String,
    pub typed: Typed,
    pub body: Body,
}

/// A `def:` operation: a helper operation with an OCL body.
#[derive(Clone, Debug)]
pub struct DefOperation {
    pub name: String,
    pub params: Vec<Parameter>,
    pub typed: Typed,
    pub body: Body,
}

// ------------------------------------------------------------------ expressions

/// A lowered OCL expression: resolved and typed.
#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    /// The static OCL type.
    pub ty: Ty,
}

/// A variable of a lowered expression. Names are unique within a constraint or body (OCL
/// names, renamed where they shadow another variable, and `_1`, `_2`, … for implicit
/// iterators); `self` and `result` are not variables but [`ExprKind::SelfRef`] and
/// [`ExprKind::Result`].
#[derive(Clone, Debug, PartialEq)]
pub struct Var {
    pub name: String,
    pub ty: Ty,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Boolean(bool),
    Integer(i64),
    Real(f64),
    String(String),
    /// `*`.
    Unlimited,
    Null,
    /// `invalid`, or an expression that could not be translated; evaluating it fails with
    /// the reason.
    Invalid(String),
    EnumLiteral(EnumId, usize),
    /// A type used as a value (see [`Ty::Type`]).
    Type(Ty),
    SelfRef,
    /// The result of the operation, in a postcondition.
    Result,
    /// A parameter of the operation, by name.
    Parameter(String),
    Var(String),
    /// `source.feature`; `source` is a single object (implicit collects are
    /// [`IteratorKind::Collect`]s). With `safe` (`?.`), a null source yields null instead of
    /// invalid.
    Feature {
        source: Box<Expr>,
        feature: FeatureId,
        safe: bool,
    },
    /// `source.name` for a `def:` attribute of `class` (or of a supertype of it).
    DefAttribute {
        source: Box<Expr>,
        class: ClassId,
        name: String,
        safe: bool,
    },
    TuplePart {
        source: Box<Expr>,
        name: String,
    },
    /// A call of an operation of the metamodel. It dispatches on the class of the object:
    /// an operation of a subclass with the same name and parameters overrides it.
    Call {
        source: Box<Expr>,
        operation: OperationId,
        args: Vec<Expr>,
        safe: bool,
    },
    /// A call of a `def:` operation declared on `class`.
    DefCall {
        source: Box<Expr>,
        class: ClassId,
        name: String,
        args: Vec<Expr>,
        safe: bool,
    },
    /// A library operation (see [`crate::library`]): `name` is its canonical name, `source`
    /// its receiver (a collection for collection operations, see [`Expr::library`]).
    Library {
        name: String,
        source: Box<Expr>,
        args: Vec<Expr>,
        safe: bool,
    },
    /// `source->name(vars | body)`. `source` is a collection; `vars` has one variable, or
    /// several for `forAll` and `exists` (all combinations).
    Iterator {
        kind: IteratorKind,
        source: Box<Expr>,
        vars: Vec<Var>,
        body: Box<Expr>,
    },
    /// `source->iterate(var; acc = init | body)`.
    Iterate {
        source: Box<Expr>,
        var: Var,
        acc: Var,
        init: Box<Expr>,
        body: Box<Expr>,
    },
    /// `T.allInstances()` for a class or enumeration type.
    AllInstances(Ty),
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Unary {
        op: UnOp,
        operand: Box<Expr>,
    },
    If {
        cond: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Box<Expr>,
    },
    /// `let var = init in body` (one variable; multi-variable lets are nested).
    Let {
        var: Var,
        init: Box<Expr>,
        body: Box<Expr>,
    },
    CollectionLiteral {
        kind: CollectionKind,
        items: Vec<CollectionItem>,
    },
    TupleLiteral(Vec<(String, Expr)>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum CollectionItem {
    Single(Expr),
    /// `first..last` (Integers).
    Range(Expr, Expr),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum IteratorKind {
    Select,
    Reject,
    /// Collects the body values; collections are flattened.
    Collect,
    CollectNested,
    ForAll,
    Exists,
    /// The first element for which the body holds, or null.
    Any,
    One,
    IsUnique,
    SortedBy,
    /// The transitive closure of the body, from the source's elements (excluded unless
    /// reached again).
    Closure,
}

impl IteratorKind {
    pub fn from_name(name: &str) -> Option<IteratorKind> {
        Some(match name {
            "select" => IteratorKind::Select,
            "reject" => IteratorKind::Reject,
            "collect" => IteratorKind::Collect,
            "collectNested" => IteratorKind::CollectNested,
            "forAll" => IteratorKind::ForAll,
            "exists" => IteratorKind::Exists,
            "any" => IteratorKind::Any,
            "one" => IteratorKind::One,
            "isUnique" => IteratorKind::IsUnique,
            "sortedBy" => IteratorKind::SortedBy,
            "closure" => IteratorKind::Closure,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            IteratorKind::Select => "select",
            IteratorKind::Reject => "reject",
            IteratorKind::Collect => "collect",
            IteratorKind::CollectNested => "collectNested",
            IteratorKind::ForAll => "forAll",
            IteratorKind::Exists => "exists",
            IteratorKind::Any => "any",
            IteratorKind::One => "one",
            IteratorKind::IsUnique => "isUnique",
            IteratorKind::SortedBy => "sortedBy",
            IteratorKind::Closure => "closure",
        }
    }
}

impl Expr {
    pub fn new(kind: ExprKind, ty: Ty) -> Expr {
        Expr { kind, ty }
    }

    pub fn invalid(reason: impl Into<String>) -> Expr {
        Expr::new(ExprKind::Invalid(reason.into()), Ty::Invalid)
    }

    /// A library operation call.
    pub fn library(name: &str, source: Expr, args: Vec<Expr>, ty: Ty) -> Expr {
        Expr::new(ExprKind::Library { name: name.to_string(), source: Box::new(source), args, safe: false }, ty)
    }

    /// Visits this expression and all sub-expressions, depth first.
    pub fn walk(&self, f: &mut dyn FnMut(&Expr)) {
        f(self);
        match &self.kind {
            ExprKind::Feature { source, .. }
            | ExprKind::DefAttribute { source, .. }
            | ExprKind::TuplePart { source, .. } => source.walk(f),
            ExprKind::Call { source, args, .. }
            | ExprKind::DefCall { source, args, .. }
            | ExprKind::Library { source, args, .. } => {
                source.walk(f);
                for a in args {
                    a.walk(f);
                }
            }
            ExprKind::Iterator { source, body, .. } => {
                source.walk(f);
                body.walk(f);
            }
            ExprKind::Iterate { source, init, body, .. } => {
                source.walk(f);
                init.walk(f);
                body.walk(f);
            }
            ExprKind::Binary { lhs, rhs, .. } => {
                lhs.walk(f);
                rhs.walk(f);
            }
            ExprKind::Unary { operand, .. } => operand.walk(f),
            ExprKind::If { cond, then_branch, else_branch } => {
                cond.walk(f);
                then_branch.walk(f);
                else_branch.walk(f);
            }
            ExprKind::Let { init, body, .. } => {
                init.walk(f);
                body.walk(f);
            }
            ExprKind::CollectionLiteral { items, .. } => {
                for item in items {
                    match item {
                        CollectionItem::Single(e) => e.walk(f),
                        CollectionItem::Range(a, b) => {
                            a.walk(f);
                            b.walk(f);
                        }
                    }
                }
            }
            ExprKind::TupleLiteral(parts) => {
                for (_, e) in parts {
                    e.walk(f);
                }
            }
            _ => {}
        }
    }
}

// ------------------------------------------------------------------ queries

impl CodeModel {
    pub fn package(&self, id: PackageId) -> &Package {
        &self.packages[id.0]
    }

    pub fn class(&self, id: ClassId) -> &Class {
        &self.classes[id.0]
    }

    pub fn enumeration(&self, id: EnumId) -> &Enum {
        &self.enums[id.0]
    }

    pub fn data_type(&self, id: DataTypeId) -> &DataType {
        &self.data_types[id.0]
    }

    pub fn feature(&self, id: FeatureId) -> &Feature {
        &self.features[id.0]
    }

    pub fn operation(&self, id: OperationId) -> &Operation {
        &self.operations[id.0]
    }

    pub fn class_ids(&self) -> impl Iterator<Item = ClassId> + '_ {
        (0..self.classes.len()).map(ClassId)
    }

    pub fn feature_ids(&self) -> impl Iterator<Item = FeatureId> + '_ {
        (0..self.features.len()).map(FeatureId)
    }

    /// Whether `sub` is `sup` or one of its subclasses.
    pub fn conforms(&self, sub: ClassId, sup: ClassId) -> bool {
        sub == sup || self.class(sub).all_supers.contains(&sup)
    }

    /// The concrete classes conforming to `class` (itself included, if concrete).
    pub fn concrete_subclasses(&self, class: ClassId) -> Vec<ClassId> {
        self.class_ids().filter(|c| self.class(*c).is_concrete() && self.conforms(*c, class)).collect()
    }

    /// All features of a class that objects store a value for, in EMF's order.
    pub fn stored_features(&self, class: ClassId) -> Vec<FeatureId> {
        self.class(class).all_features.iter().copied().filter(|f| self.feature(*f).is_stored()).collect()
    }

    /// The operations of a class including inherited ones that it does not override, its own
    /// first (as visible on objects of the class).
    pub fn all_operations(&self, class: ClassId) -> Vec<OperationId> {
        let mut out: Vec<OperationId> = Vec::new();
        for c in std::iter::once(class).chain(self.class(class).all_supers.iter().copied()) {
            for &o in &self.class(c).operations {
                let op = self.operation(o);
                let overridden = out.iter().any(|x| {
                    let x = self.operation(*x);
                    x.name == op.name && x.params.len() == op.params.len()
                });
                if !overridden {
                    out.push(o);
                }
            }
        }
        out
    }

    /// The operation whose body computes `operation` for objects of class `class`: the most
    /// specific one with a body among `operation` and the operations overriding it in
    /// `class` and its supertypes. `None` if none has a body.
    pub fn implementation(&self, class: ClassId, operation: OperationId) -> Option<OperationId> {
        let op = self.operation(operation);
        std::iter::once(class)
            .chain(self.class(class).all_supers.iter().copied())
            .flat_map(|c| self.class(c).operations.iter().copied())
            .filter(|o| {
                let x = self.operation(*o);
                x.name == op.name && x.params.len() == op.params.len()
            })
            .find(|o| self.operation(*o).body.is_some())
    }

    /// The operations whose pre- and postconditions apply to a call of `operation` on an
    /// object of class `class`: the operation as overridden in `class`, and every operation
    /// it overrides.
    pub fn contract_operations(&self, class: ClassId, operation: OperationId) -> Vec<OperationId> {
        let op = self.operation(operation);
        std::iter::once(class)
            .chain(self.class(class).all_supers.iter().copied())
            .flat_map(|c| self.class(c).operations.iter().copied())
            .filter(|o| {
                let x = self.operation(*o);
                x.name == op.name && x.params.len() == op.params.len()
            })
            .collect()
    }

    /// The `def:` attribute `name` visible on `class` (declared on it or a supertype).
    pub fn def_attribute(&self, class: ClassId, name: &str) -> Option<(ClassId, &DefAttribute)> {
        std::iter::once(class)
            .chain(self.class(class).all_supers.iter().copied())
            .find_map(|c| self.class(c).def_attributes.iter().find(|d| d.name == name).map(|d| (c, d)))
    }

    /// The `def:` operation `name` with `arity` parameters visible on `class`.
    pub fn def_operation(&self, class: ClassId, name: &str, arity: usize) -> Option<(ClassId, &DefOperation)> {
        std::iter::once(class).chain(self.class(class).all_supers.iter().copied()).find_map(|c| {
            self.class(c).def_operations.iter().find(|d| d.name == name && d.params.len() == arity).map(|d| (c, d))
        })
    }

    /// The containment features that may hold objects of `class`.
    pub fn containers_of(&self, class: ClassId) -> Vec<FeatureId> {
        self.feature_ids()
            .filter(|f| {
                let feature = self.feature(*f);
                feature.is_containment() && matches!(feature.typed.ty, Ty::Class(t) if self.conforms(class, t))
            })
            .collect()
    }

    /// Classes that no containment feature can hold: candidates for model roots.
    pub fn root_classes(&self) -> Vec<ClassId> {
        self.class_ids().filter(|c| self.class(*c).is_concrete() && self.containers_of(*c).is_empty()).collect()
    }

    /// The qualified OCL name of a type, e.g. `Set(library::Book)`.
    pub fn type_name(&self, ty: &Ty) -> String {
        match ty {
            Ty::Boolean => "Boolean".into(),
            Ty::Integer => "Integer".into(),
            Ty::Real => "Real".into(),
            Ty::String => "String".into(),
            Ty::UnlimitedNatural => "UnlimitedNatural".into(),
            Ty::Enum(e) => self.enumeration(*e).name.clone(),
            Ty::Class(c) => self.class(*c).name.clone(),
            Ty::Object => "OclAny".into(),
            Ty::DataType(d) => self.data_type(*d).name.clone(),
            Ty::Collection(k, e) => format!("{}({})", k.name(), self.type_name(e)),
            Ty::Tuple(parts) => format!(
                "Tuple({})",
                parts.iter().map(|(n, t)| format!("{n} : {}", self.type_name(t))).collect::<Vec<_>>().join(", ")
            ),
            Ty::Type(t) => self.type_name(t),
            Ty::Any => "OclAny".into(),
            Ty::Void => "OclVoid".into(),
            Ty::Invalid => "OclInvalid".into(),
        }
    }
}

impl fmt::Display for IteratorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
