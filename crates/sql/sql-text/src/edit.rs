//! A text edit (R-M3-42: digest 01's shape).

use text_size::TextRange;

/// Replace `delete` with `insert`. Both ends of `delete` fall on char boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextEdit {
    pub delete: TextRange,
    pub insert: String,
}

/// An edit that does not fit its text: `delete` reaches past the end or splits a character.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MisalignedEdit(pub TextRange);

impl TextEdit {
    /// Whether `delete` lies inside `text` with both ends on char boundaries.
    pub fn fits(&self, text: &str) -> bool {
        let (start, end) = (
            usize::from(self.delete.start()),
            usize::from(self.delete.end()),
        );
        text.is_char_boundary(start) && text.is_char_boundary(end)
    }

    /// Applies the edit to `text`; a misaligned edit changes nothing.
    pub fn apply(&self, text: &mut String) -> Result<(), MisalignedEdit> {
        if !self.fits(text) {
            return Err(MisalignedEdit(self.delete));
        }
        let range = usize::from(self.delete.start())..usize::from(self.delete.end());
        text.replace_range(range, &self.insert);
        Ok(())
    }
}
