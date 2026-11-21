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

// ## m4b-t08-stream-hold
// The eager policy (M4b Task 8, R-M4b-46): MySQL/MariaDB statements stream out without
// holding the whole script, and the stream stops at the first block mismatch. The default
// policy (`StreamSplitter::new`, `StreamPolicy::Exact`) keeps the equality property above.

use sql_split::AmbiguityReason;
use sql_split::stream::StreamPolicy;
use sql_testkit::lex::corpus_path;
use sql_testkit::split::{corpus_file, fixture, load, profile_fixture_file};

fn mysql() -> DialectCtx {
    DialectCtx::new(DialectId::MySql, None)
}

fn eager() -> StreamSplitter {
    StreamSplitter::with_policy(mysql(), ClientFlavor::MySqlCli, StreamPolicy::Eager)
}

/// `text` fed in chunks of the given lengths under `policy` (as `streamed`).
fn streamed_with(
    policy: StreamPolicy,
    text: &str,
    ctx: DialectCtx,
    flavor: ClientFlavor,
    lens: &[usize],
) -> Vec<StreamItem> {
    let mut s = StreamSplitter::with_policy(ctx, flavor, policy);
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

fn eager_mysql(text: &str, lens: &[usize]) -> Vec<StreamItem> {
    streamed_with(
        StreamPolicy::Eager,
        text,
        mysql(),
        ClientFlavor::MySqlCli,
        lens,
    )
}

fn statements(items: &[StreamItem]) -> Vec<&str> {
    items
        .iter()
        .filter_map(|i| match i {
            StreamItem::Statement { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn stmt(text: &str, offset: u64) -> StreamItem {
    StreamItem::Statement {
        text: text.into(),
        offset,
        confidence: sql_split::Confidence::Exact,
    }
}

fn unknown_block(offset: u64) -> StreamItem {
    StreamItem::Ambiguous {
        offset,
        reason: AmbiguityReason::UnknownBlock,
    }
}

fn is_unknown_block(i: &StreamItem) -> bool {
    matches!(
        i,
        StreamItem::Ambiguous {
            reason: AmbiguityReason::UnknownBlock,
            ..
        }
    )
}

fn has_unknown_block(items: &[StreamItem]) -> bool {
    items.iter().any(is_unknown_block)
}

fn is_mysql_family(id: DialectId) -> bool {
    matches!(id, DialectId::MySql | DialectId::MariaDb)
}

const THREE_INSERTS: &str =
    "INSERT INTO t VALUES (1);\nINSERT INTO t VALUES (2);\nINSERT INTO t VALUES (3);\n";

#[test]
fn mysql_statements_stream_without_blocks_when_eager() {
    let mut s = eager();
    let out = s.feed(THREE_INSERTS);
    assert!(statements(&out).len() >= 2, "{out:?}");
    assert_eq!(
        statements(&out)[..2],
        ["INSERT INTO t VALUES (1)", "INSERT INTO t VALUES (2)"]
    );
    // The default policy still holds them: a later stray `END` could refuse them.
    let mut exact = StreamSplitter::new(mysql(), ClientFlavor::MySqlCli);
    assert_eq!(exact.feed(THREE_INSERTS), []);
    assert_eq!(exact.held_len(), THREE_INSERTS.len());
}

#[test]
fn exact_policy_is_unchanged() {
    // `StreamPolicy::Exact` is `new`: the equality property on every chunking, through the
    // explicit constructor, and `new` itself unchanged on the same inputs.
    assert_eq!(StreamPolicy::default(), StreamPolicy::Exact);
    let text = script();
    let ctx = DialectCtx::new(DialectId::Postgres, None);
    for lens in [&[1usize][..], &[7], &[64 << 10], &[3, 1, 17]] {
        assert_eq!(
            streamed_with(StreamPolicy::Exact, &text, ctx, ClientFlavor::Psql, lens),
            expected(&text, &ctx, ClientFlavor::Psql)
        );
    }
    let mut runner = proptest::test_runner::TestRunner::new(ProptestConfig {
        cases: cases().min(2_000),
        ..ProptestConfig::default()
    });
    runner
        .run(
            &(sql_doc(), chunking(), setups()),
            |(doc, cuts, (id, flavor))| {
                let ctx = DialectCtx::new(id, None);
                let want = expected(&doc, &ctx, flavor);
                let exact = streamed_with(StreamPolicy::Exact, &doc, ctx, flavor, &cuts);
                prop_assert_eq!(&exact, &want);
                prop_assert_eq!(&streamed(&doc, ctx, flavor, &cuts), &want);
                Ok(())
            },
        )
        .unwrap();
}

#[test]
fn mysql_delimiter_block_holds_only_the_block() {
    let mut s = eager();
    let head = "SELECT 1;\nSELECT 2;\nDELIMITER //\n";
    let open = "CREATE PROCEDURE p() BEGIN\n  SELECT 1;\n  SELECT 2;\n";
    let mut out = s.feed(head);
    out.extend(s.feed(open));
    // Everything before the open block is out; only the block is held.
    assert_eq!(statements(&out), ["SELECT 1", "SELECT 2"]);
    assert!(
        out.iter().any(
            |i| matches!(i, StreamItem::Client { text, .. } if text.starts_with("DELIMITER //"))
        ),
        "{out:?}"
    );
    assert_eq!(s.held_len(), open.len());
    let tail = "END//\nDELIMITER ;\nSELECT 3;\nSELECT 4;\n";
    out.extend(s.feed(tail));
    // Once the block is closed it is given out too: only the last statement, which may still
    // grow, is held.
    assert_eq!(s.held_len(), "SELECT 4;\n".len());
    out.extend(s.finish());
    let whole = format!("{head}{open}{tail}");
    assert_eq!(out, expected(&whole, &mysql(), ClientFlavor::MySqlCli));
}

#[test]
fn stray_end_after_given_out_statements_stops_the_stream() {
    let text = "SELECT 1; SELECT 2; END; SELECT 3;";
    assert_eq!(
        eager_mysql(text, &[1]),
        [stmt("SELECT 1", 0), stmt("SELECT 2", 10), unknown_block(20)]
    );
    // On separate lines the statements before `END` are given out before `END` is fed (in
    // one-byte chunks); the result is the same in one chunk.
    let lines = "SELECT 1;\nSELECT 2;\nEND;\nSELECT 3;\n";
    for lens in [&[1usize][..], &[1 << 16]] {
        assert_eq!(
            eager_mysql(lines, lens),
            [stmt("SELECT 1", 0), stmt("SELECT 2", 10), unknown_block(20)]
        );
    }
    // After a block, the `END` that refuses back to it stops the stream as well.
    let after_block = "DELIMITER //\nCREATE PROCEDURE p() BEGIN SELECT 1; END//\nDELIMITER ;\n\
                       SELECT 2;\nEND;\nDELETE FROM t;\n";
    let out = eager_mysql(after_block, &[1]);
    assert_eq!(
        statements(&out),
        ["CREATE PROCEDURE p() BEGIN SELECT 1; END", "SELECT 2"]
    );
    let end_at = u64::try_from(after_block.find("END;\n").unwrap()).unwrap();
    assert_eq!(out.last(), Some(&unknown_block(end_at)));
    // Nothing is given out after the stop, whatever is fed later.
    let mut s = eager();
    let mut got = s.feed(lines);
    got.extend(s.feed("SELECT 4;\nSELECT 5;\n"));
    assert_eq!(s.held_len(), 0);
    got.extend(s.finish());
    assert_eq!(
        got,
        [stmt("SELECT 1", 0), stmt("SELECT 2", 10), unknown_block(20)]
    );
}

#[test]
fn a_mismatch_inside_a_statement_stops_at_that_statement() {
    // `END IF` closing a `LOOP`: the whole split refuses that one statement and goes on; the
    // eager stream stops there and never gives out the statement or anything after it.
    let text =
        "SELECT 1;\nCREATE PROCEDURE q() BEGIN LOOP SELECT 1; END IF; END;\nDELETE FROM t;\n";
    let whole = expected(text, &mysql(), ClientFlavor::MySqlCli);
    assert!(has_unknown_block(&whole), "{whole:?}");
    for lens in [&[1usize][..], &[5], &[1 << 16]] {
        let out = eager_mysql(text, lens);
        assert_eq!(statements(&out), ["SELECT 1"], "{out:?}");
        assert!(out.last().is_some_and(is_unknown_block), "{out:?}");
        assert_eq!(out.iter().filter(|i| is_unknown_block(i)).count(), 1);
    }
}

/// 1 KiB `INSERT` lines, cut into 64 KiB chunks, until `total` bytes are fed; `each` sees the
/// splitter and the items of every chunk.
fn feed_dump(
    s: &mut StreamSplitter,
    total: usize,
    mut each: impl FnMut(&StreamSplitter, Vec<StreamItem>),
) {
    const LINE: usize = 1024;
    const CHUNK: usize = 64 << 10;
    let mut pending = String::with_capacity(CHUNK + LINE);
    let (mut fed, mut made, mut i) = (0usize, 0usize, 0u64);
    while fed < total {
        while pending.len() < CHUNK && made < total {
            let head = format!("INSERT INTO t VALUES ({i}, '");
            let pad = LINE - head.len() - "');\n".len();
            pending.push_str(&head);
            pending.extend(std::iter::repeat_n('x', pad));
            pending.push_str("');\n");
            made += LINE;
            i += 1;
        }
        let rest = pending.split_off(CHUNK.min(pending.len()));
        fed += pending.len();
        let out = s.feed(&pending);
        each(s, out);
        pending = rest;
    }
}

#[test]
fn peak_memory_is_bounded_on_a_150mb_dump() {
    let total = 150 << 20;
    let mut s = eager();
    let (mut peak, mut n) = (0usize, 0usize);
    feed_dump(&mut s, total, |s, out| {
        peak = peak.max(s.held_len());
        n += out.len();
    });
    n += s.finish().len();
    // The longest statement (1 KiB) plus one chunk (64 KiB): the anchor is the last item.
    assert!(peak <= 1024 + (64 << 10), "held {peak} bytes");
    assert_eq!(n, total / 1024);
    println!("HV-FACT m4b-t08 eager_150mb_peak_held_bytes={peak} statements={n}");
}

/// Peak RSS under each policy, measured from outside (`/usr/bin/time -l` on this test
/// binary): `HV_T08_POLICY=exact|eager HV_T08_MB=<n> <bin> --ignored --exact rss_probe`.
#[test]
#[ignore = "perf"]
fn rss_probe() {
    let policy = match std::env::var("HV_T08_POLICY").as_deref() {
        Ok("exact") => StreamPolicy::Exact,
        _ => StreamPolicy::Eager,
    };
    let mb: usize = std::env::var("HV_T08_MB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(150);
    let mut s = StreamSplitter::with_policy(mysql(), ClientFlavor::MySqlCli, policy);
    let (mut peak, mut n) = (0usize, 0usize);
    feed_dump(&mut s, mb << 20, |s, out| {
        peak = peak.max(s.held_len());
        n += out.len();
    });
    n += s.finish().len();
    println!(
        "HV-FACT m4b-t08 rss_probe policy={policy:?} mb={mb} peak_held_bytes={peak} items={n}"
    );
}

/// The MySQL/MariaDB texts of the corpora (the split cases, the profile fixtures, the
/// `FX-SPLIT-MY` fixtures and the read corpora), each with a context it runs in.
fn mysql_corpus() -> Vec<(String, String, DialectCtx, ClientFlavor)> {
    let mut out = Vec::new();
    let cases = load(&corpus_file("mysql"))
        .into_iter()
        .chain(load(&profile_fixture_file("mysql")))
        .chain(load(&profile_fixture_file("mariadb")));
    for c in cases {
        let flavor = sql_testkit::split::flavor(c.flavor.as_deref());
        let text = c.text();
        for ctx in c.contexts() {
            if is_mysql_family(ctx.id) {
                out.push((c.id.clone(), text.clone(), ctx, flavor));
            }
        }
    }
    for k in 1..=5 {
        let id = format!("FX-SPLIT-MY-0{k}");
        for dialect in [DialectId::MySql, DialectId::MariaDb] {
            out.push((
                id.clone(),
                fixture(&id),
                DialectCtx::new(dialect, None),
                ClientFlavor::MySqlCli,
            ));
        }
    }
    for (rel, dialect) in [
        ("read/mysql.sql", DialectId::MySql),
        ("read/mariadb.sql", DialectId::MariaDb),
    ] {
        let text = std::fs::read_to_string(corpus_path(rel)).unwrap();
        out.push((
            rel.into(),
            text,
            DialectCtx::new(dialect, None),
            ClientFlavor::MySqlCli,
        ));
    }
    out
}

#[test]
fn eager_equals_whole_split_when_blocks_match() {
    let (mut compared, mut skipped) = (0, 0);
    for (id, text, ctx, flavor) in mysql_corpus() {
        let want = expected(&text, &ctx, flavor);
        if has_unknown_block(&want) {
            skipped += 1;
            continue;
        }
        for lens in [&[1usize][..], &[7], &[64 << 10], &[3, 1, 17]] {
            assert_eq!(
                streamed_with(StreamPolicy::Eager, &text, ctx, flavor, lens),
                want,
                "{id} ({:?}) in chunks of {lens:?}",
                ctx.id
            );
        }
        compared += 1;
    }
    assert!(compared >= 50, "compared {compared}, skipped {skipped}");
    println!("HV-FACT m4b-t08 corpus_compared={compared} corpus_skipped_unknown_block={skipped}");
}

/// MySQL pieces for the differential: blocks, stray and matched `END`s, `DELIMITER` in
/// comments, and plain statements.
const MY_PIECES: &[&str] = &[
    "INSERT INTO t VALUES (1);\n",
    "SELECT 1; SELECT 2;\n",
    "DELIMITER //\n",
    "DELIMITER $$\n",
    "DELIMITER ;\n",
    "-- DELIMITER //\n",
    "/* DELIMITER $$ */\n",
    "# END;\n",
    "CREATE PROCEDURE p() BEGIN SELECT 1; END//\n",
    "CREATE PROCEDURE r() BEGIN SELECT 1; END$$\n",
    "CREATE PROCEDURE q() BEGIN IF x THEN SELECT 1; END IF; END;\n",
    "CREATE TRIGGER tr BEFORE INSERT ON t FOR EACH ROW BEGIN SET NEW.a = 1; END;\n",
    "BEGIN;\n",
    "END;\n",
    "END IF;\n",
    "COMMIT;\n",
    "/*!50000 SELECT 1 */;\n",
    "SELECT 'a;b' FROM t;\n",
    "\n",
    "//\n",
    "$$\n",
    "'",
    ";",
];

fn my_doc() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![3 => prop::sample::select(MY_PIECES), 1 => prop::sample::select(PIECES)],
        0..32,
    )
    .prop_map(|v| v.concat())
}

#[test]
fn the_differential_reaches_matched_and_mismatched_scripts() {
    // The MySQL documents the property draws: both its branches are reached often, so it
    // proves the stop as well as the equality.
    use proptest::strategy::ValueTree;
    let mut runner = proptest::test_runner::TestRunner::deterministic();
    let strategy = my_doc();
    let (mut matched, mut mismatched) = (0, 0);
    for _ in 0..1_000 {
        let doc = strategy.new_tree(&mut runner).unwrap().current();
        let given_out = eager_mysql(&doc, &[1 << 16]);
        if has_unknown_block(&expected(&doc, &mysql(), ClientFlavor::MySqlCli)) {
            mismatched += 1;
            assert!(given_out.last().is_some_and(is_unknown_block), "{doc:?}");
        } else {
            matched += 1;
        }
    }
    assert!(
        matched >= 100 && mismatched >= 100,
        "{matched} / {mismatched}"
    );
    println!("HV-FACT m4b-t08 differential_draws matched={matched} mismatched={mismatched}");
}

/// Statements and client commands as (offset, text): the boundaries a stream gives out.
fn boundaries(items: &[StreamItem]) -> Vec<(u64, String)> {
    items
        .iter()
        .filter_map(|i| match i {
            StreamItem::Statement { text, offset, .. }
            | StreamItem::Client { text, offset, .. } => Some((*offset, text.clone())),
            _ => None,
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(), ..ProptestConfig::default() })]

    /// The differential: on every input whose whole split has no block mismatch, the eager
    /// stream equals the default stream and the whole split, whatever the chunking. With a
    /// mismatch (MySQL/MariaDB) it stops there: it ends with its one `UnknownBlock`, gives out
    /// nothing after it, never moves a boundary (its statements and client commands are the
    /// whole split's before that point), and is the same for every chunking.
    #[test]
    fn eager_stream_equals_default_stream_when_blocks_match(
        doc in prop_oneof![my_doc(), sql_doc()],
        cuts in chunking(),
        (id, flavor) in setups(),
    ) {
        let ctx = DialectCtx::new(id, None);
        let want = expected(&doc, &ctx, flavor);
        let eager = streamed_with(StreamPolicy::Eager, &doc, ctx, flavor, &cuts);
        let one_chunk = streamed_with(StreamPolicy::Eager, &doc, ctx, flavor, &[1 << 16]);
        prop_assert_eq!(&eager, &one_chunk);
        if !is_mysql_family(id) || !has_unknown_block(&want) {
            prop_assert_eq!(&eager, &want);
            prop_assert_eq!(&eager, &streamed(&doc, ctx, flavor, &cuts));
        } else {
            let stop = match eager.last() {
                Some(StreamItem::Ambiguous { offset, reason: AmbiguityReason::UnknownBlock }) => {
                    *offset
                }
                _ => return Err(TestCaseError::fail(format!("no stop: {eager:?}"))),
            };
            prop_assert!(!has_unknown_block(&eager[..eager.len() - 1]), "{:?}", eager);
            // The stop is a mismatch the whole split reports too.
            prop_assert!(want.contains(&unknown_block(stop)), "stop {} whole {:?}", stop, want);
            let given = boundaries(&eager);
            let whole: Vec<_> = boundaries(&want).into_iter().filter(|(o, _)| *o < stop).collect();
            prop_assert!(whole.starts_with(&given), "eager {:?}\nwhole {:?}", given, whole);
        }
    }
}
