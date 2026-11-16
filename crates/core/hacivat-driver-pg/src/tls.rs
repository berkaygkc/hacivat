//! The rustls connector for `tokio-postgres`, ring provider only (HV-SEC-02).
//!
//! `connect_raw` asks a [`TlsConnect`] to wrap the injected stream once the server has
//! accepted the `SSLRequest`. Ours runs `tokio-rustls` with the caller's [`TlsParams`]
//! (configuration, server name and verifier), reports `TlsStarted` / `TlsDone`, and turns a
//! failed handshake into a [`TlsFailure`] that is also written into `TlsParams::failure`.
//!
//! M2b (Task 10): the stream it wraps is the raw [`Observer`] (now passing TLS records through),
//! and the stream it returns puts a second, framed observer above rustls, so every message after
//! the handshake is observed decrypted (Q12). The handshake's verification outcome is captured
//! into the shared [`crate::wire::WireState`] for the insecure-auth guard (B8, Q20), and the
//! stream offers `tls-server-end-point` channel binding (B9).

use std::error::Error;
use std::fmt;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};

use futures::future::BoxFuture;
use hacivat_driver::{DriverEvent, DriverEvents};
use hacivat_net::{BoxedStream, TlsFailure, TlsParams, TlsVerified, tls_server_end_point};
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_postgres::tls::{ChannelBinding, TlsConnect, TlsStream};
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream as RustlsStream;

use crate::wire::{Observer, ObserverMode};

/// The injected stream under a named type. `connect_raw` is generic over the stream, and a
/// bare `Box<dyn AsyncStream>` there trips the compiler's higher-ranked lifetime check when
/// the connect future must be `Send`.
pub(crate) struct Injected(pub BoxedStream);

impl AsyncRead for Injected {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for Injected {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

/// What our side of a finished handshake negotiated (the fallback for `tls_summary`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalTls {
    pub version: Option<String>,
    pub cipher: Option<String>,
}

/// State shared between the connector (consumed by `connect_raw`) and the connect call.
#[derive(Clone, Default)]
pub(crate) struct Shared {
    /// Set once the handshake finished.
    pub local: Arc<Mutex<Option<LocalTls>>>,
    /// `AuthStarted` was already reported (right after `TlsDone`).
    pub auth_started: Arc<AtomicBool>,
}

impl Shared {
    pub fn local(&self) -> Option<LocalTls> {
        self.local
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The TLS side of one connect attempt. Without parameters (TLS mode Disable) the attempt
/// runs with `SslMode::Disable`, so `connect_raw` never calls [`TlsConnect::connect`].
pub(crate) struct Connector {
    tls: Option<Handshake>,
    events: DriverEvents,
    shared: Shared,
}

struct Handshake {
    connector: TlsConnector,
    server_name: ServerName<'static>,
    failure: Arc<Mutex<Option<TlsFailure>>>,
    /// Written by the verifier on success; captured right after the handshake.
    outcome: Arc<Mutex<Option<TlsVerified>>>,
}

impl Connector {
    pub fn new(params: Option<&TlsParams>, events: DriverEvents, shared: Shared) -> Self {
        let tls = params.map(|p| Handshake {
            connector: TlsConnector::from(p.config.clone()),
            server_name: p.server_name.clone(),
            failure: p.failure.clone(),
            outcome: p.outcome.clone(),
        });
        Connector {
            tls,
            events,
            shared,
        }
    }
}

/// A failed handshake: the classified failure, or `None` for a plain I/O error (the server
/// hung up mid-handshake). Its text never includes certificate or secret material.
#[derive(Debug)]
pub(crate) struct HandshakeError {
    pub failure: Option<TlsFailure>,
    pub io_kind: io::ErrorKind,
}

impl fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.failure {
            Some(failure) => write!(f, "TLS handshake failed: {failure:?}"),
            None => write!(f, "TLS handshake failed: {:?}", self.io_kind),
        }
    }
}

impl Error for HandshakeError {}

/// The failure of a handshake error: what the verifier recorded first, else the rustls error
/// inside the I/O error (alerts and protocol errors the verifier never sees).
fn handshake_failure(recorded: Option<TlsFailure>, e: &io::Error) -> Option<TlsFailure> {
    recorded.or_else(|| {
        e.get_ref()
            .and_then(|inner| inner.downcast_ref::<rustls::Error>())
            .map(TlsFailure::from_rustls)
    })
}

fn protocol_name(v: rustls::ProtocolVersion) -> Option<String> {
    match v {
        rustls::ProtocolVersion::TLSv1_3 => Some("TLSv1.3".into()),
        rustls::ProtocolVersion::TLSv1_2 => Some("TLSv1.2".into()),
        _ => None,
    }
}

impl TlsConnect<Observer<Injected>> for Connector {
    type Stream = PgTlsStream;
    type Error = HandshakeError;
    type Future = BoxFuture<'static, Result<PgTlsStream, HandshakeError>>;

    fn connect(self, stream: Observer<Injected>) -> Self::Future {
        Box::pin(async move {
            let Some(hs) = self.tls else {
                // Unreachable: without parameters the mode is Disable and no SSLRequest is sent.
                return Err(HandshakeError {
                    failure: None,
                    io_kind: io::ErrorKind::Unsupported,
                });
            };
            (self.events)(DriverEvent::TlsStarted);
            let wire = stream.state().clone();
            match hs.connector.connect(hs.server_name, stream).await {
                Ok(tls) => {
                    // The outcome of this handshake, before anything else can touch the slot.
                    wire.set_outcome(*hs.outcome.lock().unwrap_or_else(PoisonError::into_inner));
                    let (_, session) = tls.get_ref();
                    let local = LocalTls {
                        version: session.protocol_version().and_then(protocol_name),
                        cipher: session
                            .negotiated_cipher_suite()
                            .and_then(|s| s.suite().as_str())
                            .map(str::to_string),
                    };
                    *self
                        .shared
                        .local
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner) = Some(local);
                    (self.events)(DriverEvent::TlsDone);
                    self.shared.auth_started.store(true, Ordering::SeqCst);
                    (self.events)(DriverEvent::AuthStarted);
                    Ok(PgTlsStream(Observer::new(tls, wire, ObserverMode::Framed)))
                }
                Err(e) => {
                    let mut slot = hs.failure.lock().unwrap_or_else(PoisonError::into_inner);
                    let failure = handshake_failure(*slot, &e);
                    if slot.is_none() {
                        *slot = failure;
                    }
                    Err(HandshakeError {
                        failure,
                        io_kind: e.kind(),
                    })
                }
            }
        })
    }
}

/// A rustls session over the injected stream, as `tokio-postgres` wants it, with the framed
/// observer on its decrypted side (Q12).
///
/// It offers `tls-server-end-point` channel binding (RFC 5929, B9) when the leaf's signature
/// algorithm defines one, so SCRAM runs as `SCRAM-SHA-256-PLUS` where the server offers it;
/// otherwise (Ed25519 and unknown algorithms) none, and SCRAM runs as `SCRAM-SHA-256`.
pub struct PgTlsStream(Observer<RustlsStream<Observer<Injected>>>);

impl PgTlsStream {
    /// The `tls-server-end-point` hash of the server's leaf certificate, if it has one.
    pub fn end_point_hash(&self) -> Option<Vec<u8>> {
        let (_, session) = self.0.get_ref().get_ref();
        let leaf = session.peer_certificates()?.first()?;
        tls_server_end_point(leaf.as_ref())
    }
}

impl AsyncRead for PgTlsStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for PgTlsStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

impl TlsStream for PgTlsStream {
    fn channel_binding(&self) -> ChannelBinding {
        match self.end_point_hash() {
            Some(hash) => ChannelBinding::tls_server_end_point(hash),
            None => ChannelBinding::none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_verifier_wins_then_the_rustls_error_is_read() {
        let alert = io::Error::new(
            io::ErrorKind::InvalidData,
            rustls::Error::AlertReceived(rustls::AlertDescription::CertificateRequired),
        );
        assert_eq!(
            handshake_failure(None, &alert),
            Some(TlsFailure::ClientCertRequired)
        );
        assert_eq!(
            handshake_failure(Some(TlsFailure::Expired), &alert),
            Some(TlsFailure::Expired)
        );
        let eof = io::Error::from(io::ErrorKind::UnexpectedEof);
        assert_eq!(handshake_failure(None, &eof), None);
    }

    #[test]
    fn protocol_names_match_pg_stat_ssl() {
        assert_eq!(
            protocol_name(rustls::ProtocolVersion::TLSv1_3).as_deref(),
            Some("TLSv1.3")
        );
        assert_eq!(
            protocol_name(rustls::ProtocolVersion::TLSv1_2).as_deref(),
            Some("TLSv1.2")
        );
    }
}
