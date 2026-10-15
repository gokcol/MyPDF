//! MyPDF design system: native fonts, gradient fills, vector icons and controls.
//! Everything is drawn procedurally, so it stays crisp at any display scale, and
//! every control carries an accessible text label.
use super::*;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Typography

/// Inner padding for text fields, so they match the height of buttons and menus.
pub(super) const FIELD_MARGIN: egui::Vec2 = egui::vec2(8., 6.);

pub(super) const SANS: &str = "ui-sans";
pub(super) const STRONG: &str = "ui-strong";
pub(super) const BOLD: &str = "ui-bold";

type Face = Option<(&'static [u8], u32)>;
struct UiFaces {
    sans: Face,
    strong: Face,
    bold: Face,
    /// Broad Unicode coverage, so text in any script previews while editing.
    fallback: Face,
}

/// System UI faces, read once per process and shared by every egui context.
fn ui_faces() -> &'static UiFaces {
    static FACES: OnceLock<UiFaces> = OnceLock::new();
    FACES.get_or_init(|| {
        let mut files: Vec<(&str, &'static [u8])> = vec![];
        let mut face = |candidates: &[(&'static str, u32)]| -> Face {
            candidates.iter().find_map(|&(path, index)| {
                if let Some((_, bytes)) = files.iter().find(|(p, _)| *p == path) {
                    return Some((*bytes, index));
                }
                let bytes: &'static [u8] = Box::leak(std::fs::read(path).ok()?.into_boxed_slice());
                files.push((path, bytes));
                Some((bytes, index))
            })
        };
        UiFaces {
            sans: face(&[
                ("/System/Library/Fonts/SFNS.ttf", 0),
                ("C:\\Windows\\Fonts\\segoeui.ttf", 0),
            ]),
            strong: face(&[
                ("/System/Library/Fonts/HelveticaNeue.ttc", 10),
                ("C:\\Windows\\Fonts\\seguisb.ttf", 0),
            ]),
            bold: face(&[
                ("/System/Library/Fonts/HelveticaNeue.ttc", 1),
                ("C:\\Windows\\Fonts\\segoeuib.ttf", 0),
            ]),
            fallback: face(&[
                ("/System/Library/Fonts/Supplemental/Arial Unicode.ttf", 0),
                ("/Library/Fonts/Arial Unicode.ttf", 0),
                ("C:\\Windows\\Fonts\\arialuni.ttf", 0),
                ("C:\\Windows\\Fonts\\msyh.ttc", 0),
                ("/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf", 0),
                ("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 0),
            ]),
        }
    })
}

/// Prefer the platform UI font; egui's bundled fonts remain as glyph fallbacks.
/// The strong/bold families always exist, falling back to the regular face.
pub(super) fn add_ui_fonts(fonts: &mut egui::FontDefinitions) {
    let faces = ui_faces();
    let defaults = fonts.families[&egui::FontFamily::Proportional].clone();
    let mut install = |name: &str, face: Face| {
        face.map(|(bytes, index)| {
            let mut data = egui::FontData::from_static(bytes);
            data.index = index;
            fonts.font_data.insert(name.into(), Arc::new(data));
            name.to_owned()
        })
    };
    let sans = install(SANS, faces.sans);
    let strong_face = install(STRONG, faces.strong);
    let bold_face = install(BOLD, faces.bold);
    let chain = |first: &[&Option<String>]| -> Vec<String> {
        first
            .iter()
            .filter_map(|n| (*n).clone())
            .chain(defaults.iter().cloned())
            .collect()
    };
    let regular = chain(&[&sans]);
    let strong = chain(&[&strong_face, &sans]);
    let bold = chain(&[&bold_face, &strong_face, &sans]);
    fonts
        .families
        .insert(egui::FontFamily::Proportional, regular);
    fonts
        .families
        .insert(egui::FontFamily::Name(STRONG.into()), strong);
    fonts
        .families
        .insert(egui::FontFamily::Name(BOLD.into()), bold);
    // Every family, including the PDF text faces, falls back to broad coverage.
    if let Some((bytes, index)) = faces.fallback {
        let mut data = egui::FontData::from_static(bytes);
        data.index = index;
        fonts.font_data.insert("ui-fallback".into(), Arc::new(data));
        for chain in fonts.families.values_mut() {
            chain.push("ui-fallback".into());
        }
    }
}

/// A font from one of the UI families, or the default face before fonts are installed.
pub(super) fn font(ctx: &egui::Context, family: &str, size: f32) -> FontId {
    let family = egui::FontFamily::Name(family.into());
    if ctx.fonts(|f| f.definitions().families.contains_key(&family)) {
        FontId::new(size, family)
    } else {
        FontId::proportional(size)
    }
}
/// The semibold family, for `RichText::family`. Always registered by [`add_ui_fonts`].
pub(super) fn semibold() -> egui::FontFamily {
    egui::FontFamily::Name(STRONG.into())
}
pub(super) fn strong(ui: &egui::Ui, size: f32) -> FontId {
    font(ui.ctx(), STRONG, size)
}
pub(super) fn bold(ui: &egui::Ui, size: f32) -> FontId {
    font(ui.ctx(), BOLD, size)
}
/// Panel and dialog headings.
pub(super) fn title(ui: &mut egui::Ui, text: &str, size: f32) -> egui::Response {
    let font = strong(ui, size);
    ui.label(RichText::new(text).font(font).color(theme::ink()))
}
/// Small uppercase section label used in inspectors.
pub(super) fn caption(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let font = strong(ui, 10.5);
    ui.label(
        RichText::new(text.to_uppercase())
            .font(font)
            .color(theme::muted())
            .extra_letter_spacing(0.6),
    )
}

// ---------------------------------------------------------------------------
// Fills, shadows and badges

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Fill {
    /// White to soft gray, top to bottom: multiplied by a tint for glossy buttons.
    Sheen,
    /// The MyPDF blue-to-violet brand gradient.
    Brand,
    /// Coral to crimson for destructive emphasis.
    Danger,
}
fn gradient_texture(ctx: &egui::Context, fill: Fill) -> egui::TextureId {
    let id = egui::Id::new(("mypdf-gradient", fill as u8));
    if let Some(texture) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return texture.id();
    }
    const SIZE: usize = 64;
    let lerp = |a: [f32; 3], b: [f32; 3], t: f32| -> [u8; 4] {
        let c = |i: usize| (a[i] + (b[i] - a[i]) * t).round() as u8;
        [c(0), c(1), c(2), 255]
    };
    let mut rgba = Vec::with_capacity(SIZE * SIZE * 4);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let diagonal = (x + y) as f32 / (2 * (SIZE - 1)) as f32;
            let vertical = y as f32 / (SIZE - 1) as f32;
            rgba.extend(match fill {
                Fill::Sheen => lerp([255.; 3], [218.; 3], vertical),
                Fill::Brand => lerp([64., 140., 255.], [128., 84., 250.], diagonal),
                Fill::Danger => lerp([255., 110., 96.], [214., 40., 64.], diagonal),
            });
        }
    }
    let texture = ctx.load_texture(
        format!("mypdf-gradient-{}", fill as u8),
        egui::ColorImage::from_rgba_unmultiplied([SIZE, SIZE], &rgba),
        egui::TextureOptions::LINEAR,
    );
    let texture_id = texture.id();
    ctx.data_mut(|d| d.insert_temp(id, texture));
    texture_id
}
/// A rounded rectangle filled with a gradient, multiplied by `tint`.
pub(super) fn gradient_rect(
    painter: &egui::Painter,
    rect: Rect,
    radius: impl Into<egui::CornerRadius>,
    fill: Fill,
    tint: Color32,
) {
    painter.add(
        egui::epaint::RectShape::filled(rect, radius, tint).with_texture(
            gradient_texture(painter.ctx(), fill),
            Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)),
        ),
    );
}
/// Soft ambient shadow under a card. Higher elevation spreads further.
pub(super) fn shadow(
    painter: &egui::Painter,
    rect: Rect,
    radius: impl Into<egui::CornerRadius>,
    elevation: u8,
) {
    let e = elevation.clamp(1, 4);
    painter.add(
        egui::epaint::Shadow {
            offset: [0, (e * e + 1) as i8],
            blur: 3 + e * 7,
            spread: 0,
            color: Color32::from_black_alpha(14 + e * 6),
        }
        .as_shape(rect, radius),
    );
}
/// An app-icon style tile: a gradient rounded square with a white glyph.
pub(super) fn badge(painter: &egui::Painter, rect: Rect, icon: Icon, fill: Fill, tint: Color32) {
    let radius = (rect.width() * 0.28) as u8;
    shadow(painter, rect, radius, 1);
    gradient_rect(painter, rect, radius, fill, tint);
    draw_icon(
        painter,
        Rect::from_center_size(rect.center(), rect.size() * 0.54),
        icon,
        Color32::WHITE,
    );
}
/// The MyPDF mark: a folded white page with a highlighted line on the brand gradient.
pub(super) fn logo(painter: &egui::Painter, rect: Rect) {
    let radius = (rect.width() * 0.26) as u8;
    gradient_rect(painter, rect, radius, Fill::Brand, Color32::WHITE);
    let p = |x: f32, y: f32| rect.min + vec2(x, y) * rect.size();
    let sheet = vec![
        p(0.3, 0.2),
        p(0.6, 0.2),
        p(0.74, 0.34),
        p(0.74, 0.8),
        p(0.3, 0.8),
    ];
    painter.add(egui::Shape::convex_polygon(
        sheet,
        Color32::WHITE,
        Stroke::NONE,
    ));
    painter.add(egui::Shape::convex_polygon(
        vec![p(0.6, 0.2), p(0.74, 0.34), p(0.6, 0.34)],
        Color32::from_rgb(204, 214, 255),
        Stroke::NONE,
    ));
    let w = rect.height() * 0.055;
    painter.rect_filled(
        Rect::from_min_max(p(0.36, 0.45), p(0.68, 0.53)),
        1.,
        Color32::from_rgb(255, 214, 64),
    );
    for (y, x) in [(0.62, 0.68), (0.71, 0.58)] {
        painter.line_segment(
            [p(0.37, y), p(x, y)],
            Stroke::new(w, Color32::from_rgb(150, 160, 230)),
        );
    }
}

// ---------------------------------------------------------------------------
// Icons

#[derive(Clone, Copy)]
pub(super) enum Icon {
    Home,
    Blank,
    Document,
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
    Sparkle,
    Plus,
    Minus,
    Left,
    Right,
    Import,
    Export,
    Crop,
    Extract,
    Split,
    Trash,
    Close,
    Copy,
    Theme,
    Lasso,
    AlignObjectsLeft,
    AlignObjectsCenter,
    AlignObjectsRight,
    AlignObjectsTop,
    AlignObjectsMiddle,
    AlignObjectsBottom,
    Check,
    Warning,
    Info,
    HeaderFooter,
    History,
    Redact,
    FieldText,
    FieldCheck,
    FieldRadio,
    FieldDropdown,
    FieldList,
    FieldButton,
    FieldImage,
    FieldDate,
    TabOrder,
    Flatten,
}

pub(super) fn draw_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let p = |x: f32, y: f32| rect.min + vec2(x, y) * rect.size();
    let width = (rect.width() * 0.082).clamp(1.3, 3.0);
    let stroke = Stroke::new(width, color);
    // Round caps and joins: a dot under every vertex hides mitred corners.
    let line = |points: &[[f32; 2]]| {
        let points: Vec<_> = points.iter().map(|v| p(v[0], v[1])).collect();
        for point in &points {
            painter.circle_filled(*point, width / 2., color);
        }
        painter.add(egui::Shape::line(points, stroke));
    };
    let arc = |center: [f32; 2], radius: f32, from: f32, to: f32| -> Vec<[f32; 2]> {
        let steps = ((to - from).abs() / 12.).ceil().max(2.) as usize;
        (0..=steps)
            .map(|i| {
                let a = (from + (to - from) * i as f32 / steps as f32).to_radians();
                [center[0] + a.cos() * radius, center[1] + a.sin() * radius]
            })
            .collect()
    };
    let box_at = |a: [f32; 2], b: [f32; 2], radius: f32| {
        painter.rect_stroke(
            Rect::from_min_max(p(a[0], a[1]), p(b[0], b[1])),
            radius * rect.width(),
            stroke,
            egui::StrokeKind::Middle,
        );
    };
    let fill_box = |a: [f32; 2], b: [f32; 2], radius: f32, c: Color32| {
        painter.rect_filled(
            Rect::from_min_max(p(a[0], a[1]), p(b[0], b[1])),
            radius * rect.width(),
            c,
        );
    };
    let soft = color.gamma_multiply(0.22);
    let page = |lines: bool| {
        line(&[
            [0.62, 0.08],
            [0.2, 0.08],
            [0.2, 0.92],
            [0.8, 0.92],
            [0.8, 0.26],
            [0.62, 0.08],
        ]);
        line(&[[0.62, 0.08], [0.62, 0.26], [0.8, 0.26]]);
        if lines {
            line(&[[0.34, 0.5], [0.66, 0.5]]);
            line(&[[0.34, 0.68], [0.58, 0.68]]);
        }
    };
    match icon {
        Icon::HeaderFooter => {
            box_at([0.18, 0.08], [0.82, 0.92], 0.06);
            fill_box([0.3, 0.17], [0.7, 0.27], 0.02, color);
            line(&[[0.3, 0.45], [0.7, 0.45]]);
            line(&[[0.3, 0.57], [0.58, 0.57]]);
            fill_box([0.3, 0.73], [0.7, 0.83], 0.02, color);
        }
        Icon::FieldText => {
            box_at([0.08, 0.28], [0.92, 0.72], 0.08);
            line(&[[0.24, 0.4], [0.24, 0.6]]);
            line(&[[0.36, 0.5], [0.62, 0.5]]);
        }
        Icon::FieldCheck => {
            box_at([0.16, 0.16], [0.84, 0.84], 0.1);
            line(&[[0.3, 0.5], [0.45, 0.65], [0.7, 0.36]]);
        }
        Icon::FieldRadio => {
            painter.circle_stroke(rect.center(), rect.width() * 0.36, stroke);
            painter.circle_filled(rect.center(), rect.width() * 0.16, color);
        }
        Icon::FieldDropdown => {
            box_at([0.08, 0.28], [0.92, 0.72], 0.08);
            line(&[[0.24, 0.5], [0.5, 0.5]]);
            line(&[[0.64, 0.44], [0.72, 0.56], [0.8, 0.44]]);
        }
        Icon::FieldList => {
            box_at([0.12, 0.1], [0.88, 0.9], 0.08);
            line(&[[0.26, 0.3], [0.7, 0.3]]);
            fill_box([0.2, 0.42], [0.8, 0.58], 0.03, soft);
            line(&[[0.26, 0.5], [0.7, 0.5]]);
            line(&[[0.26, 0.7], [0.62, 0.7]]);
        }
        Icon::FieldButton => {
            fill_box([0.08, 0.28], [0.92, 0.72], 0.14, soft);
            box_at([0.08, 0.28], [0.92, 0.72], 0.14);
            line(&[[0.34, 0.5], [0.66, 0.5]]);
        }
        Icon::FieldImage => {
            box_at([0.1, 0.16], [0.9, 0.84], 0.08);
            line(&[
                [0.18, 0.76],
                [0.42, 0.48],
                [0.58, 0.64],
                [0.68, 0.54],
                [0.84, 0.74],
            ]);
            painter.circle_filled(p(0.68, 0.34), rect.width() * 0.07, color);
        }
        Icon::FieldDate => {
            box_at([0.14, 0.2], [0.86, 0.88], 0.08);
            line(&[[0.14, 0.4], [0.86, 0.4]]);
            line(&[[0.34, 0.1], [0.34, 0.28]]);
            line(&[[0.66, 0.1], [0.66, 0.28]]);
            for (x, y) in [
                (0.34, 0.56),
                (0.5, 0.56),
                (0.66, 0.56),
                (0.34, 0.72),
                (0.5, 0.72),
            ] {
                painter.circle_filled(p(x, y), width * 0.75, color);
            }
        }
        Icon::TabOrder => {
            line(&[[0.12, 0.3], [0.84, 0.3]]);
            line(&[[0.72, 0.2], [0.84, 0.3], [0.72, 0.4]]);
            line(&[[0.12, 0.7], [0.84, 0.7]]);
            line(&[[0.72, 0.6], [0.84, 0.7], [0.72, 0.8]]);
        }
        Icon::Flatten => {
            line(&[
                [0.5, 0.08],
                [0.88, 0.28],
                [0.5, 0.48],
                [0.12, 0.28],
                [0.5, 0.08],
            ]);
            line(&[[0.12, 0.5], [0.5, 0.7], [0.88, 0.5]]);
            line(&[[0.12, 0.72], [0.5, 0.92], [0.88, 0.72]]);
        }
        Icon::Redact => {
            line(&[[0.12, 0.26], [0.88, 0.26]]);
            fill_box([0.1, 0.42], [0.9, 0.62], 0.03, color);
            line(&[[0.12, 0.78], [0.62, 0.78]]);
        }
        Icon::History => {
            line(&arc([0.5, 0.52], 0.36, 0., 360.));
            line(&[[0.5, 0.31], [0.5, 0.52], [0.65, 0.62]]);
        }
        Icon::Reload => {
            line(&arc([0.5, 0.52], 0.32, -60., 230.));
            line(&[[0.6, 0.1], [0.68, 0.25], [0.52, 0.3]]);
        }
        Icon::TextArea => {
            for (a, b, c) in [
                ([0.1, 0.3], [0.1, 0.1], [0.3, 0.1]),
                ([0.7, 0.1], [0.9, 0.1], [0.9, 0.3]),
                ([0.9, 0.7], [0.9, 0.9], [0.7, 0.9]),
                ([0.3, 0.9], [0.1, 0.9], [0.1, 0.7]),
            ] {
                line(&[a, b, c]);
            }
            line(&[[0.3, 0.36], [0.7, 0.36]]);
            line(&[[0.3, 0.52], [0.7, 0.52]]);
            line(&[[0.3, 0.68], [0.56, 0.68]]);
        }
        Icon::Home => {
            line(&[[0.08, 0.46], [0.5, 0.1], [0.92, 0.46]]);
            line(&[[0.2, 0.36], [0.2, 0.88], [0.8, 0.88], [0.8, 0.36]]);
            fill_box([0.41, 0.6], [0.59, 0.88], 0.04, color);
        }
        Icon::Blank => {
            page(false);
            line(&[[0.36, 0.6], [0.64, 0.6]]);
            line(&[[0.5, 0.46], [0.5, 0.74]]);
        }
        Icon::Document => page(true),
        Icon::Bold | Icon::Italic => {
            let bold = matches!(icon, Icon::Bold);
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                if bold { "B" } else { "I" },
                font(
                    painter.ctx(),
                    if bold { BOLD } else { SANS },
                    rect.height() * 0.85,
                ),
                color,
            );
        }
        Icon::AlignLeft | Icon::AlignCenter | Icon::AlignRight | Icon::AlignJustify => {
            for i in 0..4 {
                let y = 0.2 + i as f32 * 0.2;
                let w = if i % 2 == 0 || matches!(icon, Icon::AlignJustify) {
                    0.8
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
            let pts = [[0.14, 0.8], [0.3, 0.18], [0.84, 0.3], [0.7, 0.84]];
            if matches!(icon, Icon::Polygon) {
                painter.add(egui::Shape::convex_polygon(
                    pts.iter().map(|v| p(v[0], v[1])).collect(),
                    soft,
                    Stroke::NONE,
                ));
                line(&[pts[3], pts[0]]);
            }
            line(&pts);
            for v in pts {
                painter.circle_filled(p(v[0], v[1]), width * 1.6, color);
            }
        }
        Icon::DoubleArrow => {
            line(&[[0.16, 0.84], [0.84, 0.16]]);
            line(&[[0.14, 0.5], [0.14, 0.86], [0.5, 0.86]]);
            line(&[[0.5, 0.14], [0.86, 0.14], [0.86, 0.5]]);
        }
        Icon::Crop => {
            line(&[[0.26, 0.06], [0.26, 0.74], [0.94, 0.74]]);
            line(&[[0.06, 0.26], [0.74, 0.26], [0.74, 0.94]]);
        }
        Icon::Extract => {
            line(&[
                [0.6, 0.28],
                [0.6, 0.1],
                [0.12, 0.1],
                [0.12, 0.9],
                [0.6, 0.9],
                [0.6, 0.72],
            ]);
            line(&[[0.38, 0.5], [0.94, 0.5]]);
            line(&[[0.78, 0.33], [0.94, 0.5], [0.78, 0.67]]);
        }
        Icon::Export => {
            line(&[
                [0.34, 0.36],
                [0.16, 0.36],
                [0.16, 0.92],
                [0.84, 0.92],
                [0.84, 0.36],
                [0.66, 0.36],
            ]);
            line(&[[0.5, 0.6], [0.5, 0.06]]);
            line(&[[0.33, 0.22], [0.5, 0.06], [0.67, 0.22]]);
        }
        Icon::Split => {
            line(&[[0.5, 0.92], [0.5, 0.56], [0.2, 0.26], [0.2, 0.1]]);
            line(&[[0.5, 0.56], [0.8, 0.26], [0.8, 0.1]]);
            line(&[[0.07, 0.23], [0.2, 0.1], [0.33, 0.23]]);
            line(&[[0.67, 0.23], [0.8, 0.1], [0.93, 0.23]]);
        }
        Icon::Trash => {
            line(&[[0.24, 0.3], [0.29, 0.9], [0.71, 0.9], [0.76, 0.3]]);
            line(&[[0.12, 0.26], [0.88, 0.26]]);
            line(&[[0.38, 0.26], [0.4, 0.1], [0.6, 0.1], [0.62, 0.26]]);
            line(&[[0.43, 0.44], [0.44, 0.74]]);
            line(&[[0.57, 0.44], [0.56, 0.74]]);
        }
        Icon::Open => {
            line(&[
                [0.08, 0.8],
                [0.08, 0.2],
                [0.38, 0.2],
                [0.48, 0.32],
                [0.84, 0.32],
                [0.84, 0.44],
            ]);
            painter.add(egui::Shape::convex_polygon(
                [[0.08, 0.8], [0.22, 0.44], [0.96, 0.44], [0.8, 0.8]]
                    .iter()
                    .map(|v| p(v[0], v[1]))
                    .collect(),
                soft,
                Stroke::NONE,
            ));
            line(&[
                [0.08, 0.8],
                [0.22, 0.44],
                [0.96, 0.44],
                [0.8, 0.8],
                [0.08, 0.8],
            ]);
        }
        Icon::Save => {
            line(&[
                [0.14, 0.12],
                [0.7, 0.12],
                [0.88, 0.3],
                [0.88, 0.88],
                [0.14, 0.88],
                [0.14, 0.12],
            ]);
            line(&[[0.32, 0.12], [0.32, 0.34], [0.64, 0.34], [0.64, 0.12]]);
            fill_box([0.3, 0.56], [0.72, 0.88], 0.04, soft);
            box_at([0.3, 0.56], [0.72, 0.88], 0.04);
        }
        Icon::Search => {
            painter.circle_stroke(p(0.43, 0.43), rect.width() * 0.29, stroke);
            line(&[[0.65, 0.65], [0.9, 0.9]]);
        }
        Icon::Sidebar | Icon::Inspector => {
            let x = if matches!(icon, Icon::Sidebar) {
                0.38
            } else {
                0.62
            };
            let (a, b) = if matches!(icon, Icon::Sidebar) {
                ([0.08, 0.16], [x, 0.84])
            } else {
                ([x, 0.16], [0.92, 0.84])
            };
            fill_box(a, b, 0.08, soft);
            box_at([0.08, 0.16], [0.92, 0.84], 0.1);
            line(&[[x, 0.16], [x, 0.84]]);
        }
        Icon::Shapes => {
            fill_box([0.08, 0.08], [0.56, 0.56], 0.06, soft);
            box_at([0.08, 0.08], [0.56, 0.56], 0.06);
            painter.circle_stroke(p(0.65, 0.65), rect.width() * 0.27, stroke);
        }
        Icon::Form => {
            box_at([0.12, 0.08], [0.88, 0.92], 0.1);
            fill_box([0.26, 0.26], [0.4, 0.4], 0.03, color);
            line(&[[0.5, 0.33], [0.74, 0.33]]);
            box_at([0.26, 0.6], [0.4, 0.74], 0.03);
            line(&[[0.5, 0.67], [0.74, 0.67]]);
        }
        Icon::Read => {
            line(&[
                [0.5, 0.84],
                [0.1, 0.76],
                [0.1, 0.16],
                [0.5, 0.25],
                [0.9, 0.16],
                [0.9, 0.76],
                [0.5, 0.84],
                [0.5, 0.25],
            ]);
        }
        Icon::Pen => {
            line(&[
                [0.16, 0.84],
                [0.22, 0.6],
                [0.68, 0.14],
                [0.86, 0.32],
                [0.4, 0.78],
                [0.16, 0.84],
            ]);
            line(&[[0.58, 0.24], [0.76, 0.42]]);
        }
        Icon::Sign => {
            line(&[
                [0.34, 0.6],
                [0.4, 0.42],
                [0.72, 0.1],
                [0.86, 0.24],
                [0.54, 0.56],
                [0.34, 0.6],
            ]);
            line(
                &(0..=16)
                    .map(|i| {
                        let t = i as f32 / 16.;
                        [0.08 + t * 0.84, 0.84 + (t * 12.).sin() * 0.045 * (1. - t)]
                    })
                    .collect::<Vec<_>>(),
            );
        }
        Icon::Shield => {
            let outline = [
                [0.5, 0.06],
                [0.86, 0.2],
                [0.84, 0.56],
                [0.5, 0.94],
                [0.16, 0.56],
                [0.14, 0.2],
            ];
            painter.add(egui::Shape::convex_polygon(
                outline.iter().map(|v| p(v[0], v[1])).collect(),
                soft,
                Stroke::NONE,
            ));
            let mut closed = outline.to_vec();
            closed.push(outline[0]);
            line(&closed);
            line(&[[0.34, 0.5], [0.46, 0.62], [0.67, 0.38]]);
        }
        Icon::Library => {
            for (x, y) in [(0.1, 0.1), (0.56, 0.1), (0.1, 0.56), (0.56, 0.56)] {
                box_at([x, y], [x + 0.34, y + 0.34], 0.07);
            }
            fill_box([0.56, 0.56], [0.9, 0.9], 0.07, soft);
        }
        Icon::Undo | Icon::Redo => {
            let flip = |v: [f32; 2]| {
                if matches!(icon, Icon::Redo) {
                    [1.0 - v[0], v[1]]
                } else {
                    v
                }
            };
            let mut path = vec![[0.14, 0.36]];
            path.extend(arc([0.58, 0.58], 0.22, -90., 90.));
            path.push([0.36, 0.8]);
            line(&path.into_iter().map(flip).collect::<Vec<_>>());
            line(&[flip([0.32, 0.18]), flip([0.14, 0.36]), flip([0.32, 0.54])]);
        }
        Icon::Select => {
            let outline = [
                [0.22, 0.08],
                [0.24, 0.86],
                [0.43, 0.67],
                [0.58, 0.94],
                [0.71, 0.87],
                [0.56, 0.6],
                [0.84, 0.6],
                [0.22, 0.08],
            ];
            line(&outline);
        }
        Icon::Highlight => {
            line(&[
                [0.2, 0.66],
                [0.54, 0.12],
                [0.84, 0.3],
                [0.5, 0.84],
                [0.2, 0.66],
            ]);
            line(&[[0.2, 0.66], [0.14, 0.8], [0.26, 0.86], [0.34, 0.75]]);
            line(&[[0.42, 0.34], [0.7, 0.52]]);
        }
        Icon::Text => {
            line(&[[0.16, 0.28], [0.16, 0.14], [0.84, 0.14], [0.84, 0.28]]);
            line(&[[0.5, 0.14], [0.5, 0.86]]);
            line(&[[0.36, 0.86], [0.64, 0.86]]);
        }
        Icon::Underline => {
            line(&[
                [0.26, 0.1],
                [0.26, 0.52],
                [0.34, 0.66],
                [0.5, 0.71],
                [0.66, 0.66],
                [0.74, 0.52],
                [0.74, 0.1],
            ]);
            line(&[[0.16, 0.9], [0.84, 0.9]]);
        }
        Icon::Strikeout => {
            line(&[
                [0.76, 0.22],
                [0.62, 0.12],
                [0.36, 0.12],
                [0.22, 0.24],
                [0.26, 0.4],
                [0.5, 0.5],
            ]);
            line(&[
                [0.6, 0.56],
                [0.76, 0.66],
                [0.74, 0.82],
                [0.62, 0.9],
                [0.36, 0.9],
                [0.22, 0.8],
            ]);
            line(&[[0.08, 0.52], [0.92, 0.52]]);
        }
        Icon::Note => {
            painter.add(egui::Shape::convex_polygon(
                [[0.12, 0.14], [0.88, 0.14], [0.88, 0.68], [0.12, 0.68]]
                    .iter()
                    .map(|v| p(v[0], v[1]))
                    .collect(),
                soft,
                Stroke::NONE,
            ));
            line(&[
                [0.48, 0.68],
                [0.88, 0.68],
                [0.88, 0.14],
                [0.12, 0.14],
                [0.12, 0.68],
                [0.28, 0.68],
                [0.26, 0.88],
                [0.48, 0.68],
            ]);
            line(&[[0.3, 0.34], [0.7, 0.34]]);
            line(&[[0.3, 0.5], [0.58, 0.5]]);
        }
        Icon::Rectangle => box_at([0.1, 0.2], [0.9, 0.8], 0.08),
        Icon::Circle => {
            painter.circle_stroke(rect.center(), rect.width() * 0.37, stroke);
        }
        Icon::Line => line(&[[0.14, 0.86], [0.86, 0.14]]),
        Icon::Arrow => {
            line(&[[0.14, 0.86], [0.84, 0.16]]);
            line(&[[0.44, 0.16], [0.84, 0.16], [0.84, 0.56]]);
        }
        Icon::Stamp => {
            painter.circle_stroke(p(0.5, 0.25), rect.width() * 0.14, stroke);
            line(&[[0.43, 0.39], [0.41, 0.56]]);
            line(&[[0.57, 0.39], [0.59, 0.56]]);
            fill_box([0.12, 0.56], [0.88, 0.74], 0.05, soft);
            box_at([0.12, 0.56], [0.88, 0.74], 0.05);
            line(&[[0.16, 0.88], [0.84, 0.88]]);
        }
        Icon::Star => {
            let mut pts: Vec<_> = (0..10)
                .map(|i| {
                    let a = i as f32 * std::f32::consts::PI / 5.0 - std::f32::consts::FRAC_PI_2;
                    let r = if i % 2 == 0 { 0.44 } else { 0.2 };
                    [0.5 + a.cos() * r, 0.53 + a.sin() * r]
                })
                .collect();
            pts.push(pts[0]);
            line(&pts);
        }
        Icon::Sparkle => {
            let star = |c: [f32; 2], r: f32| {
                let k = r * 0.24;
                let pts = vec![
                    p(c[0], c[1] - r),
                    p(c[0] + k, c[1] - k),
                    p(c[0] + r, c[1]),
                    p(c[0] + k, c[1] + k),
                    p(c[0], c[1] + r),
                    p(c[0] - k, c[1] + k),
                    p(c[0] - r, c[1]),
                    p(c[0] - k, c[1] - k),
                ];
                painter.add(egui::Shape::Path(egui::epaint::PathShape::closed_line(
                    pts.clone(),
                    stroke,
                )));
                // Concave outline: fill each quadrant as a triangle fan.
                for i in 0..4 {
                    painter.add(egui::Shape::convex_polygon(
                        vec![
                            p(c[0], c[1]),
                            pts[i * 2],
                            pts[i * 2 + 1],
                            pts[(i * 2 + 2) % 8],
                        ],
                        color,
                        Stroke::NONE,
                    ));
                }
            };
            star([0.42, 0.56], 0.34);
            star([0.78, 0.2], 0.14);
        }
        Icon::Plus | Icon::Minus => {
            line(&[[0.2, 0.5], [0.8, 0.5]]);
            if matches!(icon, Icon::Plus) {
                line(&[[0.5, 0.2], [0.5, 0.8]]);
            }
        }
        Icon::Left | Icon::Right => {
            let x = if matches!(icon, Icon::Left) {
                0.36
            } else {
                0.64
            };
            line(&[[1.0 - x, 0.2], [x, 0.5], [1.0 - x, 0.8]]);
        }
        Icon::Import => {
            line(&[[0.5, 0.08], [0.5, 0.62]]);
            line(&[[0.32, 0.45], [0.5, 0.62], [0.68, 0.45]]);
            line(&[[0.14, 0.6], [0.14, 0.88], [0.86, 0.88], [0.86, 0.6]]);
        }
        Icon::Close => {
            line(&[[0.22, 0.22], [0.78, 0.78]]);
            line(&[[0.78, 0.22], [0.22, 0.78]]);
        }
        Icon::Copy => {
            line(&[
                [0.28, 0.62],
                [0.1, 0.62],
                [0.1, 0.1],
                [0.62, 0.1],
                [0.62, 0.28],
            ]);
            fill_box([0.38, 0.38], [0.9, 0.9], 0.08, soft);
            box_at([0.38, 0.38], [0.9, 0.9], 0.08);
        }
        Icon::Theme => {
            // A half-filled circle: light and dark appearance.
            let c = rect.center();
            let r = rect.width() * 0.38;
            painter.circle_stroke(c, r, stroke);
            painter.add(egui::Shape::convex_polygon(
                (0..=16)
                    .map(|i| {
                        let a = (-90. + i as f32 * 180. / 16.).to_radians();
                        c + vec2(a.cos(), a.sin()) * r
                    })
                    .collect(),
                color,
                Stroke::NONE,
            ));
        }
        Icon::Lasso => {
            // An open rope loop with a trailing tail.
            line(
                &arc([0.52, 0.4], 0.36, 140., 400.)
                    .iter()
                    .map(|v| [v[0], 0.4 + (v[1] - 0.4) * 0.72])
                    .collect::<Vec<_>>(),
            );
            line(&[[0.27, 0.6], [0.3, 0.72], [0.22, 0.84], [0.3, 0.94]]);
            painter.circle_stroke(p(0.27, 0.6), width * 1.6, stroke);
        }
        Icon::AlignObjectsLeft
        | Icon::AlignObjectsCenter
        | Icon::AlignObjectsRight
        | Icon::AlignObjectsTop
        | Icon::AlignObjectsMiddle
        | Icon::AlignObjectsBottom => {
            let (bars_horizontal, at) = match icon {
                Icon::AlignObjectsLeft => (true, 0.0),
                Icon::AlignObjectsCenter => (true, 0.5),
                Icon::AlignObjectsRight => (true, 1.0),
                Icon::AlignObjectsTop => (false, 0.0),
                Icon::AlignObjectsMiddle => (false, 0.5),
                _ => (false, 1.0),
            };
            let swap = |v: [f32; 2]| if bars_horizontal { v } else { [v[1], v[0]] };
            let edge = 0.1 + at * 0.8;
            for (y, length) in [(0.2, 0.74), (0.6, 0.46)] {
                let x = edge - length * at;
                fill_box(swap([x, y]), swap([x + length, y + 0.2]), 0.05, soft);
                box_at(swap([x, y]), swap([x + length, y + 0.2]), 0.05);
            }
            line(&[swap([edge, 0.06]), swap([edge, 0.94])]);
        }
        Icon::Check => line(&[[0.16, 0.52], [0.4, 0.76], [0.86, 0.26]]),
        Icon::Warning => {
            line(&[[0.5, 0.1], [0.94, 0.86], [0.06, 0.86], [0.5, 0.1]]);
            line(&[[0.5, 0.38], [0.5, 0.6]]);
            painter.circle_filled(p(0.5, 0.73), width * 0.9, color);
        }
        Icon::Info => {
            painter.circle_stroke(rect.center(), rect.width() * 0.42, stroke);
            line(&[[0.5, 0.46], [0.5, 0.72]]);
            painter.circle_filled(p(0.5, 0.31), width * 0.9, color);
        }
    }
}

// ---------------------------------------------------------------------------
// Buttons

pub(super) struct ActionButton<'a> {
    icon: Icon,
    label: &'a str,
    selected: bool,
    selectable: bool,
    primary: bool,
    compact: bool,
    tab: bool,
    dropdown: bool,
    tint: Option<Color32>,
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
            tint: None,
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
    /// A segment inside a [`segment_track`]: the selected segment floats as a white card.
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
    /// Show the tool's current color as a small bar under its icon.
    pub fn tint(mut self, color: Color32) -> Self {
        self.tint = Some(color);
        self
    }
    pub fn width(&self, ui: &egui::Ui) -> f32 {
        let text = if self.compact {
            0.
        } else {
            ui.fonts_mut(|f| {
                f.layout_no_wrap(self.label.into(), FontId::proportional(13.0), theme::ink())
                    .size()
                    .x
            }) + 44.0
        };
        (if self.compact { 32.0 } else { text }) + if self.dropdown { 12. } else { 0. }
    }
}
impl egui::Widget for ActionButton<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let font = FontId::proportional(13.0);
        let base_color = theme::ink();
        let galley = ui
            .painter()
            .layout_no_wrap(self.label.into(), font, base_color);
        let size = vec2(self.width(ui), if self.tab { 30.0 } else { 32.0 });
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
            let painter = ui.painter();
            let enabled = ui.is_enabled();
            let hovered = enabled && response.hovered();
            let pressed = hovered && response.is_pointer_button_down_on();
            let filled = self.primary && enabled;
            let radius = if self.tab { 7 } else { 8 };
            if filled {
                shadow(painter, rect, radius, 1);
                gradient_rect(
                    painter,
                    rect,
                    radius,
                    Fill::Sheen,
                    if hovered {
                        theme::accent_hover()
                    } else {
                        theme::accent()
                    },
                );
            } else if self.selected && self.tab {
                shadow(painter, rect, radius, 1);
                painter.rect_filled(rect, radius, theme::raised());
            } else if self.selected {
                painter.rect_filled(rect, radius, theme::accent_soft());
            } else if hovered {
                painter.rect_filled(
                    rect,
                    radius,
                    if pressed {
                        theme::pressed()
                    } else {
                        theme::hover()
                    },
                );
            }
            let fg = if !enabled {
                theme::muted().gamma_multiply(0.55)
            } else if filled {
                Color32::WHITE
            } else if self.selected {
                theme::accent()
            } else {
                base_color
            };
            if response.has_focus() {
                painter.rect_stroke(
                    rect.expand(1.),
                    radius + 2,
                    Stroke::new(2.5, theme::accent().gamma_multiply(0.45)),
                    egui::StrokeKind::Outside,
                );
            }
            let icon_x = if self.compact && !self.dropdown {
                rect.center().x
            } else {
                rect.left() + 16.5
            };
            let lift = if self.tint.is_some() { 2.5 } else { 0. };
            let icon_rect =
                Rect::from_center_size(pos2(icon_x, rect.center().y - lift), vec2(18., 18.));
            draw_icon(painter, icon_rect, self.icon, fg);
            if let Some(tint) = self.tint {
                let bar =
                    Rect::from_center_size(pos2(icon_x, icon_rect.bottom() + 3.5), vec2(15., 3.));
                painter.rect_filled(
                    bar,
                    1.5,
                    if enabled {
                        tint
                    } else {
                        tint.gamma_multiply(0.4)
                    },
                );
                if tint.r() > 225 && tint.g() > 225 && tint.b() > 225 {
                    painter.rect_stroke(
                        bar,
                        1.5,
                        Stroke::new(0.6, theme::line()),
                        egui::StrokeKind::Outside,
                    );
                }
            }
            if !self.compact {
                painter.galley_with_override_text_color(
                    pos2(rect.left() + 31.0, rect.center().y - galley.size().y / 2.0),
                    galley,
                    fg,
                );
            }
            if self.dropdown {
                let p = pos2(rect.right() - 10., rect.center().y);
                painter.add(egui::Shape::line(
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

/// The rounded gray track that holds [`ActionButton::tab`] segments.
pub(super) fn segment_track<R>(
    ui: &mut egui::Ui,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::Frame::new()
        .fill(theme::track())
        .corner_radius(10)
        .inner_margin(egui::Margin::same(3))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.;
            ui.horizontal(add_contents).inner
        })
        .inner
}

/// A full-width segmented control. Returns the newly chosen index.
pub(super) fn segmented(ui: &mut egui::Ui, options: &[&str], selected: usize) -> Option<usize> {
    let (track, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.), egui::Sense::hover());
    ui.painter().rect_filled(track, 9, theme::track());
    let width = (track.width() - 6.) / options.len().max(1) as f32;
    let mut choice = None;
    for (i, label) in options.iter().enumerate() {
        let r = Rect::from_min_size(
            track.min + vec2(3. + i as f32 * width, 3.),
            vec2(width, track.height() - 6.),
        );
        let response = ui.interact(r, ui.id().with(("segment", i)), egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Button, true, i == selected, *label)
        });
        if i == selected {
            shadow(ui.painter(), r, 7, 1);
            ui.painter().rect_filled(r, 7, theme::raised());
        } else if response.hovered() {
            ui.painter().rect_filled(r, 7, theme::hover());
        }
        let font = if i == selected {
            strong(ui, 12.5)
        } else {
            FontId::proportional(12.5)
        };
        ui.painter().text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            *label,
            font,
            if i == selected {
                theme::ink()
            } else {
                theme::muted()
            },
        );
        if response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            choice = Some(i);
        }
    }
    choice
}

/// A round color well that opens a picker. Returns true when the color changed.
pub(super) fn color_swatch(ui: &mut egui::Ui, current: &mut [f32; 3], label: &str) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(32., 32.), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ColorButton, true, label));
    let color = Color32::from_rgb(
        (current[0] * 255.) as u8,
        (current[1] * 255.) as u8,
        (current[2] * 255.) as u8,
    );
    if response.hovered() {
        ui.painter().rect_filled(rect, 8, theme::hover());
    }
    shadow(
        ui.painter(),
        Rect::from_center_size(rect.center(), vec2(20., 20.)),
        10,
        1,
    );
    ui.painter()
        .circle_filled(rect.center(), 10., theme::raised());
    ui.painter().circle_filled(rect.center(), 8., color);
    let mut changed = false;
    egui::Popup::from_toggle_button_response(&response)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let mut picked = color;
            if egui::color_picker::color_picker_color32(
                ui,
                &mut picked,
                egui::color_picker::Alpha::Opaque,
            ) && picked != color
            {
                *current = [picked.r(), picked.g(), picked.b()].map(|c| c as f32 / 255.);
                changed = true;
            }
        });
    response
        .on_hover_text(label)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    changed
}

// ---------------------------------------------------------------------------
// Dialogs

#[derive(Clone, Copy)]
pub(super) enum DialogTone {
    Primary,
    Secondary,
    Destructive,
}

pub(super) fn dialog_button(ui: &mut egui::Ui, label: &str, tone: DialogTone) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), 32.), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let hovered = response.hovered();
    let painter = ui.painter();
    let text = match tone {
        DialogTone::Primary => {
            shadow(painter, rect, 9, 1);
            gradient_rect(
                painter,
                rect,
                9,
                Fill::Sheen,
                if hovered {
                    theme::accent_hover()
                } else {
                    theme::accent()
                },
            );
            Color32::WHITE
        }
        DialogTone::Secondary | DialogTone::Destructive => {
            painter.rect_filled(
                rect,
                9,
                if hovered {
                    theme::control_hover()
                } else {
                    theme::control()
                },
            );
            if matches!(tone, DialogTone::Destructive) {
                theme::danger()
            } else {
                theme::ink()
            }
        }
    };
    if response.has_focus() {
        painter.rect_stroke(
            rect,
            9,
            Stroke::new(2.5, theme::accent().gamma_multiply(0.45)),
            egui::StrokeKind::Outside,
        );
    }
    let font = font(ui.ctx(), STRONG, 13.5);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        font,
        text,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The shared card used by modal dialogs.
pub(super) fn modal_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(theme::surface())
        .corner_radius(18)
        .inner_margin(24)
        .stroke(Stroke::new(1., theme::line()))
        .shadow(egui::epaint::Shadow {
            offset: [0, 18],
            blur: 48,
            spread: 0,
            color: Color32::from_black_alpha(55),
        })
}

/// macOS-style alert: icon tile, centered title and message, then actions.
/// Two actions sit side by side with the default on the right; more stack vertically.
pub(super) fn confirmation(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    body: &str,
    buttons: &[(&str, DialogTone)],
    cancel: usize,
) -> Option<usize> {
    let mut decision = None;
    let destructive = buttons
        .iter()
        .any(|(_, tone)| matches!(tone, DialogTone::Destructive));
    let modal = egui::Modal::new(egui::Id::new(id))
        .backdrop_color(Color32::from_black_alpha(64))
        .frame(modal_frame())
        .show(ctx, |ui| {
            ui.set_width(292.);
            ui.vertical_centered(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(54., 54.), egui::Sense::hover());
                if destructive {
                    badge(ui.painter(), r, Icon::Trash, Fill::Danger, Color32::WHITE);
                } else {
                    badge(ui.painter(), r, Icon::Document, Fill::Brand, Color32::WHITE);
                }
                ui.add_space(14.);
                let heading = font(ctx, STRONG, 16.);
                ui.label(RichText::new(title).font(heading).color(theme::ink()));
                ui.add_space(4.);
                ui.label(RichText::new(body).size(13.).color(theme::muted()));
            });
            ui.add_space(20.);
            let mut add = |ui: &mut egui::Ui, index: usize| {
                let (label, tone) = buttons[index];
                if dialog_button(ui, label, tone).clicked() {
                    decision = Some(index);
                }
            };
            if buttons.len() == 2 {
                let half = (ui.available_width() - 10.) / 2.;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.;
                    for index in [1, 0] {
                        ui.allocate_ui(vec2(half, 32.), |ui| add(ui, index));
                    }
                });
            } else {
                ui.spacing_mut().item_spacing.y = 8.;
                for index in 0..buttons.len() {
                    add(ui, index);
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

#[derive(Clone, Copy)]
pub(super) enum Tone {
    Info,
    Success,
    Warning,
    Danger,
}
impl Tone {
    fn style(self) -> (Icon, Color32) {
        match self {
            Tone::Info => (Icon::Info, theme::accent()),
            Tone::Success => (Icon::Check, theme::success()),
            Tone::Warning => (Icon::Warning, theme::warning()),
            Tone::Danger => (Icon::Warning, theme::danger()),
        }
    }
}
/// A tinted one-line status message with an icon.
pub(super) fn callout(ui: &mut egui::Ui, tone: Tone, text: &str) {
    let (icon, color) = tone.style();
    egui::Frame::new()
        .fill(color.gamma_multiply(0.1))
        .corner_radius(9)
        .inner_margin(egui::Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(16., 16.), egui::Sense::hover());
                draw_icon(ui.painter(), r, icon, color);
                ui.add(egui::Label::new(RichText::new(text).size(12.5).color(theme::ink())).wrap());
            });
        });
}
/// A labeled form row: a small caption above the control.
pub(super) fn field<R>(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.;
        ui.label(RichText::new(label).size(12.).color(theme::muted()));
        add(ui)
    })
    .inner
}
/// A titled group of related controls on a soft surface.
pub(super) fn section<R>(
    ui: &mut egui::Ui,
    title: &str,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::Frame::new()
        .fill(theme::paper())
        .stroke(Stroke::new(1., theme::line()))
        .corner_radius(12)
        .inner_margin(14)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            caption(ui, title);
            ui.add_space(2.);
            add(ui)
        })
        .inner
}
/// A pass/fail line for verification results.
pub(super) fn check_row(ui: &mut egui::Ui, label: &str, passed: bool) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(18., 18.), egui::Sense::hover());
        let color = if passed {
            theme::success()
        } else {
            theme::danger()
        };
        ui.painter()
            .circle_filled(r.center(), 9., color.gamma_multiply(0.14));
        draw_icon(
            ui.painter(),
            r.shrink(4.),
            if passed { Icon::Check } else { Icon::Close },
            color,
        );
        ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let font = strong(ui, 12.);
            ui.label(
                RichText::new(if passed { "Passed" } else { "Failed" })
                    .font(font)
                    .color(color),
            );
        });
    });
}

/// A dashed selection outline, as used around text being edited.
pub(super) fn dashed_rect(painter: &egui::Painter, rect: Rect, color: Color32) {
    let points = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    painter.extend(egui::Shape::dashed_line(
        &points,
        Stroke::new(1., color),
        4.,
        3.,
    ));
}

/// A soft informational card with an icon, a short title and a hint.
pub(super) fn tip(ui: &mut egui::Ui, title: &str, body: &str) {
    egui::Frame::new()
        .fill(theme::accent_soft())
        .corner_radius(10)
        .inner_margin(12)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(18., 18.), egui::Sense::hover());
                draw_icon(ui.painter(), r, Icon::Info, theme::accent());
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.;
                    let font = strong(ui, 12.5);
                    ui.label(RichText::new(title).font(font).color(theme::ink()));
                    ui.label(RichText::new(body).size(11.5).color(theme::muted()));
                });
            });
        });
}

/// A dismissible notice strip: soft danger background, warning glyph and close button.
pub(super) fn notice(ui: &mut egui::Ui, message: &str) -> bool {
    let mut dismiss = false;
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(18., 18.), egui::Sense::hover());
        draw_icon(ui.painter(), r, Icon::Warning, theme::danger());
        ui.add(egui::Label::new(RichText::new(message).size(12.5).color(theme::ink())).wrap());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            dismiss = ui
                .add(ActionButton::new(Icon::Close, "Dismiss").compact())
                .clicked();
        });
    });
    dismiss
}

// ---------------------------------------------------------------------------
// Theme

/// Install light and dark styles; egui picks one per the remembered preference.
pub(super) fn install_theme(ctx: &egui::Context) {
    for (theme, dark) in [(egui::Theme::Light, false), (egui::Theme::Dark, true)] {
        let mut style = (*ctx.style_of(theme)).clone();
        style_for(&mut style, theme::palette_for(dark), dark);
        ctx.set_style_of(theme, style);
    }
    // Light when the platform does not report its appearance.
    ctx.options_mut(|o| o.fallback_theme = egui::Theme::Light);
    ctx.set_theme(theme::load_preference());
}
/// The tab-strip appearance button: Match System, Light or Dark.
pub(super) fn theme_menu(ui: &mut egui::Ui) -> Option<&'static str> {
    let current = ui.ctx().options(|o| o.theme_preference);
    let label = match current {
        egui::ThemePreference::System => "Appearance: Match System",
        egui::ThemePreference::Light => "Appearance: Light",
        egui::ThemePreference::Dark => "Appearance: Dark",
    };
    let response = ui.add(ActionButton::new(Icon::Theme, label).compact());
    let mut choice = None;
    egui::Popup::menu(&response).show(|ui| {
        ui.set_min_width(170.);
        for (preference, name, action) in [
            (
                egui::ThemePreference::System,
                "Match System",
                "theme_system",
            ),
            (egui::ThemePreference::Light, "Light", "theme_light"),
            (egui::ThemePreference::Dark, "Dark", "theme_dark"),
        ] {
            if ui.selectable_label(current == preference, name).clicked() {
                choice = Some(action);
                ui.close();
            }
        }
    });
    choice
}
/// Switch Match System / Light / Dark and remember it.
pub(super) fn set_theme_preference(ctx: &egui::Context, preference: egui::ThemePreference) {
    ctx.set_theme(preference);
    let _ = theme::save_preference(preference);
}
fn style_for(style: &mut egui::Style, p: &theme::Palette, dark: bool) {
    style.visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let v = &mut style.visuals;
    v.override_text_color = None;
    v.panel_fill = p.paper;
    v.window_fill = p.surface;
    v.window_corner_radius = 14.into();
    v.window_shadow = egui::epaint::Shadow {
        offset: [0, 14],
        blur: 40,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 120 } else { 46 }),
    };
    v.window_stroke = Stroke::new(1.0, p.line);
    v.popup_shadow = egui::epaint::Shadow {
        offset: [0, 6],
        blur: 22,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 100 } else { 34 }),
    };
    v.menu_corner_radius = 10.into();
    v.faint_bg_color = p.paper;
    v.extreme_bg_color = p.surface;
    v.code_bg_color = p.control;
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = if dark {
        Color32::from_rgb(46, 72, 128)
    } else {
        Color32::from_rgb(205, 221, 255)
    };
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.text_cursor.stroke = Stroke::new(2.0, p.accent);
    v.handle_shape = egui::style::HandleShape::Circle;
    v.indent_has_left_vline = false;
    for widget in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        widget.corner_radius = 8.into();
        widget.fg_stroke = Stroke::new(1.0, p.ink);
        widget.expansion = 0.;
    }
    // Secondary text in egui widgets (hints, weak labels) follows `muted`.
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.ink);
    v.weak_text_color = Some(p.muted);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.line);
    v.widgets.noninteractive.bg_fill = p.surface;
    v.widgets.noninteractive.weak_bg_fill = p.surface;
    v.widgets.inactive.bg_fill = p.surface;
    v.widgets.inactive.weak_bg_fill = p.surface;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.line);
    v.widgets.hovered.bg_fill = p.control;
    v.widgets.hovered.weak_bg_fill = p.control;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.accent_line);
    v.widgets.active.bg_fill = p.accent_soft;
    v.widgets.active.weak_bg_fill = p.accent_soft;
    v.widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
    // Window title bars use this fill: keep them seamless with the window body.
    v.widgets.open.weak_bg_fill = p.surface;
    v.widgets.open.bg_fill = p.surface;
    v.widgets.open.bg_stroke = Stroke::new(1.0, p.line);
    style.spacing.button_padding = vec2(12.0, 6.0);
    style.spacing.item_spacing = vec2(8.0, 8.0);
    style.spacing.interact_size = vec2(36.0, 30.0);
    style.spacing.window_margin = egui::Margin::same(18);
    style.spacing.menu_margin = egui::Margin::same(6);
    style.spacing.slider_rail_height = 4.0;
    for (text_style, font) in [
        (egui::TextStyle::Body, FontId::proportional(13.5)),
        (egui::TextStyle::Button, FontId::proportional(13.0)),
        (egui::TextStyle::Small, FontId::proportional(11.0)),
        (egui::TextStyle::Monospace, FontId::monospace(12.5)),
        (
            egui::TextStyle::Heading,
            FontId::new(17.0, egui::FontFamily::Name(STRONG.into())),
        ),
    ] {
        style.text_styles.insert(text_style, font);
    }
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
    #[test]
    fn light_and_dark_styles_follow_the_theme_preference() {
        let ctx = egui::Context::default();
        install_theme(&ctx);
        for (preference, dark) in [
            (egui::ThemePreference::Light, false),
            (egui::ThemePreference::Dark, true),
            // Without a reported system appearance, Match System is light.
            (egui::ThemePreference::System, false),
        ] {
            ctx.set_theme(preference);
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                theme::sync(ctx.theme());
                let expected = theme::palette_for(dark);
                assert_eq!(theme::is_dark(), dark);
                assert_eq!(ctx.style().visuals.panel_fill, expected.paper);
                assert_eq!(ctx.style().visuals.window_fill, expected.surface);
                assert_eq!(theme::ink(), expected.ink);
            });
        }
        theme::sync(egui::Theme::Light);
    }
    #[test]
    fn ui_font_families_always_resolve() {
        let mut fonts = egui::FontDefinitions::default();
        add_ui_fonts(&mut fonts);
        for family in [STRONG, BOLD] {
            assert!(
                !fonts.families[&egui::FontFamily::Name(family.into())].is_empty(),
                "{family} has no faces"
            );
        }
        assert!(!fonts.families[&egui::FontFamily::Proportional].is_empty());
    }
}
