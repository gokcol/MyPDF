//! Native content inspector, AcroForm panel, and local conversion workflows.
use super::*;
use mypdf_milestone0::{
    content,
    convert::{self, Format},
    document, form_design, forms, text_block,
    text_style::{self, Alignment, TextFormat},
};

type Key = (u64, u64, usize);
#[derive(Clone, PartialEq, Debug)]
enum InlineTarget {
    Run(usize),
    Annotation(String),
    Block(BlockDraft),
}
#[derive(Clone, PartialEq, Debug)]
struct BlockDraft {
    selection: text_block::Selection,
    masks: Vec<[f32; 4]>,
    font: usize,
    /// Font, size, color and right edge when the block opened.
    initial: (usize, f32, [f32; 3], f32),
}
#[derive(Clone)]
struct InlineText {
    key: Key,
    target: InlineTarget,
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
                if (b.font, self.size, self.color, self.rect[2]) != b.initial)
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
    AddText {
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
}
pub(super) enum AfterText {
    Save(u64),
    Close(Option<u64>),
}
pub(super) struct ContentState {
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
    focus_new_text: bool,
    new_rect: [f32; 4],
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
    pub image_mode: bool,
    /// Another tool (Redact) owns the page overlay.
    pub(super) paused: bool,
    rect: [f32; 4],
    pub busy: bool,
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
/// The page rendered without the words an open block replaces, so the editor
/// hides only those words and keeps backgrounds, lines and boxes around them.
struct Erased {
    key: Key,
    masks: Vec<[f32; 4]>,
    rx: Option<mpsc::Receiver<Option<egui::ColorImage>>>,
    texture: Option<TextureHandle>,
}
impl ContentState {
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
        self.new_text.clear();
        let x = point[0].clamp(0., (size.width - 10.).max(0.));
        let w = 240_f32.min(size.width - x);
        let h = (self.new_size * 2.6).min(size.height);
        self.new_rect = [x, point[1].clamp(0., (size.height - h).max(0.)), w, h];
        self.focus_new_text = true;
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
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let (inspect_tx, inspect_rx) = mpsc::channel();
        let (output_tx, output_rx) = mpsc::channel();
        Self {
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
            focus_new_text: false,
            new_rect: [48., 48., 300., 120.],
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
            image_mode: false,
            paused: false,
            rect: [0.; 4],
            busy: false,
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
    pub(super) fn content_events(&mut self, ctx: &egui::Context) {
        let mut after_text = None;
        let changes: Vec<_> = self.content.rx.try_iter().collect();
        for result in changes {
            self.content.busy = false;
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
                        match s.commit_document(bytes) {
                            Ok(()) => {
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
                                    self.content.new_text.clear();
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
                                        "Changes applied • Undo is available • Save a Copy to keep them"
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
                AfterText::Save(id) => {
                    if let Some(index) = self.tabs.iter().position(|t| t.id == id) {
                        self.active = index;
                        self.save_dialog(ctx);
                    }
                }
                AfterText::Close(id) => self.request_close(id, ctx),
            }
        }
    }
    pub(super) fn content_change(&mut self, ctx: &egui::Context, edit: Edit) {
        if self.content.busy || self.organizer.busy || !self.apply_pending_annotation() {
            return;
        }
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
        self.content.busy = true;
        self.status = Some(("Applying changes…".into(), false));
        std::thread::spawn(move || {
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> anyhow::Result<_> {
                    let bytes = if revision == 0 {
                        source.to_vec()
                    } else {
                        mypdf_milestone0::annotations::export(&source, &annotations)?
                    };
                    let mut note = None;
                    let result = match edit {
                        Edit::AddText {
                            page,
                            text,
                            rect,
                            size,
                            color,
                            format,
                        } => content::add_text_styled(
                            &bytes, page, &text, rect, size, color, &format,
                        )?,
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
                        Edit::ReduceSize(level) => {
                            let (bytes, r) = mypdf_milestone0::compress::reduce(&bytes, level)?;
                            note = Some(format!(
                                "Reduced from {} to {} ({}% smaller) • Save a Copy to keep it",
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
                                "Redacted {} • Save a Copy to keep it; Undo is available until then",
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
                                "Flattened {pages} page{} into images • Save a Copy to keep it; it can still be signed",
                                if pages == 1 { "" } else { "s" }
                            ));
                            bytes
                        }
                        Edit::HeaderFooter(stamp) => {
                            let (bytes, pages) =
                                mypdf_milestone0::header_footer::apply(&bytes, &stamp)?;
                            note = Some(format!(
                                "Added to {pages} page{} • Save a Copy to keep your changes",
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
        egui::SidePanel::right("content_inspector").exact_width(310.).resizable(false).frame(egui::Frame::new().fill(theme::paper()).inner_margin(18.)).show(ctx,|ui|{
            ui.heading("Edit PDF");ui.label(RichText::new(format!("Page {}",page+1)).small().color(theme::muted()));ui.add_space(12.);
            if self.content.loading{ui.spinner();ui.label("Inspecting page…");return;}
            let Some(info)=&self.content.inspection else{ui.label("Open a PDF to begin.");return};
            if !editable{ui.label(RichText::new(if self.content.busy{"Applying changes…"}else{"This document is read-only."}).color(theme::muted()));}
            {
                ui.horizontal(|ui|{if ui.selectable_value(&mut self.content.image_mode,false,"Text").changed()|ui.selectable_value(&mut self.content.image_mode,true,"Images").changed(){self.content.selected=None;}});ui.add_space(8.);
                if self.content.image_mode{
                    match &info.images{Err(e)=>{ui.label(e);},Ok(images)=>{
                        ui.label(RichText::new("Select an image here or on the page.").small().color(theme::muted()));
                        egui::ScrollArea::vertical().id_salt("image_objects").max_height(205.).show(ui,|ui|{for (i,image) in images.iter().enumerate(){if ui.selectable_label(self.content.selected==Some(i),format!("{}  ·  {} × {} px",image.name,image.pixels[0],image.pixels[1])).clicked(){self.content.selected=Some(i);self.content.rect=[image.rect[0],image.rect[1],image.rect[2]-image.rect[0],image.rect[3]-image.rect[1]];}}});
                        if images.is_empty(){ui.label("No directly editable image objects on this page.");}
                        if let Some(image)=self.content.selected.and_then(|i|images.get(i)){
                            ui.separator();ui.label("Position & size · points");
                            ui.add_enabled_ui(editable,|ui|{egui::Grid::new("image_dimensions").num_columns(2).spacing([12.,10.]).show(ui,|ui|{for(label,v)in ["X","Y","Width","Height"].into_iter().zip(&mut self.content.rect){ui.label(label);ui.add(egui::DragValue::new(v).speed(1.).range(-99_999.0..=99_999.0));ui.end_row();}});ui.add_space(8.);
                                if ui.add(ActionButton::new(Icon::Select,"Apply position & size").primary()).clicked(){let [x,y,w,h]=self.content.rect;change=Some(Edit::Image{page:page as u32+1,index:image.index,edit:content::ImageEdit::Place([x,y,x+w,y+h])});}
                                if ui.add(ActionButton::new(Icon::Import,"Replace image…")).clicked(){replace=Some(image.index);}
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
                    "New blank A4 PDF • Add text, artwork or pages • Save a Copy when ready".into(),
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
            tab.remember_open = false;
            let wake = ctx.clone();
            tab.worker = Worker::from_bytes(source, move || wake.request_repaint());
        }
    }
    pub(super) fn content_tools(&mut self, ctx: &egui::Context) {
        if self.editor.mode != editing::Mode::Edit {
            return;
        }
        if self.content.add_target.is_none() && !self.content.image_mode {
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
                            self.content.new_text.clear();
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
        let fonts = text_style::fonts();
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
            ui.add_enabled_ui(editable,|ui| {
                ui::caption(ui, "Font");
                if let Some(current)=fonts.get(self.content.new_font) {
                    egui::ComboBox::from_id_salt("text_font_family").width(ui.available_width()).selected_text(&current.family).show_ui(ui,|ui| {
                        for (index,font) in fonts.iter().enumerate().filter(|(_,f)|!f.bold&&!f.italic) {
                            if ui.selectable_label(font.family==current.family,&font.family).clicked() {
                                self.content.new_font=fonts.iter().position(|f|f.family==font.family && f.bold==current.bold && f.italic==current.italic).unwrap_or(index);
                            }
                        }
                    });
                    let current=&fonts[self.content.new_font];
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("text_font_style").width((ui.available_width()-86.).max(90.)).selected_text(current.style()).show_ui(ui,|ui| {
                            for (index,font) in fonts.iter().enumerate().filter(|(_,f)|f.family==current.family) { ui.selectable_value(&mut self.content.new_font,index,font.style()); }
                        });
                        ui.add(egui::DragValue::new(&mut self.content.new_size).range(4.0..=144.0).suffix(" pt")).on_hover_text("Font size");
                    });
                    ui.horizontal(|ui| {
                        let current=&fonts[self.content.new_font];
                        for (icon,label,bold,italic,selected) in [(Icon::Bold,"Bold",!current.bold,current.italic,current.bold),(Icon::Italic,"Italic",current.bold,!current.italic,current.italic)] {
                            let target=fonts.iter().position(|f|f.family==current.family && f.bold==bold && f.italic==italic);
                            if ui.add_enabled(target.is_some(),ActionButton::new(icon,label).compact().selected(selected)).clicked() { self.content.new_font=target.unwrap(); }
                        }
                    });
                }
                ui.horizontal(|ui| {
                    for (value,icon,label) in [(Alignment::Left,Icon::AlignLeft,"Align left"),(Alignment::Center,Icon::AlignCenter,"Align center"),(Alignment::Justified,Icon::AlignJustify,"Justify"),(Alignment::Right,Icon::AlignRight,"Align right")] {
                        if ui.add(ActionButton::new(icon,label).compact().selected(self.content.new_alignment==value)).clicked() { self.content.new_alignment=value; }
                    }
                });
                ui.add_space(8.); ui::caption(ui, "Color");
                editing::color_palette(ui,&mut self.content.new_color);
                ui.add_space(8.);
                if self.content.new_placed {
                    egui::CollapsingHeader::new("Position & size").show(ui,|ui| {
                        egui::Grid::new("new_text_dimensions").show(ui,|ui| {
                            for (name,v) in ["Left","Top","Width","Height"].into_iter().zip(&mut self.content.new_rect) {
                                let label=ui.label(name); ui.add(egui::DragValue::new(v).speed(1.).range(0.0..=14400.)).labelled_by(label.id); ui.end_row();
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!self.content.new_text.trim().is_empty(),ActionButton::new(Icon::Check,"Done").primary()).clicked() { self.content.new_commit=true; }
                        if ui.add(ActionButton::new(Icon::Left,"Cancel")).clicked() { self.content.new_text.clear(); self.content.new_placed=false; self.content.next_text=None; }
                    });
                }
            });
            ui.add_space(8.);
            ui.label(RichText::new(if self.content.new_placed { "Type on the page. Drag the top handle to move; drag the circles to resize. Click elsewhere on the page or press ⌘Return to finish." } else { "Click anywhere on a page and start typing." }).size(12.).color(theme::muted()));
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
        let edit = Edit::AddText {
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
            ui.heading("Convert");ui.label(RichText::new("Local files. Fresh output copies.").small().color(theme::muted()));ui.add_space(16.);
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
                ui.add_space(18.);ui.separator();ui.add_space(12.);ui.label(RichText::new("Flatten").family(ui::semibold()));
                ui.label(RichText::new("Turn every page into an image so nothing on it can be edited. Annotations and form fields are burned in; the result can still be signed.").small().color(theme::muted()));
                if ui.add_enabled(ready,ActionButton::new(Icon::Document,"Flatten PDF…")).clicked(){action="flatten";}
                ui.add_space(18.);ui.separator();ui.add_space(12.);ui.label(RichText::new("Reduce file size").family(ui::semibold()));
                ui.label(RichText::new("Make pictures and scans smaller for email or upload. Text and drawings stay sharp.").small().color(theme::muted()));
                if ui.add_enabled(ready,ActionButton::new(Icon::Extract,"Reduce File Size…")).clicked(){action="shrink";}
                ui.add_space(16.);ui.label(RichText::new("Word and Markdown conversions use document text and basic paragraphs. Fonts, columns, tables, headers and page layout are not reconstructed. Word import accepts text documents; drawings are reported. No OCR yet.").small().color(theme::muted()));
            });if self.content.busy{ui.spinner();ui.label("Converting…");}
        });
        match action {
            "shrink" => self.request_shrink(),
            "blank" => self.new_blank(ctx),
            "images" | "document" => self.import_conversion(ctx, action == "images"),
            "export" => self.export_conversion(ctx),
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
            self.content.new_text.clear();
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
                                    alignment: Alignment::Left,
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
                    self.status = Some((
                        "Text updated • Save a Copy to keep your changes".into(),
                        false,
                    ));
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
                AfterText::Save(id) => {
                    if let Some(index) = self.tabs.iter().position(|t| t.id == id) {
                        self.active = index;
                        self.save_dialog(ctx);
                    }
                }
                AfterText::Close(id) => self.request_close(id, ctx),
            }
        }
    }
    fn existing_text_panel(&mut self, ctx: &egui::Context) {
        let current = self.tabs.get(self.active).map(|t| (t.id, t.page));
        egui::SidePanel::right("content_inspector").exact_width(290.).resizable(false)
            .frame(egui::Frame::new().fill(theme::paper()).inner_margin(18.))
            .show(ctx, |ui| {
                ui.heading("Text");
                ui.add_space(16.);
                if let Some(draft) = &mut self.content.existing && current == Some((draft.key.0, draft.key.2)) {
                    ui.label(RichText::new("Editing on the page").family(ui::semibold()));
                    ui.add_space(10.);
                    if let InlineTarget::Block(block) = &mut draft.target {
                        ui.label(RichText::new(format!("{} text objects selected", block.selection.runs.len()+block.selection.annotations.len())).color(theme::muted()));
                        ui.add_space(10.);
                        ui.add_enabled_ui(!self.content.busy, |ui| {
                            let fonts = text_style::fonts();
                            let name = fonts.get(block.font).map(|f|format!("{} · {}",f.family,f.style())).unwrap_or_else(||"Default".into());
                            egui::ComboBox::from_id_salt("block_font").width(ui.available_width()).selected_text(name).show_ui(ui, |ui| {
                                for (index,font) in fonts.iter().enumerate() {
                                    ui.selectable_value(&mut block.font,index,format!("{} · {}",font.family,font.style()));
                                }
                            });
                            ui.add(egui::DragValue::new(&mut draft.size).range(4.0..=144.0).suffix(" pt")).on_hover_text("Block font size");
                            editing::color_palette(ui, &mut draft.color);
                            ui.add_space(8.);
                            ui.label(RichText::new("The selection becomes one block with this font and color. Drag the corner to resize.").small().color(theme::muted()));
                        });
                    } else {
                        ui.label(RichText::new(&draft.font).color(theme::muted()));
                        ui.label(RichText::new(format!("{:.1} pt", draft.size)).color(theme::muted()));
                    }
                    ui.add_space(18.);
                    ui.add_enabled_ui(!self.content.busy, |ui| {
                        if ui.add(ActionButton::new(Icon::Check,"Done").primary()).clicked() { self.content.new_commit = true; }
                        if ui.add(ActionButton::new(Icon::Left,"Cancel")).clicked() {
                            self.content.existing = None;
                            self.content.next_existing = None;
                            self.content.after_text = None;
                            self.content.add_after_existing = None;
                        }
                    });
                    ui.add_space(14.);
                    ui.label(RichText::new("Type directly in the text area on the page. Click outside or press ⌘Return to finish. Esc cancels.").size(13.).color(theme::muted()));
                } else {
                    ui.label(RichText::new(if self.content.area_mode { "Drag a rectangle around the text you want to edit together." } else { "Click text on the page to edit it." }).size(15.).color(theme::ink()));
                    ui.add_space(12.);
                    ui.label(RichText::new(if self.content.area_mode { "Included text highlights as you drag. Release to open a multiline text area. Esc cancels." } else { "Drag across the page, or use Select area, to edit several lines together. Add text starts a new block." }).color(theme::muted()));
                    if let Some(hint) = &self.content.selection_hint { ui.add_space(12.); ui.label(RichText::new(hint).color(theme::muted())); }
                    if self.content.loading { ui.add_space(12.); ui.spinner(); }
                    if let Some(info) = &self.content.inspection && let Err(e) = &info.text {
                        ui.add_space(12.); ui.label(RichText::new(e).small().color(theme::muted()));
                    }
                }
                if self.content.busy { ui.spinner(); ui.label("Applying text…"); }
            });
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
        .or_else(|| text_style::fonts().first())
        .map(|f| egui::FontFamily::Name(f.ui_name().into()))
        .unwrap_or(egui::FontFamily::Proportional)
}

fn selected_block(
    mut items: Vec<InlineText>,
    area: [f32; 4],
    page_size: PageSize,
) -> Option<InlineText> {
    if items.is_empty() {
        return None;
    }
    items.sort_by(|a, b| {
        a.rect[1]
            .total_cmp(&b.rect[1])
            .then(a.rect[0].total_cmp(&b.rect[0]))
    });
    // Reassemble fragments from a visual line, then order lines top to bottom.
    let mut lines: Vec<Vec<InlineText>> = Vec::new();
    for item in items {
        if let Some(line) = lines.last_mut()
            && line.iter().any(|a| {
                let overlap = a.rect[3].min(item.rect[3]) - a.rect[1].max(item.rect[1]);
                overlap > (a.rect[3] - a.rect[1]).min(item.rect[3] - item.rect[1]) * 0.5
            })
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
    for line in lines {
        let mut text = String::new();
        let mut previous: Option<&InlineText> = None;
        for item in &line {
            if let Some(prev) = previous
                && !text.ends_with(char::is_whitespace)
                && !item.text.starts_with(char::is_whitespace)
                && item.rect[0] - prev.rect[2] > first.size * 0.12
            {
                text.push(' ');
            }
            text.push_str(&item.text);
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
        .max(y + (text.lines().count() + 1) as f32 * size * 1.3)
        .min(page_size.height);
    Some(InlineText {
        key: first.key,
        target: InlineTarget::Block(BlockDraft {
            selection,
            masks,
            font,
            initial: (font, size, first.color, right),
        }),
        original: text.clone(),
        text,
        rect: [x, y, right, bottom],
        size,
        color: first.color,
        font: first.font,
        focus: true,
    })
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
    for a in session.annotations.iter().filter(|a| a.page == page) {
        if let mypdf_milestone0::annotations::Kind::Text { text } = &a.kind {
            candidates.push(InlineText {
                key,
                target: InlineTarget::Annotation(a.id.clone()),
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
                        state.selection_hint = Some("No editable text in that area. Try a larger rectangle; scanned text needs OCR.".into());
                    }
                }
                state.marquee = None;
                ui.ctx().request_repaint();
            }
        }
        // Render the new text area on the next frame, after releasing the drag.
        return;
    }
    let block_active = state
        .existing
        .as_ref()
        .is_some_and(|e| matches!(e.target, InlineTarget::Block(_)));
    let mut clicked = None;
    for (i, draft) in candidates.into_iter().enumerate() {
        if block_active {
            break;
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
        if response.hovered() {
            painter.rect_stroke(
                r,
                2.,
                Stroke::new(1., theme::accent()),
                egui::StrokeKind::Outside,
            );
        }
        if response.clicked() && !state.busy {
            clicked = Some(draft);
        }
    }
    if let Some(draft) = clicked {
        if state.existing.is_some() {
            state.new_commit = true;
            state.next_existing = Some(draft);
        } else {
            state.existing = Some(draft);
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
    ui::dashed_rect(&painter, outline, theme::accent());
    if !block {
        for center in [outline.left_center(), outline.right_center()] {
            painter.circle_filled(center, 4.5, Color32::WHITE);
            painter.circle_stroke(center, 4.5, Stroke::new(1.5, theme::accent()));
        }
    }
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.set_clip_rect(page_rect.intersect(ui.clip_rect()));
        ui.add_enabled_ui(!state.busy, |ui| {
            let font = FontId::new((draft.size * scale).clamp(4., 800.), family);
            let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, width: f32| {
                let mut job = egui::text::LayoutJob::simple(
                    text.as_str().to_owned(),
                    font.clone(),
                    color,
                    width,
                );
                if block {
                    for section in &mut job.sections {
                        section.format.line_height = Some(draft.size * scale * 1.3);
                    }
                }
                ui.fonts_mut(|f| f.layout_job(job))
            };
            let edit = if annotation || block {
                egui::TextEdit::multiline(&mut draft.text).margin(ui::FIELD_MARGIN)
            } else {
                egui::TextEdit::singleline(&mut draft.text).margin(ui::FIELD_MARGIN)
            };
            let output = edit
                .id(egui::Id::new((tab.id, page, "inline_existing_text")))
                .font(font.clone())
                .text_color(color)
                .frame(false)
                .margin(0.)
                .desired_width(rect.width())
                .min_size(rect.size())
                .char_limit(32768)
                .layouter(&mut layouter)
                .show(ui);
            let response = output.response;
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
    if block && !state.busy {
        let handle = Rect::from_center_size(rect.max, vec2(16., 16.));
        painter.circle_filled(rect.max, 5., Color32::WHITE);
        painter.circle_stroke(rect.max, 5., Stroke::new(1.5, theme::accent()));
        let response = ui
            .interact(
                handle,
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
                state.new_text.clear();
                state.new_placed = false;
                state.next_text = None;
                return;
            }
            let [x, y, w, h] = state.new_rect;
            let r = Rect::from_min_size(page_rect.min + vec2(x, y) * scale, vec2(w, h) * scale);
            let painter = ui
                .painter()
                .with_clip_rect(page_rect.intersect(ui.clip_rect()));
            ui::dashed_rect(&painter, r.expand(2.), theme::accent());
            let move_rect = Rect::from_center_size(r.center_top() - vec2(0., 9.), vec2(40., 12.));
            painter.rect_filled(move_rect, 6., Color32::WHITE);
            painter.rect_stroke(
                move_rect,
                6.,
                Stroke::new(1., theme::accent()),
                egui::StrokeKind::Inside,
            );
            painter.line_segment(
                [
                    move_rect.center() - vec2(10., 0.),
                    move_rect.center() + vec2(10., 0.),
                ],
                Stroke::new(2., theme::accent()),
            );
            let moving = ui
                .interact(
                    move_rect,
                    egui::Id::new((tab.id, page, "move_text_block")),
                    egui::Sense::drag(),
                )
                .on_hover_text("Move text box")
                .on_hover_cursor(egui::CursorIcon::Move);
            let p = &tab.pages[page];
            if moving.drag_started() {
                state.new_drag = Some(state.new_rect);
            }
            if moving.dragged()
                && let Some(original) = state.new_drag
            {
                let d = moving.total_drag_delta().unwrap_or_default() / scale;
                state.new_rect[0] = (original[0] + d.x).clamp(0., (p.width - w).max(0.));
                state.new_rect[1] = (original[1] + d.y).clamp(0., (p.height - h).max(0.));
            }
            let rgb = state.new_color;
            let color = Color32::from_rgb(
                (rgb[0] * 255.) as u8,
                (rgb[1] * 255.) as u8,
                (rgb[2] * 255.) as u8,
            );
            let font = state.text_font(state.new_size * scale);
            let align = match state.new_alignment {
                Alignment::Center => egui::Align::Center,
                Alignment::Right => egui::Align::RIGHT,
                _ => egui::Align::LEFT,
            };
            let justify = state.new_alignment == Alignment::Justified;
            let line_height = state.new_size * scale * 1.3;
            let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, width: f32| {
                let mut job = egui::text::LayoutJob::simple(
                    text.as_str().to_owned(),
                    font.clone(),
                    color,
                    width,
                );
                job.halign = align;
                job.justify = justify;
                for section in &mut job.sections {
                    section.format.line_height = Some(line_height);
                }
                ui.fonts_mut(|f| f.layout_job(job))
            };
            ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                ui.set_clip_rect(r.intersect(page_rect).intersect(ui.clip_rect()));
                let edit = egui::TextEdit::multiline(&mut state.new_text)
                    .margin(ui::FIELD_MARGIN)
                    .id(egui::Id::new((tab.id, page, "inline_new_text")))
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
    for (i, r, selected) in boxes {
        let r = Rect::from_min_max(
            page_rect.min + vec2(r[0], r[1]) * scale,
            page_rect.min + vec2(r[2], r[3]) * scale,
        );
        painter.rect_filled(
            r,
            2.,
            Color32::from_rgba_unmultiplied(39, 126, 216, if selected { 28 } else { 10 }),
        );
        painter.rect_stroke(
            r,
            2.,
            Stroke::new(
                if selected { 2. } else { 1. },
                if selected {
                    theme::accent()
                } else {
                    Color32::from_rgb(147, 183, 220)
                },
            ),
            egui::StrokeKind::Inside,
        );
        if ui
            .interact(
                r.intersect(page_rect),
                egui::Id::new((tab.id, page, "content", i)),
                egui::Sense::click(),
            )
            .clicked()
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
        click(&mut app, page.min + vec2(-16., 300.));
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
                egui::Event::Text("Changed: İstanbul!".into()),
            ],
        );
        assert_eq!(
            app.content.existing.as_ref().unwrap().text,
            "Changed: İstanbul!"
        );
        assert!(ctx.memory(|m| m.has_focus(egui::Id::new((id, 0_usize, "inline_existing_text")))));
        frame(
            &mut app,
            vec![key(egui::Key::Enter, egui::Modifiers::COMMAND)],
        );
        wait(&mut app, |a| {
            a.content.existing.is_none() && !a.content.loading && !a.content.busy
        });
        let runs = document::text_runs(&app.tabs[0].session.as_ref().unwrap().source, 1).unwrap();
        assert_eq!(runs[0].text, "Repeated");
        assert_eq!(runs[1].text, "Changed: İstanbul!");
        let r = ctx
            .read_response(egui::Id::new((id, 0_usize, "existing_text", 2_usize)))
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
                .is_some_and(|e| e.target == InlineTarget::Run(0)),
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
                    .is_some_and(|e| e.original == "Changed: İstanbul!")
        });
        frame(
            &mut app,
            vec![
                key(egui::Key::A, egui::Modifiers::COMMAND),
                egui::Event::Text("Changed: İstanbul!".into()),
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
        assert!(
            doc.extract_text(&[1])
                .unwrap()
                .contains("Changed: İstanbul!")
        );
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
