use lopdf::{Document, Object, Stream, dictionary};
use mypdf_milestone0::{annotations as ann, content, convert, document, forms, svg};
use std::sync::{Arc, OnceLock};
fn fixture() -> &'static Vec<u8> {
    static PDF: OnceLock<Vec<u8>> = OnceLock::new();
    PDF.get_or_init(|| document::fixture(&document::default_font().unwrap()).unwrap())
}

#[test]
fn add_selectable_text_to_blank_existing_and_rotated_pages() {
    let blank = convert::blank(595., 842.).unwrap();
    let added = content::add_text(
        &blank,
        1,
        "New text in İstanbul\nA second line",
        [50., 70., 400., 100.],
        20.,
        [0.1, 0.2, 0.7],
    )
    .unwrap();
    let runs = document::text_runs(&added, 1).unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].text, "New text in İstanbul");
    let modified =
        document::replace_text_object(&added, 1, runs[1].operator_index, &runs[1].text, "A line")
            .unwrap();
    assert!(
        document::text_runs(&modified, 1)
            .unwrap()
            .iter()
            .any(|r| r.text == "A line")
    );
    for page in [1, 2] {
        let added = content::add_text(
            fixture(),
            page,
            "Added block",
            [40., 35., 260., 65.],
            18.,
            [0.2, 0.4, 0.6],
        )
        .unwrap();
        let pdf = hayro::hayro_syntax::Pdf::new(added.clone()).unwrap();
        let text = mypdf_milestone0::viewer::extract_page(
            &pdf.pages()[page as usize - 1],
            &hayro::hayro_interpret::InterpreterCache::new(),
        );
        let hit = text.find("Added block", page as usize - 1).pop().unwrap();
        let left = hit.rects.iter().map(|r| r[0]).fold(f32::INFINITY, f32::min);
        let top = hit.rects.iter().map(|r| r[1]).fold(f32::INFINITY, f32::min);
        assert!((left - 40.).abs() < 3., "left {left}, page {page}");
        assert!((35. ..48.).contains(&top), "top {top}, page {page}");
        assert!(
            !document::text_runs(&added, 3 - page)
                .unwrap()
                .iter()
                .any(|r| r.text == "Added block")
        );
    }
    assert!(
        content::add_text(
            &blank,
            1,
            "Does not fit",
            [590., 70., 200., 100.],
            20.,
            [0.; 3]
        )
        .is_err()
    );
    assert!(content::add_text(&blank, 1, "Overflow", [40., 30., 25., 10.], 20., [0.; 3]).is_err());
}
fn png() -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(32, 24, image::Rgba([220, 55, 90, 160]));
    let mut out = std::io::Cursor::new(vec![]);
    image.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}
fn form_fixture() -> Vec<u8> {
    let mut d = Document::load_mem(&convert::blank(595., 842.).unwrap()).unwrap();
    let page = d.get_pages()[&1];
    let font =
        d.add_object(dictionary! {"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica"});
    let text=d.add_object(dictionary!{"FT"=>"Tx","T"=>Object::string_literal("Name"),"V"=>Object::string_literal(""),"MaxLen"=>80,"DA"=>Object::string_literal("/Helv 12 Tf 0 g")});
    let mut widgets = vec![];
    for y in [680, 625] {
        let w=d.add_object(dictionary!{"Type"=>"Annot","Subtype"=>"Widget","Rect"=>vec![70.into(),y.into(),400.into(),(y+30).into()],"Parent"=>text,"P"=>page,"F"=>4});
        widgets.push(w);
    }
    d.get_dictionary_mut(text).unwrap().set(
        "Kids",
        widgets
            .iter()
            .copied()
            .map(Object::Reference)
            .collect::<Vec<_>>(),
    );
    let off=d.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Form","BBox"=>vec![0.into(),0.into(),20.into(),20.into()]},b"0.5 G 1 w 1 1 18 18 re S".to_vec()));
    let on=d.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Form","BBox"=>vec![0.into(),0.into(),20.into(),20.into()]},b"0.2 0.6 0.3 rg 0 0 20 20 re f".to_vec()));
    let checkbox=d.add_object(dictionary!{"FT"=>"Btn","T"=>Object::string_literal("Accept"),"Type"=>"Annot","Subtype"=>"Widget","Rect"=>vec![70.into(),570.into(),90.into(),590.into()],"P"=>page,"V"=>"Off","AS"=>"Off","AP"=>dictionary!{"N"=>dictionary!{"Off"=>off,"Yes"=>on}}});
    widgets.push(checkbox);
    let radio = d.new_object_id();
    let mut radios = vec![];
    for (i, state) in ["First", "Second"].into_iter().enumerate() {
        let x = 70 + i as i64 * 60;
        let w=d.add_object(dictionary!{"Type"=>"Annot","Subtype"=>"Widget","Rect"=>vec![x.into(),520.into(),(x+20).into(),540.into()],"Parent"=>radio,"P"=>page,"AS"=>"Off","AP"=>dictionary!{"N"=>dictionary!{"Off"=>off,state=>on}}});
        radios.push(Object::Reference(w));
        widgets.push(w);
    }
    d.objects.insert(radio,Object::Dictionary(dictionary!{"FT"=>"Btn","Ff"=>1<<15,"T"=>Object::string_literal("Priority"),"V"=>"Off","Kids"=>radios}));
    let choice=d.add_object(dictionary!{"FT"=>"Ch","Ff"=>1<<17,"T"=>Object::string_literal("Country"),"Type"=>"Annot","Subtype"=>"Widget","Rect"=>vec![70.into(),450.into(),320.into(),482.into()],"P"=>page,"Opt"=>vec![Object::Array(vec![Object::string_literal("TR"),ann::text_string("Türkiye")]),Object::string_literal("France")]});
    widgets.push(choice);
    d.get_dictionary_mut(page).unwrap().set(
        "Annots",
        widgets
            .into_iter()
            .map(Object::Reference)
            .collect::<Vec<_>>(),
    );
    let stream=d.add_object(Stream::new(dictionary!{},b"BT /Helv 24 Tf 70 770 Td (MyPDF form proof) Tj /Helv 12 Tf 0 -40 Td (Name - both widgets share one value) Tj 0 -130 Td (Accept) Tj 0 -50 Td (Priority: first / second) Tj 0 -50 Td (Country) Tj ET".to_vec()));
    d.get_dictionary_mut(page)
        .unwrap()
        .set("Resources", dictionary! {"Font"=>dictionary!{"Helv"=>font}});
    d.get_dictionary_mut(page).unwrap().set("Contents", stream);
    let form=d.add_object(dictionary!{"Fields"=>vec![text.into(),checkbox.into(),radio.into(),choice.into()],"DR"=>dictionary!{"Font"=>dictionary!{"Helv"=>font}},"DA"=>Object::string_literal("/Helv 12 Tf 0 g"),"NeedAppearances"=>false});
    d.catalog_mut().unwrap().set("AcroForm", form);
    content::serialize(&mut d).unwrap()
}
#[test]
fn selected_text_occurrence_and_certified_guard() {
    let mut d = Document::load_mem(fixture()).unwrap();
    let id = d.get_pages()[&1];
    let mut c = d.get_and_decode_page_content(id).unwrap();
    let op = c
        .operations
        .iter()
        .find(|o| o.operator == "Tj")
        .unwrap()
        .clone();
    c.operations.push(op);
    let stream = d.add_object(Stream::new(dictionary! {}, c.encode().unwrap()));
    d.get_dictionary_mut(id).unwrap().set("Contents", stream);
    let bytes = content::serialize(&mut d).unwrap();
    let runs = document::text_runs(&bytes, 1).unwrap();
    let run = &runs[0];
    assert!(document::replace_run(&bytes, 1, &run.text, "Edited").is_err());
    let out =
        document::replace_text_object(&bytes, 1, run.operator_index, &run.text, "Edited").unwrap();
    assert_eq!(
        document::text_runs(&out, 1)
            .unwrap()
            .iter()
            .filter(|r| r.text == "Edited")
            .count(),
        1
    );
    assert!(
        document::replace_text_object(&out, 1, run.operator_index, &run.text, "Stale").is_err()
    );
    d.catalog_mut().unwrap().set("Perms", dictionary! {});
    let certified = content::serialize(&mut d).unwrap();
    assert!(
        document::replace_text_object(&certified, 1, run.operator_index, &run.text, "No").is_err()
    );
    assert!(content::edit_image(&certified, 1, 0, &content::ImageEdit::Delete).is_err());
}
#[test]
fn image_edits_isolate_resources_and_rotated_coordinates() {
    let images = content::images(fixture(), 1).unwrap();
    assert_eq!(images.len(), 1);
    let before2 = content::images(fixture(), 2).unwrap();
    let replaced = content::edit_image(
        fixture(),
        1,
        images[0].index,
        &content::ImageEdit::Replace(png()),
    )
    .unwrap();
    assert_eq!(content::images(&replaced, 1).unwrap()[0].pixels, [32, 24]);
    assert_eq!(content::images(&replaced, 2).unwrap()[0].pixels, [4, 4]);
    let r = [50., 85., 170., 145.];
    let moved = content::edit_image(
        &replaced,
        2,
        before2[0].index,
        &content::ImageEdit::Place(r),
    )
    .unwrap();
    let m = content::images(&moved, 2).unwrap();
    for (a, b) in m[0].rect.iter().zip(r) {
        assert!((a - b).abs() < 0.01, "{a} {b}");
    }
    assert_eq!(content::images(&moved, 1).unwrap()[0].rect, images[0].rect);
    let deleted = content::edit_image(&moved, 2, m[0].index, &content::ImageEdit::Delete).unwrap();
    assert!(content::images(&deleted, 2).unwrap().is_empty());
    assert_eq!(content::images(&deleted, 1).unwrap().len(), 1);
}
#[test]
fn form_values_multiwidget_appearances_radio_and_choice() {
    let original = form_fixture();
    let fields = forms::fields(&original).unwrap();
    assert_eq!(fields.len(), 4);
    let name = fields.iter().find(|f| f.name == "Name").unwrap();
    assert_eq!(name.widgets.len(), 2);
    let mut bytes = forms::fill(&original, name.id, "İstanbul & Paris").unwrap();
    for (name, value) in [("Accept", "Yes"), ("Priority", "Second"), ("Country", "TR")] {
        let id = forms::fields(&bytes)
            .unwrap()
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .id;
        bytes = forms::fill(&bytes, id, value).unwrap();
    }
    let d = Document::load_mem(&bytes).unwrap();
    let fields = forms::fields(&bytes).unwrap();
    assert_eq!(
        fields.iter().find(|f| f.name == "Name").unwrap().value,
        "İstanbul & Paris"
    );
    for f in &fields {
        for w in &f.widgets {
            let wd = d.get_dictionary(w.id).unwrap();
            assert!(wd.get(b"AP").is_ok());
            if f.name == "Priority" {
                let state = wd.get(b"AS").unwrap().as_name().unwrap();
                assert!(state == b"Off" || state == b"Second");
            }
        }
    }
    let name = fields.iter().find(|f| f.name == "Name").unwrap();
    assert!(forms::fill(&bytes, name.id, &"a".repeat(81)).is_err());
    let country = fields.iter().find(|f| f.name == "Country").unwrap();
    assert!(forms::fill(&bytes, country.id, "unknown").is_err());
    assert!(
        mypdf_milestone0::pages::apply(
            &bytes,
            &mypdf_milestone0::pages::Operation::Delete(vec![0])
        )
        .is_err()
    );
    let mut s = ann::Session::new(Arc::from(original.clone()), vec![], true);
    s.commit_document(bytes.clone()).unwrap();
    s.undo();
    assert_eq!(&*s.source, original);
    s.redo();
    assert_eq!(&*s.source, bytes);
}
#[test]
fn readonly_xfa_and_signed_form_rejection() {
    let mut d = Document::load_mem(&form_fixture()).unwrap();
    let id = forms::fields(&form_fixture()).unwrap()[0].id;
    d.get_dictionary_mut(id).unwrap().set("Ff", 1);
    let bytes = content::serialize(&mut d).unwrap();
    assert!(forms::fill(&bytes, id, "No").is_err());
    let form = d
        .catalog()
        .unwrap()
        .get(b"AcroForm")
        .unwrap()
        .as_reference()
        .unwrap();
    d.get_dictionary_mut(form)
        .unwrap()
        .set("XFA", Object::string_literal("unsupported"));
    let bytes = content::serialize(&mut d).unwrap();
    assert!(forms::fields(&bytes).is_err());
}
#[test]
fn vector_svg_source_recolor_and_pdf_roundtrip() {
    let source = svg::DEFAULT;
    let blank = convert::blank(595., 842.).unwrap();
    let mut a = ann::Annotation::new(
        0,
        [70., 120., 430., 300.],
        ann::Kind::Svg {
            source: source.into(),
            tint: true,
        },
        ann::Style::default(),
    );
    a.style.color = [0.8, 0.2, 0.4];
    a.rotation = 15.;
    let bytes = ann::export(&blank, &[a.clone()]).unwrap();
    let d = Document::load_mem(&bytes).unwrap();
    assert_eq!(ann::load(&d), vec![a]);
    assert!(
        !d.objects
            .values()
            .filter_map(|o| o.as_stream().ok())
            .any(|s| s
                .dict
                .get(b"Subtype")
                .and_then(Object::as_name)
                .is_ok_and(|n| n == b"Image"))
    );
    let png = svg::preview(source, Some([1., 0., 0.])).unwrap();
    let image = image::load_from_memory(&png).unwrap().to_rgba8();
    assert!(
        image
            .pixels()
            .filter(|p| p[3] > 200)
            .all(|p| p[0] > 250 && p[1] < 5 && p[2] < 5)
    );
    for bad in [
        "<svg><script>alert(1)</script></svg>",
        "<!DOCTYPE svg><svg/>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><image href=\"file:///etc/passwd\"/></svg>",
    ] {
        assert!(svg::tree(bad, None).is_err());
    }
}
#[test]
fn markdown_word_and_image_conversion_roundtrip() {
    let pdf = convert::text_to_pdf(
        "# Project notes\nİstanbul & Paris <2026>\n- One\n- Two",
        true,
    )
    .unwrap();
    let text = convert::export_text(&pdf, &[0], convert::Format::Text).unwrap();
    assert!(
        String::from_utf8(text)
            .unwrap()
            .contains("İstanbul & Paris <2026>")
    );
    let md = convert::export_text(&pdf, &[0], convert::Format::Markdown).unwrap();
    assert!(String::from_utf8(md).unwrap().starts_with("## Page 1"));
    let word = convert::export_text(&pdf, &[0], convert::Format::Word).unwrap();
    let text = convert::docx_text(&word).unwrap();
    assert!(text.contains("İstanbul & Paris <2026>"));
    let back = convert::text_to_pdf(&text, true).unwrap();
    assert!(
        String::from_utf8(convert::export_text(&back, &[0], convert::Format::Text).unwrap())
            .unwrap()
            .contains("Project notes")
    );
    let images = convert::images_to_pdf(&[png(), png()], 144.).unwrap();
    assert_eq!(Document::load_mem(&images).unwrap().get_pages().len(), 2);
    assert_eq!(content::images(&images, 1).unwrap()[0].pixels, [32, 24]);
    let dir = tempfile::tempdir().unwrap();
    for format in [convert::Format::Png, convert::Format::Jpeg] {
        let dest = dir.path().join(format.extension());
        convert::export_images(&pdf, &[0], format, 72., &dest).unwrap();
        let bytes = std::fs::read(dest.join(format!("page-1.{}", format.extension()))).unwrap();
        assert_eq!(image::load_from_memory(&bytes).unwrap().width(), 595);
        assert!(convert::export_images(&pdf, &[0], format, 72., &dest).is_err());
    }
}
#[test]
fn new_blank_and_insert_blank_preserve_source() {
    let blank = convert::blank(595., 842.).unwrap();
    let output = mypdf_milestone0::pages::apply(
        &blank,
        &mypdf_milestone0::pages::Operation::InsertBlank {
            at: 1,
            width: 595.,
            height: 842.,
        },
    )
    .unwrap();
    assert_eq!(Document::load_mem(&output).unwrap().get_pages().len(), 2);
    assert_eq!(Document::load_mem(&blank).unwrap().get_pages().len(), 1);
    assert!(convert::export_text(&blank, &[0], convert::Format::Word).is_err());
}
#[test]
#[ignore]
fn milestone5_visual_proof() {
    let dir = std::path::PathBuf::from(
        std::env::var_os("MYPDF_M5_PROOF_DIR").expect("MYPDF_M5_PROOF_DIR"),
    );
    std::fs::create_dir_all(&dir).unwrap();
    let save = |name: &str, bytes: &[u8]| std::fs::write(dir.join(name), bytes).unwrap();
    save("source.pdf", fixture());
    save("forms.pdf", &form_fixture());
    save("replacement.png", &png());
    let text =
        document::replace_run(fixture(), 1, document::ORIGINAL, document::REPLACEMENT).unwrap();
    let images = content::images(&text, 1).unwrap();
    let edited = content::edit_image(
        &text,
        1,
        images[0].index,
        &content::ImageEdit::Replace(png()),
    )
    .unwrap();
    save("edited.pdf", &edited);
    let mut form = form_fixture();
    for (name, value) in [
        ("Name", "İstanbul & Paris"),
        ("Accept", "Yes"),
        ("Priority", "Second"),
        ("Country", "TR"),
    ] {
        let id = forms::fields(&form)
            .unwrap()
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .id;
        form = forms::fill(&form, id, value).unwrap();
    }
    save("filled.pdf", &form);
    let blank = convert::blank(595., 842.).unwrap();
    let mut a = ann::Annotation::new(
        0,
        [70., 120., 430., 300.],
        ann::Kind::Svg {
            source: svg::DEFAULT.into(),
            tint: true,
        },
        ann::Style::default(),
    );
    a.style.color = [0.85, 0.25, 0.4];
    a.rotation = 15.;
    save("svg.pdf", &ann::export(&blank, &[a]).unwrap());
    save("notes.md",b"# MyPDF conversions\nNative Rust desktop\n\n## Notes\n- Local files\n- Fresh output copies\n");
    let pdf = convert::text_to_pdf(
        &String::from_utf8(std::fs::read(dir.join("notes.md")).unwrap()).unwrap(),
        true,
    )
    .unwrap();
    save("markdown.pdf", &pdf);
    save(
        "notes.docx",
        &convert::export_text(&pdf, &[0], convert::Format::Word).unwrap(),
    );
    save(
        "exported.md",
        &convert::export_text(&pdf, &[0], convert::Format::Markdown).unwrap(),
    );
    save(
        "images.pdf",
        &convert::images_to_pdf(&[png(), png()], 144.).unwrap(),
    );
}

#[test]
fn added_text_embeds_selected_styles_and_preserves_alignment() {
    use mypdf_milestone0::{
        content, convert,
        text_style::{self, Alignment, TextFormat},
    };
    let face = text_style::fonts()
        .iter()
        .find(|f| f.bold && f.italic)
        .expect("Installed bold italic TrueType face");
    let blank = convert::blank(595., 842.).unwrap();
    let mut xs = vec![];
    for alignment in [Alignment::Left, Alignment::Center, Alignment::Right] {
        let bytes = content::add_text_styled(
            &blank,
            1,
            "Styled text",
            [50., 60., 350., 90.],
            24.,
            [0.1, 0.3, 0.7],
            &TextFormat {
                font: Some(face.path.clone()),
                alignment,
            },
        )
        .unwrap();
        let doc = lopdf::Document::load_mem(&bytes).unwrap();
        let page = *doc.get_pages().get(&1).unwrap();
        let ops = doc.get_and_decode_page_content(page).unwrap().operations;
        let tm = ops.iter().find(|o| o.operator == "Tm").unwrap();
        xs.push(tm.operands[4].as_float().unwrap());
        assert!(doc.extract_text(&[1]).unwrap().contains("Styled text"));
        let descriptor = doc
            .objects
            .values()
            .filter_map(|o| o.as_dict().ok())
            .find(|d| d.has(b"FontFile2"))
            .unwrap();
        assert!(descriptor.get(b"ItalicAngle").unwrap().as_float().unwrap() < 0.);
        let font = doc
            .get_object(
                descriptor
                    .get(b"FontFile2")
                    .unwrap()
                    .as_reference()
                    .unwrap(),
            )
            .unwrap()
            .as_stream()
            .unwrap();
        // Only the glyphs used are embedded, as a tagged subset of the chosen face.
        let program = font.get_plain_content().unwrap();
        assert!(program.len() * 4 < std::fs::read(&face.path).unwrap().len());
        assert!(ttf_parser::Face::parse(&program, 0).is_ok());
        let name = descriptor.get(b"FontName").unwrap().as_name().unwrap();
        assert_eq!(name[6], b'+', "subset fonts carry a six-letter tag");
    }
    assert_eq!(xs[0], 50.);
    assert!(xs[1] > xs[0] && xs[2] > xs[1]);
    let text = "One  two three four five six seven eight nine ten eleven twelve";
    let formatted = content::add_text_styled(
        &blank,
        1,
        text,
        [50., 100., 250., 150.],
        24.,
        [0.1, 0.3, 0.7],
        &TextFormat {
            font: Some(face.path.clone()),
            alignment: Alignment::Justified,
        },
    )
    .unwrap();
    let doc = lopdf::Document::load_mem(&formatted).unwrap();
    let ops = doc
        .get_and_decode_page_content(*doc.get_pages().get(&1).unwrap())
        .unwrap()
        .operations;
    assert!(ops.iter().any(|o| o.operator == "TJ"));
    assert!(doc.extract_text(&[1]).unwrap().contains("One  two"));
    if let Some(root) = std::env::var_os("MYPDF_DRAW_PROOF_DIR") {
        std::fs::write(
            std::path::Path::new(&root).join("formatted-text.pdf"),
            formatted,
        )
        .unwrap();
    }
}

#[test]
fn images_are_added_as_page_content_where_they_are_displayed() {
    let source = document::fixture(&document::default_font().unwrap()).unwrap();
    let mut png = vec![];
    image::RgbaImage::from_fn(40, 20, |x, _| image::Rgba([x as u8 * 6, 90, 200, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let mut bytes = source.clone();
    // Page 2 is rotated; the box is in displayed coordinates on both pages.
    for page in [1, 2] {
        let before = content::images(&bytes, page).unwrap().len();
        let rect = [100., 120., 300., 220.];
        bytes = content::add_image(&bytes, page, rect, &png).unwrap();
        let images = content::images(&bytes, page).unwrap();
        assert_eq!(images.len(), before + 1);
        assert!(
            images
                .iter()
                .any(|i| i.rect.iter().zip(rect).all(|(a, b)| (a - b).abs() < 0.5)),
            "page {page}: {:?}",
            images.iter().map(|i| i.rect).collect::<Vec<_>>()
        );
    }
    let temp = tempfile::tempdir().unwrap();
    assert!(document::render(&bytes, temp.path(), 1.).is_ok());
    assert!(content::add_image(&source, 1, [0., 0., 0.5, 10.], &png).is_err());
    assert!(content::add_image(&source, 1, [0., 0., 50., 50.], b"not an image").is_err());
}

#[test]
fn removed_image_background_survives_pdf_save_and_undo_restores_original() {
    use mypdf_milestone0::image_background::{self, Options};
    let mut png = vec![];
    image::RgbaImage::from_fn(20, 20, |x, y| {
        if x == y || x == 19 - y {
            image::Rgba([20, 30, 70, 255])
        } else {
            image::Rgba([255, 255, 255, 255])
        }
    })
    .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
    .unwrap();
    let blank = convert::blank(200., 200.).unwrap();
    let original = content::add_image(&blank, 1, [20., 20., 180., 180.], &png).unwrap();
    let index = content::images(&original, 1).unwrap()[0].index;
    let extracted = content::image_png(&original, 1, index).unwrap();
    let clear = image_background::remove(&extracted, Options::default()).unwrap();
    let edited = content::edit_image(
        &original,
        1,
        index,
        &content::ImageEdit::Replace(clear.clone()),
    )
    .unwrap();
    let index = content::images(&edited, 1).unwrap()[0].index;
    let saved = content::decode_image(&content::image_png(&edited, 1, index).unwrap()).unwrap();
    assert_eq!(saved.get_pixel(0, 1)[3], 0);
    assert_eq!(saved.get_pixel(0, 0)[3], 255);
    assert_eq!(saved.get_pixel(0, 0)[0], 20);
    let mut session = ann::Session::new(original.clone().into(), vec![], true);
    session.commit_document(edited.clone()).unwrap();
    session.undo();
    assert_eq!(&*session.source, &original);
    session.redo();
    assert_eq!(&*session.source, &edited);
    // Signature/artwork annotations keep alpha and the original is one Undo away.
    let mark = ann::Annotation::new(
        0,
        [20., 20., 180., 180.],
        ann::Kind::Image { png: Arc::new(png) },
        ann::Style::default(),
    );
    let mut session = ann::Session::new(blank.into(), vec![mark.clone()], true);
    let mut next = mark.clone();
    next.kind = ann::Kind::Image {
        png: Arc::new(clear),
    };
    session.commit(vec![next]).unwrap();
    let bytes = ann::export(&session.source, &session.annotations).unwrap();
    let marks = ann::load(&Document::load_mem(&bytes).unwrap());
    let ann::Kind::Image { png } = &marks[0].kind else {
        panic!("Image lost")
    };
    assert_eq!(content::decode_image(png).unwrap().get_pixel(0, 1)[3], 0);
    session.undo();
    assert_eq!(session.annotations, vec![mark]);
}

#[test]
fn copy_embedded_indexed_signature_with_decode_and_mask_excludes_page_content() {
    let blank = convert::blank(400., 300.).unwrap();
    let source = content::add_text(
        &blank,
        1,
        "SURROUNDING TEXT MUST NOT BE COPIED",
        [10., 10., 380., 50.],
        12.,
        [1., 0., 0.],
    )
    .unwrap();
    let mut png = vec![];
    image::RgbaImage::from_pixel(16, 8, image::Rgba([0, 0, 255, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let source = content::add_image(&source, 1, [100., 100., 260., 180.], &png).unwrap();
    let selected = content::images(&source, 1).unwrap().remove(0);
    let mut doc = Document::load_mem(&source).unwrap();
    let page = doc.get_pages()[&1];
    let resources = content::resources(&doc, page).unwrap();
    let objects = doc
        .dereference(resources.get(b"XObject").unwrap())
        .unwrap()
        .1
        .as_dict()
        .unwrap();
    let image_id = objects
        .get(selected.name.as_bytes())
        .unwrap()
        .as_reference()
        .unwrap();
    let stream = doc
        .get_object_mut(image_id)
        .unwrap()
        .as_stream_mut()
        .unwrap();
    // Common PDF signature encoding: palette + Decode + transparent color key.
    stream.dict.remove(b"Filter");
    stream.dict.remove(b"DecodeParms");
    stream.dict.set("BitsPerComponent", 8);
    stream.dict.set(
        "ColorSpace",
        vec![
            Object::Name(b"Indexed".to_vec()),
            Object::Name(b"DeviceRGB".to_vec()),
            1.into(),
            Object::String(
                vec![255, 255, 255, 0, 0, 200],
                lopdf::StringFormat::Hexadecimal,
            ),
        ],
    );
    stream.dict.set("Decode", vec![255.into(), 0.into()]);
    stream.dict.set("Mask", vec![255.into(), 255.into()]);
    stream.set_content((0..128).map(|p| if p % 16 < 8 { 0 } else { 255 }).collect());
    let source = content::serialize(&mut doc).unwrap();
    let index = content::images(&source, 1).unwrap()[0].index;
    let pixels = content::decode_image(&content::image_png(&source, 1, index).unwrap()).unwrap();
    assert_eq!(pixels.dimensions(), (16, 8));
    assert_eq!(pixels.get_pixel(3, 4).0, [0, 0, 200, 255]);
    assert_eq!(pixels.get_pixel(12, 4)[3], 0);
    assert!(
        pixels.pixels().all(|p| p[3] == 0 || p[0] == 0),
        "Neighbouring red text leaked into copy"
    );
    assert!(content::image_png(&source, 1, index + 100).is_err());
}
