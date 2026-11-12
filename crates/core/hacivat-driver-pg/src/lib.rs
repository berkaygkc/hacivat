//! The PostgreSQL driver over `tokio-postgres`: stream injection through `connect_raw`, TLS
//! through `tokio-rustls`, detection and probes (HV-DRV-01). Task 13 implements it; Task 1's
//! stub reports its descriptor and refuses to connect.

mod connect;
mod probe;
mod tls;

use futures::future::BoxFuture;
use hacivat_driver::{
    CancelMechanism, Driver, DriverCaps, DriverConnect, DriverDescriptor, RawConn, RawError,
    RawErrorKind,
};
use hacivat_proto::DbmsKind;

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
        _req: DriverConnect,
    ) -> BoxFuture<'static, Result<Box<dyn RawConn>, RawError>> {
        Box::pin(async {
            Err(RawError {
                kind: RawErrorKind::Unsupported,
                server: None,
            })
        })
    }
}
