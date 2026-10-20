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
        Kind::Line { start, end, .. } | Kind::DoubleArrow { start, end } => {
            Some([a.rotated(a.local(*start)), a.rotated(a.local(*end))])
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
        Kind::Line { start, end, .. } | Kind::DoubleArrow { start, end } => {
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
