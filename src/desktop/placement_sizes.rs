//! Last-used artwork sizes in PDF points, separate from immutable library artwork.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const FILE: &str = "artwork-placement-sizes.json";
const MAX_ENTRIES: usize = 2048;

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Preferences {
    /// The longer edge avoids carrying a stretched copy's aspect into future placements.
    edges: BTreeMap<String, f32>,
}

pub(super) struct State {
    root: PathBuf,
    prefs: Preferences,
    pub(super) error: Option<String>,
    readable: bool,
}

impl State {
    pub(super) fn load(root: PathBuf) -> Self {
        let loaded = preferences::load::<Preferences>(&root, FILE).and_then(|prefs| {
            anyhow::ensure!(prefs.edges.len() <= MAX_ENTRIES, "Too many artwork sizes");
            anyhow::ensure!(
                prefs.edges.iter().all(|(id, edge)| valid(id, *edge)),
                "Invalid artwork size"
            );
            Ok(prefs)
        });
        match loaded {
            Ok(prefs) => Self {
                root,
                prefs,
                error: None,
                readable: true,
            },
            Err(error) => Self {
                root,
                prefs: Preferences::default(),
                error: Some(format!(
                    "Could not load remembered artwork sizes: {error:#}"
                )),
                // Do not replace an unreadable preference file with an empty one.
                readable: false,
            },
        }
    }

    fn size(&self, asset: &Asset, fallback: [f32; 2], page: PageSize) -> [f32; 2] {
        let aspect = if asset.aspect.is_finite() && asset.aspect > 0. {
            asset.aspect
        } else {
            1.
        };
        let size = self.prefs.edges.get(&asset.id).map_or(fallback, |edge| {
            [edge * aspect.min(1.), edge / aspect.max(1.)]
        });
        fit(size, page)
    }

    fn remember(&mut self, id: &str, rect: Box2) {
        let edge = (rect[2] - rect[0]).max(rect[3] - rect[1]);
        if !valid(id, edge)
            || rect.iter().any(|v| !v.is_finite())
            || rect[2] <= rect[0]
            || rect[3] <= rect[1]
            || self
                .prefs
                .edges
                .get(id)
                .is_some_and(|old| (old - edge).abs() < 0.01)
            || (!self.prefs.edges.contains_key(id) && self.prefs.edges.len() >= MAX_ENTRIES)
        {
            return;
        }
        self.prefs.edges.insert(id.into(), edge);
        if self.readable {
            self.error = preferences::save(&self.root, FILE, &self.prefs)
                .err()
                .map(|error| format!("Could not save remembered artwork sizes: {error:#}"));
        }
    }
}

fn valid(id: &str, edge: f32) -> bool {
    !id.is_empty() && id.len() <= 256 && edge.is_finite() && (1. ..=20_000.).contains(&edge)
}

fn fit(size: [f32; 2], page: PageSize) -> [f32; 2] {
    let scale = (page.width / size[0]).min(page.height / size[1]).min(1.);
    size.map(|v| v * scale)
}

fn dimensions(a: &Annotation) -> [f32; 2] {
    [a.rect[2] - a.rect[0], a.rect[3] - a.rect[1]]
}

impl EditorState {
    pub(super) fn asset_placement_size(
        &self,
        asset: &Asset,
        fallback: [f32; 2],
        page: PageSize,
    ) -> [f32; 2] {
        self.placement_sizes.size(asset, fallback, page)
    }

    /// Call only after a successful explicit resize, not selection, movement or undo.
    pub(super) fn remember_asset_resize(&mut self, before: &Annotation, after: &Annotation) {
        let changed = dimensions(before)
            .into_iter()
            .zip(dimensions(after))
            .any(|(old, new)| (old - new).abs() >= 0.01);
        if before.asset_id != after.asset_id || !changed {
            return;
        }
        self.remember_asset_size(after);
    }

    pub(in super::super) fn remember_asset_resizes(
        &mut self,
        before: &[Annotation],
        after: &[Annotation],
    ) {
        for mark in after {
            if let Some(previous) = before.iter().find(|a| a.id == mark.id) {
                self.remember_asset_resize(previous, mark);
            }
        }
    }

    pub(super) fn remember_asset_size(&mut self, mark: &Annotation) {
        if let Some(id) = &mark.asset_id {
            self.placement_sizes.remember(id, mark.rect);
            if let Some(error) = &self.placement_sizes.error {
                self.library_error = Some(error.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(width: f32, height: f32) -> PageSize {
        PageSize { width, height }
    }

    #[test]
    fn remembers_each_asset_across_sessions_without_changing_the_library() {
        let root = tempfile::tempdir().unwrap();
        let mut state = State::load(root.path().into());
        let check = assets::builtin_assets()
            .into_iter()
            .find(|b| b.category == "Symbols")
            .unwrap()
            .asset;
        let mut signature = Asset::new(
            "Office signature",
            Kind::Symbol(Symbol::Check),
            Style::default(),
            3.,
        );
        signature.signature = true;
        let original = signature.clone();
        state.remember(&check.id, [5., 10., 35., 40.]);
        state.remember(&signature.id, [10., 20., 118., 56.]);
        let state = State::load(root.path().into());
        let check_size = state.size(&check, [72., 72.], page(612., 792.));
        assert!((check_size[0].max(check_size[1]) - 30.).abs() < 0.001);
        assert_eq!(
            state.size(&signature, [180., 60.], page(612., 792.)),
            [108., 36.]
        );
        assert_eq!(signature, original);
        assert!(!root.path().join("assets.json").exists());
    }

    #[test]
    fn small_pages_fit_proportionally_without_forgetting_the_requested_size() {
        let root = tempfile::tempdir().unwrap();
        let mut state = State::load(root.path().into());
        let asset = Asset::new(
            "Wide stamp",
            Kind::Symbol(Symbol::Check),
            Style::default(),
            4.,
        );
        state.remember(&asset.id, [0., 0., 240., 60.]);
        assert_eq!(state.size(&asset, [180., 45.], page(100., 20.)), [80., 20.]);
        assert_eq!(
            State::load(root.path().into()).size(&asset, [180., 45.], page(612., 792.)),
            [240., 60.]
        );
        // New assets keep their own fallback; resizing one signature does not affect another.
        let other = Asset::new("Other", asset.kind.clone(), Style::default(), 4.);
        assert_eq!(
            state.size(&other, [180., 45.], page(612., 792.)),
            [180., 45.]
        );
    }

    #[test]
    fn invalid_preferences_and_non_finite_geometry_never_replace_original_data() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(FILE);
        std::fs::write(&path, b"broken preferences").unwrap();
        let mut state = State::load(root.path().into());
        assert!(state.error.is_some());
        state.remember("signature", [0., 0., 100., 20.]);
        assert_eq!(std::fs::read(&path).unwrap(), b"broken preferences");
        let clean = tempfile::tempdir().unwrap();
        let mut state = State::load(clean.path().into());
        state.remember("signature", [0., 0., f32::NAN, 20.]);
        state.remember("signature", [10., 10., 5., 5.]);
        assert!(state.prefs.edges.is_empty());
        assert!(!clean.path().join(FILE).exists());
    }

    #[test]
    fn stamp_and_signature_resize_then_reinsert_keep_pdf_size_at_different_zooms() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let profile = tempfile::tempdir().unwrap();
        app.editor.placement_sizes = State::load(profile.path().into());
        app.new_blank(&ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let check = app
            .editor
            .builtins
            .iter()
            .find(|b| b.category == "Symbols")
            .unwrap()
            .asset
            .clone();
        let mut ink = Asset::new(
            "Office ink",
            Kind::Ink {
                strokes: vec![vec![[0., 0.8], [0.5, 0.], [1., 0.7]]],
            },
            Style::default(),
            3.,
        );
        ink.signature = true;
        for (asset, scale) in [(check, 0.5), (ink, 1.5)] {
            let before_asset = asset.clone();
            app.begin_asset_placement(asset.clone());
            let mark = app.workbench.paste.take().unwrap().remove(0);
            let placed = workbench::positioned(&[mark], 0, [100., 100.], app.tabs[0].pages[0]);
            assert!(workbench::place(
                &mut app.editor,
                &mut app.tabs[0],
                placed.clone(),
                &mut app.status
            ));
            let original = placed[0].clone();
            let frame = |app: &mut Viewer, point: [f32; 2], down: Option<bool>| {
                let pos = pos2(20. + point[0] * scale, 20. + point[1] * scale);
                let mut events = vec![egui::Event::PointerMoved(pos)];
                if let Some(pressed) = down {
                    events.push(egui::Event::PointerButton {
                        pos,
                        pressed,
                        button: egui::PointerButton::Primary,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1300., 1400.))),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            page_interaction(
                                &mut app.editor,
                                &mut app.tabs[0],
                                ui,
                                0,
                                Rect::from_min_size(pos2(20., 20.), vec2(612., 792.) * scale),
                                scale,
                                &mut app.status,
                            );
                        });
                    },
                );
            };
            let start = [original.rect[2], original.rect[3]];
            let end = [
                original.rect[0] + dimensions(&original)[0] * 0.5,
                original.rect[1] + dimensions(&original)[1] * 0.5,
            ];
            frame(&mut app, start, None);
            frame(&mut app, start, Some(true));
            frame(&mut app, end, None);
            frame(&mut app, end, Some(false));
            let resized = app.tabs[0]
                .session
                .as_ref()
                .unwrap()
                .annotations
                .iter()
                .find(|a| a.id == original.id)
                .unwrap()
                .clone();
            assert!(
                (dimensions(&resized)[0] - dimensions(&original)[0] * 0.5).abs() < 0.1,
                "{scale}: {:?}",
                resized.rect
            );
            app.editor.placement_sizes = State::load(profile.path().into());
            app.begin_asset_placement(asset.clone());
            let next = &app.workbench.paste.as_ref().unwrap()[0];
            for (actual, expected) in dimensions(next).into_iter().zip(dimensions(&resized)) {
                assert!((actual - expected).abs() < 0.01);
            }
            assert_eq!(asset, before_asset);
            app.workbench.paste = None;
            app.editor_undo(false);
            assert!(
                app.tabs[0]
                    .session
                    .as_ref()
                    .unwrap()
                    .annotations
                    .contains(&original)
            );
        }
    }

    #[test]
    fn moving_or_recoloring_a_copy_does_not_replace_its_remembered_size() {
        let mut editor = EditorState::new();
        let root = tempfile::tempdir().unwrap();
        editor.placement_sizes = State::load(root.path().into());
        let mut mark = Annotation::new(
            0,
            [10., 20., 110., 60.],
            Kind::Symbol(Symbol::Check),
            Style::default(),
        );
        mark.asset_id = Some("check".into());
        let mut next = mark.clone();
        next.rect = [30., 40., 130., 80.];
        next.style.color = [1., 0., 0.];
        editor.remember_asset_resize(&mark, &next);
        assert!(!root.path().join(FILE).exists());
        next.rect[2] += 50.;
        next.rect[3] += 20.;
        editor.remember_asset_resize(&mark, &next);
        assert_eq!(State::load(root.path().into()).prefs.edges["check"], 150.);
    }
}
