//! Direct transport: name resolution and Happy Eyeballs TCP or Unix connects (HV-CONN-01).
//! Bodies in Task 5.

use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use crate::stream::StreamFactory;

/// Where a direct transport connects (HV-CONN-01).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Target {
    Tcp { host: String, port: u16 },
    Unix { path: PathBuf },
}

/// Why resolving or connecting failed, before any protocol byte (HV-CONN-03).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NetError {
    Dns {
        host: String,
    },
    Refused,
    Timeout,
    Unreachable,
    Reset,
    Io {
        kind: io::ErrorKind,
        message: String,
    },
}

/// Resolves `host`; an IP literal resolves to itself without a query (Task 5).
pub async fn resolve(_host: &str, _port: u16) -> Result<Vec<SocketAddr>, NetError> {
    Err(NetError::Io {
        kind: io::ErrorKind::Unsupported,
        message: String::new(),
    })
}

/// A factory that connects to `target` over `resolved` (Happy Eyeballs, Task 5).
pub fn direct(
    _target: Target,
    _resolved: Vec<SocketAddr>,
    _connect_timeout: Duration,
) -> StreamFactory {
    StreamFactory::new(|| Box::pin(async { Err(io::Error::from(io::ErrorKind::Unsupported)) }))
}
