//! Fit the initial decorated Windows window inside its monitor's work area.
//! Native coordinates already include the window's DPI and title-bar borders.

/// Rectangles use physical screen coordinates: left, top, right, bottom.
/// Keep an already fitting window unchanged; otherwise shrink and move only
/// as far as needed. Work areas can have negative origins or taskbars on any edge.
fn fit_outer_rect(window: [i32; 4], work: [i32; 4]) -> Option<[i32; 4]> {
    let [left, top, right, bottom] = window;
    let [work_left, work_top, work_right, work_bottom] = work;
    if right <= left || bottom <= top || work_right <= work_left || work_bottom <= work_top {
        return None;
    }
    let width = (right - left).min(work_right - work_left);
    let height = (bottom - top).min(work_bottom - work_top);
    let left = left.clamp(work_left, work_right - width);
    let top = top.clamp(work_top, work_bottom - height);
    Some([left, top, left + width, top + height])
}

#[cfg(target_os = "windows")]
pub(super) fn configure(handle: &impl raw_window_handle::HasWindowHandle) {
    use windows_sys::Win32::{
        Foundation::RECT,
        Graphics::Gdi::{
            GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
        },
        UI::WindowsAndMessaging::{
            GetWindowRect, IsIconic, IsZoomed, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos,
        },
    };
    let Ok(handle) = handle.window_handle() else {
        // Headless layout tests have no native window.
        return;
    };
    let raw_window_handle::RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = handle.hwnd.get() as windows_sys::Win32::Foundation::HWND;
    // SAFETY: eframe owns this live HWND on the UI thread. It is still hidden
    // during creation; the handle is neither retained nor used after this call.
    // Both rectangles are queried after winit establishes DPI awareness, so no
    // logical/physical conversion or guessed taskbar/title-bar height is needed.
    unsafe {
        if IsZoomed(hwnd) != 0 || IsIconic(hwnd) != 0 {
            return;
        }
        let mut window: RECT = std::mem::zeroed();
        let mut monitor: MONITORINFO = std::mem::zeroed();
        monitor.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetWindowRect(hwnd, &mut window) == 0
            || GetMonitorInfoW(
                MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
                &mut monitor,
            ) == 0
        {
            return;
        }
        let original = [window.left, window.top, window.right, window.bottom];
        let work = monitor.rcWork;
        if let Some([left, top, right, bottom]) =
            fit_outer_rect(original, [work.left, work.top, work.right, work.bottom])
            && [left, top, right, bottom] != original
        {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                left,
                top,
                right - left,
                bottom - top,
                SWP_NOACTIVATE | SWP_NOZORDER,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fit_outer_rect;

    #[test]
    fn initial_window_fits_taskbar_work_area_at_each_display_scale() {
        // 1280 x 800 logical desktop, a 48-point taskbar, and native borders.
        for scale in [1, 2, 3] {
            let work = [0, 0, 1280 * scale, 752 * scale];
            let window = [80 * scale, 60 * scale, 1296 * scale, 919 * scale];
            let fitted = fit_outer_rect(window, work).unwrap();
            assert_eq!(fitted, [64 * scale, 0, 1280 * scale, 752 * scale]);
        }
        // The reported Windows desktop uses 150% display scaling.
        assert_eq!(
            fit_outer_rect([120, 90, 1944, 1379], [0, 0, 1920, 1128]),
            Some([96, 0, 1920, 1128])
        );
    }

    #[test]
    fn window_fitting_respects_monitor_origins_and_taskbar_edges() {
        for work in [
            [-1920, 0, 0, 1040],
            [1920, 48, 3840, 1080],
            [64, -1080, 1920, 0],
            [0, 0, 1856, 1080],
            [0, 0, 640, 400],
        ] {
            let fitted = fit_outer_rect([100, 100, 2100, 1300], work).unwrap();
            assert!(fitted[0] >= work[0] && fitted[1] >= work[1]);
            assert!(fitted[2] <= work[2] && fitted[3] <= work[3]);
            assert_eq!(fit_outer_rect(fitted, work), Some(fitted));
        }
    }

    #[test]
    fn fitting_keeps_valid_geometry_and_ignores_invalid_rectangles() {
        let window = [100, 80, 1316, 939];
        assert_eq!(fit_outer_rect(window, [0, 0, 1920, 1040]), Some(window));
        assert_eq!(fit_outer_rect(window, [0, 0, 0, 1040]), None);
        assert_eq!(fit_outer_rect([0, 0, 100, 0], [0, 0, 1920, 1040]), None);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn native_startup_window_fits_the_actual_monitor_work_area() {
        use raw_window_handle::{RawWindowHandle, Win32WindowHandle, WindowHandle};
        use windows_sys::Win32::{
            Foundation::{HWND, POINT, RECT},
            Graphics::Gdi::{
                GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
            },
            UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, GetWindowRect, IsWindowVisible, WS_OVERLAPPEDWINDOW,
            },
        };
        struct HiddenWindow(HWND);
        impl Drop for HiddenWindow {
            fn drop(&mut self) {
                // SAFETY: the test owns this window and destroys it on its UI thread.
                unsafe { DestroyWindow(self.0) };
            }
        }
        // SAFETY: STATIC is a predefined system class. This test creates its own
        // hidden window; no existing application or desktop window is modified.
        unsafe {
            let mut monitor: MONITORINFO = std::mem::zeroed();
            monitor.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            assert_ne!(
                GetMonitorInfoW(
                    MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTONEAREST),
                    &mut monitor,
                ),
                0
            );
            let work = monitor.rcWork;
            let window = HiddenWindow(CreateWindowExW(
                0,
                windows_sys::core::w!("STATIC"),
                windows_sys::core::w!("MyPDF startup sizing test"),
                WS_OVERLAPPEDWINDOW,
                work.left,
                work.top,
                work.right - work.left + 300,
                work.bottom - work.top + 300,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            ));
            assert!(!window.0.is_null());
            let handle = WindowHandle::borrow_raw(RawWindowHandle::Win32(Win32WindowHandle::new(
                std::num::NonZeroIsize::new(window.0 as isize).unwrap(),
            )));
            super::configure(&handle);
            let mut actual: RECT = std::mem::zeroed();
            assert_ne!(GetWindowRect(window.0, &mut actual), 0);
            assert!(actual.left >= work.left && actual.top >= work.top);
            assert!(actual.right <= work.right && actual.bottom <= work.bottom);
            assert_eq!(
                IsWindowVisible(window.0),
                0,
                "Fitting must not show the window"
            );
            println!(
                "Native fitted bounds: {:?}; monitor work area: {:?}",
                [actual.left, actual.top, actual.right, actual.bottom],
                [work.left, work.top, work.right, work.bottom]
            );
        }
    }
}
