//! Type-ahead (Task 8): letters typed within a short pause build a prefix, and the next
//! selectable row whose text starts with it becomes the cursor. Matching never calls a casing
//! function (INV-3): letters fold through an explicit table.
//!
//! ## m4-t01b-seams: the fold is declared here; the matcher is Task 8's.

/// How typed letters match shown ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fold {
    /// Case and dotted/dotless `i` folded together (`I i İ ı` one letter, plus the Turkish
    /// pairs). The explorer sets it for a Turkish UI language or region and ties it to the
    /// filter's Turkish-tolerant toggle (D-074, R-M4-50).
    Tolerant,
    /// The access-key fold for the UI language.
    Locale { turkic: bool },
}

impl Default for Fold {
    /// `Locale { turkic: false }` (R-M4-50: the spike's Tolerant default is overruled by D-074).
    fn default() -> Self {
        Fold::Locale { turkic: false }
    }
}
