use lopdf::{Document, Object, Stream, dictionary};
use mypdf_milestone0::{content, convert, document, text_style, unicode_text};

/// A page whose text uses an embedded subset of Arial holding only the glyphs of
/// "Orhan G", like most PDFs produced by office software.
fn subset_font_pdf() -> Vec<u8> {
    let arial = text_style::fonts()
        .iter()
        .find(|f| f.family == "Arial" && !f.bold && !f.italic)
        .expect("Arial is installed");
    let data = std::fs::read(&arial.path).unwrap();
    let face = ttf_parser::Face::parse(&data, 0).unwrap();
    let mut remapper = subsetter::GlyphRemapper::new();
    for c in "Orhan G".chars() {
        remapper.remap(face.glyph_index(c).unwrap().0);
    }
    let program = subsetter::subset(&data, 0, &remapper).unwrap();
    let mut doc = Document::with_version("1.7");
    let file = doc.add_object(Stream::new(
        dictionary! { "Length1" => program.len() as i64 },
        program,
    ));
    let descriptor = doc.add_object(dictionary! {
        "Type" => "FontDescriptor",
        "FontName" => "ABCDEF+ArialMT",
        "Flags" => 32,
        "FontBBox" => vec![0.into(), (-200).into(), 1000.into(), 900.into()],
        "ItalicAngle" => 0,
        "Ascent" => 900,
        "Descent" => -200,
        "CapHeight" => 700,
        "StemV" => 80,
        "FontFile2" => file,
    });
    let font = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "TrueType",
        "BaseFont" => "ABCDEF+ArialMT",
        "FirstChar" => 32,
        "LastChar" => 255,
        "Widths" => (32..=255).map(|_| Object::Integer(556)).collect::<Vec<_>>(),
        "Encoding" => "WinAnsiEncoding",
        "FontDescriptor" => descriptor,
    });
    let content = doc.add_object(Stream::new(
        dictionary! {},
        b"BT /F1 14 Tf 72 700 Td (Name: ) Tj (Orhan G) Tj ET".to_vec(),
    ));
    let pages = doc.new_object_id();
    let page = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages,
        "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        "Contents" => content,
    });
    doc.objects.insert(
        pages,
        dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 }.into(),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
    doc.trailer.set("Root", catalog);
    let mut bytes = vec![];
    doc.save_to(&mut bytes).unwrap();
    bytes
}

#[test]
fn glyphs_missing_from_an_embedded_subset_are_set_in_an_installed_font() {
    let source = subset_font_pdf();
    let runs = document::text_runs(&source, 1).unwrap();
    let run = runs.iter().find(|r| r.text == "Orhan G").unwrap();
    // WinAnsi can encode ö and ç, but this subset has no glyphs for them.
    let (edited, families) = document::replace_text_object_reporting(
        &source,
        1,
        run.operator_index,
        "Orhan G",
        "Orhan Gökçöl",
    )
    .unwrap();
    assert!(families.iter().any(|f| f == "Arial"), "{families:?}");
    let after = document::text_runs(&edited, 1).unwrap();
    assert!(after.iter().any(|r| r.text == "Orhan Gökçöl"));
    assert!(
        after.iter().any(|r| r.text == "Name: "),
        "other text is untouched"
    );
    let temp = tempfile::tempdir().unwrap();
    assert!(document::render(&edited, temp.path(), 1.).is_ok());

    // Characters the subset already has keep the document's own font.
    let (_, families) =
        document::replace_text_object_reporting(&source, 1, run.operator_index, "Orhan G", "Ghana")
            .unwrap();
    assert!(families.is_empty(), "{families:?}");
}

#[test]
fn new_text_in_any_installed_script_is_shaped_subset_and_searchable() {
    let blank = convert::blank(595., 842.).unwrap();
    let mut writer =
        unicode_text::Writer::new(&unicode_text::Preferred::LikePdfFont("ArialMT".into())).unwrap();
    // Each sample needs a font that may not be installed everywhere.
    let samples: Vec<&str> = [
        "Orhan Gökçöl İstanbul ğüşı",
        "Привет мир Ελληνικά",
        "שלום עולם",
        "مرحبا بالعالم",
        "नमस्ते दुनिया",
        "こんにちは世界",
        "안녕하세요",
        "สวัสดีชาวโลก",
    ]
    .into_iter()
    .filter(|s| writer.measure(s, 12.).is_ok())
    .collect();
    assert!(samples.len() >= 3, "Latin, Cyrillic and Hebrew via Arial");
    let text = samples.join("\n");
    let bytes =
        content::add_text(&blank, 1, &text, [40., 40., 500., 400.], 18., [0., 0., 0.]).unwrap();
    assert!(
        bytes.len() < 400_000,
        "fonts are subset: {} bytes",
        bytes.len()
    );
    let runs = document::text_runs(&bytes, 1).unwrap();
    assert!(runs.iter().any(|r| r.text == samples[0]));
    // Right-to-left text is drawn in visual order.
    assert!(runs.iter().any(|r| r.text.contains("םולש")));
    let doc = Document::load_mem(&bytes).unwrap();
    let page = *doc.get_pages().get(&1).unwrap();
    let ops = doc.get_and_decode_page_content(page).unwrap().operations;
    assert!(
        ops.iter().any(|op| op.operator == "BDC"),
        "complex scripts carry ActualText for copy and search"
    );
    let temp = tempfile::tempdir().unwrap();
    assert!(document::render(&bytes, temp.path(), 1.).is_ok());
}

#[test]
fn text_without_any_installed_glyph_is_refused() {
    let blank = convert::blank(595., 842.).unwrap();
    let error = content::add_text(&blank, 1, "\u{10FFFD}", [40., 40., 300., 60.], 12., [0.; 3])
        .unwrap_err();
    assert!(format!("{error:#}").contains("No installed font"));
}
