//! The PostgreSQL driver over `tokio-postgres`: stream injection through `connect_raw`, TLS
//! through `tokio-rustls`, detection and probes (HV-DRV-01, HV-CONN-01, HV-CONN-07).
//!
//! - **Connect** ([`connect`]): every connection runs over a stream from the caller's
//!   [`StreamFactory`](hacivat_net::StreamFactory) through `Config::connect_raw`, so direct
//!   TCP, Unix sockets and tunnels look alike. TLS is negotiated inside the protocol by our own
//!   connector ([`tls`]) over the caller's rustls configuration.
//! - **Probes** ([`probe`]): server version, replica state, TLS summary, the current user and
//!   the database list, on the live client.

// `RawError` carries the server's message by value (HV-CONN-03, I18N-14), so its size is by
// design; it travels once per failed call.
#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

mod connect;
mod probe;
mod tls;

use futures::future::BoxFuture;
use hacivat_driver::{
    CancelMechanism, Driver, DriverCaps, DriverConnect, DriverDescriptor, RawConn, RawError,
};
use hacivat_proto::DbmsKind;

pub use connect::{config_for, connect_conn, ssl_mode};
pub use probe::{PgConn, server_version};
pub use tokio_postgres::config::SslMode;

static DESCRIPTOR: DriverDescriptor = DriverDescriptor {
    kind: DbmsKind::Postgres,
    crate_name: "hacivat-driver-pg",
    caps: DriverCaps {
        stream_injection: true,
        in_protocol_tls: true,
        cancel: CancelMechanism::ProtocolCancelRequest,
        file_based: false,
    },
};

/// The PostgreSQL driver (HV-DRV-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PgDriver;

impl PgDriver {
    pub fn new() -> Self {
        PgDriver
    }
}

impl Driver for PgDriver {
    fn descriptor(&self) -> &DriverDescriptor {
        &DESCRIPTOR
    }

    fn connect(
        &self,
        req: DriverConnect,
    ) -> BoxFuture<'static, Result<Box<dyn RawConn>, RawError>> {
        Box::pin(async move {
            let conn = connect_conn(req).await?;
            Ok(Box::new(conn) as Box<dyn RawConn>)
        })
    }
}
