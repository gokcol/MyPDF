//! Save adopts the destination and tracks the version written to disk.
use super::*;

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub(super) enum Intent {
    #[default]
    Automatic,
    As,
}
impl Intent {
    pub(super) fn destination(self, tab: &Tab) -> Option<PathBuf> {
        (self == Self::Automatic && tab.has_destination).then(|| tab.path.clone())
    }
}
pub(super) struct Completed {
    pub path: PathBuf,
    pub id: u64,
    pub revision: u64,
    pub fingerprint: mypdf_milestone0::file_save::Fingerprint,
}
impl Viewer {
    pub(super) fn finish_save(&mut self, saved: Completed) {
        let path = saved.path.canonicalize().unwrap_or(saved.path);
        self.home.remember(&path);
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == saved.id) {
            tab.path = path;
            tab.has_destination = true;
            tab.disk_fingerprint = Some(saved.fingerprint);
            self.workspace.invalidate();
            tab.unsaved_on_load = false;
            if let Some(session) = &mut tab.session {
                // A newer edit made while saving remains dirty.
                session.mark_saved(saved.revision);
            }
            if tab.session.as_ref().is_none_or(|s| !s.dirty()) {
                self.recovery.clear(saved.id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document() -> (tempfile::TempDir, Viewer, egui::Context) {
        let root = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.open_memory(
            PathBuf::from("Untitled.pdf"),
            mypdf_milestone0::convert::blank(500., 700.).unwrap(),
            &ctx,
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while app.tabs[0].session.is_none() && std::time::Instant::now() < deadline {
            app.events(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(app.tabs[0].session.is_some(), "new document loaded");
        (root, app, ctx)
    }
    #[test]
    fn first_save_adopts_identity_and_save_as_changes_it() {
        let (root, mut app, ctx) = document();
        let id = app.tabs[0].id;
        let revision = app.tabs[0].session.as_ref().unwrap().revision();
        let destination = root.path().join("Named.pdf");
        std::fs::write(&destination, app.tabs[0].bytes.as_ref().unwrap()).unwrap();
        assert_eq!(Intent::Automatic.destination(&app.tabs[0]), None);
        app.finish_save(Completed {
            path: destination.clone(),
            id,
            revision,
            fingerprint: [0; 32],
        });
        assert_eq!(app.tabs[0].name(), "Named.pdf");
        assert!(!app.tabs[0].unsaved_on_load);
        assert!(!app.tabs[0].session.as_ref().unwrap().dirty());
        assert_eq!(
            Intent::Automatic.destination(&app.tabs[0]),
            Some(destination.canonicalize().unwrap())
        );
        assert_eq!(Intent::As.destination(&app.tabs[0]), None);
        let copy = root.path().join("Copy.pdf");
        std::fs::write(&copy, app.tabs[0].bytes.as_ref().unwrap()).unwrap();
        app.finish_save(Completed {
            path: copy.clone(),
            id,
            revision,
            fingerprint: [1; 32],
        });
        assert_eq!(app.tabs[0].name(), "Copy.pdf");
        app.remember_workspace(&ctx);
        let saved: serde_json::Value = serde_json::from_slice(
            &std::fs::read(app.workspace.root.join("workspace.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            saved["tabs"][0]["path"],
            copy.canonicalize().unwrap().to_string_lossy().as_ref()
        );
    }
    #[test]
    fn completing_save_does_not_clear_newer_changes_or_switch_tabs() {
        let (root, mut app, ctx) = document();
        let id = app.tabs[0].id;
        let revision = app.tabs[0].session.as_ref().unwrap().revision();
        let mut marks = app.tabs[0].session.as_ref().unwrap().annotations.clone();
        marks.push(mypdf_milestone0::annotations::Annotation::new(
            0,
            [10., 10., 80., 80.],
            mypdf_milestone0::annotations::Kind::Rectangle,
            Default::default(),
        ));
        app.tabs[0].session.as_mut().unwrap().commit(marks).unwrap();
        let destination = root.path().join("Earlier revision.pdf");
        std::fs::write(&destination, app.tabs[0].bytes.as_ref().unwrap()).unwrap();
        app.new_blank(&ctx);
        let active_id = app.tabs[app.active].id;
        app.finish_save(Completed {
            path: destination.clone(),
            id,
            revision,
            fingerprint: [0; 32],
        });
        assert!(app.tabs[0].session.as_ref().unwrap().dirty());
        assert_eq!(app.tabs[0].name(), "Earlier revision.pdf");
        assert_eq!(app.tabs[app.active].id, active_id);
    }
    #[test]
    fn busy_save_request_cannot_replace_deferred_save_as_intent() {
        let (_, mut app, ctx) = document();
        app.save_intent = Intent::As;
        app.content.after_text = Some(content::AfterText::Save(app.tabs[0].id, Intent::As));
        app.content.busy = true;
        app.action("save", &ctx);
        assert_eq!(app.save_intent, Intent::As);
        assert!(matches!(
            app.content.after_text,
            Some(content::AfterText::Save(_, Intent::As))
        ));
    }
    fn wait_save(app: &mut Viewer, ctx: &egui::Context) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while app.saving && std::time::Instant::now() < deadline {
            app.events(ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!app.saving);
    }
    #[test]
    fn save_command_updates_existing_file_repeatedly_without_a_dialog_and_detects_conflicts() {
        let (root, mut app, ctx) = document();
        let id = app.tabs[0].id;
        let path = root.path().join("Daily.pdf");
        let source = app.tabs[0].bytes.clone().unwrap();
        let fingerprint = mypdf_milestone0::file_save::write(&path, &source, None).unwrap();
        let revision = app.tabs[0].session.as_ref().unwrap().revision();
        app.finish_save(Completed {
            path: path.clone(),
            id,
            revision,
            fingerprint,
        });
        for count in 1..=2 {
            let session = app.tabs[0].session.as_mut().unwrap();
            let mut marks = session.annotations.clone();
            marks.push(mypdf_milestone0::annotations::Annotation::new(
                0,
                [10., count as f32 * 40., 80., count as f32 * 40. + 25.],
                mypdf_milestone0::annotations::Kind::Rectangle,
                Default::default(),
            ));
            session.commit(marks).unwrap();
            if count == 1 {
                app.action("save", &ctx);
            } else {
                let _ = ctx.run(
                    egui::RawInput {
                        events: vec![egui::Event::Key {
                            key: egui::Key::S,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::COMMAND,
                        }],
                        ..Default::default()
                    },
                    |ctx| app.shortcuts(ctx),
                );
            }
            wait_save(&mut app, &ctx);
            assert_eq!(
                app.status.as_ref().map(|s| s.1),
                Some(false),
                "{:?}",
                app.status
            );
            assert_eq!(app.tabs[0].name(), "Daily.pdf");
            assert!(!app.tabs[0].session.as_ref().unwrap().dirty());
            let bytes = std::fs::read(&path).unwrap();
            assert_eq!(
                mypdf_milestone0::annotations::load(&lopdf::Document::load_mem(&bytes).unwrap())
                    .len(),
                count
            );
        }
        std::fs::write(&path, b"another application's edits").unwrap();
        app.tabs[0].session.as_mut().unwrap().mark_unsaved();
        app.action("save", &ctx);
        wait_save(&mut app, &ctx);
        assert!(app.status.as_ref().unwrap().0.contains("changed outside"));
        assert!(app.tabs[0].session.as_ref().unwrap().dirty());
        assert_eq!(std::fs::read(path).unwrap(), b"another application's edits");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }
    #[tokio::test]
    async fn save_signed_edits_preserves_signature_and_protected_saves_keep_password() {
        use mypdf_milestone0::{identity, security, signing};
        let (root, mut app, ctx) = document();
        let original = app.tabs[0].session.as_ref().unwrap().source.clone();
        let created = identity::create("Save test", "test-only-pass", true).unwrap();
        let signer = identity::FileSigner::load(&created.pkcs12, "test-only-pass", None).unwrap();
        let signed = signing::sign(&original, &signer, "SaveTest", false)
            .await
            .unwrap();
        let path = root.path().join("Signed.pdf");
        std::fs::write(&path, &signed).unwrap();
        let id = app.tabs[0].id;
        let revision = app.tabs[0].session.as_ref().unwrap().revision();
        app.finish_save(Completed {
            path: path.clone(),
            id,
            revision,
            fingerprint: mypdf_milestone0::file_save::fingerprint(&signed),
        });
        mypdf_milestone0::signature_edit::unlock(&signed).unwrap();
        let marks =
            mypdf_milestone0::annotations::load(&lopdf::Document::load_mem(&signed).unwrap());
        app.tabs[0].session = Some(mypdf_milestone0::annotations::Session::new(
            signed.clone().into(),
            marks,
            true,
        ));
        app.tabs[0].signed_original = Some(signed.clone().into());
        app.tabs[0].session.as_mut().unwrap().mark_unsaved();
        app.action("save", &ctx);
        wait_save(&mut app, &ctx);
        assert!(!app.status.as_ref().unwrap().1, "{:?}", app.status);
        let saved = std::fs::read(&path).unwrap();
        assert!(saved.starts_with(&signed));
        let report = signing::verify(&saved, None).unwrap();
        assert!(report[0].crypto_valid && report[0].digest_matches);
        // The decrypted editing source must never replace the protected file.
        let encrypted = security::protect(&original, "test-only-pass").unwrap();
        std::fs::write(&path, &encrypted).unwrap();
        app.tabs[0].disk_fingerprint = Some(mypdf_milestone0::file_save::fingerprint(&encrypted));
        app.tabs[0].signed_original = None;
        app.tabs[0].session = Some(mypdf_milestone0::annotations::Session::new(
            original.clone(),
            vec![],
            true,
        ));
        app.tabs[0].password = Some(zeroize::Zeroizing::new("test-only-pass".into()));
        app.action("save", &ctx);
        wait_save(&mut app, &ctx);
        assert!(!app.status.as_ref().unwrap().1, "{:?}", app.status);
        let saved = std::fs::read(path).unwrap();
        assert!(security::unlock(&saved, "wrong").is_err());
        assert!(security::unlock(&saved, "test-only-pass").is_ok());
    }
}
