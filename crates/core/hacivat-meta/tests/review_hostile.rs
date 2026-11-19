//! The M4 Task 3 reviewer's hostile tests (committed in fix round 1). The test oracle uses
//! Unicode case mapping on purpose, to find every character a Unicode-folding server could
//! change; the product code never does.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use hacivat_meta::fixture::ns_mysql;
use hacivat_meta::ids::NsKey;
use hacivat_meta::kind::{KindGroup, ObjectKind};
use hacivat_meta::name::{DialectId, IdentCtx, NameClass, ServerDependent, match_key};
use hacivat_meta::namespace::{NamespaceSnapshot, NsMeta, Obj, Subtree};
use hacivat_meta::view::Lookup;
use hacivat_proto::DbmsKind;
use proptest::prelude::*;

fn ascii_lower(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_uppercase() {
                (b + 32) as char
            } else {
                b as char
            }
        })
        .collect::<Vec<char>>()
        .into_iter()
        .collect::<String>()
}

/// Correct reference for byte strings: lower only ASCII bytes, keep the rest.
fn ascii_lower_str(s: &str) -> String {
    let mut v = s.as_bytes().to_vec();
    for b in &mut v {
        b.make_ascii_lowercase();
    }
    String::from_utf8(v).unwrap()
}

fn pg_cut(s: &str) -> &str {
    if s.len() <= 63 {
        return s;
    }
    let mut e = 63;
    while !s.is_char_boundary(e) {
        e -= 1;
    }
    &s[..e]
}

/// A char a Unicode-aware server could fold: any case mapping or folding changes it.
#[expect(
    clippy::disallowed_methods,
    reason = "test oracle: Unicode case mapping finds what a server could fold"
)]
fn unicode_foldable(c: char) -> bool {
    c.to_lowercase().ne(std::iter::once(c)) || c.to_uppercase().ne(std::iter::once(c))
}

fn utf8_pg() -> IdentCtx {
    IdentCtx {
        encoding: Some(Arc::from("UTF8")),
        ..IdentCtx::new(DialectId::Postgres)
    }
}

fn my(lctn: u8) -> IdentCtx {
    IdentCtx {
        lower_case_table_names: lctn,
        ..IdentCtx::new(DialectId::MySql)
    }
}

const CLASSES: [NameClass; 6] = [
    NameClass::Database,
    NameClass::Schema,
    NameClass::Table,
    NameClass::Column,
    NameClass::Routine,
    NameClass::Other,
];

/// Every Unicode scalar value, in three positions: the keys agree with an independent
/// ASCII-only reference (PG unquoted, SQLite), and every char a Unicode fold could change is
/// `ServerDependent` in each MySQL caseless class. Prints a digest so a `tr_TR` run can be
/// compared with a `C` run byte for byte. About 50 s in a debug build: run with `--ignored`
/// (fix round 1 ran it in debug and under `LC_ALL=tr_TR.UTF-8`).
#[test]
#[ignore = "slow: every Unicode scalar value (about 50 s in debug)"]
fn every_scalar_value_matches_the_documented_fold() {
    let mut h = DefaultHasher::new();
    let mut foldable_missed = Vec::new();
    for cp in 0u32..=0x10FFFF {
        let Some(c) = char::from_u32(cp) else {
            continue;
        };
        for text in [format!("{c}"), format!("Ab{c}"), format!("{c}Iİı")] {
            // PostgreSQL, UTF-8 server.
            let pg_u = match_key(
                DialectId::Postgres,
                &utf8_pg(),
                &text,
                false,
                NameClass::Table,
            )
            .expect("PG UTF8 always keys");
            assert_eq!(pg_u.as_str(), ascii_lower_str(pg_cut(&text)), "{cp:X}");
            let pg_q = match_key(
                DialectId::Postgres,
                &utf8_pg(),
                &text,
                true,
                NameClass::Column,
            )
            .unwrap();
            assert_eq!(pg_q.as_str(), pg_cut(&text), "{cp:X}");
            // SQLite.
            let sq = match_key(
                DialectId::Sqlite,
                &IdentCtx::new(DialectId::Sqlite),
                &text,
                cp % 2 == 0,
                NameClass::Table,
            )
            .unwrap();
            assert_eq!(sq.as_str(), ascii_lower_str(&text), "{cp:X}");
            // MySQL / MariaDB caseless classes.
            for d in [DialectId::MySql, DialectId::MariaDb] {
                for lctn in [0u8, 1, 2] {
                    let ctx = IdentCtx {
                        dialect: d,
                        ..my(lctn)
                    };
                    for class in CLASSES {
                        let caseless = !matches!(
                            class,
                            NameClass::Database | NameClass::Schema | NameClass::Table
                        ) || lctn != 0;
                        let k = match_key(d, &ctx, &text, false, class);
                        match &k {
                            Ok(key) => {
                                if caseless {
                                    if text.chars().any(|c| !c.is_ascii() && unicode_foldable(c)) {
                                        foldable_missed.push(cp);
                                    }
                                    assert_eq!(key.as_str(), ascii_lower_str(&text));
                                } else {
                                    assert_eq!(key.as_str(), text);
                                }
                                key.as_str().hash(&mut h);
                            }
                            Err(ServerDependent) => {
                                assert!(caseless, "exact class never server-dependent");
                                1u8.hash(&mut h);
                            }
                        }
                    }
                }
            }
            pg_u.as_str().hash(&mut h);
            sq.as_str().hash(&mut h);
        }
    }
    foldable_missed.dedup();
    assert!(
        foldable_missed.is_empty(),
        "chars a Unicode fold changes but keyed as certain: {:X?}",
        &foldable_missed[..foldable_missed.len().min(20)]
    );
    let _ = ascii_lower("x");
    println!(
        "KEY-DIGEST {:016x} LC_ALL={:?}",
        h.finish(),
        std::env::var("LC_ALL").ok()
    );
}

/// Stored names whose Unicode case image is ASCII (`İD`, the Kelvin sign, long s, dotless i)
/// never answer `Missing` to the ASCII probe a Unicode-folding server would match.
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "test oracle: the Unicode case images of every character"
)]
fn mysql_stored_name_with_ascii_image_is_never_missing() {
    let mut images = Vec::new();
    for cp in 0x80u32..=0x10FFFF {
        let Some(c) = char::from_u32(cp) else {
            continue;
        };
        let lo: String = c.to_lowercase().collect();
        let up: String = c.to_uppercase().collect();
        for img in [lo, up] {
            let ascii: String = img.chars().filter(char::is_ascii_alphabetic).collect();
            if !ascii.is_empty() && img.chars().next().is_some_and(|x| x.is_ascii()) {
                images.push((c, ascii));
            }
        }
    }
    assert!(images.iter().any(|(c, _)| *c == '\u{212A}'));
    for (c, ascii) in images {
        let stored = format!("{c}d");
        let probe = format!("{ascii}D");
        let n = ns_mysql("shop", 0).table("t", &[&stored, "plain"]).finish();
        let t = n.lookup(KindGroup::Relations, "t")[0];
        let got = n.match_child(t, ObjectKind::Column, &probe, false, &my(0));
        assert!(
            !matches!(got, Lookup::Missing(_)),
            "{:X} {stored} vs {probe}",
            c as u32
        );
    }
}

// Probe ordinary random strings: a MySQL column key is certain only if no char is foldable.
proptest! {
    #[test]
    fn mysql_key_certain_only_for_unfoldable_text(s in "\\PC{0,12}") {
        let k = match_key(DialectId::MySql, &my(0), &s, false, NameClass::Column);
        if k.is_ok() {
            prop_assert!(!s.chars().any(|c| !c.is_ascii() && unicode_foldable(c)), "{s:?}");
        }
    }
}

// ---- Findings of the review: they failed on 8d3f30c and pass after fix round 1 ----

/// MySQL account names are case-sensitive (8.4 manual §8.2.3 "Grant Table Scope Column
/// Properties"): `Bob` must not be `Found` for the probe `bob`.
#[test]
fn finding_mysql_user_names_are_case_sensitive() {
    let mut t = Subtree::new(Obj::new(ObjectKind::Root, ""));
    t.add(t.root(), Obj::new(ObjectKind::User, "Bob"));
    let n = NamespaceSnapshot::build(
        NsKey::server(),
        DbmsKind::MySql,
        hacivat_meta::name::FoldRule::Exact,
        NsMeta::default(),
        t,
    )
    .unwrap();
    let got = n.match_named(KindGroup::Other, "bob", false, &my(0));
    assert!(!matches!(got, Lookup::Found(_)), "{got:?}");
}

#[test]
fn print_sizes() {
    use hacivat_meta::props::*;
    println!(
        "SIZES Obj={} ColumnProps={} TableProps={} ColumnExt={} Option<TypeRef>={} Option<Generated>={} StateMark={} Option<ServerId>={} Props={}",
        size_of::<Obj>(),
        size_of::<ColumnProps>(),
        size_of::<TableProps>(),
        size_of::<ColumnExt>(),
        size_of::<Option<TypeRef>>(),
        size_of::<Option<Generated>>(),
        size_of::<hacivat_meta::StateMark>(),
        size_of::<Option<hacivat_meta::ServerId>>(),
        size_of::<Props>(),
    );
}
