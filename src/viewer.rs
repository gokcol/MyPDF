//! Portable document worker and searchable glyph geometry for the desktop viewer.
use anyhow::{Context as _, Result, bail};
use hayro::hayro_interpret::{self as interpret, Device, font::Glyph};
use hayro::hayro_syntax::Pdf;
use hayro::kurbo::{BezPath, Rect, Shape};
use interpret::{
    BlendMode, ClipPath, DrawMode, DrawProps, Image, ImageDrawProps, SoftMask, TransformExt,
};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc,
};

pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_RASTER_PIXELS: f32 = 12_000_000.0;
const MAX_INDEX_BYTES: usize = 64 * 1024 * 1024;
const MAX_PAGE_GLYPHS: usize = 100_000;

#[derive(Clone, Copy, Debug)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug)]
pub struct GlyphBox {
    pub range: Range<usize>,
    pub rect: [f32; 4],
}

/// Portable text styling for copying editable text to another application.
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub family: String,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub color: [u8; 3],
}
impl Default for TextStyle {
    fn default() -> Self {
        Self {
            family: "sans-serif".into(),
            size: 12.,
            bold: false,
            italic: false,
            color: [0; 3],
        }
    }
}

#[derive(Clone, Debug)]
pub struct TextSpan {
    pub range: Range<usize>,
    pub style: TextStyle,
}

/// A visual text line in page coordinates (points, with the origin at top left).
#[derive(Clone, Debug)]
pub struct TextLine {
    pub range: Range<usize>,
    pub left: f32,
    pub baseline: f32,
}

/// An independently verified glyph expansion and its original PDF mapping.
/// Ranges refer to the corrected `PageText.text`, preserving safe correspondence
/// with raw text objects that still use the PDF's original ToUnicode values.
#[derive(Clone, Debug)]
pub struct TextCorrection {
    pub range: Range<usize>,
    pub original: String,
}

#[derive(Clone, Default, Debug)]
pub struct PageText {
    pub text: String,
    pub glyphs: Vec<GlyphBox>,
    pub spans: Vec<TextSpan>,
    pub lines: Vec<TextLine>,
    pub corrections: Vec<TextCorrection>,
    pub limited: bool,
}

#[derive(Clone, Debug)]
pub struct SearchHit {
    pub page: usize,
    pub range: Range<usize>,
    pub rects: Vec<[f32; 4]>,
    pub snippet: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
}

impl PageText {
    /// Map Unicode lowercase expansions back to original UTF-8 character spans.
    pub fn find(&self, query: &str, page: usize) -> Vec<SearchHit> {
        self.find_with(query, page, SearchOptions::default())
    }
    pub fn find_with(&self, query: &str, page: usize, options: SearchOptions) -> Vec<SearchHit> {
        if query.trim().is_empty() {
            return vec![];
        }
        let needle = query.split_whitespace().collect::<Vec<_>>().join(" ");
        let needle = if options.case_sensitive {
            needle
        } else {
            needle.to_lowercase()
        };
        let mut folded = String::new();
        let mut mapping = Vec::new();
        for (start, c) in self.text.char_indices() {
            if c.is_whitespace() {
                if !folded.ends_with(' ') {
                    folded.push(' ');
                    mapping.push(start..start + c.len_utf8());
                }
                continue;
            }
            let chars = if options.case_sensitive {
                c.to_string()
            } else {
                c.to_lowercase().collect()
            };
            for lower in chars.chars() {
                for _ in 0..lower.len_utf8() {
                    mapping.push(start..start + c.len_utf8());
                }
                folded.push(lower);
            }
        }
        let chars: Vec<_> = self.text.char_indices().collect();
        let word =
            |c: char| c.is_alphanumeric() || c == '_' || ('\u{300}'..='\u{36f}').contains(&c);
        folded
            .match_indices(&needle)
            .filter(|(start, matched)| {
                if !options.whole_word {
                    return true;
                }
                let a = mapping[*start].start;
                let b = mapping[*start + matched.len() - 1].end;
                !self.text[..a].chars().next_back().is_some_and(word)
                    && !self.text[b..].chars().next().is_some_and(word)
            })
            .take(1000)
            .map(|(start, matched)| {
                let range = mapping[start].start..mapping[start + matched.len() - 1].end;
                let first_glyph = self.glyphs.partition_point(|g| g.range.end <= range.start);
                let rects = self.glyphs[first_glyph..]
                    .iter()
                    .take_while(|g| g.range.start < range.end)
                    .map(|g| g.rect)
                    .collect();
                let match_start = chars.partition_point(|(i, _)| *i < range.start);
                let match_end = chars.partition_point(|(i, _)| *i < range.end);
                let mut first = match_start.saturating_sub(24);
                let mut end = (match_end + 50).min(chars.len());
                if options.whole_word {
                    // Truncated context must not create false whole-word matches
                    // when the UI highlights the snippet. Keep the same adjacent-
                    // character rule as search, including punctuation query edges,
                    // and never trim any part of the genuine match.
                    while first < match_start && first > 0 && word(chars[first - 1].1) {
                        first += 1;
                    }
                    while end > match_end && end < chars.len() && word(chars[end].1) {
                        end -= 1;
                    }
                }
                let snippet: String = chars[first..end]
                    .iter()
                    .map(|(_, c)| *c)
                    .collect::<String>()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                SearchHit {
                    page,
                    range,
                    rects,
                    snippet,
                }
            })
            .collect()
    }
}

struct SearchRequest {
    generation: u64,
    pages: Vec<Option<Arc<PageText>>>,
    query: String,
    options: SearchOptions,
}
/// One bounded, cancellable search queue per tab. No PDF or platform objects cross threads.
pub struct SearchWorker {
    pub results: mpsc::Receiver<(u64, Vec<SearchHit>)>,
    requests: mpsc::SyncSender<SearchRequest>,
    generation: Arc<AtomicU64>,
    alive: Arc<AtomicBool>,
}
impl Drop for SearchWorker {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
    }
}
impl SearchWorker {
    pub fn new(wake: impl Fn() + Send + 'static) -> Self {
        let (requests, rx) = mpsc::sync_channel::<SearchRequest>(1);
        let (tx, results) = mpsc::sync_channel(1);
        let generation = Arc::new(AtomicU64::new(0));
        let current = generation.clone();
        let alive = Arc::new(AtomicBool::new(true));
        let running = alive.clone();
        std::thread::spawn(move || {
            while let Ok(request) = rx.recv() {
                if !running.load(Ordering::Relaxed) {
                    break;
                }
                let mut hits = vec![];
                for (page, text) in request.pages.iter().enumerate() {
                    if current.load(Ordering::Relaxed) != request.generation
                        || !running.load(Ordering::Relaxed)
                    {
                        break;
                    }
                    if let Some(text) = text {
                        hits.extend(text.find_with(&request.query, page, request.options));
                    }
                    if hits.len() >= 10_000 {
                        hits.truncate(10_000);
                        break;
                    }
                }
                if current.load(Ordering::Relaxed) == request.generation
                    && running.load(Ordering::Relaxed)
                {
                    let _ = tx.send((request.generation, hits));
                    wake();
                }
            }
        });
        Self {
            results,
            requests,
            generation,
            alive,
        }
    }
    pub fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }
    pub fn request(&self, pages: Vec<Option<Arc<PageText>>>, query: String) -> bool {
        self.request_with(pages, query, SearchOptions::default())
    }
    pub fn request_with(
        &self,
        pages: Vec<Option<Arc<PageText>>>,
        query: String,
        options: SearchOptions,
    ) -> bool {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.requests
            .try_send(SearchRequest {
                generation,
                pages,
                query,
                options,
            })
            .is_ok()
    }
}

#[derive(Clone)]
struct FontStyle {
    family: String,
    bold: bool,
    italic: bool,
}

impl FontStyle {
    fn from_glyph(glyph: &interpret::font::OutlineGlyph) -> Self {
        let data = glyph.font_data();
        let face = data
            .as_ref()
            .and_then(|data| ttf_parser::Face::parse(data.data.as_ref().as_ref(), 0).ok());
        let family = face.as_ref().and_then(|face| {
            [
                ttf_parser::name_id::TYPOGRAPHIC_FAMILY,
                ttf_parser::name_id::FAMILY,
            ]
            .into_iter()
            .find_map(|id| {
                face.names()
                    .into_iter()
                    .filter(|name| name.name_id == id)
                    .find_map(|name| name.to_string().filter(|s| !s.trim().is_empty()))
            })
        });
        let name = glyph.font_name().unwrap_or("");
        let name = name
            .split_once('+')
            .filter(|(prefix, _)| prefix.len() == 6)
            .map_or(name, |(_, rest)| rest);
        let lower = name.to_ascii_lowercase();
        let family = family.unwrap_or_else(|| {
            if name.starts_with('.') || name.is_empty() {
                if data.as_ref().is_some_and(|data| data.is_monospace) {
                    "monospace".into()
                } else if data.as_ref().is_some_and(|data| data.is_serif) {
                    "serif".into()
                } else {
                    "sans-serif".into()
                }
            } else if name.starts_with("TimesNewRoman") {
                "Times New Roman".into()
            } else if name.starts_with("Arial") {
                "Arial".into()
            } else {
                name.split('-').next().unwrap_or(name).to_owned()
            }
        });
        // A PDF font name is untrusted and is copied into each temporary glyph
        // style; cap it once before caching to keep that staging memory bounded.
        let family: String = family.chars().take(256).collect();
        Self {
            family,
            bold: data
                .as_ref()
                .and_then(|data| data.weight)
                .is_some_and(|weight| weight >= 600)
                || face.as_ref().is_some_and(|face| face.is_bold())
                || lower.contains("bold")
                || lower.contains("demi"),
            italic: data.as_ref().is_some_and(|data| data.is_italic)
                || face.as_ref().is_some_and(|face| face.is_italic())
                || lower.contains("italic")
                || lower.contains("oblique"),
        }
    }
}

struct ExtractedGlyph {
    text: String,
    original: Option<String>,
    rect: [f32; 4],
    origin: hayro::kurbo::Point,
    end: hayro::kurbo::Point,
    direction: hayro::kurbo::Vec2,
    font: Option<u128>,
    style: TextStyle,
}

struct PaintedRun {
    glyphs: Range<usize>,
    positions: Vec<(hayro::kurbo::Point, hayro::kurbo::Point)>,
}

#[derive(Default)]
struct TextDevice {
    result: PageText,
    previous: Option<(hayro::kurbo::Point, hayro::kurbo::Vec2, f32)>,
    glyph_styles: Vec<TextStyle>,
    fonts: HashMap<u128, FontStyle>,
    recoveries: HashMap<u128, crate::text_recovery::LigatureRecovery>,
    painted: HashMap<(u64, i32, i32), Vec<PaintedRun>>,
    last_fill: Option<Range<usize>>,
    visited: usize,
}

impl TextDevice {
    /// Quartz can synthesize bold by drawing the same text several times, each
    /// shifted by a fraction of a point. Those paint passes are one logical run.
    /// Compare both text/font and every glyph's placement; repeated words on
    /// another line or in another column must remain separate selections.
    fn duplicate_run(
        &mut self,
        glyphs: &[ExtractedGlyph],
        hash: u64,
        stroked: bool,
    ) -> Option<Range<usize>> {
        let first = &glyphs[0];
        let cell = (
            (first.origin.x / 2.).floor() as i32,
            (first.origin.y / 2.).floor() as i32,
        );
        for x in cell.0.saturating_sub(1)..=cell.0.saturating_add(1) {
            for y in cell.1.saturating_sub(1)..=cell.1.saturating_add(1) {
                let Some(runs) = self.painted.get(&(hash, x, y)) else {
                    continue;
                };
                for run in runs {
                    if run.glyphs.len() != glyphs.len() {
                        continue;
                    }
                    let mut offset = false;
                    let same = glyphs
                        .iter()
                        .zip(run.glyphs.clone())
                        .zip(&run.positions)
                        .all(|((glyph, index), (origin, end))| {
                            let old_style = &self.glyph_styles[index];
                            let tolerance = (glyph.style.size as f64 * 0.075).clamp(0.05, 1.);
                            let shift = (glyph.origin - *origin).hypot();
                            offset |= shift > 0.02;
                            self.result
                                .text
                                .get(self.result.glyphs[index].range.clone())
                                == Some(glyph.text.as_str())
                                && old_style.family == glyph.style.family
                                && old_style.italic == glyph.style.italic
                                && (old_style.color == glyph.style.color
                                    // FillStroke emits consecutive callbacks for
                                    // the same run even with different paint colors.
                                    || (stroked
                                        && self.last_fill.as_ref() == Some(&run.glyphs)
                                        && shift <= 0.001
                                        && (glyph.end - *end).hypot() <= 0.001))
                                && (old_style.size - glyph.style.size).abs() < 0.01
                                && shift <= tolerance
                                && (glyph.end - *end).hypot() <= tolerance
                        });
                    if same {
                        if offset || stroked {
                            for index in run.glyphs.clone() {
                                self.glyph_styles[index].bold = true;
                            }
                        }
                        return Some(run.glyphs.clone());
                    }
                }
            }
        }
        None
    }

    fn append(&mut self, glyph: &ExtractedGlyph) {
        let mut new_line = self.result.lines.is_empty();
        if let Some((previous, direction, size)) = self.previous {
            let delta = glyph.origin - previous;
            let across = delta.x * direction.y - delta.y * direction.x;
            new_line = across.abs() > f64::from(size.max(glyph.style.size)) * 0.35
                || direction.dot(glyph.direction) < 0.9;
            if new_line {
                self.result.text.push('\n');
            } else {
                let along = delta.dot(direction);
                if along > f64::from(glyph.style.size) * 0.25
                    && !self.result.text.ends_with(char::is_whitespace)
                    && !glyph.text.starts_with(char::is_whitespace)
                {
                    self.result.text.push(' ');
                }
            }
        }
        let start = self.result.text.len();
        self.result.text.push_str(&glyph.text);
        let end = self.result.text.len();
        if let Some(original) = &glyph.original {
            self.result.corrections.push(TextCorrection {
                range: start..end,
                original: original.clone(),
            });
        }
        self.result.glyphs.push(GlyphBox {
            range: start..end,
            rect: glyph.rect,
        });
        self.glyph_styles.push(glyph.style.clone());
        if new_line {
            self.result.lines.push(TextLine {
                range: start..end,
                left: glyph.origin.x as f32,
                baseline: glyph.origin.y as f32,
            });
        } else if let Some(line) = self.result.lines.last_mut() {
            line.range.end = end;
            line.left = line.left.min(glyph.origin.x as f32);
        }
        self.previous = Some((glyph.end, glyph.direction, glyph.style.size));
    }

    fn finish(mut self) -> PageText {
        for (glyph, style) in self.result.glyphs.iter().zip(self.glyph_styles) {
            match self.result.spans.last_mut() {
                Some(span) if span.style == style => span.range.end = glyph.range.end,
                _ => self.result.spans.push(TextSpan {
                    range: glyph.range.clone(),
                    style,
                }),
            }
        }
        self.result
    }
}

impl<'a> Device<'a> for TextDevice {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {
        self.last_fill = None;
    }
    fn push_clip_path(&mut self, _: &ClipPath) {
        self.last_fill = None;
    }
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {
        self.last_fill = None;
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {
        self.last_fill = None;
    }
    fn pop_clip(&mut self) {
        self.last_fill = None;
    }
    fn pop_transparency_group(&mut self) {
        self.last_fill = None;
    }
    fn draw_glyph_run(
        &mut self,
        run: &interpret::font::GlyphRun<'_, 'a>,
        props: DrawProps<'a>,
        mode: &DrawMode,
    ) {
        let color = match props.paint {
            interpret::Paint::Color(color) => {
                let [r, g, b, _] = color.to_rgba().to_rgba8();
                [r, g, b]
            }
            _ => [0; 3],
        };
        let mut glyphs = Vec::new();
        let mut run_bytes = 0;
        let mut signature = std::collections::hash_map::DefaultHasher::new();
        for glyph in run.glyphs() {
            self.visited += 1;
            if self.visited > MAX_PAGE_GLYPHS * 8
                || self.result.glyphs.len() + glyphs.len() >= MAX_PAGE_GLYPHS
                || self.result.text.len() >= 2_000_000
            {
                self.result.limited = true;
                break;
            }
            let Some(unicode) = glyph.as_unicode() else {
                continue;
            };
            let mut text = match unicode {
                interpret::hayro_cmap::BfString::Char(c) => c.to_string(),
                interpret::hayro_cmap::BfString::String(s) => s,
            };
            let original = if let Glyph::Outline(g) = &**glyph {
                let recovery = self
                    .recoveries
                    .entry(g.font_cache_key())
                    .or_insert_with(|| crate::text_recovery::LigatureRecovery::for_font(g));
                recovery
                    .recover(g)
                    .filter(|recovered| *recovered != text)
                    .map(|recovered| std::mem::replace(&mut text, recovered.to_owned()))
            } else {
                None
            };
            if text.is_empty() {
                continue;
            }
            // Include reconstructed separators in the same byte budget.
            run_bytes += text.len().saturating_add(1) + original.as_ref().map_or(0, String::len);
            if self.result.text.len().saturating_add(run_bytes) > 2_000_000 {
                self.result.limited = true;
                break;
            }
            let transform = props.transform * glyph.transform();
            let origin = transform * hayro::kurbo::Point::ZERO;
            let advance = match &**glyph {
                Glyph::Outline(g) => g.advance_width().unwrap_or(500.0) as f64,
                Glyph::Type3(_) => 500.0,
            };
            let end = transform * hayro::kurbo::Point::new(advance, 0.0);
            let basis = transform * hayro::kurbo::Point::new(0.0, 1000.0) - origin;
            let axis = transform * hayro::kurbo::Point::new(1000.0, 0.0) - origin;
            let size = basis.hypot() as f32;
            let fallback_bounds = || {
                (transform * Rect::new(0.0, -200.0, advance.max(200.0), 800.0).to_path(0.1))
                    .bounding_box()
            };
            let rect = match &**glyph {
                Glyph::Outline(g) => {
                    // Decoding an outline allocates and visits its curves. Reuse
                    // it for the empty check and bounds on this indexing pass.
                    let outline = g.outline();
                    if outline.is_empty() {
                        fallback_bounds()
                    } else {
                        (transform * outline).bounding_box()
                    }
                }
                _ => fallback_bounds(),
            };
            if !rect.is_finite()
                || !origin.is_finite()
                || !end.is_finite()
                || !size.is_finite()
                || size <= 0.
                || axis.hypot() <= 0.
            {
                continue;
            }
            let mut style = TextStyle {
                size,
                color,
                ..TextStyle::default()
            };
            let font = if let Glyph::Outline(g) = &**glyph {
                let key = g.font_cache_key();
                let metadata = self
                    .fonts
                    .entry(key)
                    .or_insert_with(|| FontStyle::from_glyph(g));
                style.family.clone_from(&metadata.family);
                style.bold = metadata.bold;
                style.italic = metadata.italic;
                Some(key)
            } else {
                None
            };
            text.hash(&mut signature);
            font.hash(&mut signature);
            glyphs.push(ExtractedGlyph {
                text,
                original,
                rect: [
                    rect.x0 as f32,
                    rect.y0 as f32,
                    rect.x1 as f32,
                    rect.y1 as f32,
                ],
                origin,
                end,
                direction: axis / axis.hypot(),
                font,
                style,
            });
        }
        if glyphs.is_empty() {
            self.last_fill = None;
            return;
        }
        let hash = signature.finish();
        let can_deduplicate = glyphs.iter().all(|glyph| glyph.font.is_some());
        if can_deduplicate
            && let Some(original) =
                self.duplicate_run(&glyphs, hash, matches!(mode, DrawMode::Stroke(_)))
        {
            self.last_fill = matches!(mode, DrawMode::Fill(_)).then_some(original);
            return;
        }
        let first = &glyphs[0];
        let cell = (
            (first.origin.x / 2.).floor() as i32,
            (first.origin.y / 2.).floor() as i32,
        );
        let start = self.result.glyphs.len();
        for glyph in &glyphs {
            self.append(glyph);
        }
        self.last_fill =
            matches!(mode, DrawMode::Fill(_)).then_some(start..self.result.glyphs.len());
        if can_deduplicate {
            self.painted
                .entry((hash, cell.0, cell.1))
                .or_default()
                .push(PaintedRun {
                    glyphs: start..self.result.glyphs.len(),
                    positions: glyphs
                        .iter()
                        .map(|glyph| (glyph.origin, glyph.end))
                        .collect(),
                });
        }
    }
}

pub fn extract_page<'a>(
    page: &hayro::hayro_syntax::page::Page<'a>,
    cache: &interpret::InterpreterCache<'a>,
) -> PageText {
    let (w, h) = page.render_dimensions();
    let mut context = interpret::Context::new(
        page.initial_transform(true).to_kurbo(),
        Rect::new(0.0, 0.0, w as f64, h as f64),
        cache,
        page.xref(),
        crate::render_fonts::settings(),
    );
    let mut device = TextDevice::default();
    interpret::interpret_page(page, &mut context, &mut device);
    device.finish()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct RasterKey {
    pub page: usize,
    pub scale: u16,
    pub thumbnail: bool,
}
impl RasterKey {
    pub fn factor(self) -> f32 {
        self.scale as f32 / 100.0
    }
}

pub enum Event {
    Loaded {
        bytes: Arc<[u8]>,
        pages: Vec<PageSize>,
        signed: bool,
        annotations: Vec<crate::annotations::Annotation>,
        capabilities: crate::annotations::Capabilities,
        editing_warning: Option<String>,
    },
    Indexed {
        page: usize,
        text: PageText,
    },
    IndexFinished {
        limited: bool,
    },
    Raster {
        key: RasterKey,
        size: [usize; 2],
        rgba: Vec<u8>,
    },
    RasterFailed {
        key: RasterKey,
        error: String,
    },
    /// The file opens only with a password; these are its protected bytes.
    PasswordNeeded(Arc<[u8]>),
    Failed(String),
}

/// The parser and its non-Send caches stay on their worker thread.
pub struct Worker {
    pub events: mpsc::Receiver<Event>,
    commands: mpsc::SyncSender<RasterKey>,
    alive: Arc<AtomicBool>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
    }
}
impl Worker {
    pub fn open(path: PathBuf, wake: impl Fn() + Send + 'static) -> Self {
        Self::spawn(Some(path), None, wake)
    }
    pub fn from_bytes(bytes: Arc<[u8]>, wake: impl Fn() + Send + 'static) -> Self {
        Self::spawn(None, Some(bytes), wake)
    }
    fn spawn(
        path: Option<PathBuf>,
        bytes: Option<Arc<[u8]>>,
        wake: impl Fn() + Send + 'static,
    ) -> Self {
        let (tx, events) = mpsc::sync_channel(2);
        let (commands, rx) = mpsc::sync_channel(8);
        let alive = Arc::new(AtomicBool::new(true));
        let running = alive.clone();
        std::thread::spawn(move || {
            let send = |event| {
                let _ = tx.send(event);
                wake();
            };
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let bytes = match bytes {
                    Some(bytes) => bytes,
                    None => read_input(path.as_deref().context("Missing PDF input")?)?,
                };
                run_worker(bytes, &rx, &running, &send)
            }));
            match outcome {
                Ok(Err(e)) => send(Event::Failed(format!("{e:#}"))),
                Err(_) => send(Event::Failed(
                    "This PDF could not be processed. Other documents are still available.".into(),
                )),
                _ => {}
            }
        });
        Self {
            events,
            commands,
            alive,
        }
    }
    pub fn request(&self, key: RasterKey) -> bool {
        self.commands.try_send(key).is_ok()
    }
}

fn read_input(path: &Path) -> Result<Arc<[u8]>> {
    let metadata = std::fs::metadata(path).context("Could not open the file")?;
    if !metadata.is_file() {
        bail!("Choose a PDF file.");
    }
    if metadata.len() > MAX_FILE_BYTES {
        bail!("This milestone supports PDFs up to 256 MB.");
    }
    use std::io::Read;
    let mut buffer = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut buffer)?;
    if buffer.len() as u64 > MAX_FILE_BYTES {
        bail!("The PDF exceeds the 256 MB limit.");
    }
    Ok(buffer.into())
}
fn run_worker(
    bytes: Arc<[u8]>,
    rx: &mpsc::Receiver<RasterKey>,
    alive: &AtomicBool,
    send: &impl Fn(Event),
) -> Result<()> {
    if crate::security::needs_password(&bytes) {
        send(Event::PasswordNeeded(bytes));
        return Ok(());
    }
    let (annotations, capabilities, rendering, editing_warning) =
        match lopdf::Document::load_mem(&bytes) {
            Ok(mut doc) => {
                let caps = crate::annotations::inspect(&doc);
                let annotations = crate::annotations::load(&doc);
                let rendering = if caps.rewritable() {
                    crate::annotations::clean_render_copy(&bytes, &mut doc)?
                } else {
                    image_render_copy(&bytes, &mut doc)
                };
                (annotations, caps, rendering, None)
            }
            Err(e) => (
                vec![],
                crate::annotations::Capabilities {
                    signed: bytes.windows(10).any(|w| w == b"/ByteRange"),
                    certified: true,
                    encrypted: false,
                    needs_signing_preparation: false,
                    unlocked: false,
                },
                bytes.to_vec(),
                Some(format!("Editing is unavailable for this PDF: {e}")),
            ),
        };
    let pdf = open_render_pdf(rendering, &bytes)?;
    if pdf.pages().is_empty() || pdf.pages().len() > 10_000 {
        bail!("The PDF must contain between 1 and 10,000 pages.");
    }
    let pages: Vec<_> = pdf
        .pages()
        .iter()
        .map(|p| {
            let (width, height) = p.render_dimensions();
            PageSize { width, height }
        })
        .collect();
    if pages.iter().any(|p| {
        !p.width.is_finite()
            || !p.height.is_finite()
            || p.width < 1.0
            || p.height < 1.0
            || p.width > 100_000.0
            || p.height > 100_000.0
    }) {
        bail!("The PDF has unsupported page dimensions.");
    }
    // A badge reports presence only, never cryptographic validity or trust.
    let signed = capabilities.signed;
    send(Event::Loaded {
        bytes,
        pages: pages.clone(),
        signed,
        annotations,
        capabilities,
        editing_warning,
    });
    let render_cache = hayro::RenderCache::new();
    let text_cache = interpret::InterpreterCache::new();
    let mut indexed = 0;
    let mut index_bytes = 0;
    let mut index_limited = false;
    let mut index_finished = false;
    while alive.load(Ordering::Relaxed) {
        if !index_finished && (indexed == pdf.pages().len() || index_bytes >= MAX_INDEX_BYTES) {
            index_limited |= indexed < pdf.pages().len();
            send(Event::IndexFinished {
                limited: index_limited,
            });
            index_finished = true;
        }
        let request = if !index_finished {
            rx.try_recv().ok()
        } else {
            match rx.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(key) => Some(key),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(_) => None,
            }
        };
        if let Some(key) = request {
            let Some(page) = pdf.pages().get(key.page) else {
                continue;
            };
            let size = pages[key.page];
            let scale = key
                .factor()
                .min((MAX_RASTER_PIXELS / (size.width * size.height)).sqrt())
                .min(8192.0 / size.width.max(size.height))
                .max(0.001);
            if size.width * scale < 1.0 || size.height * scale < 1.0 {
                send(Event::RasterFailed {
                    key,
                    error: "Page dimensions exceed the render limit.".into(),
                });
                continue;
            }
            let raster = hayro::render(
                page,
                &render_cache,
                &crate::render_fonts::settings(),
                &hayro::RenderSettings::default(),
                &hayro::PixmapSettings {
                    x_scale: scale,
                    y_scale: scale,
                    bg_color: hayro::vello_cpu::color::palette::css::WHITE,
                },
            );
            send(Event::Raster {
                key,
                size: [raster.width() as usize, raster.height() as usize],
                rgba: raster.data_as_u8_slice().to_vec(),
            });
        } else if !index_finished {
            let text = extract_page(&pdf.pages()[indexed], &text_cache);
            index_bytes += text.text.len()
                + text.glyphs.len() * std::mem::size_of::<GlyphBox>()
                + text.lines.len() * std::mem::size_of::<TextLine>()
                + text
                    .corrections
                    .iter()
                    .map(|correction| {
                        std::mem::size_of::<TextCorrection>() + correction.original.len()
                    })
                    .sum::<usize>()
                + text
                    .spans
                    .iter()
                    .map(|span| std::mem::size_of::<TextSpan>() + span.style.family.len())
                    .sum::<usize>();
            index_limited |= text.limited;
            send(Event::Indexed {
                page: indexed,
                text,
            });
            indexed += 1;
        }
    }
    Ok(())
}

/// Bound pixel density in the worker; callers may still display larger pages.
pub fn raster_key(page: usize, size: PageSize, desired: f32, thumbnail: bool) -> RasterKey {
    let scale = desired
        .min((MAX_RASTER_PIXELS / (size.width * size.height)).sqrt())
        .min(8192.0 / size.width.max(size.height))
        .clamp(0.01, 8.0);
    RasterKey {
        page,
        scale: (scale * 100.0).ceil() as u16,
        thumbnail,
    }
}

/// Render one displayed page to premultiplied RGBA, outside any worker.
pub fn render_page(bytes: &[u8], page: usize, scale: f32) -> Result<([usize; 2], Vec<u8>)> {
    let rendering = lopdf::Document::load_mem(bytes)
        .map(|mut doc| image_render_copy(bytes, &mut doc))
        .unwrap_or_else(|_| bytes.to_vec());
    let pdf = open_render_pdf(rendering, bytes)?;
    let page = pdf.pages().get(page).context("Page not found")?;
    let (width, height) = page.render_dimensions();
    let scale = scale
        .min((MAX_RASTER_PIXELS / (width * height)).sqrt())
        .min(8192.0 / width.max(height))
        .max(0.001);
    let raster = hayro::render(
        page,
        &hayro::RenderCache::new(),
        &crate::render_fonts::settings(),
        &hayro::RenderSettings::default(),
        &hayro::PixmapSettings {
            x_scale: scale,
            y_scale: scale,
            bg_color: hayro::vello_cpu::color::palette::css::WHITE,
        },
    );
    Ok((
        [raster.width() as usize, raster.height() as usize],
        raster.data_as_u8_slice().to_vec(),
    ))
}

fn image_render_copy(bytes: &[u8], doc: &mut lopdf::Document) -> Vec<u8> {
    // Display preparation must never prevent a PDF from opening. Preserve the
    // source on malformed/unsupported appearance metadata or serialization errors.
    crate::annotations::interpolated_image_render_copy(bytes, doc)
        .unwrap_or_else(|_| bytes.to_vec())
}

fn open_render_pdf(rendering: Vec<u8>, original: &[u8]) -> Result<Pdf> {
    Pdf::new(rendering)
        .or_else(|_| Pdf::new(original.to_vec()))
        .map_err(|error| anyhow::anyhow!("Could not read this PDF ({error:?})"))
}

/// Exact original bytes, atomically published to a fresh path. Existing files are never replaced.
pub fn save_copy(destination: &Path, bytes: &[u8]) -> Result<()> {
    crate::write_new(destination, bytes)
}

#[cfg(test)]
mod render_copy_tests {
    use super::*;

    #[test]
    fn unsupported_derived_pdf_falls_back_to_original_without_changing_it() {
        let source = crate::convert::blank(200., 100.).unwrap();
        let original = source.clone();
        let parsed = open_render_pdf(b"invalid viewing copy".to_vec(), &source).unwrap();
        assert_eq!(parsed.pages().len(), 1);
        assert_eq!(parsed.pages()[0].render_dimensions(), (200., 100.));
        assert_eq!(source, original);
        assert!(open_render_pdf(b"invalid viewing copy".to_vec(), b"invalid original").is_err());
    }
}
