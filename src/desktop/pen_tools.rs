//! A real stroke eraser. The brush clips ink geometry rather than covering a
//! page with white paint; document text, images, and shapes are never touched.
use super::*;
use mypdf_milestone0::annotations::{Annotation, Kind, Point2, Session};

pub(super) struct State {
    /// Brush radius in displayed-page points, independent of the zoom level.
    pub(super) radius: f32,
    gesture: Option<Gesture>,
}

struct Gesture {
    tab: u64,
    page: usize,
    last: Point2,
    revision: Option<u64>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            radius: 8.,
            gesture: None,
        }
    }
}

impl State {
    pub(super) fn active(&self) -> bool {
        self.gesture.is_some()
    }

    /// Live changes have already been committed. Releasing a brush, changing
    /// tool, or leaving the document closes its single Undo transaction.
    pub(super) fn finish(&mut self) {
        self.gesture = None;
    }

    fn advance(&mut self, tab: &mut Tab, p: Point2, status: &mut Option<(String, bool)>) {
        let Some(g) = self.gesture.as_mut().filter(|g| g.tab == tab.id) else {
            return;
        };
        let Some(session) = tab.session.as_mut() else {
            self.finish();
            return;
        };
        match apply(session, g.page, g.last, p, self.radius, g.revision) {
            Ok(revision) => {
                g.last = p;
                g.revision = revision.or(g.revision);
            }
            Err(error) => {
                *status = Some((format!("Could not erase ink: {error:#}"), true));
                self.finish();
            }
        }
    }
}

pub(super) fn controls(ui: &mut egui::Ui, state: &mut State) {
    ui.label(RichText::new("Eraser").strong());
    ui.horizontal(|ui| {
        ui.label("Size");
        let mut diameter = state.radius * 2.;
        ui.add(
            egui::Slider::new(&mut diameter, 4.0..=64.0)
                .suffix(" pt")
                .clamping(egui::SliderClamping::Always),
        );
        state.radius = diameter / 2.;
    });
    ui.label(
        RichText::new("Drag to erase pen strokes. Undo restores each stroke gesture.")
            .small()
            .color(theme::muted()),
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn overlay(
    state: &mut State,
    tab: &mut Tab,
    ctx: &egui::Context,
    ui: &egui::Ui,
    response: &egui::Response,
    page: usize,
    page_rect: Rect,
    scale: f32,
    status: &mut Option<(String, bool)>,
) -> bool {
    if !scale.is_finite() || scale <= 0. {
        state.finish();
        return false;
    }
    let editable = tab.session.as_ref().is_some_and(|s| s.editable);
    if !editable || ctx.input(|i| i.multi_touch().is_some_and(|t| t.num_touches > 1)) {
        state.finish();
        return false;
    }
    let normal = |p: egui::Pos2| {
        let p = (p - page_rect.min) / scale;
        [p.x, p.y]
    };
    let visible = page_rect.intersect(ui.clip_rect());
    let events = ctx.input(|i| i.events.clone());
    for event in events {
        match event {
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                ..
            } if visible.contains(pos)
                && (response.contains_pointer() || response.is_pointer_button_down_on()) =>
            {
                state.gesture = Some(Gesture {
                    tab: tab.id,
                    page,
                    last: normal(pos),
                    revision: None,
                });
                tab.selected_annotation = None;
                tab.group.clear();
                state.advance(tab, normal(pos), status);
            }
            egui::Event::PointerMoved(pos)
                if state
                    .gesture
                    .as_ref()
                    .is_some_and(|g| g.tab == tab.id && g.page == page) =>
            {
                state.advance(tab, normal(pos), status);
            }
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                ..
            } if state
                .gesture
                .as_ref()
                .is_some_and(|g| g.tab == tab.id && g.page == page) =>
            {
                state.advance(tab, normal(pos), status);
                state.finish();
            }
            egui::Event::PointerGone => state.finish(),
            _ => {}
        }
    }
    if state.gesture.as_ref().is_some_and(|g| g.tab != tab.id) {
        state.finish();
    }
    if response.hovered() || state.active() {
        ctx.set_cursor_icon(egui::CursorIcon::None);
        if let Some(pos) = ctx.pointer_latest_pos()
            && visible.contains(pos)
        {
            let painter = ui.painter().with_clip_rect(visible);
            let radius = state.radius * scale;
            painter.circle_stroke(pos, radius, Stroke::new(3., theme::paper()));
            painter.circle_stroke(pos, radius, Stroke::new(1., theme::accent()));
        }
    }
    state.active()
}

/// Apply one swept brush segment. Consecutive calls from one drag pass the
/// returned revision to coalesce history, while every frame still has a fresh
/// revision for autosave and an in-flight Save.
fn apply(
    session: &mut Session,
    page: usize,
    start: Point2,
    end: Point2,
    radius: f32,
    previous: Option<u64>,
) -> anyhow::Result<Option<u64>> {
    let mut changed = false;
    let annotations = session
        .annotations
        .iter()
        .filter_map(|a| {
            if a.page != page || a.object.hidden || a.object.locked {
                return Some(a.clone());
            }
            let next = erase(a, start, end, radius);
            changed |= next.as_ref() != Some(a);
            next
        })
        .collect();
    if !changed {
        return Ok(None);
    }
    session.commit_live(annotations, previous)?;
    session.name_last_change("Erase pen strokes");
    session.clear_change_focus();
    Ok(Some(session.revision()))
}

/// Clip only Ink polylines against a capsule in displayed-page coordinates.
/// Original normalized coordinates are interpolated at intersections, so
/// rotation, reflection, dimensions, pen color, and pen width remain intact.
pub(super) fn erase(
    annotation: &Annotation,
    start: Point2,
    end: Point2,
    radius: f32,
) -> Option<Annotation> {
    let Kind::Ink { strokes } = &annotation.kind else {
        return Some(annotation.clone());
    };
    if annotation.object.locked
        || annotation.object.hidden
        || !radius.is_finite()
        || radius <= 0.
        || !start.into_iter().chain(end).all(f32::is_finite)
    {
        return Some(annotation.clone());
    }
    // The rendered stroke has round caps/joins. Its outside edge must clear
    // the brush as well, rather than leaving half-width ink under the cursor.
    let radius = radius + annotation.style.width / 2.;
    let world = |p| annotation.rotated(annotation.local(p));
    let mut result = vec![];
    let mut changed = false;
    for stroke in strokes {
        if stroke.len() == 1 {
            if intervals(world(stroke[0]), world(stroke[0]), start, end, radius).is_empty() {
                result.push(stroke.clone());
            } else {
                changed = true;
            }
            continue;
        }
        let mut current = vec![];
        for pair in stroke.windows(2) {
            let cut = intervals(world(pair[0]), world(pair[1]), start, end, radius);
            if cut.is_empty() {
                append(&mut current, pair[0]);
                append(&mut current, pair[1]);
                continue;
            }
            changed = true;
            let mut t = 0.;
            for [lo, hi] in cut {
                if lo > t + 1e-6 {
                    append(&mut current, lerp(pair[0], pair[1], t));
                    append(&mut current, lerp(pair[0], pair[1], lo));
                }
                if !current.is_empty() {
                    result.push(std::mem::take(&mut current));
                }
                t = hi;
            }
            if t < 1. - 1e-6 {
                append(&mut current, lerp(pair[0], pair[1], t));
                append(&mut current, pair[1]);
            }
        }
        if !current.is_empty() {
            result.push(current);
        }
    }
    if !changed {
        return Some(annotation.clone());
    }
    if result.is_empty() {
        return None;
    }
    let mut next = annotation.clone();
    next.kind = Kind::Ink { strokes: result };
    Some(next)
}

fn append(points: &mut Vec<Point2>, p: Point2) {
    if points
        .last()
        .is_none_or(|last| (last[0] - p[0]).abs() > 1e-6 || (last[1] - p[1]).abs() > 1e-6)
    {
        points.push(p);
    }
}

fn lerp(a: Point2, b: Point2, t: f32) -> Point2 {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// Parameter intervals where p(t) lies in the eraser capsule. The capsule is
/// the union of two endpoint disks and the rectangle between them. This also
/// catches fast pointer motion across sparse ink, with no sampling gaps.
fn intervals(p: Point2, q: Point2, a: Point2, b: Point2, radius: f32) -> Vec<[f32; 2]> {
    let mut result = vec![];
    let p = [p[0] as f64, p[1] as f64];
    let q = [q[0] as f64, q[1] as f64];
    let a = [a[0] as f64, a[1] as f64];
    let b = [b[0] as f64, b[1] as f64];
    let radius = radius as f64;
    for c in [a, b] {
        if let Some(interval) = circle_interval(p, q, c, radius) {
            result.push(interval);
        }
    }
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let length = dx.hypot(dy);
    if length > 1e-9 {
        let project = |v: [f64; 2]| {
            [
                ((v[0] - a[0]) * dx + (v[1] - a[1]) * dy) / length,
                (-(v[0] - a[0]) * dy + (v[1] - a[1]) * dx) / length,
            ]
        };
        if let Some(interval) = rectangle_interval(project(p), project(q), length, radius) {
            result.push(interval);
        }
    }
    result.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let mut merged: Vec<[f32; 2]> = vec![];
    for interval in result {
        if let Some(last) = merged.last_mut()
            && interval[0] <= last[1] + 1e-6
        {
            last[1] = last[1].max(interval[1]);
        } else {
            merged.push(interval);
        }
    }
    merged
}

fn circle_interval(p: [f64; 2], q: [f64; 2], c: [f64; 2], radius: f64) -> Option<[f32; 2]> {
    let d = [q[0] - p[0], q[1] - p[1]];
    let offset = [p[0] - c[0], p[1] - c[1]];
    let aa = d[0] * d[0] + d[1] * d[1];
    let cc = offset[0] * offset[0] + offset[1] * offset[1] - radius * radius;
    if aa < 1e-18 {
        return (cc <= 0.).then_some([0., 1.]);
    }
    let bb = 2. * (offset[0] * d[0] + offset[1] * d[1]);
    let discriminant = bb * bb - 4. * aa * cc;
    if discriminant < 0. {
        return None;
    }
    let root = discriminant.sqrt();
    let lo = ((-bb - root) / (2. * aa)).max(0.);
    let hi = ((-bb + root) / (2. * aa)).min(1.);
    (hi >= lo).then_some([lo as f32, hi as f32])
}

fn rectangle_interval(p: [f64; 2], q: [f64; 2], length: f64, radius: f64) -> Option<[f32; 2]> {
    let mut lo: f64 = 0.;
    let mut hi: f64 = 1.;
    for (axis, lower, upper) in [(0, 0., length), (1, -radius, radius)] {
        let delta = q[axis] - p[axis];
        if delta.abs() < 1e-12 {
            if p[axis] < lower || p[axis] > upper {
                return None;
            }
        } else {
            let first = (lower - p[axis]) / delta;
            let last = (upper - p[axis]) / delta;
            lo = lo.max(first.min(last));
            hi = hi.min(first.max(last));
            if hi < lo {
                return None;
            }
        }
    }
    Some([lo as f32, hi as f32])
}

#[cfg(test)]
mod tests {
    use super::*;
    use mypdf_milestone0::annotations::Style;

    fn ink() -> Annotation {
        Annotation::new(
            0,
            [10., 20., 110., 120.],
            Kind::Ink {
                strokes: vec![vec![[0., 0.5], [1., 0.5]]],
            },
            Style {
                color: [0.1, 0.2, 0.8],
                width: 2.,
                ..Default::default()
            },
        )
    }

    fn strokes(a: &Annotation) -> &Vec<Vec<Point2>> {
        let Kind::Ink { strokes } = &a.kind else {
            panic!("expected ink")
        };
        strokes
    }

    #[test]
    fn eraser_splits_a_sparse_stroke_without_changing_appearance() {
        let original = ink();
        let next = erase(&original, [60., 40.], [60., 100.], 5.).unwrap();
        assert_eq!(strokes(&next).len(), 2);
        assert_eq!(strokes(&next)[0][0], [0., 0.5]);
        assert_eq!(strokes(&next)[1][1], [1., 0.5]);
        assert!((strokes(&next)[0][1][0] - 0.44).abs() < 0.001);
        assert!((strokes(&next)[1][0][0] - 0.56).abs() < 0.001);
        assert_eq!(next.style, original.style);
        assert_eq!(next.id, original.id);
        assert_eq!(next.rect, original.rect);
        next.validate().unwrap();
    }

    #[test]
    fn eraser_preserves_other_objects_and_untouched_ink_exactly() {
        let original = ink();
        assert_eq!(
            erase(&original, [0., 0.], [0., 200.], 2.),
            Some(original.clone())
        );
        for kind in [
            Kind::Rectangle,
            Kind::Text {
                text: "Keep me".into(),
            },
            Kind::Svg {
                source: "<svg/>".into(),
                tint: false,
            },
            Kind::Image {
                png: Arc::new(vec![]),
            },
        ] {
            let mut other = original.clone();
            other.kind = kind;
            assert_eq!(erase(&other, [0., 70.], [200., 70.], 20.), Some(other));
        }
    }

    #[test]
    fn eraser_can_remove_whole_ink_and_single_point_strokes() {
        assert!(erase(&ink(), [0., 70.], [120., 70.], 4.).is_none());
        let mut point = ink();
        point.kind = Kind::Ink {
            strokes: vec![vec![[0.5, 0.5]]],
        };
        assert!(erase(&point, [60., 70.], [60., 70.], 4.).is_none());
        assert_eq!(erase(&point, [20., 70.], [20., 70.], 4.), Some(point));
    }

    #[test]
    fn rotated_and_reflected_ink_is_erased_in_displayed_page_coordinates() {
        let mut original = ink();
        original.rotation = 90.;
        original.flip_x = true;
        let next = erase(&original, [30., 70.], [90., 70.], 5.).unwrap();
        assert_eq!(strokes(&next).len(), 2);
        assert_eq!(next.rotation, 90.);
        assert!(next.flip_x);
        assert!((strokes(&next)[0][1][0] - 0.44).abs() < 0.001);
        assert!((strokes(&next)[1][0][0] - 0.56).abs() < 0.001);
    }

    #[test]
    fn eraser_brush_segments_have_round_caps_and_no_fast_drag_gaps() {
        let original = ink();
        let next = erase(&original, [30., 70.], [90., 70.], 5.).unwrap();
        assert_eq!(strokes(&next).len(), 2);
        assert!((strokes(&next)[0][1][0] - 0.14).abs() < 0.001);
        assert!((strokes(&next)[1][0][0] - 0.86).abs() < 0.001);
        let tangent = erase(&original, [60., 64.], [60., 64.], 5.).unwrap();
        // Tangential contact is a split, with neither distant endpoint lost.
        assert_eq!(strokes(&tangent)[0][0], [0., 0.5]);
        assert_eq!(strokes(&tangent).last().unwrap().last(), Some(&[1., 0.5]));
    }

    #[test]
    fn a_continuous_eraser_gesture_is_one_undo_and_does_not_touch_locked_or_other_pages() {
        let original = ink();
        let mut locked = ink();
        locked.object.locked = true;
        let mut hidden = ink();
        hidden.object.hidden = true;
        let mut elsewhere = ink();
        elsewhere.page = 1;
        let before = vec![original, locked, hidden, elsewhere];
        let mut session = Session::new(Arc::from([]), before.clone(), true);
        let revision = apply(&mut session, 0, [40., 50.], [40., 90.], 5., None).unwrap();
        assert_eq!(session.annotations.len(), 4);
        assert_eq!(&session.annotations[1..], &before[1..]);
        let after_first = session.annotations.clone();
        apply(&mut session, 0, [80., 50.], [80., 90.], 5., revision).unwrap();
        assert_ne!(session.annotations, after_first);
        assert_eq!(session.undo_label().as_deref(), Some("Erase pen strokes"));
        let after = session.annotations.clone();
        session.undo();
        assert_eq!(session.annotations, before);
        assert!(!session.can_undo());
        session.redo();
        assert_eq!(session.annotations, after);
        assert_eq!(
            apply(&mut session, 0, [0., 0.], [0., 200.], 1., None).unwrap(),
            None
        );
        assert_eq!(session.annotations, after);
    }

    #[test]
    fn eraser_canvas_handles_batched_mouse_events_and_keeps_page_size_at_each_zoom() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.editor.mode = editing::Mode::Draw;
        app.editor.tool = editing::Tool::Eraser;
        app.editor.eraser.radius = 5.;
        for scale in [0.6, 1., 2.] {
            let original = ink();
            app.tabs[0]
                .session
                .as_mut()
                .unwrap()
                .commit(vec![original.clone()])
                .unwrap();
            let origin = pos2(73., 61.);
            let screen = |p: Point2| origin + vec2(p[0], p[1]) * scale;
            let frame = |app: &mut Viewer, events: Vec<egui::Event>| {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1400., 2000.))),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            editing::page_interaction(
                                &mut app.editor,
                                &mut app.tabs[0],
                                ui,
                                0,
                                Rect::from_min_size(origin, vec2(595., 842.) * scale),
                                scale,
                                &mut app.status,
                            );
                        });
                    },
                );
            };
            let start = screen([60., 40.]);
            let end = screen([60., 100.]);
            frame(&mut app, vec![egui::Event::PointerMoved(start)]);
            frame(
                &mut app,
                vec![
                    egui::Event::PointerButton {
                        pos: start,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::PointerMoved(end),
                    egui::Event::PointerButton {
                        pos: end,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            assert!(
                !app.editor.eraser.active(),
                "release ends the gesture at {scale}x"
            );
            let session = app.tabs[0].session.as_mut().unwrap();
            assert_eq!(session.annotations.len(), 1);
            let parts = strokes(&session.annotations[0]);
            assert_eq!(parts.len(), 2, "the swept brush must hit ink at {scale}x");
            assert!((parts[0][1][0] - 0.44).abs() < 0.001);
            assert!((parts[1][0][0] - 0.56).abs() < 0.001);
            session.undo();
            assert_eq!(session.annotations, vec![original]);
        }
    }
}
