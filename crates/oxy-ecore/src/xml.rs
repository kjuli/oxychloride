//! A small namespace-aware XML DOM on top of `quick-xml`.
//!
//! Both `.ecore` files and XMI instance models are loaded in two passes (build objects,
//! then resolve references), which is much simpler on a tree than on an event stream.

use std::collections::HashMap;
use std::rc::Rc;

use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};

use crate::error::LoadError;

pub const XMI_NS: &str = "http://www.omg.org/XMI";
pub const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";

#[derive(Debug, Clone)]
pub struct Element {
    /// Qualified name as written, e.g. `ecore:EPackage` or `eClassifiers`.
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Element>,
    pub text: String,
    /// Byte offset of the start tag in the source.
    pub offset: usize,
    /// Namespace prefixes in scope for this element (`""` is the default namespace).
    pub namespaces: Rc<HashMap<String, String>>,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// Looks up an attribute by namespace URI and local name (e.g. `xsi:type`).
    pub fn attr_ns(&self, ns: &str, local: &str) -> Option<&str> {
        self.attrs.iter().find_map(|(k, v)| {
            let (prefix, name) = split_qname(k);
            (name == local && prefix.is_some_and(|p| self.namespaces.get(p).is_some_and(|u| u == ns)))
                .then_some(v.as_str())
        })
    }

    pub fn local_name(&self) -> &str {
        split_qname(&self.name).1
    }

    pub fn prefix(&self) -> Option<&str> {
        split_qname(&self.name).0
    }

    /// The namespace URI of the element itself.
    pub fn namespace(&self) -> Option<&str> {
        self.namespaces.get(self.prefix().unwrap_or("")).map(String::as_str)
    }

    /// Resolves a `prefix:Name` QName value (as found in `xsi:type`) to `(nsURI, Name)`.
    pub fn resolve_qname_value<'a>(&'a self, value: &'a str) -> (Option<&'a str>, &'a str) {
        let (prefix, local) = split_qname(value);
        (self.namespaces.get(prefix.unwrap_or("")).map(String::as_str), local)
    }

    pub fn children_named<'a>(&'a self, local: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.children.iter().filter(move |c| c.local_name() == local)
    }

    pub fn is_xmlns_attr(name: &str) -> bool {
        name == "xmlns" || name.starts_with("xmlns:")
    }
}

pub fn split_qname(name: &str) -> (Option<&str>, &str) {
    match name.split_once(':') {
        Some((p, l)) => (Some(p), l),
        None => (None, name),
    }
}

/// Parses `text` into a DOM and returns the root element.
pub fn parse(text: &str, path: &str) -> Result<Element, LoadError> {
    let mut reader = quick_xml::Reader::from_str(text);
    reader.config_mut().trim_text(false);
    let mut stack: Vec<Element> = Vec::new();
    let mut root: Option<Element> = None;
    let root_ns: Rc<HashMap<String, String>> = Rc::new(HashMap::new());

    let err = |offset: usize, msg: String| LoadError::at(path, text, offset, msg);

    loop {
        let offset = reader.buffer_position() as usize;
        let event =
            reader.read_event().map_err(|e| err(reader.error_position() as usize, format!("malformed XML: {e}")))?;
        match event {
            Event::Start(start) => {
                let parent_ns = stack.last().map(|e| e.namespaces.clone()).unwrap_or_else(|| root_ns.clone());
                let el = start_element(&start, offset, parent_ns).map_err(|m| err(offset, m))?;
                stack.push(el);
            }
            Event::Empty(start) => {
                let parent_ns = stack.last().map(|e| e.namespaces.clone()).unwrap_or_else(|| root_ns.clone());
                let el = start_element(&start, offset, parent_ns).map_err(|m| err(offset, m))?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(el),
                    None => root = Some(el),
                }
            }
            Event::End(_) => {
                let el = stack.pop().ok_or_else(|| err(offset, "unexpected end tag".into()))?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(el),
                    None => root = Some(el),
                }
            }
            Event::Text(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t.xml10_content());
                }
            }
            Event::CData(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t.into_inner());
                }
            }
            Event::GeneralRef(r) => {
                if let Some(top) = stack.last_mut() {
                    if let Some(ch) = r.resolve_char_ref().map_err(|e| err(offset, e.to_string()))? {
                        top.text.push(ch);
                    } else {
                        let name = r.into_inner();
                        let replacement = match name.as_ref() {
                            "lt" => "<",
                            "gt" => ">",
                            "amp" => "&",
                            "apos" => "'",
                            "quot" => "\"",
                            other => return Err(err(offset, format!("unknown entity '&{other};'"))),
                        };
                        top.text.push_str(replacement);
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err(err(text.len(), "unexpected end of document (unclosed element)".into()));
    }
    root.ok_or_else(|| err(0, "document has no root element".into()))
}

fn start_element(
    start: &BytesStart<'_>,
    offset: usize,
    parent_ns: Rc<HashMap<String, String>>,
) -> Result<Element, String> {
    let name = start.name().as_ref().to_string();
    let mut attrs = Vec::new();
    let mut new_ns: Option<HashMap<String, String>> = None;
    for attr in start.attributes() {
        let attr = attr.map_err(|e| format!("malformed attribute: {e}"))?;
        let key = attr.key.as_ref().to_string();
        let value = attr
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|e| format!("malformed attribute value: {e}"))?
            .into_owned();
        if Element::is_xmlns_attr(&key) {
            let prefix = key.strip_prefix("xmlns:").unwrap_or("").to_string();
            new_ns.get_or_insert_with(|| (*parent_ns).clone()).insert(prefix, value.clone());
        }
        attrs.push((key, value));
    }
    Ok(Element {
        name,
        attrs,
        children: Vec::new(),
        text: String::new(),
        offset,
        namespaces: new_ns.map(Rc::new).unwrap_or(parent_ns),
    })
}

/// Escapes a string for use in a double-quoted attribute value. Line breaks and tabs are
/// written as character references so that attribute normalization preserves them.
pub fn escape_attr(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            '\t' => out.push_str("&#x9;"),
            c => out.push(c),
        }
    }
    out
}

/// Escapes element text content.
pub fn escape_text(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Converts a byte offset into a 1-based (line, column) pair.
pub fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let col = before.rfind('\n').map_or(offset, |i| offset - i - 1) + 1;
    (line, col)
}
