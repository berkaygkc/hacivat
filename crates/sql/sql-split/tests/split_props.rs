//! Splitter properties (Task 8) over generated SQL-ish text in every dialect and client flavor:
//! items never overlap, every significant token belongs to an item, a certain unit split alone
//! is itself (cut idempotence), the two modes agree on every exact statement (R-M3-8), and,
//! against PostgreSQL's own grammar (libpg_query) on generated scripts, every certain unit is
//! a statement the server sees. MySQL comment bodies: the server-side end the splitter reads
//! agrees with the lexer's `AmbiguousCommentEnd` flag and with one whole lexing of the body.
//! Lexing in chunks of any length, cut at line starts or inside lines, gives the same split.

use proptest::prelude::*;
use sql_dialect::{ClientFlavor, DialectCtx, DialectId, lex_profile};
use sql_lexer::{LexState, TokenFlags, TokenKind, lex};
use sql_split::splitter::{comment_refusal_end_at, comment_server_end, split_chunked};
use sql_split::{Confidence, Item, Split, SplitMode, SplitStart, SqlDocument, split_from};
use sql_text::{TextRange, TextSize};

const FRAGMENTS: &[&str] = &[
    " ",
    " ",
    "\n",
    "\n",
    "\n\n",
    "\t",
    ";",
    ";",
    "SELECT",
    "select",
    "1",
    "a",
    "t",
    ",",
    "(",
    ")",
    "'a;b'",
    "'",
    "\"x;y\"",
    "\"",
    "$$",
    "$$;$$",
    "$q$ x; $q$",
    "$a_tag_longer_than_any_cut_margin$ x; $a_tag_longer_than_any_cut_margin$",
    "$a_tag_longer_than_any_cut_margin",
    "-- c\n",
    "-- ;\n",
    "/*",
    "*/",
    "/* ; */",
    "/*! ; */",
    "/*!50000 x */",
    "/*!50000 '*/' */",
    "/*+ h */",
    "/*+ '*/",
    "# c\n",
    "`q;`",
    "BEGIN",
    "END",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "IF",
    "LOOP",
    "REPEAT",
    "WHILE",
    "FOR",
    "DO",
    "CREATE",
    "PROCEDURE",
    "FUNCTION",
    "TRIGGER",
    "EVENT",
    "ATOMIC",
    "NOT",
    "INSERT",
    "INTO",
    "UPDATE",
    "DELETE",
    "FROM",
    "WITH",
    "AS",
    "UNION",
    "\\",
    "\\g",
    "\\G",
    "\\c",
    "\\;",
    "\\echo x\n",
    "\nDELIMITER $$\n",
    "\nDELIMITER ;\n",
    "\nDELIMITER //\n",
    "//",
    "\nCOPY t FROM STDIN;\n",
    "\n\\.\n",
    "\n.mode csv\n",
    "source f\n",
    "İ",
    "ı",
    "é",
    "x\r\n",
    "t.end",
    "AS end",
    "AS case",
    "SET end = 1",
    "SELECT begin",
    "x > end",
    "BEGIN ATOMIC",
    "CREATE FUNCTION f() RETURNS int LANGUAGE sql",
    "FOR EACH ROW",
    "UNTIL",
    "DECLARE c HANDLER FOR",
    "\\if x\n",
    "\\endif\n",
    "STDIN",
    "TEMP",
    "DEFINER",
    "=",
    ":",
    "l:",
];

fn any_sql() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(FRAGMENTS), 0..48).prop_map(|v| v.join(" "))
}

fn dialects() -> impl Strategy<Value = (DialectId, ClientFlavor)> {
    prop::sample::select(vec![
        (DialectId::Postgres, ClientFlavor::None),
        (DialectId::Postgres, ClientFlavor::Psql),
        (DialectId::MySql, ClientFlavor::None),
        (DialectId::MySql, ClientFlavor::MySqlCli),
        (DialectId::MariaDb, ClientFlavor::MySqlCli),
        (DialectId::Sqlite, ClientFlavor::Sqlite3),
        (DialectId::Sqlite, ClientFlavor::None),
        (DialectId::Generic, ClientFlavor::None),
    ])
}

fn split_in(s: &str, id: DialectId, flavor: ClientFlavor, mode: SplitMode) -> Split {
    split_from(
        s,
        &DialectCtx::new(id, None),
        flavor,
        mode,
        &SplitStart::default(),
    )
}

fn item_range(i: &Item) -> (TextRange, TextRange) {
    match i {
        Item::Statement(s) => (s.strict, s.smart),
        Item::Empty { range } => (*range, *range),
    }
}

/// Items are in text order and neither their strict nor their smart ranges overlap; a strict
/// range sits inside its smart range; client segments overlap no strict range... unless the
/// statement goes on around the command.
fn disjoint(split: &Split) -> Result<(), String> {
    let mut last_strict = 0u32;
    let mut last_smart = 0u32;
    for (n, i) in split.items.iter().enumerate() {
        let (strict, smart) = item_range(i);
        if !smart.contains_range(strict) {
            return Err(format!(
                "item {n}: strict {strict:?} outside smart {smart:?}"
            ));
        }
        if u32::from(strict.start()) < last_strict || u32::from(smart.start()) < last_smart {
            return Err(format!("item {n} overlaps the one before"));
        }
        if strict.is_empty() && matches!(i, Item::Statement(_)) {
            return Err(format!("item {n}: empty statement"));
        }
        last_strict = strict.end().into();
        last_smart = smart.end().into();
    }
    Ok(())
}

/// Every significant token of the text (lexed whole: no client flavor) is inside one item: a
/// statement's strict range or terminator, or an empty item.
fn covers_significant(split: &Split, s: &str, id: DialectId) -> Result<(), String> {
    let ctx = DialectCtx::new(id, None);
    let lexed = lex(s, lex_profile(&ctx).unwrap(), ctx.modes, &LexState::Initial);
    for i in 0..lexed.len() {
        if lexed.is_trivia(i) {
            continue;
        }
        let r = lexed.range(i);
        let inside = split.items.iter().any(|it| match it {
            Item::Statement(s) => {
                s.strict.contains_range(r) || s.terminator.is_some_and(|t| t.contains_range(r))
            }
            Item::Empty { range } => range.contains_range(r),
        });
        if !inside {
            return Err(format!("token {:?} at {r:?} is in no item", &s[r]));
        }
    }
    Ok(())
}

/// R-M3-8: an editor statement is `Exact` exactly when an execution statement with the same
/// strict range is certain (runnable); and an execution statement is `Exact` exactly when no
/// ambiguity overlaps it.
fn exact_items_agree(editor: &Split, exec: &Split) -> Result<(), String> {
    for st in exec.statements() {
        let overlapped = exec.ambiguities_in(st.strict).next().is_some();
        if (st.confidence == Confidence::Exact) == overlapped || exec.is_certain(st) == overlapped {
            return Err(format!("execution {:?}: confidence vs overlap", st.strict));
        }
    }
    let exact_editor: Vec<TextRange> = editor
        .statements()
        .filter(|s| s.confidence == Confidence::Exact)
        .map(|s| s.strict)
        .collect();
    let certain_exec: Vec<TextRange> = exec.runnable().map(|s| s.strict).collect();
    if exact_editor == certain_exec {
        Ok(())
    } else {
        Err(format!(
            "editor exact {exact_editor:?} vs execution certain {certain_exec:?}"
        ))
    }
}

/// Cut idempotence (T8 review I3): every certain execution statement, split again on its own
/// from the lexer state and delimiter at its start, is exactly one certain statement with the
/// same text. A boundary that moved when the context is taken away was not certain.
fn certain_units_split_alone(s: &str, id: DialectId, flavor: ClientFlavor) -> Result<(), String> {
    let ctx = DialectCtx::new(id, None);
    let snap = SqlDocument::new(s.to_string(), ctx, flavor).snapshot();
    let exec = snap.execution_split();
    for st in exec.runnable() {
        let piece = &s[st.strict];
        let start = snap.split_start_at(st.strict.start());
        let alone = split_from(piece, &ctx, flavor, SplitMode::Execution, &start);
        let whole = TextRange::up_to(TextSize::from(u32::try_from(piece.len()).unwrap()));
        let items: Vec<TextRange> = alone.statements().map(|x| x.strict).collect();
        if items != [whole] || alone.runnable().count() != 1 {
            return Err(format!(
                "certain {piece:?} splits alone into {:?} (certain: {})",
                items.iter().map(|r| &piece[*r]).collect::<Vec<_>>(),
                alone.runnable().count()
            ));
        }
    }
    Ok(())
}

/// A generated PostgreSQL script that libpg_query parses: plain statements, transactions,
/// rules, and SQL-standard function bodies whose statements use `end`, `case` and `begin` as
/// names (`t.end`, `AS end`, `AS case`) beside real `CASE … END` expressions.
fn pg_script() -> impl Strategy<Value = String> {
    let expr = prop::sample::select(vec![
        "1",
        "a",
        "t.end",
        "t.begin",
        "1 AS end",
        "a AS case",
        "CASE WHEN a > 0 THEN 1 ELSE 2 END",
        "CASE a WHEN 1 THEN t.end END AS end",
    ]);
    let body_stmt = prop_oneof![
        expr.clone().prop_map(|e| format!("SELECT {e} FROM t")),
        Just("DELETE FROM t WHERE t.end < 1".to_string()),
        Just("UPDATE t SET a = CASE WHEN a THEN 1 END".to_string()),
        Just("INSERT INTO t (a) VALUES (1)".to_string()),
    ];
    let function = prop::collection::vec(body_stmt.clone(), 0..4).prop_map(|b| {
        let body: String = b.iter().map(|x| format!("{x}; ")).collect();
        format!("CREATE FUNCTION f() RETURNS SETOF int LANGUAGE sql BEGIN ATOMIC {body}END")
    });
    let stmt = prop_oneof![
        body_stmt,
        function,
        Just("BEGIN".to_string()),
        Just("COMMIT".to_string()),
        Just("END".to_string()),
        Just("CREATE RULE r AS ON INSERT TO t DO ALSO (SELECT 1; DELETE FROM u)".to_string()),
        Just("SELECT $$a;b$$, 'c;d' /* ; */".to_string()),
    ];
    prop::collection::vec(stmt, 1..8)
        .prop_map(|v| v.iter().map(|x| format!("{x};\n")).collect::<String>())
}

/// MySQL text around executable comments and hints whose bodies quote or comment out `*/`.
fn mysql_comment_text() -> impl Strategy<Value = String> {
    let part = prop::sample::select(vec![
        "/*!50000 ",
        "/*!",
        "/*+ ",
        "/*M!",
        "*/",
        " ",
        "'",
        "\"",
        "`",
        "-- ",
        "#",
        "\n",
        "\\",
        "x",
        ";",
        "/* ",
        "1",
        "'*/'",
        "\"*/\"",
        "b",
    ]);
    prop::collection::vec(part, 0..24).prop_map(|v| v.concat())
}

/// Where a server ends the comment at `at`, read from one whole lexing of its body: the first
/// `*/` in code, none after an inner executable comment (the splitter reads it in prefixes).
fn server_end_whole(s: &str, at: usize, ctx: &DialectCtx) -> Option<usize> {
    let open = if s[at..].starts_with("/*M!") { 4 } else { 3 };
    let from = (at + open).min(s.len());
    let body = &s[from..];
    let b = body.as_bytes();
    let l = lex(
        body,
        lex_profile(ctx).unwrap(),
        ctx.modes,
        &LexState::Initial,
    );
    for i in 0..l.len() {
        let st = usize::from(l.range(i).start());
        match l.kind(i) {
            TokenKind::ExecutableComment => return None,
            TokenKind::Op if b[st] == b'*' && b.get(st + 1) == Some(&b'/') => {
                return Some(from + st + 2);
            }
            _ => {}
        }
    }
    None
}

/// At least 768 cases; `PROPTEST_CASES` raises it (the default config reads it).
fn config() -> ProptestConfig {
    let d = ProptestConfig::default();
    ProptestConfig {
        cases: d.cases.max(768),
        ..d
    }
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn items_never_overlap_and_cover_every_significant_token(s in any_sql(), (id, flavor) in dialects()) {
        for mode in [SplitMode::Editor, SplitMode::Execution] {
            let sp = split_in(&s, id, flavor, mode);
            prop_assert!(disjoint(&sp).is_ok(), "{:?} {mode:?} {s:?}: {:?}", id, disjoint(&sp));
        }
        let exec = split_in(&s, id, ClientFlavor::None, SplitMode::Execution);
        let covered = covers_significant(&exec, &s, id);
        prop_assert!(covered.is_ok(), "{id:?} {s:?}: {covered:?}");
    }

    #[test]
    fn certain_units_are_cut_idempotent(s in any_sql(), (id, flavor) in dialects()) {
        let r = certain_units_split_alone(&s, id, flavor);
        prop_assert!(r.is_ok(), "{id:?} {flavor:?} {s:?}: {r:?}");
    }

    #[test]
    fn every_certain_pg_unit_is_a_statement_of_pg_query(script in pg_script()) {
        let oracle = sql_oracle_pg::split_with_parser(&script);
        prop_assert!(oracle.is_ok(), "the generator wrote SQL pg_query rejects: {script:?} {oracle:?}");
        let oracle = oracle.unwrap();
        let ctx = DialectCtx::new(DialectId::Postgres, None);
        let exec = split_from(&script, &ctx, ClientFlavor::Psql, SplitMode::Execution, &SplitStart::default());
        for st in exec.runnable() {
            prop_assert!(oracle.contains(&st.strict), "certain {:?} is no pg_query statement in {script:?}", &script[st.strict]);
        }
        if exec.ambiguous.is_empty() {
            let mine: Vec<TextRange> = exec.statements().map(|x| x.strict).collect();
            prop_assert_eq!(mine, oracle, "{:?}", script);
        }
    }

    #[test]
    fn the_server_side_comment_end_agrees_with_the_lexer(s in mysql_comment_text(), maria in any::<bool>()) {
        let id = if maria { DialectId::MariaDb } else { DialectId::MySql };
        let ctx = DialectCtx::new(id, None);
        let profile = lex_profile(&ctx).unwrap();
        let lexed = lex(&s, profile, ctx.modes, &LexState::Initial);
        let amb: Vec<TextSize> = lexed
            .errors()
            .iter()
            .filter(|e| e.code == sql_lexer::LexErrorCode::AmbiguousCommentEnd)
            .map(|e| e.range.start())
            .collect();
        let split = split_from(&s, &ctx, ClientFlavor::None, SplitMode::Execution, &SplitStart::default());
        for i in 0..lexed.len() {
            let kind = lexed.kind(i);
            if !matches!(kind, TokenKind::ExecutableComment | TokenKind::HintComment)
                || lexed.flags(i).contains(TokenFlags::UNTERMINATED)
            {
                continue;
            }
            let r = lexed.range(i);
            let (at, len) = (usize::from(r.start()), usize::from(r.len()));
            let end = comment_server_end(&s, &ctx, at, len, None);
            // Read in prefixes of any length, the end is the one a whole lexing gives.
            let whole = server_end_whole(&s, at, &ctx);
            prop_assert_eq!(end, whole, "{:?} in {:?}", &s[r], s);
            for first in [1, 5, 16, 33] {
                prop_assert_eq!(comment_server_end(&s, &ctx, at, len, Some(first)), whole, "{:?} from {}", s, first);
            }
            let flagged = amb.contains(&r.start());
            let mine = end != Some(usize::from(r.end()));
            // Both readings agree wherever the body holds no hex or bit literal, which the
            // lexer's own server-side reading takes for escaping strings.
            let body = lex(&s[r][3.min(len)..], profile, ctx.modes, &LexState::Initial);
            let raw_literal = (0..body.len())
                .any(|j| matches!(body.kind(j), TokenKind::HexString | TokenKind::BitString));
            if !raw_literal {
                prop_assert_eq!(flagged, mine, "{:?} in {:?}: end {:?}", &s[r], s, end);
            }
            // Either reading refuses, through the later of the two ends; with no certain server
            // end, or none that differs from the token's, through the end of the text.
            if flagged || mine {
                let reach = comment_refusal_end_at(&s, &ctx, at, len);
                let token_end = usize::from(r.end());
                prop_assert!(reach >= token_end, "{:?}: reach {}", &s[r], reach);
                match end {
                    Some(e) if e != token_end => prop_assert!(reach >= e, "{:?}: reach {} < {}", &s[r], reach, e),
                    _ => prop_assert_eq!(reach, s.len(), "{:?}", &s[r]),
                }
                prop_assert!(
                    split.ambiguous.iter().any(|a| a.reason == sql_split::AmbiguityReason::AmbiguousCommentEnd
                        && a.at.start() <= r.start() && a.at.end() >= r.start()),
                    "{:?} in {:?} is not refused: {:?}", &s[r], s, split.ambiguous
                );
            }
        }
    }

    #[test]
    fn editor_and_execution_agree_on_exact_statements(s in any_sql(), (id, flavor) in dialects()) {
        let editor = split_in(&s, id, flavor, SplitMode::Editor);
        let exec = split_in(&s, id, flavor, SplitMode::Execution);
        let agree = exact_items_agree(&editor, &exec);
        prop_assert!(agree.is_ok(), "{id:?} {flavor:?} {s:?}: {agree:?}");
        // Both modes see the same uncertainty and the same client commands.
        prop_assert_eq!(&editor.ambiguous, &exec.ambiguous);
    }

    #[test]
    fn any_chunking_gives_the_same_split(
        s in any_sql(),
        (id, flavor) in dialects(),
        first in 1usize..96,
        one_line in any::<bool>(),
    ) {
        // Short chunks cut at line starts, or inside lines when a line is long (all of a
        // one-line script): the tokens, and so the split, are the whole lexing's.
        let s = if one_line { s.replace(['\n', '\r'], " ") } else { s };
        let ctx = DialectCtx::new(id, None);
        for mode in [SplitMode::Editor, SplitMode::Execution] {
            let whole = split_chunked(&s, &ctx, flavor, mode, &SplitStart::default(), usize::MAX / 4);
            let chunked = split_chunked(&s, &ctx, flavor, mode, &SplitStart::default(), first);
            prop_assert_eq!(&chunked, &whole, "{:?} {:?} {:?} from {}", id, flavor, s, first);
        }
    }

    #[test]
    fn a_split_is_a_pure_function_of_its_input(s in any_sql(), (id, flavor) in dialects()) {
        let a = split_in(&s, id, flavor, SplitMode::Editor);
        let b = split_in(&s, id, flavor, SplitMode::Editor);
        prop_assert_eq!(a, b);
    }
}
