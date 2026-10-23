use super::*;
use mypdf_milestone0::navigation_edit::{self, Bookmark, LabelRule, NumberStyle};
pub(super) struct Draft {
    key: (u64, u64),
    count: usize,
    bookmarks: Vec<Bookmark>,
    original: Vec<Bookmark>,
    labels: Vec<LabelRule>,
    original_labels: Vec<LabelRule>,
    selected: Option<usize>,
    tab: bool,
    error: Option<String>,
}
/// A selection creates a page bookmark, with the extracted passage as its
/// editable title. Existing entries retain their source indices and hierarchy.
pub(super) struct SelectionBookmark {
    key: (u64, u64, usize),
    entries: Vec<Bookmark>,
    title: String,
    focus: bool,
}
fn selection_title(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(180)
        .collect()
}
impl Viewer {
    pub(super) fn open_selection_bookmark(&mut self, text: String, key: (u64, u64, usize)) {
        let Some(tab) = self.tabs.get(self.active).filter(|t| t.id == key.0) else {
            return;
        };
        let Some(session) = tab
            .session
            .as_ref()
            .filter(|s| s.editable && s.revision() == key.1)
        else {
            return;
        };
        let title = selection_title(&text);
        if title.is_empty() {
            return;
        }
        match navigation_edit::bookmarks(&session.source) {
            Ok(entries) => {
                self.content.context.bookmark = Some(SelectionBookmark {
                    key,
                    entries,
                    title,
                    focus: true,
                })
            }
            Err(e) => {
                self.status = Some((format!("Bookmarks cannot be safely edited: {e:#}"), true))
            }
        }
    }
    pub(super) fn selection_bookmark_window(&mut self, ctx: &egui::Context) {
        let Some(draft) = self.content.context.bookmark.as_mut() else {
            return;
        };
        let valid = self.tabs.get(self.active).is_some_and(|tab| {
            tab.id == draft.key.0
                && tab.page == draft.key.2
                && tab
                    .session
                    .as_ref()
                    .is_some_and(|s| s.editable && s.revision() == draft.key.1)
        }) && !self.content.busy
            && !self.organizer.busy;
        let mut apply = false;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new("selection_bookmark"))
            .frame(ui::modal_frame())
            .show(ctx, |ui| {
                ui.set_width(340.);
                ui::title(ui, "Bookmark selection", 16.);
                ui.label(
                    RichText::new(format!("Page {}", draft.key.2 + 1))
                        .small()
                        .color(theme::muted()),
                );
                let title = ui.add(
                    egui::TextEdit::singleline(&mut draft.title)
                        .hint_text("Bookmark title")
                        .char_limit(300)
                        .desired_width(f32::INFINITY),
                );
                if std::mem::take(&mut draft.focus) {
                    title.request_focus();
                }
                if !valid {
                    ui.weak("This document is protected or has changed. Close and try again.");
                }
                ui.horizontal(|ui| {
                    cancel = ui.add(ActionButton::new(Icon::Close, "Cancel")).clicked();
                    apply = ui
                        .add_enabled(
                            valid && !draft.title.trim().is_empty(),
                            ActionButton::new(Icon::Outline, "Add bookmark").primary(),
                        )
                        .clicked();
                    apply |= valid
                        && !draft.title.trim().is_empty()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter));
                });
            });
        if apply {
            let mut draft = self.content.context.bookmark.take().unwrap();
            draft.entries.push(Bookmark {
                source: None,
                level: 0,
                title: draft.title.trim().to_owned(),
                page: Some(draft.key.2),
            });
            self.content_change(
                ctx,
                content::Edit::Navigation {
                    bookmarks: Some(draft.entries),
                    labels: None,
                },
            );
            if self.content.busy {
                self.content
                    .name_pending_change("Add bookmark from selection");
            }
        } else if cancel || modal.should_close() {
            self.content.context.bookmark = None;
        }
    }
    pub(super) fn open_navigation_editor(&mut self) {
        if self.unfinished_edit(None) {
            return;
        }
        let Some(t) = self.tabs.get(self.active) else {
            return;
        };
        let Some(s) = &t.session else { return };
        match navigation_edit::bookmarks(&s.source)
            .and_then(|b| Ok((b, navigation_edit::label_rules(&s.source)?)))
        {
            Ok((bookmarks, labels)) => {
                self.navigation_editor = Some(Draft {
                    key: (t.id, s.revision()),
                    count: t.pages.len(),
                    original: bookmarks.clone(),
                    bookmarks,
                    original_labels: labels.clone(),
                    labels,
                    selected: None,
                    tab: false,
                    error: None,
                })
            }
            Err(e) => {
                self.status = Some((format!("Navigation cannot be safely edited: {e:#}"), true))
            }
        }
    }
    pub(super) fn navigation_editor_window(&mut self, ctx: &egui::Context) {
        let Some(d) = &mut self.navigation_editor else {
            return;
        };
        let valid = self.tabs.get(self.active).is_some_and(|t| {
            t.id == d.key.0
                && t.session
                    .as_ref()
                    .is_some_and(|s| s.revision() == d.key.1 && s.editable)
        }) && !self.content.busy
            && !self.organizer.busy;
        let mut close = false;
        let mut apply = false;
        let modal=egui::Modal::new(egui::Id::new("edit_navigation")).frame(ui::modal_frame()).show(ctx,|ui| {
            ui.set_width(620.);ui.heading("Bookmarks & page labels");
            ui.horizontal(|ui|{ui.selectable_value(&mut d.tab,false,"Bookmarks");ui.selectable_value(&mut d.tab,true,"Page labels");});
            ui.separator();
            ui.add_enabled_ui(valid,|ui| {
                if !d.tab {
                    ui.columns(2,|cols| {
                        egui::ScrollArea::vertical().max_height(320.).id_salt("edit_bookmarks").show(&mut cols[0],|ui| {
                            for (i,b) in d.bookmarks.iter().enumerate(){ui.horizontal(|ui|{ui.add_space(b.level.min(8) as f32*12.);
                                if ui.selectable_label(d.selected==Some(i),&b.title).clicked(){d.selected=Some(i);}});}
                            if d.bookmarks.is_empty(){ui.weak("Add a bookmark to organize this document.");}
                        });
                        let ui=&mut cols[1];
                        if let Some(i)=d.selected.filter(|i|*i<d.bookmarks.len()) {
                            let b=&mut d.bookmarks[i];ui.label("Title");ui.add(egui::TextEdit::singleline(&mut b.title).char_limit(300));
                            if let Some(page)=&mut b.page {let mut value=*page+1;ui.horizontal(|ui|{ui.label("Page");if ui.add(egui::DragValue::new(&mut value).range(1..=d.count)).changed(){*page=value-1;}});}else{ui.weak("External destination retained");}
                            if ui.add(ActionButton::new(Icon::Trash,"Delete section")).on_hover_text("Removes this bookmark and its child bookmarks; PDF pages remain").clicked(){let level=b.level;let end=i+1+d.bookmarks[i+1..].iter().take_while(|b|b.level>level).count();d.bookmarks.drain(i..end);d.selected=None;}
                        }
                        if ui.add(ActionButton::new(Icon::Plus,"Add bookmark")).clicked(){d.bookmarks.push(Bookmark{source:None,level:0,title:"New bookmark".into(),page:Some(0)});d.selected=Some(d.bookmarks.len()-1);}
                        if ui.add_enabled(d.selected.is_some(),ActionButton::new(Icon::Plus,"Add subsection")).clicked() && let Some(i)=d.selected {
                            let b=&d.bookmarks[i];let level=b.level;let page=b.page.or(Some(0));let end=i+1+d.bookmarks[i+1..].iter().take_while(|b|b.level>level).count();
                            if level<32{d.bookmarks.insert(end,Bookmark{source:None,level:level+1,title:"New subsection".into(),page});d.selected=Some(end);}
                        }
                    });
                }else{
                    ui.weak("Labels change navigation names, not the printed page content.\nEach section continues until the next section begins.");
                    let mut remove=None;
                    egui::ScrollArea::vertical().max_height(340.).id_salt("label_rules").show(ui,|ui| {
                        for (i,r) in d.labels.iter_mut().enumerate(){ui.push_id(i,|ui| {
                            ui.horizontal(|ui|{
                                ui.label("From page");let mut p=r.page+1;if ui.add(egui::DragValue::new(&mut p).range(1..=d.count)).changed(){r.page=p-1;}
                                egui::ComboBox::from_id_salt("number_style").selected_text(NumberStyle::ALL.iter().find(|(s,_)|*s==r.style).unwrap().1).show_ui(ui,|ui|{for (s,label) in NumberStyle::ALL {ui.selectable_value(&mut r.style,s,label);}});
                                ui.add(egui::TextEdit::singleline(&mut r.prefix).hint_text("Prefix / Cover").char_limit(80).desired_width(120.));
                                ui.add(egui::DragValue::new(&mut r.start).prefix("Start ").range(1..=100000));
                                if ui.add(ActionButton::new(Icon::Trash,"Remove label section").compact()).clicked(){remove=Some(i);}
                            });
                        });}
                    });
                    if let Some(i)=remove {d.labels.remove(i);}
                    if ui.add_enabled(d.labels.len()<d.count,ActionButton::new(Icon::Plus,"Add label section")).clicked(){let page=(0..d.count).find(|p|d.labels.iter().all(|r|r.page!=*p)).unwrap_or(0);d.labels.push(LabelRule{page,style:NumberStyle::Decimal,prefix:String::new(),start:1});}
                }
            });
            if let Some(error)=&d.error{ui.colored_label(theme::danger(),error);}
            if !valid{ui.weak("This document is protected or has changed. Close and reopen this editor.");}
            ui.separator();ui.horizontal(|ui|{close=ui.add(ActionButton::new(Icon::Close,"Cancel")).clicked();apply=ui.add_enabled(valid && (d.bookmarks!=d.original||d.labels!=d.original_labels),ActionButton::new(Icon::Check,"Save changes").primary()).clicked();});
        });
        if apply {
            let d = self.navigation_editor.take().unwrap();
            self.content_change(
                ctx,
                content::Edit::Navigation {
                    bookmarks: (d.bookmarks != d.original).then_some(d.bookmarks),
                    labels: (d.labels != d.original_labels).then_some(d.labels),
                },
            );
        } else if close || modal.should_close() {
            self.navigation_editor = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mypdf_milestone0::convert;

    fn fixture() -> (Viewer, egui::Context) {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        let bytes = convert::blank(500., 500.).unwrap();
        let bytes = navigation_edit::set_bookmarks(
            &bytes,
            &[
                Bookmark {
                    source: None,
                    level: 0,
                    title: "Work".into(),
                    page: Some(0),
                },
                Bookmark {
                    source: None,
                    level: 1,
                    title: "Audit".into(),
                    page: Some(0),
                },
            ],
        )
        .unwrap();
        let bytes = navigation_edit::set_labels(
            &bytes,
            &[LabelRule {
                page: 0,
                style: NumberStyle::RomanLower,
                prefix: "A-".into(),
                start: 3,
            }],
        )
        .unwrap();
        app.open_memory(PathBuf::from("Selection bookmark test.pdf"), bytes, &ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while app.tabs[0].session.is_none() {
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        app.home.visible = false;
        app.editor.mode = editing::Mode::Annotate;
        (app, ctx)
    }

    fn modal_frame(app: &mut Viewer, ctx: &egui::Context, key: Option<egui::Key>) {
        let events = key
            .map(|key| egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            })
            .into_iter()
            .collect();
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(800., 800.))),
                events,
                ..Default::default()
            },
            |ctx| app.selection_bookmark_window(ctx),
        );
    }

    #[test]
    fn selection_bookmark_prefills_then_renames_preserving_hierarchy_labels_and_undo() {
        let (mut app, ctx) = fixture();
        let session = app.tabs[0].session.as_ref().unwrap();
        let before = session.source.clone();
        let key = (app.tabs[0].id, session.revision(), 0);
        app.open_selection_bookmark("  Audit\n  summary\tİstanbul ".into(), key);
        assert_eq!(
            app.content.context.bookmark.as_ref().unwrap().title,
            "Audit summary İstanbul"
        );
        app.content.context.bookmark.as_mut().unwrap().title = "Meeting notes".into();
        modal_frame(&mut app, &ctx, None);
        modal_frame(&mut app, &ctx, Some(egui::Key::Enter));
        assert!(app.content.context.bookmark.is_none());
        assert!(app.content.busy);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while app.content.busy {
            app.content_events(&ctx);
            app.events(&ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let session = app.tabs[0].session.as_mut().unwrap();
        let bookmarks = navigation_edit::bookmarks(&session.source).unwrap();
        assert_eq!(
            bookmarks
                .iter()
                .map(|b| (b.title.as_str(), b.level, b.page))
                .collect::<Vec<_>>(),
            vec![
                ("Work", 0, Some(0)),
                ("Audit", 1, Some(0)),
                ("Meeting notes", 0, Some(0))
            ]
        );
        assert_eq!(
            navigation_edit::label_rules(&session.source).unwrap(),
            navigation_edit::label_rules(&before).unwrap()
        );
        assert_eq!(
            session.undo_label().as_deref(),
            Some("Add bookmark from selection")
        );
        assert_eq!(app.editor.mode, editing::Mode::Annotate);
        session.undo();
        assert_eq!(session.source.as_ref(), before.as_ref());
    }

    #[test]
    fn bookmark_modal_blocks_page_shortcuts_and_native_save_but_retains_title_clipboard() {
        use mypdf_milestone0::annotations::{Annotation, Kind};
        let (mut app, ctx) = fixture();
        let mark = Annotation::new(
            0,
            [40., 60., 100., 100.],
            Kind::Rectangle,
            Default::default(),
        );
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![mark.clone()])
            .unwrap();
        selection::set(&mut app.tabs[0], &mut app.editor, vec![mark.id.clone()]);
        let key = (
            app.tabs[0].id,
            app.tabs[0].session.as_ref().unwrap().revision(),
            0,
        );
        app.open_selection_bookmark("Passage".into(), key);
        let events = [egui::Key::ArrowRight, egui::Key::Delete]
            .map(|key| egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            })
            .to_vec();
        let out = ctx.run(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ctx| {
                app.shortcuts(ctx);
                app.selection_keys(ctx);
                assert!(app.workbench_keys(ctx));
                app.action("delete", ctx);
                app.action("save", ctx);
                app.action("save_as", ctx);
                app.action("copy", ctx);
                app.action("paste", ctx);
            },
        );
        let session = app.tabs[0].session.as_ref().unwrap();
        assert_eq!(session.revision(), key.1);
        assert_eq!(session.annotations, vec![mark]);
        assert!(!app.content.busy);
        assert!(app.content.context.bookmark.is_some());
        let commands = &out.viewport_output[&egui::ViewportId::ROOT].commands;
        assert!(
            commands
                .iter()
                .any(|c| matches!(c, egui::ViewportCommand::RequestCopy))
        );
        assert!(
            commands
                .iter()
                .any(|c| matches!(c, egui::ViewportCommand::RequestPaste))
        );
    }

    #[test]
    fn selection_bookmark_rejects_protected_or_stale_documents_and_cancel_changes_nothing() {
        let (mut app, ctx) = fixture();
        let key = (
            app.tabs[0].id,
            app.tabs[0].session.as_ref().unwrap().revision(),
            0,
        );
        app.tabs[0].session.as_mut().unwrap().editable = false;
        app.open_selection_bookmark("Protected".into(), key);
        assert!(app.content.context.bookmark.is_none());
        app.tabs[0].session.as_mut().unwrap().editable = true;
        app.open_selection_bookmark("Stale".into(), (key.0, key.1 + 1, key.2));
        assert!(app.content.context.bookmark.is_none());
        app.open_selection_bookmark("Cancel me".into(), key);
        modal_frame(&mut app, &ctx, None);
        modal_frame(&mut app, &ctx, Some(egui::Key::Escape));
        assert!(app.content.context.bookmark.is_none());
        assert!(!app.content.busy);
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), key.1);
        assert_eq!(app.editor.mode, editing::Mode::Annotate);
        assert_eq!(selection_title(&"ğ".repeat(400)).chars().count(), 180);
    }
}
