use mypdf_milestone0::{
    annotations::{self as ann, Annotation, Kind, Style},
    convert, viewer,
};

fn appearance(doc: &lopdf::Document) -> &lopdf::Stream {
    let annot = doc
        .objects
        .values()
        .find_map(|o| {
            let d = o.as_dict().ok()?;
            d.has(b"MyPDFData").then_some(d)
        })
        .unwrap();
    let id = annot
        .get(b"AP")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"N")
        .unwrap()
        .as_reference()
        .unwrap();
    doc.get_object(id).unwrap().as_stream().unwrap()
}

#[test]
fn overflowing_text_saves_without_shrinking_clipping_or_losing_editability() {
    let source = convert::blank(400., 400.).unwrap();
    let a = Annotation::new(
        0,
        [40., 40., 200., 48.],
        Kind::Text {
            text: "First line\nLast line".into(),
        },
        Style {
            font_size: 14.,
            ..Default::default()
        },
    );
    let output = ann::export(&source, std::slice::from_ref(&a)).unwrap();
    let doc = lopdf::Document::load_mem(&output).unwrap();
    assert_eq!(ann::load(&doc), vec![a.clone()]);
    let stream = appearance(&doc);
    let content = String::from_utf8(stream.decompressed_content().unwrap()).unwrap();
    assert!(
        content.contains("/UF 14 Tf"),
        "Save must preserve font size"
    );
    assert_eq!(
        content.matches(" Tj").count(),
        "First lineLast line".chars().count()
    );
    let ([w, _], rgba) = viewer::render_page(&output, 0, 2.).unwrap();
    let visible_below_box = (110..155)
        .flat_map(|y| (80..260).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let i = (y * w + x) * 4;
            rgba[i] < 160 && rgba[i + 3] > 0
        })
        .count();
    assert!(
        visible_below_box > 50,
        "The last line must render below the editable box, as in the editor"
    );
    // Repeated saves and autosave use the same export path and retain geometry.
    let again = ann::export(&output, std::slice::from_ref(&a)).unwrap();
    assert_eq!(
        ann::load(&lopdf::Document::load_mem(&again).unwrap()),
        vec![a]
    );
}

#[test]
fn long_words_whitespace_empty_text_and_rotated_text_all_save() {
    let source = convert::blank(400., 400.).unwrap();
    for text in [
        "Supercalifragilisticexpialidocious",
        "  AV  İstanbul\tWelt\n\nEnd",
        "",
    ] {
        for rotation in [0., 37., 90.] {
            for kind in [
                Kind::Text { text: text.into() },
                Kind::Stamp { text: text.into() },
                Kind::callout(text),
            ] {
                let mut a = Annotation::new(
                    0,
                    [140., 140., 200., 155.],
                    kind,
                    Style {
                        font_size: 14.,
                        ..Default::default()
                    },
                );
                a.rotation = rotation;
                a.flip_x = rotation == 37.;
                let output = ann::export(&source, std::slice::from_ref(&a)).unwrap();
                let doc = lopdf::Document::load_mem(&output).unwrap();
                assert_eq!(ann::load(&doc), vec![a]);
                let stream = appearance(&doc);
                let content = String::from_utf8(stream.decompressed_content().unwrap()).unwrap();
                assert_eq!(
                    content.matches(" Tj").count(),
                    text.replace('\t', "    ")
                        .chars()
                        .filter(|c| !c.is_control())
                        .count()
                );
                for value in stream.dict.get(b"BBox").unwrap().as_array().unwrap() {
                    assert!(value.as_float().unwrap().is_finite());
                }
            }
        }
    }
}

#[test]
fn annotation_font_family_and_face_are_embedded_and_survive_reopening() {
    use mypdf_milestone0::text_style::{self, AnnotationFont};
    let source = convert::blank(400., 400.).unwrap();
    let faces = text_style::fonts();
    assert!(!faces.is_empty());
    for face in faces.iter().filter(|f| f.bold || f.italic).take(6) {
        for kind in [
            Kind::Text {
                text: "İstanbul – Grüße".into(),
            },
            Kind::callout("İstanbul – Grüße"),
        ] {
            let a = Annotation::new(
                0,
                [40., 40., 300., 140.],
                kind,
                Style {
                    font: Some(Box::new(AnnotationFont::from_face(face))),
                    ..Default::default()
                },
            );
            let bytes = ann::export(&source, std::slice::from_ref(&a)).unwrap();
            let doc = lopdf::Document::load_mem(&bytes).unwrap();
            assert_eq!(ann::load(&doc), vec![a.clone()]);
            let resources = appearance(&doc)
                .dict
                .get(b"Resources")
                .unwrap()
                .as_dict()
                .unwrap();
            let font_id = resources
                .get(b"Font")
                .unwrap()
                .as_dict()
                .unwrap()
                .get(b"UF")
                .unwrap()
                .as_reference()
                .unwrap();
            let font = doc.get_dictionary(font_id).unwrap();
            let descendant = font.get(b"DescendantFonts").unwrap().as_array().unwrap()[0]
                .as_reference()
                .unwrap();
            let descriptor = doc
                .get_dictionary(descendant)
                .unwrap()
                .get(b"FontDescriptor")
                .unwrap()
                .as_reference()
                .unwrap();
            let file = doc
                .get_dictionary(descriptor)
                .unwrap()
                .get(b"FontFile2")
                .unwrap()
                .as_reference()
                .unwrap();
            let embedded = doc
                .get_object(file)
                .unwrap()
                .as_stream()
                .unwrap()
                .decompressed_content()
                .unwrap();
            let embedded_face = ttf_parser::Face::parse(&embedded, 0).unwrap();
            let original = std::fs::read(&face.path).unwrap();
            let original_face = ttf_parser::Face::parse(&original, 0).unwrap();
            assert_eq!(embedded_face.is_bold(), original_face.is_bold());
            assert_eq!(embedded_face.is_italic(), original_face.is_italic());
            assert_eq!(embedded_face.units_per_em(), original_face.units_per_em());
            let annot = doc
                .objects
                .values()
                .filter_map(|o| o.as_dict().ok())
                .find(|d| d.has(b"MyPDFData"))
                .unwrap();
            assert_eq!(
                annot
                    .get(b"DR")
                    .unwrap()
                    .as_dict()
                    .unwrap()
                    .get(b"Font")
                    .unwrap()
                    .as_dict()
                    .unwrap()
                    .get(b"UF")
                    .unwrap()
                    .as_reference()
                    .unwrap(),
                font_id
            );
            let (_, pixels) = viewer::render_page(&bytes, 0, 1.).unwrap();
            assert!(
                pixels
                    .chunks_exact(4)
                    .filter(|p| p[0] < 180 && p[3] > 0)
                    .count()
                    > 100
            );
            let again = ann::export(&bytes, std::slice::from_ref(&a)).unwrap();
            assert_eq!(
                ann::load(&lopdf::Document::load_mem(&again).unwrap()),
                vec![a]
            );
        }
    }
}
