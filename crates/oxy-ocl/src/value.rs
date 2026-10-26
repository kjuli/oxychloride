//! Runtime values of the OCL evaluator.

use std::cmp::Ordering;
use std::fmt::{self, Write as _};
use std::rc::Rc;

use oxy_ecore::{ClassifierId, Metamodel, Model, ObjectId};

use crate::types::{CollectionKind, Type};

#[derive(Clone, Debug)]
pub enum Value {
    Null,
    /// The result of a failed computation, with an explanation.
    Invalid(Rc<str>),
    Boolean(bool),
    Integer(i64),
    Real(f64),
    String(Rc<str>),
    /// The unlimited natural `*`.
    Unlimited,
    Object(ObjectId),
    /// An enumeration literal: enum and literal index.
    Enum(ClassifierId, u32),
    Collection(Rc<Collection>),
    /// Tuple parts, sorted by name.
    Tuple(Rc<Vec<(Rc<str>, Value)>>),
    Type(Rc<Type>),
    /// A model parameter of a QVT transformation (by name).
    Model(Rc<str>),
}

#[derive(Clone, Debug)]
pub struct Collection {
    pub kind: CollectionKind,
    pub items: Vec<Value>,
}

impl Value {
    pub fn invalid(message: impl Into<String>) -> Value {
        Value::Invalid(Rc::from(message.into()))
    }

    pub fn string(s: impl AsRef<str>) -> Value {
        Value::String(Rc::from(s.as_ref()))
    }

    /// Builds a collection, removing duplicates for sets.
    pub fn collection(kind: CollectionKind, items: Vec<Value>) -> Value {
        let items = if kind.is_unique() { dedup(items) } else { items };
        let kind = if kind == CollectionKind::Collection { CollectionKind::Bag } else { kind };
        Value::Collection(Rc::new(Collection { kind, items }))
    }

    pub fn tuple(mut parts: Vec<(Rc<str>, Value)>) -> Value {
        parts.sort_by(|a, b| a.0.cmp(&b.0));
        Value::Tuple(Rc::new(parts))
    }

    pub fn is_invalid(&self) -> bool {
        matches!(self, Value::Invalid(_))
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn is_undefined(&self) -> bool {
        matches!(self, Value::Null | Value::Invalid(_))
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_collection(&self) -> Option<&Collection> {
        match self {
            Value::Collection(c) => Some(c),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Integer(i) => Some(*i as f64),
            Value::Real(r) => Some(*r),
            _ => None,
        }
    }

    /// The runtime type of a value.
    pub fn type_of(&self, model: &Model) -> Type {
        match self {
            Value::Null => Type::OclVoid,
            Value::Invalid(_) => Type::OclInvalid,
            Value::Boolean(_) => Type::Boolean,
            Value::Integer(_) => Type::Integer,
            Value::Real(_) => Type::Real,
            Value::String(_) => Type::String,
            Value::Unlimited => Type::UnlimitedNatural,
            Value::Object(o) => Type::Classifier(model.class_of(*o)),
            Value::Enum(e, _) => Type::Classifier(*e),
            Value::Collection(c) => {
                // The element type is not tracked at runtime.
                Type::collection(c.kind, Type::OclAny)
            }
            Value::Tuple(parts) => Type::tuple(parts.iter().map(|(n, v)| (n.to_string(), v.type_of(model))).collect()),
            Value::Type(t) => Type::Meta(t.clone()),
            Value::Model(_) => Type::Model(Rc::from("")),
        }
    }

    /// OCL equality (`=`). Integers and reals compare numerically; collections compare by
    /// kind and contents (ignoring order for sets and bags).
    pub fn ocl_equals(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Null, Value::Null) => true,
            (Value::Boolean(a), Value::Boolean(b)) => a == b,
            (Value::Integer(a), Value::Integer(b)) => a == b,
            (Value::Integer(_) | Value::Real(_), Value::Integer(_) | Value::Real(_)) => self.as_f64() == other.as_f64(),
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Unlimited, Value::Unlimited) => true,
            (Value::Object(a), Value::Object(b)) => a == b,
            (Value::Enum(e1, l1), Value::Enum(e2, l2)) => e1 == e2 && l1 == l2,
            (Value::Collection(a), Value::Collection(b)) => {
                if a.kind != b.kind || a.items.len() != b.items.len() {
                    return false;
                }
                if a.kind.is_ordered() {
                    a.items.iter().zip(&b.items).all(|(x, y)| x.ocl_equals(y))
                } else {
                    let mut ka: Vec<Key> = a.items.iter().map(Value::key).collect();
                    let mut kb: Vec<Key> = b.items.iter().map(Value::key).collect();
                    ka.sort();
                    kb.sort();
                    ka == kb
                }
            }
            (Value::Tuple(a), Value::Tuple(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|((n1, v1), (n2, v2))| n1 == n2 && v1.ocl_equals(v2))
            }
            (Value::Type(a), Value::Type(b)) => a == b,
            (Value::Model(a), Value::Model(b)) => a == b,
            _ => false,
        }
    }

    /// A hashable/orderable identity consistent with [`Value::ocl_equals`].
    pub fn key(&self) -> Key {
        match self {
            Value::Null => Key::Null,
            Value::Invalid(_) => Key::Invalid,
            Value::Boolean(b) => Key::Bool(*b),
            Value::Integer(i) => Key::Int(*i),
            Value::Real(r) => {
                if r.fract() == 0.0 && r.abs() < 9.2e18 {
                    Key::Int(*r as i64)
                } else {
                    Key::Real(if *r == 0.0 { 0 } else { r.to_bits() })
                }
            }
            Value::String(s) => Key::Str(s.clone()),
            Value::Unlimited => Key::Unlimited,
            Value::Object(o) => Key::Object(o.0),
            Value::Enum(e, l) => Key::Enum(e.0, *l),
            Value::Collection(c) => {
                let mut keys: Vec<Key> = c.items.iter().map(Value::key).collect();
                if !c.kind.is_ordered() {
                    keys.sort();
                }
                Key::Collection(c.kind as u8, keys)
            }
            Value::Tuple(parts) => Key::Tuple(parts.iter().map(|(n, v)| (n.clone(), v.key())).collect()),
            Value::Type(t) => Key::Type(format!("{t:?}")),
            Value::Model(m) => Key::Model(m.clone()),
        }
    }

    /// Ordering used by `<`, `max`, `sortedBy`: numbers, strings, and unlimited naturals.
    pub fn ocl_compare(&self, other: &Value) -> Option<Ordering> {
        match (self, other) {
            (Value::Integer(a), Value::Integer(b)) => Some(a.cmp(b)),
            (Value::Unlimited, Value::Unlimited) => Some(Ordering::Equal),
            (Value::Unlimited, Value::Integer(_) | Value::Real(_)) => Some(Ordering::Greater),
            (Value::Integer(_) | Value::Real(_), Value::Unlimited) => Some(Ordering::Less),
            (Value::Integer(_) | Value::Real(_), Value::Integer(_) | Value::Real(_)) => {
                self.as_f64()?.partial_cmp(&other.as_f64()?)
            }
            (Value::String(a), Value::String(b)) => Some(a.cmp(b)),
            (Value::Enum(e1, a), Value::Enum(e2, b)) if e1 == e2 => Some(a.cmp(b)),
            _ => None,
        }
    }

    /// Renders the value in OCL literal syntax, e.g. `Set{1, 'a'}` or `Book 'Dune'`.
    pub fn render(&self, mm: &Metamodel, model: &Model) -> String {
        let mut out = String::new();
        self.render_into(mm, model, &mut out);
        out
    }

    fn render_into(&self, mm: &Metamodel, model: &Model, out: &mut String) {
        match self {
            Value::Null => out.push_str("null"),
            Value::Invalid(m) => {
                let _ = write!(out, "invalid ({m})");
            }
            Value::Boolean(b) => {
                let _ = write!(out, "{b}");
            }
            Value::Integer(i) => {
                let _ = write!(out, "{i}");
            }
            Value::Real(r) => out.push_str(&format_real(*r)),
            Value::String(s) => {
                out.push('\'');
                for c in s.chars() {
                    match c {
                        '\'' => out.push_str("\\'"),
                        '\\' => out.push_str("\\\\"),
                        '\n' => out.push_str("\\n"),
                        '\t' => out.push_str("\\t"),
                        c => out.push(c),
                    }
                }
                out.push('\'');
            }
            Value::Unlimited => out.push('*'),
            Value::Object(o) => out.push_str(&model.label(mm, *o)),
            Value::Enum(e, l) => {
                let _ = write!(
                    out,
                    "{}::{}",
                    mm.classifier(*e).name,
                    mm.enum_literals(*e).get(*l as usize).map_or("?", |l| l.name.as_str())
                );
            }
            Value::Collection(c) => {
                out.push_str(c.kind.name());
                out.push('{');
                for (i, v) in c.items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    v.render_into(mm, model, out);
                }
                out.push('}');
            }
            Value::Tuple(parts) => {
                out.push_str("Tuple{");
                for (i, (n, v)) in parts.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    let _ = write!(out, "{n} = ");
                    v.render_into(mm, model, out);
                }
                out.push('}');
            }
            Value::Type(t) => {
                let _ = write!(out, "{}", t.display(mm));
            }
            Value::Model(m) => {
                let _ = write!(out, "model {m}");
            }
        }
    }

    /// The `toString()` form: like [`Value::render`] but strings are not quoted.
    pub fn to_ocl_string(&self, mm: &Metamodel, model: &Model) -> String {
        match self {
            Value::String(s) => s.to_string(),
            other => other.render(mm, model),
        }
    }
}

pub fn format_real(r: f64) -> String {
    if r.is_finite() && r.fract() == 0.0 && r.abs() < 1e16 { format!("{r:.1}") } else { format!("{r}") }
}

/// Structural identity of a value, used to implement sets.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Key {
    Null,
    Invalid,
    Bool(bool),
    Int(i64),
    Real(u64),
    Str(Rc<str>),
    Unlimited,
    Object(u32),
    Enum(u32, u32),
    Collection(u8, Vec<Key>),
    Tuple(Vec<(Rc<str>, Key)>),
    Type(String),
    Model(Rc<str>),
}

/// Removes duplicates, keeping the first occurrence.
pub fn dedup(items: Vec<Value>) -> Vec<Value> {
    let mut seen = std::collections::HashSet::new();
    items.into_iter().filter(|v| seen.insert(v.key())).collect()
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => f.write_str("null"),
            Value::Invalid(m) => write!(f, "invalid ({m})"),
            Value::Boolean(b) => write!(f, "{b}"),
            Value::Integer(i) => write!(f, "{i}"),
            Value::Real(r) => f.write_str(&format_real(*r)),
            Value::String(s) => write!(f, "'{s}'"),
            Value::Unlimited => f.write_str("*"),
            Value::Object(o) => write!(f, "<object {}>", o.0),
            Value::Enum(e, l) => write!(f, "<enum {}#{}>", e.0, l),
            Value::Collection(c) => {
                write!(f, "{}{{", c.kind.name())?;
                for (i, v) in c.items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{v}")?;
                }
                f.write_str("}")
            }
            Value::Tuple(parts) => {
                f.write_str("Tuple{")?;
                for (i, (n, v)) in parts.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{n} = {v}")?;
                }
                f.write_str("}")
            }
            Value::Type(t) => write!(f, "{t:?}"),
            Value::Model(m) => write!(f, "model {m}"),
        }
    }
}
