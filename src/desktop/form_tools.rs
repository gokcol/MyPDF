//! Forms mode: add fields of every kind by dragging on the page; select,
//! move and resize them; fill them and change their properties; and run the
//! form-wide tools: detect fields, tab order, clear, flatten, import and export.
use super::*;
use content::{Edit, Inspection};
use mypdf_milestone0::{
    convert,
    form_data::{self, DataFormat},
    form_design::{
        self, Action as ButtonAction, Align, CheckStyle, DATE_FORMATS, Designed, FieldType,
        Properties, TabOrder,
    },
    form_layout::{CellKind, Column, Layout, Plan},
    forms::FieldKind,
};

pub(super) fn icon(kind: FieldType) -> Icon {
    match kind {
        FieldType::Text => Icon::FieldText,
        FieldType::Checkbox => Icon::FieldCheck,
        FieldType::Radio => Icon::FieldRadio,
        FieldType::Dropdown => Icon::FieldDropdown,
        FieldType::ListBox => Icon::FieldList,
        FieldType::Button => Icon::FieldButton,
        FieldType::Image => Icon::FieldImage,
        FieldType::Date => Icon::FieldDate,
        FieldType::Signature => Icon::Sign,
    }
}

#[derive(Clone, Copy)]
enum Grab {
    /// Drawing a new field from this page point.
    Create(egui::Pos2),
    Move {
        widget: lopdf::ObjectId,
        rect: [f32; 4],
    },
    Resize {
        widget: lopdf::ObjectId,
        rect: [f32; 4],
        corner: usize,
    },
}
pub(super) enum Confirm {
    Clear,
    Flatten,
}
/// A field being moved or resized, drawn where it is going: its pixels from the
/// page image, over a picture of the page without it.
struct Ghost {
    tab: u64,
    page: usize,
    from: [f32; 4],
    to: [f32; 4],
    revision: u64,
    /// When the change was handed over; the ghost stays until the page is redrawn.
    dropped: Option<std::time::Instant>,
    rx: Option<mpsc::Receiver<Option<egui::ColorImage>>>,
    erased: Option<TextureHandle>,
}
/// The page as it would look without one widget.
fn without_widget(bytes: &[u8], widget: lopdf::ObjectId) -> anyhow::Result<Vec<u8>> {
    let mut doc = lopdf::Document::load_mem(bytes)?;
    for page in doc.get_pages().into_values().collect::<Vec<_>>() {
        let annots = doc.get_dictionary(page)?.get(b"Annots").ok().cloned();
        let keep = |list: &Vec<lopdf::Object>| -> Vec<lopdf::Object> {
            list.iter()
                .filter(|a| a.as_reference().ok() != Some(widget))
                .cloned()
                .collect()
        };
        match annots {
            Some(lopdf::Object::Reference(id)) => {
                if let Ok(list) = doc
                    .get_object(id)
                    .and_then(lopdf::Object::as_array)
                    .map(keep)
                {
                    *doc.get_object_mut(id)? = lopdf::Object::Array(list);
                }
            }
            Some(lopdf::Object::Array(list)) => {
                doc.get_dictionary_mut(page)?.set("Annots", keep(&list));
            }
            _ => {}
        }
    }
    let mut out = vec![];
    doc.save_to(&mut out)?;
    Ok(out)
}
fn start_ghost(
    state: &mut FormState,
    tab: &Tab,
    page: usize,
    widget: lopdf::ObjectId,
    rect: [f32; 4],
    ui: &egui::Ui,
    scale: f32,
) {
    let Some(session) = tab.session.as_ref() else {
        return;
    };
    let source = session.source.clone();
    let factor = (scale * ui.ctx().pixels_per_point()).clamp(0.5, 3.);
    let (tx, rx) = mpsc::channel();
    let wake = ui.ctx().clone();
    std::thread::spawn(move || {
        let image = std::panic::catch_unwind(|| {
            let bytes = without_widget(&source, widget).ok()?;
            let (size, rgba) = viewer::render_page(&bytes, page, factor).ok()?;
            Some(egui::ColorImage::from_rgba_premultiplied(size, &rgba))
        })
        .ok()
        .flatten();
        let _ = tx.send(image);
        wake.request_repaint();
    });
    state.ghost = Some(Ghost {
        tab: tab.id,
        page,
        from: rect,
        to: rect,
        revision: session.revision(),
        dropped: None,
        rx: Some(rx),
        erased: None,
    });
}
/// Draw a moving field: the page without it at its old place, its pixels at the new one.
fn paint_ghost(
    state: &mut FormState,
    tab: &Tab,
    ui: &egui::Ui,
    page: usize,
    page_rect: Rect,
    scale: f32,
) {
    let Some(ghost) = &mut state.ghost else {
        return;
    };
    if ghost.tab != tab.id || ghost.page != page {
        return;
    }
    if let Some(rx) = &ghost.rx
        && let Ok(image) = rx.try_recv()
    {
        ghost.rx = None;
        ghost.erased = image.map(|i| {
            ui.ctx()
                .load_texture("form_ghost_page", i, egui::TextureOptions::LINEAR)
        });
    }
    let revision = tab.session.as_ref().map_or(0, |s| s.revision());
    let redrawn =
        revision > ghost.revision && tab.rasters.keys().any(|k| k.page == page && !k.thumbnail);
    match ghost.dropped {
        // The edited page is drawn, or the change did not happen.
        Some(at) if redrawn || at.elapsed() > std::time::Duration::from_secs(4) => {
            state.ghost = None;
            return;
        }
        None if state.grab.is_none() => {
            state.ghost = None;
            return;
        }
        _ => {}
    }
    // The image the page shows now: before the edit lands, or the old one kept meanwhile.
    let source = tab.stale.get(&page).map(|r| r.texture.id()).or_else(|| {
        let wanted = (scale * ui.ctx().pixels_per_point() * 100.) as u16;
        tab.rasters
            .iter()
            .filter(|(k, _)| k.page == page && !k.thumbnail)
            .min_by_key(|(k, _)| k.scale.abs_diff(wanted))
            .map(|(_, r)| r.texture.id())
    });
    let size = tab.pages[page];
    let uv = |r: [f32; 4]| {
        Rect::from_min_max(
            pos2(r[0] / size.width, r[1] / size.height),
            pos2(r[2] / size.width, r[3] / size.height),
        )
    };
    let painter = ui
        .painter()
        .with_clip_rect(page_rect.intersect(ui.clip_rect()));
    if let Some(erased) = &ghost.erased {
        // A little wider than the field, so its smoothed border edge is covered too.
        let f = ghost.from;
        let cover = [
            (f[0] - 2.).max(0.),
            (f[1] - 2.).max(0.),
            (f[2] + 2.).min(size.width),
            (f[3] + 2.).min(size.height),
        ];
        painter.image(
            erased.id(),
            screen(page_rect, scale, cover),
            uv(cover),
            Color32::WHITE,
        );
    }
    if let Some(source) = source {
        painter.image(
            source,
            screen(page_rect, scale, ghost.to),
            uv(ghost.from),
            Color32::WHITE,
        );
    }
}
/// A finished drag: keep the ghost until the moved field is drawn, or drop it.
fn drop_ghost(state: &mut FormState, moved: bool) {
    if moved {
        if let Some(ghost) = &mut state.ghost {
            ghost.dropped = Some(std::time::Instant::now());
        }
    } else {
        state.ghost = None;
    }
}

/// Property edits not applied yet.
struct Draft {
    tab: u64,
    field: lopdf::ObjectId,
    widget: lopdf::ObjectId,
    revision: u64,
    kind: FieldType,
    original: Properties,
    edited: Properties,
}
struct ValueDraft {
    tab: u64,
    field: lopdf::ObjectId,
    revision: u64,
    original: String,
    edited: String,
}
pub(super) struct FormState {
    /// The field type placed by the next drag or click on the page.
    pub tool: Option<FieldType>,
    /// A layout placed by the next drag or click on the page.
    pub layout: Option<Layout>,
    /// The layout being set up in its dialog.
    pub layout_dialog: Option<Layout>,
    /// The settings last used for each kind of layout.
    recent_layouts: Vec<Layout>,
    /// The selected field and the widget (radio button) clicked.
    pub(super) selected: Option<(lopdf::ObjectId, lopdf::ObjectId)>,
    /// Select this field once the document is newer than this revision: name, page, rectangle.
    pending_select: Option<(String, usize, [f32; 4], u64)>,
    pending_focus: Option<(u64, lopdf::ObjectId)>,
    grab: Option<(u64, usize, Grab, [f32; 4])>,
    ghost: Option<Ghost>,
    pending_edit: Option<Edit>,
    draft: Option<Draft>,
    value: Option<ValueDraft>,
    saved_values: HashMap<(u64, lopdf::ObjectId), ValueDraft>,
    saved_properties: HashMap<(u64, lopdf::ObjectId, lopdf::ObjectId), Draft>,
    pub show_order: bool,
    confirm: Option<Confirm>,
}
impl FormState {
    fn stash_value(&mut self) {
        if let Some(draft) = self.value.take()
            && draft.original != draft.edited
        {
            self.saved_values.insert((draft.tab, draft.field), draft);
        }
    }
    fn stash_properties(&mut self) {
        if let Some(draft) = self.draft.take()
            && draft.original != draft.edited
        {
            self.saved_properties
                .insert((draft.tab, draft.field, draft.widget), draft);
        }
    }
    fn pending_field(&self, tab: u64) -> Option<lopdf::ObjectId> {
        self.value
            .iter()
            .chain(self.saved_values.values())
            .find(|d| d.tab == tab && d.original != d.edited)
            .map(|d| d.field)
            .or_else(|| {
                self.draft
                    .iter()
                    .chain(self.saved_properties.values())
                    .find(|d| d.tab == tab && d.original != d.edited)
                    .map(|d| d.field)
            })
    }
    pub fn new() -> Self {
        Self {
            tool: None,
            layout: None,
            layout_dialog: None,
            recent_layouts: vec![],
            selected: None,
            pending_select: None,
            pending_focus: None,
            grab: None,
            ghost: None,
            pending_edit: None,
            draft: None,
            value: None,
            saved_values: HashMap::new(),
            saved_properties: HashMap::new(),
            show_order: false,
            confirm: None,
        }
    }
}

/// The inspection's designed fields, if it belongs to this tab.
fn designed(inspection: Option<&Inspection>, tab: u64) -> Option<&[Designed]> {
    let info = inspection.filter(|i| i.key.0 == tab)?;
    info.designed.as_ref().ok().map(Vec::as_slice)
}
fn screen(page_rect: Rect, scale: f32, r: [f32; 4]) -> Rect {
    Rect::from_min_max(
        page_rect.min + vec2(r[0], r[1]) * scale,
        page_rect.min + vec2(r[2], r[3]) * scale,
    )
}
fn next_name(fields: &[Designed], kind: FieldType) -> String {
    (1..)
        .map(|n| format!("{}{n}", kind.base_name()))
        .find(|n| !fields.iter().any(|f| &f.name == n))
        .unwrap()
}
fn corners(r: Rect) -> [egui::Pos2; 4] {
    [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
    ]
}
/// A rectangle after dragging one corner, kept at least 6 points across.
fn resized(rect: [f32; 4], corner: usize, d: egui::Vec2) -> [f32; 4] {
    let mut r = rect;
    match corner {
        0 => {
            r[0] += d.x;
            r[1] += d.y;
        }
        1 => {
            r[2] += d.x;
            r[1] += d.y;
        }
        2 => {
            r[2] += d.x;
            r[3] += d.y;
        }
        _ => {
            r[0] += d.x;
            r[3] += d.y;
        }
    }
    [
        r[0].min(r[2] - 6.),
        r[1].min(r[3] - 6.),
        r[2].max(r[0] + 6.),
        r[3].max(r[1] + 6.),
    ]
}

/// Field boxes on the page: create with a tool, select, move and resize.
#[allow(clippy::too_many_arguments)]
pub(super) fn overlay(
    state: &mut FormState,
    inspection: Option<&Inspection>,
    tab: &Tab,
    editable: bool,
    ui: &mut egui::Ui,
    page: usize,
    page_rect: Rect,
    scale: f32,
) {
    let Some(fields) = designed(inspection, tab.id) else {
        return;
    };
    let size = tab.pages[page];
    let clip = page_rect.intersect(ui.clip_rect());
    let painter = ui.painter().with_clip_rect(clip);
    let to_page = |p: egui::Pos2| (p - page_rect.min) / scale;
    // Behind the fields: clicks on empty paper deselect, drags with a tool create.
    let placing = state.tool.is_some() || state.layout.is_some();
    let background = ui
        .interact(
            page_rect,
            egui::Id::new((tab.id, page, "form_page")),
            if placing && editable {
                egui::Sense::click_and_drag()
            } else {
                egui::Sense::click()
            },
        )
        .on_hover_cursor(if placing {
            egui::CursorIcon::Crosshair
        } else {
            egui::CursorIcon::Default
        });
    if placing && editable {
        // Capture the press before the drag threshold. Native input can deliver
        // the final movement and release in one frame, in which case egui never
        // reports `drag_started`/`drag_stopped` for this response.
        let pressed_here =
            background.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed());
        if (pressed_here || background.drag_started())
            && let Some(p) = ui.input(|i| i.pointer.press_origin())
        {
            let p = to_page(p);
            state.grab = Some((
                tab.id,
                page,
                Grab::Create(p.to_pos2()),
                [p.x, p.y, p.x, p.y],
            ));
        }
        if let Some((t, g, Grab::Create(start), live)) = &mut state.grab
            && *t == tab.id
            && *g == page
            && let Some(p) = ui.ctx().pointer_latest_pos()
        {
            let p = to_page(p);
            *live = [
                start.x.min(p.x).max(0.),
                start.y.min(p.y).max(0.),
                start.x.max(p.x).min(size.width),
                start.y.max(p.y).min(size.height),
            ];
        }
        let finishing_own_drag = matches!(state.grab, Some((t, g, Grab::Create(_), _)) if t == tab.id && g == page)
            && ui.input(|i| i.pointer.primary_released());
        let released = background.drag_stopped() || background.clicked() || finishing_own_drag;
        if released
            && let Some(p) = background
                .interact_pointer_pos()
                .or_else(|| ui.ctx().pointer_latest_pos())
        {
            let drawn = match state.grab.take() {
                Some((_, _, Grab::Create(_), r)) if r[2] - r[0] >= 6. && r[3] - r[1] >= 6. => {
                    Some(r)
                }
                _ => None,
            };
            // A click places the standard size, its middle at the click.
            let [w, h] = match (&state.layout, state.tool) {
                (Some(layout), _) => {
                    let [w, h] = layout.default_size();
                    [w.min(size.width - 24.), h.min(size.height - 24.)]
                }
                (None, Some(kind)) => kind.default_size(),
                (None, None) => [100., 22.],
            };
            let rect = drawn.unwrap_or_else(|| {
                let p = to_page(p);
                let x = (p.x - w / 2.).clamp(0., (size.width - w).max(0.));
                let y = (p.y - h / 2.).clamp(0., (size.height - h).max(0.));
                [x, y, x + w, y + h]
            });
            if let Some(layout) = state.layout.take() {
                state.pending_edit = Some(Edit::InsertLayout { page, rect, layout });
            } else if let Some(kind) = state.tool.take() {
                // A radio button joins the selected radio group; anything else gets a new name.
                let group = state
                    .selected
                    .and_then(|(f, _)| fields.iter().find(|d| d.id == f))
                    .filter(|d| kind == FieldType::Radio && d.kind == FieldType::Radio);
                let properties = match group {
                    Some(group) => group.properties.clone(),
                    None => Properties::new(kind, &next_name(fields, kind)),
                };
                let revision = tab.session.as_ref().map_or(0, |s| s.revision());
                state.pending_select = Some((properties.name.clone(), page, rect, revision));
                state.pending_edit = Some(Edit::AddField {
                    page,
                    rect,
                    kind,
                    properties,
                });
            }
        }
        if let Some((t, g, Grab::Create(_), r)) = &state.grab
            && *t == tab.id
            && *g == page
        {
            let shown = screen(page_rect, scale, *r);
            painter.rect_filled(shown, 2., theme::accent().gamma_multiply(0.08));
            ui::dashed_rect(&painter, shown, theme::accent());
            // A layout shows its rows, columns and fields as it is drawn.
            if let Some(Ok(plan)) = state.layout.as_ref().map(|l| l.plan(*r)) {
                paint_plan(
                    &painter,
                    &plan,
                    |p| page_rect.min + vec2(p[0], p[1]) * scale,
                    scale,
                );
            }
        }
    } else if background.clicked() {
        state.selected = None;
    }

    paint_ghost(state, tab, ui, page, page_rect, scale);
    let hovered_field = ui.ctx().pointer_hover_pos();
    for field in fields {
        for w in field.widgets.iter().filter(|w| w.page == page) {
            let selected = state.selected.is_some_and(|(f, _)| f == field.id);
            let this = state.selected.is_some_and(|(_, wd)| wd == w.id);
            let live = match &state.grab {
                Some((t, g, Grab::Move { widget, .. } | Grab::Resize { widget, .. }, r))
                    if *t == tab.id && *g == page && *widget == w.id =>
                {
                    *r
                }
                _ => w.rect,
            };
            let r = screen(page_rect, scale, live);
            let hovered = hovered_field.is_some_and(|p| r.contains(p));
            painter.rect_filled(
                r,
                2.,
                theme::accent().gamma_multiply(if selected { 0.16 } else { 0.07 }),
            );
            painter.rect_stroke(
                r,
                2.,
                Stroke::new(
                    if this { 2. } else { 1. },
                    if selected || hovered {
                        theme::accent()
                    } else {
                        theme::accent_line()
                    },
                ),
                egui::StrokeKind::Outside,
            );
            if hovered || this || state.show_order {
                // The name, or the tab-order number, on a chip above the field.
                let label = if state.show_order {
                    format!("{}", w.tab + 1)
                } else {
                    field.name.clone()
                };
                let galley =
                    painter.layout_no_wrap(label, FontId::proportional(10.5), Color32::WHITE);
                let chip = Rect::from_min_size(
                    r.left_top() - vec2(0., galley.size().y + 4.),
                    galley.size() + vec2(8., 4.),
                );
                painter.rect_filled(chip, 4., theme::accent());
                painter.galley(chip.min + vec2(4., 2.), galley, Color32::WHITE);
            }
            if placing || !editable {
                continue;
            }
            let locked = field.signed;
            // Handles sit just outside the corners and come before the body, so a
            // small field is still moved by its middle.
            if this && !locked {
                for (corner, at) in corners(r).into_iter().enumerate() {
                    let out = (at - r.center()).normalized() * 3.;
                    let at = at + out;
                    painter.circle_filled(at, 4.5, Color32::WHITE);
                    painter.circle_stroke(at, 4.5, Stroke::new(1.5, theme::accent()));
                    let handle = ui
                        .interact(
                            Rect::from_center_size(at, vec2(11., 11.)),
                            egui::Id::new((tab.id, page, "form_resize", w.id, corner)),
                            egui::Sense::drag(),
                        )
                        .on_hover_cursor(if corner % 2 == 0 {
                            egui::CursorIcon::ResizeNwSe
                        } else {
                            egui::CursorIcon::ResizeNeSw
                        });
                    if handle.drag_started() {
                        state.grab = Some((
                            tab.id,
                            page,
                            Grab::Resize {
                                widget: w.id,
                                rect: w.rect,
                                corner,
                            },
                            w.rect,
                        ));
                        start_ghost(state, tab, page, w.id, w.rect, ui, scale);
                    }
                    if handle.dragged()
                        && let Some((_, _, Grab::Resize { rect, corner, .. }, live)) =
                            &mut state.grab
                    {
                        let d = handle.total_drag_delta().unwrap_or_default() / scale;
                        *live = resized(*rect, *corner, d);
                        if let Some(ghost) = &mut state.ghost {
                            ghost.to = *live;
                        }
                    }
                    if handle.drag_stopped()
                        && let Some((_, _, Grab::Resize { widget, rect, .. }, live)) =
                            state.grab.take()
                    {
                        drop_ghost(state, live != rect);
                        if live != rect {
                            state.pending_edit = Some(Edit::PlaceField { widget, rect: live });
                        }
                    }
                }
            }
            let response = ui
                .interact(
                    r.intersect(page_rect),
                    egui::Id::new((tab.id, page, "form_widget", w.id)),
                    egui::Sense::click_and_drag(),
                )
                .on_hover_text(format!("{} · {}", field.name, field.kind.label()))
                .on_hover_cursor(if response_dragging(ui, tab.id, page, w.id) {
                    egui::CursorIcon::Grabbing
                } else {
                    egui::CursorIcon::Grab
                });
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Button,
                    true,
                    this,
                    format!(
                        "{} · {} · page {}{}",
                        field.name,
                        field.kind.label(),
                        page + 1,
                        if locked {
                            " · signed, read only"
                        } else {
                            " · arrow keys move, Shift moves 10 points"
                        }
                    ),
                )
            });
            if response.has_focus() {
                painter.rect_stroke(
                    r.expand(3.),
                    3.,
                    Stroke::new(2., theme::accent()),
                    egui::StrokeKind::Outside,
                );
            }
            if response.clicked() || response.drag_started() || response.gained_focus() {
                state.selected = Some((field.id, w.id));
            }
            if response.drag_started() && !locked {
                state.grab = Some((
                    tab.id,
                    page,
                    Grab::Move {
                        widget: w.id,
                        rect: w.rect,
                    },
                    w.rect,
                ));
                start_ghost(state, tab, page, w.id, w.rect, ui, scale);
            }
            if response.dragged()
                && let Some((_, _, Grab::Move { rect, .. }, live)) = &mut state.grab
            {
                let d = response.total_drag_delta().unwrap_or_default() / scale;
                let (w_, h_) = (rect[2] - rect[0], rect[3] - rect[1]);
                let x = (rect[0] + d.x).clamp(0., (size.width - w_).max(0.));
                let y = (rect[1] + d.y).clamp(0., (size.height - h_).max(0.));
                *live = [x, y, x + w_, y + h_];
                if let Some(ghost) = &mut state.ghost {
                    ghost.to = *live;
                }
            }
            if response.drag_stopped()
                && let Some((_, _, Grab::Move { widget, rect }, live)) = state.grab.take()
            {
                drop_ghost(state, live != rect);
                if live != rect {
                    state.pending_edit = Some(Edit::PlaceField { widget, rect: live });
                }
            }
        }
    }
}
/// A layout's shading, lines, field boxes and labels, for previews.
fn paint_plan(
    painter: &egui::Painter,
    plan: &Plan,
    at: impl Fn([f32; 2]) -> egui::Pos2,
    scale: f32,
) {
    let rect = |r: [f32; 4]| Rect::from_min_max(at([r[0], r[1]]), at([r[2], r[3]]));
    for s in &plan.shades {
        painter.rect_filled(rect(*s), 0., Color32::from_rgb(237, 240, 245));
    }
    for l in &plan.lines {
        painter.line_segment(
            [at([l[0], l[1]]), at([l[2], l[3]])],
            Stroke::new(1., Color32::from_rgb(90, 97, 110)),
        );
    }
    for f in &plan.fields {
        let r = rect(f.rect);
        painter.rect_filled(r, 2., theme::accent().gamma_multiply(0.14));
        painter.rect_stroke(
            r,
            2.,
            Stroke::new(1., theme::accent()),
            egui::StrokeKind::Inside,
        );
    }
    for label in &plan.labels {
        let r = rect(label.rect);
        let size = (label.size * scale).clamp(5., 14.);
        let galley = painter.layout_no_wrap(
            label.text.clone(),
            FontId::proportional(size),
            Color32::from_rgb(34, 38, 46),
        );
        let x = if label.centered {
            r.center().x - galley.size().x / 2.
        } else {
            r.left()
        };
        painter.with_clip_rect(r.expand(1.)).galley(
            pos2(x, r.center().y - galley.size().y / 2.),
            galley,
            Color32::BLACK,
        );
    }
}
fn response_dragging(ui: &egui::Ui, tab: u64, page: usize, widget: lopdf::ObjectId) -> bool {
    ui.ctx()
        .dragged_id()
        .is_some_and(|id| id == egui::Id::new((tab, page, "form_widget", widget)))
}

impl Viewer {
    fn form_editable(&self) -> bool {
        !self.content.busy
            && !self.organizer.busy
            && self
                .tabs
                .get(self.active)
                .and_then(|t| t.session.as_ref())
                .is_some_and(|s| s.editable)
    }
    /// The current inspection, when it is up to date with the active document.
    pub(super) fn fresh_inspection(&self) -> Option<&Inspection> {
        let tab = self.tabs.get(self.active)?;
        let revision = tab.session.as_ref()?.revision();
        self.content
            .inspection
            .as_ref()
            .filter(|i| i.key.0 == tab.id && i.key.1 == revision)
    }

    /// The Forms toolbar: a tool for each field type, then form-wide actions.
    pub(super) fn form_ribbon(&mut self, ctx: &egui::Context) {
        if self.editor.mode != editing::Mode::Forms {
            return;
        }
        self.inspect_content(ctx);
        let editable = self.form_editable();
        let has_fields = self
            .fresh_inspection()
            .and_then(|i| i.designed.as_ref().ok())
            .is_some_and(|d| !d.is_empty());
        let mut command = None;
        egui::TopBottomPanel::top("form_ribbon")
            .frame(
                egui::Frame::new()
                    .fill(theme::surface())
                    .inner_margin(egui::Margin::symmetric(16, 7)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(((ui.available_width() - 1160.) / 2.).max(0.));
                    for kind in FieldType::ALL {
                        let selected = self.forms.tool == Some(kind);
                        if ui
                            .add_enabled(
                                editable,
                                ActionButton::new(icon(kind), kind.label())
                                    .compact()
                                    .selected(selected),
                            )
                            .on_hover_text(format!(
                                "{} · drag on the page, or click to place it",
                                kind.label()
                            ))
                            .clicked()
                        {
                            self.forms.tool = (!selected).then_some(kind);
                            self.inspector = true;
                            // In the status bar, so the page does not shift.
                            self.status = (!selected).then(|| {
                                (
                                    format!(
                                        "Drag where the {} goes, or click to place it · Esc cancels",
                                        kind.label().to_lowercase()
                                    ),
                                    false,
                                )
                            });
                        }
                    }
                    let layouts = ui.add_enabled(
                        editable,
                        ActionButton::new(Icon::Form, "Layouts").dropdown(),
                    );
                    egui::Popup::menu(&layouts).show(|ui| {
                        ui.set_min_width(230.);
                        for (icon, layout) in [
                            (Icon::Form, Layout::table()),
                            (Icon::FieldText, Layout::labeled()),
                            (Icon::FieldCheck, Layout::checklist()),
                            (Icon::FieldRadio, Layout::choice()),
                            (Icon::Sign, Layout::signatures()),
                        ] {
                            if ui.add(ActionButton::new(icon, &format!("{}…", layout.title()))).clicked() {
                                // The last settings of this kind, or its defaults.
                                let kept = self
                                    .forms
                                    .recent_layouts
                                    .iter()
                                    .find(|l| l.title() == layout.title())
                                    .cloned();
                                self.forms.layout_dialog = Some(kept.unwrap_or(layout));
                                self.forms.tool = None;
                                ui.close();
                            }
                        }
                    });
                    ui.separator();
                    let mine = ui.add(ActionButton::new(Icon::Person, "My details").dropdown());
                    egui::Popup::menu(&mine).show(|ui| {
                        ui.set_min_width(230.);
                        if ui
                            .add_enabled(
                                editable && has_fields,
                                ActionButton::new(Icon::Person, "Fill with my details…"),
                            )
                            .on_hover_text("Put your name, email, address and so on in the empty fields that ask for them")
                            .clicked()
                        {
                            command = Some("form_fill_details");
                            ui.close();
                        }
                        if ui
                            .add(ActionButton::new(Icon::Text, "Edit my details…"))
                            .clicked()
                        {
                            command = Some("my_details");
                            ui.close();
                        }
                    });
                    if ui
                        .add_enabled(editable, ActionButton::new(Icon::Sparkle, "Detect fields"))
                        .on_hover_text("Find blanks, boxes and lines on every page and turn them into fields")
                        .clicked()
                    {
                        command = Some("form_detect");
                    }
                    let order = ui.add_enabled(
                        editable && has_fields,
                        ActionButton::new(Icon::TabOrder, "Tab order").dropdown(),
                    );
                    egui::Popup::menu(&order).show(|ui| {
                        ui.set_min_width(220.);
                        for (label, key) in [
                            ("Order by rows", "form_tab_rows"),
                            ("Order by columns", "form_tab_columns"),
                        ] {
                            if ui.add(ActionButton::new(Icon::TabOrder, label)).clicked() {
                                command = Some(key);
                                ui.close();
                            }
                        }
                        ui.separator();
                        ui.checkbox(&mut self.forms.show_order, "Show tab order numbers");
                    });
                    let data = ui.add_enabled(
                        has_fields,
                        ActionButton::new(Icon::Export, "Form data").dropdown(),
                    );
                    egui::Popup::menu(&data).show(|ui| {
                        ui.set_min_width(200.);
                        if ui
                            .add_enabled(editable, ActionButton::new(Icon::Import, "Import data…"))
                            .on_hover_text("Fill this form from FDF, XFDF or CSV")
                            .clicked()
                        {
                            command = Some("form_import");
                            ui.close();
                        }
                        if ui
                            .add(ActionButton::new(Icon::Export, "Export data…"))
                            .on_hover_text("Save the values as FDF, XFDF or CSV")
                            .clicked()
                        {
                            command = Some("form_export");
                            ui.close();
                        }
                    });
                    if ui
                        .add_enabled(editable && has_fields, ActionButton::new(Icon::Close, "Clear"))
                        .on_hover_text("Reset every field to its default value")
                        .clicked()
                    {
                        command = Some("form_clear");
                    }
                    if ui
                        .add_enabled(editable && has_fields, ActionButton::new(Icon::Flatten, "Flatten"))
                        .on_hover_text("Make the fields part of the page, so they can no longer change")
                        .clicked()
                    {
                        command = Some("form_flatten");
                    }
                });
            });
        if let Some(command) = command {
            self.form_command(command, ctx);
        }
    }

    /// Form-wide commands, from the toolbar or the menu.
    pub(super) fn form_command(&mut self, command: &str, ctx: &egui::Context) {
        self.editor.mode = editing::Mode::Forms;
        if !self.form_editable() && !matches!(command, "form_export" | "my_details") {
            self.status = Some(("This document cannot be changed right now.".into(), true));
            return;
        }
        match command {
            "form_fill_details" => self.fill_with_details(),
            "my_details" => self.edit_details(false),
            "form_detect" => self.content_change(ctx, Edit::DetectFields),
            "form_tab_rows" => self.content_change(ctx, Edit::TabOrder(TabOrder::Rows)),
            "form_tab_columns" => self.content_change(ctx, Edit::TabOrder(TabOrder::Columns)),
            "form_clear" => self.forms.confirm = Some(Confirm::Clear),
            "form_flatten" => self.forms.confirm = Some(Confirm::Flatten),
            "form_import" => self.import_form_data(ctx),
            "form_export" => self.export_form_data(),
            _ => {}
        }
        if command.starts_with("form_tab") {
            self.forms.show_order = true;
        }
    }
    fn import_form_data(&mut self, ctx: &egui::Context) {
        let Some(path) = self.native_pick(
            rfd::FileDialog::new()
                .set_title("Import form data")
                .add_filter("Form data", &["fdf", "xfdf", "xml", "csv", "txt"]),
        ) else {
            return;
        };
        let format = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(DataFormat::from_extension);
        let Some(format) = format else {
            self.status = Some(("Choose an FDF, XFDF or CSV file.".into(), true));
            return;
        };
        match convert::read_bounded(&path, 16 * 1024 * 1024) {
            Ok(data) => self.content_change(ctx, Edit::ImportFormData { data, format }),
            Err(e) => self.status = Some((format!("{e:#}"), true)),
        }
    }
    fn export_form_data(&mut self) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let Some(source) = tab.session.as_ref().map(|s| s.source.clone()) else {
            return;
        };
        let stem = tab
            .path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let pdf_name = tab.name();
        let Some(path) = self.native_save(
            rfd::FileDialog::new()
                .set_title("Export form data")
                .add_filter("FDF (Acrobat form data)", &["fdf"])
                .add_filter("XFDF (XML form data)", &["xfdf"])
                .add_filter("CSV (spreadsheet)", &["csv"])
                .set_file_name(format!("{stem}.fdf")),
        ) else {
            return;
        };
        let format = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(DataFormat::from_extension)
            .unwrap_or(DataFormat::Fdf);
        let written = form_data::export(&source, format, &pdf_name).and_then(|data| {
            let dir = path
                .parent()
                .filter(|d| !d.as_os_str().is_empty())
                .unwrap_or(std::path::Path::new("."));
            let mut file = tempfile::NamedTempFile::new_in(dir)?;
            std::io::Write::write_all(&mut file, &data)?;
            file.persist(&path).map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(())
        });
        self.status = Some(match written {
            Ok(()) => (
                format!(
                    "Form data saved as {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ),
                false,
            ),
            Err(e) => (format!("{e:#}"), true),
        });
    }

    /// Save/close must not silently discard an inspector draft.
    pub(super) fn pending_form_edits(&self, tab: u64) -> bool {
        self.forms.pending_field(tab).is_some()
    }
    pub(super) fn focus_pending_form_edits(&mut self, tab: u64) -> bool {
        let Some(field) = self.forms.pending_field(tab) else {
            return false;
        };
        let Some(index) = self.tabs.iter().position(|t| t.id == tab) else {
            return false;
        };
        self.active = index;
        self.home.visible = false;
        self.inspector = true;
        self.editor.mode = editing::Mode::Forms;
        self.forms.pending_focus = Some((tab, field));
        if let Some(designed) = self
            .fresh_inspection()
            .and_then(|i| i.designed.as_ref().ok())
            .and_then(|fields| fields.iter().find(|f| f.id == field))
            && let Some(widget) = designed.widgets.first()
        {
            self.forms.selected = Some((field, widget.id));
        }
        true
    }
    pub(super) fn clear_form_drafts(&mut self, tab: u64) {
        if self.forms.pending_focus.is_some_and(|(t, _)| t == tab) {
            self.forms.pending_focus = None;
        }
        if self.forms.value.as_ref().is_some_and(|d| d.tab == tab) {
            self.forms.value = None;
        }
        if self.forms.draft.as_ref().is_some_and(|d| d.tab == tab) {
            self.forms.draft = None;
        }
        self.forms.saved_values.retain(|(t, _), _| *t != tab);
        self.forms.saved_properties.retain(|(t, _, _), _| *t != tab);
    }

    /// Edits requested on the page, selection upkeep and keys.
    pub(super) fn form_events(&mut self, ctx: &egui::Context) {
        if self.editor.mode != editing::Mode::Forms {
            self.forms.tool = None;
            self.forms.layout = None;
            self.forms.grab = None;
            return;
        }
        if let Some(edit) = self.forms.pending_edit.take() {
            self.content_change(ctx, edit);
        }
        if !ctx.input(|i| i.pointer.any_down()) {
            self.forms.grab = None;
        }
        let focus = ctx.memory(|m| m.focused());
        let canvas_focus = self.tabs.get(self.active).is_some_and(|tab| {
            self.fresh_inspection()
                .and_then(|i| i.designed.as_ref().ok())
                .is_some_and(|fields| {
                    fields.iter().any(|f| {
                        f.widgets.iter().any(|w| {
                            focus == Some(egui::Id::new((tab.id, w.page, "form_widget", w.id)))
                        })
                    })
                })
        });
        let typing = self.editor.text_editing || focus.is_some() && !canvas_focus;
        if !typing && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            if self.forms.tool.is_some() || self.forms.layout.is_some() {
                self.forms.tool = None;
                self.forms.layout = None;
                self.status = None;
            } else {
                self.forms.selected = None;
            }
        }
        let Some(fields) = self
            .fresh_inspection()
            .and_then(|i| i.designed.as_ref().ok())
            .cloned()
        else {
            return;
        };
        let active_tab = self.tabs[self.active].id;
        if let Some((tab, field)) = self.forms.pending_focus
            && tab == active_tab
            && let Some(widget) = fields
                .iter()
                .find(|f| f.id == field)
                .and_then(|f| f.widgets.first())
        {
            self.forms.pending_focus = None;
            self.forms.selected = Some((field, widget.id));
            self.tabs[self.active].jump(widget.page);
        }
        // A deleted field cannot leave a hidden draft that blocks Save forever.
        let exists = |id| fields.iter().any(|f| f.id == id);
        self.forms
            .saved_values
            .retain(|(tab, field), _| *tab != active_tab || exists(*field));
        self.forms
            .saved_properties
            .retain(|(tab, field, _), _| *tab != active_tab || exists(*field));
        if self
            .forms
            .value
            .as_ref()
            .is_some_and(|d| d.tab == active_tab && !exists(d.field))
        {
            self.forms.value = None;
        }
        if self
            .forms
            .draft
            .as_ref()
            .is_some_and(|d| d.tab == active_tab && !exists(d.field))
        {
            self.forms.draft = None;
        }
        // A field just added: select it once the document includes it.
        let revision = self
            .tabs
            .get(self.active)
            .and_then(|t| t.session.as_ref())
            .map_or(0, |s| s.revision());
        if self
            .forms
            .pending_select
            .as_ref()
            .is_some_and(|p| revision > p.3 || self.content.busy)
            && !self.content.busy
            && let Some((name, page, rect, _)) = self.forms.pending_select.take()
        {
            let found = fields.iter().find(|f| f.name == name).and_then(|f| {
                f.widgets
                    .iter()
                    .filter(|w| w.page == page)
                    .min_by(|a, b| {
                        let d = |w: &form_design::DesignWidget| {
                            (w.rect[0] - rect[0]).abs() + (w.rect[1] - rect[1]).abs()
                        };
                        d(a).total_cmp(&d(b))
                    })
                    .map(|w| (f.id, w.id))
            });
            if found.is_some() {
                self.forms.selected = found;
            }
        }
        if self.forms.selected.is_some_and(|(f, w)| {
            !fields
                .iter()
                .any(|d| d.id == f && d.widgets.iter().any(|x| x.id == w))
        }) {
            self.forms.selected = None;
        }
        if !typing
            && canvas_focus
            && self.form_editable()
            && let Some((field_id, widget_id)) = self.forms.selected
            && let Some(field) = fields.iter().find(|f| f.id == field_id && !f.signed)
            && let Some(widget) = field.widgets.iter().find(|w| w.id == widget_id)
        {
            let delta = ctx.input_mut(|i| {
                let modifiers = if i.modifiers.shift {
                    egui::Modifiers::SHIFT
                } else {
                    egui::Modifiers::NONE
                };
                let step = if modifiers.shift { 10. } else { 1. };
                let mut delta = egui::Vec2::ZERO;
                for (key, direction) in [
                    (egui::Key::ArrowLeft, vec2(-1., 0.)),
                    (egui::Key::ArrowRight, vec2(1., 0.)),
                    (egui::Key::ArrowUp, vec2(0., -1.)),
                    (egui::Key::ArrowDown, vec2(0., 1.)),
                ] {
                    if i.consume_key(modifiers, key) {
                        delta += direction * step;
                    }
                }
                delta
            });
            if delta != egui::Vec2::ZERO {
                let page = self.tabs[self.active].pages[widget.page];
                let [l, t, r, b] = widget.rect;
                let x = (l + delta.x).clamp(0., (page.width - (r - l)).max(0.));
                let y = (t + delta.y).clamp(0., (page.height - (b - t)).max(0.));
                self.content_change(
                    ctx,
                    Edit::PlaceField {
                        widget: widget_id,
                        rect: [x, y, x + r - l, y + b - t],
                    },
                );
            }
        }
        let delete = !typing
            && ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                    || i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
            });
        if delete
            && self.form_editable()
            && let Some((field, widget)) = self.forms.selected.take()
            && let Some(f) = fields.iter().find(|f| f.id == field)
            && !f.signed
        {
            let widget = (f.widgets.len() > 1).then_some(widget);
            self.content_change(ctx, Edit::RemoveField { field, widget });
        }
    }

    pub(super) fn form_dialogs(&mut self, ctx: &egui::Context) {
        self.layout_dialog(ctx);
        let Some(confirm) = &self.forms.confirm else {
            return;
        };
        let (title, body, label, edit) = match confirm {
            Confirm::Clear => (
                "Clear the form?",
                "Every field goes back to its default value. Undo is available.",
                "Clear",
                Edit::ClearForm,
            ),
            Confirm::Flatten => (
                "Flatten the form fields?",
                "The values become part of the page and can no longer be changed. Signature fields stay, so the PDF can still be signed. Undo is available until you save.",
                "Flatten",
                Edit::FlattenForm,
            ),
        };
        match ui::confirmation(
            ctx,
            "form_confirm",
            title,
            body,
            &[
                (label, ui::DialogTone::Primary),
                ("Cancel", ui::DialogTone::Secondary),
            ],
            1,
        ) {
            Some(0) => {
                self.forms.confirm = None;
                self.content_change(ctx, edit);
            }
            Some(_) => self.forms.confirm = None,
            None => {}
        }
    }

    /// Sign an empty form signature field with the starred signature.
    fn sign_field(&mut self, name: String, page: usize, rect: [f32; 4]) {
        self.digital.target_field = Some((name, page, rect));
        self.quick_sign();
    }

    /// The Forms inspector: the selected field's value and properties, or the
    /// list of fields.
    pub(super) fn form_panel(&mut self, ctx: &egui::Context) {
        let editable = self.form_editable();
        let revision = self
            .tabs
            .get(self.active)
            .and_then(|t| t.session.as_ref())
            .map_or(0, |s| s.revision());
        let (fields, values) = match self.fresh_inspection() {
            Some(info) => (
                info.designed.clone(),
                info.fields.as_ref().ok().cloned().unwrap_or_default(),
            ),
            None => (Ok(vec![]), vec![]),
        };
        let loading = self.content.loading || self.fresh_inspection().is_none();
        let page = self.tabs.get(self.active).map_or(0, |t| t.page);
        let mut edit = None;
        let mut sign = None;
        let mut jump = None;
        let mut pick_image = None;
        egui::SidePanel::right("form_inspector")
            .exact_width(310.)
            .resizable(false)
            .frame(egui::Frame::new().fill(theme::paper()).inner_margin(18.))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 10.;
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Form").size(17.).family(ui::semibold()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(ActionButton::new(Icon::Close, "Hide panel").compact())
                            .clicked()
                        {
                            self.inspector = false;
                        }
                        if self.content.busy || loading {
                            ui.spinner();
                        }
                    });
                });
                let fields = match fields {
                    Ok(f) => f,
                    Err(e) => {
                        ui::callout(ui, ui::Tone::Warning, &e);
                        return;
                    }
                };
                if let Some(tab) = self.tabs.get(self.active).map(|t| t.id)
                    && let Some(pending) = self.forms.pending_field(tab)
                    && self
                        .forms
                        .selected
                        .is_none_or(|(field, _)| field != pending)
                {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new("Unapplied form edits")
                                .small()
                                .color(theme::warning()),
                        );
                        if ui.add(ActionButton::new(Icon::Form, "Review")).clicked() {
                            self.focus_pending_form_edits(tab);
                        }
                    });
                }
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let selected = self.forms.selected.and_then(|(f, w)| {
                            let field = fields.iter().find(|d| d.id == f)?;
                            let widget = field.widgets.iter().find(|x| x.id == w)?;
                            Some((field.clone(), widget.clone()))
                        });
                        let Some((field, widget)) = selected else {
                            self.form_overview(ui, &fields, page, editable, &mut jump);
                            return;
                        };
                        ui.horizontal(|ui| {
                            let (r, _) =
                                ui.allocate_exact_size(vec2(30., 30.), egui::Sense::hover());
                            ui.painter().rect_filled(r, 8., theme::accent_soft());
                            ui::draw_icon(
                                ui.painter(),
                                r.shrink(6.),
                                icon(field.kind),
                                theme::accent(),
                            );
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 1.;
                                ui.label(
                                    RichText::new(&field.name)
                                        .font(ui::strong(ui, 14.))
                                        .color(theme::ink()),
                                );
                                ui.label(
                                    RichText::new(field.kind.label())
                                        .size(11.5)
                                        .color(theme::muted()),
                                );
                            });
                        });
                        let value = values.iter().find(|v| v.id == field.id);
                        ui.add_enabled_ui(editable, |ui| {
                            self.form_value(
                                ui,
                                &field,
                                &widget,
                                value,
                                revision,
                                &mut edit,
                                &mut sign,
                                &mut pick_image,
                            );
                            if !field.signed {
                                self.form_properties(ui, &field, &widget, revision, &mut edit);
                            }
                            ui.horizontal_wrapped(|ui| {
                                if field.kind == FieldType::Radio
                                    && ui
                                        .add(ActionButton::new(Icon::Plus, "Add a button"))
                                        .on_hover_text("Place another choice in this group")
                                        .clicked()
                                {
                                    self.forms.tool = Some(FieldType::Radio);
                                }
                                if !field.signed {
                                    let several = field.widgets.len() > 1;
                                    if several
                                        && ui
                                            .add(
                                                ActionButton::new(
                                                    Icon::Trash,
                                                    "Delete this button",
                                                )
                                                .tint(theme::danger()),
                                            )
                                            .clicked()
                                    {
                                        edit = Some(Edit::RemoveField {
                                            field: field.id,
                                            widget: Some(widget.id),
                                        });
                                    }
                                    if ui
                                        .add(
                                            ActionButton::new(
                                                Icon::Trash,
                                                if several {
                                                    "Delete group"
                                                } else {
                                                    "Delete field"
                                                },
                                            )
                                            .tint(theme::danger()),
                                        )
                                        .on_hover_text("Undo is available")
                                        .clicked()
                                    {
                                        edit = Some(Edit::RemoveField {
                                            field: field.id,
                                            widget: None,
                                        });
                                    }
                                }
                            });
                        });
                        ui.add_space(4.);
                        if ui.link("Show all fields").clicked() {
                            self.forms.selected = None;
                        }
                    });
            });
        if let Some(page) = jump
            && let Some(tab) = self.tabs.get_mut(self.active)
        {
            tab.jump(page);
        }
        if let Some((field, kind)) = pick_image
            && kind == FieldType::Image
            && let Some(path) = self.native_pick(
                rfd::FileDialog::new()
                    .set_title("Choose a picture")
                    .add_filter("Pictures", &["png", "jpg", "jpeg"]),
            )
        {
            match convert::read_bounded(&path, 32 * 1024 * 1024) {
                Ok(image) => edit = Some(Edit::FieldImage { field, image }),
                Err(e) => self.status = Some((format!("{e:#}"), true)),
            }
        }
        if let Some(edit) = edit {
            self.content_change(ctx, edit);
        }
        if let Some((name, page, rect)) = sign {
            self.sign_field(name, page, rect);
        }
    }

    /// No field selected: how to start, and every field in tab order.
    fn form_overview(
        &mut self,
        ui: &mut egui::Ui,
        fields: &[Designed],
        page: usize,
        editable: bool,
        jump: &mut Option<usize>,
    ) {
        if fields.is_empty() {
            ui::tip(
                ui,
                "Build a form",
                "Choose a field type in the toolbar, then drag on the page. Or let MyPDF find the blanks, boxes and lines for you.",
            );
            if ui
                .add_enabled(
                    editable,
                    ActionButton::new(Icon::Sparkle, "Detect fields").primary(),
                )
                .clicked()
            {
                self.forms.pending_edit = Some(Edit::DetectFields);
            }
            return;
        }
        ui.label(
            RichText::new(format!(
                "{} field{} · click one to fill it or change it",
                fields.len(),
                if fields.len() == 1 { "" } else { "s" }
            ))
            .size(12.)
            .color(theme::muted()),
        );
        let mut current = None;
        ui.spacing_mut().item_spacing.y = 2.;
        for field in fields {
            let first = &field.widgets[0];
            if current != Some(first.page) {
                current = Some(first.page);
                ui.add_space(6.);
                ui::caption(ui, &format!("Page {}", first.page + 1));
            }
            let response = ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(22., 22.), egui::Sense::hover());
                ui.painter().text(
                    r.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("{}", first.tab + 1),
                    FontId::proportional(10.5),
                    theme::muted(),
                );
                let (i, _) = ui.allocate_exact_size(vec2(16., 16.), egui::Sense::hover());
                ui::draw_icon(ui.painter(), i, icon(field.kind), theme::accent());
                ui.selectable_label(false, &field.name)
            });
            if response.inner.clicked() {
                self.forms.selected = Some((field.id, first.id));
                if first.page != page {
                    *jump = Some(first.page);
                }
            }
        }
        ui.spacing_mut().item_spacing.y = 10.;
    }

    /// The field's value: typed, chosen, ticked, a picture, or a signature.
    #[allow(clippy::too_many_arguments)]
    fn form_value(
        &mut self,
        ui: &mut egui::Ui,
        field: &Designed,
        widget: &form_design::DesignWidget,
        value: Option<&mypdf_milestone0::forms::Field>,
        revision: u64,
        edit: &mut Option<Edit>,
        sign: &mut Option<(String, usize, [f32; 4])>,
        pick_image: &mut Option<(lopdf::ObjectId, FieldType)>,
    ) {
        let Some(value) = value else { return };
        if field.kind == FieldType::Button {
            return;
        }
        ui::section(ui, "Value", |ui| {
            if let Some(reason) = &value.reason {
                ui::callout(ui, ui::Tone::Info, reason);
                return;
            }
            let current = value.value.clone();
            let tab = self.tabs[self.active].id;
            if self
                .forms
                .value
                .as_ref()
                .is_none_or(|d| d.tab != tab || d.field != field.id || d.revision != revision)
            {
                self.forms.stash_value();
                let saved = self.forms.saved_values.remove(&(tab, field.id));
                self.forms.value = Some(match saved {
                    Some(mut draft) if draft.edited != current => {
                        draft.revision = revision;
                        draft
                    }
                    _ => ValueDraft {
                        tab,
                        field: field.id,
                        revision,
                        original: current.clone(),
                        edited: current.clone(),
                    },
                });
            }
            let mut apply = None;
            match &value.kind {
                FieldKind::Text { multiline, max } => {
                    let text = &mut self.forms.value.as_mut().unwrap().edited;
                    let edit = if *multiline {
                        egui::TextEdit::multiline(text).desired_rows(4)
                    } else {
                        egui::TextEdit::singleline(text)
                    };
                    let r = ui.add(
                        edit.margin(ui::FIELD_MARGIN)
                            .desired_width(f32::INFINITY)
                            .char_limit(max.unwrap_or(32_768))
                            .hint_text("Type the value"),
                    );
                    r.widget_info(|| {
                        let mut info =
                            egui::WidgetInfo::text_edit(true, &current, &*text, "Type the value");
                        info.label = Some(format!("{} value", field.name));
                        info
                    });
                    self.editor.text_editing |= r.has_focus();
                    if !*multiline
                        && r.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        apply = Some(text.clone());
                    }
                }
                FieldKind::Date { format } => {
                    let text = &mut self.forms.value.as_mut().unwrap().edited;
                    let r = ui.add(
                        egui::TextEdit::singleline(text)
                            .margin(ui::FIELD_MARGIN)
                            .desired_width(f32::INFINITY)
                            .hint_text(format.as_str()),
                    );
                    r.widget_info(|| {
                        let mut info = egui::WidgetInfo::text_edit(true, &current, &*text, format);
                        info.label = Some(format!("{} value", field.name));
                        info
                    });
                    self.editor.text_editing |= r.has_focus();
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        apply = Some(text.clone());
                    }
                    let valid =
                        text.trim().is_empty() || form_data::parse_date(format, text).is_ok();
                    if !valid {
                        ui.label(
                            RichText::new(format!("Write the date as {format}"))
                                .size(11.5)
                                .color(theme::danger()),
                        );
                    }
                    if ui
                        .add(ActionButton::new(Icon::FieldDate, "Today"))
                        .clicked()
                    {
                        apply = Some(form_data::format_date(format, today()));
                    }
                }
                FieldKind::Choice { options, editable } => {
                    if field.kind == FieldType::ListBox {
                        egui::Frame::new()
                            .stroke(Stroke::new(1., theme::line()))
                            .corner_radius(6)
                            .inner_margin(4)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                for (v, label) in options {
                                    if ui.selectable_label(*v == current, label).clicked() {
                                        apply = Some(v.clone());
                                    }
                                }
                            });
                    } else {
                        let shown = options
                            .iter()
                            .find(|(v, _)| *v == current)
                            .map_or(current.clone(), |(_, l)| l.clone());
                        egui::ComboBox::from_id_salt(("form_choice", field.id))
                            .selected_text(if shown.is_empty() {
                                "Choose…".into()
                            } else {
                                shown
                            })
                            .width(ui.available_width())
                            .show_ui(ui, |ui| {
                                for (v, label) in options {
                                    if ui.selectable_label(*v == current, label).clicked() {
                                        apply = Some(v.clone());
                                    }
                                }
                            });
                        if *editable {
                            let text = &mut self.forms.value.as_mut().unwrap().edited;
                            let r = ui.add(
                                egui::TextEdit::singleline(text)
                                    .margin(ui::FIELD_MARGIN)
                                    .desired_width(f32::INFINITY)
                                    .hint_text("Or type a value"),
                            );
                            self.editor.text_editing |= r.has_focus();
                            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                apply = Some(text.clone());
                            }
                        }
                    }
                }
                FieldKind::Button { radio, states } => {
                    if *radio {
                        for state in states {
                            if ui.radio(current == *state, state).clicked() {
                                apply = Some(state.clone());
                            }
                        }
                    } else {
                        let on = states.first().cloned().unwrap_or_else(|| "Yes".into());
                        let mut checked = current == on;
                        if ui.checkbox(&mut checked, "Checked").changed() {
                            apply = Some(if checked { on } else { "Off".into() });
                        }
                    }
                }
                FieldKind::Image => {
                    if ui
                        .add(ActionButton::new(Icon::Import, "Choose a picture…").primary())
                        .clicked()
                    {
                        *pick_image = Some((field.id, field.kind));
                    }
                }
                FieldKind::Signature { signed } => {
                    if *signed {
                        ui::callout(
                            ui,
                            ui::Tone::Success,
                            "Signed. A signed field cannot change.",
                        );
                    } else {
                        if ui
                            .add(ActionButton::new(Icon::Sign, "Sign here").primary())
                            .on_hover_text(
                                "Signs with your starred signature and saves a signed copy",
                            )
                            .clicked()
                        {
                            *sign = Some((field.name.clone(), widget.page, widget.rect));
                        }
                        ui.label(
                            RichText::new(
                                "Uses your default signature from Settings → Digital signatures.",
                            )
                            .size(11.5)
                            .color(theme::muted()),
                        );
                    }
                }
                FieldKind::Unsupported => {}
            }
            let typed = matches!(value.kind, FieldKind::Text { .. } | FieldKind::Date { .. })
                || matches!(value.kind, FieldKind::Choice { editable: true, .. } if field.kind == FieldType::Dropdown);
            if typed {
                let draft = self
                    .forms
                    .value
                    .as_ref()
                    .map(|v| v.edited.clone())
                    .unwrap_or_default();
                if draft != current {
                    ui.label(
                        RichText::new("Unapplied value · kept while you switch fields")
                            .small()
                            .color(theme::warning()),
                    );
                }
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            draft != current,
                            ActionButton::new(Icon::Check, "Apply value").primary(),
                        )
                        .clicked()
                    {
                        apply = Some(draft);
                    }
                    if ui
                        .add_enabled(
                            self.forms
                                .value
                                .as_ref()
                                .is_some_and(|d| d.edited != current),
                            ActionButton::new(Icon::Close, "Discard value"),
                        )
                        .clicked()
                        && let Some(draft) = &mut self.forms.value
                    {
                        draft.original = current.clone();
                        draft.edited = current.clone();
                    }
                });
            }
            if let Some(v) = apply
                && v != current
            {
                *edit = Some(Edit::Field {
                    id: field.id,
                    value: v,
                });
            }
        });
    }

    /// The field's settings, applied together.
    fn form_properties(
        &mut self,
        ui: &mut egui::Ui,
        field: &Designed,
        widget: &form_design::DesignWidget,
        revision: u64,
        edit: &mut Option<Edit>,
    ) {
        let tab = self.tabs[self.active].id;
        let fresh = self.forms.draft.as_ref().is_none_or(|d| {
            d.tab != tab || d.field != field.id || d.widget != widget.id || d.revision != revision
        });
        if fresh {
            // Radio buttons share the group's settings; the export value is this button's.
            let mut p = field.properties.clone();
            if field.kind == FieldType::Radio {
                p.export_value = widget.export.clone();
                p.checked = values_selected(field, widget);
            }
            self.forms.stash_properties();
            let saved = self
                .forms
                .saved_properties
                .remove(&(tab, field.id, widget.id));
            self.forms.draft = Some(match saved {
                Some(mut draft) if draft.edited != p => {
                    draft.revision = revision;
                    draft
                }
                _ => Draft {
                    tab,
                    field: field.id,
                    widget: widget.id,
                    revision,
                    kind: field.kind,
                    original: p.clone(),
                    edited: p,
                },
            });
        }
        let draft = self.forms.draft.as_mut().unwrap();
        let kind = draft.kind;
        let p = &mut draft.edited;
        let mut focus = false;
        ui::section(ui, "Properties", |ui| {
            focus |= ui::field(
                ui,
                if kind == FieldType::Radio {
                    "Group name"
                } else {
                    "Name"
                },
                |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut p.name)
                            .margin(ui::FIELD_MARGIN)
                            .desired_width(f32::INFINITY)
                            .char_limit(120),
                    )
                },
            )
            .has_focus();
            focus |= ui::field(ui, "Tooltip", |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut p.tooltip)
                        .margin(ui::FIELD_MARGIN)
                        .desired_width(f32::INFINITY)
                        .hint_text("Shown when people point at the field")
                        .char_limit(500),
                )
            })
            .has_focus();
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut p.required, "Required");
                if kind != FieldType::Signature {
                    ui.checkbox(&mut p.read_only, "Read-only");
                }
            });
            match kind {
                FieldType::Text => {
                    ui.checkbox(&mut p.multiline, "Several lines");
                    ui.horizontal(|ui| {
                        let mut limited = p.max_length.is_some();
                        if ui.checkbox(&mut limited, "Limit to").changed() {
                            p.max_length = limited.then_some(50);
                        }
                        if let Some(max) = &mut p.max_length {
                            ui.add(
                                egui::DragValue::new(max)
                                    .range(1..=32_768)
                                    .suffix(" characters"),
                            );
                        }
                    });
                    focus |= ui::field(ui, "Default text", |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut p.default_value)
                                .margin(ui::FIELD_MARGIN)
                                .desired_width(f32::INFINITY),
                        )
                    })
                    .has_focus();
                }
                FieldType::Date => {
                    ui::field(ui, "Date format", |ui| {
                        egui::ComboBox::from_id_salt("form_date_format")
                            .selected_text(p.date_format.as_str())
                            .width(ui.available_width())
                            .show_ui(ui, |ui| {
                                for format in DATE_FORMATS {
                                    let example = form_data::format_date(format, (2026, 3, 9));
                                    ui.selectable_value(
                                        &mut p.date_format,
                                        format.to_owned(),
                                        format!("{format}   ({example})"),
                                    );
                                }
                            });
                    });
                    focus |= ui::field(ui, "Default date", |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut p.default_value)
                                .margin(ui::FIELD_MARGIN)
                                .desired_width(f32::INFINITY)
                                .hint_text(p.date_format.as_str()),
                        )
                    })
                    .has_focus();
                }
                FieldType::Checkbox | FieldType::Radio => {
                    focus |= ui::field(
                        ui,
                        if kind == FieldType::Radio {
                            "This button's value"
                        } else {
                            "Value when checked"
                        },
                        |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut p.export_value)
                                    .margin(ui::FIELD_MARGIN)
                                    .desired_width(f32::INFINITY)
                                    .char_limit(100),
                            )
                        },
                    )
                    .has_focus();
                    ui.checkbox(
                        &mut p.checked,
                        if kind == FieldType::Radio {
                            "Selected by default"
                        } else {
                            "Checked by default"
                        },
                    );
                    ui::field(ui, "Mark", |ui| {
                        ui.horizontal_wrapped(|ui| {
                            for style in CheckStyle::ALL {
                                ui.selectable_value(&mut p.check_style, style, style.label());
                            }
                        });
                    });
                }
                FieldType::Dropdown | FieldType::ListBox => {
                    ui::field(ui, "Choices", |ui| {
                        let mut remove = None;
                        let mut swap = None;
                        let count = p.options.len();
                        for (i, option) in p.options.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                let r = ui.add(
                                    egui::TextEdit::singleline(option)
                                        .margin(ui::FIELD_MARGIN)
                                        // The field's own margin comes on top of its width.
                                        .desired_width((ui.available_width() - 104.).max(60.)),
                                );
                                focus |= r.has_focus();
                                if ui
                                    .add_enabled(i > 0, egui::Button::new("↑").small())
                                    .on_hover_text("Move up")
                                    .clicked()
                                {
                                    swap = Some(i);
                                }
                                if ui
                                    .add_enabled(
                                        count > 1,
                                        ActionButton::new(Icon::Close, "Remove choice").compact(),
                                    )
                                    .clicked()
                                {
                                    remove = Some(i);
                                }
                            });
                        }
                        if let Some(i) = swap {
                            p.options.swap(i - 1, i);
                        }
                        if let Some(i) = remove {
                            let gone = p.options.remove(i);
                            if p.default_value == gone {
                                p.default_value.clear();
                            }
                        }
                        if ui
                            .add(ActionButton::new(Icon::Plus, "Add choice"))
                            .clicked()
                        {
                            p.options.push(format!("Option {}", p.options.len() + 1));
                        }
                    });
                    ui::field(ui, "Default choice", |ui| {
                        egui::ComboBox::from_id_salt("form_default_choice")
                            .selected_text(if p.default_value.is_empty() {
                                "None".to_owned()
                            } else {
                                p.default_value.clone()
                            })
                            .width(ui.available_width())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut p.default_value, String::new(), "None");
                                for option in p.options.clone() {
                                    ui.selectable_value(
                                        &mut p.default_value,
                                        option.clone(),
                                        option,
                                    );
                                }
                            });
                    });
                    if kind == FieldType::Dropdown {
                        ui.checkbox(&mut p.editable, "People can also type their own value");
                    }
                }
                FieldType::Button => {
                    focus |= ui::field(ui, "Caption", |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut p.caption)
                                .margin(ui::FIELD_MARGIN)
                                .desired_width(f32::INFINITY)
                                .char_limit(200),
                        )
                    })
                    .has_focus();
                    ui::field(ui, "When clicked", |ui| {
                        let label = match &p.action {
                            ButtonAction::None => "Nothing",
                            ButtonAction::ResetForm => "Clear the form",
                            ButtonAction::Print => "Print",
                            ButtonAction::OpenLink(_) => "Open a web page",
                        };
                        egui::ComboBox::from_id_salt("form_button_action")
                            .selected_text(label)
                            .width(ui.available_width())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut p.action, ButtonAction::None, "Nothing");
                                ui.selectable_value(
                                    &mut p.action,
                                    ButtonAction::ResetForm,
                                    "Clear the form",
                                );
                                ui.selectable_value(&mut p.action, ButtonAction::Print, "Print");
                                if ui
                                    .selectable_label(
                                        matches!(p.action, ButtonAction::OpenLink(_)),
                                        "Open a web page",
                                    )
                                    .clicked()
                                {
                                    p.action = ButtonAction::OpenLink("https://".into());
                                }
                            });
                        if let ButtonAction::OpenLink(url) = &mut p.action {
                            focus |= ui
                                .add(
                                    egui::TextEdit::singleline(url)
                                        .margin(ui::FIELD_MARGIN)
                                        .desired_width(f32::INFINITY)
                                        .hint_text("https://example.com"),
                                )
                                .has_focus();
                        }
                    });
                }
                FieldType::Image | FieldType::Signature => {}
            }
            ui.separator();
            ui::caption(ui, "Look");
            if !matches!(kind, FieldType::Image | FieldType::Signature) {
                ui.horizontal(|ui| {
                    let mut auto = p.font_size == 0.;
                    if ui.checkbox(&mut auto, "Auto text size").changed() {
                        p.font_size = if auto { 0. } else { 11. };
                    }
                    if !auto {
                        ui.add(
                            egui::DragValue::new(&mut p.font_size)
                                .range(4.0..=72.0)
                                .speed(0.5)
                                .suffix(" pt"),
                        );
                    }
                });
            }
            if matches!(
                kind,
                FieldType::Text | FieldType::Date | FieldType::Dropdown | FieldType::ListBox
            ) {
                let current = match p.align {
                    Align::Left => 0,
                    Align::Center => 1,
                    Align::Right => 2,
                };
                if let Some(i) = ui::segmented(ui, &["Left", "Center", "Right"], current) {
                    p.align = [Align::Left, Align::Center, Align::Right][i];
                }
            }
            ui.horizontal(|ui| {
                if !matches!(kind, FieldType::Image | FieldType::Signature) {
                    ui::color_swatch(ui, &mut p.text_color, "Text color");
                    ui.label(RichText::new("Text").size(12.).color(theme::muted()));
                    ui.add_space(8.);
                }
                for (label, color, fallback) in [
                    ("Border", &mut p.border, [0.45, 0.52, 0.62]),
                    ("Fill", &mut p.background, [0.95, 0.97, 1.0]),
                ] {
                    let mut on = color.is_some();
                    if ui.checkbox(&mut on, label).changed() {
                        *color = on.then_some(fallback);
                    }
                    if let Some(c) = color {
                        ui::color_swatch(ui, c, label);
                    }
                }
            });
            let dirty = p != &draft.original;
            if dirty {
                ui.label(
                    RichText::new("Unapplied properties · kept while you switch fields")
                        .small()
                        .color(theme::warning()),
                );
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        dirty,
                        ActionButton::new(Icon::Check, "Apply changes").primary(),
                    )
                    .clicked()
                {
                    *edit = Some(Edit::UpdateField {
                        field: field.id,
                        widget: widget.id,
                        properties: p.clone(),
                    });
                }
                if dirty
                    && ui
                        .add(ActionButton::new(Icon::Close, "Discard changes"))
                        .clicked()
                {
                    *p = draft.original.clone();
                }
            });
        });
        self.editor.text_editing |= focus;
    }
}
/// Whether this radio button is the group's choice.
fn values_selected(field: &Designed, widget: &form_design::DesignWidget) -> bool {
    !widget.export.is_empty() && field.value == widget.export
}
/// Today's date where the computer is.
fn today() -> (i32, u32, u32) {
    let now = time::OffsetDateTime::now_utc();
    #[cfg(target_os = "macos")]
    let now = {
        use core_foundation::{date::CFDate, timezone::CFTimeZone};
        let offset = CFTimeZone::system().seconds_from_gmt(CFDate::now());
        now + time::Duration::seconds(offset as i64)
    };
    (now.year(), now.month() as u32, now.day() as u32)
}

impl Viewer {
    /// Set up a layout, then place it on the page.
    fn layout_dialog(&mut self, ctx: &egui::Context) {
        let Some(layout) = &mut self.forms.layout_dialog else {
            return;
        };
        let mut open = true;
        let mut place = false;
        let mut cancel = false;
        let mut focus = false;
        egui::Window::new(format!("Add {}", layout.title().to_lowercase()))
            .id(egui::Id::new("form_layout_dialog"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(ui::dialog_width(ctx, 440.))
            .anchor(egui::Align2::CENTER_CENTER, vec2(0., 0.))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 10.;
                ui::dialog_body(ui, "form_layout_body", 480., |ui| {
                let lines = |ui: &mut egui::Ui, list: &mut Vec<String>, hint: &str, focus: &mut bool| {
                    let mut text = list.join("\n");
                    let r = ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .margin(ui::FIELD_MARGIN)
                            .desired_rows(5)
                            .desired_width(f32::INFINITY)
                            .hint_text(hint),
                    );
                    *focus |= r.has_focus();
                    if r.changed() {
                        *list = text.split('\n').map(String::from).collect();
                    }
                };
                match layout {
                    Layout::Table {
                        columns,
                        rows,
                        header,
                        lines: grid,
                    } => {
                        ui::field(ui, "Columns", |ui| {
                            let mut remove = None;
                            let count = columns.len();
                            for (i, column) in columns.iter_mut().enumerate() {
                                ui.horizontal(|ui| {
                                    focus |= ui
                                        .add(
                                            egui::TextEdit::singleline(&mut column.title)
                                                .margin(ui::FIELD_MARGIN)
                                                .desired_width(130.)
                                                .hint_text("Title"),
                                        )
                                        .has_focus();
                                    egui::ComboBox::from_id_salt(("layout_column_kind", i))
                                        .selected_text(column.kind.label())
                                        .width(100.)
                                        .show_ui(ui, |ui| {
                                            for kind in CellKind::ALL {
                                                ui.selectable_value(&mut column.kind, kind, kind.label());
                                            }
                                        });
                                    ui.add(
                                        egui::DragValue::new(&mut column.width)
                                            .range(0.5..=5.0)
                                            .speed(0.05)
                                            .prefix("width ")
                                            .max_decimals(1),
                                    )
                                    .on_hover_text("Relative to the other columns");
                                    if ui
                                        .add_enabled(count > 1, ActionButton::new(Icon::Close, "Remove column").compact())
                                        .clicked()
                                    {
                                        remove = Some(i);
                                    }
                                });
                                if column.kind == CellKind::Dropdown {
                                    focus |= ui
                                        .add(
                                            egui::TextEdit::singleline(&mut column.choices)
                                                .margin(ui::FIELD_MARGIN)
                                                .desired_width(f32::INFINITY)
                                                .hint_text("Choices, separated by commas"),
                                        )
                                        .has_focus();
                                }
                            }
                            if let Some(i) = remove {
                                columns.remove(i);
                            }
                            if columns.len() < 12
                                && ui.add(ActionButton::new(Icon::Plus, "Add column")).clicked()
                            {
                                columns.push(Column::new(&format!("Column {}", columns.len() + 1), CellKind::Text, 1.));
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Rows").color(theme::muted()));
                            ui.add(egui::DragValue::new(rows).range(1..=60));
                            ui.add_space(12.);
                            ui.checkbox(header, "Header row");
                            ui.checkbox(grid, "Grid lines");
                        });
                    }
                    Layout::Labeled { labels } => {
                        ui::field(ui, "Labels, one per line", |ui| {
                            lines(ui, labels, "Full name\nAddress\nPhone", &mut focus)
                        });
                        ui.label(
                            RichText::new("A label with “date” gets a date field, one with “signature” a signature field.")
                                .size(11.5)
                                .color(theme::muted()),
                        );
                    }
                    Layout::Checklist { items } => {
                        ui::field(ui, "Items, one per line", |ui| lines(ui, items, "First item", &mut focus));
                    }
                    Layout::Choice {
                        question,
                        options,
                        across,
                    } => {
                        focus |= ui::field(ui, "Question", |ui| {
                            ui.add(egui::TextEdit::singleline(question).margin(ui::FIELD_MARGIN).desired_width(f32::INFINITY))
                        })
                        .has_focus();
                        ui::field(ui, "Answers, one per line", |ui| lines(ui, options, "Yes\nNo", &mut focus));
                        let current = if *across { 0 } else { 1 };
                        if let Some(i) = ui::segmented(ui, &["Side by side", "One below another"], current) {
                            *across = i == 0;
                        }
                    }
                    Layout::Signatures { signers } => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Signers").color(theme::muted()));
                            ui.add(egui::DragValue::new(signers).range(1..=8));
                        });
                        ui.label(
                            RichText::new("Each signer gets a name, a date and a signature field.")
                                .size(11.5)
                                .color(theme::muted()),
                        );
                    }
                }
                // A small picture of the layout at its standard size.
                let [w, h] = layout.default_size();
                match layout.plan([0., 0., w, h]) {
                    Ok(plan) => {
                        let scale = (ui.available_width() / w).min(180. / h).min(1.);
                        let (area, _) = ui.allocate_exact_size(vec2(w, h) * scale, egui::Sense::hover());
                        ui.painter().rect_filled(area.expand(6.), 6, Color32::WHITE);
                        paint_plan(ui.painter(), &plan, |p| area.min + vec2(p[0], p[1]) * scale, scale);
                    }
                    Err(e) => ui::callout(ui, ui::Tone::Warning, &format!("{e:#}")),
                }
                });
                let [w, h] = layout.default_size();
                ui::dialog_footer(ui, |ui| {
                    place = ui
                        .add_enabled(layout.plan([0., 0., w, h]).is_ok(), ActionButton::new(Icon::Plus, "Place on page").primary())
                        .on_hover_text("Then drag the area on the page, or click to use this size")
                        .clicked();
                    cancel = ui.add(ActionButton::new(Icon::Close, "Cancel")).clicked();
                });
            });
        cancel |= ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        self.editor.text_editing |= focus;
        if place && let Some(layout) = self.forms.layout_dialog.take() {
            self.forms
                .recent_layouts
                .retain(|l| l.title() != layout.title());
            self.forms.recent_layouts.push(layout.clone());
            self.status = Some((
                format!(
                    "Drag the area for the {} on the page, or click to place it · Esc cancels",
                    layout.title().to_lowercase()
                ),
                false,
            ));
            self.forms.layout = Some(layout);
            self.forms.tool = None;
            self.forms.selected = None;
        } else if !open || cancel {
            self.forms.layout_dialog = None;
        }
    }
}

#[cfg(test)]
impl Viewer {
    /// Visual QA: one field of every kind on the first tab, partly filled.
    pub(super) fn preview_form(&mut self) {
        let s = self.tabs[0].session.as_mut().unwrap();
        let mut bytes = s.source.to_vec();
        let mut add =
            |kind: FieldType, name: &str, rect: [f32; 4], change: &dyn Fn(&mut Properties)| {
                let mut p = Properties::new(kind, name);
                change(&mut p);
                bytes = form_design::add(&bytes, 0, rect, kind, &p).unwrap().0;
            };
        add(
            FieldType::Text,
            "Full name",
            [70., 430., 330., 452.],
            &|p| p.default_value = "Jane Appleseed".into(),
        );
        add(FieldType::Date, "Date", [350., 430., 470., 452.], &|p| {
            p.default_value = "15.10.2026".into()
        });
        add(
            FieldType::Checkbox,
            "I agree",
            [70., 470., 84., 484.],
            &|p| p.checked = true,
        );
        add(FieldType::Radio, "Plan", [70., 500., 84., 514.], &|p| {
            p.checked = true
        });
        add(FieldType::Radio, "Plan", [150., 500., 164., 514.], &|_| {});
        add(
            FieldType::Dropdown,
            "Country",
            [70., 540., 250., 562.],
            &|p| {
                p.options = vec!["Germany".into(), "Türkiye".into(), "United Kingdom".into()];
                p.default_value = "Türkiye".into();
            },
        );
        add(
            FieldType::ListBox,
            "Interests",
            [270., 540., 450., 604.],
            &|p| {
                p.options = vec![
                    "Design".into(),
                    "Engineering".into(),
                    "Finance".into(),
                    "Legal".into(),
                ];
                p.default_value = "Engineering".into();
            },
        );
        add(FieldType::Button, "Clear", [70., 620., 170., 644.], &|p| {
            p.caption = "Clear form".into();
            p.action = ButtonAction::ResetForm;
        });
        add(FieldType::Image, "Photo", [470., 540., 560., 610.], &|_| {});
        add(
            FieldType::Signature,
            "Signature",
            [300., 640., 520., 700.],
            &|_| {},
        );
        s.commit_document(bytes).unwrap();
        self.editor.mode = editing::Mode::Forms;
        self.inspector = true;
    }
    /// Visual QA: whether a dragged field's picture of the page without it is ready.
    pub(super) fn ghost_ready(&self) -> bool {
        self.forms
            .ghost
            .as_ref()
            .is_some_and(|g| g.erased.is_some())
    }
    /// Visual QA: forget a drag in progress, so releasing the mouse changes nothing.
    pub(super) fn preview_cancel_drag(&mut self) {
        self.forms.grab = None;
        self.forms.ghost = None;
    }
    /// Visual QA: select a field by name once the page is inspected.
    pub(super) fn preview_form_select(&mut self, name: &str) {
        let fields = self
            .fresh_inspection()
            .unwrap()
            .designed
            .as_ref()
            .unwrap()
            .clone();
        let field = fields.iter().find(|f| f.name == name).unwrap();
        self.forms.selected = Some((field.id, field.widgets[0].id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;

    #[test]
    fn pending_form_values_and_properties_survive_field_and_document_switches() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let mut bytes = convert::blank(595., 842.).unwrap();
        for (name, top) in [("First", 100.), ("Second", 160.)] {
            bytes = form_design::add(
                &bytes,
                0,
                [80., top, 280., top + 24.],
                FieldType::Text,
                &Properties::new(FieldType::Text, name),
            )
            .unwrap()
            .0;
        }
        app.open_memory(PathBuf::from("Drafts A.pdf"), bytes.clone(), &ctx);
        app.editor.mode = editing::Mode::Forms;
        let frame = |app: &mut Viewer| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 900.))),
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
        };
        let settle = |app: &mut Viewer| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while app.fresh_inspection().is_none() {
                assert!(std::time::Instant::now() < until);
                frame(app);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            for _ in 0..3 {
                frame(app);
            }
        };
        settle(&mut app);
        let first_tab = app.tabs[0].id;
        let fields = app.fresh_inspection().unwrap().designed.clone().unwrap();
        let first = fields.iter().find(|f| f.name == "First").unwrap();
        let second = fields.iter().find(|f| f.name == "Second").unwrap();
        app.forms.selected = Some((first.id, first.widgets[0].id));
        frame(&mut app);
        app.forms.value.as_mut().unwrap().edited = "Keep this typed value".into();
        app.forms.draft.as_mut().unwrap().edited.tooltip = "Keep these properties".into();
        assert!(app.pending_form_edits(first_tab));
        let before_save = app.tabs[0].session.as_ref().unwrap().source.clone();
        app.save_dialog(&ctx);
        assert!(!app.saving, "save must not omit unapplied inspector edits");
        assert_eq!(app.tabs[0].session.as_ref().unwrap().source, before_save);
        assert!(app.pending_form_edits(first_tab));
        app.request_close(Some(first_tab), &ctx);
        assert!(
            app.close_intent.is_none(),
            "close must first resolve inspector drafts"
        );
        assert_eq!(app.tabs.len(), 1);
        assert!(
            app.status
                .as_ref()
                .is_some_and(|(message, _)| message.contains("form"))
        );
        frame(&mut app);
        app.forms.selected = Some((second.id, second.widgets[0].id));
        frame(&mut app);
        assert_eq!(app.forms.saved_values.len(), 1);
        assert_eq!(app.forms.saved_properties.len(), 1);
        assert!(app.forms.value.as_ref().unwrap().edited.is_empty());
        app.forms.selected = Some((first.id, first.widgets[0].id));
        frame(&mut app);
        assert_eq!(
            app.forms.value.as_ref().unwrap().edited,
            "Keep this typed value"
        );
        assert_eq!(
            app.forms.draft.as_ref().unwrap().edited.tooltip,
            "Keep these properties"
        );

        // Object IDs/revisions can be identical in another PDF. They must not
        // cause the first document's inspector draft to appear in the second.
        app.open_memory(PathBuf::from("Drafts B.pdf"), bytes, &ctx);
        settle(&mut app);
        app.forms.selected = Some((first.id, first.widgets[0].id));
        frame(&mut app);
        assert!(app.forms.value.as_ref().unwrap().edited.is_empty());
        assert!(app.forms.draft.as_ref().unwrap().edited.tooltip.is_empty());
        assert!(app.pending_form_edits(first_tab));
        assert!(!app.pending_form_edits(app.tabs[1].id));
        assert!(app.focus_pending_form_edits(first_tab));
        settle(&mut app);
        assert_eq!(app.forms.selected, Some((first.id, first.widgets[0].id)));
        assert_eq!(
            app.forms.value.as_ref().unwrap().edited,
            "Keep this typed value"
        );
        app.clear_form_drafts(first_tab);
        assert!(!app.pending_form_edits(first_tab));
    }

    #[test]
    fn rapid_form_drag_keeps_its_drawn_size() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.open_memory(
            PathBuf::from("Fast form.pdf"),
            convert::blank(595., 842.).unwrap(),
            &ctx,
        );
        app.editor.mode = editing::Mode::Forms;
        app.tabs[0].zoom = Zoom::Page;
        let frame = |app: &mut Viewer, events| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 900.))),
                    events,
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
        };
        let settle = |app: &mut Viewer| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
            loop {
                frame(app, vec![]);
                if !app.content.busy
                    && app.fresh_inspection().is_some()
                    && app.forms.pending_edit.is_none()
                {
                    break;
                }
                assert!(std::time::Instant::now() < until);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            for _ in 0..3 {
                frame(app, vec![]);
            }
        };
        settle(&mut app);
        let page = ctx
            .read_response(egui::Id::new((app.tabs[0].id, 0_usize, "form_page")))
            .unwrap()
            .rect;
        let from = page.min + vec2(100., 100.) * app.tabs[0].scale;
        let to = page.min + vec2(330., 140.) * app.tabs[0].scale;
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: egui::Modifiers::NONE,
        };
        app.forms.tool = Some(FieldType::Text);
        // An input backend can deliver the last movement and release together.
        frame(&mut app, vec![egui::Event::PointerMoved(from)]);
        frame(&mut app, vec![button(from, true)]);
        frame(
            &mut app,
            vec![egui::Event::PointerMoved(to), button(to, false)],
        );
        settle(&mut app);
        let fields = app.fresh_inspection().unwrap().designed.as_ref().unwrap();
        assert_eq!(fields.len(), 1, "a quick drag creates its field");
        for (a, b) in fields[0].widgets[0]
            .rect
            .iter()
            .zip([100., 100., 330., 140.])
        {
            assert!((a - b).abs() < 1.5, "{:?}", fields[0].widgets[0].rect);
        }
        // The canvas control is keyboard-focusable; moving it does not require
        // pointer precision and does not interpret the arrow as text input.
        let before_nudge = fields[0].widgets[0].rect[0];
        let id = egui::Id::new((
            app.tabs[0].id,
            0_usize,
            "form_widget",
            fields[0].widgets[0].id,
        ));
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&mut app, vec![]);
        frame(
            &mut app,
            vec![egui::Event::Key {
                key: egui::Key::ArrowRight,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        settle(&mut app);
        let fields = app.fresh_inspection().unwrap().designed.as_ref().unwrap();
        assert!((fields[0].widgets[0].rect[0] - before_nudge - 1.).abs() < 0.1);
    }

    #[test]
    fn fields_are_drawn_placed_moved_and_deleted_on_the_page() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.open_memory(
            PathBuf::from("Form.pdf"),
            convert::blank(595., 842.).unwrap(),
            &ctx,
        );
        app.editor.mode = editing::Mode::Forms;
        app.tabs[0].zoom = Zoom::Page;
        let frame = |app: &mut Viewer, events: Vec<egui::Event>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 900.))),
                    events,
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
        };
        // Until the document is unchanged and inspected.
        let settle = |app: &mut Viewer| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            loop {
                frame(app, vec![]);
                if !app.content.busy
                    && app.fresh_inspection().is_some()
                    && app.forms.pending_edit.is_none()
                {
                    break;
                }
                assert!(std::time::Instant::now() < deadline, "{:?}", app.status);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            for _ in 0..3 {
                frame(app, vec![]);
            }
        };
        settle(&mut app);
        let tab = app.tabs[0].id;
        let page_rect = ctx
            .read_response(egui::Id::new((tab, 0_usize, "form_page")))
            .expect("the page is shown")
            .rect;
        let scale = app.tabs[0].scale;
        let at = |p: [f32; 2]| page_rect.min + vec2(p[0], p[1]) * scale;
        let drag = |app: &mut Viewer, from: egui::Pos2, to: egui::Pos2| {
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame(app, vec![egui::Event::PointerMoved(from)]);
            frame(app, vec![button(from, true)]);
            for i in 1..=6 {
                frame(
                    app,
                    vec![egui::Event::PointerMoved(
                        from + (to - from) * i as f32 / 6.,
                    )],
                );
            }
            frame(app, vec![button(to, false)]);
        };
        let fields = |app: &Viewer| app.fresh_inspection().unwrap().designed.clone().unwrap();

        // Draw a text field with its tool.
        app.forms.tool = Some(FieldType::Text);
        drag(&mut app, at([100., 100.]), at([300., 124.]));
        settle(&mut app);
        let all = fields(&app);
        assert_eq!(all.len(), 1, "{:?}", app.status);
        assert_eq!(all[0].name, "Text1");
        for (a, b) in all[0].widgets[0].rect.iter().zip([100., 100., 300., 124.]) {
            assert!((a - b).abs() < 1.5, "{:?}", all[0].widgets[0].rect);
        }
        assert_eq!(
            app.forms.selected.map(|s| s.0),
            Some(all[0].id),
            "the new field is selected"
        );
        assert!(app.forms.tool.is_none());

        // A click places a check box of the standard size, centred on the click.
        app.forms.tool = Some(FieldType::Checkbox);
        let click = at([200., 300.]);
        drag(&mut app, click, click);
        settle(&mut app);
        let check = fields(&app)
            .into_iter()
            .find(|f| f.kind == FieldType::Checkbox)
            .unwrap();
        let r = check.widgets[0].rect;
        assert!(
            (r[2] - r[0] - 14.).abs() < 0.1 && ((r[0] + r[2]) / 2. - 200.).abs() < 1.5,
            "{r:?}"
        );

        // Drag it somewhere else: the field itself is drawn where it goes, and the
        // page never goes blank while it is redrawn.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !app.tabs[0]
            .rasters
            .keys()
            .any(|k| k.page == 0 && !k.thumbnail)
        {
            assert!(std::time::Instant::now() < deadline);
            frame(&mut app, vec![]);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        drag(&mut app, at([200., 300.]), at([260., 340.]));
        let ghost = app
            .forms
            .ghost
            .as_ref()
            .expect("the dropped field is still drawn");
        assert!(ghost.dropped.is_some() && ((ghost.to[0] + ghost.to[2]) / 2. - 260.).abs() < 2.);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while app.forms.ghost.is_some() {
            assert!(std::time::Instant::now() < deadline);
            frame(&mut app, vec![]);
            let shown = app.tabs[0]
                .rasters
                .keys()
                .any(|k| k.page == 0 && !k.thumbnail)
                || app.tabs[0].stale.contains_key(&0);
            assert!(shown, "an image of the page is always shown");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(app.tabs[0].session.as_ref().unwrap().revision() > 0);
        settle(&mut app);
        let moved = fields(&app)
            .into_iter()
            .find(|f| f.kind == FieldType::Checkbox)
            .unwrap();
        let m = moved.widgets[0].rect;
        assert!(
            ((m[0] + m[2]) / 2. - 260.).abs() < 2. && ((m[1] + m[3]) / 2. - 340.).abs() < 2.,
            "{m:?}"
        );

        // Delete removes the selected field; Undo is the usual one.
        assert_eq!(app.forms.selected.map(|s| s.0), Some(moved.id));
        frame(
            &mut app,
            vec![egui::Event::Key {
                key: egui::Key::Delete,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        settle(&mut app);
        let left = fields(&app);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].name, "Text1");

        // A table is placed by dragging its area: the layout's fields follow the drag.
        app.forms.layout = Some(Layout::table());
        drag(&mut app, at([60., 400.]), at([540., 532.]));
        settle(&mut app);
        let all = fields(&app);
        assert_eq!(all.len(), 21, "{:?}", app.status);
        let first = all.iter().find(|f| f.name == "Item 1").unwrap();
        assert!(
            (first.widgets[0].rect[0] - 61.5).abs() < 1.5,
            "{:?}",
            first.widgets[0].rect
        );
        let last = all.iter().find(|f| f.name == "Done 5").unwrap();
        assert!(last.widgets[0].rect[3] < 533., "{:?}", last.widgets[0].rect);
        assert!(app.forms.layout.is_none());
    }
}
