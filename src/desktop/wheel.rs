//! Windows reports every mouse wheel and touchpad movement in lines. egui
//! scrolls 40 points per line and eases each step in over a few frames: a
//! notch moves less than half as far as in other Windows apps, and touchpad
//! scrolling trails behind the finger. Convert lines to points instead.
use eframe::egui::{self, Event, Modifiers, MouseWheelUnit, vec2};

/// Edge and Chrome scroll 100 points per notch at the default three lines.
const POINTS_PER_LINE: f32 = 100. / 3.;
/// egui applies shorter wheel steps at once and eases longer ones in, so
/// touchpad and high-resolution wheel movement is passed on in such pieces.
const IMMEDIATE_STEP: f32 = 7.5;
/// `SPI_GETWHEELSCROLLLINES` reports this when a notch scrolls a page.
#[cfg(target_os = "windows")]
const WHEEL_PAGESCROLL: u32 = u32::MAX;

/// Lines per vertical notch (`None`: one page) and characters per horizontal notch.
#[derive(Clone, Copy, Debug)]
struct Settings {
    lines: Option<f32>,
    chars: f32,
}

fn adapt(events: &mut Vec<Event>, settings: Settings, zoom: Modifiers) {
    let mut adapted = Vec::with_capacity(events.len());
    for event in events.drain(..) {
        let Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta,
            modifiers,
        } = event
        else {
            adapted.push(event);
            continue;
        };
        // Ctrl + wheel and touchpad pinches keep egui's zoom speed.
        if modifiers.matches_any(zoom) {
            adapted.push(event);
            continue;
        }
        let Some(lines) = settings.lines else {
            adapted.push(Event::MouseWheel {
                unit: MouseWheelUnit::Page,
                delta,
                modifiers,
            });
            continue;
        };
        let points = vec2(delta.x * settings.chars, delta.y * lines) * POINTS_PER_LINE;
        // Whole notches keep egui's short easing; fractional movement follows
        // the finger. |x| + |y| bounds a piece even when Shift adds x to y.
        let notches = delta.x.fract() == 0. && delta.y.fract() == 0.;
        let pieces = if notches {
            1.
        } else {
            ((points.x.abs() + points.y.abs()) / IMMEDIATE_STEP)
                .ceil()
                .max(1.)
        };
        adapted.extend((0..pieces as usize).map(|_| Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: points / pieces,
            modifiers,
        }));
    }
    *events = adapted;
}

#[cfg(target_os = "windows")]
pub(super) fn adapt_input(ctx: &egui::Context, raw_input: &mut egui::RawInput) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETWHEELSCROLLCHARS, SPI_GETWHEELSCROLLLINES, SystemParametersInfoW,
    };
    if !raw_input.events.iter().any(|e| {
        matches!(
            e,
            Event::MouseWheel {
                unit: MouseWheelUnit::Line,
                ..
            }
        )
    }) {
        return;
    }
    // Read on each scroll so a changed Mouse setting applies at once; the
    // Windows default of three remains if a query fails.
    let read = |action| {
        let mut value = 3u32;
        // SAFETY: both queries write a single UINT through the valid pointer.
        unsafe { SystemParametersInfoW(action, 0, (&raw mut value).cast(), 0) };
        value
    };
    let lines = read(SPI_GETWHEELSCROLLLINES);
    let settings = Settings {
        lines: (lines != WHEEL_PAGESCROLL).then_some(lines as f32),
        chars: read(SPI_GETWHEELSCROLLCHARS) as f32,
    };
    adapt(
        &mut raw_input.events,
        settings,
        ctx.options(|o| o.input_options.zoom_modifier),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT: Settings = Settings {
        lines: Some(3.),
        chars: 3.,
    };

    fn wheel(delta: egui::Vec2, modifiers: Modifiers) -> Event {
        Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta,
            modifiers,
        }
    }

    /// The scroll distance a ScrollArea receives in the frame of the input.
    fn scrolled(events: Vec<Event>) -> egui::Vec2 {
        let ctx = egui::Context::default();
        let mut scrolled = egui::Vec2::ZERO;
        let _ = ctx.run(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ctx| {
                scrolled = ctx.input(|i| i.smooth_scroll_delta);
            },
        );
        scrolled
    }

    #[test]
    fn wheel_notch_scrolls_as_far_as_other_windows_apps_with_egui_easing() {
        let mut events = vec![wheel(vec2(0., -1.), Modifiers::NONE)];
        adapt(&mut events, DEFAULT, Modifiers::COMMAND);
        assert!(matches!(
            events[..],
            [Event::MouseWheel { unit: MouseWheelUnit::Point, delta, .. }] if delta == vec2(0., -100.)
        ));
        let first = scrolled(events);
        assert!(
            first.y < -IMMEDIATE_STEP && first.y > -100.,
            "eased in: {first:?}"
        );
        // The Windows Mouse setting for lines per notch is respected.
        let mut events = vec![
            wheel(vec2(0., 1.), Modifiers::NONE),
            wheel(vec2(1., 0.), Modifiers::NONE),
        ];
        adapt(
            &mut events,
            Settings {
                lines: Some(6.),
                chars: 0.,
            },
            Modifiers::COMMAND,
        );
        assert!(matches!(events[0], Event::MouseWheel { delta, .. } if delta == vec2(0., 200.)));
        assert!(matches!(events[1], Event::MouseWheel { delta, .. } if delta == egui::Vec2::ZERO));
    }

    #[test]
    fn touchpad_movement_reaches_the_document_in_the_same_frame() {
        // A precision touchpad reports small fractions of a notch per event.
        let touchpad = vec![
            wheel(vec2(0., -0.3), Modifiers::NONE),
            wheel(vec2(0.05, -0.2), Modifiers::SHIFT),
        ];
        let unadapted = scrolled(touchpad.clone());
        let mut events = touchpad;
        adapt(&mut events, DEFAULT, Modifiers::COMMAND);
        let applied = scrolled(events);
        // 0.3 of a notch is 30 points down; Shift turns (5, -20) into 15 points left.
        assert!((applied - vec2(-15., -30.)).length() < 0.01, "{applied:?}");
        assert!(
            unadapted.length() < applied.length() / 2.,
            "egui alone lags: {unadapted:?}"
        );
    }

    #[test]
    fn zoom_gestures_and_page_scrolling_keep_egui_units() {
        let zoom = wheel(vec2(0., 0.5), Modifiers::CTRL);
        let mut events = vec![zoom.clone()];
        adapt(&mut events, DEFAULT, Modifiers::CTRL);
        assert_eq!(events, vec![zoom]);
        let mut events = vec![wheel(vec2(0., -1.), Modifiers::NONE)];
        adapt(
            &mut events,
            Settings {
                lines: None,
                chars: 3.,
            },
            Modifiers::CTRL,
        );
        assert!(matches!(
            events[..],
            [Event::MouseWheel { unit: MouseWheelUnit::Page, delta, .. }] if delta == vec2(0., -1.)
        ));
    }
}
