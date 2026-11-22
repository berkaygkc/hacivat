//! M4b Task 18 (the app's Run as script, HV-SCRIPT-12): what the runner gives the app beyond
//! Task 12's contract.
//!
//! - **The tab's database check before pass 2** (R-M4b-27, R-T14-2): `ScriptRunner::run_checked`
//!   runs the app's check after a clean pass 1 and before anything is sent; a refused check
//!   sends nothing, and the first unit is sent only under the context generation the check read.
//! - **Words that say what happened** (the T12 re-review's n1, n3 and n5): the item cap, a
//!   file that reads differently under modes the connection did not report, and a lost track of
//!   the connection's events each have their own reason and message.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::fake_pg;
use common::t12::{Answers, rig, script};
use hacivat_exec::{
    ChunkError, DatabaseCheck, SCRIPT_CHUNK_BYTES, ScriptPosition, ScriptProgress,
    ScriptRefuseReason, ScriptRunOutcome, ScriptStopReason,
};
use tokio::sync::watch;

fn refused(out: &ScriptRunOutcome) -> (ScriptRefuseReason, Option<ScriptPosition>) {
    match out {
        ScriptRunOutcome::Refused(r) => (r.reason.clone(), r.at),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn stopped(out: &ScriptRunOutcome) -> (ScriptStopReason, Option<ScriptPosition>) {
    match out {
        ScriptRunOutcome::Stopped(s, _) => (s.reason.clone(), s.at),
        other => panic!("expected a stop, got {other:?}"),
    }
}

/// `unit` repeated on one line, `left` bytes in all, never read twice.
struct Repeat {
    unit: &'static str,
    left: usize,
}

impl hacivat_exec::ScriptChunks for Repeat {
    fn next_chunk(&mut self) -> Option<Result<String, ChunkError>> {
        if self.left == 0 {
            return None;
        }
        let n = (SCRIPT_CHUNK_BYTES / self.unit.len())
            .min(self.left / self.unit.len())
            .max(1);
        self.left = self.left.saturating_sub(n * self.unit.len());
        Some(Ok(self.unit.repeat(n)))
    }

    fn reopen_verified(
        self: Box<Self>,
    ) -> futures::future::BoxFuture<'static, Result<Box<dyn hacivat_exec::ScriptChunks>, ChunkError>>
    {
        Box::pin(async { Err(ChunkError::Changed) })
    }
}

/// A check that answers `answer` and counts its calls, noting how many statements of `of` the
/// rig's fake had run when it was called.
fn check_answering(
    answer: Option<u64>,
    calls: Arc<AtomicUsize>,
    sent_at_call: Arc<std::sync::Mutex<Option<usize>>>,
    sent: impl Fn() -> usize + Send + Sync + 'static,
) -> DatabaseCheck {
    Box::new(move || {
        calls.fetch_add(1, Ordering::SeqCst);
        *sent_at_call.lock().unwrap() = Some(sent());
        Box::pin(async move { answer })
    })
}

async fn run_checked(
    r: &common::t12::Rig,
    req: hacivat_exec::ScriptRunRequest,
    check: Option<DatabaseCheck>,
) -> ScriptRunOutcome {
    let (tx, _rx) = watch::channel(ScriptProgress::default());
    r.runner.run_checked(req, tx, check).await
}

// ---- the database check before pass 2 -----------------------------------------------------

#[tokio::test]
async fn the_database_check_runs_between_the_passes_before_anything_is_sent() {
    let r = Arc::new(rig(fake_pg(), false).await);
    let units = ["SELECT 1", "SELECT 2"];
    let calls = Arc::new(AtomicUsize::new(0));
    let sent_at = Arc::new(std::sync::Mutex::new(None));
    // The check reads the generation the way the app does: on the tab's session, after every
    // earlier request.
    let (_, generation) = r.session.run_context().await;
    let generation = generation.expect("a context generation");
    let probe = r.clone();
    let check = check_answering(
        Some(generation),
        calls.clone(),
        sent_at.clone(),
        move || probe.sent(&units).len(),
    );
    let req = r.request(
        script("SELECT 1;\nSELECT 2;\n").boxed(),
        None,
        Answers::new(&[]),
    );
    let out = run_checked(&r, req, Some(check)).await;
    assert!(matches!(out, ScriptRunOutcome::Finished(_)), "{out:?}");
    assert_eq!(calls.load(Ordering::SeqCst), 1, "checked once");
    assert_eq!(
        *sent_at.lock().unwrap(),
        Some(0),
        "nothing was sent before the check"
    );
    assert_eq!(r.sent(&units), units);
}

#[tokio::test]
async fn a_refused_database_check_sends_nothing() {
    let r = rig(fake_pg(), false).await;
    let units = ["SELECT 1", "DELETE FROM t WHERE id = 1"];
    let calls = Arc::new(AtomicUsize::new(0));
    let check = check_answering(None, calls.clone(), Arc::default(), || 0);
    let req = r.request(
        script("SELECT 1;\nDELETE FROM t WHERE id = 1;\n").boxed(),
        None,
        Answers::new(&[]),
    );
    let out = run_checked(&r, req, Some(check)).await;
    assert_eq!(refused(&out), (ScriptRefuseReason::DatabaseCheck, None));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(r.sent(&units), Vec::<String>::new());
}

#[tokio::test]
async fn a_file_pass_one_refuses_is_never_checked() {
    let r = rig(fake_pg(), false).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let check = check_answering(Some(0), calls.clone(), Arc::default(), || 0);
    let req = r.request(
        script("SELECT 1;\nSELECT 'unterminated").boxed(),
        None,
        Answers::new(&[]),
    );
    let out = run_checked(&r, req, Some(check)).await;
    assert!(matches!(out, ScriptRunOutcome::Refused(_)), "{out:?}");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "no check for a refused file"
    );
    assert_eq!(r.sent(&["SELECT 1"]), Vec::<String>::new());
}

#[tokio::test]
async fn a_context_that_moved_after_the_check_stops_before_the_first_unit() {
    // The check read generation g; the session's context is at another one when the first unit
    // is read (a reconnect, a replay, a queued USE in between): nothing is sent.
    let r = rig(fake_pg(), false).await;
    let (_, generation) = r.session.run_context().await;
    let stale = generation.expect("a generation").wrapping_add(1);
    let check = check_answering(Some(stale), Arc::default(), Arc::default(), || 0);
    let req = r.request(
        script("SELECT 1;\nSELECT 2;\n").boxed(),
        None,
        Answers::new(&[]),
    );
    let out = run_checked(&r, req, Some(check)).await;
    assert_eq!(
        stopped(&out),
        (
            ScriptStopReason::ContextChanged,
            Some(ScriptPosition { unit: 1, line: 1 })
        )
    );
    assert_eq!(r.sent(&["SELECT 1", "SELECT 2"]), Vec::<String>::new());
}

#[tokio::test]
async fn stop_while_the_check_waits_sends_nothing() {
    let r = rig(fake_pg(), false).await;
    let cancel = r.cancel.clone();
    let check: DatabaseCheck = Box::new(move || {
        cancel.send_replace(true);
        Box::pin(std::future::pending())
    });
    let req = r.request(script("SELECT 1;\n").boxed(), None, Answers::new(&[]));
    let out = run_checked(&r, req, Some(check)).await;
    assert_eq!(refused(&out), (ScriptRefuseReason::Cancelled, None));
    assert_eq!(r.sent(&["SELECT 1"]), Vec::<String>::new());
}

// ---- words (T12 re-review n1, n3, n5) -----------------------------------------------------

#[tokio::test]
async fn the_item_cap_has_its_own_reason() {
    // n1: a line of tiny statements is refused by the item cap, long before 64 MiB of text.
    let r = rig(fake_pg(), false).await;
    let line = Box::new(Repeat {
        unit: "SELECT 1;",
        left: 4 << 20,
    });
    let (out, _) = r.run(r.request(line, None, Answers::new(&[]))).await;
    assert_eq!(
        refused(&out),
        (
            ScriptRefuseReason::NoBoundaryItems,
            Some(ScriptPosition { unit: 1, line: 1 })
        )
    );
    assert_eq!(
        ScriptRefuseReason::NoBoundaryItems.fluent_id(),
        "exec-script-no-boundary-items"
    );
}

#[tokio::test]
async fn unknown_modes_that_read_differently_have_their_own_reason() {
    // n3: the session's modes are unknown and the file reads differently under them.
    let r = rig(fake_pg(), false).await;
    let file = "SELECT 'a\\';\nDELETE FROM t WHERE id = 1;\n-- ';\n";
    let (out, _) = r
        .run(r.request(script(file).boxed(), None, Answers::new(&[])))
        .await;
    assert_eq!(refused(&out), (ScriptRefuseReason::ModesUnknown, None));
    assert!(r.all_sent().iter().all(|q| !q.contains("DELETE")));
    assert_eq!(
        ScriptRefuseReason::ModesUnknown.fluent_id(),
        "exec-script-modes-unknown"
    );
}

#[test]
fn a_lost_track_stop_has_its_own_words() {
    // n5: a lagged or closed event stream is "lost track", never "the connection was reset".
    assert_eq!(
        ScriptStopReason::LostTrack.fluent_id(),
        "exec-script-lost-track"
    );
    assert_ne!(
        ScriptStopReason::LostTrack.fluent_id(),
        ScriptStopReason::Reconnected.fluent_id()
    );
}

#[test]
fn the_new_reasons_have_their_words_in_both_locales() {
    let ids = [
        ScriptRefuseReason::NoBoundaryItems.fluent_id(),
        ScriptRefuseReason::ModesUnknown.fluent_id(),
        ScriptRefuseReason::DatabaseCheck.fluent_id(),
        ScriptStopReason::LostTrack.fluent_id(),
    ];
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../locales");
    for locale in ["en", "tr"] {
        let ftl = std::fs::read_to_string(root.join(locale).join("hacivat-core.ftl"))
            .expect("the locale's core messages");
        for id in ids {
            assert!(
                ftl.lines().any(|l| l.starts_with(&format!("{id} ="))),
                "{locale}: {id} has no message"
            );
        }
    }
    let en = std::fs::read_to_string(root.join("en/hacivat-core.ftl")).unwrap();
    assert!(en.contains("lost track of the connection's state"));
    assert!(en.contains("statements on line"));
}
