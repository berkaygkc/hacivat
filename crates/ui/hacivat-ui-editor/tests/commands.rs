//! M3a Task 25: the editor's commands (UI-EDIT-05 single cursor, -06 movement, -13 data). Every
//! command of the table has a test here ([`every_command_has_a_test`]): the table-driven
//! [`CASES`] (`┃` is the head, `«` the anchor of a selection) or a test of its own
//! ([`SPECIAL`]). Words come from a small test lexer behind the token-oracle seam, as Task 24's
//! adapter will give them. Tests wait on conditions (`run_until_parked`), never on time.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, EntityInputHandler as _, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, Keystroke, Modifiers, MouseButton, MouseDownEvent,
    MouseUpEvent, ParentElement as _, Pixels, Point, Render, SharedString, Styled as _,
    TestAppContext, VisualTestContext, Window, div, point, px,
};
use hacivat_text::{ChangeDesc, Snapshot, TextBuffer, TokenOracle, UserEvent};
use hacivat_ui::keymaps::{PRESETS, load};
use hacivat_ui::l10n::{UiArg, UiText, UiTextProvider};
use hacivat_ui_core::keymap::{
    ContextStack, KeyboardLayoutId, KeymapPlatform, Resolution, normalize,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use hacivat_ui_editor::commands::{self, ShortcutHints};
use hacivat_ui_editor::{
    DecorationSource, DocContext, DocId, EDITOR_CONTEXT, EditorConfig, EditorEvent, EditorView,
    ExecAction, ExecAt, ExecPreview, ExecRequest, InvalidateFn, LanguageService, PreviewState,
    StatementSpan,
};
use smallvec::{SmallVec, smallvec};

// ---------------------------------------------------------------------------------------------
// A language service with words (the token-oracle seam) and a preview the test sets.

/// What the test service knows: the text as last told, and the preview it answers.
#[derive(Default)]
struct Known {
    text: RefCell<String>,
    preview: RefCell<Option<ExecPreview>>,
}

struct Sql(Rc<Known>);

/// One token per quoted run (`'…'`, `"…"`, `` `…` ``), identifier or number, two- or
/// three-character operator (`::`, `->>`, `->`, `<=`, `>=`, `<>`, `!=`), or other character.
fn lex(text: &str, base: usize) -> SmallVec<[Range<usize>; 16]> {
    let mut out = SmallVec::new();
    let mut it = text.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        let start = i;
        let mut end = i + c.len_utf8();
        if c.is_whitespace() {
            continue;
        }
        if matches!(c, '\'' | '"' | '`') {
            for (j, d) in it.by_ref() {
                end = j + d.len_utf8();
                if d == c {
                    break;
                }
            }
        } else if c.is_alphanumeric() || c == '_' {
            while let Some(&(j, d)) = it.peek() {
                if !(d.is_alphanumeric() || d == '_' || d == '$') {
                    break;
                }
                end = j + d.len_utf8();
                it.next();
            }
        } else {
            let rest = &text[start..];
            let op = ["->>", "::", "->", "<=", ">=", "<>", "!="]
                .into_iter()
                .find(|op| rest.starts_with(op));
            if let Some(op) = op {
                end = start + op.len();
                while it.peek().is_some_and(|&(j, _)| j < end) {
                    it.next();
                }
            }
        }
        out.push(base + start..base + end);
    }
    out
}

struct Words(Rc<Known>);

impl TokenOracle for Words {
    fn words_on_line(&self, line: Range<usize>) -> Option<SmallVec<[Range<usize>; 16]>> {
        let text = self.0.text.borrow();
        let body = text.get(line.clone())?;
        Some(lex(body, line.start))
    }
}

impl LanguageService for Sql {
    fn open(&self, _: DocId, snap: &Snapshot, _: DocContext, _: &mut gpui::App) {
        *self.0.text.borrow_mut() = snap.buffer.to_string();
    }
    fn set_context(&self, _: DocId, _: DocContext, _: &mut gpui::App) {}
    fn change(&self, _: DocId, snap: &Snapshot, _: &ChangeDesc, _: &mut gpui::App) {
        *self.0.text.borrow_mut() = snap.buffer.to_string();
    }
    fn close(&self, _: DocId) {}
    fn on_update(&self, _: DocId, _: InvalidateFn) {}
    fn statements(&self, _: DocId, _: u64, _: Range<usize>) -> Arc<[StatementSpan]> {
        Arc::from(Vec::new())
    }
    fn exec_preview(&self, _: DocId, _: u64, _: &ExecAt, _: ExecAction) -> Option<ExecPreview> {
        self.0.preview.borrow().clone()
    }
    fn token_oracle(&self, _: DocId) -> Rc<dyn TokenOracle> {
        Rc::new(Words(self.0.clone()))
    }
    fn decoration_sources(&self, _: DocId) -> SmallVec<[Rc<dyn DecorationSource>; 2]> {
        SmallVec::new()
    }
}

// ---------------------------------------------------------------------------------------------
// The window: one editor in an 800 × 600 box, and one tab stop after it.

struct Host {
    editor: Entity<EditorView>,
    stop: FocusHandle,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .child(div().w(px(800.0)).h(px(600.0)).child(self.editor.clone()))
            .child(
                div()
                    .id("after")
                    .track_focus(&self.stop)
                    .w(px(10.0))
                    .h(px(10.0)),
            )
    }
}

/// The editor and the tab stop after it.
type Parts = (Entity<EditorView>, FocusHandle);

struct Opened<'a> {
    editor: Entity<EditorView>,
    stop: FocusHandle,
    known: Rc<Known>,
    events: Rc<RefCell<Vec<EditorEvent>>>,
    vcx: &'a mut VisualTestContext,
}

const HEAD: char = '┃';
const ANCHOR: char = '«';

/// The text, anchor and head of a marked state (`«` omitted: a caret).
fn parse(marked: &str) -> (String, usize, usize) {
    let (mut text, mut anchor, mut head) = (String::new(), None, None);
    for c in marked.chars() {
        match c {
            HEAD => head = Some(text.len()),
            ANCHOR => anchor = Some(text.len()),
            c => text.push(c),
        }
    }
    let head = head.expect("a state marks its head");
    (text, anchor.unwrap_or(head), head)
}

fn marked(text: &str, anchor: usize, head: usize) -> String {
    let mut out = String::new();
    for (i, c) in text.char_indices().chain([(text.len(), '\0')]) {
        if i == head {
            out.push(HEAD);
        }
        if i == anchor && anchor != head {
            out.push(ANCHOR);
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

/// Opens `state` (its selection forward) focused and drawn.
fn open_with<'a>(cx: &'a mut TestAppContext, state: &str, cfg: EditorConfig) -> Opened<'a> {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let (text, anchor, head) = parse(state);
    assert!(anchor <= head, "a test state's selection runs forward");
    let known = Rc::new(Known::default());
    let slot: Rc<RefCell<Option<Parts>>> = Rc::default();
    let (out, service) = (slot.clone(), known.clone());
    let (_, vcx) = cx.add_window_view(move |window, cx| {
        let editor = cx.new(|cx| {
            EditorView::new(
                DocId(25),
                &text,
                cfg,
                Rc::new(Sql(service)),
                Rc::new(|_: &gpui::App| "Console 1".into()),
                window,
                cx,
            )
        });
        let stop = cx.focus_handle().tab_stop(true);
        *out.borrow_mut() = Some((editor.clone(), stop.clone()));
        Host { editor, stop }
    });
    let (editor, stop) = slot.borrow().clone().unwrap();
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
        v.select_range(anchor..head, cx);
    });
    draw(vcx);
    events.borrow_mut().clear();
    Opened {
        editor,
        stop,
        known,
        events,
        vcx,
    }
}

fn open<'a>(cx: &'a mut TestAppContext, state: &str) -> Opened<'a> {
    open_with(cx, state, EditorConfig::default())
}

fn run(o: &mut Opened<'_>, id: &str) {
    let ran = o
        .editor
        .update_in(o.vcx, |v, window, cx| v.run_command(id, window, cx));
    assert!(ran, "{id} is a command");
    o.vcx.run_until_parked();
}

fn state(o: &Opened<'_>) -> String {
    o.vcx.read(|cx| {
        let s = o.editor.read(cx).snapshot();
        let m = s.selection.main();
        marked(&s.buffer.to_string(), m.anchor, m.head)
    })
}

/// `before` after the command `id`.
fn after(cx: &mut TestAppContext, before: &str, id: &str) -> String {
    let mut o = open(cx, before);
    run(&mut o, id);
    state(&o)
}

/// Types `text` as the platform does (the input handler's one text path).
fn type_text(o: &mut Opened<'_>, text: &str) {
    o.editor.update_in(o.vcx, |v, window, cx| {
        v.replace_text_in_range(None, text, window, cx);
    });
}

fn editor_focused(o: &mut Opened<'_>) -> bool {
    draw(o.vcx);
    o.vcx
        .update(|window, cx| o.editor.read(cx).focus_handle(cx).is_focused(window))
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

// ---------------------------------------------------------------------------------------------
// The table.

/// `(command, before, after)`.
const CASES: &[(&str, &str, &str)] = &[
    // Characters are grapheme clusters; a CRLF is one; a selection collapses to its edge.
    ("editor.move_left", "ab┃c", "a┃bc"),
    ("editor.move_left", "a«bc┃d", "a┃bcd"),
    ("editor.move_left", "┃abc", "┃abc"),
    ("editor.move_right", "a┃e\u{301}b", "ae\u{301}┃b"),
    ("editor.move_right", "a┃\r\nb", "a\r\n┃b"),
    ("editor.move_right", "«ab┃c", "ab┃c"),
    ("editor.select_left", "ab┃c", "a┃b«c"),
    ("editor.select_left", "a\r\n┃b", "a┃\r\n«b"),
    ("editor.select_right", "a┃bc", "a«b┃c"),
    ("editor.select_right", "┃👩‍💻x", "«👩‍💻┃x"),
    // Lines: the same column; past the first or last line, the text's edge.
    ("editor.move_up", "abc\nde┃f", "ab┃c\ndef"),
    ("editor.move_up", "ab┃c", "┃abc"),
    ("editor.move_down", "ab┃c\ndef", "abc\nde┃f"),
    ("editor.move_down", "ab┃c", "abc┃"),
    ("editor.select_up", "abc\nde┃f", "ab┃c\nde«f"),
    ("editor.select_down", "ab┃c\ndef", "ab«c\nde┃f"),
    // Words are tokens (UI-EDIT-06 case 1): `::` and a quoted identifier are one word each.
    (
        "editor.move_word_right",
        "┃SELECT a.b::int",
        "SELECT┃ a.b::int",
    ),
    (
        "editor.move_word_right",
        "SELECT┃ a.b::int",
        "SELECT a┃.b::int",
    ),
    (
        "editor.move_word_right",
        "SELECT a.b┃::int",
        "SELECT a.b::┃int",
    ),
    ("editor.move_word_right", "ab┃\ncd", "ab\n┃cd"),
    (
        "editor.move_word_left",
        "SELECT \"Order Lines\"┃",
        "SELECT ┃\"Order Lines\"",
    ),
    (
        "editor.move_word_left",
        "SELECT a.b::┃int",
        "SELECT a.b┃::int",
    ),
    ("editor.select_word_right", "┃SELECT 1", "«SELECT┃ 1"),
    ("editor.select_word_left", "SELECT 1┃", "SELECT ┃1«"),
    // Smart Home (UI-EDIT-06 case 5), End before the line break.
    ("editor.move_line_start", "    SELECT┃", "    ┃SELECT"),
    ("editor.move_line_start", "    ┃SELECT", "┃    SELECT"),
    ("editor.move_line_end", "a┃bc\ndef", "abc┃\ndef"),
    ("editor.move_line_end", "┃ab\r\ncd", "ab┃\r\ncd"),
    ("editor.select_line_start", "  ab┃", "  ┃ab«"),
    ("editor.select_line_end", "a┃bc\nd", "a«bc┃\nd"),
    ("editor.move_document_start", "ab\ncd┃", "┃ab\ncd"),
    ("editor.move_document_end", "┃ab\ncd", "ab\ncd┃"),
    ("editor.select_document_start", "ab\nc┃d", "┃ab\nc«d"),
    ("editor.select_document_end", "a┃b\ncd", "a«b\ncd┃"),
    // Enter: the dominant ending, the line's indentation kept, the selection replaced.
    ("editor.newline", "  a\r\n  b┃", "  a\r\n  b\r\n  ┃"),
    ("editor.newline", "a«bc┃d", "a\n┃d"),
    ("editor.newline", "\tx┃y", "\tx\n\t┃y"),
    ("editor.newline", "  ┃  x", "  \n  ┃  x"),
    // Tab: to the next stop; over several lines it indents them.
    ("editor.tab", "┃x", "    ┃x"),
    ("editor.tab", "ab┃", "ab  ┃"),
    ("editor.tab", "a«b┃c", "a   ┃c"),
    // A wide cluster is two columns of the grid (the display model's cells).
    ("editor.tab", "日┃", "日  ┃"),
    ("editor.tab", "日本┃", "日本    ┃"),
    ("editor.tab", "\t日┃", "\t日  ┃"),
    ("editor.tab", "«a\n\nb┃", "    «a\n\n    b┃"),
    ("editor.outdent", "      a┃", "  a┃"),
    ("editor.outdent", "\ta┃", "a┃"),
    ("editor.outdent", "«    a\n  b┃", "«a\nb┃"),
    ("editor.outdent", "a┃", "a┃"),
    // Deletes: a whole cluster (UI-EDIT-06 case 4), a whole CRLF, the selection.
    ("editor.delete_backward", "I\u{307}┃", "┃"),
    ("editor.delete_backward", "a\r\n┃b", "a┃b"),
    ("editor.delete_backward", "a«bc┃d", "a┃d"),
    ("editor.delete_backward", "┃a", "┃a"),
    ("editor.delete_forward", "┃\r\nb", "┃b"),
    ("editor.delete_forward", "┃👩‍💻x", "┃x"),
    ("editor.delete_forward", "a«bc┃d", "a┃d"),
    (
        "editor.delete_word_backward",
        "SELECT \"Müşteri Adı\"┃",
        "SELECT ┃",
    ),
    ("editor.delete_word_backward", "a.b::┃int", "a.b┃int"),
    ("editor.delete_word_forward", "┃SELECT 1", "┃ 1"),
    ("editor.delete_word_forward", "x ┃'a b' y", "x ┃ y"),
    ("editor.delete_to_line_start", "ab\n  cd┃e", "ab\n┃e"),
    ("editor.delete_to_line_start", "ab\n┃cd", "ab┃cd"),
    ("editor.select_all", "a┃b\nc", "«ab\nc┃"),
    ("editor.escape", "a«bc┃d", "abc┃d"),
];

/// Commands with a test of their own, and that test's name.
const SPECIAL: &[(&str, &str)] = &[
    ("editor.page_up", "page_moves_keep_the_goal_column"),
    ("editor.page_down", "page_moves_keep_the_goal_column"),
    ("editor.select_page_up", "page_moves_keep_the_goal_column"),
    ("editor.select_page_down", "page_moves_keep_the_goal_column"),
    ("editor.undo", "undo_and_redo_restore_text_and_selection"),
    ("editor.redo", "undo_and_redo_restore_text_and_selection"),
    ("editor.copy", "clipboard_commands_delegate_and_seal"),
    ("editor.cut", "clipboard_commands_delegate_and_seal"),
    ("editor.paste", "clipboard_commands_delegate_and_seal"),
    (
        "editor.toggle_tab_moves_focus",
        "ctrl_m_toggles_tab_moves_focus",
    ),
    (
        "editor.context_menu",
        "the_context_menu_opens_at_the_caret_and_runs_its_entries",
    ),
    (
        "editor.run",
        "run_emits_selections_or_carets_and_seals_history",
    ),
    (
        "editor.run_2",
        "run_emits_selections_or_carets_and_seals_history",
    ),
    (
        "editor.run_3",
        "run_emits_selections_or_carets_and_seals_history",
    ),
    (
        "editor.describe_exec_range",
        "describe_says_what_run_would_execute",
    ),
];

fn untested_commands() -> Vec<&'static str> {
    commands::commands()
        .iter()
        .map(|c| c.id)
        .filter(|id| {
            !CASES.iter().any(|(c, _, _)| c == id) && !SPECIAL.iter().any(|(c, _)| c == id)
        })
        .collect()
}

#[gpui::test]
fn every_command_has_a_test(cx: &mut TestAppContext) {
    assert_eq!(untested_commands(), Vec::<&str>::new());
    // Every test `SPECIAL` names is a test of this file, and names the command it runs.
    let source = include_str!("commands.rs");
    for (id, test) in SPECIAL {
        let from = source
            .find(&format!("\nfn {test}("))
            .unwrap_or_else(|| panic!("{id}: no test named {test}"));
        let body = &source[from..];
        let body = &body[..body[1..].find("\n}\n").map_or(body.len(), |end| end + 1)];
        assert!(
            body.contains(&format!("\"{id}\"")),
            "{test} does not run {id}"
        );
    }
    // Nothing is listed that is not a command, and every case holds.
    for id in CASES
        .iter()
        .map(|(c, _, _)| c)
        .chain(SPECIAL.iter().map(|(c, _)| c))
    {
        assert!(commands::command_by_id(id).is_some(), "{id}");
    }
    for (id, before, want) in CASES {
        assert_eq!(after(cx, before, id), *want, "{id} on {before:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// Movement.

/// UI-EDIT-05 case 4: the goal column survives a short line; a horizontal move resets it.
#[gpui::test]
fn vertical_moves_keep_the_goal_column(cx: &mut TestAppContext) {
    let mut o = open(cx, "abcdefgh\nab\nabcdefgh\nabcdefgh┃");
    o.editor.update(o.vcx, |v, cx| v.select_range(6..6, cx));
    run(&mut o, "editor.move_down");
    assert_eq!(state(&o), "abcdefgh\nab┃\nabcdefgh\nabcdefgh");
    run(&mut o, "editor.move_down");
    assert_eq!(state(&o), "abcdefgh\nab\nabcdef┃gh\nabcdefgh");
    run(&mut o, "editor.move_up");
    run(&mut o, "editor.move_left");
    run(&mut o, "editor.move_down");
    assert_eq!(state(&o), "abcdefgh\nab\na┃bcdefgh\nabcdefgh");
    // The same after an edit, before the next frame is drawn (no layout of this text yet).
    run(&mut o, "editor.move_line_end");
    type_text(&mut o, "ij");
    run(&mut o, "editor.move_up");
    assert_eq!(state(&o), "abcdefgh\nab┃\nabcdefghij\nabcdefgh");
    run(&mut o, "editor.move_down");
    run(&mut o, "editor.move_down");
    assert_eq!(state(&o), "abcdefgh\nab\nabcdefghij\nabcdefgh┃");
}

#[gpui::test]
fn page_moves_keep_the_goal_column(cx: &mut TestAppContext) {
    let text: String = (0..200)
        .map(|i| match i {
            0 | 60 | 61 | 62 => "0123456789\n".to_string(),
            _ => format!("{}\n", i % 10),
        })
        .collect();
    let mut o = open(cx, &format!("{HEAD}{text}"));
    let rows = o.vcx.read(|cx| {
        let v = o.editor.read(cx);
        let r = v.last_layout().expect("a frame was drawn").rows();
        (f32::from(r.bounds.size.height) / f32::from(r.line_height)).floor() as usize
    });
    assert!(rows > 10, "{rows} rows are visible");
    let page = rows - 1;
    let line_col = |o: &Opened<'_>| {
        o.vcx.read(|cx| {
            let v = o.editor.read(cx);
            let s = v.status();
            (s.line as usize - 1, s.column as usize - 1)
        })
    };
    o.editor.update(o.vcx, |v, cx| v.select_range(7..7, cx));
    // A page down: the viewport's height less one row; the column clamps on a short line.
    run(&mut o, "editor.page_down");
    assert_eq!(line_col(&o), (page, 1));
    draw(o.vcx);
    let first = o
        .vcx
        .read(|cx| o.editor.read(cx).last_layout().unwrap().rows().first_line);
    // The first row may be the one before, cut by the 6 px top padding.
    assert!(
        (first as usize).abs_diff(page) <= 1,
        "the view moved by the same rows: {first}"
    );
    // Back up: the goal column is still 7.
    run(&mut o, "editor.page_up");
    assert_eq!(line_col(&o), (0, 7));
    // Up from the first page: the start of the text.
    run(&mut o, "editor.page_up");
    assert_eq!(line_col(&o), (0, 0));
    // Selecting pages extends from the anchor.
    o.editor.update(o.vcx, |v, cx| v.select_range(3..3, cx));
    run(&mut o, "editor.select_page_down");
    let sel = o
        .vcx
        .read(|cx| *o.editor.read(cx).snapshot().selection.main());
    assert_eq!(sel.anchor, 3);
    assert_eq!(line_col(&o).0, page);
    run(&mut o, "editor.select_page_up");
    let sel = o
        .vcx
        .read(|cx| *o.editor.read(cx).snapshot().selection.main());
    assert_eq!((sel.anchor, sel.head), (3, 3));
    // Past the end: the end of the text.
    for _ in 0..(200 / page + 1) {
        run(&mut o, "editor.page_down");
    }
    assert_eq!(line_col(&o).0, 200);
}

// ---------------------------------------------------------------------------------------------
// Editing.

#[gpui::test]
fn enter_inserts_the_dominant_ending_and_keeps_indent(cx: &mut TestAppContext) {
    assert_eq!(
        after(cx, "  a\r\n  b┃", "editor.newline"),
        "  a\r\n  b\r\n  ┃"
    );
    // LF when the text has no ending yet.
    assert_eq!(after(cx, "    x┃", "editor.newline"), "    x\n    ┃");
}

#[gpui::test]
fn tab_inserts_four_spaces_or_a_tab(cx: &mut TestAppContext) {
    assert_eq!(after(cx, "┃", "editor.tab"), "    ┃");
    let tabs = EditorConfig {
        insert_spaces: false,
        ..EditorConfig::default()
    };
    let mut o = open_with(cx, "a┃", tabs);
    run(&mut o, "editor.tab");
    assert_eq!(state(&o), "a\t┃");
    // The tab size is the configuration's.
    let two = EditorConfig {
        tab_size: 2,
        ..EditorConfig::default()
    };
    let mut o = open_with(cx, "┃x", two);
    run(&mut o, "editor.tab");
    assert_eq!(state(&o), "  ┃x");
    run(&mut o, "editor.outdent");
    assert_eq!(state(&o), "┃x");
}

#[gpui::test]
fn delete_word_uses_token_words(cx: &mut TestAppContext) {
    assert_eq!(
        after(
            cx,
            r#"SELECT "Müşteri Adı"┃"#,
            "editor.delete_word_backward"
        ),
        "SELECT ┃"
    );
}

#[gpui::test]
fn undo_and_redo_restore_text_and_selection(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT ┃1");
    run(&mut o, "editor.newline");
    run(&mut o, "editor.delete_word_forward");
    assert_eq!(state(&o), "SELECT \n┃");
    // The Edit menu's labels: the kind of the group each would apply.
    let kinds = |o: &Opened<'_>| {
        o.vcx.read(|cx| {
            let v = o.editor.read(cx);
            (commands::undo_kind(v), commands::redo_kind(v))
        })
    };
    assert_eq!(kinds(&o), (Some(UserEvent::DeleteForward), None));
    run(&mut o, "editor.undo");
    assert_eq!(
        kinds(&o),
        (Some(UserEvent::InputType), Some(UserEvent::DeleteForward))
    );
    assert_eq!(state(&o), "SELECT \n┃1");
    run(&mut o, "editor.undo");
    assert_eq!(state(&o), "SELECT ┃1");
    // Nothing left to undo: nothing happens.
    run(&mut o, "editor.undo");
    assert_eq!(state(&o), "SELECT ┃1");
    run(&mut o, "editor.redo");
    assert_eq!(state(&o), "SELECT \n┃1");
    run(&mut o, "editor.redo");
    assert_eq!(state(&o), "SELECT \n┃");
    run(&mut o, "editor.redo");
    assert_eq!(state(&o), "SELECT \n┃");
    o.vcx.read(|cx| {
        let v = o.editor.read(cx);
        assert!(commands::can_undo(v));
        assert!(!commands::can_redo(v));
    });
    // The host hears of an undo like of any edit.
    o.events.borrow_mut().clear();
    run(&mut o, "editor.undo");
    assert!(
        o.events
            .borrow()
            .iter()
            .any(|e| matches!(e, EditorEvent::Changed { .. })),
        "{:?}",
        o.events.borrow()
    );
    assert!(o.vcx.read(|cx| commands::can_redo(o.editor.read(cx))));
}

/// The view's `read_only` is no model filter: every editing command refuses by itself, undo
/// and redo included; moving, selecting and Run still work.
#[gpui::test]
fn a_read_only_editor_refuses_every_edit(cx: &mut TestAppContext) {
    let mut o = open(cx, "  ab ┃cd");
    // Two undo groups (a caret move away and back seals the first), the second undone: there
    // is something to undo and something to redo.
    type_text(&mut o, "x");
    run(&mut o, "editor.move_left");
    run(&mut o, "editor.move_right");
    type_text(&mut o, "y");
    run(&mut o, "editor.undo");
    assert_eq!(state(&o), "  ab x┃cd");
    o.vcx.read(|cx| {
        let v = o.editor.read(cx);
        assert!(commands::can_undo(v) && commands::can_redo(v));
        assert_eq!(commands::undo_kind(v), Some(UserEvent::InputType));
        assert_eq!(commands::redo_kind(v), Some(UserEvent::InputType));
    });
    o.editor.update(o.vcx, |v, cx| {
        let cfg = EditorConfig {
            read_only: true,
            ..*v.config()
        };
        v.set_config(cfg, cx);
    });
    for id in [
        "editor.undo",
        "editor.redo",
        "editor.newline",
        "editor.tab",
        "editor.outdent",
        "editor.delete_backward",
        "editor.delete_forward",
        "editor.delete_word_backward",
        "editor.delete_word_forward",
        "editor.delete_to_line_start",
        "editor.cut",
        "editor.paste",
    ] {
        run(&mut o, id);
        assert_eq!(state(&o), "  ab x┃cd", "{id}");
    }
    o.vcx.read(|cx| {
        let v = o.editor.read(cx);
        assert!(!commands::can_undo(v) && !commands::can_redo(v));
        assert_eq!(
            (commands::undo_kind(v), commands::redo_kind(v)),
            (None, None)
        );
    });
    run(&mut o, "editor.select_word_right");
    assert_eq!(state(&o), "  ab x«cd┃");
    run(&mut o, "editor.run");
    assert_eq!(requests(&o).len(), 1);
    // Editable again: the redo is still there.
    o.editor.update(o.vcx, |v, cx| {
        let cfg = EditorConfig {
            read_only: false,
            ..*v.config()
        };
        v.set_config(cfg, cx);
    });
    run(&mut o, "editor.redo");
    assert_eq!(state(&o), "  ab xy┃cd");
    run(&mut o, "editor.undo");
    run(&mut o, "editor.undo");
    assert_eq!(state(&o), "  ab ┃cd");
}

/// Copy, cut and paste are Task 27's bodies (`clipboard::*`, no-ops until then): the commands
/// delegate, and cut and paste close the open undo group.
#[gpui::test]
fn clipboard_commands_delegate_and_seal(cx: &mut TestAppContext) {
    let seals = |id: &str| commands::command_by_id(id).unwrap().seals_history;
    assert!(!seals("editor.copy") && seals("editor.cut") && seals("editor.paste"));
    let mut o = open(cx, "a┃b");
    type_text(&mut o, "x");
    run(&mut o, "editor.copy");
    type_text(&mut o, "y");
    run(&mut o, "editor.cut");
    type_text(&mut o, "z");
    run(&mut o, "editor.paste");
    type_text(&mut o, "w");
    assert_eq!(state(&o), "axyzw┃b");
    // Copy sealed nothing (`xy` is one group); cut and paste did.
    run(&mut o, "editor.undo");
    assert_eq!(state(&o), "axyz┃b");
    run(&mut o, "editor.undo");
    assert_eq!(state(&o), "axy┃b");
    run(&mut o, "editor.undo");
    assert_eq!(state(&o), "a┃b");
}

// ---------------------------------------------------------------------------------------------
// Escape, Tab and the focus.

#[gpui::test]
fn escape_collapses_then_lets_tab_leave(cx: &mut TestAppContext) {
    let mut o = open(cx, "a«bc┃d");
    run(&mut o, "editor.escape");
    assert_eq!(state(&o), "abc┃d");
    assert!(
        !o.vcx
            .read(|cx| commands::escape_released(o.editor.read(cx)))
    );
    // Tab still indents.
    run(&mut o, "editor.tab");
    assert_eq!(state(&o), "abc ┃d");
    assert!(editor_focused(&mut o));
    // A second Escape has nothing to collapse: it is not consumed, and the next Tab leaves.
    run(&mut o, "editor.escape");
    assert_eq!(state(&o), "abc ┃d");
    assert!(
        o.vcx
            .read(|cx| commands::escape_released(o.editor.read(cx)))
    );
    run(&mut o, "editor.tab");
    assert_eq!(state(&o), "abc ┃d", "Tab inserted nothing");
    assert!(!editor_focused(&mut o), "Tab left the editor");
    let stop = o.stop.clone();
    assert!(o.vcx.update(|window, _| stop.is_focused(window)));
    // Back in the editor, Tab indents again: the release was for one Tab.
    o.editor.update_in(o.vcx, |v, window, cx| {
        let f = v.focus_handle(cx);
        window.focus(&f, cx);
    });
    draw(o.vcx);
    run(&mut o, "editor.tab");
    assert_eq!(state(&o), "abc     ┃d");
    // Anything between the Escape and the Tab takes the release back.
    run(&mut o, "editor.escape");
    run(&mut o, "editor.move_left");
    assert!(
        !o.vcx
            .read(|cx| commands::escape_released(o.editor.read(cx)))
    );
    run(&mut o, "editor.escape");
    type_text(&mut o, "x");
    run(&mut o, "editor.tab");
    assert!(editor_focused(&mut o));
}

#[gpui::test]
fn ctrl_m_toggles_tab_moves_focus(cx: &mut TestAppContext) {
    let mut o = open(cx, "a┃");
    let mode = |o: &Opened<'_>| {
        o.vcx
            .read(|cx| o.editor.read(cx).command_state().tab_moves_focus)
    };
    assert!(!mode(&o));
    run(&mut o, "editor.toggle_tab_moves_focus");
    assert!(mode(&o));
    // The mode is said (the status bar, assistive technology).
    assert!(
        o.events
            .borrow()
            .iter()
            .any(|e| matches!(e, EditorEvent::StatusChanged))
    );
    assert_eq!(
        o.vcx
            .read(|cx| commands::status_message(o.editor.read(cx)))
            .as_deref(),
        Some("hv-editor-tab-moves-focus-on")
    );
    run(&mut o, "editor.tab");
    assert_eq!(state(&o), "a┃");
    assert!(!editor_focused(&mut o), "Tab moved the focus");
    // Shift+Tab comes back (the editor is the only other stop), still without outdenting.
    o.editor.update_in(o.vcx, |v, window, cx| {
        let f = v.focus_handle(cx);
        window.focus(&f, cx);
    });
    draw(o.vcx);
    run(&mut o, "editor.outdent");
    assert!(!editor_focused(&mut o), "Shift+Tab moved the focus");
    o.editor.update_in(o.vcx, |v, window, cx| {
        let f = v.focus_handle(cx);
        window.focus(&f, cx);
    });
    draw(o.vcx);
    // The mode stays until it is toggled off.
    assert!(mode(&o));
    run(&mut o, "editor.toggle_tab_moves_focus");
    assert!(!mode(&o));
    run(&mut o, "editor.tab");
    assert_eq!(state(&o), "a   ┃");
    assert!(editor_focused(&mut o));
}

/// UI-EDIT-13 case 1 on the bound `ctrl-m` (M3a binds no `ctrl-i`, P25): the key printed `M`
/// resolves to the command on Turkish Q and Turkish F, on every platform and preset, and the
/// command it names toggles the mode.
#[gpui::test]
fn ctrl_m_fires_from_its_key_on_tr_q_and_tr_f(cx: &mut TestAppContext) {
    let stroke = Keystroke {
        modifiers: Modifiers {
            control: true,
            ..Modifiers::none()
        },
        key: "m".into(),
        key_char: None,
    };
    let press = normalize(&stroke).expect("ctrl-m is no dead key");
    let ctx = ContextStack(vec!["Workspace", EDITOR_CONTEXT]);
    let mut o = open(cx, "┃");
    let mut expected = false;
    for platform in [
        KeymapPlatform::MacOs,
        KeymapPlatform::Windows,
        KeymapPlatform::Linux,
    ] {
        for preset in PRESETS {
            let (km, diags) = load(platform, preset, None);
            assert!(diags.is_empty(), "{diags:?}");
            for layout in ["tr-q", "tr-f", "us"] {
                let layout = KeyboardLayoutId(layout.into());
                let Resolution::Command(id) = km.resolve(&ctx, &[], &press, &layout, false) else {
                    panic!("ctrl-m is unbound on {platform:?} {preset} {layout:?}");
                };
                assert_eq!(id, "editor.toggle_tab_moves_focus");
                // No binding fires during a composition (F15).
                assert_eq!(
                    km.resolve(&ctx, &[], &press, &layout, true),
                    Resolution::PassThrough
                );
                run(&mut o, &id);
                expected = !expected;
                let on = o
                    .vcx
                    .read(|cx| o.editor.read(cx).command_state().tab_moves_focus);
                assert_eq!(on, expected);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Run and describe.

fn run_with_selection(cx: &mut TestAppContext, r: Range<usize>) -> ExecRequest {
    let mut o = open(cx, "SELECT 1;\nSELECT 2;┃");
    o.editor.update(o.vcx, |v, cx| v.select_range(r, cx));
    run(&mut o, "editor.run");
    let mut all = requests(&o);
    assert_eq!(all.len(), 1);
    all.remove(0)
}

#[gpui::test]
fn run_emits_selections_or_carets_and_seals_history(cx: &mut TestAppContext) {
    let e = run_with_selection(cx, 0..8);
    assert!(matches!(e.at, ExecAt::Selections(_)));
    assert_eq!(e.at, ExecAt::Selections(smallvec![0..8]));
    assert_eq!((e.doc, e.action), (DocId(25), ExecAction::Execute1));
    assert_eq!(e.snapshot.buffer.to_string(), "SELECT 1;\nSELECT 2;");
    let e = run_with_selection(cx, 12..12);
    assert_eq!(e.at, ExecAt::Carets(smallvec![12]));
    // Run 2 and Run 3 ask for their own action, on the snapshot they were asked on.
    let mut o = open(cx, "SELECT 1┃");
    run(&mut o, "editor.run");
    type_text(&mut o, "0");
    run(&mut o, "editor.run_2");
    type_text(&mut o, "0");
    run(&mut o, "editor.run_3");
    let all = requests(&o);
    assert_eq!(
        all.iter().map(|r| r.action).collect::<Vec<_>>(),
        [
            ExecAction::Execute1,
            ExecAction::Execute2,
            ExecAction::Execute3
        ]
    );
    assert_eq!(all[0].snapshot.buffer.to_string(), "SELECT 1");
    assert_eq!(all[1].snapshot.buffer.to_string(), "SELECT 10");
    assert_eq!(all[2].snapshot.buffer.to_string(), "SELECT 100");
    assert_eq!(all[2].snapshot.version, all[1].snapshot.version + 1);
    // Run sealed the typing: the two zeros are two undo groups.
    run(&mut o, "editor.undo");
    assert_eq!(state(&o), "SELECT 10┃");
    // Without a Run between them, typing is one group.
    let mut o = open(cx, "SELECT 1┃");
    type_text(&mut o, "0");
    type_text(&mut o, "0");
    run(&mut o, "editor.undo");
    assert_eq!(state(&o), "SELECT 1┃");
}

/// A provider that renders a message as its id and arguments.
struct Ids;

impl UiTextProvider for Ids {
    fn text(&self, id: &str, args: &[(&'static str, UiArg)]) -> SharedString {
        let mut out = id.to_string();
        for (name, arg) in args {
            match arg {
                UiArg::Count(n) => out.push_str(&format!(" {name}={n}")),
                UiArg::Text(t) => out.push_str(&format!(" {name}={t}")),
            }
        }
        out.into()
    }
    fn attr(&self, _: &str, _: &str) -> Option<SharedString> {
        None
    }
    fn generation(&self) -> u64 {
        1
    }
}

#[gpui::test]
fn describe_says_what_run_would_execute(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_global(UiText(Arc::new(Ids))));
    let long = format!("SELECT {};", "c, ".repeat(60));
    let text = format!("-- a\nSELECT 1;\n{long}\nSELECT\n  3;\n");
    let second = text.find(&long).unwrap();
    let third = text.find("SELECT\n  3").unwrap();
    let mut o = open(cx, &format!("{text}{HEAD}"));
    let said = |o: &mut Opened<'_>, preview: Option<ExecPreview>| {
        *o.known.preview.borrow_mut() = preview;
        o.events.borrow_mut().clear();
        run(o, "editor.describe_exec_range");
        assert!(
            o.events
                .borrow()
                .iter()
                .any(|e| matches!(e, EditorEvent::StatusChanged)),
            "the status bar hears of it"
        );
        o.vcx
            .read(|cx| commands::status_message(o.editor.read(cx)))
            .expect("a sentence")
            .to_string()
    };
    // Byte ranges as `(start, end)`.
    let units = |ranges: &[(usize, usize)], state| {
        Some(ExecPreview {
            ranges: ranges.iter().map(|&(a, b)| a..b).collect(),
            state,
        })
    };
    // One statement on one line, with its text.
    assert_eq!(
        said(&mut o, units(&[(5, 14)], PreviewState::Units)),
        "hv-editor-describe-one-line line=2 label=SELECT 1;"
    );
    // One over several lines: its first line.
    assert_eq!(
        said(&mut o, units(&[(third, third + 12)], PreviewState::Units)),
        "hv-editor-describe-one from=4 to=5 label=SELECT"
    );
    // A long first line is cut at 60 characters.
    let cut = said(
        &mut o,
        units(&[(second, second + long.len())], PreviewState::Units),
    );
    let label = cut
        .strip_prefix("hv-editor-describe-one-line line=3 label=")
        .unwrap();
    assert_eq!(label.chars().count(), 61, "{label}");
    assert!(label.ends_with('…') && label.starts_with("SELECT c, c, "));
    // Several statements.
    assert_eq!(
        said(
            &mut o,
            units(&[(5, 14), (third, third + 12)], PreviewState::Units)
        ),
        "hv-editor-describe-many n=2 from=2 to=5"
    );
    assert_eq!(
        said(&mut o, units(&[(5, 9), (10, 14)], PreviewState::Units)),
        "hv-editor-describe-many-line n=2 line=2"
    );
    // The verdicts that run nothing say so; so does a missing analysis (never a guess, R-M3-8).
    for (state, id) in [
        (PreviewState::Nothing, "hv-editor-describe-nothing"),
        (PreviewState::Uncertain, "hv-editor-describe-uncertain"),
        (PreviewState::NoConnection, "hv-editor-describe-offline"),
        (PreviewState::AskPreselected, "hv-editor-describe-ask"),
    ] {
        assert_eq!(said(&mut o, units(&[(5, 14)], state)), id);
    }
    assert_eq!(
        said(&mut o, units(&[], PreviewState::Units)),
        "hv-editor-describe-nothing"
    );
    assert_eq!(said(&mut o, None), "hv-editor-describe-pending");
    // The sentence goes when the caret moves on.
    run(&mut o, "editor.move_left");
    assert_eq!(
        o.vcx.read(|cx| commands::status_message(o.editor.read(cx))),
        None
    );
}

// ---------------------------------------------------------------------------------------------
// The context menu.

fn menu_open(o: &Opened<'_>) -> bool {
    o.vcx
        .read(|cx| o.editor.read(cx).command_state().menu_open())
}

fn opened_at(o: &Opened<'_>) -> Vec<Point<Pixels>> {
    o.events
        .borrow()
        .iter()
        .filter_map(|e| match e {
            EditorEvent::ContextMenuOpened { at } => Some(*at),
            _ => None,
        })
        .collect()
}

#[gpui::test]
fn the_context_menu_opens_at_the_caret_and_runs_its_entries(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(ShortcutHints(Rc::new(|_: &gpui::App, id: &str| {
            (id == "editor.run").then(|| "⌘↵".into())
        })));
    });
    let mut o = open(cx, "SELECT 1;\nSELECT ┃2;");
    assert!(!menu_open(&o));
    // Shift+F10: under the caret.
    run(&mut o, "editor.context_menu");
    draw(o.vcx);
    assert!(menu_open(&o));
    let caret = o
        .vcx
        .read(|cx| o.editor.read(cx).caret_bounds())
        .expect("the caret is painted");
    assert_eq!(opened_at(&o), [point(caret.left(), caret.bottom())]);
    assert!(!editor_focused(&mut o), "the menu has the focus");
    // Its entries, in order: the clipboard, then Run.
    let entries = o
        .vcx
        .read(|cx| commands::context_menu_entries(o.editor.read(cx), cx));
    let ids: Vec<&str> = entries.iter().map(|e| e.action.as_ref()).collect();
    assert_eq!(
        ids,
        [
            "editor.cut",
            "editor.copy",
            "editor.paste",
            "editor.select_all",
            "",
            "editor.run",
            "editor.describe_exec_range",
        ]
    );
    assert!(entries[4].separator);
    assert!(entries.iter().all(|e| e.separator || !e.disabled));
    let hint = |i: usize| {
        o.vcx
            .read(|cx| entries[i].keys.as_ref().map(|k| k(cx).to_string()))
    };
    assert_eq!(hint(5).as_deref(), Some("⌘↵"), "the host's shortcut");
    assert_eq!(hint(0), None);
    // ↓ ↓ ↓ ↵: Select all runs, the menu closes, the editor has the focus again.
    o.vcx.simulate_keystrokes("down down down enter");
    assert!(!menu_open(&o));
    assert_eq!(state(&o), "«SELECT 1;\nSELECT 2;┃");
    assert!(editor_focused(&mut o));
    // Escape closes it without running anything.
    run(&mut o, "editor.context_menu");
    draw(o.vcx);
    assert!(menu_open(&o));
    o.vcx.simulate_keystrokes("escape");
    assert!(!menu_open(&o));
    assert!(editor_focused(&mut o));
    assert_eq!(state(&o), "«SELECT 1;\nSELECT 2;┃");
    // Run from the menu asks for an execution.
    run(&mut o, "editor.context_menu");
    draw(o.vcx);
    o.vcx.simulate_keystrokes("end up enter");
    assert_eq!(requests(&o).len(), 1);
    assert!(!menu_open(&o));
}

#[gpui::test]
fn a_right_click_opens_the_menu_at_the_pointer(cx: &mut TestAppContext) {
    let mut o = open(cx, "«SELECT 1;┃\nSELECT 2;");
    let p = o.vcx.read(|cx| {
        let l = o.editor.read(cx).last_layout().unwrap();
        let b = l.bounds_for_range(2..2).unwrap();
        point(b.origin.x + px(1.0), b.origin.y + px(9.0))
    });
    o.vcx.simulate_event(MouseDownEvent {
        button: MouseButton::Right,
        position: p,
        modifiers: Modifiers::none(),
        click_count: 1,
        first_mouse: false,
    });
    o.vcx.simulate_event(MouseUpEvent {
        button: MouseButton::Right,
        position: p,
        modifiers: Modifiers::none(),
        click_count: 1,
    });
    draw(o.vcx);
    assert!(menu_open(&o));
    // At the pointer, far from the caret (the click fell inside the selection, which stays).
    assert_eq!(opened_at(&o), [p]);
    assert_eq!(state(&o), "«SELECT 1;┃\nSELECT 2;");
    // A second right click, outside the selection: the caret moves there and the menu opens
    // again at that pointer (the first menu's dismissal does not close the second).
    let p2 = o.vcx.read(|cx| {
        let l = o.editor.read(cx).last_layout().unwrap();
        // The start of the second line: left of the open menu, so outside it.
        let b = l.bounds_for_range(10..10).unwrap();
        point(b.origin.x + px(1.0), b.origin.y + px(9.0))
    });
    // The pointer moves there first, as a real one does (the open menu sees the press as
    // outside it and dismisses itself in the capture phase, before the editor's handler).
    o.vcx.simulate_event(gpui::MouseMoveEvent {
        position: p2,
        modifiers: Modifiers::none(),
        pressed_button: None,
    });
    o.vcx.simulate_event(MouseDownEvent {
        button: MouseButton::Right,
        position: p2,
        modifiers: Modifiers::none(),
        click_count: 1,
        first_mouse: false,
    });
    draw(o.vcx);
    assert!(menu_open(&o));
    assert_eq!(opened_at(&o), [p, p2]);
    assert_eq!(state(&o), "SELECT 1;\n┃SELECT 2;");
    // A click on an entry runs it; the click never reaches the text under the menu.
    let row = o
        .vcx
        .debug_bounds("menu-item-3")
        .expect("Select all is drawn");
    o.vcx.simulate_click(row.center(), Modifiers::none());
    draw(o.vcx);
    assert!(!menu_open(&o));
    assert_eq!(state(&o), "«SELECT 1;\nSELECT 2;┃");
    run(&mut o, "editor.context_menu");
    draw(o.vcx);
    assert!(menu_open(&o));
    // A press outside the menu dismisses it.
    let away = point(px(700.0), px(500.0));
    o.vcx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: away,
        modifiers: Modifiers::none(),
        click_count: 1,
        first_mouse: false,
    });
    o.vcx.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position: away,
        modifiers: Modifiers::none(),
        click_count: 1,
    });
    draw(o.vcx);
    assert!(!menu_open(&o));
    // A read-only editor offers no Cut and no Paste.
    o.editor.update(o.vcx, |v, cx| {
        let cfg = EditorConfig {
            read_only: true,
            ..*v.config()
        };
        v.set_config(cfg, cx);
    });
    let entries = o
        .vcx
        .read(|cx| commands::context_menu_entries(o.editor.read(cx), cx));
    let disabled: Vec<&str> = entries
        .iter()
        .filter(|e| e.disabled && !e.separator)
        .map(|e| e.action.as_ref())
        .collect();
    assert_eq!(disabled, ["editor.cut", "editor.paste"]);
}

// ---------------------------------------------------------------------------------------------
// The pointer: UI-EDIT-06 step 9.

fn double_click(o: &mut Opened<'_>, byte: usize) {
    let p = o.vcx.read(|cx| {
        let l = o.editor.read(cx).last_layout().unwrap();
        let b = l.bounds_for_range(byte..byte).unwrap();
        point(b.origin.x + px(1.0), b.origin.y + px(9.0))
    });
    for clicks in [1, 2] {
        o.vcx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: p,
            modifiers: Modifiers::none(),
            click_count: clicks,
            first_mouse: false,
        });
        o.vcx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: p,
            modifiers: Modifiers::none(),
            click_count: clicks,
        });
    }
    draw(o.vcx);
}

/// A double click inside a string selects its content first, a second one the string with its
/// quotes; a quoted identifier is selected with its quotes at once (UI-EDIT-06 case 2).
#[gpui::test]
fn a_double_click_in_a_string_selects_its_content_first(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT 'a b', \"Order Lines\", E'x', '' ┃");
    double_click(&mut o, 9);
    assert_eq!(state(&o), "SELECT '«a b┃', \"Order Lines\", E'x', '' ");
    double_click(&mut o, 9);
    assert_eq!(state(&o), "SELECT «'a b'┃, \"Order Lines\", E'x', '' ");
    // Again: the content (the two alternate).
    double_click(&mut o, 9);
    assert_eq!(state(&o), "SELECT '«a b┃', \"Order Lines\", E'x', '' ");
    // On the quote itself: the whole string.
    double_click(&mut o, 7);
    assert_eq!(state(&o), "SELECT «'a b'┃, \"Order Lines\", E'x', '' ");
    double_click(&mut o, 17);
    assert_eq!(state(&o), "SELECT 'a b', «\"Order Lines\"┃, E'x', '' ");
    // An empty string has no content to prefer.
    double_click(&mut o, 35);
    assert_eq!(state(&o), "SELECT 'a b', \"Order Lines\", E'x', «''┃ ");
}

/// Without tokens (reduced mode, no analysis yet) a double click selects the class word.
#[test]
fn a_double_click_without_tokens_selects_the_word() {
    let buf = TextBuffer::from_str("SELECT 'a b'");
    assert_eq!(
        commands::double_click_range(&buf, 9, &hacivat_ui_editor::NoTokens, None),
        8..9
    );
    // With tokens, and with the content already selected.
    let known = Rc::new(Known::default());
    *known.text.borrow_mut() = buf.to_string();
    let words = Words(known);
    assert_eq!(commands::double_click_range(&buf, 9, &words, None), 8..11);
    assert_eq!(
        commands::double_click_range(&buf, 9, &words, Some(8..11)),
        7..12
    );
}

/// The open menu takes every pointer event over it: a right press on a row neither moves the
/// caret under it nor reopens the menu, and the wheel does not scroll the text beneath.
#[gpui::test]
fn the_open_menu_swallows_right_presses_and_the_wheel(cx: &mut TestAppContext) {
    let text: String = (0..200).map(|i| format!("SELECT {i};\n")).collect();
    let mut o = open(cx, &format!("«SELECT 0;{HEAD}{}", &text[9..]));
    let p = o.vcx.read(|cx| {
        let l = o.editor.read(cx).last_layout().unwrap();
        let b = l.bounds_for_range(2..2).unwrap();
        point(b.origin.x + px(1.0), b.origin.y + px(9.0))
    });
    let right_press = |o: &mut Opened<'_>, at: Point<Pixels>| {
        o.vcx.simulate_event(gpui::MouseMoveEvent {
            position: at,
            modifiers: Modifiers::none(),
            pressed_button: None,
        });
        o.vcx.simulate_event(MouseDownEvent {
            button: MouseButton::Right,
            position: at,
            modifiers: Modifiers::none(),
            click_count: 1,
            first_mouse: false,
        });
        o.vcx.simulate_event(MouseUpEvent {
            button: MouseButton::Right,
            position: at,
            modifiers: Modifiers::none(),
            click_count: 1,
        });
        draw(o.vcx);
    };
    right_press(&mut o, p);
    assert!(menu_open(&o));
    let before = state(&o);
    // A right press on Copy, which lies over text outside the selection.
    let row = o.vcx.debug_bounds("menu-item-1").expect("Copy is drawn");
    right_press(&mut o, row.center());
    assert!(menu_open(&o));
    assert_eq!(opened_at(&o), [p]);
    assert_eq!(state(&o), before);
    // The wheel over the menu leaves the text where it is; beside the menu it scrolls.
    let first_line = |o: &Opened<'_>| {
        o.vcx
            .read(|cx| o.editor.read(cx).last_layout().unwrap().rows().first_line)
    };
    let wheel = |o: &mut Opened<'_>, at: Point<Pixels>| {
        o.vcx.simulate_event(gpui::ScrollEvent {
            position: at,
            delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(-180.0))),
            ..Default::default()
        });
        draw(o.vcx);
    };
    wheel(&mut o, row.center());
    assert_eq!(first_line(&o), 0);
    assert!(menu_open(&o));
    wheel(&mut o, point(px(700.0), px(300.0)));
    assert!(first_line(&o) > 0);
}
