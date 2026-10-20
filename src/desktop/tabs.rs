//! Compact document navigation. All closes go through the document save guard.
use super::*;

pub(super) struct State {
    query: String,
    offset: f32,
    drag: Option<TabDrag>,
    window_drag_requested: bool,
    close_queue: std::collections::VecDeque<u64>,
    waiting: Option<u64>,
    pub groups: Vec<Group>,
    pub current_group: u64,
    remembered: HashMap<u64, u64>,
    group_editor: Option<(Option<u64>, String)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(count: usize) -> (Viewer, egui::Context) {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        for i in 0..count {
            app.open_memory(
                PathBuf::from(format!("document-{i}.pdf")),
                mypdf_milestone0::convert::blank(500., 700.).unwrap(),
                &ctx,
            );
        }
        (app, ctx)
    }

    fn frame(app: &mut Viewer, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
        let width = ctx
            .data(|d| d.get_temp::<f32>(egui::Id::new("test_width")))
            .unwrap_or(1400.);
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(width, 800.))),
                events,
                ..Default::default()
            },
            |ctx| {
                eframe::App::update(app, ctx, &mut eframe::Frame::_new_kittest());
            },
        );
        assert!(
            !output.viewport_output.values().any(|v| v
                .commands
                .iter()
                .any(|c| matches!(c, egui::ViewportCommand::StartDrag))),
            "A tab interaction started moving the OS window"
        );
        output
    }

    fn bounds(app: &mut Viewer, ctx: &egui::Context, name: &str) -> Rect {
        for _ in 0..3 {
            frame(app, ctx, vec![]);
        }
        let output = frame(app, ctx, vec![]);
        let nodes = output.platform_output.accesskit_update.unwrap().nodes;
        let labels: Vec<_> = nodes
            .iter()
            .filter_map(|(_, n)| n.label().or_else(|| n.value()))
            .collect();
        let node = nodes
            .iter()
            .find(|(_, n)| n.label().or_else(|| n.value()) == Some(name) && n.bounds().is_some())
            .unwrap_or_else(|| panic!("Missing {name}: {labels:?}"))
            .1
            .clone();
        let r = node.bounds().unwrap();
        Rect::from_min_max(
            pos2(r.x0 as f32, r.y0 as f32),
            pos2(r.x1 as f32, r.y1 as f32),
        )
    }

    fn pointer(
        app: &mut Viewer,
        ctx: &egui::Context,
        at: egui::Pos2,
        button: egui::PointerButton,
        pressed: bool,
    ) {
        frame(
            app,
            ctx,
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }

    #[test]
    fn adaptive_tabs_fit_when_possible_and_keep_active_readable() {
        let result = widths(&[260., 260., 260.], Some(1), 440.);
        assert!(result.iter().sum::<f32>() + 8. <= 440.1);
        assert!(result[1] >= 156.);
        assert!(result.iter().all(|&w| (104. ..=220.).contains(&w)));
        let overflow = widths(&[220.; 10], Some(2), 500.);
        assert!(overflow.iter().sum::<f32>() > 500.);
    }

    #[test]
    fn actual_tab_drag_reorders_without_changing_active_or_dirty_state() {
        for (count, target_inset) in [(3, 34.), (12, 34.), (3, 14.)] {
            let (mut app, ctx) = fixture(count);
            app.active = 1;
            let active_id = app.tabs[1].id;
            let first = app.tabs[0].id;
            let from = bounds(&mut app, &ctx, "document-0.pdf").center();
            let to_rect = bounds(&mut app, &ctx, "document-2.pdf");
            let to = pos2(to_rect.right() - target_inset, to_rect.center().y);
            pointer(&mut app, &ctx, from, egui::PointerButton::Primary, true);
            frame(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(from + vec2(15., 0.))],
            );
            frame(&mut app, &ctx, vec![egui::Event::PointerMoved(to)]);
            pointer(&mut app, &ctx, to, egui::PointerButton::Primary, false);
            assert_eq!(app.tabs[2].id, first);
            assert_eq!(app.tabs[app.active].id, active_id);
            assert!(app.tabs.iter().all(dirty));
        }
    }

    #[test]
    fn windowed_tab_drags_never_request_native_window_movement() {
        for width in [720., 900., 1200.] {
            let (mut app, ctx) = fixture(8);
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("test_width"), width));
            let source = app.tabs[7].id;
            let from = bounds(&mut app, &ctx, "document-7.pdf").center();
            let to = from - vec2(200., 0.);
            pointer(&mut app, &ctx, from, egui::PointerButton::Primary, true);
            for step in 1..=10 {
                frame(
                    &mut app,
                    &ctx,
                    vec![egui::Event::PointerMoved(from.lerp(to, step as f32 / 10.))],
                );
            }
            pointer(&mut app, &ctx, to, egui::PointerButton::Primary, false);
            assert!(app.tabs.iter().position(|t| t.id == source).unwrap() < 7);
            assert_eq!(app.tabs[app.active].id, source);
        }
    }

    #[test]
    fn native_press_move_release_in_one_redraw_reorders_tabs() {
        let (mut app, ctx) = fixture(3);
        let first = app.tabs[0].id;
        let from = bounds(&mut app, &ctx, "document-0.pdf").center();
        let target = bounds(&mut app, &ctx, "document-2.pdf");
        let to = pos2(target.right() - 34., target.center().y);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(from),
                button(from, true),
                egui::Event::PointerMoved(to),
                button(to, false),
            ],
        );
        assert_eq!(app.tabs[2].id, first);
        assert!(app.tabs.iter().all(dirty));
    }

    #[test]
    fn escaping_a_tab_drag_preserves_order() {
        let (mut app, ctx) = fixture(3);
        let ids: Vec<_> = app.tabs.iter().map(|t| t.id).collect();
        let from = bounds(&mut app, &ctx, "document-0.pdf").center();
        let to = bounds(&mut app, &ctx, "document-2.pdf").center();
        pointer(&mut app, &ctx, from, egui::PointerButton::Primary, true);
        frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(to),
                egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        pointer(&mut app, &ctx, to, egui::PointerButton::Primary, false);
        assert_eq!(app.tabs.iter().map(|t| t.id).collect::<Vec<_>>(), ids);
    }

    #[test]
    fn dropping_in_empty_strip_space_moves_tab_to_end() {
        let (mut app, ctx) = fixture(3);
        let first = app.tabs[0].id;
        let from = bounds(&mut app, &ctx, "document-0.pdf").center();
        let last = bounds(&mut app, &ctx, "document-2.pdf");
        let to = pos2(last.right() + 45., last.center().y);
        pointer(&mut app, &ctx, from, egui::PointerButton::Primary, true);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(from + vec2(15., 0.))],
        );
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(to)]);
        pointer(&mut app, &ctx, to, egui::PointerButton::Primary, false);
        assert_eq!(app.tabs[2].id, first);
    }

    #[test]
    fn right_click_exposes_file_close_and_group_actions() {
        let (mut app, ctx) = fixture(3);
        let at = bounds(&mut app, &ctx, "document-0.pdf").center();
        pointer(&mut app, &ctx, at, egui::PointerButton::Secondary, true);
        pointer(&mut app, &ctx, at, egui::PointerButton::Secondary, false);
        for name in [
            "Switch Group",
            "New Group…",
            "Move to Group…",
            "Close Other Tabs",
            "Close Tabs to the Right",
            "Reopen Closed Tab",
        ] {
            bounds(&mut app, &ctx, name);
        }
    }

    #[test]
    fn bulk_close_stops_on_cancel_and_never_closes_another_group() {
        let (mut app, ctx) = fixture(4);
        let other = app.create_group("Work").unwrap();
        let protected = app.tabs[3].id;
        app.move_tab_to_group(protected, other);
        app.switch_group(0);
        let keep = app.tabs[0].id;
        let dirty_id = app.tabs[1].id;
        app.tab_action(Action::CloseOthers(keep), &ctx);
        assert_eq!(app.close_intent, Some(Some(dirty_id)));
        assert_eq!(app.tabs.len(), 4);
        let cancel = bounds(&mut app, &ctx, "Cancel").center();
        pointer(&mut app, &ctx, cancel, egui::PointerButton::Primary, true);
        pointer(&mut app, &ctx, cancel, egui::PointerButton::Primary, false);
        assert!(app.close_intent.is_none());
        assert!(app.tab_controls.close_queue.is_empty());
        app.advance_tab_closes(&ctx);
        assert_eq!(app.tabs.len(), 4);
        for tab in &mut app.tabs {
            tab.unsaved_on_load = false;
            if let Some(session) = &mut tab.session {
                session.mark_saved(session.revision());
            }
        }
        app.tab_action(Action::CloseOthers(keep), &ctx);
        assert_eq!(
            app.tabs.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![keep, protected]
        );
    }

    #[test]
    fn groups_remember_selection_cycle_locally_and_remove_without_closing() {
        let (mut app, ctx) = fixture(4);
        let work = app.create_group("Work").unwrap();
        app.move_tab_to_group(app.tabs[2].id, work);
        app.move_tab_to_group(app.tabs[3].id, work);
        app.switch_group(work);
        app.cycle_group_tab(false);
        let selected = app.tabs[app.active].id;
        app.switch_group(0);
        app.cycle_group_tab(false);
        assert_eq!(app.tabs[app.active].document_group, 0);
        app.switch_group(work);
        assert_eq!(app.tabs[app.active].id, selected);
        app.cycle_group_tab(true);
        assert_eq!(app.tabs[app.active].document_group, work);
        let kept = app.tabs.iter().map(|t| t.id).collect::<Vec<_>>();
        app.delete_group(work);
        assert_eq!(app.tab_controls.current_group, 0);
        assert!(app.tabs.iter().all(|t| t.document_group == 0 && dirty(t)));
        assert_eq!(app.tabs.iter().map(|t| t.id).collect::<Vec<_>>(), kept);
        assert!(app.create_group("  ").is_none());
        assert!(app.create_group("ungrouped").is_none());
        frame(&mut app, &ctx, vec![]);
    }

    #[test]
    fn chooser_searches_filenames_and_folders_and_keeps_focus() {
        let (mut app, ctx) = fixture(3);
        app.tabs[0].path = PathBuf::from("/Work/İstanbul/Annual report.pdf");
        assert!(matches(&app.tabs[0], "work annual"));
        assert!(!matches(&app.tabs[0], "annual budget"));
        let at = bounds(&mut app, &ctx, "Open documents (3)").center();
        pointer(&mut app, &ctx, at, egui::PointerButton::Primary, true);
        pointer(&mut app, &ctx, at, egui::PointerButton::Primary, false);
        frame(&mut app, &ctx, vec![egui::Event::Text("annual".into())]);
        assert_eq!(app.tab_controls.query, "annual");
        bounds(&mut app, &ctx, "● Annual report.pdf");
        frame(&mut app, &ctx, vec![]);
        assert!(egui::Popup::is_any_open(&ctx));
    }

    #[test]
    fn named_group_is_compact_and_context_menu_can_switch_and_remove_it() {
        let (mut app, ctx) = fixture(3);
        let work = app
            .create_group("Work with a very long descriptive name")
            .unwrap();
        app.move_tab_to_group(app.tabs[0].id, work);
        app.switch_group(work);
        let name = bounds(&mut app, &ctx, "Open documents (1)");
        assert!(name.width() <= 97.);
        pointer(
            &mut app,
            &ctx,
            name.center(),
            egui::PointerButton::Secondary,
            true,
        );
        pointer(
            &mut app,
            &ctx,
            name.center(),
            egui::PointerButton::Secondary,
            false,
        );
        bounds(&mut app, &ctx, "Switch Group");
        bounds(&mut app, &ctx, "Rename Group…");
        let remove = bounds(&mut app, &ctx, "Remove Group").center();
        pointer(&mut app, &ctx, remove, egui::PointerButton::Primary, true);
        pointer(&mut app, &ctx, remove, egui::PointerButton::Primary, false);
        assert_eq!(app.tab_controls.current_group, 0);
        assert_eq!(app.tabs.len(), 3);
        let output = frame(&mut app, &ctx, vec![]);
        let nodes = output.platform_output.accesskit_update.unwrap().nodes;
        assert!(
            !nodes
                .iter()
                .any(|(_, n)| n.label() == Some("Ungrouped") || n.label() == Some("Ungrouped ▾"))
        );
    }

    #[test]
    fn group_menu_creates_a_named_working_set() {
        let (mut app, ctx) = fixture(2);
        let tab = bounds(&mut app, &ctx, "document-1.pdf");
        let group_button = pos2(tab.right() + 30., tab.center().y);
        pointer(
            &mut app,
            &ctx,
            group_button,
            egui::PointerButton::Secondary,
            true,
        );
        pointer(
            &mut app,
            &ctx,
            group_button,
            egui::PointerButton::Secondary,
            false,
        );
        let create = bounds(&mut app, &ctx, "New Group…").center();
        pointer(&mut app, &ctx, create, egui::PointerButton::Primary, true);
        pointer(&mut app, &ctx, create, egui::PointerButton::Primary, false);
        assert!(app.tab_controls.group_editor.is_some());
        // Populate the same draft the dialog's editable field uses.
        app.tab_controls.group_editor.as_mut().unwrap().1 = "Work".into();
        let create = bounds(&mut app, &ctx, "Create").center();
        pointer(&mut app, &ctx, create, egui::PointerButton::Primary, true);
        pointer(&mut app, &ctx, create, egui::PointerButton::Primary, false);
        assert!(app.tab_controls.group_editor.is_none());
        assert_eq!(app.tab_controls.groups[1].name, "Work");
        assert_eq!(
            app.tab_controls.current_group,
            app.tab_controls.groups[1].id
        );
        assert!(app.home.visible);
        bounds(&mut app, &ctx, "Open documents (0)");
        assert_eq!(app.tabs.len(), 2);
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub(super) struct Group {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub active: Option<PathBuf>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            query: String::new(),
            offset: 0.,
            drag: None,
            window_drag_requested: false,
            close_queue: Default::default(),
            waiting: None,
            groups: vec![Group {
                id: 0,
                name: "Ungrouped".into(),
                active: None,
            }],
            current_group: 0,
            remembered: HashMap::new(),
            group_editor: None,
        }
    }
}

#[derive(Clone, Copy)]
struct TabDrag {
    source: u64,
    start: egui::Pos2,
    moved: bool,
}

#[derive(Clone, Copy)]
struct TabContext<'a> {
    groups: &'a [Group],
    current_group: u64,
    dragging: Option<u64>,
}

impl State {
    pub(super) fn request_window_drag(&mut self) {
        self.window_drag_requested = true;
    }

    pub(super) fn take_window_drag(&mut self) -> bool {
        std::mem::take(&mut self.window_drag_requested)
    }

    fn tab_gesture(
        &mut self,
        events: &[egui::Event],
        targets: &[(u64, Rect)],
        strip: Rect,
        enabled: bool,
    ) -> Option<Action> {
        let mut action = None;
        if !enabled {
            self.drag = None;
            return None;
        }
        for event in events {
            match *event {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                } => {
                    self.drag = targets
                        .iter()
                        .find(|(_, rect)| {
                            strip.contains(pos)
                                && rect.contains(pos)
                                && !Rect::from_center_size(
                                    pos2(rect.right() - 14., rect.center().y),
                                    vec2(20., 20.),
                                )
                                .contains(pos)
                        })
                        .map(|(source, _)| TabDrag {
                            source: *source,
                            start: pos,
                            moved: false,
                        });
                }
                egui::Event::PointerMoved(pos) => {
                    if let Some(drag) = &mut self.drag {
                        drag.moved |= drag.start.distance(pos) > 6.;
                    }
                }
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    ..
                } => {
                    if let Some(drag) = self.drag.take()
                        && (drag.moved || drag.start.distance(pos) > 6.)
                        && strip.contains(pos)
                        && let Some((target, rect)) = targets
                            .iter()
                            .find(|(_, r)| pos.x <= r.right())
                            .or_else(|| targets.last())
                    {
                        action = Some(Action::Move {
                            source: drag.source,
                            target: *target,
                            after: pos.x > rect.center().x,
                        });
                    }
                }
                egui::Event::WindowFocused(false)
                | egui::Event::Key {
                    key: egui::Key::Escape,
                    pressed: true,
                    ..
                } => self.drag = None,
                _ => {}
            }
        }
        action
    }
}

enum Action {
    Select(u64),
    SwitchGroup(u64),
    NewGroup,
    RenameGroup,
    RemoveGroup,
    Close(u64),
    CloseOthers(u64),
    CloseRight(u64),
    Reopen,
    Reveal(PathBuf),
    MoveToGroup {
        tab: u64,
        group: u64,
    },
    Move {
        source: u64,
        target: u64,
        after: bool,
    },
}

/// Shrink evenly, with extra room reserved for the active filename.
fn widths(natural: &[f32], active: Option<usize>, available: f32) -> Vec<f32> {
    let minimum: Vec<_> = (0..natural.len())
        .map(|i| if active == Some(i) { 156. } else { 104. })
        .collect();
    let mut widths: Vec<_> = natural
        .iter()
        .zip(&minimum)
        .map(|(&w, &min)| w.clamp(min, 220.))
        .collect();
    let budget = (available - 4. * natural.len().saturating_sub(1) as f32).max(0.);
    for _ in 0..natural.len() {
        let excess = widths.iter().sum::<f32>() - budget;
        let flexible = widths
            .iter()
            .zip(&minimum)
            .filter(|(w, m)| **w > **m)
            .count();
        if excess <= 0.1 || flexible == 0 {
            break;
        }
        for (width, &min) in widths.iter_mut().zip(&minimum) {
            *width = (*width - excess / flexible as f32).max(min);
        }
    }
    widths
}

fn dirty(tab: &Tab) -> bool {
    tab.unsaved_on_load || tab.session.as_ref().is_some_and(|s| s.dirty())
}

fn matches(tab: &Tab, query: &str) -> bool {
    let haystack = tab.path.to_string_lossy().to_lowercase();
    query.split_whitespace().all(|word| haystack.contains(word))
}

fn tab_chip(
    ui: &mut egui::Ui,
    tab: &Tab,
    width: f32,
    active: bool,
    reveal: bool,
    menu: (bool, bool, bool),
    context: TabContext<'_>,
) -> Option<Action> {
    let mut action = None;
    let name = tab.name();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 30.), egui::Sense::click_and_drag());
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, &name));
    if reveal {
        response.scroll_to_me(Some(egui::Align::Center));
    }
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let after = pointer.is_some_and(|p| p.x > rect.center().x);
    let insertion_marker = context.dragging.is_some_and(|source| source != tab.id)
        && pointer.is_some_and(|p| rect.intersect(ui.clip_rect()).contains(p));
    let close_rect =
        Rect::from_center_size(pos2(rect.right() - 14., rect.center().y), vec2(20., 20.));
    let close = ui.interact(close_rect, ui.id().with("close"), egui::Sense::click());
    close.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Close tab"));
    let painter = ui.painter();
    if active {
        painter.rect_filled(rect, theme::style().radius(), theme::raised());
        painter.line_segment(
            [
                pos2(rect.left() + 10., rect.bottom()),
                pos2(rect.right() - 10., rect.bottom()),
            ],
            Stroke::new(2., theme::accent()),
        );
    } else if response.hovered() || close.hovered() {
        painter.rect_filled(rect, theme::style().radius(), theme::hover());
    }
    if insertion_marker {
        let x = if after { rect.right() } else { rect.left() };
        painter.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(2., theme::accent()),
        );
    }
    let color = if active { theme::ink() } else { theme::muted() };
    let mut job = egui::text::LayoutJob::simple_singleline(
        tab_caption(&name),
        FontId::proportional(13.),
        color,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width((width - 48.).max(10.));
    let text = ui.fonts_mut(|f| f.layout_job(job));
    painter.galley(
        pos2(rect.left() + 10., rect.center().y - text.size().y / 2.),
        text,
        color,
    );
    if dirty(tab) {
        painter.circle_filled(
            pos2(close_rect.left() - 5., rect.center().y),
            2.5,
            theme::accent(),
        );
    }
    if active || response.hovered() || close.hovered() {
        if close.hovered() {
            painter.rect_filled(close_rect, 4, theme::pressed());
        }
        ui::draw_icon(painter, close_rect.shrink(5.), Icon::Close, theme::muted());
    }
    if close.on_hover_text("Close tab").clicked() {
        action = Some(Action::Close(tab.id));
    } else if response.clicked() {
        action = Some(Action::Select(tab.id));
    }
    response.context_menu(|ui| {
        ui.set_max_width(230.);
        if let Some(command) = group_actions(ui, context.groups, context.current_group) {
            action = Some(command);
        }
        ui.separator();
        icon_submenu(ui, Icon::Export, "Move to Group…", |ui| {
            for group in context.groups {
                if ui
                    .add_enabled(
                        group.id != tab.document_group,
                        ui::MenuItem::new(Icon::Group, &group.name).width(230.),
                    )
                    .clicked()
                {
                    action = Some(Action::MoveToGroup {
                        tab: tab.id,
                        group: group.id,
                    });
                    ui.close();
                }
            }
        });
        ui.separator();
        let finder = if cfg!(target_os = "macos") {
            "Reveal in Finder"
        } else {
            "Show in folder"
        };
        if ui
            .add_enabled(
                tab.has_destination && tab.path.is_file(),
                ui::MenuItem::new(Icon::Open, finder).width(230.),
            )
            .clicked()
        {
            action = Some(Action::Reveal(tab.path.clone()));
            ui.close();
        }
        ui.separator();
        for (label, icon, enabled, command) in [
            ("Close Tab", Icon::Close, true, Action::Close(tab.id)),
            (
                "Close Other Tabs",
                Icon::Close,
                menu.0,
                Action::CloseOthers(tab.id),
            ),
            (
                "Close Tabs to the Right",
                Icon::Right,
                menu.1,
                Action::CloseRight(tab.id),
            ),
        ] {
            if ui
                .add_enabled(enabled, ui::MenuItem::new(icon, label).width(230.))
                .clicked()
            {
                action = Some(command);
                ui.close();
            }
        }
        ui.separator();
        if ui
            .add_enabled(
                menu.2,
                ui::MenuItem::new(Icon::Undo, "Reopen Closed Tab").width(230.),
            )
            .clicked()
        {
            action = Some(Action::Reopen);
            ui.close();
        }
    });
    response.on_hover_text(format!("{}\n{}", name, tab.path.display()));
    action
}

fn icon_submenu(ui: &mut egui::Ui, icon: Icon, label: &str, content: impl FnOnce(&mut egui::Ui)) {
    let response = ui.add(ui::MenuItem::new(icon, label).width(230.));
    ui::draw_icon(
        ui.painter(),
        Rect::from_center_size(
            pos2(response.rect.right() - 10., response.rect.center().y),
            vec2(12., 12.),
        ),
        Icon::Right,
        ui.visuals().text_color(),
    );
    egui::containers::menu::SubMenu::new().show(ui, &response, content);
}

fn group_actions(ui: &mut egui::Ui, groups: &[Group], current: u64) -> Option<Action> {
    let mut action = None;
    ui.set_max_width(240.);
    icon_submenu(ui, Icon::Group, "Switch Group", |ui| {
        for group in groups {
            if ui
                .add(
                    ui::MenuItem::new(
                        if group.id == current {
                            Icon::Check
                        } else {
                            Icon::Group
                        },
                        &group.name,
                    )
                    .width(230.),
                )
                .clicked()
            {
                action = Some(Action::SwitchGroup(group.id));
                ui.close();
            }
        }
    });
    if ui
        .add_enabled(
            groups.len() < 32,
            ui::MenuItem::new(Icon::Plus, "New Group…").width(230.),
        )
        .clicked()
    {
        action = Some(Action::NewGroup);
        ui.close();
    }
    if current != 0 {
        if ui
            .add(ui::MenuItem::new(Icon::Text, "Rename Group…").width(230.))
            .clicked()
        {
            action = Some(Action::RenameGroup);
            ui.close();
        }
        if ui
            .add(ui::MenuItem::new(Icon::Ungroup, "Remove Group").width(230.))
            .on_hover_text("Moves its documents to Ungrouped. No documents are closed.")
            .clicked()
        {
            action = Some(Action::RemoveGroup);
            ui.close();
        }
    }
    action
}

impl Viewer {
    pub(super) fn sync_tab_group(&mut self) {
        if self.home.visible {
            return;
        }
        if let Some(tab) = self.tabs.get(self.active) {
            self.tab_controls.current_group = tab.document_group;
            self.tab_controls
                .remembered
                .insert(tab.document_group, tab.id);
            if let Some(group) = self
                .tab_controls
                .groups
                .iter_mut()
                .find(|g| g.id == tab.document_group)
            {
                group.active =
                    (tab.has_destination && tab.path.is_file()).then(|| tab.path.clone());
            }
        }
    }

    pub(super) fn switch_group(&mut self, id: u64) {
        if !self.tab_controls.groups.iter().any(|g| g.id == id) {
            return;
        }
        self.sync_tab_group();
        self.tab_controls.current_group = id;
        self.tab_controls.drag = None;
        let remembered = self.tab_controls.remembered.get(&id).copied();
        let saved_path = self
            .tab_controls
            .groups
            .iter()
            .find(|g| g.id == id)
            .and_then(|g| g.active.as_ref());
        let chosen = self
            .tabs
            .iter()
            .position(|t| t.document_group == id && Some(t.id) == remembered)
            .or_else(|| {
                self.tabs
                    .iter()
                    .position(|t| t.document_group == id && Some(&t.path) == saved_path)
            })
            .or_else(|| self.tabs.iter().position(|t| t.document_group == id));
        if let Some(index) = chosen {
            self.active = index;
            self.home.visible = false;
        } else {
            self.home.visible = true;
        }
        self.tab_controls.offset = 0.;
        self.tab_revealed = None;
        self.workspace.invalidate();
    }

    pub(super) fn cycle_group_tab(&mut self, backwards: bool) {
        self.sync_tab_group();
        let visible: Vec<_> = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, t)| t.document_group == self.tab_controls.current_group)
            .map(|(i, _)| i)
            .collect();
        if visible.is_empty() {
            return;
        }
        let current = visible.iter().position(|&i| i == self.active);
        let next = current
            .map(|i| (i + if backwards { visible.len() - 1 } else { 1 }) % visible.len())
            .unwrap_or(0);
        self.active = visible[next];
        self.home.visible = false;
    }

    fn create_group(&mut self, name: &str) -> Option<u64> {
        let name: String = name.trim().chars().take(40).collect();
        if name.is_empty()
            || self.tab_controls.groups.len() >= 32
            || self
                .tab_controls
                .groups
                .iter()
                .any(|g| g.name.to_lowercase() == name.to_lowercase())
        {
            return None;
        }
        let id = self
            .tab_controls
            .groups
            .iter()
            .map(|g| g.id)
            .max()
            .unwrap_or(0)
            .checked_add(1)?;
        self.tab_controls.groups.push(Group {
            id,
            name,
            active: None,
        });
        self.switch_group(id);
        Some(id)
    }

    fn delete_group(&mut self, id: u64) {
        if id == 0 {
            return;
        }
        for tab in &mut self.tabs {
            if tab.document_group == id {
                tab.document_group = 0;
            }
        }
        for tab in &mut self.workspace.closed {
            if tab.document_group == id {
                tab.document_group = 0;
            }
        }
        self.tab_controls.groups.retain(|g| g.id != id);
        self.tab_controls.remembered.remove(&id);
        if self.tab_controls.current_group == id {
            self.switch_group(0);
        }
        self.workspace.invalidate();
    }

    fn move_tab_to_group(&mut self, tab_id: u64, group_id: u64) {
        if !self.tab_controls.groups.iter().any(|g| g.id == group_id) {
            return;
        }
        let Some(index) = self.tabs.iter().position(|t| t.id == tab_id) else {
            return;
        };
        let old_group = self.tabs[index].document_group;
        self.tabs[index].document_group = group_id;
        // Stay in the original working set; show another document or Home.
        if self.active == index && old_group == self.tab_controls.current_group {
            self.home.visible = true;
            self.switch_group(old_group);
        }
        self.workspace.invalidate();
    }

    pub(super) fn tab_group_dialog(&mut self, ctx: &egui::Context) {
        let Some((id, mut name)) = self.tab_controls.group_editor.take() else {
            return;
        };
        let mut open = true;
        let mut save = false;
        let mut cancel = false;
        let valid =
            !name.trim().is_empty()
                && !self.tab_controls.groups.iter().any(|g| {
                    Some(g.id) != id && g.name.to_lowercase() == name.trim().to_lowercase()
                });
        egui::Window::new(if id.is_some() {
            "Rename Tab Group"
        } else {
            "New Tab Group"
        })
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label("Group name");
            let response = ui.add(
                egui::TextEdit::singleline(&mut name)
                    .char_limit(40)
                    .desired_width(260.),
            );
            self.editor.text_editing |= response.has_focus();
            if !valid && !name.trim().is_empty() {
                ui.weak("Choose a different group name.");
            }
            ui.horizontal(|ui| {
                save = ui
                    .add_enabled(
                        valid,
                        egui::Button::new(if id.is_some() { "Rename" } else { "Create" }),
                    )
                    .clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if save && valid {
            if let Some(id) = id {
                if let Some(group) = self.tab_controls.groups.iter_mut().find(|g| g.id == id) {
                    group.name = name.trim().into();
                }
                self.workspace.invalidate();
            } else {
                self.create_group(&name);
            }
        } else if open && !cancel {
            self.tab_controls.group_editor = Some((id, name));
        }
    }

    pub(super) fn document_tabs(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.sync_tab_group();
        let mut action = None;
        let mut targets = Vec::new();
        let mut strip_rect = Rect::NOTHING;
        // Register behind the children so tabs keep their click/drag gestures,
        // while empty space and gaps offer the same group management menu.
        let area = ui.interact(
            Rect::from_min_size(
                ui.next_widget_position(),
                vec2((ui.available_width() - 124.).max(1.), 32.),
            ),
            ui.id().with("tab_area"),
            egui::Sense::click_and_drag(),
        );
        let group_name = self
            .tab_controls
            .groups
            .iter()
            .find(|g| g.id == self.tab_controls.current_group && g.id != 0)
            .map(|g| g.name.as_str());
        let visible: Vec<_> = self
            .tabs
            .iter()
            .filter(|t| t.document_group == self.tab_controls.current_group)
            .collect();
        let label = format!("Open documents ({})", visible.len());
        let group_font = ui::bold(ui, 10.);
        let chooser_width = group_name.map_or(30., |name| {
            ui.fonts_mut(|f| {
                f.layout_no_wrap(name.into(), group_font.clone(), theme::ink())
                    .size()
                    .x
                    + 12.
            })
            .clamp(30., 96.)
        });
        // The caller renders Open, Settings and Appearance after this area.
        let available = (ui.available_width() - chooser_width - 124.).max(36.);
        let selected = (!self.home.visible)
            .then(|| self.tabs.get(self.active).map(|t| t.id))
            .flatten();
        let natural: Vec<_> = visible
            .iter()
            .map(|tab| {
                ui.fonts_mut(|f| {
                    f.layout_no_wrap(
                        tab_caption(&tab.name()),
                        FontId::proportional(13.),
                        theme::ink(),
                    )
                    .size()
                    .x + 48.
                })
            })
            .collect();
        let tab_widths = widths(
            &natural,
            visible.iter().position(|t| Some(t.id) == selected),
            available,
        );
        let content_width =
            tab_widths.iter().sum::<f32>() + 4. * visible.len().saturating_sub(1) as f32;
        let overflow = content_width > available + 0.5;
        let viewport = if overflow {
            (available - 72.).max(24.)
        } else {
            available
        };
        let reveal = selected != self.tab_revealed || (self.tab_strip_width - viewport).abs() > 1.;
        self.tab_revealed = selected;
        self.tab_strip_width = viewport;
        if !overflow {
            self.tab_controls.offset = 0.;
        }
        if overflow
            && ui
                .add_enabled(
                    self.tab_controls.offset > 1.,
                    ActionButton::new(Icon::Left, "Scroll tabs left").compact(),
                )
                .clicked()
        {
            self.tab_controls.offset = (self.tab_controls.offset - viewport * 0.75).max(0.);
        }
        ui.allocate_ui_with_layout(
            vec2(viewport, 32.),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                let output = egui::ScrollArea::horizontal()
                    .id_salt("tab_strip")
                    .scroll_source(egui::scroll_area::ScrollSource {
                        drag: false,
                        ..Default::default()
                    })
                    .horizontal_scroll_offset(self.tab_controls.offset)
                    .auto_shrink([false, true])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for (i, tab) in visible.iter().enumerate() {
                                let result = ui.push_id(tab.id, |ui| {
                                    tab_chip(
                                        ui,
                                        tab,
                                        tab_widths[i],
                                        selected == Some(tab.id),
                                        reveal && selected == Some(tab.id),
                                        (
                                            visible.len() > 1,
                                            i + 1 < visible.len(),
                                            !self.workspace.closed.is_empty(),
                                        ),
                                        TabContext {
                                            groups: &self.tab_controls.groups,
                                            current_group: self.tab_controls.current_group,
                                            dragging: self
                                                .tab_controls
                                                .drag
                                                .filter(|d| d.moved)
                                                .map(|d| d.source),
                                        },
                                    )
                                });
                                targets.push((tab.id, result.response.rect));
                                if result.inner.is_some() {
                                    action = result.inner;
                                }
                            }
                            ui.allocate_response(
                                vec2(ui.available_width().max(1.), 30.),
                                egui::Sense::hover(),
                            );
                        });
                    });
                strip_rect = output.inner_rect;
                self.tab_controls.offset = output.state.offset.x;
                if overflow
                    && self.tab_controls.drag.is_some_and(|d| d.moved)
                    && let Some(pos) = ctx.input(|i| i.pointer.interact_pos())
                    && output.inner_rect.contains(pos)
                {
                    let direction = if pos.x < output.inner_rect.left() + 24. {
                        -1.
                    } else if pos.x > output.inner_rect.right() - 24. {
                        1.
                    } else {
                        0.
                    };
                    self.tab_controls.offset = (self.tab_controls.offset + direction * 8.)
                        .clamp(0., (content_width - viewport).max(0.));
                    if direction != 0. {
                        ctx.request_repaint_after(std::time::Duration::from_millis(16));
                    }
                }
            },
        );
        if overflow
            && ui
                .add_enabled(
                    self.tab_controls.offset + viewport < content_width - 1.,
                    ActionButton::new(Icon::Right, "Scroll tabs right").compact(),
                )
                .clicked()
        {
            self.tab_controls.offset = (self.tab_controls.offset + viewport * 0.75)
                .min((content_width - viewport).max(0.));
        }
        let (rect, response) =
            ui.allocate_exact_size(vec2(chooser_width, 30.), egui::Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
        if response.hovered() || response.has_focus() {
            ui.painter().rect_filled(rect, 5., theme::hover());
        }
        let center = pos2(
            rect.center().x,
            rect.top() + if group_name.is_some() { 8. } else { 15. },
        );
        ui.painter().add(egui::Shape::line(
            vec![
                center + vec2(-5., -2.),
                center + vec2(0., 3.),
                center + vec2(5., -2.),
            ],
            Stroke::new(1.8, theme::ink()),
        ));
        if let Some(name) = group_name {
            let mut job =
                egui::text::LayoutJob::simple_singleline(name.into(), group_font, theme::ink());
            job.wrap = egui::text::TextWrapping::truncate_at_width(chooser_width - 8.);
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            ui.painter().galley(
                pos2(
                    rect.center().x - galley.size().x / 2.,
                    rect.bottom() - galley.size().y - 1.,
                ),
                galley,
                theme::ink(),
            );
        }
        response.clone().on_hover_text(format!(
            "{}{}\nRight-click to manage or switch tab groups",
            label,
            group_name.map(|n| format!(" · {n}")).unwrap_or_default()
        ));
        egui::Popup::context_menu(&response)
            .id(response.id.with("tab_groups"))
            .show(|ui| {
                action = group_actions(
                    ui,
                    &self.tab_controls.groups,
                    self.tab_controls.current_group,
                );
            });
        if response.clicked() {
            self.tab_controls.query.clear();
        }
        egui::Popup::menu(&response)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_width(ui::dialog_width(ctx, 360.));
                ui.spacing_mut().item_spacing.y = 3.;
                ui.strong(group_name.unwrap_or("Open documents"));
                let search = ui.add(
                    egui::TextEdit::singleline(&mut self.tab_controls.query)
                        .hint_text("Find an open document…")
                        .desired_width(f32::INFINITY),
                );
                if response.clicked() {
                    search.request_focus();
                }
                self.editor.text_editing |= search.has_focus();
                ui.separator();
                let query = self.tab_controls.query.to_lowercase();
                let mut found = false;
                egui::ScrollArea::vertical()
                    .max_height(320.)
                    .show(ui, |ui| {
                        for tab in &visible {
                            if !matches(tab, &query) {
                                continue;
                            }
                            found = true;
                            ui.push_id(("document_choice", tab.id), |ui| {
                                ui.horizontal(|ui| {
                                    let caption = format!(
                                        "{}{}",
                                        if dirty(tab) { "● " } else { "" },
                                        tab.name()
                                    );
                                    let w =
                                        (ui.available_width() - 32. - ui.spacing().item_spacing.x)
                                            .max(40.);
                                    if ui
                                        .add_sized(
                                            [w, 26.],
                                            egui::Button::new((caption, egui::Atom::grow()))
                                                .selected(selected == Some(tab.id))
                                                .truncate(),
                                        )
                                        .on_hover_text(tab.path.display().to_string())
                                        .clicked()
                                    {
                                        action = Some(Action::Select(tab.id));
                                        ui.close();
                                    }
                                    if ui
                                        .add(
                                            ActionButton::new(Icon::Close, "Close document")
                                                .compact(),
                                        )
                                        .clicked()
                                    {
                                        action = Some(Action::Close(tab.id));
                                        ui.close();
                                    }
                                });
                                let folder = if tab.has_destination {
                                    tab.path
                                        .parent()
                                        .map(|p| {
                                            let mut parts: Vec<_> = p
                                                .components()
                                                .rev()
                                                .take(2)
                                                .map(|part| {
                                                    part.as_os_str().to_string_lossy().into_owned()
                                                })
                                                .collect();
                                            parts.reverse();
                                            parts.join(std::path::MAIN_SEPARATOR_STR)
                                        })
                                        .unwrap_or_default()
                                } else {
                                    "Not saved yet".into()
                                };
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(folder).small().color(theme::muted()),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(tab.path.display().to_string());
                            });
                        }
                    });
                if !found {
                    ui.weak(if visible.is_empty() {
                        "No documents in this group"
                    } else {
                        "No matching documents"
                    });
                }
                ui.separator();
                ui.weak("Ctrl+Tab / Ctrl+Shift+Tab to switch");
            });
        let events = ctx.input(|i| i.events.clone());
        if let Some(command) = self.tab_controls.tab_gesture(
            &events,
            &targets,
            strip_rect,
            !egui::Popup::is_any_open(ctx),
        ) {
            action = Some(command);
        }
        // Decide ownership from the original press, never from the absence of
        // a tab drag after its release. Close buttons and clipped tabs also
        // belong to the tab strip and must never initiate window movement.
        if area.drag_started()
            && !egui::Popup::is_any_open(ctx)
            && ctx.input(|i| {
                i.pointer.primary_down()
                    && i.pointer.press_origin().is_some_and(|origin| {
                        strip_rect.contains(origin)
                            && !targets.iter().any(|(_, rect)| rect.contains(origin))
                    })
            })
        {
            self.tab_controls.request_window_drag();
        }
        if self.tab_controls.drag.is_some_and(|d| d.moved) {
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        area.context_menu(|ui| {
            action = group_actions(
                ui,
                &self.tab_controls.groups,
                self.tab_controls.current_group,
            );
        });
        if let Some(action) = action {
            self.tab_action(action, ctx);
        }
    }

    fn tab_action(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::SwitchGroup(id) => self.switch_group(id),
            Action::NewGroup => self.tab_controls.group_editor = Some((None, String::new())),
            Action::RenameGroup => {
                if let Some(group) = self
                    .tab_controls
                    .groups
                    .iter()
                    .find(|g| g.id == self.tab_controls.current_group && g.id != 0)
                {
                    self.tab_controls.group_editor = Some((Some(group.id), group.name.clone()));
                }
            }
            Action::RemoveGroup => self.delete_group(self.tab_controls.current_group),
            Action::Select(id) => {
                if let Some(i) = self.tabs.iter().position(|t| t.id == id) {
                    self.active = i;
                    self.home.visible = false;
                }
            }
            Action::Close(id) => self.request_close(Some(id), ctx),
            Action::CloseOthers(id) | Action::CloseRight(id) => {
                if self.close_intent.is_some() || self.close_after_save.is_some() {
                    return;
                }
                let right = matches!(action, Action::CloseRight(_));
                let Some(index) = self.tabs.iter().position(|t| t.id == id) else {
                    return;
                };
                let group = self.tabs[index].document_group;
                self.tab_controls.close_queue = self
                    .tabs
                    .iter()
                    .enumerate()
                    .filter(|(i, t)| {
                        t.document_group == group && if right { *i > index } else { t.id != id }
                    })
                    .map(|(_, t)| t.id)
                    .collect();
                self.tab_controls.waiting = None;
                self.advance_tab_closes(ctx);
            }
            Action::Reopen => self.reopen_closed(ctx),
            Action::Reveal(path) => {
                #[cfg(target_os = "macos")]
                let result = std::process::Command::new("open")
                    .arg("-R")
                    .arg(path)
                    .spawn();
                #[cfg(target_os = "windows")]
                let result = std::process::Command::new("explorer.exe")
                    .arg(format!("/select,{}", path.display()))
                    .spawn();
                #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                let result = std::process::Command::new("xdg-open")
                    .arg(path.parent().unwrap_or(std::path::Path::new(".")))
                    .spawn();
                match result {
                    Ok(mut child) => {
                        std::thread::spawn(move || {
                            let _ = child.wait();
                        });
                    }
                    Err(e) => self.status = Some((format!("Could not reveal file: {e}"), true)),
                }
            }
            Action::Move {
                source,
                target,
                after,
            } => self.reorder_tab(source, target, after),
            Action::MoveToGroup { tab, group } => self.move_tab_to_group(tab, group),
        }
    }

    fn reorder_tab(&mut self, source: u64, target: u64, after: bool) {
        if source == target {
            return;
        }
        let active = self.tabs.get(self.active).map(|t| t.id);
        let Some(from) = self.tabs.iter().position(|t| t.id == source) else {
            return;
        };
        let Some(to) = self.tabs.iter().position(|t| t.id == target) else {
            return;
        };
        if self.tabs[from].document_group != self.tabs[to].document_group {
            return;
        }
        let insertion = to + usize::from(after);
        let tab = self.tabs.remove(from);
        self.tabs
            .insert(insertion - usize::from(from < insertion), tab);
        if let Some(index) = self.tabs.iter().position(|t| Some(t.id) == active) {
            self.active = index;
        }
        self.tab_revealed = None;
        self.workspace.invalidate();
    }

    pub(super) fn cancel_tab_closes(&mut self) {
        self.tab_controls.close_queue.clear();
        self.tab_controls.waiting = None;
    }

    pub(super) fn advance_tab_closes(&mut self, ctx: &egui::Context) {
        if let Some(waiting) = self.tab_controls.waiting {
            if self.tabs.iter().any(|t| t.id == waiting) {
                if self.close_intent.is_none()
                    && self.close_after_save.is_none()
                    && self.content.after_text.is_none()
                    && !self.content.busy
                    && !self.saving
                {
                    self.cancel_tab_closes();
                }
                return;
            }
            self.tab_controls.waiting = None;
        }
        if self.close_intent.is_some() || self.close_after_save.is_some() {
            return;
        }
        while let Some(id) = self.tab_controls.close_queue.pop_front() {
            if !self.tabs.iter().any(|t| t.id == id) {
                continue;
            }
            self.request_close(Some(id), ctx);
            if self.tabs.iter().any(|t| t.id == id) {
                if self.close_intent.is_some() || self.content.after_text.is_some() {
                    self.tab_controls.waiting = Some(id);
                } else {
                    // A busy operation or unfinished form/shape blocked closing.
                    // Do not unexpectedly close more documents after a later edit.
                    self.cancel_tab_closes();
                }
                break;
            }
        }
    }
}
