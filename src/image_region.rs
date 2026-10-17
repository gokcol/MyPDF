//! Copy visible page artwork without depending on the PDF's object structure.
//! The source is never changed; an optional cover is an ordinary removable mark.
use anyhow::{Context, Result, ensure};
use hayro::kurbo::{Affine, Rect};
use std::sync::Arc;

pub fn capture(bytes: &[u8], page: usize, rect: [f32; 4], dpi: f32) -> Result<Vec<u8>> {
    ensure!(
        dpi.is_finite() && (72.0..=300.).contains(&dpi),
        "Choose 72–300 DPI"
    );
    let geometry = crate::annotations::page_geometry(bytes)?;
    let (bounds, transform) = geometry.get(page).context("Page not found")?;
    ensure!(
        rect.iter().all(|v| v.is_finite())
            && rect[0] >= 0.
            && rect[1] >= 0.
            && rect[2] <= bounds[2] + 0.01
            && rect[3] <= bounds[3] + 0.01
            && rect[2] - rect[0] >= 2.
            && rect[3] - rect[1] >= 2.,
        "Select an area at least 2 × 2 points inside the page"
    );
    // Crop an analysis-only copy before rendering, so a small selection on a
    // huge page retains detail without allocating a full-page bitmap.
    let r = Affine::new(*transform).transform_rect_bbox(Rect::new(
        rect[0] as f64,
        rect[1] as f64,
        rect[2] as f64,
        rect[3] as f64,
    ));
    let mut doc = lopdf::Document::load_mem(bytes)?;
    let id = *doc
        .get_pages()
        .get(&(page as u32 + 1))
        .context("Page not found")?;
    doc.get_dictionary_mut(id)?.set(
        "CropBox",
        lopdf::Object::Array(
            [r.x0, r.y0, r.x1, r.y1]
                .into_iter()
                .map(|n| lopdf::Object::Real(n as f32))
                .collect(),
        ),
    );
    let mut cropped = Vec::new();
    doc.save_to(&mut cropped)?;
    let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
    let scale = (dpi / 72.)
        .min(4096. / w.max(h))
        .min((4_000_000. / (w * h)).sqrt());
    let ([width, height], rgba) = crate::viewer::render_page(&cropped, page, scale)?;
    let image = image::RgbaImage::from_raw(width as u32, height as u32, rgba)
        .context("Could not capture this region")?;
    let mut output = std::io::Cursor::new(Vec::new());
    image.write_to(&mut output, image::ImageFormat::Png)?;
    ensure!(
        output.get_ref().len() <= crate::assets::MAX_ASSET_BYTES,
        "This copy is too large. Select a smaller region or lower the quality."
    );
    Ok(output.into_inner())
}

pub fn layers(
    page: usize,
    rect: [f32; 4],
    png: Arc<Vec<u8>>,
    cover: Option<[f32; 3]>,
) -> Result<Vec<crate::annotations::Annotation>> {
    use crate::annotations::{Annotation, Kind, Style};
    let mut layers = Vec::new();
    if let Some(color) = cover {
        layers.push(Annotation::new(
            page,
            rect,
            Kind::Rectangle,
            Style {
                color,
                fill: Some(color),
                width: 0.1,
                ..Default::default()
            },
        ));
    }
    layers.push(Annotation::new(
        page,
        rect,
        Kind::Image { png },
        Style::default(),
    ));
    for layer in &layers {
        layer.validate()?;
    }
    Ok(layers)
}
