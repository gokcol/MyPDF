//! Form data in and out: FDF and XFDF as Acrobat writes them, and CSV for
//! spreadsheets. Also dates in Acrobat's format notation.
use crate::form_design::{FieldType, kind_of};
use anyhow::{Context, Result, bail, ensure};
use lopdf::{Document, Object};

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

#[derive(Debug, PartialEq)]
enum Token {
    Day(usize),
    Month(usize),
    Year(usize),
    Literal(char),
}
fn tokens(format: &str) -> Vec<Token> {
    let chars: Vec<char> = format.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let run = chars[i..].iter().take_while(|x| **x == c).count();
        match c {
            'd' => out.push(Token::Day(run)),
            'm' => out.push(Token::Month(run)),
            'y' => out.push(Token::Year(run)),
            _ => {
                out.push(Token::Literal(c));
                i += 1;
                continue;
            }
        }
        i += run;
    }
    out
}
fn days_in(year: i32, month: u32) -> u32 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}
/// Read a date written in `format` (such as dd.mm.yyyy) as year, month, day.
pub fn parse_date(format: &str, text: &str) -> Result<(i32, u32, u32)> {
    let text = text.trim();
    let mut rest = text;
    let (mut year, mut month, mut day) = (None, None, None);
    let digits = |s: &str, max: usize| -> Option<(u32, usize)> {
        let n = s.chars().take(max).take_while(char::is_ascii_digit).count();
        (n > 0).then(|| (s[..n].parse().unwrap_or(0), n))
    };
    for token in tokens(format) {
        match token {
            Token::Literal(c) if c.is_whitespace() => rest = rest.trim_start(),
            Token::Literal(c) => {
                rest = rest
                    .strip_prefix(c)
                    .with_context(|| format!("Expected “{c}” in the date"))?
            }
            Token::Day(_) => {
                let (v, n) = digits(rest, 2).context("Expected a day")?;
                day = Some(v);
                rest = &rest[n..];
            }
            Token::Month(n @ 1..=2) => {
                let (v, used) = digits(rest, n.max(2)).context("Expected a month")?;
                month = Some(v);
                rest = &rest[used..];
            }
            Token::Month(n) => {
                let lower = rest.to_lowercase();
                let found = MONTHS.iter().enumerate().find_map(|(i, m)| {
                    let name = if n == 3 { &m[..3] } else { m };
                    lower
                        .starts_with(&name.to_lowercase())
                        .then_some((i as u32 + 1, name.len()))
                });
                let (v, used) = found.context("Expected a month name")?;
                month = Some(v);
                rest = &rest[used..];
            }
            Token::Year(n) => {
                let (v, used) =
                    digits(rest, if n <= 2 { 2 } else { 4 }).context("Expected a year")?;
                year = Some(if used <= 2 { 2000 + v as i32 } else { v as i32 });
                rest = &rest[used..];
            }
        }
    }
    ensure!(rest.trim().is_empty(), "The date has extra characters");
    let (year, month, day) = (
        year.context("The date has no year")?,
        month.context("The date has no month")?,
        day.context("The date has no day")?,
    );
    ensure!(
        (1..=12).contains(&month)
            && day >= 1
            && day <= days_in(year, month)
            && (1..=9999).contains(&year),
        "That date does not exist"
    );
    Ok((year, month, day))
}
/// Write a date in `format`.
pub fn format_date(format: &str, (year, month, day): (i32, u32, u32)) -> String {
    let mut out = String::new();
    for token in tokens(format) {
        match token {
            Token::Literal(c) => out.push(c),
            Token::Day(1) => out.push_str(&day.to_string()),
            Token::Day(_) => out.push_str(&format!("{day:02}")),
            Token::Month(1) => out.push_str(&month.to_string()),
            Token::Month(2) => out.push_str(&format!("{month:02}")),
            Token::Month(3) => out.push_str(&MONTHS[month as usize - 1][..3]),
            Token::Month(_) => out.push_str(MONTHS[month as usize - 1]),
            Token::Year(n) if n <= 2 => out.push_str(&format!("{:02}", year % 100)),
            Token::Year(_) => out.push_str(&format!("{year:04}")),
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataFormat {
    Fdf,
    Xfdf,
    Csv,
}
impl DataFormat {
    pub const ALL: [DataFormat; 3] = [DataFormat::Fdf, DataFormat::Xfdf, DataFormat::Csv];
    pub fn extension(self) -> &'static str {
        match self {
            DataFormat::Fdf => "fdf",
            DataFormat::Xfdf => "xfdf",
            DataFormat::Csv => "csv",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            DataFormat::Fdf => "FDF (Acrobat form data)",
            DataFormat::Xfdf => "XFDF (XML form data)",
            DataFormat::Csv => "CSV (spreadsheet)",
        }
    }
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension.to_ascii_lowercase().as_str() {
            "fdf" => Some(DataFormat::Fdf),
            "xfdf" | "xml" => Some(DataFormat::Xfdf),
            "csv" | "txt" => Some(DataFormat::Csv),
            _ => None,
        }
    }
}

/// A fillable field's full name, kind and current value.
pub struct FieldValue {
    pub name: String,
    pub kind: FieldType,
    pub value: String,
}
/// The values of the fields people fill: not buttons, pictures or signatures.
pub fn values(bytes: &[u8]) -> Result<Vec<FieldValue>> {
    let doc = Document::load_mem(bytes)?;
    Ok(crate::forms::fields(bytes)?
        .into_iter()
        .filter_map(|f| {
            let kind = kind_of(&doc, f.id)?;
            kind.fillable().then(|| FieldValue {
                value: match kind {
                    FieldType::Checkbox | FieldType::Radio if f.value.is_empty() => "Off".into(),
                    _ => f.value,
                },
                name: f.name,
                kind,
            })
        })
        .collect())
}

/// A PDF string: literal for plain ASCII, otherwise UTF-16 with a byte-order mark.
fn pdf_string(s: &str) -> String {
    if s.chars().all(|c| (' '..='~').contains(&c)) {
        let escaped = s
            .replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)");
        format!("({escaped})")
    } else {
        let mut hex = String::from("<FEFF");
        for unit in s.encode_utf16() {
            hex.push_str(&format!("{unit:04X}"));
        }
        hex.push('>');
        hex
    }
}
fn pdf_name(s: &str) -> String {
    let mut out = String::from("/");
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("#{b:02X}"));
        }
    }
    out
}
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Names split at dots, as a tree, so hierarchical fields export as Acrobat writes them.
#[derive(Default)]
struct Node {
    children: Vec<(String, Node)>,
    value: Option<(String, bool)>,
}
fn tree(values: &[FieldValue]) -> Node {
    let mut root = Node::default();
    for v in values {
        let mut node = &mut root;
        for part in v.name.split('.') {
            let i = match node.children.iter().position(|(n, _)| n == part) {
                Some(i) => i,
                None => {
                    node.children.push((part.to_owned(), Node::default()));
                    node.children.len() - 1
                }
            };
            node = &mut node.children[i].1;
        }
        node.value = Some((
            v.value.clone(),
            matches!(v.kind, FieldType::Checkbox | FieldType::Radio),
        ));
    }
    root
}
fn fdf_fields(node: &Node, out: &mut String) {
    for (name, child) in &node.children {
        out.push_str(&format!("<< /T {}", pdf_string(name)));
        if let Some((value, is_name)) = &child.value {
            if *is_name {
                out.push_str(&format!(" /V {}", pdf_name(value)));
            } else {
                out.push_str(&format!(" /V {}", pdf_string(value)));
            }
        }
        if !child.children.is_empty() {
            out.push_str(" /Kids [ ");
            fdf_fields(child, out);
            out.push(']');
        }
        out.push_str(" >>\n");
    }
}
fn xfdf_fields(node: &Node, out: &mut String, depth: usize) {
    for (name, child) in &node.children {
        let indent = "  ".repeat(depth);
        out.push_str(&format!("{indent}<field name=\"{}\">", xml_escape(name)));
        if let Some((value, _)) = &child.value {
            out.push_str(&format!("<value>{}</value>", xml_escape(value)));
        }
        if !child.children.is_empty() {
            out.push('\n');
            xfdf_fields(child, out, depth + 1);
            out.push_str(&indent);
        }
        out.push_str("</field>\n");
    }
}
fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) || s.starts_with(' ') || s.ends_with(' ') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

/// Write the form's values; `pdf_name` is the PDF's file name, which FDF and XFDF record.
pub fn export(bytes: &[u8], format: DataFormat, pdf_name: &str) -> Result<Vec<u8>> {
    let values = values(bytes)?;
    ensure!(!values.is_empty(), "This PDF has no form fields to export");
    Ok(match format {
        DataFormat::Fdf => {
            let mut fields = String::new();
            fdf_fields(&tree(&values), &mut fields);
            let mut out = b"%FDF-1.2\n%\xe2\xe3\xcf\xd3\n".to_vec();
            out.extend(
                format!(
                    "1 0 obj\n<< /FDF << /F {} /Fields [\n{fields}] >> >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n",
                    pdf_string(pdf_name)
                )
                .into_bytes(),
            );
            out
        }
        DataFormat::Xfdf => {
            let mut fields = String::new();
            xfdf_fields(&tree(&values), &mut fields, 2);
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xfdf xmlns=\"http://ns.adobe.com/xfdf/\" xml:space=\"preserve\">\n  <f href=\"{}\"/>\n  <fields>\n{fields}  </fields>\n</xfdf>\n",
                xml_escape(pdf_name)
            )
            .into_bytes()
        }
        DataFormat::Csv => {
            // A byte-order mark lets spreadsheet apps read the text as UTF-8.
            let mut out = "\u{feff}".to_owned();
            out.push_str(
                &values
                    .iter()
                    .map(|v| csv_cell(&v.name))
                    .collect::<Vec<_>>()
                    .join(","),
            );
            out.push_str("\r\n");
            out.push_str(
                &values
                    .iter()
                    .map(|v| csv_cell(&v.value))
                    .collect::<Vec<_>>()
                    .join(","),
            );
            out.push_str("\r\n");
            out.into_bytes()
        }
    })
}

fn fdf_values(data: &[u8]) -> Result<Vec<(String, String)>> {
    ensure!(data.starts_with(b"%FDF-"), "This is not an FDF file");
    let as_pdf = [b"%PDF".as_slice(), &data[4..]].concat();
    let doc = Document::load_mem(&as_pdf).context("The FDF file is damaged")?;
    let root = doc.trailer.get(b"Root")?.as_reference()?;
    let fdf = doc.get_dictionary(root)?.get(b"FDF")?;
    let fdf = doc.dereference(fdf)?.1.as_dict()?;
    let fields = doc.dereference(fdf.get(b"Fields")?)?.1.as_array()?.clone();
    let mut out = vec![];
    let mut pending: Vec<(Object, String, usize)> = fields
        .into_iter()
        .rev()
        .map(|f| (f, String::new(), 0))
        .collect();
    while let Some((field, prefix, depth)) = pending.pop() {
        ensure!(
            depth <= 32 && out.len() <= 10_000,
            "The FDF file is too deeply nested"
        );
        let Ok(d) = doc.dereference(&field).and_then(|(_, o)| o.as_dict()) else {
            continue;
        };
        let part = d
            .get(b"T")
            .map(|t| lopdf::decode_text_string(t).unwrap_or_default())
            .unwrap_or_default();
        let name = match (prefix.is_empty(), part.is_empty()) {
            (true, _) => part,
            (false, true) => prefix,
            (false, false) => format!("{prefix}.{part}"),
        };
        if let Ok(v) = d.get(b"V") {
            let v = doc.dereference(v).map(|(_, o)| o).unwrap_or(v);
            let value = match v {
                Object::Name(n) => String::from_utf8_lossy(n).into_owned(),
                Object::Array(a) => a
                    .first()
                    .map(|o| lopdf::decode_text_string(o).unwrap_or_default())
                    .unwrap_or_default(),
                other => lopdf::decode_text_string(other).unwrap_or_default(),
            };
            out.push((name.clone(), value));
        }
        if let Ok(kids) = d
            .get(b"Kids")
            .and_then(|k| doc.dereference(k))
            .and_then(|(_, k)| k.as_array())
        {
            for kid in kids.iter().rev() {
                pending.push((kid.clone(), name.clone(), depth + 1));
            }
        }
    }
    Ok(out)
}
fn xfdf_values(data: &[u8]) -> Result<Vec<(String, String)>> {
    use quick_xml::events::Event;
    let text = std::str::from_utf8(data).context("XFDF must be UTF-8 text")?;
    let mut reader = quick_xml::Reader::from_str(text.trim_start_matches('\u{feff}'));
    let mut path: Vec<String> = vec![];
    let mut out = vec![];
    let mut value: Option<String> = None;
    let mut seen_xfdf = false;
    loop {
        match reader.read_event()? {
            Event::Start(e) => match e.local_name().as_ref() {
                b"xfdf" => seen_xfdf = true,
                b"field" => {
                    ensure!(path.len() < 32, "The XFDF file is too deeply nested");
                    let mut name = String::new();
                    for a in e.attributes() {
                        let a = a?;
                        if a.key.local_name().as_ref() == b"name" {
                            name = a
                                .normalized_value(quick_xml::XmlVersion::Implicit1_0)?
                                .into_owned();
                        }
                    }
                    path.push(name);
                }
                b"value" if !path.is_empty() => value = Some(String::new()),
                _ => {}
            },
            Event::Empty(e) if e.local_name().as_ref() == b"value" && !path.is_empty() => {
                out.push((path.join("."), String::new()));
            }
            Event::Text(e) => {
                if let Some(v) = &mut value {
                    v.push_str(&quick_xml::escape::unescape(&e.decode()?)?);
                }
            }
            Event::CData(e) => {
                if let Some(v) = &mut value {
                    v.push_str(&String::from_utf8_lossy(&e));
                }
            }
            Event::GeneralRef(e) => {
                if let Some(v) = &mut value {
                    let n = e.decode()?;
                    v.push_str(&quick_xml::escape::unescape(&format!("&{n};"))?);
                }
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"value" => {
                    if let Some(v) = value.take() {
                        ensure!(out.len() < 10_000, "The XFDF file has too many values");
                        out.push((path.join("."), v));
                    }
                }
                b"field" => {
                    path.pop();
                }
                _ => {}
            },
            Event::DocType(_) => bail!("XFDF with a document type is not supported"),
            Event::Eof => break,
            _ => {}
        }
    }
    ensure!(seen_xfdf, "This is not an XFDF file");
    Ok(out)
}
/// RFC 4180 rows; the first row names the fields, the second holds the values.
fn csv_rows(text: &str) -> Vec<Vec<String>> {
    let mut rows = vec![];
    let mut row = vec![];
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    // Excel in some regions separates cells with semicolons.
    let first_line: String = text.lines().next().unwrap_or_default().to_owned();
    let separator = if first_line.matches(';').count() > first_line.matches(',').count() {
        ';'
    } else {
        ','
    };
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            '"' if quoted => quoted = false,
            '"' if cell.is_empty() => quoted = true,
            c if c == separator && !quoted => row.push(std::mem::take(&mut cell)),
            '\r' if !quoted => {}
            '\n' if !quoted => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            c => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows
}
fn csv_values(data: &[u8]) -> Result<Vec<(String, String)>> {
    let text = std::str::from_utf8(data).context("Save the CSV file as UTF-8 text")?;
    let rows = csv_rows(text);
    ensure!(
        rows.len() >= 2,
        "The CSV file needs a row of field names and a row of values"
    );
    Ok(rows[0]
        .iter()
        .zip(rows[1].iter())
        .map(|(n, v)| (n.trim().to_owned(), v.clone()))
        .filter(|(n, _)| !n.is_empty())
        .collect())
}

/// What an import did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportReport {
    pub filled: usize,
    /// Names in the data that the form does not have.
    pub unknown: Vec<String>,
    /// Values a field did not accept, with the reason.
    pub rejected: Vec<(String, String)>,
}
/// Fill the form from FDF, XFDF or CSV data.
pub fn import(bytes: &[u8], data: &[u8], format: DataFormat) -> Result<(Vec<u8>, ImportReport)> {
    ensure!(
        data.len() <= 16 * 1024 * 1024,
        "The data file exceeds 16 MB"
    );
    let incoming = match format {
        DataFormat::Fdf => fdf_values(data)?,
        DataFormat::Xfdf => xfdf_values(data)?,
        DataFormat::Csv => csv_values(data)?,
    };
    let mut report = ImportReport::default();
    let mut bytes = bytes.to_vec();
    for (name, value) in incoming {
        let doc = Document::load_mem(&bytes)?;
        let fields = crate::forms::fields(&bytes)?;
        let Some(field) = fields.iter().find(|f| f.name == name) else {
            report.unknown.push(name);
            continue;
        };
        let Some(kind) = kind_of(&doc, field.id).filter(|k| k.fillable()) else {
            report.unknown.push(name);
            continue;
        };
        let value = match (kind, &field.kind) {
            (FieldType::Checkbox, crate::forms::FieldKind::Button { states, .. }) => {
                let on = states.first().cloned().unwrap_or_else(|| "Yes".into());
                let lower = value.trim().to_lowercase();
                if lower.is_empty() || ["off", "no", "false", "0"].contains(&lower.as_str()) {
                    "Off".to_owned()
                } else if states.contains(&value)
                    || ["yes", "on", "true", "1", "x", "evet"].contains(&lower.as_str())
                {
                    on
                } else {
                    value
                }
            }
            (
                FieldType::Dropdown | FieldType::ListBox,
                crate::forms::FieldKind::Choice { options, .. },
            ) => {
                // A choice may be given by its label as well as its value.
                options
                    .iter()
                    .find(|(v, l)| *v == value || *l == value)
                    .map(|(v, _)| v.clone())
                    .unwrap_or(value)
            }
            (FieldType::Radio, _) if value.trim().is_empty() => "Off".to_owned(),
            _ => value,
        };
        if value == field.value || value == "Off" && field.value.is_empty() {
            continue;
        }
        match crate::forms::fill(&bytes, field.id, &value) {
            Ok(filled) => {
                bytes = filled;
                report.filled += 1;
            }
            Err(e) => report.rejected.push((name, format!("{e:#}"))),
        }
    }
    Ok((bytes, report))
}
