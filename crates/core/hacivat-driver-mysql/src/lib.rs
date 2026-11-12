//! The MySQL and MariaDB driver over `mysql_async`: the loopback-forwarder path (R18), TLS
//! with OS roots (P10) and family detection (HV-DRV-01). Task 14 implements it; Task 1's stub
//! reports its descriptor and refuses to connect.

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
    kind: DbmsKind::MySql,
    crate_name: "hacivat-driver-mysql",
    caps: DriverCaps {
        stream_injection: false,
        in_protocol_tls: true,
        cancel: CancelMechanism::KillQuery,
        file_based: false,
    },
};

/// The MySQL-family driver (HV-DRV-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct MySqlDriver;

impl MySqlDriver {
    pub fn new() -> Self {
        MySqlDriver
    }
}

impl Driver for MySqlDriver {
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
