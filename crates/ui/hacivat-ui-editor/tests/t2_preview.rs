//! M4b Task 11: the T2 read-only preview (UI-EDIT-23 step 1, R-M4b-20; design/m4b-scripts
//! surface 9) and the 20–100 MB paste confirmation (UI-EDIT-14 step 5). A preview is
//! read-only, shows the T2 banner, carries its text in the accessible description, says
//! "preview only" to find, and stays a preview whatever the host's configuration says. A paste
//! that would make the document 20–100 MB asks the host (`PasteNeedsConfirm`) when the host
//! takes that question; past 100 MB it is refused. Tests wait on conditions
//! (`run_until_parked`), never on a clock.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AppContext as _, ClipboardItem, Context, Entity, EntityInputHandler as _, Focusable as _,
    IntoElement, ParentElement as _, Render, Styled as _, TestAppContext, VisualTestContext,
    Window, div, px,
};
use hacivat_ui::button::ButtonKind;
use hacivat_ui::l10n::{EnglishDefaults, UiText};
use hacivat_ui::toast::ToastQueue;
use hacivat_ui_core::theme::{Theme, ThemeMode};
use hacivat_ui_editor::clipboard::{self, PASTE_CONFIRM_CANCEL, PASTE_CONFIRM_PREVIEW};
use hacivat_ui_editor::large_file::{self, preview_cut};
use hacivat_ui_editor::{
    DocId, EditorConfig, EditorEvent, EditorView, FailClosedService, PasteAnswer, PreviewSpec,
    Tier, TierLimits,
};

const MB: u64 = 1 << 20;

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

fn open_with<'a>(cx: &'a mut TestAppContext, text: &str, cfg: EditorConfig) -> Opened<'a> {
    cx.update(|cx| {
        Theme::install(cx, ThemeMode::Dark);
        cx.set_global(UiText(Arc::new(EnglishDefaults::new())));
    });
    let text = text.to_string();
    let slot: Rc<RefCell<Option<Entity<EditorView>>>> = Rc::default();
    let out = slot.clone();
    let (_, vcx) = cx.add_window_view(move |window, cx| {
        let editor = cx.new(|cx| {
            EditorView::new(
                DocId(12),
                &text,
                cfg,
                Rc::new(FailClosedService),
                Rc::new(|_: &gpui::App| "dump.sql".into()),
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
    });
    draw(vcx);
    events.borrow_mut().clear();
    Opened {
        editor,
        events,
        vcx,
    }
}

fn open<'a>(cx: &'a mut TestAppContext, text: &str) -> Opened<'a> {
    open_with(cx, text, EditorConfig::default())
}

/// The configuration of a host that answers `PasteNeedsConfirm` (Task 18's).
fn asking() -> EditorConfig {
    EditorConfig {
        confirm_large_paste: true,
        ..EditorConfig::default()
    }
}

fn text(o: &Opened<'_>) -> String {
    o.vcx
        .read(|cx| o.editor.read(cx).snapshot().buffer.to_string())
}

fn run(o: &mut Opened<'_>, id: &str) {
    o.editor
        .update_in(o.vcx, |v, window, cx| v.run_command(id, window, cx));
    o.vcx.run_until_parked();
}

fn put(o: &mut Opened<'_>, s: String) {
    o.vcx.write_to_clipboard(ClipboardItem::new_string(s));
}

fn paste(o: &mut Opened<'_>, s: String) {
    put(o, s);
    run(o, "editor.paste");
}

fn move_to(o: &mut Opened<'_>, at: usize) {
    o.editor.update(o.vcx, |v, cx| v.select_range(at..at, cx));
    o.vcx.run_until_parked();
}

fn asked(o: &Opened<'_>) -> Vec<(u64, u64)> {
    o.events
        .borrow()
        .iter()
        .filter_map(|e| match e {
            EditorEvent::PasteNeedsConfirm { bytes, total } => Some((*bytes, *total)),
            _ => None,
        })
        .collect()
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

fn as_preview(o: &Opened<'_>) -> Vec<String> {
    o.events
        .borrow()
        .iter()
        .filter_map(|e| match e {
            EditorEvent::PasteAsPreview { text } => Some(text.text().to_string()),
            _ => None,
        })
        .collect()
}

/// Fluent's bidi isolation marks removed (the English defaults wrap placeables in them).
fn plain(s: &str) -> String {
    s.replace(['\u{2068}', '\u{2069}'], "")
}

fn toasts(o: &Opened<'_>) -> usize {
    o.vcx.read(|cx| ToastQueue::visible_ids(cx).len())
}

fn preview(o: &mut Opened<'_>, shown: u64, total: u64) {
    o.editor.update(o.vcx, |v, cx| {
        v.set_preview(
            PreviewSpec {
                shown_bytes: shown,
                total_bytes: total,
            },
            cx,
        )
    });
    draw(o.vcx);
}

// ---------------------------------------------------------------------------------------------
// The T2 preview.

/// A preview is read-only: typing, pasting, cutting, a command that edits and undo all leave
/// the text; the selection still moves. The banner's text is the accessible description, the
/// status says T2, the host heard `TierChanged(Preview)`.
#[gpui::test]
fn preview_is_read_only_and_announces_it(cx: &mut TestAppContext) {
    let src = "SELECT 1;\nSELECT 2;\n";
    let mut o = open(cx, src);
    preview(&mut o, 2_621_440, 148 * MB);
    assert_eq!(
        o.vcx.read(|cx| o.editor.read(cx).status().tier),
        Tier::Preview
    );
    assert!(
        o.events
            .borrow()
            .iter()
            .any(|e| matches!(e, EditorEvent::TierChanged(Tier::Preview))),
        "{:?}",
        o.events.borrow()
    );
    assert_eq!(
        o.vcx.read(|cx| o.editor.read(cx).preview()),
        Some(PreviewSpec {
            shown_bytes: 2_621_440,
            total_bytes: 148 * MB,
        })
    );
    // Typing does nothing.
    o.vcx.simulate_input("x");
    o.editor.update_in(o.vcx, |v, window, cx| {
        v.replace_text_in_range(None, "y", window, cx);
    });
    o.vcx.run_until_parked();
    assert_eq!(text(&o), src);
    // Nor a paste, a cut, a line command or undo.
    paste(&mut o, "DROP TABLE t;".into());
    run(&mut o, "editor.cut");
    run(&mut o, "editor.delete_line");
    run(&mut o, "editor.undo");
    assert_eq!(text(&o), src);
    // The selection still moves (reading, copying).
    move_to(&mut o, 3);
    assert_eq!(
        o.vcx
            .read(|cx| o.editor.read(cx).snapshot().selection.main().head),
        3
    );
    // The banner's two lines are the editor's accessible description.
    let description = o
        .vcx
        .read(|cx| large_file::description(large_file::state(o.editor.read(cx)), cx));
    let d = plain(&description.expect("a preview describes itself"));
    assert!(d.contains("Read-only preview: this file is 148.0 MB."), "{d}");
    assert!(
        d.contains("The editor shows its first 2.5 MB. Run as script sends the whole file."),
        "{d}"
    );
    // Find says it searches the preview only.
    assert!(o.vcx.read(|cx| o.editor.read(cx).find_status().preview_only));
}

/// The host's configuration cannot make a preview editable again, and an edit by the host
/// (a restored text) does not move it out of T2.
#[gpui::test]
fn a_preview_stays_a_preview(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT 1;\n");
    preview(&mut o, 10, 30 * MB);
    o.editor
        .update(o.vcx, |v, cx| v.set_config(EditorConfig::default(), cx));
    assert!(o.vcx.read(|cx| o.editor.read(cx).config().read_only));
    o.vcx.simulate_input("x");
    assert_eq!(text(&o), "SELECT 1;\n");
    o.editor
        .update(o.vcx, |v, cx| v.set_text("SELECT 2;\n", cx));
    o.vcx.run_until_parked();
    assert_eq!(
        o.vcx.read(|cx| o.editor.read(cx).status().tier),
        Tier::Preview
    );
}

/// The banner (NOTES §9): drawn above the text with its own ids, never the T1 ones.
#[gpui::test]
fn the_preview_draws_the_t2_banner(cx: &mut TestAppContext) {
    let mut o = open(cx, "SELECT 1;\n");
    preview(&mut o, 2_621_440, 148 * MB);
    let (t1, t2) = (
        o.vcx.debug_bounds("editor-t1-banner"),
        o.vcx.debug_bounds("editor-t2-banner"),
    );
    assert!(t1.is_none(), "no T1 banner on a preview");
    let banner = t2.expect("the T2 banner is drawn");
    let text_top = o
        .vcx
        .read(|cx| o.editor.read(cx).last_layout().unwrap().rows().bounds.origin.y);
    assert!(banner.bottom() <= text_top, "{banner:?} above the text at {text_top:?}");
}

/// UI-EDIT-23 step 1: the preview is cut at the last line break at or before its size, never
/// between a CR and its LF; a text within the size is whole.
#[test]
fn the_preview_cut_ends_at_a_line_break() {
    assert_eq!(preview_cut("SELECT 1;\nSELECT 2;\n", 100), 20);
    assert_eq!(preview_cut("SELECT 1;\nSELECT 2;\n", 15), 10);
    // A cut at 10 would fall between the CR and the LF: the CR goes too.
    assert_eq!(preview_cut("SELECT 1;\r\nSELECT 2;\r\n", 10), 9);
    assert_eq!(preview_cut("SELECT 1;\r\nSELECT 2;\r\n", 11), 11);
    assert_eq!(preview_cut("a\rb\rc", 3), 2);
    // No line break within the size: the cut falls back to a character boundary.
    assert_eq!(preview_cut("şşşş", 3), 2);
    assert_eq!(
        TierLimits::default().preview_bytes,
        2_621_440,
        "2.5 MB, DataGrip's intellisense limit (UI-EDIT-23)"
    );
}

// ---------------------------------------------------------------------------------------------
// The 20–100 MB paste confirmation.

/// A paste that makes the document between 20 and 100 MB asks the host first and changes
/// nothing until the answer; Cancel drops it, Paste as preview hands the host the document
/// as the paste would make it (the editor's own text stays), and Paste pastes.
#[gpui::test]
fn paste_between_20_and_100_mb_asks(cx: &mut TestAppContext) {
    let mut o = open_with(cx, "SELECT 1;\n", asking());
    move_to(&mut o, 10);
    let big = "a".repeat((21 << 20) as usize);
    paste(&mut o, big.clone());
    let total = 10 + (21 << 20);
    assert_eq!(asked(&o), [(21 << 20, total)]);
    assert_eq!(text(&o), "SELECT 1;\n", "nothing pasted before the answer");
    assert!(refused(&o).is_empty());
    assert_eq!(toasts(&o), 0, "the host's dialog asks, not a toast");
    assert_eq!(
        o.vcx.read(|cx| o.editor.read(cx).pending_paste()),
        Some((21 << 20, total))
    );
    // Cancel: nothing happens, and nothing waits any more.
    o.editor
        .update(o.vcx, |v, cx| v.confirm_paste(PasteAnswer::Cancel, cx));
    o.vcx.run_until_parked();
    assert_eq!(text(&o), "SELECT 1;\n");
    assert_eq!(o.vcx.read(|cx| o.editor.read(cx).pending_paste()), None);
    // Paste as preview: the document as it would be, for the host to show as a preview.
    paste(&mut o, big.clone());
    o.editor
        .update(o.vcx, |v, cx| v.confirm_paste(PasteAnswer::AsPreview, cx));
    o.vcx.run_until_parked();
    let docs = as_preview(&o);
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].len() as u64, total);
    assert!(docs[0].starts_with("SELECT 1;\naaaa") && docs[0].ends_with("aaaa"));
    assert_eq!(text(&o), "SELECT 1;\n", "the editor's own text stays");
    // Paste: the host chose to edit it in reduced mode.
    paste(&mut o, big);
    o.editor
        .update(o.vcx, |v, cx| v.confirm_paste(PasteAnswer::Paste, cx));
    o.vcx.run_until_parked();
    assert_eq!(text(&o).len() as u64, total);
    assert_eq!(
        o.vcx.read(|cx| o.editor.read(cx).status().tier),
        Tier::Reduced
    );
    // An answer with nothing waiting does nothing.
    o.editor
        .update(o.vcx, |v, cx| v.confirm_paste(PasteAnswer::Paste, cx));
    assert_eq!(text(&o).len() as u64, total);
}

/// Past 100 MB a paste is refused even when the host would ask, with the refusal toast.
#[gpui::test]
fn paste_over_100_mb_is_refused(cx: &mut TestAppContext) {
    let mut o = open_with(cx, "", asking());
    let huge = "a".repeat(((100 << 20) + 1) as usize);
    paste(&mut o, huge);
    assert_eq!(refused(&o), [(100 << 20) + 1]);
    assert!(asked(&o).is_empty());
    assert_eq!(text(&o), "");
    assert_eq!(toasts(&o), 1);
}

/// A host that does not take the question refuses past 20 MB, as M3 did (R-M3-49): nothing
/// asks, so nothing waits for an answer that never comes.
#[gpui::test]
fn a_host_that_does_not_ask_refuses_past_20_mb(cx: &mut TestAppContext) {
    let mut o = open(cx, "");
    paste(&mut o, "a".repeat((21 << 20) as usize));
    assert_eq!(refused(&o), [21 << 20]);
    assert!(asked(&o).is_empty());
    assert_eq!(o.vcx.read(|cx| o.editor.read(cx).pending_paste()), None);
}

/// The document's size decides, not the paste's: replacing a selection counts it out, and a
/// paste that keeps the document within 20 MB pastes at once.
#[gpui::test]
fn the_question_counts_the_document_after_the_paste(cx: &mut TestAppContext) {
    let five = "b".repeat((5 << 20) as usize);
    let mut o = open_with(cx, &five, asking());
    // Selecting all 5 MB and pasting 18 MB makes 18 MB: no question.
    o.editor
        .update(o.vcx, |v, cx| v.select_range(0..(5 << 20), cx));
    paste(&mut o, "a".repeat((18 << 20) as usize));
    assert!(asked(&o).is_empty());
    assert_eq!(text(&o).len() as u64, 18 << 20);
    // 3 MB more at the end makes 21 MB: asked.
    move_to(&mut o, (18 << 20) as usize);
    paste(&mut o, "c".repeat((3 << 20) as usize));
    assert_eq!(asked(&o), [(3 << 20, 21 << 20)]);
}

/// The dialog (design/m4b-scripts surface 9): a 420 px alertdialog, "Paste as preview"
/// Default, Cancel Primary, focused first and Escape's answer; its copy follows the sizes.
#[gpui::test]
fn the_confirmation_dialog_is_safe_by_default(cx: &mut TestAppContext) {
    let o = open(cx, "");
    let spec = clipboard::paste_confirm_dialog(44_669_747, 63_963_136);
    assert_eq!(spec.size, hacivat_ui::dialog::DialogSize::Standard);
    assert_eq!(spec.buttons.len(), 2);
    assert_eq!(spec.primary, PASTE_CONFIRM_CANCEL);
    assert_eq!(spec.cancel, PASTE_CONFIRM_CANCEL);
    assert_eq!(spec.buttons[PASTE_CONFIRM_CANCEL].kind, ButtonKind::Primary);
    assert_eq!(spec.buttons[PASTE_CONFIRM_PREVIEW].kind, ButtonKind::Default);
    assert_eq!(clipboard::paste_answer(PASTE_CONFIRM_PREVIEW), PasteAnswer::AsPreview);
    assert_eq!(clipboard::paste_answer(PASTE_CONFIRM_CANCEL), PasteAnswer::Cancel);
    assert_eq!(clipboard::paste_answer(7), PasteAnswer::Cancel, "fail closed");
    o.vcx.read(|cx| {
        assert_eq!(plain(&(spec.title)(cx)), "Paste 42.6 MB?");
        assert_eq!(
            plain(&(spec.body)(cx)),
            "That makes this document 61.0 MB, too large to edit. Paste it as a read-only \
             preview, or cancel."
        );
        assert_eq!(
            (spec.buttons[PASTE_CONFIRM_PREVIEW].label)(cx).as_ref(),
            "Paste as preview"
        );
        assert_eq!((spec.buttons[PASTE_CONFIRM_CANCEL].label)(cx).as_ref(), "Cancel");
    });
}
