//! The height map (UI-EDIT-08; Task 11). M3a: uniform rows; M3b adds a chunked map for folds
//! and wrapped lines.

/// Row heights of a document.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HeightMap {
    Uniform { lines: u32, line_height: f32 },
}
