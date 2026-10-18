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
impl Viewer {
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
