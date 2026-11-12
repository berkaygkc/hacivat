//! Keymap model (UI-EDIT-13): types, plus parsing, normalization, resolution, AltGr and
//! reachability checks and display as submodules.

mod altgr;
#[cfg(test)]
mod behaviour_tests;
mod display;
mod normalize;
mod parse;
mod reach;
mod resolve;

pub use altgr::{AltGrTable, AltGrTables, Collision, altgr_collisions, option_only_printables};
pub use display::display_chord;
pub use normalize::{normalize, normalize_char};
pub use parse::{chord_string, parse_chord};
pub use reach::{layout_id_from_platform, unreachable_chords};

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Modifier state of a key press. `Default` is no modifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub cmd: bool,
}

/// A non-character key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NamedKey {
    Enter,
    Escape,
    Tab,
    Backspace,
    Delete,
    Insert,
    Space,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    /// A function key, `F(1)` = F1.
    F(u8),
}

/// A normalized key: a named key or a normalized character (NFC, `ı`/`İ`/`I` → `i`, ASCII
/// lower case; Task 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NormKey {
    Named(NamedKey),
    Char(char),
}

/// One key press of a chord.
///
/// `typed` is the character the OS would insert (`Keystroke::key_char`) when it differs from
/// `key`; it is `None` in parsed chords. Equality and hashing ignore it; resolution uses it to
/// detect AltGr text (Task 5).
#[derive(Debug, Clone, Copy)]
pub struct KeyPress {
    pub mods: Mods,
    pub key: NormKey,
    pub typed: Option<char>,
}

impl PartialEq for KeyPress {
    fn eq(&self, other: &Self) -> bool {
        self.mods == other.mods && self.key == other.key
    }
}

impl Eq for KeyPress {}

impl Hash for KeyPress {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.mods.hash(state);
        self.key.hash(state);
    }
}

/// Where a binding was loaded from, lowest priority first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindingSource {
    Default,
    Preset,
    Layout,
    User,
}

/// The keymap flavour (modifier conventions and default files).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeymapPlatform {
    MacOs,
    Windows,
    Linux,
}

/// A keyboard layout known to the keymap data: `"us" | "tr-q" | "tr-f" | "de" | "fr" | "pl"`
/// (F22: not `LayoutId`, which `gpui_surface` exports for taffy layout nodes).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyboardLayoutId(pub String);

/// Active key contexts, innermost last (for example `["Workspace", "Console", "Palette"]`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContextStack(pub Vec<&'static str>);

/// One chord bound to a command in a context.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Binding {
    pub chord: Vec<KeyPress>,
    pub command: String,
    pub context: String,
    /// Set for a layout-overlay alternative: the binding applies only on this layout.
    pub when_layout: Option<KeyboardLayoutId>,
    /// Whether the binding fires while an IME composition is in progress.
    pub during_composition: bool,
    pub source: BindingSource,
}

/// The outcome of resolving one key press.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Resolution {
    Command(String),
    /// The press is a prefix of a longer chord; wait for the next key.
    Pending,
    /// Not a binding: the press goes to the focused element (text input, AltGr text, IME).
    PassThrough,
}

/// A problem found while loading keymap sources.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KeymapDiag {
    DuplicateChord {
        chord: String,
        context: String,
    },
    Parse {
        source: BindingSource,
        line: u32,
        message: String,
    },
}

/// A chord or keymap file that cannot be parsed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeymapError {
    pub message: String,
}

impl std::fmt::Display for KeymapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for KeymapError {}

/// The loaded bindings for one platform.
///
/// Built by `Keymap::from_sources`; queried with `Keymap::resolve`, `Keymap::bindings` and
/// `Keymap::bindings_for`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    platform: KeymapPlatform,
    bindings: Vec<Binding>,
    /// First press of every binding's chord → indexes into `bindings` (the `(Mods, NormKey)`
    /// hash of UI-EDIT-13 step 4).
    index: HashMap<KeyPress, Vec<usize>>,
    /// Per layout: base-layer characters that need Shift (`shift_level` of the layout overlays).
    shift_levels: HashMap<KeyboardLayoutId, Vec<char>>,
}

impl Keymap {
    /// Every loaded binding, in load order (defaults, preset, layout overlays, user), F15.
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// The platform this keymap was loaded for.
    pub fn platform(&self) -> KeymapPlatform {
        self.platform
    }

    /// The bindings for `command` that apply on `layout`: the layout's overlay alternatives
    /// first (what the user must press on that layout), then the generic chords.
    pub fn bindings_for(&self, command: &str, layout: &KeyboardLayoutId) -> Vec<&Binding> {
        let of_command = || self.bindings.iter().filter(|b| b.command == command);
        let overlay = of_command().filter(|b| b.when_layout.as_ref() == Some(layout));
        let generic = of_command().filter(|b| b.when_layout.is_none());
        overlay.chain(generic).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn typed_char_is_ignored_by_equality_and_hashing() {
        let ctrl_alt = Mods {
            ctrl: true,
            alt: true,
            ..Mods::default()
        };
        let parsed = KeyPress {
            mods: ctrl_alt,
            key: NormKey::Char('q'),
            typed: None,
        };
        let pressed = KeyPress {
            typed: Some('@'),
            ..parsed
        };
        assert_eq!(parsed, pressed);
        let set: HashSet<KeyPress> = [parsed, pressed].into_iter().collect();
        assert_eq!(set.len(), 1);
        assert_ne!(
            parsed,
            KeyPress {
                mods: Mods::default(),
                ..parsed
            }
        );
    }

    #[test]
    fn mods_default_is_no_modifier() {
        assert_eq!(
            Mods::default(),
            Mods {
                ctrl: false,
                alt: false,
                shift: false,
                cmd: false
            }
        );
    }
}
