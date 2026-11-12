//! The connect-error dialog: class heading and body, the server's message verbatim, technical
//! details and quick fixes (NOTES surface 3, HV-CONN-03, Task 20).

use std::sync::Arc;

use hacivat_proto::ProfileId;
use hacivat_session::pipeline::ConnectError;
use hacivat_ui_core::gpui_surface::{App, Window};

/// Opens the error dialog for `id`'s failed connect (Task 20). The stub does nothing.
pub fn open_error_dialog(
    _id: ProfileId,
    _error: Arc<ConnectError>,
    _window: &mut Window,
    _cx: &mut App,
) {
}
