//! `Request::Run` (HV-EXEC-05; Task 18): typed units in order, streamed into result stores in
//! chunk steps (the actor drains its commands between chunks, R-M3-16), `StopOnError`
//! (R-M3-26), one result set kept per unit (R-M3-25), and the last suspendable unit kept as a
//! console cursor (R-M3-23). Read-only sessions route each unit through the typed guard path
//! (`readonly::guarded_query`, Task 22).
//!
//! M3a Task 1b stub: fails closed. A run is answered `ExecErrorKind::Internal` and nothing is
//! sent to the server.

use crate::session::actor::Core;
use crate::session::{ExecError, ExecErrorKind, Reply, RunSpec};

impl Core {
    /// Runs `spec`; the reply comes after its `RunEvent::RunEnded` (Task 18).
    pub(crate) async fn run(&mut self, spec: RunSpec) -> Reply {
        let _ = spec;
        Err(ExecError::of(ExecErrorKind::Internal))
    }
}
