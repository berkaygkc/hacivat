//! Row badges (Task 14): the invisible-characters badge on a name, "partial", "out of date" and
//! the Built-in group's "version N" (mocks 2, 3, 12). Outline badges only (One Filled Badge):
//! the widget draws them; this module decides which one a row carries and its words.
//!
//! - **Invisible characters** (surface 12): an object whose name holds a bidi control, a
//!   zero-width character or another default-ignorable code point (`ObjFlags::INVISIBLE_CHARS`,
//!   set by every arena build) gets the eye-off glyph and "invisible characters"; its tooltip
//!   names the first such character (`U+200D ZERO WIDTH JOINER`). The name itself is shown
//!   verbatim: copying and DDL keep it.
//! - **Partial** (surface 2): a namespace whose load state is `Partial`; the alert glyph.
//! - **Out of date** (surface 3): a namespace loaded but marked stale; the history glyph.
//! - **Built-in version** (surface 3): set from Task 26's system-catalog flag; none until then.

use hacivat_meta::LoadState;
use hacivat_meta::name::has_invisible_chars;
use hacivat_ui::icon::IconName;

/// Which badge a row carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Badge {
    Invisible,
    Partial,
    OutOfDate,
}

impl Badge {
    /// The badge's word (a Fluent id).
    pub fn message_id(self) -> &'static str {
        match self {
            Badge::Invisible => "hv-explorer-invisible",
            Badge::Partial => "hv-explorer-partial",
            Badge::OutOfDate => "hv-explorer-stale",
        }
    }

    /// The badge's glyph (drawn alone when the row is crowded).
    pub fn glyph(self) -> IconName {
        match self {
            Badge::Invisible => IconName::EyeOff,
            Badge::Partial => IconName::CircleAlert,
            Badge::OutOfDate => IconName::History,
        }
    }
}

/// The badge a namespace row carries for its load state.
pub fn namespace_badge(state: Option<&LoadState>) -> Option<Badge> {
    match state? {
        LoadState::Partial { .. } => Some(Badge::Partial),
        LoadState::Loaded { stale: true, .. } => Some(Badge::OutOfDate),
        _ => None,
    }
}

/// Unicode names of the invisible characters met most often in names (the rest are named by
/// their code point only). Unicode character names are identifiers, never translated.
const NAMES: [(u32, &str); 22] = [
    (0x00AD, "SOFT HYPHEN"),
    (0x034F, "COMBINING GRAPHEME JOINER"),
    (0x061C, "ARABIC LETTER MARK"),
    (0x180E, "MONGOLIAN VOWEL SEPARATOR"),
    (0x200B, "ZERO WIDTH SPACE"),
    (0x200C, "ZERO WIDTH NON-JOINER"),
    (0x200D, "ZERO WIDTH JOINER"),
    (0x200E, "LEFT-TO-RIGHT MARK"),
    (0x200F, "RIGHT-TO-LEFT MARK"),
    (0x202A, "LEFT-TO-RIGHT EMBEDDING"),
    (0x202B, "RIGHT-TO-LEFT EMBEDDING"),
    (0x202C, "POP DIRECTIONAL FORMATTING"),
    (0x202D, "LEFT-TO-RIGHT OVERRIDE"),
    (0x202E, "RIGHT-TO-LEFT OVERRIDE"),
    (0x2060, "WORD JOINER"),
    (0x2066, "LEFT-TO-RIGHT ISOLATE"),
    (0x2067, "RIGHT-TO-LEFT ISOLATE"),
    (0x2068, "FIRST STRONG ISOLATE"),
    (0x2069, "POP DIRECTIONAL ISOLATE"),
    (0x3164, "HANGUL FILLER"),
    (0xFEFF, "ZERO WIDTH NO-BREAK SPACE"),
    (0xFFA0, "HALFWIDTH HANGUL FILLER"),
];

/// The first invisible character of `name`, described as `U+200D ZERO WIDTH JOINER` (or
/// `U+E0001` when it is not in the short table); `None` when the name has none.
pub fn first_invisible(name: &str) -> Option<String> {
    let mut buf = [0u8; 4];
    let ch = name
        .chars()
        .find(|c| has_invisible_chars(c.encode_utf8(&mut buf)))?;
    let code = u32::from(ch);
    Some(match NAMES.iter().find(|(c, _)| *c == code) {
        Some((_, n)) => format!("U+{code:04X} {n}"),
        None => format!("U+{code:04X}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_invisible_character_is_named() {
        assert_eq!(
            first_invisible("fatura\u{200D}lar").as_deref(),
            Some("U+200D ZERO WIDTH JOINER")
        );
        assert_eq!(first_invisible("users"), None);
        assert_eq!(first_invisible("a\u{E0001}b").as_deref(), Some("U+E0001"));
    }
}
