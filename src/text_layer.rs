//! Rich text stays in the annotation stack when it is added over page artwork.
use crate::{
    annotations::{self, Annotation, Kind, Style},
    rich_text::Span,
    text_style::{Alignment, TextFormat},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextLayer {
    pub base_size: f32,
    pub base_color: [f32; 3],
    pub spans: Vec<Span>,
    pub alignment: Alignment,
}
impl TextLayer {
    pub fn validate(&self, text: &str) -> Result<()> {
        ensure!(
            self.base_size.is_finite()
                && (4. ..=144.).contains(&self.base_size)
                && self
                    .base_color
                    .iter()
                    .all(|v| v.is_finite() && (0. ..=1.).contains(v)),
            "Invalid base text style"
        );
        ensure!(self.spans.len() <= 2000, "Too many text styles");
        let mut end = 0;
        for s in &self.spans {
            ensure!(
                s.range.start == end
                    && s.range.end <= text.len()
                    && s.range.end >= s.range.start
                    && text.is_char_boundary(s.range.start)
                    && text.is_char_boundary(s.range.end),
                "Invalid text style range"
            );
            ensure!(
                s.size.is_finite()
                    && (4. ..=144.).contains(&s.size)
                    && s.color
                        .iter()
                        .all(|v| v.is_finite() && (0. ..=1.).contains(v)),
                "Invalid text style"
            );
            end = s.range.end;
        }
        ensure!(
            text.is_empty() || end == text.len(),
            "Text styles do not cover this text"
        );
        Ok(())
    }
    /// Files may name a font from another computer. Only read our installed font catalog.
    pub fn local_spans(&self, style: &Style) -> Vec<Span> {
        self.spans
            .iter()
            .cloned()
            .map(|mut s| {
                s.size = (s.size * style.font_size / self.base_size).clamp(4., 144.);
                if style.color != self.base_color {
                    s.color = style.color;
                }
                if let Some(face) = style.font.as_ref().and_then(|f| f.resolve()) {
                    s.font = Some(face.path.clone());
                }
                if !crate::text_style::fonts()
                    .iter()
                    .any(|f| Some(&f.path) == s.font.as_ref())
                {
                    s.font = None;
                }
                s
            })
            .collect()
    }
}
pub fn needed(bytes: &[u8], page: u32) -> Result<bool> {
    let doc = lopdf::Document::load_mem(bytes)?;
    let id = *doc.get_pages().get(&page).context("Page not found")?;
    Ok(doc
        .get_dictionary(id)?
        .get(b"Annots")
        .ok()
        .and_then(|a| doc.dereference(a).ok())
        .and_then(|(_, a)| a.as_array().ok())
        .is_some_and(|a| !a.is_empty()))
}
#[allow(clippy::too_many_arguments)]
pub fn add(
    bytes: &[u8],
    page: u32,
    text: &str,
    rect: [f32; 4],
    size: f32,
    color: [f32; 3],
    format: &TextFormat,
    spans: &[Span],
) -> Result<Vec<u8>> {
    let doc = crate::content::editable(bytes)?;
    let geometry = annotations::page_geometry(bytes)?;
    let bounds = geometry
        .get(page.checked_sub(1).context("Page not found")? as usize)
        .context("Page not found")?
        .0;
    ensure!(
        rect.iter().all(|v| v.is_finite())
            && rect[0] >= 0.
            && rect[1] >= 0.
            && rect[2] >= 10.
            && rect[3] >= 4.
            && rect[0] + rect[2] <= bounds[2] + 0.1
            && rect[1] + rect[3] <= bounds[3] + 0.1,
        "Text block must fit inside the page"
    );
    let mut marks = annotations::load(&doc);
    let mut a = Annotation::new(
        page.saturating_sub(1) as usize,
        [rect[0], rect[1], rect[0] + rect[2], rect[1] + rect[3]],
        Kind::Text { text: text.into() },
        Style {
            font_size: size,
            color,
            ..Default::default()
        },
    );
    a.object.text = Some(TextLayer {
        base_size: size,
        base_color: color,
        alignment: format.alignment,
        spans: if spans.is_empty() {
            vec![Span {
                range: 0..text.len(),
                font: format.font.clone(),
                size,
                color,
            }]
        } else {
            spans.to_vec()
        },
    });
    marks.push(a);
    annotations::export(bytes, &marks)
}
/// Update the same text layer in place; its position relative to other artwork is unchanged.
pub fn replace(
    bytes: &[u8],
    page: u32,
    id: &str,
    expected: &str,
    block: &crate::text_block::Replacement,
) -> Result<Vec<u8>> {
    let doc = crate::content::editable(bytes)?;
    let mut marks = annotations::load(&doc);
    let a = marks
        .iter_mut()
        .find(|a| a.id == id)
        .context("Text layer no longer exists")?;
    ensure!(
        a.page + 1 == page as usize
            && a.kind.text() == Some(expected)
            && a.object.text.is_some()
            && !a.object.locked,
        "The text layer changed; select it again"
    );
    if block.text.is_empty() {
        marks.retain(|a| a.id != id);
    } else {
        a.kind = Kind::Text {
            text: block.text.clone(),
        };
        let [x, y, w, h] = block.rect;
        a.rect = [x, y, x + w, y + h];
        a.style.font_size = block.size;
        a.style.color = block.color;
        a.object.text = Some(TextLayer {
            base_size: block.size,
            base_color: block.color,
            spans: block.spans.clone(),
            alignment: block.format.alignment,
        });
    }
    annotations::export(bytes, &marks)
}
/// A transparent PDF form preserves fonts, Unicode and mixed formatting in other viewers.
pub(crate) fn embed(doc: &mut lopdf::Document, a: &Annotation) -> Result<(lopdf::ObjectId, f32)> {
    let layer = a.object.text.as_ref().context("Missing text layer")?;
    let text = a.kind.text().context("Missing layer text")?;
    layer.validate(text)?;
    let w = a.rect[2] - a.rect[0];
    let h = a.rect[3] - a.rect[1];
    let height = h.max(72.);
    let blank = crate::convert::blank(w.max(72.), height)?;
    let pdf = if text.is_empty() {
        blank
    } else {
        crate::rich_text::add_aligned(
            &blank,
            1,
            text,
            [0., 0., w, h],
            &layer.local_spans(&a.style),
            layer.alignment,
        )?
    };
    let mut foreign = lopdf::Document::load_mem(&pdf)?;
    foreign.renumber_objects_with(doc.max_id + 1);
    let page = *foreign
        .get_pages()
        .values()
        .next()
        .context("No text page")?;
    let resources = crate::content::resources(&foreign, page)?;
    let content = foreign.get_page_content(page);
    doc.max_id = doc.max_id.max(foreign.max_id);
    doc.objects.extend(foreign.objects);
    use lopdf::{Stream, dictionary};
    let id=doc.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Form","BBox"=>vec![0.into(),0.into(),w.max(72.).into(),height.into()],"Resources"=>resources},content));
    Ok((id, height))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn image_page() -> (Vec<u8>, Annotation) {
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(80, 60, image::Rgb([30, 140, 220]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let mark = Annotation::new(
            0,
            [20., 20., 280., 200.],
            Kind::Image {
                png: png.into_inner().into(),
            },
            Style::default(),
        );
        let pdf = annotations::export(
            &crate::convert::blank(300., 240.).unwrap(),
            std::slice::from_ref(&mark),
        )
        .unwrap();
        (pdf, mark)
    }
    fn dark_pixels(bytes: &[u8]) -> usize {
        let (size, pixels) = crate::viewer::render_page(bytes, 0, 1.).unwrap();
        (50..110)
            .flat_map(|y| (40..250).map(move |x| (y * size[0] + x) * 4))
            .filter(|&i| pixels[i] < 50 && pixels[i + 1] < 50 && pixels[i + 2] < 50)
            .count()
    }
    #[test]
    fn text_over_image_stays_visible_editable_and_undoable_after_save() {
        let (source, image) = image_page();
        let rect = [40., 50., 210., 60.];
        let under = crate::content::add_text(&source, 1, "VISIBLE", rect, 26., [0.; 3]).unwrap();
        assert_eq!(
            dark_pixels(&under),
            0,
            "Reproduce the hidden page-content text"
        );
        let output = add(
            &source,
            1,
            "VISIBLE",
            rect,
            26.,
            [0.; 3],
            &TextFormat::default(),
            &[],
        )
        .unwrap();
        assert!(dark_pixels(&output) > 150);
        let marks = annotations::load(&lopdf::Document::load_mem(&output).unwrap());
        assert_eq!(marks[0], image);
        assert_eq!(marks[1].kind.text(), Some("VISIBLE"));
        let saved = annotations::export(&output, &marks).unwrap();
        assert!(dark_pixels(&saved) > 150);
        let mut session = annotations::Session::new(source.clone().into(), vec![image], true);
        session.commit_document(output).unwrap();
        session.undo();
        assert_eq!(session.source.as_ref(), source);
        session.redo();
        assert!(
            dark_pixels(&annotations::export(&session.source, &session.annotations).unwrap()) > 150
        );
    }
    #[test]
    fn editing_rich_text_retains_its_layer_order_and_styles() {
        let (source, image) = image_page();
        let format = TextFormat {
            alignment: Alignment::Center,
            ..Default::default()
        };
        let spans = vec![
            Span {
                range: 0..4,
                font: crate::text_style::fonts().first().map(|f| f.path.clone()),
                size: 22.,
                color: [0.; 3],
            },
            Span {
                range: 4..8,
                font: crate::text_style::fonts().get(1).map(|f| f.path.clone()),
                size: 30.,
                color: [0.8, 0., 0.],
            },
        ];
        let output = add(
            &source,
            1,
            "Two rows",
            [40., 50., 210., 70.],
            22.,
            [0.; 3],
            &format,
            &spans,
        )
        .unwrap();
        let mut marks = annotations::load(&lopdf::Document::load_mem(&output).unwrap());
        let id = marks[1].id.clone();
        let top = Annotation::new(
            0,
            [250., 210., 280., 230.],
            Kind::Rectangle,
            Style::default(),
        );
        marks.push(top.clone());
        let output = annotations::export(&output, &marks).unwrap();
        let block = crate::text_block::Replacement {
            selection: crate::text_block::Selection {
                runs: vec![],
                annotations: vec![(id.clone(), "Two rows".into())],
            },
            spans: crate::rich_text::edited("Two rows", "Two rows!", &spans),
            layout: vec![],
            text: "Two rows!".into(),
            rect: [40., 50., 210., 70.],
            size: 22.,
            color: [0.; 3],
            format,
        };
        let replaced = crate::text_block::replace(&output, 1, &block).unwrap();
        let marks = annotations::load(&lopdf::Document::load_mem(&replaced).unwrap());
        assert_eq!(marks[0], image);
        assert_eq!(marks[1].id, id);
        assert_eq!(marks[2], top);
        assert_eq!(marks[1].kind.text(), Some("Two rows!"));
        assert_eq!(marks[1].object.text.as_ref().unwrap().spans, block.spans);
        assert_eq!(
            marks[1].object.text.as_ref().unwrap().alignment,
            Alignment::Center
        );
    }
}
