//! Reusable appearances. Geometry, image pixels and object identity never transfer.
use super::*;
use mypdf_milestone0::annotations::Style;
use mypdf_milestone0::text_style;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
mod controls;
mod manager;

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(super) enum Preset {
    Drawing(Style),
    Text {
        font: Option<PathBuf>,
        size: f32,
        color: [f32; 3],
    },
}
impl Preset {
    fn kind(&self) -> &'static str {
        match self {
            Self::Drawing(_) => "Drawing",
            Self::Text { .. } => "Text",
        }
    }
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
    #[serde(default)]
    purpose: String,
    appearance: Preset,
    /// Built-ins come from the app, never from the user's preferences file.
    #[serde(skip)]
    built_in: bool,
}
impl Saved {
    fn valid(&self) -> bool {
        !self.name.trim().is_empty()
            && self.name.chars().count() <= 80
            && self.purpose.chars().count() <= 60
            && self.appearance.valid()
    }
}
/// Version 2 stores user styles only; built-ins are supplied by the app.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum Stored {
    Current { version: u32, entries: Vec<Saved> },
    Legacy(Vec<Saved>),
}
impl Default for Stored {
    fn default() -> Self {
        Self::Legacy(vec![])
    }
}
fn samples() -> Vec<Saved> {
    vec![
        Saved {
            name: "Review outline".into(),
            purpose: "Review".into(),
            built_in: true,
            appearance: Preset::Drawing(Style {
                color: [0.82, 0.29, 0.12],
                width: 2.,
                ..Default::default()
            }),
        },
        Saved {
            name: "Document heading".into(),
            purpose: "Headings".into(),
            built_in: true,
            appearance: Preset::Text {
                font: text_style::fonts()
                    .iter()
                    .find(|f| f.bold && !f.italic)
                    .map(|f| f.path.clone()),
                size: 18.,
                color: [0.12, 0.28, 0.43],
            },
        },
    ]
}
fn persist(root: &std::path::Path, entries: &[Saved]) -> anyhow::Result<()> {
    preferences::save(
        root,
        "styles.json",
        &Stored::Current {
            version: 2,
            entries: entries.iter().filter(|e| !e.built_in).cloned().collect(),
        },
    )
}
fn load(root: &std::path::Path) -> anyhow::Result<Vec<Saved>> {
    let stored: Stored = preferences::load(root, "styles.json")?;
    let (mut entries, migrate) = match stored {
        Stored::Current {
            version: 2,
            entries,
        } => (entries, false),
        Stored::Current {
            version: 1,
            entries,
        } => (entries, true),
        Stored::Legacy(entries) => (entries, true),
        _ => anyhow::bail!("This style library uses a newer format; the file was left unchanged."),
    };
    anyhow::ensure!(
        entries.len() <= 100 && entries.iter().all(Saved::valid),
        "Saved styles contain invalid values; the file was left unchanged."
    );
    let mut built_ins = samples();
    if migrate {
        // Only replace untouched starter samples. Any edited or renamed sample
        // remains a user style, including a custom style with the same name.
        entries.retain(|e| {
            !built_ins
                .iter()
                .any(|b| e.name == b.name && e.purpose == b.purpose && e.appearance == b.appearance)
        });
        persist(root, &entries)?;
    }
    built_ins.extend(entries);
    Ok(built_ins)
}
struct State {
    entries: Vec<Saved>,
    clipboard: Option<Preset>,
    name: String,
    error: Option<String>,
    read_only: bool,
    editor: Option<manager::Edit>,
    purpose: String,
    search: String,
    kind: usize,
    deleted: Option<(usize, Saved)>,
    root: PathBuf,
    #[cfg(test)]
    _temporary: tempfile::TempDir,
}
impl State {
    fn commit(&mut self, entries: Vec<Saved>) -> bool {
        if self.read_only {
            return false;
        }
        self.error = if entries.iter().filter(|e| !e.built_in).count() > 100 {
            Some("Keep up to 100 custom styles.".into())
        } else if !entries.iter().all(Saved::valid) {
            Some("Enter a name and valid appearance values.".into())
        } else {
            persist(&self.root, &entries).err().map(|e| e.to_string())
        };
        if self.error.is_none() {
            self.entries = samples();
            self.entries
                .extend(entries.into_iter().filter(|e| !e.built_in));
            true
        } else {
            false
        }
    }
    fn remove(&mut self, index: usize) {
        if self.entries[index].built_in {
            self.error = Some("Built-in styles stay available. Customize a copy instead.".into());
            return;
        }
        let mut entries = self.entries.clone();
        let removed = entries.remove(index);
        if self.commit(entries) {
            self.deleted = Some((index, removed));
        }
    }
    fn undo_delete(&mut self) {
        if let Some((index, entry)) = self.deleted.clone() {
            if self
                .entries
                .iter()
                .any(|e| !e.built_in && e.name == entry.name)
            {
                self.error =
                    Some("Rename the style with that name before restoring this one.".into());
                return;
            }
            let mut entries = self.entries.clone();
            entries.insert(index.min(entries.len()), entry);
            if self.commit(entries) {
                self.deleted = None;
            }
        }
    }
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
    let result = load(&root);
    let (entries, error) = match result {
        Ok(v) => (v, None),
        Err(e) => (vec![], Some(e.to_string())),
    };
    let state = Arc::new(Mutex::new(State {
        entries,
        clipboard: None,
        name: String::new(),
        read_only: error.is_some(),
        editor: None,
        purpose: String::new(),
        search: String::new(),
        kind: 0,
        deleted: None,
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
                        !state.name.trim().is_empty() && !state.read_only,
                        ActionButton::new(Icon::Plus, "Save style").compact(),
                    )
                    .clicked()
                {
                    let name = state.name.trim().to_owned();
                    if state.entries.iter().any(|e| e.built_in && e.name == name) {
                        state.error = Some("That name belongs to a built-in style. Choose a name for your own version.".into());
                        return;
                    }
                    let purpose = state
                        .entries
                        .iter()
                        .find(|e| e.name == name)
                        .map(|e| e.purpose.clone())
                        .unwrap_or_default();
                    let mut entries = state.entries.clone();
                    entries.retain(|e| e.name != name);
                    entries.push(Saved {
                        name,
                        purpose,
                        appearance: current.clone(),
                        built_in: false,
                    });
                    if state.commit(entries) {
                        state.name.clear();
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
                    manager::preview(ui, &entry.appearance, vec2(34., 27.));
                    ui.allocate_ui_with_layout(
                        vec2((ui.available_width() - 42.).max(40.), 27.),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            if ui
                                .add(egui::Button::new(&entry.name).truncate())
                                .on_hover_text(format!(
                                    "{}{}",
                                    entry.name,
                                    if entry.purpose.is_empty() {
                                        String::new()
                                    } else {
                                        format!(" · {}", entry.purpose)
                                    }
                                ))
                                .clicked()
                            {
                                result = Some(entry.appearance.clone());
                            }
                        },
                    );
                    if ui
                        .add_enabled(
                            !state.read_only && !entry.built_in,
                            if entry.built_in { ActionButton::new(Icon::Lock, "Built-in style").compact() }
                            else { ActionButton::new(Icon::Trash, "Remove saved style").compact() },
                        )
                        .clicked()
                    {
                        remove = Some(index);
                    }
                });
            }
            if let Some(index) = remove {
                state.remove(index);
            }
            manager::deleted_notice(ui, &mut state);
            ui.weak("Manage names, purposes and previews in Settings → Reusable styles.");
            if let Some(error) = &state.error {
                ui.colored_label(theme::muted(), error);
            }
        });
    result
}

#[cfg(test)]
pub(super) use manager::preview_edit;
pub(super) use manager::settings;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn built_in_upgrade_keeps_customized_samples_and_does_not_duplicate_them() {
        let temp = tempfile::tempdir().unwrap();
        let mut entries = samples();
        let Preset::Text { size, .. } = &mut entries[1].appearance else {
            panic!()
        };
        *size = 23.;
        preferences::save(
            temp.path(),
            "styles.json",
            &Stored::Current {
                version: 1,
                entries,
            },
        )
        .unwrap();
        let loaded = load(temp.path()).unwrap();
        assert_eq!(loaded.len(), 3);
        assert!(loaded[0].built_in && loaded[1].built_in && !loaded[2].built_in);
        assert_eq!(loaded[2].name, "Document heading");
        assert!(matches!(
            loaded[2].appearance,
            Preset::Text { size: 23., .. }
        ));
        assert!(matches!(
            loaded[1].appearance,
            Preset::Text { size: 18., .. }
        ));
        assert_eq!(load(temp.path()).unwrap().len(), 3);
    }
    #[test]
    fn legacy_styles_migrate_without_loss_and_built_ins_always_remain() {
        let temp = tempfile::tempdir().unwrap();
        let original = serde_json::json!([{
            "name": "Client notes", "appearance": {"Text": {"font": null, "size": 13.5, "color": [0.3, 0.2, 0.1]}}
        }]);
        preferences::save(temp.path(), "styles.json", &original).unwrap();
        let entries = load(temp.path()).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[2].name, "Client notes");
        assert_eq!(entries[2].purpose, "");
        assert!(matches!(
            entries[2].appearance,
            Preset::Text {
                size: 13.5,
                color: [0.3, 0.2, 0.1],
                ..
            }
        ));
        assert_eq!(load(temp.path()).unwrap().len(), 3);
        persist(temp.path(), &entries[2..]).unwrap();
        assert_eq!(load(temp.path()).unwrap().len(), 3);
        persist(temp.path(), &[]).unwrap();
        assert_eq!(load(temp.path()).unwrap().len(), 2);
    }
    #[test]
    fn samples_never_replace_existing_names_or_overfill_the_library() {
        let temp = tempfile::tempdir().unwrap();
        let mut entries = samples();
        entries[0].purpose = "My existing purpose".into();
        for n in 2..100 {
            let mut entry = entries[0].clone();
            entry.name = format!("Style {n}");
            entries.push(entry);
        }
        entries[1].name = "My heading".into();
        preferences::save(temp.path(), "styles.json", &entries).unwrap();
        let loaded = load(temp.path()).unwrap();
        assert_eq!(loaded.len(), 102);
        assert_eq!(loaded[2].purpose, "My existing purpose");
        assert_eq!(loaded[3].name, "My heading");
    }
    #[test]
    fn invalid_or_future_style_libraries_are_never_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        for bytes in [
            b"not json".as_slice(),
            br#"{"version":3,"entries":[]}"#,
            br#"[{"name":"Broken","appearance":{"Text":{"font":null,"size":-1,"color":[0,0,0]}}}]"#,
        ] {
            std::fs::write(temp.path().join("styles.json"), bytes).unwrap();
            assert!(load(temp.path()).is_err());
            assert_eq!(
                std::fs::read(temp.path().join("styles.json")).unwrap(),
                bytes
            );
        }
    }
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
                purpose: String::new(),
                appearance: drawing,
                built_in: false,
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
