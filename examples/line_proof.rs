//! Disposable vector proof for line, arrow and double-arrow route families.
use anyhow::{Context, Result};
use mypdf_milestone0::{
    annotations::{self, Annotation, Kind, Route, StrokeStyle, Style},
    convert,
};

fn text(page: &mut Vec<Annotation>, rect: [f32; 4], value: &str) {
    page.push(Annotation::new(
        0,
        rect,
        Kind::Text { text: value.into() },
        Style {
            font_size: 12.,
            color: [0.14, 0.17, 0.22],
            ..Default::default()
        },
    ));
}
fn main() -> Result<()> {
    let path = std::env::args().nth(1).context("Pass an output PDF path")?;
    let source = convert::blank(660., 660.)?;
    let mut annotations = vec![];
    for (column, label) in ["Line", "Arrow", "Double arrow"].into_iter().enumerate() {
        let x = 170. + column as f32 * 160.;
        text(&mut annotations, [x, 22., x + 140., 48.], label);
    }
    let start = [0., 0.];
    let end = [1., 1.];
    for (row, (label, route)) in [
        ("Straight", None),
        ("Stepped", Some(Route::Stepped)),
        ("S-curved", Some(Route::Curved)),
        ("Elbow", Some(Route::Elbow)),
    ]
    .into_iter()
    .enumerate()
    {
        let y = 70. + row as f32 * 90.;
        text(&mut annotations, [20., y + 5., 145., y + 32.], label);
        for column in 0..3 {
            let x = 170. + column as f32 * 160.;
            let kind = match (route, column) {
                (None, 0) => Kind::Line {
                    start: [0., 0.5],
                    end: [1., 0.5],
                    arrow: false,
                },
                (None, 1) => Kind::Line {
                    start: [0., 0.5],
                    end: [1., 0.5],
                    arrow: true,
                },
                (None, _) => Kind::DoubleArrow {
                    start: [0., 0.5],
                    end: [1., 0.5],
                },
                (Some(route), 0) => Kind::RoutedLine { start, end, route },
                (Some(route), 1) => Kind::RoutedArrow { start, end, route },
                (Some(route), _) => Kind::RoutedDoubleArrow { start, end, route },
            };
            annotations.push(Annotation::new(
                0,
                [x, y, x + 130., y + 42.],
                kind,
                Style {
                    color: [0.12, 0.38, 0.70],
                    width: 2.,
                    ..Default::default()
                },
            ));
        }
    }
    for (row, (label, stroke, double)) in [
        ("Wavy", StrokeStyle::Wave, false),
        ("Dashed", StrokeStyle::Dashed, false),
        ("Dotted", StrokeStyle::Dotted, false),
        ("Dash-dot", StrokeStyle::DashDot, false),
        ("Parallel", StrokeStyle::Solid, true),
    ]
    .into_iter()
    .enumerate()
    {
        let y = 440. + row as f32 * 40.;
        text(&mut annotations, [20., y - 2., 145., y + 23.], label);
        let (start, end) = ([0., 0.5], [1., 0.5]);
        annotations.push(Annotation::new(
            0,
            [170., y, 620., y + 15.],
            if double {
                Kind::DoubleLine { start, end }
            } else {
                Kind::Line {
                    start,
                    end,
                    arrow: false,
                }
            },
            Style {
                color: [0.12, 0.38, 0.70],
                width: 2.,
                stroke,
                ..Default::default()
            },
        ));
    }
    std::fs::write(path, annotations::export(&source, &annotations)?)?;
    Ok(())
}
