//! M4b Task 11: reload from disk (HV-SCRIPT-08, UI-EDIT-04 step 9, UI-EDIT-12; R-M4b-16,
//! R-M4b-38). One `Origin::Program` change from the common prefix and suffix, carets and
//! selections mapped through it, the undo history cleared, held while an IME composition is
//! open and applied once it ends, refused on a T2 preview, and the cached editor redrawn by
//! its own notification. The diff runs on the background executor: every test waits on a
//! condition (`run_until_parked`), never on a clock.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{
    AnyView, AppContext as _, Context, Entity, EntityInputHandler as _, Focusable as _,
    IntoElement, ParentElement as _, Render, StyleRefinement, Styled as _, TestAppContext,
    VisualTestContext, Window, div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use hacivat_ui_editor::{
    DocId, EditorConfig, EditorEvent, EditorView, FailClosedService, PreviewSpec, ReloadRefused,
    Tier, disk_edit,
};

/// One editor in an 800 × 600 box; with `cached`, embedded as `AnyView::cached`, as the
/// console and script hosts do. The root counts its renders.
struct Host {
    editor: Entity<EditorView>,
    cached: bool,
    renders: Rc<Cell<u32>>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        let body = div().w(px(800.0)).h(px(600.0));
        if self.cached {
            div().child(body.child(
                AnyView::from(self.editor.clone()).cached(StyleRefinement::default().size_full()),
            ))
        } else {
            div().child(body.child(self.editor.clone()))
        }
    }
}

struct Opened<'a> {
    editor: Entity<EditorView>,
    events: Rc<RefCell<Vec<EditorEvent>>>,
    renders: Rc<Cell<u32>>,
    vcx: &'a mut VisualTestContext,
}

fn draw(vcx: &mut VisualTestContext) {
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
}

fn open_with<'a>(cx: &'a mut TestAppContext, text: &str, cached: bool) -> Opened<'a> {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let text = text.to_string();
    let slot: Rc<RefCell<Option<Entity<EditorView>>>> = Rc::default();
    let renders = Rc::new(Cell::new(0));
    let (out, count) = (slot.clone(), renders.clone());
    let (_, vcx) = cx.add_window_view(move |window, cx| {
        let editor = cx.new(|cx| {
            EditorView::new(
                DocId(11),
                &text,
                EditorConfig::default(),
                Rc::new(FailClosedService),
                Rc::new(|_: &gpui::App| "orders.sql".into()),
                window,
                cx,
            )
        });
        *out.borrow_mut() = Some(editor.clone());
        Host {
            editor,
            cached,
            renders: count,
        }
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
    });
    draw(vcx);
    events.borrow_mut().clear();
    Opened {
        editor,
        events,
        renders,
        vcx,
    }
}

fn open<'a>(cx: &'a mut TestAppContext, text: &str) -> Opened<'a> {
    open_with(cx, text, false)
}

fn text(o: &Opened<'_>) -> String {
    o.vcx
        .read(|cx| o.editor.read(cx).snapshot().buffer.to_string())
}

fn version(o: &Opened<'_>) -> u64 {
    o.vcx.read(|cx| o.editor.read(cx).snapshot().version)
}

/// The main caret as (line, column in bytes), both from 0.
fn caret(o: &Opened<'_>) -> (usize, usize) {
    o.vcx.read(|cx| {
        let snap = o.editor.read(cx).snapshot();
        let head = snap.selection.main().head;
        let line = snap.buffer.byte_to_line(head);
        (line, head - snap.buffer.line_to_byte(line))
    })
}

fn selection(o: &Opened<'_>) -> (usize, usize) {
    o.vcx.read(|cx| {
        let r = o.editor.read(cx).snapshot().selection.main().range();
        (r.start, r.end)
    })
}

fn select(o: &mut Opened<'_>, r: std::ops::Range<usize>) {
    o.editor.update(o.vcx, |v, cx| v.select_range(r, cx));
    o.vcx.run_until_parked();
}

/// Sets the caret at (line, column), both from 0.
fn set_caret(o: &mut Opened<'_>, (line, col): (usize, usize)) {
    let at = o.vcx.read(|cx| {
        let snap = o.editor.read(cx).snapshot();
        snap.buffer.line_to_byte(line) + col
    });
    select(o, at..at);
}

fn reload(o: &mut Opened<'_>, disk: &str) -> Result<(), ReloadRefused> {
    let got = o
        .editor
        .update(o.vcx, |v, cx| v.replace_from_disk(disk, cx));
    o.vcx.run_until_parked();
    got
}

fn run(o: &mut Opened<'_>, id: &str) {
    let ran = o
        .editor
        .update_in(o.vcx, |v, window, cx| v.run_command(id, window, cx));
    assert!(ran, "{id} is a command");
    o.vcx.run_until_parked();
}

fn can_undo(o: &Opened<'_>) -> bool {
    o.vcx
        .read(|cx| hacivat_ui_editor::commands::can_undo(o.editor.read(cx)))
}

fn can_redo(o: &Opened<'_>) -> bool {
    o.vcx
        .read(|cx| hacivat_ui_editor::commands::can_redo(o.editor.read(cx)))
}

/// Types `s` at the caret through the platform input handler, as a keyboard does.
fn type_text(o: &mut Opened<'_>, s: &str) {
    o.editor.update_in(o.vcx, |v, window, cx| {
        v.replace_text_in_range(None, s, window, cx);
    });
    o.vcx.run_until_parked();
}

fn changed(o: &Opened<'_>) -> usize {
    o.events
        .borrow()
        .iter()
        .filter(|e| matches!(e, EditorEvent::Changed { .. }))
        .count()
}

fn reloaded(o: &Opened<'_>) -> Vec<Option<u64>> {
    o.events
        .borrow()
        .iter()
        .filter_map(|e| match e {
            EditorEvent::Reloaded { version } => Some(*version),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------

/// The plan's case: a line added above the caret moves it down one line, at the same column.
#[gpui::test]
fn reload_keeps_caret_line_and_column(cx: &mut TestAppContext) {
    let mut o = open(cx, "a\nb\nc\n");
    set_caret(&mut o, (2, 0));
    reload(&mut o, "x\na\nb\nc\n").unwrap();
    assert_eq!(text(&o), "x\na\nb\nc\n");
    assert_eq!(caret(&o), (3, 0));
    // A column is kept too: a line inserted above a caret inside a line.
    set_caret(&mut o, (2, 1));
    reload(&mut o, "y\nx\na\nb\nc\n").unwrap();
    assert_eq!(caret(&o), (3, 1));
}

/// A selection and its direction map through the change, as the caret does.
#[gpui::test]
fn reload_maps_the_selection(cx: &mut TestAppContext) {
    let src = "SELECT 1;\nSELECT müşteri FROM t;\nSELECT 3;\n";
    let mut o = open(cx, src);
    let start = src.find("müşteri").unwrap();
    select(&mut o, start..start + "müşteri".len());
    // A line inserted above the selection moves it, whole.
    let disk = "-- header\nSELECT 1;\nSELECT müşteri FROM t;\nSELECT 3;\n";
    reload(&mut o, disk).unwrap();
    let moved = disk.find("müşteri").unwrap();
    assert_eq!(selection(&o), (moved, moved + "müşteri".len()));
    // An edit below it leaves it where it is.
    let disk = "-- header\nSELECT 1;\nSELECT müşteri FROM t;\nSELECT 4;\n";
    reload(&mut o, disk).unwrap();
    assert_eq!(selection(&o), (moved, moved + "müşteri".len()));
    // The one change runs from the first byte that differs to the last: edits above and below
    // at once make it cover the selection, which then ends where the change does (the
    // documented limit of a prefix and suffix diff, ADR 0007 m4b-t11).
    let disk = "-- top\nSELECT 1;\nSELECT müşteri FROM t;\nSELECT 5;\n";
    reload(&mut o, disk).unwrap();
    assert_eq!(text(&o), disk);
    let end = disk.len() - ";\n".len();
    assert_eq!(selection(&o), (end, end));
}

/// One change, one `Changed`, one version, and nothing left to undo or redo
/// (UI-EDIT-04 step 9).
#[gpui::test]
fn reload_is_one_change_and_clears_undo(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT 1;\n");
    o.editor.update(o.vcx, |v, cx| v.select_range(9..9, cx));
    o.vcx.run_until_parked();
    type_text(&mut o, " -- a");
    // A move away seals the group: the next edit is a group of its own.
    select(&mut o, 0..0);
    type_text(&mut o, "-- b\n");
    run(&mut o, "editor.undo");
    assert!(can_undo(&o), "the saved edit is undoable before the reload");
    assert!(can_redo(&o), "and a redo waits");
    o.events.borrow_mut().clear();
    let before = version(&o);
    // The file changed on disk at both ends and in the middle: still one change, from the
    // first byte that differs to the last.
    let disk = "-- top\nSELECT 2;\n-- end\n";
    reload(&mut o, disk).unwrap();
    assert_eq!(text(&o), disk);
    assert_eq!(changed(&o), 1, "one Changed: {:?}", o.events.borrow());
    assert_eq!(version(&o), before + 1, "one version");
    assert_eq!(reloaded(&o), [Some(before + 1)]);
    assert!(!can_undo(&o), "undo has nothing to take back");
    assert!(!can_redo(&o), "nor redo");
    run(&mut o, "editor.undo");
    assert_eq!(text(&o), disk, "undo after a reload changes nothing");
    // The next edit is undoable on its own, back to the disk text.
    type_text(&mut o, "x");
    run(&mut o, "editor.undo");
    assert_eq!(text(&o), disk);
    assert!(!can_undo(&o));
}

/// R-M4b-38: while a composition is open the reload is held (accepted, nothing replaced, the
/// composition intact); when it ends the disk text is applied once, with the carets mapped.
#[gpui::test]
fn reload_is_held_during_a_composition_and_applied_after(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT 1;\nSELECT 2;\n");
    set_caret(&mut o, (1, 9));
    o.editor.update_in(o.vcx, |v, window, cx| {
        v.replace_and_mark_text_in_range(None, "か", None, window, cx);
    });
    o.vcx.run_until_parked();
    let composing = |o: &Opened<'_>| o.vcx.read(|cx| o.editor.read(cx).status().composing);
    assert!(composing(&o));
    o.events.borrow_mut().clear();
    let disk = "-- reloaded\nSELECT 1;\nSELECT 2;\n";
    assert_eq!(
        reload(&mut o, disk),
        Ok(()),
        "accepted: the host never retries"
    );
    assert_eq!(text(&o), "SELECT 1;\nSELECT 2;か\n", "nothing replaced yet");
    assert!(composing(&o), "the composition is intact");
    assert_eq!(reloaded(&o), Vec::<Option<u64>>::new(), "not applied yet");
    // A second disk event while it waits: only the newest text is applied.
    assert_eq!(reload(&mut o, disk), Ok(()));
    // The composition ends with its commit; then the disk text lands, once.
    o.editor.update_in(o.vcx, |v, window, cx| {
        v.replace_text_in_range(None, "仮", window, cx);
    });
    o.vcx.run_until_parked();
    assert!(!composing(&o));
    assert_eq!(text(&o), disk);
    let landed = reloaded(&o);
    assert_eq!(landed.len(), 1, "applied once: {landed:?}");
    assert_eq!(landed[0], Some(version(&o)));
    // The caret was after the committed text at the end of line 1; that text is gone, and
    // the caret maps to where it stood, on the line the header pushed down.
    assert_eq!(caret(&o).0, 2);
    // Nothing more lands later.
    o.vcx.run_until_parked();
    assert_eq!(reloaded(&o).len(), 1);
    assert_eq!(text(&o), disk);
}

/// The same text again is no change: no new version, no `Changed`, the history kept.
#[gpui::test]
fn reload_of_identical_text_is_a_no_op(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT 1;\n");
    o.editor.update(o.vcx, |v, cx| v.select_range(9..9, cx));
    o.vcx.run_until_parked();
    type_text(&mut o, "x");
    let disk = text(&o);
    o.events.borrow_mut().clear();
    let before = version(&o);
    reload(&mut o, &disk).unwrap();
    assert_eq!(version(&o), before);
    assert_eq!(changed(&o), 0);
    assert_eq!(reloaded(&o), [Some(before)], "the host's clean version");
    assert!(can_undo(&o), "a touch on disk takes nothing away");
}

/// An edit that lands between the call and the background diff wins: the tab is dirty now,
/// and a dirty tab never reloads (R-M4b-16). Nothing is replaced and the host hears it.
#[gpui::test]
fn an_edit_before_the_diff_lands_keeps_the_text(cx: &mut TestAppContext) {
    let o = open(cx, "SELECT 1;\n");
    o.editor.update(o.vcx, |v, cx| v.select_range(9..9, cx));
    o.vcx.run_until_parked();
    o.events.borrow_mut().clear();
    o.editor.update_in(o.vcx, |v, window, cx| {
        v.replace_from_disk("SELECT 2;\n", cx).unwrap();
        // Typed before the background diff came back.
        v.replace_text_in_range(None, "x", window, cx);
    });
    o.vcx.run_until_parked();
    assert_eq!(text(&o), "SELECT 1;x\n");
    assert_eq!(reloaded(&o), [None]);
}

/// The change never starts or ends between a CR and its LF, nor inside a character: checked
/// on the edit itself, and on the editor's text after each reload.
#[gpui::test]
fn reload_never_splits_a_crlf_or_a_multibyte_character(cx: &mut TestAppContext) {
    let pairs: &[(&str, &str)] = &[
        ("a\r\nb", "a\rb"),
        ("a\r\nb", "a\nb"),
        ("a\rb", "a\r\nb"),
        ("a\nb", "a\r\nb"),
        ("a\r\n", "a\n"),
        ("a\n", "a\r\n"),
        ("x\r\n\r\ny", "x\r\ny"),
        ("é", "è"),
        ("aé", "aèé"),
        ("şş", "ş"),
        ("İstanbul", "Istanbul"),
        ("😀", "😁"),
        ("a😀b", "a😁b"),
        ("SELECT 'ğ';\r\n", "SELECT 'ü';\r\n"),
        ("", "SELECT 1;"),
        ("SELECT 1;", ""),
    ];
    for (old, new) in pairs {
        let e = disk_edit(old, new).expect("they differ");
        for (text, r) in [(old, &e.old), (new, &e.new)] {
            for at in [r.start, r.end] {
                assert!(text.is_char_boundary(at), "{old:?} → {new:?}: {e:?}");
                let crlf =
                    at > 0 && text.as_bytes()[at - 1] == b'\r' && text[at..].starts_with('\n');
                assert!(!crlf, "{old:?} → {new:?} splits a CRLF at {at}: {e:?}");
            }
        }
        // The edit reproduces the new text, and the prefix and suffix are common.
        assert_eq!(old[..e.old.start], new[..e.new.start]);
        assert_eq!(old[e.old.end..], new[e.new.end..]);
        assert_eq!(e.old.start, e.new.start);
    }
    assert!(disk_edit("same", "same").is_none());
    // Through the editor: the caret after a CRLF stays after it.
    let mut o = open(cx, "a\r\nb\r\nc");
    set_caret(&mut o, (2, 0));
    reload(&mut o, "a\r\nB\r\nc").unwrap();
    assert_eq!(text(&o), "a\r\nB\r\nc");
    assert_eq!(caret(&o), (2, 0));
    reload(&mut o, "a\nB\r\nc").unwrap();
    assert_eq!(text(&o), "a\nB\r\nc");
    assert_eq!(caret(&o), (2, 0));
}

/// A run's marks follow the reload as they follow an edit (the M3 rule): one whose statement
/// the change does not touch keeps its state and moves with its text; one it touches goes
/// stale.
#[gpui::test]
fn a_reload_maps_run_marks_and_stales_the_ones_it_touches(cx: &mut TestAppContext) {
    use hacivat_ui_editor::{ExecMarkState, MarkKey};
    let src = "SELECT 1;\nSELECT 2;\nSELECT 3;\n";
    let mut o = open(cx, src);
    let (s2, s3) = (10..18, 20..28);
    let v0 = version(&o);
    o.editor.update(o.vcx, |v, cx| {
        v.add_exec_marks(
            v0,
            vec![(MarkKey(2), s2.clone()), (MarkKey(3), s3.clone())],
            cx,
        );
        for k in [2, 3] {
            v.set_exec_state(
                MarkKey(k),
                ExecMarkState::Succeeded {
                    took: std::time::Duration::from_millis(3),
                    rows: None,
                },
                None,
                None,
                cx,
            );
        }
    });
    o.vcx.run_until_parked();
    // The third statement changed on disk: its mark goes stale; the second's stays.
    reload(&mut o, "SELECT 1;\nSELECT 2;\nSELECT 4;\n").unwrap();
    let (stale2, stale3, r2) = o.vcx.read(|cx| {
        let m = o.editor.read(cx).marks();
        (
            m.is_stale(MarkKey(2)),
            m.is_stale(MarkKey(3)),
            m.range(MarkKey(2)),
        )
    });
    assert!(!stale2 && stale3, "{stale2} {stale3}");
    assert_eq!(r2, Some(s2.clone()));
    // A header line added above: the second statement's mark moves with it, still current.
    reload(&mut o, "-- top\nSELECT 1;\nSELECT 2;\nSELECT 4;\n").unwrap();
    let (stale2, r2) = o.vcx.read(|cx| {
        let m = o.editor.read(cx).marks();
        (m.is_stale(MarkKey(2)), m.range(MarkKey(2)))
    });
    assert!(!stale2);
    assert_eq!(r2, Some(s2.start + 7..s2.end + 7));
}

/// A T2 preview is reopened by the host, never replaced in place.
#[gpui::test]
fn reload_in_preview_tier_is_refused(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT 1;\n");
    o.editor.update(o.vcx, |v, cx| {
        v.set_preview(
            PreviewSpec {
                shown_bytes: 10,
                total_bytes: 30 << 20,
            },
            cx,
        )
    });
    o.vcx.run_until_parked();
    assert_eq!(
        o.vcx.read(|cx| o.editor.read(cx).status().tier),
        Tier::Preview
    );
    let before = version(&o);
    assert_eq!(
        reload(&mut o, "SELECT 2;\n"),
        Err(ReloadRefused::PreviewTier)
    );
    assert_eq!(text(&o), "SELECT 1;\n");
    assert_eq!(version(&o), before);
}

/// Nothing above T1's 20 MB enters the editor by a reload either (R-M3-49): the host opens
/// such a file as a preview.
#[gpui::test]
fn reload_past_the_reduced_limit_is_refused(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT 1;\n");
    let big = "x".repeat((20 << 20) + 1);
    assert_eq!(
        reload(&mut o, &big),
        Err(ReloadRefused::TooLarge {
            bytes: (20 << 20) + 1
        })
    );
    assert_eq!(text(&o), "SELECT 1;\n");
}

/// The editor is embedded as a cached view: the reload notifies the editor itself, so the next
/// frame draws the new text with no other event (no window refresh, no parent render).
#[gpui::test]
fn reload_redraws_the_cached_editor(cx: &mut TestAppContext) {
    let o = open_with(cx, "SELECT 1;\n", true);
    let frames = o.renders.get();
    o.editor.update(o.vcx, |v, cx| {
        v.replace_from_disk("SELECT 1;\nSELECT 2;\nSELECT 3;\n", cx)
            .unwrap()
    });
    o.vcx.run_until_parked();
    let (painted, lines) = o.vcx.read(|cx| {
        let l = o.editor.read(cx).last_layout().cloned().expect("a frame");
        (l.version(), l.painted_lines())
    });
    assert_eq!(painted, version(&o), "the frame is of the reloaded text");
    assert!(
        lines.contains(&2),
        "the new third line is painted: {lines:?}"
    );
    // The frame was the editor's own request: nothing refreshed the window, and the root (which
    // renders once per drawn frame) drew exactly that one frame.
    assert_eq!(o.renders.get(), frames + 1);
}
