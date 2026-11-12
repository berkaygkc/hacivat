//! Paint-level checks of DESIGN.md's named rules (R12), behind the `scene-check` feature: they
//! read `Window::rendered_scene()`, which only GPUI's `bench-support` provides. Task 8 fills in
//! the bodies.

#[cfg(feature = "scene-check")]
use gpui::{Hsla, Window};

/// Number of solid quads in the last rendered frame whose fill equals `color` (within 1/255 per
/// channel). Stub: `0` until Task 8.
#[cfg(feature = "scene-check")]
pub fn count_solid_quads(window: &Window, color: Hsla) -> usize {
    let _ = (window, color);
    0
}

/// The One Filled Badge rule: exactly one `cp_fill` quad when the capsule shows production,
/// none otherwise; panics with the count (`cp-fill quads: N`). Stub: a no-op until Task 8.
#[cfg(feature = "scene-check")]
pub fn assert_one_filled_badge(window: &Window, cp_fill: Hsla, expect_production: bool) {
    let _ = (window, cp_fill, expect_production);
}
