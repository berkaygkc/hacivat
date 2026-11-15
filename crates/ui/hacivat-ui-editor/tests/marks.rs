//! M3a Task 28: execution marks (UI-EDIT-19): container anchors rebased from the submitting
//! version, staleness, tombstones restored by undo, the end-of-line status and its collision
//! rule, the 1 Hz timer. Events are fake (`set_exec_state`), time is the fake clock.

#![allow(clippy::single_range_in_vec_init, reason = "byte ranges as data")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AppContext as _, Context, Entity, EntityInputHandler as _, IntoElement, ParentElement as _,
    Render, Styled as _, TestAppContext, VisualTestContext, Window, div, px,
};
use hacivat_ui::icon::{IconName, icon_record, icon_take};
use hacivat_ui::text::{TextFn, TextRegistry};
use hacivat_ui_core::theme::{DARK, Theme, ThemeMode};
use hacivat_ui_editor::marks::{RUNNING_GRACE, TICK, TOMBSTONE_TRANSACTIONS};
use hacivat_ui_editor::{
    DocId, EditorConfig, EditorView, ExecMarkState, FailClosedService, FrameProbe, MarkKey,
    RowsOutcome,
};

struct Host {
    editor: Entity<EditorView>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(800.0)).h(px(600.0)).child(self.editor.clone())
    }
}

/// Counts painted frames.
#[derive(Default)]
struct Frames(Cell<u32>);

impl FrameProbe for Frames {
    fn input(&self, _: Instant) {}
    fn painted(&self, _: &mut gpui::App) {
        self.0.set(self.0.get() + 1);
    }
}

fn open<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
) -> (Entity<EditorView>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        Theme::install(cx, ThemeMode::Dark);
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        });
    });
    let slot: Rc<RefCell<Option<Entity<EditorView>>>> = Rc::default();
    let out = slot.clone();
    let text = text.to_string();
    let (_, vcx) = cx.add_window_view(move |window, cx| {
        let editor = cx.new(|cx| {
            EditorView::new(
                DocId(28),
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
    // A frame that draws no label leaves the registry as it was: start each one empty.
    vcx.update(|window, cx| {
        cx.global_mut::<TextRegistry>().frame.clear();
        window.refresh();
    });
    vcx.run_until_parked();
}

fn text_fn(s: &'static str) -> Option<TextFn> {
    Some(Rc::new(move |_: &gpui::App| s.into()))
}

fn now(vcx: &VisualTestContext) -> Instant {
    vcx.executor().now()
}

fn version(e: &Entity<EditorView>, vcx: &VisualTestContext) -> u64 {
    vcx.read(|cx| e.read(cx).snapshot().version)
}

/// The texts the last frame drew.
fn drawn(vcx: &VisualTestContext) -> Vec<String> {
    vcx.read(|cx| {
        cx.global::<TextRegistry>()
            .frame
            .iter()
            .map(|t| t.text.to_string())
            .collect()
    })
}

/// Replaces `r` with `text` through the input handler (typing, deleting, pasting), without
/// drawing.
fn edit_quiet(
    e: &Entity<EditorView>,
    vcx: &mut VisualTestContext,
    r: std::ops::Range<usize>,
    text: &str,
) {
    e.update(vcx, |v, cx| v.select_range(r, cx));
    e.update_in(vcx, |v, window, cx| {
        v.replace_text_in_range(None, text, window, cx);
    });
}

/// [`edit_quiet`], then a frame.
fn edit(
    e: &Entity<EditorView>,
    vcx: &mut VisualTestContext,
    r: std::ops::Range<usize>,
    text: &str,
) {
    edit_quiet(e, vcx, r, text);
    draw(vcx);
}

fn line_of(e: &Entity<EditorView>, vcx: &VisualTestContext, offset: usize) -> usize {
    vcx.read(|cx| e.read(cx).snapshot().buffer.byte_to_line(offset))
}

/// Lines 3–5 (1-based) hold the statement of case 1; line 7 the second, line 9 the third.
const SCRIPT: &str =
    "-- report\n\n/* totals */ SELECT id,\n  total\nFROM orders;\n\nSELECT 2;\n\nSELECT 3;\n";

fn spans() -> [(MarkKey, std::ops::Range<usize>); 3] {
    let s1 = SCRIPT.find("/* totals */").unwrap()..SCRIPT.find("orders;").unwrap() + 7;
    let s2 = SCRIPT.find("SELECT 2").unwrap()..SCRIPT.find("SELECT 2").unwrap() + 9;
    let s3 = SCRIPT.find("SELECT 3").unwrap()..SCRIPT.find("SELECT 3").unwrap() + 9;
    [(MarkKey(1), s1), (MarkKey(2), s2), (MarkKey(3), s3)]
}

/// UI-EDIT-19's cases with fake events:
/// 1. Queued, then running (nothing in its first 50 ms, then the amber marker), then
///    succeeded with the app's text at the end of the header's line (the line after the leading
///    comment);
/// 2. typing inside it after success: stale, "edited since execution";
/// 3. ten lines above: the glyph moves with the statement and stays fresh;
/// 4. cancelling: the hollow amber square, never red (the hues in `gutter_scene`); the 10 s
///    escalation badge is M3b;
/// 5. a script stopped on error: #1 succeeded, #2 failed with its SQLSTATE, #3 skipped.
#[gpui::test]
fn ui_edit_19_cases_1_to_5(cx: &mut TestAppContext) {
    let (e, vcx) = open(cx, SCRIPT);
    let [one, two, three] = spans();
    let v0 = version(&e, vcx);
    e.update(vcx, |v, cx| {
        v.add_exec_marks(v0, vec![one.clone(), two.clone(), three.clone()], cx);
        for (key, _) in [&one, &two, &three] {
            v.set_exec_state(*key, ExecMarkState::Queued, None, text_fn("queued"), cx);
        }
    });
    draw(vcx);
    // Case 1: the header is the first token after the comment, on line 3 (index 2).
    let header = vcx
        .read(|cx| e.read(cx).marks().header(MarkKey(1)))
        .unwrap();
    assert_eq!(header, SCRIPT.find("SELECT id").unwrap());
    assert_eq!(line_of(&e, vcx, header), 2);
    assert_eq!(
        drawn(vcx).iter().filter(|t| t.as_str() == "queued").count(),
        3
    );
    let started = now(vcx);
    e.update(vcx, |v, cx| {
        v.set_exec_state(
            MarkKey(1),
            ExecMarkState::Running { started },
            text_fn("Statement 1"),
            text_fn("running · 0 s"),
            cx,
        );
    });
    draw(vcx);
    assert!(
        !drawn(vcx).contains(&"running · 0 s".to_string()),
        "nothing in the first 50 ms"
    );
    vcx.executor().advance_clock(RUNNING_GRACE);
    vcx.run_until_parked();
    assert!(drawn(vcx).contains(&"running · 0 s".to_string()));
    icon_record();
    e.update(vcx, |v, cx| {
        v.set_exec_state(
            MarkKey(1),
            ExecMarkState::Succeeded {
                took: Duration::from_millis(34),
                rows: Some(RowsOutcome::Fetched {
                    rows: 12,
                    more: false,
                }),
            },
            text_fn("12 rows retrieved in 34 ms"),
            text_fn("executed · 34 ms · 12 rows"),
            cx,
        );
    });
    draw(vcx);
    assert!(drawn(vcx).contains(&"executed · 34 ms · 12 rows".to_string()));
    let green = DARK.colors.green.to_hsla();
    let icons = icon_take();
    assert!(
        icons
            .iter()
            .filter(|i| i.name == IconName::Check && i.color == green)
            .count()
            >= 2,
        "the glyph and the status's check: {icons:?}"
    );

    // Case 3: ten lines above move the mark with its statement; it stays fresh.
    edit(&e, vcx, 0..0, "\n\n\n\n\n\n\n\n\n\n");
    let header = vcx
        .read(|cx| e.read(cx).marks().header(MarkKey(1)))
        .unwrap();
    assert_eq!(line_of(&e, vcx, header), 12);
    assert!(!vcx.read(|cx| e.read(cx).marks().is_stale(MarkKey(1))));
    assert!(drawn(vcx).contains(&"executed · 34 ms · 12 rows".to_string()));

    // Case 2: typing inside the statement after success makes it stale.
    let inside = vcx
        .read(|cx| e.read(cx).marks().range(MarkKey(1)))
        .unwrap()
        .start
        + 20;
    edit(&e, vcx, inside..inside, "x");
    assert!(vcx.read(|cx| e.read(cx).marks().is_stale(MarkKey(1))));
    let texts = drawn(vcx);
    assert!(
        texts.contains(&"hv-editor-mark-stale".to_string()),
        "{texts:?}"
    );
    assert!(!texts.contains(&"executed · 34 ms · 12 rows".to_string()));

    // Case 4: cancelling.
    let since = now(vcx);
    e.update(vcx, |v, cx| {
        v.set_exec_state(
            MarkKey(2),
            ExecMarkState::Cancelling { since },
            None,
            text_fn("cancelling…"),
            cx,
        );
    });
    draw(vcx);
    assert!(drawn(vcx).contains(&"cancelling…".to_string()));

    // Case 5: stopped on error: #2 failed (ink glyph, SQLSTATE chip), #3 skipped.
    icon_record();
    e.update(vcx, |v, cx| {
        v.set_exec_state(
            MarkKey(2),
            ExecMarkState::Failed {
                sqlstate: Some("42P01".into()),
            },
            text_fn("Statement 2 failed"),
            text_fn("failed"),
            cx,
        );
        v.set_exec_state(
            MarkKey(3),
            ExecMarkState::Skipped,
            None,
            text_fn("skipped: an earlier statement failed"),
            cx,
        );
    });
    draw(vcx);
    let texts = drawn(vcx);
    for t in ["failed", "42P01", "skipped: an earlier statement failed"] {
        assert!(texts.contains(&t.to_string()), "{t}: {texts:?}");
    }
    let icons = icon_take();
    let ink = |name: IconName| icons.iter().find(|i| i.name == name).map(|i| i.color);
    assert_eq!(
        ink(IconName::TriangleAlert),
        Some(DARK.colors.text.to_hsla())
    );
    assert_eq!(
        ink(IconName::SkipForward),
        Some(DARK.colors.text_2.to_hsla())
    );
    // A stale success's check dims to text_3.
    assert_eq!(ink(IconName::Check), Some(DARK.colors.text_3.to_hsla()));
    let marks = vcx.read(|cx| {
        let m = e.read(cx).marks();
        [1, 2, 3].map(|k| m.state(MarkKey(k)).cloned())
    });
    assert!(matches!(marks[0], Some(ExecMarkState::Succeeded { .. })));
    assert!(matches!(marks[1], Some(ExecMarkState::Failed { .. })));
    assert!(matches!(marks[2], Some(ExecMarkState::Skipped)));
}

/// Staleness: an edit inside or at the edge of a mark makes it stale, one elsewhere does not.
#[gpui::test]
fn edit_inside_a_mark_makes_it_stale(cx: &mut TestAppContext) {
    let (e, vcx) = open(cx, "SELECT 1;\nSELECT 2;\nSELECT 3;\n");
    let v0 = version(&e, vcx);
    e.update(vcx, |v, cx| {
        v.add_exec_marks(v0, vec![(MarkKey(1), 0..9), (MarkKey(2), 10..19)], cx);
    });
    // Typing on the third line touches neither.
    edit(&e, vcx, 27..27, " ");
    let stale = |vcx: &VisualTestContext, k| vcx.read(|cx| e.read(cx).marks().is_stale(MarkKey(k)));
    assert!(!stale(vcx, 1) && !stale(vcx, 2));
    // Typing at the end of the second statement grows it: stale.
    edit(&e, vcx, 19..19, " ");
    assert!(!stale(vcx, 1) && stale(vcx, 2));
    assert_eq!(
        vcx.read(|cx| e.read(cx).marks().range(MarkKey(2))),
        Some(10..20)
    );
    // A replacement of equal length inside the first: stale.
    edit(&e, vcx, 7..8, "9");
    assert!(stale(vcx, 1));
}

/// Units resolved at `v1`, then an edit at the first's edge and one inside the second, then the
/// marks arrive: rebased, both stale from the start; re-running the same range replaces the
/// mark; a version older than the change log is dropped.
#[gpui::test]
fn marks_submitted_at_an_older_version_are_rebased(cx: &mut TestAppContext) {
    let (e, vcx) = open(cx, "SELECT 1;\nSELECT 2;\n");
    let v1 = version(&e, vcx);
    edit(&e, vcx, 0..0, "-- x\n");
    edit(&e, vcx, 22..22, "0");
    e.update(vcx, |v, cx| {
        v.add_exec_marks(v1, vec![(MarkKey(5), 0..9), (MarkKey(6), 10..19)], cx);
    });
    let m = vcx.read(|cx| {
        let m = e.read(cx).marks();
        (
            m.range(MarkKey(5)),
            m.is_stale(MarkKey(5)),
            m.range(MarkKey(6)),
            m.is_stale(MarkKey(6)),
        )
    });
    // Mark 5 grew with the comment typed at its start edge (a container), so it is stale too.
    assert_eq!(m, (Some(0..14), true, Some(15..25), true));
    // Re-running the same range replaces the mark.
    let now_v = version(&e, vcx);
    e.update(vcx, |v, cx| {
        v.add_exec_marks(now_v, vec![(MarkKey(7), 15..25)], cx)
    });
    let m = vcx.read(|cx| {
        let m = e.read(cx).marks();
        (m.len(), m.range(MarkKey(6)), m.is_stale(MarkKey(7)))
    });
    assert_eq!(m, (2, None, false));
    // Older than the change log: dropped.
    for i in 0..300 {
        let at = vcx.read(|cx| e.read(cx).snapshot().buffer.len_bytes());
        edit_quiet(&e, vcx, at..at, if i % 2 == 0 { "a" } else { "b" });
    }
    e.update(vcx, |v, cx| {
        v.add_exec_marks(v1, vec![(MarkKey(8), 0..9)], cx)
    });
    assert_eq!(vcx.read(|cx| e.read(cx).marks().range(MarkKey(8))), None);
}

/// Deleting a whole statement removes its mark and keeps a tombstone; undo restores it with its
/// outcome. Replacing it (a paste over it) does the same. A tombstone lapses after 64 text
/// transactions.
#[gpui::test]
fn tombstone_restores_a_mark_on_undo(cx: &mut TestAppContext) {
    let (e, vcx) = open(cx, "SELECT 1;\nSELECT 2;\n");
    let v0 = version(&e, vcx);
    e.update(vcx, |v, cx| {
        v.add_exec_marks(v0, vec![(MarkKey(1), 10..19)], cx);
        v.set_exec_state(
            MarkKey(1),
            ExecMarkState::Succeeded {
                took: Duration::from_millis(5),
                rows: None,
            },
            None,
            text_fn("executed · 5 ms"),
            cx,
        );
    });
    let undo = |e: &Entity<EditorView>, vcx: &mut VisualTestContext| {
        e.update_in(vcx, |v, window, cx| {
            v.run_command("editor.undo", window, cx)
        });
        draw(vcx);
    };
    // Delete it: gone, one tombstone.
    edit(&e, vcx, 10..19, "");
    let state = vcx.read(|cx| (e.read(cx).marks().len(), e.read(cx).marks().tombstones()));
    assert_eq!(state, (0, 1));
    undo(&e, vcx);
    let m = vcx.read(|cx| {
        let m = e.read(cx).marks();
        (
            m.range(MarkKey(1)),
            m.state(MarkKey(1)).cloned(),
            m.tombstones(),
        )
    });
    assert_eq!(m.0, Some(10..19));
    assert!(matches!(m.1, Some(ExecMarkState::Succeeded { .. })));
    assert_eq!(m.2, 0);
    assert!(drawn(vcx).contains(&"executed · 5 ms".to_string()));

    // Paste over it: undo of the paste restores it.
    edit(&e, vcx, 10..19, "DELETE FROM t;");
    assert_eq!(vcx.read(|cx| e.read(cx).marks().len()), 0);
    undo(&e, vcx);
    assert_eq!(
        vcx.read(|cx| e.read(cx).marks().range(MarkKey(1))),
        Some(10..19)
    );

    // Delete it, then 64 other text transactions: the tombstone lapses.
    edit(&e, vcx, 10..19, "");
    for _ in 0..TOMBSTONE_TRANSACTIONS {
        edit_quiet(&e, vcx, 0..0, " ");
        edit_quiet(&e, vcx, 0..1, "");
    }
    assert_eq!(vcx.read(|cx| e.read(cx).marks().tombstones()), 0);
}

/// The 1 Hz clock: a running mark repaints the editor once a second (and once at 50 ms), with
/// no repaint in between; once nothing runs the timer is gone and nothing repaints.
#[gpui::test]
fn timer_stops_when_nothing_runs(cx: &mut TestAppContext) {
    let (e, vcx) = open(cx, "SELECT pg_sleep(5);\n");
    let frames = Rc::new(Frames::default());
    e.update(vcx, |v, _| v.set_probe(Some(frames.clone())));
    let v0 = version(&e, vcx);
    let ticking = |vcx: &VisualTestContext| vcx.read(|cx| e.read(cx).marks().ticking());
    e.update(vcx, |v, cx| {
        v.add_exec_marks(v0, vec![(MarkKey(1), 0..19)], cx)
    });
    draw(vcx);
    assert!(!ticking(vcx), "nothing runs yet");
    let started = now(vcx);
    e.update(vcx, |v, cx| {
        v.set_exec_state(
            MarkKey(1),
            ExecMarkState::Running { started },
            None,
            None,
            cx,
        );
    });
    vcx.run_until_parked();
    assert!(ticking(vcx));
    let at = frames.0.get();
    vcx.executor().advance_clock(RUNNING_GRACE);
    vcx.run_until_parked();
    assert_eq!(frames.0.get(), at + 1, "one repaint at 50 ms");
    vcx.executor().advance_clock(TICK / 2);
    vcx.run_until_parked();
    assert_eq!(frames.0.get(), at + 1, "nothing between ticks");
    vcx.executor().advance_clock(TICK / 2);
    vcx.run_until_parked();
    assert_eq!(frames.0.get(), at + 2, "one repaint per second");
    vcx.executor().advance_clock(TICK);
    vcx.run_until_parked();
    assert_eq!(frames.0.get(), at + 3);
    // Cancelling keeps the clock; cancelled stops it.
    let since = now(vcx);
    e.update(vcx, |v, cx| {
        v.set_exec_state(
            MarkKey(1),
            ExecMarkState::Cancelling { since },
            None,
            None,
            cx,
        );
    });
    vcx.run_until_parked();
    assert!(ticking(vcx));
    e.update(vcx, |v, cx| {
        v.set_exec_state(MarkKey(1), ExecMarkState::Cancelled, None, None, cx);
    });
    vcx.run_until_parked();
    assert!(!ticking(vcx));
    let at = frames.0.get();
    vcx.executor().advance_clock(TICK * 5);
    vcx.run_until_parked();
    assert_eq!(frames.0.get(), at, "no repaint once nothing runs");
    // Clearing the marks stops a running clock too.
    let started = now(vcx);
    e.update(vcx, |v, cx| {
        v.set_exec_state(
            MarkKey(1),
            ExecMarkState::Running { started },
            None,
            None,
            cx,
        );
    });
    vcx.run_until_parked();
    assert!(ticking(vcx));
    e.update(vcx, |v, cx| v.clear_exec_marks(cx));
    vcx.run_until_parked();
    assert!(!ticking(vcx));
}

/// The end-of-line status sits at the right; when it would come within 12 px of its line's
/// text it is hidden, except a failure's SQLSTATE chip, which moves onto the pad alone.
#[gpui::test]
fn end_of_line_status_hides_on_collision(cx: &mut TestAppContext) {
    let (e, vcx) = open(cx, "SELECT 1;\n");
    // Lines as wide as the text area, whatever the test text system's advance.
    let (advance, width) = vcx.read(|cx| {
        let l = e.read(cx).last_layout().unwrap();
        let x = |b: usize| f32::from(l.x_for_offset(b).unwrap());
        (x(1) - x(0), f32::from(l.rows().bounds.size.width))
    });
    let long = format!("SELECT '{}';", "x".repeat((width / advance) as usize));
    let text = format!("SELECT 1;\n{long}\n{long}\n");
    e.update(vcx, |v, cx| v.set_text(&text, cx));
    draw(vcx);
    let v0 = version(&e, vcx);
    let l2 = 10 + long.len() + 1;
    e.update(vcx, |v, cx| {
        v.add_exec_marks(
            v0,
            vec![
                (MarkKey(1), 0..9),
                (MarkKey(2), 10..10 + long.len()),
                (MarkKey(3), l2..l2 + long.len()),
            ],
            cx,
        );
        for k in [1, 2] {
            v.set_exec_state(
                MarkKey(k),
                ExecMarkState::Succeeded {
                    took: Duration::from_millis(42),
                    rows: None,
                },
                None,
                text_fn(if k == 1 {
                    "executed · 42 ms"
                } else {
                    "executed · 43 ms"
                }),
                cx,
            );
        }
        v.set_exec_state(
            MarkKey(3),
            ExecMarkState::Failed {
                sqlstate: Some("22012".into()),
            },
            None,
            text_fn("failed"),
            cx,
        );
    });
    draw(vcx);
    let texts = drawn(vcx);
    assert!(
        texts.contains(&"executed · 42 ms".to_string()),
        "short line: {texts:?}"
    );
    assert!(
        !texts.contains(&"executed · 43 ms".to_string()),
        "long line: {texts:?}"
    );
    assert!(
        !texts.contains(&"failed".to_string()),
        "long line: {texts:?}"
    );
    assert!(
        texts.contains(&"22012".to_string()),
        "the chip on the pad: {texts:?}"
    );
}

// ## m3a-t28-gutter (fix round 1)

/// A re-run whose range cannot be placed (off a character boundary) removes the previous mark
/// of its key: the old outcome never stays up for a statement that ran again. Review round 1,
/// Minor 8.
#[gpui::test]
fn a_rerun_that_cannot_be_placed_removes_the_old_mark(cx: &mut TestAppContext) {
    let (e, vcx) = open(cx, "SELECT 'ş';\n");
    let v0 = version(&e, vcx);
    e.update(vcx, |v, cx| {
        v.add_exec_marks(v0, vec![(MarkKey(1), 0..11)], cx);
        v.set_exec_state(MarkKey(1), ExecMarkState::Skipped, None, None, cx);
    });
    assert_eq!(vcx.read(|cx| e.read(cx).marks().len()), 1);
    // 0..9 ends inside 'ş' (bytes 8..10).
    e.update(vcx, |v, cx| {
        v.add_exec_marks(v0, vec![(MarkKey(1), 0..9)], cx)
    });
    let m = vcx.read(|cx| {
        (
            e.read(cx).marks().len(),
            e.read(cx).marks().range(MarkKey(1)),
        )
    });
    assert_eq!(m, (0, None));
}

/// Two marks on one row: hovering the glyph shows "Two statements on this line" and both
/// statements' outcomes (NOTES surface 5). Review round 1, Minor 7.
#[gpui::test]
fn the_tooltip_of_two_marks_lists_both(cx: &mut TestAppContext) {
    let (e, vcx) = open(cx, "SELECT 1; SELECT x;\n");
    let v0 = version(&e, vcx);
    e.update(vcx, |v, cx| {
        v.add_exec_marks(v0, vec![(MarkKey(1), 0..9), (MarkKey(2), 10..19)], cx);
        v.set_exec_state(
            MarkKey(1),
            ExecMarkState::Succeeded {
                took: Duration::from_millis(2),
                rows: None,
            },
            text_fn("Statement 1 ran"),
            None,
            cx,
        );
        v.set_exec_state(
            MarkKey(2),
            ExecMarkState::Failed { sqlstate: None },
            text_fn("Statement 2 failed"),
            None,
            cx,
        );
    });
    draw(vcx);
    // Hover the glyph slot (12 × 18 at x = 8, first row at y = 6), then wait out the delay.
    vcx.simulate_mouse_move(
        gpui::point(px(14.0), px(15.0)),
        None,
        gpui::Modifiers::none(),
    );
    vcx.executor().advance_clock(Duration::from_secs(2));
    vcx.run_until_parked();
    let texts = drawn(vcx);
    for t in ["hv-editor-tip-two", "Statement 1 ran", "Statement 2 failed"] {
        assert!(texts.contains(&t.to_string()), "{t}: {texts:?}");
    }
}
