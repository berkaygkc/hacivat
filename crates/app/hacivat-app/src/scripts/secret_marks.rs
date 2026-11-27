//! The secret marks in a script tab's gutter (HV-SCRIPT-10, R-M4b-17; NOTES surface 8):
//! password- and token-like literals marked as you type (debounced, off the UI thread), with
//! hover text that names the kind and never the value.
//!
//! ## m4b-t01b-seams: nothing yet (Task 17).
//!
//! ## m4b-t17-secrets-ui
//! - **The scan** ([`scan_text`]) is `scan_secrets(text, &secret_spans(text))` (R-M4b-28: the
//!   history redactor's spans, mapped one to one). It runs on the core runtime's blocking pool,
//!   never on the UI thread ([`scan_off_ui`]), [`DEBOUNCE`] after the last edit of a script tab;
//!   a newer edit replaces the waiting debounce, and an answer for an older version than the
//!   editor's is dropped (latest wins by document version). Fix round 1 (m2): at most one scan
//!   runs per tab (an edit during it waits for it, then the newer text is scanned), the text is
//!   copied out of the rope inside the blocking job, and the recovery copy reuses the tab's scan
//!   of the same version. The save's own copy of the text (`save.rs`, T14's) stays on the UI
//!   thread: it is bounded by the size tiers (a T2 preview, above 20 MB, is never saved).
//! - **Fail closed.** A scan that cannot answer (no core runtime, or it panicked) answers one
//!   `Unscanned` finding over the whole text, as the scanner does when its table fails to load:
//!   the gutter says "couldn't be checked" and a save asks.
//! - **The gutter.** Each finding is a `key-round` line mark (`hacivat_ui_editor::LineMark`) on
//!   the line its literal starts on, in `text-2`. Its tooltip and accessible name name the kind
//!   ("Looks like a password") and how to silence the line; never the value.
//! - **Fingerprints** ([`Scan::prints`]): "Save anyway" remembers each finding as
//!   `xxh3(kind ‖ its line's text without the literal ‖ the literal)`, in memory, for the tab,
//!   until quit (the plan's line-text rule and `alg/28`'s value rule together: a changed value
//!   or a changed line asks again; a line moved up or down does not). An `Unscanned` finding is
//!   never remembered.
//! - Only script tabs scan: consoles stay in Hacivat's own folder and never mark or ask (Q7).

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use hacivat_l10n::ArgValue;
use hacivat_scripts::secrets::{SecretFinding, SecretKind, SpanKind, scan_secrets};
use hacivat_ui::icon::IconName;
use hacivat_ui::text::TextFn;
use hacivat_ui_core::gpui_surface::{App, Context, Task};
use hacivat_ui_editor::LineMark;

use crate::scripts::tabs::{ScriptDoc, msg};

/// How long after the last edit a script tab's text is scanned again.
pub const DEBOUNCE: Duration = Duration::from_millis(300);

/// The marker that silences a line (never translated): the scanner's own (fix round 1, m4).
pub use hacivat_scripts::secrets::ALLOW_MARKER;

/// One scan's answer, for the text at `version`.
#[derive(Clone)]
pub struct Scan {
    pub version: u64,
    pub findings: Arc<[SecretFinding]>,
    /// Each finding's fingerprint (`None`: never remembered, an `Unscanned` finding).
    prints: Arc<[Option<u64>]>,
}

// ## m4b-t17-secrets-ui fix round 1 (m1): counts only. A print is a fast hash over the literal
// and its line, so it never reaches a `Debug` (a log or a panic message).
impl std::fmt::Debug for Scan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scan")
            .field("version", &self.version)
            .field("findings", &self.findings.len())
            .field("unscanned", &self.unscanned())
            .finish_non_exhaustive()
    }
}

impl Scan {
    fn of(version: u64, text: &str, findings: Vec<SecretFinding>) -> Scan {
        let prints = findings.iter().map(|f| fingerprint(text, f)).collect();
        Scan {
            version,
            findings: findings.into(),
            prints,
        }
    }

    /// Whether a save of this text asks first: a finding that "Save anyway" did not remember
    /// for this tab (an `Unscanned` one always asks).
    pub fn asks(&self, remembered: &std::collections::HashSet<u64>) -> bool {
        self.prints
            .iter()
            .any(|p| p.is_none_or(|p| !remembered.contains(&p)))
    }

    /// The fingerprints "Save anyway" remembers.
    pub fn prints(&self) -> impl Iterator<Item = u64> + '_ {
        self.prints.iter().flatten().copied()
    }

    /// Whether the text could not be scanned.
    pub fn unscanned(&self) -> bool {
        self.findings
            .iter()
            .any(|f| f.kind == SecretKind::Unscanned)
    }
}

/// A script tab's marks: the latest scan, the debounce waiting, and the scan running.
///
/// ## m4b-t17-secrets-ui fix round 1 (m2): at most one scan runs per tab. An edit while one
/// runs starts no other; when the running scan lands for an older version than the editor's,
/// its answer is dropped and the text is scanned again after the debounce. Blocking scans of a
/// large text never stack up.
#[derive(Default)]
pub struct Marks {
    latest: Option<Scan>,
    /// The debounce, waiting.
    timer: Option<Task<()>>,
    /// The scan, running (its blocking job is not cancellable, so it is never dropped).
    running: Option<Task<()>>,
}

impl std::fmt::Debug for Marks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Marks")
            .field("latest", &self.latest)
            .field("waiting", &self.timer.is_some())
            .field("scanning", &self.running.is_some())
            .finish()
    }
}

impl Marks {
    /// The latest scan's answer (for the version it names).
    pub fn latest(&self) -> Option<&Scan> {
        self.latest.as_ref()
    }

    /// The latest scan's answer when it is for `version`.
    pub fn at(&self, version: u64) -> Option<&Scan> {
        self.latest.as_ref().filter(|s| s.version == version)
    }

    /// A scan waits or runs.
    pub fn pending(&self) -> bool {
        self.timer.is_some() || self.running.is_some()
    }

    /// The debounce waits (a scan starts when it ends).
    pub fn waiting(&self) -> bool {
        self.timer.is_some()
    }

    /// A scan runs now (on the core runtime).
    pub fn scanning(&self) -> bool {
        self.running.is_some()
    }
}

/// What scans a text (the scanner, or a test's stand-in).
pub type Scanner = Arc<dyn Fn(&str) -> Vec<SecretFinding> + Send + Sync>;

/// A test's stand-in for the scanner (a slow one, one that panics, one that records its thread).
#[derive(Clone)]
struct ScannerOverride(Scanner);

impl hacivat_ui_core::gpui_surface::Global for ScannerOverride {}

/// Replaces the scanner for the tests (`None`: the real one).
#[cfg(feature = "test-support")]
pub fn set_scanner_for_tests(cx: &mut App, scanner: Option<Scanner>) {
    match scanner {
        Some(s) => cx.set_global(ScannerOverride(s)),
        None => {
            cx.remove_global::<ScannerOverride>();
        }
    }
}

fn scanner(cx: &App) -> Scanner {
    match cx.try_global::<ScannerOverride>() {
        Some(o) => o.0.clone(),
        None => Arc::new(scan_text),
    }
}

/// The secret-like literals of `text`: the scanner over the history redactor's spans (R-M4b-28).
/// Runs on any thread; the app calls it off the UI thread.
pub fn scan_text(text: &str) -> Vec<SecretFinding> {
    let spans: Vec<(Range<usize>, SpanKind)> = hacivat_exec::history::secret_spans(text)
        .into_iter()
        .map(|(r, k)| {
            let kind = match k {
                hacivat_exec::history::SecretSpanKind::Value => SpanKind::Password,
                hacivat_exec::history::SecretSpanKind::ConnectionValue => {
                    SpanKind::ConnectionString
                }
                hacivat_exec::history::SecretSpanKind::Armed => SpanKind::Other,
            };
            (r, kind)
        })
        .collect();
    scan_secrets(text, &spans)
}

/// Whether `text` looks like it holds a secret (a recovery copy's metadata, R-T5-4); a scan
/// that panics counts as yes (fail closed). Runs on any thread; never on the UI thread.
pub(crate) fn suspected(text: &str) -> bool {
    std::panic::catch_unwind(|| scan_text(text)).map_or(true, |f| !f.is_empty())
}

/// The one finding of a text that could not be scanned (fail closed).
fn unscanned(text: &str) -> Vec<SecretFinding> {
    vec![SecretFinding {
        range: 0..text.len(),
        line: 0,
        kind: SecretKind::Unscanned,
    }]
}

/// Scans `text` (the editor at `version`) on the core runtime's blocking pool; the answer is
/// `Unscanned` when the scan cannot run or panics.
pub(crate) fn scan_off_ui(
    version: u64,
    text: String,
    cx: &App,
) -> impl std::future::Future<Output = Scan> + use<> {
    scan_with(version, move || text, cx)
}

/// [`scan_off_ui`] whose text is made by `text` in the blocking job too (fix round 1, m2: a
/// tab's whole text is copied out of its rope there, never on the UI thread).
fn scan_with<F: FnOnce() -> String + Send + 'static>(
    version: u64,
    text: F,
    cx: &App,
) -> impl std::future::Future<Output = Scan> + use<F> {
    let scan = scanner(cx);
    let rt = cx
        .try_global::<crate::connections::ConnectionsModel>()
        .map(|m| m.runtime());
    let (tx, rx) = tokio::sync::oneshot::channel();
    if let Some(rt) = rt {
        rt.spawn(async move {
            let ran = tokio::task::spawn_blocking(move || {
                let text = text();
                let findings = scan(&text);
                Scan::of(version, &text, findings)
            })
            .await;
            let _ = tx.send(ran.ok());
        });
    }
    async move {
        match rx.await.ok().flatten() {
            Some(scan) => scan,
            None => {
                tracing::warn!("scripts: the secret scan did not answer; the text is unscanned");
                // The text went with its task: the finding's range is empty (the first line).
                Scan {
                    version,
                    findings: unscanned("").into(),
                    prints: Arc::from([None]),
                }
            }
        }
    }
}

/// Scans `doc`'s editor text `after` from now. A newer call replaces a waiting one; while a scan
/// runs, nothing starts: its landing scans a newer text again (fix round 1, m2: one scan in
/// flight per tab).
pub(crate) fn schedule(doc: &mut ScriptDoc, after: Duration, cx: &mut Context<ScriptDoc>) {
    if doc.editor().is_none() {
        return;
    }
    if doc.secrets.running.is_some() {
        doc.secrets.timer = None;
        return;
    }
    let timer = cx.background_executor().timer(after);
    doc.secrets.timer = Some(cx.spawn(async move |this, cx| {
        timer.await;
        let _ = this.update(cx, start);
    }));
}

/// The debounce passed: the scan of the editor's text now starts.
fn start(doc: &mut ScriptDoc, cx: &mut Context<ScriptDoc>) {
    doc.secrets.timer = None;
    let Some(editor) = doc.editor().cloned() else {
        return;
    };
    let snap = editor.read(cx).snapshot();
    let version = snap.version;
    let scan = scan_with(version, move || snap.buffer.to_string(), cx);
    doc.secrets.running = Some(cx.spawn(async move |this, cx| {
        let scan = scan.await;
        let _ = this.update(cx, |d, cx| landed(d, scan, cx));
    }));
}

/// A scan landed: kept, and drawn, when it is for the editor's current text; a newer text is
/// scanned again after the debounce.
fn landed(doc: &mut ScriptDoc, scan: Scan, cx: &mut Context<ScriptDoc>) {
    doc.secrets.running = None;
    let Some(editor) = doc.editor().cloned() else {
        return;
    };
    if editor.read(cx).snapshot().version != scan.version {
        // An older text's answer is dropped (latest wins by version); the text is scanned again.
        schedule(doc, DEBOUNCE, cx);
        return;
    }
    let marks = line_marks(&scan);
    let version = scan.version;
    doc.secrets.latest = Some(scan);
    editor.update(cx, |e, cx| e.set_line_marks(version, marks, cx));
    cx.notify();
}

/// The gutter marks of `scan`: one per finding, on its literal's first line.
fn line_marks(scan: &Scan) -> Vec<LineMark> {
    scan.findings
        .iter()
        .map(|f| {
            let range = if f.kind == SecretKind::Unscanned {
                0..0
            } else {
                f.range.clone()
            };
            LineMark {
                range,
                icon: IconName::KeyRound,
                label: kind_label(f.kind),
                detail: vec![hint(f.kind)],
            }
        })
        .collect()
}

/// The kind's words ("Looks like a password"), never the value.
pub(crate) fn kind_label(kind: SecretKind) -> TextFn {
    if kind == SecretKind::Unscanned {
        return Rc::new(|cx: &App| msg(cx, "hv-scripts-secret-unscanned", Vec::new()));
    }
    let selector = kind_selector(kind);
    Rc::new(move |cx: &App| {
        msg(
            cx,
            "hv-scripts-secret-gutter",
            vec![("kind", ArgValue::Text(selector.to_string()))],
        )
    })
}

/// The selector of `hv-scripts-secret-gutter` for `kind`.
fn kind_selector(kind: SecretKind) -> &'static str {
    match kind {
        SecretKind::Password => "password",
        SecretKind::ConnectionString => "url",
        SecretKind::AccessToken | SecretKind::TestKey | SecretKind::Jwt => "token",
        SecretKind::PrivateKey => "key",
        SecretKind::Unscanned => "password",
    }
}

/// The tooltip's second line: how to silence the line (an unscanned text: that a save asks).
fn hint(kind: SecretKind) -> TextFn {
    if kind == SecretKind::Unscanned {
        return Rc::new(|cx: &App| msg(cx, "hv-scripts-secret-unscanned-hint", Vec::new()));
    }
    // ## m4b-t21-shots (T17 review m5): the marker is an argument, never translated text.
    Rc::new(|cx: &App| {
        msg(
            cx,
            "hv-scripts-secret-gutter-hint",
            vec![("marker", ArgValue::Text(ALLOW_MARKER.to_string()))],
        )
    })
}

/// A finding's fingerprint (module docs); `None` for `Unscanned`. Never stored outside the
/// tab's memory, never shown.
fn fingerprint(text: &str, f: &SecretFinding) -> Option<u64> {
    use std::hash::Hasher as _;
    if f.kind == SecretKind::Unscanned {
        return None;
    }
    let (start, end) = (f.range.start.min(text.len()), f.range.end.min(text.len()));
    let (Some(literal), Some(_)) = (text.get(start..end), text.get(..start)) else {
        return None;
    };
    let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[end..].find('\n').map_or(text.len(), |i| end + i);
    let mut h = twox_hash::XxHash3_64::default();
    h.write_u8(f.kind as u8);
    h.write(&text.as_bytes()[line_start..start]);
    h.write_u8(0);
    h.write(&text.as_bytes()[end..line_end]);
    h.write_u8(0);
    h.write(literal.as_bytes());
    Some(h.finish())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn scan(text: &str) -> Scan {
        Scan::of(1, text, scan_text(text))
    }

    #[test]
    fn a_moved_line_is_remembered_and_a_changed_one_asks() {
        let a = scan("CREATE USER app PASSWORD 's3cret';\n");
        let remembered: HashSet<u64> = a.prints().collect();
        assert!(a.asks(&HashSet::new()));
        assert!(!a.asks(&remembered));
        // Moved down two lines: the same finding.
        assert!(!scan("\n\nCREATE USER app PASSWORD 's3cret';\n").asks(&remembered));
        // Another value, or another line around it: a new finding.
        assert!(scan("CREATE USER app PASSWORD 'other';\n").asks(&remembered));
        assert!(scan("CREATE USER web PASSWORD 's3cret';\n").asks(&remembered));
    }

    #[test]
    fn an_unscanned_text_always_asks() {
        let s = Scan::of(1, "x", unscanned("x"));
        let remembered: HashSet<u64> = s.prints().collect();
        assert!(remembered.is_empty());
        assert!(s.asks(&remembered));
        assert!(s.unscanned());
    }

    // ## m4b-t17-secrets-ui fix round 1 (m1)
    #[test]
    fn a_scan_debug_holds_counts_only() {
        let s = scan("CREATE USER app PASSWORD 's3cret';\n");
        let dbg = format!("{s:?}");
        let print = s.prints().next().expect("a print").to_string();
        assert!(!dbg.contains(&print), "{dbg}");
        assert!(!dbg.contains("s3cret"), "{dbg}");
        assert!(dbg.contains("findings: 1"), "{dbg}");
    }

    #[test]
    fn nothing_found_never_asks() {
        assert!(!scan("SELECT password_hash FROM users;\n").asks(&HashSet::new()));
    }
}
