// ## m3b-t01c-core-seams
//! The auditor chain (R-M3-63; Task 16): around M3a's observer, the synchronous O(1) auditors
//! (timing, the transaction marker, cancel progress, gutter state) see every `RunEvent` in
//! order on the actor's task; the asynchronous sinks (the output log, the history) get
//! `try_send` with room for [`SINK_CAPACITY`] events, and what does not fit is counted
//! (`LogMsg::Dropped`), never waited for.
//!
//! STUB until Task 16: the synchronous half works; there are no sinks yet, so nothing is
//! dropped.

use std::sync::atomic::{AtomicU64, Ordering};

use hacivat_session::session::RunEvent;

/// The room of each asynchronous sink, in events.
pub const SINK_CAPACITY: usize = 10_000;

/// A synchronous auditor: O(1) per event, never blocks (it runs on the session's actor task).
pub trait SyncAuditor: Send {
    fn on_event(&mut self, ev: &RunEvent);
}

/// The chain of a run (the module docs).
#[derive(Default)]
pub struct AuditorChain {
    sync: Vec<Box<dyn SyncAuditor>>,
    dropped: AtomicU64,
}

impl std::fmt::Debug for AuditorChain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditorChain")
            .field("sync", &self.sync.len())
            .field("dropped", &self.dropped())
            .finish()
    }
}

impl AuditorChain {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a synchronous auditor; they see each event in the order they were added.
    pub fn push_sync(&mut self, auditor: Box<dyn SyncAuditor>) {
        self.sync.push(auditor);
    }

    /// Hands `ev` to every auditor.
    pub fn on_event(&mut self, ev: &RunEvent) {
        for auditor in &mut self.sync {
            auditor.on_event(ev);
        }
    }

    /// Events the asynchronous sinks had no room for.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}
