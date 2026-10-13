//! Edit signed PDFs without breaking their signatures.
//!
//! Signed bytes are never rewritten. After the user explicitly unlocks a signed
//! document, edits work on an in-memory copy, and saving appends only the changed
//! objects as an incremental update (ISO 32000-1, 7.5.6). Every signature still
//! verifies for the revision it covers; readers report that the document changed
//! after signing. Removing a signature is an incremental update too.
use anyhow::{Context, Result, ensure};
use lopdf::{Dictionary, Document, IncrementalDocument, Object, ObjectId, Stream, dictionary};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

fn registry() -> &'static Mutex<HashSet<[u8; 32]>> {
    static UNLOCKED: OnceLock<Mutex<HashSet<[u8; 32]>>> = OnceLock::new();
    UNLOCKED.get_or_init(Default::default)
}

/// Signature dictionaries: `/Type /Sig` (or `/DocTimeStamp`) with a byte range.
fn signature_dictionaries(doc: &Document) -> impl Iterator<Item = (ObjectId, &Dictionary)> {
    doc.objects.iter().filter_map(|(id, o)| {
        let d = o.as_dict().ok()?;
        let typed = d
            .get(b"Type")
            .and_then(Object::as_name)
            .is_ok_and(|v| v == b"Sig" || v == b"DocTimeStamp");
        (typed || d.get(b"ByteRange").is_ok() && d.get(b"Contents").is_ok()).then_some((*id, d))
    })
}
/// Each signature is identified by its signature value, which survives rewrites.
fn fingerprints(doc: &Document) -> Vec<[u8; 32]> {
    signature_dictionaries(doc)
        .map(|(_, d)| {
            let contents = d.get(b"Contents").and_then(Object::as_str).unwrap_or(&[]);
            Sha256::digest(contents).into()
        })
        .collect()
}

/// Permit edits to this signed document for the rest of the session. Saving must
/// then use [`incremental_update`] against these bytes.
pub fn unlock(bytes: &[u8]) -> Result<()> {
    let doc = Document::load_mem(bytes)?;
    ensure!(!doc.is_encrypted(), "Encrypted PDFs cannot be edited");
    registry().lock().unwrap().extend(fingerprints(&doc));
    Ok(())
}
/// True when the document is signed and the user unlocked every signature in it.
pub fn is_unlocked(doc: &Document) -> bool {
    let prints = fingerprints(doc);
    let registry = registry().lock().unwrap();
    !prints.is_empty() && prints.iter().all(|p| registry.contains(p))
}

/// Objects equal for saving purposes; a stream's file offset is not content.
fn same(a: &Object, b: &Object) -> bool {
    match (a, b) {
        (Object::Stream(x), Object::Stream(y)) => x.dict == y.dict && x.content == y.content,
        _ => a == b,
    }
}
fn container(object: &Object) -> bool {
    object
        .type_name()
        .is_ok_and(|name| [b"ObjStm".as_slice(), b"XRef".as_slice()].contains(&name))
}

/// Append `edited`'s changes to `original` as an incremental update, so the
/// original bytes (and the signatures over them) stay intact.
pub fn incremental_update(original: &[u8], edited: &[u8]) -> Result<Vec<u8>> {
    let prev = Document::load_mem(original)?;
    let next = Document::load_mem(edited)?;
    ensure!(!prev.is_encrypted(), "Encrypted PDFs cannot be edited");
    let mut update = IncrementalDocument::create_from(original.to_vec(), prev.clone());
    let mut changed = 0;
    for (id, object) in &next.objects {
        if container(object) || prev.objects.get(id).is_some_and(|old| same(old, object)) {
            continue;
        }
        update.new_document.objects.insert(*id, object.clone());
        changed += 1;
    }
    for key in [b"Root".as_slice(), b"Info"] {
        if let Ok(value) = next.trailer.get(key) {
            update.new_document.trailer.set(key, value.clone());
        }
    }
    update.new_document.max_id = prev.max_id.max(next.max_id);
    if changed == 0 {
        return Ok(original.to_vec());
    }
    let mut output = vec![];
    update.save_to(&mut output)?;
    ensure!(output.starts_with(original), "The signed bytes changed");
    verify_preserved(original, &output)?;
    Ok(output)
}

/// Every signature that verified before still verifies for its revision,
/// unless the update deliberately cleared its field (see [`remove_signature`]).
fn verify_preserved(original: &[u8], output: &[u8]) -> Result<()> {
    let ok = |bytes: &[u8]| -> Result<HashSet<String>> {
        Ok(crate::signing::verify(bytes, None)?
            .into_iter()
            .filter(|s| s.crypto_valid && s.digest_matches && s.byte_range_valid)
            .map(|s| s.field)
            .collect())
    };
    let cleared: HashSet<String> = signature_fields(output)?
        .into_iter()
        .filter(|f| !f.signed)
        .map(|f| f.name)
        .collect();
    let before = ok(original)?;
    let after = ok(output)?;
    ensure!(
        before.difference(&cleared).all(|f| after.contains(f)),
        "The update would invalidate an existing signature"
    );
    Ok(())
}

/// A signature form field and where its widget sits on the page.
#[derive(Clone, Debug, PartialEq)]
pub struct SignatureField {
    pub name: String,
    /// Zero-based page index of the widget, when it has one.
    pub page: Option<usize>,
    /// Displayed page coordinates (left, top, right, bottom); `None` when invisible.
    pub rect: Option<[f32; 4]>,
    pub signed: bool,
}

/// The field's own name, decoded as the signature verifier reports it.
fn field_name(dict: &Dictionary) -> String {
    dict.get(b"T")
        .and_then(Object::as_str)
        .map(|t| String::from_utf8_lossy(t).into_owned())
        .unwrap_or_default()
}
/// Signature fields with their widgets: `(field id, field, widget ids)`.
fn signature_widgets(doc: &Document) -> Vec<(ObjectId, Dictionary, Vec<ObjectId>)> {
    let Ok(fields) = doc
        .catalog()
        .and_then(|c| c.get(b"AcroForm"))
        .and_then(|a| doc.dereference(a))
        .and_then(|(_, a)| a.as_dict())
        .and_then(|a| a.get(b"Fields"))
        .and_then(|f| doc.dereference(f))
        .and_then(|(_, f)| f.as_array())
        .cloned()
    else {
        return vec![];
    };
    let mut out = vec![];
    let mut stack: Vec<Object> = fields;
    let mut seen = HashSet::new();
    while let Some(entry) = stack.pop() {
        let Ok(id) = entry.as_reference() else {
            continue;
        };
        if !seen.insert(id) || seen.len() > 10_000 {
            continue;
        }
        let Ok(dict) = doc.get_dictionary(id) else {
            continue;
        };
        let kids: Vec<ObjectId> = dict
            .get(b"Kids")
            .and_then(Object::as_array)
            .map(|k| k.iter().filter_map(|o| o.as_reference().ok()).collect())
            .unwrap_or_default();
        let is_sig = dict
            .get(b"FT")
            .and_then(Object::as_name)
            .is_ok_and(|ft| ft == b"Sig");
        if is_sig {
            // Widgets are the field itself (merged) or kids without their own /T.
            let widgets = if kids.is_empty() {
                vec![id]
            } else {
                kids.clone()
            };
            out.push((id, dict.clone(), widgets));
        } else {
            stack.extend(kids.into_iter().map(Object::Reference));
        }
    }
    out
}

/// Signature fields in the document, signed or not, with their page positions.
pub fn signature_fields(bytes: &[u8]) -> Result<Vec<SignatureField>> {
    let doc = Document::load_mem(bytes)?;
    let geometry = crate::annotations::page_geometry(bytes)?;
    let pages: Vec<ObjectId> = doc.get_pages().into_values().collect();
    Ok(signature_widgets(&doc)
        .into_iter()
        .map(|(_, field, widgets)| {
            let widget = widgets
                .first()
                .and_then(|w| doc.get_dictionary(*w).ok())
                .unwrap_or(&field);
            let page = widget
                .get(b"P")
                .and_then(Object::as_reference)
                .ok()
                .and_then(|p| pages.iter().position(|id| *id == p))
                .or_else(|| {
                    // Widgets without /P: find the page whose /Annots lists them.
                    let widget_id = widgets.first()?;
                    pages.iter().position(|page| {
                        doc.get_dictionary(*page)
                            .ok()
                            .and_then(|d| d.get(b"Annots").ok())
                            .and_then(|a| doc.dereference(a).ok())
                            .and_then(|(_, a)| a.as_array().ok())
                            .is_some_and(|a| {
                                a.iter().any(|o| o.as_reference().ok() == Some(*widget_id))
                            })
                    })
                });
            let rect = widget
                .get(b"Rect")
                .and_then(Object::as_array)
                .ok()
                .map(|r| {
                    r.iter()
                        .filter_map(|v| v.as_float().ok())
                        .collect::<Vec<_>>()
                })
                .filter(|r| r.len() == 4 && (r[2] - r[0]).abs() > 0.5 && (r[3] - r[1]).abs() > 0.5)
                .zip(page.and_then(|p| geometry.get(p)))
                .map(|(r, (_, transform))| {
                    crate::content::displayed_rect([r[0], r[1], r[2], r[3]], *transform)
                });
            SignatureField {
                name: field_name(&field),
                page,
                rect,
                signed: field.get(b"V").is_ok(),
            }
        })
        .collect())
}

fn signature_dictionary<'a>(
    doc: &'a Document,
    name: &str,
) -> Result<(ObjectId, ObjectId, &'a Dictionary)> {
    let (field_id, field, _) = signature_widgets(doc)
        .into_iter()
        .find(|(_, f, _)| field_name(f) == name)
        .with_context(|| format!("Signature field “{name}” not found"))?;
    let value = field.get(b"V").context("That field is not signed")?;
    let sig_id = value
        .as_reference()
        .context("Inline signature values are not supported")?;
    Ok((field_id, sig_id, doc.get_dictionary(sig_id)?))
}

/// The document exactly as it was when `field` was signed.
pub fn signed_revision(bytes: &[u8], field: &str) -> Result<Vec<u8>> {
    let doc = Document::load_mem(bytes)?;
    let (_, _, sig) = signature_dictionary(&doc, field)?;
    let range: Vec<i64> = sig
        .get(b"ByteRange")?
        .as_array()?
        .iter()
        .map(|v| v.as_i64())
        .collect::<std::result::Result<_, _>>()?;
    ensure!(range.len() == 4, "Malformed signature byte range");
    let end = usize::try_from(range[2] + range[3]).context("Malformed signature byte range")?;
    ensure!(end <= bytes.len(), "The signed revision is incomplete");
    Ok(bytes[..end].to_vec())
}

/// Clear a signature from its field, as an incremental update. Other signatures
/// keep verifying; the field becomes an empty signature field that can be signed again.
pub fn remove_signature(bytes: &[u8], field: &str) -> Result<Vec<u8>> {
    let doc = Document::load_mem(bytes)?;
    ensure!(!doc.is_encrypted(), "Encrypted PDFs cannot be edited");
    let (field_id, sig_id, _) = signature_dictionary(&doc, field)?;
    let widgets = signature_widgets(&doc)
        .into_iter()
        .find(|(id, _, _)| *id == field_id)
        .map(|(_, _, w)| w)
        .unwrap_or_default();
    let mut update = IncrementalDocument::create_from(bytes.to_vec(), doc.clone());
    let mut next_id = doc.max_id;
    let mut field_dict = doc.get_dictionary(field_id)?.clone();
    field_dict.remove(b"V");
    field_dict.remove(b"Lock");
    update.new_document.set_object(field_id, field_dict);
    // An empty appearance hides the old signature graphic.
    for widget in widgets {
        let mut dict = update
            .new_document
            .get_dictionary(widget)
            .cloned()
            .or_else(|_| doc.get_dictionary(widget).cloned())?;
        let size = dict
            .get(b"Rect")
            .and_then(Object::as_array)
            .ok()
            .map(|r| {
                r.iter()
                    .filter_map(|v| v.as_float().ok())
                    .collect::<Vec<_>>()
            })
            .filter(|r| r.len() == 4)
            .map(|r| ((r[2] - r[0]).abs(), (r[3] - r[1]).abs()))
            .unwrap_or((0., 0.));
        next_id += 1;
        let appearance = (next_id, 0);
        update.new_document.set_object(
            appearance,
            Stream::new(
                dictionary! {
                    "Type" => "XObject",
                    "Subtype" => "Form",
                    "BBox" => vec![0.into(), 0.into(), size.0.into(), size.1.into()],
                },
                vec![],
            ),
        );
        dict.set("AP", dictionary! { "N" => appearance });
        update.new_document.set_object(widget, dict);
    }
    // A certification that pointed at this signature no longer applies.
    let root = doc.trailer.get(b"Root")?.as_reference()?;
    let mut catalog = doc.get_dictionary(root)?.clone();
    let certifies = catalog
        .get(b"Perms")
        .and_then(|p| doc.dereference(p))
        .and_then(|(_, p)| p.as_dict())
        .and_then(|p| p.get(b"DocMDP"))
        .and_then(Object::as_reference)
        .is_ok_and(|r| r == sig_id);
    if certifies {
        catalog.remove(b"Perms");
        update.new_document.set_object(root, catalog);
    }
    update.new_document.set_object(sig_id, Object::Null);
    update.new_document.max_id = next_id;
    let mut output = vec![];
    update.save_to(&mut output)?;
    ensure!(output.starts_with(bytes), "The signed bytes changed");
    let remaining: HashSet<String> = crate::signing::verify(bytes, None)?
        .into_iter()
        .filter(|s| s.field != field && s.crypto_valid && s.digest_matches)
        .map(|s| s.field)
        .collect();
    let after: HashSet<String> = crate::signing::verify(&output, None)?
        .into_iter()
        .filter(|s| s.crypto_valid && s.digest_matches)
        .map(|s| s.field)
        .collect();
    ensure!(
        remaining.is_subset(&after),
        "Removing this signature would invalidate another one"
    );
    ensure!(!after.contains(field), "The signature is still present");
    Ok(output)
}
