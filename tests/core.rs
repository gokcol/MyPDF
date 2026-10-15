use mypdf_milestone0::{document, identity, signing, write_new};
use std::sync::OnceLock;
use underskrift::CryptoSigner;

const PASSWORD: &str = "synthetic-test-password-only";
fn fixture() -> &'static Vec<u8> {
    static PDF: OnceLock<Vec<u8>> = OnceLock::new();
    PDF.get_or_init(|| document::fixture(&document::default_font().unwrap()).unwrap())
}
fn test_identity() -> &'static identity::CreatedIdentity {
    static ID: OnceLock<identity::CreatedIdentity> = OnceLock::new();
    ID.get_or_init(|| identity::create("MyPDF test identity", PASSWORD, true).unwrap())
}

#[test]
fn replace_actual_text_without_changing_shared_page() {
    let before = fixture();
    let after =
        document::replace_run(before, 1, document::ORIGINAL, document::REPLACEMENT).unwrap();
    let text = lopdf::Document::load_mem(&after)
        .unwrap()
        .extract_text(&[1])
        .unwrap();
    assert!(text.contains(document::REPLACEMENT));
    assert!(!text.contains(document::ORIGINAL));
    assert_eq!(
        lopdf::Document::load_mem(before)
            .unwrap()
            .extract_text(&[2])
            .unwrap(),
        lopdf::Document::load_mem(&after)
            .unwrap()
            .extract_text(&[2])
            .unwrap()
    );
    let original = lopdf::Document::load_mem(before).unwrap();
    let edited = lopdf::Document::load_mem(&after).unwrap();
    let before_image = original
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .find(|s| {
            s.dict
                .get(b"Subtype")
                .and_then(lopdf::Object::as_name)
                .is_ok_and(|v| v == b"Image")
        })
        .unwrap();
    let after_image = edited
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .find(|s| {
            s.dict
                .get(b"Subtype")
                .and_then(lopdf::Object::as_name)
                .is_ok_and(|v| v == b"Image")
        })
        .unwrap();
    assert_eq!(before_image.content, after_image.content);
}

#[test]
fn replace_unicode_cmap_and_split_tj_text() {
    let edited = document::replace_run(
        fixture(),
        1,
        document::TURKISH_ORIGINAL,
        document::TURKISH_REPLACEMENT,
    )
    .unwrap();
    assert!(
        document::text_runs(&edited, 1)
            .unwrap()
            .iter()
            .any(|r| r.text == document::TURKISH_REPLACEMENT)
    );
    let edited =
        document::replace_run(&edited, 1, "Split text example", "Split text edited").unwrap();
    assert!(
        document::text_runs(&edited, 1)
            .unwrap()
            .iter()
            .any(|r| r.text == "Split text edited" && r.operator == "TJ")
    );
}

#[test]
fn unavailable_glyph_missing_text_and_invalid_page_fail() {
    // U+0378 is unassigned, so no font has it. (Emoji are not a test of this:
    // Windows has an emoji font that can be embedded; macOS has not.)
    assert!(document::replace_run(fixture(), 1, document::ORIGINAL, "Invoice \u{0378}").is_err());
    assert!(document::replace_run(fixture(), 1, "missing text", "hello").is_err());
    assert!(document::replace_run(fixture(), 0, document::ORIGINAL, "hello").is_err());
    assert!(document::replace_run(fixture(), 99, document::ORIGINAL, "hello").is_err());
    assert!(document::replace_run(fixture(), 1, document::ORIGINAL, "line\nbreak").is_err());
}

#[test]
fn pkcs12_authentication_and_certificate_binding() {
    let id = test_identity();
    let signer = identity::FileSigner::load(&id.pkcs12, PASSWORD, None).unwrap();
    assert_eq!(signer.certificate_der(), id.certificate_der);
    assert!(identity::FileSigner::load(&id.pkcs12, "wrong", None).is_err());
    let mut damaged = id.pkcs12.clone();
    let last = damaged.len() - 1;
    damaged[last] ^= 1;
    assert!(identity::FileSigner::load(&damaged, PASSWORD, None).is_err());
    assert!(identity::FileSigner::load(&id.pkcs12, PASSWORD, Some("missing")).is_err());
}

#[test]
fn certificate_without_private_key_cannot_sign() {
    let mut store = p12_keystore::KeyStore::new();
    store.add_entry(
        "public certificate",
        p12_keystore::KeyStoreEntry::Certificate(
            p12_keystore::Certificate::from_der(&test_identity().certificate_der).unwrap(),
        ),
    );
    let data = store.writer(PASSWORD).write().unwrap();
    assert!(identity::FileSigner::load(&data, PASSWORD, None).is_err());
}

#[test]
fn ecdsa_signs_digest_once_and_rejects_wrong_digest() {
    let signer = identity::FileSigner::load(&test_identity().pkcs12, PASSWORD, None).unwrap();
    let hash = signer.digest_algorithm().digest(b"single hash test");
    let signature = signer.sign_hash(&hash).unwrap();
    identity::verify_hash(signer.certificate_der(), &hash, &signature).unwrap();
    let wrong = signer.digest_algorithm().digest(b"changed message");
    assert!(identity::verify_hash(signer.certificate_der(), &wrong, &signature).is_err());
    assert!(signer.sign_hash(&[0; 4]).is_err());
}

#[tokio::test]
async fn incremental_signatures_and_signed_edit_guard() {
    let signer = identity::FileSigner::load(&test_identity().pkcs12, PASSWORD, None).unwrap();
    let signed = signing::sign(fixture(), &signer, "First", false)
        .await
        .unwrap();
    assert!(signed.starts_with(fixture()));
    let twice = signing::sign(&signed, &signer, "Second", false)
        .await
        .unwrap();
    assert!(twice.starts_with(&signed));
    let parsed = lopdf::Document::load_mem(&twice).unwrap();
    assert_eq!(
        parsed.trailer.get(b"Size").unwrap().as_i64().unwrap(),
        i64::from(parsed.objects.keys().map(|id| id.0).max().unwrap()) + 1
    );
    let report = signing::verify(&twice, None).unwrap();
    assert_eq!(report.len(), 2);
    assert!(report.iter().all(|s| {
        s.claimed_signing_time
            .as_ref()
            .is_some_and(|t| t.starts_with("D:") && !t.starts_with("D:1970"))
    }));
    assert!(
        report
            .iter()
            .all(|s| s.crypto_valid && s.digest_matches && !s.trusted)
    );
    assert!(document::replace_run(&signed, 1, document::ORIGINAL, document::REPLACEMENT).is_err());
    assert!(
        signing::sign(&signed, &signer, "First", false)
            .await
            .is_err()
    );
}

#[test]
fn output_publication_never_overwrites_existing_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("output.pdf");
    write_new(&path, b"original").unwrap();
    assert!(write_new(&path, b"replacement").is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"original");
}

#[test]
fn expired_and_inappropriate_certificates_are_rejected() {
    let key = rcgen::KeyPair::generate().unwrap();
    let mut parameters = rcgen::CertificateParams::default();
    parameters.not_before = time::OffsetDateTime::now_utc() - time::Duration::days(2);
    parameters.not_after = time::OffsetDateTime::now_utc() - time::Duration::days(1);
    parameters.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
    let expired = parameters.self_signed(&key).unwrap();
    assert!(identity::check_certificate(expired.der()).is_err());
    let mut parameters = rcgen::CertificateParams::default();
    parameters.key_usages = vec![rcgen::KeyUsagePurpose::KeyEncipherment];
    let unsuitable = parameters.self_signed(&key).unwrap();
    assert!(identity::check_certificate(unsuitable.der()).is_err());
}

#[test]
fn ambiguous_text_replacement_is_rejected() {
    let mut doc = lopdf::Document::load_mem(fixture()).unwrap();
    let page = doc.get_pages()[&1];
    let mut content = doc.get_and_decode_page_content(page).unwrap();
    let duplicate: Vec<_> = content
        .operations
        .iter()
        .filter(|op| {
            op.operator == "Tj"
                && op
                    .operands
                    .first()
                    .is_some_and(|o| o.as_str().is_ok_and(|s| s == document::ORIGINAL.as_bytes()))
        })
        .cloned()
        .collect();
    assert_eq!(duplicate.len(), 1);
    content.operations.extend(duplicate);
    doc.change_page_content(page, content.encode().unwrap())
        .unwrap();
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    assert!(document::replace_run(&bytes, 1, document::ORIGINAL, document::REPLACEMENT).is_err());
}

#[tokio::test]
async fn stream_inputs_require_explicit_unsigned_preparation() {
    let mut doc = lopdf::Document::load_mem(fixture()).unwrap();
    doc.reference_table.cross_reference_type = lopdf::xref::XrefType::CrossReferenceStream;
    let mut stream_pdf = Vec::new();
    doc.save_to(&mut stream_pdf).unwrap();
    let signer = identity::FileSigner::load(&test_identity().pkcs12, PASSWORD, None).unwrap();
    assert!(
        signing::sign(&stream_pdf, &signer, "RejectedStream", false)
            .await
            .is_err()
    );
    let prepared = document::prepare_signing_copy(&stream_pdf).unwrap();
    assert_eq!(
        lopdf::Document::load_mem(&prepared)
            .unwrap()
            .extract_text(&[1, 2])
            .unwrap(),
        doc.extract_text(&[1, 2]).unwrap()
    );
    let signed = signing::sign(&prepared, &signer, "Prepared", false)
        .await
        .unwrap();
    assert!(document::prepare_signing_copy(&signed).is_err());
}
