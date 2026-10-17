//! Styled ranges survive local edits; reflow and PDF output use installed fonts.
use crate::unicode_text::{Preferred, Writer};
use anyhow::{Context, Result, ensure};
use lopdf::{
    Object, Stream,
    content::{Content, Operation},
    dictionary,
};
use std::{ops::Range, path::PathBuf};
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub range: Range<usize>,
    pub font: Option<PathBuf>,
    pub size: f32,
    pub color: [f32; 3],
}
impl Span {
    fn same_style(&self, b: &Self) -> bool {
        self.font == b.font && self.size == b.size && self.color == b.color
    }
}
/// Change only a byte range, keeping every other attribute and range intact.
/// Callers convert character selections to UTF-8 boundaries before applying.
pub fn restyle(
    spans: &[Span],
    range: Range<usize>,
    mut change: impl FnMut(&mut Span),
) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    for span in spans {
        let start = span.range.start.max(range.start);
        let end = span.range.end.min(range.end);
        let mut pieces = Vec::new();
        if start < end {
            if span.range.start < start {
                let mut prefix = span.clone();
                prefix.range.end = start;
                pieces.push(prefix);
            }
            let mut selected = span.clone();
            selected.range = start..end;
            change(&mut selected);
            pieces.push(selected);
            if end < span.range.end {
                let mut suffix = span.clone();
                suffix.range.start = end;
                pieces.push(suffix);
            }
        } else {
            pieces.push(span.clone());
        }
        for piece in pieces {
            if let Some(last) = out.last_mut()
                && last.range.end == piece.range.start
                && last.same_style(&piece)
            {
                last.range.end = piece.range.end;
            } else {
                out.push(piece);
            }
        }
    }
    out
}
/// A contiguous replacement inherits the style at its insertion point. Untouched
/// prefixes/suffixes retain their ranges, even with multi-byte characters.
pub fn edited(before: &str, after: &str, spans: &[Span]) -> Vec<Span> {
    if spans.is_empty() {
        return vec![];
    }
    let prefix = before
        .chars()
        .zip(after.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum::<usize>();
    let suffix = before[prefix..]
        .chars()
        .rev()
        .zip(after[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum::<usize>();
    let cut = before.len() - suffix;
    let end = after.len() - suffix;
    let inherit = spans
        .iter()
        .find(|s| s.range.contains(&prefix))
        .or_else(|| spans.last())
        .unwrap();
    let mut out: Vec<Span> = vec![];
    for (i, c) in after.char_indices() {
        let original = if i < prefix {
            Some(i)
        } else if i >= end {
            Some(cut + i - end)
        } else {
            None
        };
        let style = original
            .and_then(|i| spans.iter().find(|s| s.range.contains(&i)))
            .unwrap_or(inherit);
        if let Some(last) = out.last_mut()
            && last.range.end == i
            && last.same_style(style)
        {
            last.range.end = i + c.len_utf8();
        } else {
            let mut s = style.clone();
            s.range = i..i + c.len_utf8();
            out.push(s);
        }
    }
    out
}
struct Piece {
    text: String,
    style: usize,
    width: f32,
}
pub fn add(bytes: &[u8], page: u32, text: &str, rect: [f32; 4], spans: &[Span]) -> Result<Vec<u8>> {
    add_aligned(
        bytes,
        page,
        text,
        rect,
        spans,
        crate::text_style::Alignment::Left,
    )
}

pub fn add_aligned(
    bytes: &[u8],
    page: u32,
    text: &str,
    rect: [f32; 4],
    spans: &[Span],
    alignment: crate::text_style::Alignment,
) -> Result<Vec<u8>> {
    use crate::text_style::Alignment;
    ensure!(
        text.len() <= 32768 && !spans.is_empty() && spans.len() <= 4096,
        "Text block exceeds its style limit"
    );
    let mut end = 0;
    for s in spans {
        ensure!(
            s.range.start == end
                && s.range.end <= text.len()
                && text.is_char_boundary(s.range.start)
                && text.is_char_boundary(s.range.end)
                && s.size.is_finite()
                && (4.0..=144.).contains(&s.size)
                && s.color
                    .iter()
                    .all(|c| c.is_finite() && (0.0..=1.).contains(c)),
            "Invalid text style range"
        );
        end = s.range.end;
    }
    ensure!(end == text.len(), "Incomplete text styles");
    let mut doc = crate::content::editable(bytes)?;
    let id = *doc.get_pages().get(&page).context("Page not found")?;
    let geometry = crate::annotations::page_geometry(bytes)?;
    let (size, transform) = geometry
        .get(page as usize - 1)
        .context("Page geometry missing")?;
    let [x, y, w, h] = rect;
    ensure!(
        rect.iter().all(|v| v.is_finite())
            && x >= 0.
            && y >= 0.
            && w >= 10.
            && h >= 4.
            && x + w <= size[2] + 0.1
            && y + h <= size[3] + 0.1,
        "Text block must fit inside the page"
    );
    let mut styles: Vec<Span> = vec![];
    let mut writers = vec![];
    let mut rows: Vec<Vec<Piece>> = vec![vec![]];
    let mut paragraph_ends = std::collections::HashSet::new();
    let mut used = 0.;
    for span in spans {
        let style = if let Some(i) = styles.iter().position(|s| s.same_style(span)) {
            i
        } else {
            let font = span
                .font
                .clone()
                .map(Preferred::File)
                .unwrap_or(Preferred::File(crate::document::default_font()?));
            writers.push(Writer::new(&font)?);
            styles.push(span.clone());
            styles.len() - 1
        };
        let writer = &mut writers[style];
        // Preserve explicit line breaks. Wrap by words, falling back to the
        // script-aware writer for tokens that cannot fit on one line.
        for token in text[span.range.clone()].split_inclusive([' ', '\n', '\t']) {
            let newline = token.ends_with('\n');
            let token = token.trim_end_matches('\n').replace('\t', "    ");
            let pieces = if writer.measure(&token, span.size)? > w {
                writer
                    .wrap(&token, span.size, w)?
                    .into_iter()
                    .map(|p| p.0)
                    .collect()
            } else {
                vec![token]
            };
            for (n, piece) in pieces.into_iter().enumerate() {
                let width = writer.measure(&piece, span.size)?;
                if (used + width > w + 0.01 || n > 0) && !rows.last().unwrap().is_empty() {
                    rows.push(vec![]);
                    used = 0.;
                }
                ensure!(
                    width <= w + 0.01,
                    "A character is too wide. Enlarge the block."
                );
                if !piece.is_empty() {
                    rows.last_mut().unwrap().push(Piece {
                        text: piece,
                        style,
                        width,
                    });
                    used += width;
                }
            }
            if newline {
                paragraph_ends.insert(rows.len() - 1);
                rows.push(vec![]);
                used = 0.;
            }
        }
    }
    let mut shaped = vec![];
    let mut top = y;
    paragraph_ends.insert(rows.len() - 1);
    for (row_index, row) in rows.into_iter().enumerate() {
        let height = row
            .iter()
            .map(|p| styles[p.style].size)
            .fold(spans[0].size, f32::max);
        ensure!(
            top - y + height * 1.3 <= h + 0.1,
            "Text does not fit. Enlarge the block or use a smaller font."
        );
        // Merge adjacent pieces of a face before shaping to preserve kerning.
        let mut merged: Vec<Piece> = vec![];
        for p in row {
            if let Some(last) = merged.last_mut()
                && last.style == p.style
            {
                last.text.push_str(&p.text);
                last.width += p.width;
            } else {
                merged.push(p);
            }
        }
        let mut lines = Vec::new();
        for p in merged {
            let line = writers[p.style].shape(&p.text)?;
            let width = line.width(styles[p.style].size);
            let spaces = p.text.chars().filter(|c| *c == ' ').count();
            lines.push((p.style, line, width, spaces));
        }
        let width = lines.iter().map(|p| p.2).sum::<f32>();
        let spaces = lines.iter().map(|p| p.3).sum::<usize>();
        let mut left = x + match alignment {
            Alignment::Center => (w - width).max(0.) / 2.,
            Alignment::Right => (w - width).max(0.),
            _ => 0.,
        };
        let gap = if alignment == Alignment::Justified
            && !paragraph_ends.contains(&row_index)
            && spaces > 0
        {
            (w - width).max(0.) / spaces as f32
        } else {
            0.
        };
        for (style, line, width, spaces) in lines {
            shaped.push((style, line, left, top + height, gap));
            left += width + gap * spaces as f32;
        }
        top += height * 1.3;
    }
    let mut resources = crate::content::resources(&doc, id)?;
    let mut fonts = resources
        .get(b"Font")
        .ok()
        .map(|o| doc.dereference(o))
        .transpose()?
        .map(|(_, o)| o.as_dict().cloned())
        .transpose()?
        .unwrap_or_default();
    let mut names = vec![];
    let mut serial = 0;
    for writer in &mut writers {
        let ids = writer.embed(&mut doc)?;
        names.push(
            ids.into_iter()
                .map(|id| {
                    id.map(|id| {
                        while fonts.has(format!("MyPDFRich{serial}").as_bytes()) {
                            serial += 1;
                        }
                        let name = format!("MyPDFRich{serial}");
                        fonts.set(name.as_str(), id);
                        serial += 1;
                        name
                    })
                })
                .collect::<Vec<_>>(),
        );
    }
    resources.set("Font", fonts);
    let old = doc.get_and_decode_page_content(id)?;
    let mut depth = 0;
    let mut text_open = false;
    for op in &old.operations {
        match op.operator.as_str() {
            "q" => depth += 1,
            "Q" => {
                ensure!(depth > 0, "Unbalanced page graphics");
                depth -= 1;
            }
            "BT" => {
                ensure!(!text_open, "Nested text object");
                text_open = true;
            }
            "ET" => {
                ensure!(text_open, "Unbalanced text object");
                text_open = false;
            }
            _ => {}
        }
    }
    ensure!(depth == 0 && !text_open, "Unbalanced page content");
    let mut ops = vec![Operation::new("q", vec![])];
    ops.extend(old.operations);
    ops.push(Operation::new("Q", vec![]));
    ops.push(Operation::new("q", vec![]));
    ops.push(Operation::new(
        "cm",
        transform.iter().map(|v| Object::Real(*v as f32)).collect(),
    ));
    ops.push(Operation::new("BT", vec![]));
    for (style, line, x, y, gap) in shaped {
        let style_data = &styles[style];
        ops.push(Operation::new(
            "rg",
            style_data.color.iter().map(|v| Object::Real(*v)).collect(),
        ));
        ops.push(Operation::new(
            "Tm",
            vec![
                1.into(),
                0.into(),
                0.into(),
                (-1).into(),
                x.into(),
                y.into(),
            ],
        ));
        ops.extend(writers[style].operations(&line, &names[style], style_data.size, gap, 0.)?);
    }
    ops.push(Operation::new("ET", vec![]));
    ops.push(Operation::new("Q", vec![]));
    let stream = doc.add_object(Stream::new(
        dictionary! {},
        Content { operations: ops }.encode()?,
    ));
    let page = doc.get_dictionary_mut(id)?;
    page.set("Resources", resources);
    page.set("Contents", stream);
    crate::content::serialize(&mut doc)
}
