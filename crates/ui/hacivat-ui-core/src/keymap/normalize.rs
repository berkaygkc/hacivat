//! Key normalization for matching (UI-EDIT-13 step 2).

use super::{KeyPress, Mods, NamedKey, NormKey};
use crate::gpui_surface::Keystroke;
use icu_normalizer::ComposingNormalizerBorrowed;

/// Normalizes a key character for matching: NFC, then `ı`/`İ`/`I` → `i`, then ASCII lower case.
///
/// The Turkish `ı` and `İ` fold onto `i` so a binding on `ctrl-i` matches the key that prints
/// `ı` on a Turkish Q keyboard. This is machine matching, not human text casing.
pub fn normalize_char(c: char) -> char {
    let mut buf = [0u8; 4];
    let composed = ComposingNormalizerBorrowed::new_nfc().normalize(c.encode_utf8(&mut buf));
    let mut chars = composed.chars();
    let c = match (chars.next(), chars.next()) {
        (Some(only), None) => only,
        _ => c,
    };
    match c {
        'ı' | 'İ' | 'I' => 'i',
        other => other.to_ascii_lowercase(),
    }
}

/// The named key for a GPUI key name (`"enter"`, `"f4"`, ...), if it is one.
pub(super) fn named_key(name: &str) -> Option<NamedKey> {
    Some(match name {
        "enter" => NamedKey::Enter,
        "escape" | "esc" => NamedKey::Escape,
        "tab" => NamedKey::Tab,
        "backspace" => NamedKey::Backspace,
        "delete" => NamedKey::Delete,
        "insert" => NamedKey::Insert,
        "space" => NamedKey::Space,
        "up" => NamedKey::Up,
        "down" => NamedKey::Down,
        "left" => NamedKey::Left,
        "right" => NamedKey::Right,
        "home" => NamedKey::Home,
        "end" => NamedKey::End,
        "pageup" => NamedKey::PageUp,
        "pagedown" => NamedKey::PageDown,
        _ => {
            let n: u8 = name.strip_prefix('f')?.parse().ok()?;
            if !(1..=24).contains(&n) || name.starts_with("f0") {
                return None;
            }
            NamedKey::F(n)
        }
    })
}

/// Normalizes a GPUI keystroke into a [`KeyPress`], or `None` for a dead key.
///
/// A dead key has no `key_char` and is not a named key: it never matches a binding. GPUI also
/// reports `key_char: None` for chords such as `cmd-k`, which are not dead keys, so a missing
/// `key_char` only means "dead" while no Ctrl, Alt or Cmd is held. `typed` is the `key_char`
/// when it differs from the (normalized) key.
pub fn normalize(k: &Keystroke) -> Option<KeyPress> {
    let mods = Mods {
        ctrl: k.modifiers.control,
        alt: k.modifiers.alt,
        shift: k.modifiers.shift,
        cmd: k.modifiers.platform,
    };
    if let Some(named) = named_key(&k.key) {
        return Some(KeyPress {
            mods,
            key: NormKey::Named(named),
            typed: None,
        });
    }
    let mut key_chars = k.key.chars();
    let (Some(raw), None) = (key_chars.next(), key_chars.next()) else {
        return None;
    };
    let char_mods = mods.ctrl || mods.alt || mods.cmd;
    if k.key_char.is_none() && !char_mods {
        return None;
    }
    let key = normalize_char(raw);
    let typed = k
        .key_char
        .as_deref()
        .and_then(|s| {
            let mut it = s.chars();
            match (it.next(), it.next()) {
                (Some(c), None) => Some(c),
                _ => None,
            }
        })
        .filter(|c| *c != raw && normalize_char(*c) != key);
    Some(KeyPress {
        mods,
        key: NormKey::Char(key),
        typed,
    })
}
