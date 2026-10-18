//! First Save adopts a destination; explicit copies keep the current document identity.
use super::*;

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub(super) enum Intent {
    #[default]
    Automatic,
    As,
    Copy,
}
impl Intent {
    pub fn adopts(self) -> bool {
        self != Self::Copy
    }
}
impl Viewer {
    pub(super) fn finish_save(
        &mut self,
        path: &std::path::Path,
        id: u64,
        revision: u64,
        adopt: bool,
    ) {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.home.remember(&path);
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
            if adopt {
                tab.path = path;
                tab.has_destination = true;
                // Force workspace persistence even if the view did not change.
                self.workspace.invalidate();
            }
            if adopt {
                tab.unsaved_on_load = false;
            }
            if let Some(session) = &mut tab.session
                && adopt
            {
                // A newer edit made while saving remains dirty.
                session.mark_saved(revision);
            }
            if tab.session.as_ref().is_none_or(|s| !s.dirty()) {
                self.recovery.clear(id);
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
    fn first_save_adopts_identity_and_save_copy_keeps_it() {
        let (root, mut app, ctx) = document();
        let id = app.tabs[0].id;
        let revision = app.tabs[0].session.as_ref().unwrap().revision();
        let destination = root.path().join("Named.pdf");
        std::fs::write(&destination, app.tabs[0].bytes.as_ref().unwrap()).unwrap();
        assert!(Intent::Automatic.adopts());
        app.finish_save(&destination, id, revision, true);
        assert_eq!(app.tabs[0].name(), "Named.pdf");
        assert!(!app.tabs[0].unsaved_on_load);
        assert!(!app.tabs[0].session.as_ref().unwrap().dirty());
        assert!(Intent::Automatic.adopts());
        let copy = root.path().join("Copy.pdf");
        std::fs::write(&copy, app.tabs[0].bytes.as_ref().unwrap()).unwrap();
        app.finish_save(&copy, id, revision, Intent::Copy.adopts());
        assert_eq!(app.tabs[0].name(), "Named.pdf");
        app.finish_save(&copy, id, revision, Intent::As.adopts());
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
        app.finish_save(&destination, id, revision, true);
        assert!(app.tabs[0].session.as_ref().unwrap().dirty());
        assert_eq!(app.tabs[0].name(), "Earlier revision.pdf");
        assert_eq!(app.tabs[app.active].id, active_id);
    }
    #[test]
    fn saving_an_explicit_copy_leaves_original_edits_and_identity_pending() {
        let (root, mut app, _) = document();
        let id = app.tabs[0].id;
        let revision = app.tabs[0].session.as_ref().unwrap().revision();
        let path = root.path().join("Export.pdf");
        app.finish_save(&path, id, revision, false);
        assert_eq!(app.tabs[0].name(), "Untitled.pdf");
        assert!(app.tabs[0].session.as_ref().unwrap().dirty());
        assert!(!app.tabs[0].has_destination);
    }
    #[test]
    fn busy_save_request_cannot_replace_deferred_save_as_intent() {
        let (_, mut app, ctx) = document();
        app.save_intent = Intent::As;
        app.content.after_text = Some(content::AfterText::Save(app.tabs[0].id, Intent::As));
        app.content.busy = true;
        app.action("save_copy", &ctx);
        assert_eq!(app.save_intent, Intent::As);
        assert!(matches!(
            app.content.after_text,
            Some(content::AfterText::Save(_, Intent::As))
        ));
    }
}
