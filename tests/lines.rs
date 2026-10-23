use mypdf_milestone0::{
    annotations::{self as ann, Annotation, Kind, Primitive, Route, StrokeStyle, Style},
    convert, viewer,
};
use std::sync::Arc;

fn source() -> Vec<u8> {
    convert::blank(320., 260.).unwrap()
}
fn line(kind: Kind, width: f32) -> Annotation {
    Annotation::new(
        0,
        [50., 50., 250., 180.],
        kind,
        Style {
            width,
            color: [0.08, 0.3, 0.7],
            fill: Some([0.9, 0.1, 0.1]),
            ..Default::default()
        },
    )
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
fn numbers(object: &lopdf::Object) -> Vec<f32> {
    object
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_float().unwrap())
        .collect()
}

#[test]
fn new_line_kinds_keep_names_and_existing_line_metadata_remains_compatible() {
    for (kind, label) in [
        (
            Kind::DoubleLine {
                start: [0., 0.],
                end: [1., 1.],
            },
            "Double line",
        ),
        (
            Kind::RoutedLine {
                start: [0., 0.],
                end: [1., 1.],
                route: Route::Elbow,
            },
            "Elbow line",
        ),
        (
            Kind::RoutedLine {
                start: [0., 0.],
                end: [1., 1.],
                route: Route::Curved,
            },
            "Curved line",
        ),
    ] {
        assert_eq!(kind.label(), label);
        let a = line(kind, 2.);
        a.validate().unwrap();
        let output = ann::export(&source(), std::slice::from_ref(&a)).unwrap();
        assert_eq!(
            ann::load(&lopdf::Document::load_mem(&output).unwrap()),
            vec![a]
        );
    }
    let json = r#"{"Line":{"start":[0.0,0.0],"end":[1.0,1.0],"arrow":false}}"#;
    let kind: Kind = serde_json::from_str(json).unwrap();
    assert!(matches!(kind, Kind::Line { arrow: false, .. }));
    assert_eq!(serde_json::to_string(&kind).unwrap(), json);
}

#[test]
fn double_line_paths_are_parallel_separate_and_included_in_native_bounds() {
    for width in [0.5, 2., 30.] {
        for (start, end) in [
            ([0., 0.5], [1., 0.5]),
            ([0.5, 0.], [0.5, 1.]),
            ([0., 0.], [1., 1.]),
            ([1., 1.], [0., 0.]),
        ] {
            let a = line(Kind::DoubleLine { start, end }, width);
            let paths = ann::primitives(&a);
            assert_eq!(paths.len(), 2);
            let p = ann::double_line(a.local(start), a.local(end), width);
            for (primitive, pair) in paths.iter().zip(p) {
                assert!(
                    matches!(primitive,Primitive::Path {points,closed:false,fill:false} if points.as_slice() == pair)
                );
                assert!(
                    ((pair[1][0] - pair[0][0]) - (a.local(end)[0] - a.local(start)[0])).abs()
                        < 0.001
                );
                assert!(
                    ((pair[1][1] - pair[0][1]) - (a.local(end)[1] - a.local(start)[1])).abs()
                        < 0.001
                );
                let b = a.bounds();
                assert!(pair.iter().all(|v| {
                    v[0] >= b[0] - 0.001
                        && v[0] <= b[2] + 0.001
                        && v[1] >= b[1] - 0.001
                        && v[1] <= b[3] + 0.001
                }));
            }
            assert!(
                ((p[0][0][0] - p[1][0][0]).hypot(p[0][0][1] - p[1][0][1]) - 2. * width).abs()
                    < 0.001
            );
            let output = ann::export(&source(), std::slice::from_ref(&a)).unwrap();
            let doc = lopdf::Document::load_mem(&output).unwrap();
            let d = managed(&doc);
            assert_eq!(d.get(b"Subtype").unwrap().as_name().unwrap(), b"Ink");
            let ink = d.get(b"InkList").unwrap().as_array().unwrap();
            assert_eq!(ink.len(), 2);
            let rect = numbers(d.get(b"Rect").unwrap());
            for stroke in ink {
                let points = numbers(stroke);
                assert_eq!(points.len(), 4);
                assert!(points.chunks_exact(2).all(|p| p[0] >= rect[0] + width / 2.
                    && p[0] <= rect[2] - width / 2.
                    && p[1] >= rect[1] + width / 2.
                    && p[1] <= rect[3] - width / 2.));
            }
            assert!(d.get(b"LE").is_err());
            assert!(d.get(b"IC").is_err(), "Line tools must not acquire a fill");
        }
    }
}

#[test]
fn parallel_stroke_patterns_apply_to_both_lines_without_arrowheads() {
    for stroke in StrokeStyle::ALL {
        let mut a = line(
            Kind::DoubleLine {
                start: [0., 0.5],
                end: [1., 0.5],
            },
            2.,
        );
        a.style.stroke = stroke;
        let paths = ann::primitives(&a);
        let mut upper = false;
        let mut lower = false;
        for primitive in paths {
            let points = match primitive {
                Primitive::Path {
                    points,
                    fill: false,
                    ..
                } => points,
                Primitive::Dot { center } => vec![center],
                _ => panic!("A line must use unfilled strokes or stroke dots"),
            };
            let first = points[0];
            upper |= first[1] < 115.;
            lower |= first[1] > 115.;
            assert!(points.iter().all(|p| (p[1] - first[1]).abs()
                <= if stroke == StrokeStyle::Wave {
                    3.
                } else {
                    0.01
                }));
        }
        assert!(
            upper && lower,
            "Both parallel strokes must retain the selected pattern"
        );
        let output = ann::export(&source(), std::slice::from_ref(&a)).unwrap();
        viewer::render_page(&output, 0, 1.).unwrap();
    }
}

#[test]
fn routed_lines_export_open_native_polylines_without_arrowheads() {
    for route in [Route::Elbow, Route::Stepped, Route::Curved] {
        for (start, end) in [
            ([0., 0.], [1., 1.]),
            ([1., 1.], [0., 0.]),
            ([0.5, 0.], [0.5, 1.]),
        ] {
            let a = line(Kind::RoutedLine { start, end, route }, 2.);
            let paths = ann::primitives(&a);
            assert_eq!(paths.len(), 1);
            assert!(
                matches!(&paths[0],Primitive::Path {points,closed:false,fill:false} if *points==ann::routed_arrow(a.local(start),a.local(end),route))
            );
            let output = ann::export(&source(), std::slice::from_ref(&a)).unwrap();
            let doc = lopdf::Document::load_mem(&output).unwrap();
            assert_eq!(ann::load(&doc), vec![a.clone()]);
            let d = managed(&doc);
            assert_eq!(d.get(b"Subtype").unwrap().as_name().unwrap(), b"PolyLine");
            assert!(d.get(b"LE").is_err());
            assert_eq!(
                numbers(d.get(b"Vertices").unwrap()).len(),
                ann::routed_arrow(a.local(start), a.local(end), route).len() * 2
            );
            viewer::render_page(&output, 0, 1.).unwrap();
        }
    }
}

#[test]
fn very_short_and_coincident_parallel_lines_remain_finite_and_renderable() {
    for delta in [0., 0.00001, 0.1, 1.] {
        let mut a = line(
            Kind::DoubleLine {
                start: [0., 0.],
                end: [delta, delta],
            },
            30.,
        );
        a.rect = [100., 100., 101., 101.];
        a.validate().unwrap();
        for primitive in ann::primitives(&a) {
            let Primitive::Path { points, .. } = primitive else {
                panic!("Expected line paths")
            };
            assert!(points.iter().flatten().all(|v| v.is_finite()));
        }
        let output = ann::export(&source(), std::slice::from_ref(&a)).unwrap();
        let doc = lopdf::Document::load_mem(&output).unwrap();
        assert_eq!(ann::load(&doc), vec![a]);
        assert!(
            numbers(managed(&doc).get(b"Rect").unwrap())
                .iter()
                .all(|v| v.is_finite())
        );
        viewer::render_page(&output, 0, 1.).unwrap();
    }
    let invalid = line(
        Kind::RoutedLine {
            start: [f32::NAN, 0.],
            end: [1., 1.],
            route: Route::Elbow,
        },
        2.,
    );
    assert!(invalid.validate().is_err());
}

#[test]
fn line_history_and_export_preserve_crop_rotation_reflection_and_style() {
    for kind in [
        Kind::DoubleLine {
            start: [0., 0.],
            end: [1., 1.],
        },
        Kind::RoutedLine {
            start: [0., 0.],
            end: [1., 1.],
            route: Route::Elbow,
        },
        Kind::RoutedLine {
            start: [0., 0.],
            end: [1., 1.],
            route: Route::Curved,
        },
    ] {
        for rotate in [0, 90, 180, 270] {
            let mut doc = lopdf::Document::load_mem(&source()).unwrap();
            let page = doc.get_pages()[&1];
            let d = doc.get_dictionary_mut(page).unwrap();
            d.set(
                "CropBox",
                vec![20.into(), 20.into(), 300.into(), 240.into()],
            );
            d.set("Rotate", rotate);
            let mut original = vec![];
            doc.save_to(&mut original).unwrap();
            let mut a = line(kind.clone(), 2.);
            a.rect = [50., 50., 190., 180.];
            a.rotation = 17.;
            a.flip_horizontal();
            a.style.opacity = 0.65;
            a.style.stroke = StrokeStyle::DashDot;
            let mut session = ann::Session::new(Arc::from(original), vec![], true);
            session.commit(vec![a.clone()]).unwrap();
            session.undo();
            assert!(session.annotations.is_empty());
            session.redo();
            assert_eq!(session.annotations, vec![a.clone()]);
            let output = ann::export(&session.source, &session.annotations).unwrap();
            let saved = lopdf::Document::load_mem(&output).unwrap();
            assert_eq!(ann::load(&saved), vec![a]);
            assert!(managed(&saved).get(b"LE").is_err());
            let (_, pixels) = viewer::render_page(&output, 0, 1.).unwrap();
            assert!(pixels.chunks_exact(4).any(|p| p[0] < 220));
        }
    }
}

#[test]
fn stepped_routes_add_two_orthogonal_bends_without_changing_saved_elbows() {
    let start = [20., 30.];
    let end = [180., 130.];
    assert_eq!(
        ann::routed_arrow(start, end, Route::Elbow),
        vec![start, [180., 30.], end]
    );
    assert_eq!(
        ann::routed_arrow(start, end, Route::Stepped),
        vec![start, [100., 30.], [100., 130.], end]
    );
    assert_eq!(
        ann::routed_arrow(end, start, Route::Stepped),
        vec![end, [100., 130.], [100., 30.], start]
    );
    let elbow: Route = serde_json::from_str("\"Elbow\"").unwrap();
    let curved: Route = serde_json::from_str("\"Curved\"").unwrap();
    assert_eq!(elbow, Route::Elbow);
    assert_eq!(curved, Route::Curved);
    let a = line(
        Kind::RoutedLine {
            start: [0., 0.],
            end: [1., 1.],
            route: Route::Stepped,
        },
        2.,
    );
    assert_eq!(a.kind.label(), "Stepped line");
    let result = ann::export(&source(), std::slice::from_ref(&a)).unwrap();
    let doc = lopdf::Document::load_mem(&result).unwrap();
    assert_eq!(ann::load(&doc), vec![a]);
    assert_eq!(numbers(managed(&doc).get(b"Vertices").unwrap()).len(), 8);
    assert!(managed(&doc).get(b"LE").is_err());
}

#[test]
fn routed_double_arrows_keep_solid_heads_at_both_native_endpoints() {
    for route in [Route::Elbow, Route::Stepped, Route::Curved] {
        for (start, end) in [([0., 0.], [1., 1.]), ([1., 1.], [0., 0.])] {
            let mut a = line(Kind::RoutedDoubleArrow { start, end, route }, 2.);
            a.validate().unwrap();
            for stroke in [StrokeStyle::Solid, StrokeStyle::Dashed, StrokeStyle::Dotted] {
                a.style.stroke = stroke;
                let primitives = ann::primitives(&a);
                let heads = &primitives[primitives.len() - 2..];
                for (head, tip) in heads.iter().zip([a.local(end), a.local(start)]) {
                    assert!(
                        matches!(head,Primitive::Path {points,closed:false,fill:false} if points.len()==3 && points[1]==tip)
                    );
                }
                let output = ann::export(&source(), std::slice::from_ref(&a)).unwrap();
                let doc = lopdf::Document::load_mem(&output).unwrap();
                assert_eq!(ann::load(&doc), vec![a.clone()]);
                let d = managed(&doc);
                assert_eq!(d.get(b"Subtype").unwrap().as_name().unwrap(), b"PolyLine");
                let ends = d.get(b"LE").unwrap().as_array().unwrap();
                assert_eq!(ends.len(), 2);
                assert!(ends.iter().all(|v| v.as_name().unwrap() == b"OpenArrow"));
                assert_eq!(
                    numbers(d.get(b"Vertices").unwrap()).len(),
                    ann::routed_arrow(a.local(start), a.local(end), route).len() * 2
                );
                viewer::render_page(&output, 0, 1.).unwrap();
            }
        }
    }
}

#[test]
fn wide_legacy_arrow_defaults_can_become_explicit_endings_without_rejection() {
    use ann::{LineEnd, LineEndings};
    let mut a = line(
        Kind::Line {
            start: [0., 0.5],
            end: [1., 0.5],
            arrow: true,
        },
        30.,
    );
    let mut endings = a.line_endings().unwrap();
    assert_eq!(endings.size, 150.);
    assert!(a.style.line_endings.is_none());
    endings.start = LineEnd::Circle;
    a.style.line_endings = Some(endings);
    a.validate().unwrap();
    let output = ann::export(&source(), std::slice::from_ref(&a)).unwrap();
    assert_eq!(
        ann::load(&lopdf::Document::load_mem(&output).unwrap()),
        vec![a.clone()]
    );
    for size in [0.5, 256.] {
        a.style.line_endings = Some(LineEndings { size, ..endings });
        a.validate().unwrap();
    }
    for size in [0.49, 256.1, f32::NAN] {
        a.style.line_endings = Some(LineEndings { size, ..endings });
        assert!(a.validate().is_err());
    }
}

#[test]
fn explicit_parallel_heads_remain_solid_in_native_ink_with_dashed_shafts() {
    use ann::{LineEnd, LineEndings};
    let mut a = line(
        Kind::DoubleLine {
            start: [0., 0.5],
            end: [1., 0.5],
        },
        2.,
    );
    a.style.stroke = StrokeStyle::Dashed;
    a.style.line_endings = Some(LineEndings {
        start: LineEnd::Circle,
        end: LineEnd::OpenArrow,
        size: 16.,
    });
    let output = ann::export(&source(), std::slice::from_ref(&a)).unwrap();
    let doc = lopdf::Document::load_mem(&output).unwrap();
    let d = managed(&doc);
    assert_eq!(
        d.get(b"BS")
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"S")
            .unwrap()
            .as_name()
            .unwrap(),
        b"S"
    );
    let strokes = d.get(b"InkList").unwrap().as_array().unwrap();
    assert!(
        strokes.len() > 4,
        "Dashed shafts are explicit pieces; heads stay continuous"
    );
    assert_eq!(
        numbers(&strokes[strokes.len() - 2]).len(),
        6,
        "Open arrow is one continuous native stroke"
    );
    assert_eq!(
        numbers(&strokes[strokes.len() - 1]).len(),
        98,
        "Circle outline is a closed continuous native stroke"
    );
    assert_eq!(ann::load(&doc), vec![a]);
}
