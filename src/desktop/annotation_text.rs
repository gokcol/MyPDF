//! On-page editing of managed text boxes, without entering PDF Edit mode.
use super::*;
use editing::{EditorState, Mode, Tool};
use mypdf_milestone0::annotations::{Annotation, Kind, Style};

pub(super) struct State {
    tab: u64,
    annotation: String,
    focus: bool,
    select_all: bool,
    revision: Option<u64>,
}

// TextEdit cannot rotate or mirror its caret and glyphs. Keep transformed
// boxes in their original appearance while the inspector edits their text.
fn supports(a: &Annotation) -> bool {
    a.rotation.rem_euclid(360.) == 0. && !a.flip_x && !a.flip_y
}

pub(super) fn start(editor: &mut EditorState, tab: u64, a: &Annotation, select_all: bool) {
    if !matches!(editor.mode, Mode::Annotate | Mode::Draw)
        || !matches!(a.kind, Kind::Text { .. })
        || a.object.locked
        || a.object.hidden
        || !supports(a)
    {
        return;
    }
    editor.annotation_text = Some(State {
        tab,
        annotation: a.id.clone(),
        focus: true,
        select_all,
        revision: None,
    });
    editor.draft = Some(a.clone());
}

fn active<'a>(editor: &'a EditorState, tab: &Tab, page: usize) -> Option<&'a Annotation> {
    let state = editor.annotation_text.as_ref()?;
    let a = editor.draft.as_ref()?;
    (state.tab == tab.id
        && state.annotation == a.id
        && tab.selected_annotation.as_ref() == Some(&a.id)
        && tab.group.is_empty()
        && a.page == page
        && !a.object.locked
        && !a.object.hidden
        && supports(a)
        && matches!(a.kind, Kind::Text { .. })
        && matches!(editor.mode, Mode::Annotate | Mode::Draw)
        && editor.tool == Tool::Select
        && !editor.library_open
        && tab
            .session
            .as_ref()
            .is_some_and(|s| s.editable && s.annotations.iter().any(|mark| mark.id == a.id)))
    .then_some(a)
}

pub(super) fn sync(editor: &mut EditorState, tab: &Tab) {
    if editor.annotation_text.is_some()
        && editor
            .draft
            .as_ref()
            .is_none_or(|a| active(editor, tab, a.page).is_none())
    {
        editor.annotation_text = None;
    }
}

pub(super) fn owns_pointer(
    editor: &EditorState,
    tab: &Tab,
    page: usize,
    ui: &egui::Ui,
    page_rect: Rect,
    scale: f32,
) -> bool {
    let Some(a) = active(editor, tab, page) else {
        return false;
    };
    let bounds = a.bounds();
    let rect = Rect::from_min_max(
        page_rect.min + vec2(bounds[0], bounds[1]) * scale,
        page_rect.min + vec2(bounds[2], bounds[3]) * scale,
    )
    .shrink(8.);
    ui.input(|i| i.pointer.interact_pos().is_some_and(|p| rect.contains(p)))
}

pub(super) fn hides_text(editor: &EditorState, a: &Annotation) -> bool {
    matches!(editor.mode, Mode::Annotate | Mode::Draw)
        && editor.tool == Tool::Select
        && !editor.library_open
        && editor
            .annotation_text
            .as_ref()
            .is_some_and(|s| s.annotation == a.id)
        && editor.draft.as_ref().is_some_and(|d| d.id == a.id)
        && !a.object.locked
        && !a.object.hidden
        && supports(a)
}

/// Shares the inspector draft and commits changes immediately, even with the
/// inspector hidden. No opaque mask is painted over the underlying PDF.
#[allow(clippy::too_many_arguments)]
pub(super) fn overlay(
    editor: &mut EditorState,
    tab: &mut Tab,
    ui: &mut egui::Ui,
    page: usize,
    page_rect: Rect,
    scale: f32,
    status: &mut Option<(String, bool)>,
) {
    let Some(a) = active(editor, tab, page).cloned() else {
        return;
    };
    let bounds = a.bounds();
    let rect = Rect::from_min_max(
        page_rect.min + vec2(bounds[0], bounds[1]) * scale,
        page_rect.min + vec2(bounds[2], bounds[3]) * scale,
    );
    // Match the saved text's four-point inset and preserve the corner handles.
    let resolution = annotation_render::text_resolution(scale);
    let to_global = egui::emath::TSTransform::from_translation(page_rect.min.to_vec2())
        * egui::emath::TSTransform::from_scaling(scale / resolution);
    let text_rect = Rect::from_min_max(
        pos2(bounds[0] + 4., bounds[1] + 4.) * resolution,
        pos2(bounds[2] - 4., bounds[3] - 4.) * resolution,
    );
    let id = egui::Id::new((tab.id, &a.id, "inline_annotation_text"));
    let finish = ui.ctx().memory(|m| m.has_focus(id))
        && ui.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)
        });
    if finish {
        ui.memory_mut(|m| m.surrender_focus(id));
        editor.annotation_text = None;
        return;
    }
    let Some(Kind::Text { text }) = editor.draft.as_mut().map(|a| &mut a.kind) else {
        return;
    };
    let state = editor.annotation_text.as_mut().unwrap();
    // Lay out the editor at the same document resolution as static text.
    // A transformed sublayer scales both painting and pointer/caret geometry;
    // tiny font hinting must not rewrap the selected box when zooming out.
    let layer = egui::LayerId::new(ui.layer_id().order, id.with("document_text_layer"));
    ui.ctx().set_sublayer(ui.layer_id(), layer);
    ui.ctx().set_transform_layer(layer, to_global);
    let clip = to_global.inverse() * rect.intersect(page_rect).intersect(ui.clip_rect());
    let changed = {
        // The page is already allocated. Reserving this child in its parent's
        // flow would add unscaled document coordinates to the scroll extent.
        let mut inline_ui =
            ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(text_rect));
        let ui = &mut inline_ui;
        ui.set_clip_rect(clip);
        let mut output = egui::TextEdit::multiline(text)
            .id(id)
            .font(FontId::new(
                a.style.font_size * resolution,
                egui::FontFamily::Name("annotation".into()),
            ))
            .text_color(editing::color(a.style.color, a.style.opacity))
            .frame(false)
            .margin(0.)
            .desired_rows(1)
            .desired_width(text_rect.width().max(1.))
            .min_size(text_rect.size().max(vec2(1., 1.)))
            .char_limit(8000)
            .show(ui);
        output.response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Text box on page")
        });
        if state.focus {
            output.response.request_focus();
            if state.select_all {
                output
                    .state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(text.chars().count()),
                    )));
                output.state.store(ui.ctx(), id);
            }
            state.focus = false;
        }
        editor.text_editing |= output.response.has_focus();
        output.response.changed()
    };
    if changed && let Some(session) = &mut tab.session {
        let mut entries = session.annotations.clone();
        if let Some(target) = entries.iter_mut().find(|item| item.id == a.id) {
            *target = editor.draft.as_ref().unwrap().clone();
            match session.commit_live(entries, state.revision) {
                Ok(()) => {
                    state.revision = Some(session.revision());
                    *status = None;
                }
                Err(error) => {
                    *status = Some((format!("Could not update text box: {error:#}"), true))
                }
            }
        }
    }
}

/// Presets and custom color share the existing Fill row.
pub(super) fn fill_controls(ui: &mut egui::Ui, style: &mut Style) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.;
        let mut enabled = style.fill.is_some();
        if ui.checkbox(&mut enabled, "Fill").changed() {
            style.fill = enabled.then_some([1., 1., 1.]);
        }
        for (name, rgb) in [
            ("White", [1., 1., 1.]),
            ("Light yellow", [1., 0.96, 0.75]),
            ("Light blue", [0.83, 0.91, 1.]),
            ("Mint", [0.83, 0.96, 0.89]),
        ] {
            let (r, response) = ui.allocate_exact_size(vec2(22., 26.), egui::Sense::click());
            let selected = style.fill == Some(rgb);
            ui.painter()
                .circle_filled(r.center(), 7.5, editing::color(rgb, 1.));
            ui.painter()
                .circle_stroke(r.center(), 7.5, Stroke::new(1., theme::line()));
            if selected {
                ui.painter()
                    .circle_stroke(r.center(), 10., Stroke::new(1.5, theme::accent()));
            }
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Button,
                    ui.is_enabled(),
                    selected,
                    format!("{name} fill"),
                )
            });
            if response
                .on_hover_text(format!("{name} background"))
                .clicked()
            {
                style.fill = Some(rgb);
            }
        }
        let mut custom = style.fill.unwrap_or([1., 1., 1.]);
        if ui::color_swatch_sized(ui, &mut custom, "Custom fill color", vec2(22., 26.)) {
            style.fill = Some(custom);
        }
    });
}

#[cfg(test)]
#[path = "annotation_text_tests.rs"]
mod tests;
