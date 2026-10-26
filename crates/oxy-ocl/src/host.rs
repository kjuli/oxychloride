//! Hooks for languages built on OCL: QVT transformations add names (`this`, model parameters,
//! properties of the transformation), operations (helpers, queries, library operations) and
//! expressions (`object`, `map`, `resolve`, `new`) whose meaning depends on the transformation
//! being checked or run. The checker asks a [`CheckHost`], the evaluator a [`Host`].
//!
//! Names are looked up in the same order by both: variables of the expression, then the
//! host's names, then properties of the implicit sources (iterator elements, the object being
//! populated, `self`), then types. Calls without a source try the operations of the implicit
//! sources (the metamodel's, `def:`s, then the host's contextual ones), then the host's global
//! ones, then the library operations of the innermost implicit source. Calls with a source try
//! the metamodel's and `def:` operations, then the host's, then OCL's library.

use oxy_ecore::{ClassifierId, Metamodel, ObjectId};

use crate::eval::Evaluator;
use crate::syntax::ast::{Ident, TypeExpr};
use crate::types::Type;
use crate::value::Value;

/// What a language adds to the type checking of its OCL expressions.
pub trait CheckHost {
    /// The type of a name that is not a variable of the expression: `this`, a model
    /// parameter, a property of the transformation.
    fn variable(&self, _name: &str) -> Option<Type> {
        None
    }

    /// Whether a name of [`CheckHost::variable`] can be assigned.
    fn assignable(&self, _name: &str) -> bool {
        false
    }

    /// The result type of an operation the language defines on `source` (or, without a
    /// source, a global one): helpers, queries, library additions. `Err` says why the
    /// arguments do not fit; `None`: there is no such operation.
    fn operation(
        &self,
        _mm: &Metamodel,
        _source: Option<&Type>,
        _name: &str,
        _args: &[Type],
    ) -> Option<Result<Type, String>> {
        None
    }

    /// The type of a property the language defines on objects of `source` (intermediate
    /// properties).
    fn property(&self, _mm: &Metamodel, _source: &Type, _name: &str) -> Option<Type> {
        None
    }

    /// The result type of a mapping call on `source` (`None`: without a source); `Err` says
    /// what is wrong.
    fn mapping(&self, _mm: &Metamodel, _source: Option<&Type>, name: &[Ident], _args: &[Type]) -> Result<Type, String> {
        Err(format!("unknown mapping '{}'", path_text(name)))
    }

    /// Checks the mapping named in a `resolveIn`.
    fn mapping_reference(&self, _mm: &Metamodel, name: &[Ident]) -> Result<(), String> {
        Err(format!("unknown mapping '{}'", path_text(name)))
    }

    /// Checks a model parameter named after `@`, for objects of `class`.
    fn extent(&self, name: &str, _class: Option<ClassifierId>) -> Result<(), String> {
        Err(format!("unknown model parameter '{name}'"))
    }

    /// The type of `new T(args)` if the language gives `T` a meaning other than a class (a
    /// QVTo transformation to run); `None` if it does not.
    fn instantiate(&self, _mm: &Metamodel, _ty: &TypeExpr, _args: &[Type]) -> Option<Result<Type, String>> {
        None
    }

    /// A type the language names that is no classifier (QVTo: a transformation, `Status`).
    fn named_type(&self, _path: &[Ident]) -> Option<Type> {
        None
    }

    /// Whether objects of `class` (`object`, `new`) can be created here without naming the
    /// model to create them in.
    fn can_create(&self, _mm: &Metamodel, _class: ClassifierId) -> Result<(), String> {
        Err("objects can only be created in transformations".into())
    }
}

/// What a `resolve` asks for (besides its type and condition, which the evaluator applies).
#[derive(Clone, Debug)]
pub struct ResolveRequest {
    /// The objects to resolve (`None`: all objects the mappings had, e.g. `resolveIn(T::m)`).
    pub source: Option<Value>,
    /// From targets back to sources (`invresolve`).
    pub inverse: bool,
    /// Only the results of this mapping (`resolveIn`): its context type's name (if given)
    /// and its name.
    pub mapping: Option<Vec<String>>,
}

/// What a language adds to the evaluation of its OCL expressions. The methods take the
/// evaluator, so that they can evaluate further expressions (mapping bodies, helpers) and
/// change the model; a host keeps its own state behind shared references (`RefCell`), since
/// these calls nest.
pub trait Host {
    /// The value of a name of [`CheckHost::variable`].
    fn variable(&self, _ev: &mut Evaluator<'_>, _name: &str) -> Option<Value> {
        None
    }

    /// Assigns a name of [`CheckHost::variable`]; `false` if there is no such name.
    fn assign_variable(&self, _ev: &mut Evaluator<'_>, _name: &str, _value: Value) -> bool {
        false
    }

    /// Calls an operation the language defines (see [`CheckHost::operation`]); `None` if there
    /// is none for these values.
    fn operation(
        &self,
        _ev: &mut Evaluator<'_>,
        _source: Option<&Value>,
        _name: &str,
        _args: &[Value],
    ) -> Option<Value> {
        None
    }

    /// The value of a property the language defines on objects (intermediate properties) or on
    /// its own values (`this.x`; see [`CheckHost::property`]).
    fn property(&self, _ev: &mut Evaluator<'_>, _source: &Value, _name: &str) -> Option<Value> {
        None
    }

    /// Assigns a property the language defines; `None` if there is no such property.
    fn assign_property(
        &self,
        _ev: &mut Evaluator<'_>,
        _target: &Value,
        _name: &str,
        _value: Value,
        _add: bool,
    ) -> Option<Result<(), String>> {
        None
    }

    /// Calls a mapping (the evaluator has applied `->` to the elements of the source already).
    fn map(
        &self,
        _ev: &mut Evaluator<'_>,
        _source: Option<Value>,
        _strict: bool,
        name: &[Ident],
        _args: Vec<Value>,
    ) -> Value {
        Value::invalid(format!("unknown mapping '{}'", path_text(name)))
    }

    /// A type the language names (see [`CheckHost::named_type`]).
    fn named_type(&self, _path: &[Ident]) -> Option<Type> {
        None
    }

    /// Evaluates `new T(args)` for a `T` of [`CheckHost::instantiate`]; `None` if `T` is a
    /// class.
    fn instantiate(&self, _ev: &mut Evaluator<'_>, _ty: &TypeExpr, _args: Vec<Value>) -> Option<Value> {
        None
    }

    /// Creates an object of `class` (in the model parameter `extent`, or the one the class
    /// belongs to).
    fn create(&self, _ev: &mut Evaluator<'_>, _class: ClassifierId, _extent: Option<&str>) -> Result<ObjectId, String> {
        Err("objects can only be created in transformations".into())
    }

    /// The objects a `resolve` chooses from: the targets (or, inverse, the sources) of the
    /// mapping applications recorded for the source.
    fn resolve(&self, _ev: &mut Evaluator<'_>, _request: &ResolveRequest) -> Vec<Value> {
        Vec::new()
    }

    /// Remembers an assignment of a `late resolve`, to be done at the end of the
    /// transformation (with [`Evaluator::complete_late`]). `false`: not supported, it is
    /// resolved at once.
    fn defer(&self, _ev: &mut Evaluator<'_>, _late: crate::eval::LateResolve) -> bool {
        false
    }

    /// Writes a message of `log`/`assert`.
    fn log(&self, _ev: &mut Evaluator<'_>, _message: String) {}

    /// Whether an object may be changed (objects of input models may not).
    fn writable(&self, _ev: &Evaluator<'_>, _object: ObjectId) -> Result<(), String> {
        Ok(())
    }
}

/// `a::b::c`.
pub fn path_text(path: &[Ident]) -> String {
    path.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join("::")
}
