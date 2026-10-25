use crate::desktop::{Viewer, editing};
use eframe::{App, egui};
use egui::{Id, Modifiers, Pos2, Rect, pos2, vec2};
use mypdf_milestone0::annotations::{Annotation, Kind, Style};
use std::path::PathBuf;

fn frame(app: &mut Viewer, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
    ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 820.))),
            max_texture_side: Some(8192),
            events,
            ..Default::default()
        },
        |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
    )
}

fn fixture(mode: editing::Mode, inspector: bool) -> (Viewer, egui::Context) {
    let ctx = egui::Context::default();
    let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
    let source = mypdf_milestone0::convert::blank(595., 842.).unwrap();
    let source = mypdf_milestone0::content::add_text(
        &source,
        1,
        "Original PDF text stays in its mode",
        [40., 70., 450., 40.],
        14.,
        [0., 0., 0.],
    )
    .unwrap();
    app.open_memory(PathBuf::from("Inline annotation.pdf"), source, &ctx);
    app.editor.mode = mode;
    app.inspector = inspector;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.tabs[0].session.is_none() {
        frame(&mut app, &ctx, vec![]);
        assert!(std::time::Instant::now() < deadline, "{:?}", app.status);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    settle(&mut app, &ctx);
    (app, ctx)
}

fn settle(app: &mut Viewer, ctx: &egui::Context) {
    for _ in 0..3 {
        frame(app, ctx, vec![]);
    }
}

fn key(key: egui::Key, modifiers: Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn pointer(app: &mut Viewer, ctx: &egui::Context, at: Pos2, pressed: Option<bool>) {
    let mut events = vec![egui::Event::PointerMoved(at)];
    if let Some(pressed) = pressed {
        events.push(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    frame(app, ctx, events);
}

fn click(app: &mut Viewer, ctx: &egui::Context, at: Pos2) {
    pointer(app, ctx, at, None);
    pointer(app, ctx, at, Some(true));
    pointer(app, ctx, at, Some(false));
}

fn page(app: &Viewer, ctx: &egui::Context) -> Rect {
    ctx.read_response(Id::new((app.tabs[0].id, 0_usize, "page_actions")))
        .expect("Annotate/Draw must retain their page interaction")
        .rect
}

fn page_point(app: &Viewer, ctx: &egui::Context, x: f32, y: f32) -> Pos2 {
    let page = page(app, ctx);
    page.min + vec2(x, y) * (page.width() / app.tabs[0].pages[0].width)
}

fn annotations(app: &Viewer) -> &[Annotation] {
    &app.tabs[0].session.as_ref().unwrap().annotations
}

fn text(app: &Viewer) -> &str {
    assert_eq!(annotations(app).len(), 1);
    match &annotations(app)[0].kind {
        Kind::Text { text } => text,
        other => panic!("Expected an annotation text box, got {other:?}"),
    }
}

fn inline_id(app: &Viewer) -> Id {
    Id::new((
        app.tabs[0].id,
        &annotations(app)[0].id,
        "inline_annotation_text",
    ))
}

#[test]
fn active_text_keeps_document_layout_and_typing_when_zoomed_out() {
    for density in [1., 2.] {
        let (mut app, ctx) = fixture(editing::Mode::Annotate, false);
        ctx.set_pixels_per_point(density);
        let a = Annotation::new(
            0,
            [40., 130., 340., 230.],
            Kind::Text {
                text: "Review this document carefully before signing the approval.".into(),
            },
            Style {
                font_size: 14.,
                ..Default::default()
            },
        );
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![a.clone()])
            .unwrap();
        app.tabs[0].selected_annotation = Some(a.id.clone());
        super::start(&mut app.editor, app.tabs[0].id, &a, false);
        settle(&mut app, &ctx);
        app.inspector = false;
        let mut expected: Option<Vec<(String, egui::Vec2)>> = None;
        for scale in [1., 0.5, 0.25, 0.15] {
            app.tabs[0].zoom = crate::desktop::Zoom::Scale(scale);
            settle(&mut app, &ctx);
            let output = frame(&mut app, &ctx, vec![]);
            assert!(ctx.memory(|m| m.has_focus(inline_id(&app))));
            let shape = output
                .shapes
                .iter()
                .find_map(|p| match &p.shape {
                    egui::Shape::Text(t)
                        if t.galley.job.text.starts_with("Review this")
                            && page(&app, &ctx).contains(t.pos) =>
                    {
                        Some(t)
                    }
                    _ => None,
                })
                .expect("The on-page text editor must remain painted");
            let rows: Vec<_> = shape
                .galley
                .rows
                .iter()
                .map(|r| {
                    (
                        r.glyphs.iter().map(|g| g.chr).collect::<String>(),
                        r.size / scale,
                    )
                })
                .collect();
            if let Some(expected) = &expected {
                assert_eq!(rows.len(), expected.len());
                for ((text, size), (e_text, e_size)) in rows.iter().zip(expected) {
                    assert_eq!(text, e_text);
                    assert!(
                        (*size - *e_size).length() < 0.001,
                        "density={density}, scale={scale}: {size:?} != {e_size:?}"
                    );
                }
            } else {
                expected = Some(rows);
            }
        }
        frame(&mut app, &ctx, vec![egui::Event::Text(" İmza".into())]);
        assert!(text(&app).ends_with(" İmza"));
        assert!(app.editor.mode == editing::Mode::Annotate);
        assert!(!app.inspector);
    }
}

#[test]
fn reduced_inline_editor_does_not_enlarge_the_page_scroll_extent() {
    let (mut app, ctx) = fixture(editing::Mode::Annotate, false);
    let a = Annotation::new(
        0,
        [390., 600., 570., 780.],
        Kind::Text {
            text: "Bottom of page".into(),
        },
        Style::default(),
    );
    app.tabs[0]
        .session
        .as_mut()
        .unwrap()
        .commit(vec![a.clone()])
        .unwrap();
    app.tabs[0].selected_annotation = Some(a.id.clone());
    super::start(&mut app.editor, app.tabs[0].id, &a, false);
    for scale in [0.15, 0.5, 1., 2.] {
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let (page_rect, _) =
                    ui.allocate_exact_size(vec2(595., 842.) * scale, egui::Sense::hover());
                let before = ui.min_rect();
                super::overlay(
                    &mut app.editor,
                    &mut app.tabs[0],
                    ui,
                    0,
                    page_rect,
                    scale,
                    &mut app.status,
                );
                assert_eq!(
                    ui.min_rect(),
                    before,
                    "The text overlay must not allocate document-space coordinates in the parent"
                );
            });
        });
    }
}

#[test]
fn click_placed_annotation_text_types_immediately_with_or_without_inspector() {
    for mode in [editing::Mode::Annotate, editing::Mode::Draw] {
        for inspector in [false, true] {
            let (mut app, ctx) = fixture(mode, inspector);
            let source = app.tabs[0].session.as_ref().unwrap().source.clone();
            app.editor.select_tool(editing::Tool::Text);
            let at = page_point(&app, &ctx, 90., 160.);
            click(&mut app, &ctx, at);
            assert_eq!(text(&app), "Your text");
            assert!(ctx.memory(|m| m.has_focus(inline_id(&app))));

            frame(
                &mut app,
                &ctx,
                vec![egui::Event::Text("İstanbul — review".into())],
            );
            assert_eq!(
                text(&app),
                "İstanbul — review",
                "The placeholder is replaced"
            );
            frame(&mut app, &ctx, vec![egui::Event::Text("!".into())]);
            assert_eq!(text(&app), "İstanbul — review!");

            let outside = page_point(&app, &ctx, 430., 400.);
            click(&mut app, &ctx, outside);
            settle(&mut app, &ctx);
            assert!(app.editor.annotation_text.is_none());
            assert!(app.editor.mode == mode);
            assert_eq!(&*app.tabs[0].session.as_ref().unwrap().source, &*source);
            assert!(app.content.pending_text().is_none());
            assert!(!app.content.image_mode);

            let expected = annotations(&app).to_vec();
            app.editor_undo(false);
            assert_ne!(annotations(&app), expected.as_slice());
            app.editor_undo(true);
            assert_eq!(annotations(&app), expected.as_slice());
        }
    }
}

#[test]
fn drag_placed_annotation_text_accepts_typing_and_existing_box_reopens_in_place() {
    for mode in [editing::Mode::Annotate, editing::Mode::Draw] {
        let (mut app, ctx) = fixture(mode, false);
        app.editor.select_tool(editing::Tool::Text);
        let start = page_point(&app, &ctx, 90., 160.);
        let end = page_point(&app, &ctx, 360., 250.);
        pointer(&mut app, &ctx, start, None);
        pointer(&mut app, &ctx, start, Some(true));
        pointer(&mut app, &ctx, end, None);
        pointer(&mut app, &ctx, end, Some(false));
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Text("First revision".into())],
        );
        assert_eq!(text(&app), "First revision");
        assert!(app.editor.mode == mode);
        let outside = page_point(&app, &ctx, 430., 400.);
        click(&mut app, &ctx, outside);
        settle(&mut app, &ctx);

        let original = annotations(&app).to_vec();
        let r = original[0].rect;
        let inside = page_point(&app, &ctx, r[0] + 15., r[1] + 12.);
        click(&mut app, &ctx, inside);
        settle(&mut app, &ctx);
        assert!(ctx.memory(|m| m.has_focus(inline_id(&app))));
        frame(
            &mut app,
            &ctx,
            vec![
                key(egui::Key::A, Modifiers::COMMAND),
                egui::Event::Text("Second revision".into()),
            ],
        );
        assert_eq!(text(&app), "Second revision");
        click(&mut app, &ctx, outside);
        settle(&mut app, &ctx);
        assert!(app.editor.mode == mode);
        let edited = annotations(&app).to_vec();
        app.editor_undo(false);
        assert_eq!(annotations(&app), original.as_slice());
        app.editor_undo(true);
        assert_eq!(annotations(&app), edited.as_slice());
    }
}

#[test]
fn inline_annotation_and_properties_share_text_without_enabling_pdf_content_editing() {
    let (mut app, ctx) = fixture(editing::Mode::Annotate, true);
    app.editor.select_tool(editing::Tool::Text);
    let at = page_point(&app, &ctx, 90., 160.);
    click(&mut app, &ctx, at);
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::Text("On the page".into())],
    );
    settle(&mut app, &ctx);
    let id = annotations(&app)[0].id.clone();
    let property_id = Id::new((app.tabs[0].id, &id, "annotation_text"));
    let property = ctx
        .read_response(property_id)
        .expect("Selected text retains its properties editor")
        .rect;
    click(&mut app, &ctx, property.center());
    frame(
        &mut app,
        &ctx,
        vec![
            key(egui::Key::A, Modifiers::COMMAND),
            egui::Event::Text("From properties".into()),
        ],
    );
    assert_eq!(text(&app), "From properties");
    settle(&mut app, &ctx);

    let r = annotations(&app)[0].rect;
    let inside = page_point(&app, &ctx, r[0] + 15., r[1] + 12.);
    click(&mut app, &ctx, inside);
    frame(
        &mut app,
        &ctx,
        vec![
            key(egui::Key::A, Modifiers::COMMAND),
            egui::Event::Text("Back on the page".into()),
        ],
    );
    assert_eq!(text(&app), "Back on the page");
    assert_eq!(
        app.editor.draft.as_ref().unwrap().kind.text(),
        Some("Back on the page")
    );

    let source = app.tabs[0].session.as_ref().unwrap().source.clone();
    let underlying_text = page_point(&app, &ctx, 100., 80.);
    click(&mut app, &ctx, underlying_text);
    settle(&mut app, &ctx);
    assert!(app.editor.mode == editing::Mode::Annotate);
    assert!(app.content.pending_text().is_none());
    assert!(!app.content.image_mode);
    assert_eq!(&*app.tabs[0].session.as_ref().unwrap().source, &*source);
    assert_eq!(text(&app), "Back on the page");
}

#[test]
fn compact_fill_presets_enable_background_and_survive_inline_text_pdf_roundtrip() {
    let (mut app, ctx) = fixture(editing::Mode::Annotate, true);
    ctx.enable_accesskit();
    let annotation = Annotation::new(
        0,
        [90., 160., 360., 230.],
        Kind::Text {
            text: "Before preset".into(),
        },
        Style {
            fill: None,
            opacity: 0.4,
            ..Default::default()
        },
    );
    app.tabs[0]
        .session
        .as_mut()
        .unwrap()
        .commit(vec![annotation])
        .unwrap();
    ctx.data_mut(|data| {
        data.insert_persisted(
            Id::new("annotation_inspector"),
            egui::containers::panel::PanelState {
                rect: Rect::from_min_size(pos2(970., 0.), vec2(230., 820.)),
            },
        )
    });
    settle(&mut app, &ctx);
    let at = page_point(&app, &ctx, 105., 172.);
    click(&mut app, &ctx, at);
    settle(&mut app, &ctx);
    assert_eq!(annotations(&app)[0].style.fill, None);
    let output = frame(&mut app, &ctx, vec![]);
    let nodes = output.platform_output.accesskit_update.unwrap().nodes;
    let controls: Vec<_> = [
        "White fill",
        "Light yellow fill",
        "Light blue fill",
        "Mint fill",
        "Custom fill color",
    ]
    .into_iter()
    .map(|label| {
        let bounds = nodes
            .iter()
            .find_map(|(_, node)| {
                (node.label() == Some(label))
                    .then(|| node.bounds())
                    .flatten()
            })
            .unwrap_or_else(|| panic!("Missing compact fill control: {label}"));
        (
            label,
            Rect::from_min_max(
                pos2(bounds.x0 as f32, bounds.y0 as f32),
                pos2(bounds.x1 as f32, bounds.y1 as f32),
            ),
        )
    })
    .collect();
    let panel = egui::containers::panel::PanelState::load(&ctx, Id::new("annotation_inspector"))
        .unwrap()
        .rect;
    assert!(
        (panel.width() - 230.).abs() < 0.5,
        "Fill must not widen the panel"
    );
    let (viewport, _) = ctx
        .data(|data| {
            data.get_temp::<(Rect, egui::Vec2)>(Id::new((
                "property_scroll_viewport",
                "annotation_properties",
            )))
        })
        .unwrap();
    let row = controls[0].1;
    for (label, rect) in &controls {
        assert!(
            viewport.contains_rect(*rect),
            "{label} overlaps the scroll gutter"
        );
        assert!(
            (rect.center().y - row.center().y).abs() < 0.5,
            "Fill colors share one row"
        );
        assert!(
            rect.width() <= 26. && rect.height() <= 28.,
            "{label} stays compact"
        );
    }
    let yellow = controls
        .iter()
        .find(|(label, _)| *label == "Light yellow fill")
        .unwrap()
        .1;
    click(&mut app, &ctx, yellow.center());
    assert_eq!(annotations(&app)[0].style.fill, Some([1., 0.96, 0.75]));
    assert!((annotations(&app)[0].style.opacity - 0.4).abs() < 0.001);

    let at = page_point(&app, &ctx, 105., 172.);
    click(&mut app, &ctx, at);
    frame(
        &mut app,
        &ctx,
        vec![
            key(egui::Key::A, Modifiers::COMMAND),
            egui::Event::Text("Edited with background".into()),
        ],
    );
    assert_eq!(text(&app), "Edited with background");
    let session = app.tabs[0].session.as_ref().unwrap();
    let bytes = session.snapshot_bytes().unwrap();
    let loaded = mypdf_milestone0::annotations::load(&lopdf::Document::load_mem(&bytes).unwrap());
    assert_eq!(loaded, session.annotations);
    assert_eq!(loaded[0].kind.text(), Some("Edited with background"));
    assert_eq!(loaded[0].style.fill, Some([1., 0.96, 0.75]));
    assert!((loaded[0].style.opacity - 0.4).abs() < 0.001);
}

#[test]
fn leaving_inline_annotation_for_library_or_read_mode_keeps_text_visible() {
    let (mut app, ctx) = fixture(editing::Mode::Annotate, false);
    app.editor.select_tool(editing::Tool::Text);
    let at = page_point(&app, &ctx, 90., 160.);
    click(&mut app, &ctx, at);
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::Text("Visible annotation status".into())],
    );
    let expected = annotations(&app).to_vec();
    assert!(app.editor.annotation_text.is_some());
    let visible = |output: &egui::FullOutput| {
        output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text)
                if text.galley.job.text == "Visible annotation status")
        })
    };

    app.editor.library_open = true;
    let output = frame(&mut app, &ctx, vec![]);
    assert!(app.editor.annotation_text.is_none());
    assert!(
        visible(&output),
        "Opening the library must not hide the annotation"
    );
    assert_eq!(annotations(&app), expected.as_slice());
    app.editor.library_open = false;
    settle(&mut app, &ctx);
    assert!(
        app.editor.annotation_text.is_none(),
        "Closing the library must not revive a stale editor"
    );

    let inside = page_point(&app, &ctx, 105., 172.);
    click(&mut app, &ctx, inside);
    assert!(app.editor.annotation_text.is_some());
    app.editor.mode = editing::Mode::Read;
    let output = frame(&mut app, &ctx, vec![]);
    assert!(app.editor.annotation_text.is_none());
    assert!(
        visible(&output),
        "Read mode must paint the saved annotation"
    );
    app.editor.mode = editing::Mode::Annotate;
    let output = frame(&mut app, &ctx, vec![]);
    assert!(
        app.editor.annotation_text.is_none(),
        "Returning to Annotate must not revive the old caret"
    );
    assert!(visible(&output));
    assert_eq!(annotations(&app), expected.as_slice());
}

fn shared_annotation_paint(
    app: &mut Viewer,
    ctx: &egui::Context,
    annotation: &Annotation,
) -> egui::FullOutput {
    ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 820.))),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                editing::paint_annotation(
                    &mut app.editor,
                    ctx,
                    ui.painter(),
                    Rect::from_min_size(pos2(30., 30.), vec2(595., 842.)),
                    1.,
                    annotation,
                );
            });
        },
    )
}

#[test]
fn shared_annotation_painter_keeps_active_inline_text_in_previews() {
    let (mut app, ctx) = fixture(editing::Mode::Annotate, false);
    app.editor.select_tool(editing::Tool::Text);
    let at = page_point(&app, &ctx, 90., 160.);
    click(&mut app, &ctx, at);
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::Text("Active text in previews".into())],
    );
    assert!(app.editor.annotation_text.is_some());
    let annotation = annotations(&app)[0].clone();
    let output = shared_annotation_paint(&mut app, &ctx, &annotation);
    assert!(
        output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text)
                if text.galley.job.text == "Active text in previews")
        }),
        "Shared previews must paint the annotation even while the document has an inline editor"
    );
    assert_eq!(annotations(&app), &[annotation]);
}

#[test]
fn transformed_annotation_text_keeps_static_appearance_and_properties_editor() {
    for mode in [editing::Mode::Annotate, editing::Mode::Draw] {
        for (rotation, flip_x, flip_y) in
            [(90., false, false), (0., true, false), (0., false, true)]
        {
            let (mut app, ctx) = fixture(mode, true);
            let mut annotation = Annotation::new(
                0,
                [90., 160., 360., 230.],
                Kind::Text {
                    text: "Transformed text stays visible".into(),
                },
                Style::default(),
            );
            annotation.rotation = rotation;
            annotation.flip_x = flip_x;
            annotation.flip_y = flip_y;
            app.tabs[0]
                .session
                .as_mut()
                .unwrap()
                .commit(vec![annotation.clone()])
                .unwrap();
            settle(&mut app, &ctx);
            let at = page_point(&app, &ctx, 225., 195.);
            click(&mut app, &ctx, at);
            settle(&mut app, &ctx);
            assert!(app.editor.mode == mode);
            assert!(
                app.editor.annotation_text.is_none(),
                "Unsupported transformed text must retain its static appearance"
            );
            assert_eq!(app.editor.draft.as_ref(), Some(&annotation));
            assert_eq!(annotations(&app), &[annotation.clone()]);

            let output = shared_annotation_paint(&mut app, &ctx, &annotation);
            if flip_x || flip_y {
                assert!(
                    output.shapes.iter().any(|shape| {
                        matches!(&shape.shape, egui::Shape::Mesh(mesh)
                            if !mesh.vertices.is_empty() && !mesh.indices.is_empty())
                    }),
                    "Flipped glyphs must still paint through their transformed meshes"
                );
            } else {
                assert!(
                    output.shapes.iter().any(|shape| {
                        matches!(&shape.shape, egui::Shape::Text(text)
                            if text.galley.job.text == "Transformed text stays visible"
                                && (text.angle - rotation.to_radians()).abs() < 0.001)
                    }),
                    "The static painter must retain the text rotation"
                );
            }

            settle(&mut app, &ctx);
            let property_id = Id::new((app.tabs[0].id, &annotation.id, "annotation_text"));
            let property = ctx
                .read_response(property_id)
                .expect("Transformed text retains the properties editor")
                .rect;
            click(&mut app, &ctx, property.center());
            frame(
                &mut app,
                &ctx,
                vec![
                    key(egui::Key::A, Modifiers::COMMAND),
                    egui::Event::Text("Edited through properties".into()),
                ],
            );
            annotation.kind = Kind::Text {
                text: "Edited through properties".into(),
            };
            assert_eq!(annotations(&app), &[annotation]);
            assert!(app.editor.annotation_text.is_none());
            assert!(app.editor.mode == mode);
        }
    }
}
