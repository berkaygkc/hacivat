//! The texts of the connect surfaces, built once for the screen and once for the English copies
//! (R26): "Copy report" and "Copy details (English)" are rendered here from the `en` bundle,
//! without isolation marks, whatever the UI language. Core never formats English (§3.4).
//!
//! Every line comes from Fluent: the step names, the diagnostics, the warnings, the error's
//! heading and explanation (`error-conn-<class>-title`/`-body` with the error's arguments, never
//! the server's text), and each copied line (`hv-conn-diag-row`). The server's own message is
//! data and is copied verbatim, after `hacivat_secrets::scrub_message` (a message that is a
//! connection string loses its password). Nothing here ever holds a password.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use hacivat_driver::ServerInfo;
use hacivat_l10n::{
    ArgValue, Catalogs, Count, ExactDecimal, FormatInputs, IntoFluentArgs, L10n, MsgRef,
    NegotiationCtx, NoArgs, PrefSource, Sink, Text, build_runtime, ids, negotiate,
};
use hacivat_net::{TlsSummary, TlsVerified};
use hacivat_proto::ErrClass;
use hacivat_secrets::AuthProviderId;
use hacivat_session::pipeline::{ConnectError, StepOutcome, StepTrace, TrailStep};
use hacivat_session::{TestReport, TestWarning};
use hacivat_ui::step_trail::{TrailA11y, TrailItem, TrailState};
use hacivat_ui_core::gpui_surface::{App, Global, SharedString};

use crate::l10n_global::L10nGlobal;

/// Renders messages for one sink: the UI (the installed chain, isolated placeables) or an
/// English copy (the `en` bundle, no isolation marks).
pub(crate) struct Fmt {
    l10n: Option<Arc<L10n>>,
    sink: Sink,
}

/// The `en`-only localization the copies use, built once from the app's catalogs.
struct EnglishL10n {
    catalogs: Arc<Catalogs>,
    l10n: Arc<L10n>,
}

impl Global for EnglishL10n {}

/// Arguments built at run time (an error class's messages are found by their stem).
struct DynArgs(Vec<(&'static str, ArgValue)>);

impl IntoFluentArgs for DynArgs {
    fn into_args(self) -> Vec<(&'static str, ArgValue)> {
        self.0
    }
}

impl Fmt {
    /// The UI's texts (the installed runtime); message IDs when none is installed.
    pub(crate) fn ui(cx: &App) -> Fmt {
        Fmt {
            l10n: cx.try_global::<L10nGlobal>().map(|g| g.l10n.clone()),
            sink: Sink::Ui,
        }
    }

    /// English texts for a copy (R26): the `en` bundle of the app's catalogs, no isolation
    /// marks. Built on first use and again only when the catalogs change.
    pub(crate) fn english(cx: &mut App) -> Fmt {
        let Some(catalogs) = cx.try_global::<L10nGlobal>().map(|g| g.catalogs.clone()) else {
            return Fmt {
                l10n: None,
                sink: Sink::Machine,
            };
        };
        let cached = cx
            .try_global::<EnglishL10n>()
            .filter(|e| Arc::ptr_eq(&e.catalogs, &catalogs))
            .map(|e| e.l10n.clone());
        let l10n = cached.unwrap_or_else(|| {
            let ctx = NegotiationCtx {
                sources: vec![PrefSource::Setting("en".to_string())],
                ..Default::default()
            };
            let neg = negotiate(&ctx, catalogs.locales());
            let l10n = Arc::new(L10n::new(build_runtime(
                &catalogs,
                &neg,
                &FormatInputs::new(&neg, "UTC"),
                1,
            )));
            cx.set_global(EnglishL10n {
                catalogs,
                l10n: l10n.clone(),
            });
            l10n
        });
        Fmt {
            l10n: Some(l10n),
            sink: Sink::Machine,
        }
    }

    pub(crate) fn t<A: IntoFluentArgs>(&self, msg: MsgRef<A>, args: A) -> String {
        match &self.l10n {
            Some(l10n) => l10n.t_rendered(msg, args, self.sink).text,
            None => msg.id.to_string(),
        }
    }

    pub(crate) fn s<A: IntoFluentArgs>(&self, msg: MsgRef<A>, args: A) -> SharedString {
        self.t(msg, args).into()
    }

    /// One labelled line of a copy: `label: value` (`hv-conn-diag-row`).
    fn row(&self, label: &str, value: &str) -> String {
        self.t(
            ids::hv::CONN_DIAG_ROW,
            ids::hv::HvConnDiagRow {
                label: Text(label.to_string()),
                value: Text(value.to_string()),
            },
        )
    }
}

// ---- shared pieces -----------------------------------------------------------------------------

/// Whole milliseconds (`Connected in 54 ms`, R16).
pub(crate) fn whole_ms(d: Duration) -> Count {
    Count(u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Seconds with `digits` decimals as exact text (`0.06`, `0.214`), truncated, never through
/// `f64` (R16).
pub(crate) fn seconds(d: Duration, digits: u32) -> ExactDecimal {
    let unit = 10u128.pow(3 - digits.min(3));
    let ticks = d.as_millis() / unit;
    let scale = 10u128.pow(digits.min(3));
    let width = digits.min(3) as usize;
    ExactDecimal(format!(
        "{}.{:0width$}",
        ticks / scale,
        ticks % scale,
        width = width
    ))
}

/// A trail step's name.
pub(crate) fn step_label(f: &Fmt, step: TrailStep) -> String {
    match step {
        TrailStep::PgPass => f.t(ids::hv::CONN_STEP_PGPASS, NoArgs),
        TrailStep::Resolve => f.t(ids::hv::CONN_STEP_RESOLVE, NoArgs),
        TrailStep::Connect => f.t(ids::hv::CONN_STEP_TCP, NoArgs),
        TrailStep::Tls => f.t(ids::hv::CONN_STEP_TLS, NoArgs),
        TrailStep::SignIn => f.t(ids::hv::CONN_STEP_AUTH, NoArgs),
        TrailStep::CheckServer => f.t(ids::hv::CONN_STEP_CHECK, NoArgs),
        TrailStep::OpenFile => f.t(ids::hv::CONN_STEP_OPEN, NoArgs),
    }
}

/// A finished step's duration (`14 ms`).
fn step_duration(f: &Fmt, d: Duration) -> String {
    f.t(
        ids::hv::CONN_STEP_MS,
        ids::hv::HvConnStepMs { ms: whole_ms(d) },
    )
}

/// The step trail's items: a done step shows its duration when the pipeline timed it.
pub(crate) fn trail_items(f: &Fmt, trail: &[StepTrace]) -> Vec<TrailItem> {
    trail
        .iter()
        .map(|t| TrailItem {
            label: step_label(f, t.step).into(),
            state: match t.outcome {
                StepOutcome::Pending => TrailState::Pending,
                StepOutcome::Running => TrailState::Running,
                StepOutcome::Done => TrailState::Done {
                    duration: t.elapsed.map(|d| step_duration(f, d).into()),
                },
                StepOutcome::Skipped => TrailState::Skipped,
                StepOutcome::Failed => TrailState::Failed,
            },
        })
        .collect()
}

/// What assistive technology hears for a trail (Fluent state words).
pub(crate) fn trail_a11y(f: &Fmt) -> TrailA11y {
    TrailA11y {
        label: f.s(ids::hv::CONN_STEP_LIST, NoArgs),
        pending: f.s(ids::hv::CONN_STEP_STATE_PENDING, NoArgs),
        running: f.s(ids::hv::CONN_STEP_STATE_RUNNING, NoArgs),
        done: f.s(ids::hv::CONN_STEP_STATE_DONE, NoArgs),
        failed: f.s(ids::hv::CONN_STEP_STATE_FAILED, NoArgs),
        skipped: f.s(ids::hv::CONN_STEP_STATE_SKIPPED, NoArgs),
    }
}

/// A step's state as a copy writes it: its duration, or its state word.
fn step_state_text(f: &Fmt, t: &StepTrace) -> String {
    match (t.outcome, t.elapsed) {
        (StepOutcome::Done, Some(d)) => step_duration(f, d),
        (StepOutcome::Done, None) => f.t(ids::hv::CONN_STEP_STATE_DONE, NoArgs),
        (StepOutcome::Pending, _) => f.t(ids::hv::CONN_STEP_STATE_PENDING, NoArgs),
        (StepOutcome::Running, _) => f.t(ids::hv::CONN_STEP_STATE_RUNNING, NoArgs),
        (StepOutcome::Skipped, _) => f.t(ids::hv::CONN_STEP_STATE_SKIPPED, NoArgs),
        (StepOutcome::Failed, _) => f.t(ids::hv::CONN_STEP_STATE_FAILED, NoArgs),
    }
}

/// The server's version as the user knows it: the raw version's first word (MariaDB's old
/// `5.5.5-` prefix and any `-suffix` dropped), else the parsed version.
pub(crate) fn version_text(info: &ServerInfo) -> String {
    let raw = info.raw_version.trim();
    let raw = raw.strip_prefix("5.5.5-").unwrap_or(raw);
    let word = raw
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .split('-')
        .next()
        .unwrap_or_default();
    if !word.is_empty() {
        return word.to_string();
    }
    info.version
        .as_ref()
        .map(|v| format!("{}.{}.{}", v.major, v.minor, v.patch))
        .unwrap_or_default()
}

fn method_of(auth: AuthProviderId) -> ids::hv::HvConnDiagAuthVMethod {
    use ids::hv::HvConnDiagAuthVMethod as M;
    match auth {
        AuthProviderId::Password => M::Password,
        AuthProviderId::None => M::None,
        AuthProviderId::PgPass => M::Pgpass,
        AuthProviderId::ClientCert => M::ClientCert,
    }
}

fn method_alone(auth: AuthProviderId) -> ids::hv::HvConnDiagAuthMethodMethod {
    use ids::hv::HvConnDiagAuthMethodMethod as M;
    match auth {
        AuthProviderId::Password => M::Password,
        AuthProviderId::None => M::None,
        AuthProviderId::PgPass => M::Pgpass,
        AuthProviderId::ClientCert => M::ClientCert,
    }
}

/// The TLS version number of a summary (`TLSv1.3` → `1.3`).
pub(crate) fn tls_version_number(v: &str) -> &str {
    let v = v.trim();
    let lower = v.get(..4).filter(|p| p.eq_ignore_ascii_case("tlsv"));
    let rest = if lower.is_some() { &v[4..] } else { v };
    rest.trim_start()
}

/// The encryption row: `TLS 1.3 · verified`, `unencrypted`, or not applicable for a file.
fn tls_text(f: &Fmt, tls: Option<&TlsSummary>, file: bool) -> String {
    if file {
        return f.t(ids::hv::CONN_DIAG_FILE, NoArgs);
    }
    let Some(tls) = tls.filter(|t| t.encrypted) else {
        return f.t(ids::hv::CONN_DIAG_TLS_NONE, NoArgs);
    };
    use ids::hv::{HvConnDiagTlsNovState as N, HvConnDiagTlsVState as V};
    let (v, n) = match tls.verified {
        TlsVerified::Full => (V::Full, N::Full),
        TlsVerified::CaOnly => (V::CaOnly, N::CaOnly),
        TlsVerified::No => (V::No, N::No),
    };
    match tls.version.as_deref().map(tls_version_number) {
        Some(version) if !version.is_empty() => f.t(
            ids::hv::CONN_DIAG_TLS_V,
            ids::hv::HvConnDiagTlsV {
                version: Text(version.to_string()),
                state: v,
            },
        ),
        _ => f.t(
            ids::hv::CONN_DIAG_TLS_NOV,
            ids::hv::HvConnDiagTlsNov { state: n },
        ),
    }
}

/// One warning of a successful test.
pub(crate) fn warning_text(f: &Fmt, w: &TestWarning) -> String {
    use hacivat_driver::ProbeWarning as P;
    match w {
        TestWarning::NoTls => f.t(ids::hv::CONN_WARN_NO_TLS, NoArgs),
        TestWarning::TlsNotVerified => f.t(ids::hv::CONN_WARN_TLS_NOT_VERIFIED, NoArgs),
        TestWarning::NotTested { version } => f.t(
            ids::hv::CONN_WARN_NOT_TESTED,
            ids::hv::HvConnWarnNotTested {
                version: Text(version.clone()),
            },
        ),
        TestWarning::UnsupportedAccepted { version } => f.t(
            ids::hv::CONN_WARN_UNSUPPORTED,
            ids::hv::HvConnWarnUnsupported {
                version: Text(version.clone()),
            },
        ),
        TestWarning::Probe(P::Replica) => f.t(ids::hv::CONN_WARN_REPLICA, NoArgs),
        TestWarning::Probe(P::ServerReadOnly) => f.t(ids::hv::CONN_WARN_SERVER_READ_ONLY, NoArgs),
        TestWarning::Probe(P::SuperReadOnly) => f.t(ids::hv::CONN_WARN_SUPER_READ_ONLY, NoArgs),
        TestWarning::Probe(P::LowWaitTimeout { seconds }) => f.t(
            ids::hv::CONN_WARN_LOW_WAIT_TIMEOUT,
            ids::hv::HvConnWarnLowWaitTimeout {
                seconds: Count(u64::from(*seconds)),
            },
        ),
        TestWarning::Probe(P::LocalInfileOn) => f.t(ids::hv::CONN_WARN_LOCAL_INFILE_ON, NoArgs),
        TestWarning::Probe(P::SqliteFileNotWritable) => {
            f.t(ids::hv::CONN_WARN_SQLITE_FILE_NOT_WRITABLE, NoArgs)
        }
        TestWarning::Probe(P::SqliteJournal { mode }) => f.t(
            ids::hv::CONN_WARN_SQLITE_JOURNAL,
            ids::hv::HvConnWarnSqliteJournal {
                mode: Text(mode.clone()),
            },
        ),
    }
}

/// A diagnostics row of a successful test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiagRow {
    /// `server`, `latency`, `auth`, `warn` or `tls` (the element IDs `diag-<key>-…`).
    pub key: &'static str,
    pub label: String,
    /// One value per line (several warnings).
    pub values: Vec<String>,
    /// Data drawn in mono beside the value (the TLS cipher).
    pub extra: Option<String>,
}

/// The diagnostics of a successful test (NOTES surface 1): Server, Latency, Signed in as,
/// Warnings, Encryption.
pub(crate) fn diag_rows(f: &Fmt, r: &TestReport) -> Vec<DiagRow> {
    let file = r
        .server
        .as_ref()
        .is_some_and(|s| s.kind == hacivat_proto::DbmsKind::Sqlite);
    let none = || f.t(ids::hv::CONN_DIAG_NONE, NoArgs);
    let server = r.server.as_ref().map_or_else(none, |s| {
        f.t(
            ids::hv::CONN_DIAG_SERVER_V,
            ids::hv::HvConnDiagServerV {
                product: Text(s.product.clone()),
                version: Text(version_text(s)),
            },
        )
    });
    let latency = r.ping_median.map_or_else(none, |d| {
        f.t(
            ids::hv::CONN_DIAG_LATENCY_V,
            ids::hv::HvConnDiagLatencyV { ms: whole_ms(d) },
        )
    });
    let auth = if file {
        f.t(ids::hv::CONN_DIAG_FILE, NoArgs)
    } else {
        match r.user.as_deref().filter(|u| !u.is_empty()) {
            Some(user) => f.t(
                ids::hv::CONN_DIAG_AUTH_V,
                ids::hv::HvConnDiagAuthV {
                    user: Text(user.to_string()),
                    method: method_of(r.auth),
                },
            ),
            None => f.t(
                ids::hv::CONN_DIAG_AUTH_METHOD,
                ids::hv::HvConnDiagAuthMethod {
                    method: method_alone(r.auth),
                },
            ),
        }
    };
    let mut warnings: Vec<String> = r.warnings.iter().map(|w| warning_text(f, w)).collect();
    if warnings.is_empty() {
        warnings.push(none());
    }
    let cipher = r
        .tls
        .as_ref()
        .filter(|t| t.encrypted && !file)
        .and_then(|t| t.cipher.clone());
    vec![
        DiagRow {
            key: "server",
            label: f.t(ids::hv::CONN_DIAG_SERVER, NoArgs),
            values: vec![server],
            extra: None,
        },
        DiagRow {
            key: "latency",
            label: f.t(ids::hv::CONN_DIAG_LATENCY, NoArgs),
            values: vec![latency],
            extra: None,
        },
        DiagRow {
            key: "auth",
            label: f.t(ids::hv::CONN_DIAG_AUTH, NoArgs),
            values: vec![auth],
            extra: None,
        },
        DiagRow {
            key: "warn",
            label: f.t(ids::hv::CONN_DIAG_WARN, NoArgs),
            values: warnings,
            extra: None,
        },
        DiagRow {
            key: "tls",
            label: f.t(ids::hv::CONN_DIAG_TLS, NoArgs),
            values: vec![tls_text(f, r.tls.as_ref(), file)],
            extra: cipher,
        },
    ]
}

// ---- errors ------------------------------------------------------------------------------------

/// The `-title` and `-body` message IDs of every class, interned once (a closed set).
fn stem_ids(class: ErrClass) -> (&'static str, &'static str) {
    static IDS: OnceLock<HashMap<ErrClass, (&'static str, &'static str)>> = OnceLock::new();
    let ids = IDS.get_or_init(|| {
        ErrClass::ALL
            .iter()
            .map(|c| {
                let stem = c.fluent_stem();
                let title: &'static str = Box::leak(format!("{stem}-title").into_boxed_str());
                let body: &'static str = Box::leak(format!("{stem}-body").into_boxed_str());
                (*c, (title, body))
            })
            .collect()
    });
    ids.get(&class).copied().unwrap_or(("", ""))
}

/// The error's Fluent arguments (`ErrArgs`): every variable as text, the port never grouped.
fn error_args(e: &ConnectError) -> Vec<(&'static str, ArgValue)> {
    let a = &e.args;
    let mut args = vec![
        ("name", ArgValue::Text(a.name.clone())),
        ("engine", ArgValue::Text(a.engine.clone())),
    ];
    let mut opt = |key: &'static str, value: Option<String>| {
        if let Some(v) = value {
            args.push((key, ArgValue::Text(v)));
        }
    };
    opt("host", a.host.clone());
    opt("port", a.port.map(|p| p.to_string()));
    opt("user", a.user.clone());
    opt("db", a.database.clone());
    opt("file", a.file.clone());
    opt("version", a.version.clone());
    args
}

/// The error's heading: built from what Hacivat sent, never from the server's text.
pub(crate) fn error_title(f: &Fmt, e: &ConnectError) -> String {
    let (title, _) = stem_ids(e.class);
    f.t(MsgRef::<DynArgs>::new(title), DynArgs(error_args(e)))
}

/// The error's explanation.
pub(crate) fn error_body(f: &Fmt, e: &ConnectError) -> String {
    let (_, body) = stem_ids(e.class);
    f.t(MsgRef::<DynArgs>::new(body), DynArgs(error_args(e)))
}

/// The step the connect failed at (R25); none for `config.invalid`.
pub(crate) fn failed_step(e: &ConnectError) -> Option<TrailStep> {
    e.trail
        .iter()
        .find(|t| t.outcome == StepOutcome::Failed)
        .map(|t| t.step)
}

/// The server's error code: the SQLSTATE, else the vendor code.
pub(crate) fn error_code(e: &ConnectError) -> Option<String> {
    let server = e.server.as_ref()?;
    server
        .sqlstate
        .clone()
        .or_else(|| server.vendor_code.map(|c| c.to_string()))
}

/// The technical details (NOTES surface 3), in order: (key, label, value). The class ID, the
/// code and the server's message are data.
pub(crate) fn tech_rows(f: &Fmt, e: &ConnectError) -> Vec<(&'static str, String, String)> {
    let mut rows = Vec::new();
    if let Some(step) = failed_step(e) {
        rows.push((
            "stage",
            f.t(ids::hv::CONN_ERR_STAGE, NoArgs),
            step_label(f, step),
        ));
    }
    rows.push((
        "class",
        f.t(ids::hv::CONN_ERR_CLASS, NoArgs),
        e.class.id().to_string(),
    ));
    if let Some(code) = error_code(e) {
        rows.push(("code", f.t(ids::hv::CONN_ERR_CODE, NoArgs), code));
    }
    if let Some(server) = &e.server {
        // ## m2a-finalfix (M4): the server's message is data, shown and copied as sent, except
        // a password `scrub_message` recognizes (a server that echoes a connection string); free
        // text is never blanked (round 2, N4).
        rows.push((
            "server",
            f.t(ids::hv::CONN_ERR_SERVER, NoArgs),
            hacivat_secrets::scrub_message(&server.text),
        ));
    }
    rows.push((
        "elapsed",
        f.t(ids::hv::CONN_ERR_ELAPSED_LABEL, NoArgs),
        f.t(
            ids::hv::CONN_ERR_ELAPSED,
            ids::hv::HvConnErrElapsed {
                seconds: seconds(e.elapsed, 3),
            },
        ),
    ));
    rows
}

// ---- the copies --------------------------------------------------------------------------------

/// The error's details as text: the title, heading, explanation, the trail and the technical
/// details, one per line.
fn error_lines(f: &Fmt, e: &ConnectError) -> Vec<String> {
    let mut lines = vec![
        f.t(
            ids::hv::CONN_ERR_TITLE,
            ids::hv::HvConnErrTitle {
                name: Text(e.args.name.clone()),
            },
        ),
        error_title(f, e),
        error_body(f, e),
    ];
    lines.extend(
        e.trail
            .iter()
            .map(|t| f.row(&step_label(f, t.step), &step_state_text(f, t))),
    );
    lines.extend(
        tech_rows(f, e)
            .into_iter()
            .map(|(_, label, value)| f.row(&label, &value)),
    );
    lines
}

/// The plain text "Copy details (English)" puts on the clipboard (R26).
pub fn error_details_text(cx: &mut App, e: &ConnectError) -> String {
    let f = Fmt::english(cx);
    error_lines(&f, e).join("\n")
}

/// The plain text "Copy report" puts on the clipboard (R26): a successful test's heading, the
/// trail with durations and the diagnostics; a failed test's error details.
pub fn test_report_text(cx: &mut App, r: &TestReport) -> String {
    let f = Fmt::english(cx);
    if let Some(e) = r.error.as_ref().filter(|_| !r.ok) {
        return error_lines(&f, e).join("\n");
    }
    let mut lines = vec![
        f.t(
            ids::hv::CONN_TEST_OK,
            ids::hv::HvConnTestOk {
                ms: whole_ms(r.total),
            },
        ),
        f.t(ids::hv::CONN_TEST_OK_SUB, NoArgs),
    ];
    lines.extend(
        r.trail
            .iter()
            .map(|t| f.row(&step_label(&f, t.step), &step_state_text(&f, t))),
    );
    for row in diag_rows(&f, r) {
        for value in &row.values {
            lines.push(f.row(&row.label, value));
        }
        if let Some(cipher) = &row.extra {
            lines.push(f.row(&f.t(ids::hv::CONN_DIAG_CIPHER, NoArgs), cipher));
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds_are_exact_text_with_the_asked_decimals() {
        assert_eq!(seconds(Duration::from_millis(60), 2).0, "0.06");
        assert_eq!(seconds(Duration::from_millis(1234), 2).0, "1.23");
        assert_eq!(seconds(Duration::from_millis(214), 3).0, "0.214");
        assert_eq!(seconds(Duration::from_millis(12_005), 3).0, "12.005");
    }

    #[test]
    fn tls_versions_lose_their_prefix() {
        assert_eq!(tls_version_number("TLSv1.3"), "1.3");
        assert_eq!(tls_version_number("tlsv1.2"), "1.2");
        assert_eq!(tls_version_number("1.3"), "1.3");
    }

    #[test]
    fn every_class_has_interned_message_ids() {
        assert_eq!(
            stem_ids(ErrClass::AuthInvalid),
            (
                "error-conn-auth-invalid-title",
                "error-conn-auth-invalid-body"
            )
        );
        assert!(ErrClass::ALL.iter().all(|c| !stem_ids(*c).0.is_empty()));
    }
}
