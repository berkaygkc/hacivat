//! The production-tag prompt before a first connect (NOTES surface 4, R22, Task 20).

use hacivat_proto::EnvTag;
use hacivat_session::profile::HintSource;
use hacivat_ui_core::gpui_surface::{App, Window};
use tokio::sync::oneshot;

/// Asks which environment `name` at `host` is; `hint` is `production_hint`'s result (P7).
/// Resolves with the chosen tag, or `None` when cancelled. The stub answers `None` at once.
pub fn ask_environment(
    _name: String,
    _host: String,
    _hint: Option<(&'static str, HintSource)>,
    _window: &mut Window,
    _cx: &mut App,
) -> oneshot::Receiver<Option<EnvTag>> {
    let (tx, rx) = oneshot::channel();
    let _ = tx.send(None);
    rx
}
