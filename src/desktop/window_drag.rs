//! The compact tab strip occupies the transparent macOS title bar.
//! AppKit must not turn a document-tab gesture into a native window drag.

/// Shared pointer feedback and ownership for the logo, title and empty chrome.
/// Child controls retain their gestures; entering chrome during another drag
/// cannot turn that gesture into a window move.
pub(super) fn handle(
    response: &eframe::egui::Response,
    eligible: impl Fn(eframe::egui::Pos2) -> bool,
) -> bool {
    use eframe::egui::{CursorIcon, PointerButton, Popup};
    let ctx = &response.ctx;
    if !response.enabled()
        || Popup::is_any_open(ctx)
        || ctx.input(|i| i.viewport().fullscreen.unwrap_or(false))
    {
        return false;
    }
    let pressed_here = ctx
        .input(|i| i.pointer.primary_down() && i.pointer.press_origin().is_some_and(&eligible))
        && (response.is_pointer_button_down_on() || response.dragged_by(PointerButton::Primary));
    if pressed_here {
        ctx.set_cursor_icon(CursorIcon::Grabbing);
    } else if response.hovered() && response.hover_pos().is_some_and(&eligible) {
        ctx.set_cursor_icon(CursorIcon::Grab);
    }
    pressed_here && response.drag_started_by(PointerButton::Primary)
}

/// AppKit consumes the release inside its synchronous drag loop. Rebuild only
/// the pointer state as released, preserving its hover position without a
/// synthetic click; all keyboard and other input state stays untouched.
#[cfg(any(target_os = "macos", test))]
fn finish_native_drag(ctx: &eframe::egui::Context) {
    use eframe::egui::{CursorIcon, Event, InputState, RawInput};
    let options = ctx.options(|o| o.input_options);
    ctx.input_mut(|input| {
        let events = input
            .pointer
            .latest_pos()
            .map(Event::PointerMoved)
            .into_iter()
            .collect();
        input.pointer = InputState::default()
            .begin_pass(
                RawInput {
                    time: Some(input.time),
                    events,
                    ..Default::default()
                },
                false,
                input.pixels_per_point(),
                options,
            )
            .pointer;
    });
    ctx.stop_dragging();
    ctx.set_cursor_icon(CursorIcon::Grab);
    ctx.request_repaint();
}

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
                    // The native loop blocks eframe before it can apply this
                    // frame's cursor output. Set AppKit's cursor immediately.
                    let cursor: *mut AnyObject = msg_send![class!(NSCursor), closedHandCursor];
                    if !cursor.is_null() {
                        let _: () = msg_send![cursor, push];
                    }
                    let _: () = msg_send![window, setMovable: Bool::YES];
                    let _: () = msg_send![window, performWindowDragWithEvent: event];
                    let _: () = msg_send![window, setMovable: Bool::NO];
                    if !cursor.is_null() {
                        let _: () = msg_send![class!(NSCursor), pop];
                    }
                    finish_native_drag(ctx);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::*;

    fn button(pos: egui::Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn toolbar_frame(
        app: &mut Viewer,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, bool) {
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1400., 800.))),
                events,
                ..Default::default()
            },
            |ctx| app.toolbar(ctx),
        );
        (output, app.tab_controls.take_window_drag())
    }

    fn fixture() -> (Viewer, egui::Context, Rect, Rect) {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        for _ in 0..3 {
            toolbar_frame(&mut app, &ctx, vec![]);
        }
        let (output, _) = toolbar_frame(&mut app, &ctx, vec![]);
        let nodes = output.platform_output.accesskit_update.unwrap().nodes;
        let bounds = |name: &str| {
            let rect = nodes
                .iter()
                .find(|(_, n)| {
                    n.label().or_else(|| n.value()) == Some(name) && n.bounds().is_some()
                })
                .unwrap_or_else(|| panic!("missing toolbar label {name}"))
                .1
                .bounds()
                .unwrap();
            Rect::from_min_max(
                pos2(rect.x0 as f32, rect.y0 as f32),
                pos2(rect.x1 as f32, rect.y1 as f32),
            )
        };
        (app, ctx, bounds("MyPDF"), bounds("Home"))
    }

    #[test]
    fn window_drag_logo_title_and_empty_strip_share_cursor_and_drag_threshold() {
        for region in ["logo", "title", "empty strip"] {
            let (mut app, ctx, title, _) = fixture();
            let at = match region {
                "logo" => pos2(title.left() - 15., title.center().y),
                "title" => title.center(),
                _ => pos2(700., title.center().y),
            };
            let (hover, requested) =
                toolbar_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
            assert_eq!(
                hover.platform_output.cursor_icon,
                egui::CursorIcon::Grab,
                "{region} hover"
            );
            assert!(!requested);
            let (press, requested) = toolbar_frame(
                &mut app,
                &ctx,
                vec![button(at, egui::PointerButton::Primary, true)],
            );
            assert_eq!(
                press.platform_output.cursor_icon,
                egui::CursorIcon::Grabbing,
                "{region} held press"
            );
            assert!(!requested, "{region} should wait for movement");
            let (_, small) = toolbar_frame(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(at + vec2(1., 0.))],
            );
            assert!(!small, "{region} tiny motion remains a click");
            let (drag, requested) = toolbar_frame(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(at + vec2(24., 0.))],
            );
            assert!(
                requested,
                "{region} starts moving the window after the same threshold"
            );
            assert_eq!(drag.platform_output.cursor_icon, egui::CursorIcon::Grabbing);
            let (outside, requested) = toolbar_frame(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(pos2(700., 200.))],
            );
            assert!(!requested, "native drag is requested only once");
            assert_eq!(
                outside.platform_output.cursor_icon,
                egui::CursorIcon::Grabbing,
                "owned drag keeps feedback outside the handle"
            );
            let (release, requested) = toolbar_frame(
                &mut app,
                &ctx,
                vec![button(
                    pos2(700., 200.),
                    egui::PointerButton::Primary,
                    false,
                )],
            );
            assert!(!requested);
            assert_ne!(
                release.platform_output.cursor_icon,
                egui::CursorIcon::Grabbing,
                "release clears the held cursor"
            );
            let (hover, _) = toolbar_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
            assert_eq!(hover.platform_output.cursor_icon, egui::CursorIcon::Grab);
        }
    }

    #[test]
    fn window_drag_does_not_capture_buttons_clicks_or_secondary_gestures() {
        let (mut app, ctx, title, home) = fixture();
        let (_, requested) = toolbar_frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(home.center()),
                button(home.center(), egui::PointerButton::Primary, true),
            ],
        );
        assert!(!requested);
        let (_, requested) = toolbar_frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(title.center())],
        );
        assert!(
            !requested,
            "moving from a button onto the title must retain button ownership"
        );
        toolbar_frame(
            &mut app,
            &ctx,
            vec![button(title.center(), egui::PointerButton::Primary, false)],
        );
        for _ in 0..2 {
            let (_, requested) = toolbar_frame(
                &mut app,
                &ctx,
                vec![
                    egui::Event::PointerMoved(title.center()),
                    button(title.center(), egui::PointerButton::Primary, true),
                ],
            );
            assert!(!requested);
            let (_, requested) = toolbar_frame(
                &mut app,
                &ctx,
                vec![button(title.center(), egui::PointerButton::Primary, false)],
            );
            assert!(
                !requested,
                "single/double clicks must not initiate native drag"
            );
        }
        let (_, requested) = toolbar_frame(
            &mut app,
            &ctx,
            vec![button(title.center(), egui::PointerButton::Secondary, true)],
        );
        assert!(!requested);
        let (drag, requested) = toolbar_frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(title.center() + vec2(30., 0.))],
        );
        assert!(!requested);
        assert_ne!(drag.platform_output.cursor_icon, egui::CursorIcon::Grabbing);
    }

    #[test]
    fn window_drag_fullscreen_and_popup_have_no_window_move_affordance() {
        for popup in [false, true] {
            let ctx = egui::Context::default();
            let rect = Rect::from_min_size(pos2(20., 20.), vec2(150., 32.));
            for frame in 0..4 {
                let events = match frame {
                    1 => vec![egui::Event::PointerMoved(rect.center())],
                    2 => vec![button(rect.center(), egui::PointerButton::Primary, true)],
                    3 => vec![egui::Event::PointerMoved(rect.center() + vec2(25., 0.))],
                    _ => vec![],
                };
                let mut input = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(400., 300.))),
                    events,
                    ..Default::default()
                };
                input
                    .viewports
                    .get_mut(&egui::ViewportId::ROOT)
                    .unwrap()
                    .fullscreen = Some(!popup);
                let output = ctx.run(input, |ctx| {
                    if popup {
                        egui::Popup::open_id(ctx, egui::Id::new("window_drag_popup"));
                    }
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let response = ui.interact(
                            rect,
                            egui::Id::new("window_drag_test"),
                            egui::Sense::click_and_drag(),
                        );
                        assert!(!handle(&response, |_| true));
                    });
                });
                assert!(!matches!(
                    output.platform_output.cursor_icon,
                    egui::CursorIcon::Grab | egui::CursorIcon::Grabbing
                ));
            }
        }
    }

    #[test]
    fn window_drag_native_completion_clears_consumed_release_and_keeps_hover() {
        let (mut app, ctx, title, _) = fixture();
        let start = title.center();
        toolbar_frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(start),
                button(start, egui::PointerButton::Primary, true),
            ],
        );
        let point = start + vec2(8., 0.);
        let (_, requested) = toolbar_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(point)]);
        assert!(requested && ctx.input(|i| i.pointer.primary_down()));
        // The OS consumes mouse-up, so no PointerButton(false) reaches egui.
        finish_native_drag(&ctx);
        assert!(!ctx.input(|i| i.pointer.any_down() || i.pointer.any_click()));
        assert_eq!(ctx.pointer_hover_pos(), Some(point));
        assert!(ctx.dragged_id().is_none());
        let (next, requested) = toolbar_frame(&mut app, &ctx, vec![]);
        assert!(!requested);
        assert_eq!(next.platform_output.cursor_icon, egui::CursorIcon::Grab);
        let (press, requested) = toolbar_frame(
            &mut app,
            &ctx,
            vec![button(point, egui::PointerButton::Primary, true)],
        );
        assert!(!requested);
        assert_eq!(
            press.platform_output.cursor_icon,
            egui::CursorIcon::Grabbing
        );
        let (_, requested) = toolbar_frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(point + vec2(25., 0.))],
        );
        assert!(requested, "the next deliberate window drag starts normally");
    }
}
