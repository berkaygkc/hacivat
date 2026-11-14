//! Row-anchored two-axis scrolling (Task 17: the spike's `ScrollPos` math, follow-tail, shrink
//! clamping, thumb drag, coalesced deltas, fast-scroll velocity).

/// Where the grid is scrolled: the top row and the offset into it (`[0, row height)`), the
/// horizontal offset, and whether it follows rows streaming in at the bottom.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScrollState {
    pub top_row: u64,
    pub top_row_offset: f32,
    pub left_x: f64,
    pub follow_tail: bool,
}
