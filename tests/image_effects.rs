use mypdf_milestone0::{
    annotations as ann, assets, content, convert,
    image_effects::{self, RasterOptions},
};
use std::sync::Arc;

fn source() -> Arc<Vec<u8>> {
    let image = image::RgbaImage::from_fn(96, 64, |x, y| {
        if (30..65).contains(&x) && (15..50).contains(&y) {
            image::Rgba([220, 40, 80, 255])
        } else {
            image::Rgba([0, 0, 0, 0])
        }
    });
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).unwrap();
    Arc::new(png.into_inner())
}
#[test]
fn blur_keeps_transparency_source_pixels_and_rejects_invalid_values() {
    let source = source();
    let original = source.clone();
    let changed = image_effects::blurred(&source, 3.).unwrap();
    let pixels = assets::decode_png(&changed).unwrap().into_rgba8();
    assert_eq!(pixels.get_pixel(0, 0)[3], 0);
    let edge = pixels.get_pixel(30, 30);
    assert!(edge[3] > 0 && edge[3] < 255);
    assert!(
        edge[0] > 210,
        "No dark halo at the transparent edge: {edge:?}"
    );
    assert_eq!(source, original);
    assert_eq!(image_effects::blurred(&source, 0.).unwrap(), original);
    // Other documents may evict this effect from the shared bounded cache.
    // Reusing an allocation is an optimization; reproduced pixels must match.
    assert_eq!(changed, image_effects::blurred(&source, 3.).unwrap());
    assert!(image_effects::blurred(&source, f32::NAN).is_err());
}
#[test]
fn artwork_border_blur_and_opacity_roundtrip_with_original_and_undo() {
    let blank = convert::blank(595., 842.).unwrap();
    let source = source();
    let mut mark = ann::Annotation::new(
        0,
        [40., 50., 232., 178.],
        ann::Kind::Image {
            png: source.clone(),
        },
        Default::default(),
    );
    let mut session = ann::Session::new(blank.clone().into(), vec![mark.clone()], true);
    mark.style.image_border = true;
    mark.style.image_blur = 3.;
    mark.style.opacity = 0.55;
    mark.style.width = 3.;
    mark.style.color = [0.1, 0.3, 0.8];
    mark.style.stroke = ann::StrokeStyle::Dashed;
    session.commit(vec![mark.clone()]).unwrap();
    let pdf = session.snapshot_bytes().unwrap();
    let document = lopdf::Document::load_mem(&pdf).unwrap();
    let restored = ann::load(&document);
    assert_eq!(restored, vec![mark.clone()]);
    let ann::Kind::Image { png } = &restored[0].kind else {
        panic!()
    };
    assert_eq!(
        png, &source,
        "Original pixels are retained independently of the appearance"
    );
    let primitives = ann::primitives(&restored[0]);
    assert!(
        primitives
            .iter()
            .any(|p| matches!(p,ann::Primitive::Image{png,..} if png != &source))
    );
    assert!(
        primitives
            .iter()
            .filter(|p| matches!(p, ann::Primitive::Path { .. }))
            .count()
            > 4
    );
    // Both the PDF appearance and the editor use the same processed image.
    let images: Vec<_> = document
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .filter(|s| {
            s.dict
                .get(b"Subtype")
                .and_then(|o| o.as_name())
                .is_ok_and(|n| n == b"Image")
        })
        .collect();
    assert!(images.iter().any(|s| s.dict.has(b"SMask")));
    session.undo();
    assert_eq!(session.annotations[0].style.image_blur, 0.);
    session.redo();
    assert_eq!(session.annotations[0].style.image_blur, 3.);
}
#[test]
fn embedded_image_appearance_preserves_alpha_and_saves_undoable_copy() {
    let source = source();
    let blank = convert::blank(595., 842.).unwrap();
    let original = content::add_image(&blank, 1, [40., 50., 192., 128.], &source).unwrap();
    let options = RasterOptions {
        blur: 2.,
        opacity: 0.5,
        border: 3,
        color: [10, 30, 200],
    };
    let output = image_effects::raster(&source, options).unwrap();
    let index = content::images(&original, 1).unwrap()[0].index;
    let pixels = assets::decode_png(&output).unwrap().into_rgba8();
    assert_eq!(pixels.get_pixel(0, 0).0, [10, 30, 200, 128]);
    assert_eq!(pixels.get_pixel(10, 10)[3], 0);
    let changed = content::edit_image(
        &original,
        1,
        index,
        &content::ImageEdit::Replace(output.clone()),
    )
    .unwrap();
    let restored = content::image_png(&changed, 1, index).unwrap();
    assert_eq!(assets::decode_png(&restored).unwrap().into_rgba8(), pixels);
    let mut session = ann::Session::new(original.clone().into(), vec![], true);
    session.commit_document(changed.clone()).unwrap();
    session.undo();
    assert_eq!(&*session.source, &original);
    session.redo();
    assert_eq!(&*session.source, &changed);
}
