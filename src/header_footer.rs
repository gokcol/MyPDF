//! Header and footer text: page numbers, Bates numbers and free text placed in
//! the margins of a page range, and removed again by kind.
//!
//! Each stamp is its own content stream, marked as a pagination artifact with a
//! `/MyPDFHeaderFooter` kind both on the stream and in its marked content, so it
//! can be found again even after another edit merges the page's streams.
use anyhow::{Context as _, Result, ensure};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const MARK: &[u8] = b"MyPDFHeaderFooter";
const WRAP: &str = "Wrap";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    #[default]
    Text,
    PageNumber,
    Bates,
}
impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Text, Kind::PageNumber, Kind::Bates];
    fn tag(self) -> &'static str {
        match self {
            Kind::Text => "Text",
            Kind::PageNumber => "PageNumber",
            Kind::Bates => "Bates",
        }
    }
    fn from_tag(tag: &[u8]) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.tag().as_bytes() == tag)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Position {
    TopLeft,
    TopCenter,
    TopRight,
    #[default]
    BottomLeft,
    BottomCenter,
    BottomRight,
}
impl Position {
    pub const ALL: [Position; 6] = [
        Position::TopLeft,
        Position::TopCenter,
        Position::TopRight,
        Position::BottomLeft,
        Position::BottomCenter,
        Position::BottomRight,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Position::TopLeft => "Top Left",
            Position::TopCenter => "Top Center",
            Position::TopRight => "Top Right",
            Position::BottomLeft => "Bottom Left",
            Position::BottomCenter => "Bottom Center",
            Position::BottomRight => "Bottom Right",
        }
    }
    pub fn top(self) -> bool {
        matches!(
            self,
            Position::TopLeft | Position::TopCenter | Position::TopRight
        )
    }
    /// -1 left, 0 center, 1 right.
    pub fn column(self) -> i8 {
        match self {
            Position::TopLeft | Position::BottomLeft => -1,
            Position::TopCenter | Position::BottomCenter => 0,
            Position::TopRight | Position::BottomRight => 1,
        }
    }
    /// Which margins apply: top, bottom, left, right.
    pub fn margins_used(self) -> [bool; 4] {
        [
            self.top(),
            !self.top(),
            self.column() == -1,
            self.column() == 1,
        ]
    }
}

/// What to place, where, and on which pages.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stamp {
    pub kind: Kind,
    /// Text with optional `{page}`, `{pages}`, `{bates}`, `{date}` and `{file}` fields.
    pub template: String,
    /// Zero-based, inclusive page range; `None` for every page.
    pub pages: Option<(usize, usize)>,
    pub position: Position,
    /// Distances from the page edges in points: top, bottom, left, right.
    pub margins: [f32; 4],
    pub font: Option<PathBuf>,
    pub size: f32,
    pub color: [f32; 3],
    /// The number on the first stamped page.
    pub start: u32,
    /// Bates number width, padded with zeros.
    pub digits: u8,
    #[serde(skip)]
    pub file: String,
    #[serde(skip)]
    pub date: String,
}
impl Default for Stamp {
    fn default() -> Self {
        const MM: f32 = 72. / 25.4;
        Self {
            kind: Kind::Text,
            template: String::new(),
            pages: None,
            position: Position::BottomLeft,
            margins: [15. * MM, 10. * MM, 15. * MM, 15. * MM],
            font: None,
            size: 10.,
            color: [0.; 3],
            start: 1,
            digits: 6,
            file: String::new(),
            date: String::new(),
        }
    }
}
impl Stamp {
    /// The text for the `ordinal`-th stamped page of `count`.
    pub fn text_for(&self, ordinal: usize, count: usize) -> String {
        let number = self.start as u64 + ordinal as u64;
        let last = self.start as u64 + count.saturating_sub(1) as u64;
        self.template
            .replace("{page}", &number.to_string())
            .replace("{pages}", &last.to_string())
            .replace(
                "{bates}",
                &format!("{number:0width$}", width = self.digits as usize),
            )
            .replace("{date}", &self.date)
            .replace("{file}", &self.file)
    }
    fn validate(&self, count: usize) -> Result<(usize, usize)> {
        ensure!(
            !self.template.trim().is_empty() && self.template.chars().count() <= 500,
            "Enter 1–500 characters of text"
        );
        ensure!(
            self.size.is_finite() && (4.0..=144.0).contains(&self.size),
            "Font size must be 4–144 pt"
        );
        ensure!(
            self.margins
                .iter()
                .all(|m| m.is_finite() && (0.0..=2000.0).contains(m)),
            "Margins must be between 0 and 2,000 pt"
        );
        ensure!(
            self.color
                .iter()
                .all(|c| c.is_finite() && (0.0..=1.0).contains(c)),
            "Invalid text color"
        );
        ensure!((1..=12).contains(&self.digits), "Use 1–12 Bates digits");
        let (first, last) = self.pages.unwrap_or((0, count.saturating_sub(1)));
        ensure!(
            count > 0 && first <= last && last < count,
            "Choose pages between 1 and {count}"
        );
        Ok((first, last))
    }
}

/// Place a stamp on every page of its range. Returns the new bytes and the number of pages.
pub fn apply(bytes: &[u8], stamp: &Stamp) -> Result<(Vec<u8>, usize)> {
    let mut doc = crate::content::editable(bytes)?;
    let pages = doc.get_pages();
    let (first, last) = stamp.validate(pages.len())?;
    let geometry = crate::annotations::page_geometry(bytes)?;
    let preferred = match &stamp.font {
        Some(path) => crate::unicode_text::Preferred::File(path.clone()),
        None => crate::unicode_text::Preferred::File(crate::document::default_font()?),
    };
    let mut writer = crate::unicode_text::Writer::new(&preferred)?;
    let count = last - first + 1;
    // Shape every page first so each font is embedded once, for all pages.
    let lines = (first..=last)
        .enumerate()
        .map(|(ordinal, page)| {
            let text = stamp
                .text_for(ordinal, count)
                .replace(['\r', '\n', '\t'], " ");
            ensure!(!text.trim().is_empty(), "The text is empty");
            Ok((page, writer.shape(&text)?))
        })
        .collect::<Result<Vec<_>>>()?;
    let fonts = writer.embed(&mut doc)?;
    let size = stamp.size;
    let [top, bottom, left, right] = stamp.margins;
    let wrap = |doc: &mut Document, content: Vec<u8>| {
        doc.add_object(Stream::new(dictionary! { MARK => WRAP }, content))
    };
    for (page, line) in lines {
        let id = *pages.get(&(page as u32 + 1)).context("Page not found")?;
        let (page_size, transform) = geometry.get(page).context("Page geometry missing")?;
        let (w, h) = (page_size[2], page_size[3]);
        let width = line.width(size);
        let x = match stamp.position.column() {
            -1 => left,
            0 => (w - width) / 2.,
            _ => w - right - width,
        };
        let baseline = if stamp.position.top() {
            top + size * 0.8
        } else {
            h - bottom - size * 0.22
        };
        ensure!(
            x >= -0.1
                && x + width <= w + 0.1
                && baseline - size * 0.8 >= -0.1
                && baseline + size * 0.22 <= h + 0.1,
            "The text does not fit on page {}. Use shorter text, a smaller size or smaller margins.",
            page + 1
        );
        let mut resources = crate::content::resources(&doc, id)?;
        let mut font_dict = resources
            .get(b"Font")
            .ok()
            .map(|o| doc.dereference(o))
            .transpose()?
            .map(|(_, o)| o.as_dict().cloned())
            .transpose()?
            .unwrap_or_default();
        let mut suffix = 0;
        let names: Vec<Option<String>> = fonts
            .iter()
            .map(|font| {
                font.map(|font| {
                    while font_dict.has(format!("MyPDFHF{suffix}").as_bytes()) {
                        suffix += 1;
                    }
                    let name = format!("MyPDFHF{suffix}");
                    font_dict.set(name.as_str(), font);
                    name
                })
            })
            .collect();
        resources.set("Font", font_dict);
        let subtype = if stamp.position.top() {
            "Header"
        } else {
            "Footer"
        };
        let mut operations = vec![
            Operation::new(
                "BDC",
                vec![
                    Object::Name(b"Artifact".to_vec()),
                    Object::Dictionary(dictionary! {
                        "Type" => "Pagination",
                        "Subtype" => subtype,
                        MARK => stamp.kind.tag(),
                    }),
                ],
            ),
            Operation::new("q", vec![]),
            Operation::new(
                "cm",
                transform.iter().map(|v| Object::Real(*v as f32)).collect(),
            ),
            Operation::new("BT", vec![]),
            Operation::new("rg", stamp.color.iter().map(|v| Object::Real(*v)).collect()),
            // Display coordinates have y down; flip glyphs back upright.
            Operation::new(
                "Tm",
                vec![
                    1.into(),
                    0.into(),
                    0.into(),
                    (-1).into(),
                    x.into(),
                    baseline.into(),
                ],
            ),
        ];
        operations.extend(writer.operations(&line, &names, size, 0., 0.)?);
        operations.extend([
            Operation::new("ET", vec![]),
            Operation::new("Q", vec![]),
            Operation::new("EMC", vec![]),
        ]);
        let stamp_stream = doc.add_object(Stream::new(
            dictionary! { MARK => stamp.kind.tag() },
            Content { operations }.encode()?,
        ));
        let mut contents = content_ids(&doc, id)?;
        if !contents.is_empty() {
            // Isolate the page's own graphics state, even if it is unbalanced.
            let (before, after) = isolation(&doc, id)?;
            let before = wrap(&mut doc, before);
            let after = wrap(&mut doc, after);
            contents.insert(0, before);
            contents.push(after);
        }
        contents.push(stamp_stream);
        let dict = doc.get_object_mut(id)?.as_dict_mut()?;
        dict.set("Resources", resources);
        dict.set(
            "Contents",
            contents
                .into_iter()
                .map(Object::Reference)
                .collect::<Vec<_>>(),
        );
    }
    Ok((crate::content::serialize(&mut doc)?, count))
}

/// Pages that carry each kind of stamp.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub text: usize,
    pub page_numbers: usize,
    pub bates: usize,
}
impl Summary {
    pub fn any(&self) -> bool {
        self.text + self.page_numbers + self.bates > 0
    }
}

pub fn summary(bytes: &[u8]) -> Result<Summary> {
    let doc = Document::load_mem(bytes)?;
    let mut summary = Summary::default();
    for id in doc.get_pages().into_values() {
        let mut kinds = [false; 3];
        for stream in content_ids(&doc, id)? {
            let Ok(stream) = doc.get_object(stream).and_then(Object::as_stream) else {
                continue;
            };
            if let Some(kind) = stream_kind(stream) {
                kinds[kind as usize] = true;
            } else if let Some(content) = marked_content(stream) {
                for op in &content.operations {
                    if let Some(kind) = marked_kind(op) {
                        kinds[kind as usize] = true;
                    }
                }
            }
        }
        summary.text += kinds[Kind::Text as usize] as usize;
        summary.page_numbers += kinds[Kind::PageNumber as usize] as usize;
        summary.bates += kinds[Kind::Bates as usize] as usize;
    }
    Ok(summary)
}

/// Remove stamps of the given kinds from every page. Returns the new bytes and
/// the number of pages that changed.
pub fn remove(bytes: &[u8], kinds: &[Kind]) -> Result<(Vec<u8>, usize)> {
    let mut doc = crate::content::editable(bytes)?;
    let mut changed = 0;
    for id in doc.get_pages().into_values() {
        let mut kept = vec![];
        let mut touched = false;
        let mut stamps_left = false;
        for stream_id in content_ids(&doc, id)? {
            let Ok(stream) = doc.get_object(stream_id).and_then(Object::as_stream) else {
                kept.push(stream_id);
                continue;
            };
            match stream_kind(stream) {
                Some(kind) if kinds.contains(&kind) => {
                    touched = true;
                    continue;
                }
                Some(_) => stamps_left = true,
                None => {}
            }
            if stream_kind(stream).is_none()
                && let Some(content) = marked_content(stream)
            {
                let (operations, removed, left) = strip(content.operations, kinds);
                stamps_left |= left;
                if removed {
                    touched = true;
                    let mut replacement = stream.clone();
                    replacement.set_plain_content(Content { operations }.encode()?);
                    doc.objects.insert(stream_id, Object::Stream(replacement));
                }
            }
            kept.push(stream_id);
        }
        if !touched {
            continue;
        }
        changed += 1;
        // The isolating q/Q pairs go with the last stamp.
        if !stamps_left {
            kept.retain(|s| {
                doc.get_object(*s)
                    .and_then(Object::as_stream)
                    .ok()
                    .and_then(|s| s.dict.get(MARK).ok())
                    .and_then(|v| v.as_name().ok())
                    != Some(WRAP.as_bytes())
            });
        }
        doc.get_object_mut(id)?.as_dict_mut()?.set(
            "Contents",
            kept.into_iter().map(Object::Reference).collect::<Vec<_>>(),
        );
    }
    Ok((crate::content::serialize(&mut doc)?, changed))
}

pub(crate) fn content_ids(doc: &Document, page: ObjectId) -> Result<Vec<ObjectId>> {
    Ok(match doc.get_dictionary(page)?.get(b"Contents") {
        Ok(Object::Reference(id)) => match doc.get_object(*id)? {
            Object::Array(items) => items.iter().filter_map(|o| o.as_reference().ok()).collect(),
            _ => vec![*id],
        },
        Ok(Object::Array(items)) => items.iter().filter_map(|o| o.as_reference().ok()).collect(),
        _ => vec![],
    })
}

/// Content that opens enough `q` to absorb stray `Q`, and closes what the page left open.
pub(crate) fn isolation(doc: &Document, page: ObjectId) -> Result<(Vec<u8>, Vec<u8>)> {
    let content = doc.get_and_decode_page_content(page)?;
    let (mut depth, mut lowest, mut in_text) = (0i64, 0i64, false);
    for op in &content.operations {
        match op.operator.as_str() {
            "q" => depth += 1,
            "Q" => {
                depth -= 1;
                lowest = lowest.min(depth);
            }
            "BT" => in_text = true,
            "ET" => in_text = false,
            _ => {}
        }
    }
    let open = 1 - lowest;
    let before = "q\n".repeat(open as usize).into_bytes();
    let mut after = String::from("\n");
    if in_text {
        after.push_str("ET\n");
    }
    after.push_str(&"Q\n".repeat((open + depth) as usize));
    Ok((before, after.into_bytes()))
}

fn stream_kind(stream: &Stream) -> Option<Kind> {
    Kind::from_tag(stream.dict.get(MARK).ok()?.as_name().ok()?)
}

/// The decoded content of a stream that mentions a stamp, if any.
fn marked_content(stream: &Stream) -> Option<Content> {
    let raw = stream
        .decompressed_content()
        .unwrap_or_else(|_| stream.content.clone());
    raw.windows(MARK.len())
        .any(|w| w == MARK)
        .then(|| Content::decode(&raw).ok())
        .flatten()
}

fn marked_kind(op: &Operation) -> Option<Kind> {
    if op.operator != "BDC" {
        return None;
    }
    let dict: &Dictionary = op.operands.get(1)?.as_dict().ok()?;
    Kind::from_tag(dict.get(MARK).ok()?.as_name().ok()?)
}

/// Drop marked stamps of the given kinds: (operations, removed any, others remain).
fn strip(operations: Vec<Operation>, kinds: &[Kind]) -> (Vec<Operation>, bool, bool) {
    let mut kept = Vec::with_capacity(operations.len());
    let (mut skipping, mut removed, mut left) = (0usize, false, false);
    for op in operations {
        if skipping > 0 {
            match op.operator.as_str() {
                "BDC" | "BMC" => skipping += 1,
                "EMC" => skipping -= 1,
                _ => {}
            }
            continue;
        }
        match marked_kind(&op) {
            Some(kind) if kinds.contains(&kind) => {
                skipping = 1;
                removed = true;
                continue;
            }
            Some(_) => left = true,
            None => {}
        }
        kept.push(op);
    }
    (kept, removed, left)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_fill_per_page() {
        let stamp = Stamp {
            template: "Page {page} of {pages} · {bates} · {file} · {date}".into(),
            start: 3,
            digits: 4,
            file: "Report.pdf".into(),
            date: "2026-10-14".into(),
            ..Default::default()
        };
        assert_eq!(
            stamp.text_for(1, 5),
            "Page 4 of 7 · 0004 · Report.pdf · 2026-10-14"
        );
    }

    #[test]
    fn unbalanced_content_is_isolated() {
        let mut doc = Document::with_version("1.7");
        let content = doc.add_object(Stream::new(dictionary! {}, b"Q Q q q q BT".to_vec()));
        let page = doc.add_object(dictionary! { "Type" => "Page", "Contents" => content });
        let (before, after) = isolation(&doc, page).unwrap();
        let ops = |b: &[u8]| Content::decode(b).unwrap().operations;
        let mut all = ops(&before);
        all.extend(ops(b"Q Q q q q BT"));
        all.extend(ops(&after));
        let mut depth = 0i64;
        for op in &all {
            match op.operator.as_str() {
                "q" => depth += 1,
                "Q" => depth -= 1,
                _ => {}
            }
            assert!(depth >= 0);
        }
        assert_eq!(depth, 0);
        assert_eq!(all.iter().filter(|o| o.operator == "ET").count(), 1);
    }
}
