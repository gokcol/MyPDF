//! Autosave work runs on one serial I/O worker, independent of rendering.
use super::*;
use mypdf_milestone0::{
    annotations,
    recovery::{Entry, Store},
};
use std::time::{Duration, Instant};

enum Command {
    Write {
        id: u64,
        path: PathBuf,
        revision: u64,
        page: usize,
        source: Arc<[u8]>,
        annotations: Vec<annotations::Annotation>,
        /// The signed file that edits of an unlocked signed PDF are appended to.
        original: Option<Arc<[u8]>>,
    },
    Clear(u64),
    Restore(Entry),
    Remove(Entry),
}
enum Event {
    Scanned(Vec<Entry>, Vec<String>),
    Written(u64, u64, Result<(), String>),
    Restored(Entry, Result<RestoredPdf, String>),
    Error(String),
}
struct RestoredPdf {
    bytes: Vec<u8>,
    signed: bool,
}
impl RestoredPdf {
    fn prepare(bytes: Vec<u8>) -> anyhow::Result<Self> {
        let doc = lopdf::Document::load_mem(&bytes)?;
        anyhow::ensure!(!doc.is_encrypted(), "Encrypted recovery PDF");
        let caps = annotations::inspect(&doc);
        let signed = caps.signed || caps.certified;
        if signed {
            // These local snapshots contain edits the user already enabled.
            // Keep the complete recovered revision as the incremental-save
            // baseline so neither earlier signatures nor unsaved edits are lost.
            mypdf_milestone0::signature_edit::unlock(&bytes)?;
        }
        mypdf_milestone0::content::editable(&bytes)?;
        Ok(Self { bytes, signed })
    }
}
struct Tracking {
    revision: u64,
    changed: Instant,
    first: Instant,
    written: Option<u64>,
}

pub(super) struct Recovery {
    tx: mpsc::Sender<Command>,
    rx: mpsc::Receiver<Event>,
    tracking: HashMap<u64, Tracking>,
    origins: HashMap<u64, Entry>,
    pending: HashMap<u64, Entry>,
    entries: Vec<Entry>,
    busy: bool,
    pub open: bool,
    pub error: Option<String>,
    discard: Option<Entry>,
}
impl Recovery {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, commands) = mpsc::channel();
        let (events, rx) = mpsc::channel();
        let wake = ctx.clone();
        std::thread::spawn(move || {
            #[cfg(test)]
            let temporary = tempfile::tempdir().unwrap();
            #[cfg(test)]
            let root = temporary.path().join("recovery");
            #[cfg(not(test))]
            let root = mypdf_milestone0::assets::data_directory().join("recovery");
            let mut store = match Store::new(&root) {
                Ok(s) => s,
                Err(e) => {
                    let _ = events.send(Event::Error(format!("Autosave unavailable: {e:#}")));
                    wake.request_repaint();
                    return;
                }
            };
            match store.scan() {
                Ok((e, w)) => {
                    let _ = events.send(Event::Scanned(e, w));
                }
                Err(e) => {
                    let _ = events.send(Event::Error(format!("Recovery scan failed: {e:#}")));
                }
            }
            wake.request_repaint();
            for command in commands {
                match command {
                    Command::Write {
                        id,
                        path,
                        revision,
                        page,
                        source,
                        annotations,
                        original,
                    } => {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                            || -> anyhow::Result<()> {
                                let mut bytes = if revision == 0 {
                                    source.to_vec()
                                } else {
                                    annotations::export(&source, &annotations)?
                                };
                                if let Some(original) = original {
                                    bytes = mypdf_milestone0::signature_edit::incremental_update(
                                        &original, &bytes,
                                    )?;
                                }
                                store.write(id, path, revision, page, &bytes)
                            },
                        ))
                        .map_err(|_| "Recovery worker could not process this PDF".to_owned())
                        .and_then(|r| r.map_err(|e| format!("{e:#}")));
                        let _ = events.send(Event::Written(id, revision, result));
                    }
                    Command::Clear(id) => {
                        if let Err(e) = store.clear(id) {
                            let _ = events
                                .send(Event::Error(format!("Recovery cleanup failed: {e:#}")));
                        }
                    }
                    Command::Remove(e) => {
                        if let Err(e) = store.remove(&e) {
                            let _ = events
                                .send(Event::Error(format!("Recovery cleanup failed: {e:#}")));
                        }
                    }
                    Command::Restore(e) => {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            RestoredPdf::prepare(store.read(&e)?)
                        }))
                        .map_err(|_| {
                            "Could not process this recovery PDF; the copy was retained".to_owned()
                        })
                        .and_then(|r| r.map_err(|e| format!("{e:#}")));
                        let _ = events.send(Event::Restored(e, result));
                    }
                }
                wake.request_repaint();
            }
        });
        Self {
            tx,
            rx,
            tracking: HashMap::new(),
            origins: HashMap::new(),
            pending: HashMap::new(),
            entries: vec![],
            busy: false,
            open: false,
            error: None,
            discard: None,
        }
    }
    pub fn clear(&mut self, id: u64) {
        if self.pending.remove(&id).is_some() {
            self.busy = false;
        }
        self.tracking.remove(&id);
        let _ = self.tx.send(Command::Clear(id));
        if let Some(entry) = self.origins.remove(&id) {
            let _ = self.tx.send(Command::Remove(entry));
        }
    }
    pub(super) fn loaded(&mut self, id: u64) -> bool {
        let Some(entry) = self.pending.remove(&id) else {
            return false;
        };
        self.busy = false;
        self.error = None;
        self.entries.retain(|e| e.file != entry.file);
        self.origins.insert(id, entry);
        self.open = !self.entries.is_empty();
        true
    }
    pub(super) fn failed(&mut self, id: u64, error: &str) {
        if self.pending.remove(&id).is_some() {
            self.busy = false;
            self.error = Some(format!(
                "Could not restore: {error}. The recovery copy was retained; you can retry."
            ));
            self.open = true;
        }
    }
    pub fn label(&self, id: u64, revision: u64) -> &'static str {
        if self.error.is_some() {
            "Autosave needs attention"
        } else if self
            .tracking
            .get(&id)
            .is_some_and(|t| t.written == Some(revision))
        {
            "Recovery copy saved"
        } else {
            "Unsaved changes"
        }
    }
}
impl Viewer {
    pub(super) fn recovery_tick(&mut self, ctx: &egui::Context) {
        for event in self.recovery.rx.try_iter().collect::<Vec<_>>() {
            match event {
                Event::Scanned(entries, warnings) => {
                    self.recovery.open = !entries.is_empty();
                    self.recovery.entries = entries;
                    if !warnings.is_empty() {
                        self.recovery.error = Some(warnings.join("\n"));
                    }
                }
                Event::Error(e) => self.recovery.error = Some(e),
                Event::Written(id, revision, result) => {
                    self.recovery.busy = false;
                    if let Some(t) = self.recovery.tracking.get_mut(&id) {
                        t.first = Instant::now();
                        match result {
                            Ok(()) => {
                                t.written = Some(revision);
                                self.recovery.error = None;
                                if let Some(e) = self.recovery.origins.remove(&id) {
                                    let _ = self.recovery.tx.send(Command::Remove(e));
                                }
                            }
                            Err(e) => {
                                t.changed = Instant::now() + Duration::from_secs(25);
                                self.recovery.error =
                                    Some(format!("Autosave failed: {e}. Save to keep your work."));
                            }
                        }
                    }
                }
                Event::Restored(entry, result) => {
                    self.recovery.busy = false;
                    if !self.recovery.entries.iter().any(|e| e.file == entry.file) {
                        self.recovery.entries.push(entry.clone());
                    }
                    match result {
                        Ok(restored) if self.tabs.len() < 16 => {
                            let title = entry
                                .metadata
                                .original
                                .file_stem()
                                .unwrap_or_default()
                                .to_string_lossy();
                            let path = PathBuf::from(format!(
                                "{title} (Recovered {}).pdf",
                                self.next_id + 1
                            ));
                            self.open_memory(path, restored.bytes, ctx);
                            if let Some(tab) = self.tabs.get_mut(self.active) {
                                tab.page = entry.metadata.page;
                                tab.restore_view = false;
                                if restored.signed {
                                    tab.signed_original = tab.bytes.clone();
                                }
                                self.recovery.pending.insert(tab.id, entry);
                                self.recovery.busy = true;
                            }
                            // Loading is asynchronous. Keep the recovery entry
                            // until the renderer confirms that the tab opened.
                            self.status = Some(("Opening recovery copy…".into(), false));
                        }
                        Ok(_) => {
                            self.recovery.error =
                                Some("Close a tab before restoring another document.".into())
                        }
                        Err(e) => {
                            self.recovery.error = Some(format!(
                                "Could not restore: {e}. The recovery copy was retained; you can retry."
                            ));
                            self.recovery.open = true;
                        }
                    }
                }
            }
        }
        let clean: Vec<_> = self
            .recovery
            .tracking
            .keys()
            .copied()
            .filter(|id| {
                !self
                    .tabs
                    .iter()
                    .any(|t| t.id == *id && t.session.as_ref().is_some_and(|s| s.dirty()))
            })
            .collect();
        for id in clean {
            self.recovery.clear(id);
        }
        for tab in &self.tabs {
            let Some(s) = &tab.session else { continue };
            // A protected document's content is never written out unprotected.
            if !s.dirty() || tab.password.is_some() {
                continue;
            }
            let t = self
                .recovery
                .tracking
                .entry(tab.id)
                .or_insert_with(|| Tracking {
                    revision: s.revision(),
                    changed: Instant::now(),
                    first: Instant::now(),
                    written: None,
                });
            if t.revision != s.revision() {
                t.revision = s.revision();
                t.changed = Instant::now();
            }
            if t.written != Some(t.revision) {
                ctx.request_repaint_after(Duration::from_secs(1));
                if !self.recovery.busy
                    && (t.changed.elapsed() >= Duration::from_secs(5)
                        || t.first.elapsed() >= Duration::from_secs(30))
                {
                    self.recovery.busy = self
                        .recovery
                        .tx
                        .send(Command::Write {
                            id: tab.id,
                            path: tab.path.clone(),
                            revision: s.revision(),
                            page: tab.page,
                            source: s.source.clone(),
                            annotations: s.annotations.clone(),
                            original: tab.signed_original.clone(),
                        })
                        .is_ok();
                    if !self.recovery.busy {
                        self.recovery.error =
                            Some("Autosave worker unavailable. Save to keep your changes.".into());
                    }
                }
            }
        }
    }
    pub(super) fn recovery_window(&mut self, ctx: &egui::Context) {
        if !self.recovery.open {
            return;
        }
        let mut open = true;
        let mut restore = None;
        egui::Window::new("Document recovery")
            .open(&mut open)
            .default_width(460.)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(44., 44.), egui::Sense::hover());
                    ui::badge(ui.painter(), r, Icon::Reload, ui::Fill::Brand, Color32::WHITE);
                    ui.add_space(4.);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.;
                        ui::title(ui, "Pick up where you left off", 18.);
                        ui.label(
                            RichText::new(
                                "These local copies contain unsaved changes. Original PDFs are unchanged.",
                            )
                            .size(12.5)
                            .color(theme::muted()),
                        );
                    });
                });
                ui.add_space(12.);
                if let Some(e) = &self.recovery.error {
                    ui.colored_label(theme::danger(), e);
                }
                if self.recovery.entries.is_empty() {
                    ui.label("No recovery copies are waiting.");
                }
                egui::ScrollArea::vertical()
                    .max_height(330.)
                    .show(ui, |ui| {
                        for entry in &self.recovery.entries {
                            ui.push_id(&entry.file, |ui| {
                                egui::Frame::new()
                                    .fill(theme::paper())
                                    .stroke(Stroke::new(1., theme::line()))
                                    .corner_radius(12.)
                                    .inner_margin(12.)
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.label(
                                            RichText::new(
                                                entry
                                                    .metadata
                                                    .original
                                                    .file_name()
                                                    .unwrap_or_default()
                                                    .to_string_lossy(),
                                            )
                                            .family(ui::semibold()),
                                        );
                                        let time = time::OffsetDateTime::from_unix_timestamp(
                                            entry.metadata.updated as i64,
                                        )
                                        .map(|t| {
                                            format!(
                                                "{:04}-{:02}-{:02} · {:02}:{:02} UTC",
                                                t.year(),
                                                t.month() as u8,
                                                t.day(),
                                                t.hour(),
                                                t.minute()
                                            )
                                        })
                                        .unwrap_or_default();
                                        ui.label(RichText::new(time).small().color(theme::muted()));
                                        ui.add_enabled_ui(!self.recovery.busy, |ui| {
                                            ui.horizontal(|ui| {
                                                if ui
                                                    .add(
                                                        ActionButton::new(Icon::Undo, "Restore")
                                                            .primary(),
                                                    )
                                                    .clicked()
                                                {
                                                    restore = Some(entry.clone());
                                                }
                                                if ui
                                                    .add(ActionButton::new(
                                                        Icon::Trash,
                                                        "Discard copy",
                                                    ))
                                                    .clicked()
                                                {
                                                    self.recovery.discard = Some(entry.clone());
                                                }
                                            });
                                        });
                                    });
                                ui.add_space(8.);
                            });
                        }
                    });
                ui.label(
                    RichText::new(
                        "Close this window to decide later. View → Document Recovery reopens it.",
                    )
                    .small()
                    .color(theme::muted()),
                );
            });
        self.recovery.open = open;
        if let Some(e) = restore {
            self.recovery.error = None;
            self.recovery.busy = self.recovery.tx.send(Command::Restore(e)).is_ok();
        }
        if let Some(entry) = self.recovery.discard.clone() {
            match ui::confirmation(
                ctx,
                "discard_recovery",
                "Discard this recovery copy?",
                "Its unsaved changes will be removed. Your original PDF stays untouched.",
                &[
                    ("Keep Copy", ui::DialogTone::Primary),
                    ("Discard Copy", ui::DialogTone::Destructive),
                ],
                0,
            ) {
                Some(0) => self.recovery.discard = None,
                Some(1) => {
                    let _ = self.recovery.tx.send(Command::Remove(entry.clone()));
                    self.recovery.entries.retain(|e| e.file != entry.file);
                    self.recovery.discard = None;
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
impl Viewer {
    /// Whether autosave is keeping track of a tab's changes.
    pub(super) fn recovery_tracks(&self, id: u64) -> bool {
        self.recovery.tracking.contains_key(&id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn restore_fixture(
        bytes: &[u8],
    ) -> (
        tempfile::TempDir,
        Viewer,
        egui::Context,
        mpsc::Sender<Event>,
        Entry,
    ) {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path()).unwrap();
        store.write(1, "Original.pdf".into(), 1, 0, bytes).unwrap();
        drop(store);
        let mut store = Store::new(root.path()).unwrap();
        let entry = store.scan().unwrap().0.remove(0);
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let (tx, rx) = mpsc::channel();
        app.recovery.rx = rx;
        app.recovery.entries.push(entry.clone());
        app.recovery.open = true;
        (root, app, ctx, tx, entry)
    }

    #[test]
    fn signed_recovery_restores_editing_and_incremental_save_baseline() {
        let signed = super::super::signatures::signed_fixture(
            &mypdf_milestone0::convert::blank(595., 842.).unwrap(),
            "Recovery Desktop Signer",
        );
        assert!(mypdf_milestone0::content::editable(&signed).is_err());
        let (_root, mut app, ctx, tx, entry) = restore_fixture(&signed);
        app.recovery.error = Some("Previous failure".into());
        tx.send(Event::Restored(
            entry.clone(),
            Ok(RestoredPdf::prepare(signed.clone()).unwrap()),
        ))
        .unwrap();
        app.recovery_tick(&ctx);
        assert_eq!(
            app.recovery.entries.len(),
            1,
            "keep copy listed until loading succeeds"
        );
        assert!(app.recovery.busy);
        let deadline = Instant::now() + Duration::from_secs(15);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        let tab = &mut app.tabs[0];
        let session = tab.session.as_mut().unwrap();
        assert!(session.dirty() && session.editable && tab.signed);
        assert!(!tab.has_destination);
        assert_eq!(tab.signed_original.as_deref().unwrap(), signed);
        assert!(!app.recovery.open && !app.recovery.busy && app.recovery.error.is_none());
        assert!(app.recovery.entries.is_empty());
        assert!(
            entry.file.exists(),
            "only explicit save or discard removes recovery data"
        );
        let mut marks = session.annotations.clone();
        marks.push(annotations::Annotation::new(
            0,
            [10., 10., 80., 80.],
            annotations::Kind::Rectangle,
            Default::default(),
        ));
        session.commit(marks).unwrap();
        let edited = annotations::export(&session.source, &session.annotations).unwrap();
        let saved = mypdf_milestone0::signature_edit::incremental_update(
            tab.signed_original.as_ref().unwrap(),
            &edited,
        )
        .unwrap();
        assert!(saved.starts_with(&signed));
        let proof = mypdf_milestone0::signing::verify(&saved, None).unwrap();
        assert!(proof[0].crypto_valid && proof[0].digest_matches);
        assert_eq!(
            annotations::load(&lopdf::Document::load_mem(&saved).unwrap()).len(),
            1
        );
    }

    #[test]
    fn failed_restore_keeps_the_copy_visible_and_retryable() {
        let mut bytes = vec![];
        lopdf::Document::new().save_to(&mut bytes).unwrap();
        let (_root, mut app, ctx, tx, entry) = restore_fixture(&bytes);
        app.recovery.entries.clear();
        tx.send(Event::Restored(
            entry.clone(),
            Err("Simulated read failure".into()),
        ))
        .unwrap();
        app.recovery_tick(&ctx);
        assert!(app.recovery.open && !app.recovery.busy);
        assert_eq!(app.recovery.entries.len(), 1);
        assert!(entry.file.exists());
        // A PDF can pass storage checks but fail asynchronous renderer loading.
        tx.send(Event::Restored(
            entry.clone(),
            Ok(RestoredPdf::prepare(bytes).unwrap()),
        ))
        .unwrap();
        app.recovery_tick(&ctx);
        assert_eq!(app.recovery.entries.len(), 1);
        let deadline = Instant::now() + Duration::from_secs(15);
        while app.tabs[0].error.is_none() {
            app.events(&ctx);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(app.recovery.open && !app.recovery.busy);
        assert_eq!(app.recovery.entries.len(), 1);
        assert!(app.recovery.pending.is_empty() && app.recovery.origins.is_empty());
        assert!(
            app.recovery
                .error
                .as_ref()
                .unwrap()
                .contains("you can retry")
        );
        assert!(entry.file.exists());
    }

    #[test]
    fn autosave_tracks_revision_and_does_not_mark_document_saved() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            app.events(&ctx);
            app.recovery_tick(&ctx);
            if app.tabs[0].session.is_some() {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        let id = app.tabs[0].id;
        app.recovery.tracking.get_mut(&id).unwrap().changed =
            Instant::now() - Duration::from_secs(6);
        loop {
            app.recovery_tick(&ctx);
            if app.recovery.tracking[&id].written == Some(0) {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(app.tabs[0].session.as_ref().unwrap().dirty());
        assert_eq!(app.recovery.label(id, 0), "Recovery copy saved");
        assert_eq!(app.recovery.label(id, 1), "Unsaved changes");
        app.tabs[0].session.as_mut().unwrap().mark_saved(0);
        app.recovery_tick(&ctx);
        assert!(!app.recovery.tracking.contains_key(&id));
    }
}
