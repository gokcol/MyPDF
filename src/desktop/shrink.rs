//! Reduce File Size: a choice of three qualities, applied as an edit.
use super::*;
use mypdf_milestone0::compress::Level;

/// A file size in words: 830 KB, 12.4 MB.
pub(super) fn size(bytes: usize) -> String {
    let kb = bytes as f64 / 1024.;
    if kb < 1000. {
        format!("{} KB", kb.round().max(1.))
    } else {
        format!("{:.1} MB", kb / 1024.)
    }
}

impl Viewer {
    pub(super) fn request_shrink(&mut self) {
        let ready = self
            .tabs
            .get(self.active)
            .and_then(|t| t.session.as_ref())
            .is_some_and(|s| s.editable);
        if ready {
            self.shrink = Some(Level::Medium);
        } else {
            self.status = Some((
                "This document cannot be changed here. Signed PDFs keep their size so their signatures stay valid.".into(),
                true,
            ));
        }
    }
    pub(super) fn shrink_dialog(&mut self, ctx: &egui::Context) {
        let Some(level) = &mut self.shrink else {
            return;
        };
        let current = self
            .tabs
            .get(self.active)
            .and_then(|t| t.session.as_ref())
            .map(|s| s.source.len())
            .unwrap_or(0);
        let mut apply = false;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new("reduce_file_size"))
            .backdrop_color(Color32::from_black_alpha(64))
            .frame(ui::modal_frame())
            .show(ctx, |ui| {
                ui.set_width(360.);
                ui.vertical_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(54., 54.), egui::Sense::hover());
                    ui::badge(ui.painter(), r, Icon::Extract, ui::Fill::Brand, Color32::WHITE);
                    ui.add_space(12.);
                    ui::title(ui, "Reduce file size", 16.);
                    ui.label(
                        RichText::new(format!("This PDF is {} now. Pictures and scans get smaller; text and drawings stay sharp.", size(current)))
                            .size(13.)
                            .color(theme::muted()),
                    );
                });
                ui.add_space(12.);
                for choice in Level::ALL {
                    let selected = *level == choice;
                    let response = egui::Frame::new()
                        .fill(if selected { theme::accent_soft() } else { theme::surface() })
                        .stroke(Stroke::new(
                            if selected { 2. } else { 1. },
                            if selected { theme::accent() } else { theme::line() },
                        ))
                        .corner_radius(10)
                        .inner_margin(egui::Margin::symmetric(12, 9))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(RichText::new(choice.label()).font(ui::strong(ui, 13.5)).color(theme::ink()));
                            ui.label(RichText::new(choice.detail()).size(12.).color(theme::muted()));
                        })
                        .response
                        .interact(egui::Sense::click())
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    if response.clicked() {
                        *level = choice;
                    }
                    ui.add_space(4.);
                }
                ui.add_space(8.);
                let half = (ui.available_width() - 10.) / 2.;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.;
                    ui.allocate_ui(vec2(half, 32.), |ui| {
                        cancel = ui::dialog_button(ui, "Cancel", ui::DialogTone::Secondary).clicked();
                    });
                    ui.allocate_ui(vec2(half, 32.), |ui| {
                        apply = ui::dialog_button(ui, "Reduce", ui::DialogTone::Primary).clicked();
                    });
                });
            });
        if modal.should_close() {
            cancel = true;
        }
        if apply {
            let level = *level;
            self.shrink = None;
            self.content_change(ctx, content::Edit::ReduceSize(level));
        } else if cancel {
            self.shrink = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;

    #[test]
    fn reducing_the_file_size_is_an_edit_that_says_how_much() {
        let image = image::RgbImage::from_fn(2000, 1500, |x, y| {
            image::Rgb([(x % 251) as u8, (y % 241) as u8, ((x * y) % 239) as u8])
        });
        let mut png = std::io::Cursor::new(vec![]);
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let blank = mypdf_milestone0::convert::blank(595., 842.).unwrap();
        let photo = mypdf_milestone0::content::add_image(
            &blank,
            1,
            [40., 40., 555., 426.],
            &png.into_inner(),
        )
        .unwrap();
        let before = photo.len();
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.open_memory(PathBuf::from("Photo.pdf"), photo, &ctx);
        let frame = |app: &mut Viewer| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1100., 800.))),
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while app.tabs[0].session.as_ref().is_none_or(|s| !s.editable) {
            assert!(std::time::Instant::now() < deadline);
            frame(&mut app);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.request_shrink();
        assert_eq!(app.shrink, Some(Level::Medium));
        app.shrink = None;
        app.content_change(&ctx, content::Edit::ReduceSize(Level::Small));
        while app.content.busy || app.tabs[0].session.as_ref().unwrap().revision() == 0 {
            assert!(std::time::Instant::now() < deadline, "{:?}", app.status);
            frame(&mut app);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let after = app.tabs[0].session.as_ref().unwrap().source.len();
        assert!(after < before, "{after} {before}");
        let (message, error) = app.status.clone().unwrap();
        assert!(!error && message.starts_with("Reduced from"), "{message}");
        assert_eq!(size(830 * 1024), "830 KB");
        assert_eq!(size(12_400_000), "11.8 MB");
    }
}
