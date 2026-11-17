//! The output-log model (HV-EXEC-11; Task 23): one ring of lines per console. A line's text is
//! a Fluent message (`fluent_id` and `args`) rendered by the app; server text stays in its own
//! field (`ErrorView::server_text`).
//!
//! `SessionLog` keeps the newest [`SessionLog::CAPACITY`] lines and bumps its `watch` on every
//! change. Lines a burst pushes out of the ring before anyone read them are counted, and
//! [`SessionLog::lines`] then starts with one `Dropped { n }` line (`exec-events-dropped`), so
//! the view says that it is not showing everything. `copy_invariant` is the clipboard form:
//! ISO timestamps, ASCII digits, `ms`, whatever the locale.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::ops::Range;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use hacivat_results::CloseReason;
use hacivat_session::session::{DenyLayer, RunSummary, UnitId};
// ## m3b-t01c-core-seams
use hacivat_driver::NoticeSeverity;
use tokio::sync::watch;

use crate::status::ErrorView;

/// A line's severity (ink glyphs; never a new hue).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    Info,
    Notice,
    Warning,
    Error,
}

impl Severity {
    /// The severity's invariant name in a copied log.
    fn invariant(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Notice => "notice",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

/// What a line says.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LogMsg {
    /// A unit's rows: `more` while its cursor stays open.
    RowsRetrieved {
        rows: u64,
        more: bool,
        exec_ms: u64,
        fetch_ms: u64,
    },
    RowsAffected {
        rows: u64,
        ms: u64,
    },
    Completed {
        ms: u64,
    },
    TxCommitted,
    TxRolledBack,
    /// `rolled_back`: Hacivat's own transaction ended in a rollback (R-M3-23, P26). `idle_ms`:
    /// the idle bound, for `IdleTimeout` (0 when unknown or another reason).
    CursorClosed {
        rows: u64,
        reason: CloseReason,
        rolled_back: bool,
        idle_ms: u64,
    },
    MoreResultSets {
        skipped: u32,
    },
    Summary(RunSummary),
    /// Statement `n` (from 1, in its run) failed; the view holds the server's text verbatim.
    Error {
        n: u32,
        view: ErrorView,
    },
    /// The unit in flight was cancelled after `ms`.
    Cancelled {
        ms: u64,
    },
    ReadOnlyDenied(DenyLayer),
    /// `n` lines a burst pushed out of the ring before anyone read them.
    Dropped {
        n: u64,
    },
    /// Run refused a `COPY … FROM STDIN` / `TO STDOUT` unit: nothing was sent.
    CopyUnsupported,
    /// The run never started (its session answered before running anything: closed, lost);
    /// the view says why. It names no statement.
    NotStarted(ErrorView),
    // ## m3b-t01c-core-seams (Task 16 and the tasks named per variant write these lines; their
    // strings come with them)
    /// A server notice or a MySQL warning (R-M3-77): the text verbatim, never formatted.
    Notice {
        severity: NoticeSeverity,
        code: Option<Arc<str>>,
        text: Arc<str>,
    },
    /// A unit returned more result sets than kept (R-M3-66).
    ResultSetLimit {
        total: u32,
        limit: u32,
    },
    /// A MySQL/MariaDB DDL committed the open transaction (confirmed).
    ImplicitCommit,
    /// The classifier refused the statement on a read-only console (Task 18).
    SafetyDenied {
        class_id: &'static str,
    },
    /// The confirmed statement's transaction was committed (Task 19).
    ConfirmCommitted {
        rows: Option<u64>,
    },
    /// The confirmed statement was rolled back: by the user, by the D-014 deadline, or undone
    /// in the savepoint form (Task 19).
    ConfirmRolledBack {
        by_deadline: bool,
        savepoint: bool,
    },
    /// The confirmed statement failed in the savepoint form: only it was rolled back, the
    /// user's transaction is intact (P-M3b-5).
    ConfirmStatementFailed,
    // ## m3b-t26-prod-unlock (fix round 1, ruling on A-I2/B-I1)
    /// The confirmed statement's rollback ran, but a table does not support transactions: its
    /// changes stay (never logged as undone or discarded).
    ConfirmNotUndone,
    /// The user skipped `n` statements at the confirmation (R-M3-84).
    SkippedByUser {
        n: u32,
    },
    /// Statement `n` was retried (R-M3-61).
    Retried {
        n: u32,
    },
    /// A background result was evicted, keeping `rows` (R-M3-68, Task 8).
    Evicted {
        rows: u64,
    },
    // ## m3b-t16-run-service
    /// `n` server notices of a statement past the cap were counted, never shown (R-M3-77).
    NoticesDropped {
        n: u32,
    },
    /// A unit run as one was stopped before its end: `ran` statements after the closed result
    /// ran and none of their results is shown; `rest_not_run`: the statements after those did
    /// not run (Task 13's `CloseReport`).
    ClosedEarly {
        ran: u32,
        rest_not_run: bool,
    },
    /// A unit run as one failed on PostgreSQL: it ran as one implicit transaction, so the
    /// statements before the failing one were rolled back too (unless the text committed them
    /// itself; ADR 0002 `m3b-t13` M-2).
    RunAsOneRolledBack,
    /// A unit run as one failed or was cancelled on PostgreSQL inside the user's transaction:
    /// nothing was rolled back yet, but the transaction failed and waits for their rollback
    /// (fix round 1, review m9).
    RunAsOneTxFailed,
    // ## m3b-t19-confirm-tx (named exception, fix round 1, I1)
    /// A statement failed or was cancelled and the server rolled back the user's whole
    /// transaction with it (a SQLite interrupt, a MySQL deadlock).
    TxRolledBackByServer,
    /// Fix round 2 (R1-I1): a statement failed or was cancelled and the user's transaction
    /// ended with it, but the statement may have committed it first (an implicit-commit
    /// statement on MySQL/MariaDB, a DDL, a unit run as one, a statement Hacivat can't read):
    /// the user checks what was committed.
    TxEndedByStatement,
    // ## m3b-t22-tx-ui (named exception, T12 carry m9)
    /// The profile's startup script was not run because the console's session is read-only
    /// (a read-only session never runs it, C3). Its SQL is never in the line.
    StartupScriptSkipped,
    // ## m3b-t23-escalation (named exception): the cancel escalation's and the error bar's
    // lines (design/m3b-safe surfaces 6 and 7), written by the app.
    /// Terminate on server ended the session; `statements`: the user's open transaction it
    /// rolled back, when one was open.
    Terminated {
        statements: Option<u32>,
    },
    /// Terminate on server failed (the server's text is in the dialog, verbatim).
    TerminateFailed,
    /// Disconnect dropped the console's connection `name` while it cancelled; `local`: a
    /// SQLite file, whose statement stops with its connection (fix round 1, m4).
    Disconnected {
        name: Arc<str>,
        local: bool,
    },
    /// The user answered Stop to the error bar at statement `n` (R-M3-61).
    RunStopped {
        n: u32,
    },
    /// Roll back and continue: the user's transaction was rolled back, statement `n` runs next.
    RolledBackContinue {
        n: u32,
    },
    // ## m3b-t24-exec-ui (named exception)
    /// A line the run UI writes about a run before anything of it is sent (Task 24): why a unit
    /// was not sent (a gate denial with its reason, a parameter refusal), or what the user
    /// answered at the parameter prompt and the confirmations (`exec-confirm-skipped`,
    /// `exec-confirm-suppressed`, `exec-confirm-cancelled`). `id` is a `hacivat-core.ftl`
    /// message, `args` its arguments (a statement number raw, a name as data), `warn` an ink
    /// line the user must read (a refusal) rather than a note.
    RunUi {
        id: &'static str,
        args: Vec<(&'static str, LogArg)>,
        warn: bool,
    },
}

/// A message argument. Counts are numbers the app formats in the format locale; a statement
/// number is raw text (ASCII digits, never grouped); a duration is a [`duration_message`] the
/// app renders first and passes in as text (`exec-dur-*`, NOTES rule 6).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LogArg {
    U64(u64),
    Str(Arc<str>),
    /// A duration in milliseconds: render [`duration_message`] and pass its text.
    DurationMs(u64),
    // ## m3b-t16-run-service
    /// A configured span in milliseconds (an idle bound), not a measured time: render
    /// [`span_message`] and pass its text.
    SpanMs(u64),
}

impl LogMsg {
    /// The message's Fluent ID in `hacivat-core.ftl`: a literal in every arm, which the
    /// `l10n check` code scan counts as used (P24).
    pub fn fluent_id(&self) -> &'static str {
        // Never format `self`: an error line holds the server's text (fix round 1).
        match self {
            LogMsg::RowsRetrieved { more: true, .. } => "exec-rows-retrieved-more",
            LogMsg::RowsRetrieved { more: false, .. } => "exec-rows-retrieved",
            LogMsg::RowsAffected { .. } => "exec-rows-affected",
            LogMsg::Completed { .. } => "exec-completed",
            LogMsg::TxCommitted => "exec-tx-committed",
            LogMsg::TxRolledBack => "exec-tx-rolled-back",
            LogMsg::CursorClosed {
                reason,
                rolled_back,
                ..
            } => cursor_closed_id(*reason, *rolled_back),
            LogMsg::MoreResultSets { .. } => "exec-more-result-sets",
            LogMsg::Summary(_) => "exec-summary",
            LogMsg::Error { .. } => "exec-error",
            LogMsg::Cancelled { .. } => "exec-cancelled",
            LogMsg::ReadOnlyDenied(layer) => deny_id(*layer),
            LogMsg::Dropped { .. } => "exec-events-dropped",
            LogMsg::CopyUnsupported => "exec-copy-unsupported",
            LogMsg::NotStarted(_) => "exec-not-started",
            // ## m3b-t01c-core-seams
            LogMsg::Notice { severity, .. } => notice_id(*severity),
            LogMsg::ResultSetLimit { .. } => "exec-result-set-limit",
            LogMsg::ImplicitCommit => "exec-implicit-commit",
            LogMsg::SafetyDenied { .. } => "exec-deny-classifier",
            LogMsg::ConfirmCommitted { .. } => "exec-confirm-committed",
            LogMsg::ConfirmRolledBack {
                savepoint: true, ..
            } => "exec-confirm-undone",
            LogMsg::ConfirmRolledBack {
                by_deadline: true, ..
            } => "exec-confirm-rolled-back-deadline",
            LogMsg::ConfirmRolledBack { .. } => "exec-confirm-rolled-back",
            LogMsg::ConfirmStatementFailed => "exec-confirm-statement-failed",
            // ## m3b-t26-prod-unlock (fix round 1)
            LogMsg::ConfirmNotUndone => "exec-confirm-not-undone",
            LogMsg::SkippedByUser { .. } => "exec-skipped-by-user",
            LogMsg::Retried { .. } => "exec-retried",
            LogMsg::Evicted { .. } => "cursor-closed-evicted",
            // ## m3b-t16-run-service
            LogMsg::NoticesDropped { .. } => "exec-notices-dropped",
            LogMsg::ClosedEarly {
                rest_not_run: false,
                ..
            } => "exec-run-as-one-closed-early",
            LogMsg::ClosedEarly {
                rest_not_run: true, ..
            } => "exec-run-as-one-closed-early-rest",
            LogMsg::RunAsOneRolledBack => "exec-run-as-one-rolled-back",
            LogMsg::RunAsOneTxFailed => "exec-run-as-one-tx-failed",
            // ## m3b-t19-confirm-tx
            LogMsg::TxRolledBackByServer => "exec-tx-rolled-back-by-server",
            LogMsg::TxEndedByStatement => "exec-tx-ended-by-statement",
            // ## m3b-t22-tx-ui (named exception)
            LogMsg::StartupScriptSkipped => "exec-startup-script-skipped-read-only",
            // ## m3b-t23-escalation (named exception)
            LogMsg::Terminated {
                statements: Some(_),
            } => "exec-terminated",
            LogMsg::Terminated { statements: None } => "exec-terminated-no-tx",
            LogMsg::TerminateFailed => "exec-terminate-failed",
            LogMsg::Disconnected { local: false, .. } => "exec-disconnected",
            LogMsg::Disconnected { local: true, .. } => "exec-disconnected-local",
            LogMsg::RunStopped { .. } => "exec-run-stopped",
            LogMsg::RolledBackContinue { .. } => "exec-run-rolled-back-continue",
            // ## m3b-t24-exec-ui (named exception): the run UI names its own literal IDs.
            LogMsg::RunUi { id, .. } => id,
        }
    }

    /// The message's arguments, named as in `hacivat-core.ftl`.
    pub fn args(&self) -> Vec<(&'static str, LogArg)> {
        match self {
            LogMsg::RowsRetrieved {
                rows,
                exec_ms,
                fetch_ms,
                ..
            } => vec![
                ("rows", LogArg::U64(*rows)),
                (
                    "duration",
                    LogArg::DurationMs(exec_ms.saturating_add(*fetch_ms)),
                ),
            ],
            LogMsg::RowsAffected { rows, ms } => vec![
                ("rows", LogArg::U64(*rows)),
                ("duration", LogArg::DurationMs(*ms)),
            ],
            LogMsg::Completed { ms } | LogMsg::Cancelled { ms } => {
                vec![("duration", LogArg::DurationMs(*ms))]
            }
            LogMsg::CursorClosed {
                rows,
                reason,
                idle_ms,
                ..
            } => {
                let mut args = vec![("rows", LogArg::U64(*rows))];
                // ## m3b-t16-run-service: the idle bound is a setting ("10 min"), not a run's
                // time: it keeps whole minutes and seconds.
                if *reason == CloseReason::IdleTimeout {
                    args.push(("duration", LogArg::SpanMs(*idle_ms)));
                }
                args
            }
            LogMsg::MoreResultSets { skipped } => {
                vec![("skipped", LogArg::U64(u64::from(*skipped)))]
            }
            LogMsg::Summary(s) => vec![
                ("executed", LogArg::U64(u64::from(s.executed))),
                ("total", LogArg::U64(u64::from(s.total))),
                ("failed", LogArg::U64(u64::from(s.failed))),
                ("duration", LogArg::DurationMs(s.duration_ms)),
            ],
            // A statement number stays raw (ASCII digits, never grouped).
            LogMsg::Error { n, .. } => vec![("n", LogArg::Str(n.to_string().into()))],
            LogMsg::Dropped { n } => vec![("n", LogArg::U64(*n))],
            LogMsg::TxCommitted
            | LogMsg::TxRolledBack
            | LogMsg::ReadOnlyDenied(_)
            | LogMsg::CopyUnsupported
            | LogMsg::NotStarted(_) => Vec::new(),
            // ## m3b-t16-run-service: a notice's text is server text and its code a chip, as an
            // error's (the view shows both in their own fields, verbatim; `copy_invariant`
            // writes them after the message): the sentence takes no argument.
            LogMsg::Notice { .. } => Vec::new(),
            LogMsg::ResultSetLimit { total, limit } => vec![
                ("total", LogArg::U64(u64::from(*total))),
                ("limit", LogArg::U64(u64::from(*limit))),
            ],
            LogMsg::ConfirmCommitted { rows } => {
                rows.iter().map(|r| ("rows", LogArg::U64(*r))).collect()
            }
            LogMsg::SkippedByUser { n } => vec![("n", LogArg::U64(u64::from(*n)))],
            // A statement number stays raw (ASCII digits, never grouped).
            LogMsg::Retried { n } => vec![("n", LogArg::Str(n.to_string().into()))],
            LogMsg::Evicted { rows } => vec![("rows", LogArg::U64(*rows))],
            LogMsg::ImplicitCommit
            | LogMsg::SafetyDenied { .. }
            | LogMsg::ConfirmRolledBack { .. }
            | LogMsg::ConfirmStatementFailed
            | LogMsg::ConfirmNotUndone => Vec::new(),
            // ## m3b-t16-run-service
            LogMsg::NoticesDropped { n } => vec![("n", LogArg::U64(u64::from(*n)))],
            LogMsg::ClosedEarly { ran, .. } => vec![("n", LogArg::U64(u64::from(*ran)))],
            LogMsg::RunAsOneRolledBack | LogMsg::RunAsOneTxFailed => Vec::new(),
            // ## m3b-t19-confirm-tx
            LogMsg::TxRolledBackByServer | LogMsg::TxEndedByStatement => Vec::new(),
            // ## m3b-t22-tx-ui (named exception)
            LogMsg::StartupScriptSkipped => Vec::new(),
            // ## m3b-t23-escalation (named exception): a statement number stays raw.
            LogMsg::Terminated { statements } => statements
                .iter()
                .map(|n| ("n", LogArg::U64(u64::from(*n))))
                .collect(),
            LogMsg::TerminateFailed => Vec::new(),
            LogMsg::Disconnected { name, .. } => vec![("name", LogArg::Str(name.clone()))],
            LogMsg::RunStopped { n } | LogMsg::RolledBackContinue { n } => {
                vec![("i", LogArg::Str(n.to_string().into()))]
            }
            // ## m3b-t24-exec-ui (named exception)
            LogMsg::RunUi { args, .. } => args.clone(),
        }
    }

    /// The severity a line of this message has.
    pub fn severity(&self) -> Severity {
        match self {
            LogMsg::Error { .. }
            | LogMsg::ReadOnlyDenied(_)
            | LogMsg::CopyUnsupported
            | LogMsg::NotStarted(_) => Severity::Error,
            LogMsg::Cancelled { .. } | LogMsg::Dropped { .. } => Severity::Warning,
            LogMsg::CursorClosed { .. } | LogMsg::MoreResultSets { .. } => Severity::Notice,
            // ## m3b-t01c-core-seams
            LogMsg::SafetyDenied { .. }
            | LogMsg::ConfirmStatementFailed
            | LogMsg::ConfirmNotUndone => Severity::Error,
            LogMsg::Notice { severity, .. } => match severity {
                NoticeSeverity::Warning => Severity::Warning,
                NoticeSeverity::Debug
                | NoticeSeverity::Log
                | NoticeSeverity::Info
                | NoticeSeverity::Notice => Severity::Notice,
            },
            LogMsg::ResultSetLimit { .. }
            | LogMsg::ImplicitCommit
            | LogMsg::ConfirmRolledBack { .. }
            | LogMsg::SkippedByUser { .. }
            | LogMsg::Evicted { .. } => Severity::Notice,
            LogMsg::ConfirmCommitted { .. } | LogMsg::Retried { .. } => Severity::Info,
            // ## m3b-t16-run-service
            LogMsg::NoticesDropped { .. } | LogMsg::ClosedEarly { .. } => Severity::Notice,
            LogMsg::RunAsOneRolledBack | LogMsg::RunAsOneTxFailed => Severity::Warning,
            // ## m3b-t19-confirm-tx
            LogMsg::TxRolledBackByServer | LogMsg::TxEndedByStatement => Severity::Warning,
            // ## m3b-t22-tx-ui (named exception)
            LogMsg::StartupScriptSkipped => Severity::Notice,
            // ## m3b-t23-escalation (named exception)
            LogMsg::TerminateFailed => Severity::Error,
            LogMsg::Terminated { .. } | LogMsg::Disconnected { .. } => Severity::Warning,
            LogMsg::RunStopped { .. } | LogMsg::RolledBackContinue { .. } => Severity::Notice,
            // ## m3b-t24-exec-ui (named exception): a refusal is an error line (a click selects
            // its statement), an answer a notice.
            LogMsg::RunUi { warn: true, .. } => Severity::Error,
            LogMsg::RunUi { warn: false, .. } => Severity::Notice,
            LogMsg::RowsRetrieved { .. }
            | LogMsg::RowsAffected { .. }
            | LogMsg::Completed { .. }
            | LogMsg::TxCommitted
            | LogMsg::TxRolledBack
            | LogMsg::Summary(_) => Severity::Info,
        }
    }
}

/// One `cursor-closed-*` message per [`CloseReason`], each with its "Hacivat's transaction was
/// rolled back" form (R-M3-23, P26).
fn cursor_closed_id(reason: CloseReason, rolled_back: bool) -> &'static str {
    match (reason, rolled_back) {
        (CloseReason::SessionNeeded, false) => "cursor-closed-session-needed",
        (CloseReason::SessionNeeded, true) => "cursor-closed-session-needed-rolled-back",
        (CloseReason::IdleTimeout, false) => "cursor-closed-idle-timeout",
        (CloseReason::IdleTimeout, true) => "cursor-closed-idle-timeout-rolled-back",
        (CloseReason::TabClosed, false) => "cursor-closed-tab-closed",
        (CloseReason::TabClosed, true) => "cursor-closed-tab-closed-rolled-back",
        (CloseReason::TxEnded, false) => "cursor-closed-tx-ended",
        (CloseReason::TxEnded, true) => "cursor-closed-tx-ended-rolled-back",
        (CloseReason::UserClosed, false) => "cursor-closed-user-closed",
        (CloseReason::UserClosed, true) => "cursor-closed-user-closed-rolled-back",
        (CloseReason::MemoryLimit, false) => "cursor-closed-memory-limit",
        (CloseReason::MemoryLimit, true) => "cursor-closed-memory-limit-rolled-back",
        (CloseReason::ConnectionLost, false) => "cursor-closed-connection-lost",
        (CloseReason::ConnectionLost, true) => "cursor-closed-connection-lost-rolled-back",
        (CloseReason::ServerTerminated, false) => "cursor-closed-server-terminated",
        (CloseReason::ServerTerminated, true) => "cursor-closed-server-terminated-rolled-back",
        (CloseReason::Cancelled, false) => "cursor-closed-cancelled",
        (CloseReason::Cancelled, true) => "cursor-closed-cancelled-rolled-back",
        (CloseReason::EnforcementGap, false) => "cursor-closed-enforcement-gap",
        (CloseReason::EnforcementGap, true) => "cursor-closed-enforcement-gap-rolled-back",
        // ## m3b-t01c-core-seams: an evicted result held no cursor of its own to roll back.
        (CloseReason::Evicted, _) => "cursor-closed-evicted",
    }
}

// ## m3b-t01c-core-seams
/// One `exec-notice-*` message per [`NoticeSeverity`].
fn notice_id(severity: NoticeSeverity) -> &'static str {
    match severity {
        NoticeSeverity::Debug => "exec-notice-debug",
        NoticeSeverity::Log => "exec-notice-log",
        NoticeSeverity::Info => "exec-notice-info",
        NoticeSeverity::Notice => "exec-notice-notice",
        NoticeSeverity::Warning => "exec-notice-warning",
    }
}

/// One `exec-deny-read-only-*` sentence per [`DenyLayer`].
pub(crate) fn deny_id(layer: DenyLayer) -> &'static str {
    match layer {
        DenyLayer::Input => "exec-deny-read-only-input",
        DenyLayer::Protocol => "exec-deny-read-only-protocol",
        DenyLayer::Transaction => "exec-deny-read-only-transaction",
        DenyLayer::Engine => "exec-deny-read-only-engine",
        DenyLayer::Function => "exec-deny-read-only-function",
        // ## m3b-t01c-core-seams (Task 18)
        // ## m3b-t18-safe06 (named exception, T1c review M2): one Fluent ID for one classifier
        // denial. Step 0b reports `ExecErrorKind::SafetyDenied`; this layer is never produced,
        // and should it be, it reads as the same sentence.
        DenyLayer::Classifier => "exec-deny-classifier",
    }
}

/// A Fluent message with numeric arguments, for durations and sizes (`exec-dur-*`,
/// `exec-size-*`): the app formats each value in the format locale.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UnitMsg {
    pub id: &'static str,
    /// The arguments: whole numbers, except a size's `n`, which is in tenths (12.5 MB is 125),
    /// and a duration's `seconds`, in hundredths (2.45 s is 245): the app formats each with
    /// [`UnitMsg::fraction_digits`] digits ([`UnitMsg::decimal`]).
    pub args: Vec<(&'static str, u64)>,
}

// ## m3b-t16-run-service
impl UnitMsg {
    /// How many of argument `name`'s digits are after the decimal point: 1 for a size's `n`,
    /// 2 for a duration's `seconds`, else 0 (a whole number).
    pub fn fraction_digits(&self, name: &str) -> u32 {
        match (self.id, name) {
            ("exec-size-kb" | "exec-size-mb" | "exec-size-gb", "n") => 1,
            ("exec-dur-seconds", "seconds") => 2,
            _ => 0,
        }
    }

    /// Argument `name` as exact decimal text with its fraction digits (`245` with 2 is
    /// `"2.45"`), or `None` for a whole-number argument (or none of that name).
    pub fn decimal(&self, name: &str) -> Option<String> {
        let digits = self.fraction_digits(name);
        if digits == 0 {
            return None;
        }
        let (_, v) = self.args.iter().find(|(n, _)| *n == name)?;
        let scale = 10u64.pow(digits);
        let width = usize::try_from(digits).unwrap_or(2);
        Some(format!("{}.{:0width$}", v / scale, v % scale))
    }
}

/// The `exec-dur-*` message of `ms` milliseconds, by the one duration rule the output log, the
/// gutter marks and the status bar share (PO, ledger 535): under a second, whole milliseconds
/// (`exec-dur-ms`: `0 ms`, `340 ms`); from a second on, seconds with two decimals, cut, not
/// rounded (`exec-dur-seconds`: `2.34 s`, `185.40 s`), as the status bar's
/// `hv-status-duration-*` write it.
pub fn duration_message(ms: u64) -> UnitMsg {
    let (id, args) = if ms < 1_000 {
        ("exec-dur-ms", vec![("ms", ms)])
    } else {
        ("exec-dur-seconds", vec![("seconds", ms / 10)])
    };
    UnitMsg { id, args }
}

// ## m3b-t16-run-service
/// The `exec-dur-*` message of a configured span of `ms` milliseconds (an idle bound: "20 s",
/// "10 min", "1 min 30 s"), which reads as the setting reads: whole seconds under a minute,
/// whole minutes and seconds from a minute on (fractions dropped); under a second as
/// [`duration_message`]. One of the two named exceptions to the one duration rule (with the
/// running clock; ADR 0006 `m3b-t16`).
pub fn span_message(ms: u64) -> UnitMsg {
    if ms < 1_000 {
        return duration_message(ms);
    }
    if ms < 60_000 {
        return UnitMsg {
            id: "exec-dur-s",
            args: vec![("s", ms / 1_000)],
        };
    }
    let (min, s) = (ms / 60_000, (ms / 1_000) % 60);
    let (id, args) = if s == 0 {
        ("exec-dur-min", vec![("min", min)])
    } else {
        ("exec-dur-min-s", vec![("min", min), ("s", s)])
    };
    UnitMsg { id, args }
}

/// The `exec-size-*` message of `bytes` (1024-based, one fraction digit, rounded half up): KB,
/// MB or GB, the smallest unit whose rounded value stays below 1,024 (never "1024.0 KB").
pub fn size_message(bytes: u64) -> UnitMsg {
    const KIB: u128 = 1 << 10;
    let b = u128::from(bytes);
    let tenths_in = |unit: u128| (b * 10 + unit / 2) / unit;
    let (id, unit) = [("exec-size-kb", KIB), ("exec-size-mb", KIB << 10)]
        .into_iter()
        .find(|(_, unit)| tenths_in(*unit) < 10 * 1_024)
        .unwrap_or(("exec-size-gb", KIB << 20));
    let tenths = tenths_in(unit);
    UnitMsg {
        id,
        args: vec![("n", u64::try_from(tenths).unwrap_or(u64::MAX))],
    }
}

/// One line of a console's output log.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LogLine {
    pub seq: u64,
    pub at: SystemTime,
    pub severity: Severity,
    pub msg: LogMsg,
    /// The statement's range in the document (a click selects it).
    pub source: Option<Range<usize>>,
    pub unit: Option<UnitId>,
    // ## m3b-t16-run-service (ledger 340: the run service kept no command tag)
    /// The server's command tag of the statement the line reports (PostgreSQL's
    /// `INSERT 0 3`), verbatim, when it sent one.
    pub tag: Option<Arc<str>>,
}

#[derive(Debug, Default)]
struct Ring {
    lines: VecDeque<LogLine>,
    /// The next `seq` [`SessionLog::append`] gives.
    next_seq: u64,
    /// The newest `seq` a reader has been given (`None`: nothing read yet).
    read_upto: Option<u64>,
    /// Lines that left the ring before a reader saw them, since the last clear.
    unseen_dropped: u64,
    // ## m3b-t01c-core-seams (review M3)
    /// The newest `seq` the ring pushed out for room (never one a clear removed).
    dropped_upto: Option<u64>,
}

/// A console's output log: a ring of the newest lines.
#[derive(Debug)]
pub struct SessionLog {
    ring: Mutex<Ring>,
    changes: watch::Sender<u64>,
}

impl SessionLog {
    /// The most lines kept; a full ring drops the oldest.
    pub const CAPACITY: usize = 10_000;

    pub fn new() -> Arc<Self> {
        Arc::new(SessionLog {
            ring: Mutex::new(Ring::default()),
            changes: watch::Sender::new(0),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Ring> {
        self.ring.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Appends `line` (dropping the oldest when the ring is full) and bumps `changes`. A dropped
    /// line no reader has seen is counted for the `Dropped` line.
    pub fn push(&self, line: LogLine) {
        self.push_with(|_| line);
    }

    /// Appends a line saying `msg` now, with the next sequence number and `msg`'s severity
    /// (numbered under the ring's lock: concurrent appends never share a number).
    pub fn append(&self, msg: LogMsg, source: Option<Range<usize>>, unit: Option<UnitId>) {
        self.append_tagged(msg, None, source, unit);
    }

    // ## m3b-t16-run-service
    /// [`append`](Self::append) with the statement's command tag (ledger 340).
    pub fn append_tagged(
        &self,
        msg: LogMsg,
        tag: Option<Arc<str>>,
        source: Option<Range<usize>>,
        unit: Option<UnitId>,
    ) {
        self.push_with(|seq| LogLine {
            seq,
            at: SystemTime::now(),
            severity: msg.severity(),
            msg,
            source,
            unit,
            tag,
        });
    }

    /// Pushes the line `make` builds from the next sequence number, under one lock.
    fn push_with(&self, make: impl FnOnce(u64) -> LogLine) {
        {
            let mut ring = self.lock();
            let line = make(ring.next_seq);
            ring.next_seq = ring.next_seq.max(line.seq.saturating_add(1));
            if ring.lines.len() == Self::CAPACITY
                && let Some(old) = ring.lines.pop_front()
            {
                // ## m3b-t01c-core-seams (review M3): `lines_since`'s gap.
                ring.dropped_upto = Some(old.seq);
                if ring.read_upto.is_none_or(|seen| old.seq > seen) {
                    ring.unseen_dropped += 1;
                }
            }
            ring.lines.push_back(line);
        }
        self.changes.send_modify(|n| *n += 1);
    }

    /// The lines, oldest first; marks them read. When a burst pushed lines out before anyone
    /// read them, the first line is `Dropped { n }` (with the `seq` and time of the oldest kept
    /// line), counting every such line since the last clear.
    pub fn lines(&self) -> Arc<[LogLine]> {
        let mut ring = self.lock();
        let mut out = Vec::with_capacity(ring.lines.len() + 1);
        if ring.unseen_dropped > 0
            && let Some(first) = ring.lines.front()
        {
            out.push(LogLine {
                seq: first.seq,
                at: first.at,
                severity: Severity::Warning,
                msg: LogMsg::Dropped {
                    n: ring.unseen_dropped,
                },
                source: None,
                unit: None,
                tag: None,
            });
        }
        out.extend(ring.lines.iter().cloned());
        if let Some(last) = ring.lines.back() {
            ring.read_upto = Some(last.seq);
        }
        out.into()
    }

    // ## m3b-t01c-core-seams
    /// The lines numbered `seq` or later, oldest first, and whether the ring pushed some of
    /// them out for room before this read (the gap a reader that remembers its next `seq` must
    /// show; Task 25). Lines a [`clear`](Self::clear) removed are no gap. Marks them read, as
    /// [`lines`](Self::lines) does; never adds a `Dropped` line.
    pub fn lines_since(&self, seq: u64) -> (Arc<[LogLine]>, bool) {
        let mut ring = self.lock();
        let out: Vec<LogLine> = ring
            .lines
            .iter()
            .filter(|l| l.seq >= seq)
            .cloned()
            .collect();
        let gap = ring.dropped_upto.is_some_and(|dropped| dropped >= seq);
        if let Some(last) = ring.lines.back() {
            ring.read_upto = Some(last.seq);
        }
        (out.into(), gap)
    }

    /// Bumped on every push and clear.
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// Empties the log (the toolbar's Clear) and bumps `changes`.
    pub fn clear(&self) {
        {
            let mut ring = self.lock();
            ring.lines.clear();
            ring.unseen_dropped = 0;
        }
        self.changes.send_modify(|n| *n += 1);
    }
}

/// The log as locale-free text for the clipboard: one line per log line, tab-separated: the
/// ISO 8601 UTC timestamp with milliseconds, the severity, the message ID, then each argument
/// as `name=value` (ASCII digits; a duration as `<n>ms`). An error line adds its code chip and
/// the server's text verbatim; a line break inside server text is kept as `\n` so one log line
/// stays one copied line. Nothing is localized, case-mapped or grouped.
pub fn copy_invariant(lines: &[LogLine]) -> String {
    let mut out = String::new();
    for line in lines {
        let _ = write!(
            out,
            "{}\t{}\t{}",
            iso_utc(line.at),
            line.severity.invariant(),
            line.msg.fluent_id()
        );
        for (name, arg) in line.msg.args() {
            match arg {
                LogArg::U64(n) => {
                    let _ = write!(out, "\t{name}={n}");
                }
                LogArg::DurationMs(ms) | LogArg::SpanMs(ms) => {
                    let _ = write!(out, "\t{name}={ms}ms");
                }
                LogArg::Str(s) => {
                    let _ = write!(out, "\t{name}={}", escaped(&s));
                }
            }
        }
        // ## m3b-t16-run-service: the command tag, verbatim.
        if let Some(tag) = &line.tag {
            let _ = write!(out, "\ttag={}", escaped(tag));
        }
        // ## m3b-t16-run-service: a notice's code and the server's text, verbatim.
        if let LogMsg::Notice { code, text, .. } = &line.msg {
            if let Some(code) = code {
                let _ = write!(out, "\tcode={}", escaped(code));
            }
            let _ = write!(out, "\t{}", escaped(text));
        }
        if let LogMsg::Error { view, .. } | LogMsg::NotStarted(view) = &line.msg {
            let _ = write!(out, "\t{}", view.title_id);
            if let Some(code) = &view.code {
                let _ = write!(out, "\tcode={}", escaped(code));
            }
            if let Some(text) = &view.server_text {
                let _ = write!(out, "\t{}", escaped(text));
            }
        }
        out.push('\n');
    }
    out
}

/// `s` with `\`, tab, CR and LF written as escapes, so a value never splits a copied line.
fn escaped(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

/// `at` as `YYYY-MM-DDTHH:MM:SS.mmmZ` (UTC; a time before 1970 prints as the epoch).
fn iso_utc(at: SystemTime) -> String {
    let since = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = since.as_secs();
    let (days, day_secs) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil_from_days(i64::try_from(days).unwrap_or(i64::MAX));
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        day_secs / 3_600,
        (day_secs / 60) % 60,
        day_secs % 60,
        since.subsec_millis()
    )
}

/// The proleptic Gregorian date of `z` days after 1970-01-01 (Howard Hinnant's
/// `civil_from_days`).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (
        y,
        u32::try_from(m).unwrap_or(1),
        u32::try_from(d).unwrap_or(1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates_round_the_calendar() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(59), (1970, 3, 1));
        // 2000-02-29 (a leap day of a century divisible by 400) and the day after.
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(11_017), (2000, 3, 1));
        assert_eq!(civil_from_days(20_772), (2026, 11, 15));
    }
}
