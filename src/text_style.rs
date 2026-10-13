//! Installed TrueType faces shared by the on-page editor and PDF writer.
use std::{path::PathBuf, sync::OnceLock};
#[derive(Clone, Debug)]
pub struct FontFace {
    pub family: String,
    pub bold: bool,
    pub italic: bool,
    pub path: PathBuf,
}
impl FontFace {
    pub fn style(&self) -> &'static str {
        match (self.bold, self.italic) {
            (true, true) => "Bold Italic",
            (true, false) => "Bold",
            (false, true) => "Italic",
            _ => "Regular",
        }
    }
    pub fn ui_name(&self) -> String {
        format!("text:{}:{}", self.family, self.style())
    }
}
pub fn fonts() -> &'static [FontFace] {
    static FONTS: OnceLock<Vec<FontFace>> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut out = vec![];
        for (family, win, linux) in [
            ("Arial", "arial", "DejaVuSans"),
            ("Times New Roman", "times", "DejaVuSerif"),
            ("Courier New", "cour", "DejaVuSansMono"),
            ("Georgia", "georgia", ""),
            ("Verdana", "verdana", ""),
        ] {
            for (bold, italic, suffix, win_suffix) in [
                (false, false, "", ""),
                (true, false, " Bold", "bd"),
                (false, true, " Italic", "i"),
                (true, true, " Bold Italic", "bi"),
            ] {
                let linux_suffix = match (bold, italic) {
                    (true, true) if linux == "DejaVuSerif" => "-BoldItalic",
                    (true, true) => "-BoldOblique",
                    (true, false) => "-Bold",
                    (false, true) if linux == "DejaVuSerif" => "-Italic",
                    (false, true) => "-Oblique",
                    _ => "",
                };
                let candidates = [
                    PathBuf::from(format!(
                        "/System/Library/Fonts/Supplemental/{family}{suffix}.ttf"
                    )),
                    PathBuf::from(
                        std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into()),
                    )
                    .join(format!("Fonts/{win}{win_suffix}.ttf")),
                    PathBuf::from(format!(
                        "/usr/share/fonts/truetype/dejavu/{linux}{linux_suffix}.ttf"
                    )),
                ];
                if let Some(path) = candidates.into_iter().find(|p| p.is_file()) {
                    // Keep the displayed family honest on Linux.
                    let family = if path.starts_with("/usr/") {
                        linux.to_owned()
                    } else {
                        family.to_owned()
                    };
                    out.push(FontFace {
                        family,
                        bold,
                        italic,
                        path,
                    });
                }
            }
        }
        out
    })
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Alignment {
    #[default]
    Left,
    Center,
    Right,
    Justified,
}
#[derive(Clone, Debug, Default)]
pub struct TextFormat {
    pub font: Option<PathBuf>,
    pub alignment: Alignment,
}
