//! Server names and lookup keys (HV-MODEL-01 "Names", HV-INTRO-14, R-M4-28).
//!
//! A [`Name`] is the exact text the server reported: never normalized, never case-mapped,
//! displayed verbatim. Lookup keys ([`IdentKey`]) are built only by
//! [`sql_ident::key::match_key`], re-exported here so the model, the resolver and DDL share one
//! fold table.

use std::sync::Arc;

pub use sql_ident::key::{
    DialectId, FoldRule, IdentCtx, IdentKey, NameClass, ServerDependent, has_non_ascii_cased,
    match_key,
};
// ## m4-t24-live (named exception): identifier quoting for the app's "Copy qualified name" and
// "Open data" (R-M4-13, R-M4-28: quoting only through `sql_ident::quote`; the app has no direct
// `sql-ident` dependency, and the manifests are Task 1a's).
pub use sql_ident::quote;

/// An object name exactly as the server reported it. `Arc<str>`: the cache decoder interns the
/// strings of a namespace, so 160k columns share their few dozen type names (R-M4-26).
pub type Name = Arc<str>;

/// One part of a typed path (`schema.table`): its text without quotes and whether it was
/// quoted. The same type `sql_ident` parses.
pub type NamePart = sql_ident::Ident;

/// Unicode `Default_Ignorable_Code_Point` (DerivedCoreProperties.txt, Unicode 16.0): code
/// points a renderer shows as nothing — soft hyphen, combining grapheme joiner, Arabic letter
/// mark, the Hangul fillers U+115F, U+1160, U+3164 and U+FFA0, Khmer inherent vowels, Mongolian
/// variation selectors and vowel separator, zero-width space/joiners, bidi marks, embeddings
/// and overrides, word joiner and invisible operators, bidi isolates, variation selectors, the
/// BOM, unassigned specials, shorthand format controls, musical format controls, and tags.
const DEFAULT_IGNORABLE: [(u32, u32); 17] = [
    (0x00AD, 0x00AD),
    (0x034F, 0x034F),
    (0x061C, 0x061C),
    (0x115F, 0x1160),
    (0x17B4, 0x17B5),
    (0x180B, 0x180F),
    (0x200B, 0x200F),
    (0x202A, 0x202E),
    (0x2060, 0x206F),
    (0x3164, 0x3164),
    (0xFE00, 0xFE0F),
    (0xFEFF, 0xFEFF),
    (0xFFA0, 0xFFA0),
    (0xFFF0, 0xFFF8),
    (0x1BCA0, 0x1BCA3),
    (0x1D173, 0x1D17A),
    (0xE0000, 0xE0FFF),
];

/// Blank or invisible code points outside `Default_Ignorable_Code_Point` (the Hangul fillers
/// are in it already): the Braille pattern blank (U+2800), the interlinear annotation
/// controls (U+FFF9–U+FFFB), and the Arabic, Syriac and Kaithi format characters
/// (U+0600–U+0605, U+06DD, U+070F, U+08E2, U+110BD, U+110CD).
/// ## m4-t33b-carries (named exception; T32 re-review RR-4): and the line and paragraph
/// separators (U+2028, U+2029; general category Zl/Zp, neither control nor default-ignorable),
/// which break a line or draw nothing.
const FILLERS: [(u32, u32); 9] = [
    (0x2028, 0x2029),
    (0x0600, 0x0605),
    (0x06DD, 0x06DD),
    (0x070F, 0x070F),
    (0x08E2, 0x08E2),
    (0x2800, 0x2800),
    (0xFFF9, 0xFFFB),
    (0x110BD, 0x110BD),
    (0x110CD, 0x110CD),
];

/// Whether `name` holds a character a reader cannot see: the explorer shows a badge
/// (`model-name-has-invisible-chars`); the arena sets `ObjFlags::INVISIBLE_CHARS` from it on
/// every build.
///
/// Flags C0/C1 controls, every [`DEFAULT_IGNORABLE`] code point and the [`FILLERS`] above (the
/// T1a fix round 1 table, final in Task 3). That is the list, not a claim of completeness: a
/// visible look-alike (a Cyrillic `а`, a no-break space) is not flagged. An ASCII name is
/// checked bytewise.
pub fn has_invisible_chars(name: &str) -> bool {
    if name.is_ascii() {
        return name.bytes().any(|b| b.is_ascii_control());
    }
    let within = |c: u32, ranges: &[(u32, u32)]| ranges.iter().any(|&(a, b)| (a..=b).contains(&c));
    name.chars().any(|ch| {
        let c = u32::from(ch);
        ch.is_control() || within(c, &DEFAULT_IGNORABLE) || within(c, &FILLERS)
    })
}

/// The [`Name`] of server bytes that should be UTF-8, and whether it is **lossy** (alg/02
/// HV-MODEL-01 edge 12): invalid UTF-8 is replaced by U+FFFD, and a NUL is kept but marks the
/// name lossy too (no DDL can carry it). A lossy name gets `ObjFlags::LOSSY_NAME` and is never
/// used to generate DDL.
pub fn lossy_name(bytes: &[u8]) -> (Name, bool) {
    match std::str::from_utf8(bytes) {
        Ok(text) => (Name::from(text), text.contains('\0')),
        Err(_) => (Name::from(String::from_utf8_lossy(bytes).as_ref()), true),
    }
}

/// [`lossy_name`] for UTF-16 server text (a lone surrogate is replaced by U+FFFD).
pub fn lossy_name_utf16(units: &[u16]) -> (Name, bool) {
    match String::from_utf16(units) {
        Ok(text) => {
            let nul = text.contains('\0');
            (Name::from(text), nul)
        }
        Err(_) => (Name::from(String::from_utf16_lossy(units)), true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spike's check, now through `sql_ident::key`: the stored ASCII-caseless key never
    /// touches a non-ASCII letter, and an unchanged name shares its allocation.
    #[test]
    fn ascii_caseless_never_touches_non_ascii() {
        let n: Name = Arc::from("MÜŞTERİ_Id");
        assert_eq!(FoldRule::AsciiCaseless.key(&n).as_str(), "mÜŞterİ_id");
        assert_eq!(FoldRule::Exact.key(&n).as_str(), "MÜŞTERİ_Id");
        assert!(FoldRule::AsciiCaseless.matches("Iı", "iı"));
        assert!(!FoldRule::AsciiCaseless.matches("İ", "i"));
        let plain: Name = Arc::from("users");
        let k = FoldRule::AsciiCaseless.key(&plain);
        assert!(std::ptr::eq(k.as_str().as_ptr(), plain.as_ptr()));
    }

    #[test]
    fn ascii_names_are_checked_bytewise() {
        assert!(has_invisible_chars("a\u{7}b"));
        assert!(has_invisible_chars("a\tb"));
        assert!(!has_invisible_chars("order_items"));
    }
}
