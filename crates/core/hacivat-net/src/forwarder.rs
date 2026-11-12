//! The loopback forwarder (R18): a local listener that pipes accepted connections into a
//! [`StreamFactory`], for drivers that cannot take an injected stream (MySQL). Body in Task 5.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::future::BoxFuture;

use crate::cleanup::CleanupGuard;
use crate::stream::StreamFactory;

/// Where a forwarder listens (R18).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ForwarderAddr {
    Tcp(std::net::SocketAddr),
    Unix(PathBuf),
}

/// How a forwarder binds (R18, P10): `Tcp` always binds `127.0.0.1:0` (the MySQL path, every
/// OS); `UnixOrTcp` binds a `0600` Unix socket in a `0700` directory on Unix and `127.0.0.1:0`
/// on Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ForwarderMode {
    Tcp,
    UnixOrTcp,
}

/// A bound loopback forwarder (R18). It accepts at most `max_accepts` connections.
#[derive(Debug)]
pub struct LoopbackForwarder {
    addr: ForwarderAddr,
    accepted: Arc<AtomicU64>,
}

impl LoopbackForwarder {
    /// Binds a forwarder for `factory`; the guard closes the listener (Task 5).
    pub fn bind(
        _factory: StreamFactory,
        _private_dir: &Path,
        _max_accepts: u32,
        _mode: ForwarderMode,
    ) -> BoxFuture<'static, io::Result<(LoopbackForwarder, CleanupGuard)>> {
        Box::pin(async { Err(io::Error::from(io::ErrorKind::Unsupported)) })
    }

    pub fn addr(&self) -> &ForwarderAddr {
        &self.addr
    }

    /// How many connections it accepted so far.
    pub fn accepted(&self) -> u64 {
        self.accepted.load(Ordering::SeqCst)
    }
}
