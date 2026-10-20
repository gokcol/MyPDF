use super::*;
use mypdf_milestone0::image_background::{self, Options};

pub(super) enum Target {
    SavedSignature {
        signature: String,
        asset: Asset,
    },
    ContentImage {
        tab: u64,
        page: usize,
        index: usize,
        revision: u64,
    },
    Annotation {
        tab: u64,
        id: String,
        revision: u64,
    },
    Signature,
}
type ResultImage = (u64, Result<Vec<u8>, String>);
pub(super) struct Draft {
    target: Target,
    original: Arc<Vec<u8>>,
    options: Options,
    changed: std::time::Instant,
    generation: u64,
    dirty: bool,
    busy: bool,
    preview: Option<(Arc<Vec<u8>>, TextureHandle)>,
    error: Option<String>,
    tx: mpsc::Sender<ResultImage>,
    rx: mpsc::Receiver<ResultImage>,
}
impl Draft {
    pub(super) fn new(target: Target, original: Arc<Vec<u8>>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            target,
            original,
            options: Options::default(),
            changed: std::time::Instant::now() - std::time::Duration::from_secs(1),
            generation: 0,
            dirty: true,
            busy: false,
            preview: None,
            error: None,
            tx,
            rx,
        }
    }
}
impl Viewer {
    pub(in crate::desktop) fn remove_signature_image_background(
        &mut self,
        signature: String,
        asset: Asset,
    ) {
        if let Kind::Image { png } = &asset.kind {
            self.editor.background = Some(Draft::new(
                Target::SavedSignature {
                    signature,
                    asset: asset.clone(),
                },
                png.clone(),
            ));
        }
    }
    #[cfg(test)]
    pub(in crate::desktop) fn background_preview(&mut self) {
        let image = image::RgbaImage::from_fn(440, 180, |x, y| {
            let curve = 88. + (x as f32 / 21.).sin() * 30. + (x as f32 / 9.).sin() * 12.;
            if (30..410).contains(&x) && (y as f32 - curve).abs() < 2.5 {
                image::Rgba([24, 35, 65, 255])
            } else {
                image::Rgba([255, 255, 255, 255])
            }
        });
        let mut png = std::io::Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        self.editor.background = Some(Draft::new(Target::Signature, Arc::new(png.into_inner())));
    }
    #[cfg(test)]
    pub(in crate::desktop) fn background_preview_ready(&self) -> bool {
        self.editor
            .background
            .as_ref()
            .is_some_and(|d| d.preview.is_some() && !d.busy && !d.dirty)
    }
    pub(in crate::desktop) fn remove_content_image_background(&mut self, index: usize) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let Some(session) = &tab.session else {
            return;
        };
        match mypdf_milestone0::content::image_png(&session.source, tab.page as u32 + 1, index) {
            Ok(png) => {
                self.editor.background = Some(Draft::new(
                    Target::ContentImage {
                        tab: tab.id,
                        page: tab.page,
                        index,
                        revision: session.revision(),
                    },
                    Arc::new(png),
                ))
            }
            Err(error) => self.status = Some((format!("{error:#}"), true)),
        }
    }
    pub(in crate::desktop) fn background_window(&mut self, ctx: &egui::Context) {
        let Some(draft) = &mut self.editor.background else {
            return;
        };
        for (generation, result) in draft.rx.try_iter() {
            draft.busy = false;
            if generation != draft.generation {
                continue;
            }
            match result.and_then(|bytes| {
                assets::decode_png(&bytes)
                    .map(|image| (bytes, image.to_rgba8()))
                    .map_err(|e| e.to_string())
            }) {
                Ok((bytes, image)) => {
                    let size = [image.width() as usize, image.height() as usize];
                    draft.preview = Some((
                        Arc::new(bytes),
                        ctx.load_texture(
                            "transparent_artwork",
                            egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
                            egui::TextureOptions::LINEAR,
                        ),
                    ));
                    draft.error = None;
                }
                Err(error) => draft.error = Some(error),
            }
        }
        let before = draft.options;
        let mut apply = false;
        let mut cancel = false;
        let modal=egui::Modal::new(egui::Id::new("remove_image_background")).frame(ui::modal_frame()).show(ctx,|ui| {
            ui.set_width(470.);
            ui.heading("Remove background");
            ui.label("Make a paper or solid-color background transparent.");
            ui.add_space(12.);
            let (rect,_)=ui.allocate_exact_size(vec2(ui.available_width(),240.),egui::Sense::hover());
            let painter=ui.painter().with_clip_rect(rect);
            for y in 0..20 { for x in 0..40 {
                let square=Rect::from_min_size(rect.min+vec2(x as f32*12.,y as f32*12.),vec2(12.,12.)).intersect(rect);
                painter.rect_filled(square,0.,if (x+y)%2==0 {Color32::from_gray(230)} else {Color32::WHITE});
            }}
            if let Some((_,texture))=&draft.preview {
                let scale=(rect.width()/texture.size_vec2().x).min(rect.height()/texture.size_vec2().y);
                painter.image(texture.id(),Rect::from_center_size(rect.center(),texture.size_vec2()*scale),Rect::from_min_max(pos2(0.,0.),pos2(1.,1.)),Color32::WHITE);
            }
            ui.horizontal(|ui| { ui.label("Background color"); ui::color_swatch_srgb(ui, &mut draft.options.color, "Background color"); });
            ui.add(egui::Slider::new(&mut draft.options.tolerance,0..=120).text("Tolerance"));
            ui.add(egui::Slider::new(&mut draft.options.feather,0..=64).text("Soft edges"));
            ui.checkbox(&mut draft.options.edges_only,"Only remove background connected to an edge").on_hover_text("Preserves matching areas enclosed inside the image. Turn off for paper inside loops in a signature.");
            ui.weak("Best for scans, signatures, and images with a solid background.");
            if draft.busy || draft.dirty { ui.horizontal(|ui| {ui.spinner(); ui.label("Updating preview…");}); }
            if let Some(error)=&draft.error { ui.colored_label(theme::danger(),error); }
            ui.add_space(12.);
            ui.horizontal(|ui| {
                apply=ui.add_enabled(before==draft.options&&draft.preview.is_some()&&!draft.busy&&!draft.dirty&&draft.error.is_none(),ActionButton::new(Icon::Check,"Use transparent image").primary()).clicked();
                cancel=ui.add(ActionButton::new(Icon::Close,"Cancel")).clicked();
            });
        });
        if before != draft.options {
            draft.generation += 1;
            draft.dirty = true;
            draft.changed = std::time::Instant::now();
        }
        if draft.dirty
            && !draft.busy
            && draft.changed.elapsed() >= std::time::Duration::from_millis(150)
        {
            draft.dirty = false;
            draft.busy = true;
            let (source, options, generation, tx, wake) = (
                draft.original.clone(),
                draft.options,
                draft.generation,
                draft.tx.clone(),
                ctx.clone(),
            );
            std::thread::spawn(move || {
                let result =
                    std::panic::catch_unwind(|| image_background::remove(&source, options))
                        .map_err(|_| "Could not process this image".into())
                        .and_then(|r| r.map_err(|e| e.to_string()));
                let _ = tx.send((generation, result));
                wake.request_repaint();
            });
        }
        if draft.dirty {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }
        if apply {
            let draft = self.editor.background.take().unwrap();
            let png = draft.preview.unwrap().0;
            match draft.target {
                Target::SavedSignature { signature, asset } => {
                    self.use_transparent_signature_image(&signature, asset, png);
                }
                Target::ContentImage {
                    tab,
                    page,
                    index,
                    revision,
                } => {
                    let Some(active) = self.tabs.iter().position(|t| t.id == tab) else {
                        return;
                    };
                    if self.tabs[active]
                        .session
                        .as_ref()
                        .is_none_or(|s| s.revision() != revision || !s.editable)
                    {
                        self.status =
                            Some(("The document changed. Select the image again.".into(), true));
                        return;
                    }
                    let previous = self.active;
                    self.active = active;
                    self.content_change(
                        ctx,
                        content::Edit::Image {
                            page: page as u32 + 1,
                            index,
                            edit: mypdf_milestone0::content::ImageEdit::Replace((*png).clone()),
                        },
                    );
                    self.active = previous;
                }
                Target::Signature => {
                    if let Some(asset) = &mut self.editor.signature_image {
                        asset.kind = Kind::Image { png };
                    }
                }
                Target::Annotation { tab, id, revision } => {
                    if let Some(session) = self
                        .tabs
                        .iter_mut()
                        .find(|t| t.id == tab)
                        .and_then(|t| t.session.as_mut())
                    {
                        if session.revision() != revision || !session.editable {
                            self.status = Some((
                                "The document changed. Select the image again.".into(),
                                true,
                            ));
                            return;
                        }
                        let mut entries = session.annotations.clone();
                        if let Some(a) = entries.iter_mut().find(|a| a.id == id) {
                            a.kind = Kind::Image { png };
                            if let Err(error) = session.commit(entries) {
                                self.status = Some((error.to_string(), true));
                            } else {
                                self.editor.draft = None;
                                self.status = Some((
                                    "Background removed · Undo restores the original image".into(),
                                    false,
                                ));
                            }
                        }
                    }
                }
            }
        } else if cancel || modal.should_close() {
            self.editor.background = None;
        }
    }
}
