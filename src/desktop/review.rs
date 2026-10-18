//! Review threads stay with the annotation and share document Undo and safe saving.
use super::*;
use mypdf_milestone0::{
    annotations::Annotation,
    review::{Reply, timestamp},
};
#[derive(Default)]
pub(super) struct State {
    pub open: bool,
    pub selected: Option<String>,
    author: String,
    filter: String,
    status: u8,
    color: Option<[f32; 3]>,
    reply: String,
    error: Option<String>,
}
impl Viewer {
    pub(super) fn review_window(&mut self, ctx: &egui::Context) {
        if !self.review.open {
            return;
        }
        let Some(t) = self.tabs.get(self.active) else {
            self.review.open = false;
            return;
        };
        let Some(s) = &t.session else { return };
        let id = t.id;
        let marks = s.annotations.clone();
        let editable = s.editable && !self.content.busy && !self.organizer.busy;
        let filtered: Vec<Annotation> = marks
            .iter()
            .filter(|a| {
                let r = &a.object.review;
                (self.review.status == 0
                    || (self.review.status == 1 && !r.resolved)
                    || (self.review.status == 2 && r.resolved))
                    && (self.review.filter.is_empty()
                        || r.author
                            .to_lowercase()
                            .contains(&self.review.filter.to_lowercase())
                        || r.replies.iter().any(|r| {
                            r.author
                                .to_lowercase()
                                .contains(&self.review.filter.to_lowercase())
                        }))
                    && self.review.color.is_none_or(|c| c == a.style.color)
            })
            .cloned()
            .collect();
        let mut open = true;
        let mut change = None;
        let mut jump = None;
        let mut export = false;
        egui::Window::new("Review comments")
            .open(&mut open)
            .default_size(vec2(700., 480.))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Author");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.review.filter)
                            .hint_text("All authors")
                            .desired_width(130.),
                    );
                    egui::ComboBox::from_id_salt("review_status")
                        .selected_text(["All", "Open", "Resolved"][self.review.status as usize])
                        .show_ui(ui, |ui| {
                            for (i, label) in ["All", "Open", "Resolved"].iter().enumerate() {
                                ui.selectable_value(&mut self.review.status, i as u8, *label);
                            }
                        });
                    egui::ComboBox::from_id_salt("review_color")
                        .selected_text(if self.review.color.is_some() {
                            "Selected color"
                        } else {
                            "All colors"
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.review.color, None, "All colors");
                            let mut colors = vec![];
                            for a in &marks {
                                if !colors.contains(&a.style.color) {
                                    colors.push(a.style.color);
                                }
                            }
                            for c in colors {
                                if ui
                                    .colored_label(
                                        Color32::from_rgb(
                                            (c[0] * 255.) as u8,
                                            (c[1] * 255.) as u8,
                                            (c[2] * 255.) as u8,
                                        ),
                                        "● Select color",
                                    )
                                    .interact(egui::Sense::click())
                                    .clicked()
                                {
                                    self.review.color = Some(c);
                                    ui.close();
                                }
                            }
                        });
                    export = ui
                        .add(ActionButton::new(Icon::Export, "Export summary"))
                        .clicked();
                });
                ui.separator();
                ui.columns(2, |cols| {
                    egui::ScrollArea::vertical()
                        .id_salt("review_list")
                        .max_height(440.)
                        .show(&mut cols[0], |ui| {
                            if filtered.is_empty() {
                                ui.weak("No annotations match these filters.");
                            }
                            for a in &filtered {
                                let title = a
                                    .kind
                                    .text()
                                    .filter(|s| !s.is_empty())
                                    .unwrap_or(a.kind.label());
                                let title: String = title.chars().take(65).collect();
                                let label = format!(
                                    "{}  Page {} · {}",
                                    if a.object.review.resolved {
                                        "✓"
                                    } else {
                                        "○"
                                    },
                                    a.page + 1,
                                    title
                                );
                                if ui
                                    .selectable_label(
                                        self.review.selected.as_ref() == Some(&a.id),
                                        label,
                                    )
                                    .clicked()
                                {
                                    self.review.selected = Some(a.id.clone());
                                    self.review.reply.clear();
                                    jump = Some(a.page);
                                }
                            }
                        });
                    let ui = &mut cols[1];
                    if let Some(a) = marks
                        .iter()
                        .find(|a| Some(&a.id) == self.review.selected.as_ref())
                    {
                        let mut a = a.clone();
                        ui::title(ui, a.kind.label(), 17.);
                        if let Some(text) = a.kind.text() {
                            ui.label(text);
                        }
                        ui.weak(format!(
                            "Page {} · {}",
                            a.page + 1,
                            if a.object.review.author.is_empty() {
                                "No author"
                            } else {
                                &a.object.review.author
                            }
                        ));
                        egui::ScrollArea::vertical()
                            .id_salt("review_replies")
                            .max_height(230.)
                            .show(ui, |ui| {
                                for reply in &a.object.review.replies {
                                    ui.separator();
                                    ui.label(RichText::new(&reply.author).strong());
                                    if let Ok(t) =
                                        time::OffsetDateTime::from_unix_timestamp(reply.created)
                                    {
                                        ui.weak(format!("{t} UTC"));
                                    }
                                    ui.label(&reply.text);
                                }
                            });
                        ui.add_enabled_ui(editable && !a.object.locked, |ui| {
                            if ui
                                .checkbox(&mut a.object.review.resolved, "Resolved")
                                .changed()
                            {
                                change = Some(a.clone());
                            }
                            ui.add(
                                egui::TextEdit::singleline(&mut self.review.author)
                                    .hint_text("Your name")
                                    .char_limit(200),
                            );
                            ui.add(
                                egui::TextEdit::multiline(&mut self.review.reply)
                                    .hint_text("Reply…")
                                    .desired_rows(3)
                                    .char_limit(4000),
                            );
                            if ui
                                .add_enabled(
                                    !self.review.reply.trim().is_empty()
                                        && !self.review.author.trim().is_empty()
                                        && a.object.review.replies.len() < 100,
                                    ActionButton::new(Icon::Note, "Add reply").primary(),
                                )
                                .clicked()
                            {
                                if a.object.review.author.is_empty() {
                                    a.object.review.author = self.review.author.trim().into();
                                    a.object.review.created = timestamp();
                                }
                                a.object.review.replies.push(Reply {
                                    author: self.review.author.trim().into(),
                                    created: timestamp(),
                                    text: self.review.reply.trim().into(),
                                });
                                change = Some(a);
                                self.review.reply.clear();
                            }
                        });
                    } else {
                        ui.weak("Select an annotation to read or reply.");
                    }
                });
                if let Some(e) = &self.review.error {
                    ui.colored_label(theme::danger(), e);
                }
            });
        self.review.open = open;
        if let Some(t) = self.tabs.iter_mut().find(|t| t.id == id) {
            if let Some(page) = jump {
                t.jump(page);
            }
            if let Some(a) = change
                && selection::commit(t, &[a], None, &mut self.status).is_some()
            {
                t.session
                    .as_mut()
                    .unwrap()
                    .name_last_change("Review comment");
            }
        }
        if export
            && let Some(path) = menu::with_file_dialog(self._native_menu.as_ref(), || {
                rfd::FileDialog::new()
                    .set_title("Save annotation summary")
                    .set_file_name("Annotation summary.csv")
                    .add_filter("CSV", &["csv"])
                    .save_file()
            })
        {
            match viewer::save_copy(&path, mypdf_milestone0::review::csv(&filtered).as_bytes()) {
                Ok(()) => self.status = Some(("Annotation summary saved".into(), false)),
                Err(e) => self.review.error = Some(e.to_string()),
            }
        }
    }
}
