//! M3a Task 17: the row-anchored two-axis scroll model (HV-GRID-02). The M0 spike's nine scroll
//! tests (`spikes/spike-grid/src/scroll.rs`), ported to `ScrollState`, and the `alg/22` cases.
//! Row heights are explicit: 24 px for the alg/22 viewport cases, 21 px (the grid's) elsewhere
//! (P18).

use hacivat_ui_grid::model::{OVERSCAN_ROWS, ScrollState};

/// A grid of `rows` rows, `viewport_h` tall, 800 px wide.
fn view(rows: u64, viewport_h: f32, row_h: f32) -> ScrollState {
    let mut s = ScrollState::new(row_h);
    s.set_viewport(800.0, viewport_h);
    s.on_row_count(rows);
    s
}

/// A 600 px viewport of 24 px rows over `rows` rows, scrolled to `top`.
fn at_row(top: u64, rows: u64) -> ScrollState {
    let mut s = view(rows, 600.0, 24.0);
    s.scroll_by(top as f64 * 24.0);
    assert_eq!((s.top_row, s.top_row_offset), (top, 0.0));
    s
}

/// The top row after End.
fn end_top_row(s: &ScrollState) -> u64 {
    let mut e = *s;
    e.scroll_to_end();
    e.top_row
}

fn pos(s: &ScrollState) -> (u64, f32) {
    (s.top_row, s.top_row_offset)
}

/// The spike's nine tests (`spikes/spike-grid/src/scroll.rs:94-250`), one for one.
mod spike_scroll_tests_ported {
    use super::*;

    #[test]
    fn scroll_to_last_row_is_exact() {
        let mut s = view(10_000_000, 40.0 * 21.0, 21.0);
        s.scroll_by(1e12);
        assert_eq!(pos(&s), (10_000_000 - 40, 0.0));
        assert_eq!(s.visible_rows().end, 10_000_000);
    }

    #[test]
    fn small_steps_accumulate_without_drift() {
        let mut s = view(10_000_000, 40.0 * 21.0, 21.0);
        s.scroll_by(9_000_000.0 * 21.0);
        assert_eq!(pos(&s), (9_000_000, 0.0));
        for _ in 0..21_000 {
            s.scroll_by(1.0);
        }
        assert_eq!(pos(&s), (9_001_000, 0.0));
    }

    #[test]
    fn cannot_scroll_above_zero() {
        let mut s = view(100, 10.0 * 21.0, 21.0);
        s.scroll_by(5.0);
        assert_eq!(pos(&s), (0, 5.0));
        s.scroll_by(-100.0);
        assert_eq!(pos(&s), (0, 0.0));
    }

    #[test]
    fn visible_rows_includes_partial_last_row() {
        let mut s = view(1_000, 100.0, 21.0);
        s.scroll_by(10.0 * 21.0 + 7.0);
        assert_eq!(pos(&s), (10, 7.0));
        assert_eq!(s.visible_rows(), 10..16);
    }

    #[test]
    fn thumb_has_min_height() {
        let s = view(10_000_000, 800.0, 21.0);
        assert!(s.thumb(800.0).1 >= 24.0);
    }

    #[test]
    fn offset_stays_in_row_range_for_fractional_steps() {
        let mut s = view(10_000_000, 40.0 * 21.0, 21.0);
        s.scroll_by(5_000_000.0 * 21.0);
        for i in 0..10_000 {
            s.scroll_by(if i % 3 == 0 { -7.3 } else { 13.7 });
            assert!((0.0..21.0).contains(&s.top_row_offset), "{s:?}");
        }
    }

    #[test]
    fn scrolling_down_then_up_returns_home() {
        let mut s = view(10_000_000, 40.0 * 21.0, 21.0);
        s.scroll_by(7_654_321.0 * 21.0 + 3.0);
        let start = pos(&s);
        assert_eq!(start, (7_654_321, 3.0));
        s.scroll_by(123_456.25);
        s.scroll_by(-123_456.25);
        assert_eq!(pos(&s), start);
    }

    #[test]
    fn thumb_reaches_track_bottom_at_last_row() {
        let mut s = view(10_000_000, 800.0, 21.0);
        s.scroll_by(1e12);
        let (top, h) = s.thumb(800.0);
        assert!((top + h - 800.0).abs() < 1e-3, "{top} {h}");
    }

    #[test]
    fn short_tables_do_not_scroll() {
        let mut s = view(5, 800.0, 21.0);
        s.scroll_by(500.0);
        assert_eq!(pos(&s), (0, 0.0));
        assert_eq!(s.visible_rows(), 0..5);
        assert_eq!(s.thumb(800.0), (0.0, 800.0));
    }
}

#[test]
fn billion_rows_viewport_math() {
    let s = view(1_000_000_000, 600.0, 24.0);
    assert_eq!(s.visible_rows(), 0..25);
    assert_eq!(end_top_row(&s), 999_999_975);
}

#[test]
fn end_shows_the_last_row_fully() {
    let mut s = view(1_000_000_000, 600.0, 24.0);
    s.scroll_to_end();
    assert_eq!(pos(&s), (999_999_975, 0.0));
    assert_eq!(s.visible_rows(), 999_999_975..1_000_000_000);
    assert_eq!(s.row_top(999_999_999), 576.0);
}

#[test]
fn thousand_3px_scrolls_move_125_rows() {
    let mut s = view(10_000, 600.0, 24.0);
    for _ in 0..1_000 {
        s.scroll_by(3.0);
    }
    assert_eq!(s.top_row, 125);
    assert_eq!(s.top_row_offset, 0.0);
}

#[test]
fn thumb_dragged_to_half_is_the_middle_row() {
    let mut s = view(1_000_000_000, 600.0, 24.0);
    s.scroll_to_fraction(0.5);
    let expected = ((1_000_000_000f64 * 24.0 - 600.0) / 2.0 / 24.0).floor() as u64;
    assert!(
        s.top_row.abs_diff(expected) <= 1,
        "{} vs {expected}",
        s.top_row
    );
    assert!((0.0..24.0).contains(&s.top_row_offset));
}

#[test]
fn thumb_drag_extremes_are_exact() {
    let mut s = view(1_000_000_000, 600.0, 24.0);
    let (_, h) = s.thumb(600.0);
    s.drag_thumb(f64::from(600.0 - h), 600.0);
    assert_eq!(pos(&s), (999_999_975, 0.0));
    s.drag_thumb(1e9, 600.0);
    assert_eq!(pos(&s), (999_999_975, 0.0));
    s.drag_thumb(0.0, 600.0);
    assert_eq!(pos(&s), (0, 0.0));
    s.drag_thumb(-50.0, 600.0);
    assert_eq!(pos(&s), (0, 0.0));
    s.scroll_to_fraction(1.0);
    assert_eq!(pos(&s), (999_999_975, 0.0));
    s.scroll_to_fraction(f64::NAN);
    assert_eq!(
        pos(&s),
        (999_999_975, 0.0),
        "a non-finite fraction moves nothing"
    );
}

#[test]
fn thumb_follows_the_position_in_f64() {
    // Halfway through a billion rows the thumb sits halfway down the free track.
    let mut s = view(1_000_000_000, 600.0, 24.0);
    s.scroll_to_fraction(0.5);
    let (top, h) = s.thumb(600.0);
    assert!((top - (600.0 - h) / 2.0).abs() < 0.01, "{top} {h}");
}

#[test]
fn shrink_clamps_top_row() {
    let mut s = at_row(9_000, 10_000);
    s.on_row_count(10);
    assert_eq!(s.top_row, 0);
    assert_eq!(s.top_row_offset, 0.0);

    let mut s = at_row(9_000, 10_000);
    s.scroll_by(5.0);
    s.on_row_count(5_000);
    assert_eq!(pos(&s), (5_000 - 25, 0.0), "clamped to the new last page");
}

#[test]
fn follow_tail_sticks_only_when_at_the_bottom() {
    // At the bottom of a scrollable result: rows streaming in keep the view at the bottom.
    let mut s = view(100, 600.0, 24.0);
    s.scroll_by(1e9);
    assert!(s.follow_tail);
    s.on_row_count(200);
    assert_eq!(s.top_row, 175);
    s.on_row_count(1_000);
    assert_eq!(s.top_row, 975);

    // Scrolled away from the bottom: the view stays where the user left it.
    s.scroll_by(-24.0);
    assert!(!s.follow_tail);
    s.on_row_count(2_000);
    assert_eq!(s.top_row, 974);

    // A grid that never scrolled (the first rows of a run) keeps its first rows in view.
    let mut s = view(10, 600.0, 24.0);
    assert!(!s.follow_tail);
    s.scroll_by(30.0);
    assert!(
        !s.follow_tail,
        "a short grid is not at a bottom the user chose"
    );
    s.on_row_count(10_000);
    assert_eq!(pos(&s), (0, 0.0));

    // End (the last loaded row while streaming) follows the tail even before the grid scrolls.
    let mut s = view(10, 600.0, 24.0);
    s.scroll_to_end();
    assert!(s.follow_tail);
    s.scroll_by(30.0);
    assert!(s.follow_tail, "a wheel on a short grid keeps the choice");
    s.on_row_count(10_000);
    assert_eq!(s.top_row, 10_000 - 25);
}

#[test]
fn follow_tail_resets_when_a_run_starts_over() {
    // A result scrolled to the bottom shrinks to nothing (a re-run) and grows again: the new
    // rows are shown from the top.
    let mut s = view(100, 600.0, 24.0);
    s.scroll_to_end();
    assert!(s.follow_tail);
    s.on_row_count(0);
    assert!(!s.follow_tail);
    s.on_row_count(50);
    assert_eq!(pos(&s), (0, 0.0));

    // The explicit reset: top, no tail, queued deltas dropped, left_x kept.
    let mut s = view(1_000, 600.0, 24.0);
    s.set_content_width(2_000.0);
    s.set_left_x(500.0);
    s.scroll_to_end();
    s.queue(240.0, 0.0);
    s.on_generation_changed();
    assert_eq!(pos(&s), (0, 0.0));
    assert!(!s.follow_tail);
    s.apply_pending();
    assert_eq!(pos(&s), (0, 0.0));
    assert_eq!(s.left_x, 500.0);
    s.on_row_count(0);
    s.on_row_count(5_000);
    assert_eq!(pos(&s), (0, 0.0));

    // A shrink that leaves the view at the bottom of a grid that still scrolls keeps following.
    let mut s = view(1_000, 600.0, 24.0);
    s.scroll_to_end();
    s.on_row_count(500);
    assert!(s.follow_tail);
    assert_eq!(s.top_row, 475);
    s.on_row_count(600);
    assert_eq!(s.top_row, 575);
}

#[test]
fn a_resize_that_leaves_the_bottom_stops_following() {
    let mut s = view(100, 600.0, 24.0);
    s.scroll_to_end();
    assert_eq!(s.top_row, 75);
    // A shorter body: the last page starts later, the view is no longer at the bottom.
    s.set_viewport(800.0, 300.0);
    assert_eq!(s.max_top_row(), 88);
    assert!(!s.follow_tail);
    s.on_row_count(200);
    assert_eq!(s.top_row, 75);

    // A taller body clamps the view to the new last page: still the bottom, still following.
    let mut s = view(100, 600.0, 24.0);
    s.scroll_to_end();
    s.set_viewport(800.0, 1_200.0);
    assert_eq!(s.top_row, 50);
    assert!(s.follow_tail);
    s.on_row_count(200);
    assert_eq!(s.top_row, 150);
}

#[test]
fn the_f32_rounding_guard_moves_to_the_next_row() {
    // 21 − 1e-7 px fits in row 0 in f64, but its f32 offset rounds up to exactly 21.0: the
    // spike's guard moves to row 1 with offset 0 instead of storing an offset outside the row.
    let mut s = view(100, 600.0, 21.0);
    s.scroll_by(21.0 - 1e-7);
    assert_eq!(pos(&s), (1, 0.0));
    // From inside a row too.
    let mut s = view(100, 600.0, 21.0);
    s.scroll_by(20.0);
    s.scroll_by(1.0 - 1e-7);
    assert_eq!(pos(&s), (1, 0.0));
    // A gap the f32 offset can still hold stays in row 0.
    let mut s = view(100, 600.0, 21.0);
    s.scroll_by(21.0 - 1e-5);
    assert_eq!(s.top_row, 0);
    assert!(s.top_row_offset > 20.9 && s.top_row_offset < 21.0, "{s:?}");
}

#[test]
fn moves_the_user_did_not_make_are_not_velocity() {
    let mut s = view(100, 600.0, 24.0);
    s.scroll_to_end();
    s.apply_pending();
    assert_eq!(s.velocity_rows_per_frame(), 75.0, "End is the user's move");
    s.apply_pending();

    // Rows stream in while following: the view snaps 9,900 rows and stands still on screen.
    s.on_row_count(10_000);
    assert_eq!(s.top_row, 9_975);
    s.apply_pending();
    assert_eq!(s.velocity_rows_per_frame(), 0.0);

    // A snap and a user scroll in one frame: only the user's 2 rows count.
    s.on_row_count(20_000);
    s.scroll_by(-48.0);
    s.apply_pending();
    assert_eq!(s.velocity_rows_per_frame(), 2.0);

    // A shrink clamp, a resize and a re-run are not scrolls either.
    s.on_row_count(1_000);
    s.apply_pending();
    assert_eq!(s.velocity_rows_per_frame(), 0.0);
    s.scroll_to_end();
    s.apply_pending();
    s.set_viewport(800.0, 1_200.0);
    s.apply_pending();
    assert_eq!(s.velocity_rows_per_frame(), 0.0);
    s.on_generation_changed();
    s.apply_pending();
    assert_eq!(s.velocity_rows_per_frame(), 0.0);
}

#[test]
fn scroll_into_view_reveals_rows_above_and_below() {
    let mut s = view(1_000, 600.0, 24.0);
    s.scroll_by(100.0 * 24.0 + 5.0);
    assert_eq!(pos(&s), (100, 5.0));

    // Inside: nothing moves.
    s.scroll_into_view(110);
    assert_eq!(pos(&s), (100, 5.0));

    // Above: the row becomes the top row.
    s.scroll_into_view(40);
    assert_eq!(pos(&s), (40, 0.0));

    // Below: the row's bottom aligns with the viewport's bottom.
    s.scroll_into_view(500);
    assert_eq!(s.row_top(500) + 24.0, 600.0);
    assert_eq!(s.visible_rows().end, 501);

    // A partly visible top row is revealed whole.
    s.scroll_by(-30.0);
    let top = s.top_row;
    assert!(s.top_row_offset > 0.0);
    s.scroll_into_view(top);
    assert_eq!(pos(&s), (top, 0.0));

    // Past the end: clamped.
    s.scroll_into_view(5_000);
    assert_eq!(pos(&s), (975, 0.0));
}

#[test]
fn scroll_into_view_with_a_partial_last_row() {
    // 800 / 21 is not whole: the revealed row still ends exactly at the bottom.
    let mut s = view(1_000, 800.0, 21.0);
    s.scroll_into_view(100);
    assert!(
        (s.row_top(100) + 21.0 - 800.0).abs() < 1e-3,
        "{}",
        s.row_top(100)
    );
    assert!((0.0..21.0).contains(&s.top_row_offset));
}

#[test]
fn deltas_coalesce_into_one_move_per_frame() {
    let mut s = view(10_000, 600.0, 24.0);
    for _ in 0..10 {
        s.queue(3.0, 4.0);
    }
    assert_eq!(pos(&s), (0, 0.0), "nothing moves before the frame");
    s.set_content_width(2_000.0);
    assert!(s.apply_pending());
    assert_eq!(pos(&s), (1, 6.0));
    assert_eq!(s.left_x, 40.0);
    assert!(!s.apply_pending(), "nothing pending: no move");
}

#[test]
fn velocity_is_rows_per_frame_and_a_thumb_jump_is_infinite() {
    let mut s = view(1_000_000, 600.0, 24.0);
    assert_eq!(s.velocity_rows_per_frame(), 0.0);

    s.queue(80.0 * 24.0, 0.0);
    s.apply_pending();
    assert_eq!(s.velocity_rows_per_frame(), 80.0);

    s.queue(-12.0, 0.0);
    s.apply_pending();
    assert_eq!(s.velocity_rows_per_frame(), 0.5);

    // An idle frame drops the velocity.
    s.apply_pending();
    assert_eq!(s.velocity_rows_per_frame(), 0.0);

    // Direct scrolls between frames count too.
    s.scroll_by(48.0);
    s.apply_pending();
    assert_eq!(s.velocity_rows_per_frame(), 2.0);

    // A thumb jump is infinite for exactly one frame, however short the jump.
    s.drag_thumb(1.0, 600.0);
    s.apply_pending();
    assert_eq!(s.velocity_rows_per_frame(), f64::INFINITY);
    s.apply_pending();
    assert_eq!(s.velocity_rows_per_frame(), 0.0);
}

#[test]
fn horizontal_scroll_clamps_to_the_content() {
    let mut s = view(100, 600.0, 24.0);
    s.set_content_width(2_000.0);
    s.scroll_x_by(5_000.0);
    assert_eq!(s.left_x, 1_200.0);
    s.scroll_x_by(-1e12);
    assert_eq!(s.left_x, 0.0);
    s.set_left_x(900.0);
    assert_eq!(s.left_x, 900.0);
    // Content shrinking below the viewport resets the offset.
    s.set_content_width(500.0);
    assert_eq!(s.left_x, 0.0);
    s.scroll_x_by(10.0);
    assert_eq!(s.left_x, 0.0);
    assert_eq!(s.h_thumb(800.0), (0.0, 800.0));
}

#[test]
fn horizontal_reveal_and_thumb() {
    let mut s = view(100, 600.0, 24.0);
    s.set_content_width(4_000.0);
    s.scroll_x_into_view(1_000.0, 1_100.0);
    assert_eq!(s.left_x, 300.0, "right edge aligned");
    s.scroll_x_into_view(100.0, 200.0);
    assert_eq!(s.left_x, 100.0, "left edge aligned");
    s.scroll_x_into_view(150.0, 300.0);
    assert_eq!(s.left_x, 100.0, "already visible");
    // Wider than the viewport: the left edge wins.
    s.scroll_x_into_view(2_000.0, 3_500.0);
    assert_eq!(s.left_x, 2_000.0);

    let (_, w) = s.h_thumb(800.0);
    assert_eq!(w, 800.0 * 800.0 / 4_000.0);
    s.drag_h_thumb(f64::from(800.0 - w), 800.0);
    assert_eq!(s.left_x, 3_200.0);
    s.drag_h_thumb(0.0, 800.0);
    assert_eq!(s.left_x, 0.0);
}

#[test]
fn row_height_change_keeps_the_top_row() {
    let mut s = view(1_000, 600.0, 24.0);
    s.scroll_by(300.0 * 24.0 + 12.0);
    s.set_row_height(30.0);
    assert_eq!(s.top_row, 300);
    assert_eq!(s.top_row_offset, 15.0);
    assert_eq!(s.row_height(), 30.0);
    // Invalid heights are ignored.
    s.set_row_height(0.0);
    s.set_row_height(f32::NAN);
    assert_eq!(s.row_height(), 30.0);
}

#[test]
fn empty_and_degenerate_grids() {
    let mut s = view(0, 600.0, 24.0);
    assert_eq!(s.visible_rows(), 0..0);
    s.scroll_by(500.0);
    s.scroll_to_end();
    s.scroll_into_view(10);
    assert_eq!(pos(&s), (0, 0.0));
    assert_eq!(s.thumb(600.0), (0.0, 600.0));
    assert_eq!(s.row_at(10.0), None);

    // A viewport shorter than one row still scrolls one row at a time.
    let mut s = view(10, 5.0, 24.0);
    assert_eq!(s.viewport_rows(), 1);
    s.scroll_to_end();
    assert_eq!(s.top_row, 9);

    // Non-finite deltas move nothing.
    let mut s = view(100, 600.0, 24.0);
    s.scroll_by(f64::NAN);
    s.scroll_by(f64::INFINITY);
    s.queue(f64::NEG_INFINITY, f64::NAN);
    s.apply_pending();
    assert_eq!(pos(&s), (0, 0.0));
    assert_eq!(s.left_x, 0.0);
}

#[test]
fn row_hit_testing_and_positions() {
    let mut s = view(1_000, 600.0, 24.0);
    s.scroll_by(10.0 * 24.0 + 6.0);
    assert_eq!(s.row_top(10), -6.0);
    assert_eq!(s.row_top(11), 18.0);
    assert_eq!(s.row_at(0.0), Some(10));
    assert_eq!(s.row_at(17.9), Some(10));
    assert_eq!(s.row_at(18.0), Some(11));
    assert_eq!(s.row_at(-1.0), None);
    let mut short = view(3, 600.0, 24.0);
    short.scroll_by(0.0);
    assert_eq!(short.row_at(100.0), None, "below the last row");
}

#[test]
fn prefetch_reaches_overscan_rows_past_the_viewport() {
    let mut s = view(1_000, 600.0, 24.0);
    s.scroll_by(100.0 * 24.0);
    assert_eq!(s.prefetch_rows(), 100 - OVERSCAN_ROWS..125 + OVERSCAN_ROWS);
    let top = view(1_000, 600.0, 24.0);
    assert_eq!(top.prefetch_rows(), 0..25 + OVERSCAN_ROWS);
}

/// A small deterministic generator (SplitMix64): the crate has no `proptest` dev-dependency
/// (Task 1b's manifest is exhaustive), so the property runs over seeded random cases.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Deltas as a wheel, a trackpad and a fling produce them, with odd values mixed in.
    fn delta(&mut self) -> f64 {
        match self.next() % 8 {
            0 => (self.unit() - 0.5) * 6.0,
            1 => (self.unit() - 0.5) * 200.0,
            2 => (self.unit() - 0.5) * 1e7,
            3 => (self.unit() - 0.5) * 1e13,
            4 => f64::from(-(((self.next() % 40) as i32) - 20)) * 21.0,
            5 => (self.unit() - 0.5) * 1e-6,
            6 => [f64::NAN, f64::INFINITY, -0.0, 21.0 - f64::EPSILON][(self.next() % 4) as usize],
            _ => (self.unit() - 0.5) * 21.0 * 3.0,
        }
    }
}

/// The alg/22 property: the offset stays inside the top row and the top row inside its range.
#[test]
fn offset_stays_in_row() {
    let mut rng = Rng(0x7_17);
    for _ in 0..512 {
        let mut s = view(1_000_000, 600.0, 21.0);
        for _ in 0..64 {
            let d = rng.delta();
            if rng.next().is_multiple_of(2) {
                s.scroll_by(d);
            } else {
                s.queue(d, 0.0);
                s.apply_pending();
            }
            assert!((0.0..21.0).contains(&s.top_row_offset), "{d} {s:?}");
            assert!(s.top_row <= s.max_top_row(), "{d} {s:?}");
            if s.top_row == s.max_top_row() {
                assert_eq!(s.top_row_offset, 0.0, "{d} {s:?}");
            }
        }
    }
}

/// The alg/22 property: scrolling by `d` and back returns home when no bound clamps.
#[test]
fn scroll_by_round_trips_away_from_the_bounds() {
    let mut rng = Rng(0x7_17_02);
    for _ in 0..4_096 {
        // Start between rows ~476M and ~524M; |d| ≤ 1e9 px (~47.6M rows) never reaches a bound.
        let mut s = view(1_000_000_000, 600.0, 21.0);
        s.scroll_by(rng.unit() * 1e9 + 1e10);
        let start = pos(&s);
        let d = (rng.unit() - 0.5) * 2e9;
        s.scroll_by(d);
        s.scroll_by(-d);
        let moved =
            (s.top_row as f64 - start.0 as f64) * 21.0 + f64::from(s.top_row_offset - start.1);
        assert!(moved.abs() < 1e-3, "{start:?} {d} -> {:?}", pos(&s));
    }
}
