//! Per-layout reachability (F27) and platform layout-ID mapping.

use super::parse::chord_string;
use super::{KeyboardLayoutId, Keymap, NormKey};

/// Chords that a user on `layout` cannot press as written.
///
/// A chord is unreachable when its key character needs Shift on that layout (`shift_level`),
/// the chord has no Shift, and the keymap has no overlay alternative for the same command in
/// the same context on that layout. The result is canonical chord text, in binding order.
pub fn unreachable_chords(
    km: &Keymap,
    layout: &KeyboardLayoutId,
    shift_level: &[char],
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for b in km.bindings().iter().filter(|b| b.when_layout.is_none()) {
        let needs_shift = b.chord.iter().any(|p| {
            !p.mods.shift && matches!(p.key, NormKey::Char(c) if shift_level.contains(&c))
        });
        if !needs_shift {
            continue;
        }
        let has_overlay = km.bindings().iter().any(|o| {
            o.when_layout.as_ref() == Some(layout)
                && o.command == b.command
                && o.context == b.context
        });
        let s = chord_string(&b.chord);
        if !has_overlay && !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// Maps a platform keyboard layout ID to a keymap layout ID (`"us" | "tr-q" | "tr-f" | "de" |
/// "fr" | "pl"`); anything unknown is `"us"`.
///
/// Accepts macOS input source IDs (`com.apple.keylayout.Turkish`), Windows KLIDs
/// (`0000041F` is Turkish Q, `0001041F` Turkish F) and Linux XKB names (`tr`, `tr(f)`).
pub fn layout_id_from_platform(id: &str) -> KeyboardLayoutId {
    let id = id.trim().to_ascii_lowercase();
    let known = if let Some(name) = id.strip_prefix("com.apple.keylayout.") {
        mac_layout(name)
    } else if id.len() == 8 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
        windows_layout(&id)
    } else {
        xkb_layout(&id)
    };
    KeyboardLayoutId(known.unwrap_or("us").to_string())
}

fn mac_layout(name: &str) -> Option<&'static str> {
    if name.starts_with("turkish") {
        // "Turkish" and "Turkish-QWERTY(-PC)" are Q; "Turkish-F" and "Turkish-Standard" are F.
        return Some(if name.ends_with("-f") || name.ends_with("standard") {
            "tr-f"
        } else {
            "tr-q"
        });
    }
    match name {
        "german" | "german-din-2137" => Some("de"),
        "french" | "french-pc" => Some("fr"),
        "polish" | "polishpro" => Some("pl"),
        _ => None,
    }
}

fn windows_layout(klid: &str) -> Option<&'static str> {
    match klid {
        "0000041f" => Some("tr-q"),
        "0001041f" => Some("tr-f"),
        "00000407" => Some("de"),
        "0000040c" => Some("fr"),
        "00000415" => Some("pl"),
        _ => None,
    }
}

fn xkb_layout(name: &str) -> Option<&'static str> {
    let (base, variant) = match name.split_once(['(', '+', ':']) {
        Some((b, v)) => (b, v.trim_end_matches(')')),
        None => (name, ""),
    };
    match (base, variant) {
        ("tr", "f") => Some("tr-f"),
        ("tr", _) => Some("tr-q"),
        ("de", _) => Some("de"),
        ("fr", _) => Some("fr"),
        ("pl", _) => Some("pl"),
        _ => None,
    }
}
