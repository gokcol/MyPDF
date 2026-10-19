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
    /// Who vouches for the signer: "your trusted certificates", "macOS" or
    /// "Windows". None when the signer is not trusted.
    pub trusted_by: Option<String>,
    pub certificate_validity: String,
    pub claimed_signing_time: Option<String>,
    pub revocation: String,
    pub timestamp: Option<String>,
    pub certificate_binding_valid: bool,
    /// The signer's certificate, for display, export and session trust.
    #[serde(skip)]
    pub signer_der: Option<Vec<u8>>,
}

pub fn verify(bytes: &[u8], trusted_cert: Option<&[u8]>) -> Result<Vec<SignatureEvidence>> {
    verify_trusting(bytes, &trusted_cert.into_iter().collect::<Vec<_>>())
}
/// Verify against several explicitly trusted certificates.
pub fn verify_trusting(bytes: &[u8], trusted: &[&[u8]]) -> Result<Vec<SignatureEvidence>> {
    if underskrift::verify::extractor::extract_signatures(bytes)?.is_empty() {
        return Ok(Vec::new());
    }
    let mut stores = TrustStoreSet::new();
    if !trusted.is_empty() {
        let mut trust = TrustStore::new();
        for cert in trusted {
            trust.add_der_certificate(cert)?;
        }
        stores = stores.with_sig_store(trust);
    }
    let report = SignatureVerifier::new(&stores)
        .allow_online(false)
        .verify_pdf(bytes)?;
    Ok(report
        .signatures
        .iter()
        .map(|s| {
            // The operating system's trust: its roots and the user's trust settings.
            let chain: Vec<Vec<u8>> = s
                .signer_cert_der
                .iter()
                .chain(&s.chain_certs_der)
                .cloned()
                .collect();
            let trusted_by = if s.chain_trusted {
                Some("your trusted certificates".to_owned())
            } else if s.signer_cert_der.is_some() && crate::trust::system_trusts(&chain) {
                crate::trust::system_name().map(str::to_owned)
            } else {
                None
            };
            (s, trusted_by)
        })
        .map(|(s, trusted_by)| SignatureEvidence {
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
            trusted: trusted_by.is_some(),
            trusted_by,
            certificate_validity: format!("{:?}", s.certificate_validity),
            claimed_signing_time: s.signing_time.clone(),
            revocation: "not checked (offline validation)".into(),
            timestamp: s.timestamp_time.clone(),
            certificate_binding_valid: s.ess_cert_id_match == Some(true),
            signer_der: s.signer_cert_der.clone(),
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

/// The form's empty signature field with this name, if there is one to sign into.
pub fn empty_signature_field(bytes: &[u8], name: &str) -> Option<lopdf::ObjectId> {
    crate::forms::fields(bytes)
        .ok()?
        .into_iter()
        .find(|f| {
            f.name == name
                && matches!(f.kind, crate::forms::FieldKind::Signature { signed: false })
                && f.widgets.len() == 1
                && f.widgets[0].id == f.id
        })
        .map(|f| f.id)
}

fn checked_input(
    bytes: &[u8],
    signer: &dyn CryptoSigner,
    field: &str,
) -> Result<Vec<SignatureEvidence>> {
    if empty_signature_field(bytes, field).is_some() {
        crate::identity::check_certificate(signer.certificate_der())?;
        return checked_document(bytes, field);
    }
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
    checked_document(bytes, field)
}
/// Whether the document can take another signature.
fn checked_document(bytes: &[u8], field: &str) -> Result<Vec<SignatureEvidence>> {
    let doc = lopdf::Document::load_mem(bytes)?;
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

/// How a visible signature box is composed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub enum AppearanceLayout {
    /// Artwork beside "Digitally signed by", the date and any reason or location.
    #[default]
    ImageAndDetails,
    /// Only the artwork, such as a handwritten signature, on a transparent background.
    ImageOnly,
    /// Only the signing details, in a bordered box.
    DetailsOnly,
}
impl AppearanceLayout {
    /// The smallest signature box, in points, that keeps the appearance legible.
    pub fn minimum_size(self) -> (f32, f32) {
        match self {
            Self::ImageOnly => (24.0, 12.0),
            _ => (120.0, 48.0),
        }
    }
}

/// What sits behind a visible signature.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub enum Background {
    /// Image-only appearances are clear; others are white with a thin frame.
    #[default]
    Auto,
    /// Nothing: the page shows through.
    Clear,
    White,
    /// A soft blue-gray card.
    Tinted,
    /// White with a thin frame.
    Framed,
}
impl Background {
    pub const CHOICES: [Background; 4] = [
        Background::Clear,
        Background::White,
        Background::Tinted,
        Background::Framed,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Background::Auto => "Automatic",
            Background::Clear => "None",
            Background::White => "White",
            Background::Tinted => "Soft tint",
            Background::Framed => "Framed",
        }
    }
    /// Fill and frame colors, if any.
    pub fn colors(self) -> Option<([f32; 3], Option<[f32; 3]>)> {
        match self {
            Background::Auto | Background::Clear => None,
            Background::White => Some(([1.0; 3], None)),
            Background::Tinted => Some(([0.925, 0.945, 0.99], None)),
            Background::Framed => Some(([1.0; 3], Some([0.11, 0.43, 0.35]))),
        }
    }
}

/// Composition controls shared by saved signatures and their rendered appearance.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AppearanceTransform {
    /// Clockwise artwork rotation. Details and background remain upright.
    pub rotation: f32,
    /// Draw the artwork over the details instead of beside them.
    pub image_overlay: bool,
    pub image_opacity: f32,
    /// Insets as percentages of the image area: left, top, right, bottom.
    pub image_insets: [f32; 4],
}
impl Default for AppearanceTransform {
    fn default() -> Self {
        Self {
            rotation: 0.,
            image_overlay: false,
            image_opacity: 1.,
            image_insets: [0.; 4],
        }
    }
}

/// Text controls are saved with the signature, using family names portable between computers.
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SignatureTextStyle {
    pub offset: [f32; 2],
    /// Insets as percentages of the details area: left, top, right, bottom.
    pub insets: [f32; 4],
    pub family: String,
    pub bold: bool,
    pub italic: bool,
    pub size: f32,
    pub color: [f32; 3],
    pub alignment: u8,
}
impl Default for SignatureTextStyle {
    fn default() -> Self {
        Self {
            offset: [0., 0.],
            insets: [0.; 4],
            family: String::new(),
            bold: false,
            italic: false,
            size: 11.,
            color: [0.12, 0.19, 0.18],
            alignment: 0,
        }
    }
}
impl SignatureTextStyle {
    pub fn font(&self) -> Result<std::path::PathBuf> {
        crate::text_style::fonts()
            .iter()
            .find(|f| {
                (self.family.is_empty() || f.family == self.family)
                    && f.bold == self.bold
                    && f.italic == self.italic
            })
            .or_else(|| {
                crate::text_style::fonts()
                    .iter()
                    .find(|f| f.family == self.family)
            })
            .map(|f| f.path.clone())
            .map(Ok)
            .unwrap_or_else(crate::document::default_font)
    }
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
    pub layout: AppearanceLayout,
    pub transform: AppearanceTransform,
    pub text_style: SignatureTextStyle,
    /// "Digitally signed by …" with the signer's name.
    pub show_name: bool,
    /// The signing date and time.
    pub show_date: bool,
    pub background: Background,
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
            layout: AppearanceLayout::default(),
            transform: AppearanceTransform::default(),
            text_style: SignatureTextStyle::default(),
            show_name: true,
            show_date: true,
            background: Background::Auto,
        }
    }
}

/// The signing time as shown in a signature, such as "2026-10-14 18:30 UTC".
fn signing_time_label() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!("{} {:02}:{:02} UTC", now.date(), now.hour(), now.minute())
}

pub fn signature_artwork(request: &DesktopSigningRequest) -> Result<Vec<u8>> {
    use crate::annotations::{Annotation, Kind, Style};
    use lopdf::{Document, Object, Stream, dictionary};
    let rect = request
        .rect
        .context("An invisible signature has no appearance")?;
    let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
    let (min_w, min_h) = request.layout.minimum_size();
    ensure!(
        w.is_finite()
            && h.is_finite()
            && (min_w..=1500.0).contains(&w)
            && (min_h..=600.0).contains(&h),
        "Draw a signature rectangle at least {min_w} × {min_h} points, up to 1500 × 600"
    );
    ensure!(
        request.name.len() <= 200 && request.reason.len() <= 1000 && request.location.len() <= 300,
        "Signature appearance details are too long"
    );
    ensure!(
        request.transform.rotation.is_finite()
            && request.transform.rotation.abs() <= 360.
            && request.transform.image_opacity.is_finite()
            && (0.0..=1.0).contains(&request.transform.image_opacity),
        "Invalid signature rotation or image opacity"
    );
    ensure!(
        request
            .text_style
            .offset
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 100.)
            && request.text_style.size.is_finite()
            && (1. ..=72.).contains(&request.text_style.size)
            && request
                .text_style
                .color
                .iter()
                .all(|v| v.is_finite() && (0. ..=1.).contains(v))
            && request.text_style.alignment <= 2,
        "Invalid signature text settings"
    );
    for insets in [request.transform.image_insets, request.text_style.insets] {
        ensure!(
            insets
                .iter()
                .all(|v| v.is_finite() && (0. ..=45.).contains(v)),
            "Signature edge spacing must be between 0 and 45 percent"
        );
    }
    // A small box is laid out larger and scaled down as a whole, like an image,
    // so its details always fit and keep their proportions.
    let k = (160.0 / w).max(64.0 / h).max(1.0);
    let (w, h) = (w * k, h * k);
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
    let render_pdf = |bytes: Vec<u8>, transparent: bool| -> Result<Vec<u8>> {
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
                bg_color: if transparent {
                    hayro::vello_cpu::color::palette::css::TRANSPARENT
                } else {
                    hayro::vello_cpu::color::palette::css::WHITE
                },
            },
        );
        Ok(raster.into_png()?)
    };
    let render = |marks: Vec<Annotation>, transparent: bool| -> Result<Vec<u8>> {
        render_pdf(crate::annotations::export(&base, &marks)?, transparent)
    };
    // Fit the rotated artwork in its allotted area without rotating the details
    // or the background. Ink coordinates remain normalized inside this rectangle.
    let rotate_image =
        |mut mark: Annotation, available_w: f32, available_h: f32| -> Result<Annotation> {
            let (sin, cos) = request.transform.rotation.to_radians().sin_cos();
            let [x, y, right, bottom] = mark.rect;
            let (aw, ah) = (right - x, bottom - y);
            let fit = (available_w / (aw * cos.abs() + ah * sin.abs()))
                .min(available_h / (aw * sin.abs() + ah * cos.abs()))
                .min(1.);
            let (cx, cy) = ((x + right) / 2., (y + bottom) / 2.);
            mark.rect = [
                cx - aw * fit / 2.,
                cy - ah * fit / 2.,
                cx + aw * fit / 2.,
                cy + ah * fit / 2.,
            ];
            mark.rotation = request.transform.rotation;
            if matches!(mark.kind, Kind::Text { .. } | Kind::Stamp { .. }) {
                mark.style.font_size = (mark.style.font_size * fit).max(1.);
            }
            fit_text(&base, mark)
        };
    let background = match request.background {
        Background::Auto if request.layout == AppearanceLayout::ImageOnly => Background::Clear,
        Background::Auto => Background::Framed,
        chosen => chosen,
    };
    let transparent = background == Background::Clear;
    let backdrop: Vec<Annotation> = background
        .colors()
        .map(|(fill, frame)| {
            Annotation::new(
                0,
                if frame.is_some() {
                    [1.0, 1.0, w - 1.0, h - 1.0]
                } else {
                    [0.0, 0.0, w, h]
                },
                Kind::Rectangle,
                Style {
                    fill: Some(fill),
                    color: frame.unwrap_or(fill),
                    width: if frame.is_some() { 0.6 } else { 0.1 },
                    ..Default::default()
                },
            )
        })
        .into_iter()
        .collect();
    let mut lines = vec![];
    if request.layout != AppearanceLayout::ImageOnly {
        if request.show_name {
            lines.push(format!(
                "Digitally signed by {}",
                if request.name.trim().is_empty() {
                    "certificate holder"
                } else {
                    request.name.trim()
                }
            ));
        }
        if request.show_date {
            lines.push(signing_time_label());
        }
        for extra in [&request.reason, &request.location] {
            if !extra.trim().is_empty() {
                lines.push(extra.trim().to_owned());
            }
        }
    }
    let inset = |[x, y, r, b]: [f32; 4], edges: [f32; 4]| {
        let (width, height) = (r - x, b - y);
        [
            x + width * edges[0] / 100.,
            y + height * edges[1] / 100.,
            r - width * edges[2] / 100.,
            b - height * edges[3] / 100.,
        ]
    };
    // With no details to show, the image fills the box on its own.
    if request.layout == AppearanceLayout::ImageOnly || lines.is_empty() {
        let artwork = request
            .artwork
            .as_ref()
            .context("Choose what the signature shows: your name, the date, a note or an image")?;
        artwork.validate()?;
        if request.transform.image_opacity == 0. {
            return render(backdrop, transparent);
        }
        let [left, top, right, bottom] =
            inset([2., 2., w - 2., h - 2.], request.transform.image_insets);
        let (available_w, available_h) = (right - left, bottom - top);
        let aw = available_w.min(available_h * artwork.aspect);
        let ah = aw / artwork.aspect;
        let (x, y) = (
            left + (available_w - aw) / 2.,
            top + (available_h - ah) / 2.,
        );
        let mut style = artwork.style.clone();
        style.opacity *= request.transform.image_opacity;
        if !matches!(artwork.kind, Kind::Image { .. }) {
            style.opacity = style.opacity.max(0.01);
        }
        if matches!(artwork.kind, Kind::Text { .. } | Kind::Stamp { .. }) {
            style.font_size = style.font_size.min((ah - 4.0) / 1.25).max(1.0);
        }
        let mut marks = backdrop.clone();
        marks.push(rotate_image(
            Annotation::new(0, [x, y, x + aw, y + ah], artwork.kind.clone(), style),
            available_w,
            available_h,
        )?);
        return render(marks, transparent);
    }
    let mut annotations = backdrop;
    let artwork = request.artwork.as_ref().filter(|_| {
        request.layout == AppearanceLayout::ImageAndDetails && request.transform.image_opacity > 0.
    });
    let mut overlay = None;
    let left = if let Some(artwork) = artwork {
        artwork.validate()?;
        let image_right = if request.transform.image_overlay {
            w - 8.
        } else {
            w * 0.34
        };
        let [left, top, right, bottom] = inset(
            [8., 8., image_right, h - 8.],
            request.transform.image_insets,
        );
        let (available_w, available_h) = (right - left, bottom - top);
        let aw = available_w.min(available_h * artwork.aspect);
        let ah = aw / artwork.aspect;
        let x = left + (available_w - aw) / 2.;
        let y = top + (available_h - ah) / 2.;
        let mut style = artwork.style.clone();
        style.opacity *= request.transform.image_opacity;
        if !matches!(artwork.kind, Kind::Image { .. }) {
            style.opacity = style.opacity.max(0.01);
        }
        if matches!(artwork.kind, Kind::Text { .. } | Kind::Stamp { .. }) {
            style.font_size = style.font_size.min((ah - 8.0) / 1.25).max(1.0);
        }
        let image = rotate_image(
            Annotation::new(0, [x, y, x + aw, y + ah], artwork.kind.clone(), style),
            available_w,
            available_h,
        )?;
        if request.transform.image_overlay {
            overlay = Some(image);
            8.
        } else {
            annotations.push(image);
            w * 0.36
        }
    } else {
        8.0
    };
    // Move details independently from the image; shrink text only when needed to fit.
    let text_style = &request.text_style;
    let x = (left + w * text_style.offset[0] / 100.).clamp(0., w - 16.);
    let y = (4. + h * text_style.offset[1] / 100.).clamp(0., h - 8.);
    let [x, y, right, bottom] = inset([x, y, w - 5., h - 2.], text_style.insets);
    let width = (right - x).max(1.);
    let height = (bottom - y).max(1.);
    let font = text_style.font()?;
    let mut size = text_style.size;
    let (source, text_height) = loop {
        match crate::signature_art::lettering(
            &lines.join("\n"),
            &font,
            size,
            text_style.color,
            width,
            text_style.alignment,
        ) {
            Ok((source, text_height)) if text_height <= height => break (source, text_height),
            result if size <= 1. => {
                result?;
                anyhow::bail!("Move the text farther inside the signature box");
            }
            _ => size = (size * 0.9).max(1.),
        }
    };
    annotations.push(Annotation::new(
        0,
        [x, y, x + width, y + text_height],
        Kind::Svg {
            source,
            tint: false,
        },
        Style::default(),
    ));
    if let Some(image) = overlay {
        annotations.push(image);
    }
    render(annotations, transparent)
}

/// Shrink a text or stamp mark until it fits its box; other marks are kept as they are.
fn fit_text(
    base: &[u8],
    mut mark: crate::annotations::Annotation,
) -> Result<crate::annotations::Annotation> {
    use crate::annotations::Kind;
    if !matches!(mark.kind, Kind::Text { .. } | Kind::Stamp { .. }) {
        return Ok(mark);
    }
    let mut size = mark.style.font_size.max(1.0);
    loop {
        mark.style.font_size = size;
        match crate::annotations::export(base, std::slice::from_ref(&mark)) {
            Ok(_) => return Ok(mark),
            // At one point there is nothing legible left to shrink.
            Err(e) if size <= 1.0 => return Err(e),
            Err(_) => size = (size * 0.9).max(1.0),
        }
    }
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
            existing_field: empty_signature_field(bytes, &request.field),
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
