use anyhow::{Context, Result, bail, ensure};
use hayro::{
    PixmapSettings, RenderCache, RenderSettings, hayro_interpret::InterpreterSettings,
    hayro_syntax::Pdf,
};
use lopdf::{
    Dictionary, Document, Object, Stream, StringFormat,
    content::{Content, Operation},
    dictionary,
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::Instant,
};

pub const ORIGINAL: &str = "Invoice total: 1250.00 EUR";
pub const REPLACEMENT: &str = "Invoice total: 1450.00 EUR";
pub const TURKISH_ORIGINAL: &str = "İstanbul: sözleşme gözden geçirildi.";
pub const TURKISH_REPLACEMENT: &str = "İstanbul: sözleşme onaylandı.";

#[derive(Clone, Debug, Serialize)]
pub struct TextRun {
    pub operator_index: usize,
    pub operator: String,
    pub font: String,
    pub base_font: String,
    pub text: String,
    pub rect: Option<[f32; 4]>,
    pub size: f32,
    pub color: [f32; 3],
}

pub fn has_signatures(doc: &Document) -> bool {
    doc.objects.values().any(|o| {
        o.as_dict().is_ok_and(|d| {
            d.get(b"Type")
                .and_then(Object::as_name)
                .is_ok_and(|v| v == b"Sig")
                || (d.get(b"ByteRange").is_ok() && d.get(b"Contents").is_ok())
        })
    })
}

fn shown_bytes(op: &Operation) -> Result<Option<Vec<u8>>> {
    Ok(match op.operator.as_str() {
        "Tj" | "'" => Some(
            op.operands
                .first()
                .context("missing text operand")?
                .as_str()?
                .to_vec(),
        ),
        "\"" => Some(
            op.operands
                .get(2)
                .context("missing quoted text operand")?
                .as_str()?
                .to_vec(),
        ),
        "TJ" => Some(
            op.operands
                .first()
                .context("missing TJ operand")?
                .as_array()?
                .iter()
                .filter_map(|o| o.as_str().ok())
                .flatten()
                .copied()
                .collect(),
        ),
        _ => None,
    })
}

pub fn text_runs(bytes: &[u8], page: u32) -> Result<Vec<TextRun>> {
    let doc = Document::load_mem(bytes)?;
    let page_id = *doc
        .get_pages()
        .get(&page)
        .context("page numbers start at 1")?;
    let fonts = doc.get_page_fonts(page_id)?;
    let encodings = fonts
        .iter()
        .map(|(name, font)| Ok((name.clone(), font.get_font_encoding(&doc)?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    let content = doc.get_and_decode_page_content(page_id)?;
    let mut font = Vec::new();
    let mut size = 14.;
    let mut color = [0.; 3];
    let mut stack = Vec::new();
    let mut runs = Vec::new();
    for (i, op) in content.operations.iter().enumerate() {
        match op.operator.as_str() {
            "Tf" => {
                font = op
                    .operands
                    .first()
                    .context("missing font")?
                    .as_name()?
                    .to_vec();
                size = op
                    .operands
                    .get(1)
                    .and_then(|o| o.as_float().ok())
                    .unwrap_or(14.);
            }
            "rg" if op.operands.len() == 3 => {
                for (out, v) in color.iter_mut().zip(&op.operands) {
                    *out = v.as_float().unwrap_or(0.);
                }
            }
            "g" => {
                color = [op
                    .operands
                    .first()
                    .and_then(|o| o.as_float().ok())
                    .unwrap_or(0.); 3];
            }
            "k" if op.operands.len() == 4 => {
                let k = op.operands[3].as_float().unwrap_or(0.);
                for (out, v) in color.iter_mut().zip(&op.operands) {
                    *out = 1. - (v.as_float().unwrap_or(0.) + k).min(1.);
                }
            }
            "q" => stack.push((font.clone(), size, color)),
            "Q" => (font, size, color) = stack.pop().context("unbalanced graphics state")?,
            _ => {}
        }
        if let Some(raw) = shown_bytes(op)? {
            let encoding = encodings.get(&font).context("unsupported font encoding")?;
            runs.push(TextRun {
                operator_index: i,
                operator: op.operator.clone(),
                font: String::from_utf8_lossy(&font).into(),
                base_font: fonts
                    .get(&font)
                    .and_then(|f| f.get(b"BaseFont").ok())
                    .and_then(|n| n.as_name().ok())
                    .map(|n| String::from_utf8_lossy(n).into_owned())
                    .unwrap_or_default(),
                text: encoding.bytes_to_string(&raw)?,
                rect: None,
                size,
                color,
            });
        }
    }
    Ok(runs)
}

/// Bounded editing: replace exactly one complete page-level text-showing run.
/// Encoding round trips prevent silently dropping glyphs. A fresh stream prevents
/// accidental changes to other pages sharing the old content stream.
pub fn replace_run(bytes: &[u8], page: u32, find: &str, replacement: &str) -> Result<Vec<u8>> {
    ensure!(!find.is_empty(), "search text must not be empty");
    ensure!(
        !replacement.contains(['\n', '\r']),
        "paragraph reflow is outside this proof"
    );
    let runs = text_runs(bytes, page)?;
    let matches: Vec<_> = runs.iter().filter(|r| r.text == find).collect();
    ensure!(
        matches.len() == 1,
        "Expected one complete text run; found {}",
        matches.len()
    );
    replace_text_object(bytes, page, matches[0].operator_index, find, replacement)
}

/// Edit a particular occurrence, even when the same words occur more than once.
pub fn replace_text_object(
    bytes: &[u8],
    page: u32,
    index: usize,
    expected: &str,
    replacement: &str,
) -> Result<Vec<u8>> {
    replace_text_object_reporting(bytes, page, index, expected, replacement).map(|(b, _)| b)
}

/// Like [`replace_text_object`], also returning the installed font families that
/// set the text when the PDF's own font cannot draw it (empty otherwise).
///
/// The run keeps its font when that font's program really contains every glyph.
/// Otherwise it is re-set with [`crate::unicode_text`]: the closest installed
/// match to the original font, falling back per character across installed
/// fonts, shaped and embedded as a subset, so any Unicode text can be written.
pub fn replace_text_object_reporting(
    bytes: &[u8],
    page: u32,
    index: usize,
    expected: &str,
    replacement: &str,
) -> Result<(Vec<u8>, Vec<String>)> {
    ensure!(
        replacement.len() <= 32_768 && !replacement.contains(['\n', '\r']),
        "Use a single text run of at most 32 KB; paragraph reflow is not supported"
    );
    let mut doc = crate::content::editable(bytes)?;
    let page_id = *doc
        .get_pages()
        .get(&page)
        .context("Page numbers start at 1")?;
    let mut content = doc.get_and_decode_page_content(page_id)?;
    let runs = text_runs(bytes, page)?;
    let selected = runs
        .iter()
        .find(|r| r.operator_index == index)
        .context("Text object no longer exists")?;
    ensure!(
        selected.text == expected,
        "The text changed; select it again before editing"
    );
    let seen: std::collections::HashSet<char> = runs
        .iter()
        .filter(|r| r.font == selected.font)
        .flat_map(|r| r.text.chars())
        .collect();
    let own = {
        let fonts = doc.get_page_fonts(page_id)?;
        let font = fonts
            .get(selected.font.as_bytes())
            .context("font disappeared")?;
        let encoding = font.get_font_encoding(&doc)?;
        let encoded = encoding.string_to_bytes(replacement);
        (encoding.bytes_to_string(&encoded)? == replacement
            && crate::unicode_text::missing_glyphs(
                &doc,
                font,
                &selected.base_font,
                &seen,
                replacement,
            )
            .is_empty())
        .then_some(encoded)
    };
    let first = selected.operator_index;
    let mut families = vec![];
    let inserted = match own {
        Some(encoded) => {
            let op = &mut content.operations[first];
            let value = Object::String(encoded, StringFormat::Hexadecimal);
            match op.operator.as_str() {
                "TJ" => op.operands = vec![Object::Array(vec![value])],
                "\"" => op.operands[2] = value,
                _ => op.operands[0] = value,
            }
            1
        }
        None => {
            use crate::unicode_text::{Preferred, Writer};
            let mut writer = Writer::new(&Preferred::LikePdfFont(selected.base_font.clone()))?;
            let line = writer.shape(replacement)?;
            let ids = writer.embed(&mut doc)?;
            let mut resources = crate::content::resources(&doc, page_id)?;
            let mut fonts = doc
                .dereference(resources.get(b"Font")?)?
                .1
                .as_dict()?
                .clone();
            let mut suffix = doc.max_id;
            let names: Vec<Option<String>> = ids
                .iter()
                .map(|id| {
                    id.map(|id| {
                        while fonts.has(format!("MyPDFText{suffix}").as_bytes()) {
                            suffix += 1;
                        }
                        let name = format!("MyPDFText{suffix}");
                        fonts.set(name.as_str(), id);
                        name
                    })
                })
                .collect();
            resources.set("Font", fonts);
            doc.get_dictionary_mut(page_id)?.set("Resources", resources);
            let rise = content.operations[..first]
                .iter()
                .rev()
                .find(|op| op.operator == "Ts")
                .and_then(|op| op.operands.first()?.as_float().ok())
                .unwrap_or(0.);
            let op = &content.operations[first];
            // Quote operators move to the next line (and set spacing) before showing.
            let mut ops = match op.operator.as_str() {
                "'" => vec![Operation::new("T*", vec![])],
                "\"" => vec![
                    Operation::new("Tw", vec![op.operands[0].clone()]),
                    Operation::new("Tc", vec![op.operands[1].clone()]),
                    Operation::new("T*", vec![]),
                ],
                _ => vec![],
            };
            ops.extend(writer.operations(&line, &names, selected.size, 0., rise)?);
            ops.push(Operation::new(
                "Tf",
                vec![
                    Object::Name(selected.font.as_bytes().to_vec()),
                    selected.size.into(),
                ],
            ));
            let count = ops.len();
            content.operations.splice(first..first + 1, ops);
            families = writer.families();
            count
        }
    };
    let stream = doc.add_object(Stream::new(Dictionary::new(), content.encode()?));
    doc.get_dictionary_mut(page_id)?.set("Contents", stream);
    let output = crate::content::serialize(&mut doc)?;
    // Read the new text back. Right-to-left and complex scripts extract in visual
    // or cluster order, so for those every character must be present instead.
    let shown: String = text_runs(&output, page)?
        .iter()
        .filter(|r| (first..first + inserted).contains(&r.operator_index))
        .map(|r| r.text.as_str())
        .collect();
    let written = crate::unicode_text::visible(replacement);
    let sorted = |text: &str| {
        // Glyphs inside reordered clusters may have no Unicode value of their own.
        let mut chars: Vec<char> = text
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '\u{FFFD}')
            .collect();
        chars.sort_unstable();
        chars.dedup();
        chars
    };
    ensure!(
        shown == written
            || crate::unicode_text::complex(replacement) && sorted(&shown) == sorted(&written),
        "saved replacement failed verification"
    );
    Ok((output, families))
}

#[derive(Debug, Serialize)]
pub struct RenderEvidence {
    pub page: usize,
    pub width: u32,
    pub height: u32,
    pub milliseconds: u128,
    pub output: String,
}

pub fn render(bytes: &[u8], out: &Path, scale: f32) -> Result<Vec<RenderEvidence>> {
    ensure!(
        scale.is_finite() && scale > 0.0 && scale <= 4.0,
        "scale must be in (0, 4]"
    );
    let pdf = Pdf::new(bytes.to_vec()).map_err(|e| anyhow::anyhow!("Hayro parse failed: {e:?}"))?;
    std::fs::create_dir_all(out)?;
    let cache = RenderCache::new();
    let mut evidence = Vec::new();
    for (i, page) in pdf.pages().iter().enumerate() {
        let (w, h) = page.render_dimensions();
        ensure!(
            w.is_finite()
                && h.is_finite()
                && w > 0.0
                && h > 0.0
                && w * scale <= 8192.0
                && h * scale <= 8192.0
                && w * h * scale * scale <= 32_000_000.0,
            "page exceeds proof rendering memory budget"
        );
        let start = Instant::now();
        let pixmap = hayro::render(
            page,
            &cache,
            &InterpreterSettings::default(),
            &RenderSettings::default(),
            &PixmapSettings {
                x_scale: scale,
                y_scale: scale,
                bg_color: hayro::vello_cpu::color::palette::css::WHITE,
            },
        );
        let path = out.join(format!("page-{}.png", i + 1));
        crate::write_new(&path, &pixmap.into_png()?)?;
        evidence.push(RenderEvidence {
            page: i + 1,
            width: (w * scale) as u32,
            height: (h * scale) as u32,
            milliseconds: start.elapsed().as_millis(),
            output: path.display().to_string(),
        });
    }
    Ok(evidence)
}

fn op(name: &str, operands: Vec<Object>) -> Operation {
    Operation::new(name, operands)
}
fn line(font: &str, size: i64, y: i64, text: Object) -> Vec<Operation> {
    vec![
        op("BT", vec![]),
        op("Tf", vec![font.into(), size.into()]),
        op("Td", vec![72.into(), y.into()]),
        op("Tj", vec![text]),
        op("ET", vec![]),
    ]
}

/// Fixture with vector artwork, a raster image, a split TJ, an embedded Unicode
/// font, and a second cropped/rotated page sharing the original content stream.
pub fn fixture(font_path: &Path) -> Result<Vec<u8>> {
    let font_bytes =
        std::fs::read(font_path).context("provide a TrueType font with Turkish glyphs")?;
    let face = ttf_parser::Face::parse(&font_bytes, 0)?;
    let mut doc = Document::with_version("1.7");
    doc.reference_table.cross_reference_type = lopdf::xref::XrefType::CrossReferenceTable;
    let pages = doc.new_object_id();
    let latin_font = doc.add_object(dictionary! { "Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Courier", "Encoding"=>"WinAnsiEncoding" });
    let chars: BTreeSet<_> = [TURKISH_ORIGINAL, TURKISH_REPLACEMENT]
        .concat()
        .chars()
        .collect();
    let mut gid_map = vec![0u8; 131072];
    let mut widths = Vec::new();
    let mut mappings = String::new();
    for c in &chars {
        let code = *c as usize;
        let glyph = face.glyph_index(*c).context("font missing Turkish glyph")?;
        gid_map[code * 2..code * 2 + 2].copy_from_slice(&glyph.0.to_be_bytes());
        let width = i64::from(face.glyph_hor_advance(glyph).unwrap_or(0)) * 1000
            / i64::from(face.units_per_em());
        widths.extend([
            Object::Integer(code as i64),
            Object::Array(vec![width.into()]),
        ]);
        mappings.push_str(&format!("<{code:04X}> <{code:04X}>\n"));
    }
    let cmap = format!(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /MyPDFUnicode def /CMapType 2 def 1 begincodespacerange <0000> <FFFF> endcodespacerange {} beginbfchar {} endbfchar endcmap CMapName currentdict /CMap defineresource pop end end",
        chars.len(),
        mappings
    );
    let cmap_id = doc.add_object(Stream::new(Dictionary::new(), cmap.into_bytes()));
    let map_id = doc.add_object(Stream::new(Dictionary::new(), gid_map));
    let bbox = face.global_bounding_box();
    let units = i64::from(face.units_per_em());
    let font_data = doc.add_object(Stream::new(
        dictionary! {"Length1"=>font_bytes.len() as i64},
        font_bytes.clone(),
    ));
    let desc = doc.add_object(dictionary! {"Type"=>"FontDescriptor", "FontName"=>"MyPDFUnicode", "Flags"=>32, "FontBBox"=>vec![(i64::from(bbox.x_min)*1000/units).into(), (i64::from(bbox.y_min)*1000/units).into(), (i64::from(bbox.x_max)*1000/units).into(), (i64::from(bbox.y_max)*1000/units).into()], "ItalicAngle"=>0, "Ascent"=>i64::from(face.ascender())*1000/units, "Descent"=>i64::from(face.descender())*1000/units, "CapHeight"=>700, "StemV"=>80, "FontFile2"=>font_data});
    let cid = doc.add_object(dictionary! {"Type"=>"Font", "Subtype"=>"CIDFontType2", "BaseFont"=>"MyPDFUnicode", "CIDSystemInfo"=>dictionary!{"Registry"=>Object::string_literal("Adobe"), "Ordering"=>Object::string_literal("Identity"), "Supplement"=>0}, "FontDescriptor"=>desc, "CIDToGIDMap"=>map_id, "W"=>widths, "DW"=>1000});
    let unicode_font = doc.add_object(dictionary! {"Type"=>"Font", "Subtype"=>"Type0", "BaseFont"=>"MyPDFUnicode", "Encoding"=>"Identity-H", "DescendantFonts"=>vec![cid.into()], "ToUnicode"=>cmap_id});
    let pixels: Vec<u8> = (0..16)
        .flat_map(|i| {
            if (i + i / 4) % 2 == 0 {
                vec![30, 100, 200]
            } else {
                vec![235, 180, 60]
            }
        })
        .collect();
    let image = doc.add_object(Stream::new(dictionary! {"Type"=>"XObject", "Subtype"=>"Image", "Width"=>4, "Height"=>4, "ColorSpace"=>"DeviceRGB", "BitsPerComponent"=>8}, pixels));
    let resources = doc.add_object(dictionary! {"Font"=>dictionary!{"F1"=>latin_font, "F2"=>unicode_font}, "XObject"=>dictionary!{"Im1"=>image}});
    let mut operations = vec![
        op("q", vec![]),
        op("rg", vec![0.90.into(), 0.95.into(), 1.0.into()]),
        op("re", vec![50.into(), 560.into(), 512.into(), 100.into()]),
        op("f", vec![]),
        op("Q", vec![]),
    ];
    operations.extend(line(
        "F1",
        24,
        716,
        Object::string_literal("MyPDF / Rust milestone 0"),
    ));
    operations.extend(line(
        "F1",
        12,
        684,
        Object::string_literal("Real content editing + digital signatures"),
    ));
    operations.extend(line("F1", 14, 620, Object::string_literal(ORIGINAL)));
    operations.extend(line(
        "F2",
        14,
        580,
        Object::String(
            TURKISH_ORIGINAL
                .encode_utf16()
                .flat_map(u16::to_be_bytes)
                .collect(),
            StringFormat::Hexadecimal,
        ),
    ));
    operations.extend(vec![
        op("BT", vec![]),
        op("Tf", vec!["F1".into(), 14.into()]),
        op("Td", vec![72.into(), 520.into()]),
        op(
            "TJ",
            vec![Object::Array(vec![
                Object::string_literal("Split text "),
                0.into(),
                Object::string_literal("example"),
            ])],
        ),
        op("ET", vec![]),
    ]);
    operations.extend(vec![
        op("q", vec![]),
        op(
            "cm",
            vec![
                80.into(),
                0.into(),
                0.into(),
                80.into(),
                72.into(),
                350.into(),
            ],
        ),
        op("Do", vec!["Im1".into()]),
        op("Q", vec![]),
    ]);
    operations.extend(line(
        "F1",
        11,
        300,
        Object::string_literal("Vector background and raster image must survive edits."),
    ));
    operations.extend(line(
        "F1",
        10,
        84,
        Object::string_literal("Synthetic fixture. No personal document or identity used."),
    ));
    let content = doc.add_object(Stream::new(
        Dictionary::new(),
        Content { operations }.encode()?,
    ));
    let first = doc.add_object(dictionary! {"Type"=>"Page", "Parent"=>pages, "MediaBox"=>vec![0.into(),0.into(),612.into(),792.into()], "Resources"=>resources, "Contents"=>content});
    let second = doc.add_object(dictionary! {"Type"=>"Page", "Parent"=>pages, "MediaBox"=>vec![0.into(),0.into(),612.into(),792.into()], "CropBox"=>vec![36.into(),36.into(),576.into(),756.into()], "Rotate"=>90, "Resources"=>resources, "Contents"=>content});
    doc.objects.insert(
        pages,
        dictionary! {"Type"=>"Pages", "Kids"=>vec![first.into(),second.into()], "Count"=>2}.into(),
    );
    let catalog = doc.add_object(dictionary! {"Type"=>"Catalog", "Pages"=>pages});
    doc.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes)?;
    Ok(bytes)
}

pub fn default_font() -> Result<std::path::PathBuf> {
    for path in [
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "C:\\Windows\\Fonts\\arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ] {
        if Path::new(path).is_file() {
            return Ok(path.into());
        }
    }
    bail!("provide --font pointing to a TrueType font with Turkish glyphs")
}

/// Explicitly prepare an unsigned copy for the signing writer's verified format.
/// Rewriting an already signed document here is forbidden.
pub fn prepare_signing_copy(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut doc = Document::load_mem(bytes)?;
    ensure!(
        !doc.is_encrypted(),
        "encrypted PDFs are outside milestone 0"
    );
    ensure!(
        !has_signatures(&doc),
        "signed bytes cannot be normalized; preserve the signed original"
    );
    doc.prune_objects();
    doc.renumber_objects();
    doc.reference_table.cross_reference_type = lopdf::xref::XrefType::CrossReferenceTable;
    let mut output = Vec::new();
    doc.save_to(&mut output)?;
    Ok(output)
}
