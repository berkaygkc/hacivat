//! M3b Task 31: several carets in the editor view (UI-EDIT-05 multi, UI-EDIT-14 cases 2–3).
//! Every edit applies at each caret as one undo group; the clipboard carries one piece per
//! range (metadata, with a text-hash fallback when the platform drops it) and pastes N pieces
//! into N carets; Run follows the main caret (controller ruling 2); a modifier-drag selects a
//! column; the status counts carets. Tests wait on conditions (`run_until_parked`), never on
//! time. `┃` is a range's head, `«` its anchor.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AppContext as _, ClipboardEntry, ClipboardItem, Context, Entity, EntityInputHandler as _,
    Focusable as _, IntoElement, Modifiers, MouseButton, ParentElement as _, Pixels, Point, Render,
    Styled as _, TestAppContext, VisualTestContext, Window, div, point, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use hacivat_ui_editor::{
    DocId, EditorConfig, EditorEvent, EditorView, ExecAt, ExecRequest, FailClosedService,
};
use smallvec::smallvec;

struct Host {
    editor: Entity<EditorView>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(800.0)).h(px(600.0)).child(self.editor.clone())
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
                DocId(31),
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

/// The text with every range drawn (`┃` head, `«` anchor).
fn states(o: &Opened<'_>) -> String {
    o.vcx.read(|cx| {
        let s = o.editor.read(cx).snapshot();
        let text = s.buffer.to_string();
        let mut out = String::new();
        for (i, c) in text.char_indices().chain([(text.len(), '\0')]) {
            for r in s.selection.ranges() {
                if r.head == i {
                    out.push('┃');
                }
                if r.anchor == i && r.anchor != r.head {
                    out.push('«');
                }
            }
            if i < text.len() {
                out.push(c);
            }
        }
        out
    })
}

fn text(o: &Opened<'_>) -> String {
    o.vcx
        .read(|cx| o.editor.read(cx).snapshot().buffer.to_string())
}

fn run(o: &mut Opened<'_>, id: &str) {
    let ran = o
        .editor
        .update_in(o.vcx, |v, window, cx| v.run_command(id, window, cx));
    assert!(ran, "{id} is a command");
    o.vcx.run_until_parked();
}

fn type_text(o: &mut Opened<'_>, s: &str) {
    o.editor.update_in(o.vcx, |v, window, cx| {
        v.replace_text_in_range(None, s, window, cx);
    });
    o.vcx.run_until_parked();
}

fn put(o: &mut Opened<'_>, item: ClipboardItem) {
    o.vcx.write_to_clipboard(item);
}

fn clipboard(o: &Opened<'_>) -> (Option<String>, Option<String>) {
    let item = o.vcx.read_from_clipboard();
    let entry = item.as_ref().and_then(|i| {
        i.entries().iter().find_map(|e| match e {
            ClipboardEntry::String(s) => Some((s.text.clone(), s.metadata.clone())),
            _ => None,
        })
    });
    match entry {
        Some((t, m)) => (Some(t), m),
        None => (None, None),
    }
}

fn pieces_meta(pieces: &[(usize, bool)]) -> String {
    let p: Vec<String> = pieces
        .iter()
        .map(|(len, lw)| format!(r#"{{"len":{len},"line_wise":{lw}}}"#))
        .collect();
    format!(
        r#"{{"v":1,"src":"hacivat-editor","pieces":[{}]}}"#,
        p.join(",")
    )
}

/// Three carets: at `caret` and on the two lines below it, the last one the main caret.
fn three<'a>(cx: &'a mut TestAppContext, text: &str, caret: usize) -> Opened<'a> {
    let mut o = open(cx, text, caret);
    run(&mut o, "editor.add_caret_below");
    run(&mut o, "editor.add_caret_below");
    o
}

/// UI-EDIT-14 cases 2 and 3: three selections copied and pasted into three carets, one piece
/// each; three lines from another app spread one per caret; line-wise pieces from carets.
#[gpui::test]
fn copy_and_paste_n_pieces_into_n_carets(cx: &mut TestAppContext) {
    let mut o = three(cx, "SELECT a;\nSELECT b;\nSELECT c;\n", 7);
    run(&mut o, "editor.select_right");
    assert_eq!(states(&o), "SELECT «a┃;\nSELECT «b┃;\nSELECT «c┃;\n");
    run(&mut o, "editor.copy");
    assert_eq!(
        clipboard(&o),
        (
            Some("a\nb\nc".to_string()),
            Some(pieces_meta(&[(1, false), (1, false), (1, false)]))
        )
    );
    run(&mut o, "editor.delete_backward");
    assert_eq!(states(&o), "SELECT ┃;\nSELECT ┃;\nSELECT ┃;\n");
    run(&mut o, "editor.paste");
    assert_eq!(states(&o), "SELECT a┃;\nSELECT b┃;\nSELECT c┃;\n");
    // 3. From another app: three lines, three carets, one line each.
    put(&mut o, ClipboardItem::new_string("x\ny\nz".into()));
    run(&mut o, "editor.paste");
    assert_eq!(states(&o), "SELECT ax┃;\nSELECT by┃;\nSELECT cz┃;\n");
    // Carets without a selection copy their lines, each once, line-wise; pasted, each line goes
    // above its caret's line and the carets keep their columns.
    run(&mut o, "editor.copy");
    let line = |s: &str| (s.len(), true);
    assert_eq!(
        clipboard(&o),
        (
            Some("SELECT ax;\nSELECT by;\nSELECT cz;\n".to_string()),
            Some(pieces_meta(&[
                line("SELECT ax;\n"),
                line("SELECT by;\n"),
                line("SELECT cz;\n")
            ]))
        )
    );
    run(&mut o, "editor.paste");
    assert_eq!(
        states(&o),
        "SELECT ax;\nSELECT ax┃;\nSELECT by;\nSELECT by┃;\nSELECT cz;\nSELECT cz┃;\n"
    );
}

/// Text that is not N pieces for N carets goes whole to every caret: one line, two lines for
/// three carets, and metadata that no longer describes its text (the clipboard changed
/// outside Hacivat). Pieces whose metadata the platform dropped still distribute while the
/// text is the one copied (the text-hash fallback).
#[gpui::test]
fn foreign_clipboard_text_pastes_whole_at_each_caret(cx: &mut TestAppContext) {
    let mut o = three(cx, "a;\nb;\nc;\n", 1);
    put(&mut o, ClipboardItem::new_string("1".into()));
    run(&mut o, "editor.paste");
    assert_eq!(states(&o), "a1┃;\nb1┃;\nc1┃;\n");
    put(&mut o, ClipboardItem::new_string("x y".into()));
    run(&mut o, "editor.paste");
    assert_eq!(states(&o), "a1x y┃;\nb1x y┃;\nc1x y┃;\n");
    run(&mut o, "editor.undo");
    put(&mut o, ClipboardItem::new_string("x\ny".into()));
    run(&mut o, "editor.paste");
    assert_eq!(states(&o), "a1x\ny┃;\nb1x\ny┃;\nc1x\ny┃;\n");
    run(&mut o, "editor.undo");
    put(
        &mut o,
        ClipboardItem::new_string_with_metadata(
            "p\nq\nr\ns".into(),
            pieces_meta(&[(1, false), (1, false), (1, false)]),
        ),
    );
    run(&mut o, "editor.paste");
    assert_eq!(text(&o), "a1p\nq\nr\ns;\nb1p\nq\nr\ns;\nc1p\nq\nr\ns;\n");
    run(&mut o, "editor.undo");
    assert_eq!(states(&o), "a1┃;\nb1┃;\nc1┃;\n");
    // Three line breaks copied as three pieces (five breaks of text, six lines), then written
    // back by the platform without the metadata: one break per caret, not five.
    run(&mut o, "editor.move_right");
    run(&mut o, "editor.select_right");
    run(&mut o, "editor.copy");
    assert_eq!(clipboard(&o).0.as_deref(), Some("\n\n\n\n\n"));
    put(&mut o, ClipboardItem::new_string("\n\n\n\n\n".into()));
    run(&mut o, "editor.move_right");
    run(&mut o, "editor.paste");
    assert_eq!(states(&o), "a1;\n\n┃b1;\n\n┃c1;\n\n┃");
    // Once the clipboard holds other text, the record no longer applies.
    put(&mut o, ClipboardItem::new_string("\n\n\n\n".into()));
    run(&mut o, "editor.undo");
    run(&mut o, "editor.paste");
    assert_eq!(text(&o), "a1;\n\n\n\n\nb1;\n\n\n\n\nc1;\n\n\n\n\n");
}

/// Typing, a newline and a Backspace at three carets: each is one edit of all three, and the
/// typing is one undo group, as with one caret (UI-EDIT-12 test 5's undo, by typing).
#[gpui::test]
fn one_undo_removes_a_multi_caret_edit(cx: &mut TestAppContext) {
    let mut o = three(cx, "a\nb\nc", 1);
    assert_eq!(states(&o), "a┃\nb┃\nc┃");
    type_text(&mut o, "x");
    type_text(&mut o, "y");
    assert_eq!(states(&o), "axy┃\nbxy┃\ncxy┃");
    run(&mut o, "editor.undo");
    assert_eq!(states(&o), "a┃\nb┃\nc┃");
    run(&mut o, "editor.redo");
    assert_eq!(states(&o), "axy┃\nbxy┃\ncxy┃");
    run(&mut o, "editor.delete_backward");
    assert_eq!(states(&o), "ax┃\nbx┃\ncx┃");
    run(&mut o, "editor.newline");
    assert_eq!(states(&o), "ax\n┃\nbx\n┃\ncx\n┃");
    run(&mut o, "editor.undo");
    assert_eq!(states(&o), "ax┃\nbx┃\ncx┃");
    run(&mut o, "editor.undo");
    assert_eq!(states(&o), "axy┃\nbxy┃\ncxy┃");
    // Escape collapses to the main caret (the last added).
    run(&mut o, "editor.escape");
    assert_eq!(states(&o), "axy\nbxy\ncxy┃");
}

fn requests(o: &Opened<'_>) -> Vec<ExecRequest> {
    o.events
        .borrow()
        .iter()
        .filter_map(|e| match e {
            EditorEvent::ExecuteRequested(r) => Some(r.clone()),
            _ => None,
        })
        .collect()
}

/// Controller ruling 2: with carets in several statements, Run asks for the main caret's only
/// (the last one added); with several selections, for the main selection only.
#[gpui::test]
fn run_follows_the_main_caret(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT 1;\nSELECT 2;\nSELECT 3;", 3);
    run(&mut o, "editor.add_caret_below");
    run(&mut o, "editor.run");
    assert_eq!(requests(&o)[0].at, ExecAt::Carets(smallvec![13]));
    run(&mut o, "editor.add_caret_above");
    run(&mut o, "editor.run");
    assert_eq!(requests(&o)[1].at, ExecAt::Carets(smallvec![3]));
    // Two selections of `SELECT`: the second one is the main one.
    run(&mut o, "editor.escape");
    o.editor.update(o.vcx, |v, cx| v.select_range(0..0, cx));
    run(&mut o, "editor.select_next_occurrence");
    run(&mut o, "editor.select_next_occurrence");
    assert_eq!(states(&o), "«SELECT┃ 1;\n«SELECT┃ 2;\nSELECT 3;");
    run(&mut o, "editor.run");
    assert_eq!(requests(&o)[2].at, ExecAt::Selections(smallvec![10..16]));
}

/// The status bar's segment: how many carets, and the characters selected over every range.
#[gpui::test]
fn status_counts_carets_and_every_selected_character(cx: &mut TestAppContext) {
    let mut o = open(cx, "şey;\nşey;\nşey;", 0);
    assert_eq!(o.vcx.read(|cx| o.editor.read(cx).status().carets), 1);
    run(&mut o, "editor.select_all_occurrences");
    let s = o.vcx.read(|cx| o.editor.read(cx).status());
    assert_eq!((s.carets, s.selected_chars), (3, 9));
    assert_eq!(
        (s.line, s.column),
        (1, 4),
        "the main range is the asked-from one"
    );
}

/// A point at byte `at` of the painted text (its row's middle).
fn at_point(o: &Opened<'_>, at: usize) -> Point<Pixels> {
    o.vcx.read(|cx| {
        let l = o.editor.read(cx).last_layout().unwrap();
        let b = l.bounds_for_range(at..at).unwrap();
        point(b.origin.x + px(1.0), b.origin.y + px(9.0))
    })
}

/// The column-drag modifier: Option on macOS, Ctrl+Alt elsewhere (NOTES surface 14).
fn column_mods() -> Modifiers {
    if cfg!(target_os = "macos") {
        Modifiers::alt()
    } else {
        Modifiers {
            control: true,
            alt: true,
            ..Modifiers::default()
        }
    }
}

/// A modifier-drag selects a box in display columns, one range per line, the main one on the
/// pointer's line; a modifier-click without a drag adds a caret.
#[gpui::test]
fn a_modifier_drag_selects_a_column(cx: &mut TestAppContext) {
    let mut o = open(cx, "abcdef\nabcdef\nabcdef", 0);
    let (from, to) = (at_point(&o, 1), at_point(&o, 18));
    let m = column_mods();
    o.vcx.simulate_mouse_down(from, MouseButton::Left, m);
    o.vcx.simulate_mouse_move(to, MouseButton::Left, m);
    o.vcx.simulate_mouse_up(to, MouseButton::Left, m);
    o.vcx.run_until_parked();
    assert_eq!(states(&o), "a«bcd┃ef\na«bcd┃ef\na«bcd┃ef");
    // A click with the modifier adds a caret there (Option on macOS, Alt elsewhere).
    run(&mut o, "editor.escape");
    run(&mut o, "editor.escape");
    let add = Modifiers::alt();
    let p = at_point(&o, 9);
    o.vcx.simulate_mouse_down(p, MouseButton::Left, add);
    o.vcx.simulate_mouse_up(p, MouseButton::Left, add);
    o.vcx.run_until_parked();
    assert_eq!(states(&o), "abcdef\nab┃cdef\nabcd┃ef");
}

/// Two carets whose word deletions reach into each other: one deletion over both, one caret.
#[gpui::test]
fn deletions_that_overlap_merge(cx: &mut TestAppContext) {
    let mut o = open(cx, "abcdef gh", 3);
    let p = at_point(&o, 6);
    o.vcx
        .simulate_mouse_down(p, MouseButton::Left, Modifiers::alt());
    o.vcx
        .simulate_mouse_up(p, MouseButton::Left, Modifiers::alt());
    o.vcx.run_until_parked();
    assert_eq!(states(&o), "abc┃def┃ gh");
    run(&mut o, "editor.delete_word_backward");
    assert_eq!(states(&o), "┃ gh");
    run(&mut o, "editor.undo");
    assert_eq!(states(&o), "abc┃def┃ gh");
}

// ## m3c-t31-multi (fix round 1)

/// I1: an Option/Alt press adds a caret; a pointer that moves within the same cell before the
/// release starts no box, so every caret stays.
#[gpui::test]
fn a_modifier_click_that_jitters_in_place_keeps_every_caret(cx: &mut TestAppContext) {
    let mut o = open(cx, "abcdef\nabcdef\nabcdef", 0);
    run(&mut o, "editor.add_caret_below");
    let p = at_point(&o, 16);
    let m = Modifiers::alt();
    o.vcx.simulate_mouse_down(p, MouseButton::Left, m);
    o.vcx.simulate_mouse_move(p, MouseButton::Left, m);
    o.vcx
        .simulate_mouse_move(point(p.x + px(1.0), p.y), MouseButton::Left, m);
    o.vcx.simulate_mouse_up(p, MouseButton::Left, m);
    o.vcx.run_until_parked();
    assert_eq!(states(&o), "┃abcdef\n┃abcdef\nab┃cdef");
}

/// I3: whole lines copied at three carets stay whole lines when pasted at one caret: above its
/// line, its column kept.
#[gpui::test]
fn lines_copied_at_several_carets_paste_as_lines_at_one(cx: &mut TestAppContext) {
    let mut o = three(cx, "a;\nb;\nc;\n", 1);
    run(&mut o, "editor.copy");
    run(&mut o, "editor.escape");
    assert_eq!(states(&o), "a;\nb;\nc┃;\n");
    run(&mut o, "editor.paste");
    assert_eq!(states(&o), "a;\nb;\na;\nb;\nc;\nc┃;\n");
}

/// M1: a word deletion from the last caret reaches back over two earlier edits (a caret's and
/// a selection's): one deletion over all three, one caret, one undo.
#[gpui::test]
fn a_deletion_that_reaches_over_several_earlier_ones_merges_them(cx: &mut TestAppContext) {
    let mut o = open(cx, "abcdefgh", 0);
    o.editor.update(o.vcx, |v, cx| v.select_range(3..5, cx));
    for at in [2, 7] {
        let p = at_point(&o, at);
        o.vcx
            .simulate_mouse_down(p, MouseButton::Left, Modifiers::alt());
        o.vcx
            .simulate_mouse_up(p, MouseButton::Left, Modifiers::alt());
    }
    o.vcx.run_until_parked();
    assert_eq!(states(&o), "ab┃c«de┃fg┃h");
    run(&mut o, "editor.delete_word_backward");
    assert_eq!(states(&o), "┃h");
    run(&mut o, "editor.undo");
    assert_eq!(text(&o), "abcdefgh");
}

/// M2: a drag is one step of the selection history, and so is select all occurrences.
#[gpui::test]
fn one_gesture_is_one_undo_selection_step(cx: &mut TestAppContext) {
    let mut o = open(cx, "abc abc xyz", 9);
    let none = Modifiers::none();
    let start = at_point(&o, 1);
    o.vcx.simulate_mouse_down(start, MouseButton::Left, none);
    for at in [2, 3, 5] {
        let p = at_point(&o, at);
        o.vcx.simulate_mouse_move(p, MouseButton::Left, none);
    }
    let end = at_point(&o, 5);
    o.vcx.simulate_mouse_up(end, MouseButton::Left, none);
    o.vcx.run_until_parked();
    assert_eq!(states(&o), "a«bc a┃bc xyz");
    run(&mut o, "editor.undo_selection");
    assert_eq!(states(&o), "abc abc x┃yz", "the whole drag at once");
    o.editor.update(o.vcx, |v, cx| v.select_range(1..1, cx));
    run(&mut o, "editor.select_all_occurrences");
    assert_eq!(states(&o), "«abc┃ «abc┃ xyz");
    run(&mut o, "editor.undo_selection");
    assert_eq!(
        states(&o),
        "a┃bc abc xyz",
        "the word and the matches at once"
    );
}

/// M10: a search that began at a caret matches whole words; one from a range selected by hand
/// matches substrings (UI-EDIT-05 step 5).
#[gpui::test]
fn whole_words_only_when_the_search_began_at_a_caret(cx: &mut TestAppContext) {
    let mut o = open(cx, "id idx id", 1);
    run(&mut o, "editor.select_next_occurrence");
    run(&mut o, "editor.select_next_occurrence");
    assert_eq!(states(&o), "«id┃ idx «id┃");
    o.editor.update(o.vcx, |v, cx| v.select_range(0..2, cx));
    run(&mut o, "editor.select_next_occurrence");
    assert_eq!(states(&o), "«id┃ «id┃x id");
}
