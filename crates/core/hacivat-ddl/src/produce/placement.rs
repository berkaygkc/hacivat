//! HV-DDL-09 constraint placement: in a column, as a table element, or after the table.
//!
//! The requested mode applies only where the dialect and the plan allow it:
//! - a foreign key to a table created later in the script (or in a cycle) goes after the
//!   tables; a PostgreSQL `NOT VALID` constraint too (only `ALTER TABLE … ADD` keeps it
//!   unvalidated);
//! - `InsideColumn` applies to single-column primary keys, unique keys and foreign keys
//!   (a check stays a table element: the model does not say which columns it reads);
//! - MySQL: a foreign key is never inline (InnoDB ignores an inline `REFERENCES`); an
//!   inline unique key keeps its name only when the name is the column's, so other names stay
//!   table elements; the key of an `AUTO_INCREMENT` column must exist at `CREATE TABLE`;
//! - SQLite: there is no `ADD CONSTRAINT`, so everything stays inside the table (notice
//!   `ddl-sqlite-placement-forced` when the plan or the mode wanted it after), and the primary
//!   key of an `AUTOINCREMENT` column must sit in that column.

use hacivat_meta::Name;
use sql_dialect::DialectId;

use crate::error::NoticeCode;
use crate::ir::{ConstraintBody, Position};
use crate::options::Placement;

/// What placement needs to know about one constraint.
#[derive(Debug, Clone, Copy)]
pub struct PlaceInput<'a> {
    pub body: &'a ConstraintBody,
    pub name: Option<&'a Name>,
    /// PostgreSQL `NOT VALID`.
    pub not_valid: bool,
    /// A foreign key whose target is created later in the script, or is in a cycle.
    pub target_later: bool,
    /// The table's `AUTO_INCREMENT` / `AUTOINCREMENT` column, if any.
    pub auto_column: Option<&'a Name>,
}

/// Where a constraint goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placed {
    Inside(Position),
    /// `ALTER TABLE … ADD` after every table of the script.
    After,
}

fn first_column(b: &ConstraintBody) -> Option<&Name> {
    match b {
        ConstraintBody::PrimaryKey { columns } | ConstraintBody::Unique { columns } => {
            columns.first()
        }
        _ => None,
    }
}

/// Places one constraint (HV-DDL-09 algorithm with the dialect overrides above).
pub fn place(
    c: &PlaceInput<'_>,
    mode: Placement,
    dialect: DialectId,
) -> (Placed, Option<NoticeCode>) {
    let single = c.body.single_column();
    let is_fk = matches!(c.body, ConstraintBody::ForeignKey { .. });
    let column_eligible = matches!(
        c.body,
        ConstraintBody::PrimaryKey { .. }
            | ConstraintBody::Unique { .. }
            | ConstraintBody::ForeignKey { .. }
    );
    match dialect {
        DialectId::Sqlite => {
            if let (ConstraintBody::PrimaryKey { .. }, Some(col), Some(auto)) =
                (c.body, single, c.auto_column)
                && col == auto
            {
                return (Placed::Inside(Position::InColumn(col.clone())), None);
            }
            if c.target_later || mode == Placement::AfterTable {
                return (
                    Placed::Inside(Position::InTable),
                    Some(NoticeCode::SqlitePlacementForced),
                );
            }
            if mode == Placement::InsideColumn
                && column_eligible
                && let Some(col) = single
            {
                return (Placed::Inside(Position::InColumn(col.clone())), None);
            }
            (Placed::Inside(Position::InTable), None)
        }
        DialectId::MySql | DialectId::MariaDb => {
            let auto_key = c.auto_column.is_some() && first_column(c.body) == c.auto_column;
            if is_fk && c.target_later {
                return (Placed::After, None);
            }
            match mode {
                Placement::AfterTable if !auto_key => (Placed::After, None),
                Placement::InsideColumn => {
                    let inline = match (c.body, single) {
                        (ConstraintBody::PrimaryKey { .. }, Some(col)) => Some(col),
                        (ConstraintBody::Unique { .. }, Some(col))
                            if c.name.is_none_or(|n| n == col) =>
                        {
                            Some(col)
                        }
                        _ => None,
                    };
                    match inline {
                        Some(col) => (Placed::Inside(Position::InColumn(col.clone())), None),
                        None => (Placed::Inside(Position::InTable), None),
                    }
                }
                _ => (Placed::Inside(Position::InTable), None),
            }
        }
        _ => {
            if (is_fk && c.target_later) || c.not_valid {
                return (Placed::After, None);
            }
            match mode {
                Placement::AfterTable => (Placed::After, None),
                Placement::InsideColumn if column_eligible => match single {
                    Some(col) => (Placed::Inside(Position::InColumn(col.clone())), None),
                    None => (Placed::Inside(Position::InTable), None),
                },
                _ => (Placed::Inside(Position::InTable), None),
            }
        }
    }
}
