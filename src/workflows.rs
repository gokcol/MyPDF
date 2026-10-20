//! Local workflows that produce independent copies, never overwrite source files.
use anyhow::{Result, ensure};
use lopdf::{Dictionary, Object};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BatchOptions {
    pub ocr: bool,
    /// None, or an index into compress::Level::ALL.
    pub compression: Option<usize>,
    pub numbers: bool,
    pub watermark: String,
    pub opacity: f32,
}
impl Default for BatchOptions {
    fn default() -> Self {
        Self {
            ocr: false,
            compression: None,
            numbers: false,
            watermark: String::new(),
            opacity: 0.25,
        }
    }
}
impl BatchOptions {
    pub fn active(&self) -> bool {
        self.ocr || self.compression.is_some() || self.numbers || !self.watermark.trim().is_empty()
    }
}
pub fn process(
    bytes: &[u8],
    options: &BatchOptions,
    cancel: &crate::ocr::Cancellation,
) -> Result<Vec<u8>> {
    ensure!(options.active(), "Choose at least one operation");
    crate::content::editable(bytes)?;
    cancel.check()?;
    let mut result = bytes.to_vec();
    if options.ocr {
        result = crate::ocr::recognize_cancellable(&result, cancel, &|_| {})?.0;
    }
    cancel.check()?;
    if options.numbers {
        let stamp = crate::header_footer::Stamp {
            kind: crate::header_footer::Kind::PageNumber,
            template: "{page} / {pages}".into(),
            position: crate::header_footer::Position::BottomCenter,
            ..Default::default()
        };
        result = crate::header_footer::apply(&result, &stamp)?.0;
    }
    cancel.check()?;
    if !options.watermark.trim().is_empty() {
        ensure!(
            options.watermark.len() <= 256,
            "Watermark is too long (maximum 256 bytes)"
        );
        ensure!(
            options.opacity.is_finite() && (0.05..=1.).contains(&options.opacity),
            "Invalid opacity"
        );
        let watermark = crate::watermark::Watermark {
            mark: crate::watermark::Mark::Text {
                text: options.watermark.clone(),
                color: [0.4; 3],
                font: None,
            },
            opacity: options.opacity,
            ..Default::default()
        };
        result = crate::watermark::apply(&result, &watermark)?.0;
    }
    cancel.check()?;
    if let Some(level) = options.compression {
        let level = crate::compress::Level::ALL
            .get(level)
            .ok_or_else(|| anyhow::anyhow!("Invalid compression quality"))?;
        result = crate::compress::reduce(&result, *level)?.0;
    }
    cancel.check()?;
    Ok(result)
}
/// A separate destination folder is required. Existing files are never replaced.
pub fn batch_output(source: &Path, destination: &Path, bytes: &[u8]) -> Result<PathBuf> {
    let destination = destination.canonicalize()?;
    ensure!(destination.is_dir(), "Choose an output folder");
    ensure!(
        source.canonicalize()?.parent() != Some(destination.as_path()),
        "Choose an output folder different from the source folder"
    );
    let stem = source.file_stem().unwrap_or_default().to_string_lossy();
    for number in 1..=1000 {
        let suffix = if number == 1 {
            String::new()
        } else {
            format!("-{number}")
        };
        let path = destination.join(format!("{stem}-processed{suffix}.pdf"));
        if !path.exists() && !path.is_symlink() {
            crate::write_new(&path, bytes)?;
            return Ok(path);
        }
    }
    anyhow::bail!("Too many copies with the same name")
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShareOptions {
    pub comments: bool,
    pub attachments: bool,
    pub metadata: bool,
}
impl Default for ShareOptions {
    fn default() -> Self {
        Self {
            comments: true,
            attachments: true,
            metadata: true,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct ShareReport {
    pub notes: usize,
    pub comment_fields: usize,
    pub attachment_entries: usize,
    pub metadata_entries: usize,
}
fn remove(dict: &mut Dictionary, key: &[u8], count: &mut usize) {
    if dict.remove(key).is_some() {
        *count += 1;
    }
}
/// Removes note annotations and comment payloads while retaining visible markup,
/// drawings, links and forms. This is sharing cleanup, not content redaction.
pub fn sharing_copy(bytes: &[u8], options: ShareOptions) -> Result<(Vec<u8>, ShareReport)> {
    let mut doc = crate::content::editable(bytes)?;
    let mut report = ShareReport::default();
    // Resolve page arrays first, including indirect annotation arrays.
    for page in doc.get_pages().values().copied().collect::<Vec<_>>() {
        let entries = doc
            .get_dictionary(page)?
            .get(b"Annots")
            .ok()
            .and_then(|o| doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_array().ok())
            .cloned();
        if let Some(entries) = entries {
            let mut kept = vec![];
            for entry in entries {
                let subtype = doc
                    .dereference(&entry)
                    .ok()
                    .and_then(|(_, o)| o.as_dict().ok())
                    .and_then(|d| d.get(b"Subtype").ok())
                    .and_then(|o| o.as_name().ok());
                let note = options.comments && matches!(subtype, Some(b"Text" | b"Popup"));
                let attachment =
                    options.attachments && subtype == Some(b"FileAttachment".as_slice());
                if note {
                    report.notes += 1;
                } else if attachment {
                    report.attachment_entries += 1;
                } else {
                    kept.push(entry);
                }
            }
            doc.get_dictionary_mut(page)?.set("Annots", kept);
        }
    }
    fn scrub(object: &mut Object, options: ShareOptions, report: &mut ShareReport) {
        match object {
            Object::Dictionary(d) => scrub_dict(d, options, report),
            Object::Stream(s) => scrub_dict(&mut s.dict, options, report),
            Object::Array(a) => {
                for item in a {
                    scrub(item, options, report);
                }
            }
            _ => {}
        }
    }
    fn scrub_dict(d: &mut Dictionary, o: ShareOptions, r: &mut ShareReport) {
        let annotation = d.get(b"Type").and_then(Object::as_name).ok() == Some(b"Annot");
        let subtype = d
            .get(b"Subtype")
            .and_then(Object::as_name)
            .ok()
            .map(Vec::from);
        let editable_text = matches!(subtype.as_deref(), Some(b"FreeText" | b"Widget"));
        if o.comments && (annotation || subtype.is_some()) {
            if !editable_text {
                for key in [
                    b"Contents".as_slice(),
                    b"RC",
                    b"IRT",
                    b"Popup",
                    b"State",
                    b"StateModel",
                ] {
                    remove(d, key, &mut r.comment_fields);
                }
            }
            if annotation && !editable_text {
                remove(d, b"T", &mut r.comment_fields);
            }
        }
        if o.comments || o.metadata {
            // Prevent private editable state from resurrecting removed comments.
            for key in [b"MyPDFData".as_slice(), b"MyPDFState", b"MyPDFApDigest"] {
                remove(d, key, &mut r.metadata_entries);
            }
        }
        if o.metadata {
            for key in [b"Metadata".as_slice(), b"PieceInfo", b"LastModified"] {
                remove(d, key, &mut r.metadata_entries);
            }
            if annotation {
                for key in [b"M".as_slice(), b"CreationDate"] {
                    remove(d, key, &mut r.metadata_entries);
                }
                if !editable_text {
                    remove(d, b"T", &mut r.metadata_entries);
                }
            }
        }
        if o.attachments {
            for key in [b"EmbeddedFiles".as_slice(), b"AF", b"EF", b"RF"] {
                remove(d, key, &mut r.attachment_entries);
            }
        }
        for (_, value) in d.iter_mut() {
            scrub(value, o, r);
        }
    }
    for value in doc.objects.values_mut() {
        scrub(value, options, &mut report);
    }
    if options.metadata {
        remove(&mut doc.trailer, b"Info", &mut report.metadata_entries);
    }
    Ok((crate::content::serialize(&mut doc)?, report))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextDefaults {
    pub font: String,
    pub size: f32,
    pub color: [f32; 3],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Template {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub text_defaults: Option<TextDefaults>,
}
fn template_path(root: &Path, id: &str) -> Result<PathBuf> {
    ensure!(
        id.len() == 16 && id.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid template identifier"
    );
    ensure!(!root.is_symlink(), "Template folder cannot be a symlink");
    let path = root.join(format!("{id}.pdf"));
    ensure!(!path.is_symlink(), "Template file cannot be a symlink");
    Ok(path)
}
pub fn save_template(root: &Path, name: &str, bytes: &[u8]) -> Result<Template> {
    ensure!(
        !name.trim().is_empty() && name.chars().count() <= 80,
        "Enter a name of 1–80 characters"
    );
    crate::content::editable(bytes)?;
    let id = format!("{:016x}", rand::random::<u64>());
    let path = template_path(root, &id)?;
    crate::write_new(&path, bytes)?;
    Ok(Template {
        id,
        name: name.trim().into(),
        text_defaults: None,
    })
}
pub fn read_template(root: &Path, template: &Template) -> Result<Vec<u8>> {
    let bytes = crate::convert::read_bounded(
        &template_path(root, &template.id)?,
        crate::viewer::MAX_FILE_BYTES,
    )?;
    crate::content::editable(&bytes)?;
    Ok(bytes)
}
pub fn delete_template(root: &Path, template: &Template) -> Result<()> {
    std::fs::remove_file(template_path(root, &template.id)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{Document, Stream, dictionary};
    #[test]
    fn sharing_removes_payloads_and_preserves_visible_text_and_forms() -> Result<()> {
        let original = crate::convert::blank(300., 400.)?;
        let mut doc = Document::load_mem(&original)?;
        let page = *doc.get_pages().values().next().unwrap();
        let info =
            doc.add_object(dictionary! { "Author" => Object::string_literal("PRIVATE AUTHOR") });
        doc.trailer.set("Info", info);
        let file = doc.add_object(Stream::new(
            Dictionary::new(),
            b"PRIVATE ATTACHMENT".to_vec(),
        ));
        let spec =
            doc.add_object(dictionary! { "Type"=>"Filespec", "EF"=> dictionary! {"F"=>file} });
        doc.catalog_mut()?.set("AF", vec![Object::Reference(spec)]);
        let note = doc.add_object(dictionary! {"Type"=>"Annot", "Subtype"=>"Text", "Contents"=>Object::string_literal("PRIVATE NOTE")});
        let text = doc.add_object(dictionary! {"Type"=>"Annot", "Subtype"=>"FreeText", "Contents"=>Object::string_literal("VISIBLE TEXT"),"MyPDFData"=>Object::string_literal("PRIVATE STATE")});
        let widget = doc.add_object(dictionary! {"Type"=>"Annot", "Subtype"=>"Widget","T"=>Object::string_literal("FieldName"),"V"=>Object::string_literal("Value")});
        doc.get_dictionary_mut(page)?
            .set("Annots", vec![note.into(), text.into(), widget.into()]);
        let bytes = crate::content::serialize(&mut doc)?;
        let (clean, report) = sharing_copy(&bytes, ShareOptions::default())?;
        assert_eq!(report.notes, 1);
        let clean_doc = Document::load_mem(&clean)?;
        assert!(clean_doc.trailer.get(b"Info").is_err());
        assert!(clean_doc.catalog()?.get(b"AF").is_err());
        assert!(!String::from_utf8_lossy(&clean).contains("PRIVATE"));
        assert!(String::from_utf8_lossy(&clean).contains("VISIBLE TEXT"));
        assert!(String::from_utf8_lossy(&clean).contains("FieldName"));
        assert!(String::from_utf8_lossy(&bytes).contains("PRIVATE"));
        Ok(())
    }
    #[test]
    fn batch_and_templates_are_independent_and_do_not_overwrite() -> Result<()> {
        let root = tempfile::tempdir()?;
        let dest = root.path().join("outputs");
        std::fs::create_dir(&dest)?;
        let bytes = crate::convert::blank(300., 400.)?;
        let source = root.path().join("source.pdf");
        crate::write_new(&source, &bytes)?;
        let recipe = BatchOptions {
            numbers: true,
            watermark: "DRAFT".into(),
            compression: Some(0),
            ..Default::default()
        };
        let result = process(&bytes, &recipe, &Default::default())?;
        let runs = crate::document::text_runs(&result, 1)?;
        assert!(runs.iter().any(|r| r.text.contains("DRAFT")));
        assert!(runs.iter().any(|r| r.text.contains("1 / 1")));
        let first = batch_output(&source, &dest, &result)?;
        let second = batch_output(&source, &dest, &result)?;
        assert_ne!(first, second);
        assert_eq!(std::fs::read(&source)?, bytes);
        assert!(batch_output(&source, root.path(), &result).is_err());
        let cancel = crate::ocr::Cancellation::default();
        cancel.cancel();
        assert!(process(&bytes, &recipe, &cancel).is_err());
        let templates = root.path().join("templates");
        let item = save_template(&templates, "My form", &result)?;
        assert_eq!(read_template(&templates, &item)?, result);
        assert!(
            read_template(
                &templates,
                &Template {
                    id: "../source".into(),
                    name: "invalid".into(),
                    text_defaults: None
                }
            )
            .is_err()
        );
        delete_template(&templates, &item)?;
        assert!(first.exists());
        Ok(())
    }
}
