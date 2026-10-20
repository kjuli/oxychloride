//! Completion and hovers for HUTN model documents. Both are driven by the metamodel: which
//! classes can be instantiated, which features an object has, and which values a feature
//! takes (literals, nested objects of compatible classes, or references to identified objects
//! of compatible classes).

use std::collections::HashMap;

use oxy_ecore::hutn::{self, Block, Item, Tok, ValueKind};
use oxy_ecore::{ClassifierId, DataTypeKind, FeatureId, ObjectId, PackageId};
use oxy_ocl::Type;

use super::item;
use crate::protocol::*;
use crate::workspace::{Error, Workspace};

#[derive(Clone, Debug)]
enum Frame {
    Spec,
    Metamodel,
    Package(Option<PackageId>),
    Object {
        class: Option<ClassifierId>,
        package: Option<PackageId>,
        /// Features that already have a slot.
        set: Vec<String>,
    },
}

/// What surrounds the cursor.
struct Context {
    frames: Vec<Frame>,
    /// The feature whose values are being written.
    slot: Option<String>,
    /// Right after `feature:` or a `,` between values.
    expect_value: bool,
    /// The class written before an unterminated `"identifier` at the cursor, and where the
    /// identifier starts.
    reference: Option<(Vec<String>, usize)>,
}

impl Workspace {
    /// `@Spec` names of the packages, from a parsed document.
    fn spec_aliases(&self, doc: &hutn::Document) -> HashMap<String, PackageId> {
        doc.spec
            .iter()
            .filter_map(|s| Some((s.name.clone(), self.metamodel().package_by_ns_uri(s.ns_uri.as_deref()?)?)))
            .collect()
    }

    fn scan(&self, head: &str, aliases: &HashMap<String, PackageId>) -> Context {
        let mm = self.metamodel();
        let (tokens, _) = hutn::tokenize(head);
        let tokens: Vec<&hutn::Token> = tokens.iter().filter(|t| t.kind != Tok::Eof).collect();
        let mut frames: Vec<(Frame, Option<String>)> = Vec::new();
        let mut words: Vec<String> = Vec::new();
        let mut slot: Option<String> = None;
        let mut expect_value = false;
        let mut spec = false;
        let mut reference = None;
        let mut i = 0;
        while i < tokens.len() {
            let t = &tokens[i].kind;
            let next = tokens.get(i + 1).map(|t| &t.kind);
            let was_expecting = std::mem::replace(&mut expect_value, false);
            match t {
                Tok::At => spec = true,
                Tok::Ident(w) if next == Some(&Tok::Colon) => {
                    slot = Some(w.clone());
                    if let Some((Frame::Object { set, .. }, _)) = frames.last_mut() {
                        set.push(w.clone());
                    }
                    words.clear();
                    expect_value = true;
                    i += 1; // the colon
                }
                Tok::Ident(w) => {
                    if !was_expecting && words.is_empty() && !matches!(frames.last(), Some((Frame::Spec, _))) {
                        // A new item after a complete value.
                        slot = None;
                    }
                    if matches!(tokens.get(i.wrapping_sub(1)).map(|t| &t.kind), Some(Tok::ColonColon)) {
                        if let Some(last) = words.last_mut() {
                            last.push_str("::");
                            last.push_str(w);
                        }
                    } else {
                        words.push(w.clone());
                    }
                }
                Tok::Str(_) => {
                    // The cursor is inside an unterminated `"identifier` after a class name.
                    let raw = &head[tokens[i].span.clone()];
                    let quote = raw.chars().next().unwrap_or('"');
                    let unterminated = raw.len() == 1 || !raw.ends_with(quote);
                    if i + 1 == tokens.len() && tokens[i].span.end == head.len() && unterminated {
                        if let Some(class) = words.last() {
                            let path = class.split("::").map(str::to_string).collect();
                            reference = Some((path, tokens[i].span.start + 1));
                        }
                    }
                }
                Tok::LBrace => {
                    let frame = if spec && words.last().is_some_and(|w| w == "Spec") {
                        spec = false;
                        Frame::Spec
                    } else if matches!(frames.last(), Some((Frame::Spec, _))) {
                        Frame::Metamodel
                    } else {
                        let package = match frames.last() {
                            Some((Frame::Package(p), _)) => *p,
                            Some((Frame::Object { package, .. }, _)) => *package,
                            _ => None,
                        };
                        let path: Vec<String> =
                            words.last().map(|w| w.split("::").map(str::to_string).collect()).unwrap_or_default();
                        let top_level = frames.is_empty();
                        let is_package = top_level
                            && words.len() == 1
                            && !matches!(tokens.get(i.wrapping_sub(1)).map(|t| &t.kind), Some(Tok::Str(_)))
                            && hutn::resolve_package(mm, aliases, &path).is_some();
                        if is_package {
                            Frame::Package(hutn::resolve_package(mm, aliases, &path))
                        } else {
                            let class = hutn::resolve_class(mm, aliases, package, &path).ok();
                            Frame::Object { class, package, set: Vec::new() }
                        }
                    };
                    frames.push((frame, slot.take()));
                    words.clear();
                }
                Tok::RBrace => {
                    if let Some((_, saved)) = frames.pop() {
                        slot = saved;
                    }
                    words.clear();
                }
                Tok::Comma => {
                    words.clear();
                    expect_value = slot.is_some();
                }
                Tok::Semi => {
                    slot = None;
                    words.clear();
                }
                _ => {}
            }
            i += 1;
        }
        Context { frames: frames.into_iter().map(|(f, _)| f).collect(), slot, expect_value, reference }
    }

    /// Completion items and the start of the text they replace.
    pub(super) fn complete_hutn(
        &self,
        document: &str,
        text: &str,
        word_start: usize,
    ) -> Option<(Vec<CompletionItem>, usize)> {
        let mm = self.metamodel();
        let (doc, _) = hutn::parse(text);
        let aliases = self.spec_aliases(&doc);
        let head = &text[..word_start];
        if head[head.rfind('\n').map_or(0, |i| i + 1)..].contains("//") {
            return Some((Vec::new(), word_start));
        }
        let context = self.scan(head, &aliases);
        let mut items = Vec::new();
        if let Some((class, start)) = &context.reference {
            // `Class "|`: identifiers of objects of that class.
            let package = context.frames.iter().rev().find_map(|f| match f {
                Frame::Object { package, .. } => *package,
                Frame::Package(p) => *p,
                _ => None,
            });
            if let Ok(c) = hutn::resolve_class(mm, &aliases, package, class) {
                for (o, id) in self.identified_objects(document) {
                    if mm.conforms_to(self.model().class_of(o), c) {
                        items.push(item(&id, CompletionKind::Object, Some(self.model().label(mm, o))));
                    }
                }
            }
            return Some((items, *start));
        }
        match context.frames.last() {
            None => {
                for p in mm.user_root_packages() {
                    let mut i = item(&mm.package(p).name, CompletionKind::Keyword, Some(mm.package(p).ns_uri.clone()));
                    i.insert = Some(format!("{} {{\n}}", mm.package(p).name));
                    items.push(i);
                }
                for name in aliases.keys() {
                    items.push(item(name, CompletionKind::Keyword, Some("metamodel".into())));
                }
            }
            Some(Frame::Spec) => items.push(item("metamodel", CompletionKind::Keyword, None)),
            Some(Frame::Metamodel) => {
                if context.expect_value {
                    for p in mm.user_root_packages() {
                        let uri = &mm.package(p).ns_uri;
                        let mut i = item(uri, CompletionKind::Snippet, Some(mm.package(p).name.clone()));
                        i.insert = Some(hutn::quote(uri));
                        items.push(i);
                    }
                } else {
                    let mut i = item("nsUri", CompletionKind::Property, None);
                    i.insert = Some("nsUri: ".into());
                    items.push(i);
                }
            }
            Some(Frame::Package(p)) => self.class_items(&mut items, *p, None),
            Some(Frame::Object { class, package, set }) => {
                let Some(class) = *class else { return Some((items, word_start)) };
                match (&context.slot, context.expect_value) {
                    (Some(slot), true) => {
                        if let Some(f) = mm.lookup_feature(class, slot) {
                            self.value_items(document, &mut items, f, *package);
                        }
                    }
                    _ => {
                        for &f in mm.all_features(class) {
                            let feature = mm.feature(f);
                            let container = feature.opposite().is_some_and(|o| mm.feature(o).is_containment());
                            if feature.derived || !feature.changeable || container {
                                continue;
                            }
                            if set.contains(&feature.name) && !feature.is_many() {
                                continue;
                            }
                            let ty = Type::of_typed(mm, feature.ty, &feature.multiplicity);
                            let mut i = item(&feature.name, CompletionKind::Property, Some(ty.display(mm).to_string()));
                            i.insert = Some(format!("{}: ", feature.name));
                            items.push(i);
                        }
                    }
                }
            }
        }
        Some((items, word_start))
    }

    /// Concrete classes (conforming to `of`, if given), preferring `package`'s.
    fn class_items(&self, items: &mut Vec<CompletionItem>, package: Option<PackageId>, of: Option<ClassifierId>) {
        let mm = self.metamodel();
        let mut classes = self.concrete_classes(of);
        classes.sort_by_key(|c| Some(mm.classifier(*c).package) != package);
        for c in classes {
            let cl = mm.classifier(c);
            let label =
                if Some(cl.package) == package || package.is_none() { cl.name.clone() } else { mm.qualified_name(c) };
            items.push(item(&label, CompletionKind::Type, Some(mm.package_qualified_name(cl.package))));
        }
    }

    /// The classes of user packages that can be instantiated (conforming to `of`, if given).
    fn concrete_classes(&self, of: Option<ClassifierId>) -> Vec<ClassifierId> {
        let mm = self.metamodel();
        mm.classifier_ids()
            .filter(|c| mm.classifier(*c).package != mm.ecore_package())
            .filter(|c| mm.class_data(*c).is_some_and(|d| !d.is_abstract && !d.is_interface))
            .filter(|c| of.is_none_or(|t| mm.conforms_to(*c, t)))
            .collect()
    }

    /// Objects of a HUTN document that have an identifier.
    fn identified_objects(&self, document: &str) -> Vec<(ObjectId, String)> {
        let Some(load) = self.hutn_document(document) else { return Vec::new() };
        load.objects.iter().filter_map(|o| Some((o.object, self.model().object(o.object).xmi_id.clone()?))).collect()
    }

    fn value_items(&self, document: &str, items: &mut Vec<CompletionItem>, f: FeatureId, package: Option<PackageId>) {
        let mm = self.metamodel();
        let feature = mm.feature(f);
        let Some(ty) = feature.ty else { return };
        if feature.is_containment() {
            self.class_items(items, package, Some(ty));
            for i in items.iter_mut() {
                i.insert = Some(format!("{} {{ }}", i.label));
            }
        } else if feature.is_reference() {
            for (o, id) in self.identified_objects(document) {
                let class = self.model().class_of(o);
                if mm.conforms_to(class, ty) {
                    let name = &mm.classifier(class).name;
                    let reference = format!("{name} {}", hutn::quote(&id));
                    let mut i = item(&reference, CompletionKind::Object, Some(self.model().label(mm, o)));
                    i.insert = Some(reference);
                    items.push(i);
                }
            }
        } else if mm.is_enum(ty) {
            for l in mm.enum_literals(ty) {
                items.push(item(&l.name, CompletionKind::EnumLiteral, Some(mm.classifier(ty).name.clone())));
            }
        } else if mm.data_type_kind(ty) == Some(DataTypeKind::Boolean) {
            items.push(item("true", CompletionKind::Keyword, None));
            items.push(item("false", CompletionKind::Keyword, None));
        }
        if !feature.is_many() && !feature.multiplicity.is_required() {
            items.push(item("null", CompletionKind::Keyword, None));
        }
    }

    /// Hover on a class name, a feature name or a reference.
    pub(super) fn hover_hutn(&self, document: &str, text: &str, offset: usize) -> Option<HoverInfo> {
        let mm = self.metamodel();
        let (doc, _) = hutn::parse(text);
        let aliases = self.spec_aliases(&doc);
        let mut found = None;
        for b in &doc.blocks {
            match hutn::package_block(mm, &aliases, b) {
                Some(p) => {
                    if b.name.span.contains(&offset) || b.name.span.end == offset {
                        found = Some((
                            b.name.span.clone(),
                            format!("package {} = '{}'", mm.package_qualified_name(p), mm.package(p).ns_uri),
                        ));
                    }
                    for item in &b.items {
                        if let Item::Block(o) = item {
                            self.hover_object(document, &aliases, o, Some(p), offset, &mut found);
                        }
                    }
                }
                None => self.hover_object(document, &aliases, b, None, offset, &mut found),
            }
        }
        let (span, contents) = found?;
        let index = oxy_ocl::LineIndex::new(text);
        let p = |o: usize| {
            let p = index.position(text, o);
            Position { line: p.line, column: p.column }
        };
        Some(HoverInfo { range: Range { start: p(span.start), end: p(span.end) }, contents })
    }

    fn hover_object(
        &self,
        document: &str,
        aliases: &HashMap<String, PackageId>,
        b: &Block,
        package: Option<PackageId>,
        offset: usize,
        found: &mut Option<(std::ops::Range<usize>, String)>,
    ) {
        if !(b.span.start <= offset && offset <= b.span.end) {
            return;
        }
        let mm = self.metamodel();
        let Ok(class) = hutn::resolve_class(mm, aliases, package, &b.name.path) else { return };
        let within = |s: &std::ops::Range<usize>| s.start <= offset && offset <= s.end;
        let header = b.name.span.start..b.identifier.as_ref().map_or(b.name.span.end, |(_, s)| s.end);
        if within(&header) {
            let supers: Vec<String> = mm.super_types(class).iter().map(|s| mm.classifier(*s).name.clone()).collect();
            let extends = if supers.is_empty() { String::new() } else { format!(" extends {}", supers.join(", ")) };
            *found = Some((header, format!("class {}{extends}", mm.qualified_name(class))));
            return;
        }
        for item in &b.items {
            let Item::Slot(s) = item else { continue };
            let feature = mm.lookup_feature(class, &s.feature);
            if within(&s.feature_span) {
                if let Some(f) = feature {
                    let feat = mm.feature(f);
                    let ty = Type::of_typed(mm, feat.ty, &feat.multiplicity);
                    let kind = if feat.is_containment() {
                        "containment"
                    } else if feat.is_reference() {
                        "reference"
                    } else {
                        "attribute"
                    };
                    let bounds = multiplicity(feat.multiplicity.lower, feat.multiplicity.upper);
                    *found = Some((
                        s.feature_span.clone(),
                        format!("{kind} {}::{} : {}{bounds}", mm.classifier(class).name, feat.name, ty.display(mm)),
                    ));
                }
                return;
            }
            for v in &s.values {
                match &v.kind {
                    ValueKind::Object(nested) => self.hover_object(document, aliases, nested, package, offset, found),
                    ValueKind::Reference { identifier, .. } if within(&v.span) => {
                        let target = self
                            .identified_objects(document)
                            .into_iter()
                            .find(|(_, id)| id == identifier)
                            .map(|(o, _)| self.model().label(mm, o));
                        if let Some(label) = target {
                            *found = Some((v.span.clone(), format!("→ {label}")));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

// ---------------------------------------------------------------------- creating objects

/// Marks the cursor position in generated text (removed before the edits are returned).
const CURSOR: char = '\u{0}';

impl Workspace {
    /// The containment features of an object that can be set in a model document, and the
    /// classes of the objects each can contain.
    pub fn containment_options(&self, object: &ObjectRef) -> Result<Vec<ContainmentOption>, Error> {
        let mm = self.metamodel();
        let o = self.resolve_object(object)?;
        let options = mm
            .all_features(self.model().class_of(o))
            .iter()
            .filter_map(|&f| {
                let feature = mm.feature(f);
                if !feature.is_containment() || feature.derived || !feature.changeable {
                    return None;
                }
                let ty = feature.ty?;
                Some(ContainmentOption {
                    feature: feature.name.clone(),
                    ty: mm.qualified_name(ty),
                    many: feature.is_many(),
                    full: !feature.is_many() && !self.model().get(o, f).is_empty(),
                    classes: self.concrete_classes(Some(ty)).into_iter().map(|c| mm.qualified_name(c)).collect(),
                })
            })
            .collect();
        Ok(options)
    }

    /// The edits that add a new object of `class` (a qualified name) to a HUTN document:
    /// as the value of the containment feature `container.1` of the object at fragment
    /// `container.0` of the document, or else as a root object in a package block of the
    /// class's package (added, with its `@Spec` entry, if the document has none). The
    /// document itself is not changed.
    pub fn create_object(
        &self,
        document: &str,
        class: &str,
        container: Option<(&str, &str)>,
    ) -> Result<DocumentEdit, Error> {
        let mm = self.metamodel();
        let text = match (self.document_language(document), self.document_text(document)) {
            (Some(Language::Hutn), Some(text)) => text,
            _ => return Err(Error::NotFound(format!("no open HUTN document named '{document}'"))),
        };
        let path: Vec<&str> = class.split("::").collect();
        let c = mm
            .find_classifiers_qualified(&path)
            .into_iter()
            .find(|c| mm.is_class(*c))
            .ok_or_else(|| Error::NotFound(format!("unknown class '{class}'")))?;
        if let Some(data) = mm.class_data(c)
            && (data.is_abstract || data.is_interface)
        {
            let what = if data.is_interface { "an interface" } else { "abstract" };
            return Err(Error::Invalid(format!("'{}' is {what} and cannot be instantiated", mm.classifier(c).name)));
        }
        let (doc, _) = hutn::parse(text);
        let aliases = self.spec_aliases(&doc);
        let mut edits: Vec<(ByteRange, String)> = Vec::new();
        match container {
            Some((fragment, feature)) => {
                let object = ObjectRef { resource: Some(document.to_string()), fragment: fragment.to_string() };
                let o = self.resolve_object(&object)?;
                let span = self
                    .hutn_document(document)
                    .and_then(|l| l.objects.iter().find(|h| h.object == o))
                    .map(|h| h.span.clone())
                    .ok_or_else(|| Error::NotFound(format!("no object at '{fragment}' in '{document}'")))?;
                let owner = self.model().class_of(o);
                let f = mm.lookup_feature(owner, feature).ok_or_else(|| {
                    Error::NotFound(format!("class '{}' has no feature '{feature}'", mm.classifier(owner).name))
                })?;
                let feat = mm.feature(f);
                if !feat.is_containment() || feat.derived || !feat.changeable {
                    return Err(Error::Invalid(format!("'{feature}' is not a containment feature that can be set")));
                }
                if let Some(ty) = feat.ty
                    && !mm.conforms_to(c, ty)
                {
                    return Err(Error::Invalid(format!(
                        "'{}' is not a {}, so it cannot be a value of '{feature}'",
                        mm.classifier(c).name,
                        mm.classifier(ty).name
                    )));
                }
                let (block, package) = find_block(mm, &aliases, &doc, &span)
                    .ok_or_else(|| Error::NotFound(format!("no object at '{fragment}' in '{document}'")))?;
                let new = format!("{} {{ {CURSOR} }}", class_reference(mm, &aliases, package, c));
                let slot = block.items.iter().find_map(|i| match i {
                    Item::Slot(s) if s.feature == feature => Some(s),
                    _ => None,
                });
                match slot {
                    Some(_) if !feat.is_many() => {
                        return Err(Error::Invalid(format!("'{feature}' already has a value")));
                    }
                    // Another value of the slot: on a line of its own if the values are.
                    Some(s) => match s.values.last() {
                        Some(v) if text[s.feature_span.start..v.span.start].contains('\n') => {
                            edits
                                .push((v.span.end..v.span.end, format!(",\n{}{new}", indentation(text, v.span.start))));
                        }
                        Some(v) => edits.push((v.span.end..v.span.end, format!(", {new}"))),
                        None => edits.push((s.span.end..s.span.end, format!(" {new}"))),
                    },
                    None => edits.push(insert_in_body(text, block, &format!("{feature}: {new}"))),
                }
            }
            None => {
                let p = mm.classifier(c).package;
                let blocks: Vec<(&Block, PackageId)> =
                    doc.blocks.iter().filter_map(|b| Some((b, hutn::package_block(mm, &aliases, b)?))).collect();
                // The last block of the class's package, else of a package containing it.
                let target = blocks
                    .iter()
                    .rev()
                    .find(|(_, q)| *q == p)
                    .or_else(|| blocks.iter().rev().find(|(_, q)| contains_package(mm, *q, p)));
                match target {
                    Some((b, q)) => {
                        let new = format!("{} {{ {CURSOR} }}", class_reference(mm, &aliases, Some(*q), c));
                        edits.push(insert_in_body(text, b, &new));
                    }
                    None => {
                        let mut root = p;
                        while let Some(sp) = mm.package(root).super_package {
                            root = sp;
                        }
                        let pkg = mm.package(root);
                        let alias = doc.spec.iter().find(|s| s.ns_uri.as_deref() == Some(pkg.ns_uri.as_str()));
                        let name = alias.map_or(pkg.name.as_str(), |s| s.name.as_str());
                        let mut aliases = aliases.clone();
                        aliases.insert(name.to_string(), root);
                        let new = format!("{} {{ {CURSOR} }}", class_reference(mm, &aliases, Some(root), c));
                        let block = format!("{name} {{\n  {new}\n}}\n");
                        let end = text.len();
                        let entry = |indent: &str| {
                            format!(
                                "{indent}metamodel {} {{\n{indent}  nsUri: {}\n{indent}}}",
                                hutn::quote(name),
                                hutn::quote(&pkg.ns_uri)
                            )
                        };
                        match (alias, doc.spec.last(), doc.blocks.first()) {
                            (Some(_), _, _) => edits.push((end..end, format!("{}{block}", separator(text)))),
                            (None, Some(s), _) => {
                                let at = s.span.end;
                                edits.push((at..at, format!("\n{}", entry(&indentation(text, s.span.start)))));
                                edits.push((end..end, format!("{}{block}", separator(text))));
                            }
                            (None, None, Some(first)) => {
                                let at = first.span.start;
                                edits.push((at..at, format!("@Spec {{\n{}\n}}\n\n", entry("  "))));
                                edits.push((end..end, format!("{}{block}", separator(text))));
                            }
                            (None, None, None) => edits.push((
                                end..end,
                                format!("{}@Spec {{\n{}\n}}\n\n{block}", separator(text), entry("  ")),
                            )),
                        }
                    }
                }
            }
        }
        Ok(self.document_edit(document, text, edits))
    }

    /// Converts byte-range edits (with a [`CURSOR`] in one of them) to a [`DocumentEdit`].
    fn document_edit(&self, document: &str, text: &str, mut edits: Vec<(ByteRange, String)>) -> DocumentEdit {
        edits.sort_by_key(|(r, _)| r.start);
        let mut edited = String::with_capacity(text.len() + 64);
        let mut last = 0;
        for (r, new) in &edits {
            edited.push_str(&text[last..r.start]);
            edited.push_str(new);
            last = r.end;
        }
        edited.push_str(&text[last..]);
        let cursor = edited.find(CURSOR).unwrap_or(edited.len());
        let edited = edited.replace(CURSOR, "");
        let p = oxy_ocl::LineIndex::new(&edited).position(&edited, cursor);
        DocumentEdit {
            edits: edits
                .into_iter()
                .map(|(r, new)| TextEdit { range: self.document_range(document, &r), text: new.replace(CURSOR, "") })
                .collect(),
            cursor: Position { line: p.line, column: p.column },
        }
    }
}

type ByteRange = std::ops::Range<usize>;

/// The block defining the object at `span`, and the package its class names resolve in.
fn find_block<'d>(
    mm: &oxy_ecore::Metamodel,
    aliases: &HashMap<String, PackageId>,
    doc: &'d hutn::Document,
    span: &ByteRange,
) -> Option<(&'d Block, Option<PackageId>)> {
    fn within<'d>(
        b: &'d Block,
        package: Option<PackageId>,
        span: &ByteRange,
    ) -> Option<(&'d Block, Option<PackageId>)> {
        if b.span == *span {
            return Some((b, package));
        }
        b.items.iter().find_map(|item| match item {
            Item::Block(nested) => within(nested, package, span),
            Item::Slot(s) => s.values.iter().find_map(|v| match &v.kind {
                ValueKind::Object(nested) => within(nested, package, span),
                _ => None,
            }),
        })
    }
    doc.blocks.iter().find_map(|b| match hutn::package_block(mm, aliases, b) {
        Some(p) => b.items.iter().find_map(|item| match item {
            Item::Block(o) => within(o, Some(p), span),
            Item::Slot(_) => None,
        }),
        None => within(b, None, span),
    })
}

/// How to write class `c` inside `package`: its simple name if that resolves to it, else its
/// qualified name.
fn class_reference(
    mm: &oxy_ecore::Metamodel,
    aliases: &HashMap<String, PackageId>,
    package: Option<PackageId>,
    c: ClassifierId,
) -> String {
    let name = &mm.classifier(c).name;
    if hutn::resolve_class(mm, aliases, package, std::slice::from_ref(name)) == Ok(c) {
        name.clone()
    } else {
        mm.qualified_name(c)
    }
}

/// Whether `inner` is `outer` or one of its (transitive) subpackages.
fn contains_package(mm: &oxy_ecore::Metamodel, outer: PackageId, inner: PackageId) -> bool {
    let mut p = Some(inner);
    while let Some(q) = p {
        if q == outer {
            return true;
        }
        p = mm.package(q).super_package;
    }
    false
}

/// The leading whitespace of the line containing `offset`.
fn indentation(text: &str, offset: usize) -> String {
    let start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    text[start..].chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

/// What to put before text appended to `text` so that it starts after a blank line.
fn separator(text: &str) -> &'static str {
    if text.trim().is_empty() || text.ends_with("\n\n") {
        ""
    } else if text.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    }
}

/// The edit adding `item` (an object or a slot) at the end of a block's body.
fn insert_in_body(text: &str, block: &Block, item: &str) -> (ByteRange, String) {
    let inner = format!("{}  ", indentation(text, block.span.start));
    let Some(body) = &block.body else {
        let end = block.span.end;
        return (end..end, format!(" {{ {item} }}"));
    };
    let close = body.end - 1;
    if text.as_bytes().get(close) != Some(&b'}') {
        // An unterminated block: append to its last line.
        return (body.end..body.end, format!("\n{inner}{item}"));
    }
    let line_start = text[..close].rfind('\n').map_or(0, |i| i + 1);
    if line_start <= body.start || !text[line_start..close].trim().is_empty() {
        // `{ ... }` on one line.
        let space = if text[..close].ends_with(char::is_whitespace) { "" } else { " " };
        return (close..close, format!("{space}{item} "));
    }
    // The closing brace is on a line of its own; an empty body's blank lines are replaced.
    let from = match text[body.start + 1..line_start].find('\n') {
        Some(i) if block.items.is_empty() && text[body.start + 1..line_start].trim().is_empty() => {
            body.start + 1 + i + 1
        }
        _ => line_start,
    };
    (from..line_start, format!("{inner}{item}\n"))
}

fn multiplicity(lower: i32, upper: i32) -> String {
    match (lower, upper) {
        (0, 1) => "[?]".into(),
        (1, 1) => String::new(),
        (0, u) if u < 0 => "[*]".into(),
        (1, u) if u < 0 => "[+]".into(),
        (l, u) if u < 0 => format!("[{l}..*]"),
        (l, u) => format!("[{l}..{u}]"),
    }
}
