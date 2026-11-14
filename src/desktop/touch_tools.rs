//! Optional finger drawing and a compact, touch-sized drawing palette.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
enum InputMode {
    #[default]
    Automatic,
    Touch,
    Mouse,
}
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
enum Targets {
    Standard,
    #[default]
    Large,
    ExtraLarge,
}
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
enum HoldDelay {
    Quick,
    #[default]
    Normal,
    Slow,
}
impl HoldDelay {
    fn seconds(self) -> f64 {
        match self {
            Self::Quick => 0.35,
            Self::Normal => 0.55,
            Self::Slow => 0.85,
        }
    }
}
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
enum Smoothing {
    #[default]
    Off,
    Light,
    Strong,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct Preferences {
    draw_with_finger: bool,
    mode: InputMode,
    targets: Targets,
    hold_delay: HoldDelay,
    smoothing: Smoothing,
    gesture_hints: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            draw_with_finger: false,
            mode: InputMode::Automatic,
            targets: Targets::Large,
            hold_delay: HoldDelay::Normal,
            smoothing: Smoothing::Off,
            gesture_hints: true,
        }
    }
}
#[derive(Clone, Default)]
struct State {
    preferences: Preferences,
    seen_touch: bool,
    hardware_touch: bool,
    next_probe: f64,
    visible: bool,
    panel_open: bool,
    palette_revision: u64,
    error: Option<String>,
    load_failed: bool,
}
fn id() -> egui::Id {
    egui::Id::new("touch_tools")
}
fn state(ctx: &egui::Context) -> State {
    if let Some(state) = ctx.data(|d| d.get_temp::<State>(id())) {
        return state;
    }
    let mut state = State::default();
    #[cfg(not(test))]
    match preferences::load(&mypdf_milestone0::assets::data_directory(), "touch.json") {
        Ok(preferences) => state.preferences = preferences,
        Err(e) => {
            state.error = Some(format!("Could not load touch settings: {e}"));
            state.load_failed = true;
        }
    }
    #[cfg(test)]
    let _ = &mut state;
    ctx.data_mut(|d| d.insert_temp(id(), state.clone()));
    state
}
fn available(state: &State) -> bool {
    match state.preferences.mode {
        InputMode::Automatic => state.hardware_touch || state.seen_touch,
        InputMode::Touch => true,
        InputMode::Mouse => false,
    }
}
pub(super) fn controls_available(ctx: &egui::Context) -> bool {
    available(&state(ctx))
}
pub(super) fn mouse_only(ctx: &egui::Context) -> bool {
    state(ctx).preferences.mode == InputMode::Mouse
}
pub(super) fn draw_with_finger(ctx: &egui::Context) -> bool {
    let current = state(ctx);
    current.preferences.mode != InputMode::Mouse && current.preferences.draw_with_finger
}
fn update_preferences(ctx: &egui::Context, change: impl FnOnce(&mut Preferences)) {
    let mut current = state(ctx);
    if current.load_failed {
        return;
    }
    let mut preferences = current.preferences.clone();
    change(&mut preferences);
    #[cfg(not(test))]
    if let Err(e) = preferences::save(
        &mypdf_milestone0::assets::data_directory(),
        "touch.json",
        &preferences,
    ) {
        current.error = Some(format!("Could not save touch settings: {e}"));
        ctx.data_mut(|d| d.insert_temp(id(), current));
        return;
    }
    current.preferences = preferences;
    current.error = None;
    ctx.data_mut(|d| d.insert_temp(id(), current));
    ctx.request_repaint();
}
pub(super) fn set_finger(ctx: &egui::Context, enabled: bool) {
    update_preferences(ctx, |p| p.draw_with_finger = enabled);
}
pub(super) fn hold_delay(ctx: &egui::Context) -> f64 {
    state(ctx).preferences.hold_delay.seconds()
}
fn touchscreen_hardware() -> bool {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetSystemMetrics, NID_EXTERNAL_TOUCH, NID_INTEGRATED_TOUCH, NID_READY, SM_DIGITIZER,
        };
        // Touch digitizers only: a pen device or trackpad does not enable this UI.
        let flags = unsafe { GetSystemMetrics(SM_DIGITIZER) } as u32;
        flags & NID_READY != 0 && flags & (NID_INTEGRATED_TOUCH | NID_EXTERNAL_TOUCH) != 0
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    } // Native touch events provide the capability signal on other platforms.
}
pub(super) fn observe(ctx: &egui::Context) {
    let mut current = state(ctx);
    let time = ctx.input(|i| i.time);
    if time >= current.next_probe {
        current.hardware_touch = touchscreen_hardware();
        current.next_probe = time + 2.;
    }
    // Trackpads produce wheel/gesture events; only direct Touch identifies a screen.
    current.seen_touch |= ctx.input(|i| {
        i.events
            .iter()
            .any(|e| matches!(e, egui::Event::Touch { .. }))
    });
    let target = if available(&current) {
        match current.preferences.targets {
            Targets::Standard => 0.,
            Targets::Large => 44.,
            Targets::ExtraLarge => 52.,
        }
    } else {
        0.
    };
    if ctx.input(|i| !i.any_touches() && !i.pointer.any_down() && !i.pointer.any_released()) {
        theme::set_touch_target(target);
    }
    #[cfg(target_os = "windows")]
    ctx.request_repaint_after(std::time::Duration::from_secs(2));
    ctx.data_mut(|d| d.insert_temp(id(), current));
}
pub(super) fn smooth_stroke(ctx: &egui::Context, raw: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let passes = match state(ctx).preferences.smoothing {
        Smoothing::Off => 0,
        Smoothing::Light => 1,
        Smoothing::Strong => 3,
    };
    let mut points = raw.to_vec();
    if points.len() < 3 {
        return points;
    }
    for _ in 0..passes {
        let previous = points.clone();
        for i in 1..points.len() - 1 {
            for axis in 0..2 {
                points[i][axis] = previous[i - 1][axis] * 0.25
                    + previous[i][axis] * 0.5
                    + previous[i + 1][axis] * 0.25;
            }
        }
    }
    points
}
fn preference_section(
    ui: &mut egui::Ui,
    global: bool,
    icon: Icon,
    title: &str,
    body: impl FnOnce(&mut egui::Ui),
) {
    if global {
        ui::settings_group(ui, icon, title, body);
    } else {
        ui::section(ui, title, body);
    }
}

#[derive(Clone, Copy)]
enum PreferenceSection {
    Finger,
    Targets,
    Drawing,
    Gestures,
    Toolbar,
}

fn preference_controls(ui: &mut egui::Ui, drawing: bool, global: bool) {
    use PreferenceSection::*;
    if global && ui.available_width() >= 580. {
        for pair in [[Finger, Targets], [Drawing, Gestures]] {
            ui.columns(2, |columns| {
                for (column, section) in columns.iter_mut().zip(pair) {
                    preference_control(column, drawing, global, section);
                }
            });
        }
        preference_control(ui, drawing, global, Toolbar);
    } else {
        for section in [Finger, Targets, Drawing, Gestures, Toolbar] {
            preference_control(ui, drawing, global, section);
        }
    }
    if let Some(error) = state(ui.ctx()).error {
        ui.colored_label(theme::danger(), error);
    }
}

fn preference_control(ui: &mut egui::Ui, drawing: bool, global: bool, section: PreferenceSection) {
    let ctx = ui.ctx().clone();
    let before = state(&ctx).preferences;
    match section {
        PreferenceSection::Finger => {
            let mut finger = before.draw_with_finger;
            if drawing || global {
                preference_section(ui, global, Icon::Pen, "Finger drawing", |ui| {
                    if ui.checkbox(&mut finger, "Draw with finger").changed() {
                        set_finger(&ctx, finger);
                    }
                    ui.label(
                        RichText::new(
                            "In Draw mode. When off, one finger pans; mouse drawing is unchanged.",
                        )
                        .small()
                        .color(theme::muted()),
                    );
                });
            }
        }
        PreferenceSection::Targets => {
            preference_section(ui, global, Icon::Touch, "Touch targets", |ui| {
                let mut value = before.targets;
                egui::ComboBox::from_id_salt("touch_target_size")
                    .selected_text(match value {
                        Targets::Standard => "Standard",
                        Targets::Large => "Large · 44 pt",
                        Targets::ExtraLarge => "Extra large · 52 pt",
                    })
                    .show_ui(ui, |ui| {
                        for (v, label) in [
                            (Targets::Standard, "Standard"),
                            (Targets::Large, "Large · 44 pt"),
                            (Targets::ExtraLarge, "Extra large · 52 pt"),
                        ] {
                            ui.selectable_value(&mut value, v, label);
                        }
                    });
                if value != before.targets {
                    update_preferences(&ctx, |p| p.targets = value);
                }
                ui.label(
                    RichText::new("Larger buttons and object handles while touch mode is active.")
                        .small()
                        .color(theme::muted()),
                );
            });
        }
        PreferenceSection::Drawing => {
            preference_section(ui, global, Icon::Shapes, "Drawing", |ui| {
                ui.label("Finger stroke smoothing");
                let mut value = before.smoothing;
                egui::ComboBox::from_id_salt("touch_smoothing")
                    .selected_text(match value {
                        Smoothing::Off => "Off",
                        Smoothing::Light => "Light",
                        Smoothing::Strong => "Strong",
                    })
                    .show_ui(ui, |ui| {
                        for (v, label) in [
                            (Smoothing::Off, "Off"),
                            (Smoothing::Light, "Light"),
                            (Smoothing::Strong, "Strong"),
                        ] {
                            ui.selectable_value(&mut value, v, label);
                        }
                    });
                if value != before.smoothing {
                    update_preferences(&ctx, |p| p.smoothing = value);
                }
                ui.label(
                    RichText::new("Steadies new finger pen strokes. Mouse strokes stay unchanged.")
                        .small()
                        .color(theme::muted()),
                );
            });
        }
        PreferenceSection::Gestures => {
            preference_section(ui, global, Icon::History, "Gestures", |ui| {
                ui.label("Long-press delay");
                let mut value = before.hold_delay;
                egui::ComboBox::from_id_salt("touch_hold_delay")
                    .selected_text(match value {
                        HoldDelay::Quick => "Quick · 0.35 s",
                        HoldDelay::Normal => "Normal · 0.55 s",
                        HoldDelay::Slow => "Slow · 0.85 s",
                    })
                    .show_ui(ui, |ui| {
                        for (v, label) in [
                            (HoldDelay::Quick, "Quick · 0.35 s"),
                            (HoldDelay::Normal, "Normal · 0.55 s"),
                            (HoldDelay::Slow, "Slow · 0.85 s"),
                        ] {
                            ui.selectable_value(&mut value, v, label);
                        }
                    });
                if value != before.hold_delay {
                    update_preferences(&ctx, |p| p.hold_delay = value);
                }
                let mut hints = before.gesture_hints;
                if ui.checkbox(&mut hints, "Show gesture hints").changed() {
                    update_preferences(&ctx, |p| p.gesture_hints = hints);
                }
                if hints {
                    ui.label(
                        RichText::new("Two fingers: pan and pinch to zoom.")
                            .small()
                            .color(theme::muted()),
                    );
                    if drawing || global {
                        ui.label(
                            RichText::new(
                                "With finger drawing off, hold still to open the page menu.",
                            )
                            .small()
                            .color(theme::muted()),
                        );
                    } else {
                        ui.label(
                            RichText::new("One finger uses the selected annotation tool.")
                                .small()
                                .color(theme::muted()),
                        );
                    }
                }
            });
        }
        PreferenceSection::Toolbar => {
            preference_section(ui, global, Icon::Form, "Floating toolbar", |ui| {
                let controls = |ui: &mut egui::Ui| {
                    let mut current = state(&ctx);
                    if ui
                        .checkbox(&mut current.visible, "Show floating toolbar")
                        .changed()
                    {
                        ctx.data_mut(|d| d.insert_temp(id(), current.clone()));
                    }
                    if ui
                        .add(ActionButton::new(Icon::Reload, "Reset position"))
                        .clicked()
                    {
                        current.palette_revision = current.palette_revision.wrapping_add(1);
                        current.visible = true;
                        ctx.data_mut(|d| d.insert_temp(id(), current));
                    }
                };
                if global {
                    ui.horizontal_wrapped(controls);
                } else {
                    controls(ui);
                }
                ui.label(
                    RichText::new("Available in Draw and Annotate. Drag its title to move it.")
                        .small()
                        .color(theme::muted()),
                );
            });
        }
    }
}
pub(super) fn settings(ui: &mut egui::Ui) {
    let current = state(ui.ctx());
    ui::settings_group(ui, Icon::Touch, "Input mode", |ui| {
        let index = match current.preferences.mode {
            InputMode::Automatic => 0,
            InputMode::Touch => 1,
            InputMode::Mouse => 2,
        };
        if let Some(i) = ui::segmented(ui, &["Automatic", "Touch", "Mouse only"], index) {
            update_preferences(ui.ctx(), |p| {
                p.mode = [InputMode::Automatic, InputMode::Touch, InputMode::Mouse][i]
            });
        }
        ui.label(RichText::new(match state(ui.ctx()).preferences.mode {
            InputMode::Automatic if current.hardware_touch||current.seen_touch=>"Touchscreen detected. Touch controls are available.",
            InputMode::Automatic=>"Touch controls appear when a touchscreen is detected or used.",
            InputMode::Touch=>"Always show touch controls, including on devices that report no touchscreen.",
            InputMode::Mouse=>"Hide touch controls and disable finger drawing. Mouse and trackpad navigation stay available.",
        }).small().color(theme::muted()));
    });
    ui.add_enabled_ui(state(ui.ctx()).preferences.mode != InputMode::Mouse, |ui| {
        preference_controls(ui, false, true)
    });
}
impl Viewer {
    pub(super) fn touch_settings_button(&mut self, ui: &mut egui::Ui) {
        let mut current = state(ui.ctx());
        let selected = current.panel_open && self.inspector && !self.translation.open;
        if ui
            .add(
                ActionButton::new(Icon::Touch, "Touch settings")
                    .compact()
                    .selected(selected),
            )
            .on_hover_text("Touch settings for this mode")
            .clicked()
        {
            current.panel_open = !selected;
            if current.panel_open {
                self.inspector = true;
                self.translation.open = false;
            }
            ui.ctx().data_mut(|d| d.insert_temp(id(), current));
        }
    }
    pub(super) fn touch_settings_panel(&mut self, ctx: &egui::Context) -> bool {
        if !controls_available(ctx)
            || !state(ctx).panel_open
            || !matches!(
                self.editor.mode,
                editing::Mode::Draw | editing::Mode::Annotate
            )
        {
            return false;
        }
        let drawing = self.editor.mode == editing::Mode::Draw;
        let mut close = false;
        egui::SidePanel::right("annotation_inspector")
            .default_width(254.)
            .width_range(230. ..=340.)
            .resizable(true)
            .frame(ui::property_panel_frame())
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = vec2(8., 12.);
                ui::property_scroll(ui, "touch_settings", |ui| {
                    ui.horizontal(|ui| {
                        ui::title(ui, "Touch", 17.);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            close = ui
                                .add(
                                    ActionButton::new(Icon::Close, "Close touch settings")
                                        .compact(),
                                )
                                .clicked();
                        });
                    });
                    ui.label(
                        RichText::new(if drawing {
                            "DRAW MODE"
                        } else {
                            "ANNOTATE MODE"
                        })
                        .small()
                        .color(theme::muted()),
                    );
                    ui.separator();
                    preference_controls(ui, drawing, false);
                });
            });
        if close {
            let mut current = state(ctx);
            current.panel_open = false;
            ctx.data_mut(|d| d.insert_temp(id(), current));
        }
        true
    }
    pub(super) fn touch_toolbar(&mut self, ctx: &egui::Context) {
        let snapshot = state(ctx);
        if !controls_available(ctx)
            || !snapshot.visible
            || self.home.visible
            || !self.active_document_ready()
            || !matches!(
                self.editor.mode,
                editing::Mode::Draw | editing::Mode::Annotate
            )
        {
            return;
        }
        let mut open = true;
        let mut tool = None;
        let mut action = None;
        egui::Window::new("Touch tools")
            .id(egui::Id::new(("touch_palette", snapshot.palette_revision)))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(420.)
            .default_pos(ctx.content_rect().center_bottom() - vec2(210., 420.))
            .frame(ui::modal_frame().inner_margin(14))
            .show(ctx, |ui| {
                ui.set_max_width(ui::dialog_width(ctx, 420.));
                ui.spacing_mut().interact_size.y = ui::target_size(44.);
                ui.spacing_mut().item_spacing = vec2(6., 8.);
                if self.editor.mode == editing::Mode::Draw {
                    let mut enabled = draw_with_finger(ctx);
                    if ui.checkbox(&mut enabled, "Draw with finger").changed() {
                        set_finger(ctx, enabled);
                    }
                    if snapshot.preferences.gesture_hints {
                        ui.label(
                            RichText::new(if enabled {
                                "One finger draws · two fingers navigate"
                            } else {
                                "Drag to pan · press and hold for a menu"
                            })
                            .small()
                            .color(theme::muted()),
                        );
                    }
                }
                ui.add_enabled_ui(!self.content.busy && !self.saving, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for (value, label) in [
                            (editing::Tool::Select, "Select"),
                            (editing::Tool::Pen, "Pen"),
                            (editing::Tool::AreaHighlight, "Highlighter"),
                            (editing::Tool::Eraser, "Eraser"),
                            (editing::Tool::Lasso, "Lasso"),
                        ] {
                            if ui
                                .add(
                                    egui::Button::new(label)
                                        .min_size(vec2(78., ui::target_size(44.)))
                                        .selected(self.editor.tool == value),
                                )
                                .clicked()
                            {
                                tool = Some(value);
                            }
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        for (name, color) in [
                            ("Black", [0.12, 0.12, 0.14]),
                            ("Blue", [0.1, 0.4, 0.85]),
                            ("Green", [0.1, 0.5, 0.3]),
                            ("Red", [0.85, 0.15, 0.18]),
                            ("Yellow", [1., 0.8, 0.1]),
                        ] {
                            let (rect, response) = ui.allocate_exact_size(
                                vec2(ui::target_size(44.), ui::target_size(44.)),
                                egui::Sense::click(),
                            );
                            response.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::WidgetType::Button,
                                    ui.is_enabled(),
                                    self.editor.style.color == color,
                                    format!("{name} ink"),
                                )
                            });
                            ui.painter().circle_filled(
                                rect.center(),
                                12.,
                                Color32::from_rgb(
                                    (color[0] * 255.) as u8,
                                    (color[1] * 255.) as u8,
                                    (color[2] * 255.) as u8,
                                ),
                            );
                            if self.editor.style.color == color || response.has_focus() {
                                ui.painter().circle_stroke(
                                    rect.center(),
                                    18.,
                                    Stroke::new(2., theme::accent()),
                                );
                            }
                            if response.on_hover_text(name).clicked() {
                                self.editor.style.color = color;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(format!("Stroke width: {:.1} pt", self.editor.style.width));
                        if ui
                            .add(
                                egui::Button::new("−")
                                    .min_size(vec2(ui::target_size(44.), ui::target_size(44.))),
                            )
                            .on_hover_text("Thinner stroke")
                            .clicked()
                        {
                            self.editor.style.width =
                                (self.editor.style.width - 0.5).clamp(0.5, 12.);
                        }
                        if ui
                            .add(
                                egui::Button::new("+")
                                    .min_size(vec2(ui::target_size(44.), ui::target_size(44.))),
                            )
                            .on_hover_text("Thicker stroke")
                            .clicked()
                        {
                            self.editor.style.width =
                                (self.editor.style.width + 0.5).clamp(0.5, 12.);
                        }
                    });
                    ui.horizontal(|ui| {
                        for (label, command) in [("Undo", "undo"), ("Redo", "redo")] {
                            if ui
                                .add(
                                    egui::Button::new(label)
                                        .min_size(vec2(90., ui::target_size(44.))),
                                )
                                .clicked()
                            {
                                action = Some(command);
                            }
                        }
                    });
                });
                if let Some(error) = snapshot.error {
                    ui.colored_label(theme::danger(), error);
                }
            });
        if !open {
            let mut state = state(ctx);
            state.visible = false;
            ctx.data_mut(|d| d.insert_temp(id(), state));
        }
        if let Some(tool) = tool {
            self.toggle_editor_tool(ctx, tool);
        }
        if let Some(action) = action {
            self.action(action, ctx);
        }
    }
}

#[cfg(test)]
pub(super) fn smooth_finger_test(ctx: &egui::Context) {
    update_preferences(ctx, |p| p.smoothing = Smoothing::Strong);
}

#[cfg(test)]
pub(super) fn preview(ctx: &egui::Context) {
    let mut state = state(ctx);
    state.visible = std::env::var_os("MYPDF_PREVIEW_TOUCH_PANEL").is_none();
    state.panel_open = !state.visible;
    state.seen_touch = true;
    state.preferences.mode = InputMode::Touch;
    ctx.data_mut(|d| d.insert_temp(id(), state));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn touch_preferences_migrate_persist_and_keep_manual_overrides() {
        let old: Preferences = serde_json::from_str(r#"{"draw_with_finger":true}"#).unwrap();
        assert!(old.draw_with_finger && old.mode == InputMode::Automatic && old.gesture_hints);
        let root = tempfile::tempdir().unwrap();
        let prefs = Preferences {
            mode: InputMode::Mouse,
            targets: Targets::ExtraLarge,
            hold_delay: HoldDelay::Slow,
            smoothing: Smoothing::Strong,
            gesture_hints: false,
            ..old
        };
        preferences::save(root.path(), "touch.json", &prefs).unwrap();
        let restored: Preferences = preferences::load(root.path(), "touch.json").unwrap();
        assert!(restored.mode == InputMode::Mouse && restored.targets == Targets::ExtraLarge);
        assert!(
            restored.hold_delay == HoldDelay::Slow
                && restored.smoothing == Smoothing::Strong
                && !restored.gesture_hints
        );
        let mut current = State::default();
        assert!(!available(&current));
        current.hardware_touch = true;
        assert!(available(&current));
        current.preferences.mode = InputMode::Mouse;
        assert!(!available(&current));
        current.hardware_touch = false;
        current.preferences.mode = InputMode::Touch;
        assert!(available(&current));
        current.preferences.mode = InputMode::Automatic;
        current.seen_touch = true;
        assert!(available(&current));
    }
    #[test]
    fn touch_detection_ignores_trackpad_wheels_and_honors_mouse_only() {
        let ctx = egui::Context::default();
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0., 10.),
                    modifiers: Default::default(),
                }],
                ..Default::default()
            },
            observe,
        );
        assert!(!state(&ctx).seen_touch);
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Touch {
                    device_id: egui::TouchDeviceId(1),
                    id: egui::TouchId(1),
                    phase: egui::TouchPhase::Start,
                    pos: pos2(100., 100.),
                    force: None,
                }],
                ..Default::default()
            },
            observe,
        );
        assert!(controls_available(&ctx));
        update_preferences(&ctx, |p| {
            p.mode = InputMode::Mouse;
            p.draw_with_finger = true;
        });
        assert!(!controls_available(&ctx) && !draw_with_finger(&ctx));
        theme::set_touch_target(0.);
    }
    #[test]
    fn touch_targets_restore_the_existing_mouse_density() {
        let ctx = egui::Context::default();
        let previous = theme::touch_target();
        struct Restore(f32);
        impl Drop for Restore {
            fn drop(&mut self) {
                theme::set_touch_target(self.0);
            }
        }
        let _restore = Restore(previous);
        theme::set_touch_target(0.);
        let baseline = ui::target_size(32.);
        let density = theme::density();
        update_preferences(&ctx, |p| {
            p.mode = InputMode::Touch;
            p.targets = Targets::ExtraLarge;
        });
        let _ = ctx.run(Default::default(), observe);
        assert_eq!(ui::target_size(32.), 52.);
        assert_eq!(theme::handle_hit_radius(12.), 26.);
        update_preferences(&ctx, |p| p.mode = InputMode::Mouse);
        let _ = ctx.run(Default::default(), observe);
        assert_eq!(ui::target_size(32.), baseline);
        assert_eq!(theme::density(), density);
    }
    #[test]
    fn finger_smoothing_preserves_endpoints_and_reduces_jitter() {
        let ctx = egui::Context::default();
        let raw = vec![[0., 0.], [1., 2.], [2., -2.], [3., 2.], [4., -2.], [5., 0.]];
        assert_eq!(smooth_stroke(&ctx, &raw), raw);
        update_preferences(&ctx, |p| p.smoothing = Smoothing::Light);
        let light = smooth_stroke(&ctx, &raw);
        update_preferences(&ctx, |p| p.smoothing = Smoothing::Strong);
        let strong = smooth_stroke(&ctx, &raw);
        for points in [&light, &strong] {
            assert_eq!(points.first(), raw.first());
            assert_eq!(points.last(), raw.last());
        }
        let variation = |points: &[[f32; 2]]| {
            points
                .windows(2)
                .map(|v| (v[1][1] - v[0][1]).abs())
                .sum::<f32>()
        };
        assert!(variation(&light) < variation(&raw) && variation(&strong) < variation(&light));
    }
    #[test]
    fn ribbon_opens_contextual_touch_settings_without_an_extra_row() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.editor.mode = editing::Mode::Draw;
        update_preferences(&ctx, |p| p.mode = InputMode::Touch);
        let frame = |app: &mut Viewer, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(900., 1400.))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    app.editor_tools(ctx);
                    if app.inspector {
                        app.touch_settings_panel(ctx);
                    }
                    egui::CentralPanel::default().show(ctx, |_| {});
                },
            )
        };
        let click = |app: &mut Viewer, label: &str| {
            for _ in 0..6 {
                frame(app, vec![]);
            }
            let out = frame(app, vec![]);
            let r = out
                .platform_output
                .accesskit_update
                .unwrap()
                .nodes
                .into_iter()
                .find(|(_, n)| n.label() == Some(label))
                .unwrap()
                .1
                .bounds()
                .unwrap();
            let pos = pos2(((r.x0 + r.x1) / 2.) as f32, ((r.y0 + r.y1) / 2.) as f32);
            for pressed in [true, false] {
                frame(
                    app,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
        };
        for _ in 0..6 {
            frame(&mut app, vec![]);
        }
        let out = frame(&mut app, vec![]);
        let nodes = out.platform_output.accesskit_update.unwrap().nodes;
        assert!(
            !nodes
                .iter()
                .any(|(_, n)| n.label() == Some("Draw with finger"))
        );
        let bounds = |label| {
            nodes
                .iter()
                .find(|(_, n)| n.label() == Some(label))
                .unwrap()
                .1
                .bounds()
                .unwrap()
        };
        let touch = bounds("Touch settings");
        let select = bounds("Select");
        assert!(touch.x1 < select.x0);
        assert!((touch.y0 - select.y0).abs() < 0.1);
        click(&mut app, "Touch settings");
        assert!(state(&ctx).panel_open && app.inspector);
        click(&mut app, "Draw with finger");
        assert!(draw_with_finger(&ctx));
        app.editor.mode = editing::Mode::Annotate;
        for _ in 0..6 {
            frame(&mut app, vec![]);
        }
        let out = frame(&mut app, vec![]);
        let nodes = out.platform_output.accesskit_update.unwrap().nodes;
        assert!(out.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.job.text == "ANNOTATE MODE")));
        assert!(
            !nodes
                .iter()
                .any(|(_, n)| n.label() == Some("Draw with finger"))
        );
        click(&mut app, "Show floating toolbar");
        assert!(state(&ctx).visible);
        let revision = state(&ctx).palette_revision;
        click(&mut app, "Reset position");
        assert_eq!(state(&ctx).palette_revision, revision + 1);
        assert!(state(&ctx).visible);
        click(&mut app, "Close touch settings");
        assert!(!state(&ctx).panel_open);
        assert!(app.editor.tool == editing::Tool::Select);
    }
    #[test]
    fn toolbar_changes_tools_and_colors_toggles_finger_and_runs_history() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.open_memory(
            PathBuf::from("Touch toolbar test.pdf"),
            mypdf_milestone0::convert::blank(595., 842.).unwrap(),
            &ctx,
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        app.home.visible = false;
        app.editor.mode = editing::Mode::Draw;
        preview(&ctx);
        let frame = |app: &mut Viewer, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(900., 700.))),
                    events,
                    ..Default::default()
                },
                |ctx| app.touch_toolbar(ctx),
            )
        };
        let click = |app: &mut Viewer, label: &str| {
            for _ in 0..8 {
                frame(app, vec![]);
            }
            let output = frame(app, vec![]);
            let r = output
                .platform_output
                .accesskit_update
                .unwrap()
                .nodes
                .into_iter()
                .find(|(_, n)| n.label() == Some(label))
                .unwrap_or_else(|| panic!("missing {label}"))
                .1
                .bounds()
                .unwrap();
            if label != "Close window" {
                assert!(r.y1 - r.y0 >= 44., "{label} needs a full touch target");
            }
            let pos = pos2(((r.x0 + r.x1) / 2.) as f32, ((r.y0 + r.y1) / 2.) as f32);
            for pressed in [true, false] {
                frame(
                    app,
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
        };
        assert!(!draw_with_finger(&ctx));
        click(&mut app, "Draw with finger");
        assert!(draw_with_finger(&ctx));
        click(&mut app, "Draw with finger");
        assert!(!draw_with_finger(&ctx));
        click(&mut app, "Pen");
        assert!(app.editor.tool == editing::Tool::Pen);
        click(&mut app, "Blue ink");
        assert_eq!(app.editor.style.color, [0.1, 0.4, 0.85]);
        let mark = mypdf_milestone0::annotations::Annotation::new(
            0,
            [50., 50., 100., 100.],
            mypdf_milestone0::annotations::Kind::Rectangle,
            Default::default(),
        );
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![mark.clone()])
            .unwrap();
        click(&mut app, "Undo");
        assert!(app.tabs[0].session.as_ref().unwrap().annotations.is_empty());
        click(&mut app, "Redo");
        assert_eq!(
            app.tabs[0].session.as_ref().unwrap().annotations,
            vec![mark]
        );
        click(&mut app, "Close window");
        assert!(!state(&ctx).visible);
    }
}
