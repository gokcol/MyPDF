//! Compact, shared appearance controls for native drawings.
use super::*;
use ann::FillPattern;

pub(super) fn supports(kind: &Kind) -> bool {
    matches!(
        kind,
        Kind::Ink { .. }
            | Kind::Rectangle
            | Kind::Ellipse
            | Kind::Shape { .. }
            | Kind::Line { .. }
            | Kind::DoubleLine { .. }
            | Kind::RoutedLine { .. }
            | Kind::DoubleArrow { .. }
            | Kind::RoutedArrow { .. }
            | Kind::RoutedDoubleArrow { .. }
            | Kind::Callout { .. }
            | Kind::Polygon { .. }
    )
}

pub(super) fn fills(kind: &Kind) -> bool {
    matches!(
        kind,
        Kind::Rectangle
            | Kind::Ellipse
            | Kind::Shape { .. }
            | Kind::Polygon { closed: true, .. }
            | Kind::Callout { .. }
    )
}

pub(super) fn tool_kind(tool: Tool) -> Option<Kind> {
    Some(match tool {
        Tool::Select | Tool::Lasso | Tool::Rectangle => Kind::Rectangle,
        Tool::Ellipse => Kind::Ellipse,
        tool if tool.shape().is_some() => Kind::Shape {
            shape: tool.shape().unwrap(),
        },
        Tool::Pen => Kind::Ink {
            strokes: vec![vec![
                [0., 0.7],
                [0.2, 0.2],
                [0.4, 0.8],
                [0.65, 0.1],
                [1., 0.5],
            ]],
        },
        Tool::Line | Tool::Arrow => Kind::Line {
            start: [0., 0.8],
            end: [1., 0.2],
            arrow: tool == Tool::Arrow,
        },
        Tool::Callout => Kind::callout("Sample"),
        Tool::DoubleLine => Kind::DoubleLine {
            start: [0., 0.8],
            end: [1., 0.2],
        },
        Tool::ElbowLine | Tool::SteppedLine | Tool::CurvedLine => Kind::RoutedLine {
            start: [0., 0.8],
            end: [1., 0.2],
            route: tool.route().unwrap(),
        },
        Tool::ElbowArrow | Tool::SteppedArrow | Tool::CurvedArrow => Kind::RoutedArrow {
            start: [0., 0.8],
            end: [1., 0.2],
            route: tool.route().unwrap(),
        },
        Tool::DoubleElbowArrow | Tool::DoubleSteppedArrow | Tool::DoubleCurvedArrow => {
            Kind::RoutedDoubleArrow {
                start: [0., 0.8],
                end: [1., 0.2],
                route: tool.route().unwrap(),
            }
        }
        Tool::DoubleArrow => Kind::DoubleArrow {
            start: [0., 0.8],
            end: [1., 0.2],
        },
        Tool::Polyline | Tool::Polygon => Kind::Polygon {
            points: vec![[0., 0.8], [0.2, 0.15], [0.6, 0.4], [0.8, 0.05], [1., 0.85]],
            closed: tool == Tool::Polygon,
        },
        _ => return None,
    })
}

/// Returns the active numeric editor, so the inspector keeps its existing
/// live-edit / Undo transaction and does not steal the arrow keys from a field.
pub(super) fn controls(ui: &mut egui::Ui, kind: &Kind, style: &mut Style) -> Option<egui::Id> {
    let mut focus = None;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = vec2(6., 5.);
        let (preview, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), 60.), egui::Sense::hover());
        #[cfg(test)]
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new("drawing_live_preview"), preview));
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Stroke color")
                    .size(12.)
                    .color(theme::muted()),
            );
        });
        ui.push_id("drawing_stroke_color", |ui| {
            color_palette(ui, &mut style.color);
        });
        ui.columns(2, |columns| {
            focus = property_slider(&mut columns[0], "Width", &mut style.width, 0.5..=20., " pt")
                .or(focus);
            let before = style.opacity * 100.;
            let mut opacity = before;
            focus = property_slider(&mut columns[1], "Opacity", &mut opacity, 5.0..=100., "%")
                .or(focus);
            if opacity != before {
                style.opacity = opacity / 100.;
            }
        });
        stroke_styles(ui, &mut style.stroke);
        focus = line_ending_controls(ui, kind, style).or(focus);
        if fills(kind) {
            fill_choices(ui, style);
        }
        // Paint after controls so the preview reflects this frame's changes.
        ui.painter().rect_filled(preview, 7., theme::control());
        ui.painter().rect_stroke(
            preview,
            7.,
            Stroke::new(1., theme::line()),
            egui::StrokeKind::Inside,
        );
        paint(
            ui.painter(),
            preview.shrink2(vec2(18., 12.)),
            kind.clone(),
            style.clone(),
        );
    });
    focus
}

fn line_ending_controls(ui: &mut egui::Ui, kind: &Kind, style: &mut Style) -> Option<egui::Id> {
    let mut endings = ann::line_endings(kind, style)?;
    let before = endings;
    ui.columns(2, |columns| {
        for (column, (label, end)) in columns
            .iter_mut()
            .zip([("Start", &mut endings.start), ("End", &mut endings.end)])
        {
            column.label(RichText::new(label).size(12.).color(theme::muted()));
            let combo = egui::ComboBox::from_id_salt(("drawing_line_end", label))
                .width(column.available_width())
                .truncate()
                .selected_text(end.label())
                .show_ui(column, |ui| {
                    for choice in ann::LineEnd::ALL {
                        ui.selectable_value(end, choice, choice.label());
                    }
                });
            combo.response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::ComboBox,
                    column.is_enabled(),
                    format!("{label} head"),
                )
            });
            #[cfg(test)]
            column.ctx().data_mut(|data| {
                data.insert_temp(
                    egui::Id::new(("drawing_head_combo", label)),
                    combo.response.rect,
                )
            });
        }
    });
    let mut focus = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Head size").size(12.).color(theme::muted()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let response = ui::numeric_value(
                ui,
                egui::DragValue::new(&mut endings.size)
                    .range(0.5..=256.)
                    .clamp_existing_to_range(false)
                    .speed(0.5)
                    .max_decimals(1)
                    .suffix(" pt"),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::DragValue, ui.is_enabled(), "Head size")
            });
            #[cfg(test)]
            ui.ctx().data_mut(|data| {
                data.insert_temp(egui::Id::new("drawing_head_size"), response.rect)
            });
            if response.has_focus() {
                focus = Some(response.id);
            }
        });
    });
    ui.scope(|ui| {
        ui.spacing_mut().slider_width = ui.available_width();
        ui.spacing_mut().interact_size.y = 18.;
        let response = ui::numeric_slider(
            ui,
            egui::Slider::new(&mut endings.size, 0.5..=256.)
                .logarithmic(true)
                .clamping(egui::SliderClamping::Edits)
                .max_decimals(1)
                .show_value(false),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Slider, ui.is_enabled(), "Head size")
        });
        if response.has_focus() {
            focus = Some(response.id);
        }
    });
    if endings != before {
        style.line_endings = Some(endings);
    }
    focus
}

fn fill_choices(ui: &mut egui::Ui, style: &mut Style) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Fill").size(12.).color(theme::muted()));
        let mut color = style.fill.unwrap_or(style.color);
        if ui::color_swatch_sized(ui, &mut color, "Fill color", vec2(24., 24.)) {
            style.fill = Some(color);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(if style.fill.is_some() {
                    style.fill_pattern.label()
                } else {
                    "None"
                })
                .size(11.)
                .color(theme::muted()),
            );
        });
    });
    let width = (ui.available_width() - ui.spacing().item_spacing.x * 5.) / 6.;
    ui.horizontal(|ui| {
        for pattern in std::iter::once(None).chain(FillPattern::ALL.into_iter().map(Some)) {
            let label = pattern.map_or("No fill", FillPattern::label);
            let selected = match pattern {
                None => style.fill.is_none(),
                Some(pattern) => style.fill.is_some() && style.fill_pattern == pattern,
            };
            let (rect, response) = ui.allocate_exact_size(vec2(width, 28.), egui::Sense::click());
            #[cfg(test)]
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::new(("drawing_fill_pattern", label)), rect));
            ui.painter().rect_filled(
                rect,
                5.,
                if selected {
                    theme::accent_soft()
                } else if response.hovered() {
                    theme::control_hover()
                } else {
                    theme::control()
                },
            );
            let inner = rect.shrink2(vec2(5., 6.));
            if let Some(pattern) = pattern {
                let preview_style = Style {
                    color: if selected {
                        theme_rgb(theme::accent())
                    } else {
                        theme_rgb(theme::ink())
                    },
                    width: 0.7,
                    fill: Some(theme_rgb(if selected {
                        theme::accent()
                    } else {
                        theme::ink()
                    })),
                    fill_pattern: pattern,
                    ..Default::default()
                };
                paint(ui.painter(), inner, Kind::Rectangle, preview_style);
            } else {
                ui.painter().rect_stroke(
                    inner,
                    1.,
                    Stroke::new(0.8, theme::muted()),
                    egui::StrokeKind::Inside,
                );
                ui.painter().line_segment(
                    [inner.left_bottom(), inner.right_top()],
                    Stroke::new(1.2, theme::muted()),
                );
            }
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Button,
                    ui.is_enabled(),
                    selected,
                    if pattern.is_some() {
                        format!("{label} fill")
                    } else {
                        label.to_owned()
                    },
                )
            });
            if response.on_hover_text(label).clicked() {
                if let Some(pattern) = pattern {
                    style.fill_pattern = pattern;
                    style.fill.get_or_insert(style.color);
                } else {
                    style.fill = None;
                }
            }
        }
    });
}

fn theme_rgb(color: Color32) -> [f32; 3] {
    [
        color.r() as f32 / 255.,
        color.g() as f32 / 255.,
        color.b() as f32 / 255.,
    ]
}

/// Native drawing previews use exactly the same filled paths as the page and PDF.
fn paint(painter: &egui::Painter, rect: Rect, kind: Kind, style: Style) {
    let rect = shape_preview_rect(rect, &kind);
    let a = Annotation::new(
        0,
        [rect.left(), rect.top(), rect.right(), rect.bottom()],
        kind,
        style,
    );
    let painter = painter.with_clip_rect(
        rect.expand((a.style.width / 2. + 1.).max(4.))
            .intersect(painter.clip_rect()),
    );
    let (scale, offset) =
        if a.style.line_endings.is_some() && ann::line_endings(&a.kind, &a.style).is_some() {
            let bounds = a.bounds();
            let size = vec2(
                (bounds[2] - bounds[0]).max(1.),
                (bounds[3] - bounds[1]).max(1.),
            );
            let scale = (rect.width() / size.x).min(rect.height() / size.y).min(1.);
            let center = pos2((bounds[0] + bounds[2]) / 2., (bounds[1] + bounds[3]) / 2.);
            (scale, rect.center().to_vec2() - center.to_vec2() * scale)
        } else {
            (1., vec2(0., 0.))
        };
    let point = |p: ann::Point2| pos2(p[0] * scale + offset.x, p[1] * scale + offset.y);
    for primitive in ann::primitives(&a) {
        match primitive {
            Primitive::Dot { center } => {
                painter.circle_filled(
                    point(center),
                    a.style.width * scale / 2.,
                    color(a.style.color, a.style.opacity),
                );
            }
            Primitive::Path {
                mut points,
                closed,
                fill,
            } => {
                if fill {
                    let mut mesh = egui::Mesh::default();
                    for triangle in ann::triangulate(&points) {
                        let start = mesh.vertices.len() as u32;
                        for vertex in triangle {
                            mesh.colored_vertex(
                                point(vertex),
                                color(a.style.fill.unwrap_or(a.style.color), a.style.opacity),
                            );
                        }
                        mesh.add_triangle(start, start + 1, start + 2);
                    }
                    painter.add(egui::Shape::mesh(mesh));
                } else {
                    if closed && let Some(first) = points.first().copied() {
                        points.push(first);
                    }
                    painter.add(egui::Shape::line(
                        points.into_iter().map(point).collect(),
                        Stroke::new(a.style.width * scale, color(a.style.color, a.style.opacity)),
                    ));
                }
            }
            Primitive::Head {
                points,
                closed,
                fill,
            } => paint_head(
                &painter,
                points.into_iter().map(point).collect(),
                closed,
                fill,
                a.style.width * scale,
                color(a.style.color, a.style.opacity),
            ),
            Primitive::Text {
                text,
                rect,
                size,
                centered,
                color: text_color,
            } => {
                paint_preview_text(
                    &painter,
                    &text,
                    Rect::from_min_max(pos2(rect[0], rect[1]), pos2(rect[2], rect[3])),
                    size,
                    centered,
                    color(text_color.unwrap_or(a.style.color), a.style.opacity),
                );
            }
            _ => {}
        }
    }
}

/// Endpoint heads are solid and use the stroke color independently of body fill.
pub(super) fn paint_head(
    painter: &egui::Painter,
    mut points: Vec<egui::Pos2>,
    closed: bool,
    fill: bool,
    width: f32,
    ink: Color32,
) {
    if points.len() < 2 {
        return;
    }
    if fill {
        annotation_geometry::paint_fill(painter, &points, ink);
    } else {
        if closed {
            points.push(points[0]);
        }
        painter.add(egui::Shape::line(points, Stroke::new(width, ink)));
    }
}

/// The preview keeps named shapes recognizable inside the short preview box.
/// Generic rectangles and ellipses still show its available free aspect.
fn shape_preview_rect(rect: Rect, kind: &Kind) -> Rect {
    let Kind::Shape { shape } = kind else {
        return rect;
    };
    let ratio = if *shape == ann::Shape::Semicircle {
        2.
    } else {
        1.
    };
    let width = rect.width().min(rect.height() * ratio);
    Rect::from_center_size(rect.center(), vec2(width, width / ratio))
}

/// Preview geometry is compressed to a short panel. Fit only its display
/// galley, preserving the saved annotation's actual text size and typeface.
fn paint_preview_text(
    painter: &egui::Painter,
    text: &str,
    body: Rect,
    size: f32,
    centered: bool,
    color: Color32,
) {
    let content = body.shrink2(vec2(
        4_f32.min(body.width() * 0.2),
        4_f32.min(body.height() * 0.2),
    ));
    if content.width() <= 0. || content.height() <= 0. {
        return;
    }
    let text = text.replace('\t', "    ");
    let layout = |size| {
        painter.layout(
            text.clone(),
            egui::FontId::new(size, egui::FontFamily::Name("annotation".into())),
            color,
            content.width(),
        )
    };
    let fits = |galley: &egui::Galley| {
        galley.size().x <= content.width() && galley.size().y <= content.height()
    };
    let mut galley = layout(size);
    if !fits(&galley) {
        let (mut low, mut high) = (0.5, size.max(0.5));
        galley = layout(low);
        // Wrap boundaries are discontinuous: a binary search keeps the largest
        // legible font that fits, rather than shrinking a two-line sample twice.
        for _ in 0..10 {
            let mid = (low + high) / 2.;
            let candidate = layout(mid);
            if fits(&candidate) {
                low = mid;
                galley = candidate;
            } else {
                high = mid;
            }
        }
    }
    let point = if centered {
        content.center() - galley.size() / 2.
    } else {
        content.min
    };
    // Glyph ink can overhang its logical metrics. Clip to the text inset so it
    // cannot paint across the border or the leader, even for oversized drafts.
    painter
        .with_clip_rect(content.intersect(painter.clip_rect()))
        .galley(point, galley, color);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_shape_previews_fit_the_box_without_stretching() {
        let outer = Rect::from_min_size(pos2(30., 20.), vec2(194., 36.));
        for shape in [
            ann::Shape::Square,
            ann::Shape::Circle,
            ann::Shape::Diamond,
            ann::Shape::Triangle,
            ann::Shape::Pentagon,
            ann::Shape::Hexagon,
            ann::Shape::Semicircle,
        ] {
            let fit = shape_preview_rect(outer, &Kind::Shape { shape });
            let ratio = if shape == ann::Shape::Semicircle {
                2.
            } else {
                1.
            };
            assert!((fit.width() / fit.height() - ratio).abs() < 0.001);
            assert_eq!(fit.center(), outer.center());
            assert!(outer.contains_rect(fit));
        }
        assert_eq!(shape_preview_rect(outer, &Kind::Rectangle), outer);
        assert_eq!(shape_preview_rect(outer, &Kind::Ellipse), outer);
    }

    fn pointer(position: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    fn head_frame(
        ctx: &egui::Context,
        kind: &Kind,
        style: &mut Style,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(420., 520.))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::Area::new(egui::Id::new("head_controls_test"))
                    .fixed_pos(pos2(12., 12.))
                    .show(ctx, |ui| {
                        ui.set_width(198.);
                        controls(ui, kind, style);
                        assert!(
                            ui.min_rect().width() <= 198.1,
                            "Head controls must fit the existing narrow panel: {:?}",
                            ui.min_rect()
                        );
                    });
            },
        )
    }
    fn choose_head(
        ctx: &egui::Context,
        kind: &Kind,
        style: &mut Style,
        side: &str,
        choice: ann::LineEnd,
    ) -> egui::FullOutput {
        let at = ctx
            .data(|data| data.get_temp::<Rect>(egui::Id::new(("drawing_head_combo", side))))
            .unwrap()
            .center();
        head_frame(ctx, kind, style, pointer(at, true));
        head_frame(ctx, kind, style, pointer(at, false));
        let output = head_frame(ctx, kind, style, vec![]);
        let r = output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some(choice.label()))
            .unwrap_or_else(|| panic!("Missing {}", choice.label()))
            .1
            .bounds()
            .unwrap();
        let at = pos2(((r.x0 + r.x1) / 2.) as f32, ((r.y0 + r.y1) / 2.) as f32);
        head_frame(ctx, kind, style, pointer(at, true));
        head_frame(ctx, kind, style, pointer(at, false))
    }
    #[test]
    fn every_line_kind_shows_compact_heads_without_mutating_legacy_defaults() {
        for tool in [
            Tool::Line,
            Tool::DoubleLine,
            Tool::ElbowLine,
            Tool::SteppedLine,
            Tool::CurvedLine,
            Tool::Arrow,
            Tool::DoubleArrow,
            Tool::ElbowArrow,
            Tool::SteppedArrow,
            Tool::CurvedArrow,
            Tool::DoubleElbowArrow,
            Tool::DoubleSteppedArrow,
            Tool::DoubleCurvedArrow,
        ] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let kind = tool_kind(tool).unwrap();
            let mut style = Style::default();
            let before = style.clone();
            for _ in 0..3 {
                head_frame(&ctx, &kind, &mut style, vec![]);
            }
            let output = head_frame(&ctx, &kind, &mut style, vec![]);
            assert_eq!(
                style,
                before,
                "{} must retain legacy metadata until a control changes",
                tool.label()
            );
            let nodes = output.platform_output.accesskit_update.unwrap().nodes;
            for label in ["Start head", "End head", "Head size"] {
                assert!(
                    nodes.iter().any(|(_, node)| node.label() == Some(label)),
                    "{} is missing {label}",
                    tool.label()
                );
            }
        }
        // Inferred legacy heads can exceed the custom range. Simply viewing
        // their controls must never replace them with a clamped override.
        let ctx = egui::Context::default();
        let kind = tool_kind(Tool::Arrow).unwrap();
        let mut style = Style {
            width: 60.,
            ..Default::default()
        };
        let before = style.clone();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                line_ending_controls(ui, &kind, &mut style);
            });
        });
        assert_eq!(style, before);
        for kind in [
            Kind::Rectangle,
            Kind::Ellipse,
            Kind::Shape {
                shape: ann::Shape::Circle,
            },
            Kind::Ink {
                strokes: vec![vec![[0., 0.], [1., 1.]]],
            },
        ] {
            let ctx = egui::Context::default();
            let mut style = Style::default();
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    assert_eq!(line_ending_controls(ui, &kind, &mut style), None);
                });
            });
            assert!(
                ctx.data(
                    |data| data.get_temp::<Rect>(egui::Id::new(("drawing_head_combo", "Start")))
                )
                .is_none()
            );
        }
    }
    #[test]
    fn changing_a_legacy_150_point_head_type_keeps_its_valid_size() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let kind = tool_kind(Tool::Arrow).unwrap();
        let mut style = Style {
            width: 30.,
            ..Default::default()
        };
        let frame = |style: &mut Style, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(420., 520.))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.set_width(198.);
                        line_ending_controls(ui, &kind, style);
                    });
                },
            )
        };
        for _ in 0..3 {
            frame(&mut style, vec![]);
        }
        assert_eq!(style.line_endings, None);
        let at = ctx
            .data(|data| data.get_temp::<Rect>(egui::Id::new(("drawing_head_combo", "End"))))
            .unwrap()
            .center();
        frame(&mut style, pointer(at, true));
        frame(&mut style, pointer(at, false));
        let output = frame(&mut style, vec![]);
        let r = output
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Circle"))
            .unwrap()
            .1
            .bounds()
            .unwrap();
        let at = pos2(((r.x0 + r.x1) / 2.) as f32, ((r.y0 + r.y1) / 2.) as f32);
        frame(&mut style, pointer(at, true));
        frame(&mut style, pointer(at, false));
        let endings = style.line_endings.unwrap();
        assert_eq!(endings.size, 150.);
        assert_eq!(endings.end, ann::LineEnd::Circle);
        Annotation::new(0, [0., 0., 200., 120.], kind, style)
            .validate()
            .unwrap();
    }

    fn filled_head_centers(rect: Rect, kind: &Kind, style: &Style) -> Vec<egui::Pos2> {
        let annotation = Annotation::new(
            0,
            [rect.left(), rect.top(), rect.right(), rect.bottom()],
            kind.clone(),
            style.clone(),
        );
        let bounds = annotation.bounds();
        let scale = (rect.width() / (bounds[2] - bounds[0]).max(1.))
            .min(rect.height() / (bounds[3] - bounds[1]).max(1.))
            .min(1.);
        let offset = rect.center().to_vec2()
            - vec2((bounds[0] + bounds[2]) / 2., (bounds[1] + bounds[3]) / 2.) * scale;
        ann::primitives(&annotation)
            .into_iter()
            .filter_map(|primitive| match primitive {
                Primitive::Head {
                    points, fill: true, ..
                } => {
                    let center = points.iter().fold(egui::Vec2::ZERO, |sum, point| {
                        sum + vec2(point[0], point[1])
                    }) / points.len() as f32;
                    Some((center * scale + offset).to_pos2())
                }
                _ => None,
            })
            .collect()
    }

    fn painted_with_ink_at(
        primitives: &[egui::ClippedPrimitive],
        point: egui::Pos2,
        ink: Color32,
    ) -> bool {
        primitives.iter().any(|primitive| {
            if !primitive.clip_rect.contains(point) {
                return false;
            }
            let egui::epaint::Primitive::Mesh(mesh) = &primitive.primitive else {
                return false;
            };
            mesh.indices.chunks_exact(3).any(|triangle| {
                let vertices = [
                    mesh.vertices[triangle[0] as usize],
                    mesh.vertices[triangle[1] as usize],
                    mesh.vertices[triangle[2] as usize],
                ];
                if !vertices.iter().all(|vertex| vertex.color == ink) {
                    return false;
                }
                let [a, b, c] = vertices.map(|vertex| vertex.pos);
                let cross = |a: egui::Pos2, b: egui::Pos2, c: egui::Pos2| {
                    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
                };
                let area = cross(a, b, c);
                area.abs() > 0.0001
                    && [cross(a, b, point), cross(b, c, point), cross(c, a, point)]
                        .iter()
                        .all(|side| *side * area.signum() >= -0.0001)
            })
        })
    }

    #[test]
    fn head_controls_change_live_and_filled_heads_use_stroke_color() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let kind = tool_kind(Tool::DoubleCurvedArrow).unwrap();
        let mut style = Style {
            color: [0.1, 0.3, 0.9],
            fill: Some([0.9, 0.1, 0.1]),
            stroke: StrokeStyle::Dashed,
            ..Default::default()
        };
        for _ in 0..3 {
            head_frame(&ctx, &kind, &mut style, vec![]);
        }
        choose_head(&ctx, &kind, &mut style, "Start", ann::LineEnd::Circle);
        let output = choose_head(&ctx, &kind, &mut style, "End", ann::LineEnd::FilledArrow);
        let endings = style.line_endings.unwrap();
        assert_eq!(endings.start, ann::LineEnd::Circle);
        assert_eq!(endings.end, ann::LineEnd::FilledArrow);
        let ink = color(style.color, style.opacity);
        let preview = ctx
            .data(|data| data.get_temp::<Rect>(egui::Id::new("drawing_live_preview")))
            .unwrap()
            .shrink2(vec2(18., 12.));
        let heads = filled_head_centers(preview, &kind, &style);
        assert_eq!(heads.len(), 1);
        let rendered = ctx.tessellate(output.shapes, output.pixels_per_point);
        assert!(
            painted_with_ink_at(&rendered, heads[0], ink),
            "The same frame must visibly fill the arrow head with the stroke color"
        );
        assert!(!painted_with_ink_at(
            &rendered,
            heads[0],
            color(style.fill.unwrap(), style.opacity)
        ));
        let at = ctx
            .data(|data| data.get_temp::<Rect>(egui::Id::new("drawing_head_size")))
            .unwrap()
            .center();
        // Settle the closed combo's interaction layer before beginning a new
        // gesture on the numeric field underneath its former popup.
        head_frame(&ctx, &kind, &mut style, vec![egui::Event::PointerMoved(at)]);
        head_frame(&ctx, &kind, &mut style, vec![]);
        head_frame(&ctx, &kind, &mut style, pointer(at, true));
        head_frame(
            &ctx,
            &kind,
            &mut style,
            vec![egui::Event::PointerMoved(at + vec2(20., 0.))],
        );
        head_frame(
            &ctx,
            &kind,
            &mut style,
            vec![egui::Event::PointerMoved(at + vec2(40., 0.))],
        );
        head_frame(&ctx, &kind, &mut style, pointer(at + vec2(40., 0.), false));
        assert!(
            style.line_endings.unwrap().size > endings.size,
            "Dragging Head size updates its real point value: {:?} vs {:?}",
            style.line_endings,
            endings
        );
        assert_eq!(style.stroke, StrokeStyle::Dashed);
        assert_eq!(style.fill, Some([0.9, 0.1, 0.1]));
    }
    #[test]
    fn oversized_head_preview_fits_full_appearance_without_changing_saved_style() {
        let ctx = egui::Context::default();
        let rect = Rect::from_min_size(pos2(20., 20.), vec2(194., 36.));
        let style = Style {
            line_endings: Some(ann::LineEndings {
                start: ann::LineEnd::FilledArrow,
                end: ann::LineEnd::FilledArrow,
                size: 144.,
            }),
            ..Default::default()
        };
        let before = style.clone();
        let kind = tool_kind(Tool::DoubleCurvedArrow).unwrap();
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            let painter = ctx.layer_painter(egui::LayerId::background());
            paint(&painter, rect, kind.clone(), style.clone());
        });
        let fringe = 1. / output.pixels_per_point;
        let rendered = ctx.tessellate(output.shapes, output.pixels_per_point);
        let heads = filled_head_centers(rect, &kind, &style);
        assert_eq!(heads.len(), 2);
        assert!(heads.iter().all(|point| painted_with_ink_at(
            &rendered,
            *point,
            color(style.color, style.opacity)
        )));
        assert!(rendered.iter().all(|primitive| {
            let egui::epaint::Primitive::Mesh(mesh) = &primitive.primitive else {
                return false;
            };
            mesh.vertices
                .iter()
                .all(|vertex| rect.expand(fringe + 0.01).contains(vertex.pos))
        }));
        assert_eq!(style, before);
    }

    #[test]
    fn compact_drawing_controls_fit_narrow_panel_and_keep_all_patterns_clickable() {
        for dark in [false, true] {
            let ctx = egui::Context::default();
            ctx.set_theme(if dark {
                egui::ThemePreference::Dark
            } else {
                egui::ThemePreference::Light
            });
            let mut style = Style::default();
            let frame = |style: &mut Style, events| {
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(400., 500.))),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::Area::new(egui::Id::new("drawing_controls_test"))
                            .fixed_pos(pos2(12., 12.))
                            .show(ctx, |ui| {
                                ui.set_width(198.);
                                controls(ui, &Kind::Rectangle, style);
                                assert!(
                                    ui.min_rect().width() <= 198.1,
                                    "Controls expanded to {:?}",
                                    ui.min_rect()
                                );
                            });
                    },
                )
            };
            for _ in 0..3 {
                frame(&mut style, vec![]);
            }
            let preview = ctx
                .data(|d| d.get_temp::<Rect>(egui::Id::new("drawing_live_preview")))
                .unwrap();
            assert_eq!(preview.height(), 60.);
            assert_eq!(
                style,
                Style::default(),
                "Passive previews must not create edits"
            );
            style.opacity = 0.3333;
            let unchanged = style.clone();
            frame(&mut style, vec![]);
            assert_eq!(
                style, unchanged,
                "Preview must preserve fractional opacity exactly"
            );
            let mut last = None;
            for pattern in FillPattern::ALL {
                let id = egui::Id::new(("drawing_fill_pattern", pattern.label()));
                let rect = ctx.data(|d| d.get_temp::<Rect>(id)).unwrap();
                assert!(rect.left() >= preview.left() && rect.right() <= preview.right() + 0.1);
                if let Some(previous) = last {
                    assert!(rect.left() > previous);
                }
                last = Some(rect.right());
                frame(&mut style, pointer(rect.center(), true));
                frame(&mut style, pointer(rect.center(), false));
                assert_eq!(style.fill_pattern, pattern);
                assert!(style.fill.is_some());
            }
            let none = ctx
                .data(|d| d.get_temp::<Rect>(egui::Id::new(("drawing_fill_pattern", "No fill"))))
                .unwrap();
            frame(&mut style, pointer(none.center(), true));
            frame(&mut style, pointer(none.center(), false));
            assert!(style.fill.is_none());
        }
    }

    #[test]
    fn callout_preview_fits_text_inside_its_body_and_clips_glyph_overhang() {
        let ctx = egui::Context::default();
        install_annotation_font(&ctx);
        let rect = Rect::from_min_size(pos2(12., 12.), vec2(162., 36.));
        let Kind::Callout { box_rect, .. } = Kind::callout("") else {
            unreachable!()
        };
        let body = Rect::from_min_max(
            rect.min + vec2(box_rect[0], box_rect[1]) * rect.size(),
            rect.min + vec2(box_rect[2], box_rect[3]) * rect.size(),
        );
        for size in [12., 24., 72.] {
            for text in [
                "Review: Orhan Gökçöl",
                "Check this detail",
                "İnceleme\nTamamlandı",
            ] {
                let output = ctx.run(egui::RawInput::default(), |ctx| {
                    let painter = ctx.layer_painter(egui::LayerId::new(
                        egui::Order::Foreground,
                        egui::Id::new("callout_preview_fit_test"),
                    ));
                    paint(
                        &painter,
                        rect,
                        Kind::callout(text),
                        Style {
                            font_size: size,
                            ..Default::default()
                        },
                    );
                });
                let shapes: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| {
                        let egui::Shape::Text(text_shape) = &shape.shape else {
                            return None;
                        };
                        Some((shape.clip_rect, text_shape))
                    })
                    .collect();
                assert_eq!(shapes.len(), 1);
                let (clip, text_shape) = shapes[0];
                let text_bounds = Rect::from_min_size(text_shape.pos, text_shape.galley.size());
                assert!(
                    body.contains_rect(clip),
                    "Glyph clipping escaped the callout body: {clip:?}"
                );
                assert!(
                    clip.contains_rect(text_bounds),
                    "Preview text overflowed: {text_bounds:?} outside {clip:?}"
                );
                assert_eq!(text_shape.galley.job.text, text);
            }
        }
    }

    #[test]
    fn property_scrollbars_keep_controls_clear_and_page_width_stable_in_all_inspectors() {
        use eframe::App;
        for panel_width in [230., 254.] {
            for (case, scroll_id) in [
                (0, "new_annotation_properties"),
                (1, "annotation_properties"),
                (2, "group_properties"),
            ] {
                let ctx = egui::Context::default();
                ctx.enable_accesskit();
                // Exercise the portable menu's extra row on macOS too.
                ctx.data_mut(|data| data.insert_temp(egui::Id::new("preview_portable_menu"), true));
                let mut app =
                    Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
                app.new_blank(&ctx);
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                while app.tabs[0].session.is_none() {
                    app.events(&ctx);
                    assert!(std::time::Instant::now() < deadline);
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                app.editor.mode = Mode::Draw;
                app.sidebar = false;
                app.inspector = true;
                let style = Style {
                    stroke: StrokeStyle::DashDot,
                    line_endings: Some(ann::LineEndings {
                        start: ann::LineEnd::ClosedArrow,
                        end: ann::LineEnd::FilledArrow,
                        size: 256.,
                    }),
                    ..Default::default()
                };
                let marks: Vec<_> = [
                    ([100., 100., 260., 200.], Tool::DoubleCurvedArrow),
                    ([300., 300., 450., 450.], Tool::DoubleSteppedArrow),
                ]
                .into_iter()
                .map(|(rect, tool)| {
                    Annotation::new(0, rect, tool_kind(tool).unwrap(), style.clone())
                })
                .collect();
                app.tabs[0]
                    .session
                    .as_mut()
                    .unwrap()
                    .commit(marks.clone())
                    .unwrap();
                if case == 0 {
                    app.editor.select_tool(Tool::DoubleCurvedArrow);
                    app.editor.style = style;
                    selection::set(&mut app.tabs[0], &mut app.editor, vec![]);
                } else {
                    app.editor.select_tool(Tool::Select);
                    selection::set(
                        &mut app.tabs[0],
                        &mut app.editor,
                        marks
                            .iter()
                            .take(case)
                            .map(|mark| mark.id.clone())
                            .collect(),
                    );
                }
                let revision = app.tabs[0].session.as_ref().unwrap().revision();
                ctx.data_mut(|data| {
                    data.insert_persisted(
                        egui::Id::new("annotation_inspector"),
                        egui::containers::panel::PanelState {
                            rect: Rect::from_min_size(
                                pos2(1100. - panel_width, 0.),
                                vec2(panel_width, 1800.),
                            ),
                        },
                    )
                });
                let frame = |app: &mut Viewer, height, events| {
                    ctx.run(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(
                                pos2(0., 0.),
                                vec2(1100., height),
                            )),
                            events,
                            ..Default::default()
                        },
                        |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
                    )
                };
                let viewport = || {
                    ctx.data(|data| {
                        data.get_temp::<(Rect, egui::Vec2)>(egui::Id::new((
                            "property_scroll_viewport",
                            scroll_id,
                        )))
                    })
                    .unwrap()
                };
                let page = |app: &Viewer| {
                    ctx.read_response(egui::Id::new((app.tabs[0].id, 0_usize, "page_actions")))
                        .unwrap()
                        .rect
                };
                let check_bounds = |output: &egui::FullOutput| {
                    let (inner, _) = viewport();
                    let panel = egui::containers::panel::PanelState::load(
                        &ctx,
                        egui::Id::new("annotation_inspector"),
                    )
                    .unwrap()
                    .rect;
                    assert!(
                        (panel.width() - panel_width).abs() < 0.5,
                        "{scroll_id} grew from {panel_width} to {}",
                        panel.width()
                    );
                    assert!(
                        (inner.width() - (panel_width - 32.)).abs() < 0.5,
                        "The permanent gutter must preserve the old control width: {inner:?}"
                    );
                    assert!(
                        (panel.right() - inner.right() - 16.).abs() < 0.5,
                        "The scrollbar must occupy the old right padding"
                    );
                    let mut controls = vec![];
                    for id in [
                        egui::Id::new("drawing_live_preview"),
                        egui::Id::new(("drawing_head_combo", "Start")),
                        egui::Id::new(("drawing_head_combo", "End")),
                        egui::Id::new("drawing_head_size"),
                    ] {
                        controls.push(ctx.data(|data| data.get_temp::<Rect>(id)).unwrap());
                    }
                    for name in ["Ink", "Blue", "Green", "Coral", "Gold"] {
                        controls.push(
                            ctx.data(|data| {
                                data.get_temp::<Rect>(egui::Id::new(("color_swatch", name)))
                            })
                            .unwrap(),
                        );
                    }
                    for (_, node) in &output
                        .platform_output
                        .accesskit_update
                        .as_ref()
                        .unwrap()
                        .nodes
                    {
                        if matches!(
                            node.label(),
                            Some("Width" | "Opacity" | "Head size" | "Custom color")
                        ) && let Some(rect) = node.bounds()
                        {
                            controls.push(Rect::from_min_max(
                                pos2(rect.x0 as f32, rect.y0 as f32),
                                pos2(rect.x1 as f32, rect.y1 as f32),
                            ));
                        }
                    }
                    for control in controls {
                        assert!(
                            control.left() >= inner.left() - 0.5
                                && control.right() <= inner.right() + 0.5,
                            "{scroll_id} control {control:?} overlaps the gutter or clips at {inner:?}"
                        );
                    }
                };
                for _ in 0..4 {
                    frame(&mut app, 1800., vec![]);
                }
                let output = frame(&mut app, 1800., vec![]);
                check_bounds(&output);
                let (tall, content) = viewport();
                assert!(
                    content.y < tall.height(),
                    "Tall fixture must not need scrolling: {scroll_id}"
                );
                let page_width = page(&app).width();
                for _ in 0..4 {
                    frame(&mut app, 480., vec![]);
                }
                let output = frame(&mut app, 480., vec![]);
                check_bounds(&output);
                let (inner, content) = viewport();
                assert!(
                    content.y > inner.height(),
                    "Short fixture must overflow: {scroll_id}"
                );
                assert!(
                    (page(&app).width() - page_width).abs() < 0.5,
                    "Overflow must not reflow the PDF"
                );
                let before = ctx
                    .data(|data| data.get_temp::<Rect>(egui::Id::new("drawing_live_preview")))
                    .unwrap();
                // The 6pt solid rail starts after its permanent 4pt gap.
                let start = pos2(inner.right() + 7., inner.top() + 4.);
                for _ in 0..3 {
                    frame(&mut app, 480., vec![egui::Event::PointerMoved(start)]);
                }
                check_bounds(&frame(&mut app, 480., vec![]));
                assert!(
                    (page(&app).width() - page_width).abs() < 0.5,
                    "Hovering the rail must not resize the page"
                );
                frame(&mut app, 480., pointer(start, true));
                let end = pos2(start.x, inner.bottom() - 4.);
                frame(&mut app, 480., vec![egui::Event::PointerMoved(end)]);
                frame(&mut app, 480., pointer(end, false));
                let output = frame(&mut app, 480., vec![]);
                check_bounds(&output);
                let after = ctx
                    .data(|data| data.get_temp::<Rect>(egui::Id::new("drawing_live_preview")))
                    .unwrap();
                assert!(
                    after.top() < before.top() - 1.,
                    "Dragging the rail must really scroll {scroll_id}: {before:?} -> {after:?}"
                );
                assert!(
                    (page(&app).width() - page_width).abs() < 0.5,
                    "Dragging the rail must not resize the page"
                );
                assert_eq!(
                    app.tabs[0].session.as_ref().unwrap().revision(),
                    revision,
                    "Scrollbar gestures cannot edit the PDF"
                );
            }
        }
    }

    #[test]
    fn selected_arrow_head_changes_are_live_undoable_and_saved() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        app.editor.mode = Mode::Draw;
        let original = Annotation::new(
            0,
            [100., 100., 260., 200.],
            tool_kind(Tool::DoubleCurvedArrow).unwrap(),
            Style::default(),
        );
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![original.clone()])
            .unwrap();
        app.tabs[0].selected_annotation = Some(original.id.clone());
        let frame = |app: &mut Viewer, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1100., 820.))),
                    events,
                    ..Default::default()
                },
                |ctx| app.annotation_inspector(ctx),
            )
        };
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        let at = ctx
            .data(|data| data.get_temp::<Rect>(egui::Id::new(("drawing_head_combo", "Start"))))
            .unwrap()
            .center();
        frame(&mut app, pointer(at, true));
        frame(&mut app, pointer(at, false));
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        let output = frame(&mut app, vec![]);
        let r = output
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Circle"))
            .unwrap()
            .1
            .bounds()
            .unwrap();
        let at = pos2(((r.x0 + r.x1) / 2.) as f32, ((r.y0 + r.y1) / 2.) as f32);
        frame(&mut app, vec![egui::Event::PointerMoved(at)]);
        frame(&mut app, pointer(at, true));
        frame(&mut app, pointer(at, false));
        let session = app.tabs[0].session.as_mut().unwrap();
        let endings = session.annotations[0]
            .style
            .line_endings
            .expect("Choosing Start Circle should update the selected arrow in the live Session");
        assert_eq!(endings.start, ann::LineEnd::Circle);
        assert_eq!(endings.end, ann::LineEnd::OpenArrow);
        let saved =
            ann::load(&lopdf::Document::load_mem(&session.snapshot_bytes().unwrap()).unwrap());
        assert_eq!(saved[0].style.line_endings, Some(endings));
        session.undo();
        assert_eq!(session.annotations, vec![original]);
    }

    #[test]
    fn selected_drawing_pattern_changes_live_and_undo_restores_original() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        app.editor.mode = Mode::Draw;
        let original = Annotation::new(
            0,
            [100., 100., 240., 180.],
            Kind::Rectangle,
            Style::default(),
        );
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![original.clone()])
            .unwrap();
        app.tabs[0].selected_annotation = Some(original.id.clone());
        let frame = |app: &mut Viewer, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1100., 820.))),
                    events,
                    ..Default::default()
                },
                |ctx| app.annotation_inspector(ctx),
            )
        };
        let revision = app.tabs[0].session.as_ref().unwrap().revision();
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), revision);
        let id = egui::Id::new(("drawing_fill_pattern", FillPattern::Crosshatch.label()));
        let rect = ctx.data(|d| d.get_temp::<Rect>(id)).unwrap();
        frame(&mut app, pointer(rect.center(), true));
        frame(&mut app, pointer(rect.center(), false));
        let session = app.tabs[0].session.as_mut().unwrap();
        assert_eq!(
            session.annotations[0].style.fill_pattern,
            FillPattern::Crosshatch
        );
        assert!(session.annotations[0].style.fill.is_some());
        session.undo();
        assert_eq!(session.annotations, vec![original]);
        assert!(app.editor.mode == Mode::Draw);
    }

    #[test]
    fn patterns_are_limited_to_closed_native_drawings() {
        for kind in [
            Kind::Text {
                text: "Text".into(),
            },
            Kind::Stamp {
                text: "Approved".into(),
            },
            Kind::Svg {
                source: String::new(),
                tint: false,
            },
        ] {
            assert!(!supports(&kind));
        }
        assert!(!fills(&tool_kind(Tool::Arrow).unwrap()));
        assert!(!fills(&tool_kind(Tool::Pen).unwrap()));
        assert!(fills(&tool_kind(Tool::Polygon).unwrap()));
    }
}
