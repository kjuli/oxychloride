//! The OCL library operations a target must implement.
//!
//! After lowering, every call of a library operation is an [`ExprKind::Library`] node whose
//! `name` is one of the canonical names below (synonyms such as `toUpperCase` or `eContainer`
//! are mapped to them by [`canonical_name`]). The node's `source` is the receiver: for
//! [`Receiver::Collection`] operations always a collection (a single value used with `->` is
//! wrapped in `oclAsSet`). Operators (`+`, `and`, `=`, …) are [`ExprKind::Binary`] and
//! [`ExprKind::Unary`] nodes, iterators [`ExprKind::Iterator`]s, and `allInstances()` is
//! [`ExprKind::AllInstances`].
//!
//! The semantics are those of the engine's evaluator (`oxy-ocl/src/eval.rs`): positions are
//! 1-based, a failure (index out of range, division by zero, an empty collection's `first`)
//! makes the result `invalid` (an exception or error value in the generated code).
//!
//! [`ExprKind::Library`]: crate::model::ExprKind::Library
//! [`ExprKind::Binary`]: crate::model::ExprKind::Binary
//! [`ExprKind::Unary`]: crate::model::ExprKind::Unary
//! [`ExprKind::Iterator`]: crate::model::ExprKind::Iterator
//! [`ExprKind::AllInstances`]: crate::model::ExprKind::AllInstances

/// What a library operation is called on.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Receiver {
    /// Any value (also collections and null).
    Any,
    /// Integers and reals.
    Number,
    String,
    Collection,
}

/// A library operation: its canonical name, receiver, signature and meaning.
#[derive(Copy, Clone, Debug)]
pub struct LibraryOperation {
    pub name: &'static str,
    pub receiver: Receiver,
    pub signature: &'static str,
    pub doc: &'static str,
}

const fn op(name: &'static str, receiver: Receiver, signature: &'static str, doc: &'static str) -> LibraryOperation {
    LibraryOperation { name, receiver, signature, doc }
}

use Receiver::*;

/// Every library operation, by receiver.
pub const OPERATIONS: &[LibraryOperation] = &[
    // OclAny
    op("oclIsUndefined", Any, "() : Boolean", "the value is null (or invalid)"),
    op("oclIsInvalid", Any, "() : Boolean", "evaluating the value failed"),
    op("oclIsKindOf", Any, "(T) : Boolean", "the value conforms to type T (null conforms to nothing)"),
    op("oclIsTypeOf", Any, "(T) : Boolean", "the value's type is exactly T"),
    op("oclAsType", Any, "(T) : T", "the value as a T (invalid if it does not conform; null stays null)"),
    op("oclAsSet", Any, "() : Set(T)", "Set{value}, or Set{} for null"),
    op("oclType", Any, "() : type", "the value's type"),
    op("toString", Any, "() : String", "the value as text (OCL syntax: 'abc' is abc, 1.0 is 1.0, true is true)"),
    op("oclContainer", Any, "() : OclAny", "the object containing an object, or null"),
    op("oclContents", Any, "() : OrderedSet(OclAny)", "the objects an object contains directly"),
    op("eAllContents", Any, "() : OrderedSet(OclAny)", "the objects an object contains, transitively (depth first)"),
    // Integer and Real
    op("abs", Number, "() : Integer|Real", "absolute value"),
    op("floor", Number, "() : Integer", "largest integer not greater"),
    op("round", Number, "() : Integer", "nearest integer, halves rounded up"),
    op("max", Number, "(n) : Integer|Real", "the larger value (Real if either is Real)"),
    op("min", Number, "(n) : Integer|Real", "the smaller value (Real if either is Real)"),
    op("div", Number, "(i : Integer) : Integer", "integer division truncating toward zero; invalid for 0"),
    op("mod", Number, "(i : Integer) : Integer", "remainder of div (sign of the dividend); invalid for 0"),
    op("toInteger", Number, "() : Integer", "a Real truncated toward zero"),
    op("toReal", Number, "() : Real", "the value as a Real"),
    // String (positions count characters, 1-based)
    op("size", String, "() : Integer", "number of characters"),
    op("concat", String, "(s : String) : String", "concatenation"),
    op("substring", String, "(lower, upper : Integer) : String", "characters lower..upper (inclusive)"),
    op("toInteger", String, "() : Integer", "parses an integer (surrounding whitespace allowed)"),
    op("toReal", String, "() : Real", "parses a number (surrounding whitespace allowed)"),
    op("toBoolean", String, "() : Boolean", "'true' or 'false' (surrounding whitespace allowed)"),
    op("toUpper", String, "() : String", "upper case (also toUpperCase)"),
    op("toLower", String, "() : String", "lower case (also toLowerCase)"),
    op("trim", String, "() : String", "without leading and trailing whitespace"),
    op("indexOf", String, "(s : String) : Integer", "position of the first occurrence, 0 if none"),
    op("lastIndexOf", String, "(s : String) : Integer", "position of the last occurrence, 0 if none"),
    op("at", String, "(i : Integer) : String", "the character at position i"),
    op("characters", String, "() : Sequence(String)", "the characters"),
    op("equalsIgnoreCase", String, "(s : String) : Boolean", "equal ignoring case"),
    op("startsWith", String, "(s : String) : Boolean", "prefix test"),
    op("endsWith", String, "(s : String) : Boolean", "suffix test"),
    op("matches", String, "(regex : String) : Boolean", "the whole string matches the regular expression"),
    op("replaceAll", String, "(regex, replacement : String) : String", "replaces all matches ($1 refers to groups)"),
    op("replaceFirst", String, "(regex, replacement : String) : String", "replaces the first match"),
    op("substituteAll", String, "(old, new : String) : String", "replaces all occurrences of a plain string"),
    op("substituteFirst", String, "(old, new : String) : String", "replaces the first occurrence of a plain string"),
    op(
        "tokenize",
        String,
        "([delimiters : String [, keep : Boolean]]) : Sequence(String)",
        "splits at any of the delimiter characters (default: whitespace); with keep, delimiters are tokens too",
    ),
    // Collections (`self` is the source collection; its kind decides uniqueness and order)
    op("size", Collection, "() : Integer", "number of elements"),
    op("count", Collection, "(x) : Integer", "number of occurrences of x"),
    op("includes", Collection, "(x) : Boolean", "x is an element"),
    op("excludes", Collection, "(x) : Boolean", "x is not an element"),
    op("includesAll", Collection, "(c) : Boolean", "all elements of c are elements"),
    op("excludesAll", Collection, "(c) : Boolean", "no element of c is an element"),
    op("isEmpty", Collection, "() : Boolean", "no elements"),
    op("notEmpty", Collection, "() : Boolean", "some elements"),
    op("sum", Collection, "() : Integer|Real", "sum of the elements (0 when empty)"),
    op("max", Collection, "() : T", "the largest element (numbers or strings); invalid when empty"),
    op("min", Collection, "() : T", "the smallest element; invalid when empty"),
    op("asSet", Collection, "() : Set(T)", "without duplicates (first occurrences kept, in order)"),
    op("asOrderedSet", Collection, "() : OrderedSet(T)", "without duplicates, in order"),
    op("asSequence", Collection, "() : Sequence(T)", "the elements in order"),
    op("asBag", Collection, "() : Bag(T)", "the elements"),
    op("flatten", Collection, "() : Collection(T')", "nested collections' elements, recursively"),
    op("including", Collection, "(x) : Collection(T)", "with x added (at the end; not again for unique kinds)"),
    op("excluding", Collection, "(x) : Collection(T)", "without any occurrence of x"),
    op("includingAll", Collection, "(c) : Collection(T)", "with the elements of c added"),
    op("excludingAll", Collection, "(c) : Collection(T)", "without the elements of c"),
    op(
        "union",
        Collection,
        "(c) : Collection(T)",
        "elements of both (kind: Set∪Set=Set, ordered∪ordered=Sequence, else Bag)",
    ),
    op("intersection", Collection, "(c) : Set(T)|Bag(T)", "common elements (Bag: minimum counts)"),
    op("symmetricDifference", Collection, "(c) : Set(T)", "elements in exactly one of them"),
    op("append", Collection, "(x) : Collection(T)", "x at the end (moved there for unique kinds)"),
    op("prepend", Collection, "(x) : Collection(T)", "x at the start (moved there for unique kinds)"),
    op("insertAt", Collection, "(i : Integer, x) : Collection(T)", "x inserted at position i"),
    op("at", Collection, "(i : Integer) : T", "the element at position i"),
    op("first", Collection, "() : T", "the first element; invalid when empty"),
    op("last", Collection, "() : T", "the last element; invalid when empty"),
    op("indexOf", Collection, "(x) : Integer", "position of the first occurrence of x; invalid if absent"),
    op(
        "subSequence",
        Collection,
        "(lower, upper : Integer) : Collection(T)",
        "elements lower..upper (also subOrderedSet)",
    ),
    op("reverse", Collection, "() : Collection(T)", "the elements in reverse order"),
    op("selectByKind", Collection, "(T) : Collection(T)", "the elements conforming to T"),
    op("selectByType", Collection, "(T) : Collection(T)", "the elements of exactly type T"),
    op("product", Collection, "(c) : Set(Tuple(first, second))", "all pairs"),
];

/// The canonical name of a library operation called `name`.
pub fn canonical_name(name: &str) -> &str {
    match name {
        "toUpperCase" => "toUpper",
        "toLowerCase" => "toLower",
        "eContainer" => "oclContainer",
        "eContents" => "oclContents",
        "eClass" => "oclType",
        "subOrderedSet" => "subSequence",
        other => other,
    }
}

/// The library operation called `name` on a receiver, if there is one.
pub fn find(name: &str, receiver: Receiver) -> Option<&'static LibraryOperation> {
    OPERATIONS.iter().find(|o| o.name == name && o.receiver == receiver)
}
