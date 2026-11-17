//! The PostgreSQL introspector's output: plain structs, local to this crate, one
//! [`PgNamespace`] per schema. They hold catalog facts as the server reports them (names
//! verbatim, expressions as the server deparses them) and no model identity of their own.
//!
//! # Mapping note (for the M4 model, HV-MODEL-01)
//!
//! | Here | Model |
//! |---|---|
//! | every `oid` | the object's server id (`ObjectKey` identity; survives renames) |
//! | [`PgNamespace`] | one `NamespaceSnapshot` (kind group "relations" shares one name space: tables, views, matviews, foreign tables, composite types, sequences — they share `pg_class`) |
//! | [`PgRelation`] (`kind`) | `Table` / `PartitionedTable` / `View` / `MaterializedView` / `ForeignTable`; `CompositeType` relations are the attribute carriers of [`UserTypeKind::Composite`] and map to the type, not to a table |
//! | [`PgColumn`] | `Column` child, ordered by `num` (attnum); `num` is the key that index/constraint column lists ([`IndexElem::Column`], [`PgConstraint::columns`]) refer to |
//! | [`PgIndex`], [`PgConstraint`], [`PgTrigger`] | children of their relation; `PgConstraint::index` links a PK/UK/exclusion constraint to its backing index (`backing_index`, HV-INTRO-13) |
//! | [`ForeignKeyRef`] | `Ref { server_id: table, names: (schema, name) }` plus referenced column numbers; resolved after load, may point out of scope |
//! | [`PgSequence`], [`PgRoutine`], [`PgType`] | namespace-level objects; routines are keyed by `oid`, displayed by `name(identity_args)` (overload signature) |
//! | `comment` everywhere | the object's comment facet |
//! | [`PgWatermark`](super::PgWatermark) | `Watermark::PgXid { w, identity }` |
//! | [`PgNamespaceDelta`] | an incremental `NamespaceResult` (HV-INTRO-05 step 5): `alive` sets are the existence query results, the patches the re-read facets |
//!
//! Not modelled by the spike (documented in the report): ACLs, policies, rules other than
//! the view `_RETURN` rule, extended statistics, foreign-table servers/options, publication
//! membership, tablespaces, storage parameters other than `reloptions`.

use std::collections::BTreeSet;

/// A PostgreSQL object id.
pub type Oid = u32;

/// One schema (`pg_namespace`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgSchema {
    pub oid: Oid,
    pub name: String,
    pub owner: String,
    pub comment: Option<String>,
    /// `pg_catalog`, `information_schema` or another `pg_*` schema.
    pub system: bool,
}

/// The kind of a [`PgRelation`] (`pg_class.relkind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RelKind {
    /// `r`
    Table,
    /// `p`
    PartitionedTable,
    /// `v`
    View,
    /// `m`
    MaterializedView,
    /// `f`
    ForeignTable,
    /// `c`: a standalone composite type's attribute carrier.
    CompositeType,
}

/// `pg_class.relpersistence`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Persistence {
    Permanent,
    Unlogged,
    Temporary,
}

/// A table-like object of `pg_class` and its children.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgRelation {
    pub oid: Oid,
    pub name: String,
    pub kind: RelKind,
    pub owner: String,
    pub persistence: Persistence,
    pub comment: Option<String>,
    /// `pg_inherits` parents in `inhseqno` order (one for a partition).
    pub parents: Vec<Oid>,
    /// A partition's bound (`FOR VALUES …`), as `pg_get_expr` deparses it.
    pub partition_bound: Option<String>,
    /// A partitioned table's key (`RANGE (at)`), as `pg_get_partkeydef` deparses it.
    pub partition_key: Option<String>,
    /// A view's or materialized view's query (`pg_get_viewdef`), at the `Sources` level only.
    pub view_definition: Option<String>,
    /// `relispopulated` (false for a materialized view created `WITH NO DATA`).
    pub populated: bool,
    /// `reloptions` (`fillfactor=70`, …).
    pub options: Vec<String>,
    pub columns: Vec<PgColumn>,
    pub indexes: Vec<PgIndex>,
    pub constraints: Vec<PgConstraint>,
    pub triggers: Vec<PgTrigger>,
}

/// How a column's type is built (`pg_type.typtype`, plus array detection).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeKind {
    Base,
    /// An array type (`typlen = -1 AND typelem <> 0`) of `element`.
    Array {
        element: Oid,
    },
    /// A domain over `base`.
    Domain {
        base: Oid,
    },
    Enum,
    Composite,
    Range,
    Multirange,
    Pseudo,
}

/// A reference to a column's type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgTypeRef {
    pub oid: Oid,
    /// `format_type(atttypid, atttypmod)`: `numeric(10,2)`, `text[]`, `hvpgi_fx.mood`, as the
    /// server shows it under the introspection session's `search_path`.
    pub display: String,
    pub schema: String,
    /// `typname` (`_text` for `text[]`).
    pub name: String,
    pub kind: TypeKind,
}

/// `attidentity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Identity {
    Always,
    ByDefault,
}

/// `attgenerated` (`Virtual` is PostgreSQL 18+).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Generated {
    Stored,
    Virtual,
}

/// A column (`pg_attribute`, `attnum > 0`, not dropped).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgColumn {
    /// `attnum`.
    pub num: i16,
    pub name: String,
    pub ty: PgTypeRef,
    /// `attndims`: informational only; PostgreSQL does not enforce array dimensions.
    pub array_dims: i32,
    pub not_null: bool,
    /// The default expression, or the generation expression when `generated` is set.
    pub default: Option<String>,
    pub identity: Option<Identity>,
    pub generated: Option<Generated>,
    /// The collation name when it differs from the type's default.
    pub collation: Option<String>,
    /// `attinhcount > 0`.
    pub inherited: bool,
    pub comment: Option<String>,
}

/// One index key or included column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexElem {
    /// A plain column, by `attnum` (resolve against [`PgRelation::columns`]).
    Column(i16),
    /// An expression, as `pg_get_indexdef(oid, k, true)` deparses it.
    Expression(String),
}

/// One index key column with its ordering options (`indoption`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexKey {
    pub elem: IndexElem,
    pub descending: bool,
    pub nulls_first: bool,
}

/// An index (`pg_index` + its `pg_class` row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgIndex {
    pub oid: Oid,
    pub name: String,
    /// The access method (`btree`, `gist`, …).
    pub method: String,
    pub unique: bool,
    pub primary: bool,
    pub exclusion: bool,
    /// `indimmediate`: uniqueness checked immediately (false for a deferrable constraint).
    pub immediate: bool,
    pub valid: bool,
    /// `indnullsnotdistinct` (PostgreSQL 15+; false before).
    pub nulls_not_distinct: bool,
    pub keys: Vec<IndexKey>,
    /// `INCLUDE` columns (after `indnkeyatts`).
    pub include: Vec<IndexElem>,
    /// A partial index's predicate.
    pub predicate: Option<String>,
    /// `pg_get_indexdef(oid)`, at the `Sources` level only.
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
    /// `t`: a constraint trigger's constraint.
    Trigger,
    /// `n`: a catalogued NOT NULL constraint (tables: PostgreSQL 18+; domains: 17+).
    NotNull,
}

/// A foreign key's action (`confupdtype`/`confdeltype`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FkAction {
    NoAction,
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}

/// A foreign key's referenced side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignKeyRef {
    pub table: Oid,
    /// The referenced table's schema and name, so the reference survives when its namespace
    /// is out of scope.
    pub schema: String,
    pub name: String,
    pub columns: Vec<i16>,
    pub on_update: FkAction,
    pub on_delete: FkAction,
    /// `f` full, `p` partial, `s` simple.
    pub match_type: char,
}

/// A table constraint (`pg_constraint`, `conrelid <> 0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgConstraint {
    pub oid: Oid,
    pub name: String,
    pub kind: ConstraintKind,
    /// `conkey`, in key order.
    pub columns: Vec<i16>,
    /// `pg_get_constraintdef(oid, true)`.
    pub definition: String,
    pub deferrable: bool,
    pub deferred: bool,
    pub validated: bool,
    /// `conenforced` (PostgreSQL 18+; true before).
    pub enforced: bool,
    /// `conislocal`.
    pub local: bool,
    pub no_inherit: bool,
    /// The backing index (`conindid`) of a PK, UK, exclusion or FK constraint.
    pub index: Option<Oid>,
    pub foreign: Option<ForeignKeyRef>,
    pub comment: Option<String>,
}

/// `tgtype` timing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TriggerTiming {
    Before,
    After,
    InsteadOf,
}

/// `tgtype` events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TriggerEvent {
    Insert,
    Update,
    Delete,
    Truncate,
}

/// A user trigger (`pg_trigger`, not `tgisinternal`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgTrigger {
    pub oid: Oid,
    pub name: String,
    pub timing: TriggerTiming,
    pub events: Vec<TriggerEvent>,
    pub row_level: bool,
    /// `tgenabled`: `O` origin/local, `D` disabled, `R` replica, `A` always.
    pub enabled: char,
    pub function_schema: String,
    pub function_name: String,
    /// `pg_get_triggerdef(oid, true)`.
    pub definition: String,
    /// A constraint trigger (`tgconstraint <> 0`).
    pub constraint_trigger: bool,
    pub comment: Option<String>,
}

/// A sequence (`pg_class` relkind `S` + `pg_sequence`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgSequence {
    pub oid: Oid,
    pub name: String,
    pub owner: String,
    /// `smallint`, `integer` or `bigint`.
    pub data_type: String,
    pub start: i64,
    pub increment: i64,
    pub min: i64,
    pub max: i64,
    pub cache: i64,
    pub cycle: bool,
    /// The owning column `(table oid, attnum)` (`OWNED BY`, serial or identity).
    pub owned_by: Option<(Oid, i16)>,
    /// An identity column's sequence.
    pub identity: bool,
    pub comment: Option<String>,
}

/// `prokind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoutineKind {
    Function,
    Procedure,
    Aggregate,
    Window,
}

/// `provolatile`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Volatility {
    Immutable,
    Stable,
    Volatile,
}

/// A routine (`pg_proc`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgRoutine {
    pub oid: Oid,
    pub name: String,
    pub kind: RoutineKind,
    /// `pg_get_function_identity_arguments`: the overload signature (`a integer, b integer`).
    pub identity_args: String,
    /// `pg_get_function_result` (`integer`, `SETOF integer`, `TABLE(…)`); `None` for a
    /// procedure.
    pub result: Option<String>,
    pub language: String,
    pub volatility: Volatility,
    pub strict: bool,
    pub security_definer: bool,
    pub returns_set: bool,
    /// `proparallel`: `s` safe, `r` restricted, `u` unsafe.
    pub parallel: char,
    pub owner: String,
    /// A member of an extension (`pg_depend` deptype `e`).
    pub from_extension: bool,
    /// `pg_get_functiondef` (functions and procedures), at the `Sources` level only.
    pub definition: Option<String>,
    pub comment: Option<String>,
}

/// A domain constraint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgDomainConstraint {
    pub oid: Oid,
    pub name: String,
    /// `c` check, `n` not-null (PostgreSQL 17+ catalogues domain NOT NULL).
    pub kind: char,
    pub definition: String,
    pub validated: bool,
}

/// What a user-defined type is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserTypeKind {
    Enum {
        labels: Vec<String>,
    },
    Domain {
        base: Oid,
        /// `format_type(typbasetype, typtypmod)`.
        base_display: String,
        not_null: bool,
        default: Option<String>,
        constraints: Vec<PgDomainConstraint>,
    },
    /// A standalone composite type; its attributes are the columns of the
    /// [`RelKind::CompositeType`] relation `relid`.
    Composite {
        relid: Oid,
    },
    Range {
        subtype: String,
    },
    Base,
}

/// A user-defined type (`pg_type`); array types and the row types of tables are not listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgType {
    pub oid: Oid,
    pub name: String,
    pub kind: UserTypeKind,
    pub owner: String,
    pub comment: Option<String>,
}

/// Everything the introspector read of one schema. Every list is sorted (relations,
/// sequences and types by name then oid; routines by name, signature, oid; children by name;
/// columns by number), so two introspections of the same catalog compare equal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgNamespace {
    pub schema: PgSchema,
    pub relations: Vec<PgRelation>,
    pub sequences: Vec<PgSequence>,
    pub routines: Vec<PgRoutine>,
    pub types: Vec<PgType>,
}

/// The object ids an incremental pass found alive in the namespace (HV-INTRO-05 step 4a).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Alive {
    pub relations: BTreeSet<Oid>,
    /// Indexes, constraints (table and domain) and triggers.
    pub children: BTreeSet<Oid>,
    pub sequences: BTreeSet<Oid>,
    pub routines: BTreeSet<Oid>,
    pub types: BTreeSet<Oid>,
}

/// A relation's comments: its own and its columns' (by `attnum`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RelComments {
    pub relation: Option<String>,
    pub columns: Vec<(i16, String)>,
}

/// The re-read facets of one relation. A `Some` facet replaces that facet wholesale; `header`
/// carries the relation's own properties with empty child lists.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RelationPatch {
    pub oid: Oid,
    pub header: Option<PgRelation>,
    pub columns: Option<Vec<PgColumn>>,
    pub indexes: Option<Vec<PgIndex>>,
    pub constraints: Option<Vec<PgConstraint>>,
    pub triggers: Option<Vec<PgTrigger>>,
    pub comments: Option<RelComments>,
}

/// What an incremental pass found changed in one namespace (HV-INTRO-05's
/// `NamespaceResult`): drops are the objects missing from `alive`, the rest are re-read rows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PgNamespaceDelta {
    /// The schema row, when it (or its comment) changed.
    pub schema: Option<PgSchema>,
    pub alive: Alive,
    pub relations: Vec<RelationPatch>,
    pub sequences: Vec<PgSequence>,
    pub routines: Vec<PgRoutine>,
    pub types: Vec<PgType>,
}

impl PgNamespaceDelta {
    /// Whether the pass re-read nothing (drops are judged against the previous snapshot).
    pub fn reads_nothing(&self) -> bool {
        self.schema.is_none()
            && self.relations.is_empty()
            && self.sequences.is_empty()
            && self.routines.is_empty()
            && self.types.is_empty()
    }
}
