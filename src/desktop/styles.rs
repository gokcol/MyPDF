//! Reusable appearances. Geometry, image pixels and object identity never transfer.
use super::*;
use mypdf_milestone0::annotations::Style;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Clone, Serialize, Deserialize)]
pub(super) enum Preset {
    Drawing(Style),
    Text {
        font: Option<PathBuf>,
        size: f32,
        color: [f32; 3],
    },
}
impl Preset {
    fn valid(&self) -> bool {
        match self {
            Self::Drawing(style) => mypdf_milestone0::annotations::Annotation::new(
                0,
                [0., 0., 100., 100.],
                mypdf_milestone0::annotations::Kind::Rectangle,
                style.clone(),
            )
            .validate()
            .is_ok(),
            Self::Text { size, color, .. } => {
                size.is_finite()
                    && (1. ..=500.).contains(size)
                    && color
                        .iter()
                        .all(|v| v.is_finite() && (0. ..=1.).contains(v))
            }
        }
    }
    fn compatible(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::Drawing(_), Self::Drawing(_)) | (Self::Text { .. }, Self::Text { .. })
        )
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Saved {
    name: String,
    appearance: Preset,
}
struct State {
    entries: Vec<Saved>,
    clipboard: Option<Preset>,
    name: String,
    error: Option<String>,
    root: PathBuf,
    #[cfg(test)]
    _temporary: tempfile::TempDir,
}
fn state(ctx: &egui::Context) -> Arc<Mutex<State>> {
    let id = egui::Id::new("appearance_library");
    if let Some(state) = ctx.data(|d| d.get_temp::<Arc<Mutex<State>>>(id)) {
        return state;
    }
    #[cfg(test)]
    let temporary = tempfile::tempdir().unwrap();
    #[cfg(test)]
    let root = temporary.path().to_path_buf();
    #[cfg(not(test))]
    let root = mypdf_milestone0::assets::data_directory();
    let result = super::preferences::load::<Vec<Saved>>(&root, "styles.json");
    let (entries, error) = match result {
        Ok(v)
            if v.len() <= 100
                && v.iter().all(|e| {
                    !e.name.trim().is_empty()
                        && e.name.chars().count() <= 80
                        && e.appearance.valid()
                }) =>
        {
            (v, None)
        }
        Ok(_) => (
            vec![],
            Some("Saved styles contain invalid values; the file was left unchanged.".into()),
        ),
        Err(e) => (vec![], Some(e.to_string())),
    };
    let state = Arc::new(Mutex::new(State {
        entries,
        clipboard: None,
        name: String::new(),
        error,
        root,
        #[cfg(test)]
        _temporary: temporary,
    }));
    ctx.data_mut(|d| d.insert_temp(id, state.clone()));
    state
}
pub(super) fn copy(ctx: &egui::Context, preset: Preset) {
    state(ctx).lock().unwrap().clipboard = Some(preset);
}
pub(super) fn pasted(ctx: &egui::Context, current: &Preset) -> Option<Preset> {
    state(ctx)
        .lock()
        .unwrap()
        .clipboard
        .as_ref()
        .filter(|p| p.compatible(current))
        .cloned()
}
pub(super) fn panel(ui: &mut egui::Ui, current: Preset) -> Option<Preset> {
    let shared = state(ui.ctx());
    let mut state = shared.lock().unwrap();
    let mut result = None;
    egui::CollapsingHeader::new("Reusable styles")
        .id_salt("reusable_appearance")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .add(ActionButton::new(Icon::Copy, "Copy style"))
                    .clicked()
                {
                    state.clipboard = Some(current.clone());
                }
                let clip = state
                    .clipboard
                    .as_ref()
                    .filter(|p| p.compatible(&current))
                    .cloned();
                if ui
                    .add_enabled(clip.is_some(), ActionButton::new(Icon::Paste, "Paste"))
                    .clicked()
                {
                    result = clip;
                }
            });
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut state.name)
                        .hint_text("Style name")
                        .char_limit(80)
                        .desired_width((ui.available_width() - 40.).max(80.)),
                );
                if ui
                    .add_enabled(
                        !state.name.trim().is_empty() && state.error.is_none(),
                        ActionButton::new(Icon::Plus, "Save style").compact(),
                    )
                    .clicked()
                {
                    let name = state.name.trim().to_owned();
                    let mut entries = state.entries.clone();
                    entries.retain(|e| e.name != name);
                    entries.push(Saved {
                        name,
                        appearance: current.clone(),
                    });
                    if entries.len() > 100 {
                        state.error = Some("Keep up to 100 saved styles".into());
                    } else {
                        match super::preferences::save(&state.root, "styles.json", &entries) {
                            Ok(()) => {
                                state.entries = entries;
                                state.name.clear();
                            }
                            Err(e) => state.error = Some(e.to_string()),
                        }
                    }
                }
            });
            let mut remove = None;
            for (index, entry) in state
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.appearance.compatible(&current))
            {
                ui.horizontal(|ui| {
                    ui.allocate_ui_with_layout(
                        vec2((ui.available_width() - 42.).max(40.), 27.),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            if ui
                                .add(egui::Button::new(&entry.name).truncate())
                                .on_hover_text(&entry.name)
                                .clicked()
                            {
                                result = Some(entry.appearance.clone());
                            }
                        },
                    );
                    if ui
                        .add(ActionButton::new(Icon::Trash, "Remove saved style").compact())
                        .clicked()
                    {
                        remove = Some(index);
                    }
                });
            }
            if let Some(index) = remove {
                let mut entries = state.entries.clone();
                entries.remove(index);
                match super::preferences::save(&state.root, "styles.json", &entries) {
                    Ok(()) => {
                        state.entries = entries;
                        state.error = None;
                    }
                    Err(e) => state.error = Some(e.to_string()),
                }
            }
            if let Some(error) = &state.error {
                ui.colored_label(theme::muted(), error);
            }
        });
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_appearances_persist_and_clipboard_only_applies_compatible_styles() {
        let ctx = egui::Context::default();
        let style = Style {
            width: 4.,
            color: [0.2, 0.4, 0.8],
            ..Default::default()
        };
        let drawing = Preset::Drawing(style.clone());
        copy(&ctx, drawing.clone());
        assert!(pasted(&ctx, &drawing).is_some());
        assert!(
            pasted(
                &ctx,
                &Preset::Text {
                    font: None,
                    size: 12.,
                    color: [0.; 3]
                }
            )
            .is_none()
        );
        let shared = state(&ctx);
        let root = shared.lock().unwrap().root.clone();
        preferences::save(
            &root,
            "styles.json",
            &vec![Saved {
                name: "Blue outline".into(),
                appearance: drawing,
            }],
        )
        .unwrap();
        ctx.data_mut(|d| d.remove::<Arc<Mutex<State>>>(egui::Id::new("appearance_library")));
        // Read from disk independently of the in-memory clipboard.
        let loaded: Vec<Saved> = preferences::load(&root, "styles.json").unwrap();
        assert_eq!(loaded[0].name, "Blue outline");
        assert!(matches!(&loaded[0].appearance,Preset::Drawing(s) if *s==style));
        assert!(
            !Preset::Text {
                font: None,
                size: -1.,
                color: [0.; 3]
            }
            .valid()
        );
        assert!(
            !Preset::Text {
                font: None,
                size: 12.,
                color: [2., 0., 0.]
            }
            .valid()
        );
    }
}
