// ## m4-t13-assemble
//! SQLite assembly from the pragmas Task 18's bulk queries join (`pragma_table_xinfo`,
//! `pragma_index_list`, `pragma_index_xinfo`, `pragma_foreign_key_list`) and the index's
//! `sqlite_schema.sql`.
//!
//! - **Primary key** from the `pk` positions of `table_xinfo`. SQLite does not report a
//!   constraint name, so the key's name is the display name `<table>_pk`, flagged
//!   [`ObjFlags::SERVER_GENERATED_NAME`] (never emitted as a real name in DDL). An `INTEGER
//!   PRIMARY KEY` has no index; any other primary key is backed by its `sqlite_autoindex_*`
//!   (`origin` `pk`), which is [`ObjFlags::IMPLICIT`] for it.
//! - **Unique constraints** are their `sqlite_autoindex_*` (`origin` `u`): a unique key named
//!   by the server's index name (flagged `SERVER_GENERATED_NAME`) plus that index, implicit for
//!   it. A `CREATE [UNIQUE] INDEX` (`origin` `c`) is a plain index. The server numbers
//!   autoindexes by position, so a rebuild that drops an earlier one renames the later ones:
//!   that is the server's own name, kept as reported.
//! - **Index parts** are the key columns of `index_xinfo` (`key = 1`) in `seqno` order;
//!   `cid = -2` is an expression whose text, like a partial index's predicate, comes from the
//!   index's SQL (the scan below; `COLLATE` and `ASC`/`DESC` of a part are not part of its
//!   text). The scan is used only when its part count is the index's key count. A part's
//!   collation (`coll`, as Task 18 passes it) rides in the key's rare part.
//! - **Foreign keys** are unnamed. Each gets a display name derived from its **content**, never
//!   its position, so dropping or adding one FK leaves the others' identities alone:
//!   `<table>_<from columns>_<parent>_fkey`; when two FKs of the table share that, both take
//!   `<table>_<from columns>_<parent>_<to columns or "pk">_fkey`; a name still taken (exact
//!   duplicates, which SQLite allows) gets `_2`, `_3`, … in declaration order; a name over
//!   [`MAX_FK_NAME`] bytes is `<table>_fk_<xxh3-64 of from, parent and to, hex>`. All are
//!   flagged `SERVER_GENERATED_NAME`. A `to` column of NULL references the parent's primary
//!   key: `target_columns` is then empty. A parent that resolves is named by its stored
//!   spelling (`REFERENCES P` names table `p`).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use hacivat_meta::props::{FkProps, IndexProps, KeyProps, Props};
use hacivat_meta::{Name, Obj, ObjFlags, ObjectKind, Ref};

use super::{
    ColRef, ColumnIndex, KeyObj, PartOpts, Parts, TableId, finish, fk_action_word, fk_match_word,
    flag_if, index_obj, key_obj, key_props, mark_implicit,
};

/// The longest content-derived FK display name, in bytes; a longer one is hashed.
pub const MAX_FK_NAME: usize = 128;

/// One `table_xinfo` column with `pk > 0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PkColumnRow {
    pub table: Name,
    pub cid: i64,
    pub name: Name,
    /// The 1-based position in the primary key.
    pub pk: u32,
}

/// One `index_list` row, with the index's `sqlite_schema.sql`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexRow {
    pub table: Name,
    pub name: Name,
    pub unique: bool,
    /// `c` (`CREATE INDEX`), `u` (`UNIQUE` constraint), `pk` (primary key).
    pub origin: Name,
    pub partial: bool,
    /// `sqlite_schema.sql` (NULL for an automatic index).
    pub sql: Option<Name>,
}

/// One `index_xinfo` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexPartRow {
    pub index: Name,
    pub seqno: u32,
    /// The table column's rank; `-1` the rowid, `-2` an expression.
    pub cid: i64,
    pub name: Option<Name>,
    pub desc: bool,
    /// `coll` when it differs from the column's declared collation (`BINARY` when it declares
    /// none; for an expression part, from `BINARY`). Task 18 compares; `None` otherwise.
    pub coll: Option<Name>,
    /// A key column (`false`: an auxiliary column the index stores).
    pub key: bool,
}

/// One `foreign_key_list` row: one column of one FK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FkRow {
    pub table: Name,
    /// The FK's number in its table (0 = the last declared).
    pub id: u32,
    /// The column's position in the FK.
    pub seq: u32,
    /// `table`: the parent table, as the `REFERENCES` clause spells it.
    pub ref_table: Name,
    pub from: Name,
    /// `to` (NULL: the parent's primary key).
    pub to: Option<Name>,
    pub on_update: Name,
    pub on_delete: Name,
    /// `match` (`NONE`).
    pub match_kind: Name,
    /// `DEFERRABLE`, which `foreign_key_list` does not report: Task 18 parses it from the
    /// table's `sqlite_schema.sql` (absent: SQLite's default, not deferrable).
    pub deferrable: bool,
    /// `INITIALLY DEFERRED`, parsed the same way.
    pub initially_deferred: bool,
}

/// The SQLite rows of one schema (`main`, `temp` or an attached one).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyRows {
    pub pk_columns: Vec<PkColumnRow>,
    pub indexes: Vec<IndexRow>,
    pub index_parts: Vec<IndexPartRow>,
    pub foreign_keys: Vec<FkRow>,
}

fn part_opts(p: &IndexPartRow) -> PartOpts {
    PartOpts {
        desc: p.desc,
        collation: p.coll.clone(),
        ..PartOpts::default()
    }
}

/// Assembles primary keys, unique keys, indexes and foreign keys (module docs).
pub fn assemble_keys(rows: KeyRows, cols: &ColumnIndex) -> Vec<KeyObj> {
    let mut out = Vec::new();
    let mut parts_of: HashMap<&str, Vec<&IndexPartRow>> = HashMap::new();
    for p in &rows.index_parts {
        parts_of.entry(&p.index).or_default().push(p);
    }

    // Primary keys by table.
    let mut pks: BTreeMap<&str, Vec<&PkColumnRow>> = BTreeMap::new();
    for c in &rows.pk_columns {
        pks.entry(&c.table).or_default().push(c);
    }
    let mut pk_at: HashMap<&str, usize> = HashMap::new();
    for (table, mut cs) in pks {
        cs.sort_by_key(|c| c.pk);
        let tid = TableId::named(table);
        let backing = rows
            .indexes
            .iter()
            .find(|i| *i.table == *table && &*i.origin == "pk");
        // The direction and collation per part come from the backing autoindex.
        let ix_parts = backing
            .and_then(|ix| parts_of.get(&*ix.name))
            .map(|ps| key_parts(ps))
            .filter(|ps| ps.len() == cs.len());
        let mut p = Parts::default();
        for (i, c) in cs.iter().enumerate() {
            let ordinal = u32::try_from(c.cid).ok().and_then(|c| c.checked_add(1));
            let o = ix_parts
                .as_ref()
                .map_or_else(PartOpts::default, |ps| part_opts(ps[i]));
            p.column(
                cols,
                &tid,
                &ColRef {
                    ordinal,
                    name: Some(c.name.clone()),
                },
                o,
            );
        }
        let (props, broken) = p.into_key(KeyProps {
            unique: true,
            backing_index: backing.map(|ix| ix.name.clone()),
            ..key_props()
        });
        pk_at.insert(table, out.len());
        out.push(KeyObj::new(
            tid,
            key_obj(ObjectKind::PrimaryKey, &pk_name(table), props)
                .add_flags(ObjFlags::SERVER_GENERATED_NAME | flag_if(broken, ObjFlags::BROKEN)),
        ));
    }

    // Indexes, and the unique keys of `origin = 'u'` autoindexes.
    let mut links = Vec::new();
    for ix in &rows.indexes {
        let tid = TableId::named(ix.table.clone());
        let parts = parts_of
            .get(&*ix.name)
            .map(|ps| key_parts(ps))
            .unwrap_or_default();
        // m2: the scan is trusted only when it found as many parts as the index has.
        let sql = ix
            .sql
            .as_deref()
            .and_then(scan_index_sql)
            .filter(|s| s.parts.len() == parts.len());
        let mut p = Parts::default();
        let mut broken = parts.is_empty();
        for (i, part) in parts.iter().enumerate() {
            broken |= usize::try_from(part.seqno).ok() != Some(i);
            match part.cid {
                -2 => {
                    let text = sql
                        .as_ref()
                        .and_then(|s| s.parts.get(i))
                        .map(|t| Name::from(*t));
                    p.expression(text.as_ref(), part_opts(part));
                }
                cid => {
                    let ordinal = u32::try_from(cid).ok().and_then(|c| c.checked_add(1));
                    let name = part
                        .name
                        .clone()
                        .or_else(|| (cid == -1).then(|| Name::from("rowid")));
                    p.column(cols, &tid, &ColRef { ordinal, name }, part_opts(part));
                }
            }
        }
        let predicate = sql.as_ref().and_then(|s| s.predicate).map(Name::from);
        broken |= ix.partial && predicate.is_none();
        let origin = &*ix.origin;
        let auto = origin == "u" || origin == "pk";
        let (key, parts_broken) = p.into_key(KeyProps {
            predicate,
            unique: ix.unique,
            ..key_props()
        });
        broken |= parts_broken;
        let flags =
            flag_if(broken, ObjFlags::BROKEN) | flag_if(auto, ObjFlags::SERVER_GENERATED_NAME);
        let index_at = out.len();
        out.push(KeyObj::new(
            tid.clone(),
            index_obj(
                &ix.name,
                IndexProps {
                    key: key.clone(),
                    method: None,
                    prefix_len: Vec::new(),
                    comment: None,
                },
            )
            .add_flags(flags),
        ));
        match origin {
            "u" => {
                links.push((out.len(), index_at));
                out.push(KeyObj::new(
                    tid,
                    key_obj(
                        ObjectKind::UniqueKey,
                        &ix.name,
                        KeyProps {
                            backing_index: Some(ix.name.clone()),
                            ..key
                        },
                    )
                    .add_flags(flags),
                ));
            }
            "pk" => {
                if let Some(&pk) = pk_at.get(&*ix.table) {
                    links.push((pk, index_at));
                }
            }
            _ => {}
        }
    }

    // Foreign keys by table, in declaration order (the highest id was declared first).
    let mut fks: BTreeMap<&str, BTreeMap<std::cmp::Reverse<u32>, Vec<&FkRow>>> = BTreeMap::new();
    for r in &rows.foreign_keys {
        fks.entry(&r.table)
            .or_default()
            .entry(std::cmp::Reverse(r.id))
            .or_default()
            .push(r);
    }
    for (table, by_id) in fks {
        let built: Vec<BuiltFk> = by_id
            .into_values()
            .map(|mut fk| {
                fk.sort_by_key(|r| r.seq);
                foreign_key(table, &fk, cols)
            })
            .collect();
        for (fk, name) in built.iter().zip(fk_names(table, &built)) {
            out.push(KeyObj::new(
                TableId::named(table),
                Obj::new(ObjectKind::ForeignKey, name)
                    .with_props(Props::ForeignKey(Arc::new(fk.props.clone())))
                    .add_flags(
                        ObjFlags::SERVER_GENERATED_NAME | flag_if(fk.broken, ObjFlags::BROKEN),
                    ),
            ));
        }
    }

    for (owner, idx) in links {
        let owner = out[owner].clone();
        mark_implicit(cols, &mut out[idx], &owner);
    }
    finish(out)
}

/// The display name of an unnamed primary key.
fn pk_name(table: &str) -> Name {
    Name::from(format!("{table}_pk").as_str())
}

/// The key columns of an index, in `seqno` order.
fn key_parts<'a>(parts: &[&'a IndexPartRow]) -> Vec<&'a IndexPartRow> {
    let mut keys: Vec<&IndexPartRow> = parts.iter().copied().filter(|p| p.key).collect();
    keys.sort_by_key(|p| p.seqno);
    keys
}

/// One assembled FK before it is named.
struct BuiltFk {
    /// The parent's stored name (or its `REFERENCES` spelling when unknown).
    parent: Name,
    props: FkProps,
    broken: bool,
}

/// The content-derived display names of a table's FKs, in the order given (declaration order;
/// module docs).
fn fk_names(table: &str, fks: &[BuiltFk]) -> Vec<Name> {
    let join = |names: &[Name]| names.iter().map(|n| &**n).collect::<Vec<_>>().join("_");
    let base = |fk: &BuiltFk| format!("{table}_{}_{}_fkey", join(&fk.props.columns), fk.parent);
    let mut counts: HashMap<String, usize> = HashMap::new();
    for fk in fks {
        *counts.entry(base(fk)).or_default() += 1;
    }
    let mut taken: HashMap<String, usize> = HashMap::new();
    fks.iter()
        .map(|fk| {
            let b = base(fk);
            let mut name = if counts.get(&b).copied().unwrap_or(0) > 1 {
                let to = if fk.props.target_columns.is_empty() {
                    "pk".to_string()
                } else {
                    join(&fk.props.target_columns)
                };
                format!(
                    "{table}_{}_{}_{to}_fkey",
                    join(&fk.props.columns),
                    fk.parent
                )
            } else {
                b
            };
            if name.len() > MAX_FK_NAME {
                let mut text = String::new();
                for part in [
                    &fk.props.columns[..],
                    std::slice::from_ref(&fk.parent),
                    &fk.props.target_columns[..],
                ] {
                    text.push_str(&join(part));
                    text.push('\0');
                }
                let h = twox_hash::XxHash3_64::oneshot(text.as_bytes());
                name = format!("{table}_fk_{h:016x}");
            }
            let seen = taken.entry(name.clone()).or_default();
            *seen += 1;
            if *seen > 1 {
                let mut k = *seen;
                loop {
                    let candidate = format!("{name}_{k}");
                    if !taken.contains_key(&candidate) {
                        taken.insert(candidate.clone(), 1);
                        name = candidate;
                        break;
                    }
                    k += 1;
                }
            }
            Name::from(name.as_str())
        })
        .collect()
}

fn foreign_key(table: &str, rows: &[&FkRow], cols: &ColumnIndex) -> BuiltFk {
    let tid = TableId::named(table);
    let first = rows[0];
    let target_id = TableId::named(first.ref_table.clone());
    let target = cols.target(None, &target_id);
    let mut broken = target.dangling;
    let mut columns = Vec::with_capacity(rows.len());
    let mut target_columns = Vec::with_capacity(rows.len());
    let named_targets = rows.iter().filter(|r| r.to.is_some()).count();
    // Either every `to` is named or none (the parent's primary key); a mix is broken.
    broken |= named_targets != 0 && named_targets != rows.len();
    for (i, r) in rows.iter().enumerate() {
        broken |= usize::try_from(r.seq).ok() != Some(i) || *r.ref_table != *first.ref_table;
        match cols.resolve(&tid, &ColRef::named(r.from.clone())) {
            Some(kc) => columns.extend(kc.name),
            None => {
                broken = true;
                columns.push(r.from.clone());
            }
        }
        if let Some(to) = &r.to {
            match cols
                .resolve(&target_id, &ColRef::named(to.clone()))
                .filter(|_| target.known)
            {
                Some(kc) => target_columns.extend(kc.name),
                None => {
                    // An unknown parent keeps the text; a known parent without it is broken.
                    broken |= target.known;
                    target_columns.push(to.clone());
                }
            }
        }
    }
    let on_update = fk_action_word(&first.on_update);
    let on_delete = fk_action_word(&first.on_delete);
    let match_kind = fk_match_word(&first.match_kind);
    broken |= on_update.is_none() || on_delete.is_none() || match_kind.is_none();
    let parent = target
        .path
        .last()
        .cloned()
        .unwrap_or_else(|| first.ref_table.clone());
    let props = FkProps {
        columns,
        target: Ref {
            kind: ObjectKind::Table,
            path: target.path,
            server_id: target.server_id,
        },
        target_columns,
        on_update: on_update.unwrap_or_default(),
        on_delete: on_delete.unwrap_or_default(),
        match_kind: match_kind.unwrap_or_default(),
        deferrable: first.deferrable,
        initially_deferred: first.initially_deferred,
        not_valid: false,
        enforced: true,
        rare: None,
    };
    BuiltFk {
        parent,
        props,
        broken,
    }
}

/// The indexed parts and the predicate of a `CREATE INDEX` statement's text.
#[derive(Debug, PartialEq, Eq)]
struct IndexSql<'a> {
    /// Each part's text without its `COLLATE` and `ASC`/`DESC`, trimmed.
    parts: Vec<&'a str>,
    /// The `WHERE` expression, trimmed.
    predicate: Option<&'a str>,
}

/// Scans `CREATE [UNIQUE] INDEX … ON <table> ( <part>, … ) [WHERE <expr>]`: the first
/// top-level `(` opens the part list. Strings (`'…'`), quoted names (`"…"`, `` `…` ``, `[…]`)
/// and comments (`--`, `/* */`) are skipped. `None` when the text does not have that shape.
fn scan_index_sql(sql: &str) -> Option<IndexSql<'_>> {
    let b = sql.as_bytes();
    let mut i = 0;
    let mut depth = 0usize;
    let mut open = None;
    let mut starts = Vec::new();
    let mut parts = Vec::new();
    while i < b.len() {
        let c = b[i];
        match c {
            b'\'' | b'"' | b'`' => i = skip_quoted(b, i, c)?,
            b'[' => i = skip_quoted(b, i, b']')?,
            b'-' if b.get(i + 1) == Some(&b'-') => {
                i = b[i..]
                    .iter()
                    .position(|&x| x == b'\n')
                    .map_or(b.len(), |p| i + p);
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = find(b, i + 2, b"*/").map_or(b.len(), |p| p + 2);
                continue;
            }
            b'(' => {
                depth += 1;
                if depth == 1 && open.is_none() {
                    open = Some(i);
                    starts.push(i + 1);
                }
                i += 1;
            }
            b')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 && open.is_some() {
                    parts.push(clean_part(&sql[*starts.last()?..i]));
                    let rest = &sql[i + 1..];
                    return Some(IndexSql {
                        parts,
                        predicate: predicate(rest)?,
                    });
                }
                i += 1;
            }
            b',' if depth == 1 && open.is_some() => {
                parts.push(clean_part(&sql[*starts.last()?..i]));
                starts.push(i + 1);
                i += 1;
            }
            _ => i += 1,
        }
    }
    None
}

/// The index after the closing quote `close` of the quoted run starting at `at` (a doubled
/// closing quote is an escaped one).
fn skip_quoted(b: &[u8], at: usize, close: u8) -> Option<usize> {
    let mut i = at + 1;
    while i < b.len() {
        if b[i] == close {
            if close != b']' && b.get(i + 1) == Some(&close) {
                i += 2;
                continue;
            }
            return Some(i + 1);
        }
        i += 1;
    }
    None
}

fn find(b: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    b.get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| from + p)
}

/// What follows the part list: nothing, or `WHERE <expr>` (comments and spaces around).
/// `Some(None)`: no predicate; `None`: something else follows.
fn predicate(rest: &str) -> Option<Option<&str>> {
    let rest = skip_space(rest);
    if rest.is_empty() {
        return Some(None);
    }
    let kw = rest.get(..5)?;
    let after = &rest[5..];
    let boundary = after.bytes().next().is_none_or(|c| !is_word(c));
    if !kw.eq_ignore_ascii_case("WHERE") || !boundary {
        return None;
    }
    let expr = skip_space(after).trim_end();
    if expr.is_empty() {
        None
    } else {
        Some(Some(expr))
    }
}

/// `s` after leading white space and comments.
fn skip_space(mut s: &str) -> &str {
    loop {
        let t = s.trim_start();
        if let Some(r) = t.strip_prefix("--") {
            s = r.find('\n').map_or("", |p| &r[p..]);
        } else if let Some(r) = t.strip_prefix("/*") {
            s = r.find("*/").map_or("", |p| &r[p + 2..]);
        } else {
            return t;
        }
    }
}

fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80
}

/// A part's text without a trailing `ASC`/`DESC` and then a trailing `COLLATE <name>`.
fn clean_part(part: &str) -> &str {
    let mut p = part.trim();
    if let Some(rest) = strip_word(p, "DESC").or_else(|| strip_word(p, "ASC")) {
        p = rest;
    }
    if let Some((before, last)) = last_token(p)
        && !last.is_empty()
        && let Some(rest) = strip_word(before, "COLLATE")
    {
        p = rest;
    }
    p
}

/// `s` without the trailing keyword `word` (ASCII-caseless, a whole word), trimmed.
fn strip_word<'a>(s: &'a str, word: &str) -> Option<&'a str> {
    let s = s.trim_end();
    let at = s.len().checked_sub(word.len())?;
    let tail = s.get(at..)?;
    if !tail.eq_ignore_ascii_case(word) {
        return None;
    }
    let before = &s[..at];
    let ok = before
        .bytes()
        .last()
        .is_some_and(|c| !is_word(c) && c != b'"' && c != b'`');
    ok.then(|| before.trim_end())
}

/// The last token of `s` (a bare word or a quoted name) and the text before it.
fn last_token(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_end();
    let b = s.as_bytes();
    let last = *b.last()?;
    let start = match last {
        b'"' | b'`' | b']' => {
            let open = if last == b']' { b'[' } else { last };
            s[..s.len() - 1].rfind(char::from(open))?
        }
        c if is_word(c) => b.iter().rposition(|&c| !is_word(c)).map_or(0, |p| p + 1),
        _ => return None,
    };
    Some((&s[..start], &s[start..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_parts_and_predicate() {
        let s = scan_index_sql(
            "CREATE UNIQUE INDEX \"a(\"\"b\" ON t ( lower(x) COLLATE \"NoCase\" desc , [y z] ASC, f(a, ',') ) -- c\n WHERE x <> 'a)b'",
        )
        .unwrap();
        assert_eq!(s.parts, vec!["lower(x)", "[y z]", "f(a, ',')"]);
        assert_eq!(s.predicate, Some("x <> 'a)b'"));
        assert_eq!(
            scan_index_sql("CREATE INDEX i ON t(a)"),
            Some(IndexSql {
                parts: vec!["a"],
                predicate: None
            })
        );
    }

    #[test]
    fn refuses_what_is_not_an_index_statement() {
        assert_eq!(scan_index_sql("CREATE INDEX i ON t(a) garbage"), None);
        assert_eq!(scan_index_sql("CREATE INDEX i ON t(a"), None);
        assert_eq!(scan_index_sql("CREATE INDEX 'i ON t(a)"), None);
        assert_eq!(scan_index_sql("CREATE INDEX i ON t(a) WHEREx"), None);
    }

    #[test]
    fn keeps_words_that_only_end_like_keywords() {
        assert_eq!(clean_part("xdesc"), "xdesc");
        assert_eq!(clean_part("\"desc\""), "\"desc\"");
        assert_eq!(clean_part("a desc"), "a");
        assert_eq!(clean_part("a collate"), "a collate");
        assert_eq!(clean_part("(a) COLLATE nocase"), "(a)");
    }
}
