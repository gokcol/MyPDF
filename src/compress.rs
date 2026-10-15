//! Making a PDF smaller: photos and scans are scaled down to what the chosen
//! quality needs and stored as JPEG, uncompressed data is compressed, and
//! unused objects are dropped. Text and drawings stay as they are.
use anyhow::{Result, ensure};
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// About 200 dpi on a full page: fine for printing.
    High,
    /// About 150 dpi: good on screen and for email.
    Medium,
    /// About 100 dpi: the smallest file.
    Small,
}
impl Level {
    pub const ALL: [Level; 3] = [Level::High, Level::Medium, Level::Small];
    pub fn label(self) -> &'static str {
        match self {
            Level::High => "High quality",
            Level::Medium => "Balanced",
            Level::Small => "Smallest file",
        }
    }
    pub fn detail(self) -> &'static str {
        match self {
            Level::High => "Good for printing · pictures at about 200 dpi",
            Level::Medium => "Good for screens and email · about 150 dpi",
            Level::Small => "Good for quick sharing · about 100 dpi",
        }
    }
    /// The longest side, in pixels, a picture filling an A4 page keeps.
    fn longest(self) -> u32 {
        match self {
            Level::High => 2340,
            Level::Medium => 1755,
            Level::Small => 1170,
        }
    }
    fn quality(self) -> u8 {
        match self {
            Level::High => 85,
            Level::Medium => 72,
            Level::Small => 55,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub before: usize,
    pub after: usize,
    /// Pictures stored again, smaller.
    pub images: usize,
}

fn resolve<'a>(doc: &'a Document, o: &'a Object) -> &'a Object {
    doc.dereference(o).map(|(_, o)| o).unwrap_or(o)
}
/// The number of colour components a picture's colour space has, for the
/// spaces that can be stored as JPEG: gray and RGB.
fn components(doc: &Document, d: &Dictionary) -> Option<u8> {
    match resolve(doc, d.get(b"ColorSpace").ok()?) {
        Object::Name(n) if n == b"DeviceRGB" => Some(3),
        Object::Name(n) if n == b"DeviceGray" => Some(1),
        Object::Array(a) if a.len() == 2 && a[0].as_name().ok() == Some(b"ICCBased".as_slice()) => {
            let profile = resolve(doc, &a[1]).as_stream().ok()?;
            match profile.dict.get(b"N").and_then(Object::as_i64).ok()? {
                3 => Some(3),
                1 => Some(1),
                _ => None,
            }
        }
        _ => None,
    }
}
/// A picture, smaller, as JPEG data with its new width and height.
fn smaller(doc: &Document, id: ObjectId, level: Level) -> Option<(Vec<u8>, u32, u32)> {
    let stream = doc.get_object(id).ok()?.as_stream().ok()?;
    let d = &stream.dict;
    if d.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Image".as_slice())
        || d.get(b"ImageMask")
            .and_then(Object::as_bool)
            .unwrap_or(false)
        || d.has(b"Decode")
        || d.get(b"BitsPerComponent").and_then(Object::as_i64).ok() != Some(8)
    {
        return None;
    }
    let n = components(doc, d)?;
    let w = u32::try_from(d.get(b"Width").and_then(Object::as_i64).ok()?).ok()?;
    let h = u32::try_from(d.get(b"Height").and_then(Object::as_i64).ok()?).ok()?;
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 100_000_000 {
        return None;
    }
    let filters: Vec<Vec<u8>> = match d.get(b"Filter").map(|f| resolve(doc, f)) {
        Ok(Object::Name(f)) => vec![f.clone()],
        Ok(Object::Array(a)) => a
            .iter()
            .filter_map(|f| f.as_name().ok().map(<[u8]>::to_vec))
            .collect(),
        _ => vec![],
    };
    let image: image::DynamicImage = match filters.as_slice() {
        [f] if f == b"DCTDecode" => {
            let decoded =
                image::load_from_memory_with_format(&stream.content, image::ImageFormat::Jpeg)
                    .ok()?;
            // A JPEG whose colours do not match its colour space is left alone.
            match (n, decoded.color().channel_count()) {
                (3, 3) | (1, 1) => decoded,
                _ => return None,
            }
        }
        [] | [_] if filters.is_empty() || filters[0] == b"FlateDecode" => {
            if d.has(b"DecodeParms") {
                return None;
            }
            let raw = if filters.is_empty() {
                stream.content.clone()
            } else {
                stream.decompressed_content().ok()?
            };
            let expected = w as usize * h as usize * n as usize;
            if raw.len() < expected {
                return None;
            }
            let raw = raw[..expected].to_vec();
            if n == 3 {
                image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, raw)?)
            } else {
                image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(w, h, raw)?)
            }
        }
        _ => return None,
    };
    let longest = level.longest();
    let image = if w.max(h) > longest {
        let scale = longest as f32 / w.max(h) as f32;
        let (nw, nh) = (
            ((w as f32 * scale).round() as u32).max(1),
            ((h as f32 * scale).round() as u32).max(1),
        );
        image.resize_exact(nw, nh, image::imageops::FilterType::Triangle)
    } else {
        image
    };
    let mut jpeg = vec![];
    let mut encoder =
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, level.quality());
    match &image {
        image::DynamicImage::ImageLuma8(gray) => encoder.encode_image(gray).ok()?,
        other => encoder.encode_image(&other.to_rgb8()).ok()?,
    }
    // Only worth it when clearly smaller.
    (jpeg.len() * 10 < stream.content.len() * 9).then_some((jpeg, image.width(), image.height()))
}

/// A smaller copy of the PDF at `level`, and what changed.
pub fn reduce(bytes: &[u8], level: Level) -> Result<(Vec<u8>, Report)> {
    let mut doc = crate::content::editable(bytes)?;
    // Transparency masks keep their own sharpness.
    let masks: HashSet<ObjectId> = doc
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .flat_map(|s| [s.dict.get(b"SMask"), s.dict.get(b"Mask")])
        .filter_map(|m| m.ok()?.as_reference().ok())
        .collect();
    let ids: Vec<ObjectId> = doc
        .objects
        .iter()
        .filter(|(id, o)| !masks.contains(id) && o.as_stream().is_ok())
        .map(|(id, _)| *id)
        .collect();
    let mut images = 0;
    for id in ids {
        if let Some((jpeg, w, h)) = smaller(&doc, id, level) {
            let stream = doc.get_object_mut(id)?.as_stream_mut()?;
            stream.dict.set("Filter", "DCTDecode");
            stream.dict.remove(b"DecodeParms");
            stream.dict.set("Width", w as i64);
            stream.dict.set("Height", h as i64);
            stream.set_content(jpeg);
            images += 1;
        } else {
            let stream = doc.get_object_mut(id)?.as_stream_mut()?;
            if !stream.dict.has(b"Filter") && stream.content.len() > 64 {
                let _ = stream.compress();
            }
        }
    }
    // Page thumbnails are redrawn by every reader anyway.
    for page in doc.get_pages().into_values().collect::<Vec<_>>() {
        doc.get_dictionary_mut(page)?.remove(b"Thumb");
    }
    let out = crate::content::serialize(&mut doc)?;
    ensure!(
        out.len() < bytes.len(),
        "This PDF is already about as small as it can be at this quality"
    );
    Ok((
        out.clone(),
        Report {
            before: bytes.len(),
            after: out.len(),
            images,
        },
    ))
}
