//! Compact document-table tools; PDF mutations use the shared undo/recovery path.
use super::*;
use mypdf_milestone0::tables::{self, Table};
type Key = (u64, u64, usize);
type Analysis = Result<(Vec<Table>, Vec<Table>), String>;
#[derive(Default)]
pub(super) struct State {
    key: Option<Key>,
    rx: Option<mpsc::Receiver<(Key, Analysis)>>,
    owned: Vec<Table>,
    layers: Vec<(String, Table)>,
    layer_mode: bool,
    mode: Option<editing::Mode>,
    template: usize,
    detected: Vec<Table>,
    error: Option<String>,
    pub chooser: bool,
    area_after: bool,
    area: Option<(Key, [f32; 4])>,
    pub(super) show_panel: bool,
    rows: usize,
    columns: usize,
    pub(super) placing: bool,
    origin: Option<egui::Pos2>,
    editor: Option<Editor>,
    waiting: Option<Waiting>,
    layer: Option<egui::LayerId>,
    handoff: Option<Handoff>,
    resume_text: Option<(u64, usize)>,
}
struct Handoff {
    tab: u64,
    page: usize,
    origin: egui::Pos2,
    scale: f32,
    start: egui::Pos2,
    end: egui::Pos2,
    released: bool,
}
struct Editor {
    key: Key,
    table: Table,
    base: Table,
    cell: usize,
    text: String,
    confirmed: bool,
    owned: bool,
    focus: bool,
    layer: Option<String>,
    scope: usize,
    drag: Option<(Table, egui::Pos2, usize, usize)>,
}
struct Waiting {
    key: Key,
    next: Option<usize>,
    close: bool,
    save: Option<save::Intent>,
    detach: bool,
}
impl Editor {
    fn dirty(&self) -> bool {
        self.table != self.base || self.text != self.table.cells[self.cell]
    }
}
impl State {
    pub fn new() -> Self {
        Self {
            rows: 3,
            columns: 3,
            ..Default::default()
        }
    }
    pub fn editing_layer(&self) -> Option<&str> {
        self.editor.as_ref().and_then(|e| e.layer.as_deref())
    }
    pub fn deferring_input(&self) -> bool {
        self.handoff.is_some() || self.waiting.is_some()
    }
    pub fn owns_page(&self, id: u64, page: usize) -> bool {
        self.placing
            || self
                .editor
                .as_ref()
                .is_some_and(|e| e.key.0 == id && e.key.2 == page)
    }
}
impl Viewer {
    pub(super) fn choose_table(&mut self) {
        self.editor.select_tool(editing::Tool::Select);
        self.content.image_mode = false;
        self.table.layer_mode = self.editor.mode == editing::Mode::Annotate;
        self.table.chooser = true;
    }

    pub(super) fn table_from_selection(&mut self) {
        if self.editor.mode != editing::Mode::Edit {
            return;
        }
        if let Some(area) = self.content.release_table_selection() {
            self.table.area = Some(area);
            self.inspector = true;
        } else {
            self.status = Some((
                "Select an area around a table before detecting it. Finish any text changes first."
                    .into(),
                false,
            ));
        }
    }
    pub(super) fn table_tick(&mut self, ctx: &egui::Context) {
        if self.table.waiting.is_some() && !self.content.busy {
            let wait = self.table.waiting.take().unwrap();
            let revision = self
                .tabs
                .iter()
                .find(|t| t.id == wait.key.0)
                .and_then(|t| t.session.as_ref())
                .map(|s| s.revision());
            if revision.is_some_and(|r| r > wait.key.1) {
                if let Some(e) = &mut self.table.editor {
                    e.key.1 = revision.unwrap();
                    e.table.cells[e.cell] = e.text.clone();
                    e.base = e.table.clone();
                    if wait.detach
                        && let Some(mark) = self
                            .tabs
                            .iter()
                            .find(|t| t.id == wait.key.0)
                            .and_then(|t| t.session.as_ref())
                            .and_then(|s| {
                                s.annotations.iter().find(|a| {
                                    a.object.table.as_ref().is_some_and(|t| t.id == e.table.id)
                                })
                            })
                    {
                        e.layer = Some(mark.id.clone());
                        e.owned = true;
                        e.confirmed = true;
                    }
                    if let Some(next) = wait.next {
                        e.cell = next.min(e.table.cells.len() - 1);
                        e.text = e.table.cells[e.cell].clone();
                        e.focus = true;
                    }
                }
                if wait.close {
                    self.table.editor = None;
                }
                self.table.key = None;
                if let Some(intent) = wait.save {
                    self.table.editor = None;
                    if let Some(index) = self.tabs.iter().position(|t| t.id == wait.key.0) {
                        let previous = self.active;
                        self.active = index;
                        self.save_intent = intent;
                        self.save_dialog(ctx);
                        self.active = previous.min(self.tabs.len().saturating_sub(1));
                    }
                }
            } else if wait.close {
                self.restore_table_input();
            }
        }
        if let Some(rx) = &self.table.rx
            && let Ok((key, result)) = rx.try_recv()
        {
            self.table.rx = None;
            if self.table.key == Some(key) {
                match result {
                    Ok((owned, detected)) => {
                        self.table.owned = owned;
                        self.table.detected = detected;
                        self.table.error = None;
                    }
                    Err(e) => self.table.error = Some(e),
                }
            }
        }
        if !matches!(
            self.editor.mode,
            editing::Mode::Edit | editing::Mode::Annotate
        ) {
            return;
        }
        if self.table.mode != Some(self.editor.mode) {
            self.table.mode = Some(self.editor.mode);
            self.table.layer_mode = self.editor.mode == editing::Mode::Annotate;
            self.table.key = None;
        }
        let Some(key) = self
            .tabs
            .get(self.active)
            .and_then(|t| t.session.as_ref().map(|s| (t.id, s.revision(), t.page)))
        else {
            return;
        };
        // Undo or an unrelated document transaction invalidates a clean inspector.
        if self.table.waiting.is_none()
            && self
                .table
                .editor
                .as_ref()
                .is_some_and(|e| e.key.0 == key.0 && e.key.1 != key.1 && !e.dirty())
        {
            self.table.editor = None;
        }
        if self.table.key == Some(key)
            && self.table.rx.is_none()
            && let Some((request, area)) = self.table.area.take()
            && request == key
        {
            let found = self
                .table
                .owned
                .iter()
                .chain(&self.table.detected)
                .find(|t| {
                    let [l, t, r, b] = t.rect();
                    l >= area[0] - 3. && t >= area[1] - 3. && r <= area[2] + 3. && b <= area[3] + 3.
                })
                .map(Table::rect);
            if let Some(r) = found {
                self.open_table_at([(r[0] + r[2]) / 2., (r[1] + r[3]) / 2.]);
            } else {
                self.status = Some((
                    "No complete bordered table found in the selected area.".into(),
                    false,
                ));
            }
        }
        if self.table.key == Some(key) || self.table.rx.is_some() || self.content.busy {
            return;
        }
        let t = &self.tabs[self.active];
        let s = t.session.as_ref().unwrap();
        self.table.key = Some(key);
        self.table.layers = s
            .annotations
            .iter()
            .filter(|a| a.page == t.page && !a.object.hidden && !a.object.locked)
            .filter_map(|a| tables::from_layer(a).map(|t| (a.id.clone(), t)))
            .collect();
        self.table.owned.clear();
        self.table.detected.clear();
        let bytes = s.source.clone();
        let (tx, rx) = mpsc::channel();
        self.table.rx = Some(rx);
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<_> {
                Ok((
                    tables::owned(&bytes, key.2 as u32 + 1)?,
                    tables::detect(&bytes, key.2 as u32 + 1)?,
                ))
            })()
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send((key, result));
            wake.request_repaint();
        });
    }
    pub(super) fn open_table_at(&mut self, point: [f32; 2]) {
        if self
            .table
            .editor
            .as_ref()
            .is_some_and(|e| e.confirmed && e.dirty())
        {
            self.status = Some((
                "Finish the current table edit with Done or Cancel first.".into(),
                false,
            ));
            return;
        }
        if self.content.pending_text().is_some() {
            self.status = Some((
                "Finish the text edit before opening the table.".into(),
                false,
            ));
            return;
        }
        let Some(key) = self.table.key else { return };
        let found = self
            .table
            .layers
            .iter()
            .find(|(_, t)| t.hit(point).is_some())
            .map(|(id, t)| (t.clone(), true, Some(id.clone())))
            .or_else(|| {
                if self.editor.mode != editing::Mode::Edit {
                    return None;
                }
                self.table
                    .owned
                    .iter()
                    .find(|t| t.hit(point).is_some())
                    .map(|t| (t.clone(), true, None))
                    .or_else(|| {
                        self.table
                            .detected
                            .iter()
                            .find(|t| t.hit(point).is_some())
                            .map(|t| (t.clone(), false, None))
                    })
            });
        if let Some((table, owned, layer)) = found {
            let cell = table.hit(point).unwrap_or(0);
            self.table.editor = Some(Editor {
                key,
                base: table.clone(),
                text: table.cells[cell].clone(),
                table,
                cell,
                confirmed: owned,
                owned,
                focus: true,
                layer,
                scope: 0,
                drag: None,
            });
            self.table.chooser = false;
            self.inspector = true;
        } else {
            self.table.chooser = true;
            self.status = Some((
                if self.table.rx.is_some() {
                    "Checking page borders…"
                } else {
                    "No complete bordered table found here. Use Table to place a new grid."
                }
                .into(),
                false,
            ));
        }
    }
    pub(super) fn table_commit(
        &mut self,
        ctx: &egui::Context,
        next: Option<usize>,
        close: bool,
        save: Option<save::Intent>,
    ) -> bool {
        if self.table.waiting.is_some() || self.content.busy {
            return false;
        }
        let Some(e) = &mut self.table.editor else {
            return false;
        };
        if !e.dirty() {
            if let Some(n) = next {
                e.cell = n;
                e.text = e.table.cells[n].clone();
                e.focus = true;
            }
            if close {
                self.table.editor = None;
            }
            return false;
        }
        e.table.cells[e.cell] = e.text.clone();
        let key = e.key;
        if let Some(layer_id) = e.layer.clone() {
            let table = e.table.clone();
            let Some(tab) = self.tabs.get_mut(self.active).filter(|t| t.id == key.0) else {
                return false;
            };
            let Some(session) = &mut tab.session else {
                return false;
            };
            if session.revision() != key.1 {
                self.status = Some(("The table changed; reopen it before editing.".into(), true));
                return false;
            }
            let previous = session.annotations.iter().find(|a| a.id == layer_id);
            let result = tables::layer(&table, key.2, previous).and_then(|mark| {
                let mut marks = session.annotations.clone();
                marks.retain(|a| a.id != layer_id);
                let id = mark.id.clone();
                marks.push(mark);
                session.commit(marks)?;
                Ok(id)
            });
            return match result {
                Ok(id) => {
                    session.name_last_change("Edit table layer");
                    session.set_change_focus(key.2, None);
                    let revision = session.revision();
                    let e = self.table.editor.as_mut().unwrap();
                    e.layer = Some(id);
                    e.key.1 = revision;
                    e.base = e.table.clone();
                    if let Some(n) = next {
                        e.cell = n;
                        e.text = e.table.cells[n].clone();
                        e.focus = true;
                    }
                    if close {
                        self.table.editor = None;
                    }
                    self.table.key = None;
                    self.content.key = None;
                    if let Some(intent) = save {
                        self.table.editor = None;
                        self.save_intent = intent;
                        self.save_dialog(ctx);
                    }
                    true
                }
                Err(err) => {
                    self.status = Some((format!("{err:#}"), true));
                    false
                }
            };
        }
        let edit = if e.owned {
            content::Edit::TableWrite {
                page: key.2 as u32 + 1,
                table: e.table.clone(),
            }
        } else {
            content::Edit::TableCells {
                page: key.2 as u32 + 1,
                table: e.table.clone(),
                base: e.base.clone(),
            }
        };
        if !self.tabs.get(self.active).is_some_and(|t| {
            t.id == key.0 && t.session.as_ref().is_some_and(|s| s.revision() == key.1)
        }) {
            self.status = Some((
                "The page changed. Cancel the table edit and reopen it.".into(),
                true,
            ));
            return false;
        }
        self.content_change(ctx, edit);
        if self.content.busy {
            let key = (
                key.0,
                self.tabs[self.active].session.as_ref().unwrap().revision(),
                key.2,
            );
            if let Some(e) = &mut self.table.editor {
                e.key.1 = key.1;
            }
            self.table.waiting = Some(Waiting {
                key,
                next,
                close,
                save,
                detach: false,
            });
            true
        } else {
            false
        }
    }
    /// Finish the cell before handing the canvas to another editor or tool.
    /// Layer commits are immediate; embedded writes retain the pointer gesture
    /// until the new PDF revision and its text geometry are ready.
    pub(super) fn table_handoff(&mut self, ctx: &egui::Context) {
        if let Some(handoff) = &mut self.table.handoff
            && !handoff.released
        {
            if let Some(pos) = ctx.pointer_interact_pos() {
                handoff.end = pos;
            }
            handoff.released |=
                ctx.input(|i| i.pointer.primary_released() || !i.pointer.primary_down());
        }
        if self.table.editor.is_none() && self.table.waiting.is_none() && !self.content.busy {
            if let Some(target) = self.table.resume_text.take() {
                self.content.add_target = Some(target);
            }
            if self.table.handoff.as_ref().is_some_and(|h| h.released) {
                let h = self.table.handoff.take().unwrap();
                if self.editor.mode == editing::Mode::Edit
                    && self
                        .tabs
                        .get(self.active)
                        .is_some_and(|t| t.id == h.tab && h.page < t.pages.len())
                {
                    self.tabs[self.active].page = h.page;
                    self.tabs[self.active].page_input = (h.page + 1).to_string();
                    let a = (h.start - h.origin) / h.scale;
                    let b = (h.end - h.origin) / h.scale;
                    if h.start.distance(h.end) >= 4. {
                        let size = self.tabs[self.active].pages[h.page];
                        self.content.context.area_after_table = Some((
                            h.tab,
                            h.page,
                            [
                                a.x.min(b.x).clamp(0., size.width),
                                a.y.min(b.y).clamp(0., size.height),
                                a.x.max(b.x).clamp(0., size.width),
                                a.y.max(b.y).clamp(0., size.height),
                            ],
                        ));
                    } else {
                        self.content.context.select_after_text = Some((h.tab, h.page, [b.x, b.y]));
                    }
                }
            }
        }
        let Some(e) = &self.table.editor else { return };
        if self.table.waiting.is_some() || self.content.busy || e.drag.is_some() {
            return;
        }
        let Some(tab) = self.tabs.get(self.active).filter(|t| t.id == e.key.0) else {
            return;
        };
        let mode_changed = self.table.mode != Some(self.editor.mode);
        let tool_changed = self.editor.mode == editing::Mode::Annotate
            && self.editor.tool != editing::Tool::Select;
        let other_tool = self.content.add_target.is_some()
            || self.content.area_mode
            || self.content.region_active()
            || self.redaction.active
            || self.editor.library_open
            || self.workbench.paste.is_some()
            || self.workbench.crop.is_some();
        let mut outside = None;
        if let Some((view, offset, _)) = tab.document_view
            && let Some(layout) = &tab.document_layout
            && let Some(table_page) = layout.rects.get(e.key.2)
        {
            let press = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::PointerButton {
                        pos,
                        button,
                        pressed: true,
                        ..
                    } if matches!(
                        button,
                        egui::PointerButton::Primary | egui::PointerButton::Secondary
                    ) =>
                    {
                        Some((*pos, *button))
                    }
                    _ => None,
                })
            });
            if let Some((pos, button)) = press
                && view.contains(pos)
                && self.table.layer == ctx.layer_id_at(pos)
            {
                let origin = view.min - offset + table_page.min.to_vec2();
                let scale = layout.scale;
                let [l, t, r, b] = e.table.rect();
                let body =
                    Rect::from_min_max(origin + vec2(l, t) * scale, origin + vec2(r, b) * scale);
                let move_handle = Rect::from_center_size(
                    pos2(body.center().x, (body.top() - 18.).max(origin.y + 8.)),
                    vec2(40., 16.),
                );
                let on_handle = move_handle.contains(pos)
                    || e.table.xs.iter().skip(1).any(|x| {
                        Rect::from_center_size(
                            origin + vec2(*x, t) * scale + vec2(0., -5.),
                            vec2(12., 12.),
                        )
                        .contains(pos)
                    })
                    || e.table.ys.iter().skip(1).any(|y| {
                        Rect::from_center_size(
                            origin + vec2(r, *y) * scale + vec2(5., 0.),
                            vec2(12., 12.),
                        )
                        .contains(pos)
                    });
                if !body.expand(3.).contains(pos) && !on_handle {
                    outside = Some((pos, button, view, offset, layout.clone()));
                }
            }
        }
        if !mode_changed && !tool_changed && !other_tool && outside.is_none() {
            return;
        }
        let tab_id = tab.id;
        let old_revision = e.key.1;
        let layer = e.layer.is_some();
        if !layer
            && self.editor.mode == editing::Mode::Edit
            && let Some((start, egui::PointerButton::Primary, view, offset, layout)) = outside
            && let Some((page, rect)) = layout
                .rects
                .iter()
                .enumerate()
                .find(|(_, r)| r.translate(view.min.to_vec2() - offset).contains(start))
        {
            self.table.handoff = Some(Handoff {
                tab: tab_id,
                page,
                origin: view.min - offset + rect.min.to_vec2(),
                scale: layout.scale,
                start,
                end: ctx.pointer_interact_pos().unwrap_or(start),
                released: ctx.input(|i| i.pointer.primary_released() || !i.pointer.primary_down()),
            });
        }
        if e.confirmed && e.dirty() {
            self.table.resume_text = self.content.add_target;
            if self.table_commit(ctx, None, true, None) {
                // A table layer does not change embedded text or image indices.
                if layer && let Some(s) = self.tabs[self.active].session.as_ref() {
                    let key = (tab_id, s.revision(), self.tabs[self.active].page);
                    if let Some(info) = &mut self.content.inspection
                        && info.key == (tab_id, old_revision, key.2)
                    {
                        info.key = key;
                        self.content.key = Some(key);
                    }
                }
            } else {
                self.restore_table_input();
            }
        } else {
            self.table.editor = None;
        }
        if self.table.editor.is_none() {
            ctx.memory_mut(|m| m.surrender_focus(egui::Id::new((tab_id, "table_cell_text"))));
        }
    }

    fn restore_table_input(&mut self) {
        self.table.handoff = None;
        self.table.resume_text = None;
        if let Some(e) = &self.table.editor {
            self.editor.mode = if e.layer.is_some() {
                editing::Mode::Annotate
            } else {
                editing::Mode::Edit
            };
            self.editor.select_tool(editing::Tool::Select);
            self.content.cancel_table_handoff_tools();
            self.redaction.active = false;
            self.editor.library_open = false;
            self.workbench.paste = None;
            self.workbench.crop = None;
        }
    }
    pub(super) fn delete_table(&mut self, ctx: &egui::Context) {
        if self.content.busy || self.table.waiting.is_some() {
            return;
        }
        let Some(e) = &self.table.editor else { return };
        let key = e.key;
        let Some(session) = self
            .tabs
            .get(self.active)
            .filter(|t| t.id == key.0)
            .and_then(|t| t.session.as_ref())
        else {
            return;
        };
        if session.revision() != key.1 {
            self.status = Some(("The table changed. Reopen it before deleting.".into(), true));
            return;
        }
        if e.base.cells.iter().any(|v| v.contains('\0')) {
            self.table.editor = None;
            return;
        }
        if let Some(id) = e.layer.clone() {
            let Some(tab) = self.tabs.get_mut(self.active).filter(|t| t.id == key.0) else {
                return;
            };
            let Some(s) = &mut tab.session else { return };
            let result = if s.revision() != key.1 {
                Err(anyhow::anyhow!(
                    "The table changed. Reopen it before deleting."
                ))
            } else {
                s.commit(
                    s.annotations
                        .iter()
                        .filter(|a| a.id != id)
                        .cloned()
                        .collect(),
                )
            };
            match result {
                Ok(()) => {
                    s.name_last_change("Delete table");
                    s.set_change_focus(key.2, None);
                    selection::set(tab, &mut self.editor, vec![]);
                    self.table.editor = None;
                    self.table.key = None;
                    self.content.key = None;
                }
                Err(error) => self.status = Some((format!("{error:#}"), true)),
            }
        } else {
            let table = e.base.clone();
            self.content_change(
                ctx,
                content::Edit::TableDelete {
                    page: key.2 as u32 + 1,
                    table,
                },
            );
            if self.content.busy {
                let key = (
                    key.0,
                    self.tabs[self.active].session.as_ref().unwrap().revision(),
                    key.2,
                );
                if let Some(e) = &mut self.table.editor {
                    e.key.1 = key.1;
                }
                self.table.waiting = Some(Waiting {
                    key,
                    next: None,
                    close: true,
                    save: None,
                    detach: false,
                });
            }
        }
    }
    pub(super) fn table_save(&mut self, ctx: &egui::Context) -> bool {
        let active = self.tabs.get(self.active).map(|t| t.id);
        if self
            .table
            .editor
            .as_ref()
            .is_none_or(|e| Some(e.key.0) != active)
        {
            return false;
        }
        if self.table.waiting.is_some() {
            if let Some(w) = &mut self.table.waiting {
                w.save = Some(self.save_intent);
            }
            return true;
        }
        if self
            .table
            .editor
            .as_ref()
            .is_some_and(|e| e.confirmed && e.dirty())
        {
            self.table_commit(ctx, None, true, Some(self.save_intent));
            return true;
        }
        false
    }
    pub(super) fn pending_table(&mut self, target: Option<u64>) -> bool {
        if self
            .table
            .editor
            .as_ref()
            .is_some_and(|e| target.is_none_or(|id| id == e.key.0) && e.confirmed && e.dirty())
        {
            let e = self.table.editor.as_ref().unwrap();
            if let Some(index) = self.tabs.iter().position(|t| t.id == e.key.0) {
                self.active = index;
                self.home.visible = false;
                self.editor.mode = if e.layer.is_some() {
                    editing::Mode::Annotate
                } else {
                    editing::Mode::Edit
                };
                self.inspector = true;
            }
            self.status = Some((
                "Apply or cancel the table edit before closing this document.".into(),
                false,
            ));
            true
        } else {
            false
        }
    }
    pub(super) fn detach_table(&mut self, ctx: &egui::Context) {
        if self.content.busy || self.table.waiting.is_some() {
            return;
        }
        let Some(e) = &mut self.table.editor else {
            return;
        };
        if e.layer.is_some() {
            return;
        }
        e.table.cells[e.cell] = e.text.clone();
        if e.table.id.is_empty() {
            e.table.id = format!("{:032x}", rand::random::<u128>());
        }
        let key = e.key;
        let table = e.table.clone();
        self.content_change(
            ctx,
            content::Edit::DetachTable {
                page: key.2 as u32 + 1,
                table,
            },
        );
        if self.content.busy {
            let key = (
                key.0,
                self.tabs[self.active].session.as_ref().unwrap().revision(),
                key.2,
            );
            if let Some(e) = &mut self.table.editor {
                e.key.1 = key.1;
            }
            self.table.waiting = Some(Waiting {
                key,
                next: None,
                close: false,
                save: None,
                detach: true,
            });
        }
    }
    pub(super) fn table_panel(&mut self, ctx: &egui::Context) -> bool {
        let Some(e) = &self.table.editor else {
            return false;
        };
        if !self
            .tabs
            .get(self.active)
            .is_some_and(|t| t.id == e.key.0 && t.page == e.key.2)
        {
            return false;
        }
        let page_size = self.tabs[self.active].pages.get(e.key.2).copied();
        let waiting = self.table.waiting.is_some();
        let (mut apply, mut cancel, mut confirm, mut detach, mut delete) =
            (false, false, false, false, false);
        let mut structure = None;
        let e = self.table.editor.as_mut().unwrap();
        egui::SidePanel::right("content_inspector")
            .default_width(300.)
            .width_range(280. ..=380.)
            .resizable(true)
            .frame(egui::Frame::new().fill(theme::paper()).inner_margin(16.))
            .show(ctx, |ui| {
                ui.heading(if e.confirmed { "Table" } else { "Detected table" });
                ui.horizontal(|ui| {
                    ui.weak(if e.layer.is_some() { "Movable layer" } else { "Embedded" });
                    ui.separator();
                    ui.weak(format!("{} × {}", e.table.rows(), e.table.columns()));
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("table_properties_scroll")
                    .max_height((ui.available_height() - 118.).max(100.))
                    .show(ui, |ui| {
                        if waiting { ui.disable(); }
                        if !e.confirmed {
                            ui.label("Check the grid before editing or detaching.");
                            egui::CollapsingHeader::new("Adjust detected grid").show(ui, |ui| {
                                for (label, edges) in [("Columns", &mut e.table.xs), ("Rows", &mut e.table.ys)] {
                                    ui.label(label);
                                    egui::Grid::new(label).num_columns(2).show(ui, |ui| {
                                        for i in 0..edges.len() {
                                            let min = if i == 0 { 0. } else { edges[i - 1] + 12. };
                                            let max = edges.get(i + 1).map_or(14000., |v| v - 12.);
                                            ui.small(format!("Edge {}", i + 1));
                                            ui.add(egui::DragValue::new(&mut edges[i]).range(min..=max).speed(0.5).suffix(" pt"));
                                            ui.end_row();
                                        }
                                    });
                                }
                            });
                            confirm = ui.add_enabled(!waiting, ActionButton::new(Icon::Text, "Edit cells").primary()).clicked();
                        } else {
                            ui.small(format!("Row {} · Column {}", e.cell / e.table.columns() + 1, e.cell % e.table.columns() + 1));
                            egui::CollapsingHeader::new("Layout").default_open(true).show(ui, |ui| {
                                if e.owned {
                                    let (r, c) = (e.cell / e.table.columns(), e.cell % e.table.columns());
                                    let [l, t, right, bottom] = e.table.rect();
                                    let (mut x, mut y) = (l, t);
                                    let mut width = e.table.xs[c + 1] - e.table.xs[c];
                                    let mut height = e.table.ys[r + 1] - e.table.ys[r];
                                    let (oldw, oldh) = (width, height);
                                    egui::Grid::new("table_layout_fields").num_columns(2).spacing([12., 6.]).show(ui, |ui| {
                                        for (label, value, min, max) in [
                                            ("Left", &mut x, 0., page_size.map_or(14000., |p| (p.width - (right - l)).max(0.))),
                                            ("Top", &mut y, 0., page_size.map_or(14000., |p| (p.height - (bottom - t)).max(0.))),
                                            ("Column width", &mut width, 12., page_size.map_or(1000., |p| (oldw + p.width - right).max(oldw))),
                                            ("Row height", &mut height, 12., page_size.map_or(1000., |p| (oldh + p.height - bottom).max(oldh))),
                                        ] {
                                            ui.label(label);
                                            ui.add(egui::DragValue::new(value).range(min..=max).speed(0.5).suffix(" pt"));
                                            ui.end_row();
                                        }
                                    });
                                    e.table.translate(x - l, y - t);
                                    for v in &mut e.table.xs[c + 1..] { *v += width - oldw; }
                                    for v in &mut e.table.ys[r + 1..] { *v += height - oldh; }
                                    ui.menu_button("Rows & columns", |ui| {
                                        for (label, row, at, insert) in [
                                            ("Insert row above", true, r, true),
                                            ("Delete row", true, r, false),
                                            ("Insert column before", false, c, true),
                                            ("Delete column", false, c, false),
                                        ] {
                                            if ui.button(label).clicked() { structure = Some((row, at, insert)); ui.close(); }
                                        }
                                    });
                                    ui.weak("Top handle moves · Edge handles resize");
                                } else {
                                    ui.weak("Detach the table to move or resize its borders.");
                                }
                            });
                            egui::CollapsingHeader::new("Cell properties").default_open(true).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    for (i, label) in ["Cell", "Row", "Column", "Table"].iter().enumerate() { ui.selectable_value(&mut e.scope, i, *label); }
                                });
                                let mut style = e.table.style(e.cell);
                                let before = style.clone();
                                cell_properties(ui, &mut style);
                                if style != before {
                                    e.table.set_style(e.cell, match e.scope { 1 => tables::Scope::Row, 2 => tables::Scope::Column, 3 => tables::Scope::Table, _ => tables::Scope::Cell }, style);
                                }
                            });
                            egui::CollapsingHeader::new("Templates").show(ui, |ui| {
                                let mut choice = tables::TEMPLATES.iter().position(|n| *n == e.table.template).unwrap_or(0);
                                if template_choice(ui, &mut choice) { e.table.apply_template(choice); }
                                ui.weak("Keeps your cell text.");
                            });
                        }
                    });
                ui.separator();
                ui.add_enabled_ui(!waiting, |ui| {
                    ui.horizontal(|ui| {
                        if e.confirmed { apply = ui.add(ActionButton::new(Icon::Check, "Done").primary()).clicked(); }
                        cancel = ui.add(ActionButton::new(Icon::Close, "Cancel")).clicked();
                    });
                    if e.layer.is_none() && !e.base.cells.iter().any(|v| v.contains('\0')) {
                        detach = ui.add(ActionButton::new(Icon::Shapes, "Detach as table layer"))
                            .on_hover_text("Convert into a separate movable layer · Undo restores the embedded table").clicked();
                    }
                });
                delete=ui.add_enabled(!waiting,ActionButton::new(Icon::Trash,"Delete table")).clicked();
                if waiting { ui.spinner(); }
            });
        if confirm {
            let source = self.tabs[self.active]
                .session
                .as_ref()
                .unwrap()
                .source
                .clone();
            let e = self.table.editor.as_mut().unwrap();
            let result = (|| -> anyhow::Result<()> {
                e.table.validate()?;
                for i in 0..e.table.cells.len() {
                    e.table.cells[i] =
                        tables::cell_text(&source, e.key.2 as u32 + 1, e.table.cell(i))?.0;
                }
                Ok(())
            })();
            match result {
                Ok(()) => {
                    e.base = e.table.clone();
                    e.confirmed = true;
                    e.cell = 0;
                    e.text = e.table.cells[0].clone();
                    e.focus = true;
                }
                Err(err) => self.status = Some((format!("{err:#}"), true)),
            }
        }
        if let Some((row, at, insert)) = structure {
            let e = self.table.editor.as_mut().unwrap();
            e.table.cells[e.cell] = e.text.clone();
            match e.table.resize_structure(row, at, insert) {
                Ok(()) => {
                    e.cell = e.cell.min(e.table.cells.len() - 1);
                    e.text = e.table.cells[e.cell].clone();
                    e.focus = true;
                }
                Err(err) => self.status = Some((err.to_string(), true)),
            }
        }
        if delete {
            self.delete_table(ctx);
        } else if cancel {
            self.table.editor = None;
        } else if detach {
            self.detach_table(ctx);
        } else if apply {
            if self.table.editor.as_ref().is_some_and(Editor::dirty) {
                self.table_commit(ctx, None, true, None);
            } else {
                self.table.editor = None;
            }
        }
        true
    }
    pub(super) fn table_window(&mut self, ctx: &egui::Context) {
        if ctx
            .data_mut(|d| d.remove_temp::<bool>(egui::Id::new("table_delete")))
            .is_some()
        {
            self.delete_table(ctx);
        }
        if let Some((row, at, insert)) =
            ctx.data_mut(|d| d.remove_temp::<(bool, usize, bool)>(egui::Id::new("table_structure")))
            && self.table.waiting.is_none()
            && !self.content.busy
            && let Some(e) = &mut self.table.editor
        {
            e.table.cells[e.cell] = e.text.clone();
            match e.table.resize_structure(row, at, insert) {
                Ok(()) => {
                    e.cell = e.cell.min(e.table.cells.len() - 1);
                    e.text = e.table.cells[e.cell].clone();
                    e.focus = true;
                }
                Err(err) => self.status = Some((err.to_string(), true)),
            }
        }
        if ctx
            .data_mut(|d| d.remove_temp::<bool>(egui::Id::new("table_select_area")))
            .is_some()
            && self.table.waiting.is_none()
            && !self.content.busy
        {
            self.table.area_after = true;
            if self.table.editor.as_ref().is_some_and(Editor::dirty) {
                self.table_commit(ctx, None, true, None);
            } else {
                self.table.editor = None;
            }
        }
        if self.table.area_after && self.table.editor.is_none() && !self.content.busy {
            self.table.area_after = false;
            self.content.area_mode = true;
            self.content.image_mode = false;
        }
        if !self.table.chooser {
            return;
        }
        let mut open = true;
        let mut place = false;
        let mut choose = None;
        egui::Window::new("Table")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(330.)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.table.layer_mode, true, "Movable layer");
                    ui.add_enabled_ui(self.editor.mode == editing::Mode::Edit, |ui| {
                        ui.selectable_value(&mut self.table.layer_mode, false, "Embedded");
                    });
                });
                template_choice(ui, &mut self.table.template);
                ui.horizontal(|ui| {
                    ui.label("Rows");
                    ui.add(egui::DragValue::new(&mut self.table.rows).range(1..=60));
                    ui.label("Columns");
                    ui.add(egui::DragValue::new(&mut self.table.columns).range(1..=12));
                    place = ui
                        .add(ActionButton::new(Icon::Table, "Place").primary())
                        .clicked();
                });
                ui.small("Drag on the page to size the new table.");
                ui.separator();
                if self.table.rx.is_some() {
                    ui.spinner();
                    ui.label("Checking page borders…");
                }
                let all = self
                    .table
                    .layers
                    .iter()
                    .map(|(_, t)| (t, "Layer"))
                    .chain(
                        self.table
                            .owned
                            .iter()
                            .filter(|_| self.editor.mode == editing::Mode::Edit)
                            .map(|t| (t, "Embedded table")),
                    )
                    .chain(
                        self.table
                            .detected
                            .iter()
                            .filter(|_| self.editor.mode == editing::Mode::Edit)
                            .map(|t| (t, "Detected table")),
                    );
                egui::ScrollArea::vertical()
                    .max_height(220.)
                    .show(ui, |ui| {
                        for (i, (t, kind)) in all.enumerate() {
                            if ui
                                .button(format!(
                                    "{} {} · {} × {}",
                                    kind,
                                    i + 1,
                                    t.rows(),
                                    t.columns()
                                ))
                                .clicked()
                            {
                                choose = Some(t.rect());
                            }
                        }
                    });
                if self.table.layers.is_empty()
                    && self.table.owned.is_empty()
                    && self.table.detected.is_empty()
                    && self.table.rx.is_none()
                {
                    ui.weak("No complete bordered tables found on this page.");
                }
                if let Some(error) = &self.table.error {
                    ui.colored_label(theme::muted(), error);
                }
            });
        self.table.chooser = open;
        if place {
            if self.content.pending_text().is_some()
                || self
                    .table
                    .editor
                    .as_ref()
                    .is_some_and(|e| e.confirmed && e.dirty())
            {
                self.status = Some((
                    "Finish the current edit before placing a table.".into(),
                    false,
                ));
            } else {
                self.table.placing = true;
                self.table.chooser = false;
                self.table.editor = None;
                self.redaction.active = false;
                self.content.area_mode = false;
                self.content.add_target = None;
            }
        }
        if let Some(r) = choose {
            self.open_table_at([(r[0] + r[2]) / 2., (r[1] + r[3]) / 2.]);
        }
    }
}
/// Returns true when table editing owns page input; normal text/image selection
/// continues everywhere else.
pub(super) fn overlay(
    state: &mut State,
    tab: &Tab,
    ui: &mut egui::Ui,
    page: usize,
    rect: Rect,
    scale: f32,
) -> bool {
    state.layer = Some(ui.layer_id());
    let screen = |r: [f32; 4]| {
        Rect::from_min_max(
            rect.min + vec2(r[0], r[1]) * scale,
            rect.min + vec2(r[2], r[3]) * scale,
        )
    };
    if state.placing {
        let response = ui.interact(
            rect,
            egui::Id::new((tab.id, page, "place_table")),
            egui::Sense::drag(),
        );
        if response.drag_started() {
            state.origin = ui.input(|i| i.pointer.press_origin());
        }
        if state.origin.is_none() && response.hovered() && ui.input(|i| i.pointer.primary_pressed())
        {
            state.origin = ui.input(|i| i.pointer.interact_pos());
        }
        if let Some(start) = state.origin
            && let Some(end) = response.interact_pointer_pos()
        {
            let area = Rect::from_two_pos(start, end).intersect(rect);
            let a = (area.min - rect.min) / scale;
            let b = (area.max - rect.min) / scale;
            let mut table = Table::new(state.rows, state.columns, [a.x, a.y, b.x, b.y]);
            table.apply_template(state.template);
            paint_grid(ui, &table, screen, false);
            if response.drag_stopped() {
                if table.validate().is_ok()
                    && let Some(s) = &tab.session
                {
                    let key = (tab.id, s.revision(), page);
                    state.editor = Some(Editor {
                        key,
                        base: Table {
                            cells: vec!["\u{0}".into(); table.cells.len()],
                            ..table.clone()
                        },
                        table: table.clone(),
                        cell: 0,
                        text: String::new(),
                        confirmed: true,
                        owned: true,
                        focus: true,
                        layer: state.layer_mode.then(String::new),
                        scope: 0,
                        drag: None,
                    });
                    state.placing = false;
                    state.show_panel = true;
                }
                state.origin = None;
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            state.placing = false;
            state.origin = None;
        }
        return true;
    }
    if state.editor.is_none()
        && state.key.is_some_and(|k| k.0 == tab.id && k.2 == page)
        && let Some(pos) = ui
            .input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        ..
                    } => Some(*pos),
                    _ => None,
                })
            })
            .filter(|p| {
                rect.intersect(ui.clip_rect()).contains(*p)
                    && ui.ctx().layer_id_at(*p) == Some(ui.layer_id())
            })
    {
        let p = (pos - rect.min) / scale;
        let layer = state
            .layers
            .iter()
            .find(|(_, t)| t.hit([p.x, p.y]).is_some())
            .cloned();
        if let Some((layer_id, table)) = layer {
            let cell = table.hit([p.x, p.y]).unwrap();
            state.show_panel = true;
            state.editor = Some(Editor {
                key: state.key.unwrap(),
                base: table.clone(),
                text: table.cells[cell].clone(),
                table,
                cell,
                confirmed: true,
                owned: true,
                focus: true,
                layer: Some(layer_id),
                scope: 0,
                drag: None,
            });
        } else if state.mode == Some(editing::Mode::Edit)
            && let Some(table) = state
                .owned
                .iter()
                .find(|t| t.hit([p.x, p.y]).is_some())
                .cloned()
        {
            let cell = table.hit([p.x, p.y]).unwrap();
            state.show_panel = true;
            state.editor = Some(Editor {
                key: state.key.unwrap(),
                base: table.clone(),
                text: table.cells[cell].clone(),
                table,
                cell,
                confirmed: true,
                owned: true,
                focus: true,
                layer: None,
                scope: 0,
                drag: None,
            });
        }
    }
    let Some(e) = &mut state.editor else {
        return false;
    };
    if e.key.0 != tab.id || e.key.2 != page {
        return false;
    }
    if e.owned && e.layer.is_none() {
        ui.painter()
            .with_clip_rect(rect.intersect(ui.clip_rect()))
            .rect_filled(screen(e.base.rect()), 0., Color32::WHITE);
    }
    let moving = e.owned && state.waiting.is_none() && table_handles(e, tab, ui, page, rect, scale);
    paint_grid(ui, &e.table, screen, e.owned);
    if moving {
        return true;
    }
    if !e.confirmed {
        return true;
    }
    let waiting = state.waiting.is_some();
    let mut next = None;
    for i in 0..e.table.cells.len() {
        let r = screen(e.table.cell(i)).intersect(rect);
        if i == e.cell {
            continue;
        }
        let response = ui.interact(
            r,
            egui::Id::new((tab.id, page, "table_cell", i)),
            egui::Sense::click(),
        );
        if response.clicked() && !waiting {
            next = Some(i);
        }
        if !waiting {
            response.context_menu(|ui| cell_menu(ui, &e.table, i, e.owned));
        }
    }
    let r = screen(e.table.cell(e.cell));
    let cell_style = e.table.style(e.cell);
    let [left, top, right, bottom] = cell_style.insets;
    let mut textrect = Rect::from_min_max(
        r.min + vec2(left, top) * scale,
        r.max - vec2(right, bottom) * scale,
    );
    if cell_style.vertical != 0 {
        let galley = ui.fonts_mut(|f| {
            f.layout(
                e.text.clone(),
                FontId::proportional(cell_style.size * scale),
                rgb(cell_style.color),
                textrect.width().max(1.),
            )
        });
        let spare = (textrect.height() - galley.size().y).max(0.);
        textrect.min.y += if cell_style.vertical == 1 {
            spare / 2.
        } else {
            spare
        };
    }
    ui.painter()
        .with_clip_rect(rect.intersect(ui.clip_rect()))
        .rect_filled(r.shrink(1.), 0., rgb(cell_style.fill.unwrap_or([1.; 3])));
    ui.painter().rect_stroke(
        r,
        0.,
        Stroke::new(1.5, theme::accent()),
        egui::StrokeKind::Inside,
    );
    if !waiting && ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab)) {
        next = Some((e.cell + e.table.cells.len() - 1) % e.table.cells.len());
    } else if !waiting && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab)) {
        next = Some((e.cell + 1) % e.table.cells.len());
    }
    ui.scope_builder(egui::UiBuilder::new().max_rect(textrect), |ui| {
        ui.set_clip_rect(textrect.intersect(rect));
        ui.add_enabled_ui(!waiting, |ui| {
            let response = ui.add_sized(
                textrect.size(),
                egui::TextEdit::multiline(&mut e.text)
                    .id(egui::Id::new((tab.id, "table_cell_text")))
                    .font(FontId::new(
                        cell_style.size * scale,
                        if cell_style.bold {
                            ui::semibold()
                        } else {
                            egui::FontFamily::Proportional
                        },
                    ))
                    .text_color(rgb(cell_style.color))
                    .margin(0.)
                    .horizontal_align(match cell_style.alignment {
                        1 => egui::Align::Center,
                        2 => egui::Align::RIGHT,
                        _ => egui::Align::LEFT,
                    })
                    .char_limit(32768)
                    .frame(false)
                    .desired_width(textrect.width()),
            );
            response.context_menu(|ui| {
                if ui.button("Copy cell text").clicked() {
                    ui.ctx().copy_text(e.text.clone());
                    ui.close();
                }
                cell_menu(ui, &e.table, e.cell, e.owned);
            });
            if e.focus {
                response.request_focus();
                e.focus = false;
            }
        });
    });
    if let Some(next) = next {
        // A command is consumed by table_tick after the page frame.
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new("table_next_cell"), next));
    }
    true
}
fn paint_grid(ui: &egui::Ui, table: &Table, screen: impl Fn([f32; 4]) -> Rect, owned: bool) {
    let painter = ui.painter().with_clip_rect(ui.clip_rect());
    let scale = screen([0., 0., 1., 1.]).width();
    for i in 0..table.cells.len() {
        let r = screen(table.cell(i));
        let style = table.style(i);
        if owned {
            painter.rect_filled(r, 0., rgb(style.fill.unwrap_or([1.; 3])));
        }
        painter.rect_stroke(
            r,
            0.,
            Stroke::new(
                if owned {
                    (style.border_width * scale).max(0.4)
                } else {
                    1.
                },
                if owned {
                    rgb(style.border)
                } else {
                    theme::accent().gamma_multiply(0.7)
                },
            ),
            egui::StrokeKind::Inside,
        );
        if owned {
            let [l, t, right, b] = style.insets;
            let textrect =
                Rect::from_min_max(r.min + vec2(l, t) * scale, r.max - vec2(right, b) * scale);
            let font = FontId::new(
                style.size * scale,
                if style.bold {
                    ui::semibold()
                } else {
                    egui::FontFamily::Proportional
                },
            );
            let galley = ui.fonts_mut(|f| {
                f.layout(
                    table.cells[i].clone(),
                    font,
                    rgb(style.color),
                    textrect.width().max(1.),
                )
            });
            let x = match style.alignment {
                1 => textrect.center().x - galley.size().x / 2.,
                2 => textrect.right() - galley.size().x,
                _ => textrect.left(),
            };
            let y = match style.vertical {
                1 => textrect.center().y - galley.size().y / 2.,
                2 => textrect.bottom() - galley.size().y,
                _ => textrect.top(),
            };
            painter
                .with_clip_rect(r)
                .galley(pos2(x, y), galley, rgb(style.color));
        }
    }
}
fn table_handles(
    e: &mut Editor,
    tab: &Tab,
    ui: &egui::Ui,
    page: usize,
    page_rect: Rect,
    scale: f32,
) -> bool {
    let Some(size) = tab.pages.get(page) else {
        return false;
    };
    let screen = |x: f32, y: f32| page_rect.min + vec2(x, y) * scale;
    let [l, t, r, _b] = e.table.rect();
    let mut handles = vec![(
        0,
        0,
        pos2(
            ((l + r) / 2.) * scale + page_rect.left(),
            (t * scale + page_rect.top() - 18.).max(page_rect.top() + 8.),
        ),
    )];
    for i in 1..e.table.xs.len() {
        handles.push((1, i, screen(e.table.xs[i], t) + vec2(0., -5.)));
    }
    for i in 1..e.table.ys.len() {
        handles.push((2, i, screen(r, e.table.ys[i]) + vec2(5., 0.)));
    }
    let mut moving = false;
    for (kind, index, center) in handles {
        let rect = Rect::from_center_size(
            center,
            if kind == 0 {
                vec2(38., 14.)
            } else {
                vec2(10., 10.)
            },
        );
        let response = ui
            .interact(
                rect,
                egui::Id::new((tab.id, page, "table_handle", kind, index)),
                egui::Sense::drag(),
            )
            .on_hover_cursor(match kind {
                1 => egui::CursorIcon::ResizeHorizontal,
                2 => egui::CursorIcon::ResizeVertical,
                _ => egui::CursorIcon::Grab,
            });
        if kind == 0 {
            ui.painter().rect_filled(rect, 7., theme::surface());
            ui.painter().line_segment(
                [center - vec2(10., 0.), center + vec2(10., 0.)],
                Stroke::new(2., theme::accent()),
            );
        } else {
            ui.painter().circle_filled(center, 3.5, Color32::WHITE);
            ui.painter()
                .circle_stroke(center, 3.5, Stroke::new(1., theme::accent()));
        }
        if response.drag_started()
            && let Some(start) = ui.input(|i| i.pointer.press_origin())
        {
            e.drag = Some((e.table.clone(), start, kind, index));
        }
        if e.drag
            .as_ref()
            .is_some_and(|(_, _, k, n)| *k == kind && *n == index)
            && (response.dragged() || response.drag_stopped())
            && let Some(pointer) = ui.input(|i| i.pointer.interact_pos())
        {
            let (original, start, _, _) = e.drag.as_ref().unwrap();
            let delta = (pointer - *start) / scale;
            let mut updated = original.clone();
            let [left, top, right, bottom] = original.rect();
            match kind {
                0 => updated.translate(
                    delta.x.clamp(-left, (size.width - right).max(-left)),
                    delta.y.clamp(-top, (size.height - bottom).max(-top)),
                ),
                1 => {
                    let minimum = 12. - (original.xs[index] - original.xs[index - 1]);
                    let d = delta.x.clamp(minimum, (size.width - right).max(minimum));
                    for x in &mut updated.xs[index..] {
                        *x += d;
                    }
                }
                _ => {
                    let minimum = 12. - (original.ys[index] - original.ys[index - 1]);
                    let d = delta.y.clamp(minimum, (size.height - bottom).max(minimum));
                    for y in &mut updated.ys[index..] {
                        *y += d;
                    }
                }
            }
            e.table = updated;
            moving = true;
        }
        if response.drag_stopped() {
            e.drag = None;
        }
    }
    moving || e.drag.is_some()
}

fn cell_menu(ui: &mut egui::Ui, table: &Table, cell: usize, owned: bool) {
    let (r, c) = (cell / table.columns(), cell % table.columns());
    if owned {
        for (label, row, at, insert) in [
            ("Insert row above", true, r, true),
            ("Delete row", true, r, false),
            ("Insert column before", false, c, true),
            ("Delete column", false, c, false),
        ] {
            if ui.button(label).clicked() {
                ui.ctx().data_mut(|d| {
                    d.insert_temp(egui::Id::new("table_structure"), (row, at, insert))
                });
                ui.close();
            }
        }
        ui.separator();
    }
    if ui
        .add(ui::MenuItem::new(Icon::Trash, "Delete table"))
        .clicked()
    {
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new("table_delete"), true));
        ui.close();
    }
    if ui.button("Select area…").clicked() {
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new("table_select_area"), true));
        ui.close();
    }
}

#[cfg(test)]
pub(super) fn preview(app: &mut Viewer, ctx: &egui::Context, detected: bool) {
    let t = &app.tabs[app.active];
    let s = t.session.as_ref().unwrap();
    let key = (t.id, s.revision(), t.page);
    app.table.key = Some(key);
    app.table.owned = tables::owned(&s.source, t.page as u32 + 1).unwrap();
    app.table.detected = tables::detect(&s.source, t.page as u32 + 1).unwrap();
    let table = if detected {
        app.table.detected[0].clone()
    } else {
        app.table.owned[0].clone()
    };
    let r = table.rect();
    app.open_table_at([r[0] + 10., r[1] + 10.]);
    app.inspector = true;
    ctx.request_repaint();
}

fn cell_properties(ui: &mut egui::Ui, style: &mut tables::CellStyle) {
    ui.horizontal(|ui| {
        ui.label("Text");
        ui.add(
            egui::DragValue::new(&mut style.size)
                .range(4. ..=72.)
                .suffix(" pt"),
        );
        if ui.selectable_label(style.bold, "B").clicked() {
            style.bold = !style.bold;
        }
        if ui.selectable_label(style.italic, "I").clicked() {
            style.italic = !style.italic;
        }
        ui::color_swatch(ui, &mut style.color, "Table text color");
    });
    ui.horizontal(|ui| {
        ui.label("Align");
        for (i, label) in ["Left", "Center", "Right"].iter().enumerate() {
            ui.selectable_value(&mut style.alignment, i as u8, *label);
        }
    });
    ui.horizontal(|ui| {
        ui.label("Vertical");
        for (i, label) in ["Top", "Middle", "Bottom"].iter().enumerate() {
            ui.selectable_value(&mut style.vertical, i as u8, *label);
        }
    });
    ui.horizontal(|ui| {
        let mut fill = style.fill.is_some();
        if ui.checkbox(&mut fill, "Fill").changed() {
            style.fill = fill.then_some([0.95; 3]);
        }
        if let Some(color) = &mut style.fill {
            ui::color_swatch(ui, color, "Table cell fill");
        }
        ui.label("Border");
        ui::color_swatch(ui, &mut style.border, "Table border color");
        ui.add(
            egui::DragValue::new(&mut style.border_width)
                .range(0. ..=8.)
                .speed(0.1)
                .suffix(" pt"),
        );
    });
    ui.small("Inside spacing");
    egui::Grid::new("table_cell_insets")
        .num_columns(4)
        .spacing([6., 4.])
        .show(ui, |ui| {
            for (i, label) in ["Left", "Top", "Right", "Bottom"].iter().enumerate() {
                ui.label(*label);
                ui.add(
                    egui::DragValue::new(&mut style.insets[i])
                        .range(0. ..=36.)
                        .speed(0.5)
                        .suffix(" pt"),
                );
                if i % 2 == 1 {
                    ui.end_row();
                }
            }
        });
}
fn template_choice(ui: &mut egui::Ui, choice: &mut usize) -> bool {
    let before = *choice;
    egui::ComboBox::from_id_salt("table_template_choice")
        .width(200.)
        .selected_text(tables::TEMPLATES[*choice])
        .show_ui(ui, |ui| {
            for (i, name) in tables::TEMPLATES.iter().enumerate() {
                ui.horizontal(|ui| {
                    let (r, response) =
                        ui.allocate_exact_size(vec2(54., 30.), egui::Sense::click());
                    let mut t = Table::new(3, 3, [0., 0., 54., 30.]);
                    t.apply_template(i);
                    for cell in 0..9 {
                        let [x, y, right, bottom] = t.cell(cell);
                        let s = t.style(cell);
                        let rect =
                            Rect::from_min_max(r.min + vec2(x, y), r.min + vec2(right, bottom));
                        if let Some(c) = s.fill {
                            ui.painter().rect_filled(rect, 0., rgb(c));
                        }
                        ui.painter().rect_stroke(
                            rect,
                            0.,
                            Stroke::new(0.5, rgb(s.border)),
                            egui::StrokeKind::Inside,
                        );
                    }
                    if response.clicked() || ui.selectable_label(*choice == i, *name).clicked() {
                        *choice = i;
                        ui.close();
                    }
                });
            }
        });
    *choice != before
}
fn rgb(c: [f32; 3]) -> Color32 {
    Color32::from_rgb(
        (c[0] * 255.) as u8,
        (c[1] * 255.) as u8,
        (c[2] * 255.) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;
    fn fixture() -> (Viewer, egui::Context) {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.open_memory(
            PathBuf::from("Table test.pdf"),
            mypdf_milestone0::convert::blank(500., 600.).unwrap(),
            &ctx,
        );
        let until = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        app.editor.mode = editing::Mode::Edit;
        app.home.visible = false;
        (app, ctx)
    }
    fn frame(app: &mut Viewer, ctx: &egui::Context, events: Vec<egui::Event>) {
        app.content_events(ctx);
        app.table_tick(ctx);
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(700., 700.))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    overlay(
                        &mut app.table,
                        &app.tabs[0],
                        ui,
                        0,
                        Rect::from_min_size(pos2(20., 20.), vec2(500., 600.)),
                        1.,
                    );
                });
            },
        );
        if let Some(next) =
            ctx.data_mut(|d| d.remove_temp::<usize>(egui::Id::new("table_next_cell")))
        {
            app.table_commit(ctx, Some(next), false, None);
        }
    }
    fn wait(app: &mut Viewer, ctx: &egui::Context) {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while app.content.busy || app.table.waiting.is_some() {
            frame(app, ctx, vec![]);
            assert!(std::time::Instant::now() < until, "{:?}", app.status);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    fn editor(app: &mut Viewer) {
        let table = Table::new(2, 2, [40., 80., 440., 200.]);
        let key = (app.tabs[0].id, 0, 0);
        app.table.editor = Some(Editor {
            key,
            base: Table {
                cells: vec!["\0".into(); 4],
                ..table.clone()
            },
            table,
            cell: 0,
            text: String::new(),
            confirmed: true,
            owned: true,
            focus: true,
            layer: None,
            scope: 0,
            drag: None,
        });
    }
    #[test]
    fn typing_tab_navigation_and_undo_use_table_transactions() {
        let (mut app, ctx) = fixture();
        editor(&mut app);
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        frame(&mut app, &ctx, vec![egui::Event::Text("İstanbul".into())]);
        assert_eq!(app.table.editor.as_ref().unwrap().text, "İstanbul");
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Tab,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        wait(&mut app, &ctx);
        let e = app.table.editor.as_ref().unwrap();
        assert_eq!(e.cell, 1);
        assert_eq!(e.table.cells[0], "İstanbul");
        assert!(!e.text.contains('\t'));
        let s = app.tabs[0].session.as_mut().unwrap();
        assert_eq!(tables::owned(&s.source, 1).unwrap()[0].cells[0], "İstanbul");
        s.undo();
        assert!(tables::owned(&s.source, 1).unwrap().is_empty());
        s.redo();
        assert_eq!(tables::owned(&s.source, 1).unwrap()[0].cells[0], "İstanbul");
    }
    #[test]
    fn finishing_commits_the_cell_and_close_keeps_invalid_input() {
        let (mut app, ctx) = fixture();
        editor(&mut app);
        let e = app.table.editor.as_mut().unwrap();
        e.text = "Keep my cell".into();
        assert!(app.pending_table(None));
        assert!(app.table.editor.is_some());
        app.table_commit(&ctx, None, true, None);
        wait(&mut app, &ctx);
        assert!(app.table.editor.is_none());
        let table = tables::owned(&app.tabs[0].session.as_ref().unwrap().source, 1)
            .unwrap()
            .remove(0);
        let key = (
            app.tabs[0].id,
            app.tabs[0].session.as_ref().unwrap().revision(),
            0,
        );
        app.table.editor = Some(Editor {
            key,
            base: table.clone(),
            table,
            cell: 0,
            text: "Too much text to fit ".repeat(100),
            confirmed: true,
            owned: true,
            focus: true,
            layer: None,
            scope: 0,
            drag: None,
        });
        app.table_commit(&ctx, None, true, None);
        wait(&mut app, &ctx);
        assert!(app.table.editor.is_some());
        assert!(app.table.editor.as_ref().unwrap().dirty());
        assert!(app.status.as_ref().unwrap().1);
        assert_eq!(
            tables::owned(&app.tabs[0].session.as_ref().unwrap().source, 1).unwrap()[0].cells[0],
            "Keep my cell"
        );
    }
    #[test]
    fn command_save_includes_the_active_table_cell() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, ctx) = fixture();
        editor(&mut app);
        let path = root.path().join("Saved table.pdf");
        let source = app.tabs[0].session.as_ref().unwrap().source.clone();
        std::fs::write(&path, &source).unwrap();
        app.tabs[0].path = path.clone();
        app.tabs[0].has_destination = true;
        app.tabs[0].disk_fingerprint = Some(mypdf_milestone0::file_save::fingerprint(&source));
        app.table.editor.as_mut().unwrap().text = "Saved from the cell".into();
        app.action("save", &ctx);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while app.content.busy || app.table.waiting.is_some() || app.saving {
            app.events(&ctx);
            frame(&mut app, &ctx, vec![]);
            assert!(std::time::Instant::now() < until, "{:?}", app.status);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(app.table.editor.is_none());
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(
            tables::owned(&bytes, 1).unwrap()[0].cells[0],
            "Saved from the cell"
        );
        assert!(!app.tabs[0].session.as_ref().unwrap().dirty());
    }

    fn drag(app: &mut Viewer, ctx: &egui::Context, from: egui::Pos2, delta: egui::Vec2) {
        frame(app, ctx, vec![]);
        frame(
            app,
            ctx,
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
        frame(
            app,
            ctx,
            vec![egui::Event::PointerMoved(from + delta * 0.5)],
        );
        frame(app, ctx, vec![egui::Event::PointerMoved(from + delta)]);
        frame(
            app,
            ctx,
            vec![egui::Event::PointerButton {
                pos: from + delta,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
    }
    #[test]
    fn layer_handles_move_and_resize_then_commit_without_changing_annotate_mode() {
        let (mut app, ctx) = fixture();
        editor(&mut app);
        app.editor.mode = editing::Mode::Annotate;
        app.table.editor.as_mut().unwrap().layer = Some(String::new());
        app.table.editor.as_mut().unwrap().text = "Movable table".into();
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        drag(&mut app, &ctx, pos2(260., 82.), vec2(30., 40.));
        assert_eq!(
            app.table.editor.as_ref().unwrap().table.rect(),
            [70., 120., 470., 240.]
        );
        drag(&mut app, &ctx, pos2(290., 135.), vec2(15., 0.));
        assert_eq!(
            app.table.editor.as_ref().unwrap().table.xs,
            vec![70., 285., 485.]
        );
        drag(&mut app, &ctx, pos2(510., 200.), vec2(0., 20.));
        assert_eq!(
            app.table.editor.as_ref().unwrap().table.ys,
            vec![120., 200., 260.]
        );
        let source = app.tabs[0].session.as_ref().unwrap().source.clone();
        assert!(app.table_commit(&ctx, None, true, None));
        assert!(app.editor.mode == editing::Mode::Annotate);
        let s = app.tabs[0].session.as_mut().unwrap();
        assert_eq!(s.source, source);
        assert_eq!(s.annotations.len(), 1);
        assert_eq!(
            tables::from_layer(&s.annotations[0]).unwrap().cells[0],
            "Movable table"
        );
        s.undo();
        assert!(s.annotations.is_empty());
    }
    #[test]
    fn selected_area_detects_a_table_and_detaches_in_one_transaction() {
        let (mut app, ctx) = fixture();
        let mut table = Table::new(2, 2, [40., 80., 440., 200.]);
        table.cells[0] = "Detach me".into();
        let bytes =
            tables::write(&app.tabs[0].session.as_ref().unwrap().source, 1, &table).unwrap();
        app.tabs[0].session = Some(mypdf_milestone0::annotations::Session::new(
            bytes.clone().into(),
            vec![],
            true,
        ));
        let key = (app.tabs[0].id, 0, 0);
        app.content.table_selection = Some((key, [30., 70., 450., 210.]));
        app.table_from_selection();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while app.table.editor.is_none() {
            frame(&mut app, &ctx, vec![]);
            assert!(std::time::Instant::now() < until, "{:?}", app.status);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        app.detach_table(&ctx);
        wait(&mut app, &ctx);
        assert!(
            app.table.editor.as_ref().unwrap().layer.is_some(),
            "{:?}",
            app.status
        );
        let s = app.tabs[0].session.as_mut().unwrap();
        assert_eq!(s.annotations.len(), 1);
        assert!(tables::owned(&s.source, 1).unwrap().is_empty());
        s.undo();
        assert_eq!(s.source.as_ref(), bytes.as_slice());
        assert!(s.annotations.is_empty());
    }

    #[test]
    fn selecting_a_table_area_works_in_the_complete_desktop_layout() {
        let (mut app, ctx) = fixture();
        let mut table = Table::new(2, 2, [40., 80., 440., 200.]);
        table.cells[0] = "Name".into();
        table.cells[1] = "Status".into();
        table.cells[2] = "İstanbul".into();
        let bytes =
            tables::write(&app.tabs[0].session.as_ref().unwrap().source, 1, &table).unwrap();
        let mut doc = lopdf::Document::load_mem(&bytes).unwrap();
        let page = doc.get_pages()[&1];
        let mut ops = doc.get_and_decode_page_content(page).unwrap();
        for op in &mut ops.operations {
            if let Some(d) = op.operands.get_mut(1).and_then(|o| o.as_dict_mut().ok()) {
                d.remove(b"MyPDFTable");
            }
        }
        let stream = doc.add_object(lopdf::Stream::new(
            lopdf::Dictionary::new(),
            ops.encode().unwrap(),
        ));
        doc.get_dictionary_mut(page)
            .unwrap()
            .set("Contents", stream);
        let bytes = mypdf_milestone0::content::serialize(&mut doc).unwrap();
        app.tabs[0].session = Some(mypdf_milestone0::annotations::Session::new(
            bytes.into(),
            vec![],
            true,
        ));
        let frame = |app: &mut Viewer, events| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 900.))),
                    events,
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
        };
        for _ in 0..5 {
            frame(&mut app, vec![]);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while app.content.loading || app.content.inspection.is_none() {
            frame(&mut app, vec![]);
            assert!(
                std::time::Instant::now() < deadline,
                "Page inspection did not finish"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        app.content.area_mode = true;
        let tab = &app.tabs[0];
        let (view, offset, _) = tab.document_view.unwrap();
        let layout = tab.document_layout.as_ref().unwrap();
        let origin = view.min - offset + layout.rects[0].min.to_vec2();
        let start = origin + vec2(30., 70.) * layout.scale;
        let end = origin + vec2(450., 210.) * layout.scale;
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerMoved(end),
                egui::Event::PointerButton {
                    pos: end,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ],
        );
        assert!(
            app.content.table_selection.is_some(),
            "paused={} loading={} area={} editor={} origin={origin:?} view={view:?}",
            app.content.paused,
            app.content.loading,
            app.content.area_mode,
            app.table.editor.is_some()
        );
    }

    fn desktop_frame(app: &mut Viewer, ctx: &egui::Context, events: Vec<egui::Event>) {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1200., 900.))),
                events,
                ..Default::default()
            },
            |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
        );
    }
    fn page_point(app: &Viewer, p: [f32; 2]) -> egui::Pos2 {
        let tab = &app.tabs[app.active];
        let (view, offset, _) = tab.document_view.unwrap();
        let layout = tab.document_layout.as_ref().unwrap();
        view.min - offset + layout.rects[0].min.to_vec2() + vec2(p[0], p[1]) * layout.scale
    }
    fn click_events(pos: egui::Pos2) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            },
        ]
    }
    fn open_table_fixture(layer: bool) -> (Viewer, egui::Context) {
        let (mut app, ctx) = fixture();
        let bytes = mypdf_milestone0::content::add_text(
            &app.tabs[0].session.as_ref().unwrap().source,
            1,
            "Other text",
            [50., 330., 350., 40.],
            16.,
            [0.; 3],
        )
        .unwrap();
        let mut table = Table::new(2, 2, [40., 80., 440., 200.]);
        table.cells[0] = "Table cell".into();
        let (bytes, marks) = if layer {
            (bytes, vec![tables::layer(&table, 0, None).unwrap()])
        } else {
            (tables::write(&bytes, 1, &table).unwrap(), vec![])
        };
        app.tabs[0].session = Some(mypdf_milestone0::annotations::Session::new(
            bytes.into(),
            marks,
            true,
        ));
        app.editor.mode = if layer {
            editing::Mode::Annotate
        } else {
            editing::Mode::Edit
        };
        app.inspector = true;
        let until = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            desktop_frame(&mut app, &ctx, vec![]);
            if app.content.inspection.is_some()
                && !app.content.loading
                && app.table.rx.is_none()
                && app.table.key.is_some()
            {
                break;
            }
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        app.open_table_at([100., 100.]);
        for _ in 0..3 {
            desktop_frame(&mut app, &ctx, vec![]);
        }
        assert!(app.table.editor.is_some());
        (app, ctx)
    }
    fn desktop_until(app: &mut Viewer, ctx: &egui::Context, ready: impl Fn(&Viewer) -> bool) {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            desktop_frame(app, ctx, vec![]);
            if ready(app) {
                break;
            }
            assert!(std::time::Instant::now() < until, "{:?}", app.status);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    #[test]
    fn clicking_outside_a_clean_table_edits_pdf_text_in_one_click() {
        let (mut app, ctx) = open_table_fixture(false);
        let p = page_point(&app, [100., 342.]);
        desktop_frame(&mut app, &ctx, click_events(p));
        desktop_until(&mut app, &ctx, |a| a.content.existing_text().is_some());
        assert!(app.table.editor.is_none());
        assert_eq!(app.content.existing_text().unwrap(), "Other text");
        assert!(app.editor.mode == editing::Mode::Edit);
    }
    #[test]
    fn outside_click_commits_dirty_embedded_cells_then_opens_the_clicked_text() {
        let (mut app, ctx) = open_table_fixture(false);
        app.table.editor.as_mut().unwrap().text = "Changed cell".into();
        let p = page_point(&app, [100., 342.]);
        desktop_frame(&mut app, &ctx, click_events(p));
        assert!(app.table.waiting.is_some());
        desktop_until(&mut app, &ctx, |a| a.content.existing_text().is_some());
        assert!(app.table.editor.is_none());
        assert!(!app.table.deferring_input());
        assert_eq!(app.content.existing_text().unwrap(), "Other text");
        assert_eq!(
            tables::owned(&app.tabs[0].session.as_ref().unwrap().source, 1).unwrap()[0].cells[0],
            "Changed cell"
        );
    }
    #[test]
    fn outside_area_drag_survives_embedded_commit_and_ignores_later_pointer_motion() {
        let (mut app, ctx) = open_table_fixture(false);
        app.table.editor.as_mut().unwrap().text = "Keep this edit".into();
        let start = page_point(&app, [40., 320.]);
        let end = page_point(&app, [450., 380.]);
        desktop_frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerMoved(end),
                egui::Event::PointerButton {
                    pos: end,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ],
        );
        desktop_frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(pos2(1100., 120.))],
        );
        desktop_until(&mut app, &ctx, |a| a.content.table_selection.is_some());
        assert!(app.table.editor.is_none());
        let (_, area) = app.content.table_selection.unwrap();
        for (a, b) in area.iter().zip([40., 320., 450., 380.]) {
            assert!((a - b).abs() < 0.1, "{area:?}");
        }
        assert_eq!(app.content.existing_text().unwrap(), "Other text");
    }
    #[test]
    fn selecting_an_annotation_tool_finishes_the_layer_and_allows_drawing() {
        let (mut app, ctx) = open_table_fixture(true);
        app.table.editor.as_mut().unwrap().text = "Saved layer cell".into();
        app.editor.select_tool(editing::Tool::Highlight);
        desktop_frame(&mut app, &ctx, vec![]);
        assert!(app.table.editor.is_none());
        assert!(app.editor.mode == editing::Mode::Annotate);
        let start = page_point(&app, [45., 325.]);
        let end = page_point(&app, [220., 365.]);
        desktop_frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        desktop_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(end)]);
        desktop_frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
        let s = app.tabs[0].session.as_ref().unwrap();
        assert_eq!(s.annotations.len(), 2, "{:?}", app.status);
        assert_eq!(
            tables::from_layer(&s.annotations[0]).unwrap().cells[0],
            "Saved layer cell"
        );
        assert!(matches!(
            s.annotations[1].kind,
            mypdf_milestone0::annotations::Kind::Markup { .. }
        ));
    }
    #[test]
    fn switching_to_text_preserves_placement_after_the_embedded_table_write() {
        let (mut app, ctx) = open_table_fixture(false);
        app.table.editor.as_mut().unwrap().text = "Save before text".into();
        let target = (app.tabs[0].id, 0);
        app.content.add_target = Some(target);
        desktop_frame(&mut app, &ctx, vec![]);
        desktop_until(&mut app, &ctx, |a| {
            a.table.editor.is_none() && !a.content.busy
        });
        assert_eq!(app.content.add_target, Some(target));
        assert_eq!(
            tables::owned(&app.tabs[0].session.as_ref().unwrap().source, 1).unwrap()[0].cells[0],
            "Save before text"
        );
    }
    #[test]
    fn failed_handoff_keeps_the_cell_and_releases_competing_tools() {
        let (mut app, ctx) = open_table_fixture(true);
        let invalid = "Too much text ".repeat(500);
        app.table.editor.as_mut().unwrap().text = invalid.clone();
        app.redaction.active = true;
        app.editor.library_open = true;
        desktop_frame(&mut app, &ctx, vec![]);
        assert_eq!(app.table.editor.as_ref().unwrap().text, invalid);
        assert!(!app.redaction.active && !app.editor.library_open);
        assert!(!app.table.deferring_input());
        assert!(app.editor.mode == editing::Mode::Annotate);
        assert!(app.editor.tool == editing::Tool::Select);
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), 0);
    }
    #[test]
    fn stale_delete_and_other_pdf_mutations_preserve_the_pending_table() {
        let (mut app, ctx) = open_table_fixture(false);
        app.table.editor.as_mut().unwrap().text = "Keep my draft".into();
        app.content_change(&ctx, content::Edit::RemoveWatermarks);
        assert!(!app.content.busy);
        assert_eq!(app.table.editor.as_ref().unwrap().text, "Keep my draft");
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), 0);
        let session = app.tabs[0].session.as_mut().unwrap();
        session
            .commit(vec![
                tables::layer(&Table::new(1, 1, [40., 400., 200., 450.]), 0, None).unwrap(),
            ])
            .unwrap();
        let revision = session.revision();
        app.delete_table(&ctx);
        assert!(!app.content.busy);
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), revision);
        assert!(app.table.editor.is_some());
        assert!(app.status.as_ref().unwrap().0.contains("Reopen"));
    }
    #[test]
    fn an_annotation_commit_does_not_turn_a_failed_table_write_into_success() {
        let (mut app, ctx) = open_table_fixture(false);
        let mark = tables::layer(&Table::new(1, 1, [40., 400., 200., 450.]), 0, None).unwrap();
        let session = app.tabs[0].session.as_mut().unwrap();
        session.commit(vec![mark.clone()]).unwrap();
        app.table.editor.as_mut().unwrap().key.1 = session.revision();
        app.tabs[0].selected_annotation = Some(mark.id.clone());
        let mut draft = mark;
        draft.style.opacity = 0.5;
        app.editor.draft = Some(draft);
        let invalid = "Overflow ".repeat(1000);
        app.table.editor.as_mut().unwrap().text = invalid.clone();
        assert!(app.table_commit(&ctx, None, true, None));
        let submitted_revision = app.tabs[0].session.as_ref().unwrap().revision();
        assert_eq!(
            app.table.waiting.as_ref().unwrap().key.1,
            submitted_revision
        );
        desktop_until(&mut app, &ctx, |a| a.table.waiting.is_none());
        assert_eq!(app.table.editor.as_ref().unwrap().text, invalid);
        assert!(app.table.editor.as_ref().unwrap().dirty());
        assert_eq!(
            app.tabs[0].session.as_ref().unwrap().revision(),
            submitted_revision
        );
    }
    #[test]
    fn queued_structure_changes_cannot_mutate_a_table_during_a_write() {
        let (mut app, ctx) = open_table_fixture(false);
        app.table.editor.as_mut().unwrap().text = "Saved cell".into();
        assert!(app.table_commit(&ctx, None, false, None));
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("table_structure"), (true, 0_usize, true)));
        app.table_window(&ctx);
        assert_eq!(app.table.editor.as_ref().unwrap().table.rows(), 2);
        desktop_until(&mut app, &ctx, |a| a.table.waiting.is_none());
        assert_eq!(app.table.editor.as_ref().unwrap().table.rows(), 2);
        assert_eq!(
            app.table.editor.as_ref().unwrap().base.cells[0],
            "Saved cell"
        );
    }
    #[test]
    fn delete_table_removes_whole_layers_or_embedded_grids_and_undo_restores_them() {
        for layer in [false, true] {
            let (mut app, ctx) = open_table_fixture(layer);
            app.table.editor.as_mut().unwrap().text = "Uncommitted change".repeat(200);
            app.delete_table(&ctx);
            desktop_until(&mut app, &ctx, |a| {
                a.table.editor.is_none() && !a.content.busy
            });
            let s = app.tabs[0].session.as_mut().unwrap();
            assert!(s.annotations.is_empty());
            assert!(tables::owned(&s.source, 1).unwrap().is_empty());
            s.undo();
            if layer {
                assert_eq!(s.annotations.len(), 1);
                assert_eq!(
                    tables::from_layer(&s.annotations[0]).unwrap().cells[0],
                    "Table cell"
                );
            } else {
                assert_eq!(
                    tables::owned(&s.source, 1).unwrap()[0].cells[0],
                    "Table cell"
                );
            }
        }
        let (mut app, ctx) = fixture();
        editor(&mut app);
        app.delete_table(&ctx);
        assert!(app.table.editor.is_none());
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), 0);
    }

    #[test]
    fn table_controls_stay_inside_a_compact_edit_ribbon() {
        let (mut app, ctx) = fixture();
        ctx.enable_accesskit();
        for _ in 0..4 {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1100., 760.))),
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
        }
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1100., 760.))),
                ..Default::default()
            },
            |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
        );
        let nodes = out.platform_output.accesskit_update.unwrap().nodes;
        for label in ["Text", "Image", "Table"] {
            let node = nodes
                .iter()
                .find(|(_, n)| n.label() == Some(label) && n.bounds().is_some())
                .unwrap()
                .1
                .clone();
            let r = node.bounds().unwrap();
            assert!(
                r.x0 >= 0. && r.x1 <= 1100. && r.y1 <= 760.,
                "{label}: {r:?}"
            );
        }
        assert!(!nodes.iter().any(|(_, n)| n.label() == Some("Select area")));
    }
}
