//! `pg_service.conf` reading, for the import (HV-SEC-03).
//!
//! The format is libpq's: `[name]` starts a section, `key=value` lines fill it, and blank lines
//! and lines starting with `#` are skipped. Leading and trailing whitespace is trimmed from each
//! line, and around each key and value. Lines before the first section, and lines without `=`,
//! are skipped rather than failing the whole file, so an import can still list the rest.
//!
//! The `password` key (exact case, as libpq's keywords are) goes to [`PgService::password`],
//! never into `fields`. Under the `.pgpass` permission rule, a file that group or other may
//! access (`mode & 0o077 != 0`, Unix only) has its passwords dropped; the other fields are kept.
//!
//! The file is re-read on every call; nothing is cached. Its bytes are zeroized after use.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use secrecy::SecretString;
use zeroize::Zeroizing;

use super::{FileCheck, check_file, home_dir};

/// One `[service]` section (HV-SEC-03). The `password` key never lands in `fields`.
#[derive(Debug)]
pub struct PgService {
    pub name: String,
    /// 1-based line of the section header.
    pub line: u32,
    pub fields: Vec<(String, String)>,
    pub password: Option<SecretString>,
}

/// The key whose value is a secret.
const PASSWORD_KEY: &str = "password";

/// `PGSERVICEFILE`, then `~/.pg_service.conf`, then `PGSYSCONFDIR/pg_service.conf`. The last two
/// are used only when they exist; `PGSERVICEFILE` is taken as given.
pub fn pg_service_path() -> Option<PathBuf> {
    pg_service_path_from(
        std::env::var_os("PGSERVICEFILE"),
        home_dir().as_deref(),
        std::env::var_os("PGSYSCONFDIR"),
        |p| p.is_file(),
    )
}

/// [`pg_service_path`] over explicit inputs.
fn pg_service_path_from(
    servicefile: Option<OsString>,
    home: Option<&Path>,
    sysconfdir: Option<OsString>,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if let Some(p) = servicefile.filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let user = home.map(|h| h.join(".pg_service.conf"));
    let system = sysconfdir
        .filter(|d| !d.is_empty())
        .map(|d| PathBuf::from(d).join("pg_service.conf"));
    [user, system].into_iter().flatten().find(|p| exists(p))
}

/// Every section of `path`, in file order (HV-SEC-03). Blocking: it reads the file.
pub fn read_pg_service(path: &Path) -> io::Result<Vec<PgService>> {
    let keep_passwords = check_file(path) == FileCheck::Private;
    let bytes = Zeroizing::new(std::fs::read(path)?);
    let text = Zeroizing::new(String::from_utf8_lossy(&bytes).into_owned());
    let mut services: Vec<PgService> = Vec::new();
    for (index, raw) in text.split('\n').enumerate() {
        let line = raw.trim_ascii();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(header) = line.strip_prefix('[') {
            let Some(end) = header.find(']') else {
                continue;
            };
            services.push(PgService {
                name: header[..end].trim_ascii().to_string(),
                line: u32::try_from(index + 1).unwrap_or(u32::MAX),
                fields: Vec::new(),
                password: None,
            });
            continue;
        }
        let (Some(service), Some((key, value))) = (services.last_mut(), line.split_once('='))
        else {
            continue;
        };
        let (key, value) = (key.trim_ascii(), value.trim_ascii());
        if key == PASSWORD_KEY {
            if keep_passwords {
                service.password = Some(SecretString::from(value));
            }
        } else {
            service.fields.push((key.to_string(), value.to_string()));
        }
    }
    Ok(services)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn servicefile_wins_then_the_user_file_then_the_system_file() {
        let home = Path::new("/home/u");
        let all = |_: &Path| true;
        let none = |_: &Path| false;
        assert_eq!(
            pg_service_path_from(Some("/x/svc".into()), Some(home), Some("/etc".into()), none),
            Some(PathBuf::from("/x/svc"))
        );
        assert_eq!(
            pg_service_path_from(None, Some(home), Some("/etc".into()), all),
            Some(PathBuf::from("/home/u/.pg_service.conf"))
        );
        assert_eq!(
            pg_service_path_from(
                Some("".into()),
                Some(home),
                Some("/etc".into()),
                |p: &Path| { p.starts_with("/etc") }
            ),
            Some(Path::new("/etc").join("pg_service.conf"))
        );
        assert_eq!(
            pg_service_path_from(None, Some(home), Some("/etc".into()), none),
            None
        );
        assert_eq!(pg_service_path_from(None, None, None, all), None);
    }
}
