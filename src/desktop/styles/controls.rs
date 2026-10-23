//! Aligned, compact style controls shared by the two preset editors.
use super::*;
use mypdf_milestone0::annotations::{Primitive, StrokeStyle};

fn row<R>(ui: &mut egui::Ui, label: &str, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.add_sized(
            vec2(82., 28.),
            egui::Label::new(RichText::new(label).size(12.).color(theme::muted())),
        );
        body(ui)
    })
    .inner
}

pub(super) fn identity(ui: &mut egui::Ui, entry: &mut Saved, entries: &[Saved], rename_only: bool) {
    egui::Frame::new()
        .fill(theme::paper())
        .corner_radius(8.)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    manager::preview(ui, &entry.appearance, vec2(68., 56.));
                    ui.add(egui::Label::new(
                        RichText::new("Preview").size(10.5).color(theme::muted()),
                    ));
                });
                ui.add_space(4.);
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    ui.add_enabled_ui(!entry.built_in, |ui| {
                        ui::field(ui, "Name", |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut entry.name)
                                    .hint_text("Style name")
                                    .char_limit(80)
                                    .desired_width(ui.available_width()),
                            );
                        });
                        if !rename_only {
                            ui::field(ui, "Purpose", |ui| {
                                ui.horizontal(|ui| {
                                    ui.add_sized(
                                        vec2(
                                            (ui.available_width()
                                                - 26.
                                                - ui.spacing().item_spacing.x)
                                                .max(60.),
                                            24.,
                                        ),
                                        egui::TextEdit::singleline(&mut entry.purpose)
                                            .hint_text("e.g. Review, Notes, Headings")
                                            .char_limit(60)
                                            .desired_width(f32::INFINITY),
                                    );
                                    let response =
                                        ui.add_sized(vec2(26., 24.), egui::Button::new("⌄"));
                                    response.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::WidgetType::Button,
                                            ui.is_enabled(),
                                            "Purpose suggestions",
                                        )
                                    });
                                    egui::Popup::from_toggle_button_response(&response).show(
                                        |ui| {
                                            let mut purposes = vec![
                                                "Review".to_owned(),
                                                "Headings".into(),
                                                "Notes".into(),
                                                "Emphasis".into(),
                                                "Diagrams".into(),
                                            ];
                                            purposes.extend(
                                                entries
                                                    .iter()
                                                    .filter(|e| !e.purpose.is_empty())
                                                    .map(|e| e.purpose.clone()),
                                            );
                                            purposes.sort();
                                            purposes.dedup();
                                            for purpose in purposes {
                                                if ui
                                                    .selectable_value(
                                                        &mut entry.purpose,
                                                        purpose.clone(),
                                                        purpose,
                                                    )
                                                    .clicked()
                                                {
                                                    ui.close();
                                                }
                                            }
                                        },
                                    );
                                });
                            });
                        }
                    });
                });
            });
        });
}

/// Keep a visible track and a small handle, with the value aligned on the right.
/// Using native sliders/DragValues retains keyboard and accessibility support.
fn number(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
    logarithmic: bool,
) -> bool {
    let mut changed = false;
    ui.push_id(label, |ui| {
        row(ui, label, |ui| {
            ui.scope(|ui| {
                ui.spacing_mut().slider_width = (ui.available_width() - 80.).max(60.);
                ui.spacing_mut().interact_size.y = 18.;
                let response = ui::numeric_slider(
                    ui,
                    egui::Slider::new(value, range.clone())
                        .logarithmic(logarithmic)
                        .show_value(false)
                        .trailing_fill(true),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Slider, ui.is_enabled(), label)
                });
                changed |= response.changed();
            });
            let response = ui.add_sized(vec2(72., 26.), |ui: &mut egui::Ui| {
                ui::numeric_value(
                    ui,
                    egui::DragValue::new(value)
                        .range(range)
                        .speed(0.1)
                        .max_decimals(1)
                        .suffix(suffix),
                )
            });
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::DragValue, ui.is_enabled(), label)
            });
            changed |= response.changed();
        })
    });
    changed
}

fn patterns(ui: &mut egui::Ui, current: &mut StrokeStyle) {
    let width = ((ui.available_width() - ui.spacing().item_spacing.x * 4.) / 5.).max(30.);
    for pattern in StrokeStyle::ALL {
        let (rect, response) = ui.allocate_exact_size(vec2(width, 32.), egui::Sense::click());
        let selected = *current == pattern;
        let color = if selected {
            theme::accent()
        } else {
            theme::ink()
        };
        ui.painter().rect_filled(
            rect,
            5.,
            if selected {
                theme::accent_soft()
            } else {
                theme::control()
            },
        );
        if selected {
            ui.painter().rect_stroke(
                rect,
                5.,
                Stroke::new(1., theme::accent_line()),
                egui::StrokeKind::Inside,
            );
        }
        for mark in mypdf_milestone0::stroke::path(
            vec![
                [rect.left() + 7., rect.center().y],
                [rect.right() - 7., rect.center().y],
            ],
            false,
            1.5,
            pattern,
        ) {
            match mark {
                Primitive::Path { points, .. } => {
                    ui.painter().add(egui::Shape::line(
                        points.into_iter().map(|p| pos2(p[0], p[1])).collect(),
                        Stroke::new(1.5, color),
                    ));
                }
                Primitive::Dot { center } => {
                    ui.painter()
                        .circle_filled(pos2(center[0], center[1]), 0.8, color);
                }
                _ => {}
            }
        }
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Button,
                ui.is_enabled(),
                selected,
                format!("{} stroke style", pattern.label()),
            )
        });
        if response.on_hover_text(pattern.label()).clicked() {
            *current = pattern;
        }
    }
}

pub(super) fn appearance(ui: &mut egui::Ui, preset: &mut Preset) {
    egui::Frame::new()
        .fill(theme::paper())
        .corner_radius(8.)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 5.;
            match preset {
                Preset::Text { font, size, color } => {
                    row(ui, "Font", |ui| {
                        let fonts = text_style::fonts();
                        let label = fonts
                            .iter()
                            .find(|f| Some(&f.path) == font.as_ref())
                            .map(|f| format!("{} {}", f.family, f.style()))
                            .unwrap_or_else(|| {
                                if font.is_some() {
                                    "Unavailable font (kept)"
                                } else {
                                    "Default font"
                                }
                                .into()
                            });
                        egui::ComboBox::from_id_salt("preset_font")
                            .width(ui.available_width())
                            .selected_text(label)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(font, None, "Default font");
                                for face in fonts {
                                    ui.selectable_value(
                                        font,
                                        Some(face.path.clone()),
                                        format!("{} {}", face.family, face.style()),
                                    );
                                }
                            });
                    });
                    number(ui, "Font size", size, 1. ..=500., " pt", true);
                    row(ui, "Color", |ui| {
                        editing::color_palette(ui, color);
                    });
                }
                Preset::Drawing(style) => {
                    row(ui, "Border style", |ui| patterns(ui, &mut style.stroke));
                    row(ui, "Line color", |ui| {
                        editing::color_palette(ui, &mut style.color);
                    });
                    number(ui, "Line width", &mut style.width, 0.1..=30., " pt", false);
                    let mut opacity = style.opacity * 100.;
                    if number(ui, "Opacity", &mut opacity, 0. ..=100., "%", false) {
                        style.opacity = opacity / 100.;
                    }
                    row(ui, "Fill", |ui| {
                        if ui.selectable_label(style.fill.is_none(), "None").clicked() {
                            style.fill = None;
                        }
                        if ui.selectable_label(style.fill.is_some(), "Color").clicked()
                            && style.fill.is_none()
                        {
                            style.fill = Some(style.color);
                        }
                        if let Some(fill) = &mut style.fill {
                            ui::color_swatch(ui, fill, "Fill color");
                        }
                        if style.fill.is_some() {
                            egui::ComboBox::from_id_salt("saved_style_fill_pattern")
                                .selected_text(style.fill_pattern.label())
                                .width(100.)
                                .show_ui(ui, |ui| {
                                    for pattern in mypdf_milestone0::annotations::FillPattern::ALL {
                                        ui.selectable_value(
                                            &mut style.fill_pattern,
                                            pattern,
                                            pattern.label(),
                                        );
                                    }
                                });
                        }
                    });
                    egui::CollapsingHeader::new("More options").show(ui, |ui| {
                        number(
                            ui,
                            "Note text",
                            &mut style.font_size,
                            1. ..=144.,
                            " pt",
                            true,
                        );
                        number(
                            ui,
                            "Image blur",
                            &mut style.image_blur,
                            0. ..=32.,
                            "",
                            false,
                        );
                        row(ui, "Image", |ui| {
                            ui.checkbox(&mut style.image_border, "Show border");
                        });
                    });
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn property_cards_fit_the_available_width_without_growing_the_window() {
        for width in [360., 420., 640.] {
            for mut entry in samples() {
                entry.built_in = false;
                let ctx = egui::Context::default();
                for _ in 0..12 {
                    let _ = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(width, 720.))),
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                let right = ui.max_rect().right();
                                identity(ui, &mut entry, &[], false);
                                assert!(
                                    ui.min_rect().right() <= right + 0.5,
                                    "identity overflow at {width}: {} > {right}",
                                    ui.min_rect().right()
                                );
                                appearance(ui, &mut entry.appearance);
                                assert!(
                                    ui.min_rect().right() <= right + 0.5,
                                    "appearance overflow at {width}: {} > {right}",
                                    ui.min_rect().right()
                                );
                            });
                        },
                    );
                }
            }
        }
    }
}
