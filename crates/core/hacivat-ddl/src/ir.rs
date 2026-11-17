//! The typed DDL IR (alg/03 HV-DDL-07 "IR"; plan-m4 amendment A-8: every alg/03 variant is
//! declared; M4 renders the view-only subset and M7 fills the rest).
//!
//! Producers build [`DdlOp`]s from the model; renderers (one per dialect) turn them into
//! statements. Every expression, type and default is **raw server text** ([`Name`]),
//! emitted verbatim: the IR never re-quotes or normalizes what the server printed.
//!
//! A definition renders its own comments (MySQL inline, PostgreSQL as `COMMENT ON`
//! statements after it, SQLite never), so a `TableDef` alone is a complete table on every
//! dialect.

use hacivat_meta::props::{Deferrable, FkAction, FkMatch};
use hacivat_meta::{Name, ObjectKind};

/// A schema-qualified object name. `schema` is the namespace's name (PostgreSQL schema, MySQL
/// database, SQLite `main`/`temp`/attached alias); whether it is printed is the
/// qualification plan's decision (HV-DDL-10).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct QName {
    pub schema: Option<Name>,
    pub name: Name,
}

impl QName {
    pub fn new(schema: Option<&str>, name: &str) -> Self {
        QName {
            schema: schema.map(Name::from),
            name: Name::from(name),
        }
    }

    pub fn bare(name: &str) -> Self {
        QName::new(None, name)
    }
}

/// One DDL operation (alg/03 HV-DDL-07).
#[derive(Debug, Clone, PartialEq)]
pub enum DdlOp {
    CreateSchema(SchemaDef),
    CreateTable(TableDef),
    /// M4 renders only [`AlterAction::AddConstraint`] (postponed constraints, HV-DDL-09).
    AlterTable {
        table: QName,
        actions: Vec<AlterAction>,
    },
    DropObject {
        kind: ObjectKind,
        name: QName,
        sig: Option<Name>,
        if_exists: bool,
        cascade: bool,
    },
    CreateIndex(IndexDef),
    CreateView(ViewDef),
    CreateRoutine(RoutineDef),
    CreateTrigger(TriggerDef),
    CreateSequence(SeqDef),
    /// Hacivat addition to the alg/03 list: `ALTER SEQUENCE … OWNED BY` (a serial's sequence
    /// is created before its table and owned after it). M4 renders only `OwnedBy`.
    AlterSequence {
        name: QName,
        actions: Vec<SeqAction>,
    },
    CreateType(TypeDef),
    Rename {
        kind: ObjectKind,
        from: QName,
        to: Name,
    },
    SetSchema {
        kind: ObjectKind,
        name: QName,
        to: Name,
    },
    /// A standalone comment change; `text: None` removes the comment.
    Comment {
        target: CommentTarget,
        text: Option<Name>,
    },
    Grant(GrantDef),
    Revoke(GrantDef),
    Owner {
        kind: ObjectKind,
        name: QName,
        /// The argument type list of a routine (`integer, text`).
        sig: Option<Name>,
        owner: Name,
    },
    TableRebuild(RebuildPlan),
    /// ClickHouse `EXCHANGE TABLES`; never rendered in v1.
    Exchange {
        a: QName,
        b: QName,
    },
    Raw {
        sql: Name,
        origin: RawOrigin,
    },
    Data(DataStep),
}

impl DdlOp {
    /// The variant's name (for `RenderError::Unsupported`).
    pub fn variant(&self) -> &'static str {
        match self {
            DdlOp::CreateSchema(_) => "CreateSchema",
            DdlOp::CreateTable(_) => "CreateTable",
            DdlOp::AlterTable { .. } => "AlterTable",
            DdlOp::DropObject { .. } => "DropObject",
            DdlOp::CreateIndex(_) => "CreateIndex",
            DdlOp::CreateView(_) => "CreateView",
            DdlOp::CreateRoutine(_) => "CreateRoutine",
            DdlOp::CreateTrigger(_) => "CreateTrigger",
            DdlOp::CreateSequence(_) => "CreateSequence",
            DdlOp::AlterSequence { .. } => "AlterSequence",
            DdlOp::CreateType(_) => "CreateType",
            DdlOp::Rename { .. } => "Rename",
            DdlOp::SetSchema { .. } => "SetSchema",
            DdlOp::Comment { .. } => "Comment",
            DdlOp::Grant(_) => "Grant",
            DdlOp::Revoke(_) => "Revoke",
            DdlOp::Owner { .. } => "Owner",
            DdlOp::TableRebuild(_) => "TableRebuild",
            DdlOp::Exchange { .. } => "Exchange",
            DdlOp::Raw { .. } => "Raw",
            DdlOp::Data(_) => "Data",
        }
    }
}

/// Where a [`DdlOp::Raw`] text came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawOrigin {
    /// The server's own definition (`SHOW CREATE …`, `pg_get_…def`, `sqlite_schema.sql`).
    ServerText,
    /// Text the user edited (M7 source edit); never rendered by M4.
    UserEdited,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SchemaDef {
    pub name: Name,
    pub if_not_exists: bool,
    pub comment: Option<Name>,
}

/// A table (alg/03 HV-DDL-07 "CREATE TABLE composition").
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TableDef {
    pub name: QName,
    pub if_not_exists: bool,
    pub columns: Vec<ColumnDef>,
    /// Constraints placed inside the table (in a column or as table elements). Constraints
    /// placed after the table are separate [`DdlOp::AlterTable`] ops.
    pub constraints: Vec<ConstraintDef>,
    /// Indexes that are table elements (MySQL `KEY …`). Other dialects render them as
    /// `CREATE INDEX` statements after the table.
    pub indexes: Vec<IndexDef>,
    pub comment: Option<Name>,
    pub ext: TableDefExt,
}

/// Per-dialect table clauses.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum TableDefExt {
    #[default]
    None,
    Pg(PgTableDef),
    MySql(MySqlTableDef),
    Sqlite(SqliteTableDef),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PgTableDef {
    pub unlogged: bool,
    pub temporary: bool,
    /// `PARTITION OF parent` with its raw bound (`FOR VALUES …`, `DEFAULT`).
    pub partition_of: Option<(QName, Name)>,
    /// Raw `PARTITION BY` key (`RANGE (created_at)`).
    pub partition_by: Option<Name>,
    pub inherits: Vec<QName>,
    /// `USING am` (omitted for the default `heap`).
    pub access_method: Option<Name>,
    /// Raw storage parameters (`fillfactor=70`).
    pub with: Vec<Name>,
    pub tablespace: Option<Name>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MySqlTableDef {
    pub engine: Option<Name>,
    pub row_format: Option<Name>,
    pub collation: Option<Name>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SqliteTableDef {
    pub strict: bool,
    pub without_rowid: bool,
}

/// One column.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ColumnDef {
    pub name: Name,
    /// The declared type, raw (may be empty on SQLite).
    pub type_text: Name,
    pub collation: Option<Name>,
    pub nullable: bool,
    /// Raw default expression.
    pub default: Option<Name>,
    pub generated: Option<GeneratedDef>,
    pub identity: Option<IdentityDef>,
    /// MySQL `ON UPDATE` expression, raw.
    pub on_update: Option<Name>,
    pub comment: Option<Name>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GeneratedDef {
    pub expr: Name,
    pub stored: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityDef {
    /// PostgreSQL `GENERATED ALWAYS AS IDENTITY`.
    Always,
    /// PostgreSQL `GENERATED BY DEFAULT AS IDENTITY`.
    ByDefault,
    /// MySQL `AUTO_INCREMENT`; SQLite `AUTOINCREMENT` (rendered with its `INTEGER PRIMARY
    /// KEY`).
    AutoIncrement,
}

/// Where a constraint is written (HV-DDL-09 placement result).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Position {
    /// Inside the definition of this column.
    InColumn(Name),
    /// A table element after the columns.
    #[default]
    InTable,
}

/// A constraint.
#[derive(Debug, Clone, PartialEq)]
pub struct ConstraintDef {
    /// `None`: no `CONSTRAINT name` clause (the server names it).
    pub name: Option<Name>,
    pub body: ConstraintBody,
    pub deferrable: Deferrable,
    /// PostgreSQL `NOT VALID` (only valid after the table).
    pub not_valid: bool,
    pub position: Position,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConstraintBody {
    PrimaryKey {
        columns: Vec<Name>,
    },
    Unique {
        columns: Vec<Name>,
    },
    ForeignKey {
        columns: Vec<Name>,
        target: QName,
        target_columns: Vec<Name>,
        on_update: FkAction,
        on_delete: FkAction,
        match_kind: FkMatch,
    },
    /// The raw boolean expression; rendered `CHECK (<expr>)`.
    Check {
        expr: Name,
    },
    /// The raw text after `EXCLUDE` (`USING gist (room WITH &&)`).
    Exclusion {
        def: Name,
    },
}

impl ConstraintBody {
    /// The constraint's single column, when it has exactly one.
    pub fn single_column(&self) -> Option<&Name> {
        match self {
            ConstraintBody::PrimaryKey { columns }
            | ConstraintBody::Unique { columns }
            | ConstraintBody::ForeignKey { columns, .. } => match columns.as_slice() {
                [c] => Some(c),
                _ => None,
            },
            ConstraintBody::Check { .. } | ConstraintBody::Exclusion { .. } => None,
        }
    }
}

/// One part of an index key.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct IndexPartDef {
    pub column: Option<Name>,
    /// Raw expression (rendered in parentheses).
    pub expr: Option<Name>,
    pub desc: bool,
    /// `Some` only when it differs from the direction's default.
    pub nulls_first: Option<bool>,
    pub prefix_len: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct IndexDef {
    pub name: Name,
    pub table: QName,
    pub unique: bool,
    pub if_not_exists: bool,
    /// Raw access method (`gin`, `HASH`, `FULLTEXT`); `None` or the default is not printed.
    pub method: Option<Name>,
    pub parts: Vec<IndexPartDef>,
    pub include: Vec<Name>,
    /// Raw predicate, rendered after `WHERE` verbatim.
    pub predicate: Option<Name>,
    pub comment: Option<Name>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewKind {
    #[default]
    View,
    Materialized,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ViewBody {
    /// The query only (PostgreSQL `pg_get_viewdef`): the header is synthesized.
    Query(Name),
    /// The complete statement as the server prints it (MySQL `SHOW CREATE VIEW`, SQLite
    /// `sqlite_schema.sql`), emitted verbatim.
    ServerText(Name),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ViewDef {
    pub name: QName,
    pub kind: ViewKind,
    pub or_replace: bool,
    pub body: ViewBody,
    /// Raw `CASCADED` / `LOCAL` (PostgreSQL, with a `Query` body).
    pub check_option: Option<Name>,
    pub comment: Option<Name>,
    pub column_comments: Vec<(Name, Name)>,
}

/// A function or procedure from server text (`pg_get_functiondef`, `SHOW CREATE FUNCTION`).
#[derive(Debug, Clone, PartialEq)]
pub struct RoutineDef {
    pub name: QName,
    pub kind: ObjectKind,
    /// The argument type list (`integer, text`), for `COMMENT ON FUNCTION f(…)`.
    pub sig: Option<Name>,
    pub text: Name,
    pub comment: Option<Name>,
}

/// A trigger from server text (`pg_get_triggerdef`, `SHOW CREATE TRIGGER`,
/// `sqlite_schema.sql`).
#[derive(Debug, Clone, PartialEq)]
pub struct TriggerDef {
    pub name: Name,
    pub table: QName,
    pub text: Name,
    pub comment: Option<Name>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SeqDef {
    pub name: QName,
    pub if_not_exists: bool,
    pub data_type: Option<Name>,
    pub start: Option<i64>,
    pub increment: Option<i64>,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub cache: Option<i64>,
    pub cycle: bool,
    pub comment: Option<Name>,
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum SeqAction {
    /// `OWNED BY table.column`, or `OWNED BY NONE`.
    OwnedBy(Option<(QName, Name)>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeDef {
    pub name: QName,
    pub body: TypeBody,
    pub comment: Option<Name>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeBody {
    Enum {
        labels: Vec<Name>,
    },
    Domain {
        base: Name,
        collation: Option<Name>,
        default: Option<Name>,
        not_null: bool,
        /// `(name, raw expression)`; rendered `CONSTRAINT name CHECK (expr)`.
        checks: Vec<(Option<Name>, Name)>,
    },
    Composite {
        /// `(name, type, collation)`.
        attrs: Vec<(Name, Name, Option<Name>)>,
    },
    Range {
        subtype: Name,
    },
}

/// What a [`DdlOp::Comment`] is about.
#[derive(Debug, Clone, PartialEq)]
pub struct CommentTarget {
    pub kind: ObjectKind,
    pub name: QName,
    /// The column of a column comment.
    pub column: Option<Name>,
    /// The argument type list of a routine.
    pub sig: Option<Name>,
    /// The table of a trigger or constraint comment.
    pub on: Option<QName>,
}

/// One grant (or revoke) of privileges on one object to one grantee.
#[derive(Debug, Clone, PartialEq)]
pub struct GrantDef {
    pub kind: ObjectKind,
    pub object: QName,
    pub sig: Option<Name>,
    /// Upper-case privilege keywords (`SELECT`), sorted.
    pub privileges: Vec<Name>,
    pub columns: Vec<Name>,
    /// PostgreSQL role name, `PUBLIC`; MySQL account `'user'@'host'` as the server prints it.
    pub grantee: Name,
    pub with_grant_option: bool,
}

/// M7's SQLite table rebuild (alg/03 HV-DDL-07 "SQLite `TableRebuild`"). Declared for the
/// IR's completeness; M4 renderers refuse it.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct RebuildPlan {
    pub table: QName,
    pub new_def: TableDef,
    /// `(new column, source expression)`.
    pub column_map: Vec<(Name, Name)>,
}

impl RebuildPlan {
    pub fn new(table: QName, new_def: TableDef, column_map: Vec<(Name, Name)>) -> Self {
        RebuildPlan {
            table,
            new_def,
            column_map,
        }
    }
}

/// A data step inside a migration (M7). M4 renderers refuse it.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum DataStep {
    SetSequenceValue { seq: QName, value: i64 },
    RefreshMaterializedView(QName),
    Backfill { table: QName, sql: Name },
}

/// An `ALTER TABLE` action (alg/03 HV-DDL-05). M4 renders only `AddConstraint`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum AlterAction {
    AddColumn(ColumnDef),
    DropColumn {
        name: Name,
        if_exists: bool,
        cascade: bool,
    },
    RenameColumn {
        from: Name,
        to: Name,
    },
    AlterColumnType {
        name: Name,
        type_text: Name,
        using: Option<Name>,
    },
    SetNotNull(Name),
    DropNotNull(Name),
    SetDefault {
        name: Name,
        expr: Name,
    },
    DropDefault(Name),
    SetColumnComment {
        name: Name,
        text: Option<Name>,
    },
    AddConstraint(ConstraintDef),
    DropConstraint {
        name: Name,
        if_exists: bool,
        cascade: bool,
    },
    ValidateConstraint(Name),
}
