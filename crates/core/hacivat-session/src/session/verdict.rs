// ## m3b-t01c-core-seams
//! What the exec gate tells the session about a unit, and the classifier the read-only guard
//! asks (R-M3-83; M3b Task 1c seams).
//!
//! - [`UnitVerdict`] and [`SessionEffects`] ride in every `RunUnit`: the session uses them for
//!   scheduling (suspend, Smart, the D-062 wrap, confirm-in-transaction; Task 19) and for C8's
//!   lost state (Task 18). **The read-only guard never trusts them for safety:** it asks the
//!   injected [`StatementClassifier`] again (`Core::step0b`, `alg/13` §0.1).
//! - [`StatementClassifier`] is injected through `ManagerConfig::classifier`, so this crate
//!   depends on neither `sql-safety` nor `pg_query`; `hacivat-exec::classify_pool` implements
//!   it (Task 14) and the app's boot installs it (Task 18). `None` keeps the M2b/M3a behaviour.
//!
//! P-M3b-1: `TxThenConfirm` carries no form; the session picks the savepoint form or its own
//! `BEGIN` at execution time from the live transaction state (Task 19).

use std::fmt;
use std::sync::Arc;

use futures::future::BoxFuture;
use hacivat_proto::DbmsKind;

/// What the exec gate decided for a unit, as far as the session schedules it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum UnitVerdict {
    Read,
    Write,
    /// Not classified (M3a's runs, and every run until Task 24 wires the gate).
    #[default]
    Unknown,
    /// Run it inside a transaction, then ask Commit/Roll back (or Keep/Undo in the savepoint
    /// form, which the session picks; P-M3b-1). Refused with `Internal` until Task 19.
    TxThenConfirm,
    /// `EXPLAIN ANALYZE` of a non-read statement, wrapped in a rollback (D-062). Refused with
    /// `Internal` until Task 19.
    ExplainAnalyzeRollback,
    /// A DDL on MySQL/MariaDB that the user confirmed although it commits the open
    /// transaction.
    ImplicitCommit,
}

bitflags::bitflags! {
    /// The session-level effects of a unit (C8: what a reconnect would lose; Smart: whether it
    /// opens a write).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct SessionEffects: u16 {
        /// A session variable or setting (`SET`, `USE`).
        const SESSION_STATE = 1;
        /// A temporary table or object.
        const WRITE_TEMP = 2;
        /// A session-level advisory lock.
        const ADVISORY_LOCK = 4;
        /// MySQL `LOCK TABLES`.
        const LOCK_TABLES = 8;
        /// A prepared statement.
        const PREPARE = 16;
        /// The unit may write (Smart opens a transaction before it).
        const OPENS_WRITE = 32;
        /// A DDL statement (MySQL/MariaDB commit an open transaction before it).
        const DDL = 64;
        // ## m3b-t15-gate (named exception, T15 fix round 2, R1-I1)
        /// MySQL/MariaDB/TiDB commit an open transaction before this statement (the gate's
        /// `hacivat_exec::implicit_commit` list: most DDL, privileges, `BEGIN`, `LOCK TABLES`,
        /// `SET autocommit = 1`, `FLUSH`, …). `DDL` keeps meaning "a DDL statement"; the
        /// session's implicit-commit backstop (Task 19) keys on this bit.
        const COMMITS_FIRST = 128;
    }
}

/// Why the classifier refused a statement on a read-only session: Fluent IDs only (never the
/// statement's text).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SafetyDenial {
    /// The statement class's Fluent ID.
    pub class_id: &'static str,
    /// The reasons' Fluent IDs.
    pub reasons: Vec<&'static str>,
}

/// The read-only check's answer.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RoCheck {
    Allow,
    Deny(SafetyDenial),
}

/// The classifier the read-only guard asks (R-M3-83): injected through
/// `ManagerConfig::classifier`; `hacivat-exec::classify_pool::ClassifyPool` implements it.
pub trait StatementClassifier: Send + Sync + fmt::Debug + 'static {
    /// Whether `sql` may run on a read-only session of `kind`; `allow` is the profile's
    /// function allowlist (P-M3b-10).
    fn check_read_only(
        &self,
        kind: DbmsKind,
        sql: Arc<str>,
        allow: Arc<[Arc<str>]>,
    ) -> BoxFuture<'static, RoCheck>;

    /// The session-level effects of `sql` (C8), `None` when unknown.
    fn effects(&self, kind: DbmsKind, sql: Arc<str>) -> BoxFuture<'static, Option<SessionEffects>>;

    // ## m3b-t18-safe06 (named exception: the plan amendment C2(b)/I1 extends the trait)
    /// [`StatementClassifier::check_read_only`] in the session's real context (`ctx`: its
    /// lexing-mode setting, search path and server version, as the read-only guard knows them),
    /// and whether `sql` may change that context (the guard then reads it again before the
    /// next check). The read-only guard's step 0b calls this one.
    ///
    /// The default ignores `ctx` (the context-free check, unknown modes) and answers that the
    /// context may have changed.
    fn check_read_only_in(
        &self,
        kind: DbmsKind,
        sql: Arc<str>,
        allow: Arc<[Arc<str>]>,
        ctx: ClassifierContext,
    ) -> BoxFuture<'static, RoAnswer> {
        let _ = ctx;
        let check = self.check_read_only(kind, sql, allow);
        Box::pin(async move {
            RoAnswer {
                check: check.await,
                changes_context: true,
            }
        })
    }
}

// ## m3b-t18-safe06 (plan amendment C2(b)/I1)
/// What the read-only guard knows of the session's context when it asks the classifier
/// ([`StatementClassifier::check_read_only_in`]): server values, verbatim, never parsed here
/// (the classifier side reads them as the console's SQL context does).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct ClassifierContext {
    /// The setting that decides how the server lexes: PostgreSQL
    /// `standard_conforming_strings` (`on`/`off`), MySQL/MariaDB `sql_mode`. `None`: unknown
    /// (the classifier then reads the text under every mode, fail-closed).
    pub lex_modes: Option<Arc<str>>,
    /// PostgreSQL: the session's search path, one schema per element as the server stores it
    /// (folded; `$user` literal). `None`: unknown, so an unqualified builtin is unknown too
    /// (fail-closed). MySQL and MariaDB: the current database, alone (the T20 carry: the
    /// classifier reads unqualified `performance_schema`/`sys` names by it); `None` when there is
    /// none or it was not read. SQLite: `None`.
    pub search_path: Option<Arc<[Arc<str>]>>,
    /// The server's version as the connect read it.
    pub server_version: Option<hacivat_driver::SemVerLoose>,
}

/// The read-only check's answer in context ([`StatementClassifier::check_read_only_in`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RoAnswer {
    pub check: RoCheck,
    /// The statement may change how later statements read (the search path, the lexing modes,
    /// a role, code run from a cursor or prepared statement): the guard reads the context again
    /// before its next check.
    pub changes_context: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effects_combine_and_print_their_names() {
        let e = SessionEffects::DDL | SessionEffects::OPENS_WRITE;
        assert!(e.contains(SessionEffects::DDL) && e.intersects(SessionEffects::OPENS_WRITE));
        assert!(!e.contains(SessionEffects::PREPARE));
        assert_eq!(
            format!("{e:?}"),
            "SessionEffects(OPENS_WRITE | DDL)",
            "names, never bits"
        );
        assert_eq!(
            SessionEffects::from_bits_truncate(u16::MAX),
            SessionEffects::all()
        );
        assert!(SessionEffects::default().is_empty());
        assert_eq!(UnitVerdict::default(), UnitVerdict::Unknown);
    }
}
