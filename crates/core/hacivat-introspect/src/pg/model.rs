// ## m4-t16-pg (spike origin 5a5ab08, `src/pg/model.rs`)
//! The PostgreSQL rows as the pack sections report them: plain structs, one per catalog
//! object, holding catalog facts verbatim (names as the server spells them, expressions as it
//! deparses them). [`super::build`] turns them into the model (`hacivat_meta` objects and a
//! `NamespaceResult`); keys, indexes and constraints go through Task 13's assembly.
//!
//! | Here | Model |
//! |---|---|
//! | every `oid` | the object's server id (`ServerId::Oid`, identity across renames) |
//! | [`PgRelation`] | `Table` (`r`, `p`, partitions), `View`, `MaterializedView`, `ForeignTable` |
//! | [`PgColumn`] | a `Column` child, `ordinal = attnum` (gaps after `DROP COLUMN` stay) |
//! | [`PgIndex`], [`PgConstraint`] | Task 13's `IndexRow` / `ConstraintRow`, assembled together |
//! | [`PgTrigger`] | a `Trigger` child |
//! | [`PgSequence`], [`PgRoutine`], [`PgType`] | namespace-level objects |
//!
//! Composite types are read as types (`RetrieveTypes`, `RetrieveTypeAttributes`), never as
//! relations, so one composite type is one model object (`CompositeType`, identity = its
//! `pg_type` oid).

/// A PostgreSQL object id.
pub type Oid = u32;

/// One schema (`pg_namespace`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgSchema {
    pub oid: Oid,
    pub name: String,
    pub owner: String,
    pub comment: Option<String>,
}

/// The kind of a [`PgRelation`] (`pg_class.relkind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RelKind {
    Table,
    PartitionedTable,
    View,
    MaterializedView,
    ForeignTable,
}

impl RelKind {
    /// `pg_class.relkind` of a relation this pack reads.
    pub fn from_relkind(c: char) -> Option<RelKind> {
        Some(match c {
            'r' => RelKind::Table,
            'p' => RelKind::PartitionedTable,
            'v' => RelKind::View,
            'm' => RelKind::MaterializedView,
            'f' => RelKind::ForeignTable,
            _ => return None,
        })
    }
}

/// `pg_class.relpersistence`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Persistence {
    Permanent,
    Unlogged,
    Temporary,
}

/// One relation's own row (`RetrieveRelations`).
#[derive(Debug, Clone, PartialEq)]
pub struct PgRelation {
    pub oid: Oid,
    pub name: String,
    pub kind: RelKind,
    pub owner: String,
    pub persistence: Persistence,
    pub comment: Option<String>,
    /// A partition's parent: `(oid, schema, name)`.
    pub partition_of: Option<(Oid, String, String)>,
    pub partition_bound: Option<String>,
    pub partition_key: Option<String>,
    /// `pg_get_viewdef` (views and materialized views, `Sources` level).
    pub view_definition: Option<String>,
    pub populated: bool,
    pub options: Vec<String>,
    pub row_security: bool,
    /// `reltuples`; `-1` (never analyzed, PostgreSQL 14+) is `None`.
    pub row_estimate: Option<u64>,
    pub access_method: Option<String>,
    pub tablespace: Option<String>,
}

/// What kind of type a column's type is (`pg_type.typtype`, plus arrays and domains).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeKind {
    Base,
    Array { element: Oid },
    Domain { base: Oid },
    Enum,
    Composite,
    Range,
    Multirange,
    Pseudo,
}

/// A column's type: the display text (`format_type`, schema-qualified outside `pg_catalog`
/// under the pinned `search_path`) and the type's own schema and name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgTypeRef {
    pub oid: Oid,
    pub display: String,
    pub schema: String,
    pub name: String,
    pub kind: TypeKind,
}

/// `pg_attribute.attidentity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Identity {
    Always,
    ByDefault,
}

/// `pg_attribute.attgenerated` (`v` from PostgreSQL 18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Generated {
    Stored,
    Virtual,
}

/// One column (`RetrieveColumns`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgColumn {
    pub num: i16,
    pub name: String,
    pub ty: PgTypeRef,
    pub array_dims: i32,
    pub not_null: bool,
    pub default: Option<String>,
    pub identity: Option<Identity>,
    pub generated: Option<Generated>,
    pub collation: Option<String>,
    pub inherited: bool,
    pub comment: Option<String>,
}

/// One index key slot (`indkey[k]` with its options).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgIndexPart {
    /// `attnum`; 0 for an expression slot.
    pub attnum: i16,
    pub name: Option<String>,
    pub expression: Option<String>,
    pub descending: bool,
    pub nulls_first: bool,
    pub opclass: Option<String>,
    pub collation: Option<String>,
}

/// One index (`RetrieveIndexes`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgIndex {
    pub oid: Oid,
    pub name: String,
    pub method: String,
    pub unique: bool,
    pub primary: bool,
    pub exclusion: bool,
    pub immediate: bool,
    pub valid: bool,
    pub nulls_not_distinct: bool,
    /// The key slots (the first `indnkeyatts`).
    pub keys: Vec<PgIndexPart>,
    /// The `INCLUDE` columns: `(attnum, name)`.
    pub include: Vec<(i16, Option<String>)>,
    pub predicate: Option<String>,
    pub definition: Option<String>,
    pub comment: Option<String>,
}

/// `pg_constraint.contype`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConstraintKind {
    PrimaryKey,
    Unique,
    ForeignKey,
    Check,
    Exclusion,
    Trigger,
    /// PostgreSQL 18's catalogued column `NOT NULL` (17+ for domains).
    NotNull,
}

impl ConstraintKind {
    pub fn contype(self) -> u8 {
        match self {
            ConstraintKind::PrimaryKey => b'p',
            ConstraintKind::Unique => b'u',
            ConstraintKind::ForeignKey => b'f',
            ConstraintKind::Check => b'c',
            ConstraintKind::Exclusion => b'x',
            ConstraintKind::Trigger => b't',
            ConstraintKind::NotNull => b'n',
        }
    }

    pub fn from_contype(c: char) -> Option<ConstraintKind> {
        Some(match c {
            'p' => ConstraintKind::PrimaryKey,
            'u' => ConstraintKind::Unique,
            'f' => ConstraintKind::ForeignKey,
            'c' => ConstraintKind::Check,
            'x' => ConstraintKind::Exclusion,
            't' => ConstraintKind::Trigger,
            'n' => ConstraintKind::NotNull,
            _ => return None,
        })
    }
}

/// A foreign key's referenced side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignKeyRef {
    pub table: Oid,
    pub schema: String,
    pub name: String,
    /// `(attnum, name)` in key order.
    pub columns: Vec<(i16, Option<String>)>,
    /// `confupdtype`, `confdeltype`, `confmatchtype` as the catalog writes them.
    pub on_update: char,
    pub on_delete: char,
    pub match_type: char,
}

/// One table constraint (`RetrieveConstraints`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgConstraint {
    pub oid: Oid,
    pub name: String,
    pub kind: ConstraintKind,
    /// `(attnum, name)` in key order.
    pub columns: Vec<(i16, Option<String>)>,
    pub definition: String,
    pub check_expr: Option<String>,
    pub deferrable: bool,
    pub deferred: bool,
    pub validated: bool,
    pub enforced: bool,
    pub local: bool,
    pub no_inherit: bool,
    pub parent: Option<Oid>,
    /// `conindid`: a key's backing index, a foreign key's referenced index.
    pub index: Option<(Oid, String)>,
    pub foreign: Option<ForeignKeyRef>,
    pub exclusion_ops: Vec<String>,
    pub comment: Option<String>,
}

/// `BEFORE`, `AFTER`, `INSTEAD OF`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TriggerTiming {
    Before,
    After,
    InsteadOf,
}

/// A trigger event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TriggerEvent {
    Insert,
    Update,
    Delete,
    Truncate,
}

/// One user trigger (`RetrieveTriggers`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgTrigger {
    pub oid: Oid,
    pub name: String,
    pub timing: TriggerTiming,
    pub events: Vec<TriggerEvent>,
    pub row_level: bool,
    /// `tgenabled`: `O` origin, `D` disabled, `R` replica, `A` always.
    pub enabled: char,
    pub function: (Oid, String, String),
    pub definition: Option<String>,
    pub constraint_trigger: bool,
    pub comment: Option<String>,
}

/// One sequence (`RetrieveSequences`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgSequence {
    pub oid: Oid,
    pub name: String,
    pub owner: String,
    pub data_type: String,
    pub start: i64,
    pub increment: i64,
    pub min: i64,
    pub max: i64,
    pub cache: i64,
    pub cycle: bool,
    /// `OWNED BY`: `(table oid, table schema, table name, column name)`.
    pub owned_by: Option<(Oid, String, String, String)>,
    /// The sequence of an identity column (`deptype i`).
    pub identity: bool,
    pub comment: Option<String>,
}

/// `pg_proc.prokind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoutineKind {
    Function,
    Procedure,
    Aggregate,
    Window,
}

/// `pg_proc.provolatile`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Volatility {
    Immutable,
    Stable,
    Volatile,
}

/// One routine argument (`proargnames`, `proargmodes`, the types).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgArg {
    pub name: Option<String>,
    pub type_text: String,
    /// `proargmodes`: `i`, `o`, `b` (inout), `v` (variadic), `t` (table).
    pub mode: char,
    pub has_default: bool,
}

/// One routine (`RetrieveRoutines`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgRoutine {
    pub oid: Oid,
    pub name: String,
    pub kind: RoutineKind,
    pub identity_args: String,
    pub result: Option<String>,
    pub language: String,
    pub volatility: Volatility,
    pub strict: bool,
    pub security_definer: bool,
    pub returns_set: bool,
    pub parallel: char,
    pub owner: String,
    pub from_extension: bool,
    pub args: Vec<PgArg>,
    pub definition: Option<String>,
    pub comment: Option<String>,
}

/// One domain constraint (`RetrieveDomainConstraints`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgDomainConstraint {
    pub oid: Oid,
    pub name: String,
    /// `c` check, `n` not null (17+).
    pub kind: char,
    pub definition: String,
    pub validated: bool,
}

/// What a [`PgType`] is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserTypeKind {
    Enum {
        labels: Vec<String>,
    },
    Domain {
        base_display: String,
        not_null: bool,
        default: Option<String>,
        constraints: Vec<PgDomainConstraint>,
    },
    Composite {
        relid: Oid,
        /// `(attnum, name, type text)`.
        attrs: Vec<(i16, String, String)>,
    },
    Range {
        subtype: String,
    },
    Base,
}

/// One user type (`RetrieveTypes`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgType {
    pub oid: Oid,
    pub name: String,
    pub kind: UserTypeKind,
    pub owner: String,
    pub comment: Option<String>,
}

/// A relation with the children a pass read for it (`None`: not read).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RelChildren {
    pub columns: Option<Vec<PgColumn>>,
    pub indexes: Option<Vec<PgIndex>>,
    pub constraints: Option<Vec<PgConstraint>>,
    pub triggers: Option<Vec<PgTrigger>>,
}
