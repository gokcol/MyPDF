//! Callout text stays independent from its leader and text-box border.
use super::*;

pub(super) fn controls(ui: &mut egui::Ui, annotation: &mut Annotation) -> Option<egui::Id> {
    let Kind::Callout { text_color, .. } = &mut annotation.kind else {
        return None;
    };
    ui.push_id("callout_text", |ui| {
        ui.spacing_mut().item_spacing.y = 4.;
        ui.label(RichText::new("Text color").size(12.).color(theme::muted()));
        color_palette(ui, text_color);
        property_slider(
            ui,
            "Text size",
            &mut annotation.style.font_size,
            6.0..=72.0,
            " pt",
        )
    })
    .inner
}
