//! Native Home workspace; thumbnails are rendered lazily on one bounded worker.
use super::*;
use mypdf_milestone0::home::{Entry, History};
use std::path::Path;

const BLUE: Color32 = Color32::from_rgb(0, 119, 250);
const SOFT_BLUE: Color32 = Color32::from_rgb(239, 246, 255);
type PreviewResult = (PathBuf, Result<egui::ColorImage, String>);
struct Thumbnail {
    image: Option<TextureHandle>,
    error: Option<String>,
    used: u64,
}
pub(super) struct Home {
    pub visible: bool,
    history: Option<History>,
    pub error: Option<String>,
    query: String,
    all_recents: bool,
    all_favorites: bool,
    pub(super) focus_search: bool,
    import_open: bool,
    previews: HashMap<PathBuf, Thumbnail>,
    pending: Option<PathBuf>,
    requests: mpsc::SyncSender<PathBuf>,
    results: mpsc::Receiver<PreviewResult>,
    root: PathBuf,
    #[cfg(test)]
    _temporary: tempfile::TempDir,
}
impl Home {
    pub fn new(ctx: &egui::Context) -> Self {
        #[cfg(test)]
        let temporary = tempfile::tempdir().unwrap();
        #[cfg(test)]
        let root = temporary.path().into();
        #[cfg(not(test))]
        let root = mypdf_milestone0::assets::data_directory();
        let (requests, rx) = mpsc::sync_channel::<PathBuf>(1);
        let (tx, results) = mpsc::sync_channel(1);
        let wake = ctx.clone();
        std::thread::spawn(move || {
            while let Ok(path) = rx.recv() {
                let result = std::panic::catch_unwind(|| thumbnail(&path))
                    .map_err(|_| "Preview unavailable".into())
                    .and_then(|r| r.map_err(|e| format!("{e:#}")));
                if tx.send((path, result)).is_err() {
                    break;
                }
                wake.request_repaint();
            }
        });
        let mut home = Self {
            visible: true,
            history: None,
            error: None,
            query: String::new(),
            all_recents: false,
            all_favorites: false,
            focus_search: false,
            import_open: false,
            previews: HashMap::new(),
            pending: None,
            requests,
            results,
            root,
            #[cfg(test)]
            _temporary: temporary,
        };
        home.reload();
        home
    }
    fn reload(&mut self) {
        match History::load(self.root.clone()) {
            Ok(h) => {
                self.history = Some(h);
                self.error = None;
            }
            Err(e) => {
                self.error = Some(format!("{e:#}"));
            }
        }
        self.previews.clear();
    }
    pub fn remember(&mut self, path: &Path) {
        self.change(|h| h.record(path));
        self.previews
            .remove(&path.canonicalize().unwrap_or_else(|_| path.to_path_buf()));
    }
    fn change(&mut self, action: impl FnOnce(&mut History) -> anyhow::Result<()>) {
        if let Some(history) = &mut self.history {
            self.error = action(history).err().map(|e| format!("{e:#}"));
        }
    }
    fn poll(&mut self, ctx: &egui::Context, tick: u64) {
        for (path, result) in self.results.try_iter() {
            self.pending = None;
            let (image, error) = match result {
                Ok(image) => (
                    Some(ctx.load_texture(
                        format!("home-{}", path.display()),
                        image,
                        egui::TextureOptions::LINEAR,
                    )),
                    None,
                ),
                Err(error) => (None, Some(error)),
            };
            self.previews.insert(
                path,
                Thumbnail {
                    image,
                    error,
                    used: tick,
                },
            );
        }
        while self.previews.len() > 32 {
            let oldest = self
                .previews
                .iter()
                .min_by_key(|(_, p)| p.used)
                .map(|(k, _)| k.clone())
                .unwrap();
            self.previews.remove(&oldest);
        }
    }
    fn entries(&self, favorite: bool) -> Vec<Entry> {
        let query = self.query.to_lowercase();
        self.history
            .as_ref()
            .map(|h| {
                h.entries
                    .iter()
                    .filter(|e| {
                        (if favorite { e.favorite } else { e.opened > 0 })
                            && e.name().to_lowercase().contains(&query)
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
}
fn thumbnail(path: &Path) -> anyhow::Result<egui::ColorImage> {
    let bytes = mypdf_milestone0::convert::read_bounded(path, 64 * 1024 * 1024)?;
    let pdf = hayro::hayro_syntax::Pdf::new(bytes)
        .map_err(|e| anyhow::anyhow!("Preview unavailable: {e:?}"))?;
    let page = pdf
        .pages()
        .first()
        .ok_or_else(|| anyhow::anyhow!("No pages"))?;
    let (w, h) = page.render_dimensions();
    anyhow::ensure!(
        w.is_finite() && h.is_finite() && w >= 1. && h >= 1. && w.max(h) <= 100_000.,
        "Unsupported page dimensions"
    );
    let scale = 400. / w.max(h);
    let raster = hayro::render(
        page,
        &hayro::RenderCache::new(),
        &hayro::hayro_interpret::InterpreterSettings::default(),
        &hayro::RenderSettings::default(),
        &hayro::PixmapSettings {
            x_scale: scale,
            y_scale: scale,
            bg_color: hayro::vello_cpu::color::palette::css::WHITE,
        },
    );
    Ok(egui::ColorImage::from_rgba_premultiplied(
        [raster.width() as usize, raster.height() as usize],
        raster.data_as_u8_slice(),
    ))
}

#[derive(Clone)]
enum HomeAction {
    Open,
    Blank,
    Mode(editing::Mode),
    Import,
    Recent(PathBuf),
    Favorite(PathBuf, bool),
    Remove(PathBuf),
    Clear,
}
impl Viewer {
    pub(super) fn show_home(&mut self) {
        if self.apply_pending_annotation() {
            self.home.visible = true;
            self.home.reload();
        }
    }
    pub(super) fn home_screen(&mut self, ctx: &egui::Context) {
        self.home.poll(ctx, self.tick);
        let mut action = None;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::WHITE)).show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 5.;
            for font in ui.style_mut().text_styles.values_mut() { *font = home_font(font.size, false); }
            egui::ScrollArea::vertical().id_salt("home_scroll").auto_shrink([false, false]).show(ui, |ui| {
                let margin = ((ui.available_width() - 1140.) / 2.).max(28.);
                egui::Frame::new().inner_margin(egui::Margin { left: margin as i8, right: margin as i8, top: 26, bottom: 32 }).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Home").font(home_font(34., true)).color(INK));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let response = ui.add(egui::TextEdit::singleline(&mut self.home.query).hint_text("Search recents & favorites").desired_width(205.).margin(vec2(12., 9.)));
                            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Search recents and favorites"));
                            if self.home.focus_search { response.request_focus(); self.home.focus_search = false; }
                        });
                    });
                    if self.content.busy {
                        ui.horizontal(|ui| { ui.spinner(); ui.label("Creating your PDF…"); });
                    }
                    ui.add_space(24.);
                    if open_banner(ui).clicked() { action = Some(HomeAction::Open); }
                    ui.add_space(22.);
                    section_title(ui, "Tools for You");
                    ui.add_space(12.);
                    let tools = [
                        (Icon::Text, "Edit PDF", BLUE, HomeAction::Mode(editing::Mode::Edit)),
                        (Icon::Highlight, "Annotate", Color32::from_rgb(208, 147, 0), HomeAction::Mode(editing::Mode::Annotate)),
                        (Icon::Library, "Manage Pages", Color32::from_rgb(69, 154, 244), HomeAction::Mode(editing::Mode::Pages)),
                        (Icon::Import, "Files to PDF", Color32::from_rgb(232, 74, 89), HomeAction::Import),
                        (Icon::Sign, "Sign PDF", Color32::from_rgb(108, 91, 203), HomeAction::Mode(editing::Mode::Sign)),
                        (Icon::Blank, "New Blank PDF", BLUE, HomeAction::Blank),
                    ];
                    let columns = if ui.available_width() >= 960. { 6 } else { 3 };
                    let width = (ui.available_width() - 12. * (columns - 1) as f32) / columns as f32;
                    for row in tools.chunks(columns) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 12.;
                            for (icon, label, color, target) in row {
                                if tool_card(ui, *icon, label, *color, width).clicked() { action = Some(target.clone()); }
                            }
                        });
                        ui.add_space(6.);
                    }
                    ui.add_space(18.);
                    for favorite in [false, true] {
                        let entries = self.home.entries(favorite);
                        let columns = ((ui.available_width() + 16.) / 155.).floor().max(1.) as usize;
                        ui.horizontal(|ui| {
                            section_title(ui, if favorite { "Favorites" } else { "Recents" });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let all = if favorite { &mut self.home.all_favorites } else { &mut self.home.all_recents };
                                if entries.len() > columns && ui.link(if *all { "Show Less" } else { "View All" }).clicked() { *all = !*all; }
                                if !favorite && !entries.is_empty() && ui.link("Clear Recents").on_hover_text("Remove recent shortcuts. Your PDFs and favorites stay saved.").clicked() { action = Some(HomeAction::Clear); }
                            });
                        });
                        ui.add_space(12.);
                        if entries.is_empty() {
                            egui::Frame::new().fill(Color32::from_rgb(248, 249, 251)).corner_radius(14.).inner_margin(18.).show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    let (r, _) = ui.allocate_exact_size(vec2(24., 24.), egui::Sense::hover());
                                    ui::draw_icon(ui.painter(), r, if favorite { Icon::Star } else { Icon::Read }, Color32::from_gray(163));
                                    ui.add_space(6.);
                                    ui.label(RichText::new(if !self.home.query.is_empty() { "No matching documents." } else if favorite { "Keep a document close. Click its star to add it here." } else { "Your recently opened and saved PDFs will appear here." }).color(MUTED).size(13.));
                                });
                            });
                        } else {
                            let width = (ui.available_width() - 16. * (columns - 1) as f32) / columns as f32;
                            let all = if favorite { self.home.all_favorites } else { self.home.all_recents };
                            let limit = if all || !self.home.query.is_empty() { entries.len() } else { columns };
                            for row in entries[..limit.min(entries.len())].chunks(columns) {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 16.;
                                    for entry in row {
                                        ui.push_id((favorite, &entry.path), |ui| {
                                            if let Some(a) = document_card(ui, &mut self.home, entry, width, self.tick) { action = Some(a); }
                                        });
                                    }
                                });
                                ui.add_space(12.);
                            }
                        }
                        ui.add_space(26.);
                    }
                    if let Some(error) = &self.home.error {
                        ui.colored_label(Color32::from_rgb(165, 57, 43), format!("Home history: {error}"));
                        if ui.link("Reload history").clicked() { self.home.reload(); }
                    }
                });
            });
        });
        if let Some(action) = action {
            self.home_action(action, ctx);
        }
        self.home_import_window(ctx);
    }
    fn home_action(&mut self, action: HomeAction, ctx: &egui::Context) {
        match action {
            HomeAction::Open => self.open_dialog(ctx),
            HomeAction::Blank => self.new_blank(ctx),
            HomeAction::Recent(path) => {
                if path.is_file() {
                    self.open(path, ctx);
                } else {
                    self.status = Some((
                        format!(
                            "{} has moved or is unavailable. Open it from its new location, or remove its shortcut from Recents.",
                            path.display()
                        ),
                        true,
                    ));
                }
            }
            HomeAction::Mode(mode) => {
                if self.tabs.is_empty() {
                    self.open_dialog(ctx);
                } else {
                    self.home.visible = false;
                }
                if !self.home.visible {
                    self.editor.mode = mode;
                    self.editor.library_open = false;
                    self.inspector = true;
                }
            }
            HomeAction::Import => self.home.import_open = true,
            HomeAction::Favorite(path, value) => self.home.change(|h| h.favorite(&path, value)),
            HomeAction::Remove(path) => self.home.change(|h| h.remove_recent(&path)),
            HomeAction::Clear => {
                self.home.change(History::clear_recents);
                self.home.previews.clear();
            }
        }
    }
    fn home_import_window(&mut self, ctx: &egui::Context) {
        if !self.home.import_open {
            return;
        }
        let mut choice = None;
        let modal = egui::Modal::new(egui::Id::new("home_import"))
            .frame(
                egui::Frame::popup(&ctx.style())
                    .corner_radius(22.)
                    .inner_margin(26.),
            )
            .show(ctx, |ui| {
                ui.set_width(370.);
                ui.label(RichText::new("Create a PDF").size(23.).strong());
                ui.add_space(8.);
                ui.label(
                    RichText::new("Choose the type of file you want to convert.").color(MUTED),
                );
                ui.add_space(18.);
                for (images, icon, label, detail) in [
                    (
                        true,
                        Icon::Import,
                        "Images to PDF",
                        "PNG or JPEG · one image per page",
                    ),
                    (
                        false,
                        Icon::Text,
                        "Word or Markdown to PDF",
                        "DOCX, MD or TXT · text and basic paragraphs",
                    ),
                ] {
                    if ui
                        .add_enabled(!self.content.busy, ActionButton::new(icon, label))
                        .clicked()
                    {
                        choice = Some(images);
                    }
                    ui.label(RichText::new(detail).small().color(MUTED));
                    ui.add_space(12.);
                }
                if self.content.busy {
                    ui.spinner();
                    ui.label("Creating your PDF…");
                }
                ui.separator();
                if ui.button("Cancel").clicked() {
                    self.home.import_open = false;
                }
            });
        if modal.should_close() {
            self.home.import_open = false;
        }
        if let Some(images) = choice {
            self.home.import_open = false;
            self.import_conversion(ctx, images);
        }
    }
}
fn home_font(size: f32, bold: bool) -> FontId {
    let face = mypdf_milestone0::text_style::fonts().iter().find(|face| {
        matches!(face.family.as_str(), "Arial" | "DejaVuSans") && face.bold == bold && !face.italic
    });
    face.map_or_else(
        || FontId::proportional(size),
        |f| FontId::new(size, egui::FontFamily::Name(f.ui_name().into())),
    )
}
fn section_title(ui: &mut egui::Ui, label: &str) {
    ui.label(RichText::new(label).font(home_font(21., true)).color(INK));
}
fn button_info(response: &egui::Response, label: &str) {
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
}
fn open_banner(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), 190.), egui::Sense::click());
    button_info(&response, "Open Files");
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(
            rect,
            20.,
            if response.hovered() {
                Color32::from_rgb(231, 242, 255)
            } else {
                SOFT_BLUE
            },
        );
        let inset = rect.shrink(8.);
        let r = 12.;
        let mut border = Vec::new();
        for (center, start) in [
            (inset.right_top() + vec2(-r, r), -90.),
            (inset.right_bottom() + vec2(-r, -r), 0.),
            (inset.left_bottom() + vec2(r, -r), 90.),
            (inset.left_top() + vec2(r, r), 180.),
        ] {
            for step in 0..=8 {
                let a = (start + step as f32 * 90. / 8.).to_radians();
                border.push(center + vec2(a.cos(), a.sin()) * r);
            }
        }
        border.push(border[0]);
        painter.extend(egui::Shape::dashed_line(
            &border,
            Stroke::new(1.4, Color32::from_rgb(176, 208, 250)),
            10.,
            8.,
        ));
        if response.has_focus() {
            painter.rect_stroke(rect, 20., Stroke::new(2., BLUE), egui::StrokeKind::Inside);
        }
        let center = rect.center_top() + vec2(0., 60.);
        for (x, y) in [(-16., 5.), (16., 5.), (0., 0.)] {
            let page = Rect::from_center_size(center + vec2(x, y), vec2(42., 56.));
            painter.rect_filled(
                page.translate(vec2(0., 3.)),
                4.,
                Color32::from_rgb(210, 226, 249),
            );
            painter.rect(
                page,
                4.,
                Color32::WHITE,
                Stroke::new(1., Color32::from_rgb(186, 215, 255)),
                egui::StrokeKind::Inside,
            );
            if x == 0. {
                painter.text(
                    page.center_bottom() - vec2(0., 15.),
                    egui::Align2::CENTER_CENTER,
                    "PDF",
                    home_font(15., true),
                    Color32::from_rgb(116, 179, 253),
                );
                for y in [12., 18.] {
                    painter.line_segment(
                        [page.min + vec2(9., y), page.min + vec2(33., y)],
                        Stroke::new(1., Color32::from_rgb(181, 212, 251)),
                    );
                }
            }
        }
        let pill = Rect::from_center_size(rect.center_top() + vec2(0., 131.), vec2(142., 38.));
        painter.rect_filled(pill, 19., BLUE);
        painter.text(
            pill.center(),
            egui::Align2::CENTER_CENTER,
            "Open Files",
            home_font(16., true),
            Color32::WHITE,
        );
        painter.text(
            rect.center_top() + vec2(0., 173.),
            egui::Align2::CENTER_CENTER,
            "or drop PDFs here",
            home_font(14., false),
            Color32::from_rgb(90, 151, 230),
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
fn tool_card(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    color: Color32,
    width: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 116.), egui::Sense::click());
    button_info(&response, label);
    if ui.is_rect_visible(rect) {
        let blank = matches!(icon, Icon::Blank);
        ui.painter().rect_filled(
            rect,
            17.,
            if response.hovered() {
                Color32::from_rgb(230, 240, 253)
            } else if blank {
                SOFT_BLUE
            } else {
                Color32::from_rgb(247, 248, 250)
            },
        );
        if response.has_focus() {
            ui.painter()
                .rect_stroke(rect, 17., Stroke::new(2., BLUE), egui::StrokeKind::Inside);
        }
        let center = rect.center_top() + vec2(0., 40.);
        ui.painter()
            .circle_filled(center, 23., color.gamma_multiply(0.09));
        ui::draw_icon(
            ui.painter(),
            Rect::from_center_size(center, vec2(32., 32.)),
            icon,
            color,
        );
        ui.painter().text(
            rect.center_top() + vec2(0., 86.),
            egui::Align2::CENTER_CENTER,
            label,
            home_font(14., false),
            INK,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
fn document_card(
    ui: &mut egui::Ui,
    home: &mut Home,
    entry: &Entry,
    width: f32,
    tick: u64,
) -> Option<HomeAction> {
    let mut action = None;
    ui.vertical(|ui| {
        ui.set_width(width);
        let (rect, response) = ui.allocate_exact_size(vec2(width, 150.), egui::Sense::click());
        button_info(&response, &format!("Open {}", entry.name()));
        if ui.is_rect_visible(rect) {
            if !home.previews.contains_key(&entry.path)
                && home.pending.is_none()
                && home.requests.try_send(entry.path.clone()).is_ok()
            {
                home.pending = Some(entry.path.clone());
            }
            let painter = ui.painter();
            painter.rect_filled(
                rect,
                12.,
                if response.hovered() {
                    SOFT_BLUE
                } else {
                    Color32::from_rgb(248, 249, 251)
                },
            );
            let preview = home.previews.get_mut(&entry.path);
            if let Some(p) = preview.as_ref()
                && let Some(error) = &p.error
            {
                response.clone().on_hover_text(error);
            }
            let image = preview.and_then(|p| {
                p.used = tick;
                p.image.as_ref()
            });
            let space = rect.shrink2(vec2(18., 12.));
            let size = image.map_or(vec2(98., 138.), |t| {
                let s = t.size_vec2();
                s * (space.width() / s.x).min(space.height() / s.y)
            });
            let paper = Rect::from_center_size(rect.center(), size);
            painter.rect_filled(paper.translate(vec2(0., 2.)), 3., Color32::from_gray(219));
            painter.rect(
                paper,
                3.,
                Color32::WHITE,
                Stroke::new(0.7, Color32::from_gray(223)),
                egui::StrokeKind::Inside,
            );
            if let Some(image) = image {
                painter.image(
                    image.id(),
                    paper.shrink(1.),
                    Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)),
                    Color32::WHITE,
                );
            } else {
                ui::draw_icon(
                    painter,
                    Rect::from_center_size(paper.center(), vec2(30., 30.)),
                    Icon::Read,
                    Color32::from_rgb(184, 199, 219),
                );
            }
            if response.has_focus() {
                painter.rect_stroke(rect, 12., Stroke::new(2., BLUE), egui::StrokeKind::Inside);
            }
        }
        if response.clicked() {
            action = Some(HomeAction::Recent(entry.path.clone()));
        }
        response
            .clone()
            .on_hover_text(entry.path.display().to_string())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        response.context_menu(|ui| {
            if ui
                .button(if entry.favorite {
                    "Remove from Favorites"
                } else {
                    "Add to Favorites"
                })
                .clicked()
            {
                action = Some(HomeAction::Favorite(entry.path.clone(), !entry.favorite));
                ui.close();
            }
            if entry.opened > 0 && ui.button("Remove from Recents").clicked() {
                action = Some(HomeAction::Remove(entry.path.clone()));
                ui.close();
            }
        });
        ui.add_space(5.);
        ui.horizontal(|ui| {
            let label = if entry.favorite {
                "Remove from Favorites"
            } else {
                "Add to Favorites"
            };
            let (r, favorite) = ui.allocate_exact_size(vec2(22., 24.), egui::Sense::click());
            button_info(&favorite, &format!("{label}: {}", entry.name()));
            ui::draw_icon(
                ui.painter(),
                r.shrink(3.),
                Icon::Star,
                if entry.favorite {
                    Color32::from_rgb(227, 164, 20)
                } else {
                    Color32::from_gray(169)
                },
            );
            if favorite.has_focus() {
                ui.painter()
                    .rect_stroke(r, 4., Stroke::new(1., BLUE), egui::StrokeKind::Inside);
            }
            if favorite.on_hover_text(label).clicked() {
                action = Some(HomeAction::Favorite(entry.path.clone(), !entry.favorite));
            }
            ui.add(egui::Label::new(RichText::new(entry.name()).size(13.).color(INK)).truncate())
                .on_hover_text(entry.name());
        });
        ui.label(
            RichText::new(if entry.opened > 0 {
                entry.date()
            } else {
                "Favorite".into()
            })
            .size(11.)
            .color(MUTED),
        );
    });
    action
}

#[cfg(test)]
impl Home {
    pub(super) fn preview_favorite(&mut self, path: &Path) {
        self.change(|h| h.favorite(path, true));
    }
    pub(super) fn preview_clear(&mut self) {
        self.change(History::clear_recents);
        self.change(|h| {
            let paths: Vec<_> = h.entries.iter().map(|e| e.path.clone()).collect();
            for p in paths {
                h.favorite(&p, false)?;
            }
            Ok(())
        });
    }
    pub(super) fn preview_pending(&self) -> bool {
        self.pending.is_some()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;
    use std::time::{Duration, Instant};
    fn frame(
        app: &mut Viewer,
        ctx: &egui::Context,
        width: f32,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(width, 900.))),
                max_texture_side: Some(8192),
                events,
                ..Default::default()
            },
            |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
        )
    }
    fn wait_for(app: &mut Viewer, ctx: &egui::Context, ready: impl Fn(&Viewer) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !ready(app) {
            assert!(Instant::now() < deadline, "Home test timed out");
            frame(app, ctx, 1200., vec![]);
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    fn home_cards_are_accessible_responsive_and_new_blank_opens_a_document() {
        for width in [720., 1200.] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
            // Settle egui's first-frame panel measurements.
            for _ in 0..3 {
                frame(&mut app, &ctx, width, vec![]);
            }
            ctx.enable_accesskit();
            let out = frame(&mut app, &ctx, width, vec![]);
            let nodes = out.platform_output.accesskit_update.unwrap().nodes;
            let mut blank = None;
            for label in [
                "Open Files",
                "Edit PDF",
                "Annotate",
                "Manage Pages",
                "Files to PDF",
                "Sign PDF",
                "New Blank PDF",
            ] {
                let r = nodes
                    .iter()
                    .find(|(_, n)| n.label() == Some(label))
                    .unwrap()
                    .1
                    .bounds()
                    .unwrap();
                assert!(
                    r.x0 >= 0. && r.x1 <= width as f64,
                    "{label} is clipped at {width}: {r:?}"
                );
                if label == "New Blank PDF" {
                    blank = Some(pos2(
                        ((r.x0 + r.x1) / 2.) as f32,
                        ((r.y0 + r.y1) / 2.) as f32,
                    ));
                }
            }
            let p = blank.unwrap();
            for pressed in [true, false] {
                frame(
                    &mut app,
                    &ctx,
                    width,
                    vec![
                        egui::Event::PointerMoved(p),
                        egui::Event::PointerButton {
                            pos: p,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            assert!(!app.home.visible);
            assert_eq!(app.tabs.len(), 1);
            wait_for(&mut app, &ctx, |a| a.tabs[0].session.is_some());
            assert!(app.tabs[0].session.as_ref().unwrap().dirty());
            assert!(
                app.home.entries(false).is_empty(),
                "Unsaved blank PDFs must not create broken shortcuts"
            );
            app.show_home();
            let revision = app.tabs[0].session.as_ref().unwrap().revision();
            app.action("undo", &ctx);
            app.action("close", &ctx);
            assert_eq!(app.tabs.len(), 1);
            assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), revision);
            app.home_action(HomeAction::Mode(editing::Mode::Edit), &ctx);
            assert!(!app.home.visible && app.editor.mode == editing::Mode::Edit);
            app.close(0);
            assert!(app.home.visible);
        }
    }
    #[test]
    fn loaded_and_saved_pdfs_appear_in_home_and_missing_shortcuts_keep_work_open() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let path = root.join("Opened.pdf");
        let bytes = mypdf_milestone0::convert::blank(595., 842.).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let ctx = egui::Context::default();
        let mut app = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            vec![path.clone()],
        );
        wait_for(&mut app, &ctx, |a| a.tabs[0].session.is_some());
        assert_eq!(app.home.entries(false).len(), 1);
        let id = app.tabs[0].id;
        let saved = root.join("Saved Copy.pdf");
        std::fs::write(&saved, &bytes).unwrap();
        app.save_tx.send(Ok((saved.clone(), id, 0))).unwrap();
        frame(&mut app, &ctx, 1200., vec![]);
        assert_eq!(app.home.entries(false).len(), 2);
        app.home_action(HomeAction::Favorite(saved.clone(), true), &ctx);
        app.show_home();
        wait_for(&mut app, &ctx, |a| {
            a.home
                .previews
                .get(&saved)
                .is_some_and(|p| p.image.is_some())
        });
        app.home_action(HomeAction::Clear, &ctx);
        assert!(app.home.entries(false).is_empty());
        assert_eq!(app.home.entries(true).len(), 1);
        assert_eq!(std::fs::read(&saved).unwrap(), bytes);
        app.home_action(HomeAction::Recent(temp.path().join("Missing.pdf")), &ctx);
        assert!(app.home.visible);
        assert_eq!(app.tabs[0].id, id);
        assert_eq!(app.tabs.len(), 1);
        app.home_action(HomeAction::Recent(path), &ctx);
        assert!(!app.home.visible);
        assert_eq!(
            app.tabs.len(),
            1,
            "Opening a recent already in a tab must reuse it"
        );
    }
}
