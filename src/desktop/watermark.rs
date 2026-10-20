//! The Watermark dialog: text or a picture, its look, angle and pages, with a
//! preview of the page; and removing watermarks again.
use super::*;
use mypdf_milestone0::watermark::{Mark, PRESETS, Watermark};

/// Colours that suit watermarks.
const COLORS: [([f32; 3], &str); 4] = [
    ([0.75, 0.12, 0.12], "Red"),
    ([0.45, 0.47, 0.5], "Gray"),
    ([0.12, 0.3, 0.7], "Blue"),
    ([0.08, 0.08, 0.1], "Black"),
];

pub(super) struct Draft {
    text_mode: bool,
    text: String,
    color: [f32; 3],
    fit: bool,
    size: f32,
    /// A chosen picture: its name, bytes and preview.
    picture: Option<(String, Vec<u8>, Option<TextureHandle>, f32)>,
    width: f32,
    opacity: f32,
    angle: f32,
    behind: bool,
    all_pages: bool,
    from: usize,
    to: usize,
    preview: usize,
    /// Pages that already carry a MyPDF watermark.
    existing: usize,
    error: Option<String>,
}
impl Draft {
    fn watermark(&self) -> Result<Watermark, String> {
        let mark = if self.text_mode {
            Mark::Text {
                text: self.text.trim().to_owned(),
                color: self.color,
                font: None,
            }
        } else {
            let (_, bytes, _, _) = self.picture.as_ref().ok_or("Choose a picture first")?;
            Mark::Image {
                bytes: bytes.clone(),
                width: self.width,
            }
        };
        Ok(Watermark {
            mark,
            size: if self.fit { 0. } else { self.size },
            opacity: self.opacity,
            angle: self.angle,
            behind: self.behind,
            pages: (!self.all_pages)
                .then(|| (self.from.saturating_sub(1), self.to.saturating_sub(1))),
        })
    }
}

impl Viewer {
    pub(super) fn open_watermark(&mut self) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let Some(session) = tab.session.as_ref().filter(|s| s.editable) else {
            self.status = Some(("This document cannot be changed here.".into(), true));
            return;
        };
        let existing = mypdf_milestone0::watermark::count(&session.source).unwrap_or(0);
        self.watermark = Some(Draft {
            text_mode: true,
            text: PRESETS[0].into(),
            color: COLORS[0].0,
            fit: true,
            size: 72.,
            picture: None,
            width: 0.5,
            opacity: 0.25,
            angle: 45.,
            behind: false,
            all_pages: true,
            from: 1,
            to: tab.pages.len().max(1),
            preview: tab.page,
            existing,
            error: None,
        });
    }

    pub(super) fn watermark_dialog(&mut self, ctx: &egui::Context) {
        let Some(draft) = &mut self.watermark else {
            return;
        };
        let Some(tab) = self.tabs.get_mut(self.active) else {
            self.watermark = None;
            return;
        };
        let count = tab.pages.len().max(1);
        let mut apply = false;
        let mut remove = false;
        let mut cancel = false;
        let mut choose = false;
        let mut focus = false;
        let modal = egui::Modal::new(egui::Id::new("watermark_dialog"))
            .backdrop_color(Color32::from_black_alpha(64))
            .frame(ui::modal_frame())
            .show(ctx, |ui| {
                ui.set_width(620.);
                // Slider tracks stand out from the dialog's own background.
                ui.visuals_mut().widgets.inactive.bg_fill = theme::track();
                ui::title(ui, "Watermark", 17.);
                ui.label(
                    RichText::new("Text or a picture across the middle of each page, see-through.")
                        .size(12.5)
                        .color(theme::muted()),
                );
                ui.add_space(8.);
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(250.);
                        preview(ui, tab, draft);
                        ui.horizontal(|ui| {
                            if ui.add_enabled(draft.preview > 0, ActionButton::new(Icon::Left, "Previous page").compact()).clicked() {
                                draft.preview -= 1;
                            }
                            ui.label(RichText::new(format!("Page {} of {count}", draft.preview + 1)).size(11.5).color(theme::muted()));
                            if ui.add_enabled(draft.preview + 1 < count, ActionButton::new(Icon::Right, "Next page").compact()).clicked() {
                                draft.preview += 1;
                            }
                        });
                    });
                    ui.add_space(12.);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 9.;
                        let current = if draft.text_mode { 0 } else { 1 };
                        if let Some(i) = ui::segmented(ui, &["Text", "Picture"], current) {
                            draft.text_mode = i == 0;
                        }
                        if draft.text_mode {
                            focus |= ui
                                .add(
                                    egui::TextEdit::singleline(&mut draft.text)
                                        .margin(ui::FIELD_MARGIN)
                                        .desired_width(f32::INFINITY)
                                        .char_limit(200),
                                )
                                .has_focus();
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = vec2(6., 6.);
                                for preset in PRESETS {
                                    if ui.selectable_label(draft.text == preset, preset).clicked() {
                                        draft.text = preset.into();
                                    }
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Color").color(theme::muted()));
                                for (color, name) in COLORS {
                                    let (r, response) = ui.allocate_exact_size(vec2(22., 22.), egui::Sense::click());
                                    let fill = Color32::from_rgb((color[0] * 255.) as u8, (color[1] * 255.) as u8, (color[2] * 255.) as u8);
                                    ui.painter().circle_filled(r.center(), 9., fill);
                                    if draft.color == color {
                                        ui.painter().circle_stroke(r.center(), 11., Stroke::new(2., theme::accent()));
                                    }
                                    if response.on_hover_text(name).clicked() {
                                        draft.color = color;
                                    }
                                }
                                ui::color_swatch_sized(ui, &mut draft.color, "Custom color", vec2(22., 22.));
                            });
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut draft.fit, "Fit across the page");
                                if !draft.fit {
                                    ui.add(egui::DragValue::new(&mut draft.size).range(6.0..=300.0).suffix(" pt"));
                                }
                            });
                        } else {
                            ui.horizontal(|ui| {
                                choose = ui.add(ActionButton::new(Icon::Import, "Choose picture…")).clicked();
                                if let Some((name, ..)) = &draft.picture {
                                    ui.label(RichText::new(name).size(12.).color(theme::muted()));
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Width").color(theme::muted()));
                                let mut percent = draft.width * 100.;
                                if ui.add(egui::Slider::new(&mut percent, 10.0..=100.0).suffix(" % of the page")).changed() {
                                    draft.width = percent / 100.;
                                }
                            });
                        }
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Opacity").color(theme::muted()));
                            let mut percent = draft.opacity * 100.;
                            if ui.add(egui::Slider::new(&mut percent, 5.0..=100.0).suffix(" %")).changed() {
                                draft.opacity = percent / 100.;
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Angle").color(theme::muted()));
                            let angles = [45., 0., 90., -45.];
                            let current = angles.iter().position(|a| *a == draft.angle).unwrap_or(usize::MAX);
                            if let Some(i) = ui::segmented(ui, &["45°", "0°", "90°", "−45°"], current) {
                                draft.angle = angles[i];
                            }
                            ui.add(egui::DragValue::new(&mut draft.angle).range(-180.0..=180.0).suffix("°"));
                        });
                        let current = if draft.behind { 1 } else { 0 };
                        if let Some(i) = ui::segmented(ui, &["In front of the page", "Behind the page"], current) {
                            draft.behind = i == 1;
                        }
                        if draft.behind {
                            ui.label(
                                RichText::new("Behind keeps the page's text crisp, but scans and full-page pictures hide it.")
                                    .size(11.5)
                                    .color(theme::muted()),
                            );
                        }
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut draft.all_pages, "All pages");
                            if !draft.all_pages {
                                ui.label(RichText::new("Pages").color(theme::muted()));
                                ui.add(egui::DragValue::new(&mut draft.from).range(1..=count));
                                ui.label("to");
                                ui.add(egui::DragValue::new(&mut draft.to).range(1..=count));
                                draft.to = draft.to.max(draft.from);
                            }
                        });
                        if let Some(e) = &draft.error {
                            ui::callout(ui, ui::Tone::Danger, e);
                        }
                    });
                });
                ui.add_space(12.);
                ui.horizontal(|ui| {
                    if draft.existing > 0 {
                        remove = ui
                            .add(ActionButton::new(Icon::Trash, "Remove watermarks").tint(theme::danger()))
                            .on_hover_text(format!(
                                "Remove the watermarks MyPDF added on {} page{}",
                                draft.existing,
                                if draft.existing == 1 { "" } else { "s" }
                            ))
                            .clicked();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.allocate_ui(vec2(120., 32.), |ui| {
                            apply = ui::dialog_button(ui, "Apply", ui::DialogTone::Primary).clicked();
                        });
                        ui.allocate_ui(vec2(120., 32.), |ui| {
                            cancel = ui::dialog_button(ui, "Cancel", ui::DialogTone::Secondary).clicked();
                        });
                    });
                });
            });
        self.editor.text_editing |= focus;
        if modal.should_close() {
            cancel = true;
        }
        if choose
            && let Some(path) = self.native_pick(
                rfd::FileDialog::new()
                    .set_title("Choose a watermark picture")
                    .add_filter("Pictures", &["png", "jpg", "jpeg"]),
            )
        {
            let picked = mypdf_milestone0::convert::read_bounded(&path, 32 * 1024 * 1024).and_then(
                |bytes| {
                    let image = mypdf_milestone0::content::decode_image(&bytes)?;
                    Ok((bytes, image))
                },
            );
            let draft = self.watermark.as_mut().unwrap();
            match picked {
                Ok((bytes, image)) => {
                    let aspect = image.width() as f32 / image.height().max(1) as f32;
                    let small =
                        image::imageops::thumbnail(&image, 256, (256. / aspect).max(1.) as u32);
                    let texture = ctx.load_texture(
                        "watermark_picture",
                        egui::ColorImage::from_rgba_unmultiplied(
                            [small.width() as usize, small.height() as usize],
                            &small,
                        ),
                        egui::TextureOptions::LINEAR,
                    );
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    draft.picture = Some((name, bytes, Some(texture), aspect));
                    draft.error = None;
                }
                Err(e) => draft.error = Some(format!("{e:#}")),
            }
        }
        if remove {
            self.watermark = None;
            self.content_change(ctx, content::Edit::RemoveWatermarks);
        } else if apply {
            let draft = self.watermark.as_mut().unwrap();
            match draft.watermark() {
                Ok(watermark) => {
                    self.watermark = None;
                    self.content_change(ctx, content::Edit::Watermark(watermark));
                }
                Err(e) => draft.error = Some(e),
            }
        } else if cancel {
            self.watermark = None;
        }
    }
}

/// The page, small, with the watermark drawn over it as it will look.
fn preview(ui: &mut egui::Ui, tab: &mut Tab, draft: &Draft) {
    let page = draft.preview.min(tab.pages.len().saturating_sub(1));
    let Some(size) = tab.pages.get(page).copied() else {
        return;
    };
    let room = vec2(250., 300.);
    let factor = (room.x / size.width).min(room.y / size.height);
    let (frame, _) = ui.allocate_exact_size(room, egui::Sense::hover());
    let rect = Rect::from_center_size(frame.center(), vec2(size.width, size.height) * factor);
    let painter = ui.painter_at(frame);
    ui::shadow(&painter, rect, 4, 1);
    painter.rect_filled(rect, 2., Color32::WHITE);
    let key = viewer::raster_key(page, size, factor * ui.ctx().pixels_per_point(), true);
    tab.request(key);
    if let Some(raster) = tab.rasters.get(&key) {
        painter.image(
            raster.texture.id(),
            rect,
            Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)),
            Color32::WHITE,
        );
    }
    let shown = draft.all_pages || (draft.from..=draft.to).contains(&(page + 1));
    if !shown {
        return;
    }
    let alpha = (draft.opacity * 255.) as u8;
    // egui turns clockwise; the watermark angle is counterclockwise.
    let rot = egui::emath::Rot2::from_angle(-draft.angle.to_radians());
    let painter = painter.with_clip_rect(rect);
    if draft.text_mode {
        if draft.text.trim().is_empty() {
            return;
        }
        let mut galley = painter.layout_no_wrap(
            draft.text.trim().to_owned(),
            FontId::proportional(100.),
            Color32::WHITE,
        );
        let size_pt = if draft.fit {
            let (sin, cos) = draft.angle.to_radians().sin_cos();
            let reach = match (cos.abs() < 1e-3, sin.abs() < 1e-3) {
                (true, _) => size.height,
                (_, true) => size.width,
                _ => (size.width / cos.abs()).min(size.height / sin.abs()),
            };
            let per_point = galley.size().x / 100.;
            (0.8 * reach / per_point.max(1e-3))
                .min(0.3 * size.width.min(size.height))
                .clamp(6., 300.)
        } else {
            draft.size
        };
        let color = Color32::from_rgba_unmultiplied(
            (draft.color[0] * 255.) as u8,
            (draft.color[1] * 255.) as u8,
            (draft.color[2] * 255.) as u8,
            alpha,
        );
        galley = painter.layout_no_wrap(
            draft.text.trim().to_owned(),
            FontId::proportional((size_pt * factor).max(1.)),
            color,
        );
        let half = galley.size() / 2.;
        let pos = rect.center() - rot * half;
        painter.add(egui::epaint::TextShape::new(pos, galley, color).with_angle(rot.angle()));
    } else if let Some((_, _, Some(texture), aspect)) = &draft.picture {
        let w = rect.width() * draft.width;
        let h = w / aspect;
        let mut mesh = egui::Mesh::with_texture(texture.id());
        mesh.add_rect_with_uv(
            Rect::from_center_size(rect.center(), vec2(w, h)),
            Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)),
            Color32::from_white_alpha(alpha),
        );
        mesh.rotate(rot, rect.center());
        painter.add(mesh);
    }
}

#[cfg(test)]
impl Viewer {
    /// Visual QA: the Watermark dialog over the first page.
    pub(super) fn preview_watermark(&mut self) {
        self.open_watermark();
        if let Some(draft) = &mut self.watermark {
            draft.text = "DRAFT".into();
            draft.existing = 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;

    #[test]
    fn the_dialog_adds_a_watermark_and_removes_it() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let font = mypdf_milestone0::document::default_font().unwrap();
        app.open_memory(
            PathBuf::from("Report.pdf"),
            mypdf_milestone0::document::fixture(&font).unwrap(),
            &ctx,
        );
        let frame = |app: &mut Viewer| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 900.))),
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let until = |app: &mut Viewer, ready: &dyn Fn(&Viewer) -> bool| {
            while !ready(app) {
                assert!(std::time::Instant::now() < deadline, "{:?}", app.status);
                frame(app);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        until(&mut app, &|a| {
            a.tabs[0].session.as_ref().is_some_and(|s| s.editable)
        });
        app.open_watermark();
        frame(&mut app);
        let draft = app.watermark.as_mut().expect("the dialog is open");
        assert_eq!(draft.existing, 0);
        // Only the second page, as a picture-free DRAFT at 30 %.
        draft.text = "DRAFT".into();
        draft.opacity = 0.3;
        draft.all_pages = false;
        draft.from = 2;
        draft.to = 2;
        let watermark = draft.watermark().unwrap();
        assert_eq!(watermark.pages, Some((1, 1)));
        app.watermark = None;
        app.content_change(&ctx, content::Edit::Watermark(watermark));
        until(&mut app, &|a| {
            !a.content.busy && a.tabs[0].session.as_ref().unwrap().revision() > 0
        });
        assert!(
            app.status
                .as_ref()
                .is_some_and(|(m, e)| !e && m.starts_with("Watermark added to 1 page")),
            "{:?}",
            app.status
        );
        let source = app.tabs[0].session.as_ref().unwrap().source.clone();
        assert_eq!(mypdf_milestone0::watermark::count(&source).unwrap(), 1);
        // Opening the dialog again offers to remove it.
        until(&mut app, &|a| {
            a.tabs[0].session.as_ref().is_some_and(|s| s.editable)
        });
        app.open_watermark();
        assert_eq!(app.watermark.as_ref().unwrap().existing, 1);
        app.watermark = None;
        app.content_change(&ctx, content::Edit::RemoveWatermarks);
        until(&mut app, &|a| {
            !a.content.busy && a.tabs[0].session.as_ref().unwrap().revision() > 1
        });
        let source = app.tabs[0].session.as_ref().unwrap().source.clone();
        assert_eq!(mypdf_milestone0::watermark::count(&source).unwrap(), 0);
        // A picture is needed before a picture watermark.
        until(&mut app, &|a| {
            a.tabs[0].session.as_ref().is_some_and(|s| s.editable)
        });
        app.open_watermark();
        let draft = app.watermark.as_mut().unwrap();
        draft.text_mode = false;
        assert!(draft.watermark().is_err());
    }
}
