//! My details: name, email, address and so on, kept once on this computer.
//! "Fill with my details" shows what goes where before filling.
use super::*;
use mypdf_milestone0::form_profile::{self, Detail, Profile, Suggestion};

pub(super) struct DetailsState {
    /// Where the details are kept.
    path: PathBuf,
    #[cfg(test)]
    _folder: tempfile::TempDir,
    /// The details being edited.
    pub editor: Option<Profile>,
    /// Fill the form once the details are saved.
    fill_after_save: bool,
    /// Proposed values, each with whether it will be filled.
    pub review: Option<Vec<(Suggestion, bool)>>,
}
impl DetailsState {
    pub fn new() -> Self {
        #[cfg(test)]
        let folder = tempfile::tempdir().unwrap();
        Self {
            #[cfg(test)]
            path: folder.path().join("my_details.json"),
            #[cfg(not(test))]
            path: form_profile::file(),
            #[cfg(test)]
            _folder: folder,
            editor: None,
            fill_after_save: false,
            review: None,
        }
    }
    #[cfg(test)]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Viewer {
    /// Open the details for editing.
    pub(super) fn edit_details(&mut self, fill_after_save: bool) {
        match form_profile::load(&self.details.path) {
            Ok(profile) => {
                self.details.editor = Some(profile);
                self.details.fill_after_save = fill_after_save;
            }
            Err(e) => self.status = Some((format!("{e:#}"), true)),
        }
    }

    /// Propose values for the empty fields that ask for the saved details.
    pub(super) fn fill_with_details(&mut self) {
        self.editor.mode = editing::Mode::Forms;
        let Some(source) = self
            .tabs
            .get(self.active)
            .and_then(|t| t.session.as_ref())
            .filter(|s| s.editable)
            .map(|s| s.source.clone())
        else {
            self.status = Some(("This document cannot be changed right now.".into(), true));
            return;
        };
        let profile = match form_profile::load(&self.details.path) {
            Ok(profile) if profile.is_empty() => {
                self.edit_details(true);
                return;
            }
            Ok(profile) => profile,
            Err(e) => {
                self.status = Some((format!("{e:#}"), true));
                return;
            }
        };
        match form_profile::suggest(&source, &profile, form_profile::today()) {
            Ok(found) if found.is_empty() => {
                self.status = Some((
                    "No empty field in this form asks for your details.".into(),
                    false,
                ))
            }
            Ok(found) => self.details.review = Some(found.into_iter().map(|s| (s, true)).collect()),
            Err(e) => self.status = Some((format!("{e:#}"), true)),
        }
    }

    pub(super) fn details_dialogs(&mut self, ctx: &egui::Context) {
        self.details_review(ctx);
        self.details_editor(ctx);
    }

    fn details_review(&mut self, ctx: &egui::Context) {
        let Some(review) = &mut self.details.review else {
            return;
        };
        let chosen = review.iter().filter(|(_, on)| *on).count();
        let (mut fill, mut cancel, mut edit) = (false, false, false);
        let modal = egui::Modal::new(egui::Id::new("fill_with_details"))
            .backdrop_color(Color32::from_black_alpha(64))
            .frame(ui::modal_frame())
            .show(ctx, |ui| {
                ui.set_width(420.);
                ui.vertical_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(54., 54.), egui::Sense::hover());
                    ui::badge(
                        ui.painter(),
                        r,
                        Icon::Person,
                        ui::Fill::Brand,
                        Color32::WHITE,
                    );
                    ui.add_space(12.);
                    ui::title(ui, "Fill with my details", 16.);
                    ui.label(
                        RichText::new("Check what goes where. Only empty fields are filled.")
                            .size(13.)
                            .color(theme::muted()),
                    );
                });
                ui.add_space(12.);
                egui::ScrollArea::vertical()
                    .max_height(320.)
                    .show(ui, |ui| {
                        for (suggestion, on) in review.iter_mut() {
                            egui::Frame::new()
                                .fill(if *on {
                                    theme::accent_soft()
                                } else {
                                    theme::surface()
                                })
                                .stroke(Stroke::new(1., theme::line()))
                                .corner_radius(9)
                                .inner_margin(egui::Margin::symmetric(10, 6))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal(|ui| {
                                        ui.checkbox(on, "").widget_info(|| {
                                            egui::WidgetInfo::selected(
                                                egui::WidgetType::Checkbox,
                                                true,
                                                *on,
                                                suggestion.name.as_str(),
                                            )
                                        });
                                        ui.vertical(|ui| {
                                            ui.spacing_mut().item_spacing.y = 1.;
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&suggestion.value)
                                                        .font(ui::strong(ui, 13.))
                                                        .color(theme::ink()),
                                                )
                                                .truncate(),
                                            );
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(format!(
                                                        "{}  ·  field “{}”",
                                                        suggestion.detail.label(),
                                                        suggestion.name
                                                    ))
                                                    .size(11.5)
                                                    .color(theme::muted()),
                                                )
                                                .truncate(),
                                            );
                                        });
                                    });
                                });
                            ui.add_space(4.);
                        }
                    });
                ui.add_space(6.);
                if ui
                    .add(ActionButton::new(Icon::Person, "Edit my details…"))
                    .clicked()
                {
                    edit = true;
                }
                ui.add_space(10.);
                let half = (ui.available_width() - 10.) / 2.;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.;
                    ui.allocate_ui(vec2(half, 32.), |ui| {
                        cancel =
                            ui::dialog_button(ui, "Cancel", ui::DialogTone::Secondary).clicked();
                    });
                    ui.allocate_ui(vec2(half, 32.), |ui| {
                        ui.add_enabled_ui(chosen > 0, |ui| {
                            let label = match chosen {
                                1 => "Fill 1 field".to_owned(),
                                n => format!("Fill {n} fields"),
                            };
                            fill = ui::dialog_button(ui, &label, ui::DialogTone::Primary).clicked();
                        });
                    });
                });
            });
        if modal.should_close() {
            cancel = true;
        }
        if fill && chosen > 0 {
            let chosen: Vec<Suggestion> = self
                .details
                .review
                .take()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|(s, on)| on.then_some(s))
                .collect();
            self.content_change(ctx, content::Edit::FillDetails(chosen));
        } else if edit {
            self.details.review = None;
            self.edit_details(true);
        } else if cancel {
            self.details.review = None;
        }
    }

    fn details_editor(&mut self, ctx: &egui::Context) {
        let Some(profile) = &mut self.details.editor else {
            return;
        };
        let then_fill = self.details.fill_after_save;
        let (mut save, mut cancel) = (false, false);
        let modal = egui::Modal::new(egui::Id::new("my_details"))
            .backdrop_color(Color32::from_black_alpha(64))
            .frame(ui::modal_frame())
            .show(ctx, |ui| {
                ui.set_width(460.);
                ui.vertical_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(54., 54.), egui::Sense::hover());
                    ui::badge(ui.painter(), r, Icon::Person, ui::Fill::Brand, Color32::WHITE);
                    ui.add_space(12.);
                    ui::title(ui, "My details", 16.);
                    ui.label(
                        RichText::new("Enter them once and MyPDF fills them into forms when you ask. They stay on this computer.")
                            .size(13.)
                            .color(theme::muted()),
                    );
                });
                ui.add_space(12.);
                let half = (ui.available_width() - 12.) / 2.;
                let rows: [&[Detail]; 7] = [
                    &[Detail::FullName],
                    &[Detail::FirstName, Detail::LastName],
                    &[Detail::Email, Detail::Phone],
                    &[Detail::Company, Detail::JobTitle],
                    &[Detail::Street],
                    &[Detail::City, Detail::PostalCode],
                    &[Detail::Country, Detail::Website],
                ];
                let hint = |detail: Detail, profile: &Profile| match detail {
                    // The name parts follow the full name unless given.
                    Detail::FirstName | Detail::LastName => profile.value(detail),
                    Detail::Email => "name@example.com".into(),
                    _ => String::new(),
                };
                for row in rows {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.;
                        for detail in row {
                            let width = if row.len() == 1 { ui.available_width() } else { half };
                            let hint = hint(*detail, profile);
                            ui.allocate_ui(vec2(width, 52.), |ui| {
                                ui::field(ui, detail.label(), |ui| {
                                    if let Some(value) = profile.field(*detail) {
                                        ui.add(
                                            egui::TextEdit::singleline(value)
                                                .margin(ui::FIELD_MARGIN)
                                                .hint_text(hint)
                                                .desired_width(width),
                                        )
                                        .widget_info(|| {
                                            egui::WidgetInfo::labeled(
                                                egui::WidgetType::TextEdit,
                                                true,
                                                detail.label(),
                                            )
                                        });
                                    }
                                });
                            });
                        }
                    });
                    ui.add_space(4.);
                }
                ui.add_space(12.);
                let half = (ui.available_width() - 10.) / 2.;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.;
                    ui.allocate_ui(vec2(half, 32.), |ui| {
                        cancel = ui::dialog_button(ui, "Cancel", ui::DialogTone::Secondary).clicked();
                    });
                    ui.allocate_ui(vec2(half, 32.), |ui| {
                        let label = if then_fill { "Save and fill" } else { "Save" };
                        save = ui::dialog_button(ui, label, ui::DialogTone::Primary).clicked();
                    });
                });
            });
        if modal.should_close() {
            cancel = true;
        }
        if save {
            let profile = self.details.editor.take().unwrap_or_default();
            match form_profile::save(&self.details.path, &profile) {
                Ok(()) if then_fill && !profile.is_empty() => self.fill_with_details(),
                Ok(()) => self.status = Some(("Your details are saved.".into(), false)),
                Err(e) => {
                    self.status = Some((format!("Your details could not be saved: {e:#}"), true))
                }
            }
            self.details.fill_after_save = false;
        } else if cancel {
            self.details.editor = None;
            self.details.fill_after_save = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;
    use mypdf_milestone0::form_design::{self, FieldType, Properties};

    #[test]
    fn details_are_asked_for_once_then_reviewed_and_filled() {
        let mut pdf = mypdf_milestone0::convert::blank(595., 842.).unwrap();
        for (i, name) in ["Full name", "Email", "Company", "Notes"]
            .iter()
            .enumerate()
        {
            let mut properties = Properties::new(FieldType::Text, &name.replace(' ', ""));
            properties.tooltip = (*name).into();
            let y = 80. + i as f32 * 40.;
            pdf = form_design::add(
                &pdf,
                0,
                [60., y, 320., y + 22.],
                FieldType::Text,
                &properties,
            )
            .unwrap()
            .0;
        }
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.open_memory(PathBuf::from("Form.pdf"), pdf, &ctx);
        let frame = |app: &mut Viewer| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1300., 860.))),
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let wait = |app: &mut Viewer, ready: &dyn Fn(&Viewer) -> bool| {
            while !ready(app) {
                assert!(std::time::Instant::now() < deadline, "{:?}", app.status);
                frame(app);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        wait(&mut app, &|a| {
            a.tabs[0].session.as_ref().is_some_and(|s| s.editable)
        });
        // Nothing saved yet: the details are asked for first.
        app.fill_with_details();
        assert!(app.details.review.is_none());
        let editor = app.details.editor.as_mut().expect("the details open");
        editor.full_name = "Ayşe Yılmaz".into();
        editor.email = "ayse@example.com".into();
        assert!(app.details.fill_after_save);
        // Saving goes straight on to filling.
        let profile = app.details.editor.take().unwrap();
        form_profile::save(app.details.path(), &profile).unwrap();
        app.fill_with_details();
        let review = app
            .details
            .review
            .as_mut()
            .expect("what goes where is shown");
        let names: Vec<&str> = review.iter().map(|(s, _)| s.name.as_str()).collect();
        assert_eq!(names, ["Fullname", "Email"]);
        // Leave the email out.
        review[1].1 = false;
        let chosen: Vec<Suggestion> = review
            .iter()
            .filter(|(_, on)| *on)
            .map(|(s, _)| s.clone())
            .collect();
        app.details.review = None;
        app.content_change(&ctx, content::Edit::FillDetails(chosen));
        wait(&mut app, &|a| !a.content.busy);
        frame(&mut app);
        let source = app.tabs[0].session.as_ref().unwrap().source.clone();
        let values: Vec<(String, String)> = mypdf_milestone0::forms::fields(&source)
            .unwrap()
            .into_iter()
            .map(|f| (f.name, f.value))
            .collect();
        assert!(
            values.contains(&("Fullname".into(), "Ayşe Yılmaz".into())),
            "{values:?}"
        );
        assert!(values.contains(&("Email".into(), String::new())));
        let (message, error) = app.status.clone().unwrap();
        assert!(!error && message.starts_with("Filled 1 field"), "{message}");
        // The details were kept for next time.
        assert_eq!(form_profile::load(app.details.path()).unwrap(), profile);
    }
}
