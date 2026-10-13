use anyhow::{Context, Result, bail, ensure};
use p12_keystore::{
    Certificate, KeyStore, KeyStoreEntry, Pkcs12ImportPolicy, PrivateKey, PrivateKeyChain,
};
use p256::ecdsa::signature::hazmat::{PrehashSigner, PrehashVerifier};
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, KeyUsagePurpose};
use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, LineEnding};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use underskrift::{CryptoSigner, DigestAlgorithm, SignatureAlgorithm, error::CryptoError};
use zeroize::Zeroizing;

pub struct CreatedIdentity {
    pub pkcs12: Vec<u8>,
    pub certificate_der: Vec<u8>,
    pub certificate_pem: String,
}

pub fn create(name: &str, password: &str, ecdsa: bool) -> Result<CreatedIdentity> {
    create_personal(
        &PersonalId {
            name: name.into(),
            organization: "MyPDF personal digital ID".into(),
            email: String::new(),
            validity_days: 365,
            ecdsa,
        },
        password,
    )
}

#[derive(Clone, Debug)]
pub struct PersonalId {
    pub name: String,
    pub organization: String,
    pub email: String,
    pub validity_days: u32,
    pub ecdsa: bool,
}
pub fn create_personal(details: &PersonalId, password: &str) -> Result<CreatedIdentity> {
    let name = &details.name;
    let ecdsa = details.ecdsa;
    ensure!(
        !name.trim().is_empty() && name.len() <= 200,
        "identity name must contain 1–200 bytes"
    );
    ensure!(
        password.len() >= 12,
        "use a passphrase of at least 12 bytes"
    );
    let key = if ecdsa {
        KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)?
    } else {
        let private = rsa::RsaPrivateKey::new(&mut rand::rngs::OsRng, 2048)?;
        let pem = private.to_pkcs8_pem(LineEnding::LF)?;
        KeyPair::from_pem(&pem)?
    };
    let mut params = CertificateParams::default();
    params.distinguished_name = DistinguishedName::new();
    params.distinguished_name.push(DnType::CommonName, name);
    ensure!(
        details.organization.len() <= 200
            && details.email.len() <= 254
            && (1..=3650).contains(&details.validity_days),
        "Invalid organization, email or validity period"
    );
    if !details.organization.trim().is_empty() {
        params
            .distinguished_name
            .push(DnType::OrganizationName, &details.organization);
    }
    if !details.email.trim().is_empty() {
        ensure!(
            details.email.contains('@') && !details.email.contains(char::is_whitespace),
            "Enter a valid email address"
        );
        params.subject_alt_names.push(rcgen::SanType::Rfc822Name(
            details.email.clone().try_into()?,
        ));
    }
    params.not_before = time::OffsetDateTime::now_utc() - time::Duration::minutes(5);
    params.not_after =
        time::OffsetDateTime::now_utc() + time::Duration::days(details.validity_days as i64);
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::ContentCommitment,
    ];
    params.is_ca = rcgen::IsCa::ExplicitNoCa;
    let certificate = params.self_signed(&key)?;
    let certificate_der = certificate.der().to_vec();
    let key_der = Zeroizing::new(key.serialize_der());
    let chain = PrivateKeyChain::new(
        Sha256::digest(&certificate_der).to_vec(),
        PrivateKey::from_der(&key_der)?,
        vec![Certificate::from_der(&certificate_der)?],
    );
    let mut store = KeyStore::new();
    store.add_entry(name, KeyStoreEntry::PrivateKeyChain(chain));
    let pkcs12 = store
        .writer(password)
        .encryption_iterations(100_000)
        .mac_iterations(100_000)
        .write()?;
    Ok(CreatedIdentity {
        pkcs12,
        certificate_der,
        certificate_pem: certificate.pem(),
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IdentityInfo {
    pub subject: String,
    pub issuer: String,
    pub not_before: String,
    pub not_after: String,
    pub sha256: String,
    pub algorithm: String,
    pub self_issued: bool,
}

pub fn info(der: &[u8]) -> Result<IdentityInfo> {
    let (_, c) = x509_parser::parse_x509_certificate(der)
        .map_err(|e| anyhow::anyhow!("invalid certificate: {e}"))?;
    Ok(IdentityInfo {
        subject: c.subject().to_string(),
        issuer: c.issuer().to_string(),
        not_before: c.validity().not_before.to_string(),
        not_after: c.validity().not_after.to_string(),
        sha256: fingerprint(der),
        algorithm: algorithm(der)
            .map(|a| format!("{a:?}"))
            .unwrap_or_else(|_| {
                format!(
                    "Unsupported signing algorithm ({})",
                    c.public_key().algorithm.algorithm
                )
            }),
        self_issued: c.subject() == c.issuer(),
    })
}

pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn algorithm(der: &[u8]) -> Result<SignatureAlgorithm> {
    let (_, c) = x509_parser::parse_x509_certificate(der)
        .map_err(|e| anyhow::anyhow!("invalid certificate: {e}"))?;
    let spki = c.public_key().raw;
    if rsa::RsaPublicKey::from_public_key_der(spki).is_ok() {
        return Ok(SignatureAlgorithm::RsaPkcs1v15);
    }
    if p256::PublicKey::from_public_key_der(spki).is_ok() {
        return Ok(SignatureAlgorithm::EcdsaP256);
    }
    if p384::PublicKey::from_public_key_der(spki).is_ok() {
        return Ok(SignatureAlgorithm::EcdsaP384);
    }
    bail!("supported signing keys are RSA, P-256, and P-384")
}

pub fn check_certificate(der: &[u8]) -> Result<()> {
    let (_, c) = x509_parser::parse_x509_certificate(der)
        .map_err(|e| anyhow::anyhow!("invalid certificate: {e}"))?;
    ensure!(
        c.validity().is_valid(),
        "certificate is expired or not yet valid"
    );
    if let Some(usage) = c.key_usage()? {
        ensure!(
            usage.value.digital_signature() || usage.value.non_repudiation(),
            "certificate key usage does not permit signing"
        );
    }
    algorithm(der)?;
    Ok(())
}

enum SigningKey {
    Rsa(Box<rsa::RsaPrivateKey>),
    P256(p256::ecdsa::SigningKey),
    P384(p384::ecdsa::SigningKey),
}
pub struct FileSigner {
    key: SigningKey,
    chain: Vec<Vec<u8>>,
    algorithm: SignatureAlgorithm,
}

impl FileSigner {
    pub fn export(&self, alias: &str, password: &str) -> Result<Vec<u8>> {
        ensure!(
            !alias.trim().is_empty() && alias.len() <= 200 && password.len() >= 12,
            "Use an identity name and a passphrase of at least 12 bytes"
        );
        let key = Zeroizing::new(match &self.key {
            SigningKey::Rsa(key) => key.to_pkcs8_der()?.as_bytes().to_vec(),
            SigningKey::P256(key) => key.to_pkcs8_der()?.as_bytes().to_vec(),
            SigningKey::P384(key) => key.to_pkcs8_der()?.as_bytes().to_vec(),
        });
        let chain = PrivateKeyChain::new(
            Sha256::digest(self.certificate_der()).to_vec(),
            PrivateKey::from_der(&key)?,
            self.chain
                .iter()
                .map(|c| Certificate::from_der(c))
                .collect::<Result<Vec<_>, _>>()?,
        );
        let mut store = KeyStore::new();
        store.add_entry(alias, KeyStoreEntry::PrivateKeyChain(chain));
        Ok(store
            .writer(password)
            .encryption_iterations(100_000)
            .mac_iterations(100_000)
            .write()?)
    }
    /// Authenticate the PKCS#12 MAC and link keys to certificates before selecting
    /// an identity. Never silently select among multiple signing identities.
    pub fn load(data: &[u8], password: &str, alias: Option<&str>) -> Result<Self> {
        let store = KeyStore::from_pkcs12(data, password, Pkcs12ImportPolicy::Strict)
            .context("cannot decrypt/authenticate PKCS#12 identity")?;
        let identities: Vec<_> = store
            .entries()
            .filter_map(|(a, e)| {
                if let KeyStoreEntry::PrivateKeyChain(c) = e {
                    Some((a, c))
                } else {
                    None
                }
            })
            .collect();
        let chain = if let Some(alias) = alias {
            identities
                .iter()
                .find(|(a, _)| a.as_str() == alias)
                .map(|(_, c)| *c)
                .context("requested identity alias not found")?
        } else {
            ensure!(
                identities.len() == 1,
                "PKCS#12 has {} signing identities; supply an alias when there are several",
                identities.len()
            );
            identities[0].1
        };
        let certs: Vec<_> = chain.certs().iter().map(|c| c.as_der().to_vec()).collect();
        let cert = certs
            .first()
            .context("certificate-only entries cannot sign")?;
        check_certificate(cert)?;
        let algorithm = algorithm(cert)?;
        let key = match algorithm {
            SignatureAlgorithm::RsaPkcs1v15 => SigningKey::Rsa(Box::new(
                rsa::RsaPrivateKey::from_pkcs8_der(chain.key().as_der())?,
            )),
            SignatureAlgorithm::EcdsaP256 => SigningKey::P256(
                p256::ecdsa::SigningKey::from_pkcs8_der(chain.key().as_der())?,
            ),
            SignatureAlgorithm::EcdsaP384 => SigningKey::P384(
                p384::ecdsa::SigningKey::from_pkcs8_der(chain.key().as_der())?,
            ),
            _ => unreachable!(),
        };
        let signer = Self {
            key,
            chain: certs,
            algorithm,
        };
        let digest = digest_bytes(
            signer.digest_algorithm(),
            b"MyPDF identity key/certificate binding check",
        );
        verify_hash(
            signer.certificate_der(),
            &digest,
            &signer.sign_hash(&digest)?,
        )?;
        Ok(signer)
    }
}

#[derive(Clone, Debug)]
pub struct ImportedIdentity {
    pub alias: String,
    pub certificate_der: Vec<u8>,
    pub info: IdentityInfo,
    pub eligible: bool,
    pub explanation: Option<String>,
}
pub fn inspect_pkcs12(data: &[u8], password: &str) -> Result<Vec<ImportedIdentity>> {
    ensure!(
        data.len() <= 16 * 1024 * 1024,
        "Identity file exceeds 16 MB"
    );
    let store = KeyStore::from_pkcs12(data, password, Pkcs12ImportPolicy::Strict)
        .context("Could not authenticate/decrypt the identity. Check its passphrase.")?;
    let mut result = vec![];
    for (alias, entry) in store.entries() {
        if let KeyStoreEntry::PrivateKeyChain(chain) = entry
            && let Some(cert) = chain.certs().first()
        {
            let der = cert.as_der().to_vec();
            let info = info(&der)?;
            let check = FileSigner::load(data, password, Some(alias));
            let explanation = check.as_ref().err().map(|e| format!("{e:#}"));
            result.push(ImportedIdentity {
                alias: alias.clone(),
                certificate_der: der,
                info,
                eligible: check.is_ok(),
                explanation,
            });
        }
    }
    ensure!(
        !result.is_empty(),
        "This file contains no certificate/private-key signing identities"
    );
    Ok(result)
}
pub fn read_certificate(data: &[u8]) -> Result<Vec<u8>> {
    ensure!(data.len() <= 1024 * 1024, "Certificate exceeds 1 MB");
    let der = if data.starts_with(b"-----BEGIN CERTIFICATE-----") {
        use base64::Engine;
        let text = std::str::from_utf8(data)?;
        let encoded = text
            .lines()
            .filter(|l| !l.starts_with("-----"))
            .collect::<String>();
        base64::engine::general_purpose::STANDARD.decode(encoded)?
    } else {
        data.to_vec()
    };
    let (remaining, _) = x509_parser::parse_x509_certificate(&der)
        .map_err(|e| anyhow::anyhow!("Invalid X.509 certificate: {e}"))?;
    ensure!(remaining.is_empty(), "Certificate has trailing data");
    Ok(der)
}

impl CryptoSigner for FileSigner {
    fn certificate_der(&self) -> &[u8] {
        &self.chain[0]
    }
    fn certificate_chain_der(&self) -> Vec<&[u8]> {
        self.chain.iter().map(Vec::as_slice).collect()
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
        let result: Result<Vec<u8>> = (|| {
            ensure!(
                hash.len()
                    == if self.algorithm == SignatureAlgorithm::EcdsaP384 {
                        48
                    } else {
                        32
                    },
                "incorrect digest length"
            );
            Ok(match &self.key {
                SigningKey::Rsa(key) => key.sign(rsa::Pkcs1v15Sign::new::<Sha256>(), hash)?,
                SigningKey::P256(key) => {
                    let signature: p256::ecdsa::Signature = key.sign_prehash(hash)?;
                    signature.to_der().as_bytes().to_vec()
                }
                SigningKey::P384(key) => {
                    let signature: p384::ecdsa::Signature = key.sign_prehash(hash)?;
                    signature.to_der().as_bytes().to_vec()
                }
            })
        })();
        result.map_err(|e| CryptoError::SigningFailed(e.to_string()))
    }
}

pub fn verify_hash(cert: &[u8], hash: &[u8], signature: &[u8]) -> Result<()> {
    let (_, c) = x509_parser::parse_x509_certificate(cert)
        .map_err(|e| anyhow::anyhow!("invalid certificate: {e}"))?;
    let spki = c.public_key().raw;
    match algorithm(cert)? {
        SignatureAlgorithm::RsaPkcs1v15 => rsa::RsaPublicKey::from_public_key_der(spki)?.verify(
            rsa::Pkcs1v15Sign::new::<Sha256>(),
            hash,
            signature,
        )?,
        SignatureAlgorithm::EcdsaP256 => p256::ecdsa::VerifyingKey::from_public_key_der(spki)?
            .verify_prehash(hash, &p256::ecdsa::Signature::from_der(signature)?)?,
        SignatureAlgorithm::EcdsaP384 => p384::ecdsa::VerifyingKey::from_public_key_der(spki)?
            .verify_prehash(hash, &p384::ecdsa::Signature::from_der(signature)?)?,
        _ => unreachable!(),
    }
    Ok(())
}

pub fn digest_bytes(algorithm: DigestAlgorithm, bytes: &[u8]) -> Vec<u8> {
    algorithm.digest(bytes)
}
