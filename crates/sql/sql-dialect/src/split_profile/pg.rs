//! PostgreSQL's splitter tables (and the PostgreSQL-wire servers'). psql tracks parentheses,
//! so `CREATE RULE … DO (…; …)` needs no table row; dollar quotes are single tokens; a
//! top-level `BEGIN` is always a transaction and `END` at statement start is `COMMIT`.

use super::{BlockFamily, HeadContinuation, NAME_AFTER, SplitProfile};

pub(super) static PG: SplitProfile = SplitProfile {
    name: "pg",
    family: BlockFamily::Postgres,
    statement_starts: &[
        "SELECT", "INSERT", "UPDATE", "DELETE", "MERGE", "WITH", "CREATE", "DROP", "ALTER",
        "TRUNCATE", "GRANT", "REVOKE", "EXPLAIN", "SHOW", "COPY", "CALL", "COMMIT", "ROLLBACK",
        "VACUUM", "ANALYZE", "COMMENT",
    ],
    prev_exceptions: &[
        ",",
        ".",
        "AS",
        "UNION",
        "INTERSECT",
        "EXCEPT",
        "ALL",
        "DISTINCT",
        "FOR",
        "DO",
        "ON",
        "BEFORE",
        "AFTER",
        "OF",
        "OR",
        "THEN",
        "ELSE",
        "INSTEAD",
        "ALSO",
        "GRANT",
        "REVOKE",
        "EXPLAIN",
        "ANALYZE",
        "VERBOSE",
        "WITH",
        "RETURN",
    ],
    head_continuations: &[
        HeadContinuation {
            head: "INSERT",
            words: &[],
            once: true,
        },
        HeadContinuation {
            head: "CREATE",
            words: &[],
            once: true,
        },
        HeadContinuation {
            head: "WITH",
            words: &[],
            once: true,
        },
        HeadContinuation {
            head: "EXPLAIN",
            words: &[],
            once: true,
        },
        HeadContinuation {
            head: "ALTER",
            words: &["ALTER", "DROP"],
            once: false,
        },
        HeadContinuation {
            head: "GRANT",
            words: &[],
            once: false,
        },
        HeadContinuation {
            head: "REVOKE",
            words: &[],
            once: false,
        },
    ],
    routine_head_skips: &["OR", "REPLACE"],
    routine_kinds: &["FUNCTION", "PROCEDURE"],
    top_level_block_begins: &[],
    statement_position_after: &[],
    body_start_after: &[],
    statement_openers: &[],
    end_followers: &[],
    name_after: NAME_AFTER,
    // A `BEGIN ATOMIC` body's statements each end with `;` (PostgreSQL's grammar).
    block_end_after: &[";", "ATOMIC"],
    compound_at_top: false,
    handler: None,
};
