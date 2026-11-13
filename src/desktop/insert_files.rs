//! One insertion flow for dropped files, File → Insert Files and image tools.
use super::*;
use mypdf_milestone0::import_files::{self, Placement};

type NewDocument = Result<(PathBuf, Vec<u8>), String>;

#[derive(Default)]
pub(super) struct State {
    pub(super) draft: Option<Draft>,
    new_document: Option<mpsc::Receiver<NewDocument>>,
}
pub(super) struct Draft {
    paths: Vec<PathBuf>,
    target: Option<(u64, u64)>,
    page: usize,
    count: usize,
    before: usize,
    images: bool,
    all_images: bool,
    replace_blank: bool,
    blank: bool,
    pub(super) error: Option<String>,
}
impl Viewer {
    pub(super) fn pick_insert_files(
        &mut self,
        ctx: &egui::Context,
        at: Option<usize>,
        images_only: bool,
    ) {
        let extensions = if images_only {
            import_files::IMAGE_EXTENSIONS
        } else {
            import_files::EXTENSIONS
        };
        if let Some(paths) =
            self.native_picks(rfd::FileDialog::new().set_title("Insert files").add_filter(
                if images_only {
                    "Images"
                } else {
                    "PDFs, images and documents"
                },
                extensions,
            ))
        {
            self.prepare_file_insert(paths, at, ctx);
        }
    }
    pub(super) fn prepare_file_insert(
        &mut self,
        paths: Vec<PathBuf>,
        at: Option<usize>,
        _ctx: &egui::Context,
    ) {
        if paths.is_empty() {
            return;
        }
        if self.content.busy || self.organizer.busy {
            self.status = Some((
                "Wait for the current operation before inserting files.".into(),
                false,
            ));
            return;
        }
        let existing = !self.home.visible && !self.tabs.is_empty();
        if existing && !self.page_edits_ready(false) {
            return;
        }
        let mut target = None;
        let (mut page, mut count, mut blank) = (0, 0, false);
        if existing {
            let tab = &self.tabs[self.active];
            let Some(session) = &tab.session else { return };
            if !session.editable || tab.signed || tab.signed_original.is_some() {
                self.status = Some((
                    "Insert files requires an editable, unsigned PDF.".into(),
                    true,
                ));
                return;
            }
            target = Some((tab.id, session.revision()));
            page = tab.page;
            count = tab.pages.len();
            blank = session.annotations.is_empty() && import_files::is_blank(&session.source);
        }
        let all_images = paths.iter().all(|p| import_files::is_image(p));
        let error = import_files::validate_paths(&paths)
            .err()
            .map(|e| format!("{e:#}"));
        self.file_insert.draft = Some(Draft {
            paths,
            target,
            page,
            count,
            before: at.unwrap_or((page + 1).min(count)) + 1,
            images: target.is_some() && all_images && at.is_none(),
            all_images,
            replace_blank: blank,
            blank,
            error,
        });
    }
    pub(super) fn file_insert_dialog(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.file_insert.new_document {
            match rx.try_recv() {
                Ok(result) => {
                    self.file_insert.new_document = None;
                    self.organizer.busy = false;
                    match result {
                        Ok((name, bytes)) => {
                            if self.tabs.len() >= 16 {
                                self.status = Some((
                                    "Close a tab, then insert again (16-tab limit).".into(),
                                    true,
                                ));
                            } else {
                                let mut unique = name.clone();
                                let mut suffix = 1;
                                while unique.exists() || self.tabs.iter().any(|t| t.path == unique)
                                {
                                    unique = PathBuf::from(format!(
                                        "{} - imported {suffix}.pdf",
                                        name.file_stem().unwrap_or_default().to_string_lossy()
                                    ));
                                    suffix += 1;
                                }
                                self.open_memory(unique, bytes, ctx);
                            }
                        }
                        Err(e) => self.status = Some((e, true)),
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.file_insert.new_document = None;
                    self.organizer.busy = false;
                    self.status = Some(("File insertion stopped. Please try again.".into(), true));
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let Some(mut draft) = self.file_insert.draft.take() else {
            return;
        };
        let current = draft.target.is_none_or(|(id, revision)| {
            self.tabs
                .iter()
                .any(|t| t.id == id && t.session.as_ref().is_some_and(|s| s.revision() == revision))
        });
        let mut insert = false;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new("insert_files")).frame(ui::modal_frame()).show(ctx, |ui| {
            ui.set_width(ui::dialog_width(ctx, 480.));
            ui.heading(if draft.target.is_some() {"Insert files"} else {"Create PDF from files"});
            if let Some((id, _)) = draft.target && let Some(tab) = self.tabs.iter().find(|t| t.id == id) { ui.label(tab.name()); }
            ui.add_space(8.);
            let mut move_item = None;
            egui::ScrollArea::vertical().max_height(170.).show(ui, |ui| {
                for (i, path) in draft.paths.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.add_enabled(i > 0, egui::Button::new("↑")).on_hover_text("Move earlier").clicked() { move_item = Some((i, i-1)); }
                        if ui.add_enabled(i+1 < draft.paths.len(), egui::Button::new("↓")).on_hover_text("Move later").clicked() { move_item = Some((i, i+1)); }
                        ui.label(path.file_name().unwrap_or_default().to_string_lossy()).on_hover_text(path.display().to_string());
                    });
                }
            });
            if let Some((a,b)) = move_item { draft.paths.swap(a,b); }
            ui.separator();
            if draft.target.is_some() {
                if draft.all_images {
                    ui.radio_value(&mut draft.images, true, format!("Place on page {} · editable images", draft.page+1));
                    ui.radio_value(&mut draft.images, false, "Insert as new pages");
                }
                if !draft.images {
                    ui.horizontal(|ui| {
                        ui.label("Before page");
                        ui.add(egui::DragValue::new(&mut draft.before).range(1..=draft.count+1));
                        ui.small(format!("{} = append", draft.count+1));
                    });
                    if draft.blank { ui.checkbox(&mut draft.replace_blank, "Replace the empty page"); }
                }
            } else { ui.label("Each file becomes one or more pages, in the order above."); }
            if draft.paths.iter().any(|p| matches!(import_files::extension(p).as_str(), "gif"|"webp"|"tif"|"tiff")) {
                ui.small("GIF and WebP use the first frame. TIFF includes all pages.");
            }
            if import_files::needs_office(&draft.paths) && draft.error.is_none() {
                ui.small("DOCX and RTF use installed Microsoft Word or LibreOffice. Fonts and layout follow the installed application.");
            }
            if let Some(error) = &draft.error { ui.add_space(8.); ui.label(error); }
            if !current { ui.label("The document changed. Cancel and insert again to use the latest version."); }
            ui.add_space(10.);
            ui.horizontal(|ui| {
                insert = ui.add_enabled(current && draft.error.is_none() && !self.organizer.busy && !self.content.busy, egui::Button::new(if draft.target.is_some() {"Insert"} else {"Create PDF"})).clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if insert {
            self.start_file_insert(draft, ctx);
        } else if !cancel && !modal.should_close() {
            self.file_insert.draft = Some(draft);
        }
    }
    fn start_file_insert(&mut self, draft: Draft, ctx: &egui::Context) {
        let snapshot = if let Some((id, revision)) = draft.target {
            let Some(index) = self.tabs.iter().position(|t| t.id == id) else {
                return;
            };
            self.active = index;
            if !self.page_edits_ready(false) {
                return;
            }
            let Some(s) = &self.tabs[index].session else {
                return;
            };
            if s.revision() != revision {
                self.status = Some(("The document changed. Insert the files again.".into(), true));
                return;
            }
            Some((s.source.clone(), s.annotations.clone(), revision))
        } else {
            None
        };
        let placement = if draft.images {
            Placement::Images { page: draft.page }
        } else {
            Placement::Pages {
                at: draft.before - 1,
                replace_blank: draft.replace_blank,
            }
        };
        let page_tx = self.organizer.tx.clone();
        let new_tx = if draft.target.is_none() {
            let (tx, rx) = mpsc::channel();
            self.file_insert.new_document = Some(rx);
            Some(tx)
        } else {
            None
        };
        self.organizer.busy = true;
        self.organizer.change_name = "Insert files";
        self.status = Some(("Inserting files…".into(), false));
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || -> anyhow::Result<Vec<u8>> {
                    let bytes = match snapshot {
                        Some((source, marks, revision)) => Some(if revision == 0 {
                            source.to_vec()
                        } else {
                            mypdf_milestone0::annotations::export(&source, &marks)?
                        }),
                        None => None,
                    };
                    import_files::insert(bytes.as_deref(), &draft.paths, placement)
                },
            ))
            .map_err(|_| {
                "These files could not be processed. Your document is unchanged.".to_string()
            })
            .and_then(|r| r.map_err(|e| format!("{e:#}")));
            if let Some((id, revision)) = draft.target {
                let _ = page_tx.send(result.map(|bytes| (id, revision, bytes, None)));
            } else if let Some(tx) = new_tx {
                let name = draft
                    .paths
                    .first()
                    .and_then(|p| p.file_stem())
                    .map(|s| PathBuf::from(s).with_extension("pdf"))
                    .unwrap_or_else(|| PathBuf::from("Imported.pdf"));
                let _ = tx.send(result.map(|bytes| (name, bytes)));
            }
            wake.request_repaint();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;
    fn wait(app: &mut Viewer, ctx: &egui::Context, ready: impl Fn(&Viewer) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !ready(app) {
            let _ = ctx.run(
                egui::RawInput {
                    max_texture_side: Some(8192),
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    #[test]
    fn file_insert_is_atomic_undoable_and_rejects_stale_target() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("insert.txt");
        std::fs::write(&path, "Inserted page").unwrap();
        let blank = mypdf_milestone0::convert::blank(300., 400.).unwrap();
        app.open_memory(PathBuf::from("Insert test.pdf"), blank.clone(), &ctx);
        wait(&mut app, &ctx, |a| a.tabs[0].session.is_some());
        app.prepare_file_insert(vec![path.clone()], None, &ctx);
        let draft = app.file_insert.draft.take().unwrap();
        assert!(draft.replace_blank);
        app.start_file_insert(draft, &ctx);
        wait(&mut app, &ctx, |a| !a.organizer.busy);
        let s = app.tabs[0].session.as_mut().unwrap();
        assert_eq!(s.undo_label().as_deref(), Some("Insert files"));
        assert!(
            lopdf::Document::load_mem(&s.source)
                .unwrap()
                .extract_text(&[1])
                .unwrap()
                .contains("Inserted page")
        );
        s.undo();
        assert_eq!(s.source.as_ref(), blank);
        s.redo();
        wait(&mut app, &ctx, |a| {
            !a.content.busy && a.tabs[0].session.as_ref().unwrap().editable
        });
        app.prepare_file_insert(vec![path], None, &ctx);
        let stale = app.file_insert.draft.take().unwrap_or_else(|| {
            panic!(
                "{:?}; editable={}; signed={}; warning={:?}",
                app.status,
                app.tabs[0].session.as_ref().unwrap().editable,
                app.tabs[0].signed,
                app.tabs[0].editing_warning
            )
        });
        app.tabs[0].session.as_mut().unwrap().undo();
        app.start_file_insert(stale, &ctx);
        assert!(!app.organizer.busy);
        assert!(app.status.as_ref().unwrap().0.contains("document changed"));
    }
}
