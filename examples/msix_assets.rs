//! Generate MSIX artwork from the same native icon used by the desktop app.
#[path = "../src/desktop/icon.rs"]
mod icon;

use anyhow::{Context, Result};
use image::{RgbaImage, imageops::FilterType};
use std::path::PathBuf;

fn main() -> Result<()> {
    let out = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .context("Pass an output folder")?,
    );
    std::fs::create_dir_all(&out)?;
    let master =
        RgbaImage::from_raw(1024, 1024, icon::rgba(1024)).context("Invalid icon pixels")?;
    for (name, base) in [
        ("StoreLogo", 50),
        ("Square44x44Logo", 44),
        ("Square150x150Logo", 150),
    ] {
        for scale in [100, 125, 150, 200, 400] {
            let size = (base * scale + 50) / 100;
            let file = if scale == 100 {
                format!("{name}.png")
            } else {
                format!("{name}.scale-{scale}.png")
            };
            image::imageops::resize(&master, size, size, FilterType::Lanczos3)
                .save(out.join(file))?;
        }
    }
    for size in [16, 24, 32, 44, 48, 256] {
        image::imageops::resize(&master, size, size, FilterType::Lanczos3).save(out.join(
            format!("Square44x44Logo.targetsize-{size}_altform-unplated.png"),
        ))?;
    }
    Ok(())
}
