//! Disposable visual proof for the compact rectangle and ellipse tool presets.
use anyhow::{Context, Result};
use mypdf_milestone0::{
    annotations::{self, Annotation, FillPattern, Kind, Shape, StrokeStyle, Style},
    convert,
};

fn main() -> Result<()> {
    let path = std::env::args().nth(1).context("Pass an output PDF path")?;
    let source = convert::blank(660., 650.)?;
    let mut annotations = vec![];
    let kinds = [
        ("Rectangle", Kind::Rectangle),
        (
            "Square",
            Kind::Shape {
                shape: Shape::Square,
            },
        ),
        (
            "Diamond",
            Kind::Shape {
                shape: Shape::Diamond,
            },
        ),
        (
            "Triangle",
            Kind::Shape {
                shape: Shape::Triangle,
            },
        ),
        (
            "Pentagon",
            Kind::Shape {
                shape: Shape::Pentagon,
            },
        ),
        (
            "Hexagon",
            Kind::Shape {
                shape: Shape::Hexagon,
            },
        ),
        ("Ellipse", Kind::Ellipse),
        (
            "Circle",
            Kind::Shape {
                shape: Shape::Circle,
            },
        ),
        (
            "Semicircle",
            Kind::Shape {
                shape: Shape::Semicircle,
            },
        ),
    ];
    for (i, (label, kind)) in kinds.into_iter().enumerate() {
        let x = 40. + (i % 3) as f32 * 210.;
        let y = 35. + (i / 3) as f32 * 210.;
        let (width, height) = match &kind {
            Kind::Rectangle | Kind::Ellipse => (150., 105.),
            Kind::Shape {
                shape: Shape::Semicircle,
            } => (150., 75.),
            _ => (125., 125.),
        };
        annotations.push(Annotation::new(
            0,
            [x, y, x + width, y + height],
            kind,
            Style {
                color: [0.12, 0.38, 0.70],
                fill: Some(if i % 2 == 0 {
                    [0.85, 0.92, 1.]
                } else {
                    [0.4, 0.65, 0.8]
                }),
                fill_pattern: if i % 2 == 0 {
                    FillPattern::Solid
                } else {
                    FillPattern::Diagonal
                },
                width: 2.,
                stroke: if i == 4 {
                    StrokeStyle::Dashed
                } else {
                    StrokeStyle::Solid
                },
                ..Default::default()
            },
        ));
        annotations.push(Annotation::new(
            0,
            [x, y + 145., x + 170., y + 170.],
            Kind::Text { text: label.into() },
            Style {
                color: [0.15, 0.18, 0.22],
                font_size: 14.,
                ..Default::default()
            },
        ));
    }
    std::fs::write(path, annotations::export(&source, &annotations)?)?;
    Ok(())
}
