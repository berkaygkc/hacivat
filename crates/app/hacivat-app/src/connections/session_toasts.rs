//! The toasts of live sessions (NOTES new surfaces 6 and 7, Task 22): a session lost with
//! state, an idle disconnect, an idle transaction, a read-only enforcement gap.
//!
//! Task 1 stubs show nothing. Task 22 fills them.

use hacivat_proto::ProfileId;
use hacivat_session::GapReason;
use hacivat_session::session::LostState;
use hacivat_ui_core::gpui_surface::App;

/// A session of `profile` was lost and a reconnect would lose `lost`.
pub(crate) fn lost(_profile: ProfileId, _lost: LostState, _cx: &mut App) {}

/// `profile` was disconnected after being idle.
pub(crate) fn idle(_profile: ProfileId, _cx: &mut App) {}

/// A transaction of `profile` has been open and idle for `minutes`.
pub(crate) fn idle_tx(_profile: ProfileId, _minutes: u32, _cx: &mut App) {}

/// A read-only session of `profile` saw a statement escape the guard (the copy depends on
/// `reason`, Q35).
pub(crate) fn gap(_profile: ProfileId, _reason: GapReason, _cx: &mut App) {}
