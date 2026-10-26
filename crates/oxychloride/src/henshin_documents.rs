//! Henshin transformations in their two syntaxes: Henshin's textual syntax (`.henshin_text`)
//! and Henshin's model (`.henshin`, the XMI Henshin's graphical and tree editors write).
//!
//! A document of either syntax compiles to the same kind of [`Module`] — a `.henshin` file is
//! read into the syntax tree of the textual syntax ([`oxy_henshin::xmi::read`]) — so the
//! engine runs, checks and shows both alike. The two are converted into each other
//! ([`Workspace::export_document`]), and a pair of them (`rules.henshin_text` and
//! `rules.henshin`) is kept equivalent: [`Workspace::sync_transformation`] computes the small
//! edits that make one document mean what the other means:
//!
//! * towards a `.henshin` file, it is written again from the compiled text (keeping its ids
//!   and what the text cannot express) and compared line by line;
//! * towards a text, the rules and units that still mean the same keep their text — layout,
//!   comments and all — and only the changed ones are written anew.

use std::collections::HashMap;
use std::path::Path;

use oxy_ecore::Problem;
use oxy_henshin::Module;
use oxy_henshin::print::Printer;
use oxy_henshin::syntax::{self, Document};
use oxy_henshin::xmi::{self, Template, WriteOptions};

use crate::diagram::indent_unit;
use crate::protocol::*;
use crate::textdiff::line_replacements;
use crate::workspace::{Error, Workspace};

/// A transformation document as a syntax tree.
pub(crate) struct Syntax {
    pub(crate) document: Document,
    /// Problems of parsing (or reading) the document.
    pub(crate) problems: Vec<Problem>,
    /// For `.henshin` files: what writing the file again keeps.
    pub(crate) template: Option<Template>,
}

/// The syntax tree of a transformation in either syntax.
pub(crate) fn syntax(mm: &oxy_ecore::Metamodel, name: &str, text: &str, language: Language) -> Syntax {
    if language == Language::HenshinModel {
        let read = xmi::read(mm, text, name);
        Syntax { document: read.document, problems: read.problems, template: Some(read.template) }
    } else {
        let (document, problems) = syntax::parse(text);
        Syntax { document, problems, template: None }
    }
}

/// The text of a transformation in the textual syntax, indented like `like` (a tab if it does
/// not tell).
pub(crate) fn print(doc: &Document, like: &str) -> String {
    let indent = if like.lines().any(|l| l.starts_with([' ', '\t'])) { indent_unit(like) } else { "\t".to_string() };
    Printer::with_indent(indent).document(doc)
}

/// The first error of a compiled module, as a message.
fn first_error(module: &Module, text: &str) -> Option<String> {
    module.problems.iter().find(|p| p.is_error()).map(|p| {
        let line = text[..p.span.start.min(text.len())].matches('\n').count() + 1;
        format!("line {line}: {}", p.message)
    })
}

impl Workspace {
    /// The syntax tree of an open transformation document.
    pub(crate) fn transformation_syntax(&self, name: &str) -> Result<(Syntax, Language), Error> {
        let text =
            self.document_text(name).ok_or_else(|| Error::NotFound(format!("no open document named '{name}'")))?;
        let language = self.document_language(name).filter(|l| l.is_transformation());
        let language = language.ok_or_else(|| Error::Invalid(format!("'{name}' is not a Henshin document")))?;
        Ok((syntax(self.metamodel(), name, text, language), language))
    }

    /// Compiles a transformation of either syntax.
    pub(crate) fn compile_transformation(
        &self,
        name: &str,
        text: &str,
        language: Language,
    ) -> (Module, Option<Template>) {
        let s = syntax(self.metamodel(), name, text, language);
        (Module::from_document(self.metamodel(), &s.document, s.problems), s.template)
    }

    /// A transformation document in the other syntax: a `.henshin` file for a text (written for
    /// `target`, to which references to `.ecore` files are relative), the text of a `.henshin`
    /// file.
    pub(crate) fn convert_transformation(&self, name: &str, target: Option<&str>) -> Result<ExportResult, Error> {
        let text =
            self.document_text(name).ok_or_else(|| Error::NotFound(format!("no open document named '{name}'")))?;
        let language = self.document_language(name).unwrap_or(Language::Henshin);
        match language {
            Language::Henshin => {
                let target = target.map(str::to_string).unwrap_or_else(|| counterpart(name));
                Ok(ExportResult { text: self.write_model(name, text, &target, None)?, format: "henshin".into() })
            }
            _ => {
                let s = syntax(self.metamodel(), name, text, language);
                Ok(ExportResult { text: print(&s.document, ""), format: "henshin_text".into() })
            }
        }
    }

    /// The `.henshin` file of a text (which must have no errors).
    fn write_model(&self, name: &str, text: &str, target: &str, template: Option<&Template>) -> Result<String, Error> {
        let mm = self.metamodel();
        let module = Module::compile(mm, text);
        if let Some(e) = first_error(&module, text) {
            return Err(Error::Invalid(format!("{name} has errors ({e}); fix them first")));
        }
        Ok(xmi::write(mm, &module, &WriteOptions { path: target, name: None, template }))
    }

    /// The edits that make the transformation document `target` (of the other syntax, with
    /// the text `target_text`; it need not be open) mean what the open document `source`
    /// means. Fails while `source` has errors.
    pub fn sync_transformation(&self, source: &str, target: &str, target_text: &str) -> Result<Vec<TextEdit>, Error> {
        let text =
            self.document_text(source).ok_or_else(|| Error::NotFound(format!("no open document named '{source}'")))?;
        let language = self.document_language(source).filter(|l| l.is_transformation());
        let language = language.ok_or_else(|| Error::Invalid(format!("'{source}' is not a Henshin document")))?;
        let target_language = Language::from_path(target);
        if !target_language.is_transformation() || target_language == language {
            return Err(Error::Invalid(format!("'{target}' is not a Henshin document of the other syntax")));
        }
        let mm = self.metamodel();
        let new_text = if language == Language::Henshin {
            let previous = xmi::read(mm, target_text, target);
            let template = (!previous.template.is_empty()).then_some(&previous.template);
            self.write_model(source, text, target, template)?
        } else {
            let s = syntax(mm, source, text, language);
            let module = Module::from_document(mm, &s.document, s.problems);
            if let Some(e) = first_error(&module, text) {
                return Err(Error::Invalid(format!("{source} has errors ({e}); fix them first")));
            }
            merge_text(mm, target_text, &s.document)
        };
        Ok(crate::diagram::replacements_to_edits(target_text, line_replacements(target_text, &new_text)))
    }

    /// Converts a transformation file to the other syntax (`to`), with the workspace's
    /// metamodels and the `.ecore` files a `.henshin` file refers to.
    pub(crate) fn convert_transformation_file(&self, path: &str, to: Language) -> Result<String, Error> {
        let text = std::fs::read_to_string(path).map_err(|e| Error::NotFound(format!("cannot read '{path}': {e}")))?;
        let from = Language::from_path(path);
        if !from.is_transformation() || from == to {
            return Err(Error::Invalid(format!("{path} is not a Henshin document of the other syntax")));
        }
        let mut ws = self.clone();
        let name =
            Path::new(path).canonicalize().map_or_else(|_| path.to_string(), |p| p.to_string_lossy().into_owned());
        ws.open_document(&name, &text);
        let target = counterpart(&name);
        Ok(ws.convert_transformation(&name, Some(&target))?.text)
    }

    /// Loads the `.ecore` files a `.henshin` file refers to (relative to it) whose packages are
    /// not loaded yet.
    pub(crate) fn load_henshin_imports(&mut self, name: &str, text: &str) {
        let Ok(root) = oxy_ecore::xml::parse(text, name) else { return };
        let mut files = Vec::new();
        collect_ecore_files(&root, &mut files);
        let dir = Path::new(name).parent().map(Path::to_path_buf).unwrap_or_default();
        for f in files {
            let path = dir.join(&f);
            let Ok(path) = path.canonicalize() else { continue };
            let loaded = self.metamodel().resources().iter().any(|r| Path::new(&r.uri) == path);
            if loaded || !path.is_file() {
                continue;
            }
            // A package with the file's nsURI may be loaded from elsewhere (an OCLinEcore
            // document).
            let ns_uri = std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| oxy_ecore::xml::parse(&t, &f).ok())
                .and_then(|e| e.attr("nsURI").map(str::to_string));
            if ns_uri.is_some_and(|ns| self.metamodel().package_by_ns_uri(&ns).is_some()) {
                continue;
            }
            let _ = self.load_ecore_file(&path);
        }
    }
}

/// The `.ecore` files that `href`s in a `.henshin` file name.
fn collect_ecore_files(e: &oxy_ecore::xml::Element, out: &mut Vec<String>) {
    if let Some(href) = e.attr("href")
        && let Some((file, _)) = href.split_once('#')
        && file.to_ascii_lowercase().ends_with(".ecore")
        && !file.contains("://")
        && !out.iter().any(|f| f == file)
    {
        out.push(file.to_string());
    }
    for c in &e.children {
        collect_ecore_files(c, out);
    }
}

/// The name of the other syntax's document of a transformation: `x.henshin` for
/// `x.henshin_text` and the other way round.
pub fn counterpart(name: &str) -> String {
    if let Some(stem) = name.strip_suffix(".henshin_text") {
        format!("{stem}.henshin")
    } else if let Some(stem) = name.strip_suffix(".henshin") {
        format!("{stem}.henshin_text")
    } else {
        format!("{name}.henshin")
    }
}

/// What each rule and unit of a module means, as the XML Henshin's model has for it (written
/// once, read and written again, so that equivalent texts give equal XML), by name.
fn meanings(mm: &oxy_ecore::Metamodel, module: &Module) -> HashMap<String, String> {
    let options = WriteOptions { path: "", name: Some("m"), template: None };
    let first = xmi::write(mm, module, &options);
    let read = xmi::read(mm, &first, "");
    let again = Module::from_document(mm, &read.document, read.problems);
    let xml = xmi::write(mm, &again, &options);
    let mut out: HashMap<String, String> = HashMap::new();
    let Ok(root) = oxy_ecore::xml::parse(&xml, "") else { return out };
    let mut owner = String::new();
    for u in root.children_named("units") {
        let nested = u.children_named("annotations").any(|a| a.attr("key") == Some(xmi::NESTED));
        if !nested {
            owner = u.attr("name").unwrap_or("").to_string();
        }
        out.entry(owner.clone()).or_default().push_str(&xml[u.offset..u.end]);
    }
    out
}

/// A text whose rules and units mean what those of `new` mean: the elements of `old` that
/// still mean the same are kept as they are written (with the comments before them), the others
/// are written anew.
fn merge_text(mm: &oxy_ecore::Metamodel, old: &str, new: &Document) -> String {
    let (old_doc, problems) = syntax::parse(old);
    let old_module = Module::from_document(mm, &old_doc, problems);
    if old.trim().is_empty() || old_module.has_errors() {
        return print(new, old);
    }
    let old_meanings = meanings(mm, &old_module);
    let new_module = Module::from_document(mm, new, Vec::new());
    let new_meanings = meanings(mm, &new_module);
    let indent = if old.lines().any(|l| l.starts_with([' ', '\t'])) { indent_unit(old) } else { "\t".to_string() };
    let printer = Printer::with_indent(indent);
    // The text up to the end of the imports.
    let header_end = old_doc.imports.last().map_or(0, |i| line_end(old, i.span.end));
    let same_imports = old_doc.imports.iter().map(|i| i.text()).collect::<Vec<_>>()
        == new.imports.iter().map(|i| i.text()).collect::<Vec<_>>();
    let mut out = if same_imports {
        old[..header_end].to_string()
    } else {
        // Comments before the imports stay.
        let first = old_doc.imports.first().map_or(header_end, |i| line_start(old, i.span.start));
        let mut s = old[..first].to_string();
        for i in &new.imports {
            s.push_str(&format!("ePackageImport {}\n", oxy_henshin::print::import(i)));
        }
        s.trim_end_matches('\n').to_string()
    };
    // Each old element with what precedes it (blank lines, comments).
    let mut segments: HashMap<&str, (&str, &str)> = HashMap::new();
    let mut previous = header_end;
    for e in &old_doc.elements {
        let span = e.span();
        segments.insert(e.name().text.as_str(), (&old[previous..span.start], &old[span.start..span.end]));
        previous = span.end;
    }
    let trailer = &old[previous.max(header_end)..];
    for e in &new.elements {
        let name = e.name().text.as_str();
        let same = old_meanings.get(name).is_some_and(|m| new_meanings.get(name) == Some(m));
        match segments.get(name) {
            Some((before, text)) if same => {
                out.push_str(before);
                out.push_str(text);
            }
            Some((before, _)) => {
                out.push_str(before);
                out.push_str(printer.element(e).trim_end_matches('\n'));
            }
            None => {
                out.push_str("\n\n");
                out.push_str(printer.element(e).trim_end_matches('\n'));
            }
        }
    }
    if trailer.trim().is_empty() {
        out.push('\n');
    } else {
        out.push_str(trailer);
    }
    out
}

fn line_end(text: &str, offset: usize) -> usize {
    text[offset..].find('\n').map_or(text.len(), |i| offset + i)
}

fn line_start(text: &str, offset: usize) -> usize {
    text[..offset].rfind('\n').map_or(0, |i| i + 1)
}
