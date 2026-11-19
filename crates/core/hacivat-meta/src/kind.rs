//! Object kinds, kind groups and the per-DBMS kind schema (HV-MODEL-01; R-M4-26).
//!
//! One flat [`ObjectKind`] enum for every dialect, so DDL producers and diff code can `match`
//! exhaustively over kind × dialect. Every kind has a **stable numeric code** used by the cache
//! format: codes are never reordered or reused; a new kind takes a new code (a format *minor*
//! bump). A code this build does not know decodes as [`ObjectKind::Unknown`]; the raw server
//! kind text then lives in `Props::Generic`. Spec §5.7's `Other(u16)` is this `Unknown` seam.
//!
//! The kind schema ([`ObjectKind::group`], [`ObjectKind::allows_child`],
//! [`ObjectKind::exists_in`], [`ObjectKind::name_class`]) is M4 Task 3's (spike origin
//! `d86313b`; PostgreSQL composite types in `Relations`, SQLite virtual tables and the
//! per-engine kind lists added).

use hacivat_proto::DbmsKind;

use crate::name::NameClass;

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

    /// Every group, in [`KindGroup::index`] order.
    pub const ALL: [KindGroup; KindGroup::COUNT] = [
        KindGroup::Relations,
        KindGroup::Routines,
        KindGroup::Procedures,
        KindGroup::Types,
        KindGroup::Triggers,
        KindGroup::Other,
    ];

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

/// PostgreSQL and its wire-compatible forks.
pub(crate) fn pg_family(d: DbmsKind) -> bool {
    matches!(
        d,
        DbmsKind::Postgres
            | DbmsKind::Redshift
            | DbmsKind::Cockroach
            | DbmsKind::Yugabyte
            | DbmsKind::Timescale
    )
}

/// MySQL and its wire-compatible forks.
pub(crate) fn mysql_family(d: DbmsKind) -> bool {
    matches!(d, DbmsKind::MySql | DbmsKind::MariaDb | DbmsKind::TiDb)
}

impl ObjectKind {
    /// The name group of this kind in `dbms`, or `None` when its names are scoped to the parent
    /// object (columns, constraints; PostgreSQL triggers; MySQL indexes). Only grouped kinds
    /// enter a namespace's name index, wherever they sit in the arena (a PostgreSQL index under
    /// its table is still a `Relations` name); the others are found under their parent.
    pub fn group(self, dbms: DbmsKind) -> Option<KindGroup> {
        use ObjectKind::*;
        if dbms == DbmsKind::Sqlite {
            // One `sqlite_schema` name space for tables, views, indexes and triggers.
            return match self {
                Table | View | VirtualTable | Index | Trigger => Some(KindGroup::Relations),
                Routine | Aggregate => Some(KindGroup::Routines),
                Collation | VirtualTableModule => Some(KindGroup::Other),
                _ => None,
            };
        }
        let pg = pg_family(dbms);
        let my = mysql_family(dbms);
        match self {
            Table | View | MaterializedView | ForeignTable | VirtualTable => {
                Some(KindGroup::Relations)
            }
            // `pg_class` holds sequences, indexes and composite types too.
            Sequence | Index | CompositeType if pg => Some(KindGroup::Relations),
            // A MariaDB sequence is a table.
            Sequence if my => Some(KindGroup::Relations),
            Sequence => Some(KindGroup::Other),
            Routine | Aggregate => Some(KindGroup::Routines),
            // MySQL keeps functions and procedures in separate name spaces; `pg_proc` does not.
            Procedure if my => Some(KindGroup::Procedures),
            Procedure => Some(KindGroup::Routines),
            Domain | EnumType | CompositeType | RangeType | AliasType | TableType | ObjectType => {
                Some(KindGroup::Types)
            }
            // MySQL trigger names are unique per schema; PostgreSQL's per table.
            Trigger if my => Some(KindGroup::Triggers),
            Synonym | Extension | Collation | Operator | OperatorClass | OperatorFamily
            | Language | AccessMethod | ForeignDataWrapper | ForeignServer | EventTrigger
            | Event | Dictionary | Package | PackageBody | DbLink | Collection | MongoView
            | TimeSeries | Role | User | Login | Tablespace | Cast | VirtualTableModule => {
                Some(KindGroup::Other)
            }
            _ => None,
        }
    }

    /// The name class `sql_ident::key::match_key` matches this kind's names under in `dbms`
    /// (HV-INTRO-14). MySQL/MariaDB views, triggers and (MariaDB) sequences follow
    /// `lower_case_table_names` like tables: the vendor manuals name "database, table, and
    /// trigger names" as the file-system-dependent ones, and stored routine, event and index
    /// names as case-insensitive everywhere.
    pub fn name_class(self, dbms: DbmsKind) -> NameClass {
        use ObjectKind::*;
        let my = mysql_family(dbms);
        match self {
            Database => NameClass::Database,
            Schema => NameClass::Schema,
            Table | View | MaterializedView | ForeignTable | VirtualTable => NameClass::Table,
            Trigger | Sequence if my => NameClass::Table,
            Column => NameClass::Column,
            Routine | Procedure | Aggregate => NameClass::Routine,
            _ => NameClass::Other,
        }
    }

    /// The kind of a namespace's root object.
    pub fn is_namespace_root(self) -> bool {
        matches!(
            self,
            ObjectKind::Root | ObjectKind::Database | ObjectKind::Schema
        )
    }

    /// Whether this kind exists in `dbms` at all. Exact for the four M4 engines (alg/02
    /// HV-MODEL-01 per-DBMS matrix rows 1, 2 and 4); for the later engines only the M4
    /// engines' specific kinds are excluded until their own kind schema lands (v1.x).
    /// [`ObjectKind::Unknown`] exists everywhere (edge 11: a newer server's kind is kept).
    pub fn exists_in(self, dbms: DbmsKind) -> bool {
        use ObjectKind::*;
        if self == Unknown {
            return true;
        }
        if pg_family(dbms) {
            return matches!(
                self,
                Root | Database
                    | Schema
                    | Table
                    | View
                    | MaterializedView
                    | ForeignTable
                    | Column
                    | Index
                    | PrimaryKey
                    | UniqueKey
                    | ForeignKey
                    | Check
                    | Exclusion
                    | Default
                    | Trigger
                    | Rule
                    | Policy
                    | Partition
                    | Routine
                    | Procedure
                    | Aggregate
                    | Argument
                    | Sequence
                    | Domain
                    | EnumType
                    | CompositeType
                    | RangeType
                    | Extension
                    | Collation
                    | Operator
                    | OperatorClass
                    | OperatorFamily
                    | Cast
                    | Language
                    | AccessMethod
                    | ForeignDataWrapper
                    | ForeignServer
                    | UserMapping
                    | EventTrigger
                    | Role
                    | Tablespace
            );
        }
        if mysql_family(dbms) {
            let maria_like = matches!(dbms, DbmsKind::MariaDb | DbmsKind::TiDb);
            return match self {
                // MariaDB 10.3+: `CREATE SEQUENCE`, `CREATE AGGREGATE FUNCTION`.
                Sequence | Aggregate => maria_like,
                _ => matches!(
                    self,
                    Root | Schema
                        | Table
                        | View
                        | Column
                        | Index
                        | PrimaryKey
                        | UniqueKey
                        | ForeignKey
                        | Check
                        | Trigger
                        | Partition
                        | Subpartition
                        | Routine
                        | Procedure
                        | Argument
                        | Event
                        | User
                        | Role
                ),
            };
        }
        if dbms == DbmsKind::Sqlite {
            return matches!(
                self,
                Root | Schema
                    | Table
                    | View
                    | VirtualTable
                    | Column
                    | Index
                    | PrimaryKey
                    | UniqueKey
                    | ForeignKey
                    | Check
                    | Trigger
                    | Routine
                    | Aggregate
                    | Collation
                    | VirtualTableModule
            );
        }
        match self {
            Exclusion | Rule | Policy | Domain | RangeType | Operator | OperatorClass
            | OperatorFamily | Cast | Language | AccessMethod | ForeignDataWrapper
            | ForeignServer | UserMapping | EventTrigger | MaterializedView | ForeignTable
            | Aggregate | Extension | CompositeType | EnumType => dbms == DbmsKind::Unknown,
            Event => false,
            VirtualTable | VirtualTableModule => false,
            _ => true,
        }
    }

    /// Whether `child` may sit directly under `self` in `dbms` (the kind schema): both kinds
    /// exist there and the edge is one the server's hierarchy has. An
    /// [`ObjectKind::Unknown`] child is accepted under any parent, and anything under an
    /// unknown parent (edge 11).
    pub fn allows_child(self, child: ObjectKind, dbms: DbmsKind) -> bool {
        use ObjectKind::*;
        if !self.exists_in(dbms) || !child.exists_in(dbms) {
            return false;
        }
        if self == Unknown || child == Unknown {
            return true;
        }
        match self {
            Root => matches!(child, Role | User | Login | Tablespace),
            // PostgreSQL roles are cluster-wide (the server namespace); database-level roles and
            // users are SQL Server's.
            Database if pg_family(dbms) => matches!(
                child,
                Extension | Language | Cast | EventTrigger | ForeignDataWrapper | AccessMethod
            ),
            Database => matches!(
                child,
                Extension
                    | Language
                    | Cast
                    | EventTrigger
                    | ForeignDataWrapper
                    | AccessMethod
                    | Role
                    | User
            ),
            Schema => matches!(
                child,
                Table
                    | View
                    | MaterializedView
                    | ForeignTable
                    | VirtualTable
                    | Routine
                    | Procedure
                    | Aggregate
                    | Sequence
                    | Domain
                    | EnumType
                    | CompositeType
                    | RangeType
                    | AliasType
                    | TableType
                    | Synonym
                    | Collation
                    | Operator
                    | OperatorClass
                    | OperatorFamily
                    | Event
                    | VirtualTableModule
                    | Dictionary
                    | Package
                    | PackageBody
                    | ObjectType
                    | Collection
                    | MongoView
                    | TimeSeries
                    | Extension
            ),
            Table | ForeignTable => matches!(
                child,
                Column
                    | Index
                    | PrimaryKey
                    | UniqueKey
                    | ForeignKey
                    | Check
                    | Exclusion
                    | Default
                    | Trigger
                    | Rule
                    | Policy
                    | Partition
                    | Projection
            ),
            VirtualTable => matches!(child, Column),
            View => matches!(child, Column | Trigger | Rule),
            MaterializedView => matches!(child, Column | Index),
            Partition => matches!(child, Subpartition),
            CompositeType | TableType | ObjectType => {
                matches!(child, Column | Index | PrimaryKey | UniqueKey | Check)
            }
            Routine | Procedure | Aggregate => matches!(child, Argument),
            ForeignDataWrapper => matches!(child, ForeignServer),
            ForeignServer => matches!(child, UserMapping),
            Collection | TimeSeries => matches!(child, Index | InferredField),
            InferredField => matches!(child, InferredField),
            Package => matches!(child, Routine | Procedure),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spike's unit check: engine-specific kinds are refused on other engines.
    #[test]
    fn kind_schema_rejects_engine_specific_kinds_elsewhere() {
        use ObjectKind::*;
        assert!(Table.allows_child(Exclusion, DbmsKind::Postgres));
        assert!(!Table.allows_child(Exclusion, DbmsKind::MySql));
        assert!(Schema.allows_child(Event, DbmsKind::MariaDb));
        assert!(!Schema.allows_child(Event, DbmsKind::Postgres));
        assert!(!Column.allows_child(Column, DbmsKind::Postgres));
    }

    /// Every kind that exists on an M4 engine can be placed: it is a namespace root or some
    /// parent kind accepts it there.
    #[test]
    fn every_existing_kind_has_a_parent() {
        for dbms in [
            DbmsKind::Postgres,
            DbmsKind::MySql,
            DbmsKind::MariaDb,
            DbmsKind::Sqlite,
        ] {
            for &k in ObjectKind::ALL {
                if k.exists_in(dbms) && k != ObjectKind::Root {
                    let parents = ObjectKind::ALL
                        .iter()
                        .filter(|p| p.allows_child(k, dbms))
                        .count();
                    assert!(
                        parents > 0 || k.is_namespace_root(),
                        "{k:?} exists in {dbms:?} but has no parent"
                    );
                }
            }
        }
    }
}
