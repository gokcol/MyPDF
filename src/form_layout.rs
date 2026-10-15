//! Form layouts: whole structures placed at once. A table of fields, labelled
//! fields, a checklist, a choice group or a signature block, with their lines
//! and labels drawn into the page and their fields added.
use crate::{
    content,
    form_design::{self, Align, FieldType, Properties},
    pdf_font::PdfFont,
};
use anyhow::{Context, Result, ensure};
use lopdf::{Dictionary, Object, Stream};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// What a table column holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CellKind {
    Text,
    Number,
    Checkbox,
    Date,
    Dropdown,
}
impl CellKind {
    pub const ALL: [CellKind; 5] = [
        CellKind::Text,
        CellKind::Number,
        CellKind::Checkbox,
        CellKind::Date,
        CellKind::Dropdown,
    ];
    pub fn label(self) -> &'static str {
        match self {
            CellKind::Text => "Text",
            CellKind::Number => "Number",
            CellKind::Checkbox => "Check box",
            CellKind::Date => "Date",
            CellKind::Dropdown => "Dropdown",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Column {
    pub title: String,
    pub kind: CellKind,
    /// A dropdown's choices, separated by commas.
    pub choices: String,
    /// Width relative to the other columns.
    pub width: f32,
}
impl Column {
    pub fn new(title: &str, kind: CellKind, width: f32) -> Self {
        Self {
            title: title.into(),
            kind,
            choices: String::new(),
            width,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Layout {
    Table {
        columns: Vec<Column>,
        rows: usize,
        header: bool,
        lines: bool,
    },
    /// A label and its field on each line: Name, Address, …
    Labeled { labels: Vec<String> },
    /// A check box before each item.
    Checklist { items: Vec<String> },
    /// A question answered with one of several radio buttons.
    Choice {
        question: String,
        options: Vec<String>,
        across: bool,
    },
    /// Name, date and signature for each signer.
    Signatures { signers: usize },
}
impl Layout {
    pub fn table() -> Self {
        Layout::Table {
            columns: vec![
                Column::new("Item", CellKind::Text, 2.),
                Column::new("Quantity", CellKind::Number, 1.),
                Column::new("Date", CellKind::Date, 1.),
                Column::new("Done", CellKind::Checkbox, 0.6),
            ],
            rows: 5,
            header: true,
            lines: true,
        }
    }
    pub fn labeled() -> Self {
        Layout::Labeled {
            labels: ["Full name", "Address", "Phone", "Email", "Date"]
                .map(String::from)
                .to_vec(),
        }
    }
    pub fn checklist() -> Self {
        Layout::Checklist {
            items: ["First item", "Second item", "Third item"]
                .map(String::from)
                .to_vec(),
        }
    }
    pub fn choice() -> Self {
        Layout::Choice {
            question: "Your answer".into(),
            options: ["Yes", "No", "Not sure"].map(String::from).to_vec(),
            across: true,
        }
    }
    pub fn signatures() -> Self {
        Layout::Signatures { signers: 2 }
    }
    pub fn title(&self) -> &'static str {
        match self {
            Layout::Table { .. } => "Table",
            Layout::Labeled { .. } => "Labeled fields",
            Layout::Checklist { .. } => "Checklist",
            Layout::Choice { .. } => "Choice group",
            Layout::Signatures { .. } => "Signature block",
        }
    }
    /// The size placed with a click, in points.
    pub fn default_size(&self) -> [f32; 2] {
        match self {
            Layout::Table { rows, header, .. } => {
                [480., if *header { 22. } else { 0. } + *rows as f32 * 22.]
            }
            Layout::Labeled { labels } => [420., labels.len() as f32 * 28.],
            Layout::Checklist { items } => [320., items.len() as f32 * 22.],
            Layout::Choice {
                options, across, ..
            } => {
                if *across {
                    [420., 46.]
                } else {
                    [260., 22. + options.len() as f32 * 22.]
                }
            }
            Layout::Signatures { signers } => [480., *signers as f32 * 58.],
        }
    }
    fn check(&self) -> Result<()> {
        let short = |s: &str| s.chars().count() <= 200;
        match self {
            Layout::Table { columns, rows, .. } => {
                ensure!(
                    (1..=12).contains(&columns.len()) && (1..=60).contains(rows),
                    "A table has 1 to 12 columns and 1 to 60 rows"
                );
                ensure!(
                    columns.iter().all(|c| short(&c.title)
                        && c.choices.len() <= 2000
                        && (0.1..=10.).contains(&c.width)),
                    "A column setting is too long"
                );
            }
            Layout::Labeled { labels } => ensure!(
                (1..=40).contains(&labels.len()) && labels.iter().all(|l| short(l)),
                "Write 1 to 40 labels, one per line"
            ),
            Layout::Checklist { items } => ensure!(
                (1..=60).contains(&items.len()) && items.iter().all(|l| short(l)),
                "Write 1 to 60 items, one per line"
            ),
            Layout::Choice {
                question, options, ..
            } => ensure!(
                (2..=12).contains(&options.len())
                    && short(question)
                    && options.iter().all(|o| short(o)),
                "Write 2 to 12 answers, one per line"
            ),
            Layout::Signatures { signers } => {
                ensure!((1..=8).contains(signers), "Choose 1 to 8 signers")
            }
        }
        Ok(())
    }

    /// Lines, shading, labels and fields for a displayed rectangle.
    pub fn plan(&self, rect: [f32; 4]) -> Result<Plan> {
        self.check()?;
        let [x0, y0, x1, y1] = rect;
        let (w, h) = (x1 - x0, y1 - y0);
        ensure!(w >= 40. && h >= 12., "Make the area larger");
        let mut plan = Plan::default();
        match self {
            Layout::Table {
                columns,
                rows,
                header,
                lines,
            } => {
                let head = if *header { 22.0_f32.min(h * 0.3) } else { 0. };
                let row = (h - head) / *rows as f32;
                ensure!(
                    row >= 10.,
                    "Make the table taller: each row needs at least 10 points"
                );
                let total: f32 = columns.iter().map(|c| c.width).sum();
                let mut edges = vec![x0];
                for c in columns {
                    edges.push(edges.last().unwrap() + w * c.width / total);
                }
                ensure!(
                    edges.windows(2).all(|e| e[1] - e[0] >= 12.),
                    "Make the table wider: each column needs at least 12 points"
                );
                if *header {
                    plan.shades.push([x0, y0, x1, y0 + head]);
                }
                if *lines {
                    plan.lines.push([x0, y0, x1, y0]);
                    for r in 0..=*rows {
                        let y = y0 + head + r as f32 * row;
                        plan.lines.push([x0, y, x1, y]);
                    }
                    for x in &edges {
                        plan.lines.push([*x, y0, *x, y1]);
                    }
                }
                for (c, column) in columns.iter().enumerate() {
                    let (left, right) = (edges[c], edges[c + 1]);
                    let title = column.title.trim();
                    if *header && !title.is_empty() {
                        plan.labels.push(Label {
                            text: title.into(),
                            rect: [left + 3., y0, right - 3., y0 + head],
                            size: 9.5,
                            centered: true,
                        });
                    }
                    let base = if title.is_empty() {
                        format!("Column {}", c + 1)
                    } else {
                        title.to_owned()
                    };
                    for r in 0..*rows {
                        let top = y0 + head + r as f32 * row;
                        let cell = [left, top, right, top + row];
                        let name = format!("{base} {}", r + 1);
                        plan.fields.push(match column.kind {
                            CellKind::Checkbox => {
                                let s = (row - 4.).min(right - left - 4.).min(14.);
                                let (cx, cy) = ((left + right) / 2., top + row / 2.);
                                PlannedField::new(
                                    FieldType::Checkbox,
                                    &name,
                                    [cx - s / 2., cy - s / 2., cx + s / 2., cy + s / 2.],
                                )
                            }
                            kind => {
                                let mut f = PlannedField::new(
                                    match kind {
                                        CellKind::Date => FieldType::Date,
                                        CellKind::Dropdown => FieldType::Dropdown,
                                        _ => FieldType::Text,
                                    },
                                    &name,
                                    inset(cell, 1.5),
                                );
                                if kind == CellKind::Number {
                                    f.align = Align::Right;
                                }
                                if kind == CellKind::Dropdown {
                                    f.options = column
                                        .choices
                                        .split(',')
                                        .map(|o| o.trim().to_owned())
                                        .filter(|o| !o.is_empty())
                                        .collect();
                                }
                                f
                            }
                        });
                    }
                }
                // Inside a grid the lines are the border.
                if *lines {
                    for f in &mut plan.fields {
                        if f.kind != FieldType::Checkbox {
                            f.border = false;
                        }
                    }
                }
            }
            Layout::Labeled { labels } => {
                let labels: Vec<&str> = labels
                    .iter()
                    .map(|l| l.trim())
                    .filter(|l| !l.is_empty())
                    .collect();
                ensure!(!labels.is_empty(), "Write at least one label");
                let row = h / labels.len() as f32;
                ensure!(
                    row >= 14.,
                    "Make the area taller: each line needs at least 14 points"
                );
                let split = x0 + (w * 0.35).min(180.);
                for (i, label) in labels.iter().enumerate() {
                    let top = y0 + i as f32 * row;
                    plan.labels.push(Label {
                        text: (*label).into(),
                        rect: [x0, top, split - 8., top + row],
                        size: 10.,
                        centered: false,
                    });
                    let kind = crate::form_detect::label_kind(label).unwrap_or(FieldType::Text);
                    let fh = if kind == FieldType::Signature {
                        row - 4.
                    } else {
                        (row - 6.).min(24.)
                    };
                    let fy = top + (row - fh) / 2.;
                    plan.fields
                        .push(PlannedField::new(kind, label, [split, fy, x1, fy + fh]));
                }
            }
            Layout::Checklist { items } => {
                let items: Vec<&str> = items
                    .iter()
                    .map(|l| l.trim())
                    .filter(|l| !l.is_empty())
                    .collect();
                ensure!(!items.is_empty(), "Write at least one item");
                let row = h / items.len() as f32;
                ensure!(
                    row >= 10.,
                    "Make the area taller: each item needs at least 10 points"
                );
                let s = (row - 6.).clamp(6., 14.);
                for (i, item) in items.iter().enumerate() {
                    let top = y0 + i as f32 * row;
                    let cy = top + row / 2.;
                    plan.fields.push(PlannedField::new(
                        FieldType::Checkbox,
                        item,
                        [x0 + 1., cy - s / 2., x0 + 1. + s, cy + s / 2.],
                    ));
                    plan.labels.push(Label {
                        text: (*item).into(),
                        rect: [x0 + s + 8., top, x1, top + row],
                        size: 10.,
                        centered: false,
                    });
                }
            }
            Layout::Choice {
                question,
                options,
                across,
            } => {
                let options: Vec<&str> = options
                    .iter()
                    .map(|l| l.trim())
                    .filter(|l| !l.is_empty())
                    .collect();
                ensure!(options.len() >= 2, "Write at least two answers");
                let question = question.trim();
                let qh = if question.is_empty() {
                    0.
                } else {
                    20.0_f32.min(h * 0.45)
                };
                if !question.is_empty() {
                    plan.labels.push(Label {
                        text: question.into(),
                        rect: [x0, y0, x1, y0 + qh],
                        size: 10.5,
                        centered: false,
                    });
                }
                let name = if question.is_empty() {
                    "Choice"
                } else {
                    question
                };
                let n = options.len() as f32;
                let mut used = HashSet::new();
                for (i, option) in options.iter().enumerate() {
                    let (cell, row) = if *across {
                        let cw = w / n;
                        (
                            [x0 + i as f32 * cw, y0 + qh, x0 + (i + 1) as f32 * cw, y1],
                            h - qh,
                        )
                    } else {
                        let rh = (h - qh) / n;
                        (
                            [
                                x0,
                                y0 + qh + i as f32 * rh,
                                x1,
                                y0 + qh + (i + 1) as f32 * rh,
                            ],
                            rh,
                        )
                    };
                    ensure!(
                        row >= 10. && cell[2] - cell[0] >= 30.,
                        "Make the area larger for these answers"
                    );
                    let s = (row - 6.).clamp(6., 14.);
                    let cy = (cell[1] + cell[3]) / 2.;
                    let mut f = PlannedField::new(
                        FieldType::Radio,
                        name,
                        [cell[0] + 1., cy - s / 2., cell[0] + 1. + s, cy + s / 2.],
                    );
                    f.export = export_value(option, i, &mut used);
                    f.checked = false;
                    plan.fields.push(f);
                    plan.labels.push(Label {
                        text: (*option).into(),
                        rect: [cell[0] + s + 7., cell[1], cell[2] - 4., cell[3]],
                        size: 10.,
                        centered: false,
                    });
                }
            }
            Layout::Signatures { signers } => {
                let row = h / *signers as f32;
                ensure!(
                    row >= 36.,
                    "Make the area taller: each signer needs at least 36 points"
                );
                let gap = 12.;
                let widths = [0.38, 0.2, 0.42];
                let usable = w - 2. * gap;
                for s in 0..*signers {
                    let top = y0 + s as f32 * row;
                    let field_bottom = top + row - 16.;
                    let mut x = x0;
                    for (part, (kind, label)) in [
                        (FieldType::Text, "Name"),
                        (FieldType::Date, "Date"),
                        (FieldType::Signature, "Signature"),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let fw = usable * widths[part];
                        let fh = if kind == FieldType::Signature {
                            (row - 22.).min(46.)
                        } else {
                            22.0_f32.min(row - 22.)
                        };
                        let name = if *signers == 1 {
                            label.to_owned()
                        } else {
                            format!("Signer {} {}", s + 1, label.to_lowercase())
                        };
                        plan.fields.push(PlannedField::new(
                            kind,
                            &name,
                            [x, field_bottom - fh, x + fw, field_bottom],
                        ));
                        plan.labels.push(Label {
                            text: label.into(),
                            rect: [x, field_bottom + 2., x + fw, field_bottom + 14.],
                            size: 8.5,
                            centered: false,
                        });
                        x += fw + gap;
                    }
                }
            }
        }
        Ok(plan)
    }
}
fn inset(r: [f32; 4], by: f32) -> [f32; 4] {
    [r[0] + by, r[1] + by, r[2] - by, r[3] - by]
}
/// A radio button's export value from its answer: letters, digits and
/// underscores, different from the others.
fn export_value(answer: &str, index: usize, used: &mut HashSet<String>) -> String {
    let mut v: String = answer
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .chars()
        .take(40)
        .collect();
    if v.is_empty() || v == "Off" {
        v = format!("Choice{}", index + 1);
    }
    let mut candidate = v.clone();
    let mut n = 2;
    while !used.insert(candidate.clone()) {
        candidate = format!("{v}{n}");
        n += 1;
    }
    candidate
}

/// The pieces of a layout, in displayed page coordinates.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    /// Line segments: x1, y1, x2, y2.
    pub lines: Vec<[f32; 4]>,
    /// Shaded areas, such as a header row.
    pub shades: Vec<[f32; 4]>,
    pub labels: Vec<Label>,
    pub fields: Vec<PlannedField>,
}
#[derive(Clone, Debug)]
pub struct Label {
    pub text: String,
    /// The box the text fits in, vertically centred.
    pub rect: [f32; 4],
    pub size: f32,
    pub centered: bool,
}
#[derive(Clone, Debug)]
pub struct PlannedField {
    pub kind: FieldType,
    pub name: String,
    pub rect: [f32; 4],
    pub align: Align,
    pub options: Vec<String>,
    pub export: String,
    pub checked: bool,
    pub border: bool,
}
impl PlannedField {
    fn new(kind: FieldType, name: &str, rect: [f32; 4]) -> Self {
        Self {
            kind,
            name: name.replace('.', " ").trim().chars().take(100).collect(),
            rect,
            align: Align::Left,
            options: vec![],
            export: String::new(),
            checked: false,
            border: true,
        }
    }
}

/// Draw a layout's lines and labels into a page and add its fields. Returns the
/// new bytes and the number of fields added.
pub fn insert(
    bytes: &[u8],
    page: usize,
    rect: [f32; 4],
    layout: &Layout,
) -> Result<(Vec<u8>, usize)> {
    let plan = layout.plan(rect)?;
    let geometry = crate::annotations::page_geometry(bytes)?;
    let (bounds, inverse) = *geometry.get(page).context("Page not found")?;
    ensure!(
        rect[0] >= bounds[0] - 0.5
            && rect[1] >= bounds[1] - 0.5
            && rect[2] <= bounds[2] + 0.5
            && rect[3] <= bounds[3] + 0.5,
        "The layout must fit on the page"
    );
    let m = form_design::page_transform(inverse)?;
    let mut taken: HashSet<String> = crate::forms::fields(bytes)
        .unwrap_or_default()
        .into_iter()
        .map(|f| f.name)
        .collect();
    let mut doc = content::editable(bytes)?;
    let page_id = *doc
        .get_pages()
        .get(&(page as u32 + 1))
        .context("Page not found")?;

    // Lines, shading and labels, drawn in displayed coordinates.
    let c = m.as_coeffs();
    let mut body = format!(
        "Q\nq {} {} {} {} {} {} cm\n",
        c[0], c[1], c[2], c[3], c[4], c[5]
    );
    for s in &plan.shades {
        body.push_str(&format!(
            "0.93 0.94 0.96 rg {} {} {} {} re f\n",
            s[0],
            s[1],
            s[2] - s[0],
            s[3] - s[1]
        ));
    }
    if !plan.lines.is_empty() {
        body.push_str("0.35 0.38 0.43 RG 0.6 w 0 J\n");
        for l in &plan.lines {
            body.push_str(&format!("{} {} m {} {} l S\n", l[0], l[1], l[2], l[3]));
        }
    }
    let mut resources = content::resources(&doc, page_id)?;
    let all: String = plan.labels.iter().map(|l| l.text.as_str()).collect();
    if !all.is_empty() {
        let font = PdfFont::embed(&mut doc, &all)?;
        let mut fonts = resources
            .get(b"Font")
            .ok()
            .and_then(|o| doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok().cloned())
            .unwrap_or_default();
        let name = (0..)
            .map(|n| format!("MyPDFLayout{n}"))
            .find(|n| !fonts.has(n.as_bytes()))
            .unwrap();
        fonts.set(name.as_str(), font.id);
        resources.set("Font", fonts);
        body.push_str("0.13 0.15 0.18 rg\n");
        for label in &plan.labels {
            let width = label.rect[2] - label.rect[0];
            let height = label.rect[3] - label.rect[1];
            let mut size = label.size.min(height - 2.).max(4.);
            while size > 4. && font.width(&label.text, size) > width {
                size -= 0.5;
            }
            let x = if label.centered {
                label.rect[0] + (width - font.width(&label.text, size)).max(0.) / 2.
            } else {
                label.rect[0]
            };
            // Displayed coordinates run downward, so the text matrix flips back upright.
            let baseline = label.rect[1] + (height + size * 0.7) / 2.;
            body.push_str(&format!(
                "BT /{name} {size} Tf 1 0 0 -1 {x} {baseline} Tm <{}> Tj ET\n",
                font.hex(&label.text)?
            ));
        }
    }
    body.push_str("Q\n");
    // Earlier content is wrapped so its graphics state cannot leak into the layout.
    let open = doc.add_object(Stream::new(Dictionary::new(), b"q\n".to_vec()));
    let close = doc.add_object(Stream::new(Dictionary::new(), body.into_bytes()));
    let contents = doc.get_dictionary(page_id)?.get(b"Contents").ok().cloned();
    let mut list = vec![Object::Reference(open)];
    match contents
        .as_ref()
        .map(|c| doc.dereference(c).map(|(_, o)| o.clone()))
    {
        Some(Ok(Object::Array(a))) => list.extend(a),
        Some(Ok(Object::Stream(_))) => list.extend(contents),
        _ => {}
    }
    list.push(Object::Reference(close));
    let d = doc.get_dictionary_mut(page_id)?;
    d.set("Contents", list);
    d.set("Resources", resources);

    // The fields, each under a name no other field has.
    let mut groups: HashSet<String> = HashSet::new();
    for f in &plan.fields {
        let name = if f.kind == FieldType::Radio && groups.contains(&f.name) {
            f.name.clone()
        } else {
            let base = if f.name.is_empty() {
                f.kind.base_name().to_owned()
            } else {
                f.name.clone()
            };
            (1..)
                .map(|n| {
                    if n == 1 {
                        base.clone()
                    } else {
                        format!("{base} {n}")
                    }
                })
                .find(|n| !taken.contains(n))
                .unwrap()
        };
        taken.insert(name.clone());
        if f.kind == FieldType::Radio {
            groups.insert(name.clone());
        }
        let mut p = Properties::new(f.kind, &name);
        p.align = f.align;
        if !f.options.is_empty() {
            p.options = f.options.clone();
        }
        if !f.export.is_empty() {
            p.export_value = f.export.clone();
        }
        p.checked = f.checked;
        if !f.border {
            p.border = None;
        }
        form_design::add_in(&mut doc, m, page, f.rect, f.kind, &p)?;
    }
    let count = plan.fields.len();
    Ok((content::serialize(&mut doc)?, count))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plans_divide_the_area() {
        let plan = Layout::table().plan([50., 100., 530., 232.]).unwrap();
        assert_eq!(plan.fields.len(), 20);
        assert_eq!(plan.labels.len(), 4);
        // Header line, 6 row lines and 5 column edges.
        assert_eq!(plan.lines.len(), 12);
        let quantity = plan.fields.iter().find(|f| f.name == "Quantity 1").unwrap();
        assert_eq!(quantity.align, Align::Right);
        assert!(
            Layout::table().plan([50., 100., 530., 120.]).is_err(),
            "too short"
        );
        let sign = Layout::signatures().plan([50., 100., 530., 216.]).unwrap();
        assert_eq!(
            sign.fields
                .iter()
                .filter(|f| f.kind == FieldType::Signature)
                .count(),
            2
        );
        let mut used = HashSet::new();
        assert_eq!(export_value("Not sure", 0, &mut used), "Not_sure");
        assert_eq!(export_value("Not sure!", 1, &mut used), "Not_sure2");
        assert_eq!(export_value("Off", 2, &mut used), "Choice3");
    }
}
