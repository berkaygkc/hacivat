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

use sql_dialect::ServerVersion;
use sql_split::AmbiguityReason;
use sql_split::stream::{STREAM_HELD_MAX, StreamPolicy, has_stop};
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

/// The MySQL/MariaDB texts of the corpora, each with a context and a client it runs in:
/// - the split cases and the MySQL and MariaDB profile fixtures;
/// - the `FX-SPLIT-MY` fixtures and the read corpora;
/// - `lex/mysql.toml` under each case's own modes, alone and between two statements, with
///   and without the mysql client (fix round 1, M1);
/// - the datagrip golden rows for MySQL/MariaDB.
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
    for c in sql_testkit::lex::load("lex/mysql.toml") {
        let ctx = c.ctx();
        if !is_mysql_family(ctx.id) || c.start.is_some() {
            continue;
        }
        let wrapped = format!("SELECT 1;\n{}\nSELECT 2;\n", c.input);
        for text in [c.input.clone(), wrapped] {
            for flavor in [ClientFlavor::MySqlCli, ClientFlavor::None] {
                out.push((c.id.clone(), text.clone(), ctx, flavor));
            }
        }
    }
    for file in ["caret", "choose", "sel"] {
        let path = sql_testkit::golden::corpus(&format!("datagrip/{file}.toml"));
        for c in sql_testkit::golden::load(&path) {
            let dialect = c.dialect.as_deref().unwrap_or("postgres");
            if !matches!(dialect, "mysql" | "mariadb") {
                continue;
            }
            let text = match (&c.input, &c.fixture) {
                (Some(i), None) => sql_testkit::golden::markers::extract(i).unwrap().text,
                (None, Some(f)) => fixture(f),
                _ => continue,
            };
            let ctx = sql_testkit::lex::dialect_ctx(dialect, c.version.as_deref());
            let flavor = match c.flavor.as_deref() {
                None => ClientFlavor::MySqlCli,
                f => sql_testkit::split::flavor(f),
            };
            out.push((c.id.clone(), text, ctx, flavor));
        }
    }
    out
}

const CHUNKINGS: [&[usize]; 6] = [&[1], &[2], &[7], &[64 << 10], &[3, 1, 17], &[5, 11, 2]];

#[test]
fn eager_equals_whole_split_when_blocks_match() {
    let (mut compared, mut stopped) = (0, 0);
    for (id, text, ctx, flavor) in mysql_corpus() {
        let want = expected(&text, &ctx, flavor);
        for lens in CHUNKINGS {
            if let Err(e) = check_eager(&text, ctx, flavor, lens) {
                panic!("{id} ({ctx:?}, {flavor:?}) in chunks of {lens:?}: {e}");
            }
        }
        if has_unknown_block(&want) {
            stopped += 1;
        } else {
            compared += 1;
        }
    }
    assert!(compared >= 600 && stopped >= 18, "{compared} / {stopped}");
    println!("HV-FACT m4b-t08 corpus_equal={compared} corpus_stop_checked={stopped}");
}

/// The invariants of the eager stream of `doc` in chunks of `cuts` (R-M4b-46, R-M4b-47):
/// - it is the same in one chunk;
/// - it gives out an item that refuses, or it equals the whole split; and it refuses exactly
///   when the whole split does (no input here reaches the held-text cap);
/// - off MySQL/MariaDB, or where the whole split has no block mismatch, it equals the whole
///   split and the default stream;
/// - with one, it ends with its only `UnknownBlock`, which the whole split reports too, and its
///   statements and client commands are a prefix of the whole split's before that point.
fn check_eager(
    doc: &str,
    ctx: DialectCtx,
    flavor: ClientFlavor,
    cuts: &[usize],
) -> Result<(), String> {
    let want = expected(doc, &ctx, flavor);
    let eager = streamed_with(StreamPolicy::Eager, doc, ctx, flavor, cuts);
    let one_chunk = streamed_with(StreamPolicy::Eager, doc, ctx, flavor, &[1 << 16]);
    let fail = |what: &str| Err(format!("{what}\neager {eager:?}\nwhole {want:?}"));
    if eager != one_chunk {
        return fail("depends on the chunking");
    }
    if !(has_stop(&eager) || eager == want) || has_stop(&eager) != has_stop(&want) {
        return fail("neither a stop nor the whole split");
    }
    if !is_mysql_family(ctx.id) || !has_unknown_block(&want) {
        if eager != want || eager != streamed(doc, ctx, flavor, cuts) {
            return fail("differs from the whole split");
        }
        return Ok(());
    }
    let Some(StreamItem::Ambiguous {
        offset: stop,
        reason: AmbiguityReason::UnknownBlock,
    }) = eager.last().cloned()
    else {
        return fail("no stop");
    };
    if has_unknown_block(&eager[..eager.len() - 1]) || !want.contains(&unknown_block(stop)) {
        return fail("not one stop the whole split reports");
    }
    let given = boundaries(&eager);
    let whole: Vec<_> = boundaries(&want)
        .into_iter()
        .filter(|(o, _)| *o < stop)
        .collect();
    if !whole.starts_with(&given) {
        return fail("a boundary moved");
    }
    Ok(())
}

/// MySQL pieces for the differential: blocks, stray and matched `END`s, `DELIMITER` in
/// comments, MariaDB ORACLE-mode blocks, lone `\r` line ends, and plain statements.
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
    "SET sql_mode='ORACLE';\n",
    "DECLARE v INT;\n",
    "BEGIN NULL; DELETE FROM t; END;\n",
    "BEGIN NOT ATOMIC SELECT 1; END;\n",
    "SELECT \"a\\\"b;\" FROM `t;`;\n",
    "SELECT 1;\r",
    "DELIMITER //\r",
    "-- c\r",
    "\r",
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

/// The contexts the eager property runs in: MySQL and MariaDB with each lexical mode that
/// changes quoting or blocks, two older servers, with and without the mysql client; and the
/// other families (fix round 1, M2).
fn eager_setups() -> impl Strategy<Value = (DialectCtx, ClientFlavor)> {
    let mut v = Vec::new();
    for id in [DialectId::MySql, DialectId::MariaDb] {
        let base = DialectCtx::new(id, None);
        let mut nbe = base;
        nbe.modes.no_backslash_escapes = true;
        let mut ansi = base;
        ansi.modes.ansi_quotes = true;
        let mut oracle = base;
        oracle.modes.mariadb_oracle_mode = true;
        let old = DialectCtx::new(
            id,
            Some(match id {
                DialectId::MySql => ServerVersion {
                    major: 5,
                    minor: 7,
                    patch: 0,
                },
                _ => ServerVersion {
                    major: 10,
                    minor: 3,
                    patch: 0,
                },
            }),
        );
        for ctx in [base, nbe, ansi, oracle, old] {
            for flavor in [ClientFlavor::MySqlCli, ClientFlavor::None] {
                v.push((ctx, flavor));
            }
        }
    }
    v.push((
        DialectCtx::new(DialectId::Postgres, None),
        ClientFlavor::Psql,
    ));
    v.push((
        DialectCtx::new(DialectId::Postgres, None),
        ClientFlavor::None,
    ));
    v.push((
        DialectCtx::new(DialectId::Sqlite, None),
        ClientFlavor::Sqlite3,
    ));
    v.push((
        DialectCtx::new(DialectId::Generic, None),
        ClientFlavor::None,
    ));
    prop::sample::select(v)
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

    /// The differential (`check_eager`) over generated scripts, every chunking, and the
    /// contexts of `eager_setups`.
    #[test]
    fn eager_stream_equals_default_stream_when_blocks_match(
        doc in prop_oneof![my_doc(), sql_doc()],
        cuts in chunking(),
        (ctx, flavor) in eager_setups(),
    ) {
        check_eager(&doc, ctx, flavor, &cuts).map_err(TestCaseError::fail)?;
    }
}

// ## m4b-t08-stream-hold (fix round 1): R-M4b-47's first pass, and the held-text cap.

/// R-M4b-47's first pass over `text` in chunks of `lens`: streams it, runs nothing, and says
/// whether the file is refused (any item refuses, checked on every `feed` and on `finish`).
fn pass_one_refuses(text: &str, ctx: DialectCtx, flavor: ClientFlavor, lens: &[usize]) -> bool {
    let mut s = StreamSplitter::with_policy(ctx, flavor, StreamPolicy::Eager);
    let mut refused = false;
    let (mut at, mut k) = (0, 0);
    while at < text.len() {
        let mut end = (at + lens[k % lens.len()]).min(text.len());
        while !text.is_char_boundary(end) {
            end += 1;
        }
        refused |= has_stop(&s.feed(&text[at..end]));
        assert!(!s.stopped() || refused, "a stop is always given out");
        at = end;
        k += 1;
    }
    refused | has_stop(&s.finish())
}

#[test]
fn a_misread_block_is_a_stop_or_the_whole_split() {
    // M3's own refusals for blocks the splitter cannot see (`R1-my-stray-end`, and MariaDB
    // `sql_mode=ORACLE` anonymous blocks and routines, set by the session or by the script):
    // in every chunking, the eager stream either gives out a stop or exactly the whole split,
    // and R-M4b-47's first pass refuses the file exactly when the whole split refuses it.
    let r1 = load(&corpus_file("mysql"))
        .into_iter()
        .find(|c| c.id == "R1-my-stray-end")
        .unwrap()
        .text();
    let r2 = load(&corpus_file("mysql"))
        .into_iter()
        .find(|c| c.id == "R2-stray-end-refuses-to-start")
        .unwrap()
        .text();
    let anonymous = "DECLARE\n  v INT;\nBEGIN\n  NULL;\n  DELETE FROM t;\nEND;\nSELECT 2;\n";
    let routine = "CREATE PROCEDURE p AS\nBEGIN\n  DELETE FROM t;\nEND;\nSELECT 2;\n";
    let by_script = |body: &str| format!("SET sql_mode='ORACLE';\n{body}");
    let mut oracle = DialectCtx::new(DialectId::MariaDb, None);
    oracle.modes.mariadb_oracle_mode = true;
    let maria = DialectCtx::new(DialectId::MariaDb, None);
    let cases: Vec<(&str, String, DialectCtx)> = vec![
        ("R1-my-stray-end", r1.clone(), mysql()),
        ("R1-my-stray-end (MariaDB)", r1, maria),
        ("R2-stray-end-refuses-to-start", r2.clone(), maria),
        ("R2 (ORACLE session)", r2, oracle),
        ("anonymous block, ORACLE session", anonymous.into(), oracle),
        (
            "anonymous block, ORACLE set by the script",
            by_script(anonymous),
            maria,
        ),
        ("routine, ORACLE session", routine.into(), oracle),
        (
            "routine, ORACLE set by the script",
            by_script(routine),
            maria,
        ),
    ];
    let mut lens: Vec<Vec<usize>> = (1..=24).map(|n| vec![n]).collect();
    lens.extend([vec![64 << 10], vec![3, 1, 17], vec![5, 11, 2], vec![2, 9]]);
    for (id, text, ctx) in &cases {
        let want = expected(text, ctx, ClientFlavor::MySqlCli);
        for l in &lens {
            let eager = streamed_with(StreamPolicy::Eager, text, *ctx, ClientFlavor::MySqlCli, l);
            assert!(
                has_stop(&eager) || eager == want,
                "{id} in chunks of {l:?}: {eager:?}\nwhole {want:?}"
            );
            assert_eq!(
                pass_one_refuses(text, *ctx, ClientFlavor::MySqlCli, l),
                has_stop(&want),
                "{id} in chunks of {l:?}"
            );
            check_eager(text, *ctx, ClientFlavor::MySqlCli, l)
                .unwrap_or_else(|e| panic!("{id} in chunks of {l:?}: {e}"));
        }
    }
    // M3 refuses the stray-`END` case: so does the first pass, though the eager stream gives
    // out `DELETE FROM t` before its stop.
    let r1 = &cases[0].1;
    assert!(has_stop(&expected(r1, &mysql(), ClientFlavor::MySqlCli)));
    assert!(pass_one_refuses(r1, mysql(), ClientFlavor::MySqlCli, &[1]));
    // A script with matching blocks passes the first pass and streams as the whole split.
    let clean =
        "DELIMITER //\nCREATE PROCEDURE p() BEGIN DELETE FROM t; END//\nDELIMITER ;\nSELECT 1;\n";
    assert!(!pass_one_refuses(
        clean,
        mysql(),
        ClientFlavor::MySqlCli,
        &[1]
    ));
}

#[test]
fn refusing_items() {
    let exact = sql_split::Confidence::Exact;
    let heuristic = sql_split::Confidence::Heuristic;
    let st = |confidence| StreamItem::Statement {
        text: "SELECT 1".into(),
        offset: 0,
        confidence,
    };
    assert!(!st(exact).refuses());
    assert!(st(heuristic).refuses());
    assert!(
        !StreamItem::Client {
            text: "DELIMITER //".into(),
            offset: 0,
            supported: true
        }
        .refuses()
    );
    assert!(unknown_block(0).refuses());
    assert!(StreamItem::TooLong { offset: 0 }.refuses());
    assert!(StreamItem::NoBoundary { offset: 0 }.refuses());
    assert!(!has_stop(&[st(exact)]));
}

/// What `feed_lines` saw: the items other than `Exact` statements, the number of those, and
/// the peak `held_len()` after a feed.
struct Fed {
    other: Vec<StreamItem>,
    statements: usize,
    peak: usize,
}

/// Feeds `total` bytes made by `line(i)` in 64 KiB chunks under `Eager` (MySQL), until the end
/// or a stop.
fn feed_lines(total: usize, mut line: impl FnMut(u64) -> String) -> Fed {
    const CHUNK: usize = 64 << 10;
    let mut s = eager();
    let mut fed = Fed {
        other: Vec::new(),
        statements: 0,
        peak: 0,
    };
    let take = |items: Vec<StreamItem>, fed: &mut Fed| {
        for it in items {
            if matches!(it, StreamItem::Statement { .. }) && !it.refuses() {
                fed.statements += 1;
            } else {
                fed.other.push(it);
            }
        }
    };
    let mut pending = String::new();
    let (mut made, mut i) = (0usize, 0u64);
    loop {
        while pending.len() < CHUNK && made < total {
            let l = line(i);
            made += l.len();
            pending.push_str(&l);
            i += 1;
        }
        if pending.is_empty() {
            break;
        }
        let mut cut = CHUNK.min(pending.len());
        while !pending.is_char_boundary(cut) {
            cut += 1;
        }
        let rest = pending.split_off(cut);
        take(s.feed(&pending), &mut fed);
        fed.peak = fed.peak.max(s.held_len());
        pending = rest;
        if s.stopped() {
            break;
        }
    }
    take(s.finish(), &mut fed);
    fed
}

/// Past the cap: more than `STREAM_HELD_MAX` bytes.
const PAST_CAP: usize = STREAM_HELD_MAX + (2 << 20);

/// The most held after a feed when the cap stops the stream: the cap plus one chunk.
const CAP_AND_CHUNK: usize = STREAM_HELD_MAX + (64 << 10);

#[test]
fn held_text_is_capped_on_one_line() {
    // Statements with no line start between them never settle: one `NoBoundary` at the
    // stream's start and nothing else, once `STREAM_HELD_ITEMS_MAX` of them are held (about
    // 1 MB here, split again when the held text doubles), long before the byte cap.
    let fed = feed_lines(PAST_CAP, |i| format!("SELECT {i}; "));
    assert_eq!(fed.other, [StreamItem::NoBoundary { offset: 0 }]);
    assert_eq!(fed.statements, 0);
    assert!(fed.peak <= 4 << 20, "held {}", fed.peak);
    let short = fed.peak;
    // Few long statements on one line reach the byte cap instead.
    let long = "x".repeat(4_000);
    let fed = feed_lines(PAST_CAP, |i| format!("SELECT {i}, '{long}'; "));
    assert_eq!(fed.other, [StreamItem::NoBoundary { offset: 0 }]);
    assert!(fed.peak <= CAP_AND_CHUNK, "held {}", fed.peak);
    println!(
        "HV-FACT m4b-t08 cap_one_line_peak_held short={short} long={}",
        fed.peak
    );
}

#[test]
fn held_text_settles_on_long_lines_of_short_statements() {
    // A thousand statements per line: every line start is found, however many items stand on
    // the line before it, so only about one line is held.
    let fed = feed_lines(PAST_CAP, |i| {
        let mut l: String = (0..1_000).map(|k| format!("SELECT {k}; ")).collect();
        l.push_str(&format!("-- {i}\n"));
        l
    });
    assert_eq!(fed.other, []);
    assert!(fed.statements > 1_000_000, "{}", fed.statements);
    assert!(fed.peak <= 12_000 + (64 << 10), "held {}", fed.peak);
    println!("HV-FACT m4b-t08 cap_long_lines_peak_held={}", fed.peak);
}

#[test]
fn held_text_settles_on_lone_cr_line_ends() {
    // Classic Mac line ends: a lone `\r` starts a line for the anchor (checked against the
    // whole pass), so the stream settles and equals the whole split.
    let fed = feed_lines(PAST_CAP, |i| format!("INSERT INTO t VALUES ({i});\r"));
    assert_eq!(fed.other, []);
    assert!(fed.statements > 500_000, "{}", fed.statements);
    assert!(fed.peak <= 1_024 + (64 << 10), "held {}", fed.peak);
    println!("HV-FACT m4b-t08 cap_cr_only_peak_held={}", fed.peak);
    let small: String = (0..2_000)
        .map(|i| format!("/* n {i} */\rINSERT INTO t VALUES ({i});\r"))
        .collect();
    let mut s = eager();
    s.feed(&small[..small.len() / 2]);
    assert!(s.held_len() < 1_000, "held {}", s.held_len());
    for lens in CHUNKINGS {
        assert_eq!(
            eager_mysql(&small, lens),
            expected(&small, &mysql(), ClientFlavor::MySqlCli),
            "chunks of {lens:?}"
        );
    }
}

#[test]
fn held_text_is_capped_in_an_open_comment() {
    // An unterminated `/*` after a statement: no later line start is an item's, the cap stops
    // the stream, and the end of the file is never waited for.
    let mut first = true;
    let fed = feed_lines(PAST_CAP, |_| {
        if std::mem::take(&mut first) {
            "SELECT 1;\n/*".into()
        } else {
            "a comment line;\n".into()
        }
    });
    assert_eq!(fed.other, [StreamItem::NoBoundary { offset: 0 }]);
    assert!(fed.peak <= CAP_AND_CHUNK, "held {}", fed.peak);
    println!("HV-FACT m4b-t08 cap_open_comment_peak_held={}", fed.peak);
}

#[test]
fn held_text_is_capped_in_comments_only() {
    let fed = feed_lines(PAST_CAP, |i| format!("-- comment {i}\n"));
    assert_eq!(fed.other, [StreamItem::NoBoundary { offset: 0 }]);
    assert_eq!(fed.statements, 0);
    assert!(fed.peak <= CAP_AND_CHUNK, "held {}", fed.peak);
    println!("HV-FACT m4b-t08 cap_comments_only_peak_held={}", fed.peak);
}

#[test]
fn a_statement_past_the_cap_is_too_long() {
    // One open statement past `STREAM_HELD_MAX` (not only past `STREAM_STATEMENT_MAX`): what
    // precedes it is given out, then `TooLong` at its start, and nothing more.
    let mut first = true;
    let fed = feed_lines(PAST_CAP, |_| {
        if std::mem::take(&mut first) {
            "SELECT 1;\nINSERT INTO t VALUES ('".into()
        } else {
            "x".repeat(1_000)
        }
    });
    assert_eq!(fed.other, [StreamItem::TooLong { offset: 10 }]);
    assert_eq!(fed.statements, 1);
    assert!(fed.peak <= CAP_AND_CHUNK, "held {}", fed.peak);
    // A complete statement past the cap fed at once is `TooLong` too, and stops the stream.
    let mut s = eager();
    let big = format!(
        "SELECT '{}';\nSELECT 2;\nSELECT 3;\n",
        "y".repeat(STREAM_HELD_MAX)
    );
    let mut got = s.feed(&big);
    got.extend(s.finish());
    assert_eq!(got, [StreamItem::TooLong { offset: 0 }]);
}
