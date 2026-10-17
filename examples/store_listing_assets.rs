//! Generate the optional Microsoft Store listing logos from MyPDF's own icon.
//! Usage: cargo run --release --example store_listing_assets -- <output folder>
#[path = "../src/desktop/icon.rs"]
mod icon;

fn main() -> anyhow::Result<()> {
    let out = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("Pass an output folder"))?,
    );
    std::fs::create_dir_all(&out)?;
    for size in [300, 150, 71] {
        image::save_buffer(
            out.join(format!("MyPDF-logo-{size}x{size}.png")),
            &icon::rgba(size),
            size,
            size,
            image::ColorType::Rgba8,
        )?;
    }
    Ok(())
}
