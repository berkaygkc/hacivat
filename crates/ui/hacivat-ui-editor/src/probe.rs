//! The frame probe (B3; Tasks 19 and 36): called once per input, at handler entry, and once
//! per painted frame.

use std::time::Instant;

use hacivat_ui_core::gpui_surface::App;

/// Measures input-to-paint latency.
pub trait FrameProbe {
    fn input(&self, at: Instant);
    fn painted(&self, cx: &mut App);
}
