//! HV-MODEL-01 kind schema (M4 Task 3): name groups, allowed parent → child kinds and the kinds
//! that exist per engine, for PostgreSQL, MySQL, MariaDB and SQLite (alg/02 per-DBMS matrix
//! rows 1, 2 and 4), and the database level per engine (reconciliation A-20).

use hacivat_meta::kind::{DbLevel, KindGroup, ObjectKind, database_level};
use hacivat_meta::name::NameClass;
use hacivat_proto::DbmsKind;

use ObjectKind::*;

const PG: DbmsKind = DbmsKind::Postgres;
const MY: DbmsKind = DbmsKind::MySql;
const MARIA: DbmsKind = DbmsKind::MariaDb;
const LITE: DbmsKind = DbmsKind::Sqlite;
const M4: [DbmsKind; 4] = [PG, MY, MARIA, LITE];

/// alg/02 HV-MODEL-01 algorithm step 3: PostgreSQL relations share one name group because they
/// share `pg_class`.
#[test]
fn pg_relations_share_one_kind_group() {
    for k in [
        Table,
        View,
        MaterializedView,
        Sequence,
        Index,
        ForeignTable,
        CompositeType,
    ] {
        assert_eq!(k.group(PG), Some(KindGroup::Relations), "{k:?}");
    }
    assert_eq!(Domain.group(PG), Some(KindGroup::Types));
    assert_eq!(EnumType.group(PG), Some(KindGroup::Types));
    assert_eq!(Routine.group(PG), Some(KindGroup::Routines));
    assert_eq!(
        Procedure.group(PG),
        Some(KindGroup::Routines),
        "one pg_proc"
    );
    assert_eq!(Aggregate.group(PG), Some(KindGroup::Routines));
    // Parent-scoped names: columns, constraints, PostgreSQL triggers, rules, policies.
    for k in [Column, PrimaryKey, ForeignKey, Check, Trigger, Rule, Policy] {
        assert_eq!(k.group(PG), None, "{k:?}");
    }
}

/// The other engines' name spaces (the spike's `groups_follow_server_name_spaces`).
#[test]
fn groups_follow_server_name_spaces() {
    for d in [MY, MARIA] {
        assert_eq!(Table.group(d), Some(KindGroup::Relations));
        assert_eq!(View.group(d), Some(KindGroup::Relations));
        assert_eq!(Index.group(d), None, "MySQL index names are per table");
        assert_eq!(Procedure.group(d), Some(KindGroup::Procedures));
        assert_eq!(Routine.group(d), Some(KindGroup::Routines));
        assert_eq!(Trigger.group(d), Some(KindGroup::Triggers));
        assert_eq!(Event.group(d), Some(KindGroup::Other));
        assert_eq!(Column.group(d), None);
    }
    // MariaDB sequences are tables (one name space with tables).
    assert_eq!(Sequence.group(MARIA), Some(KindGroup::Relations));
    // SQLite: one `sqlite_schema` name space for tables, views, indexes and triggers.
    for k in [Table, View, VirtualTable, Index, Trigger] {
        assert_eq!(k.group(LITE), Some(KindGroup::Relations), "{k:?}");
    }
    assert_eq!(Routine.group(LITE), Some(KindGroup::Routines));
    assert_eq!(Column.group(LITE), None);
}

/// The kinds of each M4 engine (alg/02 matrix): engine-specific kinds exist only there.
#[test]
fn kinds_exist_per_engine() {
    for k in [
        Exclusion,
        Rule,
        Policy,
        Domain,
        RangeType,
        CompositeType,
        EnumType,
        MaterializedView,
        ForeignTable,
        Extension,
        Operator,
        Cast,
        Language,
        AccessMethod,
        ForeignDataWrapper,
        ForeignServer,
        UserMapping,
        EventTrigger,
        Tablespace,
    ] {
        assert!(k.exists_in(PG), "{k:?}");
        for d in [MY, MARIA, LITE] {
            assert!(!k.exists_in(d), "{k:?} in {d:?}");
        }
    }
    for k in [Event, User] {
        assert!(k.exists_in(MY) && k.exists_in(MARIA), "{k:?}");
        assert!(!k.exists_in(PG) && !k.exists_in(LITE), "{k:?}");
    }
    assert!(Sequence.exists_in(MARIA));
    assert!(!Sequence.exists_in(MY));
    assert!(!Sequence.exists_in(LITE));
    for k in [VirtualTable, VirtualTableModule] {
        assert!(k.exists_in(LITE), "{k:?}");
        for d in [PG, MY, MARIA] {
            assert!(!k.exists_in(d), "{k:?} in {d:?}");
        }
    }
    // Every M4 engine has the common relational kinds; none has another engine family's.
    for d in M4 {
        for k in [
            Schema, Table, View, Column, Index, PrimaryKey, ForeignKey, Check, Trigger, Unknown,
        ] {
            assert!(k.exists_in(d), "{k:?} in {d:?}");
        }
        for k in [
            Synonym,
            Package,
            Collection,
            InferredField,
            RedisDatabase,
            Dictionary,
            Quota,
        ] {
            assert!(!k.exists_in(d), "{k:?} in {d:?}");
        }
    }
    assert!(Database.exists_in(PG));
    for d in [MY, MARIA, LITE] {
        assert!(
            !Database.exists_in(d),
            "a {d:?} database is a schema or a file"
        );
    }
}

#[test]
fn kind_schema_rejects_disallowed_child() {
    // The spike's engine-specific checks.
    assert!(Table.allows_child(Exclusion, PG));
    assert!(!Table.allows_child(Exclusion, MY));
    assert!(Schema.allows_child(Event, MARIA));
    assert!(!Schema.allows_child(Event, PG));
    assert!(!Column.allows_child(Column, PG));
    // Structure: columns under relations only; no schema inside a schema; no table under a
    // column; arguments under routines only.
    for d in M4 {
        assert!(Schema.allows_child(Table, d), "{d:?}");
        assert!(Table.allows_child(Column, d), "{d:?}");
        assert!(Table.allows_child(Index, d), "{d:?}");
        assert!(View.allows_child(Column, d), "{d:?}");
        assert!(!Schema.allows_child(Column, d), "{d:?}");
        assert!(!Schema.allows_child(Schema, d), "{d:?}");
        assert!(!Column.allows_child(Table, d), "{d:?}");
        assert!(!Table.allows_child(Table, d), "{d:?}");
        assert!(!Index.allows_child(Column, d), "{d:?}");
        // An unknown kind is kept wherever the server reports it (edge 11).
        assert!(Schema.allows_child(Unknown, d), "{d:?}");
        assert!(Table.allows_child(Unknown, d), "{d:?}");
    }
    assert!(Routine.allows_child(Argument, PG));
    assert!(Procedure.allows_child(Argument, MY));
    // Server level: roles and tablespaces (PG), users and roles (MySQL/MariaDB).
    assert!(Root.allows_child(Role, PG));
    assert!(Root.allows_child(Tablespace, PG));
    assert!(Root.allows_child(User, MY));
    assert!(!Root.allows_child(Table, PG));
    // PostgreSQL database level.
    assert!(Database.allows_child(Extension, PG));
    assert!(Database.allows_child(Cast, PG));
    assert!(
        !Database.allows_child(Role, PG),
        "PostgreSQL roles are cluster-wide"
    );
    // Schemas are namespaces of their own, never objects inside the database arena.
    assert!(!Database.allows_child(Schema, PG));
    // MySQL partitions and subpartitions.
    assert!(Table.allows_child(Partition, MY));
    assert!(Partition.allows_child(Subpartition, MY));
    // SQLite virtual tables have columns; triggers sit under tables and views.
    assert!(Schema.allows_child(VirtualTable, LITE));
    assert!(VirtualTable.allows_child(Column, LITE));
    assert!(Table.allows_child(Trigger, LITE));
    assert!(!Schema.allows_child(VirtualTable, PG));
    // FDW chain (PostgreSQL).
    assert!(ForeignDataWrapper.allows_child(ForeignServer, PG));
    assert!(ForeignServer.allows_child(UserMapping, PG));
}

#[test]
fn namespace_roots_are_root_database_schema() {
    for k in ObjectKind::ALL {
        assert_eq!(
            k.is_namespace_root(),
            matches!(k, Root | Database | Schema),
            "{k:?}"
        );
    }
}

/// reconciliation A-20.
#[test]
fn database_level_per_engine() {
    assert_eq!(database_level(PG), DbLevel::Database);
    assert_eq!(database_level(MY), DbLevel::SchemaIsDatabase);
    assert_eq!(database_level(MARIA), DbLevel::SchemaIsDatabase);
    assert_eq!(database_level(LITE), DbLevel::File);
    for d in [
        DbmsKind::Redshift,
        DbmsKind::Cockroach,
        DbmsKind::Yugabyte,
        DbmsKind::Timescale,
    ] {
        assert_eq!(database_level(d), DbLevel::Database, "{d:?}");
    }
    assert_eq!(database_level(DbmsKind::TiDb), DbLevel::SchemaIsDatabase);
}

/// The name class each kind is matched under (HV-INTRO-14): MySQL/MariaDB triggers follow
/// `lower_case_table_names` like tables (the vendor manuals: "database, table, and trigger
/// names" depend on the file system), routines, events and indexes never do.
#[test]
fn name_classes_follow_the_server_rules() {
    for d in M4 {
        assert_eq!(Table.name_class(d), NameClass::Table, "{d:?}");
        assert_eq!(View.name_class(d), NameClass::Table, "{d:?}");
        assert_eq!(Column.name_class(d), NameClass::Column, "{d:?}");
        assert_eq!(Routine.name_class(d), NameClass::Routine, "{d:?}");
        assert_eq!(Procedure.name_class(d), NameClass::Routine, "{d:?}");
        assert_eq!(Schema.name_class(d), NameClass::Schema, "{d:?}");
        assert_eq!(Index.name_class(d), NameClass::Other, "{d:?}");
    }
    assert_eq!(Trigger.name_class(MY), NameClass::Table);
    assert_eq!(Trigger.name_class(MARIA), NameClass::Table);
    assert_eq!(Trigger.name_class(PG), NameClass::Other);
    assert_eq!(Event.name_class(MY), NameClass::Other);
    assert_eq!(Database.name_class(PG), NameClass::Database);
}

/// The spike's code table check, kept with the kinds (T1a's seam test pins the full table).
#[test]
fn codes_are_unique_and_round_trip() {
    let mut seen = std::collections::HashSet::new();
    for &k in ObjectKind::ALL {
        assert!(seen.insert(k.code()), "duplicate code {}", k.code());
        assert_eq!(ObjectKind::from_code(k.code()), Some(k));
        assert_eq!(ObjectKind::from_id(k.id()), Some(k));
    }
    assert_eq!(ObjectKind::ALL.len(), 67);
    assert_eq!(VirtualTable.code(), 14);
}
