//! Resize selected sheets without scaling their contents.
use super::*;
use mypdf_milestone0::pages::Operation;

pub(super) struct Draft {
    tab: u64,
    revision: u64,
    pages: Vec<usize>,
    mm: [f32; 2],
}

const PRESETS: [(&str, [f32; 2]); 5] = [
    ("A4", [210., 297.]),
    ("A3", [297., 420.]),
    ("A5", [148., 210.]),
    ("Letter", [215.9, 279.4]),
    ("Legal", [215.9, 355.6]),
];

impl Viewer {
    pub(super) fn open_page_resize(&mut self) {
        if self.organizer.busy || self.content.busy || !self.page_edits_ready(false) {
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let Some(session) = &tab.session else { return };
        if !session.editable {
            return;
        }
        let pages: Vec<_> = self.organizer.selected.iter().copied().collect();
        let Some(size) = pages.first().and_then(|p| tab.pages.get(*p)) else {
            return;
        };
        self.organizer.resize = Some(Draft {
            tab: tab.id,
            revision: session.revision(),
            pages,
            mm: [size.width * 25.4 / 72., size.height * 25.4 / 72.],
        });
    }

    pub(super) fn page_resize_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.organizer.resize.take() else {
            return;
        };
        let current = self.tabs.get(self.active).is_some_and(|tab| {
            tab.id == draft.tab
                && tab
                    .session
                    .as_ref()
                    .is_some_and(|s| s.revision() == draft.revision)
        });
        if !current {
            return;
        }
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Page size").open(&mut open).collapsible(false)
            .resizable(false).default_width(330.).show(ctx, |ui| {
                let target = if draft.pages.len() == 1 {
                    format!("Page {}", draft.pages[0] + 1)
                } else { format!("{} selected pages", draft.pages.len()) };
                ui.label(target);
                ui.add_space(10.);
                let landscape = draft.mm[0] > draft.mm[1];
                let preset = PRESETS.iter().find(|(_, size)| {
                    let size = if landscape { [size[1], size[0]] } else { *size };
                    (size[0] - draft.mm[0]).abs() < 0.1 && (size[1] - draft.mm[1]).abs() < 0.1
                }).map_or("Custom", |(name, _)| *name);
                ui.horizontal(|ui| {
                    ui.label("Paper");
                    egui::ComboBox::from_id_salt("page_size_preset").selected_text(preset)
                        .show_ui(ui, |ui| {
                            for (name, size) in PRESETS {
                                if ui.selectable_label(preset == name, name).clicked() {
                                    draft.mm = if landscape { [size[1], size[0]] } else { size };
                                }
                            }
                        });
                });
                ui.horizontal(|ui| {
                    if ui.selectable_label(!landscape, "Portrait").clicked() && landscape {
                        draft.mm.swap(0, 1);
                    }
                    if ui.selectable_label(landscape, "Landscape").clicked() && !landscape {
                        draft.mm.swap(0, 1);
                    }
                });
                egui::Grid::new("page_size_dimensions").show(ui, |ui| {
                    for (label, value) in ["Width", "Height"].into_iter().zip(&mut draft.mm) {
                        ui.label(label);
                        let r = ui.add(egui::DragValue::new(value).range(25.4..=5080.)
                            .speed(0.5).max_decimals(2).suffix(" mm"));
                        self.editor.text_editing |= r.has_focus();
                        ui.end_row();
                    }
                });
                ui.add_space(10.);
                ui.label("Content keeps its size and position from the top-left corner. Smaller pages may hide content outside the edges; resizing does not delete it.");
                ui.add_space(12.);
                ui.horizontal(|ui| {
                    apply = ui.add_enabled(!self.organizer.busy && !self.content.busy,
                        ActionButton::new(Icon::Check, "Apply page size").primary()).clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        if apply {
            self.page_change(
                ctx,
                Operation::Resize {
                    pages: draft.pages,
                    width: draft.mm[0] * 72. / 25.4,
                    height: draft.mm[1] * 72. / 25.4,
                },
            );
        } else if open && !cancel {
            self.organizer.resize = Some(draft);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mypdf_milestone0::{convert, pages};

    fn frame(app: &mut Viewer, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1000., 800.))),
                time: Some(ctx.input(|i| i.time) + 0.05),
                events,
                ..Default::default()
            },
            |ctx| {
                app.events(ctx);
                app.page_events(ctx);
                app.page_dialogs(ctx);
            },
        )
    }
    fn click(app: &mut Viewer, ctx: &egui::Context, label: &str) {
        frame(app, ctx, vec![]);
        let output = frame(app, ctx, vec![]);
        let nodes = output.platform_output.accesskit_update.unwrap().nodes;
        let r = nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label))
            .unwrap()
            .1
            .bounds()
            .unwrap();
        let pos = pos2(((r.x0 + r.x1) / 2.) as f32, ((r.y0 + r.y1) / 2.) as f32);
        for pressed in [true, false] {
            frame(
                app,
                ctx,
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
    fn settle(app: &mut Viewer, ctx: &egui::Context) {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            frame(app, ctx, vec![]);
            // Undo starts an asynchronous reload while retaining the session.
            // Wait until editing is enabled before clicking the next action.
            if !app.organizer.busy && app.tabs[0].session.as_ref().is_some_and(|s| s.editable) {
                break;
            }
            assert!(std::time::Instant::now() < until, "{:?}", app.status);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    #[test]
    fn page_size_dialog_applies_to_selection_and_blank_insert_defaults_to_a4() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let bytes = pages::apply(
            &convert::blank(300., 400.).unwrap(),
            &Operation::InsertBlank {
                at: 1,
                width: 500.,
                height: 600.,
            },
        )
        .unwrap();
        app.open_memory("Resize test.pdf".into(), bytes.clone(), &ctx);
        settle(&mut app, &ctx);
        app.organizer.use_tab(&app.tabs[0]);
        app.organizer.selected = std::collections::BTreeSet::from([1]);
        app.open_page_resize();
        app.organizer.resize.as_mut().unwrap().mm = [210., 297.];
        click(&mut app, &ctx, "Apply page size");
        settle(&mut app, &ctx);
        let source = app.tabs[0].session.as_ref().unwrap().source.clone();
        let geometry = pages_geometry(&source);
        assert_eq!(geometry[0], [300., 400.]);
        assert_eq!(geometry[1], pages::A4);
        app.editor_undo(false);
        settle(&mut app, &ctx);
        assert_eq!(app.tabs[0].session.as_ref().unwrap().source.as_ref(), bytes);
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("insert_pages_open"), true));
        click(&mut app, &ctx, "Blank page · A4");
        settle(&mut app, &ctx);
        let geometry = pages_geometry(&app.tabs[0].session.as_ref().unwrap().source);
        assert_eq!(geometry.len(), 3);
        assert!(geometry.contains(&pages::A4));
    }
    fn pages_geometry(bytes: &[u8]) -> Vec<[f32; 2]> {
        mypdf_milestone0::annotations::page_geometry(bytes)
            .unwrap()
            .into_iter()
            .map(|(b, _)| [b[2], b[3]])
            .collect()
    }
}
