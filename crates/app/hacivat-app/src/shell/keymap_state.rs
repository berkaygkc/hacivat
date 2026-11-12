//! The active keymap (F11): loaded at startup for the platform, the preset in settings, the
//! keyboard layout and the user's keymap files, and reloaded when the layout or the preset
//! changes. Task 16's dispatcher follows it; Task 17 calls [`set_keymap_preset`].

use std::sync::Arc;

use hacivat_settings::KeymapPreset;
use hacivat_ui::keymaps::{self, layouts_altgr};
use hacivat_ui_core::gpui_surface::{App, Global};
use hacivat_ui_core::keymap::{
    Collision, KeyboardLayoutId, Keymap, KeymapPlatform, altgr_collisions, layout_id_from_platform,
};

use crate::shell::AppState;

/// The keymap in effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveKeymap {
    pub preset: KeymapPreset,
    pub platform: KeymapPlatform,
    pub layout: KeyboardLayoutId,
    pub keymap: Arc<Keymap>,
}

impl Global for ActiveKeymap {}

/// The user's keymap text (`<config>/keymaps/*.toml`), kept for reloads.
#[derive(Debug, Clone, Default)]
struct UserKeymap(Option<String>);

impl Global for UserKeymap {}

/// Set once the keyboard-layout observer is installed.
struct LayoutObserved;

impl Global for LayoutObserved {}

/// The keymap flavour of the OS this build runs on.
pub fn current_platform() -> KeymapPlatform {
    if cfg!(target_os = "macos") {
        KeymapPlatform::MacOs
    } else if cfg!(target_os = "windows") {
        KeymapPlatform::Windows
    } else {
        KeymapPlatform::Linux
    }
}

/// The keymap data's name for `preset`.
pub fn preset_name(preset: KeymapPreset) -> &'static str {
    match preset {
        KeymapPreset::Hacivat => keymaps::PRESETS[0],
        KeymapPreset::Datagrip => keymaps::PRESETS[1],
    }
}

/// The keymap layout of the OS keyboard layout in use (`us` when unknown).
pub fn current_layout(cx: &App) -> KeyboardLayoutId {
    layout_id_from_platform(cx.keyboard_layout().id())
}

/// Every `*.toml` file of `<config>/keymaps`, in file-name order, as one keymap text.
fn read_user_keymaps(cx: &App) -> Option<String> {
    let dir = cx.try_global::<AppState>()?.paths.keymaps_dir();
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();
    let text: Vec<String> = files
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect();
    (!text.is_empty()).then(|| text.join("\n"))
}

fn load(
    platform: KeymapPlatform,
    preset: KeymapPreset,
    layout: KeyboardLayoutId,
    user: Option<&str>,
) -> ActiveKeymap {
    // Diagnostics (a bad user file, duplicate chords) leave the rest of the keymap usable;
    // reporting them belongs to the keymap UI (v1.x).
    let (keymap, _diags) = keymaps::load(platform, preset_name(preset), user);
    ActiveKeymap {
        preset,
        platform,
        layout,
        keymap: Arc::new(keymap),
    }
}

/// Loads the keymap for this OS, the preset in `AppState`'s settings, the current keyboard
/// layout and the user's keymap files, and follows keyboard-layout changes from then on.
pub fn install(cx: &mut App) {
    let preset = cx
        .try_global::<AppState>()
        .map_or_else(KeymapPreset::default, |s| s.settings.keymap.preset);
    let user = read_user_keymaps(cx);
    let layout = current_layout(cx);
    cx.set_global(UserKeymap(user.clone()));
    cx.set_global(load(current_platform(), preset, layout, user.as_deref()));
    if !cx.has_global::<LayoutObserved>() {
        cx.set_global(LayoutObserved);
        cx.on_keyboard_layout_change(|cx| {
            let layout = current_layout(cx);
            let Some(active) = cx.try_global::<ActiveKeymap>() else {
                return;
            };
            if active.layout == layout {
                return;
            }
            let (platform, preset) = (active.platform, active.preset);
            let user = cx.try_global::<UserKeymap>().and_then(|u| u.0.clone());
            cx.set_global(load(platform, preset, layout, user.as_deref()));
            cx.refresh_windows();
        })
        .detach();
    }
}

/// Installs the keymap for `platform` and `layout` with the `user` keymap text, keeping the
/// current preset (tests, and the settings preview of another platform).
pub fn install_with(
    cx: &mut App,
    platform: KeymapPlatform,
    layout: KeyboardLayoutId,
    user: Option<String>,
) {
    let preset = cx
        .try_global::<ActiveKeymap>()
        .map_or_else(KeymapPreset::default, |a| a.preset);
    cx.set_global(load(platform, preset, layout, user.as_deref()));
    cx.set_global(UserKeymap(user));
    cx.refresh_windows();
}

/// Switches to `preset` (keeping platform, layout and user keymap) and returns the chords that
/// type a character with AltGr on the current layout, so the caller can warn about them.
pub fn set_keymap_preset(cx: &mut App, preset: KeymapPreset) -> Vec<Collision> {
    let (platform, layout) = match cx.try_global::<ActiveKeymap>() {
        Some(a) => (a.platform, a.layout.clone()),
        None => (current_platform(), current_layout(cx)),
    };
    let user = cx.try_global::<UserKeymap>().and_then(|u| u.0.clone());
    let active = load(platform, preset, layout.clone(), user.as_deref());
    let collisions = altgr_collisions(&active.keymap, &layouts_altgr())
        .into_iter()
        .filter(|c| c.layout == layout.0)
        .collect();
    cx.set_global(active);
    cx.refresh_windows();
    collisions
}

/// The shortcut of `command` on the active keymap and layout, as the platform shows it.
pub fn shortcut_label(cx: &App, command: &str) -> Option<String> {
    let active = cx.try_global::<ActiveKeymap>()?;
    let binding = active
        .keymap
        .bindings_for(command, &active.layout)
        .into_iter()
        .next()?;
    Some(hacivat_ui_core::keymap::display_chord(
        &binding.chord,
        active.platform,
    ))
}
