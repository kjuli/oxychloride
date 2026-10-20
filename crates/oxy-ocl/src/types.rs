//! OCL types and their relation to Ecore classifiers.

use std::fmt;
use std::rc::Rc;

use oxy_ecore::{ClassifierId, DataTypeKind, Metamodel};

pub use crate::syntax::ast::CollectionKind;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Type {
    Boolean,
    Integer,
    Real,
    String,
    UnlimitedNatural,
    OclAny,
    OclVoid,
    OclInvalid,
    /// An Ecore class, enumeration, or a data type without an OCL primitive equivalent.
    Classifier(ClassifierId),
    Collection(CollectionKind, Rc<Type>),
    /// Tuple parts, sorted by name.
    Tuple(Rc<Vec<(String, Type)>>),
    /// The type of an expression that denotes a type, e.g. `Book` in `Book.allInstances()`.
    Meta(Rc<Type>),
    /// Unknown because of an earlier error; suppresses follow-up diagnostics.
    Error,
}

impl Type {
    pub fn collection(kind: CollectionKind, elem: Type) -> Type {
        Type::Collection(kind, Rc::new(elem))
    }

    pub fn tuple(mut parts: Vec<(String, Type)>) -> Type {
        parts.sort_by(|a, b| a.0.cmp(&b.0));
        Type::Tuple(Rc::new(parts))
    }

    /// The OCL type of an Ecore classifier (mapping data types to primitives).
    pub fn of_classifier(mm: &Metamodel, id: ClassifierId) -> Type {
        match mm.data_type_kind(id) {
            Some(DataTypeKind::Boolean) => Type::Boolean,
            Some(DataTypeKind::Integer) => Type::Integer,
            Some(DataTypeKind::Real) => Type::Real,
            Some(DataTypeKind::String) => Type::String,
            Some(DataTypeKind::Other) => Type::Classifier(id),
            None => Type::Classifier(id),
        }
    }

    /// The OCL type of an Ecore typed element (feature, operation, parameter).
    pub fn of_typed(mm: &Metamodel, ty: Option<ClassifierId>, multiplicity: &oxy_ecore::Multiplicity) -> Type {
        let base = ty.map_or(Type::OclAny, |c| Type::of_classifier(mm, c));
        if multiplicity.is_many() {
            Type::collection(CollectionKind::from_flags(multiplicity.ordered, multiplicity.unique), base)
        } else {
            base
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Type::Error)
    }

    pub fn is_collection(&self) -> bool {
        matches!(self, Type::Collection(..))
    }

    pub fn is_numeric(&self) -> bool {
        matches!(self, Type::Integer | Type::Real | Type::UnlimitedNatural)
    }

    /// The element type of a collection type.
    pub fn element(&self) -> Option<&Type> {
        match self {
            Type::Collection(_, e) => Some(e),
            _ => None,
        }
    }

    pub fn collection_kind(&self) -> Option<CollectionKind> {
        match self {
            Type::Collection(k, _) => Some(*k),
            _ => None,
        }
    }

    /// Recursively flattens nested collection types: `Set(Bag(T))` → `T`.
    pub fn flattened_element(&self) -> Type {
        match self {
            Type::Collection(_, e) => e.flattened_element(),
            other => other.clone(),
        }
    }

    pub fn display<'a>(&'a self, mm: &'a Metamodel) -> TypeDisplay<'a> {
        TypeDisplay { ty: self, mm }
    }

    /// Whether `self` conforms to (is a subtype of, or equal to) `other`.
    pub fn conforms_to(&self, other: &Type, mm: &Metamodel) -> bool {
        use Type::*;
        if self == other {
            return true;
        }
        match (self, other) {
            (Error, _) | (_, Error) => true,
            (OclVoid | OclInvalid, _) => true,
            (_, OclAny) => !matches!(self, Meta(_)),
            (UnlimitedNatural, Integer | Real) | (Integer, Real) => true,
            (Classifier(a), Classifier(b)) => mm.conforms_to(*a, *b),
            (Collection(k1, e1), Collection(k2, e2)) => {
                (k1 == k2 || *k2 == CollectionKind::Collection) && e1.conforms_to(e2, mm)
            }
            (Tuple(a), Tuple(b)) => {
                a.len() == b.len()
                    && a.iter().zip(b.iter()).all(|((n1, t1), (n2, t2))| n1 == n2 && t1.conforms_to(t2, mm))
            }
            (Meta(a), Meta(b)) => a.conforms_to(b, mm),
            _ => false,
        }
    }

    /// The most specific common supertype.
    pub fn common_supertype(&self, other: &Type, mm: &Metamodel) -> Type {
        use Type::*;
        if self.conforms_to(other, mm) && !matches!(self, Error) {
            return other.clone();
        }
        if other.conforms_to(self, mm) {
            return self.clone();
        }
        match (self, other) {
            (Classifier(a), Classifier(b)) if mm.is_class(*a) && mm.is_class(*b) => {
                for s in mm.all_super_types(*a) {
                    if mm.conforms_to(*b, *s) {
                        return Classifier(*s);
                    }
                }
                Classifier(mm.eobject())
            }
            (Collection(k1, e1), Collection(k2, e2)) => {
                let kind = if k1 == k2 { *k1 } else { CollectionKind::Collection };
                Type::collection(kind, e1.common_supertype(e2, mm))
            }
            _ => OclAny,
        }
    }
}

pub struct TypeDisplay<'a> {
    ty: &'a Type,
    mm: &'a Metamodel,
}

impl fmt::Display for TypeDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.ty {
            Type::Boolean => f.write_str("Boolean"),
            Type::Integer => f.write_str("Integer"),
            Type::Real => f.write_str("Real"),
            Type::String => f.write_str("String"),
            Type::UnlimitedNatural => f.write_str("UnlimitedNatural"),
            Type::OclAny | Type::Error => f.write_str("OclAny"),
            Type::OclVoid => f.write_str("OclVoid"),
            Type::OclInvalid => f.write_str("OclInvalid"),
            Type::Classifier(c) => f.write_str(&self.mm.classifier(*c).name),
            Type::Collection(k, e) => write!(f, "{}({})", k.name(), e.display(self.mm)),
            Type::Tuple(parts) => {
                f.write_str("Tuple(")?;
                for (i, (n, t)) in parts.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{n} : {}", t.display(self.mm))?;
                }
                f.write_str(")")
            }
            Type::Meta(t) => write!(f, "{}", t.display(self.mm)),
        }
    }
}

/// Resolves a primitive or OCL library type name.
pub fn library_type(name: &str) -> Option<Type> {
    Some(match name {
        "Boolean" => Type::Boolean,
        "Integer" => Type::Integer,
        "Real" => Type::Real,
        "String" => Type::String,
        "UnlimitedNatural" => Type::UnlimitedNatural,
        "OclAny" => Type::OclAny,
        "OclVoid" => Type::OclVoid,
        "OclInvalid" => Type::OclInvalid,
        _ => return None,
    })
}
