//! Statement mutations for the classifier's differential fuzz (Task 20; `alg/13` :399:
//! "mutation of the corpus"). Shared by `sql-safety/tests/mutation_props.rs` (offline, the
//! libpg_query oracle) and `hacivat-exec/tests/docker_mutation_fuzz.rs` (the server oracle on
//! Docker pg17, P-M3b-13), each of which includes this file and `sql-lexer`'s
//! `tests/support/edits.rs` (the one shared generator, T5 I1) with `#[path]`, as `mod edits`
//! and `mod mutate` at the crate root.
//!
//! Two kinds of mutation, both cut at our lexer's token boundaries:
//! - [`mutate`]: anything goes. Write fragments spliced in (`INTO`, `FOR UPDATE`, a
//!   data-modifying `WITH`, `RETURNING`, a second statement), calls of functions with side
//!   effects, wrappers (`EXPLAIN ANALYZE`, `CREATE TABLE … AS`, a sub-select), executable
//!   comments, a slice of another statement, a word replaced, a token dropped or doubled. Most
//!   results are syntax errors or writes; the oracles judge the ones the classifier allows.
//! - [`preserve`]: the meaning stays: an ASCII letter-case flip of a word, whitespace of the
//!   same kind (a newline stays a newline: PostgreSQL joins string constants across one), or a
//!   block comment where there was a space. The verdict of such a mutant is never weaker than
//!   the original's.

#![allow(dead_code)]

use sql_dialect::{DialectCtx, DialectId, LexModes, lex_profile};
use sql_lexer::{LexState, Lexed, TokenKind, lex};

use crate::edits::Rng;

/// Fixture names the mutants write to: the Docker leg creates them (`hv_t20` database), and
/// a write the server lets through shows on its side channel.
pub const TABLE: &str = "hv_t20_t";
pub const SEQUENCE: &str = "hv_t20_seq";

/// The engine family a mutation speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Pg,
    MySql,
    Sqlite,
}

impl Family {
    pub fn of(d: DialectId) -> Family {
        match d {
            DialectId::MySql | DialectId::MariaDb => Family::MySql,
            DialectId::Sqlite => Family::Sqlite,
            _ => Family::Pg,
        }
    }
}

/// Fragments spliced in at a token boundary.
fn fragments(f: Family) -> &'static [&'static str] {
    match f {
        Family::Pg => &[
            " INTO hv_t20_new ",
            " FOR UPDATE",
            " FOR NO KEY UPDATE SKIP LOCKED",
            " FOR SHARE",
            " RETURNING *",
            "; DELETE FROM hv_t20_t",
            "; UPDATE hv_t20_t SET a = a + 1",
            "; SELECT 1",
            " , nextval('hv_t20_seq') ",
            " , setval('hv_t20_seq', 42) ",
            " , pg_advisory_lock(1) ",
            " , lo_create(0) ",
            " , set_config('search_path', 'public', false) ",
            " , txid_current() ",
            " , pg_notify('hv_t20', 'x') ",
            " , pg_sleep(0) ",
            " , pg_read_file('postmaster.pid') ",
            " , currval('hv_t20_seq') ",
            " , pg_current_xact_id() ",
            " , pg_try_advisory_xact_lock(2) ",
            " , pg_logical_emit_message(false, 'p', 'x') ",
            " , dblink_exec('dbname=x', 'DELETE FROM hv_t20_t') ",
            " , hv_t20_writer() ",
            " UNION ALL SELECT 1 ",
            " WHERE EXISTS (SELECT 1 FROM hv_t20_t FOR UPDATE) ",
            " ORDER BY 1 LIMIT 1 ",
            "::regclass",
            " $$ ; DELETE FROM hv_t20_t; $$ ",
            " E'\\'; DELETE FROM hv_t20_t; --' ",
            " /* ; DELETE FROM hv_t20_t; */ ",
            " -- ; DELETE\n",
            " TABLESAMPLE SYSTEM (50) ",
        ],
        Family::MySql => &[
            " INTO OUTFILE '/tmp/hv_t20' ",
            " INTO @hv_t20 ",
            " FOR UPDATE",
            " LOCK IN SHARE MODE",
            "; DELETE FROM hv_t20_t",
            " , SLEEP(0) ",
            " , GET_LOCK('hv_t20', 0) ",
            " , RELEASE_LOCK('hv_t20') ",
            " , @hv := 1 ",
            " , LAST_INSERT_ID(5) ",
            " , BENCHMARK(1, 1) ",
            " , LOAD_FILE('/etc/hostname') ",
            " , hv_t20_writer() ",
            " /*!50000 , SLEEP(0) */ ",
            " /*! INTO OUTFILE '/tmp/hv_t20' */ ",
            " /*M! , SLEEP(0) */ ",
            " 'a\\' , SLEEP(0) , ' ",
            " \"a\\\" , SLEEP(0) , \" ",
            " # ; DELETE\n",
            " UNION ALL SELECT 1 ",
            " FROM performance_schema.threads ",
            " FROM sys.session ",
        ],
        Family::Sqlite => &[
            "; DELETE FROM hv_t20_t",
            " RETURNING *",
            " , load_extension('x') ",
            " , randomblob(4) ",
            " , hv_t20_writer() ",
            " UNION ALL SELECT 1 ",
            " REGEXP 'a' ",
            " /* ; DELETE */ ",
        ],
    }
}

/// Text put in front of the statement.
fn prefixes(f: Family) -> &'static [&'static str] {
    match f {
        Family::Pg => &[
            "EXPLAIN ",
            "EXPLAIN ANALYZE ",
            "EXPLAIN (ANALYZE, BUFFERS) ",
            "EXPLAIN (FORMAT JSON) ",
            "CREATE TABLE hv_t20_new AS ",
            "CREATE TEMP TABLE hv_t20_tmp AS ",
            "WITH hv_t20_w AS (DELETE FROM hv_t20_t RETURNING *) ",
            "WITH hv_t20_w AS (SELECT 1) ",
            "COPY (",
            "PREPARE hv_t20_p AS ",
            "DECLARE hv_t20_c CURSOR FOR ",
            "SELECT * FROM (",
            "BEGIN; ",
            "SET LOCAL statement_timeout = 0; ",
            "\u{feff}",
        ],
        Family::MySql => &[
            "EXPLAIN ",
            "EXPLAIN ANALYZE ",
            "ANALYZE ",
            "CREATE TABLE hv_t20_new AS ",
            "INSERT INTO hv_t20_t ",
            "HANDLER hv_t20_t OPEN; ",
            "/*!50000 ",
            "DO ",
        ],
        Family::Sqlite => &[
            "EXPLAIN ",
            "EXPLAIN QUERY PLAN ",
            "CREATE TABLE hv_t20_new AS ",
            "INSERT INTO hv_t20_t ",
            "WITH hv_t20_w AS (SELECT 1) ",
        ],
    }
}

/// Text put after the statement.
fn suffixes(f: Family) -> &'static [&'static str] {
    match f {
        Family::Pg => &[
            ") TO STDOUT",
            ") s",
            ";",
            "; COMMIT",
            " \\gset",
            " FOR UPDATE",
        ],
        Family::MySql => &[" */", ";", " FOR UPDATE", " INTO DUMPFILE '/tmp/hv_t20'"],
        Family::Sqlite => &[";", " RETURNING *"],
    }
}

/// Words that replace a word.
const WORDS: &[&str] = &[
    "DELETE",
    "UPDATE",
    "INSERT",
    "INTO",
    "MERGE",
    "TRUNCATE",
    "DROP",
    "CALL",
    "DO",
    "LOCK",
    "SET",
    "COPY",
    "VACUUM",
    "nextval",
    "setval",
    "lo_import",
    "pg_sleep",
    "SLEEP",
    "GET_LOCK",
    "hv_t20_writer",
    "TABLE",
    "VALUES",
    "RETURNING",
    "FOR",
    "SHARE",
    "NOWAIT",
    "LATERAL",
    "ONLY",
];

fn lexed(sql: &str, f: Family) -> Option<Lexed> {
    let d = match f {
        Family::Pg => DialectId::Postgres,
        Family::MySql => DialectId::MySql,
        Family::Sqlite => DialectId::Sqlite,
    };
    let ctx = DialectCtx::new(d, None);
    let p = lex_profile(&ctx).ok()?;
    Some(lex(sql, p, LexModes::default_for(d), &LexState::Initial))
}

/// The token boundaries of `sql` (0 and the end included).
fn boundaries(l: &Lexed, len: usize) -> Vec<usize> {
    let mut out = vec![0];
    for i in 0..l.len() {
        out.push(usize::from(l.range(i).end()));
    }
    out.push(len);
    out.dedup();
    out
}

fn splice(sql: &str, at: usize, what: &str) -> String {
    let mut s = String::with_capacity(sql.len() + what.len());
    s.push_str(&sql[..at]);
    s.push_str(what);
    s.push_str(&sql[at..]);
    s
}

/// One to three mutations of `sql`, anything goes (module docs); `donors` lends slices.
pub fn mutate(r: &mut Rng, sql: &str, f: Family, donors: &[&str]) -> String {
    let mut s = sql.to_string();
    for _ in 0..=r.below(3) {
        s = mutate_once(r, &s, f, donors);
    }
    s
}

fn mutate_once(r: &mut Rng, sql: &str, f: Family, donors: &[&str]) -> String {
    let Some(l) = lexed(sql, f) else {
        return sql.to_string();
    };
    let cuts = boundaries(&l, sql.len());
    let at = cuts[r.below(cuts.len())];
    match r.below(9) {
        0 | 1 => splice(sql, at, r.pick(fragments(f))),
        2 => format!("{}{sql}", r.pick(prefixes(f))),
        3 => format!("{sql}{}", r.pick(suffixes(f))),
        4 => {
            // A word replaced.
            let words: Vec<usize> = (0..l.len())
                .filter(|&i| l.kind(i) == TokenKind::Word)
                .collect();
            if words.is_empty() {
                return splice(sql, at, r.pick(WORDS));
            }
            let w = l.range(words[r.below(words.len())]);
            let (a, b) = (usize::from(w.start()), usize::from(w.end()));
            format!("{}{}{}", &sql[..a], r.pick(WORDS), &sql[b..])
        }
        5 => {
            // A slice of another statement, cut at its tokens.
            if donors.is_empty() {
                return sql.to_string();
            }
            let donor = donors[r.below(donors.len())];
            let Some(dl) = lexed(donor, f) else {
                return sql.to_string();
            };
            let dc = boundaries(&dl, donor.len());
            let i = r.below(dc.len());
            let j = (i + 1 + r.below(8)).min(dc.len() - 1);
            splice(sql, at, &format!(" {} ", &donor[dc[i]..dc[j]]))
        }
        6 => {
            // A token dropped.
            if l.is_empty() {
                return sql.to_string();
            }
            let t = l.range(r.below(l.len()));
            let (a, b) = (usize::from(t.start()), usize::from(t.end()));
            format!("{}{}", &sql[..a], &sql[b..])
        }
        7 => {
            // A token doubled.
            if l.is_empty() {
                return sql.to_string();
            }
            let t = l.range(r.below(l.len()));
            let (a, b) = (usize::from(t.start()), usize::from(t.end()));
            splice(sql, b, &format!(" {}", &sql[a..b]))
        }
        _ => {
            // An executable or hint comment around a piece (MySQL reads the body).
            let j = cuts[r.below(cuts.len())];
            let (a, b) = (at.min(j), at.max(j));
            let open = match f {
                Family::MySql => ["/*! ", "/*!80000 ", "/*M! ", "/*+ "][r.below(4)],
                _ => ["/* ", "/*+ "][r.below(2)],
            };
            format!("{}{open}{} */{}", &sql[..a], &sql[a..b], &sql[b..])
        }
    }
}

/// One meaning-preserving mutation of `sql` (module docs), or `None` when it has no word or
/// whitespace token.
pub fn preserve(r: &mut Rng, sql: &str, f: Family) -> Option<String> {
    let l = lexed(sql, f)?;
    let picks: Vec<usize> = (0..l.len())
        .filter(|&i| matches!(l.kind(i), TokenKind::Word | TokenKind::Whitespace))
        .collect();
    if picks.is_empty() {
        return None;
    }
    let i = picks[r.below(picks.len())];
    let t = l.range(i);
    let (a, b) = (usize::from(t.start()), usize::from(t.end()));
    let tok = &sql[a..b];
    let new = if l.kind(i) == TokenKind::Word {
        let flip = r.below(3);
        tok.chars()
            .enumerate()
            .map(|(k, c)| match flip {
                0 => c.to_ascii_uppercase(),
                1 => c.to_ascii_lowercase(),
                _ if k % 2 == 0 => c.to_ascii_uppercase(),
                _ => c.to_ascii_lowercase(),
            })
            .collect::<String>()
    } else if tok.contains('\n') || tok.contains('\r') {
        ["\n", "\r\n", "\n\n", " \n\t"][r.below(4)].to_string()
    } else {
        [" ", "\t", "   ", " /* hv */ ", " /**/ "][r.below(5)].to_string()
    };
    Some(format!("{}{new}{}", &sql[..a], &sql[b..]))
}
