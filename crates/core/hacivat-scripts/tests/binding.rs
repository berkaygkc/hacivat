//! M4b Task 3: logical-database binding (HV-SCRIPT-02; R-M4b-3, -7, -32).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use hacivat_proto::ProfileId;
use hacivat_scripts::binding::logical_for_sqlite;
use hacivat_scripts::names::same_db;
use hacivat_scripts::registry::{AttachGuard, LibraryRegistry, RegistryDiag};
use hacivat_scripts::{
    DatabaseName, DbNode, LibraryId, LibraryRoot, LibrarySnapshot, LibraryState, LocalMapping,
    LogicalDb, resolve,
};
use hacivat_settings::LocalPaths;

fn db(name: &str) -> LogicalDb {
    LogicalDb::parse(name).unwrap()
}

/// A scanned library in folder `folder`, bound to `database` with `aliases`.
fn lib(folder: &str, database: &str, aliases: &[&str]) -> LibrarySnapshot {
    let path = PathBuf::from("/libraries").join(folder);
    LibrarySnapshot {
        generation: 1,
        id: LibraryId::new(),
        root: LibraryRoot {
            id: LibraryId::new(),
            path: path.clone(),
            canonical: path,
        },
        db: db(database),
        aliases: aliases.iter().map(|a| db(a)).collect::<Vec<_>>().into(),
        entries: Arc::from([]),
        state: LibraryState::Watching,
        diagnostics: Arc::from([]),
    }
}

fn node(profile: ProfileId, database: &str) -> DbNode {
    DbNode {
        profile,
        database: DatabaseName::new(database),
        logical: db(database),
    }
}

fn map(profile: ProfileId, database: &str, library: LibraryId) -> LocalMapping {
    LocalMapping {
        profile,
        database: DatabaseName::new(database),
        library,
    }
}

#[test]
fn same_db_is_nfc_then_ascii_ci() {
    assert!(same_db("Orders", "orders"));
    assert!(same_db("mu\u{0308}s\u{0327}teri", "müşteri"));
    assert!(!same_db("İSTANBUL", "istanbul"));
    assert!(!same_db("orders", "orders_p"));
}

/// Locale casing never applies: the dotless and dotted Turkish letters stay distinct from the
/// ASCII ones, and full Unicode case folding is not used (`ß` ≠ `SS`, `Ä` ≠ `ä`). Run this file
/// under `LC_ALL=tr_TR.UTF-8` too (Task 3 step 4).
#[test]
fn same_db_never_uses_locale_or_unicode_casing() {
    for (a, b) in [
        ("İnventory", "inventory"),
        ("INVENTORY", "ınventory"),
        ("ı", "I"),
        ("i", "İ"),
        ("straße", "STRASSE"),
        ("Ärzte", "ärzte"),
        ("orders", "orders "),
        ("", "x"),
    ] {
        assert!(!same_db(a, b), "{a:?} vs {b:?}");
        assert!(!same_db(b, a), "{b:?} vs {a:?}");
    }
    for (a, b) in [
        ("INVENTORY", "inventory"),
        ("caf\u{0065}\u{0301}", "CAFé"),
        ("Ä", "A\u{0308}"),
    ] {
        assert!(same_db(a, b), "{a:?} vs {b:?}");
    }
    let x = db("Orders");
    assert!(x.same_db(&db("ORDERS")));
}

/// alg/28 `binding_table` and the plan's rows.
#[test]
fn binding_table() {
    let p = ProfileId::new();
    let q = ProfileId::new();
    let orders = lib("orders-scripts", "orders", &["orders_db"]);
    let orders2 = lib("team-orders", "ORDERS", &[]);
    let inv = lib("inv", "İnventory", &[]);
    let cafe = lib("cafe", "café", &[]);
    let other = lib("other", "billing", &[]);
    let libs = [
        orders.clone(),
        orders2.clone(),
        inv.clone(),
        cafe.clone(),
        other.clone(),
    ];

    // Automatic by name (ASCII case-insensitive): both libraries bind, ordered by folder name.
    assert_eq!(
        resolve(&node(p, "orders"), &libs, &[]),
        [orders.id, orders2.id]
    );
    assert_eq!(
        resolve(&node(p, "ORDERS"), &libs, &[]),
        [orders.id, orders2.id]
    );
    // By alias.
    assert_eq!(resolve(&node(p, "Orders_DB"), &libs, &[]), [orders.id]);
    // No partial match.
    assert!(resolve(&node(p, "orders_p"), &libs, &[]).is_empty());
    // `İ` is not ASCII: no match.
    assert!(resolve(&node(p, "inventory"), &libs, &[]).is_empty());
    assert_eq!(resolve(&node(p, "İnventory"), &libs, &[]), [inv.id]);
    // NFD vs NFC.
    assert_eq!(resolve(&node(p, "cafe\u{0301}"), &libs, &[]), [cafe.id]);

    // A mapping beats automatic binding for that node only.
    let maps = [map(p, "orders_p", other.id), map(p, "orders", other.id)];
    assert_eq!(resolve(&node(p, "orders_p"), &libs, &maps), [other.id]);
    assert_eq!(resolve(&node(p, "orders"), &libs, &maps), [other.id]);
    // … not for another profile, nor for another database of the same profile.
    assert_eq!(
        resolve(&node(q, "orders"), &libs, &maps),
        [orders.id, orders2.id]
    );
    assert_eq!(resolve(&node(p, "billing"), &libs, &maps), [other.id]);
    assert!(resolve(&node(p, "ORDERS_P"), &libs, &maps).is_empty());
    // The mapping's database is the server's exact name.
    assert_eq!(
        resolve(&node(p, "Orders"), &libs, &maps),
        [orders.id, orders2.id]
    );

    // Folder-name fallback: a library without a manifest is bound by its folder name, which
    // the service puts in `db` (manifest::ManifestRead::logical).
    let fallback = lib("reports", "reports", &[]);
    assert_eq!(
        resolve(&node(p, "Reports"), std::slice::from_ref(&fallback), &[]),
        [fallback.id]
    );

    // A mapping to a library that is no longer there binds nothing (no automatic fallback).
    let gone = LibraryId::new();
    assert!(resolve(&node(p, "orders"), &libs, &[map(p, "orders", gone)]).is_empty());

    // The node's logical name is what binds, not its server name (SQLite: `main` and the stem).
    let sqlite = DbNode {
        profile: p,
        database: DatabaseName::new("main"),
        logical: logical_for_sqlite(Path::new("/data/orders.db")).unwrap(),
    };
    assert_eq!(resolve(&sqlite, &libs, &[]), [orders.id, orders2.id]);
    assert_eq!(
        resolve(&sqlite, &libs, &[map(p, "main", cafe.id)]),
        [cafe.id]
    );
}

/// Two libraries with the same folder name keep a stable order by ID.
#[test]
fn binding_order_is_stable() {
    let p = ProfileId::new();
    let a = lib("Orders", "orders", &[]);
    let b = lib("orders", "orders", &[]);
    let c = lib("alpha", "orders", &[]);
    let mut ab = [a.id, b.id];
    ab.sort();
    let got = resolve(&node(p, "orders"), &[a.clone(), b.clone(), c.clone()], &[]);
    assert_eq!(got, [c.id, ab[0], ab[1]]);
    assert_eq!(resolve(&node(p, "orders"), &[b, c, a], &[]), got);
}

#[test]
fn sqlite_logical_db_is_the_file_stem() {
    assert_eq!(
        logical_for_sqlite(Path::new("/x/orders.db"))
            .unwrap()
            .as_str(),
        "orders"
    );
    assert_eq!(
        logical_for_sqlite(Path::new("/x/orders.backup.sqlite3"))
            .unwrap()
            .as_str(),
        "orders.backup"
    );
    assert_eq!(
        logical_for_sqlite(Path::new("relative/müşteri"))
            .unwrap()
            .as_str(),
        "müşteri"
    );
    // No stem, or one that is not a valid name: no automatic binding.
    assert!(logical_for_sqlite(Path::new("/")).is_none());
    assert!(logical_for_sqlite(Path::new("/x/a\u{202E}b.db")).is_none());
}

/// alg/28 `deleted_profile_drops_mappings` (R-M4b-2).
#[test]
fn deleted_profile_drops_mappings() {
    let app = tempfile::tempdir().unwrap();
    let paths = LocalPaths::under(app.path());
    let lib_dir = tempfile::tempdir().unwrap();
    let (mut reg, _) = LibraryRegistry::load(&paths.libraries_toml());
    let root = reg
        .add(
            lib_dir.path(),
            &AttachGuard::new(&paths, None),
            SystemTime::now(),
        )
        .unwrap();
    let (gone, kept) = (ProfileId::new(), ProfileId::new());
    for (p, d) in [(gone, "a"), (gone, "b"), (kept, "a")] {
        reg.set_mapping(map(p, d, root.id)).unwrap();
    }
    assert_eq!(reg.drop_profile(gone), 2);
    assert_eq!(reg.mappings_for(gone).count(), 0);
    assert_eq!(reg.mappings_for(kept).count(), 1);
    reg.save().unwrap();
    let (reg, diags) = LibraryRegistry::load(&paths.libraries_toml());
    assert!(diags.is_empty(), "{diags:?}");
    assert_eq!(reg.mappings().len(), 1);
    assert_eq!(reg.mappings()[0].profile, kept);

    // A profile deleted while the app was closed: pruned against the known profiles at start.
    let (mut reg, _) = LibraryRegistry::load(&paths.libraries_toml());
    assert_eq!(reg.retain_profiles(|p| p != kept), 1);
    assert!(reg.mappings().is_empty());
    assert!(!matches!(
        LibraryRegistry::load(&paths.libraries_toml()).1.first(),
        Some(RegistryDiag::OrphanMapping { .. })
    ));
}
