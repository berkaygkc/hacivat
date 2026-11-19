//! Server names and lookup keys (HV-MODEL-01 "Names", HV-INTRO-14, R-M4-28).
//!
//! A [`Name`] is the exact text the server reported: never normalized, never case-mapped,
//! displayed verbatim. Lookup keys ([`IdentKey`]) are built only by
//! [`sql_ident::key::match_key`], re-exported here so the model, the resolver and DDL share one
//! fold table.

use std::sync::Arc;

pub use sql_ident::key::{
    DialectId, FoldRule, IdentCtx, IdentKey, NameClass, ServerDependent, match_key,
};

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
const FILLERS: [(u32, u32); 8] = [
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
/// (`model-name-has-invisible-chars`).
///
/// M4 Task 1a seam (fix round 1, m1): flags C0/C1 controls, every
/// [`DEFAULT_IGNORABLE`] code point and the [`FILLERS`] above. That is the list, not a claim
/// of completeness: a visible look-alike (a Cyrillic `а`, a no-break space) is not flagged.
/// Task 3 owns the final table and its test.
pub fn has_invisible_chars(name: &str) -> bool {
    let within = |c: u32, ranges: &[(u32, u32)]| ranges.iter().any(|&(a, b)| (a..=b).contains(&c));
    name.chars().any(|ch| {
        let c = u32::from(ch);
        ch.is_control() || within(c, &DEFAULT_IGNORABLE) || within(c, &FILLERS)
    })
}
