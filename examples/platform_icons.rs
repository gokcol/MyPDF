//! Export the checked-in Windows and iPad icons from the shared artwork.
//! Usage: cargo run --release --example platform_icons -- [repository root]
//! Store listing logos: cargo run --release --example store_listing_assets --
//! packaging/windows/store-listing/assets
#[path = "../src/desktop/icon.rs"]
mod icon;

use anyhow::{Context, Result};
use image::{ImageEncoder, Rgb, RgbImage};
use std::path::{Path, PathBuf};

fn windows_icon(path: &Path) -> Result<()> {
    // PNG-backed ICO frames preserve the master's transparency and are supported
    // by every Windows version targeted by MyPDF. A zero dimension encodes 256.
    const SIZES: [u32; 8] = [16, 20, 24, 32, 48, 64, 128, 256];
    let frames: Vec<Vec<u8>> = SIZES
        .into_iter()
        .map(|size| {
            let mut png = Vec::new();
            image::codecs::png::PngEncoder::new(&mut png).write_image(
                &icon::render_master(size),
                size,
                size,
                image::ExtendedColorType::Rgba8,
            )?;
            Ok(png)
        })
        .collect::<Result<_>>()?;

    let mut ico = Vec::new();
    ico.extend_from_slice(&0_u16.to_le_bytes()); // Reserved.
    ico.extend_from_slice(&1_u16.to_le_bytes()); // Icon, not cursor.
    ico.extend_from_slice(&(SIZES.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * SIZES.len() as u32;
    for (size, png) in SIZES.into_iter().zip(&frames) {
        ico.extend_from_slice(&[size as u8, size as u8, 0, 0]);
        ico.extend_from_slice(&1_u16.to_le_bytes()); // Color planes.
        ico.extend_from_slice(&32_u16.to_le_bytes()); // RGBA bit depth.
        ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for png in frames {
        ico.extend_from_slice(&png);
    }
    std::fs::write(path, ico).with_context(|| format!("Writing {}", path.display()))
}

fn ipad_icon(path: &Path) -> Result<()> {
    // iPad app icons must have no alpha channel. Flatten the unchanged artwork
    // onto white, retaining the same scale and composition as the desktop icon.
    const SIZE: u32 = 1024;
    let rgba = icon::rgba(SIZE);
    let rgb = RgbImage::from_fn(SIZE, SIZE, |x, y| {
        let offset = ((y * SIZE + x) * 4) as usize;
        let alpha = u32::from(rgba[offset + 3]);
        Rgb(std::array::from_fn(|channel| {
            let foreground = u32::from(rgba[offset + channel]);
            ((foreground * alpha + 255 * (255 - alpha) + 127) / 255) as u8
        }))
    });
    rgb.save(path)
        .with_context(|| format!("Writing {}", path.display()))
}

fn main() -> Result<()> {
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    for size in [128, 256] {
        image::save_buffer(
            root.join(format!("packaging/branding/MyPDF-calico-{size}.png")),
            &icon::render_master(size),
            size,
            size,
            image::ColorType::Rgba8,
        )?;
    }
    windows_icon(&root.join("packaging/windows/MyPDF.ico"))?;
    ipad_icon(&root.join("ipad/MyPDFiPad/Assets.xcassets/AppIcon.appiconset/AppIcon.png"))?;
    Ok(())
}
