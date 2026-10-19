//! Local, portable signature lettering. Fonts become SVG paths, not external links.
use anyhow::{Context, Result, ensure};
use std::{
    fmt::Write,
    path::{Path, PathBuf},
};

fn shape<'a>(face: &rustybuzz::Face<'a>, text: &str) -> rustybuzz::GlyphBuffer {
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();
    rustybuzz::shape(face, &[], buffer)
}
struct Outline(String);
impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.0, "M{x} {y}");
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.0, "L{x} {y}");
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let _ = write!(self.0, "Q{x1} {y1} {x} {y}");
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let _ = write!(self.0, "C{x1} {y1} {x2} {y2} {x} {y}");
    }
    fn close(&mut self) {
        self.0.push('Z');
    }
}

/// Shape, wrap and outline Unicode text. Alignment: 0 left, 1 center, 2 right.
pub fn lettering(
    text: &str,
    font: &Path,
    size: f32,
    color: [f32; 3],
    width: f32,
    align: u8,
) -> Result<(String, f32)> {
    ensure!(
        text.len() <= 4096 && size.is_finite() && size >= 1. && width.is_finite() && width > 0.,
        "Invalid signature lettering"
    );
    let bytes = std::fs::read(font)?;
    let face = rustybuzz::Face::from_slice(&bytes, 0).context("Cannot read signature font")?;
    let scale = size / face.units_per_em() as f32;
    let measure = |s: &str| {
        shape(&face, s)
            .glyph_positions()
            .iter()
            .map(|g| g.x_advance as f32 * scale)
            .sum::<f32>()
    };
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && measure(&candidate) > width {
                lines.push(line);
                line = word.into();
            } else {
                line = candidate;
            }
        }
        lines.push(line);
    }
    let line_height = ((face.ascender() as f32 - face.descender() as f32) * scale).max(size * 1.25);
    let height = lines.len() as f32 * line_height + 2.;
    let mut paths = String::new();
    for (row, line) in lines.iter().enumerate() {
        let glyphs = shape(&face, line);
        ensure!(
            glyphs.glyph_infos().iter().all(|g| g.glyph_id != 0),
            "This font does not contain every character in your signature"
        );
        let line_width = measure(line);
        ensure!(
            line_width <= width + 0.1,
            "Signature text is too wide for this font size"
        );
        let mut x = match align {
            1 => (width - line_width) / 2.,
            2 => width - line_width,
            _ => 0.,
        };
        let baseline = 1. + row as f32 * line_height + face.ascender() as f32 * scale;
        for (info, pos) in glyphs.glyph_infos().iter().zip(glyphs.glyph_positions()) {
            let mut outline = Outline(String::new());
            face.outline_glyph(ttf_parser::GlyphId(info.glyph_id as u16), &mut outline);
            if !outline.0.is_empty() {
                write!(
                    paths,
                    "<path transform=\"translate({} {}) scale({scale} {})\" d=\"{}\"/>",
                    x + pos.x_offset as f32 * scale,
                    baseline - pos.y_offset as f32 * scale,
                    -scale,
                    outline.0
                )?;
            }
            x += pos.x_advance as f32 * scale;
        }
    }
    let color = format!(
        "#{:02x}{:02x}{:02x}",
        (color[0] * 255.) as u8,
        (color[1] * 255.) as u8,
        (color[2] * 255.) as u8
    );
    Ok((
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {width} {height}\"><g fill=\"{color}\">{paths}</g></svg>"
        ),
        height,
    ))
}

/// Keep these indices stable: saved editor choices use them.
pub const NAME_STYLES: [&str; 6] = [
    "Script", "Elegant", "Initials", "Flowing", "Personal", "Swift",
];

pub fn name_font(style: usize) -> Result<PathBuf> {
    let (mac, windows): (&[&str], &[&str]) = match style {
        0 => (
            &["Apple Chancery.ttf", "SnellRoundhand.ttc"],
            &["segoesc.ttf"],
        ),
        3 => (
            &["SnellRoundhand.ttc", "Savoye LET.ttc", "Apple Chancery.ttf"],
            &["edwardian.ttf", "itcedscr.ttf", "segoesc.ttf"],
        ),
        4 => (
            &["Savoye LET.ttc", "SnellRoundhand.ttc", "Apple Chancery.ttf"],
            &["freescpt.ttf", "segoesc.ttf"],
        ),
        5 => (
            &["SignPainter.ttc", "Brush Script.ttf", "SnellRoundhand.ttc"],
            &["brushsci.ttf", "segoescb.ttf", "segoesc.ttf"],
        ),
        _ => (&[], &[]),
    };
    let windows_root =
        PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into()))
            .join("Fonts");
    for path in mac
        .iter()
        .map(|name| Path::new("/System/Library/Fonts/Supplemental").join(name))
        .chain(windows.iter().map(|name| windows_root.join(name)))
    {
        if path.is_file() {
            return Ok(path);
        }
    }
    crate::text_style::fonts()
        .iter()
        .find(|f| f.family == "Times New Roman" && f.italic == (style != 2) && !f.bold)
        .or_else(|| crate::text_style::fonts().iter().find(|f| f.italic))
        .map(|f| f.path.clone())
        .map(Ok)
        .unwrap_or_else(crate::document::default_font)
}

pub fn from_name(
    name: &str,
    style: usize,
    flourish: bool,
    color: [f32; 3],
) -> Result<crate::assets::Asset> {
    ensure!(
        color
            .iter()
            .all(|v| v.is_finite() && (0. ..=1.).contains(v)),
        "Invalid ink color"
    );
    ensure!(
        !name.trim().is_empty() && name.chars().count() <= 200,
        "Enter your name first"
    );
    ensure!(style < NAME_STYLES.len(), "Unknown signature style");
    let text = if style == 2 {
        name.split_whitespace()
            .filter_map(|s| s.chars().next())
            .collect::<String>()
    } else {
        name.trim().into()
    };
    let font = name_font(style)?;
    let bytes = std::fs::read(&font)?;
    let face = rustybuzz::Face::from_slice(&bytes, 0).context("Cannot read signature font")?;
    let width = (shape(&face, &text)
        .glyph_positions()
        .iter()
        .map(|g| g.x_advance as f32)
        .sum::<f32>()
        * 48.
        / face.units_per_em() as f32
        + 16.)
        .max(60.);
    let (mut svg, h) = lettering(&text, &font, 48., color, width, 1)?;
    // Transform the whole word, preserving the font's connected strokes and kerning.
    // Extra space keeps the rising baseline and finishing strokes inside the artwork.
    let (width, h) = if style >= 3 {
        let (squeeze, rise) = match style {
            3 => (1., -0.025),
            4 => (0.92, -0.012),
            _ => (0.88, -0.045),
        };
        let padding = 14.;
        let lift = width * -rise;
        let inner = svg
            .split_once('>')
            .unwrap()
            .1
            .strip_suffix("</svg>")
            .unwrap();
        let w = width * squeeze + padding * 2.;
        let height = h + lift + padding * 2.;
        svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {height}\"><g transform=\"matrix({squeeze} {rise} 0 1 {padding} {})\">{inner}</g></svg>",
            padding + lift
        );
        (w, height)
    } else {
        (width, h)
    };
    if flourish {
        let color = format!(
            "#{:02x}{:02x}{:02x}",
            (color[0] * 255.) as u8,
            (color[1] * 255.) as u8,
            (color[2] * 255.) as u8
        );
        let stroke = match style {
            // Different pen-like endings: a long loop, a short return, a quick rising sweep.
            3 => format!(
                "M{} {} C{} {} {} {} {} {} S{} {} {} {}",
                width * 0.1,
                h - 18.,
                width * 0.3,
                h - 8.,
                width * 0.9,
                h - 32.,
                width * 0.92,
                h - 20.,
                width * 0.38,
                h - 3.,
                width * 0.22,
                h - 12.
            ),
            4 => format!(
                "M{} {} C{} {} {} {} {} {} Q{} {} {} {}",
                width * 0.3,
                h - 17.,
                width * 0.1,
                h - 8.,
                width * 0.7,
                h - 12.,
                width * 0.83,
                h - 24.,
                width * 0.9,
                h - 30.,
                width * 0.86,
                h - 15.
            ),
            5 => format!(
                "M{} {} Q{} {} {} {} M{} {} Q{} {} {} {}",
                width * 0.12,
                h - 14.,
                width * 0.55,
                h - 19.,
                width * 0.94,
                h - 30.,
                width * 0.58,
                h - 13.,
                width * 0.77,
                h - 18.,
                width * 0.9,
                h - 23.
            ),
            _ => format!(
                "M8 {} Q{} {} {} {} T{} {}",
                h - 7.,
                width * 0.25,
                h - 1.,
                width * 0.55,
                h - 7.,
                width - 8.,
                h - 9.
            ),
        };
        svg = svg.replace("</svg>", &format!("<path d=\"{stroke}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"1.4\" stroke-linecap=\"round\"/></svg>"));
    }
    crate::svg::tree(&svg, None)?;
    let mut asset = crate::assets::Asset::new(
        name,
        crate::annotations::Kind::Svg {
            source: svg,
            tint: false,
        },
        crate::annotations::Style::default(),
        width / h,
    );
    asset.signature = true;
    asset.validate()?;
    Ok(asset)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_names_are_local_portable_paths_and_styles_differ() {
        let mut sources = std::collections::HashSet::new();
        for style in 0..NAME_STYLES.len() {
            let asset = from_name("Orhan Gökçöl", style, true, [0.1, 0.2, 0.6]).unwrap();
            asset.validate().unwrap();
            assert!(asset.signature);
            let crate::annotations::Kind::Svg { source, .. } = asset.kind else {
                panic!("vector signature")
            };
            assert!(source.contains("<path"));
            assert!(!source.contains("<text") && !source.contains("href="));
            let png = crate::svg::preview(&source, None).unwrap();
            assert!(!png.is_empty());
            sources.insert(source);
        }
        assert_eq!(sources.len(), NAME_STYLES.len());
        assert!(from_name("", 0, false, [0.; 3]).is_err());
    }
    #[test]
    fn handwriting_keeps_names_and_diacritics_with_optional_finishing_strokes() {
        for name in ["İpek Şahin", "Jörg Müller", "A", "Ayşegül Orhan Gökçöl"] {
            for style in 3..6 {
                let plain = from_name(name, style, false, [0., 0., 0.]).unwrap();
                let repeat = from_name(name, style, false, [0., 0., 0.]).unwrap();
                let finished = from_name(name, style, true, [0., 0., 0.]).unwrap();
                let source = |asset: crate::assets::Asset| match asset.kind {
                    crate::annotations::Kind::Svg { source, .. } => source,
                    _ => unreachable!(),
                };
                let plain = source(plain);
                assert_eq!(plain, source(repeat));
                assert_ne!(plain, source(finished));
                assert!(!crate::svg::preview(&plain, None).unwrap().is_empty());
            }
        }
        assert!(from_name("Name", 6, true, [0.; 3]).is_err());
    }
}
