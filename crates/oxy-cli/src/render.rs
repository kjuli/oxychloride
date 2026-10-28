//! Human-readable terminal output.

use std::fmt::Write as _;

use oxychloride::{
    ClassifierKind, ConstraintInfo, ConstraintKind, ContractResult, Diagnostic, ObjectSummary, Origin, PackageInfo,
    ResultStatus, Severity, ValidationReport,
};

pub struct Renderer {
    color: bool,
}

impl Renderer {
    pub fn new(color: bool) -> Self {
        Renderer { color }
    }

    fn paint(&self, code: &str, s: &str) -> String {
        if self.color { format!("\x1b[{code}m{s}\x1b[0m") } else { s.to_string() }
    }

    pub fn error(&self, s: &str) -> String {
        self.paint("1;31", s)
    }

    pub fn warning(&self, s: &str) -> String {
        self.paint("1;33", s)
    }

    pub fn ok(&self, s: &str) -> String {
        self.paint("32", s)
    }

    pub fn bold(&self, s: &str) -> String {
        self.paint("1", s)
    }

    pub fn dim(&self, s: &str) -> String {
        self.paint("2", s)
    }

    fn blue(&self, s: &str) -> String {
        self.paint("1;34", s)
    }

    /// A rustc-style diagnostic with a source excerpt.
    pub fn diagnostic(&self, d: &Diagnostic, source_text: Option<&str>) -> String {
        let mut out = String::new();
        let label = match d.severity {
            Severity::Error => self.error("error"),
            Severity::Warning => self.warning("warning"),
            Severity::Info => self.bold("info"),
        };
        let _ = writeln!(out, "{label}: {}", self.bold(&d.message));
        let start = d.range.start;
        let location = match &d.origin {
            Origin::Ecore { resource, element, key, location } => {
                let key = if key == "body" || key == "derivation" { format!(" {key}") } else { String::new() };
                match location {
                    // Where it is in the file; the excerpt is of the expression.
                    Some(at) => format!("{resource}:{}:{} ({element}{key})", at.start.line + 1, at.start.column + 1),
                    None => format!("{resource} ({element}{key}) {}:{}", start.line + 1, start.column + 1),
                }
            }
            _ => format!("{}:{}:{}", d.source, start.line + 1, start.column + 1),
        };
        let _ = writeln!(out, "  {} {location}", self.blue("-->"));
        if let Some(line) = source_text.and_then(|t| t.lines().nth(start.line as usize)) {
            let number = (start.line + 1).to_string();
            let pad = " ".repeat(number.len());
            let end_col = if d.range.end.line == start.line { d.range.end.column } else { line.chars().count() as u32 };
            let width = end_col.saturating_sub(start.column).max(1) as usize;
            let paint = |s: &str| match d.severity {
                Severity::Error => self.error(s),
                _ => self.warning(s),
            };
            let _ = writeln!(out, "  {pad} {}", self.blue("|"));
            let _ = writeln!(out, "  {} {} {}", self.blue(&number), self.blue("|"), line.replace('\t', " "));
            let _ = writeln!(
                out,
                "  {pad} {} {}{}",
                self.blue("|"),
                " ".repeat(start.column as usize),
                paint(&"^".repeat(width))
            );
        }
        out.push('\n');
        out
    }

    pub fn diagnostic_summary(&self, diags: &[Diagnostic], documents: usize) -> String {
        let errors = diags.iter().filter(|d| d.severity == Severity::Error).count();
        let warnings = diags.iter().filter(|d| d.severity == Severity::Warning).count();
        let docs = if documents == 1 { "1 document".to_string() } else { format!("{documents} documents") };
        if errors == 0 && warnings == 0 {
            format!("{} {docs} checked, no problems", self.ok("✓"))
        } else {
            format!("{docs} checked: {}, {}", self.count(errors, "error"), self.count(warnings, "warning"))
        }
    }

    pub fn ecore_summary(&self, diags: &[Diagnostic], invariants: usize) -> String {
        let errors = diags.iter().filter(|d| d.severity == Severity::Error).count();
        let warnings = diags.iter().filter(|d| d.severity == Severity::Warning).count();
        if errors == 0 && warnings == 0 {
            format!("{} embedded OCL checked ({invariants} invariants), no problems", self.ok("✓"))
        } else {
            format!(
                "embedded OCL checked ({invariants} invariants): {}, {}",
                self.count(errors, "error"),
                self.count(warnings, "warning")
            )
        }
    }

    fn count(&self, n: usize, what: &str) -> String {
        let s = format!("{n} {what}{}", if n == 1 { "" } else { "s" });
        match (n, what) {
            (0, _) => s,
            (_, "error") => self.error(&s),
            _ => self.warning(&s),
        }
    }

    /// Validation results grouped by constraint, followed by a summary.
    pub fn report(&self, report: &ValidationReport) -> String {
        let mut out = String::new();
        for (index, c) in report.constraints.iter().enumerate() {
            let results: Vec<_> = report.results.iter().filter(|r| r.constraint == index).collect();
            if results.is_empty() {
                continue;
            }
            let worst = if results.iter().any(|r| r.status == ResultStatus::Error) {
                ResultStatus::Error
            } else if results.iter().any(|r| r.status == ResultStatus::Violated) {
                ResultStatus::Violated
            } else {
                ResultStatus::Satisfied
            };
            let mark = match worst {
                ResultStatus::Satisfied => self.ok("✓"),
                ResultStatus::Violated => self.error("✗"),
                ResultStatus::Error => self.warning("!"),
            };
            let _ =
                writeln!(out, "{mark} {} {}", self.bold(&constraint_title(c)), self.dim(&format!("({})", location(c))));
            for r in results {
                let status = match r.status {
                    ResultStatus::Satisfied => self.ok("satisfied"),
                    ResultStatus::Violated => self.error("violated"),
                    ResultStatus::Error => self.warning("error"),
                };
                let object = self.object(&r.object);
                let call = r.call.as_ref().map(|c| format!(" by {c}")).unwrap_or_default();
                match &r.message {
                    Some(m) => {
                        let _ = writeln!(out, "    {status} on {object}{call}: {m}");
                    }
                    None => {
                        let _ = writeln!(out, "    {status} on {object}{call}");
                    }
                }
            }
        }
        for n in &report.notes {
            let _ = writeln!(out, "{} {n}", self.dim("note:"));
        }
        let s = &report.summary;
        let headline = if s.violated == 0 && s.errors == 0 {
            self.ok(&format!("✓ all {} checks passed", s.evaluations))
        } else {
            format!(
                "{} {}, {}",
                self.error("✗"),
                self.count(s.violated, "violation"),
                self.count(s.errors, "evaluation error")
            )
        };
        let _ = writeln!(
            out,
            "\n{headline} {}",
            self.dim(&format!(
                "({} constraints on {} objects, {} evaluations, {} satisfied)",
                s.constraints, s.objects, s.evaluations, s.satisfied
            ))
        );
        out
    }

    /// `Book 'Dune' [library.xmi#//@books.0]`, or with the line for objects of HUTN documents.
    fn object(&self, o: &ObjectSummary) -> String {
        let place = match &o.range {
            Some(r) => format!("[{}:{}]", file_name(&o.resource), r.start.line + 1),
            None => format!("[{}#{}]", file_name(&o.resource), o.fragment),
        };
        format!("{} {}", o.label, self.dim(&place))
    }

    /// A failed pre- or postcondition of an evaluated call.
    pub fn contract(&self, c: &ContractResult) -> String {
        let status = match c.status {
            ResultStatus::Satisfied => self.ok("satisfied"),
            ResultStatus::Violated => self.error("violated"),
            ResultStatus::Error => self.warning("error"),
        };
        let message = c.message.as_ref().map(|m| format!(": {m}")).unwrap_or_default();
        format!(
            "{} {} {status} on {} by {}{message}",
            self.warning("!"),
            self.bold(&constraint_title(&c.constraint)),
            self.object(&c.object),
            c.call
        )
    }

    /// An OCLinEcore-like outline of a package.
    pub fn package(&self, p: &PackageInfo, indent: usize) -> String {
        let mut out = String::new();
        let pad = "  ".repeat(indent);
        let _ =
            writeln!(out, "{pad}{} {} : {} = '{}'", self.blue("package"), self.bold(&p.name), p.ns_prefix, p.ns_uri);
        for c in &p.classifiers {
            let keyword = match c.kind {
                ClassifierKind::Class if c.is_interface => "interface",
                ClassifierKind::Class if c.is_abstract => "abstract class",
                ClassifierKind::Class => "class",
                ClassifierKind::DataType => "datatype",
                ClassifierKind::Enum => "enum",
            };
            let supers =
                if c.super_types.is_empty() { String::new() } else { format!(" extends {}", c.super_types.join(", ")) };
            let _ = writeln!(out, "{pad}  {} {}{supers}", self.blue(keyword), self.bold(&c.name));
            for l in &c.literals {
                let _ = writeln!(out, "{pad}    literal {l}");
            }
            for f in &c.features {
                let kind = if f.containment {
                    "property (containment)"
                } else if f.is_reference {
                    "property"
                } else {
                    "attribute"
                };
                let bounds = match (f.lower, f.upper) {
                    (0, 1) => String::new(),
                    (0, -1 | -2) => " [*]".to_string(),
                    (1, -1 | -2) => " [+]".to_string(),
                    (1, 1) => " [1]".to_string(),
                    (l, -1 | -2) => format!(" [{l}..*]"),
                    (l, u) => format!(" [{l}..{u}]"),
                };
                let opposite = f.opposite.as_ref().map(|o| format!(" #{o}")).unwrap_or_default();
                let derived = if f.derived { " (derived)" } else { "" };
                let _ = writeln!(out, "{pad}    {} {} : {}{bounds}{opposite}{derived}", self.dim(kind), f.name, f.ty);
                if let Some(d) = &f.derivation {
                    let _ = writeln!(out, "{pad}      {} {}", self.dim("derivation:"), d);
                }
            }
            for o in &c.operations {
                let params: Vec<String> = o.parameters.iter().map(|p| format!("{} : {}", p.name, p.ty)).collect();
                let _ =
                    writeln!(out, "{pad}    {} {}({}) : {}", self.dim("operation"), o.name, params.join(", "), o.ty);
                for (keyword, conditions) in [("precondition", &o.preconditions), ("postcondition", &o.postconditions)]
                {
                    for c in conditions {
                        let name = if c.name.is_empty() { String::new() } else { format!(" {}", c.name) };
                        let _ = writeln!(out, "{pad}      {}{name}: {}", self.dim(keyword), c.expression);
                    }
                }
                if let Some(b) = &o.body {
                    let _ = writeln!(out, "{pad}      {} {}", self.dim("body:"), b);
                }
            }
            for i in &c.invariants {
                let _ = writeln!(out, "{pad}    {} {}: {}", self.dim("invariant"), i.name, i.expression);
            }
        }
        for s in &p.subpackages {
            out.push_str(&self.package(s, indent + 1));
        }
        out
    }
}

/// `pkg::Class::Name` for invariants, `pkg::Class::op(...) postcondition Name` for contracts.
fn constraint_title(c: &ConstraintInfo) -> String {
    match c.kind {
        ConstraintKind::Invariant => format!("{}::{}", c.context, c.name),
        kind => {
            let keyword = if kind == ConstraintKind::Precondition { "precondition" } else { "postcondition" };
            let name = if c.name == "<anonymous>" { String::new() } else { format!(" {}", c.name) };
            let op = c.operation.as_deref().unwrap_or("?");
            // Without the result type: `mayEnrol(course : Course)`.
            let op = op.rfind(')').map_or(op, |i| &op[..=i]);
            format!("{}::{op} {keyword}{name}", c.context)
        }
    }
}

fn location(c: &ConstraintInfo) -> String {
    match &c.origin {
        Origin::Ecore { resource, .. } => file_name(resource).to_string(),
        _ => format!("{}:{}", c.source, c.range.start.line + 1),
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}
