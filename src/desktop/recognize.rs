//! Recognize Text: asking first, then reading scanned pages with progress in
//! the status bar.
use super::*;
use mypdf_milestone0::ocr;
use std::sync::Mutex;
use std::time::Instant;

pub(super) struct Progress {
    pub cancellation: ocr::Cancellation,
    state: Mutex<ocr::Progress>,
    started: Instant,
}
impl Default for Progress {
    fn default() -> Self {
        Self {
            cancellation: Default::default(),
            state: Default::default(),
            started: Instant::now(),
        }
    }
}
impl Progress {
    pub(super) fn update(&self, state: ocr::Progress) {
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = state;
    }
    fn message(&self) -> String {
        let state = *self.state.lock().unwrap_or_else(|e| e.into_inner());
        let elapsed = self.started.elapsed().as_secs();
        let page = if state.pages > 0 && state.phase != ocr::Phase::Applying {
            format!(" · page {} of {}", state.page, state.pages)
        } else {
            String::new()
        };
        format!(
            "{}{} · {}:{:02} elapsed",
            state.phase.label(),
            page,
            elapsed / 60,
            elapsed % 60
        )
    }
}

impl Viewer {
    pub(super) fn request_ocr(&mut self) {
        if self.content.busy || self.organizer.busy {
            self.status = Some((
                "Wait for the current operation to finish before recognizing text.".into(),
                false,
            ));
            return;
        }
        if !mypdf_milestone0::ocr::available() {
            self.status = Some(("Recognize Text needs macOS.".into(), true));
            return;
        }
        let editable = self
            .tabs
            .get(self.active)
            .and_then(|t| t.session.as_ref())
            .is_some_and(|s| s.editable);
        if editable {
            self.ocr_confirm = true;
        } else {
            self.status = Some((
                "This document cannot be changed here. Signed PDFs keep their content so their signatures stay valid.".into(),
                true,
            ));
        }
    }

    pub(super) fn ocr_dialogs(&mut self, ctx: &egui::Context) {
        if self.ocr_confirm {
            match ui::confirmation(
                ctx,
                "recognize_text",
                "Recognize text?",
                "MyPDF reads the words on scanned pages, so you can search, select and copy them. Pages that already have text stay as they are. It happens on this Mac; nothing is uploaded. Processing time depends on the document. You can cancel without changing the original.",
                &[
                    ("Recognize", ui::DialogTone::Primary),
                    ("Cancel", ui::DialogTone::Secondary),
                ],
                1,
            ) {
                Some(0) => {
                    self.ocr_confirm = false;
                    let progress = Arc::new(Progress::default());
                    self.content_change(ctx, content::Edit::RecognizeText(progress));
                }
                Some(_) => self.ocr_confirm = false,
                None => {}
            }
        }
        if let Some(progress) = &self.ocr_progress {
            if self.content.busy {
                let message = progress.message();
                let cancelled = progress.cancellation.is_cancelled();
                self.status = Some((
                    if cancelled {
                        "Cancelling text recognition…".into()
                    } else {
                        message.clone()
                    },
                    false,
                ));
                egui::Window::new("Recognizing text")
                    .id(egui::Id::new("ocr_progress"))
                    .anchor(egui::Align2::RIGHT_BOTTOM, [-20., -52.])
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(message);
                        });
                        if cancelled {
                            ui.label("Cancelling… finishing the current step.");
                            ui.small("The document will remain unchanged.");
                        } else if ui.button("Cancel recognition").clicked() {
                            progress.cancellation.cancel();
                        }
                    });
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            } else {
                self.ocr_progress = None;
            }
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use eframe::App;

    #[test]
    fn scanned_pages_are_read_with_progress_and_become_searchable() {
        let font = mypdf_milestone0::document::default_font().unwrap();
        let original = mypdf_milestone0::document::fixture(&font).unwrap();
        let (scan, _) = mypdf_milestone0::redact::flatten_document(&original, 200.).unwrap();
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.open_memory(PathBuf::from("Scan.pdf"), scan, &ctx);
        let frame = |app: &mut Viewer| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 900.))),
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
        };
        // Includes creating and rendering a two-page scan in a debug build.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(240);
        while app.tabs[0].session.as_ref().is_none_or(|s| !s.editable) {
            assert!(std::time::Instant::now() < deadline);
            frame(&mut app);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.request_ocr();
        assert!(app.ocr_confirm, "it asks first");
        app.ocr_confirm = false;
        let progress = Arc::new(Progress::default());
        app.ocr_progress = Some(progress.clone());
        app.content_change(&ctx, content::Edit::RecognizeText(progress));
        let mut saw_progress = false;
        while app.content.busy {
            assert!(std::time::Instant::now() < deadline, "{:?}", app.status);
            frame(&mut app);
            saw_progress |= app
                .status
                .as_ref()
                .is_some_and(|(m, _)| m.contains(" · page") && m.contains("elapsed"));
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        frame(&mut app);
        assert!(saw_progress, "pages are counted in the status bar");
        let (message, error) = app.status.clone().unwrap();
        assert!(
            !error && message.starts_with("Text recognized on 2 pages"),
            "{message}"
        );
        let source = app.tabs[0].session.as_ref().unwrap().source.clone();
        let pdf = hayro::hayro_syntax::Pdf::new(source.to_vec()).unwrap();
        let text = mypdf_milestone0::viewer::extract_page(
            &pdf.pages()[0],
            &hayro::hayro_interpret::InterpreterCache::new(),
        )
        .text;
        assert!(text.to_lowercase().contains("invoice"), "{text}");
        assert!(app.ocr_progress.is_none());
    }
}
