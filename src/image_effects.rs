//! Local image blur with retained source pixels and a bounded appearance cache.
use anyhow::{Result, ensure};
use std::sync::{Arc, Mutex, OnceLock};

type Cache = Vec<(String, Arc<Vec<u8>>)>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RasterOptions {
    pub blur: f32,
    pub opacity: f32,
    pub border: u32,
    pub color: [u8; 3],
}
impl Default for RasterOptions {
    fn default() -> Self {
        Self {
            blur: 0.,
            opacity: 1.,
            border: 0,
            color: [35, 45, 65],
        }
    }
}
/// Page-content images have no MyPDF annotation metadata. Bake the chosen
/// appearance into a new PNG; document Undo retains their original pixels.
pub fn raster(png: &Arc<Vec<u8>>, options: RasterOptions) -> Result<Vec<u8>> {
    ensure!(
        options.opacity.is_finite()
            && (0.0..=1.).contains(&options.opacity)
            && options.border <= 100,
        "Invalid image effect"
    );
    let blurred = blurred(png, options.blur)?;
    let mut image = crate::assets::decode_png(&blurred)?.into_rgba8();
    let (w, h) = image.dimensions();
    let border = options.border.min(w / 2).min(h / 2);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        if x < border || y < border || x >= w - border || y >= h - border {
            *pixel = image::Rgba([options.color[0], options.color[1], options.color[2], 255]);
        }
        pixel[3] = (pixel[3] as f32 * options.opacity).round() as u8;
    }
    let mut output = std::io::Cursor::new(Vec::new());
    image.write_to(&mut output, image::ImageFormat::Png)?;
    ensure!(
        output.get_ref().len() <= crate::assets::MAX_ASSET_BYTES,
        "Adjusted image exceeds the asset size limit"
    );
    Ok(output.into_inner())
}
pub fn blurred(png: &Arc<Vec<u8>>, radius: f32) -> Result<Arc<Vec<u8>>> {
    ensure!(
        radius.is_finite() && (0.0..=32.).contains(&radius),
        "Blur must be between 0 and 32 pixels"
    );
    if radius == 0. {
        return Ok(png.clone());
    }
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(Vec::new()));
    let key = format!("{}:{radius}", crate::identity::fingerprint(png));
    if let Some((_, result)) = cache.lock().unwrap().iter().find(|(k, _)| k == &key) {
        return Ok(result.clone());
    }
    let mut image = crate::assets::decode_png(png)?.into_rgba8();
    // Blur premultiplied colors so transparent pixels do not introduce dark halos.
    for pixel in image.pixels_mut() {
        for channel in 0..3 {
            pixel[channel] = ((pixel[channel] as u16 * pixel[3] as u16 + 127) / 255) as u8;
        }
    }
    let mut image = image::imageops::fast_blur(&image, radius);
    for pixel in image.pixels_mut() {
        if pixel[3] > 0 {
            for channel in 0..3 {
                pixel[channel] = ((pixel[channel] as u32 * 255 + pixel[3] as u32 / 2)
                    / pixel[3] as u32)
                    .min(255) as u8;
            }
        }
    }
    let mut output = std::io::Cursor::new(Vec::new());
    image.write_to(&mut output, image::ImageFormat::Png)?;
    let result = Arc::new(output.into_inner());
    ensure!(
        result.len() <= crate::assets::MAX_ASSET_BYTES,
        "Blurred image exceeds the asset size limit"
    );
    let mut cache = cache.lock().unwrap();
    while !cache.is_empty()
        && (cache.len() >= 8
            || cache.iter().map(|(_, p)| p.len()).sum::<usize>() + result.len() > 32 * 1024 * 1024)
    {
        cache.remove(0);
    }
    cache.push((key, result.clone()));
    Ok(result)
}
