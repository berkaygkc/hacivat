//! The SQLite introspector (HV-INTRO-08, -13, -14) over `sqlite_schema` and the `pragma_*`
//! table-valued functions, one bulk query per kind for a whole schema (`main`, `temp`, an
//! attached name): `pragma_table_xinfo(m.name, ?)` joined to `sqlite_schema`, and so on.
//!
//! Change detection (HV-INTRO-08): `PRAGMA <s>.schema_version` per schema is the fast path; a
//! SHA-256 digest of the ordered `(type, name, tbl_name, sql)` rows of `sqlite_schema` is the
//! slow path, run when the caller says the file's identity changed (or on connect). A change
//! re-reads the whole schema: SQLite has no per-object marker and a full read is local.
//!
//! The schema name is an identifier in `"<s>".sqlite_schema` (it cannot be bound): it is
//! quoted by doubling `"`. Table names are bound (`pragma_table_xinfo(?, ?)`) or joined.

use std::collections::{BTreeMap, HashMap};

use sha2::{Digest, Sha256};

use crate::conn::{CatalogConn, IntroError, Row};
use crate::model::*;

/// `"<name>"` with `"` doubled.
pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// The databases of this connection: `main`, `temp` (when used), attached ones.
pub async fn list_namespaces(c: &mut dyn CatalogConn) -> Result<Vec<NamespaceInfo>, IntroError> {
    let rows = c.rows("PRAGMA database_list", &[]).await?;
    Ok(rows
        .iter()
        .map(|r| NamespaceInfo {
            name: r.str(1).to_owned(),
            detail: r.text(2),
            collation: None,
            system: r.str(1) == "temp",
        })
        .collect())
}

/// `PRAGMA <s>.schema_version`.
pub async fn schema_version(c: &mut dyn CatalogConn, schema: &str) -> Result<i64, IntroError> {
    let sql = format!("PRAGMA {}.schema_version", quote_ident(schema));
    Ok(c.rows(&sql, &[])
        .await?
        .first()
        .and_then(|r| r.i64(0))
        .unwrap_or(0))
}

/// `PRAGMA <s>.data_version`: changes when *another* connection commits to the file (data or
/// schema), never for this connection's own commits. Not on the read-only allowlist (the
/// hardened connection's authorizer denies it).
pub async fn data_version(c: &mut dyn CatalogConn, schema: &str) -> Result<i64, IntroError> {
    let sql = format!("PRAGMA {}.data_version", quote_ident(schema));
    Ok(c.rows(&sql, &[])
        .await?
        .first()
        .and_then(|r| r.i64(0))
        .unwrap_or(0))
}

/// SHA-256 of the ordered `sqlite_schema` rows (the HV-INTRO-08 digest; the spec says BLAKE3,
/// `sha2` is already a workspace dependency, and either is fine for a change check).
pub async fn schema_digest(c: &mut dyn CatalogConn, schema: &str) -> Result<String, IntroError> {
    let sql = format!(
        "SELECT type, name, tbl_name, sql FROM {}.sqlite_schema ORDER BY type, name",
        quote_ident(schema)
    );
    let rows = c.rows(&sql, &[]).await?;
    let mut h = Sha256::new();
    for r in &rows {
        for i in 0..4 {
            match r.opt(i) {
                Some(v) => {
                    h.update([1u8]);
                    h.update(v.as_bytes());
                }
                None => h.update([0u8]),
            }
            h.update([0x1f]);
        }
        h.update([0x1e]);
    }
    Ok(h.finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>())
}

/// What a schema's state was when it was read (HV-INTRO-08 watermark).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteToken {
    pub schema_version: i64,
    pub digest: String,
}

/// A full read of `schema` at `level`. Consistency without a transaction (the read-only
/// authorizer denies `BEGIN`): `schema_version` before and after; a change in between retries.
pub async fn introspect(
    c: &mut dyn CatalogConn,
    schema: &str,
    level: Level,
) -> Result<(SchemaSnapshot, SqliteToken), IntroError> {
    let mut attempts = 0;
    loop {
        let before = schema_version(c, schema).await?;
        let snap = read_schema(c, schema, level).await?;
        let digest = schema_digest(c, schema).await?;
        let after = schema_version(c, schema).await?;
        attempts += 1;
        if before == after || attempts >= 3 {
            return Ok((
                snap,
                SqliteToken {
                    schema_version: after,
                    digest,
                },
            ));
        }
    }
}

/// Why [`refresh`] re-read or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshOutcome {
    /// `schema_version` unchanged and the identity unchanged: nothing read.
    Unchanged,
    /// The identity changed but the digest matched: nothing re-read.
    SameDigest,
    Reread,
}

/// HV-INTRO-08 steps 2–3: skip when `schema_version` is unchanged (and the file identity is),
/// check the digest when the identity changed, else re-read the schema.
pub async fn refresh(
    c: &mut dyn CatalogConn,
    prev: &SchemaSnapshot,
    prev_token: &SqliteToken,
    level: Level,
    identity_changed: bool,
) -> Result<(SchemaSnapshot, SqliteToken, RefreshOutcome), IntroError> {
    let schema = prev.name.as_str();
    let sv = schema_version(c, schema).await?;
    if prev.level == Some(level) {
        if sv == prev_token.schema_version && !identity_changed {
            return Ok((prev.clone(), prev_token.clone(), RefreshOutcome::Unchanged));
        }
        if identity_changed {
            let digest = schema_digest(c, schema).await?;
            if digest == prev_token.digest {
                return Ok((
                    prev.clone(),
                    SqliteToken {
                        schema_version: sv,
                        digest,
                    },
                    RefreshOutcome::SameDigest,
                ));
            }
        }
    }
    let (snap, token) = introspect(c, schema, level).await?;
    Ok((snap, token, RefreshOutcome::Reread))
}

/// The table options after the column list's closing parenthesis (`WITHOUT ROWID`, `STRICT`),
/// for when `pragma_table_list` is unavailable (SQLite < 3.37, or a read-only connection whose
/// authorizer does not allow it). A small scanner: strings, quoted identifiers, `[…]` and
/// comments are skipped so a `)` inside them is not the end.
pub fn table_options(sql: &str) -> (bool, bool) {
    let b = sql.as_bytes();
    // `code` is `sql` with strings, quoted identifiers and comments blanked.
    let mut code = b.to_vec();
    let mut blank = |from: usize, to: usize| {
        for c in &mut code[from..to.min(b.len())] {
            *c = b' ';
        }
    };
    let mut i = 0;
    let mut depth = 0i32;
    let mut close = None;
    while i < b.len() {
        let start = i;
        match b[i] {
            q @ (b'\'' | b'"' | b'`') => {
                i += 1;
                while i < b.len() {
                    if b[i] == q {
                        if i + 1 < b.len() && b[i + 1] == q {
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
                blank(start, i + 1);
            }
            b'[' => {
                while i < b.len() && b[i] != b']' {
                    i += 1;
                }
                blank(start, i + 1);
            }
            b'-' if b.get(i + 1) == Some(&b'-') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                blank(start, i);
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 1;
                blank(start, i + 1);
            }
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    let Some(end) = close else {
        return (false, false);
    };
    let tail = String::from_utf8_lossy(&code[end + 1..]).to_ascii_uppercase();
    let words: Vec<&str> = tail
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
        .collect();
    let without_rowid = words.windows(2).any(|w| w == ["WITHOUT", "ROWID"]);
    let strict = words.contains(&"STRICT");
    (without_rowid, strict)
}

/// `CREATE VIRTUAL TABLE x USING fts5(…)` → `fts5`.
fn virtual_module(sql: &str) -> Option<String> {
    let upper = sql.to_ascii_uppercase();
    if !upper.trim_start().starts_with("CREATE VIRTUAL TABLE") {
        return None;
    }
    let at = upper.find(" USING ")? + " USING ".len();
    let rest = &sql[at..];
    let end = rest
        .find(|c: char| c == '(' || c.is_whitespace())
        .unwrap_or(rest.len());
    Some(rest[..end].to_owned())
}

/// Reads one schema at `level`.
async fn read_schema(
    c: &mut dyn CatalogConn,
    schema: &str,
    level: Level,
) -> Result<SchemaSnapshot, IntroError> {
    let s = quote_ident(schema);
    let objects = c
        .rows(
            &format!("SELECT type, name, tbl_name, sql FROM {s}.sqlite_schema"),
            &[],
        )
        .await?;
    let mut rels: BTreeMap<String, Relation> = BTreeMap::new();
    let mut index_sql: HashMap<&str, Option<&str>> = HashMap::new();
    let mut snap = SchemaSnapshot {
        name: schema.to_owned(),
        level: Some(level),
        ..SchemaSnapshot::default()
    };
    for r in &objects {
        let (ty, name, sql) = (r.str(0), r.str(1), r.opt(3));
        match ty {
            "table" | "view" => {
                let module = sql.and_then(virtual_module);
                let kind = if ty == "view" {
                    RelationKind::View
                } else if module.is_some() {
                    RelationKind::VirtualTable
                } else {
                    RelationKind::Table
                };
                let (without_rowid, strict) = match (&kind, sql) {
                    (RelationKind::Table, Some(sql)) => table_options(sql),
                    _ => (false, false),
                };
                let view = (kind == RelationKind::View).then(|| ViewDef {
                    definition: if level >= Level::Sources {
                        sql.map(str::to_owned)
                    } else {
                        None
                    },
                    ..ViewDef::default()
                });
                rels.insert(
                    name.to_owned(),
                    Relation {
                        name: name.to_owned(),
                        kind: Some(kind),
                        engine: module,
                        without_rowid,
                        strict,
                        view,
                        sql: if level >= Level::Sources {
                            sql.map(str::to_owned)
                        } else {
                            None
                        },
                        ..Relation::default()
                    },
                );
            }
            "index" => {
                index_sql.insert(name, sql);
            }
            "trigger" if level >= Level::Structure => snap.triggers.push(Trigger {
                name: name.to_owned(),
                table: r.str(2).to_owned(),
                body: if level >= Level::Sources {
                    sql.map(str::to_owned)
                } else {
                    None
                },
                ..Trigger::default()
            }),
            _ => {}
        }
    }
    if level < Level::Structure {
        snap.relations = rels.into_values().collect();
        snap.normalize();
        return Ok(snap);
    }

    // Flags from `pragma_table_list` (3.37+) when allowed: it knows shadow tables too. The
    // text scan above stays the answer otherwise.
    if let Ok(list) = c
        .rows(
            "SELECT name, type, wr, strict FROM pragma_table_list WHERE schema = ?",
            &[schema],
        )
        .await
    {
        for r in &list {
            if let Some(rel) = rels.get_mut(r.str(0)) {
                rel.without_rowid = r.flag(2);
                rel.strict = r.flag(3);
                if r.str(1) == "shadow" {
                    rel.kind = Some(RelationKind::Shadow);
                }
            }
        }
    }

    // Columns: one query for tables, one for views; when one fails (a view over a dropped
    // table, a virtual table whose module is missing) that kind goes per relation, keeping
    // the failures as broken (HV-INTRO-08 edge 9). Split so one broken view does not turn
    // 10k tables into 10k queries.
    let mut col_rows: Vec<Row> = Vec::new();
    for ty in ["table", "view"] {
        let bulk = c
            .rows(
                &format!(
                    "SELECT m.name, p.cid, p.name, p.type, p.\"notnull\", p.dflt_value, p.pk, \
                     p.hidden FROM {s}.sqlite_schema m JOIN pragma_table_xinfo(m.name, ?) p \
                     WHERE m.type = '{ty}'"
                ),
                &[schema],
            )
            .await;
        match bulk {
            Ok(rows) => col_rows.extend(rows),
            Err(_) => {
                let names: Vec<String> = rels
                    .values()
                    .filter(|r| (r.kind == Some(RelationKind::View)) == (ty == "view"))
                    .map(|r| r.name.clone())
                    .collect();
                col_rows.extend(columns_one_by_one(c, schema, names, &mut rels).await?);
            }
        }
    }
    for r in &col_rows {
        let Some(rel) = rels.get_mut(r.str(0)) else {
            continue;
        };
        let hidden = r.i64(7).unwrap_or(0);
        rel.columns.push(Column {
            name: r.str(2).to_owned(),
            ordinal: r.u32(1).map_or(0, |c| c + 1),
            data_type: r.str(3).to_owned(),
            nullable: !r.flag(4),
            default: r.text(5),
            pk_position: r.u32(6).unwrap_or(0),
            hidden: hidden == 1,
            generated: match hidden {
                2 => Some((GeneratedKind::Virtual, None)),
                3 => Some((GeneratedKind::Stored, None)),
                _ => None,
            },
            ..Column::default()
        });
    }

    // Indexes and their key columns (auxiliary columns, `key = 0`, are left out).
    let idx = c
        .rows(
            &format!(
                "SELECT m.name, il.name, il.\"unique\", il.origin, il.partial \
                 FROM {s}.sqlite_schema m JOIN pragma_index_list(m.name, ?) il \
                 WHERE m.type = 'table'"
            ),
            &[schema],
        )
        .await
        .unwrap_or_default();
    let idx_cols = c
        .rows(
            &format!(
                "SELECT il.name, ix.seqno, ix.cid, ix.name, ix.\"desc\", ix.coll \
                 FROM {s}.sqlite_schema m JOIN pragma_index_list(m.name, ?) il \
                 JOIN pragma_index_xinfo(il.name, ?) ix \
                 WHERE m.type = 'table' AND ix.\"key\" = 1"
            ),
            &[schema, schema],
        )
        .await
        .unwrap_or_default();
    let mut parts: HashMap<&str, Vec<(u32, &Row)>> = HashMap::new();
    for r in &idx_cols {
        parts
            .entry(r.str(0))
            .or_default()
            .push((r.u32(1).unwrap_or(0), r));
    }
    for r in &idx {
        let Some(rel) = rels.get_mut(r.str(0)) else {
            continue;
        };
        let name = r.str(1);
        let mut ps = parts.remove(name).unwrap_or_default();
        ps.sort_by_key(|(s, _)| *s);
        let origin = r.str(3).to_owned();
        let columns: Vec<IndexColumn> = ps
            .iter()
            .map(|(_, k)| IndexColumn {
                // cid -2: an expression key part (its text is in the index's SQL).
                name: if k.i64(2) == Some(-2) {
                    None
                } else {
                    k.text(3)
                },
                expression: None,
                descending: k.flag(4),
                sub_part: None,
                collation: k.text(5),
            })
            .collect();
        if origin == "u" || origin == "pk" {
            rel.constraints.push(Constraint {
                name: None,
                kind: if origin == "pk" {
                    ConstraintKind::PrimaryKey
                } else {
                    ConstraintKind::Unique
                },
                columns: columns.iter().filter_map(|c| c.name.clone()).collect(),
                enforced: true,
            });
        }
        rel.indexes.push(Index {
            name: name.to_owned(),
            unique: r.flag(2),
            primary: origin == "pk",
            partial: r.flag(4),
            origin: Some(origin),
            sql: index_sql.get(name).copied().flatten().map(str::to_owned),
            columns,
            visible: true,
            ..Index::default()
        });
    }
    // An INTEGER PRIMARY KEY (the rowid alias) has no index: the PK comes from `pk`.
    for rel in rels.values_mut() {
        if matches!(rel.kind, Some(RelationKind::Table | RelationKind::Shadow))
            && !rel
                .constraints
                .iter()
                .any(|c| c.kind == ConstraintKind::PrimaryKey)
        {
            let mut pk: Vec<&Column> = rel.columns.iter().filter(|c| c.pk_position > 0).collect();
            pk.sort_by_key(|c| c.pk_position);
            if !pk.is_empty() {
                rel.constraints.push(Constraint {
                    name: None,
                    kind: ConstraintKind::PrimaryKey,
                    columns: pk.iter().map(|c| c.name.clone()).collect(),
                    enforced: true,
                });
            }
        }
    }

    // Foreign keys: one row per column pair, grouped by `id` per table.
    let fks = c
        .rows(
            &format!(
                "SELECT m.name, fk.id, fk.seq, fk.\"table\", fk.\"from\", fk.\"to\", \
                 fk.on_update, fk.on_delete, fk.\"match\" \
                 FROM {s}.sqlite_schema m JOIN pragma_foreign_key_list(m.name, ?) fk \
                 WHERE m.type = 'table'"
            ),
            &[schema],
        )
        .await
        .unwrap_or_default();
    let mut by_fk: BTreeMap<(&str, i64), Vec<(u32, &Row)>> = BTreeMap::new();
    for r in &fks {
        by_fk
            .entry((r.str(0), r.i64(1).unwrap_or(0)))
            .or_default()
            .push((r.u32(2).unwrap_or(0), r));
    }
    for ((table, _), mut ps) in by_fk {
        let Some(rel) = rels.get_mut(table) else {
            continue;
        };
        ps.sort_by_key(|(s, _)| *s);
        let first = ps[0].1;
        rel.constraints.push(Constraint {
            name: None,
            kind: ConstraintKind::ForeignKey(ForeignKey {
                ref_schema: None,
                ref_table: first.str(3).to_owned(),
                ref_columns: ps.iter().map(|(_, r)| r.text(5)).collect(),
                on_update: first.text(6),
                on_delete: first.text(7),
                match_option: first.text(8),
                ref_constraint: None,
            }),
            columns: ps.iter().map(|(_, r)| r.str(4).to_owned()).collect(),
            enforced: true,
        });
    }

    snap.relations = rels.into_values().collect();
    snap.normalize();
    Ok(snap)
}

/// The columns of `names`, one `pragma_table_xinfo` each; a failing relation is marked broken
/// with the engine's message.
async fn columns_one_by_one(
    c: &mut dyn CatalogConn,
    schema: &str,
    names: Vec<String>,
    rels: &mut BTreeMap<String, Relation>,
) -> Result<Vec<Row>, IntroError> {
    let mut all = Vec::new();
    for name in names {
        match c
            .rows(
                "SELECT ?, cid, name, type, \"notnull\", dflt_value, pk, hidden \
                 FROM pragma_table_xinfo(?, ?)",
                &[&name, &name, schema],
            )
            .await
        {
            Ok(rows) => all.extend(rows),
            Err(e) => {
                if let Some(rel) = rels.get_mut(&name) {
                    rel.broken = Some(e.server_text().unwrap_or("error").to_owned());
                }
            }
        }
    }
    Ok(all)
}

/// The connection's functions (`pragma_function_list`: name, builtin, type, narg), for the
/// `Routine` kind of the HV-MODEL-01 SQLite row. Connection-level, not per schema.
pub async fn functions(
    c: &mut dyn CatalogConn,
) -> Result<Vec<(String, bool, String, i64)>, IntroError> {
    let rows = c
        .rows(
            "SELECT name, builtin, type, narg FROM pragma_function_list",
            &[],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| {
            (
                r.str(0).to_owned(),
                r.flag(1),
                r.str(2).to_owned(),
                r.i64(3).unwrap_or(-1),
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_doubles_quotes() {
        assert_eq!(quote_ident("main"), "\"main\"");
        assert_eq!(quote_ident("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn table_options_scan_skips_strings_comments_and_nesting() {
        assert_eq!(table_options("CREATE TABLE t(a)"), (false, false));
        assert_eq!(
            table_options("CREATE TABLE t(a PRIMARY KEY) WITHOUT ROWID"),
            (true, false)
        );
        assert_eq!(
            table_options("CREATE TABLE t(a INT, CHECK (a > 0)) strict, without  rowid"),
            (true, true)
        );
        assert_eq!(
            table_options("CREATE TABLE t(a DEFAULT ')', b TEXT /* ) STRICT */) "),
            (false, false)
        );
        assert_eq!(
            table_options("CREATE TABLE \"x)\"(a) -- WITHOUT ROWID\n"),
            (false, false)
        );
        assert_eq!(
            table_options("CREATE TABLE [we(ird](a, b AS (a*2)) STRICT"),
            (false, true)
        );
    }

    #[test]
    fn virtual_module_is_read_from_using() {
        assert_eq!(
            virtual_module("CREATE VIRTUAL TABLE f USING fts5(a, b)").as_deref(),
            Some("fts5")
        );
        assert_eq!(
            virtual_module("CREATE VIRTUAL TABLE r USING rtree (id, x0, x1)").as_deref(),
            Some("rtree")
        );
        assert_eq!(virtual_module("CREATE TABLE t(a)"), None);
    }
}
