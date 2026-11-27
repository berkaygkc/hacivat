//! Which backend and pre-scan the orchestrator runs for a dialect (`alg/13` SQL-SAFE-02 step 5;
//! Task 14).
//!
//! - PostgreSQL and the PostgreSQL-wire flavours: libpg_query through `sql-oracle-pg` (Task 9),
//!   every parser call on the oracle's one serial 16 MiB thread.
//! - MySQL, MariaDB and SQLite: the interim `sqlparser` path with the lexer pre-scan and the
//!   executable-comment threshold views (Task 10, R-M3-54).
//! - Every other dialect: no backend; the request is `Unknown` with `UnsupportedDialect`
//!   (fail closed; SQL Server, Oracle, ClickHouse and the generic dialect have no classifier
//!   before M5).

use smallvec::{SmallVec, smallvec};
use sql_dialect::DialectId;

use crate::backend::Backend;
use crate::backend::interim::{InterimBackend, PRESCAN, SQLPARSER};
use crate::backend::pg::{self, PgBackend};
use crate::report::BackendVersion;

/// The orchestrator's plan for one dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Plan {
    /// libpg_query (Task 9).
    Pg,
    /// `sqlparser` and the lexer pre-scan (Task 10).
    Interim,
    /// No backend: `UnsupportedDialect`.
    Unsupported,
}

/// The plan for `d`. `DialectId` is `#[non_exhaustive]`, so a dialect added later has no
/// backend until it is placed here (fail closed).
pub(crate) fn plan_for(d: DialectId) -> Plan {
    match d {
        DialectId::Postgres | DialectId::PgWire(_) => Plan::Pg,
        DialectId::MySql | DialectId::MariaDb | DialectId::Sqlite => Plan::Interim,
        _ => Plan::Unsupported,
    }
}

impl Plan {
    /// The backend this plan runs, `None` for [`Plan::Unsupported`].
    pub(crate) fn backend(self) -> Option<&'static dyn Backend> {
        match self {
            Plan::Pg => Some(&PgBackend),
            Plan::Interim => Some(&InterimBackend),
            Plan::Unsupported => None,
        }
    }

    /// The backends `SafetyReport.backends` records for this plan.
    pub(crate) fn versions(self) -> SmallVec<[BackendVersion; 3]> {
        match self {
            Plan::Pg => smallvec![pg::BACKEND],
            Plan::Interim => smallvec![SQLPARSER, PRESCAN],
            Plan::Unsupported => SmallVec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use sql_dialect::PgWireFlavor;

    use super::*;

    #[test]
    fn every_dialect_has_a_plan_and_only_four_have_a_backend() {
        let all = [
            (DialectId::Postgres, Plan::Pg),
            (DialectId::PgWire(PgWireFlavor::Redshift), Plan::Pg),
            (DialectId::PgWire(PgWireFlavor::Cockroach), Plan::Pg),
            (DialectId::PgWire(PgWireFlavor::Yugabyte), Plan::Pg),
            (DialectId::PgWire(PgWireFlavor::Timescale), Plan::Pg),
            (DialectId::PgWire(PgWireFlavor::Other), Plan::Pg),
            (DialectId::MySql, Plan::Interim),
            (DialectId::MariaDb, Plan::Interim),
            (DialectId::Sqlite, Plan::Interim),
            (DialectId::Generic, Plan::Unsupported),
            (DialectId::SqlServer, Plan::Unsupported),
            (DialectId::ClickHouse, Plan::Unsupported),
            (DialectId::Oracle, Plan::Unsupported),
        ];
        for (d, p) in all {
            assert_eq!(plan_for(d), p, "{d:?}");
            assert_eq!(p.backend().is_some(), p != Plan::Unsupported, "{d:?}");
            assert_eq!(p.versions().is_empty(), p == Plan::Unsupported, "{d:?}");
        }
        assert_eq!(
            Plan::Interim
                .versions()
                .iter()
                .map(|b| b.name)
                .collect::<Vec<_>>(),
            ["sqlparser", "lexer-prescan"]
        );
        assert_eq!(Plan::Pg.versions()[0].name, "libpg_query");
    }
}
