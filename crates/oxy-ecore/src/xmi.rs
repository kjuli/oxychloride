//! Loading XMI instance models (as written by EMF's `XMIResource`).

use std::path::Path;

use crate::ecore_loader::{canonical_uri, resolve_relative};
use crate::error::LoadError;
use crate::metamodel::*;
use crate::model::*;
use crate::xml::{self, Element, XMI_NS, XSI_NS};

/// A parsed but not yet loaded XMI document. Parsing first lets callers inspect which
/// metamodels are needed (and where `xsi:schemaLocation` says to find them).
#[derive(Debug, Clone)]
pub struct XmiDocument {
    pub uri: String,
    text: String,
    root: Element,
}

/// The outcome of loading an XMI document.
#[derive(Debug, Clone)]
pub struct XmiLoad {
    /// Index of the resource in [`Model::resources`].
    pub resource: usize,
    pub roots: Vec<ObjectId>,
    pub warnings: Vec<LoadError>,
    /// Every object created, with the byte offset of its element's start tag.
    pub objects: Vec<(ObjectId, usize)>,
}

impl XmiDocument {
    pub fn parse(text: impl Into<String>, uri: impl Into<String>) -> Result<Self, LoadError> {
        let text = text.into();
        let uri = uri.into();
        let root = xml::parse(&text, &uri)?;
        Ok(XmiDocument { uri, text, root })
    }

    pub fn read_file(path: impl AsRef<Path>) -> Result<Self, LoadError> {
        let uri = canonical_uri(path.as_ref());
        let text = std::fs::read_to_string(&uri).map_err(|e| LoadError::new(&uri, format!("cannot read file: {e}")))?;
        Self::parse(text, uri)
    }

    /// The namespace URIs of the packages the document's objects are typed by.
    pub fn used_ns_uris(&self) -> Vec<String> {
        let mut out = Vec::new();
        collect_ns(&self.root, &mut out);
        out
    }

    /// `(nsURI, location)` pairs from `xsi:schemaLocation`, with locations resolved
    /// relative to the document.
    pub fn schema_locations(&self) -> Vec<(String, String)> {
        let Some(value) = self.root.attr_ns(XSI_NS, "schemaLocation") else { return Vec::new() };
        let tokens: Vec<&str> = value.split_whitespace().collect();
        tokens
            .chunks(2)
            .filter(|c| c.len() == 2)
            .map(|c| {
                let location = c[1].split('#').next().unwrap_or(c[1]);
                (c[0].to_string(), resolve_relative(&self.uri, location))
            })
            .collect()
    }

    /// URIs of other resources this document references (resolved against its location),
    /// e.g. a PCM system referencing its repository. `pathmap:` and `platform:` URIs are
    /// included as written.
    pub fn referenced_uris(&self) -> Vec<String> {
        let mut out = Vec::new();
        collect_hrefs(&self.root, &self.uri, &mut out);
        out
    }

    /// Loads the document into `model`. Every package used must already be in `mm`.
    pub fn load(&self, mm: &Metamodel, model: &mut Model) -> Result<XmiLoad, LoadError> {
        if let Some(resource) = model.resource_of_uri(&self.uri) {
            let roots = model.resources()[resource].roots.clone();
            return Ok(XmiLoad { resource, roots, warnings: Vec::new(), objects: Vec::new() });
        }
        let root_elements: Vec<&Element> = if self.root.local_name() == "XMI" && self.root.namespace() == Some(XMI_NS) {
            self.root.children.iter().collect()
        } else {
            vec![&self.root]
        };
        let resource = model.add_resource(self.uri.clone());
        let mut loader =
            Loader { doc: self, mm, model, resource, pending: Vec::new(), warnings: Vec::new(), objects: Vec::new() };
        let mut roots = Vec::new();
        for el in root_elements {
            let class = loader.root_class(el)?;
            let id = loader.object(el, class, None)?;
            roots.push(id);
        }
        let Loader { pending, mut warnings, objects, .. } = loader;
        for p in pending {
            resolve(self, mm, model, resource, p, &mut warnings);
        }
        // References from previously loaded resources into this one.
        for r in model.resolve_deferred(mm) {
            if r.uri == self.uri {
                let source = &model.resources()[model.object(r.object).resource].uri;
                warnings.push(LoadError::new(
                    source.clone(),
                    format!("cannot resolve reference '{}#{}'", self.uri, r.fragment),
                ));
            }
        }
        Ok(XmiLoad { resource, roots, warnings, objects })
    }
}

fn collect_hrefs(el: &Element, base: &str, out: &mut Vec<String>) {
    for (k, v) in &el.attrs {
        if k.contains(':') {
            continue; // xmlns:*, xsi:*, xmi:* are not references
        }
        for token in v.split_whitespace() {
            if let Some((uri, _)) = token.split_once('#')
                && !uri.is_empty()
            {
                let resolved = resolve_relative(base, uri);
                if !out.contains(&resolved) {
                    out.push(resolved);
                }
            }
        }
    }
    for c in &el.children {
        collect_hrefs(c, base, out);
    }
}

fn collect_ns(el: &Element, out: &mut Vec<String>) {
    for (k, v) in &el.attrs {
        if Element::is_xmlns_attr(k) && v != XMI_NS && v != XSI_NS && !out.contains(v) {
            out.push(v.clone());
        }
    }
    for c in &el.children {
        collect_ns(c, out);
    }
}

struct PendingRef {
    object: ObjectId,
    feature: FeatureId,
    value: String,
    offset: usize,
}

struct Loader<'a> {
    doc: &'a XmiDocument,
    mm: &'a Metamodel,
    model: &'a mut Model,
    resource: usize,
    pending: Vec<PendingRef>,
    warnings: Vec<LoadError>,
    objects: Vec<(ObjectId, usize)>,
}

impl Loader<'_> {
    fn error(&self, offset: usize, message: impl Into<String>) -> LoadError {
        LoadError::at(&self.doc.uri, &self.doc.text, offset, message)
    }

    fn warn(&mut self, offset: usize, message: impl Into<String>) {
        let w = self.error(offset, message);
        self.warnings.push(w);
    }

    fn root_class(&self, el: &Element) -> Result<ClassifierId, LoadError> {
        if let Some(t) = el.attr_ns(XSI_NS, "type") {
            return self.resolve_type_name(el, t);
        }
        self.resolve_type_name(el, &el.name)
    }

    /// Resolves `prefix:ClassName` using the namespaces in scope at `el`.
    fn resolve_type_name(&self, el: &Element, qname: &str) -> Result<ClassifierId, LoadError> {
        let (ns, name) = el.resolve_qname_value(qname);
        let ns = ns.ok_or_else(|| self.error(el.offset, format!("no namespace declared for '{qname}'")))?;
        let package = self.mm.package_by_ns_uri(ns).ok_or_else(|| {
            self.error(
                el.offset,
                format!("unknown package nsURI '{ns}' (register its metamodel, or load its .ecore file)"),
            )
        })?;
        let class = self
            .mm
            .find_classifier_in(package, name)
            .ok_or_else(|| self.error(el.offset, format!("package '{ns}' has no classifier '{name}'")))?;
        if !self.mm.is_class(class) {
            return Err(self.error(el.offset, format!("'{name}' is not a class")));
        }
        Ok(class)
    }

    fn object(
        &mut self,
        el: &Element,
        class: ClassifierId,
        container: Option<(ObjectId, FeatureId)>,
    ) -> Result<ObjectId, LoadError> {
        if self.mm.class_data(class).is_some_and(|c| c.is_abstract || c.is_interface) {
            self.warn(el.offset, format!("instance of abstract class '{}'", self.mm.classifier(class).name));
        }
        let id = self.model.create_object(self.mm, class, self.resource, container);
        self.objects.push((id, el.offset));
        for (key, value) in &el.attrs {
            let (prefix, local) = xml::split_qname(key);
            if Element::is_xmlns_attr(key) {
                continue;
            }
            if let Some(p) = prefix {
                if el.namespaces.get(p).is_some_and(|ns| ns == XMI_NS) && local == "id" {
                    self.model.set_xmi_id(id, value.clone());
                }
                continue; // xmi:*, xsi:* and other qualified attributes are not features
            }
            self.attribute_value(el, id, class, local, value);
        }
        for child in &el.children {
            self.child_element(child, id, class)?;
        }
        Ok(id)
    }

    fn attribute_value(&mut self, el: &Element, id: ObjectId, class: ClassifierId, name: &str, value: &str) {
        let Some(feature) = self.mm.lookup_feature(class, name) else {
            self.warn(el.offset, format!("class '{}' has no feature '{name}'", self.mm.classifier(class).name));
            return;
        };
        let f = self.mm.feature(feature);
        if f.is_reference() {
            self.pending.push(PendingRef { object: id, feature, value: value.to_string(), offset: el.offset });
            return;
        }
        let Some(ty) = f.ty else {
            self.model.add(self.mm, id, feature, ModelValue::String(value.to_string()));
            return;
        };
        match parse_value(self.mm, ty, value) {
            Ok(v) => {
                if matches!(f.kind, FeatureKind::Attribute { is_id: true }) {
                    self.model.register_id(id, value);
                }
                self.model.add(self.mm, id, feature, v)
            }
            Err(e) => self.warn(el.offset, format!("feature '{name}': {e}")),
        }
    }

    fn child_element(&mut self, child: &Element, id: ObjectId, class: ClassifierId) -> Result<(), LoadError> {
        let name = child.local_name();
        let Some(feature) = self.mm.lookup_feature(class, name) else {
            self.warn(child.offset, format!("class '{}' has no feature '{name}'", self.mm.classifier(class).name));
            return Ok(());
        };
        let f = self.mm.feature(feature);
        if let Some(href) = child.attr("href") {
            self.pending.push(PendingRef { object: id, feature, value: href.to_string(), offset: child.offset });
            return Ok(());
        }
        if f.is_containment() {
            let child_class = match child.attr_ns(XSI_NS, "type") {
                Some(t) => self.resolve_type_name(child, t)?,
                None => f.ty.ok_or_else(|| self.error(child.offset, format!("feature '{name}' has no type")))?,
            };
            self.object(child, child_class, Some((id, feature)))?;
        } else if f.is_reference() {
            // Non-containment reference serialized as an element with an idref.
            if let Some(v) = child.attr_ns(XMI_NS, "idref") {
                self.pending.push(PendingRef { object: id, feature, value: v.to_string(), offset: child.offset });
            }
        } else if let Some(ty) = f.ty {
            match parse_value(self.mm, ty, &child.text) {
                Ok(v) => self.model.add(self.mm, id, feature, v),
                Err(e) => self.warn(child.offset, format!("feature '{name}': {e}")),
            }
        } else {
            self.model.add(self.mm, id, feature, ModelValue::String(child.text.clone()));
        }
        Ok(())
    }
}

fn resolve(
    doc: &XmiDocument,
    mm: &Metamodel,
    model: &mut Model,
    resource: usize,
    p: PendingRef,
    warnings: &mut Vec<LoadError>,
) {
    let tokens: Vec<&str> = p.value.split_whitespace().collect();
    let mut i = 0;
    while i < tokens.len() {
        let token = tokens[i];
        i += 1;
        // `prefix:Type uri#fragment` — skip the type hint.
        if !token.contains('#') && !token.starts_with('/') && token.contains(':') && i < tokens.len() {
            continue;
        }
        let (uri, fragment) = token.split_once('#').unwrap_or(("", token));
        let target_resource = if uri.is_empty() {
            resource
        } else {
            let resolved = resolve_relative(&doc.uri, uri);
            match model.resource_of_uri(&resolved) {
                Some(r) => r,
                None => {
                    model.defer(UnresolvedReference {
                        object: p.object,
                        feature: p.feature,
                        uri: resolved,
                        fragment: fragment.to_string(),
                    });
                    continue;
                }
            }
        };
        let target = model.resolve_fragment(mm, target_resource, fragment);
        match target {
            Some(t) if mm.feature(p.feature).ty.is_some_and(|ty| !mm.conforms_to(model.class_of(t), ty)) => {
                warnings.push(LoadError::at(
                    &doc.uri,
                    &doc.text,
                    p.offset,
                    format!(
                        "reference '{token}' points to a {}, but feature '{}' expects a {}",
                        mm.classifier(model.class_of(t)).name,
                        mm.feature(p.feature).name,
                        mm.classifier(mm.feature(p.feature).ty.unwrap()).name
                    ),
                ));
            }
            Some(t) => model.add(mm, p.object, p.feature, ModelValue::Object(t)),
            None => warnings.push(LoadError::at(
                &doc.uri,
                &doc.text,
                p.offset,
                format!("cannot resolve reference '{token}'"),
            )),
        }
    }
}
