//! Local, explicitly text-oriented document conversions. No office process or WebView.
use crate::{content, pdf_font::PdfFont};
use anyhow::{Context, Result, ensure};
use lopdf::{Document, Object, Stream, dictionary};
use std::{
    io::{Cursor, Read, Write},
    path::Path,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Format {
    Png,
    Jpeg,
    Text,
    Markdown,
    Word,
}
impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Text => "txt",
            Self::Markdown => "md",
            Self::Word => "docx",
        }
    }
}
pub fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>> {
    let mut b = vec![];
    std::fs::File::open(path)?
        .take(max + 1)
        .read_to_end(&mut b)?;
    ensure!(
        b.len() as u64 <= max,
        "Input exceeds {} MB",
        max / 1024 / 1024
    );
    Ok(b)
}
pub fn blank(width: f32, height: f32) -> Result<Vec<u8>> {
    ensure!(
        width.is_finite()
            && height.is_finite()
            && (72.0..=14_400.0).contains(&width)
            && (72.0..=14_400.0).contains(&height),
        "Invalid blank page size"
    );
    let mut d = Document::with_version("1.7");
    let pages = d.new_object_id();
    let page=d.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"MediaBox"=>vec![0.into(),0.into(),width.into(),height.into()],"Resources"=>dictionary!{}});
    d.objects.insert(
        pages,
        Object::Dictionary(
            dictionary! {"Type"=>"Pages","Kids"=>vec![Object::Reference(page)],"Count"=>1},
        ),
    );
    let root = d.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages});
    d.trailer.set("Root", root);
    content::serialize(&mut d)
}
pub fn images_to_pdf(inputs: &[Vec<u8>], dpi: f32) -> Result<Vec<u8>> {
    ensure!(
        !inputs.is_empty() && inputs.len() <= 200,
        "Choose 1–200 images"
    );
    ensure!((72.0..=600.0).contains(&dpi), "DPI must be 72–600");
    ensure!(
        inputs.iter().map(Vec::len).sum::<usize>() <= 256 * 1024 * 1024,
        "Combined images exceed 256 MB"
    );
    let mut d = Document::with_version("1.7");
    let pages = d.new_object_id();
    let mut kids = vec![];
    for input in inputs {
        let image = content::decode_image(input)?;
        let (w, h) = (
            image.width() as f32 * 72. / dpi,
            image.height() as f32 * 72. / dpi,
        );
        let id = content::embed_image(&mut d, &image)?;
        let stream = d.add_object(Stream::new(
            dictionary! {},
            format!("q {w} 0 0 {h} 0 0 cm /Im0 Do Q").into_bytes(),
        ));
        let p=d.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"MediaBox"=>vec![0.into(),0.into(),w.into(),h.into()],"Resources"=>dictionary!{"XObject"=>dictionary!{"Im0"=>id}},"Contents"=>stream});
        kids.push(Object::Reference(p));
    }
    let count = kids.len();
    d.objects.insert(
        pages,
        Object::Dictionary(dictionary! {"Type"=>"Pages","Kids"=>kids,"Count"=>count as i64}),
    );
    let root = d.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages});
    d.trailer.set("Root", root);
    content::serialize(&mut d)
}
fn text_pages(bytes: &[u8], pages: &[usize]) -> Result<Vec<String>> {
    let pdf = hayro::hayro_syntax::Pdf::new(bytes.to_vec())
        .map_err(|e| anyhow::anyhow!("PDF conversion: {e:?}"))?;
    let fonts = hayro::hayro_interpret::InterpreterCache::new();
    let mut result = vec![];
    for &p in pages {
        let page = pdf.pages().get(p).context("Page is outside the PDF")?;
        let text = crate::viewer::extract_page(page, &fonts);
        ensure!(!text.limited, "Page text exceeds the conversion limit");
        result.push(text.text);
        ensure!(
            result.iter().map(String::len).sum::<usize>() <= 4 * 1024 * 1024,
            "Extracted text exceeds 4 MB"
        );
    }
    ensure!(
        result.iter().any(|s| !s.trim().is_empty()),
        "No extractable text on the selected pages. Scanned pages need OCR, which is not included yet."
    );
    Ok(result)
}
fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn docx(pages: &[String]) -> Result<Vec<u8>> {
    let mut body = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>",
    );
    for (i, page) in pages.iter().enumerate() {
        if i > 0 {
            body.push_str("<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>");
        }
        for line in page.lines() {
            body.push_str(&format!(
                "<w:p><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
                xml(line)
            ));
        }
    }
    body.push_str("<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1134\" w:right=\"1134\" w:bottom=\"1134\" w:left=\"1134\"/></w:sectPr></w:body></w:document>");
    let mut zip = zip::ZipWriter::new(Cursor::new(vec![]));
    let opt = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (path, data) in [
        (
            "[Content_Types].xml",
            "<?xml version=\"1.0\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>",
        ),
        (
            "_rels/.rels",
            "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>",
        ),
        ("word/document.xml", body.as_str()),
    ] {
        zip.start_file(path, opt)?;
        zip.write_all(data.as_bytes())?;
    }
    Ok(zip.finish()?.into_inner())
}
pub fn export_text(bytes: &[u8], pages: &[usize], format: Format) -> Result<Vec<u8>> {
    ensure!(
        !pages.is_empty() && pages.len() <= 1000,
        "Select 1–1000 pages"
    );
    let text = text_pages(bytes, pages)?;
    match format {
        Format::Text => Ok(text.join("\n\n\u{c}\n\n").into_bytes()),
        Format::Markdown => Ok(text
            .iter()
            .zip(pages)
            .map(|(t, p)| {
                format!(
                    "## Page {}\n\n{}",
                    p + 1,
                    t.lines()
                        .map(|l| l
                            .chars()
                            .map(|c| if "\\`*_{}[]<>#+-.!|".contains(c) {
                                format!("\\{c}")
                            } else {
                                c.to_string()
                            })
                            .collect::<String>())
                        .collect::<Vec<_>>()
                        .join("\n\n")
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n")
            .into_bytes()),
        Format::Word => docx(&text),
        _ => anyhow::bail!("Choose a text format"),
    }
}
pub fn export_images(
    bytes: &[u8],
    pages: &[usize],
    format: Format,
    dpi: f32,
    destination: &Path,
) -> Result<()> {
    ensure!(
        matches!(format, Format::Png | Format::Jpeg) && (72.0..=300.0).contains(&dpi),
        "Choose PNG/JPEG at 72–300 DPI"
    );
    ensure!(
        !pages.is_empty() && pages.len() <= 200,
        "Export 1–200 pages at a time"
    );
    let parent = destination.parent().context("Choose an output folder")?;
    let stage = tempfile::tempdir_in(parent)?;
    let pdf = hayro::hayro_syntax::Pdf::new(bytes.to_vec())
        .map_err(|e| anyhow::anyhow!("PDF conversion: {e:?}"))?;
    let cache = hayro::RenderCache::new();
    let scale = dpi / 72.;
    let mut total = 0;
    for &p in pages {
        let page = pdf.pages().get(p).context("Invalid export page")?;
        let (w, h) = page.render_dimensions();
        ensure!(
            w * scale <= 8192. && h * scale <= 8192. && w * h * scale * scale <= 24_000_000.,
            "Page {} exceeds image export limits; reduce DPI",
            p + 1
        );
        let pix = hayro::render(
            page,
            &cache,
            &crate::render_fonts::settings(),
            &hayro::RenderSettings::default(),
            &hayro::PixmapSettings {
                x_scale: scale,
                y_scale: scale,
                bg_color: hayro::vello_cpu::color::palette::css::WHITE,
            },
        );
        let png = pix.into_png()?;
        let output = if format == Format::Jpeg {
            let image = image::load_from_memory(&png)?.to_rgb8();
            let mut out = vec![];
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92)
                .encode_image(&image)?;
            out
        } else {
            png
        };
        total += output.len();
        ensure!(
            total <= 512 * 1024 * 1024,
            "Export exceeds 512 MB; select fewer pages"
        );
        crate::write_new(
            &stage
                .path()
                .join(format!("page-{}.{}", p + 1, format.extension())),
            &output,
        )?;
    }
    // Reserve a fresh folder, never merge into or replace an existing directory.
    std::fs::create_dir(destination).context("Output folder already exists; choose a new name")?;
    let result = (|| -> Result<()> {
        for entry in std::fs::read_dir(stage.path())? {
            let entry = entry?;
            std::fs::rename(entry.path(), destination.join(entry.file_name()))?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(destination);
    }
    result
}

pub fn docx_text(bytes: &[u8]) -> Result<String> {
    ensure!(bytes.len() <= 32 * 1024 * 1024, "Word input exceeds 32 MB");
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    ensure!(archive.len() <= 4096, "Word archive has too many entries");
    let file = archive
        .by_name("word/document.xml")
        .context("Choose a .docx document")?;
    ensure!(
        file.size() <= 4 * 1024 * 1024,
        "Word document text exceeds 4 MB"
    );
    let mut data = String::new();
    file.take(4 * 1024 * 1024 + 1).read_to_string(&mut data)?;
    ensure!(data.len() <= 4 * 1024 * 1024, "Word XML too large");
    let mut reader = quick_xml::Reader::from_str(&data);
    let mut text = String::new();
    let mut in_text = false;
    let mut depth = 0;
    let mut skipped = 0;
    loop {
        use quick_xml::events::Event;
        match reader.read_event()? {
            Event::Start(e) => {
                depth += 1;
                ensure!(depth <= 128, "Word nesting is too deep");
                match e.local_name().as_ref() {
                    b"t" => in_text = true,
                    b"del" => skipped += 1,
                    b"drawing" | b"pict" | b"object" | b"altChunk" => anyhow::bail!(
                        "This Word document contains images or embedded content. This version imports text documents only."
                    ),
                    _ => {}
                }
            }
            Event::Empty(e) => match e.local_name().as_ref() {
                b"tab" => text.push_str("    "),
                b"br" => text.push('\n'),
                b"pStyle" => {
                    for a in e.attributes() {
                        let a = a?;
                        if a.key.local_name().as_ref() == b"val" {
                            let v = a.normalized_value(quick_xml::XmlVersion::Implicit1_0)?;
                            if let Some(level) = v
                                .strip_prefix("Heading")
                                .and_then(|v| v.parse::<usize>().ok())
                                .filter(|l| (1..=6).contains(l))
                            {
                                text.push_str(&"#".repeat(level));
                                text.push(' ');
                            }
                        }
                    }
                }
                _ => {}
            },
            Event::End(e) => {
                depth -= 1;
                match e.local_name().as_ref() {
                    b"t" => in_text = false,
                    b"p" => text.push('\n'),
                    b"tc" => text.push_str(" | "),
                    b"del" => skipped -= 1,
                    _ => {}
                }
            }
            Event::Text(e) if in_text && skipped == 0 => {
                text.push_str(&quick_xml::escape::unescape(&e.decode()?)?)
            }
            Event::GeneralRef(e) if in_text && skipped == 0 => {
                let n = e.decode()?;
                text.push_str(&quick_xml::escape::unescape(&format!("&{n};"))?);
            }
            Event::DocType(_) => anyhow::bail!("Word XML with a document type is unsupported"),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(text)
}
/// Break a line where it reaches the width: between words when it can, and
/// inside a word only when that word alone is too wide. Leading spaces stay.
fn wrap(line: &str, fits: impl Fn(&str) -> bool) -> Vec<String> {
    let mut lines = vec![];
    let mut current = String::new();
    for word in line.split_inclusive(' ') {
        let next = format!("{current}{word}");
        if current.trim().is_empty() || fits(next.trim_end()) {
            current = next;
        } else {
            lines.push(current.trim_end().to_owned());
            current = word.to_owned();
        }
        // A word wider than the line is split where it reaches the edge.
        while !fits(current.trim_end()) && current.trim_end().chars().count() > 1 {
            let mut prefix = String::new();
            for c in current.chars() {
                let longer = format!("{prefix}{c}");
                if !prefix.is_empty() && !fits(&longer) {
                    break;
                }
                prefix = longer;
            }
            current = current[prefix.len()..].to_owned();
            lines.push(prefix);
        }
    }
    lines.push(current.trim_end().to_owned());
    lines
}

/// Typeset UTF-8 text/basic Markdown on A4 with an embedded Unicode font.
pub fn text_to_pdf(text: &str, markdown: bool) -> Result<Vec<u8>> {
    ensure!(
        !text.trim().is_empty() && text.len() <= 1024 * 1024,
        "Choose 1 byte–1 MB of text"
    );
    ensure!(
        !text
            .chars()
            .any(|c| c.is_control() && !"\n\r\t\u{c}".contains(c)),
        "Text contains unsupported control characters"
    );
    let text = text
        .replace('\r', "")
        .replace('\t', "    ")
        .replace('\u{c}', "\n");
    let mut d = Document::with_version("1.7");
    let font = PdfFont::embed(&mut d, &text)?;
    let rootpages = d.new_object_id();
    let mut kids = vec![];
    let mut body = String::new();
    let mut y = 790_f32;
    let add_page = |d: &mut Document, body: &mut String, kids: &mut Vec<Object>| {
        let stream = d.add_object(Stream::new(
            dictionary! {},
            std::mem::take(body).into_bytes(),
        ));
        let id=d.add_object(dictionary!{"Type"=>"Page","Parent"=>rootpages,"MediaBox"=>vec![0.into(),0.into(),595.into(),842.into()],"Resources"=>dictionary!{"Font"=>dictionary!{"F0"=>font.id}},"Contents"=>stream});
        kids.push(Object::Reference(id));
    };
    let mut code = false;
    for raw in text.lines() {
        if markdown && raw.trim_start().starts_with("```") {
            code = !code;
            continue;
        }
        let n = raw.chars().take_while(|c| *c == '#').count();
        let heading =
            markdown && !code && (1..=6).contains(&n) && raw.as_bytes().get(n) == Some(&b' ');
        let line = if heading { &raw[n + 1..] } else { raw };
        let size = if heading { 24. - n as f32 * 2. } else { 11. };
        let lines = wrap(line, |text| font.width(text, size) <= 491.);
        for line in lines {
            if y - size < 52. {
                add_page(&mut d, &mut body, &mut kids);
                ensure!(kids.len() < 1000, "Text exceeds 1000 pages");
                y = 790.;
            }
            body.push_str(&format!(
                "BT /F0 {size} Tf 0.12 0.14 0.18 rg 1 0 0 1 52 {y} Tm <{}> Tj ET\n",
                font.hex(&line)?
            ));
            y -= size * 1.4;
        }
        y -= 4.;
    }
    add_page(&mut d, &mut body, &mut kids);
    let count = kids.len();
    d.objects.insert(
        rootpages,
        Object::Dictionary(dictionary! {"Type"=>"Pages","Count"=>count as i64,"Kids"=>kids}),
    );
    let root = d.add_object(dictionary! {"Type"=>"Catalog","Pages"=>rootpages});
    d.trailer.set("Root", root);
    content::serialize(&mut d)
}

#[cfg(test)]
mod wrap_tests {
    use super::wrap;

    #[test]
    fn lines_break_between_words() {
        let fits = |n: usize| move |t: &str| t.chars().count() <= n;
        assert_eq!(wrap("aaa bbb ccc", fits(7)), ["aaa bbb", "ccc"]);
        assert_eq!(wrap("one two", fits(20)), ["one two"]);
        assert_eq!(wrap("", fits(5)), [""]);
        // Too long for any line: split inside the word.
        assert_eq!(wrap("abcdefghij", fits(4)), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap("hi abcdefghij", fits(4)), ["hi", "abcd", "efgh", "ij"]);
        // Indentation (as in code) stays on the first line.
        assert_eq!(wrap("    code here", fits(9)), ["    code", "here"]);
    }
}
