//! Constraint placement (HV-DDL-09): `InsideColumn | InsideTable (default) | AfterTable`.
//! M4 Task 7 body (spike origin `da358e4`).
//!
//! The requested mode applies only where the dialect and the plan allow it:
//! - a foreign key to a table created later in the script, or in a cycle with this one, goes
//!   after the tables; a PostgreSQL `NOT VALID` constraint too (only `ALTER TABLE … ADD` keeps
//!   it unvalidated);
//! - `InsideColumn` applies to single-column primary, unique and foreign keys (a check stays a
//!   table element: the model does not say which columns it reads);
//! - MySQL / MariaDB: a foreign key is never inline (InnoDB parses an inline `REFERENCES` and
//!   ignores it); an inline unique key is named after its column, so a key with another name
//!   stays a table element; the key of an `AUTO_INCREMENT` column must exist at `CREATE TABLE`;
//! - SQLite: there is no `ADD CONSTRAINT`, so everything stays inside the table (notice
//!   `ddl-sqlite-placement-forced` when the plan or the mode wanted it after), and the primary
//!   key of an `AUTOINCREMENT` (or rowid-alias) column sits in that column.

use hacivat_meta::name::Name;
use sql_dialect::DialectId;
use sql_ident::QualifiedName;

use crate::ir::{ConstraintBody, ConstraintDef};
use crate::options::Placement;
use crate::script::Notice;

/// The notice of a SQLite constraint kept inside its table against the mode or the plan.
pub const SQLITE_PLACEMENT_FORCED: &str = "ddl-sqlite-placement-forced";

/// Where a constraint is written.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Position {
    /// Inside the definition of this column.
    InColumn(Name),
    /// A table element after the columns.
    InTable,
    /// `ALTER TABLE … ADD` after the table (after every table of the script when the plan
    /// says the reference is created later).
    After,
}

/// One constraint's placement, and the notice it raises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub position: Position,
    pub notice: Option<Notice>,
}

/// What placement needs to know beyond the constraints.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlacePlan<'a> {
    /// Tables created later in the script than this one, or in a cycle with it: a foreign key
    /// to one of them goes after the tables.
    pub later: &'a [QualifiedName],
    /// The table's `AUTO_INCREMENT` / `AUTOINCREMENT` / rowid-alias column, if any.
    pub auto_column: Option<&'a Name>,
}

impl PlacePlan<'_> {
    fn is_later(&self, target: &QualifiedName) -> bool {
        self.later.iter().any(|l| {
            l.parts.len() == target.parts.len()
                && l.parts
                    .iter()
                    .zip(&target.parts)
                    .all(|(a, b)| a.text == b.text)
        })
    }
}

/// Places every constraint of a table, in order (HV-DDL-09 with the dialect overrides above).
pub fn place(
    constraints: &[ConstraintDef],
    mode: Placement,
    dialect: DialectId,
    plan: &PlacePlan<'_>,
) -> Vec<Placed> {
    constraints
        .iter()
        .map(|k| place_one(k, mode, dialect, plan))
        .collect()
}

fn placed(position: Position) -> Placed {
    Placed {
        position,
        notice: None,
    }
}

fn place_one(
    k: &ConstraintDef,
    mode: Placement,
    dialect: DialectId,
    plan: &PlacePlan<'_>,
) -> Placed {
    let single = k.body.single_column();
    let target_later =
        matches!(&k.body, ConstraintBody::ForeignKey { target, .. } if plan.is_later(target));
    let first_key_column = match &k.body {
        ConstraintBody::PrimaryKey { columns } | ConstraintBody::Unique { columns, .. } => {
            columns.first()
        }
        _ => None,
    };
    match dialect {
        DialectId::Sqlite => {
            if matches!(k.body, ConstraintBody::PrimaryKey { .. })
                && let (Some(col), Some(auto)) = (single, plan.auto_column)
                && col == auto
            {
                return placed(Position::InColumn(col.clone()));
            }
            if target_later || mode == Placement::AfterTable {
                return Placed {
                    position: Position::InTable,
                    notice: Some(Notice::new(SQLITE_PLACEMENT_FORCED)),
                };
            }
            match (mode, single) {
                (Placement::InsideColumn, Some(col)) => placed(Position::InColumn(col.clone())),
                _ => placed(Position::InTable),
            }
        }
        DialectId::MySql | DialectId::MariaDb => {
            if target_later {
                return placed(Position::After);
            }
            let auto_key = plan.auto_column.is_some() && first_key_column == plan.auto_column;
            match mode {
                Placement::AfterTable if !auto_key => placed(Position::After),
                Placement::InsideColumn => {
                    let inline = match (&k.body, single) {
                        (ConstraintBody::PrimaryKey { .. }, Some(col)) => Some(col),
                        (ConstraintBody::Unique { .. }, Some(col))
                            if k.name.as_ref().is_none_or(|n| n == col) =>
                        {
                            Some(col)
                        }
                        _ => None,
                    };
                    placed(inline.map_or(Position::InTable, |c| Position::InColumn(c.clone())))
                }
                _ => placed(Position::InTable),
            }
        }
        _ => {
            if target_later || k.not_valid {
                return placed(Position::After);
            }
            match (mode, single) {
                (Placement::AfterTable, _) => placed(Position::After),
                (Placement::InsideColumn, Some(col)) => placed(Position::InColumn(col.clone())),
                _ => placed(Position::InTable),
            }
        }
    }
}
