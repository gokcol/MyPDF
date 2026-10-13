//! Stroke geometry shared by the interactive canvas and saved PDF appearances.
use crate::annotations::{Point2, Primitive};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum StrokeStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
    DashDot,
    Wave,
}
impl StrokeStyle {
    pub const ALL: [Self; 5] = [
        Self::Solid,
        Self::Dashed,
        Self::Dotted,
        Self::DashDot,
        Self::Wave,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Solid => "Solid",
            Self::Dashed => "Dashed",
            Self::Dotted => "Dotted",
            Self::DashDot => "Dash-dot",
            Self::Wave => "Wave",
        }
    }
    /// Standard PDF fallback for readers that regenerate an annotation's appearance.
    pub fn dash_array(self, width: f32) -> Option<Vec<f32>> {
        let u = width.max(0.6);
        match self {
            Self::Dashed => Some(vec![4. * u, 2.5 * u]),
            Self::Dotted => Some(vec![0., 3. * u]),
            Self::DashDot => Some(vec![4. * u, 2. * u, 0., 2. * u]),
            _ => None,
        }
    }
}

/// Continue the pattern across vertices instead of restarting at each segment.
/// Geometry is expressed in page points, so zoom and export have identical spacing.
pub fn path(points: Vec<Point2>, closed: bool, width: f32, style: StrokeStyle) -> Vec<Primitive> {
    if style == StrokeStyle::Solid || points.len() < 2 {
        return vec![Primitive::Path {
            points,
            closed,
            fill: false,
        }];
    }
    let mut p = Vec::with_capacity(points.len() + usize::from(closed));
    for point in points {
        if p.last()
            .is_none_or(|last: &Point2| distance(*last, point) > 0.0001)
        {
            p.push(point);
        }
    }
    if closed && p.len() > 1 && distance(p[0], *p.last().unwrap()) > 0.0001 {
        p.push(p[0]);
    }
    if p.len() < 2 {
        return vec![];
    }
    let mut lengths = vec![0.];
    for pair in p.windows(2) {
        lengths.push(lengths.last().unwrap() + distance(pair[0], pair[1]));
    }
    let total = *lengths.last().unwrap();
    if !total.is_finite() || total < 0.0001 {
        return vec![];
    }
    let sample = |d: f32| {
        let i = lengths
            .partition_point(|v| *v <= d)
            .saturating_sub(1)
            .min(p.len() - 2);
        let t = ((d - lengths[i]) / (lengths[i + 1] - lengths[i])).clamp(0., 1.);
        (
            i,
            t,
            [
                p[i][0] + (p[i + 1][0] - p[i][0]) * t,
                p[i][1] + (p[i + 1][1] - p[i][1]) * t,
            ],
        )
    };
    // Bound generated geometry even for unusually long paths with very thin strokes.
    let u = width.max(0.6).max(total / 12_000.);
    let mut result = vec![];
    if style == StrokeStyle::Wave {
        let normal = |a: Point2, b: Point2| {
            let len = distance(a, b);
            [-(b[1] - a[1]) / len, (b[0] - a[0]) / len]
        };
        let normals: Vec<_> = (0..p.len())
            .map(|i| {
                let before = if i == 0 {
                    if closed { p[p.len() - 2] } else { p[0] }
                } else {
                    p[i - 1]
                };
                let after = if i == p.len() - 1 {
                    if closed { p[1] } else { p[i] }
                } else {
                    p[i + 1]
                };
                if distance(before, p[i]) < 0.0001 {
                    return normal(p[i], after);
                }
                if distance(p[i], after) < 0.0001 {
                    return normal(before, p[i]);
                }
                let a = normal(before, p[i]);
                let b = normal(p[i], after);
                let len = (a[0] + b[0]).hypot(a[1] + b[1]);
                if len < 0.0001 {
                    b
                } else {
                    [(a[0] + b[0]) / len, (a[1] + b[1]) / len]
                }
            })
            .collect();
        // Whole waves meet cleanly at the endpoints and at a closed path's seam.
        let cycles = (total / (8. * u)).round().max(1.);
        let steps = (cycles as usize * 16).clamp(16, 24_000);
        let mut distances = lengths.clone();
        distances.extend((0..=steps).map(|i| total * i as f32 / steps as f32));
        distances.sort_unstable_by(f32::total_cmp);
        distances.dedup_by(|a, b| (*a - *b).abs() < 0.0001);
        let points = distances
            .into_iter()
            .map(|d| {
                let (i, t, q) = sample(d);
                let offset =
                    (d / total * cycles * std::f32::consts::TAU).sin() * width.max(0.6) * 1.2;
                [
                    q[0] + (normals[i][0] * (1. - t) + normals[i + 1][0] * t) * offset,
                    q[1] + (normals[i][1] * (1. - t) + normals[i + 1][1] * t) * offset,
                ]
            })
            .collect();
        result.push(Primitive::Path {
            points,
            closed,
            fill: false,
        });
        return result;
    }
    let cycle = match style {
        StrokeStyle::Dashed => 6.5 * u,
        StrokeStyle::Dotted => 3. * u,
        _ => 8. * u,
    };
    let mut start = 0.;
    while start < total {
        if style == StrokeStyle::Dotted {
            result.push(Primitive::Dot {
                center: sample(start).2,
            });
        } else {
            let end = (start + 4. * u).min(total);
            let mut points = vec![sample(start).2];
            let first = lengths.partition_point(|v| *v <= start);
            let last = lengths.partition_point(|v| *v < end);
            points.extend_from_slice(&p[first..last]);
            points.push(sample(end).2);
            result.push(Primitive::Path {
                points,
                closed: false,
                fill: false,
            });
            if style == StrokeStyle::DashDot && start + 6. * u < total {
                result.push(Primitive::Dot {
                    center: sample(start + 6. * u).2,
                });
            }
        }
        start += cycle;
    }
    result
}
fn distance(a: Point2, b: Point2) -> f32 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

/// Join silhouettes built from adjacent polygons (for example, the library's rings)
/// before stroking. Shared construction edges must never appear as drawing borders.
pub fn outlines(polygons: &[Vec<Point2>]) -> Vec<Vec<Point2>> {
    use std::collections::BTreeMap;
    let key = |p: Point2| {
        [
            (p[0] * 100_000.).round() as i64,
            (p[1] * 100_000.).round() as i64,
        ]
    };
    let mut edges = BTreeMap::new();
    for polygon in polygons {
        for i in 0..polygon.len() {
            let (a, b) = (polygon[i], polygon[(i + 1) % polygon.len()]);
            let (ka, kb) = (key(a), key(b));
            if ka == kb {
                continue;
            }
            let edge = if ka < kb { (ka, kb) } else { (kb, ka) };
            if edges.remove(&edge).is_none() {
                edges.insert(edge, (a, b));
            }
        }
    }
    let mut neighbors: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for &(a, b) in edges.values() {
        neighbors.entry(key(a)).or_default().push(b);
        neighbors.entry(key(b)).or_default().push(a);
    }
    let mut paths = vec![];
    while let Some((_, (a, b))) = edges.pop_first() {
        let mut path = vec![a, b];
        let mut current = b;
        while key(current) != key(a) {
            let mut next = None;
            if let Some(options) = neighbors.get_mut(&key(current)) {
                while let Some(p) = options.pop() {
                    let (ka, kb) = (key(current), key(p));
                    let edge = if ka < kb { (ka, kb) } else { (kb, ka) };
                    if edges.remove(&edge).is_some() {
                        next = Some(p);
                        break;
                    }
                }
            }
            let Some(p) = next else {
                break;
            };
            path.push(p);
            current = p;
        }
        paths.push(path);
    }
    paths
}
