//! Transactional page operations. Inputs are immutable; outputs always remain unsigned copies.
use crate::annotations::{self, Box2};
use anyhow::{Context, Result, ensure};
use hayro::kurbo::{Affine, Point};
use lopdf::{Document, Object, ObjectId, dictionary};
use std::collections::{BTreeSet, HashSet};

#[derive(Clone, Debug)]
pub enum Operation {
    Reorder(Vec<usize>),
    Delete(Vec<usize>),
    InsertBlank {
        at: usize,
        width: f32,
        height: f32,
    },
    InsertPdf {
        at: usize,
        bytes: Vec<u8>,
    },
    Rotate {
        pages: Vec<usize>,
        clockwise: bool,
    },
    /// Displayed-page points, relative to the current crop and rotation.
    Crop {
        pages: Vec<usize>,
        margins: [f32; 4],
    },
    ResetCrop(Vec<usize>),
}

fn read(bytes: &[u8]) -> Result<Document> {
    ensure!(
        bytes.len() as u64 <= crate::viewer::MAX_FILE_BYTES,
        "PDF exceeds 256 MB"
    );
    let doc = Document::load_mem(bytes).context("Could not read PDF pages")?;
    ensure!(
        annotations::inspect(&doc).rewritable(),
        "Page changes require an unsigned, unencrypted PDF. Signed originals are preserved."
    );
    let count = doc.get_pages().len();
    ensure!(
        (1..=10_000).contains(&count),
        "PDF must have 1–10,000 pages"
    );
    Ok(doc)
}
/// A PDF whose pages are only copied, never changed in place: signed PDFs are fine.
fn read_source(bytes: &[u8]) -> Result<Document> {
    ensure!(
        bytes.len() as u64 <= crate::viewer::MAX_FILE_BYTES,
        "PDF exceeds 256 MB"
    );
    let doc = Document::load_mem(bytes).context("Could not read PDF pages")?;
    ensure!(!doc.is_encrypted(), "Encrypted PDFs are not supported");
    let count = doc.get_pages().len();
    ensure!(
        (1..=10_000).contains(&count),
        "PDF must have 1–10,000 pages"
    );
    Ok(doc)
}

// Forms: fields stay with their pages. Deleting pages removes their fields;
// pages copied elsewhere bring their fields, but never their signatures.

/// The document's form dictionary, and its object when it is indirect.
fn acroform(doc: &Document) -> Option<(Option<ObjectId>, lopdf::Dictionary)> {
    match doc.catalog().ok()?.get(b"AcroForm").ok()? {
        Object::Reference(id) => Some((Some(*id), doc.get_dictionary(*id).ok()?.clone())),
        Object::Dictionary(d) => Some((None, d.clone())),
        _ => None,
    }
}
fn set_acroform(doc: &mut Document, id: Option<ObjectId>, form: lopdf::Dictionary) -> Result<()> {
    match id {
        Some(id) => {
            doc.objects.insert(id, Object::Dictionary(form));
        }
        None => doc.catalog_mut()?.set("AcroForm", form),
    }
    Ok(())
}
/// References in an array that may itself be indirect.
fn references(doc: &Document, value: Option<&Object>) -> Vec<ObjectId> {
    value
        .and_then(|v| doc.dereference(v).ok())
        .and_then(|(_, v)| v.as_array().ok())
        .map(|a| a.iter().filter_map(|o| o.as_reference().ok()).collect())
        .unwrap_or_default()
}
/// Widget annotations on these pages.
fn widgets_on(doc: &Document, pages: &[ObjectId]) -> HashSet<ObjectId> {
    pages
        .iter()
        .filter_map(|p| doc.get_dictionary(*p).ok())
        .flat_map(|d| references(doc, d.get(b"Annots").ok()))
        .filter(|id| {
            doc.get_dictionary(*id)
                .and_then(|d| d.get(b"Subtype"))
                .and_then(Object::as_name)
                .is_ok_and(|n| n == b"Widget")
        })
        .collect()
}
/// Whether a widget belongs to a signature field that holds a signature.
fn signed_signature(doc: &Document, widget: ObjectId) -> bool {
    let mut id = Some(widget);
    let mut seen = HashSet::new();
    let (mut kind, mut value) = (None, false);
    while let Some(current) = id.filter(|i| seen.insert(*i)) {
        let Ok(d) = doc.get_dictionary(current) else {
            break;
        };
        kind = kind.or_else(|| {
            d.get(b"FT")
                .and_then(Object::as_name)
                .ok()
                .map(<[u8]>::to_vec)
        });
        value |= d.get(b"V").is_ok_and(|v| !matches!(v, Object::Null));
        id = d.get(b"Parent").and_then(Object::as_reference).ok();
    }
    kind.as_deref() == Some(b"Sig") && value
}
/// Take signed signature widgets off these pages; they would claim a signature
/// that no longer covers the document. Returns the widgets taken off.
fn drop_signatures(doc: &mut Document, pages: &[ObjectId]) -> Result<HashSet<ObjectId>> {
    let signed: HashSet<ObjectId> = widgets_on(doc, pages)
        .into_iter()
        .filter(|w| signed_signature(doc, *w))
        .collect();
    if signed.is_empty() {
        return Ok(signed);
    }
    for &page in pages {
        let kept: Vec<Object> = references(doc, doc.get_dictionary(page)?.get(b"Annots").ok())
            .into_iter()
            .filter(|a| !signed.contains(a))
            .map(Object::Reference)
            .collect();
        doc.get_dictionary_mut(page)?.set("Annots", kept);
    }
    Ok(signed)
}
/// Remove fields whose widgets are all gone, keeping the rest of the form tree.
fn prune_fields(doc: &mut Document, gone: &HashSet<ObjectId>) -> Result<()> {
    fn keep(doc: &mut Document, id: ObjectId, gone: &HashSet<ObjectId>, depth: usize) -> bool {
        if gone.contains(&id) || depth > 32 {
            return false;
        }
        let Some(kids) = doc
            .get_dictionary(id)
            .ok()
            .and_then(|d| d.get(b"Kids").ok())
            .map(|k| references(doc, Some(k)))
        else {
            return true;
        };
        let kept: Vec<Object> = kids
            .into_iter()
            .filter(|k| keep(doc, *k, gone, depth + 1))
            .map(Object::Reference)
            .collect();
        if kept.is_empty() {
            return false;
        }
        if let Ok(d) = doc.get_dictionary_mut(id) {
            d.set("Kids", kept);
        }
        true
    }
    let Some((id, mut form)) = acroform(doc) else {
        return Ok(());
    };
    if gone.is_empty() {
        return Ok(());
    }
    let fields: Vec<Object> = references(doc, form.get(b"Fields").ok())
        .into_iter()
        .filter(|f| keep(doc, *f, gone, 0))
        .map(Object::Reference)
        .collect();
    form.set("Fields", fields);
    set_acroform(doc, id, form)
}
/// A field's own name, as raw PDF text.
fn field_name(doc: &Document, id: ObjectId) -> Option<Vec<u8>> {
    doc.get_dictionary(id)
        .ok()?
        .get(b"T")
        .ok()?
        .as_str()
        .ok()
        .map(<[u8]>::to_vec)
}
/// "Name" becomes "Name (2)", in the name's own text encoding.
fn renamed(name: &[u8], n: usize) -> Vec<u8> {
    let suffix = format!(" ({n})");
    let mut out = name.to_vec();
    if name.starts_with(&[0xFE, 0xFF]) {
        out.extend(suffix.encode_utf16().flat_map(u16::to_be_bytes));
    } else {
        out.extend(suffix.bytes());
    }
    out
}
/// Add another document's top-level fields (already renumbered into `doc`),
/// renaming any whose name is taken so their values stay separate.
fn merge_fields(doc: &mut Document, theirs: &lopdf::Dictionary) -> Result<()> {
    let incoming = references(doc, theirs.get(b"Fields").ok());
    if incoming.is_empty() {
        return Ok(());
    }
    let (id, mut form) = acroform(doc).unwrap_or((None, lopdf::Dictionary::new()));
    let mut fields = references(doc, form.get(b"Fields").ok());
    let mut names: HashSet<Vec<u8>> = fields.iter().filter_map(|f| field_name(doc, *f)).collect();
    for field in incoming {
        if let Some(name) = field_name(doc, field) {
            let mut unique = name.clone();
            let mut n = 2;
            while names.contains(&unique) {
                unique = renamed(&name, n);
                n += 1;
            }
            if unique != name {
                doc.get_dictionary_mut(field)?.set(
                    "T",
                    Object::String(unique.clone(), lopdf::StringFormat::Literal),
                );
            }
            names.insert(unique);
        }
        fields.push(field);
    }
    form.set(
        "Fields",
        fields
            .into_iter()
            .map(Object::Reference)
            .collect::<Vec<_>>(),
    );
    let flags = |d: &lopdf::Dictionary| d.get(b"SigFlags").and_then(Object::as_i64).unwrap_or(0);
    if flags(&form) | flags(theirs) != 0 {
        form.set("SigFlags", flags(&form) | flags(theirs));
    }
    if theirs
        .get(b"NeedAppearances")
        .and_then(Object::as_bool)
        .unwrap_or(false)
    {
        form.set("NeedAppearances", true);
    }
    if form.get(b"DA").is_err()
        && let Ok(da) = theirs.get(b"DA")
    {
        form.set("DA", da.clone());
    }
    // Default resources: add their fonts that ours lack.
    let fonts = |d: &lopdf::Dictionary, doc: &Document| {
        d.get(b"DR")
            .ok()
            .and_then(|r| doc.dereference(r).ok())
            .and_then(|(_, r)| r.as_dict().ok())
            .and_then(|r| r.get(b"Font").ok())
            .and_then(|f| doc.dereference(f).ok())
            .and_then(|(_, f)| f.as_dict().ok())
            .cloned()
    };
    if let Some(their_fonts) = fonts(theirs, doc) {
        let mut our_fonts = fonts(&form, doc).unwrap_or_default();
        for (name, font) in their_fonts.iter() {
            if !our_fonts.has(name) {
                our_fonts.set(name.clone(), font.clone());
            }
        }
        let mut resources = form
            .get(b"DR")
            .ok()
            .and_then(|r| doc.dereference(r).ok())
            .and_then(|(_, r)| r.as_dict().ok())
            .cloned()
            .unwrap_or_default();
        resources.set("Font", our_fonts);
        form.set("DR", resources);
    }
    if id.is_none() && doc.catalog()?.get(b"AcroForm").is_err() {
        doc.catalog_mut()?.set("AcroForm", form);
        return Ok(());
    }
    set_acroform(doc, id, form)
}
fn inherited(doc: &Document, mut id: ObjectId, key: &[u8]) -> Option<Object> {
    let mut seen = HashSet::new();
    while seen.insert(id) {
        let d = doc.get_dictionary(id).ok()?;
        if let Ok(value) = d.get(key) {
            return Some(value.clone());
        }
        id = d.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}
fn materialize(doc: &mut Document) -> Result<Vec<ObjectId>> {
    let ids: Vec<_> = doc.get_pages().into_values().collect();
    for &id in &ids {
        for key in [b"Resources".as_slice(), b"MediaBox", b"CropBox", b"Rotate"] {
            if let Some(value) = inherited(doc, id, key) {
                doc.get_dictionary_mut(id)?.set(key, value);
            }
        }
    }
    Ok(ids)
}
fn serialize(doc: &mut Document) -> Result<Vec<u8>> {
    doc.prune_objects();
    doc.reference_table.cross_reference_type = lopdf::xref::XrefType::CrossReferenceTable;
    doc.trailer.remove(b"Prev");
    doc.trailer.remove(b"XRefStm");
    let mut out = vec![];
    doc.save_to(&mut out)?;
    ensure!(
        out.len() as u64 <= crate::viewer::MAX_FILE_BYTES,
        "Result exceeds 256 MB"
    );
    Ok(out)
}
fn set_tree(doc: &mut Document, ids: &[ObjectId]) -> Result<()> {
    ensure!(
        !ids.is_empty() && ids.len() <= 10_000,
        "Keep between 1 and 10,000 pages"
    );
    let root = doc.add_object(dictionary! { "Type" => "Pages", "Kids" => ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(), "Count" => ids.len() as i64 });
    for &id in ids {
        doc.get_dictionary_mut(id)?.set("Parent", root);
    }
    doc.catalog_mut()?.set("Pages", root);
    // Page labels are positional; keeping the old number tree would mislabel reordered pages.
    doc.catalog_mut()?.remove(b"PageLabels");
    Ok(())
}
fn selection(pages: &[usize], count: usize) -> Result<BTreeSet<usize>> {
    ensure!(!pages.is_empty(), "Select at least one page");
    ensure!(
        pages.iter().all(|p| *p < count),
        "Page selection is out of range"
    );
    let unique: BTreeSet<_> = pages.iter().copied().collect();
    ensure!(
        unique.len() == pages.len(),
        "Page selection contains duplicates"
    );
    Ok(unique)
}
fn number(doc: &Document, o: &Object) -> Result<f32> {
    Ok(doc.dereference(o)?.1.as_float()?)
}
fn page_box(doc: &Document, id: ObjectId, key: &[u8]) -> Result<Box2> {
    let value = inherited(doc, id, key).context("Missing page box")?;
    let values = doc.dereference(&value)?.1.as_array()?;
    ensure!(values.len() == 4, "Invalid page box");
    Ok([
        number(doc, &values[0])?,
        number(doc, &values[1])?,
        number(doc, &values[2])?,
        number(doc, &values[3])?,
    ])
}
fn numbers(r: Box2) -> Object {
    Object::Array(r.into_iter().map(Object::Real).collect())
}

pub fn apply(bytes: &[u8], operation: &Operation) -> Result<Vec<u8>> {
    let mut doc = read(bytes)?;
    let old_geometry = annotations::page_geometry(bytes)?;
    let mut managed = annotations::load(&doc);
    let mut ids = materialize(&mut doc)?;
    ensure!(
        old_geometry.len() == ids.len(),
        "The PDF has inconsistent page trees"
    );
    let mut geometry_changed = false;
    match operation {
        Operation::Reorder(order) => {
            ensure!(
                selection(order, ids.len())?.len() == ids.len(),
                "Reordering must include every page once"
            );
            ids = order.iter().map(|i| ids[*i]).collect();
        }
        Operation::Delete(pages) => {
            let selected = selection(pages, ids.len())?;
            ensure!(
                selected.len() < ids.len(),
                "A PDF must keep at least one page"
            );
            let removed: HashSet<_> = selected.iter().map(|i| ids[*i]).collect();
            let gone = widgets_on(&doc, &removed.iter().copied().collect::<Vec<_>>());
            prune_fields(&mut doc, &gone)?;
            ids = ids
                .into_iter()
                .enumerate()
                .filter_map(|(i, id)| (!selected.contains(&i)).then_some(id))
                .collect();
            remove_navigation(&mut doc, &removed);
        }
        Operation::InsertBlank { at, width, height } => {
            ensure!(*at <= ids.len(), "Insertion position is out of range");
            ensure!(
                width.is_finite()
                    && height.is_finite()
                    && (1.0..=100_000.0).contains(width)
                    && (1.0..=100_000.0).contains(height),
                "Invalid blank page size"
            );
            let id = doc.add_object(dictionary! { "Type" => "Page", "MediaBox" => numbers([0.0,0.0,*width,*height]), "Resources" => dictionary!{} });
            ids.insert(*at, id);
        }
        Operation::InsertPdf {
            at,
            bytes: addition,
        } => {
            ensure!(*at <= ids.len(), "Insertion position is out of range");
            let mut other = read_source(addition)?;
            let their_pages = materialize(&mut other)?;
            // Signatures stay with their own document.
            let dropped = drop_signatures(&mut other, &their_pages)?;
            prune_fields(&mut other, &dropped)?;
            ensure!(
                ids.len() + other.get_pages().len() <= 10_000,
                "Result exceeds 10,000 pages"
            );
            other.renumber_objects_with(doc.max_id + 1);
            let additional: Vec<_> = other.get_pages().into_values().collect();
            // Import page graphs, not the second document's catalog, scripts or field hierarchy.
            for &id in &additional {
                other.get_dictionary_mut(id)?.remove(b"Parent");
            }
            let their_form = acroform(&other).map(|(_, form)| form);
            doc.max_id = other.max_id;
            doc.objects.extend(other.objects);
            if let Some(form) = their_form {
                merge_fields(&mut doc, &form)?;
            }
            ids.splice(*at..*at, additional);
        }
        Operation::Rotate { pages, clockwise } => {
            for i in selection(pages, ids.len())? {
                let current = inherited(&doc, ids[i], b"Rotate")
                    .and_then(|o| doc.dereference(&o).ok().and_then(|(_, o)| o.as_i64().ok()))
                    .unwrap_or(0);
                doc.get_dictionary_mut(ids[i])?.set(
                    "Rotate",
                    (current + if *clockwise { 90 } else { 270 }).rem_euclid(360),
                );
            }
            geometry_changed = true;
        }
        Operation::Crop { pages, margins } => {
            ensure!(
                margins.iter().all(|m| m.is_finite() && *m >= 0.0),
                "Crop margins must be non-negative points"
            );
            for i in selection(pages, ids.len())? {
                let (bounds, inverse) = old_geometry[i];
                let rect = [
                    margins[0],
                    margins[1],
                    bounds[2] - margins[2],
                    bounds[3] - margins[3],
                ];
                ensure!(
                    rect[2] - rect[0] >= 10.0 && rect[3] - rect[1] >= 10.0,
                    "Crop must leave at least 10 × 10 points on every selected page"
                );
                let a = Affine::new(inverse).transform_rect_bbox(hayro::kurbo::Rect::new(
                    rect[0] as f64,
                    rect[1] as f64,
                    rect[2] as f64,
                    rect[3] as f64,
                ));
                doc.get_dictionary_mut(ids[i])?.set(
                    "CropBox",
                    numbers([a.x0 as f32, a.y0 as f32, a.x1 as f32, a.y1 as f32]),
                );
            }
            geometry_changed = true;
        }
        Operation::ResetCrop(pages) => {
            for i in selection(pages, ids.len())? {
                let media = page_box(&doc, ids[i], b"MediaBox")?;
                doc.get_dictionary_mut(ids[i])?
                    .set("CropBox", numbers(media));
            }
            geometry_changed = true;
        }
    }
    set_tree(&mut doc, &ids)?;
    let result = serialize(&mut doc)?;
    if geometry_changed && !managed.is_empty() {
        let new_geometry = annotations::page_geometry(&result)?;
        for a in &mut managed {
            let transform =
                Affine::new(new_geometry[a.page].1).inverse() * Affine::new(old_geometry[a.page].1);
            let center = transform
                * Point::new(
                    ((a.rect[0] + a.rect[2]) / 2.0) as f64,
                    ((a.rect[1] + a.rect[3]) / 2.0) as f64,
                );
            let w = (a.rect[2] - a.rect[0]) / 2.0;
            let h = (a.rect[3] - a.rect[1]) / 2.0;
            a.rect = [
                center.x as f32 - w,
                center.y as f32 - h,
                center.x as f32 + w,
                center.y as f32 + h,
            ];
            let c = transform.as_coeffs();
            a.rotation = (a.rotation + c[1].atan2(c[0]).to_degrees() as f32).rem_euclid(360.0);
        }
        annotations::export(&result, &managed)
    } else {
        Ok(result)
    }
}

// Remove navigation to deleted pages. Keep all remaining bookmarks and external links.
fn remove_navigation(doc: &mut Document, removed: &HashSet<ObjectId>) {
    fn dangling(o: &Object, removed: &HashSet<ObjectId>) -> bool {
        match o {
            Object::Reference(id) => removed.contains(id),
            Object::Array(a) => a.first().is_some_and(|o| dangling(o, removed)),
            _ => false,
        }
    }
    fn visit(o: &mut Object, removed: &HashSet<ObjectId>) {
        match o {
            Object::Dictionary(d) => {
                if d.get(b"Dest").is_ok_and(|o| dangling(o, removed)) {
                    d.remove(b"Dest");
                }
                if d.get(b"S")
                    .and_then(Object::as_name)
                    .is_ok_and(|n| n == b"GoTo")
                    && d.get(b"D").is_ok_and(|o| dangling(o, removed))
                {
                    d.remove(b"D");
                    d.remove(b"S");
                }
                for (_, o) in d.iter_mut() {
                    visit(o, removed);
                }
            }
            Object::Array(a) => {
                for o in a {
                    visit(o, removed);
                }
            }
            Object::Reference(id) if removed.contains(id) => *o = Object::Null,
            _ => {}
        }
    }
    let mut invalid = removed.clone();
    for (&id, object) in &doc.objects {
        if matches!(object, Object::Array(_)) && dangling(object, removed) {
            invalid.insert(id);
        }
    }
    for object in doc.objects.values_mut() {
        visit(object, &invalid);
    }
}

/// A page-only output. The source document and its revision history stay untouched.
pub fn extract(bytes: &[u8], pages: &[usize]) -> Result<Vec<u8>> {
    let mut doc = read_source(bytes)?;
    let ids = materialize(&mut doc)?;
    selection(pages, ids.len())?;
    let kept: Vec<_> = pages.iter().map(|i| ids[*i]).collect();
    let removed: Vec<ObjectId> = ids
        .iter()
        .filter(|id| !kept.contains(id))
        .copied()
        .collect();
    // The copy keeps the form fields on its pages, but not any signatures.
    let mut gone = widgets_on(&doc, &removed);
    gone.extend(drop_signatures(&mut doc, &kept)?);
    prune_fields(&mut doc, &gone)?;
    let form = acroform(&doc).map(|(_, form)| form);
    remove_navigation(&mut doc, &removed.into_iter().collect());
    // A new page-only document does not inherit catalog actions or document-level trees.
    let mut catalog = dictionary! { "Type" => "Catalog" };
    if let Some(form) = form.filter(|f| !references(&doc, f.get(b"Fields").ok()).is_empty()) {
        catalog.set("AcroForm", form);
    }
    let root = doc.add_object(catalog);
    doc.trailer.set("Root", root);
    set_tree(&mut doc, &kept)?;
    serialize(&mut doc)
}

pub fn parse_range(text: &str, count: usize) -> Result<Vec<usize>> {
    let mut result = BTreeSet::new();
    for part in text.split(',') {
        let parts: Vec<_> = part.trim().split('-').collect();
        ensure!(
            parts.len() <= 2,
            "Use page numbers and ranges, for example 1, 3–5 (with a hyphen)"
        );
        let start: usize = parts[0]
            .trim()
            .parse()
            .context("Enter page numbers, for example 1, 3-5")?;
        let end = if parts.len() == 2 {
            parts[1].trim().parse().context("Invalid end page")?
        } else {
            start
        };
        ensure!(
            start >= 1 && end >= start && end <= count,
            "Page range must be between 1 and {count}"
        );
        result.extend(start - 1..end);
    }
    ensure!(!result.is_empty(), "Select at least one page");
    Ok(result.into_iter().collect())
}
