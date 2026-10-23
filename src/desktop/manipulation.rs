//! Page-space geometry for direct manipulation; previews and commits use the same result.
use super::*;
use mypdf_milestone0::annotations::{Annotation, Box2, Kind};

#[derive(Clone, Copy, Default)]
pub(super) struct Guides {
    pub x: Option<f32>,
    pub y: Option<f32>,
}
impl Guides {
    pub fn paint(self, painter: &egui::Painter, page: Rect, scale: f32) {
        let stroke = Stroke::new(1., Color32::from_rgb(224, 75, 151));
        if let Some(x) = self.x {
            let x = page.left() + x * scale;
            painter.line_segment([pos2(x, page.top()), pos2(x, page.bottom())], stroke);
        }
        if let Some(y) = self.y {
            let y = page.top() + y * scale;
            painter.line_segment([pos2(page.left(), y), pos2(page.right(), y)], stroke);
        }
    }
}
pub(super) fn snap(
    tab: &Tab,
    page: usize,
    bounds: Box2,
    exclude: &[String],
    scale: f32,
) -> ([f32; 2], Guides) {
    let size = tab.pages[page];
    let mut xs = vec![0., size.width / 2., size.width];
    let mut ys = vec![0., size.height / 2., size.height];
    if let Some(s) = &tab.session {
        for a in s
            .annotations
            .iter()
            .filter(|a| a.page == page && !exclude.contains(&a.id))
        {
            let b = a.bounds();
            xs.extend([b[0], (b[0] + b[2]) / 2., b[2]]);
            ys.extend([b[1], (b[1] + b[3]) / 2., b[3]]);
        }
    }
    if let Some((revision, cached_page, guides)) = &tab.placement_guides
        && *cached_page == page
        && tab
            .session
            .as_ref()
            .is_some_and(|s| s.revision() == *revision)
    {
        xs.extend(&guides.xs);
        ys.extend(&guides.ys);
        ys.extend(&guides.baselines);
    }
    let nearest = |values: Vec<f32>, anchors: [f32; 3]| {
        values
            .into_iter()
            .flat_map(|v| anchors.map(|a| (v - a, v)))
            .filter(|(d, _)| d.abs() <= 6. / scale)
            .min_by(|(a, _), (b, _)| a.abs().total_cmp(&b.abs()))
    };
    let x = nearest(xs, [bounds[0], (bounds[0] + bounds[2]) / 2., bounds[2]]);
    let y = nearest(ys, [bounds[1], (bounds[1] + bounds[3]) / 2., bounds[3]]);
    (
        [x.map_or(0., |v| v.0), y.map_or(0., |v| v.0)],
        Guides {
            x: x.map(|v| v.1),
            y: y.map(|v| v.1),
        },
    )
}
pub(super) fn rotation_handle(a: &Annotation, scale: f32) -> Option<[f32; 2]> {
    if matches!(
        a.kind,
        Kind::Text { .. } | Kind::Note { .. } | Kind::Markup { .. }
    ) {
        return None;
    }
    let b = a.bounds();
    Some([(b[0] + b[2]) / 2., b[1] - 25. / scale.max(0.1)])
}
pub(super) fn rotation_at(a: &Annotation, p: [f32; 2], scale: f32) -> bool {
    rotation_handle(a, scale).is_some_and(|h| (p[0] - h[0]).hypot(p[1] - h[1]) * scale < 11.)
}
pub(super) fn rotated_drag(a: &Annotation, start: [f32; 2], p: [f32; 2], snap: bool) -> Annotation {
    let c = [(a.rect[0] + a.rect[2]) / 2., (a.rect[1] + a.rect[3]) / 2.];
    let delta =
        ((p[1] - c[1]).atan2(p[0] - c[0]) - (start[1] - c[1]).atan2(start[0] - c[0])).to_degrees();
    let mut out = a.clone();
    out.rotate_by(delta);
    if snap {
        out.rotation = (out.rotation / 15.).round() * 15.;
    }
    out
}
pub(super) fn endpoints(a: &Annotation) -> Option<[[f32; 2]; 2]> {
    match &a.kind {
        Kind::Line { start, end, .. }
        | Kind::DoubleArrow { start, end }
        | Kind::DoubleLine { start, end }
        | Kind::RoutedLine { start, end, .. }
        | Kind::RoutedArrow { start, end, .. }
        | Kind::RoutedDoubleArrow { start, end, .. } => {
            Some([a.rotated(a.local(*start)), a.rotated(a.local(*end))])
        }
        Kind::Callout { tip, knee, .. } => {
            Some([a.rotated(a.local(*tip)), a.rotated(a.local(*knee))])
        }
        _ => None,
    }
}
pub(super) fn endpoint_at(a: &Annotation, p: [f32; 2], scale: f32) -> Option<usize> {
    endpoints(a)?
        .iter()
        .position(|e| (e[0] - p[0]).hypot(e[1] - p[1]) * scale < 10.)
}
pub(super) fn move_endpoint(
    a: &Annotation,
    end: usize,
    mut p: [f32; 2],
    constrain: bool,
) -> Annotation {
    if let Kind::Callout { .. } = &a.kind {
        let mut out = a.clone();
        let mut point = mypdf_milestone0::annotations::rotate(p, a.rect, -a.rotation);
        if a.flip_x {
            point[0] = a.rect[0] + a.rect[2] - point[0];
        }
        if a.flip_y {
            point[1] = a.rect[1] + a.rect[3] - point[1];
        }
        let local = [
            ((point[0] - a.rect[0]) / (a.rect[2] - a.rect[0])).clamp(0., 1.),
            ((point[1] - a.rect[1]) / (a.rect[3] - a.rect[1])).clamp(0., 1.),
        ];
        if let Kind::Callout { tip, knee, .. } = &mut out.kind {
            if end == 0 {
                *tip = local;
            } else if end == 1 {
                *knee = local;
            }
        }
        return out;
    }
    let Some(mut points) = endpoints(a) else {
        return a.clone();
    };
    if constrain {
        let anchor = points[1 - end];
        let dx = p[0] - anchor[0];
        let dy = p[1] - anchor[1];
        let angle =
            (dy.atan2(dx) / std::f32::consts::FRAC_PI_4).round() * std::f32::consts::FRAC_PI_4;
        let distance = dx.hypot(dy);
        p = [
            anchor[0] + distance * angle.cos(),
            anchor[1] + distance * angle.sin(),
        ];
    }
    points[end] = p;
    let mut out = a.clone();
    let x = points[0][0].min(points[1][0]);
    let y = points[0][1].min(points[1][1]);
    let w = (points[0][0] - points[1][0]).abs().max(1.);
    let h = (points[0][1] - points[1][1]).abs().max(1.);
    out.rect = [x, y, x + w, y + h];
    out.rotation = 0.;
    out.flip_x = false;
    out.flip_y = false;
    let local = points.map(|p| [(p[0] - x) / w, (p[1] - y) / h]);
    match &mut out.kind {
        Kind::Line { start, end, .. }
        | Kind::DoubleArrow { start, end }
        | Kind::DoubleLine { start, end }
        | Kind::RoutedLine { start, end, .. }
        | Kind::RoutedArrow { start, end, .. }
        | Kind::RoutedDoubleArrow { start, end, .. } => {
            *start = local[0];
            *end = local[1];
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mypdf_milestone0::annotations::Style;
    #[test]
    fn callout_leader_handles_preserve_the_text_box_and_transforms() {
        let mut a = Annotation::new(
            0,
            [100., 150., 360., 280.],
            Kind::callout("Keep this text"),
            Style::default(),
        );
        a.rotation = 25.;
        a.flip_x = true;
        let target = a.rotated(a.local([0.15, 0.8]));
        let out = move_endpoint(&a, 0, target, false);
        let Kind::Callout {
            box_rect: before_box,
            knee: before_knee,
            ..
        } = a.kind
        else {
            unreachable!()
        };
        let Kind::Callout {
            box_rect,
            knee,
            tip,
            ref text,
            ..
        } = out.kind
        else {
            unreachable!()
        };
        assert_eq!(box_rect, before_box);
        assert_eq!(knee, before_knee);
        assert!((tip[0] - 0.15).abs() < 0.0001 && (tip[1] - 0.8).abs() < 0.0001);
        assert_eq!(text, "Keep this text");
        assert_eq!(out.rect, a.rect);
        assert_eq!(out.rotation, a.rotation);
        assert_eq!(out.flip_x, a.flip_x);
        out.validate().unwrap();
    }
    #[test]
    fn routed_arrow_endpoint_edits_preserve_the_route_and_opposite_end() {
        use mypdf_milestone0::annotations::Route;
        for route in [Route::Elbow, Route::Stepped, Route::Curved] {
            let mut a = Annotation::new(
                0,
                [50., 80., 200., 160.],
                Kind::RoutedArrow {
                    start: [0., 0.],
                    end: [1., 1.],
                    route,
                },
                Style::default(),
            );
            a.rotation = 30.;
            let first = endpoints(&a).unwrap()[0];
            let out = move_endpoint(&a, 1, [250., 180.], false);
            assert_eq!(endpoints(&out).unwrap(), [first, [250., 180.]]);
            assert!(matches!(out.kind, Kind::RoutedArrow { route: actual, .. } if actual == route));
            out.validate().unwrap();
        }
    }
    #[test]
    fn routed_double_arrow_endpoints_keep_both_heads_and_the_route() {
        use mypdf_milestone0::annotations::Route;
        for route in [Route::Elbow, Route::Stepped, Route::Curved] {
            let mut a = Annotation::new(
                0,
                [50., 80., 200., 160.],
                Kind::RoutedDoubleArrow {
                    start: [0., 0.],
                    end: [1., 1.],
                    route,
                },
                Style::default(),
            );
            a.rotation = 30.;
            a.flip_horizontal();
            let first = endpoints(&a).unwrap()[0];
            let out = move_endpoint(&a, 1, [250., 180.], false);
            assert_eq!(endpoints(&out).unwrap(), [first, [250., 180.]]);
            assert!(matches!(out.kind,Kind::RoutedDoubleArrow {route:actual,..} if actual==route));
            assert_eq!(mypdf_milestone0::annotations::primitives(&out).len(), 3);
            out.validate().unwrap();
        }
    }

    #[test]
    fn rotated_arrow_endpoints_move_independently_and_keep_the_opposite_end() {
        let mut a = Annotation::new(
            0,
            [100., 150., 260., 200.],
            Kind::DoubleArrow {
                start: [0., 0.],
                end: [1., 1.],
            },
            Style::default(),
        );
        a.rotation = 30.;
        a.flip_horizontal();
        let before = endpoints(&a).unwrap();
        let b = move_endpoint(&a, 1, [300., 320.], false);
        assert_eq!(endpoints(&b).unwrap(), [before[0], [300., 320.]]);
        b.validate().unwrap();
        let c = move_endpoint(&b, 1, [before[0][0] + 100., before[0][1] + 6.], true);
        assert!(
            (endpoints(&c).unwrap()[1][1] - before[0][1]).abs() < 0.01,
            "Shift constrains to 45° increments"
        );
    }
    #[test]
    fn parallel_and_routed_lines_keep_their_kind_when_an_endpoint_moves() {
        use mypdf_milestone0::annotations::Route;
        for kind in [
            Kind::DoubleLine {
                start: [0., 0.],
                end: [1., 1.],
            },
            Kind::RoutedLine {
                start: [0., 0.],
                end: [1., 1.],
                route: Route::Elbow,
            },
            Kind::RoutedLine {
                start: [0., 0.],
                end: [1., 1.],
                route: Route::Curved,
            },
        ] {
            let mut a = Annotation::new(0, [50., 80., 200., 160.], kind.clone(), Style::default());
            a.rotation = 30.;
            a.flip_horizontal();
            let first = endpoints(&a).unwrap()[0];
            let out = move_endpoint(&a, 1, [250., 180.], false);
            assert_eq!(endpoints(&out).unwrap(), [first, [250., 180.]]);
            match (&kind, &out.kind) {
                (Kind::DoubleLine { .. }, Kind::DoubleLine { .. }) => {}
                (Kind::RoutedLine { route: before, .. }, Kind::RoutedLine { route: after, .. }) => {
                    assert_eq!(before, after)
                }
                _ => panic!("Moving a line endpoint must not change its kind"),
            }
            out.validate().unwrap();
            let shifted = move_endpoint(&out, 1, [first[0] + 100., first[1] + 6.], true);
            assert!((endpoints(&shifted).unwrap()[1][1] - first[1]).abs() < 0.01);
            shifted.validate().unwrap();
        }
    }

    #[test]
    fn explicit_endings_survive_endpoint_and_rotation_edits_for_line_families() {
        use mypdf_milestone0::annotations::{LineEnd, LineEndings, Route};
        let endings = LineEndings {
            start: LineEnd::Circle,
            end: LineEnd::FilledArrow,
            size: 18.,
        };
        for kind in [
            Kind::Line {
                start: [0., 0.],
                end: [1., 1.],
                arrow: false,
            },
            Kind::DoubleLine {
                start: [0., 0.],
                end: [1., 1.],
            },
            Kind::RoutedLine {
                start: [0., 0.],
                end: [1., 1.],
                route: Route::Curved,
            },
            Kind::RoutedDoubleArrow {
                start: [0., 0.],
                end: [1., 1.],
                route: Route::Stepped,
            },
        ] {
            let mut a = Annotation::new(
                0,
                [50., 80., 200., 160.],
                kind,
                Style {
                    line_endings: Some(endings),
                    ..Default::default()
                },
            );
            a.rotate_by(25.);
            a.flip_horizontal();
            let out = move_endpoint(&a, 1, [240., 180.], false);
            assert_eq!(out.style.line_endings, Some(endings));
            assert_eq!(out.line_endings(), Some(endings));
            let handle = rotation_handle(&out, 1.).unwrap();
            let turned = rotated_drag(&out, handle, [260., 60.], false);
            assert_eq!(turned.style.line_endings, Some(endings));
            turned.validate().unwrap();
        }
    }

    #[test]
    fn rotation_handle_keeps_center_and_shift_snaps_to_fifteen_degrees() {
        let a = Annotation::new(
            0,
            [100., 100., 200., 200.],
            Kind::Rectangle,
            Style::default(),
        );
        let h = rotation_handle(&a, 1.).unwrap();
        assert!(rotation_at(&a, h, 1.));
        let rotated = rotated_drag(&a, h, [240., 110.], true);
        assert_eq!(rotated.rect, a.rect);
        assert_eq!(rotated.rotation % 15., 0.);
        assert!(rotated.rotation > 0.);
        let free = rotated_drag(&a, h, [240., 110.], false);
        assert_ne!(free.rotation, rotated.rotation);
    }
}
