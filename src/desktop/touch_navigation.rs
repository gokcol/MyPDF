//! Direct touchscreen panning. Native touch also produces mouse events, so it
//! cannot rely on ScrollArea winning a drag against page selection widgets.
use super::*;
use egui::{Event as InputEvent, TouchDeviceId, TouchId, TouchPhase, Vec2};

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Frame {
    /// Suppress page actions through the end/cancel of a navigation gesture.
    pub navigating: bool,
    /// Disable egui's pointer drag scrolling to avoid applying the pan twice.
    pub tracking: bool,
    pub delta: Vec2,
    pub center: Option<egui::Pos2>,
    pub contacts: usize,
    /// Motion is consumed once, even when egui repeats layout in this frame.
    pub apply_motion: bool,
}

#[derive(Clone)]
struct Contact {
    device: TouchDeviceId,
    id: TouchId,
    origin: egui::Pos2,
    pos: egui::Pos2,
}

#[derive(Clone, Default)]
struct State {
    document: Option<u64>,
    contacts: Vec<Contact>,
    read: bool,
    dragging: bool,
    velocity: Vec2,
    last_motion: f64,
    last_frame: Option<u64>,
    mode: Option<editing::Mode>,
    frame: Frame,
}

impl State {
    fn update(
        &mut self,
        document: u64,
        read: bool,
        events: &[InputEvent],
        time: f64,
        dt: f32,
        accepts: impl Fn(egui::Pos2) -> bool,
    ) -> (Frame, bool) {
        if self.document != Some(document) {
            *self = Self {
                document: Some(document),
                ..Default::default()
            };
        }
        let mut frame = Frame {
            navigating: self.dragging,
            tracking: !self.contacts.is_empty(),
            ..Default::default()
        };
        let mut cancelled = false;
        let mut moved = false;
        let mut changed_contacts = false;
        let mut previous: Vec<_> = self
            .contacts
            .iter()
            .map(|c| (c.device, c.id, c.pos))
            .collect();
        let mut released = Vec::new();
        for event in events {
            match event {
                InputEvent::Touch {
                    device_id,
                    id,
                    phase: TouchPhase::Start,
                    pos,
                    ..
                } if pos.is_finite() && accepts(*pos) => {
                    if self.contacts.is_empty() {
                        self.read = read;
                        self.dragging = false;
                    }
                    self.velocity = Vec2::ZERO;
                    if !self
                        .contacts
                        .iter()
                        .any(|c| c.device == *device_id && c.id == *id)
                    {
                        changed_contacts |= !self.contacts.is_empty();
                        previous.push((*device_id, *id, *pos));
                        self.contacts.push(Contact {
                            device: *device_id,
                            id: *id,
                            origin: *pos,
                            pos: *pos,
                        });
                    }
                    frame.tracking = true;
                    if self.contacts.len() > 1 {
                        self.dragging = true;
                    }
                }
                InputEvent::Touch {
                    device_id,
                    id,
                    phase,
                    pos,
                    ..
                } => {
                    if let Some(contact) = self
                        .contacts
                        .iter_mut()
                        .find(|c| c.device == *device_id && c.id == *id)
                    {
                        frame.tracking = true;
                        if pos.is_finite() && *phase != TouchPhase::Cancel {
                            moved |= contact.pos != *pos;
                            contact.pos = *pos;
                            if self.read && contact.origin.distance(*pos) > 6. {
                                self.dragging = true;
                            }
                        }
                        if matches!(phase, TouchPhase::End | TouchPhase::Cancel) {
                            released.push((*device_id, *id));
                        }
                        cancelled |= *phase == TouchPhase::Cancel;
                    }
                }
                // Deliberate mouse/keyboard navigation stops a previous flick.
                // A touch's synthetic press is already stopped above.
                InputEvent::PointerButton { pressed: true, .. }
                | InputEvent::Key { pressed: true, .. }
                | InputEvent::MouseWheel { .. } => {
                    self.velocity = Vec2::ZERO;
                }
                InputEvent::WindowFocused(false) => cancelled = true,
                _ => {}
            }
        }
        frame.navigating |= self.dragging || cancelled && frame.tracking;
        frame.contacts = self.contacts.len();
        frame.center = (!self.contacts.is_empty()).then(|| {
            egui::Pos2::ZERO
                + self
                    .contacts
                    .iter()
                    .fold(Vec2::ZERO, |sum, c| sum + c.pos.to_vec2())
                    / self.contacts.len() as f32
        });
        if self.dragging && !cancelled && !changed_contacts {
            let mut count = 0;
            for contact in &self.contacts {
                if let Some((_, _, before)) = previous
                    .iter()
                    .find(|(d, id, _)| *d == contact.device && *id == contact.id)
                {
                    frame.delta += contact.pos - *before;
                    count += 1;
                }
            }
            if count > 0 {
                frame.delta /= count as f32;
            }
            if moved && frame.delta != Vec2::ZERO {
                self.velocity = frame.delta / dt.clamp(1. / 240., 0.05);
                let speed = self.velocity.length();
                if speed > 4000. {
                    self.velocity *= 4000. / speed;
                }
                self.last_motion = time;
            }
        }
        self.contacts
            .retain(|c| !released.contains(&(c.device, c.id)));
        if cancelled {
            self.contacts.clear();
            self.velocity = Vec2::ZERO;
            frame.delta = Vec2::ZERO;
        }
        if self.contacts.is_empty() {
            self.dragging = false;
            // Holding still before release is not a flick.
            if frame.tracking && time - self.last_motion > 0.12 {
                self.velocity = Vec2::ZERO;
            }
            if !frame.tracking && self.velocity != Vec2::ZERO {
                let speed = self.velocity.length();
                let next_speed = (speed - 2000. * dt.clamp(0., 0.05)).max(0.);
                self.velocity *= next_speed / speed;
                if next_speed < 20. {
                    self.velocity = Vec2::ZERO;
                }
                frame.delta = self.velocity * dt.clamp(0., 0.05);
                frame.tracking = true;
            }
        }
        (frame, cancelled)
    }
}

fn state_id() -> egui::Id {
    egui::Id::new("document_touch_navigation")
}

/// Called once before the document ScrollArea; continued touches remain owned
/// outside its viewport. Repeated layout passes keep ownership but consume no motion.
pub(super) fn update(
    ctx: &egui::Context,
    document: u64,
    mode: editing::Mode,
    view: Rect,
    layer: egui::LayerId,
    interrupt: bool,
) -> Frame {
    let mut state = ctx.data_mut(|d| d.get_temp::<State>(state_id()).unwrap_or_default());
    let frame_nr = ctx.cumulative_frame_nr();
    if state.last_frame == Some(frame_nr) && state.document == Some(document) {
        return Frame {
            delta: Vec2::ZERO,
            apply_motion: false,
            ..state.frame
        };
    }
    // Other views do not call this module, so their release/cancel events
    // cannot update its contacts. Never revive a gesture after such a gap.
    let reset = interrupt
        || state.mode != Some(mode)
        || state.document != Some(document)
        || state.last_frame.is_some_and(|last| frame_nr > last + 1);
    let abandoned_contact = reset && !state.contacts.is_empty();
    if reset {
        state = State::default();
    }
    let (events, time, dt) = ctx.input(|i| (i.events.clone(), i.time, i.stable_dt));
    let (mut frame, cancelled) = state.update(
        document,
        mode == editing::Mode::Read,
        &events,
        time,
        dt,
        |pos| view.contains(pos) && ctx.layer_id_at(pos) == Some(layer),
    );
    if abandoned_contact {
        frame.navigating = true;
        frame.tracking = true;
    }
    if cancelled && frame.tracking || abandoned_contact {
        // winit's cancelled touch emits PointerGone but never releases the
        // emulated mouse button. Clear it so the next mouse gesture is fresh.
        ctx.input_mut(|i| i.pointer = Default::default());
    }
    if state.contacts.is_empty() && state.velocity != Vec2::ZERO {
        ctx.request_repaint();
    }
    frame.apply_motion = true;
    state.mode = Some(mode);
    state.last_frame = Some(frame_nr);
    state.frame = frame;
    ctx.data_mut(|d| d.insert_temp(state_id(), state));
    frame
}

pub(super) fn navigating(ctx: &egui::Context, document: u64) -> bool {
    let frame_nr = ctx.cumulative_frame_nr();
    ctx.data(|d| {
        d.get_temp::<State>(state_id()).is_some_and(|state| {
            state.document == Some(document)
                && state.last_frame == Some(frame_nr)
                && state.frame.navigating
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(id: u64, phase: TouchPhase, pos: egui::Pos2) -> InputEvent {
        InputEvent::Touch {
            device_id: TouchDeviceId(1),
            id: TouchId(id),
            phase,
            pos,
            force: None,
        }
    }
    fn step(state: &mut State, time: f64, events: Vec<InputEvent>) -> Frame {
        state
            .update(1, true, &events, time, 1. / 60., |p| {
                Rect::from_min_max(pos2(0., 0.), pos2(300., 300.)).contains(p)
            })
            .0
    }
    fn pointer(pos: egui::Pos2, pressed: bool) -> InputEvent {
        InputEvent::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn touch_pan_keeps_ownership_outside_viewport_and_releases_with_a_flick() {
        let mut state = State::default();
        let begin = step(
            &mut state,
            1.,
            vec![touch(1, TouchPhase::Start, pos2(100., 100.))],
        );
        assert!(
            begin.tracking && !begin.navigating,
            "a stationary tap stays available to links"
        );
        let pan = step(
            &mut state,
            1.02,
            vec![touch(1, TouchPhase::Move, pos2(100., 40.))],
        );
        assert!(pan.navigating);
        assert_eq!(pan.delta, vec2(0., -60.));
        let outside = step(
            &mut state,
            1.04,
            vec![touch(1, TouchPhase::Move, pos2(90., -30.))],
        );
        assert_eq!(outside.delta, vec2(-10., -70.));
        let end = step(
            &mut state,
            1.06,
            vec![touch(1, TouchPhase::End, pos2(90., -30.))],
        );
        assert!(end.navigating, "release must not click a page object");
        let flick = step(&mut state, 1.08, vec![]);
        assert!(!flick.navigating && flick.delta.y < 0.);
        let stop = step(&mut state, 1.10, vec![pointer(pos2(80., 80.), true)]);
        assert_eq!(
            stop.delta,
            Vec2::ZERO,
            "mouse use stops inertia immediately"
        );
        assert!(!stop.tracking);
    }

    #[test]
    fn touch_taps_and_touches_started_outside_document_do_not_pan() {
        let mut state = State::default();
        step(
            &mut state,
            1.,
            vec![touch(1, TouchPhase::Start, pos2(50., 50.))],
        );
        let tap = step(
            &mut state,
            1.02,
            vec![touch(1, TouchPhase::End, pos2(52., 51.))],
        );
        assert!(!tap.navigating && tap.delta == Vec2::ZERO);
        step(
            &mut state,
            2.,
            vec![touch(2, TouchPhase::Start, pos2(-10., 50.))],
        );
        let crossing = step(
            &mut state,
            2.02,
            vec![touch(2, TouchPhase::Move, pos2(150., 50.))],
        );
        assert!(!crossing.tracking && crossing.delta == Vec2::ZERO);
        let mouse = step(
            &mut state,
            3.,
            vec![
                pointer(pos2(100., 100.), true),
                InputEvent::PointerMoved(pos2(100., 20.)),
            ],
        );
        assert!(!mouse.tracking && !mouse.navigating && mouse.delta == Vec2::ZERO);
    }

    #[test]
    fn touch_pan_handles_batched_input_and_cancellation_without_momentum() {
        let mut state = State::default();
        let pan = step(
            &mut state,
            1.,
            vec![
                touch(1, TouchPhase::Start, pos2(100., 100.)),
                touch(1, TouchPhase::Move, pos2(100., 50.)),
                touch(1, TouchPhase::End, pos2(100., 40.)),
            ],
        );
        assert!(pan.navigating);
        assert_eq!(pan.delta, vec2(0., -60.));
        step(
            &mut state,
            2.,
            vec![touch(1, TouchPhase::Start, pos2(100., 100.))],
        );
        step(
            &mut state,
            2.02,
            vec![touch(1, TouchPhase::Move, pos2(100., 50.))],
        );
        let cancelled = step(
            &mut state,
            2.04,
            vec![touch(1, TouchPhase::Cancel, pos2(0., 0.))],
        );
        assert!(cancelled.navigating);
        assert_eq!(cancelled.delta, Vec2::ZERO);
        let after = step(&mut state, 2.06, vec![]);
        assert!(!after.tracking && !after.navigating && after.delta == Vec2::ZERO);
        assert!(state.contacts.is_empty());
    }

    #[test]
    fn touch_two_finger_pan_uses_centroid_without_jumps_on_contact_changes() {
        let mut state = State::default();
        step(
            &mut state,
            1.,
            vec![touch(1, TouchPhase::Start, pos2(50., 100.))],
        );
        let second = step(
            &mut state,
            1.02,
            vec![touch(2, TouchPhase::Start, pos2(150., 100.))],
        );
        assert!(second.navigating);
        assert_eq!(second.delta, Vec2::ZERO);
        let pinch = step(
            &mut state,
            1.04,
            vec![
                touch(1, TouchPhase::Move, pos2(30., 80.)),
                touch(2, TouchPhase::Move, pos2(170., 80.)),
            ],
        );
        assert_eq!(pinch.delta, vec2(0., -20.));
        assert_eq!(pinch.center, Some(pos2(100., 80.)));
        let lift = step(
            &mut state,
            1.06,
            vec![touch(1, TouchPhase::End, pos2(30., 80.))],
        );
        assert!(lift.navigating && lift.delta == Vec2::ZERO);
        let remaining = step(
            &mut state,
            1.08,
            vec![touch(2, TouchPhase::Move, pos2(170., 60.))],
        );
        assert_eq!(remaining.delta, vec2(0., -20.));
    }

    #[test]
    fn touch_single_contact_drawing_is_preserved_and_document_switch_cancels_pan() {
        let mut state = State::default();
        let events = [
            touch(1, TouchPhase::Start, pos2(50., 100.)),
            touch(1, TouchPhase::Move, pos2(50., 20.)),
        ];
        let (drawing, _) = state.update(1, false, &events, 1., 1. / 60., |_| true);
        assert!(!drawing.navigating && drawing.delta == Vec2::ZERO);
        let (two, _) = state.update(
            1,
            false,
            &[touch(2, TouchPhase::Start, pos2(150., 100.))],
            1.02,
            1. / 60.,
            |_| true,
        );
        assert!(two.navigating);
        let (other, _) = state.update(
            2,
            true,
            &[touch(1, TouchPhase::Move, pos2(50., 0.))],
            1.04,
            1. / 60.,
            |_| true,
        );
        assert!(!other.navigating && other.delta == Vec2::ZERO);
    }

    fn native_touch(id: u64, phase: TouchPhase, pos: egui::Pos2) -> Vec<InputEvent> {
        let mut events = vec![touch(id, phase, pos)];
        match phase {
            TouchPhase::Start => events.extend([InputEvent::PointerMoved(pos), pointer(pos, true)]),
            TouchPhase::Move => events.push(InputEvent::PointerMoved(pos)),
            TouchPhase::End => events.extend([pointer(pos, false), InputEvent::PointerGone]),
            TouchPhase::Cancel => events.push(InputEvent::PointerGone),
        }
        events
    }

    fn document_frame(
        app: &mut Viewer,
        ctx: &egui::Context,
        time: &mut f64,
        events: Vec<InputEvent>,
    ) -> egui::FullOutput {
        *time += 1. / 60.;
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(800., 500.))),
                time: Some(*time),
                events,
                ..Default::default()
            },
            |ctx| app.document(ctx),
        )
    }

    fn document_fixture() -> (Viewer, egui::Context, f64) {
        use lopdf::{Object, dictionary};
        let bytes = mypdf_milestone0::convert::blank(900., 1800.).unwrap();
        let mut pdf = lopdf::Document::load_mem(&bytes).unwrap();
        let page = *pdf.get_pages().values().next().unwrap();
        let link = pdf.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Link",
            "Rect" => vec![200.into(), 1550.into(), 300.into(), 1590.into()],
            "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("https://example.com/touch-test") },
        });
        pdf.get_dictionary_mut(page)
            .unwrap()
            .set("Annots", vec![Object::Reference(link)]);
        let mut bytes = Vec::new();
        pdf.save_to(&mut bytes).unwrap();
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.open_memory(PathBuf::from("Touch navigation test.pdf"), bytes, &ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.tabs[0].session.is_none() || app.tabs[0].navigation(&ctx).is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        app.editor.mode = editing::Mode::Read;
        app.tabs[0].zoom = Zoom::Scale(1.);
        app.tabs[0].text[0] = Some(Arc::new(PageText {
            text: "abcd".into(),
            glyphs: (0..4)
                .map(|i| viewer::GlyphBox {
                    range: i..i + 1,
                    rect: [50. + i as f32 * 20., 100., 65. + i as f32 * 20., 115.],
                })
                .collect(),
            ..Default::default()
        }));
        let mut time = 1.;
        for _ in 0..3 {
            document_frame(&mut app, &ctx, &mut time, vec![]);
        }
        (app, ctx, time)
    }

    fn page_origin(app: &Viewer, ctx: &egui::Context) -> egui::Pos2 {
        ctx.read_response(egui::Id::new((app.tabs[0].id, 0_usize, "page_actions")))
            .unwrap()
            .rect
            .min
    }

    #[test]
    fn touch_document_scrolls_over_selected_text_and_cancel_restores_mouse_selection() {
        let (mut app, ctx, mut time) = document_fixture();
        let start = page_origin(&app, &ctx) + vec2(55., 108.);
        let end = start + vec2(60., 0.);
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            vec![InputEvent::PointerMoved(start), pointer(start, true)],
        );
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            vec![InputEvent::PointerMoved(end)],
        );
        document_frame(&mut app, &ctx, &mut time, vec![pointer(end, false)]);
        assert_eq!(app.page_text_selection(&ctx).as_deref(), Some("abcd"));
        let before = app.tabs[0].document_view.unwrap().1;
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            native_touch(1, TouchPhase::Start, end),
        );
        let moved = end - vec2(20., 60.);
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            native_touch(1, TouchPhase::Move, moved),
        );
        let after = app.tabs[0].document_view.unwrap().1;
        assert!(
            (after.x - before.x - 20.).abs() < 0.1 && (after.y - before.y - 60.).abs() < 0.1,
            "touch should pan both axes exactly once: {before:?} -> {after:?}"
        );
        assert_eq!(app.page_text_selection(&ctx).as_deref(), Some("abcd"));
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            native_touch(1, TouchPhase::Cancel, moved),
        );
        assert!(
            !ctx.input(|i| i.pointer.primary_down()),
            "cancel must clear winit's unreleased emulated button"
        );
        assert!(!app.editor.interaction_active());
        document_frame(&mut app, &ctx, &mut time, vec![]);
        assert_eq!(app.tabs[0].document_view.unwrap().1, after);
        let start = page_origin(&app, &ctx) + vec2(55., 108.);
        let end = start + vec2(20., 0.);
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            vec![InputEvent::PointerMoved(start), pointer(start, true)],
        );
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            vec![InputEvent::PointerMoved(end)],
        );
        document_frame(&mut app, &ctx, &mut time, vec![pointer(end, false)]);
        assert_eq!(app.page_text_selection(&ctx).as_deref(), Some("ab"));
        assert_eq!(
            app.tabs[0].document_view.unwrap().1,
            after,
            "mouse selection must not scroll the page"
        );
    }

    #[test]
    fn touch_document_tap_follows_link_but_drag_over_link_does_not() {
        let (mut app, ctx, mut time) = document_fixture();
        let point = page_origin(&app, &ctx) + vec2(230., 230.);
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            native_touch(1, TouchPhase::Start, point),
        );
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            native_touch(1, TouchPhase::End, point),
        );
        assert_eq!(
            app.link_confirm.take().as_deref(),
            Some("https://example.com/touch-test")
        );
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            native_touch(2, TouchPhase::Start, point),
        );
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            native_touch(2, TouchPhase::Move, point - vec2(0., 80.)),
        );
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            native_touch(2, TouchPhase::End, point - vec2(0., 80.)),
        );
        assert!(app.link_confirm.is_none());
        assert!(app.tabs[0].document_view.unwrap().1.y >= 80.);
        assert!(app.editor.text_selection.is_none());
    }

    #[test]
    fn touch_document_pinch_keeps_content_under_moving_centroid() {
        let (mut app, ctx, mut time) = document_fixture();
        let first = pos2(250., 250.);
        let second = pos2(450., 250.);
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            native_touch(1, TouchPhase::Start, first),
        );
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            vec![touch(2, TouchPhase::Start, second)],
        );
        let old_layout = app.tabs[0].document_layout.clone().unwrap();
        let (view, offset, _) = app.tabs[0].document_view.unwrap();
        let old_center = first + (second - first) / 2.;
        let motion = vec2(0., -20.);
        let mut events = native_touch(1, TouchPhase::Move, first + vec2(-20., -20.));
        events.push(touch(2, TouchPhase::Move, second + vec2(20., -20.)));
        document_frame(&mut app, &ctx, &mut time, events);
        let tab = &app.tabs[0];
        assert!(
            (tab.scale - 1.2).abs() < 0.01,
            "pinch should change PDF zoom, got {}",
            tab.scale
        );
        let expected = old_layout.anchored_offset(
            tab.document_layout.as_ref().unwrap(),
            offset,
            old_center - view.min,
        ) - motion;
        assert!((tab.document_view.unwrap().1 - expected).length() < 0.1);
        assert!(app.editor.text_selection.is_none() && !app.editor.interaction_active());
    }

    #[test]
    fn touch_document_ignores_pinch_with_a_finger_outside_its_viewport() {
        let (mut app, ctx, mut time) = document_fixture();
        let first = pos2(250., 250.);
        let outside = pos2(2., 250.);
        assert!(!app.tabs[0].document_view.unwrap().0.contains(outside));
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            native_touch(1, TouchPhase::Start, first),
        );
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            vec![touch(2, TouchPhase::Start, outside)],
        );
        let mut events = native_touch(1, TouchPhase::Move, first + vec2(40., 0.));
        events.push(touch(2, TouchPhase::Move, outside - vec2(40., 0.)));
        document_frame(&mut app, &ctx, &mut time, events);
        assert!(
            (app.tabs[0].scale - 1.).abs() < 0.001,
            "outside contact must not zoom the document"
        );
    }

    #[test]
    fn touch_document_view_gap_and_navigation_stop_stale_contacts_and_inertia() {
        let (mut app, ctx, mut time) = document_fixture();
        let point = pos2(250., 250.);
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            native_touch(1, TouchPhase::Start, point),
        );
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            native_touch(1, TouchPhase::Move, point - vec2(0., 60.)),
        );
        let offset = app.tabs[0].document_view.unwrap().1;
        // Release while Home/Pages/comparison owns the frame: the document's
        // gesture tracker deliberately never sees this touch event.
        time += 1. / 60.;
        let _ = ctx.run(
            egui::RawInput {
                time: Some(time),
                events: native_touch(1, TouchPhase::End, point - vec2(0., 60.)),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label("Other view");
                });
            },
        );
        document_frame(&mut app, &ctx, &mut time, vec![]);
        document_frame(&mut app, &ctx, &mut time, vec![]);
        assert_eq!(app.tabs[0].document_view.unwrap().1, offset);
        assert!(!ctx.data(|d| d.get_temp::<State>(state_id()).unwrap().frame.tracking));
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            native_touch(2, TouchPhase::Start, point),
        );
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            native_touch(2, TouchPhase::Move, point - vec2(0., 60.)),
        );
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            native_touch(2, TouchPhase::End, point - vec2(0., 60.)),
        );
        app.tabs[0].jump = Some((0, Some(500.)));
        document_frame(&mut app, &ctx, &mut time, vec![]);
        let jumped = app.tabs[0].document_view.unwrap().1;
        assert!(
            (jumped.y - 500.).abs() < 0.1,
            "explicit page navigation must override a flick: {jumped:?}"
        );
        document_frame(&mut app, &ctx, &mut time, vec![]);
        assert_eq!(app.tabs[0].document_view.unwrap().1, jumped);
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            native_touch(3, TouchPhase::Start, point),
        );
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            native_touch(3, TouchPhase::Move, point - vec2(0., 60.)),
        );
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            native_touch(3, TouchPhase::End, point - vec2(0., 60.)),
        );
        let before_mode = app.tabs[0].document_view.unwrap().1;
        app.editor.mode = editing::Mode::Draw;
        document_frame(&mut app, &ctx, &mut time, vec![]);
        assert_eq!(app.tabs[0].document_view.unwrap().1, before_mode);
    }

    #[test]
    fn touch_flick_stops_on_keyboard_navigation_and_focus_loss() {
        let mut state = State::default();
        for id in 1..=2 {
            let time = id as f64;
            step(
                &mut state,
                time,
                vec![touch(id, TouchPhase::Start, pos2(100., 100.))],
            );
            step(
                &mut state,
                time + 0.02,
                vec![touch(id, TouchPhase::Move, pos2(100., 50.))],
            );
            step(
                &mut state,
                time + 0.04,
                vec![touch(id, TouchPhase::End, pos2(100., 50.))],
            );
            let event = if id == 1 {
                InputEvent::Key {
                    key: egui::Key::PageDown,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }
            } else {
                InputEvent::WindowFocused(false)
            };
            let after = step(&mut state, time + 0.06, vec![event]);
            assert_eq!(after.delta, Vec2::ZERO);
            assert_eq!(state.velocity, Vec2::ZERO);
        }
    }

    #[test]
    fn touch_navigation_paints_inline_draft_but_not_annotations_already_in_the_raster() {
        use mypdf_milestone0::annotations::{Annotation, Kind};
        let (mut app, ctx, mut time) = document_fixture();
        let mark = Annotation::new(
            0,
            [100., 100., 400., 150.],
            Kind::Text {
                text: "Saved annotation".into(),
            },
            Default::default(),
        );
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![mark.clone()])
            .unwrap();
        app.editor.mode = editing::Mode::Annotate;
        app.editor.tool = editing::Tool::Select;
        app.tabs[0].selected_annotation = Some(mark.id.clone());
        annotation_text::start(&mut app.editor, app.tabs[0].id, &mark, false);
        app.editor.draft.as_mut().unwrap().kind = Kind::Text {
            text: "Visible current draft".into(),
        };
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            native_touch(1, TouchPhase::Start, pos2(500., 350.)),
        );
        let output = document_frame(
            &mut app,
            &ctx,
            &mut time,
            vec![touch(2, TouchPhase::Start, pos2(650., 350.))],
        );
        let texts = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .filter_map(|s| {
                    if let egui::Shape::Text(t) = &s.shape {
                        Some(t.galley.job.text.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        assert!(
            texts(&output).iter().any(|t| t == "Visible current draft"),
            "suspended inline editor must still paint its draft"
        );
        app.tabs[0].session.as_mut().unwrap().editable = false;
        let protected = document_frame(&mut app, &ctx, &mut time, vec![]);
        assert!(
            !texts(&protected)
                .iter()
                .any(|t| t == "Visible current draft" || t == "Saved annotation"),
            "nonrewritable PDF annotations are already rasterized and must not be painted twice"
        );
    }

    #[test]
    fn touch_document_pan_pinch_and_flick_apply_once_across_layout_passes() {
        for pinch in [false, true] {
            let (mut app, ctx, mut time) = document_fixture();
            ctx.options_mut(|o| o.max_passes = std::num::NonZeroUsize::new(2).unwrap());
            let first = pos2(250., 250.);
            let second = pos2(450., 250.);
            document_frame(
                &mut app,
                &ctx,
                &mut time,
                native_touch(1, TouchPhase::Start, first),
            );
            if pinch {
                document_frame(
                    &mut app,
                    &ctx,
                    &mut time,
                    vec![touch(2, TouchPhase::Start, second)],
                );
            }
            let mut events = native_touch(1, TouchPhase::Move, first - vec2(20., 20.));
            if pinch {
                events.push(touch(2, TouchPhase::Move, second + vec2(20., -20.)));
            }
            time += 1. / 60.;
            let mut snapshots = Vec::new();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(800., 500.))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    app.document(ctx);
                    snapshots.push((app.tabs[0].document_view.unwrap().1, app.tabs[0].scale));
                    if ctx.current_pass_index() == 0 {
                        ctx.request_discard("touch multipass regression");
                    }
                },
            );
            assert_eq!(output.platform_output.num_completed_passes, 2);
            assert_eq!(snapshots.len(), 2);
            assert_eq!(
                snapshots[0], snapshots[1],
                "pan/pinch must be applied once: {snapshots:?}"
            );
            assert!((snapshots[1].1 - if pinch { 1.2 } else { 1. }).abs() < 0.001);
            let mut release = native_touch(1, TouchPhase::End, first - vec2(20., 20.));
            if pinch {
                release.push(touch(2, TouchPhase::End, second + vec2(20., -20.)));
            }
            document_frame(&mut app, &ctx, &mut time, release);
            time += 1. / 60.;
            snapshots.clear();
            let _ = ctx.run(
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |ctx| {
                    app.document(ctx);
                    snapshots.push((app.tabs[0].document_view.unwrap().1, app.tabs[0].scale));
                    if ctx.current_pass_index() == 0 {
                        ctx.request_discard("touch flick multipass regression");
                    }
                },
            );
            assert_eq!(snapshots.len(), 2);
            assert_eq!(snapshots[0], snapshots[1], "flick must be applied once");
        }
    }

    fn eraser_fixture() -> (
        Viewer,
        egui::Context,
        f64,
        mypdf_milestone0::annotations::Annotation,
    ) {
        use mypdf_milestone0::annotations::{Annotation, Kind};
        let (mut app, ctx, mut time) = document_fixture();
        let ink = Annotation::new(
            0,
            [100., 100., 300., 200.],
            Kind::Ink {
                strokes: vec![vec![[0., 0.5], [1., 0.5]]],
            },
            Default::default(),
        );
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![ink.clone()])
            .unwrap();
        app.editor.mode = editing::Mode::Draw;
        app.editor.tool = editing::Tool::Eraser;
        document_frame(&mut app, &ctx, &mut time, vec![]);
        (app, ctx, time, ink)
    }

    #[test]
    fn touch_staggered_two_finger_eraser_navigation_and_cancel_leave_ink_unchanged() {
        let (mut app, ctx, mut time, ink) = eraser_fixture();
        let point = page_origin(&app, &ctx) + vec2(200., 150.);
        let revision = app.tabs[0].session.as_ref().unwrap().revision();
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            native_touch(1, TouchPhase::Start, point),
        );
        assert_eq!(
            app.tabs[0].session.as_ref().unwrap().revision(),
            revision,
            "stationary first finger must defer erasing"
        );
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            vec![touch(2, TouchPhase::Start, point + vec2(100., 0.))],
        );
        let mut pan = native_touch(1, TouchPhase::Move, point - vec2(0., 50.));
        pan.push(touch(2, TouchPhase::Move, point + vec2(100., -50.)));
        document_frame(&mut app, &ctx, &mut time, pan);
        let mut end = native_touch(1, TouchPhase::Cancel, point - vec2(0., 50.));
        end.push(touch(2, TouchPhase::Cancel, point + vec2(100., -50.)));
        document_frame(&mut app, &ctx, &mut time, end);
        assert!(app.tabs[0].document_view.unwrap().1.y >= 50.);
        assert_eq!(
            app.tabs[0].session.as_ref().unwrap().annotations,
            vec![ink.clone()]
        );
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), revision);
        assert!(!app.editor.eraser.active());
        document_frame(&mut app, &ctx, &mut time, vec![]);
        let point = page_origin(&app, &ctx) + vec2(200., 150.);
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            native_touch(3, TouchPhase::Start, point),
        );
        document_frame(
            &mut app,
            &ctx,
            &mut time,
            native_touch(3, TouchPhase::Cancel, point),
        );
        assert_eq!(app.tabs[0].session.as_ref().unwrap().annotations, vec![ink]);
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), revision);
    }

    #[test]
    fn touch_eraser_tap_and_deliberate_motion_work_and_mouse_press_stays_immediate() {
        for input in ["touch tap", "touch move", "mouse press"] {
            let (mut app, ctx, mut time, ink) = eraser_fixture();
            let point = page_origin(&app, &ctx) + vec2(200., 150.);
            if input == "mouse press" {
                document_frame(
                    &mut app,
                    &ctx,
                    &mut time,
                    vec![InputEvent::PointerMoved(point), pointer(point, true)],
                );
            } else {
                document_frame(
                    &mut app,
                    &ctx,
                    &mut time,
                    native_touch(1, TouchPhase::Start, point),
                );
                assert_eq!(
                    app.tabs[0].session.as_ref().unwrap().annotations,
                    vec![ink.clone()]
                );
                let phase = if input == "touch tap" {
                    TouchPhase::End
                } else {
                    TouchPhase::Move
                };
                let end = if input == "touch tap" {
                    point
                } else {
                    point + vec2(20., 0.)
                };
                document_frame(&mut app, &ctx, &mut time, native_touch(1, phase, end));
            }
            let edited = app.tabs[0].session.as_ref().unwrap().annotations.clone();
            assert_ne!(edited, vec![ink.clone()], "{input} must erase ink");
            if input == "touch move" {
                // A deliberate stroke was already committed; adding another
                // finger stops that stroke without undoing prior real edits.
                document_frame(
                    &mut app,
                    &ctx,
                    &mut time,
                    vec![touch(2, TouchPhase::Start, point + vec2(100., 0.))],
                );
                assert_eq!(app.tabs[0].session.as_ref().unwrap().annotations, edited);
            }
            app.tabs[0].session.as_mut().unwrap().undo();
            assert_eq!(app.tabs[0].session.as_ref().unwrap().annotations, vec![ink]);
        }
    }
}
