//! The platform input handler (UI-EDIT-12 view side; Task 27): UTF-16 ranges through the
//! buffer, marked text through `ImeCore`, `bounds_for_range` from `last_layout()`, the blur
//! rule, and `notify_text_input_changed` only after non-IME edits (R-M3-7).
//!
//! - **Every IME edit goes through `ImeCore`** (`mark`, `commit`, `unmark`) with the GPUI
//!   executor's clock, and its `Committed` through `EditorView::apply_committed`. While a
//!   handler method runs, [`InputState`] flags the call as the IME's own: the platform made
//!   those edits, so it is not told about them. Plain typing shares `UserEvent::InputType`
//!   with command typing, which is why the flag is set at the call site, not read from the
//!   event.
//! - **The platform hears of every other edit** (a command, a paste, an undo, a programmatic
//!   edit, and a composition an app path ended without a `Committed`): `after_commit` and
//!   [`on_composition_changed`] owe a notification, and the next frame's `on_frame` sends it
//!   (`notify_text_input_changed` for text, `notify_text_selection_changed` for a selection
//!   the app moved), once per frame at most. A composition an app path ended (a click in the
//!   text above all) is also discarded on the platform ([`send_owed_discard`], F4): otherwise
//!   the input method keeps it and its candidate window, and inserts it again at the new caret.
//! - **App edits wait for a composition to end** (the corrected R-M3-7 ruling): an app edit
//!   made in the middle of a composition would leave the platform's copy to land beside it
//!   (the discard above ends a composition; it does not let an edit join one). `set_text`, a
//!   "go to error" selection and a paste whose read lands while composing are held
//!   ([`defer_while_composing`]) and applied, in order, once the composition ends; a held
//!   selection is rebased over what the composition committed. While held edits wait, newer
//!   ones queue behind them (never overtaking), and the queue coalesces. Commands
//!   cannot run then: the key dispatcher and the menus run no binding while
//!   `FocusedInput.composing` (F15), and the palette takes the focus first (a focus move,
//!   which commits).
//! - **`FocusedInput { entity, composing }`** is written on focus, on every composition change
//!   and by each frame while focused in the active window (the `TextInput` pattern); the blur
//!   clears it.
//! - **The blur rule:** a focus move inside the active window commits the composition locally
//!   (the platform drops its copy on the focus loss); an app switch leaves it with the
//!   platform, which resumes it on return (`ImeCore::blur`). A composition the platform
//!   dropped without a blur (the focus moved while the window was inactive, or the editor
//!   turned read-only) is ended locally by the next frame.
//! - **Several carets** (## m3c-t31-multi, R-M3-64): typing goes to every caret and a
//!   composition is mirrored to every caret by `ImeCore`, but only when the language service
//!   puts no caret inside a string or a comment; otherwise the other carets collapse before the
//!   first mark and the status bar says so once. The platform sees the main caret only.
//! - **Geometry:** `bounds_for_range` and `character_index_for_point` read the last painted
//!   frame. Between an edit and the next frame that frame is older than the text: a position
//!   before every change since maps through it exactly (the start of a composition being
//!   updated), anything else is estimated on its row from the frame's metrics, and a frame is
//!   requested. A composition scrolled out of view is revealed.

use std::ops::Range;

use hacivat_text::{
    Committed, Origin, RangeKind, Rebased, SelectionRange, SelectionSet, TextBuffer, Transaction,
    UserEvent,
    ime::{bytes_to_utf16, utf16_to_bytes},
};
use hacivat_ui::input::FocusedInput;
use hacivat_ui_core::gpui_surface::{
    Autocapitalize, Bounds, ClipboardItem, Context, EntityInputHandler, Pixels, Point,
    TextInputConfiguration, UTF16Selection, Window, point, px, size,
};

use crate::clipboard::{self, Pasted};
use crate::element::FrameLayout;
use crate::view::EditorView;

/// The input handler's module-owned state (P11).
#[derive(Debug, Default)]
pub struct InputState {
    /// A handler method is running: its edits and composition changes are the platform's own.
    in_ime_call: bool,
    /// A non-IME edit changed the text since the last frame.
    text_owed: bool,
    /// A non-IME edit moved only the selection since the last frame.
    selection_owed: bool,
    /// Notifications sent to the platform (text, selection).
    sent: (u64, u64),
    /// App edits made while composing (or while earlier ones still wait), applied in order.
    deferred: Vec<Held>,
    /// A flush of `deferred` is scheduled.
    flush_scheduled: bool,
    /// `deferred` is being applied: its own edits go through.
    flushing: bool,
    /// Ending a composition the platform already dropped is scheduled (review M4).
    end_scheduled: bool,
    // ## m3a-f4-ime
    /// An app path ended a composition the platform still holds: it is told to drop it.
    discard_owed: bool,
    /// Discards sent to the platform (`Window::discard_marked_text`).
    discards: u64,
    // ## m3c-t31-multi (fix round 1, M8)
    /// The carets the IME fallback collapsed, with the version they are for: a composition
    /// cancelled without text gives them back.
    collapsed: Option<(u64, SelectionSet)>,
}

/// An app edit that must wait for a composition to end.
#[derive(Debug)]
pub(crate) enum AppEdit {
    SetText(String),
    /// Bytes of the text as it is when the edit is made.
    SelectRange(Range<usize>),
    Paste(Pasted),
    // ## m4b-t01b-seams (named line, R-M4b-38): the text read from disk again, held like
    // `SetText` while a composition is open (Task 11's `replace_from_disk` makes it).
    #[allow(
        dead_code,
        reason = "Task 11's replace_from_disk makes it; the stub refuses first"
    )]
    ReplaceFromDisk(String),
}

/// A held app edit.
#[derive(Debug)]
enum Held {
    SetText(String),
    // ## m4b-t01b-seams (R-M4b-38)
    ReplaceFromDisk(String),
    /// `version`: the model version `range` is for, rebased at the flush (review I1); `None`
    /// when a held `set_text` comes first, whose text the range is for.
    Select {
        range: Range<usize>,
        version: Option<u64>,
    },
    Paste(Pasted),
}

impl EditorView {
    /// How many times the editor told the platform its text changed
    /// (`notify_text_input_changed`): once per frame after edits the platform did not make
    /// (R-M3-7). For tests and the IME checklist's instrumentation.
    pub fn text_input_notifications(&self) -> u64 {
        self.input.sent.0
    }

    /// How many times the editor told the platform that the app moved the selection
    /// (`notify_text_selection_changed`).
    pub fn text_selection_notifications(&self) -> u64 {
        self.input.sent.1
    }

    // ## m3a-f4-ime
    /// How many times the editor told the platform to discard its composition
    /// (`Window::discard_marked_text`), counted when the editor is focused. The window
    /// forwards a call only while the editor is also its focused text input (set by a drawn
    /// frame), and returns nothing, so this counts calls made, not calls delivered; the tests
    /// cross-check it with the test window's own count (review M3). Every path that owes a
    /// discard runs on a composing editor, which a drawn frame has made the focused input.
    pub fn marked_text_discards(&self) -> u64 {
        self.input.discards
    }

    /// Ends an open composition as the app: its text is committed as it stands, and the
    /// platform is told at once to drop its copy (the host's toolbar Run, before it runs).
    /// Nothing happens without a composition.
    pub fn end_composition(&mut self, window: &mut Window, cx: &mut Context<EditorView>) {
        if !self.ime.is_composing() {
            return;
        }
        self.ime.unmark(&mut self.model);
        self.composition_changed(false, cx);
        send_owed_discard(self, window);
    }
}

/// App edits must wait now: a composition is open, or held edits have not been applied yet
/// (so an older held edit never lands after a newer direct one, review M3).
pub(crate) fn must_hold(v: &EditorView) -> bool {
    !v.input.flushing && (v.ime.is_composing() || !v.input.deferred.is_empty())
}

/// Holds `edit` when [`must_hold`] (`true`: the caller must not apply it now); it is applied,
/// in order, once the composition has ended. `false` when it can be applied now.
pub(crate) fn defer_while_composing(v: &mut EditorView, edit: AppEdit) -> bool {
    if !must_hold(v) {
        return false;
    }
    hold(v, edit);
    true
}

/// Queues `edit`, coalescing: a `set_text` drops the held texts and selections before it (its
/// text replaces theirs), a selection replaces a trailing selection; pastes are bounded by the
/// reads in flight.
pub(crate) fn hold(v: &mut EditorView, edit: AppEdit) {
    let queue = &mut v.input.deferred;
    let held = match edit {
        AppEdit::SetText(text) => {
            queue.retain(|h| matches!(h, Held::Paste(_)));
            Held::SetText(text)
        }
        AppEdit::SelectRange(range) => {
            if matches!(queue.last(), Some(Held::Select { .. })) {
                queue.pop();
            }
            let after_text = queue.iter().any(|h| matches!(h, Held::SetText(_)));
            Held::Select {
                range,
                version: (!after_text).then(|| v.model.version()),
            }
        }
        AppEdit::Paste(p) => Held::Paste(p),
        // ## m4b-t01b-seams (R-M4b-38): queued in order; Task 11 decides its coalescing.
        AppEdit::ReplaceFromDisk(text) => Held::ReplaceFromDisk(text),
    };
    v.input.deferred.push(held);
}

/// The editor took the focus: it is the focused input, composing or not.
pub(crate) fn on_focus(v: &mut EditorView, window: &mut Window, cx: &mut Context<EditorView>) {
    let _ = window;
    publish_focused(v, cx);
}

/// The editor lost the focus: the blur rule (a focus move inside the active window commits the
/// composition; an app switch keeps it for the platform), and it stops being the focused input.
pub(crate) fn on_blur(v: &mut EditorView, window: &mut Window, cx: &mut Context<EditorView>) {
    // The platform drops its copy on this focus loss: nothing is owed.
    let committed = ime_call(v, |v| v.ime.blur(&mut v.model, window.is_window_active()));
    if committed {
        ime_call(v, |v| v.composition_changed(false, cx));
    }
    let me = Some(cx.entity_id());
    if cx
        .try_global::<FocusedInput>()
        .is_some_and(|f| f.entity == me)
    {
        *cx.default_global::<FocusedInput>() = FocusedInput::default();
    }
}

/// After every `Committed`: an edit the IME did not make owes the platform a notification.
pub(crate) fn after_commit(
    v: &mut EditorView,
    committed: &Committed,
    event: UserEvent,
    cx: &mut Context<EditorView>,
) {
    let _ = (event, cx);
    if v.input.in_ime_call {
        return;
    }
    if committed.buffer_changed {
        v.input.text_owed = true;
    } else {
        v.input.selection_owed = true;
    }
}

/// A composition opened or closed, by whatever path (`EditorView::composition_changed`):
/// `FocusedInput.composing` follows; one that an app path ended (an undo, a click) owes the
/// platform a notification even without a `Committed`; held app edits are applied.
pub(crate) fn on_composition_changed(
    v: &mut EditorView,
    composing: bool,
    cx: &mut Context<EditorView>,
) {
    let me = Some(cx.entity_id());
    if cx
        .try_global::<FocusedInput>()
        .is_some_and(|f| f.entity == me && f.composing != composing)
    {
        cx.default_global::<FocusedInput>().composing = composing;
    }
    if composing {
        return;
    }
    if !v.input.in_ime_call {
        v.input.text_owed = true;
        // ## m3a-f4-ime: the platform did not end it, so it still holds the composition (and
        // its candidate window); it must drop it, or it inserts it again later.
        v.input.discard_owed = true;
    }
    schedule_flush(v, cx);
}

// ## m3a-f4-ime (PO finding IME-1)
/// Tells the platform to drop a composition an app path ended (a click in the text, a command,
/// `end_composition`): `Window::discard_marked_text` (the GPUI fork's carry; on macOS
/// `discardMarkedText`, after the local unmark, as AppKit text views do). Sent by the path
/// itself when it has the window (a click), else by the next frame. A focus loss owes none:
/// the platform discards on it by itself.
pub(crate) fn send_owed_discard(v: &mut EditorView, window: &mut Window) {
    if std::mem::take(&mut v.input.discard_owed) && v.focus.is_focused(window) {
        window.discard_marked_text(&v.focus);
        v.input.discards += 1;
    }
}

/// Once per frame, at the start of the element's prepaint: the focused input is published
/// (focus given before the first frame, a window active again), and what is owed is sent.
pub(crate) fn on_frame(v: &mut EditorView, window: &mut Window, cx: &mut Context<EditorView>) {
    let focused = v.focus.is_focused(window);
    let me = Some(cx.entity_id());
    if focused
        && window.is_window_active()
        && cx
            .try_global::<FocusedInput>()
            .is_none_or(|f| f.entity != me)
    {
        publish_focused(v, cx);
    }
    // ## m3a-f4-ime
    send_owed_discard(v, window);
    if std::mem::take(&mut v.input.text_owed) {
        v.input.selection_owed = false;
        if focused {
            window.notify_text_input_changed(&v.focus);
            v.input.sent.0 += 1;
        }
    }
    if std::mem::take(&mut v.input.selection_owed) && focused {
        window.notify_text_selection_changed(&v.focus);
        v.input.sent.1 += 1;
    }
    // A composition ended by a path that did not report it: the held edits still go.
    if !v.input.deferred.is_empty() && !v.ime.is_composing() {
        schedule_flush(v, cx);
    }
    if v.ime.is_composing() && (v.config.read_only || (!focused && window.is_window_active())) {
        schedule_end_dropped(v, cx);
    }
}

/// Runs `f` as the IME's own call.
fn ime_call<R>(v: &mut EditorView, f: impl FnOnce(&mut EditorView) -> R) -> R {
    let outer = std::mem::replace(&mut v.input.in_ime_call, true);
    let r = f(v);
    v.input.in_ime_call = outer;
    r
}

fn publish_focused(v: &EditorView, cx: &mut Context<EditorView>) {
    let current = FocusedInput {
        entity: Some(cx.entity_id()),
        composing: v.ime.is_composing(),
    };
    if cx.try_global::<FocusedInput>() != Some(&current) {
        *cx.default_global::<FocusedInput>() = current;
    }
}

/// Applies the held app edits after the current effect cycle (never inside the platform's own
/// call that ended the composition).
fn schedule_flush(v: &mut EditorView, cx: &mut Context<EditorView>) {
    if v.input.deferred.is_empty() || v.input.flush_scheduled {
        return;
    }
    v.input.flush_scheduled = true;
    let weak = cx.entity().downgrade();
    cx.defer(move |cx| {
        let _ = weak.update(cx, |v, cx| {
            v.input.flush_scheduled = false;
            flush(v, cx);
        });
    });
}

/// Applies the held app edits in order, unless a new composition opened meanwhile. A held
/// selection is rebased over everything since it was made (the composition's commit above
/// all), and dropped when its text is gone or the log no longer reaches back (review I1).
fn flush(v: &mut EditorView, cx: &mut Context<EditorView>) {
    while !v.ime.is_composing() && !v.input.deferred.is_empty() {
        let held = v.input.deferred.remove(0);
        v.input.flushing = true;
        match held {
            Held::SetText(text) => v.set_text(&text, cx),
            Held::Select { range, version } => {
                let range = match version {
                    None => Some(range),
                    Some(at) => match v.model.changelog().rebase(at, range, RangeKind::Content) {
                        Rebased::Mapped(r) => Some(r),
                        Rebased::Deleted | Rebased::TooOld => None,
                    },
                };
                if let Some(r) = range {
                    v.select_range(r, cx);
                }
            }
            Held::Paste(p) => clipboard::apply(v, p, cx),
            // ## m4b-t01b-seams (R-M4b-38): applied when the composition ends (the stub refuses,
            // and the host's bar stays).
            Held::ReplaceFromDisk(text) => {
                let _ = v.replace_from_disk(&text, cx);
            }
        }
        v.input.flushing = false;
    }
}

/// Ends, as the platform's own, a composition the platform has already dropped (review M4):
/// the editor turned read-only (it accepts no text, so the platform lost its input), or the
/// focus moved elsewhere while the window was inactive (GPUI sent `FocusLost`, but no blur
/// listener ran). Nothing is owed: the platform's copy is gone.
fn schedule_end_dropped(v: &mut EditorView, cx: &mut Context<EditorView>) {
    if v.input.end_scheduled {
        return;
    }
    v.input.end_scheduled = true;
    let weak = cx.entity().downgrade();
    cx.defer(move |cx| {
        let _ = weak.update(cx, |v, cx| {
            v.input.end_scheduled = false;
            if v.ime.is_composing() {
                ime_call(v, |v| {
                    v.ime.unmark(&mut v.model);
                    v.composition_changed(false, cx);
                });
            }
        });
    });
}

// ## m3c-t31-multi
/// R-M3-64: before a composition opens at several carets, when the language service puts any
/// of them inside a string or a comment, the selection collapses to the main range (a
/// selection-only commit, the platform's own call: the main range it knows is unchanged) and
/// the status bar says once (after the mark, [`say_primary_only`]) that typing goes to the main
/// caret only. `true` when it collapsed.
fn primary_only_unless_plain(v: &mut EditorView, cx: &mut Context<EditorView>) -> bool {
    let tok = v.service.token_oracle(v.doc);
    if hacivat_text::carets_in_plain_text(v.model.selection(), tok.as_ref()) {
        return false;
    }
    let main = *v.model.selection().main();
    v.input.collapsed = Some((v.model.version(), v.model.selection().clone()));
    let len = v.model.buffer().len_bytes();
    let ann = v.annotations(UserEvent::Select, Origin::Ime, cx);
    v.commit(
        Transaction {
            changes: hacivat_text::ChangeSet::identity(len),
            selection: Some(SelectionSet::single(main)),
            ann,
        },
        cx,
    );
    true
}

// ## m3c-t31-multi (fix round 1, M8)
/// The carets the fallback collapsed, mapped through what the composition did (nothing, once
/// it is cancelled), become the selection again: the platform's own call, a selection-only
/// commit that keeps the main range it knows.
fn restore_collapsed(v: &mut EditorView, cx: &mut Context<EditorView>) {
    let Some((version, set)) = v.input.collapsed.take() else {
        return;
    };
    let Some(descs) = v.model.changelog().descs_since(version) else {
        return;
    };
    let set = descs.fold(set, |s, d| s.map(d));
    let len = v.model.buffer().len_bytes();
    let ann = v.annotations(UserEvent::Select, Origin::Ime, cx);
    v.commit(
        Transaction {
            changes: hacivat_text::ChangeSet::identity(len),
            selection: Some(set),
            ann,
        },
        cx,
    );
}

/// The fallback's sentence, a polite announcement and the status bar's transient message; it
/// holds while the composition it is about is open (`commands::status_message`).
fn say_primary_only(v: &mut EditorView, cx: &mut Context<EditorView>) {
    let text = crate::commands::message(cx, "hv-editor-ime-primary-only", &[]);
    crate::commands::say_while_composing(v, text, cx);
}

/// The current text's byte range of the platform's UTF-16 range.
fn bytes_of(v: &EditorView, r16: Range<usize>) -> Range<usize> {
    utf16_to_bytes(v.model.buffer(), r16)
}

/// Where byte `at` of the current text is in `layout`'s text: the same offset when every
/// change since the frame began after it (everything before it is unchanged); `None` when a
/// change came before it or the log no longer holds them.
fn in_frame(v: &EditorView, layout: &FrameLayout, at: usize) -> Option<usize> {
    let current = v.model.version();
    if layout.version() == current {
        return Some(at);
    }
    let descs: Vec<_> = v.model.changelog().descs_since(layout.version())?.collect();
    descs
        .iter()
        .all(|d| d.changed_range().is_none_or(|c| at <= c.start))
        .then_some(at)
        .filter(|&at| at <= layout.buffer.len_bytes())
}

/// An estimate of the bounds of byte `at` of `buf` from `layout`'s metrics (its rows, the line
/// start, the monospace advance): the row is exact, x counts UTF-16 units from the line start.
fn estimate(layout: &FrameLayout, buf: &TextBuffer, at: usize) -> Bounds<Pixels> {
    let rows = layout.rows();
    let line = buf.byte_to_line(at);
    let delta = line as f64 - f64::from(rows.first_line);
    let top = f64::from(f32::from(rows.first_y)) + delta * f64::from(f32::from(rows.line_height));
    let col = buf.byte_to_utf16(at) - buf.byte_to_utf16(buf.line_to_byte(line));
    let x = layout.origin_x + col as f64 * f64::from(layout.advance);
    Bounds::new(
        point(px(x as f32), px(top as f32)),
        size(px(layout.advance.max(1.0)), rows.line_height),
    )
}

impl EntityInputHandler for EditorView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let snap = self.model.snapshot();
        Some(self.ime.text_for_range(&snap.buffer, range, adjusted_range))
    }

    /// The main selection in UTF-16, forward, with `reversed` when its head is before its
    /// anchor; `None` on a read-only editor unless the platform ignores disabled input.
    fn selected_text_range(
        &mut self,
        ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        if self.config.read_only && !ignore_disabled_input {
            return None;
        }
        let snap = self.model.snapshot();
        let main = snap.selection.main();
        Some(UTF16Selection {
            range: bytes_to_utf16(&snap.buffer, main.range()),
            reversed: main.head < main.anchor,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.ime.marked_utf16(&self.model)
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        // ## m3c-t31-multi (fix round 1, M8): kept as it stands, so the carets stay collapsed.
        self.input.collapsed = None;
        let was = self.ime.is_composing();
        ime_call(self, |v| {
            v.ime.unmark(&mut v.model);
            if was {
                v.composition_changed(false, cx);
            }
        });
    }

    /// The platform's paste (where it pastes itself rather than through the Paste command):
    /// the same normalisation and guards.
    fn paste(&mut self, item: ClipboardItem, _window: &mut Window, cx: &mut Context<Self>) {
        // A paste asked for while composing is dropped, as the command's is (review M9).
        if self.ime.is_composing() {
            return;
        }
        clipboard::paste_item(self, item, cx);
    }

    /// A commit (plain typing, or the end of a composition) through `ImeCore::commit`.
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.config.read_only {
            return;
        }
        // ## m3a-t36-benches (fix round 1, named exception): B3's probe at handler entry.
        self.mark_input_at_entry(cx);
        // ## m3c-t31-multi (fix round 1, M8): committed text keeps the carets collapsed.
        self.input.collapsed = None;
        let was = self.ime.is_composing();
        let now = cx.background_executor().now();
        ime_call(self, |v| {
            let event = if was {
                UserEvent::InputTypeCompose
            } else {
                UserEvent::InputType
            };
            if let Ok(c) = v.ime.commit(&mut v.model, range, text, now) {
                v.apply_committed(c, event, cx);
            }
            // A vetoed commit inside a composition still ends it.
            if was && !v.ime.is_composing() {
                v.composition_changed(false, cx);
            }
        });
        // ## m3a-t36-benches: a vetoed edit owes the fan-out no mark.
        self.input_handled();
    }

    /// A composition update through `ImeCore::mark`; the first opens the composition, an empty
    /// one cancels it.
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.config.read_only {
            return;
        }
        // ## m3a-t36-benches (fix round 1, named exception): B3's probe at handler entry.
        self.mark_input_at_entry(cx);
        let was = self.ime.is_composing();
        let now = cx.background_executor().now();
        // ## m3c-t31-multi (R-M3-64): a composition opening at several carets is mirrored to
        // all only when every caret is in plain text; otherwise the others collapse first.
        let collapsed = !was
            && self.model.selection().ranges().len() > 1
            && ime_call(self, |v| primary_only_unless_plain(v, cx));
        ime_call(self, |v| {
            let marked = v
                .ime
                .mark(&mut v.model, range, new_text, new_selected_range, now);
            if let Ok(c) = marked {
                v.apply_committed(c, UserEvent::InputTypeCompose, cx);
            }
            if collapsed && v.ime.is_composing() {
                say_primary_only(v, cx);
            }
            // Fix round 1 (M8): a fallback composition cancelled without text gives the
            // collapsed carets back.
            if !v.ime.is_composing() && new_text.is_empty() {
                restore_collapsed(v, cx);
            }
            if !v.ime.is_composing() {
                v.input.collapsed = None;
            }
            let is = v.ime.is_composing();
            if is != was {
                v.composition_changed(is, cx);
            }
        });
        // ## m3a-t36-benches: a vetoed edit owes the fan-out no mark.
        self.input_handled();
    }

    /// The candidate window's anchor: the range on the row of its start, from the last painted
    /// frame (module docs for the path between an edit and the next frame).
    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let bytes = bytes_of(self, range_utf16);
        let layout = self.layout.as_ref()?;
        let exact = in_frame(self, layout, bytes.start).and_then(|start| {
            let end = start + (bytes.end - bytes.start);
            layout.bounds_for_range(start..end)
        });
        let painted = exact.is_some();
        let stale = layout.version() != self.model.version();
        let b = exact.unwrap_or_else(|| estimate(layout, self.model.buffer(), bytes.start));
        let composing = self.ime.is_composing();
        // A composition is revealed; a stale frame is redrawn. Otherwise the next frame could
        // not answer better, so none is asked for (review M5).
        if !painted && (stale || composing) {
            if composing {
                self.scroll.reveal = true;
            }
            cx.notify();
        }
        Some(Bounds::new(
            b.origin,
            size(b.size.width.max(px(1.0)), b.size.height),
        ))
    }

    /// The UTF-16 offset under `point` on the painted text, mapped over edits since the frame.
    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let layout = self.layout.as_ref()?;
        let at = layout.offset_for_point(point)?;
        let at = if layout.version() == self.model.version() {
            at
        } else {
            match self
                .model
                .changelog()
                .rebase(layout.version(), at..at, RangeKind::Content)
            {
                Rebased::Mapped(r) => r.start,
                Rebased::Deleted | Rebased::TooOld => return None,
            }
        };
        Some(self.model.buffer().byte_to_utf16(at))
    }

    /// A selection the platform sets (reconversion, accessibility); ignored while composing,
    /// where the marked text's own selection rules.
    fn set_selected_text_range(
        &mut self,
        range_utf16: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ime.is_composing() {
            return;
        }
        let r = bytes_of(self, range_utf16);
        let len = self.model.buffer().len_bytes();
        let ann = self.annotations(UserEvent::Select, Origin::Ime, cx);
        ime_call(self, |v| {
            v.commit(
                Transaction {
                    changes: hacivat_text::ChangeSet::identity(len),
                    selection: Some(SelectionSet::single(SelectionRange::new(r.start, r.end))),
                    ann,
                },
                cx,
            );
        });
    }

    fn text_length_utf16(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.model.buffer().len_utf16())
    }

    fn accepts_text_input(&self, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        !self.config.read_only
    }

    /// SQL is code: no autocorrect, autocapitalisation or suggestions.
    fn text_input_configuration(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> TextInputConfiguration {
        TextInputConfiguration {
            autocorrect: false,
            autocapitalize: Autocapitalize::None,
            suggestions: false,
            ..TextInputConfiguration::default()
        }
    }
}
