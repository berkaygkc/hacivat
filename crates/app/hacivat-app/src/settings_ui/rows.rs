//! The settings rows: which exist, their choices, how the current value is read from the
//! settings and how a choice is applied (and saved).

use std::rc::Rc;

use hacivat_l10n::{MsgRef, NoArgs, Status, Text, ids};
use hacivat_settings::{
    CaretBlinkPref, FindTolerancePref, KeymapPreset, LanguagePref, Settings, TabSize,
    TextRenderingPref, ThemePref,
};
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{App, SharedString};
use hacivat_ui_core::keymap::{Collision, display_chord, parse_chord};
use hacivat_ui_editor::CaretBlink;

use crate::console::Consoles;
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
// ## m3b-t42-settings: the M3b settings (R-M3-44; NOTES surface 25).
pub const TAB_SIZE: &str = "editor.tab_size";
pub const INSERT_SPACES: &str = "editor.insert_spaces";
pub const COPY_LINE: &str = "editor.copy_line_without_selection";
pub const CARET_BLINK: &str = "editor.caret_blink";
pub const FIND_TOLERANT: &str = "editor.find_turkish_tolerant";
pub const CONFIRM_UNSAFE_WHERE: &str = "exec.confirm_unsafe_where";
pub const ERROR_POLICY_ASK: &str = "exec.error_policy_ask";
pub const MAX_VALUE_BYTES: &str = "results.max_value_bytes";
pub const RAW_DISPLAY: &str = "grid.raw_display";
pub const HISTORY_ENABLED: &str = "history.enabled";

/// The value of the choice that follows the operating system.
pub const SYSTEM: &str = "system";
// ## m3b-t42-settings
/// The values of an on/off row, and the Turkish-tolerant row's third choice.
pub const ON: &str = "on";
pub const OFF: &str = "off";
pub const AUTO: &str = "auto";

/// The tab sizes the Tab size row offers; a hand-edited other size shows as one more segment.
const TAB_SIZES: [u8; 3] = [2, 4, 8];

/// The page sections, in navigation order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    General,
    Appearance,
    Keyboard,
    // ## m3b-t42-settings (ruling M3c-37: four sections rather than one "M3b" section)
    Editor,
    Execution,
    Results,
    History,
}

impl Section {
    pub const ALL: [Section; 7] = [
        Section::General,
        Section::Appearance,
        Section::Keyboard,
        // ## m3b-t42-settings
        Section::Editor,
        Section::Execution,
        Section::Results,
        Section::History,
    ];

    /// The navigation label (and the section a search result names).
    pub fn title(self) -> MsgRef<NoArgs> {
        match self {
            Section::General => ids::hv::SETTINGS_NAV_GENERAL,
            Section::Appearance => ids::hv::SETTINGS_NAV_APPEARANCE,
            Section::Keyboard => ids::hv::SETTINGS_NAV_KEYBOARD,
            // ## m3b-t42-settings
            Section::Editor => ids::hv::SETTINGS_NAV_EDITOR,
            Section::Execution => ids::hv::SETTINGS_NAV_EXECUTION,
            Section::Results => ids::hv::SETTINGS_NAV_RESULTS,
            Section::History => ids::hv::SETTINGS_NAV_HISTORY,
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
            // ## m3b-t42-settings
            Section::Editor => ids::hv::SETTINGS_SECTION_EDITOR,
            Section::Execution => ids::hv::SETTINGS_SECTION_EXECUTION,
            Section::Results => ids::hv::SETTINGS_SECTION_RESULTS,
            Section::History => ids::hv::SETTINGS_SECTION_HISTORY,
        }
    }
}

/// One searchable setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingRow {
    pub key: &'static str,
    pub title: MsgRef<NoArgs>,
}

// ## m3b-t42-settings
/// A row of the page: its section, its title, the note under its control, and the element IDs
/// its title and note are drawn with (the tests and the shots read them).
#[derive(Debug, Clone, Copy)]
struct RowSpec {
    section: Section,
    row: SettingRow,
    note: Option<MsgRef<NoArgs>>,
    title_id: &'static str,
    note_id: &'static str,
}

/// A [`RowSpec`] whose element IDs are made from its key at compile time (a `Paragraph`'s ID
/// is a `&'static str`).
macro_rules! row {
    ($section:ident, $key:literal, $title:expr) => {
        row!(@spec $section, $key, $title, None)
    };
    ($section:ident, $key:literal, $title:expr, $note:expr) => {
        row!(@spec $section, $key, $title, Some($note))
    };
    (@spec $section:ident, $key:literal, $title:expr, $note:expr) => {
        RowSpec {
            section: Section::$section,
            row: SettingRow {
                key: $key,
                title: $title,
            },
            note: $note,
            title_id: concat!("settings-row-title-", $key),
            note_id: concat!("settings-row-note-", $key),
        }
    };
}

/// Every row with its section, in page order. The text rendering row exists on macOS only (the
/// other platforms have no such setting to change).
const ALL_ROWS: [RowSpec; 15] = [
    row!(General, "ui.language", ids::hv::SETTINGS_LANGUAGE),
    row!(
        General,
        "ui.regional_format",
        ids::hv::SETTINGS_REGIONAL_FORMAT
    ),
    row!(Appearance, "ui.theme", ids::hv::SETTINGS_THEME),
    row!(
        Appearance,
        "ui.text_rendering",
        ids::hv::SETTINGS_TEXT_RENDERING
    ),
    row!(Keyboard, "keymap.preset", ids::hv::SETTINGS_KEYMAP),
    // ## m3b-t42-settings (NOTES surface 25)
    row!(Editor, "editor.tab_size", ids::hv::SETTINGS_TAB_SIZE),
    row!(
        Editor,
        "editor.insert_spaces",
        ids::hv::SETTINGS_INSERT_SPACES
    ),
    row!(
        Editor,
        "editor.copy_line_without_selection",
        ids::hv::SETTINGS_COPY_LINE
    ),
    row!(Editor, "editor.caret_blink", ids::hv::SETTINGS_CARET_BLINK),
    row!(
        Editor,
        "editor.find_turkish_tolerant",
        ids::hv::SETTINGS_FIND_TOLERANT,
        ids::hv::SETTINGS_FIND_TOLERANT_NOTE
    ),
    row!(
        Execution,
        "exec.confirm_unsafe_where",
        ids::hv::SETTINGS_CONFIRM_UNSAFE_WHERE,
        ids::hv::SETTINGS_CONFIRM_UNSAFE_WHERE_NOTE
    ),
    row!(
        Execution,
        "exec.error_policy_ask",
        ids::hv::SETTINGS_ERROR_POLICY,
        ids::hv::SETTINGS_ERROR_POLICY_NOTE
    ),
    row!(
        Results,
        "results.max_value_bytes",
        ids::hv::SETTINGS_MAX_VALUE,
        ids::hv::SETTINGS_MAX_VALUE_NOTE
    ),
    row!(
        Results,
        "grid.raw_display",
        ids::hv::SETTINGS_RAW_DISPLAY,
        ids::hv::SETTINGS_RAW_DISPLAY_NOTE
    ),
    row!(
        History,
        "history.enabled",
        ids::hv::SETTINGS_HISTORY,
        ids::hv::SETTINGS_HISTORY_NOTE
    ),
];

fn spec(key: &str) -> Option<&'static RowSpec> {
    ALL_ROWS.iter().find(|r| r.row.key == key)
}

/// Whether `key` is a setting on this platform.
fn available(key: &str) -> bool {
    key != TEXT_RENDERING || cfg!(target_os = "macos")
}

/// The settings on this platform, in page order.
pub fn settings_rows() -> Vec<SettingRow> {
    ALL_ROWS
        .iter()
        .filter(|r| available(r.row.key))
        .map(|r| r.row)
        .collect()
}

/// The section of `key`.
pub fn row_section(key: &str) -> Option<Section> {
    spec(key).map(|r| r.section)
}

// ## m3b-t42-settings
/// The note under row `key`'s control, if it has one.
pub fn row_note(key: &str) -> Option<MsgRef<NoArgs>> {
    spec(key).and_then(|r| r.note)
}

/// The element IDs of row `key`'s title and note.
pub fn row_ids(key: &str) -> Option<(&'static str, &'static str)> {
    spec(key).map(|r| (r.title_id, r.note_id))
}

/// Whether row `key` shows its choices in the monospace face (the tab sizes, NOTES surface 25).
pub fn is_mono(key: &str) -> bool {
    key == TAB_SIZE
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

/// The choices of row `key` (none for the largest-value row, a field with a unit).
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
        // ## m3b-t42-settings
        TAB_SIZE => {
            let current = cx
                .try_global::<AppState>()
                .map_or_else(TabSize::default, |s| s.settings.editor.tab_size)
                .get();
            let mut sizes = TAB_SIZES.to_vec();
            if !sizes.contains(&current) {
                sizes.push(current);
                sizes.sort_unstable();
            }
            sizes
                .into_iter()
                .map(|n| data(&n.to_string(), &n.to_string()))
                .collect()
        }
        FIND_TOLERANT => vec![
            msg(AUTO, ids::hv::SETTINGS_AUTO),
            msg(ON, ids::hv::SETTINGS_ON),
            msg(OFF, ids::hv::SETTINGS_OFF),
        ],
        INSERT_SPACES | COPY_LINE | CARET_BLINK | CONFIRM_UNSAFE_WHERE | ERROR_POLICY_ASK
        | RAW_DISPLAY | HISTORY_ENABLED => vec![
            msg(ON, ids::hv::SETTINGS_ON),
            msg(OFF, ids::hv::SETTINGS_OFF),
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

// ## m3b-t42-settings
fn on_off(on: bool) -> String {
    if on { ON } else { OFF }.to_string()
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
        // ## m3b-t42-settings
        TAB_SIZE => settings.editor.tab_size.get().to_string(),
        INSERT_SPACES => on_off(settings.editor.insert_spaces),
        COPY_LINE => on_off(settings.editor.copy_line_without_selection),
        CARET_BLINK => on_off(settings.editor.caret_blink == CaretBlinkPref::System),
        FIND_TOLERANT => match settings.editor.find_turkish_tolerant {
            FindTolerancePref::Auto => AUTO,
            FindTolerancePref::On => ON,
            FindTolerancePref::Off => OFF,
        }
        .to_string(),
        CONFIRM_UNSAFE_WHERE => on_off(settings.exec.confirm_unsafe_where),
        ERROR_POLICY_ASK => on_off(settings.exec.error_policy_ask),
        MAX_VALUE_BYTES => settings.results.max_value_bytes.get().to_string(),
        RAW_DISPLAY => on_off(settings.grid.raw_display),
        HISTORY_ENABLED => on_off(settings.history.enabled),
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
        // ## m3b-t42-settings: each through the one writer; the editor rows also reach the
        // open editors (a console reads `[editor]` only when it makes its editor).
        TAB_SIZE => {
            if let Some(size) = value.parse().ok().and_then(TabSize::new) {
                update_settings(cx, |s| s.editor.tab_size = size);
                apply_editor_settings(cx);
            }
        }
        INSERT_SPACES => {
            update_settings(cx, |s| s.editor.insert_spaces = value == ON);
            apply_editor_settings(cx);
        }
        COPY_LINE => {
            update_settings(cx, |s| s.editor.copy_line_without_selection = value == ON);
            apply_editor_settings(cx);
        }
        CARET_BLINK => {
            let blink = if value == ON {
                CaretBlinkPref::System
            } else {
                CaretBlinkPref::Off
            };
            update_settings(cx, |s| s.editor.caret_blink = blink);
            apply_editor_settings(cx);
        }
        FIND_TOLERANT => {
            // `auto` is the default, which the store does not write (T33).
            let pref = match value {
                ON => FindTolerancePref::On,
                OFF => FindTolerancePref::Off,
                _ => FindTolerancePref::Auto,
            };
            update_settings(cx, |s| s.editor.find_turkish_tolerant = pref);
        }
        CONFIRM_UNSAFE_WHERE => {
            update_settings(cx, |s| s.exec.confirm_unsafe_where = value == ON);
        }
        ERROR_POLICY_ASK => {
            update_settings(cx, |s| s.exec.error_policy_ask = value == ON);
        }
        RAW_DISPLAY => {
            update_settings(cx, |s| s.grid.raw_display = value == ON);
        }
        HISTORY_ENABLED => {
            update_settings(cx, |s| s.history.enabled = value == ON);
        }
        _ => {}
    }
}

// ## m3b-t42-settings
/// Hands the `[editor]` settings to every open console's editor, keeping what the console set
/// itself (read-only, the tiers, the screen-reader window).
fn apply_editor_settings(cx: &mut App) {
    let Some(editor) = cx.try_global::<AppState>().map(|s| s.settings.editor) else {
        return;
    };
    let views: Vec<_> = Consoles::ids(cx)
        .into_iter()
        .filter_map(|id| Consoles::get(cx, id).and_then(|c| c.editor.clone()))
        .collect();
    for view in views {
        view.update(cx, |view, cx| {
            let mut cfg = *view.config();
            cfg.tab_size = editor.tab_size.get();
            cfg.insert_spaces = editor.insert_spaces;
            cfg.copy_line_without_selection = editor.copy_line_without_selection;
            cfg.caret_blink = match editor.caret_blink {
                CaretBlinkPref::System => CaretBlink::System,
                CaretBlinkPref::Off => CaretBlink::Off,
            };
            view.set_config(cfg, cx);
        });
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
