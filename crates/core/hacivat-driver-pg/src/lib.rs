//! The PostgreSQL driver over `tokio-postgres`: stream injection through `connect_raw`, TLS
//! through `tokio-rustls`, detection and probes (HV-DRV-01, HV-CONN-01, HV-CONN-07).
//!
//! - **Connect** ([`connect`]): every connection runs over a stream from the caller's
//!   [`StreamFactory`](hacivat_net::StreamFactory) through `Config::connect_raw`, so direct
//!   TCP, Unix sockets and tunnels look alike. TLS is negotiated inside the protocol by our own
//!   connector ([`tls`]) over the caller's rustls configuration.
//! - **Probes** ([`probe`]): server version, replica state, TLS summary, the current user and
//!   the database list, on the live client.
//! - **Execution** ([`exec`], M2b): `execute`, `batch` and the type-cache-safe `reset` (Q10),
//!   with notices collected through `poll_message` (Q12).
//! - **Wire observer** ([`wire`], M2b): the insecure-auth guard (B8), the backend PID (Q11), the
//!   transaction status and command tags (Q12), on the decrypted side of the stream.
//! - **Cancel** ([`cancel`], M2b): `CancelRequest` over the same factory and TLS settings (B23).

// `RawError` carries the server's message by value (HV-CONN-03, I18N-14), so its size is by
// design; it travels once per failed call.
#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

// ## m2b-t01-interfaces: Task 10 fills these.
mod cancel;
mod connect;
mod exec;
mod probe;
mod tls;
mod wire;

use futures::future::BoxFuture;
use hacivat_driver::{
    CancelMechanism, Driver, DriverCaps, DriverConnect, DriverDescriptor, RawConn, RawError,
};
use hacivat_proto::DbmsKind;

pub use connect::{config_for, connect_conn, ssl_mode};
pub use probe::{PgConn, server_version};
pub use tokio_postgres::config::SslMode;
// ## m2b-t10-pg
#[doc(hidden)]
pub use connect::connect_conn_with;
pub use exec::{NOTICE_CAP, RESET_SQL, SQL_PREPARED, deallocate_sql};
#[doc(hidden)]
pub use tokio_postgres::config::ChannelBinding as ChannelBindingMode;
pub use wire::{InsecureAuthRefusedMarker, Observer, ObserverMode, TlsRefusedMarker, WireState};

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
