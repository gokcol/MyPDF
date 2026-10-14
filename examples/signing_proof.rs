use mypdf_milestone0::{
    annotations::{self as ann, Annotation, Kind, Style, Symbol},
    assets::Asset,
    document, identity, signing, write_new,
};
#[cfg(target_os = "macos")]
use underskrift::CryptoSigner;
const PASSWORD: &str = "synthetic-milestone23-password-only";
#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let out = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "artifacts/milestone2-proof".into()),
    );
    std::fs::create_dir_all(&out)?;
    let created = identity::create_personal(
        &identity::PersonalId {
            name: "İstanbul Test Signer".into(),
            organization: "MyPDF synthetic proof".into(),
            email: "signer@example.invalid".into(),
            validity_days: 30,
            ecdsa: true,
        },
        PASSWORD,
    )?;
    write_new(&out.join("synthetic.pfx"), &created.pkcs12)?;
    write_new(&out.join("synthetic.cer"), &created.certificate_der)?;
    let signer = identity::FileSigner::load(&created.pkcs12, PASSWORD, None)?;
    let source = document::fixture(&document::default_font()?)?;
    write_new(&out.join("original.pdf"), &source)?;
    let mut reports = serde_json::Map::new();
    for rotation in [0, 90, 180, 270] {
        let mut doc = lopdf::Document::load_mem(&source)?;
        let page = doc.get_pages()[&1];
        doc.get_dictionary_mut(page)?.set("Rotate", rotation);
        doc.get_dictionary_mut(page)?.set(
            "CropBox",
            vec![36.into(), 36.into(), 576.into(), 756.into()],
        );
        let mut bytes = vec![];
        doc.save_to(&mut bytes)?;
        let a = Annotation::new(
            0,
            if rotation == 90 {
                [300.0, 190.0, 470.0, 260.0]
            } else if rotation == 270 {
                [300.0, 200.0, 470.0, 270.0]
            } else {
                [60.0, 260.0, 490.0, 320.0]
            },
            Kind::Text {
                text: "Reviewed with native Rust. İstanbul, Türkçe.".into(),
            },
            Style::default(),
        );
        let bytes = ann::export(&bytes, &[a])?;
        let req = signing::DesktopSigningRequest {
            field: format!("Rotation{rotation}"),
            page: 0,
            rect: Some([170.0, 370.0, 495.0, 490.0]),
            name: "İstanbul Test Signer".into(),
            reason: "Sözleşme onayı".into(),
            location: "İstanbul".into(),
            artwork: Some(Asset::new(
                "Test checkmark",
                Kind::Symbol(Symbol::Check),
                Style::default(),
                1.5,
            )),
            layout: Default::default(),
            ..Default::default()
        };
        let png = signing::signature_artwork(&req)?;
        write_new(&out.join(format!("appearance-{rotation}.png")), &png)?;
        let signed = signing::sign_custom(&bytes, &signer, &req).await?;
        write_new(&out.join(format!("signed-{rotation}.pdf")), &signed)?;
        reports.insert(
            format!("rotation-{rotation}"),
            serde_json::to_value(signing::verify(&signed, None)?)?,
        );
        for (i, part) in signing::detached_parts(&signed)?.iter().enumerate() {
            write_new(&out.join(format!("signed-{rotation}-{i}.cms")), &part.cms)?;
            write_new(
                &out.join(format!("signed-{rotation}-{i}.bin")),
                &part.signed_bytes,
            )?;
        }
        if rotation == 0 {
            let twice = signing::sign_custom(
                &signed,
                &signer,
                &signing::DesktopSigningRequest {
                    field: "SecondApproval".into(),
                    rect: None,
                    ..Default::default()
                },
            )
            .await?;
            write_new(&out.join("signed-twice.pdf"), &twice)?;
            reports.insert(
                "two-signatures".into(),
                serde_json::to_value(signing::verify(&twice, None)?)?,
            );
            let mut tampered = twice.clone();
            tampered[7] = b'6';
            write_new(&out.join("tampered.pdf"), &tampered)?;
            reports.insert(
                "tampered".into(),
                serde_json::to_value(signing::verify(&tampered, None)?)?,
            );
        }
    }
    #[cfg(target_os = "macos")]
    if std::env::args().any(|a| a == "--keychain") {
        let mut store = mypdf_milestone0::keychain::TemporaryKeychain::create(
            &out.join("synthetic.keychain-db"),
            PASSWORD,
        )?;
        let native = store.import_and_select(&created.pkcs12, PASSWORD)?;
        let found = mypdf_milestone0::keychain::list(Some(&out.join("synthetic.keychain-db")))?;
        anyhow::ensure!(
            found.len() == 1 && found[0].certificate_der == created.certificate_der,
            "Keychain discovery mismatch"
        );
        let selected = mypdf_milestone0::keychain::select(
            Some(&out.join("synthetic.keychain-db")),
            &identity::fingerprint(native.certificate_der()),
        )?;
        let signed = signing::sign_custom(
            &source,
            &selected,
            &signing::DesktopSigningRequest {
                field: "NativeKeychain".into(),
                rect: Some([170.0, 550.0, 495.0, 670.0]),
                name: "Native Keychain Test".into(),
                ..Default::default()
            },
        )
        .await?;
        write_new(&out.join("signed-keychain.pdf"), &signed)?;
        reports.insert(
            "native-keychain".into(),
            serde_json::to_value(signing::verify(&signed, None)?)?,
        );
        drop(selected);
        drop(native);
        store.close()?;
    }
    write_new(
        &out.join("report.json"),
        &serde_json::to_vec_pretty(&reports)?,
    )?;
    println!("Verified signed PDF evidence in {}", out.display());
    Ok(())
}
