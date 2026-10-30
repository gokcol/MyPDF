//! Page fitting and reading shortcuts share the actual document viewport.
use super::*;

pub(super) const GUTTER: f32 = 12.;
pub(super) const GAP: f32 = 24.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum PageLayout {
    #[default]
    Continuous,
    Single,
    Facing,
}
impl PageLayout {
    pub fn label(self) -> &'static str {
        match self {
            Self::Continuous => "Continuous",
            Self::Single => "Single page",
            Self::Facing => "Facing pages",
        }
    }
    pub fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Single,
            2 => Self::Facing,
            _ => Self::Continuous,
        }
    }
}
#[derive(Clone, PartialEq)]
pub(super) struct Layout {
    pub rects: Vec<Rect>,
    pub scale: f32,
    pub size: egui::Vec2,
}
impl Layout {
    #[cfg(test)]
    pub fn new(pages: &[PageSize], zoom: Zoom, viewport: egui::Vec2) -> Self {
        Self::arrange(pages, zoom, viewport, PageLayout::Continuous, 0)
    }
    pub fn arrange(
        pages: &[PageSize],
        zoom: Zoom,
        viewport: egui::Vec2,
        mode: PageLayout,
        current: usize,
    ) -> Self {
        // The first page is the reference for both fit modes. Keep one
        // scale throughout a mixed-size document; later pages never zoom it.
        let scale = pages.first().map_or(1., |page| {
            let reference_width = page.width * if mode == PageLayout::Facing { 2. } else { 1. };
            let gap = if mode == PageLayout::Facing { GAP } else { 0. };
            match zoom {
                Zoom::Width => (viewport.x - 2. * GUTTER - gap) / reference_width,
                Zoom::Page => ((viewport.x - 2. * GUTTER - gap) / reference_width)
                    .min((viewport.y - 2. * GAP) / page.height),
                Zoom::Scale(scale) => scale,
            }
            .clamp(0.1, 6.)
        });
        let rows: Vec<Vec<usize>> = match mode {
            PageLayout::Continuous => (0..pages.len()).map(|p| vec![p]).collect(),
            PageLayout::Single => {
                if pages.is_empty() {
                    vec![]
                } else {
                    vec![vec![current.min(pages.len() - 1)]]
                }
            }
            PageLayout::Facing => (0..pages.len())
                .step_by(2)
                .map(|p| (p..(p + 2).min(pages.len())).collect())
                .collect(),
        };
        let row_width = |row: &[usize]| {
            row.iter().map(|&p| pages[p].width * scale).sum::<f32>()
                + GAP * row.len().saturating_sub(1) as f32
        };
        let width = rows
            .iter()
            .map(|r| row_width(r) + 2. * GUTTER)
            .fold(viewport.x, f32::max);
        let mut top = GAP;
        let mut rects = vec![Rect::NOTHING; pages.len()];
        for row in rows {
            let mut left = (width - row_width(&row)) / 2.;
            let height = row
                .iter()
                .map(|&p| pages[p].height * scale)
                .fold(0., f32::max);
            for p in row {
                let size = vec2(pages[p].width, pages[p].height) * scale;
                rects[p] = Rect::from_min_size(pos2(left, top), size);
                left += size.x + GAP;
            }
            top += height + GAP;
        }
        Self {
            rects,
            scale,
            size: vec2(width, top),
        }
    }

    pub fn page_at(&self, point: egui::Vec2) -> usize {
        self.rects
            .iter()
            .enumerate()
            .filter(|(_, r)| r.is_positive())
            .min_by(|(_, a), (_, b)| {
                a.distance_sq_to_pos(point.to_pos2())
                    .total_cmp(&b.distance_sq_to_pos(point.to_pos2()))
            })
            .map_or(0, |(p, _)| p)
    }

    /// Keep the same PDF point under the pointer, including when leaving
    /// Fit Width in a document with mixed paper sizes.
    pub fn anchored_offset(
        &self,
        next: &Self,
        offset: egui::Vec2,
        pointer: egui::Vec2,
    ) -> egui::Vec2 {
        let point = offset + pointer;
        let page = self.page_at(point);
        let (Some(old), Some(new)) = (self.rects.get(page), next.rects.get(page)) else {
            return offset;
        };
        if !old.is_positive() || !new.is_positive() {
            return egui::Vec2::ZERO;
        }
        let pdf = (point - old.min.to_vec2()) / self.scale;
        (new.min.to_vec2() + pdf * next.scale - pointer).max(egui::Vec2::ZERO)
    }
}

impl Viewer {
    pub(super) fn reading_keys(&mut self, ctx: &egui::Context) {
        if self.home.visible
            || self.comparison.is_some()
            || ctx.wants_keyboard_input()
            || self.editor.text_editing
            || self.content.pending_text().is_some()
            || self.digital.open
            || self.editor.signature_open
            || self.editor.library_open
            || self.recovery.open
            || self.close_intent.is_some()
            || self.accessible_text
            || self.link_confirm.is_some()
            || ctx.memory(|m| m.top_modal_layer().is_some())
            || egui::Popup::is_any_open(ctx)
        {
            return;
        }
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        if tab.locked.is_some() {
            return;
        }
        for (key, forward) in [(egui::Key::ArrowLeft, false), (egui::Key::ArrowRight, true)] {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::ALT, key)) {
                tab.history_step(forward);
            }
        }
        let read = self.editor.mode == editing::Mode::Read;
        for (key, page) in [
            (egui::Key::PageDown, tab.page + 1),
            (egui::Key::PageUp, tab.page.saturating_sub(1)),
            (egui::Key::Home, 0),
            (egui::Key::End, tab.pages.len().saturating_sub(1)),
            (egui::Key::ArrowDown, tab.page + 1),
            (egui::Key::ArrowUp, tab.page.saturating_sub(1)),
        ] {
            if !read && matches!(key, egui::Key::ArrowUp | egui::Key::ArrowDown) {
                continue;
            }
            // Consume the event so egui does not also scroll or move focus.
            // Modified arrows belong to text selection and other shortcuts.
            if ctx.input_mut(|i| i.modifiers.is_none() && i.consume_key(egui::Modifiers::NONE, key))
            {
                tab.jump(page);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;
    use lopdf::{Document, Object, Stream, dictionary};

    fn fixture() -> Vec<u8> {
        let mut pdf = Document::with_version("1.7");
        let root = pdf.new_object_id();
        let font = pdf.add_object(
            dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" },
        );
        let mut kids = vec![];
        for (i, (width, height)) in [(595, 842), (1190, 842), (420, 842)]
            .into_iter()
            .enumerate()
        {
            let text = format!(
                "BT /F1 28 Tf 48 740 Td (Reading test - page {}) Tj 0 -48 Td /F1 14 Tf (First page sets the zoom. Up and Down change pages.) Tj ET",
                i + 1
            );
            let stream = pdf.add_object(Stream::new(dictionary! {}, text.into_bytes()));
            let page = pdf.add_object(dictionary! {
                "Type" => "Page", "Parent" => root,
                "MediaBox" => vec![0.into(), 0.into(), width.into(), height.into()],
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
                "Contents" => stream,
            });
            kids.push(Object::Reference(page));
        }
        pdf.objects.insert(
            root,
            dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => 3 }.into(),
        );
        let catalog = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => root });
        pdf.trailer.set("Root", catalog);
        let mut bytes = vec![];
        pdf.save_to(&mut bytes).unwrap();
        bytes
    }
    fn frame(app: &mut Viewer, ctx: &egui::Context, size: egui::Vec2, events: Vec<egui::Event>) {
        let modifiers = events
            .iter()
            .find_map(|e| match e {
                egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), size)),
                max_texture_side: Some(8192),
                events,
                modifiers,
                ..Default::default()
            },
            |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
        );
    }
    fn key(key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
        [true, false]
            .into_iter()
            .map(|pressed| egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            })
            .collect()
    }
    fn loaded() -> (tempfile::TempDir, Viewer, egui::Context) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Reading.pdf");
        let bytes = fixture();
        std::fs::write(&path, &bytes).unwrap();
        if let Some(proof) = std::env::var_os("MYPDF_READING_PROOF_DIR") {
            let proof = PathBuf::from(proof);
            std::fs::create_dir_all(&proof).unwrap();
            std::fs::write(proof.join("Reading.pdf"), &bytes).unwrap();
        }
        let ctx = egui::Context::default();
        let mut app = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            vec![path],
        );
        let end = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while app.tabs[0].document_view.is_none() {
            assert!(std::time::Instant::now() < end);
            frame(&mut app, &ctx, vec2(1200., 820.), vec![]);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        (dir, app, ctx)
    }

    #[test]
    fn first_page_sets_fit_zoom_and_later_sizes_do_not_change_it() {
        let (_dir, mut app, ctx) = loaded();
        for _ in 0..3 {
            frame(&mut app, &ctx, vec2(1200., 820.), vec![]);
        }
        let fit = |app: &Viewer| {
            let tab = &app.tabs[0];
            let layout = tab.document_layout.as_ref().unwrap();
            let (view, offset, _) = tab.document_view.unwrap();
            assert!((layout.rects[0].width() - (view.width() - GUTTER * 2.)).abs() < 1.);
            assert!(
                (layout.rects[0].left() - offset.x - GUTTER).abs() < 1.,
                "reference page centered"
            );
            assert!(
                layout
                    .rects
                    .iter()
                    .zip(&tab.pages)
                    .all(|(r, p)| (r.width() / p.width - tab.scale).abs() < 0.001),
                "one zoom for every page"
            );
        };
        fit(&app);
        let first_scale = app.tabs[0].scale;
        app.tabs[0].jump(1);
        for _ in 0..3 {
            frame(&mut app, &ctx, vec2(1200., 820.), vec![]);
        }
        assert_eq!(app.tabs[0].page, 1);
        assert_eq!(app.tabs[0].scale, first_scale);
        fit(&app);
        // Changing panels and window width must not require a page jump
        // to restore centered Fit Width.
        app.sidebar = false;
        for _ in 0..3 {
            frame(&mut app, &ctx, vec2(1400., 900.), vec![]);
        }
        fit(&app);
        assert!(app.tabs[0].scale > first_scale);
        app.action("actual", &ctx);
        for _ in 0..3 {
            frame(&mut app, &ctx, vec2(1000., 760.), vec![]);
        }
        assert_eq!(app.tabs[0].scale, 1., "manual zoom is retained on resize");
        app.action("fit_page", &ctx);
        for _ in 0..3 {
            frame(&mut app, &ctx, vec2(1200., 820.), vec![]);
        }
        let tab = &app.tabs[0];
        let layout = tab.document_layout.as_ref().unwrap();
        let (view, _, _) = tab.document_view.unwrap();
        assert!(layout.rects[0].height() <= view.height() - 2. * GAP + 1.);
        assert!((layout.rects[1].width() / tab.pages[1].width - tab.scale).abs() < 0.001);
    }

    #[test]
    fn read_arrows_turn_one_page_and_leave_fields_dialogs_and_editing_alone() {
        let (_dir, mut app, ctx) = loaded();
        let revision = app.tabs[0].session.as_ref().unwrap().revision();
        for (key_code, expected) in [
            (egui::Key::ArrowUp, 0),
            (egui::Key::ArrowDown, 1),
            (egui::Key::ArrowDown, 2),
            (egui::Key::ArrowDown, 2),
            (egui::Key::ArrowUp, 1),
            (egui::Key::ArrowUp, 0),
        ] {
            frame(
                &mut app,
                &ctx,
                vec2(1200., 820.),
                key(key_code, egui::Modifiers::NONE),
            );
            assert_eq!(app.tabs[0].page, expected);
            for _ in 0..3 {
                frame(&mut app, &ctx, vec2(1200., 820.), vec![]);
            }
            assert_eq!(
                app.tabs[0].page, expected,
                "no second egui scroll after turning a page"
            );
        }
        frame(
            &mut app,
            &ctx,
            vec2(1200., 820.),
            key(egui::Key::ArrowDown, egui::Modifiers::SHIFT),
        );
        assert_eq!(app.tabs[0].page, 0);
        app.editor.mode = editing::Mode::Draw;
        frame(
            &mut app,
            &ctx,
            vec2(1200., 820.),
            key(egui::Key::ArrowDown, egui::Modifiers::NONE),
        );
        assert_eq!(app.tabs[0].page, 0);
        app.editor.mode = editing::Mode::Read;
        app.digital.open = true;
        frame(
            &mut app,
            &ctx,
            vec2(1200., 820.),
            key(egui::Key::ArrowDown, egui::Modifiers::NONE),
        );
        assert_eq!(app.tabs[0].page, 0);
        app.digital.open = false;
        app.action("find", &ctx);
        for _ in 0..3 {
            frame(&mut app, &ctx, vec2(1200., 820.), vec![]);
        }
        assert!(ctx.wants_keyboard_input());
        frame(
            &mut app,
            &ctx,
            vec2(1200., 820.),
            key(egui::Key::ArrowDown, egui::Modifiers::NONE),
        );
        assert_eq!(app.tabs[0].page, 0, "search field owns its arrows");
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), revision);
    }

    #[test]
    fn resizing_and_pinching_keep_the_pdf_point_on_a_later_page() {
        let pages = [
            PageSize {
                width: 600.,
                height: 800.,
            },
            PageSize {
                width: 1200.,
                height: 800.,
            },
        ];
        let before = Layout::new(&pages, Zoom::Width, vec2(900., 700.));
        for zoom in [Zoom::Width, Zoom::Scale(2.)] {
            let after = Layout::new(&pages, zoom, vec2(1100., 700.));
            let pdf_point = vec2(700., 350.);
            let pointer = vec2(450., 300.);
            let offset = before.rects[1].min.to_vec2() + pdf_point * before.scale - pointer;
            let next = before.anchored_offset(&after, offset, pointer);
            let same = (next + pointer - after.rects[1].min.to_vec2()) / after.scale;
            assert!((same - pdf_point).length() < 0.01);
        }
        let single = Layout::new(&pages[..1], Zoom::Width, vec2(900., 700.));
        assert_eq!(
            single.size.x, 900.,
            "fit width creates no horizontal overflow for uniform pages"
        );
    }
    #[test]
    fn single_and_facing_views_preserve_page_indices_and_uniform_scale() {
        let pages = [
            PageSize {
                width: 595.,
                height: 842.,
            },
            PageSize {
                width: 842.,
                height: 595.,
            },
            PageSize {
                width: 595.,
                height: 842.,
            },
        ];
        let single = Layout::arrange(
            &pages,
            Zoom::Width,
            vec2(1000., 800.),
            PageLayout::Single,
            1,
        );
        assert!(!single.rects[0].is_positive());
        assert!(single.rects[1].is_positive());
        assert_eq!(single.page_at(vec2(400., 100.)), 1);
        let facing = Layout::arrange(
            &pages,
            Zoom::Width,
            vec2(1000., 800.),
            PageLayout::Facing,
            0,
        );
        assert_eq!(facing.rects[0].top(), facing.rects[1].top());
        assert!(facing.rects[0].right() < facing.rects[1].left());
        assert!(facing.rects[2].top() > facing.rects[0].bottom());
        assert!((facing.rects[1].width() / 842. - facing.scale).abs() < 0.001);
        assert_eq!(facing.page_at(facing.rects[1].center().to_vec2()), 1);
    }
    #[test]
    fn scrolling_defers_disk_state_and_close_flushes_even_if_source_is_temporarily_unavailable() {
        let (_dir, mut app, ctx) = loaded();
        app.remember_reading(&ctx, true);
        let path = app.tabs[0].path.clone();
        let baseline = app.home.reading(&path);
        for (page, scale) in [(1, 1.25), (2, 1.5)] {
            app.tabs[0].zoom = Zoom::Scale(scale);
            app.tabs[0].jump(page);
            frame(&mut app, &ctx, vec2(1200., 820.), vec![]);
        }
        let latest = app.tabs[0].view_preferences.clone();
        assert_ne!(Some(latest.clone()), baseline);
        assert_eq!(app.home.reading(&path), baseline);
        assert_eq!(app.reading_memory.pending[&path], latest);
        let temporary = path.with_extension("temporarily-unavailable");
        std::fs::rename(&path, &temporary).unwrap();
        let id = app.tabs[0].id;
        app.request_close(Some(id), &ctx);
        assert_eq!(app.home.reading(&path), Some(latest.clone()));
        assert!(app.reading_memory.pending.is_empty());
        std::fs::rename(&temporary, &path).unwrap();
        app.open(path, &ctx);
        let end = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while app.tabs[0].pages.is_empty() {
            assert!(std::time::Instant::now() < end);
            frame(&mut app, &ctx, vec2(1200., 820.), vec![]);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        frame(&mut app, &ctx, vec2(1200., 820.), vec![]);
        assert_eq!(app.tabs[0].page, latest.page);
        assert!(app.tabs[0].zoom == Zoom::Scale(latest.scale.unwrap()));
    }

    #[test]
    fn quiet_frame_saves_latest_reading_state_and_approved_exit_flushes_workspace() {
        let (_dir, mut app, ctx) = loaded();
        app.remember_reading(&ctx, true);
        app.workspace.invalidate();
        app.remember_workspace(&ctx);
        let workspace_file = app.workspace.root.join("workspace.json");
        let previous_workspace = std::fs::read(&workspace_file).unwrap();
        let path = app.tabs[0].path.clone();
        let previous_reading = app.home.reading(&path);
        app.tabs[0].zoom = Zoom::Scale(1.75);
        app.tabs[0].jump(1);
        frame(&mut app, &ctx, vec2(1200., 820.), vec![]);
        let latest = app.tabs[0].view_preferences.clone();
        assert_ne!(Some(latest.clone()), previous_reading);
        assert_eq!(app.home.reading(&path), previous_reading);
        assert_eq!(std::fs::read(&workspace_file).unwrap(), previous_workspace);
        let now = std::time::Instant::now();
        app.reading_memory.changed = Some(now - READING_SAVE_QUIET);
        app.reading_memory.written = Some(now - READING_SAVE_RETRY);
        app.remember_reading(&ctx, false);
        assert_eq!(app.home.reading(&path), Some(latest.clone()));
        assert!(app.reading_memory.pending.is_empty());
        // Approved exit must bypass both the workspace interval and navigation debounce.
        app.reading_memory.changed = Some(std::time::Instant::now());
        app.exit_approved = true;
        app.remember_workspace(&ctx);
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&workspace_file).unwrap()).unwrap();
        assert_eq!(
            saved["tabs"][0]["view"],
            serde_json::to_value(latest).unwrap()
        );
    }

    #[test]
    fn back_forward_and_saved_reading_position_return_to_the_same_place() {
        let (_dir, mut app, ctx) = loaded();
        let size = vec2(1200., 820.);
        app.tabs[0].jump(1);
        frame(&mut app, &ctx, size, vec![]);
        app.tabs[0].jump(2);
        frame(&mut app, &ctx, size, vec![]);
        app.tabs[0].history_step(false);
        frame(&mut app, &ctx, size, vec![]);
        assert_eq!(app.tabs[0].page, 1);
        app.tabs[0].history_step(true);
        frame(&mut app, &ctx, size, vec![]);
        assert_eq!(app.tabs[0].page, 2);
        app.sidebar = false;
        app.tabs[0].zoom = Zoom::Scale(1.25);
        app.tabs[0].page_layout = PageLayout::Single;
        frame(&mut app, &ctx, size, vec![]);
        app.remember_reading(&ctx, true);
        let state = app.home.reading(&app.tabs[0].path).unwrap();
        assert_eq!(state.page, 2);
        assert_eq!(state.scale, Some(1.25));
        assert!(!state.sidebar);
        assert_eq!(state.layout, 1);
        let path = app.tabs[0].path.clone();
        app.close(0);
        app.open(path, &ctx);
        let end = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while app.tabs[0].pages.is_empty() {
            assert!(std::time::Instant::now() < end);
            frame(&mut app, &ctx, size, vec![]);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        frame(&mut app, &ctx, size, vec![]);
        assert_eq!(app.tabs[0].page, 2);
        assert!(app.tabs[0].zoom == Zoom::Scale(1.25));
        assert!(!app.sidebar);
    }
}

const READING_SAVE_QUIET: std::time::Duration = std::time::Duration::from_millis(600);
pub(super) const READING_SAVE_CHECKPOINT: std::time::Duration = std::time::Duration::from_secs(10);
const READING_SAVE_RETRY: std::time::Duration = std::time::Duration::from_secs(1);

#[derive(Default)]
pub(super) struct Memory {
    active: Option<u64>,
    pending: HashMap<PathBuf, mypdf_milestone0::home::ReadingState>,
    pending_since: Option<std::time::Instant>,
    changed: Option<std::time::Instant>,
    written: Option<std::time::Instant>,
}
impl Memory {
    fn queue(
        &mut self,
        path: PathBuf,
        state: mypdf_milestone0::home::ReadingState,
        now: std::time::Instant,
    ) {
        if self.pending.is_empty() {
            self.pending_since = Some(now);
        }
        self.changed = Some(now);
        self.pending.insert(path, state);
    }
    pub(super) fn settling(&self, now: std::time::Instant) -> bool {
        self.changed
            .is_some_and(|at| now.saturating_duration_since(at) < READING_SAVE_QUIET)
    }
    fn save_delay(&self, now: std::time::Instant) -> std::time::Duration {
        let remaining = |at: Option<std::time::Instant>, interval: std::time::Duration| {
            interval.saturating_sub(at.map_or(interval, |at| now.saturating_duration_since(at)))
        };
        remaining(self.changed, READING_SAVE_QUIET)
            .min(remaining(self.pending_since, READING_SAVE_CHECKPOINT))
            .max(remaining(self.written, READING_SAVE_RETRY))
    }
    fn persist(
        &mut self,
        now: std::time::Instant,
        save: impl FnOnce(&[(PathBuf, mypdf_milestone0::home::ReadingState)]) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        let states: Vec<_> = self
            .pending
            .iter()
            .map(|(p, s)| (p.clone(), s.clone()))
            .collect();
        self.written = Some(now);
        // Keep the latest positions available for retry if an atomic write fails.
        save(&states)?;
        self.pending.clear();
        self.pending_since = None;
        Ok(())
    }
}
impl Tab {
    fn reading_position(&self) -> (usize, f32) {
        let top = self
            .document_view
            .zip(self.document_layout.as_ref())
            .and_then(|((_, offset, _), l)| {
                l.rects
                    .get(self.page)
                    .filter(|r| r.is_positive())
                    .map(|r| ((offset.y - r.top() + GAP) / l.scale).max(0.))
            })
            .unwrap_or(0.);
        (self.page, top)
    }
    pub(super) fn push_history(&mut self) {
        let position = self.reading_position();
        if self.back.last() != Some(&position) {
            self.back.push(position);
        }
        if self.back.len() > 100 {
            self.back.remove(0);
        }
        self.forward.clear();
    }
    pub(super) fn history_step(&mut self, forward: bool) {
        let destination = if forward {
            self.forward.pop()
        } else {
            self.back.pop()
        };
        if let Some((page, top)) = destination {
            let now = self.reading_position();
            if forward {
                self.back.push(now);
            } else {
                self.forward.push(now);
            }
            self.page = page.min(self.pages.len().saturating_sub(1));
            self.page_input = (self.page + 1).to_string();
            self.jump = Some((self.page, Some(top)));
        }
    }
}
impl Viewer {
    pub(super) fn restore_reading_panels(&mut self) {
        let active = self.tabs.get(self.active).map(|t| t.id);
        if self.reading_memory.active != active {
            self.reading_memory.active = active;
            if let Some(tab) = self.tabs.get(self.active) {
                self.sidebar = tab.view_preferences.sidebar;
                self.inspector = tab.view_preferences.inspector;
            }
        }
    }
    pub(super) fn remember_reading(&mut self, ctx: &egui::Context, flush: bool) {
        let now = std::time::Instant::now();
        if !self.home.visible
            && self.comparison.is_none()
            && let Some(tab) = self.tabs.get_mut(self.active)
            && !tab.pages.is_empty()
            && tab.document_view.is_some()
            && tab.jump.is_none()
        {
            let (page, top) = tab.reading_position();
            let state = mypdf_milestone0::home::ReadingState {
                page,
                top: (top * 10.).round() / 10.,
                scale: if let Zoom::Scale(s) = tab.zoom {
                    Some(s)
                } else {
                    None
                },
                fit_page: tab.zoom == Zoom::Page,
                layout: match tab.page_layout {
                    PageLayout::Continuous => 0,
                    PageLayout::Single => 1,
                    PageLayout::Facing => 2,
                },
                sidebar: self.sidebar,
                inspector: self.inspector,
            };
            if tab.view_preferences != state {
                tab.view_preferences = state.clone();
                self.reading_memory.changed = Some(now);
                // Queue in memory while scrolling; filesystem checks and fsync wait
                // for a quiet frame. Untitled documents have no saved identity.
                if tab.has_destination && tab.path.is_absolute() {
                    self.reading_memory.queue(tab.path.clone(), state, now);
                }
            }
        }
        if self.reading_memory.pending.is_empty() {
            return;
        }
        let delay = self.reading_memory.save_delay(now);
        if !flush && !delay.is_zero() {
            ctx.request_repaint_after(delay);
            return;
        }
        // These are identities of opened, saved documents. A temporary cloud or
        // network metadata failure must not discard their queued reading state.
        let home = &mut self.home;
        if self
            .reading_memory
            .persist(now, |states| home.save_reading(states))
            .is_err()
        {
            ctx.request_repaint_after(READING_SAVE_RETRY);
        }
    }
}

#[cfg(test)]
impl Tab {
    /// PDF coordinate at the top-center of the viewport, independent of zoom
    /// and changes in inspector width.
    pub(super) fn canvas_anchor(&self) -> (usize, egui::Vec2) {
        let (view, offset, _) = self.document_view.unwrap();
        let layout = self.document_layout.as_ref().unwrap();
        let at = offset + vec2(view.width() / 2., 0.);
        let page = layout.page_at(at);
        (page, (at - layout.rects[page].min.to_vec2()) / layout.scale)
    }
    pub(super) fn assert_canvas_anchor(&self, expected: (usize, egui::Vec2)) {
        let actual = self.canvas_anchor();
        assert_eq!(actual.0, expected.0, "The viewport changed pages");
        assert!(
            (actual.1.y - expected.1.y).abs() < 1.,
            "Viewport jumped: {expected:?} -> {actual:?}"
        );
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    use mypdf_milestone0::home::ReadingState;
    use std::time::{Duration, Instant};

    #[test]
    fn moving_positions_wait_for_quiet_but_long_scrolls_checkpoint() {
        let start = Instant::now();
        let mut memory = Memory::default();
        let path = PathBuf::from("reading.pdf");
        for step in 0..100 {
            let now = start + Duration::from_millis(step * 100);
            memory.queue(
                path.clone(),
                ReadingState {
                    top: step as f32,
                    ..Default::default()
                },
                now,
            );
            assert!(!memory.save_delay(now).is_zero());
        }
        assert!(memory.save_delay(start + READING_SAVE_CHECKPOINT).is_zero());
        let mut saved = vec![];
        memory
            .persist(start + READING_SAVE_CHECKPOINT, |states| {
                saved = states.to_vec();
                Ok(())
            })
            .unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].1.top, 99.);
        assert!(memory.pending.is_empty());
        let next = start + Duration::from_secs(12);
        memory.queue(
            path,
            ReadingState {
                top: 120.,
                ..Default::default()
            },
            next,
        );
        assert_eq!(memory.save_delay(next), READING_SAVE_QUIET);
        assert!(memory.save_delay(next + READING_SAVE_QUIET).is_zero());
    }

    #[test]
    fn failed_checkpoint_retains_latest_positions_for_retry_and_close_flush() {
        let now = Instant::now();
        let mut memory = Memory::default();
        let path = PathBuf::from("reading.pdf");
        memory.queue(
            path.clone(),
            ReadingState {
                top: 20.,
                ..Default::default()
            },
            now,
        );
        assert!(
            memory
                .persist(now, |_| anyhow::bail!("disk temporarily unavailable"))
                .is_err()
        );
        assert_eq!(memory.pending[&path].top, 20.);
        memory.queue(
            path.clone(),
            ReadingState {
                top: 30.,
                ..Default::default()
            },
            now,
        );
        assert_eq!(memory.save_delay(now), READING_SAVE_RETRY);
        // A close flush bypasses the delay and persists the newest state.
        memory
            .persist(now, |states| {
                assert_eq!(states[0].1.top, 30.);
                Ok(())
            })
            .unwrap();
        assert!(memory.pending.is_empty());
        assert!(memory.pending_since.is_none());
    }
}
