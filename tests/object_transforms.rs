use mypdf_milestone0::{
    annotations::{self as ann, Annotation, Kind, Style},
    assets::Asset,
    convert, viewer,
};
use std::sync::Arc;

fn image() -> image::RgbaImage {
    image::RgbaImage::from_fn(80, 40, |x, y| {
        if x < 25 && y < 30 {
            image::Rgba([10, 70, 180, 255])
        } else {
            image::Rgba([0, 0, 0, 0])
        }
    })
}

#[test]
fn image_signature_import_preserves_transparency_and_library_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("My signature.png");
    image().save(&path).unwrap();
    let asset = Asset::from_image(&path).unwrap();
    assert_eq!(asset.aspect, 2.);
    let Kind::Image { png } = &asset.kind else {
        panic!()
    };
    assert_eq!(
        mypdf_milestone0::assets::decode_png(png)
            .unwrap()
            .into_rgba8(),
        image()
    );
    let mut library = mypdf_milestone0::assets::Library::open(dir.path().join("library")).unwrap();
    library.entries.push(asset.clone());
    library.save().unwrap();
    assert_eq!(
        mypdf_milestone0::assets::Library::open(dir.path().join("library"))
            .unwrap()
            .entries,
        vec![asset]
    );
    image::DynamicImage::ImageRgba8(image())
        .into_rgb8()
        .save(dir.path().join("signature.jpg"))
        .unwrap();
    assert_eq!(
        Asset::from_image(&dir.path().join("signature.jpg"))
            .unwrap()
            .aspect,
        2.
    );
    std::fs::write(&path, b"not an image").unwrap();
    assert!(Asset::from_image(&path).is_err());
}

#[test]
fn rotated_reflections_hit_the_visible_drawing_and_old_metadata_still_loads() {
    let mut a = Annotation::new(
        0,
        [80., 110., 240., 210.],
        Kind::Line {
            start: [0., 0.],
            end: [1., 1.],
            arrow: true,
        },
        Style::default(),
    );
    let mut legacy = serde_json::to_value(&a).unwrap();
    legacy.as_object_mut().unwrap().remove("flip_x");
    legacy.as_object_mut().unwrap().remove("flip_y");
    assert_eq!(serde_json::from_value::<Annotation>(legacy).unwrap(), a);
    a.rotate_by(37.);
    let p = a.rotated(a.local([0.2, 0.2]));
    a.flip_horizontal();
    let expected = [320. - p[0], p[1]];
    let actual = a.rotated(a.local([0.2, 0.2]));
    assert!((actual[0] - expected[0]).abs() < 0.001 && (actual[1] - expected[1]).abs() < 0.001);
    assert!(a.hit_test(expected, 0.1));
    a.flip_vertical();
    assert!(a.hit_test([expected[0], 320. - expected[1]], 0.1));
    for _ in 0..10 {
        a.rotate_by(90.);
        a.validate().unwrap();
    }
}

#[test]
fn saved_vector_text_svg_and_image_appearances_match_reflection_and_rotation() {
    let mut png = std::io::Cursor::new(vec![]);
    image().write_to(&mut png, image::ImageFormat::Png).unwrap();
    let source = convert::blank(320., 320.).unwrap();
    let kinds = vec![
        Kind::Polygon { points: vec![[0.,0.], [1.,0.25], [0.25,1.]], closed:true },
        Kind::Text { text: "Signature F".into() },
        Kind::Svg { source: "<svg xmlns='http://www.w3.org/2000/svg' width='80' height='40'><path d='M0 0 L80 10 L20 40 Z' fill='#174d8a'/></svg>".into(), tint:false },
        Kind::Image { png: Arc::new(png.into_inner()) },
    ];
    for kind in kinds {
        let a = Annotation::new(
            0,
            [80., 110., 240., 210.],
            kind,
            Style {
                fill: Some([0.1, 0.4, 0.2]),
                ..Default::default()
            },
        );
        let original = ann::export(&source, std::slice::from_ref(&a)).unwrap();
        let (size, pixels) = viewer::render_page(&original, 0, 1.).unwrap();
        assert_eq!(size, [320, 320]);
        for mode in 0..3 {
            let mut b = a.clone();
            match mode {
                0 => b.flip_horizontal(),
                1 => b.flip_vertical(),
                _ => b.rotate_by(90.),
            }
            let pdf = ann::export(&source, std::slice::from_ref(&b)).unwrap();
            assert_eq!(
                ann::load(&lopdf::Document::load_mem(&pdf).unwrap()),
                vec![b]
            );
            let (_, transformed) = viewer::render_page(&pdf, 0, 1.).unwrap();
            let mut error = 0u64;
            let mut changed = 0;
            for y in 0..320 {
                for x in 0..320 {
                    let (sx, sy) = match mode {
                        0 => (319 - x, y),
                        1 => (x, 319 - y),
                        _ => (y, 319 - x),
                    };
                    for c in 0..3 {
                        error += (transformed[(y * 320 + x) * 4 + c] as i16
                            - pixels[(sy * 320 + sx) * 4 + c] as i16)
                            .unsigned_abs() as u64;
                        if transformed[(y * 320 + x) * 4 + c] != pixels[(y * 320 + x) * 4 + c] {
                            changed += 1;
                        }
                    }
                }
            }
            assert!(
                error as f64 / (320. * 320. * 3.) < 1.5,
                "appearance mismatch {mode}: {error}"
            );
            assert!(changed > 100, "transform must change the rendered artwork");
        }
        let mut session = ann::Session::new(Arc::from(source.clone()), vec![a.clone()], true);
        let mut b = a.clone();
        b.rotate_by(-90.);
        b.flip_vertical();
        session.commit(vec![b.clone()]).unwrap();
        session.undo();
        assert_eq!(session.annotations, vec![a]);
        session.redo();
        assert_eq!(session.annotations, vec![b]);
    }
}
