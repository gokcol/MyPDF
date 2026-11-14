//! Main-thread-only bridge to Sparkle's native, signed update installer.
use std::ffi::{CStr, c_char};

unsafe extern "C" {
    fn mypdf_updater_check(wake: extern "C" fn(), background: bool) -> *const c_char;
    fn mypdf_updater_take_quit_request() -> bool;
    fn mypdf_updater_resume_installation() -> bool;
}

static CONTEXT: std::sync::OnceLock<eframe::egui::Context> = std::sync::OnceLock::new();
extern "C" fn wake() {
    if let Some(ctx) = CONTEXT.get() {
        ctx.request_repaint();
    }
}

pub(super) fn check(ctx: &eframe::egui::Context, background: bool) -> Result<(), String> {
    if cfg!(test) {
        return Ok(());
    }
    CONTEXT.get_or_init(|| ctx.clone());
    let error = unsafe { mypdf_updater_check(wake, background) };
    if error.is_null() {
        Ok(())
    } else {
        Err(unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned())
    }
}

pub(super) fn take_quit_request() -> bool {
    !cfg!(test) && unsafe { mypdf_updater_take_quit_request() }
}

pub(super) fn resume_installation() -> bool {
    !cfg!(test) && unsafe { mypdf_updater_resume_installation() }
}
