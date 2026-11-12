//! Metadata changes use the normal document transaction, save and Undo path.
use super::*;
use mypdf_milestone0::properties::{self, Metadata, Properties};

pub(super) struct Draft {
    tab: u64,
    revision: u64,
    rx: mpsc::Receiver<Result<Properties, String>>,
    properties: Option<Properties>,
    edited: Metadata,
    error: Option<String>,
}
impl Viewer {
    pub(super) fn open_document_properties(&mut self, ctx: &egui::Context) {
        if self.home.visible {
            return;
        }
        let target = self.tabs.get(self.active).map(|t| t.id);
        if self.pending_table(target) {
            return;
        }
        if target.is_some_and(|id| self.content.has_pending_text_change(id)) {
            self.status = Some((
                "Finish the text edit before opening Document Properties.".into(),
                false,
            ));
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let Some(session) = &tab.session else { return };
        let (tx, rx) = mpsc::channel();
        let bytes = session.source.clone();
        let wake = ctx.clone();
        self.document_properties = Some(Draft {
            tab: tab.id,
            revision: session.revision(),
            rx,
            properties: None,
            edited: Metadata::default(),
            error: None,
        });
        std::thread::spawn(move || {
            let result = properties::read(&bytes).map_err(|e| format!("{e:#}"));
            let _ = tx.send(result);
            wake.request_repaint();
        });
    }
    pub(super) fn document_properties_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.document_properties.take() else {
            return;
        };
        if let Ok(result) = draft.rx.try_recv() {
            match result {
                Ok(p) => {
                    draft.edited = p.metadata.clone();
                    draft.properties = Some(p);
                }
                Err(e) => draft.error = Some(e),
            }
        }
        let Some(index) = self.tabs.iter().position(|t| t.id == draft.tab) else {
            return;
        };
        let tab = &self.tabs[index];
        let unchanged = tab
            .session
            .as_ref()
            .is_some_and(|s| s.revision() == draft.revision);
        let writable = tab.session.as_ref().is_some_and(|s| s.editable)
            && !tab.signed
            && tab.signed_original.is_none();
        let available = unchanged && writable && !self.content.busy && !self.organizer.busy;
        let mut apply = false;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("document_properties"))
            .frame(ui::modal_frame()).show(ctx, |ui| {
                ui.set_width(ui::dialog_width(ctx, 560.));
                ui.heading("Document Properties");
                ui.label(tab.name());
                ui.separator();
                egui::ScrollArea::vertical().max_height((ctx.content_rect().height()-220.).max(180.)).show(ui, |ui| {
                    if let Some(p) = &draft.properties {
                        let can_edit = available && p.warning.is_none();
                        ui.strong("Description");
                        egui::Grid::new("metadata_fields").num_columns(2).spacing([14., 8.]).show(ui, |ui| {
                            for (label, value) in properties::LABELS.iter().zip(&mut draft.edited.fields) {
                                let response = ui.label(*label);
                                ui.add_enabled(can_edit, egui::TextEdit::singleline(value).desired_width((ui.available_width()-8.).max(160.)).char_limit(16384)).labelled_by(response.id);
                                ui.end_row();
                            }
                        });
                        ui.add_space(8.);
                        ui.small("Clear a field to remove it. Apply updates the modification date; Save writes changes to the PDF. Undo is available.");
                        if let Some(warning) = &p.warning { ui.colored_label(theme::muted(), warning); }
                        ui.separator();
                        ui.strong("Document details");
                        egui::Grid::new("document_details").num_columns(2).spacing([14., 6.]).show(ui, |ui| {
                            let page = tab.pages.get(tab.page);
                            let size = page.map(|p| format!("{:.1} × {:.1} mm ({:.0} × {:.0} pt)", p.width*25.4/72., p.height*25.4/72., p.width, p.height)).unwrap_or_default();
                            let protection = if tab.signed || tab.signed_original.is_some() { "Digitally signed — metadata is read-only" } else if tab.password.is_some() { "Password protected — unlocked for this session" } else if writable { "No encryption; document editing allowed" } else { "Document is read-only" };
                            for (label, value) in [
                                ("PDF version",p.version.clone()),
                                ("Pages",tab.pages.len().to_string()),
                                ("Document data", format!("{} bytes", tab.session.as_ref().map_or(0, |s| s.source.len()))),
                                ("Current page size",size),
                                ("Created",display_date(&p.created)),
                                ("Modified",display_date(&p.modified)),
                                ("Protection",protection.to_string()),
                                ("Location",if tab.has_destination {tab.path.display().to_string()} else {"Not saved yet".into()}),
                            ] { ui.label(label); ui.add(egui::Label::new(value).wrap().selectable(true)); ui.end_row(); }
                        });
                        if !p.xmp.is_empty() {
                            ui.collapsing("XMP metadata (read-only)", |ui| {
                                let mut text = p.xmp.as_str();
                                ui.add(egui::TextEdit::multiline(&mut text).code_editor().desired_width(f32::INFINITY).desired_rows(8));
                            });
                        }
                    } else if let Some(error) = &draft.error { ui.label(error); }
                    else { ui.spinner(); ui.label("Reading document properties…"); }
                });
                if !unchanged { ui.label("The document changed. Close and reopen Properties to edit the latest version."); }
                else if !writable { ui.label("Metadata editing is unavailable for this document. Signed PDFs retain their original metadata."); }
                else if !available { ui.label("Finish the current operation or table edit before applying metadata changes."); }
                ui.separator();
                ui.horizontal(|ui| {
                    let changed = draft.properties.as_ref().is_some_and(|p| p.metadata != draft.edited && p.warning.is_none());
                    apply = ui.add_enabled(available && changed, egui::Button::new("Apply")).clicked();
                    close = ui.button(if changed {"Cancel"} else {"Close"}).clicked();
                });
            });
        if apply {
            self.active = index;
            self.content_change(ctx, content::Edit::Metadata(draft.edited.clone()));
            if self.content.busy {
                return;
            }
        }
        if !close && !modal.should_close() {
            self.document_properties = Some(draft);
        }
    }
}
fn display_date(value: &str) -> String {
    if value.is_empty() {
        return "Not set".into();
    }
    if let Some(date) = value.strip_prefix("D:")
        && date.len() >= 14
        && date.as_bytes()[..14].iter().all(u8::is_ascii_digit)
    {
        return format!(
            "{}-{}-{} {}:{}:{} {}",
            &date[..4],
            &date[4..6],
            &date[6..8],
            &date[8..10],
            &date[10..12],
            &date[12..14],
            &date[14..]
        );
    }
    value.into()
}
