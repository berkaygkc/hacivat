//! M3a Task 23: execution units (HV-EXEC-01): the strict text sent byte for byte, cursor
//! eligibility from the strict range, the PostgreSQL oracle for ambiguity (R-M3-24, R-M3-35),
//! the refusals (COPY streams, run as one), and the SQL context (R-M3-39).

mod common;

use std::cell::Cell;

use common::*;
use hacivat_driver::SemVerLoose;
use hacivat_exec::sqlctx::{dialect_ctx, dialect_id, needs_reprobe, probe_sql};
use hacivat_exec::{
    OracleLane, RefuseReason, Resolution, RunAction, SplitOracle, resolve_units,
    resolve_units_with, uncertain_message_id, units_for_candidate,
};
use hacivat_proto::DbmsKind;
use sql_dialect::{DialectId, PgWireFlavor, ServerVersion};
use sql_split::exec_range::{Candidate, CandidateKind, UnitKind, UnitSpan};
use sql_split::{AmbiguityReason, DocSnapshot};
use sql_text::{TextRange, TextSize};

/// A PostgreSQL text the splitter cannot cut with certainty (the editor's heuristic suspects a
/// missing `;` before the `SELECT` line), which PostgreSQL's grammar reads as two statements.
const VIEW: &str = "CREATE VIEW v AS\nWITH x AS (SELECT 1)\nSELECT * FROM x;\nSELECT 9;";

/// Counts the oracle's calls and answers with `answer` (relative ranges), or PostgreSQL's own
/// split when `answer` is `None`.
struct Counting {
    calls: Cell<usize>,
    texts: std::cell::RefCell<Vec<String>>,
    answer: Option<Vec<TextRange>>,
}

impl Counting {
    fn real() -> Self {
        Counting {
            calls: Cell::new(0),
            texts: Default::default(),
            answer: None,
        }
    }

    fn answering(ranges: &[(u32, u32)]) -> Self {
        Counting {
            answer: Some(
                ranges
                    .iter()
                    .map(|(a, b)| TextRange::new(TextSize::from(*a), TextSize::from(*b)))
                    .collect(),
            ),
            ..Counting::real()
        }
    }
}

impl SplitOracle for Counting {
    fn split(&self, text: &str) -> Option<Vec<TextRange>> {
        self.calls.set(self.calls.get() + 1);
        self.texts.borrow_mut().push(text.to_string());
        match &self.answer {
            Some(a) => Some(a.clone()),
            None => sql_oracle_pg::split_with_parser(text).ok(),
        }
    }
}

fn texts(r: &Resolution) -> Vec<&str> {
    units(r).iter().map(|u| &*u.text).collect()
}

#[test]
fn units_carry_strict_text_and_eligibility() {
    let r = resolve_doc(
        "SELECT 1; INSERT INTO t VALUES (1);",
        sel(0..35),
        RunAction::Execute1,
    );
    assert_eq!(
        units(&r)
            .iter()
            .map(|u| (&*u.text, u.suspendable))
            .collect::<Vec<_>>(),
        [("SELECT 1", true), ("INSERT INTO t VALUES (1)", false)]
    );
    assert_eq!(units(&r)[1].doc_range, 10..34);
    assert!(units(&r).iter().all(|u| u.kind == UnitKind::Statement));
}

#[test]
fn the_text_sent_is_the_strict_range_byte_for_byte() {
    let text = "-- title\nSELECT 'a;b' , \"q;\" ;  -- after\r\nSELECT $$ x; $$ ;\n";
    let doc = pg_doc(text);
    let r = resolve_units(&doc, &[caret(0)], RunAction::Execute3, None);
    let us = units(&r);
    assert_eq!(texts(&r), ["SELECT 'a;b' , \"q;\"", "SELECT $$ x; $$"]);
    for u in us {
        assert_eq!(&text[u.doc_range.clone()], &*u.text, "byte for byte");
    }
    // The caret on the title comment runs the statement it belongs to, without the comment.
    let at_title = resolve_units(&doc, &[caret(2)], RunAction::Execute2, None);
    assert_eq!(texts(&at_title), ["SELECT 'a;b' , \"q;\""]);
}

#[test]
fn a_parenthesised_select_is_never_cursor_eligible() {
    let r = resolve_doc("(SELECT 1);\nSELECT 2;", caret(0), RunAction::Execute3);
    assert_eq!(
        units(&r)
            .iter()
            .map(|u| (&*u.text, u.suspendable))
            .collect::<Vec<_>>(),
        [("(SELECT 1)", false), ("SELECT 2", true)]
    );
    // Locking clauses and INTO are not read heads either (R-M3-23).
    for sql in [
        "SELECT * FROM t FOR UPDATE",
        "SELECT 1 INTO x",
        "WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d",
    ] {
        let r = resolve_doc(sql, caret(0), RunAction::Execute1);
        assert!(!units(&r)[0].suspendable, "{sql}");
    }
}

#[test]
fn pg_ambiguity_resolved_by_the_oracle_when_it_covers_the_region() {
    let doc = pg_doc(VIEW);
    // Without the oracle the first statement is uncertain.
    let none = Counting::answering(&[]);
    assert!(matches!(
        resolve_units_with(&doc, &[caret(0)], RunAction::Execute1, None, &none),
        Resolution::Uncertain(b) if b.iter().any(|b| b.reason == AmbiguityReason::MissingSemicolon)
    ));
    // PostgreSQL's grammar reads the region as one statement: it runs, as resolved.
    let oracle = Counting::real();
    let r = resolve_units_with(&doc, &[caret(3)], RunAction::Execute1, None, &oracle);
    assert_eq!(
        texts(&r),
        ["CREATE VIEW v AS\nWITH x AS (SELECT 1)\nSELECT * FROM x"]
    );
    assert_eq!(units(&r)[0].doc_range, 0..53);
    assert!(!units(&r)[0].suspendable);
    // The oracle saw exactly the region: the execution statement holding the boundary.
    assert_eq!(*oracle.texts.borrow(), [&VIEW[..53]]);
    // The whole script: the region's statement, then the certain one after it.
    let whole = resolve_units(&doc, &[caret(0)], RunAction::Execute3, None);
    assert_eq!(texts(&whole), [&VIEW[..53], "SELECT 9"]);
    // A caret on the certain statement runs it without asking the oracle.
    let after = Counting::real();
    let r = resolve_units_with(&doc, &[caret(60)], RunAction::Execute1, None, &after);
    assert_eq!(texts(&r), ["SELECT 9"]);
    assert_eq!(after.calls.get(), 0);
}

#[test]
fn an_oracle_split_that_does_not_cover_the_region_stays_uncertain() {
    let doc = pg_doc(VIEW);
    for answer in [
        // Short of the region's end.
        vec![(0, 40)],
        // Text between two statements that is not a separator.
        vec![(0, 20), (38, 53)],
        // Overlapping statements.
        vec![(0, 40), (30, 53)],
        // Nothing.
        vec![],
    ] {
        let oracle = Counting::answering(&answer);
        let r = resolve_units_with(&doc, &[caret(0)], RunAction::Execute1, None, &oracle);
        assert!(matches!(r, Resolution::Uncertain(_)), "{answer:?}: {r:?}");
        let r = resolve_units_with(&doc, &[caret(0)], RunAction::Execute3, None, &oracle);
        assert!(matches!(r, Resolution::Uncertain(_)), "{answer:?}: {r:?}");
    }
    // Two statements separated by white space and `;` only do cover it.
    let split = Counting::answering(&[(0, 16), (17, 53)]);
    let r = resolve_units_with(&doc, &[caret(20)], RunAction::Execute1, None, &split);
    assert_eq!(texts(&r), ["WITH x AS (SELECT 1)\nSELECT * FROM x"]);
}

/// A PostgreSQL text whose first statement is ambiguous and nests `depth` parentheses.
fn resolve_pg_ambiguous_deep_with(depth: usize, oracle: &dyn SplitOracle) -> Resolution {
    let nested = format!("{}1{}", "(".repeat(depth), ")".repeat(depth));
    let text = format!("CREATE VIEW v AS\nWITH x AS (SELECT {nested})\nSELECT * FROM x;\n");
    resolve_units_with(
        &pg_doc(&text),
        &[caret(0)],
        RunAction::Execute1,
        None,
        oracle,
    )
}

fn resolve_pg_ambiguous_deep(depth: usize) -> Resolution {
    let nested = format!("{}1{}", "(".repeat(depth), ")".repeat(depth));
    let text = format!("CREATE VIEW v AS\nWITH x AS (SELECT {nested})\nSELECT * FROM x;\n");
    resolve_units(&pg_doc(&text), &[caret(0)], RunAction::Execute1, None)
}

#[test]
fn deep_nesting_is_never_sent_to_the_oracle() {
    assert!(matches!(
        resolve_pg_ambiguous_deep(300),
        Resolution::Uncertain(_)
    ));
    let deep = Counting::real();
    assert!(matches!(
        resolve_pg_ambiguous_deep_with(300, &deep),
        Resolution::Uncertain(_)
    ));
    assert_eq!(deep.calls.get(), 0, "never sent to the parser");
    // At the limit (255 inside the CTE's own parenthesis: 256 deep) it is.
    let at_limit = Counting::real();
    assert!(matches!(
        resolve_pg_ambiguous_deep_with(255, &at_limit),
        Resolution::Units(_)
    ));
    assert_eq!(at_limit.calls.get(), 1);
    let over = Counting::real();
    assert!(matches!(
        resolve_pg_ambiguous_deep_with(256, &over),
        Resolution::Uncertain(_)
    ));
    assert_eq!(over.calls.get(), 0);
}

#[test]
fn mysql_ambiguity_stays_uncertain() {
    let doc = mysql_doc("SELECT 1 /*! ; */;\nSELECT 2;");
    let oracle = Counting::real();
    let r = resolve_units_with(&doc, &[caret(0)], RunAction::Execute1, None, &oracle);
    assert!(
        matches!(&r, Resolution::Uncertain(b)
            if b.iter().any(|b| b.reason == AmbiguityReason::SemicolonInExecutableComment)),
        "{r:?}"
    );
    let r = resolve_units_with(&doc, &[caret(0)], RunAction::Execute3, None, &oracle);
    assert!(matches!(r, Resolution::Uncertain(_)), "{r:?}");
    assert_eq!(oracle.calls.get(), 0);
    // Nor on a PostgreSQL-wire server with its own grammar.
    let crdb = doc_in(
        VIEW,
        sql_dialect::DialectCtx::new(DialectId::PgWire(PgWireFlavor::Cockroach), None),
    );
    let r = resolve_units_with(&crdb, &[caret(0)], RunAction::Execute1, None, &oracle);
    assert!(matches!(r, Resolution::Uncertain(_)), "{r:?}");
    assert_eq!(oracle.calls.get(), 0);
}

#[test]
fn a_selection_or_several_carets_never_ask_the_oracle() {
    let doc = pg_doc(VIEW);
    let oracle = Counting::real();
    for at in [vec![sel(0..54)], vec![caret(0), caret(60)]] {
        let r = resolve_units_with(&doc, &at, RunAction::Execute1, None, &oracle);
        assert!(matches!(r, Resolution::Uncertain(_)), "{at:?}: {r:?}");
    }
    assert_eq!(oracle.calls.get(), 0);
    // A remembered run of several statements stays uncertain too.
    for kind in [CandidateKind::FromCaret, CandidateKind::WholeScript] {
        let r = resolve_units_with(&doc, &[caret(0)], RunAction::Execute1, Some(kind), &oracle);
        assert!(!matches!(r, Resolution::Units(_)), "{kind:?}: {r:?}");
    }
}

#[test]
fn a_superseded_oracle_request_is_skipped() {
    let doc = pg_doc(VIEW);
    let lane = OracleLane::new();
    let first = lane.request();
    let second = lane.request();
    // The earlier request's job sees a newer ticket and never parses.
    assert_eq!(first.split(&VIEW[..53]), None);
    assert!(matches!(
        resolve_units_with(&doc, &[caret(0)], RunAction::Execute1, None, &first),
        Resolution::Uncertain(_)
    ));
    // The newest one is answered.
    assert!(matches!(
        resolve_units_with(&doc, &[caret(0)], RunAction::Execute1, None, &second),
        Resolution::Units(_)
    ));
    // `resolve` takes a fresh ticket each time, so a lane's latest call is answered.
    assert!(matches!(
        lane.resolve(&doc, &[caret(0)], RunAction::Execute1, None),
        Resolution::Units(_)
    ));
}

const COPY_SCRIPT: &str = "SELECT 1;\nCOPY t FROM STDIN;\n1\n\\.\nSELECT 2;\n";

#[test]
fn a_copy_stream_refuses_the_whole_run() {
    let doc = pg_doc(COPY_SCRIPT);
    let r = resolve_units(&doc, &[caret(0)], RunAction::Execute3, None);
    assert_eq!(
        r,
        Resolution::Refused {
            at: 10..27,
            reason: RefuseReason::CopyStream
        }
    );
    assert_eq!(
        RefuseReason::CopyStream.fluent_id(),
        "exec-copy-unsupported"
    );
    // COPY … TO STDOUT has no data rows in the script but streams all the same.
    let out = resolve_doc("COPY (SELECT 1) TO STDOUT;", caret(0), RunAction::Execute1);
    assert!(matches!(
        out,
        Resolution::Refused {
            reason: RefuseReason::CopyStream,
            ..
        }
    ));
    // A COPY from a server file is an ordinary statement; "stdin" in a string is data.
    for sql in [
        "COPY t FROM '/srv/t.csv'",
        "COPY t FROM '/srv/stdin' WITH (FORMAT csv)",
    ] {
        assert!(
            matches!(
                resolve_doc(sql, caret(0), RunAction::Execute1),
                Resolution::Units(_)
            ),
            "{sql}"
        );
    }
}

#[test]
fn the_chooser_leaves_out_candidates_that_would_be_refused() {
    let doc = pg_doc(COPY_SCRIPT);
    // The caret in `SELECT 2`: the statement runs; "from the caret" and "whole script" hold
    // the COPY (whole script) or not.
    let at = u32::try_from(COPY_SCRIPT.rfind("SELECT 2").unwrap()).unwrap();
    match resolve_units(&doc, &[caret(at)], RunAction::Execute1, None) {
        Resolution::Units(u) => assert_eq!(u.len(), 1),
        Resolution::Ask { candidates, .. } => {
            assert!(!candidates.is_empty());
            for c in &candidates {
                assert!(!units_for_candidate(&doc, c).is_empty(), "{:?}", c.kind);
                assert_ne!(c.kind, CandidateKind::WholeScript);
            }
        }
        other => panic!("{other:?}"),
    }
}

fn verbatim(doc: &DocSnapshot, range: (u32, u32)) -> Candidate {
    Candidate {
        kind: CandidateKind::Statement,
        units: vec![UnitSpan {
            range: TextRange::new(TextSize::from(range.0), TextSize::from(range.1)),
            kind: UnitKind::Verbatim,
            key: None,
            title_comment: None,
        }],
        label: doc.text().to_string(),
    }
}

#[test]
fn a_verbatim_unit_is_refused_until_m3b() {
    let doc = pg_doc("SELECT 1; SELECT 2;");
    assert!(units_for_candidate(&doc, &verbatim(&doc, (0, 18))).is_empty());
    assert_eq!(
        RefuseReason::RunAsOne.fluent_id(),
        "exec-run-as-one-unsupported"
    );
}

#[test]
fn every_ambiguity_reason_has_a_message() {
    let reasons = [
        AmbiguityReason::SemicolonInExecutableComment,
        AmbiguityReason::UnterminatedConstruct,
        AmbiguityReason::UnknownBlock,
        AmbiguityReason::DelimiterChange,
        AmbiguityReason::MissingSemicolon,
        AmbiguityReason::AmbiguousCommentEnd,
        AmbiguityReason::ClientCommand,
        AmbiguityReason::OverlappingUnits,
    ];
    let ids: std::collections::BTreeSet<_> =
        reasons.iter().map(|r| uncertain_message_id(*r)).collect();
    assert_eq!(ids.len(), reasons.len(), "one message each");
    assert_eq!(
        uncertain_message_id(AmbiguityReason::OverlappingUnits),
        "exec-uncertain-overlapping-units"
    );
}

#[test]
fn unsupported_client_commands_pass_through() {
    let r = resolve_doc(
        "SELECT 1;\n\\echo hi\nSELECT 2;",
        caret(0),
        RunAction::Execute3,
    );
    assert_eq!(r, Resolution::UnsupportedClientCommand { at: 10..18 });
}

#[test]
fn sql_mode_ansi_changes_quote_lexing() {
    assert!(
        dialect_ctx(DbmsKind::MySql, None, Some("ANSI"))
            .modes
            .ansi_quotes
    );
    assert!(!dialect_ctx(DbmsKind::MySql, None, None).modes.ansi_quotes);
    let mariadb = dialect_ctx(DbmsKind::MariaDb, None, Some("ORACLE,ANSI_QUOTES"));
    assert!(mariadb.modes.mariadb_oracle_mode && mariadb.modes.ansi_quotes);
    // The mode reaches the split: with ANSI_QUOTES a double-quoted `;` is in a name.
    let ctx = dialect_ctx(DbmsKind::MySql, None, Some("ANSI_QUOTES"));
    let doc = doc_in("SELECT \"a;b\" FROM t;", ctx);
    assert_eq!(
        doc.execution_split().statements().count(),
        1,
        "one statement"
    );
}

#[test]
fn the_sql_context_follows_the_engine_the_version_and_the_probe() {
    assert_eq!(dialect_id(DbmsKind::Postgres), DialectId::Postgres);
    assert_eq!(
        dialect_id(DbmsKind::Redshift),
        DialectId::PgWire(PgWireFlavor::Redshift)
    );
    assert_eq!(dialect_id(DbmsKind::TiDb), DialectId::MySql);
    assert_eq!(dialect_id(DbmsKind::Unknown), DialectId::Generic);
    assert_eq!(
        probe_sql(DbmsKind::Yugabyte),
        Some("SHOW standard_conforming_strings")
    );
    assert_eq!(
        probe_sql(DbmsKind::MariaDb),
        Some("SELECT @@SESSION.sql_mode")
    );
    assert_eq!(probe_sql(DbmsKind::Sqlite), None);
    let v = SemVerLoose::parse("16.4 (Debian 16.4-1)").unwrap();
    let pg = dialect_ctx(DbmsKind::Postgres, Some(&v), Some("off"));
    assert_eq!(pg.version, Some(ServerVersion::new(16, 4, 0)));
    assert!(!pg.modes.standard_conforming_strings);
    // An answer that is neither on nor off keeps the default.
    assert!(
        dialect_ctx(DbmsKind::Postgres, None, Some("maybe"))
            .modes
            .standard_conforming_strings
    );
    assert!(needs_reprobe("SET sql_mode = 'ANSI'"));
    assert!(needs_reprobe("set standard_conforming_strings = off"));
    assert!(!needs_reprobe("SELECT 1"));
    assert!(!needs_reprobe("SETTINGS"));
}

/// Controller ruling (from Task 24): a run holding any `CopyIn` or `Verbatim` unit is refused as
/// a whole — nothing is sent, the units before it included — and a `CopyIn` unit alone is refused
/// too (M3b). Task 24's preview marks these units uncertain; Run never sends around them.
#[test]
fn a_run_holding_copy_in_or_verbatim_is_refused_whole() {
    let doc = pg_doc(COPY_SCRIPT);
    // The COPY alone, at its own caret.
    for action in [RunAction::Execute1, RunAction::Execute2] {
        assert_eq!(
            resolve_units(&doc, &[caret(12)], action, None),
            Resolution::Refused {
                at: 10..27,
                reason: RefuseReason::CopyStream
            },
            "{action:?}"
        );
    }
    // A selection of all three statements: the SELECT before the COPY is not sent either.
    let all = u32::try_from(COPY_SCRIPT.len()).unwrap();
    assert_eq!(
        resolve_units(&doc, &[sel(0..all)], RunAction::Execute1, None),
        Resolution::Refused {
            at: 10..27,
            reason: RefuseReason::CopyStream
        }
    );
    // A candidate of several units, one of them verbatim, runs nothing.
    let plain = pg_doc("SELECT 1; SELECT 2;");
    let mixed = Candidate {
        kind: CandidateKind::WholeScript,
        units: vec![
            UnitSpan {
                range: TextRange::new(TextSize::from(0), TextSize::from(8)),
                kind: UnitKind::Statement,
                key: None,
                title_comment: None,
            },
            UnitSpan {
                range: TextRange::new(TextSize::from(10), TextSize::from(18)),
                kind: UnitKind::Verbatim,
                key: None,
                title_comment: None,
            },
        ],
        label: String::new(),
    };
    assert!(units_for_candidate(&plain, &mixed).is_empty());
    // And a candidate whose first unit is the COPY's data-bearing statement.
    let copy = Candidate {
        kind: CandidateKind::WholeScript,
        units: vec![
            UnitSpan {
                range: TextRange::new(TextSize::from(0), TextSize::from(8)),
                kind: UnitKind::Statement,
                key: None,
                title_comment: None,
            },
            UnitSpan {
                range: TextRange::new(TextSize::from(10), TextSize::from(27)),
                kind: UnitKind::CopyIn {
                    data: TextRange::new(TextSize::from(29), TextSize::from(31)),
                },
                key: None,
                title_comment: None,
            },
        ],
        label: String::new(),
    };
    assert!(units_for_candidate(&doc, &copy).is_empty());
}

/// Every unit Run would send from any caret of these PostgreSQL scripts, by any action, is its
/// strict text and one whole statement to PostgreSQL's own grammar: the oracle never widens or
/// narrows a unit past a statement, and nothing crosses an uncertain boundary unresolved.
#[test]
fn every_unit_at_every_caret_is_one_statement_to_the_parser() {
    let scripts = [
        VIEW,
        "SELECT 1;\nCREATE VIEW w AS\nSELECT 2\nUNION ALL\nSELECT 3;\n",
        "CREATE VIEW v AS\nWITH a AS (SELECT 1), b AS (SELECT 2)\nSELECT * FROM a, b;\n-- tail\nSELECT 'x;y';",
        "SELECT 1\nSELECT 2;\nSELECT 3;",
        "SELECT 'open;\nSELECT 2;",
        "CREATE FUNCTION f() RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT 1; END;\nSELECT f();",
    ];
    let mut oracle_units = 0;
    for text in scripts {
        let doc = pg_doc(text);
        for at in 0..=text.len() {
            if !text.is_char_boundary(at) {
                continue;
            }
            let at = u32::try_from(at).unwrap();
            for action in [
                RunAction::Execute1,
                RunAction::Execute2,
                RunAction::Execute3,
            ] {
                let r = resolve_units(&doc, &[caret(at)], action, None);
                let Resolution::Units(us) = &r else {
                    continue;
                };
                for u in us {
                    assert_eq!(&text[u.doc_range.clone()], &*u.text, "{text:?} @{at}");
                    let whole =
                        TextRange::up_to(TextSize::from(u32::try_from(u.text.len()).unwrap()));
                    match sql_oracle_pg::split_with_parser(&u.text) {
                        Ok(pieces) => assert_eq!(pieces, [whole], "{text:?} @{at}: {:?}", u.text),
                        Err(e) => panic!("{text:?} @{at}: {:?} does not parse: {e:?}", u.text),
                    }
                    let exact = doc.execution_split().runnable().any(|s| {
                        usize::from(s.strict.start()) == u.doc_range.start
                            && usize::from(s.strict.end()) == u.doc_range.end
                    });
                    if !exact {
                        oracle_units += 1;
                    }
                }
            }
        }
    }
    assert!(oracle_units > 0, "the sweep reached the oracle path");
}
