//! The connection dialog's SSH section (NOTES surface 6, Task 20): the "Connect through SSH"
//! disclosure, hops, authentication, host-key policy and `~/.ssh/config` import; its issues
//! render by `IssueCode` (Q9).
//!
//! Task 1 stubs: the section draws nothing; "Connect through SSH" opens the edit dialog
//! (the `open_edit` seam with `DialogFocus::Ssh`). Task 20 fills both.

use hacivat_proto::ProfileId;
use hacivat_session::profile::ProfileDraft;
use hacivat_ui_core::gpui_surface::{AnyElement, App, IntoElement, Window, div};

use super::{DialogFocus, open_edit};

/// The SSH section of the form for `draft`. Task 1 stub: an empty element.
pub(crate) fn section(_draft: &ProfileDraft, _window: &mut Window, _cx: &mut App) -> AnyElement {
    div().into_any_element()
}

/// The "Connect through SSH" fix: the edit dialog of `id` with the SSH section open (Q9).
pub(crate) fn open_with_ssh(id: ProfileId, window: &mut Window, cx: &mut App) {
    open_edit(id, DialogFocus::Ssh, window, cx);
}
