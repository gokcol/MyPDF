//! Light and dark palettes. Every color is read for the theme that is active in
//! the current frame, so the whole interface follows a theme change at once.
use eframe::egui::{Color32, Theme, ThemePreference};
use std::cell::Cell;

pub(super) struct Palette {
    /// Primary text and icons.
    pub ink: Color32,
    /// Secondary text and captions.
    pub muted: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    /// Selected rows and soft highlights.
    pub accent_soft: Color32,
    /// Borders of selected or hovered cards.
    pub accent_line: Color32,
    /// Side panels and the Home background.
    pub paper: Color32,
    /// The backdrop behind document pages.
    pub canvas: Color32,
    /// The document tab strip.
    pub chrome: Color32,
    /// The main toolbar and footer.
    pub toolbar: Color32,
    /// Hairline separators and card borders.
    pub line: Color32,
    /// Cards, ribbons, windows and text fields.
    pub surface: Color32,
    /// A control floating above a track, such as the selected segment or tab.
    pub raised: Color32,
    /// The recessed track behind segmented controls.
    pub track: Color32,
    /// Hover and press overlays on transparent controls.
    pub hover: Color32,
    pub pressed: Color32,
    /// Filled secondary buttons.
    pub control: Color32,
    pub control_hover: Color32,
    pub danger: Color32,
    pub danger_soft: Color32,
    pub success: Color32,
    pub warning: Color32,
}

const LIGHT: Palette = Palette {
    ink: Color32::from_rgb(30, 31, 36),
    muted: Color32::from_rgb(108, 111, 122),
    accent: Color32::from_rgb(40, 104, 245),
    accent_hover: Color32::from_rgb(27, 86, 220),
    accent_soft: Color32::from_rgb(233, 240, 255),
    accent_line: Color32::from_rgb(184, 205, 255),
    paper: Color32::from_rgb(248, 248, 250),
    canvas: Color32::from_rgb(230, 232, 237),
    chrome: Color32::from_rgb(233, 234, 239),
    toolbar: Color32::from_rgb(246, 246, 249),
    line: Color32::from_rgb(222, 224, 230),
    surface: Color32::WHITE,
    raised: Color32::WHITE,
    track: Color32::from_rgb(229, 230, 236),
    hover: Color32::from_black_alpha(11),
    pressed: Color32::from_black_alpha(22),
    control: Color32::from_rgb(236, 237, 241),
    control_hover: Color32::from_rgb(225, 226, 232),
    danger: Color32::from_rgb(214, 52, 56),
    danger_soft: Color32::from_rgb(255, 241, 240),
    success: Color32::from_rgb(34, 150, 84),
    warning: Color32::from_rgb(222, 140, 18),
};

const DARK: Palette = Palette {
    ink: Color32::from_rgb(236, 237, 242),
    muted: Color32::from_rgb(150, 154, 167),
    accent: Color32::from_rgb(76, 139, 255),
    accent_hover: Color32::from_rgb(104, 158, 255),
    accent_soft: Color32::from_rgb(35, 50, 84),
    accent_line: Color32::from_rgb(64, 96, 168),
    paper: Color32::from_rgb(30, 31, 36),
    canvas: Color32::from_rgb(21, 22, 26),
    chrome: Color32::from_rgb(25, 26, 30),
    toolbar: Color32::from_rgb(35, 36, 41),
    line: Color32::from_rgb(54, 56, 64),
    surface: Color32::from_rgb(41, 42, 48),
    raised: Color32::from_rgb(66, 68, 77),
    track: Color32::from_rgb(22, 23, 27),
    hover: Color32::from_rgba_premultiplied(16, 16, 16, 16),
    pressed: Color32::from_rgba_premultiplied(30, 30, 30, 30),
    control: Color32::from_rgb(56, 58, 66),
    control_hover: Color32::from_rgb(68, 70, 79),
    danger: Color32::from_rgb(255, 104, 96),
    danger_soft: Color32::from_rgb(64, 32, 33),
    success: Color32::from_rgb(70, 200, 122),
    warning: Color32::from_rgb(255, 178, 54),
};

thread_local! {
    static DARK_MODE: Cell<bool> = const { Cell::new(false) };
}
/// Follow the theme egui resolved for this frame (including Match System).
pub(super) fn sync(theme: Theme) {
    DARK_MODE.with(|d| d.set(theme == Theme::Dark));
}
pub(super) fn is_dark() -> bool {
    DARK_MODE.with(Cell::get)
}
pub(super) fn palette_for(dark: bool) -> &'static Palette {
    if dark { &DARK } else { &LIGHT }
}
fn palette() -> &'static Palette {
    palette_for(is_dark())
}

macro_rules! tokens {
    ($($name:ident),* $(,)?) => {
        $(pub(super) fn $name() -> Color32 {
            palette().$name
        })*
    };
}
tokens!(
    ink,
    muted,
    accent,
    accent_hover,
    accent_soft,
    accent_line,
    paper,
    canvas,
    chrome,
    toolbar,
    line,
    surface,
    raised,
    track,
    hover,
    pressed,
    control,
    control_hover,
    danger,
    danger_soft,
    success,
    warning,
);

/// The remembered Match System / Light / Dark choice.
pub(super) fn load_preference() -> ThemePreference {
    #[cfg(not(test))]
    if let Ok(raw) = std::fs::read_to_string(preference_file())
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw)
    {
        return match value.get("theme").and_then(|t| t.as_str()) {
            Some("light") => ThemePreference::Light,
            Some("dark") => ThemePreference::Dark,
            _ => ThemePreference::System,
        };
    }
    ThemePreference::System
}
pub(super) fn save_preference(preference: ThemePreference) -> std::io::Result<()> {
    #[cfg(not(test))]
    {
        let name = match preference {
            ThemePreference::Light => "light",
            ThemePreference::Dark => "dark",
            ThemePreference::System => "system",
        };
        let path = preference_file();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::json!({ "theme": name }).to_string())?;
        std::fs::rename(tmp, path)?;
    }
    #[cfg(test)]
    let _ = preference;
    Ok(())
}
#[cfg(not(test))]
fn preference_file() -> std::path::PathBuf {
    mypdf_milestone0::assets::data_directory().join("interface.json")
}
