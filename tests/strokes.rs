use mypdf_milestone0::{
    annotations::{self as ann, Annotation, Kind, Primitive, StrokeStyle, Style, Symbol},
    assets, convert, stroke,
};

#[test]
fn stroke_patterns_continue_across_corners_and_waves_close() {
    let ring = assets::builtin_assets()
        .into_iter()
        .find(|a| a.asset.name == "Circle")
        .unwrap();
    let Kind::Vector { polygons } = ring.asset.kind else {
        panic!()
    };
    assert_eq!(
        stroke::outlines(&polygons).len(),
        2,
        "ring outlines must exclude internal construction edges"
    );
    let marks = stroke::path(
        vec![[0., 0.], [5., 0.], [5., 40.]],
        false,
        2.,
        StrokeStyle::Dashed,
    );
    let Primitive::Path { points, .. } = &marks[0] else {
        panic!()
    };
    assert_eq!(
        points,
        &vec![[0., 0.], [5., 0.], [5., 3.]],
        "dash must turn a corner without restarting"
    );
    let dots = stroke::path(
        vec![[0., 0.], [5., 0.], [5., 40.]],
        false,
        2.,
        StrokeStyle::Dotted,
    );
    assert!(matches!(dots[1], Primitive::Dot { center: [5., 1.] }));
    let wave = stroke::path(
        vec![[0., 0.], [100., 0.], [100., 70.], [0., 70.]],
        true,
        2.,
        StrokeStyle::Wave,
    );
    let Primitive::Path { points, closed, .. } = &wave[0] else {
        panic!()
    };
    assert!(*closed);
    assert!(points.len() > 100);
    assert!((points[0][0] - points.last().unwrap()[0]).abs() < 0.001);
    assert!((points[0][1] - points.last().unwrap()[1]).abs() < 0.001);
    assert!(
        points.iter().any(|p| p[1] < -1.),
        "wave must have visible amplitude"
    );
    assert!(stroke::path(vec![[0., 0.], [0., 0.]], true, 2., StrokeStyle::Wave).is_empty());
}

#[test]
fn every_drawing_stroke_style_roundtrips_with_vector_appearances() {
    let kinds = vec![
        Kind::Line {
            start: [0., 0.8],
            end: [1., 0.2],
            arrow: false,
        },
        Kind::Line {
            start: [0., 0.8],
            end: [1., 0.2],
            arrow: true,
        },
        Kind::DoubleArrow {
            start: [0., 0.8],
            end: [1., 0.2],
        },
        Kind::Rectangle,
        Kind::Ellipse,
        Kind::Polygon {
            points: vec![[0., 0.9], [0.5, 0.], [1., 0.8]],
            closed: false,
        },
        Kind::Polygon {
            points: vec![[0., 0.9], [0.5, 0.], [1., 0.8]],
            closed: true,
        },
        Kind::Ink {
            strokes: vec![vec![[0., 0.8], [0.3, 0.3], [0.6, 0.9], [1., 0.1]]],
        },
        Kind::Symbol(Symbol::Check),
        Kind::Symbol(Symbol::Star),
    ];
    let mut entries = vec![];
    for (row, kind) in kinds.iter().enumerate() {
        for (col, stroke) in StrokeStyle::ALL.into_iter().enumerate() {
            let x = 35. + col as f32 * 113.;
            let y = 50. + row as f32 * 72.;
            let a = Annotation::new(
                0,
                [x, y, x + 80., y + 42.],
                kind.clone(),
                Style {
                    stroke,
                    width: 1.7,
                    fill: matches!(
                        kind,
                        Kind::Rectangle | Kind::Ellipse | Kind::Polygon { closed: true, .. }
                    )
                    .then_some([0.88, 0.94, 0.97]),
                    ..Default::default()
                },
            );
            if matches!(kind, Kind::Rectangle | Kind::Ellipse) {
                let p = ann::primitives(&a);
                assert!(
                    p.iter()
                        .any(|p| matches!(p, Primitive::Path { fill: true, .. }))
                );
                assert!(p.iter().any(|p| matches!(
                    p,
                    Primitive::Path { fill: false, .. } | Primitive::Dot { .. }
                )));
            }
            entries.push(a);
        }
    }
    let bytes = ann::export(&convert::blank(630., 800.).unwrap(), &entries).unwrap();
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    assert_eq!(ann::load(&doc), entries);
    for d in doc
        .objects
        .values()
        .filter_map(|o| o.as_dict().ok())
        .filter(|d| d.has(b"MyPDFData"))
    {
        let a: Annotation =
            serde_json::from_slice(d.get(b"MyPDFData").unwrap().as_str().unwrap()).unwrap();
        let ap = d.get(b"AP").unwrap().as_dict().unwrap().get(b"N").unwrap();
        let stream = doc.dereference(ap).unwrap().1.as_stream().unwrap();
        let data = stream
            .decompressed_content()
            .unwrap_or_else(|_| stream.content.clone());
        let operators = lopdf::content::Content::decode(&data).unwrap();
        assert!(
            operators
                .operations
                .iter()
                .any(|o| o.operator == "S" || o.operator == "f")
        );
        assert!(
            !operators.operations.iter().any(|o| o.operator == "Do"),
            "strokes stay vector, including waves and dots"
        );
        if a.style.stroke.dash_array(a.style.width).is_some() {
            assert_eq!(
                d.get(b"BS")
                    .unwrap()
                    .as_dict()
                    .unwrap()
                    .get(b"S")
                    .unwrap()
                    .as_name()
                    .unwrap(),
                b"D"
            );
        }
    }
    if let Some(root) = std::env::var_os("MYPDF_STROKE_PROOF_DIR") {
        let root = std::path::PathBuf::from(root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("stroke-styles.pdf"), bytes).unwrap();
    }
    for asset in assets::builtin_assets() {
        for stroke in StrokeStyle::ALL {
            let mut a = Annotation::new(
                0,
                [100., 100., 250., 250.],
                asset.asset.kind.clone(),
                asset.asset.style.clone(),
            );
            a.style.stroke = stroke;
            a.rotation = 27.;
            let bytes = ann::export(&convert::blank(400., 400.).unwrap(), &[a.clone()]).unwrap();
            assert_eq!(
                ann::load(&lopdf::Document::load_mem(&bytes).unwrap()),
                vec![a]
            );
        }
    }
}

#[test]
fn legacy_styles_default_to_solid_and_live_history_is_save_safe() {
    let mut json = serde_json::to_value(Style::default()).unwrap();
    json.as_object_mut().unwrap().remove("stroke");
    assert_eq!(
        serde_json::from_value::<Style>(json).unwrap().stroke,
        StrokeStyle::Solid
    );
    let mark = Annotation::new(0, [10., 10., 80., 90.], Kind::Rectangle, Style::default());
    let mut session = ann::Session::new(
        convert::blank(200., 200.).unwrap().into(),
        vec![mark.clone()],
        true,
    );
    let mut changed = mark.clone();
    let mut token = None;
    for width in [3., 4., 5.] {
        changed.style.width = width;
        session.commit_live(vec![changed.clone()], token).unwrap();
        assert!(session.dirty());
        token = Some(session.revision());
        let saved = session.snapshot_bytes().unwrap();
        assert_eq!(
            ann::load(&lopdf::Document::load_mem(&saved).unwrap()),
            vec![changed.clone()]
        );
    }
    session.undo();
    assert_eq!(session.annotations, vec![mark.clone()]);
    session.redo();
    assert_eq!(session.annotations, vec![changed.clone()]);
    session.mark_saved(session.revision());
    changed.style.stroke = StrokeStyle::Dotted;
    session
        .commit_live(vec![changed.clone()], Some(session.revision()))
        .unwrap();
    assert!(session.dirty());
    session.undo();
    assert!(
        !session.dirty(),
        "undo must reach a saved checkpoint inside a live gesture"
    );
    assert_eq!(session.annotations[0].style.stroke, StrokeStyle::Solid);
    // A stale gesture token cannot erase the history of an unrelated document change.
    let old = token;
    session.commit(vec![mark.clone()]).unwrap();
    session.commit_live(vec![changed.clone()], old).unwrap();
    session.undo();
    assert_eq!(session.annotations, vec![mark]);
}
