use mypdf_milestone0::{
    annotations::{
        self as ann, Annotation, FillPattern, Kind, Markup, Primitive, StrokeStyle, Style,
    },
    convert, fill, viewer,
};

fn squiggly() -> Annotation {
    Annotation::new(
        0,
        [20., 20., 180., 36.],
        Kind::Markup {
            kind: Markup::Squiggly,
            quads: vec![[0., 0., 0.45, 1.], [0.46, 0., 1., 1.]],
        },
        Style {
            color: [0.05, 0.15, 0.85],
            width: 0.8,
            // An inherited drawing pattern must not turn a text wave into dots.
            stroke: StrokeStyle::Dotted,
            ..Default::default()
        },
    )
}

#[test]
fn squiggly_joins_glyph_quads_and_uses_one_continuous_vector_wave_per_line() {
    let mut mark = squiggly();
    let Kind::Markup { quads, .. } = &mut mark.kind else {
        unreachable!()
    };
    quads.push([0., 2., 0.6, 3.]);
    let paths = ann::primitives(&mark);
    assert_eq!(paths.len(), 2, "glyph quads must share the same cadence");
    for path in paths {
        let Primitive::Path {
            points,
            closed,
            fill,
        } = path
        else {
            panic!("a squiggly is a continuous vector line");
        };
        assert!(!closed && !fill);
        assert!(points.len() > 30);
        assert!(points.iter().flatten().all(|v| v.is_finite()));
        let baseline = points[0][1];
        assert!(points.iter().any(|p| p[1] > baseline + 0.9));
        assert!(points.iter().any(|p| p[1] < baseline - 0.9));
        assert!((points.last().unwrap()[1] - baseline).abs() < 0.001);
    }
    assert_eq!(mark.kind.label(), "Squiggly underline");
    assert!(mark.hit_test([40., 36.], 2.));
}

#[test]
fn squiggly_exports_as_native_pdf_markup_and_renders_sharply_after_reload() {
    let mark = squiggly();
    let bytes = ann::export(
        &convert::blank(220., 90.).unwrap(),
        std::slice::from_ref(&mark),
    )
    .unwrap();
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    assert_eq!(ann::load(&doc), vec![mark]);
    let annotation = doc
        .objects
        .values()
        .filter_map(|o| o.as_dict().ok())
        .find(|d| d.get(b"Subtype").and_then(lopdf::Object::as_name).ok() == Some(b"Squiggly"))
        .expect("standard /Squiggly annotation");
    assert_eq!(
        annotation
            .get(b"QuadPoints")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        16
    );
    assert_eq!(
        annotation
            .get(b"BS")
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"S")
            .unwrap()
            .as_name()
            .unwrap(),
        b"S"
    );
    let ap = annotation
        .get(b"AP")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"N")
        .unwrap();
    let stream = doc.dereference(ap).unwrap().1.as_stream().unwrap();
    let content = stream.decompressed_content().unwrap();
    let operators = lopdf::content::Content::decode(&content).unwrap();
    assert!(
        operators
            .operations
            .iter()
            .filter(|o| o.operator == "l")
            .count()
            > 100
    );
    assert!(!operators.operations.iter().any(|o| o.operator == "Do"));
    let (size, pixels) = viewer::render_page(&bytes, 0, 4.).unwrap();
    let mut above = 0;
    let mut below = 0;
    for y in 132..156 {
        for x in 80..720 {
            let p = &pixels[(y * size[0] + x) * 4..][..4];
            if p[0] < 100 && p[2] > 150 {
                if y < 144 {
                    above += 1;
                } else {
                    below += 1;
                }
            }
        }
    }
    assert!(
        above > 200 && below > 200,
        "visible wave above/below baseline: {above}/{below}"
    );
    let repeated = ann::export(&bytes, &ann::load(&doc)).unwrap();
    assert_eq!(
        ann::load(&lopdf::Document::load_mem(&repeated).unwrap()).len(),
        1
    );
    if let Some(root) = std::env::var_os("MYPDF_MARKUP_PROOF_DIR") {
        let root = std::path::PathBuf::from(root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("squiggly.pdf"), bytes).unwrap();
    }
}

#[test]
fn old_style_metadata_defaults_to_solid_fill_and_patterns_survive_history() {
    let mut json = serde_json::to_value(Style::default()).unwrap();
    json.as_object_mut().unwrap().remove("fill_pattern");
    assert_eq!(
        serde_json::from_value::<Style>(json).unwrap().fill_pattern,
        FillPattern::Solid
    );
    let mark = Annotation::new(
        0,
        [20., 20., 120., 100.],
        Kind::Rectangle,
        Style {
            fill: Some([0.2, 0.55, 0.85]),
            ..Default::default()
        },
    );
    let mut session = ann::Session::new(
        convert::blank(160., 140.).unwrap().into(),
        vec![mark.clone()],
        true,
    );
    let mut patterned = mark.clone();
    patterned.style.fill_pattern = FillPattern::Crosshatch;
    session.commit(vec![patterned.clone()]).unwrap();
    let saved = session.snapshot_bytes().unwrap();
    assert_eq!(
        ann::load(&lopdf::Document::load_mem(&saved).unwrap()),
        vec![patterned.clone()]
    );
    session.undo();
    assert_eq!(session.annotations, vec![mark]);
    session.redo();
    assert_eq!(session.annotations, vec![patterned]);
}

#[test]
fn pattern_fills_roundtrip_as_vectors_for_rectangles_ovals_and_concave_polygons() {
    let mut marks = vec![];
    for (row, kind) in [
        Kind::Rectangle,
        Kind::Ellipse,
        Kind::Polygon {
            points: vec![
                [0., 0.],
                [1., 0.],
                [1., 0.35],
                [0.35, 0.35],
                [0.35, 1.],
                [0., 1.],
            ],
            closed: true,
        },
    ]
    .into_iter()
    .enumerate()
    {
        for (col, fill_pattern) in FillPattern::ALL.into_iter().enumerate() {
            let x = 20. + col as f32 * 115.;
            let y = 20. + row as f32 * 120.;
            let a = Annotation::new(
                0,
                [x, y, x + 90., y + 90.],
                kind.clone(),
                Style {
                    color: [0.13, 0.2, 0.32],
                    fill: Some([0.25, 0.55, 0.85]),
                    fill_pattern,
                    width: 1.2,
                    ..Default::default()
                },
            );
            let primitives = ann::primitives(&a);
            let fills = primitives
                .iter()
                .filter(|p| {
                    matches!(
                        p,
                        Primitive::Path {
                            fill: true,
                            closed: true,
                            ..
                        }
                    )
                })
                .count();
            assert!(fills >= 1);
            if fill_pattern != FillPattern::Solid {
                assert!(fills > 5);
            }
            marks.push(a);
        }
    }
    let bytes = ann::export(&convert::blank(600., 385.).unwrap(), &marks).unwrap();
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    assert_eq!(ann::load(&doc), marks);
    for d in doc
        .objects
        .values()
        .filter_map(|o| o.as_dict().ok())
        .filter(|d| d.has(b"MyPDFData"))
    {
        let ap = d.get(b"AP").unwrap().as_dict().unwrap().get(b"N").unwrap();
        let stream = doc.dereference(ap).unwrap().1.as_stream().unwrap();
        let operators =
            lopdf::content::Content::decode(&stream.decompressed_content().unwrap()).unwrap();
        assert!(operators.operations.iter().any(|o| o.operator == "f"));
        assert!(
            !operators.operations.iter().any(|o| o.operator == "Do"),
            "hatches must remain vector artwork"
        );
    }
    let (size, pixels) = viewer::render_page(&bytes, 0, 1.).unwrap();
    for col in 1..5 {
        let x0 = 20 + col * 115;
        let blue = (25..105)
            .flat_map(|y| (x0 + 5..x0 + 85).map(move |x| (x, y)))
            .filter(|(x, y)| {
                let p = &pixels[(y * size[0] + x) * 4..][..4];
                u16::from(p[2]) > u16::from(p[0]) + 20 && p[2] > p[1]
            })
            .count();
        assert!(
            blue > 80,
            "pattern {col} must be visible at ordinary zoom: {blue}"
        );
        assert!(blue < 4000, "pattern {col} must retain blank space: {blue}");
    }
    if let Some(root) = std::env::var_os("MYPDF_MARKUP_PROOF_DIR") {
        let root = std::path::PathBuf::from(root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("pattern-fills.pdf"), bytes).unwrap();
    }
}

#[test]
fn vector_patterns_are_clipped_to_concave_edges_and_move_with_the_object() {
    let outline = [
        [0., 0.],
        [100., 0.],
        [100., 35.],
        [35., 35.],
        [35., 100.],
        [0., 100.],
    ];
    for pattern in FillPattern::ALL.into_iter().skip(1) {
        let paths = fill::paths(&outline, pattern, 2.);
        assert!(!paths.is_empty());
        assert!(
            paths.iter().flatten().all(|p| p[0] >= -0.001
                && p[1] >= -0.001
                && p[0] <= 100.001
                && p[1] <= 100.001
                && (p[0] <= 35.001 || p[1] <= 35.001)),
            "hatches must not cross the empty corner: {pattern:?}"
        );
        let moved: Vec<_> = outline.iter().map(|p| [p[0] + 73., p[1] + 46.]).collect();
        let moved = fill::paths(&moved, pattern, 2.);
        assert_eq!(paths.len(), moved.len());
        for (before, after) in paths.iter().flatten().zip(moved.iter().flatten()) {
            assert!((after[0] - before[0] - 73.).abs() < 0.001);
            assert!((after[1] - before[1] - 46.).abs() < 0.001);
        }
    }
}

#[test]
fn patterned_geometry_is_bounded_for_very_large_shapes() {
    let outline = [
        [0., 0.],
        [100_000., 0.],
        [100_000., 100_000.],
        [0., 100_000.],
    ];
    for pattern in FillPattern::ALL.into_iter().skip(1) {
        let paths = fill::paths(&outline, pattern, 0.1);
        assert!(!paths.is_empty());
        assert!(
            paths.len() < 20_000,
            "bounded pattern geometry: {pattern:?} = {}",
            paths.len()
        );
        assert!(paths.iter().flatten().flatten().all(|v| v.is_finite()));
    }
}

#[test]
fn foreign_native_squiggly_annotations_are_preserved_when_other_objects_change() {
    let bytes = ann::export(&convert::blank(220., 90.).unwrap(), &[squiggly()]).unwrap();
    let mut doc = lopdf::Document::load_mem(&bytes).unwrap();
    for object in doc.objects.values_mut() {
        if let Ok(d) = object.as_dict_mut() {
            for key in [b"MyPDFData".as_slice(), b"MyPDFState", b"MyPDFApDigest"] {
                d.remove(key);
            }
        }
    }
    let mut foreign = vec![];
    doc.save_to(&mut foreign).unwrap();
    assert!(
        ann::load(&doc).is_empty(),
        "foreign markup must not be mistaken for managed metadata"
    );
    let saved = ann::export(&foreign, &[]).unwrap();
    let saved_doc = lopdf::Document::load_mem(&saved).unwrap();
    assert_eq!(
        saved_doc
            .objects
            .values()
            .filter_map(|o| o.as_dict().ok())
            .filter(|d| {
                d.get(b"Subtype").and_then(lopdf::Object::as_name).ok() == Some(b"Squiggly")
            })
            .count(),
        1
    );
    assert_eq!(
        viewer::render_page(&foreign, 0, 2.).unwrap(),
        viewer::render_page(&saved, 0, 2.).unwrap()
    );
}

#[test]
fn hatch_fills_keep_their_geometry_when_the_border_is_wavy() {
    let mut mark = Annotation::new(
        0,
        [10., 10., 110., 100.],
        Kind::Ellipse,
        Style {
            fill: Some([0.2, 0.55, 0.85]),
            fill_pattern: FillPattern::Crosshatch,
            ..Default::default()
        },
    );
    let fills = |a: &Annotation| {
        ann::primitives(a)
            .into_iter()
            .filter_map(|p| match p {
                Primitive::Path {
                    points,
                    fill: true,
                    closed: true,
                } => Some(points),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let solid_border = fills(&mark);
    mark.style.stroke = StrokeStyle::Wave;
    assert_eq!(
        fills(&mark),
        solid_border,
        "drawing border styles must not distort hatch lines"
    );
}
