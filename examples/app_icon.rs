#[path = "../src/desktop/icon.rs"]
mod icon;
fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "dist/MyPDF.png".into());
    image::save_buffer(path, &icon::rgba(1024), 1024, 1024, image::ColorType::Rgba8)?;
    Ok(())
}
