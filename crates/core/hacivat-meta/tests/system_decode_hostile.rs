// ## m4-t23-minicat (fix round 1): the T23 security review's hostile tests, committed as
// written except where fix round 1 changed the rule they probe (m-3: the decoder now
// cross-checks `coverage` against the content, so the fixtures carry a consistent coverage and
// H10 asserts the refusal).
//! M4 Task 23 security review: hostile inputs the implementer's `system_decode.rs` does not
//! cover. Every frame here is a *valid* zstd frame with a correct checksum, so the hostile part
//! reaches the JSON, canonical-form, vocabulary and model layers (the fuzz target's mutations
//! mostly stop at the checksum).

#![cfg(feature = "system")]

use hacivat_meta::ids::NsKey;
use hacivat_meta::system::{SystemError, decode, decode_json, normalize};
use serde_json::{Value, json};

fn frame(json: &[u8]) -> Vec<u8> {
    let mut c = zstd::bulk::Compressor::new(3).unwrap();
    c.include_checksum(true).unwrap();
    c.include_contentsize(true).unwrap();
    c.include_dictid(false).unwrap();
    c.compress(json).unwrap()
}

fn pretty(v: &Value) -> Vec<u8> {
    let mut t = serde_json::to_vec_pretty(v).unwrap();
    t.push(b'\n');
    t
}

fn pg(relations: Value) -> Value {
    json!({
        "format": 1, "dbms": "postgres", "line": "17", "server_version": "17.6",
        "coverage": {"reader": "catalog", "expected_schemas": ["information_schema", "pg_catalog"],
                     "missing_schemas": ["information_schema"], "relations_without_columns": 0},
        "relations": relations
    })
}

fn rel(name: &str) -> Value {
    json!({"schema": "pg_catalog", "name": name, "kind": "table",
           "columns": [{"name": "c", "type": "integer", "nullable": true}]})
}

fn is_err(bytes: &[u8]) -> bool {
    let a = decode(bytes).is_err();
    let b = decode_json(bytes).is_err();
    assert_eq!(a, b, "decode and decode_json disagree");
    a
}

/// A canonical document with one textual change: always refused, never repaired.
fn tampered(doc: &Value, from: &str, to: &str) -> Vec<u8> {
    let text = String::from_utf8(normalize(doc).unwrap()).unwrap();
    assert!(
        text.contains(from),
        "probe text {from:?} not in the canonical form"
    );
    frame(text.replacen(from, to, 1).as_bytes())
}

#[test]
fn h1_nesting_bomb_is_an_error_not_a_stack_overflow() {
    let depth = 200_000;
    let mut s = String::from("{\"format\":1,\"relations\":");
    s.push_str(&"[".repeat(depth));
    s.push_str(&"]".repeat(depth));
    s.push('}');
    assert!(is_err(&frame(s.as_bytes())));
    let mut o = String::new();
    for _ in 0..depth {
        o.push_str("{\"a\":");
    }
    o.push('1');
    o.push_str(&"}".repeat(depth));
    assert!(is_err(&frame(o.as_bytes())));
}

#[test]
fn h2_duplicate_json_keys_are_refused() {
    let doc = pg(json!([rel("pg_class")]));
    // Top level: a second `format` (serde_json keeps the last; the bytes are not canonical).
    assert!(is_err(&tampered(
        &doc,
        "\"format\": 1,",
        "\"format\": 1,\n  \"format\": 1,"
    )));
    // Inside a relation: two `name`s; the second differs.
    assert!(is_err(&tampered(
        &doc,
        "\"name\": \"pg_class\",",
        "\"name\": \"pg_class\",\n      \"name\": \"pg_other\","
    )));
    // A second `relations` array that would replace the first.
    let text = String::from_utf8(normalize(&doc).unwrap()).unwrap();
    let evil = text.replacen("\n}\n", ",\n  \"relations\": []\n}\n", 1);
    assert!(is_err(&frame(evil.as_bytes())));
}

#[test]
fn h3_duplicate_keyed_items_are_refused_even_when_rows_differ() {
    // Same (schema, name), other kind and columns.
    let mut b = rel("pg_class");
    b["kind"] = json!("view");
    let doc = pg(json!([rel("pg_class"), b]));
    assert!(normalize(&doc).is_err());
    // Two columns of one relation with the same name at other positions.
    let mut r = rel("t");
    r["columns"] = json!([
        {"name": "c", "type": "integer", "nullable": true},
        {"name": "d", "type": "text", "nullable": true},
        {"name": "c", "type": "text", "nullable": false}
    ]);
    assert!(normalize(&pg(json!([r]))).is_err());
    // Two operators with the same key and other descriptions.
    let mut d = pg(json!([rel("pg_class")]));
    d["operators"] = json!([
        {"schema": "pg_catalog", "name": "+", "kind": "infix", "left": "integer", "right": "integer", "result": "integer", "function": "a"},
        {"schema": "pg_catalog", "name": "+", "kind": "infix", "left": "integer", "right": "integer", "result": "bigint", "function": "b"}
    ]);
    assert!(normalize(&d).is_err());
    // Two variables differing only by context.
    let mut v = pg(json!([rel("pg_class")]));
    v["variables"] = json!([{"name": "a", "context": "user"}, {"name": "a", "context": "sighup"}]);
    assert!(normalize(&v).is_err());
}

#[test]
fn h4_non_canonical_spellings_are_refused_never_repaired() {
    let doc = pg(json!([rel("pg_class")]));
    // An escaped spelling of the same string.
    assert!(is_err(&tampered(
        &doc,
        "\"line\": \"17\"",
        "\"line\": \"\\u0031\\u0037\""
    )));
    // Other number spellings of 1.
    for one in ["1.0", "1e0", "01", "-0"] {
        assert!(
            is_err(&tampered(
                &doc,
                "\"format\": 1,",
                &format!("\"format\": {one},")
            )),
            "{one}"
        );
    }
    // A field the canonical form omits, spelled out at its default.
    let mut r = pg(json!([rel("pg_class")]));
    r["routines"] = json!([{"schema": "pg_catalog", "name": "f", "kind": "function", "args": ""}]);
    assert!(is_err(&tampered(
        &r,
        "\"args\": \"\"",
        "\"args\": \"\",\n      \"set_returning\": false"
    )));
    assert!(is_err(&tampered(
        &doc,
        "\"relations\": [",
        "\"operators\": [],\n  \"relations\": ["
    )));
    // CRLF, a BOM, a missing final newline, trailing spaces.
    let text = String::from_utf8(normalize(&doc).unwrap()).unwrap();
    assert!(is_err(&frame(text.replace('\n', "\r\n").as_bytes())));
    assert!(is_err(&frame(format!("\u{feff}{text}").as_bytes())));
    assert!(is_err(&frame(text.trim_end().as_bytes())));
    assert!(is_err(&frame(format!("{text} ").as_bytes())));
    // Out-of-range and negative counts.
    for n in ["-1", "4294967296", "1.5"] {
        assert!(
            is_err(&tampered(
                &doc,
                "\"relations_without_columns\": 0",
                &format!("\"relations_without_columns\": {n}")
            )),
            "{n}"
        );
    }
    // The canonical form itself still decodes (the probes above are the only change).
    assert!(decode(&frame(&normalize(&doc).unwrap())).is_ok());
}

#[test]
fn h5_byte_order_sort_and_odd_names_round_trip() {
    // Byte order: "Z" < "a" < "é" < "😀"; control characters, quotes and backslashes survive.
    let names = [
        "😀",
        "é",
        "a",
        "Z",
        "q\"uote",
        "back\\slash",
        "nul\u{0}",
        "tab\t",
        "\u{1f}",
    ];
    let doc = pg(Value::Array(names.iter().map(|n| rel(n)).collect()));
    let json = normalize(&doc).unwrap();
    let snap = decode(&frame(&json)).expect("odd but valid names decode");
    assert_eq!(decode_json(&frame(&json)).unwrap(), json);
    let ns = &snap.namespaces[&NsKey::schema(["pg_catalog"])];
    let got: Vec<String> = ns
        .objs()
        .iter()
        .skip(1)
        .filter(|o| o.kind() == hacivat_meta::kind::ObjectKind::Table)
        .map(|o| o.name().to_string())
        .collect();
    let mut want: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    want.sort(); // String's Ord is byte order
    assert_eq!(got, want);
    // Optional keys: `None` sorts before `Some` (as in xtask's canonicalize).
    let mut d = pg(json!([rel("pg_class")]));
    d["operators"] = json!([
        {"schema": "pg_catalog", "name": "-", "kind": "infix", "left": "integer", "right": "integer", "result": "integer", "function": "int4mi"},
        {"schema": "pg_catalog", "name": "-", "kind": "prefix", "right": "integer", "result": "integer", "function": "int4um"}
    ]);
    let t = String::from_utf8(normalize(&d).unwrap()).unwrap();
    assert!(t.find("int4um").unwrap() < t.find("int4mi").unwrap());
}

#[test]
fn h6_hostile_argument_texts_never_panic() {
    let bombs = [
        "(".repeat(100_000),
        ")".repeat(3),
        "é,é,,é".to_string(),
        "x \"a,b\" DEFAULT".to_string(),
        "x text DEFAULT 'é DEFAULT '".to_string(),
        "'".repeat(7),
        "VARIADIC ".to_string(),
        "IN ".to_string(),
        "a\u{0}b integer".to_string(),
        "😀 integer, 😀😀 integer[]".to_string(),
        ",".repeat(1000),
        "[".repeat(50_000) + &"]".repeat(50_000),
    ];
    for (i, a) in bombs.iter().enumerate() {
        for dbms in ["postgres", "mysql"] {
            let mut d = pg(json!([rel("pg_class")]));
            let schema = if dbms == "postgres" {
                "pg_catalog"
            } else {
                "sys"
            };
            if dbms == "mysql" {
                d["dbms"] = json!("mysql");
                d["line"] = json!("8.4");
                d["server_version"] = json!("8.4.6");
                d["coverage"]["expected_schemas"] = json!(["sys"]);
                d["coverage"]["missing_schemas"] = json!([]);
                d["relations"] = json!([{"schema": "sys", "name": "t", "kind": "table",
                    "columns": [{"name": "c", "type": "int", "nullable": true}]}]);
            }
            d["routines"] = json!([{"schema": schema, "name": format!("f{i}"), "kind": "function",
                "args": a, "identity": a, "result": "integer"}]);
            d["types"] = if dbms == "postgres" {
                json!([{"schema": "pg_catalog", "name": "int4", "sql_name": "integer", "kind": "base", "category": "N"}])
            } else {
                json!([])
            };
            let json = normalize(&d).unwrap();
            // Decodes (args unknown or parsed) or refuses; never panics.
            let _ = decode(&frame(&json));
        }
    }
}

#[test]
fn h7_model_kind_schema_refusals_are_errors() {
    // MySQL has no aggregates or windows as schema routines; PostgreSQL schema-less routines
    // are refused by the vocabulary.
    for kind in ["aggregate", "window", "procedure"] {
        let d = json!({
            "format": 1, "dbms": "mysql", "line": "8.0", "server_version": "8.0.44",
            "coverage": {"reader": "catalog", "expected_schemas": ["sys"], "missing_schemas": ["sys"],
                         "relations_without_columns": 0, "builtins": "probe"},
            "relations": [],
            "routines": [{"schema": "sys", "name": "x", "kind": kind, "args": ""}]
        });
        let json = normalize(&d).unwrap();
        // Ok or a typed error; never a panic.
        match decode(&frame(&json)) {
            Ok(_) | Err(SystemError::Corrupt(_)) => {}
            Err(e) => panic!("{kind}: {e:?}"),
        }
    }
    let mut p = pg(json!([rel("pg_class")]));
    p["routines"] = json!([{"name": "x", "kind": "function"}]);
    assert!(normalize(&p).is_err(), "schema-less PostgreSQL routine");
    // MySQL operators/types are refused.
    let mut m = pg(json!([]));
    m["dbms"] = json!("mysql");
    m["coverage"]["expected_schemas"] = json!(["sys"]);
    m["types"] =
        json!([{"schema": "sys", "name": "t", "sql_name": "t", "kind": "base", "category": "N"}]);
    assert!(normalize(&m).is_err());
    // A dbms the vocabulary does not know, a schema of another engine.
    let mut s = pg(json!([rel("pg_class")]));
    s["dbms"] = json!("sqlite");
    assert!(normalize(&s).is_err());
    let mut s = pg(json!([rel("pg_class")]));
    s["relations"][0]["schema"] = json!("mysql");
    assert!(normalize(&s).is_err());
}

/// Rewrites the Frame_Content_Size field of a single-segment frame header to `size`.
fn with_declared_size(frame: &[u8], size: u64) -> Vec<u8> {
    let fhd = frame[4];
    let fcs_flag = fhd >> 6;
    let single = fhd & 0x20 != 0;
    let did = match fhd & 3 {
        0 => 0,
        1 => 1,
        2 => 2,
        _ => 4,
    };
    let fcs_len = match fcs_flag {
        0 => usize::from(single),
        1 => 2,
        2 => 4,
        _ => 8,
    };
    let at = 5 + usize::from(!single) + did;
    // Rebuild the header with an 8-byte FCS field (flag 3) holding `size`.
    let mut out = frame[..4].to_vec();
    out.push((fhd & 0x3f) | 0xc0);
    out.extend_from_slice(&frame[5..at]);
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&frame[at + fcs_len..]);
    out
}

#[test]
fn h8_zstd_layer_lies_are_refused() {
    let json = normalize(&pg(json!([rel("pg_class")]))).unwrap();
    let good = frame(&json);
    assert!(decode(&good).is_ok());
    // The same frame, header rewritten to declare its true size (sanity of the rewrite).
    assert!(decode(&with_declared_size(&good, json.len() as u64)).is_ok());
    // Declares more or less than it holds: the bound allocation happens, then a refusal.
    for size in [
        json.len() as u64 + 1,
        json.len() as u64 - 1,
        8 * 1024 * 1024,
        0,
    ] {
        assert!(is_err(&with_declared_size(&good, size)), "declared {size}");
    }
    // Declares more than the bound, up to u64::MAX: refused before allocating.
    for size in [
        8 * 1024 * 1024 + 1,
        u64::from(u32::MAX),
        u64::MAX - 1,
        u64::MAX,
    ] {
        assert!(is_err(&with_declared_size(&good, size)), "declared {size}");
    }
    // A skippable frame before or after the snapshot.
    let mut skip = vec![0x50, 0x2a, 0x4d, 0x18, 4, 0, 0, 0, 1, 2, 3, 4];
    let mut after = good.clone();
    after.extend_from_slice(&skip);
    assert!(is_err(&after));
    skip.extend_from_slice(&good);
    assert!(is_err(&skip));
    // Two frames whose concatenated content would be the canonical JSON.
    let (a, b) = json.split_at(json.len() / 2);
    let mut two = frame(a);
    two.extend_from_slice(&frame(b));
    assert!(is_err(&two));
    // A raw (uncompressed) block frame of the same JSON is accepted: the decoder checks the
    // content, not the encoder's choices (the digests in the manifest pin the exact bytes).
    let mut c = zstd::bulk::Compressor::new(-5).unwrap();
    c.include_checksum(true).unwrap();
    c.include_contentsize(true).unwrap();
    assert_eq!(decode_json(&c.compress(&json).unwrap()).unwrap(), json);
}

#[test]
fn h9_near_bound_snapshot_decodes_and_over_bound_is_refused() {
    // ~45k relations: close to the 8 MiB canonical bound, every name unique and sorted.
    let one = normalize(&pg(json!([rel("r00000000")]))).unwrap().len();
    let two = normalize(&pg(json!([rel("r00000000"), rel("r00000001")])))
        .unwrap()
        .len();
    let per = two - one;
    let n = (7 * 1024 * 1024 + 512 * 1024) / per;
    let mut rels: Vec<Value> = (0..n).map(|i| rel(&format!("r{i:08}"))).collect();
    let doc = pg(Value::Array(rels.clone()));
    let json = normalize(&doc).unwrap();
    assert!(json.len() > 7 * 1024 * 1024, "{} bytes", json.len());
    let snap = decode(&frame(&json)).expect("a large but valid snapshot");
    assert_eq!(
        snap.namespaces[&NsKey::schema(["pg_catalog"])].objs().len(),
        1 + 2 * rels.len()
    );
    // Grow past the bound: normalize refuses, and a frame of the over-bound text is refused.
    for i in n..n + 20_000 {
        rels.push(rel(&format!("r{i:08}")));
    }
    let big = pg(Value::Array(rels));
    assert!(normalize(&big).is_err());
    assert!(is_err(&frame(&pretty(&big))));
}

#[test]
fn h10_coverage_is_cross_checked_by_the_decoder() {
    // Review finding m-3, fixed in round 1: a file whose coverage contradicts its content (a
    // relation without columns counted as none, a schema absent but not listed missing) is
    // refused.
    let mut r = rel("pg_class");
    r["columns"] = json!([]);
    let mut d = pg(json!([r]));
    d["coverage"]["expected_schemas"] = json!(["pg_catalog"]);
    d["coverage"]["missing_schemas"] = json!([]);
    assert!(matches!(normalize(&d), Err(SystemError::Corrupt(_))));
    assert!(is_err(&frame(&pretty(&d))));
    d["coverage"]["relations_without_columns"] = json!(1);
    assert!(
        normalize(&d).is_ok(),
        "consistent: one relation without columns"
    );
    let mut m = pg(json!([rel("pg_class")]));
    m["coverage"]["missing_schemas"] = json!([]);
    assert!(
        matches!(normalize(&m), Err(SystemError::Corrupt(_))),
        "information_schema absent"
    );
}
