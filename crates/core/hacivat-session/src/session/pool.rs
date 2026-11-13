//! A profile's sessions (HV-SESS-01, -02, -08; D-035; Task 15): the default session, client
//! and job sessions, the idle pool, and the session overlay (Q17).
//!
//! Task 1 stub: no session opens (`open` and `pooled` fail with `internal`), nothing is kept.

use std::fmt;
use std::ops::Deref;
use std::sync::{Arc, Mutex, PoisonError};

use futures::future::BoxFuture;
use hacivat_proto::ErrClass;
use tokio::runtime::Handle;

use crate::budget::Budget;
use crate::pipeline::{ConnectDeps, ConnectError, Interaction, Phase, SessionSecrets};
use crate::profile::Profile;
use crate::session::{SessionHandle, SessionKind, SessionSpec};

/// What a profile's sessions keep for reconnects and side connections, in memory only (Q17):
/// the profile with the changes made since connect, the plaintext consent and the secrets that
/// worked. Dropped at disconnect, forget and quit; never persisted. `Debug` only (redacted).
#[derive(Debug)]
pub struct SessionOverlay {
    pub profile: Arc<Profile>,
    pub plaintext_consent: bool,
    pub secrets: SessionSecrets,
}

/// A session borrowed from the pool. Dropping it resets it and returns it to the idle pool
/// (closed after 60 s idle); a borrow of the default session (Single mode, SQLite) is never
/// reset (Q19).
#[derive(Debug)]
pub struct PooledSession {
    handle: SessionHandle,
}

impl Deref for PooledSession {
    type Target = SessionHandle;

    fn deref(&self) -> &SessionHandle {
        &self.handle
    }
}

/// One profile's sessions.
pub struct SessionManager {
    #[allow(dead_code, reason = "Task 15 runs sessions on it")]
    rt: Handle,
    profile: Mutex<Arc<Profile>>,
    #[allow(dead_code, reason = "Task 15 connects with them")]
    deps: Arc<ConnectDeps>,
    #[allow(dead_code, reason = "Task 15 takes permits from it")]
    budget: Arc<Budget>,
}

impl fmt::Debug for SessionManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionManager")
            .field("profile", &self.profile().id)
            .field("open", &self.open_count())
            .finish_non_exhaustive()
    }
}

/// The error of a session Task 15 cannot open yet.
fn not_yet() -> ConnectError {
    crate::pipeline::error(ErrClass::Internal, Phase::Validate)
}

impl SessionManager {
    pub fn new(
        rt: Handle,
        profile: Arc<Profile>,
        deps: Arc<ConnectDeps>,
        budget: Arc<Budget>,
    ) -> Arc<SessionManager> {
        Arc::new(SessionManager {
            rt,
            profile: Mutex::new(profile),
            deps,
            budget,
        })
    }

    fn profile(&self) -> Arc<Profile> {
        self.profile
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Opens a session for `spec`. Task 1 stub: fails with `internal`.
    pub fn open(
        self: &Arc<Self>,
        _spec: SessionSpec,
        _ix: Interaction,
    ) -> BoxFuture<'static, Result<SessionHandle, ConnectError>> {
        Box::pin(async { Err(not_yet()) })
    }

    /// Borrows a pooled session of `kind`. Task 1 stub: fails with `internal`.
    pub fn pooled(
        self: &Arc<Self>,
        _kind: SessionKind,
    ) -> BoxFuture<'static, Result<PooledSession, ConnectError>> {
        Box::pin(async { Err(not_yet()) })
    }

    pub fn default_session(&self) -> Option<SessionHandle> {
        None
    }

    pub fn open_count(&self) -> usize {
        0
    }

    /// Closes every session.
    pub fn close_all(&self) -> BoxFuture<'static, ()> {
        Box::pin(async {})
    }

    /// Replaces the overlay's profile, keeping the consent and the secrets (Q17).
    pub fn update_profile(&self, profile: Arc<Profile>) {
        *self.profile.lock().unwrap_or_else(PoisonError::into_inner) = profile;
    }

    /// The overlay, while the profile is connected. Task 1 stub: none.
    pub fn overlay(&self) -> Option<SessionOverlay> {
        None
    }
}
