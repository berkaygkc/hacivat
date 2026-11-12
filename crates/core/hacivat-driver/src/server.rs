//! Server detection: loose versions, what a server says it is, and whether Hacivat supports it
//! (HV-DRV-01, C10). Everything here is locale-invariant: ASCII digits only, ASCII case folding.

use hacivat_proto::DbmsKind;

use crate::catalog::EngineCatalog;

/// A version parsed loosely, keeping the raw string (`16.15 (Debian ...)`) (HV-DRV-01).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SemVerLoose {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub raw: String,
}

/// MariaDB before 11 prefixes its handshake version with this, for old MySQL clients.
const MARIADB_HANDSHAKE_PREFIX: &str = "5.5.5-";

impl SemVerLoose {
    /// The first `\d+(\.\d+){0,2}` run of ASCII digits in `s`, after stripping a `5.5.5-`
    /// handshake prefix; missing parts are `0`. `None` when there is no such run, or a part
    /// does not fit in a `u32`.
    pub fn parse(s: &str) -> Option<Self> {
        let body = s.strip_prefix(MARIADB_HANDSHAKE_PREFIX).unwrap_or(s);
        let bytes = body.as_bytes();
        let start = bytes.iter().position(u8::is_ascii_digit)?;
        let mut parts = [0u32; 3];
        let mut i = start;
        for (n, part) in parts.iter_mut().enumerate() {
            if n > 0 {
                // A part continues only after a `.` followed by a digit.
                if bytes.get(i) != Some(&b'.') || !bytes.get(i + 1).is_some_and(u8::is_ascii_digit)
                {
                    break;
                }
                i += 1;
            }
            let end = bytes[i..]
                .iter()
                .position(|b| !b.is_ascii_digit())
                .map_or(bytes.len(), |p| i + p);
            *part = body[i..end].parse().ok()?;
            i = end;
        }
        Some(SemVerLoose {
            major: parts[0],
            minor: parts[1],
            patch: parts[2],
            raw: s.to_string(),
        })
    }

    /// The first `n` parts, padded with zeros.
    fn prefix(&self, n: usize) -> [u32; 3] {
        let mut p = [self.major, self.minor, self.patch];
        for part in p.iter_mut().skip(n) {
            *part = 0;
        }
        p
    }
}

/// What the server reported after connecting (HV-DRV-01).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerInfo {
    pub kind: DbmsKind,
    pub product: String,
    pub version: Option<SemVerLoose>,
    pub raw_version: String,
    pub is_replica: Option<bool>,
}

/// Whether a server version is supported (C10).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Support {
    Supported,
    /// Above the floor but not in the tested list: connect with a warning.
    NotTested,
    /// Below the floor: `server.unsupported_version`.
    Unsupported {
        floor: String,
    },
}

/// A catalog version (`"14"`, `"8.0"`) and how many parts it names.
fn catalog_version(v: &str) -> Option<([u32; 3], usize)> {
    let parsed = SemVerLoose::parse(v)?;
    let parts = v.split('.').count().min(3);
    Some((parsed.prefix(parts), parts))
}

/// The support status of `info` in `catalog` (C10).
///
/// A catalog version is compared at its own precision: the floor `14` compares majors, the
/// floor `8.0` and the tested `10.11` compare major and minor. An engine with no floor (SQLite)
/// is `Supported` (P4). On an engine with a floor, a missing version is `NotTested`, and so is an
/// engine the catalog does not describe.
pub fn support_status(catalog: &EngineCatalog, info: &ServerInfo) -> Support {
    let Some(spec) = catalog.get(info.kind) else {
        return Support::NotTested;
    };
    let Some(floor) = spec.floor.as_deref() else {
        return Support::Supported;
    };
    let Some(version) = info.version.as_ref() else {
        return Support::NotTested;
    };
    if let Some((f, parts)) = catalog_version(floor)
        && version.prefix(parts) < f
    {
        return Support::Unsupported {
            floor: floor.to_string(),
        };
    }
    let tested = spec
        .tested
        .iter()
        .filter_map(|t| catalog_version(t))
        .any(|(t, parts)| version.prefix(parts) == t);
    if tested {
        Support::Supported
    } else {
        Support::NotTested
    }
}

/// `needle` occurs in `hay`, ignoring ASCII case only.
fn contains_ascii_ci(hay: &str, needle: &str) -> bool {
    hay.as_bytes()
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

/// MySQL, MariaDB or TiDB from `@@version` and `@@version_comment` (HV-DRV-01 fork
/// detection): `MariaDB` in either string wins, then `TiDB`, else MySQL.
pub fn detect_mysql_family(version: &str, version_comment: &str) -> DbmsKind {
    let says =
        |word: &str| contains_ascii_ci(version, word) || contains_ascii_ci(version_comment, word);
    if says("mariadb") {
        DbmsKind::MariaDb
    } else if says("tidb") {
        DbmsKind::TiDb
    } else {
        DbmsKind::MySql
    }
}
