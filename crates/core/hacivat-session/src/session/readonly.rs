//! Guarded read-only execution and the role probe (HV-SESS-04, SQL-SAFE-05, MCP-09 step 1,
//! B21; Task 18): every statement of a read-only session runs inside the guard plan of
//! `hacivat_guard::plan_for`, with the post checks, and the connection is labelled by the role
//! probe.
//!
//! [`run_guarded`], for PostgreSQL, MySQL and MariaDB:
//! - 1: `check_input`, else `ReadOnlyDenied(Input)`;
//! - 1a: a transaction keyword first (`guard_keyword`) is `ReadOnlyDenied(Transaction)` (Q35);
//! - 2: the plan's `before` statements (`batch`): if one fails, nothing else runs;
//! - 3: the user statement, always through the extended protocol (one statement, prepared);
//! - 4: the transaction status after it;
//! - 5: PostgreSQL's canary (`execute(Simple)`), while the guard's transaction can answer;
//! - 6: the plan's `after` statements (`ROLLBACK`, `execute(Simple)`), with their notices;
//! - 7: `check_post`: a gap is `EnforcementGap(reason)`, and the connection is discarded
//!   unreset;
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
    PostCheck, PostFacts, ResetKind, RoleFindings, RoleWarning, TxStatusLite, access_label,
    check_input, check_post, interpret_probe, plan_for, remediation_for, role_probe_sql,
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
    if TRANSACTION_KEYWORDS.contains(&guard_keyword(kind, sql).as_str()) {
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

    // 7. After a failed statement only PostgreSQL's transaction state says something: an
    // error inside its transaction always leaves it failed, so no transaction means the
    // statement ended it. MySQL may commit implicitly before a failing DDL statement, which
    // escapes nothing (the statement failed, and the reset follows).
    let verdict = match &ran {
        Ok(_) => check_post(kind, &facts),
        Err(_) if family(kind) == Family::Postgres => check_post(kind, &facts),
        Err(_) => PostCheck::Clean,
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

/// The first keyword of `sql` as `kind`'s server reads it, ASCII upper-cased, for step 1a.
///
/// - Skipped: ASCII whitespace (vertical tab included), `;` (an empty statement), `--` line
///   comments (on MySQL only before whitespace or a control character, as MySQL reads them),
///   `#` line comments on MySQL, and block comments. A line comment ends at the first line
///   feed or carriage return, whichever comes first.
/// - PostgreSQL block comments nest; other engines' do not.
/// - MySQL and MariaDB run executable comments (`/*!…*/`, `/*!NNNNN …*/`, MariaDB
///   `/*M!NNNNNN …*/`; T13 carry): their content is statement text, so the keyword is read
///   inside them. A version-gated one counts even where the server's version would skip it,
///   which can only refuse more.
/// - Empty when `sql` does not start with a word (an unterminated comment included).
pub(crate) fn guard_keyword(kind: DbmsKind, sql: &str) -> String {
    let mysql = family(kind) == Family::MySql;
    let nested = family(kind) == Family::Postgres;
    let b = sql.as_bytes();
    let at = |i: usize| b.get(i).copied();
    let mut i = 0;
    loop {
        let Some(c) = at(i) else {
            return String::new();
        };
        if c.is_ascii_whitespace() || c == b'\x0b' || c == b';' {
            i += 1;
        } else if c == b'-'
            && at(i + 1) == Some(b'-')
            && (!mysql || at(i + 2).is_none_or(|n| n.is_ascii_whitespace() || n.is_ascii_control()))
        {
            i = line_end(b, i + 2);
        } else if c == b'#' && mysql {
            i = line_end(b, i + 1);
        } else if c == b'/' && at(i + 1) == Some(b'*') {
            let executable = match (at(i + 2), at(i + 3)) {
                (Some(b'!'), _) => Some(i + 3),
                (Some(b'M'), Some(b'!')) => Some(i + 4),
                _ => None,
            };
            match executable {
                Some(start) if mysql => {
                    // Into the comment: its version, then its text.
                    i = start;
                    while at(i).is_some_and(|d| d.is_ascii_digit()) {
                        i += 1;
                    }
                }
                _ => match block_end(b, i, nested) {
                    Some(end) => i = end,
                    None => return String::new(),
                },
            }
        } else if c == b'*' && at(i + 1) == Some(b'/') && mysql {
            // The end of an executable comment the keyword search went into.
            i += 2;
        } else {
            break;
        }
    }
    b[i..]
        .iter()
        .take_while(|c| c.is_ascii_alphanumeric() || **c == b'_')
        .map(|c| char::from(c.to_ascii_uppercase()))
        .collect()
}

/// The index after the line comment that starts before `from`.
fn line_end(b: &[u8], from: usize) -> usize {
    b[from..]
        .iter()
        .position(|c| *c == b'\n' || *c == b'\r')
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

/// The probe's row sets; `None` when a statement failed or answered too many rows.
async fn probe(conn: &mut dyn RawConn, kind: DbmsKind) -> Option<Vec<Vec<Vec<Option<String>>>>> {
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
