//! Compact library management; edits are drafts until explicitly saved.
use super::*;
use mypdf_milestone0::annotations::Primitive;

#[derive(Clone)]
pub(super) struct Edit {
    original: Option<String>,
    value: Saved,
    rename_only: bool,
}
impl Edit {
    fn existing(value: &Saved, rename_only: bool) -> Self {
        Self {
            original: Some(value.name.clone()),
            value: value.clone(),
            rename_only,
        }
    }
    fn duplicate(value: &Saved, entries: &[Saved]) -> Self {
        let base: String = value.name.chars().take(65).collect();
        let mut name = format!("{base} copy");
        let mut n = 2;
        while entries.iter().any(|e| e.name == name) {
            name = format!("{base} copy {n}");
            n += 1;
        }
        Self {
            original: None,
            value: Saved {
                name,
                built_in: false,
                ..value.clone()
            },
            rename_only: false,
        }
    }
    fn save(&self, state: &mut State) -> bool {
        if self.value.built_in {
            state.error = Some("Built-in styles stay unchanged. Customize a copy instead.".into());
            return false;
        }
        let mut value = self.value.clone();
        value.name = value.name.trim().to_owned();
        value.purpose = value.purpose.trim().to_owned();
        if state
            .entries
            .iter()
            .any(|e| e.name == value.name && Some(&e.name) != self.original.as_ref())
        {
            state.error = Some("That style name is already in use. Choose another name.".into());
            return false;
        }
        let mut entries = state.entries.clone();
        if let Some(name) = &self.original {
            let Some(existing) = entries.iter_mut().find(|e| !e.built_in && &e.name == name) else {
                state.error =
                    Some("This style was removed. Cancel and create a new style instead.".into());
                return false;
            };
            *existing = value;
        } else {
            entries.push(value);
        }
        state.commit(entries)
    }
}

fn color(rgb: [f32; 3], opacity: f32) -> Color32 {
    let [r, g, b] = rgb.map(|v| (v * 255.).round() as u8);
    Color32::from_rgba_unmultiplied(r, g, b, (opacity * 255.).round() as u8)
}

/// The white tile represents paper, so pale colors remain an honest preview.
pub(super) fn preview(ui: &mut egui::Ui, preset: &Preset, size: egui::Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    painter.rect_filled(rect, 4., Color32::WHITE);
    painter.rect_stroke(
        rect,
        4.,
        Stroke::new(1., Color32::from_gray(210)),
        egui::StrokeKind::Inside,
    );
    match preset {
        Preset::Text {
            font,
            size,
            color: rgb,
        } => {
            let family = text_style::fonts()
                .iter()
                .find(|f| Some(&f.path) == font.as_ref())
                .map(|f| f.ui_name())
                .unwrap_or_else(|| "annotation".into());
            let font = ui::font(
                ui.ctx(),
                &family,
                size.clamp(10., (rect.height() - 8.).min(28.)),
            );
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Aa",
                font,
                color(*rgb, 1.),
            );
        }
        Preset::Drawing(style) => {
            let inner = rect.shrink(7.);
            let mut style = style.clone();
            style.width = style.width.clamp(0.6, 5.);
            let annotation = mypdf_milestone0::annotations::Annotation::new(
                0,
                [inner.left(), inner.top(), inner.right(), inner.bottom()],
                mypdf_milestone0::annotations::Kind::Rectangle,
                style,
            );
            for primitive in mypdf_milestone0::annotations::primitives(&annotation) {
                match primitive {
                    Primitive::Path {
                        mut points,
                        closed,
                        fill,
                    } => {
                        let rgb = if fill {
                            annotation.style.fill.unwrap_or(annotation.style.color)
                        } else {
                            annotation.style.color
                        };
                        let tint = color(rgb, annotation.style.opacity);
                        if fill {
                            let mut mesh = egui::Mesh::default();
                            for triangle in mypdf_milestone0::annotations::triangulate(&points) {
                                let start = mesh.vertices.len() as u32;
                                for p in triangle {
                                    mesh.colored_vertex(pos2(p[0], p[1]), tint);
                                }
                                mesh.add_triangle(start, start + 1, start + 2);
                            }
                            painter.add(egui::Shape::mesh(mesh));
                        } else {
                            if closed && let Some(first) = points.first().copied() {
                                points.push(first);
                            }
                            painter.add(egui::Shape::line(
                                points.into_iter().map(|p| pos2(p[0], p[1])).collect(),
                                Stroke::new(annotation.style.width, tint),
                            ));
                        }
                    }
                    Primitive::Dot { center } => {
                        painter.circle_filled(
                            pos2(center[0], center[1]),
                            annotation.style.width / 2.,
                            color(annotation.style.color, annotation.style.opacity),
                        );
                    }
                    _ => {}
                }
            }
        }
    }
}

pub(super) fn deleted_notice(ui: &mut egui::Ui, state: &mut State) {
    if let Some((_, removed)) = &state.deleted {
        let name = removed.name.clone();
        ui.horizontal_wrapped(|ui| {
            ui.weak(format!("Deleted “{name}”"));
            if ui
                .add(ActionButton::new(Icon::Undo, "Undo delete"))
                .clicked()
            {
                state.undo_delete();
            }
        });
    }
}

fn matches_filter(entry: &Saved, state: &State) -> bool {
    (state.kind == 0 || (state.kind == 1) == matches!(entry.appearance, Preset::Text { .. }))
        && (state.purpose.is_empty() || entry.purpose == state.purpose)
        && (state.search.trim().is_empty()
            || format!(
                "{} {} {}",
                entry.name,
                entry.purpose,
                entry.appearance.kind()
            )
            .to_lowercase()
            .contains(&state.search.trim().to_lowercase()))
}

fn more_button(ui: &mut egui::Ui, name: &str) -> egui::Response {
    let response = ui.add_sized(vec2(32., 30.), egui::Button::new("⋯").frame(false));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ui.is_enabled(),
            format!("More actions for {name}"),
        )
    });
    response.on_hover_text("Rename, duplicate or delete")
}

fn library(ui: &mut egui::Ui, state: &mut State) {
    ui.horizontal(|ui| {
        ui::title(ui, "Style library", 16.);
        ui.weak(format!("{} styles", state.entries.len()));
    });
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.search)
                .hint_text("Search name or purpose…")
                .desired_width((ui.available_width() - 138.).max(60.)),
        );
        let response = ui.add_enabled(
            state.entries.iter().filter(|e| !e.built_in).count() < 100,
            ActionButton::new(Icon::Plus, "New style")
                .primary()
                .dropdown(),
        );
        egui::Popup::from_toggle_button_response(&response).show(|ui| {
            for (label, icon, appearance) in [
                (
                    "New text style",
                    Icon::Text,
                    Preset::Text {
                        font: None,
                        size: 14.,
                        color: [0.12; 3],
                    },
                ),
                (
                    "New drawing style",
                    Icon::Shapes,
                    Preset::Drawing(Style::default()),
                ),
            ] {
                if ui.add(ActionButton::new(icon, label)).clicked() {
                    state.error = None;
                    state.editor = Some(Edit {
                        original: None,
                        value: Saved {
                            name: String::new(),
                            purpose: state.purpose.clone(),
                            appearance,
                            built_in: false,
                        },
                        rename_only: false,
                    });
                    ui.close();
                }
            }
        });
    });
    ui.horizontal_wrapped(|ui| {
        ui.allocate_ui_with_layout(
            vec2(190., 30.),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(190.);
                if let Some(kind) = ui::segmented(ui, &["All", "Text", "Drawing"], state.kind) {
                    state.kind = kind;
                }
            },
        );
        let mut purposes: Vec<_> = state
            .entries
            .iter()
            .filter(|e| !e.purpose.is_empty())
            .map(|e| e.purpose.clone())
            .collect();
        purposes.sort();
        purposes.dedup();
        if !purposes.contains(&state.purpose) {
            state.purpose.clear();
        }
        egui::ComboBox::from_id_salt("style_purpose_filter")
            .width(130.)
            .selected_text(if state.purpose.is_empty() {
                "All purposes"
            } else {
                &state.purpose
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut state.purpose, String::new(), "All purposes");
                for purpose in purposes {
                    ui.selectable_value(&mut state.purpose, purpose.clone(), purpose);
                }
            });
    });
    let mut edit = None;
    let mut remove = None;
    egui::ScrollArea::vertical()
        .id_salt("style_library_list")
        .max_height(290.)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if !state.entries.iter().any(|e| matches_filter(e, state)) {
                ui.weak("No matching styles.");
                if ui.button("Clear filters").clicked() {
                    state.kind = 0;
                    state.purpose.clear();
                    state.search.clear();
                }
            }
            for built_in in [true, false] {
                if !state
                    .entries
                    .iter()
                    .any(|e| e.built_in == built_in && matches_filter(e, state))
                {
                    continue;
                }
                ui::caption(ui, if built_in { "Built-in" } else { "Your styles" });
                for (index, entry) in state
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.built_in == built_in && matches_filter(e, state))
                {
                    ui.push_id(index, |ui| {
                        egui::Frame::new()
                            .inner_margin(6.)
                            .fill(theme::paper())
                            .corner_radius(7.)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    preview(ui, &entry.appearance, vec2(48., 34.));
                                    let text_width = (ui.available_width()
                                        - 64.
                                        - 2. * ui.spacing().item_spacing.x)
                                        .max(50.);
                                    ui.allocate_ui_with_layout(
                                        vec2(text_width, 34.),
                                        egui::Layout::top_down(egui::Align::Min),
                                        |ui| {
                                            ui.set_min_width(text_width);
                                            ui.spacing_mut().item_spacing.y = 1.;
                                            if ui
                                                .add(
                                                    egui::Button::new(&entry.name)
                                                        .frame(false)
                                                        .truncate(),
                                                )
                                                .on_hover_text(&entry.name)
                                                .clicked()
                                            {
                                                edit = Some(Edit::existing(entry, false));
                                            }
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(format!(
                                                        "{} · {}",
                                                        entry.appearance.kind(),
                                                        if entry.purpose.is_empty() {
                                                            "General"
                                                        } else {
                                                            &entry.purpose
                                                        }
                                                    ))
                                                    .size(11.)
                                                    .color(theme::muted()),
                                                )
                                                .truncate(),
                                            );
                                        },
                                    );
                                    let label = if built_in { "Customize" } else { "Edit" };
                                    if ui
                                        .add(
                                            ActionButton::new(
                                                if built_in { Icon::Copy } else { Icon::Pen },
                                                &format!("{label} {}", entry.name),
                                            )
                                            .compact(),
                                        )
                                        .clicked()
                                    {
                                        edit = Some(if built_in {
                                            Edit::duplicate(entry, &state.entries)
                                        } else {
                                            Edit::existing(entry, false)
                                        });
                                    }
                                    let response = more_button(ui, &entry.name);
                                    egui::Popup::from_toggle_button_response(&response).show(
                                        |ui| {
                                            if ui
                                                .add_enabled(
                                                    !built_in,
                                                    ActionButton::new(
                                                        Icon::Pen,
                                                        &format!("Rename {}", entry.name),
                                                    ),
                                                )
                                                .clicked()
                                            {
                                                edit = Some(Edit::existing(entry, true));
                                                ui.close();
                                            }
                                            if ui
                                                .add_enabled(
                                                    state
                                                        .entries
                                                        .iter()
                                                        .filter(|e| !e.built_in)
                                                        .count()
                                                        < 100,
                                                    ActionButton::new(
                                                        Icon::Copy,
                                                        &format!("Duplicate {}", entry.name),
                                                    ),
                                                )
                                                .clicked()
                                            {
                                                edit = Some(Edit::duplicate(entry, &state.entries));
                                                ui.close();
                                            }
                                            ui.separator();
                                            if ui
                                                .add_enabled(
                                                    !built_in,
                                                    ActionButton::new(
                                                        Icon::Trash,
                                                        &format!("Delete {}", entry.name),
                                                    ),
                                                )
                                                .clicked()
                                            {
                                                remove = Some(index);
                                                ui.close();
                                            }
                                        },
                                    );
                                });
                            });
                    });
                }
            }
        });
    if let Some(index) = remove {
        state.remove(index);
    }
    if edit.is_some() {
        state.error = None;
        state.editor = edit;
    }
    deleted_notice(ui, state);
    ui.weak("Apply from Reusable styles in text or drawing properties.");
}

pub(in super::super) fn settings(ui: &mut egui::Ui) {
    // Keep utility controls compact even when the shared Settings window was
    // previously widened for a digital-signature editor.
    ui.set_max_width(ui.available_width().min(620.));
    let shared = state(ui.ctx());
    let mut state = shared.lock().unwrap();
    ui.spacing_mut().item_spacing.y = 8.;
    ui.add_enabled_ui(!state.read_only, |ui| {
        if let Some(mut edit) = state.editor.take() {
            let title = if edit.value.built_in {
                format!(
                    "Built-in {} style",
                    edit.value.appearance.kind().to_lowercase()
                )
            } else if edit.rename_only {
                "Rename style".into()
            } else {
                format!(
                    "{} {} style",
                    if edit.original.is_none() {
                        "New"
                    } else {
                        "Edit"
                    },
                    edit.value.appearance.kind().to_lowercase()
                )
            };
            ui::title(ui, &title, 16.);
            controls::identity(ui, &mut edit.value, &state.entries, edit.rename_only);
            if !edit.rename_only {
                ui.add_enabled_ui(!edit.value.built_in, |ui| {
                    controls::appearance(ui, &mut edit.value.appearance)
                });
            }
            let mut cancel = false;
            let mut save = false;
            let mut duplicate = false;
            ui.horizontal(|ui| {
                if edit.value.built_in {
                    duplicate = ui
                        .add_enabled(
                            state.entries.iter().filter(|e| !e.built_in).count() < 100,
                            ActionButton::new(Icon::Copy, "Customize a copy").primary(),
                        )
                        .clicked();
                } else {
                    save = ui
                        .add_enabled(
                            !edit.value.name.trim().is_empty(),
                            ActionButton::new(Icon::Save, "Save style").primary(),
                        )
                        .clicked();
                }
                cancel = ui
                    .add(ActionButton::new(
                        Icon::Close,
                        if edit.value.built_in {
                            "Back"
                        } else {
                            "Cancel"
                        },
                    ))
                    .clicked();
            });
            ui.weak(if edit.value.built_in {
                "Built-in styles stay available. Make a copy to personalize one."
            } else {
                "Only this preset changes; existing PDF objects stay as they are."
            });
            if duplicate {
                state.editor = Some(Edit::duplicate(&edit.value, &state.entries));
            } else if save && edit.save(&mut state) {
                state.purpose.clear();
                state.search.clear();
                state.kind = 0;
            } else if !cancel {
                state.editor = Some(edit);
            } else {
                state.error = None;
            }
        } else {
            library(ui, &mut state);
        }
    });
    if let Some(error) = &state.error {
        ui::callout(ui, ui::Tone::Danger, error);
    }
}

#[cfg(test)]
pub(in super::super) fn preview_edit(ctx: &egui::Context, text: bool) {
    let shared = state(ctx);
    let mut state = shared.lock().unwrap();
    state.editor = state
        .entries
        .iter()
        .find(|e| matches!(e.appearance, Preset::Text { .. }) == text)
        .map(|e| Edit::duplicate(e, &state.entries));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_ins_are_protected_and_custom_copies_support_edit_rename_delete_undo() {
        let ctx = egui::Context::default();
        let shared = state(&ctx);
        let mut state = shared.lock().unwrap();
        assert!(state.entries.iter().all(|e| e.built_in));
        let builtin = state.entries[0].clone();
        let mut forbidden = Edit::existing(&builtin, true);
        forbidden.value.name = "Changed built-in".into();
        assert!(!forbidden.save(&mut state));
        state.remove(0);
        assert_eq!(state.entries.len(), 2);
        assert_eq!(state.entries[0].name, "Review outline");
        let mut edit = Edit::duplicate(&builtin, &state.entries);
        edit.value.name = "Client review".into();
        edit.value.purpose = "Audits".into();
        let Preset::Drawing(style) = &mut edit.value.appearance else {
            panic!()
        };
        style.width = 4.;
        style.fill = Some([1., 0.9, 0.7]);
        style.fill_pattern = mypdf_milestone0::annotations::FillPattern::Crosshatch;
        assert!(edit.save(&mut state));
        let disk = load(&state.root).unwrap();
        assert_eq!(disk.len(), 3);
        assert!(disk[0].appearance == builtin.appearance);
        assert_eq!(disk[2].purpose, "Audits");
        assert!(
            matches!(&disk[2].appearance, Preset::Drawing(s) if s.width == 4. && s.fill == Some([1., 0.9, 0.7]) && s.fill_pattern == mypdf_milestone0::annotations::FillPattern::Crosshatch)
        );
        let mut rename = Edit::existing(&state.entries[2], true);
        rename.value.name = state.entries[1].name.clone();
        assert!(!rename.save(&mut state));
        rename.value.name = "Audit outline".into();
        assert!(rename.save(&mut state));
        state.remove(2);
        assert_eq!(load(&state.root).unwrap().len(), 2);
        state.undo_delete();
        assert_eq!(load(&state.root).unwrap()[2].name, "Audit outline");
        let duplicate = Edit::duplicate(&state.entries[2], &state.entries);
        assert!(duplicate.save(&mut state));
        let duplicate2 = Edit::duplicate(&state.entries[2], &state.entries);
        assert_eq!(duplicate2.value.name, "Audit outline copy 2");
        let stored: serde_json::Value = preferences::load(&state.root, "styles.json").unwrap();
        assert_eq!(stored["version"], 2);
        assert_eq!(stored["entries"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn failed_save_keeps_the_library_and_draft_available_for_retry() {
        let ctx = egui::Context::default();
        let shared = state(&ctx);
        let mut state = shared.lock().unwrap();
        let edit = Edit::duplicate(&state.entries[0], &state.entries);
        let path = state.root.join("styles.json");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(!edit.save(&mut state));
        assert_eq!(state.entries.len(), 2);
        std::fs::remove_dir(&path).unwrap();
        assert!(edit.save(&mut state));
        assert_eq!(load(&state.root).unwrap()[2].name, "Review outline copy");
    }

    #[test]
    fn search_type_and_purpose_filters_combine_without_changing_styles() {
        let ctx = egui::Context::default();
        let shared = state(&ctx);
        let mut state = shared.lock().unwrap();
        state.search = "  REVIEW  ".into();
        assert!(matches_filter(&state.entries[0], &state));
        assert!(!matches_filter(&state.entries[1], &state));
        state.kind = 1;
        assert!(!state.entries.iter().any(|e| matches_filter(e, &state)));
        state.search.clear();
        state.purpose = "Headings".into();
        assert!(matches_filter(&state.entries[1], &state));
        state.kind = 2;
        assert!(!state.entries.iter().any(|e| matches_filter(e, &state)));
        assert_eq!(load(&state.root).unwrap().len(), 2);
    }

    #[test]
    fn compact_manager_actions_are_clickable_and_editor_cancel_keeps_saved_style() {
        for width in [420., 640.] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let frame = |events| {
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(width, 600.))),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, settings);
                    },
                )
            };
            let click = |label: &str| {
                for _ in 0..4 {
                    frame(vec![]);
                }
                let out = frame(vec![]);
                let node = out
                    .platform_output
                    .accesskit_update
                    .unwrap()
                    .nodes
                    .into_iter()
                    .find(|(_, n)| n.label() == Some(label))
                    .unwrap_or_else(|| panic!("missing {label}"));
                let r = node.1.bounds().unwrap();
                assert!(
                    r.x0 >= 0. && r.x1 <= f64::from(width),
                    "{label} clipped: {r:?}"
                );
                let pos = pos2(((r.x0 + r.x1) / 2.) as f32, ((r.y0 + r.y1) / 2.) as f32);
                for pressed in [true, false] {
                    frame(vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]);
                }
            };
            click("Review outline");
            click("Customize a copy");
            click("Save style");
            click("More actions for Review outline copy");
            click("Rename Review outline copy");
            let shared = state(&ctx);
            assert!(shared.lock().unwrap().editor.as_ref().unwrap().rename_only);
            shared.lock().unwrap().editor.as_mut().unwrap().value.name = "Renamed in UI".into();
            click("Save style");
            assert_eq!(
                load(&shared.lock().unwrap().root).unwrap()[2].name,
                "Renamed in UI"
            );
            click("Edit Renamed in UI");
            assert!(!shared.lock().unwrap().editor.as_ref().unwrap().rename_only);
            shared
                .lock()
                .unwrap()
                .editor
                .as_mut()
                .unwrap()
                .value
                .purpose = "Discard this draft".into();
            click("Cancel");
            assert_eq!(shared.lock().unwrap().entries[2].purpose, "Review");
            click("More actions for Renamed in UI");
            click("Delete Renamed in UI");
            assert_eq!(shared.lock().unwrap().entries.len(), 2);
            click("Undo delete");
            assert_eq!(shared.lock().unwrap().entries.len(), 3);
            click("New style");
            click("New text style");
            assert!(matches!(
                shared
                    .lock()
                    .unwrap()
                    .editor
                    .as_ref()
                    .unwrap()
                    .value
                    .appearance,
                Preset::Text { .. }
            ));
            click("Cancel");
        }
    }
}
