//! The run orchestrator (Task 33): on `ExecuteRequested`, the doc snapshot, `resolve_units`,
//! the chooser when the range is ambiguous, one `ExecService::run`, the marks and the result
//! tabs.
//!
//! Task 1b stub: `attach` subscribes to the editor and answers Run with a notice; it sends
//! nothing to the session (fail closed).

use hacivat_l10n::ids;
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{App, Entity, Window};
use hacivat_ui_editor::{EditorEvent, EditorView};

use crate::console::ConsoleId;
use crate::l10n_global::text_fn;

/// Wires console `console`'s editor to the run UI; called once when the console is built.
pub fn attach(console: ConsoleId, editor: &Entity<EditorView>, window: &mut Window, cx: &mut App) {
    let _ = (console, window);
    cx.subscribe(editor, |_editor, event: &EditorEvent, cx| {
        if let EditorEvent::ExecuteRequested(_) = event {
            ToastQueue::push(
                cx,
                Toast::new(ToastKind::Info, text_fn(ids::hv::CONSOLE_RUN_UNAVAILABLE)),
            );
        }
    })
    .detach();
}
