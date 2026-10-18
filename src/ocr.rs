//! Recognize Text (OCR): scanned pages get an invisible text layer, so their
//! words can be searched, selected and copied. Recognition uses the text
//! recognition built into macOS (Vision), on this computer.
use crate::header_footer::{content_ids, isolation};
use crate::pdf_font::PdfFont;
use anyhow::{Context as _, Result, ensure};
use lopdf::{Object, Stream, dictionary};
use std::sync::{
    Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
};

const MARK: &[u8] = b"MyPDFOCR";
/// Pages are read at this many pixels per point (about 216 dpi).
#[cfg(target_os = "macos")]
const SCALE: f32 = 3.0;

/// Cancellation is cooperative during PDF rendering and immediate for an
/// active Vision request. No partially recognized document is returned.
#[derive(Default)]
pub struct Cancellation {
    cancelled: AtomicBool,
    changed: Condvar,
    wait: Mutex<()>,
}

impl Cancellation {
    pub fn cancel(&self) {
        let _guard = self.wait.lock().unwrap_or_else(|e| e.into_inner());
        self.cancelled.store(true, Ordering::Release);
        self.changed.notify_all();
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    pub fn check(&self) -> Result<()> {
        ensure!(
            !self.is_cancelled(),
            "Text recognition cancelled. The document is unchanged."
        );
        Ok(())
    }

    /// Join the cancellation observer before returning, including on unwind.
    /// The native request can therefore be released safely by the caller.
    #[cfg(any(target_os = "macos", test))]
    fn run<T>(&self, work: impl FnOnce() -> T, cancel: impl FnOnce() + Send) -> T {
        struct Finished<'a> {
            token: &'a Cancellation,
            done: &'a AtomicBool,
        }
        impl Drop for Finished<'_> {
            fn drop(&mut self) {
                let _guard = self.token.wait.lock().unwrap_or_else(|e| e.into_inner());
                self.done.store(true, Ordering::Release);
                self.token.changed.notify_all();
            }
        }
        let done = AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let mut guard = self.wait.lock().unwrap_or_else(|e| e.into_inner());
                while !done.load(Ordering::Acquire) && !self.is_cancelled() {
                    guard = self.changed.wait(guard).unwrap_or_else(|e| e.into_inner());
                }
                let requested = self.is_cancelled() && !done.load(Ordering::Acquire);
                drop(guard);
                if requested {
                    cancel();
                }
            });
            let _finished = Finished {
                token: self,
                done: &done,
            };
            work()
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Preparing,
    Inspecting,
    Rendering,
    Recognizing,
    Applying,
}
impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Preparing => "Preparing document",
            Self::Inspecting => "Checking existing text",
            Self::Rendering => "Rendering scanned page",
            Self::Recognizing => "Recognizing text",
            Self::Applying => "Adding searchable text",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub phase: Phase,
    /// One-based document page (zero before the page count is known).
    pub page: usize,
    pub pages: usize,
    pub done: usize,
    pub total: usize,
}

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
    use super::{Cancellation, Line};
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
    pub fn read(
        image: &[u8],
        width: f32,
        height: f32,
        cancellation: &Cancellation,
    ) -> Result<Vec<Line>> {
        cancellation.check()?;
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
                let _: () = msg_send![request, release];
                let _: () = msg_send![handler, release];
                bail!("Text recognition is not available on this Mac");
            }
            // Accurate recognition, with language correction.
            let _: () = msg_send![request, setRecognitionLevel: 0isize];
            let _: () = msg_send![request, setUsesLanguageCorrection: Bool::YES];
            let _: () = msg_send![request, setPreferBackgroundProcessing: Bool::YES];
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
            // VNRequest.cancel is designed to interrupt an executing request.
            // The scoped observer is joined before either object is released;
            // only the cancel message is sent from that observer thread.
            let request_address = request as usize;
            let done: Bool = cancellation.run(
                || msg_send![handler, performRequests: requests, error: &mut error],
                move || {
                    objc2::rc::autoreleasepool(|_| {
                        let request = request_address as *mut AnyObject;
                        let _: () = msg_send![request, cancel];
                    });
                },
            );
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
            cancellation.check()?;
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
    read_page_cancellable(bytes, page, &Cancellation::default(), &|| {})
}

/// Read one page without changing its PDF; cancellation reaches native OCR.
pub fn read_page_cancellable(
    bytes: &[u8],
    page: usize,
    cancellation: &Cancellation,
    recognizing: &dyn Fn(),
) -> Result<Vec<Line>> {
    cancellation.check()?;
    #[cfg(target_os = "macos")]
    {
        let geometry = crate::annotations::page_geometry(bytes)?;
        let (page_box, _) = geometry.get(page).context("Page not found")?;
        let ([w, h], rgba) = crate::viewer::render_page(bytes, page, SCALE)?;
        cancellation.check()?;
        let picture =
            image::RgbaImage::from_raw(w as u32, h as u32, rgba).context("Page picture")?;
        let mut png = std::io::Cursor::new(vec![]);
        image::DynamicImage::ImageRgba8(picture)
            .to_rgb8()
            .write_to(&mut png, image::ImageFormat::Png)?;
        cancellation.check()?;
        recognizing();
        cancellation.check()?;
        vision::read(&png.into_inner(), page_box[2], page_box[3], cancellation)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (bytes, page, recognizing);
        anyhow::bail!("Recognize Text needs macOS")
    }
}

/// Pages whose own text is too little to search: scans and photos of pages.
fn needs_text(
    bytes: &[u8],
    cancellation: &Cancellation,
    progress: &dyn Fn(Progress),
) -> Result<Vec<bool>> {
    cancellation.check()?;
    let pdf = hayro::hayro_syntax::Pdf::new(bytes.to_vec())
        .map_err(|e| anyhow::anyhow!("Could not read the PDF: {e:?}"))?;
    let cache = hayro::hayro_interpret::InterpreterCache::new();
    pdf.pages()
        .iter()
        .enumerate()
        .map(|(i, p)| {
            cancellation.check()?;
            progress(Progress {
                phase: Phase::Inspecting,
                page: i + 1,
                pages: pdf.pages().len(),
                ..Default::default()
            });
            let text = crate::viewer::extract_page(p, &cache).text;
            cancellation.check()?;
            Ok(text.chars().filter(|c| !c.is_whitespace()).count() < 20)
        })
        .collect::<Result<_>>()
}

/// Recognize the text of every page that has none, adding it as an invisible
/// layer. `progress` hears (done, total) as pages finish.
pub fn recognize(bytes: &[u8], progress: &dyn Fn(usize, usize)) -> Result<(Vec<u8>, Report)> {
    recognize_cancellable(bytes, &Cancellation::default(), &|state| {
        if state.total > 0 {
            progress(state.done, state.total);
        }
    })
}

pub fn recognize_cancellable(
    bytes: &[u8],
    cancellation: &Cancellation,
    progress: &dyn Fn(Progress),
) -> Result<(Vec<u8>, Report)> {
    cancellation.check()?;
    ensure!(available(), "Recognize Text needs macOS");
    progress(Progress::default());
    let wanted = needs_text(bytes, cancellation, progress)?;
    recognize_pages(
        bytes,
        &wanted,
        cancellation,
        progress,
        &|page, recognizing| read_page_cancellable(bytes, page, cancellation, recognizing),
    )
}

type PageReader<'a> = dyn Fn(usize, &dyn Fn()) -> Result<Vec<Line>> + 'a;

fn recognize_pages(
    bytes: &[u8],
    wanted: &[bool],
    cancellation: &Cancellation,
    progress: &dyn Fn(Progress),
    read: &PageReader<'_>,
) -> Result<(Vec<u8>, Report)> {
    let total = wanted.iter().filter(|w| **w).count();
    ensure!(
        total > 0,
        "Every page already has text you can search and copy"
    );
    let mut found: Vec<(usize, Vec<Line>)> = vec![];
    for (page, _) in wanted.iter().enumerate().filter(|(_, w)| **w) {
        cancellation.check()?;
        let state = Progress {
            phase: Phase::Rendering,
            page: page + 1,
            pages: wanted.len(),
            done: found.len(),
            total,
        };
        progress(state);
        cancellation.check()?;
        let lines = read(page, &|| {
            progress(Progress {
                phase: Phase::Recognizing,
                ..state
            });
        })?;
        cancellation.check()?;
        found.push((page, lines));
    }
    let report = Report {
        pages: found.iter().filter(|(_, l)| !l.is_empty()).count(),
        skipped: wanted.len() - total,
        lines: found.iter().map(|(_, l)| l.len()).sum(),
    };
    ensure!(report.lines > 0, "No text was found on the scanned pages");
    progress(Progress {
        phase: Phase::Applying,
        page: wanted.len(),
        pages: wanted.len(),
        done: total,
        total,
    });
    cancellation.check()?;
    let result = add_text_layer(bytes, &found)?;
    cancellation.check()?;
    Ok((result, report))
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

#[cfg(test)]
mod cancellation_tests {
    use super::*;

    #[test]
    fn cancellation_before_start_does_not_parse_or_modify_the_source() {
        let source = b"not a PDF".to_vec();
        let cancellation = Cancellation::default();
        cancellation.cancel();
        let error =
            recognize_cancellable(&source, &cancellation, &|_| panic!("started")).unwrap_err();
        assert!(error.to_string().contains("cancelled"));
        assert_eq!(source, b"not a PDF");
    }

    #[test]
    fn cancel_between_pages_and_before_apply_returns_no_partial_document() {
        let source = crate::convert::blank(200., 200.).unwrap();
        for phase in [Phase::Rendering, Phase::Recognizing, Phase::Applying] {
            let cancellation = Cancellation::default();
            let original = source.clone();
            let error = recognize_pages(
                &source,
                &[true, false, true],
                &cancellation,
                &|state| {
                    if state.phase == phase {
                        cancellation.cancel();
                    }
                },
                &|_, recognizing| {
                    recognizing();
                    Ok(vec![Line {
                        text: "A scanned line".into(),
                        rect: [20., 20., 160., 40.],
                    }])
                },
            )
            .unwrap_err();
            assert!(
                error.to_string().contains("cancelled"),
                "{phase:?}: {error}"
            );
            assert_eq!(source, original);
        }
    }

    #[test]
    fn native_cancellation_observer_exits_on_completion_and_cancellation() {
        let cancellation = Cancellation::default();
        assert_eq!(cancellation.run(|| 7, || panic!("unexpected cancel")), 7);
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (interrupted_tx, interrupted_rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let token = &cancellation;
            let task = scope.spawn(move || {
                token.run(
                    || {
                        started_tx.send(()).unwrap();
                        interrupted_rx
                            .recv_timeout(std::time::Duration::from_secs(5))
                            .unwrap();
                    },
                    || {
                        interrupted_tx.send(()).unwrap();
                    },
                )
            });
            started_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            cancellation.cancel();
            task.join().unwrap();
        });
        assert!(cancellation.is_cancelled());
    }

    #[test]
    fn cancellation_observer_is_joined_if_the_worker_panics() {
        let cancellation = Cancellation::default();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cancellation.run(
                || panic!("test worker failure"),
                || panic!("unexpected cancellation"),
            );
        }));
        assert!(result.is_err());
        // No observer survives the unwind; the same token remains usable.
        assert_eq!(
            cancellation.run(|| 12, || panic!("unexpected cancellation")),
            12
        );
    }
}
