//! Recognize Text (OCR): scanned pages get an invisible text layer, so their
//! words can be searched, selected and copied. Recognition uses the text
//! recognition built into macOS (Vision), on this computer.
use crate::header_footer::{content_ids, isolation};
use crate::pdf_font::PdfFont;
use anyhow::{Context as _, Result, ensure};
use lopdf::{Object, Stream, dictionary};

const MARK: &[u8] = b"MyPDFOCR";
/// Pages are read at this many pixels per point (about 216 dpi).
const SCALE: f32 = 3.0;

/// A recognized line in displayed page coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    pub rect: [f32; 4],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Pages that got a text layer.
    pub pages: usize,
    /// Pages left alone because they already had text.
    pub skipped: usize,
    pub lines: usize,
}

/// Whether text recognition is available here.
pub fn available() -> bool {
    cfg!(target_os = "macos")
}

#[cfg(target_os = "macos")]
mod vision {
    use super::Line;
    use anyhow::{Result, bail};
    use objc2::encode::{Encode, Encoding};
    use objc2::runtime::{AnyObject, Bool};
    use objc2::{class, msg_send, sel};
    use std::ffi::{CStr, CString, c_char, c_void};

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGSize {
        width: f64,
        height: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }
    // SAFETY: the layouts match CoreGraphics' structs.
    unsafe impl Encode for CGPoint {
        const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
    }
    unsafe impl Encode for CGSize {
        const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
    }
    unsafe impl Encode for CGRect {
        const ENCODING: Encoding =
            Encoding::Struct("CGRect", &[CGPoint::ENCODING, CGSize::ENCODING]);
    }

    #[link(name = "Vision", kind = "framework")]
    unsafe extern "C" {}

    unsafe fn text(object: *mut AnyObject) -> String {
        if object.is_null() {
            return String::new();
        }
        let chars: *const c_char = unsafe { msg_send![object, UTF8String] };
        if chars.is_null() {
            return String::new();
        }
        unsafe { CStr::from_ptr(chars) }
            .to_string_lossy()
            .into_owned()
    }
    unsafe fn string(value: &str) -> *mut AnyObject {
        let value = CString::new(value).unwrap_or_default();
        unsafe { msg_send![class!(NSString), stringWithUTF8String: value.as_ptr()] }
    }

    /// The lines of text in a picture of a page, in its `width` × `height` points.
    pub fn read(image: &[u8], width: f32, height: f32) -> Result<Vec<Line>> {
        objc2::rc::autoreleasepool(|_| unsafe {
            let data: *mut AnyObject = msg_send![
                class!(NSData),
                dataWithBytes: image.as_ptr().cast::<c_void>(),
                length: image.len()
            ];
            let options: *mut AnyObject = msg_send![class!(NSDictionary), dictionary];
            let handler: *mut AnyObject = msg_send![class!(VNImageRequestHandler), alloc];
            let handler: *mut AnyObject = msg_send![handler, initWithData: data, options: options];
            let request: *mut AnyObject = msg_send![class!(VNRecognizeTextRequest), alloc];
            let request: *mut AnyObject = msg_send![request, init];
            if handler.is_null() || request.is_null() {
                bail!("Text recognition is not available on this Mac");
            }
            // Accurate recognition, with language correction.
            let _: () = msg_send![request, setRecognitionLevel: 0isize];
            let _: () = msg_send![request, setUsesLanguageCorrection: Bool::YES];
            let detects: Bool =
                msg_send![request, respondsToSelector: sel!(setAutomaticallyDetectsLanguage:)];
            if detects.as_bool() {
                let _: () = msg_send![request, setAutomaticallyDetectsLanguage: Bool::YES];
            }
            // Prefer Turkish and the common European languages this Mac can read.
            let supported: *mut AnyObject = msg_send![
                request,
                supportedRecognitionLanguagesAndReturnError: std::ptr::null_mut::<*mut AnyObject>()
            ];
            let mut known = vec![];
            if !supported.is_null() {
                let n: usize = msg_send![supported, count];
                for i in 0..n {
                    let item: *mut AnyObject = msg_send![supported, objectAtIndex: i];
                    known.push(text(item));
                }
            }
            let wanted: Vec<*mut AnyObject> = [
                "tr-TR", "en-US", "de-DE", "fr-FR", "es-ES", "it-IT", "pt-BR", "nl-NL",
            ]
            .iter()
            .filter(|l| known.iter().any(|k| k == *l))
            .map(|l| string(l))
            .collect();
            if !wanted.is_empty() {
                let languages: *mut AnyObject = msg_send![
                    class!(NSArray),
                    arrayWithObjects: wanted.as_ptr(),
                    count: wanted.len()
                ];
                let _: () = msg_send![request, setRecognitionLanguages: languages];
            }
            let requests: *mut AnyObject = msg_send![class!(NSArray), arrayWithObject: request];
            let mut error: *mut AnyObject = std::ptr::null_mut();
            let done: Bool = msg_send![handler, performRequests: requests, error: &mut error];
            let mut lines = vec![];
            if done.as_bool() {
                let results: *mut AnyObject = msg_send![request, results];
                let n: usize = if results.is_null() {
                    0
                } else {
                    msg_send![results, count]
                };
                for i in 0..n {
                    let observation: *mut AnyObject = msg_send![results, objectAtIndex: i];
                    let candidates: *mut AnyObject = msg_send![observation, topCandidates: 1usize];
                    let count: usize = msg_send![candidates, count];
                    if count == 0 {
                        continue;
                    }
                    let best: *mut AnyObject = msg_send![candidates, objectAtIndex: 0usize];
                    let words: *mut AnyObject = msg_send![best, string];
                    let line = text(words);
                    // Normalized, with the origin at the bottom left of the picture.
                    let b: CGRect = msg_send![observation, boundingBox];
                    let (x0, x1) = (b.origin.x as f32, (b.origin.x + b.size.width) as f32);
                    let (y0, y1) = (b.origin.y as f32, (b.origin.y + b.size.height) as f32);
                    if !line.trim().is_empty() {
                        lines.push(Line {
                            text: line.trim().to_owned(),
                            rect: [
                                x0 * width,
                                (1. - y1) * height,
                                x1 * width,
                                (1. - y0) * height,
                            ],
                        });
                    }
                }
            }
            let _: () = msg_send![request, release];
            let _: () = msg_send![handler, release];
            if !done.as_bool() {
                let reason: *mut AnyObject = if error.is_null() {
                    std::ptr::null_mut()
                } else {
                    msg_send![error, localizedDescription]
                };
                bail!("Text recognition failed: {}", text(reason));
            }
            Ok(lines)
        })
    }
}

/// The lines of text on one page of a PDF.
pub fn read_page(bytes: &[u8], page: usize) -> Result<Vec<Line>> {
    #[cfg(target_os = "macos")]
    {
        let geometry = crate::annotations::page_geometry(bytes)?;
        let (page_box, _) = geometry.get(page).context("Page not found")?;
        let ([w, h], rgba) = crate::viewer::render_page(bytes, page, SCALE)?;
        let picture =
            image::RgbaImage::from_raw(w as u32, h as u32, rgba).context("Page picture")?;
        let mut png = std::io::Cursor::new(vec![]);
        image::DynamicImage::ImageRgba8(picture)
            .to_rgb8()
            .write_to(&mut png, image::ImageFormat::Png)?;
        vision::read(&png.into_inner(), page_box[2], page_box[3])
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (bytes, page);
        anyhow::bail!("Recognize Text needs macOS")
    }
}

/// Pages whose own text is too little to search: scans and photos of pages.
fn needs_text(bytes: &[u8]) -> Result<Vec<bool>> {
    let pdf = hayro::hayro_syntax::Pdf::new(bytes.to_vec())
        .map_err(|e| anyhow::anyhow!("Could not read the PDF: {e:?}"))?;
    let cache = hayro::hayro_interpret::InterpreterCache::new();
    Ok(pdf
        .pages()
        .iter()
        .map(|p| {
            let text = crate::viewer::extract_page(p, &cache).text;
            text.chars().filter(|c| !c.is_whitespace()).count() < 20
        })
        .collect())
}

/// Recognize the text of every page that has none, adding it as an invisible
/// layer. `progress` hears (done, total) as pages finish.
pub fn recognize(bytes: &[u8], progress: &dyn Fn(usize, usize)) -> Result<(Vec<u8>, Report)> {
    ensure!(available(), "Recognize Text needs macOS");
    let wanted = needs_text(bytes)?;
    let total = wanted.iter().filter(|w| **w).count();
    ensure!(
        total > 0,
        "Every page already has text you can search and copy"
    );
    let mut found: Vec<(usize, Vec<Line>)> = vec![];
    for (page, _) in wanted.iter().enumerate().filter(|(_, w)| **w) {
        progress(found.len(), total);
        found.push((page, read_page(bytes, page)?));
    }
    progress(total, total);
    let report = Report {
        pages: found.iter().filter(|(_, l)| !l.is_empty()).count(),
        skipped: wanted.len() - total,
        lines: found.iter().map(|(_, l)| l.len()).sum(),
    };
    ensure!(report.lines > 0, "No text was found on the scanned pages");
    Ok((add_text_layer(bytes, &found)?, report))
}

/// Write recognized lines as invisible text over their place on each page.
pub fn add_text_layer(bytes: &[u8], pages: &[(usize, Vec<Line>)]) -> Result<Vec<u8>> {
    let mut doc = crate::content::editable(bytes)?;
    let geometry = crate::annotations::page_geometry(bytes)?;
    let all: String = pages
        .iter()
        .flat_map(|(_, lines)| lines.iter().map(|l| l.text.as_str()))
        .collect();
    let font = PdfFont::embed(&mut doc, &all)?;
    // Characters the font cannot write are left out rather than failing the page.
    let writable = |text: &str| -> String {
        text.chars()
            .map(|c| {
                if font.hex(&c.to_string()).is_ok() {
                    c
                } else {
                    ' '
                }
            })
            .collect()
    };
    let page_ids = doc.get_pages();
    for (page, lines) in pages {
        if lines.is_empty() {
            continue;
        }
        let id = *page_ids
            .get(&(*page as u32 + 1))
            .context("Page not found")?;
        let (_, transform) = geometry.get(*page).context("Page geometry missing")?;
        let mut resources = crate::content::resources(&doc, id)?;
        let mut fonts = resources
            .get(b"Font")
            .ok()
            .and_then(|o| doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok().cloned())
            .unwrap_or_default();
        let name = (0..)
            .map(|n| format!("MyPDFOCR{n}"))
            .find(|n| !fonts.has(n.as_bytes()))
            .unwrap();
        fonts.set(name.as_str(), font.id);
        resources.set("Font", fonts);
        let t = transform;
        let mut body = format!(
            "/Span << /MyPDFOCR true >> BDC q {} {} {} {} {} {} cm BT 3 Tr\n",
            t[0], t[1], t[2], t[3], t[4], t[5]
        );
        for line in lines {
            let text = writable(&line.text);
            let [x0, y0, x1, y1] = line.rect;
            let (w, h) = (x1 - x0, y1 - y0);
            if text.trim().is_empty() || w <= 0. || h <= 0. {
                continue;
            }
            let size = (h * 0.85).max(1.);
            let natural = font.width(&text, size).max(0.01);
            // Stretched to cover the words in the picture, so selection lines up.
            let scale = (100. * w / natural).clamp(10., 1000.);
            body.push_str(&format!(
                "/{name} {size} Tf {scale} Tz 1 0 0 -1 {x0} {} Tm <{}> Tj\n",
                y1 - h * 0.2,
                font.hex(&text)?
            ));
        }
        body.push_str("ET Q EMC\n");
        let layer = doc.add_object(Stream::new(
            dictionary! { MARK => "Text" },
            body.into_bytes(),
        ));
        let mut contents = content_ids(&doc, id)?;
        if !contents.is_empty() {
            let (before, after) = isolation(&doc, id)?;
            let before = doc.add_object(Stream::new(dictionary! { MARK => "Wrap" }, before));
            let after = doc.add_object(Stream::new(dictionary! { MARK => "Wrap" }, after));
            contents.insert(0, before);
            contents.push(after);
        }
        contents.push(layer);
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
    crate::content::serialize(&mut doc)
}
