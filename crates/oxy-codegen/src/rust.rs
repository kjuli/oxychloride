//! The Rust target: a crate without dependencies.
//!
//! ```text
//! <name>/Cargo.toml
//! <name>/src/lib.rs          Model (the object store), Class, metadata, XMI, validate
//! <name>/src/<package>.rs    enumerations and handle types of one EPackage, with their
//!                            features, operations, invariants and `def:`s
//! <name>/src/ocl.rs          runtime: the OCL library (runtime/rust/ocl.rs)
//! <name>/src/ecore.rs        runtime: links, XMI, multiplicities (runtime/rust/ecore.rs)
//! <name>/src/xml.rs          runtime: XML reader (runtime/rust/xml.rs)
//! ```
//!
//! Objects live in a `Model` and are referred to by copyable typed handles (`Book`, a
//! wrapper of an `ObjectId`), so references, opposites and containment need no `Rc`s. A
//! handle has methods for every feature of its class (inherited ones too): getters take
//! `&Model`, setters `&mut Model` and keep opposites and containment consistent through
//! the reflective `ecore::Store` implementation of `Model`. Storage is one `Data` variant per
//! concrete class, made of one part (struct) per class of its hierarchy that has stored
//! features.
//!
//! OCL is translated into functions returning `ocl::Result<T>`: `invalid` is
//! `Err(ocl::Error::Invalid)`, null is `None` (an `Option` where the type allows null),
//! collections are `Vec`s without nulls. The translation is typed: every expression is
//! printed as a value of the Rust type of its OCL type, or an `Option` of it ([`Code`]), and
//! converted where OCL's conformance is implicit (Integer to Real, a subclass handle to its
//! superclass handle, a value to an optional one).

use std::collections::HashMap;

use crate::model::*;
use crate::naming::{Names, pascal_case, snake_case};
use crate::python::class_order;
use crate::writer::Writer;
use crate::{Options, Output, Target};

pub const RUNTIME_OCL: &str = include_str!("../runtime/rust/ocl.rs");
pub const RUNTIME_ECORE: &str = include_str!("../runtime/rust/ecore.rs");
pub const RUNTIME_XML: &str = include_str!("../runtime/rust/xml.rs");

pub struct Rust;

impl Target for Rust {
    fn id(&self) -> &'static str {
        "rust"
    }

    fn name(&self) -> &'static str {
        "Rust"
    }

    fn description(&self) -> &'static str {
        "A Rust crate (no dependencies): typed object handles in a model store, consistent references, \
         XMI loading and saving, OCL invariants, operations and contracts as native code"
    }

    fn generate(&self, model: &CodeModel, options: &Options, output: &mut Output) {
        let names = RsNames::new(model, options);
        let dir = names.krate.clone();
        output.file(format!("{dir}/Cargo.toml"), cargo_toml(model, &names));
        output.file(format!("{dir}/src/ocl.rs"), RUNTIME_OCL);
        output.file(format!("{dir}/src/ecore.rs"), RUNTIME_ECORE);
        output.file(format!("{dir}/src/xml.rs"), RUNTIME_XML);
        let mut generator = Gen { model, names: &names, output };
        let lib = generator.lib_rs();
        for i in 0..model.packages.len() {
            let text = generator.package_module(PackageId(i));
            generator.output.file(format!("{dir}/src/{}.rs", names.modules[i]), text);
        }
        output.file(format!("{dir}/src/lib.rs"), lib);
    }
}

const KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in",
    "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super",
    "trait", "true", "type", "unsafe", "use", "where", "while", "async", "await", "dyn", "abstract", "become", "box",
    "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try", "gen",
];

/// Names generated code uses for its own purposes.
const RESERVED: &[&str] = &["m", "this", "result", "ocl", "ecore", "xml", "value", "values", "std", "core"];

/// Type names that must not be shadowed in the generated modules.
const RESERVED_TYPES: &[&str] = &[
    "String",
    "Vec",
    "Option",
    "Result",
    "Box",
    "Some",
    "None",
    "Ok",
    "Err",
    "Default",
    "Clone",
    "Copy",
    "Debug",
    "PartialEq",
    "Eq",
    "Hash",
    "From",
    "Into",
    "Iterator",
    "Self",
    "Model",
    "Class",
    "ObjectId",
    "Data",
];

/// A valid Rust identifier for a value or function.
pub fn ident(name: &str) -> String {
    let mut s = snake_case(name);
    if KEYWORDS.contains(&s.as_str()) || RESERVED.contains(&s.as_str()) {
        s.push('_');
    }
    s
}

/// A valid Rust identifier for a type: upper case first (lower-case type names would clash
/// with variables of the generated code).
pub fn type_ident(name: &str) -> String {
    let mut s: String = name.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if s.is_empty() || s.starts_with(|c: char| c.is_ascii_digit() || c == '_') {
        s.insert(0, 'X');
    }
    if let Some(first) = s.chars().next().filter(|c| c.is_lowercase()) {
        s = first.to_uppercase().chain(s.chars().skip(1)).collect();
    }
    if KEYWORDS.contains(&s.as_str()) || RESERVED_TYPES.contains(&s.as_str()) {
        s.push('_');
    }
    s
}

/// A Rust string literal.
pub fn string_literal(s: &str) -> String {
    format!("{s:?}")
}

/// The names of everything generated.
struct RsNames {
    krate: String,
    modules: Vec<String>,
    classes: Vec<String>,
    enums: Vec<String>,
    /// Literal variants, by enumeration.
    literals: Vec<Vec<String>>,
    /// Variants of `crate::Class` (and of `Data`), by class.
    class_variants: Vec<String>,
    /// `create_<class>` methods of `Model`, by class.
    creators: Vec<String>,
    /// Getters, by feature (setters are derived from them).
    features: Vec<String>,
    operations: Vec<String>,
    def_attributes: HashMap<(ClassId, String), String>,
    def_operations: HashMap<(ClassId, String, usize), String>,
    invariants: HashMap<(ClassId, usize), String>,
    /// Indices into `FEATURES` of the features with metadata (stored ones and container
    /// references).
    feature_index: HashMap<FeatureId, usize>,
    meta_features: Vec<FeatureId>,
    /// Classes with a part (their own stored features), and the part's field name.
    parts: HashMap<ClassId, String>,
}

impl RsNames {
    fn new(model: &CodeModel, options: &Options) -> RsNames {
        let krate = snake_case(options.name.as_deref().unwrap_or(&model.name));
        let mut module_names = Names::new();
        module_names.reserve(["lib", "ocl", "ecore", "xml", "main", "crate", "self", "super"]);
        let modules: Vec<String> =
            model.packages.iter().map(|p| module_names.unique(&ident(&p.path.join("_")))).collect();
        let mut variant_names = Names::new();
        let simple: Vec<String> = model.classes.iter().map(|c| pascal_case(&c.name)).collect();
        let class_variants = model
            .classes
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let clash = simple.iter().filter(|s| **s == simple[i]).count() > 1;
                let base = if clash {
                    format!("{}{}", pascal_case(&model.package(c.package).name), simple[i])
                } else {
                    simple[i].clone()
                };
                variant_names.unique(&base)
            })
            .collect();
        let mut creator_names = Names::new();
        creator_names.reserve(["new", "default"]);
        let creators = model
            .classes
            .iter()
            .map(|c| {
                let clash = model.classes.iter().filter(|x| snake_case(&x.name) == snake_case(&c.name)).count() > 1;
                let base = if clash {
                    format!("create_{}_{}", modules[c.package.0], snake_case(&c.name))
                } else {
                    format!("create_{}", snake_case(&c.name))
                };
                creator_names.unique(&base)
            })
            .collect();
        let mut names = RsNames {
            krate,
            modules,
            classes: model.classes.iter().map(|c| type_ident(&c.name)).collect(),
            enums: model.enums.iter().map(|e| type_ident(&e.name)).collect(),
            literals: model
                .enums
                .iter()
                .map(|e| {
                    let mut n = Names::new();
                    n.reserve(KEYWORDS.iter().copied());
                    e.literals.iter().map(|l| n.unique(&pascal_case(&l.name))).collect()
                })
                .collect(),
            class_variants,
            creators,
            features: vec![String::new(); model.features.len()],
            operations: vec![String::new(); model.operations.len()],
            def_attributes: HashMap::new(),
            def_operations: HashMap::new(),
            invariants: HashMap::new(),
            feature_index: HashMap::new(),
            meta_features: Vec::new(),
            parts: HashMap::new(),
        };
        for f in model.feature_ids() {
            let feature = model.feature(f);
            if feature.is_stored() || feature.is_container() {
                names.feature_index.insert(f, names.meta_features.len());
                names.meta_features.push(f);
            }
        }
        // Members are named class by class, supertypes first; inherited names are kept.
        let mut scopes: HashMap<ClassId, Vec<String>> = HashMap::new();
        for c in class_order(model) {
            let class = model.class(c);
            let mut used = Names::new();
            used.reserve(["e_id", "from_e_id", "cast", "e_class", "e_container", "e_delete", "label"]);
            for s in &class.all_supers {
                if let Some(n) = scopes.get(s) {
                    used.reserve(n.iter().map(String::as_str));
                }
            }
            let mut own = Vec::new();
            for &f in &class.features {
                let n = used.unique(&ident(&model.feature(f).name));
                // Setters and list operations are derived from the getter's name.
                for derived in
                    [format!("set_{n}"), format!("add_to_{n}"), format!("remove_from_{n}"), format!("{n}_or_initial")]
                {
                    used.reserve([derived.as_str()]);
                    own.push(derived);
                }
                names.features[f.0] = n.clone();
                own.push(n);
            }
            for d in &class.def_attributes {
                let n = used.unique(&ident(&d.name));
                names.def_attributes.insert((c, d.name.clone()), n.clone());
                own.push(n);
            }
            for &o in &class.operations {
                let op = model.operation(o);
                let inherited = op.overrides.first().map(|x| names.operations[x.0].clone());
                let n = inherited.unwrap_or_else(|| used.unique(&ident(&op.name)));
                names.operations[o.0] = n.clone();
                own.push(n);
            }
            for d in &class.def_operations {
                let n = used.unique(&ident(&d.name));
                names.def_operations.insert((c, d.name.clone(), d.params.len()), n.clone());
                own.push(n);
            }
            for (i, inv) in class.invariants.iter().enumerate() {
                let n = used.unique(&format!("check_{}", snake_case(&inv.name)));
                names.invariants.insert((c, i), n.clone());
                own.push(n);
            }
            let mut all: Vec<String> =
                class.all_supers.iter().flat_map(|s| scopes.get(s).cloned().unwrap_or_default()).collect();
            all.extend(own);
            scopes.insert(c, all);
        }
        // Part names are fields of `Data` variants and names of its accessors: unique.
        let mut part_names = Names::new();
        part_names.reserve(["default_for"]);
        for c in model.class_ids() {
            if model.class(c).features.iter().any(|f| model.feature(*f).is_stored()) {
                names.parts.insert(c, part_names.unique(&ident(&model.class(c).name)));
            }
        }
        names
    }
}

fn cargo_toml(model: &CodeModel, names: &RsNames) -> String {
    format!(
        "# Generated by Oxychloride from {}. Do not edit: changes are lost when the code is\n\
         # generated again.\n\n[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\
         rust-version = \"1.70\"\n\n[dependencies]\n",
        if model.source.is_empty() { "a metamodel" } else { &model.source },
        names.krate.replace('_', "-")
    )
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn doc_lines(w: &mut Writer, prefix: &str, text: &str) {
    for l in text.trim_end().lines() {
        if l.trim().is_empty() {
            w.line(prefix.trim_end());
        } else {
            w.line(format!("{prefix}{}", l.trim_end()));
        }
    }
}

/// Where code is written, for paths of types.
#[derive(Copy, Clone, PartialEq, Eq)]
enum Scope {
    /// `src/lib.rs`.
    Crate,
    /// The module of a package.
    Package(PackageId),
}

struct Gen<'a> {
    model: &'a CodeModel,
    names: &'a RsNames,
    output: &'a mut Output,
}

impl Gen<'_> {
    fn warn(&mut self, message: String, origin: Option<&str>) {
        self.output.warn(format!("Rust: {message}"), origin);
    }

    // ---------------------------------------------------------------- types

    fn class_path(&self, c: ClassId, scope: Scope) -> String {
        let p = self.model.class(c).package;
        if scope == Scope::Package(p) {
            self.names.classes[c.0].clone()
        } else {
            format!("crate::{}::{}", self.names.modules[p.0], self.names.classes[c.0])
        }
    }

    fn enum_path(&self, e: EnumId, scope: Scope) -> String {
        let p = self.model.enumeration(e).package;
        if scope == Scope::Package(p) {
            self.names.enums[e.0].clone()
        } else {
            format!("crate::{}::{}", self.names.modules[p.0], self.names.enums[e.0])
        }
    }

    fn part_path(&self, c: ClassId, scope: Scope) -> String {
        let p = self.model.class(c).package;
        if scope == Scope::Package(p) {
            format!("parts::{}", self.names.classes[c.0])
        } else {
            format!("crate::{}::parts::{}", self.names.modules[p.0], self.names.classes[c.0])
        }
    }

    /// The Rust type of a value of OCL type `ty`; `None` if it has none.
    fn rust_ty(&self, ty: &Ty, scope: Scope) -> Option<String> {
        Some(match ty {
            Ty::Boolean => "bool".into(),
            Ty::Integer | Ty::UnlimitedNatural => "i64".into(),
            Ty::Real => "f64".into(),
            Ty::String | Ty::DataType(_) => "String".into(),
            Ty::Enum(e) => self.enum_path(*e, scope),
            Ty::Class(c) => self.class_path(*c, scope),
            Ty::Object | Ty::Any => "crate::ObjectId".into(),
            Ty::Collection(_, e) => format!("Vec<{}>", self.rust_ty(e, scope).unwrap_or_else(|| "()".into())),
            // Parts in name order; values that are not collections may be null.
            Ty::Tuple(parts) => {
                let mut out = String::from("(");
                for (_, t) in parts {
                    let one = self.rust_ty(t, scope)?;
                    out.push_str(&if t.is_collection() { one } else { format!("Option<{one}>") });
                    out.push_str(", ");
                }
                out.pop();
                out.push(')');
                out
            }
            Ty::Void => "()".into(),
            _ => return None,
        })
    }

    /// The Rust type of a typed element (a `Vec` or `Option` of the value type).
    fn typed_ty(&self, t: &Typed, scope: Scope) -> String {
        let one = self.rust_ty(&t.ty, scope).unwrap_or_else(|| "()".into());
        if t.many {
            format!("Vec<{one}>")
        } else if t.nullable {
            format!("Option<{one}>")
        } else {
            one
        }
    }

    fn literal(&self, l: &Literal, scope: Scope) -> String {
        match l {
            Literal::Boolean(b) => b.to_string(),
            Literal::Integer(i) => i.to_string(),
            Literal::Real(r) => real_literal(*r),
            Literal::String(s) => format!("{}.to_string()", string_literal(s)),
            Literal::Enum(e, i) => format!("{}::{}", self.enum_path(*e, scope), self.names.literals[e.0][*i]),
        }
    }

    /// The serialized form of a default value, for the metadata.
    fn serialized(&self, l: &Literal) -> String {
        match l {
            Literal::Boolean(b) => b.to_string(),
            Literal::Integer(i) => i.to_string(),
            Literal::Real(r) => {
                if r.is_finite() && r.fract() == 0.0 && r.abs() < 1e16 {
                    format!("{r:.1}")
                } else {
                    format!("{r}")
                }
            }
            Literal::String(s) => s.clone(),
            Literal::Enum(e, i) => self.model.enumeration(*e).literals[*i].literal.clone(),
        }
    }

    // ---------------------------------------------------------------- lib.rs

    fn lib_rs(&mut self) -> String {
        let model = self.model;
        let names = self.names;
        let mut w = Writer::new();
        let title = model.packages.iter().map(|p| p.path.join("::")).collect::<Vec<_>>().join(", ");
        let example_class =
            model.root_classes().first().copied().or_else(|| model.class_ids().find(|c| model.class(*c).is_concrete()));
        let mut doc = format!(
            "The {title} metamodel: objects with typed handles, XMI loading and saving, and\n\
             OCL validation, generated by Oxychloride from {}.\n\n\
             Do not edit: changes are lost when the code is generated again.\n\n\
             ```no_run\nuse {}::Model;\n\n\
             let (model, _warnings) = Model::load_xmi(\"model.xmi\").unwrap();\n\
             for diagnostic in model.validate() {{\n    println!(\"{{diagnostic}}\");\n}}\n",
            if model.source.is_empty() { "a metamodel" } else { &model.source },
            names.krate,
        );
        if let Some(c) = example_class {
            doc.push_str(&format!(
                "\nlet mut model = Model::new();\nlet object = model.{}();\nprintln!(\"{{}}\", model.to_xmi());\n# let _ = object;\n",
                names.creators[c.0]
            ));
        }
        doc.push_str("```");
        doc_lines(&mut w, "//! ", &doc);
        w.blank();
        w.line("#![allow(dead_code, unused_parens, unused_braces, unused_variables, unused_mut, unused_imports)]");
        w.line("#![allow(unreachable_patterns, ambiguous_glob_reexports, non_camel_case_types, non_snake_case)]");
        w.line("#![allow(clippy::all)]");
        w.blank();
        w.line("pub mod ecore;");
        w.line("pub mod ocl;");
        w.line("mod xml;");
        w.blank();
        // The items of the packages are re-exported at the crate root; the modules are public
        // only when packages declare classifiers of the same name.
        let mut seen = std::collections::HashSet::new();
        let clash =
            model.classes.iter().map(|c| &c.name).chain(model.enums.iter().map(|e| &e.name)).any(|n| !seen.insert(n));
        for m in &names.modules {
            w.line(format!("{}mod {m};", if clash { "pub " } else { "" }));
        }
        w.blank();
        w.line("pub use ecore::{Diagnostic, DiagnosticKind};");
        w.line("pub use ocl::{Error, Handle, ObjectId};");
        for m in &names.modules {
            w.line(format!("pub use {m}::*;"));
        }
        w.blank();
        self.metadata(&mut w);
        self.class_enum(&mut w);
        self.data_enum(&mut w);
        self.model_struct(&mut w);
        self.store_impl(&mut w);
        self.invariant_table(&mut w);
        w.finish()
    }

    fn metadata(&mut self, w: &mut Writer) {
        let model = self.model;
        let names = self.names;
        w.line("/// The packages, by index.");
        w.line("pub static PACKAGES: &[ecore::PackageInfo] = &[");
        w.indent();
        for p in &model.packages {
            w.line(format!(
                "ecore::PackageInfo {{ name: {}, ns_uri: {}, ns_prefix: {} }},",
                string_literal(&p.name),
                string_literal(&p.ns_uri),
                string_literal(&p.ns_prefix)
            ));
        }
        w.dedent();
        w.line("];");
        w.blank();
        w.line("/// The classes, by index (`Class::index`).");
        w.line("pub static CLASSES: &[ecore::ClassInfo] = &[");
        w.indent();
        for c in model.class_ids() {
            let class = model.class(c);
            let supers: Vec<String> = class.all_supers.iter().map(|s| s.0.to_string()).collect();
            let features: Vec<String> =
                class.all_features.iter().filter_map(|f| names.feature_index.get(f)).map(|i| i.to_string()).collect();
            w.line(format!(
                "ecore::ClassInfo {{ name: {}, package: {}, is_abstract: {}, supers: &[{}], features: &[{}] }},",
                string_literal(&class.name),
                class.package.0,
                !class.is_concrete(),
                supers.join(", "),
                features.join(", ")
            ));
        }
        w.dedent();
        w.line("];");
        w.blank();
        w.line("/// The stored features (and container references), by index.");
        w.line("pub static FEATURES: &[ecore::FeatureInfo] = &[");
        w.indent();
        for &f in &names.meta_features {
            let feature = model.feature(f);
            let t = &feature.typed;
            let kind = match &t.ty {
                Ty::Boolean => "ecore::ValueKind::Boolean".to_string(),
                Ty::Integer | Ty::UnlimitedNatural => "ecore::ValueKind::Integer".into(),
                Ty::Real => "ecore::ValueKind::Real".into(),
                Ty::String | Ty::DataType(_) => "ecore::ValueKind::String".into(),
                Ty::Enum(e) => format!("ecore::ValueKind::Enum({})", e.0),
                Ty::Class(c) => format!("ecore::ValueKind::Object(Some({}))", c.0),
                _ => "ecore::ValueKind::Object(None)".into(),
            };
            let opposite = feature
                .opposite()
                .and_then(|o| names.feature_index.get(&o))
                .map_or("None".to_string(), |i| format!("Some({i})"));
            let default = feature
                .default
                .as_ref()
                .filter(|_| !feature.is_reference())
                .map_or("None".to_string(), |d| format!("Some({})", string_literal(&self.serialized(d))));
            w.line(format!(
                "ecore::FeatureInfo {{ name: {}, owner: {}, kind: {kind}, many: {}, lower: {}, upper: {}, unique: {}, \
                 containment: {}, container: {}, opposite: {opposite}, serialized: {}, id: {}, default: {default} }},",
                string_literal(&feature.name),
                feature.owner.0,
                t.many,
                t.lower,
                t.upper,
                t.unique,
                feature.is_containment(),
                feature.is_container(),
                feature.is_serialized(),
                matches!(feature.kind, FeatureKind::Attribute { id: true }),
            ));
        }
        w.dedent();
        w.line("];");
        w.blank();
        w.line("/// The enumerations, by index.");
        w.line("pub static ENUMS: &[ecore::EnumInfo] = &[");
        w.indent();
        for e in &model.enums {
            let n: Vec<String> = e.literals.iter().map(|l| string_literal(&l.name)).collect();
            let l: Vec<String> = e.literals.iter().map(|l| string_literal(&l.literal)).collect();
            w.line(format!(
                "ecore::EnumInfo {{ name: {}, names: &[{}], literals: &[{}] }},",
                string_literal(&e.name),
                n.join(", "),
                l.join(", ")
            ));
        }
        w.dedent();
        w.line("];");
        w.blank();
    }

    fn class_enum(&mut self, w: &mut Writer) {
        let model = self.model;
        let names = self.names;
        w.line("/// The classes of the metamodel.");
        w.line("#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]");
        w.line("pub enum Class {");
        w.indent();
        for c in model.class_ids() {
            w.line(format!("{},", names.class_variants[c.0]));
        }
        w.dedent();
        w.line("}");
        w.blank();
        w.line("impl Class {");
        w.indent();
        w.line("/// Every class, in the order of `CLASSES`.");
        let all: Vec<String> = model.class_ids().map(|c| format!("Class::{}", names.class_variants[c.0])).collect();
        w.line(format!("pub const ALL: [Class; {}] = [{}];", all.len(), all.join(", ")));
        w.blank();
        w.line("/// The index into `CLASSES`.");
        w.line("pub fn index(self) -> usize {");
        w.indent();
        w.line("self as usize");
        w.dedent();
        w.line("}");
        w.blank();
        w.line("/// The class's name, e.g. `Book`.");
        w.line("pub fn name(self) -> &'static str {");
        w.indent();
        w.line("CLASSES[self.index()].name");
        w.dedent();
        w.line("}");
        w.blank();
        w.line("/// Whether this class is `other` or one of its subclasses.");
        w.line("pub fn conforms_to(self, other: Class) -> bool {");
        w.indent();
        w.line("ecore::conforms(CLASSES, self.index(), other.index())");
        w.dedent();
        w.line("}");
        w.dedent();
        w.line("}");
        w.blank();
    }

    /// The parts (classes with stored features) of a concrete class, farthest supertype
    /// first.
    fn parts_of(&self, c: ClassId) -> Vec<ClassId> {
        let class = self.model.class(c);
        class
            .all_supers
            .iter()
            .rev()
            .copied()
            .chain(std::iter::once(c))
            .filter(|x| self.names.parts.contains_key(x))
            .collect()
    }

    fn data_enum(&mut self, w: &mut Writer) {
        let model = self.model;
        let names = self.names;
        w.line("/// The stored values of an object: one variant per concrete class, one part per class of");
        w.line("/// its hierarchy that declares stored features.");
        w.line("#[derive(Clone, Debug)]");
        w.line("pub(crate) enum Data {");
        w.indent();
        let concrete: Vec<ClassId> = model.class_ids().filter(|c| model.class(*c).is_concrete()).collect();
        for &c in &concrete {
            let parts: Vec<String> = self
                .parts_of(c)
                .iter()
                .map(|p| format!("{}: {}", names.parts[p], self.part_path(*p, Scope::Crate)))
                .collect();
            if parts.is_empty() {
                w.line(format!("{},", names.class_variants[c.0]));
            } else {
                w.line(format!("{} {{ {} }},", names.class_variants[c.0], parts.join(", ")));
            }
        }
        w.dedent();
        w.line("}");
        w.blank();
        w.line("impl Data {");
        w.indent();
        w.line("/// The values of a new object of a concrete class.");
        w.line("fn default_for(class: Class) -> Data {");
        w.indent();
        w.line("match class {");
        w.indent();
        for &c in &concrete {
            let parts: Vec<String> =
                self.parts_of(c).iter().map(|p| format!("{}: Default::default()", names.parts[p])).collect();
            let v = &names.class_variants[c.0];
            if parts.is_empty() {
                w.line(format!("Class::{v} => Data::{v},"));
            } else {
                w.line(format!("Class::{v} => Data::{v} {{ {} }},", parts.join(", ")));
            }
        }
        if concrete.len() < model.classes.len() {
            w.line("other => panic!(\"{} cannot be instantiated\", other.name()),");
        }
        w.dedent();
        w.line("}");
        w.dedent();
        w.line("}");
        let mut parts: Vec<ClassId> = names.parts.keys().copied().collect();
        parts.sort();
        for p in parts {
            let field = &names.parts[&p];
            let holders: Vec<String> = concrete
                .iter()
                .filter(|c| model.conforms(**c, p))
                .map(|c| format!("Data::{} {{ {field}, .. }}", names.class_variants[c.0]))
                .collect();
            let ty = self.part_path(p, Scope::Crate);
            for (suffix, reference) in [("", "&"), ("_mut", "&mut ")] {
                w.blank();
                w.line(format!("pub(crate) fn {field}{suffix}({reference}self) -> {reference}{ty} {{"));
                w.indent();
                w.line("match self {");
                w.indent();
                if holders.is_empty() {
                    w.line(format!("_ => unreachable!(\"no class has the part {field}\"),"));
                } else {
                    w.line(format!("{} => {field},", holders.join(" | ")));
                    if holders.len() < concrete.len() {
                        w.line(format!("_ => panic!(\"the object is not a {}\"),", model.class(p).name));
                    }
                }
                w.dedent();
                w.line("}");
                w.dedent();
                w.line("}");
            }
        }
        w.dedent();
        w.line("}");
        w.blank();
    }

    fn model_struct(&mut self, w: &mut Writer) {
        let model = self.model;
        let names = self.names;
        w.lines(
            r#"#[derive(Clone, Debug)]
struct Entry {
    class: Class,
    data: Data,
    container: Option<(ObjectId, usize)>,
}

/// A model: the objects of the metamodel's classes. Objects are created with the
/// `create_*` methods and referred to by typed handles (`Copy`), whose methods read
/// (`&Model`) and change (`&mut Model`) the objects' features.
#[derive(Clone, Debug)]
pub struct Model {
    objects: Vec<Option<Entry>>,
    /// Whether operation calls check their pre- and postconditions (on by default): a
    /// failing condition makes the call fail with `Error::Contract`.
    pub check_contracts: bool,
}

impl Default for Model {
    fn default() -> Self {
        Model { objects: Vec::new(), check_contracts: true }
    }
}

impl Model {
    pub fn new() -> Model {
        Model::default()
    }

    fn entry(&self, o: ObjectId) -> &Entry {
        match self.objects.get(o.index()) {
            Some(Some(e)) => e,
            _ => panic!("object {o:?} does not exist (deleted?)"),
        }
    }

    pub(crate) fn data(&self, o: ObjectId) -> &Data {
        &self.entry(o).data
    }

    pub(crate) fn data_mut(&mut self, o: ObjectId) -> &mut Data {
        match self.objects.get_mut(o.index()) {
            Some(Some(e)) => &mut e.data,
            _ => panic!("object {o:?} does not exist (deleted?)"),
        }
    }

    fn insert(&mut self, class: Class) -> ObjectId {
        let id = ObjectId(self.objects.len() as u32);
        self.objects.push(Some(Entry { class, data: Data::default_for(class), container: None }));
        id
    }

    /// The class of an object.
    pub fn class_of(&self, o: impl Handle) -> Class {
        self.entry(o.e_id()).class
    }

    /// Whether the object exists (it has not been deleted).
    pub fn contains(&self, o: impl Handle) -> bool {
        matches!(self.objects.get(o.e_id().index()), Some(Some(_)))
    }

    /// All objects, in creation order.
    pub fn objects(&self) -> Vec<ObjectId> {
        ecore::Store::object_ids(self)
    }

    /// The objects without a container, in creation order (the roots of an XMI document).
    pub fn roots(&self) -> Vec<ObjectId> {
        ecore::roots(self)
    }

    /// The object containing `o`, if any.
    pub fn container_of(&self, o: impl Handle) -> Option<ObjectId> {
        self.entry(o.e_id()).container.map(|(c, _)| c)
    }

    /// The objects `o` contains directly.
    pub fn contents(&self, o: impl Handle) -> Vec<ObjectId> {
        ecore::contents(self, o.e_id())
    }

    /// The objects `o` contains, transitively (depth first).
    pub fn all_contents(&self, o: impl Handle) -> Vec<ObjectId> {
        ecore::all_contents(self, o.e_id())
    }

    /// Deletes an object and its contents, and removes them from all references.
    pub fn delete(&mut self, o: impl Handle) {
        ecore::delete(self, o.e_id())
    }

    /// A label such as `Book 'Dune'`.
    pub fn label(&self, o: impl Handle) -> String {
        ecore::label(self, o.e_id())
    }

    /// EMF's URI fragment of an object, e.g. `//@books.0`.
    pub fn uri_fragment(&self, o: impl Handle) -> String {
        ecore::uri_fragment(self, o.e_id())
    }

    /// Whether `o` is an instance of the class `T` stands for (or of a subclass).
    pub fn is<T: Handle>(&self, o: impl Handle) -> bool {
        ecore::conforms(CLASSES, self.class_of(o).index(), T::CLASS)
    }

    /// `o` as a `T`, if it is one.
    pub fn cast<T: Handle>(&self, o: impl Handle) -> Option<T> {
        self.is::<T>(o).then(|| T::from_e_id(o.e_id()))
    }

    /// `o` as a `T`, or invalid (OCL's `oclAsType`).
    pub fn cast_or_invalid<T: Handle>(&self, o: impl Handle) -> ocl::Result<T> {
        match self.cast(o) {
            Some(t) => Ok(t),
            None => ocl::invalid(format!("cannot cast {} to {}", self.label(o), CLASSES[T::CLASS].name)),
        }
    }

    /// Every instance of the class `T` stands for (and of its subclasses).
    pub fn all_instances<T: Handle>(&self) -> Vec<T> {
        self.objects().into_iter().filter(|o| self.is::<T>(*o)).map(T::from_e_id).collect()
    }

    /// The elements that are instances of `T` (OCL's `selectByKind`).
    pub fn select_by_kind<T: Handle, U: Handle>(&self, items: Vec<U>) -> Vec<T> {
        items.into_iter().filter(|o| self.is::<T>(*o)).map(|o| T::from_e_id(o.e_id())).collect()
    }

    /// The elements whose class is exactly `T`'s (OCL's `selectByType`).
    pub fn select_by_type<T: Handle, U: Handle>(&self, items: Vec<U>) -> Vec<T> {
        items.into_iter().filter(|o| self.class_of(*o).index() == T::CLASS).map(|o| T::from_e_id(o.e_id())).collect()
    }

    /// Reads an XMI document. The warnings are problems that did not prevent loading
    /// (unknown features, unresolved references).
    pub fn from_xmi(text: &str) -> Result<(Model, Vec<String>), String> {
        ecore::read_xmi(text)
    }

    /// Reads an XMI file.
    pub fn load_xmi(path: impl AsRef<std::path::Path>) -> Result<(Model, Vec<String>), String> {
        let text = std::fs::read_to_string(path.as_ref()).map_err(|e| format!("{}: {e}", path.as_ref().display()))?;
        Model::from_xmi(&text)
    }

    /// The model as an XMI document (its roots, see `roots`).
    pub fn to_xmi(&self) -> String {
        ecore::write_xmi(self)
    }

    /// Writes the model as an XMI file.
    pub fn save_xmi(&self, path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
        std::fs::write(path, self.to_xmi())
    }

    /// Checks every object: the bounds of its features, and the OCL invariants of its class
    /// and its supertypes.
    pub fn validate(&self) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        for o in self.objects() {
            self.validate_object(o, &mut out);
        }
        out
    }

    /// Checks one object (see `validate`).
    pub fn validate_object(&self, o: impl Handle, out: &mut Vec<Diagnostic>) {
        let o = o.e_id();
        ecore::check_multiplicities(self, o, out);
        let class = self.class_of(o).index();
        for inv in INVARIANTS {
            if !ecore::conforms(CLASSES, class, inv.class) {
                continue;
            }
            let (violated, message) = match (inv.check)(self, o) {
                Ok(true) => continue,
                Ok(false) => (true, inv.message.and_then(|m| m(self, o).ok())),
                Err(Error::Invalid(m)) => (false, Some(format!("evaluated to invalid: {m}"))),
                Err(e) => (false, Some(e.to_string())),
            };
            out.push(Diagnostic {
                object: o,
                label: self.label(o),
                constraint: inv.name.to_string(),
                kind: DiagnosticKind::Invariant,
                violated,
                message,
            });
        }
    }
"#,
        );
        w.indent();
        for c in model.class_ids().filter(|c| model.class(*c).is_concrete()) {
            let class = model.class(c);
            w.blank();
            w.line(format!("/// Creates a {} (without a container).", class.name));
            w.line(format!("pub fn {}(&mut self) -> {} {{", names.creators[c.0], self.class_path(c, Scope::Crate)));
            w.indent();
            w.line(format!("{}(self.insert(Class::{}))", self.class_path(c, Scope::Crate), names.class_variants[c.0]));
            w.dedent();
            w.line("}");
        }
        w.dedent();
        w.line("}");
        w.blank();
    }

    fn store_impl(&mut self, w: &mut Writer) {
        let model = self.model;
        let names = self.names;
        w.lines(
            r#"impl ecore::Store for Model {
    fn packages(&self) -> &'static [ecore::PackageInfo] {
        PACKAGES
    }

    fn classes(&self) -> &'static [ecore::ClassInfo] {
        CLASSES
    }

    fn features(&self) -> &'static [ecore::FeatureInfo] {
        FEATURES
    }

    fn enums(&self) -> &'static [ecore::EnumInfo] {
        ENUMS
    }

    fn create_object(&mut self, class: usize) -> ObjectId {
        self.insert(Class::ALL[class])
    }

    fn remove_object(&mut self, o: ObjectId) {
        if let Some(slot) = self.objects.get_mut(o.index()) {
            *slot = None;
        }
    }

    fn object_ids(&self) -> Vec<ObjectId> {
        (0..self.objects.len()).filter(|i| self.objects[*i].is_some()).map(|i| ObjectId(i as u32)).collect()
    }

    fn contains_object(&self, o: ObjectId) -> bool {
        self.contains(o)
    }

    fn class_index(&self, o: ObjectId) -> usize {
        self.entry(o).class.index()
    }

    fn container(&self, o: ObjectId) -> Option<(ObjectId, usize)> {
        self.entry(o).container
    }

    fn set_container(&mut self, o: ObjectId, container: Option<(ObjectId, usize)>) {
        if let Some(Some(e)) = self.objects.get_mut(o.index()) {
            e.container = container;
        }
    }
"#,
        );
        w.indent();
        // References.
        let references: Vec<FeatureId> = names
            .meta_features
            .iter()
            .copied()
            .filter(|f| model.feature(*f).is_reference() && model.feature(*f).is_stored())
            .collect();
        let attributes: Vec<FeatureId> =
            names.meta_features.iter().copied().filter(|f| !model.feature(*f).is_reference()).collect();
        w.blank();
        w.line("fn refs(&self, o: ObjectId, feature: usize) -> Vec<ObjectId> {");
        w.indent();
        w.line("match feature {");
        w.indent();
        for &f in &references {
            // A `Vec` or an `Option` of handles: both iterate.
            let (field, part) = self.field_access(f);
            w.line(format!(
                "{} => self.data(o).{part}().{field}.iter().map(|h| h.e_id()).collect(),",
                names.feature_index[&f]
            ));
        }
        w.line("_ => Vec::new(),");
        w.dedent();
        w.line("}");
        w.dedent();
        w.line("}");
        w.blank();
        w.line("fn basic_add(&mut self, o: ObjectId, feature: usize, target: ObjectId, index: Option<usize>) {");
        w.indent();
        w.line("match feature {");
        w.indent();
        for &f in &references {
            let (field, part) = self.field_access(f);
            let feature = model.feature(f);
            let handle = self.handle_of(&feature.typed.ty, Scope::Crate);
            if feature.typed.many {
                w.line(format!(
                    "{} => ecore::insert(&mut self.data_mut(o).{part}_mut().{field}, {handle}, index, {}),",
                    names.feature_index[&f], feature.typed.unique
                ));
            } else {
                w.line(format!(
                    "{} => self.data_mut(o).{part}_mut().{field} = Some({handle}),",
                    names.feature_index[&f]
                ));
            }
        }
        w.line("_ => {}");
        w.dedent();
        w.line("}");
        w.dedent();
        w.line("}");
        w.blank();
        w.line("fn basic_remove(&mut self, o: ObjectId, feature: usize, target: ObjectId) {");
        w.indent();
        w.line("match feature {");
        w.indent();
        for &f in &references {
            let (field, part) = self.field_access(f);
            let feature = model.feature(f);
            if feature.typed.many {
                w.line(format!(
                    "{} => self.data_mut(o).{part}_mut().{field}.retain(|h| h.e_id() != target),",
                    names.feature_index[&f]
                ));
            } else {
                w.line(format!("{} => {{", names.feature_index[&f]));
                w.indent();
                w.line(format!("let value = &mut self.data_mut(o).{part}_mut().{field};"));
                w.line("if value.map(|h| h.e_id()) == Some(target) {");
                w.indent();
                w.line("*value = None;");
                w.dedent();
                w.line("}");
                w.dedent();
                w.line("}");
            }
        }
        w.line("_ => {}");
        w.dedent();
        w.line("}");
        w.dedent();
        w.line("}");
        w.blank();
        w.line("fn attribute(&self, o: ObjectId, feature: usize) -> Vec<ecore::Value> {");
        w.indent();
        w.line("match feature {");
        w.indent();
        for &f in &attributes {
            let (field, part) = self.field_access(f);
            let feature = model.feature(f);
            let to_value = |v: &str| -> String {
                match &feature.typed.ty {
                    Ty::Boolean => format!("ecore::Value::Boolean(*{v})"),
                    Ty::Integer | Ty::UnlimitedNatural => format!("ecore::Value::Integer(*{v})"),
                    Ty::Real => format!("ecore::Value::Real(*{v})"),
                    Ty::Enum(_) => format!("ecore::Value::Enum({v}.index())"),
                    _ => format!("ecore::Value::String({v}.clone())"),
                }
            };
            let t = &feature.typed;
            let access = format!("self.data(o).{part}().{field}");
            let code = if t.many || t.nullable {
                format!("{access}.iter().map(|v| {}).collect()", to_value("v"))
            } else {
                format!("vec![{}]", to_value(&format!("(&{access})")))
            };
            w.line(format!("{} => {code},", names.feature_index[&f]));
        }
        w.line("_ => Vec::new(),");
        w.dedent();
        w.line("}");
        w.dedent();
        w.line("}");
        w.blank();
        w.line("fn set_attribute(&mut self, o: ObjectId, feature: usize, values: Vec<ecore::Value>) {");
        w.indent();
        w.line("match feature {");
        w.indent();
        for &f in &attributes {
            let (field, part) = self.field_access(f);
            let feature = model.feature(f);
            let from_value = match &feature.typed.ty {
                Ty::Boolean => "v.as_boolean()".to_string(),
                Ty::Integer | Ty::UnlimitedNatural => "v.as_integer()".into(),
                Ty::Real => "v.as_real()".into(),
                Ty::Enum(e) => format!("v.as_enum().and_then({}::from_index)", self.enum_path(*e, Scope::Crate)),
                _ => "v.as_string()".into(),
            };
            let t = &feature.typed;
            let target = format!("self.data_mut(o).{part}_mut().{field}");
            if t.many {
                w.line(format!(
                    "{} => {target} = values.iter().filter_map(|v| {from_value}).collect(),",
                    names.feature_index[&f]
                ));
            } else if t.nullable {
                w.line(format!("{} => {target} = values.first().and_then(|v| {from_value}),", names.feature_index[&f]));
            } else {
                w.line(format!("{} => {{", names.feature_index[&f]));
                w.indent();
                w.line(format!("if let Some(value) = values.first().and_then(|v| {from_value}) {{"));
                w.indent();
                w.line(format!("{target} = value;"));
                w.dedent();
                w.line("}");
                w.dedent();
                w.line("}");
            }
        }
        w.line("_ => {}");
        w.dedent();
        w.line("}");
        w.dedent();
        w.line("}");
        w.dedent();
        w.line("}");
        w.blank();
    }

    /// The field of a stored feature and the part holding it.
    fn field_access(&self, f: FeatureId) -> (String, String) {
        let feature = self.model.feature(f);
        (self.names.features[f.0].clone(), self.names.parts[&feature.owner].clone())
    }

    /// A handle of type `ty` for the object id in the variable `target`.
    fn handle_of(&self, ty: &Ty, scope: Scope) -> String {
        match ty {
            Ty::Class(c) => format!("{}(target)", self.class_path(*c, scope)),
            _ => "target".into(),
        }
    }

    fn invariant_table(&mut self, w: &mut Writer) {
        let model = self.model;
        let names = self.names;
        w.line("/// An OCL invariant of a class.");
        w.line("struct Invariant {");
        w.indent();
        w.line("class: usize,");
        w.line("name: &'static str,");
        w.line("check: fn(&Model, ObjectId) -> ocl::Result<bool>,");
        w.line("message: Option<fn(&Model, ObjectId) -> ocl::Result<String>>,");
        w.dedent();
        w.line("}");
        w.blank();
        w.line("static INVARIANTS: &[Invariant] = &[");
        w.indent();
        for c in model.class_ids() {
            let class = model.class(c);
            let path = self.class_path(c, Scope::Crate);
            for (i, inv) in class.invariants.iter().enumerate() {
                let n = &names.invariants[&(c, i)];
                let message = if inv.message.is_some() {
                    format!("Some(|m, o| {path}(o).{}(m))", message_method(n))
                } else {
                    "None".into()
                };
                w.line("Invariant {");
                w.indent();
                w.line(format!("class: {},", c.0));
                w.line(format!("name: {},", string_literal(&format!("{}::{}", class.name, inv.name))));
                w.line(format!("check: |m, o| {path}(o).{n}(m),"));
                w.line(format!("message: {message},"));
                w.dedent();
                w.line("},");
            }
        }
        w.dedent();
        w.line("];");
    }

    // ---------------------------------------------------------------- package modules

    fn package_module(&mut self, p: PackageId) -> String {
        let model = self.model;
        let pkg = model.package(p);
        let scope = Scope::Package(p);
        let mut w = Writer::new();
        let mut doc = format!("The {} package", pkg.path.join("::"));
        if !pkg.ns_uri.is_empty() {
            doc.push_str(&format!(" (`{}`)", pkg.ns_uri));
        }
        doc.push('.');
        if let Some(d) = &pkg.doc {
            doc.push_str("\n\n");
            doc.push_str(d);
        }
        doc_lines(&mut w, "//! ", &doc);
        w.blank();
        w.line("use crate::ocl::{self, Handle, OclString};");
        w.line("use crate::ObjectId;");
        w.blank();
        for &e in &pkg.enums {
            self.enumeration(&mut w, e);
        }
        let classes: Vec<ClassId> = class_order(model).into_iter().filter(|c| model.class(*c).package == p).collect();
        for &c in &classes {
            self.class(&mut w, c, scope);
        }
        // Parts.
        let parts: Vec<ClassId> = classes.iter().copied().filter(|c| self.names.parts.contains_key(c)).collect();
        if !parts.is_empty() {
            w.line("/// The stored features of the classes, one struct per class.");
            w.line("pub(crate) mod parts {");
            w.indent();
            for c in parts {
                self.part(&mut w, c, scope);
            }
            w.dedent();
            w.line("}");
        }
        w.finish()
    }

    fn enumeration(&mut self, w: &mut Writer, e: EnumId) {
        let en = self.model.enumeration(e);
        let name = &self.names.enums[e.0];
        let variants = &self.names.literals[e.0];
        doc_lines(w, "/// ", en.doc.as_deref().unwrap_or(&format!("The enumeration {}.", en.name)));
        w.line("#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]");
        w.line(format!("pub enum {name} {{"));
        w.indent();
        for (i, l) in en.literals.iter().enumerate() {
            if l.literal != l.name {
                w.line(format!("/// Serialized as `{}`.", l.literal));
            }
            if i == 0 {
                w.line("#[default]");
            }
            w.line(format!("{},", variants[i]));
        }
        w.dedent();
        w.line("}");
        w.blank();
        w.line(format!("impl {name} {{"));
        w.indent();
        let all: Vec<String> = variants.iter().map(|v| format!("{name}::{v}")).collect();
        w.line(format!("pub const ALL: [{name}; {}] = [{}];", all.len(), all.join(", ")));
        w.blank();
        w.line("/// The position of the literal (0 for the first).");
        w.line("pub fn index(self) -> usize {");
        w.indent();
        w.line("self as usize");
        w.dedent();
        w.line("}");
        w.blank();
        w.line("pub fn from_index(index: usize) -> Option<Self> {");
        w.indent();
        w.line("Self::ALL.get(index).copied()");
        w.dedent();
        w.line("}");
        w.blank();
        w.line("/// The literal's name in the metamodel.");
        w.line("pub fn name(self) -> &'static str {");
        w.indent();
        w.line("crate::ENUMS[".to_string() + &e.0.to_string() + "].names[self.index()]");
        w.dedent();
        w.line("}");
        w.blank();
        w.line("/// The serialized form of the literal.");
        w.line("pub fn literal(self) -> &'static str {");
        w.indent();
        w.line("crate::ENUMS[".to_string() + &e.0.to_string() + "].literals[self.index()]");
        w.dedent();
        w.line("}");
        w.blank();
        w.line("/// The literal's value (`EEnumLiteral.value`).");
        w.line("pub fn value(self) -> i64 {");
        w.indent();
        w.line("match self {");
        w.indent();
        for (i, l) in en.literals.iter().enumerate() {
            w.line(format!("{name}::{} => {},", variants[i], l.value));
        }
        w.dedent();
        w.line("}");
        w.dedent();
        w.line("}");
        w.blank();
        w.line("/// The literal with the given serialized form or name.");
        w.line("pub fn from_literal(text: &str) -> Option<Self> {");
        w.indent();
        w.line("Self::ALL.iter().copied().find(|l| l.literal() == text || l.name() == text)");
        w.dedent();
        w.line("}");
        w.dedent();
        w.line("}");
        w.blank();
        w.line(format!("impl std::fmt::Display for {name} {{"));
        w.indent();
        w.line("/// OCL's form, e.g. `Kind::LECTURE`.");
        w.line("fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {");
        w.indent();
        w.line(format!("write!(f, \"{}::{{}}\", self.name())", en.name));
        w.dedent();
        w.line("}");
        w.dedent();
        w.line("}");
        w.blank();
        w.line(format!("impl OclString for {name} {{"));
        w.indent();
        w.line("fn ocl_string(&self) -> String {");
        w.indent();
        w.line("self.to_string()");
        w.dedent();
        w.line("}");
        w.dedent();
        w.line("}");
        w.blank();
    }

    fn part(&mut self, w: &mut Writer, c: ClassId, _scope: Scope) {
        // Part structs are named like the handles: inside `parts`, types need full paths.
        let scope = Scope::Crate;
        let model = self.model;
        let class = model.class(c);
        let name = &self.names.classes[c.0];
        let stored: Vec<FeatureId> = class.features.iter().copied().filter(|f| model.feature(*f).is_stored()).collect();
        w.blank();
        w.line(format!("/// The stored features declared by {}.", class.name));
        w.line("#[derive(Clone, Debug, PartialEq)]");
        w.line(format!("pub struct {name} {{"));
        w.indent();
        for &f in &stored {
            let feature = model.feature(f);
            w.line(format!("pub {}: {},", self.names.features[f.0], self.typed_ty(&feature.typed, scope)));
        }
        w.dedent();
        w.line("}");
        w.blank();
        w.line(format!("impl Default for {name} {{"));
        w.indent();
        w.line("fn default() -> Self {");
        w.indent();
        w.line(format!("{name} {{"));
        w.indent();
        for &f in &stored {
            let feature = model.feature(f);
            let t = &feature.typed;
            let value = if t.many {
                "Vec::new()".to_string()
            } else {
                match (&feature.default, t.nullable) {
                    (Some(d), true) => format!("Some({})", self.literal(d, scope)),
                    (Some(d), false) => self.literal(d, scope),
                    (None, true) => "None".into(),
                    (None, false) => "Default::default()".into(),
                }
            };
            w.line(format!("{}: {value},", self.names.features[f.0]));
        }
        w.dedent();
        w.line("}");
        w.dedent();
        w.line("}");
        w.dedent();
        w.line("}");
    }

    fn class(&mut self, w: &mut Writer, c: ClassId, scope: Scope) {
        let model = self.model;
        let names = self.names;
        let class = model.class(c);
        let name = &names.classes[c.0];
        let mut doc = class.doc.clone().unwrap_or_else(|| {
            format!(
                "{} {} (a handle: its features are read and changed through a `Model`).",
                if class.is_interface {
                    "The interface"
                } else if class.is_abstract {
                    "The abstract class"
                } else {
                    "The class"
                },
                class.name
            )
        });
        if !class.invariants.is_empty() {
            doc.push_str("\n\nInvariants:\n");
            for inv in &class.invariants {
                doc.push_str(&format!("\n* `{}` ({}): `{}`", inv.name, inv.body.origin, one_line(&inv.body.text)));
            }
        }
        doc_lines(w, "/// ", &doc);
        w.line("#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]");
        w.line(format!("pub struct {name}(pub(crate) ObjectId);"));
        w.blank();
        w.line(format!("impl Handle for {name} {{"));
        w.indent();
        w.line(format!("const CLASS: usize = {};", c.0));
        w.blank();
        w.line("fn e_id(self) -> ObjectId {");
        w.indent();
        w.line("self.0");
        w.dedent();
        w.line("}");
        w.blank();
        w.line("fn from_e_id(id: ObjectId) -> Self {");
        w.indent();
        w.line(format!("{name}(id)"));
        w.dedent();
        w.line("}");
        w.dedent();
        w.line("}");
        w.blank();
        w.line(format!("impl From<{name}> for ObjectId {{"));
        w.indent();
        w.line(format!("fn from(h: {name}) -> ObjectId {{"));
        w.indent();
        w.line("h.0");
        w.dedent();
        w.line("}");
        w.dedent();
        w.line("}");
        w.blank();
        for &s in &class.all_supers {
            let sp = self.class_path(s, scope);
            w.line(format!("impl From<{name}> for {sp} {{"));
            w.indent();
            w.line(format!("fn from(h: {name}) -> {sp} {{"));
            w.indent();
            w.line(format!("{sp}(h.0)"));
            w.dedent();
            w.line("}");
            w.dedent();
            w.line("}");
            w.blank();
        }
        w.line(format!("impl {name} {{"));
        w.indent();
        w.lines(format!(
            r#"/// The object's id.
pub fn e_id(self) -> ObjectId {{
    self.0
}}

/// `o` as a {cname}, if it is one.
pub fn cast(m: &crate::Model, o: impl Handle) -> Option<{name}> {{
    m.cast(o)
}}

/// The class of the object.
pub fn e_class(self, m: &crate::Model) -> crate::Class {{
    m.class_of(self)
}}

/// The object containing this one, if any.
pub fn e_container(self, m: &crate::Model) -> Option<ObjectId> {{
    m.container_of(self)
}}

/// A label such as `{cname} 'name'`.
pub fn label(self, m: &crate::Model) -> String {{
    m.label(self)
}}"#,
            cname = class.name
        ));
        let all_features = class.all_features.clone();
        for f in all_features {
            self.accessors(w, c, f, scope);
        }
        // `def:` attributes visible on the class.
        for s in std::iter::once(c).chain(class.all_supers.iter().copied()) {
            for d in &model.class(s).def_attributes {
                if s != c && model.def_attribute(c, &d.name).map(|(o, _)| o) != Some(s) {
                    continue;
                }
                let n = &names.def_attributes[&(s, d.name.clone())];
                w.blank();
                doc_lines(w, "/// ", &format!("Defined in OCL ({}): `{}`", d.body.origin, one_line(&d.body.text)));
                w.line(format!(
                    "pub fn {n}(self, m: &crate::Model) -> ocl::Result<{}> {{",
                    self.typed_ty(&d.typed, scope)
                ));
                w.indent();
                if s == c {
                    let body = self.function_body(&d.body, &d.typed, scope, &[], None);
                    w.lines(body);
                } else {
                    w.line(format!("{}(self.0).{n}(m)", self.class_path(s, scope)));
                }
                w.dedent();
                w.line("}");
            }
        }
        for o in model.all_operations(c) {
            self.operation(w, c, o, scope);
        }
        for s in std::iter::once(c).chain(class.all_supers.iter().copied()) {
            for d in &model.class(s).def_operations {
                if model.def_operation(c, &d.name, d.params.len()).map(|(o, _)| o) != Some(s) {
                    continue;
                }
                let n = &names.def_operations[&(s, d.name.clone(), d.params.len())];
                let params: Vec<(String, &Typed)> = d.params.iter().map(|p| (ident(&p.name), &p.typed)).collect();
                let signature: String =
                    params.iter().map(|(p, t)| format!(", {p}: {}", self.typed_ty(t, scope))).collect();
                w.blank();
                doc_lines(w, "/// ", &format!("Defined in OCL ({}): `{}`", d.body.origin, one_line(&d.body.text)));
                w.line(format!(
                    "pub fn {n}(self, m: &crate::Model{signature}) -> ocl::Result<{}> {{",
                    self.typed_ty(&d.typed, scope)
                ));
                w.indent();
                if s == c {
                    let bound: Vec<(String, String, Typed)> =
                        d.params.iter().map(|p| (p.name.clone(), ident(&p.name), p.typed.clone())).collect();
                    let body = self.function_body(&d.body, &d.typed, scope, &bound, None);
                    w.lines(body);
                } else {
                    let args: Vec<String> = params.iter().map(|(p, _)| p.clone()).collect();
                    let args = if args.is_empty() { String::new() } else { format!(", {}", args.join(", ")) };
                    w.line(format!("{}(self.0).{n}(m{args})", self.class_path(s, scope)));
                }
                w.dedent();
                w.line("}");
            }
        }
        for (i, inv) in class.invariants.iter().enumerate() {
            let n = names.invariants[&(c, i)].clone();
            w.blank();
            doc_lines(
                w,
                "/// ",
                &format!("Invariant `{}` ({}): `{}`", inv.name, inv.body.origin, one_line(&inv.body.text)),
            );
            w.line(format!("pub fn {n}(self, m: &crate::Model) -> ocl::Result<bool> {{"));
            w.indent();
            let body = self.function_body(&inv.body, &Typed::of_ty(&Ty::Boolean), scope, &[], None);
            w.lines(body);
            w.dedent();
            w.line("}");
            if let Some(message) = &inv.message {
                w.blank();
                w.line(format!("/// The message of the invariant `{}`.", inv.name));
                w.line(format!("pub fn {}(self, m: &crate::Model) -> ocl::Result<String> {{", message_method(&n)));
                w.indent();
                let mut printer = RsExpr::new(self, scope, &inv.body.origin, "self");
                let code = printer.ocl_string_code(message);
                w.lines(format!("Ok({code})"));
                w.dedent();
                w.line("}");
            }
        }
        w.dedent();
        w.line("}");
        w.blank();
    }

    /// The getter and setters of a feature on the handle of class `c`.
    fn accessors(&mut self, w: &mut Writer, c: ClassId, f: FeatureId, scope: Scope) {
        let model = self.model;
        let names = self.names;
        let feature = model.feature(f);
        let n = names.features[f.0].clone();
        let t = &feature.typed;
        let one = self.rust_ty(&t.ty, scope).unwrap_or_else(|| "()".into());
        let mut doc = feature.doc.clone().unwrap_or_else(|| {
            format!(
                "The {} `{}`{}.",
                if feature.is_reference() { "reference" } else { "attribute" },
                feature.name,
                if feature.owner != c { format!(" (of {})", model.class(feature.owner).name) } else { String::new() }
            )
        });
        if feature.is_containment() {
            doc.push_str(" Contained objects belong to this one.");
        }
        if let Some(o) = feature.opposite() {
            doc.push_str(&format!(" Its opposite is `{}`.", model.feature(o).name));
        }
        w.blank();
        if let Some(body) = &feature.derivation {
            doc_lines(w, "/// ", &format!("{doc}\n\nDerived ({}): `{}`", body.origin, one_line(&body.text)));
            w.line(format!("pub fn {n}(self, m: &crate::Model) -> ocl::Result<{}> {{", self.typed_ty(t, scope)));
            w.indent();
            if feature.owner == c {
                let body = self.function_body(body, t, scope, &[], None);
                w.lines(body);
            } else {
                w.line(format!("{}(self.0).{n}(m)", self.class_path(feature.owner, scope)));
            }
            w.dedent();
            w.line("}");
            return;
        }
        if !feature.is_stored() && !feature.is_container() {
            doc_lines(w, "/// ", &format!("{doc}\n\nVolatile without an OCL derivation: always invalid."));
            w.line(format!("pub fn {n}(self, m: &crate::Model) -> ocl::Result<{}> {{", self.typed_ty(t, scope)));
            w.indent();
            w.line(format!(
                "ocl::invalid({})",
                string_literal(&format!("{}::{} has no OCL derivation", model.class(feature.owner).name, feature.name))
            ));
            w.dedent();
            w.line("}");
            return;
        }
        let index = names.feature_index[&f];
        if feature.is_container() {
            let target = self.rust_ty(&t.ty, scope).unwrap_or_else(|| "ObjectId".into());
            doc_lines(w, "/// ", &doc);
            w.line(format!("pub fn {n}(self, m: &crate::Model) -> Option<{target}> {{"));
            w.indent();
            w.line(format!("crate::ecore::container_through(m, self.0, {index}).map(<{target}>::from_e_id)"));
            w.dedent();
            w.line("}");
            w.blank();
            w.line(format!(
                "/// Moves the object into the `{}` of `value` (or out of its container).",
                model.feature(feature.opposite().unwrap()).name
            ));
            w.line(format!("pub fn set_{n}(self, m: &mut crate::Model, value: Option<{target}>) {{"));
            w.indent();
            w.line(format!("crate::ecore::set_reference(m, self.0, {index}, value.map(|v| v.e_id()));"));
            w.dedent();
            w.line("}");
            return;
        }
        let part = names.parts[&feature.owner].clone();
        let field = format!("m.data(self.0).{part}().{n}");
        let field_mut = format!("m.data_mut(self.0).{part}_mut().{n}");
        let string_like = matches!(t.ty, Ty::String | Ty::DataType(_));
        if let (true, Some(body)) = (has_initial(feature), &feature.initial) {
            // OCL reads the feature through this: the initial value applies while it is unset.
            w.line(format!(
                "/// `{}`, or while it is unset its initial value ({}): `{}`",
                feature.name,
                body.origin,
                one_line(&body.text)
            ));
            w.line(format!("pub fn {n}_or_initial(self, m: &crate::Model) -> ocl::Result<Option<{one}>> {{"));
            w.indent();
            if feature.owner == c {
                let stored = if string_like { format!("{field}.clone()") } else { field.clone() };
                w.line(format!("if let Some(value) = {stored} {{"));
                w.indent();
                w.line("return Ok(Some(value));");
                w.dedent();
                w.line("}");
                let initial = Typed { nullable: true, ..t.clone() };
                let code = self.function_body(body, &initial, scope, &[], None);
                w.lines(code);
            } else {
                w.line(format!("{}(self.0).{n}_or_initial(m)", self.class_path(feature.owner, scope)));
            }
            w.dedent();
            w.line("}");
            w.blank();
        }
        doc_lines(w, "/// ", &doc);
        if t.many {
            w.line(format!("pub fn {n}(self, m: &crate::Model) -> &[{one}] {{"));
            w.indent();
            w.line(format!("&{field}"));
            w.dedent();
            w.line("}");
            if feature.is_reference() {
                w.blank();
                w.line(format!(
                    "/// Adds an object to `{}` (keeping containment and opposites consistent).",
                    feature.name
                ));
                w.line(format!("pub fn add_to_{n}(self, m: &mut crate::Model, value: impl Into<{one}>) {{"));
                w.indent();
                w.line(format!("crate::ecore::link(m, self.0, {index}, value.into().e_id(), None);"));
                w.dedent();
                w.line("}");
                w.blank();
                w.line(format!("/// Removes an object from `{}`.", feature.name));
                w.line(format!("pub fn remove_from_{n}(self, m: &mut crate::Model, value: impl Into<{one}>) {{"));
                w.indent();
                w.line(format!("crate::ecore::unlink(m, self.0, {index}, value.into().e_id());"));
                w.dedent();
                w.line("}");
                w.blank();
                w.line(format!("/// Replaces the objects of `{}`.", feature.name));
                w.line(format!("pub fn set_{n}(self, m: &mut crate::Model, values: Vec<{one}>) {{"));
                w.indent();
                w.line(format!(
                    "crate::ecore::set_references(m, self.0, {index}, values.into_iter().map(|v| v.e_id()).collect());"
                ));
                w.dedent();
                w.line("}");
            } else {
                w.blank();
                w.line(format!("/// Adds a value to `{}`.", feature.name));
                w.line(format!("pub fn add_to_{n}(self, m: &mut crate::Model, value: {one}) {{"));
                w.indent();
                w.line(format!("crate::ecore::insert(&mut {field_mut}, value, None, {});", t.unique));
                w.dedent();
                w.line("}");
                w.blank();
                w.line(format!("/// Removes a value from `{}`.", feature.name));
                w.line(format!("pub fn remove_from_{n}(self, m: &mut crate::Model, value: &{one}) {{"));
                w.indent();
                w.line(format!("{field_mut}.retain(|v| v != value);"));
                w.dedent();
                w.line("}");
                w.blank();
                w.line(format!("/// Replaces the values of `{}`.", feature.name));
                w.line(format!("pub fn set_{n}(self, m: &mut crate::Model, values: Vec<{one}>) {{"));
                w.indent();
                w.line(format!("{field_mut} = values;"));
                w.dedent();
                w.line("}");
            }
            return;
        }
        if feature.is_reference() {
            w.line(format!("pub fn {n}(self, m: &crate::Model) -> Option<{one}> {{"));
            w.indent();
            w.line(field);
            w.dedent();
            w.line("}");
            w.blank();
            w.line(format!("/// Sets `{}` (keeping containment and opposites consistent).", feature.name));
            w.line(format!("pub fn set_{n}(self, m: &mut crate::Model, value: Option<{one}>) {{"));
            w.indent();
            w.line(format!("crate::ecore::set_reference(m, self.0, {index}, value.map(|v| v.e_id()));"));
            w.dedent();
            w.line("}");
            return;
        }
        match (string_like, t.nullable) {
            (true, true) => {
                w.line(format!("pub fn {n}(self, m: &crate::Model) -> Option<&str> {{"));
                w.indent();
                w.line(format!("{field}.as_deref()"));
            }
            (true, false) => {
                w.line(format!("pub fn {n}(self, m: &crate::Model) -> &str {{"));
                w.indent();
                w.line(format!("&{field}"));
            }
            (false, true) => {
                w.line(format!("pub fn {n}(self, m: &crate::Model) -> Option<{one}> {{"));
                w.indent();
                w.line(&field);
            }
            (false, false) => {
                w.line(format!("pub fn {n}(self, m: &crate::Model) -> {one} {{"));
                w.indent();
                w.line(&field);
            }
        }
        w.dedent();
        w.line("}");
        if !feature.changeable {
            return;
        }
        w.blank();
        w.line(format!("/// Sets `{}`.", feature.name));
        match (string_like, t.nullable) {
            (true, true) => {
                w.line(format!("pub fn set_{n}(self, m: &mut crate::Model, value: Option<&str>) {{"));
                w.indent();
                w.line(format!("{field_mut} = value.map(str::to_string);"));
            }
            (true, false) => {
                w.line(format!("pub fn set_{n}(self, m: &mut crate::Model, value: impl Into<String>) {{"));
                w.indent();
                w.line(format!("{field_mut} = value.into();"));
            }
            (false, true) => {
                w.line(format!("pub fn set_{n}(self, m: &mut crate::Model, value: Option<{one}>) {{"));
                w.indent();
                w.line(format!("{field_mut} = value;"));
            }
            (false, false) => {
                w.line(format!("pub fn set_{n}(self, m: &mut crate::Model, value: {one}) {{"));
                w.indent();
                w.line(format!("{field_mut} = value;"));
            }
        }
        w.dedent();
        w.line("}");
    }

    /// An operation (own or inherited) on the handle of class `c`: dispatches on the
    /// object's class to the most specific body, checking the contracts.
    fn operation(&mut self, w: &mut Writer, c: ClassId, o: OperationId, scope: Scope) {
        let model = self.model;
        let names = self.names;
        let op = model.operation(o);
        let n = names.operations[o.0].clone();
        let params: Vec<(String, Typed)> = op.params.iter().map(|p| (ident(&p.name), p.typed.clone())).collect();
        let signature: String = params.iter().map(|(p, t)| format!(", {p}: {}", self.typed_ty(t, scope))).collect();
        let result_ty = op.result.as_ref().map_or("()".to_string(), |r| self.typed_ty(r, scope));
        let qualified = format!("{}::{}()", model.class(op.owner).name, op.name);
        let mut doc = op.doc.clone().unwrap_or_else(|| format!("The operation `{}`.", op.name));
        if let Some(body) = &op.body {
            doc.push_str(&format!("\n\nOCL body ({}): `{}`", body.origin, one_line(&body.text)));
        }
        let contract_ops = model.contract_operations(c, o);
        for x in &contract_ops {
            for p in &model.operation(*x).preconditions {
                doc.push_str(&format!(
                    "\n\nPrecondition `{}` ({}): `{}`",
                    p.name,
                    p.body.origin,
                    one_line(&p.body.text)
                ));
            }
            for p in &model.operation(*x).postconditions {
                doc.push_str(&format!(
                    "\n\nPostcondition `{}` ({}): `{}`",
                    p.name,
                    p.body.origin,
                    one_line(&p.body.text)
                ));
            }
        }
        if contract_ops.iter().any(|x| {
            let x = model.operation(*x);
            !x.preconditions.is_empty() || !x.postconditions.is_empty()
        }) {
            doc.push_str("\n\nThe conditions are checked when `m.check_contracts` is set (the default); a failing one makes the call fail with `Error::Contract`.");
        }
        w.blank();
        doc_lines(w, "/// ", &doc);
        w.line(format!("pub fn {n}(self, m: &crate::Model{signature}) -> ocl::Result<{result_ty}> {{"));
        w.indent();
        // Group the concrete classes by implementation and contracts.
        let mut groups: Vec<(Option<OperationId>, Vec<OperationId>, Vec<ClassId>)> = Vec::new();
        let concrete = model.concrete_subclasses(c);
        for k in &concrete {
            let implementation = model.implementation(*k, o);
            let contracts = model.contract_operations(*k, o);
            match groups.iter_mut().find(|(i, x, _)| *i == implementation && *x == contracts) {
                Some(g) => g.2.push(*k),
                None => groups.push((implementation, contracts, vec![*k])),
            }
        }
        if groups.is_empty() {
            // An abstract class without concrete subclasses: nothing to dispatch to.
            groups.push((model.implementation(c, o), model.contract_operations(c, o), Vec::new()));
        }
        let single = groups.len() == 1;
        if !single {
            w.line("match m.class_of(self) {");
            w.indent();
        }
        for (implementation, contracts, classes) in &groups {
            if !single {
                let pattern: Vec<String> =
                    classes.iter().map(|k| format!("crate::Class::{}", names.class_variants[k.0])).collect();
                w.line(format!("{} => {{", pattern.join(" | ")));
                w.indent();
            }
            let args: Vec<String> = params.iter().map(|(p, t)| clone_if_needed(p, t)).collect();
            let call_args = if args.is_empty() { String::new() } else { format!(", {}", args.join(", ")) };
            let checks = |w: &mut Writer, g: &mut Gen, post: bool| {
                for x in contracts {
                    let xop = model.operation(*x);
                    let conditions = if post { &xop.postconditions } else { &xop.preconditions };
                    for cond in conditions {
                        let fname = g.condition_fn_name(*x, post, &cond.name);
                        let owner = if xop.owner == c {
                            "self".to_string()
                        } else {
                            format!("{}(self.0)", g.class_path(xop.owner, scope))
                        };
                        let result_arg = match (&op.result, post) {
                            (Some(r), true) => format!(", {}", clone_if_needed("result", r)),
                            (None, true) => ", result".to_string(),
                            _ => String::new(),
                        };
                        let message = if cond.message.is_some() {
                            format!("|| {owner}.{fname}_message(m{call_args}{result_arg})")
                        } else {
                            "|| ocl::invalid(\"no message\")".to_string()
                        };
                        w.line("if m.check_contracts {");
                        w.indent();
                        w.line("ocl::check_condition(");
                        w.indent();
                        w.line(format!("{},", string_literal(if post { "postcondition" } else { "precondition" })));
                        w.line(format!(
                            "{},",
                            string_literal(&format!("{}::{}()", model.class(xop.owner).name, xop.name))
                        ));
                        w.line(format!("{},", string_literal(&cond.name)));
                        w.line("self.0,");
                        w.line("|| m.label(self),");
                        w.line(format!("{owner}.{fname}(m{call_args}{result_arg}),"));
                        w.line(format!("{message},"));
                        w.dedent();
                        w.line(")?;");
                        w.dedent();
                        w.line("}");
                    }
                }
            };
            checks(w, self, false);
            match implementation {
                Some(i) => {
                    let impl_owner = model.operation(*i).owner;
                    let receiver = if impl_owner == c {
                        "self".to_string()
                    } else {
                        format!("{}(self.0)", self.class_path(impl_owner, scope))
                    };
                    let fname = format!("{}_body", names.operations[i.0]);
                    w.line(format!("let result = {receiver}.{fname}(m{call_args})?;"));
                }
                None => {
                    w.line(format!(
                        "let result: {result_ty} = ocl::invalid({})?;",
                        string_literal(&format!("{qualified} has no OCL body"))
                    ));
                }
            }
            checks(w, self, true);
            w.line("Ok(result)");
            if !single {
                w.dedent();
                w.line("}");
            }
        }
        if !single {
            if concrete.len() < model.class_ids().count() {
                w.line(format!(
                    "other => ocl::invalid(format!(\"{} cannot be called on {{}}\", other.name())),",
                    qualified
                ));
            }
            w.dedent();
            w.line("}");
        }
        w.dedent();
        w.line("}");
        // The body and condition functions are generated with the declaring class.
        if op.owner != c {
            return;
        }
        let bound: Vec<(String, String, Typed)> =
            op.params.iter().map(|p| (p.name.clone(), ident(&p.name), p.typed.clone())).collect();
        let result_typed = op.result.clone().unwrap_or_else(|| Typed::of_ty(&Ty::Void));
        if let Some(body) = &op.body {
            w.blank();
            w.line(format!("/// The OCL body of `{}`, without contract checks.", op.name));
            w.line(format!("pub fn {n}_body(self, m: &crate::Model{signature}) -> ocl::Result<{result_ty}> {{"));
            w.indent();
            let code = self.function_body(body, &result_typed, scope, &bound, None);
            w.lines(code);
            w.dedent();
            w.line("}");
        }
        for (post, conditions) in [(false, &op.preconditions), (true, &op.postconditions)] {
            for cond in conditions {
                let fname = self.condition_fn_name(o, post, &cond.name);
                let extra = if post { format!(", result: {result_ty}") } else { String::new() };
                let result = if post { Some(&result_typed) } else { None };
                w.blank();
                w.line(format!(
                    "/// The {} `{}` of `{}`.",
                    if post { "postcondition" } else { "precondition" },
                    cond.name,
                    op.name
                ));
                w.line(format!("pub fn {fname}(self, m: &crate::Model{signature}{extra}) -> ocl::Result<bool> {{"));
                w.indent();
                let code = self.function_body(&cond.body, &Typed::of_ty(&Ty::Boolean), scope, &bound, result);
                w.lines(code);
                w.dedent();
                w.line("}");
                if let Some(message) = &cond.message {
                    w.blank();
                    w.line(format!(
                        "pub fn {fname}_message(self, m: &crate::Model{signature}{extra}) -> ocl::Result<String> {{"
                    ));
                    w.indent();
                    let mut printer = RsExpr::new(self, scope, &cond.body.origin, "self");
                    for (ocl_name, rust_name, t) in &bound {
                        printer.params.push((ocl_name.clone(), rust_name.clone(), t.clone()));
                    }
                    printer.result = result.cloned();
                    let code = printer.ocl_string_code(message);
                    w.lines(format!("Ok({code})"));
                    w.dedent();
                    w.line("}");
                }
            }
        }
    }

    fn condition_fn_name(&self, o: OperationId, post: bool, name: &str) -> String {
        format!("{}_{}_{}", self.names.operations[o.0], if post { "post" } else { "pre" }, snake_case(name))
    }

    /// The statements of a method (on `self`) returning `Ok(<body converted to typed>)`.
    fn function_body(
        &mut self,
        body: &Body,
        typed: &Typed,
        scope: Scope,
        params: &[(String, String, Typed)],
        result: Option<&Typed>,
    ) -> String {
        let mut printer = RsExpr::new(self, scope, &body.origin, "self");
        for (ocl_name, rust_name, t) in params {
            printer.params.push((ocl_name.clone(), rust_name.clone(), t.clone()));
        }
        printer.result = result.cloned();
        let code = if typed.ty == Ty::Void {
            let c = printer.expr(&body.expr);
            format!("{{\n    let _ = {};\n}}", c.text)
        } else {
            printer.convert_typed(&body.expr, typed)
        };
        if typed.ty == Ty::Void { format!("{code};\nOk(())") } else { format!("Ok({code})") }
    }
}

fn message_method(check: &str) -> String {
    format!("{}_message", check)
}

fn real_literal(r: f64) -> String {
    if r.is_infinite() {
        return if r > 0.0 { "f64::INFINITY".into() } else { "f64::NEG_INFINITY".into() };
    }
    let s = format!("{r:?}");
    if s.contains(['.', 'e', 'E', 'N']) { s } else { format!("{s}.0") }
}

/// Whether a feature has an OCL initial value that the generated code applies: one that
/// can be unset (other features have a default value).
fn has_initial(feature: &Feature) -> bool {
    feature.initial.is_some() && feature.is_stored() && !feature.typed.many && feature.typed.nullable
}

fn contains_tuple(ty: &Ty) -> bool {
    match ty {
        Ty::Tuple(_) => true,
        Ty::Collection(_, e) => contains_tuple(e),
        _ => false,
    }
}

/// Whether values of a type are `Copy` in the generated code.
fn is_copy(ty: &Ty) -> bool {
    matches!(
        ty,
        Ty::Boolean
            | Ty::Integer
            | Ty::UnlimitedNatural
            | Ty::Real
            | Ty::Enum(_)
            | Ty::Class(_)
            | Ty::Object
            | Ty::Any
            | Ty::Void
    )
}

fn clone_if_needed(name: &str, t: &Typed) -> String {
    if !t.many && is_copy(&t.ty) { name.to_string() } else { format!("{name}.clone()") }
}

/// A Rust expression: a value of the Rust type of the OCL type, or (`nullable`) an
/// `Option` of it.
#[derive(Clone, Debug)]
pub struct Code {
    pub text: String,
    pub nullable: bool,
}

impl Code {
    fn value(text: impl Into<String>) -> Code {
        Code { text: text.into(), nullable: false }
    }

    fn optional(text: impl Into<String>) -> Code {
        Code { text: text.into(), nullable: true }
    }
}

/// Translates OCL expressions into Rust.
struct RsExpr<'g, 'a> {
    g: &'g mut Gen<'a>,
    scope: Scope,
    origin: String,
    self_name: String,
    /// OCL name, Rust name and type of the parameters.
    params: Vec<(String, String, Typed)>,
    /// The operation's result type, in postconditions.
    result: Option<Typed>,
    /// Variables in scope: lowered name, Rust name, type, nullable.
    vars: Vec<(String, String, Ty, bool)>,
    /// Whether an integer literal is printed as the operand of an operator (whose other
    /// operand fixes its type).
    operand: bool,
}

impl<'g, 'a> RsExpr<'g, 'a> {
    fn new(g: &'g mut Gen<'a>, scope: Scope, origin: &str, self_name: &str) -> Self {
        RsExpr {
            g,
            scope,
            origin: origin.to_string(),
            self_name: self_name.to_string(),
            params: Vec::new(),
            result: None,
            vars: Vec::new(),
            operand: false,
        }
    }

    fn model(&self) -> &'a CodeModel {
        self.g.model
    }

    fn unsupported(&mut self, what: &str) -> String {
        let origin = self.origin.clone();
        self.g.warn(format!("{what} is not supported; it evaluates to invalid"), Some(&origin));
        format!("ocl::invalid({})?", string_literal(&format!("{what} is not supported")))
    }

    /// Like [`RsExpr::unsupported`], for a value of the Rust type `ty`.
    fn unsupported_as(&mut self, what: &str, ty: &str) -> String {
        let code = self.unsupported(what);
        code.replacen("ocl::invalid(", &format!("ocl::invalid::<{ty}>("), 1)
    }

    fn ty(&self, t: &Ty) -> Option<String> {
        self.g.rust_ty(t, self.scope)
    }

    fn ty_or_unit(&self, t: &Ty) -> String {
        self.ty(t).unwrap_or_else(|| "()".into())
    }

    // ---------------------------------------------------------------- conversions

    /// `e` as a non-null value of its own type.
    fn val(&mut self, e: &Expr) -> String {
        let c = self.expr(e);
        if c.nullable {
            format!("ocl::some({}, {})?", c.text, string_literal(&describe(self.model(), e)))
        } else {
            c.text
        }
    }

    /// `e` as a non-null value of type `to`.
    fn conv(&mut self, e: &Expr, to: &Ty) -> String {
        if e.ty == Ty::Void {
            return match to {
                Ty::Collection(..) => "Vec::new()".into(),
                _ => format!("ocl::some(None, {})?", string_literal("null")),
            };
        }
        let v = self.val(e);
        self.convert(v, &e.ty, to)
    }

    /// `e` as an `Option` of type `to`.
    fn conv_opt(&mut self, e: &Expr, to: &Ty) -> String {
        if e.ty == Ty::Void {
            return "None".into();
        }
        let c = self.expr(e);
        if c.nullable {
            if needs_conversion(&e.ty, to) {
                let inner = self.convert("v".into(), &e.ty, to);
                format!("{}.map(|v| {inner})", c.text)
            } else {
                c.text
            }
        } else {
            format!("Some({})", self.convert(c.text, &e.ty, to))
        }
    }

    /// `e` converted to a typed element (a `Vec`, an `Option` or a value).
    fn convert_typed(&mut self, e: &Expr, t: &Typed) -> String {
        if t.many {
            self.conv(e, &t.value_ty())
        } else if t.nullable {
            self.conv_opt(e, &t.ty)
        } else {
            self.conv(e, &t.ty)
        }
    }

    /// Converts a non-null value of type `from` to type `to`.
    fn convert(&mut self, code: String, from: &Ty, to: &Ty) -> String {
        if !needs_conversion(from, to) {
            return code;
        }
        match (from, to) {
            (Ty::Integer | Ty::UnlimitedNatural, Ty::Real) => format!("({code} as f64)"),
            (Ty::Class(_) | Ty::Object | Ty::Any, Ty::Class(_) | Ty::Object | Ty::Any) => {
                let t = self.ty_or_unit(to);
                format!("ocl::recast::<{t}, _>({code})")
            }
            (Ty::Collection(_, a), Ty::Collection(_, b)) => match (&**a, &**b) {
                (Ty::Void, _) => "Vec::new()".into(),
                (Ty::Class(_) | Ty::Object | Ty::Any, Ty::Class(_) | Ty::Object | Ty::Any) => {
                    let t = self.ty_or_unit(b);
                    format!("ocl::recast_all::<{t}, _>({code})")
                }
                (a, b) => {
                    let inner = self.convert("v".into(), a, b);
                    let t = self.ty_or_unit(b);
                    format!("{code}.into_iter().map(|v| {inner}).collect::<Vec<{t}>>()")
                }
            },
            _ => code,
        }
    }

    // ---------------------------------------------------------------- expressions

    fn expr(&mut self, e: &Expr) -> Code {
        let operand = std::mem::take(&mut self.operand);
        let model = self.model();
        match &e.kind {
            ExprKind::Boolean(b) => Code::value(b.to_string()),
            ExprKind::Integer(i) => Code::value(if operand { i.to_string() } else { format!("{i}_i64") }),
            ExprKind::Real(r) => {
                let text = real_literal(*r);
                Code::value(if operand || !text.ends_with(|c: char| c.is_ascii_digit()) {
                    text
                } else {
                    format!("{text}_f64")
                })
            }
            ExprKind::String(s) => Code::value(format!("{}.to_string()", string_literal(s))),
            ExprKind::Unlimited => Code::value("i64::MAX"),
            ExprKind::Null => Code::optional("None"),
            ExprKind::Invalid(reason) => Code::value(format!("ocl::invalid({})?", string_literal(reason))),
            ExprKind::EnumLiteral(en, i) => {
                Code::value(format!("{}::{}", self.g.enum_path(*en, self.scope), self.g.names.literals[en.0][*i]))
            }
            ExprKind::Type(_) => Code::value(self.unsupported("a type used as a value")),
            ExprKind::SelfRef => Code::value(self.self_name.clone()),
            ExprKind::Result => match &self.result {
                Some(t) => {
                    let t = t.clone();
                    let text = clone_if_needed("result", &t);
                    Code { text, nullable: !t.many && t.nullable }
                }
                None => Code::value(self.unsupported("'result' outside a postcondition")),
            },
            ExprKind::Parameter(p) => match self.params.iter().find(|(n, _, _)| n == p).cloned() {
                Some((_, rust_name, t)) => {
                    Code { text: clone_if_needed(&rust_name, &t), nullable: !t.many && t.nullable }
                }
                None => Code::value(self.unsupported(&format!("the parameter '{p}'"))),
            },
            ExprKind::Var(v) => match self.vars.iter().rev().find(|(n, ..)| n == v).cloned() {
                Some((_, rust_name, ty, nullable)) => {
                    let text = if is_copy(&ty) { rust_name } else { format!("{rust_name}.clone()") };
                    Code { text, nullable }
                }
                None => Code::value(self.unsupported(&format!("the variable '{v}'"))),
            },
            ExprKind::Feature { source, feature, safe } => {
                let f = *feature;
                self.navigate(source, *safe, |p, s| p.feature(s, f))
            }
            ExprKind::DefAttribute { source, class, name, safe } => {
                let Some((owner, d)) = model.def_attribute(*class, name) else {
                    return Code::value(self.unsupported(&format!("the definition '{name}'")));
                };
                let method = self.g.names.def_attributes[&(owner, name.clone())].clone();
                let nullable = !d.typed.many && d.typed.nullable;
                self.navigate(source, *safe, |_, s| Code { text: format!("{s}.{method}(m)?"), nullable })
            }
            ExprKind::TuplePart { source, name } => {
                let Ty::Tuple(parts) = &source.ty else { return Code::value(self.unsupported("this tuple")) };
                let Some(i) = parts.iter().position(|(n, _)| n == name) else {
                    return Code::value(self.unsupported(&format!("the tuple part '{name}'")));
                };
                let nullable = !parts[i].1.is_collection();
                let s = self.val(source);
                Code { text: format!("{}.{i}", paren(&s)), nullable }
            }
            ExprKind::Call { source, operation, args, safe } => {
                let op = model.operation(*operation);
                let method = self.g.names.operations[operation.0].clone();
                let args: Vec<String> =
                    args.iter().zip(&op.params).map(|(a, p)| self.convert_typed(a, &p.typed)).collect();
                let args = args.iter().map(|a| format!(", {a}")).collect::<String>();
                let nullable = op.result.as_ref().is_some_and(|r| !r.many && r.nullable);
                self.navigate(source, *safe, |_, s| Code { text: format!("{s}.{method}(m{args})?"), nullable })
            }
            ExprKind::DefCall { source, class, name, args, safe } => {
                let Some((owner, d)) = model.def_operation(*class, name, args.len()) else {
                    return Code::value(self.unsupported(&format!("the definition '{name}'")));
                };
                let method = self.g.names.def_operations[&(owner, name.clone(), args.len())].clone();
                let args: Vec<String> =
                    args.iter().zip(&d.params).map(|(a, p)| self.convert_typed(a, &p.typed)).collect();
                let args = args.iter().map(|a| format!(", {a}")).collect::<String>();
                let nullable = !d.typed.many && d.typed.nullable;
                self.navigate(source, *safe, |_, s| Code { text: format!("{s}.{method}(m{args})?"), nullable })
            }
            ExprKind::Library { name, source, args, safe } => {
                if *safe && !source.ty.is_collection() {
                    let n = name.clone();
                    let a = args.clone();
                    let ty = e.ty.clone();
                    let inner_source = (**source).clone();
                    return self.navigate(source, true, move |p, s| {
                        let var = Expr::new(ExprKind::Var("__s".into()), inner_source.ty.clone());
                        p.vars.push(("__s".into(), s.to_string(), inner_source.ty.clone(), false));
                        let c = p.library(&n, &var, &a, &ty);
                        p.vars.pop();
                        c
                    });
                }
                let mut code = self.library(name, source, args, &e.ty);
                // Unsupported operations fail; their type must still be known.
                if let (Some(rest), Some(t)) = (code.text.strip_prefix("ocl::invalid("), self.ty(&e.ty)) {
                    code.text = format!("ocl::invalid::<{t}>({rest}");
                }
                code
            }
            ExprKind::Iterator { kind, source, vars, body } => self.iterator(*kind, source, vars, body, &e.ty),
            ExprKind::Iterate { source, var, acc, init, body } => {
                let src = self.val(source);
                let element = source.ty.element().cloned().unwrap_or(Ty::Invalid);
                let acc_nullable = self.expr_nullable(init);
                let acc_ty = acc.ty.clone();
                let init_code = if acc_nullable { self.conv_opt(init, &acc_ty) } else { self.conv(init, &acc_ty) };
                let v = ident(&var.name);
                let a = ident(&acc.name);
                self.vars.push((var.name.clone(), v.clone(), element, false));
                self.vars.push((acc.name.clone(), a.clone(), acc_ty.clone(), acc_nullable));
                let body_code = if acc_nullable { self.conv_opt(body, &acc_ty) } else { self.conv(body, &acc_ty) };
                self.vars.pop();
                self.vars.pop();
                let t = self.ty_or_unit(&acc_ty);
                let t = if acc_nullable { format!("Option<{t}>") } else { t };
                Code {
                    text: format!(
                        "{{\n    let mut {a}: {t} = {init_code};\n    for {v} in {src} {{\n        {a} = {body_code};\n    }}\n    {a}\n}}"
                    ),
                    nullable: acc_nullable,
                }
            }
            ExprKind::AllInstances(t) => match t {
                Ty::Class(c) => Code::value(format!("m.all_instances::<{}>()", self.g.class_path(*c, self.scope))),
                Ty::Enum(en) => Code::value(format!("{}::ALL.to_vec()", self.g.enum_path(*en, self.scope))),
                Ty::Boolean => Code::value("vec![true, false]"),
                _ => Code::value(self.unsupported("allInstances() of this type")),
            },
            ExprKind::Binary { op, lhs, rhs } => Code::value(self.binary(*op, lhs, rhs)),
            ExprKind::Unary { op, operand } => {
                let v = self.val(operand);
                Code::value(match (op, &operand.ty) {
                    (UnOp::Not, _) => format!("!{}", paren(&v)),
                    (UnOp::Neg, Ty::Real) => format!("-{}", paren(&v)),
                    (UnOp::Neg, _) => format!("ocl::neg({v})?"),
                })
            }
            ExprKind::If { cond, then_branch, else_branch } => {
                let c = self.val(cond);
                let nullable = self.expr_nullable(then_branch) || self.expr_nullable(else_branch);
                let ty = e.ty.clone();
                let (t, f) = if nullable {
                    (self.conv_opt(then_branch, &ty), self.conv_opt(else_branch, &ty))
                } else {
                    (self.conv(then_branch, &ty), self.conv(else_branch, &ty))
                };
                Code { text: format!("if {c} {{ {t} }} else {{ {f} }}"), nullable }
            }
            ExprKind::Let { var, init, body } => {
                let nullable = self.expr_nullable(init);
                let var_ty = if var.ty == Ty::Invalid { init.ty.clone() } else { var.ty.clone() };
                let init_code = if nullable { self.conv_opt(init, &var_ty) } else { self.conv(init, &var_ty) };
                let v = ident(&var.name);
                let t = self.ty_or_unit(&var_ty);
                let t = if nullable { format!("Option<{t}>") } else { t };
                self.vars.push((var.name.clone(), v.clone(), var_ty, nullable));
                let b = self.expr(body);
                self.vars.pop();
                Code { text: format!("{{\n    let {v}: {t} = {init_code};\n    {}\n}}", b.text), nullable: b.nullable }
            }
            ExprKind::CollectionLiteral { kind, items } => {
                let element = e.ty.element().cloned().unwrap_or(Ty::Void);
                let simple = items.iter().all(|i| matches!(i, CollectionItem::Single(_)));
                let text = if simple {
                    let parts: Vec<String> = items
                        .iter()
                        .filter_map(|i| match i {
                            CollectionItem::Single(x) if x.ty != Ty::Void => Some(self.conv(x, &element)),
                            _ => None,
                        })
                        .collect();
                    if parts.is_empty() {
                        let t = self.ty_or_unit(&element);
                        format!("Vec::<{t}>::new()")
                    } else {
                        format!("vec![{}]", parts.join(", "))
                    }
                } else {
                    let mut stmts = vec!["let mut v = Vec::new();".to_string()];
                    for item in items {
                        match item {
                            CollectionItem::Single(x) => {
                                let c = self.conv(x, &element);
                                stmts.push(format!("v.push({c});"));
                            }
                            CollectionItem::Range(a, b) => {
                                let (a, b) = (self.conv(a, &Ty::Integer), self.conv(b, &Ty::Integer));
                                stmts.push(format!("v.extend(ocl::range({a}, {b})?);"));
                            }
                        }
                    }
                    stmts.push("v".into());
                    format!("{{\n    {}\n}}", stmts.join("\n    "))
                };
                Code::value(if kind.is_unique() && items.len() > 1 || !simple && kind.is_unique() {
                    format!("ocl::unique({text})")
                } else {
                    text
                })
            }
            ExprKind::TupleLiteral(parts) => {
                let Ty::Tuple(types) = &e.ty else { return Code::value(self.unsupported("this tuple")) };
                let mut out = String::from("(");
                for ((_, value), (_, t)) in parts.iter().zip(types) {
                    let code = if t.is_collection() { self.conv(value, t) } else { self.conv_opt(value, t) };
                    out.push_str(&code);
                    out.push_str(", ");
                }
                out.pop();
                out.push(')');
                Code::value(out)
            }
        }
    }

    /// Whether `e` is printed as an `Option` (as `expr` decides it, without printing).
    fn expr_nullable(&mut self, e: &Expr) -> bool {
        let model = self.model();
        let typed = |t: &Typed| !t.many && t.nullable;
        match &e.kind {
            ExprKind::Null => true,
            ExprKind::Result => self.result.as_ref().is_some_and(typed),
            ExprKind::Parameter(p) => self.params.iter().find(|(n, _, _)| n == p).is_some_and(|(_, _, t)| typed(t)),
            ExprKind::Var(v) => self.vars.iter().rev().find(|(n, ..)| n == v).is_some_and(|(.., nullable)| *nullable),
            ExprKind::Feature { source, feature, safe } => {
                let f = model.feature(*feature);
                let own = !f.typed.many && (f.is_reference() || f.typed.nullable);
                own || (*safe && self.expr_nullable(source))
            }
            ExprKind::DefAttribute { source, class, name, safe } => {
                let own = model.def_attribute(*class, name).is_some_and(|(_, d)| typed(&d.typed));
                own || (*safe && self.expr_nullable(source))
            }
            ExprKind::Call { source, operation, safe, .. } => {
                let own = model.operation(*operation).result.as_ref().is_some_and(typed);
                own || (*safe && self.expr_nullable(source))
            }
            ExprKind::DefCall { source, class, name, args, safe } => {
                let own = model.def_operation(*class, name, args.len()).is_some_and(|(_, d)| typed(&d.typed));
                own || (*safe && self.expr_nullable(source))
            }
            ExprKind::Library { name, source, safe, .. } => match name.as_str() {
                _ if *safe && !source.ty.is_collection() => self.expr_nullable(source),
                "oclAsType" => self.expr_nullable(source),
                "oclContainer" => true,
                _ => false,
            },
            ExprKind::Iterator { kind, .. } => *kind == IteratorKind::Any,
            ExprKind::Iterate { init, .. } => self.expr_nullable(init),
            ExprKind::If { then_branch, else_branch, .. } => {
                self.expr_nullable(then_branch) || self.expr_nullable(else_branch)
            }
            ExprKind::Let { var, init, body } => {
                let nullable = self.expr_nullable(init);
                self.vars.push((var.name.clone(), String::new(), var.ty.clone(), nullable));
                let b = self.expr_nullable(body);
                self.vars.pop();
                b
            }
            _ => false,
        }
    }

    /// Accesses something on a non-null object `source` (with `safe`, a null source
    /// yields null).
    fn navigate(&mut self, source: &Expr, safe: bool, access: impl FnOnce(&mut Self, &str) -> Code) -> Code {
        let src = self.expr(source);
        if safe && src.nullable {
            let inner = access(self, "__s");
            let some = if inner.nullable { inner.text } else { format!("Some({})", inner.text) };
            return Code::optional(format!("match {} {{ Some(__s) => {some}, None => None }}", src.text));
        }
        let s = if src.nullable {
            format!("ocl::some({}, {})?", src.text, string_literal(&describe(self.model(), source)))
        } else {
            src.text
        };
        access(self, &paren(&s))
    }

    /// Reading feature `f` of the object `s`.
    fn feature(&mut self, s: &str, f: FeatureId) -> Code {
        let model = self.model();
        let feature = model.feature(f);
        let n = &self.g.names.features[f.0];
        let t = &feature.typed;
        if feature.derivation.is_some() || (!feature.is_stored() && !feature.is_container()) {
            return Code { text: format!("{s}.{n}(m)?"), nullable: !t.many && t.nullable };
        }
        if t.many {
            return Code::value(format!("{s}.{n}(m).to_vec()"));
        }
        if has_initial(feature) {
            return Code::optional(format!("{s}.{n}_or_initial(m)?"));
        }
        if feature.is_reference() {
            return Code::optional(format!("{s}.{n}(m)"));
        }
        match (&t.ty, t.nullable) {
            (Ty::String | Ty::DataType(_), true) => Code::optional(format!("{s}.{n}(m).map(str::to_string)")),
            (Ty::String | Ty::DataType(_), false) => Code::value(format!("{s}.{n}(m).to_string()")),
            (_, nullable) => Code { text: format!("{s}.{n}(m)"), nullable },
        }
    }

    /// `toString()` of `e`.
    fn ocl_string_code(&mut self, e: &Expr) -> String {
        if e.ty == Ty::String {
            return self.val(e);
        }
        if contains_tuple(&e.ty) {
            return self.unsupported_as("the text of a tuple", "String");
        }
        let c = self.expr(e);
        let render = |p: &mut Self, v: &str, ty: &Ty| -> String {
            match ty {
                Ty::Class(_) | Ty::Object | Ty::Any => format!("m.label({v})"),
                Ty::Collection(k, inner) => {
                    let inner = (**inner).clone();
                    let element = match &inner {
                        Ty::String => "ocl::quote(x)".to_string(),
                        Ty::Class(_) | Ty::Object | Ty::Any => "m.label(*x)".into(),
                        _ => "x.ocl_string()".into(),
                    };
                    let _ = p;
                    format!("ocl::render_collection({}, &{v}, |x| {element})", string_literal(k.name()))
                }
                _ => format!("{v}.ocl_string()"),
            }
        };
        if c.nullable {
            let inner = render(self, "v", &e.ty);
            format!("{}.map_or_else(|| \"null\".to_string(), |v| {inner})", c.text)
        } else {
            let text = paren(&c.text);
            render(self, &text, &e.ty)
        }
    }

    fn type_arg(&mut self, args: &[Expr]) -> Option<Ty> {
        match args.first().map(|a| &a.kind) {
            Some(ExprKind::Type(t)) => Some(t.clone()),
            _ => None,
        }
    }

    fn library(&mut self, name: &str, source: &Expr, args: &[Expr], ty: &Ty) -> Code {
        let model = self.model();
        match name {
            "oclIsUndefined" => {
                let c = self.expr(source);
                return Code::value(match (c.nullable, may_fail(source)) {
                    (true, false) => format!("{}.is_none()", paren(&c.text)),
                    (true, true) => format!("ocl::is_undefined(|| Ok({}))", c.text),
                    (false, true) => format!("ocl::is_invalid(|| Ok({}))", c.text),
                    (false, false) => "false".into(),
                });
            }
            "oclIsInvalid" => {
                let c = self.expr(source);
                return Code::value(if may_fail(source) {
                    format!("ocl::is_invalid(|| Ok({}))", c.text)
                } else {
                    "false".into()
                });
            }
            "oclIsKindOf" | "oclIsTypeOf" => {
                let Some(t) = self.type_arg(args) else {
                    return Code::value(self.unsupported("a type computed at run time"));
                };
                let c = self.expr(source);
                let exact = name == "oclIsTypeOf";
                let test = |p: &mut Self, v: &str| -> String {
                    match (&source.ty, &t) {
                        (Ty::Class(_) | Ty::Object | Ty::Any, Ty::Class(target)) => {
                            if exact {
                                format!("m.class_of({v}) == crate::Class::{}", p.g.names.class_variants[target.0])
                            } else {
                                format!("m.is::<{}>({v})", p.g.class_path(*target, p.scope))
                            }
                        }
                        (Ty::Class(_) | Ty::Object | Ty::Any, Ty::Object | Ty::Any) => "true".into(),
                        (from, to) => (from == to || (!exact && statically_conforms(model, from, to))).to_string(),
                    }
                };
                return Code::value(if c.nullable {
                    let inner = test(self, "v");
                    format!("{}.map_or(false, |v| {inner})", paren(&c.text))
                } else {
                    let v = paren(&c.text);
                    test(self, &v)
                });
            }
            "oclAsType" => {
                let Some(t) = self.type_arg(args) else {
                    return Code::value(self.unsupported("a type computed at run time"));
                };
                let c = self.expr(source);
                let cast = |p: &mut Self, v: &str| -> String {
                    match (&source.ty, &t) {
                        (Ty::Class(_) | Ty::Object | Ty::Any, Ty::Class(target)) => {
                            format!("m.cast_or_invalid::<{}>({v})?", p.g.class_path(*target, p.scope))
                        }
                        (Ty::Class(_), Ty::Object) => format!("{v}.e_id()"),
                        (from, to) => p.convert(v.to_string(), from, to),
                    }
                };
                return if c.nullable {
                    let inner = cast(self, "v");
                    Code::optional(format!("match {} {{ Some(v) => Some({inner}), None => None }}", c.text))
                } else {
                    let v = paren(&c.text);
                    Code::value(cast(self, &v))
                };
            }
            "oclAsSet" => {
                if source.ty == Ty::Void {
                    let t = self.ty_or_unit(ty.element().unwrap_or(&Ty::Void));
                    return Code::value(format!("Vec::<{t}>::new()"));
                }
                let c = self.expr(source);
                return Code::value(if c.nullable {
                    format!("{}.into_iter().collect::<Vec<_>>()", paren(&c.text))
                } else {
                    format!("vec![{}]", c.text)
                });
            }
            "toString" => return Code::value(self.ocl_string_code(source)),
            "oclContainer" => {
                let v = self.val(source);
                return Code::optional(format!("m.container_of({v})"));
            }
            "oclContents" => {
                let v = self.val(source);
                return Code::value(format!("m.contents({v})"));
            }
            "eAllContents" => {
                let v = self.val(source);
                return Code::value(format!("m.all_contents({v})"));
            }
            "oclType" => return Code::value(self.unsupported("oclType()")),
            _ => {}
        }
        if let Ty::Collection(kind, element) = &source.ty {
            return Code::value(self.collection_op(name, source, *kind, element, args, ty));
        }
        match &source.ty {
            Ty::String | Ty::DataType(_) => Code::value(self.string_op(name, source, args)),
            Ty::Integer | Ty::Real | Ty::UnlimitedNatural => Code::value(self.number_op(name, source, args, ty)),
            _ => Code::value(self.unsupported(&format!("the operation '{name}' on this type"))),
        }
    }

    fn collection_op(
        &mut self,
        name: &str,
        source: &Expr,
        kind: CollectionKind,
        element: &Ty,
        args: &[Expr],
        ty: &Ty,
    ) -> String {
        let unique = kind.is_unique();
        // Collections in the result type, so that elements of different (conforming)
        // types are combined in one Vec.
        let result_element = ty.element().cloned().unwrap_or_else(|| element.clone());
        let src_as = |p: &mut Self, to: &Ty| -> String {
            let target = Ty::collection(kind, to.clone());
            p.conv(source, &target)
        };
        let arg_elem = |p: &mut Self, i: usize, to: &Ty| -> String {
            match args.get(i) {
                Some(a) => p.conv(a, to),
                None => "()".into(),
            }
        };
        let arg_coll = |p: &mut Self, i: usize, to: &Ty| -> String {
            match args.get(i) {
                Some(a) => {
                    let k = a.ty.collection_kind().unwrap_or(CollectionKind::Bag);
                    p.conv(a, &Ty::collection(k, to.clone()))
                }
                None => "Vec::new()".into(),
            }
        };
        let is_null = |i: usize| args.get(i).is_some_and(|a| a.ty == Ty::Void);
        match name {
            "size" => format!("({}.len() as i64)", self.val(source)),
            "isEmpty" => format!("{}.is_empty()", self.val(source)),
            "notEmpty" => format!("!{}.is_empty()", self.val(source)),
            "includes" | "excludes" | "count" | "indexOf" => {
                if is_null(0) {
                    return match name {
                        "includes" => "false".into(),
                        "excludes" => "true".into(),
                        "count" => "0_i64".into(),
                        _ => "ocl::invalid(\"indexOf(): element not found\")?".into(),
                    };
                }
                let s = self.val(source);
                let x = arg_elem(self, 0, element);
                match name {
                    "includes" => format!("{s}.contains(&{x})"),
                    "excludes" => format!("!{s}.contains(&{x})"),
                    "count" => format!("ocl::count(&{s}, &{x})"),
                    _ => format!("ocl::index_of(&{s}, &{x})?"),
                }
            }
            "includesAll" | "excludesAll" => {
                let s = self.val(source);
                let c = arg_coll(self, 0, element);
                let f = if name == "includesAll" { "includes_all" } else { "excludes_all" };
                format!("ocl::{f}(&{s}, &{c})")
            }
            "sum" => {
                let s = self.val(source);
                match element {
                    Ty::Real => format!("ocl::sum_real(&{s})"),
                    _ => format!("ocl::sum_int(&{s})?"),
                }
            }
            "max" | "min" => {
                let s = self.val(source);
                format!("ocl::{name}(&{s})?")
            }
            "asSet" | "asOrderedSet" => {
                let s = self.val(source);
                if unique { s } else { format!("ocl::unique({s})") }
            }
            "asSequence" | "asBag" => self.val(source),
            "flatten" => {
                let s = self.val(source);
                if element.is_collection() {
                    let flat = format!("{s}.into_iter().flatten().collect::<Vec<_>>()");
                    if unique { format!("ocl::unique({flat})") } else { flat }
                } else {
                    s
                }
            }
            "including" | "append" | "prepend" => {
                let s = src_as(self, &result_element);
                if is_null(0) {
                    return s;
                }
                let x = arg_elem(self, 0, &result_element);
                format!("ocl::{name}({s}, {x}, {unique})")
            }
            "insertAt" => {
                let s = src_as(self, &result_element);
                let i = arg_elem(self, 0, &Ty::Integer);
                if is_null(1) {
                    return s;
                }
                let x = arg_elem(self, 1, &result_element);
                format!("ocl::insert_at({s}, {i}, {x}, {unique})?")
            }
            "excluding" => {
                let s = self.val(source);
                if is_null(0) {
                    return s;
                }
                let x = arg_elem(self, 0, element);
                format!("ocl::excluding({s}, &{x})")
            }
            "includingAll" | "union" => {
                let s = src_as(self, &result_element);
                let c = arg_coll(self, 0, &result_element);
                let result_unique = ty.collection_kind().is_some_and(|k| k.is_unique());
                format!("ocl::including_all({s}, {c}, {result_unique})")
            }
            "excludingAll" => {
                let s = self.val(source);
                let c = arg_coll(self, 0, element);
                format!("ocl::excluding_all({s}, &{c})")
            }
            "intersection" => {
                let s = self.val(source);
                let c = arg_coll(self, 0, element);
                let bag = ty.collection_kind() == Some(CollectionKind::Bag);
                format!("ocl::intersection({s}, {c}, {bag})")
            }
            "symmetricDifference" => {
                let s = self.val(source);
                let c = arg_coll(self, 0, element);
                format!("ocl::symmetric_difference({s}, {c})")
            }
            "at" => {
                let s = self.val(source);
                let i = arg_elem(self, 0, &Ty::Integer);
                format!("ocl::at(&{s}, {i})?")
            }
            "first" | "last" => {
                let s = self.val(source);
                format!("ocl::{name}(&{s})?")
            }
            "subSequence" => {
                let s = self.val(source);
                let a = arg_elem(self, 0, &Ty::Integer);
                let b = arg_elem(self, 1, &Ty::Integer);
                format!("ocl::sub_sequence(&{s}, {a}, {b})?")
            }
            "reverse" => format!("ocl::reverse({})", self.val(source)),
            "selectByKind" | "selectByType" => {
                let Some(t) = self.type_arg(args) else { return self.unsupported("a type computed at run time") };
                let s = self.val(source);
                match (element, &t) {
                    (Ty::Class(_) | Ty::Object | Ty::Any, Ty::Class(c)) => {
                        let f = if name == "selectByKind" { "select_by_kind" } else { "select_by_type" };
                        format!("m.{f}::<{}, _>({s})", self.g.class_path(*c, self.scope))
                    }
                    (from, to)
                        if from == to || (name == "selectByKind" && statically_conforms(self.model(), from, to)) =>
                    {
                        s
                    }
                    _ => {
                        let t = self.ty_or_unit(&t);
                        format!("Vec::<{t}>::new()")
                    }
                }
            }
            "product" => {
                let other = args.first().and_then(|a| a.ty.element().cloned());
                match other {
                    Some(o) if !element.is_collection() && !o.is_collection() => {
                        let s = self.val(source);
                        let k = args[0].ty.collection_kind().unwrap_or(CollectionKind::Bag);
                        let c = self.conv(&args[0], &Ty::collection(k, o));
                        format!("ocl::product(&{s}, &{c})")
                    }
                    _ => self.unsupported("product() of nested collections"),
                }
            }
            other => self.unsupported(&format!("the collection operation '{other}'")),
        }
    }

    fn string_op(&mut self, name: &str, source: &Expr, args: &[Expr]) -> String {
        let s = self.val(source);
        let s = paren(&s);
        let mut a: Vec<String> = Vec::new();
        for (i, x) in args.iter().enumerate() {
            let to = match (name, i) {
                ("substring", _) | ("at", _) => Ty::Integer,
                ("tokenize", 1) => Ty::Boolean,
                _ => Ty::String,
            };
            a.push(self.conv(x, &to));
        }
        let arg = |i: usize| a.get(i).cloned().unwrap_or_default();
        match name {
            "size" => format!("ocl::str_size(&{s})"),
            "concat" => format!("format!(\"{{}}{{}}\", {s}, {})", arg(0)),
            "substring" => format!("ocl::substring(&{s}, {}, {})?", arg(0), arg(1)),
            "toInteger" => format!("ocl::str_to_integer(&{s})?"),
            "toReal" => format!("ocl::str_to_real(&{s})?"),
            "toBoolean" => format!("ocl::str_to_boolean(&{s})?"),
            "toUpper" => format!("{s}.to_uppercase()"),
            "toLower" => format!("{s}.to_lowercase()"),
            "trim" => format!("{s}.trim().to_string()"),
            "indexOf" => format!("ocl::str_index_of(&{s}, &{})", arg(0)),
            "lastIndexOf" => format!("ocl::str_last_index_of(&{s}, &{})", arg(0)),
            "at" => format!("ocl::str_at(&{s}, {})?", arg(0)),
            "characters" => format!("ocl::characters(&{s})"),
            "equalsIgnoreCase" => format!("ocl::equals_ignore_case(&{s}, &{})", arg(0)),
            "startsWith" => format!("{s}.starts_with({}.as_str())", paren(&arg(0))),
            "endsWith" => format!("{s}.ends_with({}.as_str())", paren(&arg(0))),
            "substituteAll" => format!("{s}.replace({}.as_str(), &{})", paren(&arg(0)), arg(1)),
            "substituteFirst" => format!("{s}.replacen({}.as_str(), &{}, 1)", paren(&arg(0)), arg(1)),
            "tokenize" => match a.len() {
                0 => format!("ocl::tokenize(&{s}, ocl::WHITESPACE, false)"),
                1 => format!("ocl::tokenize(&{s}, &{}, false)", arg(0)),
                _ => format!("ocl::tokenize(&{s}, &{}, {})", arg(0), arg(1)),
            },
            "matches" | "replaceAll" | "replaceFirst" => {
                self.unsupported(&format!("'{name}' (regular expressions; the generated crate has no dependencies)"))
            }
            other => self.unsupported(&format!("the String operation '{other}'")),
        }
    }

    fn number_op(&mut self, name: &str, source: &Expr, args: &[Expr], ty: &Ty) -> String {
        let real = source.ty == Ty::Real;
        match name {
            "abs" => {
                let v = self.val(source);
                if real { format!("{}.abs()", paren(&v)) } else { format!("ocl::abs({v})?") }
            }
            "floor" | "round" => {
                let v = self.val(source);
                if real { format!("ocl::{name}({v})?") } else { v }
            }
            "max" | "min" => {
                if *ty == Ty::Real {
                    let a = self.conv(source, &Ty::Real);
                    let b = args.first().map(|x| self.conv(x, &Ty::Real)).unwrap_or_default();
                    format!("ocl::{name}_real({a}, {b})")
                } else {
                    let a = self.val(source);
                    let b = args.first().map(|x| self.conv(x, &Ty::Integer)).unwrap_or_default();
                    format!("{}.{name}({b})", paren(&a))
                }
            }
            "div" | "mod" => {
                let a = self.conv(source, &Ty::Integer);
                let b = args.first().map(|x| self.conv(x, &Ty::Integer)).unwrap_or_default();
                format!("ocl::{}({a}, {b})?", if name == "div" { "div" } else { "modulo" })
            }
            "toInteger" => {
                let v = self.val(source);
                if real { format!("ocl::real_to_int({}.trunc())?", paren(&v)) } else { v }
            }
            "toReal" => self.conv(source, &Ty::Real),
            other => self.unsupported(&format!("the number operation '{other}'")),
        }
    }

    fn iterator(&mut self, kind: IteratorKind, source: &Expr, vars: &[Var], body: &Expr, ty: &Ty) -> Code {
        let src = self.val(source);
        let element = source.ty.element().cloned().unwrap_or(Ty::Invalid);
        let names: Vec<String> = vars.iter().map(|v| ident(&v.name)).collect();
        for (v, n) in vars.iter().zip(&names) {
            self.vars.push((v.name.clone(), n.clone(), element.clone(), false));
        }
        let v = names[0].clone();
        let result = match kind {
            IteratorKind::Select | IteratorKind::Reject | IteratorKind::One | IteratorKind::Any => {
                let b = self.conv(body, &Ty::Boolean);
                let f = match kind {
                    IteratorKind::Select => "select",
                    IteratorKind::Reject => "reject",
                    IteratorKind::One => "one",
                    _ => "any",
                };
                let text = format!("ocl::{f}({src}, |{v}| Ok({b}))?");
                if kind == IteratorKind::Any { Code::optional(text) } else { Code::value(text) }
            }
            IteratorKind::ForAll | IteratorKind::Exists => {
                let b = self.conv(body, &Ty::Boolean);
                let f = if kind == IteratorKind::ForAll { "for_all" } else { "exists" };
                if names.len() == 1 {
                    Code::value(format!("ocl::{f}({src}, |{v}| Ok({b}))?"))
                } else {
                    // All combinations: nested iterations over the same source.
                    let mut inner = format!("Ok({b})");
                    for n in names.iter().rev() {
                        inner = format!("ocl::{f}(__source.clone(), |{n}| {inner})");
                    }
                    Code::value(format!("{{\n    let __source = {src};\n    {inner}?\n}}"))
                }
            }
            IteratorKind::IsUnique => {
                let b = self.expr(body);
                Code::value(format!("ocl::is_unique({src}, |{v}| Ok({}))?", b.text))
            }
            IteratorKind::SortedBy => {
                let b = self.val(body);
                Code::value(format!("ocl::sorted_by({src}, |{v}| Ok({b}))?"))
            }
            IteratorKind::Collect | IteratorKind::CollectNested => {
                let target = ty.element().cloned().unwrap_or(Ty::Invalid);
                if kind == IteratorKind::Collect && body.ty.is_collection() {
                    let k = body.ty.collection_kind().unwrap_or(CollectionKind::Bag);
                    let b = self.conv(body, &Ty::collection(k, target));
                    Code::value(format!("ocl::collect_flat({src}, |{v}| Ok({b}))?"))
                } else if self.expr_nullable(body) {
                    let b = self.conv_opt(body, &target);
                    Code::value(format!("ocl::collect_some({src}, |{v}| Ok({b}))?"))
                } else {
                    let b = self.conv(body, &target);
                    Code::value(format!("ocl::collect({src}, |{v}| Ok({b}))?"))
                }
            }
            IteratorKind::Closure => {
                let target = element.clone();
                let b = if body.ty.is_collection() {
                    let k = body.ty.collection_kind().unwrap_or(CollectionKind::Bag);
                    self.conv(body, &Ty::collection(k, target))
                } else if self.expr_nullable(body) {
                    let o = self.conv_opt(body, &target);
                    format!("{}.into_iter().collect::<Vec<_>>()", paren(&o))
                } else {
                    format!("vec![{}]", self.conv(body, &target))
                };
                let text = format!("ocl::closure({src}, |{v}| Ok({b}))?");
                // The result's element type may be wider than the source's.
                Code::value(self.convert(text, &Ty::collection(CollectionKind::Set, element.clone()), ty))
            }
        };
        for _ in vars {
            self.vars.pop();
        }
        result
    }

    fn binary(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr) -> String {
        let model = self.model();
        match op {
            BinOp::And | BinOp::Or | BinOp::Implies | BinOp::Xor => {
                let a = self.conv(lhs, &Ty::Boolean);
                let b = self.conv(rhs, &Ty::Boolean);
                let (a, b) = (paren(&a), paren(&b));
                match op {
                    BinOp::And => format!("{a} && {b}"),
                    BinOp::Or => format!("{a} || {b}"),
                    BinOp::Implies => format!("!{a} || {b}"),
                    _ => format!("{a} != {b}"),
                }
            }
            BinOp::Eq | BinOp::Ne => {
                let eq = self.equality(lhs, rhs, model);
                if op == BinOp::Eq {
                    return eq;
                }
                match top_level_find(&eq, " == ") {
                    Some(i)
                        if top_level_find(&eq[i + 4..], " ").is_none() && top_level_find(&eq[..i], " ").is_none() =>
                    {
                        format!("{} != {}", &eq[..i], &eq[i + 4..])
                    }
                    _ => match eq.as_str() {
                        "true" => "false".into(),
                        "false" => "true".into(),
                        _ if eq.ends_with(".is_none()") && is_simple(&eq) => {
                            format!("{}.is_some()", eq.trim_end_matches(".is_none()"))
                        }
                        _ => format!("!{}", paren(&eq)),
                    },
                }
            }
            BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                let common = numeric_common(&lhs.ty, &rhs.ty).unwrap_or_else(|| lhs.ty.clone());
                let (a, b) = self.operands(lhs, rhs, &common);
                let o = op.symbol();
                format!("{a} {o} {b}")
            }
            BinOp::Add | BinOp::Sub | BinOp::Mul => {
                if op == BinOp::Sub && lhs.ty.is_collection() {
                    let a = self.val(lhs);
                    let element = lhs.ty.element().cloned().unwrap_or(Ty::Invalid);
                    let k = rhs.ty.collection_kind().unwrap_or(CollectionKind::Set);
                    let b = self.conv(rhs, &Ty::collection(k, element));
                    return format!("ocl::difference({a}, &{b})");
                }
                if op == BinOp::Add && lhs.ty == Ty::String {
                    let a = self.val(lhs);
                    let b = self.conv(rhs, &Ty::String);
                    return format!("format!(\"{{}}{{}}\", {a}, {b})");
                }
                let common = numeric_common(&lhs.ty, &rhs.ty).unwrap_or(Ty::Integer);
                let (a, b) = self.operands(lhs, rhs, &common);
                if common == Ty::Real {
                    format!("{a} {} {b}", op.symbol())
                } else {
                    let f = match op {
                        BinOp::Add => "add",
                        BinOp::Sub => "sub",
                        _ => "mul",
                    };
                    format!("ocl::{f}({a}, {b})?")
                }
            }
            BinOp::Div => {
                let a = self.conv(lhs, &Ty::Real);
                let b = self.conv(rhs, &Ty::Real);
                format!("ocl::divide({a}, {b})?")
            }
        }
    }

    /// Both operands as non-null values of type `common` (literals typed by the other).
    fn operands(&mut self, lhs: &Expr, rhs: &Expr, common: &Ty) -> (String, String) {
        self.operand = !matches!(rhs.kind, ExprKind::Integer(_));
        let a = self.conv(lhs, common);
        self.operand = !matches!(lhs.kind, ExprKind::Integer(_));
        let b = self.conv(rhs, common);
        self.operand = false;
        (paren(&a), paren(&b))
    }

    fn equality(&mut self, lhs: &Expr, rhs: &Expr, model: &CodeModel) -> String {
        // Comparisons with null.
        if rhs.ty == Ty::Void || lhs.ty == Ty::Void {
            let other = if rhs.ty == Ty::Void { lhs } else { rhs };
            if other.ty == Ty::Void {
                return "true".into();
            }
            let c = self.expr(other);
            return if c.nullable { format!("{}.is_none()", paren(&c.text)) } else { "false".into() };
        }
        if let (Ty::Collection(k1, e1), Ty::Collection(k2, e2)) = (&lhs.ty, &rhs.ty) {
            if k1 != k2 && *k1 != CollectionKind::Collection && *k2 != CollectionKind::Collection {
                return "false".into();
            }
            let element = common_type(model, e1, e2);
            let a = self.conv(lhs, &Ty::collection(*k1, element.clone()));
            let b = self.conv(rhs, &Ty::collection(*k2, element));
            return match k1 {
                CollectionKind::Set => format!("ocl::set_eq(&{a}, &{b})"),
                CollectionKind::Bag => format!("ocl::bag_eq(&{a}, &{b})"),
                _ => format!("{a} == {b}"),
            };
        }
        let common = match (numeric_common(&lhs.ty, &rhs.ty), &lhs.ty, &rhs.ty) {
            (Some(t), _, _) => t,
            (None, Ty::Class(a), Ty::Class(b)) if a != b => Ty::Object,
            (None, Ty::Class(_) | Ty::Object | Ty::Any, Ty::Object | Ty::Any) => Ty::Object,
            (None, Ty::Object | Ty::Any, Ty::Class(_)) => Ty::Object,
            (None, a, _) => a.clone(),
        };
        let nullable = self.expr_nullable(lhs) || self.expr_nullable(rhs);
        if nullable {
            let a = self.conv_opt(lhs, &common);
            let b = self.conv_opt(rhs, &common);
            return format!("{} == {}", paren(&a), paren(&b));
        }
        let (a, b) = self.operands(lhs, rhs, &common);
        format!("{a} == {b}")
    }
}

/// A short description of an expression for "… is null" messages.
fn describe(model: &CodeModel, e: &Expr) -> String {
    match &e.kind {
        ExprKind::Feature { feature, .. } => format!("'{}'", model.feature(*feature).name),
        ExprKind::Call { operation, .. } => format!("the result of '{}'", model.operation(*operation).name),
        ExprKind::Var(v) => format!("'{v}'"),
        ExprKind::Parameter(p) => format!("'{p}'"),
        ExprKind::Result => "'result'".into(),
        _ => "a value".into(),
    }
}

/// Whether evaluating `e` can fail.
fn may_fail(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::SelfRef
        | ExprKind::Result
        | ExprKind::Parameter(_)
        | ExprKind::Var(_)
        | ExprKind::Null
        | ExprKind::Boolean(_)
        | ExprKind::Integer(_)
        | ExprKind::Real(_)
        | ExprKind::String(_)
        | ExprKind::EnumLiteral(..) => false,
        ExprKind::Feature { source, .. } => !matches!(source.kind, ExprKind::SelfRef),
        _ => true,
    }
}

fn needs_conversion(from: &Ty, to: &Ty) -> bool {
    match (from, to) {
        (a, b) if a == b => false,
        (Ty::Integer | Ty::UnlimitedNatural, Ty::Real) => true,
        (Ty::Class(_) | Ty::Object | Ty::Any, Ty::Class(_) | Ty::Object | Ty::Any) => true,
        (Ty::Collection(_, a), Ty::Collection(_, b)) => needs_conversion(a, b) || **a == Ty::Void && **b != Ty::Void,
        _ => false,
    }
}

fn numeric_common(a: &Ty, b: &Ty) -> Option<Ty> {
    match (a, b) {
        (Ty::Real, x) | (x, Ty::Real) if x.is_numeric() => Some(Ty::Real),
        (x, y) if x.is_numeric() && y.is_numeric() => Some(Ty::Integer),
        _ => None,
    }
}

/// The common type of two element types (for comparing and combining collections).
fn common_type(model: &CodeModel, a: &Ty, b: &Ty) -> Ty {
    if a == b {
        return a.clone();
    }
    if let Some(t) = numeric_common(a, b) {
        return t;
    }
    match (a, b) {
        (Ty::Void, x) | (x, Ty::Void) => x.clone(),
        (Ty::Object | Ty::Any, _) | (_, Ty::Object | Ty::Any) => Ty::Object,
        (Ty::Class(x), Ty::Class(y)) => {
            if model.conforms(*x, *y) {
                Ty::Class(*y)
            } else if model.conforms(*y, *x) {
                Ty::Class(*x)
            } else {
                model
                    .class(*x)
                    .all_supers
                    .iter()
                    .find(|s| model.conforms(*y, **s))
                    .map_or(Ty::Object, |s| Ty::Class(*s))
            }
        }
        _ => a.clone(),
    }
}

/// Whether values of type `from` are always of type `to`.
fn statically_conforms(model: &CodeModel, from: &Ty, to: &Ty) -> bool {
    match (from, to) {
        (a, b) if a == b => true,
        (_, Ty::Any) => true,
        (Ty::Integer | Ty::UnlimitedNatural, Ty::Real) => true,
        (Ty::Class(a), Ty::Class(b)) => model.conforms(*a, *b),
        (Ty::Class(_) | Ty::Any, Ty::Object) => true,
        _ => false,
    }
}

/// Whether a Rust expression needs no parentheses as an operand.
fn is_simple(s: &str) -> bool {
    !s.starts_with(['!', '-', '&']) && top_level_find(s, " ").is_none() && top_level_find(s, "\n").is_none()
}

/// The byte offset of the first occurrence of `pattern` outside brackets, string literals
/// and turbofish generics.
fn top_level_find(s: &str, pattern: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut angles = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        if depth == 0 && angles == 0 && s[i..].starts_with(pattern) {
            return Some(i);
        }
        match c {
            '"' => in_string = true,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '<' if i >= 2 && &bytes[i - 2..i] == b"::" => angles += 1,
            '>' if angles > 0 => angles -= 1,
            _ => {}
        }
    }
    None
}

fn paren(s: &str) -> String {
    if is_simple(s) || s.starts_with("ocl::") && s.ends_with(")?") && balanced_call(s) {
        s.to_string()
    } else {
        format!("({s})")
    }
}

/// Whether `s` is a single call `f(…)?` (so it needs no parentheses).
fn balanced_call(s: &str) -> bool {
    let Some(open) = s.find('(') else { return false };
    let mut depth = 0;
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in s.char_indices().skip_while(|(i, _)| *i < open) {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return i + 2 == s.len();
                }
            }
            _ => {}
        }
    }
    false
}
