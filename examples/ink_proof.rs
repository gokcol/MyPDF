//! Fictional native review fixture for pen, eraser and object selection.
use mypdf_milestone0::{
    annotations::{self, Annotation, Kind, Style},
    content, convert,
};

fn main() -> anyhow::Result<()> {
    let root = std::path::PathBuf::from(std::env::args().nth(1).expect("output directory"));
    std::fs::create_dir_all(&root)?;
    let source = content::add_text(
        &convert::blank(595., 842.)?,
        1,
        "Pen and eraser review",
        [50., 45., 490., 50.],
        22.,
        [0.1, 0.15, 0.2],
    )?;
    let stroke = (0..=120)
        .map(|i| {
            let t = i as f32 / 120.;
            [t, 0.5 + 0.38 * (t * std::f32::consts::TAU * 2.).sin()]
        })
        .collect();
    let marks = [
        Annotation::new(
            0,
            [50., 130., 510., 210.],
            Kind::Ink {
                strokes: vec![stroke],
            },
            Style {
                color: [0.1, 0.45, 0.65],
                width: 3.,
                ..Default::default()
            },
        ),
        Annotation::new(
            0,
            [50., 280., 300., 390.],
            Kind::Rectangle,
            Style {
                color: [0.15, 0.45, 0.35],
                fill: Some([0.85, 0.95, 0.9]),
                ..Default::default()
            },
        ),
    ];
    std::fs::write(root.join("ink.pdf"), annotations::export(&source, &marks)?)?;
    Ok(())
}
