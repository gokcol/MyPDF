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
