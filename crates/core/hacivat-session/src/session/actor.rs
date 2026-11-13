//! The session actor (HV-SESS-01; Task 7): one task per connection that owns the `RawConn`,
//! runs requests in order and keeps the snapshot.
//!
//! Task 1 stub: `spawn_session` closes the connection and returns a handle without an actor,
//! so every request is answered `NotConnected` and nothing runs.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use futures::future::BoxFuture;
use tokio::runtime::Handle;
use tokio::sync::{broadcast, mpsc, watch};

use crate::pipeline::{ConnectError, LiveConnection};
use crate::profile::Profile;
use crate::session::{
    CancelState, SessionHandle, SessionId, SessionSnapshot, SessionSpec, SessionState, TxState,
};

/// Opens the side connection a cancel or a kill needs (`Department::Cancel`). Task 15's
/// implementation connects with the session overlay (Q17).
pub trait SideConnector: Send + Sync {
    fn open_side(&self) -> BoxFuture<'static, Result<LiveConnection, ConnectError>>;
}

/// Reconnects a lost session (Task 17); `attempt` counts from 1.
pub trait Reconnector: Send + Sync {
    fn reconnect(&self, attempt: u32) -> BoxFuture<'static, Result<LiveConnection, ConnectError>>;
}

/// What an actor may call outside itself.
#[derive(Clone)]
pub struct SessionHooks {
    pub side: Arc<dyn SideConnector>,
    /// `None`: a lost session stays lost (Task 17 supplies it).
    pub reconnect: Option<Arc<dyn Reconnector>>,
}

impl fmt::Debug for SessionHooks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionHooks")
            .field("reconnect", &self.reconnect.is_some())
            .finish_non_exhaustive()
    }
}

/// The next session ID.
fn next_id() -> SessionId {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    SessionId(NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst))
}

/// Starts the actor of `live` on `rt`. Task 1 stub: closes `live` and returns a handle without
/// an actor (state `Disconnected`; every request `NotConnected`).
pub fn spawn_session(
    rt: &Handle,
    spec: SessionSpec,
    _profile: Arc<Profile>,
    live: LiveConnection,
    _hooks: SessionHooks,
) -> SessionHandle {
    let snapshot = SessionSnapshot {
        state: SessionState::Disconnected,
        tx: TxState::default(),
        schema: None,
        cancel: CancelState::Idle,
        read_only: spec.read_only,
        access: live.access,
        generation: 0,
    };
    rt.spawn(live.close());
    let (commands, _closed) = mpsc::unbounded_channel();
    let (_state_tx, state) = watch::channel(snapshot);
    SessionHandle {
        id: next_id(),
        kind: spec.kind,
        commands,
        state,
        events: broadcast::Sender::new(16),
        next_seq: Arc::new(AtomicU64::new(1)),
    }
}
