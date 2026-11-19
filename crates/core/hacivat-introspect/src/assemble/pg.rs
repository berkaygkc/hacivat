// ## m4-t13-assemble
//! PostgreSQL assembly: one row per index (`pg_index` + its `pg_class` row) and one per
//! constraint (`pg_constraint`, `conrelid <> 0`), with ordered arrays (`indkey`/`indoption`,
//! `conkey`, `confkey`, `conexclop`) unnested in key order by the pack's query (Task 16).
//!
//! - Index key slots with `attnum` 0 are expressions; their text is `pg_get_indexdef(oid, k,
//!   true)`, selected only for those slots.
//! - A PK, UNIQUE or EXCLUDE constraint's `conindid` names its backing index: the index is
//!   [`ObjFlags::IMPLICIT`] with `implicit_for` the constraint, and the key carries the index's
//!   name (`backing_index`), `INCLUDE` columns and `indnullsnotdistinct` (PostgreSQL 15+).
//! - An FK's `conindid` names the **referenced** table's index: it becomes
//!   [`KeyObj::ref_constraint`], never an implicit link.
//! - `contype` `t` (constraint triggers) and `n` (catalogued NOT NULL, Task 16's column props)
//!   are not assembled here.

use std::collections::HashMap;
use std::sync::Arc;

use hacivat_meta::props::{CheckProps, FkAction, FkMatch, FkProps, IndexProps};
use hacivat_meta::{Name, Obj, ObjFlags, ObjectKind, Ref, ServerId};

use super::{
    ColRef, ColumnIndex, KeyObj, Parts, TableId, finish, flag_if, index_obj, key_obj, key_props,
    mark_implicit, resolve_names,
};

/// One key slot of an index (`indkey[k]` with `indoption[k]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexPart {
    /// The column's `attnum`; 0 for an expression slot.
    pub attnum: i16,
    /// The column's `attname` (none for an expression slot).
    pub name: Option<Name>,
    /// `pg_get_indexdef(oid, k, true)` for an expression slot.
    pub expression: Option<Name>,
    /// `indoption` bit 0.
    pub desc: bool,
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
    /// `pg_get_constraintdef(oid, true)`.
    pub definition: Name,
    pub deferrable: bool,
    /// `condeferred`.
    pub deferred: bool,
    /// `convalidated` (false: `NOT VALID`).
    pub validated: bool,
    /// `conenforced` (PostgreSQL 18+; true before).
    pub enforced: bool,
    /// `conindid` of a PK, UNIQUE or EXCLUDE constraint: its backing index.
    pub index: Option<u32>,
    /// A foreign key's referenced side.
    pub foreign: Option<FkTarget>,
    /// An exclusion constraint's operators (`conexclop` names), in key order.
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
    let n_indexes = out.len();
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
        debug_assert!(idx < n_indexes);
        let (head, tail) = out.split_at_mut(con);
        mark_implicit(cols, &mut head[idx], &tail[0]);
    }
    finish(out)
}

fn index(row: &IndexRow, cols: &ColumnIndex) -> KeyObj {
    let parts = parts(row, cols);
    let (include, inc_broken) = resolve_names(cols, &row.table, &row.include);
    let flags = flag_if(parts.broken || inc_broken, ObjFlags::BROKEN)
        | flag_if(!row.valid, ObjFlags::INVALID);
    let props = IndexProps {
        key: hacivat_meta::props::KeyProps {
            columns: parts.columns,
            expressions: parts.expressions,
            include,
            predicate: row.predicate.clone(),
            desc: parts.desc,
            unique: row.unique,
            nulls_not_distinct: row.nulls_not_distinct,
            ..key_props()
        },
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
            Ok(0) => p.expression(part.expression.as_ref(), part.desc),
            Ok(attnum) => p.column(
                cols,
                &row.table,
                &ColRef {
                    ordinal: Some(attnum),
                    name: part.name.clone(),
                },
                part.desc,
            ),
            // A system column (negative attnum) is not a column of the table's list.
            Err(_) => p.column(
                cols,
                &row.table,
                &ColRef {
                    ordinal: None,
                    name: Some(part.name.clone().unwrap_or_else(|| Name::from(""))),
                },
                part.desc,
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
    let invalid = flag_if(!row.validated, ObjFlags::INVALID);
    let sid = ServerId::Oid(row.oid);
    let ko = match row.contype {
        b'p' | b'u' => {
            let kind = if row.contype == b'p' {
                ObjectKind::PrimaryKey
            } else {
                ObjectKind::UniqueKey
            };
            let mut p = Parts::default();
            for c in &row.columns {
                p.column(cols, &row.table, c, false);
            }
            let mut broken = p.broken;
            let mut props = hacivat_meta::props::KeyProps {
                unique: true,
                deferrable: row.deferrable,
                initially_deferred: row.deferred,
                not_valid: !row.validated,
                enforced: row.enforced,
                ..key_props()
            };
            if let Some(ix) = backing {
                let (include, inc_broken) = resolve_names(cols, &row.table, &ix.include);
                broken |= inc_broken;
                props.include = include;
                props.nulls_not_distinct = ix.nulls_not_distinct;
                props.backing_index = Some(ix.name.clone());
                if ix.parts.len() == p.columns.len() {
                    p.desc = ix.parts.iter().map(|x| x.desc).collect();
                }
            }
            props.columns = p.columns;
            props.desc = p.desc;
            KeyObj::new(
                row.table.clone(),
                key_obj(kind, &row.name, props)
                    .add_flags(flag_if(broken, ObjFlags::BROKEN) | invalid),
            )
        }
        b'f' => foreign_key(row, cols),
        b'c' | b'x' => {
            let kind = if row.contype == b'c' {
                ObjectKind::Check
            } else {
                ObjectKind::Exclusion
            };
            // Expression elements of an exclusion constraint are 0 in `conkey`.
            let mut broken = row
                .columns
                .iter()
                .filter(|c| c.ordinal != Some(0))
                .any(|c| cols.resolve(&row.table, c).is_none());
            if kind == ObjectKind::Exclusion {
                broken |= row.operators.len() != row.columns.len();
            }
            let props = CheckProps {
                expr: row.definition.clone(),
                not_valid: !row.validated,
                enforced: row.enforced,
            };
            let mut ko = KeyObj::new(
                row.table.clone(),
                Obj::new(kind, row.name.clone())
                    .with_props(hacivat_meta::props::Props::Check(Arc::new(props)))
                    .add_flags(flag_if(broken, ObjFlags::BROKEN) | invalid),
            );
            if kind == ObjectKind::Exclusion {
                ko.operators = row.operators.clone();
            }
            ko
        }
        _ => return None,
    };
    let KeyObj {
        table,
        obj,
        ref_constraint,
        operators,
    } = ko;
    Some(KeyObj {
        table,
        obj: obj.with_server_id(sid),
        ref_constraint,
        operators,
    })
}

fn foreign_key(row: &ConstraintRow, cols: &ColumnIndex) -> KeyObj {
    let (columns, mut broken) = resolve_names(cols, &row.table, &row.columns);
    let invalid = flag_if(!row.validated, ObjFlags::INVALID);
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
        };
        return KeyObj::new(
            row.table.clone(),
            Obj::new(ObjectKind::ForeignKey, row.name.clone())
                .with_props(hacivat_meta::props::Props::ForeignKey(Arc::new(props)))
                .add_flags(ObjFlags::BROKEN | invalid),
        );
    };
    let schema = Some(&f.schema);
    let target_columns = if cols.knows(schema.map(|s| &**s), &f.table) {
        let (names, b) = resolve_names(cols, &f.table, &f.columns);
        broken |= b;
        names
    } else {
        // Out of this namespace: the row's names, the reference resolved on demand.
        f.columns
            .iter()
            .map(|c| {
                c.name.clone().unwrap_or_else(|| {
                    broken = true;
                    Name::from("")
                })
            })
            .collect()
    };
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
    let target_path = cols.table_path(schema, &f.table.name);
    let props = FkProps {
        columns,
        target: Ref {
            kind: ObjectKind::Table,
            path: target_path.clone(),
            server_id: f.table.server_id,
        },
        target_columns,
        on_update: on_update.unwrap_or_default(),
        on_delete: on_delete.unwrap_or_default(),
        match_kind: match_kind.unwrap_or_default(),
        deferrable: row.deferrable,
        initially_deferred: row.deferred,
        not_valid: !row.validated,
        enforced: row.enforced,
    };
    let mut ko = KeyObj::new(
        row.table.clone(),
        Obj::new(ObjectKind::ForeignKey, row.name.clone())
            .with_props(hacivat_meta::props::Props::ForeignKey(Arc::new(props)))
            .add_flags(flag_if(broken, ObjFlags::BROKEN) | invalid),
    );
    ko.ref_constraint = f.index.as_ref().map(|(oid, name)| {
        let mut path = target_path;
        path.push(name.clone());
        Ref {
            kind: ObjectKind::Index,
            path,
            server_id: Some(ServerId::Oid(*oid)),
        }
    });
    ko
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
