//! Copy, cut and paste on the page: marks between documents, and images or
//! text from other apps through the system clipboard.
use super::*;
use mypdf_milestone0::annotations::{Annotation, Kind};

/// What the system clipboard holds.
#[cfg_attr(test, derive(Clone))]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextCopy {
    Formatted,
    Plain,
}

#[cfg(not(test))]
fn write_formatted_text(
    text: &mypdf_milestone0::clipboard_text::ClipboardText,
) -> anyhow::Result<TextCopy> {
    let mut clipboard = arboard::Clipboard::new()?;
    match clipboard.set_html(text.html.as_str(), Some(text.text.as_str())) {
        Ok(()) => Ok(TextCopy::Formatted),
        Err(_) => {
            // Keep Copy useful in apps/platform sessions without a rich format.
            // Report plain text only after its write has actually succeeded.
            clipboard.set_text(text.text.as_str())?;
            Ok(TextCopy::Plain)
        }
    }
}

#[cfg(test)]
thread_local! {
    static TEST_RICH_TEXT: std::cell::RefCell<Option<(String, String)>> = const { std::cell::RefCell::new(None) };
    static FAIL_TEXT_WRITE: std::cell::Cell<(bool, bool)> = const { std::cell::Cell::new((false, false)) };
}

#[cfg(test)]
pub(super) fn rich_text_for_test() -> Option<(String, String)> {
    TEST_RICH_TEXT.with(|text| text.borrow_mut().take())
}

#[cfg(test)]
fn write_formatted_text(
    text: &mypdf_milestone0::clipboard_text::ClipboardText,
) -> anyhow::Result<TextCopy> {
    let (rich_fails, plain_fails) = FAIL_TEXT_WRITE.with(|fail| fail.replace((false, false)));
    TEST_RICH_TEXT.with(|captured| *captured.borrow_mut() = None);
    if rich_fails {
        anyhow::ensure!(!plain_fails, "Clipboard unavailable");
        set_for_test(Clip::Text(text.text.clone()));
        return Ok(TextCopy::Plain);
    }
    TEST_RICH_TEXT.with(|captured| {
        *captured.borrow_mut() = Some((text.text.clone(), text.html.clone()));
    });
    set_for_test(Clip::Text(text.text.clone()));
    Ok(TextCopy::Formatted)
}

impl Viewer {
    pub(super) fn copy_formatted_text(
        &mut self,
        ctx: &egui::Context,
        text: mypdf_milestone0::clipboard_text::ClipboardText,
    ) {
        let count = text.text.chars().count();
        let characters = if count == 1 {
            "character"
        } else {
            "characters"
        };
        match write_formatted_text(&text) {
            Ok(kind) => {
                // eframe applies queued CopyText commands after update(), which
                // would clear the HTML flavor we just wrote. This action owns
                // Copy; remove earlier text-copy output rather than queueing it.
                ctx.output_mut(|output| {
                    output
                        .commands
                        .retain(|command| !matches!(command, egui::OutputCommand::CopyText(_)));
                });
                self.status = Some((
                    match kind {
                        TextCopy::Formatted => {
                            format!("Copied {count} {characters} with formatting")
                        }
                        TextCopy::Plain => format!(
                            "Copied {count} {characters} as plain text; formatting was unavailable."
                        ),
                    },
                    false,
                ));
            }
            Err(error) => {
                self.status = Some((format!("Could not copy the selected text: {error}"), true))
            }
        }
    }
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
    image: Option<ImageClip>,
}

struct ImageClip {
    sequence: Option<u64>,
    fingerprint: String,
}

#[cfg(all(not(test), target_os = "macos"))]
fn sequence() -> Option<u64> {
    // NSPasteboard owns the general pasteboard; no retained pointer escapes.
    unsafe {
        let board: *mut objc2::runtime::AnyObject =
            objc2::msg_send![objc2::class!(NSPasteboard), generalPasteboard];
        if board.is_null() {
            return None;
        }
        let count: isize = objc2::msg_send![board, changeCount];
        Some(count as u64)
    }
}
#[cfg(all(not(test), target_os = "windows"))]
fn sequence() -> Option<u64> {
    let count = unsafe { windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber() };
    (count != 0).then_some(count as u64)
}
#[cfg(any(test, not(any(target_os = "macos", target_os = "windows"))))]
fn sequence() -> Option<u64> {
    None
}

fn fingerprint(image: &image::RgbaImage) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(image.width().to_le_bytes());
    hash.update(image.height().to_le_bytes());
    hash.update(image.as_raw());
    format!("{:x}", hash.finalize())
}

#[cfg(not(test))]
fn write_image(image: &image::RgbaImage) -> anyhow::Result<()> {
    let mut clipboard = arboard::Clipboard::new()?;
    clipboard.set_image(arboard::ImageData {
        width: image.width() as usize,
        height: image.height() as usize,
        bytes: std::borrow::Cow::Borrowed(image.as_raw()),
    })?;
    Ok(())
}
#[cfg(test)]
thread_local! {
    static FAIL_IMAGE_WRITE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
#[cfg(test)]
fn write_image(image: &image::RgbaImage) -> anyhow::Result<()> {
    anyhow::ensure!(
        !FAIL_IMAGE_WRITE.with(|f| f.replace(false)),
        "Clipboard unavailable"
    );
    set_for_test(Clip::Image(
        png(image.width(), image.height(), image.as_raw()).unwrap(),
        [image.width(), image.height()],
    ));
    Ok(())
}

impl MarksClip {
    fn matches(&self, clip: &Clip) -> bool {
        match clip {
            Clip::Text(text) => self.image.is_none() && text == &self.marker,
            Clip::Empty => self.image.is_none(),
            Clip::Image(bytes, _) => self.image.as_ref().is_some_and(|own| {
                if let Some(written) = own.sequence {
                    return sequence() == Some(written);
                }
                mypdf_milestone0::content::decode_image(bytes)
                    .is_ok_and(|pixels| fingerprint(&pixels) == own.fingerprint)
            }),
        }
    }
}

/// Image-only copies keep original pixels when possible. More complex selections
/// use their PDF appearances on a transparent canvas, never a page screenshot.
fn image_for_clipboard(marks: &[Annotation]) -> anyhow::Result<image::RgbaImage> {
    use mypdf_milestone0::{annotations, assets, object_tools};
    for mark in marks {
        mark.validate()?;
    }
    if let [mark] = marks
        && matches!(mark.kind, Kind::Image { .. })
        && !mark.style.image_border
        && mark.rotation.rem_euclid(90.) == 0.
    {
        let mut pixels = assets::decode_png(&object_tools::image_appearance(mark)?)?.into_rgba8();
        if mark.flip_x {
            image::imageops::flip_horizontal_in_place(&mut pixels);
        }
        if mark.flip_y {
            image::imageops::flip_vertical_in_place(&mut pixels);
        }
        pixels = match mark.rotation.rem_euclid(360.) as u32 {
            90 => image::imageops::rotate90(&pixels),
            180 => image::imageops::rotate180(&pixels),
            270 => image::imageops::rotate270(&pixels),
            _ => pixels,
        };
        for pixel in pixels.pixels_mut() {
            pixel[3] = (pixel[3] as f32 * mark.style.opacity).round() as u8;
        }
        return Ok(pixels);
    }
    let bounds = selection::union_bounds(marks.iter());
    let (w, h) = (bounds[2] - bounds[0], bounds[3] - bounds[1]);
    anyhow::ensure!(
        w.is_finite() && h.is_finite() && w > 0. && h > 0.,
        "Invalid image bounds"
    );
    let mut document = lopdf::Document::load_mem(&mypdf_milestone0::convert::blank(72., 72.)?)?;
    let page = document.get_pages()[&1];
    document.get_object_mut(page)?.as_dict_mut()?.set(
        "MediaBox",
        vec![
            0.into(),
            0.into(),
            lopdf::Object::Real(w),
            lopdf::Object::Real(h),
        ],
    );
    let mut base = Vec::new();
    document.save_to(&mut base)?;
    let marks: Vec<_> = marks
        .iter()
        .map(|a| {
            let mut a = a.clone();
            a.page = 0;
            a.object.hidden = false;
            a.rect = [
                a.rect[0] - bounds[0],
                a.rect[1] - bounds[1],
                a.rect[2] - bounds[0],
                a.rect[3] - bounds[1],
            ];
            a
        })
        .collect();
    let pdf = hayro::hayro_syntax::Pdf::new(annotations::export(&base, &marks)?)
        .map_err(|e| anyhow::anyhow!("Could not render copied artwork: {e:?}"))?;
    let scale = 2_f32
        .min(4096. / w.max(h))
        .min((16_000_000. / (w * h)).sqrt());
    let raster = hayro::render(
        &pdf.pages()[0],
        &hayro::RenderCache::new(),
        &mypdf_milestone0::render_fonts::settings(),
        &hayro::RenderSettings::default(),
        &hayro::PixmapSettings {
            x_scale: scale,
            y_scale: scale,
            bg_color: hayro::vello_cpu::color::palette::css::TRANSPARENT,
        },
    );
    Ok(image::load_from_memory(&raster.into_png()?)?.into_rgba8())
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
            && !self
                .tabs
                .get(self.active)
                .is_some_and(|t| self.table.has_editor(t.id))
            && !self.table.deferring_input()
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
        let image = if marks.iter().any(|a| {
            a.asset_id.is_some()
                || matches!(
                    a.kind,
                    Kind::Image { .. } | Kind::Ink { .. } | Kind::Svg { .. }
                )
        }) {
            let copied = image_for_clipboard(&marks).and_then(|pixels| {
                write_image(&pixels)?;
                Ok(ImageClip {
                    sequence: sequence(),
                    fingerprint: fingerprint(&pixels),
                })
            });
            match copied {
                Ok(image) => {
                    // The image was written directly to the native clipboard.
                    // eframe must not replace it with an earlier queued text
                    // copy when this frame finishes.
                    ctx.output_mut(|output| {
                        output
                            .commands
                            .retain(|command| !matches!(command, egui::OutputCommand::CopyText(_)));
                    });
                    Some(image)
                }
                Err(e) => {
                    self.status = Some((
                        format!("Could not copy image: {e:#}. The document was not changed."),
                        true,
                    ));
                    return None;
                }
            }
        } else {
            ctx.copy_text(marker.clone());
            None
        };
        self.editor.marks_clip = Some(MarksClip {
            marks,
            marker: marker.clone(),
            image,
        });
        Some(n)
    }

    /// Paste marks, or an image or text from another app, on the current page.
    pub(super) fn paste(&mut self, ctx: &egui::Context) {
        self.paste_clip(ctx, read());
    }

    pub(super) fn paste_clip(&mut self, ctx: &egui::Context, clip: Clip) {
        let ours = self
            .editor
            .marks_clip
            .as_ref()
            .is_some_and(|m| m.matches(&clip));
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
                .is_some_and(|m| m.matches(&clip));
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

    fn text_fixture() -> (Viewer, egui::Context) {
        let ctx = egui::Context::default();
        let app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        (app, ctx)
    }

    fn sample_rich_text() -> mypdf_milestone0::clipboard_text::ClipboardText {
        mypdf_milestone0::clipboard_text::ClipboardText {
            text: "café & Ω".into(),
            html: "<p><strong>café &amp; Ω</strong></p>".into(),
        }
    }

    #[test]
    fn formatted_copy_keeps_plain_alternative_without_eframe_overwrite() {
        let (mut app, ctx) = text_fixture();
        let copied = sample_rich_text();
        let output = ctx.run(Default::default(), |ctx| {
            ctx.copy_text("Earlier copy command".into());
            app.copy_formatted_text(ctx, copied.clone());
        });
        assert_eq!(
            rich_text_for_test(),
            Some((copied.text.clone(), copied.html))
        );
        assert!(matches!(read(), Clip::Text(text) if text == copied.text));
        assert!(
            !output
                .platform_output
                .commands
                .iter()
                .any(|command| { matches!(command, egui::OutputCommand::CopyText(_)) })
        );
        assert!(
            app.status
                .as_ref()
                .is_some_and(|(text, error)| { !error && text.contains("with formatting") })
        );
    }

    #[test]
    fn formatted_copy_falls_back_to_plain_text_with_truthful_status() {
        let (mut app, ctx) = text_fixture();
        FAIL_TEXT_WRITE.with(|fail| fail.set((true, false)));
        app.copy_formatted_text(&ctx, sample_rich_text());
        assert!(rich_text_for_test().is_none());
        assert!(matches!(read(), Clip::Text(text) if text == "café & Ω"));
        assert!(app.status.as_ref().is_some_and(|(text, error)| {
            !error && text.contains("as plain text") && text.contains("formatting was unavailable")
        }));
    }

    #[test]
    fn copy_failure_is_reported_instead_of_claiming_success() {
        let (mut app, ctx) = text_fixture();
        FAIL_TEXT_WRITE.with(|fail| fail.set((true, true)));
        app.copy_formatted_text(&ctx, sample_rich_text());
        assert!(rich_text_for_test().is_none());
        assert!(app.status.as_ref().is_some_and(|(text, error)| {
            *error && text.starts_with("Could not copy") && !text.contains("Copied")
        }));
    }

    fn image_fixture() -> (Viewer, egui::Context, Annotation) {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.open_memory(
            PathBuf::from("Clipboard test.pdf"),
            mypdf_milestone0::convert::blank(595., 842.).unwrap(),
            &ctx,
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let bytes = png(2, 1, &[240, 20, 10, 128, 10, 20, 240, 255]).unwrap();
        let mark = Annotation::new(
            0,
            [100., 100., 300., 200.],
            Kind::Image {
                png: Arc::new(bytes),
            },
            Default::default(),
        );
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![mark.clone()])
            .unwrap();
        selection::set(&mut app.tabs[0], &mut app.editor, vec![mark.id.clone()]);
        (app, ctx, mark)
    }

    #[test]
    fn ink_and_name_signatures_copy_as_images_and_keep_editable_internal_paste() {
        let (mut app, ctx, mut signature) = image_fixture();
        signature.asset_id = Some("saved-signature".into());
        for kind in [
            Kind::Ink {
                strokes: vec![vec![[0.1, 0.8], [0.3, 0.1], [0.5, 0.7], [0.9, 0.2]]],
            },
            Kind::Svg {
                source: "<svg xmlns='http://www.w3.org/2000/svg' width='200' height='100'><path d='M10 80 Q50 5 80 70 L180 20' fill='none' stroke='blue' stroke-width='3'/></svg>".into(),
                tint: false,
            },
            Kind::Text { text: "Orhan".into() },
        ] {
            signature.kind = kind;
            assert_eq!(app.copy_artwork(&ctx, vec![signature.clone()]), Some(1));
            let clip = read();
            let Clip::Image(bytes, _) = &clip else {
                panic!("A copied signature must be available to image consumers");
            };
            let pixels = mypdf_milestone0::content::decode_image(bytes).unwrap();
            assert!(pixels.pixels().any(|p| p[3] > 0));
            assert!(pixels.pixels().any(|p| p[3] == 0));
            set_for_test(clip);
            app.paste(&ctx);
            let pasted = &app.workbench.paste.as_ref().unwrap()[0];
            assert_eq!(pasted.kind, signature.kind);
            assert_ne!(pasted.id, signature.id);
        }
    }

    #[test]
    fn image_only_clipboard_pastes_with_key_event_without_text_paste_event() {
        let (mut app, ctx, _) = image_fixture();
        app.home.visible = false;
        app.editor.mode = editing::Mode::Edit;
        let bytes = png(200, 80, &[0, 0, 200, 255].repeat(200 * 80)).unwrap();
        set_for_test(Clip::Image(bytes, [200, 80]));
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::V,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                }],
                ..Default::default()
            },
            |ctx| app.shortcuts(ctx),
        );
        assert!(matches!(
            app.workbench.paste.as_ref().unwrap()[0].kind,
            Kind::Image { .. }
        ));
        app.place_pending_original();
        let session = app.tabs[0].session.as_mut().unwrap();
        assert_eq!(session.annotations.len(), 2);
        session.undo();
        assert_eq!(session.annotations.len(), 1);
        session.redo();
        assert_eq!(session.annotations.len(), 2);
    }

    #[test]
    fn image_copy_exports_pixels_and_internal_paste_keeps_editable_original() {
        let (mut app, ctx, original) = image_fixture();
        let output = ctx.run(Default::default(), |ctx| {
            ctx.copy_text("Earlier copied text".into());
            assert!(app.copy_marks(ctx, false));
        });
        let Clip::Image(bytes, size) = read() else {
            panic!("Copy must publish an image, not a marker")
        };
        assert_eq!(size, [2, 1]);
        assert_eq!(
            mypdf_milestone0::content::decode_image(&bytes)
                .unwrap()
                .as_raw(),
            &[240, 20, 10, 128, 10, 20, 240, 255]
        );
        assert!(
            !output
                .platform_output
                .commands
                .iter()
                .any(|c| matches!(c, egui::OutputCommand::CopyText(_))),
            "Queued text must not replace native image data at the end of the frame"
        );
        set_for_test(Clip::Image(bytes, size));
        app.paste(&ctx);
        let draft = &app.workbench.paste.as_ref().unwrap()[0];
        assert_eq!(draft.rect, original.rect);
        assert_eq!(draft.kind, original.kind);
        assert_ne!(draft.id, original.id);
        app.workbench.paste = None;
        set_for_test(Clip::Image(png(1, 1, &[9, 8, 7, 255]).unwrap(), [1, 1]));
        app.paste(&ctx);
        assert_ne!(app.workbench.paste.as_ref().unwrap()[0].kind, original.kind);
        app.workbench.paste = None;
        set_for_test(Clip::Empty);
        app.paste(&ctx);
        assert!(
            app.workbench.paste.is_none(),
            "An empty clipboard must not resurrect old images"
        );
    }

    #[test]
    fn image_cut_requires_successful_clipboard_write_and_undo_restores_original() {
        let (mut app, ctx, original) = image_fixture();
        FAIL_IMAGE_WRITE.with(|f| f.set(true));
        assert!(!app.copy_marks(&ctx, true));
        assert_eq!(
            app.tabs[0].session.as_ref().unwrap().annotations,
            vec![original.clone()]
        );
        assert_eq!(
            selection::selected_ids(&app.tabs[0]),
            vec![original.id.clone()]
        );
        assert!(app.status.as_ref().unwrap().1);
        assert!(app.copy_marks(&ctx, true));
        assert!(matches!(read(), Clip::Image(..)));
        let s = app.tabs[0].session.as_mut().unwrap();
        assert!(s.annotations.is_empty());
        s.undo();
        assert_eq!(s.annotations, vec![original]);
    }

    #[test]
    fn copied_image_appearance_preserves_crop_transform_opacity_and_transparency() {
        let (_, _, mut mark) = image_fixture();
        mark.flip_x = true;
        mark.rotation = 90.;
        mark.style.opacity = 0.5;
        let pixels = image_for_clipboard(&[mark.clone()]).unwrap();
        assert_eq!(pixels.dimensions(), (1, 2));
        assert_eq!(pixels.get_pixel(0, 0).0, [10, 20, 240, 128]);
        assert_eq!(pixels.get_pixel(0, 1).0, [240, 20, 10, 64]);
        mark.object.crop = Some([0., 0., 0.5, 1.]);
        assert_eq!(
            image_for_clipboard(&[mark.clone()]).unwrap().dimensions(),
            (1, 1)
        );
        mark.object.crop = None;
        mark.rotation = 30.;
        mark.style.image_border = true;
        let pixels = image_for_clipboard(&[mark]).unwrap();
        assert_eq!(pixels.get_pixel(0, 0)[3], 0);
        assert!(pixels.pixels().any(|p| p[3] > 0));
    }

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
