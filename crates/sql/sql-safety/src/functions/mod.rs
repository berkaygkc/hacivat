//! Functions, operators, casts and types (`alg/13` SAFE-03; Task 3): the per-dialect allow and
//! deny data under `data/`, the harmless-`SET` allowlist (P-M3b-6) and the catalog seam (M4).
//!
//! - **Deny first, by bare name.** A call whose last name part, ASCII-lowercased, is on the
//!   dialect's deny list is [`FnClass::Denied`] whatever its schema, quoting or the server
//!   version (a user function named like a denied builtin is denied too). Dialects without
//!   data of their own read every list.
//! - **Allow only builtins, exactly as the server resolves them.** An unqualified call (or one
//!   qualified with `pg_catalog` on PostgreSQL), folded as the server folds it
//!   ([`sql_ident::fold`], ASCII only), on a `reviewed = true` row whose version gate holds, is
//!   [`FnClass::Allowed`]. A schema-qualified user call, a quoted MySQL/MariaDB/SQLite name, an
//!   unreviewed row, a gated row on an unknown version and every other dialect are
//!   [`FnClass::Unknown`]. Matching never folds a non-ASCII letter (`İ`, `ı` stay as written).
//! - **Below the probe floor nothing is proven.** A row without `since` holds only at or above
//!   the engine's recorded probe floor (`probe_floor` in `allow.toml`: the oldest server the
//!   catalog read saw); a known older server answers `Unknown` for it, because a builtin
//!   missing there could be a user function of the same name. Hand `since` gates below the
//!   floor come from the engines' change histories.
//! - **`pg_catalog` resolves first (P-M3b-T3-1).** The allowlist assumes an unqualified
//!   PostgreSQL call reaches `pg_catalog` before any other schema. That is why
//!   [`set_allowed`] allows `SET search_path` only when its value leaves `pg_catalog` absent
//!   (implicitly first) or lists it first; the classifier checks the value (Tasks 9 and 14).

pub mod data;

use sql_dialect::{DialectId, ServerVersion};
use sql_ident::{QualifiedName, fold};

use self::data::{Engine, tables};

/// A server function catalog (M4); `ClassifyCtx.catalog` is `None` in M3b.
pub trait FnCatalog: Send + Sync {
    fn lookup(&self, f: &QualifiedName, arity: Option<u16>) -> FnLookup;
}

/// What a catalog knows about a function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FnLookup {
    Known { volatile: bool, writes: bool },
    Unknown,
}

/// What the builtin data says about a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FnClass {
    /// A reviewed pure builtin: no effect.
    Allowed,
    /// On the deny list, in this group.
    Denied(DenyGroup),
    /// Neither: a user function, an unreviewed or version-gated builtin, or an unknown name.
    Unknown,
}

/// Why a function is denied (`alg/13` SAFE-03 deny catalogue).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DenyGroup {
    /// Delays: `pg_sleep*`, `SLEEP`, `BENCHMARK`.
    Sleep,
    /// Server files: `pg_read_file`, `pg_ls_dir`, `lo_import`/`lo_export`, `LOAD_FILE`,
    /// SQLite's `load_extension`, `readfile`, `writefile`.
    FileIo,
    /// Other connections and hosts: `dblink*`, `http_*`.
    Network,
    /// Operating-system processes or native code: `sys_exec`, SQLite's `edit`.
    ProcessExec,
    /// Backend and server control: `pg_terminate_backend`, `pg_reload_conf`, `pg_stat_reset*`,
    /// WAL switches, backups, promotion.
    AdminSignal,
    /// Replication slots, origins and WAL messages; MySQL source-position waits.
    Replication,
    /// Large-object writes: `lo_create`, `lo_put`, `lo_unlink`, …
    LargeObject,
    /// Functions that run SQL text: `query_to_xml`, `cursor_to_xml`, `ts_stat`, …
    DynamicSql,
    /// Advisory locks: `pg_advisory_*`, `GET_LOCK`, …
    Advisory,
    /// Sequence advances: `nextval`, `setval`.
    Sequence,
    /// Transaction-id consumption: `txid_current`, `pg_current_xact_id`.
    XidConsume,
    /// Configuration changes: `set_config`.
    SetConfig,
}

impl DenyGroup {
    /// Every group, in declaration order.
    pub const ALL: [DenyGroup; 12] = [
        DenyGroup::Sleep,
        DenyGroup::FileIo,
        DenyGroup::Network,
        DenyGroup::ProcessExec,
        DenyGroup::AdminSignal,
        DenyGroup::Replication,
        DenyGroup::LargeObject,
        DenyGroup::DynamicSql,
        DenyGroup::Advisory,
        DenyGroup::Sequence,
        DenyGroup::XidConsume,
        DenyGroup::SetConfig,
    ];

    /// The group's name in `deny.toml` (the variant's name).
    pub fn name(self) -> &'static str {
        match self {
            DenyGroup::Sleep => "Sleep",
            DenyGroup::FileIo => "FileIo",
            DenyGroup::Network => "Network",
            DenyGroup::ProcessExec => "ProcessExec",
            DenyGroup::AdminSignal => "AdminSignal",
            DenyGroup::Replication => "Replication",
            DenyGroup::LargeObject => "LargeObject",
            DenyGroup::DynamicSql => "DynamicSql",
            DenyGroup::Advisory => "Advisory",
            DenyGroup::Sequence => "Sequence",
            DenyGroup::XidConsume => "XidConsume",
            DenyGroup::SetConfig => "SetConfig",
        }
    }

    /// The group named `name` in `deny.toml`.
    pub fn from_name(name: &str) -> Option<DenyGroup> {
        DenyGroup::ALL.into_iter().find(|g| g.name() == name)
    }
}

/// The engines whose deny lists apply to `d`: its own, or every list for a dialect without
/// data (fail closed).
fn deny_engines(d: DialectId) -> Vec<&'static Engine> {
    let t = tables();
    match d {
        DialectId::Postgres | DialectId::PgWire(_) => vec![&t.pg],
        DialectId::MySql => vec![&t.mysql],
        DialectId::MariaDb => vec![&t.mariadb],
        DialectId::Sqlite => vec![&t.sqlite],
        _ => vec![&t.pg, &t.mysql, &t.mariadb, &t.sqlite],
    }
}

/// The engine whose allow, operator and type lists describe `d`'s builtins; `None` for a
/// dialect without reviewed data (PG-wire servers differ from PostgreSQL's builtins).
fn own_engine(d: DialectId) -> Option<&'static Engine> {
    let t = tables();
    match d {
        DialectId::Postgres => Some(&t.pg),
        DialectId::MySql => Some(&t.mysql),
        DialectId::MariaDb => Some(&t.mariadb),
        DialectId::Sqlite => Some(&t.sqlite),
        _ => None,
    }
}

/// What the builtin data says about a call of `name` on `d` at `version` (`None`: unknown).
pub fn lookup_fn(d: DialectId, name: &QualifiedName, version: Option<ServerVersion>) -> FnClass {
    let Some(last) = name.parts.last() else {
        return FnClass::Unknown;
    };
    let bare = last.text.to_ascii_lowercase();
    for engine in deny_engines(d) {
        if let Some(group) = engine.deny.get(bare.as_str()) {
            return FnClass::Denied(*group);
        }
    }
    let Some(engine) = own_engine(d) else {
        return FnClass::Unknown;
    };
    let resolved = match d {
        DialectId::Postgres => {
            let builtin_schema = match name.parts.len() {
                1 => true,
                n => {
                    let schema = &name.parts[n - 2];
                    &*fold(d, &schema.text, schema.quoted) == "pg_catalog"
                }
            };
            if !builtin_schema {
                return FnClass::Unknown;
            }
            fold(d, &last.text, last.quoted).to_string()
        }
        // MySQL, MariaDB and SQLite builtins are never schema-qualified or quoted.
        _ if name.parts.len() > 1 || last.quoted => return FnClass::Unknown,
        _ => bare,
    };
    match engine.allow.get(resolved.as_str()) {
        Some(gate) if gate.holds(version, engine.floor) => FnClass::Allowed,
        _ => FnClass::Unknown,
    }
}

/// Whether `op` is a reviewed builtin operator of `d` with no version gate. Word operators
/// (`LIKE`, `DIV`, `BETWEEN`) match ASCII-case-insensitively; symbols match exactly.
pub fn operator_known(d: DialectId, op: &str) -> bool {
    own_engine(d).is_some_and(|e| {
        e.operators
            .contains(op.trim().to_ascii_lowercase().as_str())
    })
}

/// Whether a cast to `ty` (as written: `int4`, `pg_catalog.int8`, `double precision`,
/// `varchar(10)`, `text[]`, `interval day to second`, PostgreSQL's `"char"`, MySQL's
/// `char(10) character set utf8mb4`) targets a reviewed builtin type of `d` with no version
/// gate. Not covered (`false`, so the caller treats the cast as unknown): MySQL's
/// `… ARRAY` (multi-valued index casts) and any other quoted or qualified type name.
pub fn cast_known(d: DialectId, ty: &str) -> bool {
    let Some(engine) = own_engine(d) else {
        return false;
    };
    normalize_type(d, ty).is_some_and(|t| engine.types.contains(t.as_str()))
}

/// PostgreSQL's interval field qualifiers (`interval day to second`).
const INTERVAL_FIELDS: [&str; 13] = [
    "year",
    "month",
    "day",
    "hour",
    "minute",
    "second",
    "year to month",
    "day to hour",
    "day to minute",
    "day to second",
    "hour to minute",
    "hour to second",
    "minute to second",
];

/// `ty` as a key of `types.toml`: array brackets (PostgreSQL) and type modifiers dropped,
/// whitespace collapsed, ASCII letters lowered, a `pg_catalog.` qualifier removed. `None` for a
/// qualified name in another schema or an unbalanced modifier.
fn normalize_type(d: DialectId, ty: &str) -> Option<String> {
    let mut t = ty.trim();
    if d == DialectId::Postgres {
        while let Some(rest) = t.strip_suffix("[]") {
            t = rest.trim_end();
        }
    }
    let mut out = String::with_capacity(t.len());
    let mut depth = 0u32;
    for c in t.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.checked_sub(1)?,
            _ if depth > 0 => {}
            c if c.is_whitespace() => {
                if !out.ends_with(' ') {
                    out.push(' ');
                }
            }
            c => out.push(c.to_ascii_lowercase()),
        }
    }
    if depth != 0 {
        return None;
    }
    let mut out = out.trim().replace(" .", ".").replace(". ", ".");
    if d == DialectId::Postgres {
        // The single-byte `"char"` type is spelled quoted (`typname` `char`).
        if t == "\"char\"" {
            return Some("char".to_string());
        }
        if let Some(fields) = out.strip_prefix("interval ")
            && INTERVAL_FIELDS.contains(&fields)
        {
            return Some("interval".to_string());
        }
        if let Some(rest) = out.strip_prefix("pg_catalog.") {
            return Some(rest.to_string());
        }
    }
    if matches!(d, DialectId::MySql | DialectId::MariaDb) {
        // `CHAR(10) CHARACTER SET utf8mb4`: the charset names no type.
        for marker in [" character set ", " charset "] {
            if let Some(i) = out.find(marker) {
                let charset = &out[i + marker.len()..];
                if charset.is_empty()
                    || !charset
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
                {
                    return None;
                }
                out.truncate(i);
                break;
            }
        }
    }
    if out.is_empty() || out.contains('.') || out.contains('"') {
        return None;
    }
    Some(out)
}

/// The names on `d`'s function denylist (ASCII lower case), for the guard equality test
/// (P-M3b-13). A dialect without data of its own answers every list.
pub fn denied_function_names(d: DialectId) -> Box<dyn Iterator<Item = &'static str>> {
    let mut names: Vec<&'static str> = deny_engines(d)
        .into_iter()
        .flat_map(|e| e.deny.keys().map(String::as_str))
        .collect();
    names.sort_unstable();
    names.dedup();
    Box::new(names.into_iter())
}

/// Whether a read-only session may run `SET name = value` on `d` (P-M3b-6, P-M3b-T3-1).
///
/// - `name` must be on `data/<engine>/set_allow.toml`, matched whole and ASCII
///   case-insensitively. SQLite (PRAGMA has its own list) and every other dialect: never.
/// - PostgreSQL `search_path` also depends on `value`, the text after `TO`/`=` as written
///   (`public, pg_catalog`, `"$user", public`, `'my schema'`). It is allowed only when the value
///   is given and `pg_catalog` is absent from the list (so the server searches it first) or is
///   its first schema; a value that does not parse, or `None`, is refused. Each element is read
///   as PostgreSQL reads a `search_path` list: an unquoted name folds to ASCII lower case, a
///   `"quoted"` name keeps its exact text (so `"PG_CATALOG"` is another schema), and a
///   `'string'` is one schema with that exact text.
/// - Every other allowed variable ignores `value`.
pub fn set_allowed(d: DialectId, name: &str, value: Option<&str>) -> bool {
    let engine = match d {
        DialectId::Postgres | DialectId::MySql | DialectId::MariaDb => own_engine(d),
        _ => None,
    };
    let name = name.to_ascii_lowercase();
    if !engine.is_some_and(|e| e.set_allow.contains(name.as_str())) {
        return false;
    }
    if d == DialectId::Postgres && name == "search_path" {
        return value.and_then(search_path_schemas).is_some_and(|schemas| {
            schemas
                .iter()
                .position(|s| s == "pg_catalog")
                .is_none_or(|i| i == 0)
        });
    }
    true
}

/// The schemas of a PostgreSQL `search_path` value as the server resolves them; `None` when
/// it does not parse. Elements are separated by commas, with SQL whitespace around them.
fn search_path_schemas(value: &str) -> Option<Vec<String>> {
    let is_space = |c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c');
    let mut out = Vec::new();
    let mut chars = value.chars().peekable();
    loop {
        while chars.next_if(|c| is_space(*c)).is_some() {}
        let first = *chars.peek()?;
        let schema = if first == '"' || first == '\'' {
            chars.next();
            let mut s = String::new();
            loop {
                let c = chars.next()?;
                if c == first {
                    if chars.next_if_eq(&first).is_some() {
                        s.push(first);
                    } else {
                        break;
                    }
                } else {
                    s.push(c);
                }
            }
            s
        } else {
            let mut s = String::new();
            while let Some(c) = chars
                .next_if(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$' || !c.is_ascii())
            {
                s.push(c);
            }
            if s.is_empty() {
                return None;
            }
            fold(DialectId::Postgres, &s, false).to_string()
        };
        out.push(schema);
        while chars.next_if(|c| is_space(*c)).is_some() {}
        match chars.next() {
            None => return Some(out),
            Some(',') => {}
            Some(_) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_names_round_trip() {
        for g in DenyGroup::ALL {
            assert_eq!(DenyGroup::from_name(g.name()), Some(g));
        }
        assert_eq!(DenyGroup::from_name("sleep"), None);
    }

    #[test]
    fn type_names_normalise() {
        let n = |t| normalize_type(DialectId::Postgres, t);
        assert_eq!(
            n(" Timestamp(3)  WITH TIME   ZONE ").as_deref(),
            Some("timestamp with time zone")
        );
        assert_eq!(n("pg_catalog.INT8[][]").as_deref(), Some("int8"));
        assert_eq!(n("numeric(10,(2)").as_deref(), None);
        assert_eq!(n("public.int4"), None);
        assert_eq!(n("\"int4\""), None);
        assert_eq!(
            normalize_type(DialectId::MySql, "text[]").as_deref(),
            Some("text[]")
        );
        assert_eq!(n("interval day to second(3)").as_deref(), Some("interval"));
        assert_eq!(n("INTERVAL YEAR").as_deref(), Some("interval"));
        assert_eq!(
            n("interval fortnight").as_deref(),
            Some("interval fortnight")
        );
        assert_eq!(n("\"char\"").as_deref(), Some("char"));
        assert_eq!(n("\"CHAR\""), None);
        let m = |t| normalize_type(DialectId::MySql, t);
        assert_eq!(m("CHAR(10) CHARACTER SET utf8mb4").as_deref(), Some("char"));
        assert_eq!(m("char charset latin1").as_deref(), Some("char"));
        assert_eq!(m("char character set 'x'"), None);
    }

    #[test]
    fn search_path_lists_parse_as_postgres_reads_them() {
        let s = |v| search_path_schemas(v);
        assert_eq!(
            s(r#" "$user" , Public,'My Schema', "A""B" "#),
            Some(vec![
                "$user".to_string(),
                "public".to_string(),
                "My Schema".to_string(),
                "A\"B".to_string()
            ])
        );
        for bad in [
            "", " ", "a,", ",a", "a,,b", "\"a", "'a", "a b", "a;b", "E'a'",
        ] {
            assert_eq!(s(bad), None, "{bad:?}");
        }
        // A non-ASCII space is part of a name, as in PostgreSQL's lexer.
        assert_eq!(
            s("\u{a0}pg_catalog, x"),
            Some(vec!["\u{a0}pg_catalog".to_string(), "x".to_string()])
        );
    }
}
