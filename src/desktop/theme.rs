//! Graphite, Glacier and Calico palettes, each with light and dark appearances. Every color is read for the theme that is active in
//! the current frame, so the whole interface follows a theme change at once.
use eframe::egui::{Color32, Theme, ThemePreference};
use std::cell::Cell;

#[derive(Clone, Copy)]
pub(super) struct Palette {
    pub on_accent: Color32,
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
    on_accent: Color32::WHITE,
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
    on_accent: rgb(0x182438),
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

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Style {
    #[default]
    Graphite,
    Glacier,
    Calico,
}
impl Style {
    pub const ALL: [Self; 3] = [Self::Graphite, Self::Glacier, Self::Calico];
    pub fn name(self) -> &'static str {
        match self {
            Self::Graphite => "Graphite",
            Self::Glacier => "Glacier",
            Self::Calico => "Calico",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Graphite => "graphite",
            Self::Glacier => "glacier",
            Self::Calico => "calico",
        }
    }
    pub fn from_key(value: &str) -> Self {
        match value {
            "glacier" => Self::Glacier,
            "calico" => Self::Calico,
            _ => Self::Graphite,
        }
    }
    pub fn radius(self) -> u8 {
        match self {
            Self::Graphite => 7,
            Self::Glacier => 11,
            Self::Calico => 5,
        }
    }
}

/// Control spacing is independent of the palette and light/dark appearance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Density {
    #[default]
    Compact,
    Comfortable,
}
impl Density {
    pub const ALL: [Self; 2] = [Self::Compact, Self::Comfortable];
    pub fn name(self) -> &'static str {
        match self {
            Self::Compact => "Compact",
            Self::Comfortable => "Comfortable",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Comfortable => "comfortable",
        }
    }
    fn from_key(value: &str) -> Self {
        match value {
            "comfortable" => Self::Comfortable,
            _ => Self::Compact,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum IconStyle {
    #[default]
    Selective,
    Families,
    Duotone,
}
impl IconStyle {
    pub const ALL: [Self; 3] = [Self::Selective, Self::Families, Self::Duotone];
    pub fn name(self) -> &'static str {
        match self {
            Self::Selective => "Selective accents",
            Self::Families => "Color families",
            Self::Duotone => "Soft duotone",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Selective => "selective",
            Self::Families => "families",
            Self::Duotone => "duotone",
        }
    }
    pub fn from_key(value: &str) -> Self {
        match value {
            "families" => Self::Families,
            "duotone" => Self::Duotone,
            _ => Self::Selective,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IconFamily {
    Editing,
    Signing,
    Document,
}
/// Muted tints from the approved preview. Labels retain the normal text color.
pub(super) fn icon_color(family: IconFamily) -> Color32 {
    let tint = match (family, is_dark()) {
        (IconFamily::Editing, false) => rgb(0x286393),
        (IconFamily::Editing, true) => rgb(0x91b9df),
        (IconFamily::Signing, false) => rgb(0x247467),
        (IconFamily::Signing, true) => rgb(0x81c9b8),
        (IconFamily::Document, false) => rgb(0x906020),
        (IconFamily::Document, true) => rgb(0xdbb475),
    };
    let ink = ink();
    let blend = |a: u8, b: u8| (f32::from(a) * 0.88 + f32::from(b) * 0.12).round() as u8;
    Color32::from_rgb(
        blend(tint.r(), ink.r()),
        blend(tint.g(), ink.g()),
        blend(tint.b(), ink.b()),
    )
}

const GRAPHITE_LIGHT: Palette = Palette {
    ink: rgb(0x253146),
    muted: rgb(0x607087),
    accent: rgb(0x245edb),
    accent_hover: rgb(0x1e50bc),
    accent_soft: rgb(0xe0eaff),
    accent_line: rgb(0x91b8ff),
    paper: rgb(0xf0f3f7),
    canvas: rgb(0xdfe4eb),
    chrome: rgb(0xe9edf3),
    toolbar: rgb(0xf7f9fc),
    line: rgb(0xd5dce5),
    surface: rgb(0xffffff),
    raised: rgb(0xffffff),
    track: rgb(0xe9edf3),
    control: rgb(0xe9edf3),
    control_hover: rgb(0xdce4ef),
    ..LIGHT
};

const GRAPHITE_DARK: Palette = Palette {
    ink: rgb(0xe9eef6),
    muted: rgb(0xa1adbe),
    accent: rgb(0x91b8ff),
    accent_hover: rgb(0xaac9ff),
    accent_soft: rgb(0x334762),
    accent_line: rgb(0x668ac0),
    paper: rgb(0x202630),
    canvas: rgb(0x141a22),
    chrome: rgb(0x181d25),
    toolbar: rgb(0x232a34),
    line: rgb(0x343e4c),
    surface: rgb(0x2d3541),
    raised: rgb(0x2c3542),
    track: rgb(0x252e3b),
    control: rgb(0x344050),
    control_hover: rgb(0x3f4d60),
    ..DARK
};

const GLACIER_LIGHT: Palette = Palette {
    ink: rgb(0x1f3849),
    muted: rgb(0x536f81),
    accent: rgb(0x006f91),
    accent_hover: rgb(0x005d7b),
    accent_soft: rgb(0xdceff5),
    accent_line: rgb(0x79b8cc),
    paper: rgb(0xf3f7fa),
    canvas: rgb(0xd9e4ec),
    chrome: rgb(0xe5edf3),
    toolbar: rgb(0xfbfdff),
    line: rgb(0xccdbe5),
    surface: rgb(0xffffff),
    raised: rgb(0xffffff),
    track: rgb(0xe9eff4),
    control: rgb(0xe5eef4),
    control_hover: rgb(0xd6e6ef),
    ..LIGHT
};

const GLACIER_DARK: Palette = Palette {
    ink: rgb(0xe9f3f8),
    muted: rgb(0xa4bac7),
    accent: rgb(0x89d5e9),
    accent_hover: rgb(0xa2e1f0),
    accent_soft: rgb(0x315966),
    accent_line: rgb(0x6394a7),
    paper: rgb(0x23303b),
    canvas: rgb(0x15232d),
    chrome: rgb(0x1b2631),
    toolbar: rgb(0x263541),
    line: rgb(0x3b4e5d),
    surface: rgb(0x344551),
    raised: rgb(0x344956),
    track: rgb(0x1e2c37),
    control: rgb(0x3b5260),
    control_hover: rgb(0x496473),
    ..DARK
};

const CALICO_LIGHT: Palette = Palette {
    ink: rgb(0x3d3128),
    muted: rgb(0x796653),
    accent: rgb(0x975020),
    accent_hover: rgb(0x7c4019),
    accent_soft: rgb(0xefddc6),
    accent_line: rgb(0xc2966a),
    paper: rgb(0xf2ede5),
    canvas: rgb(0xdfd8cd),
    chrome: rgb(0xeae3d9),
    toolbar: rgb(0xfaf7f1),
    line: rgb(0xd9cfc1),
    surface: rgb(0xfffdf8),
    raised: rgb(0xfffaf2),
    track: rgb(0xeae3d9),
    control: rgb(0xede4d7),
    control_hover: rgb(0xe1d4c3),
    ..LIGHT
};

const CALICO_DARK: Palette = Palette {
    ink: rgb(0xf1e8dc),
    muted: rgb(0xbaaa96),
    accent: rgb(0xe9b581),
    accent_hover: rgb(0xf0c79d),
    accent_soft: rgb(0x51402e),
    accent_line: rgb(0xa98056),
    paper: rgb(0x2a2622),
    canvas: rgb(0x1e1c19),
    chrome: rgb(0x25221f),
    toolbar: rgb(0x302b26),
    line: rgb(0x4b4136),
    surface: rgb(0x3b342c),
    raised: rgb(0x3b3229),
    track: rgb(0x302820),
    control: rgb(0x493d31),
    control_hover: rgb(0x594938),
    ..DARK
};

thread_local! {
    static STYLE: Cell<Style> = const { Cell::new(Style::Graphite) };
    static ICON_STYLE: Cell<IconStyle> = const { Cell::new(IconStyle::Selective) };
    static DENSITY: Cell<Density> = const { Cell::new(Density::Compact) };
    static DARK_MODE: Cell<bool> = const { Cell::new(false) };
}
/// Follow the theme egui resolved for this frame (including Match System).
pub(super) fn sync(theme: Theme) {
    DARK_MODE.with(|d| d.set(theme == Theme::Dark));
}
pub(super) fn is_dark() -> bool {
    DARK_MODE.with(Cell::get)
}
pub(super) fn style() -> Style {
    STYLE.with(Cell::get)
}
pub(super) fn set_style(style: Style) {
    STYLE.with(|s| s.set(style));
}
pub(super) fn icon_style() -> IconStyle {
    ICON_STYLE.with(Cell::get)
}
pub(super) fn set_icon_style(style: IconStyle) {
    ICON_STYLE.with(|s| s.set(style));
}
pub(super) fn density() -> Density {
    DENSITY.with(Cell::get)
}
pub(super) fn comfortable() -> bool {
    density() == Density::Comfortable
}
pub(super) fn set_density(density: Density) {
    DENSITY.with(|value| value.set(density));
}
pub(super) fn palette_for(dark: bool) -> &'static Palette {
    palette_for_style(style(), dark)
}
pub(super) fn palette_for_style(style: Style, dark: bool) -> &'static Palette {
    match (style, dark) {
        (Style::Graphite, false) => &GRAPHITE_LIGHT,
        (Style::Graphite, true) => &GRAPHITE_DARK,
        (Style::Glacier, false) => &GLACIER_LIGHT,
        (Style::Glacier, true) => &GLACIER_DARK,
        (Style::Calico, false) => &CALICO_LIGHT,
        (Style::Calico, true) => &CALICO_DARK,
    }
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
    on_accent,
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

/// Appearance and visual style share a file, but changing either preserves the
/// other preference and any fields written by future versions.
fn decode(value: &serde_json::Value) -> (ThemePreference, Style) {
    let preference = match value.get("theme").and_then(|t| t.as_str()) {
        Some("light") => ThemePreference::Light,
        Some("dark") => ThemePreference::Dark,
        _ => ThemePreference::System,
    };
    let style = Style::from_key(
        value
            .get("style")
            .and_then(|t| t.as_str())
            .unwrap_or("graphite"),
    );
    (preference, style)
}
pub(super) fn load_preferences() -> (ThemePreference, Style) {
    #[cfg(not(test))]
    if let Ok(value) = super::preferences::load::<serde_json::Value>(
        &mypdf_milestone0::assets::data_directory(),
        "interface.json",
    ) {
        return decode(&value);
    }
    decode(&serde_json::Value::Null)
}
fn decode_icon_style(value: &serde_json::Value) -> IconStyle {
    IconStyle::from_key(
        value
            .get("icons")
            .and_then(|v| v.as_str())
            .unwrap_or("selective"),
    )
}
pub(super) fn load_icon_style() -> IconStyle {
    #[cfg(not(test))]
    if let Ok(value) = super::preferences::load::<serde_json::Value>(
        &mypdf_milestone0::assets::data_directory(),
        "interface.json",
    ) {
        return decode_icon_style(&value);
    }
    decode_icon_style(&serde_json::Value::Null)
}
fn decode_density(value: &serde_json::Value) -> Density {
    Density::from_key(
        value
            .get("density")
            .and_then(|v| v.as_str())
            .unwrap_or("compact"),
    )
}
pub(super) fn load_density() -> Density {
    #[cfg(not(test))]
    if let Ok(value) = super::preferences::load::<serde_json::Value>(
        &mypdf_milestone0::assets::data_directory(),
        "interface.json",
    ) {
        return decode_density(&value);
    }
    decode_density(&serde_json::Value::Null)
}
fn save_field(root: &std::path::Path, key: &str, value: &str) -> anyhow::Result<()> {
    let mut stored = super::preferences::load::<serde_json::Value>(root, "interface.json")?;
    if stored.is_null() {
        stored = serde_json::json!({});
    }
    anyhow::ensure!(stored.is_object(), "Invalid appearance preferences");
    stored[key] = value.into();
    super::preferences::save(root, "interface.json", &stored)
}
pub(super) fn save_preference(preference: ThemePreference) -> anyhow::Result<()> {
    let name = match preference {
        ThemePreference::Light => "light",
        ThemePreference::Dark => "dark",
        ThemePreference::System => "system",
    };
    save_current("theme", name)
}
pub(super) fn save_style(style: Style) -> anyhow::Result<()> {
    save_current("style", style.key())
}
pub(super) fn save_icon_style(style: IconStyle) -> anyhow::Result<()> {
    save_current("icons", style.key())
}
pub(super) fn save_density(density: Density) -> anyhow::Result<()> {
    save_current("density", density.key())
}
fn save_current(key: &str, value: &str) -> anyhow::Result<()> {
    #[cfg(not(test))]
    {
        save_field(&mypdf_milestone0::assets::data_directory(), key, value)
    }
    #[cfg(test)]
    {
        let _ = (key, value);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn density_defaults_and_roundtrips_preserve_every_other_preference() {
        for value in [
            serde_json::Value::Null,
            serde_json::json!({}),
            serde_json::json!({"density":"future"}),
            serde_json::json!({"density":true}),
        ] {
            assert_eq!(decode_density(&value), Density::Compact);
        }
        let root = tempfile::tempdir().unwrap();
        let initial = serde_json::json!({"theme":"dark", "style":"calico", "icons":"duotone", "future_option":{"enabled":true}});
        super::super::preferences::save(root.path(), "interface.json", &initial).unwrap();
        for density in Density::ALL {
            save_field(root.path(), "density", density.key()).unwrap();
            let saved =
                super::super::preferences::load::<serde_json::Value>(root.path(), "interface.json")
                    .unwrap();
            assert_eq!(decode_density(&saved), density);
            let mut without_density = saved.clone();
            without_density.as_object_mut().unwrap().remove("density");
            assert_eq!(without_density, initial);
            save_field(root.path(), "theme", "light").unwrap();
            save_field(root.path(), "style", "glacier").unwrap();
            save_field(root.path(), "icons", "families").unwrap();
            let saved =
                super::super::preferences::load::<serde_json::Value>(root.path(), "interface.json")
                    .unwrap();
            assert_eq!(decode_density(&saved), density);
            assert_eq!(saved["future_option"], initial["future_option"]);
            super::super::preferences::save(root.path(), "interface.json", &initial).unwrap();
        }
        let invalid = serde_json::json!(["preserve invalid data"]);
        super::super::preferences::save(root.path(), "interface.json", &invalid).unwrap();
        assert!(save_field(root.path(), "density", "comfortable").is_err());
        assert_eq!(
            super::super::preferences::load::<serde_json::Value>(root.path(), "interface.json")
                .unwrap(),
            invalid
        );
    }

    #[test]
    fn all_styles_keep_text_and_selected_controls_readable() {
        fn luminance(c: Color32) -> f32 {
            let linear = |v: u8| {
                let v = f32::from(v) / 255.;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(c.r()) + 0.7152 * linear(c.g()) + 0.0722 * linear(c.b())
        }
        for style in Style::ALL {
            for dark in [false, true] {
                let p = palette_for_style(style, dark);
                for (fg, bg) in [
                    (p.ink, p.toolbar),
                    (p.ink, p.paper),
                    (p.ink, p.surface),
                    (p.muted, p.paper),
                    (p.muted, p.toolbar),
                    (p.accent, p.accent_soft),
                    (p.on_accent, p.accent),
                    (p.on_accent, p.accent_hover),
                ] {
                    let (a, b) = (luminance(fg), luminance(bg));
                    let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                    assert!(
                        contrast >= 4.5,
                        "{style:?} dark={dark}: {fg:?} / {bg:?} contrast={contrast}"
                    );
                }
            }
        }
    }
    #[test]
    fn icon_preferences_preserve_existing_theme_and_other_settings() {
        let root = tempfile::tempdir().unwrap();
        let initial = serde_json::json!({"theme":"dark", "style":"calico", "future_option":42});
        super::super::preferences::save(root.path(), "interface.json", &initial).unwrap();
        assert_eq!(decode_icon_style(&initial), IconStyle::Selective);
        for icons in IconStyle::ALL {
            save_field(root.path(), "icons", icons.key()).unwrap();
            let saved =
                super::super::preferences::load::<serde_json::Value>(root.path(), "interface.json")
                    .unwrap();
            assert_eq!(decode(&saved), (ThemePreference::Dark, Style::Calico));
            assert_eq!(decode_icon_style(&saved), icons);
            assert_eq!(saved["future_option"], 42);
            save_field(root.path(), "theme", "light").unwrap();
            let saved =
                super::super::preferences::load::<serde_json::Value>(root.path(), "interface.json")
                    .unwrap();
            assert_eq!(decode_icon_style(&saved), icons);
            save_field(root.path(), "theme", "dark").unwrap();
        }
        assert_eq!(
            decode_icon_style(&serde_json::json!({"icons":"future"})),
            IconStyle::Selective
        );
    }
    #[test]
    fn old_preferences_default_to_graphite_and_independent_choices_round_trip() {
        let root = tempfile::tempdir().unwrap();
        let initial = serde_json::json!({"theme":"dark", "future_option":42});
        super::super::preferences::save(root.path(), "interface.json", &initial).unwrap();
        assert_eq!(decode(&initial), (ThemePreference::Dark, Style::Graphite));
        for style in Style::ALL {
            save_field(root.path(), "style", style.key()).unwrap();
            let saved =
                super::super::preferences::load::<serde_json::Value>(root.path(), "interface.json")
                    .unwrap();
            assert_eq!(decode(&saved), (ThemePreference::Dark, style));
            save_field(root.path(), "theme", "light").unwrap();
            let saved =
                super::super::preferences::load::<serde_json::Value>(root.path(), "interface.json")
                    .unwrap();
            assert_eq!(decode(&saved), (ThemePreference::Light, style));
            assert_eq!(saved["future_option"], 42);
            save_field(root.path(), "theme", "dark").unwrap();
        }
        assert_eq!(
            decode(&serde_json::json!({"style":"future"})),
            (ThemePreference::System, Style::Graphite)
        );
    }
}
