//! The Python target: a package with one module per EPackage.
//!
//! ```text
//! <name>/__init__.py      re-exports the classes, load/save/validate
//! <name>/_runtime.py      the runtime (runtime/python/_runtime.py, copied verbatim)
//! <name>/_model.py        the enumerations and classes of all packages
//! <name>/<package>.py     the enumerations and classes of one EPackage (re-exported)
//! ```
//!
//! All classes are defined in one module, supertypes first: packages may well inherit from
//! each other in both directions (PCM does), which no order of per-package modules allows.
//!
//! Classes derive from `_runtime.EObject`; stored features are descriptors (`Attribute`,
//! `Reference`) that keep containment and opposites consistent. Derived features and `def:`
//! attributes are properties, operations are methods, invariants are `check_<name>()`
//! methods listed in `_e_invariants` (which `validate` evaluates). OCL `invalid` is the
//! exception `OclInvalid`; null is `None`; collections are lists (their kind only matters to
//! a few operations, which get it as an argument).

use std::collections::HashMap;

use crate::model::*;
use crate::naming::{Names, snake_case};
use crate::writer::Writer;
use crate::{Options, Output, Target};

/// The runtime copied into every generated package.
pub const RUNTIME: &str = include_str!("../runtime/python/_runtime.py");

pub struct Python;

impl Target for Python {
    fn id(&self) -> &'static str {
        "python"
    }

    fn name(&self) -> &'static str {
        "Python"
    }

    fn description(&self) -> &'static str {
        "A Python package (3.9+, no dependencies): classes with consistent references, XMI loading and saving, \
         OCL invariants, operations and contracts as native code"
    }

    fn generate(&self, model: &CodeModel, options: &Options, output: &mut Output) {
        let names = PyNames::new(model, options);
        let package_dir = names.package.clone();
        output.file(format!("{package_dir}/_runtime.py"), RUNTIME);
        let text = Module { model, names: &names, output }.generate();
        output.file(format!("{package_dir}/_model.py"), text);
        for i in 0..model.packages.len() {
            output.file(format!("{package_dir}/{}.py", names.modules[i]), package_module(model, &names, PackageId(i)));
        }
        output.file(format!("{package_dir}/__init__.py"), init_module(model, &names));
    }
}

const KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
    "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal",
    "not", "or", "pass", "raise", "return", "try", "while", "with", "yield",
];

/// Names the generated code uses besides the members of the classes.
const RESERVED: &[&str] =
    &["self", "result", "_rt", "PACKAGE", "len", "list", "any", "all", "sum", "next", "sorted", "annotations"];

/// A valid Python identifier for `name`.
pub fn ident(name: &str) -> String {
    let mut s: String = name.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if s.is_empty() || s.starts_with(|c: char| c.is_ascii_digit()) {
        s.insert(0, '_');
    }
    if KEYWORDS.contains(&s.as_str()) || RESERVED.contains(&s.as_str()) {
        s.push('_');
    }
    s
}

/// A Python string literal.
pub fn string_literal(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The Python names of everything generated.
struct PyNames {
    /// The package directory.
    package: String,
    /// Module names, by package.
    modules: Vec<String>,
    /// The variables of the `_rt.Package`s in `_model.py`, by package.
    package_vars: Vec<String>,
    /// Names of the classes and enumerations in `_model.py` (unique across packages).
    classes: Vec<String>,
    enums: Vec<String>,
    features: Vec<String>,
    operations: Vec<String>,
    /// `def:` attributes and operations, by class and name (and arity).
    def_attributes: HashMap<(ClassId, String), String>,
    def_operations: HashMap<(ClassId, String, usize), String>,
    /// Invariant check methods, by class and index.
    invariants: HashMap<(ClassId, usize), String>,
    /// The Python base classes, by class (see [`python_bases`]).
    bases: Vec<Vec<ClassId>>,
}

impl PyNames {
    fn new(model: &CodeModel, options: &Options) -> PyNames {
        let package = ident(&snake_case(options.name.as_deref().unwrap_or(&model.name)));
        let mut module_names = Names::new();
        module_names.reserve(["_runtime", "__init__", "_model"]);
        let modules: Vec<String> =
            model.packages.iter().map(|p| module_names.unique(&ident(&snake_case(&p.path.join("_"))))).collect();
        let package_vars = modules.iter().map(|m| format!("_package_{m}")).collect();
        // Classifier names must be unique in `_model.py`: clashing ones get their module's name.
        let simple: Vec<(String, PackageId)> = model
            .classes
            .iter()
            .map(|c| (ident(&c.name), c.package))
            .chain(model.enums.iter().map(|e| (ident(&e.name), e.package)))
            .collect();
        let mut classifier_names = Names::new();
        classifier_names.reserve(["Enum", "EObject", "List", "Optional", "annotations"]);
        let unique: Vec<String> = simple
            .iter()
            .map(|(n, p)| {
                let clash = simple.iter().filter(|(m, _)| m == n).count() > 1;
                classifier_names.unique(&if clash { format!("{n}_{}", modules[p.0]) } else { n.clone() })
            })
            .collect();
        let (classes, enums) = unique.split_at(model.classes.len());
        let mut names = PyNames {
            package,
            modules,
            package_vars,
            classes: classes.to_vec(),
            enums: enums.to_vec(),
            features: vec![String::new(); model.features.len()],
            operations: vec![String::new(); model.operations.len()],
            def_attributes: HashMap::new(),
            def_operations: HashMap::new(),
            invariants: HashMap::new(),
            bases: python_bases(model),
        };
        // Members are named class by class, supertypes first, so that inherited names are
        // known (and kept) when a subclass is named.
        let mut scopes: HashMap<ClassId, Vec<String>> = HashMap::new();
        for c in class_order(model) {
            let class = model.class(c);
            let mut used = Names::new();
            for s in &class.all_supers {
                if let Some(names) = scopes.get(s) {
                    used.reserve(names.iter().map(String::as_str));
                }
            }
            let mut own = Vec::new();
            for &f in &class.features {
                let n = used.unique(&ident(&snake_case(&model.feature(f).name)));
                names.features[f.0] = n.clone();
                own.push(n);
            }
            for d in &class.def_attributes {
                let n = used.unique(&ident(&snake_case(&d.name)));
                names.def_attributes.insert((c, d.name.clone()), n.clone());
                own.push(n);
            }
            for &o in &class.operations {
                let op = model.operation(o);
                // An operation overriding an inherited one keeps its name.
                let inherited = op.overrides.first().map(|x| names.operations[x.0].clone());
                let n = inherited.unwrap_or_else(|| used.unique(&ident(&snake_case(&op.name))));
                names.operations[o.0] = n.clone();
                own.push(n);
            }
            for d in &class.def_operations {
                let n = used.unique(&ident(&snake_case(&d.name)));
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
        names
    }
}

/// The base classes of each class, such that Python can linearize every class (C3).
///
/// Ecore allows any multiple inheritance; Python requires the linearizations of all classes
/// to agree. Every class is linearized in one global order of the classes (subclasses
/// first): its bases are its direct supertypes that are not supertypes of other ones, in
/// that order, if C3 then yields its supertypes in that order; otherwise it lists all its
/// supertypes in that order (C3 can then only follow it, as the bases' linearizations
/// follow it too).
fn python_bases(model: &CodeModel) -> Vec<Vec<ClassId>> {
    let order = class_order(model);
    let rank = |c: &ClassId| std::cmp::Reverse(order.iter().position(|x| x == c).unwrap_or(0));
    let mut bases: Vec<Vec<ClassId>> = vec![Vec::new(); model.classes.len()];
    let mut linearizations: Vec<Vec<ClassId>> = vec![Vec::new(); model.classes.len()];
    for &c in &order {
        let class = model.class(c);
        let mut direct: Vec<ClassId> = class
            .supers
            .iter()
            .copied()
            .filter(|s| !class.supers.iter().any(|o| o != s && model.class(*o).all_supers.contains(s)))
            .collect();
        direct.sort_by_key(rank);
        let lists: Vec<Vec<ClassId>> =
            direct.iter().map(|b| linearizations[b.0].clone()).chain(std::iter::once(direct.clone())).collect();
        let mut all = class.all_supers.clone();
        all.sort_by_key(rank);
        let natural = c3_merge(lists).is_some_and(|l| l == all);
        bases[c.0] = if natural { direct } else { all.clone() };
        linearizations[c.0] = std::iter::once(c).chain(all).collect();
    }
    bases
}

/// C3's merge of linearizations; `None` if they are inconsistent.
fn c3_merge(mut lists: Vec<Vec<ClassId>>) -> Option<Vec<ClassId>> {
    let mut out = Vec::new();
    loop {
        lists.retain(|l| !l.is_empty());
        if lists.is_empty() {
            return Some(out);
        }
        let head = lists.iter().map(|l| l[0]).find(|h| lists.iter().all(|l| !l[1..].contains(h)))?;
        out.push(head);
        for l in &mut lists {
            if l[0] == head {
                l.remove(0);
            }
        }
    }
}

/// Classes with supertypes before subtypes (then in model order).
pub(crate) fn class_order(model: &CodeModel) -> Vec<ClassId> {
    let mut order: Vec<ClassId> = Vec::new();
    fn visit(model: &CodeModel, c: ClassId, order: &mut Vec<ClassId>, stack: &mut Vec<ClassId>) {
        if order.contains(&c) || stack.contains(&c) {
            return;
        }
        stack.push(c);
        for s in &model.class(c).supers {
            visit(model, *s, order, stack);
        }
        stack.pop();
        order.push(c);
    }
    for c in model.class_ids() {
        visit(model, c, &mut order, &mut Vec::new());
    }
    order
}

fn header(model: &CodeModel) -> String {
    format!(
        "Generated by Oxychloride from {}. Do not edit: changes are lost when the code is\ngenerated again.",
        if model.source.is_empty() { "a metamodel" } else { &model.source }
    )
}

fn docstring(w: &mut Writer, text: &str) {
    let text = text.replace("\\", "\\\\").replace("\"\"\"", "\\\"\\\"\\\"");
    let lines: Vec<&str> = text.trim_end().lines().collect();
    match lines.as_slice() {
        [] => {}
        [one] => w.line(format!("\"\"\"{one}\"\"\"")),
        [first, rest @ ..] => {
            w.line(format!("\"\"\"{first}"));
            for l in rest {
                w.line(l.trim_end());
            }
            w.line("\"\"\"");
        }
    }
}

fn init_module(model: &CodeModel, names: &PyNames) -> String {
    let mut w = Writer::new();
    let title = model.packages.iter().map(|p| p.path.join("::")).collect::<Vec<_>>().join(", ");
    docstring(
        &mut w,
        &format!(
            "The {title} metamodel{}: classes, XMI loading and saving, OCL validation.\n\n{}\n\n\
             Load a model and validate it::\n\n    from {pkg} import load, validate\n\n    resource = load(\"model.xmi\")\n    \
             for diagnostic in validate(resource):\n        print(diagnostic)",
            if model.packages.len() > 1 { "s" } else { "" },
            header(model),
            pkg = names.package
        ),
    );
    w.blank();
    w.line("from ._runtime import (");
    w.indent();
    for n in [
        "ContractViolation",
        "Diagnostic",
        "EObject",
        "OclInvalid",
        "Resource",
        "dumps",
        "load",
        "loads",
        "save",
        "set_contract_checking",
        "validate",
    ] {
        w.line(format!("{n},"));
    }
    w.dedent();
    w.line(")");
    // Classifiers whose names are unique across the packages are exported here too; the
    // others through their package's module.
    let mut exported: Vec<String> = Vec::new();
    for i in 0..model.packages.len() {
        let p = model.package(PackageId(i));
        let module = &names.modules[i];
        let mut items: Vec<String> = Vec::new();
        items.extend(
            p.enums
                .iter()
                .filter(|e| names.enums[e.0] == ident(&model.enumeration(**e).name))
                .map(|e| names.enums[e.0].clone()),
        );
        items.extend(
            p.classes
                .iter()
                .filter(|c| names.classes[c.0] == ident(&model.class(**c).name))
                .map(|c| names.classes[c.0].clone()),
        );
        w.line(format!("from . import {module}"));
        if !items.is_empty() {
            w.line(format!("from .{module} import {}", items.join(", ")));
            exported.extend(items);
        }
    }
    w.blank();
    let mut all: Vec<String> = [
        "ContractViolation",
        "Diagnostic",
        "EObject",
        "OclInvalid",
        "Resource",
        "dumps",
        "load",
        "loads",
        "save",
        "set_contract_checking",
        "validate",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    all.extend(exported);
    all.extend(names.modules.iter().cloned());
    w.line("__all__ = [");
    w.indent();
    for n in all {
        w.line(format!("{},", string_literal(&n)));
    }
    w.dedent();
    w.line("]");
    w.finish()
}

/// The module of one package: its documentation, and its classifiers (defined in
/// `_model.py`) under their own names.
fn package_module(model: &CodeModel, names: &PyNames, p: PackageId) -> String {
    let pkg = model.package(p);
    let mut w = Writer::new();
    let mut doc = format!("The {} package", pkg.path.join("::"));
    if !pkg.ns_uri.is_empty() {
        doc.push_str(&format!(" ({})", pkg.ns_uri));
    }
    doc.push_str(".\n\n");
    if let Some(d) = &pkg.doc {
        doc.push_str(d);
        doc.push_str("\n\n");
    }
    doc.push_str(&header(model));
    docstring(&mut w, &doc);
    w.blank();
    let mut items: Vec<String> = Vec::new();
    for &e in &pkg.enums {
        let public = ident(&model.enumeration(e).name);
        let internal = &names.enums[e.0];
        items.push(if *internal == public { public } else { format!("{internal} as {public}") });
    }
    for &c in &pkg.classes {
        let public = ident(&model.class(c).name);
        let internal = &names.classes[c.0];
        items.push(if *internal == public { public } else { format!("{internal} as {public}") });
    }
    w.line(format!("from ._model import {} as PACKAGE", names.package_vars[p.0]));
    if !items.is_empty() {
        w.line("from ._model import (");
        w.indent();
        for i in &items {
            w.line(format!("{i},"));
        }
        w.dedent();
        w.line(")");
    }
    w.blank();
    let all: Vec<String> = std::iter::once("PACKAGE".to_string())
        .chain(items.iter().map(|i| i.rsplit(' ').next().unwrap_or(i).to_string()))
        .collect();
    w.line(format!("__all__ = [{}]", all.iter().map(|n| string_literal(n)).collect::<Vec<_>>().join(", ")));
    w.finish()
}

/// Generates `_model.py`: the classifiers of all packages.
struct Module<'a> {
    model: &'a CodeModel,
    names: &'a PyNames,
    output: &'a mut Output,
}

impl Module<'_> {
    fn generate(mut self) -> String {
        let model = self.model;
        let mut w = Writer::new();
        let title = model.packages.iter().map(|p| p.path.join("::")).collect::<Vec<_>>().join(", ");
        docstring(&mut w, &format!("The classes of {title}.\n\n{}", header(model)));
        w.blank();
        w.line("from __future__ import annotations");
        w.blank();
        w.line("from typing import List, Optional");
        w.blank();
        w.line("from . import _runtime as _rt");
        w.blank();
        for (i, p) in model.packages.iter().enumerate() {
            w.fmt(format_args!(
                "{} = _rt.Package({}, {}, {})",
                self.names.package_vars[i],
                string_literal(&p.name),
                string_literal(&p.ns_uri),
                string_literal(&p.ns_prefix)
            ));
        }
        w.blank_lines(2);
        for e in 0..model.enums.len() {
            self.enumeration(&mut w, EnumId(e));
        }
        for c in class_order(model) {
            self.class(&mut w, c);
        }
        let mut out = w.finish();
        while out.ends_with("\n\n") {
            out.pop();
        }
        out
    }

    fn class_ref(&self, c: ClassId) -> String {
        self.names.classes[c.0].clone()
    }

    fn enum_ref(&self, e: EnumId) -> String {
        self.names.enums[e.0].clone()
    }

    fn package_var(&self, p: PackageId) -> &str {
        &self.names.package_vars[p.0]
    }

    fn enumeration(&mut self, w: &mut Writer, e: EnumId) {
        let en = self.model.enumeration(e);
        let name = &self.names.enums[e.0];
        w.line(format!("class {name}(_rt.Enum):"));
        w.indent();
        docstring(w, en.doc.as_deref().unwrap_or(&format!("The enumeration {}.", en.name)));
        w.blank();
        let values: Vec<i64> = en.literals.iter().map(|l| l.value).collect();
        let distinct = values.iter().enumerate().all(|(i, v)| !values[..i].contains(v));
        if en.literals.is_empty() {
            w.line("pass");
        }
        for (i, l) in en.literals.iter().enumerate() {
            w.line(format!("{} = {}", ident(&l.name), if distinct { l.value } else { i as i64 }));
        }
        w.dedent();
        w.blank_lines(2);
        w.line(format!("{name}._e_name = {}", string_literal(&en.name)));
        let literals: Vec<String> = en
            .literals
            .iter()
            .map(|l| format!("{}: {}", string_literal(&ident(&l.name)), string_literal(&l.literal)))
            .collect();
        w.line(format!("{name}._literals_ = {{{}}}", literals.join(", ")));
        w.line(format!("{}.register({name})", self.package_var(en.package)));
        w.blank_lines(2);
    }

    /// The Python type annotation of a typed element.
    fn annotation(&self, t: &Typed) -> String {
        let one = self.ty_annotation(&t.ty);
        if t.many {
            format!("List[{one}]")
        } else if t.nullable {
            format!("Optional[{one}]")
        } else {
            one
        }
    }

    fn ty_annotation(&self, ty: &Ty) -> String {
        match ty {
            Ty::Boolean => "bool".into(),
            Ty::Integer | Ty::UnlimitedNatural => "int".into(),
            Ty::Real => "float".into(),
            Ty::String | Ty::DataType(_) => "str".into(),
            Ty::Enum(e) => self.enum_ref(*e),
            Ty::Class(c) => self.class_ref(*c),
            Ty::Object => "_rt.EObject".into(),
            Ty::Collection(_, e) => format!("List[{}]", self.ty_annotation(e)),
            Ty::Tuple(_) => "_rt.Tuple".into(),
            _ => "object".into(),
        }
    }

    fn literal(&self, l: &Literal) -> String {
        match l {
            Literal::Boolean(b) => if *b { "True" } else { "False" }.into(),
            Literal::Integer(i) => i.to_string(),
            Literal::Real(r) => real_literal(*r),
            Literal::String(s) => string_literal(s),
            Literal::Enum(e, i) => {
                format!("{}.{}", self.enum_ref(*e), ident(&self.model.enumeration(*e).literals[*i].name))
            }
        }
    }

    fn class(&mut self, w: &mut Writer, c: ClassId) {
        let model = self.model;
        let class = model.class(c);
        let name = &self.names.classes[c.0];
        let supers: Vec<String> = self.names.bases[c.0].iter().map(|s| self.class_ref(*s)).collect();
        let bases = if supers.is_empty() { "_rt.EObject".to_string() } else { supers.join(", ") };
        w.line(format!("class {name}({bases}):"));
        w.indent();
        let mut doc = class.doc.clone().unwrap_or_else(|| {
            format!(
                "{} {}.",
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
                doc.push_str(&format!("\n* {} ({}): {}", inv.name, inv.body.origin, one_line(&inv.body.text)));
            }
        }
        docstring(w, &doc);
        w.blank();
        w.line(format!("_e_package = {}", self.package_var(class.package)));
        w.line(format!("_e_name = {}", string_literal(&class.name)));
        w.line(format!("_e_abstract = {}", if class.is_concrete() { "False" } else { "True" }));
        if !class.invariants.is_empty() {
            w.line("_e_invariants = (");
            w.indent();
            for (i, inv) in class.invariants.iter().enumerate() {
                let message = if inv.message.is_some() {
                    string_literal(&message_method(&self.names.invariants[&(c, i)]))
                } else {
                    "None".into()
                };
                w.line(format!(
                    "({}, {}, {message}),",
                    string_literal(&format!("{}::{}", class.name, inv.name)),
                    string_literal(&self.names.invariants[&(c, i)])
                ));
            }
            w.dedent();
            w.line(")");
        }
        w.blank();
        for &f in &class.features {
            self.feature(w, c, f);
        }
        w.blank();
        for d in &class.def_attributes {
            let n = &self.names.def_attributes[&(c, d.name.clone())];
            w.line("@property");
            w.line(format!("def {n}(self) -> {}:", self.annotation(&d.typed)));
            w.indent();
            docstring(w, &format!("Defined in OCL ({}): {}", d.body.origin, one_line(&d.body.text)));
            let e = self.expr(&d.body.expr, &d.body.origin);
            w.line(format!("return {e}"));
            w.dedent();
            w.blank();
        }
        for &o in &class.operations {
            self.operation(w, c, o);
        }
        for d in &class.def_operations {
            let n = &self.names.def_operations[&(c, d.name.clone(), d.params.len())];
            let params: Vec<String> =
                d.params.iter().map(|p| format!(", {}: {}", ident(&p.name), self.annotation(&p.typed))).collect();
            w.line(format!("def {n}(self{}) -> {}:", params.concat(), self.annotation(&d.typed)));
            w.indent();
            docstring(w, &format!("Defined in OCL ({}): {}", d.body.origin, one_line(&d.body.text)));
            let e = self.expr(&d.body.expr, &d.body.origin);
            w.line(format!("return {e}"));
            w.dedent();
            w.blank();
        }
        for (i, inv) in class.invariants.iter().enumerate() {
            let n = &self.names.invariants[&(c, i)];
            w.line(format!("def {n}(self) -> bool:"));
            w.indent();
            docstring(w, &format!("Invariant {} ({}): {}", inv.name, inv.body.origin, one_line(&inv.body.text)));
            let e = self.expr(&inv.body.expr, &inv.body.origin);
            w.line(format!("return {e}"));
            w.dedent();
            w.blank();
            if let Some(m) = &inv.message {
                w.line(format!("def {}(self) -> str:", message_method(n)));
                w.indent();
                let e = self.expr(m, &inv.body.origin);
                w.line(format!("return _rt.to_string({e})"));
                w.dedent();
                w.blank();
            }
        }
        w.dedent();
        w.blank_lines(2);
        w.line(format!("{}.register({name})", self.package_var(class.package)));
        w.blank_lines(2);
    }

    fn feature(&mut self, w: &mut Writer, c: ClassId, f: FeatureId) {
        let model = self.model;
        let feature = model.feature(f);
        let n = &self.names.features[f.0];
        let t = &feature.typed;
        if !feature.is_stored() && !feature.is_container() {
            // Computed: a read-only property.
            w.blank();
            w.line("@property");
            w.line(format!("def {n}(self) -> {}:", self.annotation(t)));
            w.indent();
            match &feature.derivation {
                Some(body) => {
                    docstring(w, &format!("Derived ({}): {}", body.origin, one_line(&body.text)));
                    let e = self.expr(&body.expr, &body.origin);
                    w.line(format!("return {e}"));
                }
                None => {
                    docstring(w, feature.doc.as_deref().unwrap_or("A volatile feature without an OCL derivation."));
                    w.line(format!(
                        "raise NotImplementedError({})",
                        string_literal(&format!(
                            "{}.{} has no OCL derivation: define it in a subclass",
                            model.class(c).name,
                            feature.name
                        ))
                    ));
                }
            }
            w.dedent();
            w.blank();
            return;
        }
        let kind = match &t.ty {
            Ty::Boolean => "boolean",
            Ty::Integer | Ty::UnlimitedNatural => "integer",
            Ty::Real => "real",
            Ty::String => "string",
            Ty::DataType(_) => "datatype",
            Ty::Enum(_) => "enum",
            _ => "object",
        };
        let mut args: Vec<String> = vec![string_literal(&feature.name)];
        let type_ref = match &t.ty {
            Ty::Enum(e) => Some(self.enum_ref(*e)),
            Ty::Class(cl) => Some(self.class_ref(*cl)),
            Ty::Object => Some("_rt.EObject".into()),
            _ => None,
        };
        if feature.is_reference() {
            args.push(format!("lambda: {}", type_ref.unwrap_or_else(|| "_rt.EObject".into())));
        } else {
            args.push(string_literal(kind));
            if let Some(r) = type_ref {
                args.push(format!("lambda: {r}"));
            }
        }
        if t.many {
            args.push("many=True".into());
        }
        if t.lower != 0 {
            args.push(format!("lower={}", t.lower));
        }
        if t.upper != 1 {
            args.push(format!("upper={}", t.upper));
        }
        if !t.ordered {
            args.push("ordered=False".into());
        }
        if !t.unique {
            args.push("unique=False".into());
        }
        if let Some(d) = &feature.default {
            args.push(format!("default={}", self.literal(d)));
        }
        if let FeatureKind::Reference { containment, container, opposite } = feature.kind {
            if containment {
                args.push("containment=True".into());
            }
            if container {
                args.push("container=True".into());
            }
            if let Some(o) = opposite {
                args.push(format!("opposite={}", string_literal(&model.feature(o).name)));
            }
        }
        if let FeatureKind::Attribute { id: true } = feature.kind {
            args.push("id=True".into());
        }
        if !feature.changeable {
            args.push("changeable=False".into());
        }
        if !feature.is_serialized() {
            args.push("serialized=False".into());
        }
        if let Some(body) = &feature.initial {
            let e = self.expr(&body.expr, &body.origin);
            args.push(format!("initial=lambda self: {e}"));
        }
        let class = if feature.is_reference() { "Reference" } else { "Attribute" };
        w.line(format!("{n} = _rt.{class}({})", args.join(", ")));
        let mut doc = feature.doc.clone().unwrap_or_default();
        if let Some(body) = &feature.initial {
            if !doc.is_empty() {
                doc.push(' ');
            }
            doc.push_str(&format!("Initially ({}): {}", body.origin, one_line(&body.text)));
        }
        if !doc.is_empty() {
            docstring(w, &doc);
            w.blank();
        }
    }

    fn operation(&mut self, w: &mut Writer, c: ClassId, o: OperationId) {
        let model = self.model;
        let op = model.operation(o);
        let n = self.names.operations[o.0].clone();
        let params: Vec<(String, String)> =
            op.params.iter().map(|p| (ident(&p.name), self.annotation(&p.typed))).collect();
        let signature: String = params.iter().map(|(p, t)| format!(", {p}: {t}")).collect();
        let args: String = params.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>().join(", ");
        let result = op.result.as_ref().map_or("None".to_string(), |r| self.annotation(r));
        let qualified = format!("{}::{}()", model.class(c).name, op.name);
        // The contracts of this operation and of those it overrides.
        let contract_ops = model.contract_operations(c, o);
        let has_pre = contract_ops.iter().any(|x| !model.operation(*x).preconditions.is_empty());
        let has_post = contract_ops.iter().any(|x| !model.operation(*x).postconditions.is_empty());
        let mut doc = op.doc.clone().unwrap_or_default();
        if let Some(body) = &op.body {
            doc.push_str(&format!("\n\nOCL body ({}): {}", body.origin, one_line(&body.text)));
        }
        for x in &contract_ops {
            for p in &model.operation(*x).preconditions {
                doc.push_str(&format!("\n\nPrecondition {} ({}): {}", p.name, p.body.origin, one_line(&p.body.text)));
            }
            for p in &model.operation(*x).postconditions {
                doc.push_str(&format!("\n\nPostcondition {} ({}): {}", p.name, p.body.origin, one_line(&p.body.text)));
            }
        }
        if has_pre || has_post {
            doc.push_str(
                "\n\nThe conditions are checked unless turned off with set_contract_checking(False); \
                 a violation raises ContractViolation.",
            );
        }
        let simple = !has_pre && !has_post && op.body.is_some();
        w.line(format!("def {n}(self{signature}) -> {result}:"));
        w.indent();
        docstring(w, doc.trim_start());
        if simple {
            let body = op.body.as_ref().unwrap();
            let e = self.expr(&body.expr, &body.origin);
            w.line(format!("return {e}"));
        } else {
            if has_pre {
                w.line("if _rt.contracts_enabled():");
                w.indent();
                w.line(format!("self._pre_{n}({args})"));
                w.dedent();
            }
            w.line(format!("result = self._impl_{n}({args})"));
            if has_post {
                w.line("if _rt.contracts_enabled():");
                w.indent();
                w.line(format!("self._post_{n}({args}{}result)", if args.is_empty() { "" } else { ", " }));
                w.dedent();
            }
            w.line("return result");
        }
        w.dedent();
        w.blank();
        if simple {
            return;
        }
        w.line(format!("def _impl_{n}(self{signature}) -> {result}:"));
        w.indent();
        match &op.body {
            Some(body) => {
                let e = self.expr(&body.expr, &body.origin);
                w.line(format!("return {e}"));
            }
            None => {
                docstring(w, "Override this method (in a subclass) to implement the operation.");
                w.line(format!(
                    "raise NotImplementedError({})",
                    string_literal(&format!("{qualified} has no OCL body"))
                ));
            }
        }
        w.dedent();
        w.blank();
        for (kind, post) in [("pre", false), ("post", true)] {
            if (!post && !has_pre) || (post && !has_post) {
                continue;
            }
            let extra = if post { format!(", result: {result}") } else { String::new() };
            w.line(format!("def _{kind}_{n}(self{signature}{extra}) -> None:"));
            w.indent();
            for x in &contract_ops {
                let xop = model.operation(*x);
                let conditions = if post { &xop.postconditions } else { &xop.preconditions };
                let xname = format!("{}::{}()", model.class(xop.owner).name, xop.name);
                for cond in conditions {
                    // Parameters of an overridden operation may be named differently.
                    let rename: Vec<(String, String)> = xop
                        .params
                        .iter()
                        .zip(&op.params)
                        .filter(|(a, b)| a.name != b.name)
                        .map(|(a, b)| (a.name.clone(), ident(&b.name)))
                        .collect();
                    let mut printer = PyExpr::new(self, &cond.body.origin);
                    printer.renamed = rename;
                    let e = printer.expr(&cond.body.expr);
                    let m = cond.message.as_ref().map(|m| {
                        let mut printer = PyExpr::new(self, &cond.body.origin);
                        printer.expr(m)
                    });
                    w.line(format!(
                        "_rt.check_condition({}, self, {}, {}, lambda: {e}{})",
                        string_literal(if post { "postcondition" } else { "precondition" }),
                        string_literal(&xname),
                        string_literal(&cond.name),
                        m.map(|m| format!(", lambda: {m}")).unwrap_or_default()
                    ));
                }
            }
            w.dedent();
            w.blank();
        }
    }

    fn expr(&mut self, e: &Expr, origin: &str) -> String {
        PyExpr::new(self, origin).expr(e)
    }
}

/// The method computing the message of the invariant checked by `check`.
fn message_method(check: &str) -> String {
    format!("_message_{}", check.strip_prefix("check_").unwrap_or(check))
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn real_literal(r: f64) -> String {
    if r.is_infinite() {
        return if r > 0.0 { "_rt.math.inf".into() } else { "-_rt.math.inf".into() };
    }
    let s = format!("{r:?}");
    if s.contains(['.', 'e', 'E', 'n']) { s } else { format!("{s}.0") }
}

/// Translates an expression into Python.
struct PyExpr<'m, 'a> {
    module: &'m mut Module<'a>,
    origin: String,
    /// OCL parameter names to use under another name (for inherited contracts).
    renamed: Vec<(String, String)>,
}

impl<'m, 'a> PyExpr<'m, 'a> {
    fn new(module: &'m mut Module<'a>, origin: &str) -> Self {
        PyExpr { module, origin: origin.to_string(), renamed: Vec::new() }
    }

    fn model(&self) -> &'a CodeModel {
        self.module.model
    }

    fn unsupported(&mut self, what: &str) -> String {
        let origin = self.origin.clone();
        self.module.output.warn(format!("Python: {what} is not supported; it raises OclInvalid"), Some(&origin));
        format!("_rt.invalid({})", string_literal(&format!("{what} is not supported")))
    }

    /// `e` as an operand: parenthesized unless it is atomic.
    fn atom(&mut self, e: &Expr) -> String {
        let s = self.expr(e);
        if is_atomic(&s) { s } else { format!("({s})") }
    }

    /// A type as a value, for `oclIsKindOf` and friends: a class, an enumeration, or the name
    /// of a primitive type.
    fn type_value(&mut self, t: &Ty) -> String {
        match t {
            Ty::Class(c) => self.module.class_ref(*c),
            Ty::Enum(e) => self.module.enum_ref(*e),
            Ty::Object => "_rt.EObject".into(),
            Ty::Boolean => string_literal("Boolean"),
            Ty::Integer => string_literal("Integer"),
            Ty::Real => string_literal("Real"),
            Ty::String | Ty::DataType(_) => string_literal("String"),
            Ty::UnlimitedNatural => string_literal("UnlimitedNatural"),
            Ty::Any => string_literal("OclAny"),
            Ty::Void => string_literal("OclVoid"),
            _ => self.unsupported("this type as an argument"),
        }
    }

    fn type_arg(&mut self, args: &[Expr]) -> String {
        match args.first().map(|a| &a.kind) {
            Some(ExprKind::Type(t)) => {
                let t = t.clone();
                self.type_value(&t)
            }
            _ => self.unsupported("a type computed at run time"),
        }
    }

    fn expr(&mut self, e: &Expr) -> String {
        let model = self.model();
        match &e.kind {
            ExprKind::Boolean(b) => if *b { "True" } else { "False" }.into(),
            ExprKind::Integer(i) => i.to_string(),
            ExprKind::Real(r) => real_literal(*r),
            ExprKind::String(s) => string_literal(s),
            ExprKind::Unlimited => "_rt.UNLIMITED".into(),
            ExprKind::Null => "None".into(),
            ExprKind::Invalid(reason) => format!("_rt.invalid({})", string_literal(reason)),
            ExprKind::EnumLiteral(en, i) => {
                format!("{}.{}", self.module.enum_ref(*en), ident(&model.enumeration(*en).literals[*i].name))
            }
            ExprKind::Type(t) => {
                let t = t.clone();
                self.type_value(&t)
            }
            ExprKind::SelfRef => "self".into(),
            ExprKind::Result => "result".into(),
            ExprKind::Parameter(p) => {
                self.renamed.iter().find(|(from, _)| from == p).map(|(_, to)| to.clone()).unwrap_or_else(|| ident(p))
            }
            ExprKind::Var(v) => ident(v),
            ExprKind::Feature { source, feature, safe } => {
                let attr = self.module.names.features[feature.0].clone();
                self.member(source, &attr, *safe)
            }
            ExprKind::DefAttribute { source, class, name, safe } => {
                let attr = match model.def_attribute(*class, name) {
                    Some((owner, _)) => self.module.names.def_attributes[&(owner, name.clone())].clone(),
                    None => return self.unsupported(&format!("the definition '{name}'")),
                };
                self.member(source, &attr, *safe)
            }
            ExprKind::TuplePart { source, name } => format!("{}.{}", self.atom(source), ident(name)),
            ExprKind::Call { source, operation, args, safe } => {
                let method = self.module.names.operations[operation.0].clone();
                self.call(source, &method, args, *safe)
            }
            ExprKind::DefCall { source, class, name, args, safe } => {
                let method = match model.def_operation(*class, name, args.len()) {
                    Some((owner, _)) => self.module.names.def_operations[&(owner, name.clone(), args.len())].clone(),
                    None => return self.unsupported(&format!("the definition '{name}'")),
                };
                self.call(source, &method, args, *safe)
            }
            ExprKind::Library { name, source, args, safe } => {
                if *safe {
                    let inner = Expr::new(
                        ExprKind::Library {
                            name: name.clone(),
                            source: Box::new(Expr::new(ExprKind::Var("_s".into()), source.ty.clone())),
                            args: args.clone(),
                            safe: false,
                        },
                        e.ty.clone(),
                    );
                    let s = self.expr(source);
                    let body = self.expr(&inner);
                    return format!("(lambda _s: None if _s is None else {body})({s})");
                }
                self.library(name, source, args, &e.ty)
            }
            ExprKind::Iterator { kind, source, vars, body } => self.iterator(*kind, source, vars, body),
            ExprKind::Iterate { source, var, acc, init, body } => {
                let s = self.expr(source);
                let i = self.expr(init);
                let b = self.expr(body);
                format!("_rt.iterate({s}, {i}, lambda {}, {}: {b})", ident(&var.name), ident(&acc.name))
            }
            ExprKind::AllInstances(t) => match t {
                Ty::Class(c) => format!("_rt.all_instances(self, {})", self.module.class_ref(*c)),
                Ty::Enum(en) => format!("list({})", self.module.enum_ref(*en)),
                Ty::Boolean => "[True, False]".into(),
                _ => self.unsupported("allInstances() of this type"),
            },
            ExprKind::Binary { op, lhs, rhs } => self.binary(*op, lhs, rhs),
            ExprKind::Unary { op, operand } => {
                let o = self.atom(operand);
                match op {
                    UnOp::Not => format!("not {o}"),
                    UnOp::Neg => format!("-{o}"),
                }
            }
            ExprKind::If { cond, then_branch, else_branch } => {
                let c = self.atom(cond);
                let t = self.atom(then_branch);
                let f = self.atom(else_branch);
                format!("{t} if {c} else {f}")
            }
            ExprKind::Let { var, init, body } => {
                let i = self.expr(init);
                let b = self.expr(body);
                format!("(lambda {}: {b})({i})", ident(&var.name))
            }
            ExprKind::CollectionLiteral { kind, items } => {
                let mut parts = Vec::new();
                let mut simple = true;
                for item in items {
                    match item {
                        CollectionItem::Single(x) => parts.push(self.expr(x)),
                        CollectionItem::Range(a, b) => {
                            simple = false;
                            let (a, b) = (self.expr(a), self.expr(b));
                            parts.push(format!("*_rt.ocl_range({a}, {b})"));
                        }
                    }
                }
                let list = format!("[{}]", parts.join(", "));
                if kind.is_unique() && (parts.len() > 1 || !simple) { format!("_rt.unique({list})") } else { list }
            }
            ExprKind::TupleLiteral(parts) => {
                let parts: Vec<String> = parts.iter().map(|(n, v)| format!("{}={}", ident(n), self.expr(v))).collect();
                format!("_rt.Tuple({})", parts.join(", "))
            }
        }
    }

    fn member(&mut self, source: &Expr, attr: &str, safe: bool) -> String {
        let s = self.atom(source);
        if safe { format!("_rt.safe_get({s}, {})", string_literal(attr)) } else { format!("{s}.{attr}") }
    }

    fn call(&mut self, source: &Expr, method: &str, args: &[Expr], safe: bool) -> String {
        let s = self.atom(source);
        let args: Vec<String> = args.iter().map(|a| self.expr(a)).collect();
        if safe {
            let mut all = vec![s, string_literal(method)];
            all.extend(args);
            format!("_rt.safe_call({})", all.join(", "))
        } else {
            format!("{s}.{method}({})", args.join(", "))
        }
    }

    /// Whether evaluating `e` can fail (so that `oclIsUndefined` must catch it).
    fn may_fail(e: &Expr) -> bool {
        !matches!(
            &e.kind,
            ExprKind::SelfRef
                | ExprKind::Result
                | ExprKind::Parameter(_)
                | ExprKind::Var(_)
                | ExprKind::Null
                | ExprKind::Boolean(_)
                | ExprKind::Integer(_)
                | ExprKind::Real(_)
                | ExprKind::String(_)
        ) && !matches!(&e.kind, ExprKind::Feature { source, .. } if matches!(source.kind, ExprKind::SelfRef))
    }

    fn library(&mut self, name: &str, source: &Expr, args: &[Expr], ty: &Ty) -> String {
        // Operations of any value first: they apply to collections and null too.
        match name {
            "oclIsUndefined" => {
                return if Self::may_fail(source) {
                    format!("_rt.is_undefined(lambda: {})", self.expr(source))
                } else {
                    format!("{} is None", self.atom(source))
                };
            }
            "oclIsInvalid" => return format!("_rt.is_invalid(lambda: {})", self.expr(source)),
            "oclIsKindOf" | "oclIsTypeOf" | "oclAsType" => {
                let s = self.expr(source);
                let t = self.type_arg(args);
                let f = match name {
                    "oclIsKindOf" => "is_kind_of",
                    "oclIsTypeOf" => "is_type_of",
                    _ => "as_type",
                };
                return format!("_rt.{f}({s}, {t})");
            }
            "oclAsSet" => return format!("_rt.as_set({})", self.expr(source)),
            "oclType" => return format!("type({})", self.expr(source)),
            "toString" => return format!("_rt.to_string({})", self.expr(source)),
            "oclContainer" => return format!("{}.e_container()", self.atom(source)),
            "oclContents" => return format!("{}.e_contents()", self.atom(source)),
            "eAllContents" => return format!("{}.e_all_contents()", self.atom(source)),
            _ => {}
        }
        let s = self.atom(source);
        let a: Vec<String> = args.iter().map(|x| self.expr(x)).collect();
        let arg = |i: usize| a.get(i).cloned().unwrap_or_else(|| "None".into());
        if let Ty::Collection(kind, _) = &source.ty {
            let unique = if kind.is_unique() { "True" } else { "False" };
            return match name {
                "size" => format!("len({s})"),
                "count" => format!("{s}.count({})", arg(0)),
                "includes" => format!("{} in {s}", self.atom(&args[0])),
                "excludes" => format!("{} not in {s}", self.atom(&args[0])),
                "includesAll" => format!("_rt.includes_all({s}, {})", arg(0)),
                "excludesAll" => format!("_rt.excludes_all({s}, {})", arg(0)),
                "isEmpty" => format!("len({s}) == 0"),
                "notEmpty" => format!("len({s}) > 0"),
                "sum" => format!("_rt.coll_sum({s})"),
                "max" => format!("_rt.coll_max({s})"),
                "min" => format!("_rt.coll_min({s})"),
                "asSet" | "asOrderedSet" => format!("_rt.unique({s})"),
                "asSequence" | "asBag" => format!("list({s})"),
                "flatten" => {
                    if kind.is_unique() {
                        format!("_rt.unique(_rt.flatten({s}))")
                    } else {
                        format!("_rt.flatten({s})")
                    }
                }
                "including" => format!("_rt.including({s}, {}, {unique})", arg(0)),
                "excluding" => format!("_rt.excluding({s}, {})", arg(0)),
                "includingAll" => format!("_rt.including_all({s}, {}, {unique})", arg(0)),
                "excludingAll" => format!("_rt.excluding_all({s}, {})", arg(0)),
                "union" => {
                    let result_unique = ty.collection_kind().is_some_and(|k| k.is_unique());
                    format!("_rt.including_all({s}, {}, {})", arg(0), if result_unique { "True" } else { "False" })
                }
                "intersection" => {
                    let bag = ty.collection_kind() == Some(CollectionKind::Bag);
                    format!("_rt.intersection({s}, {}, {})", arg(0), if bag { "True" } else { "False" })
                }
                "symmetricDifference" => format!("_rt.symmetric_difference({s}, {})", arg(0)),
                "append" => format!("_rt.append({s}, {}, {unique})", arg(0)),
                "prepend" => format!("_rt.prepend({s}, {}, {unique})", arg(0)),
                "insertAt" => format!("_rt.insert_at({s}, {}, {}, {unique})", arg(0), arg(1)),
                "at" => format!("_rt.at({s}, {})", arg(0)),
                "first" => format!("_rt.first({s})"),
                "last" => format!("_rt.last({s})"),
                "indexOf" => format!("_rt.index_of_element({s}, {})", arg(0)),
                "subSequence" => format!("_rt.sub_sequence({s}, {}, {})", arg(0), arg(1)),
                "reverse" => format!("list(reversed({s}))"),
                "selectByKind" | "selectByType" => {
                    let t = self.type_arg(args);
                    let f = if name == "selectByKind" { "select_by_kind" } else { "select_by_type" };
                    format!("_rt.{f}({s}, {t})")
                }
                "product" => format!("_rt.product({s}, {})", arg(0)),
                other => self.unsupported(&format!("the collection operation '{other}'")),
            };
        }
        match (&source.ty, name) {
            (Ty::String | Ty::DataType(_), _) => match name {
                "size" => format!("len({s})"),
                "concat" => format!("{s} + {}", self.atom(&args[0])),
                "substring" => format!("_rt.substring({s}, {}, {})", arg(0), arg(1)),
                "toInteger" => format!("_rt.to_integer({s})"),
                "toReal" => format!("_rt.to_real({s})"),
                "toBoolean" => format!("_rt.to_boolean({s})"),
                "toUpper" => format!("{s}.upper()"),
                "toLower" => format!("{s}.lower()"),
                "trim" => format!("{s}.strip()"),
                "indexOf" => format!("_rt.index_of({s}, {})", arg(0)),
                "lastIndexOf" => format!("_rt.last_index_of({s}, {})", arg(0)),
                "at" => format!("_rt.string_at({s}, {})", arg(0)),
                "characters" => format!("list({s})"),
                "equalsIgnoreCase" => format!("{s}.lower() == {}.lower()", self.atom(&args[0])),
                "startsWith" => format!("{s}.startswith({})", arg(0)),
                "endsWith" => format!("{s}.endswith({})", arg(0)),
                "matches" => format!("_rt.matches({s}, {})", arg(0)),
                "replaceAll" => format!("_rt.replace_all({s}, {}, {})", arg(0), arg(1)),
                "replaceFirst" => format!("_rt.replace_first({s}, {}, {})", arg(0), arg(1)),
                "substituteAll" => format!("{s}.replace({}, {})", arg(0), arg(1)),
                "substituteFirst" => format!("{s}.replace({}, {}, 1)", arg(0), arg(1)),
                "tokenize" => format!("_rt.tokenize({})", std::iter::once(s).chain(a).collect::<Vec<_>>().join(", ")),
                other => self.unsupported(&format!("the String operation '{other}'")),
            },
            (Ty::Integer | Ty::Real | Ty::UnlimitedNatural, _) => match name {
                "abs" => format!("abs({s})"),
                "floor" => format!("_rt.floor({s})"),
                "round" => format!("_rt.round_({s})"),
                "max" => format!("_rt.number_max({s}, {})", arg(0)),
                "min" => format!("_rt.number_min({s}, {})", arg(0)),
                "div" => format!("_rt.div({s}, {})", arg(0)),
                "mod" => format!("_rt.mod({s}, {})", arg(0)),
                "toInteger" => format!("int({s})"),
                "toReal" => format!("float({s})"),
                other => self.unsupported(&format!("the number operation '{other}'")),
            },
            (_, other) => self.unsupported(&format!("the operation '{other}' on this type")),
        }
    }

    fn iterator(&mut self, kind: IteratorKind, source: &Expr, vars: &[Var], body: &Expr) -> String {
        let s = self.expr(source);
        let b = self.expr(body);
        let v = ident(&vars[0].name);
        match kind {
            IteratorKind::Select => format!("[{v} for {v} in {s} if {}]", paren(&b)),
            IteratorKind::Reject => format!("[{v} for {v} in {s} if not {}]", paren(&b)),
            IteratorKind::Collect => {
                if body.ty.is_collection() {
                    format!("_rt.flatten([{b} for {v} in {s}])")
                } else {
                    format!("[{b} for {v} in {s}]")
                }
            }
            IteratorKind::CollectNested => format!("[{b} for {v} in {s}]"),
            IteratorKind::ForAll | IteratorKind::Exists => {
                let f = if kind == IteratorKind::ForAll { "all" } else { "any" };
                if vars.len() == 1 {
                    format!("{f}({b} for {v} in {s})")
                } else {
                    let fors: Vec<String> = vars.iter().map(|x| format!("for {} in _s", ident(&x.name))).collect();
                    format!("(lambda _s: {f}({b} {}))({s})", fors.join(" "))
                }
            }
            IteratorKind::Any => format!("_rt.any_({s}, lambda {v}: {b})"),
            IteratorKind::One => format!("_rt.one({s}, lambda {v}: {b})"),
            IteratorKind::IsUnique => format!("_rt.is_unique({b} for {v} in {s})"),
            IteratorKind::SortedBy => format!("_rt.sorted_by({s}, lambda {v}: {b})"),
            IteratorKind::Closure => format!("_rt.closure({s}, lambda {v}: {b})"),
        }
    }

    fn binary(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr) -> String {
        let l = self.atom(lhs);
        let r = self.atom(rhs);
        match op {
            BinOp::And => format!("{l} and {r}"),
            BinOp::Or => format!("{l} or {r}"),
            BinOp::Implies => format!("not {l} or {r}"),
            BinOp::Xor => format!("{l} != {r}"),
            BinOp::Eq | BinOp::Ne => {
                let eq = match (&lhs.ty, &rhs.ty) {
                    (Ty::Collection(k1, _), Ty::Collection(k2, _)) => {
                        if k1 != k2 && *k1 != CollectionKind::Collection && *k2 != CollectionKind::Collection {
                            "False".to_string()
                        } else {
                            match k1 {
                                CollectionKind::Set => format!("_rt.set_equals({l}, {r})"),
                                CollectionKind::Bag => format!("_rt.bag_equals({l}, {r})"),
                                _ => format!("list({l}) == list({r})"),
                            }
                        }
                    }
                    (_, Ty::Void) | (Ty::Void, _) => format!("{l} is {r}"),
                    _ => format!("{l} == {r}"),
                };
                if op == BinOp::Eq {
                    eq
                } else if let Some(rest) = eq.strip_suffix(&format!(" is {r}")) {
                    format!("{rest} is not {r}")
                } else if let Some(rest) = eq.strip_suffix(&format!(" == {r}")) {
                    format!("{rest} != {r}")
                } else {
                    format!("not {}", paren(&eq))
                }
            }
            BinOp::Lt => format!("{l} < {r}"),
            BinOp::Gt => format!("{l} > {r}"),
            BinOp::Le => format!("{l} <= {r}"),
            BinOp::Ge => format!("{l} >= {r}"),
            BinOp::Add => format!("{l} + {r}"),
            BinOp::Sub if lhs.ty.is_collection() => format!("_rt.difference({l}, {r})"),
            BinOp::Sub => format!("{l} - {r}"),
            BinOp::Mul => format!("{l} * {r}"),
            BinOp::Div => format!("_rt.divide({l}, {r})"),
        }
    }
}

/// Whether a Python expression needs no parentheses as an operand.
fn is_atomic(s: &str) -> bool {
    // Names, attribute accesses, calls, subscripts, literals and displays.
    let mut depth = 0i32;
    let mut in_string: Option<char> = None;
    let mut escaped = false;
    for c in s.chars() {
        if let Some(q) = in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                in_string = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => in_string = Some(c),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ' ' if depth == 0 => return false,
            '-' if depth == 0 => return false,
            _ => {}
        }
    }
    true
}

fn paren(s: &str) -> String {
    if is_atomic(s) { s.to_string() } else { format!("({s})") }
}
