//! M3b Task 1b: the editor's and the text core's M3b seams exist and fail closed. Folding,
//! find, multi-cursor and the line commands change nothing until Tasks 31–34 fill them.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    TestAppContext, VisualTestContext, Window, div,
};
use hacivat_text::display::HeightMap;
use hacivat_text::{FoldSet, SelectionSet, TextBuffer};
use hacivat_ui_editor::{
    DocId, EditorConfig, EditorView, ExecMarkState, FailClosedService, FindMatcher, FindStatus,
    LanguageService, MarkKey, commands,
};

#[test]
fn folding_ranges_default_is_empty() {
    assert!(FailClosedService.folding_ranges(DocId(1), 0).is_empty());
}

// ## m3c-t32-fold (named exception): Task 32 filled the chunked map; its tests are in
// `hacivat-text/tests/fold.rs`. What stays here: without a hidden run it is the uniform map.
#[test]
fn chunked_height_map_without_folds_is_uniform() {
    let chunked = HeightMap::chunked(100, 18.0, &[]);
    assert_eq!(chunked.y_of(50), HeightMap::uniform(100, 18.0).y_of(50));
    assert_eq!(chunked.total_height(), 1800.0);
    let folded = HeightMap::chunked(100, 18.0, &[3..10, 40..41]);
    assert_eq!(folded.total_height(), 92.0 * 18.0);
}

// ## m3c-t31-multi (named exception): Task 31 filled the text core's multi-cursor stubs; their
// test moved to `hacivat-text/tests/multi.rs`. What stays here: the bodies never touch text a
// selection does not ask for (a box past the last line clamps to it).
#[test]
fn text_core_multi_cursor_bodies_clamp_to_the_text() {
    let buf = TextBuffer::from_str("SELECT a, a\nFROM t;\n");
    let boxed = hacivat_text::column_selection(&buf, (0, 0), (99, 0), 4);
    assert_eq!(
        boxed.ranges().len(),
        3,
        "one caret per line, the last one empty"
    );
    assert_eq!(boxed.main().head, buf.len_bytes());
    let sel = SelectionSet::cursor(7);
    assert_eq!(
        hacivat_text::add_caret_vertical(&buf, &SelectionSet::cursor(0), false, 4),
        SelectionSet::cursor(0)
    );
    assert_eq!(hacivat_text::split_into_lines(&buf, &sel), sel);
}

// ## m3c-t32-fold (named exception): Task 32 filled the fold set (tests
// `hacivat-text/tests/fold.rs`). What stays here: a range that does not fit the text folds
// nothing.
#[test]
fn fold_set_refuses_a_range_past_the_text() {
    let buf = TextBuffer::from_str("SELECT 1\nFROM t\nWHERE x;\n");
    let mut folds = FoldSet::default();
    assert!(!folds.fold(0..200, &buf));
    assert!(!folds.unfold_at(3));
    assert!(!folds.unfold_all());
    assert!(folds.folded().is_empty());
    assert!(folds.hidden_lines(&buf).is_empty());
}

/// Every M3b editor command id (Tasks 31–34) is in the table, titled `hv-cmd-editor-*`.
#[test]
fn m3b_editor_commands_are_in_the_table() {
    let ids: Vec<&str> = commands().iter().map(|c| c.id).collect();
    // ## m3c-t31-multi (named exception): Task 31's rows are commands, no longer stubs.
    for id in [
        "editor.add_caret_above",
        "editor.add_caret_below",
        "editor.select_next_occurrence",
        "editor.select_all_occurrences",
        "editor.split_selection_into_lines",
        "editor.undo_selection",
        "editor.redo_selection",
    ] {
        let c = commands().iter().find(|c| c.id == id).unwrap();
        assert_eq!(
            c.title_id,
            format!("hv-cmd-{}", id.replace(['.', '_'], "-"))
        );
        assert!(!c.stub && !c.seals_history, "{id}");
    }
    // ## m3c-t32-fold (named exception): Task 32's rows are commands (tests `fold_view`).
    for id in [
        "editor.fold",
        "editor.unfold",
        "editor.fold_all",
        "editor.unfold_all",
    ] {
        let c = commands().iter().find(|c| c.id == id).unwrap();
        assert!(!c.stub && !c.seals_history, "{id}");
    }
    // ## m3c-t33-find (named exception): Task 33's rows are commands (tests `find`).
    for id in [
        "editor.find",
        "editor.find_next",
        "editor.find_prev",
        "editor.find_close",
    ] {
        let c = commands().iter().find(|c| c.id == id).unwrap();
        assert!(!c.stub && !c.seals_history, "{id}");
    }
    for id in [
        "editor.toggle_comment",
        "editor.duplicate_line",
        "editor.delete_line",
        "editor.move_line_up",
        "editor.move_line_down",
        "editor.normalize_line_endings",
        "editor.history_prev",
        "editor.history_next",
    ] {
        assert!(ids.contains(&id), "{id}");
        let title = format!("hv-cmd-{}", id.replace(['.', '_'], "-"));
        let c = commands().iter().find(|c| c.id == id).unwrap();
        assert_eq!(c.title_id, title);
        // ## m3c-t34-editor (named exception): Task 34's rows are commands (tests `lines`),
        // none sealing the history.
        assert!(!c.seals_history, "{id}");
        assert!(!c.stub, "{id}");
    }
    // ## m3c-t33-find (named exception): 12 less Task 33's four.
    // ## m3c-t34-editor (named exception): 8 less Task 34's eight: no editor stub is left.
    assert_eq!(commands().iter().filter(|c| c.stub).count(), 0);
    // M3a's commands are not stubs.
    for c in commands().iter().take(45) {
        assert!(!c.stub, "{}", c.id);
    }
}

struct Host {
    editor: Entity<EditorView>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.editor.clone())
    }
}

fn editor<'a>(
    cx: &'a mut TestAppContext,
    text: &'static str,
) -> (Entity<EditorView>, &'a mut VisualTestContext) {
    let slot: Rc<RefCell<Option<Entity<EditorView>>>> = Rc::default();
    let out = slot.clone();
    let (_, vcx) = cx.add_window_view(move |window, cx| {
        let editor = cx.new(|cx| {
            EditorView::new(
                DocId(7),
                text,
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
    (editor, vcx)
}

/// A matcher that finds `SELECT` (the find seams; ## m3c-t33-find: no longer stubs).
struct Select;

impl FindMatcher for Select {
    fn find(&self, hay: &str, from: usize) -> Option<std::ops::Range<usize>> {
        hay.get(from..)?
            .find("SELECT")
            .map(|i| from + i..from + i + 6)
    }

    fn find_back(&self, hay: &str, before: usize) -> Option<std::ops::Range<usize>> {
        hay.get(..before)?.rfind("SELECT").map(|i| i..i + 6)
    }
}

#[gpui::test]
fn view_seams_change_nothing(cx: &mut TestAppContext) {
    let (e, vcx) = editor(cx, "SELECT 1;\nSELECT 2;");
    let before = vcx.read(|cx| e.read(cx).snapshot());
    // ## m3c-t33-find (named exception): find is filled (tests `find`); only the fold seam
    // is checked here, and a cleared query leaves the status at its default.
    let folded = e.update(vcx, |v, cx| {
        v.set_find(Some(std::sync::Arc::new(Select)), cx);
        v.set_find(None, cx);
        v.toggle_fold_at(0, cx)
    });
    assert!(!folded);
    assert_eq!(
        vcx.read(|cx| e.read(cx).find_status()),
        FindStatus::default()
    );
    assert!(vcx.read(|cx| e.read(cx).folds().folded().is_empty()));
    // Every M3b command runs as a no-op: the text, the selection and the version stay.
    // ## m3c-t31-multi (named exception): Task 31's commands are filled (tests `multi_view`).
    // ## m3c-t33-find (named exception): `editor.find` emits `FindRequested` (tests `find`).
    // ## m3c-t34-editor (named exception): Task 34's line commands edit (tests `lines`); recall
    // only asks the host, and not on this non-empty text.
    for id in ["editor.fold_all", "editor.history_prev"] {
        let ran = e.update_in(vcx, |v, window, cx| v.run_command(id, window, cx));
        assert!(ran, "{id}");
    }
    let after = vcx.read(|cx| e.read(cx).snapshot());
    assert_eq!(after.buffer.to_string(), before.buffer.to_string());
    assert_eq!(after.selection, before.selection);
    assert_eq!(after.version, before.version);
    // An escalated mark is kept (## m3c-t34-editor: its badge is tested in `gutter_m3b`).
    let since = std::time::Instant::now();
    e.update(vcx, |v, cx| {
        v.add_exec_marks(0, vec![(MarkKey(1), 0..8)], cx);
        v.set_exec_state(
            MarkKey(1),
            ExecMarkState::Escalated { since },
            None,
            None,
            cx,
        );
    });
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    assert_eq!(
        vcx.read(|cx| e.read(cx).marks().state(MarkKey(1)).cloned()),
        Some(ExecMarkState::Escalated { since })
    );
}
