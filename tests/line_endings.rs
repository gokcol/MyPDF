use mypdf_milestone0::{
    annotations::{
        self as ann, Annotation, Kind, LineEnd, LineEndings, Point2, Primitive, Route, StrokeStyle,
        Style,
    },
    convert, viewer,
};
use std::sync::Arc;

const ENDINGS: [LineEnd; 9] = [
    LineEnd::None,
    LineEnd::OpenArrow,
    LineEnd::ClosedArrow,
    LineEnd::FilledArrow,
    LineEnd::Circle,
    LineEnd::Square,
    LineEnd::Diamond,
    LineEnd::Bar,
    LineEnd::Slash,
];

fn line_kinds(start: Point2, end: Point2) -> Vec<Kind> {
    let mut kinds = vec![
        Kind::Line {
            start,
            end,
            arrow: false,
        },
        Kind::Line {
            start,
            end,
            arrow: true,
        },
        Kind::DoubleLine { start, end },
        Kind::DoubleArrow { start, end },
    ];
    for route in [Route::Elbow, Route::Stepped, Route::Curved] {
        kinds.push(Kind::RoutedLine { start, end, route });
        kinds.push(Kind::RoutedArrow { start, end, route });
        kinds.push(Kind::RoutedDoubleArrow { start, end, route });
    }
    kinds
}

fn annotation(kind: Kind, start: LineEnd, end: LineEnd, size: f32) -> Annotation {
    Annotation::new(
        0,
        [45., 45., 245., 165.],
        kind,
        Style {
            color: [0.1, 0.3, 0.7],
            // Filled endings must follow the stroke, rather than a previous
            // shape tool's unrelated fill setting.
            fill: Some([0.9, 0.15, 0.1]),
            width: 2.,
            line_endings: Some(LineEndings { start, end, size }),
            ..Default::default()
        },
    )
}

fn managed(doc: &lopdf::Document) -> &lopdf::Dictionary {
    doc.objects
        .values()
        .find_map(|o| {
            let d = o.as_dict().ok()?;
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

fn appearance(doc: &lopdf::Document) -> lopdf::content::Content {
    let ap = managed(doc)
        .get(b"AP")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"N")
        .unwrap();
    let stream = doc.dereference(ap).unwrap().1.as_stream().unwrap();
    let content = stream
        .decompressed_content()
        .unwrap_or_else(|_| stream.content.clone());
    lopdf::content::Content::decode(&content).unwrap()
}

#[test]
fn old_saved_styles_keep_legacy_heads_until_endings_are_explicitly_changed() {
    let mut json = serde_json::to_value(Style::default()).unwrap();
    json.as_object_mut().unwrap().remove("line_endings");
    let old_style: Style = serde_json::from_value(json).unwrap();
    assert!(old_style.line_endings.is_none());
    for (kind, legacy_paths) in [
        (
            Kind::Line {
                start: [0., 0.],
                end: [1., 1.],
                arrow: false,
            },
            1,
        ),
        (
            Kind::Line {
                start: [0., 0.],
                end: [1., 1.],
                arrow: true,
            },
            2,
        ),
        (
            Kind::DoubleArrow {
                start: [0., 0.],
                end: [1., 1.],
            },
            3,
        ),
        (
            Kind::DoubleLine {
                start: [0., 0.],
                end: [1., 1.],
            },
            2,
        ),
    ] {
        let mut a = annotation(kind, LineEnd::None, LineEnd::None, 12.);
        a.style = old_style.clone();
        assert_eq!(ann::primitives(&a).len(), legacy_paths);
        let old_pdf = ann::export(&convert::blank(300., 220.).unwrap(), &[a.clone()]).unwrap();
        assert_eq!(
            ann::load(&lopdf::Document::load_mem(&old_pdf).unwrap()),
            vec![a.clone()]
        );
        a.style.line_endings = Some(LineEndings {
            start: LineEnd::None,
            end: LineEnd::None,
            size: 12.,
        });
        assert_eq!(
            ann::primitives(&a).len(),
            if matches!(a.kind, Kind::DoubleLine { .. }) {
                2
            } else {
                1
            }
        );
    }
}

#[test]
fn endpoint_styles_and_size_survive_save_reopen_copy_and_undo_redo() {
    let source = convert::blank(300., 220.).unwrap();
    for kind in line_kinds([0., 0.], [1., 1.]) {
        let a = annotation(kind, LineEnd::Circle, LineEnd::FilledArrow, 18.);
        let mut session = ann::Session::new(Arc::from(source.clone()), vec![a.clone()], true);
        let mut changed = a.clone();
        changed.style.line_endings = Some(LineEndings {
            start: LineEnd::Diamond,
            end: LineEnd::Bar,
            size: 24.,
        });
        session.commit(vec![changed.clone()]).unwrap();
        session.undo();
        assert_eq!(session.annotations, vec![a]);
        session.redo();
        assert_eq!(session.annotations, vec![changed.clone()]);
        let bytes = session.snapshot_bytes().unwrap();
        assert_eq!(
            ann::load(&lopdf::Document::load_mem(&bytes).unwrap()),
            vec![changed.clone()]
        );
        let copy = changed.duplicate();
        assert_ne!(copy.id, changed.id);
        assert_eq!(copy.kind, changed.kind);
        assert_eq!(copy.style.line_endings, changed.style.line_endings);
        assert_eq!(
            serde_json::from_str::<Annotation>(&serde_json::to_string(&copy).unwrap()).unwrap(),
            copy
        );
    }
}

#[test]
fn ending_sizes_are_validated_before_export_and_do_not_scale_with_stroke_width() {
    let kind = Kind::Line {
        start: [0., 0.5],
        end: [1., 0.5],
        arrow: false,
    };
    for size in [f32::NAN, f32::INFINITY, -1., 0., 0.49, 257.] {
        let a = annotation(kind.clone(), LineEnd::None, LineEnd::OpenArrow, size);
        assert!(a.validate().is_err(), "Invalid ending size {size}");
    }
    for size in [6., 12., 24.] {
        let mut a = annotation(kind.clone(), LineEnd::None, LineEnd::OpenArrow, size);
        a.validate().unwrap();
        let head = |a: &Annotation| {
            let primitives = ann::primitives(a);
            let Primitive::Head { points, .. } = primitives.last().unwrap() else {
                panic!("Expected an open arrow head")
            };
            points.clone()
        };
        let before = head(&a);
        a.style.width = 7.;
        assert_eq!(head(&a), before, "Ending size is measured in page points");
    }
}

#[test]
fn every_line_kind_exports_each_ending_as_editable_vector_geometry() {
    let source = convert::blank(300., 220.).unwrap();
    for kind in line_kinds([0., 0.1], [1., 0.9]) {
        for ending in ENDINGS {
            let a = annotation(kind.clone(), ending, ending, 14.);
            a.validate().unwrap();
            assert_eq!(ann::line_endings(&a.kind, &a.style), a.style.line_endings);
            let primitives = ann::primitives(&a);
            let heads: Vec<_> = primitives
                .iter()
                .filter(|p| matches!(p, Primitive::Head { .. }))
                .collect();
            assert_eq!(
                heads.len(),
                if ending == LineEnd::None { 0 } else { 2 },
                "{} / {:?}",
                kind.label(),
                ending
            );
            for head in heads {
                let Primitive::Head {
                    points,
                    closed,
                    fill,
                } = head
                else {
                    unreachable!()
                };
                assert!(points.iter().flatten().all(|v| v.is_finite()));
                assert_eq!(*fill, ending == LineEnd::FilledArrow);
                assert_eq!(
                    *closed,
                    matches!(
                        ending,
                        LineEnd::ClosedArrow
                            | LineEnd::FilledArrow
                            | LineEnd::Circle
                            | LineEnd::Square
                            | LineEnd::Diamond
                    )
                );
            }
            let bytes = ann::export(&source, std::slice::from_ref(&a)).unwrap();
            let doc = lopdf::Document::load_mem(&bytes).unwrap();
            assert_eq!(ann::load(&doc), vec![a.clone()]);
            let d = managed(&doc);
            if !matches!(kind, Kind::DoubleLine { .. }) {
                let native = match ending {
                    LineEnd::None => b"None".as_slice(),
                    LineEnd::OpenArrow => b"OpenArrow",
                    LineEnd::ClosedArrow | LineEnd::FilledArrow => b"ClosedArrow",
                    LineEnd::Circle => b"Circle",
                    LineEnd::Square => b"Square",
                    LineEnd::Diamond => b"Diamond",
                    LineEnd::Bar => b"Butt",
                    LineEnd::Slash => b"Slash",
                };
                let le = d.get(b"LE").unwrap().as_array().unwrap();
                assert_eq!(le.len(), 2);
                assert!(le.iter().all(|v| v.as_name().unwrap() == native));
            }
            let operators = appearance(&doc);
            assert!(
                !operators.operations.iter().any(|o| o.operator == "Do"),
                "Ending markers must stay vector"
            );
            if ending == LineEnd::FilledArrow {
                assert!(operators.operations.iter().any(|o| o.operator == "f"));
                if !matches!(kind, Kind::DoubleLine { .. }) {
                    let fill = numbers(d.get(b"IC").unwrap());
                    assert!(
                        fill.iter()
                            .zip(a.style.color)
                            .all(|(x, y)| (*x - y).abs() < 0.0001)
                    );
                }
            }
        }
    }
    assert!(ann::line_endings(&Kind::Rectangle, &Style::default()).is_none());
}

#[test]
fn open_heads_follow_each_endpoint_tangent_in_both_drawing_directions() {
    for (start, end) in [
        ([0., 0.], [1., 1.]),
        ([1., 1.], [0., 0.]),
        ([0.5, 0.], [0.5, 1.]),
        ([0., 0.5], [1., 0.5]),
    ] {
        for kind in line_kinds(start, end) {
            let a = annotation(kind, LineEnd::OpenArrow, LineEnd::OpenArrow, 16.);
            let center = match a.kind {
                Kind::RoutedLine { route, .. }
                | Kind::RoutedArrow { route, .. }
                | Kind::RoutedDoubleArrow { route, .. } => {
                    ann::routed_arrow(a.local(start), a.local(end), route)
                }
                _ => vec![a.local(start), a.local(end)],
            };
            for head in ann::primitives(&a).into_iter().filter_map(|p| match p {
                Primitive::Head { points, .. } => Some(points),
                _ => None,
            }) {
                assert_eq!(head.len(), 3);
                let tip = head[1];
                let distance = |p: Point2, q: Point2| (p[0] - q[0]).hypot(p[1] - q[1]);
                let (expected_tip, from) = if distance(tip, center[0]) < 0.01 {
                    (center[0], center[1])
                } else {
                    (*center.last().unwrap(), center[center.len() - 2])
                };
                assert!(
                    distance(tip, expected_tip) < 0.01,
                    "A marker must be centered at the endpoint"
                );
                let tangent = [expected_tip[0] - from[0], expected_tip[1] - from[1]];
                let rear = [
                    (head[0][0] + head[2][0]) / 2.,
                    (head[0][1] + head[2][1]) / 2.,
                ];
                let axis = [tip[0] - rear[0], tip[1] - rear[1]];
                assert!(
                    axis[0] * tangent[0] + axis[1] * tangent[1] > 0.,
                    "The head must point outward along the final segment"
                );
                let cross = axis[0] * tangent[1] - axis[1] * tangent[0];
                assert!(
                    cross.abs() < 0.02 * tangent[0].hypot(tangent[1]),
                    "{} head ignores its endpoint tangent",
                    a.kind.label()
                );
            }
        }
    }
}

#[test]
fn filled_endings_use_the_stroke_color_even_with_dashed_shaft_and_old_shape_fill() {
    let source = convert::blank(300., 220.).unwrap();
    for stroke in StrokeStyle::ALL {
        let mut a = annotation(
            Kind::RoutedArrow {
                start: [0., 0.],
                end: [1., 1.],
                route: Route::Curved,
            },
            LineEnd::None,
            LineEnd::FilledArrow,
            22.,
        );
        a.style.stroke = stroke;
        let heads: Vec<_> = ann::primitives(&a)
            .into_iter()
            .filter(|p| matches!(p, Primitive::Head { .. }))
            .collect();
        assert_eq!(
            heads.len(),
            1,
            "The shaft pattern must not fragment the head"
        );
        assert!(
            matches!(&heads[0], Primitive::Head { closed: true, fill: true, points } if points.len() == 3)
        );
        let bytes = ann::export(&source, std::slice::from_ref(&a)).unwrap();
        let doc = lopdf::Document::load_mem(&bytes).unwrap();
        let content = appearance(&doc);
        let mut fill = vec![];
        let mut filled = 0;
        for op in content.operations {
            if op.operator == "rg" {
                fill = op.operands.iter().map(|n| n.as_float().unwrap()).collect();
            } else if op.operator == "f" {
                assert_eq!(fill.len(), 3);
                assert!(
                    fill.iter()
                        .zip(a.style.color)
                        .all(|(x, y)| (*x - y).abs() < 0.0001),
                    "The marker inherited the shape's red fill"
                );
                filled += 1;
            }
        }
        assert!(filled > 0);
        viewer::render_page(&bytes, 0, 1.).unwrap();
    }
}

#[test]
fn large_endpoint_markers_remain_inside_export_bounds_after_rotation_and_reflection() {
    let source = convert::blank(360., 300.).unwrap();
    for ending in ENDINGS
        .into_iter()
        .filter(|ending| *ending != LineEnd::None)
    {
        let mut a = annotation(
            Kind::RoutedLine {
                start: [0., 0.],
                end: [1., 1.],
                route: Route::Stepped,
            },
            ending,
            ending,
            72.,
        );
        a.rect = [145., 125., 155., 135.];
        a.rotate_by(31.);
        a.flip_horizontal();
        let bounds = a.bounds();
        for primitive in ann::primitives(&a) {
            if let Primitive::Head { points, .. } = &primitive {
                let probe = a.rotated(points[0]);
                assert!(
                    a.hit_test(probe, 0.1),
                    "Ending geometry must be selectable outside the shaft rectangle"
                );
            }
            let points = match primitive {
                Primitive::Path { points, .. } | Primitive::Head { points, .. } => points,
                Primitive::Dot { center } => vec![center],
                _ => panic!("A line cannot contain raster or text primitives"),
            };
            for point in points.into_iter().map(|p| a.rotated(p)) {
                assert!(
                    point[0] >= bounds[0] - 0.001
                        && point[0] <= bounds[2] + 0.001
                        && point[1] >= bounds[1] - 0.001
                        && point[1] <= bounds[3] + 0.001,
                    "Marker clipped from interactive bounds"
                );
            }
        }
        let bytes = ann::export(&source, std::slice::from_ref(&a)).unwrap();
        let doc = lopdf::Document::load_mem(&bytes).unwrap();
        assert_eq!(ann::load(&doc), vec![a]);
        let rect = numbers(managed(&doc).get(b"Rect").unwrap());
        assert!(rect[0] <= bounds[0] && rect[2] >= bounds[2]);
        assert!(rect[1] <= 300. - bounds[3] && rect[3] >= 300. - bounds[1]);
        viewer::render_page(&bytes, 0, 1.).unwrap();
    }
}
