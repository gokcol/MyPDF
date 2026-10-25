//! Page text keeps its document layout when the view is reduced. UI font
//! hinting must not change line breaks or spacing as the page is zoomed out.
use super::*;
use mypdf_milestone0::annotations::{Annotation, Primitive};

pub(super) fn text_resolution(scale: f32) -> f32 {
    scale.max(1.)
}

pub(super) fn paint_text(
    ctx: &egui::Context,
    painter: &egui::Painter,
    a: &Annotation,
    page_rect: Rect,
    scale: f32,
    primitive: Primitive,
) {
    let Primitive::Text {
        text,
        rect,
        size,
        centered,
        color: text_color,
    } = primitive
    else {
        return;
    };
    let ink = editing::color(text_color.unwrap_or(a.style.color), a.style.opacity);
    let resolution = text_resolution(scale);
    let galley = ctx.fonts_mut(|fonts| {
        fonts.layout(
            text.replace('\t', "    "),
            FontId::new(
                size * resolution,
                egui::FontFamily::Name("annotation".into()),
            ),
            ink,
            ((rect[2] - rect[0] - 8.) * resolution).max(1.),
        )
    });
    let mut origin = [rect[0] + 4., rect[1] + 4.];
    if centered {
        origin = [
            (rect[0] + rect[2]) / 2. - galley.size().x / resolution / 2.,
            (rect[1] + rect[3]) / 2. - galley.size().y / resolution / 2.,
        ];
    }
    let point = |p| {
        let p = a.rotated(p);
        page_rect.min + vec2(p[0], p[1]) * scale
    };
    if a.flip_x || a.flip_y {
        let atlas = ctx.fonts(|f| f.font_image_size());
        for row in &galley.rows {
            let mut mesh = row.visuals.mesh.clone();
            for vertex in &mut mesh.vertices {
                vertex.pos = point([
                    origin[0] + (row.pos.x + vertex.pos.x) / resolution,
                    origin[1] + (row.pos.y + vertex.pos.y) / resolution,
                ]);
                // Row meshes use atlas-pixel UVs; only TextShape normally
                // normalizes them. A mirrored mesh must do the same itself.
                vertex.uv.x /= atlas[0] as f32;
                vertex.uv.y /= atlas[1] as f32;
            }
            painter.add(egui::Shape::mesh(mesh));
        }
    } else {
        let mut shape = egui::epaint::TextShape::new(egui::Pos2::ZERO, galley, ink)
            .with_angle(a.rotation.to_radians());
        shape.transform(egui::emath::TSTransform::from_scaling(scale / resolution));
        shape.pos = point(origin);
        painter.add(shape);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mypdf_milestone0::annotations::{Kind, Style};

    fn capture(ctx: &egui::Context, a: &Annotation, scale: f32) -> egui::FullOutput {
        ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                paint_text(
                    ctx,
                    ui.painter(),
                    a,
                    Rect::from_min_size(pos2(20., 30.), vec2(595., 842.) * scale),
                    scale,
                    Primitive::Text {
                        text: "Orhan Gökçöl reviews the document and signs below.".into(),
                        rect: a.rect,
                        size: a.style.font_size,
                        centered: false,
                        color: None,
                    },
                );
            });
        })
    }

    fn annotation() -> Annotation {
        Annotation::new(
            0,
            [40., 50., 240., 120.],
            Kind::Text {
                text: String::new(),
            },
            Style {
                font_size: 14.,
                ..Default::default()
            },
        )
    }

    #[test]
    fn reduced_text_keeps_wrapping_spacing_and_document_geometry() {
        for density in [1., 2.] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(density);
            editing::install_annotation_font(&ctx);
            let a = annotation();
            let original = serde_json::to_value(&a).unwrap();
            let mut expected: Option<Vec<(String, egui::Pos2, egui::Vec2)>> = None;
            for scale in [1., 0.75, 0.5, 0.25, 0.15, 0.1] {
                let output = capture(&ctx, &a, scale);
                let shape = output
                    .shapes
                    .iter()
                    .find_map(|p| match &p.shape {
                        egui::Shape::Text(t) if t.galley.job.text.starts_with("Orhan") => Some(t),
                        _ => None,
                    })
                    .unwrap();
                let geometry: Vec<_> = shape
                    .galley
                    .rows
                    .iter()
                    .map(|row| {
                        (
                            row.glyphs.iter().map(|g| g.chr).collect::<String>(),
                            row.pos / scale,
                            row.size / scale,
                        )
                    })
                    .collect();
                if let Some(expected) = &expected {
                    for ((text, pos, size), (e_text, e_pos, e_size)) in
                        geometry.iter().zip(expected)
                    {
                        assert_eq!(text, e_text);
                        assert!((*pos - *e_pos).length() < 0.001);
                        assert!((*size - *e_size).length() < 0.001);
                    }
                    assert_eq!(geometry.len(), expected.len());
                } else {
                    expected = Some(geometry);
                }
                assert!((shape.pos - (pos2(20., 30.) + vec2(44., 54.) * scale)).length() < 0.001);
            }
            assert_eq!(serde_json::to_value(&a).unwrap(), original);
        }
    }

    #[test]
    fn mirrored_rotated_text_has_normalized_texture_coordinates_at_every_zoom() {
        let ctx = egui::Context::default();
        editing::install_annotation_font(&ctx);
        let mut a = annotation();
        a.rotation = 27.;
        for (flip_x, flip_y) in [(true, false), (false, true), (true, true)] {
            a.flip_x = flip_x;
            a.flip_y = flip_y;
            for scale in [0.1, 0.5, 1., 3.] {
                let output = capture(&ctx, &a, scale);
                let vertices: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|p| match &p.shape {
                        egui::Shape::Mesh(m) => Some(m.vertices.iter()),
                        _ => None,
                    })
                    .flatten()
                    .collect();
                assert!(!vertices.is_empty());
                assert!(vertices.iter().all(|v| v.pos.is_finite()
                    && (0. ..=1.).contains(&v.uv.x)
                    && (0. ..=1.).contains(&v.uv.y)));
            }
        }
    }
}
