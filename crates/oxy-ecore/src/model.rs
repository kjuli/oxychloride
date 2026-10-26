//! Dynamic instance models (the "M1" level) conforming to a [`Metamodel`].

use std::collections::HashMap;
use std::fmt;

use crate::metamodel::*;

/// An object in a [`Model`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId(pub u32);

impl ObjectId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A single value stored in a structural feature.
#[derive(Clone, Debug, PartialEq)]
pub enum ModelValue {
    Boolean(bool),
    Integer(i64),
    Real(f64),
    String(String),
    /// An enumeration literal: the enum and the index of the literal.
    Enum(ClassifierId, usize),
    Object(ObjectId),
}

impl fmt::Display for ModelValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelValue::Boolean(b) => write!(f, "{b}"),
            ModelValue::Integer(i) => write!(f, "{i}"),
            ModelValue::Real(r) => write!(f, "{r}"),
            ModelValue::String(s) => write!(f, "'{s}'"),
            ModelValue::Enum(e, i) => write!(f, "#{}:{}", e.0, i),
            ModelValue::Object(o) => write!(f, "@{}", o.0),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Object {
    pub class: ClassifierId,
    /// Containing object and the containment feature holding this object.
    pub container: Option<(ObjectId, FeatureId)>,
    /// Index into [`Model::resources`].
    pub resource: usize,
    pub xmi_id: Option<String>,
    values: Vec<(FeatureId, Vec<ModelValue>)>,
    /// Deleted by [`Model::delete`]: no longer part of any resource.
    deleted: bool,
}

impl Object {
    /// Features that have a value, in the order they were set.
    pub fn set_features(&self) -> impl Iterator<Item = (FeatureId, &[ModelValue])> {
        self.values.iter().map(|(f, v)| (*f, v.as_slice()))
    }
}

/// A file (or other unit) an instance model was loaded from.
#[derive(Debug, Clone)]
pub struct ModelResource {
    pub uri: String,
    /// Other URIs this resource is known by (e.g. `pathmap://...` URIs mapped to the file).
    pub aliases: Vec<String>,
    pub roots: Vec<ObjectId>,
    pub(crate) ids: HashMap<String, ObjectId>,
}

/// A reference into a resource that is not loaded (yet). It is resolved automatically when
/// a resource with that URI is loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedReference {
    pub object: ObjectId,
    pub feature: FeatureId,
    /// Resolved URI of the target resource.
    pub uri: String,
    pub fragment: String,
}

/// A set of objects, possibly spread over several resources that may reference each other.
#[derive(Debug, Clone, Default)]
pub struct Model {
    objects: Vec<Object>,
    resources: Vec<ModelResource>,
    unresolved: Vec<UnresolvedReference>,
}

impl Model {
    pub fn new() -> Self {
        Model::default()
    }

    pub fn resources(&self) -> &[ModelResource] {
        &self.resources
    }

    pub fn resource_of_uri(&self, uri: &str) -> Option<usize> {
        self.resources.iter().position(|r| r.uri == uri || r.aliases.iter().any(|a| a == uri))
    }

    /// Makes a resource reachable under another URI as well.
    pub fn add_alias(&mut self, resource: usize, alias: impl Into<String>) {
        let alias = alias.into();
        if !self.resources[resource].aliases.contains(&alias) {
            self.resources[resource].aliases.push(alias);
        }
    }

    /// Adds an (empty) resource and returns its index.
    pub fn add_resource(&mut self, uri: impl Into<String>) -> usize {
        self.resources.push(ModelResource {
            uri: uri.into(),
            aliases: Vec::new(),
            roots: Vec::new(),
            ids: HashMap::new(),
        });
        self.resources.len() - 1
    }

    pub fn object(&self, id: ObjectId) -> &Object {
        &self.objects[id.index()]
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    pub fn object_ids(&self) -> impl Iterator<Item = ObjectId> + '_ {
        (0..self.objects.len() as u32).map(ObjectId)
    }

    pub fn class_of(&self, id: ObjectId) -> ClassifierId {
        self.objects[id.index()].class
    }

    /// All objects whose class conforms to `class`.
    pub fn instances_of(&self, mm: &Metamodel, class: ClassifierId) -> Vec<ObjectId> {
        self.object_ids().filter(|o| !self.is_deleted(*o) && mm.conforms_to(self.class_of(*o), class)).collect()
    }

    /// All objects whose class is exactly `class`.
    pub fn instances_of_exact(&self, class: ClassifierId) -> Vec<ObjectId> {
        self.object_ids().filter(|o| !self.is_deleted(*o) && self.class_of(*o) == class).collect()
    }

    /// Creates a new object. If `container` is `None`, the object becomes a root of `resource`.
    pub fn create_object(
        &mut self,
        mm: &Metamodel,
        class: ClassifierId,
        resource: usize,
        container: Option<(ObjectId, FeatureId)>,
    ) -> ObjectId {
        let id = ObjectId(self.objects.len() as u32);
        self.objects.push(Object {
            class,
            container: None,
            resource,
            xmi_id: None,
            values: Vec::new(),
            deleted: false,
        });
        match container {
            Some((parent, feature)) => self.add(mm, parent, feature, ModelValue::Object(id)),
            None => self.resources[resource].roots.push(id),
        }
        id
    }

    pub fn set_xmi_id(&mut self, id: ObjectId, xmi_id: impl Into<String>) {
        let xmi_id = xmi_id.into();
        let resource = self.objects[id.index()].resource;
        self.resources[resource].ids.insert(xmi_id.clone(), id);
        self.objects[id.index()].xmi_id = Some(xmi_id);
    }

    /// Registers an intrinsic ID (the value of an `iD` attribute) for fragment resolution.
    pub fn register_id(&mut self, id: ObjectId, value: impl Into<String>) {
        let resource = self.objects[id.index()].resource;
        self.resources[resource].ids.entry(value.into()).or_insert(id);
    }

    /// References into resources that have not been loaded.
    pub fn unresolved_references(&self) -> &[UnresolvedReference] {
        &self.unresolved
    }

    pub(crate) fn defer(&mut self, r: UnresolvedReference) {
        self.unresolved.push(r);
    }

    /// Resolves deferred references whose target resource is now loaded. References whose
    /// target object does not exist (or has the wrong type) are dropped and returned.
    pub fn resolve_deferred(&mut self, mm: &Metamodel) -> Vec<UnresolvedReference> {
        let mut dropped = Vec::new();
        for r in std::mem::take(&mut self.unresolved) {
            let Some(resource) = self.resource_of_uri(&r.uri) else {
                self.unresolved.push(r);
                continue;
            };
            match self.resolve_fragment(mm, resource, &r.fragment) {
                Some(t) if mm.feature(r.feature).ty.is_none_or(|ty| mm.conforms_to(self.class_of(t), ty)) => {
                    self.add(mm, r.object, r.feature, ModelValue::Object(t));
                }
                _ => dropped.push(r),
            }
        }
        dropped
    }

    pub fn object_by_xmi_id(&self, resource: usize, xmi_id: &str) -> Option<ObjectId> {
        self.resources.get(resource)?.ids.get(xmi_id).copied()
    }

    /// The raw values of a feature (empty if unset). Does not apply default values.
    pub fn get(&self, id: ObjectId, feature: FeatureId) -> &[ModelValue] {
        self.objects[id.index()].values.iter().find(|(f, _)| *f == feature).map(|(_, v)| v.as_slice()).unwrap_or(&[])
    }

    pub fn is_set(&self, id: ObjectId, feature: FeatureId) -> bool {
        !self.get(id, feature).is_empty()
    }

    /// The value of a feature, falling back to its default value when unset.
    /// Single-valued features yield at most one value.
    pub fn get_or_default(&self, mm: &Metamodel, id: ObjectId, feature: FeatureId) -> Vec<ModelValue> {
        let values = self.get(id, feature);
        if !values.is_empty() || mm.feature(feature).is_many() {
            return values.to_vec();
        }
        default_value(mm, feature).into_iter().collect()
    }

    fn slot_mut(&mut self, id: ObjectId, feature: FeatureId) -> &mut Vec<ModelValue> {
        let values = &mut self.objects[id.index()].values;
        let pos = match values.iter().position(|(f, _)| *f == feature) {
            Some(p) => p,
            None => {
                values.push((feature, Vec::new()));
                values.len() - 1
            }
        };
        &mut values[pos].1
    }

    /// Adds a value to a feature, maintaining containment and opposite references.
    /// For single-valued features the previous value is replaced.
    pub fn add(&mut self, mm: &Metamodel, id: ObjectId, feature: FeatureId, value: ModelValue) {
        let f = mm.feature(feature);
        let many = f.is_many();
        let unique = f.multiplicity.unique;
        {
            let slot = self.slot_mut(id, feature);
            if many {
                if unique && slot.contains(&value) {
                    return;
                }
                slot.push(value.clone());
            } else {
                slot.clear();
                slot.push(value.clone());
            }
        }
        let ModelValue::Object(target) = value else { return };
        if f.is_containment() {
            self.objects[target.index()].container = Some((id, feature));
        }
        if let Some(opposite) = f.opposite() {
            let back = ModelValue::Object(id);
            if !self.get(target, opposite).contains(&back) {
                let op = mm.feature(opposite);
                let slot = self.slot_mut(target, opposite);
                if !op.is_many() {
                    slot.clear();
                }
                slot.push(back);
                if op.is_containment() {
                    self.objects[id.index()].container = Some((target, opposite));
                }
            }
        }
    }

    /// Replaces all values of a feature (without maintaining opposites).
    pub fn set_raw(&mut self, id: ObjectId, feature: FeatureId, values: Vec<ModelValue>) {
        *self.slot_mut(id, feature) = values;
    }

    /// Adds a value to a feature as EMF does: a single-valued feature loses its previous value
    /// (and that value its back-link), an object added to a containment feature leaves its
    /// previous container (or the roots of its resource), and a single-valued opposite drops
    /// the link it had before.
    pub fn insert(&mut self, mm: &Metamodel, id: ObjectId, feature: FeatureId, value: ModelValue) {
        let f = mm.feature(feature);
        if f.is_many() {
            if f.multiplicity.unique && self.get(id, feature).contains(&value) {
                return;
            }
        } else {
            if self.get(id, feature) == std::slice::from_ref(&value) {
                return;
            }
            self.clear(mm, id, feature);
        }
        if let ModelValue::Object(target) = value {
            if f.is_containment() {
                self.detach(mm, target);
            }
            if let Some(opposite) = f.opposite() {
                let op = mm.feature(opposite);
                if !op.is_many() {
                    let previous: Vec<ModelValue> = self.get(target, opposite).to_vec();
                    for p in previous {
                        if p != ModelValue::Object(id) {
                            self.remove(mm, target, opposite, &p);
                        }
                    }
                }
                // Becoming contained by `target`: out of the previous container (or the roots).
                if op.is_containment() && self.objects[id.index()].container != Some((target, opposite)) {
                    self.detach(mm, id);
                }
            }
        }
        self.add(mm, id, feature, value);
    }

    /// Replaces all values of a feature (EMF's `eSet`), maintaining containment and opposites.
    pub fn set(&mut self, mm: &Metamodel, id: ObjectId, feature: FeatureId, values: Vec<ModelValue>) {
        self.clear(mm, id, feature);
        for v in values {
            self.insert(mm, id, feature, v);
        }
    }

    /// Removes all values of a feature, maintaining containment and opposites.
    pub fn clear(&mut self, mm: &Metamodel, id: ObjectId, feature: FeatureId) {
        let values: Vec<ModelValue> = self.get(id, feature).to_vec();
        for v in values.iter().rev() {
            self.remove(mm, id, feature, v);
        }
    }

    /// Removes one occurrence of a value from a feature, with the back-link of its opposite. An
    /// object removed from a containment feature has no container any more (and is not a root
    /// either: add it somewhere, or [`Model::make_root`]). Returns whether it was there.
    pub fn remove(&mut self, mm: &Metamodel, id: ObjectId, feature: FeatureId, value: &ModelValue) -> bool {
        let slot = self.slot_mut(id, feature);
        let Some(pos) = slot.iter().position(|v| v == value) else { return false };
        slot.remove(pos);
        let ModelValue::Object(target) = value else { return true };
        let f = mm.feature(feature);
        if f.is_containment() && self.objects[target.index()].container == Some((id, feature)) {
            self.objects[target.index()].container = None;
        }
        if let Some(opposite) = f.opposite() {
            let back = ModelValue::Object(id);
            let slot = self.slot_mut(*target, opposite);
            if let Some(p) = slot.iter().position(|v| *v == back) {
                slot.remove(p);
            }
            if mm.feature(opposite).is_containment() && self.objects[id.index()].container == Some((*target, opposite))
            {
                self.objects[id.index()].container = None;
            }
        }
        true
    }

    /// Takes an object out of its container, or out of the roots of its resource.
    pub fn detach(&mut self, mm: &Metamodel, id: ObjectId) {
        match self.objects[id.index()].container {
            Some((parent, feature)) => {
                self.remove(mm, parent, feature, &ModelValue::Object(id));
                self.objects[id.index()].container = None;
            }
            None => {
                let resource = self.objects[id.index()].resource;
                self.resources[resource].roots.retain(|r| *r != id);
            }
        }
    }

    /// Replaces the roots of a resource (objects that are no root any more are not part of it,
    /// unless contained by one).
    pub fn set_roots(&mut self, resource: usize, roots: Vec<ObjectId>) {
        self.resources[resource].roots = roots;
    }

    /// Makes an object without a container a root of its resource.
    pub fn make_root(&mut self, id: ObjectId) {
        let object = &self.objects[id.index()];
        if object.container.is_none() {
            let resource = object.resource;
            if !self.resources[resource].roots.contains(&id) {
                self.resources[resource].roots.push(id);
            }
        }
    }

    /// Whether an object was deleted (see [`Model::delete`]).
    pub fn is_deleted(&self, id: ObjectId) -> bool {
        self.objects[id.index()].deleted
    }

    /// Deletes an object and its contents (EMF's `EcoreUtil.delete`): they leave their container
    /// or resource, and every reference to them is removed. Their ids are not reused.
    pub fn delete(&mut self, mm: &Metamodel, id: ObjectId) {
        let mut doomed = vec![id];
        doomed.extend(self.all_contents(mm, id));
        self.detach(mm, id);
        let doomed_set: std::collections::HashSet<ObjectId> = doomed.iter().copied().collect();
        for o in 0..self.objects.len() {
            let o = ObjectId(o as u32);
            if doomed_set.contains(&o) || self.objects[o.index()].deleted {
                continue;
            }
            for (_, values) in &mut self.objects[o.index()].values {
                values.retain(|v| !matches!(v, ModelValue::Object(t) if doomed_set.contains(t)));
            }
        }
        for d in doomed {
            let object = &mut self.objects[d.index()];
            object.values.clear();
            object.container = None;
            object.deleted = true;
            let resource = object.resource;
            self.resources[resource].roots.retain(|r| *r != d);
            self.resources[resource].ids.retain(|_, o| *o != d);
        }
    }

    /// Direct children of an object via containment features.
    pub fn contents(&self, mm: &Metamodel, id: ObjectId) -> Vec<ObjectId> {
        let mut out = Vec::new();
        for f in mm.all_features(self.class_of(id)) {
            if mm.feature(*f).is_containment() {
                out.extend(self.get(id, *f).iter().filter_map(|v| match v {
                    ModelValue::Object(o) => Some(*o),
                    _ => None,
                }));
            }
        }
        out
    }

    /// All transitive children of an object.
    pub fn all_contents(&self, mm: &Metamodel, id: ObjectId) -> Vec<ObjectId> {
        let mut out = Vec::new();
        let mut stack = self.contents(mm, id);
        stack.reverse();
        while let Some(o) = stack.pop() {
            out.push(o);
            let mut children = self.contents(mm, o);
            children.reverse();
            stack.extend(children);
        }
        out
    }

    pub fn container(&self, id: ObjectId) -> Option<ObjectId> {
        self.objects[id.index()].container.map(|(c, _)| c)
    }

    /// The EMF-style URI fragment of an object, e.g. `//@books.0`.
    pub fn uri_fragment(&self, mm: &Metamodel, id: ObjectId) -> String {
        let mut segments = Vec::new();
        let mut cur = id;
        while let Some((parent, feature)) = self.objects[cur.index()].container {
            let f = mm.feature(feature);
            if f.is_many() {
                let index = self.get(parent, feature).iter().position(|v| *v == ModelValue::Object(cur)).unwrap_or(0);
                segments.push(format!("@{}.{}", f.name, index));
            } else {
                segments.push(format!("@{}", f.name));
            }
            cur = parent;
        }
        let resource = &self.resources[self.objects[cur.index()].resource];
        let root_index = resource.roots.iter().position(|r| *r == cur).unwrap_or(0);
        let root = if root_index == 0 { String::new() } else { root_index.to_string() };
        segments.reverse();
        if segments.is_empty() { format!("/{root}") } else { format!("/{root}/{}", segments.join("/")) }
    }

    /// Resolves an EMF-style URI fragment (`//@books.0`, `/1/@x`, name-based `//Book`) or an xmi:id.
    pub fn resolve_fragment(&self, mm: &Metamodel, resource: usize, fragment: &str) -> Option<ObjectId> {
        let res = self.resources.get(resource)?;
        let Some(rest) = fragment.strip_prefix('/') else {
            return res.ids.get(fragment).copied();
        };
        let mut segments = rest.split('/');
        let root_seg = segments.next().unwrap_or("");
        let root_index: usize = if root_seg.is_empty() { 0 } else { root_seg.parse().ok()? };
        let mut cur = *res.roots.get(root_index)?;
        for seg in segments.filter(|s| !s.is_empty()) {
            cur = self.resolve_segment(mm, cur, seg)?;
        }
        Some(cur)
    }

    fn resolve_segment(&self, mm: &Metamodel, cur: ObjectId, seg: &str) -> Option<ObjectId> {
        if let Some(feature_seg) = seg.strip_prefix('@') {
            let (name, index) = match feature_seg.rsplit_once('.') {
                Some((n, i)) if i.chars().all(|c| c.is_ascii_digit()) => (n, i.parse().ok()?),
                _ => (feature_seg, 0),
            };
            let feature = mm.lookup_feature(self.class_of(cur), name)?;
            match self.get(cur, feature).get(index)? {
                ModelValue::Object(o) => Some(*o),
                _ => None,
            }
        } else {
            // Name-based segment, as used by Ecore models themselves.
            self.contents(mm, cur).into_iter().find(|child| self.name_of(mm, *child).as_deref() == Some(seg))
        }
    }

    /// The value of a `name` attribute, if the object has one.
    pub fn name_of(&self, mm: &Metamodel, id: ObjectId) -> Option<String> {
        let f = mm.lookup_feature(self.class_of(id), "name")?;
        match self.get(id, f).first()? {
            ModelValue::String(s) => Some(s.clone()),
            _ => None,
        }
    }

    /// A human readable label such as `Book 'Dune'` (by name), `Book #dune` (by `xmi:id`) or
    /// `Book //@books.0`.
    pub fn label(&self, mm: &Metamodel, id: ObjectId) -> String {
        let class = &mm.classifier(self.class_of(id)).name;
        match (self.name_of(mm, id), &self.objects[id.index()].xmi_id) {
            (Some(name), _) => format!("{class} '{name}'"),
            (None, Some(xmi_id)) => format!("{class} #{xmi_id}"),
            (None, None) => format!("{class} {}", self.uri_fragment(mm, id)),
        }
    }
}

/// The default value of a single-valued feature when it is not set.
pub fn default_value(mm: &Metamodel, feature: FeatureId) -> Option<ModelValue> {
    let f = mm.feature(feature);
    let ty = f.ty?;
    if let Some(lit) = &f.default_value_literal {
        return parse_value(mm, ty, lit).ok();
    }
    if mm.is_enum(ty) {
        return (!mm.enum_literals(ty).is_empty()).then_some(ModelValue::Enum(ty, 0));
    }
    // Java primitive types have a non-null default; object types default to null.
    match mm.classifier(ty).instance_class_name.as_deref()? {
        "boolean" => Some(ModelValue::Boolean(false)),
        "int" | "long" | "short" | "byte" => Some(ModelValue::Integer(0)),
        "double" | "float" => Some(ModelValue::Real(0.0)),
        _ => None,
    }
}

/// Parses the serialized form of a data type or enum value.
pub fn parse_value(mm: &Metamodel, ty: ClassifierId, text: &str) -> Result<ModelValue, String> {
    if mm.is_enum(ty) {
        let lits = mm.enum_literals(ty);
        return lits
            .iter()
            .position(|l| l.literal == text || l.name == text)
            .map(|i| ModelValue::Enum(ty, i))
            .ok_or_else(|| format!("'{text}' is not a literal of enumeration {}", mm.classifier(ty).name));
    }
    match mm.data_type_kind(ty).unwrap_or(DataTypeKind::Other) {
        DataTypeKind::Boolean => match text.trim() {
            "true" => Ok(ModelValue::Boolean(true)),
            "false" => Ok(ModelValue::Boolean(false)),
            other => Err(format!("'{other}' is not a boolean")),
        },
        DataTypeKind::Integer => {
            text.trim().parse().map(ModelValue::Integer).map_err(|_| format!("'{text}' is not an integer"))
        }
        DataTypeKind::Real => {
            text.trim().parse().map(ModelValue::Real).map_err(|_| format!("'{text}' is not a number"))
        }
        DataTypeKind::String | DataTypeKind::Other => Ok(ModelValue::String(text.to_string())),
    }
}
