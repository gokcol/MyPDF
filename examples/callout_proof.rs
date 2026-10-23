//! Disposable visual proof for native callouts and routed arrows.
use anyhow::{Context, Result};
use mypdf_milestone0::{
    annotations::{self, Annotation, FillPattern, Kind, Route, StrokeStyle, Style},
    convert,
};

fn main() -> Result<()> {
    let path = std::env::args().nth(1).context("Pass an output PDF path")?;
    let source = convert::blank(640., 460.)?;
    let mut annotations = vec![];
    for (i, (text, pattern)) in [
        ("Review: Orhan Gökçöl", FillPattern::Solid),
        ("Check this detail", FillPattern::Diagonal),
    ]
    .into_iter()
    .enumerate()
    {
        let y = 40. + i as f32 * 160.;
        let mut a = Annotation::new(
            0,
            [30., y, 310., y + 130.],
            Kind::callout(text),
            Style {
                color: [0.12, 0.38, 0.70],
                fill: Some([0.90, 0.95, 1.]),
                fill_pattern: pattern,
                font_size: 14.,
                width: 1.5,
                stroke: if i == 0 {
                    StrokeStyle::Solid
                } else {
                    StrokeStyle::Dashed
                },
                ..Default::default()
            },
        );
        if let Kind::Callout { text_color, .. } = &mut a.kind {
            *text_color = [0.14, 0.17, 0.22];
        }
        annotations.push(a);
    }
    for (i, route) in [Route::Elbow, Route::Curved].into_iter().enumerate() {
        let y = 70. + i as f32 * 170.;
        annotations.push(Annotation::new(
            0,
            [350., y, 590., y + 110.],
            Kind::RoutedArrow {
                start: [0., 0.],
                end: [1., 1.],
                route,
            },
            Style {
                color: [0.85, 0.25, 0.18],
                width: 3.,
                ..Default::default()
            },
        ));
    }
    std::fs::write(path, annotations::export(&source, &annotations)?)?;
    Ok(())
}
