//! The application menu model, built from the action registry: macOS `set_menus` and the
//! Windows/Linux in-window menu (F14). Task 16.
//!
//! One model feeds both menus, so they list the same actions in the same order with the same
//! labels. macOS items carry `RunCommand { id }` (F1); their chords are bound as GPUI key
//! bindings under the context [`MENU_KEY_CONTEXT`], which no element has, so they only label
//! the native items while Hacivat's dispatcher keeps handling the keys (R7). No menu binds the
//! clipboard chords, so Cmd/Ctrl-A/C/X/V stay with the focused text input. While a modal layer
//! is open, every item but Quit carries `InertCommand` instead (greyed, its key equivalent
//! inert); the workspace rebuilds the menus when the first modal layer opens and the last one
//! closes (P47).

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Mutex;

use hacivat_l10n::ids;
use hacivat_ui::actions::ActionRegistry;
use hacivat_ui::menu::{MENU_MIN_WIDTH_PX, MenuItemSpec};
use hacivat_ui::text::TextFn;
use hacivat_ui_core::fonts::font_for;
use hacivat_ui_core::gpui_surface::{
    Action, App, InertCommand, KeyBinding, Keystroke, Menu, MenuItem, Pixels, Point, RunCommand,
    SharedString, TextRun, Window, point, px,
};
use hacivat_ui_core::keymap::chord_string;
use hacivat_ui_core::theme::{Theme, TypeStyle};

use crate::commands::{QUIT_ACTION, modal_open};
use crate::l10n_global::{text_fn_id, tr_id};
use crate::shell::ActiveKeymap;
use crate::shell::keymap_state::shortcut_label;

/// The GPUI key context of the menu key equivalents (never on an element).
pub const MENU_KEY_CONTEXT: &str = "HacivatMenu";

/// One row of a menu group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuEntry {
    Action(&'static str),
    Separator,
}

/// One top-level menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuGroup {
    pub title_id: &'static str,
    pub items: Vec<MenuEntry>,
}

/// The menus' layout by action ID; actions nobody registered are left out, and so is a group
/// left without items (the File group until Task 18 registers `connection.new`; the Edit group
/// until the console registers the editor commands, M3a Task 32).
fn layout() -> [(&'static str, Vec<MenuEntry>); 6] {
    use MenuEntry::{Action as A, Separator as S};
    [
        (
            ids::hv::MENU_APP.id,
            vec![
                A("app.about"),
                S,
                A("workspace.open_settings"),
                S,
                A("app.restart"),
                A("app.quit"),
            ],
        ),
        // M2a (Task 1, P18).
        (ids::hv::MENU_FILE.id, vec![A("connection.new")]),
        // ## m3a-t01b-seams: the editor's Edit group.
        (
            ids::hv::MENU_EDIT.id,
            vec![
                A("editor.undo"),
                A("editor.redo"),
                S,
                A("editor.cut"),
                A("editor.copy"),
                A("editor.paste"),
                S,
                A("editor.select_all"),
            ],
        ),
        (
            ids::hv::MENU_VIEW.id,
            vec![
                A("workspace.command_palette"),
                S,
                A("workspace.toggle_explorer"),
                A("explorer.focus_filter"),
                S,
                A("workspace.toggle_theme"),
                A("workspace.next_language"),
                A("workspace.reload_translations"),
            ],
        ),
        (
            ids::hv::MENU_WINDOW.id,
            vec![
                A("workspace.new_console_tab"),
                A("workspace.close_tab"),
                S,
                A("workspace.next_tab"),
                A("workspace.prev_tab"),
                S,
                A("workspace.split_right"),
                A("workspace.split_down"),
                A("workspace.focus_next_pane"),
            ],
        ),
        (ids::hv::MENU_HELP.id, vec![A("workspace.command_palette")]),
    ]
}

/// The menu model for `reg`: the groups (`hv-menu-app`, `-file`, `-edit`, `-view`, `-window`, `-help`)
/// of the registered actions, without leading, trailing or doubled separators. A group with no
/// registered action is left out.
pub fn menu_model(reg: &ActionRegistry) -> Vec<MenuGroup> {
    layout()
        .into_iter()
        .filter_map(|(title_id, entries)| {
            let mut items: Vec<MenuEntry> = Vec::new();
            for entry in entries {
                match entry {
                    MenuEntry::Action(id) if reg.get(id).is_some() => items.push(entry),
                    MenuEntry::Action(_) => {}
                    MenuEntry::Separator => {
                        if items.last().is_some_and(|l| *l != MenuEntry::Separator) {
                            items.push(entry);
                        }
                    }
                }
            }
            if items.last() == Some(&MenuEntry::Separator) {
                items.pop();
            }
            (!items.is_empty()).then_some(MenuGroup { title_id, items })
        })
        .collect()
}

/// The registered action `id`: its title message and whether it is enabled now.
fn action(id: &str, cx: &App) -> Option<(&'static str, bool)> {
    let def = cx.try_global::<ActionRegistry>()?.get(id)?;
    Some((def.title_id, (def.enabled)(cx)))
}

/// Whether the menu item of `id` is held back now: a modal layer is open and `id` is not
/// `app.quit` (P47).
fn held(id: &str, modal: bool) -> bool {
    modal && id != QUIT_ACTION
}

/// The macOS menu bar for `model`, in the current language. While a modal layer is open every
/// item but Quit carries `InertCommand`, which nothing handles, so macOS greys it (its menu
/// validation overrides `disabled`) and its key equivalent does nothing (P47).
pub fn build_menus(model: &[MenuGroup], cx: &App) -> Vec<Menu> {
    let modal = modal_open(cx);
    model
        .iter()
        .map(|group| Menu {
            name: tr_id(cx, group.title_id),
            items: group
                .items
                .iter()
                .filter_map(|entry| match *entry {
                    MenuEntry::Separator => Some(MenuItem::Separator),
                    MenuEntry::Action(id) => {
                        let (title_id, enabled) = action(id, cx)?;
                        let held = held(id, modal);
                        let action: Box<dyn Action> = if held {
                            Box::new(InertCommand { id: id.into() })
                        } else {
                            Box::new(RunCommand { id: id.into() })
                        };
                        Some(MenuItem::Action {
                            name: tr_id(cx, title_id),
                            action,
                            os_action: None,
                            checked: false,
                            disabled: !enabled || held,
                        })
                    }
                })
                .collect(),
            disabled: false,
        })
        .collect()
}

/// The Windows/Linux in-window menu for `model`: the groups one after another, separated, with
/// labels and keys re-read on every render.
pub fn popup_items(model: &[MenuGroup], cx: &App) -> Vec<MenuItemSpec> {
    let mut out: Vec<MenuItemSpec> = Vec::new();
    for group in model {
        if !out.is_empty() && !group.items.is_empty() {
            out.push(MenuItemSpec::separator());
        }
        for entry in &group.items {
            match *entry {
                MenuEntry::Separator => out.push(MenuItemSpec::separator()),
                MenuEntry::Action(id) => {
                    let Some((title_id, enabled)) = action(id, cx) else {
                        continue;
                    };
                    let mut item = MenuItemSpec::new(text_fn_id(title_id), id).disabled(!enabled);
                    if shortcut_label(cx, id).is_some() {
                        let keys: TextFn = Rc::new(move |cx: &App| {
                            shortcut_label(cx, id).unwrap_or_default().into()
                        });
                        item = item.keys(keys);
                    }
                    out.push(item);
                }
            }
        }
    }
    out
}

/// Room for an access key shown after a label, `(X)`, when the label has no usable letter.
const ACCESS_KEY_SUFFIX_PX: f32 = 24.0;

/// How wide the in-window menu of `items` draws: its widest row (label, gap, keys and the row
/// padding), the surface padding and borders, at least `MENU_MIN_WIDTH_PX`.
fn popup_width(items: &[MenuItemSpec], window: &Window, cx: &App) -> f32 {
    let t = Theme::get(cx).tokens;
    let ink = t.colors.text.to_hsla();
    let measure = |text: SharedString, style: &TypeStyle| -> f32 {
        let run = TextRun {
            len: text.len(),
            font: font_for(style),
            color: ink,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = window
            .text_system()
            .shape_line(text, px(style.size_px), &[run], None);
        f32::from(line.width)
    };
    let widest = items
        .iter()
        .filter(|i| !i.separator)
        .map(|i| {
            let label = measure((i.label)(cx), &t.type_ramp.body);
            let keys = i
                .keys
                .as_ref()
                .map(|k| k(cx))
                .filter(|k| !k.is_empty())
                .map_or(0.0, |k| t.spacing.xl + measure(k, &t.type_ramp.mono_kbd));
            label + ACCESS_KEY_SUFFIX_PX + keys + 2.0 * t.spacing.lg
        })
        .fold(0.0, f32::max);
    // Surface padding (xs on each side) and its 1 px border.
    (widest + 2.0 * t.spacing.xs + 2.0).max(MENU_MIN_WIDTH_PX)
}

/// Where the in-window menu of `items` opens: under the title bar, its right edge `sm` inside
/// the window's right edge (the app-menu button is at the right end of the title bar), clamped
/// so the whole menu stays inside the window.
pub fn in_window_menu_anchor(items: &[MenuItemSpec], window: &Window, cx: &App) -> Point<Pixels> {
    let t = Theme::get(cx).tokens;
    let width = popup_width(items, window, cx);
    let x = (window.viewport_size().width - px(width + t.spacing.sm)).max(px(0.0));
    point(x, px(t.spacing.titlebar_height))
}

/// Interns chord texts: GPUI's `KeyBinding::new` takes `&'static str`, and the set of chords in
/// the keymap data is small and fixed.
fn intern(chord: String) -> &'static str {
    static CHORDS: Mutex<Option<HashMap<String, &'static str>>> = Mutex::new(None);
    let mut map = CHORDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let map = map.get_or_insert_with(HashMap::new);
    if let Some(found) = map.get(&chord) {
        return found;
    }
    let leaked: &'static str = Box::leak(chord.clone().into_boxed_str());
    map.insert(chord, leaked);
    leaked
}

// ## m3a-t01b-seams
/// The Edit group's clipboard and select-all items get no key equivalent: Cmd/Ctrl-A/C/X/V
/// stay with whatever has the focus (a text input or the editor, through the dispatcher).
const NO_KEY_EQUIVALENT: [&str; 4] = [
    "editor.cut",
    "editor.copy",
    "editor.paste",
    "editor.select_all",
];

/// The GPUI bindings that label the menu items with their chords on the active keymap; an item
/// held back under a modal layer is labelled through its `InertCommand`.
fn menu_key_bindings(model: &[MenuGroup], cx: &App) -> Vec<KeyBinding> {
    let Some(active) = cx.try_global::<ActiveKeymap>() else {
        return Vec::new();
    };
    let modal = modal_open(cx);
    let mut seen = Vec::new();
    model
        .iter()
        .flat_map(|g| g.items.iter())
        .filter_map(|entry| match *entry {
            MenuEntry::Action(id) => Some(id),
            MenuEntry::Separator => None,
        })
        .filter(|id| {
            let fresh = !seen.contains(id);
            seen.push(*id);
            fresh && !NO_KEY_EQUIVALENT.contains(id)
        })
        .filter_map(|id| {
            let binding = active
                .keymap
                .bindings_for(id, &active.layout)
                .into_iter()
                .next()?;
            let chord = intern(chord_string(&binding.chord));
            // Only chords GPUI can parse become key equivalents.
            if chord.split(' ').any(|k| Keystroke::parse(k).is_err()) {
                return None;
            }
            let id: SharedString = id.into();
            Some(if held(&id, modal) {
                KeyBinding::new(chord, InertCommand { id }, Some(MENU_KEY_CONTEXT))
            } else {
                KeyBinding::new(chord, RunCommand { id }, Some(MENU_KEY_CONTEXT))
            })
        })
        .collect()
}

/// Rebuilds the menus after a language or keymap change (F15): on macOS the menu bar and its
/// key equivalents; elsewhere no menu, since the in-window menu reads its labels when it
/// renders.
///
/// GPUI `KeyBinding`s are owned by this function: on every platform it clears every binding,
/// then on macOS binds only the menu key equivalents, under [`MENU_KEY_CONTEXT`]. Widgets and
/// later tasks must not call `bind_keys`; keys go through Hacivat's dispatcher (the key-capture
/// hook and the keymap data, R7).
pub fn rebuild(cx: &mut App) {
    cx.clear_key_bindings();
    if !cfg!(target_os = "macos") {
        return;
    }
    let Some(reg) = cx.try_global::<ActionRegistry>() else {
        return;
    };
    let model = menu_model(reg);
    let bindings = menu_key_bindings(&model, cx);
    let menus = build_menus(&model, cx);
    cx.bind_keys(bindings);
    cx.set_menus(menus);
}
