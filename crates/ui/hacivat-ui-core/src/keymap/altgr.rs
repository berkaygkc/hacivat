//! AltGr tables and the reserved-combination checks (UI-EDIT-13 step 5).

use super::normalize::normalize_char;
use super::parse::chord_string;
use super::{Keymap, KeymapPlatform, NormKey};
use serde::Deserialize;
use std::collections::HashMap;

/// A default chord that produces a character with AltGr on some layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collision {
    /// Canonical chord text (`"ctrl-alt-s"`).
    pub chord: String,
    /// The layout id (`"pl"`).
    pub layout: String,
    /// The character AltGr plus the chord's key types on that layout.
    pub produces: char,
}

#[derive(Deserialize, Default)]
struct TableFile {
    #[serde(default)]
    layout: String,
    #[serde(default)]
    keys: HashMap<String, String>,
}

/// The AltGr layer of one layout: the character each key types with AltGr held.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AltGrTable {
    layout: String,
    keys: HashMap<char, char>,
}

fn single_char(s: &str) -> Option<char> {
    let mut it = s.chars();
    match (it.next(), it.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

impl AltGrTable {
    /// Parses a table file: `layout = "pl"` and a `[keys]` table of `key = "char"` pairs. The
    /// data is embedded and tested; a file that does not parse gives an empty table.
    pub fn parse(toml: &str) -> AltGrTable {
        let file: TableFile = ::toml::from_str(toml).unwrap_or_default();
        let keys = file
            .keys
            .iter()
            .filter_map(|(k, v)| Some((normalize_char(single_char(k)?), single_char(v)?)))
            .collect();
        AltGrTable {
            layout: file.layout,
            keys,
        }
    }

    /// The layout id this table describes.
    pub fn layout(&self) -> &str {
        &self.layout
    }
}

/// The AltGr tables of every supported layout.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AltGrTables(pub Vec<AltGrTable>);

impl AltGrTables {
    /// The character AltGr plus `key` types on `layout`, if any.
    pub fn get(&self, layout: &str, key: char) -> Option<char> {
        let key = normalize_char(key);
        self.0
            .iter()
            .find(|t| t.layout == layout)
            .and_then(|t| t.keys.get(&key).copied())
    }
}

/// Every Ctrl+Alt chord in `km` whose key types a character with AltGr on some layout.
///
/// On Windows and Linux AltGr is Ctrl+Alt, so such a chord would steal the character. Named
/// keys never collide. A binding limited to one layout collides only there. The result is
/// sorted by layout, then chord, and has no duplicates; macOS has no AltGr and returns none.
pub fn altgr_collisions(km: &Keymap, tables: &AltGrTables) -> Vec<Collision> {
    if km.platform() == KeymapPlatform::MacOs {
        return Vec::new();
    }
    let mut found: Vec<Collision> = Vec::new();
    for b in km.bindings() {
        for press in b.chord.iter().filter(|p| p.mods.ctrl && p.mods.alt) {
            let NormKey::Char(c) = press.key else {
                continue;
            };
            for table in &tables.0 {
                if b.when_layout.as_ref().is_some_and(|l| l.0 != table.layout) {
                    continue;
                }
                if let Some(produces) = tables.get(&table.layout, c) {
                    found.push(Collision {
                        chord: chord_string(&b.chord),
                        layout: table.layout.clone(),
                        produces,
                    });
                }
            }
        }
    }
    found.sort_by(|a, b| (&a.layout, &a.chord, a.produces).cmp(&(&b.layout, &b.chord, b.produces)));
    found.dedup();
    found
}

/// macOS chords that use Option (or Option+Shift) on a printable key without Cmd or Ctrl.
///
/// Option types characters and dead keys on every Mac layout, so such a chord steals text.
/// Other platforms have no such rule and return none.
pub fn option_only_printables(km: &Keymap) -> Vec<String> {
    if km.platform() != KeymapPlatform::MacOs {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    for b in km.bindings() {
        let bare_option = b.chord.iter().any(|p| {
            p.mods.alt && !p.mods.ctrl && !p.mods.cmd && matches!(p.key, NormKey::Char(_))
        });
        if bare_option {
            let s = chord_string(&b.chord);
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out
}
