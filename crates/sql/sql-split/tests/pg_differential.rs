//! The PostgreSQL parser differential (Task 13): what Run sends for a whole script is what
//! PostgreSQL's own grammar (libpg_query, through `sql-oracle-pg`) reads as its statements.
//!
//! The oracle's ranges are trimmed (no trivia, no `;`) and relative to the text it is given,
//! exactly as a unit's strict range is. Fixtures the server's grammar cannot read as written
//! (psql commands, `COPY` data) are pinned to their refusals instead: none is skipped silently.

use sql_dialect::{ClientFlavor, DialectCtx, DialectId, ServerVersion};
use sql_oracle_pg::split_with_parser;
use sql_split::exec_range::{CaretOrSelection, ExecPolicy, RangeResolution, UnitKind, resolve};
use sql_split::{DocSnapshot, SplitMode, SqlDocument};
use sql_testkit::split::fixture;
use sql_text::{TextRange, TextSize};

fn pg_doc(sql: &str) -> DocSnapshot {
    let version = ServerVersion {
        major: 18,
        minor: 0,
        patch: 0,
    };
    let ctx = DialectCtx::new(DialectId::Postgres, Some(version));
    SqlDocument::new(sql.to_owned(), ctx, ClientFlavor::Psql).snapshot()
}

fn whole_script(sql: &str) -> RangeResolution {
    resolve(
        &pg_doc(sql),
        &[CaretOrSelection::Caret(TextSize::from(0))],
        &ExecPolicy::whole_script(),
        SplitMode::Execution,
        None,
        None,
    )
}

/// The ranges Run 3 (whole script) sends for `sql`.
fn exec_ranges(sql: &str) -> Vec<TextRange> {
    match whole_script(sql) {
        RangeResolution::Units(units) => units.iter().map(|u| u.range).collect(),
        other => panic!("the whole script does not run: {other:?}\n{sql}"),
    }
}

/// The fixtures PostgreSQL's grammar reads as written, with their statement counts.
const PG_FIXTURES: [(&str, usize); 6] = [
    ("FX-CARET-01", 2),
    ("FX-SPLIT-PG-01", 2),
    ("FX-SPLIT-PG-02", 2),
    ("FX-SPLIT-PG-03", 2),
    ("FX-SPLIT-PG-04", 2),
    ("FX-SPLIT-PG-06", 3),
];

/// Scripts of this test's own, in the shapes Run meets: comments around statements, a string
/// and a dollar quote holding `;`, a missing last `;`, empty statements, a CTE with DML.
const PG_SCRIPTS: [(&str, usize); 6] = [
    (
        "-- one\nSELECT 1; -- after\n/* two */ SELECT 'a;b' ;\n\nSELECT $q$ x; y $q$",
        3,
    ),
    ("SELECT 1;;\n;SELECT 2;", 2),
    (
        "WITH gone AS (DELETE FROM audit_log WHERE actor = 'x' RETURNING *)\nSELECT count(*) FROM gone;\nBEGIN;\nROLLBACK;",
        3,
    ),
    (
        "CREATE FUNCTION f(a int) RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT CASE WHEN a > 0 THEN a ELSE 0 END; END;\nSELECT f(1);",
        2,
    ),
    (
        "SELECT E'it\\'s; fine', U&'d\\0061t\\+000061', B'101';\nSELECT 2",
        2,
    ),
    (
        "SELECT a[1:2], b::text, c ->> 'k' FROM t WHERE d @> '{}' ;",
        1,
    ),
];

#[test]
fn pg_execution_split_matches_the_parser_on_fixtures() {
    for (id, count) in PG_FIXTURES {
        let f = fixture(id);
        let ours = exec_ranges(&f);
        assert_eq!(ours.len(), count, "{id}");
        assert_eq!(ours, split_with_parser(&f).unwrap(), "{id}");
    }
    for (sql, count) in PG_SCRIPTS {
        let ours = exec_ranges(sql);
        assert_eq!(ours.len(), count, "{sql}");
        assert_eq!(ours, split_with_parser(sql).unwrap(), "{sql}");
    }
}

#[test]
fn every_unit_alone_is_one_statement_to_the_parser() {
    for (id, _) in PG_FIXTURES {
        let f = fixture(id);
        for r in exec_ranges(&f) {
            let unit = &f[r];
            let whole = TextRange::up_to(TextSize::try_from(unit.len()).unwrap());
            assert_eq!(split_with_parser(unit).unwrap(), [whole], "{id}: {unit}");
        }
    }
}

#[test]
fn fixtures_the_grammar_cannot_read_are_refused_or_carry_their_data() {
    // psql meta-commands: the script is not run as psql would run it.
    let meta = fixture("FX-SPLIT-PG-07");
    assert!(split_with_parser(&meta).is_err());
    assert!(matches!(
        whole_script(&meta),
        RangeResolution::UnsupportedClientCommand { at } if &meta[at] == "\\set ON_ERROR_STOP on"
    ));

    // COPY … FROM STDIN: the data rows are not SQL; the unit carries them apart.
    let copy = fixture("FX-SPLIT-PG-05");
    assert!(split_with_parser(&copy).is_err());
    let RangeResolution::Units(units) = whole_script(&copy) else {
        panic!("the COPY fixture resolves to units")
    };
    let texts: Vec<&str> = units.iter().map(|u| &copy[u.range]).collect();
    assert_eq!(
        texts,
        [
            "COPY audit_log(actor, message) FROM STDIN",
            "SELECT count(*) FROM audit_log"
        ]
    );
    assert!(matches!(
        units[0].kind,
        UnitKind::CopyIn { data } if &copy[data] == "sys\thello; world\nsys\tsecond\n"
    ));
    // Each unit alone is one statement to the parser; the rows are in neither.
    for t in texts {
        assert_eq!(split_with_parser(t).unwrap().len(), 1, "{t}");
    }
}
