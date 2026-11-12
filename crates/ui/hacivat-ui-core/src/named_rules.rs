//! Paint-level checks of DESIGN.md's named rules (R12), behind the `scene-check` feature: they
//! read `Window::rendered_scene()`, which only GPUI's `bench-support` provides. Release builds
//! never enable it (`AdapterFeatureInProduct`).

#[cfg(feature = "scene-check")]
use gpui::{Hsla, Rgba, Window};

/// Number of solid quads in the last rendered frame whose fill equals `color` (within 1/255 per
/// RGBA channel). Counts what was painted, not which helper painted it: gradients, borders and
/// glyphs never count.
#[cfg(feature = "scene-check")]
pub fn count_solid_quads(window: &Window, color: Hsla) -> usize {
    let want = Rgba::from(color);
    window
        .rendered_scene()
        .quads
        .iter()
        .filter_map(|quad| quad.background.as_solid())
        .filter(|fill| same_rgba(Rgba::from(*fill), want))
        .count()
}

/// The One Filled Badge rule: exactly one `cp_fill` quad when the capsule shows production,
/// none otherwise; panics with the count (`cp-fill quads: N`).
#[cfg(feature = "scene-check")]
pub fn assert_one_filled_badge(window: &Window, cp_fill: Hsla, expect_production: bool) {
    let count = count_solid_quads(window, cp_fill);
    let expected = usize::from(expect_production);
    assert!(
        count == expected,
        "One Filled Badge rule: cp-fill quads: {count} (expected {expected})"
    );
}

/// Equal within 1/255 per channel, so an `Hsla` round trip never decides the count.
#[cfg(feature = "scene-check")]
fn same_rgba(a: Rgba, b: Rgba) -> bool {
    const TOLERANCE: f32 = 1.0 / 255.0;
    [(a.r, b.r), (a.g, b.g), (a.b, b.b), (a.a, b.a)]
        .iter()
        .all(|(x, y)| (x - y).abs() <= TOLERANCE)
}

#[cfg(all(test, feature = "scene-check"))]
mod tests {
    use super::*;

    fn rgba(r: f32, g: f32, b: f32, a: f32) -> Rgba {
        Rgba { r, g, b, a }
    }

    #[test]
    fn colours_within_one_step_per_channel_match() {
        let base = rgba(0.5, 0.25, 0.75, 1.0);
        assert!(same_rgba(base, base));
        assert!(same_rgba(base, rgba(0.5 + 0.9 / 255.0, 0.25, 0.75, 1.0)));
        assert!(same_rgba(base, rgba(0.5, 0.25, 0.75, 1.0 - 0.9 / 255.0)));
    }

    #[test]
    fn colours_two_steps_apart_in_any_channel_differ() {
        let base = rgba(0.5, 0.25, 0.75, 1.0);
        assert!(!same_rgba(base, rgba(0.5 + 2.0 / 255.0, 0.25, 0.75, 1.0)));
        assert!(!same_rgba(base, rgba(0.5, 0.25 - 2.0 / 255.0, 0.75, 1.0)));
        assert!(!same_rgba(base, rgba(0.5, 0.25, 0.75 + 2.0 / 255.0, 1.0)));
        assert!(!same_rgba(base, rgba(0.5, 0.25, 0.75, 1.0 - 2.0 / 255.0)));
    }
}
