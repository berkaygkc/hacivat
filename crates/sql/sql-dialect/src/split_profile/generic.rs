//! The generic splitter tables (the `Generic` dialect, and every dialect without its own
//! profile in M3a): parentheses carry `;`s, no blocks.

use super::{BlockFamily, HeadContinuation, SplitProfile};

pub(super) static GENERIC: SplitProfile = SplitProfile {
    name: "generic",
    family: BlockFamily::Plain,
    statement_starts: &[
        "SELECT", "INSERT", "UPDATE", "DELETE", "WITH", "CREATE", "DROP", "ALTER", "GRANT",
        "REVOKE", "COMMIT", "ROLLBACK",
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
        "ON",
        "OF",
        "OR",
        "THEN",
        "ELSE",
        "WITH",
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
    ],
    routine_head_skips: &[],
    routine_kinds: &[],
    top_level_block_begins: &[],
    statement_position_after: &[],
    body_start_after: &[],
    statement_openers: &[],
    end_followers: &[],
};
