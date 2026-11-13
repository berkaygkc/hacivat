//! Post-statement checks and input limits (SQL-SAFE-05).

use hacivat_guard::{
    GapReason, InputViolation, MAX_STATEMENT_BYTES, PostCheck, PostFacts, TxStatusLite,
    check_input, check_post,
};
use hacivat_proto::DbmsKind;

/// A clean guarded statement: still inside the guard's transaction, no canary, no tag.
fn facts() -> Facts {
    Facts(PostFacts {
        tx_after_statement: Some(TxStatusLite::InTx),
        ..PostFacts::default()
    })
}

struct Facts(PostFacts);

impl Facts {
    fn tx_after(mut self, s: TxStatusLite) -> Self {
        self.0.tx_after_statement = Some(s);
        self
    }
    fn canary(mut self, v: Option<&str>) -> Self {
        self.0.canary = Some(v.map(str::to_owned));
        self
    }
    fn rollback_notice(mut self, code: &str) -> Self {
        self.0.rollback_notice_codes.push(code.to_owned());
        self
    }
    fn tag(mut self, t: &str) -> Self {
        self.0.command_tag = Some(t.to_owned());
        self
    }
    fn rows(mut self, n: u64) -> Self {
        self.0.rows_affected = Some(n);
        self
    }
}

impl std::ops::Deref for Facts {
    type Target = PostFacts;
    fn deref(&self) -> &PostFacts {
        &self.0
    }
}

const GAP_ENDED: PostCheck = PostCheck::Gap(GapReason::TransactionEndedByStatement);
const GAP_WRITE: PostCheck = PostCheck::Gap(GapReason::WriteSucceeded);

#[test]
fn post_check_gaps() {
    assert_eq!(
        check_post(DbmsKind::Postgres, &facts().canary(Some("731"))),
        PostCheck::Gap(GapReason::CanaryAssigned)
    );
    assert_eq!(
        check_post(DbmsKind::Postgres, &facts().tx_after(TxStatusLite::Idle)),
        GAP_ENDED
    );
    assert_eq!(
        check_post(DbmsKind::Postgres, &facts().rollback_notice("25P01")),
        GAP_ENDED
    );
    assert_eq!(
        check_post(DbmsKind::MySql, &facts().tag("INSERT").rows(1)),
        GAP_WRITE
    );
    assert_eq!(
        check_post(
            DbmsKind::Postgres,
            &facts().tx_after(TxStatusLite::InTx).canary(None)
        ),
        PostCheck::Clean
    );
}

#[test]
fn idle_after_statement_is_a_gap_on_every_transactional_engine() {
    for k in [DbmsKind::Postgres, DbmsKind::MySql, DbmsKind::MariaDb] {
        assert_eq!(
            check_post(k, &facts().tx_after(TxStatusLite::Idle)),
            GAP_ENDED,
            "{k:?}"
        );
        // A failed statement leaves the guard's transaction open (aborted): not an escape.
        assert_eq!(
            check_post(k, &facts().tx_after(TxStatusLite::Failed)),
            PostCheck::Clean,
            "{k:?}"
        );
    }
}

#[test]
fn unknown_transaction_state_fails_closed() {
    let unknown = PostFacts::default();
    for k in [DbmsKind::Postgres, DbmsKind::MySql, DbmsKind::MariaDb] {
        assert_eq!(check_post(k, &unknown), GAP_ENDED, "{k:?}");
    }
}

#[test]
fn null_canary_is_clean_and_any_value_is_a_gap() {
    assert_eq!(
        check_post(DbmsKind::Postgres, &facts().canary(None)),
        PostCheck::Clean
    );
    assert_eq!(
        check_post(DbmsKind::Postgres, &facts().canary(Some(""))),
        PostCheck::Gap(GapReason::CanaryAssigned)
    );
}

#[test]
fn rollback_notice_is_matched_case_insensitively_among_others() {
    let f = facts().rollback_notice("01000").rollback_notice(" 25p01 ");
    assert_eq!(check_post(DbmsKind::Postgres, &f), GAP_ENDED);
    assert_eq!(
        check_post(DbmsKind::Postgres, &facts().rollback_notice("01000")),
        PostCheck::Clean
    );
}

#[test]
fn dml_tags_need_affected_rows() {
    for tag in [
        "INSERT 0 3",
        "update 2",
        "Delete 1",
        "MERGE 1",
        "COPY 10",
        "REPLACE",
        "LOAD",
    ] {
        assert_eq!(
            check_post(DbmsKind::Postgres, &facts().tag(tag).rows(1)),
            GAP_WRITE,
            "{tag}"
        );
        assert_eq!(
            check_post(DbmsKind::Postgres, &facts().tag(tag).rows(0)),
            PostCheck::Clean,
            "{tag}"
        );
        assert_eq!(
            check_post(DbmsKind::Postgres, &facts().tag(tag)),
            PostCheck::Clean,
            "{tag}"
        );
    }
    for tag in ["SHOW", "EXPLAIN", "INSERTX", "WITHOUT", ""] {
        assert_eq!(
            check_post(DbmsKind::MySql, &facts().tag(tag).rows(5)),
            PostCheck::Clean,
            "{tag:?}"
        );
    }
}

#[test]
fn mysql_reads_that_report_affected_rows_are_writes() {
    // Fix round 2, M-1: `WITH … UPDATE`, `CALL p()`, `DO f()` and `SELECT … INTO OUTFILE`
    // report affected rows on MySQL; a statement returning a result set reports 0.
    for k in [DbmsKind::MySql, DbmsKind::MariaDb] {
        for tag in ["WITH", "with", "CALL", "Do", "SELECT"] {
            assert_eq!(
                check_post(k, &facts().tag(tag).rows(3)),
                GAP_WRITE,
                "{k:?} {tag}"
            );
            assert_eq!(
                check_post(k, &facts().tag(tag).rows(0)),
                PostCheck::Clean,
                "{k:?} {tag}"
            );
            assert_eq!(
                check_post(k, &facts().tag(tag)),
                PostCheck::Clean,
                "{k:?} {tag}"
            );
        }
    }
}

#[test]
fn postgres_select_row_counts_are_reads() {
    // PostgreSQL's `SELECT n` counts the rows returned, and its tags never start with `WITH`,
    // `CALL` or `DO` carrying a write count: the MySQL rule stays off.
    for tag in ["SELECT 5", "WITH", "CALL", "DO"] {
        assert_eq!(
            check_post(DbmsKind::Postgres, &facts().tag(tag).rows(5)),
            PostCheck::Clean,
            "{tag}"
        );
        assert_eq!(
            check_post(DbmsKind::Sqlite, &facts().tag(tag).rows(5)),
            PostCheck::Clean,
            "{tag}"
        );
    }
}

#[test]
fn ddl_tags_are_a_gap_without_a_row_count() {
    // DDL tags never carry a row count, so a successful one inside the read-only transaction
    // is an escape on its own.
    for tag in [
        "CREATE TABLE",
        "DROP TABLE",
        "ALTER TABLE",
        "TRUNCATE TABLE",
        "truncate",
        "\tGRANT",
        "REVOKE",
        "RENAME",
    ] {
        assert_eq!(
            check_post(DbmsKind::Postgres, &facts().tag(tag)),
            GAP_WRITE,
            "{tag:?}"
        );
        assert_eq!(
            check_post(DbmsKind::MySql, &facts().tag(tag).rows(0)),
            GAP_WRITE,
            "{tag:?}"
        );
    }
}

#[test]
fn transaction_end_outranks_other_gaps() {
    let f = facts()
        .tx_after(TxStatusLite::Idle)
        .canary(Some("9"))
        .tag("INSERT 0 1")
        .rows(1);
    assert_eq!(check_post(DbmsKind::Postgres, &f), GAP_ENDED);
    let f = facts().canary(Some("9")).tag("INSERT 0 1").rows(1);
    assert_eq!(check_post(DbmsKind::Postgres, &f), GAP_WRITE);
}

#[test]
fn sqlite_checks_writes_only() {
    // SQLite runs in autocommit: `Idle` is its normal state, and it has no canary.
    assert_eq!(
        check_post(DbmsKind::Sqlite, &facts().tx_after(TxStatusLite::Idle)),
        PostCheck::Clean
    );
    assert_eq!(
        check_post(DbmsKind::Sqlite, &PostFacts::default()),
        PostCheck::Clean
    );
    assert_eq!(
        check_post(DbmsKind::Sqlite, &facts().tag("DELETE").rows(2)),
        GAP_WRITE
    );
}

#[test]
fn engines_without_a_plan_always_report_a_gap() {
    for k in [DbmsKind::MsSql, DbmsKind::ClickHouse, DbmsKind::Redshift] {
        assert_eq!(check_post(k, &facts()), GAP_ENDED, "{k:?}");
    }
}

#[test]
fn input_limits() {
    assert_eq!(
        check_input(&"x".repeat(MAX_STATEMENT_BYTES + 1)),
        Err(InputViolation::TooLong {
            limit: MAX_STATEMENT_BYTES
        })
    );
    assert_eq!(check_input("select '\0'"), Err(InputViolation::Nul));
    assert!(check_input("select 1").is_ok());
}

#[test]
fn input_limit_counts_bytes_and_is_inclusive() {
    assert!(check_input(&"x".repeat(MAX_STATEMENT_BYTES)).is_ok());
    // 'ğ' is two bytes in UTF-8: half the characters, the same byte limit.
    let multibyte = "ğ".repeat(MAX_STATEMENT_BYTES / 2 + 1);
    assert_eq!(
        check_input(&multibyte),
        Err(InputViolation::TooLong {
            limit: MAX_STATEMENT_BYTES
        })
    );
    assert_eq!(check_input("\0"), Err(InputViolation::Nul));
    assert!(check_input("").is_ok());
}
