//! Embed one portable Unicode font for annotation appearance streams.
use anyhow::{Context, Result, ensure};
use lopdf::{Document, Object, ObjectId, Stream, dictionary};
use std::collections::{BTreeMap, BTreeSet};

pub struct PdfFont {
    pub id: ObjectId,
    cids: BTreeMap<char, u16>,
    widths: BTreeMap<char, f32>,
}
impl PdfFont {
    pub fn embed(doc: &mut Document, text: &str) -> Result<Self> {
        Self::embed_file(doc, text, &crate::document::default_font()?)
    }
    pub fn embed_file(doc: &mut Document, text: &str, path: &std::path::Path) -> Result<Self> {
        let font_bytes = std::fs::read(path)?;
        Self::embed_bytes(doc, text, font_bytes)
    }
    pub fn embed_bytes(doc: &mut Document, text: &str, font_bytes: Vec<u8>) -> Result<Self> {
        let face =
            ttf_parser::Face::parse(&font_bytes, 0).context("Could not read annotation font")?;
        let chars: BTreeSet<_> = text
            .chars()
            .filter(|c| !c.is_control())
            .chain(" ?".chars())
            .collect();
        ensure!(
            chars.len() < 65_000,
            "Too many characters in annotation text"
        );
        let mut cids = BTreeMap::new();
        let mut widths = BTreeMap::new();
        let mut cid_map = vec![0_u8, 0];
        let mut w = vec![];
        let mut cmap = String::from(
            "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /MyPDFUnicode def /CMapType 2 def 1 begincodespacerange <0000> <FFFF> endcodespacerange\n",
        );
        let chars: Vec<_> = chars.into_iter().collect();
        for (i, c) in chars.iter().copied().enumerate() {
            let cid = (i + 1) as u16;
            let glyph = face.glyph_index(c).with_context(|| format!("The annotation font does not contain '{c}'. Choose different text or import artwork."))?;
            let width = face
                .glyph_hor_advance(glyph)
                .unwrap_or(face.units_per_em() / 2) as f32
                * 1000.0
                / face.units_per_em() as f32;
            cids.insert(c, cid);
            widths.insert(c, width);
            cid_map.extend(glyph.0.to_be_bytes());
            w.push(Object::Integer(cid as i64));
            w.push(Object::Array(vec![Object::Real(width)]));
        }
        for (chunk, values) in chars.chunks(100).enumerate() {
            cmap.push_str(&format!("{} beginbfchar\n", values.len()));
            for (index, c) in values.iter().copied().enumerate() {
                let encoded: String = c
                    .encode_utf16(&mut [0; 2])
                    .iter()
                    .map(|u| format!("{u:04X}"))
                    .collect();
                cmap.push_str(&format!(
                    "<{:04X}> <{}>\n",
                    chunk * 100 + index + 1,
                    encoded
                ));
            }
            cmap.push_str("endbfchar\n");
        }
        cmap.push_str("endcmap CMapName currentdict /CMap defineresource pop end end");
        let upem = face.units_per_em() as f32;
        let bbox = face.global_bounding_box();
        let scale = |v: i16| v as f32 * 1000.0 / upem;
        let ascent = scale(face.ascender());
        let descent = scale(face.descender());
        let bbox = vec![
            scale(bbox.x_min).into(),
            scale(bbox.y_min).into(),
            scale(bbox.x_max).into(),
            scale(bbox.y_max).into(),
        ];
        let font_name = face
            .names()
            .into_iter()
            .find(|n| n.name_id == 6)
            .and_then(|n| n.to_string())
            .unwrap_or_else(|| "MyPDFUnicode".into());
        let italic_angle = face.italic_angle();
        let flags =
            32 | if face.is_italic() { 64 } else { 0 } | if face.is_monospaced() { 1 } else { 0 };
        let stem = if face.is_bold() { 120 } else { 80 };
        ensure!(
            face.tables().glyf.is_some(),
            "Choose a TrueType font with glyph outlines"
        );
        let font_file = doc.add_object(Stream::new(
            dictionary! { "Length1" => font_bytes.len() as i64 },
            font_bytes,
        ));
        let descriptor = doc.add_object(dictionary! { "Type" => "FontDescriptor", "FontName" => Object::Name(font_name.as_bytes().to_vec()), "Flags" => flags, "FontBBox" => Object::Array(bbox), "ItalicAngle" => italic_angle, "Ascent" => ascent, "Descent" => descent, "CapHeight" => ascent, "StemV" => stem, "FontFile2" => font_file });
        let cid_map = doc.add_object(Stream::new(dictionary! {}, cid_map));
        let cid = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "CIDFontType2", "BaseFont" => Object::Name(font_name.as_bytes().to_vec()), "CIDSystemInfo" => dictionary! { "Registry" => Object::string_literal("Adobe"), "Ordering" => Object::string_literal("Identity"), "Supplement" => 0 }, "FontDescriptor" => descriptor, "DW" => 1000, "W" => Object::Array(w), "CIDToGIDMap" => cid_map });
        let unicode = doc.add_object(Stream::new(dictionary! {}, cmap.into_bytes()));
        let id = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type0", "BaseFont" => Object::Name(font_name.as_bytes().to_vec()), "Encoding" => "Identity-H", "DescendantFonts" => vec![Object::Reference(cid)], "ToUnicode" => unicode });
        Ok(Self { id, cids, widths })
    }
    pub fn hex(&self, text: &str) -> Result<String> {
        text.chars()
            .map(|c| {
                self.cids
                    .get(&c)
                    .map(|v| format!("{v:04X}"))
                    .with_context(|| format!("Unsupported text character '{c}'"))
            })
            .collect()
    }
    pub fn width(&self, text: &str, size: f32) -> f32 {
        text.chars()
            .map(|c| self.widths.get(&c).copied().unwrap_or(500.0))
            .sum::<f32>()
            * size
            / 1000.0
    }
    pub fn wrap_preserving(&self, text: &str, size: f32, width: f32) -> Vec<String> {
        let mut lines = vec![];
        for paragraph in text.split('\n') {
            let mut line = String::new();
            for token in paragraph.split_inclusive(' ') {
                if !line.trim().is_empty()
                    && self.width((line.clone() + token).trim_end(), size) > width
                {
                    lines.push(line.trim_end().to_owned());
                    line = token.to_owned();
                } else {
                    line.push_str(token);
                }
            }
            lines.push(line);
        }
        lines
    }
    pub fn wrap(&self, text: &str, size: f32, width: f32) -> Vec<String> {
        let mut lines = vec![];
        for paragraph in text.split('\n') {
            let mut line = String::new();
            for word in paragraph.split_whitespace() {
                let candidate = if line.is_empty() {
                    word.to_owned()
                } else {
                    format!("{line} {word}")
                };
                if !line.is_empty() && self.width(&candidate, size) > width {
                    lines.push(line);
                    line = word.into();
                } else {
                    line = candidate;
                }
            }
            lines.push(line);
        }
        lines
    }
}
