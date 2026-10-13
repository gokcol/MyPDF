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
