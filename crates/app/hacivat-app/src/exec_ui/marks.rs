//! The end-of-line marks of a run (Task 33): `RunStatus` to `ExecMarkState` with Fluent labels.
//!
//! - **States.** Queued → `Queued`; Running and Fetching → `Running` (the start read on the
//!   GPUI executor clock, so the 50 ms grace and the once-a-second clock follow the UI's
//!   time); a cancel asked for the run turns the unit in flight into `Cancelling` (no run event
//!   says so, Task 23); Suspended and Succeeded → `Succeeded` with the rows ("10,000+ rows"
//!   while the cursor is open); Failed → `Failed` with the SQLSTATE chip, or "not sent:
//!   read-only" for a read-only denial; Cancelled; Skipped.
//! - **Words.** The end-of-line status and the glyph's tooltip are Fluent (`hv-editor-mark-*`,
//!   block `## m3a-t33-exec-ui`); durations are `exec-dur-*` messages rendered first and passed
//!   in as text (no number is joined to a unit in code). A running clock is read on every
//!   repaint from the executor clock. A failed unit's tooltip is "Statement N failed": the
//!   server's own text is never inside a sentence; it is in the Output tab.
//! - **Announcements.** Each change of a unit's state is announced politely
//!   (`a11y::exec_announcement`; queued and cancelling say nothing).
//! - **Results.** The run's result tabs go into the console's `ConsoleResults`; each succeeded
//!   unit's timings (`exec_ms`, and `fetch_ms`, the fetching part only) go with its tab.

use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_driver::TxStatus;
use hacivat_exec::{ResultTab, RunStatus, TabKey, UnitStatus, duration_message};
use hacivat_l10n::ArgValue;
use hacivat_session::session::{RowsOutcome as SessionRows, RunId, SessionState, TxOwner};
use hacivat_ui::text::TextFn;
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{App, SharedString, Task};
use hacivat_ui_editor::{ExecMarkState, MarkKey, RowsOutcome, a11y};
use tokio::sync::watch;

use crate::console::{ConsoleId, Consoles};
use crate::exec_ui::{console_exec, orchestrator, update_console_exec};
use crate::l10n_global::L10nGlobal;

/// The mark of unit `ordinal` of run `run` (unique in the process).
pub(crate) fn mark_key(run: RunId, ordinal: usize) -> MarkKey {
    MarkKey((run.0 << 20) | (ordinal as u64 & 0xF_FFFF))
}

/// Renders the message `id` with `args`, for the UI.
pub(crate) fn message(cx: &App, id: &str, args: Vec<(&'static str, ArgValue)>) -> SharedString {
    match cx.try_global::<L10nGlobal>() {
        Some(g) => g.l10n.t_dynamic(id, args).into(),
        None => SharedString::from(id.to_string()),
    }
}

/// `ms` as an `exec-dur-*` message.
pub(crate) fn duration_text(cx: &App, ms: u64) -> String {
    // ## m3a-f2-visual: one formatter for this and the running clock.
    unit_text(cx, duration_message(ms))
}

// ## m3a-f2-visual (named exception in Task 33's file)
/// The running clock's duration: whole seconds, as the toolbar's clock ("running · 3 s",
/// NOTES surface 5). It repaints once a second, so a millisecond part would only show which
/// frame painted it; under a second it reads "0 s", past a minute `exec-dur-min*`.
pub(crate) fn clock_message(ms: u64) -> hacivat_exec::UnitMsg {
    let s = ms / 1_000;
    // ## m3b-t16-run-service (named exception): the clock keeps its whole seconds and minutes;
    // `duration_message` now gives a finished duration's seconds with two decimals.
    let (min, rest) = (s / 60, s % 60);
    let (id, args) = if s < 60 {
        ("exec-dur-s", vec![("s", s)])
    } else if rest == 0 {
        ("exec-dur-min", vec![("min", min)])
    } else {
        ("exec-dur-min-s", vec![("min", min), ("s", rest)])
    };
    hacivat_exec::UnitMsg { id, args }
}

/// [`clock_message`] in the UI language.
pub(crate) fn clock_text(cx: &App, ms: u64) -> String {
    unit_text(cx, clock_message(ms))
}

/// A `UnitMsg` in the UI language, its numbers in the format locale.
fn unit_text(cx: &App, m: hacivat_exec::UnitMsg) -> String {
    // ## m3b-t16-run-service (named exception): a seconds value has two decimals.
    let args = m
        .args
        .iter()
        .map(|(k, v)| match m.decimal(k) {
            Some(d) => (*k, ArgValue::Decimal(hacivat_l10n::ExactDecimal(d))),
            None => (*k, ArgValue::Count(*v)),
        })
        .collect();
    message(cx, m.id, args).to_string()
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// One unit's mark as the UI last set it.
#[derive(Default, Clone)]
struct Seen {
    /// The `UnitStatus` name and its rows, so a change sets the mark once.
    shown: Option<(&'static str, u64)>,
    /// When the UI first saw the unit running (executor clock).
    started: Option<Instant>,
}

/// What the marks of a run are following.
struct Tracker {
    console: ConsoleId,
    keys: Vec<MarkKey>,
    seen: Vec<Seen>,
    ended: bool,
}

/// Follows a run's status: marks, announcements, timings, and the run's end.
pub(crate) fn follow_status(
    console: ConsoleId,
    mut status: watch::Receiver<RunStatus>,
    keys: Vec<MarkKey>,
    cx: &mut App,
) -> Task<()> {
    let mut t = Tracker {
        console,
        seen: vec![Seen::default(); keys.len()],
        keys,
        ended: false,
    };
    cx.spawn(async move |cx| {
        let mut run = None;
        loop {
            let st = status.borrow_and_update().clone();
            run = st.run.or(run);
            cx.update(|cx| apply(&mut t, &st, cx));
            if status.changed().await.is_err() {
                break;
            }
        }
        // Fix round 1 (Minor 11): the status went away without a summary (the service dropped
        // the run): the run is over all the same, so Run is free again.
        if !t.ended {
            cx.update(|cx| run_ended(t.console, run, false, cx));
        }
    })
}

/// Follows a run's result tabs into the console's results.
pub(crate) fn follow_results(
    console: ConsoleId,
    mut results: watch::Receiver<Arc<[ResultTab]>>,
    cx: &mut App,
) -> Task<()> {
    cx.spawn(async move |cx| {
        loop {
            let tabs = results.borrow_and_update().clone();
            if !tabs.is_empty() {
                cx.update(|cx| {
                    if let Some(r) = Consoles::get(cx, console).and_then(|s| s.results.clone()) {
                        r.update(cx, |r, cx| {
                            r.apply(&tabs, cx);
                        });
                    }
                });
            }
            if results.changed().await.is_err() {
                break;
            }
        }
    })
}

/// A cancel was asked: the unit in flight shows `Cancelling` at once, until its outcome
/// arrives.
pub(crate) fn note_cancel(console: ConsoleId, cx: &mut App) {
    let in_flight = update_console_exec(cx, console, |e| {
        let run = e.run.as_mut()?;
        run.cancel_asked = true;
        let st = run.status.borrow().clone();
        let keys: Vec<MarkKey> = st
            .units
            .iter()
            .zip(&run.keys)
            .filter(|(u, _)| {
                matches!(
                    u.status,
                    UnitStatus::Running { .. } | UnitStatus::Fetching { .. }
                )
            })
            .map(|(_, k)| *k)
            .collect();
        Some((keys, e.editor.clone()))
    })
    .flatten();
    let Some((keys, editor)) = in_flight else {
        return;
    };
    let Some(editor) = editor.upgrade() else {
        return;
    };
    let since = cx.background_executor().now();
    let eol: TextFn = Rc::new(|cx: &App| message(cx, "hv-editor-mark-cancelling", vec![]));
    editor.update(cx, |e, cx| {
        for key in keys {
            e.set_exec_state(
                key,
                ExecMarkState::Cancelling { since },
                Some(eol.clone()),
                Some(eol.clone()),
                cx,
            );
            e.set_exec_short(key, None, cx);
        }
    });
}

fn apply(t: &mut Tracker, st: &RunStatus, cx: &mut App) {
    let Some(exec) = console_exec(cx, t.console) else {
        return;
    };
    let cancel_asked = exec
        .run
        .as_ref()
        .is_some_and(|r| Some(r.run) == st.run && r.cancel_asked);
    let Some(editor) = exec.editor.upgrade() else {
        return;
    };
    let now = cx.background_executor().now();
    let results = Consoles::get(cx, t.console).and_then(|s| s.results.clone());
    for (i, entry) in st.units.iter().enumerate() {
        let Some(key) = t.keys.get(i).copied() else {
            continue;
        };
        let seen = &mut t.seen[i];
        let in_flight = matches!(
            entry.status,
            UnitStatus::Running { .. } | UnitStatus::Fetching { .. }
        );
        if in_flight && seen.started.is_none() {
            seen.started = Some(now);
        }
        let cancelling = in_flight && cancel_asked;
        let rows = match entry.status {
            UnitStatus::Suspended { rows } => rows,
            UnitStatus::Succeeded(s) => match s.rows {
                Some(SessionRows::Fetched { rows, .. } | SessionRows::Affected(rows)) => rows,
                None => 0,
            },
            _ => 0,
        };
        let name = if cancelling {
            "Cancelling"
        } else if matches!(entry.status, UnitStatus::Fetching { .. }) {
            // Rows streaming in: still running, as far as the mark is concerned.
            "Running"
        } else {
            entry.status.name()
        };
        if seen.shown == Some((name, rows)) {
            continue;
        }
        seen.shown = Some((name, rows));
        let n = u32::try_from(i + 1).unwrap_or(u32::MAX);
        let started = seen.started.unwrap_or(now);
        let elapsed = now.saturating_duration_since(started);
        let (state, label, eol) = mark_of(&entry.status, n, started, elapsed, cancelling);
        // ## m3b-t23-escalation (named exception): a cancel not answered keeps its hourglass.
        let escalated = crate::exec_ui::escalation::escalated_mark(t.console, cx);
        let (state, label, eol) = match escalated.filter(|_| cancelling) {
            Some((state, eol)) => (state, eol.clone(), eol),
            None => (state, label, eol),
        };
        let short = short_of(&entry.status, started, elapsed, cancelling);
        let announce = a11y::exec_announcement(cx, n, &state);
        // ## m3c-t34-editor (named exception): an outcome keeps the wall-clock time it ended
        // at, for the gutter ▶'s "Last runs" (the format locale's medium time).
        let when = ended_at(&state);
        // A statement waiting for Commit or Roll back keeps its stamp while the unit still
        // reads as running (the question's follower set it; `orchestrator::mark_awaiting`).
        let awaiting = matches!(
            editor.read(cx).marks().state(key),
            Some(ExecMarkState::Awaiting)
        );
        if awaiting && matches!(state, ExecMarkState::Running { .. }) {
            continue;
        }
        editor.update(cx, |e, cx| {
            e.set_exec_state(key, state, Some(label), Some(eol), cx);
            e.set_exec_short(key, short, cx);
            if when.is_some() {
                e.set_exec_when(key, when, cx);
            }
            if let Some(text) = announce {
                e.announce(text, cx);
            }
        });
        if let (UnitStatus::Succeeded(s), Some(results)) = (&entry.status, results.as_ref()) {
            let tab = TabKey {
                console: t.console.0,
                unit_ordinal: entry.unit.ordinal,
                result_ordinal: 0,
            };
            let (exec_ms, fetch_ms) = (s.exec_ms, s.fetch_ms);
            results.update(cx, |r, cx| {
                r.set_timing(tab, Some(exec_ms), Some(fetch_ms), cx)
            });
        }
    }
    if st.summary.is_some() && !t.ended {
        t.ended = true;
        run_ended(t.console, st.run, cancel_asked, cx);
    }
}

/// The run's summary arrived: Run is free again; the SQL context is probed when the run may
/// have changed it; a cancel that failed the user's transaction says so (R-M3-16 amended).
fn run_ended(console: ConsoleId, run: Option<RunId>, cancel_asked: bool, cx: &mut App) {
    let active = update_console_exec(cx, console, |e| {
        e.run.as_mut().filter(|r| Some(r.run) == run).map(|r| {
            r.ended = true;
            (r.seq, r.session.clone())
        })
    })
    .flatten();
    orchestrator::on_run_ended(console, cx);
    let Some((seq, session)) = active.filter(|_| cancel_asked) else {
        return;
    };
    // ## m3a-f3-ci (fix round 1, review I1): the summary comes at `RunEnded`, while the session
    // is still in the run's request; it publishes the transaction the stop left only when it
    // leaves it (after a terminate still in flight, up to `CANCEL_WAIT`). So the transaction is
    // read once the session has left request `seq`; a session that goes away says nothing.
    let mut state = session.state();
    cx.spawn(async move |cx| {
        let left = state
            .wait_for(|s| {
                !matches!(
                    s.state,
                    SessionState::Busy { req, .. } | SessionState::Cancelling { req, .. }
                        if req == seq
                )
            })
            .await
            .map(|s| s.tx.clone());
        let Ok(tx) = left else {
            return;
        };
        if tx.owner == TxOwner::User && tx.status == TxStatus::Failed {
            cx.update(|cx| {
                let name = orchestrator::console_name(cx, console);
                let text: TextFn = Rc::new(move |cx: &App| {
                    message(
                        cx,
                        "hv-tx-failed-by-stop",
                        vec![("name", ArgValue::Text(name.clone()))],
                    )
                });
                ToastQueue::push(cx, Toast::new(ToastKind::Error, text));
            });
        }
    })
    .detach();
}

/// The short form of a unit's status on a long line (NOTES surface 5): a success's duration
/// (after the green check), a running statement's clock; none for the others (queued, skipped
/// and cancelled drop their words; a failure keeps its SQLSTATE chip).
// ## m3a-t35-shots (named exception): crate-visible for the shots fixtures (`exec_ui::fixtures`).
pub(crate) fn short_of(
    status: &UnitStatus,
    started: Instant,
    elapsed: Duration,
    cancelling: bool,
) -> Option<TextFn> {
    let done = |ms: u64| -> TextFn {
        Rc::new(move |cx: &App| {
            let d = duration_text(cx, ms);
            message(
                cx,
                "hv-editor-mark-short-done",
                vec![("duration", ArgValue::Text(d))],
            )
        })
    };
    match status {
        _ if cancelling => None,
        UnitStatus::Running { .. } | UnitStatus::Fetching { .. } => {
            Some(Rc::new(move |cx: &App| {
                let ms = millis(
                    cx.background_executor()
                        .now()
                        .saturating_duration_since(started),
                );
                let d = clock_text(cx, ms); // ## m3a-f2-visual
                message(
                    cx,
                    "hv-editor-mark-short-running",
                    vec![("duration", ArgValue::Text(d))],
                )
            }))
        }
        UnitStatus::Succeeded(s) => Some(done(s.exec_ms.saturating_add(s.fetch_ms))),
        UnitStatus::Suspended { .. } => Some(done(millis(elapsed))),
        _ => None,
    }
}

/// The mark of a unit in `status`: its state, its tooltip and its end-of-line words.
// ## m3a-t35-shots (named exception): crate-visible for the shots fixtures (`exec_ui::fixtures`).
pub(crate) fn mark_of(
    status: &UnitStatus,
    n: u32,
    started: Instant,
    elapsed: Duration,
    cancelling: bool,
) -> (ExecMarkState, TextFn, TextFn) {
    let fixed = |id: &'static str| -> TextFn { Rc::new(move |cx: &App| message(cx, id, vec![])) };
    let with_duration = |id: &'static str, ms: u64| -> TextFn {
        Rc::new(move |cx: &App| {
            let d = duration_text(cx, ms);
            message(cx, id, vec![("duration", ArgValue::Text(d))])
        })
    };
    if cancelling {
        let eol = fixed("hv-editor-mark-cancelling");
        return (
            ExecMarkState::Cancelling { since: started },
            eol.clone(),
            eol,
        );
    }
    match status {
        UnitStatus::Queued { .. } => {
            let eol = fixed("hv-editor-mark-queued");
            (ExecMarkState::Queued, eol.clone(), eol)
        }
        UnitStatus::Running { .. } | UnitStatus::Fetching { .. } => {
            // Read on every repaint (the 1 Hz clock).
            let eol: TextFn = Rc::new(move |cx: &App| {
                let ms = millis(
                    cx.background_executor()
                        .now()
                        .saturating_duration_since(started),
                );
                let d = clock_text(cx, ms); // ## m3a-f2-visual
                message(
                    cx,
                    "hv-editor-mark-running",
                    vec![("duration", ArgValue::Text(d))],
                )
            });
            (ExecMarkState::Running { started }, eol.clone(), eol)
        }
        UnitStatus::Cancelling => {
            let eol = fixed("hv-editor-mark-cancelling");
            (
                ExecMarkState::Cancelling { since: started },
                eol.clone(),
                eol,
            )
        }
        UnitStatus::Suspended { rows } => {
            let ms = millis(elapsed);
            let rows = *rows;
            let eol: TextFn = Rc::new(move |cx: &App| {
                let d = duration_text(cx, ms);
                message(
                    cx,
                    "hv-editor-mark-rows-more",
                    vec![
                        ("duration", ArgValue::Text(d)),
                        ("rows", ArgValue::Count(rows)),
                    ],
                )
            });
            (
                ExecMarkState::Succeeded {
                    took: elapsed,
                    rows: Some(RowsOutcome::Fetched { rows, more: true }),
                },
                eol.clone(),
                eol,
            )
        }
        UnitStatus::Succeeded(s) => {
            let ms = s.exec_ms.saturating_add(s.fetch_ms);
            let rows = s.rows.map(|r| match r {
                SessionRows::Fetched { rows, more } => RowsOutcome::Fetched { rows, more },
                SessionRows::Affected(n) => RowsOutcome::Affected(n),
            });
            let eol: TextFn = match rows {
                Some(RowsOutcome::Fetched { rows, more }) => Rc::new(move |cx: &App| {
                    let d = duration_text(cx, ms);
                    let id = if more {
                        "hv-editor-mark-rows-more"
                    } else {
                        "hv-editor-mark-rows"
                    };
                    message(
                        cx,
                        id,
                        vec![
                            ("duration", ArgValue::Text(d)),
                            ("rows", ArgValue::Count(rows)),
                        ],
                    )
                }),
                Some(RowsOutcome::Affected(n)) => Rc::new(move |cx: &App| {
                    let d = duration_text(cx, ms);
                    message(
                        cx,
                        "hv-editor-mark-affected",
                        vec![
                            ("rows", ArgValue::Count(n)),
                            ("duration", ArgValue::Text(d)),
                        ],
                    )
                }),
                None => with_duration("hv-editor-mark-done", ms),
            };
            (
                ExecMarkState::Succeeded {
                    took: Duration::from_millis(ms),
                    rows,
                },
                eol.clone(),
                eol,
            )
        }
        // ## m3c-t34-editor (named exception; the T24/T29 carry): a confirmed statement the user
        // (Roll back, Undo), a close or the deadline rolled back: the undo glyph, "rolled back";
        // the tooltip says which.
        UnitStatus::Failed(view)
            if matches!(
                view.title_id,
                "exec-error-confirm-undone" | "exec-error-confirm-timed-out"
            ) =>
        {
            let title = view.title_id;
            (
                ExecMarkState::RolledBack,
                fixed(title),
                fixed("hv-editor-mark-rolled-back"),
            )
        }
        UnitStatus::Failed(view) if view.title_id.starts_with("exec-deny-") => {
            // A read-only denial: nothing was sent; the tooltip says why.
            let title = view.title_id;
            (
                ExecMarkState::Failed { sqlstate: None },
                fixed(title),
                fixed("hv-editor-mark-denied"),
            )
        }
        UnitStatus::Failed(view) => {
            let n = n.to_string();
            let label: TextFn = Rc::new(move |cx: &App| {
                // A statement number stays raw (never grouped).
                message(cx, "exec-error", vec![("n", ArgValue::Text(n.clone()))])
            });
            (
                ExecMarkState::Failed {
                    sqlstate: view
                        .code
                        .as_deref()
                        .map(|c| SharedString::from(c.to_string())),
                },
                label,
                fixed("hv-editor-mark-failed"),
            )
        }
        UnitStatus::Cancelled => {
            let eol = with_duration("hv-editor-mark-cancelled", millis(elapsed));
            (ExecMarkState::Cancelled, eol.clone(), eol)
        }
        UnitStatus::Skipped => {
            let eol = fixed("hv-editor-mark-skipped");
            (ExecMarkState::Skipped, eol.clone(), eol)
        }
    }
}

// ## m3c-t34-editor
/// For an outcome, the time it ended at as the gutter ▶'s "Last runs" shows it (the history
/// dialog's time format); `None` while the unit is still going.
fn ended_at(state: &ExecMarkState) -> Option<TextFn> {
    let ended = matches!(
        state,
        ExecMarkState::Succeeded { .. }
            | ExecMarkState::Failed { .. }
            | ExecMarkState::Cancelled
            | ExecMarkState::Skipped
            | ExecMarkState::RolledBack
    );
    if !ended {
        return None;
    }
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    Some(Rc::new(move |cx: &App| {
        crate::history::rows::time_text(cx, ms)
    }))
}

// ## m3a-f2-visual: the running clock in whole seconds (NOTES surface 5, "running · 3 s").
#[cfg(test)]
mod clock_tests {
    use super::clock_message;

    fn msg(ms: u64) -> (&'static str, Vec<(&'static str, u64)>) {
        let m = clock_message(ms);
        (m.id, m.args)
    }

    /// A running clock repaints once a second, so it shows whole seconds: never the
    /// milliseconds of the frame that happened to paint it, and "0 s" before the first second.
    #[test]
    fn the_running_clock_counts_whole_seconds() {
        assert_eq!(msg(0), ("exec-dur-s", vec![("s", 0)]));
        assert_eq!(msg(999), ("exec-dur-s", vec![("s", 0)]));
        assert_eq!(msg(3_012), ("exec-dur-s", vec![("s", 3)]));
        assert_eq!(msg(59_999), ("exec-dur-s", vec![("s", 59)]));
        assert_eq!(msg(61_500), ("exec-dur-min-s", vec![("min", 1), ("s", 1)]));
        assert_eq!(msg(120_400), ("exec-dur-min", vec![("min", 2)]));
    }
}
