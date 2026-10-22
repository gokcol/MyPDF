//! Standard PDF annotations, revision history, and geometry in displayed-page points.
pub use crate::stroke::StrokeStyle;
use anyhow::{Context, Result, ensure};
use hayro::hayro_interpret::TransformExt;
use hayro::kurbo::{Affine, Point, Rect};
use lopdf::{Document, Object, ObjectId, Stream, dictionary};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub type Box2 = [f32; 4];
pub type Point2 = [f32; 2];
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Markup {
    Highlight,
    Underline,
    Strikeout,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Symbol {
    Check,
    Cross,
    Star,
    Circle,
    Arrow,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Kind {
    Markup {
        kind: Markup,
        quads: Vec<Box2>,
    },
    Ink {
        strokes: Vec<Vec<Point2>>,
    },
    Note {
        text: String,
    },
    Text {
        text: String,
    },
    Rectangle,
    Ellipse,
    Line {
        start: Point2,
        end: Point2,
        arrow: bool,
    },
    DoubleArrow {
        start: Point2,
        end: Point2,
    },
    Polygon {
        points: Vec<Point2>,
        closed: bool,
    },
    Symbol(Symbol),
    Vector {
        polygons: Vec<Vec<Point2>>,
    },
    Stamp {
        text: String,
    },
    Svg {
        source: String,
        tint: bool,
    },
    Image {
        #[serde(with = "image_bytes")]
        png: Arc<Vec<u8>>,
    },
}
impl Kind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Markup {
                kind: Markup::Highlight,
                ..
            } => "Highlight",
            Self::Markup {
                kind: Markup::Underline,
                ..
            } => "Underline",
            Self::Markup {
                kind: Markup::Strikeout,
                ..
            } => "Strikeout",
            Self::Ink { .. } => "Pen / signature",
            Self::Note { .. } => "Note",
            Self::Text { .. } => "Text box",
            Self::Rectangle => "Rectangle",
            Self::Ellipse => "Ellipse",
            Self::DoubleArrow { .. } => "Double arrow",
            Self::Polygon { closed: true, .. } => "Polygon",
            Self::Polygon { .. } => "Polyline",
            Self::Line { arrow: true, .. } => "Arrow",
            Self::Line { .. } => "Line",
            Self::Symbol(_) => "Symbol",
            Self::Vector { .. } => "Vector artwork",
            Self::Stamp { .. } => "Stamp",
            Self::Image { .. } => "Artwork",
            Self::Svg { .. } => "SVG artwork",
        }
    }
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Note { text } | Self::Text { text } | Self::Stamp { text } => Some(text),
            _ => None,
        }
    }
    pub fn text_mut(&mut self) -> Option<&mut String> {
        match self {
            Self::Note { text } | Self::Text { text } | Self::Stamp { text } => Some(text),
            _ => None,
        }
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(text) = self.text() {
            ensure!(
                text.len() <= 8000
                    && !text
                        .chars()
                        .any(|c| c.is_control() && c != '\n' && c != '\t'),
                "Annotation text is too long or contains control characters"
            );
        }
        match self {
            Self::Vector { polygons } => {
                ensure!(
                    polygons.len() <= 512
                        && polygons.iter().all(|p| p.len() >= 3
                            && p.len() <= 256
                            && p.iter()
                                .flatten()
                                .all(|v| v.is_finite() && (-0.5..=1.5).contains(v))),
                    "Invalid vector artwork"
                );
            }
            Self::Ink { strokes } => {
                ensure!(
                    strokes.len() <= 2000 && strokes.iter().map(Vec::len).sum::<usize>() <= 100_000,
                    "Too many pen points"
                );
                ensure!(
                    strokes
                        .iter()
                        .flatten()
                        .flatten()
                        .all(|v| v.is_finite() && (-0.5..=1.5).contains(v)),
                    "Invalid pen geometry"
                );
            }
            Self::Markup { quads, .. } => {
                ensure!(quads.len() <= 10_000, "Too many markup rectangles");
                ensure!(
                    quads.iter().all(|r| r
                        .iter()
                        .all(|v| v.is_finite() && (-100.0..=100.0).contains(v))
                        && r[2] > r[0]
                        && r[3] > r[1]),
                    "Invalid markup geometry"
                );
            }
            Self::Polygon { points, closed } => ensure!(
                points.len() >= if *closed { 3 } else { 2 }
                    && points.len() <= 4096
                    && points
                        .iter()
                        .flatten()
                        .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
                "Invalid polygon geometry"
            ),
            Self::Line { start, end, .. } | Self::DoubleArrow { start, end } => ensure!(
                start
                    .iter()
                    .chain(end)
                    .all(|v| v.is_finite() && (-0.5..=1.5).contains(v)),
                "Invalid line geometry"
            ),
            Self::Svg { source, .. } => {
                crate::svg::tree(source, None)?;
            }
            Self::Image { png } => {
                ensure!(
                    png.len() <= crate::assets::MAX_ASSET_BYTES,
                    "Artwork exceeds the asset limit"
                );
                crate::assets::decode_png(png)?;
            }
            _ => {}
        }
        Ok(())
    }
}

pub mod image_bytes {
    use super::*;
    use base64::Engine;
    pub fn serialize<S: serde::Serializer>(
        data: &Arc<Vec<u8>>,
        s: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(data.as_slice()))
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        d: D,
    ) -> std::result::Result<Arc<Vec<u8>>, D::Error> {
        let value = String::deserialize(d)?;
        if value.len() > crate::assets::MAX_ASSET_BYTES * 4 / 3 + 8 {
            return Err(serde::de::Error::custom("Artwork exceeds the asset limit"));
        }
        base64::engine::general_purpose::STANDARD
            .decode(value)
            .map(Arc::new)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Style {
    pub color: [f32; 3],
    pub fill: Option<[f32; 3]>,
    pub opacity: f32,
    pub width: f32,
    #[serde(default)]
    pub stroke: StrokeStyle,
    pub font_size: f32,
    #[serde(default)]
    pub image_blur: f32,
    #[serde(default)]
    pub image_border: bool,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            color: [0.11, 0.43, 0.35],
            fill: None,
            opacity: 1.0,
            width: 2.0,
            stroke: StrokeStyle::Solid,
            font_size: 14.0,
            image_blur: 0.,
            image_border: false,
        }
    }
}
/// Editable object metadata travels with the artwork, including through the clipboard.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ObjectProperties {
    /// Structured table data accompanying its movable vector appearance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<crate::tables::Table>,
    #[serde(default)]
    pub review: crate::review::Review,
    /// A persistent, page-local group. Copies receive independent group IDs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub locked: bool,
    /// Normalized source-image coordinates. Original pixels are never discarded.
    #[serde(default)]
    pub crop: Option<Box2>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Annotation {
    pub id: String,
    pub page: usize,
    pub rect: Box2,
    pub rotation: f32,
    #[serde(default)]
    pub flip_x: bool,
    #[serde(default)]
    pub flip_y: bool,
    pub kind: Kind,
    pub style: Style,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
    #[serde(default)]
    pub object: ObjectProperties,
}
impl Annotation {
    pub fn new(page: usize, rect: Box2, kind: Kind, style: Style) -> Self {
        Self {
            id: format!("{:032x}", rand::random::<u128>()),
            page,
            rect,
            rotation: 0.0,
            flip_x: false,
            flip_y: false,
            kind,
            style,
            asset_id: None,
            object: ObjectProperties::default(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.id.len() == 32 && self.id.bytes().all(|c| c.is_ascii_hexdigit()),
            "Invalid annotation ID"
        );
        ensure!(
            self.rect
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 100_000.0)
                && self.rect[2] - self.rect[0] >= 1.0
                && self.rect[3] - self.rect[1] >= 1.0,
            "Invalid annotation rectangle"
        );
        ensure!(
            self.rotation.is_finite() && self.rotation.abs() <= 360.0,
            "Invalid rotation"
        );
        ensure!(
            self.style
                .color
                .iter()
                .chain(self.style.fill.iter().flatten())
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
                && (if matches!(self.kind, Kind::Image { .. }) {
                    0.0
                } else {
                    0.01
                }..=1.0)
                    .contains(&self.style.opacity)
                && self.style.image_blur.is_finite()
                && (0.0..=32.).contains(&self.style.image_blur)
                && (0.1..=30.0).contains(&self.style.width)
                && (1.0..=144.0).contains(&self.style.font_size),
            "Invalid annotation style"
        );
        ensure!(
            self.object.name.chars().count() <= 200,
            "Object name is too long"
        );
        ensure!(
            self.object
                .group
                .as_ref()
                .is_none_or(|g| g.len() == 32 && g.bytes().all(|c| c.is_ascii_hexdigit())),
            "Invalid object group"
        );
        ensure!(
            self.object.review.author.chars().count() <= 200
                && self.object.review.replies.len() <= 100
                && self
                    .object
                    .review
                    .replies
                    .iter()
                    .all(|r| r.author.chars().count() <= 200 && r.text.len() <= 16000),
            "Review thread exceeds its limit"
        );
        if let Some(crop) = self.object.crop {
            ensure!(
                matches!(self.kind, Kind::Image { .. }),
                "Only images can be cropped"
            );
            crate::object_tools::validate_crop(crop)?;
        }
        if let Some(table) = &self.object.table {
            ensure!(
                matches!(self.kind, Kind::Svg { .. }),
                "A table layer must use vector artwork"
            );
            table.validate()?;
        }
        self.kind.validate()?;
        if let Kind::Image { png } = &self.kind {
            crate::image_effects::blurred(png, self.style.image_blur)?;
        }
        Ok(())
    }
    pub fn local(&self, p: Point2) -> Point2 {
        [
            self.rect[0] + p[0] * (self.rect[2] - self.rect[0]),
            self.rect[1] + p[1] * (self.rect[3] - self.rect[1]),
        ]
    }
    pub fn rotated(&self, p: Point2) -> Point2 {
        rotate(self.reflected(p), self.rect, self.rotation)
    }
    fn reflected(&self, p: Point2) -> Point2 {
        [
            if self.flip_x {
                self.rect[0] + self.rect[2] - p[0]
            } else {
                p[0]
            },
            if self.flip_y {
                self.rect[1] + self.rect[3] - p[1]
            } else {
                p[1]
            },
        ]
    }
    fn untransformed(&self, p: Point2) -> Point2 {
        self.reflected(rotate(p, self.rect, -self.rotation))
    }
    pub fn rotate_by(&mut self, degrees: f32) {
        self.rotation = (self.rotation + degrees + 180.).rem_euclid(360.) - 180.;
    }
    /// Reflect in page coordinates, including objects that are already rotated.
    pub fn flip_horizontal(&mut self) {
        self.flip_x = !self.flip_x;
        self.rotation = -self.rotation;
    }
    pub fn flip_vertical(&mut self) {
        self.flip_y = !self.flip_y;
        self.rotation = -self.rotation;
    }
    pub fn bounds(&self) -> Box2 {
        let mut points = vec![
            [self.rect[0], self.rect[1]],
            [self.rect[2], self.rect[1]],
            [self.rect[2], self.rect[3]],
            [self.rect[0], self.rect[3]],
        ];
        for primitive in primitives(self) {
            match primitive {
                Primitive::Path { points: extra, .. } => points.extend(extra),
                Primitive::Dot { center } => points.push(center),
                _ => {}
            }
        }
        bounds(
            &points
                .into_iter()
                .map(|p| self.rotated(p))
                .collect::<Vec<_>>(),
        )
    }
    pub fn contains(&self, p: Point2) -> bool {
        let p = self.untransformed(p);
        p[0] >= self.rect[0] && p[0] <= self.rect[2] && p[1] >= self.rect[1] && p[1] <= self.rect[3]
    }
    /// Hit the drawing itself rather than the empty corners of a diagonal line's box.
    /// Tolerance is supplied in page points, so the UI can keep a constant screen target.
    pub fn hit_test(&self, p: Point2, tolerance: f32) -> bool {
        if self.object.hidden {
            return false;
        }
        let p = self.untransformed(p);
        let tolerance = tolerance + self.style.width / 2.;
        let paths = if self.style.stroke == StrokeStyle::Wave {
            primitives(self)
        } else {
            raw_primitives(self)
        };
        paths.into_iter().any(|primitive| match primitive {
            Primitive::Path { points, closed, .. } => {
                let near = |a: Point2, b: Point2| {
                    let d = [b[0] - a[0], b[1] - a[1]];
                    let length2 = d[0] * d[0] + d[1] * d[1];
                    let t = if length2 < 0.000001 {
                        0.
                    } else {
                        (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / length2).clamp(0., 1.)
                    };
                    (p[0] - a[0] - d[0] * t).hypot(p[1] - a[1] - d[1] * t) <= tolerance
                };
                if points.windows(2).any(|pair| near(pair[0], pair[1])) {
                    return true;
                }
                if !closed || points.len() < 3 {
                    return false;
                }
                if near(*points.last().unwrap(), points[0]) {
                    return true;
                }
                let mut inside = false;
                for i in 0..points.len() {
                    let a = points[i];
                    let b = points[(i + 1) % points.len()];
                    if (a[1] > p[1]) != (b[1] > p[1])
                        && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
                    {
                        inside = !inside;
                    }
                }
                inside
            }
            Primitive::Dot { center } => (p[0] - center[0]).hypot(p[1] - center[1]) <= tolerance,
            Primitive::Text { rect, .. } | Primitive::Image { rect, .. } => {
                p[0] >= rect[0] && p[0] <= rect[2] && p[1] >= rect[1] && p[1] <= rect[3]
            }
        })
    }
    pub fn duplicate(&self) -> Self {
        let mut next = self.clone();
        next.id = format!("{:032x}", rand::random::<u128>());
        next.object.locked = false;
        next.object.group = None;
        next.rect[0] += 12.0;
        next.rect[1] += 12.0;
        next.rect[2] += 12.0;
        next.rect[3] += 12.0;
        next
    }
}
pub fn rotate(p: Point2, r: Box2, degrees: f32) -> Point2 {
    let center = [(r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0];
    let (sin, cos) = degrees.to_radians().sin_cos();
    let (x, y) = (p[0] - center[0], p[1] - center[1]);
    [center[0] + x * cos - y * sin, center[1] + x * sin + y * cos]
}
pub fn bounds(points: &[Point2]) -> Box2 {
    points.iter().fold(
        [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ],
        |a, b| {
            [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[0]),
                a[3].max(b[1]),
            ]
        },
    )
}

#[derive(Clone)]
struct Snapshot {
    source: Arc<[u8]>,
    annotations: Vec<Annotation>,
    revision: u64,
    name: Option<String>,
    focus: Option<(usize, Option<Box2>)>,
}
pub struct Session {
    pub source: Arc<[u8]>,
    pub annotations: Vec<Annotation>,
    pub editable: bool,
    past: Vec<Snapshot>,
    future: Vec<Snapshot>,
    revision: u64,
    next: u64,
    saved: u64,
    name: Option<String>,
    focus: Option<(usize, Option<Box2>)>,
    retained_bytes: usize,
}
impl Session {
    pub fn new(source: Arc<[u8]>, annotations: Vec<Annotation>, editable: bool) -> Self {
        let mut session = Self {
            source,
            annotations,
            editable,
            past: vec![],
            future: vec![],
            revision: 0,
            next: 0,
            saved: 0,
            name: None,
            focus: None,
            retained_bytes: 0,
        };
        session.retained_bytes = session.history_bytes();
        session
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn dirty(&self) -> bool {
        self.revision != self.saved
    }
    pub fn mark_saved(&mut self, revision: u64) {
        self.saved = revision;
    }
    /// A recovered/new document must still be explicitly saved to a user file.
    pub fn mark_unsaved(&mut self) {
        self.saved = u64::MAX;
    }
    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }
    pub fn commit(&mut self, annotations: Vec<Annotation>) -> Result<()> {
        self.commit_live(annotations, None)
    }
    /// Update immediately, coalescing only consecutive revisions from one UI gesture.
    /// Every frame still gets a fresh revision for autosave and in-flight Save safety.
    pub fn commit_live(
        &mut self,
        annotations: Vec<Annotation>,
        previous: Option<u64>,
    ) -> Result<()> {
        ensure!(
            self.editable,
            "Signed or certified PDFs are read-only for annotations"
        );
        ensure!(
            annotations.len() <= 5000,
            "A document can contain at most 5,000 managed annotations"
        );
        for a in &annotations {
            a.validate()?;
        }
        // A locked object can be renamed, shown/hidden, or explicitly unlocked,
        // but ordinary transforms, formatting and delete cannot change it.
        for old in self.annotations.iter().filter(|a| a.object.locked) {
            let new = annotations
                .iter()
                .find(|a| a.id == old.id)
                .ok_or_else(|| anyhow::anyhow!("Unlock the object before deleting it"))?;
            let mut allowed = old.clone();
            allowed.object.name = new.object.name.clone();
            allowed.object.hidden = new.object.hidden;
            allowed.object.locked = new.object.locked;
            ensure!(&allowed == new, "Unlock the object before editing it");
        }
        if annotations == self.annotations {
            return Ok(());
        }
        if previous != Some(self.revision)
            || self.past.is_empty()
            || !self.future.is_empty()
            || self.saved == self.revision
        {
            self.past.push(Snapshot {
                source: self.source.clone(),
                annotations: self.annotations.clone(),
                revision: self.revision,
                name: self.name.clone(),
                focus: self.focus,
            });
        }
        self.future.clear();
        self.name = None;
        self.next += 1;
        self.revision = self.next;
        let before: std::collections::HashMap<_, _> = self
            .annotations
            .iter()
            .enumerate()
            .map(|(i, a)| (&a.id, (i, a)))
            .collect();
        let after: std::collections::HashMap<_, _> = annotations
            .iter()
            .enumerate()
            .map(|(i, a)| (&a.id, (i, a)))
            .collect();
        let changed: Vec<_> = annotations
            .iter()
            .enumerate()
            .filter(|(i, a)| before.get(&a.id).is_none_or(|old| *old != (*i, *a)))
            .map(|(_, a)| a)
            .chain(
                self.annotations
                    .iter()
                    .enumerate()
                    .filter(|(i, a)| after.get(&a.id).is_none_or(|new| *new != (*i, *a)))
                    .map(|(_, a)| a),
            )
            .collect();
        self.focus = changed.first().map(|first| {
            let points: Vec<_> = changed
                .iter()
                .filter(|a| a.page == first.page)
                .flat_map(|a| {
                    let b = a.bounds();
                    [[b[0], b[1]], [b[2], b[3]]]
                })
                .collect();
            (first.page, Some(bounds(&points)))
        });
        self.annotations = annotations;
        self.trim_history_to_budget(384 * 1024 * 1024);
        Ok(())
    }
    /// A page transaction shares the same undo timeline as annotation edits.
    pub fn commit_document(&mut self, bytes: Vec<u8>) -> Result<()> {
        self.commit_document_with_budget(bytes, 384 * 1024 * 1024, false)
    }
    /// Used when the caller will immediately Undo a pending edit. Reject before
    /// mutation if history trimming could otherwise discard its previous state.
    pub fn commit_document_requiring_undo(&mut self, bytes: Vec<u8>) -> Result<()> {
        self.commit_document_with_budget(bytes, 384 * 1024 * 1024, true)
    }
    fn commit_document_with_budget(
        &mut self,
        bytes: Vec<u8>,
        budget: usize,
        require_undo: bool,
    ) -> Result<()> {
        ensure!(self.editable, "This document is read-only");
        let doc = Document::load_mem(&bytes)?;
        ensure!(inspect(&doc).rewritable(), "Page result is not editable");
        let annotations = load(&doc);
        let source: Arc<[u8]> = bytes.into();
        if require_undo {
            let required = Self::buffer_bytes([
                (&self.source, self.annotations.as_slice()),
                (&source, annotations.as_slice()),
            ]);
            let current = Self::buffer_bytes(std::iter::once((&source, annotations.as_slice())));
            ensure!(
                required <= budget.max(current),
                "This edit is too large to retain Undo and Redo within the history memory limit. The PDF is unchanged and your text edit is still open; use Cancel to discard it."
            );
        }
        self.past.push(Snapshot {
            source: self.source.clone(),
            annotations: self.annotations.clone(),
            revision: self.revision,
            name: self.name.clone(),
            focus: self.focus,
        });
        self.future.clear();
        self.focus = Some((0, None));
        self.source = source;
        self.annotations = annotations;
        self.name = None;
        self.next += 1;
        self.revision = self.next;
        self.trim_history_to_budget(budget);
        Ok(())
    }
    /// Unique PDF and source-image storage in the current document plus both
    /// directions of history. Shared Arc payloads count once, not once per step.
    /// This excludes decoder/GPU caches, which the viewer budgets separately.
    pub fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }
    /// Essential source storage, excluding undo and redo. Useful as the floor
    /// when the viewer divides a shared memory budget between open documents.
    pub fn current_retained_bytes(&self) -> usize {
        Self::buffer_bytes(std::iter::once((&self.source, self.annotations.as_slice())))
    }
    /// Evict the oldest undo / most distant redo states. The current document
    /// is never discarded, even when it alone exceeds the requested budget.
    pub fn trim_history_to_budget(&mut self, budget: usize) {
        // Shared payloads can make old states effectively free. If the current
        // document alone is too large, removing those states would not help.
        let budget = budget.max(self.current_retained_bytes());
        self.retained_bytes = self.history_bytes();
        while self.past.len() + self.future.len() > 100 || self.retained_bytes > budget {
            if !self.past.is_empty() {
                self.past.remove(0);
            } else if !self.future.is_empty() {
                self.future.remove(0);
            } else {
                break;
            }
            self.retained_bytes = self.history_bytes();
        }
    }
    fn history_bytes(&self) -> usize {
        Self::buffer_bytes(
            std::iter::once((&self.source, self.annotations.as_slice())).chain(
                self.past
                    .iter()
                    .chain(&self.future)
                    .map(|s| (&s.source, s.annotations.as_slice())),
            ),
        )
    }
    fn buffer_bytes<'a>(
        states: impl IntoIterator<Item = (&'a Arc<[u8]>, &'a [Annotation])>,
    ) -> usize {
        let mut pdfs = std::collections::HashSet::new();
        let mut images = std::collections::HashSet::new();
        let mut total = 0usize;
        for (source, annotations) in states {
            if pdfs.insert(source.as_ptr()) {
                total = total.saturating_add(source.len());
            }
            for annotation in annotations {
                if let Kind::Image { png } = &annotation.kind
                    && images.insert(Arc::as_ptr(png))
                {
                    total = total.saturating_add(png.capacity());
                }
            }
        }
        total
    }
    pub fn undo(&mut self) {
        if let Some(previous) = self.past.pop() {
            self.future.push(Snapshot {
                source: std::mem::replace(&mut self.source, previous.source),
                annotations: std::mem::replace(&mut self.annotations, previous.annotations),
                revision: self.revision,
                name: self.name.clone(),
                focus: self.focus,
            });
            self.revision = previous.revision;
            self.name = previous.name;
            self.focus = previous.focus;
        }
    }
    pub fn redo(&mut self) {
        if let Some(next) = self.future.pop() {
            self.past.push(Snapshot {
                source: std::mem::replace(&mut self.source, next.source),
                annotations: std::mem::replace(&mut self.annotations, next.annotations),
                revision: self.revision,
                name: self.name.clone(),
                focus: self.focus,
            });
            self.revision = next.revision;
            self.name = next.name;
            self.focus = next.focus;
        }
    }
    pub fn set_change_focus(&mut self, page: usize, rect: Option<Box2>) {
        self.focus = Some((page, rect));
    }
    /// Some document edits (such as clearing a signature) keep the current
    /// viewport through Undo/Redo rather than navigating to a default page.
    pub fn clear_change_focus(&mut self) {
        self.focus = None;
    }
    pub fn undo_focus(&self) -> Option<(usize, Option<Box2>)> {
        self.past.last().and(self.focus)
    }
    pub fn redo_focus(&self) -> Option<(usize, Option<Box2>)> {
        self.future.last().and_then(|s| s.focus)
    }
    pub fn name_last_change(&mut self, name: impl Into<String>) {
        self.name = Some(name.into());
    }
    pub fn undo_label(&self) -> Option<String> {
        self.past.last().map(|s| {
            self.name.clone().unwrap_or_else(|| {
                change_label(&s.source, &s.annotations, &self.source, &self.annotations)
            })
        })
    }
    pub fn redo_label(&self) -> Option<String> {
        self.future.last().map(|s| {
            s.name.clone().unwrap_or_else(|| {
                change_label(&self.source, &self.annotations, &s.source, &s.annotations)
            })
        })
    }
    /// Ordered states, including states available through Redo. The cursor is current.
    pub fn history(&self) -> (usize, Vec<String>) {
        let mut states: Vec<(&Arc<[u8]>, &[Annotation])> = self
            .past
            .iter()
            .map(|s| (&s.source, s.annotations.as_slice()))
            .collect();
        states.push((&self.source, &self.annotations));
        states.extend(
            self.future
                .iter()
                .rev()
                .map(|s| (&s.source, s.annotations.as_slice())),
        );
        let mut labels = vec!["Earlier state".to_owned()];
        labels.extend(
            states
                .windows(2)
                .map(|w| change_label(w[0].0, w[0].1, w[1].0, w[1].1)),
        );
        let names = self
            .past
            .iter()
            .map(|s| &s.name)
            .chain(std::iter::once(&self.name))
            .chain(self.future.iter().rev().map(|s| &s.name));
        for (label, name) in labels.iter_mut().zip(names) {
            if let Some(name) = name {
                *label = name.clone();
            }
        }
        (self.past.len(), labels)
    }
    pub fn seek_history(&mut self, cursor: usize) {
        let target = cursor.min(self.past.len() + self.future.len());
        while self.past.len() > target {
            self.undo();
        }
        while self.past.len() < target {
            self.redo();
        }
    }
    pub fn snapshot_bytes(&self) -> Result<Vec<u8>> {
        if self.revision == 0 {
            Ok(self.source.to_vec())
        } else {
            export(&self.source, &self.annotations)
        }
    }
}

fn change_label(
    old_source: &Arc<[u8]>,
    old: &[Annotation],
    source: &Arc<[u8]>,
    new: &[Annotation],
) -> String {
    if !Arc::ptr_eq(old_source, source) {
        return "Edit PDF content or pages".into();
    }
    if new.len() > old.len() {
        let n = new.len() - old.len();
        return format!("Add {n} {}", if n == 1 { "object" } else { "objects" });
    }
    if new.len() < old.len() {
        let n = old.len() - new.len();
        return format!("Delete {n} {}", if n == 1 { "object" } else { "objects" });
    }
    let by_id: std::collections::HashMap<_, _> = new.iter().map(|a| (a.id.as_str(), a)).collect();
    if old.iter().any(|a| !by_id.contains_key(a.id.as_str())) {
        return "Replace objects".into();
    }
    let changed: Vec<_> = old
        .iter()
        .filter_map(|a| {
            by_id
                .get(a.id.as_str())
                .copied()
                .filter(|b| a != *b)
                .map(|b| (a, b))
        })
        .collect();
    if changed.is_empty() {
        return "Reorder objects".into();
    }
    let (a, b) = changed[0];
    let verb = if a.object.crop != b.object.crop {
        "Crop"
    } else if a.object.hidden != b.object.hidden {
        if b.object.hidden { "Hide" } else { "Show" }
    } else if a.object.locked != b.object.locked {
        if b.object.locked { "Lock" } else { "Unlock" }
    } else if a.object.name != b.object.name {
        "Rename"
    } else if a.rotation != b.rotation {
        "Rotate"
    } else if a.flip_x != b.flip_x || a.flip_y != b.flip_y {
        "Flip"
    } else if a.rect != b.rect {
        "Move or resize"
    } else {
        "Format"
    };
    format!(
        "{verb} {}",
        if changed.len() == 1 {
            if matches!(a.kind, Kind::Image { .. }) {
                "image".into()
            } else {
                a.kind.label().to_lowercase()
            }
        } else {
            format!("{} objects", changed.len())
        }
    )
}

#[derive(Clone, Debug)]
pub struct Capabilities {
    pub signed: bool,
    pub certified: bool,
    pub encrypted: bool,
    pub needs_signing_preparation: bool,
    /// The user unlocked every signature for editing (see [`crate::signature_edit`]).
    pub unlocked: bool,
}
impl Capabilities {
    /// Whether this document may be rewritten: unsigned and unencrypted, or a
    /// signed document the user unlocked (saved as an incremental update).
    pub fn rewritable(&self) -> bool {
        !self.encrypted && (!(self.signed || self.certified) || self.unlocked)
    }
}
pub fn inspect(doc: &Document) -> Capabilities {
    Capabilities {
        unlocked: crate::signature_edit::is_unlocked(doc),
        signed: crate::document::has_signatures(doc),
        certified: doc.catalog().is_ok_and(|d| d.get(b"Perms").is_ok()),
        encrypted: doc.is_encrypted(),
        needs_signing_preparation: matches!(
            doc.reference_table.cross_reference_type,
            lopdf::xref::XrefType::CrossReferenceStream
        ),
    }
}
pub fn text_string(s: &str) -> Object {
    let mut bytes = vec![0xfe, 0xff];
    bytes.extend(s.encode_utf16().flat_map(u16::to_be_bytes));
    Object::String(bytes, lopdf::StringFormat::Hexadecimal)
}
pub fn page_geometry(bytes: &[u8]) -> Result<Vec<(Box2, [f64; 6])>> {
    let pdf = hayro::hayro_syntax::Pdf::new(bytes.to_vec())
        .map_err(|e| anyhow::anyhow!("PDF geometry: {e:?}"))?;
    Ok(pdf
        .pages()
        .iter()
        .map(|p| {
            let (w, h) = p.render_dimensions();
            (
                [0.0, 0.0, w, h],
                p.initial_transform(true).to_kurbo().inverse().as_coeffs(),
            )
        })
        .collect())
}
fn decoded(doc: &Document, value: &Object) -> Option<Annotation> {
    let object = match value {
        Object::Reference(id) => doc.get_object(*id).ok()?,
        _ => value,
    };
    let d = object.as_dict().ok()?;
    // Other readers can move/recolor/regenerate our annotations without updating our JSON.
    // Preserve such annotations as foreign rather than replacing their current appearance with stale data.
    let state = d.get(b"MyPDFState").ok()?.as_dict().ok()?;
    for (key, expected) in state.iter() {
        if d.get(key).ok()? != expected {
            return None;
        }
    }
    let appearance = d.get(b"AP").ok()?.as_dict().ok()?.get(b"N").ok()?;
    let stream = doc.dereference(appearance).ok()?.1.as_stream().ok()?;
    if d.get(b"MyPDFApDigest").ok()?.as_str().ok()?
        != crate::identity::fingerprint(&stream.content).as_bytes()
    {
        return None;
    }
    let data = d.get(b"MyPDFData").ok()?.as_str().ok()?;
    if data.len() > crate::assets::MAX_ASSET_BYTES * 2 {
        return None;
    }
    let annotation: Annotation = serde_json::from_slice(data).ok()?;
    if annotation.validate().is_err() {
        return None;
    }
    let nm = d.get(b"NM").ok()?.as_str().ok()?;
    (nm == format!("MyPDF-{}", annotation.id).as_bytes()).then_some(annotation)
}
pub fn load(doc: &Document) -> Vec<Annotation> {
    let mut result = vec![];
    let mut seen = std::collections::HashSet::new();
    for (number, id) in doc.get_pages() {
        if let Ok(array) = doc
            .get_dictionary(id)
            .and_then(|d| d.get(b"Annots"))
            .and_then(|o| doc.dereference(o).map(|(_, o)| o))
            .and_then(Object::as_array)
        {
            for value in array {
                if let Some(mut a) = decoded(doc, value) {
                    a.page = number as usize - 1;
                    if !seen.insert(a.id.clone()) {
                        a.id = format!("{:032x}", rand::random::<u128>());
                    }
                    result.push(a);
                    if result.len() == 5000 {
                        return result;
                    }
                }
            }
        }
    }
    result
}
fn remove_managed(doc: &mut Document) -> Result<()> {
    for (_, page) in doc.get_pages() {
        let values = doc
            .get_dictionary(page)?
            .get(b"Annots")
            .ok()
            .and_then(|o| match o {
                Object::Reference(id) => doc.get_object(*id).ok(),
                _ => Some(o),
            })
            .and_then(|o| o.as_array().ok())
            .cloned()
            .unwrap_or_default();
        let keep: Vec<_> = values
            .into_iter()
            .filter(|v| decoded(doc, v).is_none())
            .collect();
        if !keep.is_empty() {
            doc.get_dictionary_mut(page)?
                .set("Annots", Object::Array(keep));
        } else {
            doc.get_dictionary_mut(page)?.remove(b"Annots");
        }
    }
    Ok(())
}
pub fn clean_render_copy(bytes: &[u8], doc: &mut Document) -> Result<Vec<u8>> {
    if load(doc).is_empty() || !inspect(doc).rewritable() {
        return Ok(bytes.to_vec());
    }
    remove_managed(doc)?;
    let mut output = vec![];
    doc.save_to(&mut output)?;
    Ok(output)
}

pub enum Primitive {
    Dot {
        center: Point2,
    },
    Path {
        points: Vec<Point2>,
        closed: bool,
        fill: bool,
    },
    Text {
        text: String,
        rect: Box2,
        size: f32,
        centered: bool,
    },
    Image {
        png: Arc<Vec<u8>>,
        rect: Box2,
    },
}
/// Triangulate simple vector silhouettes without changing their outline.
pub fn triangulate(polygon: &[Point2]) -> Vec<[Point2; 3]> {
    if polygon.len() < 3 {
        return vec![];
    }
    let cross = |a: Point2, b: Point2, c: Point2| {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    };
    let area: f32 = (0..polygon.len())
        .map(|i| {
            let a = polygon[i];
            let b = polygon[(i + 1) % polygon.len()];
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    let sign = area.signum();
    let mut indices: Vec<_> = (0..polygon.len()).collect();
    let mut result = vec![];
    while indices.len() > 3 {
        let mut found = false;
        for j in 0..indices.len() {
            let [a, b, c] = [
                indices[(j + indices.len() - 1) % indices.len()],
                indices[j],
                indices[(j + 1) % indices.len()],
            ];
            if cross(polygon[a], polygon[b], polygon[c]) * sign <= 0.0000001 {
                continue;
            }
            let inside = indices.iter().any(|&k| {
                k != a
                    && k != b
                    && k != c
                    && cross(polygon[a], polygon[b], polygon[k]) * sign >= 0.0
                    && cross(polygon[b], polygon[c], polygon[k]) * sign >= 0.0
                    && cross(polygon[c], polygon[a], polygon[k]) * sign >= 0.0
            });
            if inside {
                continue;
            }
            result.push([polygon[a], polygon[b], polygon[c]]);
            indices.remove(j);
            found = true;
            break;
        }
        if !found {
            return result;
        }
    }
    if indices.len() == 3 {
        result.push([
            polygon[indices[0]],
            polygon[indices[1]],
            polygon[indices[2]],
        ]);
    }
    result
}

pub fn primitives(a: &Annotation) -> Vec<Primitive> {
    if a.object.hidden {
        return vec![];
    }
    raw_primitives(a)
        .into_iter()
        .enumerate()
        .flat_map(|(index, primitive)| {
            match primitive {
                Primitive::Path {
                    points,
                    closed,
                    fill: true,
                } if a.style.stroke == StrokeStyle::Wave
                    && matches!(
                        a.kind,
                        Kind::Rectangle | Kind::Ellipse | Kind::Polygon { closed: true, .. }
                    ) =>
                {
                    crate::stroke::path(points, closed, a.style.width, a.style.stroke)
                        .into_iter()
                        .map(|p| match p {
                            Primitive::Path { points, closed, .. } => Primitive::Path {
                                points,
                                closed,
                                fill: true,
                            },
                            other => other,
                        })
                        .collect()
                }
                Primitive::Path {
                    points,
                    closed,
                    fill: false,
                } => {
                    // Preserve arrowheads and comment glyphs as recognizable
                    // icons, even when a drawing stroke style was selected.
                    let style = if matches!(a.kind, Kind::Note { .. })
                        || index > 0
                            && matches!(a.kind, Kind::Line { .. } | Kind::DoubleArrow { .. })
                    {
                        StrokeStyle::Solid
                    } else {
                        a.style.stroke
                    };
                    crate::stroke::path(points, closed, a.style.width, style)
                }
                Primitive::Path {
                    points,
                    closed,
                    fill: true,
                } if a.style.stroke != StrokeStyle::Solid
                    && matches!(a.kind, Kind::Vector { .. } | Kind::Symbol(_)) =>
                {
                    crate::stroke::path(points, closed, a.style.width, a.style.stroke)
                }
                other => vec![other],
            }
        })
        .collect()
}
fn raw_primitives(a: &Annotation) -> Vec<Primitive> {
    let r = a.rect;
    let box_path = |r: Box2, fill| Primitive::Path {
        points: vec![[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]],
        closed: true,
        fill,
    };
    let path = |points: Vec<Point2>, closed, fill| Primitive::Path {
        points: points.into_iter().map(|p| a.local(p)).collect(),
        closed,
        fill,
    };
    match &a.kind {
        Kind::Vector { polygons } if a.style.stroke != StrokeStyle::Solid => {
            crate::stroke::outlines(polygons)
                .into_iter()
                .map(|p| path(p, true, false))
                .collect()
        }
        Kind::Vector { polygons } => polygons
            .iter()
            .map(|polygon| Primitive::Path {
                points: polygon.iter().map(|p| a.local(*p)).collect(),
                closed: true,
                fill: true,
            })
            .collect(),
        Kind::Markup { kind, quads } => quads
            .iter()
            .map(|q| {
                let q = [
                    r[0] + q[0] * (r[2] - r[0]),
                    r[1] + q[1] * (r[3] - r[1]),
                    r[0] + q[2] * (r[2] - r[0]),
                    r[1] + q[3] * (r[3] - r[1]),
                ];
                match kind {
                    Markup::Highlight => box_path(q, true),
                    Markup::Underline | Markup::Strikeout => {
                        let y = if *kind == Markup::Underline {
                            q[3]
                        } else {
                            (q[1] + q[3]) / 2.0
                        };
                        Primitive::Path {
                            points: vec![[q[0], y], [q[2], y]],
                            closed: false,
                            fill: false,
                        }
                    }
                }
            })
            .collect(),
        Kind::Ink { strokes } => strokes
            .iter()
            .map(|p| path(p.clone(), false, false))
            .collect(),
        Kind::Rectangle => {
            let mut result = vec![];
            if a.style.fill.is_some() {
                result.push(box_path(r, true));
            }
            result.push(box_path(r, false));
            result
        }
        Kind::Ellipse => {
            let points: Vec<_> = (0..64)
                .map(|i| {
                    let angle = i as f32 / 64.0 * std::f32::consts::TAU;
                    [0.5 + 0.5 * angle.cos(), 0.5 + 0.5 * angle.sin()]
                })
                .collect();
            let mut result = vec![];
            if a.style.fill.is_some() {
                result.push(path(points.clone(), true, true));
            }
            result.push(path(points, true, false));
            result
        }
        Kind::Polygon { points, closed } => {
            let mut paths = vec![];
            if *closed && a.style.fill.is_some() {
                paths.push(path(points.clone(), true, true));
            }
            paths.push(path(points.clone(), *closed, false));
            paths
        }
        Kind::Line { start, end, .. } | Kind::DoubleArrow { start, end } => {
            let mut paths = vec![path(vec![*start, *end], false, false)];
            let double = matches!(a.kind, Kind::DoubleArrow { .. });
            if double || matches!(a.kind, Kind::Line { arrow: true, .. }) {
                let start = a.local(*start);
                let end = a.local(*end);
                for (from, tip) in
                    [(start, end), (end, start)]
                        .into_iter()
                        .take(if double { 2 } else { 1 })
                {
                    let angle = (tip[1] - from[1]).atan2(tip[0] - from[0]);
                    let len = (a.style.width * 5.0)
                        .max(9.0)
                        .min((tip[0] - from[0]).hypot(tip[1] - from[1]) * 0.4);
                    paths.push(Primitive::Path {
                        points: vec![
                            [
                                tip[0] - len * (angle - 0.45).cos(),
                                tip[1] - len * (angle - 0.45).sin(),
                            ],
                            tip,
                            [
                                tip[0] - len * (angle + 0.45).cos(),
                                tip[1] - len * (angle + 0.45).sin(),
                            ],
                        ],
                        closed: false,
                        fill: false,
                    });
                }
            }
            paths
        }
        Kind::Symbol(symbol) => match symbol {
            Symbol::Check => vec![path(
                vec![[0.1, 0.52], [0.38, 0.84], [0.91, 0.12]],
                false,
                false,
            )],
            Symbol::Cross => vec![
                path(vec![[0.12, 0.12], [0.88, 0.88]], false, false),
                path(vec![[0.88, 0.12], [0.12, 0.88]], false, false),
            ],
            Symbol::Circle => vec![path(
                (0..64)
                    .map(|i| {
                        let angle = i as f32 / 64.0 * std::f32::consts::TAU;
                        [0.5 + 0.4 * angle.cos(), 0.5 + 0.4 * angle.sin()]
                    })
                    .collect(),
                true,
                false,
            )],
            Symbol::Star => vec![path(
                (0..10)
                    .map(|i| {
                        let angle =
                            i as f32 * std::f32::consts::PI / 5.0 - std::f32::consts::FRAC_PI_2;
                        let radius = if i % 2 == 0 { 0.45 } else { 0.2 };
                        [0.5 + radius * angle.cos(), 0.5 + radius * angle.sin()]
                    })
                    .collect(),
                true,
                true,
            )],
            Symbol::Arrow => vec![path(
                vec![
                    [0.08, 0.5],
                    [0.88, 0.5],
                    [0.58, 0.18],
                    [0.88, 0.5],
                    [0.58, 0.82],
                ],
                false,
                false,
            )],
        },
        Kind::Note { .. } => {
            // A comment is an icon, not a blank text box. Keep the speech
            // bubble square when older notes have a wide/tall rectangle, and
            // use the same outline and content marks in the editor and PDF.
            let size = (r[2] - r[0]).min(r[3] - r[1]);
            let origin = [(r[0] + r[2] - size) / 2., (r[1] + r[3] - size) / 2.];
            let icon_path = |points: &[[f32; 2]], closed, fill| Primitive::Path {
                points: points
                    .iter()
                    .map(|p| [origin[0] + p[0] * size, origin[1] + p[1] * size])
                    .collect(),
                closed,
                fill,
            };
            let outline = [
                [0.20, 0.10],
                [0.80, 0.10],
                [0.86, 0.12],
                [0.90, 0.18],
                [0.90, 0.64],
                [0.86, 0.70],
                [0.80, 0.72],
                [0.44, 0.72],
                [0.22, 0.90],
                [0.22, 0.72],
                [0.20, 0.72],
                [0.14, 0.70],
                [0.10, 0.64],
                [0.10, 0.18],
                [0.14, 0.12],
            ];
            let mut result = vec![];
            if a.style.fill.is_some() {
                result.push(icon_path(&outline, true, true));
            }
            result.push(icon_path(&outline, true, false));
            result.push(icon_path(&[[0.28, 0.31], [0.72, 0.31]], false, false));
            result.push(icon_path(&[[0.28, 0.51], [0.60, 0.51]], false, false));
            result
        }
        Kind::Text { text } => {
            let mut p = vec![];
            if a.style.fill.is_some() {
                p.push(box_path(r, true));
            }
            p.push(Primitive::Text {
                text: text.clone(),
                rect: r,
                size: a.style.font_size,
                centered: false,
            });
            p
        }
        Kind::Stamp { text } => vec![
            box_path(r, false),
            Primitive::Text {
                text: text.clone(),
                rect: r,
                size: a.style.font_size,
                centered: true,
            },
        ],
        Kind::Svg { source, tint } => crate::svg::preview(source, tint.then_some(a.style.color))
            .map(|png| vec![Primitive::Image { png, rect: r }])
            .unwrap_or_default(),
        Kind::Image { png } => {
            let mut result = vec![Primitive::Image {
                png: crate::object_tools::image_appearance(a).unwrap_or_else(|_| png.clone()),
                rect: r,
            }];
            if a.style.image_border {
                let inset = (a.style.width / 2.)
                    .min((r[2] - r[0]) / 2.)
                    .min((r[3] - r[1]) / 2.);
                result.push(box_path(
                    [r[0] + inset, r[1] + inset, r[2] - inset, r[3] - inset],
                    false,
                ));
            }
            result
        }
    }
}

fn pdf_numbers(r: impl IntoIterator<Item = f32>) -> Object {
    Object::Array(r.into_iter().map(Object::Real).collect())
}
pub fn image_resource(doc: &mut Document, png: &[u8]) -> Result<ObjectId> {
    let image = crate::assets::decode_png(png)?;
    let (w, h) = (image.width(), image.height());
    let rgba = image.into_rgba8();
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    let mut alpha = Vec::with_capacity((w * h) as usize);
    for p in rgba.pixels() {
        rgb.extend(&p.0[..3]);
        alpha.push(p.0[3]);
    }
    let mut mask = Stream::new(
        dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => w, "Height" => h, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8 },
        alpha,
    );
    mask.compress()?;
    let mask = doc.add_object(mask);
    let mut stream = Stream::new(
        dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => w, "Height" => h, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "SMask" => mask },
        rgb,
    );
    stream.compress()?;
    Ok(doc.add_object(stream))
}
pub fn appearance(
    doc: &mut Document,
    a: &Annotation,
    inverse: Affine,
    font: Option<&crate::pdf_font::PdfFont>,
) -> Result<(ObjectId, Box2)> {
    let mut normal = a.bounds();
    let margin = a.style.width / 2.0 + 0.5;
    normal[0] -= margin;
    normal[1] -= margin;
    normal[2] += margin;
    normal[3] += margin;
    let pdf_box = inverse.transform_rect_bbox(Rect::new(
        normal[0] as f64,
        normal[1] as f64,
        normal[2] as f64,
        normal[3] as f64,
    ));
    let rect = [
        pdf_box.x0 as f32,
        pdf_box.y0 as f32,
        pdf_box.x1 as f32,
        pdf_box.y1 as f32,
    ];
    let rotation = Affine::translate((
        (a.rect[0] + a.rect[2]) as f64 / 2.0,
        (a.rect[1] + a.rect[3]) as f64 / 2.0,
    )) * Affine::rotate(a.rotation.to_radians() as f64)
        * Affine::scale_non_uniform(
            if a.flip_x { -1. } else { 1. },
            if a.flip_y { -1. } else { 1. },
        )
        * Affine::translate((
            -(a.rect[0] + a.rect[2]) as f64 / 2.0,
            -(a.rect[1] + a.rect[3]) as f64 / 2.0,
        ));
    let c = (inverse * rotation).as_coeffs();
    let color = a.style.color;
    let fill = a.style.fill.unwrap_or(color);
    let mut content = format!(
        "q {} {} {} {} {} {} cm /GS gs {} {} {} RG {} {} {} rg {} w 1 J 1 j\n",
        c[0],
        c[1],
        c[2],
        c[3],
        c[4],
        c[5],
        color[0],
        color[1],
        color[2],
        fill[0],
        fill[1],
        fill[2],
        a.style.width
    );
    let gs = doc.add_object(
        dictionary! { "Type" => "ExtGState", "CA" => a.style.opacity, "ca" => a.style.opacity },
    );
    let mut resources = dictionary! { "ExtGState" => dictionary! { "GS" => gs } };
    if let Some(font) = font {
        resources.set("Font", dictionary! { "UF" => font.id });
    }
    let primitives = if a.object.hidden {
        vec![]
    } else if let Kind::Svg { source, tint } = &a.kind {
        let (id, w, h) = crate::svg::embed(doc, source, tint.then_some(a.style.color))?;
        resources.set("XObject", dictionary! { "Svg" => id });
        content.push_str(&format!(
            "q {} 0 0 {} {} {} cm /Svg Do Q\n",
            (a.rect[2] - a.rect[0]) / w,
            -(a.rect[3] - a.rect[1]) / h,
            a.rect[0],
            a.rect[3]
        ));
        vec![]
    } else {
        primitives(a)
    };
    for primitive in primitives {
        match primitive {
            Primitive::Dot { center } => {
                content.push_str(&format!("q {} {} {} rg\n", color[0], color[1], color[2]));
                for i in 0..24 {
                    let angle = i as f32 / 24. * std::f32::consts::TAU;
                    content.push_str(&format!(
                        "{} {} {}\n",
                        center[0] + angle.cos() * a.style.width / 2.,
                        center[1] + angle.sin() * a.style.width / 2.,
                        if i == 0 { "m" } else { "l" }
                    ));
                }
                content.push_str("h f Q\n");
            }
            Primitive::Path {
                points,
                closed,
                fill,
            } => {
                if let Some(first) = points.first() {
                    content.push_str(&format!("{} {} m\n", first[0], first[1]));
                    for p in &points[1..] {
                        content.push_str(&format!("{} {} l\n", p[0], p[1]));
                    }
                    if closed {
                        content.push_str("h\n");
                    }
                    // A vector asset is one compound path. Painting each triangle/subpath
                    // separately creates anti-alias seams and compounds transparency.
                    if !matches!(a.kind, Kind::Vector { .. })
                        || a.style.stroke != StrokeStyle::Solid
                    {
                        content.push_str(if fill { "f\n" } else { "S\n" });
                    }
                }
            }
            Primitive::Text {
                text,
                rect,
                size,
                centered,
            } => {
                let font = font.context("Text appearance needs an embedded font")?;
                let lines = font.wrap(&text.replace('\t', "    "), size, rect[2] - rect[0] - 8.0);
                let height = lines.len() as f32 * size * 1.25;
                ensure!(
                    height <= rect[3] - rect[1] - 4.0,
                    "The text does not fit in this annotation. Enlarge the box or reduce its font size."
                );
                for (i, line) in lines.iter().enumerate() {
                    ensure!(
                        font.width(line, size) <= rect[2] - rect[0] - 8.0,
                        "A word does not fit in this annotation. Enlarge the box or reduce its font size."
                    );
                    let x = if centered {
                        (rect[0] + rect[2] - font.width(line, size)) / 2.0
                    } else {
                        rect[0] + 4.0
                    };
                    let y = if centered {
                        (rect[1] + rect[3] - height) / 2.0 + size
                    } else {
                        rect[1] + 4.0 + size
                    } + i as f32 * size * 1.25;
                    content.push_str(&format!(
                        "{} {} {} rg BT /UF {size} Tf 1 0 0 -1 {x} {y} Tm <{}> Tj ET\n",
                        color[0],
                        color[1],
                        color[2],
                        font.hex(line)?
                    ));
                }
            }
            Primitive::Image { png, rect } => {
                let id = image_resource(doc, &png)?;
                resources.set("XObject", dictionary! { "Im" => id });
                content.push_str(&format!(
                    "q {} 0 0 {} {} {} cm /Im Do Q\n",
                    rect[2] - rect[0],
                    -(rect[3] - rect[1]),
                    rect[0],
                    rect[3]
                ));
            }
        }
    }
    if matches!(a.kind, Kind::Vector { .. }) && a.style.stroke == StrokeStyle::Solid {
        content.push_str("f\n");
    }
    content.push('Q');
    let mut stream = Stream::new(
        dictionary! { "Type" => "XObject", "Subtype" => "Form", "FormType" => 1, "BBox" => pdf_numbers(rect), "Resources" => resources },
        content.into_bytes(),
    );
    stream.compress()?;
    Ok((doc.add_object(stream), rect))
}

pub fn export(bytes: &[u8], annotations: &[Annotation]) -> Result<Vec<u8>> {
    ensure!(annotations.len() <= 5000, "Too many annotations");
    let mut doc = Document::load_mem(bytes)?;
    ensure!(
        inspect(&doc).rewritable(),
        "Signed, certified, or encrypted PDFs cannot be rewritten for annotations"
    );
    let geometry = page_geometry(bytes)?;
    let pages = doc.get_pages();
    for a in annotations {
        a.validate()?;
        ensure!(a.page < geometry.len(), "Annotation page is missing");
    }
    remove_managed(&mut doc)?;
    let text: String = annotations
        .iter()
        .filter(|a| matches!(a.kind, Kind::Text { .. } | Kind::Stamp { .. }))
        .filter_map(|a| a.kind.text())
        .collect();
    let font = if text.is_empty() {
        None
    } else {
        Some(crate::pdf_font::PdfFont::embed(&mut doc, &text)?)
    };
    for a in annotations {
        let page_id = pages[&(a.page as u32 + 1)];
        let inverse = Affine::new(geometry[a.page].1);
        let (ap, rect) = appearance(&mut doc, a, inverse, font.as_ref())?;
        let subtype = match a.kind {
            Kind::Markup {
                kind: Markup::Highlight,
                ..
            } => "Highlight",
            Kind::Markup {
                kind: Markup::Underline,
                ..
            } => "Underline",
            Kind::Markup {
                kind: Markup::Strikeout,
                ..
            } => "StrikeOut",
            Kind::Ink { .. } => "Ink",
            Kind::Note { .. } => "Text",
            Kind::Text { .. } => "FreeText",
            Kind::Rectangle => "Square",
            Kind::Ellipse => "Circle",
            Kind::Line { .. } | Kind::DoubleArrow { .. } => "Line",
            Kind::Polygon { closed: true, .. } => "Polygon",
            Kind::Polygon { .. } => "PolyLine",
            _ => "Stamp",
        };
        let mut d = dictionary! { "Type" => "Annot", "Subtype" => subtype, "Rect" => pdf_numbers(rect), "P" => page_id, "F" => (if a.object.hidden { 2 } else { 4 } | if a.object.locked { 128 } else { 0 }), "NM" => Object::string_literal(format!("MyPDF-{}", a.id)), "T" => text_string("MyPDF"), "C" => pdf_numbers(a.style.color), "CA" => a.style.opacity, "BS" => dictionary! { "W" => a.style.width, "S" => "S" }, "AP" => dictionary! { "N" => ap }, "MyPDFData" => Object::string_literal(serde_json::to_string(a)?) };
        if let Some(dash) = a.style.stroke.dash_array(a.style.width) {
            d.set(
                "BS",
                dictionary! { "W" => a.style.width, "S" => "D", "D" => pdf_numbers(dash) },
            );
        }
        if let Some(text) = a.kind.text() {
            d.set("Contents", text_string(text));
        }
        if !a.object.review.author.is_empty() {
            d.set("T", text_string(&a.object.review.author));
        }
        if !a.object.review.replies.is_empty() {
            let mut text = a.kind.text().unwrap_or(a.kind.label()).to_owned();
            for reply in &a.object.review.replies {
                text.push_str(&format!("\n\n{}: {}", reply.author, reply.text));
            }
            d.set("Contents", text_string(&text));
        }
        let point = |p: Point2| {
            let p = a.rotated(p);
            let p = inverse * Point::new(p[0] as f64, p[1] as f64);
            [p.x as f32, p.y as f32]
        };
        match &a.kind {
            Kind::Markup { quads, .. } => {
                let quads: Vec<_> = quads
                    .iter()
                    .flat_map(|q| {
                        [[q[0], q[1]], [q[2], q[1]], [q[0], q[3]], [q[2], q[3]]]
                            .map(|p| point(a.local(p)))
                            .into_iter()
                            .flatten()
                    })
                    .collect();
                d.set("QuadPoints", pdf_numbers(quads));
            }
            Kind::Ink { strokes } => {
                d.set(
                    "InkList",
                    Object::Array(
                        strokes
                            .iter()
                            .map(|s| pdf_numbers(s.iter().flat_map(|p| point(a.local(*p)))))
                            .collect(),
                    ),
                );
            }
            Kind::Polygon { points, .. } => {
                d.set(
                    "Vertices",
                    pdf_numbers(points.iter().flat_map(|p| point(a.local(*p)))),
                );
                if let Some(fill) = a.style.fill {
                    d.set("IC", pdf_numbers(fill));
                }
            }
            Kind::Line { start, end, .. } | Kind::DoubleArrow { start, end } => {
                d.set(
                    "L",
                    pdf_numbers(
                        point(a.local(*start))
                            .into_iter()
                            .chain(point(a.local(*end))),
                    ),
                );
                if matches!(
                    a.kind,
                    Kind::DoubleArrow { .. } | Kind::Line { arrow: true, .. }
                ) {
                    d.set(
                        "LE",
                        vec![
                            Object::Name(if matches!(a.kind, Kind::DoubleArrow { .. }) {
                                b"OpenArrow".to_vec()
                            } else {
                                b"None".to_vec()
                            }),
                            Object::Name(b"OpenArrow".to_vec()),
                        ],
                    );
                }
            }
            Kind::Note { .. } => d.set("Name", "Comment"),
            Kind::Text { .. } => {
                d.set(
                    "DA",
                    Object::string_literal(format!(
                        "/UF {} Tf {} {} {} rg",
                        a.style.font_size, a.style.color[0], a.style.color[1], a.style.color[2]
                    )),
                );
                if let Some(font) = &font {
                    d.set(
                        "DR",
                        dictionary! { "Font" => dictionary! { "UF" => font.id } },
                    );
                }
            }
            Kind::Stamp { .. } | Kind::Image { .. } | Kind::Symbol(_) => {
                d.set("Name", "MyPDFStamp")
            }
            _ => {}
        }
        let mut state = d.clone();
        for key in [b"MyPDFData".as_slice(), b"AP", b"P", b"NM"] {
            state.remove(key);
        }
        d.set("MyPDFState", state);
        let digest = crate::identity::fingerprint(&doc.get_object(ap)?.as_stream()?.content);
        d.set("MyPDFApDigest", Object::string_literal(digest));
        let id = doc.add_object(d);
        let mut list = doc
            .get_dictionary(page_id)?
            .get(b"Annots")
            .ok()
            .and_then(|v| v.as_array().ok())
            .cloned()
            .unwrap_or_default();
        list.push(Object::Reference(id));
        doc.get_dictionary_mut(page_id)?
            .set("Annots", Object::Array(list));
    }
    // Remove superseded managed appearances/fonts so repeated edits do not grow forever.
    doc.prune_objects();
    doc.trailer.remove(b"Prev");
    doc.trailer.remove(b"XRefStm");
    doc.reference_table.cross_reference_type = lopdf::xref::XrefType::CrossReferenceTable;
    let mut out = vec![];
    doc.save_to(&mut out)?;
    ensure!(
        out.len() <= crate::viewer::MAX_FILE_BYTES as usize,
        "Annotated PDF exceeds the 256 MB limit"
    );
    ensure!(
        load(&Document::load_mem(&out)?).len() == annotations.len(),
        "Saved annotations failed the reopen check"
    );
    Ok(out)
}

#[cfg(test)]
mod memory_tests {
    use super::*;

    fn image(color: u8) -> Annotation {
        let picture = image::RgbaImage::from_pixel(8, 8, image::Rgba([color, 90, 180, 255]));
        let mut output = std::io::Cursor::new(Vec::with_capacity(4096));
        picture
            .write_to(&mut output, image::ImageFormat::Png)
            .unwrap();
        Annotation::new(
            0,
            [10., 10., 90., 90.],
            Kind::Image {
                png: Arc::new(output.into_inner()),
            },
            Style::default(),
        )
    }

    #[test]
    fn shared_pdf_and_image_payloads_count_once_across_undo_and_redo() {
        let source: Arc<[u8]> = vec![0u8; 100].into();
        let mut mark = image(10);
        let mut session = Session::new(source, vec![mark.clone()], true);
        let bytes = session.retained_bytes();
        assert_eq!(bytes, 100 + 4096);
        for i in 1..12 {
            mark.rotation = i as f32;
            session
                .commit(vec![mark.clone(), mark.duplicate()])
                .unwrap();
        }
        for _ in 0..6 {
            session.undo();
            assert_eq!(session.retained_bytes(), bytes);
            assert_eq!(
                session.history_bytes(),
                bytes,
                "redo is included without double counting"
            );
        }
        session.trim_history_to_budget(bytes);
        assert_eq!(session.past.len() + session.future.len(), 11);
        for _ in 0..6 {
            session.redo();
            assert_eq!(session.history_bytes(), bytes);
        }
    }

    #[test]
    fn image_churn_budget_evicts_oldest_states_and_releases_their_buffers() {
        let mut session = Session::new(vec![0u8; 100].into(), vec![], true);
        let mut weak = Vec::new();
        for color in 0..40 {
            let mark = image(color);
            let Kind::Image { png } = &mark.kind else {
                unreachable!()
            };
            weak.push(Arc::downgrade(png));
            session.commit(vec![mark]).unwrap();
            session.trim_history_to_budget(100 + 3 * 4096);
            assert!(session.retained_bytes() <= 100 + 3 * 4096);
        }
        assert!(weak[..37].iter().all(|w| w.upgrade().is_none()));
        session.undo();
        assert!(session.can_redo());
        assert_eq!(session.history_bytes(), session.retained_bytes());
        session.trim_history_to_budget(100 + 2 * 4096);
        assert_eq!(session.past.len(), 0, "old undo states go first");
        assert!(session.can_redo(), "nearest redo survives while it fits");
        session.redo();
        assert_eq!(session.history_bytes(), session.retained_bytes());
        drop(session);
        assert!(weak.iter().all(|w| w.upgrade().is_none()));
    }

    #[test]
    fn required_document_undo_rejects_before_mutation_when_snapshots_exceed_budget() {
        let original = crate::convert::blank(595., 842.).unwrap();
        let replacement = crate::convert::blank(600., 800.).unwrap();
        let mut session = Session::new(original.clone().into(), vec![], true);
        session.commit(vec![image(10)]).unwrap();
        session.undo();
        let source = session.source.clone();
        let before = (
            session.revision,
            session.next,
            session.past.len(),
            session.future.len(),
        );
        let result = session.commit_document_with_budget(replacement, original.len(), true);
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("history memory limit")
        );
        assert!(Arc::ptr_eq(&session.source, &source));
        assert!(session.annotations.is_empty());
        assert_eq!(
            (
                session.revision,
                session.next,
                session.past.len(),
                session.future.len()
            ),
            before
        );
        session.redo();
        assert_eq!(
            session.annotations.len(),
            1,
            "a rejected pending Undo does not destroy older history"
        );
    }

    #[test]
    fn required_document_undo_retains_latest_snapshot_with_a_bounded_budget() {
        let original = crate::convert::blank(595., 842.).unwrap();
        let replacement = crate::convert::blank(600., 800.).unwrap();
        let budget = original.len() + replacement.len();
        let mut session = Session::new(original.clone().into(), vec![], true);
        session
            .commit_document_with_budget(replacement.clone(), budget, true)
            .unwrap();
        assert!(session.can_undo());
        session.undo();
        assert_eq!(&*session.source, &original);
        assert!(session.can_redo());
        session.redo();
        assert_eq!(&*session.source, &replacement);
        assert!(session.retained_bytes() <= budget);
    }

    #[test]
    fn over_budget_current_document_is_never_discarded() {
        let mark = image(10);
        let mut session = Session::new(vec![0u8; 100].into(), vec![mark.clone()], true);
        let mut next = mark;
        next.rotation = 90.;
        session.commit(vec![next.clone()]).unwrap();
        session.trim_history_to_budget(1);
        assert_eq!(session.annotations, vec![next]);
        assert_eq!(
            session.past.len(),
            1,
            "shared history costs no extra image storage"
        );
        assert!(session.future.is_empty());
        assert_eq!(session.retained_bytes(), 100 + 4096);
    }
}
