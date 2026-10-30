//! Recover a damaged ligature mapping only when the embedded glyph is identical
//! to a shaped ligature in a locally available font with the same exact identity.
//! No document words, spelling heuristics, OCR, or global character replacement.
use hayro::hayro_interpret::font::{FontData, OutlineGlyph};
use std::collections::HashMap;
use ttf_parser::{Face, GlyphId, OutlineBuilder};

const SEQUENCES: [&str; 7] = ["ff", "fi", "fl", "ffi", "ffl", "ft", "st"];
const MAX_OUTLINE_COMMANDS: usize = 4096;

#[derive(Clone, Debug, PartialEq)]
struct Outline {
    commands: Vec<(u8, [u32; 6])>,
    limited: bool,
}
impl Outline {
    fn add(&mut self, tag: u8, values: &[f32]) {
        if self.commands.len() >= MAX_OUTLINE_COMMANDS {
            self.limited = true;
            return;
        }
        let mut coordinates = [0; 6];
        for (destination, value) in coordinates.iter_mut().zip(values) {
            *destination = value.to_bits();
        }
        self.commands.push((tag, coordinates));
    }
    fn glyph(face: &Face<'_>, glyph: GlyphId) -> Option<Self> {
        let mut outline = Self {
            commands: Vec::new(),
            limited: false,
        };
        face.outline_glyph(glyph, &mut outline)?;
        (!outline.limited && !outline.commands.is_empty()).then_some(outline)
    }
}
impl OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.add(0, &[x, y]);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.add(1, &[x, y]);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.add(2, &[x1, y1, x, y]);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.add(3, &[x1, y1, x2, y2, x, y]);
    }
    fn close(&mut self) {
        self.add(4, &[]);
    }
}

#[derive(Clone)]
struct Candidate {
    text: &'static str,
    outline: Outline,
    advance: u16,
}

#[derive(Default)]
pub(crate) struct LigatureRecovery {
    source: Option<FontData>,
    candidates: Vec<Candidate>,
    glyphs: HashMap<u16, Option<&'static str>>,
}
impl LigatureRecovery {
    pub(crate) fn for_font(glyph: &OutlineGlyph) -> Self {
        let Some(data) = glyph.font_data() else {
            return Self::default();
        };
        let Some(name) = glyph.font_name() else {
            return Self::default();
        };
        let Some((full, index)) = crate::render_fonts::exact_font(name) else {
            return Self::default();
        };
        Self::from_fonts(data.data, full, index)
    }

    fn from_fonts(source: FontData, full: FontData, index: u32) -> Self {
        let bytes = source.as_ref().as_ref();
        // The renderer exposes no embedded TTC face index; do not guess one.
        if ttf_parser::fonts_in_collection(bytes).is_some() {
            return Self::default();
        }
        let Ok(source_face) = Face::parse(bytes, 0) else {
            return Self::default();
        };
        let full_bytes = full.as_ref().as_ref();
        let Some(face) = rustybuzz::Face::from_slice(full_bytes, index) else {
            return Self::default();
        };
        if source_face.units_per_em() != face.units_per_em() as u16 {
            return Self::default();
        }
        let mut candidates = Vec::new();
        for text in SEQUENCES {
            let mut buffer = rustybuzz::UnicodeBuffer::new();
            buffer.push_str(text);
            buffer.guess_segment_properties();
            let shaped = rustybuzz::shape(&face, &[], buffer);
            let [glyph] = shaped.glyph_infos() else {
                continue;
            };
            let Ok(id) = u16::try_from(glyph.glyph_id) else {
                continue;
            };
            let glyph = GlyphId(id);
            let (Some(outline), Some(advance)) =
                (Outline::glyph(&face, glyph), face.glyph_hor_advance(glyph))
            else {
                continue;
            };
            candidates.push(Candidate {
                text,
                outline,
                advance,
            });
        }
        if candidates.is_empty() {
            return Self::default();
        }
        Self {
            source: Some(source),
            candidates,
            glyphs: HashMap::new(),
        }
    }

    pub(crate) fn recover(&mut self, glyph: &OutlineGlyph) -> Option<&'static str> {
        self.recover_glyph(u16::try_from(glyph.glyph_id().to_u32()).ok()?)
    }

    fn recover_glyph(&mut self, id: u16) -> Option<&'static str> {
        let source = self.source.as_ref()?;
        *self.glyphs.entry(id).or_insert_with(|| {
            let face = Face::parse(source.as_ref().as_ref(), 0).ok()?;
            let glyph = GlyphId(id);
            let advance = face.glyph_hor_advance(glyph)?;
            let outline = Outline::glyph(&face, glyph)?;
            let mut matches = self
                .candidates
                .iter()
                .filter(|candidate| candidate.advance == advance && candidate.outline == outline);
            let found = matches.next()?;
            // Different possible expansions for one shape are ambiguous.
            if matches.any(|candidate| candidate.text != found.text) {
                return None;
            }
            Some(found.text)
        })
    }
}

#[cfg(all(test, not(target_os = "ios")))]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn font() -> FontData {
        // The app already bundles this licensed font on every desktop platform;
        // Arial and other host fonts need not supply discretionary ligatures.
        let fonts = eframe::egui::FontDefinitions::default();
        Arc::new(fonts.font_data["Ubuntu-Light"].font.clone())
    }
    fn ligature(font: &FontData) -> (u16, &'static str) {
        let face = rustybuzz::Face::from_slice(font.as_ref().as_ref(), 0).unwrap();
        SEQUENCES
            .into_iter()
            .find_map(|text| {
                let mut buffer = rustybuzz::UnicodeBuffer::new();
                buffer.push_str(text);
                buffer.guess_segment_properties();
                let shaped = rustybuzz::shape(&face, &[], buffer);
                let [glyph] = shaped.glyph_infos() else {
                    return None;
                };
                Some((glyph.glyph_id as u16, text))
            })
            .expect("the test font supplies a standard Latin ligature")
    }

    #[test]
    fn exact_outline_and_advance_recover_ligature_but_preserve_real_brackets() {
        let font = font();
        let (id, sequence) = ligature(&font);
        let bracket = Face::parse(font.as_ref().as_ref(), 0)
            .unwrap()
            .glyph_index('[')
            .unwrap();
        let mut recovery = LigatureRecovery::from_fonts(font.clone(), font, 0);
        assert_eq!(recovery.recover_glyph(id), Some(sequence));
        assert_eq!(recovery.recover_glyph(id), Some(sequence));
        assert_eq!(recovery.recover_glyph(bracket.0), None);
    }

    #[test]
    fn changed_outline_or_advance_and_ambiguous_expansion_are_not_corrected() {
        let font = font();
        let (id, _) = ligature(&font);
        let mut changed_advance = LigatureRecovery::from_fonts(font.clone(), font.clone(), 0);
        for candidate in &mut changed_advance.candidates {
            candidate.advance = candidate.advance.saturating_add(1);
        }
        assert_eq!(changed_advance.recover_glyph(id), None);
        let mut changed_outline = LigatureRecovery::from_fonts(font.clone(), font.clone(), 0);
        for candidate in &mut changed_outline.candidates {
            candidate.outline.commands[0].1[0] ^= 1;
        }
        assert_eq!(changed_outline.recover_glyph(id), None);
        let mut ambiguous = LigatureRecovery::from_fonts(font.clone(), font, 0);
        // Every candidate is duplicated so this remains independent of which
        // ligature the platform test font shapes first.
        let duplicates: Vec<_> = ambiguous
            .candidates
            .iter()
            .cloned()
            .map(|mut c| {
                c.text = "different";
                c
            })
            .collect();
        ambiguous.candidates.extend(duplicates);
        assert_eq!(ambiguous.recover_glyph(id), None);
    }
}
