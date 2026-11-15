//! The SQL context of a console (R-M3-39; Task 33, fix round 1): how its text lexes and
//! splits, read from the session, or **unknown**.
//!
//! - **Reading it.** The probe (`sqlctx::probe_sql`: PostgreSQL `SHOW
//!   standard_conforming_strings`, MySQL/MariaDB `SELECT @@SESSION.sql_mode`) needs the
//!   server's value: an internal `Request::Exec` with `ExecOpts::values` returns it (the
//!   simple protocol).
//! - `## m3a-f1-followups` (follow-up F1-c): on a **read-only** session too. The session runs
//!   this exact internal text outside the read-only guard, as it runs its schema probe, so a
//!   server the guard cannot verify (an unanswered canary) leaves the context unknown instead
//!   of breaking the session at every connect (Task 33 ran it as a guarded typed run, which
//!   did). The guard still runs every user statement.
//! - **Unknown, never guessed.** A probe that answers no value (an error, no row, a value that
//!   is not text, a cancel) leaves the context [`SqlContext::Unknown`]: the dialect's defaults
//!   are *not* taken as known. Run then resolves under every mode combination that changes how
//!   text splits (PostgreSQL `standard_conforming_strings` on and off; MySQL `ANSI_QUOTES` ×
//!   `NO_BACKSLASH_ESCAPES`, MariaDB also its Oracle mode) and sends only what all of them
//!   resolve alike; anything mode-dependent is uncertain and sends nothing. The parser oracle
//!   is never used (`NoOracle`).
//! - A session that answers nothing at all (closed, lost) leaves the context as it was; the
//!   next connection probes again.

use hacivat_driver::ExecOpts;
use hacivat_exec::sqlctx::{dialect_ctx, probe_sql};
use hacivat_session::LiveState;
use hacivat_session::session::{ExecErrorKind, ExecOrigin, Request, SessionHandle};
use hacivat_ui_core::gpui_surface::App;
use sql_dialect::{DialectCtx, DialectId, LexModes};

use crate::console::ConsoleId;
use crate::exec_ui::orchestrator::{live_session, profile_of};
use crate::exec_ui::update_console_exec;

/// What a console knows of its SQL context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SqlContext {
    /// Not read yet: Run waits ("still reading").
    #[default]
    Pending,
    /// Read from the session (or the engine has no session modes).
    Known(DialectCtx),
    /// The probe answered no value: the dialect and version, with modes unknown.
    Unknown(DialectCtx),
}

/// What a probe answered.
enum Answer {
    /// The probe's value (`None`: the engine has no probe).
    Value(Option<String>),
    /// No value: the context is unknown.
    NoValue,
    /// The session answered nothing (closed, lost): nothing changes.
    Gone,
}

/// Every lexing-mode combination that changes how `base`'s dialect splits text (Unknown).
pub(crate) fn variants(base: &DialectCtx) -> Vec<DialectCtx> {
    let with = |modes: LexModes| DialectCtx { modes, ..*base };
    match base.id {
        DialectId::Postgres | DialectId::PgWire(_) => [true, false]
            .into_iter()
            .map(|scs| {
                with(LexModes {
                    standard_conforming_strings: scs,
                    ..base.modes
                })
            })
            .collect(),
        DialectId::MySql | DialectId::MariaDb => {
            let oracle: &[bool] = if base.id == DialectId::MariaDb {
                &[false, true]
            } else {
                &[false]
            };
            let mut out = Vec::new();
            for ansi in [false, true] {
                for nbe in [false, true] {
                    for &ora in oracle {
                        out.push(with(LexModes {
                            ansi_quotes: ansi,
                            no_backslash_escapes: nbe,
                            mariadb_oracle_mode: ora,
                            ..base.modes
                        }));
                    }
                }
            }
            out
        }
        _ => vec![*base],
    }
}

/// Probes console `console`'s SQL context and sets it (`set` gets the outcome on the UI
/// thread). `on_seq` is told the probe's request number (`exec.cancel` leaves it alone).
pub(crate) fn probe(
    console: ConsoleId,
    cx: &mut App,
    set: impl FnOnce(ConsoleId, SqlContext, &mut App) + 'static,
) {
    let Some(profile) = profile_of(cx, console) else {
        return;
    };
    let Some(session) = live_session(cx, console) else {
        return;
    };
    let kind = profile.engine;
    let version = cx
        .try_global::<crate::connections::ConnectionsModel>()
        .and_then(|m| match m.live(profile.id) {
            LiveState::Connected(info) => info.server.version.clone(),
            _ => None,
        });
    let base = dialect_ctx(kind, version.as_ref(), None);
    let Some(sql) = probe_sql(kind) else {
        set(console, SqlContext::Known(base), cx);
        return;
    };
    // ## m3a-f1-followups: one path for read-write and read-only sessions (F1-c).
    let (seq, answer) = probe_by_exec(&session, sql);
    update_console_exec(cx, console, |e| e.probe = Some(seq));
    cx.spawn(async move |cx| {
        let answer = answer.await;
        cx.update(|cx| {
            update_console_exec(cx, console, |e| {
                if e.probe == Some(seq) {
                    e.probe = None;
                }
            });
            let ctx = match answer {
                Answer::Value(row) => {
                    SqlContext::Known(dialect_ctx(kind, version.as_ref(), row.as_deref()))
                }
                Answer::NoValue => SqlContext::Unknown(base),
                Answer::Gone => return,
            };
            set(console, ctx, cx);
        });
    })
    .detach();
}

/// Whether `kind` means the session answered nothing at all.
fn gone(kind: &ExecErrorKind) -> bool {
    matches!(
        kind,
        ExecErrorKind::NotConnected | ExecErrorKind::ConnectionLost { .. }
    )
}

type AnswerFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Answer>>>;

/// An internal statement whose values come back (the simple protocol); on a read-only session
/// the session runs it outside the guard (F1-c).
fn probe_by_exec(session: &SessionHandle, sql: &str) -> (u64, AnswerFuture) {
    let ticket = session.submit(Request::Exec {
        sql: sql.to_string(),
        opts: ExecOpts::values(1),
        origin: ExecOrigin::Internal { idempotent: true },
    });
    let seq = ticket.seq;
    let fut = async move {
        match ticket.reply.await {
            Ok(Ok(summary)) => match summary.rows.first().and_then(|r| r.first()).cloned() {
                Some(Some(value)) => Answer::Value(Some(value)),
                _ => Answer::NoValue,
            },
            Ok(Err(e)) if gone(&e.kind) => Answer::Gone,
            Ok(Err(_)) => Answer::NoValue,
            Err(_) => Answer::Gone,
        }
    };
    (seq, Box::pin(fut))
}
