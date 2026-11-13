//! Guarded read-only execution and the role probe (HV-SESS-04, SQL-SAFE-05, MCP-09 step 1,
//! B21; Task 18): every statement of a read-only session runs inside the guard plan of
//! `hacivat_guard::plan_for`, with the post checks, and the connection is labelled by the role
//! probe.
//!
//! [`run_guarded`], for PostgreSQL, MySQL and MariaDB:
//! - 1: `check_input`, else `ReadOnlyDenied(Input)`;
//! - 1a: any reading of the first keyword (`guard_keywords`, as the server's lexer reads it)
//!   is a transaction keyword → `ReadOnlyDenied(Transaction)` (Q35);
//! - 2: the plan's `before` statements (`batch`): if one fails, nothing else runs; then the
//!   guard's transaction must be open, else a gap and the statement is never sent;
//! - 3: the user statement, always through the extended protocol (one statement, prepared);
//! - 4: the transaction status after it;
//! - 5: PostgreSQL's canary (`execute(Simple)`), while the guard's transaction can answer;
//! - 6: the plan's `after` statements (`ROLLBACK`, `execute(Simple)`), with their notices;
//! - 7: `check_post`, after a failed statement too (except a MySQL implicit-commit DDL): a gap
//!   is `EnforcementGap(reason)`, and the connection is discarded unreset;
//! - 8: the connection's reset and the session's environment again (`replay`, Q15).
//!
//! If step 3 fails, steps 4 to 8 still run. SQLite runs steps 1 and 1a, then the statement and
//! the post-check (a tripwire): its layers are connection-level (Task 11), its plan has no
//! statements and no reset.
//!
//! Everything fails closed. The connection is discarded (`Guarded::discard`, which the actor
//! turns into a broken session) after a gap, a lost connection, a guard statement that failed
//! after the guard's transaction began, a canary that could not be read, a failed reset or
//! replay, and whenever the connection still reports a transaction at the end. User text never
//! goes through `batch`, the trusted path.

use std::time::Duration;

use futures::future::BoxFuture;
use hacivat_driver::{
    ExecOpts, ExecSummary, Phase, Protocol, RawConn, RawError, RawErrorKind, ServerInfo, TxStatus,
    classify,
};
use hacivat_guard::{
    GapReason, PostCheck, PostFacts, ResetKind, RoleFindings, RoleWarning, TxStatusLite,
    access_label, check_input, check_post, interpret_probe, plan_for, remediation_for,
    role_probe_sql,
};
use hacivat_proto::{AccessLabel, DbmsKind, ErrClass};

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
    // 1.
    if check_input(sql).is_err() {
        return Guarded::keep(Err(ExecError::of(ExecErrorKind::ReadOnlyDenied(
            DenyLayer::Input,
        ))));
    }
    // 1a.
    let keywords = guard_keywords(kind, sql);
    if keywords.controls_transaction() {
        return Guarded::keep(Err(ExecError::of(ExecErrorKind::ReadOnlyDenied(
            DenyLayer::Transaction,
        ))));
    }
    // An engine without a guard plan has no read-only guard: nothing runs.
    let Some(plan) = plan_for(kind) else {
        return Guarded::discard(ExecError::of(ExecErrorKind::Internal), ErrClass::Internal);
    };

    // 2.
    for statement in &plan.before {
        if let Err(e) = conn.batch(statement).await {
            let error = statement_error(&e, false);
            // Kept only when nothing of the guard is left open on a live connection.
            let discard = lost_class(kind, &e)
                .or_else(|| (conn.tx_status() != TxStatus::Idle).then_some(ErrClass::Internal));
            return Guarded {
                reply: Err(error),
                discard,
            };
        }
    }
    // Fix round 1 (m-1): the user statement runs only inside the guard's read-only
    // transaction. A `before` that did not open one is a gap; nothing more is sent.
    if !plan.before.is_empty() && conn.tx_status() != TxStatus::InTx {
        tracing::warn!("the guard's read-only transaction did not open");
        return Guarded::discard(
            ExecError::of(ExecErrorKind::EnforcementGap(
                GapReason::TransactionEndedByStatement,
            )),
            ErrClass::Internal,
        );
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
        return Guarded::discard(statement_error(e, true), class);
    }

    // 4.
    let tx_after = lite(conn.tx_status());
    let mut facts = PostFacts {
        tx_after_statement: Some(tx_after),
        ..PostFacts::default()
    };
    if let Ok(summary) = &ran {
        facts.command_tag = summary.command_tag.clone();
        facts.rows_affected = summary.rows_affected;
    }

    // 5. Only where the guard's transaction can still answer: after a failed statement
    // PostgreSQL's transaction is aborted, and nothing in it can be committed.
    let mut canary_unread = false;
    if let Some(canary) = plan.canary
        && (ran.is_ok() || tx_after == TxStatusLite::InTx)
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
                    return Guarded::discard(statement_error(&e, true), class);
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
                return Guarded::discard(statement_error(&e, true), class);
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
    let verdict = match &ran {
        Ok(_) => check_post(kind, &facts),
        Err(_) if ddl_committed || family(kind) == Family::Sqlite => PostCheck::Clean,
        Err(_) => check_post(kind, &facts),
    };
    if let PostCheck::Gap(reason) = verdict {
        tracing::warn!(?reason, "a read-only statement escaped the guard");
        return Guarded::discard(
            ExecError::of(ExecErrorKind::EnforcementGap(reason)),
            ErrClass::Internal,
        );
    }
    if canary_unread {
        tracing::warn!("the guard's canary could not be read");
        return Guarded::discard(ExecError::of(ExecErrorKind::Internal), ErrClass::Internal);
    }

    // 8.
    if plan.reset != ResetKind::None {
        if let Err(e) = conn.reset().await {
            tracing::warn!(kind = ?e.kind, "the guard's reset failed");
            let class = lost_class(kind, &e).unwrap_or(ErrClass::Internal);
            return Guarded::discard(ExecError::of(ExecErrorKind::Internal), class);
        }
        for statement in replay {
            if let Err(e) = conn.batch(statement).await {
                tracing::warn!(kind = ?e.kind, "the guard's environment replay failed");
                let class = lost_class(kind, &e).unwrap_or(ErrClass::Internal);
                return Guarded::discard(ExecError::of(ExecErrorKind::Internal), class);
            }
        }
    }
    if conn.tx_status() != TxStatus::Idle {
        tracing::warn!("a guarded request left a transaction open");
        return Guarded::discard(ExecError::of(ExecErrorKind::Internal), ErrClass::Internal);
    }

    Guarded::keep(ran.map_err(|e| statement_error(&e, true)))
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
fn lost_class(kind: DbmsKind, e: &RawError) -> Option<ErrClass> {
    matches!(
        e.kind,
        RawErrorKind::Io(_) | RawErrorKind::Protocol | RawErrorKind::Timeout | RawErrorKind::Tls(_)
    )
    .then(|| classify(kind, Phase::PostConnect, e).class)
}

/// A failed statement as the session reports it. A server refusal is `Db` with the server's
/// message, whatever its code (the read-only codes among them: PostgreSQL `25006`, `42501`;
/// MySQL `1792`, `1142`, `1290`; SQLite `SQLITE_READONLY`, `SQLITE_AUTH`). A statement the
/// read-only connection will not prepare (MySQL's `ER_UNSUPPORTED_PS`) is refused by the
/// protocol layer. `sent`: the statement may have reached the server.
fn statement_error(e: &RawError, sent: bool) -> ExecError {
    let kind = match e.kind {
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
/// server's lexer can go, each ASCII upper-cased (empty when the statement does not start with
/// a word).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Keywords {
    readings: Vec<String>,
    /// False when [`MAX_READINGS`] cut the readings short.
    complete: bool,
}

impl Keywords {
    /// Step 1a: some reading is a transaction keyword, or the readings are incomplete.
    fn controls_transaction(&self) -> bool {
        !self.complete
            || self
                .readings
                .iter()
                .any(|k| TRANSACTION_KEYWORDS.contains(&k.as_str()))
    }

    /// Every reading is one of `words` (and there is one).
    fn all_in(&self, words: &[&str]) -> bool {
        self.complete
            && !self.readings.is_empty()
            && self.readings.iter().all(|k| words.contains(&k.as_str()))
    }
}

/// First keywords of MySQL statements that commit implicitly before they run (T13), so a
/// failed one may leave no transaction without having escaped anything: the server refuses
/// it afterwards under the session's read-only setting. An allowlist: any other keyword after
/// a failed statement is judged (fix round 1, I-2).
const IMPLICIT_COMMIT_KEYWORDS: [&str; 15] = [
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
    };
    read_from(sql.as_bytes(), 0, 0, lexer, &mut keywords);
    keywords
}

/// How many version-gated comments one reading may fork at (the recursion depth; a statement
/// may hold a million of them). Past it the readings are incomplete, which fails closed.
const MAX_FORKS: usize = 16;

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
            out.readings.push(String::new());
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
                // Version-gated: run (its text) or skipped (what follows it).
                read_from(b, text, forks + 1, lexer, out);
                match block_end(b, i, false) {
                    Some(end) => read_from(b, end, forks + 1, lexer, out),
                    None => out.readings.push(String::new()),
                }
                return;
            }
            match block_end(b, i, lexer.postgres) {
                Some(end) => i = end,
                None => {
                    out.readings.push(String::new());
                    return;
                }
            }
        } else if c == b'*' && at(i + 1) == Some(b'/') && lexer.mysql {
            // The end of an executable comment the reading went into.
            i += 2;
        } else {
            out.readings.push(
                b[i..]
                    .iter()
                    .take_while(|c| c.is_ascii_alphanumeric() || **c == b'_')
                    .map(|c| char::from(c.to_ascii_uppercase()))
                    .collect(),
            );
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

/// The index after the block comment that opens at `start`; `None` when it never ends.
fn block_end(b: &[u8], start: usize, nested: bool) -> Option<usize> {
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
            (b'/', b'*') if nested => {
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
/// PostgreSQL) the connection runs as a role other than its session user.
async fn probe(conn: &mut dyn RawConn, kind: DbmsKind) -> Option<Vec<Vec<Vec<Option<String>>>>> {
    if kind == DbmsKind::Postgres {
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
