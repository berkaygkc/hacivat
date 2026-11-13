//! Read-only hardening of a SQLite connection (Task 11, B21): `hacivat_guard::sqlite_hardening`
//! applied as `PRAGMA query_only`, `SQLITE_DBCONFIG_DEFENSIVE` and the attach and length
//! limits, and `hacivat_guard::sqlite_authorize` installed as the authorizer (rusqlite
//! `hooks`, `limits`). Empty until Task 11.
