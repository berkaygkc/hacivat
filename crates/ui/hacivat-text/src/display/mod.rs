//! The display model (UI-EDIT-07 pure parts, -08 uniform; Task 11): decorations in range
//! sets, line composition with visible controls ("Trojan Source"), a uniform height map and
//! row-anchored scrolling. Absolute pixels are `f64`; only viewport-relative ones are `f32`.

mod compose;
mod height;
mod rangeset;
mod scroll;

pub use compose::{Fragment, VisualLine, compose_line};
pub use height::HeightMap;
pub use rangeset::RangeSet;
pub use scroll::ScrollAnchor;

/// A text style of a decoration source (the adapter's highlight classes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StyleId(pub u16);

/// A whole-line class (the Run preview band, the current statement).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LineClass(pub u16);

/// A replacement's placeholder (M3b folds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlaceholderId(pub u16);

/// An inline widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WidgetId(pub u32);

/// What a decorated range does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Decoration {
    Mark {
        style: StyleId,
    },
    Line {
        class: LineClass,
    },
    Replace {
        placeholder: PlaceholderId,
    },
    /// `side` < 0 before the position, > 0 after it.
    Widget {
        id: WidgetId,
        side: i8,
    },
}
