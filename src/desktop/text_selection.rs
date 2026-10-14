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

/// Select by dragging, clear by clicking, and show the text cursor over text.
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
    if response.drag_started()
        && let Some(origin) = ctx.input(|i| i.pointer.press_origin())
        && let Some(anchor) = nearest(text, normal(origin))
    {
        *selection = Some(TextSelection {
            tab: tab.id,
            page,
            anchor,
            focus: anchor,
        });
    }
    if response.dragged()
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
        *selection = None;
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
        if let Some(text) = self.page_text_selection(ctx) {
            let count = text.chars().count();
            ctx.copy_text(text);
            self.status = Some((
                format!(
                    "Copied {count} character{}",
                    if count == 1 { "" } else { "s" }
                ),
                false,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mypdf_milestone0::viewer::GlyphBox;

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
            limited: false,
        };
        assert_eq!(nearest(&text, [25., 15.]), Some(1));
        assert_eq!(nearest(&text, [90., 16.]), Some(1), "past the line end");
        assert_eq!(nearest(&text, [12., 33.]), Some(2));
        assert!(over_text(&text, [15., 15.]) && !over_text(&text, [60., 60.]));
    }
}
