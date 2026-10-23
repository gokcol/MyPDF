use mypdf_milestone0::{
    annotations::{
        self as ann, Annotation, FillPattern, Kind, Primitive, Route, StrokeStyle, Style,
    },
    convert, viewer,
};
use std::sync::Arc;

fn callout() -> Annotation {
    let mut a = Annotation::new(
        0,
        [24., 36., 264., 156.],
        Kind::callout("Orhan: gözden geçirildi"),
        Style {
            color: [0.8, 0.1, 0.1],
            fill: Some([0.9, 0.95, 1.]),
            font_size: 12.,
            ..Default::default()
        },
    );
    if let Kind::Callout { text_color, .. } = &mut a.kind {
        *text_color = [0.05, 0.2, 0.65];
    }
    a
}

fn managed(doc: &lopdf::Document) -> &lopdf::Dictionary {
    doc.objects
        .values()
        .find_map(|v| {
            let d = v.as_dict().ok()?;
            d.has(b"MyPDFData").then_some(d)
        })
        .unwrap()
}

#[test]
fn callout_exports_standard_semantics_vector_text_and_editable_metadata() {
    let source = convert::blank(300., 220.).unwrap();
    let a = callout();
    let result = ann::export(&source, std::slice::from_ref(&a)).unwrap();
    let doc = lopdf::Document::load_mem(&result).unwrap();
    assert_eq!(ann::load(&doc), vec![a.clone()]);
    let d = managed(&doc);
    assert_eq!(d.get(b"Subtype").unwrap().as_name().unwrap(), b"FreeText");
    assert_eq!(d.get(b"IT").unwrap().as_name().unwrap(), b"FreeTextCallout");
    assert_eq!(d.get(b"LE").unwrap().as_name().unwrap(), b"OpenArrow");
    assert_eq!(d.get(b"CL").unwrap().as_array().unwrap().len(), 6);
    assert!(
        d.get(b"RD")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v.as_float().unwrap() >= 0.)
    );
    let da = String::from_utf8_lossy(d.get(b"DA").unwrap().as_str().unwrap());
    assert!(da.contains("0.05 0.2 0.65 rg"), "{da}");
    let ap = d
        .get(b"AP")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"N")
        .unwrap()
        .as_reference()
        .unwrap();
    let stream = doc.get_object(ap).unwrap().as_stream().unwrap();
    let content = String::from_utf8(stream.decompressed_content().unwrap()).unwrap();
    assert!(content.contains("0.8 0.1 0.1 RG"), "{content}");
    assert!(content.contains("0.05 0.2 0.65 rg BT"), "{content}");
    assert!(content.contains(" Tj ET"));
    assert!(
        !content.contains("/Im Do"),
        "Callout text and geometry must remain vector"
    );
    let pdf = hayro::hayro_syntax::Pdf::new(result).unwrap();
    let page = viewer::extract_page(
        &pdf.pages()[0],
        &hayro::hayro_interpret::InterpreterCache::new(),
    );
    assert!(
        page.text.contains("Orhan: gözden geçirildi"),
        "{}",
        page.text
    );
}

#[test]
fn callout_history_and_validation_keep_geometry_and_text_color() {
    let source = convert::blank(300., 220.).unwrap();
    let a = callout();
    let mut session = ann::Session::new(Arc::from(source), vec![], true);
    session.commit(vec![a.clone()]).unwrap();
    let mut b = a.clone();
    *b.kind.text_mut().unwrap() = "İkinci yorum".into();
    b.rect[0] += 5.;
    b.rect[2] += 5.;
    session.commit(vec![b.clone()]).unwrap();
    session.undo();
    assert_eq!(session.annotations, vec![a]);
    session.redo();
    assert_eq!(session.annotations, vec![b.clone()]);
    b.validate().unwrap();
    if let Kind::Callout { box_rect, .. } = &mut b.kind {
        box_rect[0] = box_rect[2];
    }
    assert!(b.validate().is_err());
    let mut invalid_color = callout();
    if let Kind::Callout { text_color, .. } = &mut invalid_color.kind {
        text_color[0] = f32::NAN;
    }
    assert!(invalid_color.validate().is_err());
}

#[test]
fn callout_hatches_and_dashes_are_clipped_to_the_text_box() {
    for pattern in FillPattern::ALL {
        let mut a = callout();
        a.style.fill_pattern = pattern;
        a.style.stroke = StrokeStyle::Dashed;
        let Kind::Callout { box_rect, .. } = a.kind else {
            unreachable!()
        };
        let lo = a.local([box_rect[0], box_rect[1]]);
        let hi = a.local([box_rect[2], box_rect[3]]);
        let primitives = ann::primitives(&a);
        assert!(primitives.iter().any(|p| matches!(
            p,
            Primitive::Text {
                color: Some([0.05, 0.2, 0.65]),
                ..
            }
        )));
        let mut fill_count = 0;
        let mut border_count = 0;
        for primitive in primitives {
            if let Primitive::Path { points, fill, .. } = primitive {
                if fill {
                    fill_count += 1;
                    assert!(points.iter().all(|p| p[0] >= lo[0] - 0.01
                        && p[0] <= hi[0] + 0.01
                        && p[1] >= lo[1] - 0.01
                        && p[1] <= hi[1] + 0.01));
                } else if points.iter().all(|p| {
                    p[0] >= lo[0] - 0.01
                        && p[0] <= hi[0] + 0.01
                        && p[1] >= lo[1] - 0.01
                        && p[1] <= hi[1] + 0.01
                }) {
                    border_count += 1;
                }
            }
        }
        assert!(fill_count > 0);
        assert!(
            border_count > 4,
            "The callout border must honor its dashed stroke"
        );
    }
}

#[test]
fn routed_arrows_share_polyline_geometry_with_pdf_and_keep_heads_solid() {
    let source = convert::blank(300., 220.).unwrap();
    for route in [Route::Elbow, Route::Curved] {
        for (start, end) in [
            ([0., 0.], [1., 1.]),
            ([1., 1.], [0., 0.]),
            ([0.5, 0.], [0.5, 1.]),
        ] {
            let a = Annotation::new(
                0,
                [30., 40., 250., 170.],
                Kind::RoutedArrow { start, end, route },
                Style {
                    stroke: StrokeStyle::Dashed,
                    ..Default::default()
                },
            );
            let paths = ann::primitives(&a);
            assert!(paths.len() > 2);
            assert!(
                matches!(paths.last().unwrap(), Primitive::Path {points,fill:false,..} if points.len()==3)
            );
            let output = ann::export(&source, std::slice::from_ref(&a)).unwrap();
            let doc = lopdf::Document::load_mem(&output).unwrap();
            assert_eq!(ann::load(&doc), vec![a.clone()]);
            let d = managed(&doc);
            assert_eq!(d.get(b"Subtype").unwrap().as_name().unwrap(), b"PolyLine");
            assert_eq!(
                d.get(b"Vertices").unwrap().as_array().unwrap().len(),
                ann::routed_arrow(a.local(start), a.local(end), route).len() * 2
            );
            assert_eq!(
                d.get(b"LE").unwrap().as_array().unwrap()[1]
                    .as_name()
                    .unwrap(),
                b"OpenArrow"
            );
            viewer::render_page(&output, 0, 1.).unwrap();
        }
    }
}

#[test]
fn callout_and_routed_arrow_appearances_survive_cropped_rotated_pages() {
    let source = convert::blank(340., 260.).unwrap();
    for rotation in [0, 90, 180, 270] {
        let mut doc = lopdf::Document::load_mem(&source).unwrap();
        let page = doc.get_pages()[&1];
        let d = doc.get_dictionary_mut(page).unwrap();
        d.set(
            "CropBox",
            vec![20.into(), 20.into(), 320.into(), 240.into()],
        );
        d.set("Rotate", rotation);
        let mut original = vec![];
        doc.save_to(&mut original).unwrap();
        let mut a = callout();
        a.rect = [20., 30., 200., 145.];
        a.rotate_by(17.);
        a.flip_horizontal();
        let arrow = Annotation::new(
            0,
            [30., 155., 190., 200.],
            Kind::RoutedArrow {
                start: [0., 1.],
                end: [1., 0.],
                route: Route::Curved,
            },
            Style::default(),
        );
        let output = ann::export(&original, &[a.clone(), arrow.clone()]).unwrap();
        assert_eq!(
            ann::load(&lopdf::Document::load_mem(&output).unwrap()),
            vec![a, arrow]
        );
        let (_, pixels) = viewer::render_page(&output, 0, 1.).unwrap();
        assert!(pixels.chunks_exact(4).any(|p| p[0] < 220 && p[2] > 100));
    }
}

#[test]
fn empty_callout_still_exports_as_an_editable_box_and_leader() {
    let source = convert::blank(300., 220.).unwrap();
    let mut a = callout();
    a.kind.text_mut().unwrap().clear();
    let output = ann::export(&source, std::slice::from_ref(&a)).unwrap();
    assert_eq!(
        ann::load(&lopdf::Document::load_mem(&output).unwrap()),
        vec![a]
    );
    viewer::render_page(&output, 0, 1.).unwrap();
}
