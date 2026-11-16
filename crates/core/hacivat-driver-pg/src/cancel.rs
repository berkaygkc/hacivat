//! PostgreSQL's `CancelRequest` on a new stream from the same factory (Task 10, B23).
//!
//! The request goes over the connection's own transport (direct, tunnel or proxy: the factory)
//! and with its TLS settings: the same rustls configuration (verifier, pins, CA, client
//! certificate) and the same `sslmode`. A session that ran over TLS never sends its cancel key
//! in plaintext: when the server answers the cancel's `SSLRequest` with anything but `S`, the
//! request fails before the key is written ([`ObserverMode::AwaitSslAccept`]), even under
//! `sslmode=prefer`.

use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use hacivat_driver::{CancelHandle, DriverEvents, InsecureAuth, RawError, RawErrorKind};
use hacivat_net::{StreamFactory, TlsParams};
use tokio_postgres::CancelToken;
use tokio_postgres::config::SslMode;

use crate::connect::{io_error, pg_error};
use crate::tls::{Connector, Injected, Shared};
use crate::wire::{Observer, ObserverMode, WireState};

/// The longest a cancel request may take, from opening the stream to the last byte written.
const CANCEL_TIMEOUT: Duration = Duration::from_secs(10);

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
