//! Console UX state (Task 23): the pure table the toolbar, the status bar and the capsule read
//! (production, forced read-only, the transaction badge, the cursor, cancel, waiting).

use std::time::Instant;

use hacivat_proto::{AccessLabel, EnvTag};
use hacivat_session::session::{CursorBadge, RunId, SessionSnapshot};

use crate::status::RunStatus;

/// The transaction badge: the user's transaction only (`TxOwner::User`, R-M3-27); `Failed`
/// after an error or a server cancel inside it (R-M3-16 amended).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TxBadge {
    Auto,
    UserOpen { since: Instant, statements: u32 },
    Failed,
    CursorSnapshot { since: Instant },
}

/// Whether a cancel is under way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CancelUx {
    Idle,
    Cancelling { since: Instant },
}

/// What a console shows about its session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConsoleUx {
    pub env: Option<EnvTag>,
    pub production: bool,
    pub read_only: bool,
    /// Read-only because the profile is production (R-M3-6), not because it says so.
    pub forced_read_only: bool,
    pub access: AccessLabel,
    pub tx: TxBadge,
    pub cursor: Option<CursorBadge>,
    pub cancel: CancelUx,
    pub running: Option<(RunId, Instant)>,
    /// "Waiting for a free connection" (in use, max), R-M3-43.
    pub waiting_for_connection: Option<(u8, u8)>,
}

/// The console UX for a session (Task 23).
pub fn console_ux(
    env: Option<EnvTag>,
    forced: bool,
    snap: &SessionSnapshot,
    run: Option<&RunStatus>,
    waiting: Option<(u8, u8)>,
) -> ConsoleUx {
    let _ = (env, forced, snap, run, waiting);
    unimplemented!("console_ux: Task 23")
}
