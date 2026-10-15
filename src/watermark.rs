//! Watermarks: text such as CONFIDENTIAL or DRAFT, or a picture, across the
//! middle of each page, see-through and turned, in front of or behind the page.
//!
//! Each watermark is its own content stream, marked `/MyPDFWatermark` on the
//! stream and in its marked content, so it can be removed again even after
//! another edit merges the page's streams.
use crate::header_footer::{content_ids, isolation};
use anyhow::{Context as _, Result, ensure};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use std::path::PathBuf;

const MARK: &[u8] = b"MyPDFWatermark";
const WRAP: &str = "Wrap";

/// Common watermark words.
pub const PRESETS: [&str; 6] = [
    "CONFIDENTIAL",
    "DRAFT",
    "COPY",
    "SAMPLE",
    "DO NOT COPY",
    "APPROVED",
];

#[derive(Clone, Debug, PartialEq)]
pub enum Mark {
    Text {
        text: String,
        color: [f32; 3],
        /// A font file; the default face when empty.
        font: Option<PathBuf>,
    },
    /// A PNG or JPEG picture.
    Image {
        bytes: Vec<u8>,
        /// Its width as a share of the page width.
        width: f32,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Watermark {
    pub mark: Mark,
    /// Text size in points; 0 fits the text across the page.
    pub size: f32,
    /// 0.05 (faint) to 1 (solid).
    pub opacity: f32,
    /// Degrees, counterclockwise.
    pub angle: f32,
    /// Behind the page's own content rather than over it.
    pub behind: bool,
    /// First and last page, from 0; every page when absent.
    pub pages: Option<(usize, usize)>,
}
impl Default for Watermark {
    fn default() -> Self {
        Self {
            mark: Mark::Text {
                text: PRESETS[0].into(),
                color: [0.75, 0.12, 0.12],
                font: None,
            },
            size: 0.,
            opacity: 0.25,
            angle: 45.,
            behind: false,
            pages: None,
        }
    }
}

/// Where a line of `width` (at size 1) fits across a `w` × `h` page at `angle`.
fn fitted_size(width: f32, w: f32, h: f32, angle: f32) -> f32 {
    let (sin, cos) = angle.to_radians().sin_cos();
    let reach = match (cos.abs() < 1e-3, sin.abs() < 1e-3) {
        (true, _) => h,
        (_, true) => w,
        _ => (w / cos.abs()).min(h / sin.abs()),
    };
    (0.8 * reach / width.max(1e-3))
        .min(0.3 * w.min(h))
        .clamp(6., 300.)
}

/// Add a watermark to the chosen pages. Returns the new bytes and the number of pages.
pub fn apply(bytes: &[u8], watermark: &Watermark) -> Result<(Vec<u8>, usize)> {
    ensure!(
        (0.05..=1.).contains(&watermark.opacity)
            && watermark.angle.is_finite()
            && watermark.angle.abs() <= 360.
            && (0. ..=300.).contains(&watermark.size),
        "Choose an opacity of 5–100 % and an angle between −360° and 360°"
    );
    let mut doc = crate::content::editable(bytes)?;
    let pages = doc.get_pages();
    let (first, last) = watermark
        .pages
        .unwrap_or((0, pages.len().saturating_sub(1)));
    ensure!(
        !pages.is_empty() && first <= last && last < pages.len(),
        "Choose pages between 1 and {}",
        pages.len()
    );
    let geometry = crate::annotations::page_geometry(bytes)?;
    // What is drawn: shaped text and its fonts, or an embedded picture.
    enum Drawn {
        Text {
            line: crate::unicode_text::Line,
            writer: crate::unicode_text::Writer,
            fonts: Vec<Option<lopdf::ObjectId>>,
            color: [f32; 3],
        },
        Image {
            id: lopdf::ObjectId,
            aspect: f32,
            width: f32,
        },
    }
    let drawn = match &watermark.mark {
        Mark::Text { text, color, font } => {
            let text = text.replace(['\r', '\n', '\t'], " ");
            ensure!(
                !text.trim().is_empty() && text.chars().count() <= 200,
                "Write the watermark text (up to 200 characters)"
            );
            ensure!(
                color.iter().all(|c| (0. ..=1.).contains(c)),
                "Invalid color"
            );
            let preferred = crate::unicode_text::Preferred::File(match font {
                Some(path) => path.clone(),
                None => crate::document::default_font()?,
            });
            let mut writer = crate::unicode_text::Writer::new(&preferred)?;
            let line = writer.shape(text.trim())?;
            let fonts = writer.embed(&mut doc)?;
            Drawn::Text {
                line,
                writer,
                fonts,
                color: *color,
            }
        }
        Mark::Image {
            bytes: picture,
            width,
        } => {
            ensure!(
                (0.05..=1.).contains(width),
                "Choose a picture width of 5–100 % of the page"
            );
            let decoded = crate::content::decode_image(picture)?;
            let aspect = decoded.width() as f32 / decoded.height().max(1) as f32;
            Drawn::Image {
                id: crate::content::embed_image(&mut doc, &decoded)?,
                aspect,
                width: *width,
            }
        }
    };
    let state = doc.add_object(dictionary! {
        "Type" => "ExtGState",
        "ca" => Object::Real(watermark.opacity),
        "CA" => Object::Real(watermark.opacity),
    });
    let kind = match drawn {
        Drawn::Text { .. } => "Text",
        Drawn::Image { .. } => "Image",
    };
    let (sin, cos) = watermark.angle.to_radians().sin_cos();
    for page in first..=last {
        let id = *pages.get(&(page as u32 + 1)).context("Page not found")?;
        let (page_box, transform) = geometry.get(page).context("Page geometry missing")?;
        let (w, h) = (page_box[2], page_box[3]);
        let (cx, cy) = (w / 2., h / 2.);
        let mut resources = crate::content::resources(&doc, id)?;
        let sub = |resources: &mut Dictionary,
                   doc: &Document,
                   kind: &[u8],
                   prefix: &str,
                   value: lopdf::ObjectId|
         -> String {
            let mut dict = resources
                .get(kind)
                .ok()
                .and_then(|o| doc.dereference(o).ok())
                .and_then(|(_, o)| o.as_dict().ok().cloned())
                .unwrap_or_default();
            let name = (0..)
                .map(|n| format!("{prefix}{n}"))
                .find(|n| !dict.has(n.as_bytes()))
                .unwrap();
            dict.set(name.as_str(), value);
            resources.set(kind, dict);
            name
        };
        let gs = sub(&mut resources, &doc, b"ExtGState", "MyPDFWM", state);
        let mut operations = vec![
            Operation::new(
                "BDC",
                vec![
                    Object::Name(b"Artifact".to_vec()),
                    Object::Dictionary(dictionary! {
                        "Type" => "Pagination",
                        "Subtype" => "Watermark",
                        MARK => kind,
                    }),
                ],
            ),
            Operation::new("q", vec![]),
            Operation::new("gs", vec![Object::Name(gs.into_bytes())]),
            // Drawn in displayed coordinates, whatever the page's rotation.
            Operation::new(
                "cm",
                transform.iter().map(|v| Object::Real(*v as f32)).collect(),
            ),
        ];
        match &drawn {
            Drawn::Text {
                line,
                writer,
                fonts,
                color,
            } => {
                let names: Vec<Option<String>> = fonts
                    .iter()
                    .map(|font| {
                        font.map(|font| sub(&mut resources, &doc, b"Font", "MyPDFWMF", font))
                    })
                    .collect();
                let size = if watermark.size > 0. {
                    watermark.size
                } else {
                    fitted_size(line.width(1.), w, h, watermark.angle)
                };
                let width = line.width(size);
                // Centre the line on the page: half its width back along it,
                // and half the capital height down from its baseline.
                let rise = size * 0.35;
                let x = cx - width / 2. * cos + rise * sin;
                let y = cy + width / 2. * sin + rise * cos;
                operations.extend([
                    Operation::new("BT", vec![]),
                    Operation::new("rg", color.iter().map(|v| Object::Real(*v)).collect()),
                    // Turned counterclockwise as seen, in a y-down space, glyphs upright.
                    Operation::new(
                        "Tm",
                        vec![
                            Object::Real(cos),
                            Object::Real(-sin),
                            Object::Real(-sin),
                            Object::Real(-cos),
                            Object::Real(x),
                            Object::Real(y),
                        ],
                    ),
                ]);
                operations.extend(writer.operations(line, &names, size, 0., 0.)?);
                operations.push(Operation::new("ET", vec![]));
            }
            Drawn::Image {
                id: image,
                aspect,
                width,
            } => {
                let name = sub(&mut resources, &doc, b"XObject", "MyPDFWMI", *image);
                let iw = w * width;
                let ih = iw / aspect;
                // The picture's unit square, centred and turned; its top row stays at the top.
                operations.push(Operation::new(
                    "cm",
                    vec![
                        Object::Real(iw * cos),
                        Object::Real(-iw * sin),
                        Object::Real(-ih * sin),
                        Object::Real(-ih * cos),
                        Object::Real(cx - 0.5 * iw * cos + 0.5 * ih * sin),
                        Object::Real(cy + 0.5 * iw * sin + 0.5 * ih * cos),
                    ],
                ));
                operations.push(Operation::new("Do", vec![Object::Name(name.into_bytes())]));
            }
        }
        operations.extend([Operation::new("Q", vec![]), Operation::new("EMC", vec![])]);
        let stamp = doc.add_object(Stream::new(
            dictionary! { MARK => kind },
            Content { operations }.encode()?,
        ));
        let mut contents = content_ids(&doc, id)?;
        if !contents.is_empty() {
            // Isolate the page's own graphics state, even if it is unbalanced.
            let (before, after) = isolation(&doc, id)?;
            let before = doc.add_object(Stream::new(dictionary! { MARK => WRAP }, before));
            let after = doc.add_object(Stream::new(dictionary! { MARK => WRAP }, after));
            contents.insert(0, before);
            contents.push(after);
        }
        if watermark.behind {
            contents.insert(0, stamp);
        } else {
            contents.push(stamp);
        }
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
    Ok((crate::content::serialize(&mut doc)?, last - first + 1))
}

fn marked(op: &Operation) -> bool {
    op.operator == "BDC"
        && op
            .operands
            .get(1)
            .and_then(|o| o.as_dict().ok())
            .is_some_and(|d| d.has(MARK))
}
fn stamp_stream(stream: &Stream) -> Option<bool> {
    let kind = stream.dict.get(MARK).ok()?.as_name().ok()?;
    Some(kind != WRAP.as_bytes())
}
fn mentions(stream: &Stream) -> bool {
    let raw = stream
        .decompressed_content()
        .unwrap_or_else(|_| stream.content.clone());
    raw.windows(MARK.len()).any(|w| w == MARK)
}

/// The number of pages carrying a watermark added by MyPDF.
pub fn count(bytes: &[u8]) -> Result<usize> {
    let doc = Document::load_mem(bytes)?;
    let mut pages = 0;
    for id in doc.get_pages().into_values() {
        let found = content_ids(&doc, id)?.into_iter().any(|s| {
            doc.get_object(s)
                .and_then(Object::as_stream)
                .is_ok_and(|s| {
                    stamp_stream(s) == Some(true) || stamp_stream(s).is_none() && mentions(s)
                })
        });
        pages += found as usize;
    }
    Ok(pages)
}

/// Remove every watermark MyPDF added. Returns the new bytes and the pages changed.
pub fn remove(bytes: &[u8]) -> Result<(Vec<u8>, usize)> {
    let mut doc = crate::content::editable(bytes)?;
    let mut changed = 0;
    for id in doc.get_pages().into_values() {
        let mut kept = vec![];
        let mut touched = false;
        for stream_id in content_ids(&doc, id)? {
            let Ok(stream) = doc.get_object(stream_id).and_then(Object::as_stream) else {
                kept.push(stream_id);
                continue;
            };
            match stamp_stream(stream) {
                Some(true) => {
                    touched = true;
                    continue;
                }
                Some(false) => {}
                // Another edit may have merged the watermark into the page's content.
                None if mentions(stream) => {
                    let content = Content::decode(
                        &stream
                            .decompressed_content()
                            .unwrap_or_else(|_| stream.content.clone()),
                    )?;
                    let mut operations = Vec::with_capacity(content.operations.len());
                    let mut skipping = 0usize;
                    for op in content.operations {
                        if skipping > 0 {
                            match op.operator.as_str() {
                                "BDC" | "BMC" => skipping += 1,
                                "EMC" => skipping -= 1,
                                _ => {}
                            }
                            continue;
                        }
                        if marked(&op) {
                            skipping = 1;
                            touched = true;
                            continue;
                        }
                        operations.push(op);
                    }
                    let mut replacement = stream.clone();
                    replacement.set_plain_content(Content { operations }.encode()?);
                    doc.objects.insert(stream_id, Object::Stream(replacement));
                }
                None => {}
            }
            kept.push(stream_id);
        }
        if !touched {
            continue;
        }
        changed += 1;
        // The isolating pairs go with the watermark.
        kept.retain(|s| {
            doc.get_object(*s)
                .and_then(Object::as_stream)
                .ok()
                .and_then(stamp_stream)
                != Some(false)
        });
        doc.get_object_mut(id)?.as_dict_mut()?.set(
            "Contents",
            kept.into_iter().map(Object::Reference).collect::<Vec<_>>(),
        );
    }
    ensure!(changed > 0, "This PDF has no watermark added by MyPDF");
    Ok((crate::content::serialize(&mut doc)?, changed))
}
