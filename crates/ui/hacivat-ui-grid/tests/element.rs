//! The grid element (Task 20): first frames over huge sources, placeholders and `ensure_rows`,
//! visible controls, truncation, fast-scroll's shaping cap and its quality recovery, cache
//! hits, the paint-level rules. The target builds only with `--features test-support`
//! (`SyntheticSource`, P6).

use std::rc::Rc;
use std::time::Duration;

use gpui::{Entity, TestAppContext, VisualTestContext, px, size};
use hacivat_ui::text::{TextRegistry, TextSource};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use hacivat_ui_grid::element::{FAST_SCROLL_ROWS_PER_FRAME, FAST_SHAPES_PER_FRAME};
use hacivat_ui_grid::testing::{FixtureSource, SyntheticSource, cell, grid_config};
use hacivat_ui_grid::{Align, CellKind, ColId, GridDataSource, GridView};

/// 120 Hz.
const FRAME: Duration = Duration::from_micros(8_333);

fn open(
    cx: &mut TestAppContext,
    source: Rc<dyn GridDataSource>,
    w: f32,
    h: f32,
) -> (Entity<GridView>, &mut VisualTestContext) {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let (view, vcx) =
        cx.add_window_view(|_, cx| GridView::new(source, grid_config("Result 1"), cx));
    vcx.simulate_resize(size(px(w), px(h)));
    vcx.run_until_parked();
    (view, vcx)
}

/// Repaints the window once (no input).
fn frame(vcx: &mut VisualTestContext) {
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
}

/// The texts and chips the next frame paints for a one-cell grid holding `text`.
fn paint_one(cx: &mut TestAppContext, text: &str, width: f32) -> (String, Vec<String>, bool) {
    let source = FixtureSource::texts(&[&[text]], width);
    let (view, vcx) = open(cx, Rc::new(source), 800.0, 300.0);
    vcx.update(|_, cx| view.update(cx, |v, _| v.set_paint_recording(true)));
    frame(vcx);
    vcx.read(|cx| {
        let cells = view.read(cx).painted_cells();
        assert_eq!(cells.len(), 1, "{cells:?}");
        let c = &cells[0];
        (
            c.text.to_string(),
            c.chips.iter().map(|s| s.to_string()).collect(),
            c.truncated,
        )
    })
}

fn painted_cell_text(cx: &mut TestAppContext, text: &str) -> String {
    paint_one(cx, text, 300.0).0
}

fn painted_chips(cx: &mut TestAppContext, text: &str) -> Vec<String> {
    paint_one(cx, text, 300.0).1
}

#[gpui::test]
fn ten_million_by_fifty_first_frame_renders(cx: &mut TestAppContext) {
    let (view, vcx) = open(
        cx,
        Rc::new(SyntheticSource::spike(10_000_000, 50)),
        1440.0,
        900.0,
    );
    let (stats, geom, cached) = vcx.read(|cx| {
        let v = view.read(cx);
        (v.frame_stats(), v.geometry().cloned(), v.cached_shapes())
    });
    let geom = geom.expect("the first frame was painted");
    // Only the visible rows × columns: the body's height in 21 px rows (under the 36 px header,
    // over the 9 px horizontal track), about 1,400 px of columns.
    let rows = geom.rows.end - geom.rows.start;
    let cols = (geom.cols.end - geom.cols.start) as u64;
    let body_h = f32::from(geom.body.size.height);
    assert_eq!(body_h, 900.0 - 36.0 - 9.0);
    assert_eq!(rows, (body_h / 21.0).ceil() as u64, "{geom:?}");
    assert!((8..=12).contains(&cols), "{geom:?}");
    assert_eq!(u64::from(stats.cells), rows * cols);
    assert!(cached < 1_000, "{cached} shapes for one screen");
    // Eight-digit row numbers widen the gutter past its 46 px.
    assert!(f32::from(geom.gutter.size.width) > 46.0, "{geom:?}");
    assert!(!stats.fast);
}

#[gpui::test]
fn placeholders_trigger_one_ensure_rows_per_frame(cx: &mut TestAppContext) {
    let source = FixtureSource::generated(4, 1_000, Some(10_000), true, |r, c| {
        cell(format!("r{r}c{}", c.0), CellKind::Text, Align::Start)
    });
    let (view, vcx) = open(cx, Rc::new(source.clone()), 800.0, 600.0);
    // The first screen is loaded: nothing to ask for.
    assert_eq!(source.ensure_calls(), 0);
    vcx.update(|_, cx| view.update(cx, |v, cx| v.scroll_to_row(5_000, cx)));
    vcx.run_until_parked();
    assert_eq!(source.ensure_calls_this_frame(), 1);
    let upto = source.last_ensure_upto().expect("asked for rows");
    assert!((5_000..=5_100).contains(&upto), "{upto}");
    // The placeholders painted as bars, not text.
    vcx.update(|_, cx| view.update(cx, |v, _| v.set_paint_recording(true)));
    frame(vcx);
    assert_eq!(source.ensure_calls_this_frame(), 1);
    let texts = vcx.read(|cx| view.read(cx).painted_cells().len());
    assert_eq!(texts, 0);
}

#[gpui::test]
fn tab_and_newline_paint_as_glyphs(cx: &mut TestAppContext) {
    assert_eq!(painted_cell_text(cx, "a\tb\nc"), "a→b↵c");
    assert_eq!(painted_chips(cx, "a\tb\nc"), ["→", "↵"]);
}

#[gpui::test]
fn bidi_controls_paint_as_chips(cx: &mut TestAppContext) {
    let chips = painted_chips(cx, "x\u{202E}y");
    assert!(chips.contains(&"⟨RLO⟩".to_string()), "{chips:?}");
    assert_eq!(painted_cell_text(cx, "x\u{202E}y"), "x⟨RLO⟩y");
    assert_eq!(painted_chips(cx, "\u{0}\u{2067}"), ["␀", "⟨RLI⟩"]);
}

#[gpui::test]
fn long_text_ellipsizes_inside_its_column(cx: &mut TestAppContext) {
    let long = "abcdefghijklmnopqrstuvwxyz".repeat(40);
    let (text, _, truncated) = paint_one(cx, &long, 120.0);
    assert!(truncated);
    // The registry and the copy see the whole value; only the painted run is cut.
    assert_eq!(text, long);
    let (short, _, cut) = paint_one(cx, "id", 120.0);
    assert_eq!(short, "id");
    assert!(!cut);
}

/// Scrolls `rows_per_frame` rows per frame for `frames` frames after a warm first frame.
fn fling_at_rows_per_frame(
    cx: &mut TestAppContext,
    rows_per_frame: f64,
    frames: usize,
) -> (Entity<GridView>, &mut VisualTestContext) {
    let (view, vcx) = open(
        cx,
        Rc::new(SyntheticSource::spike(10_000_000, 50)),
        1440.0,
        900.0,
    );
    vcx.update(|_, cx| view.update(cx, |v, _| v.reset_frame_log()));
    for _ in 0..frames {
        vcx.update(|_, cx| view.update(cx, |v, cx| v.scroll_px(rows_per_frame * 21.0, 0.0, cx)));
        vcx.executor().advance_clock(FRAME);
        vcx.run_until_parked();
    }
    (view, vcx)
}

#[gpui::test]
fn fast_scroll_caps_shapes_per_frame(cx: &mut TestAppContext) {
    let (view, vcx) = fling_at_rows_per_frame(cx, 400.0, 20);
    let log = vcx.read(|cx| *view.read(cx).frame_log());
    assert_eq!(log.frames, 20);
    assert_eq!(log.fast_frames, 20, "{log:?}");
    assert!(view.read_with(vcx, |v, _| v.max_shapes_per_frame()) <= FAST_SHAPES_PER_FRAME);
    // The cap was what held it: every frame meets about 420 new cells.
    assert!(log.degraded_frames > 0, "{log:?}");
    assert!(log.last.velocity > FAST_SCROLL_ROWS_PER_FRAME as f32);
}

#[gpui::test]
fn slow_scrolling_is_full_quality(cx: &mut TestAppContext) {
    // B1's fling peaks near 5 rows per frame at 120 Hz: never fast-scroll.
    let (view, vcx) = fling_at_rows_per_frame(cx, 5.0, 20);
    let log = vcx.read(|cx| *view.read(cx).frame_log());
    assert_eq!(log.fast_frames, 0, "{log:?}");
    assert_eq!(log.degraded_frames, 0, "{log:?}");
}

#[gpui::test]
fn a_thumb_jump_is_fast_for_one_frame(cx: &mut TestAppContext) {
    let (view, vcx) = open(
        cx,
        Rc::new(SyntheticSource::spike(10_000_000, 50)),
        1440.0,
        900.0,
    );
    vcx.update(|_, cx| view.update(cx, |v, cx| v.scroll_to_row(7_000_000, cx)));
    vcx.run_until_parked();
    let jump = view.read_with(vcx, |v, _| v.frame_stats());
    assert!(
        jump.fast && jump.shapes <= FAST_SHAPES_PER_FRAME,
        "{jump:?}"
    );
    assert_eq!(
        view.read_with(vcx, |v, _| v.scroll_state().top_row),
        7_000_000
    );
}

#[gpui::test]
fn quality_frame_within_100ms_of_slowing(cx: &mut TestAppContext) {
    let (view, vcx) = fling_at_rows_per_frame(cx, 400.0, 10);
    let last = view.read_with(vcx, |v, _| v.frame_stats());
    assert!(last.fast && last.degraded > 0, "{last:?}");
    // The fling stops: no more input. The degraded frame asked for the next frame; deliver
    // frames as the display would until one is full quality.
    let stopped = vcx.executor().now();
    let mut recovered = None;
    for _ in 0..12 {
        let delivered = vcx.update(|window, cx| window.simulate_next_frame(cx));
        vcx.executor().advance_clock(FRAME);
        vcx.run_until_parked();
        let s = view.read_with(vcx, |v, _| v.frame_stats());
        if delivered > 0 && !s.fast && s.degraded == 0 {
            recovered = Some((vcx.executor().now() - stopped, s));
            break;
        }
    }
    let (after, s) = recovered.expect("a full-quality frame followed without input");
    assert!(after <= Duration::from_millis(100), "{after:?}");
    // That frame shaped every visible cell: no cap outside fast-scroll.
    assert!(s.shapes > FAST_SHAPES_PER_FRAME, "{s:?}");
    // And once it is painted, nothing asks for more frames.
    let pending = vcx.update(|window, cx| window.simulate_next_frame(cx));
    assert_eq!(pending, 0);
}

#[gpui::test]
fn cache_hits_allocate_nothing(cx: &mut TestAppContext) {
    let rows: Vec<Vec<&str>> = (0..60)
        .map(|r| match r % 3 {
            0 => vec![
                "İstanbul",
                "a\tb",
                "NULL",
                "0123456789012345678901234567890123456789",
            ],
            1 => vec!["Kadıköy", "x\u{202E}y", "", "short"],
            _ => vec!["Ankara", "plain", "2026-11-14 10:00:00", "z"],
        })
        .collect();
    let rows: Vec<&[&str]> = rows.iter().map(|r| r.as_slice()).collect();
    let (view, vcx) = open(
        cx,
        Rc::new(FixtureSource::texts(&rows, 140.0)),
        900.0,
        700.0,
    );
    // The first frames shaped the screen (the counter is live: misses count).
    let warm = vcx.read(|cx| *view.read(cx).frame_log());
    assert!(warm.max_shapes > 0 && warm.cell_allocs > 0, "{warm:?}");
    let cached = view.read_with(vcx, |v, _| v.cached_shapes());
    vcx.update(|_, cx| view.update(cx, |v, _| v.reset_frame_log()));
    for _ in 0..3 {
        frame(vcx);
        let s = view.read_with(vcx, |v, _| v.frame_stats());
        assert_eq!(s.cells, warm.last.cells);
        assert_eq!((s.shapes, s.cell_allocs), (0, 0), "{s:?}");
    }
    let log = vcx.read(|cx| *view.read(cx).frame_log());
    assert_eq!((log.frames, log.cell_allocs, log.max_shapes), (3, 0, 0));
    assert_eq!(view.read_with(vcx, |v, _| v.cached_shapes()), cached);
}

#[gpui::test]
fn painted_runs_are_recorded_as_data_and_catalog(cx: &mut TestAppContext) {
    let source = FixtureSource::texts(&[&["alice", "bob"], &["carol", "dave"]], 120.0);
    let (_view, vcx) = open(cx, Rc::new(source), 600.0, 300.0);
    vcx.update(|window, cx| {
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        });
        window.refresh();
    });
    vcx.run_until_parked();
    let frame = vcx.read(|cx| cx.global::<TextRegistry>().frame.clone());
    let source_of = |t: &str| {
        frame
            .iter()
            .find(|r| r.text.as_ref() == t)
            .map(|r| r.source)
    };
    assert_eq!(source_of("carol"), Some(TextSource::Data));
    assert_eq!(source_of("c1"), Some(TextSource::Catalog));
    assert_eq!(source_of("text"), Some(TextSource::Catalog));
    assert_eq!(source_of("2"), Some(TextSource::Data));
    assert!(
        frame.iter().all(|r| !r.clipped && r.size_px >= 11.0),
        "{frame:?}"
    );
}

#[gpui::test]
fn markers_paint_for_null_and_empty(cx: &mut TestAppContext) {
    let columns = vec![hacivat_ui_grid::testing::text_column(0, "v", 160.0)];
    let rows = vec![
        vec![cell("NULL", CellKind::Null, Align::Start)],
        vec![cell("", CellKind::Empty, Align::Start)],
        vec![cell("NULL", CellKind::Text, Align::Start)],
    ];
    let (view, vcx) = open(cx, Rc::new(FixtureSource::new(columns, rows)), 400.0, 300.0);
    vcx.update(|_, cx| view.update(cx, |v, _| v.set_paint_recording(true)));
    frame(vcx);
    let cells = vcx.read(|cx| view.read(cx).painted_cells().to_vec());
    let texts: Vec<_> = cells
        .iter()
        .map(|c| (c.row, c.col, c.text.to_string()))
        .collect();
    assert_eq!(
        texts,
        [
            (0, ColId(0), "NULL".to_string()),
            (1, ColId(0), "''".to_string()),
            (2, ColId(0), "NULL".to_string()),
        ]
    );
}

#[cfg(feature = "scene-check")]
#[gpui::test]
fn eleven_px_floor_no_cp_fill_and_snapped_hairlines(cx: &mut TestAppContext) {
    use hacivat_ui_core::theme::DARK;
    let rows: Vec<Vec<String>> = (0..80)
        .map(|r| vec![format!("name {r}"), r.to_string()])
        .collect();
    let rows: Vec<Vec<&str>> = rows
        .iter()
        .map(|r| r.iter().map(String::as_str).collect())
        .collect();
    let rows: Vec<&[&str]> = rows.iter().map(|r| r.as_slice()).collect();
    let (view, vcx) = open(
        cx,
        Rc::new(FixtureSource::texts(&rows, 137.3)),
        700.0,
        400.0,
    );
    // A fractional scroll puts every row edge between device pixels.
    vcx.update(|_, cx| view.update(cx, |v, cx| v.scroll_px(7.3, 0.0, cx)));
    vcx.update(|_, cx| {
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        })
    });
    let seam_soft = gpui::Rgba::from(DARK.colors.seam_soft.to_hsla());
    let near = |a: f32, b: f32| (a - b).abs() <= 1.0 / 255.0;
    for scale in [1.0f32, 1.5, 2.0] {
        vcx.simulate_scale_factor_change(scale);
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
        let texts = vcx.read(|cx| cx.global::<TextRegistry>().frame.clone());
        assert!(!texts.is_empty());
        for t in &texts {
            assert!(t.size_px >= 11.0, "{scale}× {t:?}");
        }
        vcx.update(|window, _| {
            assert_eq!(
                hacivat_ui_core::named_rules::count_solid_quads(
                    window,
                    DARK.colors.cp_fill.to_hsla()
                ),
                0,
                "{scale}×: no cp_fill in the grid"
            );
            let mut hairlines = 0;
            for q in &window.rendered_scene().quads {
                let Some(fill) = q.background.as_solid() else {
                    continue;
                };
                let c = gpui::Rgba::from(fill);
                if !(near(c.r, seam_soft.r) && near(c.g, seam_soft.g) && near(c.b, seam_soft.b)) {
                    continue;
                }
                hairlines += 1;
                let b = q.bounds;
                for v in [b.origin.x.0, b.origin.y.0, b.size.width.0, b.size.height.0] {
                    assert!(
                        (v - v.round()).abs() < 1e-3,
                        "{scale}×: {b:?} is off the pixel grid"
                    );
                }
                // Every hairline is the same whole number of device pixels thick, wherever the
                // scroll put it: 1 logical px rounded to the device grid.
                let thickness = b.size.width.0.min(b.size.height.0);
                assert_eq!(thickness, scale.round(), "{scale}×: {b:?}");
            }
            assert!(hairlines >= 20, "{scale}×: {hairlines} hairlines");
        });
    }
}
