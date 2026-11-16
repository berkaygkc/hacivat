// ## m3b-t01c-core-seams
//! Smart mode (HV-EXEC-10, D-036; Task 19): Auto, but a unit whose effects open a write
//! (`SessionEffects::OPENS_WRITE`, from the exec gate) opens the user's transaction first,
//! through `manual_begin`.
//!
//! ## m3b-t19-confirm-tx (C2; ADR 0004 `m3b-t19`)
//! - Before a unit of a run whose verdict is not `Read`, while no transaction is open, the
//!   session opens one (owner `User`: the user chose Smart). A `Read` verdict never opens one;
//!   a unit without a verdict (`Unknown`, every M3a run) opens one, the safe direction. A
//!   `TxThenConfirm` unit then takes the savepoint form (`confirm.rs`).
//! - The user ends it (`COMMIT`, `ROLLBACK`, the badge); the mode stays Smart and the next
//!   non-`Read` unit opens again.
//! - Not opened: before the user's own transaction control (`BEGIN`, `COMMIT`, …), before an
//!   `EXPLAIN ANALYZE` that the D-062 wrap rolls back anyway, and on MySQL/MariaDB before a
//!   statement that commits an open transaction first (`SessionEffects::COMMITS_FIRST`: it
//!   would end the new transaction at once, and the implicit-commit backstop would refuse it).
//! - What opens it: PostgreSQL `BEGIN` (with the isolation level), SQLite `BEGIN IMMEDIATE`
//!   unless the statement reads (as Manual), MySQL/MariaDB `START TRANSACTION` (Smart keeps
//!   `autocommit` on, so nothing is open before it; one the user's `autocommit = 0` keeps open
//!   counts as open and is never started again).
//! - `TxCommand::SetMode(Smart)` is accepted like Auto (refused while a transaction is open;
//!   a Manual MySQL session turns `autocommit` back on), and a profile's Smart mode starts as
//!   Smart. `Request::Exec` of the user (no verdict) opens before any statement but the user's
//!   transaction control.

use crate::session::actor::Core;
use crate::session::schema::{Family, family};
use crate::session::tx::begin_sql;
use crate::session::verdict::{SessionEffects, UnitVerdict};
use crate::session::{RunUnit, TxMode};

impl Core {
    /// Whether Smart opens the user's transaction before `unit` (the module docs).
    /// `control`: the unit is the user's own transaction control.
    pub(crate) fn smart_opens_before(&self, unit: &RunUnit, control: bool) -> bool {
        self.tx.mode == TxMode::Smart
            && !control
            && !self.tx_open_now()
            && !matches!(
                unit.verdict,
                UnitVerdict::Read | UnitVerdict::ExplainAnalyzeRollback
            )
            && !(family(self.kind) == Family::MySql
                && unit.effects.contains(SessionEffects::COMMITS_FIRST))
    }

    /// What Smart sends to open the user's transaction before a statement whose first keyword
    /// is `keyword` (the module docs).
    pub(crate) fn smart_begin(&self, keyword: &str) -> Vec<String> {
        match family(self.kind) {
            Family::MySql => begin_sql(self.kind, self.tx.isolation, false),
            _ => self.manual_begin(keyword),
        }
    }
}
