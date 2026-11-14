//! MySQL's and MariaDB's splitter tables. Stored programs (`CREATE … PROCEDURE|FUNCTION|
//! TRIGGER|EVENT`, `ALTER … EVENT … DO`) are tracked without `DELIMITER`, as the server receives
//! the whole `CREATE PROCEDURE` as one statement over the protocol; inside them `BEGIN` opens a
//! block as the body, where a statement starts, or as a handler's action (`begin` and `end`
//! are non-reserved names too: `SELECT begin FROM t`). A top-level `BEGIN [WORK]` is a transaction; `BEGIN NOT ATOMIC` opens one
//! (MariaDB; on MySQL it is a syntax error, so tracking it as a block only keeps a broken
//! script in one piece). MariaDB adds `FOR … END FOR` loops, and runs `IF`, `CASE`, `LOOP`,
//! `REPEAT`, `WHILE` and `FOR` compound statements at top level too (`compound_at_top`; on
//! MySQL they are syntax errors, so tracking them keeps a broken script in one piece).

use super::{BlockFamily, BlockKind, HeadContinuation, NAME_AFTER, SplitProfile};

const STARTS: &[&str] = &[
    "SELECT", "INSERT", "UPDATE", "DELETE", "REPLACE", "WITH", "CREATE", "DROP", "ALTER",
    "TRUNCATE", "GRANT", "REVOKE", "EXPLAIN", "DESCRIBE", "SHOW", "USE", "CALL", "COMMIT",
    "ROLLBACK", "OPTIMIZE", "FLUSH",
];

const PREV: &[&str] = &[
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
    "KEY",
    "BEFORE",
    "AFTER",
    "OF",
    "OR",
    "THEN",
    "ELSE",
    "GRANT",
    "REVOKE",
    "EXPLAIN",
    "ANALYZE",
    "WITH",
    "ROW",
    "RETURN",
];

const HEADS: &[HeadContinuation] = &[
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
        head: "DESCRIBE",
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
];

const SKIPS: &[&str] = &["OR", "REPLACE", "DEFINER", "AGGREGATE"];
const ROUTINES: &[&str] = &["PROCEDURE", "FUNCTION", "TRIGGER", "EVENT"];
const POSITION: &[&str] = &[
    ";", "BEGIN", "ATOMIC", "THEN", "ELSE", "DO", "LOOP", "REPEAT", ":",
];
/// Where `END` closes a non-`CASE` block: after its last statement's `;`, or right after the
/// opener of an empty body.
const END_AFTER: &[&str] = &[
    ";", "BEGIN", "ATOMIC", "THEN", "ELSE", "DO", "LOOP", "REPEAT",
];
const BODY: &[&str] = &[
    ")",
    "ROW",
    "DETERMINISTIC",
    "SQL",
    "DATA",
    "DEFINER",
    "INVOKER",
];

pub(super) static MYSQL: SplitProfile = SplitProfile {
    name: "mysql",
    family: BlockFamily::MySql,
    statement_starts: STARTS,
    prev_exceptions: PREV,
    head_continuations: HEADS,
    routine_head_skips: SKIPS,
    routine_kinds: ROUTINES,
    top_level_block_begins: &["NOT"],
    statement_position_after: POSITION,
    body_start_after: BODY,
    statement_openers: &[
        ("IF", BlockKind::If),
        ("CASE", BlockKind::Case),
        ("LOOP", BlockKind::Loop),
        ("WHILE", BlockKind::While),
        ("REPEAT", BlockKind::Repeat),
    ],
    end_followers: &[
        ("IF", BlockKind::If),
        ("CASE", BlockKind::Case),
        ("LOOP", BlockKind::Loop),
        ("WHILE", BlockKind::While),
        ("REPEAT", BlockKind::Repeat),
    ],
    name_after: NAME_AFTER,
    block_end_after: END_AFTER,
    compound_at_top: true,
    handler: Some(("DECLARE", "HANDLER")),
};

pub(super) static MARIADB: SplitProfile = SplitProfile {
    name: "mariadb",
    family: BlockFamily::MySql,
    statement_starts: STARTS,
    prev_exceptions: PREV,
    head_continuations: HEADS,
    routine_head_skips: SKIPS,
    routine_kinds: ROUTINES,
    top_level_block_begins: &["NOT"],
    statement_position_after: POSITION,
    body_start_after: BODY,
    statement_openers: &[
        ("IF", BlockKind::If),
        ("CASE", BlockKind::Case),
        ("LOOP", BlockKind::Loop),
        ("WHILE", BlockKind::While),
        ("REPEAT", BlockKind::Repeat),
        ("FOR", BlockKind::For),
    ],
    end_followers: &[
        ("IF", BlockKind::If),
        ("CASE", BlockKind::Case),
        ("LOOP", BlockKind::Loop),
        ("WHILE", BlockKind::While),
        ("REPEAT", BlockKind::Repeat),
        ("FOR", BlockKind::For),
    ],
    name_after: NAME_AFTER,
    block_end_after: END_AFTER,
    compound_at_top: true,
    handler: Some(("DECLARE", "HANDLER")),
};
