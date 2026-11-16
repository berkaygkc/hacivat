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

#[test]
fn chunked_height_map_stub_is_uniform() {
    let chunked = HeightMap::chunked(100, 18.0, &[3..10, 40..41]);
    assert_eq!(chunked, HeightMap::uniform(100, 18.0));
    assert_eq!(chunked.y_of(50), 900.0);
    assert_eq!(chunked.total_height(), 1800.0);
}

#[test]
fn text_core_stubs_keep_the_selection() {
    let buf = TextBuffer::from_str("SELECT a, a\nFROM t;\n");
    let sel = SelectionSet::cursor(7);
    assert_eq!(hacivat_text::add_caret(&sel, 10), sel);
    assert_eq!(hacivat_text::add_caret_vertical(&buf, &sel, true, 4), sel);
    assert_eq!(hacivat_text::select_next_occurrence(&buf, &sel), None);
    assert_eq!(
        hacivat_text::select_all_occurrences(&buf, &sel, hacivat_text::MAX_OCCURRENCES),
        sel
    );
    assert_eq!(hacivat_text::split_into_lines(&buf, &sel), sel);
    // The column-selection stub is one caret at the start of the head's line (clamped).
    assert_eq!(
        hacivat_text::column_selection(&buf, (0, 0), (1, 3), 4),
        SelectionSet::cursor(12)
    );
    assert_eq!(
        hacivat_text::column_selection(&buf, (0, 0), (99, 0), 4),
        SelectionSet::cursor(buf.line_to_byte(buf.len_lines() - 1))
    );
}

#[test]
fn fold_set_stub_folds_nothing() {
    let buf = TextBuffer::from_str("SELECT 1\nFROM t\nWHERE x;\n");
    let mut folds = FoldSet::default();
    assert!(!folds.fold(0..20));
    assert!(!folds.unfold_at(3));
    assert!(!folds.unfold_all());
    assert!(folds.folded().is_empty());
    assert!(folds.hidden_lines(&buf).is_empty());
}

/// Every M3b editor command id (Tasks 31–34) is in the table, titled `hv-cmd-editor-*`.
#[test]
fn m3b_editor_commands_are_in_the_table() {
    let ids: Vec<&str> = commands().iter().map(|c| c.id).collect();
    for id in [
        "editor.add_caret_above",
        "editor.add_caret_below",
        "editor.select_next_occurrence",
        "editor.select_all_occurrences",
        "editor.split_selection_into_lines",
        "editor.undo_selection",
        "editor.redo_selection",
        "editor.fold",
        "editor.unfold",
        "editor.fold_all",
        "editor.unfold_all",
        "editor.find",
        "editor.find_next",
        "editor.find_prev",
        "editor.find_close",
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
        // A stub never seals the history: it changes nothing at all.
        assert!(!c.seals_history, "{id}");
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

/// A matcher that finds `SELECT`, to prove the stubs ignore it.
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
    let (step, all, folded) = e.update(vcx, |v, cx| {
        v.set_find(Some(Rc::new(Select)), cx);
        (
            v.find_step(false, cx),
            v.select_all_matches(10, cx),
            v.toggle_fold_at(0, cx),
        )
    });
    assert_eq!((step, all, folded), (None, 0, false));
    assert_eq!(
        vcx.read(|cx| e.read(cx).find_status()),
        FindStatus::default()
    );
    assert!(vcx.read(|cx| e.read(cx).folds().folded().is_empty()));
    // Every M3b command runs as a no-op: the text, the selection and the version stay.
    for id in [
        "editor.add_caret_below",
        "editor.select_all_occurrences",
        "editor.undo_selection",
        "editor.fold_all",
        "editor.find",
        "editor.toggle_comment",
        "editor.duplicate_line",
        "editor.delete_line",
        "editor.move_line_down",
        "editor.history_prev",
    ] {
        let ran = e.update_in(vcx, |v, window, cx| v.run_command(id, window, cx));
        assert!(ran, "{id}");
    }
    let after = vcx.read(|cx| e.read(cx).snapshot());
    assert_eq!(after.buffer.to_string(), before.buffer.to_string());
    assert_eq!(after.selection, before.selection);
    assert_eq!(after.version, before.version);
    // An escalated mark is kept and painted like a cancelling one until Task 34's badge.
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
