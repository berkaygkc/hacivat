//! The SQLite driver over bundled `rusqlite`: open modes, journal and writability probes
//! (HV-DRV-01). Task 15 implements it; Task 1's stub reports its descriptor and refuses to
//! connect.

mod open;
mod probe;

use futures::future::BoxFuture;
use hacivat_driver::{
    CancelMechanism, Driver, DriverCaps, DriverConnect, DriverDescriptor, RawConn, RawError,
    RawErrorKind,
};
use hacivat_proto::DbmsKind;

static DESCRIPTOR: DriverDescriptor = DriverDescriptor {
    kind: DbmsKind::Sqlite,
    crate_name: "hacivat-driver-sqlite",
    caps: DriverCaps {
        stream_injection: false,
        in_protocol_tls: false,
        cancel: CancelMechanism::Interrupt,
        file_based: true,
    },
};

/// The SQLite driver (HV-DRV-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SqliteDriver;

impl SqliteDriver {
    pub fn new() -> Self {
        SqliteDriver
    }
}

impl Driver for SqliteDriver {
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
