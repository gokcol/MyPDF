//! Zen mode: only the pages, full screen. Toolbars and panels step aside; the
//! page number appears when the page changes, and a small bar with the way
//! out appears when the pointer reaches the top. Esc or ⇧⌘F leaves.
use super::*;
use std::time::{Duration, Instant};

/// How long the welcome bar and the page number stay before fading.
const HINT: Duration = Duration::from_millis(2500);
const PAGE: Duration = Duration::from_millis(1200);

pub(super) struct Zen {
    since: Instant,
    /// The window was full screen already, so leaving keeps it that way.
    was_fullscreen: bool,
    mode_before: editing::Mode,
    page: Option<(usize, Instant)>,
}

impl Viewer {
    pub(super) fn toggle_zen(&mut self, ctx: &egui::Context) {
        if let Some(zen) = self.zen.take() {
            self.editor.mode = zen.mode_before;
            if !zen.was_fullscreen {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            }
            return;
        }
        if self.home.visible
            || self
                .tabs
                .get(self.active)
                .is_none_or(|t| t.pages.is_empty())
        {
            self.status = Some(("Open a document to read it in Zen mode.".into(), true));
            return;
        }
        if !self.apply_pending_annotation() {
            return;
        }
        self.comparison = None;
        self.search = false;
        let tab = &mut self.tabs[self.active];
        tab.selected_annotation = None;
        tab.group.clear();
        self.editor.draft = None;
        let was_fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        if !was_fullscreen {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
        }
        self.zen = Some(Zen {
            since: Instant::now(),
            was_fullscreen,
            mode_before: std::mem::replace(&mut self.editor.mode, editing::Mode::Read),
            page: None,
        });
    }

    /// Keys while reading in Zen mode: Esc leaves, arrows and space turn pages.
    pub(super) fn zen_keys(&mut self, ctx: &egui::Context) {
        if self.zen.is_none() {
            return;
        }
        if self.home.visible || self.tabs.get(self.active).is_none() {
            self.toggle_zen(ctx);
            return;
        }
        let dialog = ctx.memory(|m| m.top_modal_layer().is_some()) || egui::Popup::is_any_open(ctx);
        if dialog || ctx.wants_keyboard_input() {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.toggle_zen(ctx);
            return;
        }
        let tab = &mut self.tabs[self.active];
        let forward = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::Space)
        });
        let back = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft)
                || i.consume_key(egui::Modifiers::SHIFT, egui::Key::Space)
        });
        if forward {
            tab.jump(tab.page + 1);
        } else if back {
            tab.jump(tab.page.saturating_sub(1));
        }
    }

    /// The bar with the way out, and the page number when it changes.
    pub(super) fn zen_overlay(&mut self, ctx: &egui::Context) {
        let Some(zen) = &mut self.zen else {
            return;
        };
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let (page, pages) = (tab.page, tab.pages.len());
        match zen.page {
            Some((shown, _)) if shown == page => {}
            // The first page needs no announcement.
            None => zen.page = Some((page, zen.since - PAGE)),
            Some(_) => zen.page = Some((page, Instant::now())),
        }
        let welcome = zen.since.elapsed() < HINT;
        let pointer_at_top = ctx
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|p| p.y < 70.);
        let bar_visible = welcome || pointer_at_top;
        let page_visible = zen.page.is_some_and(|(_, at)| at.elapsed() < PAGE);
        if welcome || page_visible {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        let bar = ctx.animate_bool_with_time(egui::Id::new("zen_bar"), bar_visible, 0.25);
        let number = ctx.animate_bool_with_time(egui::Id::new("zen_page"), page_visible, 0.3);
        let card = |ui: &mut egui::Ui, add: &mut dyn FnMut(&mut egui::Ui)| {
            egui::Frame::new()
                .fill(theme::surface().gamma_multiply(0.94))
                .stroke(Stroke::new(1., theme::line()))
                .corner_radius(20)
                .inner_margin(egui::Margin::symmetric(14, 7))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 6],
                    blur: 20,
                    spread: 0,
                    color: Color32::from_black_alpha(40),
                })
                .show(ui, |ui| ui.horizontal(|ui| add(ui)));
        };
        let mut leave = false;
        if bar > 0. {
            egui::Area::new(egui::Id::new("zen_exit"))
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_TOP, vec2(0., 14.))
                .show(ctx, |ui| {
                    ui.set_opacity(bar);
                    card(ui, &mut |ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(18., 18.), egui::Sense::hover());
                        ui::draw_icon(ui.painter(), r, Icon::Zen, theme::accent());
                        ui.label(
                            RichText::new("Zen mode")
                                .font(ui::strong(ui, 13.))
                                .color(theme::ink()),
                        );
                        ui.label(
                            RichText::new(format!("Page {} of {pages}  ·  Esc to leave", page + 1))
                                .size(12.5)
                                .color(theme::muted()),
                        );
                        ui.add_space(4.);
                        leave = ui
                            .add(ActionButton::new(Icon::Close, "Leave Zen mode").compact())
                            .clicked();
                    });
                });
        }
        if number > 0. && !bar_visible {
            egui::Area::new(egui::Id::new("zen_page_number"))
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_BOTTOM, vec2(0., -22.))
                .interactable(false)
                .show(ctx, |ui| {
                    ui.set_opacity(number);
                    card(ui, &mut |ui| {
                        ui.label(
                            RichText::new(format!("{} / {pages}", page + 1))
                                .font(ui::strong(ui, 13.))
                                .color(theme::ink()),
                        );
                    });
                });
        }
        if leave {
            self.toggle_zen(ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;

    #[test]
    fn zen_mode_shows_only_the_pages_and_esc_brings_everything_back() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let mut text = String::new();
        for i in 1..=120 {
            text.push_str(&format!("Line {i} of a long document to read quietly.\n"));
        }
        let pdf = mypdf_milestone0::convert::text_to_pdf(&text, false).unwrap();
        app.open_memory(PathBuf::from("Long read.pdf"), pdf, &ctx);
        // The labels of what is on screen after a frame.
        let frame = |app: &mut Viewer, events: Vec<egui::Event>| -> Vec<String> {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 820.))),
                    events,
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
            output
                .platform_output
                .accesskit_update
                .map(|u| u.nodes)
                .unwrap_or_default()
                .iter()
                .filter_map(|(_, n)| n.label().map(str::to_owned))
                .collect()
        };
        let press = |key: egui::Key, modifiers: egui::Modifiers| {
            [true, false]
                .map(|pressed| egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers,
                })
                .to_vec()
        };
        let deadline = Instant::now() + Duration::from_secs(20);
        while app.tabs[0].pages.len() < 2 {
            assert!(Instant::now() < deadline);
            frame(&mut app, vec![]);
            std::thread::sleep(Duration::from_millis(10));
        }
        app.editor.mode = editing::Mode::Annotate;
        let labels = frame(&mut app, vec![]);
        for chrome in ["Home", "Toggle left panel", "Outline", "Marks"] {
            assert!(
                labels.iter().any(|l| l == chrome),
                "{chrome} is shown: {labels:?}"
            );
        }
        frame(
            &mut app,
            press(
                egui::Key::F,
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            ),
        );
        assert!(app.zen.is_some(), "⇧⌘F enters Zen mode");
        assert!(!app.search, "and does not open Find");
        assert_eq!(app.editor.mode, editing::Mode::Read);
        let labels = frame(&mut app, vec![]);
        for chrome in ["Home", "Toggle left panel", "Outline", "Marks", "Annotate"] {
            assert!(!labels.iter().any(|l| l == chrome), "{chrome} steps aside");
        }
        assert!(
            labels.iter().any(|l| l == "Leave Zen mode"),
            "the way out shows at first"
        );
        let page = app.tabs[0].page;
        frame(
            &mut app,
            press(egui::Key::ArrowRight, egui::Modifiers::NONE),
        );
        assert_eq!(app.tabs[0].page, page + 1, "arrows turn pages");
        frame(&mut app, press(egui::Key::Escape, egui::Modifiers::NONE));
        assert!(app.zen.is_none(), "Esc leaves");
        assert_eq!(
            app.editor.mode,
            editing::Mode::Annotate,
            "the mode comes back"
        );
        let labels = frame(&mut app, vec![]);
        assert!(
            labels.iter().any(|l| l == "Toggle left panel"),
            "the toolbars are back"
        );
    }
}
