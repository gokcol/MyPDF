//! Designing AcroForm fields: text, check box, radio button, dropdown, list box,
//! button, image, date and signature fields, each with portable appearances
//! drawn here rather than left to the viewer. Also moving, editing, deleting,
//! clearing, tab order and flattening.
use crate::{content, pdf_font::PdfFont};
use anyhow::{Context, Result, bail, ensure};
use hayro::kurbo::{Affine, Point};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

const READ_ONLY: i64 = 1;
const REQUIRED: i64 = 1 << 1;
const MULTILINE: i64 = 1 << 12;
const NO_TOGGLE_OFF: i64 = 1 << 14;
const RADIO: i64 = 1 << 15;
const PUSH: i64 = 1 << 16;
const COMBO: i64 = 1 << 17;
const EDIT: i64 = 1 << 18;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FieldType {
    Text,
    Checkbox,
    Radio,
    Dropdown,
    ListBox,
    Button,
    Image,
    Date,
    Signature,
}
impl FieldType {
    pub const ALL: [FieldType; 9] = [
        FieldType::Text,
        FieldType::Checkbox,
        FieldType::Radio,
        FieldType::Dropdown,
        FieldType::ListBox,
        FieldType::Button,
        FieldType::Image,
        FieldType::Date,
        FieldType::Signature,
    ];
    pub fn label(self) -> &'static str {
        match self {
            FieldType::Text => "Text field",
            FieldType::Checkbox => "Check box",
            FieldType::Radio => "Radio button",
            FieldType::Dropdown => "Dropdown",
            FieldType::ListBox => "List box",
            FieldType::Button => "Button",
            FieldType::Image => "Image",
            FieldType::Date => "Date",
            FieldType::Signature => "Signature",
        }
    }
    /// The size of a field placed with a click, in points.
    pub fn default_size(self) -> [f32; 2] {
        match self {
            FieldType::Text => [180., 22.],
            FieldType::Checkbox | FieldType::Radio => [14., 14.],
            FieldType::Dropdown => [160., 22.],
            FieldType::ListBox => [160., 64.],
            FieldType::Button => [96., 24.],
            FieldType::Image => [120., 90.],
            FieldType::Date => [110., 22.],
            FieldType::Signature => [200., 54.],
        }
    }
    /// The base of generated names, as Acrobat numbers them: Text1, Check Box1, …
    pub fn base_name(self) -> &'static str {
        match self {
            FieldType::Text => "Text",
            FieldType::Checkbox => "Check Box",
            FieldType::Radio => "Group",
            FieldType::Dropdown => "Dropdown",
            FieldType::ListBox => "List Box",
            FieldType::Button => "Button",
            FieldType::Image => "Image",
            FieldType::Date => "Date",
            FieldType::Signature => "Signature",
        }
    }
    /// Whether people type or choose a value in it.
    pub fn fillable(self) -> bool {
        !matches!(
            self,
            FieldType::Button | FieldType::Image | FieldType::Signature
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckStyle {
    #[default]
    Check,
    Cross,
    Circle,
    Square,
}
impl CheckStyle {
    pub const ALL: [CheckStyle; 4] = [
        CheckStyle::Check,
        CheckStyle::Cross,
        CheckStyle::Circle,
        CheckStyle::Square,
    ];
    pub fn label(self) -> &'static str {
        match self {
            CheckStyle::Check => "Check",
            CheckStyle::Cross => "Cross",
            CheckStyle::Circle => "Circle",
            CheckStyle::Square => "Square",
        }
    }
    /// The ZapfDingbats character viewers use when they redraw the mark.
    fn character(self) -> &'static str {
        match self {
            CheckStyle::Check => "4",
            CheckStyle::Cross => "8",
            CheckStyle::Circle => "l",
            CheckStyle::Square => "n",
        }
    }
    fn from_character(c: &str) -> Self {
        match c {
            "8" => CheckStyle::Cross,
            "l" => CheckStyle::Circle,
            "n" => CheckStyle::Square,
            _ => CheckStyle::Check,
        }
    }
}
/// What a button does when it is clicked. No scripts are involved.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    #[default]
    None,
    ResetForm,
    Print,
    OpenLink(String),
}

/// Date formats in Acrobat's notation.
pub const DATE_FORMATS: [&str; 6] = [
    "dd.mm.yyyy",
    "dd/mm/yyyy",
    "mm/dd/yyyy",
    "yyyy-mm-dd",
    "d mmmm yyyy",
    "mmm d, yyyy",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Properties {
    pub name: String,
    pub tooltip: String,
    pub required: bool,
    pub read_only: bool,
    /// Text size in points; 0 fits the text to the field.
    pub font_size: f32,
    pub text_color: [f32; 3],
    pub border: Option<[f32; 3]>,
    pub background: Option<[f32; 3]>,
    pub align: Align,
    pub multiline: bool,
    pub max_length: Option<usize>,
    /// The value a new or cleared form shows: text, the chosen option, or a date.
    pub default_value: String,
    /// The value a check box or radio button sends when it is on.
    pub export_value: String,
    /// On by default (check box) or the default choice of its group (radio button).
    pub checked: bool,
    pub check_style: CheckStyle,
    pub options: Vec<String>,
    /// A dropdown also accepts typed text.
    pub editable: bool,
    pub caption: String,
    pub action: Action,
    pub date_format: String,
}
impl Properties {
    /// Sensible starting properties for a new field of this type.
    pub fn new(kind: FieldType, name: &str) -> Self {
        let border = Some([0.45, 0.52, 0.62]);
        Self {
            name: name.into(),
            tooltip: String::new(),
            required: false,
            read_only: false,
            font_size: match kind {
                FieldType::ListBox => 10.,
                _ => 0.,
            },
            text_color: [0.; 3],
            border,
            background: match kind {
                FieldType::Button => Some([0.86, 0.88, 0.92]),
                FieldType::Signature => Some([0.95, 0.97, 1.0]),
                _ => None,
            },
            align: match kind {
                FieldType::Button => Align::Center,
                _ => Align::Left,
            },
            multiline: false,
            max_length: None,
            default_value: String::new(),
            export_value: match kind {
                FieldType::Radio => "Choice1".into(),
                _ => "Yes".into(),
            },
            checked: false,
            check_style: match kind {
                FieldType::Radio => CheckStyle::Circle,
                _ => CheckStyle::Check,
            },
            options: match kind {
                FieldType::Dropdown | FieldType::ListBox => {
                    vec!["Option 1".into(), "Option 2".into(), "Option 3".into()]
                }
                _ => vec![],
            },
            editable: false,
            caption: match kind {
                FieldType::Button => "Button".into(),
                _ => String::new(),
            },
            action: Action::None,
            date_format: DATE_FORMATS[0].into(),
        }
    }
}

/// A field as the form designer sees it.
#[derive(Clone, Debug)]
pub struct Designed {
    /// The field: for a radio button, its group.
    pub id: ObjectId,
    /// The full name, including any parent names.
    pub name: String,
    pub kind: FieldType,
    pub properties: Properties,
    /// The current value: text, a choice, or a check box's or radio group's state.
    pub value: String,
    pub widgets: Vec<DesignWidget>,
    /// A signature field that already holds a signature.
    pub signed: bool,
}
#[derive(Clone, Debug)]
pub struct DesignWidget {
    pub id: ObjectId,
    pub page: usize,
    /// Displayed page coordinates: left, top, right, bottom.
    pub rect: [f32; 4],
    /// A radio button's or check box's on value.
    pub export: String,
    /// Position in the page's tab order, from 0.
    pub tab: usize,
}

/// A PDF text string: plain for printable ASCII, so every reader shows it, else UTF-16.
fn pdf_text(s: &str) -> Object {
    if s.chars().all(|c| (' '..='~').contains(&c)) {
        Object::string_literal(s)
    } else {
        crate::annotations::text_string(s)
    }
}
fn text_of(o: &Object) -> String {
    lopdf::decode_text_string(o).unwrap_or_else(|_| {
        o.as_name()
            .map(|v| String::from_utf8_lossy(v).into_owned())
            .unwrap_or_default()
    })
}
fn inherited(doc: &Document, id: ObjectId, key: &[u8]) -> Option<Object> {
    content::inherited(doc, id, key).ok().flatten()
}
fn flags(doc: &Document, id: ObjectId) -> i64 {
    inherited(doc, id, b"Ff")
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(0)
}
fn resolve<'a>(doc: &'a Document, o: &'a Object) -> &'a Object {
    doc.dereference(o).map(|(_, o)| o).unwrap_or(o)
}
fn dict_of<'a>(doc: &'a Document, d: &'a Dictionary, key: &[u8]) -> Option<&'a Dictionary> {
    d.get(key)
        .ok()
        .map(|o| resolve(doc, o))
        .and_then(|o| o.as_dict().ok())
}
fn rect_of(d: &Dictionary) -> Result<[f32; 4]> {
    let a = d.get(b"Rect")?.as_array()?;
    ensure!(a.len() == 4, "Invalid field rectangle");
    let v: Vec<f32> = a.iter().map(|o| o.as_float()).collect::<Result<_, _>>()?;
    Ok([
        v[0].min(v[2]),
        v[1].min(v[3]),
        v[0].max(v[2]),
        v[1].max(v[3]),
    ])
}
fn number(v: f32) -> String {
    let v = (v * 1000.).round() / 1000.;
    if v == v.trunc() {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}
fn rgb(c: [f32; 3]) -> String {
    format!("{} {} {}", number(c[0]), number(c[1]), number(c[2]))
}
fn color_of(o: Option<&Object>) -> Option<[f32; 3]> {
    let a = o?.as_array().ok()?;
    let v: Vec<f32> = a.iter().filter_map(|o| o.as_float().ok()).collect();
    match v.len() {
        1 => Some([v[0]; 3]),
        3 => Some([v[0], v[1], v[2]]),
        4 => Some([
            (1. - v[0]) * (1. - v[3]),
            (1. - v[1]) * (1. - v[3]),
            (1. - v[2]) * (1. - v[3]),
        ]),
        _ => None,
    }
}
fn color_array(c: [f32; 3]) -> Object {
    Object::Array(c.iter().map(|v| Object::Real(*v)).collect())
}

/// Font name, size and color from a default-appearance string.
fn parse_da(da: &str) -> (f32, [f32; 3]) {
    let tokens: Vec<&str> = da.split_whitespace().collect();
    let mut size = 0.;
    let mut color = [0.; 3];
    let n = |t: &str| t.parse::<f32>().ok();
    for (i, t) in tokens.iter().enumerate() {
        match *t {
            "Tf" if i >= 1 => size = n(tokens[i - 1]).unwrap_or(0.),
            "rg" if i >= 3 => {
                if let (Some(r), Some(g), Some(b)) =
                    (n(tokens[i - 3]), n(tokens[i - 2]), n(tokens[i - 1]))
                {
                    color = [r, g, b];
                }
            }
            "g" if i >= 1 => {
                if let Some(g) = n(tokens[i - 1]) {
                    color = [g; 3];
                }
            }
            _ => {}
        }
    }
    (size.clamp(0., 300.), color.map(|c| c.clamp(0., 1.)))
}

/// How a widget is drawn: colors, border, text size and alignment.
#[derive(Clone, Debug)]
pub(crate) struct Look {
    border: Option<[f32; 3]>,
    background: Option<[f32; 3]>,
    border_width: f32,
    size: f32,
    color: [f32; 3],
    align: i64,
}
fn acroform_dict(doc: &Document) -> Option<&Dictionary> {
    let form = doc.catalog().ok()?.get(b"AcroForm").ok()?;
    resolve(doc, form).as_dict().ok()
}
pub(crate) fn look(doc: &Document, field: ObjectId, widget: ObjectId) -> Look {
    let wd = doc.get_dictionary(widget).ok();
    let mk = wd.and_then(|d| dict_of(doc, d, b"MK"));
    let da = wd
        .and_then(|d| d.get(b"DA").ok())
        .map(text_of)
        .or_else(|| inherited(doc, field, b"DA").as_ref().map(text_of))
        .or_else(|| acroform_dict(doc)?.get(b"DA").ok().map(text_of))
        .unwrap_or_default();
    let (size, color) = parse_da(&da);
    let border_width = wd
        .and_then(|d| dict_of(doc, d, b"BS"))
        .and_then(|bs| bs.get(b"W").ok()?.as_float().ok())
        .unwrap_or(1.)
        .clamp(0., 12.);
    Look {
        border: mk.and_then(|m| color_of(m.get(b"BC").ok())),
        background: mk.and_then(|m| color_of(m.get(b"BG").ok())),
        border_width,
        size,
        color,
        align: inherited(doc, field, b"Q")
            .and_then(|o| o.as_i64().ok())
            .unwrap_or(0),
    }
}

/// A circle as four Bézier curves.
fn circle(cx: f32, cy: f32, r: f32) -> String {
    let k = r * 0.5523;
    format!(
        "{} {} m {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c h ",
        number(cx + r),
        number(cy),
        number(cx + r),
        number(cy + k),
        number(cx + k),
        number(cy + r),
        number(cx),
        number(cy + r),
        number(cx - k),
        number(cy + r),
        number(cx - r),
        number(cy + k),
        number(cx - r),
        number(cy),
        number(cx - r),
        number(cy - k),
        number(cx - k),
        number(cy - r),
        number(cx),
        number(cy - r),
        number(cx + k),
        number(cy - r),
        number(cx + r),
        number(cy - k),
        number(cx + r),
        number(cy),
    )
}
/// Background and border.
fn frame(look: &Look, w: f32, h: f32, round: bool) -> String {
    let mut body = String::new();
    let inset = look.border_width / 2.;
    let shape = |inset: f32| {
        if round {
            circle(w / 2., h / 2., (w.min(h) / 2. - inset).max(0.5))
        } else {
            format!(
                "{} {} {} {} re ",
                number(inset),
                number(inset),
                number((w - 2. * inset).max(0.)),
                number((h - 2. * inset).max(0.))
            )
        }
    };
    if let Some(bg) = look.background {
        body.push_str(&format!("{} rg {}f\n", rgb(bg), shape(0.)));
    }
    if let Some(bc) = look.border
        && look.border_width > 0.
    {
        body.push_str(&format!(
            "{} RG {} w {}S\n",
            rgb(bc),
            number(look.border_width),
            shape(inset)
        ));
    }
    body
}
/// An appearance drawn upright in a `w` × `h` box. On a rotated page its
/// matrix turns it with the page, so people read it the right way up.
fn form_stream(
    doc: &mut Document,
    widget: ObjectId,
    w: f32,
    h: f32,
    body: String,
    resources: Dictionary,
) -> Result<ObjectId> {
    let mut dict = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Form",
        "BBox" => vec![0.into(), 0.into(), Object::Real(w), Object::Real(h)],
        "Resources" => resources,
    };
    let matrix: Option<[i64; 4]> = match rotation(doc, widget) {
        90 => Some([0, 1, -1, 0]),
        180 => Some([-1, 0, 0, -1]),
        270 => Some([0, -1, 1, 0]),
        _ => None,
    };
    if let Some([a, b, c, d]) = matrix {
        dict.set(
            "Matrix",
            vec![a.into(), b.into(), c.into(), d.into(), 0.into(), 0.into()],
        );
    }
    let mut stream = Stream::new(dict, body.into_bytes());
    stream.compress()?;
    Ok(doc.add_object(stream))
}
/// How far a widget's appearance turns counterclockwise: 0, 90, 180 or 270.
fn rotation(doc: &Document, widget: ObjectId) -> i64 {
    let r = doc
        .get_dictionary(widget)
        .ok()
        .and_then(|d| dict_of(doc, d, b"MK"))
        .and_then(|mk| mk.get(b"R").ok()?.as_i64().ok())
        .unwrap_or(0)
        .rem_euclid(360);
    if r % 90 == 0 { r } else { 0 }
}
/// The widget's size as people see it: turned sideways on a rotated page.
fn widget_size(doc: &Document, widget: ObjectId) -> Result<(f32, f32)> {
    let r = rect_of(doc.get_dictionary(widget)?)?;
    let (w, h) = ((r[2] - r[0]).max(1.), (r[3] - r[1]).max(1.));
    Ok(if rotation(doc, widget) % 180 == 90 {
        (h, w)
    } else {
        (w, h)
    })
}

/// Lines and size that fit text into a box, shrinking from `start` to `minimum`.
fn fit_lines(
    font: &PdfFont,
    text: &str,
    multiline: bool,
    start: f32,
    width: f32,
    height: f32,
    minimum: f32,
) -> Option<(f32, Vec<String>)> {
    let mut size = start.max(minimum);
    loop {
        let mut lines = vec![];
        for paragraph in text.split('\n') {
            let mut line = String::new();
            for c in paragraph.chars() {
                let mut candidate = line.clone();
                candidate.push(c);
                if multiline && !line.is_empty() && font.width(&candidate, size) > width {
                    lines.push(std::mem::take(&mut line));
                }
                line.push(c);
            }
            lines.push(line);
        }
        let tall = if multiline {
            lines.len() as f32 * size * 1.2
        } else {
            size
        };
        if lines.iter().all(|l| font.width(l, size) <= width) && tall <= height {
            return Some((size, lines));
        }
        if size <= minimum {
            return None;
        }
        size = (size - 0.5).max(minimum);
    }
}

/// Text in a box: a text field's or date field's value, or a dropdown's choice.
#[allow(clippy::too_many_arguments)]
fn text_appearance(
    doc: &mut Document,
    widget: ObjectId,
    look: &Look,
    text: &str,
    multiline: bool,
    arrow: bool,
    strict: bool,
) -> Result<ObjectId> {
    let (w, h) = widget_size(doc, widget)?;
    let mut body = frame(look, w, h, false);
    let arrow_width = if arrow { 14.0_f32.min(w / 3.) } else { 0. };
    if arrow {
        let (x, y) = (w - arrow_width / 2. - 1., h / 2.);
        body.push_str(&format!(
            "{} rg {} {} m {} {} l {} {} l f\n",
            rgb(look.color),
            number(x - 3.5),
            number(y + 2.),
            number(x + 3.5),
            number(y + 2.),
            number(x),
            number(y - 2.5)
        ));
    }
    let display = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\t', "    ");
    let mut resources = Dictionary::new();
    if !display.is_empty() {
        let font = PdfFont::embed(doc, &display)?;
        let (avail_w, avail_h) = (w - 6. - arrow_width, h - 4.);
        let start = if look.size > 0. {
            look.size
        } else {
            12.0_f32.min(avail_h)
        };
        let fitted = fit_lines(
            &font,
            &display,
            multiline,
            start,
            avail_w,
            avail_h,
            if strict { 4. } else { 1. },
        );
        let Some((size, lines)) = fitted else {
            bail!("The value will not fit in this field. Shorten it before saving.");
        };
        body.push_str(&format!(
            "/Tx BMC q 2 2 {} {} re W n BT /F0 {} Tf {} rg\n",
            number((w - 4. - arrow_width).max(0.)),
            number(h - 4.),
            number(size),
            rgb(look.color)
        ));
        for (i, line) in lines.iter().enumerate() {
            let text_w = font.width(line, size);
            let x = match look.align {
                1 => (w - arrow_width - text_w) / 2.,
                2 => w - arrow_width - text_w - 3.,
                _ => 3.,
            };
            let y = if multiline {
                h - size - 3. - i as f32 * size * 1.2
            } else {
                (h - size) / 2. + size * 0.2
            };
            body.push_str(&format!(
                "1 0 0 1 {} {} Tm <{}> Tj\n",
                number(x),
                number(y),
                font.hex(line)?
            ));
        }
        body.push_str("ET Q EMC\n");
        resources.set("Font", dictionary! { "F0" => font.id });
    }
    form_stream(doc, widget, w, h, body, resources)
}

/// A list box: its options, the chosen one highlighted.
fn list_appearance(
    doc: &mut Document,
    widget: ObjectId,
    look: &Look,
    options: &[String],
    selected: Option<usize>,
) -> Result<ObjectId> {
    let (w, h) = widget_size(doc, widget)?;
    let mut body = frame(look, w, h, false);
    let mut resources = Dictionary::new();
    let all = options.join("");
    if !all.is_empty() {
        let font = PdfFont::embed(doc, &all)?;
        let size = if look.size > 0. {
            look.size
        } else {
            10.0_f32.min(h - 4.)
        }
        .max(1.);
        let row = size * 1.25;
        body.push_str(&format!(
            "q 1 1 {} {} re W n\n",
            number(w - 2.),
            number(h - 2.)
        ));
        for (i, option) in options.iter().enumerate() {
            let top = h - 2. - i as f32 * row;
            if top < 0. {
                break;
            }
            if Some(i) == selected {
                body.push_str(&format!(
                    "0.6 0.75 0.88 rg 1 {} {} {} re f\n",
                    number(top - row),
                    number(w - 2.),
                    number(row)
                ));
            }
            body.push_str(&format!(
                "BT /F0 {} Tf {} rg 1 0 0 1 3 {} Tm <{}> Tj ET\n",
                number(size),
                rgb(look.color),
                number(top - size),
                font.hex(option)?
            ));
        }
        body.push_str("Q\n");
        resources.set("Font", dictionary! { "F0" => font.id });
    }
    form_stream(doc, widget, w, h, body, resources)
}

/// A check box's or radio button's on or off state.
fn mark_appearance(
    doc: &mut Document,
    widget: ObjectId,
    look: &Look,
    round: bool,
    style: CheckStyle,
    on: bool,
) -> Result<ObjectId> {
    let (w, h) = widget_size(doc, widget)?;
    let mut body = frame(look, w, h, round);
    if on {
        let s = w.min(h);
        let (cx, cy) = (w / 2., h / 2.);
        let c = rgb(look.color);
        match style {
            CheckStyle::Check => body.push_str(&format!(
                "{c} RG 1 J 1 j {} w {} {} m {} {} l {} {} l S\n",
                number((s * 0.12).max(0.8)),
                number(cx - s * 0.28),
                number(cy + s * 0.02),
                number(cx - s * 0.08),
                number(cy - s * 0.2),
                number(cx + s * 0.3),
                number(cy + s * 0.24)
            )),
            CheckStyle::Cross => body.push_str(&format!(
                "{c} RG 1 J {} w {} {} m {} {} l {} {} m {} {} l S\n",
                number((s * 0.11).max(0.8)),
                number(cx - s * 0.24),
                number(cy - s * 0.24),
                number(cx + s * 0.24),
                number(cy + s * 0.24),
                number(cx - s * 0.24),
                number(cy + s * 0.24),
                number(cx + s * 0.24),
                number(cy - s * 0.24)
            )),
            CheckStyle::Circle => body.push_str(&format!("{c} rg {}f\n", circle(cx, cy, s * 0.22))),
            CheckStyle::Square => body.push_str(&format!(
                "{c} rg {} {} {} {} re f\n",
                number(cx - s * 0.22),
                number(cy - s * 0.22),
                number(s * 0.44),
                number(s * 0.44)
            )),
        }
    }
    form_stream(doc, widget, w, h, body, Dictionary::new())
}

/// A push button with its caption, or an image field with its picture.
fn button_appearance(
    doc: &mut Document,
    widget: ObjectId,
    look: &Look,
    caption: &str,
    icon: Option<(ObjectId, f32, f32)>,
) -> Result<ObjectId> {
    let (w, h) = widget_size(doc, widget)?;
    let mut body = frame(look, w, h, false);
    let mut resources = Dictionary::new();
    if let Some((image, iw, ih)) = icon {
        let scale = ((w - 4.) / iw).min((h - 4.) / ih);
        let (dw, dh) = (iw * scale, ih * scale);
        body.push_str(&format!(
            "q {} 0 0 {} {} {} cm /Img Do Q\n",
            number(dw),
            number(dh),
            number((w - dw) / 2.),
            number((h - dh) / 2.)
        ));
        resources.set("XObject", dictionary! { "Img" => image });
    }
    if !caption.trim().is_empty() {
        let font = PdfFont::embed(doc, caption)?;
        let start = if look.size > 0. {
            look.size
        } else {
            12.0_f32.min(h - 6.)
        };
        if let Some((size, lines)) = fit_lines(&font, caption, false, start, w - 8., h - 4., 1.) {
            let line = &lines[0];
            body.push_str(&format!(
                "BT /F0 {} Tf {} rg 1 0 0 1 {} {} Tm <{}> Tj ET\n",
                number(size),
                rgb(look.color),
                number((w - font.width(line, size)) / 2.),
                number((h - size) / 2. + size * 0.22),
                font.hex(line)?
            ));
            resources.set("Font", dictionary! { "F0" => font.id });
        }
    }
    form_stream(doc, widget, w, h, body, resources)
}

/// The widgets of a field: the field itself when it is merged with its widget,
/// or its widget kids.
pub(crate) fn widgets_of(doc: &Document, field: ObjectId) -> Vec<ObjectId> {
    let Ok(d) = doc.get_dictionary(field) else {
        return vec![];
    };
    if d.get(b"Subtype")
        .and_then(Object::as_name)
        .is_ok_and(|s| s == b"Widget")
    {
        return vec![field];
    }
    d.get(b"Kids")
        .ok()
        .map(|o| resolve(doc, o))
        .and_then(|o| o.as_array().ok())
        .map(|kids| {
            kids.iter()
                .filter_map(|k| k.as_reference().ok())
                .filter(|k| {
                    doc.get_dictionary(*k).is_ok_and(|kd| {
                        !kd.has(b"T")
                            && kd
                                .get(b"Subtype")
                                .and_then(Object::as_name)
                                .is_ok_and(|s| s == b"Widget")
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}
/// The field a widget belongs to.
fn field_of(doc: &Document, widget: ObjectId) -> ObjectId {
    let Ok(d) = doc.get_dictionary(widget) else {
        return widget;
    };
    if d.has(b"T") || d.has(b"FT") && !d.has(b"Parent") {
        return widget;
    }
    d.get(b"Parent")
        .and_then(Object::as_reference)
        .unwrap_or(widget)
}
/// On-state names of a check box or radio widget, from its appearances.
fn on_states(doc: &Document, widget: ObjectId) -> Vec<String> {
    doc.get_dictionary(widget)
        .ok()
        .and_then(|d| dict_of(doc, d, b"AP"))
        .and_then(|ap| dict_of(doc, ap, b"N"))
        .map(|n| {
            n.iter()
                .map(|(k, _)| String::from_utf8_lossy(k).into_owned())
                .filter(|k| k != "Off")
                .collect()
        })
        .unwrap_or_default()
}
fn options_of(doc: &Document, field: ObjectId) -> Vec<(String, String)> {
    inherited(doc, field, b"Opt")
        .and_then(|o| o.as_array().ok().cloned())
        .unwrap_or_default()
        .iter()
        .map(|o| match resolve(doc, o).as_array() {
            Ok(pair) => (
                pair.first().map(text_of).unwrap_or_default(),
                pair.get(1).map(text_of).unwrap_or_default(),
            ),
            Err(_) => {
                let v = text_of(o);
                (v.clone(), v)
            }
        })
        .collect()
}
fn javascript(doc: &Document, action: &Object) -> Option<String> {
    let action = resolve(doc, action).as_dict().ok()?;
    let js = resolve(doc, action.get(b"JS").ok()?);
    match js {
        Object::Stream(s) => Some(
            String::from_utf8_lossy(
                &s.decompressed_content()
                    .unwrap_or_else(|_| s.content.clone()),
            )
            .into_owned(),
        ),
        other => Some(text_of(other)),
    }
}
/// The Acrobat date format of a date field, if it is one.
pub fn date_format(doc: &Document, field: ObjectId) -> Option<String> {
    let d = doc.get_dictionary(field).ok()?;
    let aa = dict_of(doc, d, b"AA")?;
    let js = javascript(doc, aa.get(b"F").ok()?)?;
    let start = js.find("AFDate_FormatEx(")? + "AFDate_FormatEx(".len();
    let rest = &js[start..];
    let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let end = rest[1..].find(quote)?;
    let format = &rest[1..1 + end];
    (!format.is_empty() && format.len() <= 40).then(|| format.to_owned())
}
/// Whether a push button is an image field: Acrobat's picture chooser or ours.
pub fn is_image_field(doc: &Document, field: ObjectId) -> bool {
    let Ok(d) = doc.get_dictionary(field) else {
        return false;
    };
    if d.get(b"MyPDFKind")
        .and_then(Object::as_name)
        .is_ok_and(|k| k == b"Image")
    {
        return true;
    }
    widgets_of(doc, field)
        .into_iter()
        .chain([field])
        .filter_map(|w| doc.get_dictionary(w).ok())
        .filter_map(|w| w.get(b"A").ok())
        .filter_map(|a| javascript(doc, a))
        .any(|js| js.contains("buttonImportIcon"))
}
/// The kind of a field, if the designer understands it.
pub fn kind_of(doc: &Document, field: ObjectId) -> Option<FieldType> {
    let ft = inherited(doc, field, b"FT")?;
    let ff = flags(doc, field);
    Some(match ft.as_name().ok()? {
        b"Tx" if date_format(doc, field).is_some() => FieldType::Date,
        b"Tx" => FieldType::Text,
        b"Btn" if ff & PUSH != 0 && is_image_field(doc, field) => FieldType::Image,
        b"Btn" if ff & PUSH != 0 => FieldType::Button,
        b"Btn" if ff & RADIO != 0 => FieldType::Radio,
        b"Btn" => FieldType::Checkbox,
        b"Ch" if ff & COMBO != 0 => FieldType::Dropdown,
        b"Ch" => FieldType::ListBox,
        b"Sig" => FieldType::Signature,
        _ => return None,
    })
}
fn value_of(doc: &Document, field: ObjectId) -> String {
    inherited(doc, field, b"V")
        .as_ref()
        .map(text_of)
        .unwrap_or_default()
}
fn icon_of(doc: &Document, widget: ObjectId) -> Option<(ObjectId, f32, f32)> {
    let d = doc.get_dictionary(widget).ok()?;
    let mk = dict_of(doc, d, b"MK")?;
    let icon = mk.get(b"I").ok()?.as_reference().ok()?;
    let form = doc.get_object(icon).ok()?.as_stream().ok()?;
    let image = dict_of(doc, &form.dict, b"Resources")
        .and_then(|r| dict_of(doc, r, b"XObject"))
        .and_then(|x| x.get(b"Img").ok()?.as_reference().ok())?;
    let im = doc.get_object(image).ok()?.as_stream().ok()?;
    let w = im.dict.get(b"Width").ok()?.as_float().ok()?;
    let h = im.dict.get(b"Height").ok()?.as_float().ok()?;
    Some((image, w.max(1.), h.max(1.)))
}

/// Redraw every widget of a field from its current value and look.
/// `strict` refuses text that cannot fit instead of shrinking it to nothing.
pub(crate) fn refresh(doc: &mut Document, field: ObjectId, strict: bool) -> Result<()> {
    let kind = kind_of(doc, field).context("Unsupported form field")?;
    let value = value_of(doc, field);
    for widget in widgets_of(doc, field) {
        let look = look(doc, field, widget);
        let normal: Object = match kind {
            FieldType::Text | FieldType::Date => {
                let multiline = flags(doc, field) & MULTILINE != 0;
                text_appearance(doc, widget, &look, &value, multiline, false, strict)?.into()
            }
            FieldType::Dropdown => {
                let label = options_of(doc, field)
                    .into_iter()
                    .find(|(v, _)| *v == value)
                    .map_or(value.clone(), |(_, l)| l);
                text_appearance(doc, widget, &look, &label, false, true, strict)?.into()
            }
            FieldType::ListBox => {
                let options = options_of(doc, field);
                let selected = options.iter().position(|(v, _)| *v == value);
                let labels: Vec<String> = options.into_iter().map(|(_, l)| l).collect();
                list_appearance(doc, widget, &look, &labels, selected)?.into()
            }
            FieldType::Checkbox | FieldType::Radio => {
                let states = on_states(doc, widget);
                let on = states.first().cloned().unwrap_or_else(|| "Yes".into());
                let style = doc
                    .get_dictionary(widget)
                    .ok()
                    .and_then(|d| dict_of(doc, d, b"MK"))
                    .and_then(|mk| mk.get(b"CA").ok())
                    .map(|c| CheckStyle::from_character(&text_of(c)))
                    .unwrap_or(if kind == FieldType::Radio {
                        CheckStyle::Circle
                    } else {
                        CheckStyle::Check
                    });
                let round = kind == FieldType::Radio;
                let on_id = mark_appearance(doc, widget, &look, round, style, true)?;
                let off_id = mark_appearance(doc, widget, &look, round, style, false)?;
                let state = if value == on { on.as_str() } else { "Off" };
                doc.get_dictionary_mut(widget)?
                    .set("AS", Object::Name(state.as_bytes().to_vec()));
                let mut n = Dictionary::new();
                n.set(on.as_bytes(), on_id);
                n.set("Off", off_id);
                n.into()
            }
            FieldType::Button | FieldType::Image => {
                let caption = doc
                    .get_dictionary(widget)
                    .ok()
                    .and_then(|d| dict_of(doc, d, b"MK"))
                    .and_then(|mk| mk.get(b"CA").ok())
                    .map(text_of)
                    .unwrap_or_default();
                let icon = icon_of(doc, widget);
                button_appearance(doc, widget, &look, &caption, icon)?.into()
            }
            FieldType::Signature => {
                // A signed field keeps the appearance its signature drew.
                if inherited(doc, field, b"V").is_some() {
                    continue;
                }
                let (w, h) = widget_size(doc, widget)?;
                let body = frame(&look, w, h, false);
                form_stream(doc, widget, w, h, body, Dictionary::new())?.into()
            }
        };
        doc.get_dictionary_mut(widget)?
            .set("AP", dictionary! { "N" => normal });
    }
    Ok(())
}

/// Page number (from 0) and object of each page, and the page each annotation is on.
fn page_map(doc: &Document) -> (Vec<ObjectId>, HashMap<ObjectId, usize>) {
    let pages: Vec<ObjectId> = doc.get_pages().into_values().collect();
    let mut on = HashMap::new();
    for (i, page) in pages.iter().enumerate() {
        for a in annots(doc, *page) {
            if let Ok(id) = a.as_reference() {
                on.insert(id, i);
            }
        }
    }
    (pages, on)
}
fn annots(doc: &Document, page: ObjectId) -> Vec<Object> {
    doc.get_dictionary(page)
        .ok()
        .and_then(|d| d.get(b"Annots").ok())
        .map(|o| resolve(doc, o))
        .and_then(|o| o.as_array().ok().cloned())
        .unwrap_or_default()
}
fn set_annots(doc: &mut Document, page: ObjectId, list: Vec<Object>) -> Result<()> {
    let current = doc.get_dictionary(page)?.get(b"Annots").ok().cloned();
    match current {
        Some(Object::Reference(id)) => *doc.get_object_mut(id)? = Object::Array(list),
        _ => {
            doc.get_dictionary_mut(page)?.set("Annots", list);
        }
    }
    Ok(())
}
/// The AcroForm, made an indirect object with fields, fonts and defaults.
fn acroform(doc: &mut Document) -> Result<ObjectId> {
    let current = doc.catalog()?.get(b"AcroForm").ok().cloned();
    let id = match current {
        Some(Object::Reference(id)) => id,
        Some(Object::Dictionary(d)) => {
            let id = doc.add_object(d);
            doc.catalog_mut()?.set("AcroForm", id);
            id
        }
        _ => {
            let id = doc.add_object(dictionary! { "Fields" => Vec::<Object>::new() });
            doc.catalog_mut()?.set("AcroForm", id);
            id
        }
    };
    let form = doc.get_dictionary(id)?.clone();
    ensure!(
        !form.has(b"XFA"),
        "This PDF uses an XFA form. MyPDF works with standard PDF forms."
    );
    let fields = form
        .get(b"Fields")
        .ok()
        .map(|o| resolve(doc, o).clone())
        .and_then(|o| o.as_array().ok().cloned())
        .unwrap_or_default();
    let mut dr = form
        .get(b"DR")
        .ok()
        .map(|o| resolve(doc, o).clone())
        .and_then(|o| o.as_dict().ok().cloned())
        .unwrap_or_default();
    let mut fonts = dr
        .get(b"Font")
        .ok()
        .map(|o| resolve(doc, o).clone())
        .and_then(|o| o.as_dict().ok().cloned())
        .unwrap_or_default();
    if !fonts.has(b"Helv") {
        let helv = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding" });
        fonts.set("Helv", helv);
    }
    if !fonts.has(b"ZaDb") {
        let zadb = doc.add_object(
            dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "ZapfDingbats" },
        );
        fonts.set("ZaDb", zadb);
    }
    dr.set("Font", fonts);
    let form = doc.get_dictionary_mut(id)?;
    form.set("Fields", fields);
    form.set("DR", dr);
    if !form.has(b"DA") {
        form.set("DA", Object::string_literal("/Helv 0 Tf 0 g"));
    }
    Ok(id)
}
fn form_fields(doc: &Document, form: ObjectId) -> Vec<Object> {
    doc.get_dictionary(form)
        .ok()
        .and_then(|d| d.get(b"Fields").ok())
        .and_then(|o| o.as_array().ok().cloned())
        .unwrap_or_default()
}

/// Displayed-to-PDF mapping of a page, which may be turned by a right angle.
pub(crate) fn page_transform(inverse: [f64; 6]) -> Result<Affine> {
    let m = Affine::new(inverse);
    let c = m.as_coeffs();
    ensure!(
        c[1].abs() < 1e-6 && c[2].abs() < 1e-6 || c[0].abs() < 1e-6 && c[3].abs() < 1e-6,
        "This page is turned at an unusual angle, so fields cannot be placed on it"
    );
    Ok(m)
}
/// A page's rotation: 0, 90, 180 or 270 degrees clockwise.
fn page_rotation(doc: &Document, page: ObjectId) -> i64 {
    content::inherited(doc, page, b"Rotate")
        .ok()
        .flatten()
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(0)
        .rem_euclid(360)
        / 90
        * 90
}
fn to_pdf(m: Affine, rect: [f32; 4]) -> [f32; 4] {
    let a = m * Point::new(rect[0] as f64, rect[1] as f64);
    let b = m * Point::new(rect[2] as f64, rect[3] as f64);
    [
        a.x.min(b.x) as f32,
        a.y.min(b.y) as f32,
        a.x.max(b.x) as f32,
        a.y.max(b.y) as f32,
    ]
}
fn check_rect(rect: [f32; 4]) -> Result<()> {
    ensure!(
        rect.iter().all(|v| v.is_finite() && v.abs() < 100_000.)
            && rect[2] - rect[0] >= 4.
            && rect[3] - rect[1] >= 4.,
        "A field must be at least 4 points wide and tall"
    );
    Ok(())
}

/// Full names of the fields, and the field each name belongs to.
fn names(doc: &Document) -> HashMap<String, ObjectId> {
    let mut out = HashMap::new();
    let Some(form) = acroform_dict(doc) else {
        return out;
    };
    let roots = form
        .get(b"Fields")
        .ok()
        .map(|o| resolve(doc, o))
        .and_then(|o| o.as_array().ok().cloned())
        .unwrap_or_default();
    let mut pending: Vec<(ObjectId, String)> = roots
        .iter()
        .filter_map(|o| o.as_reference().ok())
        .map(|id| (id, String::new()))
        .collect();
    let mut seen = HashSet::new();
    while let Some((id, prefix)) = pending.pop() {
        if !seen.insert(id) || seen.len() > 10_000 {
            continue;
        }
        let Ok(d) = doc.get_dictionary(id) else {
            continue;
        };
        let local = d.get(b"T").map(text_of).unwrap_or_default();
        let name = match (prefix.is_empty(), local.is_empty()) {
            (true, _) => local,
            (false, true) => prefix,
            (false, false) => format!("{prefix}.{local}"),
        };
        let children: Vec<ObjectId> = d
            .get(b"Kids")
            .ok()
            .map(|o| resolve(doc, o))
            .and_then(|o| o.as_array().ok())
            .map(|kids| {
                kids.iter()
                    .filter_map(|k| k.as_reference().ok())
                    .filter(|k| {
                        doc.get_dictionary(*k)
                            .is_ok_and(|kd| kd.has(b"T") || kd.has(b"Kids") || kd.has(b"FT"))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if children.is_empty() {
            out.insert(name, id);
        } else {
            for child in children {
                pending.push((child, name.clone()));
            }
        }
    }
    out
}
/// A name for a new field of this kind: Text1, Text2, …
pub fn next_name(bytes: &[u8], kind: FieldType) -> String {
    let taken = Document::load_mem(bytes)
        .map(|doc| names(&doc))
        .unwrap_or_default();
    (1..)
        .map(|n| format!("{}{n}", kind.base_name()))
        .find(|n| !taken.contains_key(n))
        .unwrap()
}
fn check_name(name: &str) -> Result<()> {
    ensure!(
        !name.trim().is_empty()
            && name.chars().count() <= 120
            && !name.contains('.')
            && !name.chars().any(char::is_control),
        "Give the field a name of up to 120 characters, without dots"
    );
    Ok(())
}

/// Every field the designer understands, with its widgets in tab order.
pub fn describe(bytes: &[u8]) -> Result<Vec<Designed>> {
    let doc = Document::load_mem(bytes)?;
    if acroform_dict(&doc).is_none() {
        return Ok(vec![]);
    }
    ensure!(
        !acroform_dict(&doc).is_some_and(|f| f.has(b"XFA")),
        "This PDF uses an XFA form. MyPDF works with standard PDF forms."
    );
    let geometry = crate::annotations::page_geometry(bytes)?;
    let (pages, _) = page_map(&doc);
    let mut tab = HashMap::new();
    let mut page_of = HashMap::new();
    for (i, page) in pages.iter().enumerate() {
        let mut n = 0;
        for a in annots(&doc, *page) {
            if let Ok(id) = a.as_reference()
                && doc.get_dictionary(id).is_ok_and(|d| {
                    d.get(b"Subtype")
                        .and_then(Object::as_name)
                        .is_ok_and(|s| s == b"Widget")
                })
            {
                tab.insert(id, n);
                page_of.insert(id, i);
                n += 1;
            }
        }
    }
    let mut out = vec![];
    for (name, id) in names(&doc) {
        let Some(kind) = kind_of(&doc, id) else {
            continue;
        };
        let mut widgets = vec![];
        for w in widgets_of(&doc, id) {
            let (Some(&page), Ok(d)) = (page_of.get(&w), doc.get_dictionary(w)) else {
                continue;
            };
            let Ok(r) = rect_of(d) else { continue };
            widgets.push(DesignWidget {
                id: w,
                page,
                rect: content::displayed_rect(r, geometry[page].1),
                export: on_states(&doc, w).into_iter().next().unwrap_or_default(),
                tab: tab[&w],
            });
        }
        if widgets.is_empty() {
            continue;
        }
        widgets.sort_by_key(|w| (w.page, w.tab));
        let properties = read_properties(&doc, id, widgets[0].id, kind);
        out.push(Designed {
            id,
            name,
            kind,
            properties,
            value: value_of(&doc, id),
            signed: kind == FieldType::Signature && inherited(&doc, id, b"V").is_some(),
            widgets,
        });
    }
    out.sort_by_key(|f| (f.widgets[0].page, f.widgets[0].tab));
    Ok(out)
}
/// The designer's view of a field's settings; `widget` picks a radio button.
fn read_properties(
    doc: &Document,
    field: ObjectId,
    widget: ObjectId,
    kind: FieldType,
) -> Properties {
    let ff = flags(doc, field);
    let look = look(doc, field, widget);
    let d = doc.get_dictionary(field).ok();
    let wd = doc.get_dictionary(widget).ok();
    let mk = wd.and_then(|w| dict_of(doc, w, b"MK"));
    let value = value_of(doc, field);
    let export = on_states(doc, widget)
        .into_iter()
        .next()
        .unwrap_or_default();
    let default = inherited(doc, field, b"DV")
        .as_ref()
        .map(text_of)
        .unwrap_or_default();
    Properties {
        name: d
            .and_then(|d| d.get(b"T").ok())
            .map(text_of)
            .unwrap_or_default(),
        tooltip: d
            .and_then(|d| d.get(b"TU").ok())
            .map(text_of)
            .unwrap_or_default(),
        required: ff & REQUIRED != 0,
        read_only: ff & READ_ONLY != 0,
        font_size: look.size,
        text_color: look.color,
        border: look.border,
        background: look.background,
        align: match look.align {
            1 => Align::Center,
            2 => Align::Right,
            _ => Align::Left,
        },
        multiline: ff & MULTILINE != 0,
        max_length: inherited(doc, field, b"MaxLen")
            .and_then(|o| o.as_i64().ok())
            .filter(|v| *v > 0)
            .map(|v| v as usize),
        default_value: match kind {
            FieldType::Checkbox | FieldType::Radio => String::new(),
            _ => default,
        },
        checked: matches!(kind, FieldType::Checkbox | FieldType::Radio)
            && !export.is_empty()
            && value == export,
        export_value: export,
        check_style: mk
            .and_then(|m| m.get(b"CA").ok())
            .map(|c| CheckStyle::from_character(&text_of(c)))
            .unwrap_or(if kind == FieldType::Radio {
                CheckStyle::Circle
            } else {
                CheckStyle::Check
            }),
        options: options_of(doc, field).into_iter().map(|(_, l)| l).collect(),
        editable: ff & EDIT != 0,
        caption: match kind {
            FieldType::Button => mk
                .and_then(|m| m.get(b"CA").ok())
                .map(text_of)
                .unwrap_or_default(),
            _ => String::new(),
        },
        action: wd
            .and_then(|w| w.get(b"A").ok())
            .map(|a| resolve(doc, a))
            .and_then(|a| a.as_dict().ok())
            .map(
                |a| match a.get(b"S").and_then(Object::as_name).unwrap_or_default() {
                    b"ResetForm" => Action::ResetForm,
                    b"Named" => Action::Print,
                    b"URI" => Action::OpenLink(a.get(b"URI").map(text_of).unwrap_or_default()),
                    _ => Action::None,
                },
            )
            .unwrap_or_default(),
        date_format: date_format(doc, field).unwrap_or_else(|| DATE_FORMATS[0].into()),
    }
}

fn js_action(js: &str) -> Dictionary {
    dictionary! { "S" => "JavaScript", "JS" => Object::string_literal(js) }
}
/// Write a field's settings; `widget` is the radio button whose export value changes.
fn write_properties(
    doc: &mut Document,
    field: ObjectId,
    widget: ObjectId,
    kind: FieldType,
    p: &Properties,
) -> Result<()> {
    check_name(&p.name)?;
    ensure!(
        p.tooltip.chars().count() <= 500
            && (0. ..=144.).contains(&p.font_size)
            && p.options.len() <= 500
            && p.options.iter().all(|o| o.chars().count() <= 500)
            && p.caption.chars().count() <= 200
            && p.default_value.chars().count() <= 32_768,
        "A field setting is too long"
    );
    let colors = p
        .border
        .iter()
        .chain(p.background.iter())
        .chain([&p.text_color])
        .flatten();
    ensure!(
        colors.into_iter().all(|v| (0. ..=1.).contains(v)),
        "Invalid color"
    );
    let old_export = on_states(doc, widget).into_iter().next();
    let old_value = value_of(doc, field);
    let old_default = inherited(doc, field, b"DV")
        .as_ref()
        .map(text_of)
        .unwrap_or_default();
    let mut ff = flags(doc, field) & !(READ_ONLY | REQUIRED | MULTILINE | EDIT);
    if p.read_only {
        ff |= READ_ONLY;
    }
    if p.required {
        ff |= REQUIRED;
    }
    if kind == FieldType::Text && p.multiline {
        ff |= MULTILINE;
    }
    if kind == FieldType::Dropdown && p.editable {
        ff |= EDIT;
    }
    let font = if matches!(kind, FieldType::Checkbox | FieldType::Radio) {
        "ZaDb"
    } else {
        "Helv"
    };
    let da = Object::string_literal(format!(
        "/{font} {} Tf {} rg",
        number(p.font_size),
        rgb(p.text_color)
    ));
    let options: Vec<String> = p
        .options
        .iter()
        .map(|o| o.trim().to_owned())
        .filter(|o| !o.is_empty())
        .collect();
    if matches!(kind, FieldType::Dropdown | FieldType::ListBox) {
        ensure!(!options.is_empty(), "Add at least one choice");
        ensure!(
            options.iter().collect::<HashSet<_>>().len() == options.len(),
            "Each choice must be different"
        );
        ensure!(
            p.default_value.is_empty()
                || options.contains(&p.default_value)
                || kind == FieldType::Dropdown && p.editable,
            "The default must be one of the choices"
        );
    }
    if kind == FieldType::Date && !p.default_value.is_empty() {
        crate::form_data::parse_date(&p.date_format, &p.default_value)
            .context("The default date does not match the date format")?;
    }
    if let Some(max) = p.max_length {
        ensure!(
            (1..=32_768).contains(&max) && p.default_value.chars().count() <= max,
            "The default text is longer than the maximum length"
        );
    }
    let export = p.export_value.trim().to_owned();
    if matches!(kind, FieldType::Checkbox | FieldType::Radio) {
        ensure!(
            !export.is_empty()
                && export != "Off"
                && export.len() <= 100
                && export
                    .bytes()
                    .all(|b| b.is_ascii_graphic() && !b"/()<>[]{}%#".contains(&b)),
            "Use an export value of letters, digits and simple punctuation, other than Off"
        );
    }
    let d = doc.get_dictionary_mut(field)?;
    d.set("T", pdf_text(p.name.trim()));
    if p.tooltip.trim().is_empty() {
        d.remove(b"TU");
    } else {
        d.set("TU", pdf_text(p.tooltip.trim()));
    }
    d.set("Ff", ff);
    d.set("DA", da.clone());
    d.set(
        "Q",
        match p.align {
            Align::Left => 0,
            Align::Center => 1,
            Align::Right => 2,
        },
    );
    match (kind, p.max_length) {
        (FieldType::Text, Some(max)) => d.set("MaxLen", max as i64),
        _ => {
            d.remove(b"MaxLen");
        }
    }
    match kind {
        FieldType::Text | FieldType::Date => {
            // A new default also becomes the value; clearing it clears a value that was the default.
            if p.default_value.is_empty() {
                d.remove(b"DV");
                if old_value == old_default {
                    d.remove(b"V");
                }
            } else {
                let default = pdf_text(&p.default_value);
                d.set("DV", default.clone());
                if p.default_value != old_default || old_value.is_empty() {
                    d.set("V", default);
                }
            }
        }
        FieldType::Dropdown | FieldType::ListBox => {
            d.set(
                "Opt",
                options.iter().map(|o| pdf_text(o)).collect::<Vec<_>>(),
            );
            if p.default_value.is_empty() {
                d.remove(b"DV");
            } else {
                d.set("DV", pdf_text(&p.default_value));
                d.set("V", pdf_text(&p.default_value));
            }
            let keep = options.contains(&old_value) || kind == FieldType::Dropdown && p.editable;
            if !keep && p.default_value.is_empty() {
                d.remove(b"V");
                d.remove(b"I");
            }
        }
        _ => {}
    }
    if kind == FieldType::Date {
        let format = p.date_format.trim();
        ensure!(
            !format.is_empty() && format.len() <= 40 && !format.contains(['"', '\\']),
            "Choose a date format"
        );
        d.set(
            "AA",
            dictionary! {
                "F" => js_action(&format!("AFDate_FormatEx(\"{format}\");")),
                "K" => js_action(&format!("AFDate_KeystrokeEx(\"{format}\");")),
            },
        );
    }
    // Check boxes and radio buttons: the export value names the on appearance.
    if matches!(kind, FieldType::Checkbox | FieldType::Radio) {
        let old = old_export.unwrap_or_default();
        let target = if kind == FieldType::Radio {
            widget
        } else {
            field
        };
        if kind == FieldType::Radio {
            let siblings: Vec<String> = widgets_of(doc, field)
                .into_iter()
                .filter(|w| *w != widget)
                .flat_map(|w| on_states(doc, w))
                .collect();
            ensure!(
                !siblings.contains(&export),
                "Another button in this group already uses the export value “{export}”"
            );
        }
        let placeholder = |doc: &mut Document, w: ObjectId, name: &str| -> Result<()> {
            let mut n = Dictionary::new();
            n.set(name.as_bytes(), Object::Null);
            n.set("Off", Object::Null);
            doc.get_dictionary_mut(w)?
                .set("AP", dictionary! { "N" => n });
            Ok(())
        };
        for w in widgets_of(doc, field) {
            if kind == FieldType::Checkbox || w == target {
                placeholder(doc, w, &export)?;
            }
        }
        let d = doc.get_dictionary_mut(field)?;
        let selected = old_value == old && !old.is_empty();
        if p.checked {
            d.set("V", Object::Name(export.as_bytes().to_vec()));
            d.set("DV", Object::Name(export.as_bytes().to_vec()));
        } else {
            if selected || old_value.is_empty() {
                d.set("V", Object::Name(b"Off".to_vec()));
            }
            if d.get(b"DV").ok().map(text_of).as_deref() == Some(old.as_str()) {
                d.set("DV", Object::Name(b"Off".to_vec()));
            }
        }
    }
    // Widget-level look: colors, border, caption or check style, action.
    let widgets = widgets_of(doc, field);
    for w in &widgets {
        let turned = rotation(doc, *w);
        let wd = doc.get_dictionary_mut(*w)?;
        let mut mk = Dictionary::new();
        if turned != 0 {
            mk.set("R", turned);
        }
        if let Some(bc) = p.border {
            mk.set("BC", color_array(bc));
        }
        if let Some(bg) = p.background {
            mk.set("BG", color_array(bg));
        }
        match kind {
            FieldType::Checkbox | FieldType::Radio => {
                mk.set("CA", Object::string_literal(p.check_style.character()))
            }
            FieldType::Button => mk.set("CA", pdf_text(&p.caption)),
            _ => {}
        }
        if kind == FieldType::Image {
            mk.set("TP", 1);
            if let Ok(icon) = wd
                .get(b"MK")
                .and_then(Object::as_dict)
                .and_then(|m| m.get(b"I"))
            {
                mk.set("I", icon.clone());
            }
        }
        wd.set("MK", mk);
        wd.set("DA", da.clone());
        wd.set("BS", dictionary! { "W" => 1, "S" => "S" });
        if kind == FieldType::Button {
            match &p.action {
                Action::None => {
                    wd.remove(b"A");
                }
                Action::ResetForm => wd.set("A", dictionary! { "S" => "ResetForm" }),
                Action::Print => wd.set("A", dictionary! { "S" => "Named", "N" => "Print" }),
                Action::OpenLink(url) => {
                    let url = url.trim();
                    ensure!(
                        (url.starts_with("https://")
                            || url.starts_with("http://")
                            || url.starts_with("mailto:"))
                            && url.len() <= 2000
                            && url.is_ascii()
                            && !url.contains(char::is_whitespace),
                        "Enter a web address starting with https://, http:// or mailto:"
                    );
                    wd.set(
                        "A",
                        dictionary! { "S" => "URI", "URI" => Object::string_literal(url) },
                    );
                }
            }
        }
    }
    if kind == FieldType::Image {
        let d = doc.get_dictionary_mut(field)?;
        d.set("MyPDFKind", "Image");
        // Lets people choose a picture in Adobe Acrobat; MyPDF never runs scripts.
        let target = widgets.first().copied().unwrap_or(field);
        doc.get_dictionary_mut(target)?
            .set("A", js_action("event.target.buttonImportIcon();"));
    }
    if kind == FieldType::Radio {
        // Radio buttons in a group switch each other off; the group keeps one choice.
        doc.get_dictionary_mut(field)?
            .set("Ff", ff | RADIO | NO_TOGGLE_OFF);
    }
    Ok(())
}

/// Add a field of `kind` in a displayed rectangle on `page` (from 0). A radio
/// button whose name matches an existing group joins that group.
/// Returns the new bytes and the field (for a radio button, its group).
pub fn add(
    bytes: &[u8],
    page: usize,
    rect: [f32; 4],
    kind: FieldType,
    properties: &Properties,
) -> Result<(Vec<u8>, ObjectId)> {
    let (bytes, ids) = add_many(bytes, &[(page, rect, kind, properties.clone())])?;
    Ok((bytes, ids[0]))
}

/// A field to add: page (from 0), displayed rectangle, kind and settings.
pub type NewField = (usize, [f32; 4], FieldType, Properties);
/// Add several fields in one pass, as a table or detection does.
pub fn add_many(bytes: &[u8], fields: &[NewField]) -> Result<(Vec<u8>, Vec<ObjectId>)> {
    let geometry = crate::annotations::page_geometry(bytes)?;
    let mut doc = content::editable(bytes)?;
    let mut ids = vec![];
    for (page, rect, kind, properties) in fields {
        let (_, inverse) = geometry.get(*page).context("Page not found")?;
        let m = page_transform(*inverse)?;
        ids.push(add_in(&mut doc, m, *page, *rect, *kind, properties)?);
    }
    Ok((content::serialize(&mut doc)?, ids))
}
/// Add one field to a loaded document; `m` maps the page's displayed coordinates to PDF.
pub(crate) fn add_in(
    doc: &mut Document,
    m: Affine,
    page: usize,
    rect: [f32; 4],
    kind: FieldType,
    properties: &Properties,
) -> Result<ObjectId> {
    check_rect(rect)?;
    let page_id = *doc
        .get_pages()
        .get(&(page as u32 + 1))
        .context("Page not found")?;
    let form = acroform(doc)?;
    let pdf_rect = to_pdf(m, rect);
    check_name(&properties.name)?;
    let existing = names(doc).get(properties.name.trim()).copied();
    let group = existing
        .filter(|id| kind == FieldType::Radio && kind_of(doc, *id) == Some(FieldType::Radio));
    ensure!(
        existing.is_none() || group.is_some(),
        "A field named “{}” already exists. Choose another name.",
        properties.name.trim()
    );
    let mut widget = dictionary! {
        "Type" => "Annot",
        "Subtype" => "Widget",
        "Rect" => pdf_rect.iter().map(|v| Object::Real(*v)).collect::<Vec<_>>(),
        "P" => page_id,
        "F" => 4,
    };
    // On a landscape page that is a turned portrait page, the field turns with it.
    let turned = page_rotation(doc, page_id);
    if turned != 0 {
        widget.set("MK", dictionary! { "R" => turned });
    }
    let field = match kind {
        FieldType::Radio => {
            let group = match group {
                Some(group) => group,
                None => {
                    let group = doc.add_object(dictionary! {
                        "FT" => "Btn",
                        "Ff" => RADIO | NO_TOGGLE_OFF,
                        "T" => pdf_text(properties.name.trim()),
                        "V" => "Off",
                        "Kids" => Vec::<Object>::new(),
                    });
                    let mut fields = form_fields(doc, form);
                    fields.push(group.into());
                    doc.get_dictionary_mut(form)?.set("Fields", fields);
                    group
                }
            };
            widget.set("Parent", group);
            let id = doc.add_object(widget);
            let mut kids = doc
                .get_dictionary(group)?
                .get(b"Kids")
                .ok()
                .map(|o| resolve(doc, o).clone())
                .and_then(|o| o.as_array().ok().cloned())
                .unwrap_or_default();
            kids.push(id.into());
            doc.get_dictionary_mut(group)?.set("Kids", kids);
            let mut list = annots(doc, page_id);
            list.push(id.into());
            set_annots(doc, page_id, list)?;
            // A new button in an existing group gets the next free export value.
            let mut properties = properties.clone();
            let taken: HashSet<String> = widgets_of(doc, group)
                .into_iter()
                .flat_map(|w| on_states(doc, w))
                .collect();
            if taken.contains(properties.export_value.trim()) {
                properties.export_value = (1..)
                    .map(|n| format!("Choice{n}"))
                    .find(|v| !taken.contains(v))
                    .unwrap();
            }
            write_properties(doc, group, id, kind, &properties)?;
            refresh(doc, group, false)?;
            return Ok(group);
        }
        _ => {
            let (ft, ff) = match kind {
                FieldType::Text | FieldType::Date => ("Tx", 0),
                FieldType::Checkbox => ("Btn", 0),
                FieldType::Dropdown => ("Ch", COMBO),
                FieldType::ListBox => ("Ch", 0),
                FieldType::Button | FieldType::Image => ("Btn", PUSH),
                FieldType::Signature => ("Sig", 0),
                FieldType::Radio => unreachable!(),
            };
            widget.set("FT", ft);
            widget.set("Ff", ff);
            widget.set("T", pdf_text(properties.name.trim()));
            let id = doc.add_object(widget);
            let mut fields = form_fields(doc, form);
            fields.push(id.into());
            doc.get_dictionary_mut(form)?.set("Fields", fields);
            let mut list = annots(doc, page_id);
            list.push(id.into());
            set_annots(doc, page_id, list)?;
            id
        }
    };
    if kind == FieldType::Signature {
        let form = doc.get_dictionary_mut(form)?;
        let sig_flags = form.get(b"SigFlags").and_then(Object::as_i64).unwrap_or(0);
        form.set("SigFlags", sig_flags | 1);
    }
    write_properties(doc, field, field, kind, properties)?;
    refresh(doc, field, false)?;
    Ok(field)
}

/// Change a field's settings. `widget` picks the radio button whose export
/// value and default choice change.
pub fn update(
    bytes: &[u8],
    field: ObjectId,
    widget: ObjectId,
    properties: &Properties,
) -> Result<Vec<u8>> {
    let mut doc = content::editable(bytes)?;
    let kind = kind_of(&doc, field).context("The field no longer exists")?;
    ensure!(
        !(kind == FieldType::Signature && inherited(&doc, field, b"V").is_some()),
        "A signed signature field cannot be changed"
    );
    let full = names(&doc);
    let current = full
        .iter()
        .find(|(_, id)| **id == field)
        .map(|(n, _)| n.clone())
        .unwrap_or_default();
    let prefix = current
        .rsplit_once('.')
        .map(|(p, _)| format!("{p}."))
        .unwrap_or_default();
    let renamed = format!("{prefix}{}", properties.name.trim());
    ensure!(
        renamed == current || !full.contains_key(&renamed),
        "A field named “{}” already exists. Choose another name.",
        properties.name.trim()
    );
    acroform(&mut doc)?;
    write_properties(&mut doc, field, widget, kind, properties)?;
    refresh(&mut doc, field, false)?;
    content::serialize(&mut doc)
}

/// Move or resize one widget to a displayed rectangle on its page.
pub fn place(bytes: &[u8], widget: ObjectId, rect: [f32; 4]) -> Result<Vec<u8>> {
    check_rect(rect)?;
    let doc = Document::load_mem(bytes)?;
    let (_, on) = page_map(&doc);
    let page = *on.get(&widget).context("The field is not on a page")?;
    let geometry = crate::annotations::page_geometry(bytes)?;
    let m = page_transform(geometry.get(page).context("Page not found")?.1)?;
    let mut doc = content::editable(bytes)?;
    let field = field_of(&doc, widget);
    ensure!(
        !(kind_of(&doc, field) == Some(FieldType::Signature)
            && inherited(&doc, field, b"V").is_some()),
        "A signed signature field cannot be moved"
    );
    let r = to_pdf(m, rect);
    doc.get_dictionary_mut(widget)?.set(
        "Rect",
        r.iter().map(|v| Object::Real(*v)).collect::<Vec<_>>(),
    );
    refresh(&mut doc, field, false)?;
    content::serialize(&mut doc)
}

/// Delete a field, or only `widget` when it is one of several radio buttons.
pub fn remove(bytes: &[u8], field: ObjectId, widget: Option<ObjectId>) -> Result<Vec<u8>> {
    let mut doc = content::editable(bytes)?;
    ensure!(
        !(kind_of(&doc, field) == Some(FieldType::Signature)
            && inherited(&doc, field, b"V").is_some()),
        "A signed signature field cannot be deleted"
    );
    let all = widgets_of(&doc, field);
    let gone: Vec<ObjectId> = match widget {
        Some(w) if all.len() > 1 && all.contains(&w) => vec![w],
        _ => all.clone(),
    };
    let (pages, _) = page_map(&doc);
    for page in pages {
        let list = annots(&doc, page);
        let kept: Vec<Object> = list
            .iter()
            .filter(|a| a.as_reference().is_ok_and(|id| !gone.contains(&id)))
            .cloned()
            .collect();
        if kept.len() != list.len() {
            set_annots(&mut doc, page, kept)?;
        }
    }
    if gone.len() < all.len() {
        // One radio button of a group.
        let kids: Vec<Object> = doc
            .get_dictionary(field)?
            .get(b"Kids")
            .ok()
            .map(|o| resolve(&doc, o).clone())
            .and_then(|o| o.as_array().ok().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter(|k| k.as_reference().is_ok_and(|id| !gone.contains(&id)))
            .collect();
        doc.get_dictionary_mut(field)?.set("Kids", kids);
        let value = value_of(&doc, field);
        let remaining: Vec<String> = widgets_of(&doc, field)
            .into_iter()
            .flat_map(|w| on_states(&doc, w))
            .collect();
        if !remaining.contains(&value) {
            doc.get_dictionary_mut(field)?
                .set("V", Object::Name(b"Off".to_vec()));
        }
        return content::serialize(&mut doc);
    }
    detach(&mut doc, field)?;
    content::serialize(&mut doc)
}
/// Remove a field from its parent or the AcroForm, removing parents left empty.
fn detach(doc: &mut Document, field: ObjectId) -> Result<()> {
    let parent = doc
        .get_dictionary(field)?
        .get(b"Parent")
        .and_then(Object::as_reference)
        .ok();
    match parent {
        Some(parent) => {
            let kids: Vec<Object> = doc
                .get_dictionary(parent)?
                .get(b"Kids")
                .ok()
                .map(|o| resolve(doc, o).clone())
                .and_then(|o| o.as_array().ok().cloned())
                .unwrap_or_default()
                .into_iter()
                .filter(|k| k.as_reference().ok() != Some(field))
                .collect();
            let empty = kids.is_empty();
            doc.get_dictionary_mut(parent)?.set("Kids", kids);
            if empty {
                detach(doc, parent)?;
            }
        }
        None => {
            if let Some(form) = doc
                .catalog()?
                .get(b"AcroForm")
                .ok()
                .and_then(|o| o.as_reference().ok())
            {
                let fields: Vec<Object> = form_fields(doc, form)
                    .into_iter()
                    .filter(|k| k.as_reference().ok() != Some(field))
                    .collect();
                doc.get_dictionary_mut(form)?.set("Fields", fields);
            }
        }
    }
    Ok(())
}

/// Reset every fillable field to its default value. Returns the number of
/// fields that changed.
pub fn clear(bytes: &[u8]) -> Result<(Vec<u8>, usize)> {
    let mut doc = content::editable(bytes)?;
    let mut changed = 0;
    for (_, field) in names(&doc) {
        let Some(kind) = kind_of(&doc, field) else {
            continue;
        };
        if !kind.fillable() || flags(&doc, field) & READ_ONLY != 0 {
            continue;
        }
        let before = value_of(&doc, field);
        let default = inherited(&doc, field, b"DV");
        let d = doc.get_dictionary_mut(field)?;
        match (&default, kind) {
            (Some(dv), _) => d.set("V", dv.clone()),
            (None, FieldType::Checkbox | FieldType::Radio) => {
                d.set("V", Object::Name(b"Off".to_vec()))
            }
            (None, _) => {
                d.remove(b"V");
            }
        }
        d.remove(b"I");
        d.remove(b"RV");
        refresh(&mut doc, field, false)?;
        if value_of(&doc, field) != before {
            changed += 1;
        }
    }
    Ok((content::serialize(&mut doc)?, changed))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabOrder {
    /// Left to right, then the next row down.
    Rows,
    /// Top to bottom, then the next column to the right.
    Columns,
}
/// Sort each page's fields into rows or columns, for viewers that follow the
/// page's annotation order and those that read its /Tabs setting.
pub fn set_tab_order(bytes: &[u8], order: TabOrder) -> Result<Vec<u8>> {
    let geometry = crate::annotations::page_geometry(bytes)?;
    let mut doc = content::editable(bytes)?;
    let (pages, _) = page_map(&doc);
    for (i, page) in pages.iter().enumerate() {
        let list = annots(&doc, *page);
        let mut widgets: Vec<(ObjectId, [f32; 4])> = list
            .iter()
            .filter_map(|a| a.as_reference().ok())
            .filter_map(|id| {
                let d = doc.get_dictionary(id).ok()?;
                d.get(b"Subtype")
                    .and_then(Object::as_name)
                    .is_ok_and(|s| s == b"Widget")
                    .then_some(())?;
                let r = rect_of(d).ok()?;
                Some((id, content::displayed_rect(r, geometry[i].1)))
            })
            .collect();
        if widgets.is_empty() {
            continue;
        }
        sort_tab_order(&mut widgets, order);
        let mut sorted = widgets.into_iter().map(|(id, _)| id);
        let reordered: Vec<Object> = list
            .iter()
            .map(|a| match a.as_reference() {
                Ok(id)
                    if doc.get_dictionary(id).is_ok_and(|d| {
                        d.get(b"Subtype")
                            .and_then(Object::as_name)
                            .is_ok_and(|s| s == b"Widget")
                    }) && rect_of(doc.get_dictionary(id).unwrap()).is_ok() =>
                {
                    sorted.next().map(Object::Reference).unwrap_or(a.clone())
                }
                _ => a.clone(),
            })
            .collect();
        set_annots(&mut doc, *page, reordered)?;
        doc.get_dictionary_mut(*page)?.set(
            "Tabs",
            match order {
                TabOrder::Rows => "R",
                TabOrder::Columns => "C",
            },
        );
    }
    content::serialize(&mut doc)
}
/// Rows: fields whose vertical spans overlap by half share a row. Columns likewise.
fn sort_tab_order(widgets: &mut [(ObjectId, [f32; 4])], order: TabOrder) {
    // Work in (along, across) coordinates: rows read along x, columns along y.
    let key = |r: [f32; 4]| match order {
        TabOrder::Rows => (r[0], r[1], r[2], r[3]),
        TabOrder::Columns => (r[1], r[0], r[3], r[2]),
    };
    widgets.sort_by(|a, b| {
        let (_, a0, _, _) = key(a.1);
        let (_, b0, _, _) = key(b.1);
        a0.total_cmp(&b0)
    });
    let mut lines: Vec<Vec<(ObjectId, [f32; 4])>> = vec![];
    for w in widgets.iter() {
        let (_, start, _, end) = key(w.1);
        let joins = lines.last().is_some_and(|line| {
            let (_, ls, _, le) = key(line[0].1);
            let overlap = end.min(le) - start.max(ls);
            overlap >= 0.5 * (end - start).min(le - ls)
        });
        if joins {
            lines.last_mut().unwrap().push(*w);
        } else {
            lines.push(vec![*w]);
        }
    }
    let mut out = vec![];
    for mut line in lines {
        line.sort_by(|a, b| key(a.1).0.total_cmp(&key(b.1).0));
        out.extend(line);
    }
    widgets.copy_from_slice(&out);
}

/// Draw every field into its page and remove the fields. Empty signature
/// fields stay, so the flattened PDF can still be signed there.
/// Returns the new bytes and the number of widgets flattened.
pub fn flatten(bytes: &[u8]) -> Result<(Vec<u8>, usize)> {
    let mut doc = content::editable(bytes)?;
    let (pages, _) = page_map(&doc);
    let mut count = 0;
    let mut flattened = HashSet::new();
    for page in pages {
        let list = annots(&doc, page);
        let mut drawn = vec![];
        let mut kept = vec![];
        for a in &list {
            let Ok(id) = a.as_reference() else {
                kept.push(a.clone());
                continue;
            };
            let Ok(d) = doc.get_dictionary(id) else {
                kept.push(a.clone());
                continue;
            };
            let widget = d
                .get(b"Subtype")
                .and_then(Object::as_name)
                .is_ok_and(|s| s == b"Widget");
            let field = field_of(&doc, id);
            if !widget || kind_of(&doc, field) == Some(FieldType::Signature) {
                kept.push(a.clone());
                continue;
            }
            flattened.insert(id);
            count += 1;
            let hidden = d.get(b"F").and_then(Object::as_i64).unwrap_or(0) & (2 | 32) != 0;
            if hidden {
                continue;
            }
            let Ok(rect) = rect_of(d) else { continue };
            let state = d
                .get(b"AS")
                .ok()
                .and_then(|s| s.as_name().ok())
                .map(<[u8]>::to_vec);
            let normal = dict_of(&doc, d, b"AP")
                .and_then(|ap| ap.get(b"N").ok())
                .and_then(|n| match resolve(&doc, n) {
                    Object::Dictionary(states) => state
                        .as_ref()
                        .and_then(|s| states.get(s).ok())
                        .and_then(|o| o.as_reference().ok()),
                    _ => n.as_reference().ok(),
                });
            if let Some(stream) = normal {
                drawn.push((stream, rect));
            }
        }
        if drawn.is_empty() && kept.len() == list.len() {
            continue;
        }
        if !drawn.is_empty() {
            draw_into_page(&mut doc, page, &drawn)?;
        }
        set_annots(&mut doc, page, kept)?;
    }
    // Remove the flattened fields from the form; drop the form when nothing is left.
    let fields: Vec<ObjectId> = names(&doc).into_values().collect();
    for field in fields {
        if widgets_of(&doc, field)
            .iter()
            .all(|w| flattened.contains(w))
            && kind_of(&doc, field) != Some(FieldType::Signature)
        {
            detach(&mut doc, field)?;
        }
    }
    if let Some(form) = doc
        .catalog()?
        .get(b"AcroForm")
        .ok()
        .and_then(|o| o.as_reference().ok())
        && form_fields(&doc, form).is_empty()
    {
        doc.catalog_mut()?.remove(b"AcroForm");
    }
    Ok((content::serialize(&mut doc)?, count))
}
/// Paint appearance streams into a page at their widget rectangles.
pub(crate) fn draw_into_page(
    doc: &mut Document,
    page: ObjectId,
    drawn: &[(ObjectId, [f32; 4])],
) -> Result<()> {
    let mut resources = content::resources(doc, page)?;
    let mut xobjects = resources
        .get(b"XObject")
        .ok()
        .map(|o| resolve(doc, o).clone())
        .and_then(|o| o.as_dict().ok().cloned())
        .unwrap_or_default();
    let mut body = String::from("Q\n");
    let mut n = 0;
    for (stream, rect) in drawn {
        let Ok(s) = doc.get_object(*stream).and_then(Object::as_stream) else {
            continue;
        };
        let bbox: Vec<f32> = s
            .dict
            .get(b"BBox")
            .and_then(Object::as_array)
            .map(|a| a.iter().filter_map(|o| o.as_float().ok()).collect())
            .unwrap_or_default();
        if bbox.len() != 4 {
            continue;
        }
        let matrix: Vec<f64> = s
            .dict
            .get(b"Matrix")
            .and_then(Object::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|o| o.as_float().ok().map(f64::from))
                    .collect()
            })
            .unwrap_or_default();
        let matrix = if matrix.len() == 6 {
            Affine::new([
                matrix[0], matrix[1], matrix[2], matrix[3], matrix[4], matrix[5],
            ])
        } else {
            Affine::IDENTITY
        };
        // The appearance's box, after its own matrix, is mapped onto the widget rectangle.
        let b = matrix.transform_rect_bbox(hayro::kurbo::Rect::new(
            bbox[0] as f64,
            bbox[1] as f64,
            bbox[2] as f64,
            bbox[3] as f64,
        ));
        if b.width() <= 0. || b.height() <= 0. {
            continue;
        }
        let sx = (rect[2] - rect[0]) as f64 / b.width();
        let sy = (rect[3] - rect[1]) as f64 / b.height();
        let name = loop {
            let name = format!("MyPDFField{n}");
            n += 1;
            if !xobjects.has(name.as_bytes()) {
                break name;
            }
        };
        let s = doc.get_object_mut(*stream)?.as_stream_mut()?;
        s.dict.set("Type", "XObject");
        s.dict.set("Subtype", "Form");
        xobjects.set(name.as_str(), *stream);
        body.push_str(&format!(
            "q {} 0 0 {} {} {} cm /{name} Do Q\n",
            sx,
            sy,
            rect[0] as f64 - b.x0 * sx,
            rect[1] as f64 - b.y0 * sy
        ));
    }
    resources.set("XObject", xobjects);
    // Earlier content is wrapped so its graphics state cannot leak into the fields.
    let open = doc.add_object(Stream::new(Dictionary::new(), b"q\n".to_vec()));
    let close = doc.add_object(Stream::new(Dictionary::new(), body.into_bytes()));
    let contents = doc.get_dictionary(page)?.get(b"Contents").ok().cloned();
    let mut list = vec![Object::Reference(open)];
    match contents.map(|c| resolve(doc, &c).clone()) {
        Some(Object::Array(a)) => list.extend(a),
        Some(Object::Stream(_)) => {
            list.extend(doc.get_dictionary(page)?.get(b"Contents").ok().cloned())
        }
        _ => {}
    }
    list.push(Object::Reference(close));
    let d = doc.get_dictionary_mut(page)?;
    d.set("Contents", list);
    d.set("Resources", resources);
    Ok(())
}

/// Put a picture into an image field.
pub fn set_image(bytes: &[u8], field: ObjectId, image: &[u8]) -> Result<Vec<u8>> {
    let mut doc = content::editable(bytes)?;
    ensure!(
        kind_of(&doc, field) == Some(FieldType::Image),
        "This is not an image field"
    );
    let decoded = content::decode_image(image)?;
    let (iw, ih) = (decoded.width() as f32, decoded.height() as f32);
    let image_id = content::embed_image(&mut doc, &decoded)?;
    let icon = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), Object::Real(iw), Object::Real(ih)],
            "Resources" => dictionary! { "XObject" => dictionary! { "Img" => image_id } },
        },
        format!("q {iw} 0 0 {ih} 0 0 cm /Img Do Q").into_bytes(),
    ));
    for w in widgets_of(&doc, field) {
        let d = doc.get_dictionary_mut(w)?;
        let mut mk = d
            .get(b"MK")
            .and_then(Object::as_dict)
            .cloned()
            .unwrap_or_default();
        mk.set("I", icon);
        mk.set("TP", 1);
        d.set("MK", mk);
    }
    refresh(&mut doc, field, false)?;
    content::serialize(&mut doc)
}
