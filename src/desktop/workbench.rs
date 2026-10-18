//! Object workflow tools. Drafts never change a document until placed or confirmed.
use super::*;
use mypdf_milestone0::{
    annotations::{Annotation, Kind},
    object_tools,
};

#[derive(Default)]
pub(super) struct State {
    pub reveal: Option<(u64, usize, Option<[f32; 4]>, std::time::Instant)>,
    pub request: Option<String>,
    pub paste: Option<Vec<Annotation>>,
    pub crop: Option<Crop>,
    repeat: Option<Repeat>,
    history: bool,
    palette: bool,
    query: String,
    focus_query: bool,
    choice: usize,
    rename_revision: Option<(u64, u64)>,
}
pub(super) struct Crop {
    on_page: bool,
    apply: bool,
    cancel: bool,
    key: (u64, u64),
    original: Annotation,
    texture: TextureHandle,
    pixels: [f32; 2],
    rect: [f32; 4],
    keep_aspect: bool,
    error: Option<String>,
}
struct Repeat {
    key: (u64, u64),
    marks: Vec<Annotation>,
    range: String,
}
pub(super) fn positioned(
    marks: &[Annotation],
    page: usize,
    point: [f32; 2],
    size: PageSize,
) -> Vec<Annotation> {
    let b = selection::union_bounds(marks.iter());
    let mut result = selection::moved(marks, [point[0] - b[0], point[1] - b[1]], size);
    for a in &mut result {
        a.page = page;
    }
    result
}
pub(super) fn place(
    editor: &mut editing::EditorState,
    tab: &mut Tab,
    marks: Vec<Annotation>,
    status: &mut Option<(String, bool)>,
) -> bool {
    let Some(s) = &mut tab.session else {
        return false;
    };
    let ids = marks.iter().map(|a| a.id.clone()).collect();
    let mut entries = s.annotations.clone();
    entries.extend(marks);
    match s.commit(entries) {
        Ok(()) => {
            selection::set(tab, editor, ids);
            editor.mode = editing::Mode::Draw;
            editor.tool = editing::Tool::Select;
            *status = Some(("Artwork placed · Undo to remove".into(), false));
            true
        }
        Err(e) => {
            *status = Some((format!("{e:#}"), true));
            false
        }
    }
}
#[allow(clippy::too_many_arguments)]
pub(super) fn paste_overlay(
    state: &mut State,
    editor: &mut editing::EditorState,
    tab: &mut Tab,
    ui: &mut egui::Ui,
    page: usize,
    rect: Rect,
    scale: f32,
    status: &mut Option<(String, bool)>,
) -> bool {
    let Some(marks) = &state.paste else {
        return false;
    };
    // Draw the real managed layer while its ordinary gestures are suspended.
    if let Some(s) = &tab.session
        && s.editable
    {
        for a in s.annotations.iter().filter(|a| a.page == page) {
            editing::paint_annotation(editor, ui.ctx(), ui.painter(), rect, scale, a);
        }
    }
    let r = ui
        .interact(
            rect,
            egui::Id::new((tab.id, page, "paste_preview")),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::Crosshair);
    if let Some(pos) = r.hover_pos() {
        let p = (pos - rect.min) / scale;
        let copies = positioned(marks, page, [p.x, p.y], tab.pages[page]);
        for a in &copies {
            let mut ghost = a.clone();
            ghost.style.opacity *= 0.55;
            editing::paint_annotation(
                editor,
                ui.ctx(),
                &ui.painter().with_clip_rect(rect.intersect(ui.clip_rect())),
                rect,
                scale,
                &ghost,
            );
        }
        if r.clicked() && place(editor, tab, copies, status) {
            state.paste = None;
        }
    }
    true
}

/// Page-space crop handles use the inverse of the same rotation/flip as drawing.
pub(super) fn crop_overlay(
    state: &mut State,
    editor: &mut editing::EditorState,
    tab: &Tab,
    ui: &mut egui::Ui,
    page: usize,
    rect: Rect,
    scale: f32,
) -> bool {
    let Some(d) = &mut state.crop else {
        return false;
    };
    let Some(s) = &tab.session else { return false };
    for a in s
        .annotations
        .iter()
        .filter(|a| a.page == page && (a.id != d.original.id || !d.on_page))
    {
        editing::paint_annotation(editor, ui.ctx(), ui.painter(), rect, scale, a);
    }
    if !d.on_page || d.key != (tab.id, s.revision()) || page != d.original.page {
        return true;
    }
    let Ok(mut full) = object_tools::crop(&d.original, None) else {
        return true;
    };
    full.style.opacity = 0.28;
    editing::paint_annotation(editor, ui.ctx(), ui.painter(), rect, scale, &full);
    if let Ok(preview) = object_tools::crop(&d.original, Some(d.rect)) {
        editing::paint_annotation(editor, ui.ctx(), ui.painter(), rect, scale, &preview);
    }
    let points = [
        [d.rect[0], d.rect[1]],
        [d.rect[2], d.rect[1]],
        [d.rect[2], d.rect[3]],
        [d.rect[0], d.rect[3]],
    ];
    let screen: Vec<_> = points
        .iter()
        .map(|p| {
            let p = full.rotated(full.local(*p));
            rect.min + vec2(p[0], p[1]) * scale
        })
        .collect();
    ui.painter().add(egui::Shape::closed_line(
        screen.clone(),
        Stroke::new(1.5, theme::accent()),
    ));
    for (i, c) in screen.iter().enumerate() {
        let response = ui
            .interact(
                Rect::from_center_size(*c, vec2(22., 22.)),
                egui::Id::new((tab.id, page, "canvas_crop", i)),
                egui::Sense::drag(),
            )
            .on_hover_cursor(egui::CursorIcon::Crosshair);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Slider,
                true,
                format!("Crop corner {}", i + 1),
            )
        });
        ui.painter().circle_filled(*c, 5., Color32::WHITE);
        ui.painter()
            .circle_stroke(*c, 5., Stroke::new(2., theme::accent()));
        if response.dragged()
            && let Some(p) = response.interact_pointer_pos()
        {
            let p = (p - rect.min) / scale;
            let p = mypdf_milestone0::annotations::rotate([p.x, p.y], full.rect, -full.rotation);
            let mut p = [
                (p[0] - full.rect[0]) / (full.rect[2] - full.rect[0]),
                (p[1] - full.rect[1]) / (full.rect[3] - full.rect[1]),
            ];
            if full.flip_x {
                p[0] = 1. - p[0];
            }
            if full.flip_y {
                p[1] = 1. - p[1];
            }
            let old = d.original.object.crop.unwrap_or([0., 0., 1., 1.]);
            d.rect = crop_corner(
                d.rect,
                i,
                p,
                (d.keep_aspect || ui.input(|i| i.modifiers.shift))
                    .then_some((old[2] - old[0]) / (old[3] - old[1])),
            );
        }
    }
    true
}

pub(super) fn reveal_overlay(
    state: &State,
    tab: &Tab,
    ui: &egui::Ui,
    page: usize,
    rect: Rect,
    scale: f32,
) {
    let Some((id, p, area, started)) = state.reveal else {
        return;
    };
    if id != tab.id || p != page || started.elapsed().as_secs_f32() > 1.4 {
        return;
    }
    let r = area
        .map(|a| {
            Rect::from_min_max(
                rect.min + vec2(a[0], a[1]) * scale,
                rect.min + vec2(a[2], a[3]) * scale,
            )
            .expand(5.)
        })
        .unwrap_or(rect.shrink(3.));
    let alpha = ((1. - started.elapsed().as_secs_f32() / 1.4) * 180.) as u8;
    ui.painter().rect_stroke(
        r,
        3.,
        Stroke::new(3., Color32::from_rgba_unmultiplied(52, 130, 255, alpha)),
        egui::StrokeKind::Inside,
    );
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(30));
}

const COMMANDS: &[(&str, &str, &str)] = &[
    (
        "Edit bookmarks and page labels",
        "edit_navigation",
        "outline numbering roman sections",
    ),
    ("New blank PDF", "new", "empty create document"),
    ("Reopen Closed Tab", "reopen_closed", "restore tab document"),
    (
        "Review comments",
        "review_comments",
        "reply resolve author annotation summary",
    ),
    (
        "Workspace settings",
        "workspace",
        "startup restore tabs panels",
    ),
    ("Group objects", "group_objects", "together join selection"),
    ("Ungroup objects", "ungroup_objects", "separate selection"),
    ("Open PDF", "open", "file"),
    ("Edit text", "edit_text", "font paragraph words"),
    ("Edit images", "edit_images", "picture logo"),
    ("Add image", "add_image", "picture photo import"),
    ("Crop selected image", "crop_object", "trim margins restore"),
    (
        "Remove image background",
        "object_background",
        "transparent transparency signature",
    ),
    (
        "Objects and layers",
        "objects",
        "marks name hide visible lock unlock reorder",
    ),
    (
        "Repeat selection on pages",
        "repeat_objects",
        "stamp all pages duplicate",
    ),
    (
        "Paste in original position",
        "paste_original",
        "clipboard copy region",
    ),
    (
        "Show / hide editing guides",
        "guides",
        "border outline clean canvas",
    ),
    ("Document history", "history", "undo redo changes"),
    ("Merge PDFs", "merge_files", "combine insert pages"),
    (
        "Visual signature",
        "visual_signature",
        "draw sign handwriting",
    ),
    ("Digital signature", "sign", "certificate keychain sign"),
    ("Recognize text (OCR)", "ocr", "scan searchable"),
    ("Watermark", "watermark", "background"),
    ("Header and footer", "hf_text", "page numbering"),
    ("Pages per sheet", "nup", "thumbnail handout print"),
    ("Reduce file size", "shrink", "compress"),
    ("Compare documents", "compare", "difference versions"),
    ("Document recovery", "recovery", "autosave backup"),
    ("Fit width", "fit_width", "zoom"),
    ("Zen mode", "zen", "fullscreen reading"),
];
fn commands(query: &str) -> Vec<usize> {
    let words: Vec<_> = query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    COMMANDS
        .iter()
        .enumerate()
        .filter(|(_, (label, _, aliases))| {
            let haystack = format!("{label} {aliases}").to_lowercase();
            words.iter().all(|w| haystack.contains(w))
        })
        .map(|(i, _)| i)
        .collect()
}
impl Viewer {
    pub(super) fn workbench_owns_shortcuts(&self) -> bool {
        self.workbench.palette
            || self.workbench.crop.is_some()
            || self.workbench.repeat.is_some()
            || self.workbench.paste.is_some()
    }
    pub(super) fn workbench_keys(&mut self, ctx: &egui::Context) -> bool {
        let owns_keys = !cfg!(target_os = "macos") || self._native_menu.is_none();
        if owns_keys
            && ctx.input_mut(|i| {
                i.consume_key(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::T,
                )
            })
        {
            self.reopen_closed(ctx);
            return true;
        }
        if owns_keys && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::K)) {
            self.workbench_action("tool_search", ctx);
            return true;
        }
        if self.workbench.palette
            || self.workbench.crop.is_some()
            || self.workbench.repeat.is_some()
        {
            return true;
        }
        if owns_keys
            && self.paste_keys(ctx)
            && content::context::consume_action(ctx, "paste_original")
        {
            self.paste_original();
            return true;
        }
        if self.workbench.paste.is_some() {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.workbench.paste = None;
                self.status = Some(("Paste cancelled".into(), false));
            }
            return true;
        }
        false
    }
    pub(super) fn workbench_action(&mut self, action: &str, ctx: &egui::Context) -> bool {
        match action {
            "edit_navigation" => {
                self.open_navigation_editor();
                return true;
            }
            "review_comments" => {
                self.review.open = true;
                return true;
            }
            "reopen_closed" => {
                self.reopen_closed(ctx);
                return true;
            }
            "workspace" => {
                self.workspace.options = true;
                return true;
            }
            "tool_search" => {
                self.workbench.palette = true;
                self.workbench.query.clear();
                self.workbench.focus_query = true;
                self.workbench.choice = 0;
                return true;
            }
            "guides" => {
                self.editor.hide_guides = !self.editor.hide_guides;
                return true;
            }
            "history" => {
                self.workbench.history = !self.workbench.history;
                return true;
            }
            _ => {}
        }
        if !matches!(
            action,
            "paste_original"
                | "crop_object"
                | "repeat_objects"
                | "objects"
                | "group_objects"
                | "ungroup_objects"
                | "copy_format"
                | "paste_format"
                | "object_copy"
                | "object_cut"
                | "object_delete"
                | "object_duplicate"
                | "object_rotate"
                | "object_background"
                | "merge_files"
                | "visual_signature"
        ) {
            return false;
        }
        if self.home.visible
            || self
                .tabs
                .get(self.active)
                .and_then(|t| t.session.as_ref())
                .is_none()
        {
            return true;
        }
        if self.content.pending_text().is_some() {
            self.status = Some((
                "Finish the text edit on the page (⌘Return / Ctrl+Return) before using this tool."
                    .into(),
                true,
            ));
            return true;
        }
        match action {
            "paste_original" => self.paste_original(),
            "objects" => {
                self.sidebar = true;
                self.outline = false;
                self.search = false;
                self.editor.mode = editing::Mode::Draw;
                self.editor.annotation_sidebar = true;
            }
            "group_objects" | "ungroup_objects" | "copy_format" | "paste_format" => {
                let t = &mut self.tabs[self.active];
                let ids = selection::selected_ids(t);
                let s = t.session.as_mut().unwrap();
                let mut entries = s.annotations.clone();
                let first = entries.iter().find(|a| ids.contains(&a.id)).cloned();
                if action == "copy_format" {
                    if let Some(a) = first {
                        styles::copy(ctx, styles::Preset::Drawing(a.style));
                    }
                } else {
                    let result = if action == "paste_format" {
                        if let Some(a) = first
                            && let Some(styles::Preset::Drawing(style)) =
                                styles::pasted(ctx, &styles::Preset::Drawing(a.style))
                        {
                            for a in entries.iter_mut().filter(|a| ids.contains(&a.id)) {
                                a.style = style.clone();
                            }
                            Ok(())
                        } else {
                            Err(anyhow::anyhow!("Copy an object's style first"))
                        }
                    } else {
                        object_tools::group(&mut entries, &ids, action == "ungroup_objects")
                    };
                    match result.and_then(|()| s.commit(entries)) {
                        Ok(()) => {
                            s.name_last_change(match action {
                                "group_objects" => "Group objects",
                                "ungroup_objects" => "Ungroup objects",
                                _ => "Paste style",
                            });
                            selection::set(t, &mut self.editor, ids);
                        }
                        Err(e) => self.status = Some((e.to_string(), true)),
                    }
                }
            }
            "object_copy" => {
                self.copy_marks(ctx, false);
            }
            "object_cut" => {
                self.copy_marks(ctx, true);
            }
            "object_delete" => self.editor_delete(),
            "merge_files" => self.merge_files(ctx),
            "visual_signature" => {
                self.editor.mode = editing::Mode::Annotate;
                self.editor.signature_open = true;
            }
            "object_background" => self.selected_image_background(),
            "crop_object" => self.start_crop(ctx),
            "repeat_objects" => {
                let t = &self.tabs[self.active];
                let s = t.session.as_ref().unwrap();
                let ids = selection::selected_ids(t);
                let marks: Vec<_> = s
                    .annotations
                    .iter()
                    .filter(|a| ids.contains(&a.id) && !a.object.hidden)
                    .cloned()
                    .collect();
                if s.editable && !marks.is_empty() {
                    self.workbench.repeat = Some(Repeat {
                        key: (t.id, s.revision()),
                        marks,
                        range: format!("1-{}", t.pages.len()),
                    });
                } else {
                    self.status = Some(("Select artwork on the page first".into(), false));
                }
            }
            "object_duplicate" | "object_rotate" => {
                let t = &mut self.tabs[self.active];
                let ids = selection::selected_ids(t);
                let s = t.session.as_mut().unwrap();
                let mut entries = s.annotations.clone();
                let selected: Vec<_> = entries
                    .iter()
                    .filter(|a| ids.contains(&a.id))
                    .cloned()
                    .collect();
                if selected.iter().any(|a| a.object.locked) {
                    return true;
                }
                let ids = if action == "object_duplicate" {
                    let copies: Vec<_> = mypdf_milestone0::object_tools::duplicate(&selected);
                    let ids = copies.iter().map(|a| a.id.clone()).collect();
                    entries.extend(copies);
                    ids
                } else {
                    for a in entries.iter_mut().filter(|a| ids.contains(&a.id)) {
                        a.rotate_by(90.);
                    }
                    ids
                };
                match s.commit(entries) {
                    Ok(()) => selection::set(t, &mut self.editor, ids),
                    Err(e) => self.status = Some((e.to_string(), true)),
                }
            }
            _ => {}
        }
        true
    }
    fn start_crop(&mut self, ctx: &egui::Context) {
        let t = &self.tabs[self.active];
        let s = t.session.as_ref().unwrap();
        let Some(a) = s
            .annotations
            .iter()
            .find(|a| Some(&a.id) == t.selected_annotation.as_ref() && !a.object.locked)
        else {
            self.status = Some(("Select an unlocked image first".into(), false));
            return;
        };
        let Kind::Image { png } = &a.kind else {
            self.status = Some((
                "Select an image layer. Use Copy region for artwork inside the original PDF."
                    .into(),
                false,
            ));
            return;
        };
        if !s.editable {
            return;
        }
        match mypdf_milestone0::assets::decode_png(png) {
            Ok(image) => {
                let pixels = [image.width() as f32, image.height() as f32];
                let thumb = image.thumbnail(1200, 900).into_rgba8();
                let texture = ctx.load_texture(
                    "crop_source",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [thumb.width() as usize, thumb.height() as usize],
                        thumb.as_raw(),
                    ),
                    egui::TextureOptions::LINEAR,
                );
                self.workbench.crop = Some(Crop {
                    on_page: true,
                    apply: false,
                    cancel: false,
                    key: (t.id, s.revision()),
                    original: a.clone(),
                    texture,
                    pixels,
                    rect: a.object.crop.unwrap_or([0., 0., 1., 1.]),
                    keep_aspect: false,
                    error: None,
                });
            }
            Err(e) => self.status = Some((e.to_string(), true)),
        }
    }
    pub(super) fn workbench_windows(&mut self, ctx: &egui::Context) {
        if let Some(request) = self.workbench.request.take() {
            self.workbench_action(&request, ctx);
        }
        self.context_commands(ctx);
        self.crop_window(ctx);
        self.repeat_window(ctx);
        self.history_window(ctx);
        self.palette_window(ctx);
    }
    pub(super) fn workbench_bar(&mut self, ctx: &egui::Context) {
        if let Some(d) = &mut self.workbench.crop
            && d.on_page
        {
            egui::TopBottomPanel::top("on_page_crop_controls")
                .frame(egui::Frame::new().fill(theme::paper()).inner_margin(8.))
                .show(ctx, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.strong("Crop image");
                        ui.checkbox(&mut d.keep_aspect, "Keep proportions");
                        for (i, label) in ["L", "T", "R", "B"].iter().enumerate() {
                            let mut n = d.rect[i] * 100.;
                            if ui
                                .add(
                                    egui::DragValue::new(&mut n)
                                        .prefix(format!("{label} "))
                                        .suffix("%")
                                        .range(0. ..=100.),
                                )
                                .changed()
                            {
                                let mut r = d.rect;
                                r[i] = n / 100.;
                                if object_tools::validate_crop(r).is_ok() {
                                    d.rect = r;
                                }
                            }
                        }
                        if ui
                            .add(ActionButton::new(Icon::Reload, "Full image"))
                            .clicked()
                        {
                            d.rect = [0., 0., 1., 1.];
                        }
                        if ui
                            .add(ActionButton::new(Icon::Crop, "Detail view"))
                            .clicked()
                        {
                            d.on_page = false;
                        }
                        d.apply = ui
                            .add(ActionButton::new(Icon::Check, "Done").primary())
                            .clicked();
                        d.cancel = ui.add(ActionButton::new(Icon::Close, "Cancel")).clicked();
                    });
                });
        }

        if self.workbench.paste.is_some() {
            egui::TopBottomPanel::top("paste_instruction")
                .frame(egui::Frame::new().fill(theme::paper()).inner_margin(8.))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Click a page to place the copied artwork");
                        if ui.button("Original position").clicked() {
                            self.place_pending_original();
                        }
                        if ui.button("Cancel (Esc)").clicked() {
                            self.workbench.paste = None;
                        }
                    });
                });
        }
    }
    fn crop_window(&mut self, ctx: &egui::Context) {
        let Some(d) = &mut self.workbench.crop else {
            return;
        };
        let valid = self
            .tabs
            .get(self.active)
            .and_then(|t| t.session.as_ref().map(|s| (t.id, s.revision())))
            == Some(d.key);
        if d.on_page {
            d.cancel |= ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            d.apply |= ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            if !valid || d.cancel {
                self.workbench.crop = None;
            } else if d.apply {
                let d = self.workbench.crop.take().unwrap();
                match object_tools::crop(
                    &d.original,
                    (d.rect != [0., 0., 1., 1.]).then_some(d.rect),
                ) {
                    Ok(a) => {
                        let t = &mut self.tabs[self.active];
                        selection::commit(t, std::slice::from_ref(&a), None, &mut self.status);
                        selection::set(t, &mut self.editor, vec![a.id]);
                    }
                    Err(e) => self.status = Some((e.to_string(), true)),
                }
            }
            return;
        }
        let (mut done, mut cancel, mut restore) = (false, false, false);
        let modal = egui::Modal::new(egui::Id::new("crop_image"))
            .frame(ui::modal_frame())
            .show(ctx, |ui| {
                ui.set_width(550.);
                ui.heading("Crop image");
                ui.weak("Drag a corner to frame the image. Original pixels are kept.");
                ui.add_space(12.);
                let max = vec2(530., (ctx.content_rect().height() - 260.).clamp(160., 420.));
                let scale = (max.x / d.pixels[0]).min(max.y / d.pixels[1]);
                let size = vec2(d.pixels[0], d.pixels[1]) * scale;
                let (canvas, _) =
                    ui.allocate_exact_size(vec2(550., size.y + 20.), egui::Sense::hover());
                let image = Rect::from_center_size(canvas.center(), size);
                ui.painter().rect_filled(image, 3., Color32::WHITE);
                ui.painter().image(
                    d.texture.id(),
                    image,
                    Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)),
                    Color32::WHITE,
                );
                let r = Rect::from_min_max(
                    image.min + vec2(d.rect[0] * size.x, d.rect[1] * size.y),
                    image.min + vec2(d.rect[2] * size.x, d.rect[3] * size.y),
                );
                for band in [
                    Rect::from_min_max(image.min, pos2(image.right(), r.top())),
                    Rect::from_min_max(pos2(image.left(), r.bottom()), image.max),
                    Rect::from_min_max(pos2(image.left(), r.top()), r.left_bottom()),
                    Rect::from_min_max(r.right_top(), pos2(image.right(), r.bottom())),
                ] {
                    ui.painter()
                        .rect_filled(band, 0., Color32::from_black_alpha(120));
                }
                ui.painter().rect_stroke(
                    r,
                    0.,
                    Stroke::new(2., theme::accent()),
                    egui::StrokeKind::Outside,
                );
                for (index, c) in [
                    r.left_top(),
                    r.right_top(),
                    r.right_bottom(),
                    r.left_bottom(),
                ]
                .iter()
                .enumerate()
                {
                    let handle = ui.interact(
                        Rect::from_center_size(*c, vec2(20., 20.)),
                        egui::Id::new(("crop_handle", index)),
                        egui::Sense::drag(),
                    );
                    handle.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Slider,
                            true,
                            format!("Crop corner {}", index + 1),
                        )
                    });
                    ui.painter().circle_filled(*c, 5., Color32::WHITE);
                    ui.painter()
                        .circle_stroke(*c, 5., Stroke::new(1.5, theme::accent()));
                    if handle.dragged()
                        && let Some(p) = handle.interact_pointer_pos()
                    {
                        let p = [
                            ((p.x - image.left()) / size.x).clamp(0., 1.),
                            ((p.y - image.top()) / size.y).clamp(0., 1.),
                        ];
                        let old = d.original.object.crop.unwrap_or([0., 0., 1., 1.]);
                        d.rect = crop_corner(
                            d.rect,
                            index,
                            p,
                            d.keep_aspect
                                .then_some((old[2] - old[0]) / (old[3] - old[1])),
                        );
                    }
                }
                ui.add_space(10.);
                ui.checkbox(&mut d.keep_aspect, "Keep aspect ratio");
                ui.horizontal(|ui| {
                    if ui.button("Trim empty margins").clicked()
                        && let Kind::Image { png } = &d.original.kind
                    {
                        match object_tools::trim(png) {
                            Ok(r) => {
                                d.rect = r;
                                d.error = None
                            }
                            Err(e) => d.error = Some(e.to_string()),
                        }
                    }
                    if ui.button("Full image").clicked() {
                        d.rect = [0., 0., 1., 1.];
                    }
                    restore = ui
                        .add_enabled(
                            d.original.object.crop.is_some(),
                            egui::Button::new("Restore original"),
                        )
                        .clicked();
                });
                ui.horizontal(|ui| {
                    for (i, label) in ["Left", "Top", "Right", "Bottom"].iter().enumerate() {
                        let mut value = d.rect[i] * 100.;
                        ui.label(*label);
                        if ui
                            .add(
                                egui::DragValue::new(&mut value)
                                    .range(0.0..=100.)
                                    .speed(0.2)
                                    .suffix("%"),
                            )
                            .changed()
                        {
                            let mut next = d.rect;
                            next[i] = value / 100.;
                            if object_tools::validate_crop(next).is_ok() {
                                d.rect = next;
                            }
                        }
                    }
                });
                if let Some(e) = &d.error {
                    ui.colored_label(theme::danger(), e);
                }
                if !valid {
                    ui.weak("The document changed. Cancel and open Crop again.");
                }
                ui.add_space(10.);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    done = ui
                        .add_enabled(valid, ActionButton::new(Icon::Crop, "Done").primary())
                        .clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        if done || restore && valid {
            let d = self.workbench.crop.take().unwrap();
            let result = object_tools::crop(
                &d.original,
                if restore || d.rect == [0., 0., 1., 1.] {
                    None
                } else {
                    Some(d.rect)
                },
            );
            match result {
                Ok(a) => {
                    let t = &mut self.tabs[self.active];
                    selection::commit(t, std::slice::from_ref(&a), None, &mut self.status);
                    selection::set(t, &mut self.editor, vec![a.id]);
                }
                Err(e) => self.status = Some((e.to_string(), true)),
            }
        } else if cancel || modal.should_close() {
            self.workbench.crop = None;
        }
    }
    fn repeat_window(&mut self, ctx: &egui::Context) {
        let Some(d) = &mut self.workbench.repeat else {
            return;
        };
        let Some(t) = self.tabs.get(self.active) else {
            self.workbench.repeat = None;
            return;
        };
        let valid = t
            .session
            .as_ref()
            .is_some_and(|s| (t.id, s.revision()) == d.key && s.editable);
        let mut done = false;
        let mut cancel = false;
        let pages = mypdf_milestone0::pages::parse_range(&d.range, t.pages.len());
        let count = pages
            .as_ref()
            .map(|p| {
                p.iter()
                    .filter(|&&page| !d.marks.iter().all(|a| a.page == page))
                    .count()
                    * d.marks.len()
            })
            .unwrap_or(0);
        let fits = count + t.session.as_ref().map(|s| s.annotations.len()).unwrap_or(0) <= 5000;
        let copies = pages
            .as_ref()
            .ok()
            .filter(|_| fits)
            .map(|p| {
                object_tools::repeat(
                    &d.marks,
                    &p.iter()
                        .map(|&i| (i, [t.pages[i].width, t.pages[i].height]))
                        .collect::<Vec<_>>(),
                )
            })
            .unwrap_or_default();
        let modal=egui::Modal::new(egui::Id::new("repeat_objects")).frame(ui::modal_frame()).show(ctx,|ui|{
            ui.set_width(480.);ui.heading("Repeat on pages");ui.weak("Copies the selected artwork. The source page is skipped.");
            ui.horizontal(|ui|{ui.label("Pages");ui.text_edit_singleline(&mut d.range);});
            ui.weak("Examples: 1-4, 7, 10 · One Undo removes all copies.");
            if let Err(e)=&pages {ui.colored_label(theme::danger(),e.to_string());}
            egui::ScrollArea::vertical().max_height(300.).show(ui,|ui|{
                if let Ok(pages)=&pages {ui.horizontal_wrapped(|ui|{for &page in pages.iter().filter(|&&p|!d.marks.iter().all(|a|a.page==p)).take(12) {
                    ui.vertical(|ui|{let size=t.pages[page];let scale=(100./size.width).min(130./size.height);let (r,_)=ui.allocate_exact_size(vec2(size.width,size.height)*scale,egui::Sense::hover());ui.painter().rect_filled(r,2.,Color32::WHITE);
                        if let Some(raster)=t.rasters.iter().find(|(k,_)|k.page==page).map(|(_,r)|r){ui.painter().image(raster.texture.id(),r,Rect::from_min_max(pos2(0.,0.),pos2(1.,1.)),Color32::WHITE);}
                        if let Some(s)=&t.session { for a in s.annotations.iter().filter(|a|a.page==page) { editing::paint_annotation(&mut self.editor,ctx,ui.painter(),r,scale,a); } }
                        for a in copies.iter().filter(|a|a.page==page){editing::paint_annotation(&mut self.editor,ctx,ui.painter(),r,scale,a);}
                        ui.label(format!("Page {}",page+1));});
                }});}
            });
            ui.weak("Preview shows the first 12 destination pages. Smaller pages fit the artwork proportionally.");
            if !fits { ui.colored_label(theme::danger(), "Choose fewer pages: the document limit is 5,000 objects."); }
            if !valid{ui.weak("The document changed. Reopen this dialog.");}
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                done=ui.add_enabled(valid&&!copies.is_empty(),ActionButton::new(Icon::Copy,&format!("Add {} {}",copies.len(),if copies.len()==1 { "copy" } else { "copies" })).primary()).clicked();
                cancel=ui.button("Cancel").clicked();
            });
        });
        if done {
            let s = self.tabs[self.active].session.as_mut().unwrap();
            let mut entries = s.annotations.clone();
            entries.extend(copies);
            match s.commit(entries) {
                Ok(()) => {
                    s.name_last_change("Repeat artwork on pages");
                    self.status = Some(("Artwork repeated · Undo removes all copies".into(), false))
                }
                Err(e) => self.status = Some((e.to_string(), true)),
            }
            self.workbench.repeat = None;
        } else if cancel || modal.should_close() {
            self.workbench.repeat = None;
        }
    }
    fn history_window(&mut self, ctx: &egui::Context) {
        if !self.workbench.history {
            return;
        }
        let mut open = true;
        let mut target = None;
        let text_open = self.content.pending_text().is_some();
        egui::Window::new("Document history")
            .open(&mut open)
            .default_width(300.)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.weak("Choose a state to undo or redo. New edits replace later history.");
                if text_open {
                    ui.weak("Finish the on-page text edit before changing document history.");
                }
                if let Some(s) = self.tabs.get(self.active).and_then(|t| t.session.as_ref()) {
                    let (cursor, labels) = s.history();
                    egui::ScrollArea::vertical()
                        .max_height(360.)
                        .show(ui, |ui| {
                            for (i, label) in labels.iter().enumerate() {
                                if ui
                                    .add_enabled(
                                        !text_open,
                                        egui::Button::new(format!("{i:02}  {label}"))
                                            .selected(i == cursor),
                                    )
                                    .clicked()
                                {
                                    target = Some(i);
                                }
                            }
                        });
                }
            });
        self.workbench.history = open;
        if let Some(target) = target
            && let Some(t) = self.tabs.get_mut(self.active)
            && let Some(s) = &mut t.session
        {
            s.seek_history(target);
            selection::set(t, &mut self.editor, vec![]);
        }
    }
    fn palette_window(&mut self, ctx: &egui::Context) {
        if !self.workbench.palette {
            return;
        }
        let mut action = None;
        let modal = egui::Modal::new(egui::Id::new("find_a_tool"))
            .frame(ui::modal_frame())
            .show(ctx, |ui| {
                ui.set_width(460.);
                ui.heading("Find a tool");
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.workbench.query)
                        .margin(ui::FIELD_MARGIN)
                        .hint_text("Try crop, transparent, merge, signature…")
                        .desired_width(f32::INFINITY),
                );
                if self.workbench.focus_query {
                    r.request_focus();
                    self.workbench.focus_query = false;
                }
                if r.changed() {
                    self.workbench.choice = 0;
                }
                let matches = commands(&self.workbench.query);
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)) {
                    self.workbench.choice =
                        (self.workbench.choice + 1).min(matches.len().saturating_sub(1));
                }
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)) {
                    self.workbench.choice = self.workbench.choice.saturating_sub(1);
                }
                if matches.is_empty() {
                    ui.weak("No matching tool. Try a shorter name.");
                }
                egui::ScrollArea::vertical()
                    .max_height(340.)
                    .show(ui, |ui| {
                        for (row, &i) in matches.iter().enumerate() {
                            let enabled = matches!(
                                COMMANDS[i].1,
                                "new"
                                    | "open"
                                    | "guides"
                                    | "history"
                                    | "recovery"
                                    | "workspace"
                                    | "reopen_closed"
                            ) || !self.home.visible;
                            if ui
                                .add_enabled(
                                    enabled,
                                    egui::Button::new(COMMANDS[i].0)
                                        .selected(row == self.workbench.choice)
                                        .min_size(vec2(440., 32.)),
                                )
                                .clicked()
                            {
                                action = Some(COMMANDS[i].1);
                            }
                        }
                    });
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
                    && let Some(&i) = matches.get(self.workbench.choice)
                {
                    action = Some(COMMANDS[i].1);
                }
                ui.weak("↑ ↓ to choose · Return to open · Escape to close");
            });
        if let Some(action) = action {
            self.workbench.palette = false;
            self.action(action, ctx);
        } else if modal.should_close() {
            self.workbench.palette = false;
        }
    }
    pub(super) fn objects_panel(&mut self, ui: &mut egui::Ui) {
        let Some(t) = self.tabs.get_mut(self.active) else {
            return;
        };
        let Some(s) = &t.session else { return };
        let mut entries = s.annotations.clone();
        let editable = s.editable;
        let before = entries.clone();
        let mut select = None;
        let mut reorder = None;
        let mut naming = false;
        if entries.is_empty() {
            ui.weak("Drawings, stamps and image layers appear here.");
            return;
        }
        ui.weak("Objects · topmost first");
        egui::ScrollArea::vertical()
            .id_salt("objects_list")
            .show(ui, |ui| {
                for a in entries.iter_mut().rev() {
                    let selected =
                        t.selected_annotation.as_ref() == Some(&a.id) || t.group.contains(&a.id);
                    ui.push_id(a.id.clone(), |ui| {
                        egui::Frame::new()
                            .fill(if selected {
                                theme::hover()
                            } else {
                                Color32::TRANSPARENT
                            })
                            .corner_radius(8)
                            .inner_margin(6.)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let (r, response) = ui
                                        .allocate_exact_size(vec2(38., 38.), egui::Sense::click());
                                    ui.painter().rect_filled(r, 4., Color32::WHITE);
                                    let mut thumb = a.clone();
                                    thumb.object.hidden = false;
                                    let b = thumb.bounds();
                                    let scale = (30. / (b[2] - b[0]).max(1.))
                                        .min(30. / (b[3] - b[1]).max(1.));
                                    let origin = r.center()
                                        - vec2((b[0] + b[2]) / 2., (b[1] + b[3]) / 2.) * scale;
                                    editing::paint_annotation(
                                        &mut self.editor,
                                        ui.ctx(),
                                        &ui.painter().with_clip_rect(r),
                                        Rect::from_min_size(origin, vec2(1., 1.)),
                                        scale,
                                        &thumb,
                                    );
                                    if response.clicked() {
                                        select = Some(a.clone());
                                    }
                                    let label = if a.object.name.is_empty() {
                                        a.kind.label()
                                    } else {
                                        &a.object.name
                                    };
                                    if ui
                                        .selectable_label(
                                            selected,
                                            format!("{}\nPage {}", label, a.page + 1),
                                        )
                                        .clicked()
                                    {
                                        select = Some(a.clone());
                                    }
                                });
                                if selected {
                                    ui.add_enabled_ui(editable, |ui| {
                                        let r = ui.add(
                                            egui::TextEdit::singleline(&mut a.object.name)
                                                .hint_text("Object name")
                                                .char_limit(200)
                                                .desired_width(f32::INFINITY),
                                        );
                                        self.editor.text_editing |= r.has_focus();
                                        naming |= r.has_focus();
                                        ui.horizontal(|ui| {
                                            let mut visible = !a.object.hidden;
                                            if ui.checkbox(&mut visible, "Visible").changed() {
                                                a.object.hidden = !visible;
                                            }
                                            ui.checkbox(&mut a.object.locked, "Lock");
                                        });
                                        ui.horizontal(|ui| {
                                            if ui
                                                .add(
                                                    ActionButton::new(
                                                        Icon::BringForward,
                                                        "Bring forward",
                                                    )
                                                    .compact(),
                                                )
                                                .clicked()
                                            {
                                                reorder = Some((a.id.clone(), true));
                                            }
                                            if ui
                                                .add(
                                                    ActionButton::new(
                                                        Icon::SendBackward,
                                                        "Send backward",
                                                    )
                                                    .compact(),
                                                )
                                                .clicked()
                                            {
                                                reorder = Some((a.id.clone(), false));
                                            }
                                            if ui
                                                .add(
                                                    ActionButton::new(
                                                        Icon::Copy,
                                                        "Repeat on pages",
                                                    )
                                                    .compact(),
                                                )
                                                .clicked()
                                            {
                                                self.workbench.request =
                                                    Some("repeat_objects".into());
                                            }
                                        });
                                    });
                                }
                            });
                    });
                    ui.add_space(4.);
                }
            });
        if let Some((id, forward)) = reorder
            && let Some(i) = entries.iter().position(|a| a.id == id)
        {
            // Z order is page-local even when the document contains several pages.
            let page = entries[i].page;
            let other = if forward {
                (i + 1..entries.len()).find(|&j| entries[j].page == page)
            } else {
                (0..i).rev().find(|&j| entries[j].page == page)
            };
            if let Some(j) = other {
                entries.swap(i, j);
            }
        }
        if entries != before {
            let ids = selection::selected_ids(t);
            let previous = self
                .workbench
                .rename_revision
                .filter(|(id, _)| *id == t.id && naming)
                .map(|(_, rev)| rev);
            match t.session.as_mut().unwrap().commit_live(entries, previous) {
                Ok(()) => {
                    self.workbench.rename_revision =
                        naming.then_some((t.id, t.session.as_ref().unwrap().revision()));
                    selection::set(t, &mut self.editor, ids)
                }
                Err(e) => self.status = Some((e.to_string(), true)),
            }
        }
        if !naming {
            self.workbench.rename_revision = None;
        }
        if let Some(a) = select {
            t.jump(a.page);
            selection::set(t, &mut self.editor, vec![a.id]);
            self.inspector = true;
        }
    }
    pub(super) fn place_pending_original(&mut self) {
        let Some(marks) = self.workbench.paste.take() else {
            return;
        };
        let Some(t) = self.tabs.get_mut(self.active) else {
            return;
        };
        let Some(size) = t.pages.get(t.page).copied() else {
            return;
        };
        let mut marks = selection::moved(&marks, [0., 0.], size);
        for a in &mut marks {
            a.page = t.page;
        }
        place(&mut self.editor, t, marks, &mut self.status);
    }
}
/// Resize one corner around the opposite anchor, optionally retaining aspect.
fn crop_corner(r: [f32; 4], corner: usize, p: [f32; 2], ratio: Option<f32>) -> [f32; 4] {
    let left = matches!(corner, 0 | 3);
    let top = corner < 2;
    let anchor = [
        if left { r[2] } else { r[0] },
        if top { r[3] } else { r[1] },
    ];
    let max = [
        if left { anchor[0] } else { 1. - anchor[0] },
        if top { anchor[1] } else { 1. - anchor[1] },
    ];
    let mut w = if left {
        anchor[0] - p[0]
    } else {
        p[0] - anchor[0]
    };
    let mut h = if top {
        anchor[1] - p[1]
    } else {
        p[1] - anchor[1]
    };
    const MIN: f32 = 0.00101;
    if max[0] < MIN || max[1] < MIN {
        return r;
    }
    w = w.clamp(MIN, max[0]);
    h = h.clamp(MIN, max[1]);
    if let Some(ratio) = ratio {
        let lower = MIN.max(MIN * ratio);
        let upper = max[0].min(max[1] * ratio);
        if lower > upper {
            return r;
        }
        w = w.min(h * ratio).clamp(lower, upper);
        h = w / ratio;
    }
    [
        if left { anchor[0] - w } else { anchor[0] },
        if top { anchor[1] - h } else { anchor[1] },
        if left { anchor[0] } else { anchor[0] + w },
        if top { anchor[1] } else { anchor[1] + h },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Viewer, egui::Context) {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.open_memory(
            PathBuf::from("Workflow test.pdf"),
            mypdf_milestone0::convert::blank(400., 400.).unwrap(),
            &ctx,
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        app.home.visible = false;
        app.editor.mode = editing::Mode::Draw;
        (app, ctx)
    }
    #[test]
    fn canvas_crop_rotated_flipped_image_commits_once_and_undo_restores_pixels() {
        let (mut app, ctx) = fixture();
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(4, 3, image::Rgba([30, 120, 190, 255]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let mut original = Annotation::new(
            0,
            [100., 100., 300., 250.],
            Kind::Image {
                png: Arc::new(png.into_inner()),
            },
            Default::default(),
        );
        original.rotation = 30.;
        original.flip_x = true;
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![original.clone()])
            .unwrap();
        selection::set(&mut app.tabs[0], &mut app.editor, vec![original.id.clone()]);
        app.start_crop(&ctx);
        let screen = Rect::from_min_size(pos2(0., 0.), vec2(400., 400.));
        let frame = |app: &mut Viewer, events| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events,
                    max_texture_side: Some(8192),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        crop_overlay(
                            &mut app.workbench,
                            &mut app.editor,
                            &app.tabs[0],
                            ui,
                            0,
                            screen,
                            1.,
                        );
                    });
                },
            );
        };
        frame(&mut app, vec![]);
        frame(&mut app, vec![]);
        let mapped = |local| {
            let p = original.rotated(original.local(local));
            pos2(p[0], p[1])
        };
        let from = mapped([0., 0.]);
        let to = mapped([0.2, 0.1]);
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(from),
                egui::Event::PointerButton {
                    pos: from,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        frame(&mut app, vec![egui::Event::PointerMoved(to)]);
        frame(
            &mut app,
            vec![egui::Event::PointerButton {
                pos: to,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
        let crop = app.workbench.crop.as_ref().unwrap().rect;
        assert!(
            (crop[0] - 0.2).abs() < 0.01 && (crop[1] - 0.1).abs() < 0.01,
            "{crop:?}"
        );
        assert_eq!(
            app.tabs[0].session.as_ref().unwrap().annotations[0],
            original
        );
        let revision = app.tabs[0].session.as_ref().unwrap().revision();
        app.workbench.crop.as_mut().unwrap().apply = true;
        let _ = ctx.run(Default::default(), |ctx| app.crop_window(ctx));
        let session = app.tabs[0].session.as_mut().unwrap();
        assert_eq!(session.revision(), revision + 1);
        let edited = &session.annotations[0];
        assert_eq!(edited.rotation, original.rotation);
        assert_eq!(edited.flip_x, original.flip_x);
        assert_eq!(edited.kind, original.kind);
        assert_eq!(edited.object.crop, Some(crop));
        let saved = session.snapshot_bytes().unwrap();
        assert_eq!(
            mypdf_milestone0::annotations::load(&lopdf::Document::load_mem(&saved).unwrap()),
            session.annotations
        );
        session.undo();
        assert_eq!(session.annotations, vec![original]);
    }
    #[test]
    fn paste_is_a_draft_until_click_and_escape_leaves_document_unchanged() {
        let (mut app, ctx) = fixture();
        let mark = Annotation::new(0, [20., 30., 80., 90.], Kind::Rectangle, Default::default());
        app.copy_artwork(&ctx, vec![mark]);
        app.paste(&ctx);
        assert!(app.workbench.paste.is_some());
        assert!(app.tabs[0].session.as_ref().unwrap().annotations.is_empty());
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            app.workbench_keys(ctx);
        });
        assert!(app.workbench.paste.is_none());
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), 0);
        app.paste(&ctx);
        let r = Rect::from_min_size(pos2(0., 0.), vec2(400., 400.));
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(r),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    paste_overlay(
                        &mut app.workbench,
                        &mut app.editor,
                        &mut app.tabs[0],
                        ui,
                        0,
                        r,
                        1.,
                        &mut app.status,
                    );
                });
            },
        );
        for pressed in [true, false] {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(r),
                    events: vec![
                        egui::Event::PointerMoved(pos2(160., 170.)),
                        egui::Event::PointerButton {
                            pos: pos2(160., 170.),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        paste_overlay(
                            &mut app.workbench,
                            &mut app.editor,
                            &mut app.tabs[0],
                            ui,
                            0,
                            r,
                            1.,
                            &mut app.status,
                        );
                    });
                },
            );
        }
        assert!(app.workbench.paste.is_none());
        let s = app.tabs[0].session.as_mut().unwrap();
        assert_eq!(s.annotations.len(), 1);
        assert_eq!(s.annotations[0].rect, [160., 170., 220., 230.]);
        s.undo();
        assert!(s.annotations.is_empty());
    }
    #[test]
    fn crop_corners_preserve_ratio_and_stay_inside() {
        for i in 0..4 {
            for p in [[0., 0.], [1., 1.], [0.4, 0.2]] {
                let r = crop_corner([0.2, 0.2, 0.8, 0.8], i, p, Some(1.));
                object_tools::validate_crop(r).unwrap();
                assert!(((r[2] - r[0]) / (r[3] - r[1]) - 1.).abs() < 0.001);
            }
        }
    }
    #[test]
    fn tool_search_finds_aliases_and_context_actions_respect_locks() {
        for (q, expected) in [
            ("transparent", "object_background"),
            ("merge", "merge_files"),
            ("signature", "sign"),
            ("crop", "crop_object"),
        ] {
            assert!(commands(q).iter().any(|&i| COMMANDS[i].1 == expected));
        }
        let (mut app, ctx) = fixture();
        let mut a = Annotation::new(0, [10., 10., 60., 60.], Kind::Rectangle, Default::default());
        a.object.locked = true;
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![a.clone()])
            .unwrap();
        selection::set(&mut app.tabs[0], &mut app.editor, vec![a.id.clone()]);
        app.workbench_action("object_rotate", &ctx);
        app.workbench_action("object_delete", &ctx);
        assert!(!app.copy_marks(&ctx, true));
        assert_eq!(app.tabs[0].session.as_ref().unwrap().annotations, vec![a]);
    }
    #[test]
    fn command_palette_search_and_return_open_the_tool() {
        let (mut app, ctx) = fixture();
        app.workbench_action("tool_search", &ctx);
        let screen = Rect::from_min_size(pos2(0., 0.), vec2(1200., 820.));
        for events in [
            vec![],
            vec![egui::Event::Text("transparent".into())],
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        ] {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events,
                    ..Default::default()
                },
                |ctx| app.palette_window(ctx),
            );
        }
        assert_eq!(app.workbench.query, "transparent");
        assert!(
            !app.workbench.palette,
            "Return should execute the selected tool"
        );
        assert!(
            app.status
                .as_ref()
                .unwrap()
                .0
                .contains("Select an unlocked image")
        );
    }
    #[test]
    fn object_commands_preserve_unfinished_on_page_text() {
        let (mut app, ctx) = fixture();
        let tab = &app.tabs[0];
        app.content
            .paste_new_text(tab.id, 0, tab.pages[0], "Words still being edited".into());
        let id = tab.id;
        app.editor.mode = editing::Mode::Edit;
        app.workbench_action("objects", &ctx);
        assert_eq!(app.content.pending_text(), Some(id));
        assert_eq!(app.editor.mode, editing::Mode::Edit);
        assert!(app.workbench.paste.is_none());
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), 0);
    }
}
