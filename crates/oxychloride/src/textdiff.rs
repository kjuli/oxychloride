//! Minimal line edits between two versions of a text (Myers' difference algorithm), for
//! changes that are computed as a new text — a `.henshin` file written again, a document
//! converted from its counterpart — but should reach the client as small edits, so that the
//! unchanged parts (and the editor's undo history and markers there) stay as they are.

use std::ops::Range;

/// A replacement of `old[range]` by `text`, by byte offsets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Replacement {
    pub(crate) range: Range<usize>,
    pub(crate) text: String,
}

/// The replacements of whole lines that turn `old` into `new`, in document order and not
/// overlapping (empty if the texts are equal).
pub(crate) fn line_replacements(old: &str, new: &str) -> Vec<Replacement> {
    let a: Vec<&str> = old.split_inclusive('\n').collect();
    let b: Vec<&str> = new.split_inclusive('\n').collect();
    // The common start and end need no search.
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..].iter().rev().zip(b[prefix..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (a_mid, b_mid) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);
    let script = diff(a_mid, b_mid);
    // Byte offsets of the lines of `old`.
    let mut offsets = Vec::with_capacity(a.len() + 1);
    let mut at = 0;
    for l in &a {
        offsets.push(at);
        at += l.len();
    }
    offsets.push(at);
    let mut out: Vec<Replacement> = Vec::new();
    for (ar, br) in script {
        let range = offsets[prefix + ar.start]..offsets[prefix + ar.end];
        let text: String = b_mid[br].concat();
        out.push(Replacement { range, text });
    }
    out
}

/// The changed regions of two line sequences: pairs of ranges (in `a` and in `b`) that differ,
/// with equal lines between them.
fn diff(a: &[&str], b: &[&str]) -> Vec<(Range<usize>, Range<usize>)> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    if n == 0 && m == 0 {
        return Vec::new();
    }
    if n == 0 || m == 0 {
        return vec![(0..a.len(), 0..b.len())];
    }
    let max = (n + m) as usize;
    let offset = max as isize;
    let mut v = vec![0isize; 2 * max + 2];
    let mut trace: Vec<Vec<isize>> = Vec::new();
    'search: for d in 0..=max as isize {
        trace.push(v.clone());
        let mut k = -d;
        while k <= d {
            let i = (k + offset) as usize;
            let mut x = if k == -d || (k != d && v[i - 1] < v[i + 1]) { v[i + 1] } else { v[i - 1] + 1 };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[i] = x;
            if x >= n && y >= m {
                break 'search;
            }
            k += 2;
        }
    }
    // Walk back through the trace, collecting the equal runs (the snakes).
    let mut equal: Vec<(usize, usize, usize)> = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (0..trace.len() as isize).rev() {
        let v = &trace[d as usize];
        let k = x - y;
        let prev_k = if k == -d || (k != d && v[(k - 1 + offset) as usize] < v[(k + 1 + offset) as usize]) {
            k + 1
        } else {
            k - 1
        };
        let prev_x = v[(prev_k + offset) as usize];
        let prev_y = prev_x - prev_k;
        let (sx, sy) = if d == 0 {
            (0, 0)
        } else {
            (prev_x + if prev_k == k - 1 { 1 } else { 0 }, prev_y + if prev_k == k + 1 { 1 } else { 0 })
        };
        if x > sx {
            let len = (x - sx) as usize;
            equal.push((sx as usize, sy as usize, len));
        }
        x = prev_x;
        y = prev_y;
        if d == 0 {
            break;
        }
    }
    equal.reverse();
    // The changes are what lies between the equal runs.
    let mut out = Vec::new();
    let (mut ai, mut bi) = (0usize, 0usize);
    for (ax, bx, len) in equal.into_iter().chain(std::iter::once((a.len(), b.len(), 0))) {
        if ax > ai || bx > bi {
            out.push((ai..ax, bi..bx));
        }
        ai = ax + len;
        bi = bx + len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(old: &str, edits: &[Replacement]) -> String {
        let mut out = old.to_string();
        for e in edits.iter().rev() {
            out.replace_range(e.range.clone(), &e.text);
        }
        out
    }

    #[test]
    fn finds_small_edits() {
        let cases = [
            ("a\nb\nc\n", "a\nb\nc\n"),
            ("a\nb\nc\n", "a\nx\nc\n"),
            ("a\nb\nc\n", "a\nc\n"),
            ("a\nc\n", "a\nb\nc\n"),
            ("", "a\n"),
            ("a\n", ""),
            ("a\nb", "a\nb\n"),
            ("x\ny\nz\nw\n", "y\nq\nw\nr\n"),
            ("1\n2\n3\n4\n5\n6\n", "1\n3\n4\n2\n5\n6\n"),
        ];
        for (old, new) in cases {
            let edits = line_replacements(old, new);
            assert_eq!(apply(old, &edits), new, "{old:?} -> {new:?}: {edits:?}");
        }
        assert!(line_replacements("a\nb\n", "a\nb\n").is_empty());
        let edits = line_replacements("a\nb\nc\nd\n", "a\nb\nX\nd\n");
        assert_eq!(edits, vec![Replacement { range: 4..6, text: "X\n".into() }]);
    }
}
