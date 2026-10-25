//! Smooth filled annotation outlines without changing their PDF geometry.
use eframe::egui::{self, Color32, Pos2, Stroke};
use mypdf_milestone0::annotations;

/// Use egui's antialiased boundary for convex shapes. Its filled-path
/// tessellator assumes convexity, so concave artwork keeps its triangulation.
pub(super) fn paint_fill(painter: &egui::Painter, points: &[Pos2], ink: Color32) {
    if let Some(shape) = fill_shape(points, ink) {
        painter.add(shape);
    }
}

fn fill_shape(points: &[Pos2], ink: Color32) -> Option<egui::Shape> {
    if ink == Color32::TRANSPARENT || points.iter().any(|point| !point.is_finite()) {
        return None;
    }
    // Closed annotation paths sometimes repeat their first vertex. Avoid
    // zero-length edges in egui's antialiasing normal calculation as well.
    let mut points = points.to_vec();
    points.dedup();
    if points.first() == points.last() {
        points.pop();
    }
    if points.len() < 3 {
        return None;
    }
    if is_convex(&points) {
        return Some(egui::Shape::convex_polygon(points, ink, Stroke::NONE));
    }

    let polygon: Vec<_> = points.iter().map(|point| [point.x, point.y]).collect();
    let mut mesh = egui::Mesh::default();
    for triangle in annotations::triangulate(&polygon) {
        let start = mesh.vertices.len() as u32;
        for [x, y] in triangle {
            mesh.colored_vertex(egui::pos2(x, y), ink);
        }
        mesh.add_triangle(start, start + 1, start + 2);
    }
    (!mesh.indices.is_empty()).then(|| egui::Shape::mesh(mesh))
}

fn is_convex(points: &[Pos2]) -> bool {
    let mut winding = 0.;
    let mut total_turn = 0.;
    for index in 0..points.len() {
        let a = points[index];
        let b = points[(index + 1) % points.len()];
        let c = points[(index + 2) % points.len()];
        let first = [
            f64::from(b.x) - f64::from(a.x),
            f64::from(b.y) - f64::from(a.y),
        ];
        let second = [
            f64::from(c.x) - f64::from(b.x),
            f64::from(c.y) - f64::from(b.y),
        ];
        let cross = first[0] * second[1] - first[1] * second[0];
        let dot = first[0] * second[0] + first[1] * second[1];
        if cross != 0. {
            if winding != 0. && cross.signum() != winding {
                return false;
            }
            winding = cross.signum();
        } else if dot < 0. {
            // A collinear reversal is a folded boundary, not a convex edge.
            return false;
        }
        total_turn += cross.atan2(dot);
    }
    // Equal turn signs alone also describe self-intersecting star polygons.
    // A simple convex boundary makes exactly one turn, in either direction.
    winding != 0. && (total_turn.abs() - std::f64::consts::TAU).abs() < 0.00001
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Rect, pos2, vec2};

    fn tessellate(points: &[Pos2], ink: Color32, pixels_per_point: f32) -> Vec<egui::Mesh> {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(pixels_per_point);
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(120., 120.))),
                ..Default::default()
            },
            |ctx| {
                paint_fill(&ctx.layer_painter(egui::LayerId::background()), points, ink);
            },
        );
        ctx.tessellate(output.shapes, output.pixels_per_point)
            .into_iter()
            .filter_map(|primitive| match primitive.primitive {
                egui::epaint::Primitive::Mesh(mesh) => Some(mesh),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn convex_fills_have_a_transparent_boundary_at_each_display_scale() {
        for pixels_per_point in [1., 2.] {
            for mut points in [
                vec![pos2(20., 20.), pos2(85., 28.), pos2(60., 85.)],
                vec![
                    pos2(50., 20.),
                    pos2(85., 50.),
                    pos2(50., 85.),
                    pos2(20., 50.),
                ],
            ] {
                for _ in 0..2 {
                    assert!(is_convex(&points));
                    let meshes = tessellate(&points, Color32::RED, pixels_per_point);
                    assert!(
                        meshes.iter().any(|mesh| {
                            mesh.vertices.iter().any(|vertex| vertex.color.a() == 0)
                        })
                    );
                    assert!(meshes.iter().any(|mesh| {
                        mesh.vertices
                            .iter()
                            .any(|vertex| vertex.color == Color32::RED)
                    }));
                    points.reverse();
                }
            }
        }
    }

    #[test]
    fn concave_silhouettes_keep_their_original_triangles_and_opacity() {
        let points = [
            pos2(20., 20.),
            pos2(80., 20.),
            pos2(80., 80.),
            pos2(50., 45.),
            pos2(20., 80.),
        ];
        assert!(!is_convex(&points));
        let ink = Color32::from_rgba_unmultiplied(0, 140, 90, 100);
        let expected = annotations::triangulate(
            &points
                .iter()
                .map(|point| [point.x, point.y])
                .collect::<Vec<_>>(),
        );
        let Some(egui::Shape::Mesh(mesh)) = fill_shape(&points, ink) else {
            panic!("Concave fills must retain their triangulated silhouette");
        };
        assert_eq!(mesh.indices.len(), expected.len() * 3);
        for (actual, expected) in mesh.vertices.chunks_exact(3).zip(expected) {
            for (actual, [x, y]) in actual.iter().zip(expected) {
                assert_eq!(actual.pos, pos2(x, y));
                assert_eq!(actual.color, ink);
            }
        }
        let cross =
            |a: Pos2, b: Pos2, c: Pos2| (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
        let mut area = 0.;
        for triangle in mesh.vertices.chunks_exact(3) {
            let [a, b, c] = [triangle[0].pos, triangle[1].pos, triangle[2].pos];
            area += cross(a, b, c).abs() / 2.;
            let notch = pos2(50., 70.);
            let sides = [cross(a, b, notch), cross(b, c, notch), cross(c, a, notch)];
            assert!(
                sides.iter().any(|side| *side > 0.) && sides.iter().any(|side| *side < 0.),
                "The concave notch must stay empty"
            );
        }
        assert_eq!(area, 2550.);
    }

    #[test]
    fn repeated_collinear_vertices_are_safe_and_folded_stars_are_not_convex() {
        let repeated = [
            pos2(20., 20.),
            pos2(20., 20.),
            pos2(50., 20.),
            pos2(80., 20.),
            pos2(80., 80.),
            pos2(20., 80.),
            pos2(20., 20.),
        ];
        assert!(matches!(
            fill_shape(&repeated, Color32::BLACK),
            Some(egui::Shape::Path(_))
        ));
        let star: Vec<_> = [0, 2, 4, 1, 3]
            .map(|index| {
                let angle = index as f32 * std::f32::consts::TAU / 5.;
                pos2(50. + 30. * angle.cos(), 50. + 30. * angle.sin())
            })
            .into_iter()
            .collect();
        assert!(!is_convex(&star));
        assert!(fill_shape(&[pos2(20., 20.); 4], Color32::BLACK).is_none());
        assert!(fill_shape(&[pos2(f32::NAN, 20.); 4], Color32::BLACK).is_none());
    }
}
