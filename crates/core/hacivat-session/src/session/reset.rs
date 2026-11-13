//! Returning a session to a clean state between uses (HV-SESS-08; Task 7):
//! 1. `ROLLBACK` when a transaction is open;
//! 2. the driver's `RawConn::reset` (PostgreSQL: the `DISCARD ALL` sequence without
//!    `DEALLOCATE ALL`, Q10; MySQL: `COM_RESET_CONNECTION`; SQLite: nothing);
//! 3. a verify probe (PostgreSQL `SELECT current_user = session_user`, which must answer true);
//! 4. the environment plan again (identity, read-only setup, the last schema), and MySQL's
//!    `autocommit = 0` when the session starts Manual.
//!
//! Any failure breaks the session and closes its connection: a pool never reuses a session
//! whose reset failed (Q19: a borrowed default session is never reset at all). A reset does not
//! re-probe the schema; its environment plan restores the last one (Q27).

use hacivat_driver::{ExecOpts, ExecSummary, TxStatus};
use hacivat_proto::{DbmsKind, ErrClass};

use crate::session::actor::Core;
use crate::session::schema::{Family, family};
use crate::session::{ExecError, ExecErrorKind, Reply, TxMode, TxState};

/// The verify probe of `kind`, if it has one.
pub(crate) fn verify_sql(kind: DbmsKind) -> Option<&'static str> {
    (family(kind) == Family::Postgres).then_some("SELECT current_user = session_user")
}

/// Whether the verify probe answered true (fail closed: no row is not true).
fn verified(s: &ExecSummary) -> bool {
    s.rows
        .first()
        .and_then(|row| row.first())
        .and_then(Option::as_deref)
        .is_some_and(|v| ["t", "true", "1"].iter().any(|t| v.eq_ignore_ascii_case(t)))
}

impl Core {
    /// Resets the connection (the module docs); on failure the session is broken.
    pub(crate) async fn reset(&mut self) -> Reply {
        self.last_class = None;
        match self.reset_steps().await {
            Ok(()) => {
                self.tx = TxState {
                    mode: self.initial_mode,
                    isolation: self.initial_isolation,
                    ..TxState::default()
                };
                self.observe(false);
                Ok(self.summary())
            }
            Err(e) => {
                let class = self.last_class.unwrap_or(ErrClass::Internal);
                self.broken.get_or_insert(class);
                Err(e)
            }
        }
    }

    async fn reset_steps(&mut self) -> Result<(), ExecError> {
        if self.conn()?.tx_status() != TxStatus::Idle {
            self.control("ROLLBACK").await?;
        }
        let reset = self.conn()?.reset().await;
        self.observe(false);
        reset.map_err(|e| self.raw_error(&e, true))?;
        if let Some(sql) = verify_sql(self.kind) {
            let probe = self.conn()?.execute(sql, ExecOpts::default()).await;
            self.observe(false);
            let answer = probe.map_err(|e| self.raw_error(&e, true))?;
            if !verified(&answer) {
                tracing::warn!("the reset's role check did not answer true");
                return Err(ExecError::of(ExecErrorKind::Internal));
            }
        }
        for statement in self.env_plan().statements() {
            self.control(&statement).await?;
        }
        if self.initial_mode == TxMode::Manual && family(self.kind) == Family::MySql {
            self.control("SET autocommit = 0").await?;
        }
        Ok(())
    }
}
