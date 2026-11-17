//! Object kinds, kind groups and the per-DBMS kind schema (HV-MODEL-01).
//!
//! One flat [`ObjectKind`] enum for every dialect (alg/02 HV-MODEL-01 "Decision"), so DDL
//! producers and diff code can `match` exhaustively over kind × dialect. Every kind has a
//! **stable numeric code** used by the cache format: codes are never reordered or reused; a
//! new kind takes a new code (a format *minor* bump). A code this build does not know decodes
//! as [`ObjectKind::Unknown`].

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

/// A name space inside one namespace: two objects of the same group cannot share a name
/// (up to the dialect's folding and routine overloading).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum KindGroup {
    /// PostgreSQL `pg_class` kinds (tables, views, matviews, foreign tables, sequences,
    /// indexes); MySQL tables and views; SQLite's whole `sqlite_schema` (tables, views,
    /// indexes, triggers).
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

pub(crate) fn mysql_family(d: DbmsKind) -> bool {
    matches!(d, DbmsKind::MySql | DbmsKind::MariaDb | DbmsKind::TiDb)
}

impl ObjectKind {
    /// The name group of this kind in `dbms`, or `None` when its names are scoped to the
    /// parent object (columns, constraints; PostgreSQL triggers; MySQL indexes). Only grouped
    /// kinds enter a namespace's name index; the others are found under their parent.
    pub fn group(self, dbms: DbmsKind) -> Option<KindGroup> {
        use ObjectKind::*;
        if dbms == DbmsKind::Sqlite {
            return match self {
                Table | View | Index | Trigger => Some(KindGroup::Relations),
                Routine | Aggregate => Some(KindGroup::Routines),
                Collation | VirtualTableModule => Some(KindGroup::Other),
                _ => None,
            };
        }
        let pg = pg_family(dbms);
        let my = mysql_family(dbms);
        match self {
            Table | View | MaterializedView | ForeignTable => Some(KindGroup::Relations),
            Sequence if pg => Some(KindGroup::Relations),
            Index if pg => Some(KindGroup::Relations),
            Sequence => Some(KindGroup::Other),
            Routine | Aggregate => Some(KindGroup::Routines),
            Procedure if my => Some(KindGroup::Procedures),
            Procedure => Some(KindGroup::Routines),
            Domain | EnumType | CompositeType | RangeType | AliasType | TableType | ObjectType => {
                Some(KindGroup::Types)
            }
            Trigger if my => Some(KindGroup::Triggers),
            Synonym | Extension | Collation | Operator | OperatorClass | OperatorFamily
            | Language | AccessMethod | ForeignDataWrapper | ForeignServer | EventTrigger
            | Event | Dictionary | Package | PackageBody | DbLink | Collection | MongoView
            | TimeSeries | Role | User | Login | Tablespace | Cast => Some(KindGroup::Other),
            _ => None,
        }
    }

    /// The kind of a namespace's root object.
    pub fn is_namespace_root(self) -> bool {
        matches!(
            self,
            ObjectKind::Root | ObjectKind::Database | ObjectKind::Schema
        )
    }

    /// Whether this kind exists in `dbms` at all (engine-specific kinds only; every other kind
    /// is accepted). The capability registry (HV-DRV-01) takes this over in M4.
    pub fn exists_in(self, dbms: DbmsKind) -> bool {
        use ObjectKind::*;
        match self {
            Exclusion | Rule | Policy | Domain | RangeType | Operator | OperatorClass
            | OperatorFamily | Cast | Language | AccessMethod | ForeignDataWrapper
            | ForeignServer | UserMapping | EventTrigger | MaterializedView | ForeignTable
            | Aggregate | Extension | CompositeType | EnumType => {
                pg_family(dbms) || matches!(dbms, DbmsKind::Unknown)
            }
            Event => mysql_family(dbms),
            VirtualTableModule => dbms == DbmsKind::Sqlite,
            _ => true,
        }
    }

    /// Whether `child` may sit directly under `self` in `dbms` (the kind schema).
    pub fn allows_child(self, child: ObjectKind, dbms: DbmsKind) -> bool {
        use ObjectKind::*;
        if !child.exists_in(dbms) {
            return false;
        }
        if self == Unknown || child == Unknown {
            return true;
        }
        match self {
            Root => matches!(child, Role | User | Login | Tablespace),
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
    use std::collections::HashSet;

    #[test]
    fn codes_are_unique_and_round_trip() {
        let mut seen = HashSet::new();
        for &k in ObjectKind::ALL {
            assert!(seen.insert(k.code()), "duplicate code {}", k.code());
            assert_eq!(ObjectKind::from_code(k.code()), Some(k));
        }
        assert_eq!(ObjectKind::from_code(9999), None);
    }

    #[test]
    fn groups_follow_server_name_spaces() {
        use ObjectKind::*;
        let pg = DbmsKind::Postgres;
        assert_eq!(Index.group(pg), Some(KindGroup::Relations));
        assert_eq!(Index.group(DbmsKind::MySql), None);
        assert_eq!(Trigger.group(DbmsKind::Sqlite), Some(KindGroup::Relations));
        assert_eq!(
            Procedure.group(DbmsKind::MySql),
            Some(KindGroup::Procedures)
        );
        assert_eq!(Procedure.group(pg), Some(KindGroup::Routines));
        assert_eq!(Column.group(pg), None);
    }

    #[test]
    fn kind_schema_rejects_engine_specific_kinds_elsewhere() {
        use ObjectKind::*;
        assert!(Table.allows_child(Exclusion, DbmsKind::Postgres));
        assert!(!Table.allows_child(Exclusion, DbmsKind::MySql));
        assert!(Schema.allows_child(Event, DbmsKind::MariaDb));
        assert!(!Schema.allows_child(Event, DbmsKind::Postgres));
        assert!(!Column.allows_child(Column, DbmsKind::Postgres));
    }
}
