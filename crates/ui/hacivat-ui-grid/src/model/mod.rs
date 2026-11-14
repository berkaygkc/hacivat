//! The grid model (HV-GRID-02 vertical and horizontal, -04 basic, -05 core; Task 17): no GPUI.
//! Absolute positions are `u64` rows and `f64` pixels; selections count cells in `u128`.

mod autosize;
mod columns;
mod keys;
mod scroll;
mod selection;

pub use columns::ColumnLayout;
pub use keys::Dir;
pub use scroll::ScrollState;
pub use selection::{CopyRefusal, CopyRegion, Selection};
