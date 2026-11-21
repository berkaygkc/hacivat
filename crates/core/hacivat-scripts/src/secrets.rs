//! Secret-literal detection for the save warning (HV-SCRIPT-10; R-M4b-17, -28; alg/28 C9).
//!
//! ## m4b-t01a-seams
//! Task 1a: the types and a fail-closed [`scan_secrets`] stub that answers one finding over the
//! whole text, so a save asks instead of silently passing. Task 6 writes the scanner.
//!
//! This crate never depends on `hacivat-exec` (R-M4b-28): the app passes the history
//! redactor's spans, `hacivat_exec::history::secret_spans(text)`, mapping each
//! `hacivat_exec::history::SecretSpanKind` onto [`SpanKind`] one to one
//! (`Value` → `Password`, `ConnectionValue` → `ConnectionString`, `Armed` → `Other`).

use std::ops::Range;

/// The kind of a history-redaction span, mirrored from `hacivat_exec::history::SecretSpanKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpanKind {
    /// A value right after a secret keyword (`PASSWORD 'x'`, `IDENTIFIED BY x`): reported.
    Password,
    /// A connection-string or URI password value (`password=…`, `user:pass@`): reported.
    ConnectionString,
    /// Another string of a statement a secret word armed (`WHERE password_hash = 'x'`): masked
    /// in history, never reported (C9).
    Other,
}

/// What a finding is. Never the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SecretKind {
    Password,
    ConnectionString,
    /// A vendor token format (AWS, GitHub, Slack, Stripe live, Google API key).
    AccessToken,
    /// A test-mode key (`sk_test_`/`rk_test_`): lower severity.
    TestKey,
    PrivateKey,
    Jwt,
    /// Task 1a's stub: the text was not scanned.
    Unscanned,
}

/// One finding: where and what, never the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretFinding {
    /// The byte range of the literal in the scanned text.
    pub range: Range<usize>,
    /// Zero-based line of the range's start.
    pub line: u32,
    pub kind: SecretKind,
}

/// The secret-like literals of `text`, given the history redactor's spans for it.
///
/// Task 1a stub (fail closed): one [`SecretKind::Unscanned`] finding over the whole text.
pub fn scan_secrets(text: &str, history_spans: &[(Range<usize>, SpanKind)]) -> Vec<SecretFinding> {
    let _ = history_spans;
    vec![SecretFinding {
        range: 0..text.len(),
        line: 0,
        kind: SecretKind::Unscanned,
    }]
}
// ## m4b-t01a-seams
