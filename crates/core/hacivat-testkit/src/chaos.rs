//! `ChaosProxy`: a loopback TCP proxy that can cut, blackhole and restore a connection, for the
//! liveness and reconnect tests (Task 16).
//!
//! Task 1 ships the API with a stub: the proxy does not listen (`addr` has port 0).

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

/// A running proxy toward `upstream`.
#[derive(Debug)]
pub struct ChaosProxy {
    pub addr: SocketAddr,
    #[allow(dead_code, reason = "Task 16 forwards to it")]
    upstream: SocketAddr,
}

impl ChaosProxy {
    /// Starts the proxy on loopback. Task 1 stub: does not listen.
    pub fn start(upstream: SocketAddr) -> ChaosProxy {
        ChaosProxy {
            addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            upstream,
        }
    }

    /// Resets every proxied connection after `d`.
    pub fn cut(&self, _d: Duration) {}

    /// While on, bytes are swallowed in both directions without a reset.
    pub fn blackhole(&self, _on: bool) {}

    /// Ends a cut or a blackhole.
    pub fn restore(&self) {}
}
