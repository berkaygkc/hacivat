//! `.pgpass` lookup, libpq-exact (HV-SEC-03).
//!
//! The rules follow libpq's `passwordFromFile`:
//! - each line is `host:port:database:user:password`; trailing `\n` and `\r` are stripped,
//!   empty lines and lines starting with `#` are skipped;
//! - fields compare byte for byte (case-sensitive); a field that is exactly `*` matches anything;
//! - `\x` stands for `x` in every field, so `\:` and `\\` are a literal `:` and `\`;
//! - the first matching line wins;
//! - an empty host, or one of the default socket directories (`/tmp`, `/var/run/postgresql`,
//!   `/run/postgresql`), is looked up as `localhost` instead; any other socket path matches
//!   only its literal text;
//! - port 0 means the default, 5432; ports compare as text (`05432` is not `5432`);
//! - an empty database or user name matches no line, not even `*`;
//! - the password is the fifth field: it ends at the first unescaped `:`; an empty field is
//!   no password (`None`, B27);
//! - on Unix a file that group or other may access (`mode & 0o077 != 0`) is ignored with a
//!   warning, as is anything that is not a plain file or is over 1 MiB. A missing or
//!   unreadable file has no match.
//!
//! The file is re-read on every call; nothing is cached. Its bytes are zeroized after use.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use secrecy::SecretString;
use zeroize::Zeroizing;

use super::{
    MAX_CREDENTIAL_FILE_BYTES, ReadError, home_dir, read_credential_file, secret_from_bytes,
};

/// The `.pgpass` line that matched (HV-SEC-03).
#[derive(Debug)]
pub struct PgPassMatch {
    /// 1-based line number.
    pub line: u32,
    /// The user asked for; with none asked, the line's user (`None` when it is `*`).
    pub user: Option<String>,
    /// `None` when the line's password field is empty (B27, m2b-t06): libpq then sends no
    /// password, never an empty one.
    pub password: Option<SecretString>,
}

/// Why `.pgpass` was ignored (HV-SEC-03).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PgPassWarning {
    /// Group or other may read it (`mode & 0o077 != 0`). `mode` holds the permission bits.
    Permissions { path: PathBuf, mode: u32 },
    /// The path is a directory or another non-plain file (a FIFO, a device).
    IsDirectory(PathBuf),
    /// The file is over `limit` bytes ([`MAX_CREDENTIAL_FILE_BYTES`]) and was not read.
    TooLarge { path: PathBuf, limit: u64 },
}

/// The socket directories libpq builds commonly default to; a host equal to one of them is
/// looked up as `localhost`.
const DEFAULT_SOCKET_DIRS: [&str; 3] = ["/tmp", "/var/run/postgresql", "/run/postgresql"];

/// libpq's default port.
const DEFAULT_PORT: u16 = 5432;

/// `$PGPASSFILE`, then `%APPDATA%\postgresql\pgpass.conf` (Windows), then `~/.pgpass`. The path
/// need not exist.
pub fn pgpass_path() -> Option<PathBuf> {
    let appdata = if cfg!(windows) {
        std::env::var_os("APPDATA")
    } else {
        None
    };
    pgpass_path_from(
        std::env::var_os("PGPASSFILE"),
        appdata,
        home_dir().as_deref(),
    )
}

/// [`pgpass_path`] over explicit inputs: `appdata` is only passed on Windows.
fn pgpass_path_from(
    pgpassfile: Option<OsString>,
    appdata: Option<OsString>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(p) = pgpassfile.filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    if let Some(a) = appdata.filter(|a| !a.is_empty()) {
        return Some(PathBuf::from(a).join("postgresql").join("pgpass.conf"));
    }
    home.map(|h| h.join(".pgpass"))
}

/// The first line of `path` matching the endpoint (HV-SEC-03). `user: None` matches any user
/// and reports the line's. Blocking: it reads the file.
pub fn pgpass_lookup(
    path: &Path,
    host: &str,
    port: u16,
    database: &str,
    user: Option<&str>,
) -> Result<Option<PgPassMatch>, PgPassWarning> {
    if database.is_empty() || user == Some("") {
        // libpq: no password lookup without a database and a user name.
        return Ok(None);
    }
    let file = match read_credential_file(path) {
        Ok(file) => file,
        // libpq: a file it cannot open supplies no password.
        Err(ReadError::Io(_)) => return Ok(None),
        Err(ReadError::NotAFile) => return Err(PgPassWarning::IsDirectory(path.to_path_buf())),
        Err(ReadError::TooLarge) => {
            return Err(PgPassWarning::TooLarge {
                path: path.to_path_buf(),
                limit: MAX_CREDENTIAL_FILE_BYTES,
            });
        }
    };
    if let Some(mode) = file.exposed_mode {
        return Err(PgPassWarning::Permissions {
            path: path.to_path_buf(),
            mode,
        });
    }
    let bytes = file.bytes;
    let port = if port == 0 { DEFAULT_PORT } else { port }.to_string();
    let host = if host.is_empty() || DEFAULT_SOCKET_DIRS.contains(&host) {
        "localhost"
    } else {
        host
    };
    for (index, line) in bytes.split(|&b| b == b'\n').enumerate() {
        let line = trim_line_end(line);
        if line.is_empty() || line[0] == b'#' {
            continue;
        }
        let Some(rest) = match_field(line, host.as_bytes())
            .and_then(|r| match_field(r, port.as_bytes()))
            .and_then(|r| match_field(r, database.as_bytes()))
        else {
            continue;
        };
        let (line_user, rest) = match user {
            Some(u) => match match_field(rest, u.as_bytes()) {
                Some(rest) => (Some(u.to_string()), rest),
                None => continue,
            },
            None => match take_field(rest) {
                Some((field, rest)) => (field, rest),
                None => continue,
            },
        };
        return Ok(Some(PgPassMatch {
            line: u32::try_from(index + 1).unwrap_or(u32::MAX),
            user: line_user,
            password: unescape_secret(rest),
        }));
    }
    Ok(None)
}

/// The line without its trailing `\n` and `\r` bytes.
fn trim_line_end(mut line: &[u8]) -> &[u8] {
    while let [head @ .., b'\r' | b'\n'] = line {
        line = head;
    }
    line
}

/// libpq's `pwdfMatchesString`: when the field at the start of `buf` matches `token` (a whole
/// `*` field matches anything), the text after its `:`.
fn match_field<'a>(buf: &'a [u8], token: &[u8]) -> Option<&'a [u8]> {
    if let [b'*', b':', rest @ ..] = buf {
        return Some(rest);
    }
    let mut i = 0;
    let mut t = 0;
    while i < buf.len() {
        let escaped = buf[i] == b'\\';
        if escaped {
            i += 1;
        }
        let c = buf.get(i).copied();
        if c == Some(b':') && !escaped && t == token.len() {
            return Some(&buf[i + 1..]);
        }
        if t == token.len() || c != Some(token[t]) {
            return None;
        }
        i += 1;
        t += 1;
    }
    None
}

/// The de-escaped user field at the start of `buf` and the text after its `:`. A `*` field
/// gives `None` as the user.
fn take_field(buf: &[u8]) -> Option<(Option<String>, &[u8])> {
    if let [b'*', b':', rest @ ..] = buf {
        return Some((None, rest));
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < buf.len() {
        let escaped = buf[i] == b'\\' && i + 1 < buf.len();
        if escaped {
            i += 1;
        } else if buf[i] == b':' {
            return Some((
                Some(String::from_utf8_lossy(&out).into_owned()),
                &buf[i + 1..],
            ));
        }
        out.push(buf[i]);
        i += 1;
    }
    None
}

/// The password field, libpq's de-escape loop: it ends at the first unescaped `:` or the end
/// of the line, and `\x` reads as `x` (a lone trailing `\` is kept). An empty field is `None`
/// (B27).
fn unescape_secret(buf: &[u8]) -> Option<SecretString> {
    let mut out = Zeroizing::new(Vec::with_capacity(buf.len()));
    let mut i = 0;
    while i < buf.len() && buf[i] != b':' {
        if buf[i] == b'\\' && i + 1 < buf.len() {
            i += 1;
        }
        out.push(buf[i]);
        i += 1;
    }
    (!out.is_empty()).then(|| secret_from_bytes(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pgpassfile_wins_then_appdata_then_home() {
        let home = Path::new("/home/u");
        assert_eq!(
            pgpass_path_from(Some("/x/pw".into()), Some("C:\\AD".into()), Some(home)),
            Some(PathBuf::from("/x/pw"))
        );
        assert_eq!(
            pgpass_path_from(None, Some("/ad".into()), Some(home)),
            Some(Path::new("/ad").join("postgresql").join("pgpass.conf"))
        );
        assert_eq!(
            pgpass_path_from(Some("".into()), None, Some(home)),
            Some(PathBuf::from("/home/u/.pgpass"))
        );
        assert_eq!(pgpass_path_from(None, Some("".into()), None), None);
    }

    #[test]
    fn field_matching_follows_libpq() {
        assert_eq!(match_field(b"*:rest", b"anything"), Some(&b"rest"[..]));
        assert_eq!(match_field(b"abc:rest", b"abc"), Some(&b"rest"[..]));
        assert_eq!(match_field(b"abc:rest", b"ab"), None);
        assert_eq!(match_field(b"ab:rest", b"abc"), None);
        assert_eq!(match_field(b"a\\:b:rest", b"a:b"), Some(&b"rest"[..]));
        assert_eq!(match_field(b"a\\\\:rest", b"a\\"), Some(&b"rest"[..]));
        assert_eq!(match_field(b"abc", b"abc"), None);
        assert_eq!(match_field(b"*", b"x"), None);
        assert_eq!(match_field(b"\\", b""), None);
        assert_eq!(match_field(b":rest", b""), Some(&b"rest"[..]));
    }

    #[test]
    fn take_field_unescapes_and_reports_a_wildcard_as_none() {
        assert_eq!(
            take_field(b"a\\:b:rest"),
            Some((Some("a:b".to_string()), &b"rest"[..]))
        );
        assert_eq!(take_field(b"*:rest"), Some((None, &b"rest"[..])));
        assert_eq!(take_field(b"no-colon"), None);
    }
}
