//! Explicitly attach an in-memory translation as a standard PDF comment.
use super::*;
use mypdf_milestone0::annotations::{Annotation, Box2, Kind, Style};

pub(super) struct Request {
    pub tab: u64,
    pub revision: u64,
    pub page: usize,
    pub anchor: Option<Box2>,
    pub source_object: Option<String>,
    pub source_language: String,
    pub target_language: String,
    pub text: String,
}

fn note_rect(anchor: Option<Box2>, page: PageSize) -> Result<Box2, String> {
    if !page.width.is_finite() || !page.height.is_finite() || page.width < 1. || page.height < 1. {
        return Err("This page has no usable area for a translation note.".into());
    }
    let size = 24_f32.min(page.width).min(page.height);
    let (x, y) = match anchor {
        Some(rect)
            if rect.iter().all(|v| v.is_finite()) && rect[2] > rect[0] && rect[3] > rect[1] =>
        {
            // Prefer the adjoining margin so the comment icon does not obscure
            // the words it explains. The stored source position never changes.
            if rect[2] + 6. + size <= page.width {
                (rect[2] + 6., rect[1])
            } else if rect[0] - 6. - size >= 0. {
                (rect[0] - 6. - size, rect[1])
            } else {
                (rect[0], rect[3] + 6.)
            }
        }
        Some(_) => {
            return Err("The source text position is no longer valid. Translate it again.".into());
        }
        None => (page.width - size - 18., 18.),
    };
    let x = x.clamp(0., page.width - size);
    let y = y.clamp(0., page.height - size);
    Ok([x, y, x + size, y + size])
}

impl Viewer {
    /// One Undo transaction, using the exact document/page revision captured
    /// when translation began. Calling this is the user's explicit opt-in to
    /// include the translated text in the PDF and its normal recovery snapshots.
    pub(super) fn attach_translation_note(
        &mut self,
        ctx: &egui::Context,
        request: Request,
    ) -> Result<(), String> {
        if self.home.visible {
            return Err("Open the source document before attaching its translation.".into());
        }
        if self.content.busy
            || self.organizer.busy
            || self.saving
            || self.digital.busy
            || self.signature_busy
        {
            return Err(
                "Wait for the current document operation before adding a translation note.".into(),
            );
        }
        if self.workbench.crop.is_some() {
            return Err(
                "Finish or cancel the current crop before adding a translation note.".into(),
            );
        }
        if self.content.pending_text() == Some(request.tab) {
            return Err("Finish or cancel the text edit before adding a translation note.".into());
        }
        if self.pending_form_edits(request.tab) {
            return Err(
                "Apply or discard the pending form edits before adding a translation note.".into(),
            );
        }
        let tab = self
            .tabs
            .get_mut(self.active)
            .ok_or("The source document is no longer open.")?;
        if tab.id != request.tab || tab.page != request.page || tab.error.is_some() {
            return Err(
                "The source page changed. Translate the selected text or page again.".into(),
            );
        }
        let page = *tab
            .pages
            .get(request.page)
            .ok_or("The source page is no longer available.")?;
        let session = tab
            .session
            .as_mut()
            .ok_or("The document is still loading.")?;
        if session.revision() != request.revision {
            return Err("The document changed after translation. Translate it again before attaching a note.".into());
        }
        if !session.editable || tab.signed && tab.signed_original.is_none() {
            return Err("This document is read-only. A translation note cannot be added to a protected or locked signed PDF.".into());
        }
        if let Some(id) = &request.source_object {
            let source = session
                .annotations
                .iter()
                .find(|a| &a.id == id && a.page == request.page)
                .ok_or("The source object is no longer available. Translate it again.")?;
            if source.object.locked || source.object.hidden {
                return Err(
                    "Unlock and show the source object before adding its translation note.".into(),
                );
            }
        }
        if self.editor.draft.as_ref().is_some_and(|draft| {
            session
                .annotations
                .iter()
                .find(|a| a.id == draft.id)
                .is_some_and(|stored| stored != draft)
        }) {
            return Err(
                "Finish the current annotation edit before adding a translation note.".into(),
            );
        }
        let language_valid = |language: &str| {
            !language.trim().is_empty()
                && language.len() <= 80
                && !language.chars().any(char::is_control)
        };
        if !language_valid(&request.source_language) || !language_valid(&request.target_language) {
            return Err(
                "The translation language pair is no longer valid. Translate the text again."
                    .into(),
            );
        }
        let translation = request.text.replace("\r\n", "\n").replace('\r', "\n");
        if translation.trim().is_empty() {
            return Err("Complete a translation before adding it as a note.".into());
        }
        let title = format!(
            "Translation · {} → {}",
            request.source_language.trim(),
            request.target_language.trim()
        );
        let contents = format!("{title}\n\n{}", translation.trim());
        if contents.len() > 8_000 {
            return Err("This translation is too long for one PDF note. Copy it, or translate a smaller selection. No text was attached or truncated.".into());
        }
        if contents
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
        {
            return Err(
                "This translation contains unsupported control characters. Copy it instead.".into(),
            );
        }
        let mut note = Annotation::new(
            request.page,
            note_rect(request.anchor, page)?,
            Kind::Note { text: contents },
            Style {
                color: [0.15, 0.49, 0.85],
                fill: Some([0.86, 0.93, 1.]),
                ..Default::default()
            },
        );
        note.object.name = title;
        note.object.review.author = "MyPDF Translation".into();
        note.object.review.created = mypdf_milestone0::review::timestamp();
        let mut annotations = session.annotations.clone();
        annotations.push(note.clone());
        session.commit(annotations).map_err(|_| {
            "The translation note could not be added. The document was not changed.".to_owned()
        })?;
        session.name_last_change("Add translation note");
        tab.group.clear();
        tab.selected_annotation = Some(note.id.clone());
        let note_id = note.id.clone();
        self.editor.draft = Some(note);
        // The freshly attached comment opens immediately; the icon is only its
        // page anchor, never the sole feedback that text was saved.
        self.open_note_reader(request.tab, &note_id);
        self.status = Some((
            "Translation added as a note · Save to keep it in the PDF.".into(),
            false,
        ));
        ctx.request_repaint();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mypdf_milestone0::{annotations, content, convert};

    fn document() -> (Viewer, egui::Context) {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.open_memory(
            PathBuf::from("Translation note test.pdf"),
            content::add_text(
                &convert::blank(300., 400.).unwrap(),
                1,
                "Original page text.",
                [40., 60., 160., 30.],
                12.,
                [0.; 3],
            )
            .unwrap(),
            &ctx,
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while app.tabs[0].session.is_none() {
            assert!(std::time::Instant::now() < deadline);
            app.events(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.home.visible = false;
        (app, ctx)
    }
    fn request(app: &Viewer) -> Request {
        let tab = &app.tabs[app.active];
        Request {
            tab: tab.id,
            revision: tab.session.as_ref().unwrap().revision(),
            page: tab.page,
            anchor: Some([40., 60., 200., 90.]),
            source_object: None,
            source_language: "English".into(),
            target_language: "Turkish".into(),
            text: "İstanbul’da belge gizliliği korunur.\nStraße / Größe".into(),
        }
    }
    #[test]
    fn note_exports_reloads_and_undoes_without_rewriting_original_text() {
        let (mut app, ctx) = document();
        let before = app.tabs[0].session.as_ref().unwrap().source.clone();
        let original = lopdf::Document::load_mem(&before).unwrap();
        let original_content = original.get_page_content(original.get_pages()[&1]);
        assert!(!original_content.is_empty());
        app.translation.open = true;
        app.inspector = false;
        let request = request(&app);
        app.attach_translation_note(&ctx, request).unwrap();
        let session = app.tabs[0].session.as_ref().unwrap();
        assert_eq!(session.source.as_ref(), before.as_ref());
        assert_eq!(session.annotations.len(), 1);
        let note = session.annotations[0].clone();
        assert_eq!(note.rect, [206., 60., 230., 84.]);
        assert!(!note.object.hidden);
        assert_eq!(app.tabs[0].selected_annotation.as_ref(), Some(&note.id));
        assert_eq!(app.editor.draft.as_ref(), Some(&note));
        assert!(
            !app.translation.open,
            "attachment reveals the actual saved note"
        );
        assert!(
            app.inspector,
            "attachment reveals a previously hidden right panel"
        );
        assert!(session.dirty());
        assert_eq!(session.history().0, 1, "attachment is one Undo step");
        assert_eq!(session.history().1.len(), 2);
        assert_eq!(
            session.undo_label().as_deref(),
            Some("Add translation note")
        );
        let output = annotations::export(&session.source, &session.annotations).unwrap();
        let reloaded = lopdf::Document::load_mem(&output).unwrap();
        assert_eq!(annotations::load(&reloaded), session.annotations);
        let page_id = reloaded.get_pages()[&1];
        assert_eq!(reloaded.get_page_content(page_id), original_content);
        let page_notes = reloaded
            .get_dictionary(page_id)
            .unwrap()
            .get(b"Annots")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(
            page_notes.len(),
            1,
            "note must be linked to the source page"
        );
        let pdf_note = reloaded
            .get_dictionary(page_notes[0].as_reference().unwrap())
            .unwrap();
        assert_eq!(
            pdf_note.get(b"Subtype").unwrap().as_name().unwrap(),
            b"Text"
        );
        let contents = lopdf::decode_text_string(pdf_note.get(b"Contents").unwrap()).unwrap();
        assert_eq!(contents, note.kind.text().unwrap());
        assert_eq!(
            contents,
            "Translation · English → Turkish\n\nİstanbul’da belge gizliliği korunur.\nStraße / Größe"
        );
        let flags = pdf_note.get(b"F").unwrap().as_i64().unwrap();
        assert_eq!(flags & 2, 0, "exported note must not be hidden");
        assert_ne!(flags & 4, 0, "exported note must be printable");
        let appearance = pdf_note
            .get(b"AP")
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"N")
            .unwrap()
            .as_reference()
            .unwrap();
        assert!(reloaded.get_object(appearance).unwrap().as_stream().is_ok());
        assert_ne!(
            viewer::render_page(&before, 0, 1.).unwrap().1,
            viewer::render_page(&output, 0, 1.).unwrap().1,
            "the exported comment icon must actually render"
        );

        app.editor_undo(false);
        assert!(app.editor.draft.is_none());
        assert!(app.tabs[0].selected_annotation.is_none());
        let session = app.tabs[0].session.as_ref().unwrap();
        assert!(session.annotations.is_empty());
        assert_eq!(session.source.as_ref(), before.as_ref());
        assert_eq!(session.history().0, 0);
        assert!(!session.can_undo());
        assert_eq!(
            session.redo_label().as_deref(),
            Some("Add translation note")
        );
        let undone = annotations::export(&session.source, &session.annotations).unwrap();
        assert!(annotations::load(&lopdf::Document::load_mem(&undone).unwrap()).is_empty());
        assert_eq!(
            viewer::render_page(&undone, 0, 1.).unwrap().1,
            viewer::render_page(&before, 0, 1.).unwrap().1
        );
        app.editor_undo(true);
        let session = app.tabs[0].session.as_ref().unwrap();
        assert_eq!(session.annotations, vec![note]);
        assert_eq!(session.history().0, 1);
        assert!(!session.can_redo());
        assert_eq!(session.source.as_ref(), before.as_ref());
        let redone = annotations::export(&session.source, &session.annotations).unwrap();
        assert_eq!(
            annotations::load(&lopdf::Document::load_mem(&redone).unwrap()),
            session.annotations
        );
    }
    #[test]
    fn readonly_signed_stale_busy_locked_and_oversize_requests_leave_document_unchanged() {
        let (mut app, ctx) = document();
        let session = app.tabs[0].session.as_ref().unwrap();
        let source = session.source.clone();
        let revision = session.revision();
        app.tabs[0].session.as_mut().unwrap().editable = false;
        assert!(
            app.attach_translation_note(&ctx, request(&app))
                .unwrap_err()
                .contains("read-only")
        );
        app.tabs[0].session.as_mut().unwrap().editable = true;
        app.tabs[0].signed = true;
        assert!(app.attach_translation_note(&ctx, request(&app)).is_err());
        app.tabs[0].signed = false;
        let mut stale = request(&app);
        stale.revision += 1;
        assert!(
            app.attach_translation_note(&ctx, stale)
                .unwrap_err()
                .contains("changed")
        );
        app.content.busy = true;
        assert!(
            app.attach_translation_note(&ctx, request(&app))
                .unwrap_err()
                .contains("Wait")
        );
        app.content.busy = false;
        let mut large = request(&app);
        large.text = "İ".repeat(4_001);
        assert!(
            app.attach_translation_note(&ctx, large)
                .unwrap_err()
                .contains("too long")
        );
        let session = app.tabs[0].session.as_ref().unwrap();
        assert_eq!(session.source.as_ref(), source.as_ref());
        assert_eq!(session.revision(), revision);
        assert!(session.annotations.is_empty());
        let mut locked = Annotation::new(
            0,
            [40., 60., 200., 90.],
            Kind::Text {
                text: "Original".into(),
            },
            Default::default(),
        );
        locked.object.locked = true;
        app.tabs[0]
            .session
            .as_mut()
            .unwrap()
            .commit(vec![locked.clone()])
            .unwrap();
        let revision = app.tabs[0].session.as_ref().unwrap().revision();
        let mut locked_request = request(&app);
        locked_request.source_object = Some(locked.id.clone());
        assert!(
            app.attach_translation_note(&ctx, locked_request)
                .unwrap_err()
                .contains("Unlock")
        );
        assert_eq!(app.tabs[0].session.as_ref().unwrap().revision(), revision);
        assert_eq!(
            app.tabs[0].session.as_ref().unwrap().annotations,
            vec![locked]
        );
    }
    #[test]
    fn page_note_fallback_and_edge_anchors_stay_within_the_page() {
        let page = PageSize {
            width: 300.,
            height: 400.,
        };
        assert_eq!(note_rect(None, page).unwrap(), [258., 18., 282., 42.]);
        assert_eq!(
            note_rect(Some([0., 390., 300., 400.]), page).unwrap(),
            [0., 376., 24., 400.]
        );
        assert!(note_rect(Some([f32::NAN, 0., 30., 30.]), page).is_err());
    }
}
