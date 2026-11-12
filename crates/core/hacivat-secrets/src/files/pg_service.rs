//! `pg_service.conf` reading, for the import (HV-SEC-03). Bodies in Task 8.

use std::io;
use std::path::{Path, PathBuf};

use secrecy::SecretString;

/// One `[service]` section (HV-SEC-03). The `password` key never lands in `fields`.
#[derive(Debug)]
pub struct PgService {
    pub name: String,
    /// 1-based line of the section header.
    pub line: u32,
    pub fields: Vec<(String, String)>,
    pub password: Option<SecretString>,
}

/// `PGSERVICEFILE`, then `~/.pg_service.conf`, then `PGSYSCONFDIR/pg_service.conf` (Task 8).
pub fn pg_service_path() -> Option<PathBuf> {
    None
}

/// Every section of `path` (Task 8).
pub fn read_pg_service(_path: &Path) -> io::Result<Vec<PgService>> {
    Ok(Vec::new())
}
