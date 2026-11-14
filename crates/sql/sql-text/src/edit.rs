//! A text edit (R-M3-42: digest 01's shape).

use text_size::TextRange;

/// Replace `delete` with `insert`. Both ends of `delete` fall on char boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextEdit {
    pub delete: TextRange,
    pub insert: String,
}
