//! The compact tab strip occupies the transparent macOS title bar.
//! AppKit must not turn a document-tab gesture into a native window drag.

#[cfg(target_os = "macos")]
pub(super) fn configure(handle: &impl raw_window_handle::HasWindowHandle) {
    with_window(handle, |window| {
        // SAFETY: this is the eframe-owned NSWindow, on the main UI thread.
        unsafe {
            let _: () = objc2::msg_send![window, setMovable: objc2::runtime::Bool::NO];
            let _: () =
                objc2::msg_send![window, setMovableByWindowBackground: objc2::runtime::Bool::NO];
        }
    });
}

pub(super) fn begin(frame: &eframe::Frame, ctx: &eframe::egui::Context) {
    if !ctx.input(|i| i.pointer.primary_down() && !i.viewport().fullscreen.unwrap_or(false)) {
        return;
    }
    #[cfg(target_os = "macos")]
    {
        with_window(frame, |window| {
            // Only an explicit gesture that began on empty chrome may move
            // the window. AppKit's drag loop is synchronous; restore the guard
            // as soon as the mouse is released.
            // SAFETY: AppKit is accessed only from eframe's main UI thread.
            unsafe {
                use objc2::{
                    class, msg_send,
                    runtime::{AnyObject, Bool},
                };
                let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
                let event: *mut AnyObject = msg_send![app, currentEvent];
                if !event.is_null() {
                    let _: () = msg_send![window, setMovable: Bool::YES];
                    let _: () = msg_send![window, performWindowDragWithEvent: event];
                    let _: () = msg_send![window, setMovable: Bool::NO];
                }
            }
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = frame;
        ctx.send_viewport_cmd(eframe::egui::ViewportCommand::StartDrag);
    }
}

#[cfg(target_os = "macos")]
fn with_window(
    handle: &impl raw_window_handle::HasWindowHandle,
    action: impl FnOnce(*mut objc2::runtime::AnyObject),
) {
    let Ok(handle) = handle.window_handle() else {
        // Headless UI tests have no native window.
        return;
    };
    let raw_window_handle::RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    // SAFETY: the borrowed raw handle keeps eframe's NSView alive throughout
    // this call. Its window is not retained or used after the borrow ends.
    unsafe {
        let view = handle.ns_view.as_ptr().cast::<objc2::runtime::AnyObject>();
        let window: *mut objc2::runtime::AnyObject = objc2::msg_send![view, window];
        if !window.is_null() {
            action(window);
        }
    }
}
