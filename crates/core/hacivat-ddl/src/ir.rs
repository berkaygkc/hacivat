//! The DDL IR (alg/03 HV-DDL-07 "IR", HV-DDL-05 alter actions): declared in full by M4 Task 1a
//! (X-6: M7 consumes the variants M4 does not render). Producers build it; renderers turn it
//! into text. Expressions, types and server texts are raw `Name`s, emitted verbatim.
//!
//! Names are [`QualifiedName`]s and are printed with exactly the parts they hold: the producer
//! decides which parts to keep ([`crate::produce::qualify`], HV-DDL-10). Constraints carry no
//! position: the renderer places them with [`crate::produce::placement`] (HV-DDL-09) under
//! `GenOptions::placement`; a constraint postponed past other tables (a cycle, a later table)
//! is an [`AlterAction::AddConstraint`] op of its own.

use hacivat_meta::kind::ObjectKind;
use hacivat_meta::name::Name;
use hacivat_meta::props::{FkAction, FkMatch, Identity, TypeBody};
use smallvec::SmallVec;
pub use sql_ident::QualifiedName;
use sql_ident::ident::Ident;

/// Where server-provided or user text came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RawOrigin {
    ServerText,
    UserEdited,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaDef {
    pub name: Name,
    pub if_not_exists: bool,
    pub comment: Option<Name>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PgTableDef {
    pub unlogged: bool,
    pub temporary: bool,
    /// `PARTITION OF parent FOR VALUES …`: the parent and the raw bound.
    pub partition_of: Option<(QualifiedName, Name)>,
    /// `PARTITION BY …`, raw.
    pub partition_by: Option<Name>,
    pub inherits: Vec<QualifiedName>,
    pub access_method: Option<Name>,
    /// `WITH (…)` storage parameters, each raw.
    pub with: Vec<Name>,
    pub tablespace: Option<Name>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MySqlTableDef {
    pub engine: Option<Name>,
    pub row_format: Option<Name>,
    pub charset: Option<Name>,
    pub collation: Option<Name>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SqliteTableDef {
    pub strict: bool,
    pub without_rowid: bool,
}

/// Table modifiers per dialect.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TableDefExt {
    #[default]
    None,
    Pg(PgTableDef),
    MySql(MySqlTableDef),
    Sqlite(SqliteTableDef),
}

/// A generated column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedDef {
    pub expr: Name,
    pub stored: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnDef {
    pub name: Name,
    /// The type, raw (the server's spelling).
    pub raw_type: Name,
    pub collation: Option<Name>,
    pub nullable: bool,
    pub default_raw: Option<Name>,
    pub generated: Option<GeneratedDef>,
    pub identity: Option<Identity>,
    /// MySQL `ON UPDATE`, raw (texts differ per flavor, F9).
    pub on_update: Option<Name>,
    pub comment: Option<Name>,
    // ## m4-t19-produce (named exception, T19 review I4)
    /// MySQL/MariaDB `INVISIBLE` column, written after the type and any generated clause;
    /// the other dialects refuse it.
    pub invisible: bool,
    // ## end m4-t19-produce
}

/// Constraint deferrability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Deferrable {
    #[default]
    No,
    InitiallyImmediate,
    InitiallyDeferred,
}

/// What a constraint asserts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstraintBody {
    PrimaryKey {
        columns: Vec<Name>,
    },
    Unique {
        columns: Vec<Name>,
        nulls_not_distinct: bool,
    },
    /// Columns in FK order.
    ForeignKey {
        columns: Vec<Name>,
        target: QualifiedName,
        target_columns: Vec<Name>,
        on_update: FkAction,
        on_delete: FkAction,
        match_kind: FkMatch,
    },
    Check {
        expr: Name,
    },
    /// PostgreSQL `EXCLUDE …`, raw (`pg_get_constraintdef`).
    Exclusion {
        raw: Name,
    },
}

impl ConstraintBody {
    /// The key's single column, when it is a primary, unique or foreign key on exactly one
    /// column (HV-DDL-09 `InsideColumn`).
    pub fn single_column(&self) -> Option<&Name> {
        match self {
            ConstraintBody::PrimaryKey { columns }
            | ConstraintBody::Unique { columns, .. }
            | ConstraintBody::ForeignKey { columns, .. } => match columns.as_slice() {
                [c] => Some(c),
                _ => None,
            },
            ConstraintBody::Check { .. } | ConstraintBody::Exclusion { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstraintDef {
    /// `None`: an unnamed constraint (no synthetic name leaks unless `surrogate_names`).
    pub name: Option<Name>,
    pub body: ConstraintBody,
    pub deferrable: Deferrable,
    pub not_valid: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableDef {
    pub name: QualifiedName,
    pub if_not_exists: bool,
    pub columns: Vec<ColumnDef>,
    pub constraints: Vec<ConstraintDef>,
    pub comment: Option<Name>,
    pub ext: TableDefExt,
}

/// One HV-DDL-05 alter action (v1: column add/drop/rename/type/nullability/default/comment;
/// constraint add/drop/validate). M7 adds more.
#[derive(Debug, Clone, PartialEq, Eq)]
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
        column: Name,
        raw_type: Name,
        /// PostgreSQL `USING`, raw.
        using: Option<Name>,
    },
    SetNullability {
        column: Name,
        nullable: bool,
    },
    /// `None` drops the default.
    SetDefault {
        column: Name,
        default_raw: Option<Name>,
    },
    /// `None` removes the comment.
    SetColumnComment {
        column: Name,
        comment: Option<Name>,
    },
    AddConstraint(ConstraintDef),
    DropConstraint {
        name: Name,
        if_exists: bool,
        cascade: bool,
    },
    ValidateConstraint {
        name: Name,
    },
}

/// One part of an index.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexPartDef {
    pub column: Option<Name>,
    pub expr: Option<Name>,
    pub desc: bool,
    pub nulls_first: Option<bool>,
    /// MySQL prefix length.
    pub prefix_len: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexDef {
    pub name: Name,
    pub table: QualifiedName,
    pub unique: bool,
    pub if_not_exists: bool,
    pub concurrently: bool,
    pub method: Option<Name>,
    pub parts: Vec<IndexPartDef>,
    pub include: Vec<Name>,
    pub predicate: Option<Name>,
    pub nulls_not_distinct: bool,
    pub comment: Option<Name>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewKind {
    View,
    Materialized,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewDef {
    pub name: QualifiedName,
    pub kind: ViewKind,
    pub or_replace: bool,
    /// The query, raw (server text).
    pub body: Name,
    pub check_option: Option<Name>,
    pub comment: Option<Name>,
    pub column_comments: Vec<(Name, Name)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineDef {
    pub name: QualifiedName,
    /// `Routine`, `Procedure` or `Aggregate`.
    pub kind: ObjectKind,
    /// The argument type list (PostgreSQL drops need it).
    pub sig: Option<Name>,
    /// The full definition, server text.
    pub text: Name,
    pub comment: Option<Name>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerDef {
    pub name: Name,
    pub table: QualifiedName,
    /// The full definition, server text.
    pub text: Name,
    pub comment: Option<Name>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeqDef {
    pub name: QualifiedName,
    pub if_not_exists: bool,
    pub data_type: Option<Name>,
    pub start: Option<i64>,
    pub increment: Option<i64>,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub cache: Option<i64>,
    pub cycle: bool,
    /// `OWNED BY table.column`.
    pub owned_by: Option<(QualifiedName, Name)>,
    pub comment: Option<Name>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeDef {
    pub name: QualifiedName,
    pub body: TypeBody,
    pub comment: Option<Name>,
}

/// What a `COMMENT` names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentTarget {
    pub kind: ObjectKind,
    pub name: QualifiedName,
    /// A column of `name`.
    pub column: Option<Name>,
    /// A routine's argument types.
    pub sig: Option<Name>,
}

/// A grant or revoke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantDef {
    pub kind: ObjectKind,
    pub object: QualifiedName,
    /// A routine's argument types (`integer, text`; `""` for none). Required for routines: a
    /// name alone could mean another overload (M4 Task 7 addition).
    pub sig: Option<Name>,
    /// Privileges, raw (`SELECT`, `USAGE`).
    pub privileges: Vec<Name>,
    /// Column-level grants: the columns; empty for an object-level grant.
    pub columns: Vec<Name>,
    pub grantee: Name,
    pub grantable: bool,
}

/// The SQLite 12-step table rebuild (HV-DDL-07), or a generic recreate.
#[derive(Debug, Clone, PartialEq)]
pub struct RebuildPlan {
    pub table: QualifiedName,
    pub new_def: Box<TableDef>,
    /// `(new column, expression over the old table)`, in new-column order.
    pub column_map: Vec<(Name, Name)>,
    /// Indexes, triggers and views to drop before and recreate after.
    pub recreate: Vec<DdlOp>,
    /// Whether foreign keys are on (then `PRAGMA foreign_keys` is toggled around the rebuild).
    pub foreign_keys_on: bool,
}

/// A data step inside a migration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataStep {
    /// An `UPDATE`/`INSERT … SELECT` backfill, raw.
    Backfill {
        table: QualifiedName,
        sql: Name,
    },
    SetSequenceValue {
        sequence: QualifiedName,
        value: i64,
    },
    RefreshMaterializedView {
        name: QualifiedName,
    },
}

/// One DDL operation (alg/03 HV-DDL-07).
#[derive(Debug, Clone, PartialEq)]
pub enum DdlOp {
    CreateSchema(SchemaDef),
    CreateTable(TableDef),
    AlterTable {
        table: QualifiedName,
        actions: Vec<AlterAction>,
    },
    DropObject {
        kind: ObjectKind,
        name: QualifiedName,
        sig: Option<Name>,
        if_exists: bool,
        cascade: bool,
    },
    CreateIndex(IndexDef),
    CreateView(ViewDef),
    CreateRoutine(RoutineDef),
    CreateTrigger(TriggerDef),
    CreateSequence(SeqDef),
    CreateType(TypeDef),
    Rename {
        kind: ObjectKind,
        from: QualifiedName,
        to: Name,
    },
    SetSchema {
        kind: ObjectKind,
        name: QualifiedName,
        to: Name,
    },
    /// `text: None` removes the comment (`COMMENT … IS NULL`).
    Comment {
        target: CommentTarget,
        text: Option<Name>,
    },
    Grant(GrantDef),
    Revoke(GrantDef),
    Owner {
        kind: ObjectKind,
        name: QualifiedName,
        /// A routine's argument types, as [`GrantDef::sig`] (M4 Task 7 addition).
        sig: Option<Name>,
        owner: Name,
    },
    /// PostgreSQL `ALTER TABLE … ENABLE|DISABLE ROW LEVEL SECURITY`.
    RowSecurity {
        table: QualifiedName,
        enable: bool,
    },
    TableRebuild(RebuildPlan),
    /// ClickHouse `EXCHANGE TABLES`; never rendered in v1.
    Exchange {
        a: QualifiedName,
        b: QualifiedName,
    },
    Raw {
        sql: String,
        origin: RawOrigin,
    },
    Data(DataStep),
}

impl DdlOp {
    /// The variant's name (renderer errors and tests).
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
            DdlOp::CreateType(_) => "CreateType",
            DdlOp::Rename { .. } => "Rename",
            DdlOp::SetSchema { .. } => "SetSchema",
            DdlOp::Comment { .. } => "Comment",
            DdlOp::Grant(_) => "Grant",
            DdlOp::Revoke(_) => "Revoke",
            DdlOp::Owner { .. } => "Owner",
            DdlOp::RowSecurity { .. } => "RowSecurity",
            DdlOp::TableRebuild(_) => "TableRebuild",
            DdlOp::Exchange { .. } => "Exchange",
            DdlOp::Raw { .. } => "Raw",
            DdlOp::Data(_) => "Data",
        }
    }

    /// A small `CREATE TABLE app.t (id …, name …, PRIMARY KEY (id))` for seam tests.
    #[doc(hidden)]
    pub fn sample_create_table() -> Self {
        DdlOp::CreateTable(sample_table())
    }

    /// One op of every variant, in declaration order (seam tests).
    #[doc(hidden)]
    pub fn samples() -> Vec<DdlOp> {
        let t = qn(&["app", "t"]);
        let grant = GrantDef {
            kind: ObjectKind::Table,
            object: t.clone(),
            sig: None,
            privileges: vec![name("SELECT")],
            columns: Vec::new(),
            grantee: name("reader"),
            grantable: false,
        };
        vec![
            DdlOp::CreateSchema(SchemaDef {
                name: name("app"),
                if_not_exists: false,
                comment: None,
            }),
            DdlOp::CreateTable(sample_table()),
            DdlOp::AlterTable {
                table: t.clone(),
                actions: AlterAction::samples(),
            },
            DdlOp::DropObject {
                kind: ObjectKind::Table,
                name: t.clone(),
                sig: None,
                if_exists: true,
                cascade: false,
            },
            DdlOp::CreateIndex(IndexDef {
                name: name("t_name_idx"),
                table: t.clone(),
                unique: false,
                if_not_exists: false,
                concurrently: false,
                method: None,
                parts: vec![IndexPartDef {
                    column: Some(name("name")),
                    ..IndexPartDef::default()
                }],
                include: Vec::new(),
                predicate: None,
                nulls_not_distinct: false,
                comment: None,
            }),
            DdlOp::CreateView(ViewDef {
                name: qn(&["app", "v"]),
                kind: ViewKind::View,
                or_replace: false,
                body: name("SELECT id FROM app.t"),
                check_option: None,
                comment: None,
                column_comments: Vec::new(),
            }),
            DdlOp::CreateRoutine(RoutineDef {
                name: qn(&["app", "f"]),
                kind: ObjectKind::Routine,
                sig: Some(name("integer")),
                text: name(
                    "CREATE FUNCTION app.f(integer) RETURNS integer LANGUAGE sql AS 'SELECT $1'",
                ),
                comment: None,
            }),
            DdlOp::CreateTrigger(TriggerDef {
                name: name("t_trg"),
                table: t.clone(),
                text: name(
                    "CREATE TRIGGER t_trg BEFORE INSERT ON app.t FOR EACH ROW EXECUTE FUNCTION app.g()",
                ),
                comment: None,
            }),
            DdlOp::CreateSequence(SeqDef {
                name: qn(&["app", "s"]),
                if_not_exists: false,
                data_type: None,
                start: Some(1),
                increment: Some(1),
                min: None,
                max: None,
                cache: None,
                cycle: false,
                owned_by: None,
                comment: None,
            }),
            DdlOp::CreateType(TypeDef {
                name: qn(&["app", "mood"]),
                body: TypeBody::Enum {
                    labels: vec![name("sad"), name("happy")].into(),
                },
                comment: None,
            }),
            DdlOp::Rename {
                kind: ObjectKind::Table,
                from: t.clone(),
                to: name("u"),
            },
            DdlOp::SetSchema {
                kind: ObjectKind::Table,
                name: t.clone(),
                to: name("archive"),
            },
            DdlOp::Comment {
                target: CommentTarget {
                    kind: ObjectKind::Table,
                    name: t.clone(),
                    column: None,
                    sig: None,
                },
                text: Some(name("PM's table")),
            },
            DdlOp::Grant(grant.clone()),
            DdlOp::Revoke(grant),
            DdlOp::Owner {
                kind: ObjectKind::Table,
                name: t.clone(),
                sig: None,
                owner: name("app_owner"),
            },
            DdlOp::RowSecurity {
                table: t.clone(),
                enable: true,
            },
            DdlOp::TableRebuild(RebuildPlan {
                table: t.clone(),
                new_def: Box::new(sample_table()),
                column_map: vec![(name("id"), name("id")), (name("name"), name("name"))],
                recreate: Vec::new(),
                foreign_keys_on: true,
            }),
            DdlOp::Exchange {
                a: t.clone(),
                b: qn(&["app", "t2"]),
            },
            DdlOp::Raw {
                sql: "SELECT 1".to_string(),
                origin: RawOrigin::ServerText,
            },
            DdlOp::Data(DataStep::SetSequenceValue {
                sequence: qn(&["app", "s"]),
                value: 42,
            }),
        ]
    }
}

impl AlterAction {
    /// One action of every v1 variant (seam tests).
    #[doc(hidden)]
    pub fn samples() -> Vec<AlterAction> {
        vec![
            AlterAction::AddColumn(column("note", "text", true)),
            AlterAction::DropColumn {
                name: name("note"),
                if_exists: false,
                cascade: false,
            },
            AlterAction::RenameColumn {
                from: name("name"),
                to: name("title"),
            },
            AlterAction::AlterColumnType {
                column: name("id"),
                raw_type: name("bigint"),
                using: None,
            },
            AlterAction::SetNullability {
                column: name("name"),
                nullable: false,
            },
            AlterAction::SetDefault {
                column: name("name"),
                default_raw: Some(name("''::text")),
            },
            AlterAction::SetColumnComment {
                column: name("name"),
                comment: None,
            },
            AlterAction::AddConstraint(ConstraintDef {
                name: Some(name("t_name_check")),
                body: ConstraintBody::Check {
                    expr: name("(name <> '')"),
                },
                deferrable: Deferrable::No,
                not_valid: true,
            }),
            AlterAction::DropConstraint {
                name: name("t_name_check"),
                if_exists: true,
                cascade: false,
            },
            AlterAction::ValidateConstraint {
                name: name("t_name_check"),
            },
        ]
    }
}

fn name(s: &str) -> Name {
    Name::from(s)
}

fn qn(parts: &[&str]) -> QualifiedName {
    QualifiedName {
        parts: parts
            .iter()
            .map(|p| Ident {
                text: name(p),
                quoted: false,
            })
            .collect::<SmallVec<[Ident; 3]>>(),
    }
}

fn column(n: &str, ty: &str, nullable: bool) -> ColumnDef {
    ColumnDef {
        name: name(n),
        raw_type: name(ty),
        collation: None,
        nullable,
        default_raw: None,
        generated: None,
        identity: None,
        on_update: None,
        comment: None,
        invisible: false, // ## m4-t19-produce
    }
}

fn sample_table() -> TableDef {
    TableDef {
        name: qn(&["app", "t"]),
        if_not_exists: false,
        columns: vec![column("id", "integer", false), column("name", "text", true)],
        constraints: vec![ConstraintDef {
            name: Some(name("t_pkey")),
            body: ConstraintBody::PrimaryKey {
                columns: vec![name("id")],
            },
            deferrable: Deferrable::No,
            not_valid: false,
        }],
        comment: None,
        ext: TableDefExt::None,
    }
}
