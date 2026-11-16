//! The `settings.toml` data model (`version = 1`).
//!
//! TOML keys: `version`, `ui.language`, `ui.regional_format`, `ui.time_zone`, `ui.theme`,
//! `ui.text_rendering`, `keymap.preset`; since M3a (Task 1b, R-M3-44) `editor.tab_size`,
//! `editor.insert_spaces`, `editor.copy_line_without_selection`, `editor.caret_blink`. Values: `"system"`, `"dark"`, `"light"`, `"crisp"`,
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
    /// `[editor]` (## m3a-t01b-seams; its Settings-UI rows are M3b, R-M3-44).
    pub editor: EditorSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            ui: UiSettings::default(),
            keymap: KeymapSettings::default(),
            editor: EditorSettings::default(),
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

// ## m3a-t01b-seams

/// The `[editor]` table (R-M3-44): read by the console editor (Task 32); edited by hand in
/// M3a, its Settings-UI rows are M3b.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EditorSettings {
    /// `editor.tab_size`: columns per tab stop, 1 to 16 (default 4).
    pub tab_size: TabSize,
    /// `editor.insert_spaces`: Tab inserts spaces (default `true`).
    pub insert_spaces: bool,
    /// `editor.copy_line_without_selection`: Copy and Cut take the caret's line when nothing
    /// is selected (default `true`).
    pub copy_line_without_selection: bool,
    /// `editor.caret_blink`.
    pub caret_blink: CaretBlinkPref,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            tab_size: TabSize::default(),
            insert_spaces: true,
            copy_line_without_selection: true,
            caret_blink: CaretBlinkPref::default(),
        }
    }
}

/// A tab size, 1 to 16 columns (default 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabSize(u8);

impl TabSize {
    pub const MIN: u8 = 1;
    pub const MAX: u8 = 16;

    /// `n` when it is 1 to 16.
    pub fn new(n: u8) -> Option<Self> {
        (Self::MIN..=Self::MAX).contains(&n).then_some(Self(n))
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

impl Default for TabSize {
    fn default() -> Self {
        Self(4)
    }
}

/// `"system"` (the platform's blink; static under reduced motion) or `"off"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CaretBlinkPref {
    #[default]
    System,
    Off,
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

    // ## m3a-t01b-seams
    #[test]
    fn editor_defaults_match_the_editor_config() {
        let e = Settings::default().editor;
        assert_eq!(e.tab_size.get(), 4);
        assert!(e.insert_spaces && e.copy_line_without_selection);
        assert_eq!(e.caret_blink, CaretBlinkPref::System);
        assert_eq!(TabSize::new(0), None);
        assert_eq!(TabSize::new(17), None);
        assert_eq!(TabSize::new(16).map(TabSize::get), Some(16));
    }
}
