//! Redaction: permanently remove the text, image pixels and annotations under
//! marked areas, then cover the areas with boxes.
//!
//! The precise method edits page content glyph by glyph: each removed glyph
//! becomes a positioning adjustment, so the rest of the line stays where it
//! was. The result is checked by rendering: every other glyph must be in place
//! and no glyph or image pixel may remain readable under a box. Content that
//! cannot be edited that way (text inside form objects, unusual font
//! encodings, stencil masks) is refused, and the flatten method renders the
//! page to an image with the boxes burned in instead.
use anyhow::{Context as _, Result, bail, ensure};
use hayro::{
    hayro_interpret::{
        self as interpret, BlendMode, ClipPath, Device, DrawMode, DrawProps, Image, ImageData,
        ImageDrawProps, SoftMask, TransformExt,
    },
    hayro_syntax::Pdf,
    kurbo::{BezPath, Point, Rect, Shape, Vec2},
};
use lopdf::{
    Dictionary, Document, Object, ObjectId, Stream, StringFormat,
    content::{Content, Operation},
};
use std::collections::{BTreeMap, HashMap, HashSet};

/// A marked area on a page, in displayed page coordinates (left, top, right, bottom).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub page: usize,
    pub rect: [f32; 4],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Method {
    /// Remove text glyphs, image pixels and annotations; keep the rest of the page.
    #[default]
    Precise,
    /// Replace each redacted page with an image of it, boxes burned in.
    Flatten,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    pub method: Method,
    /// Box color.
    pub fill: [f32; 3],
}
impl Default for Options {
    fn default() -> Self {
        Self {
            method: Method::Precise,
            fill: [0.; 3],
        }
    }
}

/// What was removed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub pages: usize,
    pub glyphs: usize,
    pub images: usize,
    pub annotations: usize,
    pub flattened: usize,
}

/// Tagged text runs paint with colors that carry this bit, so text from form
/// objects (painted in its own colors) is told apart.
const TAG: usize = 0x80_0000;

pub fn apply(bytes: &[u8], areas: &[Area], options: &Options) -> Result<(Vec<u8>, Report)> {
    ensure!(!areas.is_empty(), "Mark at least one area to redact");
    ensure!(
        areas.iter().all(|a| a.rect.iter().all(|v| v.is_finite())
            && a.rect[2] - a.rect[0] >= 0.5
            && a.rect[3] - a.rect[1] >= 0.5),
        "Each redaction area must be at least half a point wide and tall"
    );
    ensure!(
        options
            .fill
            .iter()
            .all(|c| c.is_finite() && (0.0..=1.0).contains(c)),
        "Invalid box color"
    );
    let doc = Document::load_mem(bytes)?;
    ensure!(
        !crate::document::has_signatures(&doc),
        "Redaction can't be applied to a signed PDF: its signed revision would still contain the original content."
    );
    let count = doc.get_pages().len();
    let mut pages: BTreeMap<usize, Vec<Rect>> = BTreeMap::new();
    for a in areas {
        ensure!(a.page < count, "Page {} does not exist", a.page + 1);
        let r = a.rect;
        pages.entry(a.page).or_default().push(Rect::new(
            r[0] as f64,
            r[1] as f64,
            r[2] as f64,
            r[3] as f64,
        ));
    }
    let mut current = bytes.to_vec();
    let mut report = Report::default();
    for (page, rects) in &pages {
        current = match options.method {
            Method::Precise => precise(&current, *page, rects, options.fill, &mut report)?,
            Method::Flatten => flatten(&current, *page, rects, options.fill, &mut report)?,
        };
        report.pages += 1;
    }
    Ok((current, report))
}

/// A glyph as drawn: its origin, box, and the device vector of 1000 glyph units.
#[derive(Clone, Copy, Debug)]
struct GlyphAt {
    origin: Point,
    rect: Rect,
    ex: Vec2,
}

/// Everything drawn on a page that matters for redaction.
struct Probe<'p> {
    areas: &'p [Rect],
    fill: [u8; 3],
    /// Tagged glyphs by run (zero-based run index).
    runs: HashMap<usize, Vec<GlyphAt>>,
    /// Every glyph, tagged or not.
    all: Vec<GlyphAt>,
    /// Glyphs under an area that no page-level run accounts for.
    untagged_under: bool,
    /// Raster images under an area, with their pixels already covered.
    images: Vec<(Rect, Option<image::RgbaImage>)>,
    /// Pixels under an area that are not the box color (verification).
    leaks: usize,
}
impl<'p> Probe<'p> {
    fn new(areas: &'p [Rect], fill: [f32; 3]) -> Self {
        Self {
            areas,
            fill: fill.map(|c| (c * 255.).round() as u8),
            runs: HashMap::new(),
            all: vec![],
            untagged_under: false,
            images: vec![],
            leaks: 0,
        }
    }
    fn inside(&self, p: Point) -> bool {
        self.areas.iter().any(|a| a.contains(p))
    }
}
impl<'a> Device<'a> for Probe<'_> {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
    fn draw_glyph_run(
        &mut self,
        run: &interpret::font::GlyphRun<'_, 'a>,
        props: DrawProps<'a>,
        _: &DrawMode,
    ) {
        // Invisible text (such as an OCR layer) is recorded too.
        let id = match props.paint {
            interpret::Paint::Color(color) => {
                let [r, g, b, _] = color.to_rgba().to_rgba8();
                ((r as usize) << 16) | ((g as usize) << 8) | b as usize
            }
            _ => 0,
        };
        for glyph in run.glyphs() {
            if self.all.len() > 200_000 {
                return;
            }
            let transform = props.transform * glyph.transform();
            let origin = transform * Point::ZERO;
            let ex = transform * Point::new(1000., 0.) - origin;
            let rect = match &**glyph {
                interpret::font::Glyph::Outline(g) if !g.outline().is_empty() => {
                    (transform * g.outline()).bounding_box()
                }
                _ => (transform * Rect::new(0., -200., 500., 800.).to_path(0.1)).bounding_box(),
            };
            let at = GlyphAt { origin, rect, ex };
            self.all.push(at);
            if id & TAG != 0 {
                self.runs.entry(id & !TAG).or_default().push(at);
            } else if self.inside(rect.center()) {
                self.untagged_under = true;
            }
        }
    }
    fn draw_image(&mut self, image: Image<'a, '_>, props: ImageDrawProps<'a>) {
        let (w, h) = (image.width() as f64, image.height() as f64);
        let bbox = (props.transform * Rect::new(0., 0., w, h).to_path(0.1)).bounding_box();
        if !self.areas.iter().any(|a| a.overlaps(bbox)) {
            return;
        }
        let Image::Raster(raster) = image else {
            self.images.push((bbox, None));
            return;
        };
        let mut covered = None;
        let mut leaks = 0;
        raster.with_rgba(
            |data, alpha| {
                let (dw, dh) = (data.width(), data.height());
                let (sx, sy) = (w / dw as f64, h / dh as f64);
                let mut out = image::RgbaImage::new(dw, dh);
                for (u, v, px) in out.enumerate_pixels_mut() {
                    let i = (v * dw + u) as usize;
                    let rgb = match &data {
                        ImageData::Rgb(d) => [d.data[i * 3], d.data[i * 3 + 1], d.data[i * 3 + 2]],
                        ImageData::Luma(d) => [d.data[i]; 3],
                    };
                    let a = alpha
                        .as_ref()
                        .filter(|a| a.width == dw && a.height == dh)
                        .map_or(255, |a| a.data[i]);
                    let p =
                        props.transform * Point::new((u as f64 + 0.5) * sx, (v as f64 + 0.5) * sy);
                    *px = if self.areas.iter().any(|r| r.contains(p)) {
                        if a > 0 && rgb.iter().zip(self.fill).any(|(c, f)| c.abs_diff(f) > 8) {
                            leaks += 1;
                        }
                        image::Rgba([self.fill[0], self.fill[1], self.fill[2], 255])
                    } else {
                        image::Rgba([rgb[0], rgb[1], rgb[2], a])
                    };
                }
                covered = Some(out);
            },
            None,
        );
        self.leaks += leaks;
        self.images.push((bbox, covered));
    }
}

/// Render a page with each page-level text run painted in its own tag color.
fn probe<'p>(
    bytes: &[u8],
    page: usize,
    runs: &[crate::document::TextRun],
    areas: &'p [Rect],
    fill: [f32; 3],
) -> Result<Probe<'p>> {
    let mut doc = Document::load_mem(bytes)?;
    let id = *doc
        .get_pages()
        .get(&(page as u32 + 1))
        .context("Page not found")?;
    let tags: HashMap<usize, usize> = runs
        .iter()
        .enumerate()
        .map(|(i, r)| (r.operator_index, i))
        .collect();
    let source = doc.get_and_decode_page_content(id)?;
    let mut operations = Vec::with_capacity(source.operations.len() + tags.len() * 2);
    for (index, op) in source.operations.into_iter().enumerate() {
        if let Some(run) = tags.get(&index) {
            let tag = TAG | run;
            let rgb: Vec<Object> = [(tag >> 16) & 255, (tag >> 8) & 255, tag & 255]
                .into_iter()
                .map(|c| Object::Real(c as f32 / 255.))
                .collect();
            operations.push(Operation::new("rg", rgb.clone()));
            operations.push(Operation::new("RG", rgb));
        }
        operations.push(op);
    }
    let stream = doc.add_object(Stream::new(
        Dictionary::new(),
        Content { operations }.encode()?,
    ));
    let dict = doc.get_dictionary_mut(id)?;
    dict.set("Contents", stream);
    dict.remove(b"Annots");
    let mut resources = crate::content::resources(&doc, id)?;
    if let Ok(cs) = resources
        .get(b"ColorSpace")
        .and_then(|o| doc.dereference(o).map(|(_, o)| o))
        .and_then(Object::as_dict)
    {
        let mut cs = cs.clone();
        cs.remove(b"DefaultRGB");
        resources.set("ColorSpace", cs);
    }
    doc.get_dictionary_mut(id)?.set("Resources", resources);
    let mut tagged = vec![];
    doc.save_to(&mut tagged)?;
    render(&tagged, page, areas, fill)
}

fn render<'p>(bytes: &[u8], page: usize, areas: &'p [Rect], fill: [f32; 3]) -> Result<Probe<'p>> {
    let pdf =
        Pdf::new(bytes.to_vec()).map_err(|e| anyhow::anyhow!("Could not read the page: {e:?}"))?;
    let page = pdf.pages().get(page).context("Page not found")?;
    let cache = interpret::InterpreterCache::new();
    let (w, h) = page.render_dimensions();
    let mut context = interpret::Context::new(
        page.initial_transform(true).to_kurbo(),
        Rect::new(0., 0., w as f64, h as f64),
        &cache,
        page.xref(),
        interpret::InterpreterSettings::default(),
    );
    let mut device = Probe::new(areas, fill);
    interpret::interpret_page(page, &mut context, &mut device);
    ensure!(
        device.all.len() <= 200_000,
        "This page has too many glyphs to redact"
    );
    Ok(device)
}

enum Piece {
    Code(Vec<u8>),
    Number(f64),
}

/// The bytes per character code of a font, when MyPDF can split its strings.
fn code_width(font: &Dictionary) -> Option<usize> {
    match font.get(b"Subtype").and_then(Object::as_name).ok()? {
        b"Type0" => match font.get(b"Encoding").and_then(Object::as_name).ok()? {
            b"Identity-H" => Some(2),
            _ => None,
        },
        _ => Some(1),
    }
}

fn pieces(op: &Operation, width: usize) -> Option<Vec<Piece>> {
    let split = |bytes: &[u8], out: &mut Vec<Piece>| {
        bytes.len().is_multiple_of(width).then(|| {
            out.extend(bytes.chunks(width).map(|c| Piece::Code(c.to_vec())));
        })
    };
    let mut out = vec![];
    match op.operator.as_str() {
        "Tj" | "'" => split(op.operands.first()?.as_str().ok()?, &mut out)?,
        "\"" => split(op.operands.get(2)?.as_str().ok()?, &mut out)?,
        "TJ" => {
            for item in op.operands.first()?.as_array().ok()? {
                match item {
                    Object::String(s, _) => split(s, &mut out)?,
                    Object::Integer(n) => out.push(Piece::Number(*n as f64)),
                    Object::Real(n) => out.push(Piece::Number(*n as f64)),
                    _ => return None,
                }
            }
        }
        _ => return None,
    }
    Some(out)
}

/// The next text-showing operation that continues from where this one ends.
fn continues_at(operations: &[Operation], index: usize) -> Option<usize> {
    for (i, op) in operations.iter().enumerate().skip(index + 1) {
        match op.operator.as_str() {
            "Tj" | "TJ" => return Some(i),
            "Td" | "TD" | "Tm" | "T*" | "'" | "\"" | "BT" | "ET" => return None,
            _ => {}
        }
    }
    None
}

fn precise(
    bytes: &[u8],
    page: usize,
    areas: &[Rect],
    fill: [f32; 3],
    report: &mut Report,
) -> Result<Vec<u8>> {
    let number = page + 1;
    let inside = |p: Point| areas.iter().any(|a| a.contains(p));
    let runs = crate::document::text_runs(bytes, number as u32).with_context(|| {
        format!("Page {number}: its text could not be read. Use Flatten pages.")
    })?;
    let before = probe(bytes, page, &runs, areas, fill)?;
    ensure!(
        !before.untagged_under,
        "Page {number}: text under a marked area is inside a form object, which MyPDF can't edit glyph by glyph. Use Flatten pages."
    );
    let mut doc = crate::content::editable(bytes)?;
    let id = *doc
        .get_pages()
        .get(&(number as u32))
        .context("Page not found")?;
    let fonts = doc.get_page_fonts(id)?;
    let mut operations = doc.get_and_decode_page_content(id)?.operations;
    let run_of: HashMap<usize, usize> = runs
        .iter()
        .enumerate()
        .map(|(i, r)| (r.operator_index, i))
        .collect();
    let mut removed: HashSet<(i64, i64)> = HashSet::new();
    let key = |p: Point| ((p.x * 100.).round() as i64, (p.y * 100.).round() as i64);
    let mut replacements = vec![];
    for (ri, run) in runs.iter().enumerate() {
        let glyphs = before.runs.get(&ri).map(Vec::as_slice).unwrap_or(&[]);
        let op = &operations[run.operator_index];
        if glyphs.is_empty() {
            // Clipping-only text is never drawn, so its position can't be checked.
            let shows =
                pieces(op, 1).is_some_and(|p| p.iter().any(|p| matches!(p, Piece::Code(_))));
            ensure!(
                !shows || run.text.trim().is_empty(),
                "Page {number} has text used only as a clipping path, which can't be checked. Use Flatten pages."
            );
            continue;
        }
        if !glyphs.iter().any(|g| inside(g.rect.center())) {
            continue;
        }
        let font = fonts
            .get(run.font.as_bytes())
            .with_context(|| format!("Page {number}: a font under a marked area is missing"))?;
        let parts = code_width(font)
            .and_then(|w| pieces(op, w))
            .filter(|p| p.iter().filter(|p| matches!(p, Piece::Code(_))).count() == glyphs.len())
            .with_context(|| format!("Page {number}: text under a marked area uses a font encoding MyPDF can't split. Use Flatten pages."))?;
        let next = continues_at(&operations, run.operator_index)
            .and_then(|i| run_of.get(&i))
            .and_then(|r| before.runs.get(r))
            .and_then(|g| g.first())
            .map(|g| g.origin);
        let mut array = vec![];
        let mut text = vec![];
        let flush = |text: &mut Vec<u8>, array: &mut Vec<Object>| {
            if !text.is_empty() {
                array.push(Object::String(
                    std::mem::take(text),
                    StringFormat::Hexadecimal,
                ));
            }
        };
        let mut g = 0;
        for (k, piece) in parts.iter().enumerate() {
            match piece {
                Piece::Number(n) => {
                    flush(&mut text, &mut array);
                    array.push(Object::Real(*n as f32));
                }
                Piece::Code(code) => {
                    let glyph = glyphs[g];
                    if !inside(glyph.rect.center()) {
                        text.extend_from_slice(code);
                    } else {
                        flush(&mut text, &mut array);
                        removed.insert(key(glyph.origin));
                        report.glyphs += 1;
                        // Keep the following glyph where it was: replace this glyph's
                        // advance by an equal adjustment along the text direction.
                        let following = glyphs.get(g + 1).map(|n| n.origin).or(next);
                        if let Some(following) = following {
                            let between: f64 = parts[k + 1..]
                                .iter()
                                .take_while(|p| matches!(p, Piece::Number(_)))
                                .map(|p| match p {
                                    Piece::Number(n) => *n,
                                    Piece::Code(_) => 0.,
                                })
                                .sum();
                            let advance = (following - glyph.origin) + glyph.ex * (between / 1000.);
                            let length = glyph.ex.hypot2();
                            if length > 1e-12 {
                                array.push(Object::Real(
                                    (-1000. * advance.dot(glyph.ex) / length) as f32,
                                ));
                            }
                        }
                    }
                    g += 1;
                }
            }
        }
        flush(&mut text, &mut array);
        let show = Operation::new("TJ", vec![Object::Array(array)]);
        let replacement = match op.operator.as_str() {
            "'" => vec![Operation::new("T*", vec![]), show],
            "\"" => vec![
                Operation::new("Tw", vec![op.operands[0].clone()]),
                Operation::new("Tc", vec![op.operands[1].clone()]),
                Operation::new("T*", vec![]),
                show,
            ],
            _ => vec![show],
        };
        replacements.push((run.operator_index, replacement));
    }
    // Images: page-level image objects under an area get covered pixels.
    let page_images = crate::content::images(bytes, number as u32)?;
    let mut used = vec![false; page_images.len()];
    let mut xobjects: Option<Dictionary> = None;
    for (bbox, covered) in &before.images {
        let Some(covered) = covered else {
            bail!(
                "Page {number}: a masked image under a marked area can't be edited. Use Flatten pages."
            );
        };
        let close = |r: [f32; 4]| {
            (r[0] as f64 - bbox.x0).abs() < 1.
                && (r[1] as f64 - bbox.y0).abs() < 1.
                && (r[2] as f64 - bbox.x1).abs() < 1.
                && (r[3] as f64 - bbox.y1).abs() < 1.
        };
        let Some(k) = (0..page_images.len()).find(|k| !used[*k] && close(page_images[*k].rect))
        else {
            bail!(
                "Page {number}: an image under a marked area is inside a form object. Use Flatten pages."
            );
        };
        used[k] = true;
        let image_id = crate::content::embed_image(&mut doc, covered)?;
        let xo = xobjects.get_or_insert_with(|| {
            crate::content::resources(&doc, id)
                .ok()
                .and_then(|r| r.get(b"XObject").ok().cloned())
                .and_then(|o| {
                    doc.dereference(&o)
                        .ok()
                        .and_then(|(_, o)| o.as_dict().ok().cloned())
                })
                .unwrap_or_default()
        });
        let mut n = 0;
        let name = loop {
            let name = format!("MyPDFRedacted{n}");
            if !xo.has(name.as_bytes()) {
                break name;
            }
            n += 1;
        };
        xo.set(name.as_str(), image_id);
        operations[page_images[k].index].operands = vec![Object::Name(name.into_bytes())];
        report.images += 1;
    }
    replacements.sort_by_key(|(i, _)| std::cmp::Reverse(*i));
    for (index, ops) in replacements {
        operations.splice(index..=index, ops);
    }
    let mut resources = crate::content::resources(&doc, id)?;
    if let Some(xo) = xobjects {
        resources.set("XObject", xo);
    }
    cover(
        &mut doc, id, page, bytes, operations, resources, areas, fill,
    )?;
    report.annotations += remove_annotations(&mut doc, id, page, bytes, areas)?;
    let result = crate::content::serialize(&mut doc)?;
    // Check the result: nothing readable under a box, everything else in place.
    // Annotations are compared apart: those under a box are already gone.
    let after = render(&without_annotations(&result, page)?, page, areas, fill)?;
    ensure!(
        !after.all.iter().any(|g| inside(g.rect.center())) && after.leaks == 0,
        "Page {number}: content under a marked area could not be removed safely. Use Flatten pages."
    );
    let mut expected: Vec<(i64, i64)> = before
        .all
        .iter()
        .map(|g| key(g.origin))
        .filter(|k| !removed.contains(k))
        .collect();
    let mut actual: Vec<(i64, i64)> = after.all.iter().map(|g| key(g.origin)).collect();
    expected.sort_unstable();
    actual.sort_unstable();
    let moved = expected.len() != actual.len()
        || expected
            .iter()
            .zip(&actual)
            .any(|(a, b)| (a.0 - b.0).abs() > 10 || (a.1 - b.1).abs() > 10);
    ensure!(
        !moved,
        "Page {number}: removing the text would move other text on the line. Use Flatten pages."
    );
    Ok(result)
}

/// A copy of the document with one page's annotations left out.
fn without_annotations(bytes: &[u8], page: usize) -> Result<Vec<u8>> {
    let mut doc = Document::load_mem(bytes)?;
    let id = *doc
        .get_pages()
        .get(&(page as u32 + 1))
        .context("Page not found")?;
    doc.get_dictionary_mut(id)?.remove(b"Annots");
    let mut out = vec![];
    doc.save_to(&mut out)?;
    Ok(out)
}

/// Draw the boxes over the page's own content, isolated from its graphics state.
#[allow(clippy::too_many_arguments)]
fn cover(
    doc: &mut Document,
    id: ObjectId,
    page: usize,
    bytes: &[u8],
    mut operations: Vec<Operation>,
    resources: Dictionary,
    areas: &[Rect],
    fill: [f32; 3],
) -> Result<()> {
    let (_, transform) = *crate::annotations::page_geometry(bytes)?
        .get(page)
        .context("Page geometry missing")?;
    let (mut depth, mut lowest, mut in_text) = (0i64, 0i64, false);
    for op in &operations {
        match op.operator.as_str() {
            "q" => depth += 1,
            "Q" => {
                depth -= 1;
                lowest = lowest.min(depth);
            }
            "BT" => in_text = true,
            "ET" => in_text = false,
            _ => {}
        }
    }
    let open = (1 - lowest) as usize;
    let mut all = vec![Operation::new("q", vec![]); open];
    all.append(&mut operations);
    if in_text {
        all.push(Operation::new("ET", vec![]));
    }
    all.extend(vec![
        Operation::new("Q", vec![]);
        (open as i64 + depth) as usize
    ]);
    all.push(Operation::new("q", vec![]));
    all.push(Operation::new(
        "cm",
        transform.iter().map(|v| Object::Real(*v as f32)).collect(),
    ));
    all.push(Operation::new(
        "rg",
        fill.iter().map(|v| Object::Real(*v)).collect(),
    ));
    for a in areas {
        all.push(Operation::new(
            "re",
            vec![
                Object::Real(a.x0 as f32),
                Object::Real(a.y0 as f32),
                Object::Real(a.width() as f32),
                Object::Real(a.height() as f32),
            ],
        ));
    }
    all.push(Operation::new("f", vec![]));
    all.push(Operation::new("Q", vec![]));
    let stream = doc.add_object(Stream::new(
        Dictionary::new(),
        Content { operations: all }.encode()?,
    ));
    let dict = doc.get_dictionary_mut(id)?;
    dict.set("Resources", resources);
    dict.set("Contents", stream);
    Ok(())
}

/// Remove annotations (and form widgets) that overlap an area.
fn remove_annotations(
    doc: &mut Document,
    id: ObjectId,
    page: usize,
    bytes: &[u8],
    areas: &[Rect],
) -> Result<usize> {
    let (_, transform) = *crate::annotations::page_geometry(bytes)?
        .get(page)
        .context("Page geometry missing")?;
    let annots = match doc.get_dictionary(id)?.get(b"Annots") {
        Ok(Object::Reference(r)) => doc.get_object(*r)?.as_array()?.clone(),
        Ok(Object::Array(a)) => a.clone(),
        _ => return Ok(0),
    };
    let mut kept = vec![];
    let mut removed = HashSet::new();
    for item in annots {
        let overlaps = item
            .as_reference()
            .ok()
            .and_then(|r| doc.get_dictionary(r).ok())
            .or_else(|| item.as_dict().ok())
            .and_then(|d| d.get(b"Rect").ok())
            .and_then(|r| r.as_array().ok())
            .and_then(|r| {
                let v: Vec<f32> = r.iter().filter_map(|v| v.as_float().ok()).collect();
                (v.len() == 4)
                    .then(|| crate::content::displayed_rect([v[0], v[1], v[2], v[3]], transform))
            })
            .is_some_and(|r| {
                let r = Rect::new(r[0] as f64, r[1] as f64, r[2] as f64, r[3] as f64);
                areas.iter().any(|a| a.overlaps(r))
            });
        if overlaps {
            if let Ok(r) = item.as_reference() {
                removed.insert(r);
            }
        } else {
            kept.push(item);
        }
    }
    let count = removed.len();
    if count == 0 {
        return Ok(0);
    }
    doc.get_dictionary_mut(id)?.set("Annots", kept);
    // Removed widgets leave the form; a parent field left without widgets loses its value.
    if let Ok(root) = doc.trailer.get(b"Root").and_then(Object::as_reference)
        && let Ok(Object::Reference(form)) = doc
            .get_dictionary(root)
            .and_then(|c| c.get(b"AcroForm"))
            .cloned()
        && let Ok(fields) = doc
            .get_dictionary(form)
            .and_then(|f| f.get(b"Fields"))
            .and_then(Object::as_array)
            .cloned()
    {
        let fields: Vec<Object> = fields
            .into_iter()
            .filter(|f| f.as_reference().is_ok_and(|r| !removed.contains(&r)))
            .collect();
        doc.get_dictionary_mut(form)?.set("Fields", fields);
    }
    let parents: Vec<(ObjectId, Vec<Object>)> = doc
        .objects
        .iter()
        .filter_map(|(oid, o)| {
            let kids = o.as_dict().ok()?.get(b"Kids").ok()?.as_array().ok()?;
            kids.iter()
                .any(|k| k.as_reference().is_ok_and(|r| removed.contains(&r)))
                .then(|| {
                    (
                        *oid,
                        kids.iter()
                            .filter(|k| k.as_reference().is_ok_and(|r| !removed.contains(&r)))
                            .cloned()
                            .collect(),
                    )
                })
        })
        .collect();
    for (parent, kids) in parents {
        let dict = doc.get_dictionary_mut(parent)?;
        if kids.is_empty() {
            dict.remove(b"V");
        }
        dict.set("Kids", kids);
    }
    Ok(count)
}

/// Replace the page with an image of it, boxes burned in.
fn flatten(
    bytes: &[u8],
    page: usize,
    areas: &[Rect],
    fill: [f32; 3],
    report: &mut Report,
) -> Result<Vec<u8>> {
    const SCALE: f32 = 200. / 72.;
    let mut doc = crate::content::editable(bytes)?;
    let id = *doc
        .get_pages()
        .get(&(page as u32 + 1))
        .context("Page not found")?;
    let geometry = crate::annotations::page_geometry(bytes)?;
    let (size, transform) = *geometry.get(page).context("Page geometry missing")?;
    // Annotations are kept apart from the image, unless they are under a box.
    let ([w, h], mut rgba) =
        crate::viewer::render_page(&without_annotations(bytes, page)?, page, SCALE)?;
    let (sx, sy) = (w as f64 / size[2] as f64, h as f64 / size[3] as f64);
    let color = fill.map(|c| (c * 255.).round() as u8);
    for a in areas {
        let (x0, x1) = (
            (a.x0 * sx).floor().max(0.) as usize,
            ((a.x1 * sx).ceil() as usize).min(w),
        );
        let (y0, y1) = (
            (a.y0 * sy).floor().max(0.) as usize,
            ((a.y1 * sy).ceil() as usize).min(h),
        );
        for y in y0..y1 {
            for x in x0..x1 {
                let i = (y * w + x) * 4;
                rgba[i..i + 4].copy_from_slice(&[color[0], color[1], color[2], 255]);
            }
        }
    }
    image_page(&mut doc, id, size, transform, [w, h], rgba)?;
    report.annotations += remove_annotations(&mut doc, id, page, bytes, areas)?;
    report.flattened += 1;
    crate::content::serialize(&mut doc)
}

/// Make a page show only this rendering of itself, at its displayed size.
fn image_page(
    doc: &mut Document,
    id: ObjectId,
    size: [f32; 4],
    transform: [f64; 6],
    [w, h]: [usize; 2],
    mut rgba: Vec<u8>,
) -> Result<()> {
    // The render is opaque on white; drop any premultiplied alpha.
    for px in rgba.chunks_mut(4) {
        px[3] = 255;
    }
    let image = image::RgbaImage::from_raw(w as u32, h as u32, rgba).context("Render failed")?;
    let image_id = crate::content::embed_image(doc, &image)?;
    let operations = vec![
        Operation::new("q", vec![]),
        Operation::new(
            "cm",
            transform.iter().map(|v| Object::Real(*v as f32)).collect(),
        ),
        Operation::new(
            "cm",
            vec![
                Object::Real(size[2]),
                0.into(),
                0.into(),
                Object::Real(-size[3]),
                0.into(),
                Object::Real(size[3]),
            ],
        ),
        Operation::new("Do", vec![Object::Name(b"MyPDFPage".to_vec())]),
        Operation::new("Q", vec![]),
    ];
    let stream = doc.add_object(Stream::new(
        Dictionary::new(),
        Content { operations }.encode()?,
    ));
    let mut xo = Dictionary::new();
    xo.set("MyPDFPage", image_id);
    let mut resources = Dictionary::new();
    resources.set("XObject", xo);
    let dict = doc.get_dictionary_mut(id)?;
    dict.set("Resources", resources);
    dict.set("Contents", stream);
    Ok(())
}

/// Turn every page into an image of itself, with annotations and form fields
/// burned in, so nothing on it can be edited. The result can still be signed.
pub fn flatten_document(bytes: &[u8], dpi: f32) -> Result<(Vec<u8>, usize)> {
    ensure!((72.0..=600.0).contains(&dpi), "Choose 72–600 dpi");
    ensure!(
        !crate::document::has_signatures(&Document::load_mem(bytes)?),
        "Flattening rewrites every page, so the existing signatures would no longer verify. Flatten the unsigned PDF, then sign it."
    );
    let mut doc = crate::content::editable(bytes)?;
    let geometry = crate::annotations::page_geometry(bytes)?;
    let pages: Vec<ObjectId> = doc.get_pages().into_values().collect();
    for (page, id) in pages.iter().enumerate() {
        let (size, transform) = *geometry.get(page).context("Page geometry missing")?;
        // Rendering includes each annotation's and field's appearance.
        let (pixels, rgba) = crate::viewer::render_page(bytes, page, dpi / 72.)?;
        image_page(&mut doc, *id, size, transform, pixels, rgba)?;
        doc.get_dictionary_mut(*id)?.remove(b"Annots");
    }
    // Nothing interactive remains: forms, document scripts and open actions.
    if let Ok(root) = doc.trailer.get(b"Root").and_then(Object::as_reference) {
        let names = {
            let catalog = doc.get_dictionary_mut(root)?;
            for key in [b"AcroForm".as_slice(), b"OpenAction", b"AA"] {
                catalog.remove(key);
            }
            catalog.get(b"Names").ok().cloned()
        };
        match names {
            Some(Object::Reference(r)) => {
                if let Ok(names) = doc.get_dictionary_mut(r) {
                    names.remove(b"JavaScript");
                }
            }
            Some(Object::Dictionary(mut names)) => {
                names.remove(b"JavaScript");
                doc.get_dictionary_mut(root)?.set("Names", names);
            }
            _ => {}
        }
    }
    Ok((crate::content::serialize(&mut doc)?, pages.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_split_by_code_width_and_tj_numbers_stay_in_place() {
        let op = Operation::new(
            "TJ",
            vec![Object::Array(vec![
                Object::string_literal("ab"),
                Object::Integer(-120),
                Object::String(vec![0, 1, 0, 2], StringFormat::Hexadecimal),
            ])],
        );
        let one = pieces(&op, 1).unwrap();
        assert_eq!(one.len(), 7);
        let two = pieces(&op, 2).unwrap();
        assert_eq!(two.len(), 4, "two 2-byte codes, a number, one more code");
        assert!(
            pieces(
                &Operation::new("Tj", vec![Object::string_literal("abc")]),
                2
            )
            .is_none()
        );
        let ops = [
            Operation::new("Tj", vec![]),
            Operation::new("Tc", vec![]),
            Operation::new("Tj", vec![]),
            Operation::new("Td", vec![]),
            Operation::new("Tj", vec![]),
        ];
        assert_eq!(continues_at(&ops, 0), Some(2));
        assert_eq!(continues_at(&ops, 2), None);
    }
}
