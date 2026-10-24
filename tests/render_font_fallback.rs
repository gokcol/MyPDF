//! Render actual nonembedded-font PDFs: a cmap-only assertion would miss the
//! interpreter dropping the Unicode glyph before it reaches the rasterizer.
use hayro::hayro_interpret::{
    InterpreterSettings,
    font::{FallbackFontQuery, FontQuery, StandardFont},
    hayro_cmap::{CharacterCollection, CidFamily},
};
use lopdf::{
    Document, Object, Stream,
    content::{Content, Operation},
    dictionary,
};
use mypdf_milestone0::{render_fonts, viewer};

fn pdf(font: lopdf::Dictionary, contents: Vec<u8>) -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    let pages = doc.new_object_id();
    let font = doc.add_object(font);
    let contents = doc.add_object(Stream::new(dictionary! {}, contents));
    let page = doc.add_object(dictionary! { "Type"=>"Page", "Parent"=>pages,
    "MediaBox"=>vec![0.into(),0.into(),360.into(),80.into()],
    "Resources"=>dictionary! {"Font"=>dictionary! {"F1"=>font}}, "Contents"=>contents });
    doc.objects.insert(
        pages,
        dictionary! { "Type"=>"Pages", "Count"=>1, "Kids"=>vec![page.into()] }.into(),
    );
    let catalog = doc.add_object(dictionary! { "Type"=>"Catalog", "Pages"=>pages });
    doc.trailer.set("Root", catalog);
    let mut bytes = vec![];
    doc.save_to(&mut bytes).unwrap();
    bytes
}
fn glyph_name(character: char) -> &'static str {
    match character {
        'N' => "N",
        'Ğ' => "Gbreve",
        'İ' => "Idot",
        'Ş' => "Scedilla",
        'ğ' => "gbreve",
        'ı' => "dotlessi",
        'ş' => "scedilla",
        'Ω' => "Omegagreek",
        'Ж' => "afii10024",
        'ש' => "afii57689",
        'ا' => "afii57415",
        _ => unreachable!(),
    }
}
fn simple_pdf(base: &str, character: char) -> Vec<u8> {
    pdf(
        dictionary! { "Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>Object::Name(base.as_bytes().to_vec()),
        "FirstChar"=>65, "LastChar"=>65, "Widths"=>vec![600.into()],
        "Encoding"=>dictionary! { "Type"=>"Encoding", "BaseEncoding"=>"WinAnsiEncoding",
            "Differences"=>vec![65.into(), Object::Name(glyph_name(character).as_bytes().to_vec())] } },
        Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 24.into()]),
                Operation::new("Td", vec![12.into(), 25.into()]),
                Operation::new("Tj", vec![Object::string_literal("A")]),
                Operation::new("ET", vec![]),
            ],
        }
        .encode()
        .unwrap(),
    )
}
fn render(bytes: &[u8], settings: &InterpreterSettings) -> Vec<u8> {
    let doc = hayro::hayro_syntax::Pdf::new(bytes.to_vec()).unwrap();
    hayro::render(
        &doc.pages()[0],
        &hayro::RenderCache::new(),
        settings,
        &Default::default(),
        &hayro::PixmapSettings {
            x_scale: 2.,
            y_scale: 2.,
            bg_color: hayro::vello_cpu::color::palette::css::WHITE,
        },
    )
    .data_as_u8_slice()
    .to_vec()
}
fn ink(rgba: &[u8]) -> usize {
    rgba.chunks_exact(4)
        .filter(|p| p[0] < 180 && p[1] < 180 && p[2] < 180)
        .count()
}

#[test]
fn standard_faces_render_missing_extended_latin_greek_and_cyrillic_glyphs() {
    let settings = render_fonts::settings();
    let mut checked = 0;
    let mut missing_before = 0;
    for (name, font) in [
        ("Times-Roman", StandardFont::TimesRoman),
        ("Times-Bold", StandardFont::TimesBold),
        ("Times-Italic", StandardFont::TimesItalic),
        ("Times-BoldItalic", StandardFont::TimesBoldItalic),
        ("Helvetica", StandardFont::Helvetica),
        ("Helvetica-Bold", StandardFont::HelveticaBold),
        ("Helvetica-Oblique", StandardFont::HelveticaOblique),
        ("Helvetica-BoldOblique", StandardFont::HelveticaBoldOblique),
        ("Courier", StandardFont::Courier),
        ("Courier-Bold", StandardFont::CourierBold),
        ("Courier-Oblique", StandardFont::CourierOblique),
        ("Courier-BoldOblique", StandardFont::CourierBoldOblique),
    ] {
        let (data, index) = (settings.font_resolver)(&FontQuery::Standard(font)).unwrap();
        let Ok(face) = ttf_parser::Face::parse(data.as_ref().as_ref(), index) else {
            continue;
        };
        for character in "ĞİŞğışΩЖשا"
            .chars()
            .filter(|c| face.glyph_index(*c).is_some())
        {
            let source = simple_pdf(name, character);
            let original = source.clone();
            if ink(&render(&source, &InterpreterSettings::default())) == 0 {
                missing_before += 1;
            }
            assert!(
                ink(&render(&source, &settings)) > 20,
                "{name}/{character} must have a visible outline"
            );
            assert_eq!(source, original);
            checked += 1;
        }
    }
    assert!(
        missing_before > 0,
        "the fixtures must expose missing original glyphs"
    );
    if cfg!(target_os = "macos") {
        assert!(
            checked >= 96,
            "all installed Base14 substitutes and scripts were exercised: {checked}"
        );
    } else {
        assert!(
            checked > 0,
            "install an Arial or DejaVu standard substitute to exercise fallback rendering"
        );
    }
}

fn mixed_simple_pdf(name: &str, text: &str, font_bytes: &[u8]) -> Vec<u8> {
    let face = ttf_parser::Face::parse(font_bytes, 0).unwrap();
    let widths: Vec<Object> = text
        .chars()
        .map(|character| {
            let glyph = face.glyph_index(character).unwrap();
            Object::Real(
                f32::from(face.glyph_hor_advance(glyph).unwrap()) * 1000.
                    / f32::from(face.units_per_em()),
            )
        })
        .collect();
    let mut differences = vec![1.into()];
    differences.extend(
        text.chars()
            .map(|character| Object::Name(glyph_name(character).as_bytes().to_vec())),
    );
    let mut operations = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 24.into()]),
    ];
    for (i, _) in text.chars().enumerate() {
        operations.push(Operation::new(
            "Tm",
            vec![
                1.into(),
                0.into(),
                0.into(),
                1.into(),
                (12 + i as i32 * 32).into(),
                25.into(),
            ],
        ));
        operations.push(Operation::new(
            "Tj",
            vec![Object::String(
                vec![(i + 1) as u8],
                lopdf::StringFormat::Hexadecimal,
            )],
        ));
    }
    operations.push(Operation::new("ET", vec![]));
    pdf(
        dictionary! {"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>Object::Name(name.as_bytes().to_vec()),
        "FirstChar"=>1,"LastChar"=>text.chars().count() as i64,"Widths"=>widths,
        "Encoding"=>dictionary! {"Type"=>"Encoding","BaseEncoding"=>"WinAnsiEncoding","Differences"=>differences}},
        Content { operations }.encode().unwrap(),
    )
}
/// Embed the original font and explicitly map CIDs using its Unicode cmap.
/// This bypasses POST-name lookup, independently proving the intended outlines.
fn embedded_cmap_reference(simple: &[u8], text: &str, font_bytes: &[u8]) -> Vec<u8> {
    let face = ttf_parser::Face::parse(font_bytes, 0).unwrap();
    let mut doc = Document::load_mem(simple).unwrap();
    let page = doc.get_pages()[&1];
    let font_id = doc
        .get_dictionary(page)
        .unwrap()
        .get(b"Resources")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Font")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"F1")
        .unwrap()
        .as_reference()
        .unwrap();
    let widths = doc
        .get_dictionary(font_id)
        .unwrap()
        .get(b"Widths")
        .unwrap()
        .clone();
    let file = doc.add_object(Stream::new(
        dictionary! {"Length1"=>font_bytes.len() as i64},
        font_bytes.to_vec(),
    ));
    let descriptor = doc.add_object(dictionary! {"Type"=>"FontDescriptor","FontName"=>"ReferenceFont","Flags"=>4,
        "FontBBox"=>vec![0.into(),(-300).into(),1000.into(),1000.into()],"ItalicAngle"=>0,"Ascent"=>900,"Descent"=>-300,
        "CapHeight"=>700,"StemV"=>80,"FontFile2"=>file});
    let mut gids = vec![0, 0];
    for character in text.chars() {
        gids.extend_from_slice(&face.glyph_index(character).unwrap().0.to_be_bytes());
    }
    let gid_map = doc.add_object(Stream::new(dictionary! {}, gids));
    let child = doc.add_object(dictionary! {"Type"=>"Font","Subtype"=>"CIDFontType2","BaseFont"=>"ReferenceFont",
        "CIDSystemInfo"=>dictionary! {"Registry"=>Object::string_literal("Adobe"),"Ordering"=>Object::string_literal("Identity"),"Supplement"=>0},
        "FontDescriptor"=>descriptor,"CIDToGIDMap"=>gid_map,"W"=>vec![1.into(),widths]});
    doc.objects.insert(font_id,dictionary! {"Type"=>"Font","Subtype"=>"Type0","BaseFont"=>"ReferenceFont","Encoding"=>"Identity-H","DescendantFonts"=>vec![child.into()]}.into());
    let mut content = Content::decode(&doc.get_page_content(page)).unwrap();
    for operation in &mut content.operations {
        if operation.operator == "Tj" {
            let code = operation.operands[0].as_str().unwrap()[0];
            operation.operands[0] = Object::String(vec![0, code], lopdf::StringFormat::Hexadecimal);
        }
    }
    doc.change_page_content(page, content.encode().unwrap())
        .unwrap();
    let mut bytes = vec![];
    doc.save_to(&mut bytes).unwrap();
    bytes
}
fn glyph_difference(a: &[u8], b: &[u8]) -> f32 {
    let mut different = 0;
    let mut combined = 0;
    for (a, b) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let a = a[0] < 180;
        let b = b[0] < 180;
        combined += usize::from(a || b);
        different += usize::from(a != b);
    }
    different as f32 / combined.max(1) as f32
}
#[test]
fn ascii_and_turkish_mix_matches_explicit_embedded_unicode_glyphs() {
    let settings = render_fonts::settings();
    let mut exposed_name_collision = false;
    let mut checked = 0;
    for face in mypdf_milestone0::text_style::fonts()
        .iter()
        .filter(|face| matches!(face.family.as_str(), "Times New Roman" | "DejaVuSerif"))
    {
        let raw = std::fs::read(&face.path).unwrap();
        let name = format!(
            "MissingSerif{}{}",
            if face.bold { "-Bold" } else { "" },
            if face.italic { "-Italic" } else { "" }
        );
        let source = mixed_simple_pdf(&name, "NNİNĞŞNıN", &raw);
        let reference = embedded_cmap_reference(&source, "NNİNĞŞNıN", &raw);
        let expected = render(&reference, &InterpreterSettings::default());
        let actual = render(&source, &settings);
        assert!(
            glyph_difference(&actual, &expected) < 0.02,
            "{} must preserve cmap-correct N and Turkish outlines",
            face.style()
        );
        let face_ref = ttf_parser::Face::parse(&raw, 0).unwrap();
        let post_n = (0..face_ref.number_of_glyphs())
            .map(ttf_parser::GlyphId)
            .rfind(|glyph| face_ref.glyph_name(*glyph) == Some("N"));
        exposed_name_collision |= post_n != face_ref.glyph_index('N');
        if let Some(dir) = std::env::var_os("MYPDF_FONT_PROOF_DIR") {
            let name = format!("ascii-turkish-{}.png", face.style().replace(' ', "-"));
            image::save_buffer(
                std::path::PathBuf::from(dir).join(name),
                &actual,
                720,
                160,
                image::ColorType::Rgba8,
            )
            .unwrap();
        }
        checked += 1;
    }
    assert!(checked > 0);
    if cfg!(target_os = "macos") {
        assert!(
            exposed_name_collision,
            "the installed Times font must expose duplicate POST-name lookup"
        );
    }
}

#[test]
fn unicode_glyph_names_render_the_same_as_adobe_glyph_names() {
    let settings = render_fonts::settings();
    for character in "NİΩЖ".chars() {
        let canonical = simple_pdf("Times-Roman", character);
        let mut doc = Document::load_mem(&canonical).unwrap();
        let font = doc
            .objects
            .values_mut()
            .find_map(|object| {
                let dict = object.as_dict_mut().ok()?;
                (dict.get(b"Type").ok()?.as_name().ok()? == b"Font").then_some(dict)
            })
            .unwrap();
        font.set("Encoding",dictionary! {"Type"=>"Encoding","BaseEncoding"=>"WinAnsiEncoding",
            "Differences"=>vec![65.into(),Object::Name(format!("uni{:04X}",character as u32).into_bytes())]});
        let mut unicode = vec![];
        doc.save_to(&mut unicode).unwrap();
        let expected = render(&canonical, &settings);
        let actual = render(&unicode, &settings);
        assert!(ink(&expected) > 20);
        assert!(
            glyph_difference(&actual, &expected) < 0.02,
            "uni glyph name for {character} must select its Unicode outline"
        );
    }
}

fn cid_pdf(name: &str, ordering: &str, text: &str) -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    let pages = doc.new_object_id();
    let descriptor=doc.add_object(dictionary! {"Type"=>"FontDescriptor","FontName"=>Object::Name(name.as_bytes().to_vec()),
        "Flags"=>4,"FontBBox"=>vec![0.into(),(-250).into(),1000.into(),1000.into()],"ItalicAngle"=>0,"Ascent"=>900,"Descent"=>-200,"CapHeight"=>700,"StemV"=>80});
    let child=doc.add_object(dictionary! {"Type"=>"Font","Subtype"=>"CIDFontType2","BaseFont"=>Object::Name(name.as_bytes().to_vec()),
        "CIDSystemInfo"=>dictionary! {"Registry"=>Object::string_literal("Adobe"),"Ordering"=>Object::string_literal(ordering),"Supplement"=>0},
        "FontDescriptor"=>descriptor,"DW"=>1000,"CIDToGIDMap"=>"Identity"});
    let mut mapping = String::from(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /MyMap def /CMapType 2 def 1 begincodespacerange <0000> <FFFF> endcodespacerange\n",
    );
    mapping += &format!("{} beginbfchar\n", text.chars().count());
    let mut codes = vec![];
    for (i, c) in text.chars().enumerate() {
        mapping += &format!("<{:04X}> <{:04X}>\n", i + 1, c as u32);
        codes.extend_from_slice(&((i + 1) as u16).to_be_bytes());
    }
    mapping += "endbfchar endcmap CMapName currentdict /CMap defineresource pop end end";
    let cmap = doc.add_object(Stream::new(dictionary! {}, mapping.into_bytes()));
    let font=doc.add_object(dictionary! {"Type"=>"Font","Subtype"=>"Type0","BaseFont"=>Object::Name(name.as_bytes().to_vec()),"Encoding"=>"Identity-H","DescendantFonts"=>vec![child.into()],"ToUnicode"=>cmap});
    let contents = doc.add_object(Stream::new(
        dictionary! {},
        Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 24.into()]),
                Operation::new("Td", vec![12.into(), 25.into()]),
                Operation::new(
                    "Tj",
                    vec![Object::String(codes, lopdf::StringFormat::Hexadecimal)],
                ),
                Operation::new("ET", vec![]),
            ],
        }
        .encode()
        .unwrap(),
    ));
    let page=doc.add_object(dictionary! {"Type"=>"Page","Parent"=>pages,"MediaBox"=>vec![0.into(),0.into(),360.into(),80.into()],"Resources"=>dictionary! {"Font"=>dictionary! {"F1"=>font}},"Contents"=>contents});
    doc.objects.insert(
        pages,
        dictionary! {"Type"=>"Pages","Count"=>1,"Kids"=>vec![page.into()]}.into(),
    );
    let catalog = doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages});
    doc.trailer.set("Root", catalog);
    let mut bytes = vec![];
    doc.save_to(&mut bytes).unwrap();
    bytes
}

#[test]
fn cid_missing_fonts_render_available_chinese_japanese_korean_and_named_rtl_faces() {
    let settings = render_fonts::settings();
    let mut checked = 0;
    for (ordering, family, name, text) in [
        ("GB1", CidFamily::AdobeGB1, "MissingChineseFont", "中文简体"),
        (
            "CNS1",
            CidFamily::AdobeCNS1,
            "MissingTraditionalFont",
            "中文繁體",
        ),
        (
            "Japan1",
            CidFamily::AdobeJapan1,
            "MissingJapaneseFont",
            "日本語かな",
        ),
        (
            "Korea1",
            CidFamily::AdobeKorea1,
            "MissingKoreanFont",
            "한글한국",
        ),
        ("Identity", CidFamily::AdobeIdentity, "ArialHebrew", "עברית"),
        // Presentation forms model an already-shaped PDF. A font resolver
        // cannot infer Arabic joining from a PDF missing its original mapping.
        (
            "Identity",
            CidFamily::AdobeIdentity,
            "GeezaPro",
            "\u{FEB3}\u{FEFC}\u{FEE1}",
        ),
    ] {
        let query = FallbackFontQuery {
            post_script_name: Some(name.into()),
            character_collection: Some(CharacterCollection {
                family,
                supplement: 0,
            }),
            ..Default::default()
        };
        let (data, index) = (settings.font_resolver)(&FontQuery::Fallback(query)).unwrap();
        let supported = ttf_parser::Face::parse(data.as_ref().as_ref(), index)
            .is_ok_and(|face| text.chars().all(|c| face.glyph_index(c).is_some()));
        if !supported {
            eprintln!(
                "No installed fallback for {ordering}/{name}; coverage is platform-dependent"
            );
            continue;
        }
        let source = cid_pdf(name, ordering, text);
        let original = source.clone();
        let before = render(&source, &InterpreterSettings::default());
        let after = render(&source, &settings);
        assert_ne!(
            after, before,
            "generic fallback renders wrong or missing {ordering} glyphs"
        );
        if let Some(dir) = std::env::var_os("MYPDF_FONT_PROOF_DIR") {
            let dir = std::path::PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            image::save_buffer(
                dir.join(format!("{ordering}-{name}.png")),
                &after,
                720,
                160,
                image::ColorType::Rgba8,
            )
            .unwrap();
        }
        assert!(
            ink(&after) > 100,
            "{ordering}/{name} must render actual glyph outlines"
        );
        let (size, rgba) = viewer::render_page(&source, 0, 2.).unwrap();
        assert_eq!(size, [720, 160]);
        assert!(rgba == after, "shared render_page must use the resolver");
        assert_eq!(source, original);
        checked += 1;
    }
    if cfg!(target_os = "macos") {
        assert_eq!(
            checked, 6,
            "this Mac's installed region/RTL fonts cover all fixtures"
        );
    }
}

#[test]
fn collection_indices_symbols_and_embedded_fonts_remain_correct() {
    let settings = render_fonts::settings();
    for font in [StandardFont::Symbol, StandardFont::ZapfDingBats] {
        let (old, old_index) =
            (InterpreterSettings::default().font_resolver)(&FontQuery::Standard(font)).unwrap();
        let (new, new_index) = (settings.font_resolver)(&FontQuery::Standard(font)).unwrap();
        assert_eq!(old_index, new_index);
        assert_eq!(old.as_ref().as_ref(), new.as_ref().as_ref());
    }
    #[cfg(target_os = "macos")]
    {
        let query = FallbackFontQuery {
            post_script_name: Some("AppleSDGothicNeo-Bold".into()),
            is_bold: true,
            ..Default::default()
        };
        let (data, index) = (settings.font_resolver)(&FontQuery::Fallback(query)).unwrap();
        assert!(
            index > 0,
            "bold Korean face is inside its TTC, not face zero"
        );
        let face = ttf_parser::Face::parse(data.as_ref().as_ref(), index).unwrap();
        assert!(face.glyph_index('한').is_some());
    }
    let text = "Embedded glyphs: ĞİŞğış ΩЖ";
    let source = mypdf_milestone0::rich_text::add(
        &mypdf_milestone0::convert::blank(360., 80.).unwrap(),
        1,
        text,
        [12., 10., 330., 60.],
        &[mypdf_milestone0::rich_text::Span {
            range: 0..text.len(),
            font: Some(mypdf_milestone0::document::default_font().unwrap()),
            size: 18.,
            color: [0.; 3],
        }],
    )
    .unwrap();
    assert!(
        render(&source, &InterpreterSettings::default()) == render(&source, &settings),
        "embedded fonts must never be substituted"
    );
}
