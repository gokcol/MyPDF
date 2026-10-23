use mypdf_milestone0::{
    annotations::{
        self as ann, Annotation, FillPattern, Kind, Point2, Primitive, Shape, StrokeStyle, Style,
    },
    convert, viewer,
};

const SHAPES: [Shape; 7] = [
    Shape::Square,
    Shape::Diamond,
    Shape::Triangle,
    Shape::Pentagon,
    Shape::Hexagon,
    Shape::Circle,
    Shape::Semicircle,
];

fn area(points: &[Point2]) -> f32 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum::<f32>()
        / 2.
}

fn inside_convex(outline: &[Point2], point: Point2) -> bool {
    let winding = area(outline).signum();
    outline
        .iter()
        .zip(outline.iter().cycle().skip(1))
        .take(outline.len())
        .all(|(a, b)| {
            ((b[0] - a[0]) * (point[1] - a[1]) - (b[1] - a[1]) * (point[0] - a[0])) * winding
                >= -0.02
        })
}

fn shape_annotation(shape: Shape, rect: [f32; 4]) -> Annotation {
    Annotation::new(
        0,
        rect,
        Kind::Shape { shape },
        Style {
            color: [0.1, 0.3, 0.65],
            fill: Some([0.8, 0.9, 1.]),
            width: 1.5,
            ..Default::default()
        },
    )
}

#[test]
fn shape_outlines_have_valid_closed_boundaries_and_distinct_silhouettes() {
    for (shape, vertices) in SHAPES.into_iter().zip([4, 4, 3, 5, 6, 64, 65]) {
        let outline = shape.outline();
        assert_eq!(outline.len(), vertices, "{}", shape.label());
        assert!(
            outline
                .iter()
                .flatten()
                .all(|v| v.is_finite() && *v >= -0.0001 && *v <= 1.0001)
        );
        assert!(
            area(&outline).abs() > 0.35,
            "{} has no usable interior",
            shape.label()
        );
        assert_ne!(
            outline.first(),
            outline.last(),
            "Closure must not add a zero-length edge"
        );
        assert!(inside_convex(&outline, [0.5, 0.5]));
        let a = shape_annotation(shape, [12., 18., 132., 98.]);
        a.validate().unwrap();
        assert_eq!(a.kind.label(), shape.label());
    }
    let semicircle = Shape::Semicircle.outline();
    let ends = [semicircle[0], *semicircle.last().unwrap()];
    assert!(
        (ends[0][1] - ends[1][1]).abs() < 0.0001,
        "The semicircle needs one flat diameter"
    );
    assert!((ends[0][0] - ends[1][0]).abs() > 0.99);
    assert!(
        semicircle.iter().any(|p| p[1] < ends[0][1] - 0.49),
        "Its arc must rise above the diameter"
    );
    assert!(area(&Shape::Triangle.outline()).abs() < area(&Shape::Pentagon.outline()).abs());
}

#[test]
fn every_shape_hatch_is_clipped_to_its_actual_silhouette() {
    for shape in SHAPES {
        for pattern in FillPattern::ALL {
            let mut a = shape_annotation(shape, [35., 45., 175., 145.]);
            a.style.fill_pattern = pattern;
            let outline: Vec<_> = shape.outline().into_iter().map(|p| a.local(p)).collect();
            let primitives = ann::primitives(&a);
            let fills: Vec<_> = primitives
                .iter()
                .filter_map(|p| match p {
                    Primitive::Path {
                        points,
                        closed: true,
                        fill: true,
                    } => Some(points),
                    _ => None,
                })
                .collect();
            assert!(!fills.is_empty(), "{} / {}", shape.label(), pattern.label());
            assert!(
                fills
                    .iter()
                    .flat_map(|points| points.iter())
                    .all(|p| inside_convex(&outline, *p)),
                "{} / {} leaks into an empty corner",
                shape.label(),
                pattern.label()
            );
            assert!(
                primitives.iter().any(|p| matches!(
                    p,
                    Primitive::Path {
                        closed: true,
                        fill: false,
                        ..
                    }
                )),
                "The closed border must remain visible"
            );
            a.style.fill = None;
            assert!(
                !ann::primitives(&a)
                    .iter()
                    .any(|p| matches!(p, Primitive::Path { fill: true, .. })),
                "No fill means no hatch"
            );
        }
    }
}

#[test]
fn shape_presets_export_native_annotations_with_vector_fill_and_stroke() {
    let source = convert::blank(650., 550.).unwrap();
    let mut entries = vec![];
    for (row, shape) in SHAPES.into_iter().enumerate() {
        for (column, pattern) in FillPattern::ALL.into_iter().enumerate() {
            let x = 20. + column as f32 * 125.;
            let y = 20. + row as f32 * 75.;
            let mut a = shape_annotation(shape, [x, y, x + 90., y + 50.]);
            a.style.fill_pattern = pattern;
            a.style.stroke = StrokeStyle::ALL[(row + column) % StrokeStyle::ALL.len()];
            entries.push(a);
        }
    }
    let bytes = ann::export(&source, &entries).unwrap();
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    assert_eq!(ann::load(&doc), entries);
    let mut count = 0;
    for d in doc
        .objects
        .values()
        .filter_map(|o| o.as_dict().ok())
        .filter(|d| d.has(b"MyPDFData"))
    {
        count += 1;
        let a: Annotation =
            serde_json::from_slice(d.get(b"MyPDFData").unwrap().as_str().unwrap()).unwrap();
        let Kind::Shape { shape } = a.kind else {
            panic!("Expected an editable shape preset")
        };
        let subtype = d.get(b"Subtype").unwrap().as_name().unwrap();
        match shape {
            Shape::Square => assert_eq!(subtype, b"Square"),
            Shape::Circle => assert_eq!(subtype, b"Circle"),
            _ => {
                assert_eq!(subtype, b"Polygon");
                assert_eq!(
                    d.get(b"Vertices").unwrap().as_array().unwrap().len(),
                    shape.outline().len() * 2
                );
            }
        }
        assert_eq!(d.get(b"IC").unwrap().as_array().unwrap().len(), 3);
        let ap = d.get(b"AP").unwrap().as_dict().unwrap().get(b"N").unwrap();
        let stream = doc.dereference(ap).unwrap().1.as_stream().unwrap();
        let data = stream
            .decompressed_content()
            .unwrap_or_else(|_| stream.content.clone());
        let content = lopdf::content::Content::decode(&data).unwrap();
        assert!(
            content.operations.iter().any(|o| o.operator == "f"),
            "{} lost its fill",
            shape.label()
        );
        assert!(
            content
                .operations
                .iter()
                .any(|o| o.operator == "S" || o.operator == "f")
        );
        assert!(
            !content.operations.iter().any(|o| o.operator == "Do"),
            "{} must not be rasterized",
            shape.label()
        );
    }
    assert_eq!(count, entries.len());
    viewer::render_page(&bytes, 0, 1.).unwrap();
    let repeated = ann::export(&bytes, &entries).unwrap();
    assert_eq!(
        ann::load(&lopdf::Document::load_mem(&repeated).unwrap()),
        entries
    );
    let removed = ann::export(&bytes, &[]).unwrap();
    assert!(ann::load(&lopdf::Document::load_mem(&removed).unwrap()).is_empty());
}

#[test]
fn shape_editability_survives_object_transforms_and_cropped_rotated_pages() {
    let source = convert::blank(420., 380.).unwrap();
    for rotation in [0, 90, 180, 270] {
        let mut doc = lopdf::Document::load_mem(&source).unwrap();
        let page = doc.get_pages()[&1];
        let d = doc.get_dictionary_mut(page).unwrap();
        d.set(
            "CropBox",
            vec![20.into(), 20.into(), 400.into(), 360.into()],
        );
        d.set("Rotate", rotation);
        let mut original = vec![];
        doc.save_to(&mut original).unwrap();
        let entries: Vec<_> = SHAPES
            .into_iter()
            .enumerate()
            .map(|(i, shape)| {
                let x = 30. + (i % 3) as f32 * 100.;
                let y = 30. + (i / 3) as f32 * 100.;
                let mut a = shape_annotation(shape, [x, y, x + 65., y + 65.]);
                a.rotate_by(17.);
                a.flip_horizontal();
                if i % 2 == 0 {
                    a.flip_vertical();
                }
                a.style.fill_pattern = FillPattern::Diagonal;
                a
            })
            .collect();
        let bytes = ann::export(&original, &entries).unwrap();
        assert_eq!(
            ann::load(&lopdf::Document::load_mem(&bytes).unwrap()),
            entries
        );
        let (_, pixels) = viewer::render_page(&bytes, 0, 1.).unwrap();
        assert!(
            pixels.chunks_exact(4).any(|p| p[0] < 160 && p[2] > 100),
            "No visible shapes at page rotation {rotation}"
        );
    }
}
