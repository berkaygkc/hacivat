// ## m4-t23-minicat (named exception in Task 26's module)
//! Canonical JSON format 1 (R-M4-17): byte-order sorted, no server ids, OIDs, state marks,
//! watermarks, timestamps, hostnames or paths, every named exclusion rule applied. The
//! generator (`xtask minicat build`, from the minicat spike `cd27ec1`) writes it and this
//! module defines it for the decoder: [`normalize`] turns any format-1 document into its
//! canonical bytes, and [`super::decode`] accepts a snapshot only when its JSON *is* those
//! bytes. xtask has no dependency on this crate, so the two agree by test: the committed
//! assets must normalize to themselves here (`tests/system_decode.rs`).
//!
//! The canonical text is `serde_json`'s pretty form (two-space indent, LF) plus one final
//! newline. Every list is sorted by byte order of its key and a duplicate key is an error;
//! columns keep their ordinal order. Kinds come from closed vocabularies and schemas from the
//! engine's system schemas: anything else fails closed.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::SystemError;

/// The one format this build reads and writes.
pub const FORMAT: u32 = 1;

/// The largest canonical JSON a snapshot may hold (the PostgreSQL 18 file is about 1.4 MB).
pub const MAX_JSON_BYTES: usize = 8 * 1024 * 1024;

/// The system schemas of PostgreSQL snapshots.
pub const PG_SCHEMAS: [&str; 2] = ["information_schema", "pg_catalog"];
/// The system schemas of MySQL and MariaDB snapshots (carry: never `hv_catalog_probe`).
pub const MYSQL_SCHEMAS: [&str; 4] = ["information_schema", "mysql", "performance_schema", "sys"];

const RELATION_KINDS: [&str; 7] = [
    "foreign_table",
    "materialized_view",
    "partitioned_table",
    "sequence",
    "system_view",
    "table",
    "view",
];
const ROUTINE_KINDS: [&str; 4] = ["aggregate", "function", "procedure", "window"];
const OPERATOR_KINDS: [&str; 3] = ["infix", "postfix", "prefix"];
const TYPE_KINDS: [&str; 7] = [
    "base",
    "composite",
    "domain",
    "enum",
    "multirange",
    "pseudo",
    "range",
];
const VOLATILITIES: [&str; 3] = ["immutable", "stable", "volatile"];

/// One server line's system catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub format: u32,
    /// `postgres`, `mysql` or `mariadb`.
    pub dbms: String,
    /// The server line: `17`, `8.4`, `10.11`.
    pub line: String,
    /// The exact server version the snapshot was read from (`17.6`, `8.4.6`).
    pub server_version: String,
    pub coverage: Coverage,
    pub relations: Vec<Relation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routines: Vec<Routine>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operators: Vec<Operator>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<TypeDef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<Variable>,
}

/// What the read could see.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    /// `catalog` (the system-schema reader) or `test` (the fixture account).
    pub reader: String,
    pub expected_schemas: Vec<String>,
    pub missing_schemas: Vec<String>,
    pub relations_without_columns: u32,
    /// Where the schema-less built-in function names come from: `probe`, `sql_functions`, or
    /// both joined with `+` (MySQL/MariaDB).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtins: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Relation {
    pub schema: String,
    pub name: String,
    pub kind: String,
    /// In ordinal order.
    pub columns: Vec<Column>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Column {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub nullable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Routine {
    /// `None` for schema-less built-ins (MySQL/MariaDB native functions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub name: String,
    pub kind: String,
    /// The argument list as the server prints it (`None`: not introspectable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<String>,
    /// PostgreSQL's identity arguments, only when they differ from `args`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volatility: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub set_returning: bool,
    /// The server's own description (PostgreSQL only: MySQL and MariaDB comments are GPL
    /// documentation text and are never read).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operator {
    pub schema: String,
    pub name: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right: Option<String>,
    pub result: String,
    pub function: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypeDef {
    pub schema: String,
    pub name: String,
    pub sql_name: String,
    pub kind: String,
    pub category: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variable {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub ty: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

impl Snapshot {
    /// The engine's system schemas, `None` for a DBMS no snapshot is made for.
    pub fn system_schemas(&self) -> Option<&'static [&'static str]> {
        match self.dbms.as_str() {
            "postgres" => Some(&PG_SCHEMAS),
            "mysql" | "mariadb" => Some(&MYSQL_SCHEMAS),
            _ => None,
        }
    }
}

fn corrupt(why: &'static str) -> SystemError {
    SystemError::Corrupt(why)
}

fn sort_unique<T, K: Ord>(
    items: &mut [T],
    key: impl Fn(&T) -> K,
    dup: &'static str,
) -> Result<(), SystemError> {
    items.sort_by_key(|a| key(a));
    if items.windows(2).any(|w| key(&w[0]) == key(&w[1])) {
        return Err(corrupt(dup));
    }
    Ok(())
}

fn one_of(v: &str, set: &[&str], why: &'static str) -> Result<(), SystemError> {
    if set.contains(&v) {
        Ok(())
    } else {
        Err(corrupt(why))
    }
}

/// Checks every closed vocabulary and the schemas.
fn validate(s: &Snapshot) -> Result<(), SystemError> {
    if s.format != FORMAT {
        return Err(SystemError::UnknownFormat(s.format));
    }
    let schemas = s.system_schemas().ok_or(corrupt("unknown dbms"))?;
    let pg = s.dbms == "postgres";
    let line_ok = !s.line.is_empty()
        && s.line.len() <= 16
        && s.line.bytes().all(|b| b.is_ascii_digit() || b == b'.');
    if !line_ok {
        return Err(corrupt("bad line"));
    }
    if s.server_version.is_empty() || s.server_version.len() > 64 {
        return Err(corrupt("bad server version"));
    }
    for r in &s.relations {
        one_of(&r.schema, schemas, "relation outside the system schemas")?;
        one_of(&r.kind, &RELATION_KINDS, "unknown relation kind")?;
    }
    for r in &s.routines {
        match &r.schema {
            Some(sc) => one_of(sc, schemas, "routine outside the system schemas")?,
            None if pg => return Err(corrupt("schema-less PostgreSQL routine")),
            None => {}
        }
        one_of(&r.kind, &ROUTINE_KINDS, "unknown routine kind")?;
        if let Some(v) = &r.volatility {
            one_of(v, &VOLATILITIES, "unknown volatility")?;
        }
    }
    for o in &s.operators {
        one_of(&o.schema, schemas, "operator outside the system schemas")?;
        one_of(&o.kind, &OPERATOR_KINDS, "unknown operator kind")?;
    }
    for t in &s.types {
        one_of(&t.schema, schemas, "type outside the system schemas")?;
        one_of(&t.kind, &TYPE_KINDS, "unknown type kind")?;
    }
    if !pg && !(s.operators.is_empty() && s.types.is_empty()) {
        return Err(corrupt("operators or types outside PostgreSQL"));
    }
    for sc in s
        .coverage
        .expected_schemas
        .iter()
        .chain(&s.coverage.missing_schemas)
    {
        one_of(sc, schemas, "coverage names another schema")?;
    }
    Ok(())
}

/// Sorts every list by byte order of its key and fails on a duplicate key; columns keep their
/// ordinal order. Idempotent.
pub fn canonicalize(s: &mut Snapshot) -> Result<(), SystemError> {
    validate(s)?;
    sort_unique(
        &mut s.relations,
        |r| (r.schema.clone(), r.name.clone()),
        "relation twice",
    )?;
    for r in &s.relations {
        let mut names: Vec<&str> = r.columns.iter().map(|c| c.name.as_str()).collect();
        names.sort_unstable();
        if names.windows(2).any(|w| w[0] == w[1]) {
            return Err(corrupt("column twice"));
        }
    }
    sort_unique(
        &mut s.routines,
        |r| {
            (
                r.schema.clone(),
                r.name.clone(),
                r.kind.clone(),
                r.identity.clone().or_else(|| r.args.clone()),
            )
        },
        "routine twice",
    )?;
    sort_unique(
        &mut s.operators,
        |o| {
            (
                o.schema.clone(),
                o.name.clone(),
                o.left.clone(),
                o.right.clone(),
            )
        },
        "operator twice",
    )?;
    sort_unique(
        &mut s.types,
        |t| (t.schema.clone(), t.name.clone()),
        "type twice",
    )?;
    sort_unique(&mut s.variables, |v| v.name.clone(), "variable twice")?;
    s.coverage.expected_schemas.sort();
    s.coverage.missing_schemas.sort();
    Ok(())
}

/// The canonical text of an already canonical `s`.
pub fn to_canonical_json(s: &Snapshot) -> Result<Vec<u8>, SystemError> {
    let mut json = serde_json::to_vec_pretty(s).map_err(|_| corrupt("unserializable"))?;
    json.push(b'\n');
    if json.len() > MAX_JSON_BYTES {
        return Err(corrupt("snapshot too large"));
    }
    Ok(json)
}

/// The typed form of a format-1 document, canonicalized.
pub fn parse(raw: &serde_json::Value) -> Result<Snapshot, SystemError> {
    if let Some(n) = raw.get("format").and_then(serde_json::Value::as_u64)
        && n != u64::from(FORMAT)
    {
        return Err(SystemError::UnknownFormat(
            u32::try_from(n).unwrap_or(u32::MAX),
        ));
    }
    let mut s = Snapshot::deserialize(raw).map_err(|_| corrupt("not format-1 JSON"))?;
    canonicalize(&mut s)?;
    Ok(s)
}

/// The canonical JSON bytes of a format-1 document: the same snapshot in any row order gives
/// the same bytes.
pub fn normalize(raw: &serde_json::Value) -> Result<Vec<u8>, SystemError> {
    to_canonical_json(&parse(raw)?)
}

/// `items` grouped by schema, each group in the items' order (for the decoder).
pub(crate) fn by_schema<T>(items: &[T], schema: impl Fn(&T) -> &str) -> BTreeMap<&str, Vec<&T>> {
    let mut out: BTreeMap<&str, Vec<&T>> = BTreeMap::new();
    for i in items {
        out.entry(schema(i)).or_default().push(i);
    }
    out
}
