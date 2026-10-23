//! Unicode text for PDF content streams, in any script.
//!
//! Every character is set in the preferred face when that face can draw it, and
//! otherwise in the first installed font that can: a list of script fonts first,
//! then every installed font. Lines are reordered for right-to-left text (bidi),
//! shaped with rustybuzz (ligatures, kerning, Arabic joining, Indic reordering,
//! mark positioning) and embedded as subset Type0 fonts. ToUnicode maps keep the
//! text selectable and searchable in other readers.
use anyhow::{Context, Result, anyhow, bail};
use lopdf::content::Operation;
use lopdf::{Document, Object, ObjectId, Stream, StringFormat, dictionary};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// The face a writer prefers before falling back to other installed fonts.
#[derive(Clone, Debug)]
pub enum Preferred {
    /// A specific font file, such as the face chosen for new text.
    File(PathBuf),
    /// The installed face that best matches a PDF BaseFont, e.g. "ABCDEF+Arial-BoldMT".
    LikePdfFont(String),
}

/// An installed face that can be embedded, with a per-character coverage cache.
struct Face {
    path: PathBuf,
    index: u32,
    family: String,
    data: Arc<Vec<u8>>,
    coverage: Mutex<HashMap<char, bool>>,
}
impl Face {
    fn parse(&self) -> Option<ttf_parser::Face<'_>> {
        ttf_parser::Face::parse(&self.data, self.index).ok()
    }
    fn covers(&self, c: char) -> bool {
        if let Some(known) = self.coverage.lock().unwrap().get(&c) {
            return *known;
        }
        let known = self.parse().is_some_and(|face| draws(&face, c));
        self.coverage.lock().unwrap().insert(c, known);
        known
    }
}

/// True when `face` has an outline (not just a bitmap) for `c`.
fn draws(face: &ttf_parser::Face<'_>, c: char) -> bool {
    struct Sink;
    impl ttf_parser::OutlineBuilder for Sink {
        fn move_to(&mut self, _: f32, _: f32) {}
        fn line_to(&mut self, _: f32, _: f32) {}
        fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
        fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
        fn close(&mut self) {}
    }
    face.glyph_index(c).is_some_and(|glyph| {
        c.is_whitespace() || ignorable(c) || face.outline_glyph(glyph, &mut Sink).is_some()
    })
}
/// Fonts whose licence forbids embedding, bitmap-only fonts, and the LastResort
/// placeholder font are never used.
fn embeddable(face: &ttf_parser::Face<'_>, family: &str) -> bool {
    let tables = face.tables();
    // Color bitmap fonts (such as Apple Color Emoji) only carry placeholder outlines.
    (tables.glyf.is_some() || tables.cff.is_some())
        && tables.sbix.is_none()
        && tables.cbdt.is_none()
        && face.permissions() != Some(ttf_parser::Permissions::Restricted)
        && face.is_subsetting_allowed()
        && face.is_outline_embedding_allowed()
        && !family
            .to_ascii_lowercase()
            .replace(' ', "")
            .contains("lastresort")
}

/// Joiners, variation selectors and other characters drawn by their neighbors.
fn ignorable(c: char) -> bool {
    matches!(c as u32, 0x200B..=0x200F | 0x2028..=0x202E | 0x2060..=0x206F | 0xFE00..=0xFE0F | 0xFEFF | 0xE0100..=0xE01EF)
}
/// Combining marks stay in the font of the character they attach to.
fn mark(c: char) -> bool {
    matches!(
        c as u32,
        0x0300..=0x036F
            | 0x0483..=0x0489
            | 0x0591..=0x05C7
            | 0x0610..=0x061A
            | 0x064B..=0x065F
            | 0x0670
            | 0x06D6..=0x06ED
            | 0x0900..=0x0903
            | 0x093A..=0x094F
            | 0x0951..=0x0957
            | 0x0962..=0x0963
            | 0x0E31
            | 0x0E34..=0x0E3A
            | 0x0E47..=0x0E4E
            | 0x1AB0..=0x1AFF
            | 0x1DC0..=0x1DFF
            | 0x20D0..=0x20FF
            | 0x302A..=0x302F
            | 0x3099..=0x309A
            | 0xFE20..=0xFE2F
    )
}
/// Scripts written without spaces may wrap between any two characters.
fn breaks_anywhere(c: char) -> bool {
    matches!(c as u32, 0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x3FFFF)
}

fn database() -> &'static fontdb::Database {
    static DB: OnceLock<fontdb::Database> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        db
    })
}
fn file_of(info: &fontdb::FaceInfo) -> Option<PathBuf> {
    match &info.source {
        fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => Some(path.clone()),
        fontdb::Source::Binary(_) => None,
    }
}
/// Load a face once per process; file data is shared between collection members.
fn load(path: &Path, index: u32) -> Option<Arc<Face>> {
    type Cache = Mutex<HashMap<(PathBuf, u32), Option<Arc<Face>>>>;
    static FACES: OnceLock<Cache> = OnceLock::new();
    static FILES: OnceLock<Mutex<HashMap<PathBuf, Arc<Vec<u8>>>>> = OnceLock::new();
    let key = (path.to_path_buf(), index);
    if let Some(face) = FACES
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .get(&key)
    {
        return face.clone();
    }
    let data = {
        let mut files = FILES.get_or_init(Default::default).lock().unwrap();
        match files.get(path) {
            Some(data) => Some(data.clone()),
            None => std::fs::read(path).ok().map(|bytes| {
                let data = Arc::new(bytes);
                files.insert(path.to_path_buf(), data.clone());
                data
            }),
        }
    };
    let face = data.and_then(|data| {
        let parsed = ttf_parser::Face::parse(&data, index).ok()?;
        let family = parsed
            .names()
            .into_iter()
            .filter(|n| n.name_id == ttf_parser::name_id::FAMILY && n.is_unicode())
            .find_map(|n| n.to_string())
            .unwrap_or_else(|| {
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into()
            });
        if !embeddable(&parsed, &family) {
            return None;
        }
        Some(Arc::new(Face {
            path: path.to_path_buf(),
            index,
            family,
            data: data.clone(),
            coverage: Mutex::new(HashMap::new()),
        }))
    });
    FACES
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .insert(key, face.clone());
    face
}

/// Families, weight and slant described by a PDF BaseFont name.
fn describe(base_font: &str) -> (Vec<String>, fontdb::Weight, fontdb::Style, u8) {
    let mut name = base_font;
    if name.len() > 7
        && name.as_bytes()[6] == b'+'
        && name[..6].bytes().all(|b| b.is_ascii_uppercase())
    {
        name = &name[7..];
    }
    let lower = name.to_ascii_lowercase();
    let weight = if lower.contains("black") || lower.contains("heavy") {
        fontdb::Weight::BLACK
    } else if lower.contains("semibold") || lower.contains("demi") {
        fontdb::Weight::SEMIBOLD
    } else if lower.contains("bold") {
        fontdb::Weight::BOLD
    } else if lower.contains("light") {
        fontdb::Weight::LIGHT
    } else {
        fontdb::Weight::NORMAL
    };
    let style = if lower.contains("italic") || lower.contains("oblique") {
        fontdb::Style::Italic
    } else {
        fontdb::Style::Normal
    };
    let family = name.split(['-', ',']).next().unwrap_or(name);
    let family = family
        .strip_suffix("PSMT")
        .or_else(|| family.strip_suffix("MT"))
        .or_else(|| family.strip_suffix("PS"))
        .unwrap_or(family);
    // "TimesNewRoman" → "Times New Roman".
    let mut spaced = String::new();
    let mut previous_lower = false;
    for c in family.chars() {
        if c.is_ascii_uppercase() && previous_lower {
            spaced.push(' ');
        }
        previous_lower = c.is_ascii_lowercase();
        spaced.push(c);
    }
    let mut families = vec![family.to_owned(), spaced];
    // 0: sans-serif, 1: serif, 2: monospace.
    let generic = if [
        "times", "serif", "georgia", "garamond", "roman", "palatino", "cambria", "book",
    ]
    .iter()
    .any(|w| lower.contains(w) && !lower.contains("sans"))
    {
        1
    } else if ["courier", "mono", "consol", "menlo"]
        .iter()
        .any(|w| lower.contains(w))
    {
        2
    } else {
        0
    };
    let aliases: &[&str] = if lower.contains("helvetica") {
        &["Helvetica", "Arial"]
    } else if lower.contains("arial") {
        &["Arial", "Helvetica"]
    } else if lower.contains("times") {
        &["Times New Roman", "Times"]
    } else if lower.contains("courier") {
        &["Courier New", "Courier"]
    } else {
        &[]
    };
    families.extend(aliases.iter().map(|a| a.to_string()));
    (families, weight, style, generic)
}

/// Script fonts with good OpenType shaping, then broad-coverage fonts.
const FALLBACKS: &[&str] = &[
    "Geeza Pro",
    "SF Arabic",
    "Arial Hebrew",
    "SF Hebrew",
    "Kohinoor Devanagari",
    "Kohinoor Bangla",
    "Kohinoor Gujarati",
    "Kohinoor Telugu",
    "Tamil Sangam MN",
    "Malayalam Sangam MN",
    "Kannada Sangam MN",
    "Gurmukhi MN",
    "Oriya Sangam MN",
    "Sinhala Sangam MN",
    "Thonburi",
    "Lao Sangam MN",
    "Khmer Sangam MN",
    "Myanmar Sangam MN",
    "Noto Sans Myanmar",
    "Noto Sans Armenian",
    "SF Armenian",
    "SF Georgian",
    "Hiragino Sans",
    "Hiragino Kaku Gothic ProN",
    "Hiragino Sans GB",
    "PingFang SC",
    "Apple SD Gothic Neo",
    "Heiti SC",
    "STHeiti",
    "Kailasa",
    "Euphemia UCAS",
    "Segoe UI",
    "Nirmala UI",
    "Leelawadee UI",
    "Malgun Gothic",
    "Microsoft YaHei",
    "Yu Gothic",
    "Microsoft JhengHei",
    "Ebrima",
    "Segoe UI Historic",
    "Segoe UI Symbol",
    "Noto Sans",
    "Noto Sans Arabic",
    "Noto Sans Hebrew",
    "Noto Sans Devanagari",
    "Noto Sans Thai",
    "Noto Sans CJK SC",
    "DejaVu Sans",
    "Arial Unicode MS",
    "Apple Symbols",
    "STIXGeneral",
    "Lucida Grande",
    "Arial",
];

/// The best installed face for a list of family names.
fn query(
    families: &[&str],
    weight: fontdb::Weight,
    style: fontdb::Style,
    generic: Option<u8>,
) -> Option<Arc<Face>> {
    let mut list: Vec<fontdb::Family<'_>> =
        families.iter().map(|f| fontdb::Family::Name(f)).collect();
    if let Some(generic) = generic {
        list.push(match generic {
            1 => fontdb::Family::Serif,
            2 => fontdb::Family::Monospace,
            _ => fontdb::Family::SansSerif,
        });
    }
    let db = database();
    let id = db.query(&fontdb::Query {
        families: &list,
        weight,
        style,
        ..Default::default()
    })?;
    let info = db.face(id)?;
    load(&file_of(info)?, info.index)
}

/// The first installed font after the preferred chain that can draw `c`.
fn fallback(c: char) -> Option<Arc<Face>> {
    type Cache = Mutex<HashMap<char, Option<(PathBuf, u32)>>>;
    static FOUND: OnceLock<Cache> = OnceLock::new();
    if let Some(found) = FOUND.get_or_init(Default::default).lock().unwrap().get(&c) {
        return found.as_ref().and_then(|(path, index)| load(path, *index));
    }
    let found = FALLBACKS
        .iter()
        .filter_map(|family| {
            query(
                &[family],
                fontdb::Weight::NORMAL,
                fontdb::Style::Normal,
                None,
            )
        })
        .find(|face| face.covers(c))
        .or_else(|| {
            // Rare characters: scan every installed face without loading it fully.
            let db = database();
            db.faces()
                .filter(|info| info.style == fontdb::Style::Normal)
                .chain(
                    db.faces()
                        .filter(|info| info.style != fontdb::Style::Normal),
                )
                .find_map(|info| {
                    let family = info.families.first().map_or("", |f| f.0.as_str());
                    let fits = db
                        .with_face_data(info.id, |data, index| {
                            ttf_parser::Face::parse(data, index)
                                .is_ok_and(|face| embeddable(&face, family) && draws(&face, c))
                        })
                        .unwrap_or(false);
                    fits.then(|| load(&file_of(info)?, info.index)).flatten()
                })
        });
    FOUND
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .insert(c, found.as_ref().map(|f| (f.path.clone(), f.index)));
    found
}

#[derive(Clone, Debug)]
struct Glyph {
    gid: u16,
    /// Shaped advance and offsets, in thousandths of the font size.
    advance: f32,
    x_offset: f32,
    y_offset: f32,
    space: bool,
}
#[derive(Clone, Debug)]
struct Piece {
    font: usize,
    glyphs: Vec<Glyph>,
}
/// One shaped line in visual order, ready for [`Writer::operations`].
#[derive(Clone, Debug, Default)]
pub struct Line {
    pieces: Vec<Piece>,
    advance: f32,
    /// The logical text, for ActualText on complex scripts.
    text: String,
}
impl Line {
    /// Width in points at `size`.
    pub fn width(&self, size: f32) -> f32 {
        self.advance * size / 1000.
    }
}

struct Used {
    face: Arc<Face>,
    /// Original glyph IDs and the text each one stands for.
    glyphs: BTreeMap<u16, Option<String>>,
    cids: HashMap<u16, u16>,
    widths: HashMap<u16, f32>,
}

/// Shapes lines of text, then embeds exactly the glyphs it used.
pub struct Writer {
    chain: Vec<Arc<Face>>,
    used: Vec<Used>,
}
impl Writer {
    pub fn new(preferred: &Preferred) -> Result<Self> {
        let primary = match preferred {
            Preferred::File(path) => load(path, 0),
            Preferred::LikePdfFont(base) => {
                let (families, weight, style, generic) = describe(base);
                let names: Vec<&str> = families.iter().map(String::as_str).collect();
                query(&names, weight, style, Some(generic))
            }
        };
        let chain = primary
            .or_else(|| {
                query(
                    &["Arial", "Helvetica"],
                    fontdb::Weight::NORMAL,
                    fontdb::Style::Normal,
                    Some(0),
                )
            })
            .into_iter()
            .collect();
        Ok(Self {
            chain,
            used: vec![],
        })
    }
    /// Family names of the fonts that received glyphs.
    pub fn families(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .used
            .iter()
            .filter(|u| !u.glyphs.is_empty())
            .map(|u| u.face.family.clone())
            .collect();
        out.dedup();
        out
    }
    fn use_face(&mut self, face: Arc<Face>) -> usize {
        if let Some(i) = self.used.iter().position(|u| Arc::ptr_eq(&u.face, &face)) {
            return i;
        }
        self.used.push(Used {
            face,
            glyphs: BTreeMap::new(),
            cids: HashMap::new(),
            widths: HashMap::new(),
        });
        self.used.len() - 1
    }
    fn font_for(&mut self, c: char) -> Result<usize> {
        if let Some(face) = self.chain.iter().find(|f| f.covers(c)).cloned() {
            return Ok(self.use_face(face));
        }
        let face = fallback(c).with_context(|| {
            if matches!(c as u32, 0x1F000..=0x1FAFF | 0x2600..=0x27BF) {
                format!("Color emoji such as “{c}” cannot be written as PDF text.")
            } else {
                format!("No installed font can show “{c}” (U+{:04X}).", c as u32)
            }
        })?;
        self.chain.push(face.clone());
        Ok(self.use_face(face))
    }
    /// Split logical text into runs that each use one font.
    fn itemize(&mut self, text: &str) -> Result<Vec<(usize, std::ops::Range<usize>)>> {
        let mut out: Vec<(usize, std::ops::Range<usize>)> = vec![];
        for (i, c) in text.char_indices() {
            let end = i + c.len_utf8();
            let inherit = c.is_whitespace() || ignorable(c) || mark(c);
            let font = match out.last() {
                Some((f, _)) if inherit && (ignorable(c) || self.used[*f].face.covers(c)) => *f,
                _ if ignorable(c) && !self.chain.is_empty() => {
                    let face = self.chain[0].clone();
                    self.use_face(face)
                }
                _ => self.font_for(c)?,
            };
            match out.last_mut() {
                Some((f, range)) if *f == font => range.end = end,
                _ => out.push((font, i..end)),
            }
        }
        Ok(out)
    }
    fn shape_run(&mut self, font: usize, text: &str, rtl: bool, record: bool) -> Result<Piece> {
        let face = self.used[font].face.clone();
        let shaper = rustybuzz::Face::from_slice(&face.data, face.index)
            .with_context(|| format!("Could not read {}", face.family))?;
        let upem = shaper.units_per_em() as f32;
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        buffer.set_direction(if rtl {
            rustybuzz::Direction::RightToLeft
        } else {
            rustybuzz::Direction::LeftToRight
        });
        let shaped = rustybuzz::shape(&shaper, &[], buffer);
        let mut starts: Vec<usize> = shaped
            .glyph_infos()
            .iter()
            .map(|g| g.cluster as usize)
            .collect();
        starts.sort_unstable();
        starts.dedup();
        let cluster_text = |start: usize| {
            let end = starts
                .iter()
                .copied()
                .find(|s| *s > start)
                .unwrap_or(text.len());
            &text[start..end]
        };
        let mut labelled = HashSet::new();
        let mut glyphs = vec![];
        for (info, position) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
            let gid = info.glyph_id as u16;
            let source = cluster_text(info.cluster as usize);
            if gid == 0 {
                if source.chars().all(|c| ignorable(c) || c.is_whitespace()) {
                    continue;
                }
                bail!("{} cannot show “{source}”.", face.family);
            }
            let scale = |v: i32| v as f32 * 1000. / upem;
            glyphs.push(Glyph {
                gid,
                advance: scale(position.x_advance),
                x_offset: scale(position.x_offset),
                y_offset: scale(position.y_offset),
                space: source == " ",
            });
            if record {
                // The first glyph of each cluster carries its text for copy and search.
                let label = labelled.insert(info.cluster).then(|| source.to_owned());
                let entry = self.used[font].glyphs.entry(gid).or_insert(None);
                if entry.is_none() {
                    *entry = label;
                }
            }
        }
        Ok(Piece { font, glyphs })
    }
    fn shape_inner(&mut self, text: &str, record: bool) -> Result<Line> {
        let mut line = Line {
            text: text.to_owned(),
            ..Default::default()
        };
        if text.is_empty() {
            return Ok(line);
        }
        let bidi = unicode_bidi::BidiInfo::new(text, None);
        for paragraph in &bidi.paragraphs {
            let (levels, runs) = bidi.visual_runs(paragraph, paragraph.range.clone());
            for run in runs {
                let rtl = levels[run.start].is_rtl();
                let mut items = self.itemize(&text[run.clone()])?;
                if rtl {
                    items.reverse();
                }
                for (font, range) in items {
                    let piece = self.shape_run(
                        font,
                        &text[run.start + range.start..run.start + range.end],
                        rtl,
                        record,
                    )?;
                    line.advance += piece.glyphs.iter().map(|g| g.advance).sum::<f32>();
                    line.pieces.push(piece);
                }
            }
        }
        Ok(line)
    }
    /// Shape one line and record its glyphs for embedding.
    pub fn shape(&mut self, text: &str) -> Result<Line> {
        self.shape_inner(text, true)
    }
    /// Width of `text` in points without recording glyphs.
    pub fn measure(&mut self, text: &str, size: f32) -> Result<f32> {
        Ok(self.shape_inner(text, false)?.width(size))
    }
    /// Actual ascent, positive descent and line gap of the faces used by a
    /// shaped line, in points. Layout and auto-sizing can share PDF metrics even
    /// when a line uses fallback fonts.
    pub fn line_metrics(&self, line: &Line, size: f32) -> [f32; 3] {
        let mut metrics = [0_f32; 3];
        let mut found = false;
        let faces: Vec<_> = if line.pieces.is_empty() {
            self.chain.first().into_iter().collect()
        } else {
            line.pieces
                .iter()
                .filter_map(|piece| self.used.get(piece.font).map(|used| &used.face))
                .collect()
        };
        for face in faces {
            if let Some(parsed) = face.parse() {
                let scale = size / parsed.units_per_em() as f32;
                metrics[0] = metrics[0].max(parsed.ascender() as f32 * scale);
                metrics[1] = metrics[1].max(-(parsed.descender() as f32) * scale);
                metrics[2] = metrics[2].max(parsed.line_gap() as f32 * scale);
                found = true;
            }
        }
        if found {
            metrics
        } else {
            [size * 0.8, size * 0.2, size * 0.1]
        }
    }
    /// Greedy line breaking at spaces, and between characters of scripts written
    /// without spaces. Returns each line and whether it ends a paragraph.
    pub fn wrap(&mut self, text: &str, size: f32, width: f32) -> Result<Vec<(String, bool)>> {
        let mut lines = vec![];
        for paragraph in text.split('\n') {
            let mut tokens: Vec<String> = vec![];
            let mut token = String::new();
            for c in paragraph.chars() {
                if breaks_anywhere(c) && !token.trim().is_empty() {
                    tokens.push(std::mem::take(&mut token));
                }
                token.push(c);
                if c == ' ' || breaks_anywhere(c) {
                    tokens.push(std::mem::take(&mut token));
                }
            }
            if !token.is_empty() {
                tokens.push(token);
            }
            let mut line = String::new();
            for token in tokens {
                let candidate = format!("{line}{token}");
                if line.trim().is_empty() || self.measure(candidate.trim_end(), size)? <= width {
                    line = candidate;
                    continue;
                }
                lines.push((line.trim_end().to_owned(), false));
                line = token;
            }
            // A single word wider than the box breaks between characters.
            let mut rest = line;
            while self.measure(rest.trim_end(), size)? > width && rest.chars().count() > 1 {
                let mut head = String::new();
                for c in rest.chars() {
                    let next = format!("{head}{c}");
                    if !head.is_empty() && !mark(c) && self.measure(&next, size)? > width {
                        break;
                    }
                    head = next;
                }
                rest = rest[head.len()..].to_owned();
                lines.push((head, false));
            }
            lines.push((rest, true));
        }
        Ok(lines)
    }

    /// Subset and embed every font that received glyphs. Returns a Type0 font
    /// object per font index, `None` for fonts that ended up unused.
    pub fn embed(&mut self, doc: &mut Document) -> Result<Vec<Option<ObjectId>>> {
        let mut ids = vec![];
        for used in &mut self.used {
            ids.push(if used.glyphs.is_empty() {
                None
            } else {
                Some(embed_font(doc, used)?)
            });
        }
        Ok(ids)
    }

    /// Content operations that show `line` from the current text position.
    /// `names` are the page resource names of the embedded fonts, `word_gap`
    /// widens spaces for justified text (points), and `rise` is the current text
    /// rise, which is restored afterwards.
    pub fn operations(
        &self,
        line: &Line,
        names: &[Option<String>],
        size: f32,
        word_gap: f32,
        rise: f32,
    ) -> Result<Vec<Operation>> {
        let mut ops = vec![];
        // Reordered or shaped text: readers copy and search the logical text.
        let actual = complex(&line.text);
        if actual {
            ops.push(Operation::new(
                "BDC",
                vec![
                    Object::Name(b"Span".to_vec()),
                    Object::Dictionary(dictionary! {
                        "ActualText" => crate::annotations::text_string(&line.text),
                    }),
                ],
            ));
        }
        let mut current_font = None;
        let mut current_rise = rise;
        for piece in line.pieces.iter().filter(|p| !p.glyphs.is_empty()) {
            let used = &self.used[piece.font];
            if current_font != Some(piece.font) {
                let name = names
                    .get(piece.font)
                    .and_then(Option::as_ref)
                    .context("Font was not embedded")?;
                ops.push(Operation::new(
                    "Tf",
                    vec![Object::Name(name.as_bytes().to_vec()), size.into()],
                ));
                current_font = Some(piece.font);
            }
            let mut array: Vec<Object> = vec![];
            let mut glyphs: Vec<u8> = vec![];
            let flush_glyphs = |array: &mut Vec<Object>, glyphs: &mut Vec<u8>| {
                if !glyphs.is_empty() {
                    array.push(Object::String(
                        std::mem::take(glyphs),
                        StringFormat::Hexadecimal,
                    ));
                }
            };
            for g in &piece.glyphs {
                let target = rise + g.y_offset * size / 1000.;
                if (target - current_rise).abs() > 0.001 {
                    flush_glyphs(&mut array, &mut glyphs);
                    if !array.is_empty() {
                        ops.push(Operation::new(
                            "TJ",
                            vec![Object::Array(std::mem::take(&mut array))],
                        ));
                    }
                    ops.push(Operation::new("Ts", vec![Object::Real(target)]));
                    current_rise = target;
                }
                if g.x_offset.abs() > 0.01 {
                    flush_glyphs(&mut array, &mut glyphs);
                    array.push(Object::Real(-g.x_offset));
                }
                let cid = used.cids.get(&g.gid).context("Glyph was not embedded")?;
                glyphs.extend(cid.to_be_bytes());
                // TJ numbers move left by thousandths of the size: fix the shaped
                // advance against the font's default width, and undo the offset.
                let mut adjust =
                    used.widths.get(&g.gid).copied().unwrap_or(0.) + g.x_offset - g.advance;
                if g.space && word_gap > 0. {
                    adjust -= word_gap * 1000. / size;
                }
                if adjust.abs() > 0.01 {
                    flush_glyphs(&mut array, &mut glyphs);
                    array.push(Object::Real(adjust));
                }
            }
            flush_glyphs(&mut array, &mut glyphs);
            if !array.is_empty() {
                ops.push(Operation::new("TJ", vec![Object::Array(array)]));
            }
        }
        if (current_rise - rise).abs() > 0.001 {
            ops.push(Operation::new("Ts", vec![Object::Real(rise)]));
        }
        if actual {
            ops.push(Operation::new("EMC", vec![]));
        }
        Ok(ops)
    }
}

/// A Type0 font with a subset TrueType or CFF program, widths and ToUnicode.
fn embed_font(doc: &mut Document, used: &mut Used) -> Result<ObjectId> {
    let face = used.face.parse().context("Font is unreadable")?;
    let mut remapper = subsetter::GlyphRemapper::new();
    for gid in used.glyphs.keys() {
        remapper.remap(*gid);
    }
    let program = subsetter::subset(&used.face.data, used.face.index, &remapper)
        .map_err(|e| anyhow!("Could not subset {}: {e:?}", used.face.family))?;
    let upem = face.units_per_em() as f32;
    let scale = |v: f32| v * 1000. / upem;
    let mut widths = vec![];
    let mut unicode = vec![];
    for (gid, text) in &used.glyphs {
        let cid = remapper.get(*gid).context("Glyph missing from subset")?;
        let width = scale(
            face.glyph_hor_advance(ttf_parser::GlyphId(*gid))
                .unwrap_or(0) as f32,
        );
        used.cids.insert(*gid, cid);
        used.widths.insert(*gid, width);
        widths.push(Object::Integer(cid as i64));
        widths.push(Object::Array(vec![Object::Real(width)]));
        if let Some(text) = text.as_ref().filter(|t| !t.is_empty()) {
            let utf16: String = text.encode_utf16().map(|u| format!("{u:04X}")).collect();
            unicode.push(format!("<{cid:04X}> <{utf16}>"));
        }
    }
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /Adobe-Identity-UCS def /CMapType 2 def 1 begincodespacerange <0000> <FFFF> endcodespacerange\n",
    );
    for chunk in unicode.chunks(100) {
        cmap.push_str(&format!(
            "{} beginbfchar\n{}\nendbfchar\n",
            chunk.len(),
            chunk.join("\n")
        ));
    }
    cmap.push_str("endcmap CMapName currentdict /CMap defineresource pop end end");

    // A subset tag derived from the glyph set keeps output reproducible.
    let mut hash = 0x811c9dc5_u32;
    for gid in used.glyphs.keys() {
        hash = (hash ^ *gid as u32).wrapping_mul(0x01000193);
    }
    let tag: String = (0..6)
        .map(|i| (b'A' + ((hash >> (i * 5)) % 26) as u8) as char)
        .collect();
    let postscript = face
        .names()
        .into_iter()
        .find(|n| n.name_id == ttf_parser::name_id::POST_SCRIPT_NAME)
        .and_then(|n| n.to_string())
        .unwrap_or_else(|| used.face.family.replace(' ', ""));
    let name = format!("{tag}+{postscript}");
    let bbox = face.global_bounding_box();
    let truetype = face.tables().glyf.is_some();
    let length = program.len() as i64;
    let mut file = Stream::new(
        if truetype {
            dictionary! { "Length1" => length }
        } else {
            dictionary! { "Subtype" => "OpenType" }
        },
        program,
    );
    let _ = file.compress();
    let file = doc.add_object(file);
    let flags =
        4 | if face.is_italic() { 64 } else { 0 } | if face.is_monospaced() { 1 } else { 0 };
    let descriptor = doc.add_object(dictionary! {
        "Type" => "FontDescriptor",
        "FontName" => Object::Name(name.as_bytes().to_vec()),
        "Flags" => flags,
        "FontBBox" => vec![
            scale(bbox.x_min as f32).into(),
            scale(bbox.y_min as f32).into(),
            scale(bbox.x_max as f32).into(),
            scale(bbox.y_max as f32).into(),
        ],
        "ItalicAngle" => face.italic_angle(),
        "Ascent" => scale(face.ascender() as f32),
        "Descent" => scale(face.descender() as f32),
        "CapHeight" => scale(face.capital_height().unwrap_or(face.ascender()) as f32),
        "StemV" => if face.is_bold() { 120 } else { 80 },
        if truetype { "FontFile2" } else { "FontFile3" } => file,
    });
    let mut descendant = dictionary! {
        "Type" => "Font",
        "Subtype" => if truetype { "CIDFontType2" } else { "CIDFontType0" },
        "BaseFont" => Object::Name(name.as_bytes().to_vec()),
        "CIDSystemInfo" => dictionary! {
            "Registry" => Object::string_literal("Adobe"),
            "Ordering" => Object::string_literal("Identity"),
            "Supplement" => 0,
        },
        "FontDescriptor" => descriptor,
        "DW" => 1000,
        "W" => Object::Array(widths),
    };
    if truetype {
        descendant.set("CIDToGIDMap", "Identity");
    }
    let descendant = doc.add_object(descendant);
    let mut to_unicode = Stream::new(dictionary! {}, cmap.into_bytes());
    let _ = to_unicode.compress();
    let to_unicode = doc.add_object(to_unicode);
    Ok(doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type0",
        "BaseFont" => Object::Name(name.into_bytes()),
        "Encoding" => "Identity-H",
        "DescendantFonts" => vec![Object::Reference(descendant)],
        "ToUnicode" => to_unicode,
    }))
}

/// Characters of `text` that the font program embedded for `font` cannot draw.
/// Encodable is not enough: embedded subsets usually contain only the glyphs a
/// document already used. `seen` holds characters already shown with this font.
pub fn missing_glyphs(
    doc: &Document,
    font: &lopdf::Dictionary,
    base_font: &str,
    seen: &HashSet<char>,
    text: &str,
) -> Vec<char> {
    let needed: Vec<char> = text
        .chars()
        .filter(|c| !c.is_whitespace() && !seen.contains(c))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    if needed.is_empty() {
        return needed;
    }
    let descriptor = |dict: &lopdf::Dictionary| -> Option<lopdf::Dictionary> {
        let d = dict.get(b"FontDescriptor").ok()?;
        doc.dereference(d).ok()?.1.as_dict().ok().cloned()
    };
    let descriptor = descriptor(font).or_else(|| {
        let first = font
            .get(b"DescendantFonts")
            .ok()?
            .as_array()
            .ok()?
            .first()?;
        let descendant = doc.dereference(first).ok()?.1.as_dict().ok()?.clone();
        descriptor(&descendant)
    });
    let program = descriptor.as_ref().and_then(|d| {
        ["FontFile2", "FontFile3", "FontFile"]
            .iter()
            .find_map(|key| {
                let stream = doc
                    .dereference(d.get(key.as_bytes()).ok()?)
                    .ok()?
                    .1
                    .as_stream()
                    .ok()?;
                Some(
                    stream
                        .decompressed_content()
                        .unwrap_or_else(|_| stream.content.clone()),
                )
            })
    });
    let Some(program) = program else {
        // Not embedded: the reader supplies a complete installed font.
        return vec![];
    };
    let subset = base_font.len() > 7
        && base_font.as_bytes()[6] == b'+'
        && base_font[..6].bytes().all(|b| b.is_ascii_uppercase());
    match ttf_parser::Face::parse(&program, 0) {
        Ok(face) => needed.into_iter().filter(|c| !draws(&face, *c)).collect(),
        // Programs we cannot inspect: trust complete fonts, never subsets.
        Err(_) if subset => needed,
        Err(_) => vec![],
    }
}

/// Whether `text` contains right-to-left characters.
pub fn has_rtl(text: &str) -> bool {
    unicode_bidi::BidiInfo::new(text, None).has_rtl()
}

/// Text without joiners and other invisible format characters, as it extracts.
pub fn visible(text: &str) -> String {
    text.chars().filter(|c| !ignorable(*c)).collect()
}
/// Right-to-left text, combining marks, or scripts that shaping reorders.
pub fn complex(text: &str) -> bool {
    has_rtl(text)
        || text.chars().any(|c| {
            mark(c)
                || ignorable(c)
                || matches!(c as u32, 0x0900..=0x0DFF | 0x0E00..=0x0FFF | 0x1000..=0x109F | 0x1780..=0x17FF | 0xA8E0..=0xA8FF)
        })
}
