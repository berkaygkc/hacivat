// ## m4b-t01a-seams
//! Logical-database and server database names (HV-SCRIPT-01 step 3, HV-SCRIPT-02).
//!
//! ## m4b-t01a-seams
//! Task 1a: the types.
//!
//! ## m4b-t03-registry
//! Task 3: the character rules of [`LogicalDb::parse`] (1–128 bytes of NFC text, no C0/C1
//! control, no bidi control) and the binding comparison [`same_db`] (NFC, then ASCII
//! case-insensitive; never locale or Unicode casing, so `İ` ≠ `I` and `ı` ≠ `i`).

use std::fmt;
use std::sync::Arc;

use icu_normalizer::ComposingNormalizerBorrowed;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The longest logical name, in bytes of its NFC text (HV-SCRIPT-01 step 3).
pub const MAX_NAME_BYTES: usize = 128;

/// The logical database a library binds (`database` or an alias in `workspace.toml`, or the
/// folder name): NFC, original case kept for display. Binding compares two of these with
/// [`LogicalDb::same_db`] (NFC plus ASCII case-insensitive equality, Task 3), never with `==`.
///
/// ## m4b-t01a-fr1 (review M4): no `PartialEq`, `Eq`, `Hash` or `Ord`, so exact-text equality
/// cannot stand in for the binding comparison by accident.
#[derive(Debug, Clone)]
pub struct LogicalDb(pub(crate) Arc<str>);

/// Whether two logical-database names bind the same library (HV-SCRIPT-02 step 4: NFC, then
/// ASCII case-insensitive; `İ` ≠ `I`).
pub fn same_db(a: &str, b: &str) -> bool {
    let nfc = ComposingNormalizerBorrowed::new_nfc();
    nfc.normalize(a).eq_ignore_ascii_case(&nfc.normalize(b))
}

/// Why a name was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
    Empty,
    /// More than [`MAX_NAME_BYTES`] bytes of NFC text.
    TooLong,
    /// A C0 or C1 control character, or DEL.
    Control,
    /// A bidi control (U+061C, U+200E/F, U+202A–E, U+2066–9).
    Bidi,
}

impl LogicalDb {
    /// A validated logical name from untrusted text, stored as NFC.
    pub fn parse(text: &str) -> Result<Self, NameError> {
        if let Some(bad) = text.chars().find_map(refusal) {
            return Err(bad);
        }
        let nfc = ComposingNormalizerBorrowed::new_nfc().normalize(text);
        match nfc.len() {
            0 => Err(NameError::Empty),
            n if n > MAX_NAME_BYTES => Err(NameError::TooLong),
            _ => Ok(LogicalDb(nfc.as_ref().into())),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The name as a UI shows it (fix round 1, review m8): every invisible or reordering
    /// character as `<U+XXXX>`. A name may hold zero-width or format characters (only controls
    /// and bidi controls are refused), so a raw display could pass for another name.
    pub fn display_escaped(&self) -> String {
        escape_untrusted(&self.0, usize::MAX)
    }

    /// The binding comparison ([`same_db`]).
    pub fn same_db(&self, other: &LogicalDb) -> bool {
        same_db(self.as_str(), other.as_str())
    }
}

/// Why `c` may not stand in a logical name, if it may not. NFC never composes a control or a
/// bidi control out of other characters, so checking the input is checking the stored text.
fn refusal(c: char) -> Option<NameError> {
    if c.is_control() {
        Some(NameError::Control)
    } else if is_bidi_control(c) {
        Some(NameError::Bidi)
    } else {
        None
    }
}

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Whether `c` is drawn as nothing or reorders text: controls, bidi controls, zero-width and
/// other format characters. Shown as `<U+XXXX>` (M3's chooser rule; GC "Untrusted input").
fn is_invisible(c: char) -> bool {
    c.is_control()
        || is_bidi_control(c)
        || matches!(
            c,
            '\u{00AD}'
                | '\u{034F}'
                | '\u{115F}'
                | '\u{1160}'
                | '\u{17B4}'
                | '\u{17B5}'
                | '\u{180B}'..='\u{180F}'
                | '\u{200B}'..='\u{200D}'
                | '\u{2028}'..='\u{2029}'
                | '\u{2060}'..='\u{2065}'
                | '\u{3164}'
                | '\u{FE00}'..='\u{FE0F}'
                | '\u{FEFF}'
                | '\u{FFA0}'
                | '\u{FFF9}'..='\u{FFFB}'
        )
}

/// `text` with every invisible or reordering character written as `<U+XXXX>`, cut after
/// `max_chars` characters with `…`: how untrusted names from a library enter a diagnostic, and
/// how the app and UI show any name read from a library (public since fix round 1, review m8).
pub fn escape_untrusted(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, c) in text.chars().enumerate() {
        if i == max_chars {
            out.push('…');
            break;
        }
        if is_invisible(c) {
            out.push_str(&format!("<U+{:04X}>", u32::from(c)));
        } else {
            out.push(c);
        }
    }
    out
}

impl fmt::Display for LogicalDb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Serialized as the plain string (`workspace.toml`'s `database` and `aliases`).
impl Serialize for LogicalDb {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

/// Deserializing applies [`LogicalDb::parse`]'s rules.
impl<'de> Deserialize<'de> for LogicalDb {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        LogicalDb::parse(&s).map_err(|_| serde::de::Error::custom("not a logical database name"))
    }
}

/// A database's exact name on its server, as the explorer shows it (`"main"` on SQLite,
/// R-M4b-32; the C-M4-4 fallback). A script tab binds to `(ProfileId, DatabaseName)` for its
/// whole life (R-M4b-4). Serialized as the plain string (`workspace.json`, Task 1b).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DatabaseName(Arc<str>);

impl DatabaseName {
    /// The server's own name; never validated, never joined to a path.
    pub fn new(exact: &str) -> Self {
        DatabaseName(exact.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DatabaseName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for DatabaseName {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for DatabaseName {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(DatabaseName(s.into()))
    }
}
// ## m4b-t01a-seams

#[cfg(test)]
mod tests {
    use super::escape_untrusted;

    #[test]
    fn escape_writes_invisible_characters_as_code_points() {
        assert_eq!(
            escape_untrusted("a\u{202E}b\u{200B}c\td", 64),
            "a<U+202E>b<U+200B>c<U+0009>d"
        );
        assert_eq!(escape_untrusted("abcdef", 3), "abc…");
        assert_eq!(escape_untrusted("müşteri", 64), "müşteri");
    }
}
