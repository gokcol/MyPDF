//! Password-protected PDFs: opening them with their password, and protecting
//! a copy with AES-256 (PDF 2.0 security handler, revision 6).
use anyhow::{Context, Result, ensure};
use lopdf::encryption::crypt_filters::{Aes256CryptFilter, CryptFilter};
use lopdf::{Document, EncryptionState, EncryptionVersion, Permissions};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Whether the PDF asks for a password before it can be opened.
pub fn needs_password(bytes: &[u8]) -> bool {
    matches!(
        hayro::hayro_syntax::Pdf::new(bytes.to_vec()),
        Err(hayro::hayro_syntax::LoadPdfError::Decryption(_))
    )
}

/// The PDF's content without its protection, opened with `password`.
pub fn unlock(bytes: &[u8], password: &str) -> Result<Vec<u8>> {
    ensure!(
        hayro::hayro_syntax::Pdf::new_with_password(bytes.to_vec(), password).is_ok(),
        "That password is not right. Try again."
    );
    let mut doc =
        Document::load_mem_with_options(bytes, lopdf::LoadOptions::with_password(password))
            .context("That password is not right. Try again.")?;
    ensure!(
        !doc.is_encrypted(),
        "That password is not right. Try again."
    );
    doc.trailer.remove(b"Encrypt");
    crate::content::serialize(&mut doc)
}

/// A copy that opens only with `password`. Anyone who opens it may print,
/// copy and edit it.
pub fn protect(bytes: &[u8], password: &str) -> Result<Vec<u8>> {
    ensure!(
        !password.is_empty() && password.chars().count() <= 127,
        "Choose a password of 1 to 127 characters"
    );
    let mut doc = Document::load_mem(bytes)?;
    ensure!(
        !doc.is_encrypted(),
        "This PDF is already protected. Remove its password first."
    );
    ensure!(
        !crate::document::has_signatures(&doc),
        "Protecting a signed PDF would rewrite it and break its signatures"
    );
    // Keep the file identifier readers use, or give the file one.
    if doc.trailer.get(b"ID").is_err() {
        let id = lopdf::Object::String(
            rand::random::<[u8; 16]>().to_vec(),
            lopdf::StringFormat::Hexadecimal,
        );
        doc.trailer.set("ID", vec![id.clone(), id]);
    }
    let key: [u8; 32] = rand::random();
    let owner = format!("{:032x}", rand::random::<u128>());
    let filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
    let state = EncryptionState::try_from(EncryptionVersion::V5 {
        encrypt_metadata: true,
        crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
        file_encryption_key: &key,
        stream_filter: b"StdCF".to_vec(),
        string_filter: b"StdCF".to_vec(),
        owner_password: &owner,
        user_password: password,
        permissions: Permissions::all(),
    })
    .context("Could not set up the password")?;
    doc.reference_table.cross_reference_type = lopdf::xref::XrefType::CrossReferenceTable;
    doc.prune_objects();
    doc.encrypt(&state).context("Could not protect the PDF")?;
    doc.trailer.remove(b"Prev");
    doc.trailer.remove(b"XRefStm");
    let mut out = vec![];
    doc.save_to(&mut out)?;
    Ok(out)
}
