//! The scrollbars (HV-GRID-02; Task 20), as the mock draws them: a 9 px track on `gutter` with
//! a 1 px `seam` edge towards the body; the thumb in `text-3` at 60 % opacity with a 3 px radius
//! (clamped to its width), 2 px inside the track; and, on the vertical track, the rows already
//! loaded below the viewport as a square `green-fill` band (Direct-Path Green). While the total
//! is unknown the track maps to the loaded rows, so the band runs from the thumb to the end.

use hacivat_ui_core::gpui_surface::{Bounds, Pixels, Window, fill, px};

use super::layers::{Palette, Snap, rect};
use crate::model::ScrollState;

/// The thumb's inset from the track's edges.
pub const THUMB_INSET_PX: f32 = 2.0;

/// The loaded band's inset from the track's sides.
pub const BAND_INSET_PX: f32 = 3.0;

/// The opacity of the thumb and the band.
pub const OPACITY: f32 = 0.6;

/// The vertical track's inner travel: `(top, length)` of where the thumb moves.
pub fn v_travel(track: Bounds<Pixels>) -> (f32, f32) {
    let top = f32::from(track.top()) + THUMB_INSET_PX;
    (
        top,
        (f32::from(track.size.height) - 2.0 * THUMB_INSET_PX).max(0.0),
    )
}

/// The horizontal track's inner travel: `(left, length)`.
pub fn h_travel(track: Bounds<Pixels>) -> (f32, f32) {
    let left = f32::from(track.left()) + THUMB_INSET_PX;
    (
        left,
        (f32::from(track.size.width) - 2.0 * THUMB_INSET_PX).max(0.0),
    )
}

/// The vertical thumb's bounds.
pub fn v_thumb(track: Bounds<Pixels>, scroll: &ScrollState) -> Bounds<Pixels> {
    let (top, travel) = v_travel(track);
    let (at, len) = scroll.thumb(travel);
    // The track's first px is its seam edge; the thumb sits in the 8 px after it.
    let x = f32::from(track.left()) + 1.0 + THUMB_INSET_PX;
    let w = f32::from(track.size.width) - 1.0 - 2.0 * THUMB_INSET_PX;
    rect(x, top + at, w, len)
}

/// The horizontal thumb's bounds.
pub fn h_thumb(track: Bounds<Pixels>, scroll: &ScrollState) -> Bounds<Pixels> {
    let (left, travel) = h_travel(track);
    let (at, len) = scroll.h_thumb(travel);
    let y = f32::from(track.top()) + 1.0 + THUMB_INSET_PX;
    let h = f32::from(track.size.height) - 1.0 - 2.0 * THUMB_INSET_PX;
    rect(left + at, y, len, h)
}

/// Paints the vertical track, its loaded band and its thumb.
pub fn paint_vertical(
    track: Bounds<Pixels>,
    scroll: &ScrollState,
    loaded: u64,
    palette: &Palette,
    snap: Snap,
    window: &mut Window,
) {
    window.paint_quad(fill(track, palette.gutter));
    let (top, bottom) = (f32::from(track.top()), f32::from(track.bottom()));
    window.paint_quad(fill(
        snap.vline(f32::from(track.left()) + snap.hair(), top, bottom),
        palette.seam,
    ));
    let thumb = v_thumb(track, scroll);
    let rows = scroll.row_count();
    if rows > 0 && loaded > 0 {
        let (travel_top, travel) = v_travel(track);
        let loaded_y = travel_top + travel * (loaded.min(rows) as f64 / rows as f64) as f32;
        let from = f32::from(thumb.bottom()) + THUMB_INSET_PX;
        let to = loaded_y - THUMB_INSET_PX;
        if to > from {
            let x = f32::from(track.left()) + 1.0 + BAND_INSET_PX;
            let w = f32::from(track.size.width) - 1.0 - 2.0 * BAND_INSET_PX;
            let mut band = palette.green_fill;
            band.a *= OPACITY;
            window.paint_quad(fill(rect(x, from, w, to - from), band));
        }
    }
    let mut ink = palette.text3;
    ink.a *= OPACITY;
    let radius = 3.0f32.min(f32::from(thumb.size.width) / 2.0);
    window.paint_quad(fill(thumb, ink).corner_radii(px(radius)));
}

/// Paints the horizontal track and its thumb.
pub fn paint_horizontal(
    track: Bounds<Pixels>,
    scroll: &ScrollState,
    palette: &Palette,
    snap: Snap,
    window: &mut Window,
) {
    window.paint_quad(fill(track, palette.gutter));
    let (left, right) = (f32::from(track.left()), f32::from(track.right()));
    window.paint_quad(fill(
        snap.hline(left, right, f32::from(track.top()) + snap.hair()),
        palette.seam,
    ));
    let thumb = h_thumb(track, scroll);
    let mut ink = palette.text3;
    ink.a *= OPACITY;
    let radius = 3.0f32.min(f32::from(thumb.size.height) / 2.0);
    window.paint_quad(fill(thumb, ink).corner_radii(px(radius)));
}
