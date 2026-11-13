//! Connecting over an injected stream with `Config::connect_raw` (HV-CONN-01).
//!
//! The stream comes from the request's factory; `connect_raw` uses only the user, password,
//! database, options and application name of the [`Config`], never a host or port. The
//! password is copied into the `Config` (as bytes; its `Debug` redacts it) only here, at
//! connect time, and the `Config` is dropped as soon as the handshake ends (P49).
//!
//! M2b (Task 10): the injected stream runs under the wire observer ([`crate::wire`]), which
//! enforces the insecure-auth guard (B8) and records the PID, the transaction status and the
//! command tags; the connection is driven through `poll_message`, so notices reach `execute`
//! (Q12); and the connection keeps a cancel handle over the same factory and TLS settings (B23).

use std::io;
use std::sync::PoisonError;
use std::sync::atomic::Ordering;

use hacivat_driver::{DriverConnect, DriverEvent, Endpoint, RawError, RawErrorKind, ServerMessage};
use hacivat_net::{TlsFailure, TlsMode};
use secrecy::ExposeSecret;
use tokio_postgres::Config;
use tokio_postgres::config::SslMode;

use crate::cancel::PgCancel;
use crate::exec::{Notices, drive};
use crate::probe::PgConn;
use crate::tls::{Connector, HandshakeError, Injected, Shared};
use crate::wire::{InsecureAuthRefusedMarker, Observer, ObserverMode, TlsRefusedMarker, WireState};

/// The `sslmode` for a profile's TLS mode (P9).
///
/// `Auto` may continue in plaintext (`Prefer`) only where the address allows it; toward a
/// public address it is `Require`, so no password ever crosses the network unencrypted. Every
/// verifying mode is `Require`: verification itself lives in the rustls configuration.
pub fn ssl_mode(mode: TlsMode, plaintext_allowed: bool) -> SslMode {
    match mode {
        TlsMode::Disable => SslMode::Disable,
        TlsMode::Auto if plaintext_allowed => SslMode::Prefer,
        TlsMode::Auto | TlsMode::Require | TlsMode::VerifyCa | TlsMode::VerifyFull => {
            SslMode::Require
        }
    }
}

/// Server settings that may arrive as their own option (the catalog defaults) and are sent as
/// `-c name=value`.
const SETTING_KEYS: [&str; 1] = ["extra_float_digits"];

/// `value` escaped for the startup `options` string: the server splits it on whitespace and
/// reads `\` as an escape.
fn escape_option_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if c == '\\' || c.is_ascii_whitespace() {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `s` without a dangling escape at its end: an odd run of trailing backslashes would escape
/// the separator we add next and glue the following option onto it.
fn without_dangling_escape(s: &str) -> &str {
    let trailing = s.bytes().rev().take_while(|b| *b == b'\\').count();
    if trailing % 2 == 1 {
        &s[..s.len() - 1]
    } else {
        s
    }
}

/// The startup `options` string: the sanitized extras in order, then the read-only default
/// last, so nothing before it can turn it off (a later `-c` wins).
fn startup_options(read_only: bool, extras: &[(String, String)]) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for (key, value) in extras {
        if key.eq_ignore_ascii_case("options") {
            let trimmed = without_dangling_escape(value.trim());
            if !trimmed.is_empty() {
                parts.push(trimmed.to_string());
            }
        } else if let Some(name) = SETTING_KEYS.iter().find(|k| key.eq_ignore_ascii_case(k)) {
            parts.push(format!("-c {name}={}", escape_option_value(value)));
        }
        // Every other key is ignored here. libpq's client-side keys with a typed setting
        // (`sslmode`, `sslrootcert`, `sslcert`/`sslkey`, `connect_timeout`) are folded into the
        // profile's typed settings where options enter a profile (`fold_typed_options`), so one
        // left over here has no effect, and neither has `target_session_attrs`;
        // `application_name` comes from the profile and `client_encoding` is always UTF8 in
        // `tokio-postgres`, which sends only fixed startup parameters.
    }
    if read_only {
        parts.push("-c default_transaction_read_only=on".into());
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// The `tokio-postgres` configuration for `req`, password included (P49): build it only at
/// connect time, never format it, drop it after the handshake.
#[doc(hidden)]
pub fn config_for(req: &DriverConnect) -> Config {
    let mut c = Config::new();
    if let Some(user) = &req.user {
        c.user(user);
    }
    if let Some(password) = &req.password {
        c.password(password.expose_secret().as_bytes());
    }
    if let Some(db) = &req.database {
        c.dbname(db);
    }
    c.application_name(&req.application_name);
    if let Some(options) = startup_options(req.read_only, &req.options) {
        c.options(options);
    }
    let mode = req.tls.as_ref().map_or(SslMode::Disable, |t| {
        ssl_mode(t.settings.mode, t.plaintext_allowed)
    });
    c.ssl_mode(mode);
    c
}

/// Connects and returns the concrete connection. [`crate::PgDriver`] boxes it; tests use it
/// to reach the client.
#[doc(hidden)]
pub async fn connect_conn(req: DriverConnect) -> Result<PgConn, RawError> {
    connect_conn_with(req, |_| {}).await
}

/// [`connect_conn`] with the `tokio-postgres` configuration adjusted by `adjust` before the
/// handshake (the driver's own tests: `channel_binding=require`).
#[doc(hidden)]
pub async fn connect_conn_with(
    req: DriverConnect,
    adjust: impl FnOnce(&mut Config),
) -> Result<PgConn, RawError> {
    let Endpoint::Network { factory, .. } = &req.endpoint else {
        return Err(raw(RawErrorKind::Unsupported));
    };
    if let Some(t) = &req.tls {
        // A failure left by an earlier attempt must not classify this one.
        *t.failure.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
    let shared = Shared::default();
    let connector = Connector::new(req.tls.as_ref(), req.events.clone(), shared.clone());
    let mut config = config_for(&req);
    adjust(&mut config);
    let mode = config.get_ssl_mode();
    let events = req.events.clone();
    let wire = WireState::new(req.insecure_auth);
    let attempt = async {
        let stream = factory.open().await.map_err(io_error)?;
        let observed = if mode == SslMode::Disable {
            shared.auth_started.store(true, Ordering::SeqCst);
            events(DriverEvent::AuthStarted);
            ObserverMode::Framed
        } else {
            ObserverMode::AwaitSslResponse
        };
        config
            .connect_raw(
                Observer::new(Injected(stream), wire.clone(), observed),
                connector,
            )
            .await
            .map_err(|e| pg_error(&e))
    };
    let result = tokio::time::timeout(req.connect_timeout, attempt).await;
    drop(config);
    let (client, connection) = match result {
        Ok(r) => r?,
        Err(_) => return Err(raw(RawErrorKind::Timeout)),
    };
    if !shared.auth_started.load(Ordering::SeqCst) {
        // `Prefer` toward a server that declined TLS: sign-in followed at once.
        events(DriverEvent::AuthStarted);
    }
    events(DriverEvent::AuthDone);
    let notices = Notices::default();
    let task = tokio::spawn(drive(connection, notices.clone(), wire.clone()));
    let local = shared.local();
    let cancel = PgCancel::new(
        client.cancel_token(),
        factory.clone(),
        req.tls.as_ref(),
        mode,
        local.is_some(),
    );
    let tls_mode = local
        .as_ref()
        .and(req.tls.as_ref().map(|t| t.settings.mode));
    Ok(PgConn::new(
        client, task, tls_mode, local, wire, notices, cancel,
    ))
}

fn raw(kind: RawErrorKind) -> RawError {
    RawError { kind, server: None }
}

pub(crate) fn io_error(e: io::Error) -> RawError {
    raw(RawErrorKind::Io(e.kind()))
}

/// The library's text for "TLS required, `N` received" (`connect_tls.rs`). It is
/// `tokio-postgres`'s own constant, never server text.
const TLS_NOT_OFFERED: &str = "server does not support TLS";

/// The library's text for "the server wants a password and the config has none"
/// (`connect_raw.rs`, an `Error::config`). It is `tokio-postgres`'s own constant, never server
/// text.
const PASSWORD_MISSING: &str = "password missing";

/// Maps a `tokio-postgres` error (HV-CONN-03). Server errors keep SQLSTATE, severity, text,
/// detail and hint verbatim; TLS failures carry their [`hacivat_net::TlsFailure`]; I/O keeps
/// its kind.
pub(crate) fn pg_error(e: &tokio_postgres::Error) -> RawError {
    if let Some(db) = e.as_db_error() {
        // The `V` field is never localized; a different `S` means a translated message.
        let lang_hint = db
            .parsed_severity()
            .filter(|v| v.to_string() != db.severity())
            .map(|_| "und".to_string());
        return RawError {
            kind: RawErrorKind::Db,
            server: Some(ServerMessage {
                sqlstate: Some(db.code().code().to_string()),
                vendor_code: None,
                severity: Some(db.severity().to_string()),
                text: db.message().to_string(),
                detail: db.detail().map(str::to_string),
                hint: db.hint().map(str::to_string),
                lang_hint,
            }),
        };
    }
    let source = std::error::Error::source(e);
    if let Some(hs) = source.and_then(|s| s.downcast_ref::<HandshakeError>()) {
        return raw(match hs.failure {
            Some(failure) => RawErrorKind::Tls(failure),
            None => RawErrorKind::Io(hs.io_kind),
        });
    }
    let inner = source
        .and_then(|s| s.downcast_ref::<io::Error>())
        .and_then(io::Error::get_ref);
    // The wire observer's refusals (B8; a TLS session's cancel answered in plaintext).
    if inner.is_some_and(|i| i.is::<InsecureAuthRefusedMarker>()) {
        return raw(RawErrorKind::InsecureAuthRefused);
    }
    if inner.is_some_and(|i| i.is::<TlsRefusedMarker>()) {
        return raw(RawErrorKind::TlsNotOffered);
    }
    if source.is_some_and(|s| s.to_string() == TLS_NOT_OFFERED) {
        return raw(RawErrorKind::TlsNotOffered);
    }
    if source.is_some_and(|s| s.to_string() == PASSWORD_MISSING) {
        return raw(RawErrorKind::PasswordRequired);
    }
    if e.is_closed() {
        return raw(RawErrorKind::Io(io::ErrorKind::ConnectionReset));
    }
    if let Some(io) = source.and_then(|s| s.downcast_ref::<io::Error>()) {
        return raw(io_kind(io));
    }
    raw(RawErrorKind::Other)
}

/// The kind of an I/O error from a live or starting connection. A rustls error inside it (an
/// alert after the handshake, which tokio-rustls reports as `InvalidData`) is a TLS failure.
/// Otherwise `Kind::Io` carries the transport's error and `Parse`/`Encode` wrap a malformed
/// message.
fn io_kind(io: &io::Error) -> RawErrorKind {
    if let Some(tls) = io
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
    {
        return RawErrorKind::Tls(TlsFailure::from_rustls(tls));
    }
    match io.kind() {
        io::ErrorKind::InvalidData | io::ErrorKind::InvalidInput => RawErrorKind::Protocol,
        kind => RawErrorKind::Io(kind),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rustls_error_after_the_handshake_is_a_tls_failure() {
        // tokio-rustls reports an alert on a live session as InvalidData around the rustls
        // error; it must not read as a malformed PostgreSQL message.
        let alert = io::Error::new(
            io::ErrorKind::InvalidData,
            rustls::Error::AlertReceived(rustls::AlertDescription::CertificateRequired),
        );
        assert_eq!(
            io_kind(&alert),
            RawErrorKind::Tls(TlsFailure::ClientCertRequired)
        );
        let decrypt = io::Error::new(io::ErrorKind::InvalidData, rustls::Error::DecryptError);
        assert_eq!(io_kind(&decrypt), RawErrorKind::Tls(TlsFailure::Protocol));
        // Without a rustls error inside, InvalidData is still a protocol error, and the
        // transport's own kinds pass through.
        let malformed = io::Error::new(io::ErrorKind::InvalidData, "invalid message length");
        assert_eq!(io_kind(&malformed), RawErrorKind::Protocol);
        let reset = io::Error::from(io::ErrorKind::ConnectionReset);
        assert_eq!(
            io_kind(&reset),
            RawErrorKind::Io(io::ErrorKind::ConnectionReset)
        );
    }

    #[test]
    fn options_escape_and_dangling_backslashes() {
        assert_eq!(escape_option_value("a b\\c"), "a\\ b\\\\c");
        assert_eq!(without_dangling_escape("x\\"), "x");
        assert_eq!(without_dangling_escape("x\\\\"), "x\\\\");
        assert_eq!(without_dangling_escape("x\\\\\\"), "x\\\\");
        assert_eq!(startup_options(false, &[]), None);
        assert_eq!(
            startup_options(false, &[("OPTIONS".into(), "  ".into())]),
            None
        );
        assert_eq!(
            startup_options(true, &[("unknown_key".into(), "1".into())]).as_deref(),
            Some("-c default_transaction_read_only=on")
        );
    }
}
