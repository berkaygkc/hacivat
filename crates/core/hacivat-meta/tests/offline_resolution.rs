//! M4 Task 30, gate G3 (spec §6 M4 exit, R-M4-5): built-in functions and system views resolve
//! offline for every supported server version, through `sql_catalog::CatalogView` (the trait
//! M5's completion calls) and through `ModelView`.
//!
//! No network, no Docker, no engine: each view is a `SnapshotView` over an empty live catalog
//! (only the server facts a cache would hold) plus the bundled snapshot of that version. With
//! feature `embedded-system` the snapshots are the `include_bytes!` table the app ships;
//! otherwise the committed `assets/system-catalogs` files.
//!
//! Each case prints one `HV-FACT g3 …` line (the gate's evidence).

#![cfg(feature = "system")]

use std::sync::Arc;

use hacivat_meta::kind::KindGroup;
use hacivat_meta::name::{DialectId, IdentCtx, IdentKey, Name, NameClass, NamePart, match_key};
use hacivat_meta::namespace::ObjFlags;
use hacivat_meta::search_path::SearchPath;
use hacivat_meta::snapshot::{Catalog, ServerFacts};
use hacivat_meta::system::SystemCatalogs;
use hacivat_meta::view::{Lookup, ModelView, SnapshotView};
use hacivat_proto::DbmsKind;
use sql_catalog::types::RelRef;
use sql_catalog::view::CatalogView;

/// The current database of the offline PostgreSQL views (any name: the system layer serves
/// every database).
const DB: &str = "app";

/// The bundled set: what the app embeds when the feature is on, else the committed files.
fn bundled() -> SystemCatalogs {
    #[cfg(feature = "embedded-system")]
    {
        SystemCatalogs::from_embedded()
    }
    #[cfg(not(feature = "embedded-system"))]
    {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../assets/system-catalogs");
        SystemCatalogs::open(&dir).expect("the committed snapshot set")
    }
}

/// A supported target (the `docker/m2a` ids) and its snapshot line.
fn target(v: &str) -> (DbmsKind, &'static str) {
    match v {
        "pg14" => (DbmsKind::Postgres, "14"),
        "pg15" => (DbmsKind::Postgres, "15"),
        "pg16" => (DbmsKind::Postgres, "16"),
        "pg17" => (DbmsKind::Postgres, "17"),
        "pg18" => (DbmsKind::Postgres, "18"),
        "mysql80" => (DbmsKind::MySql, "8.0"),
        "mysql84" => (DbmsKind::MySql, "8.4"),
        "mariadb106" => (DbmsKind::MariaDb, "10.6"),
        "mariadb1011" => (DbmsKind::MariaDb, "10.11"),
        "mariadb114" => (DbmsKind::MariaDb, "11.4"),
        other => panic!("not a supported target: {other}"),
    }
}

fn ident(dbms: DbmsKind) -> IdentCtx {
    match dbms {
        DbmsKind::Postgres => IdentCtx {
            encoding: Some(Arc::from("UTF8")),
            ..IdentCtx::new(DialectId::Postgres)
        },
        DbmsKind::MariaDb => IdentCtx::new(DialectId::MariaDb),
        _ => IdentCtx::new(DialectId::MySql),
    }
}

/// An offline view of a server of `dbms` at `version`: an empty live catalog whose facts carry
/// the version (and `extras`, the PostgreSQL initdb-only fact, ruling R-W5-6), layered with
/// what the bundled set gives that version.
fn offline_at(dbms: DbmsKind, version: &str, extras: Option<bool>) -> SnapshotView {
    let mut facts = ServerFacts::new(dbms, ident(dbms));
    facts.version = Name::from(version);
    facts.system_schema_extras = extras;
    let catalog = Catalog::new(Arc::new(facts));
    let path = match dbms {
        DbmsKind::Postgres => {
            SearchPath::for_pg(&Name::from(DB), &[Name::from("public")], None, None)
        }
        _ => SearchPath::for_mysql(None),
    };
    SnapshotView::new(catalog.current())
        .with_layers(Arc::new(bundled().overlays_for(dbms, version)))
        .with_search_path(path)
}

/// The offline view of target `v` at the exact server release its snapshot was read from
/// (the facts an offline cache holds after a connect); the initdb-only fact is not known.
fn offline_view(v: &str) -> SnapshotView {
    let (dbms, line) = target(v);
    let snap = bundled()
        .for_server(dbms, line)
        .unwrap_or_else(|| panic!("{v}: no bundled snapshot for line {line}"));
    assert_eq!(&*snap.line, line, "{v}");
    offline_at(dbms, &snap.server_version, None)
}

fn parts(dotted: &str) -> Vec<NamePart> {
    dotted
        .split('.')
        .map(|t| NamePart {
            text: Arc::from(t),
            quoted: false,
        })
        .collect()
}

fn key(view: &SnapshotView, text: &str, class: NameClass) -> IdentKey {
    let ctx = &view.snapshot().server.ident;
    match_key(ctx.dialect, ctx, text, false, class).expect("a pinned key")
}

fn split(path: &str) -> (&str, &str) {
    path.split_once('.').expect("schema.name")
}

fn label(view: &SnapshotView) -> String {
    let s = view.snapshot();
    format!("{:?} {}", s.server.dbms, s.server.version)
}

/// `path` resolves through `ModelView` as a pre-introspected system object of `group` and
/// through `CatalogView::{relation, columns}` with every column of `cols`.
fn assert_found(view: &SnapshotView, path: &str, group: KindGroup, cols: &[&str]) {
    let at = label(view);
    let hit = view.lookup(&parts(path), group);
    let Lookup::Found(h) = &hit else {
        panic!("{at}: ModelView {path}: {hit:?}");
    };
    let flags = h.obj().flags();
    assert!(
        flags.contains(ObjFlags::SYSTEM | ObjFlags::PRE_INTROSPECTED),
        "{at}: {path} {flags:?}"
    );
    let (schema, name) = split(path);
    let rel = view.relation(
        &key(view, schema, NameClass::Schema),
        &key(view, name, NameClass::Table),
    );
    let Lookup::Found(info) = &rel else {
        panic!("{at}: CatalogView::relation {path}: {rel:?}");
    };
    let columns = view.columns(&RelRef {
        schema: info.name.schema.clone(),
        name: info.name.name.clone(),
    });
    let Lookup::Found(columns) = &columns else {
        panic!("{at}: CatalogView::columns {path}: {columns:?}");
    };
    for c in cols {
        assert!(
            columns.iter().any(|x| &*x.name == *c),
            "{at}: {path} lacks column {c}"
        );
    }
    println!(
        "HV-FACT g3 found target=\"{at}\" object={path} via=ModelView,CatalogView columns={}",
        columns.len()
    );
}

/// The routines `CatalogView::functions` answers for `schema.name`, required `Found`.
fn functions(view: &SnapshotView, schema: &str, name: &str) -> Vec<sql_catalog::types::FnSig> {
    let at = label(view);
    let got = view.functions(
        &key(view, schema, NameClass::Schema),
        &key(view, name, NameClass::Routine),
    );
    let Lookup::Found(sigs) = got else {
        panic!("{at}: CatalogView::functions {schema}.{name}: {got:?}");
    };
    sigs.iter()
        .filter(|f| f.name.eq_ignore_ascii_case(name))
        .cloned()
        .collect()
}

/// A PostgreSQL built-in: found through both layers, with its argument list (`Some`).
fn assert_routine_found(view: &SnapshotView, path: &str) {
    let at = label(view);
    assert!(
        matches!(
            view.lookup(&parts(path), KindGroup::Routines),
            Lookup::Found(_)
        ),
        "{at}: ModelView {path}"
    );
    let (schema, name) = split(path);
    let sigs = functions(view, schema, name);
    assert!(!sigs.is_empty(), "{at}: {path}");
    assert!(
        sigs.iter().all(|f| f.args.is_some()),
        "{at}: {path} {sigs:?}"
    );
    println!(
        "HV-FACT g3 routine target=\"{at}\" object={path} overloads={} args=known",
        sigs.len()
    );
}

/// A MySQL/MariaDB built-in function: known by name only (`args == None`, never "takes no
/// arguments"), joined to any schema's answer; the snapshots carry no invented signature.
fn assert_routine_name_only(view: &SnapshotView, name: &str) {
    let at = label(view);
    let sigs = functions(view, "sys", name);
    assert_eq!(sigs.len(), 1, "{at}: {name} {sigs:?}");
    assert!(sigs[0].args.is_none(), "{at}: {name} {:?}", sigs[0]);
    assert!(sigs[0].is_name_only(), "{at}: {name}");
    assert!(sigs[0].returns.is_none(), "{at}: {name}");
    println!("HV-FACT g3 builtin target=\"{at}\" name={name} args=none");
}

/// A `sys` routine with the real signature the snapshot read from the server.
fn assert_routine_with_signature(view: &SnapshotView, path: &str, args: &[(&str, &str)]) {
    let at = label(view);
    let (schema, name) = split(path);
    // The schema's own routine; MySQL 8.0.16+ also has a native `FORMAT_BYTES()`, which the
    // view joins to every schema's answer by name only.
    let sigs: Vec<_> = functions(view, schema, name)
        .into_iter()
        .filter(|f| &*f.schema == schema)
        .collect();
    assert_eq!(sigs.len(), 1, "{at}: {path} {sigs:?}");
    let got: Vec<(String, String)> = sigs[0]
        .args
        .as_ref()
        .unwrap_or_else(|| panic!("{at}: {path} has no signature"))
        .iter()
        .map(|a| {
            (
                a.name.as_deref().unwrap_or("").to_string(),
                a.type_name.to_string(),
            )
        })
        .collect();
    let want: Vec<(String, String)> = args
        .iter()
        .map(|(n, t)| (n.to_string(), t.to_string()))
        .collect();
    assert_eq!(got, want, "{at}: {path}");
    assert!(sigs[0].returns.is_some(), "{at}: {path}");
    println!("HV-FACT g3 sys-routine target=\"{at}\" object={path} args={got:?}");
}

/// The `sys` routine checked per MySQL/MariaDB target, with its arguments as each snapshot
/// holds them (`sys.format_bytes(bytes text) returns text` in all five).
const SYS_ROUTINE: (&str, &[(&str, &str)]) = ("sys.format_bytes", &[("bytes", "text")]);

#[test]
fn offline_resolution_matrix() {
    for v in ["pg14", "pg15", "pg16", "pg17", "pg18"] {
        let view = offline_view(v);
        assert_found(
            &view,
            "pg_catalog.pg_class",
            KindGroup::Relations,
            &["relname", "relnamespace"],
        );
        assert_found(
            &view,
            "pg_catalog.pg_stat_activity",
            KindGroup::Relations,
            &["pid"],
        );
        assert_found(
            &view,
            "information_schema.columns",
            KindGroup::Relations,
            &["table_name"],
        );
        assert_routine_found(&view, "pg_catalog.now");
    }
    for v in [
        "mysql80",
        "mysql84",
        "mariadb106",
        "mariadb1011",
        "mariadb114",
    ] {
        let view = offline_view(v);
        // ver/01 #mysql-is-times reads this column.
        assert_found(
            &view,
            "information_schema.TABLES",
            KindGroup::Relations,
            &["CREATE_TIME"],
        );
        // Only the complete set (read as `hv_catalog`) holds the `mysql` schema.
        assert_found(&view, "mysql.user", KindGroup::Relations, &[]);
        assert_routine_name_only(&view, "NOW");
        assert_routine_with_signature(&view, SYS_ROUTINE.0, SYS_ROUTINE.1);
    }
}

/// One object per adjacent pair of supported versions, present in the newer snapshot and
/// absent from the older one, taken from a diff of the committed snapshots (the release-notes
/// reference names where the server added it; the report lists the diff).
struct Pair {
    older: &'static str,
    newer: &'static str,
    path: &'static str,
    group: KindGroup,
    notes: &'static str,
}

const PAIRS: &[Pair] = &[
    Pair {
        older: "pg14",
        newer: "pg15",
        path: "pg_catalog.pg_stat_subscription_stats",
        group: KindGroup::Relations,
        notes: "PostgreSQL 15 release notes: add system view pg_stat_subscription_stats",
    },
    Pair {
        older: "pg15",
        newer: "pg16",
        path: "pg_catalog.pg_stat_io",
        group: KindGroup::Relations,
        notes: "PostgreSQL 16 release notes: add system view pg_stat_io",
    },
    Pair {
        older: "pg16",
        newer: "pg17",
        path: "pg_catalog.pg_stat_checkpointer",
        group: KindGroup::Relations,
        notes: "PostgreSQL 17 release notes: create system view pg_stat_checkpointer",
    },
    Pair {
        older: "pg17",
        newer: "pg18",
        path: "pg_catalog.pg_aios",
        group: KindGroup::Relations,
        notes: "PostgreSQL 18 release notes: add system view pg_aios (asynchronous I/O)",
    },
    Pair {
        older: "mysql80",
        newer: "mysql84",
        path: "performance_schema.setup_meters",
        group: KindGroup::Relations,
        notes: "MySQL 8.1.0 release notes: Performance Schema setup_meters (telemetry)",
    },
    Pair {
        older: "mariadb106",
        newer: "mariadb1011",
        path: "natural_sort_key",
        group: KindGroup::Routines,
        notes: "MariaDB 10.7 release notes: NATURAL_SORT_KEY()",
    },
    Pair {
        older: "mariadb1011",
        newer: "mariadb114",
        path: "information_schema.PERIODS",
        group: KindGroup::Relations,
        notes: "MariaDB 11.4 release notes: Information Schema PERIODS (application-time periods)",
    },
];

/// The answer of `pair.path` in `view`, through `ModelView` and `CatalogView`, as one word
/// each; both layers must agree.
fn answer(view: &SnapshotView, pair: &Pair) -> &'static str {
    fn word<T>(l: &Lookup<T>) -> &'static str {
        match l {
            Lookup::Found(_) => "Found",
            Lookup::Missing(_) => "Missing",
            Lookup::Unknown(_) => "Unknown",
        }
    }
    let at = label(view);
    if pair.group == KindGroup::Routines {
        // A MySQL/MariaDB built-in: no schema; asked through any schema's answer.
        let model = word(&view.lookup(&parts(pair.path), KindGroup::Routines));
        let sql = word(&view.functions(
            &key(view, "sys", NameClass::Schema),
            &key(view, pair.path, NameClass::Routine),
        ));
        // `ModelView` sees built-ins only through the layer's own names; `CatalogView` is the
        // completion layer and is the one asserted.
        println!(
            "HV-FACT g3 builtin-lookup target=\"{at}\" name={} model={model}",
            pair.path
        );
        return sql;
    }
    let model = word(&view.lookup(&parts(pair.path), pair.group));
    let (schema, name) = split(pair.path);
    let sql = word(&view.relation(
        &key(view, schema, NameClass::Schema),
        &key(view, name, NameClass::Table),
    ));
    assert_eq!(model, sql, "{at}: {} ModelView vs CatalogView", pair.path);
    sql
}

#[test]
fn version_specific_objects_resolve_only_in_their_version() {
    for pair in PAIRS {
        let set = bundled();
        let (dbms, older_line) = target(pair.older);
        let (newer_dbms, _) = target(pair.newer);
        assert_eq!(dbms, newer_dbms);
        let newer = offline_view(pair.newer);
        let older = offline_view(pair.older);
        assert_eq!(
            answer(&newer, pair),
            "Found",
            "{} in {}",
            pair.path,
            pair.newer
        );
        // The initdb-only fact unknown (an offline cache that never ran the check): a miss
        // in the older version is never a fact.
        assert_eq!(
            answer(&older, pair),
            "Unknown",
            "{} in {}",
            pair.path,
            pair.older
        );
        // Ruling R-W5-6: on PostgreSQL proven to hold nothing beyond initdb, the older
        // snapshot's miss is a fact. MySQL/MariaDB system misses never are.
        let proven = offline_at(
            dbms,
            &set.for_server(dbms, older_line).unwrap().server_version,
            Some(false),
        );
        let want = if dbms == DbmsKind::Postgres {
            "Missing"
        } else {
            "Unknown"
        };
        assert_eq!(answer(&proven, pair), want, "{} proven", pair.older);
        println!(
            "HV-FACT g3 version-pair older={} newer={} object={} newer=Found older=Unknown \
             older_initdb_only={want} source=\"{}\"",
            pair.older, pair.newer, pair.path, pair.notes
        );
    }
}

#[test]
fn unsupported_version_is_not_served_from_another_snapshot() {
    let set = bundled();
    let cases: &[(DbmsKind, &str)] = &[
        (DbmsKind::Postgres, "13.22"),
        (DbmsKind::Postgres, "19.0"),
        (DbmsKind::Postgres, "9.6.24"),
        (DbmsKind::MySql, "5.7.44"),
        (DbmsKind::MySql, "8.1.0"),
        (DbmsKind::MySql, "8.3.0"),
        (DbmsKind::MySql, "9.1.0"),
        (DbmsKind::MariaDb, "10.5.27"),
        (DbmsKind::MariaDb, "10.7.8"),
        (DbmsKind::MariaDb, "11.8.3"),
        // A MariaDB release reported under the MySQL engine, and the reverse: no cross-engine
        // match.
        (DbmsKind::MySql, "10.11.19"),
        (DbmsKind::MariaDb, "8.0.46"),
        (DbmsKind::Sqlite, "3.45.1"),
        (DbmsKind::Postgres, ""),
    ];
    for &(dbms, version) in cases {
        assert!(
            set.for_server(dbms, version).is_none(),
            "{dbms:?} {version}: served from a snapshot"
        );
        let layers = set.overlays_for(dbms, version);
        assert!(layers.system.is_none(), "{dbms:?} {version}");
        assert!(layers.shipped_fks.is_empty(), "{dbms:?} {version}");
        if !matches!(
            dbms,
            DbmsKind::Postgres | DbmsKind::MySql | DbmsKind::MariaDb
        ) {
            continue;
        }
        // Offline, with no snapshot and no live read, nothing resolves and nothing is
        // declared absent: every answer is `Unknown`.
        let view = offline_at(dbms, version, Some(false));
        let (rel, func) = if dbms == DbmsKind::Postgres {
            ("pg_catalog.pg_class", ("pg_catalog", "now"))
        } else {
            ("information_schema.TABLES", ("sys", "NOW"))
        };
        let model = view.lookup(&parts(rel), KindGroup::Relations);
        assert!(
            matches!(model, Lookup::Unknown(_)),
            "{dbms:?} {version}: ModelView {rel}: {model:?}"
        );
        let (schema, name) = split(rel);
        let r = view.relation(
            &key(&view, schema, NameClass::Schema),
            &key(&view, name, NameClass::Table),
        );
        assert!(
            matches!(r, Lookup::Unknown(_)),
            "{dbms:?} {version}: CatalogView {rel}: {r:?}"
        );
        let f = view.functions(
            &key(&view, func.0, NameClass::Schema),
            &key(&view, func.1, NameClass::Routine),
        );
        assert!(
            matches!(f, Lookup::Unknown(_)),
            "{dbms:?} {version}: CatalogView {}.{}: {f:?}",
            func.0,
            func.1
        );
        assert!(matches!(view.settings(), Lookup::Unknown(_)));
        println!(
            "HV-FACT g3 unsupported dbms={dbms:?} version=\"{version}\" layer=none answers=Unknown"
        );
    }
}
