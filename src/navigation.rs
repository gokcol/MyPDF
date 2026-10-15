//! Moving around a PDF: its outline (bookmarks) and the links on its pages.
use anyhow::Result;
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::{HashMap, HashSet};

/// One entry of the document's outline.
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineItem {
    /// Nesting depth, from 0.
    pub level: usize,
    pub title: String,
    /// The page it goes to, from 0, when that is in this document.
    pub page: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// A page of this document, from 0.
    Page(usize),
    /// A web or mail address.
    Uri(String),
}
/// A link area on a page.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub page: usize,
    /// Displayed page coordinates: left, top, right, bottom.
    pub rect: [f32; 4],
    pub target: Target,
}

fn resolve<'a>(doc: &'a Document, o: &'a Object) -> &'a Object {
    doc.dereference(o).map(|(_, o)| o).unwrap_or(o)
}
fn text(o: &Object) -> String {
    lopdf::decode_text_string(o).unwrap_or_else(|_| {
        o.as_name()
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .unwrap_or_default()
    })
}
fn pages(doc: &Document) -> HashMap<ObjectId, usize> {
    doc.get_pages()
        .into_values()
        .enumerate()
        .map(|(i, id)| (id, i))
        .collect()
}
/// Look a name up in a PDF name tree.
fn name_tree<'a>(
    doc: &'a Document,
    node: &'a Dictionary,
    key: &[u8],
    depth: usize,
) -> Option<&'a Object> {
    if depth > 32 {
        return None;
    }
    if let Ok(names) = node
        .get(b"Names")
        .map(|o| resolve(doc, o))
        .and_then(Object::as_array)
    {
        for pair in names.chunks(2) {
            if let [k, v] = pair
                && resolve(doc, k).as_str().ok() == Some(key)
            {
                return Some(v);
            }
        }
    }
    let kids = node
        .get(b"Kids")
        .map(|o| resolve(doc, o))
        .and_then(Object::as_array)
        .ok()?;
    kids.iter().find_map(|kid| {
        let kid = resolve(doc, kid).as_dict().ok()?;
        // Skip subtrees whose range does not contain the name.
        if let Ok(limits) = kid.get(b"Limits").and_then(Object::as_array)
            && let [low, high] = limits.as_slice()
            && let (Ok(low), Ok(high)) = (resolve(doc, low).as_str(), resolve(doc, high).as_str())
            && (key < low || key > high)
        {
            return None;
        }
        name_tree(doc, kid, key, depth + 1)
    })
}
/// The page a destination goes to: an explicit destination, a named one, or
/// an action's destination.
fn destination(
    doc: &Document,
    dest: &Object,
    pages: &HashMap<ObjectId, usize>,
    depth: usize,
) -> Option<usize> {
    if depth > 8 {
        return None;
    }
    match resolve(doc, dest) {
        Object::Array(a) => match a.first()? {
            Object::Reference(page) => pages.get(page).copied(),
            // Some writers give the page number itself.
            Object::Integer(n) => usize::try_from(*n).ok().filter(|n| *n < pages.len()),
            _ => None,
        },
        Object::Dictionary(d) => destination(doc, d.get(b"D").ok()?, pages, depth + 1),
        named @ (Object::Name(_) | Object::String(..)) => {
            let key: &[u8] = match named {
                Object::Name(n) => n,
                Object::String(s, _) => s,
                _ => unreachable!(),
            };
            let catalog = doc.catalog().ok()?;
            let found = catalog
                .get(b"Dests")
                .map(|o| resolve(doc, o))
                .and_then(Object::as_dict)
                .ok()
                .and_then(|d| d.get(key).ok())
                .or_else(|| {
                    let names = resolve(doc, catalog.get(b"Names").ok()?).as_dict().ok()?;
                    let tree = resolve(doc, names.get(b"Dests").ok()?).as_dict().ok()?;
                    name_tree(doc, tree, key, 0)
                })?;
            destination(doc, found, pages, depth + 1)
        }
        _ => None,
    }
}
/// What following an action does, for go-to and web actions only.
fn action(
    doc: &Document,
    a: &Dictionary,
    page: usize,
    pages: &HashMap<ObjectId, usize>,
) -> Option<Target> {
    match a.get(b"S").and_then(Object::as_name).ok()? {
        b"URI" => {
            let uri = text(resolve(doc, a.get(b"URI").ok()?)).trim().to_owned();
            let lower = uri.to_ascii_lowercase();
            (lower.starts_with("https://")
                || lower.starts_with("http://")
                || lower.starts_with("mailto:"))
            .then_some(Target::Uri(uri))
        }
        b"GoTo" => destination(doc, a.get(b"D").ok()?, pages, 0).map(Target::Page),
        b"Named" => {
            let last = pages.len().saturating_sub(1);
            Some(Target::Page(
                match a.get(b"N").and_then(Object::as_name).ok()? {
                    b"NextPage" => (page + 1).min(last),
                    b"PrevPage" => page.saturating_sub(1),
                    b"FirstPage" => 0,
                    b"LastPage" => last,
                    _ => return None,
                },
            ))
        }
        _ => None,
    }
}

/// The document's outline, in reading order.
pub fn outline(bytes: &[u8]) -> Result<Vec<OutlineItem>> {
    let doc = Document::load_mem(bytes)?;
    let pages = pages(&doc);
    let mut out = vec![];
    let Some(root) = doc
        .catalog()?
        .get(b"Outlines")
        .ok()
        .map(|o| resolve(&doc, o))
        .and_then(|o| o.as_dict().ok())
    else {
        return Ok(out);
    };
    let mut stack: Vec<(ObjectId, usize)> = root
        .get(b"First")
        .and_then(Object::as_reference)
        .ok()
        .map(|id| (id, 0))
        .into_iter()
        .collect();
    let mut seen = HashSet::new();
    while let Some((id, level)) = stack.pop() {
        if !seen.insert(id) || out.len() >= 10_000 || level > 32 {
            continue;
        }
        let Ok(item) = doc.get_dictionary(id) else {
            continue;
        };
        let page = item
            .get(b"Dest")
            .ok()
            .and_then(|d| destination(&doc, d, &pages, 0))
            .or_else(|| {
                let a = resolve(&doc, item.get(b"A").ok()?).as_dict().ok()?;
                match action(&doc, a, 0, &pages)? {
                    Target::Page(p) => Some(p),
                    Target::Uri(_) => None,
                }
            });
        out.push(OutlineItem {
            level,
            title: item
                .get(b"Title")
                .map(|t| text(resolve(&doc, t)))
                .unwrap_or_default()
                .trim()
                .to_owned(),
            page,
        });
        // The next sibling waits until this item's children are done.
        if let Ok(next) = item.get(b"Next").and_then(Object::as_reference) {
            stack.push((next, level));
        }
        if let Ok(first) = item.get(b"First").and_then(Object::as_reference) {
            stack.push((first, level + 1));
        }
    }
    Ok(out)
}

/// Every link on every page that goes to a page or a web or mail address.
pub fn links(bytes: &[u8]) -> Result<Vec<Link>> {
    let doc = Document::load_mem(bytes)?;
    let geometry = crate::annotations::page_geometry(bytes)?;
    let pages = pages(&doc);
    let mut out = vec![];
    for (index, page) in doc.get_pages().into_values().enumerate() {
        let Some((_, inverse)) = geometry.get(index) else {
            break;
        };
        let annots = doc
            .get_dictionary(page)?
            .get(b"Annots")
            .ok()
            .map(|o| resolve(&doc, o))
            .and_then(|o| o.as_array().ok().cloned())
            .unwrap_or_default();
        for a in &annots {
            let Ok(d) = resolve(&doc, a).as_dict() else {
                continue;
            };
            if d.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Link".as_slice()) {
                continue;
            }
            let Ok(r) = d.get(b"Rect").and_then(Object::as_array) else {
                continue;
            };
            let r: Vec<f32> = r
                .iter()
                .filter_map(|v| resolve(&doc, v).as_float().ok())
                .collect();
            if r.len() != 4 {
                continue;
            }
            let target = d
                .get(b"A")
                .ok()
                .and_then(|a| resolve(&doc, a).as_dict().ok())
                .and_then(|a| action(&doc, a, index, &pages))
                .or_else(|| destination(&doc, d.get(b"Dest").ok()?, &pages, 0).map(Target::Page));
            let Some(target) = target else { continue };
            let rect = crate::content::displayed_rect(
                [
                    r[0].min(r[2]),
                    r[1].min(r[3]),
                    r[0].max(r[2]),
                    r[1].max(r[3]),
                ],
                *inverse,
            );
            out.push(Link {
                page: index,
                rect,
                target,
            });
            if out.len() >= 20_000 {
                return Ok(out);
            }
        }
    }
    Ok(out)
}
