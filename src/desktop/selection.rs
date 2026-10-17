//! Multi-object selection: rectangle marquee and freeform lasso picking, then
//! moving, scaling, aligning and restyling the chosen marks together. Every group
//! change is one undo step.
use super::*;
use editing::{EditorState, Mode, Tool};
use mypdf_milestone0::annotations::{self as ann, Annotation, Box2, Kind};

#[derive(Clone, Copy)]
pub(super) enum Transform {
    Rotate(f32),
    FlipHorizontal,
    FlipVertical,
}

/// Transform around the selection center, preserving spacing within a group.
pub(super) fn transformed(marks: &[Annotation], transform: Transform) -> Vec<Annotation> {
    if marks.is_empty() {
        return vec![];
    }
    // The centroid stays fixed as the angle changes; a newly measured axis-aligned
    // bounding box can shift its center after each small rotation of a group.
    let center = marks
        .iter()
        .fold([0., 0.], |c, a| {
            [
                c[0] + (a.rect[0] + a.rect[2]) / 2.,
                c[1] + (a.rect[1] + a.rect[3]) / 2.,
            ]
        })
        .map(|v| v / marks.len() as f32);
    marks
        .iter()
        .map(|a| {
            let mut out = a.clone();
            let p = [(a.rect[0] + a.rect[2]) / 2., (a.rect[1] + a.rect[3]) / 2.];
            // A single object's own center is stable even when its artwork overhangs.
            let center = if marks.len() == 1 { p } else { center };
            let d = [p[0] - center[0], p[1] - center[1]];
            let target = match transform {
                Transform::Rotate(angle) => {
                    out.rotate_by(angle);
                    let (sin, cos) = angle.to_radians().sin_cos();
                    [
                        center[0] + d[0] * cos - d[1] * sin,
                        center[1] + d[0] * sin + d[1] * cos,
                    ]
                }
                Transform::FlipHorizontal => {
                    out.flip_horizontal();
                    [center[0] - d[0], p[1]]
                }
                Transform::FlipVertical => {
                    out.flip_vertical();
                    [p[0], center[1] - d[1]]
                }
            };
            for i in 0..4 {
                out.rect[i] += target[i % 2] - p[i % 2];
            }
            out
        })
        .collect()
}

/// A rectangle marquee or freeform lasso being dragged on one page.
pub(super) struct Marquee {
    tab: u64,
    page: usize,
    points: Vec<[f32; 2]>,
    lasso: bool,
    additive: bool,
}

#[derive(Clone, Copy)]
enum Drag {
    Move,
    /// Uniform scale about the corner opposite the dragged handle.
    Scale {
        anchor: [f32; 2],
        corner: [f32; 2],
    },
}
pub(super) struct GroupGesture {
    tab: u64,
    page: usize,
    start: [f32; 2],
    drag: Drag,
    originals: Vec<Annotation>,
    duplicate: bool,
    guides: manipulation::Guides,
    preview: Vec<Annotation>,
}

#[derive(Clone, Copy)]
pub(super) enum Align {
    Left,
    Center,
    Right,
    Top,
    Middle,
    Bottom,
}
impl Align {
    pub(super) const ALL: [(Self, Icon, &'static str); 6] = [
        (Self::Left, Icon::AlignObjectsLeft, "Align left edges"),
        (
            Self::Center,
            Icon::AlignObjectsCenter,
            "Align horizontal centers",
        ),
        (Self::Right, Icon::AlignObjectsRight, "Align right edges"),
        (Self::Top, Icon::AlignObjectsTop, "Align top edges"),
        (
            Self::Middle,
            Icon::AlignObjectsMiddle,
            "Align vertical centers",
        ),
        (Self::Bottom, Icon::AlignObjectsBottom, "Align bottom edges"),
    ];
}

/// The selected marks: the group, or the single selection.
pub(super) fn selected_ids(tab: &Tab) -> Vec<String> {
    if tab.group.is_empty() {
        tab.selected_annotation.iter().cloned().collect()
    } else {
        tab.group.clone()
    }
}
/// The group's marks in document order.
pub(super) fn members(tab: &Tab) -> Vec<&Annotation> {
    let Some(s) = &tab.session else {
        return vec![];
    };
    s.annotations
        .iter()
        .filter(|a| tab.group.contains(&a.id))
        .collect()
}
/// Replace the selection: none, one mark (the regular inspector) or a group.
pub(super) fn set(tab: &mut Tab, editor: &mut EditorState, mut ids: Vec<String>) {
    let mut seen = HashSet::new();
    ids.retain(|id| seen.insert(id.clone()));
    editor.property_edit = None;
    editor.group_edit = None;
    tab.group.clear();
    tab.selected_annotation = None;
    editor.draft = None;
    match ids.len() {
        0 => {}
        1 => {
            editor.draft = tab
                .session
                .as_ref()
                .and_then(|s| s.annotations.iter().find(|a| a.id == ids[0]))
                .cloned();
            tab.selected_annotation = ids.pop();
        }
        _ => tab.group = ids,
    }
}
/// Drop marks that no longer exist or moved to another page, collapsing a group of one.
pub(super) fn normalize(tab: &mut Tab, editor: &mut EditorState) {
    if tab.group.is_empty() {
        return;
    }
    let members: Vec<_> = members(tab)
        .iter()
        .map(|a| (a.id.clone(), a.page))
        .collect();
    let page = members.first().map(|m| m.1);
    let ids: Vec<_> = members
        .into_iter()
        .filter(|m| Some(m.1) == page)
        .map(|m| m.0)
        .collect();
    if ids.len() != tab.group.len() {
        set(tab, editor, ids);
    }
}
/// Shift-click: add the mark to the selection, or remove it.
pub(super) fn toggle(tab: &mut Tab, editor: &mut EditorState, a: &Annotation) {
    let page_of = |id: &String| {
        tab.session
            .as_ref()
            .and_then(|s| s.annotations.iter().find(|x| &x.id == id))
            .map(|x| x.page)
    };
    let mut ids: Vec<_> = selected_ids(tab)
        .into_iter()
        .filter(|id| page_of(id) == Some(a.page))
        .collect();
    if let Some(i) = ids.iter().position(|id| id == &a.id) {
        ids.remove(i);
    } else {
        ids.push(a.id.clone());
    }
    set(tab, editor, ids);
}

pub(super) fn union_bounds<'a>(marks: impl IntoIterator<Item = &'a Annotation>) -> Box2 {
    let corners: Vec<_> = marks
        .into_iter()
        .flat_map(|a| {
            let b = a.bounds();
            [[b[0], b[1]], [b[2], b[3]]]
        })
        .collect();
    ann::bounds(&corners)
}
fn center(a: &Annotation) -> [f32; 2] {
    let b = a.bounds();
    [(b[0] + b[2]) / 2., (b[1] + b[3]) / 2.]
}
fn inside_polygon(p: [f32; 2], polygon: &[[f32; 2]]) -> bool {
    let mut inside = false;
    for i in 0..polygon.len() {
        let a = polygon[i];
        let b = polygon[(i + 1) % polygon.len()];
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            inside = !inside;
        }
    }
    inside
}
/// Marks whose visual center lies inside the marquee or lasso, matching the
/// rule used by Edit → Select area for text.
fn picks(tab: &Tab, m: &Marquee) -> Vec<String> {
    let Some(s) = &tab.session else {
        return vec![];
    };
    let first = m.points[0];
    let last = *m.points.last().unwrap();
    s.annotations
        .iter()
        .filter(|a| a.page == m.page)
        .filter(|a| {
            let c = center(a);
            if m.lasso {
                m.points.len() >= 3 && inside_polygon(c, &m.points)
            } else {
                c[0] >= first[0].min(last[0])
                    && c[0] <= first[0].max(last[0])
                    && c[1] >= first[1].min(last[1])
                    && c[1] <= first[1].max(last[1])
            }
        })
        .map(|a| a.id.clone())
        .collect()
}

/// Translate every mark, keeping the whole group on the page.
pub(super) fn moved(marks: &[Annotation], d: [f32; 2], page: PageSize) -> Vec<Annotation> {
    let b = union_bounds(marks);
    let dx = d[0].clamp(-b[0], (page.width - b[2]).max(-b[0]));
    let dy = d[1].clamp(-b[1], (page.height - b[3]).max(-b[1]));
    marks
        .iter()
        .map(|a| {
            let mut a = a.clone();
            a.rect = [
                a.rect[0] + dx,
                a.rect[1] + dy,
                a.rect[2] + dx,
                a.rect[3] + dy,
            ];
            a
        })
        .collect()
}
/// Uniform scale about `anchor`. Rotated marks stay exact because every rectangle,
/// and so every rotation center, scales by the same factor.
fn scaled(marks: &[Annotation], anchor: [f32; 2], s: f32) -> Vec<Annotation> {
    marks
        .iter()
        .map(|a| {
            let mut a = a.clone();
            let r = a.rect;
            a.rect = [
                anchor[0] + (r[0] - anchor[0]) * s,
                anchor[1] + (r[1] - anchor[1]) * s,
                anchor[0] + (r[2] - anchor[0]) * s,
                anchor[1] + (r[3] - anchor[1]) * s,
            ];
            if matches!(a.kind, Kind::Text { .. }) {
                a.style.font_size = (a.style.font_size * s).clamp(6., 144.);
            }
            a
        })
        .collect()
}
/// The scale factor for a handle dragged to `p`, limited so every mark stays at
/// least 4 points across and the group stays on the page.
fn scale_factor(
    marks: &[Annotation],
    anchor: [f32; 2],
    corner: [f32; 2],
    p: [f32; 2],
    page: PageSize,
) -> f32 {
    let ratio = |axis: usize| {
        let extent = corner[axis] - anchor[axis];
        if extent.abs() < 0.001 {
            f32::NEG_INFINITY
        } else {
            (p[axis] - anchor[axis]) / extent
        }
    };
    let s = ratio(0).max(ratio(1));
    let smallest = marks
        .iter()
        .map(|a| (a.rect[2] - a.rect[0]).min(a.rect[3] - a.rect[1]))
        .fold(f32::INFINITY, f32::min);
    let minimum = (4. / smallest.max(0.001)).max(0.02);
    let b = union_bounds(marks);
    let mut maximum = 50.0_f32;
    for (axis, size) in [(0, page.width), (1, page.height)] {
        let (low, high) = (b[axis], b[axis + 2]);
        if low < anchor[axis] - 0.001 {
            maximum = maximum.min(anchor[axis] / (anchor[axis] - low));
        }
        if high > anchor[axis] + 0.001 {
            maximum = maximum.min((size - anchor[axis]) / (high - anchor[axis]));
        }
    }
    if s.is_finite() {
        s.clamp(minimum, maximum.max(minimum))
    } else {
        1.
    }
}
pub(super) fn aligned(marks: &[Annotation], align: Align) -> Vec<Annotation> {
    let group = union_bounds(marks);
    marks
        .iter()
        .map(|a| {
            let b = a.bounds();
            let (dx, dy) = match align {
                Align::Left => (group[0] - b[0], 0.),
                Align::Center => ((group[0] + group[2] - b[0] - b[2]) / 2., 0.),
                Align::Right => (group[2] - b[2], 0.),
                Align::Top => (0., group[1] - b[1]),
                Align::Middle => (0., (group[1] + group[3] - b[1] - b[3]) / 2.),
                Align::Bottom => (0., group[3] - b[3]),
            };
            let mut a = a.clone();
            a.rect = [
                a.rect[0] + dx,
                a.rect[1] + dy,
                a.rect[2] + dx,
                a.rect[3] + dy,
            ];
            a
        })
        .collect()
}
/// Replace changed marks in one commit. `previous` coalesces a continuous edit.
pub(super) fn commit(
    tab: &mut Tab,
    changed: &[Annotation],
    previous: Option<u64>,
    status: &mut Option<(String, bool)>,
) -> Option<u64> {
    let s = tab.session.as_mut()?;
    let mut entries = s.annotations.clone();
    for a in changed {
        if let Some(target) = entries.iter_mut().find(|v| v.id == a.id) {
            *target = a.clone();
        }
    }
    match s.commit_live(entries, previous) {
        Ok(()) => Some(s.revision()),
        Err(e) => {
            *status = Some((format!("{e:#}"), true));
            None
        }
    }
}
/// Move the selection by `d` page points as one undo step.
pub(super) fn nudge(
    tab: &mut Tab,
    d: [f32; 2],
    status: &mut Option<(String, bool)>,
) -> Option<u64> {
    let ids = selected_ids(tab);
    let marks: Vec<_> = tab
        .session
        .as_ref()?
        .annotations
        .iter()
        .filter(|a| ids.contains(&a.id))
        .cloned()
        .collect();
    let page = tab.pages.get(marks.first()?.page).copied()?;
    commit(tab, &moved(&marks, d, page), None, status)
}

fn corners(b: Box2) -> [[f32; 2]; 4] {
    [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]]
}
/// The group handle or body under page point `p`, if any.
fn grab(tab: &Tab, page: usize, p: [f32; 2], scale: f32) -> Option<Drag> {
    let group = members(tab);
    if group.len() < 2 || group[0].page != page {
        return None;
    }
    let b = union_bounds(group.iter().copied());
    let reach = 10. / scale;
    let corners = corners(b);
    corners
        .iter()
        .position(|c| (p[0] - c[0]).abs() < reach && (p[1] - c[1]).abs() < reach)
        .map(|i| Drag::Scale {
            anchor: corners[(i + 2) % 4],
            corner: corners[i],
        })
        .or_else(|| {
            let pad = 4. / scale;
            (p[0] >= b[0] - pad && p[0] <= b[2] + pad && p[1] >= b[1] - pad && p[1] <= b[3] + pad)
                .then_some(Drag::Move)
        })
}

/// Start a group drag, marquee or lasso at page point `p`. Returns false to
/// leave the press to the single-object move/resize and drawing tools.
pub(super) fn begin(
    editor: &mut EditorState,
    tab: &Tab,
    page: usize,
    p: [f32; 2],
    scale: f32,
    modifiers: egui::Modifiers,
) -> bool {
    if !matches!(editor.mode, Mode::Annotate | Mode::Draw)
        || !matches!(editor.tool, Tool::Select | Tool::Lasso)
        || !tab.session.as_ref().is_some_and(|s| s.editable)
    {
        return false;
    }
    if let Some(drag) = grab(tab, page, p, scale) {
        let originals: Vec<_> = members(tab).into_iter().cloned().collect();
        editor.group_gesture = Some(GroupGesture {
            tab: tab.id,
            page,
            start: p,
            drag,
            preview: originals.clone(),
            originals,
            duplicate: modifiers.alt && matches!(drag, Drag::Move),
            guides: Default::default(),
        });
        return true;
    }
    if editor.tool == Tool::Select {
        let on_handle = tab.selected_annotation.as_ref().is_some_and(|id| {
            tab.session.as_ref().is_some_and(|s| {
                s.annotations.iter().any(|a| {
                    &a.id == id
                        && a.page == page
                        && (editing::corner_at(a, p, scale).is_some()
                            || manipulation::endpoint_at(a, p, scale).is_some())
                })
            })
        });
        if on_handle || editing::hit_annotation(tab, page, p, scale).is_some() {
            return false;
        }
    }
    editor.marquee = Some(Marquee {
        tab: tab.id,
        page,
        points: vec![p],
        lasso: editor.tool == Tool::Lasso,
        additive: modifiers.shift,
    });
    true
}

/// Follow the pointer during a marquee or group drag; finish on release.
#[allow(clippy::too_many_arguments)]
pub(super) fn update(
    editor: &mut EditorState,
    tab: &mut Tab,
    page: usize,
    pointer: Option<[f32; 2]>,
    released: bool,
    scale: f32,
    modifiers: egui::Modifiers,
    status: &mut Option<(String, bool)>,
) {
    if let Some(mut m) = editor.marquee.take() {
        if m.tab != tab.id || m.page != page {
            editor.marquee = Some(m);
        } else {
            if let Some(p) = pointer {
                if m.lasso {
                    let last = *m.points.last().unwrap();
                    if m.points.len() < 4000 && (last[0] - p[0]).hypot(last[1] - p[1]) * scale > 2.
                    {
                        m.points.push(p);
                    }
                } else {
                    m.points.truncate(1);
                    m.points.push(p);
                }
            }
            if released {
                let mut ids = if m.additive {
                    selected_ids(tab)
                } else {
                    vec![]
                };
                ids.extend(picks(tab, &m));
                set(tab, editor, ids);
            } else {
                editor.marquee = Some(m);
            }
        }
    }
    if let Some(mut g) = editor.group_gesture.take() {
        if g.tab != tab.id || g.page != page {
            editor.group_gesture = Some(g);
            return;
        }
        if let Some(p) = pointer
            && let Some(size) = tab.pages.get(page).copied()
        {
            g.preview = match g.drag {
                Drag::Move => {
                    let mut delta = [p[0] - g.start[0], p[1] - g.start[1]];
                    if modifiers.shift {
                        if delta[0].abs() > delta[1].abs() {
                            delta[1] = 0.;
                        } else {
                            delta[0] = 0.;
                        }
                    }
                    let moved = moved(&g.originals, delta, size);
                    let ids = g.originals.iter().map(|a| a.id.clone()).collect::<Vec<_>>();
                    let (delta, guides) = if editor.snapping
                        && !modifiers.command
                        && !modifiers.ctrl
                        && !modifiers.shift
                    {
                        manipulation::snap(tab, page, union_bounds(moved.iter()), &ids, scale)
                    } else {
                        ([0., 0.], Default::default())
                    };
                    g.guides = guides;
                    self::moved(&moved, delta, size)
                }
                Drag::Scale { anchor, corner } => {
                    let s = scale_factor(&g.originals, anchor, corner, p, size);
                    scaled(&g.originals, anchor, s)
                }
            };
        }
        if released {
            if g.duplicate {
                let copies: Vec<_> = g
                    .preview
                    .into_iter()
                    .map(|mut a| {
                        a.id = format!("{:032x}", rand::random::<u128>());
                        a
                    })
                    .collect();
                let ids = copies.iter().map(|a| a.id.clone()).collect();
                if let Some(s) = &mut tab.session {
                    let mut entries = s.annotations.clone();
                    entries.extend(copies);
                    match s.commit(entries) {
                        Ok(()) => set(tab, editor, ids),
                        Err(e) => *status = Some((format!("{e:#}"), true)),
                    }
                }
            } else if g.preview != g.originals {
                commit(tab, &g.preview, None, status);
            }
        } else {
            editor.group_gesture = Some(g);
        }
    }
}

pub(super) fn duplicates(editor: &EditorState, tab: u64, page: usize) -> Vec<Annotation> {
    editor
        .group_gesture
        .as_ref()
        .filter(|g| g.tab == tab && g.page == page && g.duplicate)
        .map(|g| g.preview.clone())
        .unwrap_or_default()
}
pub(super) fn guides(editor: &EditorState, tab: u64, page: usize) -> manipulation::Guides {
    editor
        .group_gesture
        .as_ref()
        .filter(|g| g.tab == tab && g.page == page)
        .map(|g| g.guides)
        .unwrap_or_default()
}
/// The in-flight preview of a mark being moved or scaled with its group.
pub(super) fn preview<'a>(
    editor: &'a EditorState,
    tab: u64,
    page: usize,
    id: &str,
) -> Option<&'a Annotation> {
    editor
        .group_gesture
        .as_ref()
        .filter(|g| g.tab == tab && g.page == page && !g.duplicate)
        .and_then(|g| g.preview.iter().find(|a| a.id == id))
}

/// Outline the group with corner handles, draw a live marquee or lasso, and set
/// the hover cursor.
pub(super) fn paint(
    editor: &EditorState,
    tab: &Tab,
    page: usize,
    painter: &egui::Painter,
    page_rect: Rect,
    scale: f32,
    hover: Option<egui::Pos2>,
) {
    let to_screen = |p: [f32; 2]| page_rect.min + vec2(p[0], p[1]) * scale;
    let screen_box = |b: Box2| Rect::from_min_max(to_screen([b[0], b[1]]), to_screen([b[2], b[3]]));
    let marks: Vec<&Annotation> = match &editor.group_gesture {
        Some(g) if g.tab == tab.id && g.page == page => g.preview.iter().collect(),
        _ => members(tab)
            .into_iter()
            .filter(|a| a.page == page)
            .collect(),
    };
    if marks.len() >= 2 {
        for a in &marks {
            painter.rect_stroke(
                screen_box(a.bounds()).expand(2.),
                2.,
                Stroke::new(1., theme::accent().gamma_multiply(0.55)),
                egui::StrokeKind::Outside,
            );
        }
        let frame = screen_box(union_bounds(marks.iter().copied())).expand(6.);
        painter.rect_stroke(
            frame,
            3.,
            Stroke::new(1.3, theme::accent()),
            egui::StrokeKind::Middle,
        );
        for c in [
            frame.left_top(),
            frame.right_top(),
            frame.right_bottom(),
            frame.left_bottom(),
        ] {
            painter.circle_filled(c, 4.5, Color32::WHITE);
            painter.circle_stroke(c, 4.5, Stroke::new(1.5, theme::accent()));
        }
        let label = format!("{} selected", marks.len());
        let font = ui::font(painter.ctx(), ui::STRONG, 11.);
        let galley = painter.layout_no_wrap(label, font, Color32::WHITE);
        let pill = Rect::from_min_size(
            frame.left_top() + vec2(0., -galley.size().y - 12.),
            galley.size() + vec2(14., 6.),
        );
        painter.rect_filled(pill, 8., theme::accent());
        painter.galley(pill.min + vec2(7., 3.), galley, Color32::WHITE);
        if editor
            .group_gesture
            .as_ref()
            .is_some_and(|g| g.tab == tab.id && matches!(g.drag, Drag::Move))
        {
            painter.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if editor.group_gesture.is_none()
            && let Some(pos) = hover
            && page_rect.contains(pos)
        {
            let p = (pos - page_rect.min) / scale;
            match grab(tab, page, [p.x, p.y], scale) {
                Some(Drag::Scale { anchor, corner }) => painter.ctx().set_cursor_icon(
                    if (corner[0] - anchor[0]) * (corner[1] - anchor[1]) > 0. {
                        egui::CursorIcon::ResizeNwSe
                    } else {
                        egui::CursorIcon::ResizeNeSw
                    },
                ),
                Some(Drag::Move) => painter.ctx().set_cursor_icon(egui::CursorIcon::Grab),
                None => {}
            }
        }
    }
    if let Some(m) = editor
        .marquee
        .as_ref()
        .filter(|m| m.tab == tab.id && m.page == page)
    {
        let fill = theme::accent().gamma_multiply(0.08);
        let stroke = Stroke::new(1.2, theme::accent());
        if m.lasso {
            let points: Vec<_> = m.points.iter().map(|p| to_screen(*p)).collect();
            if m.points.len() >= 3 {
                for t in ann::triangulate(&m.points) {
                    painter.add(egui::Shape::convex_polygon(
                        t.iter().map(|p| to_screen(*p)).collect(),
                        fill,
                        Stroke::NONE,
                    ));
                }
            }
            if let (Some(first), Some(last)) = (points.first(), points.last()) {
                painter.extend(egui::Shape::dashed_line(&[*last, *first], stroke, 4., 4.));
            }
            painter.add(egui::Shape::line(points, stroke));
        } else if let (Some(a), Some(b)) = (m.points.first(), m.points.last()) {
            let r = Rect::from_two_pos(to_screen(*a), to_screen(*b));
            painter.rect_filled(r, 0., fill);
            painter.rect_stroke(r, 0., stroke, egui::StrokeKind::Middle);
        }
    }
}

/// Visual QA: show a lasso mid-drag.
#[cfg(test)]
pub(super) fn preview_lasso(editor: &mut EditorState, tab: u64, points: Vec<[f32; 2]>) {
    editor.marquee = Some(Marquee {
        tab,
        page: 0,
        points,
        lasso: true,
        additive: false,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;
    use mypdf_milestone0::annotations::Style;
    use std::time::{Duration, Instant};

    fn frame(
        app: &mut Viewer,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1100., 760.))),
            events,
            modifiers,
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            app.update(ctx, &mut eframe::Frame::_new_kittest())
        });
    }
    /// A blank document in Draw mode with two nearby marks and one far away.
    fn setup() -> (egui::Context, Viewer, Vec<Annotation>) {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        let deadline = Instant::now() + Duration::from_secs(15);
        while app.tabs[0].session.is_none() {
            assert!(Instant::now() < deadline, "blank document did not open");
            frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
            std::thread::sleep(Duration::from_millis(10));
        }
        app.editor.mode = Mode::Draw;
        app.editor.snapping = false;
        app.inspector = true;
        let marks = vec![
            Annotation::new(
                0,
                [100., 100., 160., 160.],
                Kind::Rectangle,
                Style::default(),
            ),
            Annotation::new(0, [200., 120., 260., 200.], Kind::Ellipse, Style::default()),
            Annotation::new(
                0,
                [400., 500., 460., 560.],
                Kind::Rectangle,
                Style::default(),
            ),
        ];
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(marks.clone())
            .unwrap();
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
        }
        (ctx, app, marks)
    }
    fn screen(app: &Viewer, ctx: &egui::Context, p: [f32; 2]) -> egui::Pos2 {
        let page = ctx
            .read_response(egui::Id::new((app.tabs[0].id, 0_usize, "page_actions")))
            .unwrap()
            .rect;
        page.min + vec2(p[0], p[1]) * app.tabs[0].scale
    }
    /// Press at the first page point, move through the rest, release at the last.
    fn drag(app: &mut Viewer, ctx: &egui::Context, path: &[[f32; 2]], modifiers: egui::Modifiers) {
        let points: Vec<_> = path.iter().map(|p| screen(app, ctx, *p)).collect();
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        };
        frame(
            app,
            ctx,
            vec![
                egui::Event::PointerMoved(points[0]),
                button(points[0], true),
            ],
            modifiers,
        );
        for pair in points.windows(2) {
            for step in 1..=3 {
                let p = pair[0] + (pair[1] - pair[0]) * step as f32 / 3.;
                frame(app, ctx, vec![egui::Event::PointerMoved(p)], modifiers);
            }
        }
        let last = *points.last().unwrap();
        frame(app, ctx, vec![button(last, false)], modifiers);
        frame(app, ctx, vec![], egui::Modifiers::NONE);
    }
    fn click(app: &mut Viewer, ctx: &egui::Context, p: [f32; 2], modifiers: egui::Modifiers) {
        let pos = screen(app, ctx, p);
        for pressed in [true, false] {
            frame(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers,
                    },
                ],
                modifiers,
            );
        }
    }
    fn key(app: &mut Viewer, ctx: &egui::Context, key: egui::Key, modifiers: egui::Modifiers) {
        let event = |pressed| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        frame(app, ctx, vec![event(true), event(false)], modifiers);
    }
    fn rect_of(app: &Viewer, id: &str) -> Box2 {
        app.tabs[0]
            .session
            .as_ref()
            .unwrap()
            .annotations
            .iter()
            .find(|a| a.id == id)
            .unwrap()
            .rect
    }
    fn group(app: &Viewer) -> Vec<String> {
        let mut ids = app.tabs[0].group.clone();
        ids.sort();
        ids
    }
    fn ids(marks: &[&Annotation]) -> Vec<String> {
        let mut ids: Vec<_> = marks.iter().map(|a| a.id.clone()).collect();
        ids.sort();
        ids
    }
    fn close(a: Box2, b: Box2) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.05)
    }

    #[test]
    fn group_transforms_preserve_relative_geometry_and_are_reversible() {
        let a = Annotation::new(
            0,
            [10., 20., 80., 50.],
            Kind::Rectangle,
            ann::Style::default(),
        );
        let mut b = Annotation::new(
            0,
            [120., 100., 180., 160.],
            Kind::Ellipse,
            ann::Style::default(),
        );
        b.rotate_by(25.);
        let original = vec![a, b];
        for (operation, inverse) in [
            (Transform::Rotate(90.), Transform::Rotate(-90.)),
            (Transform::Rotate(37.), Transform::Rotate(-37.)),
            (Transform::FlipHorizontal, Transform::FlipHorizontal),
            (Transform::FlipVertical, Transform::FlipVertical),
        ] {
            let changed = transformed(&original, operation);
            assert_ne!(changed, original);
            let restored = transformed(&changed, inverse);
            for (a, b) in original.iter().zip(&restored) {
                for i in 0..4 {
                    assert!((a.rect[i] - b.rect[i]).abs() < 0.001);
                }
                assert_eq!(
                    (a.rotation, a.flip_x, a.flip_y),
                    (b.rotation, b.flip_x, b.flip_y)
                );
            }
        }
    }
    #[test]
    fn marquee_selects_by_center_then_group_moves_and_scales_as_one_undo_step() {
        let (ctx, mut app, marks) = setup();
        let revision = app.tabs[0].session.as_ref().unwrap().revision();
        drag(
            &mut app,
            &ctx,
            &[[80., 80.], [280., 230.]],
            egui::Modifiers::NONE,
        );
        assert_eq!(group(&app), ids(&[&marks[0], &marks[1]]));
        assert!(app.tabs[0].selected_annotation.is_none());
        assert_eq!(
            app.tabs[0].session.as_ref().unwrap().revision(),
            revision,
            "selecting never edits the document"
        );

        // Dragging the body of the group moves both marks together.
        drag(
            &mut app,
            &ctx,
            &[[130., 130.], [170., 150.]],
            egui::Modifiers::NONE,
        );
        assert!(close(rect_of(&app, &marks[0].id), [140., 120., 200., 180.]));
        assert!(close(rect_of(&app, &marks[1].id), [240., 140., 300., 220.]));
        assert_eq!(
            rect_of(&app, &marks[2].id),
            marks[2].rect,
            "unselected marks stay put"
        );
        assert_eq!(
            group(&app).len(),
            2,
            "the group stays selected after moving"
        );
        app.editor_undo(false);
        frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
        assert_eq!(rect_of(&app, &marks[0].id), marks[0].rect);
        assert_eq!(rect_of(&app, &marks[1].id), marks[1].rect);
        assert_eq!(
            app.tabs[0].session.as_ref().unwrap().revision(),
            revision,
            "one undo reverts the whole group move"
        );

        // Scaling from the bottom-right handle is uniform about the top-left corner.
        drag(
            &mut app,
            &ctx,
            &[[80., 80.], [280., 230.]],
            egui::Modifiers::NONE,
        );
        let originals: Vec<_> = marks[..2].to_vec();
        let b = union_bounds(&originals);
        let anchor = [b[0], b[1]];
        let target = [
            anchor[0] + (b[2] - anchor[0]) * 1.5,
            anchor[1] + (b[3] - anchor[1]) * 1.5,
        ];
        drag(
            &mut app,
            &ctx,
            &[[b[2], b[3]], target],
            egui::Modifiers::NONE,
        );
        for a in &originals {
            let expected = scaled(std::slice::from_ref(a), anchor, 1.5)[0].rect;
            assert!(
                close(rect_of(&app, &a.id), expected),
                "{:?} != {expected:?}",
                rect_of(&app, &a.id)
            );
        }
    }

    #[test]
    fn lasso_and_shift_click_build_selections_and_keys_act_on_them() {
        let (ctx, mut app, marks) = setup();
        app.editor.select_tool(Tool::Lasso);
        drag(
            &mut app,
            &ctx,
            &[
                [190., 100.],
                [480., 100.],
                [480., 580.],
                [190., 580.],
                [190., 110.],
            ],
            egui::Modifiers::NONE,
        );
        assert_eq!(
            group(&app),
            ids(&[&marks[1], &marks[2]]),
            "lasso picks by polygon"
        );
        assert!(app.editor.tool == Tool::Lasso, "the lasso stays armed");

        // Shift-click adds and removes; a group of one becomes a single selection.
        app.editor.select_tool(Tool::Select);
        click(&mut app, &ctx, [130., 130.], egui::Modifiers::NONE);
        assert_eq!(
            app.tabs[0].selected_annotation.as_deref(),
            Some(marks[0].id.as_str())
        );
        click(&mut app, &ctx, [230., 160.], egui::Modifiers::SHIFT);
        assert_eq!(group(&app), ids(&[&marks[0], &marks[1]]));
        click(&mut app, &ctx, [130., 130.], egui::Modifiers::SHIFT);
        assert!(app.tabs[0].group.is_empty());
        assert_eq!(
            app.tabs[0].selected_annotation.as_deref(),
            Some(marks[1].id.as_str())
        );

        // ⌘A selects every mark on the page; Shift+arrow nudges by 10 points.
        key(&mut app, &ctx, egui::Key::A, egui::Modifiers::COMMAND);
        assert_eq!(group(&app).len(), 3);
        key(
            &mut app,
            &ctx,
            egui::Key::ArrowRight,
            egui::Modifiers::SHIFT,
        );
        for a in &marks {
            assert!(close(
                rect_of(&app, &a.id),
                [a.rect[0] + 10., a.rect[1], a.rect[2] + 10., a.rect[3]]
            ));
        }
        // The Mac delete key (Backspace) removes the whole selection in one step.
        key(&mut app, &ctx, egui::Key::Backspace, egui::Modifiers::NONE);
        assert!(app.tabs[0].session.as_ref().unwrap().annotations.is_empty());
        assert!(app.tabs[0].group.is_empty());
        app.editor_undo(false);
        assert_eq!(app.tabs[0].session.as_ref().unwrap().annotations.len(), 3);
    }

    #[test]
    fn group_transforms_stay_on_the_page_and_align_to_shared_edges() {
        let page = PageSize {
            width: 595.,
            height: 842.,
        };
        let mut rotated = Annotation::new(
            0,
            [100., 100., 200., 150.],
            Kind::Rectangle,
            Style::default(),
        );
        rotated.rotation = 30.;
        let marks = vec![
            rotated,
            Annotation::new(0, [300., 400., 360., 470.], Kind::Ellipse, Style::default()),
        ];
        let moved = moved(&marks, [1000., -1000.], page);
        let b = union_bounds(&moved);
        assert!(
            (b[2] - page.width).abs() < 0.01 && b[1].abs() < 0.01,
            "moves clamp to the page"
        );

        let original = union_bounds(&marks);
        let anchor = [original[0], original[1]];
        let corner = [original[2], original[3]];
        let s = scale_factor(&marks, anchor, corner, [5000., 5000.], page);
        let grown = union_bounds(&scaled(&marks, anchor, s));
        assert!(grown[2] <= page.width + 0.01 && grown[3] <= page.height + 0.01);
        let s = scale_factor(&marks, anchor, corner, anchor, page);
        assert!(
            scaled(&marks, anchor, s)
                .iter()
                .all(|a| a.validate().is_ok()),
            "shrinking keeps valid marks"
        );
        let doubled = scaled(&marks, anchor, 2.);
        assert_eq!(doubled[0].rotation, 30., "rotation is preserved");
        let w = |a: &Annotation| a.bounds()[2] - a.bounds()[0];
        assert!(
            (w(&doubled[0]) - 2. * w(&marks[0])).abs() < 0.05,
            "rotated marks scale exactly"
        );

        let left = aligned(&marks, Align::Left);
        assert!((left[0].bounds()[0] - left[1].bounds()[0]).abs() < 0.01);
        let middle = aligned(&marks, Align::Middle);
        let center_y = |a: &Annotation| (a.bounds()[1] + a.bounds()[3]) / 2.;
        assert!((center_y(&middle[0]) - center_y(&middle[1])).abs() < 0.01);
    }
    #[test]
    fn option_drag_duplicates_single_and_group_with_one_undo_and_snapping_matches_edges() {
        let (ctx, mut app, marks) = setup();
        set(&mut app.tabs[0], &mut app.editor, vec![marks[0].id.clone()]);
        let rev = app.tabs[0].session.as_ref().unwrap().revision();
        drag(
            &mut app,
            &ctx,
            &[[130., 130.], [180., 210.]],
            egui::Modifiers::ALT,
        );
        let s = app.tabs[0].session.as_mut().unwrap();
        assert_eq!(s.annotations.len(), 4);
        assert_eq!(s.annotations[0], marks[0]);
        assert_eq!(s.revision(), rev + 1);
        assert!(
            s.annotations
                .iter()
                .any(|a| a.id != marks[0].id && close(a.rect, [150., 180., 210., 240.]))
        );
        s.undo();
        assert_eq!(s.annotations.len(), 3);
        set(
            &mut app.tabs[0],
            &mut app.editor,
            vec![marks[0].id.clone(), marks[1].id.clone()],
        );
        drag(
            &mut app,
            &ctx,
            &[[130., 130.], [170., 170.]],
            egui::Modifiers::ALT,
        );
        let s = app.tabs[0].session.as_mut().unwrap();
        assert_eq!(s.annotations.len(), 5);
        assert_eq!(s.annotations[0], marks[0]);
        assert_eq!(s.annotations[1], marks[1]);
        s.undo();
        assert_eq!(s.annotations.len(), 3);
        set(&mut app.tabs[0], &mut app.editor, vec![marks[0].id.clone()]);
        frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
        app.editor.snapping = true;
        drag(
            &mut app,
            &ctx,
            &[[130., 130.], [229., 130.]],
            egui::Modifiers::NONE,
        );
        assert!(
            (rect_of(&app, &marks[0].id)[0] - 200.).abs() < 0.1,
            "nearby left edges snap together: {:?}",
            rect_of(&app, &marks[0].id)
        );
        let bytes = app.tabs[0]
            .session
            .as_ref()
            .unwrap()
            .snapshot_bytes()
            .unwrap();
        let pdf = lopdf::Document::load_mem(&bytes).unwrap();
        assert_eq!(ann::load(&pdf).len(), 3, "saved PDF retains all marks");
    }
}
