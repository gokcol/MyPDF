//! Native content inspector, AcroForm panel, and local conversion workflows.
use super::*;
use mypdf_milestone0::{
    content,
    convert::{self, Format},
    document, form_design, forms, text_block,
    text_style::{self, Alignment, TextFormat},
};

#[path = "context_menu.rs"]
pub(super) mod context;
#[path = "text_formatting.rs"]
mod formatting;
#[path = "image_effects.rs"]
mod image_effects;
#[path = "image_region.rs"]
mod region;
pub(super) use context::{page_menu, smart_select};

type Key = (u64, u64, usize);
#[derive(Clone, PartialEq, Debug)]
enum InlineTarget {
    Run(usize),
    Annotation(String),
    Block(Box<BlockDraft>),
}
#[derive(Clone, PartialEq, Debug)]
struct BlockDraft {
    selection: text_block::Selection,
    masks: Vec<[f32; 4]>,
    spans: Vec<mypdf_milestone0::rich_text::Span>,
    styled_text: String,
    selected_chars: Option<std::ops::Range<usize>>,
    initial_spans: Vec<mypdf_milestone0::rich_text::Span>,
    font: usize,
    alignment: Alignment,
    /// Font, size, color and right edge when the block opened.
    initial: (usize, f32, [f32; 3], f32),
    initial_origin: [f32; 2],
    layout: Vec<mypdf_milestone0::rich_text::LineLayout>,
    preserve_layout: bool,
    initial_layout: Vec<mypdf_milestone0::rich_text::LineLayout>,
}
#[derive(Clone)]
struct InlineText {
    key: Key,
    target: InlineTarget,
    geometry: Option<document::RunLayout>,
    original: String,
    text: String,
    rect: [f32; 4],
    size: f32,
    color: [f32; 3],
    font: String,
    focus: bool,
}
impl InlineText {
    /// Whether committing would change the document. An untouched block is
    /// closed as it is rather than rewritten with uniform formatting.
    fn changed(&self) -> bool {
        self.text != self.original
            || matches!(&self.target, InlineTarget::Block(b)
                if self.rect[2] != b.initial.3
                    || b.spans != b.initial_spans
                    || b.layout != b.initial_layout
                    || [self.rect[0], self.rect[1]] != b.initial_origin
                    || b.alignment != Alignment::Left
                    || b.preserve_layout == b.layout.is_empty())
    }
}

pub(super) struct Inspection {
    pub(super) key: Key,
    text: Result<Vec<document::TextRun>, String>,
    images: Result<Vec<content::ImageObject>, String>,
    pub(super) fields: Result<Vec<forms::Field>, String>,
    /// Every field of the document, as the form designer sees it.
    pub(super) designed: Result<Vec<mypdf_milestone0::form_design::Designed>, String>,
}
/// Tab, base revision, new bytes, and an optional note for the status bar.
type Change = Result<(u64, u64, Vec<u8>, Option<String>), String>;
type Output = Result<(String, Option<PathBuf>), String>;
#[derive(Clone)]
pub(super) enum Edit {
    Navigation {
        bookmarks: Option<Vec<mypdf_milestone0::navigation_edit::Bookmark>>,
        labels: Option<Vec<mypdf_milestone0::navigation_edit::LabelRule>>,
    },
    AddText {
        spans: Vec<mypdf_milestone0::rich_text::Span>,
        page: u32,
        text: String,
        rect: [f32; 4],
        size: f32,
        color: [f32; 3],
        format: TextFormat,
    },
    Text {
        page: u32,
        ordinal: usize,
        original: String,
        replacement: String,
    },
    Block {
        page: u32,
        block: text_block::Replacement,
    },
    Image {
        page: u32,
        index: usize,
        edit: content::ImageEdit,
    },
    Field {
        id: lopdf::ObjectId,
        value: String,
    },
    HeaderFooter(mypdf_milestone0::header_footer::Stamp),
    AddImage {
        page: u32,
        rect: [f32; 4],
        image: Vec<u8>,
    },
    RemoveHeaderFooter(Vec<mypdf_milestone0::header_footer::Kind>),
    Redact {
        areas: Vec<mypdf_milestone0::redact::Area>,
        options: mypdf_milestone0::redact::Options,
    },
    Flatten {
        dpi: f32,
    },
    AddField {
        page: usize,
        rect: [f32; 4],
        kind: mypdf_milestone0::form_design::FieldType,
        properties: mypdf_milestone0::form_design::Properties,
    },
    UpdateField {
        field: lopdf::ObjectId,
        widget: lopdf::ObjectId,
        properties: mypdf_milestone0::form_design::Properties,
    },
    PlaceField {
        widget: lopdf::ObjectId,
        rect: [f32; 4],
    },
    RemoveField {
        field: lopdf::ObjectId,
        widget: Option<lopdf::ObjectId>,
    },
    FieldImage {
        field: lopdf::ObjectId,
        image: Vec<u8>,
    },
    ClearForm,
    FlattenForm,
    DetectFields,
    TabOrder(mypdf_milestone0::form_design::TabOrder),
    ImportFormData {
        data: Vec<u8>,
        format: mypdf_milestone0::form_data::DataFormat,
    },
    InsertLayout {
        page: usize,
        rect: [f32; 4],
        layout: mypdf_milestone0::form_layout::Layout,
    },
    ReduceSize(mypdf_milestone0::compress::Level),
    Watermark(mypdf_milestone0::watermark::Watermark),
    RemoveWatermarks,
    RecognizeText(Arc<super::recognize::Progress>),
    /// Fill fields with the saved details.
    FillDetails(Vec<mypdf_milestone0::form_profile::Suggestion>),
}
pub(super) enum AfterText {
    Save(u64, super::save::Intent),
    Close(Option<u64>),
    Undo(u64, usize),
}
pub(super) struct ContentState {
    pub(super) hide_guides: bool,
    context: context::State,
    change_page: usize,
    change_name: &'static str,
    pub(super) after_text: Option<AfterText>,
    existing: Option<InlineText>,
    next_existing: Option<InlineText>,
    replacing_text: bool,
    add_after_existing: Option<(u64, usize)>,
    pub(super) area_mode: bool,
    marquee: Option<(Key, egui::Pos2)>,
    selection_hint: Option<String>,
    key: Option<Key>,
    pub add_target: Option<(u64, usize)>,
    new_text: String,
    new_styled_text: String,
    new_spans: Vec<mypdf_milestone0::rich_text::Span>,
    new_selection: Option<std::ops::Range<usize>>,
    show_text_inspector: bool,
    focus_new_text: bool,
    new_rect: [f32; 4],
    new_initial_rect: Option<[f32; 4]>,
    new_drag: Option<[f32; 4]>,
    new_size: f32,
    new_color: [f32; 3],
    new_font: usize,
    new_alignment: Alignment,
    new_placed: bool,
    pub(super) new_commit: bool,
    inserting_text: bool,
    exit_text_tool: bool,
    next_text: Option<(u64, usize, [f32; 2])>,
    pub(super) loading: bool,
    pub(super) inspection: Option<Inspection>,
    selected: Option<usize>,
    image_effects: Option<image_effects::EmbeddedEffects>,
    region: region::State,
    pub image_mode: bool,
    /// Another tool (Redact) owns the page overlay.
    pub(super) paused: bool,
    rect: [f32; 4],
    pub busy: bool,
    /// Owned by the UI as well as the worker so closing the app cancels OCR.
    recognition: Option<Arc<super::recognize::Progress>>,
    tx: mpsc::Sender<Change>,
    rx: mpsc::Receiver<Change>,
    inspect_tx: mpsc::Sender<Inspection>,
    inspect_rx: mpsc::Receiver<Inspection>,
    output_tx: mpsc::Sender<Output>,
    output_rx: mpsc::Receiver<Output>,
    format: Format,
    range: String,
    dpi: f32,
    erased: Option<Erased>,
}
impl Drop for ContentState {
    fn drop(&mut self) {
        if let Some(progress) = &self.recognition {
            progress.cancellation.cancel();
        }
    }
}
/// The page rendered without the words an open block replaces, so the editor
/// hides only those words and keeps backgrounds, lines and boxes around them.
struct Erased {
    key: Key,
    masks: Vec<[f32; 4]>,
    rx: Option<mpsc::Receiver<Option<egui::ColorImage>>>,
    texture: Option<TextureHandle>,
}
impl ContentState {
    /// Read the inline editor's selection without committing or changing it.
    pub(super) fn selected_translation_text(
        &self,
        ctx: &egui::Context,
        active: Key,
    ) -> Option<String> {
        let (text, id) = if let Some(draft) = &self.existing {
            if draft.key != active {
                return None;
            }
            (
                &draft.text,
                egui::Id::new((draft.key.0, draft.key.2, "inline_existing_text")),
            )
        } else if self.new_placed {
            let (tab, page) = self.add_target?;
            if (tab, page) != (active.0, active.2) {
                return None;
            }
            (
                &self.new_text,
                egui::Id::new((tab, page, "inline_new_text")),
            )
        } else {
            return None;
        };
        let state = egui::text_edit::TextEditState::load(ctx, id)?;
        let range = state.cursor.char_range()?.as_sorted_char_range();
        if range.is_empty() {
            return None;
        }
        let selected: String = text.chars().skip(range.start).take(range.len()).collect();
        (!selected.trim().is_empty()).then_some(selected)
    }
    /// Capture the source block's page geometry together with a translation.
    /// Never infer it later from whichever editor or page happens to be active.
    pub(super) fn translation_anchor(&self, ctx: &egui::Context, active: Key) -> Option<[f32; 4]> {
        self.selected_translation_text(ctx, active)?;
        if let Some(draft) = &self.existing {
            Some(draft.rect)
        } else {
            let [x, y, width, height] = self.new_rect;
            Some([x, y, x + width, y + height])
        }
    }
    /// A note attached to a managed text object must respect its locked state.
    pub(super) fn translation_source_object(
        &self,
        ctx: &egui::Context,
        active: Key,
    ) -> Option<String> {
        self.selected_translation_text(ctx, active)?;
        match &self.existing.as_ref()?.target {
            InlineTarget::Annotation(id) => Some(id.clone()),
            _ => None,
        }
    }
    fn clear_new_text(&mut self) {
        self.new_text.clear();
        self.new_styled_text.clear();
        self.new_spans.clear();
        self.new_selection = None;
        self.new_initial_rect = None;
    }
    fn sync_new_styles(&mut self) {
        let fallback = formatting::span(
            self.new_font,
            self.new_size,
            self.new_color,
            self.new_text.len(),
        );
        formatting::sync(
            &self.new_text,
            &mut self.new_styled_text,
            &mut self.new_spans,
            fallback,
        );
    }
    fn text_font(&self, size: f32) -> FontId {
        FontId::new(
            size,
            text_style::fonts()
                .get(self.new_font)
                .map(|f| egui::FontFamily::Name(f.ui_name().into()))
                .unwrap_or(egui::FontFamily::Proportional),
        )
    }
    /// Open Add text in the middle of the page with pasted words, ready to place.
    pub(super) fn paste_new_text(&mut self, id: u64, page: usize, size: PageSize, text: String) {
        self.image_mode = false;
        self.area_mode = false;
        self.place_text(id, page, [size.width / 2. - 120., size.height / 2.], size);
        let lines = text.lines().count().max(1) as f32;
        self.new_rect[3] = (lines * self.new_size * 1.3 + self.new_size)
            .max(self.new_rect[3])
            .min(size.height - self.new_rect[1]);
        self.new_text = text;
    }
    fn place_text(&mut self, id: u64, page: usize, point: [f32; 2], size: PageSize) {
        self.add_target = Some((id, page));
        self.new_placed = true;
        self.clear_new_text();
        let x = point[0].clamp(0., (size.width - 10.).max(0.));
        let w = 240_f32.min(size.width - x);
        let h = (self.new_size * 2.6).min(size.height);
        self.new_rect = [x, point[1].clamp(0., (size.height - h).max(0.)), w, h];
        self.new_initial_rect = Some(self.new_rect);
        self.focus_new_text = true;
        self.show_text_inspector = true;
    }
    #[cfg(test)]
    pub(super) fn existing_preview(&mut self, id: u64) {
        if let Some(info) = &self.inspection
            && let Ok(runs) = &info.text
            && let Some((ordinal, run)) = runs
                .iter()
                .enumerate()
                .find(|(_, r)| r.text == document::ORIGINAL)
            && let Some(rect) = run.rect
        {
            self.existing = Some(InlineText {
                key: (id, info.key.1, info.key.2),
                target: InlineTarget::Run(ordinal),
                geometry: run.layout.clone(),
                original: run.text.clone(),
                text: document::REPLACEMENT.into(),
                rect,
                size: run.size,
                color: run.color,
                font: run.base_font.clone(),
                focus: true,
            });
        }
    }
    #[cfg(test)]
    pub(super) fn block_preview(&mut self, id: u64) {
        if let Some(info) = &self.inspection
            && let Ok(runs) = &info.text
        {
            let items = runs
                .iter()
                .enumerate()
                .filter(|(_, r)| {
                    r.text == document::ORIGINAL || r.text == document::TURKISH_ORIGINAL
                })
                .filter_map(|(ordinal, run)| {
                    Some(InlineText {
                        key: (id, info.key.1, info.key.2),
                        target: InlineTarget::Run(ordinal),
                        geometry: run.layout.clone(),
                        original: run.text.clone(),
                        text: run.text.clone(),
                        rect: run.rect?,
                        size: run.size,
                        color: run.color,
                        font: run.base_font.clone(),
                        focus: true,
                    })
                })
                .collect();
            self.existing = selected_block(
                items,
                [48., 140., 545., 260.],
                PageSize {
                    width: 595.,
                    height: 842.,
                },
            );
            self.area_mode = true;
        }
    }
    #[cfg(test)]
    pub(super) fn erased_ready(&self) -> bool {
        self.erased.as_ref().is_some_and(|e| e.rx.is_none())
    }
    #[cfg(test)]
    pub(super) fn text_preview(&mut self, id: u64) {
        self.add_target = Some((id, 0));
        self.new_placed = true;
        self.new_rect = [72., 170., 340., 100.];
        self.new_text = "Type directly on the page.\nMake it yours.".into();
        self.new_font = text_style::fonts()
            .iter()
            .position(|f| f.bold && !f.italic)
            .unwrap_or(0);
        self.new_size = 22.;
    }
    /// A click away from the open text editor: keep changes, or close an
    /// untouched selection without rewriting anything.
    pub(super) fn click_outside(&mut self, tab: u64) {
        if !self.busy && self.existing.as_ref().is_some_and(|e| e.key.0 == tab) {
            self.new_commit = true;
        }
    }
    pub(super) fn pending_text(&self) -> Option<u64> {
        if let Some(text) = &self.existing {
            return Some(text.key.0);
        }
        self.add_target
            .filter(|_| !self.new_text.trim().is_empty())
            .map(|(id, _)| id)
    }
    pub(super) fn has_pending_text_change(&self, tab: u64) -> bool {
        self.existing
            .as_ref()
            .is_some_and(|draft| draft.key.0 == tab && draft.changed())
            || self.add_target.is_some_and(|target| target.0 == tab)
                && !self.new_text.trim().is_empty()
    }
    /// Text-field Undo normally stays with typing. Once placement also changes,
    /// Undo needs the complete draft so it cannot silently leave the move behind.
    pub(super) fn placement_undo_owns_input(&self, ctx: &egui::Context, tab: u64) -> bool {
        let (placed, id) = if let Some(draft) = &self.existing {
            (
                draft.key.0 == tab
                    && matches!(&draft.target, InlineTarget::Block(block)
                    if [draft.rect[0], draft.rect[1]] != block.initial_origin
                        || draft.rect[2] != block.initial.3),
                egui::Id::new((tab, draft.key.2, "inline_existing_text")),
            )
        } else if let Some((id, page)) = self.add_target {
            (
                id == tab
                    && !self.new_text.trim().is_empty()
                    && self
                        .new_initial_rect
                        .is_some_and(|initial| initial[..3] != self.new_rect[..3]),
                egui::Id::new((tab, page, "inline_new_text")),
            )
        } else {
            return false;
        };
        placed && (!ctx.wants_keyboard_input() || ctx.memory(|memory| memory.has_focus(id)))
    }
    /// Toolbar/history Undo must not act on an older document while an edit is
    /// still in place. Commit then undo atomically; failures retain the draft.
    pub(super) fn queue_text_undo(&mut self, tab: u64, redo: bool) -> bool {
        if !self.has_pending_text_change(tab) {
            if self
                .existing
                .as_ref()
                .is_some_and(|draft| draft.key.0 == tab)
            {
                self.existing = None;
                self.next_existing = None;
            }
            if self.add_target.is_some_and(|target| target.0 == tab) {
                self.clear_new_text();
                self.new_placed = false;
                self.add_target = None;
            }
            return false;
        }
        if !redo && self.after_text.is_none() {
            let page = self
                .existing
                .as_ref()
                .map(|draft| draft.key.2)
                .or_else(|| self.add_target.map(|(_, page)| page))
                .unwrap_or(0);
            self.after_text = Some(AfterText::Undo(tab, page));
            self.new_commit = true;
            self.next_existing = None;
            self.add_after_existing = None;
            self.next_text = None;
            self.exit_text_tool = true;
        }
        true
    }
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let (inspect_tx, inspect_rx) = mpsc::channel();
        let (output_tx, output_rx) = mpsc::channel();
        Self {
            hide_guides: false,
            context: Default::default(),
            change_page: 0,
            change_name: "Edit PDF content",
            after_text: None,
            existing: None,
            next_existing: None,
            replacing_text: false,
            add_after_existing: None,
            area_mode: false,
            marquee: None,
            selection_hint: None,
            key: None,
            add_target: None,
            new_text: String::new(),
            new_styled_text: String::new(),
            new_spans: Vec::new(),
            new_selection: None,
            show_text_inspector: false,
            focus_new_text: false,
            new_rect: [48., 48., 300., 120.],
            new_initial_rect: None,
            new_drag: None,
            new_size: 18.,
            new_color: [0.13, 0.15, 0.2],
            new_font: 0,
            new_alignment: Alignment::Left,
            new_placed: false,
            new_commit: false,
            inserting_text: false,
            exit_text_tool: false,
            next_text: None,
            loading: false,
            inspection: None,
            selected: None,
            image_effects: None,
            region: region::State::default(),
            image_mode: false,
            paused: false,
            rect: [0.; 4],
            busy: false,
            recognition: None,
            tx,
            rx,
            inspect_tx,
            inspect_rx,
            output_tx,
            output_rx,
            format: Format::Png,
            range: "1".into(),
            dpi: 144.,
            erased: None,
        }
    }
}
impl Viewer {
    fn undo_finished_text(&mut self, id: u64, page: usize, ctx: &egui::Context) {
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id)
            && let Some(session) = &mut tab.session
        {
            session.undo();
            // Release the editor's invisible focus so Redo routes to the document.
            ctx.memory_mut(|memory| {
                for editor in ["inline_existing_text", "inline_new_text"] {
                    memory.surrender_focus(egui::Id::new((id, page, editor)));
                }
            });
            self.status = Some((
                "Text edit undone • Redo restores your text and position".into(),
                false,
            ));
        }
    }
    pub(super) fn content_events(&mut self, ctx: &egui::Context) {
        if std::mem::take(&mut self.content.show_text_inspector) {
            self.inspector = true;
        }
        let mut after_text = None;
        let changes: Vec<_> = self.content.rx.try_iter().collect();
        for result in changes {
            self.content.busy = false;
            if self
                .content
                .recognition
                .take()
                .is_some_and(|p| p.cancellation.is_cancelled())
            {
                self.ocr_progress = None;
                self.status = Some((
                    "Text recognition cancelled. The document is unchanged.".into(),
                    false,
                ));
                continue;
            }
            match result {
                Ok((id, revision, bytes, note)) => {
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id)
                        && let Some(s) = &mut tab.session
                    {
                        if s.revision() != revision {
                            self.content.inserting_text = false;
                            self.content.replacing_text = false;
                            self.content.next_existing = None;
                            self.content.add_after_existing = None;
                            self.content.after_text = None;
                            self.content.next_text = None;
                            self.content.exit_text_tool = false;
                            self.status=Some(("The document changed during processing. Select the object again and retry.".into(),true));
                            continue;
                        }
                        let commit = if matches!(&self.content.after_text, Some(AfterText::Undo(target, _)) if *target == id)
                        {
                            s.commit_document_requiring_undo(bytes)
                        } else {
                            s.commit_document(bytes)
                        };
                        match commit {
                            Ok(()) => {
                                s.name_last_change(self.content.change_name);
                                s.set_change_focus(self.content.change_page, None);
                                tab.keep_stale = true;
                                self.editor.draft = None;
                                self.content.key = None;
                                if self.content.replacing_text {
                                    self.content.replacing_text = false;
                                    self.content.exit_text_tool = false;
                                    self.content.existing = self.content.next_existing.take();
                                    if let Some(next) = &mut self.content.existing
                                        && next.key.0 == id
                                    {
                                        next.key.1 = s.revision();
                                    }
                                    after_text = self.content.after_text.take();
                                    self.begin_add_after_existing();
                                } else if self.content.inserting_text {
                                    self.content.inserting_text = false;
                                    after_text = self.content.after_text.take();
                                    self.content.clear_new_text();
                                    self.content.new_placed = false;
                                    if self.content.exit_text_tool {
                                        self.content.add_target = None;
                                        self.content.exit_text_tool = false;
                                    }
                                    if let Some((next_id, page, point)) =
                                        self.content.next_text.take()
                                        && let Some(t) = self.tabs.iter().find(|t| t.id == next_id)
                                        && let Some(size) = t.pages.get(page)
                                    {
                                        self.content.place_text(next_id, page, point, *size);
                                    }
                                } else {
                                    self.content.add_target = None;
                                }
                                self.status = Some((
                                    note.unwrap_or_else(|| {
                                        "Changes applied • Undo is available • Save to keep them"
                                            .into()
                                    }),
                                    false,
                                ));
                            }
                            Err(e) => {
                                self.content.inserting_text = false;
                                self.content.replacing_text = false;
                                self.content.next_existing = None;
                                self.content.add_after_existing = None;
                                self.content.after_text = None;
                                self.content.next_text = None;
                                self.content.exit_text_tool = false;
                                self.status = Some((format!("{e:#}"), true));
                            }
                        }
                    }
                }
                Err(e) => {
                    if let Some(draft) = &self.content.existing {
                        if let Some(index) = self.tabs.iter().position(|t| t.id == draft.key.0) {
                            self.active = index;
                            self.home.visible = false;
                            self.tabs[index].jump(draft.key.2);
                        }
                        self.content.image_mode = false;
                        self.editor.mode = editing::Mode::Edit;
                    }
                    self.content.inserting_text = false;
                    self.content.replacing_text = false;
                    self.content.next_existing = None;
                    self.content.add_after_existing = None;
                    self.content.after_text = None;
                    self.content.exit_text_tool = false;
                    self.content.next_text = None;
                    self.status = Some((e, true));
                }
            }
        }
        for inspection in self.content.inspect_rx.try_iter() {
            self.content.loading = false;
            if self.content.key == Some(inspection.key) {
                self.content.loading = false;
                self.content.inspection = Some(inspection);
            }
        }
        let outputs: Vec<_> = self.content.output_rx.try_iter().collect();
        for result in outputs {
            self.content.busy = false;
            match result {
                Ok((message, path)) => {
                    if let Some(path) = path {
                        self.open(path, ctx);
                    }
                    self.status = Some((message, false));
                }
                Err(e) => self.status = Some((e, true)),
            }
        }
        if let Some(action) = after_text {
            match action {
                AfterText::Save(id, intent) => {
                    if let Some(index) = self.tabs.iter().position(|t| t.id == id) {
                        self.active = index;
                        self.save_intent = intent;
                        self.save_dialog(ctx);
                    }
                }
                AfterText::Close(id) => self.request_close(id, ctx),
                AfterText::Undo(id, page) => self.undo_finished_text(id, page, ctx),
            }
        }
    }
    pub(super) fn content_change(&mut self, ctx: &egui::Context, edit: Edit) {
        if self.content.busy || self.organizer.busy || !self.apply_pending_annotation() {
            return;
        }
        self.content.change_page = self.tabs.get(self.active).map_or(0, |t| t.page);
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let Some(s) = &tab.session else { return };
        if !s.editable {
            return;
        }
        let (id, revision, source, annotations) = (
            tab.id,
            s.revision(),
            s.source.clone(),
            s.annotations.clone(),
        );
        let tx = self.content.tx.clone();
        let wake = ctx.clone();
        self.content.change_name = match &edit {
            Edit::AddText { .. } => "Add text",
            Edit::Text { .. } | Edit::Block { .. } => "Edit text",
            Edit::Image { .. } => "Edit image",
            Edit::AddImage { .. } => "Add image",
            Edit::HeaderFooter(_) => "Add header or footer",
            Edit::RemoveHeaderFooter(_) => "Remove header or footer",
            Edit::Redact { .. } => "Redact content",
            Edit::Flatten { .. } => "Flatten PDF",
            Edit::Watermark(_) => "Add watermark",
            Edit::RemoveWatermarks => "Remove watermarks",
            Edit::ReduceSize(_) => "Reduce file size",
            Edit::RecognizeText(_) => "Recognize text",
            _ => "Edit form fields",
        };
        self.content.busy = true;
        self.content.recognition = match &edit {
            Edit::RecognizeText(progress) => Some(progress.clone()),
            _ => None,
        };
        self.ocr_progress = self.content.recognition.clone();
        let recognition = self.content.recognition.clone();
        self.status = Some(("Applying changes…".into(), false));
        std::thread::spawn(move || {
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> anyhow::Result<_> {
                    if let Some(progress) = &recognition {
                        progress.cancellation.check()?;
                    }
                    let bytes = if revision == 0 {
                        source.to_vec()
                    } else {
                        mypdf_milestone0::annotations::export(&source, &annotations)?
                    };
                    if let Some(progress) = &recognition {
                        progress.cancellation.check()?;
                    }
                    let mut note = None;
                    let result = match edit {
                        Edit::AddText {
                            spans,
                            page,
                            text,
                            rect,
                            size,
                            color,
                            format,
                        } => if spans.is_empty() {
                            content::add_text_styled(&bytes, page, &text, rect, size, color, &format)?
                        } else {
                            mypdf_milestone0::rich_text::add_aligned(&bytes, page, &text, rect, &spans, format.alignment)?
                        },
                        Edit::Text {
                            page,
                            ordinal,
                            original,
                            replacement,
                        } => {
                            let runs = document::text_runs(&bytes, page)?;
                            let run = runs
                                .get(ordinal)
                                .ok_or_else(|| anyhow::anyhow!("Text object no longer exists"))?;
                            let (bytes, families) = document::replace_text_object_reporting(
                                &bytes,
                                page,
                                run.operator_index,
                                &original,
                                &replacement,
                            )?;
                            if !families.is_empty() {
                                note = Some(format!(
                                    "This PDF’s font lacks some of these characters, so the text now uses {}.",
                                    families.join(", ")
                                ));
                            }
                            bytes
                        }
                        Edit::Image { page, index, edit } => {
                            content::edit_image(&bytes, page, index, &edit)?
                        }
                        Edit::Field { id, value } => forms::fill(&bytes, id, &value)?,
                        Edit::AddField {
                            page,
                            rect,
                            kind,
                            properties,
                        } => {
                            note = Some(format!(
                                "{} added • Undo is available",
                                kind.label()
                            ));
                            form_design::add(&bytes, page, rect, kind, &properties)?.0
                        }
                        Edit::UpdateField {
                            field,
                            widget,
                            properties,
                        } => {
                            note = Some("Field updated • Undo is available".into());
                            form_design::update(&bytes, field, widget, &properties)?
                        }
                        Edit::PlaceField { widget, rect } => form_design::place(&bytes, widget, rect)?,
                        Edit::RemoveField { field, widget } => {
                            note = Some("Field deleted • Undo brings it back".into());
                            form_design::remove(&bytes, field, widget)?
                        }
                        Edit::FieldImage { field, image } => form_design::set_image(&bytes, field, &image)?,
                        Edit::ClearForm => {
                            let (bytes, n) = form_design::clear(&bytes)?;
                            note = Some(format!(
                                "Cleared {n} field{} • Undo is available",
                                if n == 1 { "" } else { "s" }
                            ));
                            bytes
                        }
                        Edit::FlattenForm => {
                            let (bytes, n) = form_design::flatten(&bytes)?;
                            anyhow::ensure!(n > 0, "This PDF has no form fields to flatten");
                            note = Some(format!(
                                "Flattened {n} field{} into the page • Undo is available until you save",
                                if n == 1 { "" } else { "s" }
                            ));
                            bytes
                        }
                        Edit::DetectFields => {
                            let found = mypdf_milestone0::form_detect::detect(&bytes)?;
                            anyhow::ensure!(
                                !found.is_empty(),
                                "No new fields found. Add fields with the tools in the toolbar."
                            );
                            let (bytes, n) =
                                mypdf_milestone0::form_detect::add_detected(&bytes, &found)?;
                            note = Some(format!(
                                "Found and added {n} field{} • Check their names and types, Undo removes them",
                                if n == 1 { "" } else { "s" }
                            ));
                            bytes
                        }
                        Edit::TabOrder(order) => {
                            note = Some(format!(
                                "Tab order set {} • The numbers show the order",
                                match order {
                                    form_design::TabOrder::Rows => "row by row",
                                    form_design::TabOrder::Columns => "column by column",
                                }
                            ));
                            form_design::set_tab_order(&bytes, order)?
                        }
                        Edit::Watermark(watermark) => {
                            let (bytes, n) = mypdf_milestone0::watermark::apply(&bytes, &watermark)?;
                            note = Some(format!(
                                "Watermark added to {n} page{} • Undo removes it",
                                if n == 1 { "" } else { "s" }
                            ));
                            bytes
                        }
                        Edit::RemoveWatermarks => {
                            let (bytes, n) = mypdf_milestone0::watermark::remove(&bytes)?;
                            note = Some(format!(
                                "Removed the watermark from {n} page{} • Undo brings it back",
                                if n == 1 { "" } else { "s" }
                            ));
                            bytes
                        }
                        Edit::FillDetails(chosen) => {
                            let bytes = mypdf_milestone0::form_profile::fill(&bytes, &chosen)?;
                            note = Some(format!(
                                "Filled {} field{} with your details • Undo is available",
                                chosen.len(),
                                if chosen.len() == 1 { "" } else { "s" }
                            ));
                            bytes
                        }
                        Edit::Navigation{bookmarks,labels} => {
                            let mut result=bytes;
                            if let Some(b)=bookmarks {result=mypdf_milestone0::navigation_edit::set_bookmarks(&result,&b)?;}
                            if let Some(l)=labels {result=mypdf_milestone0::navigation_edit::set_labels(&result,&l)?;}
                            result
                        }
                        Edit::RecognizeText(progress) => {
                            let (bytes, r) = mypdf_milestone0::ocr::recognize_cancellable(
                                &bytes,
                                &progress.cancellation,
                                &|state| progress.update(state),
                            )?;
                            note = Some(format!(
                                "Text recognized on {} page{} • Search, select and copy now work{} • Save to keep it",
                                r.pages,
                                if r.pages == 1 { "" } else { "s" },
                                if r.skipped > 0 {
                                    format!(" • {} page{} already had text", r.skipped, if r.skipped == 1 { "" } else { "s" })
                                } else {
                                    String::new()
                                }
                            ));
                            bytes
                        }
                        Edit::ReduceSize(level) => {
                            let (bytes, r) = mypdf_milestone0::compress::reduce(&bytes, level)?;
                            note = Some(format!(
                                "Reduced from {} to {} ({}% smaller) • Save to keep it",
                                super::shrink::size(r.before),
                                super::shrink::size(r.after),
                                100 - r.after * 100 / r.before.max(1)
                            ));
                            bytes
                        }
                        Edit::InsertLayout { page, rect, layout } => {
                            let (bytes, n) =
                                mypdf_milestone0::form_layout::insert(&bytes, page, rect, &layout)?;
                            note = Some(format!(
                                "{} added with {n} field{} • Undo removes it",
                                layout.title(),
                                if n == 1 { "" } else { "s" }
                            ));
                            bytes
                        }
                        Edit::ImportFormData { data, format } => {
                            let (bytes, report) =
                                mypdf_milestone0::form_data::import(&bytes, &data, format)?;
                            let mut parts = vec![format!(
                                "Filled {} field{}",
                                report.filled,
                                if report.filled == 1 { "" } else { "s" }
                            )];
                            if !report.unknown.is_empty() {
                                parts.push(format!(
                                    "not in this form: {}",
                                    report.unknown.join(", ")
                                ));
                            }
                            for (name, reason) in &report.rejected {
                                parts.push(format!("{name}: {reason}"));
                            }
                            note = Some(parts.join(" • "));
                            bytes
                        }
                        Edit::Block { page, block } => text_block::replace(&bytes, page, &block)?,
                        Edit::AddImage { page, rect, image } => {
                            note = Some(
                                "Image added to the page • Edit → Images moves, resizes or replaces it"
                                    .into(),
                            );
                            content::add_image(&bytes, page, rect, &image)?
                        }
                        Edit::Redact { areas, options } => {
                            let (bytes, r) =
                                mypdf_milestone0::redact::apply(&bytes, &areas, &options)?;
                            let mut parts = vec![];
                            for (n, what) in [
                                (r.glyphs, "character"),
                                (r.images, "image area"),
                                (r.annotations, "annotation"),
                                (r.flattened, "flattened page"),
                            ] {
                                if n > 0 {
                                    parts.push(format!("{n} {what}{}", if n == 1 { "" } else { "s" }));
                                }
                            }
                            note = Some(format!(
                                "Redacted {} • Save to keep it; Undo is available until then",
                                if parts.is_empty() {
                                    "the marked areas".to_owned()
                                } else {
                                    parts.join(", ")
                                }
                            ));
                            bytes
                        }
                        Edit::Flatten { dpi } => {
                            let (bytes, pages) =
                                mypdf_milestone0::redact::flatten_document(&bytes, dpi)?;
                            note = Some(format!(
                                "Flattened {pages} page{} into images • Save to keep it; it can still be signed",
                                if pages == 1 { "" } else { "s" }
                            ));
                            bytes
                        }
                        Edit::HeaderFooter(stamp) => {
                            let (bytes, pages) =
                                mypdf_milestone0::header_footer::apply(&bytes, &stamp)?;
                            note = Some(format!(
                                "Added to {pages} page{} • Save to keep your changes",
                                if pages == 1 { "" } else { "s" }
                            ));
                            bytes
                        }
                        Edit::RemoveHeaderFooter(kinds) => {
                            let (bytes, pages) =
                                mypdf_milestone0::header_footer::remove(&bytes, &kinds)?;
                            anyhow::ensure!(
                                pages > 0,
                                "No header or footer text added by MyPDF was found"
                            );
                            note = Some(format!(
                                "Removed from {pages} page{} • Undo restores it",
                                if pages == 1 { "" } else { "s" }
                            ));
                            bytes
                        }
                    };
                    Ok((id, revision, result, note))
                }))
                .map_err(|_| "The PDF could not be edited. Your document is unchanged.".into())
                .and_then(|r| r.map_err(|e| format!("{e:#}")));
            let _ = tx.send(result);
            wake.request_repaint();
        });
    }
    pub(super) fn inspect_content(&mut self, ctx: &egui::Context) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let Some(s) = &tab.session else { return };
        let key = (tab.id, s.revision(), tab.page);
        if self.content.key == Some(key) || self.content.loading {
            return;
        }
        self.content.key = Some(key);
        self.content.selected = None;
        self.content.inspection = None;
        self.content.loading = true;
        let bytes = s.source.clone();
        let tx = self.content.inspect_tx.clone();
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let run = || -> Inspection {
                let text = mypdf_milestone0::text_geometry::runs(&bytes, key.2 as u32 + 1)
                    .map_err(|e| format!("{e:#}"));
                let images =
                    content::images(&bytes, key.2 as u32 + 1).map_err(|e| format!("{e:#}"));
                let fields = forms::fields(&bytes).map_err(|e| format!("{e:#}"));
                let designed =
                    mypdf_milestone0::form_design::describe(&bytes).map_err(|e| format!("{e:#}"));
                Inspection {
                    key,
                    text,
                    images,
                    fields,
                    designed,
                }
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run))
                .unwrap_or_else(|_| Inspection {
                    key,
                    text: Err("Unsupported page content".into()),
                    images: Err("Unsupported images".into()),
                    fields: Err("Unsupported form structure".into()),
                    designed: Err("Unsupported form structure".into()),
                });
            let _ = tx.send(result);
            wake.request_repaint();
        });
    }
    pub(super) fn content_panel(&mut self, ctx: &egui::Context) {
        let mode = self.editor.mode;
        if mode == editing::Mode::Convert {
            self.conversion_panel(ctx);
            return;
        }
        if mode != editing::Mode::Edit {
            return;
        }
        if self.content.add_target.is_some() {
            self.new_text_panel(ctx);
            return;
        }
        self.inspect_content(ctx);
        if !self.content.image_mode {
            self.existing_text_panel(ctx);
            return;
        }
        let editable = self
            .tabs
            .get(self.active)
            .and_then(|t| t.session.as_ref())
            .is_some_and(|s| s.editable)
            && !self.content.busy
            && !self.organizer.busy;
        let page = self.tabs.get(self.active).map_or(0, |t| t.page);
        let mut change = None;
        let mut replace = None;
        let mut remove_background = None;
        if let (Some(info), Some(index), Some(session)) = (
            &self.content.inspection,
            self.content.selected,
            self.tabs.get(self.active).and_then(|t| t.session.as_ref()),
        ) && let Ok(images) = &info.images
            && let Some(image) = images.get(index)
        {
            let index = image.index;
            let key = (info.key, index);
            if self
                .content
                .image_effects
                .as_ref()
                .is_none_or(|e| e.key != key)
            {
                let source = content::image_png(&session.source, page as u32 + 1, index)
                    .map_err(|e| e.to_string());
                self.content.image_effects = Some(image_effects::EmbeddedEffects::new(key, source));
            }
        }

        egui::SidePanel::right("content_inspector").default_width(310.).width_range(280.0..=380.0).resizable(true).frame(egui::Frame::new().fill(theme::paper()).inner_margin(18.)).show(ctx,|ui|{
            egui::ScrollArea::vertical().id_salt("content_image_scroll").show(ui,|ui|{
            ui.heading("Edit PDF");ui.label(RichText::new(format!("Page {}",page+1)).small().color(theme::muted()));ui.add_space(12.);
            if ui.add_enabled(editable && !self.content.busy, ActionButton::new(Icon::Crop, "Copy region").selected(self.content.region.active)).on_hover_text("Draw a rectangle around any artwork, logo or scanned content to copy it into an editable image layer").clicked() {
                self.content.region.active = !self.content.region.active;
                self.content.region.drag = None;
                self.content.selected = None;
                self.content.region.key = self.tabs.get(self.active).and_then(|t| t.session.as_ref().map(|s| (t.id, s.revision(), t.page)));
            }
            if self.content.region.active { ui.label("Drag around the artwork on the page. Esc cancels."); }
            ui.add_space(8.);
            if self.content.loading{ui.spinner();ui.label("Inspecting page…");return;}
            let Some(info)=&self.content.inspection else{ui.label("Open a PDF to begin.");return};
            if !editable{ui.label(RichText::new(if self.content.busy{"Applying changes…"}else{"This document is read-only."}).color(theme::muted()));}
            {
                ui.horizontal(|ui|{if ui.selectable_value(&mut self.content.image_mode,false,"Text").changed()|ui.selectable_value(&mut self.content.image_mode,true,"Images").changed(){self.content.selected=None;}});ui.add_space(8.);
                if self.content.image_mode{
                    match &info.images{Err(e)=>{ui.label(e);},Ok(images)=>{
                        ui.label(RichText::new("Select an image here or on the page.").small().color(theme::muted()));
                        egui::ScrollArea::vertical().id_salt("image_objects").max_height(205.).show(ui,|ui|{for (i,image) in images.iter().enumerate(){if ui.selectable_label(self.content.selected==Some(i),format!("{}  ·  {} × {} px",image.name,image.pixels[0],image.pixels[1])).clicked(){self.content.selected=Some(i);self.content.rect=[image.rect[0],image.rect[1],image.rect[2]-image.rect[0],image.rect[3]-image.rect[1]];}}});
                        if images.is_empty(){ui.label("No separate image objects found. Use Copy region for logos, grouped artwork or scanned page details.");}
                        if let Some(image)=self.content.selected.and_then(|i|images.get(i)){
                            ui.separator();ui.label("Position & size · points");
                            ui.add_enabled_ui(editable,|ui|{egui::Grid::new("image_dimensions").num_columns(2).spacing([12.,10.]).show(ui,|ui|{for(label,v)in ["X","Y","Width","Height"].into_iter().zip(&mut self.content.rect){ui.label(label);ui.add(egui::DragValue::new(v).speed(1.).range(-99_999.0..=99_999.0));ui.end_row();}});ui.add_space(8.);
                                if ui.add(ActionButton::new(Icon::Select,"Apply position & size").primary()).clicked(){let [x,y,w,h]=self.content.rect;change=Some(Edit::Image{page:page as u32+1,index:image.index,edit:content::ImageEdit::Place([x,y,x+w,y+h])});}
                                if ui.add(ActionButton::new(Icon::Import,"Replace image…")).clicked(){replace=Some(image.index);}
                                if ui.add(ActionButton::new(Icon::FieldImage,"Remove background…")).clicked(){remove_background=Some(image.index);}
                                if let Some(effects) = &mut self.content.image_effects && effects.key == (info.key, image.index) {
                                    ui.separator();
                                    if let Some(png) = effects.panel(ui) { change=Some(Edit::Image{page:page as u32+1,index:image.index,edit:content::ImageEdit::Replace(png)}); }
                                }
                                if ui.add(ActionButton::new(Icon::Trash,"Delete image")).clicked(){change=Some(Edit::Image{page:page as u32+1,index:image.index,edit:content::ImageEdit::Delete});}
                            });
                        }
                        ui.add_space(12.);ui.label(RichText::new("Page-level images only. Existing clipping paths remain in effect.").small().color(theme::muted()));
                    }}
                }else{
                    ui.label("Click text on the page to edit it.");
                }
            }
        });
        });
        if let Some(index) = remove_background {
            self.remove_content_image_background(index);
        }
        if let Some(index) = replace
            && let Some(path) = self.native_pick(
                rfd::FileDialog::new()
                    .set_title("Replace image • PNG or JPEG")
                    .add_filter("Images", &["png", "jpg", "jpeg"]),
            )
        {
            match convert::read_bounded(&path, 32 * 1024 * 1024) {
                Ok(bytes) => {
                    change = Some(Edit::Image {
                        page: page as u32 + 1,
                        index,
                        edit: content::ImageEdit::Replace(bytes),
                    })
                }
                Err(e) => self.status = Some((format!("{e:#}"), true)),
            }
        }
        if let Some(change) = change {
            self.content_change(ctx, change);
        }
    }
    pub(super) fn new_blank(&mut self, ctx: &egui::Context) {
        if self.tabs.len() >= 16 {
            self.status = Some(("Close a tab before creating another PDF.".into(), true));
            return;
        }
        match convert::blank(595., 842.) {
            Ok(bytes) => {
                let path = PathBuf::from(format!("Untitled-{}.pdf", self.next_id + 1));
                self.open_memory(path, bytes, ctx);
                self.editor.mode = editing::Mode::Annotate;
                self.status = Some((
                    "New blank A4 PDF • Add text, artwork or pages • Save when ready".into(),
                    false,
                ));
            }
            Err(e) => self.status = Some((format!("{e:#}"), true)),
        }
    }
    pub(super) fn open_memory(&mut self, path: PathBuf, bytes: Vec<u8>, ctx: &egui::Context) {
        self.open(path, ctx);
        if let Some(tab) = self.tabs.get_mut(self.active) {
            let source: Arc<[u8]> = bytes.into();
            tab.bytes = Some(source.clone());
            tab.unsaved_on_load = true;
            tab.has_destination = false;
            tab.remember_open = false;
            let wake = ctx.clone();
            tab.worker = Worker::from_bytes(source, move || wake.request_repaint());
        }
    }
    pub(super) fn content_tools(&mut self, ctx: &egui::Context) {
        if self.editor.mode != editing::Mode::Edit {
            return;
        }
        if self.content.add_target.is_none() {
            self.inspect_content(ctx);
        }
        let editable = self
            .tabs
            .get(self.active)
            .and_then(|t| t.session.as_ref())
            .is_some_and(|s| s.editable)
            && !self.content.busy
            && !self.organizer.busy;
        egui::TopBottomPanel::top("edit_ribbon")
            .frame(
                egui::Frame::new()
                    .fill(theme::surface())
                    .inner_margin(egui::Margin::symmetric(16, 7)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(((ui.available_width() - 820.) / 2.).max(0.));
                    if ui
                        .add_enabled(
                            editable,
                            ActionButton::new(Icon::Text, "Add text")
                                .selected(self.content.add_target.is_some()),
                        )
                        .clicked()
                        && let Some(t) = self.tabs.get(self.active)
                        && !t.pages.is_empty()
                    {
                        self.redaction.active = false;
                        self.content.area_mode = false;
                        self.content.marquee = None;
                        if self.content.existing.is_some() {
                            self.content.new_commit = true;
                            self.content.add_after_existing = Some((t.id, t.page));
                        } else if self.content.pending_text().is_none() {
                            self.content.add_target = Some((t.id, t.page));
                            self.content.new_placed = false;
                            self.content.clear_new_text();
                        }
                        self.inspector = true;
                    }
                    ui.separator();
                    if ui
                        .add(ActionButton::new(Icon::Select, "Edit text").selected(
                            !self.content.image_mode
                                && !self.content.area_mode
                                && self.content.add_target.is_none()
                                && !self.redaction.active,
                        ))
                        .clicked()
                    {
                        self.redaction.active = false;
                        self.content.area_mode = false;
                        self.content.marquee = None;
                        if self.content.pending_text().is_some() {
                            self.content.new_commit = true;
                            self.content.exit_text_tool = true;
                        } else {
                            self.content.add_target = None;
                        }
                        self.content.image_mode = false;
                        self.inspector = true;
                    }
                    if ui
                        .add_enabled(
                            editable,
                            ActionButton::new(Icon::TextArea, "Select area").selected(
                                self.content.area_mode
                                    && self.content.add_target.is_none()
                                    && !self.redaction.active,
                            ),
                        )
                        .on_hover_text(
                            "Drag a rectangle around several lines to edit them as one block",
                        )
                        .clicked()
                    {
                        self.redaction.active = false;
                        if self.content.pending_text().is_some() {
                            self.content.new_commit = true;
                            self.content.exit_text_tool = true;
                        } else {
                            self.content.add_target = None;
                        }
                        self.content.image_mode = false;
                        self.content.area_mode = true;
                        self.content.marquee = None;
                        self.content.selection_hint = None;
                        self.inspector = true;
                    }
                    if ui
                        .add(ActionButton::new(Icon::Import, "Images").selected(
                            self.content.image_mode
                                && self.content.add_target.is_none()
                                && !self.redaction.active,
                        ))
                        .clicked()
                    {
                        self.redaction.active = false;
                        self.content.area_mode = false;
                        self.content.marquee = None;
                        if self.content.pending_text().is_some() {
                            self.content.new_commit = true;
                            self.content.exit_text_tool = true;
                        } else {
                            self.content.add_target = None;
                        }
                        self.content.image_mode = true;
                        self.inspector = true;
                    }
                    if ui
                        .add_enabled(editable, ActionButton::new(Icon::Plus, "Add image…"))
                        .on_hover_text("Place a PNG or JPEG on this page (or paste one with ⌘V)")
                        .clicked()
                    {
                        self.add_image_file(ctx);
                    }
                    ui.separator();
                    self.header_footer_menu(ui, editable);
                    if ui
                        .add_enabled(editable, ActionButton::new(Icon::Watermark, "Watermark"))
                        .on_hover_text("CONFIDENTIAL, DRAFT or a picture across each page")
                        .clicked()
                    {
                        self.open_watermark();
                    }
                    if ui
                        .add_enabled(
                            editable,
                            ActionButton::new(Icon::Redact, "Redact")
                                .selected(self.redaction.active),
                        )
                        .on_hover_text("Mark text, images or areas and remove them for good")
                        .clicked()
                    {
                        if self.redaction.active {
                            self.redaction.active = false;
                        } else if self.content.pending_text().is_some() {
                            self.content.new_commit = true;
                            self.content.exit_text_tool = true;
                        } else {
                            self.start_redaction();
                        }
                    }
                });
            });
    }
    fn new_text_panel(&mut self, ctx: &egui::Context) {
        self.content.sync_new_styles();
        let editable = !self.content.busy
            && !self.organizer.busy
            && self
                .tabs
                .get(self.active)
                .is_some_and(|t| t.session.as_ref().is_some_and(|s| s.editable));
        egui::SidePanel::right("new_text_panel").default_width(270.).width_range(250.0..=340.0).resizable(true).frame(egui::Frame::new().fill(theme::paper()).inner_margin(16.)).show(ctx,|ui| {
            ui.spacing_mut().item_spacing=vec2(8.,10.);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Text").size(17.).family(ui::semibold()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| { if ui.add(ActionButton::new(Icon::Close, "Hide text panel").compact()).clicked() { self.inspector=false; } });
            });
            ui.add_space(8.);
            let (sample,_) = ui.allocate_exact_size(vec2(ui.available_width(),96.),egui::Sense::hover());
            ui.painter().rect_filled(sample,7.,Color32::WHITE);
            ui.painter().rect_stroke(sample,7.,Stroke::new(1.,Color32::from_rgb(222,223,229)),egui::StrokeKind::Inside);
            let rgb=self.content.new_color;
            let color=Color32::from_rgb((rgb[0]*255.) as u8,(rgb[1]*255.) as u8,(rgb[2]*255.) as u8);
            ui.painter().text(sample.center(),egui::Align2::CENTER_CENTER,"Sample",self.content.text_font(self.content.new_size.clamp(18.,32.)),color);
            ui.add_space(10.);
            egui::ScrollArea::vertical().id_salt("new_text_properties_scroll").show(ui, |ui| {
            ui.add_enabled_ui(editable,|ui| {
                formatting::panel(ui, &self.content.new_text, &mut self.content.new_spans, &self.content.new_selection,
                    &mut self.content.new_font, &mut self.content.new_size, &mut self.content.new_color, &mut self.content.new_alignment);
                ui.add_space(8.);
                if self.content.new_placed {
                    egui::CollapsingHeader::new("Position & size").show(ui,|ui| {
                        if let Some(page) = self.tabs.get(self.active).and_then(|t| t.pages.get(t.page)) {
                            for (index, name) in ["Left", "Top", "Width", "Height"].into_iter().enumerate() {
                                let max = match index { 0 => (page.width-self.content.new_rect[2]).max(0.), 1 => (page.height-self.content.new_rect[3]).max(0.), 2 => (page.width-self.content.new_rect[0]).max(10.), _ => (page.height-self.content.new_rect[1]).max(4.) };
                                editing::property_slider(ui, name, &mut self.content.new_rect[index], (if index == 2 {10.} else if index == 3 {4.} else {0.})..=max, " pt");
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!self.content.new_text.trim().is_empty(),ActionButton::new(Icon::Check,"Done").primary()).clicked() { self.content.new_commit=true; }
                        if ui.add(ActionButton::new(Icon::Left,"Cancel")).clicked() { self.content.clear_new_text(); self.content.new_placed=false; self.content.next_text=None; }
                    });
                }
            });
            ui.add_space(8.);
            ui.label(RichText::new(if self.content.new_placed { "Type on the page. Drag the top handle to move; drag the circles to resize. Click elsewhere on the page or press ⌘Return to finish." } else { "Click anywhere on a page and start typing." }).size(12.).color(theme::muted()));
            });
        });
    }
    pub(super) fn finish_new_text(&mut self, ctx: &egui::Context) {
        if !self.content.new_commit || self.content.busy {
            return;
        }
        self.content.new_commit = false;
        if self.content.existing.is_some() {
            self.finish_existing_text(ctx);
            return;
        }
        let Some((id, page)) = self.content.add_target else {
            return;
        };
        if self.content.new_text.trim().is_empty() {
            return;
        }
        let Some(index) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        self.content.sync_new_styles();
        let edit = Edit::AddText {
            spans: self.content.new_spans.clone(),
            page: page as u32 + 1,
            text: self.content.new_text.clone(),
            rect: self.content.new_rect,
            size: self.content.new_size,
            color: self.content.new_color,
            format: TextFormat {
                font: text_style::fonts()
                    .get(self.content.new_font)
                    .map(|f| f.path.clone()),
                alignment: self.content.new_alignment,
            },
        };
        let previous = self.active;
        self.active = index;
        self.content_change(ctx, edit);
        self.content.inserting_text = self.content.busy;
        if !self.content.busy {
            self.content.after_text = None;
        }
        self.active = previous;
    }
    fn conversion_panel(&mut self, ctx: &egui::Context) {
        let mut action = "";
        let ready = self
            .tabs
            .get(self.active)
            .is_some_and(|t| !t.pages.is_empty());
        egui::SidePanel::right("conversion_panel").exact_width(310.).resizable(false).frame(egui::Frame::new().fill(theme::paper()).inner_margin(18.)).show(ctx,|ui|{
            ui.heading("Export");ui.label(RichText::new("Local files. Fresh output copies.").small().color(theme::muted()));ui.add_space(16.);
            ui.add_enabled_ui(!self.content.busy,|ui|{
                ui.label(RichText::new("Create a PDF").family(ui::semibold()));
                for(icon,label,key)in[(Icon::Plus,"New blank PDF","blank"),(Icon::Import,"Images to PDF…","images"),(Icon::Text,"Markdown / Word to PDF…","document")]{if ui.add(ActionButton::new(icon,label)).clicked(){action=key;}}
                ui.add_space(18.);ui.separator();ui.add_space(12.);ui.label(RichText::new("Export this PDF").family(ui::semibold()));
                ui.add_enabled_ui(ready,|ui|{
                    egui::ComboBox::from_id_salt("conversion_format").selected_text(match self.content.format{Format::Png=>"PNG images",Format::Jpeg=>"JPEG images",Format::Text=>"Plain text (.txt)",Format::Markdown=>"Markdown (.md)",Format::Word=>"Word (.docx)"}).width(270.).show_ui(ui,|ui|{for(f,label)in[(Format::Png,"PNG images"),(Format::Jpeg,"JPEG images"),(Format::Text,"Plain text (.txt)"),(Format::Markdown,"Markdown (.md)"),(Format::Word,"Word (.docx)")]{ui.selectable_value(&mut self.content.format,f,label);}});
                    ui.add_space(10.);ui.label("Pages · e.g. 1, 3-5");let r=ui.text_edit_singleline(&mut self.content.range);self.editor.text_editing|=r.has_focus();
                    if ui.small_button("All pages").clicked() && let Some(t)=self.tabs.get(self.active){self.content.range=format!("1-{}",t.pages.len());}
                    if matches!(self.content.format,Format::Png|Format::Jpeg){ui.horizontal(|ui|{ui.label("Resolution");ui.add(egui::DragValue::new(&mut self.content.dpi).range(72.0..=300.0).suffix(" DPI"));});}
                    ui.add_space(12.);if ui.add(ActionButton::new(Icon::Extract,"Export…").primary()).clicked(){action="export";}
                });
                ui.add_space(18.);ui.separator();ui.add_space(12.);
                if ui.add_enabled(ready,ActionButton::new(Icon::Library,"Multiple pages per sheet…")).clicked(){action="nup";}
                ui.label(RichText::new("Arrange smaller pages on new sheets, with margins, headers, and a watermark.").small().color(theme::muted()));
                ui.add_space(18.);ui.separator();ui.add_space(12.);ui.label(RichText::new("Flatten").family(ui::semibold()));
                ui.label(RichText::new("Turn every page into an image so nothing on it can be edited. Annotations and form fields are burned in; the result can still be signed.").small().color(theme::muted()));
                if ui.add_enabled(ready,ActionButton::new(Icon::Document,"Flatten PDF…")).clicked(){action="flatten";}
                ui.add_space(18.);ui.separator();ui.add_space(12.);ui.label(RichText::new("Reduce file size").family(ui::semibold()));
                ui.label(RichText::new("Make pictures and scans smaller for email or upload. Text and drawings stay sharp.").small().color(theme::muted()));
                if ui.add_enabled(ready,ActionButton::new(Icon::Extract,"Reduce File Size…")).clicked(){action="shrink";}
                ui.add_space(18.);ui.separator();ui.add_space(12.);ui.label(RichText::new("Recognize text").family(ui::semibold()));
                ui.label(RichText::new("Make scanned pages searchable, so their words can be found, selected and copied. Happens on this Mac.").small().color(theme::muted()));
                if ui.add_enabled(ready,ActionButton::new(Icon::Ocr,"Recognize Text…")).clicked(){action="ocr";}
                ui.add_space(16.);ui.label(RichText::new("Word and Markdown conversions use document text and basic paragraphs. Fonts, columns, tables, headers and page layout are not reconstructed. Word import accepts text documents; drawings are reported. Scanned pages need Recognize Text first.").small().color(theme::muted()));
            });if self.content.busy{ui.spinner();ui.label("Converting…");}
        });
        match action {
            "shrink" => self.request_shrink(),
            "ocr" => self.request_ocr(),
            "blank" => self.new_blank(ctx),
            "images" | "document" => self.import_conversion(ctx, action == "images"),
            "export" => self.export_conversion(ctx),
            "nup" => self.open_nup(),
            "flatten" => self.request_flatten(),
            _ => {}
        }
    }
    pub(super) fn import_conversion(&mut self, ctx: &egui::Context, images: bool) {
        let paths = if images {
            self.native_picks(
                rfd::FileDialog::new()
                    .set_title("Images to PDF • one image per page at 144 DPI")
                    .add_filter("Images", &["png", "jpg", "jpeg"]),
            )
        } else {
            self.native_pick(
                rfd::FileDialog::new()
                    .set_title("Text document to PDF")
                    .add_filter("Text documents", &["md", "txt", "docx"]),
            )
            .map(|p| vec![p])
        };
        let Some(paths) = paths else { return };
        let Some(destination) = self.native_save(
            rfd::FileDialog::new()
                .set_title("Create PDF • choose a new filename")
                .add_filter("PDF", &["pdf"])
                .set_file_name("converted.pdf"),
        ) else {
            return;
        };
        let tx = self.content.output_tx.clone();
        let wake = ctx.clone();
        self.content.busy = true;
        std::thread::spawn(move || {
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> anyhow::Result<_> {
                    let bytes = if images {
                        anyhow::ensure!(paths.len() <= 200, "Select at most 200 images");
                        let mut inputs = vec![];
                        let mut total = 0;
                        for p in &paths {
                            let input = convert::read_bounded(p, 32 * 1024 * 1024)?;
                            total += input.len();
                            anyhow::ensure!(
                                total <= 256 * 1024 * 1024,
                                "Combined images exceed 256 MB"
                            );
                            inputs.push(input);
                        }
                        convert::images_to_pdf(&inputs, 144.)?
                    } else {
                        let path = &paths[0];
                        let input = convert::read_bounded(path, 32 * 1024 * 1024)?;
                        let word = path
                            .extension()
                            .is_some_and(|e| e.eq_ignore_ascii_case("docx"));
                        let text = if word {
                            convert::docx_text(&input)?
                        } else {
                            String::from_utf8(input)?
                        };
                        convert::text_to_pdf(
                            &text,
                            word || path
                                .extension()
                                .is_some_and(|e| e.eq_ignore_ascii_case("md")),
                        )?
                    };
                    mypdf_milestone0::write_new(&destination, &bytes)?;
                    Ok((
                        format!("Created {}", destination.display()),
                        Some(destination),
                    ))
                }))
                .map_err(|_| "Conversion could not be completed. Inputs are unchanged.".into())
                .and_then(|r| r.map_err(|e| format!("{e:#}")));
            let _ = tx.send(result);
            wake.request_repaint();
        });
    }
    fn export_conversion(&mut self, ctx: &egui::Context) {
        if !self.apply_pending_annotation() {
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let Some(session) = &tab.session else { return };
        let pages = match mypdf_milestone0::pages::parse_range(&self.content.range, tab.pages.len())
        {
            Ok(p) => p,
            Err(e) => {
                self.status = Some((format!("{e:#}"), true));
                return;
            }
        };
        let source = session.source.clone();
        let annotations = session.annotations.clone();
        let revision = session.revision();
        let format = self.content.format;
        let dpi = self.content.dpi;
        let stem = tab
            .path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let destination = if matches!(format, Format::Png | Format::Jpeg) {
            menu::with_file_dialog(self._native_menu.as_ref(), || {
                rfd::FileDialog::new()
                    .set_title("Choose parent folder • creates a fresh image folder")
                    .pick_folder()
            })
            .map(|p| p.join(format!("{stem}-images-{:08x}", rand::random::<u32>())))
        } else {
            self.native_save(
                rfd::FileDialog::new()
                    .set_title("Export text • choose a new filename")
                    .set_file_name(format!("{stem}.{}", format.extension()))
                    .set_directory(tab.path.parent().unwrap_or(std::path::Path::new("."))),
            )
        };
        let Some(mut destination) = destination else {
            return;
        };
        if !matches!(format, Format::Png | Format::Jpeg) {
            match destination.extension() {
                None => {
                    destination.set_extension(format.extension());
                }
                Some(ext) if ext.eq_ignore_ascii_case(format.extension()) => {}
                _ => {
                    self.status = Some((
                        format!(
                            "Use a .{} filename for this conversion.",
                            format.extension()
                        ),
                        true,
                    ));
                    return;
                }
            }
        }
        let tx = self.content.output_tx.clone();
        let wake = ctx.clone();
        self.content.busy = true;
        std::thread::spawn(move || {
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> anyhow::Result<_> {
                    let bytes = if revision == 0 {
                        source.to_vec()
                    } else {
                        mypdf_milestone0::annotations::export(&source, &annotations)?
                    };
                    if matches!(format, Format::Png | Format::Jpeg) {
                        convert::export_images(&bytes, &pages, format, dpi, &destination)?;
                    } else {
                        let out = convert::export_text(&bytes, &pages, format)?;
                        mypdf_milestone0::write_new(&destination, &out)?;
                    }
                    Ok((format!("Exported {}", destination.display()), None))
                }))
                .map_err(|_| "Conversion failed; source PDF is unchanged.".into())
                .and_then(|r| r.map_err(|e| format!("{e:#}")));
            let _ = tx.send(result);
            wake.request_repaint();
        });
    }
}

impl Viewer {
    fn begin_add_after_existing(&mut self) {
        if let Some(target) = self.content.add_after_existing.take() {
            self.content.add_target = Some(target);
            self.content.new_placed = false;
            self.content.clear_new_text();
        }
    }
    fn finish_existing_text(&mut self, ctx: &egui::Context) {
        let Some(draft) = self.content.existing.clone() else {
            return;
        };
        let Some(index) = self.tabs.iter().position(|t| t.id == draft.key.0) else {
            return;
        };
        let tab = &mut self.tabs[index];
        let Some(session) = &mut tab.session else {
            return;
        };
        if session.revision() != draft.key.1 || !session.editable {
            self.content.after_text = None;
            self.content.next_existing = None;
            self.status = Some((
                "This document changed. Cancel the text edit and select it again.".into(),
                true,
            ));
            return;
        }
        if draft.changed() {
            match draft.target {
                InlineTarget::Block(block) => {
                    let previous = self.active;
                    self.active = index;
                    self.content_change(
                        ctx,
                        Edit::Block {
                            page: draft.key.2 as u32 + 1,
                            block: text_block::Replacement {
                                layout: if block.preserve_layout
                                    && block.alignment == Alignment::Left
                                {
                                    mypdf_milestone0::rich_text::edited_layout(
                                        &block.styled_text,
                                        &draft.text,
                                        &block.layout,
                                        draft.size * 1.3,
                                    )
                                } else {
                                    vec![]
                                },
                                spans: mypdf_milestone0::rich_text::edited(
                                    &block.styled_text,
                                    &draft.text,
                                    &block.spans,
                                ),
                                selection: block.selection,
                                text: draft.text,
                                rect: [
                                    draft.rect[0],
                                    draft.rect[1],
                                    draft.rect[2] - draft.rect[0],
                                    draft.rect[3] - draft.rect[1],
                                ],
                                size: draft.size,
                                color: draft.color,
                                format: TextFormat {
                                    font: text_style::fonts()
                                        .get(block.font)
                                        .map(|f| f.path.clone()),
                                    alignment: block.alignment,
                                },
                            },
                        },
                    );
                    self.content.replacing_text = self.content.busy;
                    self.active = previous;
                    return;
                }
                InlineTarget::Run(operator) => {
                    let previous = self.active;
                    self.active = index;
                    self.content_change(
                        ctx,
                        Edit::Text {
                            page: draft.key.2 as u32 + 1,
                            ordinal: operator,
                            original: draft.original,
                            replacement: draft.text,
                        },
                    );
                    self.content.replacing_text = self.content.busy;
                    self.active = previous;
                    return;
                }
                InlineTarget::Annotation(id) => {
                    let mut annotations = session.annotations.clone();
                    let Some(a) = annotations.iter_mut().find(|a| a.id == id) else {
                        return;
                    };
                    if draft.text.is_empty() {
                        annotations.retain(|a| a.id != id);
                    } else {
                        let galley = ctx.fonts_mut(|f| {
                            f.layout(
                                draft.text.clone(),
                                FontId::new(
                                    a.style.font_size,
                                    egui::FontFamily::Name("annotation".into()),
                                ),
                                theme::ink(),
                                (a.rect[2] - a.rect[0] - 8.).max(1.),
                            )
                        });
                        let bottom = a.rect[1] + galley.size().y + 8.;
                        if bottom > tab.pages[draft.key.2].height {
                            self.status=Some(("The text would extend beyond the page. Shorten it or enlarge/move the text box.".into(),true));
                            self.content.after_text = None;
                            return;
                        }
                        a.rect[3] = a.rect[3].max(bottom);
                        a.kind = mypdf_milestone0::annotations::Kind::Text { text: draft.text };
                    }
                    if let Err(e) = session.commit(annotations) {
                        self.status = Some((format!("{e:#}"), true));
                        self.content.after_text = None;
                        return;
                    }
                    self.editor.draft = None;
                    self.status = Some(("Text updated • Save to keep your changes".into(), false));
                }
            }
        }
        self.content.existing = self.content.next_existing.take();
        self.content.exit_text_tool = false;
        if let Some(draft) = &mut self.content.existing {
            draft.key.1 = self.tabs[index].session.as_ref().unwrap().revision();
        }
        self.begin_add_after_existing();
        if let Some(action) = self.content.after_text.take() {
            match action {
                AfterText::Save(id, intent) => {
                    if let Some(index) = self.tabs.iter().position(|t| t.id == id) {
                        self.active = index;
                        self.save_intent = intent;
                        self.save_dialog(ctx);
                    }
                }
                AfterText::Close(id) => self.request_close(id, ctx),
                AfterText::Undo(id, page) => self.undo_finished_text(id, page, ctx),
            }
        }
    }
    fn existing_text_panel(&mut self, ctx: &egui::Context) {
        let current = self.tabs.get(self.active).map(|t| (t.id, t.page));
        let page_size = self
            .tabs
            .get(self.active)
            .and_then(|t| t.pages.get(t.page))
            .copied();
        let mut cancel = false;
        let mut add_text = false;
        let editable = self
            .tabs
            .get(self.active)
            .and_then(|t| t.session.as_ref())
            .is_some_and(|s| s.editable)
            && !self.content.busy
            && !self.organizer.busy;
        egui::SidePanel::right("content_inspector").default_width(290.).width_range(270.0..=380.0).resizable(true)
            .frame(egui::Frame::new().fill(theme::paper()).inner_margin(18.))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Text").size(18.).family(ui::semibold()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(ActionButton::new(Icon::Close, "Hide text panel").compact()).clicked() { self.inspector = false; }
                    });
                });
                egui::ScrollArea::vertical().id_salt("text_inspector_scroll").show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(8., 10.);
                    ui.add_space(12.);
                    if let Some(draft) = &mut self.content.existing && current == Some((draft.key.0, draft.key.2)) {
                        ui.label(RichText::new(format!("{} words · {} characters", draft.text.split_whitespace().count(), draft.text.chars().count())).small().color(theme::muted()));
                        if let InlineTarget::Block(block) = &mut draft.target {
                            ui.add_space(12.);
                            ui.add_enabled_ui(!self.content.busy, |ui| {
                                let old_alignment = block.alignment;
                                formatting::panel(ui, &draft.text, &mut block.spans, &block.selected_chars,
                                    &mut block.font, &mut draft.size, &mut draft.color, &mut block.alignment);
                                if old_alignment != block.alignment { block.preserve_layout = false; }
                                ui.add_space(8.);
                                if ui.add_enabled(!block.layout.is_empty(), egui::Checkbox::new(&mut block.preserve_layout, "Preserve original line spacing")).changed() && block.preserve_layout {
                                    block.alignment = Alignment::Left;
                                }
                                ui.label(RichText::new("Keeps line breaks, indentation and paragraph gaps. Turn off for evenly spaced lines.").small().color(theme::muted()));
                                if block.preserve_layout && let Some(first) = block.layout.first_mut() {
                                    let maximum = (draft.rect[2] - draft.rect[0] - draft.size).max(0.);
                                    editing::property_slider(ui, "First line indent", &mut first.left, 0.0..=maximum, " pt");
                                }
                                if block.preserve_layout && block.layout.len() > 1 && let Some(page) = page_size {
                                    let first = block.layout[0].baseline;
                                    let last = block.layout.last().unwrap().baseline;
                                    let gaps = (block.layout.len() - 1) as f32;
                                    let mut spacing = (last - first) / gaps;
                                    let max = ((page.height - draft.rect[1] - first - draft.size * 0.3) / gaps).max(1.);
                                    let before = spacing;
                                    let maximum = max.min(144.).max(spacing);
                                    editing::property_slider(ui, "Line spacing", &mut spacing, 1.0..=maximum, " pt");
                                    if (spacing-before).abs() > 0.001 {
                                        scale_line_spacing(&mut block.layout, spacing);
                                        draft.rect[3] = (draft.rect[1] + block.layout.last().unwrap().baseline + draft.size * 0.3).min(page.height);
                                    }
                                    ui.weak("Adjusts all lines in this region; paragraph gaps stay proportional.");
                                }

                                ui.add_space(8.);
                                ui.separator();
                                ui::caption(ui, "Position & size");
                                if let Some(page) = page_size {
                                    let mut x = draft.rect[0];
                                    let mut y = draft.rect[1];
                                    let mut width = draft.rect[2] - x;
                                    let height = draft.rect[3] - y;
                                    let mut changed = false;
                                    for (label, value, maximum) in [("Left", &mut x, (page.width-width).max(0.)), ("Top", &mut y, (page.height-height).max(0.)), ("Width", &mut width, (page.width-draft.rect[0]).max(1.))] {
                                        let minimum = if label == "Width" {1.} else {0.};
                                        let old = *value;
                                        editing::property_slider(ui, label, value, minimum..=maximum, " pt");
                                        changed |= old != *value;
                                    }
                                    ui.weak(format!("Height: {height:.1} pt · Auto"));
                                    if changed { draft.rect = [x, y, (x+width).min(page.width), (y+height).min(page.height)]; }
                                }
                                ui.label(RichText::new("Narrower widths wrap long lines. Height grows as you type.").small().color(theme::muted()));
                                ui.add_space(8.);

                            });
                        } else {
                            ui.label(RichText::new(&draft.font).color(theme::muted()));
                            ui.label(RichText::new(format!("{:.1} pt", draft.size)).color(theme::muted()));
                        }
                        ui.add_space(18.);
                        ui.add_enabled_ui(!self.content.busy, |ui| {
                            ui.horizontal(|ui| {
                                if ui.add(ActionButton::new(Icon::Check,"Done").primary()).clicked() { self.content.new_commit = true; }
                                cancel = ui.add(ActionButton::new(Icon::Left,"Cancel")).clicked();
                            });
                        });
                        ui.add_space(14.);
                        ui.label(RichText::new("Type on the page. Drag the handle to move PDF text. Click outside or press ⌘Return to finish. Esc cancels.").size(12.).color(theme::muted()));
                    } else {
                        let empty = self.content.inspection.as_ref().is_some_and(|info| info.text.as_ref().is_ok_and(|runs| runs.is_empty()));
                        if empty {
                            ui.label(RichText::new("No editable text on this page.").size(15.).color(theme::ink()));
                            ui.label(RichText::new("Start a new text block anywhere on the page.").color(theme::muted()));
                            add_text = ui.add_enabled(editable, ActionButton::new(Icon::Text, "Add text").primary()).clicked();
                        } else {
                            ui.label(RichText::new(if self.content.area_mode { "Drag a rectangle around the text you want to edit together." } else { "Click text on the page to edit it." }).size(15.).color(theme::ink()));
                            ui.add_space(12.);
                            ui.label(RichText::new("Select area edits several lines together. Add text starts a new block.").color(theme::muted()));
                        }
                        if let Some(hint) = &self.content.selection_hint { ui.add_space(12.); ui.label(RichText::new(hint).color(theme::muted())); }
                        if self.content.loading { ui.add_space(12.); ui.spinner(); }
                        if let Some(info) = &self.content.inspection && let Err(e) = &info.text { ui.add_space(12.); ui.label(RichText::new(e).small().color(theme::muted())); }
                    }
                    if self.content.busy { ui.spinner(); ui.label("Applying text…"); }
                });
            });
        if add_text && let Some((id, page)) = current {
            self.redaction.active = false;
            self.content.area_mode = false;
            self.content.marquee = None;
            if self.content.existing.is_some() {
                self.content.new_commit = true;
                self.content.add_after_existing = Some((id, page));
            } else if self.content.pending_text().is_none() {
                self.content.add_target = Some((id, page));
                self.content.new_placed = false;
                self.content.clear_new_text();
            }
            self.status = Some(("Click anywhere on the page to add text.".into(), false));
        }
        if cancel {
            self.content.existing = None;
            self.content.next_existing = None;
            self.content.after_text = None;
            self.content.add_after_existing = None;
        }
    }
}

fn pdf_preview_font(name: &str) -> egui::FontFamily {
    let name = name.to_ascii_lowercase().replace([' ', '-'], "");
    let bold = name.contains("bold");
    let italic = name.contains("italic") || name.contains("oblique");
    text_style::fonts()
        .iter()
        .find(|f| {
            let family = f.family.to_ascii_lowercase().replace(' ', "");
            (name.contains(&family)
                || (name.contains("helvetica") && family == "arial")
                || (name.contains("courier") && family == "couriernew")
                || (name.contains("times") && family == "timesnewroman"))
                && f.bold == bold
                && f.italic == italic
        })
        .or_else(|| {
            text_style::fonts()
                .iter()
                .find(|f| f.bold == bold && f.italic == italic)
        })
        .or_else(|| text_style::fonts().first())
        .map(|f| egui::FontFamily::Name(f.ui_name().into()))
        .unwrap_or(egui::FontFamily::Proportional)
}

/// A click edits one visual line, including adjacent fragments with different
/// styles. Other lines are included only by the explicit rectangular selection.
fn line_at(clicked: InlineText, candidates: &[InlineText], page: PageSize) -> InlineText {
    if !matches!(clicked.target, InlineTarget::Run(_)) {
        return clicked;
    }
    let mut selected = vec![clicked.clone()];
    loop {
        let next = candidates
            .iter()
            .find(|candidate| {
                matches!(candidate.target, InlineTarget::Run(_))
                    && !selected.iter().any(|a| a.target == candidate.target)
                    && clicked.geometry.is_some()
                    && candidate.geometry.is_some()
                    && same_text_baseline(&clicked, candidate)
                    && selected.iter().any(|a| {
                        let gap =
                            (candidate.rect[0] - a.rect[2]).max(a.rect[0] - candidate.rect[2]);
                        gap < a.size.max(candidate.size) * 1.5
                    })
            })
            .cloned();
        let Some(next) = next else { break };
        selected.push(next);
        if selected.len() >= 200 {
            break;
        }
    }
    let area = clicked.rect;
    selected_block(selected, area, page).unwrap_or(clicked)
}

fn same_text_baseline(a: &InlineText, b: &InlineText) -> bool {
    if let (Some(a_layout), Some(b_layout)) = (&a.geometry, &b.geometry) {
        let direction = a_layout.direction;
        let parallel = direction[0] * b_layout.direction[0] + direction[1] * b_layout.direction[1];
        let dx = b_layout.origin[0] - a_layout.origin[0];
        let dy = b_layout.origin[1] - a_layout.origin[1];
        parallel > 0.99 && (dx * direction[1] - dy * direction[0]).abs() < a.size.min(b.size) * 0.25
    } else {
        // Missing font metrics: require strong vertical overlap, never infer a
        // paragraph from similar font sizes or left edges.
        let overlap = a.rect[3].min(b.rect[3]) - a.rect[1].max(b.rect[1]);
        overlap > (a.rect[3] - a.rect[1]).min(b.rect[3] - b.rect[1]) * 0.6
    }
}

fn selected_block(
    mut items: Vec<InlineText>,
    area: [f32; 4],
    page_size: PageSize,
) -> Option<InlineText> {
    if items.is_empty()
        || items.iter().any(|item| {
            item.geometry
                .as_ref()
                .is_some_and(|g| g.direction[0] < 0.999 || g.direction[1].abs() > 0.01)
        })
    {
        // The positioned editor works in horizontal page coordinates. Keep
        // affine text in its original PDF font/matrix through the run editor.
        return None;
    }
    items.sort_by(|a, b| {
        a.geometry
            .as_ref()
            .map_or(a.rect[1] + a.size * 0.8, |g| g.origin[1])
            .total_cmp(
                &b.geometry
                    .as_ref()
                    .map_or(b.rect[1] + b.size * 0.8, |g| g.origin[1]),
            )
            .then(a.rect[0].total_cmp(&b.rect[0]))
    });
    // Reassemble fragments from a visual line, then order lines top to bottom.
    let mut lines: Vec<Vec<InlineText>> = Vec::new();
    for item in items {
        if let Some(line) = lines.last_mut()
            && line.first().is_some_and(|a| same_text_baseline(a, &item))
        {
            line.push(item);
        } else {
            lines.push(vec![item]);
        }
    }
    for line in &mut lines {
        line.sort_by(|a, b| a.rect[0].total_cmp(&b.rect[0]));
    }
    let first = lines.first()?.first()?.clone();
    let mut selection = text_block::Selection {
        runs: vec![],
        annotations: vec![],
    };
    let mut masks = Vec::new();
    let mut paragraphs = Vec::new();
    let mut spans = Vec::new();
    let mut byte_offset = 0;
    let mut layout = Vec::new();
    for line in lines {
        let first = &line[0];
        let origin = first
            .geometry
            .as_ref()
            .map(|g| g.origin)
            .unwrap_or([first.rect[0], first.rect[1] + first.size * 0.8]);
        layout.push(mypdf_milestone0::rich_text::LineLayout {
            start: byte_offset,
            left: origin[0],
            baseline: origin[1],
        });
        let mut text = String::new();
        let mut previous: Option<&InlineText> = None;
        for item in &line {
            if let Some(prev) = previous
                && !text.ends_with(char::is_whitespace)
                && !item.text.starts_with(char::is_whitespace)
                && prev
                    .geometry
                    .as_ref()
                    .zip(item.geometry.as_ref())
                    .is_some_and(|(previous, next)| previous.needs_space_before(next))
            {
                text.push(' ');
            }
            let start = byte_offset + text.len();
            let font = text_style::fonts()
                .iter()
                .find(|f| {
                    pdf_preview_font(&item.font) == egui::FontFamily::Name(f.ui_name().into())
                })
                .map(|f| f.path.clone());
            // Spaces between fragments inherit the preceding face.
            if let Some(previous) = spans.last_mut() {
                let previous: &mut mypdf_milestone0::rich_text::Span = previous;
                previous.range.end = start;
            }
            let display_text = if item.text == item.original {
                item.geometry
                    .as_ref()
                    .map_or(item.text.as_str(), |g| g.text.as_str())
            } else {
                item.text.as_str()
            };
            text.push_str(display_text);
            spans.push(mypdf_milestone0::rich_text::Span {
                range: start..byte_offset + text.len(),
                font,
                size: item.size.clamp(4., 144.),
                color: item.color,
            });
            match &item.target {
                InlineTarget::Run(i) => selection.runs.push((*i, item.original.clone())),
                InlineTarget::Annotation(id) => selection
                    .annotations
                    .push((id.clone(), item.original.clone())),
                InlineTarget::Block(_) => return None,
            }
            masks.push(item.rect);
            previous = Some(item);
        }
        byte_offset += text.len() + 1;
        paragraphs.push(text);
    }
    let text = paragraphs.join("\n");
    let font = text_style::fonts()
        .iter()
        .position(|f| pdf_preview_font(&first.font) == egui::FontFamily::Name(f.ui_name().into()))
        .unwrap_or(0);
    let size = first.size.clamp(4., 144.);
    let x = masks
        .iter()
        .map(|r| r[0])
        .chain(layout.iter().map(|line| line.left))
        .fold(f32::INFINITY, f32::min)
        .max(0.);
    let y = (masks.iter().map(|r| r[1]).fold(f32::INFINITY, f32::min) - size * 0.2).max(0.);
    let right = (masks.iter().map(|r| r[2]).fold(area[2], f32::max) + 8.)
        .max(x + 80.)
        .min(page_size.width);
    let bottom = masks
        .iter()
        .map(|r| r[3])
        .fold(area[3], f32::max)
        .max(
            layout
                .iter()
                .map(|line| line.baseline + size * 0.3)
                .fold(y, f32::max),
        )
        .min(page_size.height);
    if layout.len() != text.split('\n').count() {
        // A legacy multiline annotation already owns its wrapping. Retain its
        // hard breaks through the normal editor rather than inventing origins.
        layout.clear();
    }
    let preserve_layout = !layout.is_empty();
    for line in &mut layout {
        line.left -= x;
        line.baseline -= y;
    }
    Some(InlineText {
        key: first.key,
        geometry: None,
        target: InlineTarget::Block(Box::new(BlockDraft {
            selection,
            masks,
            initial_spans: spans.clone(),
            spans,
            styled_text: text.clone(),
            selected_chars: None,
            font,
            alignment: Alignment::Left,
            initial: (font, size, first.color, right),
            initial_origin: [x, y],
            initial_layout: layout.clone(),
            layout,
            preserve_layout,
        })),
        original: text.clone(),
        text,
        rect: [x, y, right, bottom],
        size,
        color: first.color,
        font: first.font,
        focus: true,
    })
}

/// Keep indentation and paragraph-gap proportions when changing baseline spacing.
fn scale_line_spacing(layout: &mut [mypdf_milestone0::rich_text::LineLayout], spacing: f32) {
    if layout.len() < 2 || !spacing.is_finite() || spacing <= 0. { return; }
    let first = layout[0].baseline;
    let old = (layout.last().unwrap().baseline - first) / (layout.len() - 1) as f32;
    if old <= 0. { return; }
    for line in layout {
        line.baseline = first + (line.baseline-first) * spacing / old;
    }
}

/// Start or finish rendering the page without a block's words; the texture once ready.
fn erased_page(
    erased: &mut Option<Erased>,
    key: Key,
    block: &BlockDraft,
    session: &mypdf_milestone0::annotations::Session,
    ui: &egui::Ui,
    scale: f32,
) -> Option<egui::TextureId> {
    if erased
        .as_ref()
        .is_none_or(|e| e.key != key || e.masks != block.masks)
    {
        let (tx, rx) = mpsc::channel();
        let source = session.source.clone();
        let selection = block.selection.clone();
        let scale = scale * ui.ctx().pixels_per_point();
        let wake = ui.ctx().clone();
        std::thread::spawn(move || {
            let image = std::panic::catch_unwind(|| {
                let replacement = text_block::Replacement {
                    layout: vec![],
                    spans: vec![],
                    selection,
                    text: String::new(),
                    rect: [0., 0., 1., 1.],
                    size: 12.,
                    color: [0.; 3],
                    format: TextFormat::default(),
                };
                let bytes = text_block::replace(&source, key.2 as u32 + 1, &replacement).ok()?;
                let (size, rgba) = viewer::render_page(&bytes, key.2, scale).ok()?;
                Some(egui::ColorImage::from_rgba_premultiplied(size, &rgba))
            })
            .ok()
            .flatten();
            let _ = tx.send(image);
            wake.request_repaint();
        });
        *erased = Some(Erased {
            key,
            masks: block.masks.clone(),
            rx: Some(rx),
            texture: None,
        });
    }
    let e = erased.as_mut()?;
    if let Some(rx) = &e.rx
        && let Ok(image) = rx.try_recv()
    {
        e.rx = None;
        e.texture = image.map(|image| {
            ui.ctx()
                .load_texture("erased_block_page", image, egui::TextureOptions::LINEAR)
        });
    }
    e.texture.as_ref().map(TextureHandle::id)
}

#[derive(Clone, Copy)]
struct TextMoveGesture {
    rect: [f32; 4],
    pointer: egui::Pos2,
}

/// egui clears press_origin/total_drag_delta on release. Keep our own origin,
/// including a fast press/move/release delivered together between paint frames.
fn text_move_start(ui: &egui::Ui, response: &egui::Response) -> Option<egui::Pos2> {
    let (press, released, origin) = ui.input(|input| {
        let press = input.events.iter().find_map(|event| match event {
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                ..
            } => Some(*pos),
            _ => None,
        });
        (
            press,
            input.pointer.primary_released(),
            input.pointer.press_origin(),
        )
    });
    if response.drag_started() {
        return origin.or(press);
    }
    // A whole drag within one frame never enters egui's persistent drag state.
    // Only accept the handle's visible area in the top interactive layer.
    press.filter(|press| {
        released
            && ui.is_enabled()
            && response.rect.contains(*press)
            && ui.clip_rect().contains(*press)
            && ui.ctx().layer_id_at(*press) == Some(response.layer_id)
    })
}

/// Keep a text block's drag handle on the page, outside its editable text.
fn text_move_handle(rect: Rect, page: Rect) -> Rect {
    let y = if rect.top() - 16. >= page.top() {
        rect.top() - 9.
    } else if rect.bottom() + 16. <= page.bottom() {
        rect.bottom() + 9.
    } else {
        (rect.top() + 9.).min(page.bottom() - 6.)
    };
    let width = 40_f32.min(page.width().max(0.));
    let height = 12_f32.min(page.height().max(0.));
    Rect::from_center_size(
        pos2(
            rect.center()
                .x
                .clamp(page.left() + width / 2., page.right() - width / 2.),
            y.clamp(page.top() + height / 2., page.bottom() - height / 2.),
        ),
        vec2(width, height),
    )
}

fn paint_text_move_handle(painter: &egui::Painter, rect: Rect) {
    painter.rect_filled(rect, 6., theme::paper());
    painter.rect_stroke(
        rect,
        6.,
        Stroke::new(1., theme::accent()),
        egui::StrokeKind::Inside,
    );
    painter.line_segment(
        [rect.center() - vec2(10., 0.), rect.center() + vec2(10., 0.)],
        Stroke::new(2., theme::accent()),
    );
}

/// Move in PDF coordinates from the gesture's start, retaining the block size.
fn moved_text_rect(original: [f32; 4], delta: egui::Vec2, page: PageSize) -> [f32; 4] {
    let width = original[2] - original[0];
    let height = original[3] - original[1];
    let x = (original[0] + delta.x).clamp(0., (page.width - width).max(0.));
    let y = (original[1] + delta.y).clamp(0., (page.height - height).max(0.));
    [x, y, x + width, y + height]
}

fn existing_overlay(
    state: &mut ContentState,
    tab: &Tab,
    ui: &mut egui::Ui,
    page: usize,
    page_rect: Rect,
    scale: f32,
) {
    let Some(session) = tab.session.as_ref().filter(|s| s.editable) else {
        return;
    };
    let key = (tab.id, session.revision(), page);
    let screen = |r: [f32; 4]| {
        Rect::from_min_max(
            page_rect.min + vec2(r[0], r[1]) * scale,
            page_rect.min + vec2(r[2], r[3]) * scale,
        )
    };
    let mut candidates = Vec::new();
    if let Some(info) = &state.inspection
        && info.key == key
        && let Ok(runs) = &info.text
    {
        for (ordinal, run) in runs.iter().enumerate() {
            if let Some(rect) = run.rect {
                candidates.push(InlineText {
                    key,
                    target: InlineTarget::Run(ordinal),
                    geometry: run.layout.clone(),
                    original: run.text.clone(),
                    text: run.text.clone(),
                    rect,
                    size: run.size,
                    color: run.color,
                    font: run.base_font.clone(),
                    focus: true,
                });
            }
        }
    }
    for a in session
        .annotations
        .iter()
        .filter(|a| a.page == page && !a.object.hidden && !a.object.locked)
    {
        if let mypdf_milestone0::annotations::Kind::Text { text } = &a.kind {
            candidates.push(InlineText {
                key,
                target: InlineTarget::Annotation(a.id.clone()),
                geometry: None,
                original: text.clone(),
                text: text.clone(),
                rect: a.bounds(),
                size: a.style.font_size,
                color: a.style.color,
                font: "Text box".into(),
                focus: true,
            });
        }
    }
    // Dragging across the page selects an area, with or without Select area.
    let background = ui.interact(
        page_rect,
        egui::Id::new((tab.id, page, "text_page_background")),
        egui::Sense::click_and_drag(),
    );
    if background.drag_started()
        && !state.busy
        && !state.loading
        && state.marquee.is_none()
        && let Some(origin) = ui.input(|i| i.pointer.press_origin())
    {
        if state
            .existing
            .as_ref()
            .is_some_and(|e| e.key.0 == tab.id && e.changed())
        {
            state.new_commit = true;
        } else {
            state.existing = None;
            state.next_existing = None;
            state.marquee = Some((key, origin));
            state.selection_hint = None;
        }
    }
    let selecting = state.marquee.is_some_and(|(k, _)| k == key)
        || (state.area_mode && state.existing.is_none());
    let painter = ui
        .painter()
        .with_clip_rect(page_rect.intersect(ui.clip_rect()));
    if selecting {
        if background.hovered() && state.area_mode {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            state.marquee = None;
            return;
        }
        if let Some((start_key, start)) = state.marquee
            && start_key == key
            && let Some(end) = ui.input(|i| i.pointer.interact_pos())
        {
            let r = Rect::from_two_pos(start, end).intersect(page_rect);
            painter.rect_filled(r, 2., theme::accent().gamma_multiply(0.08));
            painter.rect_stroke(
                r,
                2.,
                Stroke::new(1., theme::accent()),
                egui::StrokeKind::Inside,
            );
            let selected: Vec<_> = candidates
                .into_iter()
                .filter(|d| r.contains(screen(d.rect).center()))
                .collect();
            for item in &selected {
                painter.rect_filled(
                    screen(item.rect).expand(2.),
                    1.,
                    theme::accent().gamma_multiply(0.17),
                );
            }
            if background.drag_stopped() {
                let a = (r.min - page_rect.min) / scale;
                let b = (r.max - page_rect.min) / scale;
                if selected.len() > 2000
                    || selected.iter().map(|d| d.text.len() + 1).sum::<usize>() > 32_768
                {
                    state.selection_hint = Some(
                        "Select a smaller block (up to 2,000 objects or 32 KB of text).".into(),
                    );
                } else {
                    state.existing =
                        selected_block(selected, [a.x, a.y, b.x, b.y], tab.pages[page]);
                    if state.existing.is_none() {
                        state.selection_hint = Some("No supported text block in that area. Select horizontal text; rotated text can be edited one run at a time. Scanned text needs OCR.".into());
                    }
                }
                state.marquee = None;
                ui.ctx().request_repaint();
            }
        }
        // Render the new text area on the next frame, after releasing the drag.
        return;
    }
    let mut clicked = None;
    for (i, draft) in candidates.iter().cloned().enumerate() {
        if let Some(active) = state.existing.as_ref().filter(|e| e.key == key)
            && let InlineTarget::Block(block) = &active.target
            && (screen(active.rect).intersects(screen(draft.rect))
                || match &draft.target {
                    InlineTarget::Run(i) => {
                        block.selection.runs.iter().any(|(ordinal, _)| ordinal == i)
                    }
                    InlineTarget::Annotation(id) => {
                        block.selection.annotations.iter().any(|(key, _)| key == id)
                    }
                    InlineTarget::Block(_) => true,
                })
        {
            continue;
        }
        let r = screen(draft.rect).expand(3.).intersect(page_rect);
        if !r.is_positive() {
            continue;
        }
        if state
            .existing
            .as_ref()
            .is_some_and(|e| e.key.0 == tab.id && e.key.2 == page && e.target == draft.target)
        {
            continue;
        }
        let response = ui
            .interact(
                r,
                egui::Id::new((tab.id, page, "existing_text", i)),
                egui::Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::Text);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                !state.busy,
                format!(
                    "Edit text: {}",
                    draft.original.chars().take(64).collect::<String>()
                ),
            )
        });
        if response.hovered() && !state.hide_guides {
            painter.rect_stroke(
                r,
                2.,
                Stroke::new(1., theme::accent()),
                egui::StrokeKind::Outside,
            );
        }
        if response.clicked() && !state.busy {
            clicked = Some(line_at(draft, &candidates, tab.pages[page]));
        }
    }
    if let Some(draft) = clicked {
        if state.existing.is_some() {
            state.new_commit = true;
            state.next_existing = Some(draft);
        } else {
            state.existing = Some(draft);
            state.show_text_inspector = true;
        }
    } else if background.clicked() {
        state.click_outside(tab.id);
    }
    let Some(draft) = state
        .existing
        .as_mut()
        .filter(|e| e.key.0 == tab.id && e.key.2 == page)
    else {
        return;
    };
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) && !state.busy {
        state.existing = None;
        state.next_existing = None;
        state.after_text = None;
        return;
    }
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)) && !state.busy {
        state.new_commit = true;
    }
    // Process placement before painting the text, selection outline and handle.
    // A single-run editor retains its existing-font replacement until it is
    // actually moved; moving requires the same safe block rewrite as area edits.
    let movable = !matches!(draft.target, InlineTarget::Annotation(_))
        && !draft
            .geometry
            .as_ref()
            .is_some_and(|g| g.direction[0] < 0.999 || g.direction[1].abs() > 0.01);
    if movable && !state.busy {
        let move_id = egui::Id::new((tab.id, page, "move_existing_block"));
        let moving = ui
            .interact(
                text_move_handle(screen(draft.rect), page_rect),
                move_id,
                egui::Sense::drag(),
            )
            .on_hover_cursor(egui::CursorIcon::Move)
            .on_hover_text("Drag to move text block");
        moving.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Move text block")
        });
        let started = text_move_start(ui, &moving);
        if let Some(pointer) = started {
            if matches!(draft.target, InlineTarget::Run(_)) {
                let original = draft.original.clone();
                // A run editor can grow beyond its original glyph bounds while
                // typing. Retain that visible width when it becomes movable.
                let font = FontId::new(
                    (draft.size * scale).clamp(4., 800.),
                    pdf_preview_font(&draft.font),
                );
                let width = ui.fonts_mut(|fonts| {
                    fonts
                        .layout_no_wrap(draft.text.clone(), font, theme::ink())
                        .size()
                        .x
                }) / scale;
                let mut area = draft.rect;
                area[2] = area[2].max(area[0] + width).min(tab.pages[page].width);
                if let Some(mut block) = selected_block(vec![draft.clone()], area, tab.pages[page])
                {
                    block.original = original;
                    block.focus = false;
                    *draft = block;
                }
            }
            ui.data_mut(|data| {
                data.insert_temp(
                    move_id,
                    TextMoveGesture {
                        rect: draft.rect,
                        pointer,
                    },
                )
            });
        }
        if (moving.dragged() || moving.drag_stopped() || started.is_some())
            && let Some(gesture) = ui.data(|data| data.get_temp::<TextMoveGesture>(move_id))
            && let Some(pointer) = ui.input(|input| input.pointer.interact_pos())
        {
            draft.rect = moved_text_rect(
                gesture.rect,
                (pointer - gesture.pointer) / scale,
                tab.pages[page],
            );
        }
        if !ui.input(|i| i.pointer.primary_down()) {
            ui.data_mut(|data| data.remove::<TextMoveGesture>(move_id));
        }
    }
    if matches!(draft.target, InlineTarget::Block(_)) && !state.busy {
        let response = ui
            .interact(
                Rect::from_center_size(screen(draft.rect).max, vec2(16., 16.)),
                egui::Id::new((tab.id, page, "resize_existing_block")),
                egui::Sense::drag(),
            )
            .on_hover_cursor(egui::CursorIcon::ResizeNwSe)
            .on_hover_text("Resize text block");
        if response.dragged()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let p = (pos - page_rect.min) / scale;
            draft.rect[2] = p.x.clamp(
                (draft.rect[0] + 10.).min(tab.pages[page].width),
                tab.pages[page].width,
            );
            draft.rect[3] = p.y.clamp(
                (draft.rect[1] + draft.size * 1.3).min(tab.pages[page].height),
                tab.pages[page].height,
            );
        }
    }
    let original = screen(draft.rect);
    let annotation = matches!(draft.target, InlineTarget::Annotation(_));
    let block = matches!(draft.target, InlineTarget::Block(_));
    let family = if let InlineTarget::Block(block) = &draft.target {
        text_style::fonts()
            .get(block.font)
            .map(|f| egui::FontFamily::Name(f.ui_name().into()))
            .unwrap_or(egui::FontFamily::Proportional)
    } else if annotation {
        egui::FontFamily::Name("annotation".into())
    } else {
        pdf_preview_font(&draft.font)
    };
    let color = Color32::from_rgb(
        (draft.color[0] * 255.) as u8,
        (draft.color[1] * 255.) as u8,
        (draft.color[2] * 255.) as u8,
    );
    let origin = original.min
        + if annotation {
            vec2(4., 4.) * scale
        } else if block {
            vec2(0., 0.)
        } else {
            vec2(0., -draft.size * 0.2) * scale
        };
    // A single run grows with its text, like the PDF it will become.
    let text_width = if annotation || block {
        original.width()
    } else {
        let font = FontId::new((draft.size * scale).clamp(4., 800.), family.clone());
        ui.fonts_mut(|f| f.layout_no_wrap(draft.text.clone(), font, color).size().x) + 2.
    };
    let width = original
        .width()
        .max(text_width)
        .max(10.)
        .min((page_rect.right() - origin.x).max(10.));
    let rect = Rect::from_min_size(
        origin,
        vec2(width, original.height().max(draft.size * 1.25 * scale)),
    );
    // Hide only the words being replaced; the page around them stays visible.
    if let InlineTarget::Block(block) = &draft.target {
        let erased = erased_page(
            &mut state.erased,
            draft.key,
            block,
            session,
            ui,
            page_rect.width() / tab.pages[page].width,
        );
        for mask in &block.masks {
            let r = screen(*mask).expand(2.);
            match erased {
                Some(texture) => {
                    let uv = Rect::from_min_max(
                        pos2(
                            (r.min.x - page_rect.min.x) / page_rect.width(),
                            (r.min.y - page_rect.min.y) / page_rect.height(),
                        ),
                        pos2(
                            (r.max.x - page_rect.min.x) / page_rect.width(),
                            (r.max.y - page_rect.min.y) / page_rect.height(),
                        ),
                    );
                    painter.image(texture, r, uv, Color32::WHITE);
                }
                // Until the clean page is ready, and if it cannot be made.
                None => {
                    painter.rect_filled(r, 0., Color32::WHITE);
                }
            }
        }
    }
    if !block {
        painter.rect_filled(original.expand(1.5), 0., Color32::WHITE);
    }
    let outline = rect.expand2(vec2(5., 3.));
    if !state.hide_guides {
        ui::dashed_rect(&painter, outline, theme::accent());
    }
    if !block && !state.hide_guides {
        for center in [outline.left_center(), outline.right_center()] {
            painter.circle_filled(center, 4.5, Color32::WHITE);
            painter.circle_stroke(center, 4.5, Stroke::new(1.5, theme::accent()));
        }
    }
    let rich = if let InlineTarget::Block(b) = &draft.target {
        Some(b.spans.clone())
    } else {
        None
    };
    let source_text = if let InlineTarget::Block(b) = &draft.target {
        b.styled_text.clone()
    } else {
        draft.original.clone()
    };
    let alignment = if let InlineTarget::Block(b) = &draft.target {
        b.alignment
    } else {
        Alignment::Left
    };
    let line_layout = if let InlineTarget::Block(b) = &draft.target {
        if b.preserve_layout && b.alignment == Alignment::Left {
            b.layout.clone()
        } else {
            vec![]
        }
    } else {
        vec![]
    };
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.set_clip_rect(page_rect.intersect(ui.clip_rect()));
        ui.add_enabled_ui(!state.busy, |ui| {
            let font = FontId::new(draft.size * scale, family);
            let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, width: f32| {
                formatting::layout_positioned(
                    ui,
                    text.as_str(),
                    &source_text,
                    rich.as_deref().unwrap_or(&[]),
                    font.clone(),
                    color,
                    width,
                    scale,
                    alignment,
                    &line_layout,
                )
            };
            let edit = if annotation || block {
                egui::TextEdit::multiline(&mut draft.text).margin(ui::FIELD_MARGIN)
            } else {
                egui::TextEdit::singleline(&mut draft.text).margin(ui::FIELD_MARGIN)
            };
            let text_id = egui::Id::new((tab.id, page, "inline_existing_text"));
            let retained_selection = context::secondary_selection(ui.ctx(), text_id);
            let mut output = edit
                .id(text_id)
                .font(font.clone())
                .text_color(color)
                .frame(false)
                .margin(0.)
                .desired_width(rect.width())
                .min_size(rect.size())
                .char_limit(32768)
                .layouter(&mut layouter)
                .show(ui);
            context::restore_secondary_selection(&mut output, retained_selection);
            context::text_menu(
                &mut output,
                &mut draft.text,
                &mut state.new_commit,
                &mut state.show_text_inspector,
                &mut state.context.cancel_text,
            );
            if let InlineTarget::Block(b) = &mut draft.target {
                formatting::capture(&draft.text, &mut b.selected_chars, output.cursor_range);
            }
            let response = output.response;
            if response.changed()
                && let InlineTarget::Block(b) = &mut draft.target
            {
                b.layout = mypdf_milestone0::rich_text::edited_layout(
                    &b.styled_text,
                    &draft.text,
                    &b.layout,
                    draft.size * 1.3,
                );
                b.spans =
                    mypdf_milestone0::rich_text::edited(&b.styled_text, &draft.text, &b.spans);
                b.styled_text = draft.text.clone();
            }
            if block {
                draft.rect[3] = draft.rect[3]
                    .max(draft.rect[1] + output.galley.size().y / scale + draft.size * 0.3)
                    .min(tab.pages[page].height);
            }
            if draft.focus {
                response.request_focus();
                draft.focus = false;
            }
            if !annotation
                && !block
                && response.lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
            {
                state.new_commit = true;
            }
        });
    });
    if movable && !state.busy {
        paint_text_move_handle(&painter, text_move_handle(screen(draft.rect), page_rect));
    }
    if block && !state.busy {
        let corner = screen(draft.rect).max;
        painter.circle_filled(corner, 5., Color32::WHITE);
        painter.circle_stroke(corner, 5., Stroke::new(1.5, theme::accent()));
    }
}

pub(super) fn overlay(
    state: &mut ContentState,
    mode: editing::Mode,
    tab: &Tab,
    ui: &mut egui::Ui,
    page: usize,
    page_rect: Rect,
    scale: f32,
) {
    if mode != editing::Mode::Edit || state.paused {
        return;
    }
    if state.image_mode && state.region.active {
        region::overlay(&mut state.region, tab, ui, page, page_rect, scale);
        return;
    }
    if state.add_target.is_some() && !state.busy && tab.session.as_ref().is_some_and(|s| s.editable)
    {
        let response = ui
            .interact(
                page_rect,
                egui::Id::new((tab.id, page, "add_text_position")),
                egui::Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::Text);
        if response.clicked()
            && let Some(point) = response.interact_pointer_pos()
        {
            let p = (point - page_rect.min) / scale;
            if state.new_placed && !state.new_text.trim().is_empty() {
                state.new_commit = true;
                state.next_text = Some((tab.id, page, [p.x, p.y]));
            } else {
                state.place_text(tab.id, page, [p.x, p.y], tab.pages[page]);
            }
        }
        if state.new_placed && state.add_target == Some((tab.id, page)) {
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)) {
                state.new_commit = true;
            }
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                state.clear_new_text();
                state.new_placed = false;
                state.next_text = None;
                return;
            }
            let [x, y, w, h] = state.new_rect;
            let r = Rect::from_min_size(page_rect.min + vec2(x, y) * scale, vec2(w, h) * scale);
            let painter = ui
                .painter()
                .with_clip_rect(page_rect.intersect(ui.clip_rect()));
            let move_rect = text_move_handle(r, page_rect);
            let move_id = egui::Id::new((tab.id, page, "move_text_block"));
            let moving = ui
                .interact(move_rect, move_id, egui::Sense::drag())
                .on_hover_text("Move text box")
                .on_hover_cursor(egui::CursorIcon::Move);
            let p = &tab.pages[page];
            let started = text_move_start(ui, &moving);
            if let Some(pointer) = started {
                state.new_initial_rect.get_or_insert(state.new_rect);
                ui.data_mut(|data| {
                    data.insert_temp(
                        move_id,
                        TextMoveGesture {
                            rect: state.new_rect,
                            pointer,
                        },
                    )
                });
            }
            if (moving.dragged() || moving.drag_stopped() || started.is_some())
                && let Some(gesture) = ui.data(|data| data.get_temp::<TextMoveGesture>(move_id))
                && let Some(pointer) = ui.input(|input| input.pointer.interact_pos())
            {
                let d = (pointer - gesture.pointer) / scale;
                state.new_rect[0] = (gesture.rect[0] + d.x).clamp(0., (p.width - w).max(0.));
                state.new_rect[1] = (gesture.rect[1] + d.y).clamp(0., (p.height - h).max(0.));
            }
            if !ui.input(|input| input.pointer.primary_down()) {
                ui.data_mut(|data| data.remove::<TextMoveGesture>(move_id));
            }
            let [x, y, w, h] = state.new_rect;
            let r = Rect::from_min_size(page_rect.min + vec2(x, y) * scale, vec2(w, h) * scale);
            if !state.hide_guides {
                ui::dashed_rect(&painter, r.expand(2.), theme::accent());
            }
            paint_text_move_handle(&painter, text_move_handle(r, page_rect));
            let rgb = state.new_color;
            let color = Color32::from_rgb(
                (rgb[0] * 255.) as u8,
                (rgb[1] * 255.) as u8,
                (rgb[2] * 255.) as u8,
            );
            let font = state.text_font(state.new_size * scale);
            state.sync_new_styles();
            let spans = state.new_spans.clone();
            let source_text = state.new_styled_text.clone();
            let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, width: f32| {
                formatting::layout(
                    ui,
                    text.as_str(),
                    &source_text,
                    &spans,
                    font.clone(),
                    color,
                    width,
                    scale,
                    state.new_alignment,
                )
            };
            ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                ui.set_clip_rect(r.intersect(page_rect).intersect(ui.clip_rect()));
                let text_id = egui::Id::new((tab.id, page, "inline_new_text"));
                let retained_selection = context::secondary_selection(ui.ctx(), text_id);
                let mut edit = egui::TextEdit::multiline(&mut state.new_text)
                    .margin(ui::FIELD_MARGIN)
                    .id(text_id)
                    .text_color(color)
                    .frame(false)
                    .margin(0.)
                    .desired_rows(1)
                    .desired_width(r.width())
                    .min_size(r.size())
                    .char_limit(32768)
                    .hint_text("Type here")
                    .layouter(&mut layouter)
                    .show(ui);
                context::restore_secondary_selection(&mut edit, retained_selection);
                context::text_menu(
                    &mut edit,
                    &mut state.new_text,
                    &mut state.new_commit,
                    &mut state.show_text_inspector,
                    &mut state.context.cancel_text,
                );
                formatting::capture(&state.new_text, &mut state.new_selection, edit.cursor_range);
                if edit.response.changed() {
                    let fallback = formatting::span(
                        state.new_font,
                        state.new_size,
                        state.new_color,
                        state.new_text.len(),
                    );
                    formatting::sync(
                        &state.new_text,
                        &mut state.new_styled_text,
                        &mut state.new_spans,
                        fallback,
                    );
                }
                if state.focus_new_text {
                    edit.response.request_focus();
                    state.focus_new_text = false;
                }
                let required = edit.galley.size().y / scale;
                if required > state.new_rect[3] {
                    state.new_rect[3] =
                        (required + state.new_size * 0.3).min((p.height - y).max(h));
                }
            });
            for (edge, center, cursor) in [
                (0, r.left_center(), egui::CursorIcon::ResizeHorizontal),
                (1, r.right_center(), egui::CursorIcon::ResizeHorizontal),
                (2, r.max, egui::CursorIcon::ResizeNwSe),
            ] {
                let handle = Rect::from_center_size(center, vec2(12., 12.));
                painter.circle_filled(center, 5., Color32::WHITE);
                painter.circle_stroke(center, 5., Stroke::new(1.5, theme::accent()));
                let resize = ui
                    .interact(
                        handle,
                        egui::Id::new((tab.id, page, "resize_text_block", edge)),
                        egui::Sense::drag(),
                    )
                    .on_hover_cursor(cursor);
                if resize.drag_started() {
                    state.new_initial_rect.get_or_insert(state.new_rect);
                    state.new_drag = Some(state.new_rect);
                }
                if resize.dragged()
                    && let Some(original) = state.new_drag
                {
                    let d = resize.total_drag_delta().unwrap_or_default() / scale;
                    if edge == 0 {
                        let right = original[0] + original[2];
                        state.new_rect[0] = (original[0] + d.x).clamp(0., (right - 10.).max(0.));
                        state.new_rect[2] = right - state.new_rect[0];
                    } else {
                        state.new_rect[2] = (original[2] + d.x).clamp(10., (p.width - x).max(10.));
                    }
                    if edge == 2 {
                        state.new_rect[3] = (original[3] + d.y)
                            .clamp(state.new_size, (p.height - y).max(state.new_size));
                    }
                }
            }
            if !ui.input(|i| i.pointer.primary_down()) {
                state.new_drag = None;
            }
        }
        return;
    }
    if !state.image_mode {
        existing_overlay(state, tab, ui, page, page_rect, scale);
        return;
    }
    let Some(info) = &state.inspection else {
        return;
    };
    if info.key.0 != tab.id || info.key.2 != page {
        return;
    }
    let painter = ui
        .painter()
        .with_clip_rect(page_rect.intersect(ui.clip_rect()));
    let boxes: Vec<(usize, [f32; 4], bool)> = info
        .images
        .as_ref()
        .map(|images| {
            images
                .iter()
                .enumerate()
                .map(|(i, m)| (i, m.rect, state.selected == Some(i)))
                .collect()
        })
        .unwrap_or_default();
    let background = ui.interact(
        page_rect,
        egui::Id::new((tab.id, page, "image_background")),
        egui::Sense::click(),
    );
    if background.clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.selected = None;
    }
    for (i, r, selected) in boxes {
        let r = Rect::from_min_max(
            page_rect.min + vec2(r[0], r[1]) * scale,
            page_rect.min + vec2(r[2], r[3]) * scale,
        );
        let response = ui.interact(
            r.intersect(page_rect),
            egui::Id::new((tab.id, page, "content", i)),
            egui::Sense::click(),
        );
        if !state.hide_guides && (selected || response.hovered()) {
            painter.rect_stroke(
                r,
                2.,
                Stroke::new(if selected { 1.5 } else { 1. }, theme::accent()),
                egui::StrokeKind::Outside,
            );
        }
        if response.clicked()
            && let Ok(images) = &info.images
        {
            state.selected = Some(i);
            let r = images[i].rect;
            state.rect = [r[0], r[1], r[2] - r[0], r[3] - r[1]];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spacing_adjustment_preserves_indents_and_is_written_to_pdf() {
        use mypdf_milestone0::{rich_text::{self, LineLayout}, text_geometry};
        let text = "First line\nSecond line\nThird line";
        let mut layout = vec![
            LineLayout { start: 0, left: 0., baseline: 20. },
            LineLayout { start: 11, left: 12., baseline: 40. },
            LineLayout { start: 23, left: 0., baseline: 80. },
        ];
        scale_line_spacing(&mut layout, 45.);
        assert_eq!(layout.iter().map(|l| l.baseline).collect::<Vec<_>>(), vec![20., 50., 110.]);
        assert_eq!(layout[1].left, 12.);
        let spans = vec![formatting::span(0, 12., [0.; 3], text.len())];
        let pdf = rich_text::add_positioned(&convert::blank(595.,842.).unwrap(), 1, text, [40.,60.,400.,150.], &spans, &layout).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spacing.pdf");
        std::fs::write(&path, pdf).unwrap();
        let reopened = text_geometry::runs(&std::fs::read(path).unwrap(), 1).unwrap();
        for (run, expected) in reopened.iter().zip([80.,110.,170.]) {
            assert!((run.layout.as_ref().unwrap().origin[1] - expected).abs() < 0.1);
        }
        assert_eq!(reopened.len(), 3);
    }

    #[test]
    fn translation_selection_is_scoped_to_its_inline_document_page_and_revision() {
        use egui::text::{CCursor, CCursorRange};
        let ctx = egui::Context::default();
        let mut content = ContentState::new();
        let selection = |id| {
            let mut state = egui::text_edit::TextEditState::default();
            state
                .cursor
                .set_char_range(Some(CCursorRange::two(CCursor::new(6), CCursor::new(14))));
            state.store(&ctx, id);
        };
        content.existing = Some(InlineText {
            key: (11, 3, 2),
            target: InlineTarget::Run(0),
            geometry: None,
            original: "First İstanbul last".into(),
            text: "First İstanbul last".into(),
            rect: [10., 10., 200., 40.],
            size: 18.,
            color: [0.; 3],
            font: "Helvetica".into(),
            focus: false,
        });
        selection(egui::Id::new((11_u64, 2_usize, "inline_existing_text")));
        assert_eq!(
            content
                .selected_translation_text(&ctx, (11, 3, 2))
                .as_deref(),
            Some("İstanbul")
        );
        assert_eq!(
            content.translation_anchor(&ctx, (11, 3, 2)),
            Some([10., 10., 200., 40.])
        );
        assert!(
            content
                .translation_source_object(&ctx, (11, 3, 2))
                .is_none()
        );
        for key in [(12, 3, 2), (11, 3, 1), (11, 4, 2)] {
            assert!(content.selected_translation_text(&ctx, key).is_none());
            assert!(content.translation_anchor(&ctx, key).is_none());
        }
        content.existing.as_mut().unwrap().target =
            InlineTarget::Annotation("source-object".into());
        assert_eq!(
            content
                .translation_source_object(&ctx, (11, 3, 2))
                .as_deref(),
            Some("source-object")
        );
        assert_eq!(
            content.existing.as_ref().unwrap().text,
            "First İstanbul last"
        );

        content.existing = None;
        content.new_placed = true;
        content.add_target = Some((11, 2));
        content.new_text = "First İstanbul last".into();
        content.new_rect = [30., 40., 200., 50.];
        selection(egui::Id::new((11_u64, 2_usize, "inline_new_text")));
        assert_eq!(
            content
                .selected_translation_text(&ctx, (11, 3, 2))
                .as_deref(),
            Some("İstanbul")
        );
        assert_eq!(
            content.translation_anchor(&ctx, (11, 3, 2)),
            Some([30., 40., 230., 90.])
        );
        for key in [(12, 3, 2), (11, 3, 1)] {
            assert!(content.selected_translation_text(&ctx, key).is_none());
        }
        assert_eq!(content.new_text, "First İstanbul last");
        assert!(!content.new_commit);
    }

    #[test]
    fn cancelled_ocr_discards_even_an_already_completed_worker_result() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let original = convert::blank(200., 200.).unwrap();
        app.open_memory(PathBuf::from("Cancel scan.pdf"), original.clone(), &ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let revision = app.tabs[0].session.as_ref().unwrap().revision();
        let progress = Arc::new(super::super::recognize::Progress::default());
        app.content.busy = true;
        app.content.recognition = Some(progress.clone());
        app.ocr_progress = Some(progress.clone());
        app.content
            .tx
            .send(Ok((
                app.tabs[0].id,
                revision,
                convert::blank(400., 400.).unwrap(),
                None,
            )))
            .unwrap();
        progress.cancellation.cancel();
        app.content_events(&ctx);
        let session = app.tabs[0].session.as_ref().unwrap();
        assert_eq!(session.source.as_ref(), original);
        assert_eq!(session.revision(), revision);
        assert!(!session.can_undo());
        assert!(!app.content.busy);
        assert!(app.ocr_progress.is_none());
        assert_eq!(
            Arc::strong_count(&progress),
            1,
            "the UI releases the completed job"
        );
        assert!(
            app.status
                .as_ref()
                .is_some_and(|(message, error)| !error && message.contains("cancelled"))
        );
    }

    #[test]
    fn dropping_content_cancels_its_ocr_worker() {
        let progress = Arc::new(super::super::recognize::Progress::default());
        let mut state = ContentState::new();
        state.recognition = Some(progress.clone());
        drop(state);
        assert!(progress.cancellation.is_cancelled());
        assert_eq!(Arc::strong_count(&progress), 1);
    }

    #[test]
    fn block_alignment_and_position_are_saved_without_changing_words() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let source = content::add_text(
            &convert::blank(595., 842.).unwrap(),
            1,
            "Keep these words",
            [50., 70., 400., 100.],
            18.,
            [0.; 3],
        )
        .unwrap();
        app.open_memory(PathBuf::from("Text layout.pdf"), source.clone(), &ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let run = mypdf_milestone0::text_geometry::runs(&source, 1)
            .unwrap()
            .remove(0);
        let item = InlineText {
            key: (app.tabs[0].id, 0, 0),
            target: InlineTarget::Run(0),
            geometry: run.layout.clone(),
            original: run.text.clone(),
            text: run.text,
            rect: run.rect.unwrap(),
            size: run.size,
            color: run.color,
            font: run.base_font,
            focus: false,
        };
        let mut draft =
            selected_block(vec![item], [50., 70., 450., 170.], app.tabs[0].pages[0]).unwrap();
        assert!(!draft.changed());
        draft.rect = [80., 200., 480., 300.];
        let InlineTarget::Block(block) = &mut draft.target else {
            panic!()
        };
        block.alignment = Alignment::Right;
        assert!(draft.changed());
        app.content.existing = Some(draft);
        app.finish_existing_text(&ctx);
        while app.content.busy {
            app.content_events(&ctx);
            assert!(std::time::Instant::now() < deadline, "{:?}", app.status);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let bytes = &app.tabs[0].session.as_ref().unwrap().source;
        let runs = mypdf_milestone0::text_geometry::runs(bytes, 1).unwrap();
        let run = runs.iter().find(|r| r.text == "Keep these words").unwrap();
        let rect = run.rect.unwrap();
        assert!(
            rect[0] > 250. && rect[1] > 195. && rect[2] < 482.,
            "{rect:?}"
        );
        assert!(app.content.existing.is_none());
    }

    #[test]
    fn edited_pdf_text_drag_moves_preview_and_commits_one_undoable_change() {
        // Exercise a normal single-run click editor and a multi-line area editor.
        for area in [false, true] {
            let ctx = egui::Context::default();
            let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
            let selected_text = if area {
                "Move me\nSecond line"
            } else {
                "Move me"
            };
            let source = content::add_text(
                &content::add_text(
                    &convert::blank(595., 842.).unwrap(),
                    1,
                    selected_text,
                    [50., 70., 300., 100.],
                    18.,
                    [0.; 3],
                )
                .unwrap(),
                1,
                "Unselected neighbor",
                [370., 70., 220., 100.],
                14.,
                [0.; 3],
            )
            .unwrap();
            app.open_memory(PathBuf::from("Movable text.pdf"), source.clone(), &ctx);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            while app.tabs[0].session.is_none() {
                app.events(&ctx);
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let runs = mypdf_milestone0::text_geometry::runs(&source, 1).unwrap();
            let neighbor = runs
                .iter()
                .find(|r| r.text == "Unselected neighbor")
                .unwrap()
                .rect;
            let items: Vec<_> = runs
                .into_iter()
                .enumerate()
                .filter(|(_, run)| run.text != "Unselected neighbor")
                .map(|(ordinal, run)| InlineText {
                    key: (app.tabs[0].id, 0, 0),
                    target: InlineTarget::Run(ordinal),
                    geometry: run.layout.clone(),
                    original: run.text.clone(),
                    text: run.text,
                    rect: run.rect.unwrap(),
                    size: run.size,
                    color: run.color,
                    font: run.base_font,
                    focus: true,
                })
                .collect();
            app.content.existing = if area {
                selected_block(items, [50., 70., 350., 170.], app.tabs[0].pages[0])
            } else {
                items.into_iter().next()
            };
            let id = app.tabs[0].id;
            let page_rect = Rect::from_min_size(pos2(20., 20.), vec2(595., 842.) * 1.25);
            let frame = |app: &mut Viewer, events| {
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(900., 1200.))),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            overlay(
                                &mut app.content,
                                editing::Mode::Edit,
                                &app.tabs[0],
                                ui,
                                0,
                                page_rect,
                                1.25,
                            );
                        });
                    },
                )
            };
            let pointer = |pos, pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            let key = |key, modifiers| egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            };
            let _ = frame(&mut app, vec![]);
            let _ = frame(&mut app, vec![]);
            let _ = frame(
                &mut app,
                vec![
                    key(egui::Key::A, egui::Modifiers::COMMAND),
                    egui::Event::Text("Moved words".into()),
                ],
            );
            let start = ctx
                .read_response(egui::Id::new((id, 0_usize, "move_existing_block")))
                .expect("selected PDF text has an accessible move handle")
                .rect
                .center();
            let _ = frame(
                &mut app,
                vec![egui::Event::PointerMoved(start), pointer(start, true)],
            );
            let end = start + vec2(65., 110.) * 1.25;
            let output = frame(&mut app, vec![egui::Event::PointerMoved(end)]);
            let draft = app
                .content
                .existing
                .as_ref()
                .expect("drag keeps the text editor open");
            let InlineTarget::Block(block) = &draft.target else {
                panic!("moving a run promotes it to a safe replacement block")
            };
            let moved = draft.rect;
            assert!(
                (moved[0] - (block.initial_origin[0] + 65.)).abs() < 0.01,
                "{moved:?}"
            );
            assert!(
                (moved[1] - (block.initial_origin[1] + 110.)).abs() < 0.01,
                "{moved:?}"
            );
            assert!(
                block.masks.iter().all(|r| r[1] < moved[1]),
                "original masks do not travel with the editor"
            );
            let expected = page_rect.min + vec2(moved[0], moved[1]) * 1.25;
            let text_position = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == "Moved words" => {
                        Some(text.pos)
                    }
                    _ => None,
                })
                .expect("edited words are painted");
            assert!(
                (text_position - expected).length() < 1.,
                "text is painted at the moved position during this drag frame: {text_position:?} vs {expected:?}"
            );
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::LineSegment { points, stroke }
                    if stroke.color == theme::accent() && (points[0] - (expected - vec2(5., 3.))).length() < 0.01)),
                "selection outline is painted at the same new position in this frame");
            assert_eq!(
                &*app.tabs[0].session.as_ref().unwrap().source,
                &source,
                "dragging previews the whole draft without mutating the PDF"
            );
            let _ = frame(&mut app, vec![pointer(end, false)]);
            assert!(
                ctx.wants_keyboard_input(),
                "moving must not route typing Undo to a previous document change"
            );
            assert_eq!(app.content.existing.as_ref().unwrap().text, "Moved words");
            let pending = app.content.existing.clone();
            let _ = frame(
                &mut app,
                vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
            );
            assert!(app.content.existing.is_none());
            assert_eq!(
                &*app.tabs[0].session.as_ref().unwrap().source,
                &source,
                "Escape cancels the move and typing without creating history"
            );
            assert!(!app.tabs[0].session.as_ref().unwrap().can_undo());
            app.content.existing = pending;
            app.finish_existing_text(&ctx);
            while app.content.busy {
                app.content_events(&ctx);
                assert!(std::time::Instant::now() < deadline, "{:?}", app.status);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(app.content.existing.is_none(), "{:?}", app.status);
            let committed = app.tabs[0].session.as_ref().unwrap().source.clone();
            let runs = mypdf_milestone0::text_geometry::runs(&committed, 1).unwrap();
            let edited: Vec<_> = runs
                .iter()
                .filter(|run| !run.text.is_empty() && run.text != "Unselected neighbor")
                .collect();
            assert_eq!(
                edited
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>(),
                "Moved words"
            );
            assert!(
                !runs
                    .iter()
                    .any(|r| r.text.contains("Move me") || r.text.contains("Second line"))
            );
            let actual = edited[0].rect.unwrap();
            assert!(
                (actual[0] - moved[0]).abs() < 4. && (actual[1] - moved[1]).abs() < 20.,
                "{actual:?} vs {moved:?}"
            );
            assert_eq!(
                runs.iter()
                    .find(|r| r.text == "Unselected neighbor")
                    .unwrap()
                    .rect,
                neighbor
            );
            app.editor_undo(false);
            let session = app.tabs[0].session.as_ref().unwrap();
            assert_eq!(
                &*session.source, &source,
                "one Undo restores original content and position"
            );
            assert!(
                !session.can_undo(),
                "movement and typing are one atomic edit"
            );
            app.editor_undo(true);
            assert_eq!(
                &*app.tabs[0].session.as_ref().unwrap().source,
                &*committed,
                "Redo restores the exact edited-and-moved PDF"
            );
        }
    }

    #[test]
    fn text_drag_keeps_final_release_position_for_coalesced_input() {
        for new_text in [false, true] {
            for event_mode in 0..3 {
                let ctx = egui::Context::default();
                let mut app =
                    Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
                app.new_blank(&ctx);
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                while app.tabs[0].pages.is_empty() {
                    app.events(&ctx);
                    assert!(std::time::Instant::now() < deadline);
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                let id = app.tabs[0].id;
                if new_text {
                    app.content
                        .place_text(id, 0, [50., 70.], app.tabs[0].pages[0]);
                    app.content.new_text = "Move text".into();
                } else {
                    app.content.existing = Some(InlineText {
                        key: (id, 0, 0),
                        target: InlineTarget::Run(0),
                        geometry: None,
                        original: "Move text".into(),
                        text: "Move text".into(),
                        rect: [50., 70., 170., 90.],
                        size: 18.,
                        color: [0.; 3],
                        font: "Helvetica".into(),
                        focus: false,
                    });
                }
                let frame = |app: &mut Viewer, events| {
                    let _ = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(800., 1000.))),
                            events,
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                overlay(
                                    &mut app.content,
                                    editing::Mode::Edit,
                                    &app.tabs[0],
                                    ui,
                                    0,
                                    Rect::from_min_size(pos2(20., 20.), vec2(595., 842.)),
                                    1.,
                                );
                            });
                        },
                    );
                };
                let button = |pos, pressed| egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                };
                frame(&mut app, vec![]);
                frame(&mut app, vec![]);
                let key = if new_text {
                    "move_text_block"
                } else {
                    "move_existing_block"
                };
                let start = ctx
                    .read_response(egui::Id::new((id, 0_usize, key)))
                    .unwrap()
                    .rect
                    .center();
                let end = start + vec2(130., 95.);
                if event_mode == 2 {
                    frame(
                        &mut app,
                        vec![
                            egui::Event::PointerMoved(start),
                            button(start, true),
                            egui::Event::PointerMoved(end),
                            button(end, false),
                        ],
                    );
                } else {
                    frame(
                        &mut app,
                        vec![egui::Event::PointerMoved(start), button(start, true)],
                    );
                    if event_mode == 0 {
                        frame(
                            &mut app,
                            vec![egui::Event::PointerMoved(start + vec2(40., 30.))],
                        );
                    }
                    frame(
                        &mut app,
                        vec![egui::Event::PointerMoved(end), button(end, false)],
                    );
                }
                let (actual, initial) = if new_text {
                    (app.content.new_rect, [50., 70.])
                } else {
                    let draft = app.content.existing.as_ref().expect("drag keeps editor");
                    let InlineTarget::Block(block) = &draft.target else {
                        panic!("fast drag promotes run: mode {event_mode}")
                    };
                    (draft.rect, block.initial_origin)
                };
                assert!(
                    (actual[0] - initial[0] - 130.).abs() < 0.01
                        && (actual[1] - initial[1] - 95.).abs() < 0.01,
                    "new={new_text} mode={event_mode}: {actual:?} vs {initial:?}"
                );
                frame(&mut app, vec![]);
                let retained = if new_text {
                    app.content.new_rect
                } else {
                    app.content.existing.as_ref().unwrap().rect
                };
                assert_eq!(actual, retained, "final placement remains after release");
            }
        }
    }

    #[test]
    fn text_move_clamps_to_page_and_handle_stays_reachable_at_small_zoom() {
        let page = PageSize {
            width: 595.,
            height: 842.,
        };
        let rect = [50., 70., 250., 130.];
        assert_eq!(
            moved_text_rect(rect, vec2(-100., -200.), page),
            [0., 0., 200., 60.]
        );
        assert_eq!(
            moved_text_rect(rect, vec2(1000., 2000.), page),
            [395., 782., 595., 842.]
        );
        for scale in [0.005, 0.05, 1., 4.] {
            let page = Rect::from_min_size(pos2(10., 10.), vec2(page.width, page.height) * scale);
            let block = Rect::from_min_size(page.min, vec2(200., 60.) * scale);
            let handle = text_move_handle(block, page);
            assert!(
                handle.is_finite() && page.contains_rect(handle),
                "{handle:?}, page: {page:?}"
            );
        }
    }

    #[test]
    fn pending_move_undo_commits_then_undoes_and_failure_keeps_previous_history() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let source = content::add_text(
            &convert::blank(595., 842.).unwrap(),
            1,
            "Move me",
            [50., 70., 300., 100.],
            18.,
            [0.; 3],
        )
        .unwrap();
        app.open_memory(PathBuf::from("Draft undo.pdf"), source.clone(), &ctx);
        let wait = |app: &mut Viewer| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while app.tabs[0].session.is_none() || app.content.busy {
                app.events(&ctx);
                app.content_events(&ctx);
                assert!(std::time::Instant::now() < deadline, "{:?}", app.status);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        wait(&mut app);
        let id = app.tabs[0].id;
        let run = mypdf_milestone0::text_geometry::runs(&source, 1)
            .unwrap()
            .remove(0);
        let original = InlineText {
            key: (id, 0, 0),
            target: InlineTarget::Run(0),
            geometry: run.layout.clone(),
            original: run.text.clone(),
            text: run.text,
            rect: run.rect.unwrap(),
            size: run.size,
            color: run.color,
            font: run.base_font,
            focus: false,
        };
        let mut moved =
            selected_block(vec![original], [50., 70., 350., 170.], app.tabs[0].pages[0]).unwrap();
        moved.text = "Moved words".into();
        moved.rect = moved_text_rect(moved.rect, vec2(45., 90.), app.tabs[0].pages[0]);
        app.content.existing = Some(moved.clone());
        assert!(!app.tabs[0].session.as_ref().unwrap().can_undo());
        assert!(
            app.content.has_pending_text_change(id),
            "toolbar Undo is available for the first unsaved draft"
        );
        assert!(app.content.placement_undo_owns_input(&ctx, id));
        app.action("edit_undo", &ctx);
        assert!(matches!(app.content.after_text, Some(AfterText::Undo(target, 0)) if target == id));
        assert_eq!(&*app.tabs[0].session.as_ref().unwrap().source, &source);
        app.finish_new_text(&ctx);
        wait(&mut app);
        assert!(app.content.existing.is_none(), "{:?}", app.status);
        assert_eq!(&*app.tabs[0].session.as_ref().unwrap().source, &source);
        assert!(app.tabs[0].session.as_ref().unwrap().can_redo());
        app.action("edit_redo", &ctx);
        let restored = app.tabs[0].session.as_ref().unwrap().source.clone();
        assert_ne!(&*restored, &source);
        assert_eq!(
            mypdf_milestone0::text_geometry::runs(&restored, 1)
                .unwrap()
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>(),
            "Moved words"
        );
        // A stale selected operator causes safe replacement to fail. Undo must
        // retain the draft and the already-committed previous change.
        moved.key.1 = app.tabs[0].session.as_ref().unwrap().revision();
        app.content.existing = Some(moved);
        app.action("undo", &ctx);
        app.finish_new_text(&ctx);
        wait(&mut app);
        assert_eq!(&*app.tabs[0].session.as_ref().unwrap().source, &*restored);
        assert!(app.tabs[0].session.as_ref().unwrap().can_undo());
        assert!(app.content.existing.is_some());
        assert!(app.content.after_text.is_none());
        assert!(app.status.as_ref().is_some_and(|(_, error)| *error));
    }

    #[test]
    fn pending_new_text_toolbar_undo_and_redo_preserve_the_complete_draft() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let source = convert::blank(595., 842.).unwrap();
        app.open_memory(PathBuf::from("New text undo.pdf"), source.clone(), &ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let id = app.tabs[0].id;
        app.content
            .place_text(id, 0, [50., 70.], app.tabs[0].pages[0]);
        app.content.new_text = "Added and moved".into();
        assert!(
            !app.content.placement_undo_owns_input(&ctx, id),
            "ordinary typing keeps its field undo"
        );
        app.content.new_rect[3] += 40.;
        assert!(
            !app.content.placement_undo_owns_input(&ctx, id),
            "automatic height growth is not a movement"
        );
        app.content.new_rect[0] += 45.;
        app.content.new_rect[1] += 90.;
        assert!(app.content.placement_undo_owns_input(&ctx, id));
        app.action("undo", &ctx);
        app.finish_new_text(&ctx);
        while app.content.busy {
            app.content_events(&ctx);
            assert!(std::time::Instant::now() < deadline, "{:?}", app.status);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let session = app.tabs[0].session.as_ref().unwrap();
        assert_eq!(&*session.source, &source);
        assert!(!session.can_undo() && session.can_redo());
        assert!(!app.content.new_placed && app.content.new_text.is_empty());
        app.action("redo", &ctx);
        let runs =
            mypdf_milestone0::text_geometry::runs(&app.tabs[0].session.as_ref().unwrap().source, 1)
                .unwrap();
        assert_eq!(
            runs.iter().map(|run| run.text.as_str()).collect::<String>(),
            "Added and moved"
        );
        let rect = runs.iter().find_map(|run| run.rect).unwrap();
        assert!(rect[0] > 90. && rect[1] > 150., "{rect:?}");
    }

    #[test]
    fn formatting_only_existing_block_commits_and_undo_restores_original() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let source = content::add_text(
            &convert::blank(595., 842.).unwrap(),
            1,
            "Plain selected end",
            [50., 70., 400., 100.],
            18.,
            [0.; 3],
        )
        .unwrap();
        app.open_memory(PathBuf::from("Selection.pdf"), source.clone(), &ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let run = mypdf_milestone0::text_geometry::runs(&source, 1)
            .unwrap()
            .remove(0);
        let item = InlineText {
            key: (app.tabs[0].id, 0, 0),
            target: InlineTarget::Run(0),
            geometry: run.layout.clone(),
            original: run.text.clone(),
            text: run.text,
            rect: run.rect.unwrap(),
            size: run.size,
            color: run.color,
            font: run.base_font,
            focus: false,
        };
        let mut draft =
            selected_block(vec![item], [50., 70., 450., 170.], app.tabs[0].pages[0]).unwrap();
        let InlineTarget::Block(block) = &mut draft.target else {
            panic!()
        };
        block.selected_chars = Some(6..14);
        formatting::apply(
            &draft.text,
            &mut block.spans,
            &block.selected_chars,
            formatting::Change::Italic(true),
        );
        formatting::apply(
            &draft.text,
            &mut block.spans,
            &block.selected_chars,
            formatting::Change::Color([0.8, 0.1, 0.2]),
        );
        assert!(
            draft.changed(),
            "Formatting without typing is a real document change"
        );
        app.content.existing = Some(draft);
        app.finish_existing_text(&ctx);
        while app.content.busy {
            app.content_events(&ctx);
            assert!(std::time::Instant::now() < deadline, "{:?}", app.status);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let session = app.tabs[0].session.as_mut().unwrap();
        let pdf = session.snapshot_bytes().unwrap();
        let runs = mypdf_milestone0::text_geometry::runs(&pdf, 1).unwrap();
        let selected = runs.iter().find(|r| r.text.contains("selected")).unwrap();
        assert!((selected.color[0] - 0.8).abs() < 0.001);
        assert!(
            selected.base_font.to_lowercase().contains("italic")
                || selected.base_font.to_lowercase().contains("oblique")
        );
        for other in runs
            .iter()
            .filter(|r| r.text.contains("Plain") || r.text.contains("end"))
        {
            assert_eq!(other.color, [0.; 3]);
        }
        session.undo();
        assert_eq!(&*session.source, &source);
        session.redo();
        assert_eq!(session.snapshot_bytes().unwrap(), pdf);
    }

    #[test]
    fn clicking_a_line_never_includes_neighboring_lines_or_columns() {
        let item = |index, text: &str, rect: [f32; 4], font: &str, color| InlineText {
            key: (1, 0, 0),
            target: InlineTarget::Run(index),
            geometry: Some(document::RunLayout {
                text: text.into(),
                origin: [rect[0], rect[3]],
                end: [rect[2], rect[3]],
                direction: [1., 0.],
                space_width: 4.,
            }),
            original: text.into(),
            text: text.into(),
            rect,
            size: 14.,
            color,
            font: font.into(),
            focus: true,
        };
        let candidates = vec![
            item(0, "MICROSOFT", [50., 70., 130., 84.], "Arial-Bold", [0.; 3]),
            item(1, "STORE", [134., 70., 180., 84.], "Arial", [0.8, 0., 0.]),
            item(
                2,
                "Version 8.11",
                [50., 89., 180., 103.],
                "Arial-Bold",
                [0.; 3],
            ),
            item(3, "Other column", [300., 70., 430., 84.], "Arial", [0.; 3]),
            item(
                4,
                "Publish Date: March 17, 2026",
                [50., 108., 280., 122.],
                "Arial-Bold",
                [0.; 3],
            ),
        ];
        let page = PageSize {
            width: 595.,
            height: 842.,
        };
        for i in 0..2 {
            let line = line_at(candidates[i].clone(), &candidates, page);
            assert_eq!(line.text, "MICROSOFT STORE");
            assert!(!line.changed(), "Selecting must not rewrite the source");
            let InlineTarget::Block(block) = line.target else {
                panic!("expected line")
            };
            assert_eq!(block.selection.runs.len(), 2);
            assert_eq!(&line.text[block.spans[1].range.clone()], "STORE");
            assert_eq!(block.spans[1].color, [0.8, 0., 0.]);
            assert_ne!(block.spans[0].font, block.spans[1].font);
        }
        for i in 2..candidates.len() {
            let line = line_at(candidates[i].clone(), &candidates, page);
            assert_eq!(line.text, candidates[i].text);
            let InlineTarget::Block(block) = line.target else {
                panic!("expected line")
            };
            assert_eq!(block.selection.runs.len(), 1);
        }
        let mut rotated = candidates[0].clone();
        rotated.geometry.as_mut().unwrap().direction = [0., 1.];
        assert!(matches!(
            line_at(rotated.clone(), &[rotated], page).target,
            InlineTarget::Run(_)
        ));
    }
    #[test]
    fn area_edit_preserves_baselines_indents_styles_and_neighboring_text() {
        let mut source = convert::blank(595., 842.).unwrap();
        for (text, rect, size, color) in [
            ("MICROSOFT STORE", [50., 50., 480., 40.], 20., [0., 0., 0.]),
            ("Version 8.11", [50., 82., 480., 30.], 12., [0., 0., 0.]),
            (
                "Publish Date: March 17, 2026",
                [50., 102., 480., 30.],
                12.,
                [0., 0., 0.],
            ),
            (
                "Indented paragraph",
                [80., 180., 450., 40.],
                14.,
                [0., 0.2, 0.7],
            ),
            (
                "Unselected footer",
                [50., 760., 480., 40.],
                12.,
                [0., 0., 0.],
            ),
        ] {
            source = content::add_text(&source, 1, text, rect, size, color).unwrap();
        }
        let runs = mypdf_milestone0::text_geometry::runs(&source, 1).unwrap();
        let items: Vec<_> = runs
            .iter()
            .enumerate()
            .filter(|(_, r)| r.text != "Unselected footer")
            .map(|(i, r)| InlineText {
                key: (1, 0, 0),
                target: InlineTarget::Run(i),
                geometry: r.layout.clone(),
                original: r.text.clone(),
                text: r.text.clone(),
                rect: r.rect.unwrap(),
                size: r.size,
                color: r.color,
                font: r.base_font.clone(),
                focus: true,
            })
            .collect();
        let page = PageSize {
            width: 595.,
            height: 842.,
        };
        let mut draft = selected_block(items.clone(), [40., 40., 540., 240.], page).unwrap();
        assert_eq!(
            draft.text,
            "MICROSOFT STORE\nVersion 8.11\nPublish Date: March 17, 2026\nIndented paragraph"
        );
        assert!(!draft.changed());
        let line = line_at(items[0].clone(), &items, page);
        assert_eq!(line.text, "MICROSOFT STORE");
        let InlineTarget::Block(single) = &line.target else {
            panic!()
        };
        assert_eq!(single.selection.runs.len(), 1);
        assert!(
            line.rect[3] < items[1].rect[1],
            "A one-line editor must not cover the next line"
        );
        draft.text = draft.text.replace("MICROSOFT STORE", "MY STORE");
        let InlineTarget::Block(block) = &draft.target else {
            panic!()
        };
        let pdf = text_block::replace(
            &source,
            1,
            &text_block::Replacement {
                selection: block.selection.clone(),
                text: draft.text.clone(),
                rect: [
                    draft.rect[0],
                    draft.rect[1],
                    draft.rect[2] - draft.rect[0],
                    draft.rect[3] - draft.rect[1],
                ],
                size: draft.size,
                color: draft.color,
                format: TextFormat::default(),
                spans: mypdf_milestone0::rich_text::edited(
                    &block.styled_text,
                    &draft.text,
                    &block.spans,
                ),
                layout: mypdf_milestone0::rich_text::edited_layout(
                    &block.styled_text,
                    &draft.text,
                    &block.layout,
                    draft.size * 1.3,
                ),
            },
        )
        .unwrap();
        let after = mypdf_milestone0::text_geometry::runs(&pdf, 1).unwrap();
        for old in runs.iter().filter(|r| r.text != "MICROSOFT STORE") {
            let new = after
                .iter()
                .find(|r| r.text == old.text)
                .unwrap_or_else(|| panic!("missing {}", old.text));
            assert_eq!(new.size, old.size);
            assert_eq!(new.color, old.color);
            let a = old.layout.as_ref().unwrap().origin;
            let b = new.layout.as_ref().unwrap().origin;
            assert!(
                (a[0] - b[0]).abs() < 0.1 && (a[1] - b[1]).abs() < 0.1,
                "{}: {a:?} vs {b:?}",
                old.text
            );
        }
        // Editing a clicked line leaves all other PDF operators and layout alone.
        let pdf = text_block::replace(
            &source,
            1,
            &text_block::Replacement {
                selection: single.selection.clone(),
                text: "TITLE".into(),
                rect: [
                    line.rect[0],
                    line.rect[1],
                    line.rect[2] - line.rect[0],
                    line.rect[3] - line.rect[1],
                ],
                size: line.size,
                color: line.color,
                format: TextFormat::default(),
                spans: mypdf_milestone0::rich_text::edited(&line.text, "TITLE", &single.spans),
                layout: single.layout.clone(),
            },
        )
        .unwrap();
        let after = mypdf_milestone0::text_geometry::runs(&pdf, 1).unwrap();
        for old in runs.iter().skip(1) {
            let new = after.iter().find(|r| r.text == old.text).unwrap();
            assert_eq!(new.rect, old.rect);
            assert_eq!(new.base_font, old.base_font);
        }
    }

    #[test]
    fn mixed_size_area_rows_follow_baselines_and_multiline_notes_keep_breaks() {
        let item = |index, text: &str, rect: [f32; 4], baseline, size| InlineText {
            key: (1, 0, 0),
            target: InlineTarget::Run(index),
            geometry: Some(document::RunLayout {
                text: text.into(),
                origin: [rect[0], baseline],
                end: [rect[2], baseline],
                direction: [1., 0.],
                space_width: 4.,
            }),
            original: text.into(),
            text: text.into(),
            rect,
            size,
            color: [0.; 3],
            font: "Arial".into(),
            focus: true,
        };
        let page = PageSize {
            width: 595.,
            height: 842.,
        };
        let items = vec![
            item(0, "Earlier", [50., 90., 100., 102.], 100., 12.),
            item(1, "Large", [50., 80., 140., 125.], 120., 40.),
            item(2, "small", [144., 110., 180., 122.], 120., 12.),
        ];
        let draft = selected_block(items, [40., 70., 300., 160.], page).unwrap();
        assert_eq!(draft.text, "Earlier\nLarge small");
        let InlineTarget::Block(block) = draft.target else {
            panic!()
        };
        assert_eq!(block.layout.len(), 2);
        assert!((block.layout[1].baseline - block.layout[0].baseline - 20.).abs() < 0.01);
        let mut note = item(0, "First\nSecond", [50., 70., 300., 150.], 90., 16.);
        note.target = InlineTarget::Annotation("note".into());
        note.geometry = None;
        let draft = selected_block(vec![note], [40., 60., 320., 170.], page).unwrap();
        assert_eq!(draft.text, "First\nSecond");
        assert!(!draft.changed());
        let InlineTarget::Block(block) = draft.target else {
            panic!()
        };
        assert!(block.layout.is_empty());
        assert!(!block.preserve_layout);
    }

    #[test]
    fn rectangular_selection_types_reflows_cancels_and_saves_in_place() {
        use eframe::App;
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let source = content::add_text(
            &convert::blank(595., 842.).unwrap(),
            1,
            "First line\nSecond line",
            [50., 70., 400., 100.],
            18.,
            [0.; 3],
        )
        .unwrap();
        app.open_memory(PathBuf::from("Rectangle.pdf"), source.clone(), &ctx);
        app.editor.mode = editing::Mode::Edit;
        app.content.area_mode = true;
        let frame = |app: &mut Viewer, events| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 820.))),
                    max_texture_side: Some(8192),
                    events,
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
        };
        let wait =
            |app: &mut Viewer| {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                for _ in 0..3 {
                    frame(app, vec![]);
                }
                while app.content.loading
                    || app.content.busy
                    || !app.content.inspection.as_ref().is_some_and(|i| {
                        i.key.1 == app.tabs[0].session.as_ref().unwrap().revision()
                    })
                {
                    assert!(std::time::Instant::now() < deadline, "{:?}", app.status);
                    frame(app, vec![]);
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                for _ in 0..3 {
                    frame(app, vec![]);
                }
            };
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let drag = |app: &mut Viewer, start, end| {
            frame(
                app,
                vec![
                    egui::Event::PointerMoved(start),
                    egui::Event::PointerButton {
                        pos: start,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            frame(app, vec![egui::Event::PointerMoved(end)]);
            frame(
                app,
                vec![
                    egui::Event::PointerMoved(end),
                    egui::Event::PointerButton {
                        pos: end,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            for _ in 0..3 {
                frame(app, vec![]);
            }
        };
        wait(&mut app);
        let id = app.tabs[0].id;
        let page = ctx
            .read_response(egui::Id::new((id, 0_usize, "text_page_background")))
            .unwrap()
            .rect;
        let scale = page.width() / 595.;
        let start = page.min + vec2(40., 60.) * scale;
        let end = page.min + vec2(420., 160.) * scale;
        drag(&mut app, start, end);
        let draft = app
            .content
            .existing
            .as_ref()
            .expect("drag opens the editor");
        assert!(matches!(&draft.target,InlineTarget::Block(b) if b.selection.runs.len()==2));
        assert_eq!(draft.text, "First line\nSecond line");
        assert!(ctx.memory(|m| m.has_focus(egui::Id::new((id, 0_usize, "inline_existing_text")))));
        frame(
            &mut app,
            vec![
                key(egui::Key::A, egui::Modifiers::COMMAND),
                egui::Event::Text("Cancel this block".into()),
            ],
        );
        frame(
            &mut app,
            vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
        );
        assert!(app.content.existing.is_none());
        assert_eq!(&*app.tabs[0].session.as_ref().unwrap().source, &source);
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        drag(&mut app, start, end);
        frame(
            &mut app,
            vec![
                key(egui::Key::A, egui::Modifiers::COMMAND),
                egui::Event::Text(
                    "One edited block in İstanbul\nA second paragraph\nThird line".into(),
                ),
            ],
        );
        let handle = ctx
            .read_response(egui::Id::new((id, 0_usize, "resize_existing_block")))
            .unwrap()
            .rect
            .center();
        let old = app.content.existing.as_ref().unwrap().rect;
        drag(&mut app, handle, handle + vec2(45., 35.));
        assert!(app.content.existing.as_ref().unwrap().rect[2] > old[2]);
        frame(
            &mut app,
            vec![key(egui::Key::Enter, egui::Modifiers::COMMAND)],
        );
        wait(&mut app);
        assert!(app.content.existing.is_none(), "{:?}", app.status);
        let session = app.tabs[0].session.as_ref().unwrap();
        let out = tempfile::tempdir().unwrap();
        let path = out.path().join("block.pdf");
        let bytes = session.snapshot_bytes().unwrap();
        mypdf_milestone0::write_new(&path, &bytes).unwrap();
        let saved = std::fs::read(path).unwrap();
        let text =
            String::from_utf8(convert::export_text(&saved, &[0], Format::Text).unwrap()).unwrap();
        assert!(
            text.contains("One edited block in İstanbul") && text.contains("Third line"),
            "{text}"
        );
        assert!(!text.contains("First line") && !text.contains("Second line"));
        app.editor_undo(false);
        assert_eq!(&*app.tabs[0].session.as_ref().unwrap().source, &source);
    }
    #[test]
    fn dragging_empty_space_selects_an_area_and_outside_clicks_close_it_untouched() {
        use eframe::App;
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let source = content::add_text(
            &convert::blank(595., 842.).unwrap(),
            1,
            "First line\nSecond line",
            [50., 70., 400., 100.],
            18.,
            [0.; 3],
        )
        .unwrap();
        app.open_memory(PathBuf::from("Area.pdf"), source.clone(), &ctx);
        app.editor.mode = editing::Mode::Edit;
        assert!(!app.content.area_mode, "no Select area tool needed");
        let run = |app: &mut Viewer, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 820.))),
                    max_texture_side: Some(8192),
                    events,
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            )
        };
        let frame = |app: &mut Viewer, events| {
            let _ = run(app, events);
        };
        let wait =
            |app: &mut Viewer| {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                for _ in 0..3 {
                    frame(app, vec![]);
                }
                while app.content.loading
                    || app.content.busy
                    || !app.content.inspection.as_ref().is_some_and(|i| {
                        i.key.1 == app.tabs[0].session.as_ref().unwrap().revision()
                    })
                {
                    assert!(std::time::Instant::now() < deadline, "{:?}", app.status);
                    frame(app, vec![]);
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                for _ in 0..3 {
                    frame(app, vec![]);
                }
            };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let drag = |app: &mut Viewer, start, end| {
            frame(
                app,
                vec![egui::Event::PointerMoved(start), button(start, true)],
            );
            frame(app, vec![egui::Event::PointerMoved(end)]);
            frame(
                app,
                vec![egui::Event::PointerMoved(end), button(end, false)],
            );
            for _ in 0..3 {
                frame(app, vec![]);
            }
        };
        let click = |app: &mut Viewer, at| {
            frame(app, vec![egui::Event::PointerMoved(at), button(at, true)]);
            frame(app, vec![button(at, false)]);
            for _ in 0..3 {
                frame(app, vec![]);
            }
        };
        wait(&mut app);
        let id = app.tabs[0].id;
        let page = ctx
            .read_response(egui::Id::new((id, 0_usize, "text_page_background")))
            .unwrap()
            .rect;
        let scale = page.width() / 595.;
        let start = page.min + vec2(40., 60.) * scale;
        let end = page.min + vec2(420., 160.) * scale;
        drag(&mut app, start, end);
        let block = app
            .content
            .existing
            .as_ref()
            .expect("dragging opens a block");
        assert!(matches!(&block.target, InlineTarget::Block(b) if b.selection.runs.len() == 2));
        let area = Rect::from_min_max(
            page.min + vec2(block.rect[0], block.rect[1]) * scale,
            page.min + vec2(block.rect[2], block.rect[3]) * scale,
        );

        // Only the replaced words are masked; lines, boxes and images around them stay.
        fn white_rects(shape: &egui::Shape, out: &mut Vec<Rect>) {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| white_rects(s, out)),
                egui::Shape::Rect(r) if r.fill == Color32::WHITE => out.push(r.rect),
                _ => {}
            }
        }
        let mut masks = vec![];
        for clipped in run(&mut app, vec![]).shapes {
            white_rects(&clipped.shape, &mut masks);
        }
        // The page's own paper is white, too.
        masks.retain(|m| !m.contains_rect(page.shrink(1.)));
        assert!(
            !masks
                .iter()
                .any(|m| m.intersect(area).area() > area.area() * 0.6),
            "the block area is not painted over: {masks:?} vs {area:?}"
        );

        // A click on the gray surround or on empty page space closes an untouched block.
        click(
            &mut app,
            page.min + vec2(-super::reading::GUTTER / 2., 300.),
        );
        assert!(app.content.existing.is_none());
        drag(&mut app, start, end);
        assert!(app.content.existing.is_some());
        click(&mut app, page.min + vec2(300., 420.) * scale);
        wait(&mut app);
        assert!(app.content.existing.is_none());
        assert_eq!(
            &*app.tabs[0].session.as_ref().unwrap().source,
            &source,
            "nothing rewritten"
        );

        // Typed changes are kept when clicking outside.
        drag(&mut app, start, end);
        frame(
            &mut app,
            vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                },
                egui::Event::Text("Rewritten block".into()),
            ],
        );
        click(&mut app, page.min + vec2(300., 420.) * scale);
        wait(&mut app);
        assert!(app.content.existing.is_none(), "{:?}", app.status);
        let bytes = app.tabs[0]
            .session
            .as_ref()
            .unwrap()
            .snapshot_bytes()
            .unwrap();
        let text =
            String::from_utf8(convert::export_text(&bytes, &[0], Format::Text).unwrap()).unwrap();
        assert!(
            text.contains("Rewritten block") && !text.contains("First line"),
            "{text}"
        );
    }
    #[test]
    fn existing_text_and_text_annotations_edit_in_place_and_cancel_safely() {
        use eframe::App;
        use mypdf_milestone0::annotations::{Annotation, Kind, Style};
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let source = convert::blank(595., 842.).unwrap();
        let source = content::add_text(
            &source,
            1,
            "Repeated",
            [50., 70., 200., 60.],
            18.,
            [0., 0., 0.],
        )
        .unwrap();
        let source = content::add_text(
            &source,
            1,
            "Repeated",
            [50., 150., 200., 60.],
            18.,
            [0., 0., 0.],
        )
        .unwrap();
        app.open_memory(PathBuf::from("Inline.pdf"), source, &ctx);
        app.editor.mode = editing::Mode::Edit;
        let frame = |app: &mut Viewer, events| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 820.))),
                    max_texture_side: Some(8192),
                    events,
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
        };
        let wait =
            |app: &mut Viewer, ready: fn(&Viewer) -> bool| {
                let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
                for _ in 0..3 {
                    frame(app, vec![]);
                }
                while !ready(app)
                    || app.content.loading
                    || !app.content.inspection.as_ref().is_some_and(|i| {
                        i.key.1 == app.tabs[0].session.as_ref().unwrap().revision()
                    })
                {
                    assert!(
                        std::time::Instant::now() < end,
                        "inline test timed out: {:?}",
                        app.status
                    );
                    frame(app, vec![]);
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                for _ in 0..3 {
                    frame(app, vec![]);
                }
            };
        let click = |app: &mut Viewer, p| {
            for pressed in [true, false] {
                frame(
                    app,
                    vec![
                        egui::Event::PointerMoved(p),
                        egui::Event::PointerButton {
                            pos: p,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
        };
        let key = |k, modifiers| egui::Event::Key {
            key: k,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        wait(&mut app, |a| {
            a.content.inspection.is_some() && !a.content.loading
        });
        let a = Annotation::new(
            0,
            [50., 240., 320., 300.],
            Kind::Text {
                text: "Your text".into(),
            },
            Style::default(),
        );
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![a])
            .unwrap();
        wait(&mut app, |a| {
            a.content
                .inspection
                .as_ref()
                .is_some_and(|i| i.key.1 == a.tabs[0].session.as_ref().unwrap().revision())
        });
        let id = app.tabs[0].id;
        let r = ctx
            .read_response(egui::Id::new((id, 0_usize, "existing_text", 1_usize)))
            .unwrap()
            .rect;
        click(&mut app, r.center());
        frame(
            &mut app,
            vec![
                key(egui::Key::A, egui::Modifiers::COMMAND),
                egui::Event::Text("İstanbul!".into()),
            ],
        );
        assert_eq!(app.content.existing.as_ref().unwrap().text, "İstanbul!");
        assert!(ctx.memory(|m| m.has_focus(egui::Id::new((id, 0_usize, "inline_existing_text")))));
        frame(
            &mut app,
            vec![key(egui::Key::Enter, egui::Modifiers::COMMAND)],
        );
        wait(&mut app, |a| {
            a.content.existing.is_none() && !a.content.loading && !a.content.busy
        });
        let runs: Vec<_> = document::text_runs(&app.tabs[0].session.as_ref().unwrap().source, 1)
            .unwrap()
            .into_iter()
            .filter(|r| !r.text.is_empty())
            .collect();
        assert_eq!(
            runs.iter().filter(|r| r.text == "Repeated").count(),
            1,
            "Only the clicked occurrence is replaced"
        );
        assert_eq!(runs[0].text, "Repeated");
        assert_eq!(
            runs.iter()
                .skip(1)
                .map(|r| r.text.as_str())
                .collect::<String>(),
            "İstanbul!"
        );
        let r = ctx
            .read_response(egui::Id::new((id, 0_usize, "existing_text", runs.len())))
            .unwrap()
            .rect;
        click(&mut app, r.center());
        frame(
            &mut app,
            vec![
                key(egui::Key::A, egui::Modifiers::COMMAND),
                egui::Event::Text("Text box changed in place".into()),
            ],
        );
        frame(
            &mut app,
            vec![key(egui::Key::Enter, egui::Modifiers::COMMAND)],
        );
        wait(&mut app, |a| {
            a.content.existing.is_none() && !a.content.loading
        });
        assert!(
            matches!(&app.tabs[0].session.as_ref().unwrap().annotations[0].kind,Kind::Text{text} if text == "Text box changed in place")
        );
        let revision = app.tabs[0].session.as_ref().unwrap().revision();
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        let r = ctx
            .read_response(egui::Id::new((id, 0_usize, "existing_text", 0_usize)))
            .unwrap()
            .rect;
        click(&mut app, r.center());
        frame(
            &mut app,
            vec![
                key(egui::Key::A, egui::Modifiers::COMMAND),
                egui::Event::Text("Cancel me".into()),
                key(egui::Key::Escape, egui::Modifiers::NONE),
            ],
        );
        assert!(app.content.existing.is_none());
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), revision);
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        let r = ctx
            .read_response(egui::Id::new((id, 0_usize, "existing_text", 0_usize)))
            .unwrap()
            .rect;
        click(&mut app, r.center());
        frame(
            &mut app,
            vec![
                key(egui::Key::A, egui::Modifiers::COMMAND),
                egui::Event::Text("First expanded text".into()),
            ],
        );
        assert!(
            app.content
                .existing
                .as_ref()
                .is_some_and(|e| matches!(&e.target, InlineTarget::Block(b) if b.selection.runs.len()==1 && b.selection.runs[0].0==0)),
            "wrong target: {:?} at {:?}",
            app.content.existing.as_ref().map(|e| (&e.target, &e.text)),
            r
        );
        let second = ctx
            .read_response(egui::Id::new((id, 0_usize, "existing_text", 1_usize)))
            .unwrap()
            .rect;
        click(&mut app, second.center());
        wait(&mut app, |a| {
            !a.content.busy
                && a.content
                    .existing
                    .as_ref()
                    .is_some_and(|e| e.original == "İstanbul!")
        });
        frame(
            &mut app,
            vec![
                key(egui::Key::A, egui::Modifiers::COMMAND),
                egui::Event::Text("İstanbul!".into()),
            ],
        );
        // Closing must commit or finish the on-page editor before asking about the PDF.
        app.request_close(Some(id), &ctx);
        wait(&mut app, |a| a.close_intent.is_some());
        assert!(app.content.existing.is_none());
        app.close_intent = None;
        let bytes = app.tabs[0]
            .session
            .as_ref()
            .unwrap()
            .snapshot_bytes()
            .unwrap();
        let doc = lopdf::Document::load_mem(&bytes).unwrap();
        assert!(doc.extract_text(&[1]).unwrap().contains("İstanbul!"));
        assert!(
            matches!(&mypdf_milestone0::annotations::load(&doc)[0].kind,Kind::Text{text} if text=="Text box changed in place")
        );
    }

    #[test]
    fn typing_on_page_and_command_return_commit_selectable_text() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.content.add_target = Some((app.tabs[0].id, 0));
        let run = |app: &mut Viewer, events| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(800., 1000.))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        overlay(
                            &mut app.content,
                            editing::Mode::Edit,
                            &app.tabs[0],
                            ui,
                            0,
                            Rect::from_min_size(pos2(20., 20.), vec2(595., 842.)),
                            1.,
                        )
                    });
                },
            );
        };
        run(&mut app, vec![]);
        for pressed in [true, false] {
            run(
                &mut app,
                vec![
                    egui::Event::PointerMoved(pos2(120., 150.)),
                    egui::Event::PointerButton {
                        pos: pos2(120., 150.),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(app.content.new_placed);
        run(
            &mut app,
            vec![egui::Event::Text("Written on the page".into())],
        );
        assert_eq!(app.content.new_text, "Written on the page");
        run(
            &mut app,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }],
        );
        assert!(app.content.new_commit);
        app.finish_new_text(&ctx);
        while app.content.busy {
            app.content_events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!app.content.new_placed);
        assert!(app.content.new_text.is_empty());
        let session = app.tabs[0].session.as_ref().unwrap();
        assert!(
            lopdf::Document::load_mem(&session.source)
                .unwrap()
                .extract_text(&[1])
                .unwrap()
                .contains("Written on the page")
        );
        assert_eq!(session.revision(), 1);
        for pressed in [true, false] {
            run(
                &mut app,
                vec![
                    egui::Event::PointerMoved(pos2(100., 400.)),
                    egui::Event::PointerButton {
                        pos: pos2(100., 400.),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        run(&mut app, vec![egui::Event::Text("Second block".into())]);
        for pressed in [true, false] {
            run(
                &mut app,
                vec![
                    egui::Event::PointerMoved(pos2(100., 620.)),
                    egui::Event::PointerButton {
                        pos: pos2(100., 620.),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(app.content.new_commit);
        app.finish_new_text(&ctx);
        while app.content.busy {
            app.content_events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(app.content.new_placed && app.content.new_text.is_empty());
        assert_eq!(app.content.new_rect[1], 600.);
        assert!(
            lopdf::Document::load_mem(&app.tabs[0].session.as_ref().unwrap().source)
                .unwrap()
                .extract_text(&[1])
                .unwrap()
                .contains("Second block")
        );
    }

    #[test]
    fn pending_text_is_committed_before_close_and_invalid_text_stays_open() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.content.add_target = Some((app.tabs[0].id, 0));
        app.content.new_text = "Keep this text".into();
        app.content.new_placed = true;
        app.request_close(None, &ctx);
        assert!(!app.exit_approved && app.close_intent.is_none() && app.content.busy);
        while app.content.busy {
            app.content_events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(app.close_intent, Some(None));
        assert!(!app.exit_approved);
        assert!(
            lopdf::Document::load_mem(&app.tabs[0].session.as_ref().unwrap().source)
                .unwrap()
                .extract_text(&[1])
                .unwrap()
                .contains("Keep this text")
        );
        app.close_intent = None;
        app.content.new_text = "CannotDiscardThisOversizedWord".into();
        app.content.new_rect = [20., 20., 10., 20.];
        app.content.new_size = 72.;
        app.content.new_placed = true;
        app.request_close(None, &ctx);
        while app.content.busy {
            app.content_events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(app.close_intent.is_none() && !app.exit_approved);
        assert_eq!(app.content.new_text, "CannotDiscardThisOversizedWord");
        assert!(app.content.after_text.is_none());
    }

    #[test]
    fn new_text_canvas_drag_and_resize_update_the_pdf_block() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.tabs[0].pages.is_empty() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.content.add_target = Some((app.tabs[0].id, 0));
        app.content.new_rect = [40., 60., 200., 80.];
        app.content.new_placed = true;
        let run = |app: &mut Viewer, events| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(800., 1000.))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        overlay(
                            &mut app.content,
                            editing::Mode::Edit,
                            &app.tabs[0],
                            ui,
                            0,
                            Rect::from_min_size(pos2(20., 20.), vec2(595., 842.)),
                            1.,
                        )
                    });
                },
            );
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        run(&mut app, vec![]);
        run(
            &mut app,
            vec![
                egui::Event::PointerMoved(pos2(260., 160.)),
                button(pos2(260., 160.), true),
            ],
        );
        run(&mut app, vec![egui::Event::PointerMoved(pos2(300., 190.))]);
        run(&mut app, vec![button(pos2(300., 190.), false)]);
        assert_eq!(app.content.new_rect, [40., 60., 240., 110.]);
        run(
            &mut app,
            vec![
                egui::Event::PointerMoved(pos2(180., 73.)),
                button(pos2(180., 73.), true),
            ],
        );
        run(&mut app, vec![egui::Event::PointerMoved(pos2(220., 123.))]);
        run(&mut app, vec![button(pos2(220., 123.), false)]);
        assert_eq!(app.content.new_rect, [80., 110., 240., 110.]);
    }
}
