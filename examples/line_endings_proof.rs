//! Disposable visual proof for editable line and arrow endpoint styles.
use anyhow::{Context, Result};
use mypdf_milestone0::{
    annotations::{self, Annotation, Kind, LineEnd, LineEndings, Route, StrokeStyle, Style},
    convert,
};

fn label(entries: &mut Vec<Annotation>, text: &str, mut rect: [f32; 4], size: f32) {
    rect[3] = rect[3].max(rect[1] + size * 2. + 8.);
    entries.push(Annotation::new(
        0,
        rect,
        Kind::Text { text: text.into() },
        Style {
            color: [0.15, 0.18, 0.22],
            font_size: size,
            ..Default::default()
        },
    ));
}

fn main() -> Result<()> {
    let path = std::env::args().nth(1).context("Pass an output PDF path")?;
    let source = convert::blank(880., 760.)?;
    let mut entries = vec![];
    label(
        &mut entries,
        "Line and arrow endings",
        [30., 20., 800., 50.],
        22.,
    );
    label(
        &mut entries,
        "Start and end styles · 18 pt",
        [30., 57., 400., 80.],
        12.,
    );
    label(
        &mut entries,
        "Independent endings on every route",
        [475., 57., 850., 80.],
        12.,
    );
    for (i, (name, ending)) in [
        ("None", LineEnd::None),
        ("Open arrow", LineEnd::OpenArrow),
        ("Closed arrow", LineEnd::ClosedArrow),
        ("Filled arrow", LineEnd::FilledArrow),
        ("Circle", LineEnd::Circle),
        ("Square", LineEnd::Square),
        ("Diamond", LineEnd::Diamond),
        ("Bar", LineEnd::Bar),
        ("Slash", LineEnd::Slash),
    ]
    .into_iter()
    .enumerate()
    {
        let y = 100. + i as f32 * 70.;
        label(&mut entries, name, [30., y - 8., 140., y + 17.], 13.);
        entries.push(Annotation::new(
            0,
            [165., y, 405., y + 1.],
            Kind::Line {
                start: [0., 0.5],
                end: [1., 0.5],
                arrow: false,
            },
            Style {
                color: [0.1, 0.35, 0.75],
                fill: Some([0.9, 0.15, 0.1]),
                width: 2.,
                line_endings: Some(LineEndings {
                    start: ending,
                    end: ending,
                    size: 18.,
                }),
                ..Default::default()
            },
        ));
    }
    let mut kinds = vec![Kind::DoubleLine {
        start: [0., 0.5],
        end: [1., 0.5],
    }];
    for route in [Route::Elbow, Route::Stepped, Route::Curved] {
        kinds.push(Kind::RoutedArrow {
            start: [0., 0.],
            end: [1., 1.],
            route,
        });
    }
    kinds.push(Kind::DoubleArrow {
        start: [0., 0.2],
        end: [1., 0.8],
    });
    for (i, kind) in kinds.into_iter().enumerate() {
        let y = 110. + i as f32 * 120.;
        label(&mut entries, kind.label(), [480., y - 22., 830., y], 13.);
        entries.push(Annotation::new(
            0,
            [500., y + 10., 810., y + 75.],
            kind,
            Style {
                color: [0.1, 0.5, 0.35],
                fill: Some([0.9, 0.15, 0.1]),
                width: 2.,
                stroke: if i == 3 {
                    StrokeStyle::Dashed
                } else {
                    StrokeStyle::Solid
                },
                line_endings: Some(LineEndings {
                    start: if i % 2 == 0 {
                        LineEnd::Diamond
                    } else {
                        LineEnd::Circle
                    },
                    end: LineEnd::FilledArrow,
                    size: 18.,
                }),
                ..Default::default()
            },
        ));
    }
    std::fs::write(path, annotations::export(&source, &entries)?)?;
    Ok(())
}
