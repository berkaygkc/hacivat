//! Fuzz target `catalog_row_decode` (M4 Task 11): an untrusted cache row never panics
//! (HV-MODEL-03; Review Focus 4).
//!
//! - **Seeded without a corpus** (fix round 1, A-m7): the target builds valid payloads once
//!   (a PostgreSQL schema with tables, columns, keys, a foreign key, a view and an unknown
//!   kind; a MySQL schema) from the public model API. The first input byte picks the mode:
//!   - the raw input, decoded as a whole payload;
//!   - the input framed behind a valid header and recomputed checksum (this minor, or a newer
//!     one), so the body decoder sees arbitrary bytes;
//!   - a valid payload with the input XOR-spliced into its body at an offset the input picks,
//!     checksum recomputed (this minor, or stamped as a newer one): deep paths from the start.
//! - Oracle: whatever decodes re-encodes and decodes to the same namespace, and fingerprints
//!   identically.
//! - The namespace-key and watermark decoders (the cache's key and compare-and-set columns),
//!   padding removal and `RowCipher::open` (an encrypted row) take the raw input too; opening
//!   arbitrary bytes is always an error.

#![no_main]

use std::sync::{Arc, OnceLock};

use hacivat_meta::codec::{
    DecodeError, FORMAT_MAJOR, FORMAT_MINOR, HEADER_LEN, decode, decode_namespace, decode_ns_key,
    decode_watermark, encode_namespace, payload_version, reseal_checksum,
};
use hacivat_meta::fingerprint::subtree;
use hacivat_meta::ids::{NsKey, ObjId, Ref, ServerId, SourceRef, StateMark};
use hacivat_meta::kind::ObjectKind;
use hacivat_meta::name::{FoldRule, Name};
use hacivat_meta::namespace::{
    Level, NamespaceSnapshot, NsMeta, Obj, ObjFlags, PgIdentity, Subtree, Watermark,
};
use hacivat_meta::persist::crypt::unpad;
use hacivat_meta::persist::{CATALOG_DOMAIN, RowCipher, RowKey};
use hacivat_meta::props::{
    ColumnExt, ColumnProps, ColumnRare, FkAction, FkMatch, FkProps, GenericProps, Identity,
    IndexProps, KeyColumn, KeyProps, PgTableExt, Props, TableExt, TableProps, ViewProps,
};
use libfuzzer_sys::fuzz_target;

/// The server namespace (an empty root) of the engine with machine id `dbms`, decoded from a
/// hand-written payload body. The fuzz crate cannot name `hacivat_proto::DbmsKind` (it has no
/// such dependency, and its manifest is not this task's), so the seed builders take the
/// dialect from this namespace's `dbms()`.
fn root_namespace(dbms: &str) -> NamespaceSnapshot {
    let mut body = vec![3, 0, dbms.len() as u8];
    body.extend_from_slice(dbms.as_bytes());
    body.push(4);
    body.extend_from_slice(b"root");
    // Header record: server kind, empty path, the dbms (string 1), exact fold, level names,
    // no watermark, loaded at 0, one kind (code 3, `root` = string 2).
    body.extend_from_slice(&[10, 0, 0, 1, 0, 0, 0, 0, 1, 3, 2]);
    // One object: the root, named "" (string 0), no parent, no flags, ids, state or props.
    body.extend_from_slice(&[1, 7, 3, 0, 0, 0, 0, 0, 0]);
    decode_namespace(&framed(FORMAT_MINOR, &body)).expect("the hand-written root decodes")
}

fn column(name: &str, ordinal: u32, ty: &str, rare: bool) -> Obj {
    Obj::new(ObjectKind::Column, name).with_props(Props::Column(Arc::new(ColumnProps {
        ordinal,
        raw_type: Name::from(ty),
        nullable: ordinal != 1,
        default_raw: (ordinal == 2).then(|| Name::from("now()")),
        comment: rare.then(|| Name::from("İşlem notu")),
        rare: rare.then(|| {
            Arc::new(ColumnRare {
                identity: Some(Identity::ByDefault),
                collation: Some(Name::from("tr-TR-x-icu")),
                ..Default::default()
            })
        }),
        ext: rare.then(|| {
            Arc::new(ColumnExt::Pg {
                storage: Some(b'x'),
                compression: None,
            })
        }),
        ..Default::default()
    })))
}

fn pg_schema() -> NamespaceSnapshot {
    let mut t = Subtree::new(Obj::new(ObjectKind::Schema, "public"));
    let root = t.root();
    for (i, name) in ["siparis", "musteri", "ürün"].iter().enumerate() {
        let oid = 16_400 + i as u32 * 10;
        let tab = t.add(
            root,
            Obj::new(ObjectKind::Table, *name)
                .with_server_id(ServerId::Oid(oid))
                .with_state(StateMark::Xmin(oid))
                .with_props(Props::Table(Arc::new(TableProps {
                    owner: Some(Name::from("app_owner")),
                    ext: TableExt::Pg(PgTableExt {
                        relkind: b'r',
                        options: vec![Name::from("fillfactor=90")],
                        ..Default::default()
                    }),
                    ..Default::default()
                }))),
        );
        t.add(tab, column("id", 1, "bigint", false));
        t.add(tab, column("created_at", 2, "timestamp with time zone", i == 1));
        t.add(tab, column("ılık", 3, "text", i == 2));
        t.add(
            tab,
            Obj::new(ObjectKind::PrimaryKey, format!("{name}_pkey").as_str())
                .add_flags(ObjFlags::SERVER_GENERATED_NAME)
                .with_props(Props::Key(Arc::new(KeyProps {
                    columns: vec![KeyColumn {
                        ordinal: 1,
                        name: Some(Name::from("id")),
                    }],
                    desc: vec![false],
                    unique: true,
                    enforced: true,
                    ..Default::default()
                }))),
        );
        t.add(
            tab,
            Obj::new(ObjectKind::Index, format!("{name}_created_idx").as_str()).with_props(
                Props::Index(Arc::new(IndexProps {
                    method: Some(Name::from("btree")),
                    prefix_len: vec![None],
                    ..Default::default()
                })),
            ),
        );
        if i > 0 {
            t.add(
                tab,
                Obj::new(ObjectKind::ForeignKey, format!("{name}_fk").as_str()).with_props(
                    Props::ForeignKey(Arc::new(FkProps {
                        columns: vec![Name::from("id")],
                        target: Ref {
                            kind: ObjectKind::Table,
                            path: ["app", "public", "siparis"]
                                .into_iter()
                                .map(Name::from)
                                .collect(),
                            server_id: Some(ServerId::Oid(16_400)),
                        },
                        target_columns: vec![Name::from("id")],
                        on_update: FkAction::NoAction,
                        on_delete: FkAction::Cascade,
                        match_kind: FkMatch::Simple,
                        deferrable: false,
                        initially_deferred: false,
                        not_valid: false,
                        enforced: true,
                    })),
                ),
            );
        }
    }
    t.add(
        root,
        Obj::new(ObjectKind::View, "v_ozet").with_props(Props::View(Arc::new(ViewProps {
            source: Some(SourceRef::of("SELECT 1")),
            updatable: Some(false),
            ..Default::default()
        }))),
    );
    t.add(
        root,
        Obj::new(ObjectKind::Unknown, "gelecek").with_props(Props::Generic(Arc::new(
            GenericProps {
                raw_kind: Some(Name::from("q")),
                ..Default::default()
            },
        ))),
    );
    NamespaceSnapshot::build(
        NsKey::schema(["app", "public"]),
        root_namespace("postgres").dbms(),
        FoldRule::Exact,
        NsMeta {
            level: Level::Structure,
            watermark: Watermark::PgXid {
                w: 1_234,
                identity: PgIdentity {
                    system_identifier: 7,
                    start_time_micros: 8,
                },
            },
            loaded_at_ms: 9,
        },
        t,
    )
    .expect("a valid seed namespace")
}

fn mysql_schema() -> NamespaceSnapshot {
    let mut t = Subtree::new(Obj::new(ObjectKind::Schema, "shop"));
    let root = t.root();
    let tab = t.add(
        root,
        Obj::new(ObjectKind::Table, "Orders")
            .with_state(StateMark::MyTable {
                created: 1,
                trigger_max: 2,
                trigger_count: 1,
            })
            .with_props(Props::Table(Arc::new(TableProps {
                ext: TableExt::MySql {
                    engine: Some(Name::from("InnoDB")),
                    collation: Some(Name::from("utf8mb4_0900_ai_ci")),
                    options: None,
                },
                ..Default::default()
            }))),
    );
    t.add(tab, column("id", 1, "int unsigned", false));
    NamespaceSnapshot::build(
        NsKey::schema(["shop"]),
        root_namespace("mysql").dbms(),
        FoldRule::AsciiCaseless,
        NsMeta {
            level: Level::Sources,
            watermark: Watermark::MyTime {
                at_utc_micros: 5,
                server_uuid: Name::from("3e11fa47-71ca-11e1-9e33-c80aa9429562"),
                version: Name::from("8.4.3"),
                priv_hash: 6,
            },
            loaded_at_ms: 7,
        },
        t,
    )
    .expect("a valid seed namespace")
}

fn seeds() -> &'static [Vec<u8>] {
    static SEEDS: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    SEEDS.get_or_init(|| {
        [pg_schema(), mysql_schema()]
            .iter()
            .map(|ns| encode_namespace(ns).expect("a seed encodes"))
            .collect()
    })
}

fn cipher() -> &'static RowCipher {
    static CIPHER: OnceLock<RowCipher> = OnceLock::new();
    CIPHER.get_or_init(|| {
        RowCipher::new(RowKey::from_bytes(&[7; 32]), CATALOG_DOMAIN).expect("a cipher")
    })
}

fn check(bytes: &[u8]) {
    let Ok(d) = decode(bytes) else {
        return;
    };
    // A crafted payload may repeat strings its table should have shared; re-encoding interns
    // them, so the smaller body can fall under the expansion budget the original met. That
    // is the only refusal allowed here.
    let again = match encode_namespace(&d.ns) {
        Ok(again) => again,
        Err(DecodeError::Malformed("strings expand past the budget")) => return,
        Err(e) => panic!("a decoded namespace does not encode: {e}"),
    };
    let back = decode_namespace(&again).expect("a re-encoded namespace decodes");
    assert_eq!(back.key(), d.ns.key());
    assert_eq!(back.objs(), d.ns.objs());
    assert_eq!(back.meta(), d.ns.meta());
    let root = ObjId { ns: 0, idx: 0 };
    assert_eq!(subtree(&back, root), subtree(&d.ns, root));
}

fn framed(minor: u16, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + body.len());
    out.extend_from_slice(b"HVNS");
    out.extend_from_slice(&FORMAT_MAJOR.to_le_bytes());
    out.extend_from_slice(&minor.to_le_bytes());
    out.push(0);
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(body);
    reseal_checksum(&mut out);
    out
}

/// `base` with `edit[2..]` XOR-spliced into its body at the offset `edit[..2]` picks (bytes past
/// the end are appended), stamped with `minor`, checksum recomputed.
fn spliced(base: &[u8], edit: &[u8], minor: u16) -> Vec<u8> {
    let mut b = base.to_vec();
    b[6..8].copy_from_slice(&minor.to_le_bytes());
    if let [lo, hi, rest @ ..] = edit {
        let span = b.len() - HEADER_LEN;
        let at = HEADER_LEN + usize::from(u16::from_le_bytes([*lo, *hi])) % span;
        for (i, x) in rest.iter().enumerate() {
            match b.get_mut(at + i) {
                Some(y) => *y ^= x,
                None => b.push(*x),
            }
        }
    }
    reseal_checksum(&mut b);
    b
}

fuzz_target!(|data: &[u8]| {
    let _ = payload_version(data);
    let (mode, rest) = data.split_first().map_or((0, data), |(m, r)| (*m, r));
    let newer = FORMAT_MINOR + 1;
    match mode % 5 {
        0 => check(data),
        1 => check(&framed(FORMAT_MINOR, rest)),
        2 => check(&framed(newer, rest)),
        m => {
            let seeds = seeds();
            let (pick, edit) = rest.split_first().map_or((0, rest), |(p, e)| (*p, e));
            let base = &seeds[usize::from(pick) % seeds.len()];
            let minor = if m == 3 { FORMAT_MINOR } else { newer };
            check(&spliced(base, edit, minor));
        }
    }

    let _ = decode_ns_key(data);
    let _ = decode_watermark(data);
    let _ = unpad(data);
    assert!(cipher().open(b"hv-fuzz-ad", data).is_err());
});
