//! Document metadata, keeping the PDF Info dictionary and XMP in agreement.
use anyhow::{Context, Result, ensure};
use lopdf::{Document, Object, Stream, dictionary};
use quick_xml::{Writer, events::Event, name::ResolveResult, reader::NsReader};

pub const LABELS: [&str; 6] = [
    "Title", "Author", "Subject", "Keywords", "Creator", "Producer",
];
const KEYS: [&str; 8] = [
    "Title",
    "Author",
    "Subject",
    "Keywords",
    "Creator",
    "Producer",
    "CreationDate",
    "ModDate",
];
const DC: &str = "http://purl.org/dc/elements/1.1/";
const PDF: &str = "http://ns.adobe.com/pdf/1.3/";
const XMP: &str = "http://ns.adobe.com/xap/1.0/";
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Metadata {
    pub fields: [String; 6],
}
#[derive(Clone, Debug)]
pub struct Properties {
    pub metadata: Metadata,
    pub created: String,
    pub modified: String,
    pub version: String,
    pub xmp: String,
    pub warning: Option<String>,
}
fn info(doc: &Document) -> Result<lopdf::Dictionary> {
    match doc.trailer.get(b"Info") {
        Ok(o) => Ok(doc.dereference(o)?.1.as_dict()?.clone()),
        Err(_) => Ok(lopdf::Dictionary::new()),
    }
}
fn xmp_packet(doc: &Document) -> Result<Option<Vec<u8>>> {
    match doc.catalog()?.get(b"Metadata") {
        Ok(o) => {
            let stream = doc.dereference(o)?.1.as_stream()?;
            ensure!(
                stream.content.len() <= 4 * 1024 * 1024,
                "XMP metadata is too large to edit"
            );
            let bytes = stream.get_plain_content()?;
            ensure!(
                bytes.len() <= 4 * 1024 * 1024,
                "XMP metadata is too large to edit"
            );
            Ok(Some(bytes))
        }
        Err(_) => Ok(None),
    }
}
pub fn read(bytes: &[u8]) -> Result<Properties> {
    let doc = Document::load_mem(bytes)?;
    let dict = info(&doc)?;
    let mut values: [String; 8] = Default::default();
    let mut xmp = String::new();
    let mut warning = None;
    match xmp_packet(&doc).and_then(|packet| {
        if let Some(packet) = packet {
            xmp = String::from_utf8(packet).context("XMP is not UTF-8")?;
            values = process_xmp(&xmp, None)?.1;
        }
        Ok(())
    }) {
        Ok(()) => {}
        Err(e) => {
            warning = Some(format!(
                "XMP cannot be safely edited: {e}. Metadata editing is disabled."
            ))
        }
    }
    for (i, key) in KEYS.iter().enumerate() {
        if let Ok(o) = dict.get(key.as_bytes()) {
            values[i] = lopdf::decode_text_string(doc.dereference(o)?.1)?;
        }
    }
    let version = doc
        .catalog()?
        .get(b"Version")
        .and_then(Object::as_name)
        .map(|v| String::from_utf8_lossy(v).into_owned())
        .unwrap_or(doc.version.clone());
    Ok(Properties {
        metadata: Metadata {
            fields: std::array::from_fn(|i| values[i].clone()),
        },
        created: values[6].clone(),
        modified: values[7].clone(),
        version,
        xmp,
        warning,
    })
}
/// Empty edited values remove that property. Unknown Info and XMP properties survive.
pub fn update(bytes: &[u8], metadata: &Metadata) -> Result<Vec<u8>> {
    let mut doc = crate::content::editable(bytes)?;
    ensure!(
        !crate::document::has_signatures(&doc) && !doc.catalog()?.has(b"Perms"),
        "Signed or certified PDFs keep their original metadata"
    );
    let old = read(bytes)?;
    ensure!(old.warning.is_none(), "{}", old.warning.unwrap_or_default());
    let mut changes: [Option<String>; 8] = Default::default();
    for (i, value) in metadata.fields.iter().enumerate() {
        ensure!(value.len() <= 16384, "{} must be at most 16 KB", LABELS[i]);
        ensure!(
            value.chars().all(|c| matches!(c, '\t' | '\n' | '\r')
                || (c >= ' ' && c != '\u{fffe}' && c != '\u{ffff}')),
            "{} contains unsupported control characters",
            LABELS[i]
        );
        if value != &old.metadata.fields[i] {
            changes[i] = Some(value.clone());
        }
    }
    if changes.iter().all(Option::is_none) {
        return Ok(bytes.to_vec());
    }
    let now = time::OffsetDateTime::now_utc();
    let iso = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        now.year(),
        now.month() as u8,
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );
    changes[7] = Some(iso);
    let packet = if old.xmp.is_empty() {
        format!(
            "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"{RDF}\"></rdf:RDF></x:xmpmeta>"
        )
    } else {
        old.xmp
    };
    let output = process_xmp(&packet, Some(&changes))?.0;
    let mut dict = info(&doc)?;
    for (i, value) in changes.iter().enumerate().take(6) {
        if let Some(value) = value {
            if value.is_empty() {
                dict.remove(KEYS[i].as_bytes());
            } else {
                dict.set(KEYS[i], crate::annotations::text_string(value));
            }
        }
    }
    dict.set(
        "ModDate",
        Object::string_literal(format!(
            "D:{:04}{:02}{:02}{:02}{:02}{:02}Z",
            now.year(),
            now.month() as u8,
            now.day(),
            now.hour(),
            now.minute(),
            now.second()
        )),
    );
    let id = doc.add_object(dict);
    doc.trailer.set("Info", id);
    let stream = doc.add_object(Stream::new(
        dictionary! {"Type"=>"Metadata", "Subtype"=>"XML"},
        output,
    ));
    doc.catalog_mut()?.set("Metadata", stream);
    crate::content::serialize(&mut doc)
}
fn namespace(ns: ResolveResult<'_>) -> String {
    match ns {
        ResolveResult::Bound(ns) => String::from_utf8_lossy(ns.as_ref()).into_owned(),
        _ => String::new(),
    }
}
fn field(ns: &str, local: &[u8]) -> Option<usize> {
    match (ns, local) {
        (DC, b"title") => Some(0),
        (DC, b"creator") => Some(1),
        (DC, b"description") => Some(2),
        (PDF, b"Keywords") => Some(3),
        (XMP, b"CreatorTool") => Some(4),
        (PDF, b"Producer") => Some(5),
        (XMP, b"CreateDate") => Some(6),
        (XMP, b"ModifyDate") => Some(7),
        _ => None,
    }
}
fn description(changes: &[Option<String>; 8]) -> String {
    let mut out = format!(
        "<rdf:Description xmlns:rdf=\"{RDF}\" xmlns:dc=\"{DC}\" xmlns:pdf=\"{PDF}\" xmlns:xmp=\"{XMP}\" rdf:about=\"\">"
    );
    for (i, value) in changes.iter().enumerate() {
        let Some(value) = value.as_ref().filter(|v| !v.is_empty()) else {
            continue;
        };
        let value = quick_xml::escape::escape(value);
        out.push_str(&match i {
            0|2 => { let tag = if i==0 { "title" } else { "description" }; format!("<dc:{tag}><rdf:Alt><rdf:li xml:lang=\"x-default\">{value}</rdf:li></rdf:Alt></dc:{tag}>") },
            1=>format!("<dc:creator><rdf:Seq><rdf:li>{value}</rdf:li></rdf:Seq></dc:creator>"),
            _=> { let tag = ["", "", "", "pdf:Keywords", "xmp:CreatorTool", "pdf:Producer", "xmp:CreateDate", "xmp:ModifyDate"][i]; format!("<{tag}>{value}</{tag}>") },
        });
    }
    out.push_str("</rdf:Description>");
    out
}
#[derive(Default)]
struct Capture {
    field: usize,
    depth: usize,
    text: String,
    items: Vec<(bool, String)>,
    item: Option<(usize, bool, String)>,
}
impl Capture {
    fn push(&mut self, text: &str) {
        if let Some((_, _, value)) = &mut self.item {
            value.push_str(text);
        } else {
            self.text.push_str(text);
        }
    }
    fn value(&self) -> String {
        if self.items.is_empty() {
            return self.text.trim().into();
        }
        if self.field == 1 {
            return self
                .items
                .iter()
                .map(|(_, s)| s.as_str())
                .collect::<Vec<_>>()
                .join("; ");
        }
        self.items
            .iter()
            .find(|(default, _)| *default)
            .unwrap_or(&self.items[0])
            .1
            .clone()
    }
}
/// Namespace-aware streaming rewrite: preserve unrelated properties, comments and PIs.
fn process_xmp(xml: &str, changes: Option<&[Option<String>; 8]>) -> Result<(Vec<u8>, [String; 8])> {
    let mut reader = NsReader::from_str(xml);
    let mut writer = Writer::new(Vec::new());
    let mut depth = 0usize;
    let mut rdf_depth = None;
    let mut descriptions = None;
    let mut rdf_count = 0;
    let mut skip = None;
    let mut capture: Option<Capture> = None;
    let mut values: [String; 8] = Default::default();
    loop {
        let event = reader.read_event()?;
        match &event {
            Event::Start(e) | Event::Empty(e) => {
                let empty = matches!(event, Event::Empty(_));
                let (ns, local) = reader.resolver().resolve_element(e.name());
                let ns = namespace(ns);
                let local = local.as_ref();
                ensure!(depth < 64, "XMP is nested too deeply");
                if ns == RDF && local == b"RDF" {
                    rdf_count += 1;
                    rdf_depth = Some(depth);
                    if empty {
                        writer.write_event(Event::Start(e.clone()))?;
                        if let Some(changes) = changes {
                            writer
                                .get_mut()
                                .extend_from_slice(description(changes).as_bytes());
                        }
                        writer.write_event(Event::End(e.to_end()))?;
                        rdf_depth = None;
                        continue;
                    }
                }
                let is_description =
                    ns == RDF && local == b"Description" && rdf_depth == depth.checked_sub(1);
                let mut document_description = is_description;
                if is_description {
                    for attr in e.attributes() {
                        let attr = attr?;
                        let (ans, alocal) = reader.resolver().resolve_attribute(attr.key);
                        if namespace(ans) == RDF
                            && alocal.as_ref() == b"about"
                            && !attr
                                .normalized_value(quick_xml::XmlVersion::Implicit1_0)?
                                .is_empty()
                        {
                            document_description = false;
                        }
                    }
                }
                let property = if descriptions == depth.checked_sub(1) {
                    field(&ns, local)
                } else {
                    None
                };
                if let Some(i) = property {
                    capture = Some(Capture {
                        field: i,
                        depth,
                        ..Default::default()
                    });
                    if changes.is_some_and(|c| c[i].is_some()) {
                        skip = Some(depth);
                    }
                }
                if ns == RDF
                    && local == b"li"
                    && let Some(c) = &mut capture
                {
                    let default = e
                        .attributes()
                        .filter_map(Result::ok)
                        .any(|a| a.key.as_ref() == b"xml:lang" && a.value.as_ref() == b"x-default");
                    if empty {
                        c.items.push((default, String::new()));
                    } else {
                        c.item = Some((depth, default, String::new()));
                    }
                }
                let mut start = e.clone();
                if document_description {
                    if !empty {
                        descriptions = Some(depth);
                    }
                    start.clear_attributes();
                    for attr in e.attributes() {
                        let attr = attr?;
                        let (ns, local) = reader.resolver().resolve_attribute(attr.key);
                        let i = field(&namespace(ns), local.as_ref());
                        if let Some(i) = i {
                            values[i] = attr
                                .normalized_value(quick_xml::XmlVersion::Implicit1_0)?
                                .into_owned();
                        }
                        if i.is_none_or(|i| changes.is_none_or(|c| c[i].is_none())) {
                            start.push_attribute(attr);
                        }
                    }
                }
                if skip.is_none() {
                    writer.write_event(if empty {
                        Event::Empty(start)
                    } else {
                        Event::Start(start)
                    })?;
                }
                if empty {
                    if skip == Some(depth) {
                        skip = None;
                    }
                    if capture.as_ref().is_some_and(|c| c.depth == depth) {
                        capture = None;
                    }
                } else {
                    depth += 1;
                }
            }
            Event::End(_) => {
                depth = depth.checked_sub(1).context("Invalid XMP nesting")?;
                if let Some(c) = &mut capture
                    && c.item.as_ref().is_some_and(|(at, _, _)| *at == depth)
                {
                    let (_, default, value) = c.item.take().unwrap();
                    c.items.push((default, value));
                }
                if let Some(c) = &capture
                    && c.depth == depth
                {
                    values[c.field] = c.value();
                    capture = None;
                }
                if descriptions == Some(depth) {
                    descriptions = None;
                }
                if rdf_depth == Some(depth) {
                    if let Some(changes) = changes {
                        writer
                            .get_mut()
                            .extend_from_slice(description(changes).as_bytes());
                    }
                    rdf_depth = None;
                }
                if skip.is_none() {
                    writer.write_event(event)?;
                }
                if skip == Some(depth) {
                    skip = None;
                }
            }
            Event::DocType(_) => anyhow::bail!("DTD declarations are not supported in XMP"),
            Event::Eof => break,
            _ => {
                if let Some(value) = &mut capture {
                    match &event {
                        Event::Text(t) => value.push(&t.xml10_content()?),
                        Event::CData(t) => value.push(&t.xml10_content()?),
                        Event::GeneralRef(r) => {
                            let entity = format!("&{};", r.decode()?);
                            value.push(&quick_xml::escape::unescape(&entity)?);
                        }
                        _ => {}
                    }
                }
                if skip.is_none() {
                    writer.write_event(event)?;
                }
            }
        }
    }
    ensure!(
        depth == 0 && rdf_count == 1,
        "XMP must contain one complete RDF document"
    );
    Ok((writer.into_inner(), values))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(xmp: Option<&str>) -> Vec<u8> {
        let mut doc = Document::with_version("1.7");
        let contents = doc.add_object(Stream::new(
            dictionary! {},
            b"q 1 0 0 rg 10 10 50 50 re f Q".to_vec(),
        ));
        let pages = doc.new_object_id();
        let page = doc.add_object(dictionary!{"Type"=>"Page", "Parent"=>pages, "MediaBox"=>vec![0.into(),0.into(),300.into(),400.into()], "Contents"=>contents, "Resources"=>dictionary!{}});
        doc.objects.insert(
            pages,
            dictionary! {"Type"=>"Pages", "Kids"=>vec![Object::Reference(page)], "Count"=>1}.into(),
        );
        let mut catalog = dictionary! {"Type"=>"Catalog", "Pages"=>pages};
        if let Some(xmp) = xmp {
            let id = doc.add_object(Stream::new(
                dictionary! {"Type"=>"Metadata", "Subtype"=>"XML"},
                xmp.as_bytes().to_vec(),
            ));
            catalog.set("Metadata", id);
        }
        let root = doc.add_object(catalog);
        doc.trailer.set("Root", root);
        let info = doc.add_object(dictionary!{"Title"=>crate::annotations::text_string("Original"), "Custom"=>Object::string_literal("Keep this"), "CreationDate"=>Object::string_literal("D:20260102123456Z")});
        doc.trailer.set("Info", info);
        crate::content::serialize(&mut doc).unwrap()
    }
    #[test]
    fn unicode_metadata_round_trip_preserves_content_and_supports_undo() {
        let bytes = fixture(None);
        let original = Document::load_mem(&bytes).unwrap();
        let metadata = Metadata {
            fields: [
                "Başlık – Grüße & <test>",
                "Çağrı Öztürk",
                "Subject",
                "PDF; Türkçe",
                "My editor",
                "Producer",
            ]
            .map(str::to_string),
        };
        let result = update(&bytes, &metadata).unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), &result).unwrap();
        let reopened = std::fs::read(file.path()).unwrap();
        let p = read(&reopened).unwrap();
        assert_eq!(p.metadata, metadata);
        assert_eq!(p.created, "D:20260102123456Z");
        assert!(p.modified.starts_with("D:"));
        assert_eq!(&process_xmp(&p.xmp, None).unwrap().1[..6], &metadata.fields);
        let doc = Document::load_mem(&result).unwrap();
        assert_eq!(
            doc.get_page_content(*doc.get_pages().get(&1).unwrap()),
            original.get_page_content(*original.get_pages().get(&1).unwrap())
        );
        assert_eq!(
            info(&doc)
                .unwrap()
                .get(b"Custom")
                .unwrap()
                .as_str()
                .unwrap(),
            b"Keep this"
        );
        let mut session = crate::annotations::Session::new(bytes.clone().into(), vec![], true);
        session.commit_document(result).unwrap();
        assert!(session.dirty());
        session.undo();
        assert_eq!(session.source.as_ref(), bytes.as_slice());
        session.redo();
        assert_eq!(read(&session.source).unwrap().metadata, metadata);
        let saved = crate::annotations::export(&session.source, &session.annotations).unwrap();
        assert_eq!(read(&saved).unwrap().metadata, metadata);
    }
    #[test]
    fn xmp_namespaces_attributes_arrays_and_unrelated_properties_survive() {
        let xml = format!(
            r#"<?xpacket begin=""?><m:xmpmeta xmlns:m="adobe:ns:meta/"><r:RDF xmlns:r="{RDF}"><r:Description xmlns:d="{DC}" xmlns:p="{PDF}" xmlns:c="urn:custom" r:about="" p:Keywords="old" c:Value="preserve"><d:title><r:Alt><r:li xml:lang="de">Deutsch</r:li><r:li xml:lang="x-default">Default &amp; title</r:li></r:Alt></d:title><d:creator><r:Seq><r:li>Alice</r:li><r:li>Bob</r:li></r:Seq></d:creator><c:Keep>keep me</c:Keep></r:Description><r:Description xmlns:p="{PDF}" r:about="other-resource" p:Keywords="other"/></r:RDF></m:xmpmeta><?xpacket end="w"?>"#
        );
        let (_, values) = process_xmp(&xml, None).unwrap();
        assert_eq!(values[0], "Default & title");
        assert_eq!(values[1], "Alice; Bob");
        assert_eq!(values[3], "old");
        let bytes = fixture(Some(&xml));
        let mut m = read(&bytes).unwrap().metadata;
        m.fields[0] = "New title".into();
        m.fields[3] = String::new();
        let result = update(&bytes, &m).unwrap();
        let p = read(&result).unwrap();
        assert_eq!(p.metadata, m);
        assert!(
            p.xmp.contains("c:Value=\"preserve\"")
                && p.xmp.contains("<c:Keep>keep me</c:Keep>")
                && p.xmp.contains("p:Keywords=\"other\"")
        );
        assert!(!p.xmp.contains("p:Keywords=\"old\"") && !p.xmp.contains("Default &amp; title"));
        assert_eq!(process_xmp(&p.xmp, None).unwrap().1[0], "New title");
        assert!(
            info(&Document::load_mem(&result).unwrap())
                .unwrap()
                .get(b"Keywords")
                .is_err()
        );
    }
    #[test]
    fn empty_rdf_noop_and_malformed_xmp_are_handled() {
        let bytes = fixture(Some(&format!("<r:RDF xmlns:r=\"{RDF}\"/>")));
        let mut m = read(&bytes).unwrap().metadata;
        assert_eq!(update(&bytes, &m).unwrap(), bytes);
        m.fields[2] = "Description".into();
        assert_eq!(read(&update(&bytes, &m).unwrap()).unwrap().metadata, m);
        let broken = fixture(Some("<broken>"));
        assert!(read(&broken).unwrap().warning.is_some());
        assert!(update(&broken, &m).is_err());
    }
    #[test]
    fn signed_encrypted_and_invalid_values_are_not_rewritten() {
        let bytes = fixture(None);
        let mut m = read(&bytes).unwrap().metadata;
        m.fields[0] = "Changed".into();
        let encrypted = crate::security::protect(&bytes, "password").unwrap();
        assert!(update(&encrypted, &m).is_err());
        let mut doc = Document::load_mem(&bytes).unwrap();
        let sig=doc.add_object(dictionary!{"Type"=>"Sig", "ByteRange"=>vec![0.into(),1.into(),2.into(),3.into()], "Contents"=>Object::string_literal("signature")});
        let field = doc.add_object(
            dictionary! {"FT"=>"Sig", "T"=>Object::string_literal("Signature"), "V"=>sig},
        );
        let form = doc.add_object(dictionary! {"Fields"=>vec![Object::Reference(field)]});
        doc.catalog_mut().unwrap().set("AcroForm", form);
        let signed = crate::content::serialize(&mut doc).unwrap();
        assert!(update(&signed, &m).is_err());
        m.fields[0] = "bad\0value".into();
        assert!(update(&bytes, &m).is_err());
        m.fields[0] = "x".repeat(16385);
        assert!(update(&bytes, &m).is_err());
    }
}
