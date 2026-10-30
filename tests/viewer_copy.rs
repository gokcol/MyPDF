use lopdf::{Dictionary, Document, Stream, dictionary};
use mypdf_milestone0::{document, viewer};

fn text_page(content: &str) -> viewer::PageText {
    let mut doc = Document::with_version("1.4");
    let pages = doc.new_object_id();
    let fonts: Vec<_> = ["Helvetica", "Times-BoldItalic", "Courier"]
        .into_iter()
        .map(|name| {
            doc.add_object(dictionary! {
                "Type" => "Font", "Subtype" => "Type1", "BaseFont" => name,
                "Encoding" => "WinAnsiEncoding"
            })
        })
        .collect();
    let stream = doc.add_object(Stream::new(Dictionary::new(), content.as_bytes().to_vec()));
    let page = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Resources" => dictionary! {"Font" => dictionary! {
            "F1" => fonts[0], "F2" => fonts[1], "F3" => fonts[2]
        }},
        "Contents" => stream
    });
    doc.objects.insert(
        pages,
        dictionary! {
            "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1
        }
        .into(),
    );
    let catalog = doc.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages});
    doc.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    let pdf = hayro::hayro_syntax::Pdf::new(bytes).unwrap();
    viewer::extract_page(
        &pdf.pages()[0],
        &hayro::hayro_interpret::InterpreterCache::new(),
    )
}

fn style_at(page: &viewer::PageText, at: usize) -> &viewer::TextStyle {
    &page
        .spans
        .iter()
        .find(|span| span.range.contains(&at))
        .unwrap()
        .style
}

#[test]
fn faux_bold_paint_passes_copy_once_and_keep_genuine_repeated_text() {
    // Quartz's synthetic bold uses four fills, including two at the same offset.
    let text = text_page(
        "BT /F1 12.8 Tf 1 0 0 1 50 700 Tm (Budget review) Tj ET\n\
         BT /F1 12.8 Tf 1 0 0 1 50.3556 700 Tm (Budget review) Tj ET\n\
         BT /F1 12.8 Tf 1 0 0 1 50.3556 700 Tm (Budget review) Tj ET\n\
         BT /F1 12.8 Tf 1 0 0 1 50.7112 700 Tm (Budget review) Tj ET\n\
         BT /F1 12.8 Tf 1 0 0 1 50 660 Tm (Budget review) Tj ET\n\
         BT /F1 12.8 Tf 1 0 0 1 250 660 Tm (Budget review) Tj ET",
    );
    let matches: Vec<_> = text
        .text
        .match_indices("Budget review")
        .map(|(at, _)| at)
        .collect();
    assert_eq!(matches.len(), 3, "{}", text.text);
    assert!(style_at(&text, matches[0]).bold);
    assert!(!style_at(&text, matches[1]).bold);
    assert!(!style_at(&text, matches[2]).bold);
    assert_eq!(text.find("Budget review", 0).len(), 3);
    assert_eq!(text.glyphs.len(), 3 * "Budget review".len());
    assert_eq!(text.lines.len(), 2);
    assert!((text.lines[1].baseline - text.lines[0].baseline - 40.).abs() < 0.01);
    assert!(!text.limited);
}

#[test]
fn fill_and_stroke_text_is_one_logical_bold_run() {
    let text = text_page("BT /F1 14 Tf 2 Tr 1 0 0 1 50 700 Tm (Outlined title) Tj ET");
    assert_eq!(text.text, "Outlined title");
    assert_eq!(text.glyphs.len(), "Outlined title".len());
    assert!(style_at(&text, 0).bold);
}

#[test]
fn fill_and_stroke_with_different_colors_copies_once_and_retains_fill_color() {
    let text =
        text_page("1 0 0 rg 0 0 0 RG BT /F1 14 Tf 2 Tr 1 0 0 1 50 700 Tm (Outlined title) Tj ET");
    assert_eq!(text.text, "Outlined title");
    let style = style_at(&text, 0);
    assert!(style.bold);
    assert_eq!(style.color, [255, 0, 0]);
}

#[test]
fn exact_duplicate_paint_does_not_make_plain_text_bold() {
    let text = text_page(
        "BT /F1 12 Tf 1 0 0 1 50 700 Tm (Repeated paint) Tj ET\n\
         BT /F1 12 Tf 1 0 0 1 50 700 Tm (Repeated paint) Tj ET",
    );
    assert_eq!(text.text, "Repeated paint");
    assert!(!style_at(&text, 0).bold);
}

#[test]
fn standard_fonts_keep_family_emphasis_size_color_and_line_geometry() {
    let text = text_page(
        "BT /F1 12 Tf 1 0 0 1 50 700 Tm (Normal text) Tj ET\n\
         0.8 0.1 0.2 rg BT /F2 16 Tf 1 0 0 1 70 670 Tm (Emphasized text) Tj ET\n\
         0 g BT /F3 9 Tf 1 0 0 1 70 650 Tm (Code text) Tj ET",
    );
    assert_eq!(text.text, "Normal text\nEmphasized text\nCode text");
    let normal = style_at(&text, 0);
    assert_eq!(normal.family, "Helvetica");
    assert!(!normal.bold && !normal.italic);
    assert!((normal.size - 12.).abs() < 0.01);
    let emphasis = style_at(&text, text.text.find("Emphasized").unwrap());
    assert_eq!(emphasis.family, "Times");
    assert!(emphasis.bold && emphasis.italic);
    assert!((emphasis.size - 16.).abs() < 0.01);
    assert_eq!(emphasis.color, [204, 26, 51]);
    let code = style_at(&text, text.text.find("Code").unwrap());
    assert_eq!(code.family, "Courier");
    assert!((code.size - 9.).abs() < 0.01);
    assert_eq!(
        text.lines.iter().map(|line| line.left).collect::<Vec<_>>(),
        vec![50., 70., 70.]
    );
    assert_eq!(
        text.lines
            .iter()
            .map(|line| line.baseline)
            .collect::<Vec<_>>(),
        vec![92., 122., 142.]
    );
    for line in &text.lines {
        assert!(!text.text[line.range.clone()].contains('\n'));
    }
}

#[test]
fn differently_colored_overlapping_text_is_not_assumed_to_be_faux_bold() {
    let text = text_page(
        "BT /F1 12 Tf 1 0 0 1 50 700 Tm (Shadow) Tj ET\n\
         1 0 0 rg BT /F1 12 Tf 1 0 0 1 50.4 700 Tm (Shadow) Tj ET",
    );
    assert_eq!(text.text.matches("Shadow").count(), 2);
    assert_eq!(text.spans.len(), 2);
    assert!(text.spans.iter().all(|span| !span.style.bold));
}

#[test]
fn unicode_extraction_retains_valid_ranges_and_embedded_font_metadata() {
    let bytes = document::fixture(&document::default_font().unwrap()).unwrap();
    let pdf = hayro::hayro_syntax::Pdf::new(bytes).unwrap();
    let text = viewer::extract_page(
        &pdf.pages()[0],
        &hayro::hayro_interpret::InterpreterCache::new(),
    );
    assert!(text.text.contains(document::TURKISH_ORIGINAL));
    for glyph in &text.glyphs {
        assert!(text.text.get(glyph.range.clone()).is_some());
    }
    for span in &text.spans {
        assert!(text.text.get(span.range.clone()).is_some());
    }
    for line in &text.lines {
        assert!(text.text.get(line.range.clone()).is_some());
    }
    let unicode = style_at(&text, text.text.find(document::TURKISH_ORIGINAL).unwrap());
    assert!(!unicode.family.is_empty());
    assert_ne!(unicode.family, "sans-serif");
    assert!(unicode.size > 0.);
}

#[test]
fn empty_outline_spaces_keep_selectable_bounds_when_text_is_rotated() {
    let horizontal = text_page("BT /F1 14 Tf 1 0 0 1 50 700 Tm (A A) Tj ET");
    let vertical = text_page("BT /F1 14 Tf 0 1 -1 0 50 700 Tm (A A) Tj ET");
    for text in [&horizontal, &vertical] {
        assert_eq!(text.text, "A A");
        assert_eq!(text.glyphs.len(), 3);
        assert_eq!(&text.text[text.glyphs[1].range.clone()], " ");
        for glyph in &text.glyphs {
            assert!(glyph.rect.iter().all(|coordinate| coordinate.is_finite()));
            assert!(glyph.rect[2] > glyph.rect[0]);
            assert!(glyph.rect[3] > glyph.rect[1]);
        }
    }
    let h = horizontal.glyphs[1].rect;
    let v = vertical.glyphs[1].rect;
    let horizontal_size = [h[2] - h[0], h[3] - h[1]];
    let vertical_size = [v[2] - v[0], v[3] - v[1]];
    assert!(horizontal_size[0] < horizontal_size[1]);
    assert!((horizontal_size[0] - vertical_size[1]).abs() < 0.001);
    assert!((horizontal_size[1] - vertical_size[0]).abs() < 0.001);
}
