//! Editable outline and PDF page-label number trees. Existing destinations survive renaming.
use anyhow::{Context, Result, ensure};
use lopdf::{Dictionary, Document, Object, ObjectId, dictionary};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq)]
pub struct Bookmark {
    pub source: Option<usize>,
    pub level: usize,
    pub title: String,
    pub page: Option<usize>,
}
fn dictionaries(doc: &Document) -> Vec<(ObjectId, Dictionary)> {
    let mut out = vec![];
    let mut seen = std::collections::HashSet::new();
    let first = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"Outlines").ok())
        .and_then(|o| doc.dereference(o).ok())
        .and_then(|(_, o)| o.as_dict().ok())
        .and_then(|d| d.get(b"First").ok())
        .and_then(|o| o.as_reference().ok());
    let mut stack: Vec<_> = first.into_iter().collect();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) || out.len() >= 10000 {
            continue;
        }
        if let Ok(d) = doc.get_dictionary(id) {
            out.push((id, d.clone()));
            if let Ok(n) = d.get(b"Next").and_then(Object::as_reference) {
                stack.push(n);
            }
            if let Ok(n) = d.get(b"First").and_then(Object::as_reference) {
                stack.push(n);
            }
        }
    }
    out
}
pub fn bookmarks(bytes: &[u8]) -> Result<Vec<Bookmark>> {
    Ok(crate::navigation::outline(bytes)?
        .into_iter()
        .enumerate()
        .map(|(i, b)| Bookmark {
            source: Some(i),
            level: b.level,
            title: b.title,
            page: b.page,
        })
        .collect())
}
pub fn set_bookmarks(bytes: &[u8], entries: &[Bookmark]) -> Result<Vec<u8>> {
    ensure!(entries.len() <= 10000, "Too many bookmarks");
    let mut doc = crate::content::editable(bytes)?;
    let original = dictionaries(&doc);
    let described = bookmarks(bytes)?;
    ensure!(
        original.len() == described.len(),
        "This outline structure cannot be safely edited"
    );
    let pages: Vec<_> = doc.get_pages().into_values().collect();
    let root = doc.add_object(dictionary! {"Type"=>"Outlines"});
    let mut nodes = vec![];
    let mut stack: Vec<usize> = vec![];
    let mut parents = vec![];
    let mut seen = std::collections::HashSet::new();
    for e in entries {
        ensure!(
            !e.title.trim().is_empty()
                && e.title.len() <= 2000
                && e.level <= 32
                && e.level <= stack.len(),
            "Invalid bookmark title or nesting"
        );
        ensure!(
            e.page.is_none_or(|p| p < pages.len()),
            "Bookmark page is outside the document"
        );
        if let Some(index) = e.source {
            ensure!(
                index < original.len() && seen.insert(index),
                "Invalid or duplicated bookmark source"
            );
        }
        let mut d = e.source.map(|i| original[i].1.clone()).unwrap_or_default();
        for key in ["First", "Last", "Next", "Prev", "Parent", "Count"] {
            d.remove(key.as_bytes());
        }
        d.set("Title", crate::annotations::text_string(&e.title));
        if e.source.is_none() || e.source.is_some_and(|i| described[i].page != e.page) {
            let page = e.page.context("Choose a destination page")?;
            d.remove(b"A");
            d.set(
                "Dest",
                Object::Array(vec![
                    Object::Reference(pages[page]),
                    Object::Name(b"Fit".to_vec()),
                ]),
            );
        }
        stack.truncate(e.level);
        parents.push(stack.last().copied());
        stack.push(nodes.len());
        nodes.push(doc.add_object(d));
    }
    for parent in std::iter::once(None).chain((0..nodes.len()).map(Some)) {
        let children: Vec<_> = parents
            .iter()
            .enumerate()
            .filter_map(|(i, p)| (*p == parent).then_some(i))
            .collect();
        if children.is_empty() {
            continue;
        }
        let parent_id = parent.map_or(root, |i| nodes[i]);
        let descendant_count = if let Some(i) = parent {
            entries
                .iter()
                .skip(i + 1)
                .take_while(|e| e.level > entries[i].level)
                .count()
        } else {
            entries.len()
        };
        let d = doc.get_dictionary_mut(parent_id)?;
        d.set("First", nodes[children[0]]);
        d.set("Last", nodes[*children.last().unwrap()]);
        d.set("Count", descendant_count as i64);
        for (n, &child) in children.iter().enumerate() {
            let d = doc.get_dictionary_mut(nodes[child])?;
            d.set("Parent", parent_id);
            if n > 0 {
                d.set("Prev", nodes[children[n - 1]]);
            }
            if n + 1 < children.len() {
                d.set("Next", nodes[children[n + 1]]);
            }
        }
    }
    let catalog = doc.trailer.get(b"Root")?.as_reference()?;
    if entries.is_empty() {
        doc.get_dictionary_mut(catalog)?.remove(b"Outlines");
    } else {
        doc.get_dictionary_mut(catalog)?.set("Outlines", root);
    }
    let mut out = vec![];
    doc.save_to(&mut out)?;
    Ok(out)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NumberStyle {
    None,
    Decimal,
    RomanLower,
    RomanUpper,
    LettersLower,
    LettersUpper,
}
impl NumberStyle {
    pub const ALL: [(Self, &'static str); 6] = [
        (Self::None, "Prefix only"),
        (Self::Decimal, "1, 2, 3"),
        (Self::RomanLower, "i, ii, iii"),
        (Self::RomanUpper, "I, II, III"),
        (Self::LettersLower, "a, b, c"),
        (Self::LettersUpper, "A, B, C"),
    ];
    fn code(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Decimal => Some("D"),
            Self::RomanLower => Some("r"),
            Self::RomanUpper => Some("R"),
            Self::LettersLower => Some("a"),
            Self::LettersUpper => Some("A"),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LabelRule {
    pub page: usize,
    pub style: NumberStyle,
    pub prefix: String,
    pub start: u32,
}
pub fn label_rules(bytes: &[u8]) -> Result<Vec<LabelRule>> {
    let doc = Document::load_mem(bytes)?;
    let mut rules = vec![];
    fn visit(doc: &Document, o: &Object, rules: &mut Vec<LabelRule>, depth: usize) -> Result<()> {
        ensure!(
            depth < 32 && rules.len() < 10000,
            "Page-label tree exceeds limits"
        );
        let d = doc.dereference(o)?.1.as_dict()?;
        if let Ok(nums) = d.get(b"Nums").and_then(Object::as_array) {
            ensure!(nums.len() % 2 == 0, "Invalid page-label tree");
            for pair in nums.chunks_exact(2) {
                let page = usize::try_from(pair[0].as_i64()?)?;
                let d = doc.dereference(&pair[1])?.1.as_dict()?;
                let code = d.get(b"S").ok().map(Object::as_name).transpose()?;
                let style = match code {
                    None => NumberStyle::None,
                    Some(b"D") => NumberStyle::Decimal,
                    Some(b"r") => NumberStyle::RomanLower,
                    Some(b"R") => NumberStyle::RomanUpper,
                    Some(b"a") => NumberStyle::LettersLower,
                    Some(b"A") => NumberStyle::LettersUpper,
                    _ => anyhow::bail!("Unsupported page numbering style"),
                };
                let prefix = d
                    .get(b"P")
                    .ok()
                    .map(lopdf::decode_text_string)
                    .transpose()?
                    .unwrap_or_default();
                let start = d
                    .get(b"St")
                    .ok()
                    .map(Object::as_i64)
                    .transpose()?
                    .unwrap_or(1);
                ensure!(
                    (1..=100000).contains(&start),
                    "Invalid starting page number"
                );
                rules.push(LabelRule {
                    page,
                    style,
                    prefix,
                    start: start as u32,
                });
            }
        }
        if let Ok(kids) = d.get(b"Kids").and_then(Object::as_array) {
            for kid in kids {
                visit(doc, kid, rules, depth + 1)?;
            }
        }
        Ok(())
    }
    if let Ok(o) = doc.catalog()?.get(b"PageLabels") {
        visit(&doc, o, &mut rules, 0)?;
    }
    rules.sort_by_key(|r| r.page);
    Ok(rules)
}
pub fn set_labels(bytes: &[u8], rules: &[LabelRule]) -> Result<Vec<u8>> {
    let mut doc = crate::content::editable(bytes)?;
    let count = doc.get_pages().len();
    let mut rules = rules.to_vec();
    rules.sort_by_key(|r| r.page);
    ensure!(
        rules.len() <= 10000 && rules.windows(2).all(|w| w[0].page != w[1].page),
        "Each label section must start on a different page"
    );
    if rules.first().is_some_and(|r| r.page > 0) {
        rules.insert(
            0,
            LabelRule {
                page: 0,
                style: NumberStyle::Decimal,
                prefix: String::new(),
                start: 1,
            },
        );
    }
    let mut nums = vec![];
    for r in &rules {
        ensure!(
            r.page < count && r.prefix.len() <= 200 && (1..=100000).contains(&r.start),
            "Invalid page label"
        );
        let mut d =
            dictionary! {"P"=>crate::annotations::text_string(&r.prefix),"St"=>r.start as i64};
        if let Some(code) = r.style.code() {
            d.set("S", Object::Name(code.as_bytes().to_vec()));
        }
        nums.push(Object::Integer(r.page as i64));
        nums.push(Object::Dictionary(d));
    }
    let catalog = doc.trailer.get(b"Root")?.as_reference()?;
    if rules.is_empty() {
        doc.get_dictionary_mut(catalog)?.remove(b"PageLabels");
    } else {
        let id = doc.add_object(dictionary! {"Nums"=>nums});
        doc.get_dictionary_mut(catalog)?.set("PageLabels", id);
    }
    let mut out = vec![];
    doc.save_to(&mut out)?;
    Ok(out)
}
pub fn labels(bytes: &[u8], count: usize) -> Result<Vec<String>> {
    let rules = label_rules(bytes)?;
    Ok((0..count)
        .map(|p| {
            let Some(r) = rules.iter().rev().find(|r| r.page <= p) else {
                return (p + 1).to_string();
            };
            let n = r.start as usize + p - r.page;
            let number = match r.style {
                NumberStyle::None => String::new(),
                NumberStyle::Decimal => n.to_string(),
                NumberStyle::LettersLower | NumberStyle::LettersUpper => {
                    let c = (b'A' + ((n - 1) % 26) as u8) as char;
                    let s = c.to_string().repeat(((n - 1) / 26 + 1).min(4000));
                    if r.style == NumberStyle::LettersLower {
                        s.to_lowercase()
                    } else {
                        s
                    }
                }
                NumberStyle::RomanLower | NumberStyle::RomanUpper => {
                    let mut n = n;
                    let mut s = String::new();
                    for (v, c) in [
                        (1000, "M"),
                        (900, "CM"),
                        (500, "D"),
                        (400, "CD"),
                        (100, "C"),
                        (90, "XC"),
                        (50, "L"),
                        (40, "XL"),
                        (10, "X"),
                        (9, "IX"),
                        (5, "V"),
                        (4, "IV"),
                        (1, "I"),
                    ] {
                        while n >= v {
                            s.push_str(c);
                            n -= v;
                        }
                    }
                    if r.style == NumberStyle::RomanLower {
                        s.to_lowercase()
                    } else {
                        s
                    }
                }
            };
            format!("{}{number}", r.prefix)
        })
        .collect())
}
