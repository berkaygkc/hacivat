//! The settings rows: which exist, their choices, how the current value is read from the
//! settings and how a choice is applied (and saved).

use std::rc::Rc;

use hacivat_l10n::{MsgRef, NoArgs, Status, Text, ids};
use hacivat_settings::{KeymapPreset, LanguagePref, Settings, TextRenderingPref, ThemePref};
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{App, SharedString};
use hacivat_ui_core::keymap::{Collision, display_chord, parse_chord};

use crate::l10n_global::{L10nGlobal, switch_locale, tr};
use crate::shell::keymap_state::set_keymap_preset;
use crate::shell::workspace::{set_language, set_theme};
use crate::shell::{ActiveKeymap, AppState, update_settings};

/// The row keys (the TOML keys the rows edit).
pub const LANGUAGE: &str = "ui.language";
pub const REGIONAL_FORMAT: &str = "ui.regional_format";
pub const THEME: &str = "ui.theme";
pub const TEXT_RENDERING: &str = "ui.text_rendering";
pub const KEYMAP_PRESET: &str = "keymap.preset";

/// The value of the choice that follows the operating system.
pub const SYSTEM: &str = "system";

/// The page sections, in navigation order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    General,
    Appearance,
    Keyboard,
}

impl Section {
    pub const ALL: [Section; 3] = [Section::General, Section::Appearance, Section::Keyboard];

    /// The navigation label.
    pub fn title(self) -> MsgRef<NoArgs> {
        match self {
            Section::General => ids::hv::SETTINGS_NAV_GENERAL,
            Section::Appearance => ids::hv::SETTINGS_NAV_APPEARANCE,
            Section::Keyboard => ids::hv::SETTINGS_NAV_KEYBOARD,
        }
    }
}

impl Section {
    /// The page heading, pre-cased for small caps in Fluent.
    pub fn heading(self) -> MsgRef<NoArgs> {
        match self {
            Section::General => ids::hv::SETTINGS_SECTION_GENERAL,
            Section::Appearance => ids::hv::SETTINGS_SECTION_APPEARANCE,
            Section::Keyboard => ids::hv::SETTINGS_SECTION_KEYBOARD,
        }
    }
}

/// One searchable setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingRow {
    pub key: &'static str,
    pub title: MsgRef<NoArgs>,
}

/// Every row with its section, in page order. The text rendering row exists on macOS only (the
/// other platforms have no such setting to change).
const ALL_ROWS: [(Section, SettingRow); 5] = [
    (
        Section::General,
        SettingRow {
            key: LANGUAGE,
            title: ids::hv::SETTINGS_LANGUAGE,
        },
    ),
    (
        Section::General,
        SettingRow {
            key: REGIONAL_FORMAT,
            title: ids::hv::SETTINGS_REGIONAL_FORMAT,
        },
    ),
    (
        Section::Appearance,
        SettingRow {
            key: THEME,
            title: ids::hv::SETTINGS_THEME,
        },
    ),
    (
        Section::Appearance,
        SettingRow {
            key: TEXT_RENDERING,
            title: ids::hv::SETTINGS_TEXT_RENDERING,
        },
    ),
    (
        Section::Keyboard,
        SettingRow {
            key: KEYMAP_PRESET,
            title: ids::hv::SETTINGS_KEYMAP,
        },
    ),
];

/// Whether `key` is a setting on this platform.
fn available(key: &str) -> bool {
    key != TEXT_RENDERING || cfg!(target_os = "macos")
}

/// The settings on this platform, in page order.
pub fn settings_rows() -> Vec<SettingRow> {
    ALL_ROWS
        .iter()
        .filter(|(_, row)| available(row.key))
        .map(|(_, row)| *row)
        .collect()
}

/// The section of `key`.
pub fn row_section(key: &str) -> Option<Section> {
    ALL_ROWS
        .iter()
        .find(|(_, row)| row.key == key)
        .map(|(section, _)| *section)
}

/// What a choice shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChoiceLabel {
    /// A Fluent message.
    Msg(MsgRef<NoArgs>),
    /// Data shown as is: a language's native name, a locale tag, a product name.
    Data(SharedString),
}

/// One choice of a row, shown as one segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// What is stored in `settings.toml` (`system` for following the OS).
    pub value: String,
    pub label: ChoiceLabel,
}

fn msg(value: &str, id: MsgRef<NoArgs>) -> Choice {
    Choice {
        value: value.to_string(),
        label: ChoiceLabel::Msg(id),
    }
}

fn data(value: &str, label: &str) -> Choice {
    Choice {
        value: value.to_string(),
        label: ChoiceLabel::Data(label.to_string().into()),
    }
}

/// Regional formats the page offers.
const REGIONAL_FORMATS: [&str; 2] = ["en-US", "tr-TR"];

/// The choices of row `key`.
pub fn choices(cx: &App, key: &str) -> Vec<Choice> {
    match key {
        LANGUAGE => {
            let mut out = vec![msg(SYSTEM, ids::hv::SETTINGS_SYSTEM)];
            if let Some(global) = cx.try_global::<L10nGlobal>() {
                out.extend(
                    global
                        .catalogs
                        .locales()
                        .iter()
                        .filter(|l| l.status != Status::Experimental)
                        .map(|l| data(&l.tag.to_string(), &l.native_name)),
                );
            }
            out
        }
        REGIONAL_FORMAT => {
            let mut out = vec![msg(SYSTEM, ids::hv::SETTINGS_SYSTEM)];
            out.extend(REGIONAL_FORMATS.iter().map(|tag| data(tag, tag)));
            out
        }
        THEME => vec![
            msg("dark", ids::hv::THEME_DARK),
            msg("light", ids::hv::THEME_LIGHT),
            msg(SYSTEM, ids::hv::SETTINGS_SYSTEM),
        ],
        TEXT_RENDERING => vec![
            msg("crisp", ids::hv::SETTINGS_TEXT_RENDERING_CRISP),
            msg("standard", ids::hv::SETTINGS_TEXT_RENDERING_STANDARD),
        ],
        KEYMAP_PRESET => vec![
            msg("hacivat", ids::hv::SETTINGS_KEYMAP_HACIVAT),
            msg("datagrip", ids::hv::SETTINGS_KEYMAP_DATAGRIP),
        ],
        _ => Vec::new(),
    }
}

fn pref_value(pref: &LanguagePref) -> String {
    match pref {
        LanguagePref::System => SYSTEM.to_string(),
        LanguagePref::Tag(tag) => tag.clone(),
    }
}

/// The stored value of row `key` in `settings`.
pub fn current_value(settings: &Settings, key: &str) -> String {
    match key {
        LANGUAGE => pref_value(&settings.ui.language),
        REGIONAL_FORMAT => pref_value(&settings.ui.regional_format),
        THEME => match settings.ui.theme {
            ThemePref::Dark => "dark",
            ThemePref::Light => "light",
            ThemePref::System => SYSTEM,
        }
        .to_string(),
        TEXT_RENDERING => match settings.ui.text_rendering {
            TextRenderingPref::Crisp => "crisp",
            TextRenderingPref::Standard => "standard",
        }
        .to_string(),
        KEYMAP_PRESET => match settings.keymap.preset {
            KeymapPreset::Hacivat => "hacivat",
            KeymapPreset::Datagrip => "datagrip",
        }
        .to_string(),
        _ => String::new(),
    }
}

/// Whether `choice` is the one `current` stands for. A language tag matches by its language
/// subtag (`tr-TR` in the file selects the `tr` catalog's segment).
pub fn is_selected(key: &str, choice: &Choice, current: &str) -> bool {
    if choice.value == current {
        return true;
    }
    key == LANGUAGE
        && choice.value != SYSTEM
        && current != SYSTEM
        && choice.value.split('-').next() == current.split('-').next()
}

/// Applies `value` to row `key`: the change takes effect now (except text rendering, which waits
/// for a restart) and is saved to `settings.toml` through the ordered writer.
pub fn apply_choice(cx: &mut App, key: &str, value: &str) {
    // Choosing what is already chosen changes nothing: no rewrite, no repeated warning.
    let unchanged = cx
        .try_global::<AppState>()
        .is_some_and(|s| current_value(&s.settings, key) == value);
    if unchanged {
        return;
    }
    match key {
        LANGUAGE => match value {
            SYSTEM => {
                let settings = update_settings(cx, |s| s.ui.language = LanguagePref::System);
                switch_locale(cx, &settings).detach();
            }
            tag => set_language(cx, tag),
        },
        REGIONAL_FORMAT => {
            let pref = if value == SYSTEM {
                LanguagePref::System
            } else {
                LanguagePref::Tag(value.to_string())
            };
            let settings = update_settings(cx, |s| s.ui.regional_format = pref);
            switch_locale(cx, &settings).detach();
        }
        THEME => set_theme(
            cx,
            match value {
                "light" => ThemePref::Light,
                SYSTEM => ThemePref::System,
                _ => ThemePref::Dark,
            },
        ),
        TEXT_RENDERING => {
            // Saved only: GPUI reads the preference on the first glyph, so the running process
            // keeps its rendering (R11) and the next start applies the saved one.
            let pref = if value == "standard" {
                TextRenderingPref::Standard
            } else {
                TextRenderingPref::Crisp
            };
            update_settings(cx, |s| s.ui.text_rendering = pref);
        }
        KEYMAP_PRESET => {
            let preset = if value == "datagrip" {
                KeymapPreset::Datagrip
            } else {
                KeymapPreset::Hacivat
            };
            update_settings(cx, |s| s.keymap.preset = preset);
            let collisions = set_keymap_preset(cx, preset);
            warn_about_collisions(cx, &collisions);
        }
        _ => {}
    }
}

/// (chord as the platform shows it, character typed) for each collision.
fn collision_pairs(cx: &App, collisions: &[Collision]) -> Vec<(String, String)> {
    let platform = cx
        .try_global::<ActiveKeymap>()
        .map_or_else(crate::shell::keymap_state::current_platform, |a| a.platform);
    collisions
        .iter()
        .map(|c| {
            let chord = parse_chord(&c.chord).map_or_else(
                |_| c.chord.clone(),
                |presses| display_chord(&presses, platform),
            );
            (chord, c.produces.to_string())
        })
        .collect()
}

/// A warning toast listing the shortcuts that type a character on this keyboard layout (F11).
/// `ToastKind` has no warning variant: `Error` is the closest (assertive, with the triangle).
fn warn_about_collisions(cx: &mut App, collisions: &[Collision]) {
    if collisions.is_empty() {
        return;
    }
    let pairs = collision_pairs(cx, collisions);
    // Each item goes through its own message, re-read on every render (follows a language switch).
    let text = Rc::new(move |cx: &App| {
        let chords = pairs
            .iter()
            .map(|(chord, produced)| {
                tr(
                    cx,
                    ids::hv::SETTINGS_KEYMAP_COLLISION,
                    ids::hv::HvSettingsKeymapCollision {
                        chord: Text(chord.clone()),
                        char: Text(produced.clone()),
                    },
                )
                .to_string()
            })
            .collect();
        tr(
            cx,
            ids::hv::SETTINGS_KEYMAP_COLLISIONS,
            ids::hv::HvSettingsKeymapCollisions { chords },
        )
    });
    ToastQueue::push(cx, Toast::new(ToastKind::Error, text));
}
