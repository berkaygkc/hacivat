//! Connecting over an injected stream with `Config::connect_raw` (HV-CONN-01).
//!
//! The stream comes from the request's factory; `connect_raw` uses only the user, password,
//! database, options and application name of the [`Config`], never a host or port. The
//! password is copied into the `Config` (as bytes; its `Debug` redacts it) only here, at
//! connect time, and the `Config` is dropped as soon as the handshake ends (P49).

use std::io;
use std::sync::PoisonError;
use std::sync::atomic::Ordering;

use hacivat_driver::{DriverConnect, DriverEvent, Endpoint, RawError, RawErrorKind, ServerMessage};
use hacivat_net::TlsMode;
use secrecy::ExposeSecret;
use tokio_postgres::Config;
use tokio_postgres::config::SslMode;

use crate::probe::PgConn;
use crate::tls::{Connector, HandshakeError, Injected, Shared};

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
        // Every other key is ignored here. libpq's client-side keys (`sslmode`, `sslrootcert`,
        // `connect_timeout`, `target_session_attrs`, ...) are applied by the pipeline;
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
    let Endpoint::Network { factory, .. } = &req.endpoint else {
        return Err(raw(RawErrorKind::Unsupported));
    };
    if let Some(t) = &req.tls {
        // A failure left by an earlier attempt must not classify this one.
        *t.failure.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
    let shared = Shared::default();
    let connector = Connector::new(req.tls.as_ref(), req.events.clone(), shared.clone());
    let config = config_for(&req);
    let mode = config.get_ssl_mode();
    let events = req.events.clone();
    let attempt = async {
        let stream = factory.open().await.map_err(io_error)?;
        if mode == SslMode::Disable {
            shared.auth_started.store(true, Ordering::SeqCst);
            events(DriverEvent::AuthStarted);
        }
        config
            .connect_raw(Injected(stream), connector)
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
    let task = tokio::spawn(async move {
        // The error (if any) reaches the client as a closed connection.
        let _ = connection.await;
    });
    let tls_mode = shared
        .local()
        .and(req.tls.as_ref().map(|t| t.settings.mode));
    Ok(PgConn::new(client, task, tls_mode, shared.local()))
}

fn raw(kind: RawErrorKind) -> RawError {
    RawError { kind, server: None }
}

fn io_error(e: io::Error) -> RawError {
    raw(RawErrorKind::Io(e.kind()))
}

/// The library's text for "TLS required, `N` received" (`connect_tls.rs`). It is
/// `tokio-postgres`'s own constant, never server text.
const TLS_NOT_OFFERED: &str = "server does not support TLS";

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
    if source.is_some_and(|s| s.to_string() == TLS_NOT_OFFERED) {
        return raw(RawErrorKind::TlsNotOffered);
    }
    if e.is_closed() {
        return raw(RawErrorKind::Io(io::ErrorKind::ConnectionReset));
    }
    if let Some(io) = source.and_then(|s| s.downcast_ref::<io::Error>()) {
        // `Kind::Io` carries the transport's error; `Parse`/`Encode` wrap a malformed message.
        return raw(match io.kind() {
            io::ErrorKind::InvalidData | io::ErrorKind::InvalidInput => RawErrorKind::Protocol,
            kind => RawErrorKind::Io(kind),
        });
    }
    raw(RawErrorKind::Other)
}

#[cfg(test)]
mod tests {
    use super::*;

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
