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
//! - **Assumption (residual risk, ADR 0005 `m3b-t03`):** an unqualified PostgreSQL call
//!   resolves to `pg_catalog` first. A `search_path` that names a writable schema before
//!   `pg_catalog` can shadow an allowed builtin with a user function of the same signature;
//!   the server's read-only transaction (L3) and the catalog (M4) are the backstop.

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
        Some(gate) if gate.holds(version) => FnClass::Allowed,
        _ => FnClass::Unknown,
    }
}

/// Whether `op` is a reviewed builtin operator of `d` with no version gate.
pub fn operator_known(d: DialectId, op: &str) -> bool {
    own_engine(d).is_some_and(|e| e.operators.contains(op.trim()))
}

/// Whether a cast to `ty` (as written: `int4`, `pg_catalog.int8`, `double precision`,
/// `varchar(10)`, `text[]`) targets a reviewed builtin type of `d` with no version gate.
pub fn cast_known(d: DialectId, ty: &str) -> bool {
    let Some(engine) = own_engine(d) else {
        return false;
    };
    normalize_type(d, ty).is_some_and(|t| engine.types.contains(t.as_str()))
}

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
    let out = out.trim().replace(" .", ".").replace(". ", ".");
    if d == DialectId::Postgres
        && let Some(rest) = out.strip_prefix("pg_catalog.")
    {
        return Some(rest.to_string());
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

/// Whether a read-only session may run `SET name …` on `d` (P-M3b-6): `name` exactly, ASCII
/// case-insensitively, on `data/<engine>/set_allow.toml`. SQLite and every other dialect: no.
pub fn set_allowed(d: DialectId, name: &str) -> bool {
    let engine = match d {
        DialectId::Postgres | DialectId::MySql | DialectId::MariaDb => own_engine(d),
        _ => None,
    };
    engine.is_some_and(|e| e.set_allow.contains(name.to_ascii_lowercase().as_str()))
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
    }
}
