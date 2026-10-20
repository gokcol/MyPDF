use super::*;
#[path = "image_background.rs"]
mod background;
#[path = "note_reader.rs"]
mod note_reader;
use mypdf_milestone0::{
    annotations::{
        self as ann, Annotation, Box2, Kind, Markup, Primitive, StrokeStyle, Style, Symbol,
    },
    assets::{self, Asset, Library},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Mode {
    Read,
    Annotate,
    Draw,
    Sign,
    Pages,
    Edit,
    Forms,
    Convert,
}
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Tool {
    Select,
    Lasso,
    Pen,
    Highlight,
    Underline,
    Strikeout,
    Note,
    Text,
    Rectangle,
    Ellipse,
    Line,
    Arrow,
    DoubleArrow,
    Polyline,
    Polygon,
    Asset,
    SignArea,
}
impl Tool {
    fn label(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Lasso => "Lasso select",
            Self::Pen => "Pen",
            Self::Highlight => "Highlight",
            Self::Underline => "Underline",
            Self::Strikeout => "Strikeout",
            Self::Note => "Note",
            Self::Text => "Text box",
            Self::Rectangle => "Rectangle",
            Self::Ellipse => "Ellipse",
            Self::Line => "Line",
            Self::Arrow => "Arrow",
            Self::DoubleArrow => "Double arrow",
            Self::Polyline => "Polyline",
            Self::Polygon => "Polygon",
            Self::Asset => "Place asset",
            Self::SignArea => "Signature rectangle",
        }
    }
}
impl Tool {
    fn icon(self) -> Icon {
        match self {
            Self::Select => Icon::Select,
            Self::Lasso => Icon::Lasso,
            Self::Pen => Icon::Pen,
            Self::Highlight => Icon::Highlight,
            Self::Underline => Icon::Underline,
            Self::Strikeout => Icon::Strikeout,
            Self::Note => Icon::Note,
            Self::Text => Icon::Text,
            Self::Rectangle | Self::SignArea => Icon::Rectangle,
            Self::Ellipse => Icon::Circle,
            Self::Line => Icon::Line,
            Self::Arrow => Icon::Arrow,
            Self::DoubleArrow => Icon::DoubleArrow,
            Self::Polyline => Icon::Polyline,
            Self::Polygon => Icon::Polygon,
            Self::Asset => Icon::Library,
        }
    }
}

struct VertexPath {
    tab: u64,
    page: usize,
    points: Vec<[f32; 2]>,
    closed: bool,
}

struct Gesture {
    tab: u64,
    page: usize,
    start: [f32; 2],
    points: Vec<[f32; 2]>,
    original: Option<Annotation>,
    /// The corner being dragged (top-left, top-right, bottom-right, bottom-left).
    resize: Option<usize>,
    endpoint: Option<usize>,
    rotate: bool,
    duplicate: bool,
    guides: manipulation::Guides,
    preview: Option<Annotation>,
}

/// The default box for a click-placed note, text box or library item.
fn placement(editor: &EditorState, tab: &Tab, page: usize, at: [f32; 2], centered: bool) -> Box2 {
    let (width, height) = match editor.tool {
        Tool::Note => (32.0, 32.0),
        Tool::Text => (240.0, 72.0),
        Tool::Asset => {
            let aspect = editor.placed.as_ref().map_or(3.0, |a| a.aspect);
            let category = editor
                .placed
                .as_ref()
                .and_then(|a| editor.builtins.iter().find(|b| b.asset.id == a.id))
                .map(|b| b.category);
            let edge = match category {
                Some("Symbols") => 72.0,
                Some("Arrows") => 180.0,
                _ => 240.0,
            };
            (edge * aspect.min(1.0), edge / aspect.max(1.0))
        }
        _ => (52.0, 52.0),
    };
    let size = tab.pages[page];
    let at = if centered {
        [at[0] - width / 2.0, at[1] - height / 2.0]
    } else {
        at
    };
    let x = at[0].clamp(0.0, (size.width - width).max(0.0));
    let y = at[1].clamp(0.0, (size.height - height).max(0.0));
    [
        x,
        y,
        (x + width).min(size.width),
        (y + height).min(size.height),
    ]
}

/// Place the armed note, text box or library item at a page point.
fn place(
    editor: &mut EditorState,
    tab: &mut Tab,
    page: usize,
    at: [f32; 2],
    centered: bool,
    status: &mut Option<(String, bool)>,
) {
    let r = placement(editor, tab, page, at, centered);
    let a = editor.base_annotation(tab, page, r, at, at, &[at]);
    commit_new(editor, tab, a, status);
    editor.tool = Tool::Select;
}

/// The selection corner under a page point, if any.
pub(super) fn corner_at(a: &Annotation, p: [f32; 2], scale: f32) -> Option<usize> {
    let b = a.bounds();
    [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]]
        .iter()
        .position(|c| (p[0] - c[0]).abs() < 12.0 / scale && (p[1] - c[1]).abs() < 12.0 / scale)
}

/// Artwork keeps its proportions when resized; shapes do with Shift.
fn keeps_proportions(kind: &ann::Kind) -> bool {
    matches!(
        kind,
        ann::Kind::Symbol(_)
            | ann::Kind::Stamp { .. }
            | ann::Kind::Svg { .. }
            | ann::Kind::Image { .. }
            | ann::Kind::Vector { .. }
            | ann::Kind::Ink { .. }
    )
}

/// The rectangle after dragging `corner` to `p`; the opposite corner stays put.
fn resized(rect: [f32; 4], corner: usize, p: [f32; 2], proportional: bool) -> [f32; 4] {
    let (ow, oh) = ((rect[2] - rect[0]).max(1.0), (rect[3] - rect[1]).max(1.0));
    let right = matches!(corner, 1 | 2);
    let bottom = matches!(corner, 2 | 3);
    let ax = if right { rect[0] } else { rect[2] };
    let ay = if bottom { rect[1] } else { rect[3] };
    let mut w = (if right { p[0] - ax } else { ax - p[0] }).max(8.0);
    let mut h = (if bottom { p[1] - ay } else { ay - p[1] }).max(8.0);
    if proportional {
        let s = (w / ow).max(h / oh).max(8.0 / ow.min(oh));
        (w, h) = (ow * s, oh * s);
    }
    let x = if right { ax } else { ax - w };
    let y = if bottom { ay } else { ay - h };
    [x, y, x + w, y + h]
}
type SvgPreviewResult = (String, Result<Arc<Vec<u8>>, String>);
struct SvgEditor {
    open: bool,
    source: String,
    name: String,
    target: Option<(u64, String)>,
    library_target: Option<String>,
    stamp: bool,
    prompt_open: bool,
    prompt: String,
    error: Option<String>,
    preview: Option<Arc<Vec<u8>>>,
    changed: Option<std::time::Instant>,
    job: Option<mpsc::Receiver<SvgPreviewResult>>,
}
impl Default for SvgEditor {
    fn default() -> Self {
        Self {
            open: false,
            source: mypdf_milestone0::svg::DEFAULT.into(),
            name: "My SVG artwork".into(),
            target: None,
            library_target: None,
            stamp: false,
            prompt_open: false,
            prompt: String::new(),
            error: None,
            preview: None,
            changed: Some(std::time::Instant::now()),
            job: None,
        }
    }
}
#[derive(Clone, Copy, PartialEq)]
enum SignatureInput {
    Name,
    Draw,
    Type,
    Image,
}

struct SignaturePlacement {
    key: (u64, u64, usize),
    point: [f32; 2],
}

pub(super) struct EditorState {
    pub hide_guides: bool,
    background: Option<background::Draft>,
    svg: SvgEditor,
    pub mode: Mode,
    pub(super) tool: Tool,
    pub(super) style: Style,
    pub text_editing: bool,
    pub(super) snapping: bool,
    pub library_open: bool,
    pub signature_open: bool,
    signature_placement: Option<SignaturePlacement>,
    pub(super) signature_manage_only: bool,
    signature_picker: bool,
    pub asset_busy: bool,
    pub library: Option<Library>,
    library_error: Option<String>,
    placed: Option<Asset>,
    pub(super) draft: Option<Annotation>,
    // Document, annotation, last committed revision, and focused property field.
    pub(super) property_edit: Option<(u64, String, u64, Option<egui::Id>)>,
    pub(super) note_requested: Option<(u64, String)>,
    pub(super) marquee: Option<selection::Marquee>,
    pub(super) group_gesture: Option<selection::GroupGesture>,
    pub(super) text_selection: Option<text_selection::TextSelection>,
    pub(super) marks_clip: Option<clipboard::MarksClip>,
    /// Tab and last revision of a continuous group property edit or nudge.
    pub(super) group_edit: Option<(u64, u64)>,
    /// The asset most recently added to the library, for "use as my signature".
    pub(super) last_added: Option<String>,
    gesture: Option<Gesture>,
    vertices: Option<VertexPath>,
    pressed: Option<(u64, usize, [f32; 2])>,
    touch_navigation: bool,
    pub pending_sign_rect: Option<(u64, usize, Box2)>,
    pub annotation_sidebar: bool,
    asset_name: String,
    asset_details: Option<Asset>,
    pub(super) removed_asset: Option<Asset>,
    pub(super) library_tab: String,
    library_query: String,
    stamp_source: usize,
    library_folder: String,
    builtins: Vec<assets::BuiltinAsset>,
    signature_name: String,
    signature_typed: String,
    signature_name_style: usize,
    signature_flourish: bool,
    signature_ink_color: [f32; 3],
    signature_ink_width: f32,
    signature_generated: Option<Result<Asset, String>>,
    signature_generated_key: String,
    signature_input: SignatureInput,
    signature_image: Option<Asset>,
    signature_error: Option<String>,
    signature_tx: mpsc::Sender<Result<Asset, String>>,
    signature_rx: mpsc::Receiver<Result<Asset, String>>,
    signature_busy: bool,
    signature_strokes: Vec<Vec<[f32; 2]>>,
    signature_pressed: Option<[f32; 2]>,
    signature_active: bool,
    import_tx: mpsc::Sender<Result<Asset, String>>,
    import_rx: mpsc::Receiver<Result<Asset, String>>,
    images: HashMap<String, TextureHandle>,
    image_used: HashMap<String, u64>,
}
impl EditorState {
    /// Estimated RGBA texture storage retained for artwork previews. Encoded
    /// originals belong to the document/library and are accounted separately.
    pub(super) fn image_cache_bytes(&self) -> usize {
        self.images
            .values()
            .map(|t| t.size()[0] * t.size()[1] * 4)
            .sum()
    }

    /// Drop least-recently-used derived artwork textures before trimming undo.
    /// A zero budget releases every texture without touching the saved artwork.
    pub(super) fn trim_image_cache_to(&mut self, budget: usize) -> usize {
        let mut bytes = self.image_cache_bytes();
        let before = bytes;
        while bytes > budget {
            let Some(oldest) = self
                .images
                .keys()
                .min_by_key(|key| self.image_used.get(*key).copied().unwrap_or(0))
                .cloned()
            else {
                break;
            };
            if let Some(texture) = self.images.remove(&oldest) {
                bytes = bytes.saturating_sub(texture.size()[0] * texture.size()[1] * 4);
            }
            self.image_used.remove(&oldest);
        }
        before - bytes
    }

    pub(super) fn interaction_active(&self) -> bool {
        self.gesture.is_some()
            || self.group_gesture.is_some()
            || self.marquee.is_some()
            || self.vertices.is_some()
    }

    pub(super) fn is_building_path(&self) -> bool {
        self.vertices.is_some()
    }

    pub fn new() -> Self {
        #[cfg(not(test))]
        let root = assets::data_directory();
        #[cfg(test)]
        let root =
            std::env::temp_dir().join(format!("mypdf-headless-{:016x}", rand::random::<u64>()));
        let (library, library_error) = match Library::open(root) {
            Ok(l) => (Some(l), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        let (import_tx, import_rx) = mpsc::channel();
        let (signature_tx, signature_rx) = mpsc::channel();
        Self {
            hide_guides: false,
            background: None,
            svg: SvgEditor::default(),
            mode: Mode::Read,
            tool: Tool::Select,
            style: Style::default(),
            text_editing: false,
            snapping: true,
            library_open: false,
            signature_open: false,
            signature_placement: None,
            signature_manage_only: false,
            signature_picker: false,
            asset_busy: false,
            library,
            library_error,
            placed: None,
            draft: None,
            property_edit: None,
            note_requested: None,
            marquee: None,
            group_gesture: None,
            text_selection: None,
            marks_clip: None,
            group_edit: None,
            last_added: None,
            gesture: None,
            vertices: None,
            pressed: None,
            touch_navigation: false,
            pending_sign_rect: None,
            annotation_sidebar: false,
            asset_name: "My asset".into(),
            asset_details: None,
            removed_asset: None,
            library_tab: "Arrows".into(),
            library_query: String::new(),
            stamp_source: 0,
            library_folder: String::new(),
            builtins: assets::builtin_assets(),
            signature_name: "My handwritten signature".into(),
            signature_typed: String::new(),
            signature_name_style: 0,
            signature_flourish: true,
            signature_ink_color: [0.1, 0.14, 0.18],
            signature_ink_width: 2.,
            signature_generated: None,
            signature_generated_key: String::new(),
            signature_input: SignatureInput::Draw,
            signature_image: None,
            signature_error: None,
            signature_tx,
            signature_rx,
            signature_busy: false,
            signature_strokes: vec![],
            signature_pressed: None,
            signature_active: false,
            import_tx,
            import_rx,
            images: HashMap::new(),
            image_used: HashMap::new(),
        }
    }
    #[cfg(test)]
    pub(super) fn svg_preview_close(&mut self) {
        self.svg.open = false;
    }
    pub(super) fn create_stamp_prompt(&mut self) {
        self.svg = SvgEditor {
            open: true,
            stamp: true,
            prompt_open: true,
            source: String::new(),
            name: "My stamp".into(),
            ..Default::default()
        };
    }
    pub(super) fn open_signature(&mut self) {
        self.signature_placement = None;
        self.signature_manage_only = false;
        self.signature_picker = false;
        self.signature_open = true;
    }

    pub(super) fn choose_signature(&mut self) {
        self.open_signature();
        self.signature_picker = true;
    }

    pub(super) fn add_asset(&mut self, asset: Asset) -> Result<(), String> {
        let library = self.library.as_mut().ok_or_else(|| {
            self.library_error
                .clone()
                .unwrap_or_else(|| "Library is unavailable".into())
        })?;
        let stamp = asset.stamp;
        let id = asset.id.clone();
        library.put(asset).map_err(|e| format!("{e:#}"))?;
        self.last_added = Some(id);
        if stamp {
            self.stamp_source = 1;
            self.library_folder.clear();
        }
        self.library_tab = if stamp { "Stamps" } else { "Saved" }.into();
        Ok(())
    }
    pub(super) fn select_tool(&mut self, tool: Tool) {
        self.tool = tool;
        self.vertices = None;
        self.gesture = None;
        self.pressed = None;
        if tool == Tool::Highlight {
            self.style.color = HIGHLIGHT;
            self.style.opacity = 0.35;
            self.style.fill = None;
        } else {
            self.style.opacity = 1.0;
            if self.style.color == HIGHLIGHT {
                self.style.color = DEFAULT_INK;
            }
        }
    }
    pub(super) fn start_signature_rectangle(&mut self) {
        self.tool = Tool::SignArea;
        self.gesture = None;
        self.pressed = None;
    }
    fn base_annotation(
        &self,
        tab: &Tab,
        page: usize,
        r: Box2,
        start: [f32; 2],
        end: [f32; 2],
        points: &[[f32; 2]],
    ) -> Annotation {
        let mut r = r;
        if matches!(
            self.tool,
            Tool::Highlight | Tool::Underline | Tool::Strikeout
        ) && let Some(text) = tab.text.get(page).and_then(Option::as_ref)
        {
            let original = r;
            for g in text
                .glyphs
                .iter()
                .filter(|g| {
                    g.rect[0] < original[2]
                        && g.rect[2] > original[0]
                        && g.rect[1] < original[3]
                        && g.rect[3] > original[1]
                })
                .take(10_000)
            {
                r[0] = r[0].min(g.rect[0]);
                r[1] = r[1].min(g.rect[1]);
                r[2] = r[2].max(g.rect[2]);
                r[3] = r[3].max(g.rect[3]);
            }
        }
        let local = |p: [f32; 2]| [(p[0] - r[0]) / (r[2] - r[0]), (p[1] - r[1]) / (r[3] - r[1])];
        let mut style = self.style.clone();
        let kind = match self.tool {
            Tool::Pen => Kind::Ink {
                strokes: vec![points.iter().map(|p| local(*p)).collect()],
            },
            Tool::Highlight | Tool::Underline | Tool::Strikeout => {
                let quads: Vec<_> = tab
                    .text
                    .get(page)
                    .and_then(Option::as_ref)
                    .map(|t| {
                        t.glyphs
                            .iter()
                            .filter(|g| {
                                g.rect[0] < r[2]
                                    && g.rect[2] > r[0]
                                    && g.rect[1] < r[3]
                                    && g.rect[3] > r[1]
                            })
                            .take(10_000)
                            .map(|g| {
                                let a = local([g.rect[0], g.rect[1]]);
                                let b = local([g.rect[2], g.rect[3]]);
                                [a[0], a[1], b[0], b[1]]
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                Kind::Markup {
                    kind: if self.tool == Tool::Highlight {
                        Markup::Highlight
                    } else if self.tool == Tool::Underline {
                        Markup::Underline
                    } else {
                        Markup::Strikeout
                    },
                    quads: if quads.is_empty() {
                        vec![[0.0, 0.0, 1.0, 1.0]]
                    } else {
                        quads
                    },
                }
            }
            Tool::Note => {
                style.fill = Some([1.0, 0.82, 0.23]);
                Kind::Note {
                    text: "New note".into(),
                }
            }
            Tool::Text => Kind::Text {
                text: "Your text".into(),
            },
            Tool::Ellipse => Kind::Ellipse,
            Tool::DoubleArrow => Kind::DoubleArrow {
                start: local(start),
                end: local(end),
            },
            Tool::Line | Tool::Arrow => Kind::Line {
                start: local(start),
                end: local(end),
                arrow: self.tool == Tool::Arrow,
            },
            Tool::Asset => {
                if let Some(asset) = &self.placed {
                    style = asset.style.clone();
                    asset.kind.clone()
                } else {
                    Kind::Symbol(Symbol::Check)
                }
            }
            _ => Kind::Rectangle,
        };
        let mut a = Annotation::new(page, r, kind, style);
        if self.tool == Tool::Asset {
            a.asset_id = self.placed.as_ref().map(|a| a.id.clone());
        }
        a
    }
}

const HIGHLIGHT: [f32; 3] = [1.0, 0.8, 0.05];
const DEFAULT_INK: [f32; 3] = [0.11, 0.43, 0.35];

pub(super) fn color_palette(ui: &mut egui::Ui, current: &mut [f32; 3]) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 7.;
        for (name, rgb) in [
            ("Ink", [0.15, 0.17, 0.21]),
            ("Blue", [0.15, 0.49, 0.85]),
            ("Green", [0.11, 0.43, 0.35]),
            ("Coral", [0.9, 0.29, 0.29]),
            ("Gold", [0.95, 0.68, 0.15]),
        ] {
            let (r, response) = ui.allocate_exact_size(vec2(24., 28.), egui::Sense::click());
            #[cfg(test)]
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::new(("color_swatch", name)), r));
            let selected = *current == rgb;
            let swatch = color(rgb, 1.);
            ui.painter().circle_filled(
                r.center(),
                if response.hovered() && !selected {
                    9.5
                } else {
                    8.5
                },
                swatch,
            );
            // A hairline keeps dark ink visible on the dark theme's panels.
            ui.painter()
                .circle_stroke(r.center(), 8.5, Stroke::new(1., theme::line()));
            if selected {
                ui.painter()
                    .circle_stroke(r.center(), 11.5, Stroke::new(2., swatch));
            }
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Button,
                    ui.is_enabled(),
                    selected,
                    format!("{name} color"),
                )
            });
            if response.on_hover_text(name).clicked() {
                *current = rgb;
                changed = true;
            }
        }
        // The picker converts RGB through HSV even without user input. Only accept
        // an actual edit, avoiding floating-point drift and phantom undo revisions.
        changed |= ui::color_swatch(ui, current, "Custom color");
    });
    changed
}
pub(super) fn property_slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
) -> Option<egui::Id> {
    let mut focus = None;
    ui.push_id(label, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(label).size(12.).color(theme::muted()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let r = ui.add(
                    egui::DragValue::new(value)
                        .range(range.clone())
                        .speed(0.5)
                        .max_decimals(1)
                        .suffix(suffix),
                );
                r.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::DragValue, ui.is_enabled(), label)
                });
                if r.has_focus() {
                    focus = Some(r.id);
                }
            });
        });
        ui.spacing_mut().slider_width = ui.available_width();
        ui.scope(|ui| {
            ui.spacing_mut().interact_size.y = 18.;
            ui.spacing_mut().slider_rail_height = 3.;
            let r = ui.add(
                egui::Slider::new(value, range)
                    .show_value(false)
                    .trailing_fill(true)
                    .handle_shape(egui::style::HandleShape::Circle),
            );
            r.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Slider, ui.is_enabled(), label)
            });
        });
    });
    focus
}

fn stroke_styles(ui: &mut egui::Ui, current: &mut StrokeStyle) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Style").size(12.).color(theme::muted()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(current.label()).size(12.).color(theme::ink()));
        });
    });
    let width = (ui.available_width() - ui.spacing().item_spacing.x * 4.) / 5.;
    ui.horizontal(|ui| {
        for style in StrokeStyle::ALL {
            let (r, response) = ui.allocate_exact_size(vec2(width, 30.), egui::Sense::click());
            let selected = *current == style;
            let c = if selected {
                theme::accent()
            } else {
                theme::ink()
            };
            ui.painter().rect_filled(
                r,
                6.,
                if selected {
                    theme::accent_soft()
                } else if response.hovered() {
                    theme::control_hover()
                } else {
                    theme::control()
                },
            );
            for mark in mypdf_milestone0::stroke::path(
                vec![
                    [r.left() + 7., r.center().y],
                    [r.right() - 7., r.center().y],
                ],
                false,
                1.4,
                style,
            ) {
                match mark {
                    Primitive::Path { points, .. } => {
                        ui.painter().add(egui::Shape::line(
                            points.into_iter().map(|p| pos2(p[0], p[1])).collect(),
                            Stroke::new(1.4, c),
                        ));
                    }
                    Primitive::Dot { center } => {
                        ui.painter()
                            .circle_filled(pos2(center[0], center[1]), 0.9, c);
                    }
                    _ => {}
                }
            }
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Button,
                    ui.is_enabled(),
                    selected,
                    format!("{} stroke style", style.label()),
                )
            });
            if response.on_hover_text(style.label()).clicked() {
                *current = style;
            }
        }
    });
}

pub(super) fn install_annotation_font(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let family = egui::FontFamily::Name("annotation".into());
    let fallback = fonts.families[&egui::FontFamily::Proportional].clone();
    fonts.families.insert(family.clone(), fallback);
    if let Ok(path) = mypdf_milestone0::document::default_font()
        && let Ok(bytes) = std::fs::read(path)
    {
        fonts.font_data.insert(
            "annotation".into(),
            Arc::new(egui::FontData::from_owned(bytes)),
        );
        fonts
            .families
            .get_mut(&family)
            .unwrap()
            .insert(0, "annotation".into());
    }
    for face in mypdf_milestone0::text_style::fonts() {
        if let Ok(bytes) = std::fs::read(&face.path) {
            let name = face.ui_name();
            fonts
                .font_data
                .insert(name.clone(), Arc::new(egui::FontData::from_owned(bytes)));
            fonts
                .families
                .insert(egui::FontFamily::Name(name.clone().into()), vec![name]);
        }
    }
    ui::add_ui_fonts(&mut fonts);
    ctx.set_fonts(fonts);
}

/// Paint library artwork fitted and centered inside `rect`.
pub(super) fn paint_asset(
    editor: &mut EditorState,
    ctx: &egui::Context,
    painter: &egui::Painter,
    rect: Rect,
    asset: &Asset,
) {
    let base = vec2(240.0, 240.0 / asset.aspect.max(0.01));
    let scale = (rect.width() / base.x).min(rect.height() / base.y);
    let origin = rect.center() - base * scale / 2.;
    let mut preview = Annotation::new(
        0,
        [0.0, 0.0, base.x, base.y],
        asset.kind.clone(),
        asset.style.clone(),
    );
    preview.style.width = (1.8 / scale).max(preview.style.width);
    paint_annotation(
        editor,
        ctx,
        &painter.with_clip_rect(rect),
        Rect::from_min_size(origin, base * scale),
        scale,
        &preview,
    );
}
fn color(rgb: [f32; 3], opacity: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        (rgb[0] * 255.0) as u8,
        (rgb[1] * 255.0) as u8,
        (rgb[2] * 255.0) as u8,
        (opacity * 255.0) as u8,
    )
}
fn transform_controls(ui: &mut egui::Ui) -> Option<selection::Transform> {
    use selection::Transform;
    let mut action = None;
    for (icon, label, transform) in [
        (Icon::RotateLeft, "Rotate −90°", Transform::Rotate(-90.)),
        (Icon::RotateRight, "Rotate 90°", Transform::Rotate(90.)),
        (
            Icon::FlipHorizontal,
            "Flip horizontal",
            Transform::FlipHorizontal,
        ),
        (Icon::FlipVertical, "Flip vertical", Transform::FlipVertical),
    ] {
        if ui.add(ActionButton::new(icon, label).compact()).clicked() {
            action = Some(transform);
        }
    }
    action
}

pub(super) fn paint_annotation(
    editor: &mut EditorState,
    ctx: &egui::Context,
    painter: &egui::Painter,
    page_rect: Rect,
    scale: f32,
    a: &Annotation,
) {
    let point = |p| {
        let p = a.rotated(p);
        page_rect.min + vec2(p[0], p[1]) * scale
    };
    for primitive in ann::primitives(a) {
        match primitive {
            Primitive::Dot { center } => {
                painter.circle_filled(
                    point(center),
                    a.style.width * scale / 2.,
                    color(a.style.color, a.style.opacity),
                );
            }
            Primitive::Path {
                points,
                closed,
                fill,
            } => {
                let mut p: Vec<_> = points.into_iter().map(point).collect();
                if p.len() < 2 {
                    continue;
                }
                if fill {
                    let c = color(a.style.fill.unwrap_or(a.style.color), a.style.opacity);
                    let polygon: Vec<_> = p.iter().map(|p| [p.x, p.y]).collect();
                    let mut mesh = egui::Mesh::default();
                    for triangle in ann::triangulate(&polygon) {
                        let start = mesh.vertices.len() as u32;
                        for p in triangle {
                            mesh.colored_vertex(pos2(p[0], p[1]), c);
                        }
                        mesh.add_triangle(start, start + 1, start + 2);
                    }
                    painter.add(egui::Shape::mesh(mesh));
                } else {
                    if closed {
                        p.push(p[0]);
                    }
                    painter.add(egui::Shape::line(
                        p,
                        Stroke::new(a.style.width * scale, color(a.style.color, a.style.opacity)),
                    ));
                }
            }
            Primitive::Text {
                text,
                rect,
                size,
                centered,
            } => {
                let c = color(a.style.color, a.style.opacity);
                let font =
                    egui::FontId::new(size * scale, egui::FontFamily::Name("annotation".into()));
                let galley = ctx.fonts_mut(|f| {
                    f.layout(
                        text.replace('\t', "    "),
                        font,
                        c,
                        ((rect[2] - rect[0] - 8.0) * scale).max(1.0),
                    )
                });
                let mut p = [rect[0] + 4.0, rect[1] + 4.0];
                if centered {
                    p = [
                        (rect[0] + rect[2]) / 2.0 - galley.size().x / scale / 2.0,
                        (rect[1] + rect[3]) / 2.0 - galley.size().y / scale / 2.0,
                    ];
                }
                if a.flip_x || a.flip_y {
                    // Transform glyph vertices as well as the text origin: mirrored
                    // text must match its saved PDF appearance, not just move sides.
                    for row in &galley.rows {
                        let mut mesh = row.visuals.mesh.clone();
                        for v in &mut mesh.vertices {
                            v.pos = point([
                                p[0] + (row.pos.x + v.pos.x) / scale,
                                p[1] + (row.pos.y + v.pos.y) / scale,
                            ]);
                        }
                        painter.add(egui::Shape::mesh(mesh));
                    }
                } else {
                    painter.add(
                        egui::epaint::TextShape::new(point(p), galley, c)
                            .with_angle(a.rotation.to_radians()),
                    );
                }
            }
            Primitive::Image { png, rect } => {
                let key = mypdf_milestone0::identity::fingerprint(&png);
                if !editor.images.contains_key(&key)
                    && let Ok(image) = assets::decode_png(&png)
                {
                    let rgba = image.thumbnail(1024, 1024).into_rgba8();
                    let size = [rgba.width() as usize, rgba.height() as usize];
                    let texture = ctx.load_texture(
                        key.clone(),
                        egui::ColorImage::from_rgba_unmultiplied(size, &rgba),
                        egui::TextureOptions::LINEAR,
                    );
                    let size = texture.size()[0] * texture.size()[1] * 4;
                    editor.trim_image_cache_to((64 * 1024 * 1024_usize).saturating_sub(size));
                    editor.images.insert(key.clone(), texture);
                }
                if let Some(image) = editor.images.get(&key) {
                    editor
                        .image_used
                        .insert(key.clone(), ctx.cumulative_frame_nr());
                    let mut mesh = egui::Mesh::with_texture(image.id());
                    for (p, uv) in [
                        ([rect[0], rect[1]], [0.0, 0.0]),
                        ([rect[2], rect[1]], [1.0, 0.0]),
                        ([rect[2], rect[3]], [1.0, 1.0]),
                        ([rect[0], rect[3]], [0.0, 1.0]),
                    ] {
                        mesh.vertices.push(egui::epaint::Vertex {
                            pos: point(p),
                            uv: pos2(uv[0], uv[1]),
                            color: Color32::from_white_alpha((a.style.opacity * 255.0) as u8),
                        });
                    }
                    mesh.indices = vec![0, 1, 2, 0, 2, 3];
                    painter.add(mesh);
                }
            }
        }
    }
}

pub(super) fn page_interaction(
    editor: &mut EditorState,
    tab: &mut Tab,
    ui: &mut egui::Ui,
    page: usize,
    page_rect: Rect,
    scale: f32,
    status: &mut Option<(String, bool)>,
) {
    let ctx = ui.ctx();
    let editable = tab.session.as_ref().is_some_and(|s| s.editable);
    if matches!(editor.mode, Mode::Edit | Mode::Forms | Mode::Convert) {
        let painter = ui
            .painter()
            .with_clip_rect(page_rect.intersect(ui.clip_rect()));
        if let Some(s) = &tab.session
            && s.editable
        {
            for a in s.annotations.iter().filter(|a| a.page == page) {
                paint_annotation(editor, ctx, &painter, page_rect, scale, a);
            }
        }
        return;
    }
    let response = ui.interact(
        page_rect,
        egui::Id::new((tab.id, page, "page_actions")),
        egui::Sense::click_and_drag(),
    );
    if editor.mode == Mode::Read {
        text_selection::interact(
            &mut editor.text_selection,
            tab,
            ctx,
            page,
            &response,
            page_rect,
            scale,
        );
    }
    let page_size = tab.pages[page];
    let normal = |p: egui::Pos2| {
        let p = (p - page_rect.min) / scale;
        [
            p.x.clamp(0.0, page_size.width),
            p.y.clamp(0.0, page_size.height),
        ]
    };
    // Library items dragged from the panel land where they are released.
    if editable && matches!(editor.mode, Mode::Read | Mode::Annotate | Mode::Draw) {
        let pointer = ctx.input(|i| i.pointer.latest_pos());
        if let Some(asset) = response.dnd_hover_payload::<Asset>()
            && let Some(pos) = pointer
        {
            let armed = (editor.tool, editor.placed.replace((*asset).clone()));
            editor.tool = Tool::Asset;
            let r = placement(editor, tab, page, normal(pos), true);
            (editor.tool, editor.placed) = armed;
            let mut ghost = Annotation::new(page, r, asset.kind.clone(), asset.style.clone());
            ghost.style.opacity *= 0.55;
            let painter = ui
                .painter()
                .with_clip_rect(page_rect.intersect(ui.clip_rect()));
            paint_annotation(editor, ctx, &painter, page_rect, scale, &ghost);
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if let Some(asset) = response.dnd_release_payload::<Asset>()
            && let Some(pos) = pointer
        {
            if editor.mode == Mode::Read {
                editor.mode = Mode::Annotate;
            }
            editor.placed = Some((*asset).clone());
            editor.tool = Tool::Asset;
            editor.draft = None;
            place(editor, tab, page, normal(pos), true, status);
        }
    }
    let vertex_tool = matches!(editor.tool, Tool::Polyline | Tool::Polygon)
        && matches!(editor.mode, Mode::Draw | Mode::Annotate)
        && editable
        && !(editor.vertices.is_none()
            && response.clicked()
            && response
                .interact_pointer_pos()
                .is_some_and(|p| hit_annotation(tab, page, normal(p), scale).is_some()));
    if vertex_tool {
        let finish_click = response.double_clicked()
            && editor.vertices.as_ref().is_some_and(|v| {
                v.tab == tab.id
                    && v.page == page
                    && v.points.last().is_some_and(|last| {
                        response.interact_pointer_pos().is_some_and(|p| {
                            let p = normal(p);
                            (p[0] - last[0]).hypot(p[1] - last[1]) * scale < 6.
                        })
                    })
            });
        if response.clicked()
            && !finish_click
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let p = normal(pointer);
            if editor
                .vertices
                .as_ref()
                .is_none_or(|v| v.tab != tab.id || v.page != page)
            {
                tab.selected_annotation = None;
                editor.draft = None;
                editor.vertices = Some(VertexPath {
                    tab: tab.id,
                    page,
                    points: vec![],
                    closed: editor.tool == Tool::Polygon,
                });
            }
            let v = editor.vertices.as_mut().unwrap();
            if v.points.len() < 4096
                && v.points
                    .last()
                    .is_none_or(|q| (q[0] - p[0]).hypot(q[1] - p[1]) > 0.5)
            {
                v.points.push(p);
            }
        }
        if editor
            .vertices
            .as_ref()
            .is_some_and(|v| v.tab == tab.id && v.page == page)
        {
            let cancel = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            let finish = finish_click
                || (!ctx.wants_keyboard_input()
                    && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)));
            if cancel {
                editor.vertices = None;
            } else if finish {
                let v = editor.vertices.take().unwrap();
                if v.points.len() >= if v.closed { 3 } else { 2 } {
                    let r = ann::bounds(&v.points);
                    let r = [r[0], r[1], r[2].max(r[0] + 2.), r[3].max(r[1] + 2.)];
                    let points = v
                        .points
                        .iter()
                        .map(|p| [(p[0] - r[0]) / (r[2] - r[0]), (p[1] - r[1]) / (r[3] - r[1])])
                        .collect();
                    let a = Annotation::new(
                        page,
                        r,
                        Kind::Polygon {
                            points,
                            closed: v.closed,
                        },
                        editor.style.clone(),
                    );
                    commit_new(editor, tab, a, status);
                } else {
                    *status = Some(("Add more points before finishing this shape".into(), false));
                    editor.vertices = Some(v);
                }
            } else if !ctx.wants_keyboard_input()
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace))
            {
                editor.vertices.as_mut().unwrap().points.pop();
            }
        }
        let painter = ui
            .painter()
            .with_clip_rect(page_rect.intersect(ui.clip_rect()));
        if let Some(v) = &editor.vertices
            && v.tab == tab.id
            && v.page == page
        {
            let mut points: Vec<_> = v
                .points
                .iter()
                .map(|p| page_rect.min + vec2(p[0], p[1]) * scale)
                .collect();
            for p in &points {
                painter.circle_filled(*p, 3., theme::accent());
            }
            if response.hovered()
                && let Some(p) = ctx.pointer_hover_pos()
            {
                points.push(p);
            }
            if v.closed && points.len() > 2 {
                points.push(points[0]);
            }
            painter.add(egui::Shape::line(points, Stroke::new(1.5, theme::accent())));
        }
    }
    let multitouch = ctx.input(|i| i.multi_touch().is_some_and(|t| t.num_touches > 1));
    if multitouch {
        editor.touch_navigation = true;
    }
    let navigation = editor.touch_navigation;
    if navigation {
        editor.gesture = None;
        editor.pressed = None;
        if !multitouch && !ctx.input(|i| i.pointer.primary_down()) {
            editor.touch_navigation = false;
        }
    }
    let first_press = ctx.input(|i| {
        i.events.iter().find_map(|event| {
            if let egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                ..
            } = event
            {
                Some(*pos)
            } else {
                None
            }
        })
    });
    if !navigation
        && (response.is_pointer_button_down_on()
            || response.drag_started()
            || response.drag_stopped()
            || first_press.is_some_and(|p| page_rect.contains(p) && response.contains_pointer()))
        && (first_press.is_some()
            || editor
                .pressed
                .is_none_or(|(id, p, _)| id != tab.id || p != page))
        && let Some(pos) = first_press.or(ctx.input(|i| i.pointer.press_origin()))
    {
        editor.pressed = Some((tab.id, page, normal(pos)));
    }
    if response.clicked()
        && editor.mode == Mode::Sign
        && editor.tool == Tool::SignArea
        && let Some(pointer) = response.interact_pointer_pos()
    {
        // A click places a default-size box; digital signing sizes it.
        let p = normal(pointer);
        editor.pending_sign_rect = Some((tab.id, page, [p[0], p[1], p[0], p[1]]));
        editor.tool = Tool::Select;
    }
    if response.clicked()
        && !vertex_tool
        && matches!(editor.mode, Mode::Read | Mode::Draw | Mode::Annotate)
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let p = normal(pointer);
        let shift = ctx.input(|i| i.modifiers.shift);
        if let Some(a) = hit_annotation(tab, page, p, scale) {
            if shift && matches!(editor.mode, Mode::Annotate | Mode::Draw) {
                selection::toggle(tab, editor, &a);
            } else {
                selection::set(tab, editor, vec![a.id.clone()]);
                if matches!(a.kind, Kind::Note { .. }) {
                    editor.note_requested = Some((tab.id, a.id.clone()));
                }
            }
            editor.library_open = false;
            if editor.tool != Tool::Lasso {
                editor.tool = Tool::Select;
            }
        } else if matches!(editor.mode, Mode::Annotate | Mode::Draw)
            && editable
            && matches!(editor.tool, Tool::Note | Tool::Text | Tool::Asset)
        {
            place(editor, tab, page, p, false, status);
        } else if !shift {
            selection::set(tab, editor, vec![]);
        }
    }
    let quick_release = !navigation
        && ctx.input(|i| i.pointer.button_released(egui::PointerButton::Primary))
        && editor
            .pressed
            .filter(|(id, p, _)| *id == tab.id && *p == page)
            .is_some_and(|(_, _, start)| {
                ctx.input(|i| i.pointer.latest_pos()).is_some_and(|p| {
                    let p = normal(p);
                    (start[0] - p[0]).hypot(start[1] - p[1]) * scale > 6.0
                })
            });
    let selecting = editor.marquee.is_some() || editor.group_gesture.is_some();
    if !navigation && !vertex_tool
        && (response.drag_started()
            || (quick_release && editor.gesture.is_none() && !selecting))
        && let Some(p) = editor
            .pressed
            .filter(|(id, p, _)| *id == tab.id && *p == page)
            .map(|(_, _, p)| p)
            .or_else(|| {
                ctx.input(|i| i.pointer.press_origin())
                    .or(response.interact_pointer_pos())
                    .map(normal)
            })
        // A marquee, lasso or group drag claims the press first.
        && !selection::begin(
            editor,
            tab,
            page,
            p,
            scale,
            ctx.input(|i| i.modifiers),
        )
    {
        let original = if editor.tool == Tool::Select {
            // The resize handle belongs to the rotated bounding box, which can lie outside the mark itself.
            tab.selected_annotation
                .as_ref()
                .and_then(|id| {
                    tab.session
                        .as_ref()?
                        .annotations
                        .iter()
                        .find(|a| {
                            &a.id == id
                                && a.page == page
                                && !a.object.hidden
                                && !editor.hide_guides
                                && (corner_at(a, p, scale).is_some()
                                    || manipulation::endpoint_at(a, p, scale).is_some()
                                    || manipulation::rotation_at(a, p, scale))
                        })
                        .cloned()
                })
                .or_else(|| hit_annotation(tab, page, p, scale))
        } else {
            None
        };
        let endpoint = original
            .as_ref()
            .filter(|_| !editor.hide_guides)
            .and_then(|a| manipulation::endpoint_at(a, p, scale));
        let rotating = original
            .as_ref()
            .is_some_and(|a| !editor.hide_guides && manipulation::rotation_at(a, p, scale));
        let resize = original
            .as_ref()
            .filter(|a| Some(&a.id) == tab.selected_annotation.as_ref() && !editor.hide_guides)
            .and_then(|a| corner_at(a, p, scale));
        if (matches!(editor.mode, Mode::Annotate | Mode::Draw)
            && editable
            && (editor.tool != Tool::Select || original.is_some())
            && original.as_ref().is_none_or(|a| !a.object.locked))
            || (editor.mode == Mode::Sign && editor.tool == Tool::SignArea)
        {
            editor.gesture = Some(Gesture {
                tab: tab.id,
                page,
                start: p,
                points: vec![p],
                original,
                rotate: rotating,
                resize,
                endpoint,
                duplicate: ctx.input(|i| i.modifiers.alt)
                    && endpoint.is_none()
                    && resize.is_none()
                    && !rotating,
                guides: Default::default(),
                preview: None,
            });
        }
    }
    if let Some(mut gesture) = editor.gesture.take() {
        if gesture.tab == tab.id
            && gesture.page == page
            && let Some(pointer) = response.interact_pointer_pos().or_else(|| {
                quick_release
                    .then(|| ctx.input(|i| i.pointer.latest_pos()))
                    .flatten()
            })
        {
            let p = normal(pointer);
            if editor.tool == Tool::Pen && gesture.original.is_none() {
                for point in stroke_events(ctx).into_iter().map(normal) {
                    if gesture.points.len() < 100_000 && gesture.points.last() != Some(&point) {
                        gesture.points.push(point);
                    }
                }
            }
            if gesture.points.len() < 100_000
                && gesture
                    .points
                    .last()
                    .is_none_or(|last| (last[0] - p[0]).hypot(last[1] - p[1]) > 0.5)
            {
                gesture.points.push(p);
            }
            let mut preview = if let Some(original) = &gesture.original {
                let mut next = original.clone();
                if gesture.rotate {
                    next = manipulation::rotated_drag(
                        original,
                        gesture.start,
                        p,
                        ctx.input(|i| i.modifiers.shift),
                    );
                } else if let Some(end) = gesture.endpoint {
                    next = manipulation::move_endpoint(
                        original,
                        end,
                        p,
                        ctx.input(|i| i.modifiers.shift),
                    );
                } else if let Some(corner) = gesture.resize {
                    let proportional =
                        keeps_proportions(&original.kind) || ctx.input(|i| i.modifiers.shift);
                    next.rect = resized(original.rect, corner, p, proportional);
                } else {
                    let w = next.rect[2] - next.rect[0];
                    let h = next.rect[3] - next.rect[1];
                    let mut delta = [p[0] - gesture.start[0], p[1] - gesture.start[1]];
                    if ctx.input(|i| i.modifiers.shift) {
                        if delta[0].abs() > delta[1].abs() {
                            delta[1] = 0.;
                        } else {
                            delta[0] = 0.;
                        }
                    }
                    let x = (original.rect[0] + delta[0])
                        .clamp(0.0, (tab.pages[page].width - w).max(0.0));
                    let y = (original.rect[1] + delta[1])
                        .clamp(0.0, (tab.pages[page].height - h).max(0.0));
                    next.rect = [x, y, x + w, y + h];
                    gesture.guides = Default::default();
                    if editor.snapping
                        && !ctx
                            .input(|i| i.modifiers.command || i.modifiers.ctrl || i.modifiers.shift)
                    {
                        let (d, guides) = manipulation::snap(
                            tab,
                            page,
                            next.bounds(),
                            std::slice::from_ref(&original.id),
                            scale,
                        );
                        next = selection::moved(&[next], d, page_size).remove(0);
                        gesture.guides = guides;
                    }
                }
                next
            } else {
                let r = if editor.tool == Tool::Pen {
                    let b = ann::bounds(&gesture.points);
                    [b[0], b[1], (b[2]).max(b[0] + 2.0), (b[3]).max(b[1] + 2.0)]
                } else {
                    [
                        gesture.start[0].min(p[0]),
                        gesture.start[1].min(p[1]),
                        gesture.start[0]
                            .max(p[0])
                            .max(gesture.start[0].min(p[0]) + 2.0),
                        gesture.start[1]
                            .max(p[1])
                            .max(gesture.start[1].min(p[1]) + 2.0),
                    ]
                };
                editor.base_annotation(tab, page, r, gesture.start, p, &gesture.points)
            };
            if editor.tool == Tool::SignArea {
                preview.style.color = [0.16, 0.41, 0.96];
            }
            gesture.preview = Some(preview);
        }
        if gesture.tab == tab.id
            && gesture.page == page
            && (response.drag_stopped() || quick_release)
        {
            if let Some(a) = gesture.preview {
                if editor.tool == Tool::SignArea {
                    editor.pending_sign_rect = Some((tab.id, page, a.rect));
                    editor.tool = Tool::Select;
                } else if gesture.duplicate && gesture.original.is_some() {
                    let mut copy = a.clone();
                    copy.id = format!("{:032x}", rand::random::<u128>());
                    copy.object.group = None;
                    commit_new(editor, tab, copy, status);
                } else if gesture.original.is_some() {
                    if let Some(s) = &mut tab.session {
                        let mut entries = s.annotations.clone();
                        if let Some(target) = entries.iter_mut().find(|v| v.id == a.id) {
                            *target = a.clone();
                        }
                        match s.commit(entries) {
                            Ok(()) => selection::set(tab, editor, vec![a.id.clone()]),
                            Err(e) => *status = Some((format!("{e:#}"), true)),
                        }
                    }
                } else if !matches!(editor.tool, Tool::Pen) || gesture.points.len() > 1 {
                    commit_new(editor, tab, a, status);
                }
            }
        } else {
            editor.gesture = Some(gesture);
        }
    }
    if editor.marquee.is_some() || editor.group_gesture.is_some() {
        let pointer = response
            .interact_pointer_pos()
            .or_else(|| {
                quick_release
                    .then(|| ctx.input(|i| i.pointer.latest_pos()))
                    .flatten()
            })
            .map(normal);
        let released =
            response.drag_stopped() || quick_release || !ctx.input(|i| i.pointer.primary_down());
        selection::update(
            editor,
            tab,
            page,
            pointer,
            released,
            scale,
            ctx.input(|i| i.modifiers),
            status,
        );
    }
    let painter = ui
        .painter()
        .with_clip_rect(page_rect.intersect(ui.clip_rect()));
    if let Some(s) = &tab.session
        && s.editable
    {
        for a in s.annotations.iter().filter(|a| a.page == page) {
            let preview = editor
                .gesture
                .as_ref()
                .filter(|g| g.tab == tab.id && g.page == page && !g.duplicate)
                .and_then(|g| g.preview.as_ref())
                .filter(|p| p.id == a.id);
            let actual = selection::preview(editor, tab.id, page, &a.id)
                .or(preview)
                .cloned()
                .unwrap_or_else(|| a.clone());
            paint_annotation(editor, ctx, &painter, page_rect, scale, &actual);
        }
    }
    if let Some(a) = editor
        .gesture
        .as_ref()
        .filter(|g| g.tab == tab.id && g.page == page && (g.original.is_none() || g.duplicate))
        .and_then(|g| g.preview.clone())
    {
        paint_annotation(editor, ctx, &painter, page_rect, scale, &a);
    }
    if let Some(g) = &editor.gesture
        && g.tab == tab.id
        && g.page == page
    {
        g.guides.paint(&painter, page_rect, scale);
    }
    for copy in selection::duplicates(editor, tab.id, page) {
        paint_annotation(editor, ctx, &painter, page_rect, scale, &copy);
    }
    selection::guides(editor, tab.id, page).paint(&painter, page_rect, scale);
    // The outline and handle must follow the same preview as the artwork in
    // this frame, including a drag that picks up a different selected object.
    let single = editor
        .gesture
        .as_ref()
        .filter(|g| g.tab == tab.id && g.page == page && g.original.is_some())
        .and_then(|g| g.preview.as_ref())
        .or_else(|| {
            tab.session.as_ref().and_then(|s| {
                s.annotations
                    .iter()
                    .find(|a| Some(&a.id) == tab.selected_annotation.as_ref() && a.page == page)
            })
        });
    if let Some(a) = single.filter(|a| !a.object.hidden && !editor.hide_guides) {
        if !a.object.locked
            && let Some(points) = manipulation::endpoints(a)
        {
            for point in points {
                let p = page_rect.min + vec2(point[0], point[1]) * scale;
                painter.circle_filled(p, 5., Color32::WHITE);
                painter.circle_stroke(p, 5., Stroke::new(1.8, theme::accent()));
            }
        }
        if !a.object.locked
            && let Some(h) = manipulation::rotation_handle(a, scale)
        {
            let b = a.bounds();
            let h = page_rect.min + vec2(h[0], h[1]) * scale;
            painter.line_segment(
                [pos2(h.x, page_rect.top() + b[1] * scale), h],
                Stroke::new(1., theme::accent()),
            );
            painter.circle_filled(h, 7., theme::paper());
            ui::draw_icon(
                &painter,
                Rect::from_center_size(h, vec2(13., 13.)),
                Icon::RotateRight,
                theme::accent(),
            );
            if editor.gesture.as_ref().is_some_and(|g| g.rotate) {
                painter.text(
                    h + vec2(14., 0.),
                    egui::Align2::LEFT_CENTER,
                    format!("{:.0}°", a.rotation),
                    FontId::proportional(12.),
                    theme::accent(),
                );
            }
        }
        let b = a.bounds();
        let r = Rect::from_min_max(
            page_rect.min + vec2(b[0], b[1]) * scale,
            page_rect.min + vec2(b[2], b[3]) * scale,
        );
        let frame = r.expand(3.0);
        painter.rect_stroke(
            frame,
            0.0,
            Stroke::new(1.2, theme::accent()),
            egui::StrokeKind::Middle,
        );
        if !a.object.locked {
            for c in [
                frame.left_top(),
                frame.right_top(),
                frame.right_bottom(),
                frame.left_bottom(),
            ] {
                painter.circle_filled(c, 4.5, Color32::WHITE);
                painter.circle_stroke(c, 4.5, Stroke::new(1.5, theme::accent()));
            }
        }
    }
    // Grab to move, grabbing while moving, diagonal arrows on the corners.
    if matches!(editor.mode, Mode::Annotate | Mode::Draw)
        && editable
        && matches!(editor.tool, Tool::Select | Tool::Lasso)
        && editor.group_gesture.is_none()
        && tab.group.len() < 2
    {
        let dragging = editor
            .gesture
            .as_ref()
            .filter(|g| g.tab == tab.id && g.page == page && g.original.is_some());
        let corner = match dragging {
            Some(g) => g.resize,
            None => response.hover_pos().and_then(|pos| {
                let p = normal(pos);
                single
                    .filter(|a| !a.object.locked && !a.object.hidden && !editor.hide_guides)
                    .and_then(|a| corner_at(a, p, scale))
            }),
        };
        let cursor = match (corner, dragging) {
            (Some(0 | 2), _) => Some(egui::CursorIcon::ResizeNwSe),
            (Some(_), _) => Some(egui::CursorIcon::ResizeNeSw),
            (None, Some(_)) => Some(egui::CursorIcon::Grabbing),
            (None, None) => response
                .hover_pos()
                .and_then(|pos| hit_annotation(tab, page, normal(pos), scale))
                .filter(|a| !a.object.locked)
                .map(|_| egui::CursorIcon::Grab),
        };
        if let Some(cursor) = cursor {
            ctx.set_cursor_icon(cursor);
        }
    }
    if !editor.hide_guides {
        selection::paint(
            editor,
            tab,
            page,
            &painter,
            page_rect,
            scale,
            response.hover_pos(),
        );
    }
    text_selection::paint(
        &editor.text_selection,
        tab,
        &painter,
        page,
        page_rect,
        scale,
    );
    if ctx.input(|i| i.pointer.any_released())
        && editor
            .pressed
            .is_some_and(|(id, p, _)| id == tab.id && p == page)
    {
        editor.pressed = None;
    }
    if matches!(editor.mode, Mode::Annotate | Mode::Draw) && !editable && response.hovered() {
        response.on_hover_text("This signed PDF is protected. Choose “Enable editing…” in the signature bar to annotate it; signatures stay valid for what they signed.");
    }
}
/// Preserve intermediate stylus/mouse samples delivered together in one frame.
/// Hover movement before the press or after release never becomes ink.
fn stroke_events(ctx: &egui::Context) -> Vec<egui::Pos2> {
    ctx.input(|i| {
        let mut active = !i.events.iter().any(|e| {
            matches!(
                e,
                egui::Event::PointerButton {
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                }
            )
        });
        let mut points = Vec::new();
        for event in &i.events {
            match event {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    ..
                } => {
                    if *pressed || active {
                        points.push(*pos);
                    }
                    active = *pressed;
                }
                egui::Event::PointerMoved(pos) if active => points.push(*pos),
                _ => {}
            }
        }
        points
    })
}

fn draw_signature_pad(ui: &mut egui::Ui, editor: &mut EditorState) -> Rect {
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width().max(200.0), 170.0),
        egui::Sense::drag(),
    );
    let normalize = |p: egui::Pos2| {
        [
            ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
            ((p.y - rect.top()) / rect.height()).clamp(0.0, 1.0),
        ]
    };
    let first = ui.input(|i| {
        i.events.iter().find_map(|e| {
            if let egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                ..
            } = e
            {
                Some(*pos)
            } else {
                None
            }
        })
    });
    if (response.is_pointer_button_down_on()
        || response.drag_started()
        || first.is_some_and(|p| rect.contains(p) && response.contains_pointer()))
        && let Some(p) = first.or(ui.input(|i| i.pointer.press_origin()))
    {
        editor.signature_pressed = Some(normalize(p));
    }
    let released = ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));
    if !editor.signature_active
        && editor.signature_strokes.len() < 2000
        && (response.drag_started() || released)
        && let Some(start) = editor.signature_pressed
    {
        editor.signature_strokes.push(vec![start]);
        editor.signature_active = true;
    }
    let count = editor.signature_strokes.iter().map(Vec::len).sum::<usize>();
    if editor.signature_active
        && count < 100_000
        && (response.dragged() || response.drag_stopped() || released)
        && let Some(p) = response
            .interact_pointer_pos()
            .or(ui.input(|i| i.pointer.latest_pos()))
        && let Some(stroke) = editor.signature_strokes.last_mut()
        && stroke.len() < 10_000
    {
        let mut samples = stroke_events(ui.ctx());
        if samples.is_empty() {
            samples.push(p);
        }
        for p in samples
            .into_iter()
            .take((100_000 - count).min(10_000 - stroke.len()))
        {
            let p = normalize(p);
            if stroke.last() != Some(&p) {
                stroke.push(p);
            }
        }
    }
    if released {
        editor.signature_pressed = None;
        editor.signature_active = false;
    }
    ui.painter().rect_filled(rect, 4.0, Color32::WHITE);
    ui.painter().rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, theme::muted()),
        egui::StrokeKind::Inside,
    );
    for stroke in &editor.signature_strokes {
        if stroke.len() > 1 {
            ui.painter().add(egui::Shape::line(
                stroke
                    .iter()
                    .map(|p| rect.min + vec2(p[0] * rect.width(), p[1] * rect.height()))
                    .collect(),
                Stroke::new(
                    editor.signature_ink_width,
                    color(editor.signature_ink_color, 1.),
                ),
            ));
        }
    }
    ui.horizontal(|ui| {
        if ui.button("Undo stroke").clicked() {
            editor.signature_strokes.pop();
        }
        if ui.button("Clear").clicked() {
            editor.signature_strokes.clear();
        }
    });
    rect
}

fn commit_new(
    editor: &mut EditorState,
    tab: &mut Tab,
    a: Annotation,
    status: &mut Option<(String, bool)>,
) {
    if let Some(s) = &mut tab.session {
        let mut entries = s.annotations.clone();
        entries.push(a.clone());
        match s.commit(entries) {
            Ok(()) => {
                tab.group.clear();
                tab.selected_annotation = Some(a.id.clone());
                if matches!(a.kind, Kind::Note { .. }) {
                    editor.note_requested = Some((tab.id, a.id.clone()));
                }
                if let Some(id) = &a.asset_id {
                    let base = editor
                        .library
                        .as_ref()
                        .and_then(|l| l.entries.iter().find(|a| &a.id == id))
                        .cloned()
                        .or_else(|| {
                            editor
                                .builtins
                                .iter()
                                .find(|b| &b.asset.id == id)
                                .map(|b| b.asset.clone())
                        });
                    if let Some(mut asset) = base {
                        asset.last_used = time::OffsetDateTime::now_utc().unix_timestamp();
                        if let Some(l) = &mut editor.library
                            && let Err(e) = l.put(asset)
                        {
                            editor.library_error = Some(format!("{e:#}"));
                        }
                    }
                }
                editor.draft = Some(a);
                *status = None;
            }
            Err(e) => *status = Some((format!("{e:#}"), true)),
        }
    }
}
pub(super) fn hit_annotation(
    tab: &Tab,
    page: usize,
    point: [f32; 2],
    scale: f32,
) -> Option<Annotation> {
    tab.session
        .as_ref()
        .and_then(|s| {
            s.annotations
                .iter()
                .rev()
                .find(|a| a.page == page && a.hit_test(point, 5. / scale))
        })
        .cloned()
}

impl Viewer {
    pub(super) fn selected_image_background(&mut self) {
        let Some(t) = self.tabs.get(self.active) else {
            return;
        };
        let Some(s) = &t.session else {
            return;
        };
        if !s.editable {
            return;
        }
        if let Some(a) = s
            .annotations
            .iter()
            .find(|a| Some(&a.id) == t.selected_annotation.as_ref() && !a.object.locked)
            && let Kind::Image { png } = &a.kind
        {
            self.editor.background = Some(background::Draft::new(
                background::Target::Annotation {
                    tab: t.id,
                    id: a.id.clone(),
                    revision: s.revision(),
                },
                png.clone(),
            ));
        } else {
            self.status = Some(("Select an unlocked image layer first".into(), false));
        }
    }
    pub(super) fn apply_pending_annotation(&mut self) -> bool {
        self.editor.property_edit = None;
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return true;
        };
        let Some(session) = &mut tab.session else {
            return true;
        };
        let Some(draft) = &self.editor.draft else {
            return true;
        };
        if !session.editable || tab.selected_annotation.as_ref() != Some(&draft.id) {
            return true;
        }
        let mut entries = session.annotations.clone();
        let Some(target) = entries.iter_mut().find(|a| a.id == draft.id) else {
            return true;
        };
        if target == draft {
            return true;
        }
        *target = draft.clone();
        match session.commit(entries) {
            Ok(()) => true,
            Err(e) => {
                self.status = Some((
                    format!("Could not apply annotation properties: {e:#}"),
                    true,
                ));
                false
            }
        }
    }
    pub(super) fn editor_undo(&mut self, redo: bool) {
        if self.content.busy || self.organizer.busy {
            return;
        }
        if let Some(tab) = self.tabs.get(self.active)
            && self.content.queue_text_undo(tab.id, redo)
        {
            return;
        }
        if let Some(tab) = self.tabs.get_mut(self.active)
            && let Some(s) = &mut tab.session
        {
            let note_id = tab
                .selected_annotation
                .as_ref()
                .filter(|id| {
                    s.annotations
                        .iter()
                        .any(|a| &a.id == *id && matches!(a.kind, Kind::Note { .. }))
                })
                .cloned();
            let focus = if redo { s.redo_focus() } else { s.undo_focus() };
            if redo {
                s.redo();
            } else {
                s.undo();
            }
            let note = note_id.and_then(|id| {
                s.annotations
                    .iter()
                    .find(|a| a.id == id && !a.object.hidden && matches!(a.kind, Kind::Note { .. }))
                    .cloned()
            });
            if let Some((page, rect)) = focus {
                let page = page.min(tab.pages.len().saturating_sub(1));
                tab.jump(page);
                self.workbench.reveal = Some((tab.id, page, rect, std::time::Instant::now()));
            }
            tab.selected_annotation = note.as_ref().map(|a| a.id.clone());
            tab.group.clear();
            self.editor.draft = note;
            self.editor.property_edit = None;
            self.editor.gesture = None;
            self.editor.marquee = None;
            self.editor.group_gesture = None;
            self.status = None;
        }
    }
    pub(super) fn editor_delete(&mut self) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let ids = selection::selected_ids(tab);
        if ids.is_empty() {
            return;
        }
        if let Some(s) = &mut tab.session {
            match s.commit(
                s.annotations
                    .iter()
                    .filter(|a| !ids.contains(&a.id))
                    .cloned()
                    .collect(),
            ) {
                Ok(()) => {
                    selection::set(tab, &mut self.editor, vec![]);
                    self.status = None;
                }
                Err(e) => self.status = Some((format!("{e:#}"), true)),
            }
        }
    }
    /// Keyboard actions on selected marks in Annotate and Draw. The caller ensures
    /// no text field has focus; an in-progress shape keeps its own keys.
    pub(super) fn selection_keys(&mut self, ctx: &egui::Context) {
        if self.editor.vertices.is_some()
            || self.editor.gesture.is_some()
            || self.editor.marquee.is_some()
            || self.editor.group_gesture.is_some()
        {
            return;
        }
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::A)) {
            let page = tab.page;
            let ids = tab
                .session
                .as_ref()
                .map(|s| {
                    s.annotations
                        .iter()
                        .filter(|a| a.page == page && !a.object.hidden)
                        .map(|a| a.id.clone())
                        .collect()
                })
                .unwrap_or_default();
            selection::set(tab, &mut self.editor, ids);
            return;
        }
        let ids = selection::selected_ids(tab);
        if ids.is_empty() {
            return;
        }
        if ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
        }) {
            self.editor_delete();
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !egui::Popup::is_any_open(ctx) {
            selection::set(tab, &mut self.editor, vec![]);
            return;
        }
        let step = if ctx.input(|i| i.modifiers.shift) {
            10.
        } else {
            1.
        };
        let mut d = [0., 0.];
        for (key, x, y) in [
            (egui::Key::ArrowLeft, -1., 0.),
            (egui::Key::ArrowRight, 1., 0.),
            (egui::Key::ArrowUp, 0., -1.),
            (egui::Key::ArrowDown, 0., 1.),
        ] {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key)) {
                d = [d[0] + x * step, d[1] + y * step];
            }
        }
        if d != [0., 0.] && tab.session.as_ref().is_some_and(|s| s.editable) {
            selection::nudge(tab, d, &mut self.status);
            // Refresh the single-mark inspector draft so it does not undo the nudge.
            selection::set(tab, &mut self.editor, ids);
        }
    }
    pub(super) fn editor_events(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            selection::normalize(tab, &mut self.editor);
        }
        for result in self.editor.signature_rx.try_iter().collect::<Vec<_>>() {
            self.editor.signature_busy = false;
            match result {
                Ok(asset) => {
                    self.editor.signature_name = asset.name.clone();
                    self.editor.signature_image = Some(asset);
                    self.editor.signature_error = None;
                }
                Err(e) => self.editor.signature_error = Some(e),
            }
        }
        for result in self.editor.import_rx.try_iter().collect::<Vec<_>>() {
            self.editor.asset_busy = false;
            match result {
                Ok(asset) => {
                    self.editor.placed = Some(asset.clone());
                    match self.editor.add_asset(asset) {
                        Ok(()) => {
                            self.editor.library_error = None;
                            self.editor.library_open = true;
                        }
                        Err(e) => self.editor.library_error = Some(e),
                    }
                }
                Err(e) => self.editor.library_error = Some(e),
            }
        }
    }
    pub(super) fn editor_tools(&mut self, ctx: &egui::Context) {
        if matches!(
            self.editor.mode,
            Mode::Read | Mode::Pages | Mode::Edit | Mode::Forms | Mode::Convert
        ) {
            return;
        }
        let mut action = None;
        egui::TopBottomPanel::top("annotation_ribbon")
            .frame(
                egui::Frame::new()
                    .fill(theme::surface())
                    .inner_margin(egui::Margin::symmetric(16, 6)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::horizontal().show(ui, |ui| {
                    // Centered on the width this mode's tools took last frame.
                    let measured = egui::Id::new(("ribbon_width", format!("{:?}", self.editor.mode)));
                    let width = ctx.data(|d| d.get_temp::<f32>(measured)).unwrap_or(790.);
                    let inset = ((ui.available_width() - width) / 2.).max(0.);
                    let compact = ui.available_width() < 930.;
                    let row = ui.horizontal(|ui| {
                        ui.add_space(inset);
                        if matches!(self.editor.mode, Mode::Annotate | Mode::Draw) {
                            let editable = self
                                .tabs
                                .get(self.active)
                                .and_then(|t| t.session.as_ref())
                                .is_some_and(|s| s.editable);
                            ui.add_enabled_ui(editable, |ui| {
                                let tools: &[Tool] = if self.editor.mode == Mode::Draw {
                                    &[
                                        Tool::Select,
                                        Tool::Lasso,
                                        Tool::Pen,
                                        Tool::Polyline,
                                        Tool::Polygon,
                                        Tool::Rectangle,
                                        Tool::Ellipse,
                                        Tool::Line,
                                        Tool::Arrow,
                                        Tool::DoubleArrow,
                                    ]
                                } else {
                                    &[
                                        Tool::Select,
                                        Tool::Lasso,
                                        Tool::Highlight,
                                        Tool::Underline,
                                        Tool::Strikeout,
                                        Tool::Text,
                                        Tool::Note,
                                    ]
                                };
                                // New marks use the current color; highlights stay yellow.
                                let ink = if self.editor.style.color == HIGHLIGHT {
                                    DEFAULT_INK
                                } else {
                                    self.editor.style.color
                                };
                                for &tool in tools {
                                    if matches!(tool, Tool::Highlight | Tool::Pen | Tool::Rectangle)
                                    {
                                        ui.separator();
                                    }
                                    let mut button = ActionButton::new(tool.icon(), tool.label())
                                        .compact()
                                        .selected(
                                            self.editor.tool == tool && !self.editor.library_open,
                                        );
                                    // Annotate shows each markup color; Draw only the active tool's.
                                    let tinted = self.editor.mode == Mode::Annotate
                                        || tool == Tool::Pen
                                        || self.editor.tool == tool;
                                    if !matches!(tool, Tool::Select | Tool::Lasso) && tinted {
                                        button = button.tint(color(
                                            if tool == Tool::Highlight { HIGHLIGHT } else { ink },
                                            1.,
                                        ));
                                    }
                                    if ui
                                        .add(button)
                                        .clicked()
                                    {
                                        self.editor.select_tool(tool);
                                        self.editor.library_open = false;
                                        if let Some(tab) = self.tabs.get_mut(self.active) {
                                            tab.signatures.panel = false;
                                        }
                                    }
                                }
                                ui.separator();
                                let libraries: &[(&str, Icon)] = if self.editor.mode == Mode::Draw {
                                    &[
                                        ("Arrows", Icon::Arrow),
                                        ("Symbols", Icon::Star),
                                        ("Saved", Icon::Library),
                                    ]
                                } else {
                                    &[("Stamps", Icon::Stamp), ("Saved", Icon::Library)]
                                };
                                for &(label, icon) in libraries {
                                    if ui
                                        .add({
                                            let b = ActionButton::new(icon, label).selected(
                                                self.editor.library_open
                                                    && self.editor.library_tab == label,
                                            );
                                            if compact { b.compact() } else { b }
                                        })
                                        .clicked()
                                    {
                                        self.inspector = true;
                                        self.editor.library_tab = label.into();
                                        self.editor.library_open = true;
                                    }
                                }
                                ui.separator();
                                if ui.add(ActionButton::new(Icon::AlignObjectsCenter,"Snap to objects").compact().selected(self.editor.snapping)).on_hover_text("Snap edges and centers · ⌘ bypasses snapping while dragging").clicked() { self.editor.snapping = !self.editor.snapping; }
                                ui::color_swatch(
                                    ui,
                                    &mut self.editor.style.color,
                                    "Color for new annotations",
                                );
                            });
                        } else if self.editor.mode == Mode::Sign {
                            action = self.signing_ribbon(ui);
                        } else {
                            ui.label(
                                RichText::new("Read, annotate, sign, or organize your pages.")
                                    .small()
                                    .color(theme::muted()),
                            );
                        }
                    });
                    let used = row.response.rect.width() - inset;
                    if (used - width).abs() > 0.5 {
                        ctx.data_mut(|d| d.insert_temp(measured, used));
                        ctx.request_repaint();
                    }
                    if self.editor.tool == Tool::SignArea {
                        ui.label(RichText::new("Drag a box where your signature goes, or click to place it · Esc cancels").small().color(theme::muted()));
                    }
                    if matches!(self.editor.tool, Tool::Polyline | Tool::Polygon) && self.editor.mode == Mode::Draw {
                        ui.label(RichText::new("Click to add points · Double-click or Return to finish · Backspace removes a point · Esc cancels").small().color(theme::muted()));
                    }
                });
            });
        if let Some(action) = action {
            self.action(action, ctx);
        }
    }
    pub(super) fn annotation_list(&mut self, ui: &mut egui::Ui) {
        self.objects_panel(ui);
    }
    /// Shared properties, alignment and actions for a multi-object selection.
    fn group_inspector(&mut self, ctx: &egui::Context) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let marks: Vec<Annotation> = selection::members(tab).into_iter().cloned().collect();
        let Some(first) = marks.first().cloned() else {
            return;
        };
        let editable = tab.session.as_ref().is_some_and(|s| s.editable)
            && marks.iter().all(|a| !a.object.locked);
        let mut changed: Option<Vec<Annotation>> = None;
        let mut arranged = None;
        let mut duplicate = false;
        let mut delete = false;
        let mut focus = false;
        // Apply one property to every mark, on top of earlier edits this frame.
        let apply = |changed: &mut Option<Vec<Annotation>>, edit: &dyn Fn(&mut Annotation)| {
            let next = changed.get_or_insert_with(|| marks.clone());
            for a in next.iter_mut() {
                edit(a);
            }
        };
        egui::SidePanel::right("annotation_inspector")
            .default_width(254.)
            .width_range(230.0..=340.0)
            .resizable(true)
            .frame(egui::Frame::new().fill(theme::paper()).inner_margin(16.))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = vec2(8., 10.);
                egui::ScrollArea::vertical()
                    .id_salt("group_properties")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui::title(ui, &format!("{} objects", marks.len()), 17.);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(format!("PAGE {}", first.page + 1))
                                            .size(10.)
                                            .color(theme::muted()),
                                    );
                                },
                            );
                        });
                        ui.separator();
                        ui.add_enabled_ui(editable, |ui| {
                            ui.horizontal(|ui| {
                                if ui.add(ActionButton::new(Icon::Group, "Group")).clicked() { self.workbench.request=Some("group_objects".into()); }
                                if ui.add_enabled(marks.iter().any(|a|a.object.group.is_some()), ActionButton::new(Icon::Ungroup,"Ungroup")).clicked() { self.workbench.request=Some("ungroup_objects".into()); }
                            });
                            if let Some(styles::Preset::Drawing(style))=styles::panel(ui,styles::Preset::Drawing(first.style.clone())) { apply(&mut changed,&|a|a.style=style.clone()); }
                            ui::caption(ui, "Color");
                            let mut color = first.style.color;
                            if color_palette(ui, &mut color) {
                                apply(&mut changed, &|a| {
                                    if !matches!(a.kind, Kind::Image { .. }) {
                                        a.style.color = color;
                                    }
                                    if let Kind::Svg { tint, .. } = &mut a.kind {
                                        *tint = true;
                                    }
                                });
                            }
                            let mut opacity = first.style.opacity * 100.;
                            focus |= property_slider(
                                ui,
                                "Opacity",
                                &mut opacity,
                                5.0..=100.0,
                                "%",
                            )
                            .is_some();
                            if opacity != first.style.opacity * 100. {
                                apply(&mut changed, &|a| a.style.opacity = opacity / 100.);
                            }
                            let mut width = first.style.width;
                            focus |= property_slider(ui, "Stroke", &mut width, 0.5..=20., " pt")
                                .is_some();
                            if width != first.style.width {
                                apply(&mut changed, &|a| a.style.width = width);
                            }
                            let mut stroke = first.style.stroke;
                            stroke_styles(ui, &mut stroke);
                            if stroke != first.style.stroke {
                                apply(&mut changed, &|a| a.style.stroke = stroke);
                            }
                            if marks.iter().any(|a| matches!(a.kind, Kind::Rectangle | Kind::Ellipse | Kind::Polygon { closed: true, .. } | Kind::Text { .. } | Kind::Stamp { .. })) {
                                let mut fill = first.style.fill;
                                ui.horizontal(|ui| {
                                    let mut filled = fill.is_some();
                                    if ui.checkbox(&mut filled, "Fill").changed() {
                                        fill = filled.then_some(first.style.color);
                                    }
                                    if let Some(color) = &mut fill {
                                        ui::color_swatch(ui, color, "Fill color");
                                    }
                                });
                                if fill != first.style.fill {
                                    apply(&mut changed, &|a| {
                                        if matches!(a.kind, Kind::Rectangle | Kind::Ellipse | Kind::Polygon { closed: true, .. } | Kind::Text { .. } | Kind::Stamp { .. }) {
                                            a.style.fill = fill;
                                        }
                                    });
                                }
                            }
                            if marks.iter().any(|a| matches!(a.kind, Kind::Text { .. } | Kind::Stamp { .. })) {
                                let mut size = first.style.font_size;
                                focus |= property_slider(ui, "Text size", &mut size, 6.0..=144.0, " pt").is_some();
                                if size != first.style.font_size {
                                    apply(&mut changed, &|a| {
                                        if matches!(a.kind, Kind::Text { .. } | Kind::Stamp { .. }) {
                                            a.style.font_size = size;
                                        }
                                    });
                                }
                            }
                            ui.separator();
                            ui::caption(ui, "Rotate & flip");
                            ui.horizontal(|ui| {
                                if let Some(transform) = transform_controls(ui) {
                                    let next = changed.get_or_insert_with(|| marks.clone());
                                    *next = selection::transformed(next, transform);
                                }
                            });
                            let mut rotation = first.rotation;
                            focus |= property_slider(ui, "Rotation", &mut rotation, -180.0..=180.0, "°").is_some();
                            if rotation != first.rotation {
                                let next = changed.get_or_insert_with(|| marks.clone());
                                *next = selection::transformed(next, selection::Transform::Rotate(rotation - first.rotation));
                            }
                            ui.separator();
                            ui::caption(ui, "Align");
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 2.;
                                for (align, icon, label) in selection::Align::ALL {
                                    if ui.add(ActionButton::new(icon, label).compact()).clicked() {
                                        arranged = Some(selection::aligned(&marks, align));
                                    }
                                }
                            });
                            ui.horizontal_wrapped(|ui| {
                                for (axis, label) in [(0, "Equal horizontal gaps"), (1, "Equal vertical gaps")] {
                                    if ui.add_enabled(marks.len() >= 3, egui::Button::new(label)).clicked() {
                                        arranged = Some(selection::distributed(&marks, axis));
                                    }
                                }
                                for (w,h,label) in [(true,false,"Same width"),(false,true,"Same height"),(true,true,"Same size")] {
                                    if ui.button(label).on_hover_text("Match the first object in document order; keep centers in place").clicked() {
                                        arranged = Some(selection::matched_size(&marks,w,h));
                                    }
                                }
                            });
                            ui.separator();
                            ui.horizontal(|ui| {
                                duplicate =
                                    ui.add(ActionButton::new(Icon::Plus, "Duplicate")).clicked();
                                delete = ui
                                    .add(ActionButton::new(Icon::Trash, "Delete").compact())
                                    .clicked();
                            });
                        });
                        if !editable {
                            ui.label(RichText::new("Read-only document").small().color(theme::muted()));
                        }
                        ui.label(
                            RichText::new(
                                "Drag to move · Corners to scale · Shift-click to add or remove · Arrow keys nudge",
                            )
                            .size(11.)
                            .color(theme::muted()),
                        );
                    });
            });
        self.editor.text_editing |= focus;
        let pointer_active = ctx.input(|i| i.pointer.any_down() || i.pointer.any_released());
        if let Some(next) = changed {
            // A slider drag or a focused value field is one undo step.
            let previous = self
                .editor
                .group_edit
                .filter(|(id, _)| *id == tab.id && (pointer_active || focus))
                .map(|(_, revision)| revision);
            if let Some(revision) = selection::commit(tab, &next, previous, &mut self.status) {
                self.editor.group_edit = Some((tab.id, revision));
            }
        }
        if !ctx.input(|i| i.pointer.any_down()) && !focus {
            self.editor.group_edit = None;
        }
        if let Some(next) = arranged {
            if next.iter().any(|a| {
                let b = a.bounds();
                let p = tab.pages[a.page];
                b[0] < 0. || b[1] < 0. || b[2] > p.width || b[3] > p.height
            }) {
                self.status=Some(("This arrangement would extend beyond the page. Move the objects inward or choose a smaller reference object.".into(),true));
            } else {
                selection::commit(tab, &next, None, &mut self.status);
            }
        }
        if duplicate && let Some(s) = &mut tab.session {
            let copies: Vec<_> = mypdf_milestone0::object_tools::duplicate(&marks);
            let mut entries = s.annotations.clone();
            entries.extend(copies.iter().cloned());
            match s.commit(entries) {
                Ok(()) => {
                    let ids = copies.into_iter().map(|a| a.id).collect();
                    selection::set(tab, &mut self.editor, ids);
                }
                Err(e) => self.status = Some((format!("{e:#}"), true)),
            }
        }
        if delete {
            self.editor_delete();
        }
    }
    pub(super) fn annotation_inspector(&mut self, ctx: &egui::Context) {
        if let Some(note) = self.selected_note() {
            self.note_inspector(ctx, note);
            return;
        }
        if self
            .tabs
            .get(self.active)
            .is_some_and(|t| t.group.len() >= 2)
        {
            self.group_inspector(ctx);
            return;
        }
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let Some(selected) = tab
            .selected_annotation
            .as_ref()
            .and_then(|id| {
                tab.session
                    .as_ref()?
                    .annotations
                    .iter()
                    .find(|a| &a.id == id)
            })
            .cloned()
        else {
            if matches!(self.editor.mode, Mode::Draw | Mode::Annotate) {
                egui::SidePanel::right("annotation_inspector")
                    .default_width(254.)
                    .width_range(230.0..=340.0)
                    .resizable(true)
                    .frame(egui::Frame::new().fill(theme::paper()).inner_margin(16.))
                    .show(ctx, |ui| {
                        ui.spacing_mut().item_spacing = vec2(8., 10.);
                        let drawing = self.editor.mode == Mode::Draw;
                        let title = if matches!(self.editor.tool, Tool::Select | Tool::Lasso) {
                            if drawing { "Drawings" } else { "Annotations" }
                        } else { self.editor.tool.label() };
                        let hint = match self.editor.tool {
                            Tool::Highlight | Tool::Underline | Tool::Strikeout => "Drag across PDF text to mark it. Click an existing annotation to edit its appearance.",
                            Tool::Note => "Click the page to add a note. Click an existing note to edit it.",
                            Tool::Text => "Click or drag on the page to add a text box.",
                            Tool::Pen => "Draw on the page with your mouse, pen, or touch input.",
                            Tool::Polyline | Tool::Polygon => "Click to add corners. Double-click the last point to finish.",
                            Tool::Asset => "Choose artwork from the library, then click or drag on the page to place it.",
                            Tool::Select | Tool::Lasso if drawing => "Click a drawing to select it, or drag across several.",
                            Tool::Select | Tool::Lasso => "Select an annotation on the page, or choose a tool above to add one.",
                            _ => "Drag on the page to create this shape.",
                        };
                        ui.label(RichText::new(title).size(17.).family(ui::semibold()));
                        ui.label(
                            RichText::new(hint)
                                .small()
                                .color(theme::muted()),
                        );
                        ui.separator();
                        ui::caption(ui, if drawing { "New drawings" } else { "New annotations" });
                        color_palette(ui, &mut self.editor.style.color);
                        if drawing || self.editor.tool == Tool::Pen {
                        self.editor.text_editing |= property_slider(
                            ui,
                            "Stroke",
                            &mut self.editor.style.width,
                            0.5..=20.,
                            " pt",
                        )
                        .is_some();
                        stroke_styles(ui, &mut self.editor.style.stroke);
                        }
                    });
            }
            return;
        };
        if self
            .editor
            .draft
            .as_ref()
            .is_none_or(|a| a.id != selected.id)
        {
            self.editor.draft = Some(selected.clone());
        }
        let mut property_focus = None;
        if ctx.input(|i| i.pointer.any_pressed()) {
            self.editor.property_edit = None;
        }
        let mut duplicate = false;
        let mut delete = false;
        let mut copy_image = false;
        let mut save_asset = false;
        let mut edit_svg = None;
        let mut update_library = false;
        let standard_asset = selected
            .asset_id
            .as_ref()
            .is_some_and(|id| self.editor.builtins.iter().any(|b| &b.asset.id == id));
        egui::SidePanel::right("annotation_inspector")
            .default_width(254.)
            .width_range(230.0..=340.0)
            .resizable(true)
            .frame(egui::Frame::new().fill(theme::paper()).inner_margin(16.))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = vec2(8., 10.);
                ui.spacing_mut().interact_size.y = 26.;
                ui.spacing_mut().button_padding = vec2(8., 4.);
                egui::ScrollArea::vertical()
                    .id_salt("annotation_properties")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(selected.kind.label())
                                    .size(17.)
                                    .family(ui::semibold()),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(format!("PAGE {}", selected.page + 1))
                                            .size(10.)
                                            .color(theme::muted()),
                                    );
                                },
                            );
                        });
                        ui.separator();
                        if matches!(selected.kind, Kind::Image { .. }) {
                            copy_image = ui.add(ActionButton::new(Icon::Copy, "Copy image"))
                                .on_hover_text("Paste this image, including its appearance, onto another page or another open PDF")
                                .clicked();
                            if ui.add_enabled(!selected.object.locked, ActionButton::new(Icon::Crop, "Crop image…")).clicked() {
                                self.workbench.request = Some("crop_object".into());
                            }
                            ui.separator();
                        }
                        if ui.add(ActionButton::new(Icon::Copy, "Repeat on pages…")).clicked() { self.workbench.request = Some("repeat_objects".into()); }
                        if selected.object.locked { ui.weak("Locked · Unlock in the Marks panel"); }
                        let editable = tab.session.as_ref().is_some_and(|s| s.editable) && !selected.object.locked;
                        ui.add_enabled_ui(editable, |ui| {
                            let a = self.editor.draft.as_mut().unwrap();
                            if let Some(styles::Preset::Drawing(style))=styles::panel(ui, styles::Preset::Drawing(a.style.clone())) { a.style=style; }

                            if let Some(text) = a.kind.text_mut() {
                                let r = ui.add(
                                    egui::TextEdit::multiline(text)
                                        .margin(ui::FIELD_MARGIN)
                                        .desired_rows(3)
                                        .char_limit(8000)
                                        .desired_width(f32::INFINITY),
                                );
                                self.editor.text_editing |= r.has_focus();
                                if r.has_focus() {
                                    property_focus = Some(r.id);
                                }
                            }
                            let image = matches!(a.kind, Kind::Image { .. });
                            if let Kind::Image { png } = &a.kind
                                && ui.add(ActionButton::new(Icon::FieldImage, "Remove background…")).clicked()
                            {
                                self.editor.background = Some(background::Draft::new(
                                    background::Target::Annotation { tab: tab.id, id: a.id.clone(), revision: tab.session.as_ref().unwrap().revision() }, png.clone()));
                            }
                            if !image {
                                ui::caption(ui, "Color");
                                if color_palette(ui, &mut a.style.color)
                                    && let Kind::Svg { tint, .. } = &mut a.kind
                                {
                                    *tint = true;
                                }
                            }
                            if let Kind::Svg { source, tint } = &mut a.kind {
                                ui.checkbox(tint, "Use selected color");
                                if ui.add(ActionButton::new(Icon::Text, "Edit SVG…")).clicked() {
                                    edit_svg = Some((source.clone(), a.id.clone()));
                                }
                            }
                            if matches!(
                                a.kind,
                                Kind::Rectangle
                                    | Kind::Ellipse
                                    | Kind::Polygon { closed: true, .. }
                                    | Kind::Text { .. }
                                    | Kind::Stamp { .. }
                            ) {
                                ui.horizontal(|ui| {
                                    let mut fill = a.style.fill.is_some();
                                    if ui.checkbox(&mut fill, "Fill").changed() {
                                        a.style.fill = fill.then_some(a.style.color);
                                    }
                                    if let Some(fill) = &mut a.style.fill {
                                        ui::color_swatch(ui, fill, "Fill color");
                                    }
                                });
                            }
                            let mut opacity = a.style.opacity * 100.;
                            property_focus =
                                property_slider(ui, "Opacity", &mut opacity, if image { 0.0..=100.0 } else { 5.0..=100.0 }, "%")
                                    .or(property_focus);
                            a.style.opacity = opacity / 100.;
                            if image {
                                property_focus = property_slider(ui, "Blur", &mut a.style.image_blur, 0.0..=32.0, " px").or(property_focus);
                                ui.checkbox(&mut a.style.image_border, "Border");
                                if a.style.image_border {
                                    color_palette(ui, &mut a.style.color);
                                    property_focus = property_slider(ui, "Border width", &mut a.style.width, 0.5..=20.0, " pt").or(property_focus);
                                    stroke_styles(ui, &mut a.style.stroke);
                                }
                                ui.label(RichText::new("Blur keeps the original image. Set it to 0 to restore detail.").small().color(theme::muted()));
                            }
                            if !matches!(
                                a.kind,
                                Kind::Svg { .. } | Kind::Image { .. } | Kind::Text { .. }
                            ) {
                                property_focus = property_slider(
                                    ui,
                                    "Stroke",
                                    &mut a.style.width,
                                    0.5..=20.0,
                                    " pt",
                                )
                                .or(property_focus);
                                ui.horizontal(|ui| {
                                    for width in [1., 2., 4., 6.] {
                                        let (r, response) = ui.allocate_exact_size(
                                            vec2(42., 26.),
                                            egui::Sense::click(),
                                        );
                                        let selected = (a.style.width - width).abs() < 0.01;
                                        ui.painter().rect_filled(
                                            r,
                                            5.,
                                            if selected {
                                                theme::accent_soft()
                                            } else {
                                                theme::control()
                                            },
                                        );
                                        ui.painter().line_segment(
                                            [
                                                r.left_center() + vec2(9., 0.),
                                                r.right_center() - vec2(9., 0.),
                                            ],
                                            Stroke::new(
                                                width,
                                                if selected {
                                                    theme::accent()
                                                } else {
                                                    theme::ink()
                                                },
                                            ),
                                        );
                                        response.widget_info(|| {
                                            egui::WidgetInfo::selected(
                                                egui::WidgetType::Button,
                                                ui.is_enabled(),
                                                selected,
                                                format!("Stroke {width} points"),
                                            )
                                        });
                                        if response.on_hover_text(format!("{width} pt")).clicked() {
                                            a.style.width = width;
                                        }
                                    }
                                });
                                stroke_styles(ui, &mut a.style.stroke);
                            }
                            if matches!(a.kind, Kind::Text { .. } | Kind::Stamp { .. }) {
                                property_focus = property_slider(
                                    ui,
                                    "Text size",
                                    &mut a.style.font_size,
                                    6.0..=72.0,
                                    " pt",
                                )
                                .or(property_focus);
                            }
                            ui.separator();
                            ui::caption(ui, "Rotate & flip");
                            ui.horizontal(|ui| {
                                if let Some(transform) = transform_controls(ui) {
                                    *a = selection::transformed(std::slice::from_ref(a), transform).remove(0);
                                }
                            });
                            ui.separator();
                            egui::CollapsingHeader::new("Position & size").show(ui, |ui| {
                                property_focus = property_slider(
                                    ui,
                                    "Rotation",
                                    &mut a.rotation,
                                    -180.0..=180.0,
                                    "°",
                                )
                                .or(property_focus);
                                let mut x = a.rect[0];
                                let mut y = a.rect[1];
                                let mut w = a.rect[2] - x;
                                let mut h = a.rect[3] - y;
                                egui::Grid::new("annotation_geometry")
                                    .num_columns(4)
                                    .spacing(vec2(8., 8.))
                                    .show(ui, |ui| {
                                        for (index, (label, value)) in [
                                            ("X", &mut x),
                                            ("Y", &mut y),
                                            ("W", &mut w),
                                            ("H", &mut h),
                                        ]
                                        .into_iter()
                                        .enumerate()
                                        {
                                            ui.label(
                                                RichText::new(label).small().color(theme::muted()),
                                            );
                                            let r = ui.add_sized(
                                                vec2(64., 26.),
                                                egui::DragValue::new(value)
                                                    .range(0.0..=10_000.0)
                                                    .max_decimals(1)
                                                    .speed(1.0),
                                            );
                                            r.widget_info(|| {
                                                egui::WidgetInfo::labeled(
                                                    egui::WidgetType::DragValue,
                                                    true,
                                                    match label {
                                                        "X" => "Left",
                                                        "Y" => "Top",
                                                        "W" => "Width",
                                                        _ => "Height",
                                                    },
                                                )
                                            });
                                            self.editor.text_editing |= r.has_focus();
                                            if r.has_focus() {
                                                property_focus = Some(r.id);
                                            }
                                            if index % 2 == 1 {
                                                ui.end_row();
                                            }
                                        }
                                    });
                                a.rect = [x, y, x + w.max(1.), y + h.max(1.)];
                            });
                            ui.horizontal(|ui| {
                                duplicate =
                                    ui.add(ActionButton::new(Icon::Plus, "Duplicate")).clicked();
                                delete = ui
                                    .add(ActionButton::new(Icon::Trash, "Delete").compact())
                                    .clicked();
                            });
                        });
                        if !editable {
                            ui.label(
                                RichText::new(if selected.object.locked { "Unlock in Marks to edit" } else { "Read-only document" })
                                    .small()
                                    .color(theme::muted()),
                            );
                        }
                        ui.separator();
                        egui::CollapsingHeader::new("Save to library").show(ui, |ui| {
                            if selected.asset_id.is_some() {
                                ui.label(RichText::new("Changes on this page affect only this copy.").small().color(theme::muted()));
                                update_library = ui.add(ActionButton::new(
                                    Icon::Library,
                                    if standard_asset { "Save as custom asset" } else { "Update library asset" },
                                )).clicked();
                                ui.separator();
                            }
                            let r = ui.add(
                                egui::TextEdit::singleline(&mut self.editor.asset_name)
                                    .margin(ui::FIELD_MARGIN)
                                    .hint_text("Asset name")
                                    .char_limit(200)
                                    .desired_width(f32::INFINITY),
                            );
                            self.editor.text_editing |= r.has_focus();
                            save_asset = ui
                                .add(ActionButton::new(Icon::Library, "Save selection"))
                                .clicked();
                        });
                        ui.label(
                            RichText::new(
                                if self
                                    .editor
                                    .draft
                                    .as_ref()
                                    .is_some_and(|a| keeps_proportions(&a.kind))
                                {
                                    "Drag to move · Drag a corner to resize in proportion"
                                } else {
                                    "Drag to move · Drag a corner to resize · Shift keeps proportions"
                                },
                            )
                                .size(11.)
                                .color(theme::muted()),
                        );
                    });
            });
        if let Some((source, id)) = edit_svg {
            self.editor.svg = SvgEditor {
                open: true,
                source,
                name: "Edited SVG artwork".into(),
                target: Some((tab.id, id)),
                error: None,
                preview: None,
                changed: Some(std::time::Instant::now()),
                ..SvgEditor::default()
            };
        }
        self.editor.text_editing |= property_focus.is_some();
        let pointer_active = ctx.input(|i| i.pointer.any_down() || i.pointer.any_released());
        let previous = self
            .editor
            .property_edit
            .as_ref()
            .filter(|(id, annotation, _, focus)| {
                *id == tab.id
                    && *annotation == selected.id
                    && (pointer_active || property_focus.is_some() && property_focus == *focus)
            })
            .map(|(_, _, revision, _)| *revision);
        if let Some(s) = &mut tab.session
            && s.editable
            && self.editor.draft.as_ref().is_some_and(|a| a != &selected)
        {
            let mut entries = s.annotations.clone();
            if let Some(target) = entries.iter_mut().find(|a| a.id == selected.id) {
                *target = self.editor.draft.clone().unwrap();
            }
            if let Err(e) = s.commit_live(entries, previous) {
                self.status = Some((format!("{e:#}"), true));
            } else {
                self.status = None;
                self.editor.property_edit =
                    Some((tab.id, selected.id.clone(), s.revision(), property_focus));
            }
        }
        if !ctx.input(|i| i.pointer.any_down()) && property_focus.is_none() {
            self.editor.property_edit = None;
        }
        if duplicate && let Some(s) = &mut tab.session {
            let a = self.editor.draft.as_ref().unwrap_or(&selected).duplicate();
            let mut entries = s.annotations.clone();
            entries.push(a.clone());
            if let Err(e) = s.commit(entries) {
                self.status = Some((format!("{e:#}"), true));
            } else {
                tab.selected_annotation = Some(a.id.clone());
                self.editor.draft = Some(a);
            }
        }
        if update_library && let Some(id) = &selected.asset_id {
            let current = self.editor.draft.as_ref().unwrap_or(&selected).clone();
            let base = self
                .editor
                .library
                .as_ref()
                .and_then(|l| l.entries.iter().find(|a| &a.id == id))
                .cloned()
                .or_else(|| {
                    self.editor
                        .builtins
                        .iter()
                        .find(|b| &b.asset.id == id)
                        .map(|b| b.asset.clone())
                });
            if let Some(mut asset) = base {
                if standard_asset {
                    asset.id = format!("{:032x}", rand::random::<u128>());
                    asset.name = format!("{} custom", asset.name);
                }
                asset.kind = if let Kind::Image { png } = &current.kind {
                    Kind::Image {
                        png: mypdf_milestone0::object_tools::cropped(png, current.object.crop)
                            .unwrap_or_else(|_| png.clone()),
                    }
                } else {
                    current.kind.clone()
                };
                asset.style = current.style.clone();
                asset.aspect =
                    (current.rect[2] - current.rect[0]) / (current.rect[3] - current.rect[1]);
                let asset_id = asset.id.clone();
                match self.editor.add_asset(asset) {
                    Ok(()) => {
                        if standard_asset {
                            let mut linked = current.clone();
                            linked.asset_id = Some(asset_id);
                            selection::commit(
                                tab,
                                std::slice::from_ref(&linked),
                                None,
                                &mut self.status,
                            );
                            self.editor.draft = Some(linked);
                        }
                        self.status = Some((
                            "Library updated. Other placed copies keep their artwork.".into(),
                            false,
                        ))
                    }
                    Err(e) => self.status = Some((e, true)),
                }
            }
        }
        if save_asset {
            let asset = Asset::new(
                &self.editor.asset_name,
                {
                    let a = self.editor.draft.as_ref().unwrap_or(&selected);
                    if let Kind::Image { png } = &a.kind {
                        Kind::Image {
                            png: mypdf_milestone0::object_tools::cropped(png, a.object.crop)
                                .unwrap_or_else(|_| png.clone()),
                        }
                    } else {
                        a.kind.clone()
                    }
                },
                self.editor
                    .draft
                    .as_ref()
                    .unwrap_or(&selected)
                    .style
                    .clone(),
                (selected.rect[2] - selected.rect[0]) / (selected.rect[3] - selected.rect[1]),
            );
            match self.editor.add_asset(asset) {
                Ok(()) => {
                    self.status = Some(("Saved to the local asset library".into(), false));
                }
                Err(e) => self.status = Some((e, true)),
            }
        }
        if copy_image {
            self.copy_marks(ctx, false);
        }
        if delete {
            self.editor_delete();
        }
    }
    pub(super) fn asset_panel(&mut self, ctx: &egui::Context) {
        if egui::DragAndDrop::has_payload_of_type::<Asset>(ctx) {
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        let mut import = false;
        if self.editor.library_open {
            let mut open = true;
            let mut place = None;
            let mut remove = None;
            egui::SidePanel::right("asset_library_panel")
                .exact_width(270.0)
                .resizable(false)
                .frame(egui::Frame::new().fill(theme::paper()).inner_margin(14.0))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.heading(&self.editor.library_tab);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add(ActionButton::new(Icon::Close, "Close asset panel").compact())
                                .clicked()
                            {
                                open = false;
                            }
                        });
                    });
                    ui.label(
                        RichText::new("Choose artwork, then click or drag on a page.")
                            .small()
                            .color(theme::muted()),
                    );
                    ui.add_space(6.0);
                    if self.editor.removed_asset.is_some()
                        && ui
                            .add(ActionButton::new(Icon::Undo, "Undo removal"))
                            .clicked()
                    {
                        let removed = self.editor.removed_asset.take().unwrap();
                        if let Err(e) = self.editor.add_asset(removed.clone()) {
                            self.editor.removed_asset = Some(removed);
                            self.editor.library_error = Some(e);
                        }
                    }
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        for (label, icon) in [
                            ("Arrows", Icon::Arrow),
                            ("Symbols", Icon::Star),
                            ("Stamps", Icon::Stamp),
                            ("Saved", Icon::Library),
                        ] {
                            if ui
                                .add(
                                    ActionButton::new(icon, label)
                                        .compact()
                                        .selected(self.editor.library_tab == label),
                                )
                                .clicked()
                            {
                                self.editor.library_tab = label.into();
                            }
                        }
                        let add = ui.add(ActionButton::new(
                            Icon::Plus,
                            if self.editor.library_tab == "Stamps" { "Add stamp" } else { "Add artwork" },
                        ).compact());
                        egui::Popup::menu(&add).show(|ui| {
                        if ui.add(ActionButton::new(Icon::Text, "SVG code…")).clicked() {
                            self.editor.svg = SvgEditor::default();
                            self.editor.svg.open = true;
                            self.editor.svg.stamp = self.editor.library_tab == "Stamps";
                            ui.close();
                        }
                        if ui
                            .add(ActionButton::new(Icon::Sparkle, "Create with prompt…"))
                            .clicked()
                        {
                            self.editor.create_stamp_prompt();
                            ui.close();
                        }
                        import = ui
                            .add_enabled(
                                !self.editor.asset_busy,
                                ActionButton::new(Icon::Import, "Import image / SVG…"),
                            )
                            .clicked();
                        if import { ui.close(); }
                        if ui
                            .add(ActionButton::new(Icon::Sign, "Signature…"))
                            .clicked()
                        {
                            self.editor.open_signature();
                            ui.close();
                        }
                    });
                    });
                    let search = ui.add(
                        egui::TextEdit::singleline(&mut self.editor.library_query)
                            .margin(ui::FIELD_MARGIN)
                            .hint_text("Search this collection…")
                            .desired_width(f32::INFINITY)
                            .char_limit(200),
                    );
                    self.editor.text_editing |= search.has_focus();
                    if self.editor.asset_busy {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Importing artwork…");
                        });
                    }
                    if let Some(error) = &self.editor.library_error {
                        ui.colored_label(theme::danger(), error);
                    }
                    let query = self.editor.library_query.to_lowercase();
                    if self.editor.library_tab == "Stamps" {
                        if let Some(i)=ui::segmented(ui,&["Standard","Custom"],self.editor.stamp_source) { self.editor.stamp_source=i;self.editor.library_folder.clear(); }
                        ui.horizontal(|ui| { ui.selectable_value(&mut self.editor.stamp_source,2,"★ Favorites"); ui.selectable_value(&mut self.editor.stamp_source,3,"Recent"); });
                    }
                    let mut all: Vec<(Option<usize>,Asset)> = self.editor.builtins.iter().map(|b|(None,b.asset.clone())).collect();
                    if let Some(library)=&self.editor.library {
                        for (i,a) in library.entries.iter().enumerate() {
                            if let Some(existing)=all.iter_mut().find(|(_,b)| b.id==a.id) { *existing=(Some(i),a.clone()); }
                            else { all.push((Some(i),a.clone())); }
                        }
                    }
                    let mut folders: Vec<_> = all.iter().map(|(_,a)|a.folder.clone()).filter(|f|!f.is_empty()).collect();
                    folders.sort(); folders.dedup();
                    if matches!(self.editor.library_tab.as_str(),"Stamps"|"Saved") && !folders.is_empty() {
                        egui::ComboBox::from_id_salt("asset_folder").selected_text(if self.editor.library_folder.is_empty(){"All folders"}else{&self.editor.library_folder}).show_ui(ui,|ui|{
                            ui.selectable_value(&mut self.editor.library_folder,String::new(),"All folders");
                            for folder in folders { ui.selectable_value(&mut self.editor.library_folder,folder.clone(),folder); }
                        });
                    }
                    let mut entries: Vec<_> = all.into_iter().filter(|(index,a)| {
                        let builtin=self.editor.builtins.iter().find(|b|b.asset.id==a.id);
                        let collection=match self.editor.library_tab.as_str() {
                            "Saved" => index.is_some() && builtin.is_none(),
                            "Stamps" => a.stamp && match self.editor.stamp_source { 0=>builtin.is_some(),1=>builtin.is_none(),2=>a.favorite,_=>a.last_used>0 },
                            other => builtin.is_some_and(|b|b.category==other),
                        };
                        collection && format!("{} {} {}",a.name,a.tags,a.folder).to_lowercase().contains(&query)
                            && (self.editor.library_folder.is_empty() || a.folder==self.editor.library_folder || !matches!(self.editor.library_tab.as_str(),"Stamps"|"Saved"))
                    }).collect();
                    if self.editor.library_tab=="Stamps" && self.editor.stamp_source==3 { entries.sort_by_key(|(_,a)|std::cmp::Reverse(a.last_used)); }
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .id_salt("asset_gallery")
                        .max_height((ui.available_height() - 44.0).max(120.0))
                        .show(ui, |ui| {
                            if entries.is_empty() {
                                ui.add_space(22.0);
                                ui.label(if query.is_empty() {
                                    "Your saved signatures and imported artwork will appear here."
                                } else {
                                    "No matching assets in this collection."
                                });
                                ui.add_space(22.0);
                            }
                            let columns =
                                ((ui.available_width() + 8.0) / 110.0).floor().max(1.0) as usize;
                            let full_width=ui.available_width();
                            let mut rows: Vec<Vec<&(Option<usize>,Asset)>>=vec![];
                            for entry in &entries {
                                let wide=entry.1.stamp && entry.1.aspect>4.5;
                                if wide || rows.last().is_none_or(|row| row.len()>=columns || row[0].1.stamp && row[0].1.aspect>4.5) { rows.push(vec![]); }
                                rows.last_mut().unwrap().push(entry);
                            }
                            for row in rows {
                                let wide=row.len()==1 && row[0].1.stamp && row[0].1.aspect>4.5;
                                let width=if wide {full_width} else {((full_width-(columns-1)as f32*8.)/columns as f32).max(100.)};
                                ui.horizontal_top(|ui| {
                                    for (index, asset) in row {
                                        ui.push_id(&asset.id, |ui| {
                                            ui.vertical(|ui| {
                                                let (rect, response) = ui.allocate_exact_size(
                                                    vec2(width, 94.0),
                                                    egui::Sense::click_and_drag(),
                                                );
                                                if response.drag_started() {
                                                    egui::DragAndDrop::set_payload(
                                                        ctx,
                                                        asset.clone(),
                                                    );
                                                }
                                                response.widget_info(|| {
                                                    egui::WidgetInfo::labeled(
                                                        egui::WidgetType::Button,
                                                        ui.is_enabled(),
                                                        format!("Place {}", asset.name),
                                                    )
                                                });
                                                let hover = response.hovered();
                                                ui.painter().rect_filled(
                                                    rect,
                                                    5.0,
                                                    if hover {
                                                        theme::accent_soft()
                                                    } else {
                                                        theme::surface()
                                                    },
                                                );
                                                ui.painter().rect_stroke(
                                                    rect,
                                                    5.0,
                                                    Stroke::new(
                                                        1.0,
                                                        if hover {
                                                            theme::accent()
                                                        } else {
                                                            theme::line()
                                                        },
                                                    ),
                                                    egui::StrokeKind::Inside,
                                                );
                                                // Artwork sits on white paper in both themes.
                                                ui.painter().rect_filled(
                                                    Rect::from_min_max(
                                                        rect.min + vec2(6.0, 5.0),
                                                        pos2(rect.right() - 6.0, rect.top() + 69.0),
                                                    ),
                                                    4.0,
                                                    Color32::WHITE,
                                                );
                                                let base = vec2(240.0, 240.0 / asset.aspect);
                                                let scale =
                                                    ((width - 18.0) / base.x).min(50.0 / base.y);
                                                let origin = pos2(
                                                    rect.center().x - base.x * scale / 2.0,
                                                    rect.top()
                                                        + 10.0
                                                        + (54.0 - base.y * scale) / 2.0,
                                                );
                                                let mut preview = Annotation::new(
                                                    0,
                                                    [0.0, 0.0, base.x, base.y],
                                                    asset.kind.clone(),
                                                    asset.style.clone(),
                                                );
                                                preview.style.width =
                                                    (1.8 / scale).max(preview.style.width);
                                                let painter =
                                                    ui.painter().with_clip_rect(rect.shrink(5.0));
                                                if ui.is_rect_visible(rect) {
                                                    paint_annotation(
                                                        &mut self.editor,
                                                        ctx,
                                                        &painter,
                                                        Rect::from_min_size(origin, base * scale),
                                                        scale,
                                                        &preview,
                                                    );
                                                }
                                                let label = if asset.name.chars().count() > 18 {
                                                    format!(
                                                        "{}…",
                                                        asset
                                                            .name
                                                            .chars()
                                                            .take(17)
                                                            .collect::<String>()
                                                    )
                                                } else {
                                                    asset.name.clone()
                                                };
                                                painter.text(
                                                    pos2(rect.center().x, rect.bottom() - 14.0),
                                                    egui::Align2::CENTER_CENTER,
                                                    label,
                                                    FontId::proportional(12.0),
                                                    theme::ink(),
                                                );
                                                response.context_menu(|ui|{
                                                    if ui.add(ActionButton::new(Icon::Copy,"Create custom variant")).clicked() {
                                                        let mut copy=asset.clone();copy.id=format!("{:032x}",rand::random::<u128>());copy.name=format!("{} copy",asset.name.chars().take(190).collect::<String>());copy.last_used=0;
                                                        if let Err(e)=self.editor.add_asset(copy.clone()){self.editor.library_error=Some(e);}else{self.editor.asset_details=Some(copy);}
                                                        ui.close();
                                                    }
                                                });
                                                if response
                                                    .on_hover_text(format!(
                                                        "Click, then click the page to place {} · or drag it onto the page",
                                                        asset.name
                                                    ))
                                                    .on_hover_cursor(egui::CursorIcon::Grab)
                                                    .clicked()
                                                {
                                                    place = Some(asset.clone());
                                                }
                                                let star_rect=Rect::from_center_size(rect.right_top()+vec2(-12.,12.),vec2(20.,20.));
                                                let star=ui.interact(star_rect,ui.id().with("favorite"),egui::Sense::click());
                                                star.widget_info(||egui::WidgetInfo::selected(egui::WidgetType::Button,true,asset.favorite,"Favorite stamp"));
                                                if hover || star.hovered() || asset.favorite { ui::draw_icon(ui.painter(),star_rect.shrink(3.),Icon::Star,if asset.favorite {theme::accent()}else{theme::muted()}); }
                                                if star.on_hover_text(if asset.favorite {"Remove from favorites"}else{"Add to favorites"}).clicked() {
                                                    let mut next=asset.clone(); next.favorite = !next.favorite;
                                                    if let Some(l)=&mut self.editor.library && let Err(e)=l.put(next) { self.editor.library_error=Some(format!("{e:#}")); }
                                                }
                                                if let Some(index) = index && !self.editor.builtins.iter().any(|b|b.asset.id==asset.id) {
                                                    ui.horizontal(|ui| {
                                                        if ui
                                                            .add(
                                                                ActionButton::new(
                                                                    Icon::Pen,
                                                                    "Manage asset",
                                                                )
                                                                .compact(),
                                                            )
                                                            .clicked()
                                                        {
                                                            self.editor.asset_details =
                                                                Some(asset.clone());
                                                        }
                                                        if ui
                                                            .add(
                                                                ActionButton::new(
                                                                    Icon::Plus,
                                                                    "Duplicate asset",
                                                                )
                                                                .compact(),
                                                            )
                                                            .clicked()
                                                        {
                                                            let mut copy = asset.clone();
                                                            copy.id = format!(
                                                                "{:032x}",
                                                                rand::random::<u128>()
                                                            );
                                                            copy.name = format!(
                                                                "{} copy",
                                                                asset
                                                                    .name
                                                                    .chars()
                                                                    .take(190)
                                                                    .collect::<String>()
                                                            );
                                                            if let Err(e) =
                                                                self.editor.add_asset(copy)
                                                            {
                                                                self.editor.library_error = Some(e);
                                                            }
                                                        }
                                                        if ui
                                                            .add(
                                                                ActionButton::new(
                                                                    Icon::Trash,
                                                                    "Remove asset",
                                                                )
                                                                .compact(),
                                                            )
                                                            .clicked()
                                                        {
                                                            remove = Some(*index);
                                                        }
                                                    });
                                                }
                                            });
                                        });
                                    }
                                });
                            }
                        });
                    ui.separator();
                    ui.label(
                        RichText::new(if self.editor.library_tab == "Saved"
                            || (self.editor.library_tab == "Stamps" && self.editor.stamp_source == 1)
                        {
                            "Stored on this computer • click a card to place"
                        } else {
                            "Built-in vector artwork • resize and recolor after placing"
                        })
                        .small()
                        .color(theme::muted()),
                    );
                });
            self.editor.library_open = open;
            if let Some(asset) = place {
                self.begin_asset_placement(asset);
            }
            if let Some(i) = remove
                && let Some(library) = &mut self.editor.library
            {
                let removed = library.entries.remove(i);
                if let Err(e) = library.save() {
                    library.entries.insert(i, removed);
                    self.editor.library_error = Some(format!("{e:#}"));
                } else {
                    self.editor.removed_asset = Some(removed);
                }
            }
        }
        if import {
            self.import_library_asset(ctx, None, self.editor.library_tab == "Stamps");
        }
    }
    pub(super) fn import_library_asset(
        &mut self,
        ctx: &egui::Context,
        target: Option<Asset>,
        stamp: bool,
    ) {
        if let Some(path) = self.native_pick(
            rfd::FileDialog::new()
                .set_title("Import reusable artwork")
                .add_filter("Artwork", &["png", "jpg", "jpeg", "svg"]),
        ) {
            let tx = self.editor.import_tx.clone();
            let wake = ctx.clone();
            self.editor.asset_busy = true;
            std::thread::spawn(move || {
                let result = (|| -> anyhow::Result<_> {
                    if path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
                    {
                        let source = String::from_utf8(mypdf_milestone0::convert::read_bounded(
                            &path,
                            256 * 1024,
                        )?)?;
                        let tree = mypdf_milestone0::svg::tree(&source, None)?;
                        return Ok(Asset::new(
                            &path.file_stem().unwrap_or_default().to_string_lossy(),
                            Kind::Svg {
                                source,
                                tint: false,
                            },
                            Style::default(),
                            tree.size().width() / tree.size().height(),
                        ));
                    }
                    Asset::from_image(&path)
                })()
                .map(|mut asset| {
                    asset.stamp = stamp;
                    if let Some(target) = target {
                        asset.id = target.id;
                        asset.name = target.name;
                        asset.stamp = target.stamp;
                    }
                    asset
                })
                .map_err(|e| format!("{e:#}"));
                let _ = tx.send(result);
                wake.request_repaint();
            });
        }
    }
    #[cfg(test)]
    pub(super) fn preview_signature_image(&mut self) {
        let source = "<svg xmlns='http://www.w3.org/2000/svg' width='320' height='100'><path d='M20 70 Q60 0 50 70 Q90 20 70 70 Q120 35 100 65 Q130 30 120 70 L180 50 Q200 30 195 65 Q230 20 215 60 L280 50 M35 85 L270 78' fill='none' stroke='#20334c' stroke-width='3' stroke-linecap='round'/></svg>";
        let png = mypdf_milestone0::svg::preview(source, None).unwrap();
        self.editor.signature_input = SignatureInput::Image;
        self.editor.signature_image = Some(Asset::new(
            "My signature",
            Kind::Image { png },
            Style::default(),
            3.2,
        ));
        self.editor.signature_name = "My signature".into();
        self.editor.open_signature();
    }
    pub(super) fn paste_signature_image(&mut self) {
        self.import_signature_clipboard(clipboard::read());
    }

    pub(super) fn import_signature_clipboard(&mut self, clip: clipboard::Clip) {
        if self.editor.signature_busy || self.editor.background.is_some() {
            return;
        }
        match clip {
            clipboard::Clip::Image(bytes, _) => {
                match Asset::from_image_bytes("My signature", &bytes) {
                    Ok(asset) => {
                        self.editor.signature_image = Some(asset);
                        self.editor.signature_input = SignatureInput::Image;
                        self.editor.signature_error = None;
                    }
                    Err(e) => self.editor.signature_error = Some(format!("{e:#}")),
                }
            }
            _ => {
                self.editor.signature_error = Some(
                    "Copy an image first, then choose Paste image. You can also use Choose image…"
                        .into(),
                )
            }
        }
    }

    pub(super) fn signature_image_paste_active(&self, ctx: &egui::Context) -> bool {
        self.editor.signature_open
            && self.editor.signature_input == SignatureInput::Image
            && self.editor.background.is_none()
            && !ctx.wants_keyboard_input()
    }

    fn begin_asset_placement(&mut self, asset: Asset) {
        let Some(tab) = self
            .tabs
            .get_mut(self.active)
            .filter(|t| t.session.as_ref().is_some_and(|s| s.editable))
        else {
            self.status = Some((
                "Saved in your library. Open an editable PDF to place it.".into(),
                false,
            ));
            return;
        };
        let page = tab.pages[tab.page];
        let width = 180_f32
            .min(page.width * 0.6)
            .min(page.height * 0.6 * asset.aspect);
        let mut mark = Annotation::new(
            tab.page,
            [0., 0., width, width / asset.aspect],
            asset.kind.clone(),
            asset.style.clone(),
        );
        mark.asset_id = Some(asset.id.clone());
        selection::set(tab, &mut self.editor, vec![]);
        self.editor.placed = Some(asset);
        if self.editor.mode != Mode::Sign {
            self.editor.mode = Mode::Draw;
        }
        self.editor.tool = Tool::Select;
        self.editor.draft = None;
        self.editor.library_open = false;
        self.workbench.paste = Some(vec![mark]);
        self.status = Some(("Move the preview to your chosen spot and click. Drag to adjust; use a corner to resize. Escape cancels.".into(), false));
    }

    #[cfg(test)]
    pub(super) fn signature_here(&mut self, key: (u64, u64, usize), point: [f32; 2]) {
        self.start_ink_signature();
        self.editor.signature_placement = Some(SignaturePlacement { key, point });
    }

    pub(super) fn start_ink_signature(&mut self) {
        self.digital.open = false;
        self.digital.adopt_asset = false;
        self.editor.library_open = false;
        self.editor.open_signature();
        self.editor.signature_error = None;
        self.editor.signature_picker = true;
    }

    pub(super) fn place_default_ink_signature(&mut self) {
        let asset = self.editor.library.as_ref().and_then(|library| {
            library
                .entries
                .iter()
                .find(|asset| self.digital.ink_is_default(&asset.id))
                .cloned()
        });
        if let Some(asset) = asset {
            self.editor.signature_open = false;
            self.place_saved_signature(asset);
        } else {
            self.editor.signature_error = Some(
                "Choose a default ink signature in Settings. You can select a signature below for now."
                    .into(),
            );
        }
    }

    pub(super) fn place_saved_signature(&mut self, asset: Asset) {
        let Some(target) = self.editor.signature_placement.take() else {
            self.begin_asset_placement(asset);
            return;
        };
        let Some(index) = self.tabs.iter().position(|t| {
            t.id == target.key.0
                && target.key.2 < t.pages.len()
                && t.session
                    .as_ref()
                    .is_some_and(|s| s.editable && s.revision() == target.key.1)
        }) else {
            self.status = Some(("The document changed or closed. Your signature is saved in the library; choose its placement again.".into(), false));
            return;
        };
        self.active = index;
        self.tabs[index].jump(target.key.2);
        self.begin_asset_placement(asset);
        if let Some(marks) = self.workbench.paste.take() {
            let tab = &mut self.tabs[index];
            let marks =
                workbench::positioned(&marks, target.key.2, target.point, tab.pages[target.key.2]);
            if workbench::place(&mut self.editor, tab, marks, &mut self.status) {
                self.editor.mode = Mode::Annotate;
                self.status = Some((
                    "Signature placed · Drag to move, use a corner to resize · Undo to remove"
                        .into(),
                    false,
                ));
            }
        }
    }

    pub(super) fn editor_windows(&mut self, ctx: &egui::Context) {
        self.asset_details_window(ctx);
        self.svg_window(ctx);
        if self.editor.signature_open {
            let mut open = true;
            let mut save = false;
            let mut choose = false;
            let mut paste = false;
            let mut cancel = false;
            let assets = self
                .editor
                .library
                .as_ref()
                .map(|l| l.entries.clone())
                .unwrap_or_default();
            let saved = self.digital.ink_assets(&assets);
            let mut manage = false;
            let mut picked = None;
            let can_place = !self.digital.adopt_asset
                && !self.editor.signature_manage_only
                && self
                    .tabs
                    .get(self.active)
                    .is_some_and(|t| t.session.as_ref().is_some_and(|s| s.editable));
            egui::Window::new("Handwritten signature")
                .collapsible(false).open(&mut open)
                .resizable([true, false])
                .default_width(ui::dialog_width(ctx, 480.))
                .max_width(ui::dialog_width(ctx, 640.))
                .show(ctx, |ui| {
                    ui::dialog_body(ui, "signature_editor_body", 460., |ui| {
                    if self.editor.signature_picker {
                        ui.label("Choose a saved signature, then place it on the page.");
                        if let Some(error) = &self.editor.signature_error { ui.colored_label(theme::danger(), error); }
                        ui.horizontal_wrapped(|ui| {
                            for asset in &saved {
                                ui.vertical(|ui| {
                                    if digital::artwork_card(ui, &mut self.editor, asset, false) { picked = Some(asset.clone()); }
                                    if self.digital.ink_is_default(&asset.id) { ui.weak("★ Default"); }
                                });
                            }
                        });
                        if saved.is_empty() { ui.weak("No ink signatures saved yet. Create one in Settings."); }
                        manage = ui.add(ActionButton::new(Icon::Settings, "Manage signatures in Settings…")).clicked();
                        return;
                    }
                    ui.label(if self.digital.adopt_asset {
                        "Draw, type, or import the image for your digital signature."
                    } else if self.editor.signature_manage_only {
                        "Save your signature here, then choose it when signing a PDF."
                    } else if self.editor.signature_placement.is_some() {
                        "Your signature will be placed where you right-clicked."
                    } else {
                        "Create your signature, then click on the page to place it."
                    });
                    ui.horizontal(|ui| {
                        for (mode, label) in [(SignatureInput::Draw, "Draw by hand"), (SignatureInput::Name, "From name"), (SignatureInput::Type, "Type"), (SignatureInput::Image, "Image")] {
                            ui.selectable_value(&mut self.editor.signature_input, mode, label);
                        }
                    });
                    ui.add_space(8.);
                    if self.editor.signature_input != SignatureInput::Image {
                        ui::color_swatch(ui, &mut self.editor.signature_ink_color, "Ink color");
                    }
                    match self.editor.signature_input {
                        SignatureInput::Name => {
                            let r = ui.add(egui::TextEdit::singleline(&mut self.editor.signature_typed).margin(ui::FIELD_MARGIN).hint_text("Your full name").char_limit(200));
                            self.editor.text_editing |= r.has_focus();
                            ui.horizontal_wrapped(|ui| {
                                for (i, label) in mypdf_milestone0::signature_art::NAME_STYLES.iter().enumerate() {
                                    ui.selectable_value(&mut self.editor.signature_name_style, i, *label);
                                }
                            });
                            ui.checkbox(&mut self.editor.signature_flourish, "Add a flourish");
                            let key = format!("{}:{}:{}:{:?}", self.editor.signature_typed, self.editor.signature_name_style, self.editor.signature_flourish, self.editor.signature_ink_color);
                            if key != self.editor.signature_generated_key {
                                self.editor.signature_generated_key = key;
                                self.editor.signature_generated = Some(mypdf_milestone0::signature_art::from_name(&self.editor.signature_typed, self.editor.signature_name_style, self.editor.signature_flourish, self.editor.signature_ink_color).map_err(|e| format!("{e:#}")));
                            }
                            if let Some(result) = self.editor.signature_generated.clone() {
                                match result {
                                    Ok(asset) => {
                                        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 150.), egui::Sense::hover());
                                        ui.painter().rect_filled(rect, 10., Color32::WHITE);
                                        paint_asset(&mut self.editor, ctx, ui.painter(), rect.shrink(16.), &asset);
                                    }
                                    Err(error) => { ui.weak(error); }
                                }
                            }
                            ui.weak("Choose a style for your name, then save it for reuse. Created locally on this device.");
                        }
                        SignatureInput::Type => {
                            let r = ui.add(egui::TextEdit::singleline(&mut self.editor.signature_typed)
                                .margin(ui::FIELD_MARGIN).hint_text("Signature text").char_limit(200));
                            self.editor.text_editing |= r.has_focus();
                        }
                        SignatureInput::Draw => {
                            ui.add(egui::Slider::new(&mut self.editor.signature_ink_width, 0.5..=8.).text("Pen width"));
                            draw_signature_pad(ui, &mut self.editor);
                        },
                        SignatureInput::Image => {
                            if let Some(asset) = self.editor.signature_image.clone() {
                                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 150.), egui::Sense::hover());
                                ui.painter().rect_filled(rect, 10., Color32::WHITE);
                                paint_asset(&mut self.editor, ctx, ui.painter(), rect.shrink(16.), &asset);
                                if let Kind::Image { png } = &asset.kind
                                    && ui.add(ActionButton::new(Icon::FieldImage, "Remove background…")).clicked()
                                {
                                    self.editor.background = Some(background::Draft::new(background::Target::Signature, png.clone()));
                                }
                            }
                            ui.horizontal(|ui| {
                                choose = ui.add_enabled(!self.editor.signature_busy, ActionButton::new(Icon::Import, "Choose image…")).clicked();
                                paste = ui.add_enabled(!self.editor.signature_busy, ActionButton::new(Icon::Paste, "Paste image")).on_hover_text("Copy an image from another app, then paste it here").clicked();
                            });
                            ui.weak(if cfg!(target_os = "macos") { "Copy an image, then click Paste image or press ⌘V." } else { "Copy an image, then click Paste image or press Ctrl+V." });
                            ui.weak("PNG or JPEG · Transparent PNG keeps the page visible behind your signature.");
                            if self.editor.signature_busy { ui.spinner(); }
                            if let Some(error) = &self.editor.signature_error { ui.colored_label(theme::danger(), error); }
                        }
                    }
                    ui.add_space(10.);
                    let r = ui.add(egui::TextEdit::singleline(&mut self.editor.signature_name)
                        .margin(ui::FIELD_MARGIN).hint_text("Library name").char_limit(200));
                    self.editor.text_editing |= r.has_focus();
                    });
                    ui::dialog_footer(ui, |ui| {
                        if !self.editor.signature_picker {
                        save = ui.add_enabled(!self.editor.signature_busy && (self.editor.signature_input != SignatureInput::Image || self.editor.signature_image.is_some()),
                            ActionButton::new(Icon::Save, if can_place { "Save & place" } else { "Save signature" }).primary()).clicked();
                        }
                        if ui.add(ActionButton::new(Icon::Close, "Cancel")).clicked() { cancel = true; }
                    });
                });
            if manage {
                self.editor.signature_open = false;
                self.digital.open = true;
                self.digital.panel = digital::Panel::Ink;
                return;
            }
            if self.editor.background.is_none()
                && !egui::Popup::is_any_open(ctx)
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
            {
                cancel = true;
            }
            self.editor.signature_open = open && !cancel;
            if let Some(asset) = picked.filter(|_| self.editor.signature_open) {
                self.editor.signature_open = false;
                self.place_saved_signature(asset);
            }
            if paste && self.editor.signature_open {
                self.paste_signature_image();
            }
            if choose
                && let Some(path) = self.native_pick(
                    rfd::FileDialog::new()
                        .set_title("Choose signature image")
                        .add_filter("Signature image", &["png", "jpg", "jpeg"]),
                )
            {
                self.editor.signature_busy = true;
                self.editor.signature_error = None;
                let tx = self.editor.signature_tx.clone();
                let wake = ctx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(Asset::from_image(&path).map_err(|e| format!("{e:#}")));
                    wake.request_repaint();
                });
            }
            if save {
                let asset = match self.editor.signature_input {
                    SignatureInput::Name => self
                        .editor
                        .signature_generated
                        .as_ref()
                        .and_then(|a| a.as_ref().ok())
                        .cloned()
                        .map(|mut asset| {
                            asset.id = format!("{:032x}", rand::random::<u128>());
                            asset.name = self.editor.signature_name.clone();
                            asset
                        }),
                    SignatureInput::Image => {
                        self.editor.signature_image.clone().map(|mut asset| {
                            asset.id = format!("{:032x}", rand::random::<u128>());
                            asset.name = self.editor.signature_name.clone();
                            asset
                        })
                    }
                    mode => {
                        let kind = if mode == SignatureInput::Type {
                            Kind::Text {
                                text: self.editor.signature_typed.clone(),
                            }
                        } else {
                            Kind::Ink {
                                strokes: self
                                    .editor
                                    .signature_strokes
                                    .iter()
                                    .filter(|s| s.len() > 1)
                                    .cloned()
                                    .collect(),
                            }
                        };
                        let empty = match &kind {
                            Kind::Text { text } => text.trim().is_empty(),
                            Kind::Ink { strokes } => strokes.is_empty(),
                            _ => false,
                        };
                        (!empty).then(|| {
                            Asset::new(
                                &self.editor.signature_name,
                                kind,
                                Style {
                                    font_size: 24.,
                                    color: self.editor.signature_ink_color,
                                    width: self.editor.signature_ink_width,
                                    ..Default::default()
                                },
                                3.,
                            )
                        })
                    }
                };
                if let Some(mut asset) = asset {
                    asset.signature = true;
                    match self.editor.add_asset(asset.clone()) {
                        Ok(()) if self.digital.adopt_asset => {
                            // digital_events adopts it as the signature appearance.
                            self.editor.signature_open = false;
                        }
                        Ok(()) => {
                            self.editor.signature_open = false;
                            if self.editor.signature_manage_only {
                                self.digital.open = true;
                                self.digital.panel = digital::Panel::Ink;
                            } else {
                                self.place_saved_signature(asset);
                            }
                        }
                        Err(e) => self.status = Some((e, true)),
                    }
                } else {
                    self.status =
                        Some(("Draw, type, or choose a signature image first".into(), true));
                }
            }
        }
        if !self.editor.signature_open {
            self.editor.signature_placement = None;
        }
    }
    pub(super) fn unfinished_edit(&mut self, target: Option<u64>) -> bool {
        if let Some(id) = self
            .content
            .pending_text()
            .filter(|id| target.is_none_or(|t| t == *id))
            && let Some(index) = self.tabs.iter().position(|t| t.id == id)
        {
            self.active = index;
            self.editor.mode = Mode::Edit;
            self.inspector = true;
            self.status = Some((
                "Finish the text with Done or ⌘Return, or cancel it, before saving or closing."
                    .into(),
                false,
            ));
            return true;
        }
        if let Some(v) = &self.editor.vertices
            && !v.points.is_empty()
            && target.is_none_or(|t| t == v.tab)
            && let Some(index) = self.tabs.iter().position(|t| t.id == v.tab)
        {
            self.active = index;
            self.tabs[index].jump(v.page);
            self.editor.mode = Mode::Draw;
            self.status = Some((
                "Finish the shape with Return, or cancel with Esc, before saving or closing."
                    .into(),
                false,
            ));
            return true;
        }
        false
    }
    pub(super) fn request_close(&mut self, id: Option<u64>, ctx: &egui::Context) {
        self.remember_reading(ctx, true);
        if let Some(pending) = self
            .tabs
            .iter()
            .find(|tab| id.is_none_or(|id| tab.id == id) && self.pending_form_edits(tab.id))
            .map(|tab| tab.id)
        {
            self.focus_pending_form_edits(pending);
            self.status = Some((
                "Apply or discard the pending form changes before closing.".into(),
                false,
            ));
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            return;
        }
        if self
            .content
            .pending_text()
            .is_some_and(|pending| id.is_none_or(|t| t == pending))
            && !self.content.busy
        {
            self.content.after_text = Some(content::AfterText::Close(id));
            self.content.new_commit = true;
            self.finish_new_text(ctx);
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            return;
        }
        if self.unfinished_edit(id) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            return;
        }
        if self.content.busy || self.organizer.busy {
            self.status = Some((
                "Wait for the current operation before closing.".into(),
                false,
            ));
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            return;
        }
        if id.is_none_or(|id| self.tabs.get(self.active).is_some_and(|tab| tab.id == id))
            && !self.apply_pending_annotation()
        {
            return;
        }
        if self.saving || self.digital.busy || self.editor.asset_busy {
            self.status = Some((
                "Wait for the current operation to finish before closing.".into(),
                true,
            ));
            return;
        }
        let dirty = self.tabs.iter().any(|t| {
            id.is_none_or(|id| t.id == id)
                && (t.unsaved_on_load || t.session.as_ref().is_some_and(|s| s.dirty()))
        });
        if dirty {
            self.close_intent = Some(id);
        } else if let Some(id) = id {
            if let Some(index) = self.tabs.iter().position(|t| t.id == id) {
                self.close(index);
            }
        } else {
            self.exit_approved = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
    pub(super) fn close_dialog(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.exit_approved {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.request_close(None, ctx);
        }
        if let Some(intent) = self.close_intent {
            let names: Vec<_> = self
                .tabs
                .iter()
                .filter(|t| {
                    intent.is_none_or(|id| t.id == id)
                        && (t.unsaved_on_load || t.session.as_ref().is_some_and(|s| s.dirty()))
                })
                .map(|t| t.name())
                .collect();
            let title = if names.len() == 1 {
                format!(
                    "Save changes to “{}”?",
                    names[0].chars().take(80).collect::<String>()
                )
            } else {
                format!("Save changes to {} documents?", names.len())
            };
            let first_save = self
                .tabs
                .iter()
                .find(|t| {
                    intent.is_none_or(|id| t.id == id)
                        && (t.unsaved_on_load || t.session.as_ref().is_some_and(|s| s.dirty()))
                })
                .is_some_and(|t| !t.has_destination);
            let decision = ui::confirmation(
                ctx,
                "unsaved_annotations",
                &title,
                if first_save {
                    "Your changes will be lost if you don’t save them. Choose a name and location for your PDF."
                } else {
                    "Your changes will be lost if you don’t save them. Saving creates a new PDF copy."
                },
                &[
                    ("Save PDF…", ui::DialogTone::Primary),
                    ("Don’t Save", ui::DialogTone::Secondary),
                    ("Cancel", ui::DialogTone::Secondary),
                ],
                2,
            );
            match decision {
                Some(0) => {
                    if let Some(index) = self.tabs.iter().position(|t| {
                        intent.is_none_or(|id| t.id == id)
                            && (t.unsaved_on_load || t.session.as_ref().is_some_and(|s| s.dirty()))
                    }) {
                        self.active = index;
                        self.close_intent = None;
                        self.close_after_save = Some(intent);
                        self.save_intent = save::Intent::Automatic;
                        self.save_dialog(ctx);
                    }
                }
                Some(1) => {
                    self.close_intent = None;
                    if let Some(id) = intent {
                        if let Some(index) = self.tabs.iter().position(|t| t.id == id) {
                            self.close(index);
                        }
                    } else {
                        self.exit_approved = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                Some(2) => {
                    self.close_intent = None;
                    self.cancel_tab_closes();
                }
                _ => {}
            }
        }
    }
}

impl Viewer {
    fn asset_details_window(&mut self, ctx: &egui::Context) {
        let Some(mut asset) = self.editor.asset_details.take() else {
            return;
        };
        let mut open = true;
        let mut save = false;
        let mut replace = false;
        let mut edit = false;
        let mut cancel = false;
        egui::Window::new("Manage asset").open(&mut open).collapsible(false)
            .resizable([true, false])
            .default_width(ui::dialog_width(ctx, 380.))
            .max_width(ui::dialog_width(ctx, 640.)).show(ctx, |ui| {
            ui::dialog_body(ui, "manage_asset_body", 400., |ui| {
            ui.label("Name");
            let r = ui.add(egui::TextEdit::singleline(&mut asset.name).margin(ui::FIELD_MARGIN).char_limit(200).desired_width(f32::INFINITY));
            self.editor.text_editing |= r.has_focus();
            ui.checkbox(&mut asset.stamp,"Include in Stamps collection");
            ui.checkbox(&mut asset.favorite,"Favorite");
            ui.label("Folder"); self.editor.text_editing |= ui.add(egui::TextEdit::singleline(&mut asset.folder).margin(ui::FIELD_MARGIN).char_limit(100).hint_text("e.g. Review, Finance")).has_focus();
            ui.label("Tags"); self.editor.text_editing |= ui.add(egui::TextEdit::singleline(&mut asset.tags).margin(ui::FIELD_MARGIN).char_limit(400).hint_text("Comma-separated keywords")).has_focus();
            ui.add_space(8.);
            ui.horizontal(|ui| {
                if matches!(asset.kind, Kind::Svg { .. }) { edit=ui.add(ActionButton::new(Icon::Text,"Edit SVG…")).clicked(); }
                replace=ui.add_enabled(!self.editor.asset_busy,ActionButton::new(Icon::Import,"Replace artwork…")).clicked();
            });
            ui.label(RichText::new("Library updates affect future placements. Existing marks in PDFs keep their own artwork.").small().color(theme::muted()));
            if let Some(e)=&self.editor.library_error { ui.colored_label(theme::danger(),e); }
            });
            ui::dialog_footer(ui, |ui| {
                save=ui.add(ActionButton::new(Icon::Save,"Save changes").primary()).clicked();
                cancel=ui.add(ActionButton::new(Icon::Close,"Cancel")).clicked();
            });
        });
        if !egui::Popup::is_any_open(ctx)
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            cancel = true;
        }
        open &= !cancel;
        if save {
            match self.editor.add_asset(asset.clone()) {
                Ok(()) => {
                    open = false;
                    self.editor.library_error = None;
                }
                Err(e) => self.editor.library_error = Some(e),
            }
        }
        if edit && let Kind::Svg { source, .. } = &asset.kind {
            self.editor.svg = SvgEditor {
                open: true,
                source: source.clone(),
                name: asset.name.clone(),
                library_target: Some(asset.id.clone()),
                stamp: asset.stamp,
                ..Default::default()
            };
            open = false;
        }
        if replace {
            self.import_library_asset(ctx, Some(asset.clone()), asset.stamp);
            open = false;
        }
        if open {
            self.editor.asset_details = Some(asset);
        }
    }
    fn svg_window(&mut self, ctx: &egui::Context) {
        if !self.editor.svg.open {
            return;
        }
        if let Some(job) = self.editor.svg.job.take() {
            match job.try_recv() {
                Ok((source, result)) if source == self.editor.svg.source => match result {
                    Ok(png) => {
                        self.editor.svg.preview = Some(png);
                        self.editor.svg.error = None;
                    }
                    Err(e) => {
                        self.editor.svg.preview = None;
                        self.editor.svg.error = Some(e);
                    }
                },
                Err(mpsc::TryRecvError::Empty) => self.editor.svg.job = Some(job),
                _ => {}
            }
        }
        if self.editor.svg.source.trim().is_empty() {
            self.editor.svg.preview = None;
            self.editor.svg.error = None;
            self.editor.svg.changed = None;
        }
        if self.editor.svg.job.is_none()
            && self
                .editor
                .svg
                .changed
                .is_some_and(|t| t.elapsed() >= std::time::Duration::from_millis(300))
        {
            self.editor.svg.changed = None;
            let (tx, rx) = mpsc::channel();
            self.editor.svg.job = Some(rx);
            let source = self.editor.svg.source.clone();
            let wake = ctx.clone();
            std::thread::spawn(move || {
                let result =
                    std::panic::catch_unwind(|| mypdf_milestone0::svg::preview(&source, None))
                        .map_err(|_| "Could not preview this SVG".to_owned())
                        .and_then(|r| r.map_err(|e| format!("{e:#}")));
                let _ = tx.send((source, result));
                wake.request_repaint();
            });
        }
        if self.editor.svg.changed.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(300));
        }
        let mut open = true;
        let mut save = false;
        let mut cancel = false;
        egui::Window::new("SVG Studio")
            .id(egui::Id::new("svg_studio"))
            .open(&mut open)
            .collapsible(false)
            .resizable([true, false])
            .default_width(ui::dialog_width(ctx, 760.))
            .max_width(ui::dialog_width(ctx, 960.))
            .show(ctx, |ui| {
                ui::dialog_body(ui, "svg_studio_body", 520., |ui| {
                ui.label(
                    RichText::new(
                        "Paste SVG code. Preview, refine, then place as scalable artwork.",
                    )
                    .color(theme::muted()),
                );
                ui.add_space(10.);
                if self.editor.svg.prompt_open {
                    egui::Frame::new().fill(theme::accent_soft()).corner_radius(8.).inner_margin(12.).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("1  Describe your stamp").family(ui::semibold()));
                            if ui.add(ActionButton::new(Icon::Close,"Hide prompt").compact()).clicked() { self.editor.svg.prompt_open=false; }
                        });
                        let r = ui.add(egui::TextEdit::multiline(&mut self.editor.svg.prompt).margin(ui::FIELD_MARGIN).hint_text("e.g. A round APPROVED stamp with a check mark, dark green, clean double border").desired_rows(3).desired_width(f32::INFINITY).char_limit(8000));
                        self.editor.text_editing |= r.has_focus();
                        if ui.add_enabled(!self.editor.svg.prompt.trim().is_empty(), ActionButton::new(Icon::Copy,"Copy prompt").primary()).clicked() {
                            ctx.copy_text(mypdf_milestone0::svg::stamp_prompt(&self.editor.svg.prompt));
                        }
                        ui.label(RichText::new("2  Run it in your preferred LLM, then paste the answer into SVG source below.").small().color(theme::muted()));
                    });
                    ui.add_space(8.);
                } else if self.editor.svg.target.is_none() && ui.add(ActionButton::new(Icon::Star,"Build an LLM prompt…")).clicked() { self.editor.svg.prompt_open=true; }
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.editor.svg.name).margin(ui::FIELD_MARGIN)
                        .hint_text("Asset name")
                        .char_limit(200),
                );
                self.editor.text_editing |= r.has_focus();
                if self.editor.svg.target.is_none() { ui.checkbox(&mut self.editor.svg.stamp,"Include in Stamps collection"); }
                ui.add_space(8.);
                ui.columns(2, |cols| {
                    cols[0].label(RichText::new("SVG source").family(ui::semibold()));
                    egui::ScrollArea::both()
                        .id_salt("svg_source_scroll")
                        .max_height(if self.editor.svg.prompt_open { 200. } else { 340. })
                        .show(&mut cols[0], |ui| {
                            let r = ui.add(
                                egui::TextEdit::multiline(&mut self.editor.svg.source).margin(ui::FIELD_MARGIN)
                                    .code_editor()
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(18)
                                    .char_limit(256 * 1024),
                            );
                            self.editor.text_editing |= r.has_focus();
                            if r.changed() {
                                self.editor.svg.preview = None;
                                self.editor.svg.changed = Some(std::time::Instant::now());
                            }
                        });
                    if cols[0].add(ActionButton::new(Icon::Import,"Extract SVG")).on_hover_text("Use SVG code from the pasted LLM answer").clicked() {
                        match mypdf_milestone0::svg::from_response(&self.editor.svg.source) {
                            Ok(source) => { self.editor.svg.source=source; self.editor.svg.preview=None; self.editor.svg.changed=Some(std::time::Instant::now()); }
                            Err(e) => self.editor.svg.error=Some(format!("{e:#}")),
                        }
                    }
                    cols[1].label(RichText::new("Preview").family(ui::semibold()));
                    let (r, _) = cols[1].allocate_exact_size(
                        vec2(cols[1].available_width(), if self.editor.svg.prompt_open { 200. } else { 300. }),
                        egui::Sense::hover(),
                    );
                    cols[1].painter().rect_filled(r, 8., Color32::WHITE);
                    if let Some(png) = self.editor.svg.preview.clone() {
                        let image = assets::decode_png(&png).ok();
                        if let Some(image) = image {
                            let size = vec2(image.width() as f32, image.height() as f32);
                            let scale =
                                ((r.width() - 28.) / size.x).min((r.height() - 28.) / size.y);
                            let rect = Rect::from_center_size(r.center(), size * scale);
                            let a = Annotation::new(
                                0,
                                [0., 0., rect.width(), rect.height()],
                                Kind::Image { png },
                                Style::default(),
                            );
                            paint_annotation(
                                &mut self.editor,
                                ctx,
                                cols[1].painter(),
                                rect,
                                1.,
                                &a,
                            );
                        }
                    }
                });
                if let Some(e) = &self.editor.svg.error {
                    ui.colored_label(theme::danger(), e);
                }
                ui.label(RichText::new("Vectors and text · Inline styles · No external resources").small().color(theme::muted()));
                });
                ui::dialog_footer(ui, |ui| {
                    save = ui
                        .add_enabled(
                            self.editor.svg.preview.is_some() && self.editor.svg.changed.is_none() && self.editor.svg.job.is_none(),
                            ActionButton::new(
                                Icon::Library,
                                if self.editor.svg.target.is_some() {
                                    "Change this placed copy"
                                } else if self.editor.svg.library_target.is_some() {
                                    "Update library asset"
                                } else {
                                    "Save to library & place"
                                },
                            )
                            .primary(),
                        )
                        .clicked();
                    cancel=ui.add(ActionButton::new(Icon::Left,"Cancel")).clicked();
                });
            });
        if !egui::Popup::is_any_open(ctx)
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            cancel = true;
        }
        if save {
            let svg = &self.editor.svg;
            let result = (|| -> anyhow::Result<Asset> {
                let tree = mypdf_milestone0::svg::tree(&svg.source, None)?;
                Ok(Asset::new(
                    &svg.name,
                    Kind::Svg {
                        source: svg.source.clone(),
                        tint: false,
                    },
                    Style::default(),
                    tree.size().width() / tree.size().height(),
                ))
            })();
            match result {
                Ok(mut asset) => {
                    asset.stamp = self.editor.svg.stamp;
                    if let Some(id) = &self.editor.svg.library_target {
                        asset.id = id.clone();
                        if let Some(old) = self
                            .editor
                            .library
                            .as_ref()
                            .and_then(|l| l.entries.iter().find(|a| &a.id == id))
                        {
                            asset.favorite = old.favorite;
                            asset.folder = old.folder.clone();
                            asset.tags = old.tags.clone();
                            asset.last_used = old.last_used;
                        }
                    }
                    if let Some((tab_id, id)) = self.editor.svg.target.clone() {
                        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id)
                            && let Some(s) = &mut tab.session
                        {
                            let mut anns = s.annotations.clone();
                            if let Some(a) = anns.iter_mut().find(|a| a.id == id) {
                                a.kind = asset.kind.clone();
                                match s.commit(anns) {
                                    Ok(()) => {
                                        self.editor.draft = None;
                                        open = false;
                                        self.status=Some(("SVG updated • move, resize, rotate or recolor in the inspector".into(),false));
                                    }
                                    Err(e) => self.editor.svg.error = Some(format!("{e:#}")),
                                }
                            }
                        }
                    } else {
                        match self.editor.add_asset(asset.clone()) {
                            Ok(()) => {
                                self.editor.placed = Some(asset);
                                self.editor.tool = Tool::Asset;
                                self.editor.mode = Mode::Annotate;
                                self.editor.library_open = true;
                                self.editor.library_tab = if self.editor.svg.stamp {
                                    "Stamps"
                                } else {
                                    "Saved"
                                }
                                .into();
                                self.editor.draft = None;
                                open = false;
                                self.status=Some(("SVG saved to your library • Click or drag on a page to place it".into(),false));
                            }
                            Err(e) => self.editor.svg.error = Some(e),
                        }
                    }
                }
                Err(e) => self.editor.svg.error = Some(format!("{e:#}")),
            }
        }
        self.editor.svg.open = open && !cancel;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn derived_artwork_cache_evicts_oldest_without_touching_library_or_originals() {
        let ctx = egui::Context::default();
        let mut editor = EditorState::new();
        let before = editor.library.as_ref().unwrap().entries.clone();
        for (name, frame) in [("older", 1), ("newer", 2)] {
            let texture = ctx.load_texture(
                name,
                egui::ColorImage::filled([16, 16], Color32::WHITE),
                egui::TextureOptions::LINEAR,
            );
            editor.images.insert(name.into(), texture);
            editor.image_used.insert(name.into(), frame);
        }
        assert_eq!(editor.image_cache_bytes(), 2048);
        assert_eq!(editor.trim_image_cache_to(1024), 1024);
        assert!(!editor.images.contains_key("older"));
        assert!(editor.images.contains_key("newer"));
        assert_eq!(editor.trim_image_cache_to(0), 1024);
        assert_eq!(editor.image_cache_bytes(), 0);
        assert!(editor.image_used.is_empty());
        assert_eq!(editor.library.as_ref().unwrap().entries, before);
    }
    fn test_frame(
        app: &mut Viewer,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        use eframe::App;
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 820.))),
                events,
                ..Default::default()
            },
            |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
        )
    }
    fn test_click(app: &mut Viewer, ctx: &egui::Context, label: &str) -> Rect {
        let output = test_frame(app, ctx, vec![]);
        let nodes = output.platform_output.accesskit_update.unwrap().nodes;
        let b = nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label))
            .unwrap_or_else(|| panic!("Missing {label}"))
            .1
            .bounds()
            .unwrap();
        let rect = Rect::from_min_max(
            pos2(b.x0 as f32, b.y0 as f32),
            pos2(b.x1 as f32, b.y1 as f32),
        );
        for pressed in [true, false] {
            test_frame(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(rect.center()),
                    egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        rect
    }
    #[test]
    fn inspector_transforms_commit_without_a_second_properties_row() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.tabs[0].session.is_none() {
            test_frame(&mut app, &ctx, vec![]);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.editor.mode = Mode::Draw;
        app.inspector = true;
        test_frame(&mut app, &ctx, vec![]);
        test_frame(&mut app, &ctx, vec![]);
        let top = app.tabs[0].document_view.unwrap().0.top();
        let a = Annotation::new(
            0,
            [10., 10., 400., 400.],
            Kind::Polygon {
                points: vec![[0., 0.], [1., 0.3], [0.2, 1.]],
                closed: true,
            },
            Style::default(),
        );
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![a.clone()])
            .unwrap();
        selection::set(&mut app.tabs[0], &mut app.editor, vec![a.id]);
        for (label, rotation, flip_x, flip_y) in [
            ("Rotate 90°", 90., false, false),
            ("Rotate −90°", 0., false, false),
            ("Flip horizontal", 0., true, false),
            ("Flip vertical", 0., true, true),
        ] {
            let rect = test_click(&mut app, &ctx, label);
            let view = app.tabs[0].document_view.unwrap().0;
            assert!(
                rect.left() >= view.right(),
                "object controls belong in the right inspector"
            );
            assert_eq!(
                view.top(),
                top,
                "selection must not move the page under the pointer"
            );
            let a = &app.tabs[0].session.as_ref().unwrap().annotations[0];
            assert_eq!((a.rotation, a.flip_x, a.flip_y), (rotation, flip_x, flip_y));
        }
        let s = app.tabs[0].session.as_mut().unwrap();
        s.undo();
        assert!(!s.annotations[0].flip_y);
        s.redo();
        assert!(s.annotations[0].flip_y);
        let mut group = s.annotations.clone();
        group.push(group[0].duplicate());
        group[1].rect = [410., 10., 510., 110.];
        let ids = group.iter().map(|a| a.id.clone()).collect();
        s.commit(group.clone()).unwrap();
        selection::set(&mut app.tabs[0], &mut app.editor, ids);
        let rect = test_click(&mut app, &ctx, "Rotate 90°");
        let view = app.tabs[0].document_view.unwrap().0;
        assert!(rect.left() >= view.right());
        assert_eq!(view.top(), top);
        assert_eq!(
            app.tabs[0].session.as_ref().unwrap().annotations,
            selection::transformed(&group, selection::Transform::Rotate(90.))
        );
    }
    #[test]
    fn signature_placement_uses_real_viewer_canvas_without_invisible_redaction_interception() {
        use eframe::App;
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        let frame = |app: &mut Viewer, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 820.))),
                    events,
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            )
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.tabs[0].session.is_none() {
            let _ = frame(&mut app, vec![]);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let asset = Asset::new(
            "Placement test",
            Kind::Text {
                text: "Signature".into(),
            },
            Style::default(),
            3.,
        );
        app.begin_asset_placement(asset);
        for _ in 0..4 {
            let _ = frame(&mut app, vec![]);
        }
        let viewport = app.tabs[0].document_view.unwrap().0;
        let point = viewport.min + vec2(viewport.width() / 2., 100.);
        for pressed in [None, Some(true), Some(false)] {
            let mut events = vec![egui::Event::PointerMoved(point)];
            if let Some(pressed) = pressed {
                events.push(egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                });
            }
            let _ = frame(&mut app, events);
        }
        assert!(
            app.workbench.paste.is_none(),
            "The page must accept the placement click"
        );
        assert_eq!(app.tabs[0].session.as_ref().unwrap().annotations.len(), 1);
        app.editor_undo(false);
        assert!(app.tabs[0].session.as_ref().unwrap().annotations.is_empty());
    }

    #[test]
    fn signature_settings_save_without_placing_and_picker_reuses_the_saved_artwork() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.tabs[0].session.is_none() {
            test_frame(&mut app, &ctx, vec![]);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.action("settings", &ctx);
        for _ in 0..3 {
            test_frame(&mut app, &ctx, vec![]);
        }
        test_click(&mut app, &ctx, "New ink signature…");
        assert!(app.editor.signature_manage_only);
        app.editor.signature_input = SignatureInput::Name;
        app.editor.signature_typed = "A. Signer".into();
        app.editor.signature_name = "Office ink".into();
        for _ in 0..3 {
            test_frame(&mut app, &ctx, vec![]);
        }
        for style in ["Flowing", "Personal", "Swift"] {
            test_click(&mut app, &ctx, style);
            test_frame(&mut app, &ctx, vec![]);
            assert!(app.editor.signature_generated.as_ref().unwrap().is_ok());
        }
        assert_eq!(app.editor.signature_name_style, 5);
        test_click(&mut app, &ctx, "Save signature");
        assert!(app.workbench.paste.is_none());
        assert!(app.tabs[0].session.as_ref().unwrap().annotations.is_empty());
        assert!(app.digital.open && app.digital.panel == digital::Panel::Ink);
        let asset = app
            .editor
            .library
            .as_ref()
            .unwrap()
            .entries
            .last()
            .unwrap()
            .clone();
        assert!(asset.signature);
        assert!(matches!(asset.kind, Kind::Svg { .. }));
        app.digital.open = false;
        let key = (
            app.tabs[0].id,
            app.tabs[0].session.as_ref().unwrap().revision(),
            0,
        );
        app.signature_here(key, [70., 90.]);
        for _ in 0..3 {
            test_frame(&mut app, &ctx, vec![]);
        }
        assert!(app.editor.signature_picker);
        test_click(&mut app, &ctx, "Office ink");
        assert!(!app.editor.signature_open);
        let marks = &app.tabs[0].session.as_ref().unwrap().annotations;
        assert_eq!(marks.len(), 1);
        assert_eq!(&marks[0].rect[..2], &[70., 90.]);
        assert_eq!(marks[0].asset_id.as_ref(), Some(&asset.id));
        app.action("settings", &ctx);
        for _ in 0..3 {
            test_frame(&mut app, &ctx, vec![]);
        }
        test_click(&mut app, &ctx, "Make default");
        assert!(app.digital.ink_is_default(&asset.id));
        test_click(&mut app, &ctx, "Delete…");
        for _ in 0..3 {
            test_frame(&mut app, &ctx, vec![]);
        }
        test_click(&mut app, &ctx, "Cancel");
        for _ in 0..3 {
            test_frame(&mut app, &ctx, vec![]);
        }
        assert!(
            app.editor
                .library
                .as_ref()
                .unwrap()
                .entries
                .iter()
                .any(|a| a.id == asset.id)
        );
        test_click(&mut app, &ctx, "Delete…");
        for _ in 0..3 {
            test_frame(&mut app, &ctx, vec![]);
        }
        test_click(&mut app, &ctx, "Delete");
        assert!(
            !app.editor
                .library
                .as_ref()
                .unwrap()
                .entries
                .iter()
                .any(|a| a.id == asset.id)
        );
        assert!(!app.digital.ink_is_default(&asset.id));
        assert_eq!(
            app.tabs[0].session.as_ref().unwrap().annotations.len(),
            1,
            "Deleting library artwork must preserve placed signatures"
        );
    }

    #[test]
    fn copied_ink_pastes_into_signature_creator_from_every_tab_with_name_focus() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let signature = Annotation::new(
            0,
            [100., 100., 300., 180.],
            Kind::Ink {
                strokes: vec![vec![[0.1, 0.8], [0.3, 0.1], [0.5, 0.7], [0.9, 0.2]]],
            },
            Style::default(),
        );
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![signature.clone()])
            .unwrap();
        selection::set(
            &mut app.tabs[0],
            &mut app.editor,
            vec![signature.id.clone()],
        );
        let library_count = app.editor.library.as_ref().unwrap().entries.len();
        for input in [
            SignatureInput::Draw,
            SignatureInput::Name,
            SignatureInput::Type,
            SignatureInput::Image,
        ] {
            assert!(app.copy_marks(&ctx, false));
            app.editor.signature_open = true;
            app.editor.signature_input = input;
            app.editor.signature_image = None;
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: egui::Key::V,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::COMMAND,
                    }],
                    ..Default::default()
                },
                |ctx| {
                    ctx.memory_mut(|m| m.request_focus(egui::Id::new("signature_name_focus")));
                    app.shortcuts(ctx);
                },
            );
            assert!(app.editor.signature_input == SignatureInput::Image);
            assert!(app.editor.signature_image.is_some());
            assert!(app.editor.signature_error.is_none());
            assert!(app.workbench.paste.is_none());
            assert_eq!(
                app.editor.library.as_ref().unwrap().entries.len(),
                library_count
            );
            assert_eq!(
                app.tabs[0].session.as_ref().unwrap().annotations,
                vec![signature.clone()]
            );
        }
    }

    #[test]
    fn signature_clipboard_preview_keeps_alpha_and_rejects_non_images() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let bytes = clipboard::png(200, 80, &vec![128; 200 * 80 * 4]).unwrap();
        let before = app.editor.library.as_ref().unwrap().entries.len();
        app.editor.signature_open = true;
        app.editor.signature_input = SignatureInput::Image;
        for _ in 0..3 {
            test_frame(&mut app, &ctx, vec![]);
        }
        clipboard::set_for_test(clipboard::Clip::Image(bytes.clone(), [200, 80]));
        test_click(&mut app, &ctx, "Paste image");
        let asset = app.editor.signature_image.clone().unwrap();
        assert_eq!(asset.aspect, 2.5);
        let Kind::Image { png } = &asset.kind else {
            panic!("Not an image")
        };
        assert_eq!(
            assets::decode_png(png)
                .unwrap()
                .to_rgba8()
                .get_pixel(0, 0)
                .0,
            [128; 4]
        );
        assert_eq!(app.editor.library.as_ref().unwrap().entries.len(), before);
        clipboard::set_for_test(clipboard::Clip::Text("Not an image".into()));
        app.paste_signature_image();
        assert!(app.editor.signature_error.is_some());
        assert_eq!(app.editor.signature_image, Some(asset));
        clipboard::set_for_test(clipboard::Clip::Image(bytes, [200, 80]));
        // A copied image must paste even while the library-name field owns focus.
        ctx.memory_mut(|m| m.request_focus(egui::Id::new("signature_name_focus")));
        app.action("paste", &ctx);
        assert!(app.editor.signature_error.is_none());
        assert!(app.workbench.paste.is_none());
        for _ in 0..3 {
            test_frame(&mut app, &ctx, vec![]);
        }
        test_click(&mut app, &ctx, "Cancel");
        assert!(!app.editor.signature_open);
        assert_eq!(app.editor.library.as_ref().unwrap().entries.len(), before);
    }

    #[test]
    fn signature_save_and_place_uses_clicked_position_and_supports_undo_save_and_cancel() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.tabs[0].session.is_none() {
            test_frame(&mut app, &ctx, vec![]);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.editor.mode = Mode::Sign;
        app.editor.signature_open = true;
        let bytes = clipboard::png(200, 80, &vec![128; 200 * 80 * 4]).unwrap();
        // Exercise the real MyPDF Copy -> clipboard image -> Signature path.
        assert!(
            app.copy_artwork(
                &ctx,
                vec![Annotation::new(
                    0,
                    [0., 0., 200., 80.],
                    Kind::Image {
                        png: Arc::new(bytes)
                    },
                    Style::default()
                )]
            )
            .is_some()
        );
        app.paste_signature_image();
        assert!(app.editor.signature_image.is_some());
        for _ in 0..3 {
            test_frame(&mut app, &ctx, vec![]);
        }
        test_click(&mut app, &ctx, "Save & place");
        assert!(!app.editor.signature_open);
        assert!(!app.editor.library_open);
        assert!(app.workbench.paste.is_some());
        assert!(app.tabs[0].session.as_ref().unwrap().annotations.is_empty());
        let rect = Rect::from_min_size(pos2(0., 0.), vec2(600., 600.));
        for pressed in [None, Some(true), Some(false)] {
            let mut events = vec![egui::Event::PointerMoved(pos2(160., 170.))];
            if let Some(pressed) = pressed {
                events.push(egui::Event::PointerButton {
                    pos: pos2(160., 170.),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                });
            }
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(rect),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        workbench::paste_overlay(
                            &mut app.workbench,
                            &mut app.editor,
                            &mut app.tabs[0],
                            ui,
                            0,
                            rect,
                            1.,
                            &mut app.status,
                        );
                    });
                },
            );
        }
        assert!(app.workbench.paste.is_none());
        let s = app.tabs[0].session.as_mut().unwrap();
        assert_eq!(s.annotations.len(), 1);
        assert_eq!(s.annotations[0].rect, [160., 170., 340., 242.]);
        let placed = s.annotations.clone();
        s.undo();
        assert!(s.annotations.is_empty());
        s.redo();
        assert_eq!(s.annotations, placed);
        let saved = lopdf::Document::load_mem(&s.snapshot_bytes().unwrap()).unwrap();
        assert_eq!(ann::load(&saved), placed);
        let asset = app.editor.placed.clone().unwrap();
        app.begin_asset_placement(asset);
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
                ..Default::default()
            },
            |ctx| {
                app.workbench_keys(ctx);
            },
        );
        assert!(app.workbench.paste.is_none());
        assert_eq!(app.tabs[0].session.as_ref().unwrap().annotations, placed);
    }

    #[test]
    fn signature_context_confirmation_places_at_anchor_and_cancel_discards_anchor() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.tabs[0].session.is_none() {
            test_frame(&mut app, &ctx, vec![]);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let key = (
            app.tabs[0].id,
            app.tabs[0].session.as_ref().unwrap().revision(),
            0,
        );
        // Creation lives in Settings; the completed signature can still use an
        // explicit right-click placement anchor.
        app.signature_here(key, [120., 230.]);
        app.editor.signature_picker = false;
        app.editor.signature_input = SignatureInput::Type;
        app.editor.signature_typed = "Test signature".into();
        for _ in 0..3 {
            test_frame(&mut app, &ctx, vec![]);
        }
        test_click(&mut app, &ctx, "Save & place");
        assert!(!app.editor.signature_open);
        assert!(app.workbench.paste.is_none());
        assert!(app.editor.signature_placement.is_none());
        let s = app.tabs[0].session.as_mut().unwrap();
        assert_eq!(s.annotations.len(), 1);
        assert_eq!(s.annotations[0].rect, [120., 230., 300., 290.]);
        s.undo();
        assert!(s.annotations.is_empty());
        s.redo();
        assert_eq!(s.annotations.len(), 1);
        let key = (
            app.tabs[0].id,
            app.tabs[0].session.as_ref().unwrap().revision(),
            0,
        );
        app.signature_here(key, [320., 330.]);
        for _ in 0..3 {
            test_frame(&mut app, &ctx, vec![]);
        }
        test_click(&mut app, &ctx, "Cancel");
        assert!(app.editor.signature_placement.is_none());
        assert_eq!(app.tabs[0].session.as_ref().unwrap().annotations.len(), 1);
        // A changed document must not receive a signature using a stale menu target.
        app.signature_here(key, [320., 330.]);
        app.tabs[0].session.as_mut().unwrap().undo();
        let asset = app
            .editor
            .library
            .as_ref()
            .unwrap()
            .entries
            .last()
            .unwrap()
            .clone();
        app.place_saved_signature(asset);
        assert!(app.tabs[0].session.as_ref().unwrap().annotations.is_empty());
        assert!(app.workbench.paste.is_none());
        assert!(app.status.as_ref().unwrap().0.contains("document changed"));
    }

    #[test]
    fn signature_image_is_previewed_then_saved_only_on_confirmation() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("Signature.png");
        image::RgbaImage::from_pixel(200, 80, image::Rgba([0, 0, 0, 128]))
            .save(&path)
            .unwrap();
        let asset = Asset::from_image(&path).unwrap();
        let before = app.editor.library.as_ref().unwrap().entries.len();
        app.editor.signature_input = SignatureInput::Image;
        app.editor.signature_open = true;
        app.editor.signature_tx.send(Ok(asset.clone())).unwrap();
        app.editor_events();
        assert_eq!(app.editor.library.as_ref().unwrap().entries.len(), before);
        for _ in 0..3 {
            test_frame(&mut app, &ctx, vec![]);
        }
        test_click(&mut app, &ctx, "Save signature");
        assert!(!app.editor.signature_open);
        let saved = app.editor.library.as_ref().unwrap().entries.last().unwrap();
        assert_eq!(saved.kind, asset.kind);
        assert_eq!(saved.aspect, 2.5);
        assert_eq!(
            app.editor.library.as_ref().unwrap().entries.len(),
            before + 1
        );
    }
    #[test]
    fn artwork_dialog_footers_remain_visible_in_small_windows_and_escape_cancels() {
        for svg in [true, false] {
            for dark in [false, true] {
                let ctx = egui::Context::default();
                ctx.enable_accesskit();
                let mut app =
                    Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
                ctx.set_theme(if dark {
                    egui::ThemePreference::Dark
                } else {
                    egui::ThemePreference::Light
                });
                if svg {
                    app.editor.svg.open = true;
                    app.editor.svg.prompt_open = true;
                    app.editor.svg.prompt = "A green approval stamp with a check mark".into();
                    app.editor.svg.source = "<svg xmlns='http://www.w3.org/2000/svg' width='20' height='20'><circle cx='10' cy='10' r='8'/></svg>".into();
                    app.editor.svg.error = Some("An intentionally long validation message to ensure errors stay inside the scrolling body instead of pushing the actions offscreen. ".repeat(5));
                } else {
                    app.editor.signature_open = true;
                    app.editor.signature_input = SignatureInput::Image;
                    app.editor.signature_error =
                        Some("An intentionally long image validation error. ".repeat(10));
                }
                let frame = |app: &mut Viewer, events| {
                    ctx.run(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(720., 480.))),
                            events,
                            ..Default::default()
                        },
                        |ctx| app.editor_windows(ctx),
                    )
                };
                let mut found = HashMap::new();
                for _ in 0..4 {
                    for (_, n) in frame(&mut app, vec![])
                        .platform_output
                        .accesskit_update
                        .unwrap()
                        .nodes
                    {
                        if let (Some(label), Some(bounds)) = (n.label(), n.bounds()) {
                            found.insert(label.to_owned(), bounds);
                        }
                    }
                }
                let primary = if svg {
                    "Save to library & place"
                } else {
                    "Save signature"
                };
                for label in [primary, "Cancel"] {
                    let bounds = found
                        .get(label)
                        .unwrap_or_else(|| panic!("Missing {label}"));
                    assert!(
                        bounds.x0 >= 0.
                            && bounds.y0 >= 0.
                            && bounds.x1 <= 720.
                            && bounds.y1 <= 480.,
                        "{label} offscreen: {bounds:?}"
                    );
                    assert!(bounds.y1 - bounds.y0 >= 30.);
                }
                frame(
                    &mut app,
                    vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
                assert!(!app.editor.svg.open && !app.editor.signature_open);
            }
        }
    }
    #[test]
    fn empty_annotation_inspector_uses_current_tool_guidance() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        app.editor.mode = Mode::Annotate;
        for (tool, title) in [
            (Tool::Select, "Annotations"),
            (Tool::Highlight, "Highlight"),
        ] {
            app.editor.tool = tool;
            let mut labels = Vec::new();
            for _ in 0..3 {
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 820.))),
                        ..Default::default()
                    },
                    |ctx| app.annotation_inspector(ctx),
                );
                labels.extend(output.shapes.iter().filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) => Some(t.galley.job.text.clone()),
                    _ => None,
                }));
            }
            assert!(
                labels.iter().any(|n| n == title),
                "Expected {title}, found {labels:?}"
            );
            assert!(!labels.iter().any(|n| n == "Drawing" || n == "Stroke"));
        }
    }
    #[test]
    fn selection_border_and_handle_follow_each_drag_frame() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.editor.mode = Mode::Draw;
        app.editor.snapping = false;
        app.editor.tool = Tool::Select;
        for (rotation, scale, previously_selected_other) in
            [(0., 1., false), (30., 1.5, false), (25., 0.8, true)]
        {
            let mut mark = Annotation::new(
                0,
                [100., 100., 240., 220.],
                Kind::Rectangle,
                Style::default(),
            );
            mark.rotation = rotation;
            let other =
                Annotation::new(0, [360., 80., 420., 150.], Kind::Ellipse, Style::default());
            app.tabs[0].selected_annotation = Some(
                if previously_selected_other {
                    &other.id
                } else {
                    &mark.id
                }
                .clone(),
            );
            app.tabs[0]
                .session
                .as_mut()
                .unwrap()
                .commit(vec![mark.clone(), other.clone()])
                .unwrap();
            let revision = app.tabs[0].session.as_ref().unwrap().revision();
            let origin = pos2(73., 61.);
            let frame = |app: &mut Viewer, point: [f32; 2], down: Option<bool>| {
                let p = origin + vec2(point[0], point[1]) * scale;
                let mut events = vec![egui::Event::PointerMoved(p)];
                if let Some(pressed) = down {
                    events.push(egui::Event::PointerButton {
                        pos: p,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 1400.))),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            page_interaction(
                                &mut app.editor,
                                &mut app.tabs[0],
                                ui,
                                0,
                                Rect::from_min_size(origin, vec2(595., 842.) * scale),
                                scale,
                                &mut app.status,
                            );
                        });
                    },
                )
            };
            let assert_frame = |output: egui::FullOutput, expected: &Annotation| {
                let b = expected.bounds();
                let rect = Rect::from_min_max(
                    origin + vec2(b[0], b[1]) * scale,
                    origin + vec2(b[2], b[3]) * scale,
                );
                let near =
                    |a: Rect, b: Rect| a.min.distance(b.min) < 0.02 && a.max.distance(b.max) < 0.02;
                let borders: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|s| match &s.shape {
                        egui::Shape::Rect(r) if r.stroke.color == theme::accent() => Some(r.rect),
                        _ => None,
                    })
                    .collect();
                assert_eq!(borders.len(), 1, "exactly one selection border");
                assert!(
                    near(borders[0], rect.expand(3.)),
                    "border {:?} must track {:?} before release",
                    borders[0],
                    rect.expand(3.)
                );
                let handles: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|s| match &s.shape {
                        egui::Shape::Circle(c) if c.stroke.color == theme::accent() => {
                            Some(c.center)
                        }
                        _ => None,
                    })
                    .collect();
                let frame = rect.expand(3.);
                assert_eq!(handles.len(), 4, "a round handle on each corner");
                for corner in [
                    frame.left_top(),
                    frame.right_top(),
                    frame.right_bottom(),
                    frame.left_bottom(),
                ] {
                    assert!(
                        handles.iter().any(|h| h.distance(corner) < 0.02),
                        "handles must track the same frame"
                    );
                }
                assert!(
                    output.shapes.iter().any(|s| match &s.shape {
                        egui::Shape::Path(p) => near(Rect::from_points(&p.points), rect),
                        _ => false,
                    }),
                    "drawing and border must use the same geometry"
                );
            };
            frame(&mut app, [170., 160.], None);
            frame(&mut app, [170., 160.], Some(true));
            let mut moved = mark.clone();
            for end in [[210., 190.], [260., 235.], [10., 20.], [245., 205.]] {
                let output = frame(&mut app, end, None);
                let x = (100_f32 + end[0] - 170.).max(0.);
                let y = (100_f32 + end[1] - 160.).max(0.);
                moved.rect = [x, y, x + 140., y + 120.];
                assert_frame(output, &moved);
                let s = app.tabs[0].session.as_ref().unwrap();
                assert_eq!(s.revision(), revision, "moving only changes the preview");
                assert_eq!(s.annotations[0], mark);
            }
            assert_frame(frame(&mut app, [245., 205.], Some(false)), &moved);
            assert!(app.editor.gesture.is_none());
            let s = app.tabs[0].session.as_ref().unwrap();
            assert_eq!(s.revision(), revision + 1, "one undo step per drag");
            assert!(
                s.annotations[0]
                    .rect
                    .into_iter()
                    .zip(moved.rect)
                    .all(|(a, b)| (a - b).abs() < 0.001)
            );
            assert_eq!(s.annotations[1], other);
            let b = moved.bounds();
            frame(&mut app, [b[2], b[3]], Some(true));
            let mut resized = moved.clone();
            for delta in [20., 45.] {
                let end = [b[2] + delta, b[3] + delta];
                resized.rect[2] = end[0];
                resized.rect[3] = end[1];
                assert_frame(frame(&mut app, end, None), &resized);
                assert_eq!(
                    app.tabs[0].session.as_ref().unwrap().revision(),
                    revision + 1
                );
            }
            assert_frame(
                frame(&mut app, [b[2] + 45., b[3] + 45.], Some(false)),
                &resized,
            );
            let session = app.tabs[0].session.as_mut().unwrap();
            assert!(
                session.annotations[0]
                    .rect
                    .into_iter()
                    .zip(resized.rect)
                    .all(|(a, b)| (a - b).abs() < 0.001)
            );
            session.undo();
            assert!(
                session.annotations[0]
                    .rect
                    .into_iter()
                    .zip(moved.rect)
                    .all(|(a, b)| (a - b).abs() < 0.001)
            );
            session.undo();
            assert_eq!(session.annotations[0], mark);
        }
    }
    fn input(app: &mut Viewer, ctx: &egui::Context, events: Vec<egui::Event>) {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 900.0))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    page_interaction(
                        &mut app.editor,
                        &mut app.tabs[0],
                        ui,
                        0,
                        Rect::from_min_size(pos2(50.0, 50.0), vec2(612.0, 792.0)),
                        1.0,
                        &mut app.status,
                    );
                });
            },
        );
    }
    fn pointer(app: &mut Viewer, ctx: &egui::Context, p: [f32; 2], down: Option<bool>) {
        let p = pos2(p[0] + 50.0, p[1] + 50.0);
        let mut events = vec![egui::Event::PointerMoved(p)];
        if let Some(pressed) = down {
            events.push(egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        input(app, ctx, events);
    }
    #[test]
    fn rotation_handle_drag_is_one_undo_step_and_keeps_object_center() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.editor.mode = Mode::Draw;
        app.editor.tool = Tool::Select;
        let mark = Annotation::new(
            0,
            [150., 150., 250., 220.],
            Kind::Rectangle,
            Default::default(),
        );
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![mark.clone()])
            .unwrap();
        selection::set(&mut app.tabs[0], &mut app.editor, vec![mark.id.clone()]);
        let revision = app.tabs[0].session.as_ref().unwrap().revision();
        input(&mut app, &ctx, vec![]);
        input(&mut app, &ctx, vec![]);
        let start = manipulation::rotation_handle(&mark, 1.).unwrap();
        pointer(&mut app, &ctx, start, Some(true));
        pointer(&mut app, &ctx, [260., 150.], None);
        pointer(&mut app, &ctx, [260., 185.], None);
        pointer(&mut app, &ctx, [260., 185.], Some(false));
        let s = app.tabs[0].session.as_mut().unwrap();
        assert_eq!(s.revision(), revision + 1);
        assert!(
            (s.annotations[0].rotation - 90.).abs() < 0.01,
            "{:?}",
            s.annotations[0]
        );
        assert_eq!(s.annotations[0].rect, mark.rect);
        s.undo();
        assert_eq!(s.annotations, vec![mark]);
    }
    #[test]
    fn corner_resizing_anchors_the_opposite_corner_and_can_keep_proportions() {
        let r = [100.0, 100.0, 200.0, 150.0];
        assert_eq!(
            resized(r, 2, [260.0, 170.0], false),
            [100.0, 100.0, 260.0, 170.0]
        );
        let free_top_left = resized(r, 0, [80.0, 90.0], false);
        assert_eq!(free_top_left, [80.0, 90.0, 200.0, 150.0]);
        let kept = resized(r, 2, [300.0, 160.0], true);
        assert_eq!(kept, [100.0, 100.0, 300.0, 200.0], "2:1 stays 2:1");
        let kept = resized(r, 0, [90.0, 40.0], true);
        assert_eq!([kept[2], kept[3]], [200.0, 150.0]);
        assert!(((kept[2] - kept[0]) / (kept[3] - kept[1]) - 2.0).abs() < 1e-4);
        let tiny = resized(r, 2, [0.0, 0.0], true);
        assert!(tiny[2] - tiny[0] >= 8.0 && tiny[3] - tiny[1] >= 8.0);
    }
    #[test]
    fn library_items_drop_onto_the_page_and_artwork_resizes_in_proportion() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.editor.mode = Mode::Annotate;
        app.editor.tool = Tool::Select;
        input(&mut app, &ctx, vec![]);
        let cursor = |app: &mut Viewer, p: [f32; 2]| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 900.0))),
                    events: vec![egui::Event::PointerMoved(pos2(p[0] + 50.0, p[1] + 50.0))],
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        page_interaction(
                            &mut app.editor,
                            &mut app.tabs[0],
                            ui,
                            0,
                            Rect::from_min_size(pos2(50.0, 50.0), vec2(612.0, 792.0)),
                            1.0,
                            &mut app.status,
                        );
                    });
                },
            )
            .platform_output
            .cursor_icon
        };
        let symbol = app
            .editor
            .builtins
            .iter()
            .find(|b| b.category == "Symbols")
            .unwrap()
            .asset
            .clone();

        // A card being dragged from the library carries its asset; releasing places it.
        egui::DragAndDrop::set_payload(&ctx, symbol);
        assert_eq!(cursor(&mut app, [300.0, 400.0]), egui::CursorIcon::Grabbing);
        pointer(&mut app, &ctx, [300.0, 400.0], Some(false));
        let annotations = &app.tabs[0].session.as_ref().unwrap().annotations;
        assert_eq!(annotations.len(), 1, "{:?}", app.status);
        let a = annotations[0].clone();
        assert!(keeps_proportions(&a.kind), "{:?}", a.kind);
        assert!(((a.rect[0] + a.rect[2]) / 2.0 - 300.0).abs() < 0.5);
        assert!(((a.rect[1] + a.rect[3]) / 2.0 - 400.0).abs() < 0.5);
        assert_eq!(app.tabs[0].selected_annotation.as_ref(), Some(&a.id));
        assert!(!egui::DragAndDrop::has_any_payload(&ctx));

        // Grab over the artwork, diagonal arrows on its corners.
        assert_eq!(cursor(&mut app, [300.0, 400.0]), egui::CursorIcon::Grab);
        assert_eq!(
            cursor(&mut app, [a.rect[2], a.rect[3]]),
            egui::CursorIcon::ResizeNwSe
        );
        assert_eq!(
            cursor(&mut app, [a.rect[2], a.rect[1]]),
            egui::CursorIcon::ResizeNeSw
        );

        // Dragging a corner sideways still scales both sides together.
        let ratio = |r: Box2| (r[2] - r[0]) / (r[3] - r[1]);
        pointer(&mut app, &ctx, [a.rect[2], a.rect[3]], Some(true));
        pointer(&mut app, &ctx, [a.rect[2] + 40.0, a.rect[3] + 2.0], None);
        pointer(&mut app, &ctx, [a.rect[2] + 80.0, a.rect[3] + 5.0], None);
        pointer(
            &mut app,
            &ctx,
            [a.rect[2] + 80.0, a.rect[3] + 5.0],
            Some(false),
        );
        let b = app.tabs[0].session.as_ref().unwrap().annotations[0].rect;
        assert!(b[2] - b[0] > a.rect[2] - a.rect[0] + 60.0, "{b:?}");
        assert!(
            (ratio(b) - ratio(a.rect)).abs() < 0.01,
            "{b:?} vs {:?}",
            a.rect
        );
        assert_eq!([b[0], b[1]], [a.rect[0], a.rect[1]]);

        // The top-left handle keeps the bottom-right corner in place.
        pointer(&mut app, &ctx, [b[0], b[1]], Some(true));
        pointer(&mut app, &ctx, [b[0] - 15.0, b[1] - 20.0], None);
        pointer(&mut app, &ctx, [b[0] - 30.0, b[1] - 30.0], None);
        pointer(&mut app, &ctx, [b[0] - 30.0, b[1] - 30.0], Some(false));
        let c = app.tabs[0].session.as_ref().unwrap().annotations[0].rect;
        assert!(c[0] < b[0] && c[1] < b[1], "{c:?}");
        assert!((c[2] - b[2]).abs() < 0.01 && (c[3] - b[3]).abs() < 0.01);
        assert!((ratio(c) - ratio(a.rect)).abs() < 0.01);
    }
    #[test]
    fn vertex_tools_commit_once_cancel_and_survive_save() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.new_blank(&ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.editor.mode = Mode::Draw;
        app.editor.snapping = false;
        for (tool, closed) in [(Tool::Polyline, false), (Tool::Polygon, true)] {
            app.editor.select_tool(tool);
            input(&mut app, &ctx, vec![]);
            // Starting on existing artwork now selects it, so begin the next
            // vertex path on empty page space instead of the previous endpoint.
            for mut p in [[90., 100.], [260., 110.], [230., 230.]] {
                if closed {
                    p[1] += 180.;
                }
                pointer(&mut app, &ctx, p, Some(true));
                pointer(&mut app, &ctx, p, Some(false));
            }
            assert!(app.editor.vertices.is_some());
            input(
                &mut app,
                &ctx,
                vec![egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            let a = app.tabs[0]
                .session
                .as_ref()
                .unwrap()
                .annotations
                .last()
                .unwrap();
            assert!(
                matches!(&a.kind,Kind::Polygon { points, closed:c } if points.len()==3 && *c==closed)
            );
            assert!(app.editor.vertices.is_none());
        }
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), 2);
        app.editor.select_tool(Tool::Polygon);
        pointer(&mut app, &ctx, [80., 340.], Some(true));
        pointer(&mut app, &ctx, [80., 340.], Some(false));
        input(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(app.editor.vertices.is_none());
        app.editor.select_tool(Tool::DoubleArrow);
        pointer(&mut app, &ctx, [80., 350.], Some(true));
        pointer(&mut app, &ctx, [250., 430.], None);
        pointer(&mut app, &ctx, [250., 430.], Some(false));
        let session = app.tabs[0].session.as_ref().unwrap();
        assert_eq!(session.annotations.len(), 3);
        assert!(matches!(
            session.annotations[2].kind,
            Kind::DoubleArrow { .. }
        ));
        let saved = ann::export(&session.source, &session.annotations).unwrap();
        assert_eq!(
            ann::load(&lopdf::Document::load_mem(&saved).unwrap()),
            session.annotations
        );
        app.editor_undo(false);
        assert_eq!(app.tabs[0].session.as_ref().unwrap().annotations.len(), 2);
    }

    #[test]
    fn canvas_gestures_create_move_resize_undo_and_guard_signed_documents() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("canvas.pdf");
        std::fs::write(
            &path,
            mypdf_milestone0::document::fixture(
                &mypdf_milestone0::document::default_font().unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Viewer::new(&cc, vec![path]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !app.tabs[0].index_done {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.editor.mode = Mode::Annotate;
        app.editor.snapping = false;
        app.editor.tool = Tool::Select;
        input(&mut app, &ctx, vec![]);
        pointer(&mut app, &ctx, [150.0, 300.0], Some(true));
        pointer(&mut app, &ctx, [220.0, 350.0], None);
        pointer(&mut app, &ctx, [220.0, 350.0], Some(false));
        assert!(
            app.tabs[0].session.as_ref().unwrap().annotations.is_empty(),
            "Select on empty space must not create a rectangle"
        );
        app.editor.tool = Tool::Rectangle;
        input(&mut app, &ctx, vec![]);
        pointer(&mut app, &ctx, [150.0, 300.0], Some(true));
        pointer(&mut app, &ctx, [180.0, 330.0], None);
        pointer(&mut app, &ctx, [300.0, 400.0], None);
        pointer(&mut app, &ctx, [300.0, 400.0], Some(false));
        assert_eq!(app.tabs[0].session.as_ref().unwrap().annotations.len(), 1);
        let original = app.tabs[0].session.as_ref().unwrap().annotations[0].rect;
        app.editor.tool = Tool::Select;
        pointer(&mut app, &ctx, [220.0, 360.0], Some(true));
        pointer(&mut app, &ctx, [250.0, 380.0], None);
        pointer(&mut app, &ctx, [280.0, 400.0], None);
        pointer(&mut app, &ctx, [280.0, 400.0], Some(false));
        let moved = app.tabs[0].session.as_ref().unwrap().annotations[0].rect;
        assert!(moved[0] > original[0] && moved[1] > original[1]);
        pointer(&mut app, &ctx, [moved[2] - 1.0, moved[3] - 1.0], Some(true));
        pointer(&mut app, &ctx, [moved[2] + 20.0, moved[3] + 20.0], None);
        pointer(
            &mut app,
            &ctx,
            [moved[2] + 50.0, moved[3] + 35.0],
            Some(false),
        );
        let resized = app.tabs[0].session.as_ref().unwrap().annotations[0].rect;
        assert!(resized[2] > moved[2]);
        app.editor_undo(false);
        assert_eq!(
            app.tabs[0].session.as_ref().unwrap().annotations[0].rect,
            moved
        );
        app.editor_undo(false);
        assert_eq!(
            app.tabs[0].session.as_ref().unwrap().annotations[0].rect,
            original
        );
        app.editor_undo(false);
        assert!(app.tabs[0].session.as_ref().unwrap().annotations.is_empty());
        app.editor_undo(true);
        assert_eq!(app.tabs[0].session.as_ref().unwrap().annotations.len(), 1);
        let before_fast_drag = app.tabs[0].session.as_ref().unwrap().annotations[0].rect;
        let center = [
            (before_fast_drag[0] + before_fast_drag[2]) / 2.0,
            (before_fast_drag[1] + before_fast_drag[3]) / 2.0,
        ];
        pointer(&mut app, &ctx, center, Some(true));
        let end = pos2(center[0] + 180.0 + 50.0, center[1] + 60.0 + 50.0);
        input(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(end),
                egui::Event::PointerButton {
                    pos: end,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        assert_eq!(
            app.tabs[0].session.as_ref().unwrap().annotations[0].rect[0],
            before_fast_drag[0] + 180.0,
            "A release in the first drag frame must preserve its pressed origin"
        );
        let before_batch = app.tabs[0].session.as_ref().unwrap().annotations[0].rect;
        let start = pos2(
            (before_batch[0] + before_batch[2]) / 2.0 + 50.0,
            (before_batch[1] + before_batch[3]) / 2.0 + 50.0,
        );
        let end = start - vec2(100.0, 70.0);
        input(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerMoved(end),
                egui::Event::PointerButton {
                    pos: end,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        assert_eq!(
            app.tabs[0].session.as_ref().unwrap().annotations[0].rect[0],
            before_batch[0] - 100.0,
            "A complete drag in one frame must preserve its pressed origin"
        );
        let s = app.tabs[0].session.as_mut().unwrap();
        let mut entries = s.annotations.clone();
        entries[0].rotation = 25.0;
        s.commit(entries).unwrap();
        let before_rotated_resize = s.annotations[0].clone();
        app.tabs[0].selected_annotation = Some(before_rotated_resize.id.clone());
        let b = before_rotated_resize.bounds();
        pointer(&mut app, &ctx, [b[2] - 1.0, b[3] - 1.0], Some(true));
        pointer(&mut app, &ctx, [b[2] + 25.0, b[3] + 25.0], None);
        pointer(&mut app, &ctx, [b[2] + 40.0, b[3] + 40.0], Some(false));
        assert!(
            app.tabs[0].session.as_ref().unwrap().annotations[0].rect[2]
                > before_rotated_resize.rect[2],
            "The handle of a rotated mark must remain interactive"
        );
        let s = app.tabs[0].session.as_mut().unwrap();
        s.mark_saved(s.revision());
        app.editor.draft = Some(s.annotations[0].clone());
        app.tabs[0].selected_annotation = Some(app.editor.draft.as_ref().unwrap().id.clone());
        app.editor.draft.as_mut().unwrap().style.opacity = 0.5;
        assert!(app.apply_pending_annotation());
        assert!(app.tabs[0].session.as_ref().unwrap().dirty());
        let revision = app.tabs[0].session.as_ref().unwrap().revision();
        app.editor.draft.as_mut().unwrap().style.opacity = 0.0;
        assert!(!app.apply_pending_annotation());
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), revision);
        app.editor.draft.as_mut().unwrap().style.opacity = 0.5;
        app.request_close(Some(app.tabs[0].id), &ctx);
        assert!(app.close_intent.is_some());
        assert_eq!(app.tabs.len(), 1);
        app.close_intent = None;
        app.tabs[0].session.as_mut().unwrap().editable = false;
        app.editor.tool = Tool::Note;
        pointer(&mut app, &ctx, [450.0, 500.0], Some(true));
        pointer(&mut app, &ctx, [450.0, 500.0], Some(false));
        assert_eq!(app.tabs[0].session.as_ref().unwrap().annotations.len(), 1);
    }
    #[test]
    fn signature_pad_keeps_press_and_release_points_in_batched_frames() {
        let ctx = egui::Context::default();
        let mut editor = EditorState::new();
        let mut run = |events| {
            let mut rect = Rect::NOTHING;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 500.0))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        rect = draw_signature_pad(ui, &mut editor);
                    });
                },
            );
            rect
        };
        let rect = run(vec![]);
        let start = rect.min + vec2(80.0, 40.0);
        let end = rect.min + vec2(250.0, 100.0);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        run(vec![
            egui::Event::PointerMoved(rect.min),
            egui::Event::PointerMoved(start),
            button(start, true),
            egui::Event::PointerMoved(rect.min + vec2(150.0, 20.0)),
            egui::Event::PointerMoved(rect.min + vec2(200.0, 80.0)),
            egui::Event::PointerMoved(end),
            button(end, false),
        ]);
        assert_eq!(editor.signature_strokes.len(), 1);
        assert_eq!(editor.signature_strokes[0].len(), 4);
        assert!((editor.signature_strokes[0][0][0] - 80.0 / rect.width()).abs() < 0.001);
        assert!((editor.signature_strokes[0][1][1] - 20.0 / rect.height()).abs() < 0.001);
        assert!((editor.signature_strokes[0][3][1] - 100.0 / rect.height()).abs() < 0.001);
    }
}
