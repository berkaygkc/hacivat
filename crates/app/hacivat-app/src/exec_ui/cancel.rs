//! Cancel (Task 33): `exec.cancel` cancels the run, or stops fetching a streaming or suspended
//! result.
//!
//! `exec.cancel` (`cmd-.` on macOS, `ctrl-f2` on Windows and Linux, R-M3-38; context `Console`)
//! cancels the request the console's session is running: the run (Stop = stop fetching, rows
//! kept, R-M3-16; the session decides when a server cancel is due), or a Fetch more / Fetch all
//! of an open cursor. Escape never cancels: the editor keeps Escape, and this command has no
//! Escape row. The unit in flight shows "cancelling…" until its outcome arrives.

use std::rc::Rc;

use hacivat_session::session::SessionState;
use hacivat_ui::actions::{ActionDef, ActionRegistry};
use hacivat_ui_core::gpui_surface::{App, Window};
use hacivat_ui_editor::FocusedEditor;

use crate::console::{ConsoleId, Consoles};
use crate::exec_ui::{console_exec, marks};

/// The key context of the console commands: the console tab's (one constant, Fix round 1).
pub(crate) const CONSOLE: &str = crate::shell::tabs::CONSOLE_CONTEXT;

/// The console whose editor has the focus, else the one last focused, else the only one.
pub(crate) fn focused_console(cx: &App) -> Option<ConsoleId> {
    let focused = cx
        .try_global::<FocusedEditor>()
        .and_then(|f| f.0.as_ref())
        .and_then(|w| w.upgrade());
    let ids = Consoles::ids(cx);
    let by_editor = focused.and_then(|editor| {
        ids.iter().copied().find(|id| {
            Consoles::get(cx, *id)
                .and_then(|s| s.editor.as_ref())
                .is_some_and(|e| e.entity_id() == editor.entity_id())
        })
    });
    by_editor
        .or_else(|| Consoles::focused(cx))
        .or_else(|| (ids.len() == 1).then(|| ids[0]))
}

/// The request console `console`'s session is running that a cancel would stop: the busy
/// request (a run, a fetch), else the console's run while it has not ended. Request 0 is an
/// idle release, which no one holds.
fn running_request(console: ConsoleId, cx: &App) -> Option<u64> {
    let session = Consoles::get(cx, console)?.session.clone()?;
    let probe = console_exec(cx, console).and_then(|e| e.probe);
    let busy = match session.state().borrow().state {
        // Request 0 is an idle release; the SQL-context probe is Hacivat's own.
        SessionState::Busy { req, .. } if req != 0 && Some(req) != probe => Some(req),
        _ => None,
    };
    busy.or_else(|| {
        console_exec(cx, console)?
            .run
            .as_ref()
            .filter(|r| !r.ended)
            .map(|r| r.seq)
    })
}

fn can_cancel(cx: &App) -> bool {
    focused_console(cx).is_some_and(|c| running_request(c, cx).is_some())
}

/// Cancels what console `console` runs; nothing when it runs nothing.
pub fn cancel(console: ConsoleId, cx: &mut App) -> bool {
    let Some(seq) = running_request(console, cx) else {
        return false;
    };
    let Some(session) = Consoles::get(cx, console).and_then(|s| s.session.clone()) else {
        return false;
    };
    marks::note_cancel(console, cx);
    let outcome = session.cancel(seq);
    cx.spawn(async move |_| {
        tracing::debug!(outcome = ?outcome.await, "exec.cancel");
    })
    .detach();
    true
}

fn run_cancel(_window: &mut Window, cx: &mut App) {
    if let Some(console) = focused_console(cx) {
        cancel(console, cx);
    }
}

/// Registers `exec.cancel`.
pub(crate) fn register(cx: &mut App) {
    let reg = cx.default_global::<ActionRegistry>();
    let _ = reg.register(ActionDef {
        id: "exec.cancel",
        title_id: "hv-cmd-exec-cancel",
        context: CONSOLE,
        run: Rc::new(run_cancel),
        enabled: can_cancel,
    });
}
