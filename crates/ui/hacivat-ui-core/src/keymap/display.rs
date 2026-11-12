//! Shortcut labels (UI-EDIT-13 step 6).

use super::{KeyPress, KeymapPlatform, NamedKey, NormKey};

fn named_label(n: NamedKey, mac: bool) -> String {
    let (glyph, word) = match n {
        NamedKey::Enter => ("↩", "Enter"),
        NamedKey::Escape => ("⎋", "Esc"),
        NamedKey::Tab => ("⇥", "Tab"),
        NamedKey::Backspace => ("⌫", "Backspace"),
        NamedKey::Delete => ("⌦", "Delete"),
        NamedKey::Insert => ("Insert", "Insert"),
        NamedKey::Space => ("Space", "Space"),
        NamedKey::Up => ("↑", "Up"),
        NamedKey::Down => ("↓", "Down"),
        NamedKey::Left => ("←", "Left"),
        NamedKey::Right => ("→", "Right"),
        NamedKey::Home => ("↖", "Home"),
        NamedKey::End => ("↘", "End"),
        NamedKey::PageUp => ("⇞", "PageUp"),
        NamedKey::PageDown => ("⇟", "PageDown"),
        NamedKey::F(n) => return format!("F{n}"),
    };
    if mac { glyph } else { word }.to_string()
}

fn key_label(key: NormKey, mac: bool) -> String {
    match key {
        NormKey::Char(c) => c.to_ascii_uppercase().to_string(),
        NormKey::Named(n) => named_label(n, mac),
    }
}

/// The label of a chord for display: macOS uses the glyphs `⌃⌥⇧⌘` followed by the key
/// (`⇧⌘P`); Windows and Linux use English modifier names joined with `+` (`Ctrl+Shift+7`),
/// as printed on keyboards. Presses of a multi-key chord are separated by a space.
pub fn display_chord(chord: &[KeyPress], platform: KeymapPlatform) -> String {
    let mac = platform == KeymapPlatform::MacOs;
    let cmd_name = if platform == KeymapPlatform::Windows {
        "Win"
    } else {
        "Super"
    };
    let presses: Vec<String> = chord
        .iter()
        .map(|p| {
            let key = key_label(p.key, mac);
            if mac {
                let mut s = String::new();
                for (on, glyph) in [
                    (p.mods.ctrl, '⌃'),
                    (p.mods.alt, '⌥'),
                    (p.mods.shift, '⇧'),
                    (p.mods.cmd, '⌘'),
                ] {
                    if on {
                        s.push(glyph);
                    }
                }
                s + &key
            } else {
                let mut parts: Vec<&str> = Vec::new();
                for (on, name) in [
                    (p.mods.ctrl, "Ctrl"),
                    (p.mods.alt, "Alt"),
                    (p.mods.shift, "Shift"),
                    (p.mods.cmd, cmd_name),
                ] {
                    if on {
                        parts.push(name);
                    }
                }
                parts.push(&key);
                parts.join("+")
            }
        })
        .collect();
    presses.join(" ")
}
