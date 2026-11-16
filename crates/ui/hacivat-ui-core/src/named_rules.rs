//! Paint-level checks of DESIGN.md's named rules (R12), behind the `scene-check` feature: they
//! read `Window::rendered_scene()`, which only GPUI's `bench-support` provides. Release builds
//! never enable it (`AdapterFeatureInProduct`).

#[cfg(feature = "scene-check")]
use gpui::{Bounds, Hsla, Pixels, Rgba, Window, px};

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

// ## m2a-t21-shots
/// A quad wider and taller than this (logical px) is badge-sized; the 7 px environment marks
/// (NOTES "NEW primitives") are not badges.
#[cfg(feature = "scene-check")]
const BADGE_MIN_PX: f32 = 8.0;

/// The `cp_fill` quads of the last frame, split by the title-bar capsule: those painted inside
/// `capsule` (logical px), and the badge-sized ones outside it. In the light theme `red` equals
/// `cp_fill`, so the 7 px production marks in dialogs and the explorer match the colour; they are
/// neither inside the capsule nor badge-sized, so they never count (T21 ruling).
#[cfg(feature = "scene-check")]
pub fn filled_badges(
    window: &Window,
    cp_fill: Hsla,
    capsule: Option<Bounds<Pixels>>,
) -> (usize, usize) {
    let want = Rgba::from(cp_fill);
    let scale = window.scale_factor();
    let (mut inside, mut outside) = (0, 0);
    for quad in &window.rendered_scene().quads {
        let Some(fill) = quad.background.as_solid() else {
            continue;
        };
        if !same_rgba(Rgba::from(fill), want) {
            continue;
        }
        let b = quad.bounds;
        let logical = Bounds::new(
            gpui::point(px(b.origin.x.0 / scale), px(b.origin.y.0 / scale)),
            gpui::size(px(b.size.width.0 / scale), px(b.size.height.0 / scale)),
        );
        if capsule.is_some_and(|c| contains(c, logical)) {
            inside += 1;
        } else if f32::from(logical.size.width) > BADGE_MIN_PX
            && f32::from(logical.size.height) > BADGE_MIN_PX
        {
            outside += 1;
        }
    }
    (inside, outside)
}

/// The One Filled Badge rule scoped to the title-bar capsule (`capsule`, logical px; `None` when
/// no capsule was painted): exactly one `cp_fill` quad inside it when it shows production, none
/// otherwise, and no badge-sized `cp_fill` quad anywhere else. Panics with both counts.
#[cfg(feature = "scene-check")]
pub fn assert_one_filled_badge_in(
    window: &Window,
    cp_fill: Hsla,
    expect_production: bool,
    capsule: Option<Bounds<Pixels>>,
) {
    let (inside, outside) = filled_badges(window, cp_fill, capsule);
    let expected = usize::from(expect_production);
    assert!(
        inside == expected && outside == 0,
        "One Filled Badge rule: cp-fill quads in the capsule: {inside} (expected {expected}); \
         badge-sized cp-fill quads outside it: {outside}"
    );
}

/// Whether `inner` lies inside `outer`, with half a pixel of slack for rounding.
#[cfg(feature = "scene-check")]
fn contains(outer: Bounds<Pixels>, inner: Bounds<Pixels>) -> bool {
    let slack = px(0.5);
    inner.left() >= outer.left() - slack
        && inner.top() >= outer.top() - slack
        && inner.right() <= outer.right() + slack
        && inner.bottom() <= outer.bottom() + slack
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
