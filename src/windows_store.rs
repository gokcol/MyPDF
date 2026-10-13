//! Current-user Windows Personal certificates, with non-exporting CNG/CryptoAPI signing.
//! Discovery reads public metadata only; key acquisition happens only when signing.
use anyhow::{Context as _, Result, ensure};
use std::ptr::{null, null_mut};
use underskrift::{CryptoSigner, DigestAlgorithm, SignatureAlgorithm, error::CryptoError};
use windows_sys::Win32::{Foundation::GetLastError, Security::Cryptography::*};

struct Store(HCERTSTORE);
impl Drop for Store {
    fn drop(&mut self) {
        unsafe {
            CertCloseStore(self.0, 0);
        }
    }
}
struct Certificate(*const CERT_CONTEXT);
impl Drop for Certificate {
    fn drop(&mut self) {
        unsafe {
            CertFreeCertificateContext(self.0);
        }
    }
}
impl Certificate {
    fn der(&self) -> &[u8] {
        // SAFETY: this wrapper owns a retained certificate context for this lifetime.
        unsafe {
            std::slice::from_raw_parts((*self.0).pbCertEncoded, (*self.0).cbCertEncoded as usize)
        }
    }
    fn has_key_reference(&self) -> bool {
        [CERT_KEY_PROV_INFO_PROP_ID, CERT_KEY_CONTEXT_PROP_ID]
            .into_iter()
            .any(|property| {
                let mut size = 0;
                // Query only the property's size. Do not acquire/unlock the private key.
                unsafe {
                    CertGetCertificateContextProperty(self.0, property, null_mut(), &mut size) != 0
                }
            })
    }
}
fn certificates() -> Result<Vec<Certificate>> {
    let name: Vec<u16> = "MY\0".encode_utf16().collect();
    // SAFETY: a NUL-terminated UTF-16 store name, no provider handle; read-only access.
    let store = Store(unsafe {
        CertOpenStore(
            CERT_STORE_PROV_SYSTEM_W,
            0,
            0,
            CERT_SYSTEM_STORE_CURRENT_USER
                | CERT_STORE_READONLY_FLAG
                | CERT_STORE_OPEN_EXISTING_FLAG,
            name.as_ptr().cast(),
        )
    });
    ensure!(
        !store.0.is_null(),
        "Cannot open Windows Personal store: {}",
        std::io::Error::last_os_error()
    );
    let mut current = null();
    let mut result = vec![];
    loop {
        // Enumeration consumes the previous context. Retain independent copies below.
        current = unsafe { CertEnumCertificatesInStore(store.0, current) };
        if current.is_null() {
            let error = unsafe { GetLastError() };
            ensure!(
                error == 0x80092004,
                "Cannot enumerate Windows certificates: {error:#x}"
            );
            break;
        }
        let owned = unsafe { CertDuplicateCertificateContext(current) };
        if owned.is_null() {
            unsafe {
                CertFreeCertificateContext(current);
            }
            anyhow::bail!("Cannot retain Windows certificate context");
        }
        result.push(Certificate(owned));
        if result.len() > 10_000 {
            unsafe {
                CertFreeCertificateContext(current);
            }
            anyhow::bail!("Windows Personal store contains too many certificates");
        }
    }
    Ok(result)
}
pub struct StoredIdentity {
    pub certificate: crate::identity::IdentityInfo,
    pub certificate_der: Vec<u8>,
    pub eligible: bool,
}
pub fn list() -> Result<Vec<StoredIdentity>> {
    Ok(certificates()?
        .into_iter()
        .filter(|c| c.has_key_reference())
        .filter_map(|c| {
            let der = c.der();
            Some(StoredIdentity {
                certificate: crate::identity::info(der).ok()?,
                certificate_der: der.to_vec(),
                eligible: crate::identity::check_certificate(der).is_ok(),
            })
        })
        .collect())
}
fn find(fingerprint: &str) -> Result<Certificate> {
    let mut matches: Vec<_> = certificates()?
        .into_iter()
        .filter(|c| {
            c.has_key_reference()
                && crate::identity::fingerprint(c.der()).eq_ignore_ascii_case(fingerprint)
        })
        .collect();
    ensure!(
        matches.len() == 1,
        "Expected exactly one matching Windows signing identity; found {}",
        matches.len()
    );
    Ok(matches.remove(0))
}
pub struct WindowsSigner {
    certificate: Vec<u8>,
    fingerprint: String,
    algorithm: SignatureAlgorithm,
}
pub fn select(fingerprint: &str) -> Result<WindowsSigner> {
    let certificate = find(fingerprint)?.der().to_vec();
    crate::identity::check_certificate(&certificate)?;
    let algorithm = crate::identity::algorithm(&certificate)?;
    Ok(WindowsSigner {
        certificate,
        fingerprint: fingerprint.into(),
        algorithm,
    })
}
struct Key {
    handle: usize,
    spec: u32,
    owned: bool,
}
impl Drop for Key {
    fn drop(&mut self) {
        if self.owned {
            // SAFETY: the API explicitly grants ownership through caller_free.
            unsafe {
                if self.spec == CERT_NCRYPT_KEY_SPEC {
                    NCryptFreeObject(self.handle);
                } else {
                    CryptReleaseContext(self.handle, 0);
                }
            }
        }
    }
}
struct Hash(usize);
impl Drop for Hash {
    fn drop(&mut self) {
        unsafe {
            CryptDestroyHash(self.0);
        }
    }
}
impl WindowsSigner {
    fn sign(&self, digest: &[u8]) -> Result<Vec<u8>> {
        ensure!(
            digest.len()
                == if self.algorithm == SignatureAlgorithm::EcdsaP384 {
                    48
                } else {
                    32
                },
            "Incorrect signing digest length"
        );
        let cert = find(&self.fingerprint)?;
        ensure!(
            cert.der() == self.certificate,
            "The selected certificate has changed"
        );
        crate::identity::check_certificate(cert.der())?;
        let mut handle = 0;
        let mut spec = 0;
        let mut caller_free = 0;
        // SAFETY: context and output storage remain live until after the key is released.
        // Native PIN/consent prompts are allowed only here, during explicit signing.
        ensure!(
            unsafe {
                CryptAcquireCertificatePrivateKey(
                    cert.0,
                    CRYPT_ACQUIRE_PREFER_NCRYPT_KEY_FLAG
                        | CRYPT_ACQUIRE_COMPARE_KEY_FLAG
                        | CRYPT_ACQUIRE_NO_HEALING,
                    null(),
                    &mut handle,
                    &mut spec,
                    &mut caller_free,
                )
            } != 0,
            "Windows key authorization failed: {}",
            std::io::Error::last_os_error()
        );
        let key = Key {
            handle,
            spec,
            owned: caller_free != 0,
        };
        let mut length = 0;
        let mut signature;
        if spec == CERT_NCRYPT_KEY_SPEC {
            let sha256: Vec<u16> = "SHA256\0".encode_utf16().collect();
            let padding = BCRYPT_PKCS1_PADDING_INFO {
                pszAlgId: sha256.as_ptr(),
            };
            let rsa = self.algorithm == SignatureAlgorithm::RsaPkcs1v15;
            let padding_ptr = if rsa {
                (&padding as *const BCRYPT_PKCS1_PADDING_INFO).cast()
            } else {
                null()
            };
            let flags = if rsa { NCRYPT_PAD_PKCS1_FLAG } else { 0 };
            let status = unsafe {
                NCryptSignHash(
                    key.handle,
                    padding_ptr,
                    digest.as_ptr(),
                    digest.len() as u32,
                    null_mut(),
                    0,
                    &mut length,
                    flags,
                )
            };
            ensure!(
                status == 0 && (1..=16_384).contains(&length),
                "Windows signing failed: {status:#x}"
            );
            signature = vec![0; length as usize];
            let status = unsafe {
                NCryptSignHash(
                    key.handle,
                    padding_ptr,
                    digest.as_ptr(),
                    digest.len() as u32,
                    signature.as_mut_ptr(),
                    signature.len() as u32,
                    &mut length,
                    flags,
                )
            };
            ensure!(status == 0, "Windows signing failed: {status:#x}");
            signature.truncate(length as usize);
            signature = match self.algorithm {
                SignatureAlgorithm::EcdsaP256 => p256::ecdsa::Signature::from_slice(&signature)?
                    .to_der()
                    .as_bytes()
                    .to_vec(),
                SignatureAlgorithm::EcdsaP384 => p384::ecdsa::Signature::from_slice(&signature)?
                    .to_der()
                    .as_bytes()
                    .to_vec(),
                _ => signature,
            };
        } else {
            ensure!(
                self.algorithm == SignatureAlgorithm::RsaPkcs1v15,
                "This legacy Windows provider cannot sign with this certificate algorithm"
            );
            let mut raw_hash = 0;
            ensure!(
                unsafe { CryptCreateHash(key.handle, CALG_SHA_256, 0, 0, &mut raw_hash) } != 0,
                "Cannot create Windows signing hash: {}",
                std::io::Error::last_os_error()
            );
            let hash = Hash(raw_hash);
            ensure!(
                unsafe { CryptSetHashParam(hash.0, HP_HASHVAL, digest.as_ptr(), 0) } != 0,
                "Cannot set Windows signing digest"
            );
            ensure!(
                unsafe { CryptSignHashW(hash.0, spec, null(), 0, null_mut(), &mut length) } != 0
                    && (1..=16_384).contains(&length),
                "Cannot determine Windows signature size"
            );
            signature = vec![0; length as usize];
            ensure!(
                unsafe {
                    CryptSignHashW(hash.0, spec, null(), 0, signature.as_mut_ptr(), &mut length)
                } != 0,
                "Windows signing failed: {}",
                std::io::Error::last_os_error()
            );
            signature.truncate(length as usize);
            signature.reverse(); // CryptoAPI RSA is little-endian; CMS requires big-endian.
        }
        crate::identity::verify_hash(&self.certificate, digest, &signature)
            .context("Windows key/certificate binding check failed")?;
        Ok(signature)
    }
}
impl CryptoSigner for WindowsSigner {
    fn certificate_der(&self) -> &[u8] {
        &self.certificate
    }
    fn certificate_chain_der(&self) -> Vec<&[u8]> {
        vec![&self.certificate]
    }
    fn signature_algorithm(&self) -> SignatureAlgorithm {
        self.algorithm
    }
    fn digest_algorithm(&self) -> DigestAlgorithm {
        if self.algorithm == SignatureAlgorithm::EcdsaP384 {
            DigestAlgorithm::Sha384
        } else {
            DigestAlgorithm::Sha256
        }
    }
    fn sign_hash(&self, hash: &[u8]) -> std::result::Result<Vec<u8>, CryptoError> {
        self.sign(hash)
            .map_err(|e| CryptoError::SigningFailed(format!("{e:#}")))
    }
}
