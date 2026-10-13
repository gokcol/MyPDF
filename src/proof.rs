use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};
use underskrift::CryptoSigner;

const TEST_PASSWORD: &str = "MyPDF-milestone0-synthetic-only";

#[derive(Debug, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub passed: bool,
    pub details: String,
}
#[derive(Serialize)]
pub struct ProofReport {
    pub version: String,
    pub checks: Vec<Check>,
    pub rendering: Vec<crate::document::RenderEvidence>,
    pub signatures: Vec<(String, Vec<crate::signing::SignatureEvidence>)>,
    pub limitations: Vec<String>,
}

fn command(name: &str, args: &[&str]) -> Result<Output> {
    Command::new(name)
        .args(args)
        .output()
        .with_context(|| format!("independent verifier {name} unavailable"))
}
fn output_text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn compare_with_poppler(pdf: &Path, hayro_dir: &Path, out: &Path) -> Result<Vec<Check>> {
    std::fs::create_dir_all(out)?;
    let prefix = out.join("reference");
    let output = command(
        "pdftoppm",
        &[
            "-r",
            "108",
            "-png",
            "-cropbox",
            pdf.to_str().context("non-UTF8 path")?,
            prefix.to_str().context("non-UTF8 path")?,
        ],
    )?;
    ensure!(
        output.status.success(),
        "Poppler reference rendering failed: {}",
        output_text(&output)
    );
    let mut checks = Vec::new();
    for page in 1..=2 {
        let hayro = image::open(hayro_dir.join(format!("page-{page}.png")))?.to_rgba8();
        let poppler = image::open(out.join(format!("reference-{page}.png")))?.to_rgba8();
        ensure!(
            hayro.dimensions() == poppler.dimensions(),
            "renderer page dimensions disagree"
        );
        let error: f64 = hayro
            .pixels()
            .zip(poppler.pixels())
            .map(|(a, b)| (0..3).map(|i| f64::from(a[i].abs_diff(b[i]))).sum::<f64>())
            .sum::<f64>()
            / (f64::from(hayro.width()) * f64::from(hayro.height()) * 3.0);
        checks.push(Check {name:format!("Independent rendering: {}/page {page}",pdf.file_name().unwrap().to_string_lossy()),passed:error<4.0,details:format!("Mean RGB absolute difference {error:.4}/255; dimensions {}×{}. This threshold allows font fallback and antialiasing differences.",hayro.width(),hayro.height())});
    }
    Ok(checks)
}

pub fn verify_independently(path: &Path, bytes: &[u8], out: &Path) -> Result<Vec<Check>> {
    let path_str = path.to_str().context("non-UTF8 proof path")?;
    let result = command("pdfsig", &["-nocert", path_str])?;
    let text = output_text(&result);
    let count = crate::signing::detached_parts(bytes)?.len();
    ensure!(count > 0, "no signatures to verify");
    let poppler_valid = result.status.success()
        && text
            .matches("Signature Validation: Signature is Valid.")
            .count()
            == count;
    let mut checks = vec![Check {
        name: format!("Poppler: {}", path.file_name().unwrap().to_string_lossy()),
        passed: poppler_valid,
        details: text,
    }];
    std::fs::create_dir_all(out)?;
    for signature in crate::signing::detached_parts(bytes)? {
        let crate::signing::DetachedSignature {
            label,
            cms,
            signed_bytes: signed,
        } = signature;
        let cms_path = out.join(format!("{label}.cms.der"));
        let content_path = out.join(format!("{label}.signed-bytes"));
        crate::write_new(&cms_path, &cms)?;
        crate::write_new(&content_path, &signed)?;
        let verified = command(
            "openssl",
            &[
                "cms",
                "-verify",
                "-binary",
                "-inform",
                "DER",
                "-in",
                cms_path.to_str().unwrap(),
                "-content",
                content_path.to_str().unwrap(),
                "-noverify",
                "-out",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            ],
        )?;
        checks.push(Check {
            name: format!(
                "OpenSSL CMS: {}/{label}",
                path.file_name().unwrap().to_string_lossy()
            ),
            passed: verified.status.success(),
            details: output_text(&verified),
        });
    }
    Ok(checks)
}

pub async fn run(out: &Path, font: &Path, with_keychain: bool) -> Result<ProofReport> {
    ensure!(
        !out.exists(),
        "choose a new proof directory; existing outputs are preserved"
    );
    std::fs::create_dir_all(out)?;
    let original = crate::document::fixture(font)?;
    let original_path = out.join("original.pdf");
    crate::write_new(&original_path, &original)?;
    let edited = crate::document::replace_run(
        &original,
        1,
        crate::document::ORIGINAL,
        crate::document::REPLACEMENT,
    )?;
    let edited = crate::document::replace_run(
        &edited,
        1,
        crate::document::TURKISH_ORIGINAL,
        crate::document::TURKISH_REPLACEMENT,
    )?;
    let edited =
        crate::document::replace_run(&edited, 1, "Split text example", "Split text edited")?;
    let edited_path = out.join("edited.pdf");
    crate::write_new(&edited_path, &edited)?;
    let mut report = ProofReport {version:env!("CARGO_PKG_VERSION").into(), checks:vec![], rendering:vec![], signatures:vec![], limitations:vec![
        "Text edits replace one complete page-level Tj/TJ/quote run; no paragraph reflow, Form XObject recursion, new glyph insertion or arbitrary font substitution.".into(),
        "Hayro 0.8 remains experimental; the synthetic corpus proves these features, not universal PDF compatibility.".into(),
        "Signing is offline PAdES B-B with RSA/SHA-256 or ECDSA P-256/SHA-256 and P-384/SHA-384. No timestamp, revocation lookup, or certification policy support in milestone 0.".into(),
        "Self-signed IDs remain untrusted until the recipient explicitly establishes trust. Test trust is confined to an in-memory verifier.".into(),
        "Keychain proof uses an isolated temporary keychain and a generated test identity. It does not certify hardware-token, Secure Enclave, or all existing-identity compatibility.".into(),
        "Signing accepts traditional cross-reference tables. Unsigned cross-reference-stream PDFs must first be explicitly prepared into a new copy. Signed stream inputs are rejected because normalization would invalidate their signed bytes.".into(),
        "The native Keychain adapter currently embeds the leaf certificate only; complete intermediate-chain discovery is a later gate. RSA and P-256 are independently proven; P-384 is implemented but not yet included in the interoperability corpus.".into(),
        "Application source is Rust. macOS Security APIs and ring's compiled crypto backend mean this is not a claim that every transitive dependency is implemented only in Rust.".into(),
    ]};
    let text = command(
        "pdftotext",
        &["-f", "1", "-l", "1", edited_path.to_str().unwrap(), "-"],
    )?;
    let text = String::from_utf8(text.stdout)?;
    report.checks.push(Check {
        name: "Independent text extraction".into(),
        passed: text.contains(crate::document::REPLACEMENT)
            && text.contains(crate::document::TURKISH_REPLACEMENT)
            && !text.contains(crate::document::ORIGINAL)
            && !text.contains(crate::document::TURKISH_ORIGINAL)
            && text.contains("Split text edited"),
        details: text,
    });
    let page2_before = crate::document::text_runs(&original, 2)?;
    let page2_after = crate::document::text_runs(&edited, 2)?;
    report.checks.push(Check {
        name: "Shared stream isolation".into(),
        passed: page2_before
            .iter()
            .map(|r| &r.text)
            .eq(page2_after.iter().map(|r| &r.text)),
        details: "Changing page 1 leaves page 2's shared original content intact.".into(),
    });
    report.checks.push(Check {
        name: "Unencodable glyph rejected".into(),
        passed: crate::document::replace_run(&original, 1, crate::document::ORIGINAL, "Invoice 🦀")
            .is_err(),
        details: "Unavailable glyphs fail before writing; no silent substitution.".into(),
    });
    report.rendering.extend(crate::document::render(
        &original,
        &out.join("render-original"),
        1.5,
    )?);
    report.rendering.extend(crate::document::render(
        &edited,
        &out.join("render-edited"),
        1.5,
    )?);
    report.checks.extend(compare_with_poppler(
        &original_path,
        &out.join("render-original"),
        &out.join("reference-original"),
    )?);
    report.checks.extend(compare_with_poppler(
        &edited_path,
        &out.join("render-edited"),
        &out.join("reference-edited"),
    )?);
    let before = image::open(out.join("render-original/page-1.png"))?.to_rgba8();
    let after = image::open(out.join("render-edited/page-1.png"))?.to_rgba8();
    let mut changed = 0usize;
    let mut unexpected = 0usize;
    for (x, y, p) in before.enumerate_pixels() {
        if p != after.get_pixel(x, y) {
            changed += 1;
            // All edited baselines are between PDF y=520 and y=620. Keep
            // the rest of the page byte-for-byte identical at the pixel level.
            if !(240..=425).contains(&y) {
                unexpected += 1;
            }
        }
    }
    let second_before = std::fs::read(out.join("render-original/page-2.png"))?;
    let second_after = std::fs::read(out.join("render-edited/page-2.png"))?;
    report.checks.push(Check {name:"Rendered edit isolation".into(),passed:changed>100 && unexpected==0 && second_before==second_after,details:format!("{changed} changed pixels in text region; {unexpected} outside it; rotated/cropped page unchanged.")});
    let rsa = crate::identity::create("MyPDF Milestone 0 RSA test identity", TEST_PASSWORD, false)?;
    crate::write_new(&out.join("test-rsa.p12"), &rsa.pkcs12)?;
    crate::write_new(&out.join("test-rsa.cer"), &rsa.certificate_der)?;
    crate::write_new(&out.join("test-rsa.pem"), rsa.certificate_pem.as_bytes())?;
    let p256 =
        crate::identity::create("MyPDF Milestone 0 P256 test identity", TEST_PASSWORD, true)?;
    crate::write_new(&out.join("test-ecdsa.pfx"), &p256.pkcs12)?;
    crate::write_new(&out.join("test-ecdsa.cer"), &p256.certificate_der)?;
    let rsa_signer = crate::identity::FileSigner::load(&rsa.pkcs12, TEST_PASSWORD, None)?;
    let ec_signer = crate::identity::FileSigner::load(&p256.pkcs12, TEST_PASSWORD, None)?;
    report.checks.push(Check {
        name: "Wrong PKCS#12 password rejected".into(),
        passed: crate::identity::FileSigner::load(&rsa.pkcs12, "wrong-password", None).is_err(),
        details: "MAC/decryption failure prevents signing.".into(),
    });
    let mut cert_only = p12_keystore::KeyStore::new();
    cert_only.add_entry(
        "certificate without private key",
        p12_keystore::KeyStoreEntry::Certificate(p12_keystore::Certificate::from_der(
            &rsa.certificate_der,
        )?),
    );
    report.checks.push(Check {
        name: "Certificate-only file rejected".into(),
        passed: crate::identity::FileSigner::load(
            &cert_only.writer(TEST_PASSWORD).write()?,
            TEST_PASSWORD,
            None,
        )
        .is_err(),
        details: "A public certificate is not a signing identity.".into(),
    });
    let cert = command(
        "openssl",
        &[
            "x509",
            "-in",
            out.join("test-rsa.pem").to_str().unwrap(),
            "-noout",
            "-text",
        ],
    )?;
    let details = output_text(&cert);
    report.checks.push(Check {
        name: "Independent certificate inspection".into(),
        passed: cert.status.success()
            && details.contains("CA:FALSE")
            && details.contains("Digital Signature")
            && details.contains("2048 bit"),
        details,
    });
    let mut signed_files: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    let rsa_signed = crate::signing::sign(&edited, &rsa_signer, "P12Signature", true).await?;
    signed_files.push((out.join("signed-p12.pdf"), rsa_signed.clone()));
    let ec_signed = crate::signing::sign(&edited, &ec_signer, "PfxSignature", true).await?;
    signed_files.push((out.join("signed-pfx.pdf"), ec_signed));
    let twice = crate::signing::sign(&rsa_signed, &ec_signer, "SecondSignature", false).await?;
    signed_files.push((out.join("signed-twice.pdf"), twice));
    report.checks.push(Check {
        name: "Signed-document edit guard".into(),
        passed: crate::document::replace_run(
            &rsa_signed,
            1,
            crate::document::REPLACEMENT,
            "tampered",
        )
        .is_err(),
        details: "Ordinary content edits cannot silently invalidate signed files.".into(),
    });
    let mut tampered = rsa_signed.clone();
    let marker = b"1450.00";
    if let Some(pos) = tampered.windows(marker.len()).position(|b| b == marker) {
        tampered[pos] = b'9';
    } else {
        // Text is hex encoded after editing; change one hex nibble in the signed
        // content stream while retaining structurally parseable PDF syntax.
        let marker = b"313435302E3030";
        let pos = tampered
            .windows(marker.len())
            .position(|b| b == marker)
            .context("cannot locate text for tamper test")?;
        tampered[pos + 1] = b'9';
    }
    let tampered_report = crate::signing::verify(&tampered, None)?;
    crate::write_new(&out.join("tampered.pdf"), &tampered)?;
    let tampered_poppler = command(
        "pdfsig",
        &["-nocert", out.join("tampered.pdf").to_str().unwrap()],
    )?;
    let tampered_details = output_text(&tampered_poppler);
    report.checks.push(Check {
        name: "Independent tamper detection".into(),
        passed: tampered_details.contains("Digest Mismatch"),
        details: tampered_details,
    });
    report.checks.push(Check {
        name: "Tampering detected".into(),
        passed: tampered_report.iter().any(|s| !s.digest_matches),
        details: format!("{tampered_report:?}"),
    });
    let untrusted = crate::signing::verify(&rsa_signed, None)?;
    let trusted = crate::signing::verify(&rsa_signed, Some(rsa_signer.certificate_der()))?;
    report.checks.push(Check {name:"Integrity distinguished from trust".into(),passed:untrusted.iter().all(|s|s.crypto_valid && !s.trusted) && trusted.iter().all(|s|s.crypto_valid && s.trusted),details:"The same valid signature is untrusted by default and trusted only by explicit in-memory test configuration.".into()});
    #[cfg(target_os = "macos")]
    if with_keychain {
        let temp = tempfile::tempdir_in(out)?;
        let mut keychain = crate::keychain::TemporaryKeychain::create(
            &temp.path().join("proof.keychain"),
            TEST_PASSWORD,
        )?;
        let native = keychain.import_and_select(&rsa.pkcs12, TEST_PASSWORD)?;
        ensure!(
            native.certificate_der() == rsa_signer.certificate_der(),
            "Keychain rediscovery picked wrong identity"
        );
        let signed = crate::signing::sign(&edited, &native, "KeychainSignature", true).await?;
        signed_files.push((out.join("signed-keychain.pdf"), signed));
        let signed =
            crate::signing::sign(&rsa_signed, &native, "SecondKeychainSignature", false).await?;
        signed_files.push((out.join("signed-p12-then-keychain.pdf"), signed));
        drop(native);
        keychain.close()?;
        report.checks.push(Check {name:"Native macOS Keychain signing".into(),passed:true,details:"Generated identity imported into isolated Keychain, rediscovered, signed with SecKeyCreateSignature; no private-key export. Test Keychain deleted and original search list restored.".into()});
    }
    #[cfg(not(target_os = "macos"))]
    ensure!(!with_keychain, "Keychain proof requires macOS");
    for (path, bytes) in signed_files {
        crate::write_new(&path, &bytes)?;
        report.signatures.push((
            path.file_name().unwrap().to_string_lossy().into(),
            crate::signing::verify(&bytes, None)?,
        ));
        let name = path.file_stem().unwrap().to_string_lossy();
        report.checks.extend(verify_independently(
            &path,
            &bytes,
            &out.join("independent").join(name.as_ref()),
        )?);
    }
    report.rendering.extend(crate::document::render(
        &rsa_signed,
        &out.join("render-signed"),
        1.5,
    )?);
    report.checks.extend(compare_with_poppler(
        &out.join("signed-p12.pdf"),
        &out.join("render-signed"),
        &out.join("reference-signed"),
    )?);
    crate::write_new(
        &out.join("report.json"),
        &serde_json::to_vec_pretty(&report)?,
    )?;
    ensure!(
        report.checks.iter().all(|c| c.passed),
        "proof checks failed; inspect {}",
        out.join("report.json").display()
    );
    Ok(report)
}
