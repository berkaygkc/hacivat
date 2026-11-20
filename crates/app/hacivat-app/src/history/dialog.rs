//! The history dialog's layer (M3b Task 27): the search field, the filters and the list, pushed
//! with `Workspace::push_layer` (the `open_dialog` pattern).
//!
//! ## m3b-t01b-seams: the module line (Task 1b). Task 27 fills it.
//!
//! ## m3b-t27-history
//!
//! `design/m3b-safe` surface 11, a 640 px dialog:
//!
//! - **Header** "Query history" and the entry count.
//! - **Bar:** the search field (Turkish-tolerant: the store folds `İ`, `I`, `ı` to `i` and drops
//!   accents with its fixed table, so `musteri` finds `MÜŞTERİLER`; debounced 150 ms, run on
//!   the core runtime), the scope ("This connection" / "All connections", two segments of their
//!   own so both are tab stops) and the outcome select.
//! - **List** (a field well, at most 250 px, scrolls): day groups and rows (`rows`). Hits are
//!   shown newest first (the store ranks text matches best first; the dialog orders by time so
//!   the day groups hold).
//! - **Preview:** the meta line (environment mark, connection, ISO time, outcome, duration,
//!   rows, run count, the "redacted" chip) with Pin and Open in new console; the masked SQL in
//!   the SQL inset; the redacted line; the target line when the entry ran on another
//!   connection than the focused console's (NOTES ruling 42); the server-message box of a
//!   failure (the store's scrubbed text, verbatim in its own field).
//! - **Footer:** the key hints; Copy SQL, Insert and run (Default), Insert (Primary).
//! - **Empty** ("No history yet…") and **no match** ("Nothing matches “q”.").
//!
//! **Keys.** The search field keeps the focus; ↑/↓ move the selection; ↵ inserts at the caret
//! (`exec_ui::insert_at_caret`: one undo group, refused while a composition is open);
//! ⌘↵ / Ctrl+↵ inserts, selects what it inserted and runs it with the editor's own Run
//! (`editor.run` → `ExecuteRequested` → the run orchestrator), so it passes the same gate as
//! any Run. Escape, ⌘W / Ctrl+W and the close button close. Enter acts on key down only, never
//! held, and never while the field composes. Tab cycles inside the dialog.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use hacivat_exec::{HistoryConfig, HistoryHit, HistoryQuery, HistoryRecord, HistoryStore, Outcome};
use hacivat_l10n::ArgValue;
use hacivat_proto::ProfileId;
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::capsule::env_swatch;
use hacivat_ui::dialog::DialogSize;
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::input::{InputEvent, TextInput};
use hacivat_ui::popover::{A11yNode, Declare as _, Tip};
use hacivat_ui::select::{Select, SelectEvent, SelectOption};
use hacivat_ui::text::{Label, TextFn, TextSource, paragraph, scroll_viewport};
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AppContext as _, ClickEvent, ClipboardItem, Context, Div, Entity,
    EntityInputHandler as _, FluentBuilder as _, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, Role, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div, px,
};
// ## m4-t32-shots (named exception; PO R-W5-14 b)
use hacivat_ui_core::gpui_surface::{Bounds, Pixels, canvas};
use hacivat_ui_core::keymap::{KeymapPlatform, display_chord, parse_chord};
use hacivat_ui_core::theme::{Theme, ThemeMode, focus_outline};

use super::rows::{self, Conn, NAME_MAX_CHARS};
use crate::connections::ConnectionsModel;
use crate::connections::error_view::env_kind;
use crate::connections::prompt_kit::{
    card, cycle_focus, footer, header, inset, meta_line, push_modal, remove_modal, trapped_tab,
    visible,
};
use crate::console::{ConsoleId, Consoles};
use crate::exec_ui::confirm::{closes_dialog, console_facts, scroll_body, sql_inset};
use crate::exec_ui::marks::{duration_text, message};
use crate::shell::Workspace;

/// The dialog's layer ID.
pub const HISTORY_LAYER_ID: &str = "query-history";
/// The search waits this long after the last keystroke (NOTES surface 11).
pub const DEBOUNCE: Duration = Duration::from_millis(150);
/// The list's most height (NOTES "History row" well).
const LIST_MAX_PX: f32 = 250.0;
/// The most characters of a stored error message shown.
const SERVER_TEXT_MAX_CHARS: usize = 4096;
/// The most hits one search shows (the store's default).
const SEARCH_LIMIT: u32 = 200;
/// The most characters of the search text a no-match sentence quotes.
const QUERY_MAX_CHARS: usize = 80;
/// How often, and how many times, Open in new console looks for the new console's editor (the
/// host opens it once the tab has drawn and its file was read on the core runtime).
const NEW_CONSOLE_POLL: Duration = Duration::from_millis(25);
const NEW_CONSOLE_TRIES: u32 = 400;

/// The outcome filter's choices, in the select's order.
const OUTCOMES: [Option<Outcome>; 4] = [
    None,
    Some(Outcome::Ok),
    Some(Outcome::Failed),
    Some(Outcome::Cancelled),
];

// ## m3b-t27-history (fix round 1, I1 and m4)

/// What the masks of the run service (`mask_secrets`: `'***'`, `/* *** */`, `-- ***`) and of
/// the store (`'***'`, `***`) all contain.
const MASK: &str = "***";

/// What Insert appends to a statement the store cut at its size cap, so the cut text never runs
/// as if it were whole.
pub const TRUNCATION_MARK: &str = "\n…";

/// Whether `r` holds masked values: the store or the run service took secrets (or every
/// literal, `mask_secrets`' over-masking) out of it. Such a text never ran as it reads.
pub fn masked(r: &HistoryRecord) -> bool {
    r.redacted || r.statement_masked.contains(MASK)
}

/// Whether the store cut `r`'s statement at its size cap (a prefix ending on a character
/// boundary, so within 3 bytes of the cap; a statement that long that was not cut counts too:
/// fail-closed).
pub fn truncated(r: &HistoryRecord) -> bool {
    r.statement_masked.len() + 3 >= HistoryConfig::default().statement_cap
}

/// Whether `r` may be inserted and run as it was saved (review I1, m4): neither masked nor cut.
/// Insert stays available either way.
pub fn runnable(r: &HistoryRecord) -> bool {
    !masked(r) && !truncated(r)
}

/// The text Insert puts in the editor: the stored statement, a cut one with
/// [`TRUNCATION_MARK`].
fn insert_text(r: &HistoryRecord) -> String {
    let mut text = r.statement_masked.to_string();
    if truncated(r) {
        text.push_str(TRUNCATION_MARK);
    }
    text
}

/// Says why an action did nothing (review m3): a toast.
fn refuse(id: &'static str, cx: &mut App) {
    ToastQueue::push(
        cx,
        Toast::new(ToastKind::Error, crate::l10n_global::text_fn_id(id)),
    );
}

// ## m3b-t27-history (fix round 1, I2)
/// Puts console `console`'s tab in front in `window` (its pane's active tab, that pane
/// focused): History acts only on a console the user sees. `false` when the console or its tab
/// is gone.
fn bring_forward(console: ConsoleId, window: &mut Window, cx: &mut App) -> bool {
    let Some(tab) = Consoles::get(cx, console).map(|s| s.tab) else {
        return false;
    };
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return false;
    };
    if ws.read(cx).active_tab() == Some(tab) {
        return true;
    }
    ws.update(cx, |ws, cx| ws.focus_tab(tab, cx))
}

/// Where the list stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListState {
    /// The first search has not answered.
    Loading,
    Ready,
    /// The store could not answer (closed, damaged).
    Failed,
}

/// The history dialog (module docs).
pub struct HistoryDialog {
    console: ConsoleId,
    /// The focused console's connection: the "This connection" scope and the target line.
    here: Option<ProfileId>,
    conns: HashMap<ProfileId, Conn>,
    store: Option<Arc<HistoryStore>>,
    input: Entity<TextInput>,
    outcome: Entity<Select>,
    /// "All connections" is pressed.
    all: bool,
    hits: Vec<HistoryHit>,
    state: ListState,
    selected: usize,
    /// The search text the shown hits answer (the no-match sentence quotes it).
    answered: String,
    /// The latest search: an older answer is dropped.
    seq: u64,
    /// The search whose answer is shown.
    shown_seq: u64,
    /// Searches superseded before they answered (tests).
    stale: u64,
    /// "Today" is the day of this instant (Unix milliseconds).
    now_ms: i64,
    /// The selection moved: the list scrolls it into view at the next frame.
    reveal: bool,
    list: ScrollHandle,
    root: FocusHandle,
    seg_this: FocusHandle,
    seg_all: FocusHandle,
    pin: FocusHandle,
    open_new: FocusHandle,
    copy: FocusHandle,
    insert_run: FocusHandle,
    insert: FocusHandle,
    // ## m4-t32-shots (named exception; PO R-W5-14 b): the ⋯ button.
    more: FocusHandle,
    /// Where the ⋯ button was last drawn (its menu opens under it).
    more_bounds: Rc<std::cell::Cell<Option<Bounds<Pixels>>>>,
    debounce: Option<Task<()>>,
    searching: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for HistoryDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root.clone()
    }
}

/// Opens the dialog over console `console` in `window`'s workspace and runs the first search.
/// An open history dialog takes the focus instead.
pub fn open(console: ConsoleId, window: &mut Window, cx: &mut App) {
    if let Some(open) = cx
        .try_global::<Open>()
        .and_then(|o| o.0.as_ref())
        .and_then(|v| v.upgrade())
    {
        let input = open.read(cx).input.read(cx).focus_handle(cx);
        window.focus(&input, cx);
        return;
    }
    // Fix round 1 (I2): the console Insert goes into is the one in front.
    if !bring_forward(console, window, cx) {
        return;
    }
    let store = super::store(cx);
    let view = cx.new(|cx| HistoryDialog::new(console, store, window, cx));
    if show(&view, window, cx) {
        view.update(cx, |d, cx| d.search_now(cx));
    }
}

/// Pushes `view` and remembers it as the open dialog.
fn show(view: &Entity<HistoryDialog>, window: &mut Window, cx: &mut App) -> bool {
    let input = view.read(cx).input.read(cx).focus_handle(cx);
    if !push_modal(HISTORY_LAYER_ID, view.clone().into(), &input, window, cx) {
        return false;
    }
    cx.set_global(Open(Some(view.downgrade())));
    true
}

// ## m4-t32-shots (named exception; PO R-W5-14 b)
/// The profile the open dialog's ⋯ menu clears (`history.clear`).
pub(super) fn open_clear_target(cx: &App) -> Option<ProfileId> {
    let view = cx.try_global::<Open>()?.0.as_ref()?.upgrade()?;
    view.read(cx).clear_target()
}

/// Searches the open dialog again (after its connection's history was cleared).
pub(super) fn search_again(cx: &mut App) {
    if let Some(view) = cx
        .try_global::<Open>()
        .and_then(|o| o.0.as_ref())
        .and_then(|v| v.upgrade())
    {
        view.update(cx, |d, cx| d.search_now(cx));
    }
}

/// The open dialog (one at a time).
#[derive(Default)]
struct Open(Option<hacivat_ui_core::gpui_surface::WeakEntity<HistoryDialog>>);

impl hacivat_ui_core::gpui_surface::Global for Open {}

/// A Fluent label for a select option.
fn option(id: &'static str) -> SelectOption {
    let label: TextFn = Rc::new(move |cx: &App| message(cx, id, vec![]));
    SelectOption {
        label,
        hint: None,
        disabled: false,
    }
}

impl HistoryDialog {
    fn new(
        console: ConsoleId,
        store: Option<Arc<HistoryStore>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let _ = window;
        let here = Consoles::get(cx, console).and_then(|s| s.profile);
        let conns = cx
            .try_global::<ConnectionsModel>()
            .map(|m| {
                m.profiles()
                    .iter()
                    .map(|p| {
                        (
                            p.id,
                            Conn {
                                name: Arc::from(p.name.as_str()),
                                env: p.environment,
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let placeholder: TextFn = Rc::new(|cx: &App| message(cx, "hv-history-search", vec![]));
        let input = cx.new(|cx| {
            let mut input = TextInput::new(cx, placeholder);
            input.set_well(true);
            input
        });
        let outcome = cx.new(|cx| {
            Select::new(
                vec![
                    option("hv-history-outcome-any"),
                    option("hv-history-outcome-ok"),
                    option("hv-history-outcome-failed"),
                    option("hv-history-outcome-cancelled"),
                ],
                0,
                cx,
            )
        });
        let subscriptions = vec![
            cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
                InputEvent::Changed => this.schedule(cx),
                // Escape in the field closes the dialog (the modal-layer rule).
                InputEvent::Cancelled => this.close(window, cx),
                InputEvent::Submitted | InputEvent::CompositionChanged(_) => {}
            }),
            cx.subscribe(&outcome, |this, _, event: &SelectEvent, cx| {
                let SelectEvent::Changed(_) = event;
                this.search_now(cx);
            }),
        ];
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
        HistoryDialog {
            console,
            here,
            conns,
            store,
            input,
            outcome,
            all: here.is_none(),
            hits: Vec::new(),
            state: ListState::Loading,
            selected: 0,
            answered: String::new(),
            seq: 0,
            shown_seq: 0,
            stale: 0,
            now_ms,
            reveal: false,
            list: ScrollHandle::new(),
            root: cx.focus_handle(),
            seg_this: cx.focus_handle().tab_stop(true),
            seg_all: cx.focus_handle().tab_stop(true),
            pin: cx.focus_handle().tab_stop(true),
            open_new: cx.focus_handle().tab_stop(true),
            copy: cx.focus_handle().tab_stop(true),
            insert_run: cx.focus_handle().tab_stop(true),
            insert: cx.focus_handle().tab_stop(true),
            // ## m4-t32-shots (named exception; PO R-W5-14 b)
            more: cx.focus_handle().tab_stop(true),
            more_bounds: Rc::default(),
            debounce: None,
            searching: None,
            _subscriptions: subscriptions,
        }
    }

    // ---- searching ---------------------------------------------------------------------------

    /// Searches again [`DEBOUNCE`] after the last keystroke (a newer keystroke drops the wait).
    fn schedule(&mut self, cx: &mut Context<Self>) {
        self.debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            let _ = this.update(cx, |d, cx| d.search_now(cx));
        }));
    }

    /// The filters as a store query.
    fn query(&self, cx: &App) -> HistoryQuery {
        let text = self.input.read(cx).value().to_string();
        HistoryQuery {
            text: (!text.trim().is_empty()).then_some(text),
            profile: if self.all { None } else { self.here },
            outcome: OUTCOMES
                .get(self.outcome.read(cx).selected())
                .copied()
                .flatten(),
            limit: SEARCH_LIMIT,
            ..HistoryQuery::default()
        }
    }

    /// Searches now, on the core runtime; the answer lands in [`HistoryDialog::landed`].
    fn search_now(&mut self, cx: &mut Context<Self>) {
        self.debounce = None;
        let Some(store) = self.store.clone() else {
            return;
        };
        // An older search still unanswered is superseded: its task is dropped with
        // `searching`, and an answer already on its way is refused by the `seq` check.
        if self.shown_seq < self.seq {
            self.stale += 1;
        }
        self.seq += 1;
        let seq = self.seq;
        let q = self.query(cx);
        let text = q.text.clone().unwrap_or_default();
        let work = super::search(&store, q, cx);
        self.searching = Some(cx.spawn(async move |this, cx| {
            let found = work.await;
            let _ = this.update(cx, |d, cx| {
                if seq != d.seq {
                    return;
                }
                d.shown_seq = seq;
                match found {
                    Ok(hits) => d.landed(hits, text, cx),
                    Err(_) => {
                        d.hits.clear();
                        d.state = ListState::Failed;
                        cx.notify();
                    }
                }
            });
        }));
    }

    /// Shows `hits` (the answer to `text`), newest first, keeping the selected entry when it is
    /// still there.
    fn landed(&mut self, mut hits: Vec<HistoryHit>, text: String, cx: &mut Context<Self>) {
        let kept = self.hits.get(self.selected).map(|h| h.id);
        hits.sort_by_key(|h| std::cmp::Reverse(h.record.ts_start_ms));
        self.selected = kept
            .and_then(|id| hits.iter().position(|h| h.id == id))
            .unwrap_or(0);
        self.hits = hits;
        self.answered = text;
        self.state = ListState::Ready;
        self.reveal = true;
        cx.notify();
    }

    fn set_scope(&mut self, all: bool, cx: &mut Context<Self>) {
        // Without a connection on the focused console only "All connections" means anything.
        let all = all || self.here.is_none();
        if self.all != all {
            self.all = all;
            self.search_now(cx);
        }
        cx.notify();
    }

    fn step(&mut self, down: bool, cx: &mut Context<Self>) {
        if self.hits.is_empty() {
            return;
        }
        self.selected = if down {
            (self.selected + 1).min(self.hits.len() - 1)
        } else {
            self.selected.saturating_sub(1)
        };
        self.reveal = true;
        cx.notify();
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.hits.len() {
            self.selected = index;
            cx.notify();
        }
    }

    fn selected_hit(&self) -> Option<&HistoryHit> {
        self.hits.get(self.selected)
    }

    // ## m3b-t29-shots (named exception; the T27 review's m6)
    /// A polite live node naming the selected entry (its first SQL line, data) and its place
    /// in the list in words ("…, 3 of 5", `hv-history-selection-a11y`): the search field keeps
    /// the focus while ↑/↓ move the selection, so the move is announced from here, even between
    /// two entries with the same first line. It also speaks when a search answer changes the
    /// selected entry. Fix round 1 (T29 review I3): it sits in the dialog card, outside the
    /// list box (a list box owns only its options), absolutely placed and of no size.
    fn selection_live(&self, cx: &App) -> Option<impl IntoElement> {
        let hit = self.selected_hit()?;
        let sql = rows::first_line(&hit.record.statement_masked);
        let name = message(
            cx,
            "hv-history-selection-a11y",
            vec![
                ("sql", ArgValue::Text(sql)),
                ("pos", ArgValue::Count((self.selected + 1) as u64)),
                ("n", ArgValue::Count(self.hits.len() as u64)),
            ],
        );
        Some(
            div().absolute().top_0().left_0().size_0().child(
                A11yNode::new("history-selection-live", Role::Status, name, false, div())
                    .live(hacivat_ui_core::gpui_surface::accesskit::Live::Polite)
                    .position(self.selected + 1, self.hits.len()),
            ),
        )
    }

    // ---- actions -------------------------------------------------------------------------------

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity_id();
        window.defer(cx, move |window, cx| remove_modal(view, window, cx));
    }

    /// Insert (`run`: Insert and run). The statement replaces the focused console's selection
    /// as one undo group; a refused insert (a composition is open, a read-only editor) keeps the
    /// dialog. Insert and run then selects what it inserted and runs it with the editor's own
    /// Run, after the dialog has gone: the same orchestrator, the same gate as any Run.
    fn insert(&mut self, run: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(record) = self.selected_hit().map(|h| h.record.clone()) else {
            return;
        };
        // Fix round 1 (I1, m4): a masked or cut text never runs; the preview's line says why.
        if run && !runnable(&record) {
            return;
        }
        let console = self.console;
        // Fix round 1 (I2): never into a console that is not in front.
        if !bring_forward(console, window, cx) {
            refuse("hv-history-console-gone", cx);
            return;
        }
        let Some(range) = crate::exec_ui::inserted_at_caret(console, &insert_text(&record), cx)
        else {
            refuse("hv-history-insert-refused", cx);
            return;
        };
        self.close(window, cx);
        let Some(editor) = Consoles::get(cx, console).and_then(|s| s.editor.clone()) else {
            return;
        };
        // After the layer is gone (`close` deferred first), so the focus goes back to the editor
        // and a chooser Run opens sits on top.
        window.defer(cx, move |window, cx| {
            let focus = editor.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            if run {
                editor.update(cx, |v, cx| {
                    v.select_range(range, cx);
                    v.run_command("editor.run", window, cx);
                });
            }
        });
    }

    fn copy_sql(&mut self, cx: &mut Context<Self>) {
        if let Some(hit) = self.selected_hit() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                hit.record.statement_masked.to_string(),
            ));
        }
    }

    /// Pins or unpins the selected entry (a pinned entry outlives retention), then searches
    /// again.
    fn toggle_pin(&mut self, cx: &mut Context<Self>) {
        let (Some(store), Some(hit)) = (self.store.clone(), self.selected_hit()) else {
            return;
        };
        let work = super::pin(&store, hit.id, !hit.pinned, cx);
        cx.spawn(async move |this, cx| {
            if work.await.is_ok() {
                let _ = this.update(cx, |d, cx| d.search_now(cx));
            } else {
                cx.update(|cx| refuse("hv-history-pin-failed", cx));
            }
        })
        .detach();
    }

    /// Opens a new console on the entry's connection and inserts the statement there, once the
    /// console's editor is open.
    fn open_in_new_console(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hit) = self.selected_hit() else {
            return;
        };
        let profile = hit.record.profile;
        if !self.conns.contains_key(&profile) {
            return;
        }
        let sql = insert_text(&hit.record);
        self.close(window, cx);
        window.defer(cx, move |window, cx| {
            let Some(ws) = window.root::<Workspace>().flatten() else {
                return;
            };
            let Some(tab) = ws.update(cx, |ws, cx| ws.open_console_tab(Some(profile), window, cx))
            else {
                return;
            };
            let handle = window.window_handle();
            cx.spawn(async move |cx| {
                for _ in 0..NEW_CONSOLE_TRIES {
                    cx.background_executor().timer(NEW_CONSOLE_POLL).await;
                    let done = cx.update(|cx| {
                        let Some(console) = Consoles::by_tab(cx, tab) else {
                            return false;
                        };
                        let Some(editor) =
                            Consoles::get(cx, console).and_then(|s| s.editor.clone())
                        else {
                            return false;
                        };
                        if crate::exec_ui::inserted_at_caret(console, &sql, cx).is_none() {
                            return false;
                        }
                        let _ = handle.update(cx, |_, window, cx| {
                            let focus = editor.read(cx).focus_handle(cx);
                            window.focus(&focus, cx);
                        });
                        true
                    });
                    if done {
                        return;
                    }
                }
                cx.update(|cx| refuse("hv-history-open-failed", cx));
            })
            .detach();
        });
    }

    // ---- keys ------------------------------------------------------------------------------------

    /// The focus order, as drawn (Tab and Shift-Tab cycle it).
    fn order(&self, cx: &App) -> Vec<FocusHandle> {
        let mut order = vec![
            self.input.read(cx).focus_handle(cx),
            if self.all {
                self.seg_all.clone()
            } else {
                self.seg_this.clone()
            },
            self.outcome.read(cx).focus_handle(cx),
            // ## m4-t32-shots (named exception; PO R-W5-14 b): the ⋯ button ends the bar.
            self.more.clone(),
        ];
        if self.selected_hit().is_some() {
            order.extend([self.pin.clone(), self.open_new.clone()]);
            order.extend([
                self.copy.clone(),
                self.insert_run.clone(),
                self.insert.clone(),
            ]);
        }
        order
    }

    /// Keys the search field must not see: ↑/↓ move the selection, ↵ inserts and ⌘↵ / Ctrl+↵
    /// inserts and runs, while the field has the focus and is not composing.
    fn on_capture_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.input.read(cx);
        if !input.focus_handle(cx).is_focused(window) || input.is_composing() {
            return;
        }
        let stroke = &event.keystroke;
        let m = &stroke.modifiers;
        match stroke.key.as_str() {
            "up" | "down" if !m.modified() => self.step(stroke.key == "down", cx),
            "enter" if event.is_held => {}
            "enter" if !m.modified() => self.insert(false, window, cx),
            "enter" if m.secondary(cx) && !m.shift && !m.alt => self.insert(true, window, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        let m = &stroke.modifiers;
        // Fix round 1 (m2, the T21 m7 rule): a composing field lets its keys bubble here where
        // the platform did not give them to the IME: Escape ends the composition (its text
        // stays, the dialog stays); Enter, ⌘↵ and Ctrl+↵ do nothing.
        if self.input.read(cx).is_composing() && matches!(stroke.key.as_str(), "escape" | "enter") {
            if stroke.key == "escape" {
                self.input.update(cx, |i, cx| i.unmark_text(window, cx));
            }
            cx.notify();
            return cx.stop_propagation();
        }
        match stroke.key.as_str() {
            "escape" => self.close(window, cx),
            "w" if closes_dialog(stroke, cx) => self.close(window, cx),
            "enter" if event.is_held => {}
            "enter" if m.secondary(cx) && !m.shift && !m.alt => self.insert(true, window, cx),
            "enter" | "space" if !m.modified() => {
                if self.seg_this.is_focused(window) {
                    self.set_scope(false, cx);
                } else if self.seg_all.is_focused(window) {
                    self.set_scope(true, cx);
                } else if stroke.key == "space" {
                    return;
                } else if self.copy.is_focused(window) {
                    self.copy_sql(cx);
                } else if self.insert_run.is_focused(window) {
                    self.insert(true, window, cx);
                } else if self.insert.is_focused(window) {
                    self.insert(false, window, cx);
                } else if self.pin.is_focused(window) {
                    self.toggle_pin(cx);
                } else if self.open_new.is_focused(window) {
                    self.open_in_new_console(window, cx);
                // ## m4-t32-shots (named exception; PO R-W5-14 b)
                } else if self.more.is_focused(window) {
                    self.open_more(window, cx);
                } else {
                    return;
                }
            }
            "left" | "right"
                if self.seg_this.is_focused(window) || self.seg_all.is_focused(window) =>
            {
                let all = !self.all;
                self.set_scope(all, cx);
                let focus = if self.all {
                    &self.seg_all
                } else {
                    &self.seg_this
                };
                window.focus(focus, cx);
            }
            _ if trapped_tab(event) => {
                cycle_focus(&self.order(cx), m.shift, window, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    // ---- drawing ---------------------------------------------------------------------------------

    fn conn(&self, profile: ProfileId) -> Option<&Conn> {
        self.conns.get(&profile)
    }

    /// One scope segment (`all`: "All connections").
    fn segment(&self, all: bool, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let pressed = self.all == all;
        let (id, label_id, handle) = if all {
            ("history-scope-all", "hv-history-filter-all", &self.seg_all)
        } else {
            (
                "history-scope-this",
                "hv-history-filter-this",
                &self.seg_this,
            )
        };
        let label = message(cx, label_id, vec![]);
        let disabled = !all && self.here.is_none();
        let ink = if disabled {
            c.text_3
        } else if pressed {
            c.text
        } else {
            c.text_2
        };
        let mut style = t.type_ramp.body;
        if pressed {
            style.weight = 600;
        }
        let pressed_fill = match theme.mode {
            ThemeMode::Dark => c.raised,
            ThemeMode::Light => c.panel,
        };
        let seg = div()
            .id(id)
            .debug_selector(move || id.to_string())
            .declare(Role::RadioButton, label.clone(), Some(pressed))
            .track_focus(handle)
            .flex()
            .flex_row()
            .items_center()
            .h_full()
            .px(px(9.0))
            .when(pressed, |el| el.bg(pressed_fill.to_hsla()))
            .when(!disabled, |el| {
                el.cursor_pointer()
                    .on_click(cx.listener(move |d, _: &ClickEvent, _, cx| d.set_scope(all, cx)))
            })
            .child(Label::new(label, TextSource::Fluent, style, ink.to_hsla()).id(id));
        focus_outline(seg, &theme, handle.is_focused(window))
    }

    fn scope(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let well = match theme.mode {
            ThemeMode::Dark => c.panel,
            ThemeMode::Light => c.ground,
        };
        div()
            .id("history-scope")
            .debug_selector(|| "history-scope".to_string())
            .role(Role::RadioGroup)
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(px(t.spacing.field_height))
            .border_1()
            .border_color(c.seam_strong.to_hsla())
            .rounded(px(t.radii.sm))
            .overflow_hidden()
            .bg(well.to_hsla())
            .child(self.segment(false, window, cx))
            .child(div().w(px(1.0)).h_full().bg(c.seam_strong.to_hsla()))
            .child(self.segment(true, window, cx))
    }

    fn bar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let tip = message(cx, "hv-history-search-tip", vec![]);
        let search = div()
            .id("history-search")
            .debug_selector(|| "history-search".to_string())
            .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
            .flex()
            .flex_row()
            .items_center()
            .flex_1()
            .min_w(px(160.0))
            .gap(px(t.spacing.sm))
            .child(
                // Raised Ink Rule: text-2 beside the field on the raised dialog.
                Icon::new(IconName::Search)
                    .size(IconSize::Chrome)
                    .color(t.colors.text_2.to_hsla()),
            )
            .child(div().flex_1().min_w_0().child(self.input.clone()));
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(t.spacing.md))
            .child(search)
            .child(self.scope(window, cx))
            .child(div().flex_none().w(px(150.0)).child(self.outcome.clone()))
            // ## m4-t32-shots (named exception; PO R-W5-14 b): the ⋯ menu ("Clear history of
            // this connection…").
            .child({
                let cell = self.more_bounds.clone();
                div()
                    .relative()
                    .debug_selector(|| "history-more".to_string())
                    .child(
                        Button::new("history-more", message(cx, "hv-history-more", vec![]))
                            .kind(ButtonKind::Icon)
                            .icon(IconName::Ellipsis)
                            .on_raised(true)
                            .focus_handle(self.more.clone())
                            .on_click(cx.listener(|d, _: &ClickEvent, window, cx| {
                                d.open_more(window, cx);
                            })),
                    )
                    .child(
                        canvas(move |b, _, _| cell.set(Some(b)), |_, _, _, _| {})
                            .absolute()
                            .size_full(),
                    )
            })
    }

    // ## m4-t32-shots (named exception; PO R-W5-14 b)
    /// The profile whose history the ⋯ menu clears: the console's connection.
    pub(super) fn clear_target(&self) -> Option<ProfileId> {
        self.here
    }

    /// Opens the ⋯ menu under its button.
    fn open_more(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let anchor = self
            .more_bounds
            .get()
            .map_or_else(|| window.mouse_position(), |b| b.bottom_left());
        let items = vec![
            hacivat_ui::menu::MenuItemSpec::new(
                Rc::new(|cx: &App| message(cx, "hv-history-clear-item", vec![])),
                "history.clear",
            )
            .disabled(!super::clear::can_clear(self.here, cx))
            .danger(true),
        ];
        let Some(ws) = window.root::<Workspace>().flatten() else {
            return;
        };
        ws.update(cx, |ws, cx| {
            ws.open_popup_menu(anchor, items, window, cx);
        });
    }

    fn list(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let widths = rows::widths(&self.hits, &theme, window, cx);
        let t = theme.tokens;
        let c = &t.colors;
        let fill = match theme.mode {
            ThemeMode::Dark => c.panel,
            ThemeMode::Light => c.ground,
        };
        let mut children: Vec<AnyElement> = vec![scroll_viewport().into_any_element()];
        if self.hits.is_empty() && self.state != ListState::Loading {
            // Fix round 1 (m9): each state's sentence has its own ID.
            let (text, id) = match self.state {
                ListState::Failed => (message(cx, "hv-history-failed", vec![]), "history-failed"),
                _ if self.answered.trim().is_empty() => {
                    (message(cx, "hv-history-empty", vec![]), "history-empty")
                }
                _ => (
                    message(
                        cx,
                        "hv-history-nomatch",
                        vec![(
                            "q",
                            ArgValue::Text(visible(self.answered.trim(), QUERY_MAX_CHARS)),
                        )],
                    ),
                    "history-nomatch",
                ),
            };
            // The well restores text-3 on its own fill (NOTES cross-cutting #8).
            children.push(
                div()
                    .w_full()
                    .px(px(t.spacing.xl))
                    .py(px(18.0))
                    .child(
                        paragraph(
                            text,
                            TextSource::Fluent,
                            t.type_ramp.body_sm,
                            c.text_3.to_hsla(),
                        )
                        .centered()
                        .id(id),
                    )
                    .into_any_element(),
            );
        }
        let mut day: Option<SharedString> = None;
        let mut selected_child = None;
        for (i, hit) in self.hits.iter().enumerate() {
            let this_day = rows::day_text(cx, hit.record.ts_start_ms);
            if day.as_ref() != Some(&this_day) {
                let title = rows::group_title(cx, &this_day, self.now_ms);
                children.push(rows::group_header(&theme, title).into_any_element());
                day = Some(this_day);
            }
            let selected = i == self.selected;
            if selected {
                selected_child = Some(children.len());
            }
            children.push(
                rows::row(
                    &theme,
                    // ## m3b-t29-shots (named exception)
                    (i, self.hits.len()),
                    hit,
                    self.conn(hit.record.profile),
                    selected,
                    widths,
                    cx,
                )
                .on_click(cx.listener(move |d, _: &ClickEvent, _, cx| d.select(i, cx)))
                .into_any_element(),
            );
        }
        if std::mem::take(&mut self.reveal)
            && let Some(child) = selected_child
        {
            self.list.scroll_to_item(child);
        }
        let name = message(cx, "hv-history-title", vec![]);
        div()
            .id("history-list")
            .debug_selector(|| "history-list".to_string())
            .role(Role::ListBox)
            .aria_label(name)
            .track_scroll(&self.list)
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .flex_none()
            .min_h(px(72.0))
            .max_h(px(LIST_MAX_PX))
            .py(px(t.spacing.xs))
            .border_1()
            .border_color(c.seam_strong.to_hsla())
            .rounded(px(t.radii.sm))
            .bg(fill.to_hsla())
            .children(children)
    }

    fn preview(&self, cx: &mut Context<Self>) -> Option<Div> {
        let hit = self.selected_hit()?;
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let r = &hit.record;
        let ink2 = c.text_2.to_hsla();
        let conn = self.conn(r.profile);
        let mono = |text: SharedString, id: &'static str| {
            Label::new(
                text,
                TextSource::Data,
                rows::mono11(&theme),
                c.text.to_hsla(),
            )
            .id(id)
        };
        let mut meta = meta_line(&theme)
            .debug_selector(|| "history-meta".to_string())
            .children(
                conn.and_then(|k| k.env)
                    .map(|e| env_swatch(env_kind(e), &theme)),
            )
            .child(match conn {
                Some(k) => mono(visible(&k.name, NAME_MAX_CHARS).into(), "history-meta-conn"),
                None => Label::new(
                    message(cx, "hv-history-conn-gone", vec![]),
                    TextSource::Fluent,
                    t.type_ramp.caption,
                    ink2,
                )
                .id("history-meta-conn"),
            })
            .child(mono(
                rows::iso_utc(r.ts_start_ms).into(),
                "history-meta-time",
            ))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(t.spacing.xs))
                    .child(rows::outcome_glyph(&theme, r.outcome))
                    .child(
                        Label::new(
                            rows::outcome_text(cx, r.outcome),
                            TextSource::Fluent,
                            t.type_ramp.caption,
                            c.text.to_hsla(),
                        )
                        .id("history-meta-outcome"),
                    ),
            )
            .child(mono(
                duration_text(cx, u64::from(r.duration_ms)).into(),
                "history-meta-duration",
            ))
            .children(rows::rows_text(cx, r).map(|text| mono(text, "history-meta-rows")))
            .child(
                Label::new(
                    message(
                        cx,
                        "hv-history-ran",
                        vec![("n", ArgValue::Count(u64::from(hit.run_count)))],
                    ),
                    TextSource::Fluent,
                    t.type_ramp.caption,
                    ink2,
                )
                .id("history-meta-ran"),
            );
        if r.redacted {
            meta = meta.child(
                div()
                    .px(px(t.spacing.xs))
                    .border_1()
                    .border_color(c.seam_strong.to_hsla())
                    .rounded(px(t.radii.xs))
                    .child(
                        Label::new(
                            message(cx, "hv-history-redacted", vec![]),
                            TextSource::Fluent,
                            rows::mono11(&theme),
                            c.text.to_hsla(),
                        )
                        .id("history-redacted-chip"),
                    ),
            );
        }
        let pin_label = message(
            cx,
            if hit.pinned {
                "hv-history-unpin"
            } else {
                "hv-history-pin"
            },
            vec![],
        );
        let actions = div()
            .flex()
            .flex_row()
            .items_center()
            .ml_auto()
            .gap(px(t.spacing.xs))
            .child(
                Button::new("history-pin", pin_label)
                    .kind(ButtonKind::Icon)
                    .icon(IconName::Pin)
                    .on_raised(true)
                    .focus_handle(self.pin.clone())
                    .on_click(cx.listener(|d, _: &ClickEvent, _, cx| d.toggle_pin(cx))),
            )
            .child(
                Button::new(
                    "history-open-new",
                    message(cx, "hv-history-open-new", vec![]),
                )
                .kind(ButtonKind::Icon)
                .icon(IconName::ExternalLink)
                .on_raised(true)
                .disabled(conn.is_none())
                .focus_handle(self.open_new.clone())
                .on_click(cx.listener(|d, _: &ClickEvent, window, cx| {
                    d.open_in_new_console(window, cx);
                })),
            );
        let line =
            |icon: IconName, text: SharedString, id: &'static str| {
                div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap(px(t.spacing.sm))
                    .child(
                        div()
                            .flex_none()
                            .pt(px(2.0))
                            .child(Icon::new(icon).size(IconSize::Chrome).color(ink2)),
                    )
                    .child(div().flex_1().min_w_0().child(
                        paragraph(text, TextSource::Fluent, t.type_ramp.body_sm, ink2).id(id),
                    ))
            };
        let mut pane = div()
            .debug_selector(|| "history-preview".to_string())
            .flex()
            .flex_col()
            .gap(px(t.spacing.md))
            .pt(px(t.spacing.lg))
            .border_t_1()
            .border_color(c.seam.to_hsla())
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(t.spacing.md))
                    .child(div().flex_1().min_w_0().child(meta))
                    .child(actions),
            );
        // Fix round 1 (I2): always says which console Insert and Insert and run go into (its
        // title and environment mark), not only when the entry ran on another connection.
        if let Some(target) = console_facts(cx, self.console) {
            let text = message(
                cx,
                "hv-history-target",
                vec![(
                    "name",
                    ArgValue::Text(visible(&target.title, NAME_MAX_CHARS)),
                )],
            );
            pane = pane.child(
                div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap(px(t.spacing.sm))
                    .child(
                        div().flex_none().pt(px(2.0)).child(
                            Icon::new(IconName::ArrowRight)
                                .size(IconSize::Chrome)
                                .color(ink2),
                        ),
                    )
                    .children(target.env.map(|e| {
                        div()
                            .flex_none()
                            .pt(px(5.0))
                            .child(env_swatch(env_kind(e), &theme))
                    }))
                    .child(
                        div().flex_1().min_w_0().child(
                            paragraph(text, TextSource::Fluent, t.type_ramp.body_sm, ink2)
                                .id("history-target"),
                        ),
                    ),
            );
        }
        // Fix round 1 (I1, m4): why Insert and run is off for this entry, above the inset so
        // it stays in sight at 960 x 600.
        if masked(r) {
            pane = pane.child(line(
                IconName::CircleAlert,
                message(cx, "hv-history-masked-line", vec![]),
                "history-masked-line",
            ));
        }
        if truncated(r) {
            pane = pane.child(line(
                IconName::CircleAlert,
                message(cx, "hv-history-truncated-line", vec![]),
                "history-truncated-line",
            ));
        }
        pane = pane.child(sql_inset(&theme, &r.statement_masked, "history-sql"));
        if r.redacted {
            pane = pane.child(line(
                IconName::EyeOff,
                message(cx, "hv-history-redacted-line", vec![]),
                "history-redacted-line",
            ));
        }
        if r.outcome == Outcome::Failed
            && let Some(text) = r.error_message.as_deref()
        {
            pane = pane.child(server_box(&theme, r.error_code.as_deref(), text, cx));
        }
        Some(pane)
    }

    fn key_hint(theme: &Theme, chord: &str, text: SharedString, platform: KeymapPlatform) -> Div {
        let t = theme.tokens;
        let ink = t.colors.text_2.to_hsla();
        let keys =
            parse_chord(chord).map_or_else(|_| String::new(), |p| display_chord(&p, platform));
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(t.spacing.xs))
            .child(Label::new(
                keys,
                TextSource::Shortcut,
                t.type_ramp.mono_kbd,
                ink,
            ))
            .child(Label::new(
                text,
                TextSource::Fluent,
                t.type_ramp.caption,
                ink,
            ))
    }

    fn footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let platform = platform();
        let run_chord = if platform == KeymapPlatform::MacOs {
            "cmd-enter"
        } else {
            "ctrl-enter"
        };
        let hints = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(t.spacing.lg))
            .child(Self::key_hint(
                &theme,
                "enter",
                message(cx, "hv-history-insert", vec![]),
                platform,
            ))
            .child(Self::key_hint(
                &theme,
                run_chord,
                message(cx, "hv-history-insert-run", vec![]),
                platform,
            ))
            .child(Self::key_hint(
                &theme,
                "escape",
                hacivat_ui::l10n::ui_text(cx, "hui-palette-hint-close"),
                platform,
            ));
        let none = self.selected_hit().is_none();
        // Fix round 1 (I1, m4): a masked or cut entry is never run from here.
        let no_run = self.selected_hit().is_none_or(|h| !runnable(&h.record));
        let copy = Button::new("history-copy", message(cx, "hv-history-copy", vec![]))
            .icon(IconName::Copy)
            .on_raised(true)
            .disabled(none)
            .focus_handle(self.copy.clone())
            .on_click(cx.listener(|d, _: &ClickEvent, _, cx| d.copy_sql(cx)));
        let insert_run = Button::new(
            "history-insert-run",
            message(cx, "hv-history-insert-run", vec![]),
        )
        .icon(IconName::Play)
        .on_raised(true)
        .disabled(no_run)
        .focus_handle(self.insert_run.clone())
        .on_click(cx.listener(|d, event: &ClickEvent, window, cx| {
            if event.click_count() <= 1 {
                d.insert(true, window, cx);
            }
        }));
        let insert = Button::new("history-insert", message(cx, "hv-history-insert", vec![]))
            .kind(ButtonKind::Primary)
            .on_raised(true)
            .disabled(none)
            .focus_handle(self.insert.clone())
            .on_click(cx.listener(|d, event: &ClickEvent, window, cx| {
                if event.click_count() <= 1 {
                    d.insert(false, window, cx);
                }
            }));
        footer(
            &theme,
            vec![hints.into_any_element()],
            vec![
                copy.into_any_element(),
                insert_run.into_any_element(),
                insert.into_any_element(),
            ],
        )
    }
}

/// The platform whose key names the hints use.
fn platform() -> KeymapPlatform {
    if cfg!(target_os = "macos") {
        KeymapPlatform::MacOs
    } else if cfg!(windows) {
        KeymapPlatform::Windows
    } else {
        KeymapPlatform::Linux
    }
}

/// The server-message box of a failed entry: the caption and the code chip, then the stored
/// message (already scrubbed by the store) verbatim, made visible, in its own field.
fn server_box(theme: &Theme, code: Option<&str>, text: &str, cx: &App) -> impl IntoElement {
    let t = theme.tokens;
    let c = &t.colors;
    inset(theme)
        .id("history-server")
        .gap(px(t.spacing.xs))
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(t.spacing.sm))
                .child(
                    Label::new(
                        message(cx, "hv-conn-err-server", vec![]),
                        TextSource::Fluent,
                        t.type_ramp.caption,
                        c.text_2.to_hsla(),
                    )
                    .id("history-server-caption"),
                )
                .children(code.map(|code| {
                    div()
                        .px(px(t.spacing.xs))
                        .border_1()
                        .border_color(c.seam_strong.to_hsla())
                        .rounded(px(t.radii.xs))
                        .child(
                            Label::new(
                                visible(code, 32),
                                TextSource::Data,
                                t.type_ramp.mono_data,
                                c.text.to_hsla(),
                            )
                            .id("history-server-code"),
                        )
                })),
        )
        .child(
            paragraph(
                visible(text, SERVER_TEXT_MAX_CHARS),
                TextSource::Data,
                t.type_ramp.mono_data,
                c.text.to_hsla(),
            )
            .id("history-server-text"),
        )
}

impl Render for HistoryDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let title = message(cx, "hv-history-title", vec![]);
        // Fix round 1 (m5): a list at the search's cap says there may be more.
        let count = message(
            cx,
            if self.hits.len() >= SEARCH_LIMIT as usize {
                "hv-history-count-capped"
            } else {
                "hv-history-count"
            },
            vec![("n", ArgValue::Count(self.hits.len() as u64))],
        );
        let sub = div().child(
            Label::new(
                count,
                TextSource::Fluent,
                t.type_ramp.body_sm,
                t.colors.text_2.to_hsla(),
            )
            .id("history-count"),
        );
        let body = div()
            .flex()
            .flex_col()
            .gap(px(t.spacing.md))
            .child(self.bar(window, cx))
            .child(self.list(window, cx))
            .children(self.preview(cx));
        let card = card(&theme, DialogSize::Form, HISTORY_LAYER_ID)
            // ## m3b-t29-shots (named exception; T29 review I3): the announcement, outside
            // the list box.
            .children(self.selection_live(cx))
            .track_focus(&self.root)
            .capture_key_down(cx.listener(Self::on_capture_key))
            .on_key_down(cx.listener(Self::on_key))
            .child(header(
                &theme,
                title.clone(),
                "history-title",
                Some(sub),
                cx.listener(|d, _, window, cx| d.close(window, cx)),
                cx,
            ))
            .child(scroll_body(body))
            .child(self.footer(cx));
        A11yNode::new("history-node", Role::Dialog, title, true, card)
    }
}

// ---- test seams and shots fixtures -------------------------------------------------------------

/// What the open dialog shows (tests, shots).
#[cfg(any(feature = "test-support", feature = "shots"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// The rows' statements, top to bottom.
    pub statements: Vec<String>,
    pub selected: Option<usize>,
    /// The first search has answered (or the store failed).
    pub loaded: bool,
    pub all: bool,
    /// The search field's text.
    pub query: String,
    /// The searches started.
    pub searches: u64,
    /// The search whose answer is shown.
    pub answered: u64,
    // ## m3b-t27-history (fix round 1)
    /// The console Insert goes into.
    pub console: ConsoleId,
    /// Insert and run is offered for the selected entry.
    pub can_run: bool,
    /// The store query the filters make now.
    pub query_profile: Option<ProfileId>,
    pub query_outcome: Option<Outcome>,
    /// Searches superseded before they answered (their answers are never shown).
    pub stale: u64,
}

/// What the open history dialog shows; `None` when none is open.
#[cfg(any(feature = "test-support", feature = "shots"))]
pub fn shown(cx: &App) -> Option<Shown> {
    let view = cx.try_global::<Open>()?.0.as_ref()?.upgrade()?;
    let d = view.read(cx);
    Some(Shown {
        statements: d
            .hits
            .iter()
            .map(|h| h.record.statement_masked.to_string())
            .collect(),
        selected: (!d.hits.is_empty()).then_some(d.selected),
        loaded: d.state != ListState::Loading,
        all: d.all,
        query: d.input.read(cx).value().to_string(),
        searches: d.seq,
        answered: d.shown_seq,
        console: d.console,
        can_run: d.selected_hit().is_some_and(|h| runnable(&h.record)),
        query_profile: d.query(cx).profile,
        query_outcome: d.query(cx).outcome,
        stale: d.stale,
    })
}

/// Picks outcome choice `index` (0 = any, 1 = succeeded, 2 = failed, 3 = cancelled) as the
/// select does when the user picks it, and searches (tests).
#[cfg(feature = "test-support")]
pub fn pick_outcome(index: usize, cx: &mut App) {
    let Some(view) = cx
        .try_global::<Open>()
        .and_then(|o| o.0.as_ref())
        .and_then(|v| v.upgrade())
    else {
        return;
    };
    view.update(cx, |d, cx| {
        d.outcome.update(cx, |s, cx| s.set_selected(index, cx));
        d.search_now(cx);
    });
}

/// Shots fixtures for `design/m3b-safe` surface 11: the dialog over a console with given hits
/// (no store, no search), a fixed "now" and given connections.
#[cfg(any(feature = "test-support", feature = "shots"))]
pub mod fixtures {
    use super::*;

    /// One fixture: what the dialog shows.
    pub struct Fixture {
        pub hits: Vec<HistoryHit>,
        /// The connections the hits name: (profile, name, environment).
        pub conns: Vec<(ProfileId, String, Option<hacivat_proto::EnvTag>)>,
        pub query: String,
        pub all: bool,
        /// The outcome select's choice (0 = any, 1 = succeeded, 2 = failed, 3 = cancelled).
        pub outcome: usize,
        pub selected: usize,
        pub now_ms: i64,
    }

    /// The `design/m3b-safe` surface 11 scenarios (the mock's).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Scenario {
        /// This connection, any outcome, the newest selected.
        List,
        /// All connections, searched for `musteri`.
        Search,
        /// All connections, failed only: the server-message box.
        AllFailed,
        /// The entry a password was removed from.
        Redacted,
        /// All connections, an entry of `orders-prod` selected: the target line.
        Other,
        Empty,
        NoMatch,
    }

    /// "Now" in the fixtures: 2026-11-16 12:00:00 UTC (15:00 in Istanbul).
    pub const NOW_MS: i64 = 1_794_830_400_000;

    /// One mock entry: (connection 0 = the console's, 1 = `orders-prod`, 2 =
    /// `orders-staging`), minutes before [`NOW_MS`], the statement, the outcome, the duration
    /// in ms, the rows, pinned, the run count, redacted, the error (code, message).
    type Entry = (
        u8,
        i64,
        &'static str,
        Outcome,
        u32,
        Option<u64>,
        bool,
        u32,
        bool,
        Option<(&'static str, &'static str)>,
    );

    /// The mock's eight entries, newest first.
    const ENTRIES: [Entry; 8] = [
        (
            1,
            28,
            "SELECT c.city, count(*) AS adet, sum(o.total) AS ciro FROM orders o JOIN customers c ON c.id = o.customer_id WHERE o.placed_at >= date_trunc('day', now()) GROUP BY c.city ORDER BY ciro DESC;",
            Outcome::Ok,
            61,
            Some(12),
            true,
            4,
            false,
            None,
        ),
        (
            0,
            39,
            "SELECT c.full_name AS \"müşteri adı\", c.city, count(o.id) AS siparis FROM customers c JOIN orders o ON o.customer_id = c.id GROUP BY 1, 2;",
            Outcome::Ok,
            42,
            Some(100),
            false,
            1,
            false,
            None,
        ),
        (
            1,
            62,
            "UPDATE orders SET status = 'iptal' WHERE id = 1048213;",
            Outcome::Ok,
            12,
            Some(1),
            false,
            1,
            false,
            None,
        ),
        (
            0,
            236,
            "SELECT * FROM shipmnts WHERE carrier = 'Aras Kargo';",
            Outcome::Failed,
            4,
            None,
            false,
            2,
            false,
            Some(("42P01", "relation \"shipmnts\" does not exist")),
        ),
        (
            0,
            253,
            "ALTER ROLE raporcu PASSWORD '***';",
            Outcome::Ok,
            3,
            None,
            false,
            1,
            true,
            None,
        ),
        (
            2,
            1_248,
            "SELECT id, status, total, placed_at FROM orders WHERE status = 'beklemede' ORDER BY placed_at;",
            Outcome::Ok,
            9,
            Some(184),
            false,
            7,
            false,
            None,
        ),
        (
            0,
            1_377,
            "SELECT pg_sleep(30), carrier FROM shipments;",
            Outcome::Cancelled,
            12_408,
            None,
            false,
            1,
            false,
            None,
        ),
        (
            0,
            1_770,
            "SELECT * FROM MÜŞTERİLER WHERE şehir = 'İSTANBUL';",
            Outcome::Ok,
            18,
            Some(2_140),
            false,
            1,
            false,
            None,
        ),
    ];

    /// Scenario `s` over console `console` (its connection plays `orders-dev`).
    pub fn scenario(s: Scenario, console: ConsoleId, cx: &App) -> Fixture {
        let here = Consoles::get(cx, console)
            .and_then(|c| c.profile)
            .unwrap_or_default();
        let (prod, staging) = (ProfileId::new(), ProfileId::new());
        let profile = |k: u8| match k {
            0 => here,
            1 => prod,
            _ => staging,
        };
        let all: Vec<HistoryHit> = ENTRIES
            .iter()
            .enumerate()
            .map(
                |(i, &(k, ago, sql, outcome, ms, rows, pinned, runs, redacted, error))| {
                    HistoryHit {
                        id: i as i64 + 1,
                        record: hacivat_exec::HistoryRecord {
                            ts_start_ms: NOW_MS - ago * 60_000,
                            duration_ms: ms,
                            profile: profile(k),
                            database: None,
                            schema: None,
                            console: 1,
                            origin: hacivat_exec::HistoryOrigin::Console,
                            statement_masked: Arc::from(sql),
                            text_hash: [0; 32],
                            fingerprint: 0,
                            params_masked: None,
                            outcome,
                            rows_affected: None,
                            rows_fetched: rows,
                            error_code: error.map(|(code, _)| code.to_string()),
                            error_message: error.map(|(_, text)| text.to_string()),
                            tx_mode: hacivat_session::session::TxMode::Auto,
                            run: None,
                            redacted,
                        },
                        run_count: runs,
                        pinned,
                    }
                },
            )
            .collect();
        let pick = |keep: &dyn Fn(usize, &HistoryHit) -> bool| -> Vec<HistoryHit> {
            all.iter()
                .enumerate()
                .filter(|(i, h)| keep(*i, h))
                .map(|(_, h)| h.clone())
                .collect()
        };
        let mine = |_: usize, h: &HistoryHit| h.record.profile == here;
        let (hits, query, all_conns, outcome, selected) = match s {
            Scenario::List => (pick(&mine), "", false, 0, 0),
            Scenario::Search => (pick(&|i, _| i == 1 || i == 7), "musteri", true, 0, 1),
            Scenario::AllFailed => (
                pick(&|_, h| h.record.outcome == Outcome::Failed),
                "",
                true,
                2,
                0,
            ),
            Scenario::Redacted => (pick(&mine), "", false, 0, 2),
            Scenario::Other => (pick(&|_, _| true), "", true, 0, 0),
            Scenario::Empty => (Vec::new(), "", false, 0, 0),
            Scenario::NoMatch => (Vec::new(), "faturalar", false, 0, 0),
        };
        Fixture {
            hits,
            conns: vec![
                (
                    prod,
                    "orders-prod".into(),
                    Some(hacivat_proto::EnvTag::Production),
                ),
                (
                    staging,
                    "orders-staging".into(),
                    Some(hacivat_proto::EnvTag::Staging),
                ),
            ],
            query: query.into(),
            all: all_conns,
            outcome,
            selected,
            now_ms: NOW_MS,
        }
    }

    /// Opens the dialog over `console` showing `f`.
    pub fn open(console: ConsoleId, f: Fixture, window: &mut Window, cx: &mut App) {
        let view = cx.new(|cx| {
            let mut d = HistoryDialog::new(console, None, window, cx);
            for (id, name, env) in f.conns {
                d.conns.insert(
                    id,
                    Conn {
                        name: Arc::from(name.as_str()),
                        env,
                    },
                );
            }
            d.input.update(cx, |i, cx| {
                i.set_value(f.query.clone());
                cx.notify();
            });
            d.outcome.update(cx, |s, cx| s.set_selected(f.outcome, cx));
            d.all = f.all || d.here.is_none();
            d.now_ms = f.now_ms;
            d.hits = f.hits;
            d.selected = f.selected.min(d.hits.len().saturating_sub(1));
            d.answered = f.query;
            d.state = ListState::Ready;
            d.reveal = true;
            d
        });
        show(&view, window, cx);
    }
}
