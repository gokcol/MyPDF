//! Bounded edits to page content. Shared streams/resources are copied before editing.
use anyhow::{Context, Result, ensure};
use hayro::kurbo::{Affine, Point, Rect};
use lopdf::{
    Dictionary, Document, Object, ObjectId, Stream,
    content::{Content, Operation},
    dictionary,
};
use std::collections::HashSet;

pub fn editable(bytes: &[u8]) -> Result<Document> {
    ensure!(
        bytes.len() as u64 <= crate::viewer::MAX_FILE_BYTES,
        "PDF exceeds 256 MB"
    );
    let doc = Document::load_mem(bytes)?;
    let caps = crate::annotations::inspect(&doc);
    ensure!(
        !caps.signed && !caps.certified && !caps.encrypted,
        "Editing requires an unsigned, unencrypted PDF. The original is preserved."
    );
    Ok(doc)
}
pub fn serialize(doc: &mut Document) -> Result<Vec<u8>> {
    doc.prune_objects();
    doc.reference_table.cross_reference_type = lopdf::xref::XrefType::CrossReferenceTable;
    doc.trailer.remove(b"Prev");
    doc.trailer.remove(b"XRefStm");
    let mut bytes = vec![];
    doc.save_to(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= crate::viewer::MAX_FILE_BYTES,
        "Result exceeds 256 MB"
    );
    Ok(bytes)
}
pub fn inherited(doc: &Document, mut id: ObjectId, key: &[u8]) -> Result<Option<Object>> {
    let mut seen = HashSet::new();
    while seen.insert(id) && seen.len() < 128 {
        let d = doc.get_dictionary(id)?;
        if let Ok(o) = d.get(key) {
            return Ok(Some(doc.dereference(o)?.1.clone()));
        }
        match d.get(b"Parent").and_then(Object::as_reference) {
            Ok(p) => id = p,
            Err(_) => return Ok(None),
        }
    }
    anyhow::bail!("Invalid PDF inheritance tree")
}
pub fn resources(doc: &Document, page: ObjectId) -> Result<Dictionary> {
    Ok(inherited(doc, page, b"Resources")?
        .unwrap_or(Object::Dictionary(Dictionary::new()))
        .as_dict()?
        .clone())
}
#[derive(Clone, Debug)]
pub struct ImageObject {
    pub index: usize,
    pub name: String,
    pub pixels: [i64; 2],
    pub rect: [f32; 4],
    pub matrix: [f64; 6],
}
#[derive(Clone, Debug)]
pub enum ImageEdit {
    Replace(Vec<u8>),
    Place([f32; 4]),
    Delete,
}

/// Add selectable PDF text in displayed-page coordinates, including blank,
/// cropped and rotated pages. Existing streams/resources remain isolated.
pub fn add_text(
    bytes: &[u8],
    page: u32,
    text: &str,
    rect: [f32; 4],
    size: f32,
    color: [f32; 3],
) -> Result<Vec<u8>> {
    add_text_styled(bytes, page, text, rect, size, color, &Default::default())
}
pub fn add_text_styled(
    bytes: &[u8],
    page: u32,
    text: &str,
    rect: [f32; 4],
    size: f32,
    color: [f32; 3],
    format: &crate::text_style::TextFormat,
) -> Result<Vec<u8>> {
    ensure!(
        !text.trim().is_empty() && text.len() <= 32_768,
        "Enter 1–32,768 bytes of text"
    );
    ensure!(
        size.is_finite() && (4.0..=144.0).contains(&size),
        "Font size must be 4–144 pt"
    );
    ensure!(
        rect.iter().all(|v| v.is_finite())
            && color
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
        "Invalid text position or color"
    );
    let mut doc = editable(bytes)?;
    let id = *doc.get_pages().get(&page).context("Page not found")?;
    let geometry = crate::annotations::page_geometry(bytes)?;
    let (page_size, transform) = geometry
        .get(page.saturating_sub(1) as usize)
        .context("Page geometry missing")?;
    let [x, y, w, h] = rect;
    ensure!(
        x >= 0.
            && y >= 0.
            && w >= 10.
            && h >= size
            && x + w <= page_size[2] + 0.1
            && y + h <= page_size[3] + 0.1,
        "Text block must fit inside the page"
    );
    let normalized = text.replace("\r\n", "\n").replace('\t', "    ");
    let text = normalized.as_str();
    let font = if let Some(path) = &format.font {
        crate::pdf_font::PdfFont::embed_file(&mut doc, text, path)?
    } else {
        crate::pdf_font::PdfFont::embed(&mut doc, text)?
    };
    let lines = font.wrap_preserving(text, size, w);
    ensure!(
        lines.len() as f32 * size * 1.3 <= h,
        "Text does not fit. Enlarge the block or use a smaller font."
    );
    ensure!(
        lines.iter().all(|line| font.width(line, size) <= w),
        "A word is too wide. Enlarge the block or add a line break."
    );
    let mut resources = resources(&doc, id)?;
    let mut fonts = resources
        .get(b"Font")
        .ok()
        .map(|o| doc.dereference(o))
        .transpose()?
        .map(|(_, o)| o.as_dict().cloned())
        .transpose()?
        .unwrap_or_default();
    let mut suffix = doc.max_id;
    while fonts.has(format!("MyPDFText{suffix}").as_bytes()) {
        suffix += 1;
    }
    let name = format!("MyPDFText{suffix}");
    fonts.set(name.as_str(), font.id);
    resources.set("Font", fonts);
    // Isolate even an unbalanced original CTM/text state by requiring balanced
    // q/Q and text objects, then wrapping original content in a fresh q/Q pair.
    let old = doc.get_and_decode_page_content(id)?;
    let mut depth = 0usize;
    let mut in_text = false;
    for op in &old.operations {
        match op.operator.as_str() {
            "q" => depth += 1,
            "Q" => {
                ensure!(depth > 0, "Page has unbalanced graphics state");
                depth -= 1;
            }
            "BT" => {
                ensure!(!in_text, "Page has nested text objects");
                in_text = true;
            }
            "ET" => {
                ensure!(in_text, "Page has unbalanced text objects");
                in_text = false;
            }
            _ => {}
        }
    }
    ensure!(depth == 0 && !in_text, "Page content is unbalanced");
    let mut operations = vec![Operation::new("q", vec![])];
    operations.extend(old.operations);
    operations.push(Operation::new("Q", vec![]));
    operations.push(Operation::new("q", vec![]));
    operations.push(Operation::new(
        "cm",
        transform.iter().map(|v| Object::Real(*v as f32)).collect(),
    ));
    operations.push(Operation::new("BT", vec![]));
    operations.push(Operation::new(
        "Tf",
        vec![Object::Name(name.into_bytes()), size.into()],
    ));
    operations.push(Operation::new(
        "rg",
        color.iter().map(|v| Object::Real(*v)).collect(),
    ));
    let paragraph_ends: HashSet<_> = text
        .split('\n')
        .scan(0_usize, |end, p| {
            *end += font.wrap_preserving(p, size, w).len();
            Some(*end - 1)
        })
        .collect();
    // Display coordinates have y down; flip glyphs back to an upright baseline.
    for (index, line) in lines.iter().enumerate() {
        operations.push(Operation::new(
            "Tm",
            vec![
                1.into(),
                0.into(),
                0.into(),
                (-1).into(),
                (x + match format.alignment {
                    crate::text_style::Alignment::Center => (w - font.width(line, size)) / 2.,
                    crate::text_style::Alignment::Right => w - font.width(line, size),
                    _ => 0.,
                })
                .into(),
                (y + size + index as f32 * size * 1.3).into(),
            ],
        ));
        let hex = font.hex(line)?;
        let encoded = hex
            .as_bytes()
            .chunks_exact(2)
            .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
            .collect::<Vec<_>>();
        let spaces = line.chars().filter(|c| *c == ' ').count();
        let ends_paragraph = paragraph_ends.contains(&index);
        if format.alignment == crate::text_style::Alignment::Justified
            && !ends_paragraph
            && spaces > 0
        {
            let gap = (w - font.width(line, size)) / spaces as f32 * 1000. / size;
            let mut parts = vec![];
            for (ch, cid) in line.chars().zip(encoded.chunks_exact(2)) {
                parts.push(Object::String(
                    cid.to_vec(),
                    lopdf::StringFormat::Hexadecimal,
                ));
                if ch == ' ' {
                    parts.push(Object::Real(-gap));
                }
            }
            operations.push(Operation::new("TJ", vec![Object::Array(parts)]));
        } else {
            operations.push(Operation::new(
                "Tj",
                vec![Object::String(encoded, lopdf::StringFormat::Hexadecimal)],
            ));
        }
    }
    operations.push(Operation::new("ET", vec![]));
    operations.push(Operation::new("Q", vec![]));
    let stream = doc.add_object(Stream::new(
        dictionary! {},
        Content { operations }.encode()?,
    ));
    let page = doc.get_object_mut(id)?.as_dict_mut()?;
    page.set("Resources", resources);
    page.set("Contents", stream);
    serialize(&mut doc)
}

pub fn images(bytes: &[u8], page: u32) -> Result<Vec<ImageObject>> {
    let doc = Document::load_mem(bytes)?;
    let id = *doc.get_pages().get(&page).context("Page not found")?;
    let resources = resources(&doc, id)?;
    let xo = resources
        .get(b"XObject")
        .ok()
        .map(|o| doc.dereference(o))
        .transpose()?
        .map(|(_, o)| o.as_dict())
        .transpose()?;
    let geometry = crate::annotations::page_geometry(bytes)?;
    let display = Affine::new(
        geometry
            .get(page.saturating_sub(1) as usize)
            .context("Page geometry missing")?
            .1,
    )
    .inverse();
    let mut ctm = Affine::IDENTITY;
    let mut stack = vec![];
    let mut result = vec![];
    let content = doc.get_and_decode_page_content(id)?;
    ensure!(
        content.operations.len() < 1_000_000,
        "Page content is too complex"
    );
    for (index, op) in content.operations.iter().enumerate() {
        match op.operator.as_str() {
            "q" => {
                ensure!(stack.len() < 256, "Graphics state is too deep");
                stack.push(ctm);
            }
            "Q" => ctm = stack.pop().context("Unbalanced graphics state")?,
            "cm" => {
                ensure!(op.operands.len() == 6, "Invalid image transform");
                let mut v = [0.0; 6];
                for (v, o) in v.iter_mut().zip(&op.operands) {
                    *v = o.as_float()? as f64;
                }
                ensure!(v.iter().all(|v| v.is_finite()), "Invalid image transform");
                ctm *= Affine::new(v);
            }
            "Do" => {
                let name = op
                    .operands
                    .first()
                    .context("Missing image name")?
                    .as_name()?;
                if let Some(xo) = xo
                    && let Ok(o) = xo.get(name)
                {
                    let stream = doc.dereference(o)?.1.as_stream()?;
                    if stream
                        .dict
                        .get(b"Subtype")
                        .and_then(Object::as_name)
                        .is_ok_and(|v| v == b"Image")
                    {
                        ensure!(result.len() < 4096, "Too many images on this page");
                        let r = (display * ctm).transform_rect_bbox(Rect::new(0., 0., 1., 1.));
                        ensure!(
                            ctm.determinant().abs() > 1e-10,
                            "Image transform cannot be edited"
                        );
                        result.push(ImageObject {
                            index,
                            name: String::from_utf8_lossy(name).into(),
                            pixels: [
                                stream.dict.get(b"Width")?.as_i64()?,
                                stream.dict.get(b"Height")?.as_i64()?,
                            ],
                            rect: [r.x0 as f32, r.y0 as f32, r.x1 as f32, r.y1 as f32],
                            matrix: ctm.as_coeffs(),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    Ok(result)
}

/// Decode bounded PNG/JPEG artwork without external resources.
pub fn decode_image(bytes: &[u8]) -> Result<image::RgbaImage> {
    ensure!(bytes.len() <= 32 * 1024 * 1024, "Image exceeds 32 MB");
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    ensure!(
        matches!(
            reader.format(),
            Some(image::ImageFormat::Png | image::ImageFormat::Jpeg)
        ),
        "Choose a PNG or JPEG image"
    );
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode()?.to_rgba8();
    ensure!(
        image.width() as u64 * image.height() as u64 <= 24_000_000,
        "Image exceeds 24 million pixels"
    );
    Ok(image)
}
pub fn embed_image(doc: &mut Document, image: &image::RgbaImage) -> Result<ObjectId> {
    let mut rgb = Vec::with_capacity(image.len() / 4 * 3);
    let mut alpha = Vec::with_capacity(image.len() / 4);
    for p in image.pixels() {
        rgb.extend_from_slice(&p.0[..3]);
        alpha.push(p[3]);
    }
    let mut d = dictionary! {"Type"=>"XObject","Subtype"=>"Image","Width"=>image.width() as i64,"Height"=>image.height() as i64,"ColorSpace"=>"DeviceRGB","BitsPerComponent"=>8};
    if alpha.iter().any(|a| *a != 255) {
        let mut mask = Stream::new(
            dictionary! {"Type"=>"XObject","Subtype"=>"Image","Width"=>image.width() as i64,"Height"=>image.height() as i64,"ColorSpace"=>"DeviceGray","BitsPerComponent"=>8},
            alpha,
        );
        mask.compress()?;
        d.set("SMask", doc.add_object(mask));
    }
    let mut s = Stream::new(d, rgb);
    s.compress()?;
    Ok(doc.add_object(s))
}
pub fn edit_image(bytes: &[u8], page: u32, index: usize, edit: &ImageEdit) -> Result<Vec<u8>> {
    let mut doc = editable(bytes)?;
    let selected = images(bytes, page)?
        .into_iter()
        .find(|i| i.index == index)
        .context("Image no longer exists")?;
    let id = *doc.get_pages().get(&page).context("Page missing")?;
    let mut content = doc.get_and_decode_page_content(id)?;
    match edit {
        ImageEdit::Delete => {
            content.operations.remove(index);
        }
        ImageEdit::Replace(bytes) => {
            let image = decode_image(bytes)?;
            let image_id = embed_image(&mut doc, &image)?;
            let mut res = resources(&doc, id)?;
            let mut xo = doc.dereference(res.get(b"XObject")?)?.1.as_dict()?.clone();
            let mut n = 0;
            let name = loop {
                let name = format!("MyPDFImage{n}");
                if !xo.has(name.as_bytes()) {
                    break name;
                }
                n += 1;
            };
            xo.set(name.as_str(), image_id);
            res.set("XObject", xo);
            doc.get_dictionary_mut(id)?.set("Resources", res);
            content.operations[index].operands = vec![Object::Name(name.into_bytes())];
        }
        ImageEdit::Place(r) => {
            ensure!(
                r.iter().all(|v| v.is_finite() && v.abs() < 100_000.)
                    && r[2] - r[0] >= 1.
                    && r[3] - r[1] >= 1.,
                "Image must be at least 1 point wide and tall"
            );
            let old = selected.rect;
            let w = old[2] - old[0];
            let h = old[3] - old[1];
            ensure!(w > 0.001 && h > 0.001, "Image has no editable extent");
            let inverse =
                Affine::new(crate::annotations::page_geometry(bytes)?[(page - 1) as usize].1);
            let change = Affine::translate((r[0] as f64, r[1] as f64))
                * Affine::scale_non_uniform(((r[2] - r[0]) / w) as f64, ((r[3] - r[1]) / h) as f64)
                * Affine::translate((-old[0] as f64, -old[1] as f64));
            let original = Affine::new(selected.matrix);
            let delta = original.inverse() * inverse * change * inverse.inverse() * original;
            let ops = vec![
                Operation::new("q", vec![]),
                Operation::new(
                    "cm",
                    delta
                        .as_coeffs()
                        .into_iter()
                        .map(|v| Object::Real(v as f32))
                        .collect(),
                ),
                content.operations[index].clone(),
                Operation::new("Q", vec![]),
            ];
            content.operations.splice(index..=index, ops);
        }
    }
    let stream = doc.add_object(Stream::new(
        Dictionary::new(),
        Content {
            operations: content.operations,
        }
        .encode()?,
    ));
    doc.get_dictionary_mut(id)?.set("Contents", stream);
    serialize(&mut doc)
}
/// Transform a PDF widget rectangle into displayed page coordinates.
pub fn displayed_rect(rect: [f32; 4], inverse: [f64; 6]) -> [f32; 4] {
    let m = Affine::new(inverse).inverse();
    let a = m * Point::new(rect[0] as f64, rect[1] as f64);
    let b = m * Point::new(rect[2] as f64, rect[3] as f64);
    [
        a.x.min(b.x) as f32,
        a.y.min(b.y) as f32,
        a.x.max(b.x) as f32,
        a.y.max(b.y) as f32,
    ]
}
