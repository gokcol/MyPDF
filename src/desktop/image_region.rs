use super::*;
use mypdf_milestone0::{annotations as ann, image_region};

#[derive(Default)]
pub(super) struct State {
    pub active: bool,
    pub key: Option<Key>,
    pub drag: Option<(Key, egui::Pos2)>,
    request: Option<(Key, [f32; 4])>,
    draft: Option<Draft>,
}
struct Draft {
    key: Key,
    rect: [f32; 4],
    source: Arc<[u8]>,
    annotations: Vec<ann::Annotation>,
    dpi: f32,
    rendered_dpi: Option<f32>,
    pending: Option<mpsc::Receiver<Result<Vec<u8>, String>>>,
    preview: Option<(Arc<Vec<u8>>, TextureHandle, [u32; 2])>,
    error: Option<String>,
    cover: bool,
    color: [u8; 3],
}

pub(super) fn overlay(
    state: &mut State,
    tab: &Tab,
    ui: &mut egui::Ui,
    page: usize,
    page_rect: Rect,
    scale: f32,
) {
    let Some(session) = &tab.session else { return };
    let key = (tab.id, session.revision(), page);
    if state.key != Some(key) || !session.editable {
        return;
    }
    let response = ui
        .interact(
            page_rect,
            egui::Id::new((tab.id, page, "copy_region")),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::Crosshair);
    if response.drag_started()
        && let Some(start) = ui.input(|i| i.pointer.press_origin())
    {
        state.drag = Some((key, start));
    }
    if let Some((drag_key, start)) = state.drag
        && drag_key == key
        && let Some(end) = ui.input(|i| i.pointer.interact_pos())
    {
        let r = Rect::from_two_pos(start, end).intersect(page_rect);
        let painter = ui
            .painter()
            .with_clip_rect(page_rect.intersect(ui.clip_rect()));
        painter.rect_filled(r, 0., theme::accent().gamma_multiply(0.1));
        ui::dashed_rect(&painter, r, theme::accent());
        if response.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
            let a = (r.min - page_rect.min) / scale;
            let b = (r.max - page_rect.min) / scale;
            if b.x - a.x >= 2. && b.y - a.y >= 2. {
                state.request = Some((
                    key,
                    [
                        a.x.max(0.),
                        a.y.max(0.),
                        b.x.min(tab.pages[page].width),
                        b.y.min(tab.pages[page].height),
                    ],
                ));
                state.active = false;
            }
            state.drag = None;
        }
    }
}

impl Viewer {
    #[cfg(test)]
    pub(in crate::desktop) fn region_preview(&mut self) {
        let tab = &self.tabs[self.active];
        let session = tab.session.as_ref().unwrap();
        self.editor.mode = editing::Mode::Edit;
        self.content.image_mode = true;
        self.content.region.request = Some((
            (tab.id, session.revision(), tab.page),
            [40., 35., 440., 195.],
        ));
    }
    #[cfg(test)]
    pub(in crate::desktop) fn region_preview_ready(&self) -> bool {
        self.content
            .region
            .draft
            .as_ref()
            .is_some_and(|d| d.preview.is_some() && d.pending.is_none())
    }
    pub(in crate::desktop) fn image_region_window(&mut self, ctx: &egui::Context) {
        let current = self
            .tabs
            .get(self.active)
            .and_then(|t| t.session.as_ref().map(|s| (t.id, s.revision(), t.page)));
        if self.content.region.active
            && (self.content.region.key != current
                || self.editor.mode != editing::Mode::Edit
                || !self.content.image_mode
                || self.redaction.active
                || ctx.input(|i| i.key_pressed(egui::Key::Escape)))
        {
            self.content.region.active = false;
            self.content.region.drag = None;
        }
        if let Some((key, rect)) = self.content.region.request.take()
            && current == Some(key)
            && let Some(session) = self.tabs[self.active].session.as_ref()
            && session.editable
        {
            self.content.region.draft = Some(Draft {
                key,
                rect,
                source: session.source.clone(),
                annotations: session.annotations.clone(),
                dpi: 200.,
                rendered_dpi: None,
                pending: None,
                preview: None,
                error: None,
                cover: false,
                color: [255; 3],
            });
        }
        let Some(draft) = &mut self.content.region.draft else {
            return;
        };
        if let Some(rx) = &draft.pending
            && let Ok(result) = rx.try_recv()
        {
            draft.pending = None;
            match result.and_then(|png| {
                let image =
                    mypdf_milestone0::assets::decode_png(&png).map_err(|e| e.to_string())?;
                let size = [image.width(), image.height()];
                Ok((png, size, image.thumbnail(900, 500).into_rgba8()))
            }) {
                Ok((png, size, image)) => {
                    draft.preview = Some((
                        Arc::new(png),
                        ctx.load_texture(
                            "region_copy",
                            egui::ColorImage::from_rgba_unmultiplied(
                                [image.width() as usize, image.height() as usize],
                                image.as_raw(),
                            ),
                            egui::TextureOptions::LINEAR,
                        ),
                        size,
                    ));
                    draft.error = None;
                }
                Err(error) => draft.error = Some(error),
            }
        }
        let valid = current == Some(draft.key)
            && self
                .tabs
                .get(self.active)
                .and_then(|t| t.session.as_ref())
                .is_some_and(|s| s.editable)
            && !self.content.busy
            && !self.organizer.busy;
        let (mut add, mut cancel) = (false, false);
        let modal = egui::Modal::new(egui::Id::new("copy_region_preview"))
            .frame(ui::modal_frame()).show(ctx, |ui| {
                ui.set_width(460.);
                ui.heading("Copy region as image");
                ui.label("Everything inside the rectangle becomes one editable image layer.");
                ui.add_space(12.);
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 220.), egui::Sense::hover());
                ui.painter().rect_filled(rect, 8., theme::surface());
                if let Some((_, texture, size)) = &draft.preview {
                    let scale = (rect.width() / texture.size_vec2().x).min(rect.height() / texture.size_vec2().y);
                    ui.painter().image(texture.id(), Rect::from_center_size(rect.center(), texture.size_vec2() * scale),
                        Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)), Color32::WHITE);
                    ui.weak(format!("{} × {} pixels · raster copy", size[0], size[1]));
                }
                ui.add_enabled_ui(draft.pending.is_none(), |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Quality");
                        for (label, dpi) in [("150 DPI", 150.), ("200 DPI", 200.), ("300 DPI", 300.)] {
                            ui.selectable_value(&mut draft.dpi, dpi, label);
                        }
                    });
                });
                ui.add_space(8.);
                ui.checkbox(&mut draft.cover, "Cover the original area");
                if draft.cover {
                    ui.horizontal(|ui| { ui.label("Cover color"); ui.color_edit_button_srgb(&mut draft.color); });
                    ui.weak("Adds a separate rectangle behind the copy. The cover stays in place when you move the image; delete it to reveal the original.");
                    ui.weak("This is a visual cover, not redaction. Underlying content remains in the PDF.");
                } else {
                    ui.weak("The original stays visible when you move the copy or remove its background.");
                }
                if draft.pending.is_some() { ui.horizontal(|ui| { ui.spinner(); ui.label("Preparing preview…"); }); }
                if let Some(error) = &draft.error { ui.colored_label(theme::danger(), error); }
                if !valid { ui.colored_label(theme::danger(), "The document or page changed. Cancel and select the region again."); }
                ui.add_space(12.);
                ui.horizontal(|ui| {
                    let response = ui.add_enabled(valid && draft.preview.is_some() && draft.pending.is_none()
                        && draft.error.is_none() && draft.rendered_dpi == Some(draft.dpi),
                        ActionButton::new(Icon::Plus, "Add image layer").primary());
                    #[cfg(test)]
                    ctx.data_mut(|d| d.insert_temp(egui::Id::new("region_add_button"), response.rect));
                    add = response.clicked();
                    cancel = ui.add(ActionButton::new(Icon::Close, "Cancel")).clicked();
                });
            });
        if draft.pending.is_none() && draft.rendered_dpi != Some(draft.dpi) && valid {
            let (source, annotations, rect, dpi, page) = (
                draft.source.clone(),
                draft.annotations.clone(),
                draft.rect,
                draft.dpi,
                draft.key.2,
            );
            draft.rendered_dpi = Some(dpi);
            draft.preview = None;
            draft.error = None;
            let (tx, rx) = mpsc::channel();
            draft.pending = Some(rx);
            let wake = ctx.clone();
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(|| -> anyhow::Result<Vec<u8>> {
                    let snapshot = ann::export(&source, &annotations)?;
                    image_region::capture(&snapshot, page, rect, dpi)
                })
                .map_err(|_| "Could not capture this region".to_owned())
                .and_then(|r| r.map_err(|e| format!("{e:#}")));
                let _ = tx.send(result);
                wake.request_repaint();
            });
        }
        if add {
            let draft = self.content.region.draft.take().unwrap();
            let cover = draft.cover.then(|| draft.color.map(|c| c as f32 / 255.));
            self.use_region_copy(draft.key, draft.rect, draft.preview.unwrap().0, cover);
        } else if cancel || modal.should_close() {
            self.content.region.draft = None;
        }
    }

    fn use_region_copy(
        &mut self,
        key: Key,
        rect: [f32; 4],
        png: Arc<Vec<u8>>,
        cover: Option<[f32; 3]>,
    ) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let Some(session) = tab.session.as_mut() else {
            return;
        };
        if (tab.id, session.revision(), tab.page) != key {
            return;
        }
        let result = image_region::layers(key.2, rect, png, cover).and_then(|layers| {
            let selected = layers.last().unwrap().id.clone();
            let mut annotations = session.annotations.clone();
            annotations.extend(layers);
            session.commit(annotations)?;
            Ok(selected)
        });
        match result {
            Ok(selected) => {
                self.content.region.active = false;
                self.editor.mode = editing::Mode::Draw;
                self.editor.tool = editing::Tool::Select;
                selection::set(tab, &mut self.editor, vec![selected]);
                self.inspector = true;
                self.status = Some(("Image layer added • Drag to move • Image controls are in the right panel • Undo restores the page".into(), false));
            }
            Err(error) => self.status = Some((format!("{error:#}"), true)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Viewer, egui::Context, usize) {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(80, 50, image::Rgba([25, 80, 210, 255]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let source = content::add_image(
            &convert::blank(300., 240.).unwrap(),
            1,
            [40., 40., 140., 100.],
            png.get_ref(),
        )
        .unwrap();
        let images = content::images(&source, 1).unwrap();
        let index = images[0].index;
        assert_ne!(index, 0);
        app.open_memory(PathBuf::from("Region check.pdf"), source, &ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let key = (
            app.tabs[0].id,
            app.tabs[0].session.as_ref().unwrap().revision(),
            0,
        );
        app.editor.mode = editing::Mode::Edit;
        app.content.image_mode = true;
        app.content.key = Some(key);
        app.content.inspection = Some(Inspection {
            key,
            images: Ok(images),
            text: Ok(vec![]),
            fields: Ok(vec![]),
            designed: Ok(vec![]),
        });
        (app, ctx, index)
    }
    fn frame(app: &mut Viewer, ctx: &egui::Context, events: Vec<egui::Event>) {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1000., 800.))),
                events,
                ..Default::default()
            },
            |ctx| {
                app.content_panel(ctx);
                egui::CentralPanel::default().show(ctx, |ui| {
                    super::super::overlay(
                        &mut app.content,
                        app.editor.mode,
                        &app.tabs[0],
                        ui,
                        0,
                        Rect::from_min_max(pos2(10., 10.), pos2(310., 250.)),
                        1.,
                    );
                });
                app.image_region_window(ctx);
            },
        );
    }
    fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }
    #[test]
    fn embedded_image_inspector_resolves_operator_id_from_selected_list_entry() {
        let (mut app, ctx, index) = fixture();
        app.content.selected = Some(0);
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.content.image_effects.as_ref().unwrap().key.1, index);
        // A stale selection must not prevent the inspector or Copy region tool from rendering.
        app.content.selected = Some(99);
        frame(&mut app, &ctx, vec![]);
    }
    #[test]
    fn region_drag_preview_add_selects_new_image_and_undo_removes_both_layers() {
        let (mut app, ctx, _) = fixture();
        let key = app.content.key.unwrap();
        app.content.region.active = true;
        app.content.region.key = Some(key);
        frame(&mut app, &ctx, vec![]);
        let start = pos2(45., 45.);
        let end = pos2(155., 115.);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(start), button(start, true)],
        );
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(end)]);
        frame(&mut app, &ctx, vec![button(end, false)]);
        assert!(app.content.region.draft.is_some());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while app.content.region.draft.as_ref().unwrap().preview.is_none() {
            frame(&mut app, &ctx, vec![]);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.content.region.draft.as_mut().unwrap().cover = true;
        frame(&mut app, &ctx, vec![]);
        let add = ctx
            .data(|d| d.get_temp::<Rect>(egui::Id::new("region_add_button")))
            .unwrap()
            .center();
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(add), button(add, true)],
        );
        frame(&mut app, &ctx, vec![button(add, false)]);
        assert!(app.content.region.draft.is_none());
        assert_eq!(app.editor.mode, editing::Mode::Draw);
        let s = app.tabs[0].session.as_ref().unwrap();
        assert_eq!(s.annotations.len(), 2);
        assert_eq!(s.annotations[1].rect, [35., 35., 145., 105.]);
        assert!(matches!(s.annotations[1].kind, ann::Kind::Image { .. }));
        let saved = s.snapshot_bytes().unwrap();
        assert_eq!(
            ann::load(&lopdf::Document::load_mem(&saved).unwrap()).len(),
            2
        );
        app.editor_undo(false);
        assert!(app.tabs[0].session.as_ref().unwrap().annotations.is_empty());
    }
}
