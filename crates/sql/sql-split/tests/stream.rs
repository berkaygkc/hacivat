//! The streaming splitter (SQL-SPLIT-05 file execution; Task 5): a script fed in chunks of any
//! size yields exactly the execution split of the whole script, in text order: each statement
//! (its strict text, offset and confidence), each client command, each uncertain boundary.
//! One statement is capped at `STREAM_STATEMENT_MAX`.

use proptest::prelude::*;
use sql_dialect::{ClientFlavor, DialectCtx, DialectId};
use sql_split::{
    Item, STREAM_STATEMENT_MAX, SplitMode, SplitStart, StreamItem, StreamSplitter, split_from,
};

const PIECES: &[&str] = &[
    "SELECT 1;\n",
    "SELECT a, b FROM t WHERE x = 'y';\n",
    "INSERT INTO t VALUES (1, 'a;b');\n",
    "SELECT 1\nSELECT 2\n",
    "-- a note\n",
    "/* block\n   comment */\n",
    "\n",
    "\n\n",
    "\r\n",
    "CREATE FUNCTION f() RETURNS int AS $$ SELECT 1; $$ LANGUAGE sql;\n",
    "CREATE FUNCTION g() RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT 1; SELECT 2; END;\n",
    "COPY t FROM STDIN;\n1\tdon't\n\\.\n",
    "COPY t FROM STDIN; SELECT 1;\n",
    "\\set x 'a\n",
    "\\if :x\n",
    "\\endif\n",
    "SELECT 1 \\g\n",
    "DELIMITER //\n",
    "DELIMITER ;\n",
    "CREATE PROCEDURE p() BEGIN SELECT 1; END//\n",
    "CREATE PROCEDURE q() BEGIN IF x THEN SELECT 1; END IF; END;\n",
    "/*!50000 SELECT 1 */;\n",
    "/*! , \"*/ x; -- \" */\n",
    "BEGIN;\n",
    "END;\n",
    "source f.sql\n",
    ".mode csv\n",
    "'",
    "$$",
    "/*",
    "\u{feff}",
    "İ",
    ";",
];

fn sql_doc() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(PIECES), 0..32).prop_map(|v| v.concat())
}

fn setups() -> impl Strategy<Value = (DialectId, ClientFlavor)> {
    prop::sample::select(vec![
        (DialectId::Postgres, ClientFlavor::Psql),
        (DialectId::Postgres, ClientFlavor::None),
        (DialectId::MySql, ClientFlavor::MySqlCli),
        (DialectId::MariaDb, ClientFlavor::MySqlCli),
        (DialectId::Sqlite, ClientFlavor::Sqlite3),
        (DialectId::Generic, ClientFlavor::None),
    ])
}

/// Chunk lengths, used in turn.
fn chunking() -> impl Strategy<Value = Vec<usize>> {
    prop::collection::vec(
        prop_oneof![3 => 1usize..8, 2 => 1usize..64, 1 => 1usize..4096],
        1..8,
    )
}

fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000)
}

/// The execution split of the whole text, as stream items in text order: by offset, then a
/// client command before a statement before an uncertain boundary.
fn expected(text: &str, ctx: &DialectCtx, flavor: ClientFlavor) -> Vec<StreamItem> {
    let split = split_from(
        text,
        ctx,
        flavor,
        SplitMode::Execution,
        &SplitStart::default(),
    );
    let mut out: Vec<(u64, u8, StreamItem)> = Vec::new();
    for c in &split.client {
        let offset = u64::from(u32::from(c.range.start()));
        out.push((
            offset,
            0,
            StreamItem::Client {
                text: text[c.range].to_owned(),
                offset,
                supported: c.supported,
            },
        ));
    }
    for item in &split.items {
        if let Item::Statement(s) = item {
            let offset = u64::from(u32::from(s.strict.start()));
            let it = if s.strict.len() > u32::try_from(STREAM_STATEMENT_MAX).unwrap().into() {
                StreamItem::TooLong { offset }
            } else {
                StreamItem::Statement {
                    text: text[s.strict].to_owned(),
                    offset,
                    confidence: s.confidence,
                }
            };
            out.push((offset, 1, it));
        }
    }
    for a in &split.ambiguous {
        let offset = u64::from(u32::from(a.at.start()));
        out.push((
            offset,
            2,
            StreamItem::Ambiguous {
                offset,
                reason: a.reason,
            },
        ));
    }
    out.sort_by_key(|(o, r, _)| (*o, *r));
    out.into_iter().map(|(_, _, it)| it).collect()
}

/// `text` fed in chunks of the given lengths (cut back to char boundaries), in turn.
fn streamed(text: &str, ctx: DialectCtx, flavor: ClientFlavor, lens: &[usize]) -> Vec<StreamItem> {
    let mut s = StreamSplitter::new(ctx, flavor);
    let mut out = Vec::new();
    let (mut at, mut k) = (0, 0);
    while at < text.len() {
        let mut end = (at + lens[k % lens.len()]).min(text.len());
        while !text.is_char_boundary(end) {
            end += 1;
        }
        out.extend(s.feed(&text[at..end]));
        at = end;
        k += 1;
    }
    out.extend(s.finish());
    out
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(), ..ProptestConfig::default() })]

    #[test]
    fn stream_splitter_equals_full_split_for_any_chunking(
        doc in sql_doc(),
        cuts in chunking(),
        (id, flavor) in setups(),
    ) {
        let ctx = DialectCtx::new(id, None);
        prop_assert_eq!(streamed(&doc, ctx, flavor, &cuts), expected(&doc, &ctx, flavor));
    }
}

fn script() -> String {
    let mut text = String::from("\\set x 1\n");
    for i in 0..400 {
        text.push_str(&format!(
            "-- statement {i}\nSELECT c{i}, 'İ;{i}' FROM t{i};\n\nCOPY t{i} FROM STDIN;\n{i}\ta\n\\.\n"
        ));
    }
    text
}

#[test]
fn one_byte_chunks_equal_64k_chunks() {
    let ctx = DialectCtx::new(DialectId::Postgres, None);
    let text = script();
    let one = streamed(&text, ctx, ClientFlavor::Psql, &[1]);
    let big = streamed(&text, ctx, ClientFlavor::Psql, &[64 << 10]);
    assert_eq!(one, big);
    assert_eq!(one, expected(&text, &ctx, ClientFlavor::Psql));
    assert_eq!(
        one.iter()
            .filter(|i| matches!(i, StreamItem::Statement { .. }))
            .count(),
        800
    );
}

#[test]
fn statement_over_64_mib_is_too_long() {
    let ctx = DialectCtx::new(DialectId::Postgres, None);
    let mut s = StreamSplitter::new(ctx, ClientFlavor::Psql);
    let mut out = s.feed("SELECT 1;\nSELECT '");
    let chunk = "a".repeat(1 << 20);
    for _ in 0..65 {
        out.extend(s.feed(&chunk));
    }
    out.extend(s.feed("';\nSELECT 2;\n"));
    out.extend(s.finish());
    assert_eq!(
        out.first(),
        Some(&StreamItem::Statement {
            text: "SELECT 1".into(),
            offset: 0,
            confidence: sql_split::Confidence::Exact,
        })
    );
    assert!(
        out.contains(&StreamItem::TooLong { offset: 10 }),
        "{:?}",
        out.iter().take(4).collect::<Vec<_>>()
    );
    // Nothing after a statement too long to hold is run: its end is never looked for.
    assert!(
        !out.iter()
            .skip(1)
            .any(|i| matches!(i, StreamItem::Statement { .. })),
        "{out:?}"
    );
}
