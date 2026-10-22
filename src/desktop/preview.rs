//! Manual visual QA of the actual egui layout, without a window server.
//! Run with MYPDF_PREVIEW_DIR and `cargo test --bin mypdf native_visual_preview -- --ignored`.
use super::*;
use eframe::App;

#[derive(Default)]
struct Preview {
    textures: HashMap<egui::TextureId, egui::ColorImage>,
    time: f64,
    size: Option<egui::Vec2>,
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
        self.time += 1. / 60.;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    pos2(0.0, 0.0),
                    self.size.unwrap_or(vec2(1200.0, 820.0)),
                )),
                max_texture_side: Some(8192),
                time: Some(self.time),
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

    /// Async document loading can finish on any frame. Give newly visible
    /// areas their first layout, then advance past fade/scroll animations and
    /// allow measured popup sizes to settle before taking a reference image.
    /// This uses virtual time, not a wall-clock sleep or disabled animations.
    fn settle(&mut self, app: &mut Viewer, ctx: &egui::Context) {
        self.capture(app, ctx);
        self.time = self.time.ceil() + f64::from(ctx.style().animation_time) + 1.;
        for _ in 0..4 {
            self.capture(app, ctx);
        }
    }
    fn save(&self, ctx: &egui::Context, output: egui::FullOutput, path: &std::path::Path) {
        let scale = output.pixels_per_point;
        let size = self.size.unwrap_or(vec2(1200.0, 820.0));
        let (w, h) = ((size.x * scale) as u32, (size.y * scale) as u32);
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

const PREVIEW_AGREEMENT: &str = "# Service Agreement\n\nThis agreement is made between Northwind Ltd and the Client.\n\n## 1. Services\n\nNorthwind delivers the monthly report by the fifth working day.\n\n## 2. Payment\n\nPayment is due within thirty days of the invoice date.\n\nLate payments carry a fee of 2% per month.\n\n## 3. Term\n\nThis agreement lasts for twelve months and renews automatically.\n\n## 4. Disputes\n\nDisputes are settled by the courts of Ankara.";

/// Capture the unmodified application UI with fictional, public sample documents.
/// Run on Windows for the Store listing; no window decorations or marketing overlays.
#[test]
#[ignore = "Manual Store artwork; requires explicit output, sample and isolated data folders"]
fn store_listing_preview() {
    let root = PathBuf::from(std::env::var_os("MYPDF_PREVIEW_DIR").expect("Set MYPDF_PREVIEW_DIR"));
    let samples =
        PathBuf::from(std::env::var_os("MYPDF_SAMPLE_DIR").expect("Set MYPDF_SAMPLE_DIR"));
    assert!(
        std::env::var_os("MYPDF_DATA_DIR").is_some(),
        "Use an isolated MYPDF_DATA_DIR"
    );
    std::fs::create_dir_all(&root).unwrap();
    for (scene, active) in [
        ("01-read", 0),
        ("02-stamps", 0),
        ("03-forms", 1),
        ("04-page-layout", 0),
    ] {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(2.0);
        ctx.set_theme(egui::Theme::Light);
        let mut app = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            [
                "Quarterly Report.pdf",
                "Supplier Registration.pdf",
                "Service Agreement v1.pdf",
            ]
            .map(|name| samples.join(name))
            .to_vec(),
        );
        ctx.set_theme(egui::Theme::Light);
        app.home.visible = false;
        app.active = active;
        for tab in &mut app.tabs {
            tab.zoom = Zoom::Page;
        }
        app.editor.mode = match scene {
            "02-stamps" => editing::Mode::Annotate,
            "03-forms" => editing::Mode::Forms,
            _ => editing::Mode::Read,
        };
        app.editor.library_open = scene == "02-stamps";
        app.editor.library_tab = "Stamps".into();
        let mut preview = Preview::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            preview.capture(&mut app, &ctx);
            if app.tabs[active].index_done
                && app.tabs[active].rasters.keys().any(|key| !key.thumbnail)
                && app.tabs[active].pending.is_empty()
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Scene {scene} did not load"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if scene == "04-page-layout" {
            app.editor.mode = editing::Mode::Pages;
            app.preview_nup();
        }
        for _ in 0..16 {
            preview.capture(&mut app, &ctx);
        }
        while app.nup_preview_pending() || !app.tabs[active].pending.is_empty() {
            assert!(
                std::time::Instant::now() < deadline,
                "Scene {scene} did not settle"
            );
            preview.capture(&mut app, &ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let output = preview.capture(&mut app, &ctx);
        preview.save(&ctx, output, &root.join(format!("{scene}.png")));
    }
}

const SCENES: &[&str] = &[
    "TabGroups",
    "DocumentChooser",
    "WorkflowSettings",
    "Appearance",
    "ReusableStyles",
    "ReusableTextStyle",
    "ReusableDrawingStyle",
    "ReusableColorPicker",
    "ReusableColorPickerMore",
    "Templates",
    "Batch",
    "SharingCopy",
    "Home",
    "HomeEmpty",
    "Viewer",
    "Arrows",
    "Symbols",
    "Stamps",
    "Pages",
    "Crop",
    "Edit",
    "EditBlank",
    "InlineText",
    "BlockText",
    "Images",
    "Convert",
    "Draw",
    "Inspector",
    "SheetLayout",
    "SignatureImage",
    "TransparentImage",
    "ImageEffects",
    "CropImage",
    "ReviewComments",
    "BookmarkEditor",
    "Workspace",
    "Objects",
    "ToolSearch",
    "History",
    "RepeatArtwork",
    "ContextDrawing",
    "ContextImage",
    "ContextText",
    "ContextBlank",
    "RegionCopy",
    "AddText",
    "Prompt",
    "Confirmation",
    "Group",
    "Lasso",
    "DigitalIds",
    "MySignatures",
    "MySignaturesEditor",
    "MySignaturesPlacement",
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
    "Outline",
    "PasswordPrompt",
    "Protect",
    "ReduceSize",
    "LinkConfirm",
    "Watermark",
    "RecognizeText",
    "MyDetails",
    "FillDetails",
    "Zen",
    "CompareChoose",
    "Compare",
    "Signatures",
    "WindowsMenus",
    "Table",
    "DetectedTable",
];

#[test]
#[ignore = "Manual visual QA; writes only to the explicitly selected preview directory"]
fn native_visual_preview() {
    let root = PathBuf::from(std::env::var_os("MYPDF_PREVIEW_DIR").expect("Set MYPDF_PREVIEW_DIR"));
    std::fs::create_dir_all(&root).unwrap();
    let theme = if std::env::var("MYPDF_PREVIEW_THEME").as_deref() == Ok("dark") {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    for &collection in SCENES {
        if let Ok(only) = std::env::var("MYPDF_PREVIEW_ONLY")
            && !only.split(',').any(|v| v.eq_ignore_ascii_case(collection))
        {
            continue;
        }
        let validate_only = std::env::var_os("MYPDF_PREVIEW_VALIDATE_ONLY").is_some();
        capture_scene(
            collection,
            theme,
            (!validate_only).then_some(root.as_path()),
        );
        if let Some(baselines) = std::env::var_os("MYPDF_PREVIEW_BASELINE") {
            assert!(!validate_only, "Baseline comparison needs rendered PNGs");
            compare_baseline(&root, std::path::Path::new(&baselines), collection);
        }
    }
}

/// Optional reviewed baselines. The harness never replaces these itself: a UI
/// change must be visually reviewed before its new captures become baselines.
fn compare_baseline(output: &std::path::Path, baselines: &std::path::Path, scene: &str) {
    let filename = format!("{}.png", scene.to_lowercase());
    let actual = image::open(output.join(&filename)).unwrap().into_rgba8();
    let expected = image::open(baselines.join(&filename))
        .unwrap_or_else(|e| panic!("{scene}: reviewed baseline missing or unreadable: {e}"))
        .into_rgba8();
    assert_eq!(
        actual.dimensions(),
        expected.dimensions(),
        "{scene}: baseline dimensions changed"
    );
    // Ignore small rasterization differences, but not shifted controls, covered
    // pages, missing text, or an unrelated modal hiding the intended screen.
    let changed = actual
        .pixels()
        .zip(expected.pixels())
        .filter(|(a, b)| a.0.iter().zip(b.0.iter()).any(|(a, b)| a.abs_diff(*b) > 16))
        .count();
    let fraction = changed as f64 / (actual.width() as f64 * actual.height() as f64);
    eprintln!(
        "{scene}: {:.4}% pixels differ from reviewed baseline",
        fraction * 100.
    );
    assert!(
        fraction <= 0.005,
        "{scene}: {:.2}% of pixels differ from the reviewed baseline (limit 0.5%); inspect {} before accepting a new baseline",
        fraction * 100.,
        output.join(filename).display()
    );
}

/// Each scene owns its document, Viewer, input context and texture cache. Test
/// constructors already use isolated temporary profile/library directories.
/// A scene must work alone as well as in the full gallery; never rely on the
/// preceding scene to dismiss a modal or create document content.
fn capture_scene(collection: &str, theme: egui::Theme, root: Option<&std::path::Path>) {
    let fixture = tempfile::tempdir().unwrap();
    let pdf = fixture.path().join("Document.pdf");
    let source =
        mypdf_milestone0::document::fixture(&mypdf_milestone0::document::default_font().unwrap())
            .unwrap();
    let source = if matches!(collection, "AddText" | "EditBlank") {
        mypdf_milestone0::convert::blank(595., 842.).unwrap()
    } else if matches!(collection, "Table" | "DetectedTable") {
        let source = mypdf_milestone0::content::add_text(
            &mypdf_milestone0::convert::blank(595., 842.).unwrap(),
            1,
            "Quarterly review",
            [50., 55., 540., 90.],
            22.,
            [0.1, 0.15, 0.2],
        )
        .unwrap();
        let mut table = mypdf_milestone0::tables::Table::new(4, 3, [50., 130., 545., 350.]);
        table.apply_template(1);
        table.cells = vec![
            "Item",
            "Owner",
            "Status",
            "Annual review",
            "Deniz",
            "In progress",
            "Supplier audit",
            "Orhan",
            "Approved",
            "Follow-up",
            "Selin",
            "",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let bytes = mypdf_milestone0::tables::write(&source, 1, &table).unwrap();
        if collection == "DetectedTable" {
            let mut doc = lopdf::Document::load_mem(&bytes).unwrap();
            let id = doc.get_pages()[&1];
            let mut ops = doc.get_and_decode_page_content(id).unwrap();
            for op in &mut ops.operations {
                if op.operator == "BDC"
                    && let Some(d) = op.operands.get_mut(1).and_then(|o| o.as_dict_mut().ok())
                {
                    d.remove(b"MyPDFTable");
                }
            }
            let stream = doc.add_object(lopdf::Stream::new(
                lopdf::Dictionary::new(),
                ops.encode().unwrap(),
            ));
            doc.get_dictionary_mut(id).unwrap().set("Contents", stream);
            mypdf_milestone0::content::serialize(&mut doc).unwrap()
        } else {
            bytes
        }
    } else if collection == "RepeatArtwork" {
        let source = mypdf_milestone0::convert::text_to_pdf(PREVIEW_AGREEMENT, true).unwrap();
        let mut doc = lopdf::Document::load_mem(&source).unwrap();
        let page_id = *doc.get_pages().values().next().unwrap();
        let original = doc.get_dictionary(page_id).unwrap().clone();
        let parent = original.get(b"Parent").unwrap().as_reference().unwrap();
        let mut kids = Vec::new();
        for page in 0..52 {
            let mut dictionary = original.clone();
            if page % 3 == 1 {
                dictionary.set("MediaBox", vec![0.into(), 0.into(), 842.into(), 595.into()]);
            }
            kids.push(lopdf::Object::Reference(doc.add_object(dictionary)));
        }
        let pages = doc.get_dictionary_mut(parent).unwrap();
        pages.set("Kids", kids);
        pages.set("Count", 52);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        bytes
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
    ctx.set_theme(theme);
    if let Ok(style) = std::env::var("MYPDF_PREVIEW_STYLE") {
        ui::set_visual_style(&ctx, theme::Style::from_key(&style)).unwrap();
    }
    if let Ok(style) = std::env::var("MYPDF_PREVIEW_ICONS") {
        ui::set_icon_style(&ctx, theme::IconStyle::from_key(&style)).unwrap();
    }
    if collection == "WindowsMenus" {
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("preview_portable_menu"), true));
    }
    app.editor.mode = editing::Mode::Annotate;
    app.tabs[0].zoom = Zoom::Page;
    let mut preview = Preview::default();
    if let Ok(size) = std::env::var("MYPDF_PREVIEW_SIZE") {
        let (w, h) = size.split_once('x').expect("Use WIDTHxHEIGHT");
        preview.size = Some(vec2(w.parse().unwrap(), h.parse().unwrap()));
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !app.tabs[0].index_done || !app.tabs[0].rasters.keys().any(|key| !key.thumbnail) {
        assert!(std::time::Instant::now() < deadline);
        preview.capture(&mut app, &ctx);
        std::thread::sleep(std::time::Duration::from_millis(10));
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
    if matches!(
        collection,
        "Edit" | "EditBlank" | "Images" | "InlineText" | "BlockText" | "Table" | "DetectedTable"
    ) {
        app.editor.mode = editing::Mode::Edit;
        app.content.image_mode = collection == "Images";
    }
    if matches!(collection, "Table" | "DetectedTable") {
        super::table_tools::preview(&mut app, &ctx, collection == "DetectedTable");
    }
    if collection == "Viewer" {
        app.editor.mode = editing::Mode::Read;
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
    if matches!(
        collection,
        "ImageEffects"
            | "CropImage"
            | "Objects"
            | "ToolSearch"
            | "History"
            | "RepeatArtwork"
            | "ReviewComments"
    ) {
        use mypdf_milestone0::annotations::{Annotation, Kind, StrokeStyle, Style};
        let image = image::RgbaImage::from_fn(320, 200, |x, y| {
            let inside = (x as f32 - 160.).powi(2) + (y as f32 - 100.).powi(2) < 70.0_f32.powi(2);
            if inside {
                image::Rgba([35, 130, 190, 255])
            } else {
                image::Rgba([0, 0, 0, 0])
            }
        });
        let mut png = std::io::Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let mark = Annotation::new(
            0,
            [150., 330., 470., 530.],
            Kind::Image {
                png: Arc::new(png.into_inner()),
            },
            Style {
                image_blur: 3.,
                image_border: true,
                width: 3.,
                stroke: StrokeStyle::Dashed,
                opacity: 0.85,
                ..Default::default()
            },
        );
        app.editor.mode = editing::Mode::Draw;
        app.tabs[0].selected_annotation = Some(mark.id.clone());
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![mark])
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
    if collection.starts_with("Context") {
        use mypdf_milestone0::annotations::{Annotation, Kind};
        app.editor.mode = editing::Mode::Edit;
        app.content = content::ContentState::new();
        let kind = if collection == "ContextImage" {
            Kind::Image {
                png: Arc::new(clipboard::png(2, 2, &[20, 110, 180, 255].repeat(4)).unwrap()),
            }
        } else {
            Kind::Rectangle
        };
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![Annotation::new(
                0,
                [150., 330., 400., 460.],
                kind,
                Default::default(),
            )])
            .unwrap();
        if collection == "ContextText" {
            app.content.text_preview(app.tabs[0].id);
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
    if collection == "Appearance" {
        app.digital.open = true;
        app.digital.panel = digital::Panel::Interface;
    }
    if collection.starts_with("Reusable") {
        app.digital.open = true;
        app.digital.panel = digital::Panel::Styles;
        if collection != "ReusableStyles" {
            styles::preview_edit(&ctx, collection == "ReusableTextStyle");
        }
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
        if collection == "MySignaturesPlacement" {
            app.editor.mode = editing::Mode::Sign;
            app.editor.tool = editing::Tool::SignArea;
            app.editor.pending_sign_rect = Some((app.tabs[0].id, 0, [80., 310., 380., 410.]));
            app.digital_events(&ctx);
            app.editor.tool = editing::Tool::Select;
            app.digital.panel = digital::Panel::Signatures;
            app.sidebar = false;
            preview.capture(&mut app, &ctx);
            while !app.digital.previews_ready() {
                assert!(std::time::Instant::now() < deadline);
                preview.capture(&mut app, &ctx);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
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
    if matches!(collection, "FormDesign" | "FormOrder" | "FormDrag") {
        app.preview_form();
    }
    if collection == "FormDrag" {
        app.forms.layout_dialog = None;
        app.editor.mode = editing::Mode::Forms;
    }
    if collection == "Outline" {
        app.forms.layout_dialog = None;
        app.editor.mode = editing::Mode::Read;
        app.preview_outline();
    }
    if collection == "PasswordPrompt" {
        app.outline = false;
        app.preview_password_prompt();
    }
    if collection == "Protect" {
        app.preview_protect();
    }
    if collection == "ReduceSize" {
        app.protect = None;
        app.shrink = Some(mypdf_milestone0::compress::Level::Medium);
    }
    if collection == "Watermark" {
        app.link_confirm = None;
        app.editor.mode = editing::Mode::Edit;
        app.preview_watermark();
    }
    if collection == "RecognizeText" {
        app.watermark = None;
        app.ocr_confirm = true;
    }
    if collection == "MyDetails" {
        app.ocr_confirm = false;
        app.editor.mode = editing::Mode::Forms;
        app.details.editor = Some(mypdf_milestone0::form_profile::Profile {
            full_name: "Ayşe Yılmaz".into(),
            email: "ayse@example.com".into(),
            phone: "+90 555 123 45 67".into(),
            company: "Northwind Ltd".into(),
            city: "İstanbul".into(),
            country: "Türkiye".into(),
            ..Default::default()
        });
    }
    if collection == "FillDetails" {
        use mypdf_milestone0::form_profile::{Detail, Suggestion};
        app.details.editor = None;
        app.details.review = Some(
            [
                ("Adi_Soyadi", Detail::FullName, "Ayşe Yılmaz"),
                ("E_posta", Detail::Email, "ayse@example.com"),
                ("Telefon", Detail::Phone, "+90 555 123 45 67"),
                ("Sirket", Detail::Company, "Northwind Ltd"),
                ("Tarih", Detail::Today, "15.10.2026"),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, (name, detail, value))| {
                (
                    Suggestion {
                        field: (i as u32 + 1, 0),
                        name: name.into(),
                        detail,
                        value: value.into(),
                    },
                    i != 3,
                )
            })
            .collect(),
        );
    }
    if collection == "Zen" {
        app.details.review = None;
        app.ocr_confirm = false;
        app.editor.mode = editing::Mode::Read;
        app.toggle_zen(&ctx);
    }
    if collection == "CompareChoose" {
        if app.zen.is_some() {
            app.toggle_zen(&ctx);
        }
        app.request_compare();
    }
    if collection == "Compare" {
        app.compare_chooser = None;
        let versions = [
            ("Agreement v1.pdf", PREVIEW_AGREEMENT.to_owned()),
            (
                "Agreement v2.pdf",
                PREVIEW_AGREEMENT
                    .replace("fifth", "third")
                    .replace("thirty days", "sixty days")
                    .replace("Late payments carry a fee of 2% per month.\n\n", "")
                    .replace(
                        "renews automatically.",
                        "renews automatically.\n\nEither party may end it with sixty days' notice.",
                    ),
            ),
        ]
        .map(|(name, text)| {
            let path = fixture.path().join(name);
            std::fs::write(
                &path,
                mypdf_milestone0::convert::text_to_pdf(&text, true).unwrap(),
            )
            .unwrap();
            app.open(path.clone(), &ctx);
            let path = std::fs::canonicalize(&path).unwrap();
            app.tabs.iter().find(|t| t.path == path).unwrap().id
        });
        app.start_compare(
            comparison::Pick::Tab(versions[0]),
            comparison::Pick::Tab(versions[1]),
            &ctx,
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let drawn = |app: &Viewer| {
            app.comparison.as_ref().is_some_and(|c| c.result.is_some())
                && versions.iter().all(|id| {
                    app.tabs
                        .iter()
                        .find(|t| t.id == *id)
                        .is_some_and(|t| t.rasters.keys().any(|k| !k.thumbnail))
                })
        };
        while !drawn(&app) {
            assert!(std::time::Instant::now() < deadline);
            preview.capture(&mut app, &ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if let Some(c) = &mut app.comparison {
            c.select(1);
        }
        for _ in 0..3 {
            preview.capture(&mut app, &ctx);
        }
    }
    if collection == "LinkConfirm" {
        app.shrink = None;
        app.link_confirm = Some("https://www.example.com/privacy-policy".into());
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
        app.comparison = None;
        app.tabs.truncate(1);
        app.active = 0;
        // Let go of any drag from an earlier scene without changing anything.
        app.preview_cancel_drag();
        app.link_confirm = None;
        app.ocr_confirm = false;
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
    if collection == "SheetLayout" {
        app.preview_nup();
    }
    if collection == "SignatureImage" {
        app.preview_signature_image();
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
        || app.nup_preview_pending()
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
    if collection == "RegionCopy" {
        app.region_preview();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !app.region_preview_ready() {
            assert!(std::time::Instant::now() < deadline);
            preview.capture(&mut app, &ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    if collection == "TransparentImage" {
        app.background_preview();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !app.background_preview_ready() {
            assert!(std::time::Instant::now() < deadline);
            preview.capture(&mut app, &ctx);
            std::thread::sleep(std::time::Duration::from_millis(15));
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
    if let Some(action) = match collection {
        "WorkflowSettings" => Some("workflow_settings"),
        "Templates" => Some("templates"),
        "Batch" => Some("batch"),
        "SharingCopy" => Some("sharing_copy"),
        _ => None,
    } {
        app.productivity_action(action, &ctx);
        for _ in 0..4 {
            preview.capture(&mut app, &ctx);
        }
        if collection == "SharingCopy" {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while app.productivity.is_busy() {
                assert!(std::time::Instant::now() < deadline);
                preview.capture(&mut app, &ctx);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }
    if let Some(action) = match collection {
        "CropImage" => Some("crop_object"),
        "Objects" => Some("objects"),
        "ToolSearch" => Some("tool_search"),
        "History" => Some("history"),
        "RepeatArtwork" => Some("repeat_objects"),
        "ReviewComments" => Some("review_comments"),
        "BookmarkEditor" => Some("edit_navigation"),
        "Workspace" => Some("workspace"),
        _ => None,
    } {
        app.workbench_action(action, &ctx);
        for _ in 0..3 {
            preview.capture(&mut app, &ctx);
        }
    }
    if collection.starts_with("Context") {
        // The inspector changes the viewport after its async content arrives.
        // Production preserves reading position across that resize; screenshots
        // need a deliberate, repeatable initial page position instead of an
        // anchor inherited from however many loading frames happened to run.
        preview.settle(&mut app, &ctx);
        app.tabs[0].jump(0);
        preview.settle(&mut app, &ctx);
        let t = &app.tabs[0];
        let (view, offset, _) = t.document_view.unwrap();
        assert!(
            offset.y.abs() < 0.01,
            "{collection}: page 1 must start at the top"
        );
        let layout = t.document_layout.as_ref().unwrap();
        let point = if collection == "ContextText" {
            vec2(120., 190.)
        } else if collection == "ContextBlank" {
            vec2(450., 600.)
        } else {
            vec2(150., 380.)
        };
        let p = view.min - offset + layout.rects[0].min.to_vec2() + point * layout.scale;
        for pressed in [true, false] {
            preview.capture_with(
                &mut app,
                &ctx,
                vec![
                    egui::Event::PointerMoved(p),
                    egui::Event::PointerButton {
                        pos: p,
                        button: egui::PointerButton::Secondary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        preview.settle(&mut app, &ctx);
    }
    if matches!(collection, "TabGroups" | "DocumentChooser") {
        ctx.enable_accesskit();
        app.tab_controls.groups.push(tabs::Group {
            id: 1,
            name: "Work".into(),
            active: None,
        });
        for name in [
            "Annual report 2026",
            "Audit checklist",
            "Project proposal",
            "Meeting notes",
            "Supplier agreement",
            "Budget review",
        ] {
            let path = fixture.path().join(format!("{name}.pdf"));
            std::fs::copy(&pdf, &path).unwrap();
            app.open(path, &ctx);
        }
        for tab in &mut app.tabs {
            tab.document_group = 1;
        }
        app.active = 0;
        app.home.visible = false;
        preview.settle(&mut app, &ctx);
        if collection == "DocumentChooser" {
            let snapshot = preview.capture(&mut app, &ctx);
            let nodes = snapshot.platform_output.accesskit_update.unwrap().nodes;
            let bounds = nodes
                .iter()
                .find(|(_, n)| n.label() == Some("Open documents (7)"))
                .unwrap()
                .1
                .bounds()
                .unwrap();
            let at = pos2(
                (bounds.x0 + bounds.x1) as f32 / 2.,
                (bounds.y0 + bounds.y1) as f32 / 2.,
            );
            for pressed in [true, false] {
                preview.capture_with(
                    &mut app,
                    &ctx,
                    vec![
                        egui::Event::PointerMoved(at),
                        egui::Event::PointerButton {
                            pos: at,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            preview.settle(&mut app, &ctx);
        }
    }
    if collection.starts_with("ReusableColorPicker") {
        ctx.enable_accesskit();
        for label in if collection == "ReusableColorPickerMore" {
            vec!["Custom color", "More colors…"]
        } else {
            vec!["Custom color"]
        } {
            preview.settle(&mut app, &ctx);
            let output = preview.capture(&mut app, &ctx);
            let rect = output
                .platform_output
                .accesskit_update
                .unwrap()
                .nodes
                .into_iter()
                .find(|(_, n)| n.label() == Some(label))
                .unwrap()
                .1
                .bounds()
                .unwrap();
            let pos = pos2(
                ((rect.x0 + rect.x1) / 2.) as f32,
                ((rect.y0 + rect.y1) / 2.) as f32,
            );
            for pressed in [true, false] {
                preview.capture_with(
                    &mut app,
                    &ctx,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
        }
        preview.settle(&mut app, &ctx);
    }
    if collection == "RepeatArtwork" {
        preview.settle(&mut app, &ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !app.tabs[0].pending.is_empty() {
            assert!(std::time::Instant::now() < deadline);
            preview.capture(&mut app, &ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    if collection == "WindowsMenus" {
        for pressed in [true, false] {
            preview.capture_with(
                &mut app,
                &ctx,
                vec![
                    egui::Event::PointerMoved(pos2(24., 13.)),
                    egui::Event::PointerButton {
                        pos: pos2(24., 13.),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        preview.settle(&mut app, &ctx);
    }
    let output = preview.capture_with(&mut app, &ctx, events);
    assert_scene(collection, &app, &output);
    if let Some(root) = root {
        preview.save(
            &ctx,
            output,
            &root.join(format!("{}.png", collection.to_lowercase())),
        );
    }
}

fn rendered_text(output: &egui::FullOutput) -> Vec<String> {
    fn visit(shape: &egui::Shape, text: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(shape) => text.push(shape.galley.job.text.clone()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| visit(s, text)),
            _ => {}
        }
    }
    let mut text = vec![];
    output
        .shapes
        .iter()
        .for_each(|s| visit(&s.shape, &mut text));
    text
}

/// Validate what the screenshot claims to show, not just that a PNG was written.
fn assert_scene(scene: &str, app: &Viewer, output: &egui::FullOutput) {
    assert_eq!(
        app.home.visible,
        matches!(scene, "Home" | "HomeEmpty"),
        "{scene}: Home visibility"
    );
    let text = rendered_text(output);
    if scene == "WindowsMenus" {
        for label in [
            "File",
            "Edit",
            "View",
            "Annotate",
            "Edit PDF",
            "Scan & OCR",
            "Sign",
            "Tools",
            "Window",
            "Help",
            "Save",
            "Save As…",
        ] {
            assert!(
                text.iter().any(|t| t == label),
                "Missing menu command {label}: {text:?}"
            );
        }
        assert!(!text.iter().any(|t| t == "Save a Copy…"));
    }

    for (owner, title) in [
        ("TransparentImage", "Remove background"),
        ("Prompt", "SVG Studio"),
        ("SheetLayout", "Multiple pages per sheet"),
        ("SignatureImage", "Handwritten signature"),
        ("CropImage", "Crop image"),
        ("History", "Document history"),
        ("ToolSearch", "Find a tool"),
        ("RepeatArtwork", "Repeat on pages"),
    ] {
        assert_eq!(
            text.iter().any(|t| t == title),
            scene == owner,
            "{scene}: expected only its own modal/window; found title {title:?}. Visible text: {text:?}"
        );
    }
    if matches!(scene, "FormDesign" | "FormOrder" | "FormDrag") {
        let fields = app
            .fresh_inspection()
            .expect("form scene inspected")
            .designed
            .as_ref()
            .expect("form fixture valid");
        assert!(
            fields.iter().any(|f| f.name == "Country"),
            "{scene}: its own field fixture is present"
        );
        assert_eq!(app.forms.show_order, scene == "FormOrder");
        if scene == "FormDrag" {
            assert!(
                app.ghost_ready(),
                "FormDrag must show the field being dragged"
            );
        }
    }
    if scene == "ContextDrawing" || scene == "ContextImage" {
        assert!(
            text.iter().any(|t| t == "Duplicate"),
            "{scene}: context menu must be open"
        );
    }
    if scene == "EditBlank" {
        assert!(
            text.iter().any(|t| t == "No editable text on this page."),
            "blank Edit must offer its own empty state"
        );
        assert!(
            text.iter().any(|t| t == "Text"),
            "blank Edit must offer text insertion"
        );
    }
    if scene == "Viewer" {
        assert!(app.editor.mode == editing::Mode::Read);
        assert!(app.close_intent.is_none() && !app.digital.open && !app.ocr_confirm);
    }
    if scene.starts_with("Reusable") {
        assert!(app.digital.panel == digital::Panel::Styles && app.digital.open);
        if scene == "ReusableStyles" {
            for label in ["Review outline", "Document heading", "New style"] {
                assert!(
                    text.iter().any(|t| t == label),
                    "Missing {label} in {scene}"
                );
            }
        } else {
            assert!(text.iter().any(|t| t == "Save style"));
            assert!(text.iter().any(|t| t == "Cancel"));
            assert!(text.iter().any(|t| t
                == if scene == "ReusableTextStyle" {
                    "Font size"
                } else {
                    "Line width"
                }));
        }
        if scene == "ReusableColorPicker" {
            assert!(text.iter().any(|t| t == "More colors…"));
        }
        if scene == "ReusableColorPickerMore" {
            assert!(text.iter().any(|t| t == "Standard colors"));
            assert!(text.iter().any(|t| t == "Hex"));
        }
    }
}

#[test]
fn visual_scenes_are_independent_and_check_expected_content() {
    // Previously the first scene leaked its modal into every later capture;
    // FormOrder/FormDrag also silently depended on running FormDesign first.
    for scene in [
        "TransparentImage",
        "Viewer",
        "EditBlank",
        "FormOrder",
        "FormDrag",
        "ContextDrawing",
    ] {
        capture_scene(scene, egui::Theme::Light, None);
    }
}
