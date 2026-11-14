//! Stamps keep their user-facing identity even when their artwork is SVG.
use super::*;
use mypdf_milestone0::{
    annotations::{Annotation, Kind, Style},
    assets::Asset,
    svg,
};

pub(super) fn is_stamp(editor: &editing::EditorState, a: &Annotation) -> bool {
    a.object.stamp
        || matches!(a.kind, Kind::Stamp { .. })
        || a.asset_id.as_ref().is_some_and(|id| {
            editor
                .builtins
                .iter()
                .any(|b| &b.asset.id == id && b.asset.stamp)
                || editor
                    .library
                    .as_ref()
                    .is_some_and(|l| l.entries.iter().any(|asset| &asset.id == id && asset.stamp))
        })
}

pub(super) fn text_controls(ui: &mut egui::Ui, kind: &mut Kind) -> Option<egui::Id> {
    let Kind::Svg { source, .. } = kind else {
        return None;
    };
    let Ok(fields) = svg::text_fields(source) else {
        return None;
    };
    if fields.is_empty() {
        ui.label(
            RichText::new(
                "This stamp has no editable text. Use Edit artwork to change its design.",
            )
            .small()
            .color(theme::muted()),
        );
        return None;
    }
    let mut focus = None;
    for (index, (_, mut value)) in fields.iter().cloned().enumerate() {
        let label = if fields.len() == 1 {
            "Stamp text".to_owned()
        } else {
            format!("Stamp text {}", index + 1)
        };
        ui::caption(ui, &label);
        let response = ui.add(
            egui::TextEdit::singleline(&mut value)
                .id(ui.id().with(("stamp_text", index)))
                .margin(ui::FIELD_MARGIN)
                .desired_width(f32::INFINITY)
                .char_limit(8000),
        );
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &label));
        if response.has_focus() {
            focus = Some(response.id);
        }
        if response.changed() {
            match svg::replace_text_field(source, index, &value) {
                Ok(next) => *source = next,
                Err(error) => {
                    ui.colored_label(theme::danger(), error.to_string());
                }
            }
        }
    }
    focus
}

pub(super) fn colors(ui: &mut egui::Ui, kind: &mut Kind, style: &mut Style) {
    if let Kind::Svg { tint, .. } = kind {
        ui::caption(ui, "Stamp color");
        let mut original = !*tint;
        if ui.checkbox(&mut original, "Original colors").changed() {
            *tint = !original;
        }
        if editing::color_palette(ui, &mut style.color) {
            *tint = true;
        }
    }
}

pub(super) fn preview(ui: &mut egui::Ui, editor: &mut editing::EditorState, asset: &Asset) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 100.), egui::Sense::hover());
    ui.painter().rect_filled(rect, 8., theme::control());
    editing::paint_asset(editor, ui.ctx(), ui.painter(), rect.shrink(14.), asset);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stamp_identity_survives_without_the_original_library() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let asset = app
            .editor
            .builtins
            .iter()
            .find(|b| b.asset.stamp)
            .unwrap()
            .asset
            .clone();
        let mut old = Annotation::new(0, [20., 20., 180., 80.], asset.kind, asset.style);
        old.asset_id = Some(asset.id);
        assert!(is_stamp(&app.editor, &old));
        old.object.stamp = true;
        old.asset_id = None;
        app.editor.builtins.clear();
        assert!(is_stamp(&app.editor, &old));
        let bytes = mypdf_milestone0::convert::blank(400., 400.).unwrap();
        let saved = mypdf_milestone0::annotations::export(&bytes, &[old.clone()]).unwrap();
        let loaded =
            mypdf_milestone0::annotations::load(&lopdf::Document::load_mem(&saved).unwrap());
        assert_eq!(loaded, vec![old]);
        assert!(is_stamp(&app.editor, &loaded[0]));
    }
}
