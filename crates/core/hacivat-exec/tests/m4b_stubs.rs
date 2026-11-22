// ## m4b-t01a-seams
//! M4b Task 1a: the run service's script-workspace seams fail closed (F31) until their tasks:
//! "Run as script" refused, pulled no chunk and sent nothing until Task 12 filled it.
//! ## m4b-t06-secrets (named exception, fix round 1 m9): the secret spans are Task 6's now;
//! their seam test pins the filled shape (no span in a clean text, `Armed` vs `Value`).

mod common;

use hacivat_exec::history::{SecretSpanKind, secret_spans};

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

// ## m4b-t12-script-run (named exception): `run_file` is Task 12's now; its stub test went
// with the stub, and `tests/script_run.rs` covers what it does.
