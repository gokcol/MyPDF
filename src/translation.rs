//! On-device translation using Apple's installed models on Mac or a native
//! CTranslate2 engine on Windows. Model downloads are explicit and contain no text.
//!
//! Results live in memory; this module does not persist text or contact a server.
use serde::{Deserialize, Serialize};
use std::sync::mpsc;

#[cfg(any(target_os = "windows", test))]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod ctranslate;

#[cfg(target_os = "windows")]
use ctranslate as platform;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Language {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Installed,
    DownloadRequired,
    Unsupported,
}

// Deliberately not Debug: operations can contain private document text.
#[derive(Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum Operation {
    Languages,
    Check {
        source: String,
        target: String,
    },
    Download {
        source: String,
        target: String,
    },
    Translate {
        source: String,
        target: String,
        texts: Vec<String>,
    },
}

// Deliberately not Debug: translated document content is private too.
#[derive(Deserialize)]
#[serde(tag = "event", content = "value", rename_all = "snake_case")]
pub enum Event {
    Languages(Vec<Language>),
    Availability(Availability),
    Progress { completed: usize, total: usize },
    Translated { index: usize, text: String },
    Finished,
    Failed(String),
    Cancelled,
}

impl Event {
    fn terminal(&self) -> bool {
        matches!(self, Self::Finished | Self::Failed(_) | Self::Cancelled)
    }
}

/// Receive events without blocking the UI. Cancellation immediately closes the
/// Rust delivery channel, discards late results, and asks Apple to cancel work.
pub struct Job {
    id: u64,
    receiver: mpsc::Receiver<Event>,
}

impl Job {
    pub fn try_recv(&self) -> Result<Event, mpsc::TryRecvError> {
        self.receiver.try_recv()
    }

    pub fn cancel(&self) {
        platform::cancel(self.id);
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Unsupported platforms never fall back to a cloud service.
pub fn available() -> Result<(), String> {
    platform::available()
}

pub fn start(operation: Operation) -> Result<Job, String> {
    validate(&operation)?;
    available()?;
    platform::start(operation)
}

fn validate(operation: &Operation) -> Result<(), String> {
    let pair = match operation {
        Operation::Languages => None,
        Operation::Check { source, target }
        | Operation::Download { source, target }
        | Operation::Translate { source, target, .. } => Some((source, target)),
    };
    if let Some((source, target)) = pair {
        let valid = |id: &str| {
            !id.is_empty()
                && id.len() <= 48
                && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        };
        if !valid(source) || !valid(target) {
            return Err("Choose a supported source and target language.".into());
        }
        if source.eq_ignore_ascii_case(target) {
            return Err("Choose two different languages.".into());
        }
    }
    if let Operation::Translate { texts, .. } = operation {
        if texts.is_empty() || texts.iter().all(|text| text.trim().is_empty()) {
            return Err(
                "There is no text to translate. Scanned pages need local OCR first.".into(),
            );
        }
        if texts.len() > 2_000 || texts.iter().map(String::len).sum::<usize>() > 2_000_000 {
            return Err("Translate a smaller selection or fewer pages at once.".into());
        }
    }
    Ok(())
}

#[cfg(all(target_os = "macos", mypdf_apple_translation))]
mod platform {
    use super::*;
    use std::{
        collections::HashMap,
        ffi::{CStr, CString, c_char},
        sync::{
            Mutex, OnceLock,
            atomic::{AtomicU64, Ordering},
        },
    };

    type Callback = extern "C" fn(u64, *const c_char);
    unsafe extern "C" {
        fn mypdf_translation_available() -> bool;
        fn mypdf_translation_start(id: u64, request: *const c_char, callback: Callback);
        fn mypdf_translation_cancel(id: u64);
    }

    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    static SENDERS: OnceLock<Mutex<HashMap<u64, mpsc::Sender<Event>>>> = OnceLock::new();

    fn senders() -> &'static Mutex<HashMap<u64, mpsc::Sender<Event>>> {
        SENDERS.get_or_init(Mutex::default)
    }

    pub fn available() -> Result<(), String> {
        // The bridge checks OS availability before referencing the new framework.
        if unsafe { mypdf_translation_available() } {
            Ok(())
        } else {
            Err(
                "Offline Apple translation requires macOS 26 or later. No cloud fallback is used."
                    .into(),
            )
        }
    }

    pub fn start(operation: Operation) -> Result<Job, String> {
        let json = serde_json::to_string(&operation)
            .map_err(|_| "Cannot prepare the local translation request.")?;
        let request =
            CString::new(json).map_err(|_| "Cannot prepare the local translation request.")?;
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel();
        senders()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, sender);
        // Swift copies the UTF-8 request before this call returns. Callback strings
        // remain owned by Swift and are copied synchronously in receive().
        unsafe { mypdf_translation_start(id, request.as_ptr(), receive) };
        Ok(Job { id, receiver })
    }

    extern "C" fn receive(id: u64, value: *const c_char) {
        if value.is_null() {
            return;
        }
        let bytes = unsafe { CStr::from_ptr(value) }.to_bytes();
        let event = serde_json::from_slice::<Event>(bytes).unwrap_or_else(|_| {
            Event::Failed("Apple translation returned an unreadable response.".into())
        });
        let terminal = event.terminal();
        let mut senders = senders().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(sender) = senders.get(&id) {
            let _ = sender.send(event);
        }
        if terminal {
            senders.remove(&id);
        }
    }

    pub fn cancel(id: u64) {
        // Serialize cancellation and callbacks: after Cancelled, no stale result
        // can be delivered even if Apple's current inference takes time to stop.
        let sender = senders()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
        if let Some(sender) = sender {
            let _ = sender.send(Event::Cancelled);
            unsafe { mypdf_translation_cancel(id) };
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn cancelled_jobs_drop_late_native_results() {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let (sender, receiver) = mpsc::channel();
            senders().lock().unwrap().insert(id, sender);
            cancel(id);
            assert!(matches!(receiver.try_recv(), Ok(Event::Cancelled)));
            let late = CString::new(
                r#"{"event":"translated","value":{"index":0,"text":"synthetic result"}}"#,
            )
            .unwrap();
            receive(id, late.as_ptr());
            assert!(matches!(
                receiver.try_recv(),
                Err(mpsc::TryRecvError::Disconnected)
            ));
            cancel(id); // idempotent; no extra terminal events
        }
    }
}

#[cfg(not(any(
    target_os = "windows",
    all(target_os = "macos", mypdf_apple_translation)
)))]
mod platform {
    use super::*;
    pub fn available() -> Result<(), String> {
        let message = "Apple on-device translation is unavailable in this build. It requires macOS 26 or later and a build made with Xcode 26 or later. No cloud fallback is used.";
        Err(message.into())
    }
    pub fn start(_: Operation) -> Result<Job, String> {
        Err(available().unwrap_err())
    }
    pub fn cancel(_: u64) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preparation_requests_cannot_contain_document_text() {
        let json = serde_json::to_value(Operation::Download {
            source: "tr".into(),
            target: "en".into(),
        })
        .unwrap();
        assert_eq!(json.as_object().unwrap().len(), 3);
        assert!(json.get("texts").is_none());
    }

    #[test]
    fn validates_pairs_and_empty_content_before_native_call() {
        assert!(
            validate(&Operation::Check {
                source: "en".into(),
                target: "en".into()
            })
            .is_err()
        );
        assert!(
            validate(&Operation::Check {
                source: "en_US".into(),
                target: "tr".into()
            })
            .is_err()
        );
        assert!(
            validate(&Operation::Check {
                source: "zh-Hans".into(),
                target: "tr".into()
            })
            .is_ok()
        );
        assert!(
            validate(&Operation::Translate {
                source: "de".into(),
                target: "en".into(),
                texts: vec![" \n".into()]
            })
            .is_err()
        );
    }

    #[test]
    fn wire_events_preserve_unicode_and_terminal_semantics() {
        let event: Event = serde_json::from_str(
            r#"{"event":"translated","value":{"index":1,"text":"İyi günler"}}"#,
        )
        .unwrap();
        assert!(!event.terminal());
        match event {
            Event::Translated { index, text } => {
                assert_eq!(index, 1);
                assert_eq!(text, "İyi günler");
            }
            _ => panic!("expected translated event"),
        }
        assert!(Event::Cancelled.terminal());
        assert!(Event::Failed("Failed".into()).terminal());
    }

    #[cfg(all(target_os = "macos", mypdf_apple_translation))]
    #[test]
    #[ignore = "requires a logged-in macOS 26+ session; queries Apple locally without downloading models"]
    fn apple_language_catalog_and_pair_availability() {
        assert!(
            std::process::Command::new("sh")
                .arg("native/translation/smoke.sh")
                .status()
                .expect("run native Apple translation smoke")
                .success()
        );
    }
}
