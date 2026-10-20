//! Completion and hovers for OCLinEcore documents.
//!
//! A forward scan over the tokens before the cursor tells whether the cursor is inside an
//! embedded OCL expression (then OCL completion applies, with `self` typed by the enclosing
//! class) or in the metamodel structure (then keywords, type names and qualifiers apply).

use oxy_ecore::{ClassifierId, ElementRef, Metamodel};
use oxy_ocl::Type;
use oxy_ocl::syntax::ast::{ClassifierDefKind, EcoreDocument, TypeRef};
use oxy_ocl::syntax::lexer::{TokenKind, tokenize};
use oxy_ocl::syntax::{FEATURE_QUALIFIERS, OPERATION_QUALIFIERS};

use super::{OclScope, item};
use crate::protocol::*;
use crate::workspace::Workspace;

/// Keywords that start an embedded constraint.
const CONSTRAINT_WORDS: &[&str] = &[
    "invariant",
    "inv",
    "precondition",
    "pre",
    "postcondition",
    "post",
    "body",
    "derivation",
    "derive",
    "initial",
    "init",
];

const PRIMITIVE_TYPES: &[&str] = &["Boolean", "Integer", "Real", "String", "UnlimitedNatural"];

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Block {
    Package,
    Class,
    DataType,
    Enum,
    Feature,
    Operation,
    Other,
}

/// What surrounds the cursor.
struct Context {
    /// Enclosing braces, outermost first.
    blocks: Vec<Block>,
    /// Tokens of the statement the cursor is in, up to the cursor.
    statement: Vec<TokenKind>,
    /// Inside the OCL expression of a constraint.
    in_expression: bool,
    /// The constraint keyword of the statement, if it is a constraint.
    constraint: Option<String>,
}

fn word(t: &TokenKind) -> Option<&str> {
    match t {
        TokenKind::Ident(w) => Some(w),
        _ => None,
    }
}

/// The kind of block a statement opens with `{`.
fn block_kind(statement: &[TokenKind]) -> Block {
    let first =
        statement.iter().filter_map(word).find(|w| !matches!(*w, "abstract" | "primitive" | "static" | "definition"));
    match first {
        Some("package") => Block::Package,
        Some("class" | "interface") => Block::Class,
        Some("datatype") => Block::DataType,
        Some("enum") => Block::Enum,
        Some("attribute" | "property") => Block::Feature,
        Some("operation") => Block::Operation,
        _ => Block::Other,
    }
}

fn scan(text: &str) -> Context {
    let (tokens, _) = tokenize(text);
    let mut blocks: Vec<(Block, Vec<TokenKind>)> = Vec::new();
    let mut statement: Vec<TokenKind> = Vec::new();
    // Set right after a `}`: a following `{` continues the same element (qualifiers, then body).
    let mut closed: Option<Vec<TokenKind>> = None;
    let mut expression_depth: Option<i32> = None;
    let mut constraint: Option<String> = None;
    for t in tokens.iter().filter(|t| t.kind != TokenKind::Eof) {
        if let Some(depth) = expression_depth.as_mut() {
            match t.kind {
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => *depth += 1,
                TokenKind::RParen | TokenKind::RBracket => *depth -= 1,
                TokenKind::RBrace => *depth -= 1,
                TokenKind::Semi if *depth == 0 => *depth = -1,
                _ => {}
            }
            if *depth >= 0 {
                continue;
            }
            expression_depth = None;
            match t.kind {
                // The end of a constraint message: the expression follows the colon.
                TokenKind::RParen => {
                    statement.push(TokenKind::RParen);
                    continue;
                }
                TokenKind::Semi => {
                    statement.clear();
                    constraint = None;
                    continue;
                }
                _ => {} // `}` closing the element: handled below
            }
        }
        if let Some(opener) = closed.take() {
            if t.kind == TokenKind::LBrace {
                blocks.push((block_kind(&opener), opener));
                continue;
            }
            statement.clear();
        }
        match &t.kind {
            TokenKind::Semi => {
                statement.clear();
                constraint = None;
            }
            TokenKind::LBrace => {
                let opener = std::mem::take(&mut statement);
                blocks.push((block_kind(&opener), opener));
                constraint = None;
            }
            TokenKind::RBrace => {
                closed = blocks.pop().map(|(_, opener)| opener);
                statement.clear();
                constraint = None;
            }
            TokenKind::Ident(w)
                if CONSTRAINT_WORDS.contains(&w.as_str()) && statement.iter().all(|t| word(t) == Some("callable")) =>
            {
                constraint = Some(w.clone());
                statement.push(t.kind.clone());
            }
            TokenKind::Colon | TokenKind::LParen if constraint.is_some() && !statement.contains(&TokenKind::Colon) => {
                // The expression (or, for `(`, the message) starts.
                if t.kind == TokenKind::Colon || statement.len() >= 2 {
                    expression_depth = Some(0);
                }
                statement.push(t.kind.clone());
            }
            other => statement.push(other.clone()),
        }
    }
    Context {
        blocks: blocks.into_iter().map(|(b, _)| b).collect(),
        statement,
        in_expression: expression_depth.is_some(),
        constraint,
    }
}

/// Whether the cursor is inside a `--` comment (a heuristic that ignores strings).
fn in_line_comment(text: &str) -> bool {
    let line = &text[text.rfind('\n').map_or(0, |i| i + 1)..];
    let mut in_string = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if in_string => {
                chars.next();
            }
            '\'' => in_string = !in_string,
            '-' if !in_string && chars.peek() == Some(&'-') => return true,
            _ => {}
        }
    }
    false
}

impl Workspace {
    pub(super) fn complete_oclinecore(
        &self,
        document: &str,
        text: &str,
        offset: usize,
        word_start: usize,
    ) -> Option<Vec<CompletionItem>> {
        let head = &text[..word_start];
        if in_line_comment(head) {
            return Some(Vec::new());
        }
        let context = scan(head);
        if context.in_expression {
            let compiled = self.compile(true, Some(&[]));
            let src = compiled.source_of(document)?;
            let scope = self.expression_scope(document, offset, context.constraint.as_deref());
            return Some(self.ocl_items(&compiled, src, text, word_start, &scope));
        }
        Some(self.structure_items(document, &context, head))
    }

    /// `self`, parameters and `result` for an expression in the element at `offset`.
    fn expression_scope(&self, document: &str, offset: usize, constraint: Option<&str>) -> OclScope {
        let mm = self.metamodel();
        let Some(built) = self.oclinecore_document(document) else { return OclScope::default() };
        match built.element_at(offset as u32) {
            Some(ElementRef::Classifier(c)) => OclScope { class: Some(c), variables: Vec::new() },
            Some(ElementRef::Feature(f)) => OclScope { class: Some(mm.feature(f).owner), variables: Vec::new() },
            Some(ElementRef::Operation(o)) => {
                let op = mm.operation(o);
                let mut variables: Vec<(String, Type)> =
                    op.params.iter().map(|p| (p.name.clone(), Type::of_typed(mm, p.ty, &p.multiplicity))).collect();
                if matches!(constraint, Some("postcondition" | "post")) && op.ty.is_some() {
                    variables.push(("result".into(), Type::of_typed(mm, op.ty, &op.multiplicity)));
                }
                OclScope { class: Some(op.owner), variables }
            }
            _ => OclScope::default(),
        }
    }

    fn structure_items(&self, document: &str, context: &Context, head: &str) -> Vec<CompletionItem> {
        let mm = self.metamodel();
        let mut items = Vec::new();
        let keywords = |items: &mut Vec<CompletionItem>, words: &[&str]| {
            for w in words {
                items.push(item(w, CompletionKind::Keyword, None));
            }
        };
        let statement = &context.statement;
        let last = statement.last();
        let first = statement.iter().find_map(word);
        if let Some(qualifier) = head.trim_end().strip_suffix("::") {
            // `ecore::` or `pkg::`: the classifiers of that package.
            let name = super::last_word(qualifier);
            for p in mm.find_packages_qualified(&[name]) {
                for c in &mm.package(p).classifiers {
                    items.push(item(&mm.classifier(*c).name, CompletionKind::Type, Some(mm.package_qualified_name(p))));
                }
            }
            return items;
        }
        let qualifier_list =
            statement.iter().all(|t| matches!(t, TokenKind::Ident(_) | TokenKind::Comma | TokenKind::Bang));
        if !statement.is_empty() && qualifier_list {
            // Inside `{ ordered, ... }` after a feature or operation.
            match context.blocks.last() {
                Some(Block::Feature) => keywords(&mut items, FEATURE_QUALIFIERS),
                Some(Block::Operation) => keywords(&mut items, OPERATION_QUALIFIERS),
                _ => {}
            }
            if !items.is_empty() {
                return items;
            }
        }
        if statement.is_empty() {
            match context.blocks.last() {
                None => keywords(&mut items, &["import", "package", "module"]),
                Some(Block::Package) => keywords(
                    &mut items,
                    &["class", "abstract", "interface", "enum", "datatype", "package", "annotation", "documentation"],
                ),
                Some(Block::Class) => keywords(
                    &mut items,
                    &["attribute", "property", "operation", "invariant", "annotation", "documentation"],
                ),
                Some(Block::Feature) => {
                    keywords(&mut items, &["derivation", "initial", "annotation"]);
                    keywords(&mut items, FEATURE_QUALIFIERS);
                }
                Some(Block::Operation) => {
                    keywords(&mut items, &["precondition", "postcondition", "body", "annotation"]);
                    keywords(&mut items, OPERATION_QUALIFIERS);
                }
                Some(Block::Enum) => keywords(&mut items, &["literal"]),
                Some(Block::DataType) => keywords(&mut items, &["serializable"]),
                Some(Block::Other) => {}
            }
            return items;
        }
        let in_header = matches!(first, Some("class" | "interface" | "abstract"));
        match last {
            Some(TokenKind::Colon) if matches!(first, Some("attribute" | "property" | "operation")) => {
                self.type_items(document, &mut items, false);
            }
            Some(TokenKind::Ident(w)) if (w == "extends" && in_header) || w == "throws" => {
                self.type_items(document, &mut items, true);
            }
            Some(TokenKind::Comma) if in_header && statement.iter().any(|t| word(t) == Some("extends")) => {
                self.type_items(document, &mut items, true);
            }
            Some(TokenKind::Ident(_) | TokenKind::RParen | TokenKind::RBracket)
                if first == Some("class") || first == Some("abstract") =>
            {
                if !statement.iter().any(|t| word(t) == Some("extends")) {
                    keywords(&mut items, &["extends"]);
                }
            }
            Some(TokenKind::RParen | TokenKind::Ident(_) | TokenKind::RBracket) if first == Some("operation") => {
                keywords(&mut items, &["throws"]);
            }
            _ if first == Some("abstract") && statement.len() == 1 => keywords(&mut items, &["class"]),
            _ => {}
        }
        items
    }

    /// Type names usable in the document: OCL primitive types and collection types (unless
    /// only classes are wanted), the classifiers of loaded packages and Ecore's data types.
    fn type_items(&self, document: &str, items: &mut Vec<CompletionItem>, classes_only: bool) {
        let mm = self.metamodel();
        if !classes_only {
            for t in PRIMITIVE_TYPES {
                items.push(item(t, CompletionKind::Type, Some("primitive type".into())));
            }
            for k in ["Set", "OrderedSet", "Sequence", "Bag"] {
                let mut i = item(k, CompletionKind::Type, Some("collection type".into()));
                i.insert = Some(format!("{k}()"));
                items.push(i);
            }
        }
        let own = self.oclinecore_document(document).map(|b| b.resource);
        for c in mm.classifier_ids() {
            let cl = mm.classifier(c);
            if classes_only && !mm.is_class(c) {
                continue;
            }
            let package = cl.package;
            if package == mm.ecore_package() {
                if classes_only {
                    continue;
                }
                let mut i = item(&format!("ecore::{}", cl.name), CompletionKind::Type, Some("Ecore data type".into()));
                if !mm.is_class(c) {
                    i.label = cl.name.clone();
                    i.insert = Some(format!("ecore::{}", cl.name));
                    items.push(i);
                }
                continue;
            }
            let mut root = package;
            while let Some(p) = mm.package(root).super_package {
                root = p;
            }
            let detail = if Some(mm.package(root).resource) == own {
                kind_name(mm, c).to_string()
            } else {
                format!("{} {}", kind_name(mm, c), mm.package_qualified_name(package))
            };
            items.push(item(&cl.name, CompletionKind::Type, Some(detail)));
        }
    }

    /// Hover on the name of a defined element or on a type reference.
    pub(super) fn hover_oclinecore(&self, document: &str, text: &str, offset: usize) -> Option<HoverInfo> {
        let built = self.oclinecore_document(document)?;
        let mm = self.metamodel();
        let offset = offset as u32;
        let contains = |s: oxy_ocl::Span| s.start <= offset && offset <= s.end;
        // Type references resolve against the metamodel.
        let mut found: Option<(oxy_ocl::Span, String)> = None;
        visit_types(&built.document, &mut |t: &TypeRef| {
            if contains(t.span) {
                let names: Vec<&str> = t.path.iter().map(|i| i.name.as_str()).collect();
                let description = match names.as_slice() {
                    [single] if PRIMITIVE_TYPES.contains(single) => format!("primitive type {single}"),
                    _ => match mm.find_classifiers_qualified(&names).first() {
                        Some(c) => describe(mm, *c),
                        None => return,
                    },
                };
                found = Some((t.span, description));
            }
        });
        if found.is_none() {
            // The element whose definition starts at a name under the cursor.
            let element = built.element_at(offset)?;
            let description = match element {
                ElementRef::Classifier(c) => describe(mm, c),
                ElementRef::Feature(f) => {
                    let feature = mm.feature(f);
                    let ty = Type::of_typed(mm, feature.ty, &feature.multiplicity);
                    let kind = if feature.is_containment() {
                        "containment"
                    } else if feature.is_reference() {
                        "reference"
                    } else {
                        "attribute"
                    };
                    format!("{kind} {}::{} : {}", mm.classifier(feature.owner).name, feature.name, ty.display(mm))
                }
                ElementRef::Operation(o) => {
                    let op = mm.operation(o);
                    let params: Vec<String> = op
                        .params
                        .iter()
                        .map(|p| format!("{} : {}", p.name, Type::of_typed(mm, p.ty, &p.multiplicity).display(mm)))
                        .collect();
                    let result = if op.ty.is_some() {
                        format!(" : {}", Type::of_typed(mm, op.ty, &op.multiplicity).display(mm))
                    } else {
                        String::new()
                    };
                    format!("operation {}::{}({}){result}", mm.classifier(op.owner).name, op.name, params.join(", "))
                }
                ElementRef::Package(p) => {
                    format!("package {} = '{}'", mm.package_qualified_name(p), mm.package(p).ns_uri)
                }
            };
            // Only on the element's name, not anywhere in its body.
            let word_start = text[..offset as usize]
                .char_indices()
                .rev()
                .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
                .last()
                .map_or(offset as usize, |(i, _)| i);
            let word_end = offset as usize
                + text[offset as usize..]
                    .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .unwrap_or(text.len() - offset as usize);
            let name = match element {
                ElementRef::Classifier(c) => mm.classifier(c).name.clone(),
                ElementRef::Feature(f) => mm.feature(f).name.clone(),
                ElementRef::Operation(o) => mm.operation(o).name.clone(),
                ElementRef::Package(p) => mm.package(p).name.clone(),
            };
            if text[word_start..word_end] != name {
                return None;
            }
            found = Some((oxy_ocl::Span::new(word_start, word_end), description));
        }
        let (span, contents) = found?;
        let index = oxy_ocl::LineIndex::new(text);
        let p = |o: u32| {
            let p = index.position(text, o as usize);
            Position { line: p.line, column: p.column }
        };
        Some(HoverInfo { range: Range { start: p(span.start), end: p(span.end) }, contents })
    }
}

fn kind_name(mm: &Metamodel, c: ClassifierId) -> &'static str {
    match mm.class_data(c) {
        Some(d) if d.is_interface => "interface",
        Some(d) if d.is_abstract => "abstract class",
        Some(_) => "class",
        None if mm.is_enum(c) => "enum",
        None => "datatype",
    }
}

fn describe(mm: &Metamodel, c: ClassifierId) -> String {
    let supers: Vec<String> = mm.super_types(c).iter().map(|s| mm.classifier(*s).name.clone()).collect();
    let extends = if supers.is_empty() { String::new() } else { format!(" extends {}", supers.join(", ")) };
    format!("{} {}{extends}", kind_name(mm, c), mm.qualified_name(c))
}

/// Calls `f` for every type reference in the document.
fn visit_types(doc: &EcoreDocument, f: &mut dyn FnMut(&TypeRef)) {
    fn package(p: &oxy_ocl::syntax::ast::PackageDef, f: &mut dyn FnMut(&TypeRef)) {
        for c in &p.classifiers {
            if let ClassifierDefKind::Class { supers, features, operations, .. } = &c.kind {
                supers.iter().for_each(&mut *f);
                for feature in features {
                    feature.ty.iter().for_each(&mut *f);
                }
                for op in operations {
                    op.ty.iter().for_each(&mut *f);
                    op.throws.iter().for_each(&mut *f);
                    for p in &op.params {
                        p.ty.iter().for_each(&mut *f);
                    }
                }
            }
        }
        for sub in &p.packages {
            package(sub, f);
        }
    }
    for p in &doc.packages {
        package(p, f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_structure_and_expressions() {
        let c = scan("package p : p = 'u' {\n  class A {\n    attribute x : ");
        assert_eq!(c.blocks, [Block::Package, Block::Class]);
        assert!(!c.in_expression);
        assert_eq!(c.statement.last(), Some(&TokenKind::Colon));

        let c = scan("package p {\n class A {\n invariant I: Set{1}->includes(self.");
        assert!(c.in_expression);
        assert_eq!(c.constraint.as_deref(), Some("invariant"));

        let c = scan("package p {\n class A {\n invariant I('m' + ");
        assert!(c.in_expression);
        let c = scan("package p {\n class A {\n invariant I('m'): x;\n ");
        assert!(!c.in_expression && c.statement.is_empty());

        // Qualifiers, then the body of the same feature.
        let c = scan("package p {\n class A {\n property b : B[*] { ordered } {\n ");
        assert_eq!(c.blocks.last(), Some(&Block::Feature));
        let c = scan("package p {\n class A {\n operation f() : Boolean {\n post: result = true;\n ");
        assert_eq!(c.blocks.last(), Some(&Block::Operation));
        let c = scan("package p {\n class A {\n operation f() { body: true; }\n ");
        assert_eq!(c.blocks.last(), Some(&Block::Class));
    }
}
