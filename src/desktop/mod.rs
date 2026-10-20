mod clipboard;
mod comparison;
mod content;
mod details;
mod digital;
mod editing;
mod form_tools;
mod header_footer;
mod home;
pub mod icon;
mod interface;
mod manipulation;
mod menu;
mod navigate;
mod navigation_editor;
mod nup;
pub mod open_with;
mod organize;
mod preferences;
#[cfg(test)]
mod preview;
mod print;
mod productivity;
mod protect;
mod reading;
mod recognize;
mod recovery;
mod redaction;
mod review;
mod save;
mod selection;
mod shrink;
mod sign_tools;
mod signatures;
mod styles;
mod tabs;
mod text_selection;
mod theme;
mod translation;
mod translation_note;
mod ui;
mod watermark;
mod window_drag;
mod workbench;
mod workspace;
mod zen;
use interface::{StatusContext, tab_caption};
use ui::{ActionButton, Icon};

use eframe::egui::{self, Color32, FontId, Rect, RichText, Stroke, TextureHandle, pos2, vec2};
use mypdf_milestone0::navigation;
use mypdf_milestone0::viewer::{self, Event, PageSize, PageText, RasterKey, SearchHit, Worker};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, mpsc};

const CACHE_BUDGET: usize = 128 * 1024 * 1024;
const HISTORY_BUDGET: usize = 512 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq)]
enum Zoom {
    Width,
    Page,
    Scale(f32),
}
#[derive(Clone)]
struct Raster {
    texture: TextureHandle,
    bytes: usize,
    used: u64,
}
struct Tab {
    document_group: u64,
    placement_guides: Option<(u64, usize, mypdf_milestone0::form_detect::PageGuides)>,
    id: u64,
    path: PathBuf,
    /// False for new, imported, or recovered in-memory documents until first Save.
    has_destination: bool,
    worker: Worker,
    bytes: Option<Arc<[u8]>>,
    pages: Vec<PageSize>,
    signed: bool,
    /// The signed file that edits are appended to, once editing is unlocked.
    signed_original: Option<Arc<[u8]>>,
    signatures: signatures::TabSignatures,
    unsaved_on_load: bool,
    remember_open: bool,
    session: Option<mypdf_milestone0::annotations::Session>,
    capabilities: Option<mypdf_milestone0::annotations::Capabilities>,
    editing_warning: Option<String>,
    selected_annotation: Option<String>,
    /// A multi-object selection on one page; empty unless at least two marks are chosen.
    group: Vec<String>,
    error: Option<String>,
    page: usize,
    page_input: String,
    zoom: Zoom,
    scale: f32,
    jump: Option<(usize, Option<f32>)>,
    document_view: Option<(Rect, egui::Vec2, f32)>,
    document_layout: Option<reading::Layout>,
    last_pinch: Option<std::time::Instant>,
    page_layout: reading::PageLayout,
    view_preferences: mypdf_milestone0::home::ReadingState,
    restore_view: bool,
    /// A related PDF copy starts with the source tab’s canvas offset.
    restore_scroll: bool,
    back: Vec<(usize, f32)>,
    forward: Vec<(usize, f32)>,
    text: Vec<Option<Arc<PageText>>>,
    search_worker: viewer::SearchWorker,
    search_at: Option<std::time::Instant>,
    searching: bool,
    jump_to_first_hit: bool,
    query: String,
    search_options: viewer::SearchOptions,
    hits: Vec<SearchHit>,
    hit: usize,
    indexed: usize,
    index_done: bool,
    search_limited: bool,
    search_dirty: bool,
    rasters: HashMap<RasterKey, Raster>,
    page_reorders: Vec<organize::PageReorder>,
    /// The previous version's page images, shown until the edited pages are drawn.
    stale: HashMap<usize, Raster>,
    /// The outline and links, read in the background.
    nav: Option<navigate::Nav>,
    /// A protected file waiting for its password.
    locked: Option<protect::Locked>,
    /// The password copies are saved with.
    password: Option<zeroize::Zeroizing<String>>,
    /// The next document change only edits page content, so old images may stand in.
    keep_stale: bool,
    pending: HashSet<RasterKey>,
    render_errors: HashMap<usize, String>,
}
impl Tab {
    fn name(&self) -> String {
        self.path
            .file_name()
            .unwrap_or(self.path.as_os_str())
            .to_string_lossy()
            .into_owned()
    }
    fn jump(&mut self, page: usize) {
        if !self.pages.is_empty() {
            if self.page != page.min(self.pages.len() - 1) {
                self.push_history();
            }
            self.page = page.min(self.pages.len() - 1);
            self.page_input = (self.page + 1).to_string();
            self.jump = Some((self.page, None));
        }
    }
    fn select_hit(&mut self, hit: usize) {
        if self.hits.is_empty() {
            return;
        }
        self.push_history();
        self.hit = hit % self.hits.len();
        let found = &self.hits[self.hit];
        let y = found.rects.first().map(|r| r[1]);
        self.page = found.page;
        self.page_input = (self.page + 1).to_string();
        self.jump = Some((self.page, y));
    }
    fn request(&mut self, key: RasterKey) {
        if !self.pending.contains(&key)
            && !self.rasters.contains_key(&key)
            && !self.render_errors.contains_key(&key.page)
            && self.worker.request(key)
        {
            self.pending.insert(key);
        }
    }
    fn search(&mut self, ctx: &egui::Context) {
        let results: Vec<_> = self.search_worker.results.try_iter().collect();
        for (generation, hits) in results {
            if generation != self.search_worker.generation() {
                continue;
            }
            self.hits = hits;
            self.hit = self.hit.min(self.hits.len().saturating_sub(1));
            self.searching = false;
            if self.jump_to_first_hit && !self.hits.is_empty() {
                self.select_hit(0);
                self.jump_to_first_hit = false;
            }
        }
        if !self.search_dirty {
            return;
        }
        if self.query.trim().is_empty() {
            self.hits.clear();
            self.search_dirty = false;
            self.searching = false;
            return;
        }
        let at = self.search_at.get_or_insert_with(std::time::Instant::now);
        if at.elapsed() >= std::time::Duration::from_millis(150)
            && self.search_worker.request_with(
                self.text.clone(),
                self.query.clone(),
                self.search_options,
            )
        {
            self.search_dirty = false;
            self.search_at = None;
            self.searching = true;
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
    }
}

/// "1 page", "12 pages".
fn pages_label(count: usize) -> String {
    format!("{count} page{}", if count == 1 { "" } else { "s" })
}

/// A small rounded status label tinted with `color`.
fn pill(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.12))
        .corner_radius(9)
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.).color(color))
        })
        .inner
}

pub struct Viewer {
    productivity: productivity::State,
    workbench: workbench::State,
    home: home::Home,
    content: content::ContentState,
    organizer: organize::Organizer,
    editor: editing::EditorState,
    digital: digital::DigitalState,
    sign_tools: sign_tools::State,
    tabs: Vec<Tab>,
    active: usize,
    tab_revealed: Option<u64>,
    tab_strip_width: f32,
    tab_controls: tabs::State,
    status_context: Option<StatusContext>,
    status_seen: Option<(String, bool)>,
    next_id: u64,
    tick: u64,
    sidebar: bool,
    inspector: bool,
    accessible_text: bool,
    recovery: recovery::Recovery,
    search: bool,
    /// The sidebar shows the document's outline.
    outline: bool,
    focus_search: bool,
    status: Option<(String, bool)>,
    saving: bool,
    /// The file name the next Save suggests.
    save_name: Option<String>,
    save_intent: save::Intent,
    saving_adopts: bool,
    /// A web or mail link waiting for the user to agree to open it.
    link_confirm: Option<String>,
    protect: Option<protect::ProtectDraft>,
    unprotect: bool,
    /// Reduce File Size is open, with this quality chosen.
    shrink: Option<mypdf_milestone0::compress::Level>,
    watermark: Option<watermark::Draft>,
    nup: Option<nup::Draft>,
    ocr_confirm: bool,
    ocr_progress: Option<Arc<recognize::Progress>>,
    save_events: mpsc::Receiver<Result<(PathBuf, u64, u64), String>>,
    save_tx: mpsc::Sender<Result<(PathBuf, u64, u64), String>>,
    close_intent: Option<Option<u64>>,
    close_after_save: Option<Option<u64>>,
    exit_approved: bool,
    _native_menu: Option<menu::NativeMenu>,
    menu_events: mpsc::Receiver<String>,
    signature_prompt: Option<signatures::Prompt>,
    signature_busy: bool,
    signature_tx: mpsc::Sender<signatures::SignatureJob>,
    signature_rx: mpsc::Receiver<signatures::SignatureJob>,
    header_footer: header_footer::HeaderFooterState,
    redaction: redaction::Redaction,
    forms: form_tools::FormState,
    /// Compare Documents: choosing the versions, then the side-by-side view.
    compare_chooser: Option<comparison::Chooser>,
    comparison: Option<comparison::Comparison>,
    /// Zen mode: only the pages, full screen.
    zen: Option<zen::Zen>,
    details: details::DetailsState,
    reading_memory: reading::Memory,
    workspace: workspace::State,
    review: review::State,
    navigation_editor: Option<navigation_editor::Draft>,
    translation: translation::State,
}
impl Viewer {
    pub fn new(cc: &eframe::CreationContext<'_>, paths: Vec<PathBuf>) -> Self {
        #[cfg(target_os = "macos")]
        window_drag::configure(cc);
        editing::install_annotation_font(&cc.egui_ctx);
        ui::install_theme(&cc.egui_ctx);
        let (native_menu, menu_events) = menu::install(cc.egui_ctx.clone());
        let (save_tx, save_events) = mpsc::channel();
        let (signature_tx, signature_rx) = mpsc::channel();
        let mut app = Self {
            productivity: productivity::State::default(),
            workbench: workbench::State::default(),
            home: home::Home::new(&cc.egui_ctx),
            content: content::ContentState::new(),
            organizer: organize::Organizer::new(),
            editor: editing::EditorState::new(),
            digital: digital::DigitalState::new(),
            sign_tools: sign_tools::State::default(),
            tabs: vec![],
            active: 0,
            tab_revealed: None,
            tab_strip_width: 0.,
            tab_controls: tabs::State::default(),
            status_context: None,
            status_seen: None,
            next_id: 0,
            tick: 0,
            sidebar: true,
            inspector: true,
            accessible_text: false,
            recovery: recovery::Recovery::new(&cc.egui_ctx),
            search: false,
            outline: false,
            save_name: None,
            save_intent: save::Intent::Automatic,
            saving_adopts: false,
            link_confirm: None,
            protect: None,
            unprotect: false,
            shrink: None,
            watermark: None,
            nup: None,
            ocr_confirm: false,
            ocr_progress: None,
            focus_search: false,
            status: None,
            saving: false,
            save_events,
            save_tx,
            close_intent: None,
            close_after_save: None,
            exit_approved: false,
            _native_menu: native_menu,
            menu_events,
            signature_prompt: None,
            signature_busy: false,
            signature_tx,
            signature_rx,
            header_footer: header_footer::HeaderFooterState::new(),
            redaction: redaction::Redaction::new(),
            forms: form_tools::FormState::new(),
            compare_chooser: None,
            comparison: None,
            zen: None,
            details: details::DetailsState::new(),
            reading_memory: Default::default(),
            workspace: workspace::State::new(),
            review: review::State::default(),
            navigation_editor: None,
            translation: translation::State::new(),
        };
        app.restore_tab_groups();
        if paths.is_empty() {
            app.restore_workspace(&cc.egui_ctx);
        }
        for path in paths {
            app.open(path, &cc.egui_ctx);
        }
        app
    }
    fn open(&mut self, path: PathBuf, ctx: &egui::Context) {
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        if let Some(i) = self.tabs.iter().position(|t| t.path == path) {
            self.active = i;
            self.home.visible = false;
            if self.tabs[i].bytes.is_some() && self.tabs[i].path.is_file() {
                self.home.remember(&self.tabs[i].path);
            }
            return;
        }
        if self.tabs.len() >= 16 {
            self.status = Some((
                "Close a tab before opening another PDF (16-tab limit).".into(),
                true,
            ));
            return;
        }
        self.next_id += 1;
        let wake = ctx.clone();
        let worker = Worker::open(path.clone(), move || wake.request_repaint());
        let wake = ctx.clone();
        let search_worker = viewer::SearchWorker::new(move || wake.request_repaint());
        let saved_view = self.home.reading(&path);
        let restore_view = saved_view.is_some();
        let view_preferences = saved_view.unwrap_or_default();
        self.tabs.push(Tab {
            document_group: self.tab_controls.current_group,
            placement_guides: None,
            id: self.next_id,
            has_destination: true,
            path,
            worker,
            bytes: None,
            pages: vec![],
            signed: false,
            signed_original: None,
            signatures: Default::default(),
            unsaved_on_load: false,
            remember_open: true,
            session: None,
            capabilities: None,
            editing_warning: None,
            selected_annotation: None,
            group: vec![],
            error: None,
            page: 0,
            page_input: "1".into(),
            document_view: None,
            document_layout: None,
            last_pinch: None,
            page_layout: reading::PageLayout::from_code(view_preferences.layout),
            restore_view,
            restore_scroll: false,
            view_preferences,
            back: vec![],
            forward: vec![],
            zoom: Zoom::Width,
            scale: 1.0,
            jump: Some((0, None)),
            text: vec![],
            search_worker,
            search_at: None,
            searching: false,
            jump_to_first_hit: false,
            query: String::new(),
            search_options: Default::default(),
            hits: vec![],
            hit: 0,
            indexed: 0,
            index_done: false,
            search_limited: false,
            search_dirty: false,
            rasters: HashMap::new(),
            page_reorders: vec![],
            stale: HashMap::new(),
            nav: None,
            locked: None,
            password: None,
            keep_stale: false,
            pending: HashSet::new(),
            render_errors: HashMap::new(),
        });
        self.active = self.tabs.len() - 1;
        self.home.visible = false;
        self.status = None;
    }
    fn native_pick(&self, dialog: rfd::FileDialog) -> Option<PathBuf> {
        menu::with_file_dialog(self._native_menu.as_ref(), || dialog.pick_file())
    }
    fn native_picks(&self, dialog: rfd::FileDialog) -> Option<Vec<PathBuf>> {
        menu::with_file_dialog(self._native_menu.as_ref(), || dialog.pick_files())
    }
    fn native_save(&self, dialog: rfd::FileDialog) -> Option<PathBuf> {
        menu::with_file_dialog(self._native_menu.as_ref(), || dialog.save_file())
    }
    fn text_command(ctx: &egui::Context, key: egui::Key, shift: bool) {
        let modifiers = egui::Modifiers {
            shift,
            ..egui::Modifiers::COMMAND
        };
        ctx.input_mut(|i| {
            i.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            })
        });
    }
    fn open_dialog(&mut self, ctx: &egui::Context) {
        if let Some(paths) = self.native_picks(
            rfd::FileDialog::new()
                .set_title("Open PDF")
                .add_filter("PDF documents", &["pdf"]),
        ) {
            for path in paths {
                self.open(path, ctx);
            }
        }
    }
    fn save_dialog(&mut self, ctx: &egui::Context) {
        if let Some(id) = self.tabs.get(self.active).map(|t| t.id)
            && self.pending_form_edits(id)
        {
            self.focus_pending_form_edits(id);
            self.status = Some((
                "Apply or discard the pending form changes before saving.".into(),
                false,
            ));
            return;
        }
        if let Some(id) = self
            .content
            .pending_text()
            .filter(|id| self.tabs.get(self.active).is_some_and(|t| t.id == *id))
            && !self.content.busy
        {
            self.content.after_text = Some(content::AfterText::Save(id, self.save_intent));
            self.content.new_commit = true;
            self.finish_new_text(ctx);
            return;
        }
        if self.unfinished_edit(self.tabs.get(self.active).map(|t| t.id)) {
            return;
        }
        if self.content.busy || self.organizer.busy {
            self.status = Some((
                "Wait for the current operation before saving.".into(),
                false,
            ));
            return;
        }
        if !self.apply_pending_annotation() {
            return;
        }
        if self.saving {
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else {
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
        let tab_id = tab.id;
        let revision = tab.session.as_ref().map_or(0, |s| s.revision());
        let annotations = tab
            .session
            .as_ref()
            .map(|s| s.annotations.clone())
            .unwrap_or_default();
        let signed_original = tab.signed_original.clone();
        let password = tab.password.clone();
        let stem = tab.path.file_stem().unwrap_or_default().to_string_lossy();
        let adopts = self.save_intent.adopts();
        let name = self.save_name.take().unwrap_or_else(|| {
            if !tab.has_destination {
                tab.name()
            } else {
                format!("{stem}-copy.pdf")
            }
        });
        let dialog = rfd::FileDialog::new()
            .set_title(if adopts {
                "Save PDF • choose a new filename"
            } else {
                "Save a Copy • source stays open"
            })
            .add_filter("PDF document", &["pdf"])
            .set_file_name(name)
            .set_directory(tab.path.parent().unwrap_or(std::path::Path::new(".")));
        let Some(destination) = self.native_save(dialog) else {
            self.close_after_save = None;
            self.cancel_tab_closes();
            self.save_intent = save::Intent::Automatic;
            return;
        };
        self.saving_adopts = adopts;
        self.save_intent = save::Intent::Automatic;
        self.saving = true;
        self.status = Some(("Saving a complete copy…".into(), false));
        let tx = self.save_tx.clone();
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<_> {
                let bytes = if revision == 0 {
                    bytes.to_vec()
                } else {
                    mypdf_milestone0::annotations::export(&bytes, &annotations)?
                };
                // Signed bytes stay untouched: edits are appended as a new revision.
                let bytes = match signed_original {
                    Some(original) => {
                        mypdf_milestone0::signature_edit::incremental_update(&original, &bytes)?
                    }
                    None => bytes,
                };
                // A protected document's copies keep its password.
                let bytes = match &password {
                    Some(password) => mypdf_milestone0::security::protect(&bytes, password)?,
                    None => bytes,
                };
                viewer::save_copy(&destination, &bytes)?;
                Ok((destination, tab_id, revision))
            })()
            .map_err(|e| format!("Could not save: {e:#}. Existing files are preserved."));
            let _ = tx.send(result);
            wake.request_repaint();
        });
    }
    fn close(&mut self, index: usize) {
        if index < self.tabs.len() {
            let t = &self.tabs[index];
            if t.path.is_absolute() && t.path.is_file() {
                self.workspace.closed.push(workspace::SavedTab {
                    document_group: t.document_group,
                    path: t.path.clone(),
                    view: t.view_preferences.clone(),
                });
                if self.workspace.closed.len() > 16 {
                    self.workspace.closed.remove(0);
                }
            }
            self.recovery.clear(self.tabs[index].id);
            self.clear_form_drafts(self.tabs[index].id);
            let closed_group = self.tabs[index].document_group;
            let was_active = self.active == index;
            self.tabs.remove(index);
            if self.active > index {
                self.active -= 1;
            }
            self.active = self.active.min(self.tabs.len().saturating_sub(1));
            if was_active {
                self.home.visible = true;
                self.switch_group(closed_group);
            }
            if self.tabs.is_empty() {
                self.home.visible = true;
            }
        }
    }
    fn action(&mut self, action: &str, ctx: &egui::Context) {
        if self.productivity.owns_input()
            && !matches!(
                action,
                "workflow_settings" | "templates" | "batch" | "sharing_copy"
            )
        {
            match action {
                "copy" => ctx.send_viewport_cmd(egui::ViewportCommand::RequestCopy),
                "cut" => ctx.send_viewport_cmd(egui::ViewportCommand::RequestCut),
                "paste" => ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste),
                "select_all" => Self::text_command(ctx, egui::Key::A, false),
                "edit_undo" | "edit_redo" if ctx.wants_keyboard_input() => {
                    Self::text_command(ctx, egui::Key::Z, action == "edit_redo")
                }
                _ => {}
            }
            return;
        }
        if self.productivity_action(action, ctx) {
            return;
        }
        if action == "settings" {
            self.digital.save_if_dirty();
            self.digital.open = true;
            self.digital.panel = digital::Panel::Ink;
            return;
        }
        if action == "paste" && self.editor.signature_open {
            let clip = clipboard::read();
            if matches!(clip, clipboard::Clip::Image(..)) || !ctx.wants_keyboard_input() {
                self.import_signature_clipboard(clip);
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
            }
            return;
        }
        if action == "paste" && self.live_text_has_focus(ctx) {
            if let clip @ clipboard::Clip::Image(..) = clipboard::read() {
                self.paste_image_over_text(ctx, clip);
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
            }
            return;
        }
        if self.context_native_action(action, ctx) {
            return;
        }
        if self.workbench_action(action, ctx) {
            return;
        }
        if self.home.visible
            && matches!(
                action,
                "save"
                    | "save_as"
                    | "save_copy"
                    | "print"
                    | "protect"
                    | "unprotect"
                    | "shrink"
                    | "ocr"
                    | "watermark"
                    | "close"
                    | "undo"
                    | "redo"
                    | "zoom_in"
                    | "zoom_out"
                    | "actual"
                    | "fit_width"
                    | "fit_page"
                    | "edit_text"
                    | "edit_images"
                    | "hf_page_numbers"
                    | "hf_bates"
                    | "hf_text"
                    | "hf_remove_numbers"
                    | "hf_remove_all"
                    | "redact"
                    | "flatten"
            )
        {
            return;
        }
        match action {
            "translation" => self.open_translation_panel(),
            "translate_selection" => self.translate_selection(ctx),
            "translate_page" => self.translate_page(ctx),
            "translation_languages" => self.translation_languages(ctx),
            "home" => self.show_home(),
            "new" => self.new_blank(ctx),
            "open" => self.open_dialog(ctx),
            "save" | "save_as" | "save_copy" => {
                if self.content.busy || self.organizer.busy {
                    self.status = Some((
                        "Wait for the current operation before saving.".into(),
                        false,
                    ));
                } else if !self.saving {
                    self.save_intent = match action {
                        "save_as" => save::Intent::As,
                        "save_copy" => save::Intent::Copy,
                        _ => save::Intent::Automatic,
                    };
                    self.save_dialog(ctx);
                }
            }
            "nup" => self.open_nup(),
            "close" => self.request_close(self.tabs.get(self.active).map(|t| t.id), ctx),
            "quit" => self.request_close(None, ctx),
            "identities" => {
                self.digital.open = true;
                self.digital.panel = digital::Panel::Identities;
            }
            "sign_manage" => {
                self.workbench.paste = None;
                self.editor.signature_open = false;
                self.editor.pending_sign_rect = None;
                self.editor.select_tool(editing::Tool::Select);
                self.action("settings", ctx);
            }
            "sign_ink" | "sign_digital" | "sign_quick" => self.signing_ribbon_action(action),
            "quick_sign" => self.quick_sign(),
            "edit_text" | "edit_images" => {
                self.editor.mode = editing::Mode::Edit;
                self.content.image_mode = action == "edit_images";
                self.inspector = true;
            }
            "hf_page_numbers" => {
                self.open_header_footer(mypdf_milestone0::header_footer::Kind::PageNumber)
            }
            "hf_bates" => self.open_header_footer(mypdf_milestone0::header_footer::Kind::Bates),
            "hf_text" => self.open_header_footer(mypdf_milestone0::header_footer::Kind::Text),
            "hf_remove_numbers" => self.request_remove_page_numbers(),
            "redact" => self.start_redaction(),
            "flatten" => self.request_flatten(),
            "print" => self.print_document(ctx),
            "compare" => self.request_compare(),
            "zen" => self.toggle_zen(ctx),
            "form_fill_details" => {
                self.inspector = true;
                self.fill_with_details();
            }
            "my_details" => self.edit_details(false),
            "protect" => self.request_protect(),
            "unprotect" => self.request_unprotect(),
            "shrink" => self.request_shrink(),
            "ocr" => self.request_ocr(),
            "watermark" => {
                self.editor.mode = editing::Mode::Edit;
                self.open_watermark();
            }
            "form_detect" | "form_clear" | "form_tab_rows" | "form_tab_columns"
            | "form_flatten" | "form_import" | "form_export" => {
                self.inspector = true;
                self.form_command(action, ctx);
            }
            "hf_remove_all" => {
                self.remove_header_footer(ctx, mypdf_milestone0::header_footer::Kind::ALL.to_vec())
            }
            "theme_system" => ui::set_theme_preference(ctx, egui::ThemePreference::System),
            "theme_light" => ui::set_theme_preference(ctx, egui::ThemePreference::Light),
            "theme_dark" => ui::set_theme_preference(ctx, egui::ThemePreference::Dark),
            "appearance" => {
                self.digital.open = true;
                self.digital.panel = digital::Panel::Signatures;
            }
            "sign" => {
                self.digital.open = true;
                self.digital.panel = digital::Panel::Sign;
                self.digital.use_document(self.tabs.get(self.active));
            }
            "validate" => {
                self.digital.open = true;
                self.digital.panel = digital::Panel::Validate;
            }
            "copy" if self.pages_clipboard_keys(ctx) => self.copy_pages(ctx),
            "copy" if self.page_text_selection(ctx).is_some() => self.copy_page_text(ctx),
            "copy" | "cut" if self.marks_keys(ctx) && self.copy_marks(ctx, action == "cut") => {}
            "paste" if self.paste_keys(ctx) => self.paste(ctx),
            "add_image" => {
                self.editor.mode = editing::Mode::Edit;
                self.add_image_file(ctx);
            }
            "select_all"
                if self.editor.mode == editing::Mode::Read
                    && !ctx.wants_keyboard_input()
                    && !self.editor.text_editing =>
            {
                self.editor.text_selection = self
                    .tabs
                    .get(self.active)
                    .and_then(|t| text_selection::TextSelection::all(t, t.page));
            }
            "paste" if self.pages_clipboard_keys(ctx) => self.paste_pages(ctx),
            "cut" => ctx.send_viewport_cmd(egui::ViewportCommand::RequestCut),
            "copy" => ctx.send_viewport_cmd(egui::ViewportCommand::RequestCopy),
            "paste" => ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste),
            "select_all" => Self::text_command(ctx, egui::Key::A, false),
            "edit_undo" | "edit_redo" => {
                if self
                    .tabs
                    .get(self.active)
                    .is_some_and(|tab| self.content.placement_undo_owns_input(ctx, tab.id))
                {
                    self.editor_undo(action == "edit_redo");
                } else if ctx.wants_keyboard_input() {
                    Self::text_command(ctx, egui::Key::Z, action == "edit_redo");
                } else if !self.home.visible {
                    self.editor_undo(action == "edit_redo");
                }
            }
            "undo" => self.editor_undo(false),
            "redo" => self.editor_undo(true),
            "find" if self.home.visible => self.home.focus_search = true,
            "find" => {
                self.editor.annotation_sidebar = false;
                self.search = true;
                self.sidebar = true;
                self.focus_search = true;
            }
            "sidebar" => self.sidebar = !self.sidebar,
            "inspector" => self.inspector = !self.inspector,
            "recovery" => self.recovery.open = true,
            "page_text" => self.accessible_text = !self.accessible_text,
            "zoom_in" | "zoom_out" | "actual" | "fit_width" | "fit_page" => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.zoom = match action {
                        "actual" => Zoom::Scale(1.0),
                        "fit_width" => Zoom::Width,
                        "fit_page" => Zoom::Page,
                        "zoom_in" => Zoom::Scale((tab.scale * 1.2).min(6.0)),
                        _ => Zoom::Scale((tab.scale / 1.2).max(0.1)),
                    };
                    tab.jump = Some((tab.page, None));
                }
            }
            _ => {}
        }
    }
    fn events(&mut self, ctx: &egui::Context) {
        for tab in &mut self.tabs {
            for event in tab.worker.events.try_iter().take(16).collect::<Vec<_>>() {
                match event {
                    Event::Loaded {
                        bytes,
                        pages,
                        signed,
                        annotations,
                        capabilities,
                        editing_warning,
                    } => {
                        if std::mem::take(&mut tab.remember_open) && !tab.unsaved_on_load {
                            self.home.remember(&tab.path);
                        }
                        let continuing_view =
                            tab.document_layout.is_some() && tab.pages.len() == pages.len();
                        tab.text.resize_with(pages.len(), || None);
                        tab.pages = pages;
                        if tab.session.is_none() {
                            tab.session = Some(mypdf_milestone0::annotations::Session::new(
                                bytes.clone(),
                                annotations,
                                capabilities.rewritable() && editing_warning.is_none(),
                            ));
                        }
                        if tab.unsaved_on_load {
                            if let Some(s) = &mut tab.session {
                                s.mark_unsaved();
                            }
                            tab.unsaved_on_load = false;
                        }
                        if std::mem::take(&mut tab.restore_view) {
                            let v = &tab.view_preferences;
                            tab.zoom = v
                                .scale
                                .filter(|v| v.is_finite())
                                .map(|v| Zoom::Scale(v.clamp(0.1, 6.)))
                                .unwrap_or(if v.fit_page { Zoom::Page } else { Zoom::Width });
                            tab.page = v.page.min(tab.pages.len().saturating_sub(1));
                            tab.page_input = (tab.page + 1).to_string();
                            tab.jump = Some((
                                tab.page,
                                Some(v.top.max(0.).min(tab.pages[tab.page].height)),
                            ));
                        } else if !continuing_view {
                            tab.jump(tab.page.min(tab.pages.len().saturating_sub(1)));
                        }
                        tab.bytes = Some(bytes);
                        if let Some(session) = &mut tab.session {
                            session.editable =
                                capabilities.rewritable() && editing_warning.is_none();
                        }
                        tab.capabilities = Some(capabilities);
                        tab.editing_warning = editing_warning;
                        tab.signed = signed;
                        if self.recovery.loaded(tab.id) {
                            self.status =
                                Some(("Recovered your changes • Save when ready".into(), false));
                        }
                    }
                    Event::Indexed { page, text } => {
                        tab.text[page] = Some(Arc::new(text));
                        tab.indexed += 1;
                        tab.search_dirty = true;
                    }
                    Event::IndexFinished { limited } => {
                        tab.index_done = true;
                        tab.search_limited = limited;
                    }
                    Event::Raster { key, size, rgba } => {
                        tab.pending.remove(&key);
                        if !key.thumbnail {
                            tab.stale.remove(&key.page);
                        }
                        let bytes = rgba.len();
                        let image = egui::ColorImage::from_rgba_premultiplied(size, &rgba);
                        let texture = ctx.load_texture(
                            format!("{}-{}-{}-{}", tab.id, key.page, key.scale, key.thumbnail),
                            image,
                            egui::TextureOptions::LINEAR,
                        );
                        tab.rasters.insert(
                            key,
                            Raster {
                                texture,
                                bytes,
                                used: self.tick,
                            },
                        );
                    }
                    Event::RasterFailed { key, error } => {
                        tab.pending.remove(&key);
                        tab.render_errors.insert(key.page, error);
                    }
                    Event::PasswordNeeded(encrypted) => {
                        tab.locked = Some(protect::Locked::new(encrypted));
                    }
                    Event::Failed(error) => {
                        tab.pending.clear();
                        self.recovery.failed(tab.id, &error);
                        tab.error = Some(error);
                    }
                }
            }
            tab.search(ctx);
            if tab.error.is_none()
                && (!tab.pending.is_empty()
                    || tab.bytes.is_none() && tab.error.is_none()
                    || !tab.index_done)
            {
                ctx.request_repaint_after(std::time::Duration::from_millis(80));
            }
        }
        let saved: Vec<_> = self.save_events.try_iter().collect();
        for saved in saved {
            self.saving = false;
            self.status = Some(match saved {
                Ok((path, id, revision)) => {
                    self.finish_save(&path, id, revision, self.saving_adopts);
                    if let Some(intent) = self.close_after_save.take() {
                        self.request_close(intent, ctx);
                    }
                    (
                        format!(
                            "{} {} • original file preserved",
                            if self.saving_adopts {
                                "Saved"
                            } else {
                                "Exported a copy to"
                            },
                            path.display()
                        ),
                        false,
                    )
                }
                Err(e) => {
                    self.close_after_save = None;
                    self.cancel_tab_closes();
                    (e, true)
                }
            });
        }
        let actions: Vec<_> = self.menu_events.try_iter().collect();
        for action in actions {
            self.action(&action, ctx);
        }
        let dropped = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect::<Vec<_>>()
        });
        for path in dropped {
            self.open(path, ctx);
        }
    }
    /// In Pages, Copy and Paste act on whole pages unless a text field has focus.
    fn pages_clipboard_keys(&self, ctx: &egui::Context) -> bool {
        !self.home.visible
            && self.editor.mode == editing::Mode::Pages
            && !ctx.wants_keyboard_input()
            && !self.editor.text_editing
    }
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.productivity.owns_input() {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Comma)) {
            self.action("settings", ctx);
            return;
        }
        if self.editor.signature_open {
            if content::context::paste_requested(ctx) {
                let clip = clipboard::read();
                if matches!(clip, clipboard::Clip::Image(..))
                    || self.signature_image_paste_active(ctx)
                {
                    content::context::consume_action(ctx, "paste");
                    self.import_signature_clipboard(clip);
                }
            }
            return;
        }
        if self.live_text_has_focus(ctx)
            && content::context::paste_requested(ctx)
            && let clip @ clipboard::Clip::Image(..) = clipboard::read()
        {
            content::context::consume_action(ctx, "paste");
            self.paste_image_over_text(ctx, clip);
            return;
        }
        if self.context_shortcuts(ctx) {
            return;
        }
        if self.workbench_keys(ctx) {
            return;
        }
        if self.pages_clipboard_keys(ctx) {
            let (copy, paste) = ctx.input_mut(|i| {
                let copy = i.events.iter().any(|e| matches!(e, egui::Event::Copy));
                let paste = i.events.iter().any(|e| matches!(e, egui::Event::Paste(_)));
                i.events
                    .retain(|e| !matches!(e, egui::Event::Copy | egui::Event::Paste(_)));
                (copy, paste)
            });
            if copy {
                self.copy_pages(ctx);
            }
            if paste {
                self.paste_pages(ctx);
            }
        } else if self.page_text_selection(ctx).is_some()
            && ctx.input_mut(|i| {
                let copy = i.events.iter().any(|e| matches!(e, egui::Event::Copy));
                i.events.retain(|e| !matches!(e, egui::Event::Copy));
                copy
            })
        {
            self.copy_page_text(ctx);
        } else if self.marks_keys(ctx) || self.paste_keys(ctx) {
            // Image-only clipboards may emit Command/Ctrl-V without a Paste
            // event (the window backend can only provide pasted text).
            let paste = self.paste_keys(ctx) && content::context::consume_action(ctx, "paste");
            let (copy, cut) = ctx.input(|i| {
                (
                    i.events.iter().any(|e| matches!(e, egui::Event::Copy)),
                    i.events.iter().any(|e| matches!(e, egui::Event::Cut)),
                )
            });
            if (copy || cut) && self.marks_keys(ctx) {
                self.copy_marks(ctx, cut);
            }
            if paste && self.paste_keys(ctx) {
                self.paste(ctx);
            }
        }
        if !self.home.visible
            && !self.digital.open
            && !self.editor.signature_open
            && !self.editor.library_open
            && self
                .tabs
                .get(self.active)
                .is_some_and(|tab| self.content.placement_undo_owns_input(ctx, tab.id))
        {
            if ctx.input_mut(|i| {
                i.consume_key(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::Z,
                )
            }) {
                self.editor_undo(true);
            } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)) {
                self.editor_undo(false);
            }
        }
        if !self.home.visible
            && !ctx.wants_keyboard_input()
            && !self.editor.text_editing
            && !self.digital.open
            && !self.editor.signature_open
            && !self.editor.library_open
        {
            if ctx.input_mut(|i| {
                i.consume_key(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::Z,
                )
            }) {
                self.editor_undo(true);
            } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)) {
                self.editor_undo(false);
            }
            if matches!(
                self.editor.mode,
                editing::Mode::Annotate | editing::Mode::Draw
            ) {
                self.selection_keys(ctx);
            }
        }
        // Before ⌘F, which would also match ⇧⌘F.
        if ctx.input_mut(|i| {
            i.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::F,
            )
        }) {
            self.toggle_zen(ctx);
        }
        // macOS menu accelerators own these shortcuts to prevent duplicate dialogs.
        if !cfg!(target_os = "macos") || self._native_menu.is_none() {
            if ctx.input_mut(|i| {
                i.consume_key(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::S,
                )
            }) {
                self.action("save_as", ctx);
            }
            for (key, action) in [
                (egui::Key::N, "new"),
                (egui::Key::O, "open"),
                (egui::Key::S, "save"),
                (egui::Key::P, "print"),
                (egui::Key::W, "close"),
                (egui::Key::F, "find"),
                (egui::Key::Equals, "zoom_in"),
                (egui::Key::Minus, "zoom_out"),
                (egui::Key::Num0, "actual"),
            ] {
                if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, key)) {
                    self.action(action, ctx);
                }
            }
        }
        for (key, action) in [
            (egui::Key::L, "sidebar"),
            (egui::Key::R, "inspector"),
            (egui::Key::H, "home"),
        ] {
            if ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, key)
            }) {
                self.action(action, ctx);
            }
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Tab))
            && !self.tabs.is_empty()
        {
            self.cycle_group_tab(false);
        }
        if ctx.input_mut(|i| {
            i.consume_key(
                egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
                egui::Key::Tab,
            )
        }) && !self.tabs.is_empty()
        {
            self.cycle_group_tab(true);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.search = false;
        }
        self.reading_keys(ctx);
    }

    fn toolbar(&mut self, ctx: &egui::Context) {
        self.advance_tab_closes(ctx);
        self.tab_group_dialog(ctx);
        if self.zen.is_some() {
            return;
        }
        let mut action = None;
        egui::TopBottomPanel::top("document_tabs")
            .frame(
                egui::Frame::new()
                    .fill(theme::chrome())
                    .inner_margin(egui::Margin::symmetric(12, 6)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.;
                    #[cfg(target_os = "macos")]
                    ui.add_space(70.);
                    let (mark, move_handle) =
                        ui.allocate_exact_size(vec2(22., 22.), egui::Sense::drag());
                    ui::logo(ui.painter(), mark);
                    if move_handle.drag_started() {
                        self.tab_controls.request_window_drag();
                    }
                    if ui.available_width() > 960. {
                        let move_handle = ui.add(
                            egui::Label::new(
                                RichText::new("MyPDF")
                                    .font(ui::bold(ui, 14.))
                                    .color(theme::ink()),
                            )
                            .sense(egui::Sense::drag()),
                        );
                        if move_handle.drag_started() {
                            self.tab_controls.request_window_drag();
                        }
                    }
                    #[cfg(not(target_os = "macos"))]
                    ui.menu_button("Tools", |ui| {
                        for (id, label) in menu::WORKFLOW_ITEMS {
                            if ui.button(*label).clicked() {
                                action = Some(*id);
                                ui.close();
                            }
                        }
                    });
                    let mut home = ActionButton::new(Icon::Home, "Home")
                        .tab()
                        .selected(self.home.visible);
                    if ui.available_width() < 850. {
                        home = home.compact();
                    }
                    if ui.add(home).on_hover_text("Home · ⇧⌘H").clicked() {
                        action = Some("home");
                    }
                    self.document_tabs(ui, ctx);
                    if ui
                        .add(ActionButton::new(Icon::Plus, "Open PDF").compact())
                        .clicked()
                    {
                        action = Some("open");
                    }
                    if ui
                        .add(ActionButton::new(Icon::Settings, "Settings").compact())
                        .on_hover_text(if cfg!(target_os = "macos") {
                            "Settings · ⌘,"
                        } else {
                            "Settings · Ctrl+,"
                        })
                        .clicked()
                    {
                        action = Some("settings");
                    }
                    if let Some(choice) = ui::theme_menu(ui) {
                        action = Some(choice);
                    }
                });
            });
        if let Some(a) = action.take() {
            self.action(a, ctx);
        }
        if self.home.visible || self.comparison.is_some() {
            return;
        }
        egui::TopBottomPanel::top("main_toolbar")
            .frame(
                egui::Frame::new()
                    .fill(theme::toolbar())
                    .inner_margin(egui::Margin::symmetric(12, 6)),
            )
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.x = 4.;
                const MODES: [(editing::Mode, Icon, &str); 8] = [
                    (editing::Mode::Read, Icon::Read, "Read"),
                    (editing::Mode::Annotate, Icon::Pen, "Annotate"),
                    (editing::Mode::Edit, Icon::Text, "Edit"),
                    (editing::Mode::Draw, Icon::Shapes, "Draw"),
                    (editing::Mode::Forms, Icon::Form, "Forms"),
                    (editing::Mode::Sign, Icon::Sign, "Sign"),
                    (editing::Mode::Pages, Icon::Library, "Pages"),
                    (editing::Mode::Convert, Icon::Export, "Export"),
                ];
                let track_width = |ui: &egui::Ui, compact: bool| {
                    MODES
                        .iter()
                        .map(|(_, icon, label)| {
                            let mut b = ActionButton::new(*icon, label).tab();
                            if compact {
                                b = b.compact();
                            }
                            b.width(ui)
                        })
                        .sum::<f32>()
                        + 2. * (MODES.len() - 1) as f32
                        + 6.
                };
                let full = ui.available_width();
                // Reserve space for file actions and the search, translation and panel controls.
                let compact = full < track_width(ui, false) + 356.;
                ui.horizontal(|ui| {
                    for (icon, label, key) in [
                        (Icon::Blank, "New blank PDF", "new"),
                        (Icon::Open, "Open…", "open"),
                        (Icon::Save, "Save PDF…", "save"),
                        (Icon::Print, "Print… (⌘P)", "print"),
                        (Icon::Compare, "Compare Documents…", "compare"),
                    ] {
                        let enabled = !matches!(key, "save" | "print")
                            || (!self.saving
                                && self
                                    .tabs
                                    .get(self.active)
                                    .is_some_and(|t| t.bytes.is_some()));
                        if ui
                            .add_enabled(enabled, ActionButton::new(icon, label).compact())
                            .clicked()
                        {
                            action = Some(key);
                        }
                    }
                    let used = ui.min_rect().width();
                    let track = track_width(ui, compact);
                    let centered = (full - track) / 2. - used;
                    let room = full - track - used - 186.;
                    ui.add_space(centered.min(room).max(8.));
                    ui::segment_track(ui, |ui| {
                        for (mode, icon, label) in MODES {
                            let button = ActionButton::new(icon, label)
                                .tab()
                                .selected(self.editor.mode == mode);
                            if ui
                                .add(if compact { button.compact() } else { button })
                                .clicked()
                                && self.apply_pending_annotation()
                            {
                                self.editor.mode = mode;
                                self.editor.library_open = false;
                                if let Some(tab) = self.tabs.get_mut(self.active) {
                                    tab.signatures.panel = false;
                                }
                            }
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(ActionButton::new(Icon::Zen, "Zen mode · ⇧⌘F").compact())
                            .clicked()
                        {
                            action = Some("zen");
                        }
                        if ui
                            .add(
                                ActionButton::new(Icon::Inspector, "Toggle right panel")
                                    .compact()
                                    .selected(self.inspector),
                            )
                            .clicked()
                        {
                            self.inspector = !self.inspector;
                        }
                        if ui
                            .add(
                                ActionButton::new(Icon::Sidebar, "Toggle left panel")
                                    .compact()
                                    .selected(self.sidebar),
                            )
                            .clicked()
                        {
                            self.sidebar = !self.sidebar;
                        }
                        if ui
                            .add(
                                ActionButton::new(Icon::Translate, "Translate")
                                    .compact()
                                    .selected(self.translation.open),
                            )
                            .on_hover_text("Translate text or the current page on this device")
                            .clicked()
                        {
                            action = Some("translation");
                        }
                        if ui
                            .add(ActionButton::new(Icon::Search, "Find").compact())
                            .clicked()
                        {
                            action = Some("find");
                        }
                    });
                });
            });
        egui::TopBottomPanel::bottom("document_navigation")
            .frame(
                egui::Frame::new()
                    .fill(theme::toolbar())
                    .inner_margin(egui::Margin::symmetric(12, 4)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 5.;
                    if let Some(tab) = self.tabs.get_mut(self.active).filter(|t| !t.pages.is_empty() && t.error.is_none()) {
                        if ui
                            .add(ActionButton::new(Icon::Minus, "Zoom out").compact())
                            .clicked()
                        {
                            action = Some("zoom_out");
                        }
                        egui::ComboBox::from_id_salt("zoom")
                            .width(82.0)
                            .selected_text(match tab.zoom {
                                Zoom::Width => "Fit width".into(),
                                Zoom::Page => "Fit page".into(),
                                Zoom::Scale(s) => format!("{}%", (s * 100.0).round() as i32),
                            })
                            .show_ui(ui, |ui| {
                                for (label, mode) in [
                                    ("Fit width", Zoom::Width),
                                    ("Fit page", Zoom::Page),
                                    ("50%", Zoom::Scale(0.5)),
                                    ("75%", Zoom::Scale(0.75)),
                                    ("100%", Zoom::Scale(1.0)),
                                    ("150%", Zoom::Scale(1.5)),
                                    ("200%", Zoom::Scale(2.0)),
                                    ("400%", Zoom::Scale(4.0)),
                                ] {
                                    if ui.selectable_value(&mut tab.zoom, mode, label).clicked() {
                                        tab.jump = Some((tab.page, None));
                                        ui.close();
                                    }
                                }
                            });
                        if ui
                            .add(ActionButton::new(Icon::Plus, "Zoom in").compact())
                            .clicked()
                        {
                            action = Some("zoom_in");
                        }
                        ui.menu_button(tab.page_layout.label(), |ui| {
                            for mode in [reading::PageLayout::Continuous, reading::PageLayout::Single, reading::PageLayout::Facing] {
                                if ui.selectable_value(&mut tab.page_layout, mode, mode.label()).changed() {
                                    tab.jump = Some((tab.page, None)); ui.close();
                                }
                            }
                        });
                        ui.separator();
                        if ui.add_enabled(!tab.back.is_empty(), ActionButton::new(Icon::Undo, "Back to previous view").compact()).clicked() { tab.history_step(false); }
                        if ui.add_enabled(!tab.forward.is_empty(), ActionButton::new(Icon::Redo, "Forward to next view").compact()).clicked() { tab.history_step(true); }
                        ui.separator();
                        if ui
                            .add_enabled(
                                tab.page > 0,
                                ActionButton::new(Icon::Left, "Previous page").compact(),
                            )
                            .clicked()
                        {
                            tab.jump(tab.page.saturating_sub(1));
                        }
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut tab.page_input).margin(ui::FIELD_MARGIN)
                                .desired_width(30.0)
                                .horizontal_align(egui::Align::Center),
                        );
                        r.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::TextEdit,
                                true,
                                "Page number",
                            )
                        });
                        self.editor.text_editing |= r.has_focus();
                        if r.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            && let Ok(n) = tab.page_input.parse::<usize>()
                        {
                            tab.jump(n.saturating_sub(1));
                        }
                        ui.label(format!("/ {}", tab.pages.len()));
                        if let Some(label)=tab.nav.as_ref().and_then(|n|n.label(tab.page)) && label!=(tab.page+1).to_string() {ui.label(RichText::new(label).color(theme::accent())).on_hover_text("Page label");}
                        if ui
                            .add_enabled(
                                tab.page + 1 < tab.pages.len(),
                                ActionButton::new(Icon::Right, "Next page").compact(),
                            )
                            .clicked()
                        {
                            tab.jump(tab.page + 1);
                        }
                    }

                    ui.separator();
                    let session = self.tabs.get(self.active).and_then(|t| t.session.as_ref());
                    let pending_text = self.tabs.get(self.active).is_some_and(|tab| self.content.has_pending_text_change(tab.id));
                    if ui
                        .add_enabled(
                            !self.content.busy && !self.organizer.busy && (pending_text || session.is_some_and(|s| s.can_undo())),
                            ActionButton::new(Icon::Undo, &if pending_text { "Undo text edit".into() } else { session.and_then(|s| s.undo_label()).map(|s| format!("Undo {s}")).unwrap_or_else(|| "Undo".into()) }).compact(),
                        )
                        .clicked()
                    {
                        action = Some("undo");
                    }
                    if ui
                        .add_enabled(
                            !pending_text && !self.content.busy && !self.organizer.busy && session.is_some_and(|s| s.can_redo()),
                            ActionButton::new(Icon::Redo, &session.and_then(|s| s.redo_label()).map(|s| format!("Redo {s}")).unwrap_or_else(|| "Redo".into())).compact(),
                        )
                        .clicked()
                    {
                        action = Some("redo");
                    }
                    if ui.add(ActionButton::new(Icon::Search, "Find a tool (⌘K)").compact()).clicked() { action = Some("tool_search"); }
                    if ui.add(ActionButton::new(Icon::History, "Document history").compact()).clicked() { action = Some("history"); }
                    if ui.add(ActionButton::new(Icon::Guides, if self.editor.hide_guides { "Show editing guides" } else { "Hide editing guides" }).compact().selected(!self.editor.hide_guides)).clicked() { action = Some("guides"); }
                    if let Some(tab) = self.tabs.get(self.active)
                        && let Some(s) = &tab.session
                    {
                        let label = if self.saving { "Saving…" } else if s.dirty() { self.recovery.label(tab.id, s.revision()) } else { "Saved" };
                        let (dot, _) = ui.allocate_exact_size(vec2(8., 8.), egui::Sense::hover());
                        ui.painter().circle_filled(
                            dot.center(),
                            3.5,
                            if label == "Saved" || label.contains("copy saved") {
                                theme::success()
                            } else {
                                theme::warning()
                            },
                        );
                        ui.label(RichText::new(label).size(11.5).color(theme::muted())).on_hover_text(if s.dirty() { "Your PDF has unsaved changes. Save writes a new PDF and switches to it; Save a Copy exports without changing this document." } else { "No unsaved document changes." });
                    }
                    if let Some(tab) = self.tabs.get_mut(self.active) {
                        if tab.signed_original.is_some() && !tab.signatures.no_active_signatures() {
                            let pill = pill(ui, "Signed · editing", theme::warning())
                                .on_hover_text("Edits are saved after the signed version, so signatures stay valid for what they signed. Click for signature details.");
                            if pill.interact(egui::Sense::click()).clicked() {
                                tab.signatures.show_panel(true);
                            }
                        } else if tab.signed && !tab.signatures.no_active_signatures() {
                            let pill = pill(ui, "Signed PDF", theme::success())
                                .on_hover_text("Existing digital signatures are preserved. Use “Enable editing…” in the signature bar to edit. Click for signature details.");
                            if pill.interact(egui::Sense::click()).clicked() {
                                tab.signatures.show_panel(true);
                            }
                        } else if let Some(warning) = &tab.editing_warning {
                            pill(ui, "Read-only", theme::muted()).on_hover_text(warning);
                        }
                        if tab.password.is_some() {
                            pill(ui, "Protected", theme::accent()).on_hover_text(
                                "Opens with a password. Copies you save keep it; File → Remove Password… saves one without.",
                            );
                        }
                    }
                    if let Some(e) = &self.recovery.error {
                        if ui.small_button("Autosave issue").on_hover_text(e).clicked() {
                            self.recovery.open = true;
                        }
                    } else if ui.available_width() > 220. {
                        let message =
                            self.status
                                .as_ref()
                                .map(|(s, _)| s.clone())
                                .unwrap_or_else(|| self.tool_guidance().into());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add(
                                egui::Label::new(RichText::new(&message).size(11.5).color(theme::muted()))
                                    .truncate(),
                            )
                            .on_hover_text(message);
                        });
                    }
                });
            });
        if let Some(action) = action {
            self.action(action, ctx);
        }
    }
    fn sidebar(&mut self, ctx: &egui::Context) {
        const SIDEBAR_TABS: [(Icon, &str); 4] = [
            (Icon::Library, "Pages"),
            (Icon::Outline, "Outline"),
            (Icon::Search, "Search"),
            (Icon::Note, "Marks"),
        ];
        if !self.sidebar || self.tabs.is_empty() {
            return;
        }
        if self.editor.annotation_sidebar {
            egui::SidePanel::left("sidebar")
                .resizable(true)
                .default_width(240.0)
                .width_range(190.0..=360.0)
                .frame(egui::Frame::new().fill(theme::paper()).inner_margin(12.0))
                .show(ctx, |ui| {
                    match ui::segmented_icons(ui, &SIDEBAR_TABS, 3) {
                        Some(0) => {
                            self.editor.annotation_sidebar = false;
                            self.search = false;
                            self.outline = false;
                        }
                        Some(1) => {
                            self.editor.annotation_sidebar = false;
                            self.search = false;
                            self.outline = true;
                        }
                        Some(2) => {
                            self.editor.annotation_sidebar = false;
                            self.search = true;
                            self.outline = false;
                            self.focus_search = true;
                        }
                        _ => {}
                    }
                    ui.add_space(4.);
                    self.annotation_list(ui);
                });
            return;
        }
        let tab = &mut self.tabs[self.active];
        let tick = self.tick;
        egui::SidePanel::left("sidebar").resizable(true).default_width(220.0).width_range(190.0..=360.0).frame(egui::Frame::new().fill(theme::paper()).inner_margin(12.0)).show(ctx, |ui| {
            let current = if self.search { 2 } else if self.outline { 1 } else { 0 };
            match ui::segmented_icons(ui, &SIDEBAR_TABS, current) {
                Some(0) => { self.search = false; self.outline = false; }
                Some(1) => { self.search = false; self.outline = true; }
                Some(2) => { self.search = true; self.outline = false; self.focus_search = true; }
                Some(_) => self.editor.annotation_sidebar = true,
                None => {}
            }
            ui.add_space(4.);
            if self.search {
                let response = ui.add(egui::TextEdit::singleline(&mut tab.query).margin(ui::FIELD_MARGIN).hint_text("Find in document…").margin(vec2(10., 7.)).desired_width(f32::INFINITY));
                self.editor.text_editing |= response.has_focus();
                if self.focus_search { response.request_focus(); self.focus_search = false; }
                let mut options_changed = false;
                ui.horizontal(|ui| {
                    options_changed |= ui.toggle_value(&mut tab.search_options.case_sensitive, "Aa").on_hover_text("Match case").changed();
                    options_changed |= ui.checkbox(&mut tab.search_options.whole_word, "Whole words").changed();
                });
                if response.changed() || options_changed { tab.search_worker.invalidate(); tab.search_dirty = true; tab.search_at = Some(std::time::Instant::now()); tab.hits.clear(); tab.hit = 0; tab.jump_to_first_hit = true; tab.search(ctx); }
                if (response.has_focus() || response.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter)) { let next = if ui.input(|i| i.modifiers.shift) { tab.hit + tab.hits.len().saturating_sub(1) } else { tab.hit + 1 }; tab.select_hit(next); response.request_focus(); }
                ui.horizontal(|ui| {
                    let message = if tab.hits.is_empty() { "0 matches".into() } else { format!("{} of {}", tab.hit + 1, tab.hits.len()) };
                    ui.label(RichText::new(message).color(theme::muted()).size(12.0));
                    if ui.add_enabled(!tab.hits.is_empty(), ActionButton::new(Icon::Left, "Previous page").compact()).on_hover_text("Previous match • Shift+Enter").clicked() { tab.select_hit(tab.hit + tab.hits.len().saturating_sub(1)); }
                    if ui.add_enabled(!tab.hits.is_empty(), ActionButton::new(Icon::Right, "Next page").compact()).on_hover_text("Next match • Enter").clicked() { tab.select_hit(tab.hit + 1); }
                });
                if !tab.index_done { ui.label(RichText::new(format!("Indexing {}/{} pages…", tab.indexed, tab.pages.len())).color(theme::muted()).size(11.0)); }
                if tab.searching || tab.search_dirty && !tab.query.is_empty() { ui.label(RichText::new("Searching…").color(theme::muted()).size(11.0)); }
                if tab.search_limited { ui.label(RichText::new("Search is partial: this document reached the text index limit.").color(theme::muted()).size(12.0)); }
                if !tab.query.is_empty() && tab.hits.is_empty() && tab.index_done && !tab.search_limited && !tab.searching && !tab.search_dirty {
                    ui.label(RichText::new("No matches. If this is a scan, its words need to be recognized first.").color(theme::muted()).size(12.0));
                    if ui.add(ActionButton::new(Icon::Ocr, "Recognize Text…")).clicked() { self.ocr_confirm = true; }
                }
                let mut chosen = None;
                egui::ScrollArea::vertical().id_salt((tab.id, "search_results")).show(ui, |ui| {
                    for (i, hit) in tab.hits.iter().enumerate().take(1000) { let selected = i == tab.hit; egui::Frame::new().fill(if selected { theme::accent_soft() } else { Color32::TRANSPARENT }).stroke(Stroke::new(1., if selected { theme::accent_line() } else { Color32::TRANSPARENT })).corner_radius(8.0).inner_margin(8.0).show(ui, |ui| {
                        ui.label(RichText::new(format!("Page {}", hit.page + 1)).size(11.0).family(ui::semibold()).color(theme::accent()));
                        if ui.add(egui::Button::new(&hit.snippet).wrap().frame(false)).clicked() { chosen = Some(i); }
                    }); }
                });
                if let Some(index) = chosen { tab.select_hit(index); }
            } else if self.outline {
                if ui.add(ActionButton::new(Icon::Outline,"Edit bookmarks & labels…")).clicked(){self.workbench.request=Some("edit_navigation".into());}
                    navigate::outline_list(tab, ui, ctx);
            } else {
                ui.label(RichText::new(pages_label(tab.pages.len())).color(theme::muted()).size(11.5));
                egui::ScrollArea::vertical().id_salt((tab.id, "thumbnails")).show_rows(ui, 176.0, tab.pages.len(), |ui, range| {
                    for page in range { let size = tab.pages[page]; let factor = (132.0 / size.width).min(146.0 / size.height); let display = vec2(size.width * factor, size.height * factor); let key = viewer::raster_key(page, size, factor * ctx.pixels_per_point(), true); tab.request(key);
                        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 176.0), egui::Sense::click());
                        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, page == tab.page, format!("Go to page {}", page + 1)));
                        let image = Rect::from_center_size(pos2(rect.center().x, rect.top() + 77.0), display);
                        let current = page == tab.page;
                        ui::shadow(ui.painter(), image, 2, 1);
                        if current {
                            ui.painter().rect_stroke(image.expand(4.0), 6, Stroke::new(2.5, theme::accent()), egui::StrokeKind::Middle);
                        } else if response.hovered() {
                            ui.painter().rect_stroke(image.expand(4.0), 6, Stroke::new(2.0, theme::accent_line()), egui::StrokeKind::Middle);
                        }
                        ui.painter().rect_filled(image, 2.0, Color32::WHITE);
                        if let Some(raster) = tab.rasters.get_mut(&key).or_else(|| tab.stale.get_mut(&page)) { raster.used = tick; ui.painter().image(raster.texture.id(), image, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE); }
                        if let Some(session) = &tab.session && session.editable {
                            for a in session.annotations.iter().filter(|a| a.page == page) {
                                editing::paint_annotation(&mut self.editor, ctx, ui.painter(), image, factor, a);
                            }
                        }
                        let number = pos2(rect.center().x, rect.bottom() - 11.0);
                        if current {
                            ui.painter().rect_filled(Rect::from_center_size(number, vec2(26. + 6. * (page + 1).to_string().len() as f32, 18.)), 9, theme::accent());
                        }
                        ui.painter().text(number, egui::Align2::CENTER_CENTER, (page + 1).to_string(), ui::strong(ui, 11.5), if current { theme::on_accent() } else { theme::muted() });
                        if response.on_hover_text(format!("Go to page {}", page + 1)).clicked() { tab.jump(page); }
                    }
                });
            }
        });
    }
    fn document(&mut self, ctx: &egui::Context) {
        let preview = self.live_signature_preview(ctx);
        let mut error_action = None;
        egui::CentralPanel::default()
            // Keep scrollbars and canvas hit testing clear of side-panel resize handles.
            // The handles extend 5 pt into the canvas; scroll contents also have a clip margin.
            .frame(egui::Frame::new().fill(theme::canvas()).inner_margin(egui::Margin::symmetric(8, 0)))
            .show(ctx, |ui| {
                let Some(tab) = self.tabs.get_mut(self.active) else {
                    ui.vertical_centered(|ui| {
                        ui.add_space((ui.available_height() * 0.2).max(30.0));
                        let (mark, _) = ui.allocate_exact_size(vec2(72., 72.), egui::Sense::hover());
                        ui::shadow(ui.painter(), mark, 18, 2);
                        ui::logo(ui.painter(), mark);
                        ui.add_space(18.0);
                        ui.label(RichText::new("Welcome to MyPDF").font(ui::bold(ui, 32.)).color(theme::ink()));
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new(
                                "Read, edit, annotate and sign. Your documents stay on your desktop.",
                            )
                            .size(16.0)
                            .color(theme::muted()),
                        );
                        ui.add_space(20.0);
                        if ui.add(ActionButton::new(Icon::Plus,"New Blank PDF")).clicked() { self.new_blank(ctx); }
                        if ui
                            .add(ActionButton::new(Icon::Open, "Open PDF…").primary())
                            .clicked()
                        {
                            self.open_dialog(ctx);
                        }
                        ui.add_space(20.0);
                        ui.label(
                            RichText::new("Drop files here  •  ⌘O on Mac  /  Ctrl+O on Windows")
                                .size(12.0)
                                .color(theme::muted()),
                        );
                    });
                    return;
                };
                if let Some(error) = &tab.error {
                    ui.centered_and_justified(|ui| {
                        ui.vertical_centered(|ui| {
                            ui.heading("Could not open this PDF");
                            ui.label(error);
                            ui.add_space(16.);
                            ui.horizontal(|ui| {
                                if ui.add(ActionButton::new(Icon::Open, "Open another PDF…").primary()).clicked() { error_action = Some("open"); }
                                if ui.add(ActionButton::new(Icon::Redo, "Retry")).clicked() { error_action = Some("retry"); }
                                if ui.add(ActionButton::new(Icon::Close, "Close")).clicked() { error_action = Some("close"); }
                            });
                            ui.label(
                                RichText::new(tab.path.display().to_string())
                                    .small()
                                    .color(theme::muted()),
                            );
                        });
                    });
                    return;
                }
                if tab.pages.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.spinner();
                        ui.label("Opening PDF…");
                    });
                    return;
                }
                // Reserve only the scrollbar's actual layout width. The old
                // canvas was wider than the viewport even in Fit Width.
                let available = ui.available_size() - vec2(ui.spacing().scroll.allocated_width(), 0.);
                let mut layout = reading::Layout::arrange(&tab.pages, tab.zoom, available, tab.page_layout, tab.page);
                let mut anchor_offset = None;
                let delta = ctx.input(|i| i.zoom_delta());
                if delta.is_finite() && (delta - 1.).abs() > 0.0001
                    && let Some((view, offset, _)) = tab.document_view
                    && let Some(old) = &tab.document_layout
                    && let Some(pointer) = ctx.pointer_hover_pos()
                    && view.contains(pointer) && ctx.layer_id_at(pointer) == Some(ui.layer_id())
                {
                    let pointer = pointer - view.min;
                    let next = (old.scale * delta).clamp(0.1, 6.);
                    tab.zoom = Zoom::Scale(next);
                    layout = reading::Layout::arrange(&tab.pages, tab.zoom, available, tab.page_layout, tab.page);
                    anchor_offset = Some(old.anchored_offset(&layout, offset, pointer));
                    tab.last_pinch=Some(std::time::Instant::now());
                    tab.jump = None;
                }
                // Resizing the window or toggling panels keeps your reading
                // position instead of jumping back to the top of the page.
                if anchor_offset.is_none() && tab.jump.is_none()
                    && let Some((view, offset, _)) = tab.document_view
                    && let Some(old) = &tab.document_layout
                    && old.rects.len() == layout.rects.len() && (*old != layout || tab.restore_scroll)
                {
                    let mut next = old.anchored_offset(&layout, offset, vec2(view.width() / 2., 0.));
                    next.x = (next.x + (view.width() - available.x) / 2.).max(0.);
                    anchor_offset = Some(next);
                }
                tab.restore_scroll = false;
                let max_width = layout.size.x;
                let mut scroll = egui::ScrollArea::both()
                    .scroll_source(egui::scroll_area::ScrollSource {
                        drag: self.editor.mode == editing::Mode::Read,
                        ..Default::default()
                    })
                    .id_salt((tab.id, "document"))
                    .auto_shrink([false, false]);
                if let Some(offset) = anchor_offset {
                    scroll = scroll.scroll_offset(offset);
                }
                if let Some((page, y)) = tab.jump.take() {
                    scroll = scroll.vertical_scroll_offset(
                        (layout.rects[page].top() + y.unwrap_or(0.0) * layout.scale - reading::GAP).max(0.0),
                    );
                    if matches!(tab.zoom, Zoom::Width | Zoom::Page) {
                        // Wider later pages may need horizontal scrolling,
                        // but the reference page opens centered and fills
                        // the viewport, rather than being pushed to its side.
                        scroll = scroll.horizontal_scroll_offset(((max_width - available.x) / 2.).max(0.));
                    }
                }
                let mut visible_page = tab.page;
                let mut follow = None;
                tab.navigation(ctx);
                let output = scroll.show_viewport(ui, |ui, viewport| {
                    let (canvas, canvas_response) =
                        ui.allocate_exact_size(layout.size, egui::Sense::click());
                    if canvas_response.clicked()
                        && matches!(self.editor.mode, editing::Mode::Read | editing::Mode::Draw | editing::Mode::Annotate)
                        && let Some(pointer) = canvas_response.interact_pointer_pos()
                        && !layout.rects.iter().any(|rect| rect.translate(canvas.min.to_vec2()).contains(pointer))
                    {
                        tab.selected_annotation = None;
                        tab.group.clear();
                        self.editor.draft = None;
                    }
                    // In Edit, a click on the gray surround ends the open text editor too.
                    if canvas_response.clicked() && self.editor.mode == editing::Mode::Edit {
                        self.content.click_outside(tab.id);
                    }
                    let mut most_visible = 0.;
                    for (page, rect) in layout.rects.iter().enumerate().filter(|(_, r)| r.is_positive()) {
                        let overlap = (rect.bottom().min(viewport.bottom()) - rect.top().max(viewport.top())).max(0.);
                        if overlap > most_visible {
                            most_visible = overlap;
                            visible_page = page;
                        }
                    }
                    if tab.page_layout==reading::PageLayout::Facing && layout.rects.get(tab.page).is_some_and(|r|r.is_positive() && r.bottom()>viewport.top() && r.top()<viewport.bottom()) { visible_page=tab.page; }
                    for (page, rect) in layout.rects.iter().enumerate().filter(|(_, r)| r.is_positive()) {
                        let size = tab.pages[page];
                        let scale = layout.scale;
                        if rect.bottom() < viewport.min.y - 300.0 || rect.top() > viewport.max.y + 300.0 {
                            continue;
                        }
                        let page_rect = rect.translate(canvas.min.to_vec2());
                        ui::shadow(ui.painter(), page_rect, 1, 2);
                        ui.painter().rect_filled(page_rect, 0.0, Color32::WHITE);
                        let key =
                            viewer::raster_key(page, size, scale * ctx.pixels_per_point(), false);
                        let fallback = tab
                            .rasters
                            .keys()
                            .filter(|k| k.page == page && !k.thumbnail)
                            .min_by_key(|k| k.scale.abs_diff(key.scale))
                            .copied();
                        let settling=tab.last_pinch.is_some_and(|t|t.elapsed()<std::time::Duration::from_millis(120));
                        if !settling || (fallback.is_none() && !tab.pending.iter().any(|k|k.page==page && !k.thumbnail)) { tab.request(key); }
                        if settling { ctx.request_repaint_after(std::time::Duration::from_millis(120)); }
                        if let Some(raster) = tab.rasters.get_mut(&key) {
                            raster.used = self.tick;
                            ui.painter().image(
                                raster.texture.id(),
                                page_rect,
                                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        } else if let Some(fallback) = fallback {
                            if let Some(raster) = tab.rasters.get_mut(&fallback) {
                                raster.used = self.tick;
                                ui.painter().image(
                                    raster.texture.id(),
                                    page_rect,
                                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                    Color32::WHITE,
                                );
                            }
                        } else if let Some(raster) = tab.stale.get(&page) {
                            // The page before the edit, until the edited page is drawn.
                            ui.painter().image(
                                raster.texture.id(),
                                page_rect,
                                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        } else {
                            ui.painter().text(
                                page_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                tab.render_errors
                                    .get(&page)
                                    .map(String::as_str)
                                    .unwrap_or("Rendering page…"),
                                FontId::proportional(14.0),
                                theme::muted(),
                            );
                        }
                        if self.search && !tab.query.is_empty() {
                            for (index, hit) in
                                tab.hits.iter().enumerate().filter(|(_, h)| h.page == page)
                            {
                                for r in &hit.rects {
                                    let highlight = Rect::from_min_max(
                                        page_rect.min + vec2(r[0], r[1]) * scale,
                                        page_rect.min + vec2(r[2], r[3]) * scale,
                                    )
                                    .expand(1.5)
                                    .intersect(page_rect);
                                    ui.painter().rect_filled(
                                        highlight,
                                        1.0,
                                        if index == tab.hit {
                                            Color32::from_rgba_unmultiplied(244, 151, 29, 115)
                                        } else {
                                            Color32::from_rgba_unmultiplied(248, 213, 70, 85)
                                        },
                                    );
                                }
                            }
                        }
                        if self.workbench.paste.is_none() && self.workbench.crop.is_none() && !self.organizer.busy && !self.redaction.active {
                            content::smart_select(&mut self.content,&mut self.editor,tab,ui,page,page_rect,scale,&mut self.inspector);
                        }
                        let placing = workbench::crop_overlay(&mut self.workbench, &mut self.editor, tab, ui, page, page_rect, scale) || workbench::paste_overlay(&mut self.workbench, &mut self.editor, tab, ui, page, page_rect, scale, &mut self.status);
                        if !placing {
                        editing::page_interaction(
                            &mut self.editor,
                            tab,
                            ui,
                            page,
                            page_rect,
                            scale,
                            &mut self.status,
                        );
                        }
                        self.content.hide_guides = self.editor.hide_guides;
                        self.content.paused = self.productivity.owns_input() || placing || self.redaction.active && self.editor.mode == editing::Mode::Edit;
                        if self.editor.mode == editing::Mode::Forms {
                            let editable = !self.content.busy
                                && !self.organizer.busy
                                && tab.session.as_ref().is_some_and(|s| s.editable);
                            form_tools::overlay(&mut self.forms, self.content.inspection.as_ref(), tab, editable, ui, page, page_rect, scale);
                        } else {
                            content::overlay(&mut self.content,self.editor.mode,tab,ui,page,page_rect,scale);
                        }
                        if !placing && !self.content.paused && !self.organizer.busy {
                            content::page_menu(&mut self.content, &mut self.editor, tab, ui, page, page_rect, scale);
                        }
                        if self.redaction.active && self.editor.mode == editing::Mode::Edit && !placing {
                            redaction::overlay(&mut self.redaction, tab, ui, page, page_rect, scale);
                        }
                        if matches!(self.editor.mode, editing::Mode::Read | editing::Mode::Sign)
                            || tab.signed_original.is_none()
                        {
                            signatures::page_overlay(tab, ui, page, page_rect, scale);
                        }
                        if let Some((target, target_page, [l, t, r, b], texture)) = &preview
                            && *target == tab.id && *target_page == page {
                            let rect = Rect::from_min_max(
                                page_rect.min + vec2(*l, *t) * scale,
                                page_rect.min + vec2(*r, *b) * scale);
                            ui.painter().with_clip_rect(ui.clip_rect().intersect(page_rect)).image(
                                texture.id(), rect, Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)), Color32::WHITE);
                            ui.painter().rect_stroke(rect, 0., Stroke::new(1., theme::accent()), egui::StrokeKind::Outside);
                        }
                        workbench::reveal_overlay(&self.workbench,tab,ui,page,page_rect,scale);
                        if let Some(target) = navigate::link_overlay(tab, self.editor.mode, ui, page, page_rect, scale) {
                            follow = Some(target);
                        }
                        ui.painter().rect_stroke(
                            page_rect,
                            0.0,
                            Stroke::new(1.0, Color32::from_black_alpha(12)),
                            egui::StrokeKind::Outside,
                        );
                    }
                });
                tab.document_view = Some((output.inner_rect, output.state.offset, max_width));
                tab.scale = layout.scale;
                tab.document_layout = Some(layout);
                match follow {
                    Some(navigation::Target::Page(page)) => tab.jump(page),
                    Some(navigation::Target::Uri(uri)) => self.link_confirm = Some(uri),
                    None => {}
                }
                if visible_page != tab.page {
                    tab.page = visible_page;
                    tab.page_input = (visible_page + 1).to_string();
                }
            });
        match error_action {
            Some("retry") => self.retry_document(ctx),
            Some(action) => self.action(action, ctx),
            None => {}
        }
    }
    fn trim_cache(&mut self) {
        // Derived pages (including transitional rasters) and image textures have
        // separate caps; together with history the managed estimate is 704 MiB
        // (not a process-footprint cap: decoder/driver allocations are separate).
        // Current documents are never discarded, even when they exceed it.
        self.editor.trim_image_cache_to(if self.tabs.is_empty() {
            0
        } else {
            64 * 1024 * 1024
        });
        let mut total: usize = self
            .tabs
            .iter()
            .flat_map(|t| t.rasters.values().chain(t.stale.values()))
            .map(|r| r.bytes)
            .sum();
        let mut candidates: Vec<_> = self
            .tabs
            .iter()
            .enumerate()
            .flat_map(|(i, t)| {
                t.rasters
                    .iter()
                    .map(move |(k, r)| (r.used, i, Some(*k), k.page, r.bytes))
                    .chain(
                        t.stale
                            .iter()
                            .map(move |(page, r)| (r.used, i, None, *page, r.bytes)),
                    )
            })
            .collect();
        candidates.sort_by_key(|r| r.0);
        for (_, tab, key, page, bytes) in candidates {
            if total <= CACHE_BUDGET {
                break;
            }
            if let Some(key) = key {
                self.tabs[tab].rasters.remove(&key);
            } else {
                self.tabs[tab].stale.remove(&page);
            }
            total -= bytes;
        }
        let mut retained: usize = self
            .tabs
            .iter()
            .filter_map(|t| t.session.as_ref())
            .map(|s| s.retained_bytes())
            .sum();
        // Retain the active document's most recent undo steps for as long as possible.
        let order: Vec<_> = (0..self.tabs.len())
            .filter(|i| *i != self.active)
            .chain((self.active < self.tabs.len()).then_some(self.active))
            .collect();
        for i in order {
            if retained <= HISTORY_BUDGET {
                break;
            }
            if let Some(session) = &mut self.tabs[i].session {
                let before = session.retained_bytes();
                if before <= session.current_retained_bytes() {
                    continue;
                }
                session.trim_history_to_budget(before.saturating_sub(retained - HISTORY_BUDGET));
                retained = retained.saturating_sub(before - session.retained_bytes());
            }
        }
    }
}

impl eframe::App for Viewer {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        #[cfg(target_os = "macos")]
        window_drag::configure(frame);
        theme::sync(ctx.theme());
        self.tick += 1;
        if let Some(menu) = &self._native_menu {
            let tab = self.tabs.get(self.active);
            let session = tab.and_then(|t| t.session.as_ref());
            let pending_text = tab.is_some_and(|tab| self.content.has_pending_text_change(tab.id));
            let placement =
                tab.is_some_and(|tab| self.content.placement_undo_owns_input(ctx, tab.id));
            menu.history_labels(
                if pending_text {
                    Some("text edit".into())
                } else {
                    session.and_then(|s| s.undo_label())
                },
                if pending_text {
                    None
                } else {
                    session.and_then(|s| s.redo_label())
                },
                ctx.wants_keyboard_input() && !placement,
            );
        }
        let arrivals = open_with::take(ctx);
        arrivals.activate(ctx);
        for path in arrivals.paths {
            self.open(path, ctx);
        }
        self.content_events(ctx);
        self.page_events(ctx);
        self.events(ctx);
        self.translation_tick(ctx);
        self.editor_events();
        self.digital_events(ctx);
        self.signature_events(ctx);
        self.shortcuts(ctx);
        self.zen_keys(ctx);
        self.compare_tick(ctx);
        if !self.home.visible
            && matches!(
                self.editor.mode,
                editing::Mode::Edit | editing::Mode::Draw | editing::Mode::Annotate
            )
        {
            self.inspect_content(ctx);
        }
        self.editor.text_editing = false;
        self.sync_status_context();
        self.toolbar(ctx);
        if self.tab_controls.take_window_drag() {
            window_drag::begin(frame, ctx);
        }
        self.restore_reading_panels();
        if !self.home.visible
            && self.comparison.is_none()
            && self.zen.is_none()
            && self.active_document_ready()
        {
            self.editor_tools(ctx);
            self.page_tools(ctx);
            self.content_tools(ctx);
            self.form_ribbon(ctx);
            self.signature_bar(ctx);
            self.workbench_bar(ctx);
        }
        self.sync_signature_panel();
        self.sync_status_context();
        if let Some((status, true)) = &self.status {
            let status = status.clone();
            let mut dismiss = false;
            egui::TopBottomPanel::bottom("status")
                .frame(
                    egui::Frame::new()
                        .fill(theme::danger_soft())
                        .inner_margin(egui::Margin::symmetric(14, 3)),
                )
                .show(ctx, |ui| {
                    dismiss = ui::notice(ui, &status);
                });
            if dismiss {
                self.status = None;
            }
        }
        self.note_reader_requests(ctx);
        let digital_docked = self.digital_docked();
        if digital_docked {
            self.digital_window(ctx);
        }
        let translation_visible = !digital_docked
            && self.translation.open
            && self.inspector
            && self.zen.is_none()
            && self.comparison.is_none();
        if translation_visible {
            self.translation_panel(ctx);
        }
        if self.home.visible {
            self.home_screen(ctx);
        } else if self.comparison.is_some() {
            self.compare_screen(ctx);
        } else if self.zen.is_some() {
            self.document(ctx);
            self.zen_overlay(ctx);
        } else if !self.active_document_ready() {
            self.document(ctx);
        } else if self.editor.mode == editing::Mode::Pages {
            if self.inspector && !translation_visible {
                self.organizer_panel(ctx);
            }
            self.page_overview(ctx);
        } else {
            if !digital_docked || ctx.available_rect().width() > 650. {
                self.sidebar(ctx);
            }
            if self.signature_panel_open() && !translation_visible && !digital_docked {
                self.signature_panel(ctx);
            } else if self.inspector
                && self.workbench.crop.is_none()
                && !translation_visible
                && !digital_docked
            {
                if self.editor.mode == editing::Mode::Edit && self.redaction.active {
                    self.redaction_panel(ctx);
                } else if self.editor.mode == editing::Mode::Forms {
                    self.form_panel(ctx);
                } else if matches!(
                    self.editor.mode,
                    editing::Mode::Edit | editing::Mode::Forms | editing::Mode::Convert
                ) {
                    self.content_panel(ctx);
                } else {
                    self.asset_panel(ctx);
                    if !self.editor.library_open {
                        self.annotation_inspector(ctx);
                    }
                }
            }
            self.document(ctx);
        }
        self.finish_new_text(ctx);
        if let Some(text) = content::context::take_translation_text(ctx) {
            self.translate_text(ctx, text, "Selected text".into());
        }
        self.form_events(ctx);
        self.form_dialogs(ctx);
        self.password_prompt(ctx);
        self.protect_dialogs(ctx);
        self.shrink_dialog(ctx);
        self.link_dialog(ctx);
        self.watermark_dialog(ctx);
        self.nup_dialog(ctx);
        self.ocr_dialogs(ctx);
        self.compare_dialog(ctx);
        self.details_dialogs(ctx);
        self.page_dialogs(ctx);
        self.digital_events(ctx);
        self.editor_windows(ctx);
        if !digital_docked {
            self.digital_window(ctx);
        }
        self.background_window(ctx);
        self.image_region_window(ctx);
        self.workbench_windows(ctx);
        self.productivity_windows(ctx);
        self.header_footer_window(ctx);
        self.redaction_dialogs(ctx);
        self.signature_dialogs(ctx);
        self.close_dialog(ctx);
        if self.exit_approved {
            for tab in &self.tabs {
                self.recovery.clear(tab.id);
            }
        } else {
            self.recovery_tick(ctx);
        }
        self.recovery_window(ctx);
        if self.accessible_text {
            egui::Window::new("Accessible page text")
                .open(&mut self.accessible_text)
                .collapsible(false)
                .default_size(vec2(520., 420.))
                .show(ctx, |ui| {
                    if let Some(tab) = self.tabs.get(self.active) {
                        ui.label(format!("{} · Page {}", tab.name(), tab.page + 1));
                        if let Some(text) = tab.text.get(tab.page).and_then(Option::as_ref) {
                            if text.text.trim().is_empty() {
                                ui.label("This page contains no extractable text.");
                            }
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                let mut value = text.text.as_str();
                                ui.add(
                                    egui::TextEdit::multiline(&mut value)
                                        .margin(ui::FIELD_MARGIN)
                                        .desired_width(f32::INFINITY)
                                        .desired_rows(18),
                                )
                                .widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::WidgetType::TextEdit,
                                        true,
                                        "Page text",
                                    )
                                });
                            });
                        } else {
                            ui.label("Page text is being indexed…");
                        }
                    }
                });
        }
        self.remember_reading(ctx, self.exit_approved);
        self.remember_workspace(ctx);
        self.workspace_window(ctx);
        self.review_window(ctx);
        self.navigation_editor_window(ctx);
        self.trim_cache();
        if !ctx.input(|i| i.raw.hovered_files.is_empty()) {
            let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop"));
            let painter = ctx.layer_painter(layer);
            let rect = ctx.content_rect();
            painter.rect_filled(rect, 0.0, theme::paper().gamma_multiply(0.9));
            let zone = rect.shrink(24.);
            painter.rect_filled(zone, 22, theme::accent().gamma_multiply(0.06));
            home::dashed_outline(
                &painter,
                zone,
                22.,
                Stroke::new(2., theme::accent().gamma_multiply(0.7)),
            );
            let tile = Rect::from_center_size(zone.center() - vec2(0., 36.), vec2(72., 72.));
            ui::badge(
                &painter,
                tile,
                Icon::Import,
                ui::Fill::Brand,
                Color32::WHITE,
            );
            painter.text(
                zone.center() + vec2(0., 34.),
                egui::Align2::CENTER_CENTER,
                "Drop PDFs to open",
                ui::font(ctx, ui::STRONG, 22.),
                theme::ink(),
            );
            painter.text(
                zone.center() + vec2(0., 62.),
                egui::Align2::CENTER_CENTER,
                "Each file opens in its own tab",
                FontId::proportional(13.5),
                theme::muted(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;
    use std::time::{Duration, Instant};

    fn frame(app: &mut Viewer, ctx: &egui::Context, events: Vec<egui::Event>) {
        let modifiers = events
            .iter()
            .find_map(|event| {
                if let egui::Event::Key { modifiers, .. } = event {
                    Some(*modifiers)
                } else {
                    None
                }
            })
            .unwrap_or_default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1100.0, 760.0))),
            events,
            modifiers,
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            app.update(ctx, &mut eframe::Frame::_new_kittest())
        });
    }
    fn key(key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
        vec![
            egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            },
            egui::Event::Key {
                key,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers,
            },
        ]
    }
    fn until(app: &mut Viewer, ctx: &egui::Context, ready: impl Fn(&Viewer) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !ready(app) {
            assert!(
                Instant::now() < deadline,
                "Desktop event loop did not reach the expected state"
            );
            frame(app, ctx, vec![]);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn clicks_select_drawings_and_clear_on_empty_page_or_surrounding_canvas() {
        use mypdf_milestone0::annotations::{Annotation, Kind, Style};
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        until(&mut app, &ctx, |a| a.tabs[0].session.is_some());
        app.editor.mode = editing::Mode::Draw;
        app.inspector = true;
        let line = Annotation::new(
            0,
            [100., 100., 280., 260.],
            Kind::Line {
                start: [0., 1.],
                end: [1., 0.],
                arrow: false,
            },
            Style::default(),
        );
        let ellipse = Annotation::new(0, [310., 100., 430., 220.], Kind::Ellipse, Style::default());
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![line.clone(), ellipse.clone()])
            .unwrap();
        let revision = app.tabs[0].session.as_ref().unwrap().revision();
        app.tabs[0].selected_annotation = Some(line.id.clone());
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        let page = |app: &Viewer| {
            ctx.read_response(egui::Id::new((app.tabs[0].id, 0_usize, "page_actions")))
                .unwrap()
                .rect
        };
        let click = |app: &mut Viewer, p: egui::Pos2| {
            for pressed in [true, false] {
                frame(
                    app,
                    &ctx,
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
        for tool in [
            editing::Tool::Line,
            editing::Tool::Arrow,
            editing::Tool::Rectangle,
            editing::Tool::Pen,
            editing::Tool::Select,
        ] {
            app.editor.select_tool(tool);
            let rect = page(&app);
            let scale = app.tabs[0].scale;
            click(&mut app, rect.min + vec2(110., 110.) * scale);
            assert!(
                app.tabs[0].selected_annotation.is_none(),
                "empty space inside a diagonal line's bounding box must deselect"
            );
            assert!(app.editor.draft.is_none());
            frame(&mut app, &ctx, vec![]);
            assert!(
                (page(&app).width() - rect.width()).abs() < 1.,
                "deselection must not resize the page"
            );
            click(&mut app, rect.min + vec2(190., 180.) * scale);
            assert_eq!(
                app.tabs[0].selected_annotation.as_deref(),
                Some(line.id.as_str())
            );
            let gray = rect.min + vec2(-reading::GUTTER / 2., 70.);
            click(&mut app, gray);
            assert!(
                app.tabs[0].selected_annotation.is_none(),
                "gray page surround clears selection too"
            );
            let rect = page(&app);
            click(&mut app, rect.min + vec2(370., 160.) * scale);
            assert_eq!(
                app.tabs[0].selected_annotation.as_deref(),
                Some(ellipse.id.as_str())
            );
            click(&mut app, rect.min + vec2(312., 102.) * scale);
            assert!(
                app.tabs[0].selected_annotation.is_none(),
                "ellipse bounding-box corners are outside the shape"
            );
        }
        assert_eq!(
            app.tabs[0].session.as_ref().unwrap().revision(),
            revision,
            "selection never edits document contents"
        );
    }

    #[test]
    fn drawing_inspector_updates_on_each_frame_and_groups_slider_undo() {
        use mypdf_milestone0::annotations::{Annotation, Kind, StrokeStyle, Style};
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        until(&mut app, &ctx, |a| a.tabs[0].session.is_some());
        app.editor.mode = editing::Mode::Draw;
        app.inspector = true;
        let kinds = [
            Kind::Rectangle,
            Kind::Ellipse,
            Kind::Line {
                start: [0., 0.],
                end: [1., 1.],
                arrow: true,
            },
            Kind::DoubleArrow {
                start: [0., 0.],
                end: [1., 1.],
            },
            Kind::Polygon {
                points: vec![[0., 0.], [1., 0.], [0.5, 1.]],
                closed: true,
            },
            Kind::Ink {
                strokes: vec![vec![[0., 0.], [1., 1.]]],
            },
            Kind::Svg {
                source: mypdf_milestone0::svg::DEFAULT.into(),
                tint: false,
            },
        ];
        let bounds = |app: &mut Viewer, label: &str, last: bool| {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1100., 760.))),
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
            let nodes = output.platform_output.accesskit_update.unwrap().nodes;
            assert!(
                !nodes
                    .iter()
                    .any(|(_, n)| matches!(n.label(), Some("Apply" | "Reset")))
            );
            let matches: Vec<_> = nodes
                .iter()
                .filter(|(_, n)| n.label() == Some(label))
                .collect();
            let node = if last {
                matches.last()
            } else {
                matches.first()
            }
            .unwrap_or_else(|| panic!("missing {label}"));
            let r = node.1.bounds().unwrap();
            Rect::from_min_max(
                pos2(r.x0 as f32, r.y0 as f32),
                pos2(r.x1 as f32, r.y1 as f32),
            )
        };
        let pointer = |p, pressed| egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let click = |app: &mut Viewer, p| {
            for pressed in [true, false] {
                frame(
                    app,
                    &ctx,
                    vec![egui::Event::PointerMoved(p), pointer(p, pressed)],
                );
            }
        };
        for kind in kinds {
            let mark = Annotation::new(0, [100., 100., 250., 200.], kind, Style::default());
            app.tabs[0]
                .session
                .as_mut()
                .unwrap()
                .commit(vec![mark.clone()])
                .unwrap();
            app.tabs[0].selected_annotation = Some(mark.id.clone());
            app.editor.draft = None;
            for _ in 0..3 {
                frame(&mut app, &ctx, vec![]);
            }
            let blue = bounds(&mut app, "Blue color", false).center();
            click(&mut app, blue);
            assert_eq!(
                app.tabs[0].session.as_ref().unwrap().annotations[0]
                    .style
                    .color,
                [0.15, 0.49, 0.85]
            );
            if !matches!(mark.kind, Kind::Svg { .. }) {
                for style in StrokeStyle::ALL {
                    let button =
                        bounds(&mut app, &format!("{} stroke style", style.label()), false)
                            .center();
                    click(&mut app, button);
                    assert_eq!(
                        app.tabs[0].session.as_ref().unwrap().annotations[0]
                            .style
                            .stroke,
                        style
                    );
                }
            }
            let before = app.tabs[0].session.as_ref().unwrap().annotations[0].clone();
            let slider = bounds(&mut app, "Opacity", true);
            let at =
                |fraction: f32| pos2(slider.left() + slider.width() * fraction, slider.center().y);
            frame(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(at(0.85)), pointer(at(0.85), true)],
            );
            let mut last_opacity = 1.;
            for fraction in [0.7, 0.5, 0.3] {
                frame(
                    &mut app,
                    &ctx,
                    vec![egui::Event::PointerMoved(at(fraction))],
                );
                let s = app.tabs[0].session.as_ref().unwrap();
                assert!(
                    s.annotations[0].style.opacity < last_opacity,
                    "opacity must change before mouse release"
                );
                last_opacity = s.annotations[0].style.opacity;
                assert!(s.dirty());
            }
            frame(&mut app, &ctx, vec![pointer(at(0.3), false)]);
            let after = app.tabs[0].session.as_ref().unwrap().annotations[0].clone();
            app.editor_undo(false);
            assert_eq!(
                app.tabs[0].session.as_ref().unwrap().annotations[0],
                before,
                "one Undo reverses the entire slider gesture"
            );
            app.editor_undo(true);
            assert_eq!(app.tabs[0].session.as_ref().unwrap().annotations[0], after);
            let saved = app.tabs[0]
                .session
                .as_ref()
                .unwrap()
                .snapshot_bytes()
                .unwrap();
            assert_eq!(
                mypdf_milestone0::annotations::load(&lopdf::Document::load_mem(&saved).unwrap()),
                vec![after]
            );
        }
    }

    #[test]
    fn main_ribbon_offers_signing_tab() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        until(&mut app, &ctx, |a| a.tabs[0].session.is_some());
        for mode in [
            editing::Mode::Read,
            editing::Mode::Annotate,
            editing::Mode::Edit,
            editing::Mode::Draw,
        ] {
            app.editor.mode = mode;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1100., 760.))),
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
            assert!(
                output
                    .platform_output
                    .accesskit_update
                    .unwrap()
                    .nodes
                    .iter()
                    .any(|(_, n)| n.label() == Some("Sign")),
                "{mode:?}"
            );
        }
    }
    #[test]
    fn home_to_add_text_types_in_place_and_reopens_saved_pdf() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        until(&mut app, &ctx, |a| a.tabs[0].session.is_some());
        app.editor.mode = editing::Mode::Edit;
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1100., 760.))),
                ..Default::default()
            },
            |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
        );
        let nodes = output.platform_output.accesskit_update.unwrap().nodes;
        let r = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Add text"))
            .unwrap()
            .1
            .bounds()
            .unwrap();
        let button = pos2(((r.x0 + r.x1) / 2.) as f32, ((r.y0 + r.y1) / 2.) as f32);
        let click = |app: &mut Viewer, p| {
            for pressed in [true, false] {
                frame(
                    app,
                    &ctx,
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
        click(&mut app, button);
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        let page = ctx
            .read_response(egui::Id::new((
                app.tabs[0].id,
                0_usize,
                "add_text_position",
            )))
            .unwrap()
            .rect;
        let point = page.min + vec2(80., 100.);
        click(&mut app, point);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Text("Typed in place on this page".into())],
        );
        let text_id = egui::Id::new((app.tabs[0].id, 0_usize, "inline_new_text"));
        assert!(
            ctx.memory(|m| m.has_focus(text_id)),
            "Typing must focus the on-page editor"
        );
        let text_box = ctx.read_response(text_id).unwrap().rect;
        assert!(
            (text_box.left() - point.x).abs() < 1.
                && (ctx.data(|data| data
                    .get_temp::<f32>(egui::Id::new("new_text_first_line_center"))
                    .unwrap())
                    - point.y)
                    .abs()
                    < 2.,
            "The first text line must be centered at the clicked point"
        );
        app.show_home();
        assert!(app.home.visible && app.tabs.len() == 1);
        frame(&mut app, &ctx, vec![]);
        app.home.visible = false;
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        frame(
            &mut app,
            &ctx,
            key(egui::Key::Enter, egui::Modifiers::COMMAND),
        );
        until(&mut app, &ctx, |a| {
            !a.content.busy && a.tabs[0].session.as_ref().unwrap().revision() > 0
        });
        let saved = app.tabs[0]
            .session
            .as_ref()
            .unwrap()
            .snapshot_bytes()
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("In-place text.pdf");
        viewer::save_copy(&path, &saved).unwrap();
        app.open(path, &ctx);
        until(&mut app, &ctx, |a| a.tabs[1].index_done);
        assert!(
            app.tabs[1].text[0]
                .as_ref()
                .unwrap()
                .text
                .contains("Typed in place on this page")
        );
        assert!(!app.tabs[1].session.as_ref().unwrap().dirty());
    }

    #[test]
    fn pinch_zoom_is_anchored_and_only_changes_the_document() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        until(&mut app, &ctx, |a| a.tabs[0].session.is_some());
        app.action("actual", &ctx);
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        let (view, offset, width) = app.tabs[0].document_view.unwrap();
        let pointer = view.center();
        let page = app.tabs[0].pages[0];
        let before = vec2(
            offset.x + pointer.x - view.min.x - (width - page.width) / 2.,
            offset.y + pointer.y - view.min.y - 24.,
        );
        let ui_zoom = ctx.zoom_factor();
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(pointer), egui::Event::Zoom(1.5)],
        );
        assert_eq!(app.tabs[0].scale, 1.5);
        assert_eq!(ctx.zoom_factor(), ui_zoom);
        let (after_view, after_offset, after_width) = app.tabs[0].document_view.unwrap();
        let after = vec2(
            (after_offset.x + pointer.x - after_view.min.x - (after_width - page.width * 1.5) / 2.)
                / 1.5,
            (after_offset.y + pointer.y - after_view.min.y - 24.) / 1.5,
        );
        assert!(
            (before - after).length() < 1.,
            "anchor moved: {before:?} -> {after:?}"
        );
        frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(pos2(10., 10.)),
                egui::Event::Zoom(2.),
            ],
        );
        assert_eq!(app.tabs[0].scale, 1.5);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(pointer), egui::Event::Zoom(0.5)],
        );
        assert_eq!(app.tabs[0].scale, 0.75);
    }

    #[test]
    fn desktop_event_loop_search_navigation_tabs_and_error_recovery() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("original.pdf");
        let second = temp.path().join("edited.pdf");
        let bytes = mypdf_milestone0::document::fixture(
            &mypdf_milestone0::document::default_font().unwrap(),
        )
        .unwrap();
        std::fs::write(&first, &bytes).unwrap();
        std::fs::write(&second, &bytes).unwrap();
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Viewer::new(&cc, vec![first.clone(), second]);
        until(&mut app, &ctx, |a| {
            a.tabs.iter().all(|t| t.index_done) && !a.tabs[1].rasters.is_empty()
        });
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.active, 1);
        app.action("actual", &ctx);
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.tabs[1].scale, 1.0);
        app.action("find", &ctx);
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![egui::Event::Text("sözleşme".into())]);
        until(&mut app, &ctx, |a| a.tabs[1].hits.len() == 2);
        let (menu_tx, menu_rx) = mpsc::channel();
        app.menu_events = menu_rx;
        menu_tx.send("select_all".into()).unwrap();
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![egui::Event::Text("sözleşme".into())]);
        assert_eq!(
            app.tabs[1].query, "sözleşme",
            "Native Select All must reach egui text fields"
        );
        assert_eq!(app.tabs[1].query, "sözleşme");
        until(&mut app, &ctx, |a| {
            !a.tabs[1].search_dirty && !a.tabs[1].searching && a.tabs[1].hits.len() == 2
        });
        assert_eq!(app.tabs[1].hit, 0);
        frame(&mut app, &ctx, key(egui::Key::Enter, egui::Modifiers::NONE));
        assert_eq!(
            app.tabs[1].hit, 1,
            "Enter must navigate even when TextEdit surrenders focus"
        );
        assert_eq!(app.tabs[1].page, 1);
        frame(
            &mut app,
            &ctx,
            key(egui::Key::Enter, egui::Modifiers::SHIFT),
        );
        assert_eq!(app.tabs[1].hit, 0);
        frame(&mut app, &ctx, key(egui::Key::Tab, egui::Modifiers::CTRL));
        assert_eq!(app.active, 0);
        assert_eq!(app.tabs[0].query, "");
        assert!(app.tabs[0].zoom == Zoom::Width);
        frame(&mut app, &ctx, key(egui::Key::Tab, egui::Modifiers::CTRL));
        assert_eq!(app.active, 1);
        assert_eq!(app.tabs[1].query, "sözleşme");
        assert_eq!(app.tabs[1].scale, 1.0);
        app.open(first, &ctx);
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.active, 0);
        let invalid = temp.path().join("invalid.pdf");
        std::fs::write(&invalid, b"invalid PDF").unwrap();
        app.open(invalid, &ctx);
        until(&mut app, &ctx, |a| a.tabs[2].error.is_some());
        assert!(app.tabs[0].bytes.is_some() && app.tabs[1].bytes.is_some());
        app.close(2);
        assert_eq!(app.tabs.len(), 2);
    }
    #[test]
    fn editing_canvas_keeps_drag_gestures_when_the_page_overflows_the_scroll_viewport() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("scroll-canvas.pdf");
        std::fs::write(
            &path,
            mypdf_milestone0::document::fixture(
                &mypdf_milestone0::document::default_font().unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Viewer::new(&cc, vec![path]);
        until(&mut app, &ctx, |a| a.tabs[0].index_done);
        app.action("actual", &ctx);
        app.editor.mode = editing::Mode::Annotate;
        app.editor.select_tool(editing::Tool::Rectangle);
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        let rect = ctx
            .read_response(egui::Id::new((app.tabs[0].id, 0_usize, "page_actions")))
            .unwrap()
            .rect;
        let start = rect.min + vec2(150.0, 250.0);
        let end = rect.min + vec2(300.0, 320.0);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(start), button(start, true)],
        );
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(start + vec2(20.0, 20.0))],
        );
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(end)]);
        frame(&mut app, &ctx, vec![button(end, false)]);
        let session = app.tabs[0].session.as_ref().unwrap();
        assert_eq!(
            session.annotations.len(),
            1,
            "The viewer scroll area must not capture annotation drags"
        );
        assert_eq!(session.revision(), 1, "A gesture must commit once");
        assert_eq!(session.annotations[0].rect, [150.0, 250.0, 300.0, 320.0]);
    }
    #[test]
    fn organizer_drag_transactions_reload_search_and_share_annotation_undo() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("pages.pdf");
        let original = mypdf_milestone0::document::fixture(
            &mypdf_milestone0::document::default_font().unwrap(),
        )
        .unwrap();
        std::fs::write(&path, &original).unwrap();
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Viewer::new(&cc, vec![path.clone()]);
        until(&mut app, &ctx, |a| a.tabs[0].index_done);
        let annotation = mypdf_milestone0::annotations::Annotation::new(
            0,
            [60.0, 70.0, 180.0, 100.0],
            mypdf_milestone0::annotations::Kind::Rectangle,
            Default::default(),
        );
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![annotation])
            .unwrap();
        app.editor.mode = editing::Mode::Pages;
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        let first = ctx
            .read_response(egui::Id::new((app.tabs[0].id, 0_usize, "organizer_page")))
            .unwrap()
            .rect;
        let second = ctx
            .read_response(egui::Id::new((app.tabs[0].id, 1_usize, "organizer_page")))
            .unwrap()
            .rect;
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: egui::Modifiers::NONE,
        };
        let target = second.center() + vec2(20.0, 0.0);
        frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(first.center()),
                button(first.center(), true),
                egui::Event::PointerMoved(target),
                button(target, false),
            ],
        );
        until(&mut app, &ctx, |a| {
            !a.organizer.busy
                && a.tabs[0].index_done
                && a.tabs[0].session.as_ref().unwrap().revision() == 2
        });
        assert_eq!(app.tabs[0].session.as_ref().unwrap().annotations[0].page, 1);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        app.editor_undo(false);
        frame(&mut app, &ctx, vec![]);
        until(&mut app, &ctx, |a| a.tabs[0].index_done);
        assert_eq!(app.tabs[0].session.as_ref().unwrap().annotations[0].page, 0);
        app.editor_undo(true);
        frame(&mut app, &ctx, vec![]);
        until(&mut app, &ctx, |a| a.tabs[0].index_done);
        app.page_change(
            &ctx,
            mypdf_milestone0::pages::Operation::Crop {
                pages: vec![1],
                margins: [18.0; 4],
            },
        );
        until(&mut app, &ctx, |a| {
            !a.organizer.busy
                && a.tabs[0].index_done
                && a.tabs[0].session.as_ref().unwrap().revision() == 3
        });
        let result = app.tabs[0]
            .session
            .as_ref()
            .unwrap()
            .snapshot_bytes()
            .unwrap();
        let output = temp.path().join("organized.pdf");
        mypdf_milestone0::write_new(&output, &result).unwrap();
        app.open(output, &ctx);
        until(&mut app, &ctx, |a| a.tabs[1].index_done);
        assert_eq!(app.tabs[1].session.as_ref().unwrap().annotations[0].page, 1);
        assert!(!app.tabs[1].session.as_ref().unwrap().dirty());
        assert_eq!(app.tabs[0].pages[1].width, app.tabs[1].pages[1].width);
    }
    #[test]
    fn content_transactions_refresh_search_undo_and_safe_copy() {
        use mypdf_milestone0::{content as core, document};
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("source.pdf");
        let original = document::fixture(&document::default_font().unwrap()).unwrap();
        std::fs::write(&path, &original).unwrap();
        let ctx = egui::Context::default();
        let mut app = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            vec![path.clone()],
        );
        until(&mut app, &ctx, |a| a.tabs[0].index_done);
        app.editor.mode = editing::Mode::Edit;
        let (ordinal, run) = document::text_runs(&original, 1)
            .unwrap()
            .into_iter()
            .enumerate()
            .find(|(_, r)| r.text == document::ORIGINAL)
            .unwrap();
        app.content_change(
            &ctx,
            content::Edit::Text {
                page: 1,
                ordinal,
                original: run.text,
                replacement: document::REPLACEMENT.into(),
            },
        );
        until(&mut app, &ctx, |a| {
            !a.content.busy
                && a.tabs[0].index_done
                && a.tabs[0].session.as_ref().unwrap().revision() == 1
        });
        assert!(
            app.tabs[0].text[0]
                .as_ref()
                .unwrap()
                .text
                .contains(document::REPLACEMENT)
        );
        let source = app.tabs[0].session.as_ref().unwrap().source.clone();
        let image = core::images(&source, 1).unwrap()[0].clone();
        app.content_change(
            &ctx,
            content::Edit::Image {
                page: 1,
                index: image.index,
                edit: core::ImageEdit::Delete,
            },
        );
        until(&mut app, &ctx, |a| {
            !a.content.busy
                && a.tabs[0].index_done
                && a.tabs[0].session.as_ref().unwrap().revision() == 2
        });
        assert!(
            core::images(&app.tabs[0].session.as_ref().unwrap().source, 1)
                .unwrap()
                .is_empty()
        );
        app.editor_undo(false);
        frame(&mut app, &ctx, vec![]);
        until(&mut app, &ctx, |a| a.tabs[0].index_done);
        assert_eq!(
            core::images(&app.tabs[0].session.as_ref().unwrap().source, 1)
                .unwrap()
                .len(),
            1
        );
        let output = temp.path().join("edited.pdf");
        mypdf_milestone0::write_new(
            &output,
            &app.tabs[0]
                .session
                .as_ref()
                .unwrap()
                .snapshot_bytes()
                .unwrap(),
        )
        .unwrap();
        app.open(output, &ctx);
        until(&mut app, &ctx, |a| a.tabs[1].index_done);
        assert!(
            app.tabs[1].text[0]
                .as_ref()
                .unwrap()
                .text
                .contains(document::REPLACEMENT)
        );
        assert_eq!(std::fs::read(path).unwrap(), original);
    }
    #[test]
    fn blank_document_is_unsaved_and_blank_page_is_undoable() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        until(&mut app, &ctx, |a| a.tabs[0].index_done);
        assert!(app.tabs[0].session.as_ref().unwrap().dirty());
        app.page_change(
            &ctx,
            mypdf_milestone0::pages::Operation::InsertBlank {
                at: 1,
                width: 595.,
                height: 842.,
            },
        );
        until(&mut app, &ctx, |a| {
            !a.organizer.busy && a.tabs[0].pages.len() == 2 && a.tabs[0].index_done
        });
        app.editor_undo(false);
        frame(&mut app, &ctx, vec![]);
        until(&mut app, &ctx, |a| {
            a.tabs[0].pages.len() == 1 && a.tabs[0].index_done
        });
    }

    #[test]
    fn translation_toolbar_opens_visible_page_action_without_editing_document() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        until(&mut app, &ctx, |a| a.tabs[0].session.is_some());
        let mut button = None;
        for _ in 0..3 {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1100., 760.))),
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
            button = output
                .platform_output
                .accesskit_update
                .unwrap()
                .nodes
                .iter()
                .find(|(_, n)| n.label() == Some("Translate"))
                .and_then(|(_, n)| n.bounds());
        }
        let r = button.unwrap();
        let pos = pos2(((r.x0 + r.x1) / 2.) as f32, ((r.y0 + r.y1) / 2.) as f32);
        for pressed in [true, false] {
            frame(
                &mut app,
                &ctx,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(app.translation.open && app.inspector);
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1100., 760.))),
                ..Default::default()
            },
            |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
        );
        let nodes = output.platform_output.accesskit_update.unwrap().nodes;
        let action = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Translate page"))
            .unwrap();
        let r = action.1.bounds().unwrap();
        assert!(
            r.y0 >= 0. && r.y1 < 300. && r.x1 <= 1100.,
            "page action is visible at panel top: {r:?}"
        );
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), 0);
    }

    #[test]
    fn responsive_mode_controls_have_accessible_labels_and_stay_in_bounds() {
        for width in [720., 1200.] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
            app.home.visible = false;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(width, 820.))),
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
            let nodes = output.platform_output.accesskit_update.unwrap().nodes;
            let mut previous = 0.;
            for label in [
                "Read", "Annotate", "Edit", "Draw", "Forms", "Sign", "Pages", "Export",
            ] {
                let node = &nodes
                    .iter()
                    .find(|(_, n)| n.label() == Some(label))
                    .unwrap()
                    .1;
                let r = node.bounds().unwrap();
                assert!(
                    r.x0 >= previous && r.x1 <= width as f64,
                    "{label} is clipped or overlaps at width {width}: {r:?}"
                );
                previous = r.x1;
            }
            for label in [
                "Toggle left panel",
                "Toggle right panel",
                "Find",
                "Translate",
            ] {
                let r = nodes
                    .iter()
                    .find(|(_, n)| n.label() == Some(label))
                    .unwrap()
                    .1
                    .bounds()
                    .unwrap();
                assert!(
                    r.x0 >= previous && r.x1 <= width as f64,
                    "{label} is clipped"
                );
            }
            app.action("sidebar", &ctx);
            assert!(!app.sidebar && app.inspector);
            app.action("inspector", &ctx);
            assert!(!app.sidebar && !app.inspector);
            app.action("sidebar", &ctx);
            assert!(app.sidebar && !app.inspector);
        }
    }
}
