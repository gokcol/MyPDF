//! Fictional visual proof of editable, vector annotation appearances.
use mypdf_milestone0::{
    annotations::{self, Annotation, FillPattern, Kind, Markup, Style},
    content, convert,
};

fn main() -> anyhow::Result<()> {
    let root = std::path::PathBuf::from(std::env::args().nth(1).expect("output directory"));
    std::fs::create_dir_all(&root)?;
    let mut source = convert::blank(595., 842.)?;
    for (text, y, size) in [
        ("Annotation review", 45., 24.),
        ("Select this sentence to review or bookmark.", 100., 16.),
        ("Continuous squiggly underline", 142., 16.),
    ] {
        source = content::add_text(
            &source,
            1,
            text,
            [50., y, 500., 34.],
            size,
            [0.1, 0.15, 0.2],
        )?;
    }
    let mut marks = vec![Annotation::new(
        0,
        [50., 145., 300., 164.],
        Kind::Markup {
            kind: Markup::Squiggly,
            quads: vec![[0., 0., 1., 1.]],
        },
        Style {
            color: [0.8, 0.15, 0.25],
            width: 1.2,
            ..Default::default()
        },
    )];
    for (i, pattern) in FillPattern::ALL.into_iter().enumerate() {
        let x = 50. + (i % 2) as f32 * 260.;
        let y = 220. + (i / 2) as f32 * 145.;
        source = content::add_text(
            &source,
            1,
            pattern.label(),
            [x, y - 30., 220., 25.],
            13.,
            [0.15, 0.2, 0.25],
        )?;
        marks.push(Annotation::new(
            0,
            [x, y, x + 210., y + 90.],
            if i % 2 == 0 {
                Kind::Rectangle
            } else {
                Kind::Ellipse
            },
            Style {
                color: [0.1, 0.35, 0.55],
                fill: Some([0.2, 0.55, 0.65]),
                fill_pattern: pattern,
                opacity: 0.85,
                width: 1.5,
                ..Default::default()
            },
        ));
    }
    std::fs::write(
        root.join("annotations.pdf"),
        annotations::export(&source, &marks)?,
    )?;
    std::fs::write(root.join("selection.pdf"), source)?;
    Ok(())
}
