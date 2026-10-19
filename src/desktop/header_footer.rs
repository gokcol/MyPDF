//! Edit → Header & Footer: page numbers, Bates numbers and text in the page
//! margins, with a live page preview and a history of recent text.
use super::*;
use mypdf_milestone0::header_footer::{Kind, Position, Stamp, Summary};
use mypdf_milestone0::text_style;
use serde::{Deserialize, Serialize};

const MM: f32 = 72. / 25.4;
const HISTORY: usize = 20;
/// Page number formats: label and template.
const FORMATS: [(&str, &str); 6] = [
    ("1", "{page}"),
    ("Page 1", "Page {page}"),
    ("Page 1 of n", "Page {page} of {pages}"),
    ("1 of n", "{page} of {pages}"),
    ("1/n", "{page}/{pages}"),
    ("- 1 -", "- {page} -"),
];
const FIELDS: [(&str, &str); 4] = [
    ("Page number", "{page}"),
    ("Page count", "{pages}"),
    ("Date", "{date}"),
    ("File name", "{file}"),
];
const COLORS: [[f32; 3]; 5] = [
    [0., 0., 0.],
    [0.33, 0.33, 0.33],
    [0.62, 0.62, 0.62],
    [0.84, 0.2, 0.16],
    [0.1, 0.36, 0.85],
];

/// Settings remembered between uses: recent text and each kind's last layout.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    history: Vec<String>,
    text: Stamp,
    page_numbers: Stamp,
    bates: Stamp,
    format: usize,
    prefix: String,
    suffix: String,
}
impl Default for Saved {
    fn default() -> Self {
        Self {
            history: vec![],
            text: Stamp::default(),
            page_numbers: Stamp {
                kind: Kind::PageNumber,
                template: FORMATS[2].1.into(),
                position: Position::BottomCenter,
                ..Default::default()
            },
            bates: Stamp {
                kind: Kind::Bates,
                template: "{bates}".into(),
                position: Position::BottomRight,
                ..Default::default()
            },
            format: 2,
            prefix: String::new(),
            suffix: String::new(),
        }
    }
}
impl Saved {
    #[cfg(not(test))]
    fn path() -> PathBuf {
        mypdf_milestone0::assets::data_directory().join("header_footer.json")
    }
    fn load() -> Self {
        #[cfg(not(test))]
        if let Ok(raw) = std::fs::read(Self::path())
            && let Ok(saved) = serde_json::from_slice(&raw)
        {
            return saved;
        }
        Self::default()
    }
    fn save(&self) {
        #[cfg(not(test))]
        if let Ok(raw) = serde_json::to_vec_pretty(self) {
            let path = Self::path();
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, raw);
        }
    }
    fn stamp(&self, kind: Kind) -> &Stamp {
        match kind {
            Kind::Text => &self.text,
            Kind::PageNumber => &self.page_numbers,
            Kind::Bates => &self.bates,
        }
    }
}

struct Dialog {
    tab: u64,
    stamp: Stamp,
    all_pages: bool,
    /// One-based page range.
    from: usize,
    to: usize,
    preview: usize,
    format: usize,
    prefix: String,
    suffix: String,
}

pub(super) struct HeaderFooterState {
    saved: Saved,
    dialog: Option<Dialog>,
    summary: Option<(u64, u64, Summary)>,
    job: Option<(u64, u64, mpsc::Receiver<Option<Summary>>)>,
    confirm_remove: Option<u64>,
}
impl HeaderFooterState {
    pub fn new() -> Self {
        Self {
            saved: Saved::load(),
            dialog: None,
            summary: None,
            job: None,
            confirm_remove: None,
        }
    }
}

fn today() -> String {
    let date = time::OffsetDateTime::now_utc().date();
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day()
    )
}

fn title(kind: Kind) -> &'static str {
    match kind {
        Kind::Text => "Add Text",
        Kind::PageNumber => "Add Page Numbers",
        Kind::Bates => "Add Bates Numbers",
    }
}

impl Viewer {
    /// Open the dialog for the active document.
    pub(super) fn open_header_footer(&mut self, kind: Kind) {
        let Some(tab) = self.tabs.get(self.active).filter(|t| !t.pages.is_empty()) else {
            self.status = Some(("Open a PDF first.".into(), false));
            return;
        };
        if !tab.session.as_ref().is_some_and(|s| s.editable) {
            self.status = Some((
                if tab.signed {
                    "This PDF is signed. Choose “Enable editing…” in the signature bar first."
                } else {
                    "This PDF is read-only."
                }
                .into(),
                true,
            ));
            return;
        }
        let saved = &self.header_footer.saved;
        let mut stamp = saved.stamp(kind).clone();
        stamp.kind = kind;
        stamp.pages = None;
        stamp.file = tab.name();
        stamp.date = today();
        if stamp.font.as_ref().is_some_and(|f| !f.is_file()) {
            stamp.font = None;
        }
        self.header_footer.dialog = Some(Dialog {
            tab: tab.id,
            stamp,
            all_pages: true,
            from: 1,
            to: tab.pages.len(),
            preview: tab.page,
            format: saved.format.min(FORMATS.len() - 1),
            prefix: saved.prefix.clone(),
            suffix: saved.suffix.clone(),
        });
    }

    /// What the active document already carries, checked in the background.
    pub(super) fn header_footer_summary(&mut self, ctx: &egui::Context) -> Option<Summary> {
        let tab = self.tabs.get(self.active)?;
        let session = tab.session.as_ref()?;
        let key = (tab.id, session.revision());
        let state = &mut self.header_footer;
        if let Some((tab, revision, rx)) = &state.job
            && let Ok(summary) = rx.try_recv()
        {
            if let Some(summary) = summary {
                state.summary = Some((*tab, *revision, summary));
            }
            state.job = None;
        }
        if let Some((tab, revision, summary)) = state.summary
            && (tab, revision) == key
        {
            return Some(summary);
        }
        if state.job.as_ref().is_none_or(|(t, r, _)| (*t, *r) != key) {
            let (tx, rx) = mpsc::channel();
            let source = session.source.clone();
            let wake = ctx.clone();
            std::thread::spawn(move || {
                let summary = std::panic::catch_unwind(|| {
                    mypdf_milestone0::header_footer::summary(&source).ok()
                })
                .ok()
                .flatten();
                let _ = tx.send(summary);
                wake.request_repaint();
            });
            state.job = Some((key.0, key.1, rx));
        }
        None
    }

    /// The Header & Footer menu in the Edit ribbon.
    pub(super) fn header_footer_menu(&mut self, ui: &mut egui::Ui, enabled: bool) {
        let response = ui.add_enabled(
            enabled,
            ActionButton::new(Icon::HeaderFooter, "Header & Footer").dropdown(),
        );
        let ctx = ui.ctx().clone();
        let mut chosen = None;
        let mut summary = None;
        if egui::Popup::is_id_open(&ctx, egui::Popup::default_response_id(&response)) {
            summary = self.header_footer_summary(&ctx);
        }
        egui::Popup::menu(&response).show(|ui| {
            ui.set_min_width(220.);
            for (kind, icon, label) in [
                (Kind::PageNumber, Icon::Document, "Add Page Numbers…"),
                (Kind::Bates, Icon::Document, "Add Bates Numbers…"),
                (Kind::Text, Icon::Text, "Add Text…"),
            ] {
                if ui.add(ActionButton::new(icon, label)).clicked() {
                    chosen = Some(Ok(kind));
                    ui.close();
                }
            }
            ui.separator();
            let numbered = summary.is_some_and(|s| s.page_numbers + s.bates > 0);
            if ui
                .add_enabled(
                    numbered,
                    ActionButton::new(Icon::Trash, "Remove Page Numbers…"),
                )
                .on_disabled_hover_text(if summary.is_none() {
                    "Checking this document…"
                } else {
                    "No page or Bates numbers added by MyPDF"
                })
                .clicked()
            {
                chosen = Some(Err(false));
                ui.close();
            }
            if ui
                .add_enabled(
                    summary.is_some_and(|s| s.any()),
                    ActionButton::new(Icon::Trash, "Remove All Text"),
                )
                .on_hover_text("Remove every header and footer that MyPDF added")
                .clicked()
            {
                chosen = Some(Err(true));
                ui.close();
            }
        });
        match chosen {
            Some(Ok(kind)) => self.open_header_footer(kind),
            Some(Err(false)) => self.request_remove_page_numbers(),
            Some(Err(true)) => self.remove_header_footer(&ctx, Kind::ALL.to_vec()),
            None => {}
        }
    }

    #[cfg(test)]
    pub(super) fn preview_header_footer(&mut self) {
        self.header_footer.saved.history = vec![
            "A00F251e - Release Protocol".into(),
            "Confidential – internal use".into(),
        ];
        self.open_header_footer(Kind::Text);
        if let Some(d) = &mut self.header_footer.dialog {
            d.stamp.template = "A00F251e - Release Protocol · {page}/{pages}".into();
            d.stamp.margins[1] = 4. * MM;
            d.stamp.margins[2] = 12. * MM;
            d.stamp.color = [0.1, 0.36, 0.85];
        }
    }
    pub(super) fn request_remove_page_numbers(&mut self) {
        self.header_footer.confirm_remove = self.tabs.get(self.active).map(|t| t.id);
    }

    pub(super) fn remove_header_footer(&mut self, ctx: &egui::Context, kinds: Vec<Kind>) {
        self.content_change(ctx, content::Edit::RemoveHeaderFooter(kinds));
    }

    /// The Add dialog and the removal confirmation.
    pub(super) fn header_footer_window(&mut self, ctx: &egui::Context) {
        if let Some(id) = self.header_footer.confirm_remove
            && let Some(choice) = ui::confirmation(
                ctx,
                "remove_page_numbers",
                "Remove page numbers?",
                "Page and Bates numbers that MyPDF added are removed from every page. Undo restores them.",
                &[
                    ("Cancel", ui::DialogTone::Secondary),
                    ("Remove", ui::DialogTone::Destructive),
                ],
                0,
            )
        {
            self.header_footer.confirm_remove = None;
            if choice == 1 && self.tabs.get(self.active).is_some_and(|t| t.id == id) {
                self.remove_header_footer(ctx, vec![Kind::PageNumber, Kind::Bates]);
            }
        }
        let Some(dialog) = &mut self.header_footer.dialog else {
            return;
        };
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == dialog.tab) else {
            self.header_footer.dialog = None;
            return;
        };
        let count = tab.pages.len().max(1);
        let mut close = false;
        let mut apply = false;
        let history = self.header_footer.saved.history.clone();
        let modal = egui::Modal::new(egui::Id::new("header_footer"))
            .backdrop_color(Color32::from_black_alpha(64))
            .frame(ui::modal_frame())
            .show(ctx, |ui| {
                ui.set_width(620.);
                ui.spacing_mut().item_spacing = vec2(8., 10.);
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(40., 40.), egui::Sense::hover());
                    ui::badge(
                        ui.painter(),
                        r,
                        Icon::HeaderFooter,
                        ui::Fill::Brand,
                        Color32::WHITE,
                    );
                    ui.add_space(4.);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.;
                        ui::title(ui, title(dialog.stamp.kind), 18.);
                        ui.label(
                            RichText::new("Placed in the page margin, on every page you choose.")
                                .size(12.5)
                                .color(theme::muted()),
                        );
                    });
                });
                ui::section(ui, "Pages range", |ui| {
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut dialog.all_pages, true, "All pages");
                        ui.add_space(16.);
                        ui.radio_value(&mut dialog.all_pages, false, "Pages from:");
                        ui.add_enabled_ui(!dialog.all_pages, |ui| {
                            ui.add(egui::DragValue::new(&mut dialog.from).range(1..=count));
                            ui.label("to:");
                            ui.add(egui::DragValue::new(&mut dialog.to).range(1..=count));
                        });
                        dialog.to = dialog.to.max(dialog.from);
                    });
                });
                match dialog.stamp.kind {
                    Kind::Text => ui::section(ui, "Text", |ui| {
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut dialog.stamp.template)
                                    .id(egui::Id::new("header_footer_text"))
                                    .hint_text("Enter text")
                                    .char_limit(500)
                                    .margin(ui::FIELD_MARGIN)
                                    .desired_width(ui.available_width() - 175.),
                            );
                            let recent = ui
                                .add(
                                    ActionButton::new(Icon::History, "Recent text")
                                        .compact()
                                        .dropdown(),
                                )
                                .on_hover_text("Recently used text");
                            egui::Popup::menu(&recent).show(|ui| {
                                ui.set_min_width(260.);
                                if history.is_empty() {
                                    ui.label(
                                        RichText::new("No recent text yet").color(theme::muted()),
                                    );
                                }
                                for text in &history {
                                    if ui.add(egui::Button::new(text).truncate()).clicked() {
                                        dialog.stamp.template = text.clone();
                                        ui.close();
                                    }
                                }
                            });
                            let field = ui
                                .add(ActionButton::new(Icon::Plus, "Field").dropdown())
                                .on_hover_text(
                                    "Insert a page number, page count, date or file name",
                                );
                            egui::Popup::menu(&field).show(|ui| {
                                for (label, token) in FIELDS {
                                    if ui.button(label).clicked() {
                                        dialog.stamp.template.push_str(token);
                                        ui.close();
                                    }
                                }
                            });
                        });
                    }),
                    Kind::PageNumber => ui::section(ui, "Format", |ui| {
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt("page_number_format")
                                .width(200.)
                                .selected_text(FORMATS[dialog.format].0)
                                .show_ui(ui, |ui| {
                                    for (i, (label, _)) in FORMATS.iter().enumerate() {
                                        ui.selectable_value(&mut dialog.format, i, *label);
                                    }
                                });
                            ui.add_space(12.);
                            ui.label("Start at");
                            ui.add(
                                egui::DragValue::new(&mut dialog.stamp.start).range(0..=999_999),
                            );
                        });
                        dialog.stamp.template = FORMATS[dialog.format].1.into();
                    }),
                    Kind::Bates => ui::section(ui, "Bates number", |ui| {
                        egui::Grid::new("bates_grid")
                            .num_columns(4)
                            .spacing(vec2(10., 8.))
                            .show(ui, |ui| {
                                ui.label("Prefix");
                                ui.add(
                                    egui::TextEdit::singleline(&mut dialog.prefix)
                                        .char_limit(60)
                                        .desired_width(150.),
                                );
                                ui.label("Suffix");
                                ui.add(
                                    egui::TextEdit::singleline(&mut dialog.suffix)
                                        .char_limit(60)
                                        .desired_width(150.),
                                );
                                ui.end_row();
                                ui.label("Start at");
                                ui.add(
                                    egui::DragValue::new(&mut dialog.stamp.start)
                                        .range(0..=999_999_999),
                                );
                                ui.label("Digits");
                                ui.add(
                                    egui::DragValue::new(&mut dialog.stamp.digits).range(1..=12),
                                );
                                ui.end_row();
                            });
                        dialog.stamp.template = format!(
                            "{}{{bates}}{}",
                            dialog.prefix.replace('{', "(").replace('}', ")"),
                            dialog.suffix.replace('{', "(").replace('}', ")")
                        );
                    }),
                }
                dialog.stamp.pages =
                    (!dialog.all_pages).then_some((dialog.from - 1, dialog.to - 1));
                ui::section(ui, "Layout and font options", |ui| {
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width(250.);
                            preview(ui, tab, dialog);
                            ui.horizontal(|ui| {
                                if ui
                                    .add_enabled(
                                        dialog.preview > 0,
                                        ActionButton::new(Icon::Left, "Previous page").compact(),
                                    )
                                    .clicked()
                                {
                                    dialog.preview -= 1;
                                }
                                ui.add_sized(
                                    vec2(160., 28.),
                                    egui::Label::new(
                                        RichText::new(format!("{} of {count}", dialog.preview + 1))
                                            .color(theme::ink()),
                                    ),
                                );
                                if ui
                                    .add_enabled(
                                        dialog.preview + 1 < count,
                                        ActionButton::new(Icon::Right, "Next page").compact(),
                                    )
                                    .clicked()
                                {
                                    dialog.preview += 1;
                                }
                            });
                        });
                        ui.add_space(12.);
                        ui.vertical(|ui| {
                            options(ui, &mut dialog.stamp);
                        });
                    });
                });
                ui.add_space(4.);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let ready = !dialog.stamp.template.trim().is_empty();
                        if ui
                            .add_enabled(ready, ActionButton::new(Icon::Check, "Apply").primary())
                            .clicked()
                        {
                            apply = true;
                        }
                        if ui.add(ActionButton::new(Icon::Close, "Cancel")).clicked() {
                            close = true;
                        }
                    });
                });
            });
        if modal.should_close() {
            close = true;
        }
        if apply {
            self.apply_header_footer(ctx);
        } else if close {
            self.header_footer.dialog = None;
        }
    }

    /// Remember the settings and recent text, then stamp the document.
    fn apply_header_footer(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.header_footer.dialog.take() else {
            return;
        };
        let saved = &mut self.header_footer.saved;
        let mut kept = dialog.stamp.clone();
        kept.pages = None;
        match kept.kind {
            Kind::Text => {
                let text = dialog.stamp.template.trim().to_owned();
                saved.history.retain(|t| *t != text);
                saved.history.insert(0, text);
                saved.history.truncate(HISTORY);
                saved.text = kept;
            }
            Kind::PageNumber => {
                saved.format = dialog.format;
                saved.page_numbers = kept;
            }
            Kind::Bates => {
                saved.prefix = dialog.prefix.clone();
                saved.suffix = dialog.suffix.clone();
                saved.bates = kept;
            }
        }
        saved.save();
        if let Some(index) = self.tabs.iter().position(|t| t.id == dialog.tab) {
            self.active = index;
            self.content_change(ctx, content::Edit::HeaderFooter(dialog.stamp));
        }
    }
}

/// A thumbnail of the chosen page with the text where it will go.
fn preview(ui: &mut egui::Ui, tab: &mut Tab, dialog: &mut Dialog) {
    let page = dialog.preview.min(tab.pages.len().saturating_sub(1));
    let size = tab.pages[page];
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
    let stamp = &dialog.stamp;
    let (first, last) = stamp.pages.unwrap_or((0, tab.pages.len() - 1));
    if page < first || page > last || stamp.template.trim().is_empty() {
        return;
    }
    let text = stamp.text_for(page - first, last - first + 1);
    let family = stamp
        .font
        .as_ref()
        .and_then(|path| text_style::fonts().iter().find(|f| &f.path == path))
        .map(|f| egui::FontFamily::Name(f.ui_name().into()))
        .unwrap_or(egui::FontFamily::Proportional);
    let color = Color32::from_rgb(
        (stamp.color[0] * 255.) as u8,
        (stamp.color[1] * 255.) as u8,
        (stamp.color[2] * 255.) as u8,
    );
    let galley = painter.layout_no_wrap(
        text,
        FontId::new((stamp.size * factor).max(1.), family),
        color,
    );
    let [top, bottom, left, right] = stamp.margins;
    let width = galley.size().x;
    let x = match stamp.position.column() {
        -1 => rect.left() + left * factor,
        0 => rect.center().x - width / 2.,
        _ => rect.right() - right * factor - width,
    };
    let baseline = if stamp.position.top() {
        rect.top() + (top + stamp.size * 0.8) * factor
    } else {
        rect.bottom() - (bottom + stamp.size * 0.22) * factor
    };
    let ascent = galley.rows.first().map_or(0., |r| r.row.height()) * 0.8;
    painter.galley(pos2(x, baseline - ascent), galley, color);
}

/// Position, margins, font and color.
fn options(ui: &mut egui::Ui, stamp: &mut Stamp) {
    ui::caption(ui, "Position");
    egui::ComboBox::from_id_salt("header_footer_position")
        .width(ui.available_width() - 28.)
        .selected_text(stamp.position.label())
        .show_ui(ui, |ui| {
            for position in Position::ALL {
                ui.selectable_value(&mut stamp.position, position, position.label());
            }
        });
    let used = stamp.position.margins_used();
    egui::Grid::new("header_footer_margins")
        .num_columns(2)
        .spacing(vec2(12., 6.))
        .show(ui, |ui| {
            for row in [[0, 1], [2, 3]] {
                for i in row {
                    let label = ["Top", "Bottom", "Left", "Right"][i];
                    ui.vertical(|ui| {
                        ui.label(RichText::new(label).size(11.5).color(theme::muted()));
                        let mut mm = stamp.margins[i] / MM;
                        ui.add_enabled_ui(used[i], |ui| {
                            if ui
                                .add(
                                    egui::DragValue::new(&mut mm)
                                        .range(0.0..=500.0)
                                        .speed(0.5)
                                        .fixed_decimals(1)
                                        .suffix(" mm"),
                                )
                                .changed()
                            {
                                stamp.margins[i] = mm * MM;
                            }
                        });
                    });
                }
                ui.end_row();
            }
        });
    ui::caption(ui, "Font");
    let fonts = text_style::fonts();
    let current = stamp
        .font
        .as_ref()
        .and_then(|path| fonts.iter().position(|f| &f.path == path))
        .unwrap_or_else(|| fonts.iter().position(|f| !f.bold && !f.italic).unwrap_or(0));
    if let Some(face) = fonts.get(current) {
        egui::ComboBox::from_id_salt("header_footer_font")
            .width(ui.available_width() - 28.)
            .selected_text(&face.family)
            .show_ui(ui, |ui| {
                for font in fonts.iter().filter(|f| !f.bold && !f.italic) {
                    if ui
                        .selectable_label(font.family == face.family, &font.family)
                        .clicked()
                    {
                        let same_style = fonts.iter().find(|f| {
                            f.family == font.family
                                && f.bold == face.bold
                                && f.italic == face.italic
                        });
                        stamp.font = Some(same_style.unwrap_or(font).path.clone());
                    }
                }
            });
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("header_footer_style")
                .width(130.)
                .selected_text(face.style())
                .show_ui(ui, |ui| {
                    for style in fonts.iter().filter(|f| f.family == face.family) {
                        if ui
                            .selectable_label(style.path == face.path, style.style())
                            .clicked()
                        {
                            stamp.font = Some(style.path.clone());
                        }
                    }
                });
            ui.add(
                egui::DragValue::new(&mut stamp.size)
                    .range(4.0..=144.0)
                    .speed(0.25)
                    .suffix(" pt"),
            );
        });
    }
    ui::caption(ui, "Text color");
    ui.horizontal(|ui| {
        for color in COLORS {
            let (r, response) = ui.allocate_exact_size(vec2(26., 26.), egui::Sense::click());
            let fill = Color32::from_rgb(
                (color[0] * 255.) as u8,
                (color[1] * 255.) as u8,
                (color[2] * 255.) as u8,
            );
            if stamp.color == color {
                ui.painter()
                    .circle_stroke(r.center(), 12., Stroke::new(2., theme::accent()));
            }
            ui.painter().circle_filled(r.center(), 9., fill);
            ui.painter()
                .circle_stroke(r.center(), 9., Stroke::new(1., theme::line()));
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                stamp.color = color;
            }
        }
        ui::color_swatch(ui, &mut stamp.color, "Custom color");
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;

    #[test]
    fn text_is_stamped_remembered_found_and_removed() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let source = mypdf_milestone0::convert::blank(595., 842.).unwrap();
        app.open_memory(PathBuf::from("Memo.pdf"), source, &ctx);
        let frame = |app: &mut Viewer| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 820.))),
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
        };
        let until = |app: &mut Viewer, ready: &dyn Fn(&mut Viewer) -> bool| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while !ready(app) {
                assert!(std::time::Instant::now() < deadline, "{:?}", app.status);
                frame(app);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        let texts = |app: &Viewer| -> String {
            let source = &app.tabs[0].session.as_ref().unwrap().source;
            mypdf_milestone0::document::text_runs(source, 1)
                .unwrap()
                .into_iter()
                .map(|r| r.text)
                .collect()
        };
        until(&mut app, &|a| {
            a.tabs[0].session.as_ref().is_some_and(|s| s.editable)
        });
        app.open_header_footer(Kind::Text);
        app.header_footer.dialog.as_mut().unwrap().stamp.template =
            "Memo {page}/{pages} · {file}".into();
        for _ in 0..3 {
            frame(&mut app);
        }
        assert!(app.header_footer.dialog.is_some(), "the dialog stays open");
        app.apply_header_footer(&ctx);
        until(&mut app, &|a| !a.content.busy);
        assert_eq!(texts(&app), "Memo 1/1 · Memo.pdf", "{:?}", app.status);
        assert_eq!(
            app.header_footer.saved.history.first().map(String::as_str),
            Some("Memo {page}/{pages} · {file}")
        );
        until(&mut app, &|a| {
            a.header_footer_summary(&ctx).is_some_and(|s| s.text == 1)
        });

        app.remove_header_footer(&ctx, Kind::ALL.to_vec());
        until(&mut app, &|a| !a.content.busy);
        assert_eq!(texts(&app), "");
        until(&mut app, &|a| {
            a.tabs[0].session.as_ref().is_some_and(|s| s.editable)
        });
        app.remove_header_footer(&ctx, Kind::ALL.to_vec());
        until(&mut app, &|a| !a.content.busy);
        assert!(
            app.status
                .as_ref()
                .is_some_and(|(s, error)| *error && s.contains("No header or footer")),
            "{:?}",
            app.status
        );
    }
}
