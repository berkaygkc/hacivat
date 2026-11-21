//! Incremental relexing (SQL-LEX-03, R-M3-65; Task 5): `relex` restarts at the last exact
//! state before an edit and splices until the new tokens re-synchronise, and the result always
//! equals a full lex of the new text. Also the two lexer follow-ups: raw `x'…'`/`b'…'` inside
//! a comment body the server runs (ledger 200), and a client command's argument ends at the end
//! of its line (ledger 274).

#[path = "support/edits.rs"]
mod edits;
#[path = "support/gen_sql.rs"]
mod gen_sql;

use std::time::Instant;

use proptest::prelude::*;
use sql_dialect::{ClientFlavor, DialectCtx, DialectId, LexDialect, LexModes};
use sql_lexer::{LexState, Lexed, RelexDelta, TokenFlags, TokenKind, lex, lex_with_client, relex};
use sql_testkit::lex::{dialect_ctx, kinds_of, profile};
use sql_text::{TextEdit, TextRange, TextSize};

// ---- generators --------------------------------------------------------------------------------

/// Whole lines and pieces: statements, `$$` bodies, `COPY` data, psql meta-commands, MySQL
/// executable comments and hints, client delimiters, comments, quotes that open and close.
const PIECES: &[&str] = &[
    "SELECT 1;\n",
    "SELECT a, b FROM t WHERE x = 'y';\n",
    "select 'it''s';\n",
    "INSERT INTO t VALUES (1, 'a;b');\n",
    "-- a note\n",
    "/* block\n   comment */\n",
    "/* nested /* inner */ still */\n",
    "\n",
    "\n\n",
    "\r\n",
    "  ",
    "CREATE FUNCTION f() RETURNS int AS $$ SELECT 1; $$ LANGUAGE sql;\n",
    "DO $body$\nBEGIN\n  PERFORM 1;\nEND\n$body$;\n",
    "$$",
    "$q$ x; $q$",
    "COPY t FROM STDIN;\n1\tdon't\n\\.\n",
    "COPY t FROM STDIN;\n",
    "\\.\n",
    "\\set x 'a\n",
    "\\echo don't\n",
    "\\if :x\n",
    "\\endif\n",
    "\\g\n",
    "SELECT 1 \\gset p_\n",
    "\\;",
    "DELIMITER //\n",
    "DELIMITER ;\n",
    "CREATE PROCEDURE p() BEGIN SELECT 1; END//\n",
    "/*!50000 SELECT x'\\' */;\n",
    "/*! , \"*/ x; -- \" */\n",
    "/*+ SET_VAR(a=1) */",
    "/*M! SELECT 1 */;\n",
    "SELECT E'a\\'b';\n",
    "SELECT \"q;\" FROM `t;`;\n",
    "# hash comment\n",
    "source f.sql\n",
    ".mode csv\n",
    "BEGIN;\n",
    "END;\n",
    "'",
    "\"",
    "`",
    "/*",
    "*/",
    "--",
    "x'0F'",
    "b'01'",
    "\u{feff}",
    "İ",
    "ı",
    ";",
    "\\",
];

fn sql_doc() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(PIECES), 0..40).prop_map(|v| v.concat())
}

/// What an edit inserts.
const INSERTS: &[&str] = &[
    "",
    "x",
    " ",
    "\n",
    "\r",
    "\r\n",
    "'",
    "\"",
    "`",
    "$",
    "$$",
    ";",
    "-",
    "--",
    "/*",
    "*/",
    "*",
    "/",
    "\\",
    "\\.",
    "!",
    "+",
    "#",
    "e",
    "x'",
    "b'",
    "\u{feff}",
    "İ",
    "ç",
    "SELECT 2;\n",
    "END",
    "BEGIN",
    "DELIMITER $$\n",
    "\\set y 1\n",
    "COPY t FROM STDIN;\n",
    "/*!",
    "/*+",
    "/*M!",
    "-- c\n",
    "\n\n",
];

/// An edit of `doc`: where (any char boundary), how much it deletes, and what it inserts.
fn edit_in(doc: &str) -> impl Strategy<Value = TextEdit> + use<> {
    let doc = doc.to_owned();
    (
        any::<prop::sample::Index>(),
        prop_oneof![4 => 0usize..4, 2 => 0usize..32, 1 => 0usize..4096],
        prop::collection::vec(prop::sample::select(INSERTS), 0..3),
    )
        .prop_map(move |(at, del, ins)| {
            let a = floor(&doc, at.index(doc.len() + 1));
            let b = floor(&doc, (a + del).min(doc.len()));
            TextEdit {
                delete: range(a, b),
                insert: ins.concat(),
            }
        })
}

fn setups() -> impl Strategy<Value = (DialectCtx, ClientFlavor)> {
    prop::sample::select(vec![
        (dialect_ctx("postgres", None), ClientFlavor::Psql),
        (dialect_ctx("postgres", None), ClientFlavor::None),
        (dialect_ctx("postgres", Some("14.0.0")), ClientFlavor::Psql),
        (dialect_ctx("mysql", Some("8.4.0")), ClientFlavor::MySqlCli),
        (dialect_ctx("mysql", Some("8.0.36")), ClientFlavor::None),
        (
            dialect_ctx("mariadb", Some("11.4.0")),
            ClientFlavor::MySqlCli,
        ),
        (dialect_ctx("sqlite", Some("3.51.0")), ClientFlavor::Sqlite3),
        (dialect_ctx("generic", None), ClientFlavor::Psql),
    ])
}

fn modes_strategy() -> impl Strategy<Value = Option<LexModes>> {
    prop_oneof![
        3 => Just(None),
        1 => prop::array::uniform6(any::<bool>()).prop_map(|b| Some(LexModes {
            standard_conforming_strings: b[0],
            ansi_quotes: b[1],
            no_backslash_escapes: b[2],
            pipes_as_concat: b[3],
            high_not_precedence: b[4],
            mariadb_oracle_mode: b[5],
        })),
    ]
}

/// The proptest case count: `PROPTEST_CASES` when set (nightly: 100,000), else 10,000 (PR).
fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000)
}

// ---- helpers -----------------------------------------------------------------------------------

fn floor(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn at(n: usize) -> TextSize {
    TextSize::from(u32::try_from(n).unwrap())
}

fn range(a: usize, b: usize) -> TextRange {
    TextRange::new(at(a), at(b))
}

fn applied(doc: &str, edit: &TextEdit) -> String {
    let mut s = doc.to_owned();
    edit.apply(&mut s).unwrap();
    s
}

fn lex_doc(text: &str, p: &'static LexDialect, m: LexModes, client: ClientFlavor) -> Lexed {
    lex_with_client(text, p, m, &LexState::Initial, client)
}

/// `doc` lexed, then relexed after `edit`, and the relex's answer.
fn relexed(
    doc: &str,
    edit: &TextEdit,
    p: &'static LexDialect,
    m: LexModes,
    client: ClientFlavor,
) -> (Lexed, RelexDelta) {
    let mut l = lex_doc(doc, p, m, client);
    let new_text = applied(doc, edit);
    let delta = relex(&mut l, &new_text.as_str(), edit, p, m);
    (l, delta)
}

/// Every checkpoint of `l` is a line start where lexing the rest from its state gives the
/// tokens `l` has there.
fn checkpoints_are_exact(
    text: &str,
    l: &Lexed,
    p: &'static LexDialect,
    m: LexModes,
) -> Result<(), String> {
    let mut probe = 0;
    while probe <= text.len() {
        let (cp, state) = l.checkpoint_before(at(probe));
        let c = usize::from(cp);
        if c > probe {
            return Err(format!("checkpoint {c} after {probe}"));
        }
        if c > 0 && text.as_bytes()[c - 1] != b'\n' {
            return Err(format!("checkpoint {c} is not a line start"));
        }
        let tail = lex_with_client(&text[c..], p, m, &state, l.client());
        let from = l.token_at(cp).unwrap_or(l.len());
        if c < text.len() && usize::from(l.range(from).start()) != c {
            return Err(format!("checkpoint {c} is not a token start"));
        }
        let want: Vec<(TokenKind, TextRange, TokenFlags)> = (from..l.len())
            .map(|i| (l.kind(i), l.range(i), l.flags(i)))
            .collect();
        let got: Vec<(TokenKind, TextRange, TokenFlags)> = (0..tail.len())
            .map(|i| (tail.kind(i), tail.range(i) + cp, tail.flags(i)))
            .collect();
        if want != got {
            return Err(format!("lexing from checkpoint {c} differs"));
        }
        probe += 997;
    }
    // And every stored checkpoint, not only those the probes land on (fix round 1, C1).
    edits::checkpoints_hold(text, l, p, m)
}

/// The lexer setups, by index (the seeded property picks one).
fn setup_at(i: usize) -> (DialectCtx, ClientFlavor) {
    let all = [
        (dialect_ctx("postgres", None), ClientFlavor::Psql),
        (dialect_ctx("postgres", None), ClientFlavor::None),
        (dialect_ctx("postgres", Some("14.0.0")), ClientFlavor::Psql),
        (dialect_ctx("mysql", Some("8.4.0")), ClientFlavor::MySqlCli),
        (dialect_ctx("mysql", Some("8.0.36")), ClientFlavor::None),
        (
            dialect_ctx("mariadb", Some("11.4.0")),
            ClientFlavor::MySqlCli,
        ),
        (dialect_ctx("sqlite", Some("3.51.0")), ClientFlavor::Sqlite3),
        (dialect_ctx("generic", None), ClientFlavor::Psql),
    ];
    all[i % all.len()]
}

/// One seeded case of the run property: a script (up to 16 KiB), then 1–8 edits, each biased
/// to end at a checkpoint or a line start; after each, the relexed lexing equals a full lex and
/// every checkpoint holds.
fn relex_run(seed: u64) -> Result<(), String> {
    let mut r = edits::Rng::new(seed);
    let (ctx, client) = setup_at(r.below(8));
    let p = profile(&ctx);
    let m = ctx.modes;
    let mut text = edits::doc(&mut r);
    let mut l = lex_doc(&text, p, m, client);
    for step in 0..1 + r.below(8) {
        let marks: Vec<usize> = l.checkpoints().iter().map(|c| usize::from(*c)).collect();
        let e = edits::edit(&mut r, &text, &marks);
        e.apply(&mut text).map_err(|x| format!("{x:?}"))?;
        relex(&mut l, &text.as_str(), &e, p, m);
        if l != lex_doc(&text, p, m, client) {
            return Err(format!(
                "seed {seed}, edit {step} ({e:?}): relex differs from a full lex"
            ));
        }
        edits::checkpoints_hold(&text, &l, p, m)
            .map_err(|x| format!("seed {seed}, edit {step}: {x}"))?;
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: edits::cases(10_000), ..ProptestConfig::default() })]

    /// Runs of edits over scripts of every size (fix round 1, I1); 10,000 cases in PR (each is up
    /// to eight edits on up to 16 KiB), `PROPTEST_CASES` otherwise.
    #[test]
    fn incremental_relex_over_edit_runs(seed in any::<u64>()) {
        relex_run(seed).map_err(TestCaseError::fail)?;
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(), ..ProptestConfig::default() })]

    #[test]
    fn incremental_relex_equals_full_relex(
        (doc, edit) in sql_doc().prop_flat_map(|d| { let e = edit_in(&d); (Just(d), e) }),
        (ctx, client) in setups(),
        modes in modes_strategy(),
    ) {
        let p = profile(&ctx);
        let m = modes.unwrap_or(ctx.modes);
        let new_text = applied(&doc, &edit);
        let (got, delta) = relexed(&doc, &edit, p, m, client);
        let full = lex_doc(&new_text, p, m, client);
        prop_assert_eq!(&got, &full);
        prop_assert!(delta.changed.end <= got.len());
        // Every token outside `changed` is one the old lexing had (moved by the edit).
        checkpoints_are_exact(&new_text, &got, p, m).map_err(TestCaseError::fail)?;
    }
}

// ---- the plan's cases --------------------------------------------------------------------------

fn pg() -> DialectCtx {
    DialectCtx::new(DialectId::Postgres, None)
}

fn mysql() -> DialectCtx {
    DialectCtx::new(DialectId::MySql, None)
}

/// `n` lines of short, distinct statements.
fn lines(n: usize) -> String {
    (0..n)
        .map(|i| format!("SELECT c{i} FROM t{i};\n"))
        .collect()
}

/// `d` lexed (PostgreSQL), then relexed after inserting `ins` at `at` (moved back to a char
/// boundary); the relexed lexing equals a full lex.
fn relex_at(d: &str, at_byte: usize, ins: &str) -> RelexDelta {
    let ctx = pg();
    let p = profile(&ctx);
    let a = floor(d, at_byte);
    let edit = TextEdit {
        delete: range(a, a),
        insert: ins.to_owned(),
    };
    let (l, delta) = relexed(d, &edit, p, ctx.modes, ClientFlavor::Psql);
    assert_eq!(
        l,
        lex_doc(&applied(d, &edit), p, ctx.modes, ClientFlavor::Psql)
    );
    delta
}

fn kinds(ctx: DialectCtx, text: &str) -> Vec<TokenKind> {
    kinds_of(&lex(text, profile(&ctx), ctx.modes, &LexState::Initial))
}

fn lex_pg_psql(text: &str) -> Lexed {
    let ctx = pg();
    lex_with_client(
        text,
        profile(&ctx),
        ctx.modes,
        &LexState::Initial,
        ClientFlavor::Psql,
    )
}

fn kind_at(l: &Lexed, byte: u32) -> TokenKind {
    l.kind(l.token_at(TextSize::from(byte)).unwrap())
}

/// A text whose `line` starts at the first line start at or after 2048 (a checkpoint), after a
/// line ending in `prev`; and where `line` starts (fix round 1, C1: the reviewer's repros).
fn at_a_checkpoint(prev: &str, line: &str) -> (String, usize) {
    let mut t = String::new();
    while t.len() < 2048 - 40 {
        t.push_str("SELECT 1;\n");
    }
    t.push_str(&" ".repeat(2048 - t.len() - prev.len() - 1));
    t.push_str(prev);
    t.push('\n');
    let c = t.len();
    assert_eq!(c, 2048);
    t.push_str(line);
    t.push_str("\nSELECT 2;\nSELECT 3;\n");
    (t, c)
}

#[test]
fn a_checkpoint_after_a_joined_line_is_dropped() {
    // Join the line before a checkpoint (delete its line feed), then type what makes the token
    // before the old checkpoint look past it: `U` + `&'`, `-` + `- `, `$` + `fn$`.
    let cases = [
        (
            "postgres",
            ClientFlavor::None,
            "SELECT U",
            "&x FROM t;",
            1,
            "'",
        ),
        (
            "postgres",
            ClientFlavor::None,
            "SELECT $",
            "fn x; SELECT 4;",
            3,
            "$",
        ),
        (
            "mysql",
            ClientFlavor::None,
            "SELECT 1 -",
            "-x; SELECT 5;",
            1,
            " ",
        ),
        (
            "mysql",
            ClientFlavor::MySqlCli,
            "SELECT 1 -",
            "-x; SELECT 5; END;",
            1,
            " ",
        ),
    ];
    for (dialect, client, prev, line, offset, typed) in cases {
        let ctx = dialect_ctx(dialect, None);
        let p = profile(&ctx);
        let (mut text, c) = at_a_checkpoint(prev, line);
        let mut l = lex_doc(&text, p, ctx.modes, client);
        assert!(
            l.checkpoints().contains(&at(c)),
            "{dialect} {line}: no checkpoint at {c}"
        );
        for e in [
            TextEdit {
                delete: range(c - 1, c),
                insert: String::new(),
            },
            TextEdit {
                delete: range(c - 1 + offset, c - 1 + offset),
                insert: typed.into(),
            },
        ] {
            e.apply(&mut text).unwrap();
            relex(&mut l, &text.as_str(), &e, p, ctx.modes);
            assert_eq!(
                l,
                lex_doc(&text, p, ctx.modes, client),
                "{dialect} {line}: {e:?}"
            );
            edits::checkpoints_hold(&text, &l, p, ctx.modes).unwrap();
        }
    }
}

#[test]
fn typing_inside_one_statement_relexes_one_line() {
    let d = lines(50_000);
    let delta = relex_at(&d, 1_000_000, "x");
    assert!(!delta.full);
    assert!(delta.changed.len() < 200, "{delta:?}");
}

#[test]
fn opening_quote_at_offset_zero_relexes_the_tail() {
    let d = lines(2_000);
    let delta = relex_at(&d, 0, "'");
    assert!(delta.full, "nothing after the quote re-synchronises");
    let ctx = pg();
    let l = lex_doc(
        &format!("'{d}"),
        profile(&ctx),
        ctx.modes,
        ClientFlavor::Psql,
    );
    assert_eq!(delta.changed, 0..l.len());
}

#[test]
fn delimiter_change_forces_a_tail_relex() {
    let ctx = mysql();
    let p = profile(&ctx);
    let body: String = (0..500).map(|i| format!("SELECT {i}//\n")).collect();
    let doc = format!("DELIMITER //\n{body}");
    // `//` → `$$` on the `DELIMITER` line.
    let edit = TextEdit {
        delete: range(10, 12),
        insert: "$$".into(),
    };
    let (l, delta) = relexed(&doc, &edit, p, ctx.modes, ClientFlavor::MySqlCli);
    assert_eq!(
        l,
        lex_doc(&applied(&doc, &edit), p, ctx.modes, ClientFlavor::MySqlCli)
    );
    assert!(delta.full);
    assert_eq!(delta.changed.end, l.len());
    // Typing in a statement below it does not.
    let edit = TextEdit {
        delete: range(20, 20),
        insert: "0".into(),
    };
    let (_, delta) = relexed(&doc, &edit, p, ctx.modes, ClientFlavor::MySqlCli);
    assert!(!delta.full);
}

#[test]
fn checkpoint_before_is_exact_at_line_starts() {
    let ctx = pg();
    let p = profile(&ctx);
    // Multi-line strings, comments and dollar quotes between plain lines.
    let block = "SELECT 1;\nSELECT 'a\nb';\n/* x\ny */\nSELECT $$\n;\n$$;\n-- c\n";
    let text = block.repeat(400);
    let l = lex_doc(&text, p, ctx.modes, ClientFlavor::Psql);
    checkpoints_are_exact(&text, &l, p, ctx.modes).unwrap();
    // Checkpoints exist (about every 2 KiB), not only the start.
    let (cp, state) = l.checkpoint_before(at(text.len()));
    assert!(usize::from(cp) + 4096 > text.len(), "{cp:?}");
    assert_eq!(state, LexState::Initial);
    // A lexing that starts inside a construct answers its start state before any checkpoint.
    let inside = lex_with_client(
        "a\n';\nSELECT 1;\n",
        p,
        ctx.modes,
        &LexState::InString {
            rule: sql_lexer::StringRule::Standard,
            quote: b'\'',
            kind: TokenKind::String,
        },
        ClientFlavor::None,
    );
    assert!(matches!(
        inside.checkpoint_before(at(3)),
        (cp, LexState::InString { .. }) if cp == at(0)
    ));
}

#[test]
fn hex_literal_inside_a_comment_body_is_not_a_string() {
    use TokenKind::*;
    assert_eq!(
        kinds(mysql(), "/* x'a */ SELECT 1"),
        [BlockComment, Whitespace, Word, Whitespace, Number]
    );
    // In a body the server runs, `x'…'` and `b'…'` are raw: a backslash does not escape the
    // closing quote, so the comment ends at its `*/` in both readings.
    for text in [
        "/*!50000 SELECT x'\\' */ SELECT 1",
        "/*! SELECT b'\\' */ SELECT 1",
        "/*+ SET_VAR(x'\\') */ SELECT 1",
        "/*M! SELECT X'\\' */ SELECT 1",
    ] {
        let ctx = if text.starts_with("/*M!") {
            DialectCtx::new(DialectId::MariaDb, None)
        } else {
            mysql()
        };
        let l = lex(text, profile(&ctx), ctx.modes, &LexState::Initial);
        assert!(l.errors().is_empty(), "{text}: {:?}", l.errors());
        assert!(!l.flags(0).contains(TokenFlags::ERROR), "{text}");
    }
    // A prefix needs a word start: `ax'…'` is a word and an escaping string.
    let ctx = mysql();
    let l = lex(
        "/*! SELECT ax'\\' */ SELECT 1",
        profile(&ctx),
        ctx.modes,
        &LexState::Initial,
    );
    assert!(l.flags(0).contains(TokenFlags::ERROR));
}

#[test]
fn client_command_argument_ends_at_end_of_line() {
    let l = lex_pg_psql("\\set x 'a\nSELECT 1;");
    assert_eq!(kind_at(&l, 10), TokenKind::Word); // byte 10 = 'S'
    assert_eq!(kind_at(&l, 0), TokenKind::ClientCommand);
    assert_eq!(l.range(0), range(0, 9));
    // `\\` ends a psql command inside its line; a quoted one does not.
    let l = lex_pg_psql("\\echo a \\\\ SELECT 'x\n");
    assert_eq!(l.range(0), range(0, 10));
    assert_eq!(kind_at(&l, 11), TokenKind::Word);
    let l = lex_pg_psql("\\echo '\\\\' don't\nSELECT 1;");
    assert_eq!(l.range(0), range(0, 16));
    assert_eq!(kind_at(&l, 17), TokenKind::Word);
    // The mysql client: `\g` is two bytes, `\. file` runs to the end of its line.
    let ctx = mysql();
    let l = lex_with_client(
        "SELECT 1\\g\n\\. it's.sql\nSELECT 2;",
        profile(&ctx),
        ctx.modes,
        &LexState::Initial,
        ClientFlavor::MySqlCli,
    );
    assert_eq!(kinds_of(&l)[3], TokenKind::ClientCommand);
    assert_eq!(l.range(3), range(8, 10));
    assert_eq!(kind_at(&l, 11), TokenKind::ClientCommand);
    assert_eq!(l.range(l.token_at(at(11)).unwrap()), range(11, 22));
    assert_eq!(kind_at(&l, 23), TokenKind::Word);
    // Without a client flavor a backslash is still an unknown byte.
    assert_eq!(
        kind_at(
            &lex(
                "\\set x 'a\nSELECT 1;",
                profile(&pg()),
                pg().modes,
                &LexState::Initial,
            ),
            10
        ),
        TokenKind::String
    );
}

// ---- performance (R-M3-9; P9: `HACIVAT_PERF=1`, release) ------------------------------------

#[test]
fn gen_matches_the_xtask_shape() {
    let a = gen_sql::gen_sql(50_000);
    assert_eq!(a.lines().count(), 50_000);
    assert!((1_800_000..=2_200_000).contains(&a.len()), "{}", a.len());
}

#[test]
#[ignore = "perf"]
fn relex_typing_edit_p95_under_2ms() {
    if !gen_sql::perf_enabled() {
        return;
    }
    // The relex half of R-M3-9 (the resplit half is `sql-split`'s test of the same name): the
    // gen-sql 50k-line fixture, 1,000 single-character edits at random lines (a character typed
    // or one deleted), each on the fixture (undone, unmeasured, before the next).
    let ctx = pg();
    let p = profile(&ctx);
    let fixture = gen_sql::gen_sql(50_000);
    let mut text = fixture.clone();
    let mut l = lex_doc(&text, p, ctx.modes, ClientFlavor::Psql);
    let mut rng = gen_sql::Positions::new(7);
    let mut times = Vec::with_capacity(1_000);
    for i in 0..1_000 {
        let a = floor(&fixture, rng.below(fixture.len()));
        let (edit, undo) = if i % 2 == 0 {
            (
                TextEdit {
                    delete: range(a, a),
                    insert: "x".into(),
                },
                TextEdit {
                    delete: range(a, a + 1),
                    insert: String::new(),
                },
            )
        } else {
            let b = (a + 1..=fixture.len())
                .find(|i| fixture.is_char_boundary(*i))
                .unwrap_or(a);
            (
                TextEdit {
                    delete: range(a, b),
                    insert: String::new(),
                },
                TextEdit {
                    delete: range(a, a),
                    insert: fixture[a..b].to_owned(),
                },
            )
        };
        edit.apply(&mut text).unwrap();
        let start = Instant::now();
        relex(&mut l, &text.as_str(), &edit, p, ctx.modes);
        times.push(start.elapsed());
        undo.apply(&mut text).unwrap();
        relex(&mut l, &text.as_str(), &undo, p, ctx.modes);
    }
    assert_eq!(text, fixture);
    assert_eq!(l, lex_doc(&text, p, ctx.modes, ClientFlavor::Psql));
    let p95 = gen_sql::p95(&mut times);
    println!("relex p95 over 1,000 typing edits on 50k lines: {p95:?}");
    assert!(p95.as_micros() <= 2_000, "{p95:?}");
}

#[test]
#[ignore = "perf"]
fn worst_case_relex_under_10ms() {
    if !gen_sql::perf_enabled() {
        return;
    }
    // An opening quote at offset 0 of 1 MiB: nothing re-synchronises, the tail is relexed once.
    let ctx = pg();
    let p = profile(&ctx);
    let mut text = gen_sql::gen_sql(50_000);
    text.truncate(floor(&text, 1 << 20));
    let mut times = Vec::new();
    for _ in 0..20 {
        let mut l = lex_doc(&text, p, ctx.modes, ClientFlavor::Psql);
        let edit = TextEdit {
            delete: range(0, 0),
            insert: "'".into(),
        };
        let new_text = applied(&text, &edit);
        let start = Instant::now();
        let delta = relex(&mut l, &new_text.as_str(), &edit, p, ctx.modes);
        times.push(start.elapsed());
        assert!(delta.full);
    }
    let p95 = gen_sql::p95(&mut times);
    println!("worst-case relex (quote at 0 of 1 MiB): {p95:?}");
    assert!(p95.as_micros() <= 10_000, "{p95:?}");
}
