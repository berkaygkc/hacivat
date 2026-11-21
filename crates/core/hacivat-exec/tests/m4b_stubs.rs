// ## m4b-t01a-seams
//! M4b Task 1a: the run service's script-workspace seams fail closed (F31) until their tasks:
//! "Run as script" refuses, pulls no chunk and sends nothing (Task 12).
//! ## m4b-t06-secrets (named exception, fix round 1 m9): the secret spans are Task 6's now;
//! their seam test pins the filled shape (no span in a clean text, `Armed` vs `Value`).

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{fake_pg, service_over};
use hacivat_exec::history::{SecretSpanKind, secret_spans};
use hacivat_exec::{
    ChunkError, ErrorPolicy, ScriptChunks, ScriptRefuseReason, ScriptRunOutcome, ScriptRunRequest,
    ScriptRunner, SessionLog, run_file,
};

// ## m4b-t06-secrets (named exception): `secret_spans` is Task 6's now; the seam keeps its
// shape: no span in a clean text, `Armed` for an armed statement's string (C9), `Value` for a
// value position.
#[test]
fn exec_secret_spans_tell_value_from_armed() {
    assert!(secret_spans("SELECT 1").is_empty());
    let t = "SELECT password_hash FROM users WHERE password_hash = 'x'";
    assert_eq!(secret_spans(t), [(54..57, SecretSpanKind::Armed)]);
    assert_eq!(
        secret_spans("ALTER ROLE app PASSWORD 'x'"),
        [(24..27, SecretSpanKind::Value)]
    );
}
// ## m4b-t06-secrets

/// A source that counts how often it is pulled.
struct Counting {
    pulled: Arc<AtomicUsize>,
}

impl ScriptChunks for Counting {
    fn next_chunk(&mut self) -> Option<Result<String, ChunkError>> {
        let n = self.pulled.fetch_add(1, Ordering::SeqCst);
        (n == 0).then(|| Ok("DELETE FROM t;\nDROP TABLE t;\n".to_string()))
    }
}

#[tokio::test]
async fn run_file_stub_refuses_and_sends_nothing() {
    let (bench, session, fake) = service_over(fake_pg()).await;
    let before = fake.executed();
    let pulled = Arc::new(AtomicUsize::new(0));
    let runner = ScriptRunner::new();
    let req = || ScriptRunRequest {
        console: 7,
        engine: bench.engine,
        log: SessionLog::new(),
        policy: ErrorPolicy::StopOnError,
    };
    let source = || {
        Box::new(Counting {
            pulled: pulled.clone(),
        }) as Box<dyn ScriptChunks>
    };
    assert_eq!(
        run_file(&runner, &session, source(), req()).await,
        ScriptRunOutcome::Refused(ScriptRefuseReason::NotImplemented)
    );
    assert_eq!(
        runner.run(&session, source(), req()).await,
        ScriptRunOutcome::Refused(ScriptRefuseReason::NotImplemented)
    );
    assert_eq!(pulled.load(Ordering::SeqCst), 0, "a chunk was read");
    assert_eq!(fake.executed(), before, "a statement was sent");
    assert!(bench.log.lines().is_empty());
}
