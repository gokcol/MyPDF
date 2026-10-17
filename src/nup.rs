//! Multiple source pages on each new sheet. Page content stays vector-based;
//! annotation/widget appearances are flattened, never copied as live signatures.
use anyhow::{Context, Result, ensure};
use hayro::kurbo::Affine;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};

pub const PRESETS: [(usize, usize); 8] = [
    (1, 1),
    (2, 1),
    (2, 2),
    (3, 2),
    (4, 2),
    (3, 3),
    (4, 3),
    (4, 4),
];

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub columns: usize,
    pub rows: usize,
    pub sheet: [f32; 2],
    /// Top, bottom, left, right, in points.
    pub margins: [f32; 4],
    pub gap: f32,
    /// Clip these edges from each displayed source page before fitting it.
    pub trim: [f32; 4],
    pub header: String,
    pub footer: String,
    pub text_size: f32,
    pub watermark: String,
    pub watermark_opacity: f32,
    pub watermark_angle: f32,
}
impl Default for Layout {
    fn default() -> Self {
        Self {
            columns: 2,
            rows: 2,
            sheet: [842., 595.],
            margins: [24.; 4],
            gap: 12.,
            trim: [0.; 4],
            header: String::new(),
            footer: String::new(),
            text_size: 10.,
            watermark: String::new(),
            watermark_opacity: 0.15,
            watermark_angle: 35.,
        }
    }
}
impl Layout {
    pub fn capacity(&self) -> usize {
        self.columns.saturating_mul(self.rows).max(1)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=8).contains(&self.columns) && (1..=8).contains(&self.rows),
            "Choose 1–8 columns and rows"
        );
        ensure!(
            self.sheet
                .iter()
                .all(|v| v.is_finite() && (72. ..=14400.).contains(v)),
            "Sheet dimensions must be 72–14,400 pt"
        );
        ensure!(
            self.margins
                .iter()
                .chain(&self.trim)
                .all(|v| v.is_finite() && (0. ..=2000.).contains(v))
                && self.gap.is_finite()
                && (0. ..=144.).contains(&self.gap),
            "Margins, trimming, and spacing must be nonnegative"
        );
        ensure!(
            self.text_size.is_finite() && (6. ..=48.).contains(&self.text_size),
            "Header/footer size must be 6–48 pt"
        );
        ensure!(
            self.header.chars().count() <= 500
                && self.footer.chars().count() <= 500
                && self.watermark.chars().count() <= 200,
            "Header/footer or watermark text is too long"
        );
        ensure!(
            (0.05..=1.).contains(&self.watermark_opacity)
                && self.watermark_angle.is_finite()
                && self.watermark_angle.abs() <= 180.,
            "Invalid watermark opacity or angle"
        );
        let c = self.cell(0);
        ensure!(
            c[2] >= 12. && c[3] >= 12.,
            "Margins and spacing leave too little space for the pages"
        );
        Ok(())
    }
    /// Cell [left, top, width, height] in displayed sheet coordinates, row first.
    pub fn cell(&self, slot: usize) -> [f32; 4] {
        let [top, bottom, left, right] = self.margins;
        let header = if self.header.trim().is_empty() {
            0.
        } else {
            self.text_size * 1.5 + 8.
        };
        let footer = if self.footer.trim().is_empty() {
            0.
        } else {
            self.text_size * 1.5 + 8.
        };
        let w = (self.sheet[0] - left - right - self.gap * self.columns.saturating_sub(1) as f32)
            / self.columns.max(1) as f32;
        let h = (self.sheet[1]
            - top
            - bottom
            - header
            - footer
            - self.gap * self.rows.saturating_sub(1) as f32)
            / self.rows.max(1) as f32;
        [
            left + (slot % self.columns.max(1)) as f32 * (w + self.gap),
            top + header + (slot / self.columns.max(1)) as f32 * (h + self.gap),
            w,
            h,
        ]
    }
}
fn numbers(values: impl IntoIterator<Item = f32>) -> Object {
    Object::Array(values.into_iter().map(Object::Real).collect())
}

/// Paint visible annotation appearances into the source page before turning it
/// into a Form XObject. Unsupported visible marks fail rather than disappearing.
fn flatten_appearances(doc: &mut Document, page: ObjectId) -> Result<()> {
    let list = doc
        .get_dictionary(page)?
        .get(b"Annots")
        .ok()
        .map(|o| doc.dereference(o).map(|(_, v)| v.clone()))
        .transpose()?
        .and_then(|o| o.as_array().ok().cloned())
        .unwrap_or_default();
    let mut drawn = vec![];
    for object in list {
        let d = doc.dereference(&object)?.1.as_dict()?.clone();
        let subtype = d
            .get(b"Subtype")
            .and_then(Object::as_name)
            .unwrap_or_default();
        if d.get(b"F").and_then(Object::as_i64).unwrap_or(0) & (1 | 2 | 32) != 0
            || matches!(subtype, b"Link" | b"Popup")
        {
            continue;
        }
        let normal = d
            .get(b"AP")
            .ok()
            .and_then(|o| doc.dereference(o).ok()?.1.as_dict().ok())
            .and_then(|ap| ap.get(b"N").ok())
            .cloned();
        let Some(mut normal) = normal else {
            // Empty signature fields have no printed appearance.
            if subtype == b"Widget"
                && d.get(b"FT")
                    .and_then(Object::as_name)
                    .is_ok_and(|v| v == b"Sig")
            {
                continue;
            }
            anyhow::bail!(
                "A visible annotation has no saved appearance. Flatten this PDF before creating the sheet layout."
            );
        };
        if let Object::Dictionary(states) = doc.dereference(&normal)?.1 {
            normal = states
                .get(d.get(b"AS").and_then(Object::as_name).unwrap_or(b"Off"))?
                .clone();
        }
        let stream = doc.dereference(&normal)?.1.as_stream()?.clone();
        let bbox = stream.dict.get(b"BBox")?.as_array()?;
        ensure!(
            bbox.len() == 4 && bbox.iter().all(|n| n.as_float().is_ok_and(f32::is_finite)),
            "Invalid annotation appearance bounds"
        );
        let bbox = bbox
            .iter()
            .map(Object::as_float)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ensure!(
            bbox[2] > bbox[0] && bbox[3] > bbox[1],
            "Empty annotation appearance bounds"
        );
        if let Ok(matrix) = stream.dict.get(b"Matrix") {
            let matrix = matrix
                .as_array()?
                .iter()
                .map(Object::as_float)
                .collect::<std::result::Result<Vec<_>, _>>()?;
            ensure!(
                matrix.len() == 6
                    && matrix.iter().all(|v| v.is_finite())
                    && (matrix[0] * matrix[3] - matrix[1] * matrix[2]).abs() > 1e-12,
                "Invalid annotation appearance transform"
            );
        }
        let rect = d
            .get(b"Rect")?
            .as_array()?
            .iter()
            .map(Object::as_float)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ensure!(
            rect.len() == 4
                && rect.iter().all(|v| v.is_finite())
                && rect[2] > rect[0]
                && rect[3] > rect[1],
            "Invalid annotation rectangle"
        );
        let id = doc.add_object(stream);
        drawn.push((id, [rect[0], rect[1], rect[2], rect[3]]));
    }
    if !drawn.is_empty() {
        crate::form_design::draw_into_page(doc, page, &drawn)?;
    }
    doc.get_dictionary_mut(page)?.remove(b"Annots");
    Ok(())
}

pub fn export(bytes: &[u8], pages: &[usize], layout: &Layout) -> Result<Vec<u8>> {
    layout.validate()?;
    ensure!(
        bytes.len() as u64 <= crate::viewer::MAX_FILE_BYTES,
        "PDF exceeds 256 MB"
    );
    let mut doc = Document::load_mem(bytes)?;
    ensure!(
        !doc.is_encrypted(),
        "Unlock the PDF before exporting a sheet layout"
    );
    let original: Vec<_> = doc.get_pages().into_values().collect();
    ensure!(
        !pages.is_empty() && pages.len() <= 10000 && pages.iter().all(|p| *p < original.len()),
        "Choose valid source pages"
    );
    let geometry = crate::annotations::page_geometry(bytes)?;
    let parent = doc.new_object_id();
    let mut kids = vec![];
    for sheet in pages.chunks(layout.capacity()) {
        let mut resources = Dictionary::new();
        let mut content = String::new();
        for (slot, index) in sheet.iter().copied().enumerate() {
            let page = original[index];
            let (bounds, inverse) = geometry.get(index).context("Page geometry missing")?;
            let [top, bottom, left, right] = layout.trim;
            let (w, h) = (bounds[2] - left - right, bounds[3] - top - bottom);
            ensure!(
                w >= 1. && h >= 1.,
                "Trimming removes all of source page {}",
                index + 1
            );
            flatten_appearances(&mut doc, page)?;
            let forward = Affine::new(*inverse).inverse().as_coeffs();
            let mut body = format!(
                "q {} {} {} {} {} {} cm\n",
                forward[0], forward[1], forward[2], forward[3], forward[4], forward[5]
            )
            .into_bytes();
            for id in crate::header_footer::content_ids(&doc, page)? {
                let stream = doc.get_object(id)?.as_stream()?;
                body.extend(stream.get_plain_content_with_limit(
                    (crate::viewer::MAX_FILE_BYTES as usize).saturating_sub(body.len()),
                )?);
                body.push(b'\n');
            }
            body.extend(b"\nQ\n");
            let mut dict = dictionary! { "Type"=>"XObject", "Subtype"=>"Form", "FormType"=>1, "BBox"=>numbers([left,top,bounds[2]-right,bounds[3]-bottom]), "Resources"=>crate::content::resources(&doc,page)? };
            if let Ok(group) = doc.get_dictionary(page)?.get(b"Group") {
                dict.set("Group", group.clone());
            }
            let mut form = Stream::new(dict, body);
            form.compress()?;
            let id = doc.add_object(form);
            let name = format!("Page{slot}");
            resources.set(name.as_str(), id);
            let [x, y, cw, ch] = layout.cell(slot);
            let scale = (cw / w).min(ch / h);
            let x = x + (cw - w * scale) / 2. - left * scale;
            let y = layout.sheet[1] - y - (ch - h * scale) / 2. + top * scale;
            content.push_str(&format!(
                "q {scale} 0 0 {} {x} {y} cm /{name} Do Q\n",
                -scale
            ));
        }
        let content = doc.add_object(Stream::new(Dictionary::new(), content.into_bytes()));
        kids.push(Object::Reference(doc.add_object(dictionary! { "Type"=>"Page", "Parent"=>parent, "MediaBox"=>numbers([0.,0.,layout.sheet[0],layout.sheet[1]]), "Contents"=>content, "Resources"=>dictionary! {"XObject"=>resources} })));
    }
    doc.objects.insert(
        parent,
        dictionary! { "Type"=>"Pages", "Count"=>kids.len() as i64, "Kids"=>kids }.into(),
    );
    let mut catalog = dictionary! { "Type"=>"Catalog", "Pages"=>parent };
    for key in [b"OCProperties".as_slice(), b"OutputIntents".as_slice()] {
        if let Ok(value) = doc.catalog()?.get(key) {
            catalog.set(key, value.clone());
        }
    }
    let catalog = doc.add_object(catalog);
    doc.trailer = dictionary! { "Root"=>catalog };
    let mut out = crate::content::serialize(&mut doc)?;
    for (text, position) in [
        (&layout.header, crate::header_footer::Position::TopCenter),
        (&layout.footer, crate::header_footer::Position::BottomCenter),
    ] {
        if !text.trim().is_empty() {
            out = crate::header_footer::apply(
                &out,
                &crate::header_footer::Stamp {
                    template: text.clone(),
                    position,
                    margins: layout.margins,
                    size: layout.text_size,
                    ..Default::default()
                },
            )?
            .0;
        }
    }
    if !layout.watermark.trim().is_empty() {
        out = crate::watermark::apply(
            &out,
            &crate::watermark::Watermark {
                mark: crate::watermark::Mark::Text {
                    text: layout.watermark.clone(),
                    color: [0.3; 3],
                    font: None,
                },
                opacity: layout.watermark_opacity,
                angle: layout.watermark_angle,
                behind: false,
                ..Default::default()
            },
        )?
        .0;
    }
    Ok(out)
}
