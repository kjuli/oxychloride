//! `.henshin` files: transformation modules as EMF writes them (XMI of Henshin's metamodel,
//! `http://www.eclipse.org/emf/2011/Henshin`), the format of Henshin's graphical and tree
//! editors.
//!
//! The textual syntax and the model are two notations for the same rules and units:
//!
//! * [`read`] turns a `.henshin` file into the syntax tree of the textual syntax, with spans in
//!   the XML (so problems, hovers and diagram selections point into the file), which compiles
//!   like a text into a [`Module`](crate::Module);
//! * [`write`] writes a compiled module as a `.henshin` file, so the file means exactly what
//!   the engine runs.
//!
//! The mapping follows Henshin's own translation of the textual syntax
//! (`Henshin_text2HenshinTransformation.qvto`): a rule is a left- and a right-hand side graph
//! with mappings between the preserved nodes; `forbid` and `require` elements become a
//! negative and a positive nested condition named `forbid` and `require`, a `matchingFormula`
//! the formula over its condition graphs (all conjoined in the left-hand side's formula);
//! multi-rules contain copies of their kernel's nodes and edges, related by multi-mappings.
//! Units are Henshin's units: a sequence of calls is a sequential unit, `priority [...]` a
//! priority unit and so on. Henshin's units cannot nest, so a nested block of a unit becomes a
//! unit of its own, named after its place (`unit_2`) and marked with the annotation
//! [`NESTED`], which reading inlines again. Attribute values are JavaScript in the model (see
//! [`js`](crate::js)); JavaScript the textual syntax cannot express reads as
//! `javascript("…")`.
//!
//! Writing keeps what it can of an existing file (a [`Template`], from [`read`]): the ids of
//! elements that are still there, and what the textual syntax does not express (descriptions,
//! annotations, the names of attribute conditions, …). Elements are matched by *keys* made of
//! names (`unit:enrol/lhs/node:student`); new elements get ids derived from their keys, so
//! writing the same module twice gives the same file. A rule, unit or node whose name is new
//! where the template has a name that is gone (at the same position) is taken to be renamed,
//! and keeps its ids.

mod read;
mod write;

use std::collections::HashMap;

pub use read::{Read, read};
pub use write::{WriteOptions, write};

/// The namespace of Henshin's metamodel.
pub const HENSHIN_NS: &str = "http://www.eclipse.org/emf/2011/Henshin";

/// The key of the annotation that marks units made for nested blocks of a unit (see the
/// module documentation). Its value is `block` for a unit that stands for a `{ ... }` block,
/// `inline` for one whose elements belong to the enclosing unit.
pub const NESTED: &str = "henshin_text:nested";

/// What [`write`] keeps of an existing file: ids, and content the textual syntax cannot
/// express, by element key.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Template {
    pub(crate) ids: HashMap<String, String>,
    pub(crate) extras: HashMap<String, Extra>,
    /// The names of the rules and units, in order (for recognizing renamed ones).
    pub(crate) units: Vec<String>,
    /// The names of each rule's nodes (left-hand side, then created), in order.
    pub(crate) nodes: HashMap<String, Vec<String>>,
}

/// The parts of an element that writing does not produce.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Extra {
    /// XML attributes, in their order.
    pub(crate) attributes: Vec<(String, String)>,
    /// Child elements, as written in the file.
    pub(crate) children: Vec<String>,
    /// The element's name in the file, where the text calls it otherwise (the file's name is
    /// not a name of the textual syntax, or the element has none: `Some(None)`).
    pub(crate) name: Option<Option<String>>,
}

impl Template {
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

/// Hands out keys: the first use of a key is the key itself, repeated ones get `#2`, `#3`, ….
#[derive(Default)]
pub(crate) struct Keys(HashMap<String, usize>);

impl Keys {
    pub(crate) fn next(&mut self, base: String) -> String {
        let n = self.0.entry(base.clone()).or_insert(0);
        *n += 1;
        if *n == 1 { base } else { format!("{base}#{n}") }
    }
}

/// The key of a unit or rule.
pub(crate) fn unit_key(name: &str) -> String {
    format!("unit:{name}")
}

/// A stable id derived from a key: `_` and the base64 of a 128-bit FNV-1a hash (the shape of
/// the UUIDs EMF writes).
pub(crate) fn derived_id(key: &str) -> String {
    const PRIME: u128 = 0x0000000001000000000000000000013B;
    let mut h: u128 = 0x6c62272e07bb014262b821756295c58d;
    for b in key.bytes() {
        h ^= b as u128;
        h = h.wrapping_mul(PRIME);
    }
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let bytes = h.to_be_bytes();
    let mut out = String::from("_");
    let mut acc: u32 = 0;
    let mut bits = 0;
    for b in bytes {
        acc = (acc << 8) | b as u32;
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            out.push(ALPHABET[((acc >> bits) & 63) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((acc << (6 - bits)) & 63) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests;
