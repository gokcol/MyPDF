//! Replace a rectangular text selection as one reflowable, selectable PDF block.
use anyhow::{Context, Result, ensure};
use lopdf::{Dictionary, Object, Stream, StringFormat};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq)]
pub struct Selection {
    /// Stable text-showing ordinals and expected contents on the selected page.
    pub runs: Vec<(usize, String)>,
    pub annotations: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
pub struct Replacement {
    pub selection: Selection,
    pub text: String,
    /// Displayed page coordinates: left, top, width, height.
    pub rect: [f32; 4],
    pub size: f32,
    pub color: [f32; 3],
    pub format: crate::text_style::TextFormat,
}

pub fn replace(bytes: &[u8], page: u32, block: &Replacement) -> Result<Vec<u8>> {
    let mut doc = crate::content::editable(bytes)?;
    let id = *doc.get_pages().get(&page).context("Page not found")?;
    ensure!(
        block.selection.runs.len() + block.selection.annotations.len() <= 2000,
        "Select at most 2,000 text objects at a time"
    );
    ensure!(
        !block.selection.runs.is_empty() || !block.selection.annotations.is_empty(),
        "Select some text first"
    );
    let before = crate::text_geometry::runs(bytes, page)?;
    let mut selected = HashSet::new();
    let mut operators = HashSet::new();
    for (ordinal, expected) in &block.selection.runs {
        let run = before
            .get(*ordinal)
            .context("Selected text no longer exists")?;
        ensure!(
            run.text == *expected && run.rect.is_some(),
            "The text changed; select it again"
        );
        ensure!(selected.insert(*ordinal), "Duplicate text selection");
        operators.insert(run.operator_index);
    }
    let mut content = doc.get_and_decode_page_content(id)?;
    let mut render_mode = 0;
    let mut stack = Vec::new();
    for (index, op) in content.operations.iter_mut().enumerate() {
        match op.operator.as_str() {
            "q" => stack.push(render_mode),
            "Q" => render_mode = stack.pop().context("Unbalanced graphics state")?,
            "Tr" => {
                render_mode = op
                    .operands
                    .first()
                    .context("Missing text render mode")?
                    .as_i64()?
            }
            _ => {}
        }
        if operators.contains(&index) {
            ensure!(
                render_mode < 4,
                "Text used as a clipping mask cannot be grouped"
            );
            // Keep quote operators' line movement/spacing but remove their actual
            // strings. Never conceal the original words in an invisible text layer.
            let empty = Object::String(vec![], StringFormat::Hexadecimal);
            match op.operator.as_str() {
                "TJ" => op.operands = vec![Object::Array(vec![empty])],
                "\"" => op.operands[2] = empty,
                _ => op.operands[0] = empty,
            }
        }
    }
    let stream = doc.add_object(Stream::new(Dictionary::new(), content.encode()?));
    doc.get_dictionary_mut(id)?.set("Contents", stream);
    let mut annotations = crate::annotations::load(&doc);
    let mut ids = HashSet::new();
    for (id, expected) in &block.selection.annotations {
        let a = annotations
            .iter()
            .find(|a| &a.id == id)
            .context("Text box no longer exists")?;
        ensure!(
            a.page + 1 == page as usize
                && matches!(&a.kind, crate::annotations::Kind::Text { text } if text == expected),
            "The text box changed; select it again"
        );
        ensure!(ids.insert(id.clone()), "Duplicate text box selection");
    }
    annotations.retain(|a| !ids.contains(&a.id));
    let stripped = crate::content::serialize(&mut doc)?;
    let stripped = if ids.is_empty() {
        stripped
    } else {
        crate::annotations::export(&stripped, &annotations)?
    };
    // A removed show operation may have advanced another, unselected run in the
    // same text object. Reject that transaction rather than moving neighboring text.
    let after = crate::text_geometry::runs(&stripped, page)?;
    ensure!(
        before.len() == after.len(),
        "Text structure changed unexpectedly"
    );
    for (ordinal, (a, b)) in before.iter().zip(&after).enumerate() {
        if selected.contains(&ordinal) {
            continue;
        }
        let unchanged = match (a.rect, b.rect) {
            (Some(a), Some(b)) => a.into_iter().zip(b).all(|(a, b)| (a - b).abs() < 0.1),
            (None, None) => true,
            _ => false,
        };
        ensure!(
            a.text == b.text && unchanged,
            "This selection shares positioning with nearby text. Include the complete line and try again."
        );
    }
    if block.text.trim().is_empty() {
        return Ok(stripped);
    }
    crate::content::add_text_styled(
        &stripped,
        page,
        &block.text,
        block.rect,
        block.size,
        block.color,
        &block.format,
    )
}
