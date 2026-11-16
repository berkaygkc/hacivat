//! The libpg_query build proof (R-M3-1), run by `build-test` on macOS, Windows and Linux: the
//! vendored C library compiles, links and parses, and its pre-generated protobuf code decodes
//! what the C library returns.

#[test]
fn pg_query_links_and_counts_statements() {
    assert_eq!(sql_oracle_pg::statement_count("SELECT 1; SELECT 2"), Ok(2));
    assert_eq!(
        sql_oracle_pg::statement_count("SELECT ';'; -- ;\nSELECT 2"),
        Ok(2)
    );
    assert_eq!(sql_oracle_pg::statement_count(""), Ok(0));
}

#[test]
fn version_names_postgres_18() {
    let v = sql_oracle_pg::version();
    assert!(v.contains("18"), "{v}");
    // "18.6" is read from the linked C library at run time (`PG_VERSION_NUM`), so a stale or
    // mismatched libpg_query build fails here.
    assert_eq!(v, "pg_query 18.1.0 (PostgreSQL 18.6)");
}

#[test]
fn parse_error_is_an_error_not_a_panic() {
    let e = sql_oracle_pg::statement_count("SELEC 1").unwrap_err();
    assert!(e.message.contains("syntax error"), "{e:?}");
    assert!(sql_oracle_pg::statement_count("SELECT 'a\0b'").is_err());
}
