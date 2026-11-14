//! Splitter properties (Task 8) over generated SQL-ish text in every dialect and client flavor:
//! items never overlap, every significant token belongs to an item, execution mode never
//! returns a heuristic item, and the two modes agree on every exact statement (R-M3-8).

use proptest::prelude::*;
use sql_dialect::{ClientFlavor, DialectCtx, DialectId, lex_profile};
use sql_lexer::{LexState, lex};
use sql_split::{Confidence, Item, Split, SplitMode, SplitStart, split_from};
use sql_text::TextRange;

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

/// R-M3-8: an editor statement is `Exact` exactly when an execution statement has the same
/// strict range and no ambiguity overlaps it.
fn exact_items_agree(editor: &Split, exec: &Split) -> Result<(), String> {
    let exact_editor: Vec<TextRange> = editor
        .statements()
        .filter(|s| s.confidence == Confidence::Exact)
        .map(|s| s.strict)
        .collect();
    let certain_exec: Vec<TextRange> = exec
        .statements()
        .filter(|s| exec.ambiguities_in(s.strict).next().is_none())
        .map(|s| s.strict)
        .collect();
    if exact_editor == certain_exec {
        Ok(())
    } else {
        Err(format!(
            "editor exact {exact_editor:?} vs execution certain {certain_exec:?}"
        ))
    }
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
    fn execution_mode_never_returns_heuristic_items(s in any_sql(), (id, flavor) in dialects()) {
        let exec = split_in(&s, id, flavor, SplitMode::Execution);
        prop_assert!(exec.statements().all(|i| i.confidence == Confidence::Exact));
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
    fn a_split_is_a_pure_function_of_its_input(s in any_sql(), (id, flavor) in dialects()) {
        let a = split_in(&s, id, flavor, SplitMode::Editor);
        let b = split_in(&s, id, flavor, SplitMode::Editor);
        prop_assert_eq!(a, b);
    }
}
