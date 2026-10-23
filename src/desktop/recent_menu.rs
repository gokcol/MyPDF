//! Recent PDF shortcuts shared by the File and compact Open menus.
use super::*;

const LIMIT: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Recent {
    pub path: PathBuf,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Command(&'static str),
    Recent(PathBuf),
}

/// Home provides newest-first paths. Keep a short list and distinguish same-name PDFs.
pub(super) fn items(paths: Vec<PathBuf>) -> Vec<Recent> {
    paths
        .iter()
        .take(LIMIT)
        .map(|path| {
            let name = path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy();
            let duplicate = paths
                .iter()
                .filter(|p| p.file_name() == path.file_name())
                .count()
                > 1;
            let title = if duplicate {
                format!(
                    "{name} — {}",
                    path.parent()
                        .unwrap_or_else(|| std::path::Path::new(""))
                        .display()
                )
            } else {
                name.into_owned()
            };
            let chars: Vec<_> = title.chars().filter(|c| !c.is_control()).collect();
            let label = if chars.len() > 80 {
                chars[..48]
                    .iter()
                    .chain(std::iter::once(&'…'))
                    .chain(&chars[chars.len() - 31..])
                    .collect()
            } else {
                chars.into_iter().collect()
            };
            Recent {
                path: path.clone(),
                label,
            }
        })
        .collect()
}

/// Return the live child bounds so a toolbar hover menu stays open while its
/// pointer crosses into this submenu. Ordinary application menus use egui's tracking.
pub(super) fn submenu(
    ui: &mut egui::Ui,
    recent: &[Recent],
    width: f32,
) -> (Option<Action>, Option<Rect>) {
    let response = ui.add(ui::MenuItem::new(Icon::History, "Open Recent").width(width));
    ui::draw_icon(
        ui.painter(),
        Rect::from_center_size(
            pos2(response.rect.right() - 12., response.rect.center().y),
            vec2(12., 12.),
        ),
        Icon::Right,
        ui.visuals().text_color(),
    );
    let popup_id = egui::containers::menu::SubMenu::id_from_widget_id(response.id);
    let needs_measurement =
        egui::AreaState::load(ui.ctx(), popup_id).is_none_or(|state| state.size.is_none());
    let mut action = None;
    let popup = egui::containers::menu::SubMenu::new()
        .config(
            egui::containers::menu::MenuConfig::new()
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
        )
        .show(ui, &response, |ui| {
            let width = ui::menu_width(ui, recent.iter().map(|entry| (entry.label.as_str(), None)))
                .min(420.);
            if recent.is_empty() {
                ui.add_enabled(
                    false,
                    ui::MenuItem::new(Icon::Document, "No Recent PDFs").width(width),
                );
            } else {
                egui::ScrollArea::vertical()
                    .max_height((ui.ctx().content_rect().height() - 64.).max(120.))
                    .show(ui, |ui| {
                        for entry in recent {
                            if ui
                                .add(ui::MenuItem::new(Icon::Document, &entry.label).width(width))
                                .on_hover_text(entry.path.display().to_string())
                                .clicked()
                            {
                                action = Some(Action::Recent(entry.path.clone()));
                                egui::Popup::close_all(ui.ctx());
                            }
                        }
                    });
            }
        });
    if needs_measurement && popup.is_some() {
        ui.ctx()
            .request_discard("Measure the recent PDF submenu before displaying it");
    }
    (action, popup.map(|popup| popup.response.rect))
}

impl Viewer {
    pub(super) fn recent_menu_action(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::Command(command) => self.action(command, ctx),
            Action::Recent(path) => self.shell_action(shell_menu::Action::Document(path), ctx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_labels_preserve_order_disambiguate_names_and_stay_compact() {
        let mut paths = vec![
            PathBuf::from("/Work/Ünicode.pdf"),
            PathBuf::from("/Personal/Ünicode.pdf"),
        ];
        paths.extend((0..12).map(|i| PathBuf::from(format!("/Archive/{i}.pdf"))));
        let entries = items(paths.clone());
        assert_eq!(entries.len(), LIMIT);
        assert_eq!(
            entries.iter().map(|e| &e.path).collect::<Vec<_>>(),
            paths[..LIMIT].iter().collect::<Vec<_>>()
        );
        assert_eq!(entries[0].label, "Ünicode.pdf — /Work");
        assert_eq!(entries[1].label, "Ünicode.pdf — /Personal");
        let long = items(vec![PathBuf::from(format!(
            "/Work/{}.pdf",
            "Ü".repeat(100)
        ))]);
        assert_eq!(long[0].label.chars().count(), 80);
        assert_eq!(
            long[0]
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .chars()
                .count(),
            104
        );
    }

    fn menu_frame(
        ctx: &egui::Context,
        toolbar: bool,
        recent: &[Recent],
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Option<Action>) {
        let mut action = None;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(720., 560.))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    action = if toolbar {
                        toolbar_file_button(ui, true, recent)
                    } else {
                        portable_menu::bar(ui, false, false, recent)
                    };
                });
            },
        );
        (output, action)
    }

    fn bounds(output: &egui::FullOutput, label: &str) -> Rect {
        let nodes = &output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes;
        let r = nodes
            .iter()
            .find(|(_, node)| node.label() == Some(label))
            .unwrap_or_else(|| {
                panic!(
                    "Missing {label}; labels: {:?}",
                    nodes
                        .iter()
                        .filter_map(|(_, n)| n.label())
                        .collect::<Vec<_>>()
                )
            })
            .1
            .bounds()
            .unwrap();
        Rect::from_min_max(
            pos2(r.x0 as f32, r.y0 as f32),
            pos2(r.x1 as f32, r.y1 as f32),
        )
    }

    fn pointer(at: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    #[test]
    fn recent_submenu_keeps_parent_visible_and_routes_choice_in_both_file_menus() {
        for toolbar in [false, true] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let recent = items(vec![
                PathBuf::from("/Work/Newest.pdf"),
                PathBuf::from("/Work/Older.pdf"),
            ]);
            let (output, _) = menu_frame(&ctx, toolbar, &recent, vec![]);
            let open = bounds(&output, if toolbar { "Open…" } else { "File" }).center();
            if toolbar {
                assert_eq!(
                    menu_frame(
                        &ctx,
                        toolbar,
                        &recent,
                        vec![egui::Event::PointerMoved(open)]
                    )
                    .1,
                    None
                );
            } else {
                menu_frame(&ctx, toolbar, &recent, pointer(open, true));
                assert_eq!(
                    menu_frame(&ctx, toolbar, &recent, pointer(open, false)).1,
                    None
                );
            }
            let (output, _) = menu_frame(&ctx, toolbar, &recent, vec![]);
            let submenu = bounds(&output, "Open Recent").center();
            assert_eq!(
                menu_frame(
                    &ctx,
                    toolbar,
                    &recent,
                    vec![egui::Event::PointerMoved(submenu)]
                )
                .1,
                None
            );
            let (output, _) = menu_frame(&ctx, toolbar, &recent, vec![]);
            let newest = bounds(&output, "Newest.pdf");
            let older = bounds(&output, "Older.pdf");
            assert!(newest.top() < older.top());
            assert!(newest.left() >= 0. && newest.right() <= 720.);
            bounds(&output, "Save As…");
            bounds(&output, "Open Recent");
            let (output, action) = menu_frame(
                &ctx,
                toolbar,
                &recent,
                vec![egui::Event::PointerMoved(newest.center())],
            );
            assert_eq!(action, None, "Hovering a recent must never open it");
            bounds(&output, "Open Recent");
            bounds(&output, "Newest.pdf");
            menu_frame(&ctx, toolbar, &recent, pointer(newest.center(), true));
            assert_eq!(
                menu_frame(&ctx, toolbar, &recent, pointer(newest.center(), false)).1,
                Some(Action::Recent(recent[0].path.clone()))
            );
            assert!(
                !egui::Popup::is_any_open(&ctx),
                "Choosing a recent closes both menus"
            );
        }
    }

    #[test]
    fn recent_choice_reuses_tab_and_switches_to_its_group_without_altering_edits() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("Recent.pdf");
        std::fs::write(&path, mypdf_milestone0::convert::blank(500., 700.).unwrap()).unwrap();
        app.open(path.clone(), &ctx);
        let id = app.tabs[0].id;
        app.tabs[0].unsaved_on_load = true;
        app.tab_controls.groups.push(tabs::Group {
            id: 1,
            name: "Work".into(),
            active: None,
        });
        app.tabs[0].document_group = 1;
        app.switch_group(0);
        app.recent_menu_action(Action::Recent(path), &ctx);
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.tabs[0].id, id);
        assert!(app.tabs[0].unsaved_on_load);
        assert_eq!(app.tab_controls.current_group, 1);
        assert!(!app.home.visible);
        app.recent_menu_action(Action::Recent(folder.path().join("Missing.pdf")), &ctx);
        assert_eq!(app.tabs.len(), 1);
        assert!(app.status.as_ref().is_some_and(|(_, error)| *error));
    }
}
