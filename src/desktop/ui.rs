//! Native vector controls: crisp at any display scale, with accessible text labels.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum Icon {
    Home,
    Blank,
    Open,
    Save,
    Search,
    Sidebar,
    Inspector,
    Form,
    Shapes,
    Read,
    Pen,
    Sign,
    Shield,
    Library,
    Undo,
    Redo,
    Reload,
    Select,
    Highlight,
    Underline,
    Strikeout,
    Text,
    TextArea,
    Bold,
    Italic,
    AlignLeft,
    AlignCenter,
    AlignRight,
    AlignJustify,
    Note,
    Rectangle,
    Circle,
    Line,
    Arrow,
    DoubleArrow,
    Polyline,
    Polygon,
    Stamp,
    Star,
    Plus,
    Minus,
    Left,
    Right,
    Import,
    Crop,
    Extract,
    Split,
    Trash,
}

pub(super) fn draw_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let p = |x: f32, y: f32| rect.min + vec2(x, y) * rect.size();
    let stroke = Stroke::new(1.65, color);
    let line = |points: &[[f32; 2]]| {
        painter.add(egui::Shape::line(
            points.iter().map(|v| p(v[0], v[1])).collect(),
            stroke,
        ));
    };
    let box_at = |a: [f32; 2], b: [f32; 2]| {
        painter.rect_stroke(
            Rect::from_min_max(p(a[0], a[1]), p(b[0], b[1])),
            2.0,
            stroke,
            egui::StrokeKind::Inside,
        );
    };
    match icon {
        Icon::Reload => {
            line(
                &(0..=24)
                    .map(|i| {
                        let angle = 0.35 + i as f32 / 24. * 5.2;
                        [0.5 + angle.cos() * 0.34, 0.5 + angle.sin() * 0.34]
                    })
                    .collect::<Vec<_>>(),
            );
            line(&[[0.62, 0.12], [0.77, 0.27], [0.56, 0.29]]);
        }
        Icon::TextArea => {
            for (a, b, c) in [
                ([0.1, 0.32], [0.1, 0.1], [0.32, 0.1]),
                ([0.68, 0.1], [0.9, 0.1], [0.9, 0.32]),
                ([0.9, 0.68], [0.9, 0.9], [0.68, 0.9]),
                ([0.32, 0.9], [0.1, 0.9], [0.1, 0.68]),
            ] {
                line(&[a, b, c]);
            }
            line(&[[0.28, 0.34], [0.72, 0.34]]);
            line(&[[0.28, 0.5], [0.72, 0.5]]);
            line(&[[0.28, 0.66], [0.57, 0.66]]);
        }
        Icon::Home => {
            line(&[[0.1, 0.43], [0.5, 0.1], [0.9, 0.43]]);
            line(&[
                [0.2, 0.38],
                [0.2, 0.9],
                [0.4, 0.9],
                [0.4, 0.61],
                [0.6, 0.61],
                [0.6, 0.9],
                [0.8, 0.9],
                [0.8, 0.38],
            ]);
        }
        Icon::Blank => {
            line(&[
                [0.65, 0.1],
                [0.18, 0.1],
                [0.18, 0.9],
                [0.84, 0.9],
                [0.84, 0.3],
                [0.65, 0.1],
                [0.65, 0.3],
                [0.84, 0.3],
            ]);
            line(&[[0.35, 0.58], [0.67, 0.58]]);
            line(&[[0.51, 0.42], [0.51, 0.74]]);
        }

        Icon::Bold | Icon::Italic => {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                if matches!(icon, Icon::Bold) { "B" } else { "I" },
                FontId::proportional(rect.height() * 0.85),
                color,
            );
        }
        Icon::AlignLeft | Icon::AlignCenter | Icon::AlignRight | Icon::AlignJustify => {
            for i in 0..4 {
                let y = 0.2 + i as f32 * 0.2;
                let w = if i % 2 == 0 || matches!(icon, Icon::AlignJustify) {
                    0.78
                } else {
                    0.5
                };
                let x = match icon {
                    Icon::AlignRight => 0.9 - w,
                    Icon::AlignCenter => 0.5 - w / 2.,
                    _ => 0.1,
                };
                line(&[[x, y], [x + w, y]]);
            }
        }
        Icon::Polyline | Icon::Polygon => {
            let pts = [[0.12, 0.8], [0.3, 0.15], [0.85, 0.28], [0.72, 0.84]];
            line(&pts);
            if matches!(icon, Icon::Polygon) {
                line(&[pts[3], pts[0]]);
            }
            for v in pts {
                painter.circle_filled(p(v[0], v[1]), 2.4, PAPER);
                painter.circle_stroke(p(v[0], v[1]), 2.4, stroke);
            }
        }
        Icon::DoubleArrow => {
            line(&[[0.14, 0.85], [0.85, 0.14]]);
            line(&[[0.14, 0.52], [0.14, 0.85], [0.47, 0.85]]);
            line(&[[0.52, 0.14], [0.85, 0.14], [0.85, 0.47]]);
        }
        Icon::Crop => {
            line(&[[0.25, 0.05], [0.25, 0.75], [0.95, 0.75]]);
            line(&[[0.05, 0.25], [0.75, 0.25], [0.75, 0.95]]);
        }
        Icon::Extract => {
            box_at([0.12, 0.08], [0.65, 0.92]);
            line(&[[0.42, 0.5], [0.98, 0.5], [0.8, 0.32]]);
            line(&[[0.98, 0.5], [0.8, 0.68]]);
        }
        Icon::Split => {
            line(&[[0.5, 0.9], [0.5, 0.55], [0.2, 0.25], [0.2, 0.08]]);
            line(&[[0.5, 0.55], [0.8, 0.25], [0.8, 0.08]]);
            line(&[[0.06, 0.22], [0.2, 0.08], [0.34, 0.22]]);
            line(&[[0.66, 0.22], [0.8, 0.08], [0.94, 0.22]]);
        }
        Icon::Trash => {
            box_at([0.22, 0.3], [0.78, 0.9]);
            line(&[[0.1, 0.25], [0.9, 0.25]]);
            line(&[[0.37, 0.25], [0.37, 0.1], [0.63, 0.1], [0.63, 0.25]]);
            line(&[[0.4, 0.43], [0.4, 0.76]]);
            line(&[[0.6, 0.43], [0.6, 0.76]]);
        }
        Icon::Open => {
            line(&[
                [0.08, 0.78],
                [0.08, 0.22],
                [0.4, 0.22],
                [0.5, 0.34],
                [0.85, 0.34],
            ]);
            line(&[
                [0.08, 0.78],
                [0.23, 0.44],
                [0.95, 0.44],
                [0.78, 0.78],
                [0.08, 0.78],
            ]);
        }
        Icon::Save => {
            box_at([0.15, 0.1], [0.85, 0.9]);
            box_at([0.32, 0.1], [0.68, 0.38]);
            box_at([0.3, 0.6], [0.7, 0.9]);
        }
        Icon::Search => {
            painter.circle_stroke(p(0.42, 0.42), rect.width() * 0.29, stroke);
            line(&[[0.64, 0.64], [0.91, 0.91]]);
        }
        Icon::Inspector => {
            box_at([0.1, 0.15], [0.9, 0.85]);
            line(&[[0.64, 0.15], [0.64, 0.85]]);
        }
        Icon::Shapes => {
            box_at([0.06, 0.08], [0.58, 0.6]);
            painter.circle_stroke(p(0.66, 0.66), rect.width() * 0.28, stroke);
        }
        Icon::Form => {
            box_at([0.12, 0.06], [0.88, 0.94]);
            box_at([0.25, 0.26], [0.37, 0.38]);
            line(&[[0.49, 0.32], [0.75, 0.32]]);
            box_at([0.25, 0.61], [0.37, 0.73]);
            line(&[[0.49, 0.67], [0.75, 0.67]]);
        }
        Icon::Sidebar => {
            box_at([0.1, 0.15], [0.9, 0.85]);
            line(&[[0.36, 0.15], [0.36, 0.85]]);
        }
        Icon::Read => {
            line(&[
                [0.5, 0.85],
                [0.15, 0.75],
                [0.15, 0.15],
                [0.5, 0.25],
                [0.85, 0.15],
                [0.85, 0.75],
                [0.5, 0.85],
                [0.5, 0.25],
            ]);
        }
        Icon::Pen | Icon::Sign => {
            line(&[
                [0.18, 0.8],
                [0.25, 0.55],
                [0.7, 0.1],
                [0.88, 0.28],
                [0.43, 0.73],
                [0.18, 0.8],
            ]);
            line(&[[0.62, 0.18], [0.8, 0.36]]);
            if matches!(icon, Icon::Sign) {
                line(&[[0.1, 0.95], [0.9, 0.95]]);
            }
        }
        Icon::Shield => {
            line(&[
                [0.5, 0.08],
                [0.87, 0.22],
                [0.83, 0.63],
                [0.5, 0.92],
                [0.17, 0.63],
                [0.13, 0.22],
                [0.5, 0.08],
            ]);
            line(&[[0.32, 0.49], [0.46, 0.63], [0.69, 0.35]]);
        }
        Icon::Library => {
            for x in [0.12, 0.57] {
                for y in [0.12, 0.57] {
                    box_at([x, y], [x + 0.3, y + 0.3]);
                }
            }
        }
        Icon::Undo | Icon::Redo => {
            let flip = |x| {
                if matches!(icon, Icon::Redo) {
                    1.0 - x
                } else {
                    x
                }
            };
            line(&[[flip(0.35), 0.16], [flip(0.12), 0.38], [flip(0.35), 0.6]]);
            line(&[
                [flip(0.12), 0.38],
                [flip(0.65), 0.38],
                [flip(0.84), 0.55],
                [flip(0.84), 0.8],
            ]);
        }
        Icon::Select => {
            line(&[
                [0.2, 0.1],
                [0.23, 0.87],
                [0.43, 0.66],
                [0.59, 0.95],
                [0.73, 0.87],
                [0.57, 0.6],
                [0.88, 0.6],
                [0.2, 0.1],
            ]);
        }
        Icon::Highlight => {
            line(&[
                [0.2, 0.68],
                [0.52, 0.15],
                [0.83, 0.33],
                [0.5, 0.85],
                [0.2, 0.68],
            ]);
            line(&[[0.1, 0.95], [0.76, 0.95]]);
        }
        Icon::Text => {
            line(&[[0.15, 0.2], [0.85, 0.2]]);
            line(&[[0.5, 0.2], [0.5, 0.85]]);
            line(&[[0.3, 0.85], [0.7, 0.85]]);
        }
        Icon::Underline => {
            line(&[
                [0.25, 0.12],
                [0.25, 0.57],
                [0.35, 0.7],
                [0.65, 0.7],
                [0.75, 0.57],
                [0.75, 0.12],
            ]);
            line(&[[0.15, 0.92], [0.85, 0.92]]);
        }
        Icon::Strikeout => {
            line(&[
                [0.76, 0.2],
                [0.62, 0.12],
                [0.34, 0.12],
                [0.2, 0.3],
                [0.35, 0.46],
                [0.64, 0.57],
                [0.78, 0.74],
                [0.64, 0.9],
                [0.34, 0.9],
                [0.2, 0.8],
            ]);
            line(&[[0.08, 0.5], [0.92, 0.5]]);
        }
        Icon::Note => {
            line(&[
                [0.15, 0.15],
                [0.85, 0.15],
                [0.85, 0.65],
                [0.65, 0.85],
                [0.15, 0.85],
                [0.15, 0.15],
            ]);
            line(&[[0.65, 0.85], [0.65, 0.65], [0.85, 0.65]]);
            line(&[[0.3, 0.35], [0.7, 0.35]]);
            line(&[[0.3, 0.52], [0.6, 0.52]]);
        }
        Icon::Rectangle => {
            box_at([0.1, 0.2], [0.9, 0.8]);
        }
        Icon::Circle => {
            painter.circle_stroke(rect.center(), rect.width() * 0.36, stroke);
        }
        Icon::Line => line(&[[0.15, 0.85], [0.85, 0.15]]),
        Icon::Arrow => {
            line(&[[0.12, 0.8], [0.85, 0.15], [0.45, 0.15]]);
            line(&[[0.85, 0.15], [0.85, 0.55]]);
        }
        Icon::Stamp => {
            line(&[
                [0.12, 0.8],
                [0.12, 0.65],
                [0.38, 0.57],
                [0.38, 0.35],
                [0.3, 0.25],
                [0.35, 0.1],
                [0.65, 0.1],
                [0.7, 0.25],
                [0.62, 0.35],
                [0.62, 0.57],
                [0.88, 0.65],
                [0.88, 0.8],
                [0.12, 0.8],
            ]);
            line(&[[0.1, 0.95], [0.9, 0.95]]);
        }
        Icon::Star => {
            let pts: Vec<_> = (0..10)
                .map(|i| {
                    let a = i as f32 * std::f32::consts::PI / 5.0 - std::f32::consts::FRAC_PI_2;
                    let r = if i % 2 == 0 { 0.43 } else { 0.19 };
                    p(0.5 + a.cos() * r, 0.5 + a.sin() * r)
                })
                .collect();
            painter.add(egui::Shape::closed_line(pts, stroke));
        }
        Icon::Plus | Icon::Minus => {
            line(&[[0.2, 0.5], [0.8, 0.5]]);
            if matches!(icon, Icon::Plus) {
                line(&[[0.5, 0.2], [0.5, 0.8]]);
            }
        }
        Icon::Left | Icon::Right => {
            let x = if matches!(icon, Icon::Left) {
                0.35
            } else {
                0.65
            };
            line(&[[1.0 - x, 0.2], [x, 0.5], [1.0 - x, 0.8]]);
        }
        Icon::Import => {
            line(&[[0.5, 0.1], [0.5, 0.65], [0.3, 0.45]]);
            line(&[[0.5, 0.65], [0.7, 0.45]]);
            line(&[[0.15, 0.6], [0.15, 0.85], [0.85, 0.85], [0.85, 0.6]]);
        }
    }
}

pub(super) struct ActionButton<'a> {
    icon: Icon,
    label: &'a str,
    selected: bool,
    selectable: bool,
    primary: bool,
    compact: bool,
    tab: bool,
    dropdown: bool,
}
impl<'a> ActionButton<'a> {
    pub fn new(icon: Icon, label: &'a str) -> Self {
        Self {
            icon,
            label,
            selected: false,
            selectable: false,
            primary: false,
            compact: false,
            tab: false,
            dropdown: false,
        }
    }
    pub fn selected(mut self, value: bool) -> Self {
        self.selected = value;
        self.selectable = true;
        self
    }
    pub fn primary(mut self) -> Self {
        self.primary = true;
        self
    }
    pub fn tab(mut self) -> Self {
        self.tab = true;
        self
    }
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }
    pub fn dropdown(mut self) -> Self {
        self.dropdown = true;
        self
    }
}
impl egui::Widget for ActionButton<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let font = FontId::proportional(13.0);
        let base_color = if ui.visuals().dark_mode {
            Color32::from_gray(223)
        } else {
            INK
        };
        let galley = ui
            .painter()
            .layout_no_wrap(self.label.into(), font, base_color);
        let size = vec2(
            (if self.compact {
                34.0
            } else {
                galley.size().x + 46.0
            }) + if self.dropdown { 12. } else { 0. },
            if self.tab { 46.0 } else { 34.0 },
        );
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        response.widget_info(|| {
            if self.selectable {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Button,
                    ui.is_enabled(),
                    self.selected,
                    self.label,
                )
            } else {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), self.label)
            }
        });
        if ui.is_rect_visible(rect) {
            let enabled = ui.is_enabled();
            let filled = self.primary && enabled;
            let fill = if filled {
                if response.hovered() {
                    Color32::from_rgb(45, 76, 175)
                } else {
                    ACCENT
                }
            } else if self.selected {
                if ui.visuals().dark_mode {
                    PAPER
                } else {
                    Color32::from_rgb(222, 234, 250)
                }
            } else if response.hovered() {
                if ui.visuals().dark_mode {
                    Color32::from_rgb(60, 63, 70)
                } else {
                    Color32::from_rgb(227, 229, 234)
                }
            } else {
                Color32::TRANSPARENT
            };
            let fg = if !enabled {
                Color32::from_rgb(163, 172, 189)
            } else if filled {
                Color32::WHITE
            } else if self.selected {
                if ui.visuals().dark_mode { INK } else { ACCENT }
            } else {
                base_color
            };
            let border = if filled {
                ACCENT
            } else if self.selected && !self.tab {
                Color32::from_rgb(168, 191, 244)
            } else {
                Color32::TRANSPARENT
            };
            let radius = if self.tab {
                egui::CornerRadius {
                    nw: 7,
                    ne: 7,
                    sw: 0,
                    se: 0,
                }
            } else {
                egui::CornerRadius::same(8)
            };
            ui.painter().rect_filled(rect, radius, fill);
            ui.painter().rect_stroke(
                rect,
                radius,
                Stroke::new(
                    if response.has_focus() { 2.0 } else { 1.0 },
                    if response.has_focus() { ACCENT } else { border },
                ),
                egui::StrokeKind::Inside,
            );
            let icon_rect =
                Rect::from_center_size(pos2(rect.left() + 17.0, rect.center().y), vec2(19.0, 19.0));
            draw_icon(ui.painter(), icon_rect, self.icon, fg);
            if !self.compact {
                ui.painter().galley_with_override_text_color(
                    pos2(rect.left() + 32.0, rect.center().y - galley.size().y / 2.0),
                    galley,
                    fg,
                );
            }
            if self.dropdown {
                let p = pos2(rect.right() - 10., rect.center().y);
                ui.painter().add(egui::Shape::line(
                    vec![p + vec2(-3., -1.5), p + vec2(0., 1.5), p + vec2(3., -1.5)],
                    Stroke::new(1.4, fg),
                ));
            }
        }
        response
            .on_hover_text(self.label)
            .on_hover_cursor(egui::CursorIcon::PointingHand)
    }
}

#[derive(Clone, Copy)]
pub(super) enum DialogTone {
    Primary,
    Secondary,
    Destructive,
}

pub(super) fn dialog_button(ui: &mut egui::Ui, label: &str, tone: DialogTone) -> egui::Response {
    let (fill, text) = match tone {
        DialogTone::Primary => (Color32::from_rgb(0, 122, 255), Color32::WHITE),
        DialogTone::Secondary => (Color32::from_rgb(226, 226, 230), INK),
        DialogTone::Destructive => (
            Color32::from_rgb(226, 226, 230),
            Color32::from_rgb(183, 46, 44),
        ),
    };
    ui.add_sized(
        vec2(ui.available_width(), 35.),
        egui::Button::new(RichText::new(label).size(14.).color(text))
            .fill(fill)
            .stroke(Stroke::NONE)
            .corner_radius(16.),
    )
}

pub(super) fn confirmation(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    body: &str,
    buttons: &[(&str, DialogTone)],
    cancel: usize,
) -> Option<usize> {
    let mut decision = None;
    let modal = egui::Modal::new(egui::Id::new(id))
        .backdrop_color(Color32::from_black_alpha(40))
        .frame(
            egui::Frame::new()
                .fill(Color32::from_rgb(242, 242, 244))
                .corner_radius(24.)
                .inner_margin(22.)
                .stroke(Stroke::new(1., Color32::from_white_alpha(180)))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 8],
                    blur: 32,
                    spread: 0,
                    color: Color32::from_black_alpha(65),
                }),
        )
        .show(ctx, |ui| {
            ui.set_width(286.);
            ui.spacing_mut().item_spacing.y = 8.;
            ui.label(RichText::new(title).size(17.).strong().color(INK));
            ui.add_space(7.);
            ui.label(RichText::new(body).size(14.).color(INK));
            ui.add_space(16.);
            for (index, (label, tone)) in buttons.iter().enumerate() {
                if dialog_button(ui, label, *tone).clicked() {
                    decision = Some(index);
                }
            }
        });
    if modal.is_top_modal && !modal.any_popup_open && decision.is_none() {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            decision = Some(cancel);
        } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
            decision = Some(0);
        }
    }
    decision
}

pub(super) fn install_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Light);
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::light();
    style.visuals.panel_fill = PAPER;
    style.visuals.window_fill = PAPER;
    style.visuals.window_corner_radius = 16.into();
    style.visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 8],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(45),
    };
    style.visuals.window_stroke = Stroke::new(1.0, Color32::from_rgb(218, 218, 224));
    style.visuals.selection.bg_fill = Color32::from_rgb(218, 231, 255);
    style.visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    for widget in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
    ] {
        widget.corner_radius = 7.into();
        widget.fg_stroke = Stroke::new(1.0, INK);
        widget.bg_stroke = Stroke::new(1.0, Color32::from_rgb(217, 226, 239));
    }
    style.visuals.widgets.inactive.bg_fill = Color32::WHITE;
    style.visuals.widgets.inactive.weak_bg_fill = Color32::WHITE;
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(232, 239, 253);
    style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(232, 239, 253);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(217, 230, 255);
    style.visuals.widgets.active.weak_bg_fill = Color32::from_rgb(217, 230, 255);
    style.visuals.widgets.noninteractive.fg_stroke.color = INK;
    style.visuals.widgets.noninteractive.bg_stroke.color = Color32::from_rgb(226, 232, 241);
    style.visuals.extreme_bg_color = Color32::WHITE;
    style.spacing.button_padding = vec2(12.0, 8.0);
    style.spacing.item_spacing = vec2(8.0, 8.0);
    style.spacing.interact_size = vec2(36.0, 32.0);
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, FontId::proportional(13.0));
    style
        .text_styles
        .insert(egui::TextStyle::Heading, FontId::proportional(21.0));
    ctx.set_style(style);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn confirmation_keyboard_chooses_safe_default_and_cancel() {
        for (key, expected) in [(egui::Key::Escape, 2), (egui::Key::Enter, 0)] {
            let ctx = egui::Context::default();
            let mut decision = None;
            let buttons = [
                ("Save a Copy…", DialogTone::Primary),
                ("Don’t Save", DialogTone::Secondary),
                ("Cancel", DialogTone::Secondary),
            ];
            for events in [
                vec![],
                vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            ] {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(800., 600.))),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        decision = confirmation(
                            ctx,
                            "test_confirm",
                            "Save changes?",
                            "Keep your edits.",
                            &buttons,
                            2,
                        );
                    },
                );
            }
            assert_eq!(decision, Some(expected));
        }
    }
}
