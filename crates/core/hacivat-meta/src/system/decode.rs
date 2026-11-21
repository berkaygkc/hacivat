// ## m4-t23-minicat (named exception in Task 26's module)
//! Decoding one shipped snapshot (R-M4-33): a zstd frame (checksum on) of canonical JSON
//! format 1. Untrusted bytes: decoding never panics (fuzz target `system_snapshot_decode`).
//!
//! What a file must be, checked in this order, each failure an error:
//! - exactly one zstd frame (no skippable frame, no second frame, no trailing byte), whose
//!   header declares its content size and carries a content checksum;
//! - a declared size of at most [`MAX_JSON_BYTES`]: the buffer is allocated from the declared
//!   size only after that bound, and decompression fails if the content is longer or shorter,
//!   so a length is never trusted; libzstd verifies the checksum;
//! - JSON that is format 1 and **is** its own canonical form ([`normalize`]): a reordered,
//!   re-indented or hand-edited file is refused, never repaired;
//! - closed vocabularies and the engine's system schemas only (`normalize`'s validation), a
//!   `coverage` that agrees with the content (fix round 1, review m-3), and a namespace the
//!   model's kind schema accepts.
//!
//! The mapping: one [`NamespaceSnapshot`] per system schema, keyed `NsKey::schema([schema])`
//! with no database (PostgreSQL's system schemas exist in every database; the system layer,
//! Task 26, places them), stored rule `FoldRule::Exact` (PostgreSQL; MySQL/MariaDB as read on
//! the pinned Linux images, `lower_case_table_names = 0`). Relations with their columns,
//! schema routines with their arguments, PostgreSQL types (`ObjectKind::Unknown`, raw kind
//! `pg-type`, as Task 16's live system read) and operators. Every object is flagged `SYSTEM |
//! PRE_INTROSPECTED`. MySQL/MariaDB built-in names go to [`SystemSnapshot::builtins`] (name
//! only); operators, types and setting names to their side tables.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use sql_catalog::SettingInfo;

use super::normalize::{self, MAX_JSON_BYTES, Snapshot};
use super::{SystemError, SystemOperator, SystemSnapshot, SystemType};
use crate::ids::NsKey;
use crate::kind::ObjectKind;
use crate::name::{FoldRule, Name};
use crate::namespace::{Level, NamespaceSnapshot, NsMeta, Obj, ObjFlags, Subtree};
use crate::props::{
    ArgMode, ColumnProps, FnArg, FnKind, GenericProps, Props, RoutineProps, TableProps, ViewProps,
    Volatility,
};
use hacivat_proto::DbmsKind;

/// The zstd frame magic number, little-endian.
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];
/// `Frame_Header_Descriptor` bit 2: `Content_Checksum_flag`.
const CHECKSUM_FLAG: u8 = 0x04;
/// `Frame_Header_Descriptor` bit 4, which a decoder does not interpret: the generator never
/// sets it, so a file with it set is not one the generator wrote.
const UNUSED_FLAG: u8 = 0x10;

/// The raw kind of a PostgreSQL type object (Task 16's `pg::system::SYSTEM_TYPE_RAW_KIND`).
const PG_TYPE_RAW_KIND: &str = "pg-type";

fn corrupt(why: &'static str) -> SystemError {
    SystemError::Corrupt(why)
}

/// The canonical JSON in `bytes`, checked exactly as [`decode`] checks it (module docs): it
/// answers `Ok` exactly when `decode` does.
pub fn decode_json(bytes: &[u8]) -> Result<Vec<u8>, SystemError> {
    decode_all(bytes).map(|(json, _)| json)
}

/// The snapshot in `bytes` (module docs).
pub fn decode(bytes: &[u8]) -> Result<SystemSnapshot, SystemError> {
    decode_all(bytes).map(|(_, snap)| snap)
}

fn decode_all(bytes: &[u8]) -> Result<(Vec<u8>, SystemSnapshot), SystemError> {
    if bytes.len() < 6 || bytes[..4] != ZSTD_MAGIC {
        return Err(corrupt("not a zstd frame"));
    }
    if bytes[4] & CHECKSUM_FLAG == 0 {
        return Err(corrupt("no content checksum"));
    }
    if bytes[4] & UNUSED_FLAG != 0 {
        return Err(corrupt("bad frame header"));
    }
    match zstd::zstd_safe::find_frame_compressed_size(bytes) {
        Ok(n) if n == bytes.len() => {}
        Ok(_) => return Err(corrupt("bytes after the frame")),
        Err(_) => return Err(corrupt("truncated or bad frame")),
    }
    let declared = match zstd::zstd_safe::get_frame_content_size(bytes) {
        Ok(Some(n)) => n,
        Ok(None) => return Err(corrupt("no content size")),
        Err(_) => return Err(corrupt("bad frame header")),
    };
    let declared = usize::try_from(declared).map_err(|_| corrupt("snapshot too large"))?;
    if declared > MAX_JSON_BYTES {
        return Err(corrupt("snapshot too large"));
    }
    let mut d = zstd::bulk::Decompressor::new().map_err(SystemError::Io)?;
    let json = d
        .decompress(bytes, declared)
        .map_err(|_| corrupt("bad zstd content"))?;
    if json.len() != declared {
        return Err(corrupt("content size mismatch"));
    }
    let value: serde_json::Value =
        serde_json::from_slice(&json).map_err(|_| corrupt("not JSON"))?;
    let snap = normalize::parse(&value)?;
    if normalize::to_canonical_json(&snap)? != json {
        return Err(corrupt("not canonical"));
    }
    let system = to_system(&snap)?;
    Ok((json, system))
}

fn n(s: &str) -> Name {
    Name::from(s)
}

fn on(s: &Option<String>) -> Option<Name> {
    s.as_deref().map(Name::from)
}

fn flagged(o: Obj) -> Obj {
    o.add_flags(ObjFlags::SYSTEM | ObjFlags::PRE_INTROSPECTED)
}

fn dbms_of(s: &str) -> Result<DbmsKind, SystemError> {
    match s {
        "postgres" => Ok(DbmsKind::Postgres),
        "mysql" => Ok(DbmsKind::MySql),
        "mariadb" => Ok(DbmsKind::MariaDb),
        _ => Err(corrupt("unknown dbms")),
    }
}

fn to_system(s: &Snapshot) -> Result<SystemSnapshot, SystemError> {
    let dbms = dbms_of(&s.dbms)?;
    let pg = dbms == DbmsKind::Postgres;
    let schemas = s.system_schemas().ok_or(corrupt("unknown dbms"))?;
    let type_names: BTreeSet<&str> = s.types.iter().map(|t| t.sql_name.as_str()).collect();
    let rels = normalize::by_schema(&s.relations, |r| r.schema.as_str());
    let in_schemas: Vec<&normalize::Routine> =
        s.routines.iter().filter(|r| r.schema.is_some()).collect();
    let routines = normalize::by_schema(&in_schemas, |r| r.schema.as_deref().unwrap_or_default());
    let types = normalize::by_schema(&s.types, |t| t.schema.as_str());
    let operators = normalize::by_schema(&s.operators, |o| o.schema.as_str());
    let mut namespaces = BTreeMap::new();
    for schema in schemas {
        let (r, f, t, o) = (
            rels.get(schema),
            routines.get(schema),
            types.get(schema),
            operators.get(schema),
        );
        if r.is_none() && f.is_none() && t.is_none() && o.is_none() {
            continue;
        }
        let mut tree = Subtree::new(flagged(Obj::new(ObjectKind::Schema, n(schema))));
        let root = tree.root();
        for rel in r.into_iter().flatten() {
            let (kind, props) = relation_kind(&rel.kind)?;
            let id = tree.add(
                root,
                flagged(Obj::new(kind, n(&rel.name)).with_props(props)),
            );
            for (i, c) in rel.columns.iter().enumerate() {
                let props = ColumnProps {
                    ordinal: u32::try_from(i + 1).map_err(|_| corrupt("too many columns"))?,
                    raw_type: n(&c.ty),
                    nullable: c.nullable,
                    ..ColumnProps::default()
                };
                tree.add(
                    id,
                    flagged(
                        Obj::new(ObjectKind::Column, n(&c.name))
                            .with_props(Props::Column(Arc::new(props))),
                    ),
                );
            }
        }
        for rt in f.into_iter().flatten() {
            let (kind, fn_kind) = routine_kind(&rt.kind)?;
            let args = rt.args.as_deref().and_then(|a| {
                if pg {
                    pg_args(a, &type_names)
                } else {
                    mysql_args(a)
                }
            });
            let props = RoutineProps {
                args,
                returns: on(&rt.result),
                returns_set: rt.set_returning,
                kind: fn_kind,
                volatility: match rt.volatility.as_deref() {
                    Some("immutable") => Volatility::Immutable,
                    Some("stable") => Volatility::Stable,
                    Some("volatile") => Volatility::Volatile,
                    _ => Volatility::Unknown,
                },
                overload_sig: if pg {
                    rt.identity.as_ref().or(rt.args.as_ref()).map(|a| n(a))
                } else {
                    None
                },
                comment: on(&rt.description),
                ..RoutineProps::default()
            };
            tree.add(
                root,
                flagged(Obj::new(kind, n(&rt.name)).with_props(Props::Routine(Arc::new(props)))),
            );
        }
        for ty in t.into_iter().flatten() {
            let props = GenericProps {
                comment: on(&ty.description),
                raw_kind: Some(n(PG_TYPE_RAW_KIND)),
                ..GenericProps::default()
            };
            tree.add(
                root,
                flagged(
                    Obj::new(ObjectKind::Unknown, n(&ty.name))
                        .with_props(Props::Generic(Arc::new(props))),
                ),
            );
        }
        for op in o.into_iter().flatten() {
            let props = GenericProps {
                comment: on(&op.description),
                ..GenericProps::default()
            };
            tree.add(
                root,
                flagged(
                    Obj::new(ObjectKind::Operator, n(&op.name))
                        .with_props(Props::Generic(Arc::new(props))),
                ),
            );
        }
        let key = NsKey::schema([*schema]);
        let meta = NsMeta {
            level: Level::Structure,
            ..NsMeta::default()
        };
        let ns = NamespaceSnapshot::build(key.clone(), dbms, FoldRule::Exact, meta, tree)
            .map_err(|_| corrupt("the model refuses a namespace"))?;
        namespaces.insert(key, Arc::new(ns));
    }
    let builtins = s
        .routines
        .iter()
        .filter(|r| r.schema.is_none())
        .map(|r| {
            flagged(
                Obj::new(ObjectKind::Routine, n(&r.name))
                    .with_props(Props::Routine(Arc::new(RoutineProps::default()))),
            )
        })
        .collect();
    let operators = s
        .operators
        .iter()
        .map(|o| SystemOperator {
            schema: n(&o.schema),
            name: n(&o.name),
            kind: n(&o.kind),
            left: on(&o.left),
            right: on(&o.right),
            result: n(&o.result),
            function: n(&o.function),
            comment: on(&o.description),
        })
        .collect();
    let types = s
        .types
        .iter()
        .map(|t| SystemType {
            schema: n(&t.schema),
            name: n(&t.name),
            sql_name: n(&t.sql_name),
            kind: n(&t.kind),
            category: n(&t.category),
            element: on(&t.element),
            comment: on(&t.description),
        })
        .collect();
    let settings = s
        .variables
        .iter()
        .map(|v| SettingInfo {
            name: n(&v.name),
            type_name: on(&v.ty),
            context: on(&v.context).or_else(|| on(&v.scope)),
        })
        .collect();
    Ok(SystemSnapshot {
        dbms,
        line: n(&s.line),
        server_version: n(&s.server_version),
        format: s.format,
        namespaces,
        builtins,
        operators,
        types,
        settings,
    })
}

/// The model kind and props of a format-1 relation kind. A MariaDB `SEQUENCE` keeps its
/// columns, so it is a table here.
fn relation_kind(kind: &str) -> Result<(ObjectKind, Props), SystemError> {
    let table = |partitioned| {
        Props::Table(Arc::new(TableProps {
            partitioned,
            ..TableProps::default()
        }))
    };
    let view = || Props::View(Arc::new(ViewProps::default()));
    Ok(match kind {
        "table" | "sequence" => (ObjectKind::Table, table(false)),
        "partitioned_table" => (ObjectKind::Table, table(true)),
        "foreign_table" => (ObjectKind::ForeignTable, table(false)),
        "view" | "system_view" => (ObjectKind::View, view()),
        "materialized_view" => (ObjectKind::MaterializedView, view()),
        _ => return Err(corrupt("unknown relation kind")),
    })
}

fn routine_kind(kind: &str) -> Result<(ObjectKind, FnKind), SystemError> {
    Ok(match kind {
        "function" => (ObjectKind::Routine, FnKind::Function),
        "window" => (ObjectKind::Routine, FnKind::Window),
        "aggregate" => (ObjectKind::Aggregate, FnKind::Aggregate),
        "procedure" => (ObjectKind::Procedure, FnKind::Procedure),
        _ => return Err(corrupt("unknown routine kind")),
    })
}

/// `text` split at top-level commas (outside parentheses, brackets and quotes), each part
/// trimmed; `None` when the quoting or nesting does not balance.
pub(crate) fn split_top(text: &str) -> Option<Vec<&str>> {
    let mut parts = Vec::new();
    if text.trim().is_empty() {
        return Some(parts);
    }
    let (mut depth, mut start) = (0usize, 0usize);
    let mut quote: Option<char> = None;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match quote {
            Some(q) if c == q => {
                // A doubled quote is an escaped one.
                if chars.peek().is_some_and(|&(_, d)| d == q) {
                    chars.next();
                } else {
                    quote = None;
                }
            }
            Some(_) => {}
            None => match c {
                '\'' | '"' | '`' => quote = Some(c),
                '(' | '[' => depth += 1,
                ')' | ']' => depth = depth.checked_sub(1)?,
                ',' if depth == 0 => {
                    parts.push(text[start..i].trim());
                    start = i + 1;
                }
                _ => {}
            },
        }
    }
    if quote.is_some() || depth != 0 {
        return None;
    }
    parts.push(text[start..].trim());
    if parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    Some(parts)
}

/// The byte offset of the first top-level ` DEFAULT ` in `arg` (outside quotes).
fn default_at(arg: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (i, c) in arg.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '\'' || c == '"' => quote = Some(c),
            None if arg[i..].starts_with(" DEFAULT ") => return Some(i),
            None => {}
        }
    }
    None
}

/// PostgreSQL `pg_get_function_arguments` text as arguments: `[mode ][name ]type[ DEFAULT
/// expr]`. A type is recognised only as a type spelling of the same snapshot, so a name is
/// never mistaken for the first word of `double precision`; anything not recognised makes
/// the whole list `None` (unknown), never a guess.
fn pg_args(text: &str, types: &BTreeSet<&str>) -> Option<Vec<FnArg>> {
    let mut out = Vec::new();
    for part in split_top(text)? {
        let (body, has_default) = match default_at(part) {
            Some(i) => (part[..i].trim(), true),
            None => (part, false),
        };
        let (mode, body) = [
            ("INOUT ", ArgMode::InOut),
            ("OUT ", ArgMode::Out),
            ("VARIADIC ", ArgMode::Variadic),
            ("IN ", ArgMode::In),
        ]
        .into_iter()
        .find_map(|(p, m)| body.strip_prefix(p).map(|rest| (m, rest)))
        .unwrap_or((ArgMode::In, body));
        let (name, ty) = if types.contains(body) {
            (None, body)
        } else {
            let (name, ty) = body.split_once(' ')?;
            if !types.contains(ty) {
                return None;
            }
            (Some(n(name)), ty)
        };
        out.push(FnArg {
            name,
            type_name: n(ty),
            mode,
            has_default,
        });
    }
    Some(out)
}

/// The generator's MySQL/MariaDB argument text (`[IN |OUT |INOUT ]name type`, from
/// `INFORMATION_SCHEMA.PARAMETERS`) as arguments; `None` when a part does not read so.
fn mysql_args(text: &str) -> Option<Vec<FnArg>> {
    let mut out = Vec::new();
    for part in split_top(text)? {
        let (mode, body) = [
            ("INOUT ", ArgMode::InOut),
            ("OUT ", ArgMode::Out),
            ("IN ", ArgMode::In),
        ]
        .into_iter()
        .find_map(|(p, m)| part.strip_prefix(p).map(|rest| (m, rest)))
        .unwrap_or((ArgMode::In, part));
        let (name, ty) = body.split_once(' ')?;
        if name.is_empty() || ty.trim().is_empty() {
            return None;
        }
        out.push(FnArg {
            name: Some(n(name)),
            type_name: n(ty.trim()),
            mode,
            has_default: false,
        });
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_top_respects_nesting_and_quotes() {
        assert_eq!(split_top("").unwrap(), Vec::<&str>::new());
        assert_eq!(
            split_top("a int, b numeric(10,2)").unwrap(),
            ["a int", "b numeric(10,2)"]
        );
        assert_eq!(
            split_top("e enum('a','b,c'), f int").unwrap(),
            ["e enum('a','b,c')", "f int"]
        );
        assert_eq!(
            split_top("x text DEFAULT 'it''s, ok'").unwrap(),
            ["x text DEFAULT 'it''s, ok'"]
        );
        assert!(split_top("a (int").is_none());
        assert!(split_top("a int)").is_none());
        assert!(split_top("'open").is_none());
        assert!(split_top("a int,,b int").is_none());
    }

    #[test]
    fn pg_args_never_guess_a_name() {
        let types: BTreeSet<&str> = ["integer", "double precision", "text", "integer[]"].into();
        let a = pg_args("double precision, VARIADIC xs integer[]", &types).unwrap();
        assert_eq!(a[0].name, None);
        assert_eq!(&*a[0].type_name, "double precision");
        assert_eq!(a[1].mode, ArgMode::Variadic);
        assert_eq!(a[1].name.as_deref(), Some("xs"));
        // An unknown type: the list is unknown, not a guess.
        assert!(pg_args("x mystery", &types).is_none());
        let d = pg_args("t text DEFAULT ' DEFAULT '::text", &types).unwrap();
        assert!(d[0].has_default);
        assert_eq!(&*d[0].type_name, "text");
    }
}
