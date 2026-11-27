//! Console UX state (Task 23): the pure table the toolbar, the status bar and the capsule read
//! (production, read-only, the transaction badge, the cursor, cancel, waiting).
//!
//! The table, row by row:
//! - `production`: the profile is tagged production; `read_only` is the session's (## m3b-t26-
//!   prod-unlock: `forced_read_only` and the production policy are gone, R-M3-6 REVISED ends:
//!   a production session is read-only only when its profile is);
//! - `tx`: only the user's transaction is a badge (`TxOwner::User`, R-M3-27): open is
//!   `UserOpen`, failed is `Failed` (R-M3-16 amended); Hacivat's own transaction around a
//!   PostgreSQL cursor is `CursorSnapshot`; the read-only guard's, or one nobody claimed, is
//!   `Auto`;
//! - `cursor`: the session's open cursor; `cancel`: a cancel on its way (any rung);
//! - `running`: a run that started and has not ended (a session that is `Busy` with request 0
//!   is releasing an idle cursor, which no run holds, and is not running anything);
//! - `waiting_for_connection`: the pool's (in use, max) while the console waits (R-M3-43).

use std::time::Instant;

use hacivat_driver::TxStatus;
use hacivat_proto::{AccessLabel, EnvTag};
use hacivat_session::session::{
    CancelState, CursorBadge, RunId, SessionSnapshot, SessionState, TxOwner,
};

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
    pub access: AccessLabel,
    pub tx: TxBadge,
    pub cursor: Option<CursorBadge>,
    pub cancel: CancelUx,
    pub running: Option<(RunId, Instant)>,
    /// "Waiting for a free connection" (in use, max), R-M3-43.
    pub waiting_for_connection: Option<(u8, u8)>,
}

/// The console UX for a session; `run` is the console's latest run.
pub fn console_ux(
    env: Option<EnvTag>,
    snap: &SessionSnapshot,
    run: Option<&RunStatus>,
    waiting: Option<(u8, u8)>,
) -> ConsoleUx {
    let production = env == Some(EnvTag::Production);
    ConsoleUx {
        env,
        production,
        read_only: snap.read_only,
        access: snap.access,
        tx: tx_badge(snap),
        cursor: snap.cursor,
        cancel: cancel_ux(snap),
        running: run.and_then(running),
        waiting_for_connection: waiting,
    }
}

fn tx_badge(snap: &SessionSnapshot) -> TxBadge {
    // ## m3a-t33-exec-ui (named exception, T34 review I3): a transaction lives on its
    // connection; with none (disconnected, connecting again, broken) there is nothing to show,
    // whatever the last published `tx` says.
    if !session_live(&snap.state) {
        return TxBadge::Auto;
    }
    let tx = &snap.tx;
    match (tx.owner, tx.status) {
        // The session sets `since` whenever a transaction opens; were it missing, the badge
        // still shows (never hidden), from the best clue there is.
        (TxOwner::User, TxStatus::InTx) => TxBadge::UserOpen {
            since: tx
                .since
                .or_else(|| busy_since(snap))
                .unwrap_or_else(Instant::now),
            statements: tx.statements,
        },
        (TxOwner::User, TxStatus::Failed) => TxBadge::Failed,
        (TxOwner::Cursor, TxStatus::InTx) => match tx.since.or(snap.cursor.map(|c| c.since)) {
            Some(since) => TxBadge::CursorSnapshot { since },
            None => TxBadge::Auto,
        },
        _ => TxBadge::Auto,
    }
}

// ## m3a-t33-exec-ui (named exception)
/// Whether the session holds a connection (idle, or running something).
pub fn session_live(state: &SessionState) -> bool {
    matches!(
        state,
        SessionState::Idle | SessionState::Busy { .. } | SessionState::Cancelling { .. }
    )
}

fn busy_since(snap: &SessionSnapshot) -> Option<Instant> {
    match snap.state {
        SessionState::Busy { since, .. } | SessionState::Cancelling { since, .. } => Some(since),
        _ => None,
    }
}

fn cancel_ux(snap: &SessionSnapshot) -> CancelUx {
    match snap.cancel {
        CancelState::Cancelling { since } | CancelState::SlowCancel { since } => {
            CancelUx::Cancelling { since }
        }
        CancelState::Terminating | CancelState::Deactivating => match snap.state {
            SessionState::Cancelling { since, .. } | SessionState::Busy { since, .. } => {
                CancelUx::Cancelling { since }
            }
            _ => CancelUx::Idle,
        },
        CancelState::Idle => match snap.state {
            SessionState::Cancelling { since, .. } => CancelUx::Cancelling { since },
            _ => CancelUx::Idle,
        },
    }
}

fn running(run: &RunStatus) -> Option<(RunId, Instant)> {
    match (run.run, run.started, run.summary) {
        (Some(id), Some(started), None) => Some((id, started)),
        _ => None,
    }
}
