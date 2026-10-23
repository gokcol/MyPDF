//! Vector hatch fills shared by the canvas and saved PDF appearances.
use crate::annotations::Point2;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum FillPattern {
    #[default]
    Solid,
    Diagonal,
    Crosshatch,
    Horizontal,
    Dots,
}

impl FillPattern {
    pub const ALL: [Self; 5] = [
        Self::Solid,
        Self::Diagonal,
        Self::Crosshatch,
        Self::Horizontal,
        Self::Dots,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Solid => "Solid",
            Self::Diagonal => "Diagonal",
            Self::Crosshatch => "Crosshatch",
            Self::Horizontal => "Horizontal",
            Self::Dots => "Dots",
        }
    }
}

/// Return closed, filled polygons clipped to the shape. No texture resolution or
/// PDF-viewer-specific pattern implementation is involved. Triangular clipping
/// also handles concave polygons without drawing through their empty corners.
pub fn paths(outline: &[Point2], pattern: FillPattern, width: f32) -> Vec<Vec<Point2>> {
    if outline.len() < 3 || outline.iter().flatten().any(|v| !v.is_finite()) {
        return vec![];
    }
    if pattern == FillPattern::Solid {
        return vec![outline.to_vec()];
    }
    let bounds = crate::annotations::bounds(outline);
    if bounds[2] <= bounds[0] || bounds[3] <= bounds[1] {
        return vec![];
    }
    // Anchor the pattern to the object, so moving it cannot make its hatch jump.
    let local: Vec<_> = outline
        .iter()
        .map(|p| [p[0] - bounds[0], p[1] - bounds[1]])
        .collect();
    let triangles = crate::annotations::triangulate(&local);
    let pitch = (width * 3.0).max(6.0);
    let ink = (width * 0.35).clamp(0.45, 1.5);
    let mut result = match pattern {
        FillPattern::Solid => unreachable!(),
        FillPattern::Horizontal => stripes(&triangles, [0., 1.], pitch, ink),
        FillPattern::Diagonal => {
            stripes(&triangles, [std::f32::consts::FRAC_1_SQRT_2; 2], pitch, ink)
        }
        FillPattern::Crosshatch => {
            let mut paths = stripes(&triangles, [std::f32::consts::FRAC_1_SQRT_2; 2], pitch, ink);
            paths.extend(stripes(
                &triangles,
                [
                    std::f32::consts::FRAC_1_SQRT_2,
                    -std::f32::consts::FRAC_1_SQRT_2,
                ],
                pitch,
                ink,
            ));
            paths
        }
        FillPattern::Dots => dots(&triangles, pitch, (width * 0.45).clamp(0.6, 1.6)),
    };
    for path in &mut result {
        for p in path {
            p[0] += bounds[0];
            p[1] += bounds[1];
        }
    }
    result
}

fn stripes(triangles: &[[Point2; 3]], normal: Point2, pitch: f32, ink: f32) -> Vec<Vec<Point2>> {
    let projection = |p: Point2| p[0] * normal[0] + p[1] * normal[1];
    let ranges: Vec<_> = triangles
        .iter()
        .map(|t| {
            t.iter()
                .map(|p| projection(*p))
                .fold([f32::INFINITY, f32::NEG_INFINITY], |r, p| {
                    [r[0].min(p), r[1].max(p)]
                })
        })
        .collect();
    // Keep generated geometry bounded for very large or intricate polygons.
    let span: f32 = ranges.iter().map(|r| r[1] - r[0]).sum();
    let pitch = pitch.max(span / 5000.0);
    let mut result = vec![];
    for (triangle, range) in triangles.iter().zip(ranges) {
        let first = ((range[0] - ink) / pitch - 0.5).ceil() as i32;
        let last = ((range[1] + ink) / pitch - 0.5).floor() as i32;
        for i in first..=last {
            let center = (i as f32 + 0.5) * pitch;
            let clipped = clip(triangle, |p| projection(p) - center + ink / 2.0);
            let clipped = clip(&clipped, |p| center + ink / 2.0 - projection(p));
            if area(&clipped).abs() > 0.00001 {
                result.push(clipped);
            }
        }
    }
    result
}

fn dots(triangles: &[[Point2; 3]], mut pitch: f32, radius: f32) -> Vec<Vec<Point2>> {
    let bounds: Vec<_> = triangles
        .iter()
        .map(|t| crate::annotations::bounds(t))
        .collect();
    let range = |r: [f32; 4], pitch: f32| {
        [
            ((r[0] - radius) / pitch - 0.5).ceil() as i32,
            ((r[1] - radius) / pitch - 0.5).ceil() as i32,
            ((r[2] + radius) / pitch - 0.5).floor() as i32,
            ((r[3] + radius) / pitch - 0.5).floor() as i32,
        ]
    };
    for _ in 0..12 {
        let candidates: f32 = bounds
            .iter()
            .map(|r| {
                let r = range(*r, pitch);
                (r[2] - r[0] + 1).max(0) as f32 * (r[3] - r[1] + 1).max(0) as f32
            })
            .sum();
        if candidates <= 10000.0 {
            break;
        }
        pitch *= (candidates / 9000.0).sqrt().max(1.05);
    }
    let mut result = vec![];
    for (triangle, bounds) in triangles.iter().zip(bounds) {
        let r = range(bounds, pitch);
        let sign = cross(triangle[0], triangle[1], triangle[2]).signum();
        for y in r[1]..=r[3] {
            for x in r[0]..=r[2] {
                let center = [(x as f32 + 0.5) * pitch, (y as f32 + 0.5) * pitch];
                let mut circle: Vec<_> = (0..12)
                    .map(|i| {
                        let angle = i as f32 / 12.0 * std::f32::consts::TAU;
                        [
                            center[0] + angle.cos() * radius,
                            center[1] + angle.sin() * radius,
                        ]
                    })
                    .collect();
                for i in 0..3 {
                    circle = clip(&circle, |p| {
                        cross(triangle[i], triangle[(i + 1) % 3], p) * sign
                    });
                }
                if area(&circle).abs() > 0.00001 {
                    result.push(circle);
                }
            }
        }
    }
    result
}

fn cross(a: Point2, b: Point2, c: Point2) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn area(points: &[Point2]) -> f32 {
    if points.len() < 3 {
        return 0.;
    }
    points[1..]
        .windows(2)
        .map(|p| cross(points[0], p[0], p[1]))
        .sum::<f32>()
        / 2.0
}

/// Clip a convex subject to the positive half-plane of a linear distance.
fn clip(points: &[Point2], distance: impl Fn(Point2) -> f32) -> Vec<Point2> {
    if points.len() < 3 {
        return vec![];
    }
    let mut result = Vec::with_capacity(points.len() + 1);
    let mut previous = *points.last().unwrap();
    let mut previous_d = distance(previous);
    for &p in points {
        let d = distance(p);
        if (d >= 0.) != (previous_d >= 0.) {
            let t = previous_d / (previous_d - d);
            result.push([
                previous[0] + (p[0] - previous[0]) * t,
                previous[1] + (p[1] - previous[1]) * t,
            ]);
        }
        if d >= 0. {
            result.push(p);
        }
        previous = p;
        previous_d = d;
    }
    result.dedup_by(|a, b| (a[0] - b[0]).abs() < 0.00001 && (a[1] - b[1]).abs() < 0.00001);
    if result.len() > 1
        && (result[0][0] - result.last().unwrap()[0]).abs() < 0.00001
        && (result[0][1] - result.last().unwrap()[1]).abs() < 0.00001
    {
        result.pop();
    }
    result
}
