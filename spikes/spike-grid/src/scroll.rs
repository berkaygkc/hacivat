//! Row-anchored vertical scroll model.

/// Scroll position anchored to a row. `offset_px` is in `[0, row_h)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollPos {
    pub top_row: u64,
    pub offset_px: f32,
}

/// Largest valid `top_row`: the last page shows the final row fully, with `offset_px == 0`.
fn max_top_row(total_rows: u64, viewport_rows: u64) -> u64 {
    total_rows.saturating_sub(viewport_rows.max(1))
}

/// Whole rows that fit in `viewport_h` (at least one).
pub fn viewport_rows(viewport_h: f32, row_h: f32) -> u64 {
    ((viewport_h as f64 / row_h as f64).floor() as u64).max(1)
}

/// Moves `pos` by `delta_px` (positive scrolls towards later rows) and clamps it to
/// `[0, total_rows - viewport_rows]`. The arithmetic is relative to the anchor row in `f64`,
/// so the result never depends on an absolute pixel offset.
pub fn scroll_by(
    pos: ScrollPos,
    delta_px: f64,
    row_h: f32,
    total_rows: u64,
    viewport_rows: u64,
) -> ScrollPos {
    const TOP: ScrollPos = ScrollPos {
        top_row: 0,
        offset_px: 0.0,
    };
    let row_h_f = row_h as f64;
    let delta = if delta_px.is_finite() { delta_px } else { 0.0 };
    let max_top = max_top_row(total_rows, viewport_rows);

    let within = pos.offset_px as f64 + delta;
    let row_steps = (within / row_h_f).floor();
    let mut offset = within - row_steps * row_h_f;
    // `as` saturates; i128 keeps `top_row + row_steps` exact for any u64 anchor.
    let mut top = pos.top_row as i128 + row_steps as i128;
    if offset >= row_h_f || (offset as f32) >= row_h {
        top += 1;
        offset = 0.0;
    }
    if top < 0 {
        return TOP;
    }
    if top >= max_top as i128 {
        return ScrollPos {
            top_row: max_top,
            offset_px: 0.0,
        };
    }
    ScrollPos {
        top_row: top as u64,
        offset_px: offset.max(0.0) as f32,
    }
}

/// Rows that intersect a viewport of height `viewport_h`, including a partial last row.
pub fn visible_rows(
    pos: ScrollPos,
    viewport_h: f32,
    row_h: f32,
    total_rows: u64,
) -> std::ops::Range<u64> {
    let start = pos.top_row.min(total_rows);
    let span = ((pos.offset_px as f64 + viewport_h.max(0.0) as f64) / row_h as f64).ceil() as u64;
    start..start.saturating_add(span).min(total_rows)
}

/// Minimum scrollbar thumb height in logical px.
pub const MIN_THUMB_PX: f32 = 24.0;

/// Vertical scrollbar thumb `(top, height)` relative to a track of height `viewport_h`.
pub fn scrollbar_thumb(pos: ScrollPos, viewport_h: f32, row_h: f32, total_rows: u64) -> (f32, f32) {
    let track = viewport_h.max(0.0) as f64;
    let content = total_rows as f64 * row_h as f64;
    let max_top = max_top_row(total_rows, viewport_rows(viewport_h, row_h));
    if max_top == 0 || content <= track {
        return (0.0, track as f32);
    }
    let height = (track * track / content)
        .max(MIN_THUMB_PX as f64)
        .min(track);
    let scrolled = pos.top_row as f64 * row_h as f64 + pos.offset_px as f64;
    let max_scrolled = max_top as f64 * row_h as f64;
    let frac = (scrolled / max_scrolled).clamp(0.0, 1.0);
    ((frac * (track - height)) as f32, height as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_to_last_row_is_exact() {
        let p = scroll_by(
            ScrollPos {
                top_row: 0,
                offset_px: 0.0,
            },
            1e12,
            21.0,
            10_000_000,
            40,
        );
        assert_eq!(p.top_row, 10_000_000 - 40);
        assert_eq!(p.offset_px, 0.0);
        assert_eq!(
            visible_rows(p, 40.0 * 21.0, 21.0, 10_000_000).end,
            10_000_000
        );
    }

    #[test]
    fn small_steps_accumulate_without_drift() {
        let mut p = ScrollPos {
            top_row: 9_000_000,
            offset_px: 0.0,
        };
        for _ in 0..21_000 {
            p = scroll_by(p, 1.0, 21.0, 10_000_000, 40);
        }
        assert_eq!(
            p,
            ScrollPos {
                top_row: 9_001_000,
                offset_px: 0.0
            }
        );
    }

    #[test]
    fn cannot_scroll_above_zero() {
        assert_eq!(
            scroll_by(
                ScrollPos {
                    top_row: 0,
                    offset_px: 5.0
                },
                -100.0,
                21.0,
                100,
                10
            ),
            ScrollPos {
                top_row: 0,
                offset_px: 0.0
            }
        );
    }

    #[test]
    fn visible_rows_includes_partial_last_row() {
        assert_eq!(
            visible_rows(
                ScrollPos {
                    top_row: 10,
                    offset_px: 7.0
                },
                100.0,
                21.0,
                1000
            ),
            10..16
        );
    }

    #[test]
    fn thumb_has_min_height() {
        assert!(
            scrollbar_thumb(
                ScrollPos {
                    top_row: 0,
                    offset_px: 0.0
                },
                800.0,
                21.0,
                10_000_000
            )
            .1 >= 24.0
        );
    }

    #[test]
    fn offset_stays_in_row_range_for_fractional_steps() {
        let mut p = ScrollPos {
            top_row: 5_000_000,
            offset_px: 0.0,
        };
        for i in 0..10_000 {
            let d = if i % 3 == 0 { -7.3 } else { 13.7 };
            p = scroll_by(p, d, 21.0, 10_000_000, 40);
            assert!((0.0..21.0).contains(&p.offset_px), "{p:?}");
        }
    }

    #[test]
    fn scrolling_down_then_up_returns_home() {
        let start = ScrollPos {
            top_row: 7_654_321,
            offset_px: 3.0,
        };
        let there = scroll_by(start, 123_456.25, 21.0, 10_000_000, 40);
        let back = scroll_by(there, -123_456.25, 21.0, 10_000_000, 40);
        assert_eq!(back, start);
    }

    #[test]
    fn thumb_reaches_track_bottom_at_last_row() {
        let end = scroll_by(
            ScrollPos {
                top_row: 0,
                offset_px: 0.0,
            },
            1e12,
            21.0,
            10_000_000,
            38,
        );
        let (top, h) = scrollbar_thumb(end, 800.0, 21.0, 10_000_000);
        assert!((top + h - 800.0).abs() < 1e-3, "{top} {h}");
    }

    #[test]
    fn short_tables_do_not_scroll() {
        let p = scroll_by(
            ScrollPos {
                top_row: 0,
                offset_px: 0.0,
            },
            500.0,
            21.0,
            5,
            40,
        );
        assert_eq!(
            p,
            ScrollPos {
                top_row: 0,
                offset_px: 0.0
            }
        );
        assert_eq!(visible_rows(p, 800.0, 21.0, 5), 0..5);
        assert_eq!(scrollbar_thumb(p, 800.0, 21.0, 5), (0.0, 800.0));
    }
}
