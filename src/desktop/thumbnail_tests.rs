//! Regression coverage for the thumbnail sidebar following document navigation.
use super::*;
use eframe::App;
use lopdf::{Document, Object, Stream, dictionary};
use std::collections::BTreeSet;

const PAGE_COUNT: usize = 20;
const WINDOW_SIZE: egui::Vec2 = vec2(1200., 820.);

fn fixture() -> Vec<u8> {
    let mut pdf = Document::with_version("1.7");
    let pages = pdf.new_object_id();
    let font = pdf.add_object(
        dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" },
    );
    let mut kids = Vec::new();
    for index in 0..PAGE_COUNT {
        let content = pdf.add_object(Stream::new(
            dictionary! {},
            format!(
                "BT /F1 24 Tf 40 740 Td (Thumbnail navigation page {}) Tj ET",
                index + 1
            )
            .into_bytes(),
        ));
        let page = pdf.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
            "Contents" => content,
        });
        kids.push(Object::Reference(page));
    }
    pdf.objects.insert(
        pages,
        dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => PAGE_COUNT as i64 }.into(),
    );
    let catalog = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
    pdf.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).unwrap();
    bytes
}

fn frame(app: &mut Viewer, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
    ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0., 0.), WINDOW_SIZE)),
            max_texture_side: Some(8192),
            events,
            ..Default::default()
        },
        |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
    )
}

fn settled(app: &mut Viewer, ctx: &egui::Context) -> egui::FullOutput {
    for _ in 0..4 {
        frame(app, ctx, vec![]);
    }
    frame(app, ctx, vec![])
}

fn loaded() -> (tempfile::TempDir, Viewer, egui::Context) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Thumbnail navigation.pdf");
    std::fs::write(&path, fixture()).unwrap();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = Viewer::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        vec![path],
    );
    app.home.visible = false;
    app.editor.mode = editing::Mode::Read;
    app.sidebar = true;
    app.inspector = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while app.tabs[0].document_view.is_none() || app.tabs[0].session.is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "document did not load"
        );
        frame(&mut app, &ctx, vec![]);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    settled(&mut app, &ctx);
    assert_eq!(app.tabs[0].pages.len(), PAGE_COUNT);
    (dir, app, ctx)
}

fn press(app: &mut Viewer, ctx: &egui::Context, key: egui::Key) -> egui::FullOutput {
    frame(
        app,
        ctx,
        [true, false]
            .into_iter()
            .map(|pressed| egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            })
            .collect(),
    );
    settled(app, ctx)
}

fn thumbnails(output: &egui::FullOutput) -> Vec<(usize, Rect, bool, bool)> {
    output
        .platform_output
        .accesskit_update
        .as_ref()
        .expect("accessibility tree")
        .nodes
        .iter()
        .filter_map(|(_, node)| {
            let page = node
                .label()?
                .strip_prefix("Go to page ")?
                .parse::<usize>()
                .ok()?
                - 1;
            let bounds = node.bounds()?;
            Some((
                page,
                Rect::from_min_max(
                    pos2(bounds.x0 as f32, bounds.y0 as f32),
                    pos2(bounds.x1 as f32, bounds.y1 as f32),
                ),
                node.toggled() == Some(egui::accesskit::Toggled::True),
                node.aria_current() == Some(egui::accesskit::AriaCurrent::Page),
            ))
        })
        .collect()
}

fn thumbnail_clip(output: &egui::FullOutput, page: usize, row: Rect) -> Option<Rect> {
    // Match the page-number paint to its accessible row. Its clip rectangle is
    // the actual scroll viewport, so a row hidden under the header/footer does
    // not count as visible merely because it lies inside the application window.
    let label = (page + 1).to_string();
    let anchor = pos2(row.center().x, row.bottom() - 11.);
    output.shapes.iter().find_map(|clipped| {
        if let egui::epaint::Shape::Text(text) = &clipped.shape {
            let center = text.pos + text.galley.rect.center().to_vec2();
            if text.galley.text() == label && center.distance(anchor) <= 1. {
                return Some(clipped.clip_rect);
            }
        }
        None
    })
}

fn is_visible(output: &egui::FullOutput, page: usize) -> bool {
    thumbnails(output).iter().any(|(index, row, _, _)| {
        *index == page
            && thumbnail_clip(output, page, *row).is_some_and(|clip| {
                row.top() >= clip.top() - 0.5 && row.bottom() <= clip.bottom() + 0.5
            })
    })
}

fn assert_current_visible(app: &Viewer, output: &egui::FullOutput, expected: usize) {
    assert_eq!(app.tabs[app.active].page, expected);
    assert_eq!(
        thumbnails(output)
            .iter()
            .filter_map(|(page, _, _, current)| current.then_some(*page))
            .collect::<Vec<_>>(),
        vec![expected],
        "exactly the current document page must be announced as current"
    );
    assert!(
        is_visible(output, expected),
        "current page {} must be visible in sidebar: {:?}",
        expected + 1,
        thumbnails(output)
    );
}

#[test]
fn arrow_navigation_moves_the_sidebar_selection_and_reveals_offscreen_thumbnails() {
    let (_dir, mut app, ctx) = loaded();
    let revision = app.tabs[0].session.as_ref().unwrap().revision();
    for expected in 1..=10 {
        let output = press(&mut app, &ctx, egui::Key::ArrowDown);
        assert_current_visible(&app, &output, expected);
        assert_eq!(app.organizer.selected, BTreeSet::from([expected]));
        assert!(
            thumbnails(&output)
                .iter()
                .any(|(page, _, selected, _)| *page == expected && *selected)
        );
    }
    for expected in (0..10).rev() {
        let output = press(&mut app, &ctx, egui::Key::ArrowUp);
        assert_current_visible(&app, &output, expected);
        assert_eq!(app.organizer.selected, BTreeSet::from([expected]));
    }
    assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), revision);
}

#[test]
fn home_and_end_reveal_pages_outside_the_virtualized_thumbnail_range() {
    let (_dir, mut app, ctx) = loaded();
    for (key, expected) in [
        (egui::Key::End, PAGE_COUNT - 1),
        (egui::Key::Home, 0),
        (egui::Key::End, PAGE_COUNT - 1),
    ] {
        let output = press(&mut app, &ctx, key);
        assert_current_visible(&app, &output, expected);
        assert_eq!(app.organizer.selected, BTreeSet::from([expected]));
    }
}

#[test]
fn manually_scrolling_thumbnails_is_retained_until_the_document_page_changes() {
    let (_dir, mut app, ctx) = loaded();
    frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::PointerMoved(pos2(110., 450.)),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0., -1100.),
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    for _ in 0..40 {
        frame(&mut app, &ctx, vec![]);
    }
    let output = frame(&mut app, &ctx, vec![]);
    assert_eq!(
        app.tabs[0].page, 0,
        "sidebar scrolling must not navigate the PDF"
    );
    assert!(
        !is_visible(&output, 0),
        "manual scrolling should move away from page one"
    );
    let before = thumbnails(&output);
    let output = settled(&mut app, &ctx);
    assert_eq!(
        thumbnails(&output),
        before,
        "idle redraws must not pull thumbnails back"
    );
    let output = press(&mut app, &ctx, egui::Key::ArrowDown);
    assert_current_visible(&app, &output, 1);
}

#[test]
fn following_the_current_page_preserves_explicit_thumbnail_operation_selections() {
    let (_dir, mut app, ctx) = loaded();
    app.organizer.selected = BTreeSet::from([0, 2, 4]);
    let output = press(&mut app, &ctx, egui::Key::End);
    assert_current_visible(&app, &output, PAGE_COUNT - 1);
    assert_eq!(app.organizer.selected, BTreeSet::from([0, 2, 4]));
    app.organizer.selected = BTreeSet::from([7]);
    let output = press(&mut app, &ctx, egui::Key::Home);
    assert_current_visible(&app, &output, 0);
    assert_eq!(app.organizer.selected, BTreeSet::from([7]));
}

#[test]
fn reopening_the_sidebar_reveals_navigation_that_happened_while_it_was_hidden() {
    let (_dir, mut app, ctx) = loaded();
    app.sidebar = false;
    press(&mut app, &ctx, egui::Key::End);
    app.sidebar = true;
    let output = settled(&mut app, &ctx);
    assert_current_visible(&app, &output, PAGE_COUNT - 1);
    app.outline = true;
    press(&mut app, &ctx, egui::Key::Home);
    app.outline = false;
    let output = settled(&mut app, &ctx);
    assert_current_visible(&app, &output, 0);
}
