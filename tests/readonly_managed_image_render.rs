//! Read-only viewing can smooth our saved image appearances without changing
//! signed source bytes or another author's explicit image sampling choices.
use lopdf::{Document, Object, ObjectId, Stream, dictionary};
use mypdf_milestone0::{
    annotations::{self as ann, Annotation, Kind, Style},
    convert, identity, signing,
    viewer::{self, Event, Worker},
};
use std::{io::Cursor, sync::Arc, time::Duration};

#[derive(Clone, Copy)]
struct Appearance {
    page: ObjectId,
    annotation: ObjectId,
    form: ObjectId,
    image: ObjectId,
    mask: ObjectId,
}
fn save(doc: &mut Document) -> Vec<u8> {
    let mut bytes = vec![];
    doc.save_to(&mut bytes).unwrap();
    bytes
}
fn fixture() -> Vec<u8> {
    // Fictional, high-frequency blue ink. Transparent pixels deliberately carry
    // unrelated RGB so the test also exercises the PDF soft-mask path.
    let image = image::RgbaImage::from_fn(128, 64, |x, y| {
        let center = 31.5 + 21. * ((x as f32 + 0.5) / 17.).sin();
        if ((y as f32 + 0.5) - center).abs() < 1.8 {
            image::Rgba([12, 18, 108, 255])
        } else {
            image::Rgba([40, 180, 220, 0])
        }
    });
    let mut png = Cursor::new(vec![]);
    image.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let exported = ann::export(
        &convert::blank(256., 128.).unwrap(),
        &[Annotation::new(
            0,
            [33.27, 30.14, 159.84, 93.71],
            Kind::Image {
                png: Arc::new(png.into_inner()),
            },
            Style::default(),
        )],
    )
    .unwrap();
    // Reproduce a saved legacy appearance even if future exports start writing
    // explicit interpolation flags. Its integrity-checked AP content stays intact.
    let mut doc = Document::load_mem(&exported).unwrap();
    let ids = appearance(&doc);
    set_interpolation(&mut doc, ids.image, None);
    set_interpolation(&mut doc, ids.mask, None);
    save(&mut doc)
}
fn appearance(doc: &Document) -> Appearance {
    let page = doc.get_pages()[&1];
    let values = doc
        .dereference(doc.get_dictionary(page).unwrap().get(b"Annots").unwrap())
        .unwrap()
        .1
        .as_array()
        .unwrap();
    let annotation = values[0].as_reference().unwrap();
    let ap = doc
        .dereference(doc.get_dictionary(annotation).unwrap().get(b"AP").unwrap())
        .unwrap()
        .1
        .as_dict()
        .unwrap();
    let form = ap.get(b"N").unwrap().as_reference().unwrap();
    let resources = doc
        .dereference(
            doc.get_object(form)
                .unwrap()
                .as_stream()
                .unwrap()
                .dict
                .get(b"Resources")
                .unwrap(),
        )
        .unwrap()
        .1
        .as_dict()
        .unwrap();
    let xobjects = doc
        .dereference(resources.get(b"XObject").unwrap())
        .unwrap()
        .1
        .as_dict()
        .unwrap();
    let image = xobjects.get(b"Im").unwrap().as_reference().unwrap();
    let mask = doc
        .get_object(image)
        .unwrap()
        .as_stream()
        .unwrap()
        .dict
        .get(b"SMask")
        .unwrap()
        .as_reference()
        .unwrap();
    Appearance {
        page,
        annotation,
        form,
        image,
        mask,
    }
}
fn set_interpolation(doc: &mut Document, id: ObjectId, value: Option<bool>) {
    let dictionary = &mut doc
        .get_object_mut(id)
        .unwrap()
        .as_stream_mut()
        .unwrap()
        .dict;
    if let Some(value) = value {
        dictionary.set("Interpolate", value);
    } else {
        dictionary.remove(b"Interpolate");
    }
}
fn explicit_reference(bytes: &[u8], value: bool) -> Vec<u8> {
    let mut doc = Document::load_mem(bytes).unwrap();
    let ids = appearance(&doc);
    set_interpolation(&mut doc, ids.image, Some(value));
    set_interpolation(&mut doc, ids.mask, Some(value));
    save(&mut doc)
}
fn raw_render(bytes: &[u8], scale: f32) -> ([usize; 2], Vec<u8>) {
    let pdf = hayro::hayro_syntax::Pdf::new(bytes.to_vec()).unwrap();
    let raster = hayro::render(
        &pdf.pages()[0],
        &hayro::RenderCache::new(),
        &Default::default(),
        &Default::default(),
        &hayro::PixmapSettings {
            x_scale: scale,
            y_scale: scale,
            bg_color: hayro::vello_cpu::color::palette::css::WHITE,
        },
    );
    (
        [raster.width() as usize, raster.height() as usize],
        raster.data_as_u8_slice().to_vec(),
    )
}
fn mark_readonly(doc: &mut Document) {
    // A structural signature marker is sufficient for non-cryptographic tests;
    // the main end-to-end test below additionally creates and verifies real CMS.
    doc.add_object(dictionary! {
        "Type" => "Sig", "ByteRange" => vec![0.into(), 1.into(), 2.into(), 3.into()],
        "Contents" => Object::string_literal("synthetic render-only marker")
    });
}
fn assert_worker_raster(bytes: Arc<[u8]>, reference: &[u8], scales: &[f32]) {
    let expected_annotations = ann::load(&Document::load_mem(&bytes).unwrap()).len();
    let worker = Worker::from_bytes(bytes.clone(), || {});
    let mut next = 0;
    loop {
        match worker.events.recv_timeout(Duration::from_secs(10)).unwrap() {
            Event::Loaded {
                bytes: loaded,
                pages,
                signed,
                capabilities,
                annotations,
                ..
            } => {
                assert_eq!(
                    loaded, bytes,
                    "the worker must retain the exact signed source"
                );
                assert!(signed && !capabilities.rewritable());
                assert_eq!(annotations.len(), expected_annotations);
                assert!(worker.request(viewer::raster_key(0, pages[0], scales[0], false)));
            }
            Event::Raster { key, size, rgba } => {
                assert_eq!((size, rgba), raw_render(reference, key.factor()));
                next += 1;
                if next == scales.len() {
                    break;
                }
                assert!(worker.request(viewer::RasterKey {
                    page: 0,
                    scale: (scales[next] * 100.).ceil() as u16,
                    thumbnail: false,
                }));
            }
            Event::Indexed { .. } | Event::IndexFinished { .. } => {}
            Event::Failed(error) | Event::RasterFailed { error, .. } => panic!("{error}"),
            Event::PasswordNeeded(_) => panic!("fixture is not encrypted"),
        }
    }
}

#[tokio::test]
async fn signed_managed_image_uses_smooth_readonly_page_and_worker_rasters_without_changing_source()
{
    let created = identity::create("Synthetic render test", "test-only-passphrase", true).unwrap();
    let signer = identity::FileSigner::load(&created.pkcs12, "test-only-passphrase", None).unwrap();
    let signed = signing::sign(&fixture(), &signer, "RenderTestSignature", false)
        .await
        .unwrap();
    let source: Arc<[u8]> = signed.clone().into();
    let original_digest = identity::fingerprint(&signed);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("signed-render.pdf");
    std::fs::write(&path, &signed).unwrap();
    let reference = explicit_reference(&signed, true);
    for scale in [0.25, 0.5, 0.75, 1.] {
        assert_eq!(
            viewer::render_page(&source, 0, scale).unwrap(),
            raw_render(&reference, scale)
        );
        assert_ne!(
            raw_render(&signed, scale).1,
            raw_render(&reference, scale).1,
            "the fixture must actually distinguish nearest and smooth image rendering"
        );
    }
    assert_worker_raster(source.clone(), &reference, &[0.25, 0.5, 0.75, 1.]);
    assert_eq!(identity::fingerprint(&source), original_digest);
    assert_eq!(std::fs::read(path).unwrap(), signed);
    let copy = directory.path().join("signed-exact-copy.pdf");
    viewer::save_copy(&copy, &source).unwrap();
    assert_eq!(std::fs::read(copy).unwrap(), signed);
    let report = signing::verify(&source, None).unwrap();
    assert!(report[0].crypto_valid && report[0].digest_matches && report[0].covers_current_file);
}

#[test]
fn readonly_render_preserves_explicit_false_on_images_or_soft_masks() {
    for explicit_on_mask in [false, true] {
        let mut doc = Document::load_mem(&fixture()).unwrap();
        let ids = appearance(&doc);
        set_interpolation(&mut doc, ids.image, None);
        set_interpolation(&mut doc, ids.mask, None);
        set_interpolation(
            &mut doc,
            if explicit_on_mask {
                ids.mask
            } else {
                ids.image
            },
            Some(false),
        );
        mark_readonly(&mut doc);
        let bytes = save(&mut doc);
        let original = bytes.clone();
        let mut doc = Document::load_mem(&bytes).unwrap();
        assert_eq!(
            ann::interpolated_image_render_copy(&bytes, &mut doc).unwrap(),
            bytes
        );
        for scale in [0.25, 0.5, 0.75] {
            assert_eq!(
                viewer::render_page(&bytes, 0, scale).unwrap(),
                raw_render(&bytes, scale)
            );
        }
        assert_worker_raster(bytes.clone().into(), &bytes, &[0.5]);
        assert_eq!(bytes, original);
    }
}

#[test]
fn readonly_render_clones_shared_managed_resources_without_touching_foreign_images_or_appearances()
{
    let mut doc = Document::load_mem(&fixture()).unwrap();
    let ids = appearance(&doc);
    // The page and a foreign annotation share precisely the resources that
    // must be adjusted for our managed appearance. Every original object must
    // remain intact, including indirect dictionaries and the page's Annots list.
    let mut resources = doc
        .get_object(ids.form)
        .unwrap()
        .as_stream()
        .unwrap()
        .dict
        .get(b"Resources")
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    let xobjects = doc.add_object(
        resources
            .get(b"XObject")
            .unwrap()
            .as_dict()
            .unwrap()
            .clone(),
    );
    resources.set("XObject", xobjects);
    let resources = doc.add_object(resources);
    doc.get_object_mut(ids.form)
        .unwrap()
        .as_stream_mut()
        .unwrap()
        .dict
        .set("Resources", resources);
    let mut foreign = doc.get_dictionary(ids.annotation).unwrap().clone();
    for name in [b"MyPDFState".as_slice(), b"MyPDFData", b"MyPDFApDigest"] {
        foreign.remove(name);
    }
    foreign.set("NM", Object::string_literal("Foreign shared appearance"));
    let foreign = doc.add_object(foreign);
    let annots = doc.add_object(vec![
        Object::Reference(ids.annotation),
        Object::Reference(foreign),
    ]);
    doc.get_dictionary_mut(ids.page)
        .unwrap()
        .set("Annots", annots);
    doc.get_dictionary_mut(ids.page).unwrap().set(
        "Resources",
        dictionary! { "XObject" => dictionary! { "ForeignRaster" => ids.image } },
    );
    let contents = doc.add_object(Stream::new(
        dictionary! {},
        b"q 60 0 0 30 180 5 cm /ForeignRaster Do Q\n".to_vec(),
    ));
    doc.get_dictionary_mut(ids.page)
        .unwrap()
        .set("Contents", contents);
    mark_readonly(&mut doc);
    let bytes = save(&mut doc);
    assert_eq!(
        ann::load(&doc).len(),
        1,
        "the managed integrity marker must remain valid"
    );
    let original_objects = doc.objects.clone();
    let prepared = ann::interpolated_image_render_copy(&bytes, &mut doc).unwrap();
    assert_ne!(prepared, bytes);
    for (id, original) in &original_objects {
        if *id != ids.page {
            assert_eq!(
                doc.objects.get(id),
                Some(original),
                "shared object {id:?} changed"
            );
        }
    }
    let fresh = appearance(&doc);
    assert_ne!(fresh.annotation, ids.annotation);
    assert_ne!(fresh.form, ids.form);
    assert_ne!(fresh.image, ids.image);
    assert_ne!(fresh.mask, ids.mask);
    for id in [fresh.image, fresh.mask] {
        assert!(
            doc.get_object(id)
                .unwrap()
                .as_stream()
                .unwrap()
                .dict
                .get(b"Interpolate")
                .unwrap()
                .as_bool()
                .unwrap()
        );
    }
    for id in [ids.image, ids.mask] {
        assert!(
            doc.get_object(id)
                .unwrap()
                .as_stream()
                .unwrap()
                .dict
                .get(b"Interpolate")
                .is_err()
        );
    }
    let annots = doc
        .get_dictionary(ids.page)
        .unwrap()
        .get(b"Annots")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(annots[1].as_reference().unwrap(), foreign);
    let foreign_appearance = doc
        .get_dictionary(foreign)
        .unwrap()
        .get(b"AP")
        .unwrap()
        .as_dict()
        .unwrap();
    assert_eq!(
        foreign_appearance
            .get(b"N")
            .unwrap()
            .as_reference()
            .unwrap(),
        ids.form
    );
    let mut reference = Document::load_mem(&bytes).unwrap();
    // Change only the managed appearance for an independent raster reference;
    // the foreign page-image region must stay byte-for-byte identical.
    let mut own_form = reference
        .get_object(ids.form)
        .unwrap()
        .as_stream()
        .unwrap()
        .clone();
    let mut own_resources = reference
        .dereference(own_form.dict.get(b"Resources").unwrap())
        .unwrap()
        .1
        .as_dict()
        .unwrap()
        .clone();
    let mut own_xobjects = reference
        .dereference(own_resources.get(b"XObject").unwrap())
        .unwrap()
        .1
        .as_dict()
        .unwrap()
        .clone();
    let mut image = reference
        .get_object(ids.image)
        .unwrap()
        .as_stream()
        .unwrap()
        .clone();
    let mut mask = reference
        .get_object(ids.mask)
        .unwrap()
        .as_stream()
        .unwrap()
        .clone();
    image.dict.set("Interpolate", true);
    mask.dict.set("Interpolate", true);
    image.dict.set("SMask", reference.add_object(mask));
    own_xobjects.set("Im", reference.add_object(image));
    own_resources.set("XObject", own_xobjects);
    own_form.dict.set("Resources", own_resources);
    let form = reference.add_object(own_form);
    let mut own_annotation = reference.get_dictionary(ids.annotation).unwrap().clone();
    own_annotation.set("AP", dictionary! { "N" => form });
    let annotation = reference.add_object(own_annotation);
    reference.get_dictionary_mut(ids.page).unwrap().set(
        "Annots",
        vec![Object::Reference(annotation), Object::Reference(foreign)],
    );
    let reference = save(&mut reference);
    assert_eq!(
        viewer::render_page(&bytes, 0, 0.5).unwrap(),
        raw_render(&reference, 0.5)
    );
    assert_eq!(raw_render(&prepared, 0.5), raw_render(&reference, 0.5));
}

#[test]
fn unrecognized_foreign_image_appearances_keep_original_rendering_choices() {
    let mut doc = Document::load_mem(&fixture()).unwrap();
    let ids = appearance(&doc);
    doc.get_dictionary_mut(ids.annotation)
        .unwrap()
        .remove(b"MyPDFData");
    mark_readonly(&mut doc);
    let foreign = save(&mut doc);
    let mut parsed = Document::load_mem(&foreign).unwrap();
    assert_eq!(
        ann::interpolated_image_render_copy(&foreign, &mut parsed).unwrap(),
        foreign
    );
    for scale in [0.25, 0.5] {
        assert_eq!(
            viewer::render_page(&foreign, 0, scale).unwrap(),
            raw_render(&foreign, scale)
        );
    }
}

#[test]
fn unsigned_page_previews_smooth_managed_images_without_dropping_their_appearance() {
    let source = fixture();
    let original = source.clone();
    let reference = explicit_reference(&source, true);
    assert!(ann::inspect(&Document::load_mem(&source).unwrap()).rewritable());
    for scale in [0.25, 0.5, 0.75, 1.] {
        assert_eq!(
            viewer::render_page(&source, 0, scale).unwrap(),
            raw_render(&reference, scale)
        );
        assert_ne!(
            raw_render(&source, scale).1,
            raw_render(&reference, scale).1
        );
    }
    assert_eq!(source, original);
}

#[test]
fn excessive_managed_appearance_copies_fall_back_to_original_rendering_without_mutating_source() {
    let mut doc = Document::load_mem(&fixture()).unwrap();
    let ids = appearance(&doc);
    doc.get_dictionary_mut(ids.page)
        .unwrap()
        .set("Annots", vec![Object::Reference(ids.annotation); 257]);
    mark_readonly(&mut doc);
    let source = save(&mut doc);
    let original_objects = doc.objects.clone();
    assert!(ann::interpolated_image_render_copy(&source, &mut doc).is_err());
    assert_eq!(
        doc.objects, original_objects,
        "an exceeded preparation budget must not partly modify the document"
    );
    assert_eq!(
        viewer::render_page(&source, 0, 0.5).unwrap(),
        raw_render(&source, 0.5)
    );
    assert_worker_raster(source.clone().into(), &source, &[0.5]);
}
