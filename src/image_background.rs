//! Local removal of a chosen, nearly uniform image background. Not AI segmentation.
use anyhow::{Result, ensure};
use image::ImageFormat;
use std::io::Cursor;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    pub color: [u8; 3],
    pub tolerance: u8,
    pub feather: u8,
    pub edges_only: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            color: [255; 3],
            tolerance: 24,
            feather: 12,
            edges_only: false,
        }
    }
}

pub fn remove(bytes: &[u8], options: Options) -> Result<Vec<u8>> {
    let mut image = crate::assets::decode_png(bytes)?.to_rgba8();
    let (w, h) = image.dimensions();
    let distance = |p: &image::Rgba<u8>| {
        (0..3)
            .map(|c| p[c].abs_diff(options.color[c]))
            .max()
            .unwrap()
    };
    let limit = options.tolerance.saturating_add(options.feather);
    let mut mask = vec![!options.edges_only; (w * h) as usize];
    if options.edges_only {
        let mut queue = std::collections::VecDeque::new();
        for y in 0..h {
            queue.push_back((0, y));
            queue.push_back((w - 1, y));
        }
        for x in 0..w {
            queue.push_back((x, 0));
            queue.push_back((x, h - 1));
        }
        while let Some((x, y)) = queue.pop_front() {
            let index = (y * w + x) as usize;
            if mask[index] {
                continue;
            }
            let p = image.get_pixel(x, y);
            if p[3] != 0 && distance(p) > limit {
                continue;
            }
            mask[index] = true;
            if x > 0 {
                queue.push_back((x - 1, y));
            }
            if y > 0 {
                queue.push_back((x, y - 1));
            }
            if x + 1 < w {
                queue.push_back((x + 1, y));
            }
            if y + 1 < h {
                queue.push_back((x, y + 1));
            }
        }
    }
    for (index, p) in image.pixels_mut().enumerate() {
        if !mask[index] || p[3] == 0 {
            continue;
        }
        let d = distance(p);
        if d <= options.tolerance {
            p[3] = 0;
        } else if d < limit && options.feather != 0 {
            let alpha = f32::from(d - options.tolerance) / f32::from(options.feather);
            // Remove the background contribution at soft edges to reduce light halos.
            for c in 0..3 {
                p[c] = ((f32::from(p[c]) - (1. - alpha) * f32::from(options.color[c])) / alpha)
                    .clamp(0., 255.) as u8;
            }
            p[3] = (f32::from(p[3]) * alpha).round() as u8;
        }
    }
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, ImageFormat::Png)?;
    let bytes = out.into_inner();
    ensure!(
        bytes.len() <= crate::assets::MAX_ASSET_BYTES,
        "Transparent image exceeds the artwork size limit"
    );
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removes_paper_keeps_ink_and_can_preserve_enclosed_white_regions() {
        let mut img = image::RgbaImage::from_pixel(9, 9, image::Rgba([255, 255, 255, 255]));
        for x in 2..7 {
            img.put_pixel(x, 2, image::Rgba([0, 0, 0, 255]));
            img.put_pixel(x, 6, image::Rgba([0, 0, 0, 255]));
        }
        for y in 2..7 {
            img.put_pixel(2, y, image::Rgba([0, 0, 0, 255]));
            img.put_pixel(6, y, image::Rgba([0, 0, 0, 255]));
        }
        img.put_pixel(0, 0, image::Rgba([10, 20, 30, 0]));
        let mut png = Cursor::new(vec![]);
        img.write_to(&mut png, ImageFormat::Png).unwrap();
        for edges_only in [false, true] {
            let bytes = remove(
                png.get_ref(),
                Options {
                    edges_only,
                    ..Default::default()
                },
            )
            .unwrap();
            let out = crate::assets::decode_png(&bytes).unwrap().to_rgba8();
            assert_eq!(out.get_pixel(0, 0)[3], 0);
            assert_eq!(out.get_pixel(8, 8)[3], 0);
            assert_eq!(out.get_pixel(2, 2)[3], 255);
            assert_eq!(out.get_pixel(4, 4)[3], if edges_only { 255 } else { 0 });
        }
    }
}
