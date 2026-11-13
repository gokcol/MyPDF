//! Bounded, atomic file insertion. Source files are never modified.
use anyhow::{Context, Result, ensure};
use image::RgbaImage;
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
mod native_image;
mod office;
pub use office::{CONVERTER_WARNING, converter_available};
pub const EXTENSIONS: &[&str] = &[
    "pdf", "png", "jpg", "jpeg", "jpe", "jfif", "bmp", "gif", "webp", "tif", "tiff", "heic",
    "heif", "svg", "ico", "pnm", "ppm", "pgm", "pbm", "tga", "txt", "md", "markdown", "docx",
    "rtf",
];
pub const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "jpe", "jfif", "bmp", "gif", "webp", "tif", "tiff", "heic", "heif",
    "svg", "ico", "pnm", "ppm", "pgm", "pbm", "tga",
];
pub fn extension(path: &Path) -> String {
    path.extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase()
}
pub fn is_image(path: &Path) -> bool {
    IMAGE_EXTENSIONS.contains(&extension(path).as_str())
}
pub fn needs_office(paths: &[PathBuf]) -> bool {
    paths
        .iter()
        .any(|p| matches!(extension(p).as_str(), "docx" | "rtf"))
}
#[derive(Clone, Copy, Debug)]
pub enum Placement {
    Pages { at: usize, replace_blank: bool },
    Images { page: usize },
}
enum Item {
    Pdf(Vec<u8>),
    Image(Vec<u8>, [u32; 2]),
    Svg(String, [f32; 2]),
}
pub fn is_blank(bytes: &[u8]) -> bool {
    let Ok(doc) = lopdf::Document::load_mem(bytes) else {
        return false;
    };
    let pages = doc.get_pages();
    pages.len() == 1
        && pages.values().all(|id| {
            doc.get_dictionary(*id)
                .is_ok_and(|d| !d.has(b"Annots") && !d.has(b"Contents"))
        })
        && doc
            .catalog()
            .is_ok_and(|d| !d.has(b"AcroForm") && !d.has(b"Outlines"))
}
pub fn validate_paths(paths: &[PathBuf]) -> Result<()> {
    ensure!(!paths.is_empty() && paths.len() <= 50, "Choose 1–50 files");
    let mut total = 0u64;
    for path in paths {
        ensure!(
            EXTENSIONS.contains(&extension(path).as_str()),
            "Unsupported file: {}",
            path.display()
        );
        let meta =
            std::fs::metadata(path).with_context(|| format!("Cannot read {}", path.display()))?;
        ensure!(
            meta.is_file(),
            "Choose files, not folders: {}",
            path.display()
        );
        total = total
            .checked_add(meta.len())
            .context("Input size overflow")?;
        ensure!(
            total <= 256 * 1024 * 1024,
            "Combined input files exceed 256 MB"
        );
    }
    if needs_office(paths) {
        ensure!(converter_available(), "{CONVERTER_WARNING}");
    }
    Ok(())
}
fn png(image: RgbaImage) -> Result<Item> {
    let pixels = [image.width(), image.height()];
    dimensions(pixels[0], pixels[1])?;
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png)?;
    ensure!(
        out.get_ref().len() <= 32 * 1024 * 1024,
        "Converted image exceeds 32 MB"
    );
    Ok(Item::Image(out.into_inner(), pixels))
}
fn dimensions(w: u32, h: u32) -> Result<()> {
    ensure!(
        w > 0 && h > 0 && w <= 8192 && h <= 8192 && u64::from(w) * u64::from(h) <= 24_000_000,
        "Image exceeds 8192 pixels per side or 24 million pixels"
    );
    Ok(())
}
fn text(bytes: Vec<u8>) -> Result<String> {
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        ensure!(bytes.len().is_multiple_of(2), "Invalid UTF-16 text");
        let little = bytes[0] == 0xff;
        let words: Vec<_> = bytes[2..]
            .chunks_exact(2)
            .map(|c| {
                if little {
                    u16::from_le_bytes([c[0], c[1]])
                } else {
                    u16::from_be_bytes([c[0], c[1]])
                }
            })
            .collect();
        return String::from_utf16(&words).context("Invalid UTF-16 text");
    }
    Ok(String::from_utf8(bytes)
        .context("Save this text file as UTF-8 or UTF-16 and try again")?
        .trim_start_matches('\u{feff}')
        .into())
}
fn load(path: &Path) -> Result<Vec<Item>> {
    let ext = extension(path);
    let limit = if ext == "pdf" {
        256 * 1024 * 1024
    } else {
        32 * 1024 * 1024
    };
    let bytes = crate::convert::read_bounded(path, limit)?;
    match ext.as_str() {
        "pdf" => Ok(vec![Item::Pdf(bytes)]),
        "docx" | "rtf" => Ok(vec![Item::Pdf(office::convert(&bytes, &ext)?)]),
        "txt" | "md" | "markdown" => {
            ensure!(bytes.len() <= 2 * 1024 * 1024, "Text input exceeds 2 MB");
            Ok(vec![Item::Pdf(crate::convert::text_to_pdf(
                &text(bytes)?,
                ext != "txt",
            )?)])
        }
        "svg" => {
            let source = String::from_utf8(bytes).context("SVG must use UTF-8")?;
            let tree = crate::svg::tree(&source, None)?;
            let size = [tree.size().width(), tree.size().height()];
            Ok(vec![Item::Svg(source, size)])
        }
        "heic" | "heif" => Ok(vec![png(native_image::decode(&bytes, &ext)?)?]),
        "tif" | "tiff" => tiff_pages(&bytes),
        "tga" => {
            // TGA has no reliable magic signature.
            let mut reader =
                image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Tga);
            reader.limits(image_limits());
            Ok(vec![png(reader.decode()?.into_rgba8())?])
        }
        _ => Ok(vec![png(crate::content::decode_image(&bytes)?)?]),
    }
}
fn image_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    limits
}
fn tiff_pages(bytes: &[u8]) -> Result<Vec<Item>> {
    use tiff::{
        ColorType,
        decoder::{Decoder, DecodingResult, Limits},
    };
    let mut limits = Limits::default();
    limits.decoding_buffer_size = 128 * 1024 * 1024;
    limits.intermediate_buffer_size = 128 * 1024 * 1024;
    let mut decoder = Decoder::new(Cursor::new(bytes))?.with_limits(limits);
    let mut result = vec![];
    let mut total = 0usize;
    loop {
        ensure!(result.len() < 200, "TIFF exceeds 200 pages");
        let (w, h) = decoder.dimensions()?;
        dimensions(w, h)?;
        let color = decoder.colortype()?;
        let orientation = decoder
            .get_tag_unsigned::<u16>(tiff::tags::Tag::Orientation)
            .unwrap_or(1);
        let mut data = match decoder.read_image()? {
            DecodingResult::U8(v) => v,
            DecodingResult::U16(v) => v.into_iter().map(|n| (n >> 8) as u8).collect(),
            _ => anyhow::bail!("TIFF requires an 8-bit or 16-bit image"),
        };
        let (channels, bits) = match color {
            ColorType::Gray(b) => (1, b),
            ColorType::GrayA(b) => (2, b),
            ColorType::RGB(b) => (3, b),
            ColorType::RGBA(b) | ColorType::CMYK(b) => (4, b),
            _ => anyhow::bail!("This TIFF color format is not supported"),
        };
        if channels == 1 && bits < 8 {
            ensure!(matches!(bits, 1 | 2 | 4), "Unsupported TIFF sample depth");
            let stride = (w as usize * bits as usize).div_ceil(8);
            ensure!(
                data.len() == stride * h as usize,
                "Invalid packed TIFF data"
            );
            data = data
                .chunks_exact(stride)
                .flat_map(|row| {
                    (0..w as usize).map(move |x| {
                        let bit = x * bits as usize;
                        (row[bit / 8] >> (8 - bits as usize - bit % 8)) & ((1 << bits) - 1)
                    })
                })
                .collect();
        }
        ensure!(
            data.len() == w as usize * h as usize * channels,
            "Invalid TIFF pixel data"
        );
        let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
        for pixel in data.chunks_exact(channels) {
            let p = match color {
                ColorType::Gray(_) => {
                    let v = if bits < 8 {
                        (u16::from(pixel[0]) * 255 / ((1u16 << bits) - 1)) as u8
                    } else {
                        pixel[0]
                    };
                    [v, v, v, 255]
                }
                ColorType::GrayA(_) => [pixel[0], pixel[0], pixel[0], pixel[1]],
                ColorType::RGB(_) => [pixel[0], pixel[1], pixel[2], 255],
                ColorType::RGBA(_) => [pixel[0], pixel[1], pixel[2], pixel[3]],
                ColorType::CMYK(_) => {
                    let k = 255 - u16::from(pixel[3]);
                    [
                        ((255 - u16::from(pixel[0])) * k / 255) as u8,
                        ((255 - u16::from(pixel[1])) * k / 255) as u8,
                        ((255 - u16::from(pixel[2])) * k / 255) as u8,
                        255,
                    ]
                }
                _ => unreachable!(),
            };
            rgba.extend_from_slice(&p);
        }
        let mut image = image::DynamicImage::ImageRgba8(
            RgbaImage::from_raw(w, h, rgba).context("Invalid TIFF dimensions")?,
        );
        if let Some(o) = image::metadata::Orientation::from_exif(orientation as u8) {
            image.apply_orientation(o);
        }
        let item = png(image.into_rgba8())?;
        if let Item::Image(b, _) = &item {
            total += b.len();
            ensure!(total <= 256 * 1024 * 1024, "TIFF pages exceed 256 MB");
        }
        result.push(item);
        if !decoder.more_images() {
            break;
        }
        decoder.next_image()?;
    }
    Ok(result)
}
fn as_pdf(item: Item) -> Result<Vec<u8>> {
    use crate::annotations::{self, Annotation, Kind, Style};
    let (kind, natural) = match item {
        Item::Pdf(bytes) => return Ok(bytes),
        Item::Image(bytes, [w, h]) => (
            Kind::Image { png: bytes.into() },
            [w as f32 / 2., h as f32 / 2.],
        ),
        Item::Svg(source, size) => (
            Kind::Svg {
                source,
                tint: false,
            },
            size,
        ),
    };
    let [width, height] = crate::pages::A4;
    let blank = crate::convert::blank(width, height)?;
    let rect = crate::convert::fit_a4(natural);
    annotations::export(&blank, &[Annotation::new(0, rect, kind, Style::default())])
}
/// Complete conversion and insertion before committing any bytes to the open document.
pub fn insert(source: Option<&[u8]>, paths: &[PathBuf], placement: Placement) -> Result<Vec<u8>> {
    validate_paths(paths)?;
    if let Some(source) = source {
        crate::content::editable(source)?;
    }
    let mut result = source.map(<[u8]>::to_vec);
    match placement {
        Placement::Pages {
            mut at,
            replace_blank,
        } => {
            if replace_blank {
                ensure!(
                    source.is_some_and(is_blank),
                    "The target page is no longer empty"
                );
            }
            for path in paths {
                for item in
                    load(path).with_context(|| format!("Could not insert {}", path.display()))?
                {
                    let pdf = as_pdf(item)?;
                    let doc = lopdf::Document::load_mem(&pdf)?;
                    let count = doc.get_pages().len();
                    ensure!(
                        !doc.is_encrypted() && (1..=10_000).contains(&count),
                        "Input PDF is locked or has no usable pages"
                    );
                    result = Some(match result {
                        Some(current) => crate::pages::apply(
                            &current,
                            &crate::pages::Operation::InsertPdf { at, bytes: pdf },
                        )?,
                        None => {
                            // Copy page graphs even for a new document; imported signatures stay with their original.
                            let blank =
                                crate::convert::blank(crate::pages::A4[0], crate::pages::A4[1])?;
                            let joined = crate::pages::apply(
                                &blank,
                                &crate::pages::Operation::InsertPdf { at: 0, bytes: pdf },
                            )?;
                            crate::pages::apply(
                                &joined,
                                &crate::pages::Operation::Delete(vec![count]),
                            )?
                        }
                    });
                    at += count;
                }
            }
            let mut bytes = result.context("No files to insert")?;
            if replace_blank {
                // All inserted pages precede/follow the one known blank page.
                let index = if at == lopdf::Document::load_mem(&bytes)?.get_pages().len() {
                    0
                } else {
                    at
                };
                bytes = crate::pages::apply(&bytes, &crate::pages::Operation::Delete(vec![index]))?;
            }
            Ok(bytes)
        }
        Placement::Images { page } => {
            let source = source.context("Open a PDF before placing images")?;
            ensure!(
                paths.iter().all(|p| is_image(p)),
                "Only images can be placed on a page"
            );
            let geometry = crate::annotations::page_geometry(source)?;
            let size = geometry.get(page).context("Target page no longer exists")?;
            let doc = lopdf::Document::load_mem(source)?;
            let mut marks = crate::annotations::load(&doc);
            let mut count = 0usize;
            let mut total = 0usize;
            for path in paths {
                for item in load(path)? {
                    total += match &item {
                        Item::Image(b, _) => b.len(),
                        Item::Svg(s, _) => s.len(),
                        Item::Pdf(b) => b.len(),
                    };
                    ensure!(total <= 256 * 1024 * 1024, "Converted images exceed 256 MB");
                    let (kind, w, h) = match item {
                        Item::Image(bytes, [w, h]) => (
                            crate::annotations::Kind::Image { png: bytes.into() },
                            w as f32,
                            h as f32,
                        ),
                        Item::Svg(source, [w, h]) => (
                            crate::annotations::Kind::Svg {
                                source,
                                tint: false,
                            },
                            w,
                            h,
                        ),
                        _ => anyhow::bail!("Choose images for page placement"),
                    };
                    let (pw, ph) = (size.0[2], size.0[3]);
                    let scale = (pw / w).min(ph / h);
                    let (w, h) = (w * scale, h * scale);
                    let x = (pw - w) / 2.;
                    let y = (ph - h) / 2.;
                    marks.push(crate::annotations::Annotation::new(
                        page,
                        [x, y, x + w, y + h],
                        kind,
                        crate::annotations::Style::default(),
                    ));
                    count += 1;
                    ensure!(count <= 200, "Choose at most 200 images/pages");
                }
            }
            crate::annotations::export(source, &marks)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        annotations::{self, Session},
        convert, pages, viewer,
    };
    fn fixture(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
    fn raster(format: image::ImageFormat) -> Vec<u8> {
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            40,
            20,
            image::Rgb([240, 30, 20]),
        ));
        let mut data = Cursor::new(Vec::new());
        if format == image::ImageFormat::Ico {
            image.into_rgba8().write_to(&mut data, format).unwrap();
        } else {
            image.write_to(&mut data, format).unwrap();
        }
        data.into_inner()
    }
    fn count(bytes: &[u8]) -> usize {
        lopdf::Document::load_mem(bytes).unwrap().get_pages().len()
    }
    #[test]
    fn every_inserted_image_fits_the_target_page_and_new_pages_are_a4() {
        let dir = tempfile::tempdir().unwrap();
        for (index, (w, h)) in [(2, 1), (3000, 1000), (1000, 3000), (1, 1), (1, 4000)]
            .into_iter()
            .enumerate()
        {
            let mut data = Cursor::new(vec![]);
            image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                w,
                h,
                image::Rgb([240, 30, 20]),
            ))
            .write_to(&mut data, image::ImageFormat::Png)
            .unwrap();
            let path = fixture(dir.path(), &format!("image-{index}.png"), data.get_ref());
            for (source, placement, expected) in [
                (
                    None,
                    Placement::Pages {
                        at: 0,
                        replace_blank: false,
                    },
                    pages::A4,
                ),
                (
                    Some(convert::blank(320., 240.).unwrap()),
                    Placement::Images { page: 0 },
                    [320., 240.],
                ),
                (
                    Some(convert::blank(320., 240.).unwrap()),
                    Placement::Pages {
                        at: 0,
                        replace_blank: true,
                    },
                    pages::A4,
                ),
            ] {
                let output =
                    insert(source.as_deref(), std::slice::from_ref(&path), placement).unwrap();
                let geometry = annotations::page_geometry(&output).unwrap();
                assert_eq!(&geometry[0].0[2..], &expected);
                let marks = annotations::load(&lopdf::Document::load_mem(&output).unwrap());
                let [x, y, r, b] = marks[0].rect;
                assert!(
                    x >= -0.001
                        && y >= -0.001
                        && r <= expected[0] + 0.001
                        && b <= expected[1] + 0.001
                );
                assert!(((r - x) / (b - y) - w as f32 / h as f32).abs() < 0.001);
                assert!(
                    ((r - x) - expected[0]).abs() < 0.001 || ((b - y) - expected[1]).abs() < 0.001
                );
                assert!(
                    (x - (expected[0] - r)).abs() < 0.001 && (y - (expected[1] - b)).abs() < 0.001
                );
            }
        }
        let svg = fixture(dir.path(), "wide.svg", br#"<svg xmlns="http://www.w3.org/2000/svg" width="900" height="300"><rect width="900" height="300" fill="red"/></svg>"#);
        let bytes = insert(
            None,
            &[svg],
            Placement::Pages {
                at: 0,
                replace_blank: false,
            },
        )
        .unwrap();
        assert_eq!(
            &annotations::page_geometry(&bytes).unwrap()[0].0[2..],
            &pages::A4
        );
    }
    #[test]
    fn raster_formats_place_on_blank_page_and_survive_save_and_undo() {
        for (ext, format) in [
            ("png", image::ImageFormat::Png),
            ("jpg", image::ImageFormat::Jpeg),
            ("bmp", image::ImageFormat::Bmp),
            ("gif", image::ImageFormat::Gif),
            ("webp", image::ImageFormat::WebP),
            ("tif", image::ImageFormat::Tiff),
            ("ico", image::ImageFormat::Ico),
            ("tga", image::ImageFormat::Tga),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = fixture(dir.path(), &format!("image.{ext}"), &raster(format));
            let blank = convert::blank(300., 400.).unwrap();
            let result = insert(Some(&blank), &[path], Placement::Images { page: 0 }).unwrap();
            assert_eq!(count(&result), 1, "{ext}");
            let doc = lopdf::Document::load_mem(&result).unwrap();
            let marks = annotations::load(&doc);
            assert_eq!(marks.len(), 1, "{ext}");
            assert!(matches!(marks[0].kind, annotations::Kind::Image { .. }));
            let saved = annotations::export(&result, &marks).unwrap();
            let (size, rgba) = viewer::render_page(&saved, 0, 1.).unwrap();
            let center = ((size[1] / 2) * size[0] + size[0] / 2) * 4;
            assert!(rgba[center] > 180 && rgba[center + 1] < 80, "{ext}");
            let mut session = Session::new(blank.clone().into(), vec![], true);
            session.commit_document(result).unwrap();
            session.undo();
            assert_eq!(session.source.as_ref(), blank);
            session.redo();
            assert_eq!(session.annotations.len(), 1);
        }
    }
    #[test]
    fn mixed_pages_keep_order_position_and_existing_annotations() {
        let dir = tempfile::tempdir().unwrap();
        let paths = vec![
            fixture(dir.path(), "first.txt", b"First imported page"),
            fixture(dir.path(), "second.md", b"# Second imported page"),
            fixture(
                dir.path(),
                "third.pdf",
                &convert::blank(250., 350.).unwrap(),
            ),
        ];
        let original = convert::blank(300., 400.).unwrap();
        let mark = annotations::Annotation::new(
            0,
            [10., 10., 50., 50.],
            annotations::Kind::Image {
                png: raster(image::ImageFormat::Png).into(),
            },
            Default::default(),
        );
        let original = annotations::export(&original, &[mark]).unwrap();
        let output = insert(
            Some(&original),
            &paths,
            Placement::Pages {
                at: 0,
                replace_blank: false,
            },
        )
        .unwrap();
        let doc = lopdf::Document::load_mem(&output).unwrap();
        assert_eq!(count(&output), 4);
        assert!(
            doc.extract_text(&[1])
                .unwrap()
                .contains("First imported page")
        );
        assert!(
            doc.extract_text(&[2])
                .unwrap()
                .contains("Second imported page")
        );
        assert_eq!(annotations::load(&doc)[0].page, 3);
        assert_eq!(
            annotations::page_geometry(&output).unwrap()[2].0,
            [0., 0., 250., 350.]
        );
        let new = insert(
            None,
            &paths,
            Placement::Pages {
                at: 0,
                replace_blank: false,
            },
        )
        .unwrap();
        assert_eq!(count(&new), 3);
    }
    #[test]
    fn image_page_imports_remain_editable_layers() {
        let dir = tempfile::tempdir().unwrap();
        let input = raster(image::ImageFormat::Png);
        let paths = vec![fixture(dir.path(), "picture.png", &input)];
        let original = convert::images_to_pdf(&[input], 144.).unwrap();
        let blank = convert::blank(300., 400.).unwrap();
        let imported = insert(
            Some(&blank),
            &paths,
            Placement::Pages {
                at: 0,
                replace_blank: true,
            },
        )
        .unwrap();
        let doc = lopdf::Document::load_mem(&imported).unwrap();
        let marks = annotations::load(&doc);
        assert_eq!(marks.len(), 1);
        assert!(matches!(marks[0].kind, annotations::Kind::Image { .. }));
        assert_eq!(
            annotations::page_geometry(&original).unwrap(),
            annotations::page_geometry(&imported).unwrap()
        );
        assert_eq!(
            viewer::render_page(&original, 0, 2.).unwrap(),
            viewer::render_page(&imported, 0, 2.).unwrap()
        );
    }
    #[test]
    fn replacing_blank_is_explicit_and_rejects_nonempty_pages() {
        let dir = tempfile::tempdir().unwrap();
        let paths = vec![fixture(dir.path(), "note.txt", b"Replacement")];
        let blank = convert::blank(300., 400.).unwrap();
        for at in [0, 1] {
            let output = insert(
                Some(&blank),
                &paths,
                Placement::Pages {
                    at,
                    replace_blank: true,
                },
            )
            .unwrap();
            assert_eq!(count(&output), 1);
            assert!(!is_blank(&output));
            assert!(
                insert(
                    Some(&output),
                    &paths,
                    Placement::Pages {
                        at: 0,
                        replace_blank: true
                    }
                )
                .is_err()
            );
        }
        let bad = fixture(dir.path(), "broken.pdf", b"not a PDF");
        let mut paths = paths;
        paths.push(bad);
        assert!(
            insert(
                Some(&blank),
                &paths,
                Placement::Pages {
                    at: 1,
                    replace_blank: false
                }
            )
            .is_err()
        );
        assert!(is_blank(&blank));
    }
    #[test]
    fn tiff_imports_every_frame_and_honors_orientation() {
        let mut bytes = Cursor::new(Vec::new());
        {
            use tiff::encoder::{TiffEncoder, colortype::RGB8};
            let mut encoder = TiffEncoder::new(&mut bytes).unwrap();
            encoder.write_image::<RGB8>(4, 2, &[255; 24]).unwrap();
            let mut second = encoder.new_image::<RGB8>(4, 2).unwrap();
            second
                .encoder()
                .write_tag(tiff::tags::Tag::Orientation, 6u16)
                .unwrap();
            second.write_data(&[0; 24]).unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        let path = fixture(dir.path(), "multipage.tiff", &bytes.into_inner());
        let output = insert(
            None,
            &[path],
            Placement::Pages {
                at: 0,
                replace_blank: false,
            },
        )
        .unwrap();
        assert_eq!(count(&output), 2);
        for (bounds, _) in annotations::page_geometry(&output).unwrap() {
            assert_eq!(&bounds[2..], &pages::A4);
        }
        let marks = annotations::load(&lopdf::Document::load_mem(&output).unwrap());
        for (mark, aspect) in marks.iter().zip([2., 0.5]) {
            let [x, y, r, b] = mark.rect;
            assert!(((r - x) / (b - y) - aspect).abs() < 0.001);
        }
    }
    #[test]
    fn svg_stays_vector_and_utf16_text_is_readable() {
        let dir = tempfile::tempdir().unwrap();
        let svg = fixture(dir.path(), "shape.svg", br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="60"><rect width="100" height="60" fill="#ff0000"/></svg>"##);
        let output = insert(
            Some(&convert::blank(300., 400.).unwrap()),
            &[svg],
            Placement::Images { page: 0 },
        )
        .unwrap();
        assert!(matches!(
            &annotations::load(&lopdf::Document::load_mem(&output).unwrap())[0].kind,
            annotations::Kind::Svg { .. }
        ));
        let mut bytes = vec![0xff, 0xfe];
        bytes.extend("Türkçe Grüße".encode_utf16().flat_map(u16::to_le_bytes));
        let txt = fixture(dir.path(), "unicode.txt", &bytes);
        let output = insert(
            None,
            &[txt],
            Placement::Pages {
                at: 0,
                replace_blank: false,
            },
        )
        .unwrap();
        assert!(
            lopdf::Document::load_mem(&output)
                .unwrap()
                .extract_text(&[1])
                .unwrap()
                .contains("Türkçe Grüße")
        );
        let rotated = pages::apply(
            &output,
            &pages::Operation::Rotate {
                pages: vec![0],
                clockwise: true,
            },
        )
        .unwrap();
        assert_eq!(count(&rotated), 1);
    }
}
