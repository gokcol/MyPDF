//! Native Home workspace; thumbnails are rendered lazily on one bounded worker.
use super::*;
use mypdf_milestone0::home::{Entry, History};
use std::path::Path;

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
    pub(super) fn reading(&self, path: &Path) -> Option<mypdf_milestone0::home::ReadingState> {
        self.history.as_ref().and_then(|h| h.reading(path))
    }
    pub(super) fn save_reading(
        &mut self,
        states: &[(PathBuf, mypdf_milestone0::home::ReadingState)],
    ) -> anyhow::Result<()> {
        let result = self
            .history
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Home history is unavailable"))
            .and_then(|history| history.save_reading(states));
        self.error = result.as_ref().err().map(|error| format!("{error:#}"));
        result
    }
    pub fn remember(&mut self, path: &Path) {
        self.change(|h| h.record(path));
        self.previews
            .remove(&path.canonicalize().unwrap_or_else(|_| path.to_path_buf()));
    }
    pub(super) fn recent_paths(&self) -> Vec<PathBuf> {
        self.history
            .as_ref()
            .map(|h| {
                let mut entries: Vec<_> = h.entries.iter().filter(|e| e.opened > 0).collect();
                entries.sort_by(|a, b| b.opened.cmp(&a.opened).then_with(|| a.path.cmp(&b.path)));
                entries.into_iter().map(|e| e.path.clone()).collect()
            })
            .unwrap_or_default()
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
        &mypdf_milestone0::render_fonts::settings(),
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
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::paper())).show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 5.;
            egui::ScrollArea::vertical().id_salt("home_scroll").auto_shrink([false, false]).show(ui, |ui| {
                let margin = ((ui.available_width() - 1140.) / 2.).max(28.);
                egui::Frame::new().inner_margin(egui::Margin { left: margin as i8, right: margin as i8, top: 26, bottom: 32 }).show(ui, |ui| {
                    if ui.available_width() >= 800. {
                        ui.horizontal(|ui| {
                            home_heading(ui);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                home_search(ui, &mut self.home, 230.);
                            });
                        });
                    } else {
                        home_heading(ui);
                        ui.add_space(10.);
                        home_search(ui, &mut self.home, ui.available_width());
                    }
                    if self.content.busy {
                        ui.horizontal(|ui| { ui.spinner(); ui.label("Creating your PDF…"); });
                    }
                    ui.add_space(18.);
                    if open_banner(ui).clicked() { action = Some(HomeAction::Open); }
                    ui.add_space(20.);
                    for favorite in [false, true] {
                        let entries = self.home.entries(favorite);
                        let columns = ((ui.available_width() + 16.) / 186.).floor().max(1.) as usize;
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
                            egui::Frame::new().fill(theme::surface()).stroke(Stroke::new(1., theme::line())).corner_radius(16.).inner_margin(16.).show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    let (r, _) = ui.allocate_exact_size(vec2(34., 34.), egui::Sense::hover());
                                    ui.painter().rect_filled(r, 10, theme::accent_soft());
                                    ui::draw_icon(ui.painter(), r.shrink(8.), if favorite { Icon::Star } else { Icon::Document }, theme::accent());
                                    ui.add_space(6.);
                                    ui.label(RichText::new(if !self.home.query.is_empty() { "No matching documents." } else if favorite { "Keep a document close. Click its star to add it here." } else { "Your recently opened and saved PDFs will appear here." }).color(theme::muted()).size(13.));
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
                        ui.add_space(20.);
                        if !favorite {
                            if let Some(tool) = quick_tools(ui) { action = Some(tool); }
                            ui.add_space(20.);
                        }
                    }
                    if let Some(error) = &self.home.error {
                        ui.colored_label(theme::danger(), format!("Home history: {error}"));
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
            .backdrop_color(Color32::from_black_alpha(64))
            .frame(ui::modal_frame())
            .show(ctx, |ui| {
                ui.set_width(380.);
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(44., 44.), egui::Sense::hover());
                    ui.painter().rect_filled(r, 10, theme::accent_soft());
                    ui::draw_icon(ui.painter(), r.shrink(11.), Icon::Import, theme::accent());
                    ui.add_space(4.);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.;
                        ui::title(ui, "Create a PDF", 18.);
                        ui.label(
                            RichText::new("Choose the kind of file to convert.")
                                .size(13.)
                                .color(theme::muted()),
                        );
                    });
                });
                ui.add_space(16.);
                ui.add_enabled_ui(!self.content.busy, |ui| {
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
                        if option_row(ui, icon, label, detail).clicked() {
                            choice = Some(images);
                        }
                        ui.add_space(4.);
                    }
                });
                if self.content.busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Creating your PDF…").color(theme::muted()));
                    });
                }
                ui.add_space(12.);
                if ui::dialog_button(ui, "Cancel", ui::DialogTone::Secondary).clicked() {
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
fn home_heading(ui: &mut egui::Ui) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.;
        ui.label(
            RichText::new("Home")
                .font(home_font(ui, 28., true))
                .color(theme::ink()),
        );
        ui.label(
            RichText::new("Read, edit, annotate and sign. Everything stays on your device.")
                .size(13.)
                .color(theme::muted()),
        );
    });
}
fn home_search(ui: &mut egui::Ui, home: &mut Home, width: f32) {
    let response = ui.add(
        egui::TextEdit::singleline(&mut home.query)
            .hint_text("Search recents & favorites")
            .desired_width(width)
            .min_size(vec2(0., ui::target_size(28.)))
            .vertical_align(egui::Align::Center)
            .margin(ui::FIELD_MARGIN),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::TextEdit,
            true,
            "Search recents and favorites",
        )
    });
    if home.focus_search {
        response.request_focus();
        home.focus_search = false;
    }
}
fn home_font(ui: &egui::Ui, size: f32, bold: bool) -> FontId {
    if bold {
        ui::bold(ui, size)
    } else {
        FontId::proportional(size)
    }
}
fn section_title(ui: &mut egui::Ui, label: &str) {
    ui::title(ui, label, 19.);
}
fn button_info(response: &egui::Response, label: &str) {
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
}
/// A selectable dialog row: colored icon tile, title, detail and a chevron.
fn option_row(ui: &mut egui::Ui, icon: Icon, label: &str, detail: &str) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), 62.), egui::Sense::click());
    button_info(&response, label);
    let enabled = ui.is_enabled();
    let hovered = enabled && response.hovered();
    let painter = ui.painter();
    painter.rect(
        rect,
        12,
        if hovered {
            theme::accent_soft()
        } else {
            theme::surface()
        },
        Stroke::new(
            1.,
            if hovered {
                theme::accent_line()
            } else {
                theme::line()
            },
        ),
        egui::StrokeKind::Inside,
    );
    if response.has_focus() {
        painter.rect_stroke(
            rect,
            12,
            Stroke::new(2.5, theme::accent()),
            egui::StrokeKind::Inside,
        );
    }
    let tile = Rect::from_center_size(pos2(rect.left() + 32., rect.center().y), vec2(36., 36.));
    painter.rect_filled(
        tile,
        9,
        if enabled {
            theme::accent_soft()
        } else {
            theme::control()
        },
    );
    ui::draw_icon(
        painter,
        tile.shrink(9.),
        icon,
        if enabled {
            theme::accent()
        } else {
            theme::muted()
        },
    );
    painter.text(
        pos2(rect.left() + 60., rect.center().y - 9.),
        egui::Align2::LEFT_CENTER,
        label,
        ui::strong(ui, 14.),
        if enabled {
            theme::ink()
        } else {
            theme::muted()
        },
    );
    painter.text(
        pos2(rect.left() + 60., rect.center().y + 10.),
        egui::Align2::LEFT_CENTER,
        detail,
        FontId::proportional(12.),
        theme::muted(),
    );
    ui::draw_icon(
        painter,
        Rect::from_center_size(pos2(rect.right() - 20., rect.center().y), vec2(14., 14.)),
        Icon::Right,
        if hovered {
            theme::accent()
        } else {
            theme::muted()
        },
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
/// A dashed rounded-rectangle outline, used for drop zones.
pub(super) fn dashed_outline(painter: &egui::Painter, rect: Rect, r: f32, stroke: Stroke) {
    let mut border = Vec::new();
    for (center, start) in [
        (rect.right_top() + vec2(-r, r), -90.),
        (rect.right_bottom() + vec2(-r, -r), 0.),
        (rect.left_bottom() + vec2(r, -r), 90.),
        (rect.left_top() + vec2(r, r), 180.),
    ] {
        for step in 0..=8 {
            let a = (start + step as f32 * 90. / 8.).to_radians();
            border.push(center + vec2(a.cos(), a.sin()) * r);
        }
    }
    border.push(border[0]);
    painter.extend(egui::Shape::dashed_line(&border, stroke, 9., 7.));
}
/// A compact drop strip that uses the selected interface accent.
fn open_banner(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), 104.), egui::Sense::click());
    button_info(&response, "Open Files");
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, 14, theme::accent_soft());
        dashed_outline(
            painter,
            rect.shrink(6.),
            9.,
            Stroke::new(
                1.,
                if response.hovered() {
                    theme::accent()
                } else {
                    theme::accent_line()
                },
            ),
        );
        if response.has_focus() {
            painter.rect_stroke(
                rect,
                14,
                Stroke::new(2.5, theme::accent()),
                egui::StrokeKind::Inside,
            );
        }
        let tile = Rect::from_center_size(pos2(rect.left() + 46., rect.center().y), vec2(44., 44.));
        painter.rect_filled(
            tile,
            10,
            if response.hovered() {
                theme::accent_hover()
            } else {
                theme::accent()
            },
        );
        ui::draw_icon(painter, tile.shrink(11.), Icon::Open, theme::on_accent());
        let left = rect.left() + 82.;
        painter.text(
            pos2(left, rect.center().y - 12.),
            egui::Align2::LEFT_CENTER,
            "Open Files",
            ui::strong(ui, 18.),
            theme::ink(),
        );
        let hint = if rect.width() >= 480. {
            "or drop PDFs anywhere in this window"
        } else {
            "or drop PDFs here"
        };
        painter.text(
            pos2(left, rect.center().y + 14.),
            egui::Align2::LEFT_CENTER,
            hint,
            home_font(ui, 12.5, false),
            theme::muted(),
        );
        if rect.width() >= 480. {
            ui::draw_icon(
                painter,
                Rect::from_center_size(pos2(rect.right() - 32., rect.center().y), vec2(18., 18.)),
                Icon::Right,
                theme::accent(),
            );
        }
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
fn quick_tools(ui: &mut egui::Ui) -> Option<HomeAction> {
    section_title(ui, "Tools for You");
    ui.add_space(10.);
    let tools = [
        (
            Icon::Text,
            "Edit PDF",
            "Text & images",
            HomeAction::Mode(editing::Mode::Edit),
        ),
        (
            Icon::Highlight,
            "Annotate",
            "Highlight & comment",
            HomeAction::Mode(editing::Mode::Annotate),
        ),
        (
            Icon::Library,
            "Manage Pages",
            "Reorder, rotate, crop",
            HomeAction::Mode(editing::Mode::Pages),
        ),
        (
            Icon::Import,
            "Files to PDF",
            "Images, Word, Markdown",
            HomeAction::Import,
        ),
        (
            Icon::Sign,
            "Sign PDF",
            "Ink or digital signature",
            HomeAction::Mode(editing::Mode::Sign),
        ),
        (
            Icon::Blank,
            "New Blank PDF",
            "Start from scratch",
            HomeAction::Blank,
        ),
    ];
    let columns = if ui.available_width() >= 960. {
        6
    } else if ui.available_width() >= 520. {
        3
    } else {
        2
    };
    let gap = 12.;
    let width = (ui.available_width() - gap * (columns - 1) as f32) / columns as f32;
    let mut action = None;
    for row in tools.chunks(columns) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (icon, label, detail, target) in row {
                if tool_card(ui, *icon, label, detail, width).clicked() {
                    action = Some(target.clone());
                }
            }
        });
        ui.add_space(6.);
    }
    action
}
/// A quiet theme surface with a stronger border on hover.
fn card(painter: &egui::Painter, rect: Rect, hovered: bool, focused: bool) {
    ui::shadow(painter, rect, 12, 1);
    painter.rect(
        rect,
        16,
        theme::surface(),
        Stroke::new(
            1.,
            if hovered {
                theme::accent_line()
            } else {
                theme::line()
            },
        ),
        egui::StrokeKind::Inside,
    );
    if focused {
        painter.rect_stroke(
            rect,
            16,
            Stroke::new(2.5, theme::accent()),
            egui::StrokeKind::Inside,
        );
    }
}
fn tool_card(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    detail: &str,
    width: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 98.), egui::Sense::click());
    button_info(&response, label);
    if ui.is_rect_visible(rect) {
        card(ui.painter(), rect, response.hovered(), response.has_focus());
        let tile = Rect::from_center_size(rect.center_top() + vec2(0., 27.), vec2(32., 32.));
        ui.painter().rect_filled(tile, 8, theme::accent_soft());
        ui::draw_icon(ui.painter(), tile.shrink(7.), icon, theme::accent());
        ui.painter().text(
            rect.center_top() + vec2(0., 60.),
            egui::Align2::CENTER_CENTER,
            label,
            ui::strong(ui, 13.),
            theme::ink(),
        );
        let mut job = egui::text::LayoutJob::simple_singleline(
            detail.into(),
            home_font(ui, 11.5, false),
            theme::muted(),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width((width - 16.).max(1.));
        let text = ui.fonts_mut(|fonts| fonts.layout_job(job));
        ui.painter().galley(
            pos2(rect.center().x - text.size().x / 2., rect.top() + 76.),
            text,
            theme::muted(),
        );
    }
    response
        .on_hover_text(detail)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
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
        let (rect, response) = ui.allocate_exact_size(vec2(width, 130.), egui::Sense::click());
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
                14,
                if response.hovered() {
                    theme::accent_soft()
                } else {
                    theme::control()
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
            let size = image.map_or(vec2(76., 106.), |t| {
                let s = t.size_vec2();
                s * (space.width() / s.x).min(space.height() / s.y)
            });
            let lift = if response.hovered() { -3. } else { 0. };
            let paper = Rect::from_center_size(rect.center() + vec2(0., lift), size);
            ui::shadow(painter, paper, 3, if response.hovered() { 3 } else { 1 });
            painter.rect(
                paper,
                3.,
                Color32::WHITE,
                Stroke::new(0.7, Color32::from_gray(226)),
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
                    Icon::Document,
                    Color32::from_gray(125),
                );
            }
            if response.has_focus() {
                painter.rect_stroke(
                    rect,
                    14,
                    Stroke::new(2.5, theme::accent()),
                    egui::StrokeKind::Inside,
                );
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
            ui.set_min_height(ui::target_size(32.));
            ui.spacing_mut().item_spacing.x = ui::control_gap(6.);
            let label = if entry.favorite {
                "Remove from Favorites"
            } else {
                "Add to Favorites"
            };
            let (r, favorite) = ui.allocate_exact_size(
                vec2(ui::target_size(22.), ui::target_size(24.)),
                egui::Sense::click(),
            );
            button_info(&favorite, &format!("{label}: {}", entry.name()));
            ui::draw_icon(
                ui.painter(),
                Rect::from_center_size(r.center(), vec2(17., 18.)),
                Icon::Star,
                if entry.favorite {
                    theme::accent()
                } else if favorite.hovered() {
                    theme::accent_hover()
                } else {
                    theme::muted()
                },
            );
            if entry.favorite {
                ui.painter()
                    .circle_filled(r.center() + vec2(0., 0.6), 2.6, theme::accent());
            }
            if favorite.has_focus() {
                ui.painter().rect_stroke(
                    r,
                    4.,
                    Stroke::new(1.5, theme::accent()),
                    egui::StrokeKind::Inside,
                );
            }
            if favorite.on_hover_text(label).clicked() {
                action = Some(HomeAction::Favorite(entry.path.clone(), !entry.favorite));
            }
            let name = entry.name();
            let mut job = egui::text::LayoutJob::simple(
                name.clone(),
                ui::strong(ui, 13.),
                theme::ink(),
                ui.available_width().max(1.),
            );
            job.wrap.max_rows = 2;
            ui.add(egui::Label::new(job).wrap()).on_hover_text(&name);
        });
        if let Some(reading) = &entry.reading {
            ui.label(
                RichText::new(format!("Resume · page {}", reading.page.saturating_add(1)))
                    .size(11.5)
                    .color(theme::accent()),
            )
            .on_hover_text(format!("Last opened {}", entry.date()));
        } else {
            ui.label(
                RichText::new(if entry.opened > 0 {
                    entry.date()
                } else {
                    "Favorite".into()
                })
                .size(11.5)
                .color(theme::muted()),
            );
        }
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

    #[test]
    fn recent_paths_are_newest_first_even_when_loaded_history_is_unordered() {
        let ctx = egui::Context::default();
        let mut home = Home::new(&ctx);
        let root = home.root.clone();
        let older = root.join("Older.pdf");
        let newest = root.join("Newest.pdf");
        let favorite = root.join("Favorite only.pdf");
        home.history.as_mut().unwrap().entries = vec![
            Entry {
                path: older.clone(),
                opened: 100,
                favorite: false,
                reading: None,
            },
            Entry {
                path: favorite,
                opened: 0,
                favorite: true,
                reading: None,
            },
            Entry {
                path: newest.clone(),
                opened: 300,
                favorite: false,
                reading: None,
            },
        ];
        assert_eq!(home.recent_paths(), vec![newest, older]);
    }

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
    fn settings_opens_without_a_document() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.action("settings", &ctx);
        assert!(app.digital.open && app.home.visible && app.tabs.is_empty());
        assert!(app.digital.panel == digital::Panel::Ink);
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
    fn home_recents_show_resume_before_tools_with_readable_names_and_optional_touch_targets() {
        for density in theme::Density::ALL {
            for width in [720., 1200.] {
                let ctx = egui::Context::default();
                ctx.enable_accesskit();
                let mut app =
                    Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
                theme::set_density(density);
                let name = "Annual audit findings and corrective action plan";
                let path = app.home.root.join(format!("{name}.pdf"));
                app.home.history.as_mut().unwrap().entries = vec![Entry {
                    path: path.clone(),
                    opened: 1_797_000_000_000,
                    favorite: false,
                    reading: Some(mypdf_milestone0::home::ReadingState {
                        page: 11,
                        ..Default::default()
                    }),
                }];
                app.home.previews.insert(
                    path,
                    Thumbnail {
                        image: None,
                        error: None,
                        used: 0,
                    },
                );
                for _ in 0..3 {
                    frame(&mut app, &ctx, width, vec![]);
                }
                let output = frame(&mut app, &ctx, width, vec![]);
                let nodes = output.platform_output.accesskit_update.unwrap().nodes;
                let bounds = |label: &str| {
                    nodes
                        .iter()
                        .find(|(_, n)| {
                            n.label() == Some(label)
                                || n.role() == egui::accesskit::Role::Label
                                    && n.value() == Some(label)
                        })
                        .unwrap_or_else(|| panic!("Missing Home label: {label}"))
                        .1
                        .bounds()
                        .unwrap()
                };
                let search = bounds("Search recents and favorites");
                let banner = bounds("Open Files");
                let recent = bounds(&format!("Open {name}"));
                let resume = bounds("Resume · page 12");
                let tools = bounds("Edit PDF");
                let filename = bounds(name);
                let favorite = bounds(&format!("Add to Favorites: {name}"));
                assert_eq!(banner.y1 - banner.y0, 104.);
                assert!(
                    recent.y0 > banner.y1 && recent.y1 < tools.y0,
                    "Recents should be visible before quick tools"
                );
                assert!(resume.y0 >= recent.y1 && resume.y1 < tools.y0);
                assert!(
                    filename.y1 - filename.y0 >= 24.,
                    "Long filenames need two lines"
                );
                for rect in [search, banner, recent, resume, filename, favorite, tools] {
                    assert!(
                        rect.x0 >= 0. && rect.x1 <= width as f64,
                        "Home item clipped: {rect:?}"
                    );
                }
                if density == theme::Density::Comfortable {
                    assert!(search.y1 - search.y0 >= 44.);
                    assert!(favorite.x1 - favorite.x0 >= 44. && favorite.y1 - favorite.y0 >= 44.);
                } else {
                    assert_eq!(favorite.x1 - favorite.x0, 22.);
                    assert_eq!(favorite.y1 - favorite.y0, 24.);
                }
            }
        }
        theme::set_density(theme::Density::Compact);
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
        app.save_tx
            .send(Ok(save::Completed {
                path: saved.clone(),
                id,
                revision: 0,
                fingerprint: [0; 32],
            }))
            .unwrap();
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
        app.home_action(HomeAction::Recent(saved), &ctx);
        assert!(!app.home.visible);
        assert_eq!(
            app.tabs.len(),
            1,
            "Opening a recent already in a tab must reuse it"
        );
    }
}
