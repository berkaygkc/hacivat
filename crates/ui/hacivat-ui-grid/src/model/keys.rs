//! Key semantics (Task 17): direction and extend, jumps to the edge (never "next non-empty"),
//! the last row is the last loaded while streaming, Tab wraps rows and leaves from the last cell.

/// A grid direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}
