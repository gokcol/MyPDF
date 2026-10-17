use mypdf_milestone0::{
    annotations::{self as ann, Annotation, Kind, Markup, Style, Symbol},
    assets, document,
};
use std::sync::Arc;

fn source() -> Vec<u8> {
    document::fixture(&document::default_font().unwrap()).unwrap()
}

#[test]
fn repeated_exports_prune_superseded_appearances() {
    let entries = samples();
    let first = ann::export(&source(), &entries).unwrap();
    let mut latest = first.clone();
    for _ in 0..5 {
        latest = ann::export(&latest, &entries).unwrap();
    }
    assert!(
        latest.len() < first.len() + 4096,
        "Superseded appearances must not accumulate"
    );
    assert_eq!(
        ann::load(&lopdf::Document::load_mem(&latest).unwrap()),
        entries
    );
}

#[test]
fn builtin_artwork_exports_as_editable_vector_annotations_without_library_writes() {
    let library = assets::builtin_assets();
    assert_eq!(
        library.iter().filter(|a| a.category == "Stamps").count(),
        26
    );
    let ids: std::collections::HashSet<_> = library.iter().map(|b| &b.asset.id).collect();
    assert_eq!(ids.len(), library.len());
    assert_eq!(assets::builtin_assets()[0].asset.id, library[0].asset.id);
    let entries: Vec<_> = library
        .iter()
        .map(|builtin| {
            builtin.asset.validate().unwrap();
            let x = 24.0;
            let y = 24.0;
            let width = 240.0 * builtin.asset.aspect.min(1.0);
            let height = width / builtin.asset.aspect;
            let mut annotation = Annotation::new(
                0,
                [x, y, x + width, y + height],
                builtin.asset.kind.clone(),
                builtin.asset.style.clone(),
            );
            annotation.asset_id = Some(builtin.asset.id.clone());
            annotation
        })
        .collect();
    let original = source();
    let output = ann::export(&original, &entries).unwrap();
    let saved = lopdf::Document::load_mem(&output).unwrap();
    assert_eq!(ann::load(&saved), entries);
    assert_eq!(
        saved.extract_text(&[1]).unwrap(),
        lopdf::Document::load_mem(&original)
            .unwrap()
            .extract_text(&[1])
            .unwrap()
    );
    for entry in &entries {
        assert!(!ann::primitives(entry).is_empty());
    }
}
fn samples() -> Vec<Annotation> {
    vec![
        Annotation::new(
            0,
            [72.0, 250.0, 310.0, 304.0],
            Kind::Text {
                text: "İstanbul: gözden geçirildi.".into(),
            },
            Style {
                font_size: 14.0,
                ..Default::default()
            },
        ),
        Annotation::new(
            0,
            [350.0, 250.0, 530.0, 300.0],
            Kind::Stamp {
                text: "REVIEWED".into(),
            },
            Style::default(),
        ),
        Annotation::new(
            0,
            [72.0, 320.0, 150.0, 372.0],
            Kind::Symbol(Symbol::Check),
            Style::default(),
        ),
        Annotation::new(
            0,
            [170.0, 320.0, 250.0, 372.0],
            Kind::Ellipse,
            Style::default(),
        ),
        Annotation::new(
            0,
            [270.0, 320.0, 350.0, 372.0],
            Kind::Rectangle,
            Style::default(),
        ),
        Annotation::new(
            0,
            [370.0, 320.0, 520.0, 372.0],
            Kind::Ink {
                strokes: vec![vec![
                    [0.0, 0.5],
                    [0.2, 0.1],
                    [0.4, 0.8],
                    [0.65, 0.3],
                    [1.0, 0.5],
                ]],
            },
            Style::default(),
        ),
        Annotation::new(
            0,
            [72.0, 400.0, 104.0, 432.0],
            Kind::Note {
                text: "Türkçe not".into(),
            },
            Style {
                color: [0.9, 0.7, 0.1],
                ..Default::default()
            },
        ),
        Annotation::new(
            1,
            [72.0, 190.0, 310.0, 244.0],
            Kind::Text {
                text: "Rotated page: İstanbul".into(),
            },
            Style::default(),
        ),
        Annotation::new(
            1,
            [72.0, 290.0, 250.0, 320.0],
            Kind::Markup {
                kind: Markup::Highlight,
                quads: vec![[0.0, 0.0, 1.0, 1.0]],
            },
            Style {
                opacity: 0.35,
                color: [1.0, 0.8, 0.1],
                ..Default::default()
            },
        ),
    ]
}

#[test]
fn annotation_appearances_roundtrip_and_preserve_existing_page_content() {
    let original = source();
    let annotations = samples();
    let output = ann::export(&original, &annotations).unwrap();
    let before = lopdf::Document::load_mem(&original).unwrap();
    let after = lopdf::Document::load_mem(&output).unwrap();
    assert_eq!(ann::load(&after), annotations);
    for page in [1, 2] {
        assert_eq!(
            before.extract_text(&[page]).unwrap(),
            after.extract_text(&[page]).unwrap()
        );
    }
    for (number, page) in after.get_pages() {
        if let Ok(list) = after
            .get_dictionary(page)
            .unwrap()
            .get(b"Annots")
            .and_then(lopdf::Object::as_array)
        {
            for a in list {
                let d = after.get_dictionary(a.as_reference().unwrap()).unwrap();
                assert!(
                    d.get(b"AP").unwrap().as_dict().unwrap().get(b"N").is_ok(),
                    "page {number}"
                );
            }
        }
    }
    let repeated = ann::export(&output, &annotations).unwrap();
    assert_eq!(
        ann::load(&lopdf::Document::load_mem(&repeated).unwrap()).len(),
        annotations.len()
    );
    let removed = ann::export(&output, &[]).unwrap();
    assert!(ann::load(&lopdf::Document::load_mem(&removed).unwrap()).is_empty());
    let rendered = hayro::hayro_syntax::Pdf::new(output).unwrap();
    let text = mypdf_milestone0::viewer::extract_page(
        &rendered.pages()[0],
        &hayro::hayro_interpret::InterpreterCache::new(),
    );
    assert!(
        text.text.contains("İstanbul: gözden geçirildi."),
        "{}",
        text.text
    );
}

#[test]
fn annotation_history_tracks_save_undo_and_redo() {
    let mut session = ann::Session::new(Arc::from(source()), vec![], true);
    session.commit(samples()).unwrap();
    let revision = session.revision();
    assert!(session.dirty());
    session.mark_saved(revision);
    assert!(!session.dirty());
    session.undo();
    assert!(session.annotations.is_empty());
    assert!(session.dirty());
    session.redo();
    assert!(!session.dirty());
    let mut next = session.annotations.clone();
    next.push(next[0].duplicate());
    session.commit(next).unwrap();
    assert!(session.dirty());
    session.undo();
    assert!(!session.dirty());
    let mut readonly = ann::Session::new(session.source.clone(), vec![], false);
    assert!(readonly.commit(samples()).is_err());
}

#[test]
fn annotations_cannot_rewrite_signed_or_certified_pdfs() {
    let mut doc = lopdf::Document::load_mem(&source()).unwrap();
    doc.add_object(lopdf::dictionary! { "Type" => "Sig", "ByteRange" => lopdf::Object::Array([0_i64, 1, 2, 3].map(lopdf::Object::Integer).to_vec()), "Contents" => lopdf::Object::string_literal("synthetic") });
    let mut signed = vec![];
    doc.save_to(&mut signed).unwrap();
    assert!(ann::export(&signed, &samples()).is_err());
    let mut doc = lopdf::Document::load_mem(&source()).unwrap();
    doc.catalog_mut()
        .unwrap()
        .set("Perms", lopdf::Dictionary::new());
    let mut certified = vec![];
    doc.save_to(&mut certified).unwrap();
    assert!(ann::export(&certified, &samples()).is_err());
}

#[test]
fn library_persists_assets_and_detects_another_writer() {
    let tmp = tempfile::tempdir().unwrap();
    let mut first = assets::Library::open(tmp.path().join("library")).unwrap();
    let mut other = assets::Library::open(first.root().to_owned()).unwrap();
    first.entries.push(assets::Asset::new(
        "Approved",
        Kind::Stamp {
            text: "APPROVED".into(),
        },
        Style::default(),
        3.0,
    ));
    first.save().unwrap();
    assert!(other.save().is_err());
    let reload = assets::Library::open(first.root().to_owned()).unwrap();
    assert_eq!(reload.entries[0].name, "Approved");
}

#[test]
fn svg_import_is_rasterized_and_rejects_external_file_references() {
    let tmp = tempfile::tempdir().unwrap();
    let safe = tmp.path().join("check.svg");
    std::fs::write(&safe, r##"<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128"><path d="M10 64 L45 100 L115 20" fill="none" stroke="#176B55" stroke-width="10"/></svg>"##).unwrap();
    let png = assets::import_artwork(&safe).unwrap();
    assert_eq!(assets::decode_png(&png).unwrap().width(), 128);
    std::fs::write(&safe, r#"<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128"><image href="/private/missing.png" width="128" height="128"/></svg>"#).unwrap();
    assert!(assets::import_artwork(&safe).is_err());
}

#[test]
fn indirect_annotation_arrays_roundtrip_and_foreign_notes_are_preserved() {
    let output = ann::export(&source(), &samples()).unwrap();
    let mut doc = lopdf::Document::load_mem(&output).unwrap();
    let page = doc.get_pages()[&1];
    let mut array = doc
        .get_dictionary(page)
        .unwrap()
        .get(b"Annots")
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    let foreign=doc.add_object(lopdf::dictionary! { "Type" => "Annot", "Subtype" => "Text", "Rect" => vec![10.into(),10.into(),30.into(),30.into()], "Contents" => lopdf::Object::string_literal("Foreign note") });
    array.push(foreign.into());
    let indirect = doc.add_object(array);
    doc.get_dictionary_mut(page)
        .unwrap()
        .set("Annots", indirect);
    let mut input = vec![];
    doc.save_to(&mut input).unwrap();
    assert_eq!(ann::load(&doc).len(), samples().len());
    let deleted = ann::export(&input, &[]).unwrap();
    let after = lopdf::Document::load_mem(&deleted).unwrap();
    let array = after
        .get_dictionary(page)
        .unwrap()
        .get(b"Annots")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(array.len(), 1);
    assert_eq!(
        after
            .get_dictionary(array[0].as_reference().unwrap())
            .unwrap()
            .get(b"Contents")
            .unwrap()
            .as_str()
            .unwrap(),
        b"Foreign note"
    );
}

#[test]
fn reader_modified_geometry_and_appearances_are_kept_instead_of_restoring_stale_metadata() {
    for change in ["geometry", "appearance"] {
        let output = ann::export(&source(), &samples()).unwrap();
        let mut doc = lopdf::Document::load_mem(&output).unwrap();
        let page = doc.get_pages()[&1];
        let annot = doc
            .get_dictionary(page)
            .unwrap()
            .get(b"Annots")
            .unwrap()
            .as_array()
            .unwrap()[0]
            .as_reference()
            .unwrap();
        if change == "geometry" {
            doc.get_dictionary_mut(annot)
                .unwrap()
                .set("Rect", vec![20.into(), 30.into(), 260.into(), 90.into()]);
        } else {
            let ap = doc
                .get_dictionary(annot)
                .unwrap()
                .get(b"AP")
                .unwrap()
                .as_dict()
                .unwrap()
                .get(b"N")
                .unwrap()
                .as_reference()
                .unwrap();
            doc.get_object_mut(ap)
                .unwrap()
                .as_stream_mut()
                .unwrap()
                .set_content(b"q 1 0 0 rg 0 0 100 100 re f Q".to_vec());
        }
        let managed = ann::load(&doc);
        assert_eq!(managed.len(), samples().len() - 1);
        let foreign = doc.get_dictionary(annot).unwrap().clone();
        let mut bytes = vec![];
        doc.save_to(&mut bytes).unwrap();
        let saved = ann::export(&bytes, &managed).unwrap();
        let after = lopdf::Document::load_mem(&saved).unwrap();
        assert_eq!(after.get_dictionary(annot).unwrap(), &foreign);
    }
}

#[test]
fn vector_appearance_uses_one_compound_fill_without_internal_seams() {
    let original = source();
    for builtin in assets::builtin_assets()
        .into_iter()
        .filter(|b| matches!(b.asset.kind, Kind::Vector { .. }))
    {
        let a = Annotation::new(
            0,
            [60.0, 80.0, 240.0, 180.0],
            builtin.asset.kind,
            Style {
                opacity: 0.6,
                ..builtin.asset.style
            },
        );
        let bytes = ann::export(&original, &[a]).unwrap();
        let doc = lopdf::Document::load_mem(&bytes).unwrap();
        let page = doc.get_dictionary(doc.get_pages()[&1]).unwrap();
        let values = page.get(b"Annots").unwrap().as_array().unwrap();
        let dictionary = doc.dereference(&values[0]).unwrap().1.as_dict().unwrap();
        let appearance = dictionary
            .get(b"AP")
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"N")
            .unwrap();
        let stream = doc.dereference(appearance).unwrap().1.as_stream().unwrap();
        let content = String::from_utf8(stream.decompressed_content().unwrap()).unwrap();
        assert_eq!(
            content.lines().filter(|line| *line == "f").count(),
            1,
            "{} must paint once for seamless transparency",
            builtin.asset.name
        );
    }
}

#[test]
fn polygons_and_double_arrows_round_trip_as_standard_pdf_annotations() {
    let kinds = [
        Kind::Polygon {
            points: vec![[0., 0.], [1., 0.2], [0.6, 1.]],
            closed: false,
        },
        Kind::Polygon {
            points: vec![[0., 0.], [1., 0.], [0.7, 1.], [0.3, 0.5]],
            closed: true,
        },
        Kind::DoubleArrow {
            start: [0., 1.],
            end: [1., 0.],
        },
    ];
    let entries: Vec<_> = kinds
        .into_iter()
        .enumerate()
        .map(|(i, kind)| {
            Annotation::new(
                0,
                [50. + i as f32 * 175., 430., 200. + i as f32 * 175., 540.],
                kind,
                Style {
                    fill: Some([0.6, 0.8, 0.9]),
                    ..Default::default()
                },
            )
        })
        .collect();
    let bytes = ann::export(&source(), &entries).unwrap();
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    assert_eq!(ann::load(&doc), entries);
    let dictionaries: Vec<_> = doc
        .objects
        .values()
        .filter_map(|o| o.as_dict().ok())
        .filter(|d| d.has(b"MyPDFData"))
        .collect();
    for (subtype, vertices) in [(b"PolyLine".as_slice(), 6), (b"Polygon".as_slice(), 8)] {
        let d = dictionaries
            .iter()
            .find(|d| d.get(b"Subtype").unwrap().as_name().unwrap() == subtype)
            .unwrap();
        assert_eq!(
            d.get(b"Vertices").unwrap().as_array().unwrap().len(),
            vertices
        );
    }
    let line = dictionaries
        .iter()
        .find(|d| d.get(b"Subtype").unwrap().as_name().unwrap() == b"Line")
        .unwrap();
    assert!(
        line.get(b"LE")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .all(|o| o.as_name().unwrap() == b"OpenArrow")
    );
    assert_eq!(ann::primitives(&entries[2]).len(), 3);
    assert!(
        Kind::Polygon {
            points: vec![[0., 0.], [1., 1.]],
            closed: true
        }
        .validate()
        .is_err()
    );
    if let Some(root) = std::env::var_os("MYPDF_DRAW_PROOF_DIR") {
        std::fs::write(std::path::Path::new(&root).join("draw-shapes.pdf"), bytes).unwrap();
    }
}

#[test]
fn stamp_library_updates_svg_and_images_without_changing_placed_marks() {
    let root = tempfile::tempdir().unwrap();
    let mut library = assets::Library::open(root.path().into()).unwrap();
    let mut asset = assets::Asset::new(
        "Approval",
        Kind::Svg {
            source: mypdf_milestone0::svg::DEFAULT.into(),
            tint: false,
        },
        Style::default(),
        2.,
    );
    asset.stamp = true;
    library.put(asset.clone()).unwrap();
    let placed = Annotation::new(
        0,
        [50., 400., 250., 500.],
        asset.kind.clone(),
        asset.style.clone(),
    );
    let before = ann::export(&source(), std::slice::from_ref(&placed)).unwrap();
    asset.kind = Kind::Svg {
        source: mypdf_milestone0::svg::DEFAULT.replace("#278ed9", "#1c6e59"),
        tint: false,
    };
    library.put(asset.clone()).unwrap();
    assert_eq!(library.entries.len(), 1);
    assert_eq!(
        assets::Library::open(root.path().into()).unwrap().entries[0],
        asset
    );
    let mut updated = placed.clone();
    updated.kind = asset.kind.clone();
    updated.rect = [100., 420., 400., 570.];
    let updated_bytes = ann::export(&source(), std::slice::from_ref(&updated)).unwrap();
    assert_eq!(
        ann::load(&lopdf::Document::load_mem(&updated_bytes).unwrap())[0],
        updated
    );
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(32, 16)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    asset.kind = Kind::Image {
        png: Arc::new(png.into_inner()),
    };
    asset.name = "Updated approval".into();
    library.put(asset.clone()).unwrap();
    assert_eq!(library.entries.len(), 1);
    assert_eq!(
        assets::Library::open(root.path().into()).unwrap().entries[0],
        asset
    );
    assert_eq!(
        ann::load(&lopdf::Document::load_mem(&before).unwrap())[0],
        placed
    );
    let mut invalid = asset.clone();
    invalid.name = String::new();
    assert!(library.put(invalid).is_err());
    assert_eq!(library.entries[0], asset);
    let mut legacy = serde_json::to_value(&asset).unwrap();
    legacy.as_object_mut().unwrap().remove("stamp");
    assert!(
        !serde_json::from_value::<assets::Asset>(legacy)
            .unwrap()
            .stamp
    );
    let response = format!(
        "Here is the SVG:\n```svg\n{}\n```",
        mypdf_milestone0::svg::DEFAULT
    );
    assert_eq!(
        mypdf_milestone0::svg::from_response(&response).unwrap(),
        mypdf_milestone0::svg::DEFAULT
    );
    assert!(mypdf_milestone0::svg::from_response("<svg><script>alert(1)</script></svg>").is_err());
    assert!(
        mypdf_milestone0::svg::stamp_prompt("Round APPROVED stamp")
            .contains("Round APPROVED stamp")
    );
}
