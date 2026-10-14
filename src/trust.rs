//! Which signers to trust when validating signatures: a list of certificates
//! the user chose to trust, kept between sessions, and the operating system's
//! own trust (macOS system roots and Keychain trust settings, or the Windows
//! certificate stores).
use anyhow::{Context as _, Result, ensure};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A certificate the user trusts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trusted {
    pub sha256: String,
    pub subject: String,
    pub issuer: String,
    pub not_after: String,
    #[serde(with = "der_base64")]
    pub der: Vec<u8>,
}

mod der_base64 {
    use base64::Engine as _;
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(der: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(der))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        base64::engine::general_purpose::STANDARD
            .decode(text)
            .map_err(serde::de::Error::custom)
    }
}

/// The user's trusted certificates, stored as JSON.
pub struct TrustList {
    path: PathBuf,
    entries: Vec<Trusted>,
}

pub fn default_path() -> PathBuf {
    crate::assets::data_directory().join("trusted_certificates.json")
}

impl TrustList {
    /// An empty list that will be saved at `path` once something is trusted.
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_owned(),
            entries: vec![],
        }
    }

    /// Open the list at `path`; a missing file is an empty list.
    pub fn open(path: &Path) -> Result<Self> {
        let entries = match std::fs::read(path) {
            Ok(raw) => serde_json::from_slice::<Vec<Trusted>>(&raw)
                .with_context(|| format!("{} is not a trusted-certificates list", path.display()))?
                .into_iter()
                // Keep only entries whose certificate still parses and matches its fingerprint.
                .filter(|t| crate::identity::fingerprint(&t.der) == t.sha256)
                .collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            path: path.to_owned(),
            entries,
        })
    }

    pub fn entries(&self) -> &[Trusted] {
        &self.entries
    }

    pub fn ders(&self) -> Vec<Vec<u8>> {
        self.entries.iter().map(|t| t.der.clone()).collect()
    }

    pub fn contains(&self, der: &[u8]) -> bool {
        let sha256 = crate::identity::fingerprint(der);
        self.entries.iter().any(|t| t.sha256 == sha256)
    }

    /// Trust a certificate (DER). Returns false when it was already trusted.
    pub fn add(&mut self, der: &[u8]) -> Result<bool> {
        let info = crate::identity::info(der).context("Not a valid certificate")?;
        if self.contains(der) {
            return Ok(false);
        }
        self.entries.push(Trusted {
            sha256: crate::identity::fingerprint(der),
            subject: info.subject,
            issuer: info.issuer,
            not_after: info.not_after,
            der: der.to_vec(),
        });
        self.save()?;
        Ok(true)
    }

    /// Stop trusting a certificate. Returns false when it was not in the list.
    pub fn remove(&mut self, sha256: &str) -> Result<bool> {
        let before = self.entries.len();
        self.entries.retain(|t| t.sha256 != sha256);
        if self.entries.len() == before {
            return Ok(false);
        }
        self.save()?;
        Ok(true)
    }

    /// Write atomically: a temporary file in the same folder, then a rename.
    fn save(&self) -> Result<()> {
        let dir = self.path.parent().context("Invalid trust list path")?;
        std::fs::create_dir_all(dir)?;
        let raw = serde_json::to_vec_pretty(&self.entries)?;
        let mut file = tempfile::NamedTempFile::new_in(dir)?;
        std::io::Write::write_all(&mut file, &raw)?;
        file.as_file().sync_all()?;
        file.persist(&self.path)
            .map_err(|e| anyhow::anyhow!("Could not save trusted certificates: {e}"))?;
        Ok(())
    }
}

/// Read a certificate from DER or PEM bytes.
pub fn read_certificate(bytes: &[u8]) -> Result<Vec<u8>> {
    let der = if bytes.starts_with(b"-----BEGIN") {
        let text = std::str::from_utf8(bytes).context("PEM must be text")?;
        let body: String = text
            .lines()
            .skip_while(|l| !l.starts_with("-----BEGIN CERTIFICATE"))
            .skip(1)
            .take_while(|l| !l.starts_with("-----END"))
            .collect();
        base64::engine::general_purpose::STANDARD
            .decode(body.trim())
            .context("The PEM certificate is damaged")?
    } else {
        bytes.to_vec()
    };
    ensure!(
        crate::identity::info(&der).is_ok(),
        "Choose an X.509 certificate (.cer, .crt or .pem)"
    );
    Ok(der)
}

/// Which operating system trust store this platform consults.
pub fn system_name() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some("macOS")
    } else if cfg!(target_os = "windows") {
        Some("Windows")
    } else {
        None
    }
}

/// Whether the operating system trusts this chain (signer first, then any
/// intermediates), offline, for general certificate use.
pub fn system_trusts(chain: &[Vec<u8>]) -> bool {
    if chain.is_empty() {
        return false;
    }
    #[cfg(target_os = "macos")]
    return crate::keychain::evaluate(chain);
    #[cfg(target_os = "windows")]
    return crate::windows_trust::evaluate(chain);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusted_certificates_persist_and_reject_tampering() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("trusted.json");
        let created =
            crate::identity::create("Trust List Test", "synthetic-trust-list", true).unwrap();
        let der = crate::identity::inspect_pkcs12(&created.pkcs12, "synthetic-trust-list")
            .unwrap()
            .remove(0)
            .certificate_der;
        let mut list = TrustList::open(&path).unwrap();
        assert!(list.entries().is_empty());
        assert!(list.add(&der).unwrap());
        assert!(!list.add(&der).unwrap(), "already trusted");
        let reopened = TrustList::open(&path).unwrap();
        assert!(reopened.contains(&der));
        assert!(reopened.entries()[0].subject.contains("Trust List Test"));
        // An edited certificate no longer matches its fingerprint and is dropped.
        let raw = std::fs::read_to_string(&path).unwrap();
        let tampered = raw.replace(&reopened.entries()[0].sha256, &"0".repeat(64));
        std::fs::write(&path, tampered).unwrap();
        assert!(TrustList::open(&path).unwrap().entries().is_empty());
        std::fs::write(&path, raw).unwrap();
        let mut list = TrustList::open(&path).unwrap();
        let sha = list.entries()[0].sha256.clone();
        assert!(list.remove(&sha).unwrap());
        assert!(!TrustList::open(&path).unwrap().contains(&der));
        assert!(list.add(b"not a certificate").is_err());
    }

    #[test]
    fn certificates_read_from_der_or_pem() {
        let created = crate::identity::create("Pem Test", "synthetic-pem-test", true).unwrap();
        let der = crate::identity::inspect_pkcs12(&created.pkcs12, "synthetic-pem-test")
            .unwrap()
            .remove(0)
            .certificate_der;
        let pem = format!(
            "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
            base64::engine::general_purpose::STANDARD.encode(&der)
        );
        assert_eq!(read_certificate(&der).unwrap(), der);
        assert_eq!(read_certificate(pem.as_bytes()).unwrap(), der);
        assert!(read_certificate(b"hello").is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_system_roots_are_trusted() {
        let anchors = security_framework::trust::SecTrust::copy_anchor_certificates().unwrap();
        assert!(
            anchors.iter().any(|root| system_trusts(&[root.to_der()])),
            "at least one system root evaluates as trusted"
        );
    }

    #[test]
    fn a_fresh_self_signed_certificate_is_not_system_trusted() {
        let created =
            crate::identity::create("Untrusted Test", "synthetic-untrusted", true).unwrap();
        let der = crate::identity::inspect_pkcs12(&created.pkcs12, "synthetic-untrusted")
            .unwrap()
            .remove(0)
            .certificate_der;
        assert!(!system_trusts(&[der]));
        assert!(!system_trusts(&[]));
    }
}
