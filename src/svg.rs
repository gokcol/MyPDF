//! Static SVG assets: editable source, bounded native preview, vector PDF appearances.
use anyhow::{Context, Result, ensure};
use lopdf::{Document, ObjectId, Stream, dictionary};
use std::sync::{Arc, Mutex, OnceLock};
pub const DEFAULT: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 240 120\">\n  <path fill=\"#278ed9\" d=\"M12 42 H156 V12 L228 60 156 108 V78 H12 Z\"/>\n</svg>";
/// Prompt text is copied by the user; MyPDF never sends it to a model.
pub fn stamp_prompt(description: &str) -> String {
    format!(
        "Create a polished, reusable PDF stamp as a single standalone SVG.\n\nDesign brief:\n{}\n\nRequirements:\n- Return ONLY the complete SVG code, without markdown fences or explanation.\n- Include xmlns=\"http://www.w3.org/2000/svg\" and a tightly fitted viewBox; transparent background.\n- Use clean vector paths, basic shapes, and optional text. Keep strokes legible at small sizes and leave padding so nothing clips.\n- Use explicit inline fill/stroke attributes and a restrained color palette. Prefer paths for lettering if practical; otherwise use common system fonts.\n- No scripts, animations, event handlers, foreignObject, filters, style blocks, images, external URLs, linked fonts, DOCTYPE, or entities. No raster content.\n- Keep the SVG under 256 KB. It must remain scalable and easy to recolor.\n",
        description.trim()
    )
}
/// Extract a single SVG from a pasted model reply, then validate using the same importer.
pub fn from_response(reply: &str) -> Result<String> {
    ensure!(reply.len() <= 256 * 1024, "SVG response exceeds 256 KB");
    let start = reply
        .find("<svg")
        .context("No SVG found in this response")?;
    let end = reply
        .rfind("</svg>")
        .context("SVG is missing its closing tag")?
        + 6;
    let source = reply
        .get(start..end)
        .context("Invalid SVG response")?
        .to_owned();
    tree(&source, None)?;
    Ok(source)
}

fn recolor(source: &str, color: [f32; 3]) -> Result<String> {
    use quick_xml::events::Event;
    let hex = format!(
        "#{:02x}{:02x}{:02x}",
        (color[0] * 255.) as u8,
        (color[1] * 255.) as u8,
        (color[2] * 255.) as u8
    );
    let mut reader = quick_xml::Reader::from_str(source);
    let mut writer = quick_xml::Writer::new(Vec::new());
    loop {
        let event = reader.read_event()?;
        let event = match event {
            Event::Start(e) | Event::Empty(e) => {
                let empty = source
                    .as_bytes()
                    .get(reader.buffer_position().saturating_sub(2) as usize)
                    == Some(&b'/');
                let mut out = e.to_owned();
                out.clear_attributes();
                let mut rootfill = false;
                for a in e.attributes() {
                    let a = a?;
                    let k = std::str::from_utf8(a.key.as_ref())?;
                    let v = a.normalized_value(quick_xml::XmlVersion::Implicit1_0)?;
                    rootfill |= k == "fill";
                    let v = if (k == "fill" || k == "stroke" || k == "stop-color" || k == "color")
                        && v != "none"
                        && v != "transparent"
                    {
                        hex.clone()
                    } else if k == "style" {
                        v.split(';')
                            .map(|part| {
                                if let Some((key, value)) = part.split_once(':')
                                    && ["fill", "stroke", "stop-color", "color"]
                                        .contains(&key.trim())
                                    && value.trim() != "none"
                                    && value.trim() != "transparent"
                                {
                                    format!("{key}:{hex}")
                                } else {
                                    part.into()
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(";")
                    } else {
                        v.into_owned()
                    };
                    out.push_attribute((k, v.as_str()));
                }
                if e.local_name().as_ref() == b"svg" && !rootfill {
                    out.push_attribute(("fill", hex.as_str()));
                }
                if empty {
                    Event::Empty(out)
                } else {
                    Event::Start(out)
                }
            }
            Event::Eof => break,
            e => e.into_owned(),
        };
        writer.write_event(event)?;
    }
    Ok(String::from_utf8(writer.into_inner())?)
}
pub fn tree(source: &str, color: Option<[f32; 3]>) -> Result<resvg::usvg::Tree> {
    ensure!(
        !source.trim().is_empty() && source.len() <= 256 * 1024,
        "SVG source must be 1 byte–256 KB"
    );
    // No scripts, active content, document entities, or file/network resolvers.
    let mut reader = quick_xml::Reader::from_str(source);
    let mut depth = 0;
    let mut count = 0;
    loop {
        use quick_xml::events::Event;
        match reader.read_event()? {
            Event::Start(e) => {
                depth += 1;
                count += 1;
                ensure!(depth <= 64 && count <= 10_000, "SVG is too complex");
                ensure!(
                    ![
                        b"script".as_slice(),
                        b"foreignObject",
                        b"style",
                        b"filter",
                        b"image"
                    ]
                    .contains(&e.local_name().as_ref()),
                    "Scripts, embedded HTML, CSS style blocks and filters are unsupported in SVG assets; use inline fill/stroke styles"
                );
            }
            Event::Empty(e) => {
                count += 1;
                ensure!(count <= 10_000, "SVG is too complex");
                ensure!(
                    ![
                        b"script".as_slice(),
                        b"foreignObject",
                        b"style",
                        b"filter",
                        b"image"
                    ]
                    .contains(&e.local_name().as_ref()),
                    "Unsupported SVG content"
                );
            }
            Event::End(_) => depth -= 1,
            Event::DocType(_) => anyhow::bail!("SVG document types and entities are unsupported"),
            Event::Eof => break,
            _ => {}
        }
    }
    let mut options = resvg::usvg::Options::default();
    static FONTS: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    options.fontdb = FONTS
        .get_or_init(|| {
            let mut fonts = resvg::usvg::fontdb::Database::new();
            fonts.load_system_fonts();
            Arc::new(fonts)
        })
        .clone();
    let external = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = external.clone();
    options.image_href_resolver.resolve_string = Box::new(move |_, _| {
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
        None
    });
    options.image_href_resolver.resolve_data = Box::new(|_, _, _| None);
    let source = if let Some(c) = color {
        recolor(source, c)?
    } else {
        source.into()
    };
    // Embedded rasters would make a supposedly vector asset only partly scalable.
    ensure!(
        !source.contains("<image"),
        "Use PNG/JPEG import for raster artwork; SVG assets accept vectors and text"
    );
    let tree = resvg::usvg::Tree::from_str(&source, &options)?;
    ensure!(
        !external.load(std::sync::atomic::Ordering::Relaxed),
        "SVG references an external file or image"
    );
    ensure!(
        tree.size().width() <= 10_000.
            && tree.size().height() <= 10_000.
            && !tree.root().children().is_empty(),
        "SVG is empty or its dimensions exceed 10,000 units"
    );
    Ok(tree)
}
pub fn preview(source: &str, color: Option<[f32; 3]>) -> Result<Arc<Vec<u8>>> {
    type Cache = Vec<(String, Arc<Vec<u8>>)>;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let key = format!(
        "{}{:?}",
        crate::identity::fingerprint(source.as_bytes()),
        color
    );
    if let Some((_, png)) = CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .iter()
        .find(|(k, _)| k == &key)
    {
        return Ok(png.clone());
    }
    let tree = tree(source, color)?;
    let scale = 1024. / tree.size().width().max(tree.size().height());
    let mut pix = resvg::tiny_skia::Pixmap::new(
        (tree.size().width() * scale).ceil() as u32,
        (tree.size().height() * scale).ceil() as u32,
    )
    .context("SVG preview allocation failed")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pix.as_mut(),
    );
    let png = Arc::new(pix.encode_png()?);
    let mut cache = CACHE.get().unwrap().lock().unwrap();
    if cache.len() >= 16 {
        cache.remove(0);
    }
    cache.push((key, png.clone()));
    Ok(png)
}
pub fn embed(
    doc: &mut Document,
    source: &str,
    color: Option<[f32; 3]>,
) -> Result<(ObjectId, f32, f32)> {
    let tree = tree(source, color)?;
    let w = tree.size().width();
    let h = tree.size().height();
    let bytes = svg2pdf::to_pdf(
        &tree,
        svg2pdf::ConversionOptions {
            embed_text: false,
            ..Default::default()
        },
        svg2pdf::PageOptions::default(),
    )
    .map_err(|e| anyhow::anyhow!("SVG conversion: {e}"))?;
    let mut foreign = Document::load_mem(&bytes)?;
    foreign.renumber_objects_with(doc.max_id + 1);
    let page = *foreign
        .get_pages()
        .values()
        .next()
        .context("SVG has no page")?;
    let resources = crate::content::resources(&foreign, page)?;
    let content = foreign.get_page_content(page);
    doc.max_id = doc.max_id.max(foreign.max_id);
    doc.objects.extend(foreign.objects);
    let id=doc.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Form","BBox"=>vec![0.into(),0.into(),w.into(),h.into()],"Resources"=>resources},content));
    Ok((id, w, h))
}
