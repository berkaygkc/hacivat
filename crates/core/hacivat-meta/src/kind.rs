//! Object kinds, kind groups and the per-DBMS kind schema (HV-MODEL-01; R-M4-26).
//!
//! One flat [`ObjectKind`] enum for every dialect, so DDL producers and diff code can `match`
//! exhaustively over kind × dialect. Every kind has a **stable numeric code** used by the cache
//! format: codes are never reordered or reused; a new kind takes a new code (a format *minor*
//! bump). A code this build does not know decodes as [`ObjectKind::Unknown`]; the raw server
//! kind text then lives in `Props::Generic`. Spec §5.7's `Other(u16)` is this `Unknown` seam.
//!
//! M4 Task 1a seam (spike origin `d86313b`, declarations only): the codes and ids are the
//! declaration; the kind schema ([`ObjectKind::group`], [`ObjectKind::allows_child`],
//! [`ObjectKind::exists_in`]) fails closed until Task 3.

use hacivat_proto::DbmsKind;

macro_rules! object_kinds {
    ($($(#[$m:meta])* $v:ident = $code:literal => $id:literal),* $(,)?) => {
        /// A catalog object kind (alg/02 HV-MODEL-01 data structures).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum ObjectKind { $($(#[$m])* $v,)* }

        impl ObjectKind {
            /// Every kind, in declaration order.
            pub const ALL: &'static [ObjectKind] = &[$(ObjectKind::$v),*];

            /// The stable cache code.
            pub const fn code(self) -> u16 {
                match self { $(ObjectKind::$v => $code,)* }
            }

            /// The kind with cache code `code`, if this build knows it.
            pub const fn from_code(code: u16) -> Option<Self> {
                match code { $($code => Some(ObjectKind::$v),)* _ => None }
            }

            /// The stable machine id (Fluent ids use it: `model-kind-<id>`).
            pub const fn id(self) -> &'static str {
                match self { $(ObjectKind::$v => $id,)* }
            }
        }
    };
}

object_kinds! {
    /// The root of a server-level namespace (roles, tablespaces, databases list).
    Root = 3 => "root",
    Database = 1 => "database",
    Schema = 2 => "schema",
    Table = 10 => "table",
    View = 11 => "view",
    MaterializedView = 12 => "materialized-view",
    ForeignTable = 13 => "foreign-table",
    /// A SQLite virtual table (`CREATE VIRTUAL TABLE … USING fts5`). Not in the model spike's
    /// table; it takes the free relation code 14 (M4 Task 1a).
    VirtualTable = 14 => "virtual-table",
    Column = 20 => "column",
    Index = 21 => "index",
    PrimaryKey = 22 => "primary-key",
    UniqueKey = 23 => "unique-key",
    ForeignKey = 24 => "foreign-key",
    Check = 25 => "check",
    /// PostgreSQL `EXCLUDE` constraint.
    Exclusion = 26 => "exclusion",
    Default = 27 => "default",
    Trigger = 28 => "trigger",
    Rule = 29 => "rule",
    Policy = 30 => "policy",
    Partition = 31 => "partition",
    Subpartition = 32 => "subpartition",
    /// A function (PostgreSQL `prokind = 'f'`, MySQL `FUNCTION`, SQLite `function_list`).
    Routine = 40 => "routine",
    Procedure = 41 => "procedure",
    Aggregate = 42 => "aggregate",
    Argument = 43 => "argument",
    Sequence = 50 => "sequence",
    Domain = 60 => "domain",
    EnumType = 61 => "enum-type",
    CompositeType = 62 => "composite-type",
    RangeType = 63 => "range-type",
    AliasType = 64 => "alias-type",
    TableType = 65 => "table-type",
    Synonym = 70 => "synonym",
    Extension = 80 => "extension",
    Collation = 81 => "collation",
    Operator = 82 => "operator",
    OperatorClass = 83 => "operator-class",
    OperatorFamily = 84 => "operator-family",
    Cast = 85 => "cast",
    Language = 86 => "language",
    AccessMethod = 87 => "access-method",
    ForeignDataWrapper = 88 => "foreign-data-wrapper",
    ForeignServer = 89 => "foreign-server",
    UserMapping = 90 => "user-mapping",
    EventTrigger = 91 => "event-trigger",
    Role = 100 => "role",
    User = 101 => "user",
    Login = 102 => "login",
    Tablespace = 103 => "tablespace",
    /// MySQL / MariaDB scheduled event.
    Event = 110 => "event",
    /// SQLite virtual-table module (`fts5`, `rtree`, …).
    VirtualTableModule = 120 => "virtual-table-module",
    Dictionary = 130 => "dictionary",
    RowPolicy = 131 => "row-policy",
    SettingsProfile = 132 => "settings-profile",
    Quota = 133 => "quota",
    Projection = 134 => "projection",
    Package = 140 => "package",
    PackageBody = 141 => "package-body",
    ObjectType = 142 => "object-type",
    DbLink = 143 => "db-link",
    Collection = 150 => "collection",
    MongoView = 151 => "mongo-view",
    TimeSeries = 152 => "time-series",
    InferredField = 153 => "inferred-field",
    RedisDatabase = 160 => "redis-database",
    RedisModule = 161 => "redis-module",
    /// A kind the server reported that this build does not model (a newer relkind), or a cache
    /// code this build does not know. Shown in the tree; DDL producers refuse it.
    Unknown = 65535 => "unknown",
}

impl ObjectKind {
    /// The kind whose machine id is `id`, if this build knows it.
    pub fn from_id(id: &str) -> Option<Self> {
        ObjectKind::ALL.iter().copied().find(|k| k.id() == id)
    }
}

/// A name space inside one namespace: two objects of the same group cannot share a name (up
/// to the dialect's folding and routine overloading). PostgreSQL composite types sit in
/// `Relations` (they share `pg_class`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum KindGroup {
    /// PostgreSQL `pg_class` kinds; MySQL tables and views; SQLite's whole `sqlite_schema`.
    Relations,
    /// Functions, aggregates; PostgreSQL procedures too (one `pg_proc`).
    Routines,
    /// MySQL / MariaDB procedures (a separate name space from functions).
    Procedures,
    /// Types and domains.
    Types,
    /// Schema-scoped trigger names (MySQL).
    Triggers,
    /// Every other schema-scoped kind (collations, operators, events, extensions, …).
    Other,
}

impl KindGroup {
    /// The number of groups.
    pub const COUNT: usize = 6;

    /// A dense index in `0..COUNT`.
    pub const fn index(self) -> usize {
        match self {
            KindGroup::Relations => 0,
            KindGroup::Routines => 1,
            KindGroup::Procedures => 2,
            KindGroup::Types => 3,
            KindGroup::Triggers => 4,
            KindGroup::Other => 5,
        }
    }
}

/// What a "database" is on a server: PostgreSQL databases hold schemas; a MySQL/MariaDB schema
/// is a database; a SQLite database is a file (with `main`, `temp` and attached schemas).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DbLevel {
    Database,
    SchemaIsDatabase,
    File,
}

/// The database level of `dbms` (R-M4-4: the four M4 engines and their wire-compatible
/// families; the v1.x engines follow their own kind schema in v1.x and default to `Database`).
pub fn database_level(dbms: DbmsKind) -> DbLevel {
    match dbms {
        DbmsKind::MySql | DbmsKind::MariaDb | DbmsKind::TiDb | DbmsKind::ClickHouse => {
            DbLevel::SchemaIsDatabase
        }
        DbmsKind::Sqlite => DbLevel::File,
        _ => DbLevel::Database,
    }
}

impl ObjectKind {
    /// The name group of this kind in `dbms`, or `None` when its names are scoped to the parent
    /// object (columns, constraints; PostgreSQL triggers; MySQL indexes).
    ///
    /// Task 1a stub: `None` for every kind (no namespace-level name index is built until
    /// Task 3; lookups answer `Unknown` meanwhile). Not a fact (fix round 1, m9): nothing may
    /// hide or merge a kind group from it before Task 3.
    pub fn group(self, dbms: DbmsKind) -> Option<KindGroup> {
        let _ = dbms;
        None
    }

    /// Whether `child` may sit directly under `self` in `dbms` (the kind schema).
    ///
    /// Task 1a stub: `false` (no arena can be built until Task 3).
    pub fn allows_child(self, child: ObjectKind, dbms: DbmsKind) -> bool {
        let _ = (child, dbms);
        false
    }

    /// Whether this kind exists in `dbms` at all.
    ///
    /// Task 1a stub: `false` (nothing is asserted to exist until Task 3). Not a fact (fix
    /// round 1, m9): nothing may hide a kind from it before Task 3.
    pub fn exists_in(self, dbms: DbmsKind) -> bool {
        let _ = dbms;
        false
    }
}
