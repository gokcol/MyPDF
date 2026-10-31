//! Narrow windows retain all navigation, tools, and save status in both densities.
use super::*;
use eframe::App;
use std::time::{Duration, Instant};

#[test]
fn comfortable_footer_keeps_tools_visible_and_clickable_in_narrow_windows() {
    struct Restore(theme::Density);
    impl Drop for Restore {
        fn drop(&mut self) {
            theme::set_density(self.0);
        }
    }
    let _restore = Restore(theme::density());
    for (density, width) in [
        (theme::Density::Compact, 720.),
        (theme::Density::Compact, 1200.),
        (theme::Density::Comfortable, 720.),
        (theme::Density::Comfortable, 1200.),
    ] {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        ui::set_density(&ctx, density).unwrap();
        app.new_blank(&ctx);
        app.home.visible = false;
        app.editor.mode = editing::Mode::Read;
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(Instant::now() < deadline, "Blank PDF did not load");
            std::thread::sleep(Duration::from_millis(5));
        }
        let frame = |app: &mut Viewer, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(width, 820.))),
                    events,
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            )
        };
        frame(&mut app, vec![]);
        frame(&mut app, vec![]);
        let output = frame(&mut app, vec![]);
        assert!(
            app.tabs[0].session.as_ref().unwrap().dirty(),
            "New blank PDF is an unsaved document"
        );
        let footer =
            egui::containers::panel::PanelState::load(&ctx, egui::Id::new("document_navigation"))
                .unwrap()
                .rect;
        let nodes = output.platform_output.accesskit_update.unwrap().nodes;
        let bounds = |label: &str| {
            nodes
                .iter()
                .find(|(_, node)| {
                    node.label() == Some(label)
                        && node.role() == egui::accesskit::Role::Button
                        && node.bounds().is_some_and(|r| r.y0 >= footer.top() as f64)
                })
                .unwrap_or_else(|| panic!("Missing footer button {label}"))
                .1
                .bounds()
                .unwrap()
        };
        let labels = [
            "Zoom out",
            "Zoom in",
            "Back to previous view",
            "Forward to next view",
            "Previous page",
            "Next page",
            "Undo",
            "Redo",
            "Document history",
            "Hide editing guides",
        ];
        let mut actions: Vec<_> = labels.iter().map(|label| (*label, bounds(label))).collect();
        let find = nodes
            .iter()
            .find(|(_, node)| {
                node.label()
                    .is_some_and(|label| label.starts_with("Find a tool ("))
                    && node.role() == egui::accesskit::Role::Button
                    && node.bounds().is_some_and(|r| r.y0 >= footer.top() as f64)
            })
            .expect("Find a tool remains present")
            .1
            .bounds()
            .unwrap();
        actions.push(("Find a tool", find));
        let page_number = nodes
            .iter()
            .find(|(_, node)| {
                node.role() == egui::accesskit::Role::TextInput
                    && node.label() == Some("Page number")
            })
            .expect("Page number remains accessible")
            .1
            .bounds()
            .unwrap();
        actions.push(("Page number", page_number));
        for (index, (label, rect)) in actions.iter().enumerate() {
            assert!(
                rect.x0 >= 0. && rect.x1 <= width as f64 && rect.y1 <= 820.,
                "{label} is offscreen at {width}, {density:?}: {rect:?}"
            );
            assert!(
                footer.contains(pos2(rect.x0 as f32, rect.y0 as f32))
                    && footer.contains(pos2(rect.x1 as f32, rect.y1 as f32)),
                "{label} is outside the footer"
            );
            if density == theme::Density::Comfortable {
                assert!(
                    rect.width() >= 44. && rect.height() >= 44.,
                    "{label} lost its touch target"
                );
            }
            for (other_label, other) in &actions[index + 1..] {
                assert!(
                    rect.x1 <= other.x0
                        || other.x1 <= rect.x0
                        || rect.y1 <= other.y0
                        || other.y1 <= rect.y0,
                    "{label} overlaps {other_label}"
                );
            }
        }
        let status = nodes
            .iter()
            .find(|(_, node)| {
                node.role() == egui::accesskit::Role::Label
                    && node.value() == Some("Unsaved changes")
            })
            .expect("The real unsaved document status remains present")
            .1
            .bounds()
            .unwrap();
        assert!(
            status.x1 <= width as f64 && status.y1 <= 820.,
            "Save status is clipped"
        );
        if width == 720. {
            assert!(
                bounds("Document history").y0 >= bounds("Next page").y1,
                "Narrow windows put tools on a second row"
            );
        } else {
            assert!(
                (bounds("Document history").y0 - bounds("Next page").y0).abs() < 0.1,
                "Wide windows retain a single footer row"
            );
        }
        let guides = bounds("Hide editing guides");
        let point = pos2(
            ((guides.x0 + guides.x1) / 2.) as f32,
            ((guides.y0 + guides.y1) / 2.) as f32,
        );
        for pressed in [true, false] {
            frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(
            app.editor.hide_guides,
            "The visible guides control must still activate"
        );
    }
}
