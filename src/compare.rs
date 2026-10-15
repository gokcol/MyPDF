//! Comparing two versions of a document: which words were removed, added or
//! changed, and where they are on the pages of each version.
//!
//! Lines are compared first, then the words of the lines that differ, so long
//! documents with few changes stay quick.
use crate::viewer::PageText;

/// A word and where it is: page (from 0) and displayed rectangle.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub text: String,
    pub page: usize,
    pub rect: [f32; 4],
    line: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Added,
    Removed,
    Changed,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub kind: Kind,
    /// The words in the older version, if any.
    pub old: Vec<Word>,
    /// The words in the newer version, if any.
    pub new: Vec<Word>,
    /// Where the change is in each version: its first word, or for words that
    /// are not there, a thin mark where they were taken out or would go.
    pub old_at: Option<(usize, [f32; 4])>,
    pub new_at: Option<(usize, [f32; 4])>,
}
impl Change {
    pub fn old_text(&self) -> String {
        self.old
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
    pub fn new_text(&self) -> String {
        self.new
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
    /// The page the change is on, in the newer version when it is there.
    pub fn page(&self) -> usize {
        self.new_at.or(self.old_at).map_or(0, |(page, _)| page)
    }
}

/// The words of every page, in reading order, with their lines.
pub fn words(pages: &[PageText]) -> Vec<Word> {
    let mut out: Vec<Word> = vec![];
    let mut line = 0;
    for (page, text) in pages.iter().enumerate() {
        line += 1;
        let mut current: Option<Word> = None;
        let mut previous_end = 0;
        for glyph in &text.glyphs {
            let piece = &text.text[glyph.range.clone()];
            let between = text.text.get(previous_end..glyph.range.start).unwrap_or("");
            previous_end = glyph.range.end;
            if between.contains('\n') {
                line += 1;
            }
            let gap = between.chars().any(char::is_whitespace) || piece.trim().is_empty();
            if gap && let Some(word) = current.take() {
                out.push(word);
            }
            if piece.trim().is_empty() {
                continue;
            }
            match &mut current {
                Some(word) => {
                    word.text.push_str(piece);
                    word.rect = [
                        word.rect[0].min(glyph.rect[0]),
                        word.rect[1].min(glyph.rect[1]),
                        word.rect[2].max(glyph.rect[2]),
                        word.rect[3].max(glyph.rect[3]),
                    ];
                }
                None => {
                    current = Some(Word {
                        text: piece.to_owned(),
                        page,
                        rect: glyph.rect,
                        line,
                    })
                }
            }
        }
        out.extend(current);
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Same,
    Delete,
    Insert,
}
/// Myers' shortest edit script between `a` and `b`, or None when they differ
/// in more than `limit` places.
fn diff<T: PartialEq>(a: &[T], b: &[T], limit: usize) -> Option<Vec<Op>> {
    // Common beginnings and endings need no search.
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (a, b) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);
    let (n, m) = (a.len() as isize, b.len() as isize);
    let max = (n + m) as usize;
    let limit = limit.min(max) as isize;
    let offset = max as isize + 1;
    let mut v = vec![0isize; 2 * max + 3];
    // Before each step d, the furthest points on diagonals -d-1..=d+1.
    let mut trace: Vec<Vec<isize>> = vec![];
    let mut found = false;
    'search: for d in 0..=limit {
        trace.push(v[(offset - d - 1) as usize..=(offset + d + 1) as usize].to_vec());
        let mut k = -d;
        while k <= d {
            let index = (k + offset) as usize;
            let mut x = if k == -d || (k != d && v[index - 1] < v[index + 1]) {
                v[index + 1]
            } else {
                v[index - 1] + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[index] = x;
            if x >= n && y >= m {
                found = true;
                break 'search;
            }
            k += 2;
        }
    }
    if !found {
        return None;
    }
    // Walk back through the trace to recover the script.
    let mut ops = vec![Op::Same; suffix];
    let (mut x, mut y) = (n, m);
    for d in (0..trace.len() as isize).rev() {
        let at = |k: isize| trace[d as usize][(k + d + 1) as usize];
        let k = x - y;
        let previous_k = if k == -d || (k != d && at(k - 1) < at(k + 1)) {
            k + 1
        } else {
            k - 1
        };
        let previous_x = at(previous_k);
        let previous_y = previous_x - previous_k;
        while x > previous_x && y > previous_y {
            ops.push(Op::Same);
            x -= 1;
            y -= 1;
        }
        if d > 0 {
            ops.push(if x == previous_x {
                Op::Insert
            } else {
                Op::Delete
            });
        }
        x = previous_x;
        y = previous_y;
    }
    ops.extend(std::iter::repeat_n(Op::Same, prefix));
    ops.reverse();
    Some(ops)
}

/// The place of the word at `index`, or a mark just before it, or after the
/// last word.
fn place(words: &[Word], index: usize, gap: bool) -> Option<(usize, [f32; 4])> {
    match words.get(index) {
        Some(w) if gap => Some((
            w.page,
            [w.rect[0] - 1., w.rect[1], w.rect[0] + 1., w.rect[3]],
        )),
        Some(w) => Some((w.page, w.rect)),
        None => words.last().map(|w| {
            (
                w.page,
                [w.rect[2] - 1., w.rect[1], w.rect[2] + 1., w.rect[3]],
            )
        }),
    }
}

/// Group a word-level script into changes; `a` and `b` are every word, and
/// the script covers those from `i` and `j` on.
fn changes_of(
    a: &[Word],
    b: &[Word],
    (mut i, mut j): (usize, usize),
    ops: &[Op],
    out: &mut Vec<Change>,
) {
    let (mut i0, mut j0) = (i, j);
    let mut flush = |i0: &mut usize, j0: &mut usize, i: usize, j: usize| {
        if (*i0, *j0) != (i, j) {
            out.push(change(a, b, *i0..i, *j0..j));
        }
        (*i0, *j0) = (i, j);
    };
    for op in ops {
        match op {
            Op::Same => {
                flush(&mut i0, &mut j0, i, j);
                i += 1;
                j += 1;
                (i0, j0) = (i, j);
            }
            Op::Delete => i += 1,
            Op::Insert => j += 1,
        }
    }
    flush(&mut i0, &mut j0, i, j);
}

fn change(
    a: &[Word],
    b: &[Word],
    old: std::ops::Range<usize>,
    new: std::ops::Range<usize>,
) -> Change {
    Change {
        kind: match (old.is_empty(), new.is_empty()) {
            (false, false) => Kind::Changed,
            (false, true) => Kind::Removed,
            _ => Kind::Added,
        },
        old_at: place(a, old.start, old.is_empty()),
        new_at: place(b, new.start, new.is_empty()),
        old: a[old].to_vec(),
        new: b[new].to_vec(),
    }
}

/// What changed from the older version to the newer one.
pub fn compare(older: &[PageText], newer: &[PageText]) -> Result<Vec<Change>, String> {
    let (a, b) = (words(older), words(newer));
    if a.is_empty() && b.is_empty() {
        return Err("These documents have no text MyPDF can read. If they are scans, use Recognize Text on each one first.".into());
    }
    let lines = |words: &[Word]| -> Vec<(String, std::ops::Range<usize>)> {
        let mut out: Vec<(String, std::ops::Range<usize>)> = vec![];
        for (i, w) in words.iter().enumerate() {
            match out.last_mut() {
                Some((text, range)) if words[range.start].line == w.line => {
                    text.push(' ');
                    text.push_str(&w.text);
                    range.end = i + 1;
                }
                _ => out.push((w.text.clone(), i..i + 1)),
            }
        }
        out
    };
    let (la, lb) = (lines(&a), lines(&b));
    let keys_a: Vec<&str> = la.iter().map(|(t, _)| t.as_str()).collect();
    let keys_b: Vec<&str> = lb.iter().map(|(t, _)| t.as_str()).collect();
    let ops = diff(&keys_a, &keys_b, 4000).ok_or(
        "These documents are too different to compare line by line. Make sure you chose two versions of the same document.",
    )?;
    let mut out = vec![];
    let (mut i, mut j) = (0, 0);
    let mut k = 0;
    while k < ops.len() {
        if ops[k] == Op::Same {
            i += 1;
            j += 1;
            k += 1;
            continue;
        }
        // A run of differing lines: compare their words.
        let (i0, j0) = (i, j);
        while k < ops.len() && ops[k] != Op::Same {
            match ops[k] {
                Op::Delete => i += 1,
                Op::Insert => j += 1,
                Op::Same => unreachable!(),
            }
            k += 1;
        }
        let span =
            |lines: &[(String, std::ops::Range<usize>)], from: usize, to: usize, all: usize| {
                if from == to {
                    // No lines on this side: an empty range where they would be.
                    let at = lines.get(from).map_or(all, |(_, r)| r.start);
                    at..at
                } else {
                    lines[from].1.start..lines[to - 1].1.end
                }
            };
        let old = span(&la, i0, i, a.len());
        let new = span(&lb, j0, j, b.len());
        let texts_old: Vec<&str> = a[old.clone()].iter().map(|w| w.text.as_str()).collect();
        let texts_new: Vec<&str> = b[new.clone()].iter().map(|w| w.text.as_str()).collect();
        match diff(&texts_old, &texts_new, 2000) {
            Some(word_ops) => changes_of(&a, &b, (old.start, new.start), &word_ops, &mut out),
            // Rewritten from scratch: one change for the whole passage.
            None => out.push(change(&a, &b, old, new)),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edit_scripts_are_shortest() {
        let a: Vec<char> = "ABCABBA".chars().collect();
        let b: Vec<char> = "CBABAC".chars().collect();
        let ops = diff(&a, &b, 100).unwrap();
        assert_eq!(ops.iter().filter(|o| **o != Op::Same).count(), 5);
        assert_eq!(diff(&a, &a, 0).unwrap().len(), 7);
        assert!(diff(&a, &b, 2).is_none());
        let empty: Vec<char> = vec![];
        assert_eq!(diff(&empty, &b, 10).unwrap(), vec![Op::Insert; 6]);
    }
    #[test]
    fn edit_scripts_turn_one_sequence_into_the_other() {
        let mut seed = 7u32;
        let mut next = || {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            (seed >> 16) % 4
        };
        for _ in 0..300 {
            let a: Vec<u32> = (0..next() * 5).map(|_| next()).collect();
            let b: Vec<u32> = (0..next() * 5).map(|_| next()).collect();
            let ops = diff(&a, &b, 100).unwrap();
            let (mut i, mut j, mut rebuilt) = (0, 0, vec![]);
            for op in ops {
                match op {
                    Op::Same => {
                        assert_eq!(a[i], b[j]);
                        rebuilt.push(a[i]);
                        i += 1;
                        j += 1;
                    }
                    Op::Delete => i += 1,
                    Op::Insert => {
                        rebuilt.push(b[j]);
                        j += 1;
                    }
                }
            }
            assert_eq!((i, &rebuilt), (a.len(), &b));
        }
    }
}
