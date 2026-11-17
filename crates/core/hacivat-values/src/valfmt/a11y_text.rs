// ## m3b-t01c-core-seams (P-M3b-17)
//! `Purpose::A11y` (Task 40): what the grid's screen-reader subtree reads for a cell. Its own
//! file, so Task 40 never shares `display.rs` with Task 38.
//!
//! ## m3c-t40: the cell's display text (numbers grouped, NULL, chips and flags as the cell has
//! them), with two changes for speech (HV-GRID-16 step 2):
//! - a run of control characters (line breaks, tabs, bells; the grid draws them as chips) is
//!   spoken as one space, and none at the start or the end;
//! - at most [`A11Y_MAX_GRAPHEMES`] graphemes, then `…` (the "very wide cells" edge case).
//!
//! A text of plain characters within the cap is returned untouched (one byte scan).

use unicode_segmentation::UnicodeSegmentation;

use crate::valfmt::{ColumnFacts, Display, DisplaySettings};
use crate::value::ValueRef;

/// The most graphemes a cell's screen-reader text keeps.
const A11Y_MAX_GRAPHEMES: usize = 1_000;

/// The screen-reader text of `v` in column `col` under `s`.
pub(super) fn text(v: ValueRef<'_>, col: &ColumnFacts<'_>, s: &DisplaySettings) -> Display {
    let mut d = super::display::cell(v, col, s);
    let plain = !d.text.chars().any(char::is_control);
    if !(plain && d.text.len() <= A11Y_MAX_GRAPHEMES) {
        d.text = spoken(&d.text);
    }
    d
}

/// `text` with each run of control graphemes as one space (none at the ends), cut after
/// [`A11Y_MAX_GRAPHEMES`] graphemes with `…`.
fn spoken(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(4 * A11Y_MAX_GRAPHEMES));
    let mut kept = 0;
    let mut gap = false;
    for g in text.graphemes(true) {
        if g.starts_with(char::is_control) {
            gap = !out.is_empty();
            continue;
        }
        if kept == A11Y_MAX_GRAPHEMES {
            out.push('…');
            return out;
        }
        if gap {
            out.push(' ');
            kept += 1;
            gap = false;
            if kept == A11Y_MAX_GRAPHEMES {
                out.push('…');
                return out;
            }
        }
        out.push_str(g);
        kept += 1;
    }
    out
}
