//! Generate MSIX artwork from the same native icon used by the desktop app.
#[path = "../src/desktop/icon.rs"]
#[allow(dead_code)] // This exporter always uses the master, not runtime caches.
mod icon;

use anyhow::{Context, Result};
use std::path::PathBuf;

fn main() -> Result<()> {
    let out = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .context("Pass an output folder")?,
    );
    std::fs::create_dir_all(&out)?;
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
            image::save_buffer(
                out.join(file),
                &icon::render_master(size),
                size,
                size,
                image::ColorType::Rgba8,
            )?;
        }
    }
    for size in [16, 24, 32, 44, 48, 256] {
        image::save_buffer(
            out.join(format!(
                "Square44x44Logo.targetsize-{size}_altform-unplated.png"
            )),
            &icon::render_master(size),
            size,
            size,
            image::ColorType::Rgba8,
        )?;
    }
    Ok(())
}
