//! The `settings.toml` data model (`version = 1`).
//!
//! TOML keys: `version`, `ui.language`, `ui.regional_format`, `ui.time_zone`, `ui.theme`,
//! `ui.text_rendering`, `keymap.preset`. Values: `"system"`, `"dark"`, `"light"`, `"crisp"`,
//! `"standard"`, `"hacivat"`, `"datagrip"`, or a BCP-47 tag or IANA zone. Reading and writing
//! (with per-key validation) land in Task 7.

/// The current settings format version.
pub const SETTINGS_VERSION: u32 = 1;

/// The whole settings file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Settings {
    pub version: u32,
    pub ui: UiSettings,
    pub keymap: KeymapSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            ui: UiSettings::default(),
            keymap: KeymapSettings::default(),
        }
    }
}

/// The `[ui]` table.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct UiSettings {
    /// `ui.language`: the UI language.
    pub language: LanguagePref,
    /// `ui.regional_format`: the locale for numbers, dates and times (list conjunctions follow
    /// the UI language, m2b-t22-live ruling).
    pub regional_format: LanguagePref,
    /// `ui.time_zone`.
    pub time_zone: TimeZonePref,
    /// `ui.theme`.
    pub theme: ThemePref,
    /// `ui.text_rendering` (applies after restart, R11).
    pub text_rendering: TextRenderingPref,
}

/// `"system"` or a BCP-47 tag such as `"tr"` or `"en-GB"`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum LanguagePref {
    #[default]
    System,
    Tag(String),
}

/// `"system"` or an IANA zone such as `"Europe/Istanbul"`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum TimeZonePref {
    #[default]
    System,
    Iana(String),
}

/// `"dark"`, `"light"` or `"system"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ThemePref {
    #[default]
    Dark,
    Light,
    System,
}

/// `"crisp"` or `"standard"` (L12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextRenderingPref {
    #[default]
    Crisp,
    Standard,
}

/// The `[keymap]` table.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct KeymapSettings {
    /// `keymap.preset`.
    pub preset: KeymapPreset,
}

/// `"hacivat"` or `"datagrip"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum KeymapPreset {
    #[default]
    Hacivat,
    Datagrip,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec() {
        let s = Settings::default();
        assert_eq!(
            (s.version, s.ui.theme, s.ui.text_rendering, s.keymap.preset),
            (
                1,
                ThemePref::Dark,
                TextRenderingPref::Crisp,
                KeymapPreset::Hacivat
            )
        );
        assert_eq!(
            (s.ui.language, s.ui.regional_format, s.ui.time_zone),
            (
                LanguagePref::System,
                LanguagePref::System,
                TimeZonePref::System
            )
        );
    }
}
