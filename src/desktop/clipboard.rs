//! Copy, cut and paste on the page: marks between documents, and images or
//! text from other apps through the system clipboard.
use super::*;
use mypdf_milestone0::annotations::{Annotation, Kind};

/// What the system clipboard holds.
pub(super) enum Clip {
    Text(String),
    /// PNG bytes and pixel size.
    Image(Vec<u8>, [u32; 2]),
    Empty,
}

#[cfg(not(test))]
pub(super) fn read() -> Clip {
    let Ok(mut clipboard) = arboard::Clipboard::new() else {
        return Clip::Empty;
    };
    if let Ok(image) = clipboard.get_image()
        && let Some(png) = png(image.width as u32, image.height as u32, &image.bytes)
    {
        return Clip::Image(png, [image.width as u32, image.height as u32]);
    }
    match clipboard.get_text() {
        Ok(text) if !text.trim().is_empty() => Clip::Text(text),
        _ => Clip::Empty,
    }
}
#[cfg(test)]
thread_local! {
    static TEST_CLIP: std::cell::RefCell<Option<Clip>> = const { std::cell::RefCell::new(None) };
}
/// Tests never touch the real clipboard.
#[cfg(test)]
pub(super) fn read() -> Clip {
    TEST_CLIP
        .with(|c| c.borrow_mut().take())
        .unwrap_or(Clip::Empty)
}
#[cfg(test)]
pub(super) fn set_for_test(clip: Clip) {
    TEST_CLIP.with(|c| *c.borrow_mut() = Some(clip));
}

pub(super) fn png(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return None;
    }
    let image = image::RgbaImage::from_raw(width, height, rgba.to_vec())?;
    let mut out = vec![];
    image
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .ok()?;
    Some(out)
}

/// Marks copied from a page, with the text that marks them on the system clipboard.
pub(super) struct MarksClip {
    marks: Vec<Annotation>,
    marker: String,
}

/// A box of `size` points centered on the page, shrunk to fit `room` of it.
fn centered(page: PageSize, size: [f32; 2], room: f32) -> [f32; 4] {
    let fit = (page.width * room / size[0])
        .min(page.height * room / size[1])
        .min(1.);
    let (w, h) = (size[0] * fit, size[1] * fit);
    let (x, y) = ((page.width - w) / 2., (page.height - h) / 2.);
    [x, y, x + w, y + h]
}

impl Viewer {
    /// Copy and paste act on marks in Annotate and Draw when no text field has focus.
    pub(super) fn marks_keys(&self, ctx: &egui::Context) -> bool {
        !self.home.visible
            && matches!(
                self.editor.mode,
                editing::Mode::Annotate | editing::Mode::Draw
            )
            && !ctx.wants_keyboard_input()
            && !self.editor.text_editing
    }
    /// Paste places clipboard content in Read, Annotate, Draw and Edit.
    pub(super) fn paste_keys(&self, ctx: &egui::Context) -> bool {
        !self.home.visible
            && matches!(
                self.editor.mode,
                editing::Mode::Read
                    | editing::Mode::Annotate
                    | editing::Mode::Draw
                    | editing::Mode::Edit
            )
            && !ctx.wants_keyboard_input()
            && !self.editor.text_editing
            && self.content.pending_text().is_none()
            && self
                .tabs
                .get(self.active)
                .is_some_and(|t| t.session.as_ref().is_some_and(|s| s.editable))
    }

    pub(super) fn copy_marks(&mut self, ctx: &egui::Context, cut: bool) -> bool {
        let Some(tab) = self.tabs.get(self.active) else {
            return false;
        };
        let ids = selection::selected_ids(tab);
        let Some(session) = &tab.session else {
            return false;
        };
        let marks: Vec<Annotation> = session
            .annotations
            .iter()
            .filter(|a| ids.contains(&a.id))
            .cloned()
            .collect();
        if cut && marks.iter().any(|a| a.object.locked) {
            self.status = Some(("Unlock the object before cutting it".into(), false));
            return false;
        }
        let Some(n) = self.copy_artwork(ctx, marks) else {
            return false;
        };
        if cut {
            self.editor_delete();
        }
        self.status = Some((
            format!(
                "{} {n} object{} • {} pastes on any page of any open PDF",
                if cut { "Cut" } else { "Copied" },
                if n == 1 { "" } else { "s" },
                if cfg!(target_os = "macos") {
                    "⌘V"
                } else {
                    "Ctrl+V"
                },
            ),
            false,
        ));
        true
    }

    /// Store self-contained artwork independently of its source document.
    /// Region previews can copy directly without adding anything to that PDF.
    pub(super) fn copy_artwork(
        &mut self,
        ctx: &egui::Context,
        marks: Vec<Annotation>,
    ) -> Option<usize> {
        let tab = self.tabs.get(self.active)?;
        marks.first()?;
        let n = marks.len();
        let marker = format!(
            "MyPDF: {n} mark{} from {}",
            if n == 1 { "" } else { "s" },
            tab.name()
        );
        self.editor.marks_clip = Some(MarksClip {
            marks,
            marker: marker.clone(),
        });
        ctx.copy_text(marker);
        Some(n)
    }

    /// Paste marks, or an image or text from another app, on the current page.
    pub(super) fn paste(&mut self, ctx: &egui::Context) {
        let clip = read();
        let ours = match &clip {
            Clip::Text(text) => self
                .editor
                .marks_clip
                .as_ref()
                .is_some_and(|m| m.marker == *text),
            Clip::Empty => self.editor.marks_clip.is_some(),
            Clip::Image(..) => false,
        };
        if ours {
            self.paste_marks();
            return;
        }
        match clip {
            Clip::Image(png, pixels) => self.paste_image(ctx, png, pixels),
            Clip::Text(text) => self.paste_text(text),
            Clip::Empty => {
                self.status = Some(("The clipboard has no text or image to paste.".into(), false))
            }
        }
    }

    fn paste_marks(&mut self) {
        let Some(clip) = &self.editor.marks_clip else {
            return;
        };
        self.workbench.paste = Some(
            clip.marks
                .iter()
                .zip(mypdf_milestone0::object_tools::duplicate(&clip.marks))
                .map(|(a, mut c)| {
                    c.rect = a.rect;
                    c.object.hidden = false;
                    c
                })
                .collect(),
        );
        self.status = Some((
            "Click a page to place the artwork · Escape cancels · ⇧⌘V keeps its original position"
                .into(),
            false,
        ));
    }
    pub(super) fn paste_original(&mut self) {
        if self.workbench.paste.is_none() {
            let clip = read();
            let ours = self
                .editor
                .marks_clip
                .as_ref()
                .is_some_and(|m| match &clip {
                    Clip::Text(t) => t == &m.marker,
                    Clip::Empty => true,
                    _ => false,
                });
            if !ours {
                self.status = Some(("Copy artwork in MyPDF first".into(), false));
                return;
            }
            self.paste_marks();
        }
        self.place_pending_original();
    }

    fn paste_image(&mut self, ctx: &egui::Context, png: Vec<u8>, pixels: [u32; 2]) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let page = tab.page.min(tab.pages.len().saturating_sub(1));
        let Some(size) = tab.pages.get(page).copied() else {
            return;
        };
        // Screenshots are usually 2× pixels; show them at their point size.
        let rect = centered(size, [pixels[0] as f32 / 2., pixels[1] as f32 / 2.], 0.6);
        let _ = ctx;
        self.workbench.paste = Some(vec![Annotation::new(
            page,
            rect,
            Kind::Image { png: Arc::new(png) },
            Default::default(),
        )]);
        self.status = Some((
            "Click a page to place the image · Escape cancels".into(),
            false,
        ));
    }
    fn paste_text(&mut self, text: String) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let page = tab.page.min(tab.pages.len().saturating_sub(1));
        let Some(size) = tab.pages.get(page).copied() else {
            return;
        };
        let text = text.replace("\r\n", "\n");
        if self.editor.mode == editing::Mode::Edit {
            // Real page text: open Add text with the pasted words, ready to place.
            self.content.paste_new_text(tab.id, page, size, text);
            self.inspector = true;
            return;
        }
        let mut style = self.editor.style.clone();
        style.font_size = style.font_size.clamp(8., 36.);
        let longest = text.lines().map(|l| l.chars().count()).max().unwrap_or(1) as f32;
        let lines = text.lines().count().max(1) as f32;
        let wanted = [
            (longest * style.font_size * 0.55 + 16.).clamp(60., size.width * 0.8),
            lines * style.font_size * 1.35 + 16.,
        ];
        let rect = centered(size, wanted, 0.9);
        self.add_mark(
            Annotation::new(page, rect, Kind::Text { text }, style),
            "Pasted text box • double-click to edit",
        );
    }

    fn add_mark(&mut self, mark: Annotation, done: &str) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let Some(session) = tab.session.as_mut() else {
            return;
        };
        let mut entries = session.annotations.clone();
        entries.push(mark.clone());
        match session.commit(entries) {
            Ok(()) => {
                if !matches!(
                    self.editor.mode,
                    editing::Mode::Annotate | editing::Mode::Draw
                ) {
                    self.editor.mode = editing::Mode::Annotate;
                }
                selection::set(tab, &mut self.editor, vec![mark.id]);
                self.status = Some((done.into(), false));
            }
            Err(e) => self.status = Some((format!("{e:#}"), true)),
        }
    }

    /// Edit → Add image…: a PNG or JPEG placed as page content on the current page.
    pub(super) fn add_image_file(&mut self, ctx: &egui::Context) {
        let Some(path) = self.native_pick(
            rfd::FileDialog::new()
                .set_title("Add image")
                .add_filter("Images", &["png", "jpg", "jpeg"]),
        ) else {
            return;
        };
        let read = std::fs::metadata(&path)
            .map_err(anyhow::Error::from)
            .and_then(|m| {
                anyhow::ensure!(m.len() <= 32 * 1024 * 1024, "Image exceeds 32 MB");
                Ok(std::fs::read(&path)?)
            })
            .and_then(|bytes| {
                let image = mypdf_milestone0::content::decode_image(&bytes)?;
                Ok((bytes, [image.width(), image.height()]))
            });
        match read {
            Ok((bytes, pixels)) => {
                let Some(tab) = self.tabs.get(self.active) else {
                    return;
                };
                let page = tab.page.min(tab.pages.len().saturating_sub(1));
                let Some(size) = tab.pages.get(page).copied() else {
                    return;
                };
                // Images at 144 dpi, at most 60% of the page.
                let rect = centered(size, [pixels[0] as f32 / 2., pixels[1] as f32 / 2.], 0.6);
                self.content_change(
                    ctx,
                    content::Edit::AddImage {
                        page: page as u32 + 1,
                        rect,
                        image: bytes,
                    },
                );
            }
            Err(e) => self.status = Some((format!("{e:#}"), true)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;

    #[test]
    fn marks_images_and_text_paste_onto_pages_of_any_open_pdf() {
        let temp = tempfile::tempdir().unwrap();
        let blank = mypdf_milestone0::convert::blank(595., 842.).unwrap();
        let (a, b) = (temp.path().join("A.pdf"), temp.path().join("B.pdf"));
        std::fs::write(&a, &blank).unwrap();
        std::fs::write(&b, &blank).unwrap();
        let ctx = egui::Context::default();
        let mut app = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            vec![a, b],
        );
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
        let editable = |t: &Tab| t.session.as_ref().is_some_and(|s| s.editable);
        until(&mut app, &|a| a.tabs.iter().all(editable));
        let marks =
            |app: &Viewer, tab: usize| app.tabs[tab].session.as_ref().unwrap().annotations.clone();

        // Paste previews do not mutate until placed; original-position keeps geometry.
        app.active = 0;
        app.editor.mode = editing::Mode::Annotate;
        let rect = Annotation::new(
            0,
            [100., 100., 200., 150.],
            Kind::Rectangle,
            Default::default(),
        );
        let session = app.tabs[0].session.as_mut().unwrap();
        session.commit(vec![rect.clone()]).unwrap();
        selection::set(&mut app.tabs[0], &mut app.editor, vec![rect.id.clone()]);
        assert!(app.copy_marks(&ctx, false));
        let marker = app.editor.marks_clip.as_ref().unwrap().marker.clone();
        for step in 1..=2 {
            set_for_test(Clip::Text(marker.clone()));
            app.paste(&ctx);
            assert!(app.workbench.paste.is_some());
            app.place_pending_original();
            let all = marks(&app, 0);
            assert_eq!(all.len(), step + 1);
            let pasted = all.last().unwrap();
            assert_ne!(pasted.id, rect.id);
            assert_eq!(pasted.rect[0], 100.);
            assert_eq!(app.tabs[0].selected_annotation.as_ref(), Some(&pasted.id));
        }

        // Into another PDF, at the same place; cutting removes the original.
        app.active = 1;
        set_for_test(Clip::Text(marker.clone()));
        app.paste(&ctx);
        app.place_pending_original();
        assert_eq!(marks(&app, 1).len(), 1);
        assert_eq!(marks(&app, 1)[0].rect, rect.rect);
        assert!(app.copy_marks(&ctx, true));
        assert!(marks(&app, 1).is_empty());

        // Images and text from other apps become marks in Annotate.
        let png = png(200, 100, &[200; 200 * 100 * 4]).unwrap();
        set_for_test(Clip::Image(png.clone(), [200, 100]));
        app.paste(&ctx);
        app.place_pending_original();
        let image = marks(&app, 1).pop().unwrap();
        assert!(matches!(image.kind, Kind::Image { .. }));
        assert_eq!(
            [image.rect[2] - image.rect[0], image.rect[3] - image.rect[1]],
            [100., 50.]
        );
        set_for_test(Clip::Text("Hello\nWorld".into()));
        app.paste(&ctx);
        app.place_pending_original();
        assert!(
            matches!(&marks(&app, 1).pop().unwrap().kind, Kind::Text { text } if text == "Hello\nWorld")
        );

        // Images remain reversible layers in Edit; text still opens Add text.
        app.editor.mode = editing::Mode::Edit;
        until(&mut app, &|a| editable(&a.tabs[1]));
        set_for_test(Clip::Image(png, [200, 100]));
        app.paste(&ctx);
        app.place_pending_original();
        until(&mut app, &|a| !a.content.busy && editable(&a.tabs[1]));
        let source = app.tabs[1].session.as_ref().unwrap().source.clone();
        assert_eq!(
            mypdf_milestone0::content::images(&source, 1).unwrap().len(),
            0,
            "{:?}",
            app.status
        );
        app.editor.mode = editing::Mode::Edit;
        set_for_test(Clip::Text("Typed elsewhere".into()));
        app.paste(&ctx);
        app.place_pending_original();
        assert_eq!(app.content.pending_text(), Some(app.tabs[1].id));
    }
}
