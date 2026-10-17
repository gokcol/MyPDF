use lopdf::{
    Object, Stream,
    content::{Content, Operation},
    dictionary,
};
use mypdf_milestone0::{annotations as ann, content, convert, image_region, pages, viewer};
use std::sync::Arc;

fn vector_logo() -> Vec<u8> {
    let mut doc = lopdf::Document::load_mem(&convert::blank(300., 240.).unwrap()).unwrap();
    let form = doc.add_object(Stream::new(dictionary! {
        "Type"=>"XObject", "Subtype"=>"Form", "BBox"=>vec![0.into(),0.into(),100.into(),60.into()]
    }, Content { operations: vec![
        Operation::new("rg", vec![0.1.into(),0.2.into(),0.85.into()]),
        Operation::new("re", vec![0.into(),0.into(),100.into(),60.into()]),
        Operation::new("f", vec![]),
    ]}.encode().unwrap()));
    let stream = doc.add_object(Stream::new(
        dictionary! {},
        Content {
            operations: vec![
                Operation::new("q", vec![]),
                Operation::new(
                    "cm",
                    vec![
                        1.into(),
                        0.into(),
                        0.into(),
                        1.into(),
                        40.into(),
                        140.into(),
                    ],
                ),
                Operation::new("Do", vec![Object::Name(b"Logo".to_vec())]),
                Operation::new("Q", vec![]),
            ],
        }
        .encode()
        .unwrap(),
    ));
    let page = *doc.get_pages().get(&1).unwrap();
    let d = doc.get_dictionary_mut(page).unwrap();
    d.set(
        "Resources",
        dictionary! {"XObject"=>dictionary!{"Logo"=>form}},
    );
    d.set("Contents", stream);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

#[test]
fn vector_region_becomes_editable_image_with_removable_cover_and_atomic_undo() {
    let source = vector_logo();
    assert!(
        content::images(&source, 1).unwrap().is_empty(),
        "Vector artwork is not a bitmap image object"
    );
    let rect = [35., 35., 145., 105.];
    let png = Arc::new(image_region::capture(&source, 0, rect, 144.).unwrap());
    let image = image::load_from_memory(&png).unwrap().into_rgba8();
    assert_eq!(image.dimensions(), (220, 140));
    assert!(image.get_pixel(100, 70)[2] > 200);
    assert_eq!(image.get_pixel(0, 0).0, [255; 4]);
    let mut session = ann::Session::new(source.clone().into(), vec![], true);
    let layers = image_region::layers(0, rect, png, Some([1.; 3])).unwrap();
    session.commit(layers).unwrap();
    let saved = session.snapshot_bytes().unwrap();
    let (size, before) = viewer::render_page(&source, 0, 1.).unwrap();
    let (after_size, after) = viewer::render_page(&saved, 0, 1.).unwrap();
    assert_eq!(size, after_size);
    let mut changed = 0;
    for (i, (a, b)) in before
        .chunks_exact(4)
        .zip(after.chunks_exact(4))
        .enumerate()
    {
        if a != b {
            let (x, y) = (i % size[0], i / size[0]);
            assert!(
                (35..145).contains(&x) && (35..105).contains(&y),
                "Outside content changed at {x},{y}"
            );
            if a.iter()
                .zip(b)
                .any(|(a, b)| (*a as i16 - *b as i16).abs() > 2)
            {
                changed += 1;
            }
        }
    }
    // Resampling a 2x copy may antialias its edge differently; its interior and
    // placement must remain unchanged, and everything outside stays exact.
    assert!(changed < 700, "{changed} significantly different pixels");
    let mut layers = ann::load(&lopdf::Document::load_mem(&saved).unwrap());
    assert_eq!(layers.len(), 2);
    // The image moves; its separate original-position cover stays put.
    layers[1].rect = [165., 135., 275., 205.];
    layers[1].style.image_blur = 2.;
    layers[1].style.image_border = true;
    session.commit(layers).unwrap();
    let changed = session.snapshot_bytes().unwrap();
    let ([w, _], pixels) = viewer::render_page(&changed, 0, 1.).unwrap();
    assert_eq!(&pixels[(70 * w + 80) * 4..(70 * w + 80) * 4 + 3], &[255; 3]);
    assert!(pixels[(170 * w + 220) * 4 + 2] > 150);
    session.undo();
    session.undo();
    assert!(session.annotations.is_empty());
    assert_eq!(&*session.source, &source);
    // Removing the cover reveals original artwork; it was never redacted.
    assert!(
        viewer::render_page(&source, 0, 1.).unwrap()
            == viewer::render_page(&session.snapshot_bytes().unwrap(), 0, 1.).unwrap()
    );
}

#[test]
fn region_capture_uses_display_coordinates_after_page_crop_and_rotation() {
    let mut source = pages::apply(
        &vector_logo(),
        &pages::Operation::Crop {
            pages: vec![0],
            margins: [10.; 4],
        },
    )
    .unwrap();
    for _ in 0..4 {
        let ([w, h], pixels) = viewer::render_page(&source, 0, 1.).unwrap();
        let rect = [15., 20., (w - 15) as f32, (h - 20) as f32];
        let png = image_region::capture(&source, 0, rect, 72.).unwrap();
        let region = image::load_from_memory(&png).unwrap().into_rgba8();
        assert_eq!(region.dimensions(), ((w - 30) as u32, (h - 40) as u32));
        for y in (5..h - 45).step_by(13) {
            for x in (5..w - 35).step_by(17) {
                let index = ((y + 20) * w + x + 15) * 4;
                let got = region.get_pixel(x as u32, y as u32).0;
                for k in 0..4 {
                    assert!((got[k] as i16 - pixels[index + k] as i16).abs() <= 1);
                }
            }
        }
        source = pages::apply(
            &source,
            &pages::Operation::Rotate {
                pages: vec![0],
                clockwise: true,
            },
        )
        .unwrap();
    }
    assert!(image_region::capture(&source, 0, [-1., 0., 10., 10.], 150.).is_err());
    assert!(image_region::capture(&source, 0, [0., 0., 10., 10.], f32::NAN).is_err());
    assert!(image_region::capture(&source, 4, [0., 0., 10., 10.], 150.).is_err());
    let mut locked = ann::Session::new(source.into(), vec![], false);
    let png =
        Arc::new(image_region::capture(&vector_logo(), 0, [35., 35., 145., 105.], 144.).unwrap());
    assert!(
        locked
            .commit(image_region::layers(0, [35., 35., 145., 105.], png, None).unwrap())
            .is_err()
    );
}
