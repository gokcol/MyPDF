//! Display-sized, coverage-filtered artwork textures. The saved PNG is untouched.
use super::*;
use image::{ImageReader, Rgba, RgbaImage};
use mypdf_milestone0::assets;
use std::io::Cursor;

const CACHE_BYTES: usize = 64 * 1024 * 1024;
const MAX_PREVIEW_EDGE: u32 = 2048;

/// Select a small, reusable texture level from the artwork's physical display
/// size. Keeping the level just above the display size avoids both aliasing
/// from severe GPU minification and repeated decoding for every zoom tick.
pub(super) fn texture(
    ctx: &egui::Context,
    png: &[u8],
    display_points: egui::Vec2,
    images: &mut HashMap<String, TextureHandle>,
    image_used: &mut HashMap<String, u64>,
) -> Option<TextureHandle> {
    if png.len() > assets::MAX_ASSET_BYTES {
        return None;
    }
    let source = ImageReader::new(Cursor::new(png))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()?;
    if source.0 == 0 || source.1 == 0 || source.0 > 4096 || source.1 > 4096 {
        return None;
    }
    let size = level_size(source, display_points, ctx.pixels_per_point());
    let key = format!(
        "annotation-image:{}:{}x{}",
        mypdf_milestone0::identity::fingerprint(png),
        size.0,
        size.1
    );
    if !images.contains_key(&key) {
        let source = assets::decode_png(png).ok()?.into_rgba8();
        let filtered = if source.dimensions() == size {
            source
        } else {
            area_reduce(&source, size)
        };
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [size.0 as usize, size.1 as usize],
            filtered.as_raw(),
        );
        let texture = ctx.load_texture(&key, image, egui::TextureOptions::LINEAR);
        let bytes = size.0 as usize * size.1 as usize * 4;
        trim_cache(images, image_used, CACHE_BYTES.saturating_sub(bytes));
        images.insert(key.clone(), texture);
    }
    image_used.insert(key.clone(), ctx.cumulative_frame_nr());
    images.get(&key).cloned()
}

fn level_size(source: (u32, u32), display_points: egui::Vec2, pixels_per_point: f32) -> (u32, u32) {
    let density = if pixels_per_point.is_finite() {
        pixels_per_point.max(0.01)
    } else {
        1.0
    };
    let finite_size = |v: f32| {
        if v.is_finite() {
            v.abs().max(1.0) * density
        } else {
            MAX_PREVIEW_EDGE as f32
        }
    };
    let ratio = (finite_size(display_points.x) / source.0 as f32)
        .max(finite_size(display_points.y) / source.1 as f32);
    let edge = source.0.max(source.1);
    let wanted = (edge as f32 * ratio).ceil();
    // Alternating powers of two and their midpoint: at most 1.5x the required
    // linear resolution, with a fixed number of cache entries across zooms.
    let mut level = MAX_PREVIEW_EDGE;
    for power in [1_u32, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048] {
        if wanted <= power as f32 {
            level = power;
            break;
        }
        let midpoint = power + power / 2;
        if midpoint <= MAX_PREVIEW_EDGE && wanted <= midpoint as f32 {
            level = midpoint;
            break;
        }
    }
    let scale = (level.min(edge) as f64 / edge as f64).min(1.0);
    (
        ((source.0 as f64 * scale).round() as u32).max(1),
        ((source.1 as f64 * scale).round() as u32).max(1),
    )
}

/// Exact source-pixel coverage averaging. Accumulating premultiplied channels
/// stops invisible RGB outside an ink stroke from creating dark/color fringes.
/// This never increases resolution and needs only the final RGBA allocation.
fn area_reduce(source: &RgbaImage, size: (u32, u32)) -> RgbaImage {
    let width = size.0.min(source.width()).max(1);
    let height = size.1.min(source.height()).max(1);
    if width == source.width() && height == source.height() {
        return source.clone();
    }
    let sx = source.width() as f64 / width as f64;
    let sy = source.height() as f64 / height as f64;
    RgbaImage::from_fn(width, height, |x, y| {
        let left = x as f64 * sx;
        let right = (x + 1) as f64 * sx;
        let top = y as f64 * sy;
        let bottom = (y + 1) as f64 * sy;
        let mut total = [0.0_f64; 4];
        for iy in top.floor() as u32..(bottom.ceil() as u32).min(source.height()) {
            let dy = (bottom.min(iy as f64 + 1.0) - top.max(iy as f64)).max(0.0);
            for ix in left.floor() as u32..(right.ceil() as u32).min(source.width()) {
                let dx = (right.min(ix as f64 + 1.0) - left.max(ix as f64)).max(0.0);
                let rgba = source.get_pixel(ix, iy).0;
                let alpha_weight = rgba[3] as f64 * dx * dy;
                for channel in 0..3 {
                    total[channel] += rgba[channel] as f64 * alpha_weight;
                }
                total[3] += alpha_weight;
            }
        }
        let alpha = (total[3] / (sx * sy)).round().clamp(0.0, 255.0) as u8;
        if alpha == 0 || total[3] == 0.0 {
            return Rgba([0; 4]);
        }
        Rgba([
            (total[0] / total[3]).round().clamp(0.0, 255.0) as u8,
            (total[1] / total[3]).round().clamp(0.0, 255.0) as u8,
            (total[2] / total[3]).round().clamp(0.0, 255.0) as u8,
            alpha,
        ])
    })
}

fn trim_cache(
    images: &mut HashMap<String, TextureHandle>,
    image_used: &mut HashMap<String, u64>,
    budget: usize,
) {
    let mut bytes: usize = images.values().map(|t| t.size()[0] * t.size()[1] * 4).sum();
    while bytes > budget {
        let Some(oldest) = images
            .keys()
            .min_by_key(|key| image_used.get(*key).copied().unwrap_or(0))
            .cloned()
        else {
            break;
        };
        if let Some(texture) = images.remove(&oldest) {
            bytes = bytes.saturating_sub(texture.size()[0] * texture.size()[1] * 4);
        }
        image_used.remove(&oldest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strong_reduction_keeps_checker_and_thin_ink_coverage() {
        let checker = RgbaImage::from_fn(512, 512, |x, y| {
            let value = if (x + y) % 2 == 0 { 0 } else { 255 };
            Rgba([value, value, value, 255])
        });
        let reduced = area_reduce(&checker, (16, 16));
        assert!(reduced.pixels().all(|p| p.0 == [128, 128, 128, 255]));

        // Each destination pixel covers one very thin blue vertical stroke.
        // Sampling the original with bilinear alone could miss every stroke.
        let ink = RgbaImage::from_fn(512, 64, |x, _| {
            if x % 32 == 0 {
                Rgba([12, 48, 220, 255])
            } else {
                Rgba([0, 0, 0, 0])
            }
        });
        let reduced = area_reduce(&ink, (16, 2));
        assert!(reduced.pixels().all(|p| p.0 == [12, 48, 220, 8]));
    }

    #[test]
    fn transparent_edges_keep_ink_color_without_dark_or_hidden_color_fringe() {
        let source = RgbaImage::from_fn(4, 2, |x, _| {
            if x == 0 {
                Rgba([255, 96, 48, 255])
            } else {
                Rgba([0, 200, 255, 0])
            }
        });
        assert_eq!(
            area_reduce(&source, (1, 1)).get_pixel(0, 0).0,
            [255, 96, 48, 64]
        );
        let clear = RgbaImage::from_pixel(8, 8, Rgba([255, 0, 255, 0]));
        assert_eq!(area_reduce(&clear, (1, 1)).get_pixel(0, 0).0, [0; 4]);
    }

    #[test]
    fn physical_density_and_quantized_levels_are_bounded_by_the_source() {
        assert_eq!(level_size((1024, 512), vec2(30.0, 15.0), 1.0), (32, 16));
        assert_eq!(level_size((1024, 512), vec2(31.0, 15.5), 1.0), (32, 16));
        assert_eq!(level_size((1024, 512), vec2(30.0, 15.0), 2.0), (64, 32));
        assert_eq!(level_size((100, 50), vec2(500.0, 250.0), 2.0), (100, 50));
        assert_eq!(
            level_size((4096, 2048), vec2(4000.0, 2000.0), 2.0),
            (2048, 1024)
        );
        assert_eq!(level_size((1, 100), vec2(0.0, 0.0), 1.0), (1, 100));
    }

    #[test]
    fn area_filter_preserves_non_integer_coverage_and_never_upscales() {
        let source = RgbaImage::from_fn(7, 3, |x, _| {
            if x < 2 {
                Rgba([80, 120, 200, 255])
            } else {
                Rgba([0; 4])
            }
        });
        let reduced = area_reduce(&source, (3, 1));
        assert_eq!(reduced.get_pixel(0, 0).0, [80, 120, 200, 219]);
        assert_eq!(reduced.get_pixel(1, 0).0, [0; 4]);
        assert_eq!(area_reduce(&source, (100, 100)), source);
    }
}
