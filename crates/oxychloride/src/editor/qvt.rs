//! Hovers and completion for QVT documents (`.qvto`, `.qvtr`), from the compiled document:
//! the types its checker recorded (for `expr.` and `expr->`), the variables of the body at the
//! cursor, the transformation's declarations and model types, and the keywords.

use oxy_ocl::Type;
use oxy_qvt::Compiled;
use oxy_qvt::library::{ELEMENT_OPERATIONS, MODEL_OPERATIONS, STRING_OPERATIONS};

use super::{OCL_KEYWORDS, arrow_items, collect_types, item, last_word, signature_items};
use crate::protocol::*;
use crate::workspace::Workspace;

const QVTO_KEYWORDS: &[&str] = &[
    "import",
    "library",
    "access",
    "extends",
    "transformation",
    "modeltype",
    "uses",
    "in",
    "out",
    "inout",
    "main",
    "mapping",
    "query",
    "helper",
    "property",
    "configuration",
    "intermediate",
    "init",
    "population",
    "end",
    "when",
    "where",
    "inherits",
    "merges",
    "disjuncts",
    "abstract",
    "object",
    "map",
    "xmap",
    "resolve",
    "resolveone",
    "resolveIn",
    "resolveoneIn",
    "invresolve",
    "invresolveone",
    "late",
    "var",
    "while",
    "elif",
    "switch",
    "case",
    "return",
    "break",
    "continue",
    "log",
    "assert",
    "new",
    "this",
    "result",
    "compute",
    "do",
    "List",
];

const QVTR_KEYWORDS: &[&str] = &[
    "import",
    "transformation",
    "key",
    "top",
    "relation",
    "abstract",
    "overrides",
    "checkonly",
    "enforce",
    "domain",
    "primitive",
    "when",
    "where",
    "query",
    "opposite",
    "default_values",
];

/// Iterators and operations of Imperative OCL after `->`.
const QVT_ARROW: &[(&str, &str, &str)] = &[
    ("xselect", "xselect(x | condition)", "collection"),
    ("xcollect", "xcollect(x | expression)", "Sequence/Bag"),
    ("selectOne", "selectOne(x | condition)", "element"),
    ("collectOne", "collectOne(x | expression)", "value"),
    ("forEach", "forEach(x) { }", "statement"),
    ("forOne", "forOne(x | condition) { }", "statement"),
    ("add", "add(x)", "statement"),
];

impl Workspace {
    fn compiled_qvt(&self, name: &str) -> Option<Compiled> {
        let ocl = self.compile(true, None);
        self.compile_qvt(name, &ocl).ok()
    }

    /// The declaration at `offset`, else the type of the expression there.
    pub(super) fn hover_qvt(&self, name: &str, text: &str, offset: usize) -> Option<HoverInfo> {
        let compiled = self.compiled_qvt(name)?;
        let offset32 = offset as u32;
        if let Some(s) = compiled
            .symbols
            .iter()
            .find(|s| s.span.start <= offset32 && offset32 <= s.span.end && s.span.end > s.span.start)
        {
            return Some(HoverInfo { range: self.text_range(text, &s.span.range()), contents: s.detail.clone() });
        }
        // The name of a mapping, helper, query or relation where it is called.
        let start = text[..offset].rfind(|c: char| !(c.is_alphanumeric() || c == '_')).map_or(0, |i| i + 1);
        let end = text[offset..].find(|c: char| !(c.is_alphanumeric() || c == '_')).map_or(text.len(), |i| offset + i);
        let word = &text[start..end];
        if text[end..].trim_start().starts_with('(')
            && let Some(s) = compiled.operations().into_iter().find(|s| s.name == word)
        {
            return Some(HoverInfo { range: self.text_range(text, &(start..end)), contents: s.detail.clone() });
        }
        if let Some((span, ty)) = compiled.type_at(offset32) {
            let mm = self.metamodel();
            let contents = match ty {
                Type::Meta(t) => format!("type {}", t.display(mm)),
                t => t.display(mm).to_string(),
            };
            return Some(HoverInfo { range: self.text_range(text, &span.range()), contents });
        }
        let s = compiled.operations().into_iter().find(|s| s.name == word)?;
        Some(HoverInfo { range: self.text_range(text, &(start..end)), contents: s.detail.clone() })
    }

    pub(super) fn complete_qvt(
        &self,
        name: &str,
        text: &str,
        word_start: usize,
        language: Language,
    ) -> Vec<CompletionItem> {
        let Some(compiled) = self.compiled_qvt(name) else { return Vec::new() };
        let mm = self.metamodel();
        let ocl = self.compile(true, None);
        let head = &text[..word_start];
        let mut items = Vec::new();
        let operations = |items: &mut Vec<CompletionItem>, kinds: &[&str]| {
            for s in compiled.operations().into_iter().filter(|s| kinds.contains(&s.kind)) {
                let mut i = item(&s.name, CompletionKind::Operation, Some(s.detail.clone()));
                i.insert = Some(format!("{}()", s.name));
                items.push(i);
            }
        };
        if let Some(op_pos) = head.strip_suffix("->").map(str::len) {
            let ty = compiled.type_ending_at(op_pos as u32).cloned().unwrap_or(Type::Error);
            arrow_items(&mut items, &ty);
            if language == Language::Qvto {
                for (n, sig, result) in QVT_ARROW {
                    let mut i = item(n, CompletionKind::Iterator, Some(result.to_string()));
                    i.insert = Some(sig.to_string());
                    items.push(i);
                }
                items.push(item("map", CompletionKind::Keyword, None));
            }
        } else if let Some(dot_pos) = head.strip_suffix('.').filter(|h| !h.ends_with('.')).map(str::len) {
            let ty = compiled.type_ending_at(dot_pos as u32).cloned().unwrap_or(Type::Error);
            match &ty {
                Type::Model(_) => {
                    let ops: Vec<(&str, &str)> = MODEL_OPERATIONS.iter().map(|(_, s, r)| (*s, *r)).collect();
                    signature_items(&mut items, &ops, CompletionKind::Operation);
                    // `this.`: the transformation's properties.
                    for s in compiled.symbols.iter().filter(|s| s.kind == "property") {
                        items.push(item(&s.name, CompletionKind::Property, Some(s.detail.clone())));
                    }
                }
                _ => {
                    self.dot_items(&mut items, &ocl.env, &ty);
                    let extra = match &ty {
                        Type::String => STRING_OPERATIONS,
                        Type::Classifier(c) if mm.is_class(*c) => ELEMENT_OPERATIONS,
                        _ => &[],
                    };
                    let ops: Vec<(&str, &str)> = extra.iter().map(|(_, s, r)| (*s, *r)).collect();
                    signature_items(&mut items, &ops, CompletionKind::Operation);
                    operations(&mut items, &["helper", "query"]);
                    if language == Language::Qvto {
                        for k in
                            ["map", "xmap", "resolve", "resolveone", "resolveIn", "resolveoneIn", "invresolve", "late"]
                        {
                            items.push(item(k, CompletionKind::Keyword, None));
                        }
                    }
                }
            }
        } else if let Some(qualifier) = head.strip_suffix("::") {
            let word = last_word(qualifier);
            if let Some(alias) = compiled.aliases().iter().find(|a| a.name == word) {
                for p in &alias.packages {
                    collect_types(mm, *p, &mut items);
                }
            }
            for c in mm.find_classifiers(word) {
                for l in mm.enum_literals(c) {
                    items.push(item(&l.name, CompletionKind::EnumLiteral, Some(mm.classifier(c).name.clone())));
                }
                // `Class::mapping` (in `resolveIn`, `inherits`, ...).
                operations(&mut items, &["mapping"]);
            }
            for p in mm.find_packages_qualified(&[word]) {
                for c in &mm.package(p).classifiers {
                    items.push(item(&mm.classifier(*c).name, CompletionKind::Type, None));
                }
            }
        } else if matches!(last_word(head.trim_end()), "map" | "xmap") && head.ends_with(char::is_whitespace) {
            operations(&mut items, &["mapping"]);
        } else {
            let offset = word_start as u32;
            if let Some(scope) = compiled.scope_at(offset) {
                for (n, t) in scope.variables.iter().filter(|(n, _)| !n.starts_with('_')) {
                    items.push(item(n, CompletionKind::Variable, Some(t.display(mm).to_string())));
                }
                if let Some(t) = &scope.self_type {
                    self.dot_items(&mut items, &ocl.env, t);
                    items.retain(|i| i.kind != CompletionKind::Operation || !i.label.starts_with("ocl"));
                }
            }
            for s in &compiled.symbols {
                let kind = match s.kind {
                    "parameter" => CompletionKind::Variable,
                    "modeltype" | "import" => CompletionKind::Type,
                    "mapping" | "helper" | "query" | "relation" => CompletionKind::Operation,
                    "property" => CompletionKind::Property,
                    _ => continue,
                };
                items.push(item(&s.name, kind, Some(s.detail.clone())));
            }
            let keywords = if language == Language::Qvto { QVTO_KEYWORDS } else { QVTR_KEYWORDS };
            for k in keywords.iter().chain(OCL_KEYWORDS) {
                items.push(item(k, CompletionKind::Keyword, None));
            }
            for t in super::LIBRARY_TYPES {
                items.push(item(t, CompletionKind::Type, None));
            }
            for a in compiled.aliases() {
                for p in &a.packages {
                    collect_types(mm, *p, &mut items);
                }
            }
        }
        items
    }
}
