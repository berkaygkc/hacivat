//! `HvNorm1` statement fingerprints (SQL-PARAM-03; M3b Task 6, P-M3b-18): literals and pure
//! `?` lists become placeholders, whitespace and comments collapse, keywords are
//! ASCII-uppercased by the lexer's keyword table, identifiers and quoted text stay byte-exact,
//! and the hash is XXH3-64 of the normalised text.

use proptest::prelude::*;
use sql_dialect::{DialectCtx, DialectId};
use sql_split::{FingerprintAlgo, hv_norm1};
use twox_hash::XxHash3_64;

fn pg() -> DialectCtx {
    DialectCtx::new(DialectId::Postgres, None)
}

fn mysql() -> DialectCtx {
    DialectCtx::new(DialectId::MySql, None)
}

fn norm(s: &str, ctx: &DialectCtx) -> String {
    hv_norm1(s, ctx).normalized.to_string()
}

#[test]
fn hv_norm1_groups_in_lists() {
    assert_eq!(
        hv_norm1("SELECT * FROM t WHERE a IN (1,2,3)", &pg()).hash,
        hv_norm1("select * from t where a in (9)", &pg()).hash
    );
}

#[test]
fn hv_norm1_keeps_turkish_identifiers_exact() {
    assert_ne!(
        hv_norm1("SELECT müşteri FROM t", &pg()).hash,
        hv_norm1("SELECT musteri FROM t", &pg()).hash
    );
}

#[test]
fn alg10_param03_mysql_case() {
    let a = hv_norm1("SELECT * FROM t WHERE id IN (1,2,3) AND name='x'", &mysql());
    let b = hv_norm1(
        "select * from t where id in (9) and name = 'y' -- c",
        &mysql(),
    );
    // `NAME` is in MySQL's keyword table, so it is uppercased (P-M3b-18; alg/10's sample text
    // predates the ruling). What the case pins is that both statements agree.
    assert_eq!(
        &*a.normalized,
        "SELECT * FROM t WHERE id IN (?…) AND NAME = ?"
    );
    assert_eq!(a.normalized, b.normalized);
    assert_eq!(a.hash, b.hash);
}

#[test]
fn hash_is_xxh3_of_the_normalised_text_and_versioned() {
    let f = hv_norm1("select 1", &pg());
    assert_eq!(f.algo, FingerprintAlgo::HvNorm1);
    assert_eq!(f.version, 1);
    assert_eq!(&*f.normalized, "SELECT ?");
    assert_eq!(f.hash, XxHash3_64::oneshot(b"SELECT ?"));
}

#[test]
fn keywords_upper_identifiers_and_quoted_text_exact() {
    assert_eq!(
        norm("select \"Müşteri\", Ad, ad from \"T\" where x = 'y'", &pg()),
        "SELECT \"Müşteri\", Ad, ad FROM \"T\" WHERE x = ?"
    );
    // Only ASCII casing: a Turkish dotless `ı` is not `I`, so `lımıt` is no keyword.
    assert_eq!(norm("select 1 lımıt 2", &pg()), "SELECT ? lımıt ?");
    // Identifiers differing in case keep differing.
    assert_ne!(
        hv_norm1("SELECT Ad FROM t", &pg()).hash,
        hv_norm1("SELECT ad FROM t", &pg()).hash
    );
}

#[test]
fn literals_and_placeholders_become_question_marks() {
    assert_eq!(
        norm(
            "SELECT 1, -2.5, 'a', E'b', $$c$$, B'101', X'ff', $1, :n FROM t",
            &pg()
        ),
        "SELECT ?, - ?, ?, ?, ?, ?, ?, ?, ? FROM t"
    );
    // A MySQL charset introducer belongs to its string.
    assert_eq!(norm("SELECT _utf8mb4'x', ?", &mysql()), "SELECT ?, ?");
    // Lists of placeholders collapse; mixed lists keep their shape.
    assert_eq!(
        norm("SELECT f(1, 2), g(a, 1)", &pg()),
        "SELECT f (?…), g (a, ?)"
    );
}

#[test]
fn comments_and_whitespace_never_change_the_hash() {
    let a = hv_norm1("SELECT a\n  FROM t -- note\n WHERE b = 1", &pg());
    let b = hv_norm1("/* lead */ select a from t where b = 2", &pg());
    assert_eq!(a.normalized, b.normalized);
    assert_eq!(a.hash, b.hash);
}

#[test]
fn executable_comments_are_code_and_hints_are_placeholders() {
    assert_eq!(
        norm(
            "SELECT /*!80000 sql_no_cache */ a FROM t WHERE b = 'x'",
            &mysql()
        ),
        "SELECT /*!80000 SQL_NO_CACHE */ a FROM t WHERE b = ?"
    );
    assert_eq!(
        norm("SELECT /*! 1 + 'x' */ a", &mysql()),
        "SELECT /*! ? + ? */ a"
    );
    assert_eq!(
        norm("SELECT /*+ MAX_EXECUTION_TIME(1000) */ a FROM t", &mysql()),
        "SELECT /*+…*/ a FROM t"
    );
}

#[test]
fn never_panics_on_broken_text() {
    for s in [
        "'",
        "/*",
        "$$",
        "\"",
        "SELECT (",
        ")))",
        "",
        " ",
        "\u{feff}SELECT 1",
        "/*! '",
    ] {
        let _ = hv_norm1(s, &pg());
        let _ = hv_norm1(s, &mysql());
        let _ = hv_norm1(s, &DialectCtx::new(DialectId::SqlServer, None));
    }
}

proptest! {
    #[test]
    fn literal_values_never_change_the_hash(a in 0u32..1_000_000, b in "[a-z ]{0,12}", n in 1usize..6) {
        let list: Vec<String> = (0..n).map(|i| (a as usize + i).to_string()).collect();
        let x = format!("SELECT * FROM t WHERE c IN ({}) AND d = '{b}'", list.join(", "));
        let y = "select * from t where c in (0) and d = ''";
        prop_assert_eq!(hv_norm1(&x, &pg()).hash, hv_norm1(y, &pg()).hash);
    }

    #[test]
    fn normalising_is_deterministic_and_total(s in "\\PC{0,64}") {
        let a = hv_norm1(&s, &pg());
        let b = hv_norm1(&s, &pg());
        prop_assert_eq!(a, b);
    }
}
