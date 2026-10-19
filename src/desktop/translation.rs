//! Private, read-only translation. Only language preferences are persisted.
use super::*;
use engine::{Availability, Event, Language, Operation};
use mypdf_milestone0::{ocr, translation as engine};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

const MAX_TEXT_BYTES: usize = 500_000;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct Preferences {
    source: String,
    target: String,
    favorites: Vec<String>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            source: "en".into(),
            target: "tr".into(),
            favorites: vec!["en".into(), "tr".into(), "de".into()],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct DocumentKey {
    tab: u64,
    revision: u64,
    page: usize,
}
fn document_key(tab: &Tab) -> DocumentKey {
    DocumentKey {
        tab: tab.id,
        revision: tab.session.as_ref().map_or(0, |s| s.revision()),
        page: tab.page,
    }
}

// No Debug or serialization: these fields contain private document text.
struct Source {
    key: DocumentKey,
    label: String,
    text: String,
    anchor: Option<[f32; 4]>,
    source_object: Option<String>,
    attached_note: Option<String>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum JobKind {
    Languages,
    Check,
    Download,
    Translate,
}
struct ActiveJob {
    kind: JobKind,
    job: engine::Job,
    since: Instant,
}
struct PageJob {
    key: DocumentKey,
    label: String,
    cancellation: Arc<ocr::Cancellation>,
    result: mpsc::Receiver<Result<String, String>>,
}
impl Drop for PageJob {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

pub(super) struct State {
    pub open: bool,
    preferences: Preferences,
    languages: Vec<Language>,
    show_languages: bool,
    language_filter: String,
    availability: Option<Availability>,
    source: Option<Source>,
    results: Vec<Option<String>>,
    completed: bool,
    progress: (usize, usize),
    job: Option<ActiveJob>,
    page_job: Option<PageJob>,
    auto_translate: bool,
    message: Option<String>,
    error: Option<String>,
    unsupported: Option<String>,
}

impl State {
    pub fn new() -> Self {
        let root = mypdf_milestone0::assets::data_directory();
        Self {
            open: false,
            preferences: preferences::load(&root, "translation.json").unwrap_or_default(),
            languages: vec![],
            show_languages: false,
            language_filter: String::new(),
            availability: None,
            source: None,
            results: vec![],
            completed: false,
            progress: (0, 0),
            job: None,
            page_job: None,
            auto_translate: false,
            message: None,
            error: None,
            unsupported: None,
        }
    }
    fn persist(&mut self) {
        if preferences::save(
            &mypdf_milestone0::assets::data_directory(),
            "translation.json",
            &self.preferences,
        )
        .is_err()
        {
            self.error = Some(
                "The language preference could not be saved. Translation is still available."
                    .into(),
            );
        }
    }
    fn name(&self, id: &str) -> String {
        self.languages
            .iter()
            .find(|l| l.id == id)
            .map(|l| l.name.clone())
            .unwrap_or_else(|| match base_language(id) {
                "en" => "English".into(),
                "tr" => "Turkish".into(),
                "de" => "German".into(),
                _ => id.into(),
            })
    }
    fn cancel(&mut self) {
        self.job = None;
        self.page_job = None;
        self.auto_translate = false;
        self.results.clear();
        self.completed = false;
        self.progress = (0, 0);
    }
    fn clear_document(&mut self) {
        // Language-only work can finish when a user changes the open document.
        if self
            .job
            .as_ref()
            .is_some_and(|j| j.kind == JobKind::Translate)
        {
            self.job = None;
        }
        self.page_job = None;
        self.source = None;
        self.results.clear();
        self.completed = false;
        self.progress = (0, 0);
        self.auto_translate = false;
    }
    fn start(&mut self, operation: Operation, kind: JobKind) {
        self.job = None;
        self.error = None;
        match engine::start(operation) {
            Ok(job) => {
                self.job = Some(ActiveJob {
                    kind,
                    job,
                    since: Instant::now(),
                })
            }
            Err(error) => {
                self.error = Some(error);
                self.auto_translate = false;
            }
        }
    }
    fn open(&mut self) {
        self.open = true;
        self.unsupported = engine::available().err();
        if self.unsupported.is_none() && self.job.is_none() {
            if self.languages.is_empty() {
                self.start(Operation::Languages, JobKind::Languages);
            } else if self.availability.is_none() {
                self.check();
            }
        }
    }
    fn check(&mut self) {
        self.availability = None;
        if self.preferences.source == self.preferences.target {
            self.error = Some("Choose two different languages.".into());
            self.auto_translate = false;
            return;
        }
        self.start(
            Operation::Check {
                source: self.preferences.source.clone(),
                target: self.preferences.target.clone(),
            },
            JobKind::Check,
        );
    }
    fn pair_changed(&mut self) {
        self.cancel();
        if let Some(source) = &mut self.source {
            source.attached_note = None;
        }
        self.message = None;
        self.persist();
        self.check();
    }
    fn translate(&mut self) {
        if self.availability != Some(Availability::Installed) {
            self.auto_translate = true;
            self.check();
            return;
        }
        let Some(source) = &self.source else {
            return;
        };
        if source.text.trim().is_empty() {
            return;
        }
        let chunks = chunks(&source.text);
        self.results = vec![None; chunks.len()];
        self.progress = (0, chunks.len());
        self.completed = false;
        self.auto_translate = false;
        self.message = None;
        self.start(
            Operation::Translate {
                source: self.preferences.source.clone(),
                target: self.preferences.target.clone(),
                texts: chunks,
            },
            JobKind::Translate,
        );
    }
    fn download(&mut self) {
        self.auto_translate = false;
        // This operation has no text field: document content never enters the
        // model-download/consent path, even when a selection is open.
        self.start(
            Operation::Download {
                source: self.preferences.source.clone(),
                target: self.preferences.target.clone(),
            },
            JobKind::Download,
        );
    }
    fn output(&self) -> String {
        self.results
            .iter()
            .filter_map(|s| s.as_deref())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
    fn receive(&mut self, kind: JobKind, event: Event) -> bool {
        match event {
            Event::Languages(mut languages) => {
                languages.sort_by(|a, b| a.name.cmp(&b.name));
                languages.dedup_by(|a, b| a.id == b.id);
                self.languages = languages;
                for id in [&mut self.preferences.source, &mut self.preferences.target] {
                    if let Some(lang) = self.languages.iter().find(|l| l.id == *id).or_else(|| {
                        self.languages
                            .iter()
                            .find(|l| base_language(&l.id) == base_language(id))
                    }) {
                        *id = lang.id.clone();
                    }
                }
            }
            Event::Availability(value) => self.availability = Some(value),
            Event::Progress { completed, total } => self.progress = (completed, total),
            Event::Translated { index, text } => {
                if kind == JobKind::Translate
                    && let Some(slot) = self.results.get_mut(index)
                {
                    *slot = Some(text);
                }
            }
            Event::Failed(error) => {
                self.error = Some(error);
                self.auto_translate = false;
                if kind == JobKind::Translate {
                    self.results.clear();
                    self.completed = false;
                    // A language pack may have been removed since the last check.
                    self.availability = None;
                }
                return true;
            }
            Event::Cancelled => {
                self.message = Some("Translation cancelled.".into());
                self.results.clear();
                self.completed = false;
                return true;
            }
            Event::Finished => {
                if kind == JobKind::Translate {
                    self.completed =
                        !self.results.is_empty() && self.results.iter().all(Option::is_some);
                    if !self.completed {
                        self.error = Some("The translation did not finish. Try again.".into());
                        self.results.clear();
                    }
                }
                return true;
            }
        }
        false
    }
}

fn base_language(id: &str) -> &str {
    id.split(['-', '_']).next().unwrap_or(id)
}

/// Preserve Unicode and keep related sentences together within modest batches.
fn chunks(text: &str) -> Vec<String> {
    let mut rest = text.trim();
    let mut out = vec![];
    while !rest.is_empty() {
        let hard = rest.char_indices().nth(1800).map_or(rest.len(), |(i, _)| i);
        let end = if hard == rest.len() {
            hard
        } else {
            rest[..hard]
                .char_indices()
                .rev()
                .find(|(i, c)| *i > hard / 2 && (*c == '\n' || *c == '.' || *c == '!' || *c == '?'))
                .map(|(i, c)| i + c.len_utf8())
                .or_else(|| {
                    rest[..hard]
                        .rfind(char::is_whitespace)
                        .filter(|i| *i > hard / 2)
                })
                .unwrap_or(hard)
        };
        out.push(rest[..end].trim().to_owned());
        rest = rest[end..].trim_start();
    }
    out
}

/// Match the current page's saved appearance without changing the document.
/// Hayro extracts and renders visible annotation appearances itself, so emitting
/// the current annotations once avoids both missing edits and duplicate text.
/// Unchanged documents keep their original bytes, including digital signatures.
fn translation_page_snapshot(
    source: &[u8],
    annotations: Option<&[mypdf_milestone0::annotations::Annotation]>,
) -> anyhow::Result<Vec<u8>> {
    match annotations {
        Some(annotations) => mypdf_milestone0::annotations::export(source, annotations),
        None => Ok(source.to_vec()),
    }
}

fn translation_page_text(bytes: &[u8], page: usize) -> anyhow::Result<String> {
    let pdf = hayro::hayro_syntax::Pdf::new(bytes.to_vec())
        .map_err(|_| anyhow::anyhow!("Cannot read this page."))?;
    let page = pdf
        .pages()
        .get(page)
        .ok_or_else(|| anyhow::anyhow!("Page not found."))?;
    let text = viewer::extract_page(page, &hayro::hayro_interpret::InterpreterCache::new());
    anyhow::ensure!(
        !text.limited,
        "Page text exceeds the extraction limit. Select a smaller area instead."
    );
    Ok(reflow_translation_page(&text))
}

struct TranslationLine {
    text: String,
    rect: Option<[f32; 4]>,
    height: f32,
}

/// PDF text has visual line breaks rather than reliable paragraph metadata.
/// Join only adjacent, similarly sized lines in the same column. Preserve the
/// extraction order, headings, list boundaries and uncertain layout; this does
/// not affect selectable text, inline drafts, or the document itself.
fn reflow_translation_page(page: &PageText) -> String {
    let mut offset = 0;
    let lines = page
        .text
        .split_inclusive('\n')
        .map(|raw| {
            let start = offset;
            offset += raw.len();
            let begin = page.glyphs.partition_point(|g| g.range.end <= start);
            let mut rect: Option<[f32; 4]> = None;
            let mut height = 0_f32;
            for glyph in page.glyphs[begin..]
                .iter()
                .take_while(|g| g.range.start < offset)
            {
                let r = glyph.rect;
                if !r.iter().all(|v| v.is_finite()) || r[2] <= r[0] || r[3] <= r[1] {
                    continue;
                }
                // Space glyphs use a fallback em box, which is taller than visible
                // letters and would make ordinary wrapped lines look inconsistent.
                if page
                    .text
                    .get(glyph.range.clone())
                    .is_none_or(|s| s.trim().is_empty())
                {
                    continue;
                }
                rect = Some(rect.map_or(r, |old| {
                    [
                        old[0].min(r[0]),
                        old[1].min(r[1]),
                        old[2].max(r[2]),
                        old[3].max(r[3]),
                    ]
                }));
                height = height.max(r[3] - r[1]);
            }
            TranslationLine {
                text: raw.trim().to_owned(),
                rect,
                height,
            }
        })
        .collect::<Vec<_>>();
    reflow_translation_lines(&lines)
}

fn translation_list_start(text: &str) -> bool {
    let Some(first) = text.split_whitespace().next() else {
        return false;
    };
    if matches!(first, "•" | "◦" | "▪" | "‣" | "-" | "–" | "—" | "*") {
        return true;
    }
    let marker = first.trim_start_matches('(').trim_end_matches(['.', ')']);
    marker.len() < first.len()
        && !marker.is_empty()
        && marker.len() <= 4
        && (marker.chars().all(|c| c.is_ascii_digit())
            || marker.len() == 1 && marker.chars().all(|c| c.is_ascii_alphabetic()))
}

fn translation_soft_wrap(a: &TranslationLine, b: &TranslationLine) -> bool {
    let (Some(ar), Some(br)) = (a.rect, b.rect) else {
        return false;
    };
    let h = a.height.min(b.height);
    if h <= 0. || a.height.max(b.height) > h * 1.4 {
        return false;
    }
    let gap = br[1] - ar[3];
    if br[1] <= ar[1] + h * 0.5
        || gap < -h * 0.2
        || gap > h * 1.3
        || (ar[0] - br[0]).abs() > h * 2.
        || ar[0].max(br[0]) >= ar[2].min(br[2])
        || ar[3] - ar[1] > a.height * 1.8
        || br[3] - br[1] > b.height * 1.8
        || translation_list_start(&b.text)
    {
        return false;
    }
    let heading = |line: &TranslationLine, other: [f32; 4]| {
        if translation_list_start(&line.text) || line.text.ends_with(['.', '!', '?', ';', ',']) {
            return false;
        }
        let letters: Vec<_> = line.text.chars().filter(|c| c.is_alphabetic()).collect();
        line.text.ends_with(':')
            || letters.len() > 1 && letters.iter().all(|c| c.is_uppercase())
            || line.text.chars().next().is_some_and(char::is_uppercase)
                && line.text.split_whitespace().count() <= 8
                && line
                    .rect
                    .is_some_and(|r| r[2] - r[0] < (other[2] - other[0]) * 0.65)
    };
    !heading(a, br) && !heading(b, ar)
}

fn reflow_translation_lines(lines: &[TranslationLine]) -> String {
    let mut text = String::new();
    let mut previous: Option<&TranslationLine> = None;
    for line in lines {
        if line.text.is_empty() {
            previous = None;
            if !text.is_empty() && !text.ends_with("\n\n") {
                text.push_str("\n\n");
            }
            continue;
        }
        if let Some(previous) = previous {
            if translation_soft_wrap(previous, line) {
                text.push(' ');
            } else if translation_list_start(&line.text) || translation_list_start(&previous.text) {
                text.push('\n');
            } else {
                text.push_str("\n\n");
            }
        }
        text.push_str(&line.text);
        previous = Some(line);
    }
    text.trim().to_owned()
}

fn reflow_translation_ocr(lines: &[ocr::Line]) -> String {
    let lines = lines
        .iter()
        .map(|line| TranslationLine {
            text: line.text.trim().to_owned(),
            rect: Some(line.rect),
            height: line.rect[3] - line.rect[1],
        })
        .collect::<Vec<_>>();
    reflow_translation_lines(&lines)
}

impl Viewer {
    fn current_translation_selection(&self, ctx: &egui::Context) -> Option<String> {
        if self.home.visible {
            return None;
        }
        let tab = self.tabs.get(self.active)?;
        let key = document_key(tab);
        self.content
            .selected_translation_text(ctx, (key.tab, key.revision, key.page))
            .or_else(|| {
                if !matches!(
                    self.editor.mode,
                    editing::Mode::Read | editing::Mode::Annotate
                ) {
                    return None;
                }
                let selection = self.editor.text_selection?;
                let tab = self
                    .tabs
                    .get(self.active)
                    .filter(|t| t.id == selection.tab && t.page == selection.page)?;
                selection.text(tab)
            })
    }
    pub(super) fn open_translation_panel(&mut self) {
        self.translation.open();
        self.inspector = true;
    }

    pub(super) fn translate_selection(&mut self, ctx: &egui::Context) {
        let text = self.current_translation_selection(ctx);
        if let Some(text) = text {
            self.translate_text(ctx, text, "Selected text".into());
        } else {
            self.translation.open();
            self.inspector = true;
            self.translation.message = Some("Select text on a page, then choose Translate selected text. You can also translate the current page.".into());
        }
    }
    pub(super) fn translate_text(&mut self, ctx: &egui::Context, text: String, label: String) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        if self.home.visible {
            return;
        }
        let key = document_key(tab);
        let active = (key.tab, key.revision, key.page);
        let inline = self
            .content
            .selected_translation_text(ctx, active)
            .as_deref()
            == Some(text.as_str());
        let (anchor, source_object) = if inline {
            (
                self.content.translation_anchor(ctx, active),
                self.content.translation_source_object(ctx, active),
            )
        } else {
            let anchor = self
                .editor
                .text_selection
                .filter(|s| {
                    s.tab == key.tab
                        && s.page == key.page
                        && s.text(tab).as_deref() == Some(text.as_str())
                })
                .and_then(|s| s.rects(tab).first().copied());
            (anchor, None)
        };
        self.translation.cancel();
        self.translation.clear_document();
        self.translation.error = None;
        self.translation.message = None;
        self.translation.open = true;
        self.inspector = true;
        if text.len() > MAX_TEXT_BYTES {
            self.translation.error =
                Some("This text is too large to translate at once. Select a smaller area.".into());
        } else if text.trim().is_empty() {
            self.translation.message =
                Some("No text was found. Use Read scanned page for local OCR.".into());
        } else {
            self.translation.source = Some(Source {
                key,
                label: format!("{label} · Page {}", key.page + 1),
                text,
                anchor,
                source_object,
                attached_note: None,
            });
            self.translation.auto_translate = true;
        }
        self.translation.open();
        if self.translation.job.is_none()
            && self.translation.unsupported.is_none()
            && self.translation.source.is_some()
        {
            self.translation.translate();
        }
        ctx.request_repaint();
    }
    pub(super) fn set_translation_anchor(&mut self, anchor: [f32; 4], object: Option<String>) {
        if let Some(source) = &mut self.translation.source {
            source.anchor = Some(anchor);
            source.source_object = object;
        }
    }
    pub(super) fn translate_page(&mut self, ctx: &egui::Context) {
        self.start_translation_page(ctx, false);
    }
    fn start_translation_page(&mut self, ctx: &egui::Context, scan: bool) {
        if self.home.visible {
            return;
        }
        let Some(tab) = self
            .tabs
            .get(self.active)
            .filter(|t| t.error.is_none() && !t.pages.is_empty())
        else {
            return;
        };
        let Some(bytes) = tab
            .session
            .as_ref()
            .map(|s| s.source.clone())
            .or_else(|| tab.bytes.clone())
        else {
            return;
        };
        // Clone only the current annotation state; never carry undo history or
        // mutate the Session to create the private, temporary reading copy.
        let annotations = tab
            .session
            .as_ref()
            .filter(|s| s.revision() > 0)
            .map(|s| s.annotations.clone());
        let key = document_key(tab);
        let label = format!(
            "{} · Page {}",
            if scan { "Scanned page" } else { "Page text" },
            key.page + 1
        );
        self.translation.cancel();
        self.translation.clear_document();
        self.translation.message = None;
        self.translation.error = None;
        self.translation.open();
        self.inspector = true;
        let cancellation = Arc::new(ocr::Cancellation::default());
        let cancel = cancellation.clone();
        let (tx, result) = mpsc::channel();
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let read = || -> anyhow::Result<String> {
                cancel.check()?;
                let snapshot = translation_page_snapshot(&bytes, annotations.as_deref())?;
                cancel.check()?;
                let text = if scan {
                    reflow_translation_ocr(&ocr::read_page_cancellable(
                        &snapshot,
                        key.page,
                        &cancel,
                        &|| {},
                    )?)
                } else {
                    translation_page_text(&snapshot, key.page)?
                };
                cancel.check()?;
                anyhow::ensure!(
                    text.len() <= MAX_TEXT_BYTES,
                    "This page has too much text. Select a smaller area instead."
                );
                Ok(text)
            };
            let output = read().map_err(|_| {
                if cancel.is_cancelled() {
                    "Reading cancelled.".into()
                } else {
                    "Could not read the complete page text. Try a smaller selection or local OCR."
                        .into()
                }
            });
            let _ = tx.send(output);
            wake.request_repaint();
        });
        self.translation.page_job = Some(PageJob {
            key,
            label,
            cancellation,
            result,
        });
    }
    pub(super) fn translation_languages(&mut self, ctx: &egui::Context) {
        self.translation.open();
        self.translation.show_languages = true;
        self.inspector = true;
        ctx.request_repaint();
    }
    pub(super) fn translation_tick(&mut self, ctx: &egui::Context) {
        let current = self
            .tabs
            .get(self.active)
            .filter(|_| !self.home.visible)
            .map(document_key);
        let source_key = self
            .translation
            .source
            .as_ref()
            .map(|s| s.key)
            .or_else(|| self.translation.page_job.as_ref().map(|j| j.key));
        if source_key.is_some() && source_key != current {
            self.translation.clear_document();
            self.translation.message =
                Some("The page changed. Select text or translate this page.".into());
        }
        if let Some(job) = self.translation.page_job.as_ref()
            && let Ok(result) = job.result.try_recv()
        {
            let key = job.key;
            let label = job.label.clone();
            self.translation.page_job = None;
            match result {
                Ok(text) if !text.trim().is_empty() => {
                    self.translation.source = Some(Source { key, label, text, anchor: None, source_object: None, attached_note: None });
                    self.translation.auto_translate = true;
                    if self.translation.job.is_none() && self.translation.unsupported.is_none() { self.translation.translate(); }
                }
                Ok(_) => self.translation.message = Some("No selectable text on this page. Read scanned page uses local OCR without changing your PDF.".into()),
                Err(error) => self.translation.error = Some(error),
            }
        }
        if let Some(job) = &self.translation.job {
            let kind = job.kind;
            let events: Vec<_> = std::iter::from_fn(|| job.job.try_recv().ok())
                .take(64)
                .collect();
            for event in events {
                let success = matches!(event, Event::Finished);
                if self.translation.receive(kind, event) {
                    self.translation.job = None;
                    if success {
                        match kind {
                            JobKind::Languages => self.translation.check(),
                            JobKind::Download => {
                                self.translation.message = Some(
                                    "Language download finished. Translate when you are ready."
                                        .into(),
                                );
                                self.translation.check();
                            }
                            JobKind::Check
                                if self.translation.availability
                                    == Some(Availability::Installed)
                                    && self.translation.auto_translate =>
                            {
                                self.translation.translate()
                            }
                            JobKind::Check => self.translation.auto_translate = false,
                            _ => {}
                        }
                    }
                    break;
                }
            }
        }
        if self.translation.job.is_some() || self.translation.page_job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
    pub(super) fn translation_panel(&mut self, ctx: &egui::Context) {
        let mut translate_page = false;
        let mut translate_selection = false;
        let mut read_scan = false;
        let mut attach_note = None;
        let mut open_note = None;
        let cropping = self.workbench.crop.is_some();
        let has_page = self.active_document_ready() && !self.home.visible;
        let can_annotate = self
            .tabs
            .get(self.active)
            .and_then(|t| t.session.as_ref())
            .is_some_and(|s| s.editable);
        let has_selection = self.current_translation_selection(ctx).is_some();
        let state = &mut self.translation;
        egui::SidePanel::right("translation_panel").default_width(340.).width_range(300.0..=480.0).resizable(true)
            .frame(egui::Frame::new().fill(theme::paper()).inner_margin(16.)).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui::title(ui, "Translate", 20.);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let close = ui.add(ActionButton::new(Icon::Close, "")).on_hover_text("Close translation");
                    close.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Close translation"));
                    if close.clicked() { state.open = false; state.cancel(); state.clear_document(); }
                });
            });
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(has_selection, ActionButton::new(Icon::TextArea,"Selection")).clicked() { translate_selection = true; }
                if ui.add_enabled(has_page, ActionButton::new(Icon::Document,"Translate page")).clicked() { translate_page = true; }
            });
            ui.horizontal(|ui| { pill(ui, "On-device", theme::accent()); ui.small("No cloud fallback"); });
            #[cfg(target_os="windows")]
            ui.small("CTranslate2 · Offline language packs");
            ui.add_space(8.);
            egui::ScrollArea::vertical().id_salt("translation_scroll").show(ui, |ui| {
                if let Some(message) = &state.unsupported { ui.label(message); ui.separator(); }
                let old_pair = (state.preferences.source.clone(), state.preferences.target.clone());
                let available = state.unsupported.is_none() && !state.languages.is_empty();
                ui.add_enabled_ui(available, |ui| {
                    ui.columns(2, |columns| {
                        let from = ui::caption(&mut columns[0], "FROM");
                        language_picker(&mut columns[0], "translation_from", &mut state.preferences.source, &state.languages, &state.preferences.favorites).labelled_by(from.id);
                        let to = ui::caption(&mut columns[1], "TO");
                        language_picker(&mut columns[1], "translation_to", &mut state.preferences.target, &state.languages, &state.preferences.favorites).labelled_by(to.id);
                    });
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("quick_translation_pairs").selected_text("Quick pairs").width(140.).show_ui(ui, |ui| {
                            for (from,to,label) in [("en","tr","English → Turkish"),("tr","en","Turkish → English"),("en","de","English → German"),("de","en","German → English")] {
                                if ui.button(label).clicked() {
                                    state.preferences.source = state.languages.iter().find(|l| base_language(&l.id)==from).map_or(from.into(),|l| l.id.clone());
                                    state.preferences.target = state.languages.iter().find(|l| base_language(&l.id)==to).map_or(to.into(),|l| l.id.clone());
                                    ui.close();
                                }
                            }
                        });
                        if ui.add(ActionButton::new(Icon::Swap, "Swap")).clicked() { std::mem::swap(&mut state.preferences.source, &mut state.preferences.target); }
                    });
                });
                if old_pair != (state.preferences.source.clone(), state.preferences.target.clone()) { state.pair_changed(); }
                ui.add_space(6.);
                match state.availability {
                    Some(Availability::Installed) => { ui.label(RichText::new("Ready offline").color(theme::accent())); }
                    Some(Availability::DownloadRequired) => {
                        ui.label("Download this language pair to translate offline.");
                        if ui.add_enabled(state.job.is_none(), ActionButton::new(Icon::Import,"Download languages…").primary()).clicked() { state.download(); }
                    }
                    Some(Availability::Unsupported) => { ui.label("This language pair is not supported on this device. Choose another pair; nothing will be uploaded."); }
                    None => {}
                }
                if let Some(job) = &state.job {
                    let label = match job.kind { JobKind::Languages => "Finding available languages", JobKind::Check => "Checking downloaded languages", JobKind::Download => "Downloading languages", JobKind::Translate => "Translating on this device" };
                    ui.horizontal(|ui| { ui.spinner(); ui.label(format!("{label} · {}s",job.since.elapsed().as_secs())); });
                    if job.kind == JobKind::Translate && state.progress.1 > 0 { ui.add(egui::ProgressBar::new(state.progress.0 as f32/state.progress.1 as f32).show_percentage()); }
                    if job.kind == JobKind::Download && state.progress.1 > 0 {
                        ui.add(egui::ProgressBar::new(state.progress.0 as f32/state.progress.1 as f32).text(format!("{:.1} / {:.1} MB", state.progress.0 as f64/1_000_000., state.progress.1 as f64/1_000_000.)));
                    }
                    if ui.button("Cancel").clicked() { state.cancel(); state.message = Some("Cancelled. Your PDF is unchanged.".into()); }
                }
                if state.page_job.is_some() {
                    ui.horizontal(|ui| { ui.spinner(); ui.label("Reading page locally…"); });
                    if ui.button("Cancel reading").clicked() { state.cancel(); }
                }
                if let Some(error) = &state.error {
                    ui.colored_label(theme::danger(), error);
                    if state.unsupported.is_none() && state.availability.is_none() && ui.add_enabled(state.job.is_none(), ActionButton::new(Icon::Reload,"Check languages again")).clicked() {
                        if state.languages.is_empty() { state.start(Operation::Languages,JobKind::Languages); }
                        else { state.check(); }
                    }
                }
                if let Some(message) = &state.message { ui.label(message); }
                ui.separator();
                if let Some(source) = &state.source {
                    ui.small(&source.label);
                    egui::CollapsingHeader::new("Original text").default_open(false).show(ui, |ui| { readonly_text(ui,"translation_original", &source.text, 7); });
                    ui.add_space(6.);
                    ui::caption(ui, &state.name(&state.preferences.target).to_uppercase());
                    let output = state.output();
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(state.job.is_none() && state.availability==Some(Availability::Installed), ActionButton::new(Icon::Translate, if state.completed { "Translate again" } else { "Translate" }).primary()).clicked() { state.translate(); }
                        if ui.add_enabled(state.completed,ActionButton::new(Icon::Copy,"Copy")).on_hover_text("Copy translation to the system clipboard").clicked() { ctx.copy_text(output.clone()); }
                    });
                    if output.is_empty() { ui.label(RichText::new("Your translation will appear here.").color(theme::muted())); }
                    else { readonly_text(ui,"translation_result", &output, 12); }
                    let source = state.source.as_ref().expect("source shown above");
                    if ui.add_enabled(state.completed && can_annotate && !cropping && source.attached_note.is_none(),
                        ActionButton::new(Icon::Note, if source.attached_note.is_some() { "Note attached" } else { "Attach translation as note" }))
                        .on_hover_text("Add a note beside the selected text, or a page note for a whole-page translation. Save the PDF to keep it.")
                        .clicked() {
                        attach_note = Some(translation_note::Request {
                            tab: source.key.tab, revision: source.key.revision, page: source.key.page,
                            anchor: source.anchor, source_object: source.source_object.clone(),
                            source_language: state.name(&state.preferences.source), target_language: state.name(&state.preferences.target), text: output,
                        });
                    }
                    if let Some(note_id) = &source.attached_note
                        && ui.add(ActionButton::new(Icon::Note,"Open note")).clicked() {
                        open_note = Some((source.key.tab, note_id.clone()));
                    }
                    if cropping { ui.small("Finish or cancel the current crop before attaching a note."); }
                    if !can_annotate { ui.small("This document is read-only. Copy is available; adding a note requires editing access."); }
                } else if state.page_job.is_none() {
                    ui.label("Select text and right-click Translate, or translate the current page.");
                }
                if has_page && ocr::available() && ui.add_enabled(state.page_job.is_none(),ActionButton::new(Icon::Ocr,"Read scanned page")).on_hover_text("Read words locally without adding an OCR layer or changing the PDF").clicked() { read_scan = true; }
                ui.add_space(8.);
                ui.separator();
                if ui.add(ActionButton::new(Icon::Plus,"Add / manage languages").selected(state.show_languages)).clicked() { state.show_languages = !state.show_languages; }
                if state.show_languages {
                    ui.small("Choose another supported language. Download its pair above for offline use. Favorites appear first in the language menus.");
                    ui.add(egui::TextEdit::singleline(&mut state.language_filter).id(egui::Id::new("translation_language_filter")).hint_text("Find a language…").desired_width(f32::INFINITY))
                        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, ui.is_enabled(), "Find a language"));
                    let query = state.language_filter.to_lowercase();
                    let mut choose = None; let mut favorite_change = false;
                    egui::ScrollArea::vertical().id_salt("language_catalog").max_height(220.).show(ui, |ui| {
                        for lang in &state.languages {
                            if !lang.name.to_lowercase().contains(&query) && !lang.id.to_lowercase().contains(&query) { continue; }
                            ui.horizontal(|ui| {
                                let mut favorite = state.preferences.favorites.contains(&lang.id) || state.preferences.favorites.iter().any(|id| base_language(id)==base_language(&lang.id));
                                if ui.checkbox(&mut favorite, &lang.name).changed() {
                                    state.preferences.favorites.retain(|id| base_language(id)!=base_language(&lang.id));
                                    if favorite { state.preferences.favorites.push(lang.id.clone()); }
                                    favorite_change = true;
                                }
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let label = format!("Use {} as target language", lang.name);
                                    let use_language = ui.small_button("Use").on_hover_text(&label);
                                    use_language.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &label));
                                    if use_language.clicked() { choose=Some(lang.id.clone()); }
                                });
                            });
                        }
                    });
                    if favorite_change { state.persist(); }
                    if let Some(id) = choose {
                        state.preferences.target = id;
                        if state.preferences.source==state.preferences.target { state.preferences.source=state.languages.iter().find(|l| base_language(&l.id)=="en").map_or("en".into(),|l| l.id.clone()); }
                        state.pair_changed();
                    }
                    if ui.add_enabled(state.job.is_none(), ActionButton::new(Icon::Reload,"Refresh languages")).clicked() { state.start(Operation::Languages,JobKind::Languages); }
                }
                ui.add_space(10.);
                ui.small("Translation stays in memory unless you attach it as a note or copy it. Adding a note changes the PDF and supports Undo.");
                #[cfg(target_os="macos")]
                ui.small("Apple may collect language/usage metrics, but not the original or translated text. Language downloads require internet.");
            });
        });
        if translate_selection {
            self.translate_selection(ctx);
        }
        if translate_page {
            self.translate_page(ctx);
        }
        if read_scan {
            self.start_translation_page(ctx, true);
        }
        if let Some((tab, note_id)) = open_note {
            self.open_note_reader(tab, &note_id);
        }
        if let Some(request) = attach_note {
            match self.attach_translation_note(ctx, request) {
                Ok(()) => {
                    if let Some(tab) = self.tabs.get(self.active)
                        && let Some(source) = &mut self.translation.source
                    {
                        source.key = document_key(tab);
                        source.attached_note = tab.selected_annotation.clone();
                    }
                    self.translation.message =
                        Some("Translation attached as a note. Save your PDF to keep it.".into());
                }
                Err(error) => self.translation.error = Some(error),
            }
        }
    }
}

fn language_picker(
    ui: &mut egui::Ui,
    id: &str,
    selected: &mut String,
    languages: &[Language],
    favorites: &[String],
) -> egui::Response {
    let name = languages.iter().find(|l| l.id == *selected).map_or_else(
        || {
            match base_language(selected) {
                "en" => "English",
                "tr" => "Turkish",
                "de" => "German",
                _ => selected.as_str(),
            }
            .to_owned()
        },
        |l| l.name.clone(),
    );
    egui::ComboBox::from_id_salt(id)
        .width(ui.available_width())
        .truncate()
        .selected_text(name)
        .show_ui(ui, |ui| {
            for preferred in [true, false] {
                for lang in languages {
                    let favorite = favorites
                        .iter()
                        .any(|id| base_language(id) == base_language(&lang.id));
                    if favorite == preferred {
                        ui.selectable_value(selected, lang.id.clone(), &lang.name);
                    }
                }
                if preferred {
                    ui.separator();
                }
            }
        })
        .response
}
fn readonly_text(ui: &mut egui::Ui, id: &str, text: &str, rows: usize) {
    let mut value = text;
    let height = ui.text_style_height(&egui::TextStyle::Body) * rows as f32 + 24.;
    egui::ScrollArea::vertical()
        .id_salt((id, "scroll"))
        .max_height(height)
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut value)
                    .id_salt(id)
                    .desired_width(f32::INFINITY)
                    .desired_rows(rows)
                    .font(egui::TextStyle::Body)
                    .margin(ui::FIELD_MARGIN),
            )
            .widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::TextEdit,
                    true,
                    if id.ends_with("original") {
                        "Original text"
                    } else {
                        "Translation result"
                    },
                )
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use mypdf_milestone0::{content, convert};
    #[test]
    fn translation_language_catalog_stays_bounded_and_search_accepts_input() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.translation.open = true;
        app.translation.show_languages = true;
        app.translation.availability = Some(Availability::Installed);
        app.translation.languages = (0..47)
            .map(|i| Language {
                id: format!("language-{i}"),
                name: if i == 0 {
                    "German".into()
                } else {
                    format!("Language {i} ({})", "regional variation ".repeat(5))
                },
            })
            .collect();
        let frame = |app: &mut Viewer, events| {
            let mut canvas = Rect::NOTHING;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 820.))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    app.translation_panel(ctx);
                    canvas = ctx.available_rect();
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.label("Document canvas");
                    });
                },
            );
            assert!(
                canvas.is_finite() && canvas.width() >= 700. && canvas.height() > 600.,
                "canvas: {canvas:?}"
            );
            let meshes = ctx.tessellate(output.shapes, output.pixels_per_point);
            for primitive in meshes {
                assert!(primitive.clip_rect.is_finite());
                if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
                    assert!(
                        mesh.vertices.iter().all(|v| v.pos.x.is_finite()
                            && v.pos.y.is_finite()
                            && v.pos.x.abs() < 5000.
                            && v.pos.y.abs() < 5000.),
                        "unbounded translation mesh"
                    );
                }
            }
        };
        for _ in 0..5 {
            frame(&mut app, vec![]);
        }
        let id = egui::Id::new("translation_language_filter");
        let field = ctx.read_response(id).expect("search field").rect;
        assert!(
            field.is_finite() && field.width() <= 480. && field.center().y < 820.,
            "field: {field:?}"
        );
        for pressed in [true, false] {
            frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(field.center()),
                    egui::Event::PointerButton {
                        pos: field.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        frame(&mut app, vec![egui::Event::Text("German".into())]);
        assert_eq!(app.translation.language_filter, "German");
        assert!(ctx.memory(|m| m.has_focus(id)));

        app.translation.language_filter.clear();
        app.translation.source = Some(Source {
            key: DocumentKey {
                tab: 1,
                revision: 0,
                page: 0,
            },
            label: "Selected text · Page 1".into(),
            text: "Synthetic original text. ".repeat(50),
            anchor: None,
            source_object: None,
            attached_note: Some("test-note".into()),
        });
        app.translation.results = vec![Some(
            "Synthetic translation remains visible and scrollable. ".repeat(60),
        )];
        app.translation.completed = true;
        for _ in 0..5 {
            frame(&mut app, vec![]);
        }
        for _ in 0..20 {
            let field = ctx.read_response(id).unwrap().rect;
            if field.center().y < 760. {
                break;
            }
            frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(pos2(1190., 150.)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: vec2(0., -100.),
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        for _ in 0..20 {
            frame(&mut app, vec![]);
        }
        let field = ctx.read_response(id).unwrap().rect;
        assert!(
            field.width() <= 480. && field.center().y < 820.,
            "field with results: {field:?}"
        );
        for pressed in [true, false] {
            frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(field.center()),
                    egui::Event::PointerButton {
                        pos: field.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        frame(&mut app, vec![egui::Event::Text("German".into())]);
        assert_eq!(app.translation.language_filter, "German");
    }

    #[test]
    fn translation_page_reflow_joins_visual_wraps_but_preserves_structure() {
        let line = |text: &str, x: f32, y: f32, width: f32, height: f32| TranslationLine {
            text: text.into(),
            rect: Some([x, y, x + width, y + height]),
            height,
        };
        let lines = vec![
            line("Quarterly summary", 30., 20., 140., 18.),
            line(
                "Revenue increased with service contracts in",
                30.,
                55.,
                300.,
                10.,
            ),
            line(
                "İstanbul and Berlin during the quarter.",
                30.,
                70.,
                280.,
                10.,
            ),
            line(
                "Another paragraph starts after the gap.",
                30.,
                110.,
                280.,
                10.,
            ),
            line("• First item continues on", 30., 145., 220., 10.),
            line("the next visual line.", 40., 160., 180., 10.),
            line("• Second item stays separate.", 30., 175., 240., 10.),
            line("1. Numbered item", 30., 200., 150., 10.),
            line("2. Another numbered item", 30., 215., 230., 10.),
        ];
        assert_eq!(
            reflow_translation_lines(&lines),
            concat!(
                "Quarterly summary\n\n",
                "Revenue increased with service contracts in İstanbul and Berlin during the quarter.\n\n",
                "Another paragraph starts after the gap.\n",
                "• First item continues on the next visual line.\n",
                "• Second item stays separate.\n",
                "1. Numbered item\n2. Another numbered item",
            )
        );
        let columns = vec![
            line("Left column text", 30., 20., 140., 10.),
            line("Right column text", 250., 20., 140., 10.),
            line("Second right-column sentence.", 250., 35., 140., 10.),
        ];
        assert!(
            reflow_translation_lines(&columns).starts_with("Left column text\n\nRight column text")
        );
        let unknown = vec![
            TranslationLine {
                text: "Explicit first line".into(),
                rect: None,
                height: 0.,
            },
            TranslationLine {
                text: "Unknown geometry".into(),
                rect: None,
                height: 0.,
            },
        ];
        assert_eq!(
            reflow_translation_lines(&unknown),
            "Explicit first line\n\nUnknown geometry"
        );
    }

    #[test]
    fn translation_reflows_real_pdf_visual_lines_without_changing_original_extraction() {
        let sentence = "Revenue increased because new service contracts in Istanbul and Berlin supported steady growth during the quarter.";
        let bytes = content::add_text(
            &convert::blank(400., 400.).unwrap(),
            1,
            sentence,
            [30., 30., 220., 180.],
            16.,
            [0.; 3],
        )
        .unwrap();
        let pdf = hayro::hayro_syntax::Pdf::new(bytes.clone()).unwrap();
        let original = viewer::extract_page(
            &pdf.pages()[0],
            &hayro::hayro_interpret::InterpreterCache::new(),
        );
        assert!(
            original.text.contains('\n'),
            "fixture must contain a real visual wrap"
        );
        assert_eq!(translation_page_text(&bytes, 0).unwrap(), sentence);
        assert!(
            original.text.contains('\n'),
            "translation must not modify source selection text"
        );
    }

    #[test]
    fn translation_page_snapshot_reads_current_annotations_once_without_mutation() {
        use mypdf_milestone0::annotations::{self, Annotation, Kind, Session};
        let base = content::add_text(
            &convert::blank(400., 400.).unwrap(),
            1,
            "Page body",
            [30., 30., 300., 40.],
            16.,
            [0.; 3],
        )
        .unwrap();
        let initial = Annotation::new(
            0,
            [30., 100., 330., 140.],
            Kind::Text {
                text: "Old annotation".into(),
            },
            Default::default(),
        );
        let source = annotations::export(&base, std::slice::from_ref(&initial)).unwrap();
        let original = source.clone();
        let mut session = Session::new(source.into(), vec![initial.clone()], true);
        let mut updated = initial;
        updated.kind = Kind::Text {
            text: "Updated annotation".into(),
        };
        let added = Annotation::new(
            0,
            [30., 180., 330., 220.],
            Kind::Text {
                text: "New annotation".into(),
            },
            Default::default(),
        );
        let mut hidden = Annotation::new(
            0,
            [30., 260., 330., 300.],
            Kind::Text {
                text: "Hidden annotation".into(),
            },
            Default::default(),
        );
        hidden.object.hidden = true;
        session.commit(vec![updated, added, hidden]).unwrap();
        let revision = session.revision();
        let snapshot =
            translation_page_snapshot(&session.source, Some(&session.annotations)).unwrap();
        let text = translation_page_text(&snapshot, 0).unwrap();
        for expected in ["Page body", "Updated annotation", "New annotation"] {
            assert_eq!(text.matches(expected).count(), 1, "{expected}: {text}");
        }
        assert!(!text.contains("Old annotation") && !text.contains("Hidden annotation"));
        assert_eq!(session.source.as_ref(), original);
        assert_eq!(session.revision(), revision);
        assert!(session.dirty());

        let deleted = translation_page_snapshot(&session.source, Some(&[])).unwrap();
        let text = translation_page_text(&deleted, 0).unwrap();
        assert!(text.contains("Page body") && !text.contains("annotation"));
        assert_eq!(
            translation_page_snapshot(&original, None).unwrap(),
            original
        );
    }

    #[test]
    fn translation_snapshot_ocr_render_includes_visible_artwork_and_foreign_text() {
        use mypdf_milestone0::annotations::{self, Annotation, Kind};
        let blank = convert::blank(300., 300.).unwrap();
        let foreign = Annotation::new(
            0,
            [20., 20., 280., 70.],
            Kind::Text {
                text: "Foreign text".into(),
            },
            Default::default(),
        );
        let foreign_pdf = annotations::export(&blank, &[foreign]).unwrap();
        // Simulate another PDF editor's FreeText appearance, which MyPDF must
        // keep when it replaces only its own annotation objects.
        let mut doc = lopdf::Document::load_mem(&foreign_pdf).unwrap();
        for object in doc.objects.values_mut() {
            if let Ok(dict) = object.as_dict_mut() {
                dict.remove(b"MyPDFData");
            }
        }
        let mut source = vec![];
        doc.save_to(&mut source).unwrap();
        let mut artwork = Annotation::new(
            0,
            [60., 100., 220., 220.],
            Kind::Rectangle,
            Default::default(),
        );
        artwork.style.fill = Some([0.; 3]);
        let snapshot = translation_page_snapshot(&source, Some(&[artwork.clone()])).unwrap();
        assert_eq!(
            translation_page_text(&snapshot, 0)
                .unwrap()
                .matches("Foreign text")
                .count(),
            1
        );
        let (size, before) = viewer::render_page(&source, 0, 1.).unwrap();
        let (updated_size, after) = viewer::render_page(&snapshot, 0, 1.).unwrap();
        assert_eq!(size, updated_size);
        assert_ne!(
            before, after,
            "OCR must receive current artwork rather than the old page"
        );
        artwork.object.hidden = true;
        let hidden = translation_page_snapshot(&source, Some(&[artwork])).unwrap();
        assert_eq!(viewer::render_page(&hidden, 0, 1.).unwrap().1, before);
    }

    #[test]
    fn chunks_preserve_turkish_and_german_without_truncation() {
        let unit = "İstanbul, ışık, Straße, Größe. ";
        let text = unit.repeat(600);
        let parts = chunks(&text);
        assert!(parts.len() > 1);
        assert!(parts.iter().all(|s| s.chars().count() <= 1800));
        assert_eq!(
            parts.join(" ").split_whitespace().collect::<Vec<_>>(),
            text.split_whitespace().collect::<Vec<_>>()
        );
    }
    #[test]
    fn preferences_cannot_persist_document_text() {
        let mut state = State::new();
        state.source = Some(Source {
            key: DocumentKey {
                tab: 1,
                revision: 1,
                page: 0,
            },
            label: "Secret filename".into(),
            text: "Private sentence".into(),
            anchor: None,
            source_object: None,
            attached_note: None,
        });
        state.results = vec![Some("Private translation".into())];
        let stored = serde_json::to_string(&state.preferences).unwrap();
        assert!(!stored.contains("Private") && !stored.contains("Secret"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&stored)
                .unwrap()
                .as_object()
                .unwrap()
                .len(),
            3
        );
    }
    #[test]
    fn incomplete_or_failed_translation_never_looks_finished() {
        let mut state = State::new();
        state.results = vec![Some("First".into()), None];
        state.receive(JobKind::Translate, Event::Finished);
        assert!(!state.completed && state.results.is_empty() && state.error.is_some());
        state.results = vec![Some("First".into())];
        state.availability = Some(Availability::Installed);
        state.receive(
            JobKind::Translate,
            Event::Failed("Cancelled service".into()),
        );
        assert!(!state.completed && state.results.is_empty());
        assert!(state.availability.is_none());
    }
    #[test]
    fn clear_document_discards_private_results() {
        let mut state = State::new();
        state.source = Some(Source {
            key: DocumentKey {
                tab: 1,
                revision: 1,
                page: 0,
            },
            label: "Page".into(),
            text: "Old".into(),
            anchor: None,
            source_object: None,
            attached_note: None,
        });
        state.results = vec![Some("Alt".into())];
        state.completed = true;
        state.clear_document();
        assert!(state.source.is_none() && state.results.is_empty() && !state.completed);
    }
}
