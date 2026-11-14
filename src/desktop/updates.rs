//! Non-blocking update checks and offline release notes.
use super::*;
use mypdf_milestone0::updates::{self as release, Channel, Report};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Preferences {
    automatic: bool,
    last_check: u64,
    seen_notes: String,
    dismissed: String,
    cached: Option<Report>,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn due(last: u64, now: u64) -> bool {
    last == 0 || now < last || now - last >= 86_400
}
fn notes_id() -> String {
    use sha2::{Digest, Sha256};
    format!(
        "{}:{:x}",
        env!("CARGO_PKG_VERSION"),
        Sha256::digest(release::CHANGELOG.as_bytes())
    )
}
fn notice_key(report: &Report) -> String {
    format!(
        "{:?}:{}:{}",
        report.channel,
        report.installed,
        report
            .available
            .as_ref()
            .and_then(|a| a.version.as_deref())
            .unwrap_or("store-update")
    )
}
pub(super) struct State {
    pub open: bool,
    notes: bool,
    prefs: Preferences,
    root: Option<PathBuf>,
    pending: Option<mpsc::Receiver<Result<Report, String>>>,
    report: Option<Report>,
    error: Option<String>,
    preference_error: Option<String>,
    cached: bool,
    notification: bool,
    background: bool,
    next_tick: Instant,
    window_handle: Option<isize>,
}
impl Default for State {
    fn default() -> Self {
        let root = if cfg!(test) {
            None
        } else {
            Some(mypdf_milestone0::assets::data_directory())
        };
        let loaded = root
            .as_ref()
            .map(|p| preferences::load::<Preferences>(p, "updates.json"));
        let preference_error = loaded
            .as_ref()
            .and_then(|r| r.as_ref().err())
            .map(|e| format!("Could not load update preferences: {e}"));
        let mut prefs = loaded.and_then(Result::ok).unwrap_or_default();
        if prefs.cached.as_ref().is_some_and(|r| {
            release::Version::parse(&r.installed)
                != release::Version::parse(env!("CARGO_PKG_VERSION"))
                || r.available
                    .as_ref()
                    .is_some_and(|a| !release::valid_destination(r.channel, &a.url))
        }) {
            prefs.cached = None;
        }
        Self {
            open: !cfg!(test) && prefs.seen_notes != notes_id(),
            notes: true,
            report: prefs.cached.clone(),
            prefs,
            root,
            pending: None,
            error: None,
            preference_error,
            cached: true,
            notification: false,
            background: false,
            next_tick: Instant::now(),
            window_handle: None,
        }
    }
}
impl State {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        #[cfg(target_os = "windows")]
        let window_handle = {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            _cc.window_handle().ok().and_then(|h| match h.as_raw() {
                RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
                _ => None,
            })
        };
        #[cfg(not(target_os = "windows"))]
        let window_handle = None;
        Self {
            window_handle,
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(super) fn preview_available(&mut self, channel: Channel) {
        self.show(false);
        self.cached = false;
        self.report = Some(Report { channel, installed: env!("CARGO_PKG_VERSION").into(), available: Some(release::Available {
            version: (channel != Channel::Store).then(|| "2.4.3".into()),
            notes: "Improved image editing and page sizing.\n\n• Resize individual pages.\n• Drag image handles at the page edges.".into(),
            url: channel.url().into(),
        }) });
    }
    fn begin_install(&mut self, _ctx: &egui::Context, _background: bool) {
        #[cfg(target_os = "macos")]
        if let Err(error) = super::mac_updater::check(_ctx, _background) {
            self.error = Some(error);
        }
    }
    fn persist(&mut self) {
        if let Some(root) = &self.root {
            self.preference_error = preferences::save(root, "updates.json", &self.prefs)
                .err()
                .map(|e| format!("Could not save update preferences: {e}"));
        }
    }
    pub fn show(&mut self, notes: bool) {
        self.open = true;
        self.notes = notes;
        self.notification = false;
    }
    fn start(&mut self, ctx: &egui::Context, background: bool) {
        if self.pending.is_some() {
            return;
        }
        self.prefs.last_check = now();
        self.persist();
        self.error = None;
        self.background = background;
        let pending = match release::prepare(self.window_handle) {
            Ok(pending) => pending,
            Err(error) => {
                self.accept(Err(format!("{error:#}")));
                return;
            }
        };
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                release::check_prepared(pending)
            }))
            .map_err(|_| "The update check could not finish. Please try again.".to_owned())
            .and_then(|r| r.map_err(|e| format!("{e:#}")));
            let _ = tx.send(result);
            wake.request_repaint();
        });
    }
    fn accept(&mut self, result: Result<Report, String>) {
        match result {
            Ok(report) => {
                self.notification = self.background
                    && report.available.is_some()
                    && self.prefs.dismissed != notice_key(&report);
                self.prefs.cached = Some(report.clone());
                self.report = Some(report);
                self.cached = false;
                self.error = None;
            }
            Err(error) => {
                self.error = Some(error);
                self.cached = self.report.is_some();
            }
        }
        self.persist();
    }
    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(result) = self.pending.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(r) => Some(r),
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("Update checker stopped. Please try again.".into()))
            }
            Err(mpsc::TryRecvError::Empty) => None,
        }) {
            self.pending = None;
            let native_update = !cfg!(test)
                && result.as_ref().is_ok_and(|r| {
                    matches!(r.channel, Channel::MacArm | Channel::MacIntel)
                        && r.available.is_some()
                });
            self.accept(result);
            if native_update {
                self.notification = false;
                self.begin_install(ctx, self.background);
            }
        }
        if self.next_tick <= Instant::now() {
            self.next_tick = Instant::now() + Duration::from_secs(60);
            if !cfg!(test) && self.prefs.automatic && due(self.prefs.last_check, now()) {
                self.start(ctx, true);
            }
        }
        if self.prefs.automatic {
            ctx.request_repaint_after(Duration::from_secs(60));
        }
    }
    fn window(&mut self, ctx: &egui::Context, unsaved: bool) {
        self.poll(ctx);
        if self.notification && !self.open {
            egui::Area::new(egui::Id::new("update_notification"))
                .anchor(egui::Align2::RIGHT_BOTTOM, vec2(-20., -60.))
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("MyPDF update available");
                            if ui.button("View").clicked() {
                                self.show(false);
                            }
                            if ui.button("Later").clicked() {
                                self.notification = false;
                                if let Some(r) = &self.report {
                                    self.prefs.dismissed = notice_key(r);
                                }
                                self.persist();
                            }
                        });
                    });
                });
        }
        if !self.open {
            return;
        }
        let mut open = true;
        let mut close = false;
        let mut check = false;
        let mut changed = false;
        let width = ui::dialog_width(ctx, 620.);
        let mut install = false;
        egui::Window::new("MyPDF updates")
            .open(&mut open).collapsible(false).resizable(false)
            .default_width(width).default_pos(ctx.content_rect().center() - vec2(width / 2., 240.))
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.add_space(8.);
                ui.horizontal_top(|ui| {
                    let (mark, _) = ui.allocate_exact_size(vec2(64., 64.), egui::Sense::hover());
                    ui::logo(ui.painter(), mark);
                    ui.add_space(12.);
                    ui.vertical(|ui| {
                        let title = if self.notes { "What's New in MyPDF" }
                            else if self.pending.is_some() { "Checking for updates…" }
                            else if self.error.is_some() { "Couldn't check for updates" }
                            else if self.report.as_ref().is_some_and(|r| r.available.is_some()) { "A new version of MyPDF is available!" }
                            else if self.report.is_some() { "You're up to date" }
                            else { "Keep MyPDF up to date" };
                        ui.label(RichText::new(title).size(21.).strong());
                        ui.add_space(6.);
                        ui.label(format!("You have MyPDF {}.", self.report.as_ref().map_or(env!("CARGO_PKG_VERSION"), |r| r.installed.as_str())));
                        if let Some(report) = &self.report && !self.notes {
                            ui.weak(format!("Updates from {}", report.channel.label()));
                        }
                    });
                });
                ui.add_space(16.);
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.notes, true, "What's New");
                    ui.selectable_value(&mut self.notes, false, "Check for Updates");
                });
                ui.add_space(8.);
                egui::Frame::group(ui.style()).inner_margin(16.).corner_radius(10).show(ui, |ui| {
                    ui.set_min_width((width - 34.).max(120.));
                    egui::ScrollArea::vertical().id_salt("update_release_notes")
                        .max_height((ctx.content_rect().height() - 320.).clamp(100., 300.))
                        .min_scrolled_height(140.).show(ui, |ui| {
                        ui.set_min_height(180.);
                        if self.notes {
                            let log = release::changelog();
                            ui.heading(format!("Version {}", env!("CARGO_PKG_VERSION")));
                            ui.add_space(12.);
                            for item in log.unreleased { ui.label(format!("• {item}")); ui.add_space(4.); }
                            if let Some(notes) = log.releases.into_iter().find(|n| n.version == env!("CARGO_PKG_VERSION")) {
                                for item in notes.changes { ui.label(format!("• {item}")); ui.add_space(4.); }
                            }
                        } else {
                            if self.pending.is_some() { ui.horizontal(|ui| { ui.spinner(); ui.label("Looking for the latest version…"); }); }
                            if let Some(error) = &self.error { ui.colored_label(theme::danger(), error); }
                            if let Some(report) = &self.report {
                                if self.cached { ui.weak("Last successful check · check again to refresh"); }
                                if let Some(update) = &report.available {
                                    ui.heading(update.version.as_ref().map_or("Available update".into(), |v| format!("Version {v}")));
                                    ui.add_space(12.);
                                    for line in update.notes.lines() {
                                        if line.starts_with('#') { ui.strong(line.trim_start_matches('#').trim()); }
                                        else { ui.label(line); }
                                    }
                                } else { ui.label("You have the latest published version available for your installation."); }
                            } else if self.pending.is_none() && self.error.is_none() {
                                ui.label("Check for new features, improvements, and fixes.");
                                ui.add_space(8.);
                                ui.weak("Mac updates come from GitHub. Windows Store installations update through Microsoft Store.");
                            }
                        }
                        ui.add_space(16.);
                        ui.hyperlink_to("Release history", release::RELEASES_URL);
                    });
                });
                ui.add_space(12.);
                changed |= ui.checkbox(&mut self.prefs.automatic, "Automatically check for updates").changed();
                if let Some(error) = &self.preference_error { ui.colored_label(theme::danger(), error); }
                if !self.notes && unsaved && self.report.as_ref().is_some_and(|r| r.channel == Channel::Store && r.available.is_some()) {
                    ui.weak("Finish editing and save your open documents before updating through Microsoft Store.");
                }
                ui.add_space(12.);
                ui.horizontal_wrapped(|ui| {
                    close = ui.add_sized([90., 34.], egui::Button::new("Close")).clicked();
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !self.notes && let Some(report) = &self.report && let Some(update) = &report.available {
                            let store = report.channel == Channel::Store;
                            let mac = matches!(report.channel, Channel::MacArm | Channel::MacIntel);
                            let label = if store { "Update through Microsoft Store" } else if mac { "Download and Install…" } else { "Download Windows update" };
                            if ui.add_enabled(!store || !unsaved, ui::ActionButton::new(Icon::Export, label).primary()).clicked() {
                                if mac { install = true; } else { ctx.open_url(egui::OpenUrl::new_tab(&update.url)); }
                            }
                        }
                        check = ui.add_enabled(self.pending.is_none(), ui::ActionButton::new(Icon::Redo, "Check now")).clicked();
                    });
                });
                ui.add_space(4.);
            });
        open &= !close;
        if install {
            self.begin_install(ctx, false);
        }
        if changed {
            self.persist();
            self.next_tick = Instant::now();
        }
        if check {
            self.notes = false;
            self.start(ctx, false);
        }
        if !open {
            self.prefs.seen_notes = notes_id();
            self.persist();
        }
        self.open = open;
    }
}
impl Viewer {
    pub(super) fn update_action(&mut self, action: &str, ctx: &egui::Context) -> bool {
        match action {
            "whats_new" => self.updates.show(true),
            "check_updates" => {
                self.updates.show(false);
                self.updates.start(ctx, false);
            }
            _ => return false,
        }
        true
    }
    pub(super) fn updates_window(&mut self, ctx: &egui::Context) {
        #[cfg(target_os = "macos")]
        if super::mac_updater::take_quit_request() {
            self.request_close(None, ctx);
        }
        let unsaved = self.tabs.iter().any(|t| {
            t.session.as_ref().is_some_and(|s| s.dirty())
                || self.content.has_pending_text_change(t.id)
                || self.pending_form_edits(t.id)
                || self.table.has_editor(t.id)
        }) || self.content.busy
            || self.organizer.busy;
        self.updates.window(ctx, unsaved);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn close_button_dismisses_and_marks_notes_seen() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut state = State::default();
        state.show(true);
        let frame = |state: &mut State, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1000., 850.))),
                    events,
                    ..Default::default()
                },
                |ctx| state.window(ctx, false),
            )
        };
        for notes in [true, false] {
            state.show(notes);
            // Release-note wrapping can change the initial window height; let
            // egui constrain it to the viewport before locating the button.
            for _ in 0..6 {
                frame(&mut state, vec![]);
            }
            let output = frame(&mut state, vec![]);
            let nodes = output.platform_output.accesskit_update.unwrap().nodes;
            let r = nodes
                .iter()
                .find(|(_, n)| n.label() == Some("Close"))
                .unwrap()
                .1
                .bounds()
                .unwrap();
            let pos = pos2(((r.x0 + r.x1) / 2.) as f32, ((r.y0 + r.y1) / 2.) as f32);
            for pressed in [true, false] {
                frame(
                    &mut state,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            assert!(!state.open, "Close must dismiss either tab");
            assert_eq!(state.prefs.seen_notes, notes_id());
        }
    }
    #[test]
    fn daily_checks_and_failed_checks_keep_cached_information() {
        assert!(!due(1000, 1001));
        assert!(due(1000, 87400));
        assert!(due(1000, 900));
        assert!(due(0, 1));
        let mut state = State {
            background: true,
            ..State::default()
        };
        let report = Report {
            channel: Channel::MacArm,
            installed: "2.4.2".into(),
            available: Some(release::Available {
                version: Some("2.4.3".into()),
                notes: "Fix".into(),
                url: release::RELEASES_URL.into(),
            }),
        };
        state.accept(Ok(report.clone()));
        assert!(state.notification);
        state.prefs.dismissed = notice_key(&report);
        state.accept(Ok(report));
        assert!(!state.notification);
        state.accept(Err("Offline".into()));
        assert!(state.cached);
        assert!(state.report.unwrap().available.is_some());
    }
    #[test]
    fn notes_and_store_update_controls_work_without_a_document() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut state = State::default();
        state.show(true);
        let labels = |state: &mut State, dirty: bool| {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(900., 900.))),
                    ..Default::default()
                },
                |ctx| state.window(ctx, dirty),
            );
            output.platform_output.accesskit_update.unwrap().nodes
        };
        labels(&mut state, false);
        assert!(
            labels(&mut state, false)
                .iter()
                .any(|(_, n)| n.label() == Some("What's New"))
        );
        state.show(false);
        state.report = Some(Report {
            channel: Channel::Store,
            installed: "2.4.2.0".into(),
            available: Some(release::Available {
                version: None,
                notes: "Store update".into(),
                url: release::STORE_URL.into(),
            }),
        });
        for dirty in [true, false] {
            labels(&mut state, dirty);
            let nodes = labels(&mut state, dirty);
            let button = &nodes
                .iter()
                .find(|(_, n)| n.label() == Some("Update through Microsoft Store"))
                .unwrap()
                .1;
            assert_eq!(button.is_disabled(), dirty);
        }
    }
}
