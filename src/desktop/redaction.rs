//! Edit → Redact: mark areas (or every match of a search), then remove what is
//! under them for good. Also Flatten PDF, which turns every page into an image.
use super::*;
use mypdf_milestone0::redact::{Area, Method, Options};

const QUALITY: [(&str, f32); 3] = [("Standard", 150.), ("High", 200.), ("Print", 300.)];

pub(super) struct Redaction {
    pub active: bool,
    /// Marked areas: tab, page, displayed page rectangle.
    marks: Vec<(u64, usize, [f32; 4])>,
    drag: Option<(u64, usize, egui::Pos2)>,
    query: String,
    flatten_pages: bool,
    color: [f32; 3],
    confirm: Option<u64>,
    /// Tab and revision when redactions were applied, until the change lands.
    applied: Option<(u64, u64)>,
    flatten: Option<u64>,
    quality: usize,
}
impl Redaction {
    pub fn new() -> Self {
        Self {
            active: false,
            marks: vec![],
            drag: None,
            query: String::new(),
            flatten_pages: false,
            color: [0.; 3],
            confirm: None,
            applied: None,
            flatten: None,
            quality: 0,
        }
    }
    fn count(&self, tab: u64) -> usize {
        self.marks.iter().filter(|m| m.0 == tab).count()
    }
}

/// Drag to mark an area; click a marked area to unmark it.
pub(super) fn overlay(
    state: &mut Redaction,
    tab: &Tab,
    ui: &mut egui::Ui,
    page: usize,
    page_rect: Rect,
    scale: f32,
) {
    let response = ui.interact(
        page_rect,
        egui::Id::new((tab.id, page, "redact_page")),
        egui::Sense::click_and_drag(),
    );
    let size = tab.pages[page];
    let normal = |p: egui::Pos2| {
        let p = (p - page_rect.min) / scale;
        [p.x.clamp(0., size.width), p.y.clamp(0., size.height)]
    };
    let screen = |r: [f32; 4]| {
        Rect::from_min_max(
            page_rect.min + vec2(r[0], r[1]) * scale,
            page_rect.min + vec2(r[2], r[3]) * scale,
        )
    };
    let painter = ui
        .painter()
        .with_clip_rect(page_rect.intersect(ui.clip_rect()));
    let hovered_mark = response.hover_pos().and_then(|pos| {
        let p = normal(pos);
        state.marks.iter().rposition(|m| {
            m.0 == tab.id
                && m.1 == page
                && (m.2[0]..=m.2[2]).contains(&p[0])
                && (m.2[1]..=m.2[3]).contains(&p[1])
        })
    });
    if response.hovered() {
        ui.ctx().set_cursor_icon(if hovered_mark.is_some() {
            egui::CursorIcon::PointingHand
        } else {
            egui::CursorIcon::Crosshair
        });
    }
    if response.drag_started()
        && let Some(origin) = ui.input(|i| i.pointer.press_origin())
    {
        state.drag = Some((tab.id, page, origin));
    }
    if let Some((t, p, start)) = state.drag
        && (t, p) == (tab.id, page)
        && let Some(end) = ui.input(|i| i.pointer.interact_pos())
    {
        let r = Rect::from_two_pos(start, end).intersect(page_rect);
        painter.rect_filled(r, 0., theme::danger().gamma_multiply(0.12));
        ui::dashed_rect(&painter, r, theme::danger());
        if response.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
            let (a, b) = (normal(r.min), normal(r.max));
            if b[0] - a[0] >= 2. && b[1] - a[1] >= 2. {
                state.marks.push((tab.id, page, [a[0], a[1], b[0], b[1]]));
            }
            state.drag = None;
        }
    }
    if response.clicked()
        && let Some(i) = hovered_mark
    {
        state.marks.remove(i);
    } else if let Some(i) = hovered_mark {
        response.on_hover_text_at_pointer("Marked for redaction · click to unmark");
        let _ = i;
    }
    for m in state.marks.iter().filter(|m| m.0 == tab.id && m.1 == page) {
        let r = screen(m.2);
        painter.rect_filled(r, 0., theme::danger().gamma_multiply(0.16));
        // Diagonal hatching, like a redaction mark in other editors.
        let hatch = painter.with_clip_rect(r.intersect(painter.clip_rect()));
        let mut x = r.left() - r.height();
        while x < r.right() {
            hatch.line_segment(
                [pos2(x, r.bottom()), pos2(x + r.height(), r.top())],
                Stroke::new(1., theme::danger().gamma_multiply(0.45)),
            );
            x += 7.;
        }
        painter.rect_stroke(
            r,
            0.,
            Stroke::new(1.5, theme::danger()),
            egui::StrokeKind::Middle,
        );
    }
}

impl Viewer {
    pub(super) fn start_redaction(&mut self) {
        self.editor.mode = editing::Mode::Edit;
        self.content.add_target = None;
        self.content.image_mode = false;
        self.content.area_mode = false;
        self.redaction.active = true;
        self.inspector = true;
        self.status = None;
    }

    /// Mark every occurrence of the search text on the active document.
    fn mark_matches(&mut self) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let mut found = 0;
        for (page, text) in tab.text.iter().enumerate() {
            let Some(text) = text else { continue };
            for hit in text.find(&self.redaction.query, page) {
                // One mark per line of the match (in any direction), not one per glyph:
                // consecutive glyphs join while each sits next to the one before.
                let mut lines: Vec<[f32; 4]> = vec![];
                let mut previous: Option<[f32; 4]> = None;
                for r in hit.rects {
                    let near = previous.is_some_and(|p| {
                        let size = (p[2] - p[0])
                            .max(p[3] - p[1])
                            .max(r[2] - r[0])
                            .max(r[3] - r[1]);
                        let (pc, rc) = (
                            [(p[0] + p[2]) / 2., (p[1] + p[3]) / 2.],
                            [(r[0] + r[2]) / 2., (r[1] + r[3]) / 2.],
                        );
                        (pc[0] - rc[0]).hypot(pc[1] - rc[1]) <= size * 1.6
                    });
                    match lines.last_mut().filter(|_| near) {
                        Some(l) => {
                            *l = [
                                l[0].min(r[0]),
                                l[1].min(r[1]),
                                l[2].max(r[2]),
                                l[3].max(r[3]),
                            ]
                        }
                        None => lines.push(r),
                    }
                    previous = Some(r);
                }
                for r in lines {
                    let r = [r[0] - 1., r[1] - 1., r[2] + 1., r[3] + 1.];
                    if !self.redaction.marks.contains(&(tab.id, page, r)) {
                        self.redaction.marks.push((tab.id, page, r));
                        found += 1;
                    }
                }
            }
        }
        let indexed = tab.text.iter().filter(|t| t.is_some()).count();
        self.status = Some((
            if found == 0 {
                format!("No matches for “{}”", self.redaction.query)
            } else {
                format!("Marked {found} match{}", if found == 1 { "" } else { "es" })
            } + &if indexed < tab.pages.len() {
                format!(" · {indexed} of {} pages searched so far", tab.pages.len())
            } else {
                String::new()
            },
            false,
        ));
    }

    pub(super) fn redaction_panel(&mut self, ctx: &egui::Context) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let id = tab.id;
        let signed = tab.signed || tab.signed_original.is_some();
        let marked = self.redaction.count(id);
        let pages: std::collections::BTreeSet<usize> = self
            .redaction
            .marks
            .iter()
            .filter(|m| m.0 == id)
            .map(|m| m.1)
            .collect();
        let busy = self.content.busy;
        let mut search = false;
        let mut apply = false;
        let mut clear = false;
        let mut close = false;
        egui::SidePanel::right("redaction_panel")
            .exact_width(310.)
            .resizable(false)
            .frame(egui::Frame::new().fill(theme::paper()).inner_margin(18.))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().id_salt("redaction_panel").show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 10.;
                ui.horizontal(|ui| {
                    ui::title(ui, "Redact", 17.);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = ui
                            .add(ActionButton::new(Icon::Close, "Stop redacting").compact())
                            .clicked();
                    });
                });
                ui.label(
                    RichText::new("Drag over text, images or anything else to mark it. Click a mark to unmark it.")
                        .size(12.5)
                        .color(theme::muted()),
                );
                if signed {
                    ui::callout(
                        ui,
                        ui::Tone::Warning,
                        "Signed PDFs can't be redacted: the signed version inside the file would still hold the original. Redact an unsigned copy, then sign it.",
                    );
                }
                ui::section(ui, "Find and mark", |ui| {
                    ui.horizontal(|ui| {
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut self.redaction.query)
                                .hint_text("Name, number, email…")
                                .margin(ui::FIELD_MARGIN)
                                .desired_width(ui.available_width() - 120.),
                        );
                        self.editor.text_editing |= r.has_focus();
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        search = (ui
                            .add_enabled(
                                !self.redaction.query.trim().is_empty(),
                                ActionButton::new(Icon::Search, "Mark all"),
                            )
                            .clicked()
                            || enter)
                            && !self.redaction.query.trim().is_empty();
                    });
                });
                ui::section(ui, "Marked", |ui| {
                    ui.label(
                        RichText::new(match marked {
                            0 => "Nothing marked yet".to_owned(),
                            n => format!(
                                "{n} area{} on {} page{}",
                                if n == 1 { "" } else { "s" },
                                pages.len(),
                                if pages.len() == 1 { "" } else { "s" }
                            ),
                        })
                        .color(theme::ink()),
                    );
                    if marked > 0 && ui.small_button("Unmark all").clicked() {
                        clear = true;
                    }
                });
                ui::section(ui, "Box color", |ui| {
                    ui.horizontal(|ui| {
                        let choice = if self.redaction.color == [0.; 3] { 0 } else if self.redaction.color == [1.; 3] { 1 } else { 2 };
                        let width = (ui.available_width() - 30. - ui.spacing().item_spacing.x).max(0.);
                        ui.allocate_ui_with_layout(vec2(width, 30.), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            if let Some(c) = ui::segmented(ui, &["Black", "White"], choice) {
                                self.redaction.color = [c as f32; 3];
                            }
                        });
                        ui::color_swatch_sized(ui, &mut self.redaction.color, "Redaction color", vec2(30., 30.));
                    });
                });
                ui::section(ui, "Method", |ui| {
                    ui.radio_value(&mut self.redaction.flatten_pages, false, "Remove the marked content");
                    ui.label(
                        RichText::new("Text, image pixels and annotations under the boxes are deleted; the rest of the page stays editable and searchable.")
                            .size(11.5)
                            .color(theme::muted()),
                    );
                    ui.radio_value(&mut self.redaction.flatten_pages, true, "Flatten marked pages to images");
                    ui.label(
                        RichText::new("Recommended for sensitive vector artwork or complex pages. Rebuilds marked pages from visible pixels; those pages lose selectable text and editable objects.")
                            .size(11.5)
                            .color(theme::muted()),
                    );
                });
                apply = ui
                    .add_enabled(
                        marked > 0 && !signed && !busy,
                        ActionButton::new(Icon::Trash, "Apply Redactions…").primary(),
                    )
                    .clicked();
                if busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Redacting…");
                    });
                }
                ui::callout(ui, ui::Tone::Warning, if self.redaction.flatten_pages {
                    "Only marked pages are flattened. Metadata, bookmarks and other pages are not sanitized; review the saved copy before sharing."
                } else {
                    "Vector drawings can remain under the cover. For sensitive artwork, choose Flatten marked pages to images. Metadata and bookmarks are not sanitized."
                });
                });
            });
        if search {
            self.mark_matches();
        }
        if clear {
            self.redaction.marks.retain(|m| m.0 != id);
        }
        if apply {
            self.redaction.confirm = Some(id);
        }
        if close {
            self.redaction.active = false;
        }
    }

    /// Confirmations, and tidying up once a redaction has landed.
    pub(super) fn redaction_dialogs(&mut self, ctx: &egui::Context) {
        if let Some((id, revision)) = self.redaction.applied
            && !self.content.busy
        {
            self.redaction.applied = None;
            let landed = self
                .tabs
                .iter()
                .find(|t| t.id == id)
                .and_then(|t| t.session.as_ref())
                .is_some_and(|s| s.revision() != revision);
            if landed {
                self.redaction.marks.retain(|m| m.0 != id);
            }
        }
        if let Some(id) = self.redaction.confirm {
            let n = self.redaction.count(id);
            if let Some(choice) = ui::confirmation(
                ctx,
                "apply_redactions",
                &format!("Apply {n} redaction{}?", if n == 1 { "" } else { "s" }),
                "What's under the marks is removed for good when you save. Until then, Undo brings it back.",
                &[
                    ("Cancel", ui::DialogTone::Secondary),
                    ("Redact", ui::DialogTone::Destructive),
                ],
                0,
            ) {
                self.redaction.confirm = None;
                if choice == 1 {
                    self.apply_redactions(ctx, id);
                }
            }
        }
        let Some(id) = self.redaction.flatten else {
            return;
        };
        let mut decision = None;
        let modal = egui::Modal::new(egui::Id::new("flatten_pdf"))
            .backdrop_color(Color32::from_black_alpha(64))
            .frame(ui::modal_frame())
            .show(ctx, |ui| {
                ui.set_width(400.);
                ui.spacing_mut().item_spacing.y = 10.;
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(40., 40.), egui::Sense::hover());
                    ui::badge(ui.painter(), r, Icon::Document, ui::Fill::Brand, Color32::WHITE);
                    ui.add_space(4.);
                    ui::title(ui, "Flatten this PDF?", 18.);
                });
                ui.label(
                    RichText::new("Every page becomes an image. Annotations, pasted images and form fields are burned in, and nothing on the pages can be edited or selected any more. You can still sign it digitally afterwards.")
                        .size(13.)
                        .color(theme::ink()),
                );
                ui::caption(ui, "Quality");
                let labels: Vec<String> = QUALITY
                    .iter()
                    .map(|(label, dpi)| format!("{label} · {dpi:.0} dpi"))
                    .collect();
                let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
                if let Some(q) = ui::segmented(ui, &labels, self.redaction.quality) {
                    self.redaction.quality = q;
                }
                ui.label(
                    RichText::new("Higher quality keeps small print sharp and makes a larger file. Undo is available until you save.")
                        .size(11.5)
                        .color(theme::muted()),
                );
                ui.add_space(4.);
                ui.columns(2, |columns| {
                    if ui::dialog_button(&mut columns[0], "Cancel", ui::DialogTone::Secondary).clicked() {
                        decision = Some(false);
                    }
                    if ui::dialog_button(&mut columns[1], "Flatten", ui::DialogTone::Primary).clicked() {
                        decision = Some(true);
                    }
                });
            });
        if modal.should_close() {
            decision.get_or_insert(false);
        }
        if let Some(go) = decision {
            self.redaction.flatten = None;
            if go {
                self.apply_flatten(ctx, id);
            }
        }
    }

    /// Remove what is under the document's marks.
    fn apply_redactions(&mut self, ctx: &egui::Context, id: u64) {
        let Some(index) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        let Some(revision) = self.tabs[index].session.as_ref().map(|s| s.revision()) else {
            return;
        };
        let areas = self
            .redaction
            .marks
            .iter()
            .filter(|m| m.0 == id)
            .map(|m| Area {
                page: m.1,
                rect: m.2,
            })
            .collect();
        let options = Options {
            method: if self.redaction.flatten_pages {
                Method::Flatten
            } else {
                Method::Precise
            },
            fill: self.redaction.color,
        };
        self.active = index;
        self.redaction.applied = Some((id, revision));
        self.content_change(ctx, content::Edit::Redact { areas, options });
    }

    fn apply_flatten(&mut self, ctx: &egui::Context, id: u64) {
        if let Some(index) = self.tabs.iter().position(|t| t.id == id) {
            self.active = index;
            self.content_change(
                ctx,
                content::Edit::Flatten {
                    dpi: QUALITY[self.redaction.quality].1,
                },
            );
        }
    }

    #[cfg(test)]
    pub(super) fn preview_redaction(&mut self) {
        let id = self.tabs[0].id;
        self.redaction.query = "1250.00".into();
        self.mark_matches();
        self.redaction.marks.push((id, 0, [60., 250., 200., 330.]));
    }

    pub(super) fn request_flatten(&mut self) {
        let Some(tab) = self.tabs.get(self.active).filter(|t| !t.pages.is_empty()) else {
            self.status = Some(("Open a PDF first.".into(), false));
            return;
        };
        if tab.signed || tab.signed_original.is_some() {
            self.status = Some((
                "Flattening would break this PDF's signatures. Flatten an unsigned copy, then sign it.".into(),
                true,
            ));
            return;
        }
        if !tab.session.as_ref().is_some_and(|s| s.editable) {
            self.status = Some(("This PDF is read-only.".into(), true));
            return;
        }
        self.redaction.flatten = Some(tab.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;

    #[test]
    fn marked_words_and_areas_are_redacted_and_documents_flatten() {
        let temp = tempfile::tempdir().unwrap();
        let pdf = temp.path().join("Memo.pdf");
        let bytes = mypdf_milestone0::content::add_text(
            &mypdf_milestone0::convert::blank(595., 842.).unwrap(),
            1,
            "Name: Orhan Gökçöl\nCity: İstanbul",
            [60., 100., 480., 80.],
            16.,
            [0.; 3],
        )
        .unwrap();
        std::fs::write(&pdf, bytes).unwrap();
        let ctx = egui::Context::default();
        let mut app = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            vec![pdf],
        );
        app.tabs[0].zoom = Zoom::Page;
        let frame = |app: &mut Viewer, events: Vec<egui::Event>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 900.))),
                    events,
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
        };
        let until = |app: &mut Viewer, ready: &dyn Fn(&Viewer) -> bool| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while !ready(app) {
                assert!(std::time::Instant::now() < deadline, "{:?}", app.status);
                frame(app, vec![]);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        let editable =
            |a: &Viewer| a.tabs[0].session.as_ref().is_some_and(|s| s.editable) && !a.content.busy;
        until(&mut app, &|a| {
            editable(a) && a.tabs[0].text.first().is_some_and(Option::is_some)
        });
        let id = app.tabs[0].id;
        let text = |app: &Viewer| -> String {
            mypdf_milestone0::document::text_runs(&app.tabs[0].session.as_ref().unwrap().source, 1)
                .unwrap()
                .into_iter()
                .map(|r| r.text)
                .collect()
        };

        // Search marks every match; a drag marks an area; a click unmarks it.
        app.start_redaction();
        assert!(app.redaction.active && app.editor.mode == editing::Mode::Edit);
        app.redaction.query = "Gökçöl".into();
        app.mark_matches();
        assert_eq!(app.redaction.count(id), 1);
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        let page = ctx
            .read_response(egui::Id::new((id, 0_usize, "redact_page")))
            .expect("the redaction overlay is active")
            .rect;
        let scale = app.tabs[0].scale;
        let at = |x: f32, y: f32| page.min + vec2(x, y) * scale;
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let (a, b) = (at(300., 400.), at(400., 460.));
        frame(
            &mut app,
            vec![egui::Event::PointerMoved(a), button(a, true)],
        );
        frame(
            &mut app,
            vec![egui::Event::PointerMoved(a + vec2(30., 20.))],
        );
        frame(&mut app, vec![egui::Event::PointerMoved(b)]);
        frame(&mut app, vec![button(b, false)]);
        assert_eq!(app.redaction.count(id), 2);
        let inside = at(350., 430.);
        frame(&mut app, vec![egui::Event::PointerMoved(inside)]);
        frame(&mut app, vec![button(inside, true)]);
        frame(&mut app, vec![button(inside, false)]);
        assert_eq!(app.redaction.count(id), 1, "a click unmarks");

        app.redaction.color = [0.2, 0.4, 0.6];
        app.apply_redactions(&ctx, id);
        until(&mut app, &|a| editable(a) && a.redaction.applied.is_none());
        assert_eq!(app.redaction.count(id), 0, "marks clear once applied");
        let after = text(&app);
        assert!(
            !after.contains("Gökçöl") && after.contains("City"),
            "{after}"
        );
        assert!(
            app.status
                .as_ref()
                .is_some_and(|(s, _)| s.contains("Redacted 6 characters"))
        );

        // Flatten: nothing selectable remains; Undo brings the text back.
        app.request_flatten();
        assert_eq!(app.redaction.flatten, Some(id));
        frame(&mut app, vec![]);
        app.redaction.flatten = None;
        app.apply_flatten(&ctx, id);
        until(&mut app, &|a| editable(a));
        assert_eq!(text(&app), "", "{:?}", app.status);
        app.editor_undo(false);
        until(&mut app, &|a| editable(a));
        assert!(text(&app).contains("City"));
    }
}
