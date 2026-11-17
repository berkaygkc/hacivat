//! M3b Task 31: an IME composition with several carets (UI-EDIT-12 mirroring, D-072, R-M3-64).
//! With every caret in plain text the marked text is at every caret, the platform sees the main
//! caret's only, and one undo takes the whole composition back; with a caret inside a string or
//! a comment the others collapse first and the composition goes to the main caret, said once.
//! The tests call the `EntityInputHandler` methods on the entity, as the platform does, and
//! wait on conditions (`run_until_parked`).

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, EntityInputHandler as _, Focusable as _, IntoElement,
    ParentElement as _, Render, Styled as _, TestAppContext, VisualTestContext, Window, div, px,
};
use hacivat_text::{ChangeDesc, Snapshot, TokenOracle};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use hacivat_ui_editor::{
    DecorationSource, DocContext, DocId, EditorConfig, EditorView, ExecAction, ExecAt, ExecPreview,
    FailClosedService, InvalidateFn, LanguageService, StatementSpan, commands,
};
use smallvec::SmallVec;

/// A service whose strings are `'…'` (the text as last told).
#[derive(Default)]
struct Quoted(Rc<RefCell<String>>);

struct Quotes(Rc<RefCell<String>>);

impl TokenOracle for Quotes {
    fn words_on_line(&self, _: Range<usize>) -> Option<SmallVec<[Range<usize>; 16]>> {
        None
    }

    fn in_string_or_comment(&self, at: usize) -> Option<bool> {
        let text = self.0.borrow();
        Some(text.get(..at)?.matches('\'').count() % 2 == 1)
    }
}

impl LanguageService for Quoted {
    fn open(&self, _: DocId, snap: &Snapshot, _: DocContext, _: &mut gpui::App) {
        *self.0.borrow_mut() = snap.buffer.to_string();
    }
    fn set_context(&self, _: DocId, _: DocContext, _: &mut gpui::App) {}
    fn change(&self, _: DocId, snap: &Snapshot, _: &ChangeDesc, _: &mut gpui::App) {
        *self.0.borrow_mut() = snap.buffer.to_string();
    }
    fn close(&self, _: DocId) {}
    fn on_update(&self, _: DocId, _: InvalidateFn) {}
    fn statements(&self, _: DocId, _: u64, _: Range<usize>) -> Arc<[StatementSpan]> {
        Arc::from(Vec::new())
    }
    fn exec_preview(&self, d: DocId, v: u64, at: &ExecAt, a: ExecAction) -> Option<ExecPreview> {
        FailClosedService.exec_preview(d, v, at, a)
    }
    fn token_oracle(&self, _: DocId) -> Rc<dyn TokenOracle> {
        Rc::new(Quotes(self.0.clone()))
    }
    fn decoration_sources(&self, _: DocId) -> SmallVec<[Rc<dyn DecorationSource>; 2]> {
        SmallVec::new()
    }
}

struct Host {
    editor: Entity<EditorView>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(800.0)).h(px(600.0)).child(self.editor.clone())
    }
}

/// Opens `text` focused and drawn with a caret at `caret` and one added on each of the
/// `below` lines under it (the last one the main caret).
fn open<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
    caret: usize,
    below: usize,
) -> (Entity<EditorView>, &'a mut VisualTestContext) {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let text = text.to_string();
    let slot: Rc<RefCell<Option<Entity<EditorView>>>> = Rc::default();
    let out = slot.clone();
    let (_, vcx) = cx.add_window_view(move |window, cx| {
        let editor = cx.new(|cx| {
            EditorView::new(
                DocId(64),
                &text,
                EditorConfig::default(),
                Rc::new(Quoted::default()),
                Rc::new(|_: &gpui::App| "Console 1".into()),
                window,
                cx,
            )
        });
        *out.borrow_mut() = Some(editor.clone());
        Host { editor }
    });
    let editor = slot.borrow().clone().unwrap();
    vcx.update(|window, _| window.activate_window());
    editor.update_in(vcx, |v, window, cx| {
        let f = v.focus_handle(cx);
        window.focus(&f, cx);
        v.select_range(caret..caret, cx);
        for _ in 0..below {
            assert!(v.run_command("editor.add_caret_below", window, cx));
        }
    });
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    (editor, vcx)
}

fn text(e: &Entity<EditorView>, vcx: &VisualTestContext) -> String {
    vcx.read(|cx| e.read(cx).snapshot().buffer.to_string())
}

fn ranges(e: &Entity<EditorView>, vcx: &VisualTestContext) -> Vec<(usize, usize)> {
    vcx.read(|cx| {
        e.read(cx)
            .snapshot()
            .selection
            .ranges()
            .iter()
            .map(|r| (r.anchor, r.head))
            .collect()
    })
}

fn mark(e: &Entity<EditorView>, vcx: &mut VisualTestContext, s: &str, sel: Option<Range<usize>>) {
    e.update_in(vcx, |v, window, cx| {
        v.replace_and_mark_text_in_range(None, s, sel, window, cx);
    });
    vcx.run_until_parked();
}

fn commit(e: &Entity<EditorView>, vcx: &mut VisualTestContext, s: &str) {
    e.update_in(vcx, |v, window, cx| {
        v.replace_text_in_range(None, s, window, cx);
    });
    vcx.run_until_parked();
}

fn marked16(e: &Entity<EditorView>, vcx: &mut VisualTestContext) -> Option<Range<usize>> {
    e.update_in(vcx, |v, window, cx| v.marked_text_range(window, cx))
}

/// UI-EDIT-12 test 5: three carets, a Japanese composition. Each caret shows the same marked
/// text; the commit inserts at all three; one undo removes all three.
#[gpui::test]
fn ime_mirrors_to_every_caret_in_plain_text(cx: &mut TestAppContext) {
    let (e, vcx) = open(cx, "a\nb\nc\n", 1, 2);
    assert_eq!(ranges(&e, vcx), [(1, 1), (3, 3), (5, 5)]);
    mark(&e, vcx, "か", None);
    mark(&e, vcx, "かな", Some(0..1));
    assert_eq!(text(&e, vcx), "aかな\nbかな\ncかな\n");
    // The platform sees the main caret's composition only (UTF-16 offsets): the last line's.
    assert_eq!(marked16(&e, vcx), Some(9..11));
    let sel = e.update_in(vcx, |v, window, cx| {
        v.selected_text_range(false, window, cx)
    });
    assert_eq!(sel.unwrap().range, 9..10);
    assert_eq!(ranges(&e, vcx), [(1, 4), (9, 12), (17, 20)]);
    assert!(vcx.read(|cx| e.read(cx).status().composing));
    commit(&e, vcx, "仮名");
    assert_eq!(text(&e, vcx), "a仮名\nb仮名\nc仮名\n");
    assert_eq!(ranges(&e, vcx), [(7, 7), (15, 15), (23, 23)]);
    assert!(!vcx.read(|cx| e.read(cx).status().composing));
    e.update_in(vcx, |v, window, cx| {
        assert!(v.run_command("editor.undo", window, cx));
    });
    vcx.run_until_parked();
    assert_eq!(text(&e, vcx), "a\nb\nc\n");
    assert_eq!(ranges(&e, vcx), [(1, 1), (3, 3), (5, 5)]);
    // No sentence: nothing fell back.
    assert_eq!(vcx.read(|cx| commands::status_message(e.read(cx))), None);
}

/// R-M3-64: a caret inside a string. The other carets collapse first, the composition goes to
/// the main caret only, and the status bar says so once.
#[gpui::test]
fn ime_falls_back_to_the_primary_caret_inside_a_string(cx: &mut TestAppContext) {
    // The first caret is inside `'ab'` (byte 8), the main one below it in plain text.
    let (e, vcx) = open(cx, "SELECT 'ab';\nSELECT xy;", 8, 1);
    assert_eq!(ranges(&e, vcx), [(8, 8), (21, 21)]);
    mark(&e, vcx, "ş", None);
    assert_eq!(text(&e, vcx), "SELECT 'ab';\nSELECT xşy;");
    assert_eq!(ranges(&e, vcx), [(23, 23)]);
    assert_eq!(
        vcx.read(|cx| commands::status_message(e.read(cx)))
            .map(|s| s.to_string()),
        Some("hv-editor-ime-primary-only".to_string())
    );
    commit(&e, vcx, "ş");
    assert_eq!(text(&e, vcx), "SELECT 'ab';\nSELECT xşy;");
    assert_eq!(ranges(&e, vcx), [(23, 23)]);
    // With the main caret inside the string and the other in plain text, the same.
    let (e, vcx) = open(cx, "SELECT xy;\nSELECT 'ab';", 9, 1);
    assert_eq!(ranges(&e, vcx), [(9, 9), (20, 20)]);
    mark(&e, vcx, "ü", None);
    assert_eq!(text(&e, vcx), "SELECT xy;\nSELECT 'aüb';");
    assert_eq!(ranges(&e, vcx), [(22, 22)]);
}
