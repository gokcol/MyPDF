//! View-dependent guidance and bounded document captions.
use super::*;
use editing::{Mode, Tool};

#[derive(PartialEq)]
pub(super) struct StatusContext {
    tab: Option<u64>,
    mode: Mode,
    tool: Tool,
    redaction: bool,
    image: bool,
    area: bool,
    add: bool,
    home: bool,
    ready: bool,
}

pub(super) fn tab_caption(name: &str) -> String {
    let chars: Vec<_> = name.chars().collect();
    if chars.len() <= 26 {
        return name.into();
    }
    // Retain the suffix that often distinguishes similarly named documents.
    chars[..16]
        .iter()
        .chain(std::iter::once(&'…'))
        .chain(chars[chars.len() - 9..].iter())
        .collect()
}

impl Viewer {
    pub(super) fn retry_document(&mut self, ctx: &egui::Context) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        if tab.session.is_some() {
            // Restart rendering from the current source, never reopen the old
            // disk file over unsaved edits or remove its recovery checkpoint.
            tab.bytes = None;
            tab.refresh_document(ctx);
        } else {
            let wake = ctx.clone();
            tab.worker = Worker::open(tab.path.clone(), move || wake.request_repaint());
            tab.error = None;
            tab.index_done = false;
        }
    }
    pub(super) fn active_document_ready(&self) -> bool {
        self.tabs
            .get(self.active)
            .is_some_and(|t| t.error.is_none() && !t.pages.is_empty())
    }
    pub(super) fn sync_status_context(&mut self) {
        let next = StatusContext {
            tab: self.tabs.get(self.active).map(|t| t.id),
            mode: self.editor.mode,
            tool: self.editor.tool,
            redaction: self.redaction.active,
            image: self.content.image_mode,
            area: self.content.area_mode,
            add: self.content.add_target.is_some(),
            home: self.home.visible,
            ready: self.active_document_ready(),
        };
        if self.status_context.as_ref().is_some_and(|old| *old != next)
            && !self.saving
            && !self.content.busy
            && !self.organizer.busy
            && self.status == self.status_seen
            && self.status.as_ref().is_some_and(|(_, error)| !error)
        {
            self.status = None;
        }
        self.status_context = Some(next);
        self.status_seen = self.status.clone();
    }
    pub(super) fn tool_guidance(&self) -> &'static str {
        if self.home.visible {
            return "Open a PDF or start with a blank page";
        }
        if !self.active_document_ready() {
            return match self.tabs.get(self.active) {
                Some(tab) if tab.locked.is_some() => "Enter the document password to continue",
                Some(tab) if tab.error.is_some() => "Open another file, retry, or close this tab",
                Some(_) => "Opening PDF…",
                None => "Open a PDF or start with a blank page",
            };
        }
        match self.editor.mode {
            Mode::Read => "Select text to copy or annotate · ↑ / ↓ navigate pages",
            Mode::Edit if self.redaction.active => {
                "Mark areas to redact · Nothing is removed until you apply"
            }
            Mode::Edit if self.content.add_target.is_some() => {
                "Click a page and type · ⌘Return finishes · Esc cancels"
            }
            Mode::Edit if self.content.image_mode => {
                "Select an image to edit, or capture artwork with Copy region"
            }
            Mode::Edit if self.content.area_mode => {
                "Drag around text to edit several lines together"
            }
            Mode::Edit => "Click text to edit it · Add text starts a new block",
            Mode::Annotate | Mode::Draw => match self.editor.tool {
                Tool::Select | Tool::Lasso => {
                    "Click an object to select · Click outside to deselect · Arrow keys nudge"
                }
                Tool::Polyline | Tool::Polygon => {
                    "Click to add corners · Double-click or Return finishes · Esc cancels"
                }
                Tool::Asset => "Click or drag to place artwork · Select it to move or resize",
                Tool::Highlight | Tool::Underline | Tool::Strikeout => {
                    "Drag across text to mark it"
                }
                Tool::Note => "Click a page to add a note",
                Tool::Text => "Click or drag to add a text box",
                _ => "Drag on the page to draw · Esc cancels",
            },
            Mode::Forms => "Fill a field or choose Design to add and arrange fields",
            Mode::Sign => "Choose a signature or a digital certificate",
            Mode::Pages => "Select pages to reorder, rotate, extract or delete",
            Mode::Convert => "Choose an output format and save a new file",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captions_preserve_unicode_and_distinguishing_suffixes() {
        assert_eq!(tab_caption("Report.pdf"), "Report.pdf");
        let caption = tab_caption("İstanbul değerlendirme sözleşmesi-final.pdf");
        assert_eq!(caption.chars().count(), 26);
        assert!(caption.ends_with("final.pdf"));
    }
    #[test]
    fn guidance_follows_tool_and_preserves_errors() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx), vec![]);
        app.sync_status_context();
        app.status = Some(("Old redaction instruction".into(), false));
        app.sync_status_context();
        app.editor.mode = Mode::Edit;
        app.sync_status_context();
        assert!(app.status.is_none());
        app.status = Some(("Save failed".into(), true));
        app.editor.mode = Mode::Read;
        app.sync_status_context();
        assert_eq!(app.status.as_ref().unwrap().0, "Save failed");
        assert!(!app.active_document_ready());
        app.status = Some(("Saved signed copy".into(), false));
        app.editor.mode = Mode::Sign;
        app.sync_status_context();
        assert_eq!(app.status.as_ref().unwrap().0, "Saved signed copy");
    }
    #[test]
    fn tab_chooser_and_open_remain_visible_with_long_names_at_narrow_width() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        for n in 0..8 {
            app.open_memory(
                PathBuf::from(format!(
                    "Very long document name with a distinguishing ending-{n}.pdf"
                )),
                mypdf_milestone0::convert::blank(500., 700.).unwrap(),
                &ctx,
            );
        }
        for width in [720., 1100., 720.] {
            for _ in 0..5 {
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(width, 600.))),
                        ..Default::default()
                    },
                    |ctx| app.toolbar(ctx),
                );
                let nodes = output.platform_output.accesskit_update.unwrap().nodes;
                for label in ["All open tabs", "Open PDF"] {
                    let node = nodes
                        .iter()
                        .find(|(_, n)| {
                            n.label() == Some(label) && n.bounds().is_some_and(|r| r.y0 < 55.)
                        })
                        .expect("fixed tab-strip action");
                    let r = node.1.bounds().unwrap();
                    assert!(
                        r.x0 >= 0. && r.x1 <= width as f64,
                        "{label} exceeds width {width}: {r:?}"
                    );
                }
            }
        }
        assert_eq!(app.tab_revealed, Some(app.tabs[7].id));
    }
    #[test]
    fn retry_restarts_rendering_without_discarding_unsaved_document() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while app.tabs[0].session.is_none() {
            assert!(std::time::Instant::now() < deadline);
            app.events(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let id = app.tabs[0].id;
        let session = app.tabs[0].session.as_ref().unwrap();
        let revision = session.revision();
        let source = session.source.clone();
        app.tabs[0].error = Some("Simulated renderer failure".into());
        app.retry_document(&ctx);
        let tab = &app.tabs[0];
        assert_eq!(tab.id, id);
        assert!(tab.error.is_none());
        let session = tab.session.as_ref().unwrap();
        assert!(session.dirty());
        assert_eq!(session.revision(), revision);
        assert!(Arc::ptr_eq(&session.source, &source));
    }
}
