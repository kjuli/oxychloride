//! Identifier conventions shared by the targets.

use std::collections::HashSet;

/// Splits an identifier into lower-case words: `loanCount` → `loan`, `count`;
/// `nsURI` → `ns`, `uri`; `XMLParser2` → `xml`, `parser2`; `SCIENCE_FICTION` → `science`,
/// `fiction`.
pub fn words(name: &str) -> Vec<String> {
    let chars: Vec<char> = name.chars().collect();
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
            continue;
        }
        let prev = i.checked_sub(1).map(|j| chars[j]);
        let next = chars.get(i + 1).copied();
        let boundary = c.is_uppercase()
            && !cur.is_empty()
            && (prev.is_some_and(|p| p.is_lowercase() || p.is_ascii_digit())
                || (prev.is_some_and(char::is_uppercase) && next.is_some_and(char::is_lowercase)));
        if boundary {
            words.push(std::mem::take(&mut cur));
        }
        cur.extend(c.to_lowercase());
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
}

/// `loanCount` → `loan_count`.
pub fn snake_case(name: &str) -> String {
    let s = words(name).join("_");
    if s.is_empty() {
        "_".into()
    } else if s.starts_with(|c: char| c.is_ascii_digit()) {
        format!("_{s}")
    } else {
        s
    }
}

/// `loanCount` → `LOAN_COUNT`.
pub fn upper_snake_case(name: &str) -> String {
    snake_case(name).to_uppercase()
}

/// `SCIENCE_FICTION` → `ScienceFiction`, `book` → `Book`.
pub fn pascal_case(name: &str) -> String {
    let s: String = words(name)
        .iter()
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
        })
        .collect();
    if s.is_empty() {
        "X".into()
    } else if s.starts_with(|c: char| c.is_ascii_digit()) {
        format!("X{s}")
    } else {
        s
    }
}

/// Makes names unique within a scope by appending `_2`, `_3`, … to repeated ones.
#[derive(Default, Debug)]
pub struct Names {
    used: HashSet<String>,
}

impl Names {
    pub fn new() -> Names {
        Names::default()
    }

    /// Reserves names that must not be given out.
    pub fn reserve<'a>(&mut self, names: impl IntoIterator<Item = &'a str>) {
        self.used.extend(names.into_iter().map(str::to_string));
    }

    /// `name`, or `name_2`, `name_3`, … if taken.
    pub fn unique(&mut self, name: &str) -> String {
        let out = if self.used.contains(name) {
            (2..).map(|i| format!("{name}_{i}")).find(|n| !self.used.contains(n)).unwrap()
        } else {
            name.to_string()
        };
        self.used.insert(out.clone());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cases() {
        assert_eq!(snake_case("loanCount"), "loan_count");
        assert_eq!(snake_case("nsURI"), "ns_uri");
        assert_eq!(snake_case("XMLParser2"), "xml_parser2");
        assert_eq!(snake_case("SCIENCE_FICTION"), "science_fiction");
        assert_eq!(snake_case("isbn"), "isbn");
        assert_eq!(snake_case("_1"), "_1");
        assert_eq!(pascal_case("SCIENCE_FICTION"), "ScienceFiction");
        assert_eq!(pascal_case("book"), "Book");
        assert_eq!(pascal_case("LECTURE"), "Lecture");
        assert_eq!(upper_snake_case("loanCount"), "LOAN_COUNT");
        let mut names = Names::new();
        assert_eq!(names.unique("a"), "a");
        assert_eq!(names.unique("a"), "a_2");
    }
}
