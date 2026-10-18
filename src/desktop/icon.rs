//! Approved calico artwork shared by the application and platform packaging.
//! Embed the master so installed apps never depend on a source-tree asset path.

pub fn rgba(size: u32) -> Vec<u8> {
    // Runtime sizes are exported once, so opening a window never resamples the
    // full-resolution master on the UI thread (especially in debug builds).
    let prepared: Option<&[u8]> = match size {
        128 => Some(include_bytes!(
            "../../packaging/branding/MyPDF-calico-128.png"
        )),
        256 => Some(include_bytes!(
            "../../packaging/branding/MyPDF-calico-256.png"
        )),
        _ => None,
    };
    if let Some(png) = prepared {
        return image::load_from_memory_with_format(png, image::ImageFormat::Png)
            .expect("the bundled runtime icon must be a valid PNG")
            .into_rgba8()
            .into_raw();
    }
    render_master(size)
}

/// Used by the asset exporter to regenerate runtime sizes from the master.
pub fn render_master(size: u32) -> Vec<u8> {
    if size == 0 {
        return Vec::new();
    }
    let mut master = image::load_from_memory_with_format(
        include_bytes!("../../packaging/branding/MyPDF-calico-v2.png"),
        image::ImageFormat::Png,
    )
    .expect("the bundled calico app icon must be a valid PNG")
    .into_rgba32f();
    // Filter premultiplied color, otherwise transparent pixels produce dark
    // fringes around the cat and rounded tile at small Dock/taskbar sizes.
    for pixel in master.pixels_mut() {
        for channel in 0..3 {
            pixel[channel] *= pixel[3];
        }
    }
    let scaled =
        image::imageops::resize(&master, size, size, image::imageops::FilterType::Lanczos3);
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for pixel in scaled.pixels() {
        let alpha = pixel[3].clamp(0., 1.);
        for channel in 0..3 {
            let value = if alpha > 0. {
                pixel[channel] / alpha
            } else {
                0.
            };
            pixels.push((value.clamp(0., 1.) * 255.).round() as u8);
        }
        pixels.push((alpha * 255.).round() as u8);
    }
    pixels
}
