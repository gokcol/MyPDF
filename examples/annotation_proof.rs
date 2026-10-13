use mypdf_milestone0::{
    annotations::{self as ann, Annotation, Kind, Style, Symbol},
    document, write_new,
};
fn main() -> anyhow::Result<()> {
    let out = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "artifacts/milestone3-proof".into()),
    );
    std::fs::create_dir_all(&out)?;
    let source = document::fixture(&document::default_font()?)?;
    let mut a = vec![
        Annotation::new(
            0,
            [72.0, 270.0, 530.0, 332.0],
            Kind::Text {
                text: "İstanbul: sözleşme gözden geçirildi. Native Rust annotation.".into(),
            },
            Style::default(),
        ),
        Annotation::new(
            0,
            [420.0, 390.0, 490.0, 440.0],
            Kind::Symbol(Symbol::Check),
            Style::default(),
        ),
        Annotation::new(
            0,
            [180.0, 390.0, 370.0, 445.0],
            Kind::Stamp {
                text: "APPROVED".into(),
            },
            Style {
                font_size: 20.0,
                ..Default::default()
            },
        ),
        Annotation::new(
            1,
            [310.0, 380.0, 472.0, 458.0],
            Kind::Text {
                text: "Rotated page\nİstanbul, Türkçe.".into(),
            },
            Style::default(),
        ),
    ];
    a[2].rotation = -8.0;
    let output = ann::export(&source, &a)?;
    write_new(&out.join("annotations.pdf"), &output)?;
    document::render(&output, &out.join("hayro"), 1.0)?;
    Ok(())
}
