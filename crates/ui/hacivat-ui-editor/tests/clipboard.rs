//! M3a Task 27: the editor's clipboard (UI-EDIT-14 M3a scope). Copy and cut, line-wise without a
//! selection; a paste read asynchronously and applied to the selection current when it lands,
//! as one `InputPaste` group: a leading BOM stripped, line endings converted to the document's
//! dominant one, a NUL kept visibly with a warning, a paste that would take the document past
//! 20 MB refused (R-M3-49). Tests wait on conditions (`run_until_parked`), never on time.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AppContext as _, ClipboardEntry, ClipboardItem, Context, Entity, EntityInputHandler as _,
    ExternalPaths, Focusable as _, IntoElement, ParentElement as _, Render, Styled as _,
    TestAppContext, VisualTestContext, Window, div, px,
};
use hacivat_text::display::{Fragment, compose_line};
use hacivat_ui::toast::ToastQueue;
use hacivat_ui_core::theme::{Theme, ThemeMode};
use hacivat_ui_editor::{DocId, EditorConfig, EditorEvent, EditorView, FailClosedService};

const HEAD: char = '┃';
const ANCHOR: char = '«';

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

fn draw(vcx: &mut VisualTestContext) {
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
}

fn open_with<'a>(cx: &'a mut TestAppContext, state: &str, cfg: EditorConfig) -> Opened<'a> {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let (text, anchor, head) = parse(state);
    let slot: Rc<RefCell<Option<Entity<EditorView>>>> = Rc::default();
    let out = slot.clone();
    let (_, vcx) = cx.add_window_view(move |window, cx| {
        let editor = cx.new(|cx| {
            EditorView::new(
                DocId(14),
                &text,
                cfg,
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
        assert!(anchor <= head, "test states select forward");
        v.select_range(anchor..head, cx);
    });
    draw(vcx);
    events.borrow_mut().clear();
    Opened {
        editor,
        events,
        vcx,
    }
}

fn open<'a>(cx: &'a mut TestAppContext, state: &str) -> Opened<'a> {
    open_with(cx, state, EditorConfig::default())
}

fn state(o: &Opened<'_>) -> String {
    o.vcx.read(|cx| {
        let s = o.editor.read(cx).snapshot();
        let m = s.selection.main();
        let text = s.buffer.to_string();
        let mut out = String::new();
        for (i, c) in text.char_indices().chain([(text.len(), '\0')]) {
            if i == m.head {
                out.push(HEAD);
            }
            if i == m.anchor && m.anchor != m.head {
                out.push(ANCHOR);
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

fn put(o: &mut Opened<'_>, s: &str) {
    o.vcx
        .write_to_clipboard(ClipboardItem::new_string(s.into()));
}

fn clipboard_text(o: &Opened<'_>) -> Option<String> {
    o.vcx.read_from_clipboard().and_then(|i| i.text())
}

fn toasts(o: &Opened<'_>) -> usize {
    o.vcx.read(|cx| ToastQueue::visible_ids(cx).len())
}

fn refused(o: &Opened<'_>) -> Vec<u64> {
    o.events
        .borrow()
        .iter()
        .filter_map(|e| match e {
            EditorEvent::PasteRefused { bytes } => Some(*bytes),
            _ => None,
        })
        .collect()
}

fn move_to(o: &mut Opened<'_>, at: usize) {
    o.editor.update(o.vcx, |v, cx| v.select_range(at..at, cx));
}

// ---------------------------------------------------------------------------------------------

/// UI-EDIT-14 cases 1, 4, 5 and 6.
#[gpui::test]
fn ui_edit_14_cases_1_4_5_6(cx: &mut TestAppContext) {
    // 1. No selection: Copy takes line 2 with its break; pasted on line 5 it goes above that
    //    line and the caret keeps its column.
    let mut o = open(cx, "l1\nl┃2\nl3\nl4\nl5");
    run(&mut o, "editor.copy");
    assert_eq!(clipboard_text(&o).as_deref(), Some("l2\n"));
    move_to(&mut o, "l1\nl2\nl3\nl4\nl".len());
    run(&mut o, "editor.paste");
    assert_eq!(state(&o), "l1\nl2\nl3\nl4\nl2\nl┃5");
    // 4. A leading BOM is stripped.
    o.editor.update(o.vcx, |v, cx| v.set_text("", cx));
    put(&mut o, "\u{FEFF}SELECT 1");
    run(&mut o, "editor.paste");
    assert_eq!(state(&o), "SELECT 1┃");
    // 5. CRLF pasted into an LF document becomes LF (a CR alone too); U+2028 is not a break.
    o.editor.update(o.vcx, |v, cx| v.set_text("x\ny", cx));
    move_to(&mut o, 1);
    put(&mut o, "a\r\nb\rc\u{2028}d");
    run(&mut o, "editor.paste");
    assert_eq!(text(&o), "xa\nb\nc\u{2028}d\ny");
    // 6. A paste that would take the document past 20 MB is refused; the buffer is unchanged.
    let before = text(&o);
    let big = "a".repeat(21 << 20);
    put(&mut o, &big);
    run(&mut o, "editor.paste");
    assert_eq!(text(&o), before);
    assert_eq!(refused(&o), [21 << 20]);
    assert_eq!(toasts(&o), 1, "the refusal is shown");
}

/// Pasted text keeps a CRLF document CRLF, and a paste is one undo group of its own.
#[gpui::test]
fn paste_follows_a_crlf_document_and_is_one_group(cx: &mut TestAppContext) {
    let mut o = open(cx, "a\r\nb\r\n┃");
    put(&mut o, "x\ny\r\nz");
    o.editor.update_in(o.vcx, |v, window, cx| {
        v.replace_text_in_range(None, "q", window, cx);
    });
    run(&mut o, "editor.paste");
    assert_eq!(text(&o), "a\r\nb\r\nqx\r\ny\r\nz");
    run(&mut o, "editor.undo");
    assert_eq!(text(&o), "a\r\nb\r\nq");
}

/// The read is asynchronous: the paste lands on the selection current when it lands, not the
/// one at the key press.
#[gpui::test]
fn paste_applies_to_the_selection_current_when_it_lands(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT ┃ FROM t");
    put(&mut o, "id");
    o.editor.update_in(o.vcx, |v, window, cx| {
        v.run_command("editor.paste", window, cx);
        // Before the read lands, the selection moves to `t`.
        v.select_range(13..14, cx);
    });
    assert_eq!(
        text(&o),
        "SELECT  FROM t",
        "nothing yet: the read is in flight"
    );
    o.vcx.run_until_parked();
    assert_eq!(state(&o), "SELECT  FROM id┃");
}

/// A NUL is kept and drawn as a visible control chip, and a warning toast says so.
#[gpui::test]
fn nul_paste_warns_and_shows_the_control_glyph(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT ┃");
    put(&mut o, "'a\0b\0'");
    run(&mut o, "editor.paste");
    assert_eq!(text(&o), "SELECT 'a\0b\0'");
    assert_eq!(toasts(&o), 1);
    draw(o.vcx);
    let (controls, chip) = o.vcx.read(|cx| {
        let v = o.editor.read(cx);
        let buf = v.snapshot().buffer;
        let line = compose_line(&buf, 0, 4, &[]);
        let controls = line
            .fragments
            .iter()
            .filter(|f| matches!(f, Fragment::Control { .. }))
            .count();
        let l = v.last_layout().unwrap();
        let nul = "SELECT 'a".len();
        let chip = l.x_for_offset(nul + 1).unwrap() - l.x_for_offset(nul).unwrap();
        (controls, chip)
    });
    assert_eq!(controls, 2);
    let advance = o.vcx.read(|cx| {
        let l = o.editor.read(cx).last_layout().unwrap().clone();
        l.x_for_offset(1).unwrap() - l.x_for_offset(0).unwrap()
    });
    assert!(
        chip > advance,
        "the chip is wider than one character: {chip:?} vs {advance:?}"
    );
    // No NUL, no toast.
    put(&mut o, "x");
    run(&mut o, "editor.paste");
    assert_eq!(toasts(&o), 1);
}

/// R-M3-49: the limit is the document after the paste (the selection it replaces counted out),
/// not the paste alone.
#[gpui::test]
fn paste_over_20_mb_is_refused(cx: &mut TestAppContext) {
    let doc = "b".repeat(19 << 20);
    let state = format!("«{doc}┃");
    let mut o = open(cx, &state);
    // Replacing the whole 19 MB with 2 MB fits.
    let two = "c".repeat(2 << 20);
    put(&mut o, &two);
    run(&mut o, "editor.paste");
    assert_eq!(
        o.vcx
            .read(|cx| o.editor.read(cx).snapshot().buffer.len_bytes()),
        2 << 20
    );
    assert!(refused(&o).is_empty());
    // 2 MB + 19 MB more does not.
    let nineteen = "d".repeat(19 << 20);
    put(&mut o, &nineteen);
    run(&mut o, "editor.paste");
    assert_eq!(
        o.vcx
            .read(|cx| o.editor.read(cx).snapshot().buffer.len_bytes()),
        2 << 20
    );
    assert_eq!(refused(&o), [19 << 20]);
}

/// Copy and cut of a selection; cut is one undo group; a line-wise cut takes the whole line,
/// and the last line takes the break before it.
#[gpui::test]
fn copy_and_cut_selection_and_lines(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT «a┃, b\nFROM t");
    run(&mut o, "editor.copy");
    assert_eq!(clipboard_text(&o).as_deref(), Some("a"));
    run(&mut o, "editor.cut");
    assert_eq!(state(&o), "SELECT ┃, b\nFROM t");
    run(&mut o, "editor.undo");
    assert_eq!(text(&o), "SELECT a, b\nFROM t");
    // A selection's text pastes inline even at a line's start (it was not line-wise).
    move_to(&mut o, "SELECT a, b\n".len());
    run(&mut o, "editor.paste");
    assert_eq!(state(&o), "SELECT a, b\na┃FROM t");
    // A line-wise cut of the last line removes it with the break before it.
    run(&mut o, "editor.cut");
    assert_eq!(text(&o), "SELECT a, b");
    assert_eq!(clipboard_text(&o).as_deref(), Some("aFROM t\n"));
    run(&mut o, "editor.undo");
    assert_eq!(text(&o), "SELECT a, b\naFROM t");
    // Line-wise cut of the first line.
    move_to(&mut o, 2);
    run(&mut o, "editor.cut");
    assert_eq!(state(&o), "┃aFROM t");
}

/// With `copy_line_without_selection` off, Copy and Cut without a selection do nothing.
#[gpui::test]
fn no_line_copy_when_the_setting_is_off(cx: &mut TestAppContext) {
    let cfg = EditorConfig {
        copy_line_without_selection: false,
        ..EditorConfig::default()
    };
    let mut o = open_with(cx, "SELECT ┃1", cfg);
    put(&mut o, "kept");
    run(&mut o, "editor.copy");
    run(&mut o, "editor.cut");
    assert_eq!(clipboard_text(&o).as_deref(), Some("kept"));
    assert_eq!(text(&o), "SELECT 1");
}

/// The platform's own paste (`EntityInputHandler::paste`) takes the same normalisation; file
/// paths on the clipboard are not text to insert.
#[gpui::test]
fn the_platform_paste_is_normalised_and_paths_are_not_text(cx: &mut TestAppContext) {
    let mut o = open(cx, "┃");
    o.editor.update_in(o.vcx, |v, window, cx| {
        v.paste(
            ClipboardItem::new_string("\u{FEFF}a\r\nb".into()),
            window,
            cx,
        );
    });
    o.vcx.run_until_parked();
    assert_eq!(text(&o), "a\nb");
    let paths = ClipboardItem {
        entries: vec![ClipboardEntry::ExternalPaths(ExternalPaths(
            vec![std::path::PathBuf::from("/tmp/x.sql")].into(),
        ))],
    };
    o.vcx.write_to_clipboard(paths);
    run(&mut o, "editor.paste");
    assert_eq!(text(&o), "a\nb");
}

/// Line-wise metadata from elsewhere that does not describe this text is ignored (pasted
/// inline).
#[gpui::test]
fn foreign_metadata_is_not_line_wise(cx: &mut TestAppContext) {
    let mut o = open(cx, "ab┃c");
    o.vcx
        .write_to_clipboard(ClipboardItem::new_string_with_metadata(
            "x\n".into(),
            r#"{"v":1,"src":"hacivat-editor","pieces":[{"len":99,"line_wise":true}]}"#.into(),
        ));
    run(&mut o, "editor.paste");
    assert_eq!(state(&o), "abx\n┃c");
}

/// Paste and cut are refused while composing (no binding runs then, F15; this is the editor's
/// own guard), and so is the platform's own paste (review M9: dropped, not held); copy still
/// copies (the caret's line, marked text included).
#[gpui::test]
fn paste_and_cut_are_refused_while_composing_and_copy_runs(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT «1┃");
    put(&mut o, "2");
    o.editor.update_in(o.vcx, |v, window, cx| {
        v.replace_and_mark_text_in_range(None, "ş", None, window, cx);
    });
    run(&mut o, "editor.paste");
    run(&mut o, "editor.cut");
    assert_eq!(
        clipboard_text(&o).as_deref(),
        Some("2"),
        "cut copied nothing"
    );
    o.editor.update_in(o.vcx, |v, window, cx| {
        v.paste(ClipboardItem::new_string("3".into()), window, cx);
    });
    o.vcx.run_until_parked();
    run(&mut o, "editor.copy");
    assert_eq!(clipboard_text(&o).as_deref(), Some("SELECT ş\n"));
    o.editor.update_in(o.vcx, |v, window, cx| {
        v.replace_text_in_range(None, "ş", window, cx);
    });
    o.vcx.run_until_parked();
    assert_eq!(
        text(&o),
        "SELECT ş",
        "nothing pasted after the composition either"
    );
}

#[gpui::test]
fn a_read_only_editor_copies_but_never_cuts_or_pastes(cx: &mut TestAppContext) {
    let cfg = EditorConfig {
        read_only: true,
        ..EditorConfig::default()
    };
    let mut o = open_with(cx, "SELECT «1┃", cfg);
    run(&mut o, "editor.copy");
    assert_eq!(clipboard_text(&o).as_deref(), Some("1"));
    run(&mut o, "editor.cut");
    put(&mut o, "2");
    run(&mut o, "editor.paste");
    o.editor.update_in(o.vcx, |v, window, cx| {
        v.paste(ClipboardItem::new_string("3".into()), window, cx);
    });
    o.vcx.run_until_parked();
    assert_eq!(text(&o), "SELECT 1");
}
