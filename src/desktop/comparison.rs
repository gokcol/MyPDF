//! Compare Documents: two versions side by side. What was removed, added or
//! changed is marked on both pages and listed on the right.
use super::*;
use mypdf_milestone0::compare::{Change, Kind};

/// A version to compare: an open document or a file.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Pick {
    Tab(u64),
    File(PathBuf),
}
/// Choosing the two versions.
pub(super) struct Chooser {
    pub older: Option<Pick>,
    pub newer: Option<Pick>,
}

pub(super) struct Comparison {
    pub older: u64,
    pub newer: u64,
    job: Option<mpsc::Receiver<Result<Vec<Change>, String>>>,
    pub result: Option<Result<Arc<Vec<Change>>, String>>,
    pub selected: Option<usize>,
    /// Scroll both versions (and the list) to the selected change.
    reveal: bool,
    /// Some pages had more text than MyPDF reads.
    limited: bool,
}
impl Comparison {
    fn new(older: u64, newer: u64) -> Self {
        Self {
            older,
            newer,
            job: None,
            result: None,
            selected: None,
            reveal: false,
            limited: false,
        }
    }
    fn changes(&self) -> Arc<Vec<Change>> {
        match &self.result {
            Some(Ok(changes)) => changes.clone(),
            _ => Arc::default(),
        }
    }
    pub(super) fn select(&mut self, index: usize) {
        let count = self.changes().len();
        if count > 0 {
            self.selected = Some(index.min(count - 1));
            self.reveal = true;
        }
    }
    fn step(&mut self, forward: bool) {
        let count = self.changes().len();
        if count == 0 {
            return;
        }
        let next = match (self.selected, forward) {
            (None, _) => 0,
            (Some(i), true) => (i + 1) % count,
            (Some(i), false) => (i + count - 1) % count,
        };
        self.select(next);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Side {
    Older,
    Newer,
}
impl Side {
    fn words(self, change: &Change) -> &[mypdf_milestone0::compare::Word] {
        match self {
            Side::Older => &change.old,
            Side::Newer => &change.new,
        }
    }
    fn at(self, change: &Change) -> Option<(usize, [f32; 4])> {
        match self {
            Side::Older => change.old_at,
            Side::Newer => change.new_at,
        }
    }
}

fn color(kind: Kind) -> Color32 {
    match kind {
        Kind::Removed => theme::danger(),
        Kind::Added => theme::success(),
        Kind::Changed => theme::warning(),
    }
}
fn label(kind: Kind) -> &'static str {
    match kind {
        Kind::Removed => "Removed",
        Kind::Added => "Added",
        Kind::Changed => "Changed",
    }
}
fn shorten(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_owned()
    } else {
        format!("{}…", text.chars().take(limit - 1).collect::<String>())
    }
}

/// The highlights of one version's page: neighbouring words on a line join
/// into one mark. Each mark names its change.
fn marks(changes: &[Change], side: Side, page: usize) -> Vec<(usize, [f32; 4])> {
    let mut out: Vec<(usize, [f32; 4])> = vec![];
    for (index, change) in changes.iter().enumerate() {
        let words = side.words(change);
        if words.is_empty() {
            // Where the words were taken out or would go.
            if let Some((p, r)) = side.at(change)
                && p == page
            {
                out.push((index, [r[0] - 1., r[1] - 1., r[2] + 1., r[3] + 1.]));
            }
            continue;
        }
        for word in words.iter().filter(|w| w.page == page) {
            match out.last_mut() {
                // The same line: the boxes overlap by most of their height.
                Some((i, r))
                    if *i == index
                        && r[3].min(word.rect[3]) - r[1].max(word.rect[1])
                            > 0.5 * (r[3] - r[1]).min(word.rect[3] - word.rect[1])
                        && word.rect[0] >= r[0]
                        && word.rect[0] - r[2] < 20. =>
                {
                    r[2] = r[2].max(word.rect[2]);
                    r[1] = r[1].min(word.rect[1]);
                    r[3] = r[3].max(word.rect[3]);
                }
                _ => out.push((index, word.rect)),
            }
        }
    }
    out
}

/// Draw a page's picture, asking for it when it is not drawn yet.
fn paint_page(ui: &egui::Ui, tab: &mut Tab, page: usize, rect: Rect, scale: f32, tick: u64) {
    let size = tab.pages[page];
    ui::shadow(ui.painter(), rect, 1, 2);
    ui.painter().rect_filled(rect, 0.0, Color32::WHITE);
    let key = viewer::raster_key(page, size, scale * ui.ctx().pixels_per_point(), false);
    tab.request(key);
    let fallback = tab
        .rasters
        .keys()
        .filter(|k| k.page == page && !k.thumbnail)
        .min_by_key(|k| k.scale.abs_diff(key.scale))
        .copied();
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    let shown = [Some(key), fallback]
        .into_iter()
        .flatten()
        .find(|k| tab.rasters.contains_key(k));
    if let Some(raster) = shown.and_then(|k| tab.rasters.get_mut(&k)) {
        raster.used = tick;
        ui.painter()
            .image(raster.texture.id(), rect, uv, Color32::WHITE);
    } else {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            tab.render_errors
                .get(&page)
                .map(String::as_str)
                .unwrap_or("Rendering page…"),
            FontId::proportional(14.0),
            theme::muted(),
        );
    }
    ui.painter().rect_stroke(
        rect,
        0.0,
        Stroke::new(1.0, Color32::from_black_alpha(12)),
        egui::StrokeKind::Outside,
    );
}

/// One version's pages with its marks. Returns the change clicked on.
#[allow(clippy::too_many_arguments)]
fn column(
    ui: &mut egui::Ui,
    tab: &mut Tab,
    side: Side,
    changes: &[Change],
    selected: Option<usize>,
    reveal: bool,
    tick: u64,
) -> Option<usize> {
    ui.horizontal(|ui| {
        ui.add_space(12.);
        let (text, tint) = match side {
            Side::Older => ("Older", theme::muted()),
            Side::Newer => ("Newer", theme::accent()),
        };
        pill(ui, text, tint);
        ui.add(
            egui::Label::new(
                RichText::new(tab.name())
                    .font(ui::strong(ui, 13.))
                    .color(theme::ink()),
            )
            .truncate(),
        );
    });
    ui.add_space(4.);
    if tab.pages.is_empty() {
        ui.centered_and_justified(|ui| ui.spinner());
        return None;
    }
    let available = ui.available_size();
    let width = available.x - 14.;
    let widest = tab.pages.iter().map(|p| p.width).fold(1.0, f32::max);
    let scale = ((width - 32.) / widest).clamp(0.1, 4.);
    let mut tops = Vec::with_capacity(tab.pages.len());
    let mut height = 12.0;
    for page in &tab.pages {
        tops.push(height);
        height += page.height * scale + 16.0;
    }
    let mut scroll = egui::ScrollArea::vertical()
        .id_salt(("compare", tab.id))
        .auto_shrink([false, false]);
    if reveal
        && let Some(change) = selected.and_then(|i| changes.get(i))
        && let Some((page, r)) = side.at(change)
        && page < tops.len()
    {
        scroll =
            scroll.vertical_scroll_offset((tops[page] + r[1] * scale - available.y / 3.).max(0.));
    }
    let mut clicked = None;
    scroll.show_viewport(ui, |ui, viewport| {
        let (canvas, response) = ui.allocate_exact_size(vec2(width, height), egui::Sense::click());
        let pointer = response
            .clicked()
            .then(|| response.interact_pointer_pos())
            .flatten();
        for (page, top) in tops.iter().copied().enumerate() {
            let size = tab.pages[page];
            if top + size.height * scale < viewport.min.y - 200. || top > viewport.max.y + 200. {
                continue;
            }
            let page_rect = Rect::from_min_size(
                canvas.min + vec2((width - size.width * scale) / 2., top),
                vec2(size.width * scale, size.height * scale),
            );
            paint_page(ui, tab, page, page_rect, scale, tick);
            for (index, r) in marks(changes, side, page) {
                let mark = Rect::from_min_max(
                    page_rect.min + vec2(r[0], r[1]) * scale,
                    page_rect.min + vec2(r[2], r[3]) * scale,
                )
                .expand(1.)
                .intersect(page_rect);
                let tint = color(changes[index].kind);
                let chosen = selected == Some(index);
                ui.painter().rect_filled(
                    mark,
                    2.,
                    tint.gamma_multiply(if chosen { 0.42 } else { 0.24 }),
                );
                if chosen {
                    ui.painter().rect_stroke(
                        mark,
                        2.,
                        Stroke::new(1.5, tint),
                        egui::StrokeKind::Outside,
                    );
                }
                if pointer.is_some_and(|p| mark.expand(2.).contains(p)) {
                    clicked = Some(index);
                }
            }
        }
    });
    clicked
}

impl Viewer {
    pub(super) fn request_compare(&mut self) {
        let newer = (!self.home.visible)
            .then(|| self.tabs.get(self.active))
            .flatten()
            .map(|t| Pick::Tab(t.id));
        let older = match self.tabs.as_slice() {
            [a, b] => [a, b]
                .into_iter()
                .find(|t| Some(Pick::Tab(t.id)) != newer)
                .map(|t| Pick::Tab(t.id)),
            _ => None,
        };
        self.compare_chooser = Some(Chooser { older, newer });
    }

    fn pick_name(&self, pick: &Pick) -> String {
        match pick {
            Pick::Tab(id) => self
                .tabs
                .iter()
                .find(|t| t.id == *id)
                .map(|t| t.name())
                .unwrap_or_default(),
            Pick::File(path) => path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned(),
        }
    }

    pub(super) fn compare_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut chooser) = self.compare_chooser.take() else {
            return;
        };
        let documents: Vec<(Pick, String)> = self
            .tabs
            .iter()
            .filter(|t| t.error.is_none())
            .map(|t| (Pick::Tab(t.id), t.name()))
            .collect();
        let names = [&chooser.older, &chooser.newer].map(|p| p.as_ref().map(|p| self.pick_name(p)));
        let same = chooser.older.is_some() && chooser.older == chooser.newer;
        let ready = chooser.older.is_some() && chooser.newer.is_some() && !same;
        let (mut browse, mut start, mut cancel) = (None, false, false);
        let modal = egui::Modal::new(egui::Id::new("compare_documents"))
            .backdrop_color(Color32::from_black_alpha(64))
            .frame(ui::modal_frame())
            .show(ctx, |ui| {
                ui.set_width(380.);
                ui.vertical_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(54., 54.), egui::Sense::hover());
                    ui::badge(ui.painter(), r, Icon::Compare, ui::Fill::Brand, Color32::WHITE);
                    ui.add_space(12.);
                    ui::title(ui, "Compare documents", 16.);
                    ui.label(
                        RichText::new("See two versions side by side, with the words that were added, removed or changed marked on both.")
                            .size(13.)
                            .color(theme::muted()),
                    );
                });
                ui.add_space(14.);
                for (side, title) in [(Side::Older, "Older version"), (Side::Newer, "Newer version")] {
                    let (current, name) = match side {
                        Side::Older => (&mut chooser.older, &names[0]),
                        Side::Newer => (&mut chooser.newer, &names[1]),
                    };
                    ui::field(ui, title, |ui| {
                        egui::ComboBox::from_id_salt(("compare_pick", title))
                            .width(ui.available_width())
                            .selected_text(name.clone().unwrap_or_else(|| "Choose…".into()))
                            .show_ui(ui, |ui| {
                                for (pick, document) in &documents {
                                    if ui
                                        .selectable_label(current.as_ref() == Some(pick), document)
                                        .clicked()
                                    {
                                        *current = Some(pick.clone());
                                    }
                                }
                                if !documents.is_empty() {
                                    ui.separator();
                                }
                                if ui.selectable_label(false, "Choose a file…").clicked() {
                                    browse = Some(side);
                                }
                            });
                    });
                    ui.add_space(6.);
                }
                if ui
                    .add(ActionButton::new(Icon::Swap, "Swap older and newer"))
                    .clicked()
                {
                    std::mem::swap(&mut chooser.older, &mut chooser.newer);
                }
                if same {
                    ui.add_space(4.);
                    ui::callout(ui, ui::Tone::Warning, "Choose two different documents.");
                }
                ui.add_space(14.);
                let half = (ui.available_width() - 10.) / 2.;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.;
                    ui.allocate_ui(vec2(half, 32.), |ui| {
                        cancel = ui::dialog_button(ui, "Cancel", ui::DialogTone::Secondary).clicked();
                    });
                    ui.allocate_ui(vec2(half, 32.), |ui| {
                        ui.add_enabled_ui(ready, |ui| {
                            start = ui::dialog_button(ui, "Compare", ui::DialogTone::Primary).clicked();
                        });
                    });
                });
            });
        if modal.should_close() {
            cancel = true;
        }
        if let Some(side) = browse
            && let Some(path) = self.native_pick(
                rfd::FileDialog::new()
                    .set_title(match side {
                        Side::Older => "Choose the older version",
                        Side::Newer => "Choose the newer version",
                    })
                    .add_filter("PDF documents", &["pdf"]),
            )
        {
            let path = std::fs::canonicalize(&path).unwrap_or(path);
            // A file that is already open is the open document.
            let pick = self
                .tabs
                .iter()
                .find(|t| t.path == path)
                .map_or(Pick::File(path), |t| Pick::Tab(t.id));
            match side {
                Side::Older => chooser.older = Some(pick),
                Side::Newer => chooser.newer = Some(pick),
            }
        }
        if start && ready {
            if let (Some(older), Some(newer)) = (chooser.older, chooser.newer) {
                self.start_compare(older, newer, ctx);
            }
        } else if !cancel {
            self.compare_chooser = Some(chooser);
        }
    }

    pub(super) fn start_compare(&mut self, older: Pick, newer: Pick, ctx: &egui::Context) {
        let tab_of = |this: &mut Self, pick: Pick| match pick {
            Pick::Tab(id) => this.tabs.iter().any(|t| t.id == id).then_some(id),
            Pick::File(path) => {
                this.open(path.clone(), ctx);
                let path = std::fs::canonicalize(&path).unwrap_or(path);
                this.tabs.iter().find(|t| t.path == path).map(|t| t.id)
            }
        };
        let (Some(older), Some(newer)) = (tab_of(self, older), tab_of(self, newer)) else {
            return;
        };
        if let Some(index) = self.tabs.iter().position(|t| t.id == newer) {
            self.active = index;
        }
        self.home.visible = false;
        self.comparison = Some(Comparison::new(older, newer));
    }

    /// Start or finish the comparison as both versions become ready. Ends it
    /// when either is closed or another document is chosen.
    pub(super) fn compare_tick(&mut self, ctx: &egui::Context) {
        let Some(c) = &mut self.comparison else {
            return;
        };
        let index = |id: u64| self.tabs.iter().position(|t| t.id == id);
        let (Some(a), Some(b)) = (index(c.older), index(c.newer)) else {
            self.comparison = None;
            return;
        };
        if self.home.visible || self.active != b {
            self.comparison = None;
            return;
        }
        if c.result.is_none() && c.job.is_none() {
            if let Some((tab, error)) = [a, b].into_iter().find_map(|i| {
                self.tabs[i]
                    .error
                    .as_ref()
                    .map(|e| (self.tabs[i].name(), e))
            }) {
                c.result = Some(Err(format!("{tab} could not be opened. {error}")));
            } else if [a, b].iter().all(|&i| self.tabs[i].index_done) {
                let texts = |tab: &Tab| -> Vec<PageText> {
                    tab.text
                        .iter()
                        .map(|t| t.as_deref().cloned().unwrap_or_default())
                        .collect()
                };
                let (old, new) = (texts(&self.tabs[a]), texts(&self.tabs[b]));
                c.limited = [a, b].iter().any(|&i| self.tabs[i].search_limited)
                    || old.iter().chain(&new).any(|t| t.limited);
                let (tx, rx) = mpsc::channel();
                let wake = ctx.clone();
                std::thread::spawn(move || {
                    let result =
                        std::panic::catch_unwind(|| mypdf_milestone0::compare::compare(&old, &new))
                            .unwrap_or_else(|_| {
                                Err("MyPDF could not compare these documents.".into())
                            });
                    let _ = tx.send(result);
                    wake.request_repaint();
                });
                c.job = Some(rx);
            }
        }
        if let Some(job) = &c.job {
            match job.try_recv() {
                Ok(result) => {
                    c.job = None;
                    let first = result.as_ref().is_ok_and(|r| !r.is_empty());
                    c.result = Some(result.map(Arc::new));
                    if first {
                        c.select(0);
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    c.job = None;
                    c.result = Some(Err("MyPDF could not compare these documents.".into()));
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
    }

    pub(super) fn compare_screen(&mut self, ctx: &egui::Context) {
        let Some(c) = &mut self.comparison else {
            return;
        };
        let changes = c.changes();
        if !ctx.wants_keyboard_input() {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)) {
                c.step(true);
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)) {
                c.step(false);
            }
        }
        let names = [c.older, c.newer].map(|id| {
            self.tabs
                .iter()
                .find(|t| t.id == id)
                .map(|t| t.name())
                .unwrap_or_default()
        });
        let (mut done, mut swap) = (false, false);
        egui::TopBottomPanel::top("compare_bar")
            .frame(
                egui::Frame::new()
                    .fill(theme::toolbar())
                    .inner_margin(egui::Margin::symmetric(12, 6)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(20., 20.), egui::Sense::hover());
                    ui::draw_icon(ui.painter(), r, Icon::Compare, theme::accent());
                    ui.label(
                        RichText::new("Compare")
                            .font(ui::bold(ui, 14.))
                            .color(theme::ink()),
                    );
                    ui.add_space(6.);
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("{}  →  {}", names[0], names[1]))
                                .size(12.5)
                                .color(theme::muted()),
                        )
                        .truncate(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 4.;
                        done = ui
                            .add(ActionButton::new(Icon::Check, "Done").primary())
                            .clicked();
                        swap = ui
                            .add(ActionButton::new(Icon::Swap, "Swap sides"))
                            .on_hover_text("Show the newer version on the left")
                            .clicked();
                        ui.add_space(10.);
                        let any = !changes.is_empty();
                        if ui
                            .add_enabled(
                                any,
                                ActionButton::new(Icon::Right, "Next change").compact(),
                            )
                            .on_hover_text("Next change · ↓")
                            .clicked()
                        {
                            c.step(true);
                        }
                        let position = match c.selected {
                            Some(i) if any => format!("{} of {}", i + 1, changes.len()),
                            _ => format!("{} changes", changes.len()),
                        };
                        ui.label(RichText::new(position).size(12.5).color(theme::muted()));
                        if ui
                            .add_enabled(
                                any,
                                ActionButton::new(Icon::Left, "Previous change").compact(),
                            )
                            .on_hover_text("Previous change · ↑")
                            .clicked()
                        {
                            c.step(false);
                        }
                    });
                });
            });
        if self.inspector {
            self.change_list(ctx, &changes);
        }
        let Some(c) = &mut self.comparison else {
            return;
        };
        let reveal = std::mem::take(&mut c.reveal);
        let (older, newer, selected, result) = (c.older, c.newer, c.selected, c.result.clone());
        let mut clicked = None;
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::canvas())
                    .inner_margin(egui::Margin::symmetric(0, 8)),
            )
            .show(ctx, |ui| {
                match &result {
                    None => {
                        ui.centered_and_justified(|ui| {
                            ui.vertical_centered(|ui| {
                                ui.add_space(ui.available_height() * 0.4);
                                ui.spinner();
                                ui.label(
                                    RichText::new("Reading both versions…").color(theme::muted()),
                                );
                            });
                        });
                        return;
                    }
                    Some(Err(message)) => {
                        ui.vertical_centered(|ui| {
                            ui.add_space(ui.available_height() * 0.3);
                            ui.set_max_width(420.);
                            ui::callout(ui, ui::Tone::Warning, message);
                        });
                        return;
                    }
                    Some(Ok(_)) => {}
                }
                let index = |id: u64| self.tabs.iter().position(|t| t.id == id);
                let (Some(a), Some(b)) = (index(older), index(newer)) else {
                    return;
                };
                let full = ui.available_rect_before_wrap();
                let half = (full.width() - 1.) / 2.;
                let left = Rect::from_min_size(full.min, vec2(half, full.height()));
                let right =
                    Rect::from_min_size(full.min + vec2(half + 1., 0.), vec2(half, full.height()));
                ui.painter().vline(
                    full.center().x,
                    full.y_range(),
                    Stroke::new(1., theme::line()),
                );
                let tick = self.tick;
                for (rect, i, side) in [(left, a, Side::Older), (right, b, Side::Newer)] {
                    let mut column_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(rect)
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );
                    if let Some(index) = column(
                        &mut column_ui,
                        &mut self.tabs[i],
                        side,
                        &changes,
                        selected,
                        reveal,
                        tick,
                    ) {
                        clicked = Some(index);
                    }
                }
                ui.allocate_rect(full, egui::Sense::hover());
            });
        if let Some(c) = &mut self.comparison
            && let Some(index) = clicked
        {
            c.select(index);
        }
        if swap && let Some(c) = &mut self.comparison {
            let (older, newer) = (c.newer, c.older);
            *c = Comparison::new(older, newer);
            if let Some(index) = self.tabs.iter().position(|t| t.id == newer) {
                self.active = index;
            }
        }
        if done {
            self.comparison = None;
        }
    }

    fn change_list(&mut self, ctx: &egui::Context, changes: &[Change]) {
        let Some(c) = &mut self.comparison else {
            return;
        };
        let reveal = c.reveal;
        let limited = c.limited;
        let ready = matches!(c.result, Some(Ok(_)));
        egui::SidePanel::right("compare_changes")
            .exact_width(300.)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::paper())
                    .inner_margin(egui::Margin::symmetric(14, 12)),
            )
            .show(ctx, |ui| {
                ui::title(ui, "Changes", 15.);
                if !ready {
                    ui::caption(ui, "The list appears when both versions are read.");
                    return;
                }
                let count = |kind| changes.iter().filter(|c| c.kind == kind).count();
                ui.horizontal_wrapped(|ui| {
                    for kind in [Kind::Changed, Kind::Added, Kind::Removed] {
                        pill(ui, &format!("{} {}", count(kind), label(kind).to_lowercase()), color(kind));
                    }
                });
                if limited {
                    ui.add_space(6.);
                    ui::callout(ui, ui::Tone::Warning, "Some pages have more text than MyPDF reads, so changes there may be missed.");
                }
                ui.add_space(8.);
                if changes.is_empty() {
                    ui::callout(ui, ui::Tone::Success, "No differences in the text: both versions have the same words in the same order.");
                }
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for (index, change) in changes.iter().enumerate() {
                        let chosen = c.selected == Some(index);
                        let response = egui::Frame::new()
                            .fill(if chosen { theme::accent_soft() } else { theme::surface() })
                            .stroke(Stroke::new(1., if chosen { theme::accent_line() } else { theme::line() }))
                            .corner_radius(9)
                            .inner_margin(egui::Margin::symmetric(10, 8))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    pill(ui, label(change.kind), color(change.kind));
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.label(RichText::new(format!("Page {}", change.page() + 1)).size(11.5).color(theme::muted()));
                                    });
                                });
                                let (old, new) = (change.old_text(), change.new_text());
                                if !old.is_empty() {
                                    ui.add(egui::Label::new(RichText::new(shorten(&old, 160)).size(12.5).strikethrough().color(theme::danger())).wrap());
                                }
                                if !new.is_empty() {
                                    ui.add(egui::Label::new(RichText::new(shorten(&new, 160)).size(12.5).color(theme::ink())).wrap());
                                }
                            })
                            .response
                            .interact(egui::Sense::click())
                            .on_hover_cursor(egui::CursorIcon::PointingHand);
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{} on page {}", label(change.kind), change.page() + 1))
                        });
                        if chosen && reveal {
                            response.scroll_to_me(Some(egui::Align::Center));
                        }
                        if response.clicked() {
                            c.select(index);
                        }
                        ui.add_space(5.);
                    }
                    ui.add_space(4.);
                    ui.label(
                        RichText::new("Only text is compared; pictures and formatting are not.")
                            .size(11.5)
                            .color(theme::muted()),
                    );
                });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;

    #[test]
    fn two_versions_are_compared_side_by_side_and_changes_can_be_stepped() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let older =
            "Payment is due in thirty days.\nThe seller insures the goods.\nDisputes go to Ankara.";
        let newer =
            "Payment is due in sixty days.\nDisputes go to Ankara.\nThis starts on signing.";
        for (name, text) in [("Old.pdf", older), ("New.pdf", newer)] {
            let pdf = mypdf_milestone0::convert::text_to_pdf(text, false).unwrap();
            app.open_memory(PathBuf::from(name), pdf, &ctx);
        }
        let ids = [app.tabs[0].id, app.tabs[1].id];
        let frame = |app: &mut Viewer| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1300., 860.))),
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
        };
        // Choosing: the open document is the newer version, the other one older.
        app.request_compare();
        let chooser = app.compare_chooser.as_ref().unwrap();
        assert_eq!(chooser.newer, Some(Pick::Tab(ids[1])));
        assert_eq!(chooser.older, Some(Pick::Tab(ids[0])));
        app.compare_chooser = None;
        app.start_compare(Pick::Tab(ids[0]), Pick::Tab(ids[1]), &ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while app.comparison.as_ref().is_some_and(|c| c.result.is_none()) {
            assert!(std::time::Instant::now() < deadline);
            frame(&mut app);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let c = app.comparison.as_ref().unwrap();
        let changes = c.changes();
        let kinds: Vec<Kind> = changes.iter().map(|c| c.kind).collect();
        assert_eq!(kinds, [Kind::Changed, Kind::Removed, Kind::Added]);
        assert_eq!(c.selected, Some(0), "the first change is shown");
        // Down and up step through them, wrapping around.
        let key = |app: &mut Viewer, key| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1300., 860.))),
                    events: [true, false]
                        .map(|pressed| egui::Event::Key {
                            key,
                            physical_key: None,
                            pressed,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        })
                        .to_vec(),
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
        };
        key(&mut app, egui::Key::ArrowDown);
        assert_eq!(app.comparison.as_ref().unwrap().selected, Some(1));
        key(&mut app, egui::Key::ArrowUp);
        key(&mut app, egui::Key::ArrowUp);
        assert_eq!(app.comparison.as_ref().unwrap().selected, Some(2));
        // Choosing another document ends the comparison; the tabs stay.
        app.active = 0;
        frame(&mut app);
        assert!(app.comparison.is_none());
        assert_eq!(app.tabs.len(), 2);
        // Swapping sides compares the other way round.
        app.start_compare(Pick::Tab(ids[1]), Pick::Tab(ids[0]), &ctx);
        while app.comparison.as_ref().is_some_and(|c| c.result.is_none()) {
            assert!(std::time::Instant::now() < deadline);
            frame(&mut app);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let kinds: Vec<Kind> = app
            .comparison
            .as_ref()
            .unwrap()
            .changes()
            .iter()
            .map(|c| c.kind)
            .collect();
        assert_eq!(kinds, [Kind::Changed, Kind::Added, Kind::Removed]);
    }
}
