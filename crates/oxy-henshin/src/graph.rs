//! The graphs that rules transform: the objects of a model as nodes, their references as links.
//!
//! Like an EMF resource (Henshin's `EGraph`), a [`Graph`] keeps both sides of a bidirectional
//! reference and the containment of every node consistent: adding a link adds its opposite,
//! replaces the old value of a single-valued feature and moves a contained node out of its old
//! container. Unlike [`Model`], nodes can be deleted; their ids are never reused, so the nodes of
//! two states of a transformation can be compared by id.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use oxy_ecore::{ClassifierId, FeatureId, Metamodel, Model, ModelValue, ObjectId};

/// A node of a [`Graph`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

impl NodeId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub class: ClassifierId,
    /// The identifier of the object the node was created from (its `xmi:id` or HUTN
    /// identifier), if any.
    pub identifier: Option<String>,
    /// The containing node and the containment feature holding this node.
    pub container: Option<(NodeId, FeatureId)>,
    attributes: Vec<(FeatureId, Vec<ModelValue>)>,
    references: Vec<(FeatureId, Vec<NodeId>)>,
}

impl Node {
    /// Attributes that have a value.
    pub fn attributes(&self) -> impl Iterator<Item = (FeatureId, &[ModelValue])> {
        self.attributes.iter().filter(|(_, v)| !v.is_empty()).map(|(f, v)| (*f, v.as_slice()))
    }

    /// References that have targets, in the order they were set.
    pub fn references(&self) -> impl Iterator<Item = (FeatureId, &[NodeId])> {
        self.references.iter().filter(|(_, v)| !v.is_empty()).map(|(f, v)| (*f, v.as_slice()))
    }
}

/// A link of a graph: source, reference, target.
pub type Link = (NodeId, FeatureId, NodeId);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Graph {
    nodes: Vec<Option<Node>>,
}

impl Graph {
    pub fn new() -> Self {
        Graph::default()
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.index()).and_then(Option::as_ref)
    }

    fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id.index()).and_then(Option::as_mut)
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.node(id).is_some()
    }

    /// The live nodes, in creation order.
    pub fn node_ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes.iter().enumerate().filter(|(_, n)| n.is_some()).map(|(i, _)| NodeId(i as u32))
    }

    pub fn len(&self) -> usize {
        self.nodes.iter().filter(|n| n.is_some()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The id the next created node gets.
    pub fn next_id(&self) -> NodeId {
        NodeId(self.nodes.len() as u32)
    }

    pub fn class_of(&self, id: NodeId) -> Option<ClassifierId> {
        self.node(id).map(|n| n.class)
    }

    /// Nodes that are not contained in another node, in creation order.
    pub fn roots(&self) -> Vec<NodeId> {
        self.node_ids().filter(|n| self.node(*n).is_some_and(|n| n.container.is_none())).collect()
    }

    pub fn add_node(&mut self, class: ClassifierId) -> NodeId {
        let id = self.next_id();
        self.nodes.push(Some(Node {
            class,
            identifier: None,
            container: None,
            attributes: Vec::new(),
            references: Vec::new(),
        }));
        id
    }

    /// The targets of a reference of a node (empty if unset or if the node does not exist).
    pub fn targets(&self, source: NodeId, feature: FeatureId) -> &[NodeId] {
        self.node(source)
            .and_then(|n| n.references.iter().find(|(f, _)| *f == feature))
            .map_or(&[], |(_, t)| t.as_slice())
    }

    pub fn has_link(&self, source: NodeId, feature: FeatureId, target: NodeId) -> bool {
        self.targets(source, feature).contains(&target)
    }

    /// All links of the graph (both sides of bidirectional references).
    pub fn links(&self) -> Vec<Link> {
        let mut out = Vec::new();
        for s in self.node_ids() {
            for (f, targets) in self.node(s).map(|n| n.references()).into_iter().flatten() {
                out.extend(targets.iter().map(|t| (s, f, *t)));
            }
        }
        out
    }

    /// The links ending at a node: `(source, feature)`.
    pub fn incoming(&self, target: NodeId) -> Vec<(NodeId, FeatureId)> {
        let mut out = Vec::new();
        for s in self.node_ids() {
            for (f, targets) in self.node(s).map(|n| n.references()).into_iter().flatten() {
                out.extend(targets.iter().filter(|t| **t == target).map(|_| (s, f)));
            }
        }
        out
    }

    fn push_raw(&mut self, source: NodeId, feature: FeatureId, target: NodeId) {
        let Some(node) = self.node_mut(source) else { return };
        match node.references.iter_mut().find(|(f, _)| *f == feature) {
            Some((_, targets)) => targets.push(target),
            None => node.references.push((feature, vec![target])),
        }
    }

    fn remove_raw(&mut self, source: NodeId, feature: FeatureId, target: NodeId) -> bool {
        let Some(node) = self.node_mut(source) else { return false };
        let Some((_, targets)) = node.references.iter_mut().find(|(f, _)| *f == feature) else { return false };
        let before = targets.len();
        targets.retain(|t| *t != target);
        targets.len() != before
    }

    /// Whether `ancestor` contains `node` (transitively), or is `node`.
    pub fn contains_transitively(&self, ancestor: NodeId, node: NodeId) -> bool {
        let mut cur = Some(node);
        let mut steps = 0;
        while let Some(c) = cur {
            if c == ancestor {
                return true;
            }
            steps += 1;
            if steps > self.nodes.len() {
                return false;
            }
            cur = self.node(c).and_then(|n| n.container.map(|(p, _)| p));
        }
        false
    }

    /// Adds a link, keeping opposites and containment consistent (see the module
    /// documentation). Returns `false` if the link cannot be added: a missing node, or a
    /// containment cycle.
    pub fn add_link(&mut self, mm: &Metamodel, source: NodeId, feature: FeatureId, target: NodeId) -> bool {
        if !self.contains(source) || !self.contains(target) {
            return false;
        }
        let f = mm.feature(feature);
        // Container references are set through their containment side.
        if let Some(op) = f.opposite()
            && mm.feature(op).is_containment()
        {
            return self.add_link(mm, target, op, source);
        }
        if self.has_link(source, feature, target) {
            return true;
        }
        if f.is_containment() {
            if self.contains_transitively(target, source) {
                return false;
            }
            if let Some((p, cf)) = self.node(target).and_then(|n| n.container) {
                self.remove_link(mm, p, cf, target);
            }
        }
        if !f.is_many() {
            for old in self.targets(source, feature).to_vec() {
                self.remove_link(mm, source, feature, old);
            }
        }
        self.push_raw(source, feature, target);
        if f.is_containment()
            && let Some(t) = self.node_mut(target)
        {
            t.container = Some((source, feature));
        }
        if let Some(op) = f.opposite() {
            if !mm.feature(op).is_many() {
                for old in self.targets(target, op).to_vec() {
                    if old != source {
                        self.remove_link(mm, target, op, old);
                    }
                }
            }
            if !self.has_link(target, op, source) {
                self.push_raw(target, op, source);
            }
        }
        true
    }

    /// Removes a link and its opposite.
    pub fn remove_link(&mut self, mm: &Metamodel, source: NodeId, feature: FeatureId, target: NodeId) {
        self.remove_raw(source, feature, target);
        if let Some(t) = self.node_mut(target)
            && t.container == Some((source, feature))
        {
            t.container = None;
        }
        if let Some(op) = mm.feature(feature).opposite() {
            self.remove_raw(target, op, source);
            if let Some(s) = self.node_mut(source)
                && s.container == Some((target, op))
            {
                s.container = None;
            }
        }
    }

    /// Deletes a node and every link from or to it. Nodes it contained become roots.
    pub fn remove_node(&mut self, mm: &Metamodel, id: NodeId) {
        let Some(node) = self.node(id) else { return };
        let outgoing: Vec<(FeatureId, Vec<NodeId>)> = node.references.iter().map(|(f, t)| (*f, t.clone())).collect();
        for (f, targets) in outgoing {
            for t in targets {
                self.remove_link(mm, id, f, t);
            }
        }
        for (s, f) in self.incoming(id) {
            self.remove_link(mm, s, f, id);
        }
        self.nodes[id.index()] = None;
    }

    /// The values of an attribute (empty if unset).
    pub fn attribute(&self, id: NodeId, feature: FeatureId) -> &[ModelValue] {
        self.node(id).and_then(|n| n.attributes.iter().find(|(f, _)| *f == feature)).map_or(&[], |(_, v)| v.as_slice())
    }

    /// Replaces the values of an attribute (empty: unsets it).
    pub fn set_attribute(&mut self, id: NodeId, feature: FeatureId, values: Vec<ModelValue>) {
        let Some(node) = self.node_mut(id) else { return };
        match node.attributes.iter_mut().find(|(f, _)| *f == feature) {
            Some((_, v)) => *v = values,
            None if values.is_empty() => {}
            None => node.attributes.push((feature, values)),
        }
    }

    pub fn set_identifier(&mut self, id: NodeId, identifier: Option<String>) {
        if let Some(n) = self.node_mut(id) {
            n.identifier = identifier;
        }
    }

    // ------------------------------------------------------------------ labels

    /// The value of a `name` attribute, if the node has one.
    pub fn name_of(&self, mm: &Metamodel, id: NodeId) -> Option<String> {
        let f = mm.lookup_feature(self.class_of(id)?, "name")?;
        match self.attribute(id, f).first()? {
            ModelValue::String(s) => Some(s.clone()),
            _ => None,
        }
    }

    /// A label such as `Book 'Dune'` (by name), `Book "dune"` (by identifier) or `Book #3`.
    pub fn label(&self, mm: &Metamodel, id: NodeId) -> String {
        let Some(node) = self.node(id) else { return format!("#{}", id.0) };
        let class = &mm.classifier(node.class).name;
        match (self.name_of(mm, id), &node.identifier) {
            (Some(name), _) => format!("{class} '{name}'"),
            (None, Some(identifier)) => format!("{class} \"{identifier}\""),
            (None, None) => format!("{class} #{}", id.0),
        }
    }

    // ------------------------------------------------------------------ models

    /// The graph of a model resource: its objects (with their contents) and the links between
    /// them. References to objects of other resources are left out.
    pub fn from_model(mm: &Metamodel, model: &Model, resource: usize) -> (Graph, Vec<(ObjectId, NodeId)>) {
        let mut graph = Graph::new();
        let mut map: HashMap<ObjectId, NodeId> = HashMap::new();
        let mut order = Vec::new();
        for &root in model.resources().get(resource).map(|r| r.roots.as_slice()).unwrap_or(&[]) {
            order.push(root);
            order.extend(model.all_contents(mm, root));
        }
        for &o in &order {
            let id = graph.add_node(model.class_of(o));
            graph.set_identifier(id, model.object(o).xmi_id.clone());
            map.insert(o, id);
        }
        for &o in &order {
            let id = map[&o];
            let object = model.object(o);
            if let Some((parent, f)) = object.container
                && let Some(p) = map.get(&parent)
                && let Some(n) = graph.node_mut(id)
            {
                n.container = Some((*p, f));
            }
            for (f, values) in object.set_features() {
                if mm.feature(f).is_reference() {
                    let targets: Vec<NodeId> = values
                        .iter()
                        .filter_map(|v| match v {
                            ModelValue::Object(t) => map.get(t).copied(),
                            _ => None,
                        })
                        .collect();
                    if !targets.is_empty()
                        && let Some(n) = graph.node_mut(id)
                    {
                        n.references.push((f, targets));
                    }
                } else if !values.is_empty() {
                    graph.set_attribute(id, f, values.to_vec());
                }
            }
        }
        let pairs = order.iter().map(|o| (*o, map[o])).collect();
        (graph, pairs)
    }

    /// The graph as a model with one resource (named `uri`). Returns the model and the object
    /// of each node.
    pub fn to_model(&self, mm: &Metamodel, uri: &str) -> (Model, HashMap<NodeId, ObjectId>) {
        let mut model = Model::new();
        let resource = model.add_resource(uri);
        let mut map: HashMap<NodeId, ObjectId> = HashMap::new();
        // Containers before their contents, in containment order.
        let mut stack: Vec<NodeId> = self.roots();
        stack.reverse();
        while let Some(n) = stack.pop() {
            let Some(node) = self.node(n) else { continue };
            let container = node.container.and_then(|(p, f)| Some((*map.get(&p)?, f)));
            let o = model.create_object(mm, node.class, resource, container);
            if let Some(identifier) = &node.identifier {
                model.set_xmi_id(o, identifier.clone());
            }
            map.insert(n, o);
            let mut children: Vec<NodeId> = Vec::new();
            for (f, targets) in node.references() {
                if mm.feature(f).is_containment() {
                    children
                        .extend(targets.iter().filter(|t| self.node(**t).is_some_and(|c| c.container == Some((n, f)))));
                }
            }
            children.reverse();
            stack.extend(children);
        }
        for (n, &o) in &map {
            let Some(node) = self.node(*n) else { continue };
            for (f, values) in node.attributes() {
                model.set_raw(o, f, values.to_vec());
            }
            for (f, targets) in node.references() {
                let values = targets.iter().filter_map(|t| map.get(t)).map(|t| ModelValue::Object(*t)).collect();
                model.set_raw(o, f, values);
            }
        }
        (model, map)
    }

    // ------------------------------------------------------------------ metamodel changes

    /// The same graph for another metamodel: classes and features are mapped by their
    /// qualified names. Nodes whose class no longer exists are dropped (with their links),
    /// values of features that no longer exist too.
    pub fn migrate(&self, from: &Metamodel, to: &Metamodel) -> Graph {
        let mut classes: HashMap<ClassifierId, Option<ClassifierId>> = HashMap::new();
        let mut features: HashMap<FeatureId, Option<FeatureId>> = HashMap::new();
        let mut class_of = |c: ClassifierId| {
            *classes.entry(c).or_insert_with(|| {
                let path = from.qualified_name(c);
                let path: Vec<&str> = path.split("::").collect();
                to.find_classifiers_qualified(&path).into_iter().find(|c| to.is_class(*c))
            })
        };
        let mut graph = Graph { nodes: Vec::with_capacity(self.nodes.len()) };
        for node in &self.nodes {
            graph.nodes.push(node.as_ref().and_then(|n| {
                Some(Node {
                    class: class_of(n.class)?,
                    identifier: n.identifier.clone(),
                    container: None,
                    attributes: Vec::new(),
                    references: Vec::new(),
                })
            }));
        }
        let mut feature_of = |f: FeatureId, class: ClassifierId| {
            *features.entry(f).or_insert_with(|| {
                let feature = from.feature(f);
                let mapped = to.lookup_feature(class, &feature.name)?;
                (to.feature(mapped).is_reference() == feature.is_reference()).then_some(mapped)
            })
        };
        for (i, node) in self.nodes.iter().enumerate() {
            let (Some(old), Some(class)) = (node, graph.nodes[i].as_ref().map(|n| n.class)) else { continue };
            let id = NodeId(i as u32);
            for (f, values) in &old.attributes {
                if let Some(mapped) = feature_of(*f, class) {
                    graph.set_attribute(id, mapped, values.clone());
                }
            }
            for (f, targets) in &old.references {
                let Some(mapped) = feature_of(*f, class) else { continue };
                let targets: Vec<NodeId> = targets.iter().copied().filter(|t| graph.contains(*t)).collect();
                if !targets.is_empty()
                    && let Some(n) = graph.node_mut(id)
                {
                    n.references.push((mapped, targets));
                }
            }
            if let Some((p, f)) = old.container
                && graph.contains(p)
                && let Some(pc) = graph.class_of(p)
                && let Some(mapped) = feature_of(f, pc)
                && let Some(n) = graph.node_mut(id)
            {
                n.container = Some((p, mapped));
            }
        }
        graph
    }

    // ------------------------------------------------------------------ isomorphism

    /// A hash that isomorphic graphs share, and the colour of each node it was computed from
    /// (Weisfeiler-Lehman refinement over classes, attribute values and links).
    pub fn fingerprint(&self) -> Fingerprint {
        let ids: Vec<NodeId> = self.node_ids().collect();
        let mut incoming: HashMap<NodeId, Vec<(FeatureId, NodeId)>> = HashMap::new();
        for (s, f, t) in self.links() {
            incoming.entry(t).or_default().push((f, s));
        }
        let mut colors: HashMap<NodeId, u64> = HashMap::new();
        for &n in &ids {
            let node = self.node(n).expect("live node");
            let mut attributes: Vec<u64> = node.attributes().map(|(f, v)| hash_of(&(f.0, values_hash(v)))).collect();
            attributes.sort_unstable();
            colors.insert(n, hash_of(&(node.class.0, attributes)));
        }
        let mut classes = distinct(&colors);
        for _ in 0..ids.len().min(8) {
            let mut next = HashMap::with_capacity(colors.len());
            for &n in &ids {
                let node = self.node(n).expect("live node");
                let mut out: Vec<(u32, u64)> = node
                    .references()
                    .flat_map(|(f, t)| t.iter().map(move |t| (f.0, *t)))
                    .map(|(f, t)| (f, colors[&t]))
                    .collect();
                out.sort_unstable();
                let mut inc: Vec<(u32, u64)> =
                    incoming.get(&n).into_iter().flatten().map(|(f, s)| (f.0, colors[s])).collect();
                inc.sort_unstable();
                next.insert(n, hash_of(&(colors[&n], out, inc)));
            }
            colors = next;
            let refined = distinct(&colors);
            if refined == classes {
                break;
            }
            classes = refined;
        }
        let mut all: Vec<u64> = colors.values().copied().collect();
        all.sort_unstable();
        Fingerprint { hash: hash_of(&all), colors }
    }

    /// Whether two graphs are isomorphic: the same structure, classes and attribute values, up
    /// to node identities (and the order of multi-valued features).
    pub fn isomorphic(&self, fp: &Fingerprint, other: &Graph, other_fp: &Fingerprint) -> bool {
        if fp.hash != other_fp.hash || self.len() != other.len() {
            return false;
        }
        let mut by_color: HashMap<u64, Vec<NodeId>> = HashMap::new();
        for (n, c) in &other_fp.colors {
            by_color.entry(*c).or_default().push(*n);
        }
        for v in by_color.values_mut() {
            v.sort_unstable();
        }
        // Rarest colours first, then neighbours of mapped nodes.
        let mut order: Vec<NodeId> = self.node_ids().collect();
        order.sort_by_key(|n| (by_color.get(&fp.colors[n]).map_or(0, Vec::len), *n));
        let mut mapping: HashMap<NodeId, NodeId> = HashMap::new();
        let mut used: HashSet<NodeId> = HashSet::new();
        self.extend_isomorphism(other, fp, &by_color, &order, 0, &mut mapping, &mut used)
    }

    #[allow(clippy::too_many_arguments)]
    fn extend_isomorphism(
        &self,
        other: &Graph,
        fp: &Fingerprint,
        by_color: &HashMap<u64, Vec<NodeId>>,
        order: &[NodeId],
        i: usize,
        mapping: &mut HashMap<NodeId, NodeId>,
        used: &mut HashSet<NodeId>,
    ) -> bool {
        let Some(&a) = order.get(i) else { return true };
        let Some(candidates) = by_color.get(&fp.colors[&a]) else { return false };
        for &b in candidates {
            if used.contains(&b) || !self.compatible(other, a, b, mapping) {
                continue;
            }
            mapping.insert(a, b);
            used.insert(b);
            if self.extend_isomorphism(other, fp, by_color, order, i + 1, mapping, used) {
                return true;
            }
            mapping.remove(&a);
            used.remove(&b);
        }
        false
    }

    /// Whether `a` (of `self`) can be mapped to `b` (of `other`) given the mapping so far.
    fn compatible(&self, other: &Graph, a: NodeId, b: NodeId, mapping: &HashMap<NodeId, NodeId>) -> bool {
        let (Some(na), Some(nb)) = (self.node(a), other.node(b)) else { return false };
        if na.class != nb.class {
            return false;
        }
        let mut aa: Vec<(u32, u64)> = na.attributes().map(|(f, v)| (f.0, values_hash(v))).collect();
        let mut ab: Vec<(u32, u64)> = nb.attributes().map(|(f, v)| (f.0, values_hash(v))).collect();
        aa.sort_unstable();
        ab.sort_unstable();
        if aa != ab {
            return false;
        }
        // Links between `a` and mapped nodes (and itself) must correspond.
        let count = |g: &Graph, s: NodeId, f: FeatureId, t: NodeId| g.targets(s, f).iter().filter(|x| **x == t).count();
        for (f, targets) in na.references() {
            for &t in targets {
                let mapped = if t == a { Some(b) } else { mapping.get(&t).copied() };
                if let Some(mt) = mapped
                    && count(self, a, f, t) != count(other, b, f, mt)
                {
                    return false;
                }
            }
        }
        for (f, targets) in nb.references() {
            for &t in targets {
                let back = if t == b { Some(a) } else { mapping.iter().find(|(_, v)| **v == t).map(|(k, _)| *k) };
                if let Some(bt) = back
                    && count(other, b, f, t) != count(self, a, f, bt)
                {
                    return false;
                }
            }
        }
        // Links from mapped nodes to `a`.
        for (&m, &mb) in mapping {
            let (Some(nm), Some(nmb)) = (self.node(m), other.node(mb)) else { continue };
            let into_a: Vec<(FeatureId, usize)> = nm
                .references()
                .map(|(f, t)| (f, t.iter().filter(|x| **x == a).count()))
                .filter(|(_, c)| *c > 0)
                .collect();
            let into_b: Vec<(FeatureId, usize)> = nmb
                .references()
                .map(|(f, t)| (f, t.iter().filter(|x| **x == b).count()))
                .filter(|(_, c)| *c > 0)
                .collect();
            if into_a.len() != into_b.len() || into_a.iter().any(|x| !into_b.contains(x)) {
                return false;
            }
        }
        true
    }
}

/// See [`Graph::fingerprint`].
#[derive(Clone, Debug, PartialEq)]
pub struct Fingerprint {
    pub hash: u64,
    pub colors: HashMap<NodeId, u64>,
}

fn hash_of<T: Hash>(value: &T) -> u64 {
    let mut h = DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
}

fn values_hash(values: &[ModelValue]) -> u64 {
    let mut h = DefaultHasher::new();
    for v in values {
        match v {
            ModelValue::Boolean(b) => (0u8, *b).hash(&mut h),
            ModelValue::Integer(i) => (1u8, *i).hash(&mut h),
            ModelValue::Real(r) => (2u8, r.to_bits()).hash(&mut h),
            ModelValue::String(s) => (3u8, s).hash(&mut h),
            ModelValue::Enum(e, l) => (4u8, e.0, *l).hash(&mut h),
            ModelValue::Object(o) => (5u8, o.0).hash(&mut h),
        }
    }
    h.finish()
}

fn distinct(colors: &HashMap<NodeId, u64>) -> usize {
    colors.values().collect::<HashSet<_>>().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ECORE: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<ecore:EPackage xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI"
    xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    xmlns:ecore="http://www.eclipse.org/emf/2002/Ecore" name="t" nsURI="http://t" nsPrefix="t">
  <eClassifiers xsi:type="ecore:EClass" name="Box">
    <eStructuralFeatures xsi:type="ecore:EReference" name="items" upperBound="-1" eType="#//Item" containment="true" eOpposite="#//Item/box"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="best" eType="#//Item"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="Item">
    <eStructuralFeatures xsi:type="ecore:EReference" name="box" eType="#//Box" eOpposite="#//Box/items"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="n"
        eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EInt"/>
  </eClassifiers>
</ecore:EPackage>"##;

    fn setup() -> (Metamodel, ClassifierId, ClassifierId, FeatureId, FeatureId, FeatureId) {
        let mut mm = Metamodel::new();
        mm.load_ecore_str(ECORE, "t.ecore").unwrap();
        let class = |n: &str| mm.find_classifiers(n)[0];
        let (b, i) = (class("Box"), class("Item"));
        let items = mm.lookup_feature(b, "items").unwrap();
        let box_ = mm.lookup_feature(i, "box").unwrap();
        let best = mm.lookup_feature(b, "best").unwrap();
        (mm, b, i, items, box_, best)
    }

    #[test]
    fn keeps_opposites_and_containment_consistent() {
        let (mm, b, i, items, box_, best) = setup();
        let mut g = Graph::new();
        let (b1, b2, x) = (g.add_node(b), g.add_node(b), g.add_node(i));
        assert!(g.add_link(&mm, b1, items, x));
        assert_eq!(g.targets(x, box_), [b1]);
        assert_eq!(g.node(x).unwrap().container, Some((b1, items)));
        // Setting the container side moves the item.
        assert!(g.add_link(&mm, x, box_, b2));
        assert!(g.targets(b1, items).is_empty());
        assert_eq!(g.targets(b2, items), [x]);
        assert_eq!(g.roots(), [b1, b2]);
        // Single-valued references are replaced.
        let y = g.add_node(i);
        g.add_link(&mm, b1, best, x);
        g.add_link(&mm, b1, best, y);
        assert_eq!(g.targets(b1, best), [y]);
        // Deleting a node removes the links to it; its contents become roots.
        g.remove_node(&mm, b2);
        assert!(g.targets(x, box_).is_empty());
        assert_eq!(g.node(x).unwrap().container, None);
        g.remove_node(&mm, y);
        assert!(g.targets(b1, best).is_empty());
        assert_eq!(g.len(), 2);
    }

    #[test]
    fn recognizes_isomorphic_graphs() {
        let (mm, b, i, items, _, _) = setup();
        let n = mm.lookup_feature(i, "n").unwrap();
        let build = |values: &[i64]| {
            let mut g = Graph::new();
            let bx = g.add_node(b);
            for v in values {
                let x = g.add_node(i);
                g.set_attribute(x, n, vec![ModelValue::Integer(*v)]);
                g.add_link(&mm, bx, items, x);
            }
            g
        };
        let (a, c, d) = (build(&[1, 2]), build(&[2, 1]), build(&[1, 1]));
        let (fa, fc, fd) = (a.fingerprint(), c.fingerprint(), d.fingerprint());
        assert!(a.isomorphic(&fa, &c, &fc));
        assert!(!a.isomorphic(&fa, &d, &fd));
        // Ids do not matter.
        let mut e = Graph::new();
        let x = e.add_node(i);
        let bx = e.add_node(b);
        let y = e.add_node(i);
        e.set_attribute(y, n, vec![ModelValue::Integer(1)]);
        e.set_attribute(x, n, vec![ModelValue::Integer(2)]);
        e.add_link(&mm, bx, items, y);
        e.add_link(&mm, bx, items, x);
        let fe = e.fingerprint();
        assert!(a.isomorphic(&fa, &e, &fe));
    }
}
