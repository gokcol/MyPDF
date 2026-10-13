use lopdf::{
    Document, Object, Stream,
    content::{Content, Operation},
    dictionary,
};
use mypdf_milestone0::{
    annotations as ann, content, convert, document, text_block::*, text_geometry,
};

fn block(runs: Vec<(usize, String)>) -> Replacement {
    Replacement {
        selection: Selection {
            runs,
            annotations: vec![],
        },
        text: "Edited together in İstanbul\nA new second line\nAnd a third".into(),
        rect: [48., 70., 440., 170.],
        size: 18.,
        color: [0.1, 0.3, 0.6],
        format: Default::default(),
    }
}

#[test]
fn rectangular_block_replaces_multiple_lines_and_text_boxes_with_one_undo_step() {
    let blank = convert::blank(595., 842.).unwrap();
    let source = content::add_text(
        &blank,
        1,
        "First line\nSecond line",
        [50., 70., 400., 70.],
        18.,
        [0.; 3],
    )
    .unwrap();
    let source = content::add_text(
        &source,
        1,
        "Keep this line",
        [50., 350., 400., 60.],
        18.,
        [0.; 3],
    )
    .unwrap();
    let annotation = ann::Annotation::new(
        0,
        [50., 160., 400., 200.],
        ann::Kind::Text {
            text: "A text box".into(),
        },
        ann::Style::default(),
    );
    let id = annotation.id.clone();
    let source = ann::export(&source, &[annotation]).unwrap();
    let before = text_geometry::runs(&source, 1).unwrap();
    let mut edit = block(vec![(0, "First line".into()), (1, "Second line".into())]);
    edit.selection.annotations.push((id, "A text box".into()));
    let output = replace(&source, 1, &edit).unwrap();
    if let Ok(dir) = std::env::var("MYPDF_BLOCK_PROOF_DIR") {
        let dir = std::path::Path::new(&dir);
        mypdf_milestone0::write_new(&dir.join("original.pdf"), &source).unwrap();
        mypdf_milestone0::write_new(&dir.join("edited.pdf"), &output).unwrap();
    }
    let after = text_geometry::runs(&output, 1).unwrap();
    assert_eq!(before[2].rect, after[2].rect);
    let text = convert::export_text(&output, &[0], convert::Format::Text).unwrap();
    let text = String::from_utf8(text).unwrap();
    assert!(
        text.contains("Edited together in İstanbul") && text.contains("And a third"),
        "{text}"
    );
    for old in ["First line", "Second line", "A text box"] {
        assert!(!text.contains(old), "old text survived: {old}");
    }
    assert!(ann::load(&Document::load_mem(&output).unwrap()).is_empty());
    let mut session = ann::Session::new(
        source.clone().into(),
        ann::load(&Document::load_mem(&source).unwrap()),
        true,
    );
    session.commit_document(output.clone()).unwrap();
    session.undo();
    assert_eq!(&*session.source, &source);
    assert_eq!(session.annotations.len(), 1);
    session.redo();
    assert_eq!(&*session.source, &output);
    // Overflow/stale selections fail before the caller commits any document bytes.
    edit.rect[3] = 20.;
    assert!(replace(&source, 1, &edit).is_err());
    edit.rect[3] = 170.;
    edit.selection.runs[0].1 = "Stale text".into();
    assert!(replace(&source, 1, &edit).is_err());
}

#[test]
fn block_edit_preserves_crop_rotation_other_pages_and_images() {
    let original = document::fixture(&document::default_font().unwrap()).unwrap();
    let original = mypdf_milestone0::pages::apply(
        &original,
        &mypdf_milestone0::pages::Operation::Crop {
            pages: vec![0, 1],
            margins: [12.; 4],
        },
    )
    .unwrap();
    for page in [1, 2] {
        let source = content::add_text(
            &original,
            page,
            "First selected line\nSecond selected line",
            [40., 300., 420., 75.],
            18.,
            [0.; 3],
        )
        .unwrap();
        let before = text_geometry::runs(&source, page).unwrap();
        let selected = before
            .iter()
            .enumerate()
            .filter(|(_, r)| r.text.contains("selected line"))
            .map(|(i, r)| (i, r.text.clone()))
            .collect();
        let mut edit = block(selected);
        edit.rect = [40., 300., 420., 110.];
        let result = replace(&source, page, &edit).unwrap();
        let a = Document::load_mem(&source).unwrap();
        let b = Document::load_mem(&result).unwrap();
        assert_eq!(
            a.get_page_content(a.get_pages()[&(3 - page)]),
            b.get_page_content(b.get_pages()[&(3 - page)])
        );
        let images = |d: &Document| {
            d.objects
                .values()
                .filter_map(|o| o.as_stream().ok())
                .filter(|s| {
                    s.dict
                        .get(b"Subtype")
                        .and_then(Object::as_name)
                        .is_ok_and(|v| v == b"Image")
                })
                .map(|s| s.content.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(images(&a), images(&b));
        let extracted =
            convert::export_text(&result, &[page as usize - 1], convert::Format::Text).unwrap();
        assert!(
            String::from_utf8(extracted)
                .unwrap()
                .contains("And a third")
        );
    }
}

#[test]
fn partial_shared_line_is_rejected_but_whole_line_can_be_reflowed() {
    let mut d = Document::load_mem(&convert::blank(595., 842.).unwrap()).unwrap();
    let page = d.get_pages()[&1];
    let font =
        d.add_object(dictionary! {"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica"});
    d.get_dictionary_mut(page)
        .unwrap()
        .set("Resources", dictionary! {"Font"=>dictionary!{"F1"=>font}});
    let operations = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 18.into()]),
        Operation::new(
            "Tm",
            vec![
                1.into(),
                0.into(),
                0.into(),
                1.into(),
                50.into(),
                700.into(),
            ],
        ),
        Operation::new("Tj", vec![Object::string_literal("First ")]),
        Operation::new("Tj", vec![Object::string_literal("second")]),
        Operation::new("ET", vec![]),
    ];
    let stream = d.add_object(Stream::new(
        dictionary! {},
        Content { operations }.encode().unwrap(),
    ));
    d.get_dictionary_mut(page).unwrap().set("Contents", stream);
    let source = content::serialize(&mut d).unwrap();
    let edit = block(vec![(0, "First ".into())]);
    assert!(
        replace(&source, 1, &edit)
            .unwrap_err()
            .to_string()
            .contains("complete line")
    );
    let mut edit = block(vec![(0, "First ".into()), (1, "second".into())]);
    assert!(replace(&source, 1, &edit).is_ok());
    edit.text.clear();
    let deleted = replace(&source, 1, &edit).unwrap();
    assert!(
        document::text_runs(&deleted, 1)
            .unwrap()
            .iter()
            .all(|r| r.text.is_empty())
    );
}
