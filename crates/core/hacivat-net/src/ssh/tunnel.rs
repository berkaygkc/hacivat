//! The tunnel manager (HV-SEC-01, Task 9): opens a chain of SSH hops (russh, ring only), shares
//! it between connections of the same chain, hands out `direct-tcpip` stream factories, keeps
//! it alive and releases it 30 s after its last lease.
//!
//! Task 1 stub: every `acquire` refuses, so no connection crosses an unchecked tunnel.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use super::known_hosts::KnownHosts;
use super::{SshError, SshPrompter, TransportGeneration, TunnelEvent, TunnelRequest};
use crate::stream::StreamFactory;

/// Tunnel timings.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TunnelConfig {
    /// How long a chain stays open after its last lease.
    pub idle_grace: Duration,
    pub keepalive: Duration,
    /// Unanswered keepalives before the chain counts as dead.
    pub keepalive_max: u32,
}

impl Default for TunnelConfig {
    /// 30 s grace, 30 s keepalive, 3 unanswered keepalives.
    fn default() -> Self {
        TunnelConfig {
            idle_grace: Duration::from_secs(30),
            keepalive: Duration::from_secs(30),
            keepalive_max: 3,
        }
    }
}

/// The app's SSH chains.
pub struct TunnelManager {
    #[allow(dead_code, reason = "Task 9 checks host keys against it")]
    known: Arc<KnownHosts>,
    #[allow(dead_code, reason = "Task 9 applies the timings")]
    config: TunnelConfig,
    events: broadcast::Sender<TunnelEvent>,
}

impl std::fmt::Debug for TunnelManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TunnelManager")
            .field("sessions_open", &self.sessions_open())
            .finish_non_exhaustive()
    }
}

impl TunnelManager {
    pub fn new(known: Arc<KnownHosts>, config: TunnelConfig) -> Arc<TunnelManager> {
        Arc::new(TunnelManager {
            known,
            config,
            events: broadcast::Sender::new(16),
        })
    }

    /// Opens (or shares) the chain of `req`, asking `prompter` for unknown keys and missing
    /// secrets. Task 1 stub: refuses (`SshError::Protocol { hop: 0 }`).
    pub fn acquire(
        self: &Arc<Self>,
        _req: TunnelRequest,
        _prompter: Option<Arc<dyn SshPrompter>>,
        _cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<TunnelLease, SshError>> {
        Box::pin(async { Err(SshError::Protocol { hop: 0 }) })
    }

    /// Open SSH sessions, all chains and hops counted.
    pub fn sessions_open(&self) -> usize {
        0
    }

    pub fn events(&self) -> broadcast::Receiver<TunnelEvent> {
        self.events.subscribe()
    }
}

/// A use of an open chain. Dropping it releases the chain (in reverse hop order, after the
/// grace period at zero users).
#[derive(Debug)]
pub struct TunnelLease {
    generation: TransportGeneration,
    bastion: String,
    hops: u8,
}

impl TunnelLease {
    /// A factory of `direct-tcpip` streams to `host:port` as the bastion sees it. Task 1 stub:
    /// never constructed; its factory fails.
    pub fn factory(&self, _host: &str, _port: u16) -> StreamFactory {
        StreamFactory::new(|| {
            Box::pin(async {
                Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "ssh tunnel: Task 9",
                ))
            })
        })
    }

    pub fn generation(&self) -> TransportGeneration {
        self.generation
    }

    /// The last hop's host, as the capsule names the path.
    pub fn bastion(&self) -> &str {
        &self.bastion
    }

    pub fn hops(&self) -> u8 {
        self.hops
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::SshSpec;
    use std::path::PathBuf;

    #[tokio::test]
    async fn the_stub_refuses_every_tunnel() {
        let known = KnownHosts::load(Vec::new(), PathBuf::from("/nonexistent/known_hosts"));
        let tunnels = TunnelManager::new(known, TunnelConfig::default());
        let req = TunnelRequest {
            spec: SshSpec::default(),
            secrets: Vec::new(),
            share: true,
            connect_timeout: Duration::from_secs(1),
        };
        let lease = tunnels.acquire(req, None, CancellationToken::new()).await;
        assert_eq!(lease.err(), Some(SshError::Protocol { hop: 0 }));
        assert_eq!(tunnels.sessions_open(), 0);
        assert_eq!(TunnelConfig::default().keepalive_max, 3);
    }
}
