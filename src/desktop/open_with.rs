//! Files the system asks MyPDF to open: Open With, a PDF dropped on the Dock
//! icon, or a double-click once MyPDF is the default PDF app.
//!
//! macOS sends them to the MyPDF that is already open. On Windows, Open With
//! starts another MyPDF with the file; it hands the file to the one already
//! open and quits, so documents open as tabs in one window.
use eframe::egui;
use std::path::PathBuf;
use std::sync::Mutex;

/// Files to open, and whether the window should come to the front.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Arrivals {
    pub paths: Vec<PathBuf>,
    pub focus: bool,
}
impl Arrivals {
    pub fn activate(&self, ctx: &egui::Context) {
        if self.focus {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
    }
}
static WAITING: Mutex<Arrivals> = Mutex::new(Arrivals {
    paths: Vec::new(),
    focus: false,
});
static WAKE: Mutex<Option<egui::Context>> = Mutex::new(None);

fn deliver(paths: Vec<PathBuf>, focus: bool) {
    if paths.is_empty() && !focus {
        return;
    }
    {
        let mut waiting = WAITING.lock().unwrap_or_else(|e| e.into_inner());
        waiting.paths.extend(paths);
        waiting.focus |= focus || !waiting.paths.is_empty();
    }
    if let Some(ctx) = WAKE.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        ctx.request_repaint();
    }
}

/// What has arrived since the last frame. Anything later wakes `ctx`.
pub(super) fn take(ctx: &egui::Context) -> Arrivals {
    WAKE.lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(|| ctx.clone());
    std::mem::take(&mut *WAITING.lock().unwrap_or_else(|e| e.into_inner()))
}

/// When another MyPDF is already open, give it these files (or just bring it
/// to the front) and return true: this one should then quit.
#[cfg(not(target_os = "macos"))]
pub fn hand_over(paths: &[PathBuf]) -> bool {
    mailbox::hand_over(&mypdf_milestone0::assets::data_directory(), paths)
}
#[cfg(target_os = "macos")]
pub fn hand_over(_: &[PathBuf]) -> bool {
    false
}

/// Listen for files the system asks MyPDF to open. Call before the app
/// starts, so the files a launch was asked to open are not missed. (On
/// Windows the installer lists MyPDF under Open With, when the user agrees.)
pub fn install() {
    #[cfg(target_os = "macos")]
    mac::install();
}

/// Requests from MyPDFs started with a file while another one is open: each
/// is a small file in the data folder, picked up by the open one.
#[cfg(any(not(target_os = "macos"), test))]
mod mailbox {
    use std::fs::File;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    /// Requests older than this were left behind when MyPDF quit.
    const STALE: Duration = Duration::from_secs(30);

    pub(super) enum Claim {
        /// This MyPDF receives files; the lock is held while it runs.
        Mine(Option<File>),
        /// Another MyPDF is open.
        Taken,
    }
    pub(super) fn claim(root: &Path) -> Claim {
        let _ = std::fs::create_dir_all(root);
        let Ok(file) = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.join("instance.lock"))
        else {
            return Claim::Mine(None);
        };
        match file.try_lock() {
            Ok(()) => Claim::Mine(Some(file)),
            Err(std::fs::TryLockError::WouldBlock) => Claim::Taken,
            // Without file locks, each MyPDF opens its own window.
            Err(std::fs::TryLockError::Error(_)) => Claim::Mine(None),
        }
    }

    pub(super) fn send(root: &Path, paths: &[PathBuf]) -> std::io::Result<()> {
        let mut names = vec![];
        for path in paths {
            let path = std::path::absolute(path)?;
            names.push(
                path.to_str()
                    .ok_or_else(|| std::io::Error::other("unusual file name"))?
                    .to_owned(),
            );
        }
        let inbox = root.join("open");
        std::fs::create_dir_all(&inbox)?;
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let name = format!("{time:024}-{}", std::process::id());
        let part = inbox.join(format!("{name}.part"));
        std::fs::write(&part, serde_json::to_vec(&names)?)?;
        std::fs::rename(&part, inbox.join(format!("{name}.request")))
    }

    /// The files of every waiting request, oldest first, or None when there
    /// is no request. Each request is taken once.
    pub(super) fn receive(root: &Path) -> Option<Vec<PathBuf>> {
        let mut requests: Vec<PathBuf> = std::fs::read_dir(root.join("open"))
            .ok()?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|e| e == "request"))
            .collect();
        requests.sort();
        let mut paths = vec![];
        let mut any = false;
        for request in requests {
            let fresh = std::fs::metadata(&request)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|time| time.elapsed().ok())
                .is_none_or(|age| age < STALE);
            let bytes = std::fs::read(&request);
            let _ = std::fs::remove_file(&request);
            if fresh
                && let Ok(bytes) = bytes
                && let Ok(names) = serde_json::from_slice::<Vec<String>>(&bytes)
            {
                any = true;
                paths.extend(names.into_iter().map(PathBuf::from));
            }
        }
        any.then_some(paths)
    }

    #[cfg(not(target_os = "macos"))]
    pub(super) fn hand_over(root: &Path, paths: &[PathBuf]) -> bool {
        match claim(root) {
            Claim::Mine(lock) => {
                let root = root.to_path_buf();
                std::thread::spawn(move || {
                    let _lock = lock;
                    loop {
                        std::thread::sleep(Duration::from_millis(250));
                        if let Some(paths) = receive(&root) {
                            super::deliver(paths, true);
                        }
                    }
                });
                false
            }
            Claim::Taken => {
                // Let the open MyPDF come to the front when it gets the files.
                #[cfg(windows)]
                // SAFETY: no pointers; it only grants permission.
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(
                        windows_sys::Win32::UI::WindowsAndMessaging::ASFW_ANY,
                    );
                }
                send(root, paths).is_ok()
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject};
    use objc2::{ClassType, class, define_class, msg_send, sel};
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;

    /// kCoreEventClass, kAEOpenDocuments and keyDirectObject.
    const CORE_EVENTS: u32 = u32::from_be_bytes(*b"aevt");
    const OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");
    const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and OpenHandler
        // does not implement Drop.
        #[unsafe(super(NSObject))]
        #[name = "MyPDFOpenHandler"]
        struct OpenHandler;

        impl OpenHandler {
            #[unsafe(method(applicationWillFinishLaunching:))]
            fn will_finish_launching(&self, _: *mut AnyObject) {
                // AppKit installs its own handler while launching; this is the
                // moment Apple asks apps to take the event over.
                listen(self);
            }

            #[unsafe(method(openDocuments:withReply:))]
            fn open_documents(&self, event: *mut AnyObject, _: *mut AnyObject) {
                // SAFETY: AppKit passes the open-documents Apple event.
                super::deliver(unsafe { paths(event) }, true);
            }
        }
    );

    fn listen(handler: &OpenHandler) {
        // SAFETY: the handler lives for the rest of the process.
        unsafe {
            let manager: *mut AnyObject =
                msg_send![class!(NSAppleEventManager), sharedAppleEventManager];
            let _: () = msg_send![
                manager,
                setEventHandler: handler,
                andSelector: sel!(openDocuments:withReply:),
                forEventClass: CORE_EVENTS,
                andEventID: OPEN_DOCUMENTS
            ];
        }
    }

    /// The files named in an open-documents event.
    unsafe fn paths(event: *mut AnyObject) -> Vec<PathBuf> {
        if event.is_null() {
            return vec![];
        }
        // SAFETY: Apple event descriptors, read on the main thread.
        unsafe {
            let list: *mut AnyObject = msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT];
            if list.is_null() {
                return vec![];
            }
            let count: isize = msg_send![list, numberOfItems];
            let items: Vec<*mut AnyObject> = if count == 0 {
                vec![list]
            } else {
                (1..=count)
                    .map(|i| msg_send![list, descriptorAtIndex: i])
                    .collect()
            };
            items
                .into_iter()
                .filter_map(|item| {
                    let url: *mut AnyObject = msg_send![item, fileURLValue];
                    if url.is_null() {
                        return None;
                    }
                    let path: *mut AnyObject = msg_send![url, path];
                    let text: *const std::ffi::c_char = msg_send![path, UTF8String];
                    (!text.is_null()).then(|| {
                        let bytes = std::ffi::CStr::from_ptr(text).to_bytes();
                        PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
                    })
                })
                .collect()
        }
    }

    pub(super) fn install() {
        // SAFETY: Foundation calls with valid arguments; the handler is kept
        // for the rest of the process because neither center retains it.
        unsafe {
            let handler: Retained<OpenHandler> = msg_send![OpenHandler::class(), new];
            let handler = Retained::into_raw(handler);
            listen(&*handler);
            let name = c"NSApplicationWillFinishLaunchingNotification";
            let name: *mut AnyObject =
                msg_send![class!(NSString), stringWithUTF8String: name.as_ptr()];
            let center: *mut AnyObject = msg_send![class!(NSNotificationCenter), defaultCenter];
            let _: () = msg_send![
                center,
                addObserver: handler,
                selector: sel!(applicationWillFinishLaunching:),
                name: name,
                object: std::ptr::null_mut::<AnyObject>()
            ];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivered_files_wait_until_taken_and_wake_the_app() {
        let ctx = egui::Context::default();
        assert_eq!(take(&ctx), Arrivals::default());
        deliver(
            vec![PathBuf::from("/tmp/a.pdf"), PathBuf::from("/tmp/b.pdf")],
            false,
        );
        assert_eq!(
            take(&ctx),
            Arrivals {
                paths: vec![PathBuf::from("/tmp/a.pdf"), PathBuf::from("/tmp/b.pdf")],
                focus: true,
            }
        );
        assert_eq!(take(&ctx), Arrivals::default(), "each file is opened once");
        deliver(vec![], true);
        assert!(
            take(&ctx).focus,
            "a reopen request also activates the window"
        );
    }

    #[test]
    fn external_open_restores_and_raises_once() {
        let ctx = egui::Context::default();
        let output = ctx.run(Default::default(), |ctx| {
            Arrivals {
                paths: vec![],
                focus: true,
            }
            .activate(ctx);
        });
        let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
        assert_eq!(
            commands,
            &[
                egui::ViewportCommand::Minimized(false),
                egui::ViewportCommand::Visible(true),
                egui::ViewportCommand::Focus,
            ]
        );
        let output = ctx.run(Default::default(), |ctx| Arrivals::default().activate(ctx));
        assert!(
            output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .is_empty(),
            "ordinary frames must not steal focus"
        );
    }

    #[test]
    fn a_second_mypdf_hands_its_files_to_the_open_one() {
        let root = tempfile::tempdir().unwrap();
        let mailbox::Claim::Mine(Some(_lock)) = mailbox::claim(root.path()) else {
            panic!("the first MyPDF receives files");
        };
        assert!(matches!(mailbox::claim(root.path()), mailbox::Claim::Taken));
        assert_eq!(mailbox::receive(root.path()), None);
        let pdf = root.path().join("Ünïcode name.pdf");
        mailbox::send(root.path(), std::slice::from_ref(&pdf)).unwrap();
        mailbox::send(root.path(), &[]).unwrap();
        assert_eq!(mailbox::receive(root.path()), Some(vec![pdf]));
        assert_eq!(mailbox::receive(root.path()), None, "taken once");
        // A request left from a MyPDF that quit long ago is dropped.
        mailbox::send(root.path(), &[PathBuf::from("/old.pdf")]).unwrap();
        let request = std::fs::read_dir(root.path().join("open"))
            .unwrap()
            .flatten()
            .next()
            .unwrap()
            .path();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(120);
        std::fs::File::options()
            .write(true)
            .open(&request)
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert_eq!(mailbox::receive(root.path()), None);
        assert!(!request.exists());
    }
}
