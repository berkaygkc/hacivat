//! Guarded read-only execution and the role probe (HV-SESS-04, SQL-SAFE-05, MCP-09 step 1,
//! B21; Task 18): every statement of a read-only session runs inside the guard plan of
//! `hacivat_guard::plan_for`, with the post checks, and the connection is labelled by the role
//! probe.
//!
//! [`run_guarded`], for PostgreSQL, MySQL and MariaDB:
//! - 1: `check_input`, else `ReadOnlyDenied(Input)`;
//! - 1a: any reading of the first keyword (`guard_keywords`, as the server's lexer reads it)
//!   is a transaction keyword → `ReadOnlyDenied(Transaction)` (Q35);
//! - 1b (Task 23 finding F1): PostgreSQL, a call of a function whose effect a rollback does not
//!   undo (`hacivat_guard::check_functions`) → `ReadOnlyDenied(Function)`;
//! - 2: the plan's `before` statements (`batch`): if one fails, nothing else runs; then the
//!   guard's transaction must be open, else a gap and the statement is never sent;
//! - 3: the user statement, always through the extended protocol (one statement, prepared);
//! - 4: the transaction status after it;
//! - 5: PostgreSQL's canary (`execute(Simple)`), while the guard's transaction can answer;
//! - 6: the plan's `after` statements (`ROLLBACK`, `execute(Simple)`), with their notices;
//! - 7: `check_post`, after a failed statement too (except a MySQL implicit-commit DDL): a gap
//!   is `EnforcementGap(reason)`, and the connection is discarded unreset. A statement that
//!   succeeded is also judged with every reading of its first keyword as the command tag
//!   (M3a Task 22: a MySQL tag is read from the text, which a version-gated comment can
//!   mislead);
//! - 8: the connection's reset and the session's environment again (`replay`, Q15).
//!
//! If step 3 fails, steps 4 to 8 still run. SQLite runs steps 1, 1a and 1b (which passes every
//! engine but PostgreSQL), then the statement and the post-check (a tripwire): its layers are
//! connection-level (Task 11), its plan has no statements and no reset.
//!
//! The typed path (M3a Task 22, R-M3-17: `guarded_query`, `GuardedCursor`) runs the same
//! steps around a typed query that may outlive its request as a console cursor: steps 1–3 when
//! the query is sent, PostgreSQL's canary once more after the first chunk, and steps 4–8 when
//! the cursor ends (the section at the end of this file).
//!
//! Everything fails closed. The connection is discarded (`Guarded::discard`, which the actor
//! turns into a broken session) after a gap, a lost connection, a guard statement that failed
//! after the guard's transaction began, a canary that could not be read, a failed reset or
//! replay, and whenever the connection still reports a transaction at the end. User text never
//! goes through `batch`, the trusted path.

use std::collections::HashSet;
use std::time::Duration;

use futures::future::BoxFuture;
use hacivat_driver::{
    ExecOpts, ExecSummary, Phase, Protocol, QuerySpec, RawConn, RawError, RawErrorKind, ResultEnd,
    ServerInfo, TxStatus, classify,
};
use hacivat_guard::{
    GapReason, GuardPlan, PostCheck, PostFacts, ResetKind, RoleFindings, RoleWarning, TxStatusLite,
    access_label, check_functions, check_input, check_post, interpret_probe, plan_for,
    remediation_for, role_probe_sql,
};
use hacivat_proto::{AccessLabel, DbmsKind, ErrClass};

// ## m2b-t23-conformance (named exception, T17 carry P1)
use crate::session::recovery::connection_ending;
use crate::session::schema::{Family, family};
use crate::session::{DenyLayer, ExecError, ExecErrorKind, Reply};

/// First keywords a read-only statement may not start with (Q35): they end or nest the guard's
/// transaction. This list produces `DenyLayer::Transaction`; it is not a classifier, and the
/// post-checks still catch what gets past it.
const TRANSACTION_KEYWORDS: [&str; 8] = [
    "COMMIT",
    "ROLLBACK",
    "BEGIN",
    "START",
    "END",
    "ABORT",
    "SAVEPOINT",
    "RELEASE",
];

/// How long the role probe may take before the label is guarded (it is then `Unparsed`).
const PROBE_LIMIT: Duration = Duration::from_secs(5);

/// The most rows a probe statement may answer; more is `Unparsed`.
const PROBE_MAX_ROWS: usize = 10_000;

/// What one guarded request did: its reply, and the class of the broken session when the
/// connection must not be used again.
#[derive(Debug)]
pub(crate) struct Guarded {
    pub(crate) reply: Reply,
    pub(crate) discard: Option<ErrClass>,
}

impl Guarded {
    fn keep(reply: Reply) -> Self {
        Guarded {
            reply,
            discard: None,
        }
    }

    fn discard(error: ExecError, class: ErrClass) -> Self {
        Guarded {
            reply: Err(error),
            discard: Some(class),
        }
    }
}

/// Runs `sql` on `conn` inside `kind`'s guard plan; `replay` is the session's
/// `EnvPlan::replay()` (its environment without PostgreSQL's client identity), run after the
/// reset (Q15). The module docs list the steps.
pub fn run_guarded<'a>(
    conn: &'a mut dyn RawConn,
    kind: DbmsKind,
    sql: &'a str,
    opts: ExecOpts,
    replay: &'a [String],
) -> BoxFuture<'a, Result<ExecSummary, ExecError>> {
    Box::pin(async move { guarded(conn, kind, sql, opts, replay).await.reply })
}

/// [`run_guarded`], and whether the connection must be discarded (the actor's entry point).
pub(crate) async fn guarded(
    conn: &mut dyn RawConn,
    kind: DbmsKind,
    sql: &str,
    opts: ExecOpts,
    replay: &[String],
) -> Guarded {
    // 1, 1a, 1b.
    let keywords = match precheck(kind, sql) {
        Ok(keywords) => keywords,
        Err(denied) => return Guarded::keep(Err(denied)),
    };
    // An engine without a guard plan has no read-only guard: nothing runs.
    let Some(plan) = plan_for(kind) else {
        return Guarded::discard(ExecError::of(ExecErrorKind::Internal), ErrClass::Internal);
    };

    // 2.
    if let Err(failure) = open_guard(conn, kind, &plan).await {
        return failure.into_guarded();
    }

    // 3.
    let ran = conn
        .execute(
            sql,
            ExecOpts {
                protocol: Protocol::Extended,
                ..opts
            },
        )
        .await;
    if let Err(e) = &ran
        && let Some(class) = lost_class(kind, e)
    {
        return Guarded::discard(statement_error(kind, e, true), class);
    }

    // 4–8.
    let outcome = match &ran {
        Ok(summary) => Ran {
            ok: true,
            command_tag: summary.command_tag.clone(),
            rows_affected: summary.rows_affected,
        },
        Err(_) => Ran::default(),
    };
    if let Err(failure) = post_steps(conn, kind, &plan, &keywords, outcome, replay).await {
        return failure.into_guarded();
    }
    Guarded::keep(ran.map_err(|e| statement_error(kind, &e, true)))
}

/// Steps 1, 1a and 1b, on the text alone (nothing is sent): `check_input`, the transaction
/// keywords in every reading of the first keyword, and PostgreSQL's function check. `Ok` holds
/// those readings (step 7 judges them).
fn precheck(kind: DbmsKind, sql: &str) -> Result<Keywords, ExecError> {
    let denied = |layer| Err(ExecError::of(ExecErrorKind::ReadOnlyDenied(layer)));
    // 1.
    if check_input(sql).is_err() {
        return denied(DenyLayer::Input);
    }
    // 1a.
    let keywords = guard_keywords(kind, sql);
    if keywords.controls_transaction() {
        return denied(DenyLayer::Transaction);
    }
    // ## m2b-t23-f1: 1b. A call no rollback undoes (PostgreSQL; a tripwire, the probe's row 6
    // keeps the label honest).
    if let Err(violation) = check_functions(kind, sql) {
        tracing::debug!(
            ?violation,
            "a read-only statement calls a non-transactional function"
        );
        return denied(DenyLayer::Function);
    }
    Ok(keywords)
}

/// Step 2: the plan's `before` statements (`batch`, trusted text only): if one fails, nothing
/// else runs; then the guard's transaction must be open, else a gap and the statement is never
/// sent.
async fn open_guard(
    conn: &mut dyn RawConn,
    kind: DbmsKind,
    plan: &GuardPlan,
) -> Result<(), GuardFailure> {
    for statement in &plan.before {
        if let Err(e) = conn.batch(statement).await {
            let error = statement_error(kind, &e, false);
            // Kept only when nothing of the guard is left open on a live connection. A lost
            // connection's status is never read (T23, T17 carry P1: a driver's status after a
            // loss says nothing about the guard's transaction).
            let discard = lost_class(kind, &e)
                .or_else(|| (conn.tx_status() != TxStatus::Idle).then_some(ErrClass::Internal));
            return Err(GuardFailure { error, discard });
        }
    }
    // Fix round 1 (m-1): the user statement runs only inside the guard's read-only
    // transaction. A `before` that did not open one is a gap; nothing more is sent.
    if !plan.before.is_empty() && conn.tx_status() != TxStatus::InTx {
        tracing::warn!("the guard's read-only transaction did not open");
        return Err(GuardFailure::gap(GapReason::TransactionEndedByStatement));
    }
    Ok(())
}

/// What the user statement did, as steps 5 to 7 judge it. The default is a statement that
/// failed or (a typed query) was closed before its end: no tag, no count.
#[derive(Debug, Clone, Default)]
struct Ran {
    /// It ran to its end without an error.
    ok: bool,
    command_tag: Option<String>,
    rows_affected: Option<u64>,
}

/// Steps 4 to 8, once the user statement is over (M2b: `execute` returned; the typed path: the
/// query is closed). Every `GuardFailure` from here discards the connection.
///
/// - 4: the transaction status after the statement;
/// - 5: PostgreSQL's canary (`execute(Simple)`), while the guard's transaction can answer;
/// - 6: the plan's `after` statements (`ROLLBACK`, `execute(Simple)`), with their notices;
/// - 7: `check_post`, after a failed statement too (except a MySQL implicit-commit DDL and
///   SQLite): a gap is `EnforcementGap(reason)`. A statement that ran to its end is also judged
///   under every reading of its first keyword ([`verdict_of_readings`]);
/// - 8: the connection's reset and the session's environment again (`replay`, Q15); the
///   connection must then report no transaction.
async fn post_steps(
    conn: &mut dyn RawConn,
    kind: DbmsKind,
    plan: &GuardPlan,
    keywords: &Keywords,
    ran: Ran,
    replay: &[String],
) -> Result<(), GuardFailure> {
    let discard = |error: ExecError, class: ErrClass| {
        Err(GuardFailure {
            error,
            discard: Some(class),
        })
    };
    // 4.
    let tx_after = lite(conn.tx_status());
    let mut facts = PostFacts {
        tx_after_statement: Some(tx_after),
        command_tag: ran.command_tag,
        rows_affected: ran.rows_affected,
        ..PostFacts::default()
    };

    // 5. Only where the guard's transaction can still answer: after a failed statement
    // PostgreSQL's transaction is aborted, and nothing in it can be committed.
    let mut canary_unread = false;
    if let Some(canary) = plan.canary
        && (ran.ok || tx_after == TxStatusLite::InTx)
    {
        match conn.execute(canary, simple(1)).await {
            Ok(answer) => match answer.rows.as_slice() {
                [row] if row.len() == 1 && !answer.truncated => {
                    facts.canary = Some(row[0].clone());
                }
                _ => canary_unread = true,
            },
            Err(e) => {
                if let Some(class) = lost_class(kind, &e) {
                    return discard(statement_error(kind, &e, true), class);
                }
                canary_unread = true;
            }
        }
    }

    // 6.
    for statement in &plan.after {
        match conn.execute(statement, simple(1)).await {
            Ok(answer) => facts.rollback_notice_codes.extend(answer.notice_codes),
            Err(e) => {
                tracing::warn!(kind = ?e.kind, "the guard's rollback failed");
                let class = lost_class(kind, &e).unwrap_or(ErrClass::Internal);
                return discard(statement_error(kind, &e, true), class);
            }
        }
    }

    // 7. A failed statement is judged by its transaction state too (it has no command tag):
    // on PostgreSQL an error inside the transaction leaves it failed, so no transaction means
    // the statement ended it; a MySQL procedure may commit, write, commit and then fail
    // (fix round 1, I-2). The one exception: MySQL commits implicitly before a DDL or
    // administration statement and then refuses it under the read-only setting, which escapes
    // nothing, so a failed statement every reading of whose first keyword is in
    // `IMPLICIT_COMMIT_KEYWORDS` is not judged. SQLite runs in autocommit.
    let ddl_committed = family(kind) == Family::MySql && keywords.all_in(&IMPLICIT_COMMIT_KEYWORDS);
    let verdict = if ran.ok {
        verdict_of_readings(kind, &facts, keywords)
    } else if ddl_committed || family(kind) == Family::Sqlite {
        PostCheck::Clean
    } else {
        check_post(kind, &facts)
    };
    if let PostCheck::Gap(reason) = verdict {
        tracing::warn!(?reason, "a read-only statement escaped the guard");
        return Err(GuardFailure::gap(reason));
    }
    if canary_unread {
        tracing::warn!("the guard's canary could not be read");
        return discard(ExecError::of(ExecErrorKind::Internal), ErrClass::Internal);
    }

    // 8.
    if plan.reset != ResetKind::None {
        if let Err(e) = conn.reset().await {
            tracing::warn!(kind = ?e.kind, "the guard's reset failed");
            let class = lost_class(kind, &e).unwrap_or(ErrClass::Internal);
            return discard(ExecError::of(ExecErrorKind::Internal), class);
        }
        for statement in replay {
            if let Err(e) = conn.batch(statement).await {
                tracing::warn!(kind = ?e.kind, "the guard's environment replay failed");
                let class = lost_class(kind, &e).unwrap_or(ErrClass::Internal);
                return discard(ExecError::of(ExecErrorKind::Internal), class);
            }
        }
    }
    if conn.tx_status() != TxStatus::Idle {
        tracing::warn!("a guarded request left a transaction open");
        return discard(ExecError::of(ExecErrorKind::Internal), ErrClass::Internal);
    }
    Ok(())
}

// ## m3a-t22-typed-readonly
/// Step 7 for a statement that ran to its end: `check_post` on the facts as they are, then once
/// more with every reading of the first keyword as the command tag; the first gap wins. A
/// MySQL or MariaDB command tag is the text's first keyword read as code (the driver's
/// `command_tag`), so `/*!99999 SELECT 1 */ CREATE TABLE …` is tagged `SELECT` where a current
/// server runs `CREATE` (T14 review m14, routed to Task 22): every way the server may read the
/// text counts. A failed statement is never judged this way (a refused `CREATE` would be read
/// as a schema write).
fn verdict_of_readings(kind: DbmsKind, facts: &PostFacts, keywords: &Keywords) -> PostCheck {
    if let PostCheck::Gap(reason) = check_post(kind, facts) {
        return PostCheck::Gap(reason);
    }
    for word in keywords.words().filter(|w| !w.is_empty()) {
        let read = PostFacts {
            command_tag: Some(word.to_string()),
            ..facts.clone()
        };
        if let PostCheck::Gap(reason) = check_post(kind, &read) {
            return PostCheck::Gap(reason);
        }
    }
    PostCheck::Clean
}

/// `ExecOpts` for the guard's own statements: the simple protocol (several results, notices),
/// at most `max_rows` rows.
fn simple(max_rows: usize) -> ExecOpts {
    ExecOpts {
        max_rows,
        protocol: Protocol::Simple,
    }
}

fn lite(status: TxStatus) -> TxStatusLite {
    match status {
        TxStatus::Idle => TxStatusLite::Idle,
        TxStatus::InTx => TxStatusLite::InTx,
        TxStatus::Failed => TxStatusLite::Failed,
    }
}

/// The class of a lost connection, when `e` is one.
///
/// ## m2b-t23-conformance (named exception, T17 carry P1): a server error that ends the
/// connection is a loss too, by the session's own rule (`recovery::connection_ending`: a
/// PostgreSQL `FATAL`/`PANIC` of a recovery class such as `57P01`, MySQL's connection-ending
/// codes). Before, a `FATAL 57P01` answering the guard's `BEGIN` read the dead connection's
/// status and ended the profile `Lost { internal }` instead of reconnecting the idle session.
fn lost_class(kind: DbmsKind, e: &RawError) -> Option<ErrClass> {
    let class = || classify(kind, Phase::PostConnect, e).class;
    match e.kind {
        RawErrorKind::Io(_)
        | RawErrorKind::Protocol
        | RawErrorKind::Timeout
        | RawErrorKind::Tls(_) => Some(class()),
        RawErrorKind::Db => connection_ending(kind, e, class()),
        _ => None,
    }
}

/// A failed statement as the session reports it. A server refusal is `Db` with the server's
/// message, whatever its code (the read-only codes among them: PostgreSQL `25006`, `42501`;
/// MySQL `1792`, `1142`, `1290`; SQLite `SQLITE_READONLY`, `SQLITE_AUTH`). A statement the
/// read-only connection will not prepare (MySQL's `ER_UNSUPPORTED_PS`) is refused by the
/// protocol layer. `sent`: the statement may have reached the server.
///
/// ## m2b-t23-conformance (named exception, T17 carry P1): a server error that ends the
/// connection ([`lost_class`]) is `ConnectionLost`, with the server's message kept, as on a
/// read-write session (T17 I6).
fn statement_error(engine: DbmsKind, e: &RawError, sent: bool) -> ExecError {
    let kind = match e.kind {
        RawErrorKind::Db if lost_class(engine, e).is_some() => ExecErrorKind::ConnectionLost {
            outcome_unknown: sent,
        },
        RawErrorKind::Db => ExecErrorKind::Db,
        RawErrorKind::Cancelled => ExecErrorKind::Cancelled,
        RawErrorKind::Unsupported => ExecErrorKind::ReadOnlyDenied(DenyLayer::Protocol),
        RawErrorKind::Io(_)
        | RawErrorKind::Protocol
        | RawErrorKind::Timeout
        | RawErrorKind::Tls(_) => ExecErrorKind::ConnectionLost {
            outcome_unknown: sent,
        },
        _ => ExecErrorKind::Internal,
    };
    ExecError {
        kind,
        server: e.server.clone(),
    }
}

/// How many readings of one statement's first keyword are followed (each version-gated
/// executable comment doubles them). Past it the readings are incomplete, which fails closed.
const MAX_READINGS: usize = 64;

/// The first keyword of a statement as its server may read it: one reading per way the
/// server's lexer can go, each ASCII upper-cased. `Some("")` when the statement starts with
/// something other than a word (`(`); `None` when the reading holds no statement at all (only
/// comments, as when a version-gated comment holds the whole statement and is skipped).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Keywords {
    readings: Vec<Option<String>>,
    /// False when [`MAX_READINGS`] or [`MAX_FORKS`] cut the readings short.
    complete: bool,
    /// The version-gated comments already forked at, by position: both readings of one reach
    /// the same text after it, so it is followed once (review n-1: a chain of them stays
    /// linear instead of doubling).
    forked: HashSet<usize>,
}

impl Keywords {
    fn words(&self) -> impl Iterator<Item = &str> {
        self.readings.iter().flatten().map(String::as_str)
    }

    /// Step 1a: some reading is a transaction keyword, or the readings are incomplete.
    fn controls_transaction(&self) -> bool {
        !self.complete || self.words().any(|k| TRANSACTION_KEYWORDS.contains(&k))
    }

    /// Every reading that holds a statement starts with one of `words`, and one does. A
    /// reading without a statement (a skipped version-gated comment that held all of it) runs
    /// nothing on the server, so it neither commits nor counts (review n-4).
    fn all_in(&self, words: &[&str]) -> bool {
        self.complete && self.words().next().is_some() && self.words().all(|k| words.contains(&k))
    }
}

/// First keywords of MySQL statements that commit implicitly before they run (T13), so a
/// failed one may leave no transaction without having escaped anything: the server refuses
/// it afterwards under the session's read-only setting. An allowlist: any other keyword after
/// a failed statement is judged (fix round 1, I-2). Fix round 2 (review I-2 note) adds the
/// preparable replication and administration verbs `RESET`, `CHANGE` and `STOP`.
const IMPLICIT_COMMIT_KEYWORDS: [&str; 18] = [
    "CREATE",
    "ALTER",
    "DROP",
    "RENAME",
    "TRUNCATE",
    "GRANT",
    "REVOKE",
    "ANALYZE",
    "OPTIMIZE",
    "REPAIR",
    "FLUSH",
    "INSTALL",
    "UNINSTALL",
    "CACHE",
    "LOAD",
    "RESET",
    "CHANGE",
    "STOP",
];

/// The lexer rules of one engine that decide where a statement's first keyword is.
#[derive(Debug, Clone, Copy)]
struct Lexer {
    /// MySQL and MariaDB: `#` comments, `--` only before whitespace or a control character,
    /// `/*!…*/` executable comments.
    mysql: bool,
    /// MariaDB: `/*M!…*/` is executable too (an ordinary comment on MySQL).
    mariadb: bool,
    /// PostgreSQL: block comments nest, and a line comment also ends at a carriage return.
    postgres: bool,
}

/// The first keyword of `sql` as `kind`'s server reads it, for steps 1a and 7.
///
/// - Skipped: ASCII whitespace (vertical tab included), `;` (an empty statement), `--` line
///   comments (on MySQL and MariaDB only before whitespace or a control character), `#` line
///   comments on MySQL and MariaDB, and block comments. A line comment ends at a line feed; on
///   PostgreSQL also at a carriage return (its lexer's `non_newline` is `[^\n\r]`).
/// - PostgreSQL block comments nest; other engines' do not.
/// - MySQL and MariaDB run executable comments (T13 carry): `/*!…*/`, and on MariaDB only
///   `/*M!…*/`. An unversioned one always runs, so the keyword is read inside it. A
///   version-gated one (`/*!NNNNN …*/`) runs or is skipped depending on the server's version,
///   so it is read both ways: inside it, and after its end (fix round 1, I-1). Callers judge
///   every reading: step 1a refuses when any reading is a transaction keyword.
pub(crate) fn guard_keywords(kind: DbmsKind, sql: &str) -> Keywords {
    let lexer = Lexer {
        mysql: family(kind) == Family::MySql,
        mariadb: kind == DbmsKind::MariaDb,
        postgres: family(kind) == Family::Postgres,
    };
    let mut keywords = Keywords {
        readings: Vec::new(),
        complete: true,
        forked: HashSet::new(),
    };
    read_from(sql.as_bytes(), 0, 0, lexer, &mut keywords);
    keywords
}

/// How many version-gated comments one reading may fork at (the recursion depth; a statement
/// may hold a million of them). Past it the readings are incomplete, which fails closed.
const MAX_FORKS: usize = 16;

/// Nesting levels a block comment may hold: PostgreSQL's nest without a bound; MySQL's and
/// MariaDB's do not, except that a skipped version-gated comment may hold one level
/// (`consume_comment(lip, 1)`, "version allowed to have one level of comment inside"; fix
/// round 2, I-1b).
const NESTS_FREELY: u32 = u32::MAX;
const SKIPPED_VERSIONED_NESTING: u32 = 1;

/// Follows one reading from `i`, forking at each version-gated executable comment; `forks` is
/// how many forks led here.
fn read_from(b: &[u8], mut i: usize, forks: usize, lexer: Lexer, out: &mut Keywords) {
    // Once incomplete, the verdict is already the strictest: stop.
    if !out.complete || out.readings.len() >= MAX_READINGS || forks > MAX_FORKS {
        out.complete = false;
        return;
    }
    let at = |i: usize| b.get(i).copied();
    loop {
        let Some(c) = at(i) else {
            out.readings.push(None);
            return;
        };
        if c.is_ascii_whitespace() || c == b'\x0b' || c == b';' {
            i += 1;
        } else if c == b'-'
            && at(i + 1) == Some(b'-')
            && (!lexer.mysql
                || at(i + 2).is_none_or(|n| n.is_ascii_whitespace() || n.is_ascii_control()))
        {
            i = line_end(b, i + 2, lexer.postgres);
        } else if c == b'#' && lexer.mysql {
            i = line_end(b, i + 1, false);
        } else if c == b'/' && at(i + 1) == Some(b'*') {
            let executable = match (at(i + 2), at(i + 3)) {
                (Some(b'!'), _) if lexer.mysql => Some(i + 3),
                (Some(b'M'), Some(b'!')) if lexer.mariadb => Some(i + 4),
                _ => None,
            };
            if let Some(start) = executable {
                let mut text = start;
                while at(text).is_some_and(|d| d.is_ascii_digit()) {
                    text += 1;
                }
                if text == start {
                    // Always run: its text is the statement's.
                    i = start;
                    continue;
                }
                // Version-gated: run (its text) or skipped (what follows it). Both readings from
                // here were followed already if another reading reached this comment.
                if !out.forked.insert(i) {
                    return;
                }
                read_from(b, text, forks + 1, lexer, out);
                match block_end(b, i, SKIPPED_VERSIONED_NESTING) {
                    Some(end) => read_from(b, end, forks + 1, lexer, out),
                    None => out.readings.push(None),
                }
                return;
            }
            let nesting = if lexer.postgres { NESTS_FREELY } else { 0 };
            match block_end(b, i, nesting) {
                Some(end) => i = end,
                None => {
                    out.readings.push(None);
                    return;
                }
            }
        } else if c == b'*' && at(i + 1) == Some(b'/') && lexer.mysql {
            // The end of an executable comment the reading went into.
            i += 2;
        } else {
            out.readings.push(Some(
                b[i..]
                    .iter()
                    .take_while(|c| c.is_ascii_alphanumeric() || **c == b'_')
                    .map(|c| char::from(c.to_ascii_uppercase()))
                    .collect(),
            ));
            return;
        }
    }
}

/// The index after the line comment whose text starts at `from`: after the next line feed, or
/// also a carriage return when `cr_ends` (PostgreSQL).
fn line_end(b: &[u8], from: usize, cr_ends: bool) -> usize {
    b[from..]
        .iter()
        .position(|c| *c == b'\n' || (cr_ends && *c == b'\r'))
        .map_or(b.len(), |p| from + p + 1)
}

/// The index after the block comment that opens at `start`; `None` when it never ends. A `/*`
/// inside opens a nested comment only while fewer than `nesting` levels are open; past that it
/// is plain text (as in MySQL's `consume_comment`).
fn block_end(b: &[u8], start: usize, nesting: u32) -> Option<usize> {
    let mut depth = 1u32;
    let mut i = start + 2;
    while i + 1 < b.len() {
        match (b[i], b[i + 1]) {
            (b'*', b'/') => {
                depth -= 1;
                i += 2;
                if depth == 0 {
                    return Some(i);
                }
            }
            (b'/', b'*') if depth <= nesting => {
                depth += 1;
                i += 2;
            }
            _ => i += 1,
        }
    }
    None
}

/// The access label of a fresh connection and the role probe's warnings (MCP-09 step 1).
///
/// - Read-write: `ReadWrite`, no probe.
/// - Read-only on PostgreSQL or MySQL: the fixed statements of `role_probe_sql`, each through
///   `execute(Simple)`, read by `interpret_probe`. A probe that fails, answers too many rows
///   or takes longer than [`PROBE_LIMIT`] is `Unparsed`, so guarded.
/// - Read-only on MariaDB (C19), SQLite or any other engine: `ReadOnlyGuarded`, no probe.
///
/// The engine is the connection's as well as the profile's: a MySQL profile whose server is
/// MariaDB is MariaDB, and two engines that disagree otherwise are `Unparsed`.
pub fn access_for<'a>(
    conn: &'a mut dyn RawConn,
    kind: DbmsKind,
    read_only: bool,
) -> BoxFuture<'a, (AccessLabel, Vec<RoleWarning>)> {
    Box::pin(async move {
        if !read_only {
            return (AccessLabel::ReadWrite, Vec::new());
        }
        let actual = conn.kind();
        let kind = if kind == actual {
            kind
        } else if kind == DbmsKind::MariaDb || actual == DbmsKind::MariaDb {
            DbmsKind::MariaDb
        } else {
            return (AccessLabel::ReadOnlyGuarded, vec![RoleWarning::Unparsed]);
        };
        if !matches!(kind, DbmsKind::Postgres | DbmsKind::MySql) {
            return (access_label(kind, true, None), Vec::new());
        }
        let findings = match tokio::time::timeout(PROBE_LIMIT, probe(conn, kind)).await {
            Ok(Some(answers)) => interpret_probe(kind, &answers),
            _ => RoleFindings {
                least_privilege: false,
                warnings: vec![RoleWarning::Unparsed],
            },
        };
        (access_label(kind, true, Some(&findings)), findings.warnings)
    })
}

/// PostgreSQL's identity check before the probe (fix round 1, I-3): the probe judges
/// `current_user`'s roles, but `SET ROLE NONE` returns to the session user, so the two must be
/// the same role. `current_user` and `session_user` are SQL keywords, not names the
/// `search_path` resolves, and the comparison is done here, not by a server operator.
const PG_IDENTITY_SQL: &str = "SELECT current_user, session_user";

/// Whether the identity check's answer is one row of two equal, non-NULL names.
fn same_role(answer: &ExecSummary) -> bool {
    matches!(
        answer.rows.as_slice(),
        [row] if matches!(row.as_slice(), [Some(current), Some(session)] if current == session)
    ) && !answer.truncated
}

/// The probe's row sets; `None` when a statement failed or answered too many rows, or (on
/// PostgreSQL) the reset failed or the connection runs as a role other than its session user.
///
/// PostgreSQL first resets the connection the way every guarded request ends (fix round 2,
/// I-3b): the reset's `SET SESSION AUTHORIZATION DEFAULT` returns to the signed-in login and
/// its `RESET ALL` restores a startup or role-default `role`, so the identity check and the
/// probe judge the identity every guarded request after the first runs as. Without it a
/// superuser login with a startup `session_authorization` reads as the reader it named, and
/// the first guarded request's reset turns it back into the superuser.
async fn probe(conn: &mut dyn RawConn, kind: DbmsKind) -> Option<Vec<Vec<Vec<Option<String>>>>> {
    if kind == DbmsKind::Postgres {
        if let Err(e) = conn.reset().await {
            tracing::debug!(kind = ?e.kind, "the role probe's reset failed");
            return None;
        }
        match conn.execute(PG_IDENTITY_SQL, simple(1)).await {
            Ok(answer) if same_role(&answer) => {}
            Ok(_) => {
                tracing::debug!("the session runs as another role than its session user");
                return None;
            }
            Err(e) => {
                tracing::debug!(kind = ?e.kind, "the role probe's identity check failed");
                return None;
            }
        }
    }
    let mut answers = Vec::new();
    for statement in role_probe_sql(kind) {
        match conn.execute(statement, simple(PROBE_MAX_ROWS)).await {
            Ok(answer) if !answer.truncated => answers.push(answer.rows),
            Ok(_) => return None,
            Err(e) => {
                tracing::debug!(kind = ?e.kind, "a role probe statement failed");
                return None;
            }
        }
    }
    Some(answers)
}

/// The remediation SQL for a connection to `server`: its engine and major version, so
/// PostgreSQL before 14 gets the schema-grant form (with a `<schema>` placeholder) and an
/// unknown version the current one (`hacivat_guard::remediation_for`).
pub fn remediation_for_server(server: &ServerInfo) -> Option<&'static str> {
    remediation_for(server.kind, server.version.as_ref().map(|v| v.major))
}

// ## m3a-t01b-seams, m3a-t22-typed-readonly
// The typed read-only path (HV-SESS-04 on typed execution, R-M3-17; Task 22). A console cursor
// outlives the request that opened it, so the guard's steps are three calls around the
// cursor's life, with the guard's state ([`GuardedCursor`]) between them:
//
// 1. [`guarded_query`]: steps 1–3. `check_input` (else `ReadOnlyDenied(Input)`), the
//    transaction keywords in every reading (`ReadOnlyDenied(Transaction)`), PostgreSQL's
//    function check (`ReadOnlyDenied(Function)`), `plan.before` through `batch` (the run sets
//    `TxOwner::Guard`), then the typed `query`, always `Protocol::Extended`. User text never
//    goes through `batch`. When the `query` call itself fails (a refused prepare, a statement
//    the read-only connection will not prepare) the post steps run here, before the failure is
//    returned: the run loop then has no hold to end.
// 2. [`GuardedCursor::after_first_chunk`]: PostgreSQL only, at the first `Suspended`: the
//    guard's transaction must still be open, and the canary runs while the named portal stays
//    open (the PostgreSQL exception to "one open query per connection", P5). A transaction ID
//    already assigned is `CanaryAssigned`: the connection is discarded at once and the result
//    is `Closed { EnforcementGap }` with its rows. MySQL and SQLite take no statement beside an
//    open query, so their checks wait for the end.
// 3. [`GuardedCursor::finish`]: steps 4–8 ([`post_steps`]) at the cursor's end (complete,
//    released, cancelled, failed or idle-bounded), after the actor closed the query: the
//    canary (PostgreSQL), `ROLLBACK`, `check_post`, `reset`, `replay`. The post steps wait for
//    the cursor (R-M3-17), and no other statement joins the guard's transaction: the actor
//    releases the cursor before any other request (D-019). A statement that ran to its end is
//    judged by its `ResultEnd` (MySQL: the final OK packet's affected rows, `WriteSucceeded`);
//    one closed early or failed by its transaction state only (as M2b judges a failed one).
//
// A `GuardFailure` with `discard` set breaks the session (the actor discards the connection);
// with `ExecErrorKind::EnforcementGap` the actor also emits `SessionEvent::EnforcementGap` and
// closes the result `Closed { EnforcementGap }` with its rows kept (Task 18 acts on both).
//
// SQLite: steps 1, 1a, 1b, the query, then the post-check of a completed statement (a
// tripwire); its layers are the connection's (the authorizer and `stmt_readonly`, Task 15).

/// A failed step of the typed guard path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GuardFailure {
    pub(crate) error: ExecError,
    /// The class of the broken session when the connection must not be used again.
    pub(crate) discard: Option<ErrClass>,
}

impl GuardFailure {
    /// The guard found an escape: the connection is discarded unreset.
    fn gap(reason: GapReason) -> GuardFailure {
        GuardFailure {
            error: ExecError::of(ExecErrorKind::EnforcementGap(reason)),
            discard: Some(ErrClass::Internal),
        }
    }

    /// The guard could not tell: the connection is discarded.
    fn internal() -> GuardFailure {
        GuardFailure {
            error: ExecError::of(ExecErrorKind::Internal),
            discard: Some(ErrClass::Internal),
        }
    }

    /// The same failure as the M2b path reports it.
    fn into_guarded(self) -> Guarded {
        Guarded {
            reply: Err(self.error),
            discard: self.discard,
        }
    }
}

/// The guard's state while its typed query is open: what the canary and the post steps need.
#[derive(Debug)]
pub(crate) struct GuardedCursor {
    pub(crate) kind: DbmsKind,
    plan: GuardPlan,
    /// The readings of the statement's first keyword (step 7).
    keywords: Keywords,
}

/// Steps 1–3 for `spec` on a read-only session, with the session's environment `replay` (run
/// after the reset when the query fails at once). `Ok`: the query is open inside the guard's
/// read-only transaction.
pub(crate) async fn guarded_query(
    conn: &mut dyn RawConn,
    kind: DbmsKind,
    spec: QuerySpec<'_>,
    replay: &[String],
) -> Result<GuardedCursor, GuardFailure> {
    // 1, 1a, 1b: nothing is sent.
    let keywords = precheck(kind, spec.sql).map_err(|error| GuardFailure {
        error,
        discard: None,
    })?;
    // An engine without a guard plan has no read-only guard: nothing runs.
    let Some(plan) = plan_for(kind) else {
        return Err(GuardFailure::internal());
    };
    // 2.
    open_guard(conn, kind, &plan).await?;
    // 3. The extended (prepared) protocol only, inside the guard's transaction.
    let queried = conn
        .query(QuerySpec {
            protocol: Protocol::Extended,
            ..spec
        })
        .await;
    let guard = GuardedCursor {
        kind,
        plan,
        keywords,
    };
    let Err(e) = queried else {
        return Ok(guard);
    };
    if let Some(class) = lost_class(kind, &e) {
        return Err(GuardFailure {
            error: statement_error(kind, &e, true),
            discard: Some(class),
        });
    }
    // The statement was refused before a result: the guard ends here, as M2b's does after a
    // failed statement (MySQL's prepare left the read-only transaction open; PostgreSQL's
    // failed it).
    post_steps(
        conn,
        kind,
        &guard.plan,
        &guard.keywords,
        Ran::default(),
        replay,
    )
    .await?;
    Err(GuardFailure {
        error: statement_error(kind, &e, true),
        discard: None,
    })
}

impl GuardedCursor {
    /// After the first chunk, while the query is suspended: on PostgreSQL the guard's
    /// transaction must still be open and the canary must answer SQL NULL. A gap discards the
    /// connection; so does a canary that cannot be read (it may have failed the transaction the
    /// portal lives in). Other engines: nothing (no statement runs beside their open query).
    pub(crate) async fn after_first_chunk(
        &mut self,
        conn: &mut dyn RawConn,
    ) -> Result<(), GuardFailure> {
        let Some(canary) = self.plan.canary else {
            return Ok(());
        };
        if conn.tx_status() != TxStatus::InTx {
            tracing::warn!("a read-only query left the guard's transaction partway");
            return Err(GuardFailure::gap(GapReason::TransactionEndedByStatement));
        }
        match conn.execute(canary, simple(1)).await {
            Ok(answer) => match answer.rows.as_slice() {
                [row] if row.len() == 1 && !answer.truncated => match &row[0] {
                    None => Ok(()),
                    Some(_) => {
                        tracing::warn!("a read-only query was assigned a transaction ID");
                        Err(GuardFailure::gap(GapReason::CanaryAssigned))
                    }
                },
                _ => {
                    tracing::warn!("the guard's canary could not be read");
                    Err(GuardFailure::internal())
                }
            },
            Err(e) => {
                tracing::warn!(kind = ?e.kind, "the guard's canary failed");
                let class = lost_class(self.kind, &e).unwrap_or(ErrClass::Internal);
                Err(GuardFailure {
                    error: statement_error(self.kind, &e, true),
                    discard: Some(class),
                })
            }
        }
    }

    /// Steps 4–8 at the cursor's end, once the actor closed the query: `completed` when the
    /// statement ran to its end (its `end` then holds the command tag and affected rows the
    /// post-check reads), else it failed or was closed early and is judged by its transaction
    /// state. `replay` is the session's environment, run after the reset.
    pub(crate) async fn finish(
        self,
        conn: &mut dyn RawConn,
        replay: &[String],
        completed: bool,
        end: Option<&ResultEnd>,
    ) -> Result<(), GuardFailure> {
        let ran = match end {
            Some(end) if completed => Ran {
                ok: true,
                command_tag: end.command_tag.clone(),
                rows_affected: end.rows_affected,
            },
            // A statement that completed without a result end (a drain read it to its end):
            // judged as completed, without a tag.
            None if completed => Ran {
                ok: true,
                ..Ran::default()
            },
            _ => Ran::default(),
        };
        post_steps(conn, self.kind, &self.plan, &self.keywords, ran, replay).await
    }
}
