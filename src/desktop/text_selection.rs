//! Read mode: drag across page text to select it, then copy it.
use super::*;
use mypdf_milestone0::viewer::PageText;

/// A run of glyphs in reading order on one page.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct TextSelection {
    pub tab: u64,
    pub page: usize,
    anchor: usize,
    focus: usize,
}
impl TextSelection {
    fn span(&self) -> (usize, usize) {
        (self.anchor.min(self.focus), self.anchor.max(self.focus))
    }
    /// The selected text, as extracted from the page.
    pub fn text(&self, tab: &Tab) -> Option<String> {
        let text = tab.text.get(self.page)?.as_ref()?;
        let (lo, hi) = self.span();
        let (start, end) = (
            text.glyphs.get(lo)?.range.start,
            text.glyphs.get(hi)?.range.end,
        );
        let (start, end) = (start.min(end), start.max(end));
        text.text
            .get(start..end)
            .map(str::to_owned)
            .filter(|t| !t.trim().is_empty())
    }
    pub(super) fn clipboard_text(
        &self,
        tab: &Tab,
    ) -> Option<mypdf_milestone0::clipboard_text::ClipboardText> {
        let text = tab.text.get(self.page)?.as_ref()?;
        let (lo, hi) = self.span();
        let start = text.glyphs.get(lo)?.range.start;
        let end = text.glyphs.get(hi)?.range.end;
        mypdf_milestone0::clipboard_text::from_page(text, start.min(end)..start.max(end))
    }
    pub(super) fn contains(&self, tab: &Tab, p: [f32; 2]) -> bool {
        self.rects(tab).iter().any(|r| {
            p[0] >= r[0] - 2. && p[0] <= r[2] + 2. && p[1] >= r[1] - 2. && p[1] <= r[3] + 2.
        })
    }
    pub(super) fn rects(&self, tab: &Tab) -> Vec<[f32; 4]> {
        let Some(text) = tab.text.get(self.page).and_then(Option::as_ref) else {
            return vec![];
        };
        let (lo, hi) = self.span();
        let mut bands: Vec<[f32; 4]> = vec![];
        for g in text.glyphs.iter().skip(lo).take(hi + 1 - lo) {
            let r = g.rect;
            match bands.last_mut() {
                Some(b)
                    if r[1] < b[3] && r[3] > b[1] && r[0] >= b[0] - 2. && r[0] <= b[2] + 12. =>
                {
                    b[0] = b[0].min(r[0]);
                    b[1] = b[1].min(r[1]);
                    b[2] = b[2].max(r[2]);
                    b[3] = b[3].max(r[3]);
                }
                _ => bands.push(r),
            }
        }
        bands
    }
    pub(super) fn word_at(tab: &Tab, page: usize, p: [f32; 2]) -> Option<Self> {
        let text = tab.text.get(page)?.as_ref()?;
        if !over_text(text, p) {
            return None;
        }
        let i = nearest(text, p)?;
        let word = |i: usize| {
            text.text
                .get(text.glyphs[i].range.clone())
                .is_some_and(|s| !s.trim().is_empty())
        };
        if !word(i) {
            return None;
        }
        let mut lo = i;
        let mut hi = i;
        let same_line = |i: usize, j: usize| {
            let a = text.glyphs[i].rect;
            let b = text.glyphs[j].rect;
            a[1] < b[3] && a[3] > b[1] && (a[0] - b[2]).abs().min((b[0] - a[2]).abs()) < 12.
        };
        while lo > 0 && word(lo - 1) && same_line(lo, lo - 1) {
            lo -= 1;
        }
        while hi + 1 < text.glyphs.len() && word(hi + 1) && same_line(hi, hi + 1) {
            hi += 1;
        }
        Some(Self {
            tab: tab.id,
            page,
            anchor: lo,
            focus: hi,
        })
    }
    /// Every glyph of a page.
    pub fn all(tab: &Tab, page: usize) -> Option<Self> {
        let text = tab.text.get(page)?.as_ref()?;
        (!text.glyphs.is_empty()).then(|| Self {
            tab: tab.id,
            page,
            anchor: 0,
            focus: text.glyphs.len() - 1,
        })
    }
}

/// The glyph under a page point, or the closest one, favoring the same line.
fn nearest(text: &PageText, p: [f32; 2]) -> Option<usize> {
    text.glyphs
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let r = g.rect;
            let dx = (r[0] - p[0]).max(p[0] - r[2]).max(0.);
            let dy = (r[1] - p[1]).max(p[1] - r[3]).max(0.);
            (i, dy * 4. + dx)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

fn over_text(text: &PageText, p: [f32; 2]) -> bool {
    text.glyphs.iter().any(|g| {
        let r = g.rect;
        p[0] >= r[0] - 1. && p[0] <= r[2] + 1. && p[1] >= r[1] - 1. && p[1] <= r[3] + 1.
    })
}

fn focus_at(
    selection: &mut Option<TextSelection>,
    tab: u64,
    page: usize,
    focus: usize,
    extend: bool,
    glyph_count: usize,
) {
    let anchor = selection
        .filter(|s| extend && s.tab == tab && s.page == page && s.anchor < glyph_count)
        .map_or(focus, |s| s.anchor);
    *selection = Some(TextSelection {
        tab,
        page,
        anchor,
        focus,
    });
}

/// Drag to select, Shift-click after release to extend from the original anchor,
/// and click without Shift to clear. Selection never crosses tabs or pages.
pub(super) fn interact(
    selection: &mut Option<TextSelection>,
    tab: &Tab,
    ctx: &egui::Context,
    page: usize,
    response: &egui::Response,
    page_rect: Rect,
    scale: f32,
) {
    let Some(text) = tab.text.get(page).and_then(Option::as_ref) else {
        return;
    };
    if text.glyphs.is_empty() {
        return;
    }
    let normal = |p: egui::Pos2| {
        let p = (p - page_rect.min) / scale;
        [p.x, p.y]
    };
    if let Some(pos) = response.hover_pos()
        && over_text(text, normal(pos))
    {
        ctx.set_cursor_icon(egui::CursorIcon::Text);
    }
    let shift = ctx.input(|i| i.modifiers.shift);
    if response.drag_started()
        && let Some(origin) = ctx.input(|i| i.pointer.press_origin())
        && let Some(focus) = nearest(text, normal(origin))
    {
        focus_at(selection, tab.id, page, focus, shift, text.glyphs.len());
    }
    if (response.dragged() || response.drag_stopped())
        && let Some(s) = selection
            .as_mut()
            .filter(|s| s.tab == tab.id && s.page == page)
        && let Some(pos) = response.interact_pointer_pos()
        && let Some(focus) = nearest(text, normal(pos))
    {
        s.focus = focus;
        ctx.set_cursor_icon(egui::CursorIcon::Text);
    }
    if response.clicked() {
        if shift
            && let Some(pos) = response.interact_pointer_pos()
            && let Some(focus) = nearest(text, normal(pos))
        {
            focus_at(selection, tab.id, page, focus, true, text.glyphs.len());
        } else {
            *selection = None;
        }
    }
}

/// Highlight the selection, one band per line.
pub(super) fn paint(
    selection: &Option<TextSelection>,
    tab: &Tab,
    painter: &egui::Painter,
    page: usize,
    page_rect: Rect,
    scale: f32,
) {
    let Some(s) = selection.filter(|s| s.tab == tab.id && s.page == page) else {
        return;
    };
    let Some(text) = tab.text.get(page).and_then(Option::as_ref) else {
        return;
    };
    let (lo, hi) = s.span();
    let mut bands: Vec<[f32; 4]> = vec![];
    for g in text.glyphs.iter().skip(lo).take(hi + 1 - lo) {
        let r = g.rect;
        match bands.last_mut() {
            Some(b) if r[1] < b[3] && r[3] > b[1] && r[0] >= b[0] - 2. => {
                b[0] = b[0].min(r[0]);
                b[1] = b[1].min(r[1]);
                b[2] = b[2].max(r[2]);
                b[3] = b[3].max(r[3]);
            }
            _ => bands.push(r),
        }
    }
    for b in bands {
        painter.rect_filled(
            Rect::from_min_max(
                page_rect.min + vec2(b[0], b[1]) * scale,
                page_rect.min + vec2(b[2], b[3]) * scale,
            ),
            1.,
            Color32::from_rgba_unmultiplied(64, 132, 255, 70),
        );
    }
}

impl Viewer {
    /// The selected page text in Read mode, when nothing else wants Copy.
    pub(super) fn page_text_selection(&self, ctx: &egui::Context) -> Option<String> {
        if ctx.wants_keyboard_input() || self.editor.text_editing {
            return None;
        }
        let s = self.editor.text_selection?;
        let tab = self.tabs.iter().find(|t| t.id == s.tab)?;
        s.text(tab)
    }
    pub(super) fn copy_page_text(&mut self, ctx: &egui::Context) {
        if self.page_text_selection(ctx).is_none() {
            return;
        }
        let selection = self.editor.text_selection.unwrap();
        if let Some(text) = self
            .tabs
            .iter()
            .find(|tab| tab.id == selection.tab)
            .and_then(|tab| selection.clipboard_text(tab))
        {
            self.copy_formatted_text(ctx, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mypdf_milestone0::viewer::GlyphBox;

    fn pointer_fixture() -> (Viewer, egui::Context) {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.open_memory(
            PathBuf::from("Text selection pointer test.pdf"),
            mypdf_milestone0::convert::blank(300., 300.).unwrap(),
            &ctx,
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let text = "abcd\nefgh\nijkl";
        app.tabs[0].text[0] = Some(Arc::new(PageText {
            text: text.into(),
            glyphs: text
                .char_indices()
                .filter(|(_, c)| *c != '\n')
                .map(|(start, _)| {
                    let x = 40. + (start % 5) as f32 * 20.;
                    let y = 50. + (start / 5) as f32 * 30.;
                    GlyphBox {
                        range: start..start + 1,
                        rect: [x, y, x + 10., y + 12.],
                    }
                })
                .collect(),
            ..Default::default()
        }));
        app.editor.mode = editing::Mode::Read;
        pointer_frame(&mut app, &ctx, 0, egui::Modifiers::NONE, vec![]);
        (app, ctx)
    }

    fn pointer_frame(
        app: &mut Viewer,
        ctx: &egui::Context,
        page: usize,
        modifiers: egui::Modifiers,
        events: Vec<egui::Event>,
    ) {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(400., 400.))),
                modifiers,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    editing::page_interaction(
                        &mut app.editor,
                        &mut app.tabs[0],
                        ui,
                        page,
                        Rect::from_min_size(pos2(20., 20.), vec2(300., 300.)),
                        1.,
                        &mut app.status,
                    );
                });
            },
        );
    }

    fn glyph_point(index: usize) -> egui::Pos2 {
        pos2(
            65. + (index % 4) as f32 * 20.,
            76. + (index / 4) as f32 * 30.,
        )
    }

    fn pointer_button(pos: egui::Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        }
    }

    fn drag_text(
        app: &mut Viewer,
        ctx: &egui::Context,
        from: usize,
        to: usize,
        modifiers: egui::Modifiers,
    ) {
        let from = glyph_point(from);
        let to = glyph_point(to);
        pointer_frame(
            app,
            ctx,
            0,
            modifiers,
            vec![
                egui::Event::PointerMoved(from),
                pointer_button(from, true, modifiers),
            ],
        );
        pointer_frame(app, ctx, 0, modifiers, vec![egui::Event::PointerMoved(to)]);
        pointer_frame(
            app,
            ctx,
            0,
            modifiers,
            vec![pointer_button(to, false, modifiers)],
        );
        assert!(!ctx.input(|i| i.pointer.primary_down()));
    }

    fn click_text(
        app: &mut Viewer,
        ctx: &egui::Context,
        page: usize,
        index: usize,
        modifiers: egui::Modifiers,
    ) {
        let at = glyph_point(index);
        pointer_frame(
            app,
            ctx,
            page,
            modifiers,
            vec![
                egui::Event::PointerMoved(at),
                pointer_button(at, true, modifiers),
            ],
        );
        pointer_frame(
            app,
            ctx,
            page,
            modifiers,
            vec![pointer_button(at, false, modifiers)],
        );
    }

    #[test]
    fn shift_click_after_mouse_release_extends_forward_backward_and_copies_lines() {
        let (mut app, ctx) = pointer_fixture();
        drag_text(&mut app, &ctx, 1, 5, egui::Modifiers::NONE);
        assert_eq!(app.page_text_selection(&ctx).as_deref(), Some("bcd\nef"));
        let anchor = app.editor.text_selection.unwrap().anchor;
        // Shift is pressed only for a new click, after the selection drag and
        // its mouse release have finished.
        click_text(&mut app, &ctx, 0, 10, egui::Modifiers::SHIFT);
        let selected = app.editor.text_selection.unwrap();
        assert_eq!((selected.anchor, selected.focus), (anchor, 10));
        assert_eq!(
            selected.text(&app.tabs[0]).as_deref(),
            Some("bcd\nefgh\nijk")
        );
        app.copy_page_text(&ctx);
        assert_eq!(clipboard::rich_text_for_test().unwrap().0, "bcd\nefgh\nijk");
        click_text(&mut app, &ctx, 0, 0, egui::Modifiers::SHIFT);
        let selected = app.editor.text_selection.unwrap();
        assert_eq!((selected.anchor, selected.focus), (anchor, 0));
        assert_eq!(selected.text(&app.tabs[0]).as_deref(), Some("ab"));
        click_text(&mut app, &ctx, 0, 3, egui::Modifiers::NONE);
        assert!(app.editor.text_selection.is_none());
    }

    #[test]
    fn shift_click_keeps_reversed_drag_anchor_and_shift_drag_uses_it_too() {
        let (mut app, ctx) = pointer_fixture();
        drag_text(&mut app, &ctx, 6, 4, egui::Modifiers::NONE);
        assert_eq!(app.page_text_selection(&ctx).as_deref(), Some("efg"));
        click_text(&mut app, &ctx, 0, 11, egui::Modifiers::SHIFT);
        let selected = app.editor.text_selection.unwrap();
        assert_eq!((selected.anchor, selected.focus), (6, 11));
        assert_eq!(selected.text(&app.tabs[0]).as_deref(), Some("gh\nijkl"));
        drag_text(&mut app, &ctx, 11, 2, egui::Modifiers::SHIFT);
        let selected = app.editor.text_selection.unwrap();
        assert_eq!((selected.anchor, selected.focus), (6, 2));
        assert_eq!(selected.text(&app.tabs[0]).as_deref(), Some("cd\nefg"));
    }

    #[test]
    fn shift_click_starts_local_anchor_without_reusing_another_page_or_tab() {
        let (mut app, ctx) = pointer_fixture();
        click_text(&mut app, &ctx, 0, 5, egui::Modifiers::SHIFT);
        let selected = app.editor.text_selection.unwrap();
        assert_eq!((selected.anchor, selected.focus), (5, 5));
        let page = app.tabs[0].pages[0];
        let text = app.tabs[0].text[0].clone();
        app.tabs[0].pages.push(page);
        app.tabs[0].text.push(text);
        pointer_frame(&mut app, &ctx, 1, egui::Modifiers::NONE, vec![]);
        click_text(&mut app, &ctx, 1, 10, egui::Modifiers::SHIFT);
        let selected = app.editor.text_selection.unwrap();
        assert_eq!(
            (selected.page, selected.anchor, selected.focus),
            (1, 10, 10)
        );
        app.editor.text_selection = Some(TextSelection {
            tab: app.tabs[0].id + 1,
            page: 1,
            anchor: 2,
            focus: 3,
        });
        click_text(&mut app, &ctx, 1, 6, egui::Modifiers::SHIFT);
        let selected = app.editor.text_selection.unwrap();
        assert_eq!(
            (selected.tab, selected.anchor, selected.focus),
            (app.tabs[0].id, 6, 6)
        );
    }

    #[test]
    fn shift_click_over_highlight_extends_text_while_normal_click_selects_annotation() {
        use mypdf_milestone0::annotations::{Annotation, Kind, Markup};
        let (mut app, ctx) = pointer_fixture();
        let mark = Annotation::new(
            0,
            [40., 110., 120., 122.],
            Kind::Markup {
                kind: Markup::Highlight,
                quads: vec![[0., 0., 1., 1.]],
            },
            Default::default(),
        );
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![mark.clone()])
            .unwrap();
        drag_text(&mut app, &ctx, 1, 5, egui::Modifiers::NONE);
        click_text(&mut app, &ctx, 0, 10, egui::Modifiers::SHIFT);
        assert_eq!(
            app.page_text_selection(&ctx).as_deref(),
            Some("bcd\nefgh\nijk")
        );
        assert!(app.tabs[0].selected_annotation.is_none());
        click_text(&mut app, &ctx, 0, 10, egui::Modifiers::NONE);
        assert!(app.editor.text_selection.is_none());
        assert_eq!(
            app.tabs[0].selected_annotation.as_deref(),
            Some(mark.id.as_str())
        );
        // A selection on a different page cannot suppress this annotation's
        // normal action merely because Shift is held during the click.
        let page = app.tabs[0].pages[0];
        let text = app.tabs[0].text[0].clone();
        app.tabs[0].pages.push(page);
        app.tabs[0].text.push(text);
        app.editor.text_selection = Some(TextSelection {
            tab: app.tabs[0].id,
            page: 1,
            anchor: 1,
            focus: 5,
        });
        click_text(&mut app, &ctx, 0, 10, egui::Modifiers::SHIFT);
        assert!(app.editor.text_selection.is_none());
        assert_eq!(
            app.tabs[0].selected_annotation.as_deref(),
            Some(mark.id.as_str())
        );
    }

    #[test]
    fn dragging_over_page_text_selects_it_for_copying() {
        use eframe::App;
        let temp = tempfile::tempdir().unwrap();
        let pdf = temp.path().join("Read.pdf");
        let font = mypdf_milestone0::document::default_font().unwrap();
        std::fs::write(&pdf, mypdf_milestone0::document::fixture(&font).unwrap()).unwrap();
        let ctx = egui::Context::default();
        let mut app = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            vec![pdf],
        );
        app.tabs[0].zoom = Zoom::Page;
        let frame = |app: &mut Viewer, events: Vec<egui::Event>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1100., 900.))),
                    events,
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while app.tabs[0].text.first().is_none_or(Option::is_none) {
            assert!(std::time::Instant::now() < deadline);
            frame(&mut app, vec![]);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        assert!(app.editor.mode == editing::Mode::Read);
        let text = app.tabs[0].text[0].clone().unwrap();
        let wanted = "Invoice total";
        let at = text.text.find(wanted).expect("fixture text");
        let first = text
            .glyphs
            .iter()
            .find(|g| g.range.start == at)
            .unwrap()
            .rect;
        let last = text
            .glyphs
            .iter()
            .find(|g| g.range.end == at + wanted.len())
            .unwrap()
            .rect;
        let page = ctx
            .read_response(egui::Id::new((app.tabs[0].id, 0_usize, "page_actions")))
            .unwrap()
            .rect;
        let scale = app.tabs[0].scale;
        let screen = |p: [f32; 2]| page.min + vec2(p[0], p[1]) * scale;
        let start = screen([first[0] + 0.5, (first[1] + first[3]) / 2.]);
        let end = screen([last[2] - 0.5, (last[1] + last[3]) / 2.]);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(
            &mut app,
            vec![egui::Event::PointerMoved(start), button(start, true)],
        );
        frame(
            &mut app,
            vec![egui::Event::PointerMoved(start + vec2(20., 0.))],
        );
        frame(&mut app, vec![egui::Event::PointerMoved(end)]);
        frame(&mut app, vec![button(end, false)]);
        assert_eq!(app.page_text_selection(&ctx).as_deref(), Some(wanted));
        app.copy_page_text(&ctx);
        assert!(
            app.status
                .as_ref()
                .is_some_and(|(s, _)| s.contains("13 characters"))
        );

        // ⌘A selects the page's text; a click clears the selection.
        app.action("select_all", &ctx);
        assert!(
            app.page_text_selection(&ctx)
                .is_some_and(|t| t.contains("Split text"))
        );
        let blank = screen([500., 780.]);
        frame(
            &mut app,
            vec![egui::Event::PointerMoved(blank), button(blank, true)],
        );
        frame(&mut app, vec![button(blank, false)]);
        assert!(app.editor.text_selection.is_none());
    }
    #[test]
    fn nearest_glyph_prefers_the_same_line() {
        let text = PageText {
            text: "ab\ncd".into(),
            glyphs: vec![
                GlyphBox {
                    range: 0..1,
                    rect: [10., 10., 20., 20.],
                },
                GlyphBox {
                    range: 1..2,
                    rect: [20., 10., 30., 20.],
                },
                GlyphBox {
                    range: 3..4,
                    rect: [10., 30., 20., 40.],
                },
                GlyphBox {
                    range: 4..5,
                    rect: [20., 30., 30., 40.],
                },
            ],
            ..Default::default()
        };
        assert_eq!(nearest(&text, [25., 15.]), Some(1));
        assert_eq!(nearest(&text, [90., 16.]), Some(1), "past the line end");
        assert_eq!(nearest(&text, [12., 33.]), Some(2));
        assert!(over_text(&text, [15., 15.]) && !over_text(&text, [60., 60.]));
    }
}
