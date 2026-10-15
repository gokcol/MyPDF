//! Native thumbnail organizer and bounded background page transactions.
use super::*;
use mypdf_milestone0::pages::{self, Operation};
use std::collections::BTreeSet;

type Change = Result<(u64, u64, Vec<u8>), String>;
/// Pages copied from one document, ready to paste into any open document.
pub(super) struct PageClip {
    bytes: Vec<u8>,
    pages: usize,
    from: String,
}
pub(super) struct Organizer {
    tab: u64,
    pub selected: BTreeSet<usize>,
    range: String,
    before: usize,
    split_every: usize,
    pub crop: bool,
    margins: [f32; 4],
    dragged: Option<usize>,
    pressed: Option<(usize, egui::Pos2)>,
    pub busy: bool,
    tx: mpsc::Sender<Change>,
    rx: mpsc::Receiver<Change>,
    output_tx: mpsc::Sender<Result<String, String>>,
    output_rx: mpsc::Receiver<Result<String, String>>,
    pub clipboard: Option<Arc<PageClip>>,
    clip_tx: mpsc::Sender<Result<PageClip, String>>,
    clip_rx: mpsc::Receiver<Result<PageClip, String>>,
    /// Pages to select once a paste has reloaded: tab, page count, pages.
    pending_select: Option<(u64, usize, BTreeSet<usize>)>,
}
impl Organizer {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let (output_tx, output_rx) = mpsc::channel();
        let (clip_tx, clip_rx) = mpsc::channel();
        Self {
            tab: 0,
            selected: BTreeSet::new(),
            range: "1".into(),
            before: 1,
            split_every: 1,
            crop: false,
            margins: [18.0; 4],
            dragged: None,
            pressed: None,
            busy: false,
            tx,
            rx,
            output_tx,
            output_rx,
            clipboard: None,
            clip_tx,
            clip_rx,
            pending_select: None,
        }
    }
}
impl Tab {
    /// Rebuild rendering/search only when an undo or page edit changes the PDF source.
    pub(super) fn refresh_document(&mut self, ctx: &egui::Context) {
        let Some(source) = self.session.as_ref().map(|s| s.source.clone()) else {
            return;
        };
        if self.bytes.as_ref().is_some_and(|b| Arc::ptr_eq(b, &source)) {
            return;
        }
        let wake = ctx.clone();
        self.worker = Worker::from_bytes(source.clone(), move || wake.request_repaint());
        self.bytes = Some(source);
        // After a content edit, each page keeps its sharpest old image until redrawn.
        self.stale.clear();
        if std::mem::take(&mut self.keep_stale) {
            for (key, raster) in self.rasters.drain() {
                let sharper = self
                    .stale
                    .get(&key.page)
                    .is_none_or(|old| old.texture.size()[0] < raster.texture.size()[0]);
                if !key.thumbnail && sharper {
                    self.stale.insert(key.page, raster);
                }
            }
        }
        self.rasters.clear();
        self.pending.clear();
        self.render_errors.clear();
        self.text.clear();
        self.hits.clear();
        self.search_worker.invalidate();
        self.indexed = 0;
        self.index_done = false;
        self.search_dirty = true;
        self.searching = false;
        self.selected_annotation = None;
        self.group.clear();
        self.error = None;
        if let Some(session) = &mut self.session {
            session.editable = false;
        }
    }
}
impl Viewer {
    pub(super) fn page_events(&mut self, ctx: &egui::Context) {
        for result in self.organizer.rx.try_iter() {
            self.organizer.busy = false;
            match result {
                Ok((id, revision, bytes)) => {
                    let active = self.tabs.get(self.active).is_some_and(|t| t.id == id);
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id)
                        && let Some(s) = &mut tab.session
                    {
                        if s.revision() != revision {
                            self.status=Some(("The document changed while pages were processing. Please try again.".into(),true));
                            continue;
                        }
                        match s.commit_document(bytes) {
                            Ok(()) => {
                                if active {
                                    self.editor.draft = None;
                                }
                                if self.organizer.tab == id {
                                    self.organizer.selected.clear();
                                }
                                self.status=Some(("Pages updated • Undo is available • Save a Copy to keep your changes".into(),false));
                            }
                            Err(e) => self.status = Some((format!("{e:#}"), true)),
                        }
                    }
                }
                Err(e) => self.status = Some((e, true)),
            }
        }
        for result in self.organizer.clip_rx.try_iter() {
            match result {
                Ok(clip) => {
                    self.status = Some((
                        format!(
                            "Copied {} • Open another PDF in Pages and press ⌘V (or Paste pages) to insert after the selected page",
                            plural(clip.pages, "page")
                        ),
                        false,
                    ));
                    ctx.copy_text(format!("{} from {}", plural(clip.pages, "page"), clip.from));
                    self.organizer.clipboard = Some(Arc::new(clip));
                }
                Err(e) => self.status = Some((e, true)),
            }
        }
        for result in self.organizer.output_rx.try_iter() {
            self.organizer.busy = false;
            self.status = Some(match result {
                Ok(s) => (s, false),
                Err(s) => (s, true),
            });
        }
        for tab in &mut self.tabs {
            tab.refresh_document(ctx);
        }
    }
    pub(super) fn page_change(&mut self, ctx: &egui::Context, operation: Operation) {
        if self.organizer.busy || self.content.busy || !self.apply_pending_annotation() {
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let Some(s) = &tab.session else { return };
        if !s.editable {
            self.status = Some(("This PDF is read-only for page changes.".into(), true));
            return;
        }
        let (id, revision, source, annotations) = (
            tab.id,
            s.revision(),
            s.source.clone(),
            s.annotations.clone(),
        );
        let tx = self.organizer.tx.clone();
        let wake = ctx.clone();
        self.organizer.busy = true;
        self.status = Some(("Updating pages…".into(), false));
        std::thread::spawn(move || {
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> anyhow::Result<_> {
                    let current = if revision == 0 {
                        source.to_vec()
                    } else {
                        mypdf_milestone0::annotations::export(&source, &annotations)?
                    };
                    Ok((id, revision, pages::apply(&current, &operation)?))
                }))
                .map_err(|_| {
                    "This PDF could not be processed. Your document is unchanged.".to_string()
                })
                .and_then(|r| r.map_err(|e| format!("{e:#}")));
            let _ = tx.send(result);
            wake.request_repaint();
        });
    }
    pub(super) fn page_tools(&mut self, ctx: &egui::Context) {
        if self.editor.mode != editing::Mode::Pages {
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        if self.organizer.tab != tab.id {
            self.organizer.tab = tab.id;
            self.organizer.pressed = None;
            self.organizer.dragged = None;
            self.organizer.selected = BTreeSet::from([tab.page]);
            self.organizer.range = (tab.page + 1).to_string();
            self.organizer.crop = false;
        }
        if let Some((id, count, pages)) = &self.organizer.pending_select
            && *id == tab.id
            && *count == tab.pages.len()
        {
            self.organizer.selected = pages.clone();
            self.organizer.pending_select = None;
        }
        self.organizer.selected.retain(|i| *i < tab.pages.len());
        if self.organizer.selected.is_empty() && !tab.pages.is_empty() {
            self.organizer
                .selected
                .insert(tab.page.min(tab.pages.len() - 1));
        }
        let count = tab.pages.len();
        let editable = tab.session.as_ref().is_some_and(|s| s.editable)
            && count > 0
            && !self.organizer.busy
            && !self.content.busy;
        let mut action = "";
        egui::TopBottomPanel::top("page_tools")
            .frame(
                egui::Frame::new()
                    .fill(theme::surface())
                    .inner_margin(egui::Margin::symmetric(14, 6)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::horizontal().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                count > 0 && !self.organizer.selected.is_empty(),
                                ActionButton::new(Icon::Copy, "Copy pages"),
                            )
                            .on_hover_text(
                                "Copy the selected pages (⌘C) to paste into any open PDF",
                            )
                            .clicked()
                        {
                            action = "copy";
                        }
                        if ui
                            .add_enabled(
                                editable && self.organizer.clipboard.is_some(),
                                ActionButton::new(Icon::Import, "Paste pages"),
                            )
                            .on_hover_text(match &self.organizer.clipboard {
                                Some(clip) => format!(
                                    "Insert {} from {} after the selected page (⌘V)",
                                    plural(clip.pages, "page"),
                                    clip.from
                                ),
                                None => "Copy pages first".into(),
                            })
                            .clicked()
                        {
                            action = "paste";
                        }
                        ui.separator();
                        ui.add_enabled_ui(editable, |ui| {
                            for (icon, label, key) in [
                                (Icon::Plus, "Blank page", "blank"),
                                (Icon::Import, "Insert…", "insert"),
                                (Icon::Import, "Merge…", "merge"),
                                (Icon::Redo, "Rotate right", "right"),
                                (Icon::Undo, "Rotate left", "left"),
                                (Icon::Crop, "Crop", "crop"),
                                (Icon::Extract, "Extract…", "extract"),
                                (Icon::Split, "Split…", "split"),
                                (Icon::Trash, "Delete", "delete"),
                            ] {
                                if ui
                                    .add(
                                        ActionButton::new(icon, label)
                                            .selected(key == "crop" && self.organizer.crop),
                                    )
                                    .clicked()
                                {
                                    action = key;
                                }
                            }
                        });
                        if self.organizer.busy {
                            ui.spinner();
                        }
                    })
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} pages · {} selected",
                            count,
                            self.organizer.selected.len()
                        ))
                        .color(theme::muted())
                        .small(),
                    );
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.organizer.range)
                            .margin(ui::FIELD_MARGIN)
                            .hint_text("1, 3-5")
                            .desired_width(95.0),
                    );
                    self.editor.text_editing |= response.has_focus();
                    if ui.small_button("Select range").clicked() {
                        match pages::parse_range(&self.organizer.range, count) {
                            Ok(p) => self.organizer.selected = p.into_iter().collect(),
                            Err(e) => self.status = Some((e.to_string(), true)),
                        }
                    }
                    if ui.small_button("Select all").clicked() {
                        self.organizer.selected = (0..count).collect();
                    }
                    ui.separator();
                    ui.label("Move before page");
                    let r = ui
                        .add(egui::DragValue::new(&mut self.organizer.before).range(1..=count + 1));
                    self.editor.text_editing |= r.has_focus();
                    if ui
                        .add_enabled(editable, egui::Button::new("Move"))
                        .clicked()
                    {
                        action = "move";
                    }
                    ui.label(
                        RichText::new(
                            "Drag to reorder · ⌘/Ctrl-click to select · double-click to read",
                        )
                        .small()
                        .color(theme::muted()),
                    );
                });
            });
        let selected: Vec<_> = self.organizer.selected.iter().copied().collect();
        match action {
            "undo" | "redo" => self.editor_undo(action == "redo"),
            "right" | "left" => self.page_change(
                ctx,
                Operation::Rotate {
                    pages: selected,
                    clockwise: action == "right",
                },
            ),
            "delete" => self.page_change(ctx, Operation::Delete(selected)),
            "move" => self.move_pages(ctx, self.organizer.before.saturating_sub(1)),
            "crop" => self.organizer.crop = !self.organizer.crop,
            "blank" => {
                let at = selected.first().copied().unwrap_or(count.saturating_sub(1)) + 1;
                let size = self.tabs[self.active].pages[at.saturating_sub(1)];
                self.page_change(
                    ctx,
                    Operation::InsertBlank {
                        at,
                        width: size.width,
                        height: size.height,
                    },
                );
            }
            "insert" => self.insert_dialog(ctx),
            "copy" => self.copy_pages(ctx),
            "paste" => self.paste_pages(ctx),
            "merge" => self.import_pages(ctx, count),
            "extract" => self.export_pages(ctx, false),
            "split" => self.export_pages(ctx, true),
            _ => {}
        }
    }
    /// Copy the selected pages, with their annotations, to the app's page clipboard.
    pub(super) fn copy_pages(&mut self, ctx: &egui::Context) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let Some(s) = &tab.session else { return };
        let selected: Vec<usize> = if self.organizer.tab == tab.id {
            self.organizer.selected.iter().copied().collect()
        } else {
            vec![tab.page]
        };
        if selected.is_empty() {
            return;
        }
        let (source, annotations, revision) =
            (s.source.clone(), s.annotations.clone(), s.revision());
        let from = tab.name();
        let tx = self.organizer.clip_tx.clone();
        let wake = ctx.clone();
        self.status = Some(("Copying pages…".into(), false));
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || -> anyhow::Result<PageClip> {
                    let current = if revision == 0 {
                        source.to_vec()
                    } else {
                        mypdf_milestone0::annotations::export(&source, &annotations)?
                    };
                    Ok(PageClip {
                        bytes: pages::extract(&current, &selected)?,
                        pages: selected.len(),
                        from,
                    })
                },
            ))
            .map_err(|_| "These pages could not be copied.".to_string())
            .and_then(|r| r.map_err(|e| format!("{e:#}")));
            let _ = tx.send(result);
            wake.request_repaint();
        });
    }
    /// Insert the copied pages after the selected page (or at the end).
    pub(super) fn paste_pages(&mut self, ctx: &egui::Context) {
        let Some(clip) = self.organizer.clipboard.clone() else {
            self.status = Some((
                "Copy pages first: select pages in Pages and press ⌘C.".into(),
                false,
            ));
            return;
        };
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let count = tab.pages.len();
        let at = if self.organizer.tab == tab.id {
            self.organizer.selected.last().map_or(count, |p| p + 1)
        } else {
            count
        };
        let id = tab.id;
        self.page_change(
            ctx,
            Operation::InsertPdf {
                at,
                bytes: clip.bytes.clone(),
            },
        );
        if self.organizer.busy {
            self.organizer.pending_select =
                Some((id, count + clip.pages, (at..at + clip.pages).collect()));
            self.status = Some((
                format!("Pasting {} from {}…", plural(clip.pages, "page"), clip.from),
                false,
            ));
        }
    }
    fn move_pages(&mut self, ctx: &egui::Context, before: usize) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let selected = &self.organizer.selected;
        let mut order: Vec<_> = (0..tab.pages.len())
            .filter(|i| !selected.contains(i))
            .collect();
        let at = (0..before.min(tab.pages.len()))
            .filter(|i| !selected.contains(i))
            .count();
        order.splice(at..at, selected.iter().copied());
        self.page_change(ctx, Operation::Reorder(order));
    }
    fn import_pages(&mut self, ctx: &egui::Context, at: usize) {
        let Some(paths) = self.native_picks(
            rfd::FileDialog::new()
                .set_title("Insert PDFs in selected order")
                .add_filter("PDF", &["pdf"]),
        ) else {
            return;
        };
        if self.organizer.busy || self.content.busy || !self.apply_pending_annotation() {
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let Some(s) = &tab.session else { return };
        let (id, revision, source, annotations) = (
            tab.id,
            s.revision(),
            s.source.clone(),
            s.annotations.clone(),
        );
        let tx = self.organizer.tx.clone();
        let wake = ctx.clone();
        self.organizer.busy = true;
        self.status = Some(("Importing pages…".into(), false));
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<_> {
                use std::io::Read;
                let mut current = if revision == 0 {
                    source.to_vec()
                } else {
                    mypdf_milestone0::annotations::export(&source, &annotations)?
                };
                let mut at = at;
                for path in paths {
                    let mut bytes = vec![];
                    std::fs::File::open(path)?
                        .take(viewer::MAX_FILE_BYTES + 1)
                        .read_to_end(&mut bytes)?;
                    let before = lopdf::Document::load_mem(&current)?.get_pages().len();
                    current = pages::apply(&current, &Operation::InsertPdf { at, bytes })?;
                    at += lopdf::Document::load_mem(&current)?.get_pages().len() - before;
                }
                Ok((id, revision, current))
            })()
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(result);
            wake.request_repaint();
        });
    }
    fn insert_dialog(&mut self, ctx: &egui::Context) {
        // The insertion controls live in the organizer panel, so the position is reviewable.
        self.organizer.crop = false;
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("insert_pages_open"), true));
    }
    pub(super) fn page_dialogs(&mut self, ctx: &egui::Context) {
        let mut open = ctx
            .data(|d| d.get_temp::<bool>(egui::Id::new("insert_pages_open")))
            .unwrap_or(false);
        if !open {
            return;
        }
        let count = self.tabs.get(self.active).map_or(0, |t| t.pages.len());
        let mut action = "";
        egui::Window::new("Insert pages").open(&mut open).collapsible(false).resizable(false).show(ctx,|ui| {
            ui.horizontal(|ui|{ui.label("Before page");ui.add(egui::DragValue::new(&mut self.organizer.before).range(1..=count+1));ui.label(format!("({} = append)",count+1));});
            ui.add_space(10.0);
            if ui.add(ActionButton::new(Icon::Plus,"Blank page · same size")).clicked(){action="blank";}
            if ui.add(ActionButton::new(Icon::Import,"Pages from PDF…")).clicked(){action="pdf";}
            ui.label(RichText::new("Imported pages keep their contents and annotations.\nDocument-level bookmarks are not merged.").small().color(theme::muted()));
        });
        if !action.is_empty() {
            open = false;
        }
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("insert_pages_open"), open));
        let at = self.organizer.before.saturating_sub(1).min(count);
        if action == "pdf" {
            self.import_pages(ctx, at);
        }
        if action == "blank"
            && let Some(tab) = self.tabs.get(self.active)
            && let Some(size) = tab.pages.get(tab.page)
        {
            self.page_change(
                ctx,
                Operation::InsertBlank {
                    at,
                    width: size.width,
                    height: size.height,
                },
            );
        }
    }
    fn export_pages(&mut self, ctx: &egui::Context, split: bool) {
        if self.organizer.busy || self.content.busy || !self.apply_pending_annotation() {
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let Some(s) = &tab.session else { return };
        let (source, annotations, revision) =
            (s.source.clone(), s.annotations.clone(), s.revision());
        let stem = tab
            .path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let count = tab.pages.len();
        let selected: Vec<_> = self.organizer.selected.iter().copied().collect();
        let destination = if split {
            menu::with_file_dialog(self._native_menu.as_ref(), || {
                rfd::FileDialog::new()
                    .set_title("Choose parent folder for split PDFs")
                    .pick_folder()
            })
        } else {
            self.native_save(
                rfd::FileDialog::new()
                    .set_title("Extract pages to a new PDF")
                    .add_filter("PDF", &["pdf"])
                    .set_file_name(format!("{stem}-extracted.pdf")),
            )
        };
        let Some(destination) = destination else {
            return;
        };
        let every = self.organizer.split_every.max(1);
        let tx = self.organizer.output_tx.clone();
        let wake = ctx.clone();
        self.organizer.busy = true;
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<String> {
                let current = if revision == 0 {
                    source.to_vec()
                } else {
                    mypdf_milestone0::annotations::export(&source, &annotations)?
                };
                if split {
                    // Stage an entire new directory; existing files are never touched. Failed staging is removed.
                    let staging = tempfile::Builder::new()
                        .prefix(".mypdf-split-")
                        .tempdir_in(&destination)?;
                    let all: Vec<_> = (0..count).collect();
                    for (i, chunk) in all.chunks(every).enumerate() {
                        mypdf_milestone0::write_new(
                            &staging.path().join(format!("{stem}-{:03}.pdf", i + 1)),
                            &pages::extract(&current, chunk)?,
                        )?;
                    }
                    let folder =
                        destination.join(format!("{stem}-split-{:08x}", rand::random::<u32>()));
                    // Reserve a fresh destination atomically, then move only into that owned empty directory.
                    std::fs::create_dir(&folder)?;
                    for entry in std::fs::read_dir(staging.path())? {
                        let entry = entry?;
                        if let Err(e) =
                            std::fs::rename(entry.path(), folder.join(entry.file_name()))
                        {
                            let _ = std::fs::remove_dir_all(&folder);
                            return Err(e.into());
                        }
                    }
                    Ok(format!(
                        "Saved {} PDFs in {}",
                        count.div_ceil(every),
                        folder.display()
                    ))
                } else {
                    mypdf_milestone0::write_new(
                        &destination,
                        &pages::extract(&current, &selected)?,
                    )?;
                    Ok(format!(
                        "Extracted {} pages to {}",
                        selected.len(),
                        destination.display()
                    ))
                }
            })()
            .map_err(|e| format!("Could not export pages: {e:#}. Existing files are preserved."));
            let _ = tx.send(result);
            wake.request_repaint();
        });
    }
    pub(super) fn organizer_panel(&mut self, ctx: &egui::Context) {
        let mut action = "";
        egui::SidePanel::right("page_properties").exact_width(248.0).frame(egui::Frame::new().fill(theme::paper()).inner_margin(18.0)).show(ctx,|ui|{
            ui.heading(if self.organizer.crop {"Crop pages"}else{"Page tools"});ui.add_space(16.0);
            if self.organizer.crop {
                ui.label(RichText::new("MARGINS · POINTS").small().color(theme::muted()));
                for (label,value) in ["Left","Top","Right","Bottom"].into_iter().zip(&mut self.organizer.margins) {
                    ui.horizontal(|ui|{ui.label(label);let r=ui.add(egui::DragValue::new(value).range(0.0..=100_000.0).speed(1.0).suffix(" pt"));self.editor.text_editing|=r.has_focus();});
                }
                ui.add_space(12.0);
                if ui.add_enabled(!self.organizer.busy,ActionButton::new(Icon::Crop,"Apply crop").primary()).clicked(){action="crop";}
                if ui.add_enabled(!self.organizer.busy,ActionButton::new(Icon::Undo,"Reset to full page")).clicked(){action="reset";}
                ui.add_space(10.0);ui.label(RichText::new("The shaded area will be hidden. Cropping keeps the original content; it is not redaction.").small().color(theme::muted()));
            } else {
                ui::caption(ui, "Split document");
                ui.horizontal(|ui|{ui.label("Every");let r=ui.add(egui::DragValue::new(&mut self.organizer.split_every).range(1..=10_000));self.editor.text_editing|=r.has_focus();ui.label("pages");});
                ui.label(RichText::new("Split saves the whole document into a new folder. Extract saves just the selected pages.").small().color(theme::muted()));
                ui.add_space(20.0);
                ui::tip(ui, "Your original stays intact", "Reorder, rotate, crop and delete are undoable. Use Save a Copy when you are ready.");
            }
        });
        let pages = self.organizer.selected.iter().copied().collect();
        match action {
            "crop" => self.page_change(
                ctx,
                Operation::Crop {
                    pages,
                    margins: self.organizer.margins,
                },
            ),
            "reset" => self.page_change(ctx, Operation::ResetCrop(pages)),
            _ => {}
        }
    }
    pub(super) fn page_overview(&mut self, ctx: &egui::Context) {
        let mut drop_before = None;
        let first_press = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                } => Some(*pos),
                _ => None,
            })
        });
        let pointer = ctx.input(|i| i.pointer.interact_pos());
        let released = ctx.input(|i| i.pointer.any_released());
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::canvas()).inner_margin(24.0))
            .show(ctx, |ui| {
                let Some(tab) = self.tabs.get_mut(self.active) else {
                    ui.label("Open a PDF to organize its pages.");
                    return;
                };
                let columns = ((ui.available_width() + 24.0) / 174.0).floor().max(1.0) as usize;
                let cell = (ui.available_width() - (columns - 1) as f32 * 24.0) / columns as f32;
                let count = tab.pages.len();
                egui::ScrollArea::vertical()
                    .scroll_source(egui::scroll_area::ScrollSource {
                        drag: false,
                        ..Default::default()
                    })
                    .id_salt(("organize", tab.id))
                    .show_rows(ui, 224.0, count.div_ceil(columns), |ui, rows| {
                        for row in rows {
                            ui.horizontal_top(|ui| {
                                ui.spacing_mut().item_spacing.x = 24.0;
                                for i in row * columns..((row + 1) * columns).min(count) {
                                    let size = tab.pages[i];
                                    let scale =
                                        ((cell - 16.0) / size.width).min(174.0 / size.height);
                                    let (area, _) = ui.allocate_exact_size(
                                        vec2(cell, 208.0),
                                        egui::Sense::hover(),
                                    );
                                    let response = ui.interact(
                                        area,
                                        egui::Id::new((tab.id, i, "organizer_page")),
                                        egui::Sense::click_and_drag(),
                                    );
                                    response.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::WidgetType::Button,
                                            true,
                                            format!("Page {}", i + 1),
                                        )
                                    });
                                    let rect = Rect::from_center_size(
                                        pos2(area.center().x, area.top() + 88.0),
                                        vec2(size.width, size.height) * scale,
                                    );
                                    let selected = self.organizer.selected.contains(&i);
                                    ui::shadow(
                                        ui.painter(),
                                        rect,
                                        2,
                                        if response.hovered() { 3 } else { 1 },
                                    );
                                    ui.painter().rect_filled(rect, 1.0, Color32::WHITE);
                                    let key = viewer::raster_key(
                                        i,
                                        size,
                                        scale * ctx.pixels_per_point(),
                                        true,
                                    );
                                    tab.request(key);
                                    if let Some(raster) = tab.rasters.get_mut(&key) {
                                        raster.used = self.tick;
                                        ui.painter().image(
                                            raster.texture.id(),
                                            rect,
                                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                            Color32::WHITE,
                                        );
                                    }
                                    if let Some(s) = &tab.session {
                                        for a in s.annotations.iter().filter(|a| a.page == i) {
                                            editing::paint_annotation(
                                                &mut self.editor,
                                                ctx,
                                                &ui.painter().with_clip_rect(rect),
                                                rect,
                                                scale,
                                                a,
                                            );
                                        }
                                    }
                                    if self.organizer.crop && selected {
                                        let [l, t, r, b] = self.organizer.margins;
                                        let crop = Rect::from_min_max(
                                            rect.min + vec2(l, t) * scale,
                                            rect.max - vec2(r, b) * scale,
                                        )
                                        .intersect(rect);
                                        for shade in [
                                            Rect::from_min_max(
                                                rect.min,
                                                pos2(rect.right(), crop.top()),
                                            ),
                                            Rect::from_min_max(
                                                pos2(rect.left(), crop.bottom()),
                                                rect.max,
                                            ),
                                            Rect::from_min_max(
                                                pos2(rect.left(), crop.top()),
                                                pos2(crop.left(), crop.bottom()),
                                            ),
                                            Rect::from_min_max(
                                                pos2(crop.right(), crop.top()),
                                                pos2(rect.right(), crop.bottom()),
                                            ),
                                        ] {
                                            if shade.is_positive() {
                                                ui.painter().rect_filled(
                                                    shade,
                                                    0.0,
                                                    Color32::from_black_alpha(90),
                                                );
                                            }
                                        }
                                        ui.painter().rect_stroke(
                                            crop,
                                            0.0,
                                            Stroke::new(1.5, theme::accent()),
                                            egui::StrokeKind::Inside,
                                        );
                                    }
                                    if selected {
                                        ui.painter().rect_stroke(
                                            rect.expand(4.0),
                                            6.0,
                                            Stroke::new(2.5, theme::accent()),
                                            egui::StrokeKind::Outside,
                                        );
                                    }
                                    let badge = Rect::from_center_size(
                                        pos2(area.center().x, area.bottom() - 12.0),
                                        vec2(30.0 + 6.0 * (i + 1).to_string().len() as f32, 20.0),
                                    );
                                    if selected {
                                        ui.painter().rect_filled(badge, 10.0, theme::accent());
                                    }
                                    ui.painter().text(
                                        badge.center(),
                                        egui::Align2::CENTER_CENTER,
                                        (i + 1).to_string(),
                                        ui::strong(ui, 12.0),
                                        if selected {
                                            Color32::WHITE
                                        } else {
                                            theme::muted()
                                        },
                                    );
                                    if response.clicked() {
                                        let modifiers = ui.input(|i| i.modifiers);
                                        if modifiers.command {
                                            if !self.organizer.selected.remove(&i) {
                                                self.organizer.selected.insert(i);
                                            }
                                        } else if modifiers.shift {
                                            let start = self
                                                .organizer
                                                .selected
                                                .first()
                                                .copied()
                                                .unwrap_or(i);
                                            self.organizer
                                                .selected
                                                .extend(start.min(i)..=start.max(i));
                                        } else {
                                            self.organizer.selected = BTreeSet::from([i]);
                                        }
                                        tab.page = i;
                                        self.organizer.range = (i + 1).to_string();
                                    }
                                    if response.double_clicked() {
                                        self.editor.mode = editing::Mode::Read;
                                        tab.jump(i);
                                    }
                                    if let Some(p) = first_press.filter(|p| {
                                        area.contains(*p)
                                            && ui.clip_rect().contains(*p)
                                            && ctx.layer_id_at(*p) == Some(ui.layer_id())
                                    }) {
                                        self.organizer.pressed = Some((i, p));
                                    }
                                    if let Some(p) = pointer.filter(|p| {
                                        area.contains(*p)
                                            && ui.clip_rect().contains(*p)
                                            && ctx.layer_id_at(*p) == Some(ui.layer_id())
                                    }) {
                                        let after = p.x > area.center().x;
                                        drop_before = Some(i + usize::from(after));
                                        if self.organizer.dragged.is_some() {
                                            let x = if after {
                                                area.right() + 10.0
                                            } else {
                                                area.left() - 10.0
                                            };
                                            ui.painter().line_segment(
                                                [pos2(x, area.top()), pos2(x, area.bottom())],
                                                Stroke::new(3.0, theme::accent()),
                                            );
                                        }
                                    }
                                }
                            });
                        }
                    });
            });
        if let Some((start, origin)) = self.organizer.pressed
            && pointer.is_some_and(|p| p.distance(origin) > 4.0)
        {
            self.organizer.dragged = Some(start);
            if !self.organizer.selected.contains(&start) {
                self.organizer.selected = BTreeSet::from([start]);
            }
            if released && let Some(before) = drop_before {
                self.move_pages(ctx, before);
            }
        }
        if released {
            self.organizer.dragged = None;
            self.organizer.pressed = None;
        }
    }
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;
    use mypdf_milestone0::annotations::{Annotation, Kind, Style};

    #[test]
    fn pages_copy_with_annotations_and_paste_into_another_open_pdf() {
        let temp = tempfile::tempdir().unwrap();
        let font = mypdf_milestone0::document::default_font().unwrap();
        let first = temp.path().join("Contract.pdf");
        let second = temp.path().join("Memo.pdf");
        std::fs::write(&first, mypdf_milestone0::document::fixture(&font).unwrap()).unwrap();
        std::fs::write(
            &second,
            mypdf_milestone0::convert::blank(595., 842.).unwrap(),
        )
        .unwrap();
        let ctx = egui::Context::default();
        let mut app = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            vec![first, second],
        );
        let frame = |app: &mut Viewer, events: Vec<egui::Event>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 820.))),
                    events,
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
        };
        let until = |app: &mut Viewer, ready: &dyn Fn(&Viewer) -> bool| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while !ready(app) {
                assert!(std::time::Instant::now() < deadline, "{:?}", app.status);
                frame(app, vec![]);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        let ready = |t: &Tab| t.session.as_ref().is_some_and(|s| s.editable);
        until(&mut app, &|a| a.tabs.iter().all(ready));
        app.editor.mode = editing::Mode::Pages;

        // Mark page 2 of the contract, then copy just that page.
        app.active = 0;
        let session = app.tabs[0].session.as_mut().unwrap();
        let mut marks = session.annotations.clone();
        marks.push(Annotation::new(
            1,
            [100., 100., 220., 160.],
            Kind::Rectangle,
            Style::default(),
        ));
        session.commit(marks).unwrap();
        frame(&mut app, vec![]);
        app.organizer.selected = BTreeSet::from([1]);
        app.copy_pages(&ctx);
        until(&mut app, &|a| a.organizer.clipboard.is_some());
        assert_eq!(
            app.tabs[0].pages.len(),
            2,
            "copying leaves the source alone"
        );

        // Paste after the memo's only page; the pasted page becomes the selection.
        app.active = 1;
        frame(&mut app, vec![]);
        app.paste_pages(&ctx);
        until(&mut app, &|a| {
            !a.organizer.busy && a.tabs[1].pages.len() == 2 && ready(&a.tabs[1])
        });
        frame(&mut app, vec![]);
        assert_eq!(app.organizer.selected, BTreeSet::from([1]));
        let memo = app.tabs[1].session.as_ref().unwrap();
        assert!(
            memo.annotations
                .iter()
                .any(|a| a.page == 1 && matches!(a.kind, Kind::Rectangle)),
            "annotations travel with the page"
        );
        let text = |bytes: &[u8], page| -> String {
            mypdf_milestone0::document::text_runs(bytes, page)
                .unwrap()
                .into_iter()
                .map(|r| r.text)
                .collect()
        };
        let contract = app.tabs[0].session.as_ref().unwrap().source.clone();
        assert_eq!(text(&memo.source, 2), text(&contract, 2));

        // ⌘V (a paste event) in Pages pastes the pages again, after the selection.
        frame(
            &mut app,
            vec![egui::Event::Paste("1 page from Contract.pdf".into())],
        );
        until(&mut app, &|a| {
            !a.organizer.busy && a.tabs[1].pages.len() == 3 && ready(&a.tabs[1])
        });
        frame(&mut app, vec![]);
        assert_eq!(app.organizer.selected, BTreeSet::from([2]));
        app.editor_undo(false);
        until(&mut app, &|a| {
            a.tabs[1].pages.len() == 2 && ready(&a.tabs[1])
        });
    }
}
