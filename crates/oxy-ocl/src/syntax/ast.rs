//! Abstract syntax of OCL expressions and Complete OCL documents.

use std::fmt;

use crate::span::Span;

/// Identifies an expression node within one parsed source (for side tables such as types).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ExprId(pub u32);

#[derive(Clone, Debug, PartialEq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

impl fmt::Display for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum CollectionKind {
    Collection,
    Set,
    OrderedSet,
    Bag,
    Sequence,
}

impl CollectionKind {
    pub fn from_name(name: &str) -> Option<CollectionKind> {
        Some(match name {
            "Collection" => CollectionKind::Collection,
            "Set" => CollectionKind::Set,
            "OrderedSet" => CollectionKind::OrderedSet,
            "Bag" => CollectionKind::Bag,
            "Sequence" => CollectionKind::Sequence,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            CollectionKind::Collection => "Collection",
            CollectionKind::Set => "Set",
            CollectionKind::OrderedSet => "OrderedSet",
            CollectionKind::Bag => "Bag",
            CollectionKind::Sequence => "Sequence",
        }
    }

    pub fn is_unique(self) -> bool {
        matches!(self, CollectionKind::Set | CollectionKind::OrderedSet)
    }

    pub fn is_ordered(self) -> bool {
        matches!(self, CollectionKind::Sequence | CollectionKind::OrderedSet)
    }

    /// The collection kind for given ordered/unique flags (as on Ecore typed elements).
    pub fn from_flags(ordered: bool, unique: bool) -> CollectionKind {
        match (ordered, unique) {
            (true, true) => CollectionKind::OrderedSet,
            (false, true) => CollectionKind::Set,
            (true, false) => CollectionKind::Sequence,
            (false, false) => CollectionKind::Bag,
        }
    }

    /// Kind of the result of `collect`/implicit collect: Bag for unordered, Sequence for ordered.
    pub fn collect_kind(self) -> CollectionKind {
        if self.is_ordered() { CollectionKind::Sequence } else { CollectionKind::Bag }
    }
}

/// A type as written in the source.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeExpr {
    pub kind: TypeExprKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TypeExprKind {
    /// `Integer`, `Book`, `library::Book`.
    Path(Vec<Ident>),
    Collection(CollectionKind, Box<TypeExpr>),
    Tuple(Vec<(Ident, TypeExpr)>),
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VarDecl {
    pub name: Ident,
    pub ty: Option<TypeExpr>,
    pub init: Option<Expr>,
    pub span: Span,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum BinOp {
    Implies,
    Xor,
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Implies => "implies",
            BinOp::Xor => "xor",
            BinOp::Or => "or",
            BinOp::And => "and",
            BinOp::Eq => "=",
            BinOp::Ne => "<>",
            BinOp::Lt => "<",
            BinOp::Gt => ">",
            BinOp::Le => "<=",
            BinOp::Ge => ">=",
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum UnOp {
    Not,
    Neg,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CollectionItem {
    Single(Expr),
    Range(Expr, Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub id: ExprId,
    pub span: Span,
    pub kind: ExprKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Bool(bool),
    Int(i64),
    Real(f64),
    Str(String),
    Null,
    Invalid,
    /// The unlimited natural `*`.
    Unlimited,
    SelfRef,
    /// A variable, implicit property, type, or `Enum::Literal`; `@pre` is recorded.
    Name {
        path: Vec<Ident>,
        at_pre: bool,
    },
    /// A type used as a value, e.g. the argument of `oclIsKindOf(Set(Integer))`.
    TypeLit(TypeExpr),
    /// `source.name` (or `source?.name`).
    Property {
        source: Box<Expr>,
        name: Ident,
        safe: bool,
        at_pre: bool,
    },
    /// `source.name(args)`, `source->name(args)`, or `name(args)` on implicit `self`.
    Call {
        source: Option<Box<Expr>>,
        name: Ident,
        args: Vec<Expr>,
        arrow: bool,
        safe: bool,
        at_pre: bool,
    },
    /// `source->name(v1, v2 | body)`; `iterators` is empty for an implicit iterator.
    Iterator {
        source: Box<Expr>,
        name: Ident,
        iterators: Vec<VarDecl>,
        body: Box<Expr>,
        safe: bool,
    },
    /// `source->iterate(v; acc : T = init | body)`.
    Iterate {
        source: Box<Expr>,
        iterators: Vec<VarDecl>,
        accumulator: Box<VarDecl>,
        body: Box<Expr>,
        safe: bool,
    },
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
    Let {
        vars: Vec<VarDecl>,
        body: Box<Expr>,
    },
    CollectionLit {
        kind: CollectionKind,
        element_type: Option<TypeExpr>,
        items: Vec<CollectionItem>,
    },
    TupleLit(Vec<VarDecl>),
    /// Placeholder produced by error recovery.
    Error,
}

/// Iterator names recognized after `->`.
pub const ITERATORS: &[&str] = &[
    "select",
    "reject",
    "collect",
    "collectNested",
    "forAll",
    "exists",
    "any",
    "one",
    "isUnique",
    "sortedBy",
    "closure",
];

pub fn is_iterator_name(name: &str) -> bool {
    ITERATORS.contains(&name)
}

impl Expr {
    /// Visits this expression and all sub-expressions, depth first.
    pub fn walk(&self, f: &mut dyn FnMut(&Expr)) {
        f(self);
        let var = |v: &VarDecl, f: &mut dyn FnMut(&Expr)| {
            if let Some(i) = &v.init {
                i.walk(f);
            }
        };
        match &self.kind {
            ExprKind::Property { source, .. } => source.walk(f),
            ExprKind::Call { source, args, .. } => {
                if let Some(s) = source {
                    s.walk(f);
                }
                for a in args {
                    a.walk(f);
                }
            }
            ExprKind::Iterator { source, iterators, body, .. } => {
                source.walk(f);
                for v in iterators {
                    var(v, f);
                }
                body.walk(f);
            }
            ExprKind::Iterate { source, iterators, accumulator, body, .. } => {
                source.walk(f);
                for v in iterators {
                    var(v, f);
                }
                var(accumulator, f);
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
            ExprKind::Let { vars, body } => {
                for v in vars {
                    var(v, f);
                }
                body.walk(f);
            }
            ExprKind::CollectionLit { items, .. } => {
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
            ExprKind::TupleLit(parts) => {
                for p in parts {
                    var(p, f);
                }
            }
            _ => {}
        }
    }
}

// ------------------------------------------------------------------ documents

/// A Complete OCL document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document {
    pub imports: Vec<Import>,
    /// Package declarations; contexts outside any `package` block are collected in a
    /// declaration with an empty path.
    pub packages: Vec<PackageDecl>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Import {
    pub alias: Option<Ident>,
    pub uri: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PackageDecl {
    pub path: Vec<Ident>,
    pub contexts: Vec<ContextDecl>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContextDecl {
    /// Path of the classifier, or of the classifier plus operation/property name.
    pub path: Vec<Ident>,
    pub kind: ContextKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ContextKind {
    Classifier { self_name: Option<Ident>, invariants: Vec<Constraint>, definitions: Vec<Definition> },
    Operation { params: Vec<VarDecl>, result: Option<TypeExpr>, constraints: Vec<Constraint> },
    Property { ty: Option<TypeExpr>, constraints: Vec<Constraint> },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ConstraintKind {
    Inv,
    Pre,
    Post,
    Body,
    Init,
    Derive,
}

impl ConstraintKind {
    pub fn keyword(self) -> &'static str {
        match self {
            ConstraintKind::Inv => "inv",
            ConstraintKind::Pre => "pre",
            ConstraintKind::Post => "post",
            ConstraintKind::Body => "body",
            ConstraintKind::Init => "init",
            ConstraintKind::Derive => "derive",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Constraint {
    pub kind: ConstraintKind,
    pub name: Option<Ident>,
    /// Optional message expression: `inv Name('message'): ...`.
    pub message: Option<Expr>,
    pub expr: Expr,
    pub span: Span,
}

/// `def: name(params) : Type = body` or `def: name : Type = body`.
#[derive(Clone, Debug, PartialEq)]
pub struct Definition {
    pub is_static: bool,
    pub name: Ident,
    /// `Some` for operation definitions (possibly empty), `None` for attributes.
    pub params: Option<Vec<VarDecl>>,
    pub ty: Option<TypeExpr>,
    pub body: Expr,
    pub span: Span,
}

// ------------------------------------------------------------------ OCLinEcore

/// An OCLinEcore document: Ecore packages written as text, with embedded OCL.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EcoreDocument {
    pub imports: Vec<Import>,
    pub packages: Vec<PackageDef>,
}

/// `package name : nsPrefix = 'nsURI' { ... }`.
#[derive(Clone, Debug, PartialEq)]
pub struct PackageDef {
    pub name: Ident,
    pub ns_prefix: Option<Ident>,
    pub ns_uri: Option<(String, Span)>,
    pub annotations: Vec<AnnotationDef>,
    pub packages: Vec<PackageDef>,
    pub classifiers: Vec<ClassifierDef>,
    pub span: Span,
    /// The braces of the body (`None` for `package p : p = 'u';`).
    pub body: Option<Span>,
}

/// `annotation 'source' ('key' = 'value', ...);` or `documentation 'text';`.
#[derive(Clone, Debug, PartialEq)]
pub struct AnnotationDef {
    pub source: String,
    pub details: Vec<(String, String)>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClassifierDef {
    pub name: Ident,
    /// `: 'java.lang.String'`.
    pub instance_class: Option<String>,
    pub annotations: Vec<AnnotationDef>,
    pub kind: ClassifierDefKind,
    pub span: Span,
    /// `class`, `interface`, `enum` or `datatype`.
    pub keyword: Span,
    /// From `extends` to the last supertype.
    pub extends: Option<Span>,
    /// The braces of a qualifier list such as `{ interface }`.
    pub qualifiers: Option<Span>,
    /// The braces of the body (`None` for `class A;`).
    pub body: Option<Span>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClassifierDefKind {
    Class {
        is_abstract: bool,
        is_interface: bool,
        supers: Vec<TypeRef>,
        features: Vec<FeatureDef>,
        operations: Vec<OperationDef>,
        invariants: Vec<Constraint>,
    },
    DataType,
    Enum {
        literals: Vec<LiteralDef>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct LiteralDef {
    pub name: Ident,
    pub value: Option<i64>,
    /// `: 'literal'`.
    pub literal: Option<String>,
    pub span: Span,
}

/// A type reference: `Book`, `ecore::EInt`, or a collection type `Set(Book)`.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeRef {
    pub path: Vec<Ident>,
    pub collection: Option<CollectionKind>,
    pub span: Span,
}

/// `[lower..upper]`, `[*]`, `[+]`, `[?]`, `[n]`. `upper` is -1 for `*`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct MultiplicityDef {
    pub lower: i32,
    pub upper: i32,
    pub span: Span,
}

/// A qualifier such as `ordered`, `!unique` or `composes`.
#[derive(Clone, Debug, PartialEq)]
pub struct Qualifier {
    pub name: String,
    pub negated: bool,
    pub span: Span,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FeatureKeyword {
    Attribute,
    Property,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeatureDef {
    pub keyword: FeatureKeyword,
    pub name: Ident,
    /// `#opposite`.
    pub opposite: Option<Ident>,
    pub ty: Option<TypeRef>,
    pub multiplicity: Option<MultiplicityDef>,
    /// `= 'default value'`.
    pub default: Option<String>,
    pub qualifiers: Vec<Qualifier>,
    /// `initial:` and `derivation:` constraints.
    pub constraints: Vec<Constraint>,
    pub annotations: Vec<AnnotationDef>,
    pub span: Span,
    /// The braces of the qualifier list.
    pub qualifier_span: Option<Span>,
    /// The braces of the body with the constraints.
    pub body: Option<Span>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParamDef {
    pub name: Ident,
    pub ty: Option<TypeRef>,
    pub multiplicity: Option<MultiplicityDef>,
    pub qualifiers: Vec<Qualifier>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OperationDef {
    pub name: Ident,
    pub params: Vec<ParamDef>,
    /// The parentheses around the parameters.
    pub params_span: Span,
    /// `None` for operations without a result.
    pub ty: Option<TypeRef>,
    pub multiplicity: Option<MultiplicityDef>,
    pub throws: Vec<TypeRef>,
    pub qualifiers: Vec<Qualifier>,
    /// `precondition`, `postcondition` and `body` constraints.
    pub constraints: Vec<Constraint>,
    pub annotations: Vec<AnnotationDef>,
    pub span: Span,
    /// The braces of the qualifier list.
    pub qualifier_span: Option<Span>,
    /// The braces of the body with the constraints.
    pub body: Option<Span>,
}
