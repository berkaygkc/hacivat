//! MariaDB's lexer profile: MySQL's, plus `/*M! … */` executable comments (MySQL reads those
//! as ordinary comments). MariaDB's `sql_mode=ORACLE` grammar is not lexed differently in M3a
//! (the classifier denies it, `LexModes::mariadb_oracle_mode`).

use super::{ExecVendor, LexDialect};
use crate::id::DialectId;

pub(super) static MARIADB: LexDialect =
    super::mysql::family("mariadb", DialectId::MariaDb, ExecVendor::MariaDb);
