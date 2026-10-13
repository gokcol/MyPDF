//! macOS signing uses opaque SecKey handles. No private-key export is performed.
use anyhow::{Context, Result, ensure};
use core_foundation::{
    array::{CFArray, CFArrayRef},
    base::TCFType,
};
use security_framework::{
    identity::SecIdentity,
    import_export::Pkcs12ImportOptions,
    item::{ItemClass, ItemSearchOptions, Limit, Reference, SearchResult},
    key::{Algorithm, SecKey},
    os::macos::keychain::{CreateOptions, SecKeychain},
};
use serde::Serialize;
use std::path::Path;
use underskrift::{CryptoSigner, DigestAlgorithm, SignatureAlgorithm, error::CryptoError};

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecKeychainDelete(keychain: security_framework_sys::base::SecKeychainRef) -> i32;
    fn SecKeychainCopySearchList(search_list: *mut CFArrayRef) -> i32;
    fn SecKeychainSetSearchList(search_list: CFArrayRef) -> i32;
}

pub struct KeychainSigner {
    key: SecKey,
    certificate: Vec<u8>,
    algorithm: SignatureAlgorithm,
}
impl KeychainSigner {
    pub fn new(identity: SecIdentity) -> Result<Self> {
        let certificate = identity.certificate()?.to_der();
        crate::identity::check_certificate(&certificate)?;
        let algorithm = crate::identity::algorithm(&certificate)?;
        let signer = Self {
            key: identity.private_key()?,
            certificate,
            algorithm,
        };
        let digest = crate::identity::digest_bytes(
            signer.digest_algorithm(),
            b"MyPDF native Keychain identity binding check",
        );
        crate::identity::verify_hash(
            signer.certificate_der(),
            &digest,
            &signer.sign_hash(&digest)?,
        )?;
        Ok(signer)
    }
}
impl CryptoSigner for KeychainSigner {
    fn certificate_der(&self) -> &[u8] {
        &self.certificate
    }
    fn certificate_chain_der(&self) -> Vec<&[u8]> {
        vec![&self.certificate]
    }
    fn digest_algorithm(&self) -> DigestAlgorithm {
        if self.algorithm == SignatureAlgorithm::EcdsaP384 {
            DigestAlgorithm::Sha384
        } else {
            DigestAlgorithm::Sha256
        }
    }
    fn signature_algorithm(&self) -> SignatureAlgorithm {
        self.algorithm
    }
    fn sign_hash(&self, hash: &[u8]) -> std::result::Result<Vec<u8>, CryptoError> {
        let alg = match self.algorithm {
            SignatureAlgorithm::RsaPkcs1v15 => Algorithm::RSASignatureDigestPKCS1v15SHA256,
            SignatureAlgorithm::EcdsaP256 => Algorithm::ECDSASignatureDigestX962SHA256,
            SignatureAlgorithm::EcdsaP384 => Algorithm::ECDSASignatureDigestX962SHA384,
            _ => {
                return Err(CryptoError::UnsupportedKeyType(
                    "unsupported Keychain algorithm".into(),
                ));
            }
        };
        self.key.create_signature(alg, hash).map_err(|e| {
            CryptoError::SigningFailed(format!("Keychain authorization/signing failed: {e}"))
        })
    }
}

fn identities(keychain: Option<&SecKeychain>) -> Result<Vec<SecIdentity>> {
    let mut query = ItemSearchOptions::new();
    query
        .class(ItemClass::identity())
        .load_refs(true)
        .limit(Limit::All);
    if let Some(keychain) = keychain {
        query.keychains(std::slice::from_ref(keychain));
    }
    match query.search() {
        Ok(items) => Ok(items
            .into_iter()
            .filter_map(|i| {
                if let SearchResult::Ref(Reference::Identity(id)) = i {
                    Some(id)
                } else {
                    None
                }
            })
            .collect()),
        Err(e) if e.code() == -25300 => Ok(vec![]),
        Err(e) => Err(e.into()),
    }
}

#[derive(Serialize)]
pub struct StoredIdentity {
    pub certificate: crate::identity::IdentityInfo,
    #[serde(skip_serializing)]
    pub certificate_der: Vec<u8>,
    pub eligible: bool,
}
pub fn list(path: Option<&Path>) -> Result<Vec<StoredIdentity>> {
    let keychain = path.map(SecKeychain::open).transpose()?;
    identities(keychain.as_ref())?
        .iter()
        .map(|id| {
            let der = id.certificate()?.to_der();
            Ok(StoredIdentity {
                certificate: crate::identity::info(&der)?,
                certificate_der: der.clone(),
                eligible: crate::identity::check_certificate(&der).is_ok(),
            })
        })
        .collect()
}
pub fn select(path: Option<&Path>, sha256: &str) -> Result<KeychainSigner> {
    let keychain = path.map(SecKeychain::open).transpose()?;
    let matches: Vec<_> = identities(keychain.as_ref())?
        .into_iter()
        .filter(|id| {
            id.certificate().is_ok_and(|c| {
                crate::identity::fingerprint(&c.to_der()).eq_ignore_ascii_case(sha256)
            })
        })
        .collect();
    ensure!(
        matches.len() == 1,
        "expected exactly one matching Keychain identity, found {}",
        matches.len()
    );
    KeychainSigner::new(matches.into_iter().next().unwrap())
}

/// Isolate generated proof credentials from the user's login keychain. Apple's
/// legacy keychain creation may alter its search list, so preserve and restore it.
pub struct TemporaryKeychain {
    keychain: SecKeychain,
    original_search_list: CFArray<SecKeychain>,
    closed: bool,
}
impl TemporaryKeychain {
    pub fn create(path: &Path, password: &str) -> Result<Self> {
        let mut raw = std::ptr::null();
        // SAFETY: output is valid storage; success returns a retained CFArray.
        let status = unsafe { SecKeychainCopySearchList(&mut raw) };
        ensure!(
            status == 0 && !raw.is_null(),
            "cannot snapshot Keychain search list ({status})"
        );
        let original_search_list = unsafe { CFArray::<SecKeychain>::wrap_under_create_rule(raw) };
        let mut options = CreateOptions::new();
        options.password(password).prompt_user(false);
        let keychain = options
            .create(path)
            .context("cannot create isolated test Keychain")?;
        Ok(Self {
            keychain,
            original_search_list,
            closed: false,
        })
    }
    pub fn import_and_select(&self, pkcs12: &[u8], password: &str) -> Result<KeychainSigner> {
        let imported = Pkcs12ImportOptions::new()
            .keychain(self.keychain.clone())
            .passphrase(password)
            .import(pkcs12)
            .context("native PKCS#12 import into isolated test Keychain failed")?;
        ensure!(
            imported.iter().filter(|i| i.identity.is_some()).count() == 1,
            "native import did not produce one identity"
        );
        // Rediscover from the actual keychain, rather than using import's handle.
        let stored = identities(Some(&self.keychain))?;
        ensure!(
            stored.len() == 1,
            "test identity was not found in its Keychain"
        );
        KeychainSigner::new(stored.into_iter().next().unwrap())
    }
    pub fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        // SAFETY: both CF objects are retained by this guard throughout the calls.
        let deleted = unsafe { SecKeychainDelete(self.keychain.as_concrete_TypeRef()) };
        let restored =
            unsafe { SecKeychainSetSearchList(self.original_search_list.as_concrete_TypeRef()) };
        self.closed = deleted == 0 && restored == 0;
        ensure!(
            self.closed,
            "isolated Keychain cleanup failed (delete={deleted}, restore={restored})"
        );
        let mut raw = std::ptr::null();
        // SAFETY: success returns a retained array; compare native CF values.
        let status = unsafe { SecKeychainCopySearchList(&mut raw) };
        ensure!(
            status == 0 && !raw.is_null(),
            "cannot verify restored search list ({status})"
        );
        let restored = unsafe { CFArray::<SecKeychain>::wrap_under_create_rule(raw) };
        ensure!(
            restored.as_CFType() == self.original_search_list.as_CFType(),
            "Keychain search list changed after cleanup"
        );
        Ok(())
    }
}
impl Drop for TemporaryKeychain {
    fn drop(&mut self) {
        if let Err(e) = self.close() {
            eprintln!("Keychain cleanup: {e}");
        }
    }
}
