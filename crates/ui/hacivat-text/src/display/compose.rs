//! Line composition (UI-EDIT-07 pure part; Task 11): text, tabs expanded by `unicode-width`,
//! and `Control` fragments for C0/C1, U+FEFF and every bidi control, always visible.

use smallvec::{SmallVec, smallvec};

use crate::buffer::TextBuffer;
use crate::display::{Decoration, PlaceholderId, RangeSet};

/// One piece of a visual line.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Fragment {
    Text {
        bytes: std::ops::Range<usize>,
    },
    Tab {
        width: u8,
    },
    /// A control character shown as `glyph` (`⟨RLO⟩`, `␀`, …).
    Control {
        byte: usize,
        glyph: char,
    },
    Placeholder {
        id: PlaceholderId,
    },
}

/// One painted row: the buffer lines it shows (more than one once folds exist) and its
/// fragments.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VisualLine {
    pub first_line: u32,
    pub last_line: u32,
    pub fragments: SmallVec<[Fragment; 8]>,
}

/// Composes buffer line `line` (Task 11). Task 1b: one text fragment over the line without
/// its break (no tab expansion, no control fragments, no decorations).
pub fn compose_line(
    buf: &TextBuffer,
    line: u32,
    tab_size: u8,
    decos: &[&RangeSet<Decoration>],
) -> VisualLine {
    let _ = (tab_size, decos);
    VisualLine {
        first_line: line,
        last_line: line,
        fragments: smallvec![Fragment::Text {
            bytes: buf.line_range(line as usize, false),
        }],
    }
}
