//! Character formatting shared by new and existing on-page text editors.
use super::*;
use mypdf_milestone0::rich_text::{self, Span};
use std::ops::Range;

pub(super) fn span(font: usize, size: f32, color: [f32; 3], len: usize) -> Span {
    Span {
        range: 0..len,
        font: text_style::fonts().get(font).map(|f| f.path.clone()),
        size,
        color,
    }
}

pub(super) fn sync(text: &str, styled: &mut String, spans: &mut Vec<Span>, fallback: Span) {
    *spans = if spans.is_empty() && !text.is_empty() {
        vec![fallback]
    } else {
        rich_text::edited(styled, text, spans)
    };
    *styled = text.to_owned();
}

/// TextEdit reports Unicode character offsets; PDF spans use UTF-8 byte offsets.
/// None means the inspector has focus: retain the last on-page selection.
pub(super) fn capture(
    text: &str,
    selected: &mut Option<Range<usize>>,
    cursor: Option<egui::text::CCursorRange>,
) {
    if let Some(cursor) = cursor {
        let chars = cursor.as_sorted_char_range();
        let byte = |n| text.char_indices().nth(n).map_or(text.len(), |(i, _)| i);
        *selected = Some(byte(chars.start)..byte(chars.end));
    }
}

#[derive(Clone, Debug)]
pub(super) enum Change {
    Family(String),
    Face(usize),
    Bold(bool),
    Italic(bool),
    Size(f32),
    Color([f32; 3]),
}

pub(super) fn apply(
    text: &str,
    spans: &mut Vec<Span>,
    selected: &Option<Range<usize>>,
    change: Change,
) {
    let range = selected
        .clone()
        .filter(|r| !r.is_empty())
        .unwrap_or(0..text.len());
    *spans = rich_text::restyle(spans, range, |s| {
        let fonts = text_style::fonts();
        match &change {
            Change::Size(size) => s.size = *size,
            Change::Color(color) => s.color = *color,
            Change::Face(index) => s.font = fonts.get(*index).map(|f| f.path.clone()),
            _ => {
                if let Some(current) = fonts
                    .iter()
                    .find(|f| Some(&f.path) == s.font.as_ref())
                    .or_else(|| fonts.first())
                {
                    let family = if let Change::Family(family) = &change {
                        family
                    } else {
                        &current.family
                    };
                    let bold = if let Change::Bold(bold) = change {
                        bold
                    } else {
                        current.bold
                    };
                    let italic = if let Change::Italic(italic) = change {
                        italic
                    } else {
                        current.italic
                    };
                    if let Some(face) = fonts
                        .iter()
                        .find(|f| &f.family == family && f.bold == bold && f.italic == italic)
                    {
                        s.font = Some(face.path.clone());
                    }
                }
            }
        }
    });
}

// Stable IDs also let native UI regression tests exercise the actual controls.
pub(super) fn control(ui: &mut egui::Ui, name: &str, button: ActionButton<'_>) -> egui::Response {
    let response = ui.add(button);
    #[cfg(test)]
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::new(("text_style_control", name)), response.rect));
    #[cfg(not(test))]
    let _ = name;
    response
}

#[allow(clippy::too_many_arguments)]
pub(super) fn panel(
    ui: &mut egui::Ui,
    text: &str,
    spans: &mut Vec<Span>,
    selected: &Option<Range<usize>>,
    font: &mut usize,
    size: &mut f32,
    color: &mut [f32; 3],
    alignment: &mut Alignment,
) {
    let fonts = text_style::fonts();
    let at = selected
        .as_ref()
        .map_or(0, |r| r.start.min(text.len().saturating_sub(1)));
    if let Some(style) = spans.iter().find(|s| s.range.contains(&at)) {
        if let Some(index) = fonts
            .iter()
            .position(|f| Some(&f.path) == style.font.as_ref())
        {
            *font = index;
        }
        *size = style.size;
        *color = style.color;
    }
    let selected_text = selected
        .as_ref()
        .filter(|r| !r.is_empty())
        .and_then(|r| text.get(r.clone()));
    ui.label(
        RichText::new(selected_text.map_or_else(
            || "Text block".to_owned(),
            |s| format!("{} selected characters", s.chars().count()),
        ))
        .small()
        .color(theme::muted()),
    );
    let mut changes = Vec::new();
    ui::caption(ui, "Font");
    if let Some(current) = fonts.get(*font) {
        egui::ComboBox::from_id_salt("font_family")
            .width(ui.available_width())
            .selected_text(&current.family)
            .show_ui(ui, |ui| {
                for face in fonts.iter().filter(|f| !f.bold && !f.italic) {
                    if ui
                        .selectable_label(face.family == current.family, &face.family)
                        .clicked()
                    {
                        changes.push(Change::Family(face.family.clone()));
                    }
                }
            });
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("font_style")
                .width((ui.available_width() - 90.).max(90.))
                .selected_text(current.style())
                .show_ui(ui, |ui| {
                    for (index, face) in fonts
                        .iter()
                        .enumerate()
                        .filter(|(_, f)| f.family == current.family)
                    {
                        if ui.selectable_label(index == *font, face.style()).clicked() {
                            changes.push(Change::Face(index));
                        }
                    }
                });
            if ui
                .add(egui::DragValue::new(size).range(4.0..=144.).suffix(" pt"))
                .on_hover_text("Font size")
                .changed()
            {
                changes.push(Change::Size(*size));
            }
        });
        ui.horizontal(|ui| {
            if control(
                ui,
                "bold",
                ActionButton::new(Icon::Bold, "Bold")
                    .compact()
                    .selected(current.bold),
            )
            .clicked()
            {
                changes.push(Change::Bold(!current.bold));
            }
            if control(
                ui,
                "italic",
                ActionButton::new(Icon::Italic, "Italic")
                    .compact()
                    .selected(current.italic),
            )
            .clicked()
            {
                changes.push(Change::Italic(!current.italic));
            }
        });
    }
    ui.add_space(8.);
    ui::caption(ui, "Alignment");
    ui.horizontal(|ui| {
        for (value, icon, label) in [
            (Alignment::Left, Icon::AlignLeft, "Align left"),
            (Alignment::Center, Icon::AlignCenter, "Align center"),
            (Alignment::Right, Icon::AlignRight, "Align right"),
            (Alignment::Justified, Icon::AlignJustify, "Justify"),
        ] {
            if ui
                .add(
                    ActionButton::new(icon, label)
                        .compact()
                        .selected(*alignment == value),
                )
                .clicked()
            {
                *alignment = value;
            }
        }
    });
    ui.add_space(8.);
    ui::caption(ui, "Color");
    if editing::color_palette(ui, color) {
        changes.push(Change::Color(*color));
    }
    for change in changes {
        // Keep defaults useful before a new, empty text box is placed.
        match &change {
            Change::Face(i) => *font = *i,
            Change::Family(_) | Change::Bold(_) | Change::Italic(_) => {
                let mut default = vec![span(*font, *size, *color, 1)];
                apply("x", &mut default, &None, change.clone());
                if let Some(index) = fonts
                    .iter()
                    .position(|f| Some(&f.path) == default[0].font.as_ref())
                {
                    *font = index;
                }
            }
            _ => {}
        }
        apply(text, spans, selected, change);
    }
    ui.add_space(8.);
    ui.label(RichText::new("Select characters to format them. With no selection, formatting applies to the block. Alignment applies to the whole block.").small().color(theme::muted()));
}

#[allow(clippy::too_many_arguments)]
pub(super) fn layout(
    ui: &egui::Ui,
    text: &str,
    source: &str,
    spans: &[Span],
    font: FontId,
    color: Color32,
    width: f32,
    scale: f32,
    alignment: Alignment,
) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), font.clone(), color, width);
    job.halign = match alignment {
        Alignment::Center => egui::Align::Center,
        Alignment::Right => egui::Align::RIGHT,
        _ => egui::Align::LEFT,
    };
    job.justify = alignment == Alignment::Justified;
    if !spans.is_empty() {
        job.sections.clear();
        for s in rich_text::edited(source, text, spans) {
            let family = text_style::fonts()
                .iter()
                .find(|f| Some(&f.path) == s.font.as_ref())
                .map(|f| egui::FontFamily::Name(f.ui_name().into()))
                .unwrap_or(font.family.clone());
            job.sections.push(egui::text::LayoutSection {
                leading_space: 0.,
                byte_range: s.range,
                format: egui::TextFormat {
                    font_id: FontId::new(s.size * scale, family),
                    color: Color32::from_rgb(
                        (s.color[0] * 255.) as u8,
                        (s.color[1] * 255.) as u8,
                        (s.color[2] * 255.) as u8,
                    ),
                    line_height: Some(s.size * scale * 1.3),
                    ..Default::default()
                },
            });
        }
    }
    ui.fonts_mut(|f| f.layout_job(job))
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::text::{CCursor, CCursorRange};

    #[test]
    fn selected_unicode_text_keeps_other_attributes_and_survives_pdf_reopen() {
        let text = "Plain İstanbul bold end";
        let normal = span(0, 16., [0.; 3], text.len());
        let mut spans = vec![normal.clone()];
        let mut selected = None;
        capture(
            text,
            &mut selected,
            Some(CCursorRange::two(CCursor::new(14), CCursor::new(6))),
        );
        assert_eq!(text.get(selected.clone().unwrap()), Some("İstanbul"));
        apply(text, &mut spans, &selected, Change::Italic(true));
        apply(text, &mut spans, &selected, Change::Color([0.8, 0.1, 0.2]));
        apply(text, &mut spans, &selected, Change::Size(20.));
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].font, normal.font);
        assert_eq!(spans[2].font, normal.font);
        assert_eq!(spans[0].color, normal.color);
        assert_eq!(spans[2].color, normal.color);
        assert_eq!(spans[0].size, 16.);
        assert_eq!(spans[2].size, 16.);
        // Applying another attribute across a mixed selection must retain italics.
        apply(text, &mut spans, &Some(0..text.len()), Change::Bold(true));
        let italic = text_style::fonts()
            .iter()
            .find(|f| Some(&f.path) == spans[1].font.as_ref())
            .unwrap();
        assert!(italic.bold && italic.italic);
        let source = convert::blank(595., 842.).unwrap();
        for alignment in [
            Alignment::Left,
            Alignment::Center,
            Alignment::Right,
            Alignment::Justified,
        ] {
            let pdf =
                rich_text::add_aligned(&source, 1, text, [40., 80., 480., 160.], &spans, alignment)
                    .unwrap();
            let runs = mypdf_milestone0::text_geometry::runs(&pdf, 1).unwrap();
            let word = runs.iter().find(|r| r.text.contains("İstanbul")).unwrap();
            assert!((word.color[0] - 0.8).abs() < 0.001);
            assert!(
                word.base_font.to_lowercase().contains("italic")
                    || word.base_font.to_lowercase().contains("oblique")
            );
            assert_eq!(word.size, 20.);
            for run in runs
                .iter()
                .filter(|r| r.text.contains("Plain") || r.text.contains("end"))
            {
                assert_eq!(run.color, [0.; 3]);
                assert_eq!(run.size, 16.);
                assert!(
                    !run.base_font.to_lowercase().contains("italic")
                        && !run.base_font.to_lowercase().contains("oblique")
                );
            }
        }
    }

    #[test]
    fn inspector_clicks_keep_text_selection_after_focus_moves() {
        let ctx = egui::Context::default();
        let text_id = egui::Id::new("selected_text_editor");
        let mut text = "Plain İstanbul end".to_owned();
        let mut spans = vec![span(0, 16., [0.; 3], text.len())];
        let mut selected = None;
        let (mut font, mut size, mut color, mut align) = (0, 16., [0.; 3], Alignment::Left);
        let mut frame = |events: Vec<egui::Event>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1000., 700.))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::SidePanel::right("styles")
                        .default_width(290.)
                        .show(ctx, |ui| {
                            panel(
                                ui, &text, &mut spans, &selected, &mut font, &mut size, &mut color,
                                &mut align,
                            );
                        });
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let edit = egui::TextEdit::multiline(&mut text).id(text_id).show(ui);
                        capture(&text, &mut selected, edit.cursor_range);
                    });
                },
            );
        };
        frame(vec![]);
        let mut state = egui::TextEdit::load_state(&ctx, text_id).unwrap();
        state
            .cursor
            .set_char_range(Some(CCursorRange::two(CCursor::new(6), CCursor::new(14))));
        state.store(&ctx, text_id);
        ctx.memory_mut(|m| m.request_focus(text_id));
        frame(vec![]);
        for id in [
            egui::Id::new(("text_style_control", "italic")),
            egui::Id::new(("color_swatch", "Coral")),
        ] {
            let rect = ctx.data(|d| d.get_temp::<Rect>(id)).unwrap();
            let pos = rect.center();
            frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ]);
            frame(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }]);
            frame(vec![]);
        }
        assert_eq!(text.get(selected.unwrap()), Some("İstanbul"));
        assert_eq!(spans.len(), 3);
        let face = text_style::fonts()
            .iter()
            .find(|f| Some(&f.path) == spans[1].font.as_ref())
            .unwrap();
        assert!(face.italic);
        assert_eq!(spans[1].color, [0.9, 0.29, 0.29]);
        assert_eq!(spans[0].color, [0.; 3]);
        assert_eq!(spans[2].color, [0.; 3]);
    }
}
