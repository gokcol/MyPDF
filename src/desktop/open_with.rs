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
        waiting.focus |= focus;
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

/// Listen for files the system asks MyPDF to open, and on Windows list
/// MyPDF under Open With. Call before the app starts, so the files a launch
/// was asked to open are not missed.
pub fn install() {
    #[cfg(target_os = "macos")]
    mac::install();
    #[cfg(windows)]
    windows::register();
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

/// Open With for PDFs, for this Windows user. Windows keeps the user's own
/// choice of default app; this only adds MyPDF to the list.
#[cfg(windows)]
mod windows {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
        RegCloseKey, RegCreateKeyExW, RegGetValueW, RegSetValueExW,
    };
    use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain([0]).collect()
    }
    /// A text value under HKEY_CURRENT_USER.
    fn read(key: &str, name: Option<&str>) -> Option<String> {
        let (key, name) = (wide(key), name.map(wide));
        let name = name.as_ref().map_or(null(), |n| n.as_ptr());
        let mut buffer = vec![0u16; 1024];
        let mut size = (buffer.len() * 2) as u32;
        // SAFETY: NUL-terminated names and a buffer of the given size.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name,
                RRF_RT_REG_SZ,
                null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if status != 0 {
            return None;
        }
        let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
        Some(String::from_utf16_lossy(&buffer[..end]))
    }
    fn write(key: &str, name: Option<&str>, value: &str) -> bool {
        let (key, name, value) = (wide(key), name.map(wide), wide(value));
        let mut handle: HKEY = null_mut();
        // SAFETY: NUL-terminated names and value; the key is closed again.
        unsafe {
            if RegCreateKeyExW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                0,
                null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                null(),
                &mut handle,
                null_mut(),
            ) != 0
            {
                return false;
            }
            let written = RegSetValueExW(
                handle,
                name.as_ref().map_or(null(), |n| n.as_ptr()),
                0,
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32,
            ) == 0;
            RegCloseKey(handle);
            written
        }
    }

    pub(super) fn register() {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let (Some(path), Some(file)) = (exe.to_str(), exe.file_name().and_then(|n| n.to_str()))
        else {
            return;
        };
        let command = format!("\"{path}\" \"%1\"");
        let classes = r"Software\Classes";
        let app = format!(r"{classes}\Applications\{file}");
        let app_command = format!(r"{app}\shell\open\command");
        // Already listed from this place.
        if read(&app_command, None).as_deref() == Some(command.as_str()) {
            return;
        }
        let document = format!(r"{classes}\MyPDF.Document");
        let listed = [
            write(&app, Some("FriendlyAppName"), "MyPDF"),
            write(&format!(r"{app}\SupportedTypes"), Some(".pdf"), ""),
            write(&document, None, "PDF Document"),
            write(&format!(r"{document}\shell\open\command"), None, &command),
            write(
                &format!(r"{classes}\.pdf\OpenWithProgids"),
                Some("MyPDF.Document"),
                "",
            ),
            // Last, so a launch that could not finish tries again.
            write(&app_command, None, &command),
        ];
        if listed.contains(&true) {
            // SAFETY: no items; it tells Explorer that file types changed.
            unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, null(), null()) };
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
                super::deliver(unsafe { paths(event) }, false);
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
        deliver(vec![], true);
        assert_eq!(
            take(&ctx),
            Arrivals {
                paths: vec![PathBuf::from("/tmp/a.pdf"), PathBuf::from("/tmp/b.pdf")],
                focus: true,
            }
        );
        assert_eq!(take(&ctx), Arrivals::default(), "each file is opened once");
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
