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
    #[serde(default)]
    dead: HashMap<String, String>,
    #[serde(default)]
    shift_keys: HashMap<String, String>,
    #[serde(default)]
    shift_dead: HashMap<String, String>,
}

/// The AltGr layers of one layout: the characters each key types with AltGr held, and with
/// Shift+AltGr held. A dead key counts as typing its accent.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AltGrTable {
    layout: String,
    altgr: HashMap<char, Vec<char>>,
    shift_altgr: HashMap<char, Vec<char>>,
}

fn single_char(s: &str) -> Option<char> {
    let mut it = s.chars();
    match (it.next(), it.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

/// Folds `key = "chars"` tables into one layer. Several keys can normalize to the same name
/// (Turkish `ı` and `i`), so a name maps to every character its keys type, in file order,
/// sorted for determinism.
fn layer(tables: [&HashMap<String, String>; 2]) -> HashMap<char, Vec<char>> {
    let mut out: HashMap<char, Vec<char>> = HashMap::new();
    for table in tables {
        for (k, v) in table {
            let Some(key) = single_char(k).map(normalize_char) else {
                continue;
            };
            let chars = out.entry(key).or_default();
            for c in v.chars() {
                if !chars.contains(&c) {
                    chars.push(c);
                }
            }
        }
    }
    for chars in out.values_mut() {
        chars.sort_unstable();
    }
    out
}

impl AltGrTable {
    /// Parses a table file: `layout = "pl"`, and the tables `[keys]`, `[dead]` (value: the
    /// accent), `[shift_keys]` and `[shift_dead]`, each of `key = "chars"` pairs. The data is
    /// embedded and tested; a file that does not parse gives an empty table.
    pub fn parse(toml: &str) -> AltGrTable {
        let file: TableFile = ::toml::from_str(toml).unwrap_or_default();
        AltGrTable {
            layout: file.layout,
            altgr: layer([&file.keys, &file.dead]),
            shift_altgr: layer([&file.shift_keys, &file.shift_dead]),
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
        self.get_all(layout, key, false).first().copied()
    }

    /// Every character AltGr (or Shift+AltGr when `shift`) plus `key` types on `layout`.
    /// More than one only when several physical keys share a normalized name.
    pub fn get_all(&self, layout: &str, key: char, shift: bool) -> &[char] {
        let key = normalize_char(key);
        self.0
            .iter()
            .find(|t| t.layout == layout)
            .and_then(|t| {
                let layer = if shift { &t.shift_altgr } else { &t.altgr };
                layer.get(&key)
            })
            .map_or(&[], Vec::as_slice)
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
                for &produces in tables.get_all(&table.layout, c, press.mods.shift) {
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
