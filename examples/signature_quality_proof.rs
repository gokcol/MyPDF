//! Compare the former bitmap signature with the vector appearance at high zoom.
use mypdf_milestone0::{convert, identity, signature_art, signing};
use underskrift::{CryptoSigner, PdfSigner, SigningOptions};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let out = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "artifacts/signature-quality-proof".into()),
    );
    std::fs::create_dir_all(&out)?;
    let password = "synthetic-quality-proof-only";
    let created = identity::create_personal(
        &identity::PersonalId {
            name: "MyPDF Quality Test".into(),
            organization: "Synthetic visual test".into(),
            email: "quality@example.invalid".into(),
            validity_days: 1,
            ecdsa: true,
        },
        password,
    )?;
    let signer = identity::FileSigner::load(&created.pkcs12, password, None)?;
    let source = convert::blank(420., 180.)?;
    let request = signing::DesktopSigningRequest {
        field: "QualityTest".into(),
        rect: Some([24., 24., 396., 156.]),
        name: "Deniz Yılmaz".into(),
        reason: "Signature quality preview".into(),
        show_date: false,
        background: signing::Background::Tinted,
        artwork: Some(signature_art::from_name(
            "Deniz Yılmaz",
            4,
            true,
            [0., 0., 0.65],
        )?),
        text_style: signing::SignatureTextStyle {
            bold: true,
            size: 15.,
            alignment: 1,
            offset: [0., 18.],
            ..Default::default()
        },
        transform: signing::AppearanceTransform {
            rotation: -20.,
            ..Default::default()
        },
        ..Default::default()
    };
    use underskrift::visual::{
        ImageConfig, ImageFormat, ImageScale, SignatureLayout, SignatureRect,
        VisibleSignatureConfig,
    };
    let old = PdfSigner::new()
        .options(SigningOptions {
            field_name: request.field.clone(),
            digest_algorithm: signer.digest_algorithm(),
            visible_signature: Some(VisibleSignatureConfig {
                page: 0,
                rect: SignatureRect::Absolute {
                    llx: 24.,
                    lly: 24.,
                    urx: 396.,
                    ury: 156.,
                },
                layout: SignatureLayout::ImageOnly(ImageConfig {
                    data: signing::signature_artwork(&request)?,
                    format: ImageFormat::Png,
                    scale: ImageScale::Stretch,
                }),
                background_color: None,
                border: None,
            }),
            ..Default::default()
        })
        .sign(&source, &signer)
        .await?;
    let new = signing::sign_custom(&source, &signer, &request).await?;
    for (name, bytes) in [("before-bitmap.pdf", old), ("after-vector.pdf", new)] {
        let report = signing::verify(&bytes, None)?;
        anyhow::ensure!(
            report.len() == 1
                && report[0].crypto_valid
                && report[0].digest_matches
                && report[0].covers_current_file,
            "Invalid proof signature"
        );
        std::fs::write(out.join(name), bytes)?;
    }
    println!("Verified bitmap/vector comparison in {}", out.display());
    Ok(())
}
