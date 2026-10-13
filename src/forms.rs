//! Standard AcroForm values with explicit, portable widget appearances.
use crate::{content, pdf_font::PdfFont};
use anyhow::{Context, Result, ensure};
use lopdf::{Document, Object, ObjectId, Stream, dictionary};
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
                if wd.has(b"AA") || wd.has(b"A") {
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
        if inherited(&doc, id, b"AA").is_some() {
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
                FieldKind::Text {
                    multiline: flags & (1 << 12) != 0,
                    max: inherited(&doc, id, b"MaxLen")
                        .and_then(|o| o.as_i64().ok())
                        .map(|v| v.max(0) as usize),
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
                reason = Some("Signature and unknown field types are read-only here".into());
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
    let display = match &field.kind {
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
        FieldKind::Choice { options, editable } => {
            ensure!(
                *editable || value.is_empty() || options.iter().any(|(v, _)| v == value),
                "Choose one of the field's options"
            );
            options
                .iter()
                .find(|(v, _)| v == value)
                .map(|(_, l)| l.clone())
                .unwrap_or_else(|| value.into())
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
        FieldKind::Unsupported => anyhow::bail!("Unsupported form field"),
    };
    let display = display
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\t', "    ");
    let font = PdfFont::embed(&mut doc, &display)?;
    for w in &field.widgets {
        if w.id != id {
            doc.get_dictionary_mut(w.id)?.remove(b"V");
        }
        let r = rect(doc.get_dictionary(w.id)?)?;
        let width = (r[2] - r[0]).abs();
        let height = (r[3] - r[1]).abs();
        let multiline = matches!(
            field.kind,
            FieldKind::Text {
                multiline: true,
                ..
            }
        );
        let mut size = 12.0_f32.min((height - 4.).max(4.));
        let lines = loop {
            let mut lines = vec![];
            for paragraph in display.split('\n') {
                let mut line = String::new();
                for c in paragraph.chars() {
                    let mut candidate = line.clone();
                    candidate.push(c);
                    if multiline && !line.is_empty() && font.width(&candidate, size) > width - 6. {
                        lines.push(std::mem::take(&mut line));
                    }
                    line.push(c);
                }
                lines.push(line);
            }
            if lines.iter().all(|l| font.width(l, size) <= width - 6.)
                && lines.len() as f32 * size * 1.2 <= height - 4.
            {
                break lines;
            }
            size -= 0.5;
            ensure!(
                size >= 4.,
                "The value will not fit in this field. Shorten it before saving."
            );
        };
        let mut body = format!(
            "q 1 1 1 rg 0 0 {width} {height} re f 0.7 G 0.5 w 0.25 0.25 {} {} re S 2 2 {} {} re W n BT /F0 {size} Tf 0 g\n",
            width - 0.5,
            height - 0.5,
            width - 4.,
            height - 4.
        );
        let align = inherited(&doc, id, b"Q")
            .and_then(|o| o.as_i64().ok())
            .unwrap_or(0);
        for (i, line) in lines.iter().enumerate() {
            let textw = font.width(line, size);
            let x = match align {
                1 => (width - textw) / 2.,
                2 => width - textw - 3.,
                _ => 3.,
            };
            let y = if multiline {
                height - size - 3. - i as f32 * size * 1.2
            } else {
                (height - size) / 2. + size * 0.2
            };
            body.push_str(&format!("1 0 0 1 {x} {y} Tm <{}> Tj\n", font.hex(line)?));
        }
        body.push_str("ET Q");
        let mut stream = Stream::new(
            dictionary! {"Type"=>"XObject","Subtype"=>"Form","BBox"=>vec![0.into(),0.into(),width.into(),height.into()],"Resources"=>dictionary!{"Font"=>dictionary!{"F0"=>font.id}}},
            body.into_bytes(),
        );
        stream.compress()?;
        let ap = doc.add_object(stream);
        doc.get_dictionary_mut(w.id)?
            .set("AP", dictionary! {"N"=>ap});
    }
    let d = doc.get_dictionary_mut(id)?;
    d.set("V", crate::annotations::text_string(value));
    d.remove(b"RV");
    if let FieldKind::Choice { options, .. } = &field.kind {
        if let Some(i) = options.iter().position(|(v, _)| v == value) {
            d.set("I", vec![Object::Integer(i as i64)]);
        } else {
            d.remove(b"I");
        }
    }
    content::serialize(&mut doc)
}
