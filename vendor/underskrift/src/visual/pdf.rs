//! Import a generated signature PDF as a Form XObject without rasterization.
use super::{AppearanceStream, Border, Color};
use crate::error::VisualError;
use lopdf::{dictionary, Document, Object, Stream};

pub(super) fn build_pdf_appearance(
    data: &[u8],
    width: f32,
    height: f32,
    background: Option<&Color>,
    border: Option<&Border>,
) -> Result<AppearanceStream, VisualError> {
    let invalid = |e: lopdf::Error| VisualError::AppearanceError(e.to_string());
    let mut doc = Document::load_mem(data).map_err(invalid)?;
    let pages = doc.get_pages();
    if doc.is_encrypted() || pages.len() != 1 {
        return Err(VisualError::AppearanceError(
            "A signature appearance must be a single unencrypted PDF page".into(),
        ));
    }
    let page = *pages.values().next().unwrap();
    let dict = doc.get_dictionary(page).map_err(invalid)?;
    // The caller supplies a normalized page with its appearances painted into
    // the content and with direct MediaBox/Resources (no inherited geometry).
    if dict.has(b"Annots") || dict.has(b"Rotate") || dict.has(b"CropBox") {
        return Err(VisualError::AppearanceError(
            "Normalize the signature PDF geometry and paint its annotations first".into(),
        ));
    }
    let bbox = dict
        .get(b"MediaBox")
        .and_then(Object::as_array)
        .map_err(invalid)?;
    let bbox = bbox
        .iter()
        .map(Object::as_float)
        .collect::<Result<Vec<_>, _>>()
        .map_err(invalid)?;
    if bbox.len() != 4
        || !bbox.iter().all(|v| v.is_finite())
        || bbox[0] != 0.
        || bbox[1] != 0.
        || bbox[2] <= 0.
        || bbox[3] <= 0.
    {
        return Err(VisualError::InvalidDimensions(
            "Invalid signature PDF bounds".into(),
        ));
    }
    let resources = dict.get(b"Resources").map_err(invalid)?.clone();
    let content = doc.get_page_content(page).map_err(invalid)?;
    let form = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "FormType" => 1,
            "BBox" => bbox.iter().copied().map(Object::Real).collect::<Vec<_>>(),
            "Resources" => resources,
        },
        content,
    ));
    doc.trailer = dictionary! { "Root" => form };
    doc.prune_objects();
    // Missing resource references must fail before we sign, not produce a PDF
    // with a valid cryptographic signature but incomplete visible artwork.
    for id in doc.traverse_objects(|_| {}) {
        doc.get_object(id).map_err(invalid)?;
    }
    let mut content = String::from("q\n");
    if let Some(bg) = background {
        content.push_str(&format!(
            "{} {} {} rg 0 0 {width} {height} re f\n",
            bg.r, bg.g, bg.b
        ));
    }
    if let Some(border) = border {
        content.push_str(&format!(
            "{} {} {} RG {} w 0 0 {width} {height} re S\n",
            border.color.r, border.color.g, border.color.b, border.width
        ));
    }
    content.push_str(&format!(
        "{} 0 0 {} 0 0 cm /PdfAppearance Do\nQ\n",
        width / bbox[2],
        height / bbox[3]
    ));
    Ok(AppearanceStream {
        content: content.into_bytes(),
        font_resources: Vec::new(),
        embedded_font_resources: Vec::new(),
        image_resources: Vec::new(),
        pdf_resources: Some(doc),
        bbox: [0., 0., width, height],
    })
}
