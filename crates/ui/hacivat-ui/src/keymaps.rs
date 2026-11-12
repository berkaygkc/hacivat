//! Embedded keymap data: default keymaps, presets, layout overlays and AltGr tables (UI-EDIT-13).
//!
//! The data lives in `keymaps/` next to this crate and is compiled in, so Settings and CI read
//! the same tables (F11). The model (parsing, resolution, checks) is `hacivat_ui_core::keymap`.

use hacivat_ui_core::keymap::{
    AltGrTable, AltGrTables, Collision, KeyboardLayoutId, Keymap, KeymapDiag, KeymapPlatform,
};
use serde::Deserialize;

/// The keymap presets, in the order Settings lists them.
pub const PRESETS: [&str; 2] = ["hacivat", "datagrip"];

const DEFAULT_MACOS: &str = include_str!("../keymaps/default-macos.toml");
const DEFAULT_WINDOWS: &str = include_str!("../keymaps/default-windows.toml");
const DEFAULT_LINUX: &str = include_str!("../keymaps/default-linux.toml");

const PRESET_HACIVAT: &str = include_str!("../keymaps/presets/hacivat.toml");
const PRESET_DATAGRIP: &str = include_str!("../keymaps/presets/datagrip.toml");

/// Layout overlays: `(layout id, file)`. `us` has no overlay: its base layer needs no help.
const LAYOUTS: [(&str, &str); 5] = [
    ("tr-q", include_str!("../keymaps/layouts/tr-q.toml")),
    ("tr-f", include_str!("../keymaps/layouts/tr-f.toml")),
    ("de", include_str!("../keymaps/layouts/de.toml")),
    ("fr", include_str!("../keymaps/layouts/fr.toml")),
    ("pl", include_str!("../keymaps/layouts/pl.toml")),
];

const ALTGR: [&str; 5] = [
    include_str!("../keymaps/altgr/tr-q.toml"),
    include_str!("../keymaps/altgr/tr-f.toml"),
    include_str!("../keymaps/altgr/de.toml"),
    include_str!("../keymaps/altgr/fr.toml"),
    include_str!("../keymaps/altgr/pl.toml"),
];

fn preset_text(preset: &str) -> Option<&'static str> {
    match preset {
        "hacivat" => Some(PRESET_HACIVAT),
        "datagrip" => Some(PRESET_DATAGRIP),
        _ => None,
    }
}

/// Loads the keymap for `platform`: the platform defaults, then `preset`, the layout overlays
/// and the optional `user` keymap file text. An unknown preset name adds a `Parse` diagnostic
/// and loads the defaults only.
pub fn load(
    platform: KeymapPlatform,
    preset: &str,
    user: Option<&str>,
) -> (Keymap, Vec<KeymapDiag>) {
    let default = match platform {
        KeymapPlatform::MacOs => DEFAULT_MACOS,
        KeymapPlatform::Windows => DEFAULT_WINDOWS,
        KeymapPlatform::Linux => DEFAULT_LINUX,
    };
    let ids: Vec<KeyboardLayoutId> = LAYOUTS
        .iter()
        .map(|(id, _)| KeyboardLayoutId((*id).to_string()))
        .collect();
    let overlays: Vec<(&KeyboardLayoutId, &str)> = ids
        .iter()
        .zip(LAYOUTS.iter())
        .map(|(id, (_, text))| (id, *text))
        .collect();
    let (preset_text, unknown) = match preset_text(preset) {
        Some(text) => (text, false),
        None => ("", true),
    };
    let (km, mut diags) = Keymap::from_sources(platform, default, preset_text, &overlays, user);
    if unknown {
        diags.push(KeymapDiag::Parse {
            source: hacivat_ui_core::keymap::BindingSource::Preset,
            line: 0,
            message: format!("unknown keymap preset `{preset}`"),
        });
    }
    (km, diags)
}

/// The embedded AltGr tables (TR-Q, TR-F, DE, FR, PL), used at runtime by Settings and in CI.
pub fn layouts_altgr() -> AltGrTables {
    AltGrTables(ALTGR.iter().map(|t| AltGrTable::parse(t)).collect())
}

#[derive(Deserialize)]
struct ShiftFile {
    shift_level: Option<String>,
}

/// Base-layer characters that need Shift on `layout` (`shift_level` of its overlay file); empty
/// for a layout without an overlay such as `us`.
pub fn shift_level(layout: &KeyboardLayoutId) -> Vec<char> {
    LAYOUTS
        .iter()
        .find(|(id, _)| *id == layout.0)
        .and_then(|(_, text)| toml::from_str::<ShiftFile>(text).ok())
        .and_then(|f| f.shift_level)
        .map(|s| s.chars().collect())
        .unwrap_or_default()
}

#[derive(Deserialize)]
struct PresetFile {
    #[serde(default)]
    accepted_altgr_shadowing: Vec<Accepted>,
}

#[derive(Deserialize)]
struct Accepted {
    chord: String,
    layout: String,
    produces: String,
}

/// The AltGr collisions `preset` knowingly accepts (`[[accepted_altgr_shadowing]]` in its
/// file, F12), sorted like `altgr_collisions`. Empty for an unknown preset.
pub fn accepted_altgr_shadowing(preset: &str) -> Vec<Collision> {
    let Some(text) = preset_text(preset) else {
        return Vec::new();
    };
    let Ok(file) = toml::from_str::<PresetFile>(text) else {
        return Vec::new();
    };
    let mut out: Vec<Collision> = file
        .accepted_altgr_shadowing
        .into_iter()
        .filter_map(|a| {
            let mut it = a.produces.chars();
            let produces = it.next().filter(|_| it.next().is_none())?;
            Some(Collision {
                chord: a.chord,
                layout: a.layout,
                produces,
            })
        })
        .collect();
    out.sort_by(|a, b| (&a.layout, &a.chord, a.produces).cmp(&(&b.layout, &b.chord, b.produces)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacivat_ui_core::keymap::{
        ContextStack, KeyPress, Mods, NormKey, Resolution, altgr_collisions, parse_chord,
    };

    const ALL: [KeymapPlatform; 3] = [
        KeymapPlatform::MacOs,
        KeymapPlatform::Windows,
        KeymapPlatform::Linux,
    ];

    fn layout(id: &str) -> KeyboardLayoutId {
        KeyboardLayoutId(id.into())
    }

    #[test]
    fn shipped_data_loads_without_diagnostics() {
        for p in ALL {
            for preset in PRESETS {
                let (km, diags) = load(p, preset, None);
                assert!(diags.is_empty(), "{p:?} {preset}: {diags:?}");
                assert!(!km.bindings().is_empty());
            }
        }
    }

    #[test]
    fn unknown_preset_is_diagnosed() {
        let (_, diags) = load(KeymapPlatform::Windows, "emacs", None);
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn every_altgr_table_is_populated() {
        let tables = layouts_altgr();
        let ids: Vec<&str> = tables.0.iter().map(AltGrTable::layout).collect();
        assert_eq!(ids, vec!["tr-q", "tr-f", "de", "fr", "pl"]);
        for l in ["tr-q", "tr-f", "de", "fr", "pl"] {
            assert!(
                tables.get(l, '7').is_some() || tables.get(l, 'a').is_some(),
                "{l}"
            );
        }
    }

    #[test]
    fn m1_default_bindings_per_platform() {
        let cmd = |p| {
            let (km, _) = load(p, "hacivat", None);
            move |command: &str| -> Vec<String> {
                km.bindings()
                    .iter()
                    .filter(|b| b.command == command)
                    .map(|b| hacivat_ui_core::keymap::chord_string(&b.chord))
                    .collect()
            }
        };
        let mac = cmd(KeymapPlatform::MacOs);
        let win = cmd(KeymapPlatform::Windows);
        let lin = cmd(KeymapPlatform::Linux);
        assert_eq!(mac("workspace.command_palette"), ["cmd-k"]);
        assert_eq!(win("workspace.command_palette"), ["ctrl-k"]);
        assert_eq!(mac("workspace.open_settings"), ["cmd-,"]);
        assert_eq!(win("workspace.close_tab"), ["ctrl-w", "ctrl-f4"]);
        assert_eq!(lin("workspace.close_tab"), ["ctrl-w"]);
        assert_eq!(mac("workspace.next_tab"), ["ctrl-tab"]);
        assert_eq!(mac("workspace.prev_tab"), ["ctrl-shift-tab"]);
        assert_eq!(mac("workspace.toggle_explorer"), ["cmd-1", "shift-cmd-1"]);
        assert_eq!(win("workspace.toggle_explorer"), ["alt-1", "alt-shift-1"]);
        assert_eq!(mac("app.quit"), ["cmd-q"]);
        assert!(win("app.quit").is_empty());
        assert_eq!(lin("app.quit"), ["ctrl-q"]);
    }

    #[test]
    fn datagrip_preset_swaps_palette_and_settings() {
        let (km, _) = load(KeymapPlatform::Windows, "datagrip", None);
        let ws = ContextStack(vec!["Workspace"]);
        let press = |s: &str| -> KeyPress { parse_chord(s).unwrap()[0] };
        let us = layout("us");
        assert_eq!(
            km.resolve(&ws, &[], &press("ctrl-shift-a"), &us, false),
            Resolution::Command("workspace.command_palette".into())
        );
        assert_eq!(
            km.resolve(&ws, &[], &press("ctrl-k"), &us, false),
            Resolution::PassThrough
        );
        assert_eq!(
            km.resolve(&ws, &[], &press("ctrl-alt-s"), &us, false),
            Resolution::Command("workspace.open_settings".into())
        );
        let (mac, _) = load(KeymapPlatform::MacOs, "datagrip", None);
        assert_eq!(
            mac.resolve(&ws, &[], &press("cmd-shift-a"), &us, false),
            Resolution::Command("workspace.command_palette".into())
        );
    }

    #[test]
    fn french_alt_shift_1_opens_the_explorer_and_polish_altgr_s_types() {
        let (km, _) = load(KeymapPlatform::Windows, "datagrip", None);
        let ws = ContextStack(vec!["Workspace"]);
        let press = |s: &str| -> KeyPress { parse_chord(s).unwrap()[0] };
        assert_eq!(
            km.resolve(&ws, &[], &press("alt-shift-1"), &layout("fr"), false),
            Resolution::Command("workspace.toggle_explorer".into())
        );
        // The overlay chord belongs to FR only.
        assert_eq!(
            km.resolve(&ws, &[], &press("alt-shift-1"), &layout("us"), false),
            Resolution::PassThrough
        );
        // AltGr+S on PL arrives as ctrl+alt+s with the typed character: text, not a command.
        let altgr_s = KeyPress {
            mods: Mods {
                ctrl: true,
                alt: true,
                ..Mods::default()
            },
            key: NormKey::Char('s'),
            typed: Some('ś'),
        };
        assert_eq!(
            km.resolve(&ws, &[], &altgr_s, &layout("pl"), false),
            Resolution::PassThrough
        );
    }

    #[test]
    fn shift_level_reads_the_overlay_files() {
        assert!(shift_level(&layout("fr")).contains(&'1'));
        assert!(shift_level(&layout("tr-q")).contains(&'/'));
        assert!(shift_level(&layout("us")).is_empty());
    }

    #[test]
    fn macos_datagrip_has_no_collisions_to_accept() {
        let (km, _) = load(KeymapPlatform::MacOs, "datagrip", None);
        assert!(altgr_collisions(&km, &layouts_altgr()).is_empty());
    }
}
