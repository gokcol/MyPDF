use anyhow::{Context, Result, ensure};
use serde::Serialize;
use underskrift::{
    CryptoSigner, PdfSigner, SigningOptions,
    trust::{TrustStore, TrustStoreSet},
    verify::{CryptoValidity, SignatureVerifier},
    visual::{
        Border, SignatureLayout, SignatureRect, TextConfig, TextLine, VisibleSignatureConfig,
    },
};

#[derive(Clone, Debug, Serialize)]
pub struct SignatureEvidence {
    pub field: String,
    pub signer: Option<crate::identity::IdentityInfo>,
    pub status: String,
    pub crypto_valid: bool,
    pub digest_matches: bool,
    pub byte_range_valid: bool,
    pub covers_current_file: bool,
    pub trusted: bool,
    pub certificate_validity: String,
    pub claimed_signing_time: Option<String>,
    pub revocation: String,
    pub timestamp: Option<String>,
    pub certificate_binding_valid: bool,
}

pub fn verify(bytes: &[u8], trusted_cert: Option<&[u8]>) -> Result<Vec<SignatureEvidence>> {
    if underskrift::verify::extractor::extract_signatures(bytes)?.is_empty() {
        return Ok(Vec::new());
    }
    let mut stores = TrustStoreSet::new();
    if let Some(cert) = trusted_cert {
        let mut trust = TrustStore::new();
        trust.add_der_certificate(cert)?;
        stores = stores.with_sig_store(trust);
    }
    let report = SignatureVerifier::new(&stores)
        .allow_online(false)
        .verify_pdf(bytes)?;
    Ok(report
        .signatures
        .iter()
        .map(|s| SignatureEvidence {
            field: s.field_name.clone(),
            signer: s
                .signer_cert_der
                .as_deref()
                .and_then(|der| crate::identity::info(der).ok()),
            status: format!("{:?}", s.status),
            crypto_valid: s.cryptographic_validity == CryptoValidity::Valid,
            digest_matches: s.digest_matches,
            byte_range_valid: s.integrity_ok,
            covers_current_file: s.covers_whole_document,
            trusted: s.chain_trusted,
            certificate_validity: format!("{:?}", s.certificate_validity),
            claimed_signing_time: s.signing_time.clone(),
            revocation: "not checked (offline validation)".into(),
            timestamp: s.timestamp_time.clone(),
            certificate_binding_valid: s.ess_cert_id_match == Some(true),
        })
        .collect())
}

pub async fn sign(
    bytes: &[u8],
    signer: &dyn CryptoSigner,
    field: &str,
    visible: bool,
) -> Result<Vec<u8>> {
    let before = checked_input(bytes, signer, field)?;
    let config = visible.then(|| VisibleSignatureConfig {
        page: 0,
        rect: SignatureRect::Absolute {
            llx: 72.0,
            lly: 140.0,
            urx: 420.0,
            ury: 210.0,
        },
        layout: SignatureLayout::TextOnly(TextConfig {
            lines: vec![
                TextLine::new("MyPDF certificate signature"),
                TextLine::new(format!("Field: {field}")),
                TextLine::new("Identity and integrity: see signature details"),
            ],
            ..Default::default()
        }),
        background_color: None,
        border: Some(Border::default()),
    });
    let output = PdfSigner::new()
        .options(SigningOptions {
            field_name: field.into(),
            digest_algorithm: signer.digest_algorithm(),
            visible_signature: config,
            reason: Some("MyPDF milestone 0 interoperability proof".into()),
            ..Default::default()
        })
        .sign(bytes, signer)
        .await
        .context("PDF signing failed")?;
    ensure!(
        output.starts_with(bytes),
        "signing rewrote previous document bytes"
    );
    let after = verify(&output, None)?;
    ensure!(
        after.len() == before.len() + 1
            && after.iter().all(|s| s.crypto_valid
                && s.digest_matches
                && s.byte_range_valid
                && s.certificate_binding_valid),
        "signed output failed verification: {after:?}"
    );
    Ok(output)
}

fn checked_input(
    bytes: &[u8],
    signer: &dyn CryptoSigner,
    field: &str,
) -> Result<Vec<SignatureEvidence>> {
    ensure!(
        !field.is_empty()
            && field.len() <= 100
            && field
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
        "use a signature field name of 1–100 ASCII letters, digits, underscores or hyphens"
    );
    crate::identity::check_certificate(signer.certificate_der())?;
    let doc = lopdf::Document::load_mem(bytes)?;
    for value in doc.objects.values() {
        if let Ok(dict) = value.as_dict()
            && let Ok(name) = dict.get(b"T").and_then(lopdf::Object::as_str)
        {
            let decoded = if name.starts_with(&[0xfe, 0xff]) {
                String::from_utf16_lossy(
                    &name[2..]
                        .chunks_exact(2)
                        .map(|v| u16::from_be_bytes([v[0], v[1]]))
                        .collect::<Vec<_>>(),
                )
            } else {
                String::from_utf8_lossy(name).into_owned()
            };
            ensure!(
                decoded != field,
                "A form field already uses this name. Choose a new signature field name."
            );
        }
    }
    ensure!(
        matches!(
            doc.reference_table.cross_reference_type,
            lopdf::xref::XrefType::CrossReferenceTable
        ),
        "cross-reference-stream inputs fail the current writer's Adobe interoperability gate; use prepare-signing on an unsigned copy first. Existing signed streams must not be rewritten"
    );
    ensure!(
        !doc.is_encrypted(),
        "encrypted-PDF signing needs a later interoperability gate"
    );
    ensure!(doc.get_pages().contains_key(&1), "PDF has no pages");
    // Reject certification constraints until DocMDP policy handling is proved.
    ensure!(
        doc.catalog()?.get(b"Perms").is_err(),
        "Certified PDF signing is not supported yet"
    );
    let prior = underskrift::verify::extractor::extract_signatures(bytes)?;
    ensure!(
        !prior.iter().any(|s| s.field_name == field),
        "signature field name is already used"
    );
    let before = verify(bytes, None)?;
    ensure!(
        before
            .iter()
            .all(|s| s.crypto_valid && s.digest_matches && s.byte_range_valid),
        "existing signature failed integrity checks"
    );
    Ok(before)
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesktopSigningRequest {
    pub field: String,
    pub page: usize,
    pub rect: Option<crate::annotations::Box2>,
    pub name: String,
    pub reason: String,
    pub location: String,
    pub artwork: Option<crate::assets::Asset>,
}
impl Default for DesktopSigningRequest {
    fn default() -> Self {
        Self {
            field: format!("MyPDF_{:08x}", rand::random::<u32>()),
            page: 0,
            rect: Some([72.0, 580.0, 390.0, 676.0]),
            name: String::new(),
            reason: String::new(),
            location: String::new(),
            artwork: None,
        }
    }
}

pub fn signature_artwork(request: &DesktopSigningRequest) -> Result<Vec<u8>> {
    use crate::annotations::{Annotation, Kind, Style};
    use lopdf::{Document, Object, Stream, dictionary};
    let rect = request
        .rect
        .context("An invisible signature has no appearance")?;
    let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
    ensure!(
        w.is_finite()
            && h.is_finite()
            && (120.0..=1500.0).contains(&w)
            && (48.0..=600.0).contains(&h),
        "Draw a signature rectangle at least 120 × 48 points, up to 1500 × 600"
    );
    ensure!(
        request.name.len() <= 200 && request.reason.len() <= 1000 && request.location.len() <= 300,
        "Signature appearance details are too long"
    );
    let mut doc = Document::with_version("1.7");
    let pages = doc.new_object_id();
    let content = doc.add_object(Stream::new(dictionary! {}, vec![]));
    let page = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages, "MediaBox" => Object::Array(vec![0.into(), 0.into(), w.into(), h.into()]), "Resources" => dictionary! {}, "Contents" => content });
    doc.objects.insert(
        pages,
        dictionary! { "Type" => "Pages", "Kids" => vec![Object::Reference(page)], "Count" => 1 }
            .into(),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
    doc.trailer.set("Root", catalog);
    doc.reference_table.cross_reference_type = lopdf::xref::XrefType::CrossReferenceTable;
    let mut base = vec![];
    doc.save_to(&mut base)?;
    let mut annotations = vec![Annotation::new(
        0,
        [1.0, 1.0, w - 1.0, h - 1.0],
        Kind::Rectangle,
        Style {
            width: 0.6,
            ..Default::default()
        },
    )];
    let left = if let Some(artwork) = &request.artwork {
        artwork.validate()?;
        let aw = (w * 0.34 - 8.0).min((h - 16.0) * artwork.aspect);
        let ah = aw / artwork.aspect;
        let x = 8.0 + (w * 0.34 - 8.0 - aw) / 2.0;
        let y = (h - ah) / 2.0;
        let mut style = artwork.style.clone();
        if matches!(artwork.kind, Kind::Text { .. } | Kind::Stamp { .. }) {
            style.font_size = style.font_size.min((ah - 8.0) / 1.25).max(6.0);
        }
        annotations.push(Annotation::new(
            0,
            [x, y, x + aw, y + ah],
            artwork.kind.clone(),
            style,
        ));
        w * 0.36
    } else {
        8.0
    };
    let mut text = format!(
        "Digitally signed by {}\n{}",
        if request.name.trim().is_empty() {
            "certificate holder"
        } else {
            &request.name
        },
        time::OffsetDateTime::now_utc().replace_nanosecond(0)?
    );
    if !request.reason.trim().is_empty() {
        text.push_str(&format!("\n{}", request.reason));
    }
    if !request.location.trim().is_empty() {
        text.push_str(&format!("\n{}", request.location));
    }
    // Try a bounded font-size reduction; never clip or hide appearance details.
    let mut last = None;
    for size in [11.0, 10.0, 9.0, 8.0, 7.0, 6.0] {
        let mut current = annotations.clone();
        current.push(Annotation::new(
            0,
            [left, 4.0, w - 5.0, h - 4.0],
            Kind::Text { text: text.clone() },
            Style {
                font_size: size,
                color: [0.12, 0.19, 0.18],
                ..Default::default()
            },
        ));
        match crate::annotations::export(&base, &current) {
            Ok(bytes) => {
                let pdf = hayro::hayro_syntax::Pdf::new(bytes)
                    .map_err(|e| anyhow::anyhow!("Signature appearance: {e:?}"))?;
                let raster = hayro::render(
                    &pdf.pages()[0],
                    &hayro::RenderCache::new(),
                    &hayro::hayro_interpret::InterpreterSettings::default(),
                    &hayro::RenderSettings::default(),
                    &hayro::PixmapSettings {
                        x_scale: 2.0,
                        y_scale: 2.0,
                        bg_color: hayro::vello_cpu::color::palette::css::WHITE,
                    },
                );
                return Ok(raster.into_png()?);
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("Signature details do not fit in the rectangle")))
}

pub async fn sign_custom(
    bytes: &[u8],
    signer: &dyn CryptoSigner,
    request: &DesktopSigningRequest,
) -> Result<Vec<u8>> {
    use hayro::hayro_interpret::TransformExt;
    use underskrift::visual::{ImageConfig, ImageFormat, ImageScale};
    let before = checked_input(bytes, signer, &request.field)?;
    ensure!(
        request.name.len() <= 200 && request.reason.len() <= 1000 && request.location.len() <= 300,
        "Signature details are too long"
    );
    let pdf = hayro::hayro_syntax::Pdf::new(bytes.to_vec())
        .map_err(|e| anyhow::anyhow!("PDF geometry: {e:?}"))?;
    let page = pdf
        .pages()
        .get(request.page)
        .context("Signature page is missing")?;
    let visible_signature = if let Some(rect) = request.rect {
        let (w, h) = page.render_dimensions();
        ensure!(
            rect.iter().all(|v| v.is_finite())
                && rect[0] >= 0.0
                && rect[1] >= 0.0
                && rect[2] <= w
                && rect[3] <= h,
            "Signature rectangle must fit within the displayed page"
        );
        let png = signature_artwork(request)?;
        let image = crate::assets::decode_png(&png)?.into_rgba8();
        use hayro::hayro_syntax::page::Rotation;
        let image = match page.rotation() {
            Rotation::None => image,
            Rotation::Horizontal => image::imageops::rotate270(&image),
            Rotation::Flipped => image::imageops::rotate180(&image),
            Rotation::FlippedHorizontal => image::imageops::rotate90(&image),
        };
        let mut data = std::io::Cursor::new(vec![]);
        image::DynamicImage::ImageRgba8(image).write_to(&mut data, image::ImageFormat::Png)?;
        let rect = page
            .initial_transform(true)
            .to_kurbo()
            .inverse()
            .transform_rect_bbox(hayro::kurbo::Rect::new(
                rect[0] as f64,
                rect[1] as f64,
                rect[2] as f64,
                rect[3] as f64,
            ));
        Some(VisibleSignatureConfig {
            page: request.page as u32,
            rect: SignatureRect::Absolute {
                llx: rect.x0 as f32,
                lly: rect.y0 as f32,
                urx: rect.x1 as f32,
                ury: rect.y1 as f32,
            },
            layout: SignatureLayout::ImageOnly(ImageConfig {
                data: data.into_inner(),
                format: ImageFormat::Png,
                scale: ImageScale::Stretch,
            }),
            background_color: None,
            border: None,
        })
    } else {
        None
    };
    let output = PdfSigner::new()
        .options(SigningOptions {
            field_name: request.field.clone(),
            page: request.page as u32,
            digest_algorithm: signer.digest_algorithm(),
            visible_signature,
            reason: (!request.reason.trim().is_empty()).then(|| request.reason.clone()),
            location: (!request.location.trim().is_empty()).then(|| request.location.clone()),
            ..Default::default()
        })
        .sign(bytes, signer)
        .await
        .context("PDF signing failed")?;
    ensure!(
        output.starts_with(bytes),
        "Signing rewrote earlier document bytes"
    );
    let after = verify(&output, None)?;
    ensure!(
        after.len() == before.len() + 1
            && after.iter().all(|s| s.crypto_valid
                && s.digest_matches
                && s.byte_range_valid
                && s.certificate_binding_valid),
        "Signed output failed verification"
    );
    Ok(output)
}

/// Extract exact detached CMS and signed byte ranges for an independent OpenSSL
/// verifier. This does not use the signing library's cryptographic verifier.
pub struct DetachedSignature {
    pub label: String,
    pub cms: Vec<u8>,
    pub signed_bytes: Vec<u8>,
}

pub fn detached_parts(bytes: &[u8]) -> Result<Vec<DetachedSignature>> {
    let doc = lopdf::Document::load_mem(bytes)?;
    let mut parts = Vec::new();
    for o in doc.objects.values() {
        let Ok(d) = o.as_dict() else { continue };
        if !d
            .get(b"Type")
            .and_then(lopdf::Object::as_name)
            .is_ok_and(|s| s == b"Sig")
        {
            continue;
        }
        let range = d.get(b"ByteRange")?.as_array()?;
        ensure!(range.len() == 4, "unsupported ByteRange");
        let offsets: Vec<_> = range
            .iter()
            .map(|v| usize::try_from(v.as_i64()?).context("invalid byte range"))
            .collect::<Result<_>>()?;
        let mut signed = Vec::new();
        for pair in offsets.chunks_exact(2) {
            let end = pair[0]
                .checked_add(pair[1])
                .context("byte range overflow")?;
            signed.extend_from_slice(bytes.get(pair[0]..end).context("byte range exceeds file")?);
        }
        let cms = d.get(b"Contents")?.as_str()?.to_vec();
        parts.push(DetachedSignature {
            label: format!("signature-{}", parts.len() + 1),
            cms,
            signed_bytes: signed,
        });
    }
    Ok(parts)
}
