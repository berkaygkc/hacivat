//! M4 Task 1a seams of `hacivat-meta`: every stub fails closed (Global Constraints, "No silent
//! weakening") and the kind codes are the model spike's (`d86313b`).

use std::collections::HashSet;
use std::sync::Arc;

use hacivat_meta::ids::{NsKey, ObjectKey, ServerId};
use hacivat_meta::kind::{KindGroup, ObjectKind};
use hacivat_meta::merge::{MergeError, MergePolicy, NamespaceResult, merge};
use hacivat_meta::name::{DialectId, IdentCtx, NamePart};
use hacivat_meta::snapshot::{Catalog, ServerFacts};
use hacivat_meta::view::{Freshness, Lookup, ModelView, SnapshotView, UnknownReason};
use hacivat_proto::DbmsKind;
use smallvec::smallvec;

fn empty_catalog() -> Catalog {
    Catalog::new(Arc::new(ServerFacts::new(
        DbmsKind::Postgres,
        IdentCtx::new(DialectId::Postgres),
    )))
}

fn empty_view() -> SnapshotView {
    SnapshotView::new(empty_catalog().current())
}

fn path(dotted: &str) -> Vec<NamePart> {
    dotted
        .split('.')
        .map(|p| NamePart {
            text: Arc::from(p),
            quoted: false,
        })
        .collect()
}

fn table_key() -> ObjectKey {
    ObjectKey {
        kind: ObjectKind::Table,
        path: smallvec![Arc::from("db"), Arc::from("public"), Arc::from("t")],
        server_id: Some(ServerId::Oid(16400)),
        overload_sig: None,
        raw_kind: None,
    }
}

#[test]
fn stub_lookup_is_unknown_never_found_or_missing() {
    let v = empty_view();
    for group in [
        KindGroup::Relations,
        KindGroup::Routines,
        KindGroup::Procedures,
        KindGroup::Types,
        KindGroup::Triggers,
        KindGroup::Other,
    ] {
        for p in ["public.t", "t", "db.public.t", ""] {
            assert!(
                matches!(
                    v.lookup(&path(p), group),
                    Lookup::Unknown(UnknownReason::NotLoaded)
                ),
                "{group:?} {p}"
            );
        }
    }
    assert!(matches!(
        v.children(&table_key(), ObjectKind::Column),
        Lookup::Unknown(UnknownReason::NotLoaded)
    ));
    assert!(matches!(
        v.find_by_server_id(&Arc::from("db"), ServerId::Oid(16400)),
        Lookup::Unknown(UnknownReason::NotLoaded)
    ));
    assert_eq!(
        v.freshness(&NsKey::schema(["db", "public"])),
        Freshness::Unknown
    );
}

#[test]
fn stub_fk_referencing_is_unknown() {
    let snap = empty_catalog().current();
    assert!(matches!(
        snap.fk_referencing(&table_key()),
        Lookup::Unknown(UnknownReason::NotLoaded)
    ));
    assert!(matches!(
        snap.resolve(&hacivat_meta::ids::Ref {
            kind: ObjectKind::Table,
            path: table_key().path,
            server_id: None,
        }),
        Lookup::Unknown(UnknownReason::NotLoaded)
    ));
}

// m4-t10 named exception (fix round 1, M-3): `merge` is filled; without a base it is refused.
#[test]
fn merge_without_a_base_is_refused_and_publishes_nothing() {
    let catalog = empty_catalog();
    let before = catalog.current();
    assert!(matches!(
        merge(None, NamespaceResult::empty_full(), &MergePolicy::default()),
        Err(MergeError::NoBase)
    ));
    let publisher = catalog.publisher().expect("the first publisher");
    // ## m4-t09-snapshots (named exception): the publish body landed; a refused change list
    // publishes nothing (an empty list is a no-op that also leaves the generation).
    assert!(
        publisher
            .publish(vec![hacivat_meta::snapshot::CatalogChange::Remove(
                NsKey::schema(["db", "missing"])
            )])
            .is_err()
    );
    assert!(publisher.publish(Vec::new()).is_ok());
    let after = catalog.current();
    assert_eq!(after.generation, before.generation);
    assert!(after.namespaces.is_empty());
    // Single writer: a second publisher is refused.
    assert!(catalog.publisher().is_none());
}

// ## m4-t11-codec (named exception: the cache stub is filled by Task 11; the seam now checks
// that an open that cannot create its file stays memory-only and writes nothing)
#[cfg(all(feature = "persist", unix))]
#[test]
fn stub_cache_is_memory_only() {
    use hacivat_meta::persist::{CacheStore, Encryption, WriterId};
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let ro = dir.path().join("ro");
    std::fs::create_dir(&ro).unwrap();
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o500)).unwrap();
    let file = ro.join("c.v1.sqlite");
    assert!(CacheStore::open(&file, Encryption::None, WriterId(1)).is_err());
    // Memory-only: nothing was written.
    assert!(!file.exists());
    assert_eq!(std::fs::read_dir(&ro).unwrap().count(), 0);
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn stub_system_catalogs_serve_nothing() {
    use hacivat_meta::system::SystemCatalogs;
    for (dbms, version) in [
        (DbmsKind::Postgres, "17.11"),
        (DbmsKind::MySql, "8.4.3"),
        (DbmsKind::MariaDb, "11.4.4"),
        (DbmsKind::Sqlite, "3.50.0"),
    ] {
        assert!(
            SystemCatalogs::empty().for_server(dbms, version).is_none(),
            "{dbms:?}"
        );
    }
}

#[cfg(feature = "persist")]
#[test]
fn row_key_debug_is_redacted() {
    use hacivat_meta::persist::RowKey;
    let shown = format!("{:?}", RowKey::from_bytes(&[7; 32]));
    assert!(!shown.contains('7'), "{shown}");
    let shown = format!("{:#?}", RowKey::from_bytes(&[0xAB; 32]));
    for leak in ["171", "ab", "AB"] {
        assert!(!shown.contains(leak), "{shown}");
    }
}

/// The model spike's table (`d86313b`, `kind.rs`): 66 kinds with their stable cache codes, plus
/// `VirtualTable` (the plan's kind list; SQLite virtual tables), which the spike lacks and which
/// takes the free relation code 14. Codes are never reordered or reused.
const SPIKE_CODES: [(ObjectKind, u16, &str); 67] = [
    (ObjectKind::Root, 3, "root"),
    (ObjectKind::Database, 1, "database"),
    (ObjectKind::Schema, 2, "schema"),
    (ObjectKind::Table, 10, "table"),
    (ObjectKind::View, 11, "view"),
    (ObjectKind::MaterializedView, 12, "materialized-view"),
    (ObjectKind::ForeignTable, 13, "foreign-table"),
    (ObjectKind::VirtualTable, 14, "virtual-table"),
    (ObjectKind::Column, 20, "column"),
    (ObjectKind::Index, 21, "index"),
    (ObjectKind::PrimaryKey, 22, "primary-key"),
    (ObjectKind::UniqueKey, 23, "unique-key"),
    (ObjectKind::ForeignKey, 24, "foreign-key"),
    (ObjectKind::Check, 25, "check"),
    (ObjectKind::Exclusion, 26, "exclusion"),
    (ObjectKind::Default, 27, "default"),
    (ObjectKind::Trigger, 28, "trigger"),
    (ObjectKind::Rule, 29, "rule"),
    (ObjectKind::Policy, 30, "policy"),
    (ObjectKind::Partition, 31, "partition"),
    (ObjectKind::Subpartition, 32, "subpartition"),
    (ObjectKind::Routine, 40, "routine"),
    (ObjectKind::Procedure, 41, "procedure"),
    (ObjectKind::Aggregate, 42, "aggregate"),
    (ObjectKind::Argument, 43, "argument"),
    (ObjectKind::Sequence, 50, "sequence"),
    (ObjectKind::Domain, 60, "domain"),
    (ObjectKind::EnumType, 61, "enum-type"),
    (ObjectKind::CompositeType, 62, "composite-type"),
    (ObjectKind::RangeType, 63, "range-type"),
    (ObjectKind::AliasType, 64, "alias-type"),
    (ObjectKind::TableType, 65, "table-type"),
    (ObjectKind::Synonym, 70, "synonym"),
    (ObjectKind::Extension, 80, "extension"),
    (ObjectKind::Collation, 81, "collation"),
    (ObjectKind::Operator, 82, "operator"),
    (ObjectKind::OperatorClass, 83, "operator-class"),
    (ObjectKind::OperatorFamily, 84, "operator-family"),
    (ObjectKind::Cast, 85, "cast"),
    (ObjectKind::Language, 86, "language"),
    (ObjectKind::AccessMethod, 87, "access-method"),
    (ObjectKind::ForeignDataWrapper, 88, "foreign-data-wrapper"),
    (ObjectKind::ForeignServer, 89, "foreign-server"),
    (ObjectKind::UserMapping, 90, "user-mapping"),
    (ObjectKind::EventTrigger, 91, "event-trigger"),
    (ObjectKind::Role, 100, "role"),
    (ObjectKind::User, 101, "user"),
    (ObjectKind::Login, 102, "login"),
    (ObjectKind::Tablespace, 103, "tablespace"),
    (ObjectKind::Event, 110, "event"),
    (ObjectKind::VirtualTableModule, 120, "virtual-table-module"),
    (ObjectKind::Dictionary, 130, "dictionary"),
    (ObjectKind::RowPolicy, 131, "row-policy"),
    (ObjectKind::SettingsProfile, 132, "settings-profile"),
    (ObjectKind::Quota, 133, "quota"),
    (ObjectKind::Projection, 134, "projection"),
    (ObjectKind::Package, 140, "package"),
    (ObjectKind::PackageBody, 141, "package-body"),
    (ObjectKind::ObjectType, 142, "object-type"),
    (ObjectKind::DbLink, 143, "db-link"),
    (ObjectKind::Collection, 150, "collection"),
    (ObjectKind::MongoView, 151, "mongo-view"),
    (ObjectKind::TimeSeries, 152, "time-series"),
    (ObjectKind::InferredField, 153, "inferred-field"),
    (ObjectKind::RedisDatabase, 160, "redis-database"),
    (ObjectKind::RedisModule, 161, "redis-module"),
    (ObjectKind::Unknown, 65535, "unknown"),
];

#[test]
fn kind_codes_are_unique_and_match_the_spike_table() {
    assert_eq!(ObjectKind::ALL.len(), SPIKE_CODES.len());
    let mut codes = HashSet::new();
    let mut ids = HashSet::new();
    for (kind, code, id) in SPIKE_CODES {
        assert_eq!(kind.code(), code, "{kind:?}");
        assert_eq!(kind.id(), id, "{kind:?}");
        assert_eq!(ObjectKind::from_code(code), Some(kind), "{code}");
        assert!(ObjectKind::ALL.contains(&kind), "{kind:?}");
        assert!(codes.insert(code), "duplicate code {code}");
        assert!(ids.insert(id), "duplicate id {id}");
    }
    // A code this build does not know is not a kind (the decoder maps it to `Unknown`).
    assert_eq!(ObjectKind::from_code(9999), None);
}

/// Fix round 1 (m1): the Hangul fillers and every other default-ignorable or filler code point
/// are flagged; ordinary Turkish and ASCII names are not.
#[test]
fn invisible_chars_cover_default_ignorables_and_fillers() {
    use hacivat_meta::name::has_invisible_chars;
    for c in [
        '\u{3164}',
        '\u{115F}',
        '\u{1160}',
        '\u{FFA0}',
        '\u{00AD}',
        '\u{034F}',
        '\u{061C}',
        '\u{180E}',
        '\u{200B}',
        '\u{200D}',
        '\u{200E}',
        '\u{202E}',
        '\u{2066}',
        '\u{2069}',
        '\u{2060}',
        '\u{FE0F}',
        '\u{FEFF}',
        '\u{1D173}',
        '\u{E0041}',
        '\u{2800}',
        '\u{FFF9}',
        '\u{0600}',
        '\u{0007}',
        '\u{009B}',
    ] {
        assert!(
            has_invisible_chars(&format!("a{c}b")),
            "U+{:04X}",
            u32::from(c)
        );
    }
    for name in [
        "müşteri",
        "MÜŞTERİ",
        "ılık",
        "order items",
        "t_1",
        "€uro",
        "日本",
    ] {
        assert!(!has_invisible_chars(name), "{name}");
    }
}
