//! The PostgreSQL regression differential (Task 20; SQL-T13; the P0 exit, `alg/10` :171): our
//! PG split equals libpg_query's on at least 99.9 % of the regression corpus's statements
//! (`sql_testkit::differential` says how they are compared), and every disagreement is
//! listed.
//!
//! Needs the corpus: `cargo xtask fetch-pg-regress --tag REL_17_2` (network once; pinned by
//! digest). It runs with the `differential` feature (T1a R1-M2: the plain test loops never
//! build libpg_query through this crate), in the nightly `sql-differential` job and at the
//! gate:
//!
//! ```text
//! cargo test -p sql-testkit --features differential --test pg_regress_differential -- --ignored
//! ```
//!
//! Without the feature this file compiles to nothing, so `cargo test --workspace` stays as it
//! was.

#[cfg(feature = "differential")]
mod differential {
    use sql_testkit::differential::{TAG, compare_dir, corpus_dir};

    #[test]
    #[ignore = "network"]
    fn pg_regress_split_agrees_at_least_999_per_mille() {
        let dir = corpus_dir(TAG);
        let Some(o) = compare_dir(&dir) else {
            panic!(
                "no corpus at {}: run `cargo xtask fetch-pg-regress --tag {TAG}` first",
                dir.display()
            );
        };
        eprintln!(
            "pg_regress {TAG}: {} files, {} statements: {} agree, {} disagree, {} not comparable \
             (syntax errors), {} uncertain (refused); {:.2} per mille; unreadable: {:?}",
            o.files,
            o.statements,
            o.agreed,
            o.disagreed.len(),
            o.not_comparable,
            o.uncertain,
            o.per_mille(),
            o.unreadable
        );
        for d in &o.disagreed {
            eprintln!("  disagree {} {:?}: {:?}", d.file, d.range, d.head);
        }
        assert!(o.files >= 200, "{} files", o.files);
        assert!(o.agreed >= 10_000, "{} statements agree", o.agreed);
        assert!(
            o.per_mille() >= 999.0,
            "{:.2} per mille agree (the P0 exit is 999)",
            o.per_mille()
        );
    }
}
