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
    let preset = styles::Preset::Text {
        font: fonts.get(*font).map(|f| f.path.clone()),
        size: *size,
        color: *color,
    };
    if let Some(styles::Preset::Text {
        font: next_font,
        size: next_size,
        color: next_color,
    }) = styles::panel(ui, preset)
    {
        let range = selected
            .clone()
            .filter(|r| !r.is_empty())
            .unwrap_or(0..text.len());
        *spans = rich_text::restyle(spans, range, |s| {
            s.font = next_font.clone();
            s.size = next_size;
            s.color = next_color;
        });
        if let Some(i) = fonts
            .iter()
            .position(|f| Some(&f.path) == next_font.as_ref())
        {
            *font = i;
        }
        *size = next_size;
        *color = next_color;
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
                .width(ui.available_width())
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
        });
        let old_size = *size;
        editing::property_slider(ui, "Font size", size, 4.0..=144., " pt");
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("font_size_presets")
                .selected_text("Common sizes")
                .width((ui.available_width() - 80.).max(90.))
                .show_ui(ui, |ui| {
                    for value in [
                        6., 8., 9., 10., 10.5, 11., 12., 14., 16., 18., 20., 24., 28., 32., 36.,
                        48., 60., 72., 96., 144.,
                    ] {
                        ui.selectable_value(size, value, format!("{value} pt"));
                    }
                });
            for (icon, label, delta) in [
                (Icon::Minus, "Decrease font size", -1.),
                (Icon::Plus, "Increase font size", 1.),
            ] {
                if control(ui, label, ActionButton::new(icon, label).compact()).clicked() {
                    *size = (*size + delta).clamp(4., 144.);
                }
            }
        });
        if *size != old_size {
            changes.push(Change::Size(*size));
        }
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
    job.sections[0].format.line_height = Some(font.size * 1.3);
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

/// Retain the PDF's explicit line origins while keeping a normal TextEdit
/// galley: every glyph/newline still maps to the original Unicode cursor index.
#[allow(clippy::too_many_arguments)]
pub(super) fn layout_positioned(
    ui: &egui::Ui,
    text: &str,
    source: &str,
    spans: &[Span],
    font: FontId,
    color: Color32,
    width: f32,
    scale: f32,
    alignment: Alignment,
    lines: &[rich_text::LineLayout],
) -> Arc<egui::Galley> {
    let ordinary = layout(
        ui,
        text,
        source,
        spans,
        font.clone(),
        color,
        width,
        scale,
        alignment,
    );
    if lines.is_empty() || !scale.is_finite() || scale <= 0. {
        return ordinary;
    }
    let lines = rich_text::edited_layout(source, text, lines, font.size / scale * 1.3);
    let starts: Vec<_> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    if lines.len() != starts.len()
        || lines.iter().zip(&starts).any(|(line, start)| {
            line.start != *start || !line.left.is_finite() || !line.baseline.is_finite()
        })
    {
        return ordinary;
    }

    let mut galley = (*ordinary).clone();
    // Stored indentation already describes the original alignment. Explicit
    // alignment/reflow changes use layout() instead of these retained origins.
    Arc::make_mut(&mut galley.job).halign = egui::Align::LEFT;
    Arc::make_mut(&mut galley.job).justify = false;
    galley.rows.clear();
    galley.rect = Rect::ZERO;
    galley.mesh_bounds = Rect::NOTHING;
    galley.num_vertices = 0;
    galley.num_indices = 0;
    let mut wrap_shift = 0.;
    for (line_index, paragraph) in text.split('\n').enumerate() {
        let line = lines[line_index];
        let end = line.start + paragraph.len();
        let indent = line.left * scale;
        let mut job = (*galley.job).clone();
        job.text = paragraph.to_owned();
        job.wrap.max_width = (width - indent).max(1.);
        job.sections = galley
            .job
            .sections
            .iter()
            .filter_map(|section| {
                let start = section.byte_range.start.max(line.start);
                let end = section.byte_range.end.min(end);
                (start < end).then(|| egui::text::LayoutSection {
                    byte_range: start - line.start..end - line.start,
                    ..section.clone()
                })
            })
            .collect();
        if job.sections.is_empty() {
            let format = galley
                .job
                .sections
                .iter()
                .rev()
                .find(|section| section.byte_range.start <= line.start)
                .map(|section| section.format.clone())
                .unwrap_or_else(|| egui::TextFormat::simple(font.clone(), color));
            job.sections.push(egui::text::LayoutSection {
                leading_space: 0.,
                byte_range: 0..paragraph.len(),
                format,
            });
        }
        let paragraph = ui.fonts_mut(|fonts| fonts.layout_job(job));
        let mut byte = 0;
        let mut previous_size: Option<f32> = None;
        for (row_index, placed) in paragraph.rows.iter().enumerate() {
            let mut placed = placed.clone();
            let row = Arc::make_mut(&mut placed.row);
            let row_end = byte
                + row
                    .glyphs
                    .iter()
                    .map(|glyph| glyph.chr.len_utf8())
                    .sum::<usize>();
            let size = paragraph
                .job
                .sections
                .iter()
                .filter(|section| {
                    section.byte_range.start < row_end && byte < section.byte_range.end
                })
                .map(|section| section.format.font_id.size)
                .reduce(f32::max)
                .unwrap_or(paragraph.job.sections[0].format.font_id.size);
            if let Some(previous) = previous_size {
                wrap_shift += previous.max(size) * 1.3;
            }
            previous_size = Some(size);
            byte = row_end;

            // egui aligns differently sized fonts at their bottoms. PDF runs
            // share a baseline, so move each glyph and its mesh together.
            let baseline = row
                .glyphs
                .iter()
                .map(|glyph| glyph.font_ascent)
                .reduce(f32::max)
                .unwrap_or(size);
            row.visuals.mesh.translate(vec2(indent, 0.));
            for index in 0..row.glyphs.len() {
                let glyph = row.glyphs[index];
                let start = glyph.first_vertex as usize;
                let end = row
                    .glyphs
                    .get(index + 1)
                    .map_or(row.visuals.glyph_vertex_range.end, |next| {
                        next.first_vertex as usize
                    });
                for vertex in &mut row.visuals.mesh.vertices[start..end] {
                    vertex.pos.y += baseline - glyph.pos.y;
                }
                row.glyphs[index].pos = pos2(glyph.pos.x + indent, baseline);
            }
            row.size.x += indent;
            row.size.y = row.size.y.max(
                baseline
                    + row
                        .glyphs
                        .iter()
                        .map(|glyph| glyph.line_height - glyph.font_ascent)
                        .fold(0., f32::max),
            );
            row.ends_with_newline =
                row_index + 1 == paragraph.rows.len() && line_index + 1 < lines.len();
            row.visuals.mesh_bounds = row.visuals.mesh.calc_bounds();
            // Cursor X lookup uses row-local glyphs, unlike pointer hit tests,
            // so keep indentation in the glyphs/mesh and leave row.pos.x at 0.
            placed.pos = pos2(0., line.baseline * scale + wrap_shift - baseline);
            galley.rect |= Rect::from_min_size(placed.pos, row.size);
            galley.mesh_bounds |= row.visuals.mesh_bounds.translate(placed.pos.to_vec2());
            galley.num_vertices += row.visuals.mesh.vertices.len();
            galley.num_indices += row.visuals.mesh.indices.len();
            galley.rows.push(placed);
        }
    }
    galley.rect |= galley.mesh_bounds;
    Arc::new(galley)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::text::{CCursor, CCursorRange};

    fn positioned(
        text: &str,
        spans: &[Span],
        lines: &[rich_text::LineLayout],
        width: f32,
        scale: f32,
    ) -> Arc<egui::Galley> {
        let ctx = egui::Context::default();
        let mut galley = None;
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                galley = Some(layout_positioned(
                    ui,
                    text,
                    text,
                    spans,
                    FontId::monospace(12. * scale),
                    Color32::BLACK,
                    width,
                    scale,
                    Alignment::Left,
                    lines,
                ));
            });
        });
        galley.unwrap()
    }

    fn test_span(range: Range<usize>, size: f32) -> Span {
        Span {
            range,
            font: None,
            size,
            color: [0.; 3],
        }
    }

    #[test]
    fn positioned_lines_preserve_indentation_gaps_and_unicode_cursor_geometry() {
        let text = "İlk satır\nSecond line\nParagraph";
        let lines = [
            rich_text::LineLayout {
                start: 0,
                left: 24.,
                baseline: 18.,
            },
            rich_text::LineLayout {
                start: text.find("Second").unwrap(),
                left: 4.,
                baseline: 40.,
            },
            rich_text::LineLayout {
                start: text.find("Paragraph").unwrap(),
                left: 30.,
                baseline: 100.,
            },
        ];
        let galley = positioned(text, &[test_span(0..text.len(), 12.)], &lines, 600., 1.5);
        assert_eq!(galley.rows.len(), 3);
        assert_eq!(galley.text(), text);
        for (row, line) in galley.rows.iter().zip(&lines) {
            let first = row.glyphs.first().unwrap();
            assert!((first.pos.x + row.pos.x - line.left * 1.5).abs() < 0.01);
            assert!((first.pos.y + row.pos.y - line.baseline * 1.5).abs() < 0.01);
            let index = text[..line.start].chars().count();
            let caret = galley.pos_from_cursor(CCursor::new(index));
            assert!((caret.left() - line.left * 1.5).abs() < 0.01);
            assert_eq!(
                galley.cursor_from_pos(caret.center().to_vec2()).index,
                index
            );
            assert!(galley.rect.contains_rect(row.rect()));
        }
        assert!(galley.rows[2].min_y() - galley.rows[1].max_y() > 40.);
        assert!(galley.rect.contains_rect(galley.mesh_bounds));
        assert_eq!(galley.end().index, text.chars().count());
    }

    #[test]
    fn positioned_mixed_sizes_share_baseline_and_include_ascender_overflow() {
        let text = "small BIG small\nNext";
        let lines = [
            rich_text::LineLayout {
                start: 0,
                left: 20.,
                baseline: 16.,
            },
            rich_text::LineLayout {
                start: text.find("Next").unwrap(),
                left: 0.,
                baseline: 90.,
            },
        ];
        let galley = positioned(
            text,
            &[
                test_span(0..6, 12.),
                test_span(6..9, 30.),
                test_span(9..text.len(), 12.),
            ],
            &lines,
            400.,
            1.,
        );
        assert_eq!(galley.rows.len(), 2);
        for (row, line) in galley.rows.iter().zip(&lines) {
            for glyph in &row.glyphs {
                assert!((row.pos.y + glyph.pos.y - line.baseline).abs() < 0.01);
            }
        }
        assert!(galley.rows[0].glyphs[6].font_height > galley.rows[0].glyphs[0].font_height);
        assert!(
            galley.rect.top() < 0.,
            "a larger font keeps its original baseline"
        );
        assert!(galley.rect.contains_rect(galley.mesh_bounds));
        for index in [1, 7, 12, 17] {
            let caret = galley.pos_from_cursor(CCursor::new(index));
            assert_eq!(
                galley.cursor_from_pos(caret.center().to_vec2()).index,
                index
            );
        }
    }

    #[test]
    fn positioned_soft_wraps_keep_indent_and_shift_following_original_lines() {
        let text = "one two three four five six seven\nTail";
        let lines = [
            rich_text::LineLayout {
                start: 0,
                left: 18.,
                baseline: 18.,
            },
            rich_text::LineLayout {
                start: text.find("Tail").unwrap(),
                left: 5.,
                baseline: 100.,
            },
        ];
        let galley = positioned(text, &[test_span(0..text.len(), 12.)], &lines, 90., 1.);
        let wrapped_rows = galley.rows.len() - 1;
        assert!(wrapped_rows > 1);
        let mut cursor_index = 0;
        for (index, row) in galley.rows.iter().enumerate() {
            let first = row.glyphs.first().unwrap();
            let (left, baseline) = if index < wrapped_rows {
                (18., 18. + index as f32 * 12. * 1.3)
            } else {
                (5., 100. + (wrapped_rows - 1) as f32 * 12. * 1.3)
            };
            assert!((first.pos.x + row.pos.x - left).abs() < 0.01);
            assert!((first.pos.y + row.pos.y - baseline).abs() < 0.01);
            let caret = galley.pos_from_cursor(CCursor {
                index: cursor_index,
                prefer_next_row: true,
            });
            assert!((caret.left() - left).abs() < 0.01);
            assert_eq!(
                galley.cursor_from_pos(caret.center().to_vec2()).index,
                cursor_index
            );
            assert_eq!(row.ends_with_newline, index + 1 == wrapped_rows);
            cursor_index += row.char_count_including_newline();
        }
        assert_eq!(cursor_index, text.chars().count());
        assert!(
            galley.rows[..wrapped_rows]
                .iter()
                // egui retains trailing spaces for cursor movement even when
                // their logical advances extend past the visible wrap edge.
                .all(|row| row
                    .glyphs
                    .iter()
                    .filter(|glyph| !glyph.chr.is_whitespace())
                    .all(|glyph| glyph.max_x() <= 90.1))
        );
    }

    #[test]
    fn positioned_live_newline_keeps_later_gap_and_trailing_empty_cursor() {
        let source = "A\nB\n";
        let text = "A\nNew\nB\n";
        let lines = [
            rich_text::LineLayout {
                start: 0,
                left: 18.,
                baseline: 18.,
            },
            rich_text::LineLayout {
                start: 2,
                left: 4.,
                baseline: 80.,
            },
            rich_text::LineLayout {
                start: 4,
                left: 8.,
                baseline: 110.,
            },
        ];
        let ctx = egui::Context::default();
        let mut result = None;
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                result = Some(layout_positioned(
                    ui,
                    text,
                    source,
                    &[test_span(0..source.len(), 12.)],
                    FontId::monospace(12.),
                    Color32::BLACK,
                    300.,
                    1.,
                    Alignment::Left,
                    &lines,
                ));
            });
        });
        let galley = result.unwrap();
        assert_eq!(galley.rows.len(), 4);
        for (row, (left, baseline)) in
            galley.rows[..3]
                .iter()
                .zip([(18., 18.), (18., 33.6), (4., 95.6)])
        {
            assert!((row.glyphs[0].pos.x - left).abs() < 0.01);
            assert!((row.pos.y + row.glyphs[0].pos.y - baseline).abs() < 0.01);
        }
        assert!(galley.rows[3].glyphs.is_empty());
        assert_eq!(galley.end().index, text.chars().count());
        let caret = galley.pos_from_cursor(galley.end());
        assert!((caret.left() - 8.).abs() < 0.01);
        assert_eq!(
            galley.cursor_from_pos(caret.center().to_vec2()).index,
            text.chars().count()
        );
        assert!(galley.rect.contains_rect(caret));
    }

    #[test]
    fn positioned_native_textedit_drag_selects_across_indented_paragraph_gap() {
        let ctx = egui::Context::default();
        let source = "First\nİkinci\nLast";
        let mut text = source.to_owned();
        let spans = [test_span(0..source.len(), 12.)];
        let lines = [
            rich_text::LineLayout {
                start: 0,
                left: 30.,
                baseline: 18.,
            },
            rich_text::LineLayout {
                start: source.find('İ').unwrap(),
                left: 6.,
                baseline: 80.,
            },
            rich_text::LineLayout {
                start: source.find("Last").unwrap(),
                left: 40.,
                baseline: 130.,
            },
        ];
        let mut frame = |events| {
            let mut output = None;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(500., 400.))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, width| {
                            layout_positioned(
                                ui,
                                text.as_str(),
                                source,
                                &spans,
                                FontId::monospace(12.),
                                Color32::BLACK,
                                width,
                                1.,
                                Alignment::Left,
                                &lines,
                            )
                        };
                        output = Some(
                            egui::TextEdit::multiline(&mut text)
                                .id(egui::Id::new("positioned_text_editor"))
                                .desired_width(300.)
                                .desired_rows(1)
                                .margin(0.)
                                .frame(false)
                                .layouter(&mut layouter)
                                .show(ui),
                        );
                    });
                },
            );
            output.unwrap()
        };
        let output = frame(vec![]);
        let point = |index| {
            output.galley_pos
                + output
                    .galley
                    .pos_from_cursor(CCursor::new(index))
                    .center()
                    .to_vec2()
        };
        let (start, end) = (point(8), point(15));
        frame(vec![
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ]);
        frame(vec![egui::Event::PointerMoved(end)]);
        let selected = frame(vec![egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }])
        .cursor_range
        .unwrap();
        assert_eq!(selected.as_sorted_char_range(), 8..15);
        let mut bytes = None;
        capture(source, &mut bytes, Some(selected));
        assert_eq!(source.get(bytes.unwrap()), Some("inci\nLa"));
    }

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
            egui::Id::new(("text_style_control", "Increase font size")),
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
        assert_eq!(spans[1].size, 17.);
        assert_eq!(spans[0].size, 16.);
        assert_eq!(spans[2].size, 16.);
        assert_eq!(spans[0].color, [0.; 3]);
        assert_eq!(spans[2].color, [0.; 3]);
    }
}
