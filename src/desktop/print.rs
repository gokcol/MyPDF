//! Printing through the system's print panel: the document as it looks now,
//! with annotations and form values.
use super::*;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "windows")]
pub(super) struct State {
    owner: isize,
    pending: Option<mpsc::Receiver<Result<String, String>>>,
}

#[cfg(target_os = "windows")]
impl State {
    pub fn new(handle: &impl raw_window_handle::HasWindowHandle) -> Self {
        let owner = handle
            .window_handle()
            .ok()
            .and_then(|handle| match handle.as_raw() {
                raw_window_handle::RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
                _ => None,
            })
            .unwrap_or(0);
        Self {
            owner,
            pending: None,
        }
    }

    fn start(
        &mut self,
        bytes: Vec<u8>,
        pages: Vec<viewer::PageSize>,
        name: String,
        ctx: &egui::Context,
    ) -> anyhow::Result<bool> {
        anyhow::ensure!(
            self.pending.is_none(),
            "A document is already being sent to the printer."
        );
        let Some(job) = windows::choose_printer(self.owner, pages.len())? else {
            return Ok(false);
        };
        let (tx, rx) = mpsc::channel();
        let wake = ctx.clone();
        std::thread::Builder::new()
            .name("MyPDF printing".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    job.spool(&bytes, &pages, &name, None)
                }))
                .map_err(|_| "The print worker stopped unexpectedly.".to_owned())
                .and_then(|result| {
                    result
                        .map(|()| format!("Sent {name} to the printer"))
                        .map_err(|e| format!("{e:#}"))
                });
                let _ = tx.send(result);
                wake.request_repaint();
            })?;
        self.pending = Some(rx);
        Ok(true)
    }

    pub fn poll(&mut self) -> Option<(String, bool)> {
        let result = match self.pending.as_ref()?.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("The print worker stopped unexpectedly.".into())
            }
        };
        self.pending = None;
        Some(match result {
            Ok(message) => (message, false),
            Err(error) => (format!("Could not print: {error}"), true),
        })
    }
}

/// Show the macOS print panel for a PDF file and print it. Returns whether it
/// was printed (false when the panel was cancelled).
#[cfg(target_os = "macos")]
fn print_file(path: &std::path::Path) -> anyhow::Result<bool> {
    use objc2::msg_send;
    use objc2::runtime::Bool;
    let operation = print_operation(path)?;
    // SAFETY: AppKit objects used on the main thread, as the print panel requires.
    unsafe {
        let _: () = msg_send![operation.0, setShowsPrintPanel: Bool::YES];
        let _: () = msg_send![operation.0, setShowsProgressPanel: Bool::YES];
        let printed: Bool = msg_send![operation.0, runOperation];
        Ok(printed.as_bool())
    }
}

/// A PDFKit print operation for the file, fitted to the paper.
#[cfg(target_os = "macos")]
struct Operation(
    *mut objc2::runtime::AnyObject,
    *mut objc2::runtime::AnyObject,
);
#[cfg(target_os = "macos")]
impl Drop for Operation {
    fn drop(&mut self) {
        // SAFETY: the document was created with alloc/init and is released once.
        unsafe {
            let _: () = objc2::msg_send![self.1, release];
        }
    }
}
#[cfg(target_os = "macos")]
fn print_operation(path: &std::path::Path) -> anyhow::Result<Operation> {
    use anyhow::{Context as _, ensure};
    use objc2::runtime::{AnyObject, Bool};
    use objc2::{class, msg_send};
    #[link(name = "PDFKit", kind = "framework")]
    unsafe extern "C" {}
    let path = std::ffi::CString::new(path.to_str().context("Unusual file name")?)?;
    // SAFETY: Foundation and PDFKit calls with valid, NUL-terminated input.
    unsafe {
        let string: *mut AnyObject =
            msg_send![class!(NSString), stringWithUTF8String: path.as_ptr()];
        let url: *mut AnyObject = msg_send![class!(NSURL), fileURLWithPath: string];
        let document: *mut AnyObject = msg_send![class!(PDFDocument), alloc];
        let document: *mut AnyObject = msg_send![document, initWithURL: url];
        ensure!(
            !document.is_null(),
            "macOS could not prepare this document for printing"
        );
        let info: *mut AnyObject = msg_send![class!(NSPrintInfo), sharedPrintInfo];
        // kPDFPrintPageScaleDownToFit: large pages shrink to the paper, small ones stay as they are.
        let operation: *mut AnyObject = msg_send![
            document,
            printOperationForPrintInfo: info,
            scalingMode: 2isize,
            autoRotate: Bool::YES
        ];
        let operation = Operation(operation, document);
        ensure!(
            !operation.0.is_null(),
            "macOS could not prepare this document for printing"
        );
        Ok(operation)
    }
}

impl Viewer {
    /// Print the active document as it looks now.
    pub(super) fn print_document(&mut self, ctx: &egui::Context) {
        let _ = ctx;
        if !self.apply_pending_annotation() {
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let Some(session) = &tab.session else {
            self.status = Some(("Wait until the document has opened.".into(), false));
            return;
        };
        let bytes = session.snapshot_bytes();
        let name = tab.name();
        #[cfg(target_os = "windows")]
        {
            self.status = match bytes
                .and_then(|bytes| self.printing.start(bytes, tab.pages.clone(), name, ctx))
            {
                Ok(true) => Some(("Sending document to the printer…".into(), false)),
                Ok(false) => None,
                Err(error) => Some((format!("Could not print: {error:#}"), true)),
            };
        }
        #[cfg(not(target_os = "windows"))]
        {
            // A private temporary copy, removed once printing is done.
            let file = bytes.and_then(|bytes| {
                let mut file = tempfile::Builder::new()
                    .prefix("MyPDF-print-")
                    .suffix(".pdf")
                    .tempfile()?;
                std::io::Write::write_all(&mut file, &bytes)?;
                Ok(file)
            });
            let file = match file {
                Ok(file) => file,
                Err(e) => {
                    self.status = Some((format!("Could not prepare printing: {e:#}"), true));
                    return;
                }
            };
            #[cfg(target_os = "macos")]
            {
                let result =
                    menu::with_file_dialog(self._native_menu.as_ref(), || print_file(file.path()));
                self.status = Some(match result {
                    Ok(true) => (format!("Sent {name} to the printer"), false),
                    Ok(false) => (String::new(), false),
                    Err(e) => (format!("Could not print: {e:#}"), true),
                });
                if self.status.as_ref().is_some_and(|(s, _)| s.is_empty()) {
                    self.status = None;
                }
            }
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            {
                // Elsewhere the system's PDF reader prints it.
                let (copy, path) = match file.keep() {
                    Ok(kept) => kept,
                    Err(e) => {
                        self.status = Some((format!("Could not prepare printing: {e}"), true));
                        return;
                    }
                };
                drop(copy);
                navigate::open_external(&path.to_string_lossy());
                self.status = Some((format!("Opened {name} in your PDF reader to print"), false));
            }
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    #[test]
    fn macos_prepares_a_print_job_for_the_document() {
        let font = mypdf_milestone0::document::default_font().unwrap();
        let mut file = tempfile::Builder::new().suffix(".pdf").tempfile().unwrap();
        std::io::Write::write_all(
            &mut file,
            &mypdf_milestone0::document::fixture(&font).unwrap(),
        )
        .unwrap();
        // The print panel itself needs a person; the job behind it is checked here.
        let operation = print_operation(file.path()).unwrap();
        assert!(!operation.0.is_null());
    }
}

#[cfg(all(test, target_os = "windows"))]
mod windows_state_tests {
    use super::*;

    #[test]
    fn printing_reports_failures_and_prevents_duplicate_jobs() {
        let (tx, rx) = mpsc::channel();
        let mut state = State {
            owner: 0,
            pending: Some(rx),
        };
        assert!(state.poll().is_none());
        assert!(
            state
                .start(vec![], vec![], "test".into(), &egui::Context::default())
                .is_err()
        );
        tx.send(Err("Printer is offline".into())).unwrap();
        assert_eq!(
            state.poll(),
            Some(("Could not print: Printer is offline".into(), true))
        );
        assert!(state.pending.is_none());
        let (tx, rx) = mpsc::channel();
        state.pending = Some(rx);
        tx.send(Ok("Sent test to the printer".into())).unwrap();
        assert_eq!(
            state.poll(),
            Some(("Sent test to the printer".into(), false))
        );
        assert!(state.pending.is_none());
    }
}
