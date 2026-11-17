//! M3c Task 33: find in the editor view (UI-EDIT-22 without replace; R-M3-10). The matcher is
//! the app's (here a literal one); the editor counts on the background executor in batches,
//! steps and wraps, selects every match up to a cap, opens a fold a step lands in, puts the
//! scrollbar's ticks by visible row, and drags the horizontal thumb. Tests wait on conditions
//! (`run_until_parked`), never on time.
#![allow(clippy::single_range_in_vec_init, reason = "line ranges as data")]

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gpui::{
    AppContext as _, Context, Entity, Focusable as _, IntoElement, Modifiers, MouseButton,
    ParentElement as _, Render, Styled as _, TestAppContext, VisualTestContext, Window, div, point,
    px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use hacivat_ui_editor::commands::status_message;
use hacivat_ui_editor::fold_gutter::fold_range;
use hacivat_ui_editor::{DocId, EditorConfig, EditorEvent, EditorView, FailClosedService};
use hacivat_ui_editor::{FindMatcher, FindStatus};

struct Host {
    editor: Entity<EditorView>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(800.0)).h(px(600.0)).child(self.editor.clone())
    }
}

/// A case-sensitive literal matcher that counts its calls.
struct Lit {
    needle: &'static str,
    calls: Arc<AtomicUsize>,
}

impl Lit {
    fn new(needle: &'static str) -> Arc<Self> {
        Arc::new(Lit {
            needle,
            calls: Arc::default(),
        })
    }
}

impl FindMatcher for Lit {
    fn find(&self, hay: &str, from: usize) -> Option<Range<usize>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let i = hay.get(from..)?.find(self.needle)?;
        Some(from + i..from + i + self.needle.len())
    }
    fn find_back(&self, hay: &str, before: usize) -> Option<Range<usize>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let i = hay.get(..before)?.rfind(self.needle)?;
        Some(i..i + self.needle.len())
    }
}

struct Opened<'a> {
    editor: Entity<EditorView>,
    events: Rc<RefCell<Vec<EditorEvent>>>,
    vcx: &'a mut VisualTestContext,
}

fn draw(vcx: &mut VisualTestContext) {
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
}

/// Opens `text` with a caret at `caret`, focused and drawn.
fn open<'a>(cx: &'a mut TestAppContext, text: &str, caret: usize) -> Opened<'a> {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let text = text.to_string();
    let slot: Rc<RefCell<Option<Entity<EditorView>>>> = Rc::default();
    let out = slot.clone();
    let (_, vcx) = cx.add_window_view(move |window, cx| {
        let editor = cx.new(|cx| {
            EditorView::new(
                DocId(33),
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
    let events: Rc<RefCell<Vec<EditorEvent>>> = Rc::default();
    let seen = events.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&editor, move |_, ev: &EditorEvent, _| {
            seen.borrow_mut().push(ev.clone());
        })
        .detach();
    });
    vcx.update(|window, _| window.activate_window());
    editor.update_in(vcx, |v, window, cx| {
        let f = v.focus_handle(cx);
        window.focus(&f, cx);
        v.select_range(caret..caret, cx);
    });
    draw(vcx);
    events.borrow_mut().clear();
    Opened {
        editor,
        events,
        vcx,
    }
}

fn status(o: &Opened<'_>) -> FindStatus {
    o.vcx.read(|cx| o.editor.read(cx).find_status())
}

fn main_range(o: &Opened<'_>) -> Range<usize> {
    o.vcx
        .read(|cx| o.editor.read(cx).snapshot().selection.main().range())
}

fn set(o: &mut Opened<'_>, m: Arc<Lit>) {
    o.editor.update(o.vcx, |v, cx| v.set_find(Some(m), cx));
    o.vcx.run_until_parked();
}

fn step(o: &mut Opened<'_>, back: bool) -> Option<Range<usize>> {
    let r = o.editor.update(o.vcx, |v, cx| v.find_step(back, cx));
    o.vcx.run_until_parked();
    r
}

#[gpui::test]
fn steps_select_matches_and_wrap(cx: &mut TestAppContext) {
    let mut o = open(cx, "a x a x a", 2);
    set(&mut o, Lit::new("a"));
    let s = status(&o);
    assert_eq!((s.total, s.counted, s.active), (Some(3), 3, true));
    assert_eq!(s.current, None, "the caret is not on a match");
    assert_eq!(step(&mut o, false), Some(4..5));
    assert_eq!(main_range(&o), 4..5);
    assert_eq!(status(&o).current, Some(2));
    assert_eq!(step(&mut o, false), Some(8..9));
    assert_eq!(step(&mut o, false), Some(0..1), "wraps to the first");
    assert_eq!(status(&o).current, Some(1));
    assert_eq!(step(&mut o, true), Some(8..9), "back wraps to the last");
    assert!(
        o.events
            .borrow()
            .iter()
            .any(|e| matches!(e, EditorEvent::FindChanged)),
        "the host hears of the steps"
    );
    // Cleared: nothing active, steps find nothing.
    o.editor.update(o.vcx, |v, cx| v.set_find(None, cx));
    assert_eq!(status(&o), FindStatus::default());
    assert_eq!(step(&mut o, false), None);
}

#[gpui::test]
fn an_edit_recounts(cx: &mut TestAppContext) {
    let mut o = open(cx, "ab ab", 0);
    set(&mut o, Lit::new("ab"));
    assert_eq!(status(&o).total, Some(2));
    o.editor.update(o.vcx, |v, cx| v.select_range(5..5, cx));
    o.vcx.simulate_input(" ab");
    o.vcx.run_until_parked();
    assert_eq!(status(&o).total, Some(3));
}

#[gpui::test]
fn select_all_matches_caps_at_10000(cx: &mut TestAppContext) {
    let text = "ab ".repeat(10_500);
    let mut o = open(cx, &text, 0);
    set(&mut o, Lit::new("ab"));
    assert_eq!(status(&o).total, Some(10_500));
    let n = o
        .editor
        .update(o.vcx, |v, cx| v.select_all_matches(10_000, cx));
    assert_eq!(n, 10_000);
    let ranges = o
        .vcx
        .read(|cx| o.editor.read(cx).snapshot().selection.ranges().to_vec());
    assert_eq!(ranges.len(), 10_000);
    assert_eq!(ranges[0].range(), 0..2);
    assert_eq!(ranges[9_999].range(), 29_997..29_999);
    // Past the cap it says how many of how many (no provider here: the message id).
    let said = o.vcx.read(|cx| status_message(o.editor.read(cx)));
    assert_eq!(said.as_deref(), Some("hv-find-capped"));
    // Under the cap nothing is said.
    let small = open_more(o.vcx);
    let n = small
        .editor
        .update(small.vcx, |v, cx| v.select_all_matches(10_000, cx));
    assert_eq!(n, 2);
    let said = small.vcx.read(|cx| status_message(small.editor.read(cx)));
    assert_eq!(said, None);
}

/// A second editor in the same app ("ab ab", the query set).
fn open_more(vcx: &mut VisualTestContext) -> Opened<'_> {
    let editor = vcx.update(|window, cx| {
        cx.new(|cx| {
            EditorView::new(
                DocId(34),
                "ab ab",
                EditorConfig::default(),
                Rc::new(FailClosedService),
                Rc::new(|_: &gpui::App| "Console 2".into()),
                window,
                cx,
            )
        })
    });
    editor.update(vcx, |v, cx| v.set_find(Some(Lit::new("ab")), cx));
    vcx.run_until_parked();
    Opened {
        editor,
        events: Rc::default(),
        vcx,
    }
}

#[gpui::test]
fn counting_never_blocks_the_ui_thread(cx: &mut TestAppContext) {
    // 5 MiB of text with a match every 16 bytes: five count batches at least.
    let text = "x SELECT 1; -- \n".repeat(5 << 16);
    let o = open(cx, &text, 0);
    let m = Lit::new("SELECT");
    let calls = m.calls.clone();
    let during = o.editor.update(o.vcx, |v, cx| {
        v.set_find(Some(m), cx);
        v.find_status()
    });
    // Setting the query searched nothing on the UI thread: the count is pending.
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        (during.total, during.counted, during.active),
        (None, 0, true)
    );
    o.vcx.run_until_parked();
    let s = status(&o);
    // More than the 100,000 kept: the count goes on past the cap.
    assert_eq!(
        (s.total, s.counted, s.capped),
        (Some(5 << 16), 5 << 16, true)
    );
    // The count came back in batches: the host heard several partial counts.
    let changes = o
        .events
        .borrow()
        .iter()
        .filter(|e| matches!(e, EditorEvent::FindChanged))
        .count();
    assert!(changes >= 4, "{changes} updates");
}

#[gpui::test]
fn a_newer_query_drops_the_older_count(cx: &mut TestAppContext) {
    let text = "ab cd ".repeat(100_000);
    let o = open(cx, &text, 0);
    o.editor.update(o.vcx, |v, cx| {
        v.set_find(Some(Lit::new("ab")), cx);
        v.set_find(Some(Lit::new("cd x")), cx);
    });
    o.vcx.run_until_parked();
    assert_eq!(status(&o).total, Some(0));
}

fn run_cmd(o: &mut Opened<'_>, id: &str) {
    let ran = o
        .editor
        .update_in(o.vcx, |v, window, cx| v.run_command(id, window, cx));
    assert!(ran, "{id}");
    o.vcx.run_until_parked();
}

// ## m3c-t33-find (fix round 1, I1, I3)
/// Find as you type (`find_from`) and a step while the count runs search nothing on the UI
/// thread: the step waits for the background and lands as one selection change (one
/// undo-selection step).
#[gpui::test]
fn find_from_searches_off_the_ui_thread_and_commits_once(cx: &mut TestAppContext) {
    let text = "x SELECT 1; -- \n".repeat(1 << 14);
    let mut o = open(cx, &text, 5);
    let m = Lit::new("SELECT");
    let calls = m.calls.clone();
    let (from, step) = o.editor.update(o.vcx, |v, cx| {
        v.set_find(Some(m), cx);
        (v.find_from(5..5, cx), v.find_step(false, cx))
    });
    assert_eq!((from, step), (None, None), "both wait for the background");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "nothing searched on the UI thread"
    );
    assert_eq!(main_range(&o), 5..5);
    o.vcx.run_until_parked();
    assert_eq!(main_range(&o), 18..24, "the first match after the origin");
    assert_eq!(o.vcx.read(|cx| o.editor.read(cx).find_made()), Some(18..24));
    // One selection change: one undo-selection step back to the origin.
    run_cmd(&mut o, "editor.undo_selection");
    assert_eq!(main_range(&o), 5..5);
}

/// M8: select all while the count runs searches nothing on the UI thread; the selection lands
/// from the background.
#[gpui::test]
fn select_all_while_counting_lands_from_the_background(cx: &mut TestAppContext) {
    let o = open(cx, &"ab ".repeat(300), 0);
    let m = Lit::new("ab");
    let calls = m.calls.clone();
    let n = o.editor.update(o.vcx, |v, cx| {
        v.set_find(Some(m), cx);
        v.select_all_matches(10_000, cx)
    });
    assert_eq!(n, 0, "it waits for the background");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    o.vcx.run_until_parked();
    let ranges = o
        .vcx
        .read(|cx| o.editor.read(cx).snapshot().selection.ranges().len());
    assert_eq!(ranges, 300);
}

/// A query with no match puts the selection back at the origin.
#[gpui::test]
fn find_from_without_a_match_returns_to_the_origin(cx: &mut TestAppContext) {
    let o = open(cx, "ab ab", 1);
    o.editor.update(o.vcx, |v, cx| {
        v.set_find(Some(Lit::new("ab")), cx);
        v.find_from(1..1, cx);
    });
    o.vcx.run_until_parked();
    assert_eq!(main_range(&o), 3..5);
    o.editor.update(o.vcx, |v, cx| {
        v.set_find(Some(Lit::new("zz")), cx);
        v.find_from(1..1, cx);
    });
    o.vcx.run_until_parked();
    assert_eq!(main_range(&o), 1..1);
}

/// An edit while a count runs: the older count's batches never land; the total is the new
/// text's.
#[gpui::test]
fn editing_while_a_count_runs_recounts(cx: &mut TestAppContext) {
    let text = "ab ".repeat(400_000);
    let o = open(cx, &text, 0);
    o.editor.update(o.vcx, |v, cx| {
        v.set_find(Some(Lit::new("ab")), cx);
        v.set_text("ab xy ab", cx);
    });
    o.vcx.run_until_parked();
    let s = status(&o);
    assert_eq!((s.total, s.counted, s.capped), (Some(2), 2, false));
}

/// T31's select next occurrence says so when every occurrence is selected already, and its
/// select all occurrences says how many of how many past the cap.
#[gpui::test]
fn occurrence_announcements(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT a FROM a;", 7);
    run_cmd(&mut o, "editor.select_next_occurrence");
    run_cmd(&mut o, "editor.select_next_occurrence");
    assert_eq!(
        o.vcx
            .read(|cx| o.editor.read(cx).snapshot().selection.ranges().len()),
        2
    );
    run_cmd(&mut o, "editor.select_next_occurrence");
    let said = o.vcx.read(|cx| status_message(o.editor.read(cx)));
    assert_eq!(said.as_deref(), Some("hv-find-no-more"));
}

#[gpui::test]
fn select_all_occurrences_past_the_cap_says_so(cx: &mut TestAppContext) {
    let text = "ab ".repeat(10_500);
    let mut o = open(cx, &text, 0);
    run_cmd(&mut o, "editor.select_all_occurrences");
    let n = o
        .vcx
        .read(|cx| o.editor.read(cx).snapshot().selection.ranges().len());
    assert_eq!(n, 10_000);
    let said = o.vcx.read(|cx| status_message(o.editor.read(cx)));
    assert_eq!(said.as_deref(), Some("hv-find-capped"));
}

#[gpui::test]
fn a_step_into_a_fold_opens_it(cx: &mut TestAppContext) {
    let text = "SELECT 1;\nSELECT\n  a,\n  needle,\n  b\nFROM t;\nSELECT 2;\n";
    let mut o = open(cx, text, 0);
    let stmt = text.find("SELECT\n").unwrap()..text.find("FROM t;").unwrap() + 7;
    let folded = o.editor.update(o.vcx, |v, cx| fold_range(v, stmt, cx));
    assert!(folded, "the statement folds");
    draw(o.vcx);
    set(&mut o, Lit::new("needle"));
    let at = text.find("needle").unwrap();
    assert_eq!(step(&mut o, false), Some(at..at + 6));
    assert!(
        o.vcx
            .read(|cx| o.editor.read(cx).folds().folded().is_empty()),
        "the fold opened"
    );
}

#[test]
fn scrollbar_ticks_sit_by_visible_row() {
    use hacivat_text::display::HeightMap;
    use hacivat_ui_editor::scrollbar::row_span;
    let track = gpui::Bounds::new(point(px(0.0), px(0.0)), gpui::size(px(8.0), px(120.0)));
    // 20 lines; lines 2–9 hidden under the header on line 1: 12 visible rows.
    let map = HeightMap::chunked(20, 18.0, &[2..10]);
    // Line 15 is visible row 7 of 12: 7 / 12 of 120 px.
    let b = row_span(track, &map, 15..16);
    assert_eq!(f32::from(b.top()), 70.0);
    // A hidden line sits on its fold's header (row 1).
    let b = row_span(track, &map, 5..6);
    assert_eq!(f32::from(b.top()), 10.0);
    // Without folds: by line, as before.
    let b = row_span(track, &HeightMap::uniform(20, 18.0), 15..16);
    assert_eq!(f32::from(b.top()), 90.0);
}

#[gpui::test]
fn horizontal_thumb_drags(cx: &mut TestAppContext) {
    let long = format!("SELECT '{}';\nSELECT 2;\n", "x".repeat(600));
    let o = open(cx, &long, 0);
    let x0 = |o: &Opened<'_>| {
        o.vcx.read(|cx| {
            let v = o.editor.read(cx);
            let layout = v.last_layout().expect("a frame");
            f32::from(layout.x_for_offset(0).expect("line 1 is painted"))
        })
    };
    let before = x0(&o);
    // The thumb sits at the text area's bottom left (the gutter is 58 px wide).
    let at = point(px(70.0), px(595.0));
    o.vcx
        .simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
    o.vcx.simulate_mouse_move(
        point(px(370.0), px(595.0)),
        MouseButton::Left,
        Modifiers::none(),
    );
    o.vcx.simulate_mouse_up(
        point(px(370.0), px(595.0)),
        MouseButton::Left,
        Modifiers::none(),
    );
    draw(o.vcx);
    let after = x0(&o);
    assert!(after < before - 100.0, "{before} → {after}");
    // The press on the thumb did not move the caret.
    assert_eq!(main_range(&o), 0..0);
}

/// NOTES surface 16 / M3c-10 on the rendered scene: every other match on the painted rows has a
/// 1 px `text-3` outline and no fill, the current one (the selection) none; the scrollbar has a
/// 2 px tick per matched line across the track, `text-2`, the current one `blue`.
#[cfg(feature = "scene-check")]
#[gpui::test]
fn outlines_and_scrollbar_ticks(cx: &mut TestAppContext) {
    use hacivat_ui_core::theme::DARK;
    let mut lines = vec!["needle 0".to_string(), "x needle needle".to_string()];
    lines.extend((2..100).map(|i| {
        if i == 50 {
            "needle 50".to_string()
        } else {
            "SELECT 1;".to_string()
        }
    }));
    let text = lines.join("\n") + "\n";
    let mut o = open(cx, &text, 0);
    set(&mut o, Lit::new("needle"));
    assert_eq!(step(&mut o, false), Some(0..6));
    draw(o.vcx);
    let c = &DARK.colors;
    let same = |a: gpui::Hsla, b: gpui::Hsla| {
        (a.h - b.h).abs() < 1e-3
            && (a.s - b.s).abs() < 1e-3
            && (a.l - b.l).abs() < 1e-3
            && (a.a - b.a).abs() < 1e-3
    };
    type Quad = (f32, f32, f32, f32, Option<gpui::Hsla>, gpui::Hsla, f32);
    let quads: Vec<Quad> = o.vcx.update(|window, _| {
        let s = window.scale_factor();
        window
            .rendered_scene()
            .quads
            .iter()
            .map(|q| {
                (
                    q.bounds.origin.x.0 / s,
                    q.bounds.origin.y.0 / s,
                    q.bounds.size.width.0 / s,
                    q.bounds.size.height.0 / s,
                    q.background.as_solid(),
                    q.border_color,
                    q.border_widths.left.0 / s,
                )
            })
            .collect()
    });
    // The scene may hold a quad more than once (several paints of the frame): count each once.
    let mut unique: Vec<Quad> = Vec::new();
    for q in quads {
        if !unique.iter().any(|u| format!("{u:?}") == format!("{q:?}")) {
            unique.push(q);
        }
    }
    let quads = unique;
    // Two outlines, on line 1 (rows are 18 px from y = 6); none on line 0's current match.
    let outlines: Vec<&Quad> = quads
        .iter()
        .filter(|q| q.6 > 0.0 && same(q.5, c.text_3.to_hsla()))
        .collect();
    assert_eq!(outlines.len(), 2, "{outlines:?}");
    for q in &outlines {
        assert_eq!(q.1, 24.0, "{q:?}");
        assert!(q.4.is_none_or(|f| f.a == 0.0), "no fill: {q:?}");
        assert!((q.6 - 1.0).abs() < 1e-3);
    }
    // The ticks: across the 8 px track less 1 px each side, 2 px tall.
    let ticks = |ink: gpui::Hsla| -> Vec<f32> {
        quads
            .iter()
            .filter(|q| q.0 == 793.0 && q.2 == 6.0 && q.4.is_some_and(|f| same(f, ink)))
            .map(|q| q.1)
            .collect()
    };
    assert_eq!(ticks(c.blue.to_hsla()), [0.0], "the current match's tick");
    // Lines 1 and 50 of 101: rows 5 and 297 of the 600 px track.
    assert_eq!(ticks(c.text_2.to_hsla()), [5.0, 297.0]);
}

/// ALG UI-EDIT-22 test 3 (the pipeline): a 20 MB document's count completes within 200 ms
/// (release build, `HACIVAT_PERF=1`): the text flattened once, then searched in batches, every
/// batch handed back to the view. The step to the first match from the top is instant.
#[gpui::test]
#[ignore = "perf"]
fn counting_20_mb_under_200ms(cx: &mut TestAppContext) {
    if std::env::var_os("HACIVAT_PERF").is_none() {
        return;
    }
    let line = "INSERT INTO t (id, name) VALUES (1, 'abc');\n";
    let text = line.repeat(20 * (1 << 20) / line.len());
    let o = open(cx, &text, 0);
    let at = std::time::Instant::now();
    o.editor
        .update(o.vcx, |v, cx| v.set_find(Some(Lit::new("INSERT")), cx));
    o.vcx.run_until_parked();
    let took = at.elapsed();
    let s = status(&o);
    assert_eq!(
        s.total,
        Some(u32::try_from(text.len() / line.len()).unwrap())
    );
    // Past the kept cap the step runs on the background executor (fix round 1, I1).
    let at = std::time::Instant::now();
    let now = o.editor.update(o.vcx, |v, cx| v.find_step(false, cx));
    let ui = at.elapsed();
    o.vcx.run_until_parked();
    let stepped = at.elapsed();
    assert_eq!((now, main_range(&o)), (None, 0..6));
    println!("find perf: count of 20 MB {took:?}, first step {stepped:?} (UI thread {ui:?})");
    assert!(took.as_millis() < 200, "{took:?}");
}
