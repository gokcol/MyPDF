//! Shared selection shortcuts. Page objects own keys only when a typing field
//! does not; Escape applies unfinished text before clearing its boundary.
use super::*;
use editing::Mode;
use mypdf_milestone0::annotations::Kind;

impl Viewer {
    pub(super) fn selection_keys(&mut self, ctx: &egui::Context) {
        if self.home.visible
            || self.digital.open
            || self.editor.signature_open
            || self.editor.library_open
            || self.editor.selection_dialog_open()
            || self.about_open
            || self.recovery.open
            || self.close_intent.is_some()
            || self.link_confirm.is_some()
            || self.accessible_text
            || self.comparison.is_some()
            || ctx.memory(|memory| memory.top_modal_layer().is_some())
            || self.productivity.owns_input()
            || self.organizer.busy
            || self.redaction.active
            || self.workbench_owns_shortcuts()
            || self.table.chooser
            || self
                .tabs
                .get(self.active)
                .is_some_and(|t| self.table.has_editor(t.id))
            || self.table.deferring_input()
            || self.editor.interaction_active()
            || egui::Popup::is_any_open(ctx)
            || !matches!(self.editor.mode, Mode::Edit | Mode::Annotate | Mode::Draw)
        {
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        if self.content.busy || self.content.image_nudge_pending(tab.id) {
            if self.editor.mode == Mode::Edit
                && self.content.image_nudge_pending(tab.id)
                && !ctx.wants_keyboard_input()
                && !self.editor.text_editing
            {
                let delta = arrow_delta(ctx);
                self.content.queue_image_nudge(tab.id, delta);
            }
            return;
        }
        let ids = selection::selected_ids(tab);
        let has_marks = !ids.is_empty();
        if (has_marks
            || self.content.has_keyboard_selection(tab.id)
            || self.editor.text_selection.is_some())
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            // A validation failure must keep its draft and visible boundary.
            self.finish_content_selection(ctx);
            let content_pending = self
                .tabs
                .get(self.active)
                .is_some_and(|tab| self.content.pending_text() == Some(tab.id));
            if content_pending && !self.content.busy {
                return;
            }
            if has_marks && self.apply_pending_annotation() {
                if let Some(focus) = ctx.memory(|memory| memory.focused()) {
                    ctx.memory_mut(|memory| memory.surrender_focus(focus));
                }
                selection::set(&mut self.tabs[self.active], &mut self.editor, vec![]);
            }
            self.editor.text_selection = None;
            return;
        }
        // Plain Enter and arrows retain their usual caret/numeric-field meaning.
        if ctx.wants_keyboard_input() || self.editor.text_editing {
            return;
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
            if self.focus_content_selection() {
                return;
            }
            let Some(tab) = self.tabs.get(self.active) else {
                return;
            };
            let selected = tab
                .session
                .as_ref()
                .and_then(|session| {
                    session
                        .annotations
                        .iter()
                        .find(|mark| ids.len() == 1 && ids.contains(&mark.id))
                })
                .cloned();
            if let Some(mark) = selected {
                self.inspector = true;
                if matches!(mark.kind, Kind::Note { .. }) {
                    self.editor.note_requested = Some((tab.id, mark.id));
                } else if mark.kind.text().is_some() && !mark.object.locked {
                    ctx.memory_mut(|memory| {
                        memory.request_focus(egui::Id::new((tab.id, &mark.id, "annotation_text")))
                    });
                }
            }
            return;
        }
        if matches!(self.editor.mode, Mode::Annotate | Mode::Draw) {
            if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::A)) {
                let tab = &mut self.tabs[self.active];
                let ids = tab
                    .session
                    .as_ref()
                    .map(|session| {
                        session
                            .annotations
                            .iter()
                            .filter(|mark| mark.page == tab.page && !mark.object.hidden)
                            .map(|mark| mark.id.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                selection::set(tab, &mut self.editor, ids);
            } else if has_marks
                && ctx.input_mut(|input| {
                    input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                        || input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
                })
            {
                self.editor_delete();
            }
        }
        let delta = arrow_delta(ctx);
        if delta != [0., 0.] {
            if self.nudge_content_selection(ctx, delta) {
                return;
            }
            if has_marks && self.apply_pending_annotation() {
                let tab = &mut self.tabs[self.active];
                if tab.session.as_ref().is_some_and(|session| session.editable) {
                    selection::nudge(tab, delta, &mut self.status);
                    selection::set(tab, &mut self.editor, ids);
                }
            }
        }
    }
}

/// Count each key press, including repeats coalesced into the same frame.
fn arrow_delta(ctx: &egui::Context) -> [f32; 2] {
    let mut delta = [0., 0.];
    ctx.input_mut(|input| {
        input.events.retain(|event| {
            let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            else {
                return true;
            };
            let step = if modifiers.matches_exact(egui::Modifiers::NONE) {
                1.
            } else if modifiers.matches_exact(egui::Modifiers::SHIFT) {
                10.
            } else {
                return true;
            };
            let direction = match key {
                egui::Key::ArrowLeft => [-1., 0.],
                egui::Key::ArrowRight => [1., 0.],
                egui::Key::ArrowUp => [0., -1.],
                egui::Key::ArrowDown => [0., 1.],
                _ => return true,
            };
            delta[0] += direction[0] * step;
            delta[1] += direction[1] * step;
            false
        })
    });
    delta
}

#[cfg(test)]
mod tests {
    use super::*;
    use mypdf_milestone0::{
        annotations::{Annotation, Session, Style},
        convert,
    };
    use std::time::{Duration, Instant};

    fn fixture(kind: Kind) -> (Viewer, egui::Context, Annotation) {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let source = convert::blank(300., 400.).unwrap();
        app.open_memory(
            PathBuf::from("Keyboard selection.pdf"),
            source.clone(),
            &ctx,
        );
        let deadline = Instant::now() + Duration::from_secs(15);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        let mark = Annotation::new(0, [50., 70., 120., 100.], kind, Style::default());
        app.tabs[0].session = Some(Session::new(source.into(), vec![mark.clone()], true));
        app.home.visible = false;
        selection::set(&mut app.tabs[0], &mut app.editor, vec![mark.id.clone()]);
        (app, ctx, mark)
    }
    fn key(app: &mut Viewer, ctx: &egui::Context, key: egui::Key, modifiers: egui::Modifiers) {
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                modifiers,
                ..Default::default()
            },
            |ctx| app.selection_keys(ctx),
        );
    }
    fn selected(app: &Viewer) -> &Annotation {
        &app.tabs[0].session.as_ref().unwrap().annotations[0]
    }

    #[test]
    fn edit_and_annotate_keys_nudge_in_page_points_preserve_modes_and_undo() {
        for mode in [Mode::Edit, Mode::Annotate, Mode::Draw] {
            let (mut app, ctx, mark) = fixture(Kind::Rectangle);
            app.editor.mode = mode;
            key(&mut app, &ctx, egui::Key::ArrowRight, egui::Modifiers::NONE);
            assert_eq!(selected(&app).rect, [51., 70., 121., 100.]);
            key(&mut app, &ctx, egui::Key::ArrowDown, egui::Modifiers::SHIFT);
            assert_eq!(selected(&app).rect, [51., 80., 121., 110.]);
            assert!(app.editor.mode == mode);
            app.editor_undo(false);
            assert_eq!(selected(&app).rect, [51., 70., 121., 100.]);
            app.editor_undo(false);
            assert_eq!(selected(&app), &mark);
        }
    }

    #[test]
    fn nudging_stops_at_page_edges_and_locked_marks_do_not_move() {
        let (mut app, ctx, _) = fixture(Kind::Rectangle);
        app.editor.mode = Mode::Edit;
        let session = app.tabs[0].session.as_mut().unwrap();
        session.annotations[0].rect = [0., 0., 70., 30.];
        let id = selected(&app).id.clone();
        selection::set(&mut app.tabs[0], &mut app.editor, vec![id]);
        let revision = app.tabs[0].session.as_ref().unwrap().revision();
        key(&mut app, &ctx, egui::Key::ArrowUp, egui::Modifiers::SHIFT);
        assert_eq!(selected(&app).rect, [0., 0., 70., 30.]);
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), revision);
        app.tabs[0].session.as_mut().unwrap().annotations[0]
            .object
            .locked = true;
        let id = selected(&app).id.clone();
        selection::set(&mut app.tabs[0], &mut app.editor, vec![id]);
        key(&mut app, &ctx, egui::Key::ArrowRight, egui::Modifiers::NONE);
        assert_eq!(selected(&app).rect, [0., 0., 70., 30.]);
    }

    #[test]
    fn escape_applies_pending_annotation_properties_before_deselecting() {
        let (mut app, ctx, mark) = fixture(Kind::Rectangle);
        app.editor.mode = Mode::Annotate;
        app.editor.draft.as_mut().unwrap().style.color = [0.2, 0.5, 0.8];
        key(&mut app, &ctx, egui::Key::Escape, egui::Modifiers::NONE);
        assert!(app.tabs[0].selected_annotation.is_none());
        assert!(app.editor.draft.is_none());
        assert_eq!(selected(&app).style.color, [0.2, 0.5, 0.8]);
        app.editor_undo(false);
        assert_eq!(selected(&app), &mark);
        assert!(app.editor.mode == Mode::Annotate);
    }

    #[test]
    fn enter_focuses_text_properties_and_typing_arrows_do_not_move_the_object() {
        let (mut app, ctx, mark) = fixture(Kind::Text {
            text: "Editable words".into(),
        });
        app.editor.mode = Mode::Annotate;
        key(&mut app, &ctx, egui::Key::Enter, egui::Modifiers::NONE);
        assert!(app.inspector);
        let focus = egui::Id::new((app.tabs[0].id, &mark.id, "annotation_text"));
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(focus));
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.annotation_inspector(ctx)
        });
        assert!(ctx.wants_keyboard_input());
        key(&mut app, &ctx, egui::Key::ArrowLeft, egui::Modifiers::NONE);
        assert_eq!(selected(&app).rect, mark.rect);
        assert!(app.editor.mode == Mode::Annotate);
    }

    #[test]
    fn table_setup_window_keeps_selection_keys_from_objects_behind_it() {
        let (mut app, ctx, mark) = fixture(Kind::Text {
            text: "Behind the table setup".into(),
        });
        app.editor.mode = Mode::Edit;
        app.table.chooser = true;
        app.editor.draft.as_mut().unwrap().style.color = [0.2, 0.5, 0.8];
        let revision = app.tabs[0].session.as_ref().unwrap().revision();
        for event in [egui::Key::ArrowRight, egui::Key::Enter, egui::Key::Escape] {
            key(&mut app, &ctx, event, egui::Modifiers::NONE);
        }
        assert_eq!(selected(&app), &mark);
        assert_eq!(
            app.tabs[0].selected_annotation.as_deref(),
            Some(mark.id.as_str())
        );
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), revision);
        assert_eq!(
            app.editor.draft.as_ref().unwrap().style.color,
            [0.2, 0.5, 0.8]
        );
        assert!(ctx.memory(|memory| memory.focused()).is_none());
        assert!(app.table.chooser);
    }
}
