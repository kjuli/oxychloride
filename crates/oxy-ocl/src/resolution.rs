//! How the type checker resolved the names, properties and calls of an expression.
//!
//! OCL's name resolution is contextual: `name` may be a variable, a property of `self` or of
//! an implicit iterator, or a type; `coll.name` collects `name` from every element; `op()` may
//! be an operation of the metamodel, a `def:` operation or a library operation. The checker
//! (`check.rs`) decides this once and records the outcome per expression in an [`Analysis`],
//! together with the expression's static type. Tools that translate OCL, such as code
//! generators, read it (see [`crate::Environment::analysis`]) instead of repeating the rules.

use std::collections::HashMap;

use oxy_ecore::{ClassifierId, FeatureId, OperationId};

use crate::syntax::ast::ExprId;
use crate::types::Type;

/// A property as resolved by the checker.
#[derive(Clone, Debug, PartialEq)]
pub enum Property {
    /// A structural feature of the metamodel (possibly derived).
    Feature(FeatureId),
    /// A `def:` attribute of a Complete OCL document, declared on `class` (the class the
    /// source conforms to, or one of its supertypes).
    DefAttribute { class: ClassifierId, name: String },
    /// A part of a tuple.
    TuplePart(String),
    /// `T.name` for a type `T`.
    TypeName,
}

/// A user-defined operation as resolved by the checker.
#[derive(Clone, Debug, PartialEq)]
pub enum UserOperation {
    /// An operation of the metamodel (an `EOperation`). Calls dispatch on the class of the
    /// object, so an operation of a subclass with the same name and arity overrides it.
    Operation(OperationId),
    /// A `def:` operation of a Complete OCL document, declared on `class`.
    Def { class: ClassifierId, name: String, arity: usize },
}

/// How a name, property or call expression was resolved.
///
/// Implicit sources are counted from the innermost: depth 0 is the element of the innermost
/// iterator without declared variables (`books->select(copies > 0)`), or `self` when there is
/// none; depth 1 is the next outer one, and so on.
#[derive(Clone, Debug, PartialEq)]
pub enum Resolution {
    /// A variable: `self` (or its alias), a parameter, `result`, a `let`, iterator or
    /// accumulator variable.
    Variable,
    /// A name that is a property of an implicit source.
    ImplicitProperty { depth: usize, property: Property },
    /// `source.name`. With `collect`, the source is a collection and the property is
    /// collected from its elements (and flattened), as `source->collect(name)`.
    Property { property: Property, collect: bool },
    /// A name that denotes a type (e.g. `Book` in `Book.allInstances()`).
    Type(Type),
    /// `Enumeration::Literal`; `literal` indexes the enumeration's literals.
    EnumLiteral { enumeration: ClassifierId, literal: usize },
    /// A call of a user-defined operation: on the call's source, or (`implicit`) on the
    /// implicit source of that depth. With `collect`, the source is a collection and the
    /// operation is called on every element (implicit collect).
    Operation { operation: UserOperation, implicit: Option<usize>, collect: bool },
    /// A call of a library operation (of numbers, strings, collections, types or `OclAny`),
    /// named like the call. A call without a source applies to the innermost implicit
    /// source. With `collect`, the source is a collection and the operation is applied to
    /// every element (`names.toUpper()`).
    Library { collect: bool },
}

/// The static types and resolutions of the expressions of one source.
#[derive(Clone, Debug, Default)]
pub struct Analysis {
    pub(crate) types: HashMap<ExprId, Type>,
    pub(crate) resolutions: HashMap<ExprId, Resolution>,
}

impl Analysis {
    /// The static type of an expression (absent for expressions that were not checked, such
    /// as those after a syntax error).
    pub fn type_of(&self, expr: ExprId) -> Option<&Type> {
        self.types.get(&expr)
    }

    /// How a name, property or call expression was resolved (absent for other expressions,
    /// and for ones that could not be resolved).
    pub fn resolution(&self, expr: ExprId) -> Option<&Resolution> {
        self.resolutions.get(&expr)
    }

    pub(crate) fn extend(&mut self, other: Analysis) {
        self.types.extend(other.types);
        self.resolutions.extend(other.resolutions);
    }
}
