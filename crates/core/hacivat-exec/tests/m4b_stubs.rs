// ## m4b-t01a-seams
//! M4b Task 1a: the run service's script-workspace seams fail closed (F31) until their tasks:
//! the secret spans cover the whole text (a caller warns, never passes; Task 6), and "Run as
//! script" refuses, pulls no chunk and sends nothing (Task 12).

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{fake_pg, service_over};
use hacivat_exec::history::{SecretSpanKind, secret_spans};
use hacivat_exec::{
    ChunkError, ErrorPolicy, ScriptChunks, ScriptRefuseReason, ScriptRunOutcome, ScriptRunRequest,
    ScriptRunner, SessionLog, run_file,
};

#[test]
fn exec_secret_spans_stub_covers_the_whole_text() {
    assert_eq!(secret_spans("SELECT 1").len(), 1);
    assert_eq!(secret_spans("SELECT 1"), [(0..8, SecretSpanKind::Value)]);
    // A text with no secret at all is still one value span: the stub never says "clean".
    let t = "SELECT password_hash FROM users WHERE password_hash = 'x'";
    assert_eq!(secret_spans(t), [(0..t.len(), SecretSpanKind::Value)]);
}

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
