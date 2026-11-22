//! M4b Task 12: the reduced "Run as script" (HV-EXEC-15 reduced form, HV-SCRIPT-12; R-M4b-20,
//! R-M4b-24, R-M4b-47, R-T8-1, R-T10-3): a file streamed twice through the splitter, nothing
//! run unless the first pass is clean, then every unit classified, gated and confirmed exactly
//! as Run does, one at a time, on the tab's session.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use common::t12::{Answers, FileScript, Pass2, Rig, managed_rig, pg_context, rig, script};
use common::{eventually, fake_pg, pg_error};
use hacivat_driver::fake::{FakeDriver, FakeQuery};
use hacivat_exec::{
    ChunkError, ConfirmAnswer, ErrorPolicy, HistoryMeta, HistoryOrigin, HistoryRecord, LogMsg,
    Outcome, SCRIPT_CHUNK_BYTES, ScriptHistory, ScriptPhase, ScriptPosition, ScriptRefuseReason,
    ScriptRunOutcome, ScriptStopReason,
};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use sql_split::AmbiguityReason;
use tokio::sync::mpsc;

/// The refusal of `out`, or a panic naming it.
fn refused(out: &ScriptRunOutcome) -> (ScriptRefuseReason, Option<ScriptPosition>) {
    match out {
        ScriptRunOutcome::Refused(r) => (r.reason.clone(), r.at),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// The stop of `out`, or a panic naming it.
fn stopped(out: &ScriptRunOutcome) -> (ScriptStopReason, Option<ScriptPosition>) {
    match out {
        ScriptRunOutcome::Stopped(s, _) => (s.reason.clone(), s.at),
        other => panic!("expected a stop, got {other:?}"),
    }
}

fn at(unit: u64, line: u64) -> Option<ScriptPosition> {
    Some(ScriptPosition { unit, line })
}

fn mysql() -> FakeDriver {
    FakeDriver::new(DbmsKind::MySql)
}

fn mariadb() -> FakeDriver {
    FakeDriver::new(DbmsKind::MariaDb)
}

/// A query starting with `prefix` that fails at once.
fn failing(prefix: &'static str) -> FakeQuery {
    FakeQuery {
        prefix,
        fail_after: Some((0, pg_error("42P01", "relation \"nope\" does not exist"))),
        sets: vec![common::generated(1)],
        ..Default::default()
    }
}

/// Nothing at all reached the fake from `r`'s script: no statement of `units`.
fn nothing_of(r: &Rig, units: &[&str]) {
    assert_eq!(r.sent(units), Vec::<String>::new(), "a statement was sent");
}

#[test]
fn every_reason_has_its_words_in_both_locales() {
    use hacivat_exec::GateReason;
    let refusals = [
        ScriptRefuseReason::Uncertain(AmbiguityReason::UnknownBlock),
        ScriptRefuseReason::NotExact,
        ScriptRefuseReason::TooLong,
        ScriptRefuseReason::NoBoundary,
        ScriptRefuseReason::ClientCommand,
        ScriptRefuseReason::Params(None),
        ScriptRefuseReason::CopyUnsupported,
        ScriptRefuseReason::Denied(GateReason::of("exec-gate-read-only")),
        ScriptRefuseReason::Read(ChunkError::Changed),
        ScriptRefuseReason::Read(ChunkError::Decode),
        ScriptRefuseReason::Read(ChunkError::Io(std::io::ErrorKind::Other)),
        ScriptRefuseReason::ModesDiffer,
        ScriptRefuseReason::Cancelled,
    ];
    let stops = [
        ScriptStopReason::Cancelled,
        ScriptStopReason::FileChanged,
        ScriptStopReason::Read(ChunkError::Decode),
        ScriptStopReason::Denied(GateReason::of("exec-gate-read-only")),
        ScriptStopReason::ConfirmCancelled,
        ScriptStopReason::ContextChanged,
        ScriptStopReason::ContextUnconfirmed,
        ScriptStopReason::ConnectionChanged,
        ScriptStopReason::Reconnected,
        ScriptStopReason::Read(ChunkError::Io(std::io::ErrorKind::Other)),
        ScriptStopReason::NotSent,
        ScriptStopReason::Split(ScriptRefuseReason::NoBoundary),
    ];
    let ids: Vec<&str> = refusals
        .iter()
        .map(ScriptRefuseReason::fluent_id)
        .chain(stops.iter().map(ScriptStopReason::fluent_id))
        .collect();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../locales");
    for locale in ["en", "tr"] {
        let ftl = std::fs::read_to_string(root.join(locale).join("hacivat-core.ftl"))
            .expect("the locale's core messages");
        for id in &ids {
            assert!(
                ftl.lines().any(|l| l.starts_with(&format!("{id} ="))),
                "{locale}: {id} has no message"
            );
        }
    }
    // R-T8-3's wording.
    let en = std::fs::read_to_string(root.join("en/hacivat-core.ftl")).unwrap();
    assert!(en.contains("no statement boundary within 64 MiB"));
}

// ---- the gate, per unit ---------------------------------------------------------------------

#[tokio::test]
async fn every_unit_goes_through_the_gate() {
    let r = rig(fake_pg(), false).await;
    let port = Answers::new(&[ConfirmAnswer::Cancel]);
    let units = ["SELECT 1", "DELETE FROM t", "SELECT 2"];
    let src = script("SELECT 1;\nDELETE FROM t;\nSELECT 2;\n").boxed();
    let (out, _) = r
        .run(r.request(src, Some(EnvTag::Production), port.clone()))
        .await;
    assert_eq!(
        stopped(&out),
        (ScriptStopReason::ConfirmCancelled, at(2, 2))
    );
    assert_eq!(r.sent(&units), ["SELECT 1"]);
    // The question names the statement and the run's total, with a typed name on production.
    let q = port.asked.lock().unwrap()[0].clone();
    assert_eq!((q.index, q.total), (2, 3));
    assert!(q.production);
    assert!(q.type_to_confirm.is_some(), "{q:?}");
    assert_eq!(&*q.sql_masked, "DELETE FROM t");
}

#[tokio::test]
async fn run_as_script_gates_every_unit() {
    // alg/28: three units, the second destructive: the gate asks for the second only.
    let r = rig(fake_pg(), false).await;
    let port = Answers::new(&[ConfirmAnswer::Execute]);
    let units = ["SELECT 1", "DELETE FROM t", "SELECT 2"];
    let src = script("SELECT 1;\nDELETE FROM t;\nSELECT 2;\n").boxed();
    let (out, progress) = r.run(r.request(src, Some(EnvTag::Dev), port.clone())).await;
    let ScriptRunOutcome::Finished(s) = out else {
        panic!("expected the run to finish, got {out:?}");
    };
    assert_eq!((s.units_total, s.units_run, s.units_failed), (3, 3, 0));
    assert_eq!(port.asked_units(), [2]);
    assert_eq!(r.sent(&units), units);
    assert_eq!(progress.phase, ScriptPhase::Done);
    assert_eq!(progress.units_run, 3);
}

#[tokio::test]
async fn classified_bytes_equal_sent_bytes() {
    // The M3b rule (preview equals what Run sends), per streamed unit: the runner's classifier
    // saw exactly the texts the session sent, in the same order.
    let r = rig(fake_pg(), false).await;
    let text = "SELECT 1;\n-- a comment\nUPDATE t SET a = 'x;y' WHERE id = 1;\n\nSELECT $$a;b$$;\n";
    let units = [
        "SELECT 1",
        "UPDATE t SET a = 'x;y' WHERE id = 1",
        "SELECT $$a;b$$",
    ];
    let (out, _) = r
        .run(r.request(script(text).boxed(), None, Answers::new(&[])))
        .await;
    assert!(matches!(out, ScriptRunOutcome::Finished(_)), "{out:?}");
    let classified = r.classified.lock().unwrap().clone();
    // Pass 1 gates every unit (R-T12-2), then pass 2 classifies each again right before it is
    // sent: the second reading is exactly what was sent.
    assert_eq!(classified, [units, units].concat());
    assert_eq!(r.sent(&units), classified[units.len()..]);
}

#[tokio::test]
async fn read_only_profile_denies_writes_unit_by_unit() {
    // R-T12-2 (fix round 1, I5): every unit meets the gate in pass 1 too; the write a read-only
    // connection denies refuses the file, so not even the read before it runs.
    let r = rig(common::t12::pg_read_only(3), true).await;
    // The session sends its own `SELECT 1` at connect: the script reads other numbers.
    let units = ["SELECT 10", "DELETE FROM t WHERE id = 1", "SELECT 20"];
    let src = script("SELECT 10;\nDELETE FROM t WHERE id = 1;\nSELECT 20;\n").boxed();
    let (out, _) = r.run(r.request(src, None, Answers::new(&[]))).await;
    let (reason, pos) = refused(&out);
    assert!(
        matches!(&reason, ScriptRefuseReason::Denied(g) if g.fluent_id == "exec-gate-read-only"),
        "{reason:?}"
    );
    assert_eq!(pos, at(2, 2));
    nothing_of(&r, &units);
}

#[tokio::test]
async fn a_denied_statement_late_in_a_file_sends_nothing() {
    // R-T12-2: a denial three chunks in refuses the file before the first unit runs. The log
    // says why, naming the line, with the gate's own reason on the next line.
    let r = rig(common::t12::pg_read_only(8), true).await;
    let text = format!("{}DELETE FROM t WHERE id = 1;\n", chunked_script(3));
    let line = text.lines().count() as u64;
    let (out, _) = r
        .run(r.request(script(&text).boxed(), None, Answers::new(&[])))
        .await;
    let (reason, pos) = refused(&out);
    assert!(
        matches!(reason, ScriptRefuseReason::Denied(_)),
        "{reason:?}"
    );
    assert_eq!(pos.map(|p| p.line), Some(line));
    nothing_of(&r, &["SELECT 1", "SELECT 2", "SELECT 3"]);
    assert!(r.all_sent().iter().all(|q| !q.starts_with("SELECT '")));
    let ids: Vec<&str> = r.log.lines().iter().map(|l| l.msg.fluent_id()).collect();
    assert_eq!(ids, ["exec-script-denied-before", "exec-gate-read-only"]);
}

#[tokio::test]
async fn execute_all_never_answers_a_typed_name_and_skip_skips() {
    // M3b NOTES ruling 32 / RC-1: on production Execute all covers nothing, so the second
    // destructive unit asks again; Skip leaves a unit unsent and the run goes on.
    let r = rig(fake_pg(), false).await;
    let port = Answers::new(&[ConfirmAnswer::ExecuteAll, ConfirmAnswer::Skip]);
    let units = ["DELETE FROM a", "DELETE FROM b", "SELECT 3"];
    let src = script("DELETE FROM a;\nDELETE FROM b;\nSELECT 3;\n").boxed();
    let (out, _) = r
        .run(r.request(src, Some(EnvTag::Production), port.clone()))
        .await;
    let ScriptRunOutcome::Finished(s) = out else {
        panic!("expected the run to finish, got {out:?}");
    };
    assert_eq!(port.asked_units(), [1, 2]);
    assert_eq!((s.units_run, s.units_skipped), (2, 1));
    assert_eq!(r.sent(&units), ["DELETE FROM a", "SELECT 3"]);
    // Off production Execute all answers the later calm confirmations.
    let r = rig(fake_pg(), false).await;
    let port = Answers::new(&[ConfirmAnswer::ExecuteAll]);
    let src = script("DELETE FROM a;\nDELETE FROM b;\nSELECT 3;\n").boxed();
    let (out, _) = r.run(r.request(src, Some(EnvTag::Dev), port.clone())).await;
    assert!(matches!(out, ScriptRunOutcome::Finished(_)), "{out:?}");
    assert_eq!(port.asked_units(), [1]);
    assert_eq!(r.sent(&units), units);
}

// ---- parameters, errors, stop ---------------------------------------------------------------

#[tokio::test]
async fn bindable_placeholder_refuses_the_run_before_sending_it() {
    let r = rig(fake_pg(), false).await;
    let units = ["SELECT 1", "SELECT * FROM t WHERE id = :id"];
    let src = script("SELECT 1;\nSELECT * FROM t WHERE id = :id;\n").boxed();
    let (out, _) = r.run(r.request(src, None, Answers::new(&[]))).await;
    assert_eq!(refused(&out), (ScriptRefuseReason::Params(None), at(2, 2)));
    nothing_of(&r, &units);
}

#[tokio::test]
async fn stop_on_error_is_the_default() {
    assert_eq!(ErrorPolicy::default(), ErrorPolicy::StopOnError);
    // alg/28 `run_as_script_stops_on_first_error`: unit 2 fails, unit 3 is never sent.
    let r = rig(fake_pg().queries(vec![failing("UPDATE nope")]), false).await;
    let units = [
        "UPDATE a SET x = 1 WHERE id = 1",
        "UPDATE nope SET x = 1 WHERE id = 1",
        "UPDATE c SET x = 1 WHERE id = 1",
    ];
    let src = script(&format!("{};\n{};\n{};\n", units[0], units[1], units[2]));
    let req = r.request(src.boxed(), None, Answers::new(&[]));
    let (out, _) = r.run(req).await;
    let ScriptRunOutcome::Failed(s) = out else {
        panic!("expected a failure, got {out:?}");
    };
    assert_eq!((s.units_run, s.units_failed), (1, 1));
    assert_eq!(s.failed_at, at(2, 2));
    assert_eq!(r.sent(&units), units[..2]);
}

/// One padding statement of about 1 KiB (`SELECT 'ppp…'`).
fn pad_stmt() -> String {
    format!("SELECT '{}';\n", "p".repeat(1000))
}

/// At least `n` bytes of padding statements, pushing the next statement into a later chunk.
fn padding(n: usize) -> String {
    pad_stmt().repeat(n / pad_stmt().len() + 1)
}

#[tokio::test]
async fn cancel_stops_between_units_and_mid_unit() {
    // Between units: Stop is pressed while the second chunk is read; the unit after it is
    // never sent.
    let r = rig(fake_pg(), false).await;
    let cancel = r.cancel.clone();
    let text = format!("SELECT 1;\n{}SELECT 2;\n", padding(2 * SCRIPT_CHUNK_BYTES));
    let src = script(&text).hook(move |pass, read| {
        if pass == 2 && read > SCRIPT_CHUNK_BYTES {
            cancel.send_replace(true);
        }
    });
    let (out, _) = r.run(r.request(src.boxed(), None, Answers::new(&[]))).await;
    assert_eq!(stopped(&out).0, ScriptStopReason::Cancelled);
    assert_eq!(r.sent(&["SELECT 1", "SELECT 2"]), ["SELECT 1"]);

    // Mid-unit: the second statement runs for a minute unless cancelled.
    let slow = FakeQuery {
        prefix: "SELECT 2",
        sets: vec![common::generated(1)],
        first_delay: Duration::from_secs(60),
        ..Default::default()
    };
    let r = rig(fake_pg().queries(vec![slow]), false).await;
    let units = ["SELECT 1", "SELECT 2", "SELECT 3"];
    let src = script("SELECT 1;\nSELECT 2;\nSELECT 3;\n").boxed();
    let req = r.request(src, None, Answers::new(&[]));
    let started = Instant::now();
    let (out, _) = tokio::join!(r.run(req), async {
        eventually("SELECT 2 is sent", || r.sent(&units).len() == 2).await;
        r.cancel.send_replace(true);
    })
    .0;
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "the cancel ended the unit"
    );
    assert_eq!(stopped(&out), (ScriptStopReason::Cancelled, at(2, 2)));
    assert_eq!(r.sent(&units), ["SELECT 1", "SELECT 2"]);
}

#[tokio::test]
async fn one_summary_history_row() {
    let r = rig(fake_pg(), false).await;
    let (tx, mut rx) = mpsc::channel::<Arc<HistoryRecord>>(16);
    let profile = ProfileId::new();
    let mut req = r.request(
        script("SELECT 1;\nSELECT 2;\nSELECT 3;\n").boxed(),
        None,
        Answers::new(&[]),
    );
    req.history = Some(ScriptHistory {
        sink: tx,
        meta: HistoryMeta {
            profile,
            console: 7,
            private: false,
            origin: HistoryOrigin::File,
        },
        file_name: Arc::from("orders_dump.sql"),
        database: Some("orders".into()),
    });
    let (out, _) = r.run(req).await;
    assert!(matches!(out, ScriptRunOutcome::Finished(_)), "{out:?}");
    let row = rx.recv().await.expect("one row");
    assert!(rx.try_recv().is_err(), "a second history row");
    assert_eq!(row.origin, HistoryOrigin::File);
    assert_eq!(row.profile, profile);
    assert_eq!(&*row.statement_masked, "orders_dump.sql");
    assert_eq!(row.outcome, Outcome::Ok);
    assert_eq!(row.database.as_deref(), Some("orders"));
}

#[tokio::test]
async fn non_utf8_file_is_decoded_before_splitting() {
    // A windows-1254 file, decoded by its source (as `hacivat_scripts` decodes with the file's
    // encoding): the units carry the decoded text.
    const BYTES: &[u8] = b"INSERT INTO m VALUES ('\xdd\xfe\xf0\xfd');\nSELECT 1;\n";
    let decoded: String = BYTES
        .iter()
        .map(|&b| match b {
            0xdd => 'İ',
            0xfe => 'ş',
            0xf0 => 'ğ',
            0xfd => 'ı',
            b => char::from(b),
        })
        .collect();
    let r = rig(fake_pg(), false).await;
    let units = ["INSERT INTO m VALUES ('İşğı')", "SELECT 1"];
    // Chunks of 23 bytes cut the decoded text inside a character: the source widens them.
    let src = script(&decoded).sizes(1, &[23]).sizes(2, &[24]).boxed();
    let (out, _) = r.run(r.request(src, None, Answers::new(&[]))).await;
    assert!(matches!(out, ScriptRunOutcome::Finished(_)), "{out:?}");
    assert_eq!(r.sent(&units), units);
}

// ---- R-M4b-47: two passes, nothing run unless pass 1 is clean -------------------------------

#[tokio::test]
async fn a_block_mismatch_refuses_the_file_before_any_unit() {
    // The plan's `a_block_mismatch_stops_before_the_next_unit`, under R-M4b-47: the stray
    // `END` stops pass 1, so not even the statements before it run.
    let r = rig(mysql(), false).await;
    let units = ["SELECT 1", "SELECT 2", "DELETE FROM t"];
    let src = script("SELECT 1; SELECT 2; END; DELETE FROM t;").boxed();
    let (out, _) = r.run(r.request(src, None, Answers::new(&[]))).await;
    let (reason, pos) = refused(&out);
    assert_eq!(
        reason,
        ScriptRefuseReason::Uncertain(AmbiguityReason::UnknownBlock)
    );
    assert_eq!(pos.map(|p| p.line), Some(1));
    nothing_of(&r, &units);
}

#[tokio::test]
async fn r1_my_stray_end_executes_nothing() {
    // `sql-testkit/corpus/split/mysql.toml` `R1-my-stray-end`: the eager stream gives out
    // `DELETE FROM t` before its stop; pass 1 refuses the file, so nothing runs. On MySQL and
    // MariaDB, in every chunking tried.
    let text = "CREATE PROCEDURE p() BEGIN SELECT 1; END; DELETE FROM t; END;\nSELECT 2;";
    let units = [
        "CREATE PROCEDURE p() BEGIN SELECT 1",
        "END",
        "DELETE FROM t",
        "SELECT 2",
    ];
    for fake in [mysql as fn() -> FakeDriver, mariadb] {
        for sizes in [vec![1], vec![7], vec![3, 1, 17], vec![64 << 10]] {
            let r = rig(fake(), false).await;
            let src = script(text).sizes(1, &sizes).sizes(2, &sizes).boxed();
            let (out, progress) = r.run(r.request(src, None, Answers::new(&[]))).await;
            assert!(
                matches!(refused(&out).0, ScriptRefuseReason::Uncertain(_)),
                "{sizes:?}: {out:?}"
            );
            assert_eq!(progress.units_run, 0);
            assert!(
                r.all_sent().iter().all(|q| !q.contains("DELETE")),
                "{sizes:?}"
            );
            nothing_of(&r, &units);
        }
    }
}

#[tokio::test]
async fn mariadb_oracle_block_executes_nothing() {
    // MariaDB `sql_mode=ORACLE` anonymous blocks and routines the splitter cannot see as
    // blocks (T8 review I1): set by the script and by the session's modes. Nothing runs, not
    // even the `SET` before them.
    let anonymous = "DECLARE\n  v INT;\nBEGIN\n  NULL;\n  DELETE FROM t;\nEND;\nSELECT 2;\n";
    let routine = "CREATE PROCEDURE p AS\nBEGIN\n  DELETE FROM t;\nEND;\nSELECT 2;\n";
    for body in [anonymous, routine] {
        let by_script = format!("SET sql_mode='ORACLE';\n{body}");
        let r = rig(mariadb(), false).await;
        let src = script(&by_script).boxed();
        let (out, _) = r.run(r.request(src, None, Answers::new(&[]))).await;
        assert!(matches!(out, ScriptRunOutcome::Refused(_)), "{out:?}");
        assert!(r.all_sent().iter().all(|q| !q.contains("DELETE")));
        nothing_of(&r, &["SET sql_mode='ORACLE'", "SELECT 2", "DELETE FROM t"]);

        let r = rig(mariadb(), false).await;
        let mut req = r.request(script(body).boxed(), None, Answers::new(&[]));
        req.dialect.modes.mariadb_oracle_mode = true;
        let (out, _) = r.run(req).await;
        assert!(matches!(out, ScriptRunOutcome::Refused(_)), "{out:?}");
        nothing_of(&r, &["SELECT 2", "DELETE FROM t", "NULL"]);
    }
}

#[tokio::test]
async fn a_refusal_mid_file_refuses_the_whole_file() {
    // T8 fr1 n1: a `DelimiterChange` mid-file does not stop the stream (`stopped()` is false),
    // but pass 1 checks every batch with `any_refuses`, so nothing before it runs either.
    let text = "SELECT 1;\nDELIMITER $$\nCREATE PROCEDURE p() SELECT begin FROM t; DELETE FROM x; SELECT end FROM t$$\nDELIMITER ;\nSELECT 2;\n";
    for sizes in [vec![1], vec![64 << 10]] {
        let r = rig(mysql(), false).await;
        let src = script(text).sizes(1, &sizes).sizes(2, &sizes).boxed();
        let (out, _) = r.run(r.request(src, None, Answers::new(&[]))).await;
        assert!(
            matches!(
                refused(&out),
                (ScriptRefuseReason::Uncertain(_) | ScriptRefuseReason::NotExact, Some(p)) if p.line == 3
            ),
            "{out:?}"
        );
        nothing_of(&r, &["SELECT 1", "SELECT 2"]);
    }
}

#[tokio::test]
async fn pg_runs_two_passes_too() {
    // An unterminated string at the end: M3's whole split refuses the script, so pass 1 does.
    let r = rig(fake_pg(), false).await;
    let src = script("SELECT 1;\nSELECT 'unterminated;\n").boxed();
    let (out, _) = r.run(r.request(src, None, Answers::new(&[]))).await;
    assert!(
        matches!(refused(&out), (ScriptRefuseReason::Uncertain(_) | ScriptRefuseReason::NotExact, Some(p)) if p.line == 2),
        "{out:?}"
    );
    nothing_of(&r, &["SELECT 1"]);
}

#[tokio::test]
async fn unsupported_client_command_and_copy_refuse_the_file() {
    let r = rig(fake_pg(), false).await;
    let src = script("SELECT 1;\n\\c other\nSELECT 2;\n").boxed();
    let (out, _) = r.run(r.request(src, None, Answers::new(&[]))).await;
    assert_eq!(refused(&out), (ScriptRefuseReason::ClientCommand, at(2, 2)));
    nothing_of(&r, &["SELECT 1", "SELECT 2"]);

    let r = rig(fake_pg(), false).await;
    let src = script("SELECT 1;\nCOPY t FROM STDIN;\n1\n\\.\nSELECT 2;\n").boxed();
    let (out, _) = r.run(r.request(src, None, Answers::new(&[]))).await;
    assert_eq!(
        refused(&out),
        (ScriptRefuseReason::CopyUnsupported, at(2, 2))
    );
    nothing_of(&r, &["SELECT 1", "SELECT 2"]);
}

#[tokio::test]
async fn a_read_error_in_pass_one_runs_nothing() {
    let r = rig(fake_pg(), false).await;
    let text = format!("SELECT 1;\n{}SELECT 2;\n", padding(2 * SCRIPT_CHUNK_BYTES));
    let src = script(&text).pass1_fails_at(1, ChunkError::Decode).boxed();
    let (out, _) = r.run(r.request(src, None, Answers::new(&[]))).await;
    assert_eq!(
        refused(&out).0,
        ScriptRefuseReason::Read(ChunkError::Decode)
    );
    nothing_of(&r, &["SELECT 1", "SELECT 2"]);
}

// ---- R-T10-3: change detection between and during the passes --------------------------------

#[tokio::test]
async fn a_change_between_the_passes_executes_nothing() {
    let r = rig(fake_pg(), false).await;
    let src = script("SELECT 1;\nSELECT 2;\n");
    let stats = src.stats.clone();
    let src = src.pass2(Pass2::Refused(ChunkError::Changed)).boxed();
    let (out, _) = r.run(r.request(src, None, Answers::new(&[]))).await;
    assert_eq!(
        refused(&out),
        (ScriptRefuseReason::Read(ChunkError::Changed), None)
    );
    assert_eq!(stats.reopens.load(Ordering::SeqCst), 1);
    nothing_of(&r, &["SELECT 1", "SELECT 2"]);
}

/// A script of `n` 64 KiB chunks, each starting with `SELECT <i>;` and filled with padding
/// statements up to the chunk's end.
fn chunked_script(n: usize) -> String {
    let mut s = String::new();
    for i in 1..=n {
        s.push_str(&format!("SELECT {i};\n"));
        while s.len() + pad_stmt().len() <= i * SCRIPT_CHUNK_BYTES {
            s.push_str(&pad_stmt());
        }
        let fill = i * SCRIPT_CHUNK_BYTES - s.len();
        if fill >= 12 {
            s.push_str(&format!("SELECT '{}';\n", "q".repeat(fill - 12)));
        }
        while s.len() < i * SCRIPT_CHUNK_BYTES {
            s.push('\n');
        }
    }
    s
}

#[tokio::test]
async fn a_change_during_pass_two_stops_and_nothing_more_runs() {
    let units = ["SELECT 1", "SELECT 2", "SELECT 3", "SELECT 4"];
    // The reader notices (the adapter's per-block stamp check): chunk 2 of pass 2 fails.
    let r = rig(fake_pg(), false).await;
    let src = script(&chunked_script(4))
        .pass2(Pass2::FailAt(2, ChunkError::Changed))
        .boxed();
    let (out, _) = r.run(r.request(src, None, Answers::new(&[]))).await;
    assert_eq!(stopped(&out).0, ScriptStopReason::FileChanged);
    assert!(r.sent(&units).len() <= 2, "{:?}", r.sent(&units));
    assert!(!r.sent(&units).contains(&"SELECT 3".to_string()));

    // The reader does not notice: the text of the third chunk changed between the passes
    // (same length); the runner's own record of pass 1 stops pass 2 before that chunk.
    let r = rig(fake_pg(), false).await;
    let first = chunked_script(4);
    let second = first.replacen("SELECT 3;", "DELETE 3;", 1);
    assert_eq!(first.len(), second.len());
    let src = script(&first).pass2(Pass2::Text(second)).boxed();
    let (out, _) = r.run(r.request(src, None, Answers::new(&[]))).await;
    assert_eq!(stopped(&out).0, ScriptStopReason::FileChanged);
    let sent = r.all_sent();
    assert!(sent.iter().all(|q| !q.starts_with("DELETE")), "{sent:?}");
    assert!(!r.sent(&units).contains(&"SELECT 3".to_string()));
    assert!(r.sent(&units).contains(&"SELECT 1".to_string()));
}

#[tokio::test]
async fn short_reads_keep_identical_chunks() {
    // R-T8-1: the reader's own reads differ between the passes (short reads of 1 to 70,000
    // bytes); the runner re-chunks the text it is handed into 64 KiB pieces cut at a character
    // boundary, so pass 2 feeds the splitter exactly pass 1's chunks and the run finishes.
    let mut text = String::new();
    for i in 0..3000 {
        text.push_str(&format!("INSERT INTO t VALUES ({i}, 'çğıöşü {i}');\n"));
    }
    assert!(text.len() > 2 * SCRIPT_CHUNK_BYTES);
    let r = rig(fake_pg(), false).await;
    let src = script(&text)
        .sizes(1, &[1, 7, 4093, 70_000, 13])
        .sizes(2, &[65_536, 3, 9_999, 1, 33_000])
        .boxed();
    let (out, _) = r.run(r.request(src, None, Answers::new(&[]))).await;
    let ScriptRunOutcome::Finished(s) = out else {
        panic!("expected the run to finish, got {out:?}");
    };
    assert_eq!((s.units_total, s.units_run), (3000, 3000));
    let inserts: Vec<String> = r
        .all_sent()
        .into_iter()
        .filter(|q| q.starts_with("INSERT"))
        .collect();
    assert_eq!(inserts.len(), 3000);
    assert_eq!(inserts[2999], "INSERT INTO t VALUES (2999, 'çğıöşü 2999')");
}

/// A file of `chunked_script(n)` in a temporary folder.
fn file_of(n: usize) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let path = dir.path().join("dump.sql");
    std::fs::write(&path, chunked_script(n)).expect("the script is written");
    (dir, path)
}

#[tokio::test]
async fn run_as_script_stops_when_file_changes_mid_run() {
    // alg/28: the file is rewritten in place after pass 2's first block: the run stops before
    // the next unit with the changed-on-disk notice.
    let (dir, path) = file_of(4);
    let target = path.clone();
    let done = Arc::new(AtomicBool::new(false));
    let once = done.clone();
    let src = FileScript::open(path).hook(move |pass, block| {
        if pass == 2 && block == 1 && !once.swap(true, Ordering::SeqCst) {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .open(&target)
                .expect("opens for writing");
            std::io::Write::write_all(&mut f, b"DELETE FROM t;\n-- rewritten in place, longer\n")
                .expect("rewrites");
            f.set_len(5 * SCRIPT_CHUNK_BYTES as u64).expect("grows");
        }
    });
    let r = rig(fake_pg(), false).await;
    let (out, _) = r
        .run(r.request(Box::new(src), None, Answers::new(&[])))
        .await;
    assert_eq!(stopped(&out).0, ScriptStopReason::FileChanged);
    assert!(done.load(Ordering::SeqCst));
    let units = ["SELECT 1", "SELECT 2", "SELECT 3", "SELECT 4"];
    assert_eq!(r.sent(&units), ["SELECT 1"]);
    assert!(r.all_sent().iter().all(|q| !q.starts_with("DELETE")));
    drop(dir);
}

#[tokio::test]
async fn rename_replace_mid_run_keeps_old_inode() {
    // alg/28: another file is renamed over the path mid-run: the stamp check stops the run, and
    // no byte of the new file is sent.
    let (dir, path) = file_of(4);
    let target = path.clone();
    let other = dir.path().join("other.sql");
    let src = FileScript::open(path).hook(move |pass, block| {
        if pass == 2 && block == 1 {
            std::fs::write(&other, "DELETE FROM t;\n").expect("the other file");
            std::fs::rename(&other, &target).expect("renamed over the script");
        }
    });
    let r = rig(fake_pg(), false).await;
    let (out, _) = r
        .run(r.request(Box::new(src), None, Answers::new(&[])))
        .await;
    assert_eq!(stopped(&out).0, ScriptStopReason::FileChanged);
    assert_eq!(
        r.sent(&["SELECT 1", "SELECT 2", "SELECT 3", "SELECT 4"]),
        ["SELECT 1"]
    );
    assert!(r.all_sent().iter().all(|q| !q.starts_with("DELETE")));
    drop(dir);
}

// ---- memory ---------------------------------------------------------------------------------

#[tokio::test]
#[ignore = "perf"]
async fn one_line_pg_script_over_64_mib_is_refused() {
    // T8 m-fr3-3: off MySQL the stream is `Exact`, which holds a line with no boundary whole;
    // the runner refuses once it holds more than 64 MiB, before anything runs. About 200 s in a
    // debug build (the `Exact` stream splits the held line again on every chunk), so it runs
    // with the perf set.
    if std::env::var("HACIVAT_PERF").as_deref() != Ok("1") {
        return;
    }
    let stmt = format!("SELECT '{}';", "x".repeat(4000));
    let n = (64 << 20) / stmt.len() + 64;
    let text = stmt.repeat(n);
    assert!(!text.contains('\n'));
    let r = rig(fake_pg(), false).await;
    let (out, progress) = r
        .run(r.request(script(&text).boxed(), None, Answers::new(&[])))
        .await;
    assert_eq!(refused(&out), (ScriptRefuseReason::NoBoundary, at(1, 1)));
    assert_eq!(progress.units_run, 0);
    assert!(r.all_sent().iter().all(|q| !q.starts_with("SELECT '")));
}

/// `unit` repeated on one line, `left` bytes in all, read 64 KiB at a time (a pass 2 is never
/// reached: its file is refused).
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

/// This process's resident set, in KiB (`ps`).
fn rss_kib() -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

/// A generated MySQL dump: `units` INSERTs of about 1 KiB, the same text in both passes.
struct Dump {
    units: usize,
    at: usize,
    pass: u8,
}

impl Dump {
    fn row(i: usize) -> String {
        format!(
            "INSERT INTO `hvm4b_dump` VALUES ({i},'{}');\n",
            "d".repeat(1000)
        )
    }
}

impl hacivat_exec::ScriptChunks for Dump {
    fn next_chunk(&mut self) -> Option<Result<String, ChunkError>> {
        if self.at >= self.units {
            return None;
        }
        let mut s = String::with_capacity(64 << 10);
        while self.at < self.units && s.len() < 60 << 10 {
            s.push_str(&Dump::row(self.at));
            self.at += 1;
        }
        Some(Ok(s))
    }

    fn reopen_verified(
        self: Box<Self>,
    ) -> futures::future::BoxFuture<'static, Result<Box<dyn hacivat_exec::ScriptChunks>, ChunkError>>
    {
        let units = self.units;
        let pass = self.pass;
        Box::pin(async move {
            Ok(Box::new(Dump {
                units,
                at: 0,
                pass: pass + 1,
            }) as Box<dyn hacivat_exec::ScriptChunks>)
        })
    }
}

#[tokio::test]
#[ignore = "perf"]
async fn mysql_dump_of_150_mb_runs_in_constant_memory() {
    if std::env::var("HACIVAT_PERF").as_deref() != Ok("1") {
        return;
    }
    let units = 150 * 1024;
    let mut r = rig(mysql(), false).await;
    // The rig's pool records every text it classifies; pass 1 classifies every unit, so this
    // run uses a pool that keeps nothing (its RSS is the runner's).
    r.runner = hacivat_exec::ScriptRunner::new(
        std::sync::Arc::new(hacivat_exec::ExecService::new(
            hacivat_results::MemoryBudget::new(1 << 30, 256 << 20),
        )),
        hacivat_exec::ClassifyPool::new(1),
    );
    let before = rss_kib();
    let (tx, mut rx) = tokio::sync::watch::channel(hacivat_exec::ScriptProgress::default());
    let req = r.request(
        Box::new(Dump {
            units,
            at: 0,
            pass: 1,
        }),
        None,
        Answers::new(&[]),
    );
    let started = Instant::now();
    // Pass 1 holds only the runner's buffers (pass 2's growth is the fake's own log of every
    // statement it ran): its RSS is sampled when pass 2 starts.
    let sample = async {
        let _ = rx.wait_for(|p| p.phase != ScriptPhase::Checking).await;
        rss_kib()
    };
    let (out, after_pass1) = tokio::join!(hacivat_exec::run_file(&r.runner, req, tx), sample);
    let ScriptRunOutcome::Finished(s) = out else {
        panic!("expected the run to finish, got {out:?}");
    };
    let growth_mib = after_pass1.saturating_sub(before) / 1024;
    println!(
        "HV-FACT t12 target=fake-mysql bytes={} units={} pass1_rss_growth_mib={growth_mib} peak_held={} secs={}",
        units * Dump::row(0).len(),
        s.units_run,
        s.peak_held,
        started.elapsed().as_secs()
    );
    assert_eq!(s.units_run, units as u64);
    assert!(s.peak_held <= 256 << 10, "held {}", s.peak_held);
    assert!(growth_mib < 64, "RSS grew {growth_mib} MiB in pass 1");
}

// ---- fix round 1: lexing modes (I1) --------------------------------------------------------

/// A split that differs across `standard_conforming_strings` (review P3).
const SCS_SPLIT: &str = "SELECT 'a\\';\nDELETE FROM t WHERE id = 1;\n-- ';\n";

#[tokio::test]
async fn unknown_modes_must_read_alike() {
    // P3: the session's modes are unknown, and the file reads differently under `on` and
    // `off`: nothing is sent (M3's rule).
    let r = rig(fake_pg(), false).await;
    let (out, _) = r
        .run(r.request(script(SCS_SPLIT).boxed(), None, Answers::new(&[])))
        .await;
    assert_eq!(refused(&out), (ScriptRefuseReason::ModesDiffer, None));
    assert!(
        r.all_sent().iter().all(|q| !q.contains("DELETE")),
        "{:?}",
        r.all_sent()
    );
    assert!(r.all_sent().iter().all(|q| !q.starts_with("SELECT 'a")));
}

#[tokio::test]
async fn known_session_modes_split_the_file() {
    // The session reads `off`: the file is split in its modes, so the backslash escapes the
    // quote and the DELETE is inside one string, never a statement of its own.
    let r = rig(pg_context(&["off", "off", "off", "off"]), false).await;
    let (out, _) = r
        .run(r.request(
            script(SCS_SPLIT).boxed(),
            None,
            Answers::new(&[ConfirmAnswer::Execute]),
        ))
        .await;
    assert!(matches!(out, ScriptRunOutcome::Finished(_)), "{out:?}");
    assert_eq!(
        r.sent(&["DELETE FROM t WHERE id = 1"]),
        Vec::<String>::new()
    );
    let whole = "SELECT 'a\\';\nDELETE FROM t WHERE id = 1;\n-- '";
    assert_eq!(r.sent(&[whole]), [whole]);
}

#[tokio::test]
async fn a_tab_that_read_other_modes_is_refused() {
    // M3's rule: the tab read `on`, the session now reads `off`.
    let r = rig(pg_context(&["off"]), false).await;
    let mut req = r.request(script("SELECT 1;\n").boxed(), None, Answers::new(&[]));
    req.modes_known = true;
    assert!(req.dialect.modes.standard_conforming_strings);
    let (out, _) = r.run(req).await;
    assert_eq!(refused(&out), (ScriptRefuseReason::ModesDiffer, None));
    nothing_of(&r, &["SELECT 1"]);
}

#[tokio::test]
async fn a_mode_change_mid_run_stops_before_the_next_unit() {
    // The file was split under `on`; the SET turns it off, so the rest no longer reads as it
    // was split: the run stops before statement 3.
    let r = rig(pg_context(&["on", "off", "off", "off"]), false).await;
    let units = [
        "SELECT 1",
        "SET standard_conforming_strings = off",
        "SELECT 2",
    ];
    let src = script("SELECT 1;\nSET standard_conforming_strings = off;\nSELECT 2;\n").boxed();
    let (out, _) = r.run(r.request(src, None, Answers::new(&[]))).await;
    assert_eq!(stopped(&out), (ScriptStopReason::ContextChanged, at(3, 3)));
    assert_eq!(r.sent(&units), units[..2]);
}

// ---- fix round 1: the live environment tag (I2) -------------------------------------------

#[tokio::test]
async fn a_retag_to_production_gates_the_next_unit_as_production() {
    // Off production `DROP TABLE` runs without a question; retagged to production while the
    // second chunk is read, the next unit asks with a typed name (and Cancel stops the run).
    let text = format!(
        "SELECT 1;\n{}DROP TABLE t;\n",
        padding(2 * SCRIPT_CHUNK_BYTES)
    );
    let r = rig(fake_pg(), false).await;
    let env = r.env.clone();
    let src = script(&text).hook(move |pass, read| {
        if pass == 2 && read > SCRIPT_CHUNK_BYTES {
            env.send_replace(Some(EnvTag::Production));
        }
    });
    let port = Answers::new(&[ConfirmAnswer::Cancel]);
    let (out, _) = r
        .run(r.request(src.boxed(), Some(EnvTag::Dev), port.clone()))
        .await;
    assert_eq!(stopped(&out).0, ScriptStopReason::ConfirmCancelled);
    let asked = port.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(
        asked[0].production && asked[0].type_to_confirm.is_some(),
        "{asked:?}"
    );
    assert_eq!(r.sent(&["SELECT 1", "DROP TABLE t"]), ["SELECT 1"]);
    // Without the retag, the same file runs to the end on dev.
    let r = rig(fake_pg(), false).await;
    let port = Answers::new(&[]);
    let (out, _) = r
        .run(r.request(script(&text).boxed(), Some(EnvTag::Dev), port.clone()))
        .await;
    assert!(matches!(out, ScriptRunOutcome::Finished(_)), "{out:?}");
    assert!(port.asked_units().is_empty());
    assert_eq!(
        r.sent(&["SELECT 1", "DROP TABLE t"]),
        ["SELECT 1", "DROP TABLE t"]
    );
}

#[tokio::test]
async fn a_retag_between_the_gate_and_the_send_stops() {
    // M3's B-M4/A-M4 rule: the tag is read again right before the send. A retag while the
    // dialog is open stops the run, and the confirmed statement is not sent.
    let r = rig(fake_pg(), false).await;
    let env = r.env.clone();
    let port = Answers::with_hook(&[ConfirmAnswer::Execute], move || {
        env.send_replace(Some(EnvTag::Production));
    });
    let src = script("SELECT 1;\nDELETE FROM t;\n").boxed();
    let (out, _) = r.run(r.request(src, Some(EnvTag::Dev), port)).await;
    assert_eq!(
        stopped(&out),
        (ScriptStopReason::ConnectionChanged, at(2, 2))
    );
    assert_eq!(r.sent(&["SELECT 1", "DELETE FROM t"]), ["SELECT 1"]);
}

// ---- fix round 1: a lost or reset connection (I3) ------------------------------------------

#[tokio::test]
async fn a_reset_connection_stops_the_run() {
    // The connection is lost between units (a ping fails while the second chunk is read) and
    // the session recovers in place: the new connection lost what the file set, so nothing
    // more is sent, and the log says where the script stopped.
    let reset = hacivat_driver::RawError {
        kind: hacivat_driver::RawErrorKind::Io(std::io::ErrorKind::ConnectionReset),
        server: None,
    };
    let r = managed_rig(fake_pg().pings(vec![Err(reset)])).await;
    let session = r.session.clone();
    let text = format!("SELECT 1;\n{}SELECT 2;\n", padding(2 * SCRIPT_CHUNK_BYTES));
    let src = script(&text).hook(move |pass, read| {
        if pass == 2 && read > SCRIPT_CHUNK_BYTES && read <= 2 * SCRIPT_CHUNK_BYTES {
            drop(session.submit(hacivat_session::session::Request::Ping));
        }
    });
    let (out, _) = r.run(r.request(src.boxed(), None, Answers::new(&[]))).await;
    assert_eq!(stopped(&out).0, ScriptStopReason::Reconnected);
    assert_eq!(r.sent(&["SELECT 1", "SELECT 2"]), ["SELECT 1"]);
    let ids: Vec<&str> = r.log.lines().iter().map(|l| l.msg.fluent_id()).collect();
    assert!(ids.contains(&"exec-script-reconnected"), "{ids:?}");
}

// ---- fix round 1: the item cap off MySQL (I4) and the log --------------------------------

#[tokio::test]
async fn one_line_of_tiny_statements_is_refused_by_the_item_cap() {
    // Review P7: `Exact` holds about 50-80 bytes per byte of tiny statements on one line. The
    // runner refuses once the held text holds more than 65,536 items, long before 64 MiB. The
    // 65 MiB line is generated (no copy held by the test), so the test's peak RSS is the
    // runner's (fix round 1 measures it with `/usr/bin/time -l`).
    let r = rig(fake_pg(), false).await;
    let line = Box::new(Repeat {
        unit: "SELECT 1;",
        left: 65 << 20,
    });
    let (out, progress) = r.run(r.request(line, None, Answers::new(&[]))).await;
    assert_eq!(refused(&out), (ScriptRefuseReason::NoBoundary, at(1, 1)));
    assert_eq!(progress.units_run, 0);
    assert!(
        progress.bytes < 4 << 20,
        "read {} bytes before the refusal",
        progress.bytes
    );
    nothing_of(&r, &["SELECT 1"]);
}

#[tokio::test]
async fn log_lines_carry_the_units_place_in_the_file() {
    // Review: each unit is its own run, so the service numbers it 1; the console's log says
    // the statement's number in the file, and one summary line covers the run.
    let r = rig(fake_pg().queries(vec![failing("UPDATE nope")]), false).await;
    let text = "UPDATE a SET x = 1 WHERE id = 1;\nUPDATE b SET x = 1 WHERE id = 1;\nUPDATE nope SET x = 1 WHERE id = 1;\n";
    let (out, _) = r
        .run(r.request(script(text).boxed(), None, Answers::new(&[])))
        .await;
    assert!(matches!(out, ScriptRunOutcome::Failed(_)), "{out:?}");
    let msgs: Vec<LogMsg> = r.log.lines().iter().map(|l| l.msg.clone()).collect();
    assert!(
        msgs.iter().any(|m| matches!(m, LogMsg::Error { n: 3, .. })),
        "{msgs:?}"
    );
    assert!(!msgs.iter().any(|m| matches!(m, LogMsg::Error { n: 1, .. })));
    let summaries: Vec<&LogMsg> = msgs
        .iter()
        .filter(|m| matches!(m, LogMsg::Summary(_)))
        .collect();
    assert_eq!(summaries.len(), 1, "{msgs:?}");
    let LogMsg::Summary(s) = summaries[0] else {
        unreachable!()
    };
    assert_eq!((s.executed, s.failed, s.total), (2, 1, 3));
}
