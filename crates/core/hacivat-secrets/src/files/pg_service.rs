//! `pg_service.conf` reading, for the import (HV-SEC-03).
//!
//! The format is libpq's `parseServiceFile`:
//! - each line is trimmed of trailing whitespace and leading whitespace; blank lines and lines
//!   starting with `#` are skipped;
//! - `[name]` starts a section (text after the `]` is ignored);
//! - `key=value` splits at the first `=`. Whitespace at the end of the key is trimmed, but the
//!   value is kept as written, so `password= x` gives ` x` (libpq does not trim it either).
//!
//! Departures, so an import can still list the rest of a file that libpq would reject: lines
//! before the first section, lines without `=` and a header without `]` are skipped.
//!
//! Every section is returned in file order, **duplicates included**: libpq uses the first
//! `[name]` it finds, so callers that dedupe keep the first. Within a section every field is
//! listed in order, and libpq likewise uses the first value of a repeated key.
//!
//! Passwords: the exact key `password` goes to [`PgService::password`] (the first one in the
//! section wins) and never into `fields`. Any other casing of it (`PASSWORD`, `Password`) is
//! dropped, since libpq does not accept it as a keyword, and it never lands in `fields` either.
//! Under the `.pgpass` permission rule, a file that group or other may access
//! (`mode & 0o077 != 0`, Unix only) has its passwords dropped, each with a
//! [`ServiceFileWarning`]; the other fields are kept.
//!
//! The file is opened once, checked through its handle and read up to 1 MiB, on every call;
//! nothing is cached. Its bytes are zeroized after use.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use secrecy::SecretString;
use zeroize::Zeroizing;

use super::{ReadError, home_dir, read_credential_file};

/// One `[service]` section (HV-SEC-03). The `password` key never lands in `fields`.
#[derive(Debug)]
pub struct PgService {
    pub name: String,
    /// 1-based line of the section header.
    pub line: u32,
    pub fields: Vec<(String, String)>,
    pub password: Option<SecretString>,
}

/// What [`read_pg_service`] read: the sections and why any password was left out (HV-SEC-03).
#[derive(Debug)]
pub struct PgServiceFile {
    pub services: Vec<PgService>,
    pub warnings: Vec<ServiceFileWarning>,
}

/// Something in a service file that was not imported (HV-SEC-03). Never carries a secret.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ServiceFileWarning {
    /// Group or other may access the file (`mode & 0o077 != 0`), so the password of `service`
    /// (at 1-based `line`) was dropped. One warning per section.
    PasswordIgnoredInsecureFile {
        path: PathBuf,
        mode: u32,
        service: String,
        line: u32,
    },
}

/// The key whose value is a secret.
const PASSWORD_KEY: &str = "password";

/// The service files libpq searches, in its order, that exist: the user file (`PGSERVICEFILE`;
/// otherwise `%APPDATA%\postgresql\.pg_service.conf` on Windows, `~/.pg_service.conf`
/// elsewhere), then the system file `PGSYSCONFDIR/pg_service.conf`. libpq looks a service up in
/// the user file first and reads the system file only when the user file lacks it.
///
/// Gap: without `PGSYSCONFDIR`, libpq falls back to its compiled-in `sysconfdir`, which varies
/// by build and is not searched here.
pub fn pg_service_paths() -> Vec<PathBuf> {
    let appdata = if cfg!(windows) {
        std::env::var_os("APPDATA")
    } else {
        None
    };
    pg_service_paths_from(
        std::env::var_os("PGSERVICEFILE"),
        appdata,
        home_dir().as_deref(),
        std::env::var_os("PGSYSCONFDIR"),
        |p| p.is_file(),
    )
}

/// The first of [`pg_service_paths`].
pub fn pg_service_path() -> Option<PathBuf> {
    pg_service_paths().into_iter().next()
}

/// [`pg_service_paths`] over explicit inputs: `appdata` is only passed on Windows.
fn pg_service_paths_from(
    servicefile: Option<OsString>,
    appdata: Option<OsString>,
    home: Option<&Path>,
    sysconfdir: Option<OsString>,
    exists: impl Fn(&Path) -> bool,
) -> Vec<PathBuf> {
    let user = servicefile
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            appdata
                .filter(|a| !a.is_empty())
                .map(|a| PathBuf::from(a).join("postgresql").join(".pg_service.conf"))
        })
        .or_else(|| home.map(|h| h.join(".pg_service.conf")));
    let system = sysconfdir
        .filter(|d| !d.is_empty())
        .map(|d| PathBuf::from(d).join("pg_service.conf"));
    [user, system]
        .into_iter()
        .flatten()
        .filter(|p| exists(p))
        .collect()
}

/// Every section of `path`, in file order, and the warnings (HV-SEC-03). Blocking: it reads
/// the file.
///
/// Errors: the open or read error (`NotFound` for a missing file); `InvalidInput` for a
/// directory, FIFO or device; `FileTooLarge` over 1 MiB.
pub fn read_pg_service(path: &Path) -> io::Result<PgServiceFile> {
    let file = read_credential_file(path).map_err(|e| match e {
        ReadError::Io(e) => e,
        ReadError::NotAFile => io::Error::from(io::ErrorKind::InvalidInput),
        ReadError::TooLarge => io::Error::from(io::ErrorKind::FileTooLarge),
    })?;
    let text = Zeroizing::new(String::from_utf8_lossy(&file.bytes).into_owned());
    let mut services: Vec<PgService> = Vec::new();
    let mut warnings = Vec::new();
    // Whether the current section has had a `password` line already.
    let mut password_seen = false;
    for (index, raw) in text.split('\n').enumerate() {
        let line_no = u32::try_from(index + 1).unwrap_or(u32::MAX);
        let line = raw.trim_ascii();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(header) = line.strip_prefix('[') {
            let Some(end) = header.find(']') else {
                continue;
            };
            services.push(PgService {
                name: header[..end].to_string(),
                line: line_no,
                fields: Vec::new(),
                password: None,
            });
            password_seen = false;
            continue;
        }
        let (Some(service), Some((key, value))) = (services.last_mut(), line.split_once('='))
        else {
            continue;
        };
        let key = key.trim_ascii_end();
        if key == PASSWORD_KEY {
            if password_seen {
                continue;
            }
            password_seen = true;
            match file.exposed_mode {
                None => service.password = Some(SecretString::from(value)),
                Some(mode) => warnings.push(ServiceFileWarning::PasswordIgnoredInsecureFile {
                    path: path.to_path_buf(),
                    mode,
                    service: service.name.clone(),
                    line: line_no,
                }),
            }
        } else if !key.eq_ignore_ascii_case(PASSWORD_KEY) {
            service.fields.push((key.to_string(), value.to_string()));
        }
    }
    Ok(PgServiceFile { services, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_user_file_then_the_system_file() {
        let home = Path::new("/home/u");
        let all = |_: &Path| true;
        let none = |_: &Path| false;
        let etc = || Some(OsString::from("/etc"));
        let sys = Path::new("/etc").join("pg_service.conf");
        assert_eq!(
            pg_service_paths_from(Some("/x/svc".into()), None, Some(home), etc(), all),
            vec![PathBuf::from("/x/svc"), sys.clone()]
        );
        assert_eq!(
            pg_service_paths_from(None, None, Some(home), etc(), all),
            vec![home.join(".pg_service.conf"), sys.clone()]
        );
        assert_eq!(
            pg_service_paths_from(None, Some("/ad".into()), Some(home), None, all),
            vec![Path::new("/ad").join("postgresql").join(".pg_service.conf")]
        );
        // A user file that does not exist is skipped, as libpq does.
        assert_eq!(
            pg_service_paths_from(Some("".into()), None, Some(home), etc(), |p: &Path| p
                .starts_with("/etc")),
            vec![sys]
        );
        assert!(
            pg_service_paths_from(Some("/x/svc".into()), None, Some(home), etc(), none).is_empty()
        );
        assert!(pg_service_paths_from(None, None, None, Some("".into()), all).is_empty());
    }
}
