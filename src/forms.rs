//! Standard AcroForm values with explicit, portable widget appearances.
use crate::{content, form_design};
use anyhow::{Context, Result, ensure};
use lopdf::{Document, Object, ObjectId};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq)]
pub enum FieldKind {
    Text {
        multiline: bool,
        max: Option<usize>,
    },
    Choice {
        options: Vec<(String, String)>,
        editable: bool,
    },
    Button {
        radio: bool,
        states: Vec<String>,
    },
    /// A text field that holds a date in an Acrobat format such as dd.mm.yyyy.
    Date {
        format: String,
    },
    /// A button that shows a picture people choose.
    Image,
    Signature {
        signed: bool,
    },
    Unsupported,
}
#[derive(Clone, Debug)]
pub struct Widget {
    pub id: ObjectId,
    pub page: usize,
    pub rect: [f32; 4],
}
#[derive(Clone, Debug)]
pub struct Field {
    pub id: ObjectId,
    pub name: String,
    pub value: String,
    pub kind: FieldKind,
    pub widgets: Vec<Widget>,
    pub reason: Option<String>,
    pub required: bool,
}
fn inherited(doc: &Document, id: ObjectId, key: &[u8]) -> Option<Object> {
    content::inherited(doc, id, key).ok().flatten()
}
fn string(o: &Object) -> String {
    lopdf::decode_text_string(o).unwrap_or_else(|_| {
        o.as_name()
            .map(|v| String::from_utf8_lossy(v).into_owned())
            .unwrap_or_default()
    })
}
fn rect(d: &lopdf::Dictionary) -> Result<[f32; 4]> {
    let a = d.get(b"Rect")?.as_array()?;
    ensure!(a.len() == 4, "Invalid field rectangle");
    Ok([
        a[0].as_float()?,
        a[1].as_float()?,
        a[2].as_float()?,
        a[3].as_float()?,
    ])
}
fn normal_states(doc: &Document, id: ObjectId) -> Result<Vec<String>> {
    let d = doc.get_dictionary(id)?;
    let ap = doc.dereference(d.get(b"AP")?)?.1.as_dict()?;
    let n = doc.dereference(ap.get(b"N")?)?.1.as_dict()?;
    n.iter()
        .map(|(k, _)| Ok(std::str::from_utf8(k)?.to_owned()))
        .collect()
}
pub fn fields(bytes: &[u8]) -> Result<Vec<Field>> {
    let doc = Document::load_mem(bytes)?;
    let Some(form) = doc.catalog()?.get(b"AcroForm").ok() else {
        return Ok(vec![]);
    };
    let form = doc.dereference(form)?.1.as_dict()?;
    ensure!(
        !form.has(b"XFA"),
        "XFA forms are not supported. Use a standard AcroForm PDF."
    );
    let roots = doc.dereference(form.get(b"Fields")?)?.1.as_array()?;
    let geometry = crate::annotations::page_geometry(bytes)?;
    let mut pages = HashMap::new();
    for (n, id) in doc.get_pages() {
        if let Ok(a) = doc.get_dictionary(id)?.get(b"Annots") {
            for o in doc.dereference(a)?.1.as_array()? {
                if let Ok(id) = o.as_reference() {
                    pages.insert(id, (n - 1) as usize);
                }
            }
        }
    }
    let mut pending: Vec<_> = roots
        .iter()
        .map(|o| o.as_reference().map(|id| (id, String::new())))
        .collect::<Result<_, _>>()?;
    let mut seen = HashSet::new();
    let mut out = vec![];
    while let Some((id, prefix)) = pending.pop() {
        ensure!(
            seen.insert(id) && seen.len() <= 4096,
            "Form tree is cyclic or exceeds 4096 objects"
        );
        let d = doc.get_dictionary(id)?;
        let local = d.get(b"T").map(string).unwrap_or_default();
        let name = if prefix.is_empty() {
            local
        } else if local.is_empty() {
            prefix
        } else {
            format!("{prefix}.{local}")
        };
        let kids = d
            .get(b"Kids")
            .ok()
            .map(|o| doc.dereference(o))
            .transpose()?
            .map(|(_, o)| o.as_array())
            .transpose()?
            .cloned()
            .unwrap_or_default();
        let kids: Vec<_> = kids
            .iter()
            .map(Object::as_reference)
            .collect::<Result<_, _>>()?;
        let children = kids
            .iter()
            .copied()
            .filter(|id| {
                doc.get_dictionary(*id)
                    .is_ok_and(|d| d.has(b"T") || d.has(b"Kids") || d.has(b"FT"))
            })
            .collect::<Vec<_>>();
        if !children.is_empty() {
            for child in children.into_iter().rev() {
                pending.push((child, name.clone()));
            }
            continue;
        }
        let widget_ids = if d
            .get(b"Subtype")
            .and_then(Object::as_name)
            .is_ok_and(|s| s == b"Widget")
        {
            vec![id]
        } else {
            kids
        };
        let mut widgets = vec![];
        let mut reason = None;
        for wid in widget_ids {
            let wd = doc.get_dictionary(wid)?;
            if let Some(&page) = pages.get(&wid) {
                let r = rect(wd)?;
                ensure!(
                    r.iter().all(|v| v.is_finite())
                        && (r[2] - r[0]).abs() >= 1.
                        && (r[3] - r[1]).abs() >= 1.,
                    "Invalid field dimensions"
                );
                let mk = wd
                    .get(b"MK")
                    .ok()
                    .and_then(|o| doc.dereference(o).ok())
                    .and_then(|(_, o)| o.as_dict().ok());
                if mk
                    .and_then(|m| m.get(b"R").ok())
                    .and_then(|o| o.as_i64().ok())
                    .unwrap_or(0)
                    % 360
                    != 0
                {
                    reason = Some("Rotated widget appearances are read-only".into());
                }
                // Date formatting and Acrobat's picture chooser are understood without running them.
                let understood = (!wd.has(b"AA") || form_design::date_format(&doc, id).is_some())
                    && (!wd.has(b"A") || form_design::is_image_field(&doc, id));
                if !understood {
                    reason = Some(
                        "Fields with actions are read-only; scripts are never executed".into(),
                    );
                }
                widgets.push(Widget {
                    id: wid,
                    page,
                    rect: content::displayed_rect(r, geometry[page].1),
                });
            }
        }
        let flags = inherited(&doc, id, b"Ff")
            .and_then(|o| o.as_i64().ok())
            .unwrap_or(0);
        if flags & 1 != 0 {
            reason = Some("Read-only field".into());
        }
        if inherited(&doc, id, b"AA").is_some() && form_design::date_format(&doc, id).is_none() {
            reason = Some("Fields with validation or calculation scripts are read-only".into());
        }
        if widgets.is_empty() {
            reason = Some("This field has no visible page widget".into());
        }
        let kind = match inherited(&doc, id, b"FT")
            .and_then(|o| o.as_name().ok().map(<[u8]>::to_vec))
            .as_deref()
        {
            Some(b"Tx") => {
                if flags & ((1 << 13) | (1 << 20) | (1 << 24) | (1 << 25)) != 0 {
                    reason = Some(
                        "Password, file-select, comb and rich-text fields are read-only".into(),
                    );
                }
                match form_design::date_format(&doc, id) {
                    Some(format) => FieldKind::Date { format },
                    None => FieldKind::Text {
                        multiline: flags & (1 << 12) != 0,
                        max: inherited(&doc, id, b"MaxLen")
                            .and_then(|o| o.as_i64().ok())
                            .map(|v| v.max(0) as usize),
                    },
                }
            }
            Some(b"Ch") => {
                if flags & (1 << 21) != 0 {
                    reason = Some("Multiple-selection choice fields are read-only".into());
                }
                let options = inherited(&doc, id, b"Opt")
                    .and_then(|o| o.as_array().ok().cloned())
                    .unwrap_or_default()
                    .iter()
                    .map(|o| {
                        if let Ok(a) = o.as_array() {
                            (
                                a.first().map(string).unwrap_or_default(),
                                a.get(1).map(string).unwrap_or_default(),
                            )
                        } else {
                            let v = string(o);
                            (v.clone(), v)
                        }
                    })
                    .collect();
                FieldKind::Choice {
                    options,
                    editable: flags & (1 << 18) != 0,
                }
            }
            Some(b"Btn") if flags & (1 << 16) != 0 && form_design::is_image_field(&doc, id) => {
                FieldKind::Image
            }
            Some(b"Sig") => FieldKind::Signature {
                signed: inherited(&doc, id, b"V").is_some(),
            },
            Some(b"Btn") => {
                if flags & (1 << 16) != 0 {
                    reason = Some("Push buttons are read-only".into());
                }
                let mut states = vec![];
                for w in &widgets {
                    match normal_states(&doc, w.id) {
                        Ok(v) => {
                            if !v.iter().any(|s| s == "Off") {
                                reason = Some("Button has no Off appearance".into());
                            }
                            states.extend(v.into_iter().filter(|s| s != "Off"));
                        }
                        Err(_) => reason = Some("Button has no usable state appearances".into()),
                    }
                }
                states.sort();
                states.dedup();
                if states.is_empty() {
                    reason = Some("Button has no on-state appearance".into());
                }
                FieldKind::Button {
                    radio: flags & (1 << 15) != 0,
                    states,
                }
            }
            _ => {
                reason = Some("Unknown field types are read-only here".into());
                FieldKind::Unsupported
            }
        };
        let value = inherited(&doc, id, b"V")
            .as_ref()
            .map(string)
            .unwrap_or_default();
        out.push(Field {
            id,
            name: if name.is_empty() {
                format!("Field {}", id.0)
            } else {
                name
            },
            value,
            kind,
            widgets,
            reason,
            required: flags & 2 != 0,
        });
    }
    out.sort_by_key(|f| {
        (
            f.widgets.first().map(|w| w.page).unwrap_or(usize::MAX),
            f.id,
        )
    });
    Ok(out)
}

pub fn fill(bytes: &[u8], id: ObjectId, value: &str) -> Result<Vec<u8>> {
    ensure!(
        value.len() <= 32_768
            && !value
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t'),
        "Field value is too long or contains control characters"
    );
    let mut doc = content::editable(bytes)?;
    let field = fields(bytes)?
        .into_iter()
        .find(|f| f.id == id)
        .context("Field no longer exists")?;
    ensure!(
        field.reason.is_none(),
        "{}",
        field.reason.unwrap_or_default()
    );
    let stored = match &field.kind {
        FieldKind::Text { multiline, max } => {
            ensure!(
                *multiline || !value.contains(['\n', '\r']),
                "This is a single-line field"
            );
            ensure!(
                max.is_none_or(|m| value.chars().count() <= m),
                "Value exceeds this field's maximum length"
            );
            value.to_owned()
        }
        FieldKind::Date { format } => {
            if value.trim().is_empty() {
                String::new()
            } else {
                let date = crate::form_data::parse_date(format, value)
                    .with_context(|| format!("Enter the date as {format}"))?;
                crate::form_data::format_date(format, date)
            }
        }
        FieldKind::Choice { options, editable } => {
            ensure!(
                *editable || value.is_empty() || options.iter().any(|(v, _)| v == value),
                "Choose one of the field's options"
            );
            value.to_owned()
        }
        FieldKind::Button { states, radio } => {
            ensure!(
                value == "Off" || states.iter().any(|v| v == value),
                "Unknown button state"
            );
            let flags = inherited(&doc, id, b"Ff")
                .and_then(|o| o.as_i64().ok())
                .unwrap_or(0);
            ensure!(
                !*radio || flags & (1 << 14) == 0 || value != "Off",
                "This radio group requires a selection"
            );
            doc.get_dictionary_mut(id)?
                .set("V", Object::Name(value.as_bytes().to_vec()));
            let mut chosen = false;
            for w in &field.widgets {
                if w.id != id {
                    doc.get_dictionary_mut(w.id)?.remove(b"V");
                }
                let has = normal_states(&doc, w.id)?.iter().any(|s| s == value);
                let on = has && (!*radio || !chosen || flags & (1 << 25) != 0);
                chosen |= on;
                doc.get_dictionary_mut(w.id)?.set(
                    "AS",
                    Object::Name(if on { value.as_bytes() } else { b"Off" }.to_vec()),
                );
            }
            return content::serialize(&mut doc);
        }
        FieldKind::Image => anyhow::bail!("Choose a picture for an image field"),
        FieldKind::Signature { .. } => anyhow::bail!("Sign a signature field with Sign"),
        FieldKind::Unsupported => anyhow::bail!("Unsupported form field"),
    };
    for w in &field.widgets {
        if w.id != id {
            doc.get_dictionary_mut(w.id)?.remove(b"V");
        }
    }
    let d = doc.get_dictionary_mut(id)?;
    if stored.is_empty() {
        d.remove(b"V");
    } else {
        d.set("V", crate::annotations::text_string(&stored));
    }
    d.remove(b"RV");
    if let FieldKind::Choice { options, .. } = &field.kind {
        if let Some(i) = options.iter().position(|(v, _)| v == value) {
            d.set("I", vec![Object::Integer(i as i64)]);
        } else {
            d.remove(b"I");
        }
    }
    form_design::refresh(&mut doc, id, true)?;
    content::serialize(&mut doc)
}
