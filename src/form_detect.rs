//! Finding where a form's fields belong on a page that has none: blanks of
//! underscores or dots, drawn lines and boxes, and box characters such as ☐,
//! each named after the label beside or above it.
use crate::form_design::{self, FieldType, Properties};
use anyhow::Result;
use hayro::{
    hayro_interpret::{
        self as interpret, BlendMode, ClipPath, Device, DrawMode, DrawProps, Image, ImageDrawProps,
        SoftMask, TransformExt,
    },
    hayro_syntax::Pdf,
    kurbo::{BezPath, PathEl, Point, Rect},
};
use std::collections::HashSet;

/// A field the page layout suggests.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub page: usize,
    /// Displayed page coordinates: left, top, right, bottom.
    pub rect: [f32; 4],
    pub kind: FieldType,
    pub name: String,
    pub multiline: bool,
}

/// Lines and boxes drawn on a page, in displayed coordinates.
#[derive(Default)]
struct Shapes {
    lines: Vec<Rect>,
    verticals: Vec<Rect>,
    boxes: Vec<Rect>,
    count: usize,
}
impl Shapes {
    fn add(&mut self, path: &BezPath, stroked: bool) {
        // Each subpath on its own: a box, a line, or something else.
        let mut points: Vec<Point> = vec![];
        let mut straight = true;
        let mut finish = |points: &mut Vec<Point>, straight: bool| {
            if points.len() >= 2 && straight {
                let mut r = Rect::from_points(points[0], points[0]);
                for p in points.iter() {
                    r = r.union_pt(*p);
                }
                let axis = points
                    .windows(2)
                    .all(|w| (w[0].x - w[1].x).abs() < 0.5 || (w[0].y - w[1].y).abs() < 0.5);
                if axis {
                    let (w, h) = (r.width(), r.height());
                    if h <= 2.0 && w >= 20. {
                        self.lines.push(r);
                    } else if w <= 2.0 && h >= 8. {
                        self.verticals.push(r);
                    } else if stroked && points.len() >= 4 && w >= 5. && h >= 5. {
                        self.boxes.push(r);
                    }
                }
            }
            points.clear();
        };
        for el in path.elements() {
            match el {
                PathEl::MoveTo(p) => {
                    finish(&mut points, straight);
                    straight = true;
                    points.push(*p);
                }
                PathEl::LineTo(p) => points.push(*p),
                PathEl::ClosePath => {
                    if let Some(first) = points.first().copied() {
                        points.push(first);
                    }
                }
                _ => straight = false,
            }
        }
        finish(&mut points, straight);
    }
}
impl<'a> Device<'a> for Shapes {
    fn draw_path(&mut self, path: &BezPath, props: DrawProps<'a>, mode: &DrawMode) {
        self.count += 1;
        if self.count > 50_000 {
            return;
        }
        let stroked = match mode {
            DrawMode::Stroke(_) | DrawMode::FillAndStroke(..) => true,
            DrawMode::Fill(_) => false,
            DrawMode::Invisible => return,
        };
        self.add(&(props.transform * path.clone()), stroked);
    }
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
    fn draw_glyph_run(
        &mut self,
        _: &interpret::font::GlyphRun<'_, 'a>,
        _: DrawProps<'a>,
        _: &DrawMode,
    ) {
    }
}

#[derive(Clone)]
struct Glyph {
    text: String,
    rect: Rect,
    /// Position in reading order, and whether the text has a space before it.
    index: usize,
    space_before: bool,
}
const BOX_CHARACTERS: [char; 11] = [
    '☐', '☑', '☒', '□', '▢', '❑', '❏', '⬜', '◻', '\u{f06f}', '\u{f0a8}',
];

pub(crate) fn label_kind(label: &str) -> Option<FieldType> {
    let lower = label.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    if has(&[
        "signature",
        "sign here",
        "signed",
        "imza",
        "unterschrift",
        "firma",
        "signé",
    ]) {
        Some(FieldType::Signature)
    } else if has(&["date", "tarih", "datum", "fecha", "data ", "birth", "dob"]) {
        Some(FieldType::Date)
    } else {
        None
    }
}
fn clean_label(raw: &str) -> String {
    let words: Vec<&str> = raw.split_whitespace().collect();
    let mut label = words.join(" ");
    label = label
        .trim_matches(|c: char| {
            c.is_whitespace() || ":：*_.…()[]-–—#".contains(c) || BOX_CHARACTERS.contains(&c)
        })
        .replace('.', " ");
    label.chars().take(40).collect::<String>().trim().to_owned()
}
/// Text of the glyphs on a line, with the spaces between words.
fn join(glyphs: &[&Glyph]) -> String {
    // Roughly the font size, from the tallest glyph.
    let em = glyphs.iter().map(|g| g.rect.height()).fold(4., f64::max) / 0.9;
    let mut out = String::new();
    let mut last: Option<&Glyph> = None;
    for g in glyphs {
        if let Some(l) = last {
            let space = if g.index == l.index + 1 {
                g.space_before
            } else {
                g.rect.x0 - l.rect.x1 > em * 0.2
            };
            if space && !out.ends_with(' ') {
                out.push(' ');
            }
        }
        out.push_str(&g.text);
        last = Some(g);
    }
    out
}
fn same_line(g: &Rect, band: &Rect) -> bool {
    let center = (g.y0 + g.y1) / 2.;
    center >= band.y0 - 3. && center <= band.y1 + 3.
}
/// The label for a field: text just before it on its line, after it for a
/// check box, or the line just above it.
fn label_for(rect: &Rect, glyphs: &[Glyph], checkbox: bool) -> String {
    let before: Vec<&Glyph> = {
        let mut v: Vec<&Glyph> = glyphs
            .iter()
            .filter(|g| {
                same_line(&g.rect, rect) && g.rect.x1 <= rect.x0 + 2. && g.rect.x0 >= rect.x0 - 220.
            })
            .filter(|g| !g.text.chars().all(|c| c == '_' || c == '.' || c == '…'))
            .collect();
        v.sort_by(|a, b| a.rect.x0.total_cmp(&b.rect.x0));
        // Only the words nearest the field, back to a wide gap.
        let mut start = v.len();
        let mut edge = rect.x0;
        while start > 0 && edge - v[start - 1].rect.x1 < 24. {
            start -= 1;
            edge = v[start].rect.x0;
        }
        v[start..].to_vec()
    };
    let after: Vec<&Glyph> = {
        let mut v: Vec<&Glyph> = glyphs
            .iter()
            .filter(|g| {
                same_line(&g.rect, rect) && g.rect.x0 >= rect.x1 - 2. && g.rect.x0 <= rect.x1 + 220.
            })
            .collect();
        v.sort_by(|a, b| a.rect.x0.total_cmp(&b.rect.x0));
        let mut end = 0;
        let mut edge = rect.x1;
        while end < v.len() && v[end].rect.x0 - edge < 24. {
            edge = v[end].rect.x1;
            end += 1;
        }
        v[..end].to_vec()
    };
    let above = || -> String {
        let mut v: Vec<&Glyph> = glyphs
            .iter()
            .filter(|g| {
                g.rect.y1 <= rect.y0 + 1.
                    && g.rect.y1 >= rect.y0 - 18.
                    && g.rect.x1 >= rect.x0
                    && g.rect.x0 <= rect.x1
            })
            .collect();
        v.sort_by(|a, b| a.rect.x0.total_cmp(&b.rect.x0));
        clean_label(&join(&v))
    };
    let (first, second) = if checkbox {
        (after, before)
    } else {
        (before, after)
    };
    for candidate in [clean_label(&join(&first)), clean_label(&join(&second))] {
        if !candidate.is_empty() {
            return candidate;
        }
    }
    if checkbox { String::new() } else { above() }
}
fn overlaps(a: &Rect, b: &Rect, share: f64) -> bool {
    let i = a.intersect(*b);
    i.width() > 0. && i.height() > 0. && i.area() >= share * a.area().min(b.area())
}

/// Suggest fields for every page, skipping places that already have one.
pub fn detect(bytes: &[u8]) -> Result<Vec<Suggestion>> {
    let pdf =
        Pdf::new(bytes.to_vec()).map_err(|e| anyhow::anyhow!("Could not read the PDF: {e:?}"))?;
    let existing: Vec<(usize, Rect)> = crate::forms::fields(bytes)
        .unwrap_or_default()
        .iter()
        .flat_map(|f| f.widgets.iter())
        .map(|w| {
            (
                w.page,
                Rect::new(
                    w.rect[0] as f64,
                    w.rect[1] as f64,
                    w.rect[2] as f64,
                    w.rect[3] as f64,
                ),
            )
        })
        .collect();
    let mut out: Vec<Suggestion> = vec![];
    let cache = interpret::InterpreterCache::new();
    for (index, page) in pdf.pages().iter().enumerate().take(500) {
        let text = crate::viewer::extract_page(page, &cache);
        let glyphs: Vec<Glyph> = text
            .glyphs
            .iter()
            .enumerate()
            .map(|(index, g)| Glyph {
                text: text.text[g.range.clone()].to_owned(),
                rect: Rect::new(
                    g.rect[0] as f64,
                    g.rect[1] as f64,
                    g.rect[2] as f64,
                    g.rect[3] as f64,
                ),
                index,
                space_before: text.text[..g.range.start].ends_with(char::is_whitespace),
            })
            .collect();
        let (w, h) = page.render_dimensions();
        let mut shapes = Shapes::default();
        let mut context = interpret::Context::new(
            page.initial_transform(true).to_kurbo(),
            Rect::new(0., 0., w as f64, h as f64),
            &cache,
            page.xref(),
            interpret::InterpreterSettings::default(),
        );
        interpret::interpret_page(page, &mut context, &mut shapes);
        let page_width = w as f64;
        // A field is about one and a half text lines tall.
        let mut heights: Vec<f64> = glyphs
            .iter()
            .filter(|g| g.text.chars().all(char::is_alphanumeric))
            .map(|g| g.rect.height())
            .filter(|h| *h > 2.)
            .collect();
        heights.sort_by(f64::total_cmp);
        let line = heights
            .get(heights.len() / 2)
            .map_or(16., |h| (h * 1.7).clamp(12., 22.));
        let mut found: Vec<(Rect, FieldType, bool, String)> = vec![];
        let blank = |r: &Rect, glyphs: &[Glyph]| {
            !glyphs.iter().any(|g| {
                let c = g.rect.center();
                r.inset(-1.).contains(c) && !g.text.trim().is_empty()
            })
        };

        // 1. Runs of underscores or dots typed as blanks.
        let mut i = 0;
        while i < glyphs.len() {
            let filler = |g: &Glyph| {
                g.text.chars().all(|c| c == '_') && !g.text.is_empty()
                    || g.text.chars().all(|c| c == '.' || c == '…') && !g.text.is_empty()
            };
            if !filler(&glyphs[i]) {
                i += 1;
                continue;
            }
            let dots = glyphs[i].text.starts_with(['.', '…']);
            let mut j = i + 1;
            while j < glyphs.len()
                && filler(&glyphs[j])
                && glyphs[j].text.starts_with(['.', '…']) == dots
                && (glyphs[j].rect.x0 - glyphs[j - 1].rect.x1).abs() < 4.
                && (glyphs[j].rect.y1 - glyphs[j - 1].rect.y1).abs() < 3.
            {
                j += 1;
            }
            let run: Vec<&Glyph> = glyphs[i..j].iter().collect();
            let count: usize = run.iter().map(|g| g.text.chars().count()).sum();
            let left = run.first().unwrap().rect.x0;
            let right = run.last().unwrap().rect.x1;
            if count >= if dots { 5 } else { 3 } && right - left >= 18. {
                let bottom = run.iter().map(|g| g.rect.y1).fold(f64::MIN, f64::max) + 1.;
                let r = Rect::new(left, bottom - line, right, bottom);
                found.push((r, FieldType::Text, false, label_for(&r, &glyphs, false)));
            }
            i = j;
        }

        // 2. Box characters: ☐ Yes  ☐ No.
        for g in &glyphs {
            if g.text.chars().count() == 1 && g.text.chars().all(|c| BOX_CHARACTERS.contains(&c)) {
                let side = g.rect.width().max(g.rect.height()).clamp(8., 20.);
                let r = Rect::from_center_size(g.rect.center(), (side, side));
                found.push((r, FieldType::Checkbox, false, label_for(&r, &glyphs, true)));
            }
        }

        // 3. Drawn boxes that are empty: check boxes, text fields, text areas.
        let boxes: Vec<Rect> = shapes
            .boxes
            .iter()
            .copied()
            .filter(|b| b.width() < page_width * 0.7 || b.height() < h as f64 * 0.4)
            .collect();
        for b in &boxes {
            let container = boxes
                .iter()
                .any(|o| o != b && b.contains(o.center()) && o.area() < b.area() * 0.9);
            if container || !blank(b, &glyphs) {
                continue;
            }
            let (bw, bh) = (b.width(), b.height());
            let inner = b.inset(-1.5);
            if (6.0..=24.).contains(&bw)
                && (6.0..=24.).contains(&bh)
                && (bw - bh).abs() <= bw.max(bh) * 0.25
            {
                found.push((
                    inner,
                    FieldType::Checkbox,
                    false,
                    label_for(b, &glyphs, true),
                ));
            } else if bw >= 40. && (10.0..=34.).contains(&bh) {
                found.push((inner, FieldType::Text, false, label_for(b, &glyphs, false)));
            } else if bw >= 60. && (34.0..=260.).contains(&bh) {
                found.push((inner, FieldType::Text, true, label_for(b, &glyphs, false)));
            }
        }

        // 4. Lines to write on: the empty stretches just above each line.
        for l in &shapes.lines {
            if l.width() > page_width * 0.95
                || boxes.iter().any(|b| {
                    l.x0 >= b.x0 - 2.
                        && l.x1 <= b.x1 + 2.
                        && ((l.y0 - b.y0).abs() < 2. || (l.y1 - b.y1).abs() < 2.)
                })
            {
                continue;
            }
            let y = l.center().y;
            let band = Rect::new(l.x0, y - line, l.x1, y - 0.5);
            // Split the stretch where text or a vertical rule interrupts it.
            let mut taken: Vec<(f64, f64)> = glyphs
                .iter()
                .filter(|g| {
                    let overlap = g.rect.y1.min(band.y1) - g.rect.y0.max(band.y0);
                    overlap > g.rect.height() * 0.3 && g.rect.x1 > band.x0 && g.rect.x0 < band.x1
                })
                .map(|g| (g.rect.x0 - 3., g.rect.x1 + 3.))
                .chain(
                    shapes
                        .verticals
                        .iter()
                        .filter(|v| {
                            v.y0 < band.y1
                                && v.y1 > band.y0
                                && v.x0 > band.x0 + 1.
                                && v.x1 < band.x1 - 1.
                        })
                        .map(|v| (v.x0 - 1.5, v.x1 + 1.5)),
                )
                .collect();
            taken.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut start = band.x0;
            for (a, b) in taken.iter().copied().chain([(band.x1, band.x1)]) {
                if a - start >= 36. {
                    let r = Rect::new(start, band.y0, a, band.y1);
                    found.push((r, FieldType::Text, false, label_for(&r, &glyphs, false)));
                }
                start = start.max(b);
            }
        }

        // Keep the first suggestion for a place, and none where a field exists.
        let mut kept: Vec<(Rect, FieldType, bool, String)> = vec![];
        for (r, kind, multiline, label) in found {
            if existing
                .iter()
                .any(|(p, e)| *p == index && overlaps(&r, e, 0.3))
                || kept.iter().any(|(k, ..)| overlaps(&r, k, 0.3))
            {
                continue;
            }
            let kind = match (kind, label_kind(&label)) {
                (FieldType::Text, Some(special)) => special,
                (kind, _) => kind,
            };
            kept.push((r, kind, multiline, label));
        }
        for (mut r, kind, multiline, name) in kept {
            if kind == FieldType::Signature && r.height() < 36. {
                // Room for a signature, as long as it stays on the page.
                r.y0 = (r.y1 - 36.).max(0.);
            }
            out.push(Suggestion {
                page: index,
                rect: [r.x0 as f32, r.y0 as f32, r.x1 as f32, r.y1 as f32],
                kind,
                name,
                multiline: multiline && kind == FieldType::Text,
            });
            if out.len() >= 500 {
                return Ok(out);
            }
        }
    }
    Ok(out)
}

/// Add suggested fields, named after their labels. Returns the new bytes and
/// how many fields were added.
pub fn add_detected(bytes: &[u8], suggestions: &[Suggestion]) -> Result<(Vec<u8>, usize)> {
    let mut taken: HashSet<String> = crate::forms::fields(bytes)
        .unwrap_or_default()
        .into_iter()
        .map(|f| f.name)
        .collect();
    let mut counters: std::collections::HashMap<FieldType, usize> = Default::default();
    let mut fields = vec![];
    for s in suggestions {
        let base = if s.name.is_empty() {
            // Acrobat-style names for unlabelled fields: Text1, Check Box1, …
            let n = counters.entry(s.kind).or_insert(0);
            loop {
                *n += 1;
                let name = format!("{}{n}", s.kind.base_name());
                if !taken.contains(&name) {
                    break name;
                }
            }
        } else {
            s.name.clone()
        };
        let name = (1..)
            .map(|n| {
                if n == 1 {
                    base.clone()
                } else {
                    format!("{base} {n}")
                }
            })
            .find(|n| !taken.contains(n))
            .unwrap();
        let mut properties = Properties::new(s.kind, &name);
        properties.multiline = s.multiline;
        taken.insert(name);
        fields.push((s.page, s.rect, s.kind, properties));
    }
    let (bytes, ids) = form_design::add_many(bytes, &fields)?;
    Ok((bytes, ids.len()))
}
