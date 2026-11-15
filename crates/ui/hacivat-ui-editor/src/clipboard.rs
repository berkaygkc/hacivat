//! The clipboard (UI-EDIT-14, M3a scope; Task 27): copy and cut (line-wise without a
//! selection), and a paste read asynchronously and applied to the selection current when it
//! lands, as one `InputPaste` transaction.
//!
//! - **Copy** takes the main selection; without one (and with `copy_line_without_selection`)
//!   the caret's whole line with its break, a break of the dominant kind added to a last line
//!   that has none. The clipboard string carries the piece metadata of UI-EDIT-14 step 1
//!   (`{"v":1,"src":"hacivat-editor","pieces":[{"len":…,"line_wise":…}]}`, one piece in M3a),
//!   so a line-wise copy pastes above the caret's line with the caret's column kept. Metadata
//!   that does not describe the text exactly is ignored.
//! - **Cut** copies, then deletes what it copied in one `DeleteCut` transaction; a line-wise
//!   cut of the last line takes the break before it.
//! - **Paste** reads with `read_from_clipboard_async` (never waiting on the UI thread) and
//!   normalises on the background executor: one leading U+FEFF stripped, every CRLF, CR and LF
//!   converted to the document's dominant ending (U+2028 is not a break), NULs kept, where the
//!   element draws each as a visible `NUL` chip, and counted for a warning toast. Only string
//!   entries are text: file paths and images are not inserted. A paste that would take the
//!   document past T1's 20 MB is refused with `EditorEvent::PasteRefused { bytes }` and a toast
//!   (R-M3-49); a failed read shows the clipboard-unavailable toast with Try again.
//! - **Never inside a composition:** paste and cut are refused while composing (no binding
//!   runs then either, F15), and a read that lands while composing waits until the composition
//!   ends (`ime::defer_while_composing`, the corrected R-M3-7 ruling). Copy changes no text and
//!   always runs.
//! - A read-only editor copies, and neither cuts nor pastes.

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use hacivat_text::{ChangeSet, LineEnding, Origin, SelectionSet, Transaction, UserEvent};
use hacivat_ui::actions::ActionFn;
use hacivat_ui::icon::IconName;
use hacivat_ui::l10n::UiArg;
use hacivat_ui::text::TextFn;
use hacivat_ui::toast::{Toast, ToastAction, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{App, ClipboardEntry, ClipboardItem, Context, Window};

use crate::commands::message;
use crate::ime::{self, AppEdit};
use crate::view::{EditorEvent, EditorView};

/// The module-owned state (P11): the clipboard reads in flight.
#[derive(Debug, Clone, Default)]
pub struct PasteState {
    reads: u32,
}

impl PasteState {
    /// Paste reads asked for whose text has not been applied (or refused) yet.
    pub fn reads_in_flight(&self) -> u32 {
        self.reads
    }
}

/// Normalised pasted text, ready to apply.
#[derive(Debug)]
pub(crate) struct Pasted {
    text: Arc<str>,
    nuls: usize,
    line_wise: bool,
}

/// The piece metadata of one copied piece (UI-EDIT-14 step 1).
fn metadata(len: usize, line_wise: bool) -> String {
    format!(
        r#"{{"v":1,"src":"hacivat-editor","pieces":[{{"len":{len},"line_wise":{line_wise}}}]}}"#
    )
}

/// What Copy takes: the text, whether it is line-wise, and the bytes Cut deletes.
fn copied(v: &EditorView) -> Option<(String, bool, Range<usize>)> {
    let buf = v.model.buffer();
    let main = v.model.selection().main().range();
    if !main.is_empty() {
        return Some((buf.slice(main.clone()).into_owned(), false, main));
    }
    if !v.config.copy_line_without_selection {
        return None;
    }
    let line = buf.byte_to_line(main.start);
    let whole = buf.line_range(line, true);
    let content = buf.line_range(line, false);
    let mut text = buf.slice(whole.clone()).into_owned();
    let cut = if whole.end > content.end {
        whole
    } else {
        // The last line has no break: copy it with one, and cut the break before it.
        text.push_str(buf.dominant_line_ending().as_str());
        let from = if line > 0 {
            buf.line_range(line - 1, false).end
        } else {
            whole.start
        };
        from..whole.end
    };
    Some((text, true, cut))
}

fn write(text: String, line_wise: bool, cx: &mut App) {
    let meta = metadata(text.len(), line_wise);
    cx.write_to_clipboard(ClipboardItem::new_string_with_metadata(text, meta));
}

pub fn copy(v: &mut EditorView, window: &mut Window, cx: &mut Context<EditorView>) {
    let _ = window;
    if let Some((text, line_wise, _)) = copied(v) {
        write(text, line_wise, cx);
    }
}

pub fn cut(v: &mut EditorView, window: &mut Window, cx: &mut Context<EditorView>) {
    let _ = window;
    if v.config.read_only || v.ime.is_composing() {
        return;
    }
    let Some((text, line_wise, range)) = copied(v) else {
        return;
    };
    write(text, line_wise, cx);
    let len = v.model.buffer().len_bytes();
    let Ok(changes) = ChangeSet::from_edits(len, [(range.clone(), Arc::from(""))]) else {
        return;
    };
    let ann = v.annotations(UserEvent::DeleteCut, Origin::User, cx);
    v.commit(
        Transaction {
            changes,
            selection: Some(SelectionSet::cursor(range.start)),
            ann,
        },
        cx,
    );
}

pub fn paste(v: &mut EditorView, window: &mut Window, cx: &mut Context<EditorView>) {
    let _ = window;
    if v.config.read_only || v.ime.is_composing() {
        return;
    }
    let read = cx.read_from_clipboard_async();
    v.paste.reads += 1;
    cx.spawn(async move |this, cx| {
        let result = read.await.map_err(|_| ());
        let _ = this.update(cx, |v, cx| {
            v.paste.reads = v.paste.reads.saturating_sub(1);
            landed(v, result, cx);
        });
    })
    .detach();
}

/// A clipboard read finished.
fn landed(
    v: &mut EditorView,
    result: Result<Option<ClipboardItem>, ()>,
    cx: &mut Context<EditorView>,
) {
    match result {
        Ok(Some(item)) => paste_item(v, &item, cx),
        Ok(None) => {}
        Err(()) => {
            let weak = cx.entity().downgrade();
            let retry: ActionFn = Rc::new(move |window: &mut Window, cx: &mut App| {
                let _ = weak.update(cx, |v, cx| paste(v, window, cx));
            });
            ToastQueue::push(cx, unavailable_toast(retry));
        }
    }
}

/// Pastes `item` (a landed read, or the platform's own paste): its string entries normalised
/// on the background executor, then applied.
pub(crate) fn paste_item(v: &mut EditorView, item: &ClipboardItem, cx: &mut Context<EditorView>) {
    if v.config.read_only {
        return;
    }
    let Some((text, line_wise)) = text_of(item) else {
        return;
    };
    let ending = v.model.buffer().dominant_line_ending();
    v.paste.reads += 1;
    let work = cx
        .background_executor()
        .spawn(async move { normalize(&text, ending, line_wise) });
    cx.spawn(async move |this, cx| {
        let pasted = work.await;
        let _ = this.update(cx, |v, cx| {
            v.paste.reads = v.paste.reads.saturating_sub(1);
            apply(v, pasted, cx);
        });
    })
    .detach();
}

/// The text of `item`'s string entries, and whether its metadata says line-wise for exactly
/// that text; `None` when it holds no text.
fn text_of(item: &ClipboardItem) -> Option<(String, bool)> {
    let mut text = String::new();
    let mut meta = None;
    let mut strings = 0;
    for entry in item.entries() {
        if let ClipboardEntry::String(s) = entry {
            text.push_str(s.text());
            meta = s.metadata.clone();
            strings += 1;
        }
    }
    if text.is_empty() {
        return None;
    }
    let line_wise = strings == 1 && meta.as_deref() == Some(&*metadata(text.len(), true));
    Some((text, line_wise))
}

/// UI-EDIT-14 step 3: one leading BOM stripped, endings converted to `ending`, NULs counted.
fn normalize(text: &str, ending: LineEnding, line_wise: bool) -> Pasted {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let nl = ending.as_str();
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(['\r', '\n']) {
        out.push_str(&rest[..i]);
        out.push_str(nl);
        let skip = if rest[i..].starts_with("\r\n") { 2 } else { 1 };
        rest = &rest[i + skip..];
    }
    out.push_str(rest);
    let nuls = out.bytes().filter(|&b| b == 0).count();
    Pasted {
        text: Arc::from(out),
        nuls,
        line_wise,
    }
}

/// Applies normalised pasted text to the selection current now: above the caret's line when
/// it is line-wise and nothing is selected, else over the main selection. Held while a
/// composition is open; refused past the size limit (R-M3-49).
pub(crate) fn apply(v: &mut EditorView, p: Pasted, cx: &mut Context<EditorView>) {
    if v.config.read_only {
        return;
    }
    if v.ime.is_composing() {
        ime::defer_while_composing(v, AppEdit::Paste(p));
        return;
    }
    let buf = v.model.buffer();
    let len = buf.len_bytes();
    let main = v.model.selection().main().range();
    let (at, caret) = if p.line_wise && main.is_empty() {
        let start = buf.line_to_byte(buf.byte_to_line(main.start));
        (start..start, main.start + p.text.len())
    } else {
        (main.clone(), main.start + p.text.len())
    };
    let after = (len - at.len() + p.text.len()) as u64;
    let limit = v.config.tiers.reduced_max_bytes;
    if after > limit {
        let bytes = p.text.len() as u64;
        cx.emit(EditorEvent::PasteRefused { bytes });
        ToastQueue::push(cx, refused_toast(bytes, limit));
        return;
    }
    let Ok(changes) = ChangeSet::from_edits(len, [(at, p.text.clone())]) else {
        return;
    };
    let ann = v.annotations(UserEvent::InputPaste, Origin::User, cx);
    let done = v.commit(
        Transaction {
            changes,
            selection: Some(SelectionSet::cursor(caret)),
            ann,
        },
        cx,
    );
    if done.is_some() && p.nuls > 0 {
        ToastQueue::push(cx, nul_toast(p.nuls));
    }
}

/// Whole megabytes, rounded up (a size shown as "{ $n } MB").
fn megabytes(bytes: u64) -> u64 {
    bytes.div_ceil(1 << 20).max(1)
}

fn size_text(cx: &App, bytes: u64) -> UiArg {
    UiArg::Text(message(
        cx,
        "hui-editor-size-mb",
        &[("n", UiArg::Count(megabytes(bytes)))],
    ))
}

/// A wide toast in ink (Hue Is Meaning: errors keep the ink, never a new hue).
fn notice(icon: IconName, title: TextFn, body: TextFn) -> Toast {
    let mut toast = Toast::new(ToastKind::Info, body);
    toast.title = Some(title);
    toast.icon = Some(icon);
    toast.wide = true;
    toast
}

fn refused_toast(bytes: u64, limit: u64) -> Toast {
    notice(
        IconName::CircleAlert,
        Rc::new(|cx: &App| message(cx, "hui-editor-paste-refused-title", &[])),
        Rc::new(move |cx: &App| {
            message(
                cx,
                "hui-editor-paste-refused-body",
                &[
                    ("size", size_text(cx, bytes)),
                    ("limit", size_text(cx, limit)),
                ],
            )
        }),
    )
}

fn nul_toast(n: usize) -> Toast {
    let n = n as u64;
    notice(
        IconName::TriangleAlert,
        Rc::new(|cx: &App| message(cx, "hui-editor-nul-title", &[])),
        Rc::new(move |cx: &App| message(cx, "hui-editor-nul-body", &[("n", UiArg::Count(n))])),
    )
}

/// The read failed: a sticky toast (it carries an action) with Try again.
fn unavailable_toast(retry: ActionFn) -> Toast {
    let mut toast = notice(
        IconName::ClipboardX,
        Rc::new(|cx: &App| message(cx, "hui-editor-clipboard-title", &[])),
        Rc::new(|cx: &App| message(cx, "hui-editor-clipboard-body", &[])),
    );
    toast.sticky = true;
    toast.actions = vec![ToastAction {
        label: Rc::new(|cx: &App| message(cx, "hui-editor-try-again", &[])),
        run: retry,
    }];
    toast
}

// ## m3a-t27-ime-clipboard: the parts no window reaches (the read failure, the toasts' text).
#[cfg(test)]
mod t27_tests {
    use std::cell::RefCell;
    use std::sync::Arc as StdArc;

    use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext, div, px};
    use hacivat_text::TextBuffer;
    use hacivat_ui::l10n::{EnglishDefaults, UiText};
    use hacivat_ui_core::gpui_surface::{IntoElement, ParentElement as _, Render, Styled as _};

    use super::*;
    use crate::config::{DocId, EditorConfig};
    use crate::service::FailClosedService;

    #[test]
    fn normalisation_strips_one_leading_bom_and_converts_every_ending() {
        let crlf = TextBuffer::from_str("a\r\nb\r\n").dominant_line_ending();
        let p = normalize("\u{FEFF}\u{FEFF}a\nb\r\nc\rd\u{2028}e\0", crlf, false);
        assert_eq!(&*p.text, "\u{FEFF}a\r\nb\r\nc\r\nd\u{2028}e\0");
        assert_eq!(p.nuls, 1);
        let cr = TextBuffer::from_str("a\rb\rc").dominant_line_ending();
        assert_eq!(&*normalize("x\r\ny\nz", cr, false).text, "x\ry\rz");
        // A lone CR at the end and a CRLF split across nothing else.
        let lf = TextBuffer::from_str("").dominant_line_ending();
        assert_eq!(&*normalize("\r\r\n\n", lf, false).text, "\n\n\n");
    }

    #[test]
    fn metadata_marks_line_wise_only_for_its_own_text() {
        let item = |text: &str, meta: String| {
            ClipboardItem::new_string_with_metadata(text.to_string(), meta)
        };
        assert_eq!(
            text_of(&item("ab\n", metadata(3, true))),
            Some(("ab\n".into(), true))
        );
        assert_eq!(
            text_of(&item("ab\n", metadata(4, true))),
            Some(("ab\n".into(), false))
        );
        assert_eq!(
            text_of(&item("ab", metadata(2, false))),
            Some(("ab".into(), false))
        );
        assert_eq!(text_of(&ClipboardItem::new_string(String::new())), None);
    }

    /// The toasts' texts come from `hui.ftl` (block `m3a-t27-clipboard`) with their sizes and
    /// counts as Fluent arguments.
    #[gpui::test]
    fn the_toasts_read_their_fluent_texts(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(UiText(StdArc::new(EnglishDefaults::new())));
            let r = refused_toast(31_400_000, 20 << 20);
            assert_eq!(r.title.as_ref().unwrap()(cx), "Couldn't paste");
            assert_eq!(
                (r.text)(cx),
                "The pasted text is \u{2068}\u{2068}30\u{2069} MB\u{2069}, which would take \
                 this document past \u{2068}\u{2068}20\u{2069} MB\u{2069}. Paste a smaller part."
            );
            assert_eq!(
                (r.icon, r.wide, r.kind),
                (Some(IconName::CircleAlert), true, ToastKind::Info)
            );
            let one = nul_toast(1);
            assert_eq!(
                (one.text)(cx),
                "\u{2068}1\u{2069} NUL character is marked in the text so you can find it. \
                 Most servers reject it."
            );
            let two = nul_toast(2);
            assert_eq!(
                two.title.as_ref().unwrap()(cx),
                "The pasted text contains NUL characters"
            );
            assert!((two.text)(cx).starts_with("\u{2068}2\u{2069} NUL characters are marked"));
            let u = unavailable_toast(Rc::new(|_: &mut Window, _: &mut App| {}));
            assert!(u.sticky);
            assert_eq!(u.icon, Some(IconName::ClipboardX));
            assert_eq!(u.title.as_ref().unwrap()(cx), "Couldn't read the clipboard");
            assert_eq!((u.actions[0].label)(cx), "Try again");
        });
    }

    struct Host {
        editor: Entity<EditorView>,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().w(px(800.0)).h(px(600.0)).child(self.editor.clone())
        }
    }

    fn open(cx: &mut TestAppContext) -> (Entity<EditorView>, &mut VisualTestContext) {
        let slot: Rc<RefCell<Option<Entity<EditorView>>>> = Rc::default();
        let out = slot.clone();
        let (_, vcx) = cx.add_window_view(move |window, cx| {
            let editor = cx.new(|cx| {
                EditorView::new(
                    DocId(14),
                    "SELECT 1",
                    EditorConfig::default(),
                    Rc::new(FailClosedService),
                    Rc::new(|_: &App| "Console 1".into()),
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

    /// A failed read (the clipboard denied or unavailable) changes nothing and shows the
    /// clipboard-unavailable toast; an empty clipboard shows nothing.
    #[gpui::test]
    fn a_failed_read_shows_the_unavailable_toast(cx: &mut TestAppContext) {
        let (e, vcx) = open(cx);
        e.update(vcx, |v, cx| landed(v, Ok(None), cx));
        assert!(vcx.read(|cx| ToastQueue::visible_ids(cx).is_empty()));
        e.update(vcx, |v, cx| landed(v, Err(()), cx));
        vcx.run_until_parked();
        assert_eq!(vcx.read(|cx| ToastQueue::visible_ids(cx).len()), 1);
        assert_eq!(
            vcx.read(|cx| e.read(cx).snapshot().buffer.to_string()),
            "SELECT 1"
        );
        assert_eq!(vcx.read(|cx| e.read(cx).paste.reads_in_flight()), 0);
    }
}
