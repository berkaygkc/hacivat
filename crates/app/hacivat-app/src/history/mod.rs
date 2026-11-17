//! The query history dialog (HV-EXEC-12 UI, D-061; M3b Task 27, `design/m3b-safe`): search,
//! filter by connection and outcome, pin, and insert a statement at the caret.
//!
//! ## m3b-t01b-seams: module lines, `install` and the stubs (Task 1b). `history.open` is
//! registered (chord ⌘⇧H / Ctrl+Shift+H, context `Workspace`) but disabled, so the key passes
//! through and the palette does not list it; [`open`] opens nothing. Task 27 fills `dialog`
//! and `rows`, [`open`] and the command's `enabled`.
//!
//! ## m3b-t27-history
//!
//! - **The store.** The dialog reads Task 17's `HistoryStore` once the app has opened it
//!   ([`set_store`]; Task 24's `open_history` hands it over). Without one (history off, or not
//!   open yet) `history.open` is disabled and its chord passes through.
//! - **Threads (Q4).** Every search and pin runs on the core runtime ([`search`], [`pin`]); the
//!   UI thread only awaits a `tokio::sync::oneshot`, never SQLite.
//! - **What is shown** is what the store kept: the statement with its bind values masked and
//!   its secrets redacted (Task 17), never a bind value. History is a convenience, never an
//!   audit trail; nothing here logs a record.

pub mod dialog;
pub mod rows;

use std::rc::Rc;
use std::sync::Arc;

use futures::FutureExt as _;
use futures::future::BoxFuture;
use hacivat_exec::{HistoryError, HistoryHit, HistoryQuery, HistoryStore};
use hacivat_ui::actions::{ActionDef, ActionRegistry};
use hacivat_ui_core::gpui_surface::{App, Global};

use crate::exec_ui::cancel::focused_console;

/// Registers `history.open`.
pub fn install(cx: &mut App) {
    let _ = cx.default_global::<ActionRegistry>().register(ActionDef {
        id: "history.open",
        title_id: "hv-cmd-history-open",
        context: "Workspace",
        // ## m3b-t27-history: opens in the window the command runs in.
        run: Rc::new(|window, cx| {
            if let Some(console) = focused_console(cx) {
                dialog::open(console, window, cx);
            }
        }),
        enabled: can_open,
    });
}

// ## m3b-t27-history

/// Opens the history dialog over the focused console, in the workspace window (after the
/// current update). Nothing without a store or a console.
pub fn open(cx: &mut App) {
    if !can_open(cx) {
        return;
    }
    let Some(console) = focused_console(cx) else {
        return;
    };
    crate::exec_ui::in_workspace_window(cx, move |_, window, cx| {
        dialog::open(console, window, cx);
    });
}

/// Whether `history.open` can run: the history is open and a console has (or last had) the
/// focus.
fn can_open(cx: &App) -> bool {
    store(cx).is_some() && focused_console(cx).is_some()
}

/// The history store the dialog reads, once the app opened it.
#[derive(Default)]
struct HistorySource {
    store: Option<Arc<HistoryStore>>,
}

impl Global for HistorySource {}

/// Hands the dialog the opened history (`None`: the history is off). **Merge point (Task 24):**
/// `exec_ui::orchestrator::open_history` calls it when the store answers.
pub fn set_store(store: Option<Arc<HistoryStore>>, cx: &mut App) {
    cx.default_global::<HistorySource>().store = store;
}

/// The history the dialog reads.
pub fn store(cx: &App) -> Option<Arc<HistoryStore>> {
    cx.try_global::<HistorySource>()?.store.clone()
}

/// Runs `work` on the core runtime and hands back a future the UI thread may await: it only
/// waits on a channel. Without a core runtime the answer is `Disabled`.
fn on_core<T: Send + 'static>(
    work: BoxFuture<'static, Result<T, HistoryError>>,
    cx: &App,
) -> BoxFuture<'static, Result<T, HistoryError>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    if let Some(m) = cx.try_global::<crate::connections::ConnectionsModel>() {
        m.runtime().spawn(async move {
            let _ = tx.send(work.await);
        });
    }
    async move { rx.await.unwrap_or(Err(HistoryError::Disabled)) }.boxed()
}

/// Searches `store` with `q` off the UI thread.
pub(crate) fn search(
    store: &HistoryStore,
    q: HistoryQuery,
    cx: &App,
) -> BoxFuture<'static, Result<Vec<HistoryHit>, HistoryError>> {
    on_core(store.search(q), cx)
}

// ## m3c-t34-editor (named exception): the editor's history recall.
/// The `n` newest statements of `profile`, newest first, off the UI thread.
pub(crate) fn recall(
    store: &HistoryStore,
    profile: hacivat_proto::ProfileId,
    n: u32,
    cx: &App,
) -> BoxFuture<'static, Result<Vec<Arc<str>>, HistoryError>> {
    on_core(store.recall(profile, String::new(), n), cx)
}

/// Pins or unpins entry `id` off the UI thread.
pub(crate) fn pin(
    store: &HistoryStore,
    id: i64,
    pinned: bool,
    cx: &App,
) -> BoxFuture<'static, Result<(), HistoryError>> {
    on_core(store.pin(id, pinned), cx)
}
