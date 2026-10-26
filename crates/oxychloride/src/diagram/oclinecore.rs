//! Diagram edits of OCLinEcore documents, made on the syntax tree: new elements are written
//! the way `oxy_ocl::oclinecore::print` writes them, changed elements get new names, types,
//! multiplicities or qualifier lists in place.

use std::collections::{HashMap, HashSet};
use std::ops::Range as ByteRange;

use oxy_ecore::ecore_loader::resolve_relative;
use oxy_ecore::{ClassifierId, Metamodel, Multiplicity, PackageId};
use oxy_ocl::Span;
use oxy_ocl::oclinecore::{multiplicity, name as quoted, type_name};
use oxy_ocl::syntax::ast::*;

use super::*;

#[derive(Clone, Copy)]
enum Node<'a> {
    Package(&'a PackageDef),
    Classifier(&'a ClassifierDef),
    Feature(&'a FeatureDef),
    Operation(&'a OperationDef),
    Literal(&'a LiteralDef),
}

/// An OCLinEcore document with the ids of the elements it defines.
pub(crate) struct Doc<'a> {
    text: &'a str,
    doc: &'a EcoreDocument,
    nodes: HashMap<String, Node<'a>>,
    /// Every type reference, with the id of the package it is written in.
    refs: Vec<(&'a TypeRef, String)>,
}

impl<'a> Doc<'a> {
    pub(crate) fn new(text: &'a str, doc: &'a EcoreDocument) -> Self {
        let mut d = Doc { text, doc, nodes: HashMap::new(), refs: Vec::new() };
        let mut alloc = IdAllocator::default();
        for p in &doc.packages {
            d.package(p, "", &mut alloc);
        }
        d
    }

    fn package(&mut self, p: &'a PackageDef, prefix: &str, alloc: &mut IdAllocator) {
        let id = alloc.next(format!("{prefix}{}", p.name.name));
        self.nodes.insert(id.clone(), Node::Package(p));
        for c in &p.classifiers {
            let cid = alloc.next(classifier_id(&id, &c.name.name));
            self.nodes.insert(cid.clone(), Node::Classifier(c));
            match &c.kind {
                ClassifierDefKind::Class { supers, features, operations, .. } => {
                    self.refs.extend(supers.iter().map(|s| (s, id.clone())));
                    for f in features {
                        self.nodes.insert(alloc.next(member_id(&cid, &f.name.name)), Node::Feature(f));
                        self.refs.extend(f.ty.iter().map(|t| (t, id.clone())));
                    }
                    for o in operations {
                        self.nodes.insert(alloc.next(operation_id(&cid, &o.name.name)), Node::Operation(o));
                        let types = o.ty.iter().chain(o.params.iter().filter_map(|p| p.ty.as_ref())).chain(&o.throws);
                        self.refs.extend(types.map(|t| (t, id.clone())));
                    }
                }
                ClassifierDefKind::Enum { literals } => {
                    for l in literals {
                        self.nodes.insert(alloc.next(member_id(&cid, &l.name.name)), Node::Literal(l));
                    }
                }
                ClassifierDefKind::DataType => {}
            }
        }
        for s in &p.packages {
            self.package(s, &format!("{id}::"), alloc);
        }
    }

    fn slice(&self, span: Span) -> &'a str {
        self.text.get(span.range()).unwrap_or("")
    }

    /// A span extended over the `;` that ends its statement.
    fn statement(&self, span: Span) -> ByteRange<usize> {
        let end = span.end as usize;
        let rest = &self.text[end..];
        let skipped = rest.len() - rest.trim_start().len();
        if rest.trim_start().starts_with(';') { span.start as usize..end + skipped + 1 } else { span.range() }
    }

    fn span_of(&self, node: Node<'_>) -> ByteRange<usize> {
        match node {
            Node::Package(p) => p.span.range(),
            Node::Classifier(c) => c.span.range(),
            Node::Feature(f) => f.span.range(),
            Node::Operation(o) => o.span.range(),
            Node::Literal(l) => self.statement(l.span),
        }
    }

    pub(crate) fn source_map(&self) -> SourceMap {
        let mut map = SourceMap::default();
        for (id, node) in &self.nodes {
            map.spans.insert(id.clone(), self.span_of(*node));
            match node {
                Node::Classifier(c) => {
                    if let ClassifierDefKind::Class { invariants, .. } = &c.kind {
                        let list = invariants
                            .iter()
                            .map(|i| (i.name.as_ref().map(|n| n.name.clone()).unwrap_or_default(), i.span.range()))
                            .collect();
                        map.invariants.insert(id.clone(), list);
                    }
                }
                Node::Feature(f) => {
                    if let Some(t) = &f.ty {
                        map.type_texts.insert(id.clone(), self.slice(t.span).to_string());
                    }
                }
                Node::Operation(o) => {
                    if let Some(t) = &o.ty {
                        map.type_texts.insert(id.clone(), self.slice(t.span).to_string());
                    }
                }
                Node::Package(_) | Node::Literal(_) => {}
            }
        }
        map
    }

    /// The resources whose classifiers the document can use: its own and those it imports.
    pub(crate) fn visible_resources(&self, mm: &Metamodel, name: &str, own: usize) -> Vec<usize> {
        let mut out = vec![own];
        for import in &self.doc.imports {
            let target = import.uri.split('#').next().unwrap_or("");
            let resource = match mm.package_by_ns_uri(target) {
                Some(p) => Some(mm.package(p).resource),
                None => mm.resource_of_uri(&resolve_relative(name, target)),
            };
            if let Some(r) = resource
                && !out.contains(&r)
            {
                out.push(r);
            }
        }
        out
    }

    fn node(&self, id: &str) -> Result<Node<'a>, Error> {
        self.nodes.get(id).copied().ok_or_else(|| Error::NotFound(format!("no element '{id}' in the document")))
    }

    fn class_def(&self, cx: &Context, c: ClassifierId) -> Result<&'a ClassifierDef, Error> {
        match cx.id(Element::Classifier(c)).map(|id| self.node(id)) {
            Some(Ok(Node::Classifier(cd))) => Ok(cd),
            _ => Err(Error::NotFound(format!("'{}' is not defined in the document", cx.mm.qualified_name(c)))),
        }
    }

    fn type_text(&self, cx: &Context, t: ClassifierId, owner: ClassifierId) -> String {
        type_name(cx.mm, t, cx.mm.classifier(owner).package)
    }

    // -------------------------------------------------------------- inserting

    /// Inserts members into the body of a package or classifier (`span`), creating the body
    /// for `class A;`.
    fn insert_into(
        &self,
        span: Span,
        body: Option<Span>,
        after: Option<ByteRange<usize>>,
        member: &str,
        blank: bool,
    ) -> ByteEdit {
        match body {
            Some(b) => insert_member(
                self.text,
                span.start as usize,
                b.start as usize + 1,
                b.end as usize - 1,
                after,
                member,
                blank,
            ),
            None => {
                let end = span.end as usize;
                let indent = indentation(self.text, span.start as usize);
                let unit = indent_unit(self.text);
                let lines: Vec<String> = member.lines().map(|l| format!("{indent}{unit}{l}")).collect();
                let semi = if self.text[..end].ends_with(';') { end - 1 } else { end };
                ByteEdit::replace(semi..end, format!(" {{\n{}\n{indent}}}", lines.join("\n")))
            }
        }
    }

    fn last_annotation(&self, annotations: &[AnnotationDef]) -> Option<ByteRange<usize>> {
        annotations.last().map(|a| self.statement(a.span))
    }

    /// Adds attributes or references after the last feature.
    fn insert_feature(&self, cd: &ClassifierDef, member: &str) -> ByteEdit {
        let ClassifierDefKind::Class { features, .. } = &cd.kind else { unreachable!("checked to be a class") };
        let after = features.last().map(|f| f.span.range()).or_else(|| self.last_annotation(&cd.annotations));
        self.insert_into(cd.span, cd.body, after, member, false)
    }

    /// Adds operations after the last operation (or feature).
    fn insert_operation(&self, cd: &ClassifierDef, member: &str) -> ByteEdit {
        let ClassifierDefKind::Class { features, operations, .. } = &cd.kind else {
            unreachable!("checked to be a class")
        };
        let after = operations
            .last()
            .map(|o| o.span.range())
            .or_else(|| features.last().map(|f| f.span.range()))
            .or_else(|| self.last_annotation(&cd.annotations));
        self.insert_into(cd.span, cd.body, after, member, false)
    }

    fn parameters(&self, cx: &Context, owner: ClassifierId, params: &[ParameterSpec]) -> Result<String, Error> {
        let mut out = Vec::new();
        for p in params {
            cx.check_name(&p.name)?;
            let t = cx.resolve_type(&p.ty)?;
            out.push(format!("{} : {}{}", quoted(&p.name), self.type_text(cx, t, owner), mult(p.lower, p.upper)));
        }
        Ok(out.join(", "))
    }

    // -------------------------------------------------------------- edits

    pub(crate) fn edit(&self, cx: &Context, edit: &MetamodelEdit) -> Result<(Vec<ByteEdit>, Option<String>), Error> {
        let mm = cx.mm;
        match edit {
            MetamodelEdit::AddClassifier { package, classifier, name } => {
                let Element::Package(p) = cx.element(package)? else {
                    return Err(Error::Invalid(format!("'{package}' is not a package")));
                };
                cx.check_new_classifier(p, name)?;
                let Node::Package(pd) = self.node(package)? else { unreachable!("a package") };
                let n = quoted(name);
                let member = match classifier {
                    NewClassifier::Class => format!("class {n};"),
                    NewClassifier::AbstractClass => format!("abstract class {n};"),
                    NewClassifier::Interface => format!("interface {n};"),
                    NewClassifier::Enum => format!("enum {n};"),
                    NewClassifier::DataType => format!("datatype {n} : 'java.lang.String';"),
                };
                let after =
                    pd.classifiers.last().map(|c| c.span.range()).or_else(|| self.last_annotation(&pd.annotations));
                let edit = self.insert_into(pd.span, pd.body, after, &member, !pd.classifiers.is_empty());
                Ok((vec![edit], Some(classifier_id(package, name))))
            }
            MetamodelEdit::AddAttribute { owner, name, ty, lower, upper } => {
                let c = cx.class(owner)?;
                cx.check_new_feature(c, name)?;
                let t = cx.feature_type(ty, false)?;
                let member =
                    format!("attribute {} : {}{};", quoted(name), self.type_text(cx, t, c), mult(*lower, *upper));
                Ok((vec![self.insert_feature(self.class_def(cx, c)?, &member)], Some(member_id(owner, name))))
            }
            MetamodelEdit::AddReference { owner, name, ty, lower, upper, containment, opposite } => {
                let c = cx.class(owner)?;
                cx.check_new_feature(c, name)?;
                let t = cx.feature_type(ty, true)?;
                let back = opposite.as_ref().map(|o| format!("#{}", quoted(o))).unwrap_or_default();
                let qualifiers = if *containment { " { composes }" } else { "" };
                let member = format!(
                    "property {}{back} : {}{}{qualifiers};",
                    quoted(name),
                    self.type_text(cx, t, c),
                    mult(*lower, *upper)
                );
                let mut members = vec![(c, member)];
                if let Some(o) = opposite {
                    if !cx.in_resource(t) {
                        return Err(Error::Invalid("the opposite would have to be added to another metamodel".into()));
                    }
                    cx.check_new_feature(t, o)?;
                    if t == c && o == name {
                        return Err(Error::Invalid("a reference cannot be its own opposite".into()));
                    }
                    let other = format!("property {}#{} : {}[?];", quoted(o), quoted(name), self.type_text(cx, c, t));
                    members.push((t, other));
                }
                let edits = if members.len() == 2 && members[0].0 == members[1].0 {
                    let joined = format!("{}\n{}", members[0].1, members[1].1);
                    vec![self.insert_feature(self.class_def(cx, c)?, &joined)]
                } else {
                    members
                        .iter()
                        .map(|(class, m)| Ok(self.insert_feature(self.class_def(cx, *class)?, m)))
                        .collect::<Result<_, Error>>()?
                };
                Ok((edits, Some(member_id(owner, name))))
            }
            MetamodelEdit::AddOperation { owner, name, ty, lower, upper, parameters } => {
                let c = cx.class(owner)?;
                cx.check_name(name)?;
                let params = self.parameters(cx, c, parameters)?;
                let result = match ty {
                    Some(t) if !t.is_empty() => {
                        let t = cx.resolve_type(t)?;
                        format!(" : {}{}", self.type_text(cx, t, c), mult(*lower, *upper))
                    }
                    _ => String::new(),
                };
                let member = format!("operation {}({params}){result};", quoted(name));
                Ok((vec![self.insert_operation(self.class_def(cx, c)?, &member)], Some(operation_id(owner, name))))
            }
            MetamodelEdit::AddLiteral { owner, name } => {
                let e = cx.enumeration(owner)?;
                cx.check_name(name)?;
                if mm.enum_literal_index(e, name).is_some() {
                    return Err(Error::Invalid(format!(
                        "'{}' already has a literal named '{name}'",
                        mm.classifier(e).name
                    )));
                }
                let cd = self.class_def(cx, e)?;
                let ClassifierDefKind::Enum { literals } = &cd.kind else { unreachable!("an enumeration") };
                let after =
                    literals.last().map(|l| self.statement(l.span)).or_else(|| self.last_annotation(&cd.annotations));
                let member = format!("literal {};", quoted(name));
                Ok((vec![self.insert_into(cd.span, cd.body, after, &member, false)], Some(member_id(owner, name))))
            }
            MetamodelEdit::AddSuperType { owner, super_type } => {
                let c = cx.class(owner)?;
                let s = cx.resolve_type(super_type)?;
                check_super_type(cx, c, s)?;
                let cd = self.class_def(cx, c)?;
                let ClassifierDefKind::Class { supers, .. } = &cd.kind else { unreachable!("a class") };
                let t = self.type_text(cx, s, c);
                let edit = match supers.last() {
                    Some(last) => ByteEdit::insert(last.span.end as usize, format!(", {t}")),
                    None => ByteEdit::insert(cd.name.span.end as usize, format!(" extends {t}")),
                };
                Ok((vec![edit], None))
            }
            MetamodelEdit::RemoveSuperType { owner, super_type } => {
                let c = cx.class(owner)?;
                let s = cx.resolve_type(super_type)?;
                let edit = self
                    .rewrite_extends(cx, c, &HashSet::from([s]))?
                    .ok_or_else(|| Error::NotFound(format!("'{super_type}' is not a supertype of '{owner}'")))?;
                Ok((vec![edit], None))
            }
            MetamodelEdit::Update { element, properties } => self.update(cx, element, properties),
            MetamodelEdit::Delete { elements } => {
                let deleted = cx.deleted(elements)?;
                let mut edits = Vec::new();
                for e in &deleted {
                    if let Some(id) = cx.id(*e) {
                        edits.push(ByteEdit::delete(removal(self.text, self.span_of(self.node(id)?), Some("--"))));
                    }
                }
                for c in cx.classes() {
                    if cx.is_deleted(Element::Classifier(c), &deleted) {
                        continue;
                    }
                    let gone: HashSet<ClassifierId> = mm
                        .super_types(c)
                        .iter()
                        .copied()
                        .filter(|s| deleted.contains(&Element::Classifier(*s)))
                        .collect();
                    if !gone.is_empty()
                        && let Some(edit) = self.rewrite_extends(cx, c, &gone)?
                    {
                        edits.push(edit);
                    }
                }
                for f in cx.features() {
                    if cx.is_deleted(Element::Feature(f), &deleted) {
                        continue;
                    }
                    if let Some(o) = mm.feature(f).opposite()
                        && deleted.contains(&Element::Feature(o))
                        && let Some(Ok(Node::Feature(fd))) = cx.id(Element::Feature(f)).map(|id| self.node(id))
                        && let Some(opp) = &fd.opposite
                    {
                        let hash = self.text[..opp.span.start as usize].trim_end().len().saturating_sub(1);
                        edits.push(ByteEdit::delete(hash..opp.span.end as usize));
                    }
                }
                Ok((edits, None))
            }
        }
    }

    /// Whether a type reference written in package `enclosing` names `target`.
    fn refers_to(&self, cx: &Context, t: &TypeRef, enclosing: PackageId, target: ClassifierId) -> bool {
        let mm = cx.mm;
        let cl = mm.classifier(target);
        let Some(last) = t.path.last() else { return false };
        if last.name != cl.name {
            return false;
        }
        if t.path.len() == 1 {
            return cl.package == enclosing
                || !mm.package(enclosing).classifiers.iter().any(|c| mm.classifier(*c).name == last.name);
        }
        let qualifier: Vec<&str> = t.path[..t.path.len() - 1].iter().map(|i| i.name.as_str()).collect();
        mm.package_qualified_name(cl.package).split("::").collect::<Vec<_>>().ends_with(&qualifier)
    }

    /// The `extends` clause of class `c` without the supertypes `remove` (`None` if it has none
    /// of them).
    fn rewrite_extends(
        &self,
        cx: &Context,
        c: ClassifierId,
        remove: &HashSet<ClassifierId>,
    ) -> Result<Option<ByteEdit>, Error> {
        let cd = self.class_def(cx, c)?;
        let (ClassifierDefKind::Class { supers, .. }, Some(extends)) = (&cd.kind, cd.extends) else {
            return Ok(None);
        };
        let package = cx.mm.classifier(c).package;
        let keep: Vec<&TypeRef> =
            supers.iter().filter(|t| !remove.iter().any(|r| self.refers_to(cx, t, package, *r))).collect();
        if keep.len() == supers.len() {
            return Ok(None);
        }
        Ok(Some(if keep.is_empty() {
            let start = self.text[..extends.start as usize].trim_end().len();
            ByteEdit::delete(start..extends.end as usize)
        } else {
            let list: Vec<&str> = keep.iter().map(|t| self.slice(t.span)).collect();
            ByteEdit::replace(extends.range(), format!("extends {}", list.join(", ")))
        }))
    }

    fn update(&self, cx: &Context, element: &str, p: &ElementUpdate) -> Result<(Vec<ByteEdit>, Option<String>), Error> {
        let mm = cx.mm;
        let mut edits = Vec::new();
        let mut renamed = None;
        let replace_name = |ident: &Ident, name: &str| ByteEdit::replace(ident.span.range(), quoted(name));
        match (cx.element(element)?, self.node(element)?) {
            (Element::Package(pid), Node::Package(pd)) => {
                if let Some(name) = p.name.as_ref().filter(|n| **n != pd.name.name) {
                    cx.check_name(name)?;
                    edits.push(replace_name(&pd.name, name));
                    // Qualified type references through the package.
                    let path: Vec<String> = mm.package_qualified_name(pid).split("::").map(str::to_string).collect();
                    for (t, _) in &self.refs {
                        let n = t.path.len();
                        if n > 1
                            && t.path[..n - 1]
                                .iter()
                                .map(|i| &i.name)
                                .eq(path[path.len().saturating_sub(n - 1)..].iter())
                        {
                            edits.push(replace_name(&t.path[n - 2], name));
                        }
                    }
                    renamed = Some(match element.rsplit_once("::") {
                        Some((parent, _)) => classifier_id(parent, name),
                        None => name.clone(),
                    });
                }
            }
            (Element::Classifier(c), Node::Classifier(cd)) => {
                let cl = mm.classifier(c);
                if let Some(name) = p.name.as_ref().filter(|n| **n != cl.name) {
                    cx.check_new_classifier(cl.package, name)?;
                    edits.push(replace_name(&cd.name, name));
                    for (t, package) in &self.refs {
                        if let Ok(Element::Package(enclosing)) = cx.element(package)
                            && self.refers_to(cx, t, enclosing, c)
                            && let Some(last) = t.path.last()
                        {
                            edits.push(replace_name(last, name));
                        }
                    }
                    let package = cx.id(Element::Package(cl.package)).unwrap_or_default();
                    renamed = Some(classifier_id(package, name));
                }
                let data = mm.class_data(c);
                if let Some(a) = p.is_abstract {
                    let data = data.ok_or_else(|| Error::Invalid(format!("'{element}' is not a class")))?;
                    let start = cd.span.start as usize;
                    if a && !data.is_abstract {
                        edits.push(ByteEdit::insert(start, "abstract "));
                    } else if !a && self.text[start..].starts_with("abstract") {
                        let rest = &self.text[start + "abstract".len()..];
                        let len = "abstract".len() + rest.len() - rest.trim_start().len();
                        edits.push(ByteEdit::delete(start..start + len));
                    }
                }
                if let Some(i) = p.is_interface {
                    data.ok_or_else(|| Error::Invalid(format!("'{element}' is not a class")))?;
                    let keyword = self.slice(cd.keyword);
                    if i && keyword == "class" && !cd.qualifiers.is_some_and(|q| self.slice(q).contains("interface")) {
                        edits.push(ByteEdit::replace(cd.keyword.range(), "interface"));
                    } else if !i && keyword == "interface" {
                        edits.push(ByteEdit::replace(cd.keyword.range(), "class"));
                    } else if !i && let Some(q) = cd.qualifiers {
                        let rest: Vec<&str> = self
                            .slice(q)
                            .trim_matches(['{', '}'])
                            .split(',')
                            .map(str::trim)
                            .filter(|w| !w.is_empty() && *w != "interface")
                            .collect();
                        edits.push(if rest.is_empty() {
                            ByteEdit::delete(self.text[..q.start as usize].trim_end().len()..q.end as usize)
                        } else {
                            ByteEdit::replace(q.range(), format!("{{ {} }}", rest.join(", ")))
                        });
                    }
                }
            }
            (Element::Feature(f), Node::Feature(fd)) => {
                let feature = mm.feature(f);
                if let Some(name) = p.name.as_ref().filter(|n| **n != feature.name) {
                    cx.check_new_feature(feature.owner, name)?;
                    edits.push(replace_name(&fd.name, name));
                    for g in cx.features() {
                        if mm.feature(g).opposite() == Some(f)
                            && let Some(Ok(Node::Feature(gd))) = cx.id(Element::Feature(g)).map(|id| self.node(id))
                            && let Some(o) = &gd.opposite
                        {
                            edits.push(replace_name(o, name));
                        }
                    }
                    renamed = cx.id(Element::Classifier(feature.owner)).map(|owner| member_id(owner, name));
                }
                let ty = p.ty.as_ref().map(|t| cx.feature_type(t, feature.is_reference())).transpose()?;
                let anchor = fd.opposite.as_ref().unwrap_or(&fd.name).span.end as usize;
                self.typed(
                    cx,
                    &mut edits,
                    feature.owner,
                    ty.map(Some),
                    (p.lower, p.upper),
                    feature.multiplicity,
                    fd.ty.as_ref(),
                    fd.multiplicity,
                    anchor,
                )?;
                let mut q = Qualifiers::new(&fd.qualifiers);
                if let Some(v) = p.containment {
                    if !feature.is_reference() {
                        return Err(Error::Invalid("only references can be containments".into()));
                    }
                    q.set("composes", v);
                }
                if let Some(v) = p.derived {
                    q.set("derived", v);
                }
                if let Some(v) = p.changeable {
                    q.set("readonly", !v);
                }
                if let Some(v) = p.transient {
                    q.set("transient", v);
                }
                if let Some(v) = p.volatile {
                    q.set("volatile", v);
                }
                if let Some(v) = p.is_id {
                    q.set("id", v);
                }
                if let Some(v) = p.ordered {
                    q.set("ordered", v);
                }
                if let Some(v) = p.unique {
                    q.negate("unique", !v);
                }
                edits.extend(self.qualifier_edit(&q, fd.qualifier_span, fd.body, fd.span));
            }
            (Element::Operation(o), Node::Operation(od)) => {
                let op = mm.operation(o);
                if let Some(name) = p.name.as_ref().filter(|n| **n != op.name) {
                    cx.check_name(name)?;
                    edits.push(replace_name(&od.name, name));
                    renamed = cx.id(Element::Classifier(op.owner)).map(|owner| operation_id(owner, name));
                }
                let ty = match p.ty.as_deref() {
                    Some("") => Some(None),
                    Some(t) => Some(Some(cx.resolve_type(t)?)),
                    None => None,
                };
                let anchor = od.params_span.end as usize;
                self.typed(
                    cx,
                    &mut edits,
                    op.owner,
                    ty,
                    (p.lower, p.upper),
                    op.multiplicity,
                    od.ty.as_ref(),
                    od.multiplicity,
                    anchor,
                )?;
                if let Some(params) = &p.parameters {
                    let text = self.parameters(cx, op.owner, params)?;
                    edits.push(ByteEdit::replace(od.params_span.range(), format!("({text})")));
                }
            }
            (Element::Literal(e, _), Node::Literal(ld)) => {
                if let Some(name) = p.name.as_ref().filter(|n| **n != ld.name.name) {
                    cx.check_name(name)?;
                    if mm.enum_literal_index(e, name).is_some() {
                        return Err(Error::Invalid(format!(
                            "'{}' already has a literal named '{name}'",
                            mm.classifier(e).name
                        )));
                    }
                    edits.push(replace_name(&ld.name, name));
                    renamed = cx.id(Element::Classifier(e)).map(|owner| member_id(owner, name));
                }
            }
            _ => return Err(Error::Invalid(format!("'{element}' cannot be changed"))),
        }
        Ok((edits, renamed))
    }

    /// Edits the type (`Some(None)` removes it) and multiplicity written after `anchor`.
    #[allow(clippy::too_many_arguments)]
    fn typed(
        &self,
        cx: &Context,
        edits: &mut Vec<ByteEdit>,
        owner: ClassifierId,
        ty: Option<Option<ClassifierId>>,
        (lower, upper): (Option<i32>, Option<i32>),
        current: Multiplicity,
        type_ref: Option<&TypeRef>,
        written: Option<MultiplicityDef>,
        anchor: usize,
    ) -> Result<(), Error> {
        let m = (lower.is_some() || upper.is_some())
            .then(|| mult(lower.unwrap_or(current.lower), upper.unwrap_or(current.upper)));
        match (type_ref, ty) {
            (Some(t), Some(None)) => {
                let end = written.map_or(t.span.end, |w| w.span.end) as usize;
                edits.push(ByteEdit::delete(anchor..end));
                return Ok(());
            }
            (None, Some(None)) | (None, None) => {
                if m.is_some() {
                    return Err(Error::Invalid("a multiplicity needs a type".into()));
                }
                return Ok(());
            }
            (None, Some(Some(t))) => {
                let text = self.type_text(cx, t, owner);
                edits.push(ByteEdit::insert(anchor, format!(" : {text}{}", m.unwrap_or_default())));
                return Ok(());
            }
            (Some(tr), Some(Some(t))) => edits.push(ByteEdit::replace(tr.span.range(), self.type_text(cx, t, owner))),
            (Some(_), None) => {}
        }
        if let (Some(m), Some(tr)) = (m, type_ref) {
            match written {
                Some(w) if m.is_empty() => {
                    let start = self.text[..w.span.start as usize].trim_end_matches([' ', '\t']).len();
                    edits.push(ByteEdit::delete(start..w.span.end as usize));
                }
                Some(w) => edits.push(ByteEdit::replace(w.span.range(), m)),
                None if !m.is_empty() => edits.push(ByteEdit::insert(tr.span.end as usize, m)),
                None => {}
            }
        }
        Ok(())
    }

    /// The edit that writes a changed qualifier list of an element (`span`) with an optional
    /// qualifier list (`list`) and body.
    fn qualifier_edit(&self, q: &Qualifiers, list: Option<Span>, body: Option<Span>, span: Span) -> Option<ByteEdit> {
        if !q.changed {
            return None;
        }
        match list {
            Some(l) if q.tokens.is_empty() => {
                Some(ByteEdit::delete(self.text[..l.start as usize].trim_end().len()..l.end as usize))
            }
            Some(l) => Some(ByteEdit::replace(l.range(), q.text())),
            None if q.tokens.is_empty() => None,
            None => Some(match body {
                Some(b) => ByteEdit::insert(b.start as usize, format!("{} ", q.text())),
                None => {
                    let end = span.end as usize;
                    let at = if self.text[..end].ends_with(';') { end - 1 } else { end };
                    ByteEdit::insert(at, format!(" {}", q.text()))
                }
            }),
        }
    }
}

/// Checks that `s` can become a supertype of `c`.
pub(crate) fn check_super_type(cx: &Context, c: ClassifierId, s: ClassifierId) -> Result<(), Error> {
    let mm = cx.mm;
    if !mm.is_class(s) {
        return Err(Error::Invalid(format!("'{}' is not a class", mm.classifier(s).name)));
    }
    if s == c || mm.conforms_to(s, c) {
        return Err(Error::Invalid(format!(
            "'{}' cannot extend '{}': the inheritance would be cyclic",
            mm.classifier(c).name,
            mm.classifier(s).name
        )));
    }
    if mm.super_types(c).contains(&s) {
        return Err(Error::Invalid(format!("'{}' already extends '{}'", mm.classifier(c).name, mm.classifier(s).name)));
    }
    Ok(())
}

/// A multiplicity as OCLinEcore writes it.
fn mult(lower: i32, upper: i32) -> String {
    multiplicity(&Multiplicity { lower, upper, ..Multiplicity::default() })
}

/// A qualifier list being changed.
struct Qualifiers {
    tokens: Vec<(String, bool)>,
    changed: bool,
}

impl Qualifiers {
    fn new(list: &[Qualifier]) -> Self {
        Qualifiers { tokens: list.iter().map(|q| (q.name.clone(), q.negated)).collect(), changed: false }
    }

    /// Writes `name` (or nothing) for a property that is off by default.
    fn set(&mut self, name: &str, on: bool) {
        let current = self.tokens.iter().any(|(n, neg)| n == name && !neg);
        let negated = self.tokens.iter().any(|(n, neg)| n == name && *neg);
        if current == on && !negated {
            return;
        }
        self.tokens.retain(|(n, _)| n != name);
        if on {
            self.tokens.push((name.to_string(), false));
        }
        self.changed = true;
    }

    /// Writes `!name` (or nothing) for a property that is on by default.
    fn negate(&mut self, name: &str, negated: bool) {
        let current = self.tokens.iter().any(|(n, neg)| n == name && *neg);
        let plain = self.tokens.iter().any(|(n, neg)| n == name && !neg);
        if current == negated && !plain {
            return;
        }
        self.tokens.retain(|(n, _)| n != name);
        if negated {
            self.tokens.push((name.to_string(), true));
        }
        self.changed = true;
    }

    fn text(&self) -> String {
        let list: Vec<String> =
            self.tokens.iter().map(|(n, neg)| if *neg { format!("!{n}") } else { n.clone() }).collect();
        format!("{{ {} }}", list.join(", "))
    }
}
