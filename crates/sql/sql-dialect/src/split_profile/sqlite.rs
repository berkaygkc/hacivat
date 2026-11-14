//! SQLite's splitter tables. `CREATE [TEMP|TEMPORARY] TRIGGER … BEGIN … END` carries `;`s, with
//! `CASE … END` inside; a top-level `BEGIN [DEFERRED|IMMEDIATE|EXCLUSIVE] [TRANSACTION]` is a
//! transaction and `END [TRANSACTION]` at statement start is `COMMIT`.

use super::{BlockFamily, HeadContinuation, SplitProfile};

pub(super) static SQLITE: SplitProfile = SplitProfile {
    name: "sqlite",
    family: BlockFamily::Sqlite,
    statement_starts: &[
        "SELECT", "INSERT", "UPDATE", "DELETE", "REPLACE", "WITH", "CREATE", "DROP", "ALTER",
        "EXPLAIN", "PRAGMA", "VACUUM", "ATTACH", "DETACH", "REINDEX", "ANALYZE", "COMMIT",
        "ROLLBACK",
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
        "DO",
        "ON",
        "BEFORE",
        "AFTER",
        "OF",
        "OR",
        "THEN",
        "ELSE",
        "INSTEAD",
        "EXPLAIN",
        "PLAN",
        "WITH",
    ],
    head_continuations: &[
        HeadContinuation {
            head: "INSERT",
            words: &[],
            once: true,
        },
        HeadContinuation {
            head: "REPLACE",
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
    ],
    routine_head_skips: &["TEMP", "TEMPORARY"],
    routine_kinds: &["TRIGGER"],
    top_level_block_begins: &[],
    statement_position_after: &[],
    body_start_after: &[],
    statement_openers: &[],
    end_followers: &[],
};
