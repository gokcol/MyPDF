//! Native thumbnail organizer and bounded background page transactions.
use super::*;
use mypdf_milestone0::pages::{self, Operation};
use std::collections::BTreeSet;

type Change = Result<(u64, u64, Vec<u8>, Option<Vec<usize>>), String>;
/// Weak source identities tie image remapping to the existing Undo/Redo history.
pub(super) struct PageReorder {
    before: std::sync::Weak<[u8]>,
    after: std::sync::Weak<[u8]>,
    /// New page index -> old page index.
    order: Vec<usize>,
}

fn inverse_order(order: &[usize]) -> Vec<usize> {
    let mut inverse = vec![0; order.len()];
    for (new, &old) in order.iter().enumerate() {
        inverse[old] = new;
    }
    inverse
}
/// Pages copied from one document, ready to paste into any open document.
pub(super) struct PageClip {
    bytes: Vec<u8>,
    pages: usize,
    from: String,
    followup: Option<ClipFollowup>,
}
struct ClipFollowup {
    tab: u64,
    revision: u64,
    operation: ClipOperation,
    selected: Vec<usize>,
}
#[derive(Clone, Copy, PartialEq)]
enum ClipOperation {
    Copy,
    Cut,
    Duplicate,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum ThumbnailAction {
    Copy,
    Cut,
    PasteBefore,
    PasteAfter,
    Duplicate,
    RotateLeft,
    RotateRight,
    Extract,
    Delete,
    SelectAll,
}
impl ThumbnailAction {
    fn label(self) -> &'static str {
        match self {
            Self::Copy => "Copy pages",
            Self::Cut => "Cut pages",
            Self::PasteBefore => "Paste before",
            Self::PasteAfter => "Paste after",
            Self::Duplicate => "Duplicate pages",
            Self::RotateLeft => "Rotate left",
            Self::RotateRight => "Rotate right",
            Self::Extract => "Extract pages…",
            Self::Delete => "Delete pages",
            Self::SelectAll => "Select all pages",
        }
    }
    fn icon(self) -> Icon {
        match self {
            Self::Copy | Self::Duplicate => Icon::Copy,
            Self::Cut => Icon::Cut,
            Self::PasteBefore | Self::PasteAfter => Icon::Paste,
            Self::RotateLeft => Icon::RotateLeft,
            Self::RotateRight => Icon::RotateRight,
            Self::Extract => Icon::Extract,
            Self::Delete => Icon::Trash,
            Self::SelectAll => Icon::Select,
        }
    }
    fn shortcut(self) -> Option<egui::KeyboardShortcut> {
        use egui::{Key, KeyboardShortcut, Modifiers as M};
        let (modifiers, key) = match self {
            Self::Copy => (M::COMMAND, Key::C),
            Self::Cut => (M::COMMAND, Key::X),
            Self::PasteAfter => (M::COMMAND, Key::V),
            Self::Duplicate => (M::COMMAND, Key::D),
            Self::SelectAll => (M::COMMAND, Key::A),
            Self::Delete => (
                M::NONE,
                if cfg!(target_os = "macos") {
                    Key::Backspace
                } else {
                    Key::Delete
                },
            ),
            _ => return None,
        };
        Some(KeyboardShortcut::new(modifiers, key))
    }
}

/// A secondary click targets that page without throwing away a multiselection.
/// Both thumbnail views share this selection policy and compact menu.
pub(super) fn thumbnail_context_menu(
    ui: &mut egui::Ui,
    response: &egui::Response,
    organizer: &mut Organizer,
    tab: &mut Tab,
    page: usize,
    blocked: bool,
) -> Option<ThumbnailAction> {
    if response.clicked() || response.secondary_clicked() {
        organizer.select_thumbnail(
            tab,
            page,
            ui.input(|i| i.modifiers),
            response.secondary_clicked(),
        );
        if let Some(focus) = ui.ctx().memory(|m| m.focused()) {
            ui.ctx().memory_mut(|m| m.surrender_focus(focus));
        }
    }
    let mut action = None;
    response.context_menu(|ui| {
        let items = [
            ThumbnailAction::Copy,
            ThumbnailAction::Cut,
            ThumbnailAction::PasteBefore,
            ThumbnailAction::PasteAfter,
            ThumbnailAction::Duplicate,
            ThumbnailAction::RotateLeft,
            ThumbnailAction::RotateRight,
            ThumbnailAction::Extract,
            ThumbnailAction::Delete,
            ThumbnailAction::SelectAll,
        ];
        let width = ui::menu_width(ui, items.iter().map(|a| (a.label(), a.shortcut())));
        ui.set_width(width);
        ui.spacing_mut().interact_size.y = 24.;
        ui.spacing_mut().item_spacing.y = 2.;
        ui.spacing_mut().button_padding = vec2(6., 2.);
        for item in items {
            if matches!(
                item,
                ThumbnailAction::Duplicate
                    | ThumbnailAction::RotateLeft
                    | ThumbnailAction::Extract
                    | ThumbnailAction::Delete
                    | ThumbnailAction::SelectAll
            ) {
                ui.separator();
            }
            if ui
                .add_enabled(
                    organizer.action_enabled(tab, item, blocked),
                    ui::MenuItem::new(item.icon(), item.label())
                        .shortcut(item.shortcut())
                        .width(width),
                )
                .clicked()
            {
                action = Some(item);
                ui.close();
            }
        }
    });
    action
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
    change_name: &'static str,
    tx: mpsc::Sender<Change>,
    rx: mpsc::Receiver<Change>,
    output_tx: mpsc::Sender<Result<String, String>>,
    output_rx: mpsc::Receiver<Result<String, String>>,
    pub clipboard: Option<Arc<PageClip>>,
    clip_tx: mpsc::Sender<Result<PageClip, String>>,
    clip_rx: mpsc::Receiver<Result<PageClip, String>>,
    /// Pages to select once a paste has reloaded: tab, page count, pages.
    pending_select: Option<(u64, usize, BTreeSet<usize>)>,
    thumbnail_focus: Option<u64>,
    thumbnail_bounds: Option<Rect>,
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
            change_name: "Edit pages",
            tx,
            rx,
            output_tx,
            output_rx,
            clipboard: None,
            clip_tx,
            clip_rx,
            pending_select: None,
            thumbnail_focus: None,
            thumbnail_bounds: None,
        }
    }
    pub(super) fn use_tab(&mut self, tab: &Tab) {
        if self.tab != tab.id {
            self.tab = tab.id;
            self.pressed = None;
            self.dragged = None;
            self.selected = BTreeSet::from([tab.page]);
            self.range = (tab.page + 1).to_string();
            self.crop = false;
        }
        self.selected.retain(|page| *page < tab.pages.len());
        // A new document may first be shown before Loaded supplies its pages.
        // Restore the current page once ready, and after deleting/rotating pages,
        // so sidebar actions have the same valid default target as Pages mode.
        if self.selected.is_empty() && !tab.pages.is_empty() {
            self.selected.insert(tab.page.min(tab.pages.len() - 1));
        }
        if let Some((id, count, pages)) = &self.pending_select
            && *id == tab.id
            && *count == tab.pages.len()
        {
            self.selected = pages.clone();
            self.pending_select = None;
        }
    }
    pub(super) fn thumbnail_area(&mut self, tab: u64, bounds: Rect) {
        if self.thumbnail_focus.is_some_and(|id| id != tab) {
            self.thumbnail_focus = None;
        }
        self.thumbnail_bounds = Some(bounds);
    }
    fn select_thumbnail(
        &mut self,
        tab: &mut Tab,
        page: usize,
        modifiers: egui::Modifiers,
        secondary: bool,
    ) {
        self.use_tab(tab);
        self.thumbnail_focus = Some(tab.id);
        if secondary {
            if !self.selected.contains(&page) {
                self.selected = BTreeSet::from([page]);
            }
        } else if modifiers.command {
            if !self.selected.remove(&page) {
                self.selected.insert(page);
            }
        } else if modifiers.shift {
            let anchor = self.selected.first().copied().unwrap_or(page);
            self.selected.extend(anchor.min(page)..=anchor.max(page));
        } else {
            self.selected = BTreeSet::from([page]);
        }
        // The caller owns navigation: the sidebar must call Tab::jump while
        // the previous reading position is still available for Back history.
        self.range = self
            .selected
            .iter()
            .map(|p| (p + 1).to_string())
            .collect::<Vec<_>>()
            .join(", ");
    }
    fn action_enabled(&self, tab: &Tab, action: ThumbnailAction, blocked: bool) -> bool {
        let selected = self.tab == tab.id && !self.selected.is_empty();
        let readable = selected && tab.session.is_some() && !blocked;
        let editable = readable && tab.session.as_ref().is_some_and(|s| s.editable);
        match action {
            ThumbnailAction::Copy | ThumbnailAction::Extract => readable,
            ThumbnailAction::Cut | ThumbnailAction::Delete => {
                editable && self.selected.len() < tab.pages.len()
            }
            ThumbnailAction::PasteBefore | ThumbnailAction::PasteAfter => {
                editable && self.clipboard.is_some()
            }
            ThumbnailAction::Duplicate
            | ThumbnailAction::RotateLeft
            | ThumbnailAction::RotateRight => editable,
            ThumbnailAction::SelectAll => !tab.pages.is_empty(),
        }
    }
}
impl Tab {
    /// Old page index -> new page index, only for a known pure reorder.
    pub(super) fn reorder_mapping(&self, source: &Arc<[u8]>) -> Option<Vec<usize>> {
        let current = self.bytes.as_ref()?;
        self.page_reorders.iter().find_map(|change| {
            let before = change.before.upgrade()?;
            let after = change.after.upgrade()?;
            if Arc::ptr_eq(current, &before) && Arc::ptr_eq(source, &after) {
                Some(inverse_order(&change.order))
            } else if Arc::ptr_eq(current, &after) && Arc::ptr_eq(source, &before) {
                Some(change.order.clone())
            } else {
                None
            }
        })
    }

    /// Rebuild rendering/search only when an undo or page edit changes the PDF source.
    pub(super) fn refresh_document(&mut self, ctx: &egui::Context) -> Option<Vec<usize>> {
        // Release weak allocations promptly when history trimming removes a
        // source, including frames where only annotations have changed.
        self.page_reorders
            .retain(|change| change.before.strong_count() > 0 && change.after.strong_count() > 0);
        let source = self.session.as_ref().map(|s| s.source.clone())?;
        if self.bytes.as_ref().is_some_and(|b| Arc::ptr_eq(b, &source)) {
            return None;
        }
        let mapping = self.reorder_mapping(&source);
        let wake = ctx.clone();
        self.worker = Worker::from_bytes(source.clone(), move || wake.request_repaint());
        self.bytes = Some(source);
        // After a content edit, each page keeps its sharpest old image until redrawn.
        let keep_stale = std::mem::take(&mut self.keep_stale);
        if !keep_stale {
            self.stale.clear();
        }
        if let Some(mapping) = &mapping {
            // Page contents and dimensions are unchanged. Move the existing GPU
            // textures with their pages, including thumbnails, without a blank frame.
            self.rasters = self
                .rasters
                .drain()
                .map(|(mut key, raster)| {
                    key.page = mapping[key.page];
                    (key, raster)
                })
                .collect();
            let old_pages = self.pages.clone();
            for (old, &new) in mapping.iter().enumerate() {
                self.pages[new] = old_pages[old];
            }
            self.page = mapping[self.page];
            self.page_input = (self.page + 1).to_string();
            if let Some((page, _)) = &mut self.jump {
                *page = mapping[*page];
            }
            for (page, _) in self.back.iter_mut().chain(&mut self.forward) {
                if let Some(new) = mapping.get(*page) {
                    *page = *new;
                }
            }
            self.document_layout = None;
        } else if keep_stale {
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
        if mapping.is_none() {
            self.rasters.clear();
        }
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
        mapping
    }
}
impl Viewer {
    /// Page snapshots and document replacement must not leave an editor draft
    /// behind on an old page index or silently omit it from a copied page.
    fn page_edits_ready(&mut self, copying: bool) -> bool {
        let Some(id) = self.tabs.get(self.active).map(|tab| tab.id) else {
            return false;
        };
        let verb = if copying { "copying" } else { "changing" };
        if self.pending_form_edits(id) {
            self.focus_pending_form_edits(id);
            self.status = Some((
                format!("Apply or discard the pending form changes before {verb} pages."),
                false,
            ));
            return false;
        }
        if self.unfinished_edit(Some(id)) {
            self.status = Some((
                format!("Finish the current edit with Done or Cancel before {verb} pages."),
                false,
            ));
            return false;
        }
        self.apply_pending_annotation()
    }

    pub(super) fn page_events(&mut self, ctx: &egui::Context) {
        for result in self.organizer.rx.try_iter() {
            self.organizer.busy = false;
            match result {
                Ok((id, revision, bytes, order)) => {
                    let active = self.tabs.get(self.active).is_some_and(|t| t.id == id);
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id)
                        && let Some(s) = &mut tab.session
                    {
                        if s.revision() != revision {
                            self.status=Some(("The document changed while pages were processing. Please try again.".into(),true));
                            continue;
                        }
                        let before = Arc::downgrade(&s.source);
                        match s.commit_document(bytes) {
                            Ok(()) => {
                                s.name_last_change(self.organizer.change_name);
                                let reordered = order.is_some();
                                if let Some(order) = order {
                                    tab.page_reorders.push(PageReorder {
                                        before,
                                        after: Arc::downgrade(&s.source),
                                        order,
                                    });
                                }
                                if active {
                                    self.editor.draft = None;
                                }
                                if self.organizer.tab == id && !reordered {
                                    self.organizer.selected.clear();
                                }
                                self.status = Some((
                                    "Pages updated • Undo is available • Save to keep your changes"
                                        .into(),
                                    false,
                                ));
                            }
                            Err(e) => self.status = Some((format!("{e:#}"), true)),
                        }
                    }
                }
                Err(e) => self.status = Some((e, true)),
            }
        }
        let copied: Vec<_> = self.organizer.clip_rx.try_iter().collect();
        for result in copied {
            match result {
                Ok(mut clip) => {
                    let followup = clip.followup.take();
                    let duplicate = followup
                        .as_ref()
                        .is_some_and(|f| f.operation == ClipOperation::Duplicate);
                    self.status = Some((
                        format!(
                            "Copied {} • Open another PDF in Pages and press ⌘V (or Paste pages) to insert after the selected page",
                            plural(clip.pages, "page")
                        ),
                        false,
                    ));
                    let clip = Arc::new(clip);
                    if !duplicate {
                        ctx.copy_text(format!("{} from {}", plural(clip.pages, "page"), clip.from));
                        self.organizer.clipboard = Some(clip.clone());
                    }
                    if let Some(followup) = followup {
                        let unchanged = !self.organizer.busy
                            && !self.content.busy
                            && self.tabs.get(self.active).is_some_and(|tab| {
                                tab.id == followup.tab
                                    && tab.session.as_ref().is_some_and(|s| {
                                        s.editable && s.revision() == followup.revision
                                    })
                            });
                        if !unchanged {
                            self.status = Some(("The document changed or another page operation is running. The original pages are unchanged.".into(), true));
                            continue;
                        }
                        if followup.operation == ClipOperation::Cut {
                            self.page_change(ctx, Operation::Delete(followup.selected));
                            if self.organizer.busy {
                                self.organizer.change_name = "Cut pages";
                            }
                        } else {
                            let at = followup.selected.last().map_or(0, |page| page + 1);
                            self.insert_page_clip(ctx, &clip, at);
                            if self.organizer.busy {
                                self.organizer.change_name = "Duplicate pages";
                            }
                        }
                    }
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
            if let Some(mapping) = tab.refresh_document(ctx)
                && self.organizer.tab == tab.id
            {
                self.organizer.selected = self
                    .organizer
                    .selected
                    .iter()
                    .filter_map(|old| mapping.get(*old).copied())
                    .collect();
                self.organizer.range = self
                    .organizer
                    .selected
                    .iter()
                    .map(|page| (page + 1).to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
            }
        }
    }
    pub(super) fn page_change(&mut self, ctx: &egui::Context, operation: Operation) {
        if self.organizer.busy || self.content.busy || !self.page_edits_ready(false) {
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let Some(s) = &tab.session else { return };
        if let Operation::Reorder(order) = &operation
            && order.iter().copied().eq(0..tab.pages.len())
        {
            return;
        }
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
        self.organizer.change_name = match &operation {
            Operation::Reorder(_) => "Reorder pages",
            Operation::Delete(_) => "Delete pages",
            Operation::InsertBlank { .. } => "Insert blank page",
            Operation::InsertPdf { .. } => "Insert PDF pages",
            Operation::Rotate { .. } => "Rotate pages",
            Operation::Crop { .. } => "Crop pages",
            Operation::ResetCrop(_) => "Restore page crop",
        };
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
                    let bytes = pages::apply(&current, &operation)?;
                    let order = match operation {
                        Operation::Reorder(order) => Some(order),
                        _ => None,
                    };
                    Ok((id, revision, bytes, order))
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
        self.organizer.use_tab(tab);
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
                            "{} · {} selected",
                            pages_label(count),
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
    fn thumbnail_owns_input(&mut self, ctx: &egui::Context) -> bool {
        let Some(tab) = self.tabs.get(self.active) else {
            return false;
        };
        let pages_mode = self.editor.mode == editing::Mode::Pages;
        if !pages_mode {
            if !self.sidebar || self.search || self.outline || self.editor.annotation_sidebar {
                self.organizer.thumbnail_focus = None;
            }
            if !egui::Popup::is_any_open(ctx)
                && ctx.input(|i| {
                    i.pointer.any_pressed()
                        && i.pointer.interact_pos().is_some_and(|p| {
                            self.organizer
                                .thumbnail_bounds
                                .is_none_or(|bounds| !bounds.contains(p))
                        })
                })
            {
                self.organizer.thumbnail_focus = None;
            }
        }
        (pages_mode || self.organizer.thumbnail_focus == Some(tab.id))
            && !self.home.visible
            && !self.digital.open
            && !self.editor.signature_open
            && !self.editor.library_open
            && !self.editor.selection_dialog_open()
            && !self.about_open
            && !self.recovery.open
            && self.close_intent.is_none()
            && !self.productivity.owns_input()
            && !self.workbench_owns_shortcuts()
            && !ctx.wants_keyboard_input()
            && !self.editor.text_editing
            && !ctx.memory(|m| m.top_modal_layer().is_some())
    }
    /// Thumbnail focus owns only whole-page commands, leaving normal PDF text,
    /// annotation and numeric-field shortcuts untouched after a canvas click.
    pub(super) fn thumbnail_command(&mut self, action: &str, ctx: &egui::Context) -> bool {
        let command = match action {
            "copy" => ThumbnailAction::Copy,
            "cut" => ThumbnailAction::Cut,
            "paste" => ThumbnailAction::PasteAfter,
            "select_all" => ThumbnailAction::SelectAll,
            "duplicate" | "object_duplicate" => ThumbnailAction::Duplicate,
            "delete" => ThumbnailAction::Delete,
            _ => return false,
        };
        if !self.thumbnail_owns_input(ctx) {
            return false;
        }
        self.thumbnail_action(ctx, command);
        true
    }
    pub(super) fn thumbnail_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if !self.thumbnail_owns_input(ctx) {
            return false;
        }
        for (key, action) in [
            ("copy", ThumbnailAction::Copy),
            ("cut", ThumbnailAction::Cut),
            ("paste", ThumbnailAction::PasteAfter),
            ("object_duplicate", ThumbnailAction::Duplicate),
            ("all", ThumbnailAction::SelectAll),
            ("delete", ThumbnailAction::Delete),
        ] {
            if content::context::consume_action(ctx, key) {
                self.thumbnail_action(ctx, action);
                return true;
            }
        }
        false
    }
    pub(super) fn thumbnail_action(&mut self, ctx: &egui::Context, action: ThumbnailAction) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        self.organizer.use_tab(tab);
        if !self
            .organizer
            .action_enabled(tab, action, self.organizer.busy || self.content.busy)
        {
            return;
        }
        let selected: Vec<_> = self.organizer.selected.iter().copied().collect();
        match action {
            ThumbnailAction::Copy => self.copy_pages(ctx),
            ThumbnailAction::Cut => self.copy_pages_operation(ctx, ClipOperation::Cut),
            ThumbnailAction::Duplicate => self.copy_pages_operation(ctx, ClipOperation::Duplicate),
            ThumbnailAction::PasteAfter => self.paste_pages(ctx),
            ThumbnailAction::PasteBefore => {
                if let Some(clip) = self.organizer.clipboard.clone() {
                    self.insert_page_clip(ctx, &clip, selected.first().copied().unwrap_or(0));
                }
            }
            ThumbnailAction::RotateLeft | ThumbnailAction::RotateRight => self.page_change(
                ctx,
                Operation::Rotate {
                    pages: selected,
                    clockwise: action == ThumbnailAction::RotateRight,
                },
            ),
            ThumbnailAction::Delete => self.page_change(ctx, Operation::Delete(selected)),
            ThumbnailAction::Extract => self.export_pages(ctx, false),
            ThumbnailAction::SelectAll => {
                self.organizer.selected = (0..tab.pages.len()).collect();
                self.organizer.range = format!("1-{}", tab.pages.len());
            }
        }
    }
    /// Copy the selected pages, with their annotations, to the app's page clipboard.
    pub(super) fn copy_pages(&mut self, ctx: &egui::Context) {
        self.copy_pages_operation(ctx, ClipOperation::Copy);
    }
    fn copy_pages_operation(&mut self, ctx: &egui::Context, operation: ClipOperation) {
        if self.organizer.busy
            || self.content.busy
            || !self.page_edits_ready(operation == ClipOperation::Copy)
        {
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let Some(s) = &tab.session else { return };
        let selected: Vec<usize> = if self.organizer.tab == tab.id {
            self.organizer.selected.iter().copied().collect()
        } else {
            vec![tab.page]
        };
        if selected.is_empty()
            || (operation != ClipOperation::Copy
                && (!s.editable
                    || (operation == ClipOperation::Cut && selected.len() >= tab.pages.len())))
        {
            return;
        }
        let id = tab.id;
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
                        followup: (operation != ClipOperation::Copy).then_some(ClipFollowup {
                            tab: id,
                            revision,
                            operation,
                            selected,
                        }),
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
        self.insert_page_clip(ctx, &clip, at);
    }
    fn insert_page_clip(&mut self, ctx: &egui::Context, clip: &PageClip, at: usize) {
        if self.organizer.busy || self.content.busy {
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let id = tab.id;
        let count = tab.pages.len();
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
    pub(super) fn merge_files(&mut self, ctx: &egui::Context) {
        if let Some(t) = self.tabs.get(self.active) {
            let count = t.pages.len();
            self.import_pages(ctx, count);
        }
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
                Ok((id, revision, current, None))
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
        if self.organizer.busy || self.content.busy || !self.page_edits_ready(true) {
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
                        "Extracted {} to {}",
                        pages_label(selected.len()),
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
                ui::tip(ui, "Your original stays intact", "Reorder, rotate, crop and delete are undoable. Use Save when you are ready.");
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
        let mut context_action = None;
        let blocked = self.organizer.busy || self.content.busy;
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
                self.organizer.use_tab(tab);
                self.organizer
                    .thumbnail_area(tab.id, ui.max_rect().intersect(ui.clip_rect()));
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
                                    if let Some(action) = thumbnail_context_menu(
                                        ui,
                                        &response,
                                        &mut self.organizer,
                                        tab,
                                        i,
                                        blocked,
                                    ) {
                                        context_action = Some(action);
                                    }
                                    if response.clicked() {
                                        tab.page = i;
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
        if let Some(action) = context_action {
            self.thumbnail_action(ctx, action);
        }
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
    fn reordering_pages_keeps_rendered_images_selection_and_undo_redo_without_blank_frames() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("Reorder.pdf");
        let font = mypdf_milestone0::document::default_font().unwrap();
        let bytes = mypdf_milestone0::document::fixture(&font).unwrap();
        let bytes = pages::apply(
            &bytes,
            &Operation::InsertBlank {
                at: 2,
                width: 400.,
                height: 300.,
            },
        )
        .unwrap();
        std::fs::write(&path, bytes).unwrap();
        let ctx = egui::Context::default();
        let mut app = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            vec![path],
        );
        app.editor.mode = editing::Mode::Pages;
        let frame = |app: &mut Viewer| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 820.))),
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
        };
        let until = |app: &mut Viewer, ready: &dyn Fn(&Viewer) -> bool| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while !ready(app) {
                assert!(std::time::Instant::now() < deadline, "{:?}", app.status);
                frame(app);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        let ready = |a: &Viewer| a.tabs[0].session.as_ref().is_some_and(|s| s.editable);
        until(&mut app, &|a| {
            ready(a) && a.tabs[0].rasters.keys().filter(|k| k.thumbnail).count() == 3
        });
        let full = viewer::raster_key(0, app.tabs[0].pages[0], 1., false);
        app.tabs[0].request(full);
        until(&mut app, &|a| a.tabs[0].rasters.contains_key(&full));
        let original: HashMap<_, _> = app.tabs[0]
            .rasters
            .iter()
            .map(|(key, raster)| (*key, raster.texture.id()))
            .collect();
        let sizes = app.tabs[0].pages.clone();
        let assert_images = |app: &Viewer, order: &[usize]| {
            let mapping = inverse_order(order);
            let tab = &app.tabs[0];
            assert_eq!(tab.rasters.len(), original.len());
            for (key, texture) in &original {
                let moved = RasterKey {
                    page: mapping[key.page],
                    ..*key
                };
                assert_eq!(
                    tab.rasters.get(&moved).map(|r| r.texture.id()),
                    Some(*texture)
                );
            }
            for (new, &old) in order.iter().enumerate() {
                assert_eq!(tab.pages[new].width, sizes[old].width);
                assert_eq!(tab.pages[new].height, sizes[old].height);
            }
        };

        app.tabs[0].page = 0;
        app.organizer.selected = BTreeSet::from([0, 1]);
        // Dropping back in the same position must not rewrite the PDF.
        let source = app.tabs[0].session.as_ref().unwrap().source.clone();
        app.move_pages(&ctx, 0);
        assert!(!app.organizer.busy);
        assert!(Arc::ptr_eq(
            &source,
            &app.tabs[0].session.as_ref().unwrap().source
        ));

        app.move_pages(&ctx, 3);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while app.organizer.busy {
            assert!(std::time::Instant::now() < deadline);
            // Inspect the commit frame before the new worker can supply images.
            app.page_events(&ctx);
            if app.organizer.busy {
                assert_images(&app, &[0, 1, 2]);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_images(&app, &[2, 0, 1]);
        assert_eq!(app.organizer.selected, BTreeSet::from([1, 2]));
        assert_eq!(app.tabs[0].page, 1);
        until(&mut app, &ready);
        assert_images(&app, &[2, 0, 1]);

        app.editor_undo(false);
        app.page_events(&ctx);
        assert_images(&app, &[0, 1, 2]);
        assert_eq!(app.organizer.selected, BTreeSet::from([0, 1]));
        assert_eq!(app.tabs[0].page, 0);
        until(&mut app, &ready);
        app.editor_undo(true);
        app.page_events(&ctx);
        assert_images(&app, &[2, 0, 1]);
        assert_eq!(app.organizer.selected, BTreeSet::from([1, 2]));
        assert_eq!(app.tabs[0].page, 1);
        until(&mut app, &ready);

        // A real content/geometry change must still invalidate the old images.
        let session = app.tabs[0].session.as_mut().unwrap();
        let rotated = pages::apply(
            &session.source,
            &Operation::Rotate {
                pages: vec![0],
                clockwise: true,
            },
        )
        .unwrap();
        session.commit_document(rotated).unwrap();
        app.page_events(&ctx);
        assert!(app.tabs[0].rasters.is_empty());
    }

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

    fn thumbnail_fixture() -> (Viewer, egui::Context) {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let mut bytes = mypdf_milestone0::convert::blank(500., 500.).unwrap();
        for width in [400., 300., 200.] {
            bytes = pages::apply(
                &bytes,
                &Operation::InsertBlank {
                    at: 0,
                    width,
                    height: 500.,
                },
            )
            .unwrap();
        }
        app.open_memory(PathBuf::from("Thumbnail page commands.pdf"), bytes, &ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        app.home.visible = false;
        app.editor.mode = editing::Mode::Read;
        app.sidebar = true;
        app.organizer.use_tab(&app.tabs[0]);
        (app, ctx)
    }
    fn thumbnail_frame(
        app: &mut Viewer,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 900.))),
                events,
                ..Default::default()
            },
            |ctx| {
                app.page_events(ctx);
                app.events(ctx);
                app.sidebar(ctx);
                egui::CentralPanel::default().show(ctx, |_ui| {});
            },
        )
    }
    fn until_thumbnail(app: &mut Viewer, ctx: &egui::Context, ready: impl Fn(&Viewer) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !ready(app) {
            assert!(
                std::time::Instant::now() < deadline,
                "status={:?}; active={}; busy={}; tabs={:?}",
                app.status,
                app.active,
                app.organizer.busy,
                app.tabs
                    .iter()
                    .map(|tab| (
                        &tab.path,
                        tab.pages.len(),
                        tab.session.as_ref().map(|s| (s.editable, s.revision())),
                        &tab.error
                    ))
                    .collect::<Vec<_>>()
            );
            thumbnail_frame(app, ctx, vec![]);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    #[test]
    fn thumbnail_secondary_selection_preserves_multiple_pages_and_command_shift_ranges() {
        let (mut app, _ctx) = thumbnail_fixture();
        let tab = &mut app.tabs[0];
        let organizer = &mut app.organizer;
        organizer.select_thumbnail(tab, 0, egui::Modifiers::NONE, false);
        organizer.select_thumbnail(tab, 2, egui::Modifiers::COMMAND, false);
        assert_eq!(organizer.selected, BTreeSet::from([0, 2]));
        organizer.select_thumbnail(tab, 2, egui::Modifiers::NONE, true);
        assert_eq!(organizer.selected, BTreeSet::from([0, 2]));
        organizer.select_thumbnail(tab, 1, egui::Modifiers::NONE, true);
        assert_eq!(organizer.selected, BTreeSet::from([1]));
        organizer.select_thumbnail(tab, 3, egui::Modifiers::SHIFT, false);
        assert_eq!(organizer.selected, BTreeSet::from([1, 2, 3]));
        assert_eq!(organizer.range, "2, 3, 4");
    }

    #[test]
    fn sidebar_thumbnail_navigation_records_history_and_secondary_click_only_selects() {
        let (mut app, ctx) = thumbnail_fixture();
        thumbnail_frame(&mut app, &ctx, vec![]);
        let output = thumbnail_frame(&mut app, &ctx, vec![]);
        let nodes = output.platform_output.accesskit_update.unwrap().nodes;
        let thumbnail_point = |label| {
            let bounds = nodes
                .iter()
                .find(|(_, node)| node.label() == Some(label))
                .unwrap()
                .1
                .bounds()
                .unwrap();
            pos2(
                ((bounds.x0 + bounds.x1) / 2.) as f32,
                ((bounds.y0 + bounds.y1) / 2.) as f32,
            )
        };
        let click = |app: &mut Viewer, point, button| {
            for pressed in [true, false] {
                thumbnail_frame(
                    app,
                    &ctx,
                    vec![
                        egui::Event::PointerMoved(point),
                        egui::Event::PointerButton {
                            pos: point,
                            button,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
        };

        click(
            &mut app,
            thumbnail_point("Go to page 3"),
            egui::PointerButton::Primary,
        );
        assert_eq!(app.tabs[0].page, 2);
        assert_eq!(app.tabs[0].back, vec![(0, 0.)]);
        app.tabs[0].history_step(false);
        assert_eq!(app.tabs[0].page, 0);
        assert_eq!(app.tabs[0].jump, Some((0, Some(0.))));

        click(
            &mut app,
            thumbnail_point("Go to page 2"),
            egui::PointerButton::Secondary,
        );
        assert_eq!(app.organizer.selected, BTreeSet::from([1]));
        assert_eq!(app.tabs[0].page, 0);
        assert!(app.tabs[0].back.is_empty());
    }

    #[test]
    fn sidebar_context_menu_has_copy_and_paste_actions_and_keeps_multiselection() {
        let (mut app, ctx) = thumbnail_fixture();
        thumbnail_frame(&mut app, &ctx, vec![]);
        thumbnail_frame(&mut app, &ctx, vec![]);
        app.organizer.selected = BTreeSet::from([0, 1]);
        let point = pos2(100., 120.);
        let mut labels = BTreeSet::new();
        for pressed in [true, false] {
            let output = thumbnail_frame(
                &mut app,
                &ctx,
                vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Secondary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            if let Some(update) = output.platform_output.accesskit_update {
                labels.extend(
                    update
                        .nodes
                        .iter()
                        .filter_map(|(_, node)| node.label().map(str::to_owned)),
                );
            }
        }
        let output = thumbnail_frame(&mut app, &ctx, vec![]);
        if let Some(update) = output.platform_output.accesskit_update {
            labels.extend(
                update
                    .nodes
                    .iter()
                    .filter_map(|(_, node)| node.label().map(str::to_owned)),
            );
        }
        assert!(egui::Popup::is_any_open(&ctx));
        assert_eq!(app.organizer.selected, BTreeSet::from([0, 1]));
        for label in [
            "Copy pages",
            "Cut pages",
            "Paste before",
            "Paste after",
            "Duplicate pages",
            "Rotate left",
            "Rotate right",
            "Extract pages…",
            "Delete pages",
            "Select all pages",
        ] {
            assert!(
                labels.contains(label),
                "missing menu row {label}: {labels:?}"
            );
        }
        assert!(app.editor.mode == editing::Mode::Read);
    }
    #[test]
    fn sidebar_copy_shortcut_is_scoped_to_thumbnails_and_read_only_copy_does_not_enable_editing() {
        let (mut app, ctx) = thumbnail_fixture();
        thumbnail_frame(&mut app, &ctx, vec![]);
        let tab = &mut app.tabs[0];
        tab.session.as_mut().unwrap().editable = false;
        app.organizer
            .select_thumbnail(tab, 1, egui::Modifiers::NONE, false);
        app.organizer
            .thumbnail_area(tab.id, Rect::from_min_max(pos2(0., 0.), pos2(220., 900.)));
        let mut consumed = false;
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Copy],
                ..Default::default()
            },
            |ctx| {
                consumed = app.thumbnail_shortcuts(ctx);
            },
        );
        assert!(consumed);
        until_thumbnail(&mut app, &ctx, |app| app.organizer.clipboard.is_some());
        assert_eq!(app.organizer.clipboard.as_ref().unwrap().pages, 1);
        assert!(!app.tabs[0].session.as_ref().unwrap().editable);
        assert!(app.editor.mode == editing::Mode::Read);
        let _ = ctx.run(
            egui::RawInput {
                events: vec![
                    egui::Event::PointerButton {
                        pos: pos2(500., 400.),
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::Copy,
                ],
                ..Default::default()
            },
            |ctx| {
                consumed = app.thumbnail_shortcuts(ctx);
            },
        );
        assert!(
            !consumed,
            "a canvas click returns Copy to text or annotation selection"
        );
        assert!(app.organizer.thumbnail_focus.is_none());
    }
    #[test]
    fn thumbnail_duplicate_cut_and_paste_before_are_undoable_and_preserve_clipboard() {
        let (mut app, ctx) = thumbnail_fixture();
        app.organizer.selected = BTreeSet::from([1, 2]);
        app.thumbnail_action(&ctx, ThumbnailAction::Copy);
        until_thumbnail(&mut app, &ctx, |app| app.organizer.clipboard.is_some());
        let original_clip = app.organizer.clipboard.clone().unwrap();
        app.thumbnail_action(&ctx, ThumbnailAction::Duplicate);
        until_thumbnail(&mut app, &ctx, |app| {
            app.tabs[0].pages.len() == 6 && app.tabs[0].session.as_ref().is_some_and(|s| s.editable)
        });
        assert!(Arc::ptr_eq(
            &original_clip,
            app.organizer.clipboard.as_ref().unwrap()
        ));
        assert_eq!(app.organizer.selected, BTreeSet::from([3, 4]));
        let original_widths: Vec<_> = app.tabs[0].pages.iter().map(|p| p.width).collect();
        assert_eq!(&original_widths[1..3], &original_widths[3..5]);
        app.thumbnail_action(&ctx, ThumbnailAction::Cut);
        until_thumbnail(&mut app, &ctx, |app| {
            app.tabs[0].pages.len() == 4 && app.tabs[0].session.as_ref().is_some_and(|s| s.editable)
        });
        assert_eq!(app.organizer.clipboard.as_ref().unwrap().pages, 2);
        app.organizer.selected = BTreeSet::from([0]);
        app.thumbnail_action(&ctx, ThumbnailAction::PasteBefore);
        until_thumbnail(&mut app, &ctx, |app| {
            app.tabs[0].pages.len() == 6 && app.tabs[0].session.as_ref().is_some_and(|s| s.editable)
        });
        assert_eq!(app.organizer.selected, BTreeSet::from([0, 1]));
        app.editor_undo(false);
        until_thumbnail(&mut app, &ctx, |app| {
            app.tabs[0].pages.len() == 4 && app.tabs[0].session.as_ref().is_some_and(|s| s.editable)
        });
        assert!(app.editor.mode == editing::Mode::Read);
        app.organizer.selected = (0..4).collect();
        let tab = &app.tabs[0];
        assert!(
            !app.organizer
                .action_enabled(tab, ThumbnailAction::Delete, false)
        );
        assert!(
            !app.organizer
                .action_enabled(tab, ThumbnailAction::Cut, false)
        );
    }

    #[test]
    fn sidebar_copies_multiple_pages_between_open_documents_at_the_selected_insertion_point() {
        let (mut app, ctx) = thumbnail_fixture();
        app.organizer.selected = BTreeSet::from([0, 2]);
        let source_widths: Vec<_> = app.tabs[0].pages.iter().map(|p| p.width).collect();
        app.thumbnail_action(&ctx, ThumbnailAction::Copy);
        until_thumbnail(&mut app, &ctx, |app| app.organizer.clipboard.is_some());
        let clip = app.organizer.clipboard.clone().unwrap();
        assert_eq!(clip.pages, 2);
        app.open_memory(
            PathBuf::from("Target memo.pdf"),
            mypdf_milestone0::convert::blank(600., 700.).unwrap(),
            &ctx,
        );
        until_thumbnail(&mut app, &ctx, |app| {
            app.tabs.len() == 2
                && app.tabs[1].session.as_ref().is_some_and(|s| s.editable)
                && app.tabs[1].pages.len() == 1
        });
        app.active = 1;
        app.organizer.use_tab(&app.tabs[1]);
        assert!(Arc::ptr_eq(
            &clip,
            app.organizer.clipboard.as_ref().unwrap()
        ));
        app.thumbnail_action(&ctx, ThumbnailAction::PasteBefore);
        until_thumbnail(&mut app, &ctx, |app| {
            app.tabs[1].pages.len() == 3 && app.tabs[1].session.as_ref().is_some_and(|s| s.editable)
        });
        assert_eq!(
            app.tabs[0].pages.len(),
            4,
            "Copy never deletes source pages"
        );
        assert_eq!(
            app.tabs[1]
                .pages
                .iter()
                .map(|p| p.width)
                .collect::<Vec<_>>(),
            vec![source_widths[0], source_widths[2], 600.]
        );
        assert_eq!(app.organizer.selected, BTreeSet::from([0, 1]));
        assert!(app.editor.mode == editing::Mode::Read);
    }

    #[test]
    fn thumbnail_page_actions_preserve_unfinished_pdf_text_and_async_followups() {
        let (mut app, ctx) = thumbnail_fixture();
        let id = app.tabs[0].id;
        let size = app.tabs[0].pages[0];
        let source = app.tabs[0].session.as_ref().unwrap().source.clone();
        let revision = app.tabs[0].session.as_ref().unwrap().revision();
        app.content
            .paste_new_text(id, 0, size, "Keep this unfinished text".into());
        app.editor.mode = editing::Mode::Edit;
        app.organizer.selected = BTreeSet::from([0]);
        for action in [
            ThumbnailAction::Copy,
            ThumbnailAction::Extract,
            ThumbnailAction::Cut,
            ThumbnailAction::Duplicate,
            ThumbnailAction::RotateRight,
            ThumbnailAction::Delete,
        ] {
            app.thumbnail_action(&ctx, action);
            assert!(!app.organizer.busy);
            assert_eq!(app.content.pending_text(), Some(id));
            assert!(app.content.has_pending_text_change(id));
            assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), revision);
            assert!(Arc::ptr_eq(
                &app.tabs[0].session.as_ref().unwrap().source,
                &source
            ));
            assert!(app.status.as_ref().unwrap().0.contains(
                if matches!(action, ThumbnailAction::Copy | ThumbnailAction::Extract) {
                    "before copying pages"
                } else {
                    "before changing pages"
                }
            ));
            assert!(app.organizer.clipboard.is_none());
        }
        // A draft can be opened while the original immutable snapshot is being
        // extracted. Its later Cut/Duplicate completion must check the same guard.
        for operation in [ClipOperation::Cut, ClipOperation::Duplicate] {
            app.organizer
                .clip_tx
                .send(Ok(PageClip {
                    bytes: pages::extract(&source, &[0]).unwrap(),
                    pages: 1,
                    from: "Snapshot before text edit".into(),
                    followup: Some(ClipFollowup {
                        tab: id,
                        revision,
                        operation,
                        selected: vec![0],
                    }),
                }))
                .unwrap();
            app.page_events(&ctx);
            assert!(!app.organizer.busy);
            assert_eq!(app.content.pending_text(), Some(id));
            assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), revision);
            assert_eq!(app.tabs[0].pages.len(), 4);
            assert!(Arc::ptr_eq(
                &app.tabs[0].session.as_ref().unwrap().source,
                &source
            ));
        }
    }

    #[test]
    fn global_shortcuts_prioritize_sidebar_pages_over_selected_annotations_for_keys_and_clipboard_events()
     {
        let (mut app, ctx) = thumbnail_fixture();
        thumbnail_frame(&mut app, &ctx, vec![]);
        app.editor.mode = editing::Mode::Annotate;
        let mark = Annotation::new(0, [30., 30., 100., 80.], Kind::Rectangle, Style::default());
        app.tabs[0].selected_annotation = Some(mark.id.clone());
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![mark])
            .unwrap();
        app.organizer.selected = BTreeSet::from([0, 2]);
        app.organizer.thumbnail_focus = Some(app.tabs[0].id);
        app.organizer.thumbnail_area(
            app.tabs[0].id,
            Rect::from_min_max(pos2(0., 0.), pos2(220., 900.)),
        );
        let command_key = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        for event in [command_key(egui::Key::C), egui::Event::Copy] {
            app.organizer.clipboard = None;
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![event],
                    ..Default::default()
                },
                |ctx| app.shortcuts(ctx),
            );
            until_thumbnail(&mut app, &ctx, |app| app.organizer.clipboard.is_some());
            assert_eq!(app.organizer.clipboard.as_ref().unwrap().pages, 2);
            assert!(
                app.editor.marks_clip.is_none(),
                "Copy belongs to focused thumbnails, not the selected rectangle"
            );
            assert!(app.editor.mode == editing::Mode::Annotate);
        }
        for (event, count) in [
            (
                egui::Event::Paste("2 pages from another document".into()),
                6,
            ),
            (command_key(egui::Key::V), 8),
        ] {
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![event],
                    ..Default::default()
                },
                |ctx| app.shortcuts(ctx),
            );
            until_thumbnail(&mut app, &ctx, |app| {
                app.tabs[0].pages.len() == count
                    && app.tabs[0].session.as_ref().is_some_and(|s| s.editable)
            });
            assert!(app.editor.mode == editing::Mode::Annotate);
            assert_eq!(app.organizer.selected.len(), 2);
        }
    }

    #[test]
    fn sidebar_selection_initializes_after_a_new_document_finishes_loading() {
        let (mut app, _ctx) = thumbnail_fixture();
        let tab = &mut app.tabs[0];
        let pages = std::mem::take(&mut tab.pages);
        let mut organizer = Organizer::new();
        organizer.use_tab(tab);
        assert!(
            organizer.selected.is_empty(),
            "no target exists while pages load"
        );
        tab.pages = pages;
        tab.page = 2;
        organizer.use_tab(tab);
        assert_eq!(organizer.selected, BTreeSet::from([2]));
        assert!(organizer.action_enabled(tab, ThumbnailAction::Copy, false));
        // The same repair applies when a page transaction invalidates the old
        // selection, without replacing a still-valid multiselection.
        organizer.selected = BTreeSet::from([100]);
        organizer.use_tab(tab);
        assert_eq!(organizer.selected, BTreeSet::from([2]));
        organizer.selected = BTreeSet::from([0, 3]);
        organizer.use_tab(tab);
        assert_eq!(organizer.selected, BTreeSet::from([0, 3]));
    }
}
