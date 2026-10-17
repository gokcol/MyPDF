use lopdf::{Document, Object, Stream, dictionary};
use mypdf_milestone0::{
    annotations::{self, Annotation, Kind, Style},
    nup::{self, Layout},
    viewer,
};
fn fixture() -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    let parent = doc.new_object_id();
    let font =
        doc.add_object(dictionary! {"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica"});
    let mut kids = vec![];
    for i in 0..5 {
        let body = format!(
            "0.9 0.95 1 rg 0 0 200 140 re f 0.1 0.2 0.4 rg BT /F1 18 Tf 25 80 Td (PAGE {}) Tj ET 0.9 0.1 0.2 rg 25 30 30 20 re f",
            i + 1
        );
        let contents = doc.add_object(Stream::new(dictionary! {}, body.into_bytes()));
        kids.push(Object::Reference(doc.add_object(dictionary!{"Type"=>"Page","Parent"=>parent,"Contents"=>contents,"Rotate"=>(i%4*90) as i64,"CropBox"=>vec![10.into(),20.into(),190.into(),120.into()]})));
    }
    doc.objects.insert(parent,dictionary!{"Type"=>"Pages","Count"=>5,"Kids"=>kids,"MediaBox"=>vec![0.into(),0.into(),200.into(),140.into()],"Resources"=>dictionary!{"Font"=>dictionary!{"F1"=>font}}}.into());
    let root = doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>parent});
    doc.trailer.set("Root", root);
    let mut bytes = vec![];
    doc.save_to(&mut bytes).unwrap();
    bytes
}
fn extracted(bytes: &[u8], page: usize) -> String {
    let pdf = hayro::hayro_syntax::Pdf::new(bytes.to_vec()).unwrap();
    viewer::extract_page(
        &pdf.pages()[page],
        &hayro::hayro_interpret::InterpreterCache::new(),
    )
    .text
}
#[test]
fn presets_create_correct_sheets_order_and_preserve_vector_text() {
    let source = fixture();
    for (columns, rows) in nup::PRESETS {
        let layout = Layout {
            columns,
            rows,
            ..Default::default()
        };
        let out = nup::export(&source, &[0, 1, 2, 3, 4], &layout).unwrap();
        let doc = Document::load_mem(&out).unwrap();
        assert_eq!(doc.get_pages().len(), 5usize.div_ceil(columns * rows));
        assert!(doc.objects.values().any(|o| o.as_stream().is_ok_and(|s| {
            s.dict
                .get(b"Subtype")
                .and_then(Object::as_name)
                .is_ok_and(|v| v == b"Form")
        })));
        for sheet in 0..doc.get_pages().len() {
            let text = extracted(&out, sheet);
            for i in sheet * columns * rows..((sheet + 1) * columns * rows).min(5) {
                assert!(text.contains(&format!("PAGE {}", i + 1)), "{text}");
            }
        }
    }
    let layout = Layout {
        columns: 2,
        rows: 1,
        header: "Review".into(),
        footer: "Sheet {page} of {pages}".into(),
        watermark: "COPY".into(),
        ..Default::default()
    };
    let out = nup::export(&source, &[4, 0, 3], &layout).unwrap();
    let first = extracted(&out, 0);
    let second = extracted(&out, 1);
    for s in ["PAGE 5", "PAGE 1", "Review", "Sheet 1 of 2", "COPY"] {
        assert!(first.contains(s), "{first}");
    }
    for s in ["PAGE 4", "Sheet 2 of 2"] {
        assert!(second.contains(s), "{second}");
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.pdf");
    std::fs::write(&path, &source).unwrap();
    assert!(mypdf_milestone0::write_new(&path, &out).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), source);
}
#[test]
fn one_up_render_matches_source_crop_rotation_and_annotation_appearance() {
    let source = annotations::export(
        &fixture(),
        &[Annotation::new(
            1,
            [25., 30., 60., 90.],
            Kind::Ellipse,
            Style {
                fill: Some([0.1, 0.6, 0.2]),
                ..Default::default()
            },
        )],
    )
    .unwrap();
    let geometry = annotations::page_geometry(&source).unwrap();
    for (page, (rect, _)) in geometry.iter().enumerate() {
        let layout = Layout {
            columns: 1,
            rows: 1,
            sheet: [rect[2], rect[3]],
            margins: [0.; 4],
            gap: 0.,
            ..Default::default()
        };
        let out = nup::export(&source, &[page], &layout).unwrap();
        let (size, before) = viewer::render_page(&source, page, 2.).unwrap();
        let (after_size, after) = viewer::render_page(&out, 0, 2.).unwrap();
        assert_eq!(size, after_size);
        let error: f64 = before
            .iter()
            .zip(&after)
            .map(|(a, b)| (*a as f64 - *b as f64).abs())
            .sum::<f64>()
            / before.len() as f64;
        assert!(error < 0.5, "rotation page {page}: {error}");
        assert!(annotations::load(&Document::load_mem(&out).unwrap()).is_empty());
    }
}
#[test]
fn trimming_and_invalid_layouts_are_bounded_and_do_not_mutate_source() {
    let source = fixture();
    let before = source.clone();
    let layout = Layout {
        columns: 1,
        rows: 1,
        trim: [10.; 4],
        sheet: [160., 80.],
        margins: [0.; 4],
        gap: 0.,
        ..Default::default()
    };
    let out = nup::export(&source, &[0], &layout).unwrap();
    assert_eq!(
        annotations::page_geometry(&out).unwrap()[0].0,
        [0., 0., 160., 80.]
    );
    let (_, original) = viewer::render_page(&source, 0, 1.).unwrap();
    let (_, trimmed) = viewer::render_page(&out, 0, 1.).unwrap();
    for y in 0..80 {
        for x in 0..160 {
            assert_eq!(
                &trimmed[(y * 160 + x) * 4..(y * 160 + x) * 4 + 4],
                &original[((y + 10) * 180 + x + 10) * 4..((y + 10) * 180 + x + 10) * 4 + 4]
            );
        }
    }
    for bad in [
        Layout {
            columns: 0,
            ..Default::default()
        },
        Layout {
            rows: 9,
            ..Default::default()
        },
        Layout {
            gap: f32::NAN,
            ..Default::default()
        },
        Layout {
            trim: [1000.; 4],
            ..Default::default()
        },
        Layout {
            margins: [1000.; 4],
            ..Default::default()
        },
    ] {
        assert!(nup::export(&source, &[0], &bad).is_err());
    }
    assert!(nup::export(&source, &[], &layout).is_err());
    assert!(nup::export(&source, &[5], &layout).is_err());
    assert_eq!(source, before);
}

#[test]
fn form_values_are_flattened_and_missing_visible_appearances_are_reported() {
    use mypdf_milestone0::{
        convert,
        form_design::{self, FieldType, Properties},
        forms,
    };
    let source = convert::blank(320., 320.).unwrap();
    let (source, id) = form_design::add(
        &source,
        0,
        [20., 30., 250., 60.],
        FieldType::Text,
        &Properties::new(FieldType::Text, "Name"),
    )
    .unwrap();
    let source = forms::fill(&source, id, "Ada Lovelace").unwrap();
    let layout = Layout {
        columns: 1,
        rows: 1,
        sheet: [320., 320.],
        margins: [0.; 4],
        gap: 0.,
        ..Default::default()
    };
    let out = nup::export(&source, &[0], &layout).unwrap();
    assert!(extracted(&out, 0).contains("Ada Lovelace"));
    assert!(forms::fields(&out).unwrap().is_empty());
    let mut doc = Document::load_mem(&source).unwrap();
    let note=doc.add_object(dictionary!{"Type"=>"Annot","Subtype"=>"Text","Rect"=>vec![10.into(),10.into(),40.into(),40.into()],"Contents"=>Object::string_literal("Don't lose this")});
    let page = doc.get_pages()[&1];
    doc.get_dictionary_mut(page)
        .unwrap()
        .set("Annots", vec![Object::Reference(note)]);
    let mut bytes = vec![];
    doc.save_to(&mut bytes).unwrap();
    assert!(
        nup::export(&bytes, &[0], &layout)
            .unwrap_err()
            .to_string()
            .contains("no saved appearance")
    );
}
