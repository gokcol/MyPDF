//! Visual SVG studio: object properties and direct manipulation share the source.
use super::*;
use mypdf_milestone0::svg_edit::{self, Element, Geometry, LayerAction, Scene};

type GeometryCache = (String, Scene, [f32; 2], Vec<Option<Geometry>>);
type Layout = (Scene, [f32; 2], Vec<Option<Geometry>>);

#[derive(Default)]
pub(super) struct State {
    pub source_mode: bool,
    selected: usize,
    rotation: f32,
    border_tab: bool,
    border: Option<svg_edit::Border>,
    replace_frame: bool,
    solid_preview: bool,
    undo: Vec<String>,
    redo: Vec<String>,
    restored: bool,
    last_edit: Option<(std::time::Instant, usize)>,
    cached: Option<GeometryCache>,
    texture: Option<(String, TextureHandle)>,
    drag: Option<Drag>,
    pub error: Option<String>,
}
struct Drag {
    source: String,
    geometry: Geometry,
    start: egui::Pos2,
    corner: Option<usize>,
}
impl State {
    pub fn undo_redo(&mut self, source: &mut String, redo: bool) {
        let stack = if redo { &mut self.redo } else { &mut self.undo };
        if let Some(next) = stack.pop() {
            if redo {
                self.undo.push(source.clone());
            } else {
                self.redo.push(source.clone());
            }
            *source = next;
            self.restored = true;
            self.last_edit = None;
            self.error = None;
            self.drag = None;
            self.border = None;
        }
    }
    pub fn remember(&mut self, before: String, after: &str) {
        if self.restored {
            self.restored = false;
            return;
        }
        if before == after || self.drag.is_some() {
            return;
        }
        if !self
            .last_edit
            .is_some_and(|(time, index)| time.elapsed().as_millis() < 700 && index == self.selected)
        {
            self.checkpoint(before);
        }
        self.redo.clear();
        self.last_edit = Some((std::time::Instant::now(), self.selected));
    }
    fn checkpoint(&mut self, source: String) {
        if self.undo.last() != Some(&source) {
            self.undo.push(source);
        }
        if self.undo.len() > 64 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    fn apply(&mut self, source: &mut String, result: anyhow::Result<String>) {
        match result {
            Ok(next) => {
                *source = next;
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    fn geometry(&mut self, source: &str) -> Option<Layout> {
        if self.cached.as_ref().is_none_or(|cache| cache.0 != source) {
            let parsed = Scene::parse(source).and_then(|scene| {
                let (size, geometry) = svg_edit::geometry(source, &scene)?;
                Ok((source.to_owned(), scene, size, geometry))
            });
            match parsed {
                Ok(cache) => self.cached = Some(cache),
                Err(_) => return None,
            }
        }
        self.cached
            .as_ref()
            .map(|c| (c.1.clone(), c.2, c.3.clone()))
    }
}

pub(super) fn toolbar(ui: &mut egui::Ui, source: &mut String, state: &mut State) {
    ui.horizontal_wrapped(|ui| {
        if ui
            .add(ActionButton::new(Icon::Shapes, "Visual editor").selected(!state.source_mode))
            .clicked()
        {
            state.source_mode = false;
        }
        if ui
            .add(ActionButton::new(Icon::Text, "SVG source").selected(state.source_mode))
            .clicked()
        {
            state.source_mode = true;
        }
        ui.separator();
        if ui
            .add_enabled(
                !state.undo.is_empty(),
                ActionButton::new(Icon::Undo, "Undo").compact(),
            )
            .clicked()
        {
            state.undo_redo(source, false);
        }
        if ui
            .add_enabled(
                !state.redo.is_empty(),
                ActionButton::new(Icon::Redo, "Redo").compact(),
            )
            .clicked()
        {
            state.undo_redo(source, true);
        }
        ui.menu_button("Add object", |ui| {
            for kind in ["Text", "Rectangle", "Ellipse", "Line", "Triangle"] {
                if ui
                    .add(ActionButton::new(
                        match kind {
                            "Text" => Icon::Text,
                            "Rectangle" => Icon::Rectangle,
                            "Ellipse" => Icon::Ellipse,
                            "Line" => Icon::Line,
                            _ => Icon::Triangle,
                        },
                        kind,
                    ))
                    .clicked()
                {
                    match svg_edit::add(source, kind) {
                        Ok((next, selected)) => {
                            state.selected = selected;
                            state.border_tab = false;
                            *source = next;
                            state.error = None;
                            state.last_edit = None;
                        }
                        Err(error) => state.error = Some(error.to_string()),
                    }
                    ui.close();
                }
            }
        });
    });
    ui.add_space(6.);
}
fn field(
    ui: &mut egui::Ui,
    e: &Element,
    label: &str,
    key: &str,
    default: &str,
    changes: &mut Vec<(String, String)>,
) -> bool {
    let mut value = e.value(key, default).to_owned();
    let mut focused = false;
    ui.vertical(|ui| {
        ui.label(RichText::new(label).small().color(theme::muted()));
        if let Ok(mut number) = value.parse::<f32>() {
            let r = ui.add(egui::DragValue::new(&mut number).speed(0.25).range(
                if matches!(
                    key,
                    "width" | "height" | "r" | "rx" | "ry" | "font-size" | "stroke-width"
                ) {
                    0. ..=10000.
                } else {
                    -10000. ..=10000.
                },
            ));
            focused |= r.has_focus();
            if r.changed() {
                changes.push((key.into(), number.to_string()));
            }
        } else {
            let r = ui.add(
                egui::TextEdit::singleline(&mut value)
                    .desired_width(160.)
                    .char_limit(200),
            );
            focused |= r.has_focus();
            if r.changed() {
                changes.push((key.into(), value));
            }
        }
    });
    focused
}
fn choice(
    ui: &mut egui::Ui,
    e: &Element,
    label: &str,
    key: &str,
    default: &str,
    options: &[(&str, &str)],
    changes: &mut Vec<(String, String)>,
) {
    let current = e.value(key, default);
    let mut value = current.to_owned();
    ui.horizontal(|ui| {
        ui.label(label);
        egui::ComboBox::from_id_salt(key)
            .selected_text(options.iter().find(|(_, v)| *v == current).map_or(
                match (key, current) {
                    ("font-weight", "700") => "Bold",
                    ("font-weight", "400") => "Regular",
                    _ => current,
                },
                |(label, _)| label,
            ))
            .show_ui(ui, |ui| {
                for (label, option) in options {
                    ui.selectable_value(&mut value, (*option).into(), *label);
                }
            });
    });
    if value != current {
        changes.push((key.into(), value));
    }
}
fn paint(
    ui: &mut egui::Ui,
    e: &Element,
    label: &str,
    key: &str,
    default: &str,
    changes: &mut Vec<(String, String)>,
) -> bool {
    let current = e.value(key, default);
    let mut value = current.to_owned();
    let mut focused = false;
    ui.horizontal(|ui| {
        ui.label(label);
        let mut none = current == "none";
        if ui.checkbox(&mut none, "None").changed() { changes.push((key.into(), if none { "none" } else { "#2563eb" }.into())); }
        if !none {
            // usvg resolves named colors, short hex, rgb(), and other SVG color syntax.
            let escaped = quick_xml::escape::escape(current);
            let sample = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\"><path fill=\"{escaped}\" d=\"M0 0H1V1H0Z\"/></svg>");
            let mut rgb = mypdf_milestone0::svg::tree(&sample, None).ok().and_then(|tree| {
                if let resvg::usvg::Node::Path(path) = &tree.root().children()[0]
                    && let Some(fill) = path.fill()
                    && let resvg::usvg::Paint::Color(c) = fill.paint() { return Some([c.red, c.green, c.blue]); }
                None
            }).unwrap_or([37, 99, 235]);
            if ui.color_edit_button_srgb(&mut rgb).changed() { changes.push((key.into(), format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]))); }
            let r = ui.add(egui::TextEdit::singleline(&mut value).desired_width(110.).char_limit(200));
            focused |= r.has_focus();
            if r.changed() { changes.push((key.into(), value)); }
        }
    });
    focused
}

fn values_len_allows_remove(element: &Element) -> bool {
    element
        .value("points", "")
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|v| !v.is_empty())
        .count()
        > if element.tag == "polygon" { 6 } else { 4 }
}

pub(super) fn inspector(ui: &mut egui::Ui, source: &mut String, state: &mut State) -> bool {
    let mut focused = false;
    ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
    ui.horizontal(|ui| {
        if ui
            .add(ActionButton::new(Icon::Inspector, "Object").selected(!state.border_tab))
            .clicked()
        {
            state.border_tab = false;
        }
        if ui
            .add(ActionButton::new(Icon::Stamp, "Stamp border").selected(state.border_tab))
            .clicked()
        {
            state.border_tab = true;
            if let Ok(scene) = Scene::parse(source) {
                state.replace_frame = scene
                    .elements
                    .get(state.selected)
                    .is_some_and(|e| matches!(e.tag.as_str(), "rect" | "ellipse" | "circle"));
            }
        }
    });
    ui.add_space(10.);
    if state.border_tab {
        focused |= border_controls(ui, source, state);
    } else {
        focused |= controls(ui, source, state);
    }
    focused
}

fn border_controls(ui: &mut egui::Ui, source: &mut String, state: &mut State) -> bool {
    use svg_edit::BorderKind;
    let Ok(scene) = Scene::parse(source) else {
        ui.label("Open SVG source to correct the artwork.");
        return false;
    };
    let existing = svg_edit::current_border(source).ok().flatten();
    if state.border.is_none() {
        state.border = Some(
            existing
                .clone()
                .unwrap_or_else(|| svg_edit::Border::for_scene(&scene)),
        );
    }
    let border = state.border.as_mut().unwrap();
    let mut apply = false;
    let mut remove = false;
    let mut changed = false;
    let mut focused = false;
    egui::ScrollArea::vertical()
        .id_salt("border_properties")
        .max_height(425.)
        .show(ui, |ui| {
            ui.label(
                RichText::new("Frame your stamp")
                    .family(ui::semibold())
                    .size(17.),
            );
            ui.label(
                RichText::new("Fitted to the canvas, behind your lettering.")
                    .small()
                    .color(theme::muted()),
            );
            ui.add_space(8.);
            for row in BorderKind::ALL.chunks(3) {
                ui.columns(3, |cols| {
                    for (&kind, col) in row.iter().zip(cols.iter_mut()) {
                        let (rect, response) = col.allocate_exact_size(
                            vec2(col.available_width(), 52.),
                            egui::Sense::click(),
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, kind.label())
                        });
                        let selected = existing.is_some() && border.kind == kind;
                        let color = if selected {
                            theme::accent()
                        } else {
                            theme::muted()
                        };
                        col.painter().rect_filled(
                            rect,
                            9.,
                            if selected {
                                theme::accent_soft()
                            } else {
                                theme::control()
                            },
                        );
                        col.painter().rect_stroke(
                            rect,
                            9.,
                            Stroke::new(
                                if selected { 1.5 } else { 1. },
                                if selected { color } else { theme::line() },
                            ),
                            egui::StrokeKind::Inside,
                        );
                        let shape = Rect::from_center_size(
                            rect.center() - vec2(0., 9.),
                            vec2((rect.width() - 24.).max(10.), 24.),
                        );
                        let stroke = Stroke::new(1.5, color);
                        if kind == BorderKind::Oval {
                            col.painter().add(egui::Shape::ellipse_stroke(
                                shape.center(),
                                shape.size() / 2.,
                                stroke,
                            ));
                        } else {
                            let radius = if matches!(
                                kind,
                                BorderKind::Rectangle | BorderKind::DoubleRectangle
                            ) {
                                0.
                            } else {
                                6.
                            };
                            if kind == BorderKind::Dashed {
                                col.painter().extend(egui::Shape::dashed_line(
                                    &[
                                        shape.left_top(),
                                        shape.right_top(),
                                        shape.right_bottom(),
                                        shape.left_bottom(),
                                        shape.left_top(),
                                    ],
                                    stroke,
                                    4.,
                                    3.,
                                ));
                            } else {
                                col.painter().rect_stroke(
                                    shape,
                                    radius,
                                    stroke,
                                    egui::StrokeKind::Inside,
                                );
                            }
                            if matches!(
                                kind,
                                BorderKind::DoubleRounded | BorderKind::DoubleRectangle
                            ) {
                                col.painter().rect_stroke(
                                    shape.shrink(4.),
                                    radius * 0.6,
                                    Stroke::new(1., color),
                                    egui::StrokeKind::Inside,
                                );
                            }
                        }
                        col.painter().text(
                            pos2(rect.center().x, rect.bottom() - 12.),
                            egui::Align2::CENTER_CENTER,
                            kind.label(),
                            FontId::new(11., ui::semibold()),
                            theme::ink(),
                        );
                        if response.clicked() {
                            border.kind = kind;
                            apply = true;
                        }
                    }
                });
                ui.add_space(6.);
            }
            ui.add_space(4.);
            ui::section(ui, "Border", |ui| {
                changed |= border_color(ui, "Color", &mut border.color, &mut focused);
                let short = scene.view_box[2].min(scene.view_box[3]);
                ui.columns(2, |cols| {
                    cols[0].label(RichText::new("Thickness").small().color(theme::muted()));
                    let r = cols[0].add(
                        egui::DragValue::new(&mut border.width)
                            .speed(0.2)
                            .range(0.1..=(short * 0.12).max(0.1)),
                    );
                    changed |= r.changed();
                    focused |= r.has_focus();
                    cols[1].label(RichText::new("Padding").small().color(theme::muted()));
                    let r = cols[1].add(
                        egui::DragValue::new(&mut border.inset)
                            .speed(0.2)
                            .range(0. ..=short * 0.3),
                    );
                    changed |= r.changed();
                    focused |= r.has_focus();
                });
                if matches!(
                    border.kind,
                    BorderKind::Rounded | BorderKind::DoubleRounded | BorderKind::Dashed
                ) {
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut border.radius, 0. ..=short / 2.).text("Corners"),
                        )
                        .changed();
                }
            });
            ui.add_space(8.);
            ui::section(ui, "Background", |ui| {
                let mut enabled = border.background.is_some();
                if ui.checkbox(&mut enabled, "Fill behind the stamp").changed() {
                    border.background = enabled.then(|| border.color.clone());
                    changed = true;
                }
                if let Some(color) = &mut border.background {
                    changed |= border_color(ui, "Color", color, &mut focused);
                    let mut percent = border.background_opacity * 100.;
                    if ui
                        .add(
                            egui::Slider::new(&mut percent, 0. ..=100.)
                                .text("Opacity")
                                .suffix("%"),
                        )
                        .changed()
                    {
                        border.background_opacity = percent / 100.;
                        changed = true;
                    }
                } else {
                    ui.label(
                        RichText::new("Transparent background")
                            .small()
                            .color(theme::muted()),
                    );
                }
            });
            ui.add_space(8.);
            if existing.is_none()
                && scene
                    .elements
                    .get(state.selected)
                    .is_some_and(|e| matches!(e.tag.as_str(), "rect" | "ellipse" | "circle"))
            {
                ui.checkbox(&mut state.replace_frame, "Replace the selected frame");
            }
            if existing.is_none() {
                apply |= ui
                    .add(ActionButton::new(Icon::Stamp, "Apply border").primary())
                    .clicked();
            }
            if existing.is_some() {
                remove = ui
                    .add(ActionButton::new(Icon::Trash, "Remove border"))
                    .clicked();
            }
        });
    if remove {
        let result = svg_edit::remove_border(source);
        state.apply(source, result);
        state.border = None;
        state.last_edit = None;
    } else if apply || changed && existing.is_some() {
        let replace = (existing.is_none() && state.replace_frame).then_some(state.selected);
        let result = svg_edit::set_border(source, border, replace);
        state.apply(source, result);
        state.last_edit = None;
    }
    focused
}
fn border_color(ui: &mut egui::Ui, label: &str, value: &mut String, focused: &mut bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        let mut rgb = [37, 99, 235];
        if let Some(hex) = value.strip_prefix('#')
            && hex.len() == 6
            && let Ok(n) = u32::from_str_radix(hex, 16)
        {
            rgb = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        }
        if ui.color_edit_button_srgb(&mut rgb).changed() {
            *value = format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]);
            changed = true;
        }
        let r = ui.add(
            egui::TextEdit::singleline(value)
                .desired_width(100.)
                .margin(ui::FIELD_MARGIN)
                .char_limit(100),
        );
        changed |= r.changed();
        *focused |= r.has_focus();
    });
    changed
}

pub(super) fn controls(ui: &mut egui::Ui, source: &mut String, state: &mut State) -> bool {
    let scene = match Scene::parse(source) {
        Ok(scene) => scene,
        Err(error) => {
            ui.label(error.to_string());
            ui.label("Open SVG source to correct the artwork.");
            return false;
        }
    };
    state.selected = state.selected.min(scene.elements.len().saturating_sub(1));
    ui::caption(ui, "Layers · front to back");
    egui::ScrollArea::vertical()
        .id_salt("svg_objects")
        .max_height(76.)
        .show(ui, |ui| {
            for (index, element) in scene.elements.iter().enumerate().rev() {
                if ui
                    .selectable_label(state.selected == index, element.label())
                    .clicked()
                {
                    state.selected = index;
                    state.last_edit = None;
                }
            }
        });
    ui.horizontal_wrapped(|ui| {
        for (label, action) in [
            ("Duplicate", LayerAction::Duplicate),
            ("Delete", LayerAction::Delete),
            ("Forward", LayerAction::Forward),
            ("Backward", LayerAction::Backward),
        ] {
            if ui
                .add(
                    ActionButton::new(
                        match label {
                            "Duplicate" => Icon::Copy,
                            "Delete" => Icon::Trash,
                            "Forward" => Icon::BringForward,
                            _ => Icon::SendBackward,
                        },
                        label,
                    )
                    .compact(),
                )
                .clicked()
            {
                match svg_edit::layer(source, state.selected, action) {
                    Ok((next, index)) => {
                        *source = next;
                        state.selected = index;
                        state.error = None;
                        state.last_edit = None;
                    }
                    Err(error) => state.error = Some(error.to_string()),
                }
            }
        }
    });
    let Ok(scene) = Scene::parse(source) else {
        return false;
    };
    let Some(element) = scene.elements.get(state.selected) else {
        return false;
    };
    let mut changes = Vec::new();
    let mut text = None;
    let mut focused = false;
    ui.add_space(8.);
    egui::ScrollArea::vertical().id_salt("svg_properties").max_height(320.).show(ui, |ui| {
        if element.tag == "text" {
            ui::section(ui, "Text & font", |ui| {
            if let Some((_, value)) = &element.text {
                let mut value = value.clone();
                let r = ui.add(egui::TextEdit::singleline(&mut value).desired_width(f32::INFINITY).char_limit(8000));
                r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Artwork text"));
                focused |= r.has_focus();
                if r.changed() { text = Some(value); }
            } else { ui.label(RichText::new("Nested text spans can be edited in SVG source.").small().color(theme::muted())); }
            let family = element.value("font-family", "Arial");
            let mut chosen = family.to_owned();
            egui::ComboBox::from_id_salt("svg_font").width(ui.available_width()).selected_text(family).show_ui(ui, |ui| {
                let mut families: Vec<_> = mypdf_milestone0::text_style::fonts().iter().map(|f| f.family.as_str()).collect();
                families.sort_unstable(); families.dedup();
                for family in families { ui.selectable_value(&mut chosen, family.into(), family); }
            });
            if chosen != family { changes.push(("font-family".into(), chosen)); }
            focused |= field(ui, element, "Font size", "font-size", "16", &mut changes);
            choice(ui, element, "Weight", "font-weight", "normal", &[("Light", "300"), ("Regular", "normal"), ("Medium", "500"), ("Semibold", "600"), ("Bold", "bold"), ("Heavy", "900")], &mut changes);
            choice(ui, element, "Style", "font-style", "normal", &[("Regular", "normal"), ("Italic", "italic"), ("Oblique", "oblique")], &mut changes);
            choice(ui, element, "Alignment", "text-anchor", "start", &[("Left", "start"), ("Center", "middle"), ("Right", "end")], &mut changes);
            choice(ui, element, "Decoration", "text-decoration", "none", &[("None", "none"), ("Underline", "underline"), ("Strike through", "line-through")], &mut changes);
            focused |= field(ui, element, "Letter spacing", "letter-spacing", "0", &mut changes);
            if element.attributes.contains_key("textLength") {
                focused |= field(ui, element, "Text width", "textLength", "100", &mut changes);
            }
            });
            ui.add_space(8.);
        }
        ui::section(ui, "Geometry", |ui| {
        egui::CollapsingHeader::new("Position & shape").default_open(false).show(ui, |ui| {
        let fields: &[(&str, &str, &str)] = match element.tag.as_str() {
            "rect" => &[("X", "x", "0"), ("Y", "y", "0"), ("Width", "width", "100"), ("Height", "height", "50"), ("Corner X", "rx", "0"), ("Corner Y", "ry", element.value("rx", "0"))],
            "text" | "use" => &[("X", "x", "0"), ("Y", "y", "0")],
            "circle" => &[("Center X", "cx", "0"), ("Center Y", "cy", "0"), ("Radius", "r", "20")],
            "ellipse" => &[("Center X", "cx", "0"), ("Center Y", "cy", "0"), ("Radius X", "rx", "20"), ("Radius Y", "ry", "10")],
            "line" => &[("Start X", "x1", "0"), ("Start Y", "y1", "0"), ("End X", "x2", "100"), ("End Y", "y2", "50")],
            _ => &[],
        };
        for pair in fields.chunks(2) {
            ui.columns(2, |columns| { for ((label, key, default), column) in pair.iter().zip(columns.iter_mut()) { focused |= field(column, element, label, key, default, &mut changes); } });
        }
        if matches!(element.tag.as_str(), "polygon" | "polyline") {
            let values = element.value("points", "").split(|c: char| c.is_whitespace() || c == ',').filter(|v| !v.is_empty()).map(str::parse::<f32>).collect::<Result<Vec<_>, _>>();
            if let Ok(values) = values
                && values.len().is_multiple_of(2) && values.len() <= 128 {
                egui::CollapsingHeader::new("Points").show(ui, |ui| {
                    let mut values = values.clone();
                    let mut changed = false;
                    let mut remove = None;
                    for (index, pair) in values.chunks_exact_mut(2).enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(format!("{}", index+1));
                            for value in pair {
                                let r = ui.add(egui::DragValue::new(value).speed(0.5));
                                focused |= r.has_focus(); changed |= r.changed();
                            }
                            if values_len_allows_remove(element) && ui.small_button("−").clicked() { remove = Some(index); }
                        });
                    }
                    if let Some(index) = remove { values.drain(index*2..index*2+2); changed = true; }
                    if ui.small_button("Add point").clicked() && values.len() < 128 {
                        let x = values.get(values.len().saturating_sub(2)).copied().unwrap_or(0.)+10.;
                        let y = values.last().copied().unwrap_or(0.)+10.;
                        values.extend([x, y]); changed = true;
                    }
                    if changed { changes.push(("points".into(), values.iter().map(ToString::to_string).collect::<Vec<_>>().join(" "))); }
                });
            }
        }
        if fields.is_empty() { ui.label(RichText::new("Drag to move; use the corner handles to resize. Path points remain available in SVG source.").small().color(theme::muted())); }
        });
        egui::CollapsingHeader::new("Rotate, flip & align").show(ui, |ui| {
        if let Some((_, _, geometries)) = state.geometry(source)
            && let Some(g) = geometries.get(state.selected).copied().flatten() {
            ui.horizontal(|ui| {
                ui.label("Rotate by");
                focused |= ui.add(egui::DragValue::new(&mut state.rotation).speed(1.).suffix("°").range(-360. ..=360.)).has_focus();
                if ui.button("Apply rotation").clicked() {
                    let [x, y] = g.local_center;
                    let result = svg_edit::transform(source, state.selected, &format!("rotate({} {x} {y})", state.rotation));
                    state.apply(source, result);
                }
            });
            ui.horizontal_wrapped(|ui| {
                for (label, angle) in [("↶ 15°", -15.), ("↷ 15°", 15.)] {
                    if ui.button(label).clicked() {
                        let [x, y] = g.local_center;
                        let result = svg_edit::transform(source, state.selected, &format!("rotate({angle} {x} {y})"));
                        state.apply(source, result);
                    }
                }
                for (label, factors) in [("Flip H", [-1., 1.]), ("Flip V", [1., -1.])] {
                    if ui.button(label).clicked() {
                        let [x, y, w, h] = g.bounds;
                        let result = svg_edit::scale(source, state.selected, g, [x+w/2., y+h/2.], factors);
                        state.apply(source, result);
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                for (label, dx, dy) in [("Center H", true, false), ("Center V", false, true)] {
                    if ui.small_button(label).clicked()
                        && let Some((_, size, _)) = state.geometry(source) {
                        let [x, y, w, h] = g.bounds;
                        let result = svg_edit::translate(source, state.selected, g, if dx { size[0]/2.-x-w/2. } else { 0. }, if dy { size[1]/2.-y-h/2. } else { 0. });
                        state.apply(source, result);
                    }
                }
            });
        }
        });
        });
        ui.add_space(8.);
        ui::section(ui, "Appearance", |ui| {
        focused |= paint(ui, element, "Background / fill", "fill", "black", &mut changes);
        focused |= paint(ui, element, "Outline", "stroke", "none", &mut changes);
        focused |= field(ui, element, "Outline width", "stroke-width", "1", &mut changes);
        choice(ui, element, "Outline style", "stroke-dasharray", "none", &[("Solid", "none"), ("Dashed", "6 4"), ("Dotted", "1 3")], &mut changes);
        choice(ui, element, "Line ends", "stroke-linecap", "butt", &[("Flat", "butt"), ("Round", "round"), ("Square", "square")], &mut changes);
        choice(ui, element, "Joins", "stroke-linejoin", "miter", &[("Sharp", "miter"), ("Round", "round"), ("Bevel", "bevel")], &mut changes);
        for (label, key) in [("Fill opacity", "fill-opacity"), ("Outline opacity", "stroke-opacity")] {
            let mut alpha = element.value(key, "1").parse::<f32>().unwrap_or(1.)*100.;
            if ui.add(egui::Slider::new(&mut alpha, 0. ..=100.).text(label).suffix("%")).changed() { changes.push((key.into(), (alpha/100.).to_string())); }
        }
        let mut opacity = element.value("opacity", "1").parse::<f32>().unwrap_or(1.)*100.;
        if ui.add(egui::Slider::new(&mut opacity, 0. ..=100.).text("Object opacity").suffix("%")).changed() { changes.push(("opacity".into(), (opacity/100.).to_string())); }
        });
    });
    if !changes.is_empty() || text.is_some() {
        let result = svg_edit::update(source, state.selected, &changes, text.as_deref());
        state.apply(source, result);
    }
    focused
}

pub(super) fn canvas(
    ui: &mut egui::Ui,
    source: &mut String,
    png: Option<Arc<Vec<u8>>>,
    state: &mut State,
) {
    ui.horizontal(|ui| {
        ui::caption(ui, "Preview");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut grid = !state.solid_preview;
            if ui.checkbox(&mut grid, "Transparency grid").changed() {
                state.solid_preview = !grid;
            }
        });
    });
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), 390.),
        egui::Sense::click_and_drag(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 8., Color32::from_gray(246));
    let Some((_, size, geometries)) = state.geometry(source) else {
        return;
    };
    let scale = ((rect.width() - 28.) / size[0]).min((rect.height() - 28.) / size[1]);
    let art = Rect::from_center_size(rect.center(), vec2(size[0], size[1]) * scale);
    painter.rect_filled(art, 0., Color32::WHITE);
    if !state.solid_preview {
        let checker = painter.with_clip_rect(art);
        let cell = 12.;
        for row in 0..(art.height() / cell).ceil() as i32 {
            for column in 0..(art.width() / cell).ceil() as i32 {
                if (row + column) % 2 == 0 {
                    checker.rect_filled(
                        Rect::from_min_size(
                            art.min + vec2(column as f32 * cell, row as f32 * cell),
                            vec2(cell, cell),
                        ),
                        0.,
                        Color32::from_gray(238),
                    );
                }
            }
        }
    }

    if let Some(png) = png {
        let key = mypdf_milestone0::identity::fingerprint(&png);
        if state.texture.as_ref().is_none_or(|t| t.0 != key)
            && let Ok(image) = mypdf_milestone0::assets::decode_png(&png)
        {
            let image = image.to_rgba8();
            let color = egui::ColorImage::from_rgba_unmultiplied(
                [image.width() as usize, image.height() as usize],
                image.as_raw(),
            );
            state.texture = Some((
                key,
                ui.ctx()
                    .load_texture("svg_studio_preview", color, egui::TextureOptions::LINEAR),
            ));
        }
        if let Some((_, texture)) = &state.texture {
            painter.image(
                texture.id(),
                art,
                Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)),
                Color32::WHITE,
            );
        }
    }
    let screen = |g: Geometry| {
        Rect::from_min_size(
            art.min + vec2(g.bounds[0], g.bounds[1]) * scale,
            vec2(g.bounds[2], g.bounds[3]) * scale,
        )
    };
    let corners = |r: Rect| {
        [
            r.left_top(),
            r.right_top(),
            r.right_bottom(),
            r.left_bottom(),
        ]
    };
    if !state.source_mode {
        if let Some(g) = geometries.get(state.selected).copied().flatten() {
            let r = screen(g);
            painter.rect_stroke(
                r,
                0.,
                Stroke::new(1.5, theme::accent()),
                egui::StrokeKind::Outside,
            );
            for point in corners(r) {
                painter.circle_filled(point, 4., Color32::WHITE);
                painter.circle_stroke(point, 4., Stroke::new(1.5, theme::accent()));
            }
        }
        if response.clicked() || response.drag_started() {
            state.border_tab = false;
            let point = if response.drag_started() {
                ui.input(|i| i.pointer.press_origin())
            } else {
                response.interact_pointer_pos()
            };
            if let Some(point) = point {
                let corner = geometries
                    .get(state.selected)
                    .copied()
                    .flatten()
                    .and_then(|g| {
                        corners(screen(g))
                            .iter()
                            .position(|p| p.distance(point) < 10.)
                    });
                if corner.is_none()
                    && let Some(index) =
                        geometries.iter().enumerate().rev().find_map(|(index, g)| {
                            g.filter(|g| screen(*g).expand(4.).contains(point))
                                .map(|_| index)
                        })
                {
                    state.selected = index;
                }
                if response.drag_started()
                    && let Some(g) = geometries.get(state.selected).copied().flatten()
                    && (corner.is_some() || screen(g).expand(4.).contains(point))
                {
                    state.checkpoint(source.clone());
                    state.drag = Some(Drag {
                        source: source.clone(),
                        geometry: g,
                        start: point,
                        corner,
                    });
                    state.last_edit = None;
                }
            }
        }
        if response.dragged()
            && let Some(drag) = &state.drag
            && let Some(point) = response.interact_pointer_pos()
        {
            let delta = (point - drag.start) / scale;
            let result = if let Some(corner) = drag.corner {
                let [x, y, w, h] = drag.geometry.bounds;
                let left = corner == 0 || corner == 3;
                let top = corner < 2;
                let sx =
                    ((w + if left { -delta.x } else { delta.x }) / w.max(0.01)).clamp(0.02, 100.);
                let mut sy =
                    ((h + if top { -delta.y } else { delta.y }) / h.max(0.01)).clamp(0.02, 100.);
                let sx = if w.abs() < 0.01 { 1. } else { sx };
                if h.abs() < 0.01 {
                    sy = 1.;
                }
                if ui.input(|i| i.modifiers.shift) {
                    sy = sx;
                }
                svg_edit::scale(
                    &drag.source,
                    state.selected,
                    drag.geometry,
                    [if left { x + w } else { x }, if top { y + h } else { y }],
                    [sx, sy],
                )
            } else {
                svg_edit::translate(
                    &drag.source,
                    state.selected,
                    drag.geometry,
                    delta.x,
                    delta.y,
                )
            };
            state.apply(source, result);
        }
        if response.drag_stopped() {
            state.drag = None;
            state.restored = true;
        }
    }
    ui.label(
        RichText::new("Click an object to select · Drag to move · Corners to resize · Shift keeps proportions")
            .small()
            .color(theme::muted()),
    );
    if let Some(error) = &state.error {
        ui.colored_label(theme::danger(), error);
    }
}
