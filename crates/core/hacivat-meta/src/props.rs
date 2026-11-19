//! Per-kind properties with per-engine extensions (HV-MODEL-01 `KindProps`; field lists A-2,
//! facets A-3). Declarations from M4 Task 1a; the `Props` accessors and `RoutineProps`'s
//! default are Task 3's (spike origin `d86313b`).
//!
//! Every variant holds an `Arc`, so [`Props`] is two words and cloning an object bumps a
//! reference count instead of deep-copying. Expressions and type texts are kept **raw**,
//! exactly as the server printed them: never re-quoted, never normalized.

use std::sync::Arc;
use std::time::SystemTime;

use bitflags::bitflags;
use sql_catalog::types::{FnArg, FnKind, Volatility};

use crate::ids::{Ref, SourceRef};
use crate::name::Name;
use crate::snapshot::LoadState;

/// The typed properties of one object.
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
    ForeignKey(Arc<FkProps>),
    /// Check and exclusion constraints.
    Check(Arc<CheckProps>),
    Routine(Arc<RoutineProps>),
    Sequence(Arc<SeqProps>),
    Trigger(Arc<TriggerProps>),
    Type(Arc<TypeProps>),
    /// Kinds with only common properties (extensions, collations, roles, events, …) and
    /// [`crate::kind::ObjectKind::Unknown`] objects (their raw server kind in `raw_kind`).
    Generic(Arc<GenericProps>),
}

impl Props {
    /// The object's comment, whatever its kind (keys, checks and triggers have none).
    pub fn comment(&self) -> Option<&Name> {
        match self {
            Props::None => None,
            Props::Database(p) => p.comment.as_ref(),
            Props::Schema(p) => p.comment.as_ref(),
            Props::Table(p) => p.comment.as_ref(),
            Props::View(p) => p.comment.as_ref(),
            Props::Column(p) => p.comment.as_ref(),
            Props::Index(p) => p.comment.as_ref(),
            Props::Key(_) | Props::ForeignKey(_) | Props::Check(_) | Props::Trigger(_) => None,
            Props::Routine(p) => p.comment.as_ref(),
            Props::Sequence(p) => p.comment.as_ref(),
            Props::Type(p) => p.comment.as_ref(),
            Props::Generic(p) => p.comment.as_ref(),
        }
    }

    /// The overload signature of a routine (part of its identity, `ObjectKey::overload_sig`).
    pub fn overload_sig(&self) -> Option<&Name> {
        match self {
            Props::Routine(p) => p.overload_sig.as_ref(),
            _ => None,
        }
    }

    /// The raw server kind of an unknown object (`ObjectKey::raw_kind`).
    pub fn raw_kind(&self) -> Option<&Name> {
        match self {
            Props::Generic(p) => p.raw_kind.as_ref(),
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

/// PostgreSQL `relpersistence`; MySQL and SQLite temporary tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Persistence {
    #[default]
    Permanent,
    Unlogged,
    Temporary,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TableProps {
    pub comment: Option<Name>,
    pub owner: Option<Name>,
    pub persistence: Persistence,
    /// A partitioned parent (PostgreSQL `relkind = 'p'`).
    pub partitioned: bool,
    /// The partitioned parent of a partition.
    pub partition_of: Option<Ref>,
    /// The partition key, raw (`RANGE (created_at)`).
    pub partition_key: Option<Name>,
    /// The partition bound, raw (`FOR VALUES FROM (…) TO (…)`).
    pub partition_bound: Option<Name>,
    /// PostgreSQL `relrowsecurity` (Task 19 emits `ENABLE ROW LEVEL SECURITY` and the notice
    /// `ddl-policies-not-loaded`).
    pub row_security: bool,
    pub ext: TableExt,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum TableExt {
    #[default]
    None,
    Pg(PgTableExt),
    MySql {
        engine: Option<Name>,
        collation: Option<Name>,
        /// `CREATE_OPTIONS`, raw.
        options: Option<Name>,
    },
    Sqlite {
        without_rowid: bool,
        strict: bool,
        /// The module of a virtual table (`fts5`).
        module: Option<Name>,
    },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PgTableExt {
    /// `pg_class.relkind` as a byte.
    pub relkind: u8,
    pub tablespace: Option<Name>,
    pub access_method: Option<Name>,
    /// `reloptions`, each `name=value` raw.
    pub options: Vec<Name>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ViewProps {
    pub owner: Option<Name>,
    pub comment: Option<Name>,
    pub source: Option<SourceRef>,
    /// `CASCADED` / `LOCAL`, raw.
    pub check_option: Option<Name>,
    pub updatable: Option<bool>,
    /// MySQL `ALGORITHM`, `DEFINER`, `SQL SECURITY`, raw.
    pub algorithm: Option<Name>,
    pub definer: Option<Name>,
    pub sql_security: Option<Name>,
}

/// A column's type as a reference to its type object (PostgreSQL), beside the raw text.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TypeRef {
    pub schema: Option<Name>,
    pub name: Name,
}

/// A column's identity kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Identity {
    Always,
    ByDefault,
    /// MySQL `AUTO_INCREMENT`, SQLite `AUTOINCREMENT`.
    AutoIncrement,
    /// SQLite `INTEGER PRIMARY KEY` (an alias of the rowid).
    RowidAlias,
}

/// A generated column's storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratedKind {
    Stored,
    Virtual,
}

/// A generated column: its storage and its expression, raw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Generated {
    pub kind: GeneratedKind,
    pub expr: Name,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ColumnProps {
    /// The server's position number: PostgreSQL `attnum` (with gaps after `DROP COLUMN`),
    /// MySQL `ORDINAL_POSITION`, SQLite `cid + 1`.
    pub ordinal: u32,
    /// The declared type, raw (`character varying(64)`, `int unsigned`).
    pub raw_type: Name,
    pub type_ref: Option<TypeRef>,
    pub nullable: bool,
    pub default_raw: Option<Name>,
    pub comment: Option<Name>,
    /// SQLite hidden column, MySQL `INVISIBLE`.
    pub hidden: bool,
    /// The fields most columns lack, behind one pointer (R-W1-2: ≤ 250 B per column).
    pub rare: Option<Arc<ColumnRare>>,
    /// The per-engine extras, behind one pointer (R-W1-2); `None` when the engine has none.
    pub ext: Option<Arc<ColumnExt>>,
}

/// The column fields most columns lack (R-W1-2, ruled before Task 11 freezes the cache
/// format): kept out of [`ColumnProps`] so the common column stays small.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ColumnRare {
    /// PostgreSQL 18 named `NOT NULL` constraint.
    pub not_null_constraint: Option<Name>,
    pub identity: Option<Identity>,
    pub generated: Option<Generated>,
    pub collation: Option<Name>,
    /// MySQL `ENUM`/`SET` labels, in declaration order.
    pub enum_labels: Option<Arc<[Name]>>,
}

impl ColumnProps {
    /// The column's identity kind, if any.
    pub fn identity(&self) -> Option<Identity> {
        self.rare.as_ref().and_then(|r| r.identity)
    }

    /// The column's generation, if it is generated.
    pub fn generated(&self) -> Option<&Generated> {
        self.rare.as_ref().and_then(|r| r.generated.as_ref())
    }

    /// The column's collation, if declared.
    pub fn collation(&self) -> Option<&Name> {
        self.rare.as_ref().and_then(|r| r.collation.as_ref())
    }

    /// The per-engine extras ([`ColumnExt::None`] when absent).
    pub fn ext(&self) -> &ColumnExt {
        self.ext.as_deref().unwrap_or(&ColumnExt::None)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum ColumnExt {
    #[default]
    None,
    MySql {
        /// `COLUMN_TYPE`, raw (`enum('a','b')`, `int(10) unsigned zerofill`).
        column_type: Option<Name>,
        /// `EXTRA`, raw (`on update CURRENT_TIMESTAMP`; MySQL and MariaDB texts differ, F9).
        extra: Option<Name>,
        charset: Option<Name>,
    },
    Pg {
        /// `attstorage` when not the type default.
        storage: Option<u8>,
        compression: Option<Name>,
    },
}

/// One column of a key or index, by the server's ordinal and (when a plain column) its name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeyColumn {
    pub ordinal: u32,
    pub name: Option<Name>,
}

/// The shared fields of keys and indexes. `Default` is an **enforced** key with nothing else
/// set (m4-t19 named exception below).
#[derive(Debug, Clone, PartialEq)]
pub struct KeyProps {
    /// Key columns in key order.
    pub columns: Vec<KeyColumn>,
    /// Expression parts, raw, in key order.
    pub expressions: Vec<Name>,
    /// PostgreSQL `INCLUDE` columns.
    pub include: Vec<Name>,
    /// Partial-index predicate, raw.
    pub predicate: Option<Name>,
    /// Per part, in key order: descending.
    pub desc: Vec<bool>,
    pub unique: bool,
    /// PostgreSQL ≥ 15 `indnullsnotdistinct`; `false` below 15 and on other engines.
    pub nulls_not_distinct: bool,
    pub deferrable: bool,
    pub initially_deferred: bool,
    pub not_valid: bool,
    /// PostgreSQL 18 `conenforced`; `true` otherwise.
    pub enforced: bool,
    /// MySQL `IS_VISIBLE = NO`, MariaDB `IGNORED`.
    pub invisible: bool,
    /// The index backing a key, when it has its own name.
    pub backing_index: Option<Name>,
    /// The constraint an implicit index exists for.
    pub implicit_for: Option<Ref>,
    /// The per-part options most keys lack (R-W2-8; m4-t13-assemble named exception).
    pub rare: Option<Arc<KeyRare>>,
}

/// Index properties: the key fields plus the access method.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct IndexProps {
    pub key: KeyProps,
    /// Access method, raw (`btree`, `gin`, `BTREE`, `FULLTEXT`).
    pub method: Option<Name>,
    /// MySQL prefix lengths per part, in key order (`None` = whole column).
    pub prefix_len: Vec<Option<u32>>,
    pub comment: Option<Name>,
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
pub struct FkProps {
    /// Columns in FK order.
    pub columns: Vec<Name>,
    pub target: Ref,
    pub target_columns: Vec<Name>,
    pub on_update: FkAction,
    pub on_delete: FkAction,
    pub match_kind: FkMatch,
    pub deferrable: bool,
    pub initially_deferred: bool,
    pub not_valid: bool,
    pub enforced: bool,
    /// The fields most foreign keys lack (R-W2-8; m4-t13-assemble named exception).
    pub rare: Option<Arc<FkRare>>,
}

/// `Default` is an **enforced** check (m4-t19 named exception below).
#[derive(Debug, Clone, PartialEq)]
pub struct CheckProps {
    /// The bare check expression, raw (the DDL producer writes `CHECK (expr)`): PostgreSQL
    /// `pg_get_expr(conbin, conrelid, true)`, MySQL/MariaDB `CHECK_CLAUSE`. For a PostgreSQL
    /// exclusion constraint, the text after `EXCLUDE` without the deferral words (the producer
    /// writes `EXCLUDE expr` and then the deferral from [`CheckRare`]).
    pub expr: Name,
    pub not_valid: bool,
    pub enforced: bool,
    /// The fields most checks lack (R-W2-8; m4-t13-assemble named exception).
    pub rare: Option<Arc<CheckRare>>,
}

// ## m4-t19-produce (named exception in Task 1a/3's file; T19 review I2): a default key or
// check is **enforced**. A derived `Default` made `enforced = false`, so every constraint built
// with `..Default::default()` read as NOT ENFORCED, a wrong fact for the DDL producer. The
// field name and its meaning are unchanged, so the cache codec (T11: one bool,
// `put_bool(p.enforced)`) needs nothing; when T13 adds `rare` to these structs, these impls
// gain `rare: None`.

impl Default for KeyProps {
    fn default() -> Self {
        KeyProps {
            columns: Vec::new(),
            expressions: Vec::new(),
            include: Vec::new(),
            predicate: None,
            desc: Vec::new(),
            unique: false,
            nulls_not_distinct: false,
            deferrable: false,
            initially_deferred: false,
            not_valid: false,
            enforced: true,
            invisible: false,
            backing_index: None,
            implicit_for: None,
        }
    }
}

impl Default for CheckProps {
    fn default() -> Self {
        CheckProps {
            expr: Name::from(""),
            not_valid: false,
            enforced: true,
        }
    }
}
// ## end m4-t19-produce

/// Routine properties: the `sql_catalog::FnSig` fields (schema and name are the object's) plus
/// the owner, MySQL's definer and specific name (MySQL routine identity is `(ROUTINE_TYPE,
/// SPECIFIC_NAME)`, F9).
#[derive(Debug, Clone, PartialEq)]
pub struct RoutineProps {
    /// `None` = name only (MySQL/MariaDB built-ins from the snapshots).
    pub args: Option<Vec<FnArg>>,
    pub returns: Option<Name>,
    pub returns_set: bool,
    pub kind: FnKind,
    pub volatility: Volatility,
    pub language: Option<Name>,
    pub owner: Option<Name>,
    pub definer: Option<Name>,
    pub specific_name: Option<Name>,
    pub security_definer: bool,
    /// Normalized argument type list; part of the identity of overloads.
    pub overload_sig: Option<Name>,
    pub source: Option<SourceRef>,
    pub comment: Option<Name>,
}

impl Default for RoutineProps {
    /// A function with unknown arguments (`args = None`, name only), unknown volatility and
    /// nothing else known.
    fn default() -> Self {
        RoutineProps {
            args: None,
            returns: None,
            returns_set: false,
            kind: FnKind::Function,
            volatility: Volatility::Unknown,
            language: None,
            owner: None,
            definer: None,
            specific_name: None,
            security_definer: false,
            overload_sig: None,
            source: None,
            comment: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SeqProps {
    pub owner: Option<Name>,
    pub data_type: Option<Name>,
    pub start: Option<i64>,
    pub increment: Option<i64>,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub cache: Option<i64>,
    pub cycle: bool,
    /// The owning column (`OWNED BY`): its table and the column name.
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
    /// The events a trigger fires on.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct TriggerEvents: u8 {
        const INSERT = 1;
        const UPDATE = 2;
        const DELETE = 4;
        const TRUNCATE = 8;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TriggerProps {
    pub timing: TriggerTiming,
    pub events: TriggerEvents,
    pub for_each_row: bool,
    /// The trigger function (PostgreSQL).
    pub function: Option<Ref>,
    pub when_expr: Option<Name>,
    pub source: Option<SourceRef>,
    pub enabled: bool,
    /// MySQL `DEFINER`, raw.
    pub definer: Option<Name>,
}

// ## m4-t19 (controller fix): a trigger is enabled unless the catalog says otherwise. A derived
// `Default` gave `enabled: false`, so any producer path that forgot the field would render a
// disabled trigger (a wrong fact).
impl Default for TriggerProps {
    fn default() -> Self {
        TriggerProps {
            timing: TriggerTiming::default(),
            events: TriggerEvents::default(),
            for_each_row: false,
            function: None,
            when_expr: None,
            source: None,
            enabled: true,
            definer: None,
        }
    }
}

/// One attribute of a composite type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeAttr {
    pub name: Name,
    pub raw_type: Name,
}

/// The body of a type.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum TypeBody {
    /// Labels in sort order.
    Enum {
        labels: Arc<[Name]>,
    },
    Domain {
        base: Name,
        not_null: bool,
        /// PostgreSQL 17+ named domain `NOT NULL` (`contype = 'n'`).
        not_null_constraint: Option<Name>,
        default_raw: Option<Name>,
        /// Check expressions, raw.
        checks: Vec<Name>,
    },
    Composite {
        attrs: Vec<TypeAttr>,
    },
    Range {
        subtype: Name,
    },
    #[default]
    Base,
}

/// Type properties: the owner, the comment and the body.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TypeProps {
    pub owner: Option<Name>,
    pub comment: Option<Name>,
    pub body: TypeBody,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct GenericProps {
    pub owner: Option<Name>,
    /// Extension version, raw.
    pub version: Option<Name>,
    pub comment: Option<Name>,
    /// The server's own kind text of an `ObjectKind::Unknown` object (a newer relkind).
    pub raw_kind: Option<Name>,
}

// Facets (A-3, R-M4-30): side tables with their own load state, loaded on demand.

/// The facets an object may have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FacetKind {
    Sources,
    Grants,
    Dependencies,
    Statistics,
}

/// One facet: its load state and, once loaded (or partial), its value.
#[derive(Debug, Clone, PartialEq)]
pub struct Facet<T> {
    pub state: LoadState,
    pub value: Option<Arc<T>>,
}

impl<T> Facet<T> {
    /// A facet that has not been loaded.
    pub fn not_loaded() -> Self {
        Facet {
            state: LoadState::NotLoaded,
            value: None,
        }
    }
}

/// One privilege grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub grantee: Name,
    /// The privilege, raw (`SELECT`, `USAGE`).
    pub privilege: Name,
    /// Column-level grants: the columns; empty for an object-level grant.
    pub columns: Vec<Name>,
    pub grantable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GrantsFacet {
    pub entries: Vec<Grant>,
}

/// Where a dependency list comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DepsKind {
    /// The server's dependency catalog (PostgreSQL `pg_depend`/`pg_rewrite`).
    #[default]
    Catalog,
    /// Usage views (MySQL `VIEW_TABLE_USAGE`/`VIEW_ROUTINE_USAGE`).
    Usage,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DepsFacet {
    pub depends_on: Vec<Ref>,
    pub kind: DepsKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatsFacet {
    /// PostgreSQL `reltuples` (`-1` → `None`), MySQL `TABLE_ROWS`; display only.
    pub row_estimate: Option<u64>,
    pub at: SystemTime,
}

// ## m4-t13-assemble (R-W2-8): the rare parts of keys, foreign keys and checks. Named exception
// in Task 1a/3's file; the cache codec encodes them from Task 11's merge on (format minor bump).

/// The per-part options of a key or index that most lack (R-W2-8). Each vector runs parallel
/// to [`KeyProps::desc`], in key order, and is **empty** when no part has the option.
/// `with_options` (PostgreSQL index `reloptions`, MySQL `WITH PARSER`) is deferred.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeyRare {
    /// PostgreSQL `indoption` bit 1, raw (a `DESC` part is `NULLS FIRST` by default).
    pub nulls_first: Vec<bool>,
    /// PostgreSQL `indclass` when not the type's default operator class, schema-qualified
    /// outside `pg_catalog`, with its parameters (`gist_trgm_ops(siglen=32)`).
    pub opclass: Vec<Option<Name>>,
    /// PostgreSQL `indcollation` when it differs from the column's collation; SQLite
    /// `index_xinfo.coll` when it differs from the column's declared one.
    pub collation: Vec<Option<Name>>,
}

impl KeyRare {
    /// Whether no option is set (the key then stores no rare part).
    pub fn is_empty(&self) -> bool {
        self.nulls_first.is_empty() && self.opclass.is_empty() && self.collation.is_empty()
    }
}

impl KeyProps {
    /// Per part, raw `NULLS FIRST` (empty when no part has it).
    pub fn nulls_first(&self) -> &[bool] {
        self.rare.as_deref().map_or(&[], |r| &r.nulls_first)
    }

    /// Per part, a non-default operator class (empty when no part has one).
    pub fn opclass(&self) -> &[Option<Name>] {
        self.rare.as_deref().map_or(&[], |r| &r.opclass)
    }

    /// Per part, a collation other than the column's (empty when no part has one).
    pub fn collation(&self) -> &[Option<Name>] {
        self.rare.as_deref().map_or(&[], |r| &r.collation)
    }
}

/// The fields of a foreign key that most lack (R-W2-8).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FkRare {
    /// The referenced table's index the FK relies on: PostgreSQL `conindid` (a unique index or
    /// a key's backing index), MySQL `UNIQUE_CONSTRAINT_NAME` (which names an index).
    pub fk_ref_index: Option<Ref>,
}

impl FkProps {
    /// The referenced table's index the FK relies on, when the server says.
    pub fn fk_ref_index(&self) -> Option<&Ref> {
        self.rare.as_deref().and_then(|r| r.fk_ref_index.as_ref())
    }
}

/// The fields of a check or exclusion constraint that most lack (R-W2-8).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CheckRare {
    /// PostgreSQL `connoinherit` (`CHECK (…) NO INHERIT`).
    pub no_inherit: bool,
    /// A PostgreSQL exclusion constraint's operators (`conexclop`), per key part, in key order,
    /// schema-qualified outside `pg_catalog`.
    pub excl_ops: Vec<Name>,
    /// A PostgreSQL exclusion constraint's `condeferrable`.
    pub excl_deferrable: bool,
    /// A PostgreSQL exclusion constraint's `condeferred`.
    pub excl_initially_deferred: bool,
}

impl CheckProps {
    /// `NO INHERIT` (PostgreSQL).
    pub fn no_inherit(&self) -> bool {
        self.rare.as_deref().is_some_and(|r| r.no_inherit)
    }

    /// An exclusion constraint's operators, per key part.
    pub fn excl_ops(&self) -> &[Name] {
        self.rare.as_deref().map_or(&[], |r| &r.excl_ops)
    }
}
