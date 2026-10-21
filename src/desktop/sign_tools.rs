//! Signing ribbon and persisted placement preferences.
use super::*;

pub(super) struct State {
    signing: SigningPreferences,
    preference_root: PathBuf,
    preference_error: Option<String>,
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct SigningPreferences {
    ink_default: bool,
    quick_default: bool,
}
impl Default for SigningPreferences {
    fn default() -> Self {
        Self {
            ink_default: false,
            quick_default: true,
        }
    }
}
impl Default for State {
    fn default() -> Self {
        #[cfg(not(test))]
        let root = mypdf_milestone0::assets::data_directory();
        #[cfg(test)]
        let root = std::env::temp_dir().join(format!(
            "mypdf-sign-tools-test-{:016x}",
            rand::random::<u64>()
        ));
        Self::at(root)
    }
}
impl State {
    fn at(root: PathBuf) -> Self {
        let (signing, preference_error) = match preferences::load(&root, "signing-options.json") {
            Ok(value) => (value, None),
            Err(error) => (
                SigningPreferences::default(),
                Some(format!("Could not load signing options: {error}")),
            ),
        };
        Self {
            signing,
            preference_root: root,
            preference_error,
        }
    }
    fn save_signing(&mut self) {
        self.preference_error =
            preferences::save(&self.preference_root, "signing-options.json", &self.signing)
                .err()
                .map(|error| format!("Could not save signing options: {error}"));
    }
    pub(super) fn placement_settings(&mut self, ui: &mut egui::Ui, ink: bool) {
        ui.push_id(if ink { "ink_placement" } else { "quick_placement" }, |ui| {
            ui.strong(if ink { "When choosing Sign in ink" } else { "When choosing Quick Sign" });
            let choice = if ink { &mut self.signing.ink_default } else { &mut self.signing.quick_default };
            let mut changed = false;
            ui.horizontal_wrapped(|ui| {
                changed |= ui.radio_value(choice, true, if ink { "Use my default ink signature" } else { "Use my default digital signature" }).changed();
                changed |= ui.radio_value(choice, false, if ink { "Choose a signature each time" } else { "Review options after drawing" }).changed();
            });
            ui.weak(if ink { "Choose the default with Make default below. You always place the signature yourself." } else { "Star a signature below to make it the default. Quick Sign always waits for you to draw its region." });
            if changed { self.save_signing(); }
            if let Some(error) = &self.preference_error {
                ui.colored_label(theme::danger(), error);
            }
            ui.separator();
        });
    }
}

impl Viewer {
    pub(super) fn signing_ribbon(&mut self, ui: &mut egui::Ui) -> Option<&'static str> {
        let mut action = None;
        for (icon, label, key) in [
            (Icon::Pen, "Sign in ink", "sign_ink"),
            (Icon::Shield, "Digital Sign", "sign_digital"),
            (Icon::Sign, "Quick Sign", "sign_quick"),
            (Icon::Settings, "Manage Signs", "sign_manage"),
        ] {
            if ui.add(ActionButton::new(icon, label)).clicked() {
                action = Some(key);
            }
            if key != "sign_manage" {
                ui.separator();
            }
        }
        if let Some(tab) = self.tabs.get_mut(self.active)
            && (tab.signed || tab.signed_original.is_some())
        {
            ui.separator();
            if ui
                .add(
                    ActionButton::new(Icon::Document, "Document signatures")
                        .selected(tab.signatures.panel),
                )
                .on_hover_text("Inspect this PDF’s signatures and editing options")
                .clicked()
            {
                tab.signatures.show_panel(!tab.signatures.panel);
            }
        }
        action
    }

    pub(super) fn signing_ribbon_action(&mut self, action: &str) {
        if !self.apply_pending_annotation() {
            return;
        }
        self.workbench.paste = None;
        self.editor.signature_open = false;
        self.editor.pending_sign_rect = None;
        self.editor.mode = editing::Mode::Sign;
        match action {
            "sign_ink" => {
                self.editor.tool = editing::Tool::Select;
                self.start_ink_signature();
                if self.sign_tools.signing.ink_default {
                    self.place_default_ink_signature();
                }
            }
            "sign_quick" if self.sign_tools.signing.quick_default => self.quick_sign_in_area(),
            _ => self.sign_in_area(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;

    fn frame(app: &mut Viewer, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 800.))),
                events,
                ..Default::default()
            },
            |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
        )
    }
    fn click(app: &mut Viewer, ctx: &egui::Context, at: egui::Pos2) {
        for pressed in [true, false] {
            frame(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }
    fn bounds(app: &mut Viewer, ctx: &egui::Context, label: &str) -> Vec<egui::Pos2> {
        for _ in 0..3 {
            frame(app, ctx, vec![]);
        }
        let nodes = frame(app, ctx, vec![])
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes;
        let ribbon_y = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Read"))
            .and_then(|(_, n)| n.bounds())
            .map(|r| r.y0);
        let mut results: Vec<_> = nodes
            .into_iter()
            .filter(|(_, n)| n.label() == Some(label))
            .filter_map(|(_, n)| n.bounds())
            .filter(|r| label != "Sign" || Some(r.y0) == ribbon_y)
            .map(|r| pos2(((r.x0 + r.x1) / 2.) as f32, ((r.y0 + r.y1) / 2.) as f32))
            .collect();
        results.sort_by(|a, b| a.x.total_cmp(&b.x));
        results
    }
    #[test]
    fn ribbon_routes_actions_and_preserves_default_choices_without_signing() {
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let profile = tempfile::tempdir().unwrap();
            // Existing context-menu preferences are reused without migration or reset.
            std::fs::write(
                profile.path().join("signing-options.json"),
                r#"{"ink_default":false,"quick_default":true}"#,
            )
            .unwrap();
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("preview_portable_menu"), true));
            ctx.set_theme(theme);
            let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
            app.sign_tools = State::at(profile.path().into());
            app.new_blank(&ctx);
            for _ in 0..500 {
                frame(&mut app, &ctx, vec![]);
                if app.tabs[0].session.is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(app.tabs[0].session.is_some());
            let at = bounds(&mut app, &ctx, "Sign")[0];
            click(&mut app, &ctx, at);
            assert!(app.editor.mode == editing::Mode::Sign);
            for label in ["Sign in ink", "Digital Sign", "Quick Sign", "Manage Signs"] {
                assert_eq!(bounds(&mut app, &ctx, label).len(), 1, "{label}");
            }
            assert!(bounds(&mut app, &ctx, "Use default").is_empty());
            let at = bounds(&mut app, &ctx, "Manage Signs")[0];
            click(&mut app, &ctx, at);
            assert!(app.digital.open);
            let at = bounds(&mut app, &ctx, "Use my default ink signature")[0];
            click(&mut app, &ctx, at);
            let at = bounds(&mut app, &ctx, "Digital signatures")[0];
            click(&mut app, &ctx, at);
            let at = bounds(&mut app, &ctx, "Review options after drawing")[0];
            click(&mut app, &ctx, at);
            assert!(app.editor.pending_sign_rect.is_none());
            assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), 0);
            let at = bounds(&mut app, &ctx, "Close")[0];
            click(&mut app, &ctx, at);
            assert!(!app.digital.open);
            let restored = State::at(profile.path().into());
            assert!(restored.signing.ink_default && !restored.signing.quick_default);
            for label in ["Quick Sign", "Digital Sign"] {
                let at = bounds(&mut app, &ctx, label)[0];
                click(&mut app, &ctx, at);
                assert!(!app.digital.open && !app.digital.quick);
                assert!(app.editor.tool == editing::Tool::SignArea);
                assert!(app.editor.pending_sign_rect.is_none());
            }
            // An absent default opens the ink picker rather than dropping the action.
            let at = bounds(&mut app, &ctx, "Sign in ink")[0];
            click(&mut app, &ctx, at);
            assert!(app.editor.signature_open);
            assert!(app.workbench.paste.is_none());
            app.editor.signature_open = false;
            let at = bounds(&mut app, &ctx, "Manage Signs")[0];
            click(&mut app, &ctx, at);
            assert!(app.digital.open && app.digital.panel == digital::Panel::Ink);
            assert!(app.editor.tool == editing::Tool::Select);
            assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), 0);
        }
    }
}
