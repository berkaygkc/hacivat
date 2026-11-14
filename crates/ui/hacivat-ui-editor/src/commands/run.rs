//! Run, Run 2 and Run 3, and Describe execution range (Task 25).
//!
//! - **Run** asks the host to execute (`EditorEvent::ExecuteRequested`): the document, the
//!   action, the exact snapshot, and where: the non-empty selections when there is one, else
//!   the carets. It changes nothing in the editor. The command is flagged `seals_history`, so
//!   typing before and after a Run is two undo groups. While an IME composition is open it
//!   refuses (the dispatcher never fires a binding then, F15; this covers every other path).
//! - **Describe execution range** says what Run would execute, in one sentence, to assistive
//!   technology (`EditorView::announce`) and to the status bar (`commands::status_message`). It
//!   reads the language service's preview at the editor's version and never guesses (R-M3-8): a
//!   missing or uncertain answer is said as such.

use std::ops::Range;

use hacivat_text::Snapshot;
use hacivat_text::units::SCAN_LIMIT;
use hacivat_ui::l10n::UiArg;
use hacivat_ui_core::gpui_surface::{App, Context, SharedString, Window};
use smallvec::SmallVec;
use unicode_segmentation::UnicodeSegmentation as _;

use super::{message, say};
use crate::service::{ExecAction, ExecAt, ExecPreview, ExecRequest, PreviewState};
use crate::view::{EditorEvent, EditorView};

type Cx<'a> = Context<'a, EditorView>;

/// The most characters of a statement's first line a sentence quotes.
const LABEL_CHARS: usize = 60;

/// Where Run is asked on `snap`: its non-empty selections, else its carets.
fn exec_at(snap: &Snapshot) -> ExecAt {
    let ranges = snap.selection.ranges();
    let selected: SmallVec<[Range<usize>; 1]> = ranges
        .iter()
        .filter(|r| !r.is_empty())
        .map(|r| r.range())
        .collect();
    if selected.is_empty() {
        ExecAt::Carets(ranges.iter().map(|r| r.head).collect())
    } else {
        ExecAt::Selections(selected)
    }
}

fn request(v: &mut EditorView, action: ExecAction, cx: &mut Cx<'_>) {
    super::acted(v);
    if v.ime.is_composing() {
        return;
    }
    let snapshot = v.model.snapshot();
    let at = exec_at(&snapshot);
    cx.emit(EditorEvent::ExecuteRequested(ExecRequest {
        doc: v.doc,
        action,
        at,
        snapshot,
    }));
}

pub(super) fn run(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    request(v, ExecAction::Execute1, cx);
}

pub(super) fn run_2(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    request(v, ExecAction::Execute2, cx);
}

pub(super) fn run_3(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    request(v, ExecAction::Execute3, cx);
}

/// The first line of the statement at `r`, at most [`LABEL_CHARS`] characters of it (then an
/// ellipsis): data, quoted in the sentence.
fn label(snap: &Snapshot, r: &Range<usize>) -> SharedString {
    let buf = &snap.buffer;
    let line_end = buf.line_range(buf.byte_to_line(r.start), false).end;
    let end = buf.snap(
        r.end.min(line_end).min(r.start + SCAN_LIMIT),
        hacivat_text::Bias::Before,
    );
    let text = buf.slice(r.start..end.max(r.start));
    let text = text.trim();
    let mut clusters = text.grapheme_indices(true);
    match clusters.nth(LABEL_CHARS) {
        Some((cut, _)) => format!("{}…", &text[..cut]).into(),
        None => SharedString::from(text.to_string()),
    }
}

/// The sentence for `preview` (`None`: the service has no analysis of this version yet).
fn sentence(snap: &Snapshot, preview: Option<&ExecPreview>, cx: &App) -> SharedString {
    let Some(preview) = preview else {
        return message(cx, "hv-editor-describe-pending", &[]);
    };
    match preview.state {
        PreviewState::Nothing => return message(cx, "hv-editor-describe-nothing", &[]),
        PreviewState::Uncertain => return message(cx, "hv-editor-describe-uncertain", &[]),
        PreviewState::NoConnection => return message(cx, "hv-editor-describe-offline", &[]),
        PreviewState::AskPreselected => return message(cx, "hv-editor-describe-ask", &[]),
        PreviewState::Units => {}
    }
    let len = snap.buffer.len_bytes();
    let (Some(first), Some(last)) = (preview.ranges.first(), preview.ranges.last()) else {
        return message(cx, "hv-editor-describe-nothing", &[]);
    };
    if first.start > len || last.end > len || !snap.buffer.is_char_boundary(first.start) {
        // A range that is not of this text: say nothing about it.
        return message(cx, "hv-editor-describe-pending", &[]);
    }
    let line = |at: usize| UiArg::Count(snap.buffer.byte_to_line(at) as u64 + 1);
    let from = line(first.start);
    // A range that ends at a line's start does not reach that line.
    let to = line(last.end.saturating_sub(1).max(last.start));
    let one_line = from == to;
    match (preview.ranges.len(), one_line) {
        (1, true) => message(
            cx,
            "hv-editor-describe-one-line",
            &[("line", from), ("label", UiArg::Text(label(snap, first)))],
        ),
        (1, false) => message(
            cx,
            "hv-editor-describe-one",
            &[
                ("from", from),
                ("to", to),
                ("label", UiArg::Text(label(snap, first))),
            ],
        ),
        (n, true) => message(
            cx,
            "hv-editor-describe-many-line",
            &[("n", UiArg::Count(n as u64)), ("line", from)],
        ),
        (n, false) => message(
            cx,
            "hv-editor-describe-many",
            &[("n", UiArg::Count(n as u64)), ("from", from), ("to", to)],
        ),
    }
}

pub(super) fn describe_exec_range(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    super::acted(v);
    let snap = v.model.snapshot();
    let preview =
        v.service
            .exec_preview(v.doc, snap.version, &exec_at(&snap), ExecAction::Execute1);
    let text = sentence(&snap, preview.as_ref(), cx);
    say(v, text, cx);
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::{Entity, TestAppContext, VisualTestContext};
    use hacivat_text::UserEvent;

    use super::*;
    use crate::config::{DocId, EditorConfig};
    use crate::service::FailClosedService;

    fn open<'a>(
        cx: &'a mut TestAppContext,
        text: &'static str,
    ) -> (Entity<EditorView>, &'a mut VisualTestContext) {
        let (editor, vcx) = cx.add_window_view(move |window, cx| {
            EditorView::new(
                DocId(25),
                text,
                EditorConfig::default(),
                Rc::new(FailClosedService),
                Rc::new(|_: &App| "Console 1".into()),
                window,
                cx,
            )
        });
        (editor, vcx)
    }

    fn mark(e: &Entity<EditorView>, vcx: &mut VisualTestContext, text: &str) {
        e.update(vcx, |v, cx| {
            let now = cx.background_executor().now();
            let c = v
                .ime
                .mark(&mut v.model, None, text, None, now)
                .expect("the mark applies");
            v.apply_committed(c, UserEvent::InputTypeCompose, cx);
            assert!(v.ime.is_composing());
        });
    }

    /// Run refuses while a composition is open, on the path every caller takes (`run_command`:
    /// the keymap, the context menu, the host's toolbar and menu bar): nothing is asked, the
    /// composition goes on, and its undo group stays whole, so one undo after it is committed
    /// takes back all of the composed text.
    #[gpui::test]
    fn run_while_composing_does_nothing(cx: &mut TestAppContext) {
        let (e, vcx) = open(cx, "SELECT ");
        let asked: Rc<RefCell<u32>> = Rc::default();
        let seen = asked.clone();
        vcx.update(|_, cx| {
            cx.subscribe(&e, move |_, ev: &EditorEvent, _| {
                if matches!(ev, EditorEvent::ExecuteRequested(_)) {
                    *seen.borrow_mut() += 1;
                }
            })
            .detach();
        });
        e.update(vcx, |v, cx| v.select_range(7..7, cx));
        mark(&e, vcx, "ş");
        for id in ["editor.run", "editor.run_2", "editor.run_3"] {
            e.update_in(vcx, |v, window, cx| {
                assert!(v.run_command(id, window, cx));
                assert!(v.ime.is_composing(), "{id} left the composition open");
            });
        }
        vcx.run_until_parked();
        assert_eq!(*asked.borrow(), 0);
        // The composition goes on and is committed: still one undo group.
        mark(&e, vcx, "şu");
        e.update_in(vcx, |v, window, cx| {
            let now = cx.background_executor().now();
            let c = v
                .ime
                .commit(&mut v.model, None, "şu", now)
                .expect("the text is accepted");
            v.apply_committed(c, UserEvent::InputType, cx);
            assert!(!v.ime.is_composing());
            assert_eq!(v.model.buffer().to_string(), "SELECT şu");
            assert_eq!(v.model.undo_depth(), 1, "one composition, one group");
            // Once it is committed, Run asks.
            assert!(v.run_command("editor.run", window, cx));
            assert!(v.run_command("editor.undo", window, cx));
            assert_eq!(v.model.buffer().to_string(), "SELECT ");
        });
        vcx.run_until_parked();
        assert_eq!(*asked.borrow(), 1);
    }

    /// Undo is not the IME's edit: it ends the composition first (which stays its own undo
    /// group), tells the host, and then undoes that group through the one fan-out.
    #[gpui::test]
    fn undo_ends_the_composition_first(cx: &mut TestAppContext) {
        let (e, vcx) = open(cx, "SELECT ");
        let events: Rc<RefCell<Vec<String>>> = Rc::default();
        let seen = events.clone();
        vcx.update(|_, cx| {
            cx.subscribe(&e, move |_, ev: &EditorEvent, _| {
                seen.borrow_mut().push(format!("{ev:?}"));
            })
            .detach();
        });
        e.update(vcx, |v, cx| v.select_range(7..7, cx));
        mark(&e, vcx, "ş");
        events.borrow_mut().clear();
        e.update_in(vcx, |v, window, cx| {
            assert!(v.run_command("editor.undo", window, cx));
            assert!(!v.ime.is_composing());
            assert_eq!(v.model.buffer().to_string(), "SELECT ");
            assert_eq!((v.model.undo_depth(), v.model.redo_depth()), (0, 1));
            assert!(v.run_command("editor.redo", window, cx));
            assert_eq!(v.model.buffer().to_string(), "SELECT ş");
        });
        vcx.run_until_parked();
        let events = events.borrow();
        assert!(
            events.iter().any(|e| e == "CompositionChanged(false)"),
            "{events:?}"
        );
        assert_eq!(
            events.iter().filter(|e| e.starts_with("Changed")).count(),
            2,
            "{events:?}"
        );
    }

    /// Describe execution range is announced as well as shown.
    #[gpui::test]
    fn describe_announces_its_sentence(cx: &mut TestAppContext) {
        let (e, vcx) = open(cx, "SELECT 1");
        e.update_in(vcx, |v, window, cx| {
            assert!(v.run_command("editor.describe_exec_range", window, cx));
            // The fail-closed service is always uncertain; no provider is installed.
            let said: Vec<String> = v
                .a11y
                .nodes()
                .pending
                .iter()
                .map(ToString::to_string)
                .collect();
            assert_eq!(said, ["hv-editor-describe-uncertain"]);
            assert_eq!(
                super::super::status_message(v).as_deref(),
                Some("hv-editor-describe-uncertain")
            );
        });
    }
}
