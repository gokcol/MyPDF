//! Installed substitutes for PDF fonts that are not embedded. Font programs
//! remain local; embedded PDF fonts are never replaced by this resolver.
use hayro::hayro_interpret::{
    FontResolverFn, InterpreterSettings,
    font::{FallbackFontQuery, FontData, FontQuery, FontStretch, StandardFont},
    hayro_cmap::CidFamily,
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};

const MAX_FONT_BYTES: u64 = 80 * 1024 * 1024;
const FONT_CACHE_BYTES: usize = 160 * 1024 * 1024;
type Resolved = (FontData, u32);

/// Use the same resolver for rasterization, indexing and extraction. PDF text
/// positioning is preserved. Font substitution cannot reconstruct missing
/// encodings/Unicode maps or add shaping that an original PDF omitted.
pub fn settings() -> InterpreterSettings {
    static RESOLVER: OnceLock<FontResolverFn> = OnceLock::new();
    let resolver = RESOLVER.get_or_init(|| {
        let original = InterpreterSettings::default().font_resolver;
        let mut faces: Vec<Option<FontData>> = vec![None; 12];
        for face in crate::text_style::fonts() {
            let family = match face.family.as_str() {
                "Times New Roman" | "DejaVuSerif" => 0,
                "Arial" | "DejaVuSans" => 4,
                "Courier New" | "DejaVuSansMono" => 8,
                _ => continue,
            };
            let slot = family + usize::from(face.bold) + 2 * usize::from(face.italic);
            let Ok(bytes) = crate::convert::read_bounded(&face.path, MAX_FONT_BYTES) else {
                continue;
            };
            // Validate the face rather than hard-coding one language. Installed
            // Unicode alternatives also cover Greek, Cyrillic, Hebrew and Arabic
            // where those glyphs are supplied by the chosen font.
            if ttf_parser::Face::parse(&bytes, 0).is_ok_and(|font| {
                "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"
                    .chars()
                    .all(|character| font.glyph_index(character).is_some())
            }) {
                faces[slot] = Some(Arc::new(bytes));
            }
        }
        Arc::new(move |query| {
            if let FontQuery::Fallback(fallback) = query
                && let Some(font) = system_fonts().resolve(fallback)
            {
                return Some(font);
            }
            let standard = match query {
                FontQuery::Standard(font) => *font,
                FontQuery::Fallback(query) => query.pick_standard_font(),
            };
            if let Some(slot) = standard_slot(standard)
                && let Some(data) = &faces[slot]
            {
                return Some((data.clone(), 0));
            }
            original(query)
        })
    });
    InterpreterSettings {
        font_resolver: resolver.clone(),
        ..InterpreterSettings::default()
    }
}

struct SystemFonts {
    database: fontdb::Database,
    /// Faces in a TTC share one owned byte buffer, including regular/bold pairs.
    files: Mutex<FontFiles>,
}
// This bounds cache-held bytes. Active documents may retain their own Arcs;
// evicting an old cache entry must not invalidate an already-open PDF's font.
#[derive(Default)]
struct FontFiles {
    entries: HashMap<PathBuf, (FontData, u64)>,
    clock: u64,
}
impl FontFiles {
    fn get(&mut self, path: &std::path::Path) -> Option<FontData> {
        let (data, used) = self.entries.get_mut(path)?;
        self.clock = self.clock.saturating_add(1);
        *used = self.clock;
        Some(data.clone())
    }
    fn insert(&mut self, path: PathBuf, data: FontData, budget: usize) {
        let size = data.as_ref().as_ref().len();
        if size > budget {
            return;
        }
        self.entries.remove(&path);
        while self
            .entries
            .values()
            .map(|(data, _)| data.as_ref().as_ref().len())
            .sum::<usize>()
            .saturating_add(size)
            > budget
        {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(path, _)| path.clone())
            else {
                break;
            };
            self.entries.remove(&oldest);
        }
        self.clock = self.clock.saturating_add(1);
        self.entries.insert(path, (data, self.clock));
    }
}
fn system_fonts() -> &'static SystemFonts {
    static FONTS: OnceLock<SystemFonts> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut database = fontdb::Database::new();
        database.load_system_fonts();
        SystemFonts {
            database,
            files: Mutex::default(),
        }
    })
}
impl SystemFonts {
    fn face(&self, id: fontdb::ID, required: &str) -> Option<Resolved> {
        let info = self.database.face(id)?;
        let data: FontData = match &info.source {
            fontdb::Source::File(path) => {
                let mut files = self.files.lock().unwrap_or_else(|error| error.into_inner());
                if let Some(data) = files.get(path) {
                    data.clone()
                } else {
                    let bytes = crate::convert::read_bounded(path, MAX_FONT_BYTES).ok()?;
                    if !usable_face(&bytes, info.index, required) {
                        return None;
                    }
                    let data: FontData = Arc::new(bytes);
                    files.insert(path.clone(), data.clone(), FONT_CACHE_BYTES);
                    data
                }
            }
            fontdb::Source::Binary(data) | fontdb::Source::SharedFile(_, data) => data.clone(),
        };
        if !usable_face(data.as_ref().as_ref(), info.index, required) {
            return None;
        }
        Some((data, info.index))
    }
    fn family(&self, name: &str, query: &FallbackFontQuery, required: &str) -> Option<Resolved> {
        let families = [fontdb::Family::Name(name)];
        let id = self.database.query(&fontdb::Query {
            families: &families,
            weight: fontdb::Weight(
                if query.is_bold {
                    query.font_weight.max(700)
                } else {
                    query.font_weight
                }
                .clamp(1, 1000) as u16,
            ),
            style: if query.is_italic {
                fontdb::Style::Italic
            } else {
                fontdb::Style::Normal
            },
            stretch: stretch(query.font_stretch),
        })?;
        self.face(id, required)
    }
    fn resolve(&self, query: &FallbackFontQuery) -> Option<Resolved> {
        let (families, required) = regional_families(query);
        for name in [
            query.post_script_name.as_deref(),
            query.font_name.as_deref(),
            query.font_family.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            let name = strip_subset(name);
            if let Some(id) = self
                .database
                .faces()
                .find(|face| face.post_script_name.eq_ignore_ascii_case(name))
                .map(|face| face.id)
                && let Some(font) = self.face(id, required)
            {
                return Some(font);
            }
            if let Some(font) = self.family(name, query, required) {
                return Some(font);
            }
        }
        for name in families
            .iter()
            .copied()
            .chain(["Arial Unicode MS", "Arial Unicode"])
        {
            if let Some(font) = self.family(name, query, required) {
                return Some(font);
            }
        }
        None
    }
}
/// Resolve only an exact PostScript font identity for verifying embedded glyph
/// outlines. A similar family or fallback font cannot justify a text correction.
pub(crate) fn exact_font(name: &str) -> Option<Resolved> {
    let name = strip_subset(name);
    if name.is_empty() || name.len() > 256 {
        return None;
    }
    fn find(fonts: &SystemFonts, name: &str) -> Option<Resolved> {
        let id = fonts
            .database
            .faces()
            .find(|face| face.post_script_name.eq_ignore_ascii_case(name))?
            .id;
        fonts.face(id, "")
    }
    if let Some(font) = find(system_fonts(), name) {
        return Some(font);
    }
    // Office's private fonts are not necessarily registered with the OS. Read
    // only its font resources when present, never its documents or preferences.
    #[cfg(target_os = "macos")]
    {
        static OFFICE: OnceLock<SystemFonts> = OnceLock::new();
        let fonts = OFFICE.get_or_init(|| {
            let mut database = fontdb::Database::new();
            let directory =
                std::path::Path::new("/Applications/Microsoft Word.app/Contents/Resources/DFonts");
            if let Ok(entries) = std::fs::read_dir(directory) {
                let mut paths: Vec<_> = entries
                    .take(2048)
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .filter(|path| {
                        path.extension().is_some_and(|extension| {
                            extension.to_str().is_some_and(|extension| {
                                ["ttf", "otf", "ttc"]
                                    .iter()
                                    .any(|wanted| extension.eq_ignore_ascii_case(wanted))
                            })
                        })
                    })
                    .collect();
                paths.sort();
                for path in paths {
                    if std::fs::metadata(&path)
                        .is_ok_and(|meta| meta.is_file() && meta.len() <= MAX_FONT_BYTES)
                    {
                        let _ = database.load_font_file(path);
                    }
                }
            }
            SystemFonts {
                database,
                files: Mutex::default(),
            }
        });
        if let Some(font) = find(fonts, name) {
            return Some(font);
        }
    }
    None
}

fn usable_face(bytes: &[u8], index: u32, required: &str) -> bool {
    bytes.len() as u64 <= MAX_FONT_BYTES
        && ttf_parser::Face::parse(bytes, index).is_ok_and(|font| {
            font.number_of_glyphs() > 0
                && required
                    .chars()
                    .all(|character| font.glyph_index(character).is_some())
        })
}
fn strip_subset(name: &str) -> &str {
    if name.len() > 7
        && name.as_bytes().get(6) == Some(&b'+')
        && name.as_bytes()[..6].iter().all(u8::is_ascii_uppercase)
    {
        &name[7..]
    } else {
        name
    }
}
fn regional_families(query: &FallbackFontQuery) -> (&'static [&'static str], &'static str) {
    match query
        .character_collection
        .as_ref()
        .map(|collection| &collection.family)
    {
        Some(CidFamily::AdobeGB1) if query.is_serif => (
            &[
                "Songti SC",
                "Noto Serif CJK SC",
                "Noto Serif SC",
                "SimSun",
                "Hiragino Sans GB",
            ],
            "中文简",
        ),
        Some(CidFamily::AdobeGB1) => (
            &[
                "Hiragino Sans GB",
                "PingFang SC",
                "Noto Sans CJK SC",
                "Noto Sans SC",
                "Microsoft YaHei",
                "SimHei",
            ],
            "中文简",
        ),
        Some(CidFamily::AdobeCNS1) if query.is_serif => (
            &[
                "Songti TC",
                "Noto Serif CJK TC",
                "Noto Serif TC",
                "PMingLiU",
                "MingLiU",
                "Heiti TC",
            ],
            "中文繁體",
        ),
        Some(CidFamily::AdobeCNS1) => (
            &[
                "Heiti TC",
                "PingFang TC",
                "Noto Sans CJK TC",
                "Noto Sans TC",
                "Microsoft JhengHei",
                "Hiragino Sans GB",
            ],
            "中文繁體",
        ),
        Some(CidFamily::AdobeJapan1) if query.is_serif => (
            &[
                "Hiragino Mincho ProN",
                "Yu Mincho",
                "Noto Serif CJK JP",
                "Noto Serif JP",
                "MS Mincho",
                "Hiragino Sans",
            ],
            "日本語かなカナ",
        ),
        Some(CidFamily::AdobeJapan1) => (
            &[
                "Hiragino Sans",
                "Hiragino Kaku Gothic ProN",
                "Yu Gothic",
                "Meiryo",
                "Noto Sans CJK JP",
                "Noto Sans JP",
            ],
            "日本語かなカナ",
        ),
        Some(CidFamily::AdobeKorea1) if query.is_serif => (
            &[
                "AppleMyungjo",
                "Batang",
                "Noto Serif CJK KR",
                "Noto Serif KR",
                "Apple SD Gothic Neo",
            ],
            "한글",
        ),
        Some(CidFamily::AdobeKorea1) => (
            &[
                "Apple SD Gothic Neo",
                "Malgun Gothic",
                "Noto Sans CJK KR",
                "Noto Sans KR",
            ],
            "한글",
        ),
        _ => (&[], ""),
    }
}
fn stretch(value: FontStretch) -> fontdb::Stretch {
    match value {
        FontStretch::Normal => fontdb::Stretch::Normal,
        FontStretch::UltraCondensed => fontdb::Stretch::UltraCondensed,
        FontStretch::ExtraCondensed => fontdb::Stretch::ExtraCondensed,
        FontStretch::Condensed => fontdb::Stretch::Condensed,
        FontStretch::SemiCondensed => fontdb::Stretch::SemiCondensed,
        FontStretch::SemiExpanded => fontdb::Stretch::SemiExpanded,
        FontStretch::Expanded => fontdb::Stretch::Expanded,
        FontStretch::ExtraExpanded => fontdb::Stretch::ExtraExpanded,
        FontStretch::UltraExpanded => fontdb::Stretch::UltraExpanded,
    }
}
fn standard_slot(font: StandardFont) -> Option<usize> {
    Some(match font {
        StandardFont::TimesRoman => 0,
        StandardFont::TimesBold => 1,
        StandardFont::TimesItalic => 2,
        StandardFont::TimesBoldItalic => 3,
        StandardFont::Helvetica => 4,
        StandardFont::HelveticaBold => 5,
        StandardFont::HelveticaOblique => 6,
        StandardFont::HelveticaBoldOblique => 7,
        StandardFont::Courier => 8,
        StandardFont::CourierBold => 9,
        StandardFont::CourierOblique => 10,
        StandardFont::CourierBoldOblique => 11,
        StandardFont::Symbol | StandardFont::ZapfDingBats => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn font_file_cache_evicts_least_recent_without_invalidating_active_faces() {
        let mut cache = FontFiles::default();
        let a: FontData = Arc::new(vec![1_u8; 6]);
        let b: FontData = Arc::new(vec![2_u8; 6]);
        let c: FontData = Arc::new(vec![3_u8; 6]);
        cache.insert("a.ttc".into(), a.clone(), 12);
        cache.insert("b.ttc".into(), b.clone(), 12);
        let active = cache.get(std::path::Path::new("a.ttc")).unwrap();
        cache.insert("c.ttc".into(), c, 12);
        assert!(cache.get(std::path::Path::new("b.ttc")).is_none());
        assert!(cache.get(std::path::Path::new("a.ttc")).is_some());
        assert!(cache.get(std::path::Path::new("c.ttc")).is_some());
        assert_eq!(active.as_ref().as_ref(), &[1_u8; 6]);
        cache.insert("b.ttc".into(), b, 12);
        assert!(cache.get(std::path::Path::new("b.ttc")).is_some());
        assert_eq!(
            cache
                .entries
                .values()
                .map(|(data, _)| data.as_ref().as_ref().len())
                .sum::<usize>(),
            12
        );
    }
}
