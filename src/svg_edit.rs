//! Source-preserving edits for the visual SVG studio. Definitions and unsupported
//! artwork stay intact; each edit targets one XML element, never a text match.
use anyhow::{Context, Result, ensure};
use quick_xml::{Reader, events::Event};
use std::{collections::BTreeMap, ops::Range};

#[derive(Clone, Debug)]
pub struct Element {
    pub tag: String,
    pub attributes: BTreeMap<String, String>,
    pub inherited: BTreeMap<String, String>,
    pub range: Range<usize>,
    opening: Range<usize>,
    parent: usize,
    empty: bool,
    pub text: Option<(Range<usize>, String)>,
}
impl Element {
    pub fn value<'a>(&'a self, key: &str, fallback: &'a str) -> &'a str {
        self.attributes
            .get(key)
            .or_else(|| self.inherited.get(key))
            .map_or(fallback, String::as_str)
    }
    pub fn label(&self) -> String {
        if let Some(part) = self.attributes.get("data-mypdf-border-part") {
            return if part == "inner" {
                "Inner frame"
            } else {
                "Outer frame"
            }
            .into();
        }
        if let Some((_, text)) = &self.text {
            return format!("Text · {}", text.chars().take(28).collect::<String>());
        }
        let name = match self.tag.as_str() {
            "rect" => "Rectangle",
            "ellipse" => "Ellipse",
            "circle" => "Circle",
            "line" => "Line",
            "path" => "Path",
            "polygon" => "Polygon",
            "polyline" => "Polyline",
            "text" => "Text",
            "use" => "Symbol",
            _ => "Object",
        };
        self.attributes
            .get("id")
            .map_or_else(|| name.into(), |id| format!("{name} · {id}"))
    }
}
#[derive(Clone)]
pub struct Scene {
    pub elements: Vec<Element>,
    pub view_box: [f32; 4],
    end: usize,
}
fn attributes(e: &quick_xml::events::BytesStart<'_>) -> Result<BTreeMap<String, String>> {
    let mut attrs = BTreeMap::new();
    for attr in e.attributes() {
        let attr = attr?;
        attrs.insert(
            std::str::from_utf8(attr.key.as_ref())?.into(),
            attr.normalized_value(quick_xml::XmlVersion::Implicit1_0)?
                .into_owned(),
        );
    }
    Ok(attrs)
}
fn effective(attrs: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut out = attrs.clone();
    if let Some(style) = attrs.get("style") {
        for part in style.split(';') {
            if let Some((key, value)) = part.split_once(':') {
                out.insert(key.trim().into(), value.trim().into());
            }
        }
    }
    out
}
const INHERITED: &[&str] = &[
    "fill",
    "stroke",
    "stroke-width",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-dasharray",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "text-anchor",
    "letter-spacing",
    "fill-opacity",
    "stroke-opacity",
];
impl Scene {
    pub fn parse(source: &str) -> Result<Self> {
        ensure!(source.len() <= 256 * 1024, "SVG source exceeds 256 KB");
        let mut reader = Reader::from_str(source);
        // opening offset, inherited properties, hidden definition, element index
        type Frame = (usize, BTreeMap<String, String>, bool, Option<usize>);
        let mut stack: Vec<Frame> = Vec::new();
        let mut elements: Vec<Element> = Vec::new();
        let mut view_box = [0., 0., 240., 120.];
        let mut end = None;
        loop {
            let before = reader.buffer_position() as usize;
            match reader.read_event()? {
                Event::Start(e) | Event::Empty(e) => {
                    ensure!(stack.len() < 64, "SVG nesting is too deep");
                    let after = reader.buffer_position() as usize;
                    let empty = source[before..after].ends_with("/>");
                    let tag = std::str::from_utf8(e.local_name().as_ref())?.to_owned();
                    let attrs = attributes(&e)?;
                    let inherited = stack.last().map(|s| s.1.clone()).unwrap_or_default();
                    let mut resolved = effective(&attrs);
                    if stack.is_empty() && tag == "svg" {
                        if let Some(v) = attrs.get("viewBox") {
                            let numbers: Vec<f32> = v
                                .split(|c: char| c.is_whitespace() || c == ',')
                                .filter(|s| !s.is_empty())
                                .map(str::parse)
                                .collect::<std::result::Result<_, _>>()?;
                            ensure!(numbers.len() == 4, "Invalid SVG viewBox");
                            view_box.copy_from_slice(&numbers);
                        } else {
                            let tree = crate::svg::tree(source, None)?;
                            view_box = [0., 0., tree.size().width(), tree.size().height()];
                        }
                    }
                    let blocked = stack.last().is_some_and(|s| s.2)
                        || matches!(
                            tag.as_str(),
                            "defs" | "clipPath" | "mask" | "symbol" | "pattern" | "marker"
                        );
                    let drawable = !blocked
                        && matches!(
                            tag.as_str(),
                            "rect"
                                | "circle"
                                | "ellipse"
                                | "line"
                                | "path"
                                | "polygon"
                                | "polyline"
                                | "text"
                                | "use"
                        );
                    let index = drawable.then_some(elements.len());
                    if drawable {
                        elements.push(Element {
                            tag,
                            attributes: resolved.clone(),
                            inherited: inherited.clone(),
                            range: before..after,
                            opening: before..after,
                            parent: stack.last().map_or(0, |s| s.0),
                            empty,
                            text: None,
                        });
                        ensure!(
                            elements.len() <= 512,
                            "Visual editing supports up to 512 objects; use SVG source for this artwork"
                        );
                    }
                    resolved.retain(|key, _| INHERITED.contains(&key.as_str()));
                    let mut cascade = inherited;
                    cascade.extend(resolved);
                    if !empty {
                        stack.push((before, cascade, blocked, index));
                    }
                }
                Event::End(e) => {
                    let (_, _, _, index) = stack.pop().context("Unexpected SVG closing tag")?;
                    if let Some(index) = index {
                        let element = &mut elements[index];
                        element.range.end = reader.buffer_position() as usize;
                        let range = element.opening.end..before;
                        if element.tag == "text" && !source[range.clone()].contains('<') {
                            element.text = Some((
                                range.clone(),
                                quick_xml::escape::unescape(&source[range])?.into_owned(),
                            ));
                        }
                    }
                    if stack.is_empty() && e.local_name().as_ref() == b"svg" {
                        end = Some(before);
                    }
                }
                Event::DocType(_) => anyhow::bail!("SVG document types are unsupported"),
                Event::Eof => break,
                _ => {}
            }
        }
        ensure!(
            view_box.iter().all(|v| v.is_finite()) && view_box[2] > 0. && view_box[3] > 0.,
            "Invalid SVG canvas dimensions"
        );
        Ok(Self {
            elements,
            view_box,
            end: end.context("SVG is missing its closing tag")?,
        })
    }
}

/// Override presentation properties, removing only the same declarations from
/// inline CSS (otherwise CSS would silently win over the visual controls).
pub fn update(
    source: &str,
    index: usize,
    changes: &[(String, String)],
    text: Option<&str>,
) -> Result<String> {
    let scene = Scene::parse(source)?;
    let element = scene.elements.get(index).context("Select an object")?;
    let mut reader = Reader::from_str(&source[element.opening.clone()]);
    let event = reader.read_event()?;
    let e = match event {
        Event::Start(e) | Event::Empty(e) => e,
        _ => anyhow::bail!("Invalid SVG element"),
    };
    let mut attrs = attributes(&e)?;
    if let Some(style) = attrs.get_mut("style") {
        *style = style
            .split(';')
            .filter(|part| {
                !changes
                    .iter()
                    .any(|(key, _)| part.split_once(':').is_some_and(|(k, _)| k.trim() == key))
            })
            .collect::<Vec<_>>()
            .join(";");
    }
    for (key, value) in changes {
        ensure!(
            !key.starts_with("on") && !matches!(key.as_str(), "href" | "xlink:href" | "id"),
            "This attribute is not editable here"
        );
        attrs.insert(key.clone(), value.clone());
    }
    let mut opening =
        quick_xml::events::BytesStart::new(std::str::from_utf8(e.name().as_ref())?.to_owned());
    for (key, value) in &attrs {
        opening.push_attribute((key.as_str(), value.as_str()));
    }
    let mut writer = quick_xml::Writer::new(Vec::new());
    writer.write_event(if element.empty {
        Event::Empty(opening)
    } else {
        Event::Start(opening)
    })?;
    let mut out = source.to_owned();
    if let Some(text) = text {
        ensure!(text.chars().count() <= 8000, "Text is too long");
        let (range, _) = element
            .text
            .as_ref()
            .context("This lettering uses paths or nested text; edit it in SVG source")?;
        out.replace_range(range.clone(), &quick_xml::escape::escape(text));
    }
    out.replace_range(
        element.opening.clone(),
        &String::from_utf8(writer.into_inner())?,
    );
    crate::svg::tree(&out, None)?;
    Ok(out)
}

#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    /// Bounds and parent transform in rendered canvas coordinates.
    pub bounds: [f32; 4],
    pub parent: resvg::tiny_skia::Transform,
    pub local_center: [f32; 2],
}
/// Temporary wrapper IDs leave existing IDs/references untouched. They also give
/// us the parent transform, needed for correct dragging inside transformed groups.
pub fn geometry(source: &str, scene: &Scene) -> Result<([f32; 2], Vec<Option<Geometry>>)> {
    let mut annotated = source.to_owned();
    let prefix = format!("mypdf-{}-", crate::identity::fingerprint(source.as_bytes()));
    for (index, element) in scene.elements.iter().enumerate().rev() {
        annotated.insert_str(element.range.end, "</g>");
        annotated.insert_str(element.range.start, &format!("<g id=\"{prefix}{index}\">"));
    }
    // Wrappers can exceed the source byte budget; the original remains bounded.
    let tree = crate::svg::tree(&annotated, None)?;
    let bounds = (0..scene.elements.len())
        .map(|index| {
            let node = tree.node_by_id(&format!("{prefix}{index}"))?;
            let b = node.abs_bounding_box();
            let local = node.bounding_box();
            Some(Geometry {
                bounds: [b.x(), b.y(), b.width(), b.height()],
                parent: node.abs_transform(),
                local_center: [
                    local.x() + local.width() / 2.,
                    local.y() + local.height() / 2.,
                ],
            })
        })
        .collect();
    Ok(([tree.size().width(), tree.size().height()], bounds))
}

pub fn translate(source: &str, index: usize, g: Geometry, dx: f32, dy: f32) -> Result<String> {
    let inverse = g
        .parent
        .invert()
        .context("This object's transform cannot be moved")?;
    let x = inverse.sx * dx + inverse.kx * dy;
    let y = inverse.ky * dx + inverse.sy * dy;
    transform(source, index, &format!("translate({x} {y})"))
}
pub fn transform(source: &str, index: usize, prefix: &str) -> Result<String> {
    let scene = Scene::parse(source)?;
    let element = scene.elements.get(index).context("Select an object")?;
    let original = element
        .attributes
        .get("transform")
        .map_or("", String::as_str);
    update(
        source,
        index,
        &[("transform".into(), format!("{prefix} {original}"))],
        None,
    )
}

pub enum LayerAction {
    Delete,
    Duplicate,
    Forward,
    Backward,
}
pub fn layer(source: &str, index: usize, action: LayerAction) -> Result<(String, usize)> {
    let scene = Scene::parse(source)?;
    let element = scene.elements.get(index).context("Select an object")?;
    let mut out = source.to_owned();
    let selected;
    match action {
        LayerAction::Delete => {
            ensure!(
                scene.elements.len() > 1,
                "Keep at least one object in the artwork"
            );
            out.replace_range(element.range.clone(), "");
            selected = index.saturating_sub(1);
        }
        LayerAction::Duplicate => {
            // Remap every ID in the fragment, including nested text spans.
            let fragment = &source[element.range.clone()];
            let mut reader = Reader::from_str(fragment);
            let mut ids = BTreeMap::new();
            let prefix = format!("copy-{}-", crate::identity::fingerprint(source.as_bytes()));
            loop {
                match reader.read_event()? {
                    Event::Start(e) | Event::Empty(e) => {
                        if let Some(id) = attributes(&e)?.get("id") {
                            ids.insert(id.clone(), format!("{prefix}{id}"));
                        }
                    }
                    Event::Eof => break,
                    _ => {}
                }
            }
            let mut reader = Reader::from_str(fragment);
            let mut writer = quick_xml::Writer::new(Vec::new());
            loop {
                match reader.read_event()? {
                    Event::Start(e) | Event::Empty(e) => {
                        let empty = fragment[..reader.buffer_position() as usize].ends_with("/>");
                        let mut attrs = attributes(&e)?;
                        for (key, value) in &mut attrs {
                            if key == "id" {
                                if let Some(new) = ids.get(value) {
                                    *value = new.clone();
                                }
                            } else {
                                for (old, new) in &ids {
                                    *value = value
                                        .replace(&format!("url(#{old})"), &format!("url(#{new})"));
                                    if matches!(key.as_str(), "href" | "xlink:href")
                                        && value == &format!("#{old}")
                                    {
                                        *value = format!("#{new}");
                                    }
                                }
                            }
                        }
                        let mut start = quick_xml::events::BytesStart::new(
                            std::str::from_utf8(e.name().as_ref())?.to_owned(),
                        );
                        for (k, v) in &attrs {
                            start.push_attribute((k.as_str(), v.as_str()));
                        }
                        writer.write_event(if empty {
                            Event::Empty(start)
                        } else {
                            Event::Start(start)
                        })?;
                    }
                    Event::Eof => break,
                    event => writer.write_event(event)?,
                }
            }
            out.insert_str(element.range.end, &String::from_utf8(writer.into_inner())?);
            selected = index + 1;
        }
        LayerAction::Forward | LayerAction::Backward => {
            let forward = matches!(action, LayerAction::Forward);
            let other = if forward {
                scene
                    .elements
                    .iter()
                    .enumerate()
                    .skip(index + 1)
                    .find(|(_, e)| e.parent == element.parent)
            } else {
                scene
                    .elements
                    .iter()
                    .enumerate()
                    .take(index)
                    .rev()
                    .find(|(_, e)| e.parent == element.parent)
            }
            .context("No more objects in this group in that direction")?;
            let (a, b) = if forward {
                (element, other.1)
            } else {
                (other.1, element)
            };
            let replacement = format!(
                "{}{}{}",
                &source[b.range.clone()],
                &source[a.range.end..b.range.start],
                &source[a.range.clone()]
            );
            out.replace_range(a.range.start..b.range.end, &replacement);
            selected = other.0;
        }
    }
    crate::svg::tree(&out, None)?;
    Ok((out, selected))
}
pub fn add(source: &str, kind: &str) -> Result<(String, usize)> {
    let scene = Scene::parse(source)?;
    let [x, y, w, h] = scene.view_box;
    let (cx, cy) = (x + w / 2., y + h / 2.);
    let (sw, sh) = (w * 0.4, h * 0.4);
    let stroke = (w.min(h) / 80.).max(0.1);
    let content = match kind {
        "Rectangle" => format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{sw}\" height=\"{sh}\" rx=\"0\" fill=\"#dbeafe\" stroke=\"#2563eb\" stroke-width=\"{stroke}\"/>",
            cx - sw / 2.,
            cy - sh / 2.
        ),
        "Ellipse" => format!(
            "<ellipse cx=\"{cx}\" cy=\"{cy}\" rx=\"{}\" ry=\"{}\" fill=\"#dbeafe\" stroke=\"#2563eb\" stroke-width=\"{stroke}\"/>",
            sw / 2.,
            sh / 2.
        ),
        "Line" => format!(
            "<line x1=\"{}\" y1=\"{cy}\" x2=\"{}\" y2=\"{cy}\" stroke=\"#2563eb\" stroke-width=\"{stroke}\"/>",
            cx - sw / 2.,
            cx + sw / 2.
        ),
        "Text" => format!(
            "<text x=\"{cx}\" y=\"{cy}\" text-anchor=\"middle\" font-family=\"Arial\" font-size=\"{}\" fill=\"#1e293b\">Your text</text>",
            h * 0.16
        ),
        "Triangle" => format!(
            "<polygon points=\"{cx},{} {},{} {},{}\" fill=\"#dbeafe\" stroke=\"#2563eb\" stroke-width=\"{stroke}\"/>",
            cy - sh / 2.,
            cx + sw / 2.,
            cy + sh / 2.,
            cx - sw / 2.,
            cy + sh / 2.
        ),
        _ => anyhow::bail!("Unknown shape"),
    };
    let mut out = source.to_owned();
    out.insert_str(scene.end, &content);
    crate::svg::tree(&out, None)?;
    Ok((out, scene.elements.len()))
}

/// Apply a canvas-space scale around a fixed point, including transformed parents.
pub fn scale(
    source: &str,
    index: usize,
    g: Geometry,
    origin: [f32; 2],
    factors: [f32; 2],
) -> Result<String> {
    ensure!(
        factors
            .iter()
            .all(|s| s.is_finite() && s.abs() >= 0.001 && s.abs() <= 1000.),
        "Scale is out of range"
    );
    let [sx, sy] = factors;
    let global = resvg::tiny_skia::Transform::from_row(
        sx,
        0.,
        0.,
        sy,
        origin[0] * (1. - sx),
        origin[1] * (1. - sy),
    );
    let inverse = g
        .parent
        .invert()
        .context("This object's transform cannot be resized")?;
    // pre_concat(A, B) = A * B.
    let local = inverse.pre_concat(global).pre_concat(g.parent);
    transform(
        source,
        index,
        &format!(
            "matrix({} {} {} {} {} {})",
            local.sx, local.ky, local.kx, local.sy, local.tx, local.ty
        ),
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BorderKind {
    #[default]
    Rounded,
    Rectangle,
    Oval,
    DoubleRounded,
    DoubleRectangle,
    Dashed,
}
impl BorderKind {
    pub const ALL: [Self; 6] = [
        Self::Rounded,
        Self::Rectangle,
        Self::Oval,
        Self::DoubleRounded,
        Self::DoubleRectangle,
        Self::Dashed,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Rounded => "Rounded",
            Self::Rectangle => "Square corners",
            Self::Oval => "Oval",
            Self::DoubleRounded => "Double rounded",
            Self::DoubleRectangle => "Double frame",
            Self::Dashed => "Dashed",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Rounded => "rounded",
            Self::Rectangle => "rectangle",
            Self::Oval => "oval",
            Self::DoubleRounded => "double-rounded",
            Self::DoubleRectangle => "double-rectangle",
            Self::Dashed => "dashed",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Border {
    pub kind: BorderKind,
    pub color: String,
    pub width: f32,
    pub inset: f32,
    pub radius: f32,
    pub background: Option<String>,
    pub background_opacity: f32,
}
impl Border {
    pub fn for_scene(scene: &Scene) -> Self {
        let short = scene.view_box[2].min(scene.view_box[3]);
        let color = scene
            .elements
            .iter()
            .find(|e| e.tag == "text")
            .map(|e| e.value("fill", "#2563eb"))
            .filter(|c| c.starts_with('#'))
            .unwrap_or("#2563eb")
            .to_owned();
        Self {
            kind: BorderKind::Rounded,
            color,
            width: short / 32.,
            inset: short / 32.,
            radius: short * 0.18,
            background: None,
            background_opacity: 0.12,
        }
    }
}
/// Locate only borders generated by this editor. Unrelated artwork is never
/// guessed to be a border and removed implicitly.
type BorderGroup = (Range<usize>, BTreeMap<String, String>);
fn border_group(source: &str) -> Result<Option<BorderGroup>> {
    let mut reader = Reader::from_str(source);
    let mut depth = 0usize;
    let mut found = None;
    loop {
        let before = reader.buffer_position() as usize;
        match reader.read_event()? {
            Event::Start(e) => {
                if found.is_none() && e.local_name().as_ref() == b"g" {
                    let attrs = attributes(&e)?;
                    if attrs.contains_key("data-mypdf-border") {
                        found = Some((before, depth, attrs));
                    }
                }
                depth += 1;
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if found.as_ref().is_some_and(|(_, d, _)| *d == depth) {
                    let (start, _, attrs) = found.unwrap();
                    return Ok(Some((start..reader.buffer_position() as usize, attrs)));
                }
            }
            Event::Eof => return Ok(None),
            _ => {}
        }
    }
}
pub fn current_border(source: &str) -> Result<Option<Border>> {
    let Some((_, attrs)) = border_group(source)? else {
        return Ok(None);
    };
    let mut border = Border::for_scene(&Scene::parse(source)?);
    border.kind = BorderKind::ALL
        .into_iter()
        .find(|k| Some(k.key()) == attrs.get("data-mypdf-border").map(String::as_str))
        .unwrap_or_default();
    border.color = attrs.get("stroke").cloned().unwrap_or(border.color);
    for (key, value) in [
        ("stroke-width", &mut border.width),
        ("data-inset", &mut border.inset),
        ("data-radius", &mut border.radius),
        ("data-background-opacity", &mut border.background_opacity),
    ] {
        if let Some(number) = attrs
            .get(key)
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|n| n.is_finite())
        {
            *value = number;
        }
    }
    border.background = attrs
        .get("data-background")
        .filter(|v| v.as_str() != "none")
        .cloned();
    Ok(Some(border))
}
/// Insert the frame at the back of the root canvas, not in a transformed group.
/// Switching templates replaces the managed frame, preserving text and artwork.
pub fn set_border(
    source: &str,
    border: &Border,
    replace_selected: Option<usize>,
) -> Result<String> {
    let scene = Scene::parse(source)?;
    let [x, y, w, h] = scene.view_box;
    ensure!(
        [
            border.width,
            border.inset,
            border.radius,
            border.background_opacity
        ]
        .iter()
        .all(|v| v.is_finite()),
        "Border values must be finite"
    );
    ensure!(
        border.width > 0.
            && border.inset >= 0.
            && border.radius >= 0.
            && (0. ..=1.).contains(&border.background_opacity),
        "Invalid border settings"
    );
    let inset = border.inset + border.width / 2.;
    let fw = w - 2. * inset;
    let fh = h - 2. * inset;
    ensure!(
        fw > 0. && fh > 0.,
        "Reduce padding or thickness to fit this border"
    );
    let double = matches!(
        border.kind,
        BorderKind::DoubleRounded | BorderKind::DoubleRectangle
    );
    let gap = border.width * 2.5;
    ensure!(
        !double || fw > 2. * gap && fh > 2. * gap,
        "Reduce padding or thickness to fit the double border"
    );
    let color = quick_xml::escape::escape(&border.color);
    let background = border.background.as_deref().unwrap_or("none");
    let bg = quick_xml::escape::escape(background);
    let mut frame = format!(
        "<g xmlns=\"http://www.w3.org/2000/svg\" data-mypdf-border=\"{}\" data-inset=\"{}\" data-radius=\"{}\" data-background=\"{bg}\" data-background-opacity=\"{}\" stroke=\"{color}\" stroke-width=\"{}\" fill=\"none\">",
        border.kind.key(),
        border.inset,
        border.radius,
        border.background_opacity,
        border.width
    );
    for inner in 0..if double { 2 } else { 1 } {
        let d = if inner == 1 { gap } else { 0. };
        let (bx, by, bw, bh) = (x + inset + d, y + inset + d, fw - 2. * d, fh - 2. * d);
        let fill = if inner == 0 { bg.as_ref() } else { "none" };
        let common = format!(
            "data-mypdf-border-part=\"{}\" fill=\"{fill}\" fill-opacity=\"{}\"",
            if inner == 0 { "outer" } else { "inner" },
            border.background_opacity
        );
        if border.kind == BorderKind::Oval {
            frame.push_str(&format!(
                "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\" {common}/>",
                bx + bw / 2.,
                by + bh / 2.,
                bw / 2.,
                bh / 2.
            ));
        } else {
            let radius = if matches!(
                border.kind,
                BorderKind::Rectangle | BorderKind::DoubleRectangle
            ) {
                0.
            } else {
                (border.radius - d).max(0.).min(bw.min(bh) / 2.)
            };
            let dash = if border.kind == BorderKind::Dashed {
                format!(
                    " stroke-dasharray=\"{} {}\"",
                    border.width * 3.,
                    border.width * 2.
                )
            } else {
                String::new()
            };
            frame.push_str(&format!("<rect x=\"{bx}\" y=\"{by}\" width=\"{bw}\" height=\"{bh}\" rx=\"{radius}\" {common}{dash}/>"));
        }
    }
    frame.push_str("</g>");
    let mut out = source.to_owned();
    if let Some((range, _)) = border_group(source)? {
        out.replace_range(range, "");
    } else if let Some(index) = replace_selected {
        let selected = scene
            .elements
            .get(index)
            .context("Select a frame to replace")?;
        ensure!(
            matches!(selected.tag.as_str(), "rect" | "ellipse" | "circle"),
            "Only rectangle and oval frames can be replaced"
        );
        out.replace_range(selected.range.clone(), "");
    }
    let mut reader = Reader::from_str(&out);
    let start = loop {
        match reader.read_event()? {
            Event::Start(e) if e.local_name().as_ref() == b"svg" => {
                break reader.buffer_position() as usize;
            }
            Event::Eof => anyhow::bail!("Missing SVG canvas"),
            _ => {}
        }
    };
    out.insert_str(start, &frame);
    crate::svg::tree(&out, None)?;
    Ok(out)
}

pub fn remove_border(source: &str) -> Result<String> {
    let mut out = source.to_owned();
    let (range, _) = border_group(source)?.context("There is no template border to remove")?;
    out.replace_range(range, "");
    crate::svg::tree(&out, None)?;
    Ok(out)
}
