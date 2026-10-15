use mypdf_milestone0::{annotations as ann, document, identity, signing};
use std::sync::OnceLock;
use underskrift::CryptoSigner;
const PASSWORD: &str = "synthetic-test-password-only";
fn id() -> &'static identity::CreatedIdentity {
    static ID: OnceLock<identity::CreatedIdentity> = OnceLock::new();
    ID.get_or_init(|| {
        identity::create_personal(
            &identity::PersonalId {
                name: "İstanbul Test Signer".into(),
                organization: "MyPDF Tests".into(),
                email: "signer@example.invalid".into(),
                validity_days: 30,
                ecdsa: true,
            },
            PASSWORD,
        )
        .unwrap()
    })
}
fn source(rotation: i32) -> Vec<u8> {
    let source = document::fixture(&document::default_font().unwrap()).unwrap();
    let mut doc = lopdf::Document::load_mem(&source).unwrap();
    let page = doc.get_pages()[&1];
    let dict = doc.get_dictionary_mut(page).unwrap();
    dict.set("Rotate", rotation);
    dict.set(
        "CropBox",
        vec![36.into(), 36.into(), 576.into(), 756.into()],
    );
    let mut bytes = vec![];
    doc.save_to(&mut bytes).unwrap();
    bytes
}
#[test]
fn personal_certificate_profile_import_export_and_public_inspection() {
    let cert = x509_parser::parse_x509_certificate(&id().certificate_der)
        .unwrap()
        .1;
    assert!(cert.subject().to_string().contains("MyPDF Tests"));
    assert!(
        cert.subject_alternative_name()
            .unwrap()
            .unwrap()
            .value
            .general_names
            .iter()
            .any(|n| matches!(
                n,
                x509_parser::extensions::GeneralName::RFC822Name("signer@example.invalid")
            ))
    );
    let entries = identity::inspect_pkcs12(&id().pkcs12, PASSWORD).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].eligible);
    assert!(identity::inspect_pkcs12(&id().pkcs12, "wrong").is_err());
    let signer =
        identity::FileSigner::load(&id().pkcs12, PASSWORD, Some(&entries[0].alias)).unwrap();
    let exported = signer
        .export("Re-exported identity", "new-test-passphrase")
        .unwrap();
    assert!(identity::FileSigner::load(&exported, PASSWORD, None).is_err());
    assert_eq!(
        identity::FileSigner::load(
            &exported,
            "new-test-passphrase",
            Some("Re-exported identity")
        )
        .unwrap()
        .certificate_der(),
        id().certificate_der
    );
    assert_eq!(
        identity::read_certificate(id().certificate_pem.as_bytes()).unwrap(),
        id().certificate_der
    );
    assert_eq!(
        identity::read_certificate(&id().certificate_der).unwrap(),
        id().certificate_der
    );
    let mut invalid = id().certificate_der.clone();
    invalid.push(0);
    assert!(identity::read_certificate(&invalid).is_err());
    let details = identity::PersonalId {
        name: "Test".into(),
        organization: String::new(),
        email: "bad address".into(),
        validity_days: 0,
        ecdsa: true,
    };
    assert!(identity::create_personal(&details, PASSWORD).is_err());
}
#[tokio::test]
async fn custom_appearance_signing_handles_all_page_rotations_and_unicode_metadata() {
    let signer = identity::FileSigner::load(&id().pkcs12, PASSWORD, None).unwrap();
    for rotation in [0, 90, 180, 270] {
        let bytes = source(rotation);
        let request = signing::DesktopSigningRequest {
            field: format!("Rotated{rotation}"),
            page: 0,
            rect: Some([170.0, 370.0, 495.0, 490.0]),
            name: "İstanbul Test Signer".into(),
            reason: "Sözleşme onayı".into(),
            location: "İstanbul".into(),
            artwork: None,
            layout: Default::default(),
            ..Default::default()
        };
        let png = signing::signature_artwork(&request).unwrap();
        assert_eq!(image::load_from_memory(&png).unwrap().width(), 650);
        let signed = signing::sign_custom(&bytes, &signer, &request)
            .await
            .unwrap();
        assert!(signed.starts_with(&bytes));
        let report = signing::verify(&signed, None).unwrap();
        assert_eq!(report.len(), 1);
        assert!(
            report[0].digest_matches
                && report[0].crypto_valid
                && report[0].covers_current_file
                && report[0].certificate_binding_valid
        );
        assert!(!report[0].trusted);
        assert_eq!(
            report[0].signer.as_ref().unwrap().sha256,
            identity::fingerprint(&id().certificate_der)
        );
        assert!(signing::verify(&signed, Some(&id().certificate_der)).unwrap()[0].trusted);
        let parsed = lopdf::Document::load_mem(&signed).unwrap();
        let signature = parsed
            .objects
            .values()
            .filter_map(|o| o.as_dict().ok())
            .find(|d| {
                d.get(b"Type")
                    .and_then(lopdf::Object::as_name)
                    .is_ok_and(|n| n == b"Sig")
            })
            .unwrap();
        assert_eq!(
            signature.get(b"Reason").unwrap().as_str().unwrap(),
            ann::text_string(&request.reason).as_str().unwrap()
        );
        let pdf = hayro::hayro_syntax::Pdf::new(signed).unwrap();
        let raster = hayro::render(
            &pdf.pages()[0],
            &hayro::RenderCache::new(),
            &hayro::hayro_interpret::InterpreterSettings::default(),
            &hayro::RenderSettings::default(),
            &hayro::PixmapSettings {
                x_scale: 1.0,
                y_scale: 1.0,
                bg_color: hayro::vello_cpu::color::palette::css::WHITE,
            },
        );
        let img = image::load_from_memory(&raster.into_png().unwrap())
            .unwrap()
            .into_rgb8();
        // A free page area now contains the signing text. Geometry is in displayed page coordinates.
        let dark = (390..480)
            .flat_map(|y| (185..480).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let p = img.get_pixel(x, y);
                p[0] < 180 && p[1] < 180 && p[2] < 180
            })
            .count();
        assert!(
            dark > 100,
            "Appearance missing at rotation {rotation}: {dark}"
        );
    }
}
#[tokio::test]
async fn custom_multiple_signatures_tamper_detection_and_field_collision_guard() {
    let signer = identity::FileSigner::load(&id().pkcs12, PASSWORD, None).unwrap();
    let source = source(0);
    let first = signing::DesktopSigningRequest {
        field: "First".into(),
        rect: None,
        ..Default::default()
    };
    let signed = signing::sign_custom(&source, &signer, &first)
        .await
        .unwrap();
    let second = signing::DesktopSigningRequest {
        field: "Second".into(),
        rect: None,
        ..Default::default()
    };
    let twice = signing::sign_custom(&signed, &signer, &second)
        .await
        .unwrap();
    assert!(twice.starts_with(&signed));
    let report = signing::verify(&twice, None).unwrap();
    assert_eq!(report.len(), 2);
    assert!(report.iter().all(|s| s.crypto_valid && s.digest_matches));
    assert_eq!(report.iter().filter(|s| s.covers_current_file).count(), 1);
    assert!(
        signing::sign_custom(&twice, &signer, &second)
            .await
            .is_err()
    );
    let mut tampered = twice.clone();
    assert!(tampered.starts_with(b"%PDF-1."));
    tampered[7] = if tampered[7] == b'7' { b'6' } else { b'7' };
    let report = signing::verify(&tampered, None).unwrap();
    assert!(report.iter().all(|s| !s.digest_matches));
    assert!(
        signing::sign_custom(
            &tampered,
            &signer,
            &signing::DesktopSigningRequest {
                field: "Third".into(),
                rect: None,
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    let mut doc = lopdf::Document::load_mem(&source).unwrap();
    doc.add_object(
        lopdf::dictionary! { "FT" => "Tx", "T" => lopdf::Object::string_literal("ExistingField") },
    );
    let mut bytes = vec![];
    doc.save_to(&mut bytes).unwrap();
    assert!(
        signing::sign_custom(
            &bytes,
            &signer,
            &signing::DesktopSigningRequest {
                field: "ExistingField".into(),
                rect: None,
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
}
#[tokio::test]
async fn image_only_appearance_is_transparent_compact_and_signs() {
    let signer = identity::FileSigner::load(&id().pkcs12, PASSWORD, None).unwrap();
    let signature = mypdf_milestone0::assets::Asset::new(
        "Handwritten signature",
        ann::Kind::Ink {
            strokes: vec![vec![[0.0, 0.8], [0.3, 0.1], [0.6, 0.9], [1.0, 0.2]]],
        },
        ann::Style::default(),
        2.5,
    );
    // Smaller than the 120 × 48 minimum that a details layout needs.
    let request = signing::DesktopSigningRequest {
        field: "Handwritten".into(),
        rect: Some([170.0, 370.0, 270.0, 410.0]),
        artwork: Some(signature),
        layout: signing::AppearanceLayout::ImageOnly,
        ..Default::default()
    };
    let png = signing::signature_artwork(&request).unwrap();
    let image = image::load_from_memory(&png).unwrap().into_rgba8();
    assert_eq!(image.width(), 200);
    assert_eq!(image.get_pixel(0, 0)[3], 0, "the page shows through");
    assert!(image.pixels().any(|p| p[3] > 200), "the signature is drawn");
    let signed = signing::sign_custom(&source(0), &signer, &request)
        .await
        .unwrap();
    let report = signing::verify(&signed, None).unwrap();
    assert!(
        report
            .iter()
            .all(|s| s.crypto_valid && s.digest_matches && s.byte_range_valid)
    );
    let without_image = signing::DesktopSigningRequest {
        artwork: None,
        ..request.clone()
    };
    assert!(signing::signature_artwork(&without_image).is_err());
    let details = signing::DesktopSigningRequest {
        layout: signing::AppearanceLayout::DetailsOnly,
        ..request
    };
    assert!(
        signing::signature_artwork(&details).is_err(),
        "details need the larger minimum box"
    );
}
#[test]
fn signature_contents_ending_in_a_zero_byte_are_extracted_whole() {
    use lopdf::{Document, Object, dictionary};
    // DER that legitimately ends in 0x00, in short and long length forms.
    let mut long = vec![0x30, 0x81, 0x80, 0x04, 0x7e];
    long.extend((0..126_u8).map(|i| if i == 125 { 0 } else { i | 1 }));
    for der in [vec![0x30, 0x03, 0x02, 0x01, 0x00], long] {
        let mut doc = Document::with_version("1.7");
        let mut contents = der.clone();
        contents.resize(der.len() + 64, 0); // the zero padding of a signature placeholder
        let sig = doc.add_object(dictionary! {
            "Type" => "Sig",
            "ByteRange" => vec![0.into(), 10.into(), 20.into(), 30.into()],
            "Contents" => Object::String(contents, lopdf::StringFormat::Hexadecimal),
        });
        let field = doc.add_object(dictionary! {
            "FT" => "Sig",
            "T" => Object::string_literal("Signature1"),
            "V" => sig,
        });
        let pages = doc.new_object_id();
        let page = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages,
            "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
        });
        doc.objects.insert(
            pages,
            dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 }.into(),
        );
        let catalog = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages,
            "AcroForm" => dictionary! { "Fields" => vec![field.into()] },
        });
        doc.trailer.set("Root", catalog);
        let mut bytes = vec![];
        doc.save_to(&mut bytes).unwrap();
        let found = underskrift::verify::extractor::extract_signatures(&bytes).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].cms_bytes, der,
            "the final 0x00 is part of the signature"
        );
    }
}
#[tokio::test]
async fn signed_pdfs_edit_incrementally_and_signatures_can_be_removed() {
    use mypdf_milestone0::{content, signature_edit};
    let signer = identity::FileSigner::load(&id().pkcs12, PASSWORD, None).unwrap();
    let first = signing::DesktopSigningRequest {
        field: "First".into(),
        rect: Some([60., 60., 300., 140.]),
        ..Default::default()
    };
    let signed = signing::sign_custom(&source(0), &signer, &first)
        .await
        .unwrap();
    let edit = |bytes: &[u8]| {
        content::add_text(
            bytes,
            1,
            "Approved with changes",
            [50., 300., 400., 330.],
            14.,
            [0.; 3],
        )
    };
    assert!(
        edit(&signed).is_err(),
        "signed PDFs stay locked until unlocked"
    );
    let fields = signature_edit::signature_fields(&signed).unwrap();
    assert_eq!(fields.len(), 1);
    assert!(fields[0].name == "First" && fields[0].signed && fields[0].page == Some(0));
    let r = fields[0].rect.unwrap();
    assert!((r[0] - 60.).abs() < 1. && (r[1] - 60.).abs() < 1. && (r[2] - 300.).abs() < 1.);

    signature_edit::unlock(&signed).unwrap();
    let edited = edit(&signed).unwrap();
    let saved = signature_edit::incremental_update(&signed, &edited).unwrap();
    assert!(saved.starts_with(&signed), "the signed bytes are kept");
    let report = signing::verify(&saved, None).unwrap();
    assert_eq!(report.len(), 1);
    assert!(report[0].crypto_valid && report[0].digest_matches && report[0].byte_range_valid);
    assert!(
        !report[0].covers_current_file,
        "readers see changes after signing"
    );
    assert!(
        document::text_runs(&saved, 1)
            .unwrap()
            .iter()
            .any(|r| r.text == "Approved with changes")
    );
    assert_eq!(
        signature_edit::signed_revision(&saved, "First").unwrap(),
        signed
    );

    // Removing one signature keeps the other valid and leaves an empty field.
    let second = signing::DesktopSigningRequest {
        field: "Second".into(),
        rect: None,
        ..Default::default()
    };
    let twice = signing::sign_custom(&saved, &signer, &second)
        .await
        .unwrap();
    let removed = signature_edit::remove_signature(&twice, "First").unwrap();
    assert!(removed.starts_with(&twice));
    let report = signing::verify(&removed, None).unwrap();
    assert!(report.iter().all(|s| s.field != "First"));
    assert!(
        report
            .iter()
            .any(|s| s.field == "Second" && s.crypto_valid && s.digest_matches)
    );
    let fields = signature_edit::signature_fields(&removed).unwrap();
    assert!(fields.iter().any(|f| f.name == "First" && !f.signed));

    // A removal is an ordinary, undoable edit: later edits rewrite the file, and
    // saving appends both over the signed original.
    signature_edit::unlock(&removed).unwrap();
    let rewritten = edit(&removed).unwrap();
    let saved = signature_edit::incremental_update(&twice, &rewritten).unwrap();
    assert!(saved.starts_with(&twice));
    let report = signing::verify(&saved, None).unwrap();
    assert!(report.iter().all(|s| s.field != "First"));
    assert!(
        report
            .iter()
            .any(|s| s.field == "Second" && s.crypto_valid && s.digest_matches)
    );
}

#[tokio::test]
async fn redaction_refuses_signed_pdfs_whose_signed_revision_keeps_the_original() {
    use mypdf_milestone0::redact::{self, Area, Options};
    let signer = identity::FileSigner::load(&id().pkcs12, PASSWORD, None).unwrap();
    let signed = signing::sign_custom(&source(0), &signer, &Default::default())
        .await
        .unwrap();
    let error = redact::apply(
        &signed,
        &[Area {
            page: 0,
            rect: [50., 50., 200., 80.],
        }],
        &Options::default(),
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("signed"), "{error:#}");
}

#[tokio::test]
async fn flattened_pdfs_can_be_signed_and_signed_pdfs_are_not_flattened() {
    use mypdf_milestone0::redact;
    let (flat, _) = redact::flatten_document(&source(0), 100.).unwrap();
    let signer = identity::FileSigner::load(&id().pkcs12, PASSWORD, None).unwrap();
    let signed = signing::sign_custom(&flat, &signer, &Default::default())
        .await
        .unwrap();
    let report = signing::verify(&signed, None).unwrap();
    assert!(report[0].crypto_valid && report[0].digest_matches && report[0].covers_current_file);
    let error = redact::flatten_document(&signed, 100.).unwrap_err();
    assert!(format!("{error:#}").contains("signatures"), "{error:#}");
}

/// Acrobat fails to validate visible signatures whose appearance draws an image
/// directly ("Expected a dict object"); it expects its layered /FRM, /n0, /n2
/// structure. Incremental updates must also repeat the trailer's /Info.
#[tokio::test]
async fn visible_signatures_use_acrobat_layered_appearances_and_keep_the_trailer() {
    use lopdf::{Document, Object, dictionary};
    let mut doc = Document::load_mem(&source(0)).unwrap();
    let info = doc.add_object(dictionary! { "Title" => Object::string_literal("Layered") });
    doc.trailer.set("Info", info);
    let mut with_info = vec![];
    doc.save_to(&mut with_info).unwrap();

    let signer = identity::FileSigner::load(&id().pkcs12, PASSWORD, None).unwrap();
    let request = signing::DesktopSigningRequest {
        field: "Layered".into(),
        rect: Some([72., 100., 312., 172.]),
        name: "Layered Test".into(),
        ..Default::default()
    };
    let signed = signing::sign_custom(&with_info, &signer, &request)
        .await
        .unwrap();
    let report = signing::verify(&signed, None).unwrap();
    assert!(report[0].crypto_valid && report[0].digest_matches && report[0].covers_current_file);

    // The update's own trailer repeats /Info.
    let last = signed.windows(7).rposition(|w| w == b"trailer").unwrap();
    let trailer = String::from_utf8_lossy(&signed[last..]);
    assert!(trailer.contains("/Info"), "{trailer}");

    let doc = Document::load_mem(&signed).unwrap();
    let page = doc.get_pages()[&1];
    let widget = doc
        .objects
        .values()
        .filter_map(|o| o.as_dict().ok())
        .find(|d| d.get(b"FT").and_then(Object::as_name).ok() == Some(b"Sig"))
        .expect("the signature widget");
    assert_eq!(widget.get(b"P").unwrap().as_reference().unwrap(), page);
    let xobjects = |id: lopdf::ObjectId| -> lopdf::Dictionary {
        let stream = doc.get_object(id).unwrap().as_stream().unwrap();
        let resources = stream.dict.get(b"Resources").unwrap().as_dict().unwrap();
        resources
            .get(b"XObject")
            .unwrap()
            .as_dict()
            .unwrap()
            .clone()
    };
    let normal = widget
        .get(b"AP")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"N")
        .unwrap()
        .as_reference()
        .unwrap();
    let top = xobjects(normal);
    assert_eq!(top.len(), 1);
    let frm = top.get(b"FRM").unwrap().as_reference().unwrap();
    let layers = xobjects(frm);
    let n2 = layers.get(b"n2").unwrap().as_reference().unwrap();
    assert!(layers.get(b"n0").is_ok());
    let artwork = xobjects(n2);
    assert!(artwork.iter().any(|(_, v)| {
        v.as_reference()
            .ok()
            .and_then(|r| doc.get_object(r).ok())
            .and_then(|o| o.as_stream().ok())
            .and_then(|s| s.dict.get(b"Subtype").ok())
            .and_then(|s| s.as_name().ok())
            == Some(b"Image")
    }));
}

#[tokio::test]
async fn signed_pdfs_can_be_reordered_after_unlock_and_copied_without_their_signatures() {
    use mypdf_milestone0::{pages, signature_edit};
    let signer = identity::FileSigner::load(&id().pkcs12, PASSWORD, None).unwrap();
    let request = signing::DesktopSigningRequest {
        field: "Reviewer".into(),
        rect: Some([72., 100., 312., 172.]),
        ..Default::default()
    };
    let signed = signing::sign_custom(&source(0), &signer, &request)
        .await
        .unwrap();

    // Copied pages are not signed: no signature, and no box pretending to be one.
    let copy = pages::extract(&signed, &[0]).unwrap();
    assert!(signing::verify(&copy, None).unwrap().is_empty());
    assert!(signature_edit::signature_fields(&copy).unwrap().is_empty());
    assert!(!String::from_utf8_lossy(&copy).contains("/FT /Sig"));

    // Locked until "Edit anyway"; then pages reorder and save after the signed bytes.
    assert!(pages::apply(&signed, &pages::Operation::Reorder(vec![1, 0])).is_err());
    signature_edit::unlock(&signed).unwrap();
    let reordered = pages::apply(&signed, &pages::Operation::Reorder(vec![1, 0])).unwrap();
    let saved = signature_edit::incremental_update(&signed, &reordered).unwrap();
    assert!(saved.starts_with(&signed));
    let report = signing::verify(&saved, None).unwrap();
    assert!(report[0].crypto_valid && report[0].digest_matches && !report[0].covers_current_file);
    let fields = signature_edit::signature_fields(&saved).unwrap();
    assert_eq!(fields[0].page, Some(1), "the signature moved with its page");
}

#[test]
fn signature_backgrounds_paint_what_their_names_say() {
    use mypdf_milestone0::signing::{Background, DesktopSigningRequest, signature_artwork};
    // Pixels at the frame (left edge) and in empty space (right edge), mid-height.
    let sample = |background| {
        let png = signature_artwork(&DesktopSigningRequest {
            rect: Some([0., 0., 200., 60.]),
            name: "Jane".into(),
            background,
            ..Default::default()
        })
        .unwrap();
        let image = mypdf_milestone0::assets::decode_png(&png)
            .unwrap()
            .into_rgba8();
        let y = image.height() / 2;
        (
            image.get_pixel(2, y).0,
            image.get_pixel(image.width() - 8, y).0,
        )
    };
    let (_, clear) = sample(Background::Clear);
    assert_eq!(clear[3], 0, "the page shows through");
    let (_, white) = sample(Background::White);
    assert_eq!(white, [255, 255, 255, 255]);
    let (_, tint) = sample(Background::Tinted);
    assert!(
        tint[3] == 255 && tint[2] > tint[0] && tint[0] < 245,
        "{tint:?}"
    );
    let (frame, inside) = sample(Background::Framed);
    assert_eq!(inside, [255, 255, 255, 255]);
    assert!(
        frame[1] > frame[0] && frame[0] < 200,
        "a green frame: {frame:?}"
    );
}
