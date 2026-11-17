//! Per-kind properties with per-engine extensions (HV-MODEL-01 `KindProps`).
//!
//! Every variant holds an `Arc`, so [`Props`] is two words and cloning an object (subtree
//! replacement copies the untouched part of a namespace) bumps a reference count instead of
//! deep-copying. Expressions and type texts are kept **raw**, exactly as the server printed
//! them: never re-quoted, never normalized (alg/02 HV-MODEL-01, the DBeaver default pitfall).

use std::sync::Arc;

use bitflags::bitflags;

use crate::ids::{Ref, SourceRef};
use crate::name::Name;

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Props {
    /// No typed properties (the kind has none, or the cache held a props tag this build does
    /// not know).
    #[default]
    None,
    Database(Arc<DatabaseProps>),
    Schema(Arc<SchemaProps>),
    Table(Arc<TableProps>),
    View(Arc<ViewProps>),
    Column(Arc<ColumnProps>),
    Index(Arc<IndexProps>),
    /// Primary and unique keys.
    Key(Arc<KeyProps>),
    ForeignKey(Arc<ForeignKeyProps>),
    /// Check and exclusion constraints.
    Check(Arc<CheckProps>),
    Routine(Arc<RoutineProps>),
    Sequence(Arc<SequenceProps>),
    Trigger(Arc<TriggerProps>),
    Type(Arc<TypeProps>),
    /// Kinds with only common properties (extensions, collations, roles, events, …).
    Generic(Arc<GenericProps>),
}

impl Props {
    /// The object's comment, whatever its kind.
    pub fn comment(&self) -> Option<&Name> {
        match self {
            Props::None => None,
            Props::Database(p) => p.comment.as_ref(),
            Props::Schema(p) => p.comment.as_ref(),
            Props::Table(p) => p.comment.as_ref(),
            Props::View(p) => p.comment.as_ref(),
            Props::Column(p) => p.comment.as_ref(),
            Props::Index(p) => p.comment.as_ref(),
            Props::Key(_) | Props::ForeignKey(_) | Props::Check(_) => None,
            Props::Routine(p) => p.comment.as_ref(),
            Props::Sequence(p) => p.comment.as_ref(),
            Props::Trigger(_) => None,
            Props::Type(p) => p.comment.as_ref(),
            Props::Generic(p) => p.comment.as_ref(),
        }
    }

    /// The overload signature of a routine (part of its identity).
    pub fn overload_sig(&self) -> Option<&Name> {
        match self {
            Props::Routine(p) => p.overload_sig.as_ref(),
            _ => None,
        }
    }

    pub fn as_column(&self) -> Option<&ColumnProps> {
        match self {
            Props::Column(p) => Some(p),
            _ => None,
        }
    }

    pub fn as_table(&self) -> Option<&TableProps> {
        match self {
            Props::Table(p) => Some(p),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DatabaseProps {
    pub owner: Option<Name>,
    pub encoding: Option<Name>,
    pub collation: Option<Name>,
    pub ctype: Option<Name>,
    pub comment: Option<Name>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SchemaProps {
    pub owner: Option<Name>,
    pub comment: Option<Name>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TableProps {
    pub owner: Option<Name>,
    pub comment: Option<Name>,
    /// The partitioned parent of a partition (PostgreSQL declarative partitioning).
    pub partition_of: Option<Ref>,
    /// The partition key expression, raw (`RANGE (created_at)`).
    pub partition_key: Option<Name>,
    /// The partition bound, raw (`FOR VALUES FROM (…) TO (…)`).
    pub partition_bound: Option<Name>,
    /// Estimated row count (PostgreSQL `reltuples`, MySQL `TABLE_ROWS`); display only.
    pub row_estimate: Option<i64>,
    pub ext: TableExt,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum TableExt {
    #[default]
    None,
    Pg(PgTableExt),
    MySql(MySqlTableExt),
    Sqlite(SqliteTableExt),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PgTableExt {
    /// `pg_class.relkind` (`r`, `p`, `f`, …) as a byte.
    pub relkind: u8,
    /// `pg_class.relpersistence` (`p`, `u`, `t`).
    pub persistence: u8,
    pub tablespace: Option<Name>,
    pub access_method: Option<Name>,
    /// `reloptions`, each `name=value` raw.
    pub options: Vec<Name>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MySqlTableExt {
    pub engine: Option<Name>,
    pub row_format: Option<Name>,
    pub collation: Option<Name>,
    pub auto_increment: Option<u64>,
    /// `CREATE_TIME`, microseconds since the Unix epoch (server time zone resolved).
    pub create_time: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SqliteTableExt {
    pub without_rowid: bool,
    pub strict: bool,
    /// The module of a virtual table (`fts5`).
    pub virtual_module: Option<Name>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ViewProps {
    pub owner: Option<Name>,
    pub comment: Option<Name>,
    pub source: Option<SourceRef>,
    /// `CASCADED` / `LOCAL`, raw.
    pub check_option: Option<Name>,
    pub updatable: Option<bool>,
    pub ext: ViewExt,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum ViewExt {
    #[default]
    None,
    MySql(MySqlViewExt),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MySqlViewExt {
    pub algorithm: Option<Name>,
    pub definer: Option<Name>,
    pub sql_security: Option<Name>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Generated {
    #[default]
    No,
    Stored,
    Virtual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Identity {
    #[default]
    No,
    Always,
    ByDefault,
    /// MySQL `AUTO_INCREMENT`, SQLite `AUTOINCREMENT`.
    AutoIncrement,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ColumnProps {
    /// 1-based position as the server reports it.
    pub ordinal: u32,
    /// The declared type, raw (`character varying(64)`, `int unsigned`).
    pub type_text: Name,
    pub nullable: bool,
    pub default_expr: Option<Name>,
    pub generated: Generated,
    pub generation_expr: Option<Name>,
    pub identity: Identity,
    pub collation: Option<Name>,
    pub comment: Option<Name>,
    pub ext: ColumnExt,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum ColumnExt {
    #[default]
    None,
    Pg(PgColumnExt),
    MySql(MySqlColumnExt),
    Sqlite(SqliteColumnExt),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PgColumnExt {
    pub attnum: i16,
    /// `attstorage` as a byte, when not the type default.
    pub storage: Option<u8>,
    pub compression: Option<Name>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MySqlColumnExt {
    /// `COLUMN_TYPE`, raw (`enum('a','b')`, `int(10) unsigned zerofill`).
    pub column_type: Option<Name>,
    /// `EXTRA`, raw (`on update CURRENT_TIMESTAMP`).
    pub extra: Option<Name>,
    pub charset: Option<Name>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SqliteColumnExt {
    /// `table_xinfo.hidden`: 0 normal, 1 hidden (virtual table), 2 virtual generated,
    /// 3 stored generated.
    pub hidden: u8,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct IndexPart {
    /// The column, when the part is a plain column.
    pub column: Option<Name>,
    /// The expression, raw, when the part is an expression.
    pub expr: Option<Name>,
    pub desc: bool,
    pub nulls_first: Option<bool>,
    /// MySQL prefix length.
    pub prefix_len: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct IndexProps {
    pub unique: bool,
    pub primary: bool,
    /// Access method, raw (`btree`, `gin`, `BTREE`, `FULLTEXT`).
    pub method: Option<Name>,
    pub parts: Vec<IndexPart>,
    /// PostgreSQL `INCLUDE` columns.
    pub include: Vec<Name>,
    /// Partial-index predicate, raw.
    pub predicate: Option<Name>,
    pub comment: Option<Name>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Deferrable {
    #[default]
    No,
    InitiallyImmediate,
    InitiallyDeferred,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct KeyProps {
    pub columns: Vec<Name>,
    /// The backing index, when it has its own name.
    pub index_name: Option<Name>,
    pub deferrable: Deferrable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FkAction {
    #[default]
    NoAction,
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FkMatch {
    #[default]
    Simple,
    Full,
    Partial,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForeignKeyProps {
    pub columns: Vec<Name>,
    pub target: Ref,
    pub target_columns: Vec<Name>,
    pub on_update: FkAction,
    pub on_delete: FkAction,
    pub match_kind: FkMatch,
    pub deferrable: Deferrable,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CheckProps {
    /// The check (or exclusion) expression, raw.
    pub expr: Name,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RoutineKind {
    #[default]
    Function,
    Procedure,
    Aggregate,
    Window,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArgMode {
    #[default]
    In,
    Out,
    InOut,
    Variadic,
    Table,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Volatility {
    #[default]
    Unknown,
    Immutable,
    Stable,
    Volatile,
}

/// One routine argument. Arguments are props, not child objects: completion and signature
/// help read the whole list at once.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Arg {
    pub name: Option<Name>,
    pub mode: ArgMode,
    pub type_text: Name,
    pub default_expr: Option<Name>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RoutineProps {
    pub kind: RoutineKind,
    pub args: Vec<Arg>,
    /// Return type, raw; `None` for procedures.
    pub returns: Option<Name>,
    pub returns_set: bool,
    pub language: Option<Name>,
    pub volatility: Volatility,
    pub security_definer: bool,
    /// Normalized argument type list; part of the identity of overloads.
    pub overload_sig: Option<Name>,
    pub source: Option<SourceRef>,
    pub comment: Option<Name>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SequenceProps {
    pub data_type: Option<Name>,
    pub start: Option<i64>,
    pub increment: Option<i64>,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub cache: Option<i64>,
    pub cycle: bool,
    /// The owning column (`OWNED BY`), as a reference to its table plus the column name.
    pub owned_by: Option<(Ref, Name)>,
    pub comment: Option<Name>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TriggerTiming {
    #[default]
    Before,
    After,
    InsteadOf,
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct TriggerEvents: u8 {
        const INSERT = 1;
        const UPDATE = 2;
        const DELETE = 4;
        const TRUNCATE = 8;
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TriggerProps {
    pub timing: TriggerTiming,
    pub events: TriggerEvents,
    pub for_each_row: bool,
    /// The trigger function (PostgreSQL).
    pub function: Option<Ref>,
    pub when_expr: Option<Name>,
    pub source: Option<SourceRef>,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TypeProps {
    /// Base type of a domain or alias type, raw.
    pub base: Option<Name>,
    /// Enum labels in sort order.
    pub labels: Vec<Name>,
    pub not_null: bool,
    pub default_expr: Option<Name>,
    pub comment: Option<Name>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct GenericProps {
    pub owner: Option<Name>,
    /// Extension version, raw.
    pub version: Option<Name>,
    pub comment: Option<Name>,
}
