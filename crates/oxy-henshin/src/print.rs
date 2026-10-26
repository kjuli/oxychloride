//! Writes syntax trees as Henshin text, in the layout of Henshin's examples: one element per
//! line, blocks indented by one level, the edges of a graph in one `edges [...]` list.
//!
//! The printer is the inverse of the [parser](crate::syntax::parse) up to layout and comments:
//! parsing its output gives the same tree (with other spans). It writes the documents that
//! [`xmi::read`](crate::xmi::read) makes from `.henshin` files, and the text that diagram edits
//! insert.

use crate::syntax::*;

/// Henshin text with the given unit of indentation (e.g. a tab).
#[derive(Clone, Debug)]
pub struct Printer {
    pub indent: String,
}

impl Default for Printer {
    fn default() -> Self {
        Printer { indent: "\t".into() }
    }
}

/// A name, escaped with `^` if it is a keyword.
pub fn name(n: &str) -> String {
    if is_keyword(n) { format!("^{n}") } else { n.to_string() }
}

/// A dotted name.
pub fn qname(q: &QName) -> String {
    q.parts.iter().map(|p| name(&p.text)).collect::<Vec<_>>().join(".")
}

/// A string literal of the textual syntax.
pub fn string_literal(s: &str) -> String {
    crate::js::string_literal(s)
}

/// An expression of the textual syntax.
pub fn expr(e: &Expr) -> String {
    fn prec(op: BinOp) -> u8 {
        match op {
            BinOp::Or => 1,
            BinOp::And => 2,
            BinOp::Eq | BinOp::Ne => 3,
            BinOp::Ge | BinOp::Le | BinOp::Gt | BinOp::Lt => 4,
            BinOp::Add | BinOp::Sub => 5,
            BinOp::Mul | BinOp::Div => 6,
        }
    }
    fn go(e: &Expr) -> (String, u8) {
        const ATOM: u8 = 7;
        match &e.kind {
            ExprKind::Name(n) => (name(n), ATOM),
            ExprKind::Qualified(parts) => (parts.iter().map(|p| name(p)).collect::<Vec<_>>().join("."), ATOM),
            ExprKind::Call(f, args) => {
                let args: Vec<String> = args.iter().map(|a| go(a).0).collect();
                (format!("{f}({})", args.join(", ")), ATOM)
            }
            ExprKind::Str(s) => (string_literal(s), ATOM),
            ExprKind::Int(i) => (i.to_string(), if *i < 0 { ATOM - 1 } else { ATOM }),
            ExprKind::Real(r) => (crate::value::format_real(*r), if *r < 0.0 { ATOM - 1 } else { ATOM }),
            ExprKind::Bool(b) => (b.to_string(), ATOM),
            ExprKind::Not(inner) | ExprKind::Neg(inner) => {
                let op = if matches!(e.kind, ExprKind::Not(_)) { "!" } else { "-" };
                let (s, p) = go(inner);
                (if p < ATOM { format!("{op}({s})") } else { format!("{op}{s}") }, ATOM)
            }
            ExprKind::Binary(op, l, r) => {
                let p = prec(*op);
                let (ls, lp) = go(l);
                let (rs, rp) = go(r);
                let ls = if lp < p { format!("({ls})") } else { ls };
                let rs = if rp <= p { format!("({rs})") } else { rs };
                (format!("{ls} {} {rs}", op.symbol()), p)
            }
            ExprKind::Error => (String::new(), ATOM),
        }
    }
    go(e).0
}

/// A formula of condition graphs (`!a AND (b OR c)`).
pub fn logic(l: &Logic) -> String {
    fn go(l: &Logic) -> (String, u8) {
        match l {
            Logic::Ref(n) => (name(&n.text), 3),
            Logic::Error => (String::new(), 3),
            Logic::Not(inner) => {
                let (s, p) = go(inner);
                (if p < 3 { format!("!({s})") } else { format!("!{s}") }, 3)
            }
            Logic::And(a, b) => {
                let (a, ap) = go(a);
                let (b, bp) = go(b);
                let a = if ap < 2 { format!("({a})") } else { a };
                let b = if bp <= 2 { format!("({b})") } else { b };
                (format!("{a} AND {b}"), 2)
            }
            Logic::Or(a, b) | Logic::Xor(a, b) => {
                let op = if matches!(l, Logic::Or(..)) { "OR" } else { "XOR" };
                let (a, _) = go(a);
                let (b, bp) = go(b);
                let b = if bp <= 1 { format!("({b})") } else { b };
                (format!("{a} {op} {b}"), 1)
            }
        }
    }
    go(l).0
}

/// `IN name:Type`.
pub fn param(p: &ParamDecl) -> String {
    let kind = p.kind.keyword();
    let ty = if p.ty.parts.is_empty() { String::new() } else { format!(":{}", qname(&p.ty)) };
    format!("{kind}{}{}{ty}", if kind.is_empty() { "" } else { " " }, name(&p.name.text))
}

/// `(IN a:EInt, b:Node)`.
pub fn params(ps: &[ParamDecl]) -> String {
    format!("({})", ps.iter().map(param).collect::<Vec<_>>().join(", "))
}

fn action(a: &Option<(Action, std::ops::Range<usize>)>) -> String {
    a.as_ref().map(|(a, _)| format!("{} ", a.keyword())).unwrap_or_default()
}

/// `set name=value`, `forbid name=value`, …
pub fn attribute(a: &AttributeDecl) -> String {
    let prefix = if a.action.is_some() {
        action(&a.action)
    } else if a.set {
        "set ".to_string()
    } else {
        String::new()
    };
    format!("{prefix}{}={}", qname(&a.name), expr(&a.value))
}

/// `(create a->b:ref)`.
pub fn edge(e: &EdgeDecl) -> String {
    format!("({}{}->{}:{})", action(&e.action), name(&e.source.text), name(&e.target.text), qname(&e.ty))
}

/// `edges [(a->b:r), (create a->c:s)]`.
pub fn edges(es: &[EdgeDecl]) -> String {
    format!("edges [{}]", es.iter().map(edge).collect::<Vec<_>>().join(", "))
}

/// A unit call, `name(a, b)`.
pub fn call(target: &str, args: &[String]) -> String {
    format!("{}({})", name(target), args.iter().map(|a| name(a)).collect::<Vec<_>>().join(", "))
}

impl Printer {
    pub fn with_indent(indent: impl Into<String>) -> Printer {
        Printer { indent: indent.into() }
    }

    fn line(&self, out: &mut String, depth: usize, text: &str) {
        for _ in 0..depth {
            out.push_str(&self.indent);
        }
        out.push_str(text);
        out.push('\n');
    }

    /// The whole document.
    pub fn document(&self, doc: &Document) -> String {
        let mut out = String::new();
        for i in &doc.imports {
            out.push_str(&format!("ePackageImport {}\n", import(i)));
        }
        for e in &doc.elements {
            out.push('\n');
            out.push_str(&self.element(e));
        }
        out
    }

    /// A rule or unit (with a final line break), at the top level.
    pub fn element(&self, e: &Element) -> String {
        let mut out = String::new();
        match e {
            Element::Rule(r) => {
                self.line(&mut out, 0, &format!("rule {}{} {{", name(&r.name.text), params(&r.params)));
                self.rule_items(&mut out, 1, &r.items);
                self.line(&mut out, 0, "}");
            }
            Element::Unit(u) => {
                self.line(&mut out, 0, &format!("unit {}{} {{", name(&u.name.text), params(&u.params)));
                self.unit_items(&mut out, 1, &u.items);
                self.line(&mut out, 0, "}");
            }
        }
        out
    }

    pub fn rule_items(&self, out: &mut String, depth: usize, items: &[RuleItem]) {
        for item in items {
            self.rule_item(out, depth, item);
        }
    }

    pub fn rule_item(&self, out: &mut String, depth: usize, item: &RuleItem) {
        match item {
            RuleItem::JavaImport(q, _) => self.line(out, depth, &format!("javaImport {}", qname(q))),
            RuleItem::CheckDangling(v, _) => self.line(out, depth, &format!("checkDangling {v}")),
            RuleItem::InjectiveMatching(v, _) => self.line(out, depth, &format!("injectiveMatching {v}")),
            RuleItem::Conditions(c, _) => {
                let list: Vec<String> = c.iter().map(expr).collect();
                self.line(out, depth, &format!("conditions [{}]", list.join(", ")));
            }
            RuleItem::Graph(g) => {
                self.line(out, depth, "graph {");
                self.graph_items(out, depth + 1, &g.items);
                self.line(out, depth, "}");
            }
        }
    }

    pub fn graph_items(&self, out: &mut String, depth: usize, items: &[GraphItem]) {
        for item in items {
            self.graph_item(out, depth, item);
        }
    }

    /// A node declaration with its attribute block.
    pub fn node(&self, out: &mut String, depth: usize, n: &NodeDecl) {
        let head = format!("{}node {}:{}", action(&n.action), name(&n.name.text), qname(&n.class));
        self.attribute_block(out, depth, head, &n.attributes);
    }

    fn attribute_block(&self, out: &mut String, depth: usize, head: String, attributes: &[AttributeDecl]) {
        if attributes.is_empty() {
            self.line(out, depth, &head);
            return;
        }
        self.line(out, depth, &format!("{head} {{"));
        for a in attributes {
            self.line(out, depth + 1, &attribute(a));
        }
        self.line(out, depth, "}");
    }

    pub fn graph_item(&self, out: &mut String, depth: usize, item: &GraphItem) {
        match item {
            GraphItem::Node(n) => self.node(out, depth, n),
            GraphItem::Reuse(r) => {
                self.attribute_block(out, depth, format!("reuse {}", name(&r.name.text)), &r.attributes)
            }
            GraphItem::Edges(e) => self.line(out, depth, &edges(&e.edges)),
            GraphItem::MultiRule(m) => {
                self.line(out, depth, &format!("multiRule {} {{", name(&m.name.text)));
                self.rule_items(out, depth + 1, &m.items);
                self.line(out, depth, "}");
            }
            GraphItem::Formula(f) => {
                self.line(out, depth, "matchingFormula {");
                self.line(out, depth + 1, &format!("formula {}", logic(&f.formula)));
                for g in &f.graphs {
                    self.line(out, depth + 1, &format!("conditionGraph {} {{", name(&g.name.text)));
                    self.graph_items(out, depth + 2, &g.items);
                    self.line(out, depth + 1, "}");
                }
                self.line(out, depth, "}");
            }
        }
    }

    pub fn unit_items(&self, out: &mut String, depth: usize, items: &[UnitItem]) {
        for item in items {
            self.unit_item(out, depth, item);
        }
    }

    /// Unit elements on one line, separated by spaces (in `independent [...]` and the like).
    fn inline_items(items: &[UnitItem]) -> String {
        items
            .iter()
            .map(|i| match i {
                UnitItem::Call { target, args, .. } => {
                    call(&target.text, &args.iter().map(|a| a.text.clone()).collect::<Vec<_>>())
                }
                other => {
                    let mut s = String::new();
                    Printer::with_indent("").unit_item(&mut s, 0, other);
                    s.split_whitespace().collect::<Vec<_>>().join(" ")
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn unit_item(&self, out: &mut String, depth: usize, item: &UnitItem) {
        match item {
            UnitItem::Call { target, args, .. } => {
                let args: Vec<String> = args.iter().map(|a| a.text.clone()).collect();
                self.line(out, depth, &call(&target.text, &args));
            }
            UnitItem::Sequence(items, _) => {
                self.line(out, depth, "{");
                self.unit_items(out, depth + 1, items);
                self.line(out, depth, "}");
            }
            UnitItem::Strict(v, _) => self.line(out, depth, &format!("strict {v}")),
            UnitItem::Rollback(v, _) => self.line(out, depth, &format!("rollback {v}")),
            UnitItem::Independent(lists, _) | UnitItem::Priority(lists, _) => {
                let keyword = if matches!(item, UnitItem::Independent(..)) { "independent" } else { "priority" };
                let lists: Vec<String> = lists.iter().map(|l| Printer::inline_items(l)).collect();
                self.line(out, depth, &format!("{keyword} [{}]", lists.join(", ")));
            }
            UnitItem::If { condition, then, otherwise, .. } => {
                self.line(out, depth, &format!("if ({}) then {{", Printer::inline_items(condition)));
                self.unit_items(out, depth + 1, then);
                match otherwise {
                    Some(o) => {
                        self.line(out, depth, "} else {");
                        self.unit_items(out, depth + 1, o);
                        self.line(out, depth, "}");
                    }
                    None => self.line(out, depth, "}"),
                }
            }
            UnitItem::For { count, body, .. } => {
                self.line(out, depth, &format!("for ({}) {{", expr(count)));
                self.unit_items(out, depth + 1, body);
                self.line(out, depth, "}");
            }
            UnitItem::While { body, .. } => {
                self.line(out, depth, "while {");
                self.unit_items(out, depth + 1, body);
                self.line(out, depth, "}");
            }
        }
    }
}

/// An import: a dotted package name, or a string (an nsURI).
pub fn import(q: &QName) -> String {
    let dotted = q.parts.iter().all(|p| {
        let mut chars = p.text.chars();
        chars.next().is_some_and(|c| c.is_alphabetic() || c == '_') && chars.all(|c| c.is_alphanumeric() || c == '_')
    });
    if dotted { qname(q) } else { string_literal(&q.text()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tree without spans, for comparing.
    fn shape(text: &str) -> String {
        let (doc, problems) = parse(text);
        assert!(problems.is_empty(), "{problems:?}");
        let printed = Printer::default().document(&doc);
        let (again, problems) = parse(&printed);
        assert!(problems.is_empty(), "{printed}\n{problems:?}");
        assert_eq!(Printer::default().document(&again), printed);
        printed
    }

    #[test]
    fn prints_what_it_parses() {
        let text = r#"ePackageImport bank
rule r(IN a:EInt, VAR x:EDouble, bank:Bank) {
  checkDangling false
  javaImport java.lang
  conditions [a + 2 * (b - 1) >= 3 AND !c OR d == "x\"y", -(a - 1) < 0]
  graph { node bank:Bank { name="b" set credit=x - a forbid id=3 } create node n:Account
    forbid node f:Account reuse bank { delete x=1 }
    edges [(bank->n:accounts), (create bank -> f : accounts)]
    multiRule m { graph { node k:Client edges [(bank->k:clients)] } }
    matchingFormula { formula !g1 AND (g2 OR g3) XOR !(g1 AND g2) conditionGraph g1 { node y:Bank } }
  }
}
unit u(IN a:EInt) {
  strict false a(a) { b() } independent [ a() b(), c() ] priority [a()]
  if (a()) then { b() } else { c() } for (a - 1) { d() } while { e() }
}"#;
        let printed = shape(text);
        assert!(
            printed.contains("\tconditions [a + 2 * (b - 1) >= 3 AND !c OR d == \"x\\\"y\", -(a - 1) < 0]"),
            "{printed}"
        );
        assert!(
            printed
                .contains("\t\tnode bank:Bank {\n\t\t\tname=\"b\"\n\t\t\tset credit=x - a\n\t\t\tforbid id=3\n\t\t}")
        );
        assert!(printed.contains("\t\tedges [(bank->n:accounts), (create bank->f:accounts)]"));
        assert!(printed.contains("formula !g1 AND (g2 OR g3) XOR !(g1 AND g2)"), "{printed}");
        assert!(printed.contains("\tindependent [a() b(), c()]"));
        assert!(printed.contains("\tif (a()) then {\n\t\tb()\n\t} else {\n\t\tc()\n\t}"));
    }
}
