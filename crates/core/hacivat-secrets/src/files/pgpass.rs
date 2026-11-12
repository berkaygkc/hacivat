//! `.pgpass` lookup, libpq-exact (HV-SEC-03). Bodies in Task 8.

use std::path::{Path, PathBuf};

use secrecy::SecretString;

/// The `.pgpass` line that matched (HV-SEC-03).
#[derive(Debug)]
pub struct PgPassMatch {
    /// 1-based line number.
    pub line: u32,
    pub user: Option<String>,
    pub password: SecretString,
}

/// Why `.pgpass` was ignored (HV-SEC-03).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PgPassWarning {
    /// Group or other may read it (`mode & 0o077 != 0`).
    Permissions {
        path: PathBuf,
        mode: u32,
    },
    IsDirectory(PathBuf),
}

/// `$PGPASSFILE`, then `%APPDATA%\postgresql\pgpass.conf`, then `~/.pgpass` (Task 8).
pub fn pgpass_path() -> Option<PathBuf> {
    None
}

/// The first line of `path` matching the endpoint (Task 8).
pub fn pgpass_lookup(
    _path: &Path,
    _host: &str,
    _port: u16,
    _database: &str,
    _user: Option<&str>,
) -> Result<Option<PgPassMatch>, PgPassWarning> {
    Ok(None)
}
