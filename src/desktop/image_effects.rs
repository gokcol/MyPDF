use super::*;
use mypdf_milestone0::assets;
use mypdf_milestone0::image_effects::{self, RasterOptions};

type PreviewResult = (RasterOptions, Result<Vec<u8>, String>);

pub(super) struct EmbeddedEffects {
    pub key: (Key, usize),
    source: Result<Arc<Vec<u8>>, String>,
    options: RasterOptions,
    preview_options: Option<RasterOptions>,
    preview: Option<(Vec<u8>, TextureHandle)>,
    pending: Option<mpsc::Receiver<PreviewResult>>,
    error: Option<String>,
}
impl EmbeddedEffects {
    pub fn new(key: (Key, usize), source: Result<Vec<u8>, String>) -> Self {
        Self {
            key,
            source: source.map(Arc::new),
            options: Default::default(),
            preview_options: None,
            preview: None,
            pending: None,
            error: None,
        }
    }
    pub fn panel(&mut self, ui: &mut egui::Ui) -> Option<Vec<u8>> {
        ui::caption(ui, "Image appearance");
        if let Err(error) = &self.source {
            ui.weak(error);
            return None;
        }
        if let Some(rx) = &self.pending
            && let Ok((options, result)) = rx.try_recv()
        {
            self.pending = None;
            if options == self.options {
                self.preview_options = Some(options);
                match result.and_then(|bytes| {
                    assets::decode_png(&bytes)
                        .map(|image| (bytes, image.thumbnail(300, 180).into_rgba8()))
                        .map_err(|e| e.to_string())
                }) {
                    Ok((bytes, image)) => {
                        let texture = ui.ctx().load_texture(
                            "embedded_image_effects",
                            egui::ColorImage::from_rgba_unmultiplied(
                                [image.width() as usize, image.height() as usize],
                                image.as_raw(),
                            ),
                            egui::TextureOptions::LINEAR,
                        );
                        self.preview = Some((bytes, texture));
                        self.error = None;
                    }
                    Err(error) => self.error = Some(error),
                }
            }
        }
        let (rect, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), 130.), egui::Sense::hover());
        let painter = ui.painter().with_clip_rect(rect);
        for y in 0..11 {
            for x in 0..32 {
                painter.rect_filled(
                    Rect::from_min_size(
                        rect.min + vec2(x as f32 * 12., y as f32 * 12.),
                        vec2(12., 12.),
                    ),
                    0.,
                    if (x + y) % 2 == 0 {
                        Color32::from_gray(220)
                    } else {
                        Color32::WHITE
                    },
                );
            }
        }
        if let Some((_, texture)) = &self.preview {
            let scale =
                (rect.width() / texture.size_vec2().x).min(rect.height() / texture.size_vec2().y);
            painter.image(
                texture.id(),
                Rect::from_center_size(rect.center(), texture.size_vec2() * scale),
                Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)),
                Color32::WHITE,
            );
        }
        ui::numeric_slider(
            ui,
            egui::Slider::new(&mut self.options.opacity, 0.0..=1.).text("Opacity"),
        );
        ui::numeric_slider(
            ui,
            egui::Slider::new(&mut self.options.blur, 0.0..=32.)
                .step_by(0.5)
                .text("Blur · px"),
        );
        ui::numeric_slider(
            ui,
            egui::Slider::new(&mut self.options.border, 0..=100).text("Border · px"),
        );
        if self.options.border > 0 {
            ui.horizontal(|ui| {
                ui.label("Border color");
                ui::color_swatch_srgb(ui, &mut self.options.color, "Border color");
            });
        }
        if self.pending.is_none() && self.preview_options != Some(self.options) {
            let source = self.source.as_ref().unwrap().clone();
            let options = self.options;
            let (tx, rx) = mpsc::channel();
            self.pending = Some(rx);
            let wake = ui.ctx().clone();
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(|| image_effects::raster(&source, options))
                    .map_err(|_| "Could not process this image".to_owned())
                    .and_then(|r| r.map_err(|e| e.to_string()));
                let _ = tx.send((options, result));
                wake.request_repaint();
            });
        }
        if self.pending.is_some() {
            ui.spinner();
        }
        if let Some(error) = &self.error {
            ui.colored_label(theme::danger(), error);
        }
        ui.weak("Preview updates here. Apply writes this appearance into the image; Undo restores the original.");
        if ui
            .add_enabled(
                self.options != RasterOptions::default()
                    && self.preview_options == Some(self.options)
                    && self.error.is_none()
                    && self.preview.is_some(),
                ActionButton::new(Icon::Check, "Apply image appearance"),
            )
            .clicked()
        {
            return self.preview.as_ref().map(|p| p.0.clone());
        }
        None
    }
}
