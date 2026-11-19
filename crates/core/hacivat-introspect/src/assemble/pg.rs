// ## m4-t13-assemble
//! PostgreSQL assembly: one row per index (`pg_index` + its `pg_class` row) and one per
//! constraint (`pg_constraint`, `conrelid <> 0`), with ordered arrays (`indkey`/`indoption`/
//! `indclass`/`indcollation`, `conkey`, `confkey`, `conexclop`) unnested in key order by the
//! pack's query (Task 16).
//!
//! - Index key slots with `attnum` 0 are expressions; their text is `pg_get_indexdef(oid, k,
//!   true)`, selected only for those slots. `NULLS FIRST`, a non-default operator class and a
//!   collation other than the column's ride in the key's rare part (R-W2-8).
//! - A PK, UNIQUE or EXCLUDE constraint's `conindid` names its backing index: the index is
//!   [`ObjFlags::IMPLICIT`] with `implicit_for` the constraint, and the key carries the index's
//!   name (`backing_index`), `INCLUDE` columns, `indnullsnotdistinct` (PostgreSQL 15+) and the
//!   per-part options. A PK or UNIQUE whose backing index row is not in the pass is incomplete
//!   and flagged broken: Task 16 reads a constraint's index in the same pass.
//! - An FK's `conindid` names the **referenced** table's index: it becomes
//!   `FkProps`' `fk_ref_index`, never an implicit link.
//! - A CHECK stores the bare expression (`pg_get_expr(conbin, conrelid, true)`); an EXCLUDE
//!   stores the text after `EXCLUDE` without the deferral words, its operators and deferral in
//!   `CheckRare` (the DDL contract: the producer writes `CHECK (expr)` and `EXCLUDE expr`).
//! - A constraint that is not local (`conislocal` false) or was cloned from a partitioned
//!   parent (`conparentid`) is [`ObjFlags::INHERITED`]: DDL does not repeat it.
//! - `contype` `t` (constraint triggers) and `n` (catalogued NOT NULL, Task 16's column props)
//!   are not assembled here.

use std::collections::HashMap;
use std::sync::Arc;

use hacivat_meta::props::{
    CheckProps, CheckRare, FkAction, FkMatch, FkProps, FkRare, IndexProps, KeyProps, Props,
};
use hacivat_meta::{Name, Obj, ObjFlags, ObjectKind, Ref, ServerId};

use super::{
    ColRef, ColumnIndex, KeyObj, PartOpts, Parts, TableId, finish, flag_if, index_obj, key_obj,
    key_props, mark_implicit, resolve_names, target_names,
};

/// One key slot of an index (`indkey[k]` with `indoption[k]`, `indclass[k]`,
/// `indcollation[k]`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexPart {
    /// The column's `attnum`; 0 for an expression slot.
    pub attnum: i16,
    /// The column's `attname` (none for an expression slot).
    pub name: Option<Name>,
    /// `pg_get_indexdef(oid, k, true)` for an expression slot.
    pub expression: Option<Name>,
    /// `indoption` bit 0.
    pub desc: bool,
    /// `indoption` bit 1, raw.
    pub nulls_first: bool,
    /// `indclass` when it is not the type's default operator class: schema-qualified outside
    /// `pg_catalog`, with its parameters.
    pub opclass: Option<Name>,
    /// `indcollation` when it differs from the column's collation.
    pub collation: Option<Name>,
}

/// One index of a relation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexRow {
    /// `indrelid` and its name.
    pub table: TableId,
    pub oid: u32,
    pub name: Name,
    /// The access method (`btree`, `gist`, …).
    pub method: Option<Name>,
    pub unique: bool,
    /// `indisvalid`.
    pub valid: bool,
    /// `indnullsnotdistinct` (PostgreSQL 15+; false before).
    pub nulls_not_distinct: bool,
    /// The key slots (the first `indnkeyatts`), in key order.
    pub parts: Vec<IndexPart>,
    /// The `INCLUDE` columns (after `indnkeyatts`).
    pub include: Vec<ColRef>,
    /// `pg_get_expr(indpred, indrelid)`.
    pub predicate: Option<Name>,
    pub comment: Option<Name>,
}

/// A foreign key's referenced side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FkTarget {
    /// `confrelid` and its name.
    pub table: TableId,
    /// The referenced table's schema (the FK may point out of this namespace).
    pub schema: Name,
    /// `confkey` with the referenced table's `attname`s, in FK order.
    pub columns: Vec<ColRef>,
    /// `confupdtype` / `confdeltype`: `a` no action, `r` restrict, `c` cascade, `n` set null,
    /// `d` set default.
    pub on_update: u8,
    pub on_delete: u8,
    /// `confmatchtype`: `s` simple, `f` full, `p` partial.
    pub match_type: u8,
    /// `conindid` and the index's name: the referenced table's unique index.
    pub index: Option<(u32, Name)>,
}

/// One constraint of a relation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstraintRow {
    /// `conrelid` and its name.
    pub table: TableId,
    pub oid: u32,
    pub name: Name,
    /// `contype`: `p`, `u`, `f`, `c`, `x` are assembled; `t` and `n` are skipped.
    pub contype: u8,
    /// `conkey` with the `attname`s, in key order (0 for an exclusion's expression element).
    pub columns: Vec<ColRef>,
    /// `pg_get_constraintdef(oid, true)`: an exclusion constraint's text comes from it.
    pub definition: Name,
    /// `pg_get_expr(conbin, conrelid, true)` of a CHECK: its bare expression.
    pub check_expr: Option<Name>,
    pub deferrable: bool,
    /// `condeferred`.
    pub deferred: bool,
    /// `convalidated` (false: `NOT VALID`).
    pub validated: bool,
    /// `conenforced` (PostgreSQL 18+; true before).
    pub enforced: bool,
    /// `connoinherit`.
    pub no_inherit: bool,
    /// `conislocal`.
    pub local: bool,
    /// `conparentid` (0 → `None`): the partitioned parent's constraint it was cloned from.
    pub parent: Option<u32>,
    /// `conindid` of a PK, UNIQUE or EXCLUDE constraint: its backing index.
    pub index: Option<u32>,
    /// A foreign key's referenced side.
    pub foreign: Option<FkTarget>,
    /// An exclusion constraint's operators (`conexclop` names, schema-qualified outside
    /// `pg_catalog`), in key order.
    pub operators: Vec<Name>,
}

/// The PostgreSQL rows of one namespace (or one fragment).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyRows {
    pub indexes: Vec<IndexRow>,
    pub constraints: Vec<ConstraintRow>,
}

/// Assembles keys, indexes, foreign keys, checks and exclusion constraints (module docs).
pub fn assemble_keys(rows: KeyRows, cols: &ColumnIndex) -> Vec<KeyObj> {
    let mut out = Vec::with_capacity(rows.indexes.len() + rows.constraints.len());
    let mut index_at: HashMap<(Option<ServerId>, u32), usize> = HashMap::new();
    for row in &rows.indexes {
        index_at.insert((row.table.server_id, row.oid), out.len());
        out.push(index(row, cols));
    }
    // (constraint position in `out`, its backing index position).
    let mut links = Vec::new();
    for row in &rows.constraints {
        let backing = match row.contype {
            b'p' | b'u' | b'x' => row
                .index
                .and_then(|oid| index_at.get(&(row.table.server_id, oid)).copied()),
            _ => None,
        };
        let backing_row = backing.map(|i| &rows.indexes[i]);
        if let Some(obj) = constraint(row, backing_row, cols) {
            if let Some(i) = backing {
                links.push((out.len(), i));
            }
            out.push(obj);
        }
    }
    for (con, idx) in links {
        let owner = out[con].clone();
        mark_implicit(cols, &mut out[idx], &owner);
    }
    finish(out)
}

fn opts(part: &IndexPart) -> PartOpts {
    PartOpts {
        desc: part.desc,
        nulls_first: part.nulls_first,
        opclass: part.opclass.clone(),
        collation: part.collation.clone(),
    }
}

fn index(row: &IndexRow, cols: &ColumnIndex) -> KeyObj {
    let (include, inc_broken) = resolve_names(cols, &row.table, &row.include);
    let (key, broken) = parts(row, cols).into_key(KeyProps {
        include,
        predicate: row.predicate.clone(),
        unique: row.unique,
        nulls_not_distinct: row.nulls_not_distinct,
        ..key_props()
    });
    let flags =
        flag_if(broken || inc_broken, ObjFlags::BROKEN) | flag_if(!row.valid, ObjFlags::INVALID);
    let props = IndexProps {
        key,
        method: row.method.clone(),
        prefix_len: Vec::new(),
        comment: row.comment.clone(),
    };
    KeyObj::new(
        row.table.clone(),
        index_obj(&row.name, props)
            .with_server_id(ServerId::Oid(row.oid))
            .add_flags(flags),
    )
}

fn parts(row: &IndexRow, cols: &ColumnIndex) -> Parts {
    let mut p = Parts::default();
    for part in &row.parts {
        match u32::try_from(part.attnum) {
            Ok(0) => p.expression(part.expression.as_ref(), opts(part)),
            Ok(attnum) => p.column(
                cols,
                &row.table,
                &ColRef {
                    ordinal: Some(attnum),
                    name: part.name.clone(),
                },
                opts(part),
            ),
            // A system column (negative attnum) is not a column of the table's list.
            Err(_) => p.column(
                cols,
                &row.table,
                &ColRef {
                    ordinal: None,
                    name: Some(part.name.clone().unwrap_or_else(|| Name::from(""))),
                },
                opts(part),
            ),
        }
    }
    p
}

fn constraint(
    row: &ConstraintRow,
    backing: Option<&IndexRow>,
    cols: &ColumnIndex,
) -> Option<KeyObj> {
    let flags = flag_if(!row.validated, ObjFlags::INVALID)
        | flag_if(!row.local || row.parent.is_some(), ObjFlags::INHERITED);
    let ko = match row.contype {
        b'p' | b'u' => key(row, backing, cols),
        b'f' => foreign_key(row, cols),
        b'c' | b'x' => check(row, cols),
        _ => return None,
    };
    let KeyObj { table, obj } = ko;
    Some(KeyObj {
        table,
        obj: obj.with_server_id(ServerId::Oid(row.oid)).add_flags(flags),
    })
}

fn key(row: &ConstraintRow, backing: Option<&IndexRow>, cols: &ColumnIndex) -> KeyObj {
    let kind = if row.contype == b'p' {
        ObjectKind::PrimaryKey
    } else {
        ObjectKind::UniqueKey
    };
    let mut p = Parts::default();
    // The per-part options come from the backing index when its slots line up.
    let ix_parts = backing
        .map(|ix| ix.parts.as_slice())
        .filter(|ps| ps.len() == row.columns.len());
    for (i, c) in row.columns.iter().enumerate() {
        let o = ix_parts.map_or_else(PartOpts::default, |ps| opts(&ps[i]));
        p.column(cols, &row.table, c, o);
    }
    let mut base = KeyProps {
        unique: true,
        deferrable: row.deferrable,
        initially_deferred: row.deferred,
        not_valid: !row.validated,
        enforced: row.enforced,
        ..key_props()
    };
    // m1: without its backing index row the key's INCLUDE, NULLS NOT DISTINCT and part
    // options are unknown: it is kept, flagged broken.
    let mut broken = backing.is_none();
    if let Some(ix) = backing {
        let (include, inc_broken) = resolve_names(cols, &row.table, &ix.include);
        broken |= inc_broken;
        base.include = include;
        base.nulls_not_distinct = ix.nulls_not_distinct;
        base.backing_index = Some(ix.name.clone());
    }
    let (props, parts_broken) = p.into_key(base);
    KeyObj::new(
        row.table.clone(),
        key_obj(kind, &row.name, props)
            .add_flags(flag_if(broken || parts_broken, ObjFlags::BROKEN)),
    )
}

fn check(row: &ConstraintRow, cols: &ColumnIndex) -> KeyObj {
    let exclusion = row.contype == b'x';
    let kind = if exclusion {
        ObjectKind::Exclusion
    } else {
        ObjectKind::Check
    };
    // Expression elements of an exclusion constraint are 0 in `conkey`.
    let mut broken = row
        .columns
        .iter()
        .filter(|c| c.ordinal != Some(0))
        .any(|c| cols.resolve(&row.table, c).is_none());
    let expr = if exclusion {
        broken |= row.operators.len() != row.columns.len();
        exclusion_body(&row.definition)
    } else {
        row.check_expr.as_deref()
    };
    broken |= expr.is_none();
    let rare = CheckRare {
        no_inherit: row.no_inherit,
        excl_ops: if exclusion {
            row.operators.clone()
        } else {
            Vec::new()
        },
        excl_deferrable: exclusion && row.deferrable,
        excl_initially_deferred: exclusion && row.deferred,
    };
    let props = CheckProps {
        expr: Name::from(expr.unwrap_or("")),
        not_valid: !row.validated,
        enforced: row.enforced,
        rare: (rare != CheckRare::default()).then(|| Arc::new(rare)),
    };
    KeyObj::new(
        row.table.clone(),
        Obj::new(kind, row.name.clone())
            .with_props(Props::Check(Arc::new(props)))
            .add_flags(flag_if(broken, ObjFlags::BROKEN)),
    )
}

/// The text after `EXCLUDE` of `pg_get_constraintdef`, without the deferral words the server
/// appends after the last `)` (` DEFERRABLE`, ` INITIALLY DEFERRED`). `None` when the text does
/// not start with `EXCLUDE `.
fn exclusion_body(def: &str) -> Option<&str> {
    let mut body = def.trim().strip_prefix("EXCLUDE ")?.trim();
    loop {
        let shorter = [" INITIALLY DEFERRED", " INITIALLY IMMEDIATE", " DEFERRABLE"]
            .iter()
            .find_map(|w| body.strip_suffix(w));
        match shorter {
            Some(b) => body = b.trim_end(),
            None => break,
        }
    }
    (!body.is_empty()).then_some(body)
}

fn foreign_key(row: &ConstraintRow, cols: &ColumnIndex) -> KeyObj {
    let (columns, mut broken) = resolve_names(cols, &row.table, &row.columns);
    let Some(f) = &row.foreign else {
        // An FK row without its referenced side: kept, broken.
        let props = FkProps {
            columns,
            target: Ref {
                kind: ObjectKind::Table,
                path: Default::default(),
                server_id: None,
            },
            target_columns: Vec::new(),
            on_update: FkAction::NoAction,
            on_delete: FkAction::NoAction,
            match_kind: FkMatch::Simple,
            deferrable: row.deferrable,
            initially_deferred: row.deferred,
            not_valid: !row.validated,
            enforced: row.enforced,
            rare: None,
        };
        return KeyObj::new(
            row.table.clone(),
            Obj::new(ObjectKind::ForeignKey, row.name.clone())
                .with_props(Props::ForeignKey(Arc::new(props)))
                .add_flags(ObjFlags::BROKEN),
        );
    };
    let target = cols.target(Some(&f.schema), &f.table);
    let (target_columns, tb) = target_names(cols, &f.table, &target, &f.columns);
    broken |= tb || target.dangling;
    broken |= columns.len() != target_columns.len();
    let on_update = action(f.on_update);
    let on_delete = action(f.on_delete);
    let match_kind = match f.match_type {
        b's' => Some(FkMatch::Simple),
        b'f' => Some(FkMatch::Full),
        b'p' => Some(FkMatch::Partial),
        _ => None,
    };
    broken |= on_update.is_none() || on_delete.is_none() || match_kind.is_none();
    let fk_ref_index = f.index.as_ref().map(|(oid, name)| {
        let mut path = target.path.clone();
        path.push(name.clone());
        Ref {
            kind: ObjectKind::Index,
            path,
            server_id: Some(ServerId::Oid(*oid)),
        }
    });
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
        deferrable: row.deferrable,
        initially_deferred: row.deferred,
        not_valid: !row.validated,
        enforced: row.enforced,
        rare: fk_ref_index.map(|r| {
            Arc::new(FkRare {
                fk_ref_index: Some(r),
            })
        }),
    };
    KeyObj::new(
        row.table.clone(),
        Obj::new(ObjectKind::ForeignKey, row.name.clone())
            .with_props(Props::ForeignKey(Arc::new(props)))
            .add_flags(flag_if(broken, ObjFlags::BROKEN)),
    )
}

fn action(code: u8) -> Option<FkAction> {
    match code {
        b'a' => Some(FkAction::NoAction),
        b'r' => Some(FkAction::Restrict),
        b'c' => Some(FkAction::Cascade),
        b'n' => Some(FkAction::SetNull),
        b'd' => Some(FkAction::SetDefault),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::exclusion_body;

    #[test]
    fn exclusion_body_drops_the_keyword_and_the_deferral_words() {
        assert_eq!(
            exclusion_body(
                "EXCLUDE USING gist (r WITH &&) WHERE ((a > 0)) DEFERRABLE INITIALLY DEFERRED"
            ),
            Some("USING gist (r WITH &&) WHERE ((a > 0))")
        );
        assert_eq!(
            exclusion_body("EXCLUDE USING gist (r WITH =)"),
            Some("USING gist (r WITH =)")
        );
        assert_eq!(exclusion_body("CHECK ((a > 0))"), None);
        assert_eq!(exclusion_body("EXCLUDE"), None);
    }
}
