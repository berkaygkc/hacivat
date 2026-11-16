// ## m3b-t01c-core-seams
//! Where the classifier runs (R-M3-3, R-M3-53; Task 14): `sql_safety::classify` is pure and
//! synchronous, so the pool owns std threads with 16 MiB stacks and answers through a
//! `tokio::sync::oneshot`; the UI thread never classifies. It is also the read-only guard's
//! [`StatementClassifier`] (R-M3-83), installed by the app's boot from Task 18.
//!
//! STUB until Task 14 (fail closed, F31): `classify` answers on the caller's thread with
//! `sql-safety`'s stub (`Unknown`, `[NotImplemented]`); as a `StatementClassifier` it denies
//! every statement and knows no effects. Nothing installs it before Task 18.

use std::sync::Arc;

use futures::future::BoxFuture;
use hacivat_proto::DbmsKind;
use hacivat_session::session::{RoCheck, SafetyDenial, SessionEffects, StatementClassifier};
use sql_safety::{ClassifyCtx, ReasonCode, SafetyReport};
use tokio::sync::oneshot;

/// The classifier's threads (the module docs).
#[derive(Debug)]
pub struct ClassifyPool {
    threads: usize,
}

impl ClassifyPool {
    /// A pool of `threads` classifier threads (R-M3-53: 2). STUB: no thread yet.
    pub fn new(threads: usize) -> Arc<Self> {
        Arc::new(ClassifyPool {
            threads: threads.max(1),
        })
    }

    /// How many threads the pool runs.
    pub fn threads(&self) -> usize {
        self.threads
    }

    /// Classifies `sql` in `ctx` off the caller's thread; the report arrives on the receiver.
    /// STUB: `sql-safety`'s stub report, at once.
    pub fn classify(&self, sql: Arc<str>, ctx: ClassifyCtx) -> oneshot::Receiver<SafetyReport> {
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(sql_safety::classify(&sql, &ctx));
        rx
    }
}

/// The class Fluent ID of a statement the stub refuses.
const STUB_CLASS_ID: &str = "safety-class-unknown";

impl StatementClassifier for ClassifyPool {
    /// STUB until Task 14: `Deny` (not implemented), whatever the statement.
    fn check_read_only(
        &self,
        kind: DbmsKind,
        sql: Arc<str>,
        allow: Arc<[Arc<str>]>,
    ) -> BoxFuture<'static, RoCheck> {
        let _ = (kind, sql, allow);
        Box::pin(async {
            RoCheck::Deny(SafetyDenial {
                class_id: STUB_CLASS_ID,
                reasons: vec![ReasonCode::NotImplemented.fluent_id()],
            })
        })
    }

    /// STUB until Task 14: unknown (`None`): recovery falls back to its keyword scan.
    fn effects(&self, kind: DbmsKind, sql: Arc<str>) -> BoxFuture<'static, Option<SessionEffects>> {
        let _ = (kind, sql);
        Box::pin(async { None })
    }
}
