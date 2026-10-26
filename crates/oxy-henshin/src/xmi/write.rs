//! Writing compiled modules as `.henshin` files (see the [module documentation](super)).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use oxy_ecore::xml::{escape_attr, escape_text};
use oxy_ecore::{ClassifierId, ClassifierKind, FeatureId, Metamodel, PackageId};

use super::{Extra, Keys, NESTED, Template, derived_id, unit_key};
use crate::js;
use crate::module::{Callable, Condition, ConditionGraph, Module, NodeKind, Param, ParamType, Rule, RuleNode, Step};
use crate::syntax::ParamKind;
use crate::value::CExpr;

/// Options of [`write`].
#[derive(Clone, Copy, Debug, Default)]
pub struct WriteOptions<'a> {
    /// Where the file will be: references to `.ecore` files are relative to it.
    pub path: &'a str,
    /// The name of the module (default: the template's, else the file name without its
    /// extension).
    pub name: Option<&'a str>,
    /// The file being replaced, whose ids and extra content are kept.
    pub template: Option<&'a Template>,
}

/// Writes a module (which must have no errors) as a `.henshin` file.
pub fn write(mm: &Metamodel, module: &Module, options: &WriteOptions) -> String {
    let mut w = Writer {
        mm,
        module,
        options,
        keys: Keys::default(),
        used: HashSet::new(),
        ids: HashMap::new(),
        taken: module.rules.iter().map(|r| r.name.clone()).chain(module.units.iter().map(|u| u.name.clone())).collect(),
        units: Vec::new(),
        renames: Renames::new(module, options.template),
    };
    w.module()
}

// ---------------------------------------------------------------------- XML

/// An element being written.
#[derive(Default)]
struct X {
    tag: String,
    attributes: Vec<(String, String)>,
    children: Vec<X>,
    /// Child elements kept from the template, as they were written.
    raw: Vec<String>,
    text: Option<String>,
}

impl X {
    fn new(tag: &str) -> X {
        X { tag: tag.into(), ..X::default() }
    }

    fn attr(&mut self, name: &str, value: impl Into<String>) {
        self.attributes.push((name.into(), value.into()));
    }

    fn with(mut self, name: &str, value: impl Into<String>) -> X {
        self.attr(name, value);
        self
    }

    fn push(&mut self, child: X) {
        self.children.push(child);
    }

    fn write(&self, out: &mut String, depth: usize) {
        let indent = "  ".repeat(depth);
        out.push_str(&indent);
        out.push('<');
        out.push_str(&self.tag);
        for (k, v) in &self.attributes {
            out.push_str(&format!(" {k}=\"{}\"", escape_attr(v)));
        }
        if let Some(text) = &self.text {
            out.push_str(&format!(">{}</{}>\n", escape_text(text), self.tag));
        } else if self.children.is_empty() && self.raw.is_empty() {
            out.push_str("/>\n");
        } else {
            out.push_str(">\n");
            for r in &self.raw {
                out.push_str(&format!("{indent}  {r}\n"));
            }
            for c in &self.children {
                c.write(out, depth + 1);
            }
            out.push_str(&format!("{indent}</{}>\n", self.tag));
        }
    }
}

/// A node of a graph being written, before its edges are known.
struct NodeX {
    x: X,
    id: String,
}

/// The nodes a condition can refer to: match slots with their element ids, names and classes.
#[derive(Clone, Default)]
struct Host {
    key: String,
    slots: HashMap<usize, (String, String, ClassifierId)>,
}

/// The ids of a rule's nodes, by node index.
#[derive(Default)]
struct RuleIds {
    lhs: Vec<Option<String>>,
    rhs: Vec<Option<String>>,
}

/// A unit being written: the parameter mappings it collects.
struct UnitCx<'p> {
    name: String,
    key: String,
    params: &'p [Param],
    mappings: Vec<X>,
    /// The arguments rules and units are called with, to tell calls apart that Henshin's
    /// per-unit parameter mappings cannot.
    calls: HashMap<Callable, Vec<usize>>,
}

struct Writer<'a> {
    mm: &'a Metamodel,
    module: &'a Module,
    options: &'a WriteOptions<'a>,
    keys: Keys,
    /// Ids handed out.
    used: HashSet<String>,
    /// Ids by key, for elements referred to before they are written (units, parameters).
    ids: HashMap<String, String>,
    /// Names of units and rules, for naming the units of nested blocks.
    taken: HashSet<String>,
    /// The units of the module, in order.
    units: Vec<Option<X>>,
    /// Rules, units and nodes renamed since the template.
    renames: Renames,
}

/// Rules, units and nodes renamed since the template was written: names that are new where the
/// template has one that is gone, at the same position.
#[derive(Default)]
struct Renames {
    /// Old unit names by new ones.
    units: HashMap<String, String>,
    /// Old node names by new ones, by (new) rule name.
    nodes: HashMap<String, HashMap<String, String>>,
}

/// Pairs of names at the same position, of which the new one is not in `old` and the old one
/// not in `new`.
fn positional(new: &[String], old: &[String]) -> HashMap<String, String> {
    new.iter()
        .zip(old)
        .filter(|(n, o)| n != o && !old.contains(n) && !new.contains(o))
        .map(|(n, o)| (n.clone(), o.clone()))
        .collect()
}

impl Renames {
    fn new(module: &Module, template: Option<&Template>) -> Renames {
        let Some(t) = template else { return Renames::default() };
        let mut order: Vec<(usize, String)> = module
            .rules
            .iter()
            .map(|r| (r.span.start, r.name.clone()))
            .chain(module.units.iter().map(|u| (u.span.start, u.name.clone())))
            .collect();
        order.sort();
        let names: Vec<String> = order.into_iter().map(|(_, n)| n).collect();
        let units = positional(&names, &t.units);
        let mut nodes = HashMap::new();
        for r in &module.rules {
            let old = units.get(&r.name).unwrap_or(&r.name);
            let Some(before) = t.nodes.get(old) else { continue };
            let lhs = r.nodes.iter().filter(|n| n.kind.is_lhs());
            let now: Vec<String> =
                lhs.chain(r.nodes.iter().filter(|n| !n.kind.is_lhs())).map(|n| n.name.clone()).collect();
            nodes.insert(r.name.clone(), positional(&now, before));
        }
        Renames { units, nodes }
    }

    fn unit<'n>(&'n self, name: &'n str) -> String {
        if let Some(old) = self.units.get(name) {
            return old.clone();
        }
        // The units made for the nested blocks of a renamed unit.
        for (new, old) in &self.units {
            if let Some(rest) = name.strip_prefix(&format!("{new}_")) {
                return format!("{old}_{rest}");
            }
        }
        name.to_string()
    }

    /// The template's key for a key.
    fn key(&self, key: &str) -> String {
        if self.units.is_empty() && self.nodes.values().all(HashMap::is_empty) {
            return key.to_string();
        }
        let mut rule_nodes = None;
        let segments: Vec<String> = key
            .split('/')
            .map(|segment| {
                let node = |n: &str| -> String {
                    rule_nodes
                        .and_then(|m: &HashMap<String, String>| m.get(n))
                        .cloned()
                        .unwrap_or_else(|| n.to_string())
                };
                if let Some(u) = segment.strip_prefix("unit:") {
                    let (name, suffix) = u.split_once('#').map_or((u, String::new()), |(n, s)| (n, format!("#{s}")));
                    rule_nodes = self.nodes.get(name);
                    format!("unit:{}{suffix}", self.unit(name))
                } else if let Some(n) = segment.strip_prefix("node:") {
                    format!("node:{}", node(n))
                } else if let Some(e) = segment.strip_prefix("edge:")
                    && let Some((s, rest)) = e.split_once("->")
                    && let Some((t, f)) = rest.split_once(':')
                {
                    format!("edge:{}->{}:{f}", node(s), node(t))
                } else if let Some(m) = segment.strip_prefix("mapping:") {
                    match m.split_once("->") {
                        // A parameter mapping: `unit.param->unit.param`.
                        Some((a, b)) => {
                            let unit = |x: &str| match x.split_once('.') {
                                Some((u, p)) => format!("{}.{p}", self.unit(u)),
                                None => x.to_string(),
                            };
                            format!("mapping:{}->{}", unit(a), unit(b))
                        }
                        None => format!("mapping:{}", node(m)),
                    }
                } else if let Some(m) = segment.strip_prefix("multiMapping:")
                    && let Some((side, n)) = m.split_once(':')
                {
                    format!("multiMapping:{side}:{}", node(n))
                } else {
                    segment.to_string()
                }
            })
            .collect();
        segments.join("/")
    }
}

impl Writer<'_> {
    fn template(&self) -> Option<&Template> {
        self.options.template
    }

    fn extra(&self, key: &str) -> Option<&Extra> {
        self.template().and_then(|t| t.extras.get(&self.renames.key(key)))
    }

    /// The id of the element with this key: the template's, else derived from the key.
    fn id(&mut self, key: &str) -> String {
        if let Some(id) = self.ids.get(key) {
            return id.clone();
        }
        let mut id =
            self.template().and_then(|t| t.ids.get(&self.renames.key(key)).cloned()).unwrap_or_else(|| derived_id(key));
        let mut n = 1;
        while self.used.contains(&id) {
            n += 1;
            id = derived_id(&format!("{key}#{n}"));
        }
        self.used.insert(id.clone());
        self.ids.insert(key.to_string(), id.clone());
        id
    }

    /// A new element with its `xsi:type` and id.
    fn element(&mut self, tag: &str, xsi_type: Option<&str>, key: &str) -> X {
        let mut x = X::new(tag);
        if let Some(t) = xsi_type {
            x.attr("xsi:type", t);
        }
        let id = self.id(key);
        x.attr("xmi:id", id);
        x
    }

    /// Adds the template's extra content of the element: attributes after the name, child
    /// elements first.
    fn finish(&self, x: &mut X, key: &str) {
        let Some(extra) = self.extra(key) else { return };
        let at = x
            .attributes
            .iter()
            .position(|(k, _)| k == "name")
            .or_else(|| x.attributes.iter().position(|(k, _)| k == "xmi:id"));
        let mut at = at.map_or(0, |i| i + 1);
        for (k, v) in &extra.attributes {
            if let Some(existing) = x.attributes.iter_mut().find(|(n, _)| n == k) {
                // Names taken from the template (of graphs and the module).
                existing.1 = v.clone();
            } else {
                x.attributes.insert(at, (k.clone(), v.clone()));
                at += 1;
            }
        }
        x.raw.extend(extra.children.iter().cloned());
        match &extra.name {
            Some(Some(name)) => match x.attributes.iter_mut().find(|(k, _)| k == "name") {
                Some(existing) => existing.1 = name.clone(),
                None => x.attributes.insert(at.min(x.attributes.len()), ("name".into(), name.clone())),
            },
            Some(None) => x.attributes.retain(|(k, _)| k != "name"),
            None => {}
        }
    }

    // ------------------------------------------------------------------ references

    /// The URI of the file (or registered package) a root package comes from.
    fn package_base(&self, root: PackageId) -> String {
        let mm = self.mm;
        let pkg = mm.package(root);
        if root == mm.ecore_package() {
            return pkg.ns_uri.clone();
        }
        let uri = &mm.resources()[pkg.resource].uri;
        if uri.to_ascii_lowercase().ends_with(".ecore") {
            if Path::new(uri).is_absolute() && !self.options.path.is_empty() {
                return oxy_ecore::ecore_writer::relative_path(self.options.path, uri);
            }
            return Path::new(uri).file_name().map_or_else(|| uri.clone(), |f| f.to_string_lossy().into_owned());
        }
        pkg.ns_uri.clone()
    }

    /// Where a package is: the URI of its file, the root's index in it (empty for the first)
    /// and the path of package names below the root.
    fn locate(&self, p: PackageId) -> (String, String, Vec<String>) {
        let mm = self.mm;
        let mut path = Vec::new();
        let mut root = p;
        while let Some(sp) = mm.package(root).super_package {
            path.push(mm.package(root).name.clone());
            root = sp;
        }
        path.reverse();
        let index = mm.resources()[mm.package(root).resource].roots.iter().position(|r| *r == root).unwrap_or(0);
        let prefix = if index == 0 { String::new() } else { index.to_string() };
        (self.package_base(root), prefix, path)
    }

    /// `base#/` or `base#//sub` for a package.
    fn package_href(&self, p: PackageId) -> String {
        let (base, prefix, path) = self.locate(p);
        if path.is_empty() { format!("{base}#/{prefix}") } else { format!("{base}#/{prefix}/{}", path.join("/")) }
    }

    /// `base#//Class` or `base#//sub/Class`.
    fn classifier_href(&self, c: ClassifierId) -> String {
        let (base, prefix, mut path) = self.locate(self.mm.classifier(c).package);
        path.push(self.mm.classifier(c).name.clone());
        format!("{base}#/{prefix}/{}", path.join("/"))
    }

    fn feature_href(&self, f: FeatureId) -> String {
        let feature = self.mm.feature(f);
        format!("{}/{}", self.classifier_href(feature.owner), feature.name)
    }

    /// A `<type href="..."/>` child.
    fn type_ref(&self, href: String) -> X {
        X::new("type").with("href", href)
    }

    /// The type of a parameter (with its `xsi:type`, as the reference's type is `EClassifier`).
    fn param_type(&self, c: ClassifierId) -> X {
        let kind = match self.mm.classifier(c).kind {
            ClassifierKind::Class(_) => "ecore:EClass",
            ClassifierKind::Enum(_) => "ecore:EEnum",
            ClassifierKind::DataType(_) => "ecore:EDataType",
        };
        X::new("type").with("xsi:type", kind).with("href", self.classifier_href(c))
    }

    fn js(&self, e: &CExpr, params: &[Param]) -> String {
        js::print_compiled(self.mm, e, params)
    }

    // ------------------------------------------------------------------ module

    fn module(&mut self) -> String {
        let mut root = self.element("henshin:Module", None, "module");
        let mut head = X::new("henshin:Module");
        head.attr("xmi:version", "2.0");
        head.attr("xmlns:xmi", "http://www.omg.org/XMI");
        head.attr("xmlns:xsi", "http://www.w3.org/2001/XMLSchema-instance");
        head.attr("xmlns:ecore", "http://www.eclipse.org/emf/2002/Ecore");
        head.attr("xmlns:henshin", super::HENSHIN_NS);
        head.attributes.append(&mut root.attributes);
        root = head;
        let name = self.options.name.map(str::to_string).unwrap_or_else(|| {
            Path::new(self.options.path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
        });
        if !name.is_empty() {
            root.attr("name", name);
        }
        self.finish(&mut root, "module");
        for p in &self.module.packages {
            root.push(X::new("imports").with("href", self.package_href(*p)));
        }
        // Rules and units in the order of the text.
        let mut order: Vec<(usize, Callable)> = self
            .module
            .rules
            .iter()
            .enumerate()
            .map(|(i, r)| (r.span.start, Callable::Rule(i)))
            .chain(self.module.units.iter().enumerate().map(|(i, u)| (u.span.start, Callable::Unit(i))))
            .collect();
        order.sort_by_key(|(at, c)| (*at, matches!(c, Callable::Unit(_))));
        let module = self.module;
        for (_, c) in order {
            match c {
                Callable::Rule(i) => {
                    let rule = &module.rules[i];
                    let (x, _) = self.rule(rule, &unit_key(&rule.name), "units", None);
                    self.units.push(Some(x));
                }
                Callable::Unit(i) => {
                    let unit = &module.units[i];
                    self.unit(&unit.name, &unit.params, &unit.body, None);
                }
            }
        }
        for u in self.units.drain(..).flatten() {
            root.push(u);
        }
        let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        root.write(&mut out, 0);
        out
    }

    // ------------------------------------------------------------------ rules

    /// The attributes of a node's right-hand side: the values it sets, and the matched ones
    /// it keeps.
    fn rhs_attributes(n: &RuleNode) -> Vec<(FeatureId, &CExpr)> {
        let mut out = Vec::new();
        match n.kind {
            NodeKind::Create => out.extend(n.sets.iter().map(|(f, e)| (*f, e))),
            NodeKind::Preserve => {
                for (f, e) in &n.matches {
                    if n.unsets.contains(f) || out.iter().any(|(g, _)| g == f) {
                        continue;
                    }
                    let set = n.sets.iter().find(|(g, _)| g == f).map(|(_, s)| s);
                    out.push((*f, set.unwrap_or(e)));
                }
                for (f, e) in &n.sets {
                    if !out.iter().any(|(g, _)| g == f) {
                        out.push((*f, e));
                    }
                }
            }
            NodeKind::Delete => {}
        }
        out
    }

    fn attribute(&mut self, node_key: &str, f: FeatureId, value: &CExpr, params: &[Param]) -> X {
        let key = self.keys.next(format!("{node_key}/attr:{}", self.mm.feature(f).name));
        let mut x = self.element("attributes", None, &key);
        x.attr("value", self.js(value, params));
        x.push(self.type_ref(self.feature_href(f)));
        self.finish(&mut x, &key);
        x
    }

    /// A node with its attributes (its edges are added later).
    fn node(
        &mut self,
        graph_key: &str,
        name: &str,
        class: ClassifierId,
        attributes: &[(FeatureId, &CExpr)],
        params: &[Param],
    ) -> NodeX {
        let key = self.keys.next(format!("{graph_key}/node:{name}"));
        let mut x = self.element("nodes", None, &key);
        let id = x.attributes.last().map(|(_, v)| v.clone()).unwrap_or_default();
        if !name.is_empty() {
            x.attr("name", name);
        }
        self.finish(&mut x, &key);
        x.push(self.type_ref(self.classifier_href(class)));
        for (f, e) in attributes {
            let a = self.attribute(&key, *f, e, params);
            x.push(a);
        }
        NodeX { x, id }
    }

    /// A graph of nodes and edges (`(source, feature, target)` by node index), with the
    /// nodes' `incoming` and `outgoing` lists.
    fn graph(
        &mut self,
        tag: &str,
        key: &str,
        name: &str,
        nodes: Vec<NodeX>,
        edges: &[(usize, FeatureId, usize)],
        formula: Option<X>,
    ) -> X {
        let mut g = self.element(tag, None, key);
        g.attr("name", name);
        self.finish(&mut g, key);
        let mut incoming: Vec<Vec<String>> = vec![Vec::new(); nodes.len()];
        let mut outgoing: Vec<Vec<String>> = vec![Vec::new(); nodes.len()];
        let mut edge_xs = Vec::new();
        for (s, f, t) in edges {
            let ekey = self.keys.next(format!(
                "{key}/edge:{}->{}:{}",
                node_name(&nodes[*s]),
                node_name(&nodes[*t]),
                self.mm.feature(*f).name
            ));
            let mut e = self.element("edges", None, &ekey);
            let id = e.attributes.last().map(|(_, v)| v.clone()).unwrap_or_default();
            e.attr("source", nodes[*s].id.clone());
            e.attr("target", nodes[*t].id.clone());
            self.finish(&mut e, &ekey);
            e.push(self.type_ref(self.feature_href(*f)));
            outgoing[*s].push(id.clone());
            incoming[*t].push(id);
            edge_xs.push(e);
        }
        for (i, mut n) in nodes.into_iter().enumerate() {
            // `incoming` and `outgoing` follow the name (and the template's extras).
            if !incoming[i].is_empty() {
                n.x.attributes.push(("incoming".into(), incoming[i].join(" ")));
            }
            if !outgoing[i].is_empty() {
                n.x.attributes.push(("outgoing".into(), outgoing[i].join(" ")));
            }
            g.push(n.x);
        }
        for e in edge_xs {
            g.push(e);
        }
        if let Some(f) = formula {
            g.push(f);
        }
        g
    }

    /// A rule (or multi-rule, with its kernel and the kernel's node ids).
    fn rule(&mut self, rule: &Rule, key: &str, tag: &str, kernel: Option<(&Rule, &RuleIds)>) -> (X, RuleIds) {
        let mut x = self.element(tag, (tag == "units").then_some("henshin:Rule"), key);
        x.attr("name", rule.name.clone());
        let (kernel_dangling, kernel_injective) = kernel.map_or((true, true), |(k, _)| (k.check_dangling, k.injective));
        if !rule.check_dangling || !kernel_dangling {
            x.attr("checkDangling", rule.check_dangling.to_string());
        }
        if !rule.injective || !kernel_injective {
            x.attr("injectiveMatching", rule.injective.to_string());
        }
        self.finish(&mut x, key);
        for p in &rule.params {
            let px = self.parameter(&rule.name, key, p);
            x.push(px);
        }
        // Nodes: the left-hand side has the preserved and deleted ones, the right-hand side the
        // preserved and created ones.
        let lhs_key = format!("{key}/lhs");
        let rhs_key = format!("{key}/rhs");
        let mut ids = RuleIds::default();
        let mut lhs_nodes = Vec::new();
        let mut rhs_nodes = Vec::new();
        let mut lhs_index = HashMap::new();
        let mut rhs_index = HashMap::new();
        for (i, n) in rule.nodes.iter().enumerate() {
            if n.kind.is_lhs() {
                let attrs: Vec<(FeatureId, &CExpr)> = n.matches.iter().map(|(f, e)| (*f, e)).collect();
                let nx = self.node(&lhs_key, &n.name, n.class, &attrs, &rule.params);
                ids.lhs.push(Some(nx.id.clone()));
                lhs_index.insert(i, lhs_nodes.len());
                lhs_nodes.push(nx);
            } else {
                ids.lhs.push(None);
            }
            if n.kind != NodeKind::Delete {
                let attrs = Self::rhs_attributes(n);
                let nx = self.node(&rhs_key, &n.name, n.class, &attrs, &rule.params);
                ids.rhs.push(Some(nx.id.clone()));
                rhs_index.insert(i, rhs_nodes.len());
                rhs_nodes.push(nx);
            } else {
                ids.rhs.push(None);
            }
        }
        // Edges: a multi-rule has copies of its kernel's.
        let kernel_edges = kernel.map(|(k, _)| k.edges.as_slice()).unwrap_or_default();
        let mut lhs_edges = Vec::new();
        let mut rhs_edges = Vec::new();
        for e in kernel_edges.iter().chain(&rule.edges) {
            if e.kind != NodeKind::Create
                && let (Some(s), Some(t)) = (lhs_index.get(&e.source), lhs_index.get(&e.target))
            {
                lhs_edges.push((*s, e.feature, *t));
            }
            if e.kind != NodeKind::Delete
                && let (Some(s), Some(t)) = (rhs_index.get(&e.source), rhs_index.get(&e.target))
            {
                rhs_edges.push((*s, e.feature, *t));
            }
        }
        // The application condition, over the left-hand side's nodes.
        let host = Host {
            key: lhs_key.clone(),
            slots: rule
                .nodes
                .iter()
                .enumerate()
                .filter_map(|(i, n)| Some((i, (ids.lhs[i].clone()?, n.name.clone(), n.class))))
                .collect(),
        };
        let formula = self.formula(&rule.condition, &host, &format!("{lhs_key}/formula"), "formula", &rule.params);
        let lhs = self.graph("lhs", &lhs_key, "Lhs", lhs_nodes, &lhs_edges, formula);
        let rhs = self.graph("rhs", &rhs_key, "Rhs", rhs_nodes, &rhs_edges, None);
        x.push(lhs);
        x.push(rhs);
        for (i, c) in rule.conditions.iter().enumerate() {
            let ckey = format!("{key}/condition:{i}");
            let mut cx = self.element("attributeConditions", None, &ckey);
            self.finish(&mut cx, &ckey);
            cx.attr("conditionText", self.js(c, &rule.params));
            x.push(cx);
        }
        for (i, n) in rule.nodes.iter().enumerate() {
            if let (Some(l), Some(r)) = (&ids.lhs[i], &ids.rhs[i]) {
                let mkey = self.keys.next(format!("{key}/mapping:{}", n.name));
                let mut m = self.element("mappings", None, &mkey);
                m.attr("origin", l.clone());
                m.attr("image", r.clone());
                self.finish(&mut m, &mkey);
                x.push(m);
            }
        }
        for m in &rule.multi_rules {
            let (mx, _) = self.rule(m, &format!("{key}/multi:{}", m.name), "multiRules", Some((rule, &ids)));
            x.push(mx);
        }
        if let Some((_, kernel_ids)) = kernel {
            for (i, n) in rule.nodes.iter().enumerate().take(rule.inherited) {
                for (side, origin, image) in [("lhs", &kernel_ids.lhs, &ids.lhs), ("rhs", &kernel_ids.rhs, &ids.rhs)] {
                    if let (Some(Some(o)), Some(Some(im))) = (origin.get(i), image.get(i)) {
                        let mkey = self.keys.next(format!("{key}/multiMapping:{side}:{}", n.name));
                        let mut m = self.element("multiMappings", None, &mkey);
                        m.attr("origin", o.clone());
                        m.attr("image", im.clone());
                        self.finish(&mut m, &mkey);
                        x.push(m);
                    }
                }
            }
        }
        // A multi-rule has its kernel's imports, and its own.
        let inherited = kernel.map(|(k, _)| k.java_imports.as_slice()).unwrap_or_default();
        for j in rule.java_imports.iter().filter(|j| !inherited.contains(j)) {
            x.push(X { tag: "javaImports".into(), text: Some(j.clone()), ..X::default() });
        }
        (x, ids)
    }

    fn parameter(&mut self, unit: &str, unit_key: &str, p: &Param) -> X {
        let key = format!("{unit_key}/param:{}", p.name);
        let id = self.param_id(unit, &p.name);
        let mut x = X::new("parameters");
        x.attr("xmi:id", id);
        x.attr("name", p.name.clone());
        if p.kind != ParamKind::Unknown {
            x.attr("kind", p.kind.keyword());
        }
        self.finish(&mut x, &key);
        if let ParamType::Data(c) | ParamType::Class(c) = p.ty {
            x.push(self.param_type(c));
        }
        x
    }

    // ------------------------------------------------------------------ conditions

    /// The formula of a condition (`None` for `true`). `path` is the key of its position.
    fn formula(&mut self, c: &Condition, host: &Host, path: &str, tag: &str, params: &[Param]) -> Option<X> {
        match c {
            Condition::True => None,
            Condition::Graph(g) => Some(self.nested_condition(g, host, tag, params)),
            Condition::Not(inner) => {
                let mut x = self.element(tag, Some("henshin:Not"), path);
                self.finish(&mut x, path);
                if let Some(child) = self.formula(inner, host, &format!("{path}/child"), "child", params) {
                    x.push(child);
                } else {
                    x.push(self.element("child", Some("henshin:True"), &format!("{path}/child")));
                }
                Some(x)
            }
            Condition::And(list) => self.conjunction(list, host, path, tag, params),
            Condition::Or(a, b) | Condition::Xor(a, b) => {
                let t = if matches!(c, Condition::Or(..)) { "henshin:Or" } else { "henshin:Xor" };
                let mut x = self.element(tag, Some(t), path);
                self.finish(&mut x, path);
                for (side, f) in [("left", a), ("right", b)] {
                    let sub = format!("{path}/{side}");
                    let child = self
                        .formula(f, host, &sub, side, params)
                        .unwrap_or_else(|| self.element(side, Some("henshin:True"), &sub));
                    x.push(child);
                }
                Some(x)
            }
        }
    }

    /// A conjunction, nested to the left as Henshin builds them.
    fn conjunction(&mut self, list: &[Condition], host: &Host, path: &str, tag: &str, params: &[Param]) -> Option<X> {
        match list {
            [] => None,
            [single] => self.formula(single, host, path, tag, params),
            [rest @ .., last] => {
                let mut x = self.element(tag, Some("henshin:And"), path);
                self.finish(&mut x, path);
                let left = format!("{path}/left");
                let right = format!("{path}/right");
                let l = self
                    .conjunction(rest, host, &left, "left", params)
                    .unwrap_or_else(|| self.element("left", Some("henshin:True"), &left));
                let r = self
                    .formula(last, host, &right, "right", params)
                    .unwrap_or_else(|| self.element("right", Some("henshin:True"), &right));
                x.push(l);
                x.push(r);
                Some(x)
            }
        }
    }

    /// A nested condition: its conclusion has copies of the host's nodes it refers to (mapped
    /// from them) and its own nodes.
    fn nested_condition(&mut self, g: &ConditionGraph, host: &Host, tag: &str, params: &[Param]) -> X {
        let key = self.keys.next(format!("{}/nc:{}", host.key, g.name));
        let mut nc = self.element(tag, Some("henshin:NestedCondition"), &key);
        self.finish(&mut nc, &key);
        let conclusion_key = format!("{key}/conclusion");
        let own: HashSet<usize> = g.nodes.iter().map(|n| n.slot).collect();
        let mut outer = Vec::new();
        used_outer(g, &own, &mut outer);
        let mut nodes = Vec::new();
        let mut local: HashMap<usize, usize> = HashMap::new();
        let mut inner_host = Host { key: conclusion_key.clone(), slots: HashMap::new() };
        let mut mappings = Vec::new();
        for slot in outer {
            let Some((origin, name, class)) = host.slots.get(&slot).cloned() else { continue };
            let checks: Vec<(FeatureId, &CExpr)> =
                g.checks.iter().filter(|(s, _, _)| *s == slot).map(|(_, f, e)| (*f, e)).collect();
            let nx = self.node(&conclusion_key, &name, class, &checks, params);
            let mkey = self.keys.next(format!("{key}/mapping:{name}"));
            let mut m = self.element("mappings", None, &mkey);
            m.attr("origin", origin);
            m.attr("image", nx.id.clone());
            self.finish(&mut m, &mkey);
            mappings.push(m);
            inner_host.slots.insert(slot, (nx.id.clone(), name, class));
            local.insert(slot, nodes.len());
            nodes.push(nx);
        }
        for n in &g.nodes {
            let attrs: Vec<(FeatureId, &CExpr)> = n.matches.iter().map(|(f, e)| (*f, e)).collect();
            let nx = self.node(&conclusion_key, &n.name, n.class, &attrs, params);
            inner_host.slots.insert(n.slot, (nx.id.clone(), n.name.clone(), n.class));
            local.insert(n.slot, nodes.len());
            nodes.push(nx);
        }
        let edges: Vec<(usize, FeatureId, usize)> =
            g.edges.iter().filter_map(|(s, f, t)| Some((*local.get(s)?, *f, *local.get(t)?))).collect();
        let formula = self.formula(&g.condition, &inner_host, &format!("{conclusion_key}/formula"), "formula", params);
        let conclusion = self.graph("conclusion", &conclusion_key, &g.name, nodes, &edges, formula);
        nc.push(conclusion);
        for m in mappings {
            nc.push(m);
        }
        nc
    }

    // ------------------------------------------------------------------ units

    fn unit_id(&mut self, name: &str) -> String {
        self.id(&unit_key(name))
    }

    fn param_id(&mut self, unit: &str, param: &str) -> String {
        self.id(&format!("{}/param:{param}", unit_key(unit)))
    }

    /// A unit and the units of its nested blocks (after it). `body` is what the unit does;
    /// `nested` the role of a unit made for a nested block.
    fn unit(&mut self, name: &str, params: &[Param], body: &Step, nested: Option<&str>) -> String {
        let key = unit_key(name);
        let slot = self.units.len();
        self.units.push(None);
        // A unit that only is a loop, a choice, … is a unit of that kind.
        let body = match body {
            Step::Sequence { steps, strict: true, rollback: true } if steps.len() == 1 && is_control(&steps[0]) => {
                &steps[0]
            }
            other => other,
        };
        let xsi = match body {
            Step::Sequence { .. } | Step::Call { .. } => "henshin:SequentialUnit",
            Step::Independent(_) => "henshin:IndependentUnit",
            Step::Priority(_) => "henshin:PriorityUnit",
            Step::If { .. } => "henshin:ConditionalUnit",
            Step::For { .. } => "henshin:IteratedUnit",
            Step::While(_) => "henshin:LoopUnit",
        };
        let id = self.unit_id(name);
        let mut x = X::new("units");
        x.attr("xsi:type", xsi);
        x.attr("xmi:id", id.clone());
        x.attr("name", name);
        self.finish(&mut x, &key);
        if let Some(role) = nested {
            let akey = format!("{key}/annotation:nested");
            let mut a = self.element("annotations", None, &akey);
            a.attr("key", NESTED);
            a.attr("value", role);
            x.children.insert(0, a);
        }
        for p in params {
            let px = self.parameter(name, &key, p);
            x.push(px);
        }
        let mut cx =
            UnitCx { name: name.to_string(), key: key.clone(), params, mappings: Vec::new(), calls: HashMap::new() };
        let mut refs: Vec<(&str, String)> = Vec::new();
        match body {
            Step::Sequence { steps, strict, rollback } => {
                let subs: Vec<String> =
                    steps.iter().enumerate().map(|(i, s)| self.item_ref(&mut cx, s, &(i + 1).to_string())).collect();
                if !subs.is_empty() {
                    refs.push(("subUnits", subs.join(" ")));
                }
                if !strict {
                    refs.push(("strict", "false".into()));
                }
                if !rollback {
                    refs.push(("rollback", "false".into()));
                }
            }
            Step::Call { .. } => {
                let sub = self.item_ref(&mut cx, body, "1");
                refs.push(("subUnits", sub));
            }
            Step::Independent(branches) | Step::Priority(branches) => {
                let subs: Vec<String> = branches
                    .iter()
                    .enumerate()
                    .map(|(i, b)| self.branch_ref(&mut cx, b, &(i + 1).to_string()))
                    .collect();
                if !subs.is_empty() {
                    refs.push(("subUnits", subs.join(" ")));
                }
            }
            Step::If { condition, then, otherwise } => {
                refs.push(("if", self.branch_ref(&mut cx, condition, "if")));
                refs.push(("then", self.branch_ref(&mut cx, then, "then")));
                if let Some(o) = otherwise {
                    refs.push(("else", self.branch_ref(&mut cx, o, "else")));
                }
            }
            Step::For { count, body } => {
                refs.push(("subUnit", self.branch_ref(&mut cx, body, "body")));
                refs.push(("iterations", self.js(count, params)));
            }
            Step::While(body) => refs.push(("subUnit", self.branch_ref(&mut cx, body, "body"))),
        }
        for (k, v) in refs {
            x.attr(k, v);
        }
        for m in cx.mappings {
            x.push(m);
        }
        self.units[slot] = Some(x);
        id
    }

    /// The unit an element of a sequence calls: the rule or unit it calls, or a unit made for
    /// it.
    fn item_ref(&mut self, cx: &mut UnitCx, step: &Step, path: &str) -> String {
        match step {
            Step::Call { target, args, .. } => {
                // Henshin maps parameters per unit, not per call: a second call of the same rule
                // with other arguments goes into a unit of its own.
                match cx.calls.get(target) {
                    Some(previous) if previous != args => {
                        let wrapper = Step::Sequence { steps: vec![step.clone()], strict: true, rollback: true };
                        return self.nested(cx, &wrapper, path, "inline");
                    }
                    Some(_) => {}
                    None => {
                        cx.calls.insert(*target, args.clone());
                        let module = self.module;
                        let callee = module.name_of(*target).to_string();
                        for (i, a) in args.iter().enumerate() {
                            let Some(p) = module.params(*target).get(i) else { continue };
                            let Some(own) = cx.params.get(*a) else { continue };
                            let ours = self.param_id(&cx.name, &own.name);
                            let theirs = self.param_id(&callee, &p.name);
                            self.mapping(cx, &ours, &theirs, &format!("{}.{}->{callee}.{}", cx.name, own.name, p.name));
                            self.mapping(cx, &theirs, &ours, &format!("{callee}.{}->{}.{}", p.name, cx.name, own.name));
                        }
                    }
                }
                self.unit_id(self.module.name_of(*target))
            }
            Step::Sequence { .. } => self.nested(cx, step, path, "block"),
            _ => self.nested(cx, step, path, "inline"),
        }
    }

    /// The unit a branch (a list of elements: of a choice, a condition, a loop body) is: the
    /// one element's unit, or a unit made for the list.
    fn branch_ref(&mut self, cx: &mut UnitCx, step: &Step, path: &str) -> String {
        match step {
            Step::Sequence { steps, strict: true, rollback: true } if steps.len() == 1 => {
                self.item_ref(cx, &steps[0], path)
            }
            Step::Sequence { .. } => self.nested(cx, step, path, "inline"),
            other => self.item_ref(cx, other, path),
        }
    }

    /// A unit made for a nested block: it has the parameters of the enclosing unit, mapped
    /// from and to them.
    fn nested(&mut self, cx: &mut UnitCx, step: &Step, path: &str, role: &str) -> String {
        let mut name = format!("{}_{path}", cx.name);
        while self.taken.contains(&name) {
            name.push('_');
        }
        self.taken.insert(name.clone());
        for p in cx.params {
            let ours = self.param_id(&cx.name, &p.name);
            let theirs = self.param_id(&name, &p.name);
            self.mapping(cx, &ours, &theirs, &format!("{}.{}->{name}.{}", cx.name, p.name, p.name));
            self.mapping(cx, &theirs, &ours, &format!("{name}.{}->{}.{}", p.name, cx.name, p.name));
        }
        self.unit(&name, cx.params, step, Some(role))
    }

    fn mapping(&mut self, cx: &mut UnitCx, source: &str, target: &str, what: &str) {
        let key = self.keys.next(format!("{}/mapping:{what}", cx.key));
        let mut m = self.element("parameterMappings", None, &key);
        m.attr("source", source);
        m.attr("target", target);
        self.finish(&mut m, &key);
        cx.mappings.push(m);
    }
}

fn node_name(n: &NodeX) -> &str {
    n.x.attributes.iter().find(|(k, _)| k == "name").map_or("", |(_, v)| v.as_str())
}

/// Whether a step is a control structure (not a call or a sequence).
fn is_control(s: &Step) -> bool {
    !matches!(s, Step::Call { .. } | Step::Sequence { .. })
}

/// The slots of the enclosing graphs a condition graph (and the conditions nested in it) refer
/// to, in the order they are first used.
fn used_outer(g: &ConditionGraph, own: &HashSet<usize>, out: &mut Vec<usize>) {
    let add = |s: usize, out: &mut Vec<usize>| {
        if !own.contains(&s) && !out.contains(&s) {
            out.push(s);
        }
    };
    for (s, _, t) in &g.edges {
        add(*s, out);
        add(*t, out);
    }
    for (s, _, _) in &g.checks {
        add(*s, out);
    }
    fn nested(c: &Condition, own: &HashSet<usize>, out: &mut Vec<usize>) {
        match c {
            Condition::True => {}
            Condition::Graph(g) => {
                let mut inner = Vec::new();
                let inner_own: HashSet<usize> = g.nodes.iter().map(|n| n.slot).collect();
                used_outer(g, &inner_own, &mut inner);
                for s in inner {
                    if !own.contains(&s) && !out.contains(&s) {
                        out.push(s);
                    }
                }
            }
            Condition::Not(c) => nested(c, own, out),
            Condition::And(cs) => cs.iter().for_each(|c| nested(c, own, out)),
            Condition::Or(a, b) | Condition::Xor(a, b) => {
                nested(a, own, out);
                nested(b, own, out);
            }
        }
    }
    nested(&g.condition, own, out);
}
