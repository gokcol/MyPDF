//! Locate exact text-showing operators, including repeated identical text.
//! IDs are painted into an analysis-only copy; the document being edited is never altered.
use anyhow::{Context as _, Result, ensure};
use hayro::{
    hayro_interpret::{
        self as interpret, BlendMode, ClipPath, Device, DrawMode, DrawProps, Image, ImageDrawProps,
        SoftMask, TransformExt, font::Glyph,
    },
    hayro_syntax::Pdf,
    kurbo::{BezPath, Point, Rect, Shape},
};
use lopdf::{
    Dictionary, Object, Stream,
    content::{Content, Operation},
};
use std::collections::HashMap;

pub fn runs(bytes: &[u8], page: u32) -> Result<Vec<crate::document::TextRun>> {
    let mut runs = crate::document::text_runs(bytes, page)?;
    if runs.is_empty() {
        return Ok(runs);
    }
    ensure!(
        runs.len() <= 20_000,
        "This page has too many text objects for editing"
    );
    let mut doc = lopdf::Document::load_mem(bytes)?;
    let id = *doc.get_pages().get(&page).context("Page not found")?;
    let source = doc.get_and_decode_page_content(id)?;
    let ids: HashMap<_, _> = runs
        .iter()
        .enumerate()
        .map(|(i, r)| (r.operator_index, i + 1))
        .collect();
    let mut operations = Vec::new();
    let mut spacing = HashMap::new();
    let mut text_spacing = [0_f32; 2]; // Character and word spacing in text space.
    let mut spacing_stack = Vec::new();
    for (index, op) in source.operations.into_iter().enumerate() {
        match op.operator.as_str() {
            "Tc" => text_spacing[0] = number(&op, 0),
            "Tw" => text_spacing[1] = number(&op, 0),
            "\"" => text_spacing = [number(&op, 1), number(&op, 0)],
            "q" => spacing_stack.push(text_spacing),
            "Q" => text_spacing = spacing_stack.pop().unwrap_or([0.; 2]),
            _ => {}
        }
        // XObject text is not edited by the page-level writer; exclude it from this lookup.
        if op.operator == "Do" {
            continue;
        }
        if let Some(tag) = ids.get(&index) {
            let font_size = runs[*tag - 1].size;
            if font_size.is_finite() && font_size.abs() > f32::EPSILON {
                spacing.insert(*tag, text_spacing.map(|v| v / font_size * 1000.));
            }
            let rgb: Vec<Object> = [(tag >> 16) & 255, (tag >> 8) & 255, tag & 255]
                .into_iter()
                .map(|c| Object::Real(c as f32 / 255.))
                .collect();
            operations.push(Operation::new("rg", rgb.clone()));
            operations.push(Operation::new("RG", rgb));
        }
        operations.push(op);
    }
    let stream = doc.add_object(Stream::new(
        Dictionary::new(),
        Content { operations }.encode()?,
    ));
    let page_dict = doc.get_dictionary_mut(id)?;
    page_dict.set("Contents", stream);
    page_dict.remove(b"Annots");
    // RGB IDs must not be remapped by an inherited DefaultRGB color space.
    let mut resources = crate::content::resources(&doc, id)?;
    if let Ok(cs) = resources
        .get(b"ColorSpace")
        .and_then(|o| doc.dereference(o).map(|(_, o)| o))
        .and_then(Object::as_dict)
    {
        let mut cs = cs.clone();
        cs.remove(b"DefaultRGB");
        resources.set("ColorSpace", cs);
    }
    doc.get_dictionary_mut(id)?.set("Resources", resources);
    let mut analysis = Vec::new();
    doc.save_to(&mut analysis)?;
    let pdf = Pdf::new(analysis).map_err(|e| anyhow::anyhow!("Could not locate text: {e:?}"))?;
    let page = pdf
        .pages()
        .get(page as usize - 1)
        .context("Page not found")?;
    let cache = interpret::InterpreterCache::new();
    let (w, h) = page.render_dimensions();
    let mut context = interpret::Context::new(
        page.initial_transform(true).to_kurbo(),
        Rect::new(0., 0., w as f64, h as f64),
        &cache,
        page.xref(),
        interpret::InterpreterSettings::default(),
    );
    let mut device = Geometry {
        boxes: HashMap::new(),
        glyphs: HashMap::new(),
        spacing,
        font_spaces: HashMap::new(),
        count: 0,
    };
    interpret::interpret_page(page, &mut context, &mut device);
    ensure!(
        device.count <= 100_000,
        "This page has too many glyphs for editing"
    );
    for (index, run) in runs.iter_mut().enumerate() {
        if let Some((rect, size)) = device.boxes.get(&(index + 1)) {
            run.rect = Some([
                rect.x0 as f32,
                rect.y0 as f32,
                rect.x1 as f32,
                rect.y1 as f32,
            ]);
            run.size = *size;
        }
        if let Some(glyphs) = device.glyphs.get(&(index + 1)) {
            run.layout = collected_layout(glyphs, &device.font_spaces, &run.text);
        }
    }
    Ok(runs)
}
fn number(op: &Operation, index: usize) -> f32 {
    op.operands
        .get(index)
        .and_then(|value| value.as_float().ok())
        .unwrap_or(0.)
}

struct PositionedText {
    text: Option<String>,
    origin: [f32; 2],
    end: [f32; 2],
    direction: [f32; 2],
    units_to_page: f32,
    font: u128,
}

impl PositionedText {
    fn layout(&self, spaces: &HashMap<u128, f32>) -> crate::document::RunLayout {
        crate::document::RunLayout {
            text: self.text.clone().unwrap_or_default(),
            origin: self.origin,
            end: self.end,
            direction: self.direction,
            // Use the font's actual space advance whenever the page supplies one.
            // A quarter em is a conservative fallback for fonts with no spaces.
            space_width: spaces.get(&self.font).copied().unwrap_or(250.) * self.units_to_page,
        }
    }
}

fn collected_layout(
    glyphs: &[Option<PositionedText>],
    spaces: &HashMap<u128, f32>,
    original: &str,
) -> Option<crate::document::RunLayout> {
    let glyphs: Vec<_> = glyphs.iter().map(Option::as_ref).collect::<Option<_>>()?;
    let first = glyphs.first()?.layout(spaces);
    let last = glyphs.last()?.layout(spaces);
    let plain: Option<String> = glyphs.iter().map(|g| g.text.as_deref()).collect();
    let mut text = original.to_owned();
    // Glyph Unicode may be unavailable or differ for ligatures/custom encodings.
    // Never replace decoded operator contents with an unverified transcription.
    if plain.as_deref() == Some(original) {
        text.clear();
        let mut previous: Option<crate::document::RunLayout> = None;
        for glyph in glyphs {
            let layout = glyph.layout(spaces);
            if previous
                .as_ref()
                .is_some_and(|p| p.needs_space_before(&layout))
            {
                text.push(' ');
            }
            text.push_str(&layout.text);
            previous = Some(layout);
        }
    }
    Some(crate::document::RunLayout {
        text,
        end: last.end,
        ..first
    })
}

struct Geometry {
    boxes: HashMap<usize, (Rect, f32)>,
    glyphs: HashMap<usize, Vec<Option<PositionedText>>>,
    spacing: HashMap<usize, [f32; 2]>,
    font_spaces: HashMap<u128, f32>,
    count: usize,
}
impl<'a> Device<'a> for Geometry {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
    fn draw_glyph_run(
        &mut self,
        run: &interpret::font::GlyphRun<'_, 'a>,
        props: DrawProps<'a>,
        mode: &DrawMode,
    ) {
        if matches!(mode, DrawMode::Invisible) {
            return;
        }
        let interpret::Paint::Color(color) = props.paint else {
            return;
        };
        let [r, g, b, _] = color.to_rgba().to_rgba8();
        let id = ((r as usize) << 16) | ((g as usize) << 8) | b as usize;
        if id == 0 {
            return;
        }
        for glyph in run.glyphs() {
            self.count += 1;
            if self.count > 100_000 {
                return;
            }
            let transform = props.transform * glyph.transform();
            let origin = transform * Point::ZERO;
            let size = ((transform * Point::new(0., 1000.)) - origin).hypot() as f32;
            let unicode = glyph.as_unicode().map(|text| match text {
                interpret::hayro_cmap::BfString::Char(c) => c.to_string(),
                interpret::hayro_cmap::BfString::String(s) => s,
            });
            let positioned = match &**glyph {
                Glyph::Outline(g) => g.advance_width().and_then(|advance| {
                    let [character, word] = self.spacing.get(&id).copied()?;
                    let axis = transform * Point::new(1., 0.) - origin;
                    let units_to_page = axis.hypot() as f32;
                    if !advance.is_finite()
                        || advance <= 0.
                        || !units_to_page.is_finite()
                        || units_to_page <= 0.
                    {
                        return None;
                    }
                    let space = unicode.as_deref() == Some(" ");
                    if space {
                        self.font_spaces.insert(g.font_cache_key(), advance);
                    }
                    let end = transform
                        * Point::new(
                            (advance + character + if space { word } else { 0. }) as f64,
                            0.,
                        );
                    if !origin.is_finite() || !end.is_finite() {
                        return None;
                    }
                    Some(PositionedText {
                        text: unicode.clone(),
                        origin: [origin.x as f32, origin.y as f32],
                        end: [end.x as f32, end.y as f32],
                        direction: [axis.x as f32 / units_to_page, axis.y as f32 / units_to_page],
                        units_to_page,
                        font: g.font_cache_key(),
                    })
                }),
                Glyph::Type3(_) => None,
            };
            self.glyphs.entry(id).or_default().push(positioned);
            let rect = match &**glyph {
                Glyph::Outline(g) if !g.outline().is_empty() => {
                    (transform * g.outline()).bounding_box()
                }
                _ => (transform * Rect::new(0., -200., 500., 800.).to_path(0.1)).bounding_box(),
            };
            if rect.is_finite() && size.is_finite() {
                self.boxes
                    .entry(id)
                    .and_modify(|(r, s)| {
                        *r = r.union(rect);
                        *s = s.max(size);
                    })
                    .or_insert((rect, size));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn positioned_fixture(operations: Vec<Operation>) -> Vec<u8> {
        let blank = crate::convert::blank(595., 842.).unwrap();
        let mut doc = lopdf::Document::load_mem(&blank).unwrap();
        let page = doc.get_pages()[&1];
        let font = doc.add_object(lopdf::dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
            "Encoding" => "WinAnsiEncoding"
        });
        let mut content = vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.into()]),
            Operation::new(
                "Tm",
                vec![
                    1.into(),
                    0.into(),
                    0.into(),
                    1.into(),
                    50.into(),
                    750.into(),
                ],
            ),
        ];
        content.extend(operations);
        content.push(Operation::new("ET", vec![]));
        let stream = doc.add_object(Stream::new(
            Dictionary::new(),
            Content {
                operations: content,
            }
            .encode()
            .unwrap(),
        ));
        let page = doc.get_dictionary_mut(page).unwrap();
        page.set(
            "Resources",
            lopdf::dictionary! {"Font" => lopdf::dictionary! {"F1" => font}},
        );
        page.set("Contents", stream);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        bytes
    }

    fn show(text: &str) -> Operation {
        Operation::new("Tj", vec![Object::string_literal(text)])
    }

    fn reconstructed(runs: &[crate::document::TextRun]) -> String {
        let mut text = String::new();
        let mut previous: Option<&crate::document::RunLayout> = None;
        for run in runs {
            let layout = run.layout.as_ref().expect("Outline font geometry");
            if previous.is_some_and(|p| p.needs_space_before(layout)) {
                text.push(' ');
            }
            text.push_str(&layout.text);
            previous = Some(layout);
        }
        text
    }

    #[test]
    fn advance_boundaries_keep_split_numbers_and_versions_intact() {
        let bytes = positioned_fixture(vec![
            show("Version "),
            show("8.1"),
            show("1, 202"),
            show("6"),
        ]);
        let located = runs(&bytes, 1).unwrap();
        assert_eq!(reconstructed(&located), "Version 8.11, 2026");
        for pair in located.windows(2) {
            let previous = pair[0].layout.as_ref().unwrap();
            let next = pair[1].layout.as_ref().unwrap();
            assert!(!previous.needs_space_before(next));
            for (a, b) in previous.end.into_iter().zip(next.origin) {
                assert!(
                    (a - b).abs() < 0.01,
                    "Adjacent advances: {previous:?} {next:?}"
                );
            }
        }
        // Ink bounds do contain gaps between some adjacent characters. Those
        // sidebearings must never become new spaces in the editing draft.
        assert!(located.windows(2).any(|pair| {
            pair[1].rect.unwrap()[0] - pair[0].rect.unwrap()[2] > located[0].size * 0.12
        }));
    }

    #[test]
    fn positioned_tj_word_gaps_are_display_only_and_keep_exact_replacement_keys() {
        let bytes = positioned_fixture(vec![Operation::new(
            "TJ",
            vec![Object::Array(vec![
                Object::string_literal("Version"),
                (-278).into(),
                Object::string_literal("8.1"),
                (-20).into(),
                Object::string_literal("1, "),
                Object::string_literal("202"),
                (-20).into(),
                Object::string_literal("6"),
            ])],
        )]);
        let located = runs(&bytes, 1).unwrap();
        assert_eq!(located[0].text, "Version8.11, 2026");
        assert_eq!(reconstructed(&located), "Version 8.11, 2026");
        assert_eq!(
            crate::document::text_runs(&bytes, 1).unwrap()[0].text,
            located[0].text
        );
        let replaced = crate::document::replace_text_object(
            &bytes,
            1,
            located[0].operator_index,
            &located[0].text,
            "Version 8.12, 2026",
        )
        .unwrap();
        assert_eq!(
            crate::document::text_runs(&replaced, 1).unwrap()[0].text,
            "Version 8.12, 2026"
        );
    }

    #[test]
    fn explicit_numeric_spaces_and_character_tracking_are_not_reinterpreted() {
        let bytes = positioned_fixture(vec![
            Operation::new("Tc", vec![2.into()]),
            show("202"),
            show("6 "),
            Operation::new("Tw", vec![4.into()]),
            show("1 234 567"),
        ]);
        assert_eq!(reconstructed(&runs(&bytes, 1).unwrap()), "2026 1 234 567");
    }

    #[test]
    fn separate_runs_keep_real_word_gaps_on_rotated_pages() {
        let mut bytes = positioned_fixture(vec![
            show("Hello"),
            Operation::new(
                "TJ",
                vec![Object::Array(vec![
                    (-278).into(),
                    Object::string_literal("world"),
                ])],
            ),
        ]);
        for _ in 0..4 {
            let located = runs(&bytes, 1).unwrap();
            assert_eq!(reconstructed(&located), "Hello world");
            assert!(
                located[0]
                    .layout
                    .as_ref()
                    .unwrap()
                    .needs_space_before(located[1].layout.as_ref().unwrap())
            );
            bytes = crate::pages::apply(
                &bytes,
                &crate::pages::Operation::Rotate {
                    pages: vec![0],
                    clockwise: true,
                },
            )
            .unwrap();
        }
    }
    #[test]
    fn geometry_tracks_crop_and_rotation_without_matching_the_wrong_occurrence() {
        let blank = crate::convert::blank(595., 842.).unwrap();
        let mut bytes = crate::content::add_text(
            &blank,
            1,
            "Rotate me",
            [50., 80., 200., 60.],
            18.,
            [0., 0., 0.],
        )
        .unwrap();
        bytes = crate::pages::apply(
            &bytes,
            &crate::pages::Operation::Crop {
                pages: vec![0],
                margins: [18.; 4],
            },
        )
        .unwrap();
        for _ in 0..4 {
            let located = runs(&bytes, 1).unwrap()[0].rect.unwrap();
            let pdf = Pdf::new(bytes.clone()).unwrap();
            let text =
                crate::viewer::extract_page(&pdf.pages()[0], &interpret::InterpreterCache::new());
            let hit = text.find("Rotate me", 0).remove(0);
            let rect = hit
                .rects
                .iter()
                .map(|r| Rect::new(r[0] as f64, r[1] as f64, r[2] as f64, r[3] as f64))
                .reduce(|a, b| a.union(b))
                .unwrap();
            for (a, b) in located
                .into_iter()
                .zip([rect.x0, rect.y0, rect.x1, rect.y1])
            {
                assert!((a as f64 - b).abs() < 0.1);
            }
            bytes = crate::pages::apply(
                &bytes,
                &crate::pages::Operation::Rotate {
                    pages: vec![0],
                    clockwise: true,
                },
            )
            .unwrap();
        }
    }
    #[test]
    fn exact_operator_geometry_and_unicode_replacement_preserve_other_occurrences() {
        let source = crate::convert::blank(595., 842.).unwrap();
        let source = crate::content::add_text(
            &source,
            1,
            "Repeated",
            [40., 80., 200., 60.],
            18.,
            [0., 0.5, 0.],
        )
        .unwrap();
        let source = crate::content::add_text(
            &source,
            1,
            "Repeated",
            [220., 220., 200., 60.],
            18.,
            [0., 0., 0.],
        )
        .unwrap();
        let positioned = runs(&source, 1).unwrap();
        assert_eq!(positioned.len(), 2);
        let first = positioned[0].rect.unwrap();
        let second = positioned[1].rect.unwrap();
        assert!((first[0] - 40.).abs() < 5. && (second[0] - 220.).abs() < 5.);
        assert!(first[1] > 70. && first[3] < 120. && second[1] > 210. && second[3] < 260.);
        assert!(
            (positioned[0].size - 18.).abs() < 1.,
            "font size {}",
            positioned[0].size
        );
        let changed = crate::document::replace_text_object(
            &source,
            1,
            positioned[1].operator_index,
            "Repeated",
            "New text: İstanbul!",
        )
        .unwrap();
        let updated = runs(&changed, 1).unwrap();
        assert_eq!(updated[0].text, "Repeated");
        assert_eq!(updated[1].text, "New text: İstanbul!");
        assert!((updated[1].rect.unwrap()[0] - second[0]).abs() < 5.);
    }
}
