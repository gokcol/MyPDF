//! Manual visual QA of the actual egui layout, without a window server.
//! Run with MYPDF_PREVIEW_DIR and `cargo test --bin mypdf native_visual_preview -- --ignored`.
use super::*;
use eframe::App;

#[derive(Default)]
struct Preview {
    textures: HashMap<egui::TextureId, egui::ColorImage>,
}
impl Preview {
    fn capture(&mut self, app: &mut Viewer, ctx: &egui::Context) -> egui::FullOutput {
        self.capture_with(app, ctx, vec![])
    }
    fn capture_with(
        &mut self,
        app: &mut Viewer,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 820.0))),
                max_texture_side: Some(8192),
                events,
                ..Default::default()
            },
            |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
        );
        for (id, delta) in &output.textures_delta.set {
            let egui::ImageData::Color(image) = &delta.image;
            if let Some([x, y]) = delta.pos {
                let target = self.textures.get_mut(id).unwrap();
                for row in 0..image.size[1] {
                    let offset = (y + row) * target.size[0] + x;
                    target.pixels[offset..offset + image.size[0]].copy_from_slice(
                        &image.pixels[row * image.size[0]..(row + 1) * image.size[0]],
                    );
                }
            } else {
                self.textures.insert(*id, (**image).clone());
            }
        }
        output
    }
    fn save(&self, ctx: &egui::Context, output: egui::FullOutput, path: &std::path::Path) {
        let scale = output.pixels_per_point;
        let (w, h) = ((1200.0 * scale) as u32, (820.0 * scale) as u32);
        let mut canvas = image::RgbaImage::from_pixel(w, h, image::Rgba([232, 237, 245, 255]));
        let cross = |a: egui::Vec2, b: egui::Vec2| a.x * b.y - a.y * b.x;
        for primitive in ctx.tessellate(output.shapes, scale) {
            let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
                continue;
            };
            let Some(texture) = self.textures.get(&mesh.texture_id) else {
                continue;
            };
            for triangle in mesh.indices.chunks_exact(3) {
                let v = triangle.map_vertices(&mesh.vertices);
                let points = v.map(|vertex| vertex.pos.to_vec2() * scale);
                let area = cross(points[1] - points[0], points[2] - points[0]);
                if area.abs() < 0.0001 {
                    continue;
                }
                let minimum = points
                    .iter()
                    .fold(primitive.clip_rect.min.to_vec2() * scale, |a, p| {
                        a.max(p.min(points[0]).min(points[1]).min(points[2]))
                    });
                let maximum = points
                    .iter()
                    .fold(primitive.clip_rect.max.to_vec2() * scale, |a, p| {
                        a.min(p.max(points[0]).max(points[1]).max(points[2]))
                    });
                for y in (minimum.y.floor().max(0.0) as u32)
                    ..(maximum.y.ceil().min(h as f32).max(0.0) as u32)
                {
                    for x in (minimum.x.floor().max(0.0) as u32)
                        ..(maximum.x.ceil().min(w as f32).max(0.0) as u32)
                    {
                        let p = vec2(x as f32 + 0.5, y as f32 + 0.5);
                        let weights = [
                            cross(points[1] - p, points[2] - p) / area,
                            cross(points[2] - p, points[0] - p) / area,
                            cross(points[0] - p, points[1] - p) / area,
                        ];
                        if weights.iter().any(|v| *v < -0.0001) {
                            continue;
                        }
                        let uv = (0..3).fold(egui::Vec2::ZERO, |sum, i| {
                            sum + v[i].uv.to_vec2() * weights[i]
                        });
                        let tx = (uv.x * texture.size[0] as f32)
                            .floor()
                            .clamp(0.0, (texture.size[0] - 1) as f32)
                            as usize;
                        let ty = (uv.y * texture.size[1] as f32)
                            .floor()
                            .clamp(0.0, (texture.size[1] - 1) as f32)
                            as usize;
                        let sampled = texture.pixels[ty * texture.size[0] + tx].to_array();
                        let mut src = [0.0; 4];
                        for c in 0..4 {
                            src[c] = (0..3)
                                .map(|i| v[i].color.to_array()[c] as f32 * weights[i])
                                .sum::<f32>()
                                * sampled[c] as f32
                                / 255.0;
                        }
                        let dst = canvas.get_pixel_mut(x, y);
                        for c in 0..3 {
                            dst[c] = (src[c] + dst[c] as f32 * (1.0 - src[3] / 255.0))
                                .clamp(0.0, 255.0) as u8;
                        }
                    }
                }
            }
        }
        canvas.save(path).unwrap();
    }
}
trait TriangleVertices {
    fn map_vertices(&self, vertices: &[egui::epaint::Vertex]) -> [egui::epaint::Vertex; 3];
}
impl TriangleVertices for [u32] {
    fn map_vertices(&self, vertices: &[egui::epaint::Vertex]) -> [egui::epaint::Vertex; 3] {
        [
            vertices[self[0] as usize],
            vertices[self[1] as usize],
            vertices[self[2] as usize],
        ]
    }
}

#[test]
#[ignore = "Manual visual QA; writes only to the explicitly selected preview directory"]
fn native_visual_preview() {
    let root = PathBuf::from(std::env::var_os("MYPDF_PREVIEW_DIR").expect("Set MYPDF_PREVIEW_DIR"));
    std::fs::create_dir_all(&root).unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let pdf = fixture.path().join("Document.pdf");
    let source =
        mypdf_milestone0::document::fixture(&mypdf_milestone0::document::default_font().unwrap())
            .unwrap();
    let source = if std::env::var("MYPDF_PREVIEW_ONLY").as_deref() == Ok("AddText") {
        mypdf_milestone0::convert::blank(595., 842.).unwrap()
    } else {
        source
    };
    std::fs::write(&pdf, source).unwrap();
    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(2.0);
    let mut app = Viewer::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        vec![pdf.clone()],
    );
    if std::env::var("MYPDF_PREVIEW_THEME").as_deref() == Ok("dark") {
        ctx.set_theme(egui::Theme::Dark);
    }
    app.editor.mode = editing::Mode::Annotate;
    app.tabs[0].zoom = Zoom::Page;
    let mut preview = Preview::default();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !app.tabs[0].index_done || !app.tabs[0].rasters.keys().any(|key| !key.thumbnail) {
        assert!(std::time::Instant::now() < deadline);
        preview.capture(&mut app, &ctx);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    for collection in [
        "Home",
        "HomeEmpty",
        "Viewer",
        "Arrows",
        "Symbols",
        "Stamps",
        "Pages",
        "Crop",
        "Edit",
        "InlineText",
        "BlockText",
        "Images",
        "Convert",
        "Draw",
        "Inspector",
        "AddText",
        "Prompt",
        "Confirmation",
        "Group",
        "Lasso",
        "DigitalIds",
        "MySignatures",
        "MySignaturesEditor",
        "NewDigitalId",
        "SignDocument",
        "QuickPrompt",
        "QuickSign",
        "HeaderFooter",
        "Trusted",
        "Redact",
        "Flatten",
        "FormEmpty",
        "FormDesign",
        "FormOrder",
        "FormLayout",
        "FormDrag",
        "Signatures",
    ] {
        if let Ok(only) = std::env::var("MYPDF_PREVIEW_ONLY")
            && !only.split(',').any(|v| v.eq_ignore_ascii_case(collection))
        {
            continue;
        }
        app.home.visible = matches!(collection, "Home" | "HomeEmpty");
        if collection == "Home" {
            for (i, name) in [
                "Getting started",
                "Project notes",
                "Research paper",
                "Design review",
                "Meeting minutes",
                "Reading list",
            ]
            .iter()
            .enumerate()
            {
                let path = fixture.path().join(format!("{name}.pdf"));
                let bytes = mypdf_milestone0::convert::text_to_pdf(&format!("# {name}\n\nMyPDF workspace\n\nA place for your documents and ideas.\n\n## Notes\n\nOpen a PDF to read, annotate, edit, or sign.\n\nKeep your important files in Favorites."), true).unwrap();
                std::fs::write(&path, bytes).unwrap();
                app.home.remember(&path);
                if i == 0 || i == 2 {
                    app.home.preview_favorite(&path);
                }
            }
        }
        if collection == "HomeEmpty" {
            app.home.preview_clear();
        }
        app.editor.mode = if matches!(collection, "Pages" | "Crop") {
            editing::Mode::Pages
        } else {
            editing::Mode::Annotate
        };
        if matches!(collection, "Edit" | "Images" | "InlineText" | "BlockText") {
            app.editor.mode = editing::Mode::Edit;
            app.content.image_mode = collection == "Images";
        }
        if collection == "Convert" {
            app.editor.mode = editing::Mode::Convert;
        }
        if matches!(collection, "Draw" | "Inspector" | "Prompt") {
            app.editor.mode = editing::Mode::Draw;
        }
        if collection == "Inspector" {
            let a = mypdf_milestone0::annotations::Annotation::new(
                0,
                [250., 340., 430., 480.],
                mypdf_milestone0::annotations::Kind::Ellipse,
                mypdf_milestone0::annotations::Style {
                    fill: Some([0.11, 0.43, 0.35]),
                    opacity: 0.5,
                    ..Default::default()
                },
            );
            app.tabs[0].selected_annotation = Some(a.id.clone());
            app.tabs[0]
                .session
                .as_mut()
                .unwrap()
                .commit(vec![a])
                .unwrap();
        }
        if matches!(collection, "Group" | "Lasso") {
            use mypdf_milestone0::annotations::{Annotation, Kind, Style};
            app.editor.mode = editing::Mode::Draw;
            let style = |rgb: [f32; 3]| Style {
                color: rgb,
                width: 3.,
                ..Default::default()
            };
            let marks = vec![
                Annotation::new(
                    0,
                    [90., 590., 230., 690.],
                    Kind::Rectangle,
                    style([0.16, 0.41, 0.96]),
                ),
                Annotation::new(
                    0,
                    [260., 580., 360., 690.],
                    Kind::Ellipse,
                    style([0.9, 0.29, 0.29]),
                ),
                Annotation::new(
                    0,
                    [380., 600., 520., 680.],
                    Kind::Line {
                        start: [0., 1.],
                        end: [1., 0.],
                        arrow: true,
                    },
                    style([0.95, 0.68, 0.15]),
                ),
            ];
            let ids = marks.iter().map(|a| a.id.clone()).collect::<Vec<_>>();
            app.tabs[0].session.as_mut().unwrap().commit(marks).unwrap();
            if collection == "Group" {
                selection::set(&mut app.tabs[0], &mut app.editor, ids);
            } else {
                selection::set(&mut app.tabs[0], &mut app.editor, vec![]);
                app.editor.select_tool(editing::Tool::Lasso);
            }
        }
        app.digital.open = false;
        if collection == "DigitalIds" {
            app.digital.preview_identities(digital::Panel::Identities);
        }
        if collection == "Trusted" {
            app.digital.preview_trusted();
        }
        if collection == "NewDigitalId" {
            app.digital.preview_new_id();
        }
        if collection.starts_with("MySignatures") {
            use mypdf_milestone0::annotations::{Kind, Style};
            let strokes = vec![
                (0..=60)
                    .map(|i| {
                        let t = i as f32 / 60.;
                        [t, 0.55 - (t * 18.).sin() * 0.3 * (1. - t * 0.5)]
                    })
                    .collect(),
                vec![[0.15, 0.85], [0.95, 0.78]],
            ];
            let library = app.editor.library.as_mut().unwrap();
            let mut ids = vec![];
            for (name, kind, aspect) in [
                (
                    "Check mark",
                    Kind::Symbol(mypdf_milestone0::annotations::Symbol::Check),
                    1.,
                ),
                (
                    "Initials",
                    Kind::Text {
                        text: "J.A.".into(),
                    },
                    2.,
                ),
                ("My signature", Kind::Ink { strokes }, 3.),
            ] {
                let asset = mypdf_milestone0::assets::Asset::new(
                    name,
                    kind,
                    Style {
                        color: [0.08, 0.18, 0.55],
                        width: 2.4,
                        font_size: 24.,
                        ..Default::default()
                    },
                    aspect,
                );
                ids.push(asset.id.clone());
                library.put(asset).unwrap();
            }
            ids.reverse();
            app.digital.preview_signatures(&ids);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            preview.capture(&mut app, &ctx);
            while !app.digital.previews_ready() {
                assert!(std::time::Instant::now() < deadline);
                preview.capture(&mut app, &ctx);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            if collection == "MySignaturesEditor" {
                // Scroll the window down to the editor.
                for _ in 0..40 {
                    preview.capture_with(
                        &mut app,
                        &ctx,
                        vec![
                            egui::Event::PointerMoved(pos2(600., 500.)),
                            egui::Event::MouseWheel {
                                unit: egui::MouseWheelUnit::Point,
                                delta: vec2(0., -40.),
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                    );
                }
            }
        }
        if collection == "SignDocument" {
            app.digital.preview_identities(digital::Panel::Sign);
            app.digital.use_document(app.tabs.first());
        }
        if collection == "QuickPrompt" {
            app.digital.preview_quick_prompt();
        }
        if collection == "QuickSign" {
            app.digital.preview_identities(digital::Panel::Sign);
            app.quick_sign();
        }
        if collection == "Redact" {
            app.start_redaction();
            app.preview_redaction();
        }
        if collection == "Flatten" {
            app.redaction.active = false;
            app.editor.mode = editing::Mode::Convert;
            app.request_flatten();
        }
        if collection == "HeaderFooter" {
            app.editor.mode = editing::Mode::Edit;
            app.preview_header_footer();
        }
        if collection == "FormEmpty" {
            app.redaction.active = false;
            app.editor.mode = editing::Mode::Forms;
            app.inspector = true;
        }
        if collection == "FormDesign" {
            app.preview_form();
        }
        if collection == "FormDrag" {
            app.forms.layout_dialog = None;
            app.editor.mode = editing::Mode::Forms;
        }
        if collection == "FormLayout" {
            app.editor.mode = editing::Mode::Forms;
            app.forms.show_order = false;
            app.forms.selected = None;
            app.forms.layout_dialog = Some(mypdf_milestone0::form_layout::Layout::table());
        }
        if collection == "FormOrder" {
            app.editor.mode = editing::Mode::Forms;
            app.forms.selected = None;
            app.forms.show_order = true;
        }
        // The last scene swaps in a signed copy of the document.
        if collection == "Signatures" {
            // Let go of any drag from an earlier scene without changing anything.
            app.preview_cancel_drag();
            preview.capture_with(
                &mut app,
                &ctx,
                vec![egui::Event::PointerButton {
                    pos: pos2(5., 5.),
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            let signed = fixture.path().join("Signed contract.pdf");
            let bytes = std::fs::read(&pdf).unwrap();
            std::fs::write(
                &signed,
                signatures::signed_fixture(&bytes, "Jane Appleseed"),
            )
            .unwrap();
            app.digital.open = false;
            app.close(0);
            app.open(signed, &ctx);
            app.editor.mode = editing::Mode::Read;
            app.tabs[0].zoom = Zoom::Page;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while app.tabs[0].signatures.views.is_none()
                || !app.tabs[0].rasters.keys().any(|key| !key.thumbnail)
            {
                assert!(std::time::Instant::now() < deadline);
                preview.capture(&mut app, &ctx);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            app.tabs[0].signatures.panel = true;
            app.tabs[0].signatures.selected = Some("Approval".into());
        }
        if collection == "AddText" {
            app.editor.mode = editing::Mode::Edit;
            app.content.text_preview(app.tabs[0].id);
        }
        if collection == "Prompt" {
            app.editor.create_stamp_prompt();
        }
        if collection == "Confirmation" {
            app.editor.svg_preview_close();
            app.tabs[0].session.as_mut().unwrap().mark_unsaved();
            app.close_intent = Some(Some(app.tabs[0].id));
        }
        app.organizer.crop = collection == "Crop";
        app.editor.library_open = matches!(collection, "Arrows" | "Symbols" | "Stamps");
        app.editor.library_tab = collection.into();
        for _ in 0..16 {
            preview.capture(&mut app, &ctx);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !app.tabs[0].pending.is_empty()
            || app.content.loading
            || (app.home.visible && app.home.preview_pending())
        {
            assert!(std::time::Instant::now() < deadline);
            preview.capture(&mut app, &ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if collection == "FormDrag" {
            // Halfway through dragging the dropdown down and to the right.
            app.preview_form_select("Country");
            preview.capture(&mut app, &ctx);
            let page = ctx
                .read_response(egui::Id::new((app.tabs[0].id, 0_usize, "form_page")))
                .unwrap()
                .rect;
            let scale = app.tabs[0].scale;
            let from = page.min + vec2(160., 551.) * scale;
            let to = from + vec2(110., 70.) * scale;
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            preview.capture_with(&mut app, &ctx, vec![egui::Event::PointerMoved(from)]);
            preview.capture_with(&mut app, &ctx, vec![button(from, true)]);
            for i in 1..=8 {
                preview.capture_with(
                    &mut app,
                    &ctx,
                    vec![egui::Event::PointerMoved(
                        from + (to - from) * i as f32 / 8.,
                    )],
                );
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            while !app.ghost_ready() {
                assert!(std::time::Instant::now() < deadline);
                preview.capture(&mut app, &ctx);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        if collection == "FormDesign" {
            app.preview_form_select("Country");
            for _ in 0..3 {
                preview.capture(&mut app, &ctx);
            }
        }
        if collection == "InlineText" {
            app.content.existing_preview(app.tabs[0].id);
            for _ in 0..3 {
                preview.capture(&mut app, &ctx);
            }
        }
        if collection == "BlockText" {
            app.content.block_preview(app.tabs[0].id);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            for _ in 0..3 {
                preview.capture(&mut app, &ctx);
            }
            while !app.content.erased_ready() {
                assert!(std::time::Instant::now() < deadline);
                preview.capture(&mut app, &ctx);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        // A lasso is drawn only while the mouse button is held, here on the gray canvas.
        let events = if collection == "Lasso" {
            let lasso = (0..=40)
                .map(|i| {
                    let a = i as f32 / 40. * std::f32::consts::TAU * 0.92 + 2.6;
                    [
                        300. + a.cos() * 230. + (a * 3.).sin() * 14.,
                        640. + a.sin() * 95.,
                    ]
                })
                .collect();
            selection::preview_lasso(&mut app.editor, app.tabs[0].id, lasso);
            let p = pos2(240., 420.);
            vec![
                egui::Event::PointerMoved(p),
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        } else {
            vec![]
        };
        let output = preview.capture_with(&mut app, &ctx, events);
        preview.save(
            &ctx,
            output,
            &root.join(format!("{}.png", collection.to_lowercase())),
        );
    }
}
