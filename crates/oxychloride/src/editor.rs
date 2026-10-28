//! Editor support: hovers and completion for Complete OCL, OCLinEcore, HUTN, Henshin and QVT
//! documents.

use oxy_ecore::{ClassifierId, ElementRef, Metamodel};
use oxy_ocl::env::SourceKind;
use oxy_ocl::span::Position as OclPosition;
use oxy_ocl::{SourceId, Type};

use crate::protocol::*;
use crate::workspace::{Compiled, Workspace, range};

mod henshin;
mod hutn;
mod oclinecore;
mod qvt;

/// Keywords of OCL expressions.
const OCL_KEYWORDS: &[&str] = &[
    "and", "or", "xor", "not", "implies", "if", "then", "else", "endif", "let", "in", "true", "false", "null",
    "invalid", "self",
];

/// Keywords of Complete OCL documents.
const OCL_DOCUMENT_KEYWORDS: &[&str] =
    &["context", "inv", "def", "pre", "post", "body", "init", "derive", "package", "endpackage", "import"];

const LIBRARY_TYPES: &[&str] = &[
    "Boolean",
    "Integer",
    "Real",
    "String",
    "UnlimitedNatural",
    "OclAny",
    "OclVoid",
    "OclInvalid",
    "Set",
    "OrderedSet",
    "Bag",
    "Sequence",
    "Collection",
    "Tuple",
];

/// `(signature, result)` of operations available on every value.
const OCLANY_OPS: &[(&str, &str)] = &[
    ("oclIsUndefined()", "Boolean"),
    ("oclIsInvalid()", "Boolean"),
    ("oclIsKindOf(type)", "Boolean"),
    ("oclIsTypeOf(type)", "Boolean"),
    ("oclAsType(type)", "type"),
    ("oclType()", "type"),
    ("oclAsSet()", "Set"),
    ("oclContainer()", "OclAny"),
    ("oclContents()", "OrderedSet(OclAny)"),
    ("toString()", "String"),
];

const NUMERIC_OPS: &[(&str, &str)] = &[
    ("abs()", "Integer/Real"),
    ("floor()", "Integer"),
    ("round()", "Integer"),
    ("max(n)", "Integer/Real"),
    ("min(n)", "Integer/Real"),
    ("div(i)", "Integer"),
    ("mod(i)", "Integer"),
];

const STRING_OPS: &[(&str, &str)] = &[
    ("size()", "Integer"),
    ("concat(s)", "String"),
    ("substring(lower, upper)", "String"),
    ("toInteger()", "Integer"),
    ("toReal()", "Real"),
    ("toBoolean()", "Boolean"),
    ("toUpperCase()", "String"),
    ("toLowerCase()", "String"),
    ("trim()", "String"),
    ("indexOf(s)", "Integer"),
    ("lastIndexOf(s)", "Integer"),
    ("at(i)", "String"),
    ("characters()", "Sequence(String)"),
    ("equalsIgnoreCase(s)", "Boolean"),
    ("startsWith(s)", "Boolean"),
    ("endsWith(s)", "Boolean"),
    ("matches(regex)", "Boolean"),
    ("replaceAll(regex, replacement)", "String"),
    ("replaceFirst(regex, replacement)", "String"),
    ("tokenize(delimiters)", "Sequence(String)"),
];

const COLLECTION_OPS: &[(&str, &str)] = &[
    ("size()", "Integer"),
    ("isEmpty()", "Boolean"),
    ("notEmpty()", "Boolean"),
    ("includes(x)", "Boolean"),
    ("excludes(x)", "Boolean"),
    ("includesAll(c)", "Boolean"),
    ("excludesAll(c)", "Boolean"),
    ("count(x)", "Integer"),
    ("sum()", "element"),
    ("max()", "element"),
    ("min()", "element"),
    ("first()", "element"),
    ("last()", "element"),
    ("at(i)", "element"),
    ("indexOf(x)", "Integer"),
    ("asSet()", "Set"),
    ("asOrderedSet()", "OrderedSet"),
    ("asSequence()", "Sequence"),
    ("asBag()", "Bag"),
    ("flatten()", "collection"),
    ("including(x)", "collection"),
    ("excluding(x)", "collection"),
    ("union(c)", "collection"),
    ("intersection(c)", "collection"),
    ("append(x)", "collection"),
    ("prepend(x)", "collection"),
    ("reverse()", "collection"),
    ("selectByKind(type)", "collection"),
    ("selectByType(type)", "collection"),
];

const ITERATORS: &[(&str, &str, &str)] = &[
    ("select", "select(x | condition)", "collection"),
    ("reject", "reject(x | condition)", "collection"),
    ("collect", "collect(x | expression)", "Bag/Sequence"),
    ("forAll", "forAll(x | condition)", "Boolean"),
    ("exists", "exists(x | condition)", "Boolean"),
    ("any", "any(x | condition)", "element"),
    ("one", "one(x | condition)", "Boolean"),
    ("isUnique", "isUnique(x | expression)", "Boolean"),
    ("sortedBy", "sortedBy(x | expression)", "OrderedSet/Sequence"),
    ("closure", "closure(x | expression)", "Set/OrderedSet"),
    ("collectNested", "collectNested(x | expression)", "Bag/Sequence"),
    ("iterate", "iterate(x; acc : T = init | expression)", "T"),
];

/// Names an OCL expression can use besides the global ones.
#[derive(Default)]
struct OclScope {
    /// The type of `self` (whose features are also available without `self.`).
    class: Option<ClassifierId>,
    /// Further variables (parameters, `result`) and their types.
    variables: Vec<(String, Type)>,
}

impl Workspace {
    fn offset_in(&self, text: &str, position: Position) -> usize {
        oxy_ocl::LineIndex::new(text).offset(text, OclPosition { line: position.line, column: position.column })
    }

    /// Hover information at `position` in an open document: the type of the OCL expression
    /// there, or (in OCLinEcore and HUTN documents) a description of the element or feature.
    pub fn hover(&self, document: &str, position: Position) -> Option<HoverInfo> {
        let language = self.document_language(document)?;
        let text = self.document_text(document)?;
        let offset = self.offset_in(text, position);
        match language {
            Language::Hutn => return self.hover_hutn(document, text, offset),
            Language::Henshin | Language::HenshinModel => return self.hover_henshin(document, text, language, offset),
            Language::Qvto | Language::Qvtr => return self.hover_qvt(document, text, offset),
            // XMI is XML: the XML tooling (with the generated schemas) covers it.
            Language::Xmi => return None,
            Language::Ecore => return self.hover_ecore(document, text, offset),
            Language::Ocl | Language::OclInEcore => {}
        }
        let compiled = self.compile(true, Some(&[document.to_string()]));
        let src = compiled.source_of(document)?;
        if let Some((span, ty)) = compiled.env.type_at(src, offset as u32) {
            let source = compiled.env.source(src);
            let contents = match ty {
                Type::Meta(t) => format!("type {}", t.display(self.metamodel())),
                t => t.display(self.metamodel()).to_string(),
            };
            return Some(HoverInfo { range: range(source, span), contents });
        }
        if language == Language::OclInEcore {
            return self.hover_oclinecore(document, text, offset);
        }
        None
    }

    /// The qualified name of the class whose definition in an open OCLinEcore document contains
    /// `position` (anywhere in it, including its features and constraints).
    pub fn class_at(&self, document: &str, position: Position) -> Option<String> {
        if self.document_language(document)? != Language::OclInEcore {
            return None;
        }
        let offset = self.offset_in(self.document_text(document)?, position) as u32;
        let mm = self.metamodel();
        self.oclinecore_document(document)?
            .elements
            .iter()
            .filter_map(|(e, s)| match e {
                ElementRef::Classifier(c) if mm.is_class(*c) && s.start <= offset && offset <= s.end => Some((*c, s)),
                _ => None,
            })
            .min_by_key(|(_, s)| s.end - s.start)
            .map(|(c, _)| mm.qualified_name(c))
    }

    /// Completion proposals at `position` in an open document.
    pub fn complete(&self, document: &str, position: Position) -> Option<CompletionList> {
        let language = self.document_language(document)?;
        let text = self.document_text(document)?;
        let offset = self.offset_in(text, position);
        let word_start = text[..offset]
            .char_indices()
            .rev()
            .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
            .last()
            .map_or(offset, |(i, _)| i);
        let items = match language {
            Language::Ocl => {
                let compiled = self.compile(true, Some(&[document.to_string()]));
                let src = compiled.source_of(document)?;
                let scope =
                    OclScope { class: context_class(self.metamodel(), &text[..word_start]), variables: Vec::new() };
                let mut items = self.ocl_items(&compiled, src, text, word_start, &scope);
                let head = &text[..word_start];
                if !(head.ends_with('.') || head.ends_with("->") || head.ends_with("::")) {
                    items.extend(OCL_DOCUMENT_KEYWORDS.iter().map(|k| item(k, CompletionKind::Keyword, None)));
                }
                items
            }
            Language::OclInEcore => self.complete_oclinecore(document, text, offset, word_start)?,
            Language::Hutn => {
                let (items, start) = self.complete_hutn(document, text, word_start)?;
                return Some(finish_completion(text, start, offset, items));
            }
            Language::Henshin => self.complete_henshin(text, word_start),
            Language::Qvto | Language::Qvtr => self.complete_qvt(document, text, word_start, language),
            Language::Ecore => {
                let compiled = self.compile(true, Some(&[]));
                let (src, at, _) = self.embedded_at(&compiled, document, text, offset)?;
                let source = compiled.env.source(src);
                let start = word_start_in(&source.text, at);
                let scope = self.embedded_scope(&compiled.env, src);
                let items = self.ocl_items(&compiled, src, &source.text, start, &scope);
                // Identifiers are written as they are: the word starts as far before the cursor.
                return Some(finish_completion(text, offset - (at - start), offset, items));
            }
            // `.henshin` files are XML: they are edited as rule diagrams.
            Language::Xmi | Language::HenshinModel => return None,
        };
        Some(finish_completion(text, word_start, offset, items))
    }

    /// The OCL embedded in an open `.ecore` document at `offset` (in the `value` of an
    /// annotation's `<details>`): its source, the offset in the source's (unescaped) text, and
    /// the offset of the `<details>` tag.
    fn embedded_at(
        &self,
        compiled: &Compiled,
        document: &str,
        text: &str,
        offset: usize,
    ) -> Option<(SourceId, usize, usize)> {
        let uri = crate::ecore_documents::ecore_document_uri(document);
        compiled.env.sources().find_map(|(id, source)| match &source.kind {
            SourceKind::Annotation { resource, offset: Some(tag), .. } if *resource == uri => {
                crate::ecore_documents::value_offset(text, *tag, offset).map(|at| (id, at, *tag))
            }
            _ => None,
        })
    }

    /// The type of the OCL expression at `offset` in an `.ecore` document's annotations.
    fn hover_ecore(&self, document: &str, text: &str, offset: usize) -> Option<HoverInfo> {
        let compiled = self.compile(true, Some(&[]));
        let (src, at, tag) = self.embedded_at(&compiled, document, text, offset)?;
        let (span, ty) = compiled.env.type_at(src, at as u32)?;
        let contents = match ty {
            Type::Meta(t) => format!("type {}", t.display(self.metamodel())),
            t => t.display(self.metamodel()).to_string(),
        };
        let bytes = crate::ecore_documents::value_range(text, tag, span.start as usize..span.end as usize)?;
        let index = oxy_ocl::LineIndex::new(text);
        let p = |o: usize| {
            let p = index.position(text, o);
            Position { line: p.line, column: p.column }
        };
        Some(HoverInfo { range: Range { start: p(bytes.start), end: p(bytes.end) }, contents })
    }

    /// What an embedded expression may use: `self` (the class whose element it annotates), and
    /// an operation's parameters (and `result` in postconditions).
    fn embedded_scope(&self, env: &oxy_ocl::Environment, src: SourceId) -> OclScope {
        if let Some(inv) = env.invariants().iter().find(|i| i.body.source == src) {
            return OclScope { class: Some(inv.context), variables: Vec::new() };
        }
        if let Some(c) = env.operation_constraints().iter().find(|c| c.body.source == src) {
            let mut variables = c.params.clone();
            if c.kind == oxy_ocl::syntax::ast::ConstraintKind::Post {
                variables.push(("result".to_string(), c.result.clone()));
            }
            return OclScope { class: Some(c.class), variables };
        }
        // Derivations, initial values and operation bodies: `<class>::<feature>`, `<class>::<op>()`.
        let mm = self.metamodel();
        let SourceKind::Annotation { element, .. } = &env.source(src).kind else {
            return OclScope { class: None, variables: Vec::new() };
        };
        let (class, member) = element.rsplit_once("::").unwrap_or((element, ""));
        let path: Vec<&str> = class.split("::").collect();
        let class = mm.find_classifiers_qualified(&path).into_iter().next();
        let mut variables = Vec::new();
        if let (Some(c), Some(op)) = (class, member.strip_suffix("()")) {
            if let Some(&o) = mm.all_operations(c).iter().find(|o| mm.operation(**o).name == op) {
                for p in &mm.operation(o).params {
                    variables.push((p.name.clone(), Type::of_typed(mm, p.ty, &p.multiplicity)));
                }
            }
        }
        OclScope { class, variables }
    }

    /// Completions inside an OCL expression: after `.`, `->` and `::`, or names in `scope`,
    /// keywords and types.
    fn ocl_items(
        &self,
        compiled: &Compiled,
        src: SourceId,
        text: &str,
        word_start: usize,
        scope: &OclScope,
    ) -> Vec<CompletionItem> {
        let head = &text[..word_start];
        let mm = self.metamodel();
        let mut items = Vec::new();
        if let Some(op_pos) = head.strip_suffix("->").map(str::len).or_else(|| head.strip_suffix("?->").map(str::len)) {
            let ty = compiled.env.type_ending_at(src, op_pos as u32).cloned().unwrap_or(Type::Error);
            arrow_items(&mut items, &ty);
        } else if let Some(dot_pos) = head.strip_suffix('.').filter(|h| !h.ends_with('.')).map(str::len) {
            let dot_pos = if head[..dot_pos].ends_with('?') { dot_pos - 1 } else { dot_pos };
            let ty = compiled.env.type_ending_at(src, dot_pos as u32).cloned().unwrap_or(Type::Error);
            self.dot_items(&mut items, &compiled.env, &ty);
        } else if let Some(qualifier) = head.strip_suffix("::") {
            let name = last_word(qualifier);
            for c in mm.find_classifiers(name) {
                for l in mm.enum_literals(c) {
                    items.push(item(&l.name, CompletionKind::EnumLiteral, Some(mm.classifier(c).name.clone())));
                }
            }
            for p in mm.find_packages_qualified(&[name]) {
                for c in &mm.package(p).classifiers {
                    items.push(item(&mm.classifier(*c).name, CompletionKind::Type, None));
                }
            }
        } else {
            for (name, ty) in &scope.variables {
                items.push(item(name, CompletionKind::Variable, Some(ty.display(mm).to_string())));
            }
            if let Some(class) = scope.class {
                let self_ty = Type::Classifier(class);
                self.dot_items(&mut items, &compiled.env, &self_ty);
                items.retain(|i| i.kind != CompletionKind::Operation || !i.label.starts_with("ocl"));
            }
            for k in OCL_KEYWORDS {
                items.push(item(k, CompletionKind::Keyword, None));
            }
            for t in LIBRARY_TYPES {
                items.push(item(t, CompletionKind::Type, None));
            }
            for p in mm.user_root_packages().collect::<Vec<_>>() {
                collect_types(mm, p, &mut items);
            }
        }
        items
    }

    fn dot_items(&self, items: &mut Vec<CompletionItem>, env: &oxy_ocl::Environment, ty: &Type) {
        let mm = self.metamodel();
        match ty {
            Type::Classifier(c) if mm.is_class(*c) => {
                for &f in mm.all_features(*c) {
                    let feature = mm.feature(f);
                    let t = Type::of_typed(mm, feature.ty, &feature.multiplicity);
                    items.push(item(&feature.name, CompletionKind::Property, Some(t.display(mm).to_string())));
                }
                for &o in mm.all_operations(*c) {
                    let op = mm.operation(o);
                    let params: Vec<String> = op.params.iter().map(|p| p.name.clone()).collect();
                    let t = Type::of_typed(mm, op.ty, &op.multiplicity);
                    let mut i = item(&op.name, CompletionKind::Operation, Some(t.display(mm).to_string()));
                    i.insert = Some(format!("{}({})", op.name, params.join(", ")));
                    items.push(i);
                }
                for d in env.def_attributes().filter(|d| mm.conforms_to(*c, d.class)) {
                    items.push(item(&d.name, CompletionKind::Property, Some(d.ty.display(mm).to_string())));
                }
                for d in env.def_operations().filter(|d| mm.conforms_to(*c, d.class)) {
                    let params: Vec<&str> = d.params.iter().map(|(n, _)| n.as_str()).collect();
                    let mut i = item(&d.name, CompletionKind::Operation, Some(d.ty.display(mm).to_string()));
                    i.insert = Some(format!("{}({})", d.name, params.join(", ")));
                    items.push(i);
                }
                signature_items(items, OCLANY_OPS, CompletionKind::Operation);
            }
            Type::Integer | Type::Real | Type::UnlimitedNatural => {
                signature_items(items, NUMERIC_OPS, CompletionKind::Operation);
                signature_items(items, OCLANY_OPS, CompletionKind::Operation);
            }
            Type::String => {
                signature_items(items, STRING_OPS, CompletionKind::Operation);
                signature_items(items, OCLANY_OPS, CompletionKind::Operation);
            }
            Type::Tuple(parts) => {
                for (n, t) in parts.iter() {
                    items.push(item(n, CompletionKind::Property, Some(t.display(mm).to_string())));
                }
            }
            Type::Meta(_) => {
                let mut i = item("allInstances", CompletionKind::Operation, Some("Set".into()));
                i.insert = Some("allInstances()".into());
                items.push(i);
            }
            Type::Collection(_, elem) => {
                // Implicit collect: properties of the elements.
                self.dot_items(items, env, elem);
                items.retain(|i| i.kind == CompletionKind::Property);
            }
            _ => signature_items(items, OCLANY_OPS, CompletionKind::Operation),
        }
    }
}

fn arrow_items(items: &mut Vec<CompletionItem>, ty: &Type) {
    for (name, sig, result) in ITERATORS {
        let mut i = item(name, CompletionKind::Iterator, Some(result.to_string()));
        i.insert = Some(sig.to_string());
        items.push(i);
    }
    signature_items(items, COLLECTION_OPS, CompletionKind::Operation);
    if matches!(ty, Type::Collection(k, _) if k.is_ordered()) {
        signature_items(
            items,
            &[("subSequence(lower, upper)", "collection"), ("insertAt(i, x)", "collection")],
            CompletionKind::Operation,
        );
    }
}

fn signature_items(items: &mut Vec<CompletionItem>, ops: &[(&str, &str)], kind: CompletionKind) {
    for (sig, result) in ops {
        let name = sig.split('(').next().unwrap_or(sig);
        let mut i = item(name, kind, Some(result.to_string()));
        i.insert = Some(sig.to_string());
        items.push(i);
    }
}

fn item(label: &str, kind: CompletionKind, detail: Option<String>) -> CompletionItem {
    CompletionItem { label: label.to_string(), kind, detail, insert: None }
}

fn collect_types(mm: &Metamodel, p: oxy_ecore::PackageId, items: &mut Vec<CompletionItem>) {
    for c in &mm.package(p).classifiers {
        items.push(item(&mm.classifier(*c).name, CompletionKind::Type, Some(mm.package_qualified_name(p))));
    }
    for s in &mm.package(p).subpackages {
        collect_types(mm, *s, items);
    }
}

/// The class of the nearest preceding `context` declaration.
fn context_class(mm: &Metamodel, head: &str) -> Option<oxy_ecore::ClassifierId> {
    let pos = head.rfind("context")?;
    let rest = &head[pos + "context".len()..];
    let mut words = rest.split(|c: char| c.is_whitespace() || c == '(').filter(|w| !w.is_empty());
    let mut name = words.next()?;
    if let Some(next) = words.next()
        && (next == ":" || name.ends_with(':'))
    {
        // `context b : Book` – the class follows the colon.
        name = if next == ":" { words.next()? } else { next };
    }
    let path: Vec<&str> = name.split("::").filter(|s| !s.is_empty()).collect();
    for len in (1..=path.len()).rev() {
        if let Some(c) = mm.find_classifiers_qualified(&path[..len]).first()
            && mm.is_class(*c)
        {
            return Some(*c);
        }
    }
    None
}

/// The identifier at the end of `text`.
fn last_word(text: &str) -> &str {
    let start = text
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
        .last()
        .map_or(text.len(), |(i, _)| i);
    &text[start..]
}

/// Filters proposals by the typed prefix (`text[start..offset]`) and removes duplicates.
/// Where the identifier before `offset` starts.
fn word_start_in(text: &str, offset: usize) -> usize {
    text[..offset]
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
        .last()
        .map_or(offset, |(i, _)| i)
}

fn finish_completion(text: &str, start: usize, offset: usize, mut items: Vec<CompletionItem>) -> CompletionList {
    let lower = text[start..offset].to_lowercase();
    items.retain(|i| i.label.to_lowercase().starts_with(&lower));
    let mut seen = std::collections::HashSet::new();
    items.retain(|i| seen.insert((i.label.clone(), i.kind)));
    let index = oxy_ocl::LineIndex::new(text);
    let p = |o: usize| {
        let p = index.position(text, o);
        Position { line: p.line, column: p.column }
    };
    CompletionList { range: Range { start: p(start), end: p(offset) }, items }
}
