//! Family detection and probes: MySQL, MariaDB, TiDB, read-only, wait timeout, local infile
//! (HV-DRV-01, HV-CONN-07). Values are read as bytes or numbers and parsed with ASCII rules
//! only; nothing depends on the server's message language.

use hacivat_driver::{
    ProbeWarning, RawError, RawErrorKind, SemVerLoose, ServerInfo, detect_mysql_family,
};
use hacivat_net::{TlsMode, TlsSummary, TlsVerified};
use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Row, Value};

use crate::connect::raw_error;
use crate::tls::verified;

/// The server variables detection and the probe need.
const VARS: &str = "SELECT @@version, @@version_comment, @@read_only, @@wait_timeout, \
                    @@local_infile, @@super_read_only";
/// The same without `super_read_only`, which MariaDB does not have.
const VARS_MARIADB: &str =
    "SELECT @@version, @@version_comment, @@read_only, @@wait_timeout, @@local_infile";
/// `ER_UNKNOWN_SYSTEM_VARIABLE`.
const UNKNOWN_SYSTEM_VARIABLE: u16 = 1193;

/// Schemas every server has; never offered as a database to open.
const SYSTEM_SCHEMAS: [&str; 4] = ["information_schema", "mysql", "performance_schema", "sys"];

/// MariaDB before 11 prefixes its handshake version with this, for old MySQL clients.
const MARIADB_HANDSHAKE_PREFIX: &str = "5.5.5-";

pub(crate) struct ServerVars {
    pub version: String,
    pub comment: String,
    pub read_only: Option<i64>,
    pub super_read_only: Option<i64>,
    pub wait_timeout: Option<i64>,
    pub local_infile: Option<i64>,
}

fn text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::Bytes(b) => Some(String::from_utf8_lossy(b).into_owned()),
        Value::Int(i) => Some(i.to_string()),
        Value::UInt(u) => Some(u.to_string()),
        _ => None,
    }
}

/// An integer or boolean variable: `1`, `0`, `ON`, `OFF` (ASCII case ignored).
pub(crate) fn int(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Int(i) => Some(*i),
        Value::UInt(u) => i64::try_from(*u).ok(),
        Value::Bytes(b) => {
            let s = std::str::from_utf8(b).ok()?.trim();
            if s.eq_ignore_ascii_case("on") {
                Some(1)
            } else if s.eq_ignore_ascii_case("off") {
                Some(0)
            } else {
                s.parse().ok()
            }
        }
        _ => None,
    }
}

async fn first_row(conn: &mut Conn, sql: &str) -> Result<Option<Row>, mysql_async::Error> {
    conn.query_first::<Row, _>(sql).await
}

/// Reads [`ServerVars`]; on MariaDB `super_read_only` is unknown and left `None`.
pub(crate) async fn server_vars(conn: &mut Conn) -> Result<ServerVars, RawError> {
    let (row, has_super) = match first_row(conn, VARS).await {
        Ok(row) => (row, true),
        Err(mysql_async::Error::Server(e)) if e.code == UNKNOWN_SYSTEM_VARIABLE => (
            first_row(conn, VARS_MARIADB)
                .await
                .map_err(|e| raw_error(&e))?,
            false,
        ),
        Err(e) => return Err(raw_error(&e)),
    };
    let row = row.ok_or(RawError {
        kind: RawErrorKind::Protocol,
        server: None,
    })?;
    let at = |i: usize| row.as_ref(i);
    Ok(ServerVars {
        version: text(at(0)).unwrap_or_default(),
        comment: text(at(1)).unwrap_or_default(),
        read_only: int(at(2)),
        wait_timeout: int(at(3)),
        local_infile: int(at(4)),
        super_read_only: if has_super { int(at(5)) } else { None },
    })
}

/// A version string without MariaDB's `5.5.5-` handshake prefix (removed only when a digit
/// follows it).
#[doc(hidden)]
pub fn server_version_text(v: &str) -> &str {
    match v.strip_prefix(MARIADB_HANDSHAKE_PREFIX) {
        Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => rest,
        _ => v,
    }
}

pub(crate) fn server_info(vars: &ServerVars) -> ServerInfo {
    let raw = server_version_text(&vars.version);
    let kind = detect_mysql_family(raw, &vars.comment);
    ServerInfo {
        kind,
        product: kind.display_name().to_owned(),
        version: SemVerLoose::parse(raw),
        raw_version: raw.to_owned(),
        is_replica: None,
    }
}

/// The probe's findings from the server variables (HV-CONN-07); an unknown value reports
/// nothing.
#[doc(hidden)]
pub fn probe_warnings(
    read_only: Option<i64>,
    super_read_only: Option<i64>,
    wait_timeout: Option<i64>,
    local_infile: Option<i64>,
) -> Vec<ProbeWarning> {
    let mut out = Vec::new();
    if read_only == Some(1) {
        out.push(ProbeWarning::ServerReadOnly);
    }
    if super_read_only == Some(1) {
        out.push(ProbeWarning::SuperReadOnly);
    }
    if let Some(seconds) = wait_timeout
        .filter(|w| *w < 60)
        .and_then(|w| u32::try_from(w).ok())
    {
        out.push(ProbeWarning::LowWaitTimeout { seconds });
    }
    if local_infile == Some(1) {
        out.push(ProbeWarning::LocalInfileOn);
    }
    out
}

/// `SHOW DATABASES` without the system schemas, in server order, at most `limit`.
#[doc(hidden)]
pub fn user_databases(names: Vec<String>, limit: usize) -> Vec<String> {
    names
        .into_iter()
        .filter(|n| !SYSTEM_SCHEMAS.iter().any(|s| s.eq_ignore_ascii_case(n)))
        .take(limit)
        .collect()
}

pub(crate) async fn list_databases(conn: &mut Conn, limit: usize) -> Result<Vec<String>, RawError> {
    let rows: Vec<Row> = conn
        .query("SHOW DATABASES")
        .await
        .map_err(|e| raw_error(&e))?;
    let names = rows.iter().filter_map(|r| text(r.as_ref(0))).collect();
    Ok(user_databases(names, limit))
}

/// The account the server authenticated, without its host part (`hv_test@%` gives `hv_test`).
pub(crate) async fn current_user(conn: &mut Conn) -> Result<Option<String>, RawError> {
    let row = first_row(conn, "SELECT CURRENT_USER()")
        .await
        .map_err(|e| raw_error(&e))?;
    Ok(row
        .and_then(|r| text(r.as_ref(0)))
        .map(|u| match u.rsplit_once('@') {
            Some((user, _host)) => user.to_owned(),
            None => u,
        }))
}

/// A session status value; `Ok(None)` when the server reports it empty, `Err` when the query
/// failed.
async fn session_status(conn: &mut Conn, sql: &str) -> Result<Option<String>, ()> {
    let row = first_row(conn, sql).await.map_err(|_| ())?;
    Ok(row
        .and_then(|r| text(r.as_ref(1)))
        .filter(|v| !v.is_empty()))
}

/// The summary for a connection whose TLS ran in `mode` (`Disable` when plaintext).
/// `encrypted` is the server's answer when there is one (`Ssl_version` is empty on a plaintext
/// session), else what was negotiated.
pub(crate) fn summary(
    mode: TlsMode,
    answered: Option<bool>,
    version: Option<String>,
    cipher: Option<String>,
) -> TlsSummary {
    let encrypted = answered.unwrap_or(mode != TlsMode::Disable);
    TlsSummary {
        encrypted,
        version,
        cipher,
        verified: if encrypted {
            verified(mode)
        } else {
            TlsVerified::No
        },
    }
}

/// The session's TLS from `Ssl_version` and `Ssl_cipher`.
pub(crate) async fn tls_summary(conn: &mut Conn, mode: TlsMode) -> TlsSummary {
    let version = session_status(conn, "SHOW SESSION STATUS LIKE 'Ssl_version'").await;
    let cipher = session_status(conn, "SHOW SESSION STATUS LIKE 'Ssl_cipher'").await;
    let answered = version.as_ref().ok().map(Option::is_some);
    summary(
        mode,
        answered,
        version.ok().flatten(),
        cipher.ok().flatten(),
    )
}
