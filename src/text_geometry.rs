//! Locate exact text-showing operators, including repeated identical text.
//! IDs are painted into an analysis-only copy; the document being edited is never altered.
use anyhow::{Context as _, Result, ensure};
use hayro::{
    hayro_interpret::{
        self as interpret, BlendMode, ClipPath, Device, DrawMode, DrawProps, Image, ImageDrawProps,
        SoftMask, TransformExt, font::Glyph,
    },
    hayro_syntax::Pdf,
    kurbo::{BezPath, Point, Rect, Shape},
};
use lopdf::{
    Dictionary, Object, Stream,
    content::{Content, Operation},
};
use std::collections::HashMap;

pub fn runs(bytes: &[u8], page: u32) -> Result<Vec<crate::document::TextRun>> {
    let mut runs = crate::document::text_runs(bytes, page)?;
    if runs.is_empty() {
        return Ok(runs);
    }
    ensure!(
        runs.len() <= 20_000,
        "This page has too many text objects for editing"
    );
    let mut doc = lopdf::Document::load_mem(bytes)?;
    let id = *doc.get_pages().get(&page).context("Page not found")?;
    let source = doc.get_and_decode_page_content(id)?;
    let ids: HashMap<_, _> = runs
        .iter()
        .enumerate()
        .map(|(i, r)| (r.operator_index, i + 1))
        .collect();
    let mut operations = Vec::new();
    for (index, op) in source.operations.into_iter().enumerate() {
        // XObject text is not edited by the page-level writer; exclude it from this lookup.
        if op.operator == "Do" {
            continue;
        }
        if let Some(tag) = ids.get(&index) {
            let rgb: Vec<Object> = [(tag >> 16) & 255, (tag >> 8) & 255, tag & 255]
                .into_iter()
                .map(|c| Object::Real(c as f32 / 255.))
                .collect();
            operations.push(Operation::new("rg", rgb.clone()));
            operations.push(Operation::new("RG", rgb));
        }
        operations.push(op);
    }
    let stream = doc.add_object(Stream::new(
        Dictionary::new(),
        Content { operations }.encode()?,
    ));
    let page_dict = doc.get_dictionary_mut(id)?;
    page_dict.set("Contents", stream);
    page_dict.remove(b"Annots");
    // RGB IDs must not be remapped by an inherited DefaultRGB color space.
    let mut resources = crate::content::resources(&doc, id)?;
    if let Ok(cs) = resources
        .get(b"ColorSpace")
        .and_then(|o| doc.dereference(o).map(|(_, o)| o))
        .and_then(Object::as_dict)
    {
        let mut cs = cs.clone();
        cs.remove(b"DefaultRGB");
        resources.set("ColorSpace", cs);
    }
    doc.get_dictionary_mut(id)?.set("Resources", resources);
    let mut analysis = Vec::new();
    doc.save_to(&mut analysis)?;
    let pdf = Pdf::new(analysis).map_err(|e| anyhow::anyhow!("Could not locate text: {e:?}"))?;
    let page = pdf
        .pages()
        .get(page as usize - 1)
        .context("Page not found")?;
    let cache = interpret::InterpreterCache::new();
    let (w, h) = page.render_dimensions();
    let mut context = interpret::Context::new(
        page.initial_transform(true).to_kurbo(),
        Rect::new(0., 0., w as f64, h as f64),
        &cache,
        page.xref(),
        interpret::InterpreterSettings::default(),
    );
    let mut device = Geometry {
        boxes: HashMap::new(),
        count: 0,
    };
    interpret::interpret_page(page, &mut context, &mut device);
    ensure!(
        device.count <= 100_000,
        "This page has too many glyphs for editing"
    );
    for (index, run) in runs.iter_mut().enumerate() {
        if let Some((rect, size)) = device.boxes.get(&(index + 1)) {
            run.rect = Some([
                rect.x0 as f32,
                rect.y0 as f32,
                rect.x1 as f32,
                rect.y1 as f32,
            ]);
            run.size = *size;
        }
    }
    Ok(runs)
}
struct Geometry {
    boxes: HashMap<usize, (Rect, f32)>,
    count: usize,
}
impl<'a> Device<'a> for Geometry {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
    fn draw_glyph_run(
        &mut self,
        run: &interpret::font::GlyphRun<'_, 'a>,
        props: DrawProps<'a>,
        mode: &DrawMode,
    ) {
        if matches!(mode, DrawMode::Invisible) {
            return;
        }
        let interpret::Paint::Color(color) = props.paint else {
            return;
        };
        let [r, g, b, _] = color.to_rgba().to_rgba8();
        let id = ((r as usize) << 16) | ((g as usize) << 8) | b as usize;
        if id == 0 {
            return;
        }
        for glyph in run.glyphs() {
            self.count += 1;
            if self.count > 100_000 {
                return;
            }
            let transform = props.transform * glyph.transform();
            let origin = transform * Point::ZERO;
            let size = ((transform * Point::new(0., 1000.)) - origin).hypot() as f32;
            let rect = match &**glyph {
                Glyph::Outline(g) if !g.outline().is_empty() => {
                    (transform * g.outline()).bounding_box()
                }
                _ => (transform * Rect::new(0., -200., 500., 800.).to_path(0.1)).bounding_box(),
            };
            if rect.is_finite() && size.is_finite() {
                self.boxes
                    .entry(id)
                    .and_modify(|(r, s)| {
                        *r = r.union(rect);
                        *s = s.max(size);
                    })
                    .or_insert((rect, size));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn geometry_tracks_crop_and_rotation_without_matching_the_wrong_occurrence() {
        let blank = crate::convert::blank(595., 842.).unwrap();
        let mut bytes = crate::content::add_text(
            &blank,
            1,
            "Rotate me",
            [50., 80., 200., 60.],
            18.,
            [0., 0., 0.],
        )
        .unwrap();
        bytes = crate::pages::apply(
            &bytes,
            &crate::pages::Operation::Crop {
                pages: vec![0],
                margins: [18.; 4],
            },
        )
        .unwrap();
        for _ in 0..4 {
            let located = runs(&bytes, 1).unwrap()[0].rect.unwrap();
            let pdf = Pdf::new(bytes.clone()).unwrap();
            let text =
                crate::viewer::extract_page(&pdf.pages()[0], &interpret::InterpreterCache::new());
            let hit = text.find("Rotate me", 0).remove(0);
            let rect = hit
                .rects
                .iter()
                .map(|r| Rect::new(r[0] as f64, r[1] as f64, r[2] as f64, r[3] as f64))
                .reduce(|a, b| a.union(b))
                .unwrap();
            for (a, b) in located
                .into_iter()
                .zip([rect.x0, rect.y0, rect.x1, rect.y1])
            {
                assert!((a as f64 - b).abs() < 0.1);
            }
            bytes = crate::pages::apply(
                &bytes,
                &crate::pages::Operation::Rotate {
                    pages: vec![0],
                    clockwise: true,
                },
            )
            .unwrap();
        }
    }
    #[test]
    fn exact_operator_geometry_and_unicode_replacement_preserve_other_occurrences() {
        let source = crate::convert::blank(595., 842.).unwrap();
        let source = crate::content::add_text(
            &source,
            1,
            "Repeated",
            [40., 80., 200., 60.],
            18.,
            [0., 0.5, 0.],
        )
        .unwrap();
        let source = crate::content::add_text(
            &source,
            1,
            "Repeated",
            [220., 220., 200., 60.],
            18.,
            [0., 0., 0.],
        )
        .unwrap();
        let positioned = runs(&source, 1).unwrap();
        assert_eq!(positioned.len(), 2);
        let first = positioned[0].rect.unwrap();
        let second = positioned[1].rect.unwrap();
        assert!((first[0] - 40.).abs() < 5. && (second[0] - 220.).abs() < 5.);
        assert!(first[1] > 70. && first[3] < 120. && second[1] > 210. && second[3] < 260.);
        assert!(
            (positioned[0].size - 18.).abs() < 1.,
            "font size {}",
            positioned[0].size
        );
        let changed = crate::document::replace_text_object(
            &source,
            1,
            positioned[1].operator_index,
            "Repeated",
            "New text: İstanbul!",
        )
        .unwrap();
        let updated = runs(&changed, 1).unwrap();
        assert_eq!(updated[0].text, "Repeated");
        assert_eq!(updated[1].text, "New text: İstanbul!");
        assert!((updated[1].rect.unwrap()[0] - second[0]).abs() < 5.);
    }
}
