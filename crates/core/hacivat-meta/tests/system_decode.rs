//! M4 Task 23: decoding the shipped system-catalog snapshots (`hacivat_meta::system`,
//! R-M4-17, R-M4-33). A snapshot file is untrusted bytes at rest (Review Focus 4): decoding
//! never panics, refuses anything but one checksummed zstd frame of canonical JSON format 1,
//! and maps it to one namespace per system schema with every object flagged
//! `PRE_INTROSPECTED`.

#![cfg(feature = "system")]

use std::path::PathBuf;

use hacivat_meta::ids::NsKey;
use hacivat_meta::kind::ObjectKind;
use hacivat_meta::namespace::ObjFlags;
use hacivat_meta::props::Props;
use hacivat_meta::system::{SystemError, decode, decode_json, normalize};
use hacivat_proto::DbmsKind;
use serde_json::{Value, json};
use sql_catalog::ArgMode;

/// A small PostgreSQL snapshot in canonical order.
fn pg_doc() -> Value {
    json!({
        "format": 1,
        "dbms": "postgres",
        "line": "17",
        "server_version": "17.6",
        "coverage": {
            "reader": "catalog",
            "expected_schemas": ["information_schema", "pg_catalog"],
            "missing_schemas": [],
            "relations_without_columns": 0
        },
        "relations": [
            {"schema": "information_schema", "name": "tables", "kind": "view", "columns": [
                {"name": "table_catalog", "type": "information_schema.sql_identifier", "nullable": true}
            ]},
            {"schema": "pg_catalog", "name": "pg_class", "kind": "table", "columns": [
                {"name": "oid", "type": "oid", "nullable": false},
                {"name": "relname", "type": "name", "nullable": false}
            ]},
            {"schema": "pg_catalog", "name": "pg_stat_activity", "kind": "view", "columns": [
                {"name": "datid", "type": "oid", "nullable": true},
                {"name": "pid", "type": "integer", "nullable": true}
            ]}
        ],
        "routines": [
            {"schema": "pg_catalog", "name": "count", "kind": "aggregate", "args": "", "result": "bigint", "volatility": "immutable"},
            {"schema": "pg_catalog", "name": "lower", "kind": "function", "args": "text", "result": "text", "volatility": "immutable", "description": "lowercase"},
            {"schema": "pg_catalog", "name": "make_interval", "kind": "function",
             "args": "years integer DEFAULT 0, secs double precision DEFAULT 0.0",
             "identity": "years integer, secs double precision",
             "result": "interval", "volatility": "immutable"},
            {"schema": "pg_catalog", "name": "now", "kind": "function", "args": "", "result": "timestamp with time zone", "volatility": "stable"}
        ],
        "operators": [
            {"schema": "pg_catalog", "name": "+", "kind": "infix", "left": "integer", "right": "integer", "result": "integer", "function": "int4pl", "description": "add"}
        ],
        "types": [
            {"schema": "pg_catalog", "name": "float8", "sql_name": "double precision", "kind": "base", "category": "N"},
            {"schema": "pg_catalog", "name": "int4", "sql_name": "integer", "kind": "base", "category": "N"},
            {"schema": "pg_catalog", "name": "interval", "sql_name": "interval", "kind": "base", "category": "T"},
            {"schema": "pg_catalog", "name": "text", "sql_name": "text", "kind": "base", "category": "S"},
            {"schema": "pg_catalog", "name": "timestamptz", "sql_name": "timestamp with time zone", "kind": "base", "category": "D"}
        ],
        "variables": [
            {"name": "search_path", "type": "string", "context": "user"},
            {"name": "work_mem", "type": "integer", "context": "user"}
        ]
    })
}

/// A small MySQL snapshot: a `sys` function with arguments and two probed built-in names.
fn my_doc() -> Value {
    json!({
        "format": 1,
        "dbms": "mysql",
        "line": "8.4",
        "server_version": "8.4.6",
        "coverage": {
            "reader": "catalog",
            "expected_schemas": ["information_schema", "mysql", "performance_schema", "sys"],
            "missing_schemas": [],
            "relations_without_columns": 0,
            "builtins": "probe"
        },
        "relations": [
            {"schema": "information_schema", "name": "TABLES", "kind": "system_view", "columns": [
                {"name": "TABLE_CATALOG", "type": "varchar(64)", "nullable": false}
            ]},
            {"schema": "mysql", "name": "user", "kind": "table", "columns": [
                {"name": "Host", "type": "char(255)", "nullable": false}
            ]},
            {"schema": "performance_schema", "name": "threads", "kind": "table", "columns": [
                {"name": "THREAD_ID", "type": "bigint unsigned", "nullable": false}
            ]},
            {"schema": "sys", "name": "sys_config", "kind": "table", "columns": [
                {"name": "variable", "type": "varchar(128)", "nullable": false}
            ]}
        ],
        "routines": [
            {"name": "abs", "kind": "function"},
            {"name": "concat", "kind": "function"},
            {"schema": "sys", "name": "format_path", "kind": "function", "args": "in_path varchar(512)", "result": "varchar(512)"},
            {"schema": "sys", "name": "ps_setup_show_enabled", "kind": "procedure", "args": "IN in_show_instruments tinyint(1), IN in_kind enum('a','b,c')"}
        ],
        "variables": [
            {"name": "autocommit"},
            {"name": "sql_mode"}
        ]
    })
}

/// The canonical bytes of `doc`.
fn canonical(doc: &Value) -> Vec<u8> {
    normalize(doc).expect("a valid format-1 document")
}

/// A snapshot file: one zstd frame, level 19, checksum on (the generator's settings).
fn frame(json: &[u8]) -> Vec<u8> {
    let mut c = zstd::bulk::Compressor::new(19).unwrap();
    c.include_checksum(true).unwrap();
    c.include_contentsize(true).unwrap();
    c.include_dictid(false).unwrap();
    c.compress(json).unwrap()
}

fn assets_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../assets/system-catalogs")
}

/// The committed snapshot files (none until the first CI-generated set is committed).
fn committed() -> Vec<(String, Vec<u8>)> {
    let Ok(entries) = std::fs::read_dir(assets_dir()) else {
        return Vec::new();
    };
    let mut out: Vec<(String, Vec<u8>)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".hvsnap.zst"))
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read(&p).unwrap(),
            )
        })
        .collect();
    out.sort();
    out
}

#[test]
fn decode_of_truncated_asset_is_error_not_panic() {
    let json = canonical(&pg_doc());
    let file = frame(&json);
    assert!(decode(&file).is_ok());
    // Every truncation, including the empty file.
    for n in 0..file.len() {
        assert!(decode(&file[..n]).is_err(), "a {n}-byte prefix decoded");
    }
    // One flipped bit anywhere: refused, or (a bit libzstd does not interpret) the very same
    // canonical bytes. Never another snapshot, never a panic.
    let mut refused = 0;
    for i in 0..file.len() {
        for bit in 0..8 {
            let mut bad = file.clone();
            bad[i] ^= 1 << bit;
            match decode_json(&bad) {
                Err(_) => refused += 1,
                Ok(got) => assert_eq!(got, json, "a flipped bit {bit} at {i} changed the JSON"),
            }
        }
    }
    assert!(
        refused * 10 >= file.len() * 8 * 9,
        "only {refused} of {} flips refused",
        file.len() * 8
    );
    // Trailing bytes and a second frame are not one snapshot.
    let mut trailing = file.clone();
    trailing.push(0);
    assert!(decode(&trailing).is_err());
    let mut two = file.clone();
    two.extend_from_slice(&file);
    assert!(decode(&two).is_err());
    // No checksum: refused even though the content is canonical.
    let mut c = zstd::bulk::Compressor::new(19).unwrap();
    c.include_checksum(false).unwrap();
    let unchecked = c.compress(&json).unwrap();
    assert!(matches!(decode(&unchecked), Err(SystemError::Corrupt(_))));
    // A frame that declares more content than a snapshot may hold is refused before any
    // allocation of that size (the declared size is never trusted).
    let big = vec![b' '; 9 * 1024 * 1024];
    assert!(matches!(decode(&frame(&big)), Err(SystemError::Corrupt(_))));
    // Not canonical (other whitespace, other order): refused.
    let loose = serde_json::to_vec(&pg_doc()).unwrap();
    assert!(matches!(
        decode(&frame(&loose)),
        Err(SystemError::Corrupt(_))
    ));
    let mut unsorted = pg_doc();
    unsorted["relations"].as_array_mut().unwrap().reverse();
    let mut text = serde_json::to_vec_pretty(&unsorted).unwrap();
    text.push(b'\n');
    assert!(matches!(
        decode(&frame(&text)),
        Err(SystemError::Corrupt(_))
    ));
    // Another format number.
    let mut v2 = pg_doc();
    v2["format"] = json!(2);
    let mut text = serde_json::to_vec_pretty(&v2).unwrap();
    text.push(b'\n');
    assert!(matches!(
        decode(&frame(&text)),
        Err(SystemError::UnknownFormat(2))
    ));
    // Garbage and random-looking input.
    assert!(decode(b"not a snapshot").is_err());
    assert!(decode(&[0x28, 0xb5, 0x2f, 0xfd]).is_err());
    // Every committed asset, truncated by one byte, is an error too.
    for (name, bytes) in committed() {
        assert!(decode(&bytes[..bytes.len() - 1]).is_err(), "{name}");
    }
}

#[test]
fn decoded_objects_are_pre_introspected_and_probe_routines_are_name_only() {
    let my = decode(&frame(&canonical(&my_doc()))).unwrap();
    assert_eq!(my.dbms, DbmsKind::MySql);
    assert_eq!(&*my.line, "8.4");
    assert_eq!(&*my.server_version, "8.4.6");
    assert_eq!(my.format, 1);
    let keys: Vec<NsKey> = my.namespaces.keys().cloned().collect();
    assert_eq!(
        keys,
        ["information_schema", "mysql", "performance_schema", "sys"]
            .map(|s| NsKey::schema([s]))
            .to_vec()
    );
    for ns in my.namespaces.values() {
        for o in ns.objs() {
            assert!(
                o.flags()
                    .contains(ObjFlags::PRE_INTROSPECTED | ObjFlags::SYSTEM),
                "{} {:?}",
                o.name(),
                o.flags()
            );
        }
    }
    // The probed built-ins: schema-less, name only (`args = None` is "not known", never "no
    // arguments").
    let names: Vec<&str> = my.builtins.iter().map(|o| &**o.name()).collect();
    assert_eq!(names, ["abs", "concat"]);
    for o in &my.builtins {
        assert_eq!(o.kind(), ObjectKind::Routine);
        assert!(o.flags().contains(ObjFlags::PRE_INTROSPECTED));
        let Props::Routine(r) = o.props() else {
            panic!("{}: {:?}", o.name(), o.props())
        };
        assert!(r.args.is_none(), "{}", o.name());
    }
    // A `sys` routine carries its arguments, quoted commas included.
    let sys = &my.namespaces[&NsKey::schema(["sys"])];
    let proc_idx = sys
        .lookup_kind(ObjectKind::Procedure, "ps_setup_show_enabled")
        .next()
        .expect("the procedure");
    let Props::Routine(p) = sys.obj(proc_idx).unwrap().props() else {
        panic!("not a routine")
    };
    let args = p.args.as_ref().expect("sys routines carry arguments");
    assert_eq!(args.len(), 2);
    assert_eq!(args[0].name.as_deref(), Some("in_show_instruments"));
    assert_eq!(&*args[0].type_name, "tinyint(1)");
    assert_eq!(args[0].mode, ArgMode::In);
    assert_eq!(&*args[1].type_name, "enum('a','b,c')");
    let f_idx = sys
        .lookup_kind(ObjectKind::Routine, "format_path")
        .next()
        .expect("the function");
    let Props::Routine(f) = sys.obj(f_idx).unwrap().props() else {
        panic!("not a routine")
    };
    assert_eq!(f.returns.as_deref(), Some("varchar(512)"));
    assert_eq!(f.args.as_ref().unwrap()[0].name.as_deref(), Some("in_path"));
    // Variable names in the side table, never values.
    let vars: Vec<&str> = my.settings.iter().map(|s| &*s.name).collect();
    assert_eq!(vars, ["autocommit", "sql_mode"]);

    let pg = decode(&frame(&canonical(&pg_doc()))).unwrap();
    assert_eq!(pg.dbms, DbmsKind::Postgres);
    assert!(pg.builtins.is_empty());
    let cat = &pg.namespaces[&NsKey::schema(["pg_catalog"])];
    let act = cat
        .lookup_kind(ObjectKind::View, "pg_stat_activity")
        .next()
        .expect("pg_stat_activity");
    let cols: Vec<&str> = cat.children(act).map(|(_, o)| &**o.name()).collect();
    assert_eq!(cols, ["datid", "pid"]);
    for o in cat.objs() {
        assert!(
            o.flags().contains(ObjFlags::PRE_INTROSPECTED),
            "{}",
            o.name()
        );
    }
    // PostgreSQL arguments: names, multi-word types and defaults.
    let mi = cat
        .lookup_kind(ObjectKind::Routine, "make_interval")
        .next()
        .unwrap();
    let Props::Routine(r) = cat.obj(mi).unwrap().props() else {
        panic!("not a routine")
    };
    let args = r.args.as_ref().unwrap();
    assert_eq!(args.len(), 2);
    assert_eq!(args[1].name.as_deref(), Some("secs"));
    assert_eq!(&*args[1].type_name, "double precision");
    assert!(args[1].has_default);
    assert_eq!(
        r.overload_sig.as_deref(),
        Some("years integer, secs double precision")
    );
    let now = cat.lookup_kind(ObjectKind::Routine, "now").next().unwrap();
    let Props::Routine(r) = cat.obj(now).unwrap().props() else {
        panic!("not a routine")
    };
    assert_eq!(r.args.as_deref(), Some(&[][..]), "no arguments, known");
    assert!(
        cat.lookup_kind(ObjectKind::Aggregate, "count")
            .next()
            .is_some()
    );
    // Operators, types and settings in their side tables.
    assert_eq!(pg.operators.len(), 1);
    assert_eq!(&*pg.operators[0].function, "int4pl");
    assert!(pg.types.iter().any(|t| &*t.sql_name == "double precision"));
    assert_eq!(pg.settings.len(), 2);
    assert_eq!(pg.settings[0].context.as_deref(), Some("user"));
}

#[test]
fn decode_round_trips_the_canonical_json() {
    for doc in [pg_doc(), my_doc()] {
        let json = canonical(&doc);
        assert_eq!(json.last(), Some(&b'\n'), "a final newline");
        assert!(!json.contains(&b'\r'));
        let back = decode_json(&frame(&json)).unwrap();
        assert_eq!(back, json, "the payload is the canonical JSON itself");
        // Re-normalizing the decoded JSON gives the same bytes (idempotent).
        let again: Value = serde_json::from_slice(&back).unwrap();
        assert_eq!(normalize(&again).unwrap(), json);
        // Any row order normalizes to the same bytes.
        let mut shuffled = doc.clone();
        for key in ["relations", "routines", "operators", "types", "variables"] {
            if let Some(list) = shuffled.get_mut(key).and_then(Value::as_array_mut) {
                list.reverse();
            }
        }
        assert_eq!(normalize(&shuffled).unwrap(), json);
        assert!(decode(&frame(&json)).is_ok());
    }
    // Unknown fields, a duplicate key and an unknown kind fail closed.
    let mut extra = pg_doc();
    extra["relations"][0]["oid"] = json!(12345);
    assert!(normalize(&extra).is_err());
    let mut dup = pg_doc();
    let first = dup["relations"][0].clone();
    dup["relations"].as_array_mut().unwrap().push(first);
    assert!(normalize(&dup).is_err());
    let mut kind = pg_doc();
    kind["relations"][0]["kind"] = json!("x");
    assert!(normalize(&kind).is_err());
    let mut schema = my_doc();
    schema["relations"][0]["schema"] = json!("hv_catalog_probe");
    assert!(normalize(&schema).is_err(), "only the system schemas");
    // Every committed asset is canonical by this crate's own rules (the generator in xtask
    // and this decoder agree byte for byte).
    for (name, bytes) in committed() {
        let json = decode_json(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let v: Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(normalize(&v).unwrap(), json, "{name}");
        let snap = decode(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!snap.namespaces.is_empty(), "{name}");
    }
}
