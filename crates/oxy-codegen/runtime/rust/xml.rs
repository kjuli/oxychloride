//! A small XML reader and escaping, enough for XMI documents. Copied verbatim into every
//! generated crate; do not edit.

#![allow(dead_code)]

use std::collections::HashMap;

/// An element: its qualified name, attributes (in document order), children and text.
#[derive(Clone, Debug, Default)]
pub struct Element {
    pub name: String,
    pub attributes: Vec<(String, String)>,
    pub children: Vec<Element>,
    pub text: String,
}

impl Element {
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    /// The local part of the name (`library:Library` → `Library`).
    pub fn local_name(&self) -> &str {
        self.name.rsplit(':').next().unwrap_or(&self.name)
    }

    /// The prefix of the name, if any.
    pub fn prefix(&self) -> Option<&str> {
        self.name.split_once(':').map(|(p, _)| p)
    }
}

/// A parsed document: the root element and the namespace prefixes declared anywhere in it.
#[derive(Clone, Debug)]
pub struct Document {
    pub root: Element,
    pub namespaces: HashMap<String, String>,
}

/// Parses a document.
pub fn parse(text: &str) -> Result<Document, String> {
    let mut p = Parser { s: text.as_bytes(), text, pos: 0, namespaces: HashMap::new() };
    p.skip_prolog()?;
    let root = p.element()?;
    Ok(Document { root, namespaces: p.namespaces })
}

struct Parser<'a> {
    s: &'a [u8],
    text: &'a str,
    pos: usize,
    namespaces: HashMap<String, String>,
}

impl Parser<'_> {
    fn error<T>(&self, message: &str) -> Result<T, String> {
        let line = self.text[..self.pos.min(self.text.len())].matches('\n').count() + 1;
        Err(format!("line {line}: {message}"))
    }

    fn starts_with(&self, prefix: &str) -> bool {
        self.s[self.pos..].starts_with(prefix.as_bytes())
    }

    fn skip_ws(&mut self) {
        while self.pos < self.s.len() && self.s[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn skip_until(&mut self, end: &str) -> Result<(), String> {
        match self.text[self.pos..].find(end) {
            Some(i) => {
                self.pos += i + end.len();
                Ok(())
            }
            None => self.error(&format!("missing '{end}'")),
        }
    }

    /// Skips the XML declaration, processing instructions, comments and a doctype.
    fn skip_prolog(&mut self) -> Result<(), String> {
        loop {
            self.skip_ws();
            if self.starts_with("<?") {
                self.skip_until("?>")?;
            } else if self.starts_with("<!--") {
                self.skip_until("-->")?;
            } else if self.starts_with("<!") {
                self.skip_until(">")?;
            } else {
                return Ok(());
            }
        }
    }

    fn name(&mut self) -> Result<String, String> {
        let start = self.pos;
        while self.pos < self.s.len() {
            let c = self.s[self.pos];
            if c.is_ascii_whitespace() || matches!(c, b'=' | b'>' | b'/' | b'<') {
                break;
            }
            self.pos += 1;
        }
        if start == self.pos {
            return self.error("expected a name");
        }
        Ok(self.text[start..self.pos].to_string())
    }

    fn element(&mut self) -> Result<Element, String> {
        if !self.starts_with("<") {
            return self.error("expected an element");
        }
        self.pos += 1;
        let mut e = Element { name: self.name()?, ..Element::default() };
        loop {
            self.skip_ws();
            if self.starts_with("/>") {
                self.pos += 2;
                return Ok(e);
            }
            if self.starts_with(">") {
                self.pos += 1;
                break;
            }
            let name = self.name()?;
            self.skip_ws();
            if !self.starts_with("=") {
                return self.error("expected '='");
            }
            self.pos += 1;
            self.skip_ws();
            let quote = match self.s.get(self.pos) {
                Some(b'"') => "\"",
                Some(b'\'') => "'",
                _ => return self.error("expected a quoted value"),
            };
            self.pos += 1;
            let end = match self.text[self.pos..].find(quote) {
                Some(i) => self.pos + i,
                None => return self.error("unterminated attribute value"),
            };
            let value = unescape(&self.text[self.pos..end]);
            self.pos = end + 1;
            if let Some(prefix) = name.strip_prefix("xmlns:") {
                self.namespaces.entry(prefix.to_string()).or_insert_with(|| value.clone());
            } else if name == "xmlns" {
                self.namespaces.entry(String::new()).or_insert_with(|| value.clone());
            }
            e.attributes.push((name, value));
        }
        // Content.
        loop {
            if self.pos >= self.s.len() {
                return self.error(&format!("unclosed element '{}'", e.name));
            }
            if self.starts_with("</") {
                self.pos += 2;
                let name = self.name()?;
                if name != e.name {
                    return self.error(&format!("'</{name}>' does not close '<{}>'", e.name));
                }
                self.skip_ws();
                if !self.starts_with(">") {
                    return self.error("expected '>'");
                }
                self.pos += 1;
                return Ok(e);
            }
            if self.starts_with("<!--") {
                self.skip_until("-->")?;
            } else if self.starts_with("<![CDATA[") {
                let start = self.pos + 9;
                self.skip_until("]]>")?;
                e.text.push_str(&self.text[start..self.pos - 3]);
            } else if self.starts_with("<?") {
                self.skip_until("?>")?;
            } else if self.starts_with("<") {
                let child = self.element()?;
                e.children.push(child);
            } else {
                let start = self.pos;
                while self.pos < self.s.len() && self.s[self.pos] != b'<' {
                    self.pos += 1;
                }
                e.text.push_str(&unescape(&self.text[start..self.pos]));
            }
        }
    }
}

/// Replaces entity and character references.
pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';') else { break };
        let entity = &rest[1..end];
        let c = match entity {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Escapes text for an attribute value (or element content).
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
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
