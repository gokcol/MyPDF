//! Portable document worker and searchable glyph geometry for the desktop viewer.
use anyhow::{Context as _, Result, bail};
use hayro::hayro_interpret::{self as interpret, Device, font::Glyph};
use hayro::hayro_syntax::Pdf;
use hayro::kurbo::{BezPath, Rect, Shape};
use interpret::{
    BlendMode, ClipPath, DrawMode, DrawProps, Image, ImageDrawProps, SoftMask, TransformExt,
};
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

#[derive(Clone, Default, Debug)]
pub struct PageText {
    pub text: String,
    pub glyphs: Vec<GlyphBox>,
    pub limited: bool,
}

#[derive(Clone, Debug)]
pub struct SearchHit {
    pub page: usize,
    pub range: Range<usize>,
    pub rects: Vec<[f32; 4]>,
    pub snippet: String,
}

impl PageText {
    /// Map Unicode lowercase expansions back to original UTF-8 character spans.
    pub fn find(&self, query: &str, page: usize) -> Vec<SearchHit> {
        if query.trim().is_empty() {
            return vec![];
        }
        let needle = query
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
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
            for lower in c.to_lowercase() {
                for _ in 0..lower.len_utf8() {
                    mapping.push(start..start + c.len_utf8());
                }
                folded.push(lower);
            }
        }
        let chars: Vec<_> = self.text.char_indices().collect();
        folded
            .match_indices(&needle)
            .take(1000)
            .map(|(start, matched)| {
                let range = mapping[start].start..mapping[start + matched.len() - 1].end;
                let first_glyph = self.glyphs.partition_point(|g| g.range.end <= range.start);
                let rects = self.glyphs[first_glyph..]
                    .iter()
                    .take_while(|g| g.range.start < range.end)
                    .map(|g| g.rect)
                    .collect();
                let first = chars
                    .partition_point(|(i, _)| *i < range.start)
                    .saturating_sub(24);
                let end = (chars.partition_point(|(i, _)| *i < range.end) + 50).min(chars.len());
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
                        hits.extend(text.find(&request.query, page));
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
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.requests
            .try_send(SearchRequest {
                generation,
                pages,
                query,
            })
            .is_ok()
    }
}

struct TextDevice {
    result: PageText,
    previous: Option<hayro::kurbo::Point>,
}
impl<'a> Device<'a> for TextDevice {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
    fn draw_glyph_run(
        &mut self,
        run: &interpret::font::GlyphRun<'_, 'a>,
        props: DrawProps<'a>,
        _: &DrawMode,
    ) {
        for glyph in run.glyphs() {
            if self.result.glyphs.len() >= MAX_PAGE_GLYPHS || self.result.text.len() >= 2_000_000 {
                self.result.limited = true;
                break;
            }
            let Some(unicode) = glyph.as_unicode() else {
                continue;
            };
            let text = match unicode {
                interpret::hayro_cmap::BfString::Char(c) => c.to_string(),
                interpret::hayro_cmap::BfString::String(s) => s,
            };
            let transform = props.transform * glyph.transform();
            let origin = transform * hayro::kurbo::Point::ZERO;
            let advance = match &**glyph {
                Glyph::Outline(g) => g.advance_width().unwrap_or(500.0) as f64,
                Glyph::Type3(_) => 500.0,
            };
            let end = transform * hayro::kurbo::Point::new(advance, 0.0);
            let basis = transform * hayro::kurbo::Point::new(0.0, 1000.0) - origin;
            if let Some(previous) = self.previous {
                let distance = (origin - previous).hypot();
                if distance > basis.hypot() * 1.4 {
                    self.result.text.push('\n');
                } else if distance > basis.hypot() * 0.25
                    && !self.result.text.ends_with(char::is_whitespace)
                    && !text.starts_with(char::is_whitespace)
                {
                    self.result.text.push(' ');
                }
            }
            let start = self.result.text.len();
            self.result.text.push_str(&text);
            let rect = match &**glyph {
                Glyph::Outline(g) if !g.outline().is_empty() => {
                    (transform * g.outline()).bounding_box()
                }
                _ => (transform * Rect::new(0.0, -200.0, advance.max(200.0), 800.0).to_path(0.1))
                    .bounding_box(),
            };
            if rect.is_finite() {
                self.result.glyphs.push(GlyphBox {
                    range: start..self.result.text.len(),
                    rect: [
                        rect.x0 as f32,
                        rect.y0 as f32,
                        rect.x1 as f32,
                        rect.y1 as f32,
                    ],
                });
            }
            self.previous = Some(end);
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
        interpret::InterpreterSettings::default(),
    );
    let mut device = TextDevice {
        result: PageText::default(),
        previous: None,
    };
    interpret::interpret_page(page, &mut context, &mut device);
    device.result
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
    let (annotations, capabilities, rendering, editing_warning) =
        match lopdf::Document::load_mem(&bytes) {
            Ok(mut doc) => {
                let caps = crate::annotations::inspect(&doc);
                let annotations = crate::annotations::load(&doc);
                let rendering = if caps.rewritable() {
                    crate::annotations::clean_render_copy(&bytes, &mut doc)?
                } else {
                    bytes.to_vec()
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
    let pdf = Pdf::new(rendering).map_err(|e| anyhow::anyhow!("Could not read this PDF ({e:?}). Password-protected PDFs are not supported in this milestone."))?;
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
                &interpret::InterpreterSettings::default(),
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
            index_bytes += text.text.len() + text.glyphs.len() * std::mem::size_of::<GlyphBox>();
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

/// Exact original bytes, atomically published to a fresh path. Existing files are never replaced.
pub fn save_copy(destination: &Path, bytes: &[u8]) -> Result<()> {
    crate::write_new(destination, bytes)
}
