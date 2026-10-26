//! Values of parameters and attributes, and the evaluation of attribute expressions.
//!
//! Henshin evaluates attribute expressions with a JavaScript engine; the textual syntax only
//! has a small expression language (literals, parameters, arithmetic, comparisons, `AND`, `OR`,
//! `!`), which is evaluated here with the usual (JavaScript-like) semantics: `+` concatenates
//! when one side is a string, numbers compare across integers and reals.

use std::fmt;

use oxy_ecore::{ClassifierId, DataTypeKind, FeatureId, Metamodel, ModelValue};

use crate::graph::{Graph, NodeId};
use crate::syntax::BinOp;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    Str(String),
    /// An enumeration literal: the enum and the index of the literal.
    Enum(ClassifierId, usize),
    Node(NodeId),
}

impl Value {
    pub fn from_model(v: &ModelValue) -> Value {
        match v {
            ModelValue::Boolean(b) => Value::Bool(*b),
            ModelValue::Integer(i) => Value::Int(*i),
            ModelValue::Real(r) => Value::Real(*r),
            ModelValue::String(s) => Value::Str(s.clone()),
            ModelValue::Enum(e, l) => Value::Enum(*e, *l),
            // Attributes never hold objects.
            ModelValue::Object(o) => Value::Node(NodeId(o.0)),
        }
    }

    /// The value of an attribute of a node (its default value if unset).
    pub fn of_attribute(mm: &Metamodel, graph: &Graph, node: NodeId, feature: FeatureId) -> Value {
        match graph.attribute(node, feature).first() {
            Some(v) => Value::from_model(v),
            None => oxy_ecore::default_value(mm, feature).map_or(Value::Null, |v| Value::from_model(&v)),
        }
    }

    /// The value as the value of an attribute: converted to the attribute's type, or an error.
    pub fn to_model(&self, mm: &Metamodel, feature: FeatureId) -> Result<Option<ModelValue>, String> {
        let f = mm.feature(feature);
        let Some(ty) = f.ty else { return Err(format!("attribute '{}' has no type", f.name)) };
        let mismatch = || {
            format!("cannot assign {} to attribute '{}' of type {}", self.describe(mm), f.name, mm.classifier(ty).name)
        };
        if *self == Value::Null {
            return Ok(None);
        }
        if mm.is_enum(ty) {
            return match self {
                Value::Enum(e, l) if *e == ty => Ok(Some(ModelValue::Enum(*e, *l))),
                Value::Str(s) => mm
                    .enum_literals(ty)
                    .iter()
                    .position(|l| l.name == *s || l.literal == *s)
                    .map(|l| Some(ModelValue::Enum(ty, l)))
                    .ok_or_else(mismatch),
                _ => Err(mismatch()),
            };
        }
        let kind = mm.data_type_kind(ty).unwrap_or(DataTypeKind::Other);
        Ok(Some(match (kind, self) {
            (DataTypeKind::Boolean, Value::Bool(b)) => ModelValue::Boolean(*b),
            (DataTypeKind::Integer, Value::Int(i)) => ModelValue::Integer(*i),
            (DataTypeKind::Integer, Value::Real(r)) if r.fract() == 0.0 && r.abs() < 9.0e15 => {
                ModelValue::Integer(*r as i64)
            }
            (DataTypeKind::Real, Value::Int(i)) => ModelValue::Real(*i as f64),
            (DataTypeKind::Real, Value::Real(r)) => ModelValue::Real(*r),
            (DataTypeKind::String, v) | (DataTypeKind::Other, v) if !matches!(v, Value::Node(_)) => {
                ModelValue::String(v.to_text(mm))
            }
            _ => return Err(mismatch()),
        }))
    }

    pub fn is_truthy(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The text of a value when concatenated to a string.
    pub fn to_text(&self, mm: &Metamodel) -> String {
        match self {
            Value::Null => "null".into(),
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Real(r) => format_real(*r),
            Value::Str(s) => s.clone(),
            Value::Enum(e, l) => mm.enum_literals(*e).get(*l).map_or_else(|| l.to_string(), |l| l.name.clone()),
            Value::Node(n) => format!("#{}", n.0),
        }
    }

    /// The value as it would be written in a rule: strings quoted, nodes by label.
    pub fn render(&self, mm: &Metamodel, graph: &Graph) -> String {
        match self {
            Value::Str(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
            Value::Node(n) => graph.label(mm, *n),
            Value::Enum(e, l) => {
                format!("{}.{}", mm.classifier(*e).name, mm.enum_literals(*e).get(*l).map_or("?", |l| l.name.as_str()))
            }
            other => other.to_text(mm),
        }
    }

    fn describe(&self, mm: &Metamodel) -> String {
        match self {
            Value::Str(s) => format!("\"{s}\""),
            Value::Node(n) => format!("node #{}", n.0),
            other => other.to_text(mm),
        }
    }

    /// Equality as attribute matching uses it (numbers compare by value).
    pub fn equals(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Int(a), Value::Real(b)) | (Value::Real(b), Value::Int(a)) => (*a as f64) == *b,
            (a, b) => a == b,
        }
    }
}

pub(crate) fn format_real(r: f64) -> String {
    if r.fract() == 0.0 && r.abs() < 1e15 { format!("{r:.1}") } else { r.to_string() }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => f.write_str("null"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Real(r) => f.write_str(&format_real(*r)),
            Value::Str(s) => write!(f, "\"{s}\""),
            Value::Enum(e, l) => write!(f, "enum#{}:{l}", e.0),
            Value::Node(n) => write!(f, "#{}", n.0),
        }
    }
}

/// A resolved attribute expression.
#[derive(Clone, Debug, PartialEq)]
pub enum CExpr {
    /// A parameter, by index.
    Param(usize),
    Literal(Value),
    Not(Box<CExpr>),
    Neg(Box<CExpr>),
    Binary(BinOp, Box<CExpr>, Box<CExpr>),
    /// An expression the engine cannot evaluate: why, and the expression in Henshin's
    /// JavaScript (empty if it has none, e.g. after a syntax error), so that it can still be
    /// written to a `.henshin` file.
    Unsupported {
        reason: String,
        source: String,
    },
}

/// Why an expression has no value.
#[derive(Clone, Debug, PartialEq)]
pub enum EvalError {
    /// A parameter it uses has no value (yet).
    Unbound(usize),
    Failed(String),
}

impl CExpr {
    /// An expression the engine cannot evaluate.
    pub fn unsupported(reason: impl Into<String>, source: &crate::syntax::Expr) -> CExpr {
        CExpr::Unsupported { reason: reason.into(), source: crate::js::print(source) }
    }

    /// The parameter, if the expression is just a parameter.
    pub fn as_param(&self) -> Option<usize> {
        match self {
            CExpr::Param(p) => Some(*p),
            _ => None,
        }
    }

    pub fn params(&self, out: &mut Vec<usize>) {
        match self {
            CExpr::Param(p) => out.push(*p),
            CExpr::Not(e) | CExpr::Neg(e) => e.params(out),
            CExpr::Binary(_, l, r) => {
                l.params(out);
                r.params(out);
            }
            CExpr::Literal(_) | CExpr::Unsupported { .. } => {}
        }
    }

    pub fn eval(&self, mm: &Metamodel, bindings: &[Option<Value>]) -> Result<Value, EvalError> {
        match self {
            CExpr::Param(p) => bindings.get(*p).cloned().flatten().ok_or(EvalError::Unbound(*p)),
            CExpr::Literal(v) => Ok(v.clone()),
            CExpr::Unsupported { reason, .. } => Err(EvalError::Failed(reason.clone())),
            CExpr::Not(e) => match e.eval(mm, bindings)? {
                Value::Bool(b) => Ok(Value::Bool(!b)),
                v => Err(EvalError::Failed(format!("'!' needs a Boolean, not {}", v.describe(mm)))),
            },
            CExpr::Neg(e) => match e.eval(mm, bindings)? {
                Value::Int(i) => {
                    i.checked_neg().map(Value::Int).ok_or_else(|| EvalError::Failed("integer overflow".into()))
                }
                Value::Real(r) => Ok(Value::Real(-r)),
                v => Err(EvalError::Failed(format!("'-' needs a number, not {}", v.describe(mm)))),
            },
            CExpr::Binary(op, l, r) => {
                // `AND`/`OR` short-circuit, as in JavaScript.
                if matches!(op, BinOp::And | BinOp::Or) {
                    let left = l.eval(mm, bindings)?;
                    let Some(lb) = left.is_truthy() else {
                        return Err(EvalError::Failed(format!("'{}' needs Booleans", op.symbol())));
                    };
                    if (*op == BinOp::And && !lb) || (*op == BinOp::Or && lb) {
                        return Ok(Value::Bool(lb));
                    }
                    let right = r.eval(mm, bindings)?;
                    return right
                        .is_truthy()
                        .map(Value::Bool)
                        .ok_or_else(|| EvalError::Failed(format!("'{}' needs Booleans", op.symbol())));
                }
                let left = l.eval(mm, bindings)?;
                let right = r.eval(mm, bindings)?;
                binary(mm, *op, &left, &right).map_err(EvalError::Failed)
            }
        }
    }
}

fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Int(i) => Some(*i as f64),
        Value::Real(r) => Some(*r),
        _ => None,
    }
}

fn binary(mm: &Metamodel, op: BinOp, l: &Value, r: &Value) -> Result<Value, String> {
    let fail = || format!("'{}' cannot combine {} and {}", op.symbol(), l.describe(mm), r.describe(mm));
    Ok(match op {
        BinOp::Eq => Value::Bool(l.equals(r)),
        BinOp::Ne => Value::Bool(!l.equals(r)),
        BinOp::Add if matches!(l, Value::Str(_)) || matches!(r, Value::Str(_)) => {
            Value::Str(format!("{}{}", l.to_text(mm), r.to_text(mm)))
        }
        BinOp::Add | BinOp::Sub | BinOp::Mul => match (l, r) {
            (Value::Int(a), Value::Int(b)) => {
                let v = match op {
                    BinOp::Add => a.checked_add(*b),
                    BinOp::Sub => a.checked_sub(*b),
                    _ => a.checked_mul(*b),
                };
                v.map(Value::Int).ok_or_else(|| "integer overflow".to_string())?
            }
            _ => {
                let (a, b) = (number(l).ok_or_else(fail)?, number(r).ok_or_else(fail)?);
                Value::Real(match op {
                    BinOp::Add => a + b,
                    BinOp::Sub => a - b,
                    _ => a * b,
                })
            }
        },
        BinOp::Div => match (l, r) {
            (_, Value::Int(0)) => return Err("division by zero".into()),
            (Value::Int(a), Value::Int(b)) if a % b == 0 => Value::Int(a / b),
            _ => {
                let (a, b) = (number(l).ok_or_else(fail)?, number(r).ok_or_else(fail)?);
                if b == 0.0 {
                    return Err("division by zero".into());
                }
                Value::Real(a / b)
            }
        },
        BinOp::Ge | BinOp::Le | BinOp::Gt | BinOp::Lt => {
            let ordering = match (l, r) {
                (Value::Str(a), Value::Str(b)) => a.partial_cmp(b),
                _ => number(l).ok_or_else(fail)?.partial_cmp(&number(r).ok_or_else(fail)?),
            };
            let Some(o) = ordering else { return Ok(Value::Bool(false)) };
            Value::Bool(match op {
                BinOp::Ge => o.is_ge(),
                BinOp::Le => o.is_le(),
                BinOp::Gt => o.is_gt(),
                _ => o.is_lt(),
            })
        }
        BinOp::And | BinOp::Or => return Err(fail()),
    })
}
