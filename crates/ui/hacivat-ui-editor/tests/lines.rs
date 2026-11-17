//! M3c Task 34: the line commands (NOTES surface 17): comment or uncomment with the dialect's
//! line comment (a block comment for a part of one line), duplicate, delete and move lines,
//! each one undo group at every caret; a moved line carries a fold with it; mixed line endings
//! are reported and normalised to the dominant one; history recall at the last line of an
//! empty or recalled editor; a drag that rests past the text's edge keeps scrolling.
//!
//! `┃` is a head, `«` the anchor of the selection whose head follows; several ranges may be
//! marked, in order.

#![allow(
    clippy::single_range_in_vec_init,
    clippy::cloned_ref_to_slice_refs,
    reason = "byte ranges as data"
)]

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, Focusable as _, IntoElement, Modifiers, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, Render, Styled as _,
    TestAppContext, VisualTestContext, Window, div, point, px,
};
use hacivat_text::{
    ChangeDesc, FoldKind, FoldRange, LineEnding, SelectionRange, SelectionSet, Snapshot,
    TokenOracle,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use hacivat_ui_editor::{
    CommentTokens, DecorationSource, DocContext, DocId, EditorConfig, EditorEvent, EditorView,
    ExecAction, ExecAt, ExecPreview, InvalidateFn, LanguageService, NoTokens, StatementSpan,
};
use smallvec::SmallVec;

/// A service with the dialect's comment tokens and the foldable ranges it is given.
#[derive(Default)]
struct Svc {
    line_comment: Option<&'static str>,
    folds: Vec<FoldRange>,
}

impl LanguageService for Svc {
    fn open(&self, _: DocId, _: &Snapshot, _: DocContext, _: &mut gpui::App) {}
    fn set_context(&self, _: DocId, _: DocContext, _: &mut gpui::App) {}
    fn change(&self, _: DocId, _: &Snapshot, _: &ChangeDesc, _: &mut gpui::App) {}
    fn close(&self, _: DocId) {}
    fn on_update(&self, _: DocId, _: InvalidateFn) {}
    fn statements(&self, _: DocId, _: u64, _: Range<usize>) -> Arc<[StatementSpan]> {
        Arc::from(Vec::new())
    }
    fn exec_preview(&self, _: DocId, _: u64, _: &ExecAt, _: ExecAction) -> Option<ExecPreview> {
        None
    }
    fn token_oracle(&self, _: DocId) -> Rc<dyn TokenOracle> {
        Rc::new(NoTokens)
    }
    fn decoration_sources(&self, _: DocId) -> SmallVec<[Rc<dyn DecorationSource>; 2]> {
        SmallVec::new()
    }
    fn folding_ranges(&self, _: DocId, _: u64) -> Arc<[FoldRange]> {
        self.folds.clone().into()
    }
    fn comment_tokens(&self, _: DocId) -> CommentTokens {
        match self.line_comment {
            Some(line) => CommentTokens {
                line,
                ..CommentTokens::SQL
            },
            None => CommentTokens::SQL,
        }
    }
}

struct Host {
    editor: Entity<EditorView>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(800.0)).h(px(400.0)).child(self.editor.clone())
    }
}

struct Opened<'a> {
    editor: Entity<EditorView>,
    events: Rc<RefCell<Vec<EditorEvent>>>,
    vcx: &'a mut VisualTestContext,
}

const HEAD: char = '┃';
const ANCHOR: char = '«';

/// The text and the ranges of a marked state.
fn parse(marked: &str) -> (String, Vec<SelectionRange>) {
    let (mut text, mut ranges, mut anchor) = (String::new(), Vec::new(), None);
    for c in marked.chars() {
        match c {
            ANCHOR => anchor = Some(text.len()),
            HEAD => {
                let head = text.len();
                ranges.push(SelectionRange::new(anchor.take().unwrap_or(head), head));
            }
            c => text.push(c),
        }
    }
    assert!(!ranges.is_empty(), "a state marks a head");
    (text, ranges)
}

/// The marked state of `text` with `sel`.
fn marked(text: &str, sel: &SelectionSet) -> String {
    let mut out = String::new();
    for (i, c) in text.char_indices().chain([(text.len(), '\0')]) {
        for r in sel.ranges() {
            if r.anchor == i && !r.is_empty() {
                out.push(ANCHOR);
            }
            if r.head == i {
                out.push(HEAD);
            }
        }
        if i < text.len() {
            out.push(c);
        }
    }
    out
}

fn draw(vcx: &mut VisualTestContext) {
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
}

fn open_with<'a>(cx: &'a mut TestAppContext, state: &str, svc: Svc) -> Opened<'a> {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let (text, ranges) = parse(state);
    let slot: Rc<RefCell<Option<Entity<EditorView>>>> = Rc::default();
    let out = slot.clone();
    let svc = Rc::new(svc);
    let (_, vcx) = cx.add_window_view(move |window, cx| {
        let editor = cx.new(|cx| {
            EditorView::new(
                DocId(34),
                &text,
                EditorConfig::default(),
                svc,
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
    let sink = events.clone();
    vcx.update(|window, cx| {
        cx.subscribe(&editor, move |_, e: &EditorEvent, _| {
            sink.borrow_mut().push(e.clone());
        })
        .detach();
        window.activate_window();
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    let n = ranges.len();
    editor.update(vcx, |v, cx| {
        v.select_ranges(SelectionSet::normalized(ranges, n - 1), cx);
    });
    draw(vcx);
    Opened {
        editor,
        events,
        vcx,
    }
}

fn open<'a>(cx: &'a mut TestAppContext, state: &str) -> Opened<'a> {
    open_with(cx, state, Svc::default())
}

impl Opened<'_> {
    fn run(&mut self, id: &str) {
        let e = self.editor.clone();
        e.update_in(self.vcx, |v, window, cx| {
            assert!(v.run_command(id, window, cx), "{id} is a command");
        });
        draw(self.vcx);
    }

    fn state(&self) -> String {
        self.vcx.read(|cx| {
            let snap = self.editor.read(cx).snapshot();
            marked(&snap.buffer.to_string(), &snap.selection)
        })
    }

    fn text(&self) -> String {
        self.vcx
            .read(|cx| self.editor.read(cx).snapshot().buffer.to_string())
    }

    fn undo(&mut self) {
        self.run("editor.undo");
    }
}

/// `id` on `before` gives `want`, and one undo restores `before`.
fn case(cx: &mut TestAppContext, id: &str, before: &str, want: &str) {
    let mut o = open(cx, before);
    o.run(id);
    assert_eq!(o.state(), want, "{id} on {before:?}");
    o.undo();
    assert_eq!(
        o.text(),
        parse(before).0,
        "one undo takes back {id} on {before:?}"
    );
}

#[gpui::test]
fn toggle_comment_uses_the_dialect_line_comment(cx: &mut TestAppContext) {
    // A caret: its line, at its indentation; again: back.
    case(
        cx,
        "editor.toggle_comment",
        "SELECT 1;\n  SEL┃ECT 2;\n",
        "SELECT 1;\n  -- SEL┃ECT 2;\n",
    );
    case(
        cx,
        "editor.toggle_comment",
        "SELECT 1;\n  -- SEL┃ECT 2;\n",
        "SELECT 1;\n  SEL┃ECT 2;\n",
    );
    // Several lines: each line with text, at the least indentation; a blank line stays blank.
    case(
        cx,
        "editor.toggle_comment",
        "«SELECT a,\n\n    b\nFROM t;┃\n",
        "«-- SELECT a,\n\n--     b\n-- FROM t;┃\n",
    );
    // Mixed: one commented line and one not comment both; all commented uncomment.
    case(
        cx,
        "editor.toggle_comment",
        "«-- a\nb┃\n",
        "«-- -- a\n-- b┃\n",
    );
    case(cx, "editor.toggle_comment", "«-- a\n--b┃\n", "«a\nb┃\n");
    // A part of one line: a block comment around it, and back.
    case(
        cx,
        "editor.toggle_comment",
        "SELECT «a┃, b;\n",
        "SELECT /* «a┃ */, b;\n",
    );
    case(
        cx,
        "editor.toggle_comment",
        "SELECT «/* a */┃, b;\n",
        "SELECT «a┃, b;\n",
    );
    // Several carets on different lines: every line, one undo group.
    case(
        cx,
        "editor.toggle_comment",
        "a┃\nb\nc┃\n",
        "-- a┃\nb\n-- c┃\n",
    );
    // The dialect's own token.
    let mut o = open_with(
        cx,
        "SELECT┃ 1;\n",
        Svc {
            line_comment: Some("#"),
            ..Svc::default()
        },
    );
    o.run("editor.toggle_comment");
    assert_eq!(o.state(), "# SELECT┃ 1;\n");
}

#[gpui::test]
fn duplicate_and_delete_lines(cx: &mut TestAppContext) {
    // Duplicate: the copy below, the caret on it at the same column.
    case(cx, "editor.duplicate_line", "a\nbc┃d\ne", "a\nbcd\nbc┃d\ne");
    // The last line without a break, and a selection over two lines (kept, on the copy).
    case(cx, "editor.duplicate_line", "x\ny┃", "x\ny\ny┃");
    case(cx, "editor.duplicate_line", "«a\nb┃\nc", "a\nb\n«a\nb┃\nc");
    // Two carets on one line duplicate it once; carets on two lines each.
    case(
        cx,
        "editor.duplicate_line",
        "a┃b┃\nc┃\n",
        "ab\na┃b┃\nc\nc┃\n",
    );
    // Delete: the line with its break, the caret at its column on the next one.
    case(cx, "editor.delete_line", "ab\ncd┃e\nfg\n", "ab\nfg┃\n");
    case(cx, "editor.delete_line", "ab\ncde\nf┃g", "ab\nc┃de");
    case(cx, "editor.delete_line", "on┃ly", "┃");
    case(cx, "editor.delete_line", "a┃\nb\nc┃\nd", "b┃\nd┃");
}

#[gpui::test]
fn move_lines_up_and_down(cx: &mut TestAppContext) {
    case(cx, "editor.move_line_up", "a\nb┃c\nd", "b┃c\na\nd");
    case(cx, "editor.move_line_down", "a\nb┃c\nd", "a\nd\nb┃c");
    // The first line up, the last line down: nothing moves (no undo group).
    let mut o = open(cx, "a┃\nb");
    o.run("editor.move_line_up");
    assert_eq!(o.state(), "a┃\nb");
    o.run("editor.move_line_down");
    o.run("editor.move_line_down");
    assert_eq!(o.state(), "b\na┃");
    // A selection over two lines moves them both, kept.
    case(cx, "editor.move_line_down", "«a\nb┃\nc\n", "c\n«a\nb┃\n");
    // Carets on neighbouring lines move as one block.
    case(cx, "editor.move_line_up", "x\na┃\nb┃\n", "a┃\nb┃\nx\n");
    // CRLF text keeps its breaks.
    case(cx, "editor.move_line_down", "a┃\r\nb\r\n", "b\r\na┃\r\n");
}

/// A folded range: lines 1–4 (0-based) of [`FOLD_TEXT`].
const FOLD_TEXT: &str = "top\nSELECT a,\n  b,\n  c\nFROM t;\nend\n";

fn fold_svc() -> Svc {
    let start = FOLD_TEXT.find("SELECT").unwrap();
    let end = FOLD_TEXT.find("t;").unwrap() + 2;
    Svc {
        folds: vec![FoldRange {
            range: start..end,
            kind: FoldKind::Statement,
        }],
        ..Svc::default()
    }
}

fn fold_all(o: &mut Opened<'_>) {
    o.run("editor.fold_all");
    o.vcx.read(|cx| {
        assert_eq!(o.editor.read(cx).folds().folded().len(), 1, "folded");
    });
}

fn folded_lines(o: &Opened<'_>) -> Vec<(u32, u32)> {
    o.vcx.read(|cx| o.editor.read(cx).folds().lines().to_vec())
}

#[gpui::test]
fn move_line_moves_a_fold_with_it(cx: &mut TestAppContext) {
    // The line above the fold moves down: it steps over the whole fold, which stays folded.
    {
        let state = FOLD_TEXT.replacen("top", "top┃", 1);
        let mut o = open_with(cx, &state, fold_svc());
        fold_all(&mut o);
        o.run("editor.move_line_down");
        assert_eq!(
            o.text(),
            "SELECT a,\n  b,\n  c\nFROM t;\ntop\nend\n",
            "the line went below the fold"
        );
        assert_eq!(folded_lines(&o), vec![(0, 3)], "the fold is still folded");
        assert_eq!(
            o.state().find('┃'),
            Some("SELECT a,\n  b,\n  c\nFROM t;\ntop".len())
        );
        // One undo puts both back, the fold folded where it was.
        o.undo();
        assert_eq!(o.text(), FOLD_TEXT);
    }
    // The fold's header moves up: its lines go with it, folded.
    let state = FOLD_TEXT.replacen("SELECT", "┃SELECT", 1);
    let mut o = open_with(cx, &state, fold_svc());
    fold_all(&mut o);
    o.run("editor.move_line_up");
    assert_eq!(o.text(), "SELECT a,\n  b,\n  c\nFROM t;\ntop\nend\n");
    assert_eq!(
        folded_lines(&o),
        vec![(0, 3)],
        "the fold moved with its header"
    );
    assert_eq!(
        o.state().find('┃'),
        Some(0),
        "the caret stays on the header"
    );
}

#[gpui::test]
fn delete_line_on_a_fold_header_unfolds_it(cx: &mut TestAppContext) {
    let state = FOLD_TEXT.replacen("SELECT", "┃SELECT", 1);
    let mut o = open_with(cx, &state, fold_svc());
    fold_all(&mut o);
    o.run("editor.delete_line");
    assert_eq!(o.text(), "top\n  b,\n  c\nFROM t;\nend\n");
    assert!(folded_lines(&o).is_empty(), "the fold opened");
}

#[gpui::test]
fn adding_a_caret_into_a_fold_opens_it(cx: &mut TestAppContext) {
    let state = FOLD_TEXT.replacen("end", "e┃nd", 1);
    let mut o = open_with(cx, &state, fold_svc());
    fold_all(&mut o);
    o.run("editor.add_caret_above");
    assert!(folded_lines(&o).is_empty(), "the caret opened the fold");
    assert_eq!(o.state().matches('┃').count(), 2);
}

#[gpui::test]
fn line_commands_refuse_a_read_only_editor(cx: &mut TestAppContext) {
    let mut o = open(cx, "a\nb┃\nc");
    o.editor.update(o.vcx, |v, cx| {
        v.set_config(
            EditorConfig {
                read_only: true,
                ..EditorConfig::default()
            },
            cx,
        );
    });
    for id in [
        "editor.toggle_comment",
        "editor.duplicate_line",
        "editor.delete_line",
        "editor.move_line_up",
        "editor.move_line_down",
        "editor.normalize_line_endings",
    ] {
        o.run(id);
        assert_eq!(o.state(), "a\nb┃\nc", "{id}");
    }
}

#[gpui::test]
fn normalize_line_endings_to_the_dominant_one(cx: &mut TestAppContext) {
    let mut o = open(cx, "a\r\nb\r\nc\nd┃\r\n");
    let stats = |o: &Opened<'_>| o.vcx.read(|cx| o.editor.read(cx).line_endings());
    assert_eq!((stats(&o).lf, stats(&o).crlf), (1, 3));
    assert!(
        o.vcx
            .read(|cx| o.editor.read(cx).status().mixed_line_endings)
    );
    o.run("editor.normalize_line_endings");
    assert_eq!(o.text(), "a\r\nb\r\nc\r\nd\r\n");
    assert!(
        !o.vcx
            .read(|cx| o.editor.read(cx).status().mixed_line_endings)
    );
    assert!(
        o.events
            .borrow()
            .iter()
            .any(|e| matches!(e, EditorEvent::LineEndingsChanged { mixed: false })),
        "the host hears the endings are uniform"
    );
    // One undo group: mixed again, and the host hears it.
    o.undo();
    assert_eq!(o.text(), "a\r\nb\r\nc\nd\r\n");
    assert!(
        o.events
            .borrow()
            .iter()
            .any(|e| matches!(e, EditorEvent::LineEndingsChanged { mixed: true }))
    );
    // To a chosen ending (the status bar menu's second row).
    o.editor.update(o.vcx, |v, cx| {
        v.normalize_line_endings_to(LineEnding::Lf, cx);
    });
    assert_eq!(o.text(), "a\nb\nc\nd\n");
    // Uniform text: the command changes nothing.
    o.run("editor.normalize_line_endings");
    assert_eq!(o.text(), "a\nb\nc\nd\n");
}

#[gpui::test]
fn history_recall_at_the_last_line(cx: &mut TestAppContext) {
    // An empty editor: ⌃⌥↑ asks the host for an older statement.
    let mut o = open(cx, "┃");
    o.run("editor.history_prev");
    assert!(matches!(
        o.events.borrow().last(),
        Some(EditorEvent::HistoryRecall { older: true })
    ));
    // The host puts it in: one transaction, the caret at its end, said in the status bar.
    o.editor.update(o.vcx, |v, cx| {
        v.show_recalled(
            "SELECT 1\nFROM t;",
            "From history: statement 1 of 2".into(),
            cx,
        );
    });
    draw(o.vcx);
    assert_eq!(o.state(), "SELECT 1\nFROM t;┃");
    let said = o
        .vcx
        .read(|cx| hacivat_ui_editor::commands::status_message(o.editor.read(cx)));
    assert_eq!(said.as_deref(), Some("From history: statement 1 of 2"));
    // A recalled text at its last line asks again, older or newer.
    o.events.borrow_mut().clear();
    o.run("editor.history_next");
    assert!(matches!(
        o.events.borrow().last(),
        Some(EditorEvent::HistoryRecall { older: false })
    ));
    // Not on another line.
    o.events.borrow_mut().clear();
    o.editor.update(o.vcx, |v, cx| v.select_range(2..2, cx));
    o.run("editor.history_prev");
    assert!(
        o.events
            .borrow()
            .iter()
            .all(|e| !matches!(e, EditorEvent::HistoryRecall { .. }))
    );
    // Not once the recalled text was edited.
    let len = o.text().len();
    o.editor.update(o.vcx, |v, cx| v.select_range(len..len, cx));
    o.editor.update_in(o.vcx, |v, window, cx| {
        use gpui::EntityInputHandler as _;
        v.replace_text_in_range(None, " -- x", window, cx);
    });
    o.events.borrow_mut().clear();
    o.run("editor.history_prev");
    assert!(
        o.events
            .borrow()
            .iter()
            .all(|e| !matches!(e, EditorEvent::HistoryRecall { .. }))
    );
    // One undo takes back the edit, a second the recall.
    o.undo();
    o.undo();
    assert_eq!(o.text(), "");
}

#[gpui::test]
fn drag_past_the_edge_autoscrolls(cx: &mut TestAppContext) {
    let text: String = (0..300).map(|i| format!("line {i}\n")).collect();
    let o = open(cx, &format!("┃{text}"));
    let (top, bottom, left) = o.vcx.read(|cx| {
        let rows = *o.editor.read(cx).last_layout().unwrap().rows();
        (rows.bounds.top(), rows.bounds.bottom(), rows.bounds.left())
    });
    let at = point(left + px(20.0), top + px(5.0));
    o.vcx.simulate_event(MouseDownEvent {
        position: at,
        button: MouseButton::Left,
        modifiers: Modifiers::none(),
        click_count: 1,
        first_mouse: false,
    });
    let below = point(left + px(20.0), bottom + px(30.0));
    o.vcx.simulate_event(MouseMoveEvent {
        position: below,
        pressed_button: Some(MouseButton::Left),
        modifiers: Modifiers::none(),
    });
    draw(o.vcx);
    let line = |o: &Opened<'_>| {
        o.vcx
            .read(|cx| o.editor.read(cx).last_layout().unwrap().rows().first_line)
    };
    let after_move = line(&o);
    // The pointer rests past the edge: the view keeps scrolling, frame by frame.
    for _ in 0..10 {
        o.vcx
            .executor()
            .advance_clock(hacivat_ui_editor::DRAG_AUTOSCROLL_TICK);
        draw(o.vcx);
    }
    let rested = line(&o);
    assert!(
        rested > after_move,
        "still scrolling: {after_move} → {rested}"
    );
    // The selection followed.
    let head = o
        .vcx
        .read(|cx| o.editor.read(cx).snapshot().selection.main().head);
    assert!(head > 0);
    // The button goes up: it stops.
    o.vcx.simulate_event(MouseUpEvent {
        position: below,
        button: MouseButton::Left,
        modifiers: Modifiers::none(),
        click_count: 1,
    });
    draw(o.vcx);
    let stopped = line(&o);
    for _ in 0..5 {
        o.vcx
            .executor()
            .advance_clock(hacivat_ui_editor::DRAG_AUTOSCROLL_TICK);
        draw(o.vcx);
    }
    assert_eq!(line(&o), stopped, "no scroll after the button went up");
}
