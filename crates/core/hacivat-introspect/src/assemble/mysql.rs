// ## m4-t13-assemble
//! MySQL and MariaDB assembly from `INFORMATION_SCHEMA` rows (Task 17's `Structure` pass).
//!
//! - **Indexes** come from `STATISTICS`, one row per key part, grouped here by `(TABLE_NAME,
//!   INDEX_NAME)` in `SEQ_IN_INDEX` order (no `GROUP_CONCAT`: no `group_concat_max_len`, no
//!   warning 1260; MySQL/SQLite spike difference 5). A gap or repeat in the sequence is a
//!   missing part: the index is kept, broken. Functional key parts (8.0.13+) have
//!   `COLUMN_NAME` NULL and `EXPRESSION` set. `COLLATION` `D` is descending, kept as the
//!   server reports it (MariaDB 10.6 stores `DESC` parts ascending, F9).
//! - **Keys.** `PRIMARY` and every unique index are keys (`TABLE_CONSTRAINTS` lists each unique
//!   index as a `UNIQUE` constraint of the same name); the same-named index is
//!   [`ObjFlags::IMPLICIT`] for the key. `PRIMARY` is the server's own name
//!   ([`ObjFlags::SERVER_GENERATED_NAME`]). `KeyProps` has no prefix lengths: a key's DDL
//!   renders its parts through its backing index (`IndexProps::prefix_len`; Task 19).
//! - **Foreign keys** from `REFERENTIAL_CONSTRAINTS` with their `KEY_COLUMN_USAGE` rows grouped
//!   by `(TABLE_NAME, CONSTRAINT_NAME)` in `ORDINAL_POSITION` order; the referenced columns by
//!   position. A target in this schema is named by its stored table (`lower_case_table_names`
//!   ≥ 1 folds the spelling). The index InnoDB creates for an FK is implicit only under
//!   [`FkIndexRule::NamedAfterConstraint`], which Task 17's probe decides.
//! - **Checks** (8.0.16+, MariaDB 10.2+): the `CHECK` rows of `TABLE_CONSTRAINTS` joined with
//!   `CHECK_CONSTRAINTS` — by name on MySQL (check names are unique per schema), by
//!   `(TABLE_NAME, CONSTRAINT_NAME)` on MariaDB (per table, F9). A check without its clause is
//!   kept, broken, with an empty expression.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use hacivat_meta::props::{CheckProps, FkProps, FkRare, IndexProps, KeyProps, Props};
use hacivat_meta::{Name, Obj, ObjFlags, ObjectKind, Ref};

use super::{
    ColRef, ColumnIndex, KeyObj, PartOpts, Parts, TableId, finish, fk_action_word, fk_match_word,
    flag_if, index_obj, key_obj, key_props, mark_implicit,
};

/// The server family: how check clauses join.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    MySql,
    MariaDb,
}

/// Which index an FK created (InnoDB creates one when no index supports the FK).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FkIndexRule {
    /// No index is marked implicit for an FK: DDL shows every index (MySQL then accepts the
    /// explicit index the FK would have created).
    #[default]
    None,
    /// A non-unique index named like an FK of its table whose key columns are exactly the FK's
    /// columns is the FK's (pinned by Task 17's probe on every server of the matrix, CI
    /// 44649745390; Task 17b adopts it).
    NamedAfterConstraint,
}

/// One `STATISTICS` row: one key part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatRow {
    pub table: Name,
    pub index: Name,
    /// `SEQ_IN_INDEX` (1-based).
    pub seq_in_index: u32,
    /// `COLUMN_NAME` (NULL for a functional key part).
    pub column: Option<Name>,
    /// `EXPRESSION` (8.0.13+).
    pub expression: Option<Name>,
    /// `COLLATION`: `A`, `D` or NULL (not sorted).
    pub collation: Option<Name>,
    /// `SUB_PART`: the prefix length.
    pub sub_part: Option<u32>,
    /// `NON_UNIQUE`.
    pub non_unique: bool,
    /// `INDEX_TYPE` (`BTREE`, `HASH`, `FULLTEXT`, `SPATIAL`).
    pub index_type: Option<Name>,
    /// `IS_VISIBLE = YES` (MySQL), `IGNORED = NO` (MariaDB 10.6+); true before.
    pub visible: bool,
    /// `INDEX_COMMENT` (empty → `None`).
    pub comment: Option<Name>,
}

/// One `REFERENTIAL_CONSTRAINTS` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FkRow {
    pub table: Name,
    /// `CONSTRAINT_NAME`.
    pub name: Name,
    /// `UNIQUE_CONSTRAINT_SCHEMA`: the referenced table's schema.
    pub referenced_schema: Name,
    /// `REFERENCED_TABLE_NAME`.
    pub referenced_table: Name,
    /// `UNIQUE_CONSTRAINT_NAME`: the referenced table's index the FK relies on.
    pub unique_constraint_name: Option<Name>,
    /// `UPDATE_RULE`, `DELETE_RULE`: `CASCADE`, `SET NULL`, `SET DEFAULT`, `RESTRICT`,
    /// `NO ACTION` (an unspecified `ON UPDATE` reads `NO ACTION` on MySQL and `RESTRICT` on
    /// MariaDB, kept as reported, F9).
    pub update_rule: Name,
    pub delete_rule: Name,
    /// `MATCH_OPTION` (`NONE`).
    pub match_option: Name,
}

/// One `KEY_COLUMN_USAGE` row of a foreign key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FkColumnRow {
    pub table: Name,
    /// `CONSTRAINT_NAME`.
    pub constraint: Name,
    /// `COLUMN_NAME`.
    pub column: Name,
    /// `ORDINAL_POSITION` (1-based position in the FK).
    pub ordinal_position: u32,
    /// `REFERENCED_COLUMN_NAME`.
    pub referenced_column: Option<Name>,
}

/// One `TABLE_CONSTRAINTS` row of type `CHECK`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckRow {
    pub table: Name,
    pub name: Name,
    /// `ENFORCED = YES` (MySQL 8.0.16+); true on MariaDB.
    pub enforced: bool,
}

/// One `CHECK_CONSTRAINTS` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckClauseRow {
    /// `TABLE_NAME` (MariaDB only; MySQL's view has no such column).
    pub table: Option<Name>,
    pub name: Name,
    /// `CHECK_CLAUSE`, raw.
    pub clause: Name,
}

/// The MySQL/MariaDB rows of one schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRows {
    pub flavor: Flavor,
    pub fk_index: FkIndexRule,
    pub indexes: Vec<StatRow>,
    pub foreign_keys: Vec<FkRow>,
    pub fk_columns: Vec<FkColumnRow>,
    pub checks: Vec<CheckRow>,
    pub check_clauses: Vec<CheckClauseRow>,
}

impl KeyRows {
    /// No rows, for `flavor`, with no FK-created index rule.
    pub fn new(flavor: Flavor) -> Self {
        KeyRows {
            flavor,
            fk_index: FkIndexRule::None,
            indexes: Vec::new(),
            foreign_keys: Vec::new(),
            fk_columns: Vec::new(),
            checks: Vec::new(),
            check_clauses: Vec::new(),
        }
    }
}

/// The MySQL primary key's (and its index's) fixed name.
const PRIMARY: &str = "PRIMARY";

/// Assembles keys, indexes, foreign keys and checks (module docs).
pub fn assemble_keys(rows: KeyRows, cols: &ColumnIndex) -> Vec<KeyObj> {
    let mut out = Vec::new();

    // Indexes, and the keys of `PRIMARY` and unique indexes.
    let mut by_index: BTreeMap<(Name, Name), Vec<&StatRow>> = BTreeMap::new();
    for r in &rows.indexes {
        by_index
            .entry((r.table.clone(), r.index.clone()))
            .or_default()
            .push(r);
    }
    // (key position, its index position).
    let mut links = Vec::new();
    // Plain indexes by table, for the FK-created index rule.
    let mut plain: HashMap<(Name, Name), usize> = HashMap::new();
    for ((table, name), mut parts) in by_index {
        parts.sort_by_key(|r| r.seq_in_index);
        let table = TableId::named(table);
        let first = parts[0];
        let mut p = Parts::default();
        let mut prefix_len = Vec::with_capacity(parts.len());
        let mut broken = false;
        for (i, r) in parts.iter().enumerate() {
            broken |= usize::try_from(r.seq_in_index).ok() != Some(i + 1);
            let desc = r.collation.as_deref() == Some("D");
            match &r.column {
                Some(c) => p.column(
                    cols,
                    &table,
                    &ColRef::named(c.clone()),
                    PartOpts::desc(desc),
                ),
                None => p.expression(r.expression.as_ref(), PartOpts::desc(desc)),
            }
            prefix_len.push(r.sub_part);
        }
        let unique = !first.non_unique;
        let primary = unique && *name == *PRIMARY;
        let (key, parts_broken) = p.into_key(KeyProps {
            unique,
            invisible: !first.visible,
            ..key_props()
        });
        broken |= parts_broken;
        let flags =
            flag_if(broken, ObjFlags::BROKEN) | flag_if(primary, ObjFlags::SERVER_GENERATED_NAME);
        let index_at = out.len();
        out.push(KeyObj::new(
            table.clone(),
            index_obj(
                &name,
                IndexProps {
                    key: key.clone(),
                    method: first.index_type.clone(),
                    prefix_len,
                    comment: first.comment.clone(),
                },
            )
            .add_flags(flags),
        ));
        if unique {
            let kind = if primary {
                ObjectKind::PrimaryKey
            } else {
                ObjectKind::UniqueKey
            };
            links.push((out.len(), index_at));
            out.push(KeyObj::new(
                table,
                key_obj(
                    kind,
                    &name,
                    KeyProps {
                        backing_index: Some(name.clone()),
                        ..key
                    },
                )
                .add_flags(flags),
            ));
        } else {
            plain.insert((table.name, name), index_at);
        }
    }

    // Foreign keys.
    let mut fk_cols: HashMap<(Name, Name), Vec<&FkColumnRow>> = HashMap::new();
    for r in &rows.fk_columns {
        fk_cols
            .entry((r.table.clone(), r.constraint.clone()))
            .or_default()
            .push(r);
    }
    for fk in &rows.foreign_keys {
        let mut parts = fk_cols
            .remove(&(fk.table.clone(), fk.name.clone()))
            .unwrap_or_default();
        parts.sort_by_key(|r| r.ordinal_position);
        let fk_at = out.len();
        out.push(foreign_key(fk, &parts, cols));
        if rows.fk_index == FkIndexRule::NamedAfterConstraint
            && let Some(&ix) = plain.get(&(fk.table.clone(), fk.name.clone()))
            && fk_created(&out[ix], &out[fk_at])
        {
            links.push((fk_at, ix));
        }
    }

    // Checks.
    let mut by_name: HashMap<&str, Vec<&CheckClauseRow>> = HashMap::new();
    for c in &rows.check_clauses {
        by_name.entry(&c.name).or_default().push(c);
    }
    for chk in &rows.checks {
        let candidates = by_name.get(&*chk.name).map(Vec::as_slice).unwrap_or(&[]);
        let clause = match rows.flavor {
            Flavor::MySql => match candidates {
                [one] => Some(&one.clause),
                _ => None,
            },
            Flavor::MariaDb => {
                let mut it = candidates
                    .iter()
                    .filter(|c| c.table.as_deref() == Some(&*chk.table));
                match (it.next(), it.next()) {
                    (Some(one), None) => Some(&one.clause),
                    _ => None,
                }
            }
        };
        let props = CheckProps {
            expr: clause.cloned().unwrap_or_else(|| Name::from("")),
            not_valid: false,
            enforced: chk.enforced,
            rare: None,
        };
        out.push(KeyObj::new(
            TableId::named(chk.table.clone()),
            Obj::new(ObjectKind::Check, chk.name.clone())
                .with_props(Props::Check(Arc::new(props)))
                .add_flags(flag_if(clause.is_none(), ObjFlags::BROKEN)),
        ));
    }

    for (owner, idx) in links {
        let owner = out[owner].clone();
        mark_implicit(cols, &mut out[idx], &owner);
    }
    finish(out)
}

fn foreign_key(fk: &FkRow, parts: &[&FkColumnRow], cols: &ColumnIndex) -> KeyObj {
    let table = TableId::named(fk.table.clone());
    let target = TableId::named(fk.referenced_table.clone());
    let to = cols.target(Some(&fk.referenced_schema), &target);
    let in_scope = to.known;
    let mut broken = parts.is_empty() || to.dangling;
    let mut columns = Vec::with_capacity(parts.len());
    let mut target_columns = Vec::with_capacity(parts.len());
    for (i, r) in parts.iter().enumerate() {
        broken |= usize::try_from(r.ordinal_position).ok() != Some(i + 1);
        match cols.resolve(&table, &ColRef::named(r.column.clone())) {
            Some(kc) => columns.extend(kc.name),
            None => {
                broken = true;
                columns.push(r.column.clone());
            }
        }
        match &r.referenced_column {
            Some(rc) if in_scope => match cols.resolve(&target, &ColRef::named(rc.clone())) {
                Some(kc) => target_columns.extend(kc.name),
                None => {
                    broken = true;
                    target_columns.push(rc.clone());
                }
            },
            Some(rc) => target_columns.push(rc.clone()),
            None => {
                broken = true;
                target_columns.push(Name::from(""));
            }
        }
    }
    let on_update = fk_action_word(&fk.update_rule);
    let on_delete = fk_action_word(&fk.delete_rule);
    let match_kind = fk_match_word(&fk.match_option);
    broken |= on_update.is_none() || on_delete.is_none() || match_kind.is_none();
    let fk_ref_index = fk.unique_constraint_name.as_ref().map(|name| {
        let mut path = to.path.clone();
        path.push(name.clone());
        Ref {
            kind: ObjectKind::Index,
            path,
            server_id: None,
        }
    });
    let props = FkProps {
        columns,
        target: Ref {
            kind: ObjectKind::Table,
            path: to.path,
            server_id: to.server_id,
        },
        target_columns,
        on_update: on_update.unwrap_or_default(),
        on_delete: on_delete.unwrap_or_default(),
        match_kind: match_kind.unwrap_or_default(),
        deferrable: false,
        initially_deferred: false,
        not_valid: false,
        enforced: true,
        rare: fk_ref_index.map(|r| {
            Arc::new(FkRare {
                fk_ref_index: Some(r),
            })
        }),
    };
    KeyObj::new(
        table,
        Obj::new(ObjectKind::ForeignKey, fk.name.clone())
            .with_props(Props::ForeignKey(Arc::new(props)))
            .add_flags(flag_if(broken, ObjFlags::BROKEN)),
    )
}

/// Whether the plain index `idx` is the one InnoDB created for `fk`: its leading key columns
/// are the FK's columns, in order.
fn fk_created(idx: &KeyObj, fk: &KeyObj) -> bool {
    let (Some(k), Some(f)) = (idx.key(), fk.fk()) else {
        return false;
    };
    // ## m4-t17b-mysql-facets (named exception): exactly the FK's columns. An index the user
    // made longer (named like the constraint, made before it) also supports the FK, so InnoDB
    // creates none; hiding it would drop its extra parts from the DDL.
    !f.columns.is_empty()
        && k.columns.len() == f.columns.len()
        && k.columns
            .iter()
            .zip(&f.columns)
            .all(|(c, n)| c.name.as_ref() == Some(n))
}
