// ## m4b-t10-service (fix round 1, I1, ruling R-T10-2)
//! Events as one connection may see them. The service announces every change once, on an
//! internal channel; each subscriber names the environment of the connection it serves
//! ([`LibraryService::subscribe`](super::LibraryService::subscribe)) and receives only what that
//! connection shows:
//!
//! - `Changed` and `GitChanged` carry no path and always pass;
//! - `FileChanged` and `FileDeleted` pass only for a path shown under the connection;
//! - `FileRenamed { from, to }` passes when both are shown; when only `from` is, it arrives as
//!   [`LibraryEvent::FileLeftScope`] for `from` ("no longer listed under this connection"),
//!   never with `to`; when `from` is not shown, nothing arrives (the new file is a listing
//!   change).
//!
//! So a staging tab never learns the name of a `@production` file through an event.

use std::sync::Arc;

use arc_swap::ArcSwap;
use hacivat_proto::EnvTag;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

use super::WorkspaceSnapshot;
use crate::envscope::{self, admits};
use crate::events::LibraryEvent;
use crate::pathsafe::RelPath;
use crate::registry::LibraryId;
use crate::scan::{self, EntryKind};

/// A receiver of the service's events, filtered for one environment. On
/// `RecvError::Lagged` some events were missed: re-read the snapshot (a tab compares its file's
/// `(len, mtime)` with the snapshot's entry, m8).
#[derive(Debug)]
pub struct Subscription {
    rx: broadcast::Receiver<LibraryEvent>,
    env: Option<EnvTag>,
    snapshot: Arc<ArcSwap<WorkspaceSnapshot>>,
}

impl Subscription {
    pub(super) fn new(
        rx: broadcast::Receiver<LibraryEvent>,
        env: Option<EnvTag>,
        snapshot: Arc<ArcSwap<WorkspaceSnapshot>>,
    ) -> Self {
        Subscription { rx, env, snapshot }
    }

    /// The environment this subscription filters for.
    pub fn env(&self) -> Option<EnvTag> {
        self.env
    }

    /// The next event this connection may see.
    pub async fn recv(&mut self) -> Result<LibraryEvent, RecvError> {
        loop {
            let e = self.rx.recv().await?;
            if let Some(e) = self.filter(e) {
                return Ok(e);
            }
        }
    }

    /// The next event already waiting, if any.
    pub fn try_recv(&mut self) -> Result<LibraryEvent, TryRecvError> {
        loop {
            let e = self.rx.try_recv()?;
            if let Some(e) = self.filter(e) {
                return Ok(e);
            }
        }
    }

    fn filter(&self, e: LibraryEvent) -> Option<LibraryEvent> {
        match e {
            LibraryEvent::Changed { .. } | LibraryEvent::GitChanged { .. } => Some(e),
            LibraryEvent::FileChanged { id, ref rel, .. } => self.shows_now(id, rel).then_some(e),
            LibraryEvent::FileDeleted { ref rel, .. }
            | LibraryEvent::FileLeftScope { ref rel, .. } => self.shows_path(rel).then_some(e),
            LibraryEvent::FileRenamed { id, from, to } => {
                match (self.shows_path(&from), self.shows_now(id, &to)) {
                    (true, true) => Some(LibraryEvent::FileRenamed { id, from, to }),
                    (true, false) => Some(LibraryEvent::FileLeftScope { id, rel: from }),
                    (false, _) => None,
                }
            }
        }
    }

    /// Whether the path's folders admit this connection (for a file that is gone).
    fn shows_path(&self, rel: &RelPath) -> bool {
        let kind = scan::script_kind(rel.name()).unwrap_or(EntryKind::Sql);
        admits(&envscope::entry_scope(rel, kind), self.env)
    }

    /// Whether the file is shown now: its path, and its listed entry when there is one (a link
    /// narrowed to its target's scope).
    fn shows_now(&self, id: LibraryId, rel: &RelPath) -> bool {
        if !self.shows_path(rel) {
            return false;
        }
        let snap = self.snapshot.load();
        let entry = snap
            .library(id)
            .and_then(|l| l.entries.iter().find(|e| &e.rel == rel));
        entry.is_none_or(|e| envscope::visible(e, self.env))
    }
}
// ## m4b-t10-service
