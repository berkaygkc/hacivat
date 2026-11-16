//! M3a Task 29: the large-file tiers (UI-EDIT-23 T0/T1, D-071): the choice by size and line
//! length, the 80 % rule that keeps a document from flapping between tiers, the T1 banner and
//! the status segment, and the editor's accessible description in T1. Tests wait on conditions
//! (`run_until_parked`); the one wall-clock check is a perf test (`HACIVAT_PERF=1`).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    TestAppContext, VisualTestContext, Window, div,
};
use hacivat_ui::l10n::{EnglishDefaults, UiText};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use hacivat_ui_editor::large_file::{self, SizeText, TierState};
use hacivat_ui_editor::{
    DocId, EditorConfig, EditorEvent, EditorView, FailClosedService, Tier, TierLimits,
};

const MB: u64 = 1 << 20;

fn tier(bytes: u64, longest_line: u64) -> Tier {
    large_file::tier_for(bytes, longest_line, &TierLimits::default())
}

#[test]
fn tier_choice() {
    assert_eq!(tier(3 << 20, 60_000), Tier::Full);
    assert_eq!(tier(12 << 20, 1_000), Tier::Reduced);
    assert_eq!(tier(15 << 20, 15 << 20), Tier::Reduced);
    // The limits are inclusive: 4 MB is still T0, one byte more is T1.
    assert_eq!(tier(4 << 20, 100), Tier::Full);
    assert_eq!(tier((4 << 20) + 1, 100), Tier::Reduced);
    // A small document with a line past 64 KB is T1 (UI-EDIT-23 step 1).
    assert_eq!(tier(200_000, 64 << 10), Tier::Full);
    assert_eq!(tier(200_000, (64 << 10) + 1), Tier::Reduced);
    // Nothing above 20 MB enters the editor in M3a (R-M3-49), and there is no T2 until M4b:
    // a document that got there anyway stays in reduced mode.
    assert_eq!(tier(30 << 20, 100), Tier::Reduced);
}

/// The tier after each size, from T0, with short lines.
fn walk(sizes: &[u64]) -> Vec<Tier> {
    let limits = TierLimits::default();
    let mut s = large_file::open(0, 0, &limits);
    sizes
        .iter()
        .map(|&b| {
            large_file::update(&mut s, b, 100, &limits);
            s.tier
        })
        .collect()
}

#[test]
fn tiers_do_not_flap_near_the_limit() {
    let limit = 4 * MB;
    // T0 returns only strictly below 80 % of 4 MB (3,355,443.2 bytes).
    let back = limit * 80 / 100;
    use Tier::{Full, Reduced};
    assert_eq!(
        walk(&[limit, limit + 1, limit, limit - 1, back + 2, back + 1, back]),
        [Full, Reduced, Reduced, Reduced, Reduced, Reduced, Full]
    );
    // Typing back and forth across the limit switches once each way, not per keystroke.
    let mut sizes = Vec::new();
    for _ in 0..50 {
        sizes.extend([limit + 1, limit]);
    }
    let tiers = walk(&sizes);
    assert!(tiers.iter().all(|t| *t == Reduced));
    // `update` says when the tier moved, and only then.
    let limits = TierLimits::default();
    let mut s = large_file::open(limit, 100, &limits);
    assert_eq!(s.tier, Full);
    assert_eq!(
        large_file::update(&mut s, limit + 1, 100, &limits),
        Some(Reduced)
    );
    assert_eq!(large_file::update(&mut s, limit + 2, 100, &limits), None);
    assert_eq!(large_file::update(&mut s, limit, 100, &limits), None);
    assert_eq!(
        large_file::update(&mut s, back - 1, 100, &limits),
        Some(Full)
    );
    assert_eq!(s.bytes, back - 1, "the state keeps the size for the banner");
}

#[test]
fn the_long_line_rule_has_the_same_margin() {
    let limits = TierLimits::default();
    let long = 64 << 10;
    let mut s = large_file::open(100_000, long + 1, &limits);
    assert_eq!(s.tier, Tier::Reduced);
    // Not below 80 % of 64 KB (52,428.8 bytes): stays.
    assert_eq!(large_file::update(&mut s, 100_000, long, &limits), None);
    assert_eq!(
        large_file::update(&mut s, 100_000, long * 80 / 100 + 1, &limits),
        None
    );
    assert_eq!(
        large_file::update(&mut s, 100_000, long * 80 / 100, &limits),
        Some(Tier::Full)
    );
}

// ## m3a-t29 fix round 1 (M1): the 80 % margin applies to the limits that caused T1.

/// A document that entered T1 by its size, with a line of 55 KB (legal in T0, above 80 % of
/// 64 KB), returns to T0 once the size is below 80 %.
#[test]
fn a_size_entered_t1_with_a_long_legal_line_returns() {
    let limits = TierLimits::default();
    let mut s = large_file::open(5 * MB, 55_000, &limits);
    assert_eq!((s.tier, s.by_size, s.by_line), (Tier::Reduced, true, false));
    assert_eq!(
        large_file::update(&mut s, 3 * MB, 55_000, &limits),
        Some(Tier::Full)
    );
    assert_eq!(
        (s.by_size, s.by_line),
        (false, false),
        "the cause clears in T0"
    );
}

/// The causes accumulate while in T1, and each needs its margin; a limit that did not cause T1
/// needs only to be within its plain limit.
#[test]
fn every_cause_needs_its_margin() {
    let limits = TierLimits::default();
    let long = 64 << 10;
    let mut s = large_file::open(5 * MB, 100, &limits);
    assert_eq!((s.by_size, s.by_line), (true, false));
    assert_eq!(large_file::update(&mut s, 5 * MB, long + 1, &limits), None);
    assert_eq!((s.by_size, s.by_line), (true, true));
    // Size below 80 %, the line legal but not below 80 % of 64 KB: stays.
    assert_eq!(large_file::update(&mut s, 3 * MB, 60_000, &limits), None);
    assert_eq!(
        large_file::update(&mut s, 3 * MB, 50_000, &limits),
        Some(Tier::Full)
    );
    // A line-entered T1 at 3.5 MB (within the size limit, above its 80 %) returns when the line
    // is below its margin, whatever the size.
    let mut s = large_file::open(3 * MB + MB / 2, long + 1, &limits);
    assert_eq!((s.tier, s.by_size, s.by_line), (Tier::Reduced, false, true));
    assert_eq!(
        large_file::update(&mut s, 3 * MB + MB / 2, 50_000, &limits),
        Some(Tier::Full)
    );
}

/// A window whose root holds one editor.
struct Host {
    editor: Entity<EditorView>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(gpui::px(900.0))
            .h(gpui::px(600.0))
            .child(self.editor.clone())
    }
}

fn open(cx: &mut TestAppContext, text: String) -> (Entity<EditorView>, &mut VisualTestContext) {
    cx.update(|cx| {
        Theme::install(cx, ThemeMode::Dark);
        cx.set_global(UiText(Arc::new(EnglishDefaults::new())));
    });
    let slot: Rc<RefCell<Option<Entity<EditorView>>>> = Rc::default();
    let out = slot.clone();
    let (_, vcx) = cx.add_window_view(move |window, cx| {
        let editor = cx.new(|cx| {
            EditorView::new(
                DocId(29),
                &text,
                EditorConfig::default(),
                Rc::new(FailClosedService),
                Rc::new(|_: &gpui::App| "Console 1".into()),
                window,
                cx,
            )
        });
        *out.borrow_mut() = Some(editor.clone());
        Host { editor }
    });
    let editor = slot.borrow().clone().unwrap();
    draw(vcx);
    (editor, vcx)
}

fn draw(vcx: &mut VisualTestContext) {
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
}

/// `bytes` of short SQL lines.
fn sql_of(bytes: u64) -> String {
    let line = "SELECT col FROM t;\n";
    line.repeat(usize::try_from(bytes).unwrap() / line.len() + 1)
}

/// Fluent's bidi isolation marks removed (the English defaults wrap placeables in them).
fn plain(s: &str) -> String {
    s.replace(['\u{2068}', '\u{2069}'], "")
}

fn state(e: &Entity<EditorView>, vcx: &VisualTestContext) -> TierState {
    vcx.read(|cx| *large_file::state(e.read(cx)))
}

fn description(e: &Entity<EditorView>, vcx: &VisualTestContext) -> Option<String> {
    vcx.read(|cx| large_file::description(large_file::state(e.read(cx)), cx))
        .map(|d| plain(&d))
}

#[gpui::test]
fn t1_shows_the_banner_the_status_and_the_description(cx: &mut TestAppContext) {
    let (e, vcx) = open(cx, sql_of(5 * MB));
    assert_eq!(state(&e, vcx).tier, Tier::Reduced);
    assert_eq!(vcx.read(|cx| e.read(cx).status().tier), Tier::Reduced);
    let banner = vcx.debug_bounds("editor-t1-banner").expect("the T1 banner");
    // The banner sits above the text: the caret on the first line is below it.
    let caret = vcx
        .read(|cx| e.read(cx).caret_bounds())
        .expect("a drawn caret");
    assert!(banner.bottom() <= caret.top(), "{banner:?} {caret:?}");
    assert_eq!(
        description(&e, vcx).as_deref(),
        Some(
            "This document is large (5.0 MB), so the editor is in reduced mode. \
             Reduced mode keeps typing fast. It ends when the document is smaller than 3.2 MB."
        )
    );
    // Shrinking below 80 % of 4 MB ends reduced mode: the banner goes, and the host hears it.
    let events: Rc<RefCell<Vec<String>>> = Rc::default();
    let seen = events.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&e, move |_, ev: &EditorEvent, _| {
            if let EditorEvent::TierChanged(t) = ev {
                seen.borrow_mut().push(format!("{t:?}"));
            }
        })
        .detach();
    });
    e.update(vcx, |v, cx| v.set_text(&sql_of(3 * MB), cx));
    draw(vcx);
    assert_eq!(*events.borrow(), ["Full"]);
    assert_eq!(state(&e, vcx).tier, Tier::Full);
    assert!(vcx.debug_bounds("editor-t1-banner").is_none());
    assert_eq!(description(&e, vcx), None);
}

#[gpui::test]
fn t0_has_no_banner(cx: &mut TestAppContext) {
    let (e, vcx) = open(cx, sql_of(MB));
    assert_eq!(state(&e, vcx).tier, Tier::Full);
    assert_eq!(
        state(&e, vcx).bytes,
        vcx.read(|cx| e.read(cx).snapshot().buffer.len_bytes()) as u64
    );
    assert!(vcx.debug_bounds("editor-t1-banner").is_none());
}

#[gpui::test]
fn a_long_line_says_why_without_a_size(cx: &mut TestAppContext) {
    let text = format!("SELECT '{}';\n", "x".repeat(70_000));
    let (e, vcx) = open(cx, text);
    assert_eq!(state(&e, vcx).tier, Tier::Reduced);
    assert!(vcx.debug_bounds("editor-t1-banner").is_some());
    // ## m3a-f4-ime (EDIT-2): the long-line copy says when it ends, as the size copy does.
    assert_eq!(
        description(&e, vcx).as_deref(),
        Some(
            "This document has a very long line, so the editor is in reduced mode. \
             Reduced mode keeps typing fast. It ends when every line is shorter than 51 KB."
        )
    );
}

/// M1: the copy follows the cause. A 3.5 MB document in T1 because of a long line says so (not
/// "it ends below 3.2 MB", which shrinking would not make true).
#[gpui::test]
fn the_copy_follows_the_cause(cx: &mut TestAppContext) {
    let mut text = sql_of(3 * MB + MB / 2);
    text.push_str(&format!("SELECT '{}';\n", "x".repeat(70_000)));
    let (e, vcx) = open(cx, text);
    assert_eq!(state(&e, vcx).tier, Tier::Reduced);
    assert_eq!(
        description(&e, vcx).as_deref(),
        Some(
            "This document has a very long line, so the editor is in reduced mode. \
             Reduced mode keeps typing fast. It ends when every line is shorter than 51 KB."
        )
    );
}

#[gpui::test]
fn the_host_formats_the_sizes(cx: &mut TestAppContext) {
    let (e, vcx) = open(cx, sql_of(5 * MB));
    // The app formats sizes in the format locale (L3); the editor only asks.
    vcx.update(|_, cx| {
        cx.set_global(SizeText(Rc::new(|bytes: u64, _: &gpui::App| {
            format!("<{bytes}>").into()
        })))
    });
    let bytes = state(&e, vcx).bytes;
    let back = (4 * MB) * 80 / 100;
    assert_eq!(
        description(&e, vcx).as_deref(),
        Some(
            format!(
                "This document is large (<{bytes}>), so the editor is in reduced mode. \
                 Reduced mode keeps typing fast. It ends when the document is smaller than <{back}>."
            )
            .as_str()
        )
    );
}

/// Typing in a 19 MB document stays within the B3 budget: each keystroke's commit and frame
/// take under 16 ms at the 95th percentile (release build, `HACIVAT_PERF=1`).
#[gpui::test]
#[ignore = "perf"]
fn typing_in_reduced_mode_stays_within_budget(cx: &mut TestAppContext) {
    if std::env::var_os("HACIVAT_PERF").is_none() {
        return;
    }
    let (e, vcx) = open(cx, sql_of(19 * MB));
    assert_eq!(state(&e, vcx).tier, Tier::Reduced);
    let mut took = Vec::new();
    for i in 0..200 {
        let at = std::time::Instant::now();
        e.update_in(vcx, |v, window, cx| {
            use gpui::EntityInputHandler as _;
            v.replace_text_in_range(None, if i % 10 == 9 { "\n" } else { "x" }, window, cx);
        });
        draw(vcx);
        took.push(at.elapsed());
    }
    took.sort();
    let p95 = took[took.len() * 95 / 100];
    assert!(p95.as_secs_f64() < 0.016, "p95 {p95:?}");
}

// ---------------------------------------------------------------------------------------------
// ## m3a-f4-ime (PO finding EDIT-2): a long-line T1 ends once its cause is gone.

/// Replaces the UTF-16 range `r` (ASCII text: bytes) with `text` through the input handler, as
/// typing over a selection does.
fn replace(
    e: &Entity<EditorView>,
    vcx: &mut VisualTestContext,
    r: std::ops::Range<usize>,
    text: &str,
) {
    e.update_in(vcx, |v, window, cx| {
        use gpui::EntityInputHandler as _;
        v.replace_text_in_range(Some(r), text, window, cx);
    });
}

/// Records the host's `TierChanged` events.
fn tier_events(e: &Entity<EditorView>, vcx: &mut VisualTestContext) -> Rc<RefCell<Vec<Tier>>> {
    let events: Rc<RefCell<Vec<Tier>>> = Rc::default();
    let seen = events.clone();
    vcx.update(|_, cx| {
        cx.subscribe(e, move |_, ev: &EditorEvent, _| {
            if let EditorEvent::TierChanged(t) = ev {
                seen.borrow_mut().push(*t);
            }
        })
        .detach();
    });
    events
}

/// Lets the text rest past the re-measure delay.
fn rest(vcx: &mut VisualTestContext) {
    vcx.executor()
        .advance_clock(large_file::REMEASURE_AFTER + std::time::Duration::from_millis(10));
    draw(vcx);
}

/// The PO's repro: Select All and Delete in a long-line T1 leaves an empty document, and the
/// banner, the status segment and the host's tier leave with the text (at once: no line is
/// longer than the document).
#[gpui::test]
fn emptying_a_long_line_document_ends_reduced_mode(cx: &mut TestAppContext) {
    let text = format!("SELECT '{}';\n", "x".repeat(70_000));
    let len = text.len();
    let (e, vcx) = open(cx, text);
    assert_eq!(state(&e, vcx).tier, Tier::Reduced);
    let events = tier_events(&e, vcx);
    replace(&e, vcx, 0..len, "");
    draw(vcx);
    assert_eq!(state(&e, vcx).tier, Tier::Full);
    assert_eq!(vcx.read(|cx| e.read(cx).status().tier), Tier::Full);
    assert!(vcx.debug_bounds("editor-t1-banner").is_none());
    assert_eq!(description(&e, vcx), None);
    assert_eq!(*events.borrow(), [Tier::Full]);
}

/// Deleting the long line from a larger document, or splitting it into short lines, ends T1
/// once the text has rested (the longest line is measured again off the UI thread, never per
/// keystroke); a line still above 80 % of 64 KB keeps it (the hysteresis).
#[gpui::test]
fn removing_or_splitting_the_long_line_ends_reduced_mode_after_a_rest(cx: &mut TestAppContext) {
    let head = sql_of(MB);
    let long = format!("SELECT '{}';\n", "x".repeat(70_000));
    let at = head.len();
    let (e, vcx) = open(cx, format!("{head}{long}{head}"));
    assert_eq!(state(&e, vcx).tier, Tier::Reduced);
    let events = tier_events(&e, vcx);
    // Shortened to 60 KB: legal in T0, but above 80 % of 64 KB.
    replace(&e, vcx, at + 8..at + 8 + 10_000, "");
    draw(vcx);
    assert_eq!(state(&e, vcx).tier, Tier::Reduced, "not per keystroke");
    rest(vcx);
    assert_eq!(state(&e, vcx).tier, Tier::Reduced, "the margin holds");
    assert!(events.borrow().is_empty());
    // Split in two 30 KB lines: both below the margin.
    replace(&e, vcx, at + 30_000..at + 30_000, "\n");
    draw(vcx);
    assert_eq!(state(&e, vcx).tier, Tier::Reduced, "not per keystroke");
    rest(vcx);
    assert_eq!(state(&e, vcx).tier, Tier::Full);
    assert!(vcx.debug_bounds("editor-t1-banner").is_none());
    assert_eq!(*events.borrow(), [Tier::Full]);
    // A long line again, then deleted outright: back to T1, then out after a rest.
    replace(&e, vcx, at..at, &long);
    draw(vcx);
    assert_eq!(state(&e, vcx).tier, Tier::Reduced);
    replace(&e, vcx, at..at + long.len(), "");
    rest(vcx);
    assert_eq!(state(&e, vcx).tier, Tier::Full);
    assert_eq!(*events.borrow(), [Tier::Full, Tier::Reduced, Tier::Full]);
}

/// An edit while a re-measure waits restarts the wait (the waiting measure is dropped), so a
/// long line added meanwhile keeps T1. (`after_edit`'s version check is a second guard that
/// this path does not reach: every text change replaces the waiting task first; review M2.)
#[gpui::test]
fn an_edit_restarts_the_remeasure_wait(cx: &mut TestAppContext) {
    let long = format!("SELECT '{}';\n", "x".repeat(70_000));
    let (e, vcx) = open(cx, format!("{long}SELECT 1;\n"));
    // Split the long line (both halves short), then, before the rest is over, add a long line.
    replace(&e, vcx, 30_000..30_000, "\n");
    vcx.executor()
        .advance_clock(large_file::REMEASURE_AFTER / 2);
    replace(&e, vcx, 0..0, &long);
    rest(vcx);
    assert_eq!(
        state(&e, vcx).tier,
        Tier::Reduced,
        "the newer long line counts"
    );
}

/// Both causes: the copy names both exits.
#[gpui::test]
fn both_causes_name_both_exits(cx: &mut TestAppContext) {
    let mut text = sql_of(5 * MB);
    text.push_str(&format!("SELECT '{}';\n", "x".repeat(70_000)));
    let (e, vcx) = open(cx, text);
    assert_eq!(
        description(&e, vcx).as_deref(),
        Some(
            "This document is large (5.1 MB), so the editor is in reduced mode. \
             Reduced mode keeps typing fast. It ends when the document is smaller than 3.2 MB \
             and every line is shorter than 51 KB."
        )
    );
}

// ## m3a-f4-ime (PO finding EDIT-3): the T1 notice sits above the code, never under it.

/// Scrolled by part of a row, the first row starts above the code area. The editor element
/// paints inside a content mask of its own bounds, so nothing of that row (its line number
/// above all, which the gutter draws unclipped) reaches the notice above it. The test platform
/// draws no glyphs, so the check reads the mask on the element's own ground quad; the shots
/// (`notice-reduced-mode`) show the glyphs.
#[cfg(feature = "scene-check")]
#[gpui::test]
fn a_part_row_never_paints_over_the_t1_notice(cx: &mut TestAppContext) {
    let mut text = "SELECT col FROM t;\n".repeat(400);
    text.push_str(&format!("SELECT '{}';\n", "x".repeat(70_000)));
    let (e, vcx) = open(cx, text);
    assert_eq!(state(&e, vcx).tier, Tier::Reduced);
    // ## m3b-t24-exec-ui (named exception, the T24 fix round 2): `ScrollWheelEvent` is now
    // `ScrollEvent` (the compiler's deprecation note); workspace clippy with `--all-features`.
    vcx.simulate_event(gpui::ScrollEvent {
        position: gpui::point(gpui::px(450.0), gpui::px(400.0)),
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.0), gpui::px(-14.0))),
        ..Default::default()
    });
    draw(vcx);
    let banner = vcx.debug_bounds("editor-t1-banner").expect("the T1 notice");
    let (first_top, area_top) = vcx.read(|cx| {
        let rows = *e.read(cx).last_layout().unwrap().rows();
        (rows.first_y, rows.bounds.top())
    });
    assert!(
        first_top < area_top,
        "a part row: {first_top:?} {area_top:?}"
    );
    let ground = vcx.read(|cx| Theme::get(cx).tokens.colors.panel.to_hsla());
    // The element's ground: the tall quad of the editor's ground under the notice, and the
    // top of the content mask it was painted in.
    let masks: Vec<(f32, f32)> = vcx.update(|window, _| {
        let scale = window.scale_factor();
        window
            .rendered_scene()
            .quads
            .iter()
            .filter(|q| q.background.as_solid() == Some(ground))
            .filter(|q| q.bounds.origin.y.0 / scale >= f32::from(banner.bottom()))
            .filter(|q| q.bounds.size.height.0 / scale > 100.0)
            .map(|q| {
                (
                    q.bounds.origin.y.0 / scale,
                    q.content_mask.bounds.origin.y.0 / scale,
                )
            })
            .collect()
    });
    assert_eq!(masks.len(), 1, "the editor's ground: {masks:?}");
    let (top, mask_top) = masks[0];
    assert!(
        mask_top >= top - 0.5,
        "the editor paints in a mask of its bounds (top {top}), not one from {mask_top}"
    );
    assert!(mask_top >= f32::from(banner.bottom()));
}
