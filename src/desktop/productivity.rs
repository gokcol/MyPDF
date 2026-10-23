//! Local copy workflows and placement settings. All long PDF work runs off the UI thread.
use super::*;
use mypdf_milestone0::workflows::{self, BatchOptions, ShareOptions, Template};
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Preferences {
    guides: bool,
    recipe: BatchOptions,
}
#[derive(Clone, Copy, PartialEq)]
enum Panel {
    Settings,
    Templates,
    Batch,
    Share,
}
struct Preview {
    bytes: Vec<u8>,
    texture: egui::TextureHandle,
    summary: String,
}
enum Event {
    Failure(String),
    Snapshot(Result<Vec<u8>, String>),
    TemplateSaved(Template),
    TemplateOpened(Template, Vec<u8>),
    Saved(String),
    Preview(Result<(Vec<u8>, egui::ColorImage, String), String>),
    Row(String),
    Done,
}
pub(super) struct State {
    prefs: Preferences,
    panel: Option<Panel>,
    templates: Vec<Template>,
    name: String,
    delete: Option<String>,
    files: Vec<PathBuf>,
    destination: Option<PathBuf>,
    preview: Option<Preview>,
    preview_index: usize,
    share: ShareOptions,
    source: Option<Vec<u8>>,
    source_name: String,
    source_style: Option<workflows::TextDefaults>,
    busy: bool,
    cancel: Arc<mypdf_milestone0::ocr::Cancellation>,
    tx: mpsc::Sender<Event>,
    rx: mpsc::Receiver<Event>,
    message: String,
    results: Vec<String>,
}
fn root() -> PathBuf {
    #[cfg(test)]
    {
        std::env::temp_dir().join(format!("mypdf-productivity-tests-{}", std::process::id()))
    }
    #[cfg(not(test))]
    {
        mypdf_milestone0::assets::data_directory()
    }
}
impl Default for State {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        let prefs = preferences::load(&root(), "workflow-settings.json");
        let templates = preferences::load(&root(), "templates.json");
        let message = prefs
            .as_ref()
            .err()
            .or(templates.as_ref().err())
            .map(|e| format!("Could not load workflow settings: {e:#}"))
            .unwrap_or_default();
        Self {
            prefs: prefs.unwrap_or_default(),
            templates: templates.unwrap_or_default(),
            panel: None,
            name: String::new(),
            delete: None,
            files: vec![],
            destination: None,
            preview: None,
            preview_index: 0,
            share: Default::default(),
            source: None,
            source_name: String::new(),
            source_style: None,
            busy: false,
            cancel: Arc::default(),
            tx,
            rx,
            message,
            results: vec![],
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
impl State {
    #[cfg(test)]
    pub(super) fn is_busy(&self) -> bool {
        self.busy
    }
    pub(super) fn owns_input(&self) -> bool {
        self.panel.is_some()
    }
    fn save(&mut self) {
        if let Err(e) = preferences::save(&root(), "workflow-settings.json", &self.prefs) {
            self.message = format!("Could not save settings: {e:#}");
        }
    }
    fn save_templates(&mut self) -> anyhow::Result<()> {
        preferences::save(&root(), "templates.json", &self.templates)
    }
    fn job(
        &mut self,
        ctx: &egui::Context,
        message: &str,
        task: impl FnOnce(Arc<mypdf_milestone0::ocr::Cancellation>) -> anyhow::Result<Event>
        + Send
        + 'static,
    ) {
        self.busy = true;
        self.message = message.into();
        self.cancel = Arc::default();
        let cancel = self.cancel.clone();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let event = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| task(cancel)))
                .unwrap_or_else(|_| {
                    Err(anyhow::anyhow!(
                        "Could not process this PDF; originals are unchanged"
                    ))
                })
                .unwrap_or_else(|e| Event::Failure(format!("{e:#}")));
            let _ = tx.send(event);
            ctx.request_repaint();
        });
    }
    fn preview(&mut self, ctx: &egui::Context) {
        let is_share = self.panel == Some(Panel::Share);
        let source = self.source.clone();
        let path = self.files.get(self.preview_index).cloned();
        let options = self.prefs.recipe.clone();
        let share = self.share;
        self.preview = None;
        self.busy = true;
        self.message = "Preparing preview…".into();
        self.cancel = Arc::default();
        let cancel = self.cancel.clone();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||->anyhow::Result<_>{
                let bytes=if is_share { source.ok_or_else(||anyhow::anyhow!("No document snapshot"))? } else { mypdf_milestone0::convert::read_bounded(&path.ok_or_else(||anyhow::anyhow!("Choose a PDF"))?,viewer::MAX_FILE_BYTES)? };
                let (out,summary)=if is_share {
                    let (out,r)=workflows::sharing_copy(&bytes,share)?;
                    (out,format!("Removed: {} notes, {} comment fields, {} attachment references, {} metadata entries.",r.notes,r.comment_fields,r.attachment_entries,r.metadata_entries))
                } else { let out=workflows::process(&bytes,&options,&cancel)?;let summary=format!("{} KB → {} KB · Preview of the first page",bytes.len()/1024,out.len()/1024);(out,summary) };
                cancel.check()?;
                let (size,rgba)=viewer::render_page(&out,0,0.8)?;
                cancel.check()?;
                Ok((out,egui::ColorImage::from_rgba_unmultiplied(size,&rgba),summary))
            })).map_err(|_|"This PDF could not be processed; originals are unchanged".to_string()).and_then(|r|r.map_err(|e|format!("{e:#}")));
            let _ = tx.send(Event::Preview(result));
            ctx.request_repaint();
        });
    }
    fn run_batch(&mut self, ctx: &egui::Context) {
        let Some(destination) = self.destination.clone() else {
            return;
        };
        self.busy = true;
        self.results.clear();
        self.message = "Processing saved files…".into();
        self.cancel = Arc::default();
        let cancel = self.cancel.clone();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let files = self.files.clone();
        let options = self.prefs.recipe.clone();
        std::thread::spawn(move || {
            for path in files {
                if cancel.is_cancelled() {
                    break;
                }
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                    || -> anyhow::Result<PathBuf> {
                        let bytes =
                            mypdf_milestone0::convert::read_bounded(&path, viewer::MAX_FILE_BYTES)?;
                        let out = workflows::process(&bytes, &options, &cancel)?;
                        cancel.check()?;
                        workflows::batch_output(&path, &destination, &out)
                    },
                ))
                .unwrap_or_else(|_| {
                    Err(anyhow::anyhow!(
                        "This PDF could not be processed; original unchanged"
                    ))
                });
                let line = match result {
                    Ok(out) => format!("✓ {}", out.display()),
                    Err(e) => format!("{}: {e:#}", path.display()),
                };
                let _ = tx.send(Event::Row(line));
                ctx.request_repaint();
            }
            let _ = tx.send(Event::Done);
            ctx.request_repaint();
        });
    }
}
impl State {
    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        if ui.checkbox(&mut self.prefs.guides, "Snap to detected table edges and text baselines")
            .on_hover_text("Optional placement guides while dragging. Vector table edges only; scanned table lines are not detected.")
            .changed()
        {
            self.save();
        }
    }

    fn templates_ui(&mut self, ui: &mut egui::Ui, template_open: &mut Option<Template>) {
        ui.label("Templates store this document's pages, forms, placed artwork and current Add text style. Opening one always creates a new, unsaved copy.");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.name)
                    .hint_text("Template name")
                    .char_limit(80),
            );
            if ui
                .add_enabled(
                    self.source.is_some() && self.templates.len() < 100,
                    egui::Button::new("Save current as template"),
                )
                .clicked()
            {
                let name = self.name.clone();
                let source = self.source.clone().unwrap();
                let style = self.source_style.clone();
                self.job(ui.ctx(), "Saving template…", move |cancel| {
                    cancel.check()?;
                    let mut item =
                        workflows::save_template(&root().join("templates"), &name, &source)?;
                    item.text_defaults = style;
                    Ok(Event::TemplateSaved(item))
                });
            }
        });
        ui.separator();
        let mut dirty = false;
        let mut remove = None;
        for item in &mut self.templates {
            ui.push_id(item.id.clone(), |ui| {
                ui.horizontal(|ui| {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut item.name)
                            .char_limit(80)
                            .desired_width(240.),
                    );
                    dirty |= response.changed();
                    if ui.button("New copy").clicked() {
                        *template_open = Some(item.clone());
                    }
                    if ui.button("Delete…").clicked() {
                        self.delete = Some(item.id.clone());
                    }
                });
                if self.delete.as_ref() == Some(&item.id) {
                    ui.horizontal(|ui| {
                        ui.label("Delete this template from the library?");
                        if ui.button("Delete template").clicked() {
                            remove = Some(item.clone());
                        }
                        if ui.button("Keep").clicked() {
                            self.delete = None;
                        }
                    });
                }
            });
        }
        if let Some(item) = remove {
            match workflows::delete_template(&root().join("templates"), &item) {
                Ok(()) => {
                    self.templates.retain(|t| t.id != item.id);
                    dirty = true;
                    self.delete = None;
                }
                Err(e) => self.message = format!("{e:#}"),
            }
        }
        if dirty && let Err(e) = self.save_templates() {
            self.message = format!("Could not save template names: {e:#}");
        }
        if self.templates.is_empty() {
            ui.weak("No templates yet. Open a document to save your first one.");
        }
    }

    fn batch_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label("Process saved PDFs locally. Originals remain unchanged; each result goes to a different folder. Signed or encrypted PDFs are skipped with an explanation.");
        ui.horizontal(|ui| {
            if ui.button("Choose PDFs…").clicked()
                && let Some(files) = rfd::FileDialog::new()
                    .add_filter("PDF", &["pdf"])
                    .pick_files()
            {
                if files.len() > 50 {
                    self.message = "Choose at most 50 PDFs per batch".into();
                } else {
                    self.files = files;
                    self.preview = None;
                    self.preview_index = 0;
                    self.results.clear();
                }
            }
            if ui.button("Output folder…").clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                self.destination = Some(path);
            }
        });
        ui.weak(
            self.destination
                .as_ref()
                .map_or("Choose a separate output folder".into(), |p| {
                    p.display().to_string()
                }),
        );
        let before = self.prefs.recipe.clone();
        ui.add_enabled(
            mypdf_milestone0::ocr::available(),
            egui::Checkbox::new(&mut self.prefs.recipe.ocr, "Recognize text (on-device OCR)"),
        )
        .on_hover_text("OCR is available on macOS. No cloud service is used.");
        ui.checkbox(
            &mut self.prefs.recipe.numbers,
            "Page numbers — bottom center, page / total",
        );
        egui::ComboBox::from_label("Compression")
            .selected_text(self.prefs.recipe.compression.map_or("Off", |i| {
                mypdf_milestone0::compress::Level::ALL
                    .get(i)
                    .map_or("Off", |l| l.label())
            }))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.prefs.recipe.compression, None, "Off");
                for (i, level) in mypdf_milestone0::compress::Level::ALL.iter().enumerate() {
                    ui.selectable_value(&mut self.prefs.recipe.compression, Some(i), level.label());
                }
            });
        ui.horizontal(|ui| {
            ui.label("Watermark");
            ui.add(
                egui::TextEdit::singleline(&mut self.prefs.recipe.watermark)
                    .hint_text("Leave empty for none")
                    .char_limit(128),
            );
        });
        if !self.prefs.recipe.watermark.is_empty() {
            ui::numeric_slider(
                ui,
                egui::Slider::new(&mut self.prefs.recipe.opacity, 0.05..=1.)
                    .text("Watermark opacity"),
            );
        }
        if before != self.prefs.recipe {
            self.preview = None;
            self.save();
        }
        if !self.files.is_empty() {
            egui::ComboBox::from_label("Preview file")
                .selected_text(
                    self.files
                        .get(self.preview_index)
                        .map_or(String::new(), |p| {
                            p.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned()
                        }),
                )
                .show_ui(ui, |ui| {
                    for (i, path) in self.files.iter().enumerate() {
                        if ui
                            .selectable_value(
                                &mut self.preview_index,
                                i,
                                path.file_name().unwrap_or_default().to_string_lossy(),
                            )
                            .changed()
                        {
                            self.preview = None;
                        }
                    }
                });
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.files.is_empty() && self.prefs.recipe.active(),
                    egui::Button::new("Preview result"),
                )
                .clicked()
            {
                self.preview(ctx);
            }
            if ui
                .add_enabled(
                    self.preview.is_some() && self.destination.is_some(),
                    egui::Button::new(format!("Create {} copies", self.files.len())),
                )
                .clicked()
            {
                self.run_batch(ctx);
            }
        });
        ui.weak("Preview a representative file before running. Existing outputs are kept; new copies receive numbered filenames.");
    }

    fn share_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(format!("Source snapshot: {}", self.source_name));
        ui.label("Creates a separate PDF. Review the complete output before sending it.");
        let mut changed = ui
            .checkbox(
                &mut self.share.comments,
                "Remove notes and comment text (retain visible highlights and drawings)",
            )
            .changed();
        changed |= ui
            .checkbox(
                &mut self.share.attachments,
                "Remove embedded file attachments",
            )
            .changed();
        changed |= ui
            .checkbox(
                &mut self.share.metadata,
                "Remove document metadata and annotation author/date details",
            )
            .changed();
        if changed {
            self.preview = None;
        }
        ui.weak("This is not redaction: page content, form values, bookmarks and hidden layers remain. MyPDF's private object-editing data is removed when cleaning comments or metadata; visible artwork remains.");
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.source.is_some(),
                    egui::Button::new("Review cleaned copy"),
                )
                .clicked()
            {
                self.preview(ctx);
            }
            if ui
                .add_enabled(
                    self.preview.is_some(),
                    egui::Button::new("Save sharing copy…"),
                )
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("PDF", &["pdf"])
                    .set_file_name("Shared copy.pdf")
                    .save_file()
            {
                let bytes = self.preview.as_ref().unwrap().bytes.clone();
                self.job(ctx, "Saving sharing copy…", move |cancel| {
                    cancel.check()?;
                    mypdf_milestone0::write_new(&path, &bytes)?;
                    Ok(Event::Saved(format!("Saved {}", path.display())))
                });
            }
        });
    }
}

impl Viewer {
    pub(super) fn productivity_action(&mut self, action: &str, ctx: &egui::Context) -> bool {
        let panel = match action {
            "workflow_settings" => Panel::Settings,
            "templates" => Panel::Templates,
            "batch" => Panel::Batch,
            "sharing_copy" => Panel::Share,
            _ => return false,
        };
        if self.productivity.busy || self.content.busy || self.organizer.busy || self.signature_busy
        {
            self.status = Some((
                "Wait for the current workflow or cancel it first".into(),
                true,
            ));
            return true;
        }
        if matches!(panel, Panel::Templates | Panel::Share) && self.content.pending_text().is_some()
        {
            self.status = Some((
                "Finish the current text edit before creating a copy".into(),
                true,
            ));
            return true;
        }
        self.productivity.panel = Some(panel);
        self.productivity.preview = None;
        self.productivity.source = None;
        self.productivity.source_style = Some(self.content.text_defaults());
        self.productivity.message.clear();
        if matches!(panel, Panel::Templates | Panel::Share)
            && let Some(tab) = self.tabs.get(self.active).filter(|_| !self.home.visible)
            && let Some(session) = &tab.session
        {
            self.productivity.source_name = tab.name();
            let source = session.source.clone();
            let annotations = session.annotations.clone();
            let revision = session.revision();
            self.productivity
                .job(ctx, "Preparing document snapshot…", move |cancel| {
                    cancel.check()?;
                    let bytes = if revision == 0 {
                        source.to_vec()
                    } else {
                        mypdf_milestone0::annotations::export(&source, &annotations)?
                    };
                    cancel.check()?;
                    Ok(Event::Snapshot(Ok(bytes)))
                });
        }
        true
    }
    pub(super) fn productivity_windows(&mut self, ctx: &egui::Context) {
        let mut opened = None;
        let events: Vec<_> = self.productivity.rx.try_iter().collect();
        for event in events {
            match event {
                Event::Failure(error) => {
                    self.productivity.busy = false;
                    self.productivity.message = error;
                }
                Event::Saved(message) => {
                    self.productivity.busy = false;
                    self.productivity.message = message;
                }
                Event::Snapshot(result) => {
                    self.productivity.busy = false;
                    match result {
                        Ok(bytes) => {
                            self.productivity.source = Some(bytes);
                            self.productivity.message.clear();
                            if self.productivity.panel == Some(Panel::Share) {
                                self.productivity.preview(ctx);
                            }
                        }
                        Err(e) => self.productivity.message = e,
                    }
                }
                Event::TemplateSaved(item) => {
                    self.productivity.busy = false;
                    self.productivity.templates.push(item.clone());
                    if let Err(e) = self.productivity.save_templates() {
                        self.productivity.templates.pop();
                        let _ = workflows::delete_template(&root().join("templates"), &item);
                        self.productivity.message = format!("Could not save template index: {e:#}");
                    } else {
                        self.productivity.name.clear();
                        self.productivity.message = "Template saved".into();
                    }
                }
                Event::TemplateOpened(item, bytes) => {
                    self.productivity.busy = false;
                    opened = Some((item, bytes));
                }
                Event::Preview(result) => {
                    self.productivity.busy = false;
                    match result {
                        Ok((bytes, image, summary)) => {
                            self.productivity.preview = Some(Preview {
                                bytes,
                                texture: ctx.load_texture(
                                    "workflow_preview",
                                    image,
                                    Default::default(),
                                ),
                                summary,
                            });
                            self.productivity.message.clear();
                        }
                        Err(e) => self.productivity.message = e,
                    }
                }
                Event::Row(row) => self.productivity.results.push(row),
                Event::Done => {
                    self.productivity.busy = false;
                    self.productivity.message = if self.productivity.cancel.is_cancelled() {
                        "Canceled. Completed copies remain in the output folder."
                    } else {
                        "Batch finished. Review the per-file results below."
                    }
                    .into();
                }
            }
        }
        let guides = self.productivity.prefs.guides;
        if self.content.layout_guides != guides {
            self.content.set_layout_guides(guides);
            for tab in &mut self.tabs {
                tab.placement_guides = None;
            }
        }
        let Some(panel) = self.productivity.panel else {
            return;
        };
        let mut open = !ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
            || self.productivity.busy;
        let mut template_open = None;
        let title = match panel {
            Panel::Settings => "Workflow settings",
            Panel::Templates => "Document templates",
            Panel::Batch => "Batch processing",
            Panel::Share => "Prepare copy for sharing",
        };
        let state = &mut self.productivity;
        egui::Modal::new(egui::Id::new("productivity_window")).show(ctx, |ui| {
            ui.set_width(ui::dialog_width(ctx, 780.));
            ui.set_max_height((ctx.content_rect().height() - 60.).max(200.));
            ui.spacing_mut().item_spacing.y = 6.;
            ui.horizontal(|ui| {
                ui.heading(title);
                if ui
                    .add_enabled(!state.busy, egui::Button::new("Done"))
                    .clicked()
                {
                    open = false;
                }
            });
            ui.separator();
            if state.busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(&state.message);
                    if ui.button("Cancel").clicked() {
                        state.cancel.cancel();
                        state.message = "Canceling after the current operation…".into();
                    }
                });
            }
            egui::ScrollArea::vertical()
                .max_height((ctx.content_rect().height() - 180.).max(180.))
                .show(ui, |ui| {
                    ui.add_enabled_ui(!state.busy, |ui| match panel {
                        Panel::Settings => state.settings_ui(ui),
                        Panel::Templates => state.templates_ui(ui, &mut template_open),
                        Panel::Batch => state.batch_ui(ui, ctx),
                        Panel::Share => state.share_ui(ui, ctx),
                    });
                    if !state.message.is_empty() {
                        ui.label(&state.message);
                    }
                    if let Some(preview) = &state.preview {
                        ui.separator();
                        ui.label(&preview.summary);
                        if ui
                            .add_enabled(
                                !state.busy,
                                egui::Button::new("Open full preview as a new PDF"),
                            )
                            .clicked()
                        {
                            template_open = Some(Template {
                                id: String::new(),
                                name: "Workflow preview.pdf".into(),
                                text_defaults: None,
                            });
                        }
                        ui.add(
                            egui::Image::new(&preview.texture)
                                .max_width(ui.available_width().min(520.))
                                .max_height(360.)
                                .shrink_to_fit(),
                        );
                    }
                    for row in &state.results {
                        ui.label(row);
                    }
                });
        });
        if !open {
            if state.busy {
                state.message = "Cancel the current operation before closing".into();
            } else {
                state.panel = None;
                state.preview = None;
                state.source = None;
            }
        }
        let guides = state.prefs.guides;
        if self.content.layout_guides != guides {
            self.content.set_layout_guides(guides);
            for tab in &mut self.tabs {
                tab.placement_guides = None;
            }
        }
        if let Some(item) = template_open {
            if item.id.is_empty() {
                if let Some(preview) = &self.productivity.preview {
                    opened = Some((item, preview.bytes.clone()));
                }
            } else {
                self.productivity
                    .job(ctx, "Opening template copy…", move |cancel| {
                        cancel.check()?;
                        let bytes = workflows::read_template(&root().join("templates"), &item)?;
                        cancel.check()?;
                        Ok(Event::TemplateOpened(item, bytes))
                    });
            }
        }
        if let Some((item, bytes)) = opened {
            self.productivity.panel = None;
            self.productivity.source = None;
            self.productivity.preview = None;
            if let Some(style) = &item.text_defaults {
                self.content.apply_text_defaults(style);
            }
            let name = item.name.replace(['/', '\\'], "-");
            self.open_memory(
                PathBuf::from(format!("{}.pdf", name.trim_end_matches(".pdf"))),
                bytes,
                ctx,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Viewer, egui::Context) {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        app.home.visible = false;
        (app, ctx)
    }
    #[test]
    fn sharing_preview_and_dialog_commands_never_edit_source() {
        let (mut app, ctx) = fixture();
        let note = mypdf_milestone0::annotations::Annotation::new(
            0,
            [50., 50., 80., 80.],
            mypdf_milestone0::annotations::Kind::Note {
                text: "Private review".into(),
            },
            Default::default(),
        );
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![note])
            .unwrap();
        let before = app.tabs[0]
            .session
            .as_ref()
            .unwrap()
            .snapshot_bytes()
            .unwrap();
        let revision = app.tabs[0].session.as_ref().unwrap().revision();
        app.productivity_action("sharing_copy", &ctx);
        assert!(app.workbench_owns_shortcuts());
        app.action("undo", &ctx);
        app.action("new", &ctx);
        assert_eq!(app.tabs.len(), 1);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while app.productivity.busy {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1000., 760.))),
                    ..Default::default()
                },
                |ctx| app.productivity_windows(ctx),
            );
            assert!(
                std::time::Instant::now() < deadline,
                "{}",
                app.productivity.message
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let preview = app.productivity.preview.as_ref().expect("cleaned preview");
        let doc = lopdf::Document::load_mem(&preview.bytes).unwrap();
        let page = *doc.get_pages().values().next().unwrap();
        assert!(
            doc.get_dictionary(page)
                .unwrap()
                .get(b"Annots")
                .unwrap()
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            app.tabs[0]
                .session
                .as_ref()
                .unwrap()
                .snapshot_bytes()
                .unwrap(),
            before
        );
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), revision);
    }
    #[test]
    fn batch_reports_each_file_and_creates_new_outputs_only() {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("out");
        std::fs::create_dir(&dest).unwrap();
        let good = root.path().join("good.pdf");
        let bad = root.path().join("bad.pdf");
        let bytes = mypdf_milestone0::convert::blank(300., 400.).unwrap();
        std::fs::write(&good, &bytes).unwrap();
        std::fs::write(&bad, b"broken PDF").unwrap();
        let mut state = State::default();
        state.files = vec![good.clone(), bad.clone()];
        state.destination = Some(dest.clone());
        state.prefs.recipe.numbers = true;
        state.run_batch(&egui::Context::default());
        let mut rows = vec![];
        loop {
            match state
                .rx
                .recv_timeout(std::time::Duration::from_secs(20))
                .unwrap()
            {
                Event::Row(row) => rows.push(row),
                Event::Done => break,
                _ => panic!("unexpected preview"),
            }
        }
        assert_eq!(rows.len(), 2);
        assert!(rows[0].starts_with('✓'));
        assert!(rows[1].contains("bad.pdf"));
        assert!(dest.join("good-processed.pdf").exists());
        assert!(!dest.join("bad-processed.pdf").exists());
        assert_eq!(std::fs::read(good).unwrap(), bytes);
        assert_eq!(std::fs::read(bad).unwrap(), b"broken PDF");
    }
}
