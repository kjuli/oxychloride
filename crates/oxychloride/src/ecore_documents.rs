//! `.ecore` documents: metamodels as EMF writes them, edited as XML ([`Language::Ecore`]).
//!
//! An open `.ecore` document stands for its file: its text is an overlay that the locator reads
//! instead of the file, so the workspace loads it — with the other metamodels, again on every
//! change — wherever the file is used: registered, a package of an Eclipse project, referred to
//! by other files, or else because it is open. Its diagnostics are the problems loading it and
//! those of the OCL embedded in its annotations (as Eclipse OCL stores it: `<details
//! key="name" value="ocl"/>` under `http://www.eclipse.org/emf/2002/Ecore/OCL…`), located in
//! the `value` of their `<details>` — through XML's escapes. The OCL of `.ecore` files that are
//! not open is located in the files the same way (the `location` of [`Origin::Ecore`]).

use std::collections::HashMap;
use std::ops::Range as ByteRange;
use std::path::Path;
use std::sync::Arc;

use oxy_ecore::ecore_loader::canonical_uri;
use oxy_ocl::LineIndex;
use oxy_ocl::env::{Source, SourceKind};

use crate::protocol::*;
use crate::workspace::Workspace;

/// The URI of the file an `.ecore` document stands for.
pub(crate) fn ecore_document_uri(name: &str) -> String {
    canonical_uri(Path::new(name))
}

/// Texts of `.ecore` resources and their line indexes, read once per request.
#[derive(Default)]
pub(crate) struct ResourceTexts {
    texts: HashMap<String, Option<(String, LineIndex)>>,
}

impl Workspace {
    /// Opens (or changes) an `.ecore` document, or a document that was one.
    pub(crate) fn open_ecore_document(&mut self, name: &str, text: &str, language: Language) -> Vec<Diagnostic> {
        self.insert_document(name, text, language);
        let uri = ecore_document_uri(name);
        let overlays = Arc::make_mut(&mut self.overlays);
        if language == Language::Ecore {
            overlays.insert(uri, text.to_string());
        } else {
            overlays.remove(&uri);
        }
        self.reload();
        self.diagnostics(Some(name))
    }

    /// Loads the open `.ecore` documents that nothing else loaded (the registry, the Eclipse
    /// projects, the files referring to them).
    pub(crate) fn load_ecore_documents(&mut self) {
        let names: Vec<String> = self.documents_of(Language::Ecore);
        for name in names {
            let uri = ecore_document_uri(&name);
            if self.base_mm.resource_of_uri(&uri).is_some() {
                continue;
            }
            if let Err(e) = self.load_metamodel_file(Path::new(&uri)) {
                self.ecore_errors.insert(name, e);
            }
        }
    }

    /// The open `.ecore` document standing for a resource, if any.
    pub(crate) fn ecore_document_of(&self, resource: &str) -> Option<&str> {
        self.document_names()
            .find(|n| self.document_language(n) == Some(Language::Ecore) && ecore_document_uri(n) == resource)
    }

    /// The problems loading an `.ecore` document: why it does not load, and the references in
    /// it that cannot be resolved.
    pub(crate) fn ecore_document_problems(&self, name: &str) -> Vec<Diagnostic> {
        let uri = ecore_document_uri(name);
        let error = self.ecore_errors.get(name).map(|e| (Severity::Error, e));
        let warnings = self.load_warnings.get(&uri).into_iter().flatten().map(|w| (Severity::Warning, w));
        error
            .into_iter()
            .chain(warnings)
            .map(|(severity, e)| {
                let p =
                    e.location.map(|(l, c)| Position { line: l as u32 - 1, column: c as u32 - 1 }).unwrap_or_default();
                Diagnostic {
                    severity,
                    message: e.message.clone(),
                    source: name.to_string(),
                    origin: Origin::Document { name: name.to_string() },
                    range: Range { start: p, end: p },
                }
            })
            .collect()
    }

    /// Problems loading the `.ecore` files of the Eclipse projects that are not open as
    /// documents (those are reported for the documents).
    pub(crate) fn project_load_problems(&self) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        let mut files: Vec<&String> = self.load_warnings.keys().collect();
        files.sort();
        for file in files {
            if self.ecore_document_of(file).is_some() || self.platform.logical_uris(file).is_empty() {
                continue;
            }
            out.extend(self.load_warnings[file].iter().map(crate::workspace::load_diagnostic));
        }
        out
    }

    /// Locates a diagnostic of OCL embedded in an `.ecore` file in the file: in the open
    /// document standing for it, else as the `location` of its [`Origin::Ecore`].
    pub(crate) fn locate_embedded(
        &self,
        mut d: Diagnostic,
        source: &Source,
        span: oxy_ocl::Span,
        texts: &mut ResourceTexts,
    ) -> Diagnostic {
        let SourceKind::Annotation { resource, offset: Some(tag), .. } = &source.kind else { return d };
        let document = self.ecore_document_of(resource);
        let Some((text, index)) = texts.texts.entry(resource.clone()).or_insert_with(|| {
            let text = match document {
                Some(name) => self.document_text(name).map(str::to_string),
                None => self.locator().read(resource),
            }?;
            let index = LineIndex::new(&text);
            Some((text, index))
        }) else {
            return d;
        };
        let Some(bytes) = value_range(text, *tag, span.start as usize..span.end as usize) else { return d };
        let p = |o: usize| {
            let p = index.position(text, o);
            Position { line: p.line, column: p.column }
        };
        let range = Range { start: p(bytes.start), end: p(bytes.end) };
        match document {
            Some(name) => {
                d.origin = Origin::Document { name: name.to_string() };
                d.source = name.to_string();
                d.range = range;
            }
            None => {
                if let Origin::Ecore { location, .. } = &mut d.origin {
                    *location = Some(range);
                }
            }
        }
        d
    }
}

impl ResourceTexts {
    pub(crate) fn new() -> Self {
        ResourceTexts::default()
    }
}

/// Where a part (`span`, byte offsets into the unescaped value) of the `value` of the XML tag
/// at `tag` is in `text`: XML's escapes (`&lt;`, `&#xA;`, …) are as long as they are written.
pub(crate) fn value_range(text: &str, tag: usize, span: ByteRange<usize>) -> Option<ByteRange<usize>> {
    let offsets = value_offsets(text, tag)?;
    let raw = |d: usize| offsets.iter().find(|(decoded, _)| *decoded >= d).or(offsets.last()).map(|(_, r)| *r);
    let (from, to) = (raw(span.start)?, raw(span.end)?);
    Some(from..to.max(from))
}

/// The unescaped value's offset at a place in the written `value` of the tag at `tag`, if the
/// place is in it.
pub(crate) fn value_offset(text: &str, tag: usize, raw: usize) -> Option<usize> {
    let offsets = value_offsets(text, tag)?;
    let (first, last) = (offsets.first()?.1, offsets.last()?.1);
    if raw < first || raw > last {
        return None;
    }
    offsets.iter().rev().find(|(_, r)| *r <= raw).map(|(d, _)| *d)
}

/// The `value` of the XML tag at `tag`: pairs of offsets into the unescaped value and into
/// `text`, at the start of every character and at the end.
fn value_offsets(text: &str, tag: usize) -> Option<Vec<(usize, usize)>> {
    let bytes = text.as_bytes();
    let mut i = tag + 1;
    // The attributes of the tag: `name = "value"`.
    let (start, quote) = loop {
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            if matches!(bytes[i], b'>' | b'/') {
                return None;
            }
            i += 1;
        }
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let name_start = i;
        while i < bytes.len() && !matches!(bytes[i], b'=' | b'>' | b'/') && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let name = &text[name_start..i];
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'=') {
            i += 1;
        }
        let quote = *bytes.get(i).filter(|q| matches!(q, b'"' | b'\''))?;
        if name == "value" {
            break (i + 1, quote);
        }
        i += 1;
        while i < bytes.len() && bytes[i] != quote {
            i += 1;
        }
        i += 1;
    };
    let (mut raw, mut decoded) = (start, 0usize);
    let mut out = Vec::new();
    loop {
        out.push((decoded, raw));
        let c = *bytes.get(raw)?;
        if c == quote {
            return Some(out);
        }
        if c == b'&' {
            let end = text[raw..].find(';').map(|e| raw + e)?;
            let ch = match &text[raw + 1..end] {
                "lt" => '<',
                "gt" => '>',
                "amp" => '&',
                "quot" => '"',
                "apos" => '\'',
                e => {
                    let code = match e.strip_prefix("#x").or_else(|| e.strip_prefix("#X")) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok(),
                        None => e.strip_prefix('#').and_then(|d| d.parse().ok()),
                    };
                    code.and_then(char::from_u32).unwrap_or('?')
                }
            };
            decoded += ch.len_utf8();
            raw = end + 1;
        } else {
            let ch = text[raw..].chars().next()?;
            decoded += ch.len_utf8();
            raw += ch.len_utf8();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locates_values_through_escapes() {
        let text = r#"<x><details key="k" value="a &lt;&gt; b&#xA;and c"/></x>"#;
        let tag = text.find("<details").unwrap();
        let r = value_range(text, tag, 2..4).unwrap();
        assert_eq!(&text[r], "&lt;&gt;");
        let r = value_range(text, tag, 7..12).unwrap();
        assert_eq!(&text[r], "and c");
        let r = value_range(text, tag, 0..1).unwrap();
        assert_eq!(&text[r], "a");
        let text = "<details value='x' key=\"k\"/>";
        assert_eq!(value_range(text, 0, 0..1).map(|r| &text[r]), Some("x"));
        assert_eq!(value_range("<details key=\"k\"/>", 0, 0..1), None);
        let text = r#"<details key="k" value="a&lt;b"/>"#;
        let at = text.find("b\"").unwrap();
        assert_eq!(value_offset(text, 0, at), Some(2));
        assert_eq!(value_offset(text, 0, at - 2), Some(1));
        assert_eq!(value_offset(text, 0, 3), None);
    }
}
