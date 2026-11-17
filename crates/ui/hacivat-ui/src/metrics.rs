//! ## m3c-t41-chrome (F2 review M9): the chrome metrics that the M3a follow-up F2 and the
//! narrow panes (design/m3b-safe surface 24) introduced as raw numbers, named once here.
//! Task 43 documents them in DESIGN.md ("Query, Results and Grid"); widgets read them from
//! here instead of repeating the numbers.

/// A wrapped chrome sentence's line pitch: the cursor strip's reason and the console's waiting
/// sentence (NOTES surface 11).
pub const WRAP_LINE_PX: f32 = 16.0;
/// A wrapped bar's padding above and below its lines (NOTES surface 11).
pub const WRAP_PAD_PX: f32 = 4.0;

/// The compact button (NOTES "Strip buttons"): 20 px tall.
pub const COMPACT_HEIGHT_PX: f32 = 20.0;
/// Its horizontal padding.
pub const COMPACT_PAD_PX: f32 = 7.0;
/// The gap between its icon and its label (the mock's `.strip .btn` gap).
pub const COMPACT_GAP_PX: f32 = 5.0;
/// Its label's size.
pub const COMPACT_LABEL_PX: f32 = 12.0;

/// The gap between the cursor strip's sentence and its glyph or buttons.
pub const STRIP_GAP_PX: f32 = 6.0;
/// The gap between a menu footer's chord and its word.
pub const MENU_FOOTER_GAP_PX: f32 = 5.0;

/// A narrow pane's chips wrap their label on 15 px lines (NOTES "Narrow toolbar").
pub const NARROW_LINE_PX: f32 = 15.0;
/// A narrow chip's padding: 2 px above and below, 7 px on the sides; at least 22 px tall.
pub const NARROW_CHIP_PAD_Y_PX: f32 = 2.0;
pub const NARROW_CHIP_PAD_X_PX: f32 = 7.0;
pub const NARROW_CHIP_MIN_PX: f32 = 22.0;
