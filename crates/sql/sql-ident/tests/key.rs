//! HV-INTRO-14 identifier keys (M4 Task 3; R-M4-28, R-M4-41): `match_key` folds by the server's
//! own rule per dialect and name class, never by the process locale and never by Unicode case
//! mapping. The suite runs under the default locale and under `LC_ALL=tr_TR.UTF-8` (CI
//! `turkey-test`); every expected value below is fixed, so a locale-dependent fold fails it.

use std::sync::Arc;

use sql_dialect::PgWireFlavor;
use sql_ident::key::{
    DialectId, FoldRule, IdentCtx, IdentKey, NameClass, ServerDependent, has_non_ascii_cased,
    match_key,
};

const CLASSES: [NameClass; 8] = [
    NameClass::Database,
    NameClass::Schema,
    NameClass::Table,
    NameClass::Column,
    NameClass::Routine,
    NameClass::Trigger,
    NameClass::Account,
    NameClass::Other,
];

fn pg_ctx(encoding: Option<&str>) -> IdentCtx {
    IdentCtx {
        encoding: encoding.map(Arc::from),
        ..IdentCtx::new(DialectId::Postgres)
    }
}

fn ik_pg(text: &str, quoted: bool) -> IdentKey {
    match_key(
        DialectId::Postgres,
        &pg_ctx(Some("UTF8")),
        text,
        quoted,
        NameClass::Table,
    )
    .expect("a PostgreSQL key on a UTF8 server")
}

fn lctn(n: u8) -> IdentCtx {
    IdentCtx {
        lower_case_table_names: n,
        ..IdentCtx::new(DialectId::MySql)
    }
}

fn ik_my(n: u8, text: &str, class: NameClass) -> IdentKey {
    match_key(DialectId::MySql, &lctn(n), text, false, class).expect("a MySQL key")
}

fn ik_sqlite(text: &str) -> IdentKey {
    match_key(
        DialectId::Sqlite,
        &IdentCtx::new(DialectId::Sqlite),
        text,
        false,
        NameClass::Table,
    )
    .expect("a SQLite key")
}

#[test]
fn pg_ident_key_ascii_only() {
    assert_eq!(ik_pg("MÜŞTERİ", false).as_str(), "mÜŞterİ");
    assert_eq!(ik_pg("Users", false).as_str(), "users");
    assert_eq!(ik_pg("Users", true).as_str(), "Users");
    assert_eq!(ik_pg("ılık", false).as_str(), "ılık");
    assert_eq!(ik_pg("ISTANBUL", false).as_str(), "istanbul");
    // Unquoted input folds; quoted input is the catalog form.
    assert_eq!(ik_pg("USERS", false), ik_pg("users", true));
    assert_ne!(ik_pg("Users", true), ik_pg("users", true));
}

#[test]
fn pg_ident_key_cuts_at_63_bytes_on_a_character_boundary() {
    let long = "a".repeat(70);
    assert_eq!(ik_pg(&long, true).as_str().len(), 63);
    // 62 ASCII bytes and a two-byte `ş`: the cut drops the whole character.
    let text = format!("{}ş", "b".repeat(62));
    assert_eq!(ik_pg(&text, true).as_str(), "b".repeat(62));
    assert_eq!(
        ik_pg(&format!("{}X", "a".repeat(62)), false).as_str(),
        format!("{}x", "a".repeat(62))
    );
}

/// HV-INTRO-14 rule 3: a single-byte server encoding lowers high-bit letters by the server's
/// locale (`downcase_identifier`), so unquoted non-ASCII input is server-dependent there, and so
/// is a long quoted non-ASCII name (the 63-byte cut counts server-encoding bytes). An unknown
/// encoding is treated the same way (fail closed).
#[test]
fn pg_non_utf8_server_makes_unquoted_non_ascii_server_dependent() {
    for enc in [Some("LATIN1"), Some("SQL_ASCII"), Some("WIN1254"), None] {
        let ctx = pg_ctx(enc);
        let key = |text: &str, quoted: bool| {
            match_key(DialectId::Postgres, &ctx, text, quoted, NameClass::Column)
        };
        assert_eq!(key("Müşteri", false), Err(ServerDependent), "{enc:?}");
        assert_eq!(key("Müşteri", true).unwrap().as_str(), "Müşteri", "{enc:?}");
        assert_eq!(key("Users", false).unwrap().as_str(), "users", "{enc:?}");
        assert_eq!(
            key(&format!("{}ü", "a".repeat(62)), true),
            Err(ServerDependent),
            "{enc:?}"
        );
    }
    for utf8 in ["UTF8", "utf8", "UTF-8"] {
        let ctx = pg_ctx(Some(utf8));
        assert_eq!(
            match_key(
                DialectId::Postgres,
                &ctx,
                "Müşteri",
                false,
                NameClass::Column
            )
            .unwrap()
            .as_str(),
            "müşteri"
        );
    }
}

#[test]
fn mysql_lctn0_table_names_exact_columns_ascii_caseless() {
    assert_ne!(
        ik_my(0, "Items", NameClass::Table),
        ik_my(0, "items", NameClass::Table)
    );
    assert_ne!(
        ik_my(0, "Shop", NameClass::Database),
        ik_my(0, "shop", NameClass::Database)
    );
    assert_eq!(
        ik_my(0, "ITEMS", NameClass::Column),
        ik_my(0, "items", NameClass::Column)
    );
    assert_eq!(
        ik_my(0, "MyProc", NameClass::Routine),
        ik_my(0, "myproc", NameClass::Routine)
    );
    // Backticks never change MySQL's comparison.
    assert_eq!(
        match_key(DialectId::MySql, &lctn(0), "Items", true, NameClass::Table),
        Ok(ik_my(0, "Items", NameClass::Table))
    );
    // A non-ASCII table name is a byte key under lctn = 0.
    assert_eq!(ik_my(0, "Müşteri", NameClass::Table).as_str(), "Müşteri");
}

/// `ISTANBUL` → `istanbul`, never `ıstanbul`: a Turkish (or any locale) fold of `I` fails here.
#[test]
fn mysql_lctn1_ascii_i_never_becomes_dotless() {
    for n in [1, 2] {
        assert_eq!(ik_my(n, "ISTANBUL", NameClass::Table).as_str(), "istanbul");
        assert_eq!(
            ik_my(n, "ISTANBUL", NameClass::Database).as_str(),
            "istanbul"
        );
        assert_eq!(
            ik_my(n, "Items", NameClass::Table),
            ik_my(n, "items", NameClass::Table)
        );
    }
    assert_eq!(ik_my(0, "ID", NameClass::Column).as_str(), "id");
}

/// R-M4-41: until Task 17 pins the server's rule, a probe holding a non-ASCII **cased** letter is
/// `ServerDependent` for every MySQL/MariaDB class that compares case-insensitively: columns,
/// routines and the other caseless names under any `lower_case_table_names`, database and table
/// names under 1 and 2. Uncased letters (`日本`) and case-sensitive classes key normally.
#[test]
fn mysql_non_ascii_cased_column_is_server_dependent_until_pinned() {
    for d in [DialectId::MySql, DialectId::MariaDb] {
        for n in [0, 1, 2] {
            let ctx = IdentCtx {
                dialect: d,
                ..lctn(n)
            };
            let key = |text: &str, class| match_key(d, &ctx, text, false, class);
            for class in [NameClass::Column, NameClass::Routine, NameClass::Other] {
                assert_eq!(
                    key("Äb", class),
                    Err(ServerDependent),
                    "{d:?} {n} {class:?}"
                );
                assert_eq!(
                    key("äb", class),
                    Err(ServerDependent),
                    "{d:?} {n} {class:?}"
                );
                assert_eq!(
                    key("ıd", class),
                    Err(ServerDependent),
                    "{d:?} {n} {class:?}"
                );
                assert_eq!(key("日本", class).unwrap().as_str(), "日本");
                assert_eq!(key("ID", class).unwrap().as_str(), "id");
            }
            for class in [NameClass::Database, NameClass::Schema, NameClass::Table] {
                if n == 0 {
                    assert_eq!(key("Äb", class).unwrap().as_str(), "Äb", "{d:?} {class:?}");
                } else {
                    assert_eq!(
                        key("Äb", class),
                        Err(ServerDependent),
                        "{d:?} {n} {class:?}"
                    );
                    assert_eq!(key("日本", class).unwrap().as_str(), "日本");
                }
            }
        }
    }
    // An unknown `lower_case_table_names` value pins nothing for the classes it governs.
    let odd = lctn(7);
    assert_eq!(
        match_key(DialectId::MySql, &odd, "items", false, NameClass::Table),
        Err(ServerDependent)
    );
    assert!(match_key(DialectId::MySql, &odd, "items", false, NameClass::Column).is_ok());
}

/// ver/01 `#sqlite-case` (MySQL/SQLite spike F6): SQLite compares names ASCII-caselessly only.
#[test]
fn sqlite_ident_key_ascii_lowercase() {
    assert_eq!(ik_sqlite("Müşteri"), ik_sqlite("müşteri"));
    assert_ne!(ik_sqlite("MÜŞTERİ"), ik_sqlite("müşteri"));
    assert_eq!(ik_sqlite("ITEMS"), ik_sqlite("items"));
    assert_eq!(ik_sqlite("ISTANBUL").as_str(), "istanbul");
    for class in CLASSES {
        assert!(
            match_key(
                DialectId::Sqlite,
                &IdentCtx::new(DialectId::Sqlite),
                "MÜŞTERİ",
                true,
                class
            )
            .is_ok(),
            "{class:?}"
        );
    }
}

/// The fixed key table: identical under `LC_ALL=C` and `LC_ALL=tr_TR.UTF-8`; no Unicode or
/// locale case mapping (`İ`, `ı`, `ẞ`, `Σ` and the Kelvin sign stay as written).
#[test]
fn match_key_is_locale_free() {
    let cases: [(&str, &str, &str, &str); 8] = [
        // (input, PG unquoted, MySQL column, SQLite)
        ("ISTANBUL", "istanbul", "istanbul", "istanbul"),
        ("istanbul", "istanbul", "istanbul", "istanbul"),
        ("İstanbul", "İstanbul", "", "İstanbul"),
        ("Iı", "iı", "", "iı"),
        ("STRAẞE", "straẞe", "", "straẞe"),
        ("ΣΑΣ", "ΣΑΣ", "", "ΣΑΣ"),
        ("\u{212A}elvin", "\u{212A}elvin", "", "\u{212A}elvin"),
        ("Order_Items", "order_items", "order_items", "order_items"),
    ];
    for (input, pg, my, sqlite) in cases {
        assert_eq!(ik_pg(input, false).as_str(), pg, "{input}");
        let mysql = match_key(DialectId::MySql, &lctn(0), input, false, NameClass::Column);
        if my.is_empty() {
            assert_eq!(mysql, Err(ServerDependent), "{input}");
        } else {
            assert_eq!(mysql.unwrap().as_str(), my, "{input}");
        }
        assert_eq!(ik_sqlite(input).as_str(), sqlite, "{input}");
    }
}

/// Dialects outside M4's four engines have no pinned rule: fail closed.
#[test]
fn unpinned_dialects_are_server_dependent() {
    for d in [
        DialectId::Generic,
        DialectId::SqlServer,
        DialectId::ClickHouse,
        DialectId::Oracle,
    ] {
        for class in CLASSES {
            assert_eq!(
                match_key(d, &IdentCtx::new(d), "users", false, class),
                Err(ServerDependent),
                "{d:?} {class:?}"
            );
        }
    }
    // The forks that run PostgreSQL's parser share its fold; Redshift (quoted ASCII folded,
    // 127 bytes), CockroachDB (Unicode lowercasing) and unnamed forks are not pinned (I1).
    for (flavor, pinned) in [
        (PgWireFlavor::Timescale, true),
        (PgWireFlavor::Yugabyte, true),
        (PgWireFlavor::Redshift, false),
        (PgWireFlavor::Cockroach, false),
        (PgWireFlavor::Other, false),
    ] {
        let fork = DialectId::PgWire(flavor);
        let ctx = IdentCtx {
            encoding: Some(Arc::from("UTF8")),
            ..IdentCtx::new(fork)
        };
        for class in CLASSES {
            for quoted in [false, true] {
                let k = match_key(fork, &ctx, "Users", quoted, class);
                if pinned {
                    let want = if quoted { "Users" } else { "users" };
                    assert_eq!(k.unwrap().as_str(), want, "{flavor:?} {class:?}");
                } else {
                    assert_eq!(k, Err(ServerDependent), "{flavor:?} {class:?} {quoted}");
                }
            }
        }
    }
}

/// MySQL 8.4 §8.2.3: `User` (and role) comparisons are case-sensitive, whatever
/// `lower_case_table_names` says (I2).
#[test]
fn mysql_account_names_are_exact() {
    for d in [DialectId::MySql, DialectId::MariaDb] {
        for n in [0, 1, 2] {
            let ctx = IdentCtx {
                dialect: d,
                ..lctn(n)
            };
            let key = |t: &str| match_key(d, &ctx, t, false, NameClass::Account);
            assert_ne!(key("Bob").unwrap(), key("bob").unwrap(), "{d:?} {n}");
            assert_eq!(key("Bob").unwrap().as_str(), "Bob");
            assert_eq!(
                key("Müşteri").unwrap().as_str(),
                "Müşteri",
                "exact: never unpinned"
            );
        }
    }
    assert_eq!(ik_pg("Bob", false).as_str(), "bob");
    let pg = match_key(
        DialectId::Postgres,
        &pg_ctx(Some("UTF8")),
        "Bob",
        true,
        NameClass::Account,
    );
    assert_eq!(pg.unwrap().as_str(), "Bob");
}

/// MySQL 8.4 §11.2.3: `lower_case_table_names` "does not affect case sensitivity of trigger
/// identifiers"; they follow the file system. Exact under 0 (Linux), unpinned above (I3).
#[test]
fn mysql_trigger_names_exact_under_lctn0_unpinned_above() {
    for d in [DialectId::MySql, DialectId::MariaDb] {
        let ctx0 = IdentCtx {
            dialect: d,
            ..lctn(0)
        };
        assert_eq!(
            match_key(d, &ctx0, "Trg", false, NameClass::Trigger)
                .unwrap()
                .as_str(),
            "Trg"
        );
        for n in [1, 2] {
            let ctx = IdentCtx {
                dialect: d,
                ..lctn(n)
            };
            assert_eq!(
                match_key(d, &ctx, "trg", false, NameClass::Trigger),
                Err(ServerDependent),
                "{d:?} {n}"
            );
        }
    }
}

/// HV-INTRO-14 rule 3, cut side: on a non-UTF-8 server a quoted non-ASCII name is certain only
/// when it fits 63 bytes at 4 bytes per non-ASCII character (`EUC_TW`, `MULE_INTERNAL`).
#[test]
fn pg_non_utf8_quoted_names_fit_the_cut_at_four_bytes_a_character() {
    let ctx = pg_ctx(Some("EUC_TW"));
    let key = |t: &str| match_key(DialectId::Postgres, &ctx, t, true, NameClass::Table);
    let fits = format!("abc{}", "\u{4E2D}".repeat(15)); // 3 + 60 = 63
    assert_eq!(key(&fits).unwrap().as_str(), fits);
    let over = format!("abcd{}", "\u{4E2D}".repeat(15)); // 4 + 60 = 64 (49 UTF-8 bytes)
    assert_eq!(key(&over), Err(ServerDependent));
    assert!(
        match_key(
            DialectId::Postgres,
            &pg_ctx(Some("UTF8")),
            &over,
            true,
            NameClass::Table
        )
        .is_ok()
    );
}

/// The stored policy agrees with `match_key`: a catalog name keyed by its namespace's
/// `FoldRule` equals the key `match_key` gives the same text in its catalog (quoted) form,
/// whenever `match_key` gives one.
#[test]
fn stored_rule_agrees_with_match_key() {
    let corpus = [
        "users",
        "Users",
        "USERS",
        "Order_Items",
        "ISTANBUL",
        "İstanbul",
        "ılık",
        "Müşteri",
        "MÜŞTERİ",
        "日本",
        "e\u{301}",
        "a b",
        "",
    ];
    let ctxs = [
        (DialectId::Postgres, pg_ctx(Some("UTF8"))),
        (DialectId::MySql, lctn(0)),
        (DialectId::MySql, lctn(1)),
        (
            DialectId::MariaDb,
            IdentCtx {
                dialect: DialectId::MariaDb,
                ..lctn(2)
            },
        ),
        (DialectId::Sqlite, IdentCtx::new(DialectId::Sqlite)),
    ];
    for (d, ctx) in &ctxs {
        for class in CLASSES {
            let Some(rule) = FoldRule::for_names(*d, ctx, class) else {
                // Unpinned (MySQL triggers under lctn >= 1): `match_key` never keys.
                for text in corpus {
                    assert_eq!(match_key(*d, ctx, text, true, class), Err(ServerDependent));
                }
                continue;
            };
            for text in corpus {
                if let Ok(k) = match_key(*d, ctx, text, true, class) {
                    let name: Arc<str> = Arc::from(text);
                    assert_eq!(rule.key(&name), k, "{d:?} {class:?} {text}");
                    assert!(rule.matches(text, k.as_str()), "{d:?} {class:?} {text}");
                }
            }
        }
    }
    assert_eq!(
        FoldRule::for_names(DialectId::MySql, &lctn(0), NameClass::Table),
        Some(FoldRule::Exact)
    );
    assert_eq!(
        FoldRule::for_names(DialectId::MySql, &lctn(1), NameClass::Table),
        Some(FoldRule::AsciiCaseless)
    );
    assert_eq!(
        FoldRule::for_names(DialectId::MySql, &lctn(0), NameClass::Column),
        Some(FoldRule::AsciiCaseless)
    );
    assert_eq!(
        FoldRule::for_names(DialectId::Postgres, &pg_ctx(None), NameClass::Column),
        Some(FoldRule::Exact)
    );
    assert_eq!(
        FoldRule::for_names(
            DialectId::Generic,
            &IdentCtx::new(DialectId::Generic),
            NameClass::Table
        ),
        None
    );
}

#[test]
fn a_stored_key_shares_the_name_when_folding_changes_nothing() {
    let name: Arc<str> = Arc::from("users");
    let k = FoldRule::AsciiCaseless.key(&name);
    assert!(std::ptr::eq(k.as_str().as_ptr(), name.as_ptr()));
    let upper: Arc<str> = Arc::from("Users");
    assert_eq!(FoldRule::AsciiCaseless.key(&upper).as_str(), "users");
    assert_eq!(FoldRule::Exact.key(&upper).as_str(), "Users");
    assert!(FoldRule::AsciiCaseless.matches("Iı", "iı"));
    assert!(!FoldRule::AsciiCaseless.matches("İ", "i"));
    assert!(!FoldRule::Exact.matches("Users", "users"));
}

#[test]
fn non_ascii_cased_letters_are_detected_without_case_mapping() {
    for cased in ["Ä", "ä", "İ", "ı", "ẞ", "σ", "ǅ", "\u{212A}", "Ⓐ"] {
        assert!(has_non_ascii_cased(cased), "{cased}");
    }
    for uncased in ["", "abc", "ABC", "日本", "٣", "e\u{301}", "_$#", "a b"] {
        assert!(!has_non_ascii_cased(uncased), "{uncased}");
    }
}
