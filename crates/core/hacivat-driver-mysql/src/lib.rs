//! The MySQL and MariaDB driver over `mysql_async`: the loopback-forwarder path (R18), TLS
//! with OS roots (P10) and family detection (HV-DRV-01).
//!
//! `mysql_async` 0.37 cannot speak over a caller-provided stream (`Conn::new` always dials, and
//! its `Endpoint` holds a `TcpStream`), so `stream_injection` is false. A Direct TCP endpoint
//! is dialled by `mysql_async` itself; every other endpoint goes through a TCP loopback
//! forwarder on `127.0.0.1:0`, never a Unix socket (`mysql_async` applies no TLS over one).
//!
//! `mysql_async::Opts` keeps the password as a `String` and derives `Debug` (P49): this crate
//! builds it at the last moment, drops its own copy after the handshake and never formats it.

// `RawError` carries the server's message by value (HV-CONN-03); one per failed call.
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

#[doc(hidden)]
pub use connect::{
    MySqlConn, connect_conn, opts_for, opts_for_forwarded, plaintext_fallback_allowed, raw_error,
    trail_events,
};
#[doc(hidden)]
pub use probe::{probe_warnings, server_version_text, user_databases};
#[doc(hidden)]
pub use tls::os_roots;
pub use tls::ssl_opts;

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

/// The MySQL-family driver (HV-DRV-01). MariaDB resolves to it through the catalog fallback.
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
        req: DriverConnect,
    ) -> BoxFuture<'static, Result<Box<dyn RawConn>, RawError>> {
        Box::pin(async move {
            let conn = connect::connect(req).await?;
            Ok(Box::new(conn) as Box<dyn RawConn>)
        })
    }
}
