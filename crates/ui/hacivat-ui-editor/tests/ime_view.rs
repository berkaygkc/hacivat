//! M3a Task 27: the editor's platform input handler (UI-EDIT-12 view side, R-M3-7). The tests
//! call the `EntityInputHandler` methods on the entity, as the platform does: UTF-16 ranges,
//! marked text, `bounds_for_range` from the last frame (and its estimate between an edit and
//! the next frame), `character_index_for_point`, the reversed selection, read-only, the text
//! input configuration, `FocusedInput`, the blur rule, `notify_text_input_changed` only after
//! non-IME edits, and app edits waiting for a composition to end. Tests wait on conditions
//! (`run_until_parked`) and drive time through the fake clock only.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AppContext as _, ClipboardItem, Context, Entity, EntityInputHandler as _, FocusHandle,
    Focusable as _, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _,
    TestAppContext, VisualTestContext, Window, div, point, px,
};
use hacivat_ui::input::FocusedInput;
use hacivat_ui_core::gpui_surface::Autocapitalize;
use hacivat_ui_core::theme::{Theme, ThemeMode};
use hacivat_ui_editor::{DocId, EditorConfig, EditorEvent, EditorView, FailClosedService};

/// One editor in an 800 × 600 box and a focusable stop beside it (an in-window focus move).
struct Host {
    editor: Entity<EditorView>,
    other: FocusHandle,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .child(div().w(px(800.0)).h(px(600.0)).child(self.editor.clone()))
            .child(
                div()
                    .id("other")
                    .track_focus(&self.other)
                    .w(px(10.0))
                    .h(px(10.0)),
            )
    }
}

/// The editor and the stop beside it.
type Parts = (Entity<EditorView>, FocusHandle);

struct Opened<'a> {
    editor: Entity<EditorView>,
    other: FocusHandle,
    events: Rc<RefCell<Vec<EditorEvent>>>,
    vcx: &'a mut VisualTestContext,
}

fn draw(vcx: &mut VisualTestContext) {
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
}

/// Opens `text` with the caret at `caret` (bytes), focused, in the active window, drawn.
fn open_with<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
    caret: usize,
    cfg: EditorConfig,
) -> Opened<'a> {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let text = text.to_string();
    let slot: Rc<RefCell<Option<Parts>>> = Rc::default();
    let out = slot.clone();
    let (_, vcx) = cx.add_window_view(move |window, cx| {
        let editor = cx.new(|cx| {
            EditorView::new(
                DocId(27),
                &text,
                cfg,
                Rc::new(FailClosedService),
                Rc::new(|_: &gpui::App| "Console 1".into()),
                window,
                cx,
            )
        });
        let other = cx.focus_handle();
        *out.borrow_mut() = Some((editor.clone(), other.clone()));
        Host { editor, other }
    });
    let (editor, other) = slot.borrow().clone().unwrap();
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
        other,
        events,
        vcx,
    }
}

fn open<'a>(cx: &'a mut TestAppContext, text: &str, caret: usize) -> Opened<'a> {
    open_with(cx, text, caret, EditorConfig::default())
}

fn text(o: &Opened<'_>) -> String {
    o.vcx
        .read(|cx| o.editor.read(cx).snapshot().buffer.to_string())
}

fn composing(o: &Opened<'_>) -> bool {
    o.vcx.read(|cx| o.editor.read(cx).status().composing)
}

fn mark(o: &mut Opened<'_>, text: &str) {
    o.editor.update_in(o.vcx, |v, window, cx| {
        v.replace_and_mark_text_in_range(None, text, None, window, cx);
    });
}

fn commit(o: &mut Opened<'_>, text: &str) {
    o.editor.update_in(o.vcx, |v, window, cx| {
        v.replace_text_in_range(None, text, window, cx);
    });
}

fn run(o: &mut Opened<'_>, id: &str) {
    let ran = o
        .editor
        .update_in(o.vcx, |v, window, cx| v.run_command(id, window, cx));
    assert!(ran, "{id} is a command");
    o.vcx.run_until_parked();
}

/// Undoes until nothing is left, and returns how many undos that took.
fn undo_all(o: &mut Opened<'_>) -> usize {
    let mut n = 0;
    while o
        .vcx
        .read(|cx| hacivat_ui_editor::commands::can_undo(o.editor.read(cx)))
    {
        run(o, "editor.undo");
        n += 1;
        assert!(n < 100, "undo terminates");
    }
    n
}

fn notifications(o: &Opened<'_>) -> u64 {
    o.vcx
        .read(|cx| o.editor.read(cx).text_input_notifications())
}

fn composition_events(o: &Opened<'_>) -> Vec<bool> {
    o.events
        .borrow()
        .iter()
        .filter_map(|e| match e {
            EditorEvent::CompositionChanged(c) => Some(*c),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Geometry.

/// The candidate window sits at the painted caret: `bounds_for_range` of the caret lands
/// within 2 px of `caret_bounds`, on the caret's row, one row high, at least 1 px wide.
#[gpui::test]
fn bounds_for_range_is_within_2px_of_the_caret(cx: &mut TestAppContext) {
    let src = "SELECT 1;\nSELECT 'ş😀', 2";
    let o = open(cx, src, src.len());
    let (caret, utf16_len) = o.vcx.read(|cx| {
        let v = o.editor.read(cx);
        (v.caret_bounds().unwrap(), v.snapshot().buffer.len_utf16())
    });
    let b = o
        .editor
        .update_in(o.vcx, |v, window, cx| {
            let eb = v.last_layout().unwrap().rows().bounds;
            v.bounds_for_range(utf16_len..utf16_len, eb, window, cx)
        })
        .expect("the caret's row is painted");
    let lh = o
        .vcx
        .read(|cx| o.editor.read(cx).last_layout().unwrap().rows().line_height);
    assert!(
        (b.origin.x - caret.origin.x).abs() <= px(2.0),
        "{b:?} vs {caret:?}"
    );
    assert!(
        (b.origin.y - caret.origin.y).abs() <= px(2.0),
        "{b:?} vs {caret:?}"
    );
    assert_eq!(b.size.height, lh);
    assert!(b.size.width >= px(1.0));
    // A range inside the line starts at its first character's x, across the surrogate pair.
    let emoji_at = src.find('😀').unwrap();
    let (x_emoji, start16) = o.vcx.read(|cx| {
        let v = o.editor.read(cx);
        let s = v.snapshot();
        (
            v.last_layout().unwrap().x_for_offset(emoji_at).unwrap(),
            s.buffer.byte_to_utf16(emoji_at),
        )
    });
    let r = o
        .editor
        .update_in(o.vcx, |v, window, cx| {
            let eb = v.last_layout().unwrap().rows().bounds;
            v.bounds_for_range(start16..start16 + 2, eb, window, cx)
        })
        .unwrap();
    assert!(
        (r.origin.x - x_emoji).abs() <= px(0.5),
        "{r:?} vs {x_emoji:?}"
    );
    assert!(r.size.width > px(1.0), "the emoji's width: {r:?}");
}

/// Between an edit and the next frame the last layout is older than the text: the start of a
/// composition still maps through it exactly (the edit is after it), and a range on a line the
/// frame never painted is estimated on its row, never `None`.
#[gpui::test]
fn bounds_for_range_between_an_edit_and_the_next_frame(cx: &mut TestAppContext) {
    let o = open(cx, "SELECT ", 7);
    let painted_x = o.vcx.read(|cx| {
        o.editor
            .read(cx)
            .last_layout()
            .unwrap()
            .x_for_offset(7)
            .unwrap()
    });
    // In one update: a test window draws a dirty window when an update ends, as a real one
    // does on its next frame.
    let (stale, marked, b) = o.editor.update_in(o.vcx, |v, window, cx| {
        v.replace_and_mark_text_in_range(None, "nihongo", None, window, cx);
        let stale = v.last_layout().unwrap().version() != v.snapshot().version;
        let marked = v.marked_text_range(window, cx).expect("composing");
        let eb = v.last_layout().unwrap().rows().bounds;
        (
            stale,
            marked.clone(),
            v.bounds_for_range(marked, eb, window, cx),
        )
    });
    assert!(stale, "no frame since the mark");
    assert_eq!(marked, 7..14);
    let b = b.expect("never None after a frame");
    assert!(
        (b.origin.x - painted_x).abs() <= px(0.5),
        "{b:?} at {painted_x:?}"
    );
    // A commit adds two lines; the third line was never painted: estimated on its row.
    let (rows, b) = o.editor.update_in(o.vcx, |v, window, cx| {
        v.replace_text_in_range(None, "a\nb\nc", window, cx);
        let rows = *v.last_layout().unwrap().rows();
        let len16 = v.snapshot().buffer.len_utf16();
        (
            rows,
            v.bounds_for_range(len16..len16, rows.bounds, window, cx),
        )
    });
    let b = b.expect("estimated");
    let expected_top = rows.first_y + rows.line_height * 2.0;
    assert!(
        (b.origin.y - expected_top).abs() <= px(0.5),
        "{b:?} vs {expected_top:?}"
    );
    // The next frame paints the line, and the answer is exact again.
    draw(o.vcx);
    let (caret, len16) = o.vcx.read(|cx| {
        let v = o.editor.read(cx);
        (v.caret_bounds().unwrap(), v.snapshot().buffer.len_utf16())
    });
    let b = o
        .editor
        .update_in(o.vcx, |v, window, cx| {
            let eb = v.last_layout().unwrap().rows().bounds;
            v.bounds_for_range(len16..len16, eb, window, cx)
        })
        .unwrap();
    assert!((b.origin.x - caret.origin.x).abs() <= px(2.0));
}

/// UI-EDIT-12 test 8: the candidate window's query answers in < 1 ms at line 49,000 of a
/// 50,000-line document (a wall-time assert: perf-gated, P27).
#[gpui::test]
#[ignore = "perf"]
fn bounds_for_range_at_line_49000_under_1ms(cx: &mut TestAppContext) {
    if std::env::var_os("HACIVAT_PERF").is_none_or(|v| v != "1") {
        return;
    }
    let src: String = (0..50_000).map(|i| format!("SELECT {i} AS n;\n")).collect();
    let at = src
        .match_indices('\n')
        .nth(48_999)
        .map(|(i, _)| i + 1)
        .unwrap();
    let o = open(cx, &src, at);
    draw(o.vcx);
    let at16 = o
        .vcx
        .read(|cx| o.editor.read(cx).snapshot().buffer.byte_to_utf16(at));
    let (best, b) = o.editor.update_in(o.vcx, |v, window, cx| {
        let eb = v.last_layout().unwrap().rows().bounds;
        let mut best = Duration::MAX;
        let mut b = None;
        for _ in 0..200 {
            let t = std::time::Instant::now();
            b = v.bounds_for_range(at16..at16 + 6, eb, window, cx);
            best = best.min(t.elapsed());
        }
        (best, b)
    });
    assert!(b.is_some());
    assert!(best < Duration::from_millis(1), "{best:?}");
}

/// `character_index_for_point` answers the UTF-16 offset under a point of the painted text
/// (the inverse of `bounds_for_range`), `None` off the text, and follows an edit made since
/// the frame.
#[gpui::test]
fn character_index_for_point_round_trips_bounds(cx: &mut TestAppContext) {
    let src = "SELECT ş😀xy";
    let o = open(cx, src, 0);
    let half = o
        .vcx
        .read(|cx| o.editor.read(cx).last_layout().unwrap().rows().line_height / 2.0);
    // Every character, the line's last included (review M7: its left half lands before it).
    for (i, _) in src.char_indices() {
        let p = o.vcx.read(|cx| {
            let l = o.editor.read(cx).last_layout().unwrap().clone();
            let b = l.bounds_for_range(i..i).unwrap();
            point(b.origin.x + px(0.5), b.origin.y + half)
        });
        let got = o.editor.update_in(o.vcx, |v, window, cx| {
            v.character_index_for_point(p, window, cx)
        });
        let want = o
            .vcx
            .read(|cx| o.editor.read(cx).snapshot().buffer.byte_to_utf16(i));
        assert_eq!(got, Some(want), "at byte {i}");
    }
    // Far below the painted text: nothing.
    let none = o.editor.update_in(o.vcx, |v, window, cx| {
        v.character_index_for_point(point(px(-500.0), px(-500.0)), window, cx)
    });
    assert_eq!(none, None);
    // Three characters typed at the start, no frame yet: the point over `x` still finds `x`.
    let x_at = src.find('x').unwrap();
    let p = o.vcx.read(|cx| {
        let l = o.editor.read(cx).last_layout().unwrap().clone();
        let b = l.bounds_for_range(x_at..x_at).unwrap();
        point(b.origin.x + px(0.5), b.origin.y + half)
    });
    let got = o.editor.update_in(o.vcx, |v, window, cx| {
        v.replace_text_in_range(None, "abc", window, cx);
        v.character_index_for_point(p, window, cx)
    });
    let want = o.vcx.read(|cx| {
        let s = o.editor.read(cx).snapshot();
        s.buffer.byte_to_utf16(x_at + 3)
    });
    assert_eq!(got, Some(want));
}

// ---------------------------------------------------------------------------------------------
// Ranges, read-only, configuration.

/// A selection made backwards is reported forward with `reversed`; a forward one is not.
#[gpui::test]
fn reversed_selection_reports_reversed(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT ş", 9);
    run(&mut o, "editor.select_left");
    run(&mut o, "editor.select_left");
    let sel = o
        .editor
        .update_in(o.vcx, |v, window, cx| {
            v.selected_text_range(false, window, cx)
        })
        .unwrap();
    assert_eq!((sel.range, sel.reversed), (6..8, true));
    o.editor.update(o.vcx, |v, cx| v.select_range(0..6, cx));
    let sel = o
        .editor
        .update_in(o.vcx, |v, window, cx| {
            v.selected_text_range(false, window, cx)
        })
        .unwrap();
    assert_eq!((sel.range, sel.reversed), (0..6, false));
}

/// A read-only editor accepts no text: neither a commit nor marked text lands, and the
/// selection is reported only when the platform asks while ignoring disabled input.
#[gpui::test]
fn read_only_editor_refuses_text_input(cx: &mut TestAppContext) {
    let cfg = EditorConfig {
        read_only: true,
        ..EditorConfig::default()
    };
    let mut o = open_with(cx, "SELECT 1", 8, cfg);
    let accepts = o
        .editor
        .update_in(o.vcx, |v, window, cx| v.accepts_text_input(window, cx));
    assert!(!accepts);
    commit(&mut o, "x");
    mark(&mut o, "ş");
    assert_eq!(text(&o), "SELECT 1");
    assert!(!composing(&o));
    let (off, on) = o.editor.update_in(o.vcx, |v, window, cx| {
        (
            v.selected_text_range(false, window, cx),
            v.selected_text_range(true, window, cx),
        )
    });
    assert!(off.is_none());
    assert_eq!(on.unwrap().range, 8..8);
}

/// SQL is code: no autocorrect, no autocapitalisation, no suggestions.
#[gpui::test]
fn text_input_configuration_turns_text_assistance_off(cx: &mut TestAppContext) {
    let o = open(cx, "", 0);
    let c = o.editor.update_in(o.vcx, |v, window, cx| {
        v.text_input_configuration(window, cx)
    });
    assert!(!c.autocorrect);
    assert_eq!(c.autocapitalize, Autocapitalize::None);
    assert!(!c.suggestions);
}

/// Marked text over an astral character: UTF-16 ranges count the surrogate pair, and a range
/// that splits it is snapped outward (UI-EDIT-12 test 2). The whole length and a selection the
/// platform sets are UTF-16 too.
#[gpui::test]
fn marked_text_after_an_emoji_reports_utf16(cx: &mut TestAppContext) {
    let mut o = open(cx, "😀", 4);
    mark(&mut o, "ş");
    let (marked, sel, piece, len) = o.editor.update_in(o.vcx, |v, window, cx| {
        let mut adjusted = None;
        let piece = v.text_for_range(1..2, &mut adjusted, window, cx);
        (
            v.marked_text_range(window, cx),
            v.selected_text_range(false, window, cx).unwrap().range,
            (piece, adjusted),
            v.text_length_utf16(window, cx),
        )
    });
    assert_eq!(marked, Some(2..3));
    assert_eq!(sel, 3..3);
    assert_eq!(piece, (Some("😀".to_string()), Some(0..2)));
    assert_eq!(len, Some(3));
    commit(&mut o, "ş");
    o.editor.update_in(o.vcx, |v, window, cx| {
        v.set_selected_text_range(0..2, window, cx);
    });
    let sel = o
        .vcx
        .read(|cx| o.editor.read(cx).snapshot().selection.main().range());
    assert_eq!(sel, 0..4);
}

// ---------------------------------------------------------------------------------------------
// FocusedInput, the blur rule, the clock.

/// The focused editor publishes `FocusedInput { entity, composing }`, and `composing` flips on
/// every mark and unmark (the key dispatcher runs no binding while it is true, F15).
#[gpui::test]
fn focused_input_composing_flips_on_mark_and_unmark(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT ", 7);
    let me = o.editor.entity_id();
    let focused = |o: &Opened<'_>| o.vcx.read(|cx| *cx.global::<FocusedInput>());
    assert_eq!(
        focused(&o),
        FocusedInput {
            entity: Some(me),
            composing: false
        }
    );
    mark(&mut o, "k");
    assert!(focused(&o).composing);
    assert!(composing(&o));
    mark(&mut o, "か");
    o.editor
        .update_in(o.vcx, |v, window, cx| v.unmark_text(window, cx));
    assert!(!focused(&o).composing);
    // A commit ends one, and so does an empty mark (a cancelled composition).
    mark(&mut o, "ş");
    assert!(focused(&o).composing);
    commit(&mut o, "ş");
    assert!(!focused(&o).composing);
    mark(&mut o, "x");
    mark(&mut o, "");
    assert!(!focused(&o).composing);
    assert_eq!(text(&o), "SELECT かş");
    assert_eq!(
        composition_events(&o),
        [true, false, true, false, true, false]
    );
}

/// The blur rule: a focus move inside the active window commits the composition locally (the
/// platform drops its copy on the focus loss); an app switch leaves it with the platform, which
/// resumes and commits it on return, so the text is there once.
#[gpui::test]
fn blur_rule_active_and_inactive_window(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT ", 7);
    let me = o.editor.entity_id();
    mark(&mut o, "ş");
    let other = o.other.clone();
    o.vcx.update(|window, cx| window.focus(&other, cx));
    draw(o.vcx);
    assert!(!composing(&o));
    assert_eq!(text(&o), "SELECT ş");
    assert_eq!(
        o.vcx.read(|cx| *cx.global::<FocusedInput>()),
        FocusedInput::default()
    );
    assert_eq!(composition_events(&o), [true, false]);
    // Back in the editor; a composition, then the window resigns key (an app switch).
    o.editor.update_in(o.vcx, |v, window, cx| {
        let f = v.focus_handle(cx);
        window.focus(&f, cx);
    });
    draw(o.vcx);
    mark(&mut o, "感じ");
    o.vcx.deactivate_window();
    assert!(composing(&o), "the platform keeps it");
    assert_eq!(
        o.vcx.read(|cx| *cx.global::<FocusedInput>()),
        FocusedInput::default()
    );
    o.vcx.update(|window, _| window.activate_window());
    draw(o.vcx);
    assert_eq!(
        o.vcx.read(|cx| *cx.global::<FocusedInput>()),
        FocusedInput {
            entity: Some(me),
            composing: true
        }
    );
    commit(&mut o, "感じ");
    assert_eq!(text(&o), "SELECT ş感じ");
    assert!(!composing(&o));
    // Two compositions, two undo groups.
    assert_eq!(undo_all(&mut o), 2);
    assert_eq!(text(&o), "SELECT ");
}

/// IME commits are stamped with the executor's clock: two characters 100 ms apart join one
/// typing group, a third 700 ms later starts another.
#[gpui::test]
fn ime_commits_take_the_executor_clock(cx: &mut TestAppContext) {
    let mut o = open(cx, "", 0);
    commit(&mut o, "a");
    o.vcx.executor().advance_clock(Duration::from_millis(100));
    commit(&mut o, "b");
    o.vcx.executor().advance_clock(Duration::from_millis(700));
    commit(&mut o, "c");
    run(&mut o, "editor.undo");
    assert_eq!(text(&o), "ab");
    run(&mut o, "editor.undo");
    assert_eq!(text(&o), "");
}

// ---------------------------------------------------------------------------------------------
// notify_text_input_changed (R-M3-7) and app edits during a composition.

/// The platform hears of an edit only when it did not make it: typing and a composition send
/// nothing; a paste and an undo send one each (one per frame at most).
#[gpui::test]
fn notify_only_after_non_ime_edits(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT ", 7);
    let base = notifications(&o);
    commit(&mut o, "ş");
    draw(o.vcx);
    mark(&mut o, "か");
    draw(o.vcx);
    commit(&mut o, "仮");
    draw(o.vcx);
    assert_eq!(notifications(&o) - base, 0, "IME edits");
    o.vcx
        .write_to_clipboard(ClipboardItem::new_string("x".into()));
    run(&mut o, "editor.paste");
    draw(o.vcx);
    assert_eq!(text(&o), "SELECT ş仮x");
    assert_eq!(notifications(&o) - base, 1, "the paste");
    run(&mut o, "editor.undo");
    draw(o.vcx);
    assert_eq!(notifications(&o) - base, 2, "the undo");
    // A selection the app moves is a selection change, not a text change.
    let sel_before = o
        .vcx
        .read(|cx| o.editor.read(cx).text_selection_notifications());
    o.editor.update(o.vcx, |v, cx| v.select_range(0..6, cx));
    draw(o.vcx);
    assert_eq!(notifications(&o) - base, 2);
    let sel_after = o
        .vcx
        .read(|cx| o.editor.read(cx).text_selection_notifications());
    assert_eq!(sel_after - sel_before, 1);
}

/// A composition that an app path ends without a `Committed` (an undo on an empty history
/// after the unmark) still owes the platform its notification.
#[gpui::test]
fn a_composition_ended_by_an_app_path_owes_the_platform(cx: &mut TestAppContext) {
    let mut o = open(cx, "", 0);
    let base = notifications(&o);
    mark(&mut o, "ş");
    draw(o.vcx);
    assert_eq!(notifications(&o), base);
    run(&mut o, "editor.redo");
    draw(o.vcx);
    assert!(!composing(&o));
    assert_eq!(notifications(&o) - base, 1);
}

/// UI-EDIT-12 test 7 (corrected ruling, R-M3-7): GPUI cannot discard the platform's
/// composition, so an app edit made while composing waits until the composition ends, then
/// applies in order, each its own undo group: a programmatic `set_text`, a "go to error"
/// selection, a paste whose read landed while composing.
#[gpui::test]
fn app_edits_wait_for_the_composition_to_end(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT ", 7);
    o.vcx
        .write_to_clipboard(ClipboardItem::new_string("VALUES".into()));
    // The paste is asked for, then a composition starts before its read lands.
    o.editor.update_in(o.vcx, |v, window, cx| {
        v.run_command("editor.paste", window, cx);
        v.replace_and_mark_text_in_range(None, "ş", None, window, cx);
    });
    o.vcx.run_until_parked();
    o.editor.update(o.vcx, |v, cx| {
        v.set_text("SELECT 2", cx);
        v.select_range(0..6, cx);
    });
    o.vcx.run_until_parked();
    assert_eq!(text(&o), "SELECT ş", "nothing lands inside the composition");
    assert!(composing(&o));
    commit(&mut o, "ş");
    o.vcx.run_until_parked();
    // The paste landed first (its read finished first), then set_text and the selection.
    assert_eq!(text(&o), "SELECT 2");
    let sel = o
        .vcx
        .read(|cx| o.editor.read(cx).snapshot().selection.main().range());
    assert_eq!(sel, 0..6);
    assert!(!composing(&o));
    // composition, paste, set_text: three groups.
    assert_eq!(undo_all(&mut o), 3);
    assert_eq!(text(&o), "SELECT ");
}

// ---------------------------------------------------------------------------------------------
// ## m3a-t27-ime-clipboard (fix round 1)

fn selection(o: &Opened<'_>) -> std::ops::Range<usize> {
    o.vcx
        .read(|cx| o.editor.read(cx).snapshot().selection.main().range())
}

/// Review I1: a "go to error" held during a composition before its target is rebased over
/// what the composition committed: kana that becomes longer kanji, and a cancelled one.
#[gpui::test]
fn a_held_selection_follows_a_composition_of_another_length(cx: &mut TestAppContext) {
    let mut o = open(cx, "x FROM t", 0);
    mark(&mut o, "か");
    // `t` is at 10..11 in "かx FROM t".
    o.editor.update(o.vcx, |v, cx| v.select_range(10..11, cx));
    commit(&mut o, "漢字");
    o.vcx.run_until_parked();
    assert_eq!(text(&o), "漢字x FROM t");
    assert_eq!(selection(&o), 13..14, "still `t`");
    // A cancelled composition (an empty mark) takes its text away.
    o.editor.update(o.vcx, |v, cx| v.select_range(0..0, cx));
    mark(&mut o, "か");
    o.editor.update(o.vcx, |v, cx| v.select_range(16..17, cx));
    mark(&mut o, "");
    o.vcx.run_until_parked();
    assert_eq!(text(&o), "漢字x FROM t");
    assert_eq!(selection(&o), 13..14, "still `t`");
}

/// Review M3: held edits coalesce (a later `set_text` replaces earlier held texts and
/// selections; a selection replaces a trailing selection), and a direct edit made after a
/// composition ends but before the held ones apply waits behind them, so an older held edit
/// never overwrites a newer one.
#[gpui::test]
fn held_app_edits_coalesce_and_keep_their_order(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT ", 7);
    mark(&mut o, "ş");
    o.editor.update(o.vcx, |v, cx| {
        v.set_text("A", cx);
        v.select_range(0..1, cx);
        v.set_text("SELECT 22", cx);
        v.select_range(0..1, cx);
        v.select_range(0..6, cx);
    });
    commit(&mut o, "ş");
    o.vcx.run_until_parked();
    assert_eq!(text(&o), "SELECT 22");
    assert_eq!(selection(&o), 0..6);
    // The composition and one set_text: two groups.
    assert_eq!(undo_all(&mut o), 2);
    // The composition ends and, in the same update, the host selects directly: its selection
    // is the newer one and wins.
    o.editor
        .update(o.vcx, |v, cx| v.set_text("SELECT 1, 2", cx));
    o.vcx.run_until_parked();
    mark(&mut o, "x");
    o.editor.update(o.vcx, |v, cx| v.select_range(0..1, cx));
    o.editor.update_in(o.vcx, |v, window, cx| {
        v.unmark_text(window, cx);
        v.select_range(7..8, cx);
    });
    o.vcx.run_until_parked();
    assert_eq!(selection(&o), 7..8);
}

/// Review M4, case A: the focus moves while the window is inactive; GPUI sends the platform
/// `FocusLost` (it drops its composition) but no blur listener runs. Back in the app the
/// editor ends the composition locally, so bindings are not blocked and the next keystroke
/// does not replace the stranded text.
#[gpui::test]
fn a_composition_dropped_while_the_window_was_inactive_ends(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT ", 7);
    mark(&mut o, "ş");
    o.vcx.deactivate_window();
    let other = o.other.clone();
    o.vcx.update(|window, cx| window.focus(&other, cx));
    o.vcx.update(|window, _| window.activate_window());
    draw(o.vcx);
    assert!(!composing(&o));
    assert_eq!(text(&o), "SELECT ş");
    assert!(!o.vcx.read(|cx| cx.global::<FocusedInput>().composing));
}

/// Review M4, case B: the editor turns read-only mid-composition; it accepts no text from then
/// on (the platform drops the composition), so it ends the composition itself.
#[gpui::test]
fn a_composition_ends_when_the_editor_turns_read_only(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT ", 7);
    mark(&mut o, "ş");
    o.editor.update(o.vcx, |v, cx| {
        let cfg = EditorConfig {
            read_only: true,
            ..*v.config()
        };
        v.set_config(cfg, cx);
    });
    draw(o.vcx);
    assert!(!composing(&o));
    assert_eq!(text(&o), "SELECT ş");
    assert_eq!(composition_events(&o), [true, false]);
}

/// Review M5: a range on a row the current frame did not paint, with no composition, asks for
/// no frame (the next one could not answer it better).
#[gpui::test]
fn bounds_for_range_on_a_current_frame_requests_no_frame(cx: &mut TestAppContext) {
    let src: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let o = open(cx, &src, 0);
    let notified = Rc::new(std::cell::Cell::new(0u32));
    let n = notified.clone();
    o.vcx.update(|_, cx| {
        cx.observe(&o.editor, move |_, _| n.set(n.get() + 1))
            .detach();
    });
    let at16 = o.vcx.read(|cx| {
        let b = o.editor.read(cx).snapshot().buffer;
        b.byte_to_utf16(b.line_to_byte(150))
    });
    let b = o.editor.update_in(o.vcx, |v, window, cx| {
        let eb = v.last_layout().unwrap().rows().bounds;
        v.bounds_for_range(at16..at16, eb, window, cx)
    });
    o.vcx.run_until_parked();
    assert!(b.is_some(), "estimated");
    assert_eq!(notified.get(), 0);
}

/// Review M6: a range starting between the CR and the LF of a CRLF does not panic.
#[gpui::test]
fn frame_bounds_inside_a_crlf_do_not_panic(cx: &mut TestAppContext) {
    let o = open(cx, "a\r\nb", 0);
    let b = o.vcx.read(|cx| {
        let l = o.editor.read(cx).last_layout().unwrap().clone();
        l.bounds_for_range(2..3)
    });
    assert!(b.is_some());
}

/// Review M7: a point on the left half of a line's last character lands before it, on the
/// right half after it (ASCII, a two-byte letter, and a base with a combining mark).
#[gpui::test]
fn the_last_characters_halves_hit_before_and_after(cx: &mut TestAppContext) {
    for (src, start) in [("SELECT x", 7), ("SELECT ş", 7), ("SELECT e\u{301}", 7)] {
        let o = open(cx, src, 0);
        let (l, r) = o.vcx.read(|cx| {
            let l = o.editor.read(cx).last_layout().unwrap().clone();
            let x0 = l.x_for_offset(start).unwrap();
            let x1 = l.x_for_offset(src.len()).unwrap();
            let b = l.bounds_for_range(start..start).unwrap();
            let y = b.origin.y + b.size.height / 2.0;
            (
                l.offset_for_point(point(x0 + (x1 - x0) * 0.25, y)),
                l.offset_for_point(point(x0 + (x1 - x0) * 0.75, y)),
            )
        });
        assert_eq!(l, Some(start), "{src:?}: left half");
        assert_eq!(r, Some(src.len()), "{src:?}: right half");
    }
}

/// Review M8: the handler's UTF-16 answers after a CRLF and after a combining mark.
#[gpui::test]
fn crlf_and_combining_marks_at_the_handler(cx: &mut TestAppContext) {
    for (src, caret, sel16, marked16) in [("a\r\nb", 3, 3..3, 3..4), ("e\u{301}", 3, 2..2, 2..3)] {
        let mut o = open(cx, src, caret);
        let sel = o
            .editor
            .update_in(o.vcx, |v, window, cx| {
                v.selected_text_range(false, window, cx)
            })
            .unwrap();
        assert_eq!(sel.range, sel16, "{src:?}");
        mark(&mut o, "ş");
        let marked = o
            .editor
            .update_in(o.vcx, |v, window, cx| v.marked_text_range(window, cx));
        assert_eq!(marked, Some(marked16), "{src:?}");
    }
}

/// Review M8: two app edits in one frame are one notification.
#[gpui::test]
fn two_app_edits_in_one_frame_notify_once(cx: &mut TestAppContext) {
    let o = open(cx, "SELECT ", 7);
    let base = notifications(&o);
    o.editor.update(o.vcx, |v, cx| {
        v.set_text("SELECT 1", cx);
        v.set_text("SELECT 2", cx);
    });
    draw(o.vcx);
    assert_eq!(notifications(&o) - base, 1);
}
