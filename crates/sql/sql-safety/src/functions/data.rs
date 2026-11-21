//! The loader of `data/<engine>/{allow,deny,operators,types,set_allow}.toml`, compiled in with
//! `include_str!` (Task 3), and the offline freshness digest of `data/functions.digest`.
//!
//! Every file is `rows = [ { name = …, source = …, reviewed = true|false, since = "16",
//! until = "18", group = "Sleep" }, … ]`: `since` is inclusive, `until` exclusive, both
//! optional; `group` only in `deny.toml`. `allow.toml` also records `probe_floor`, the oldest
//! server the catalog read saw: a row without `since` holds only from there on (`Gate::holds`).
//! Only `reviewed = true` rows count as allowed, known
//! operators, known cast targets or harmless `SET`s; a deny row denies whatever its review
//! state (fail closed). A file that does not parse loads as empty, which also fails closed:
//! every lookup answers `Unknown`/`false` (`compiled_tables_parse` keeps that from shipping).
//!
//! The digest (`cargo xtask gen-fn-catalog --check`, R1-M3): XXH3-64 with seed 0 over, for each
//! file of [`DIGEST_FILES`] in order, its path, `\n`, its byte length in decimal, `\n`, and its
//! bytes with CRLF read as LF; written `xxh3-64:<16 hex digits>`. `xtask/src/gen_fn_catalog.rs`
//! keeps its own copy of the list and the algorithm (xtask takes no SQL crate); its test
//! compares the two lists between the markers below.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use serde::Deserialize;
use sql_dialect::ServerVersion;
use twox_hash::XxHash3_64;

use super::DenyGroup;

macro_rules! data_files {
    ($($path:literal),* $(,)?) => {
        /// The files the digest covers, in digest order (relative to `data/`).
        pub const DIGEST_FILES: &[&str] = &[$($path),*];
        /// Their compiled-in text, in the same order.
        const TEXTS: &[&str] = &[$(include_str!(concat!("../../data/", $path))),*];
    };
}

// digest-files:begin
data_files!(
    "pg/allow.toml",
    "pg/deny.toml",
    "pg/operators.toml",
    "pg/types.toml",
    "pg/set_allow.toml",
    "mysql/manual-index.txt",
    "mysql/allow.toml",
    "mysql/deny.toml",
    "mysql/operators.toml",
    "mysql/types.toml",
    "mysql/set_allow.toml",
    "mariadb/allow.toml",
    "mariadb/deny.toml",
    "mariadb/operators.toml",
    "mariadb/types.toml",
    "mariadb/set_allow.toml",
    "sqlite/allow.toml",
    "sqlite/deny.toml",
    "sqlite/operators.toml",
    "sqlite/types.toml",
    "sqlite/set_allow.toml",
);
// digest-files:end

/// The digest of `files` (`(path, bytes)` in order): see the module docs.
pub fn digest_of_files<'a>(files: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> String {
    let mut stream = Vec::new();
    for (path, bytes) in files {
        let text = lf(bytes);
        stream.extend_from_slice(path.as_bytes());
        stream.push(b'\n');
        stream.extend_from_slice(text.len().to_string().as_bytes());
        stream.push(b'\n');
        stream.extend_from_slice(&text);
    }
    format!("xxh3-64:{:016x}", XxHash3_64::oneshot(&stream))
}

/// `bytes` with every CRLF read as LF.
fn lf(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            continue;
        }
        out.push(b);
    }
    out
}

/// The digest of the compiled-in data.
pub fn compiled_digest() -> String {
    digest_of_files(
        DIGEST_FILES
            .iter()
            .zip(TEXTS)
            .map(|(p, t)| (*p, t.as_bytes())),
    )
}

/// The compiled-in text of `path` (one of [`DIGEST_FILES`]).
fn text(path: &str) -> &'static str {
    DIGEST_FILES
        .iter()
        .position(|p| *p == path)
        .and_then(|i| TEXTS.get(i))
        .copied()
        .unwrap_or("")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    /// `allow.toml` only: the oldest server version the catalog read saw (the probe floor).
    probe_floor: Option<String>,
    rows: Vec<RawRow>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRow {
    name: String,
    since: Option<String>,
    until: Option<String>,
    source: String,
    reviewed: bool,
    group: Option<String>,
}

/// The server versions a row holds for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Gate {
    since: Option<ServerVersion>,
    until: Option<ServerVersion>,
}

impl Gate {
    /// Whether the row holds at `version` on an engine whose catalog read saw nothing older
    /// than `floor`. A row without `since` is proven only from the floor on; a hand `since`
    /// below the floor comes from the engine's change history. An unknown version holds only
    /// for a row that holds on every server from the floor on. A builtin missing from the
    /// server could be a user function of the same name, so anything unproven is refused. No
    /// floor (a data file without one): nothing holds.
    pub(crate) fn holds(
        &self,
        version: Option<ServerVersion>,
        floor: Option<ServerVersion>,
    ) -> bool {
        let Some(floor) = floor else {
            return false;
        };
        let since = self.since.unwrap_or(floor);
        match version {
            None => since <= floor && self.until.is_none(),
            Some(v) => since <= v && self.until.is_none_or(|u| v < u),
        }
    }

    fn is_open(&self) -> bool {
        self.since.is_none() && self.until.is_none()
    }
}

/// One engine's lists.
#[derive(Debug, Default)]
pub(crate) struct Engine {
    /// The oldest server version the catalog read saw (`allow.toml`'s `probe_floor`).
    pub(crate) floor: Option<ServerVersion>,
    /// Reviewed builtin functions, by the name the server resolves (PostgreSQL: folded; the
    /// others: ASCII lower case).
    pub(crate) allow: HashMap<String, Gate>,
    /// Denied names, ASCII lower case.
    pub(crate) deny: HashMap<String, DenyGroup>,
    /// Reviewed, ungated builtin operators (word operators ASCII lower case).
    pub(crate) operators: HashSet<String>,
    /// Reviewed, ungated builtin cast targets, ASCII lower case.
    pub(crate) types: HashSet<String>,
    /// Reviewed harmless `SET` variables, ASCII lower case.
    pub(crate) set_allow: HashSet<String>,
}

/// Every engine's lists.
#[derive(Debug, Default)]
pub(crate) struct Tables {
    pub(crate) pg: Engine,
    pub(crate) mysql: Engine,
    pub(crate) mariadb: Engine,
    pub(crate) sqlite: Engine,
}

/// The compiled-in lists, parsed once.
pub(crate) fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| Tables {
        pg: engine("pg").unwrap_or_default(),
        mysql: engine("mysql").unwrap_or_default(),
        mariadb: engine("mariadb").unwrap_or_default(),
        sqlite: engine("sqlite").unwrap_or_default(),
    })
}

/// `"16"`, `"10.11"`, `"8.0.26"` as a version.
fn version(s: &str) -> Result<ServerVersion, String> {
    let parts: Vec<u16> = s
        .split('.')
        .map(|p| p.parse::<u16>())
        .collect::<Result<_, _>>()
        .map_err(|_| format!("bad version {s:?}"))?;
    match parts[..] {
        [major] => Ok(ServerVersion::new(major, 0, 0)),
        [major, minor] => Ok(ServerVersion::new(major, minor, 0)),
        [major, minor, patch] => Ok(ServerVersion::new(major, minor, patch)),
        _ => Err(format!("bad version {s:?}")),
    }
}

/// A file's rows with their gates.
type Rows = Vec<(RawRow, Gate)>;

/// The rows of `path`; `allow.toml` must carry its probe floor and no other file may.
fn rows(path: &str) -> Result<Rows, String> {
    rows_and_floor(path).and_then(|(rows, floor)| match floor {
        None => Ok(rows),
        Some(_) => Err(format!("{path}: a probe_floor outside allow.toml")),
    })
}

fn rows_and_floor(path: &str) -> Result<(Rows, Option<ServerVersion>), String> {
    let file: RawFile = toml::from_str(text(path)).map_err(|e| format!("{path}: {e}"))?;
    let floor = file.probe_floor.as_deref().map(version).transpose()?;
    let rows = file
        .rows
        .into_iter()
        .map(|r| {
            let gate = Gate {
                since: r.since.as_deref().map(version).transpose()?,
                until: r.until.as_deref().map(version).transpose()?,
            };
            if r.name.is_empty() || r.source.is_empty() {
                return Err(format!("{path}: a row without a name or source"));
            }
            Ok((r, gate))
        })
        .collect::<Result<_, String>>()?;
    Ok((rows, floor))
}

/// The lists of `engine` (`pg`, `mysql`, `mariadb`, `sqlite`).
fn engine(engine: &str) -> Result<Engine, String> {
    let at = |file: &str| format!("{engine}/{file}");
    let mut e = Engine::default();
    let (allow, floor) = rows_and_floor(&at("allow.toml"))?;
    e.floor = Some(floor.ok_or_else(|| format!("{engine}/allow.toml: no probe_floor"))?);
    for (r, gate) in allow {
        if r.group.is_some() {
            return Err(format!("{engine}/allow.toml: {}: a group", r.name));
        }
        if r.reviewed {
            let key = if engine == "pg" {
                r.name
            } else {
                r.name.to_ascii_lowercase()
            };
            e.allow.insert(key, gate);
        }
    }
    for (r, _) in rows(&at("deny.toml"))? {
        let group = r
            .group
            .as_deref()
            .and_then(DenyGroup::from_name)
            .ok_or_else(|| format!("{engine}/deny.toml: {}: no group", r.name))?;
        e.deny.insert(r.name.to_ascii_lowercase(), group);
    }
    for (r, gate) in rows(&at("operators.toml"))? {
        if r.reviewed && gate.is_open() {
            e.operators.insert(r.name.to_ascii_lowercase());
        }
    }
    for (r, gate) in rows(&at("types.toml"))? {
        if r.reviewed && gate.is_open() {
            e.types.insert(r.name.to_ascii_lowercase());
        }
    }
    for (r, _) in rows(&at("set_allow.toml"))? {
        if r.reviewed {
            e.set_allow.insert(r.name.to_ascii_lowercase());
        }
    }
    // A denied name is never allowed, whatever the lists say.
    let denied: Vec<String> = e.deny.keys().cloned().collect();
    for name in denied {
        e.allow.remove(&name);
    }
    Ok(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiled_tables_parse() {
        for name in ["pg", "mysql", "mariadb", "sqlite"] {
            if let Err(e) = engine(name) {
                panic!("{e}");
            }
        }
    }

    #[test]
    fn versions_and_gates() {
        assert_eq!(version("16"), Ok(ServerVersion::new(16, 0, 0)));
        assert_eq!(version("10.11"), Ok(ServerVersion::new(10, 11, 0)));
        assert_eq!(version("8.0.26"), Ok(ServerVersion::new(8, 0, 26)));
        for bad in ["", "x", "1.2.3.4", "16."] {
            assert!(version(bad).is_err(), "{bad:?}");
        }
        let v = |a, b, c| Some(ServerVersion::new(a, b, c));
        let pg14 = v(14, 0, 0);
        let g = Gate {
            since: v(16, 0, 0),
            until: v(18, 0, 0),
        };
        assert!(!g.holds(None, pg14));
        assert!(!g.holds(v(15, 9, 0), pg14));
        assert!(g.holds(v(16, 0, 0), pg14));
        assert!(g.holds(v(17, 6, 0), pg14));
        assert!(!g.holds(v(18, 0, 0), pg14));
        // No `since`: proven only from the floor on.
        let open = Gate::default();
        assert!(open.holds(None, pg14));
        assert!(open.holds(v(14, 0, 0), pg14));
        assert!(!open.holds(v(13, 22, 0), pg14));
        // A hand `since` below the floor.
        let my_floor = v(8, 0, 44);
        let hand = Gate {
            since: v(8, 0, 11),
            until: None,
        };
        assert!(hand.holds(None, my_floor));
        assert!(hand.holds(v(8, 0, 12), my_floor));
        assert!(!hand.holds(v(5, 7, 44), my_floor));
        assert!(!open.holds(v(8, 0, 12), my_floor));
        // No floor: nothing holds.
        assert!(!open.holds(v(17, 0, 0), None));
        assert!(!hand.holds(None, None));
    }

    #[test]
    fn every_engine_records_its_probe_floor() {
        let t = tables();
        for e in [&t.pg, &t.mysql, &t.mariadb, &t.sqlite] {
            assert!(e.floor.is_some());
        }
        assert_eq!(t.pg.floor, Some(ServerVersion::new(14, 0, 0)));
    }

    #[test]
    fn every_listed_file_is_compiled_in() {
        assert_eq!(DIGEST_FILES.len(), TEXTS.len());
        for p in DIGEST_FILES {
            assert!(!text(p).is_empty(), "{p}");
        }
    }
}
