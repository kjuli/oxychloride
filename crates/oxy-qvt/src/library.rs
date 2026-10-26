//! The parts of QVT's standard library that OCL's does not have: operations on strings
//! (`firstToUpper`, `replace`, ...), on model objects (`container`, `subobjectsOfKind`,
//! `deepclone`, ...) and on any value (`repr`). Operations on models (`rootObjects`, ...)
//! depend on a run and are in [`crate::operational`].

use std::collections::HashMap;

use oxy_ecore::{Metamodel, ModelValue, ObjectId};
use oxy_ocl::types::CollectionKind;
use oxy_ocl::{Evaluator, Type, Value};

/// `(name, signature, result)` of the string operations, for completion.
pub const STRING_OPERATIONS: &[(&str, &str, &str)] = &[
    ("firstToUpper", "firstToUpper()", "String"),
    ("lastToUpper", "lastToUpper()", "String"),
    ("toUpper", "toUpper()", "String"),
    ("toLower", "toLower()", "String"),
    ("length", "length()", "Integer"),
    ("substringBefore", "substringBefore(s)", "String"),
    ("substringAfter", "substringAfter(s)", "String"),
    ("replace", "replace(old, new)", "String"),
    ("match", "match(regex)", "Boolean"),
    ("asInteger", "asInteger()", "Integer"),
    ("asFloat", "asFloat()", "Real"),
    ("asBoolean", "asBoolean()", "Boolean"),
    ("quotify", "quotify(quote)", "String"),
    ("unquotify", "unquotify(quote)", "String"),
    ("isQuoted", "isQuoted(quote)", "Boolean"),
    ("normalizeSpace", "normalizeSpace()", "String"),
    ("format", "format(value)", "String"),
];

/// `(name, signature, result)` of the operations on model objects, for completion.
pub const ELEMENT_OPERATIONS: &[(&str, &str, &str)] = &[
    ("container", "container()", "OclAny"),
    ("subobjects", "subobjects()", "Set(OclAny)"),
    ("allSubobjects", "allSubobjects()", "Set(OclAny)"),
    ("subobjectsOfType", "subobjectsOfType(T)", "Set(T)"),
    ("subobjectsOfKind", "subobjectsOfKind(T)", "Set(T)"),
    ("allSubobjectsOfType", "allSubobjectsOfType(T)", "Set(T)"),
    ("allSubobjectsOfKind", "allSubobjectsOfKind(T)", "Set(T)"),
    ("deepclone", "deepclone()", "the object's type"),
    ("clone", "clone()", "the object's type"),
    ("metaClassName", "metaClassName()", "String"),
    ("repr", "repr()", "String"),
];

/// `(name, signature, result)` of the operations on models (model parameters), for
/// completion.
pub const MODEL_OPERATIONS: &[(&str, &str, &str)] = &[
    ("rootObjects", "rootObjects()", "Set(OclAny)"),
    ("objects", "objects()", "Set(OclAny)"),
    ("objectsOfType", "objectsOfType(T)", "Set(T)"),
    ("objectsOfKind", "objectsOfKind(T)", "Set(T)"),
    ("removeElement", "removeElement(object)", "OclVoid"),
];

fn arity(args: &[Type], n: usize, result: Type) -> Result<Type, String> {
    if args.len() == n { Ok(result) } else { Err(format!("expects {n} argument(s), but {} were given", args.len())) }
}

fn typed(mm: &Metamodel, args: &[Type], params: &[Type], result: Type) -> Result<Type, String> {
    arity(args, params.len(), Type::Error)?;
    for (i, (a, p)) in args.iter().zip(params).enumerate() {
        if !a.conforms_to(p, mm) {
            return Err(format!("argument {} has type {}, but {} is expected", i + 1, a.display(mm), p.display(mm)));
        }
    }
    Ok(result)
}

fn type_argument(args: &[Type], wrap: impl Fn(Type) -> Type) -> Result<Type, String> {
    match args {
        [Type::Meta(t)] => Ok(wrap((**t).clone())),
        [Type::Error] => Ok(Type::Error),
        [_] => Err("expects a type argument".into()),
        _ => Err(format!("expects 1 argument(s), but {} were given", args.len())),
    }
}

/// The result type of a library operation (`None`: there is no such operation).
pub fn check_operation(
    mm: &Metamodel,
    source: Option<&Type>,
    name: &str,
    args: &[Type],
) -> Option<Result<Type, String>> {
    let src = source?;
    let set = |t: Type| Type::collection(CollectionKind::Set, t);
    if name == "repr" {
        return Some(arity(args, 0, Type::String));
    }
    Some(match src {
        Type::Model(_) => match name {
            "rootObjects" | "objects" => arity(args, 0, set(Type::OclAny)),
            "objectsOfType" | "objectsOfKind" => type_argument(args, set),
            "removeElement" => arity(args, 1, Type::OclVoid),
            _ => return None,
        },
        Type::String => {
            let s = Type::String;
            match name {
                "firstToUpper" | "lastToUpper" | "toUpper" | "toLower" | "normalizeSpace" => arity(args, 0, s),
                "length" | "asInteger" => arity(args, 0, Type::Integer),
                "asFloat" => arity(args, 0, Type::Real),
                "asBoolean" => arity(args, 0, Type::Boolean),
                "substringBefore" | "substringAfter" | "quotify" | "unquotify" => typed(mm, args, &[Type::String], s),
                "isQuoted" | "match" => typed(mm, args, &[s], Type::Boolean),
                "replace" => typed(mm, args, &[s.clone(), s.clone()], s),
                "format" => {
                    if args.is_empty() {
                        Err("expects at least 1 argument(s)".into())
                    } else {
                        Ok(s)
                    }
                }
                _ => return None,
            }
        }
        Type::Classifier(c) if mm.is_class(*c) => element_operation(src, name, args)?,
        Type::OclAny => element_operation(src, name, args)?,
        Type::Collection(_, _) => match name {
            "add" => arity(args, 1, Type::OclVoid),
            _ => return None,
        },
        _ => return None,
    })
}

fn element_operation(src: &Type, name: &str, args: &[Type]) -> Option<Result<Type, String>> {
    let set = |t: Type| Type::collection(CollectionKind::Set, t);
    Some(match name {
        "container" => arity(args, 0, Type::OclAny),
        "subobjects" | "allSubobjects" => arity(args, 0, set(Type::OclAny)),
        "subobjectsOfType" | "subobjectsOfKind" | "allSubobjectsOfType" | "allSubobjectsOfKind" => {
            type_argument(args, set)
        }
        "deepclone" | "clone" => arity(args, 0, src.clone()),
        "metaClassName" => arity(args, 0, Type::String),
        // Old-style QVTo: the objects of a type the object contains (transitively).
        "allInstances" => type_argument(args, set),
        _ => return None,
    })
}

/// Evaluates a library operation (`None`: there is no such operation for these values).
pub fn call_operation(ev: &mut Evaluator<'_>, source: Option<&Value>, name: &str, args: &[Value]) -> Option<Value> {
    let src = source?;
    if name == "repr" && args.is_empty() {
        return Some(Value::string(src.render(ev.metamodel(), ev.model())));
    }
    match src {
        // OCL's `matches`: the whole string matches a regular expression.
        Value::String(_) if name == "match" => Some(ev.call(src.clone(), "matches", args)),
        Value::String(s) => string_operation(s, name, args),
        Value::Object(o) => element_call(ev, *o, name, args),
        _ => None,
    }
}

fn string_operation(s: &str, name: &str, args: &[Value]) -> Option<Value> {
    let upper_first = |s: &str| {
        let mut c = s.chars();
        match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => String::new(),
        }
    };
    Some(match (name, args) {
        ("firstToUpper", []) => Value::string(upper_first(s)),
        ("lastToUpper", []) => {
            let mut chars: Vec<char> = s.chars().collect();
            if let Some(last) = chars.pop() {
                let mut out: String = chars.into_iter().collect();
                out.extend(last.to_uppercase());
                Value::string(out)
            } else {
                Value::string("")
            }
        }
        ("toUpper", []) => Value::string(s.to_uppercase()),
        ("toLower", []) => Value::string(s.to_lowercase()),
        ("length", []) => Value::Integer(s.chars().count() as i64),
        ("normalizeSpace", []) => Value::string(s.split_whitespace().collect::<Vec<_>>().join(" ")),
        ("substringBefore", [Value::String(m)]) => {
            Value::string(s.find(&**m).map_or(s.to_string(), |i| s[..i].to_string()))
        }
        ("substringAfter", [Value::String(m)]) => {
            Value::string(s.find(&**m).map_or(String::new(), |i| s[i + m.len()..].to_string()))
        }
        ("replace", [Value::String(a), Value::String(b)]) => Value::string(s.replace(&**a, b)),
        ("asInteger", []) => s.trim().parse().map_or(Value::Null, Value::Integer),
        ("asFloat", []) => s.trim().parse().map_or(Value::Null, Value::Real),
        ("asBoolean", []) => match s.trim() {
            "true" => Value::Boolean(true),
            "false" => Value::Boolean(false),
            _ => Value::Null,
        },
        ("quotify", [Value::String(q)]) => Value::string(format!("{q}{s}{q}")),
        ("unquotify", [Value::String(q)]) => {
            Value::string(s.strip_prefix(&**q).and_then(|r| r.strip_suffix(&**q)).unwrap_or(s))
        }
        ("isQuoted", [Value::String(q)]) => {
            Value::Boolean(s.len() >= 2 * q.len() && s.starts_with(&**q) && s.ends_with(&**q))
        }
        ("format", values) => Value::string(format_string(s, values)),
        _ => return None,
    })
}

/// `'%s and %s'.format(Sequence{a, b})`: `%s` (and `%d`) replaced by the values in order.
fn format_string(s: &str, values: &[Value]) -> String {
    let items: Vec<Value> = match values {
        [Value::Collection(c)] => c.items.clone(),
        other => other.to_vec(),
    };
    let mut out = String::new();
    let mut next = items.iter();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' && matches!(chars.peek(), Some('s' | 'd' | 'f')) {
            chars.next();
            match next.next() {
                Some(Value::String(v)) => out.push_str(v),
                Some(v) => out.push_str(&v.to_string()),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn element_call(ev: &mut Evaluator<'_>, o: ObjectId, name: &str, args: &[Value]) -> Option<Value> {
    let mm = ev.metamodel();
    let set =
        |items: Vec<ObjectId>| Value::collection(CollectionKind::Set, items.into_iter().map(Value::Object).collect());
    let of = |ev: &Evaluator<'_>, items: Vec<ObjectId>, t: &Type, exact: bool| {
        Value::collection(
            CollectionKind::Set,
            items.into_iter().map(Value::Object).filter(|v| ev.conforms(v, t, exact)).collect(),
        )
    };
    Some(match (name, args) {
        ("container", []) => ev.model().container(o).map_or(Value::Null, Value::Object),
        ("subobjects", []) => set(ev.model().contents(mm, o)),
        ("allSubobjects", []) => set(ev.model().all_contents(mm, o)),
        ("subobjectsOfType" | "subobjectsOfKind", [Value::Type(t)]) => {
            let items = ev.model().contents(mm, o);
            of(ev, items, t, name == "subobjectsOfType")
        }
        ("allSubobjectsOfType" | "allSubobjectsOfKind", [Value::Type(t)]) => {
            let items = ev.model().all_contents(mm, o);
            of(ev, items, t, name == "allSubobjectsOfType")
        }
        ("metaClassName", []) => Value::string(&mm.classifier(ev.model().class_of(o)).name),
        ("allInstances", [Value::Type(t)]) => {
            let items = ev.model().all_contents(mm, o);
            of(ev, items, t, false)
        }
        ("deepclone" | "clone", []) => {
            let resource = ev.model().object(o).resource;
            let mut create = |ev: &mut Evaluator<'_>, class| match ev.model_mut() {
                Some(model) => Ok(model.create_object(mm, class, resource, None)),
                None => Err("the model cannot be changed here".to_string()),
            };
            match clone_object(ev, o, name == "deepclone", &mut create) {
                Ok(c) => Value::Object(c),
                Err(e) => Value::invalid(e),
            }
        }
        _ => return None,
    })
}

/// Creates the objects of a copy.
pub type Create<'c> = dyn FnMut(&mut Evaluator<'_>, oxy_ecore::ClassifierId) -> Result<ObjectId, String> + 'c;

/// Copies an object (`deep`: with its contents), as EMF's copier does: references between
/// copied objects point to the copies. The copies are made by `create`.
pub fn clone_object(
    ev: &mut Evaluator<'_>,
    o: ObjectId,
    deep: bool,
    create: &mut Create<'_>,
) -> Result<ObjectId, String> {
    let mm = ev.metamodel();
    let originals: Vec<ObjectId> =
        if deep { std::iter::once(o).chain(ev.model().all_contents(mm, o)).collect() } else { vec![o] };
    let mut copies: HashMap<ObjectId, ObjectId> = HashMap::new();
    for &orig in &originals {
        let class = ev.model().class_of(orig);
        copies.insert(orig, create(ev, class)?);
    }
    let Some(model) = ev.model_mut() else { return Err("the model cannot be changed here".into()) };
    for &orig in &originals {
        let copy = copies[&orig];
        let features: Vec<_> = mm.all_features(model.class_of(orig)).to_vec();
        for f in features {
            let feature = mm.feature(f);
            if feature.derived || feature.opposite().is_some_and(|op| mm.feature(op).is_containment()) {
                continue;
            }
            let values: Vec<ModelValue> = model.get(orig, f).to_vec();
            for v in values {
                let v = match v {
                    ModelValue::Object(t) => match copies.get(&t) {
                        Some(c) => ModelValue::Object(*c),
                        None if feature.is_containment() => continue,
                        None => ModelValue::Object(t),
                    },
                    other => other,
                };
                // Links between copies are set from both sides; `insert` keeps one.
                model.insert(mm, copy, f, v);
            }
        }
    }
    let copy = copies[&o];
    model.make_root(copy);
    Ok(copy)
}
