//! PostgreSQL's `CancelRequest` on a new stream from the same factory (Task 10, B23).
//!
//! The request goes over the connection's own transport (direct, tunnel or proxy: the factory)
//! and with its TLS settings: the same rustls configuration (verifier, pins, CA, client
//! certificate) and the same `sslmode`. A session that ran over TLS never sends its cancel key
//! in plaintext: when the server answers the cancel's `SSLRequest` with anything but `S`, the
//! request fails before the key is written ([`ObserverMode::AwaitSslAccept`]), even under
//! `sslmode=prefer`.
//!
//! ## m4-pgcancel
//! **A cancel ends once the server acted on it.** PostgreSQL answers nothing to a
//! `CancelRequest`: the server process that reads it signals the backend, then closes the
//! connection. `tokio-postgres`'s `cancel_query_raw` returns once the request is written, so a
//! cancel that "ended" could still be on its way and land on the session's **next** statement
//! (CI run 44595239297: a `SELECT count(*)` after a Stop got 57014). The cancel stream here
//! ([`AwaitClose`]) makes its shutdown, the request's last step, wait until the server closed
//! the connection, as libpq's `PQcancel` does. The backend then holds the signal before it can
//! read another statement, and PostgreSQL drops a cancel that reaches an idle session. Every
//! caller that awaits the cancel before the next statement (the session's `settle`, the
//! cursor's `guard`, [`crate::typed`]'s close of a Run as one) relies on this.

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, ready};
use std::time::Duration;

use futures::future::BoxFuture;
use hacivat_driver::{CancelHandle, DriverEvents, InsecureAuth, RawError, RawErrorKind};
use hacivat_net::{BoxedStream, StreamFactory, TlsParams};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_postgres::CancelToken;
use tokio_postgres::config::SslMode;

use crate::connect::{io_error, pg_error};
use crate::tls::{Connector, Injected, Shared};
use crate::wire::{Observer, ObserverMode, WireState};

/// The longest a cancel request may take, from opening the stream to the server's close of it
/// (## m4-pgcancel). A cancel that times out may still land: its callers treat it as unsettled.
const CANCEL_TIMEOUT: Duration = Duration::from_secs(10);

// ## m4-pgcancel
/// The cancel connection's stream: its shutdown, after shutting down the write half, reads and
/// drops whatever arrives until the server closes the connection (a read error counts as
/// closed, as in libpq). It sits below TLS (whose shutdown sends `close_notify`, then shuts
/// this stream down), so the server's TLS alerts are dropped with the rest; [`CANCEL_TIMEOUT`]
/// bounds the wait.
pub(crate) struct AwaitClose {
    inner: BoxedStream,
    write_shut: bool,
}

impl AwaitClose {
    pub(crate) fn new(inner: BoxedStream) -> AwaitClose {
        AwaitClose {
            inner,
            write_shut: false,
        }
    }
}

impl AsyncRead for AwaitClose {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for AwaitClose {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if !self.write_shut {
            // The request is written and flushed by now. A failed shutdown (a peer that already
            // went away) changes nothing: the read below sees the close either way.
            let _ = ready!(Pin::new(&mut self.inner).poll_shutdown(cx));
            self.write_shut = true;
        }
        let mut scratch = [0_u8; 256];
        loop {
            let mut buf = ReadBuf::new(&mut scratch);
            match ready!(Pin::new(&mut self.inner).poll_read(cx, &mut buf)) {
                Ok(()) if buf.filled().is_empty() => return Poll::Ready(Ok(())),
                Ok(()) => {}
                Err(_) => return Poll::Ready(Ok(())),
            }
        }
    }
}

/// How the cancel stream's observer starts: no `SSLRequest` without TLS; with one, a session
/// that ran over TLS accepts only `S`.
pub(crate) fn cancel_mode(ssl_mode: SslMode, session_tls: bool) -> ObserverMode {
    match ssl_mode {
        SslMode::Disable => ObserverMode::Passthrough,
        _ if session_tls => ObserverMode::AwaitSslAccept,
        _ => ObserverMode::AwaitSslResponse,
    }
}

/// Cancels the statement running on one PostgreSQL connection (B23).
pub(crate) struct PgCancel {
    pub token: CancelToken,
    pub factory: StreamFactory,
    /// The connection's TLS parameters, forked: slots of the cancel's own, and every cancel
    /// forks them again (one per handshake).
    pub tls: Option<TlsParams>,
    pub mode: ObserverMode,
}

impl PgCancel {
    pub fn new(
        token: CancelToken,
        factory: StreamFactory,
        tls: Option<&TlsParams>,
        ssl_mode: SslMode,
        session_tls: bool,
    ) -> PgCancel {
        // ## m2b-t14-pipeline: TLS parameters of the cancel's own (T10 review M1): its
        // verifier records into slots the session never reads.
        let tls = tls.map(TlsParams::fork);
        PgCancel {
            token,
            factory,
            tls,
            mode: cancel_mode(ssl_mode, session_tls),
        }
    }
}

impl CancelHandle for PgCancel {
    fn cancel(&self) -> BoxFuture<'static, Result<(), RawError>> {
        let token = self.token.clone();
        let factory = self.factory.clone();
        let tls = self.tls.as_ref().map(TlsParams::fork);
        let mode = self.mode;
        Box::pin(async move {
            let attempt = async {
                let stream = factory.open().await.map_err(io_error)?;
                // ## m4-pgcancel: ends only once the server closed the connection.
                let stream: BoxedStream = Box::new(AwaitClose::new(stream));
                let events: DriverEvents = Arc::new(|_| {});
                let connector = Connector::new(tls.as_ref(), events, Shared::default());
                // No authentication runs on a cancel stream; the state only carries the mode.
                let stream = Observer::new(
                    Injected(stream),
                    WireState::new(InsecureAuth::RefuseUnlessVerified),
                    mode,
                );
                token
                    .cancel_query_raw(stream, connector)
                    .await
                    .map_err(|e| pg_error(&e))
            };
            match tokio::time::timeout(CANCEL_TIMEOUT, attempt).await {
                Ok(r) => r,
                Err(_) => Err(RawError {
                    kind: RawErrorKind::Timeout,
                    server: None,
                }),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tls_session_never_cancels_in_plaintext() {
        assert_eq!(
            cancel_mode(SslMode::Prefer, true),
            ObserverMode::AwaitSslAccept
        );
        assert_eq!(
            cancel_mode(SslMode::Require, true),
            ObserverMode::AwaitSslAccept
        );
        // A plaintext session under `prefer` takes TLS when offered, else continues as before.
        assert_eq!(
            cancel_mode(SslMode::Prefer, false),
            ObserverMode::AwaitSslResponse
        );
        assert_eq!(
            cancel_mode(SslMode::Disable, false),
            ObserverMode::Passthrough
        );
    }
}
