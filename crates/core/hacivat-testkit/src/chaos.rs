//! `ChaosProxy`: a loopback TCP proxy that can cut, blackhole and restore a connection, for the
//! liveness and reconnect tests (Task 16).
//!
//! - [`ChaosProxy::cut`]: closes every relay and closes every new connection at once for the
//!   given time (the peer sees the connection end).
//! - [`ChaosProxy::blackhole`]: bytes are read and thrown away in both directions; no
//!   connection is closed, so the peers see silence.
//! - [`ChaosProxy::restore`]: ends a cut and a blackhole.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::task::JoinHandle;

#[derive(Debug)]
struct Shared {
    accepted: AtomicUsize,
    /// While set, new connections are closed at once.
    refusing: AtomicBool,
    blackhole: AtomicBool,
    /// Bumped by every cut (and on drop): a relay that sees a change ends.
    epoch: watch::Sender<u64>,
}

/// A running proxy toward `upstream`.
#[derive(Debug)]
pub struct ChaosProxy {
    pub addr: SocketAddr,
    shared: Arc<Shared>,
    runtime: Handle,
    accept_task: JoinHandle<()>,
    /// The pending end of the last cut; a newer cut or a restore supersedes it.
    cut_generation: Arc<AtomicUsize>,
}

impl ChaosProxy {
    /// Starts the proxy on a free loopback port. Needs a tokio runtime.
    ///
    /// # Panics
    /// When no loopback port can be bound.
    pub async fn start(upstream: SocketAddr) -> ChaosProxy {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the chaos proxy");
        let addr = listener.local_addr().expect("proxy address");
        let (epoch, _) = watch::channel(0);
        let shared = Arc::new(Shared {
            accepted: AtomicUsize::new(0),
            refusing: AtomicBool::new(false),
            blackhole: AtomicBool::new(false),
            epoch,
        });
        let accept_task = tokio::spawn(accept_loop(listener, upstream, Arc::clone(&shared)));
        ChaosProxy {
            addr,
            shared,
            runtime: Handle::current(),
            accept_task,
            cut_generation: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Closes every proxied connection now and closes every new one for `for_` (or until
    /// [`restore`](Self::restore)).
    pub fn cut(&self, for_: Duration) {
        self.shared.refusing.store(true, Ordering::SeqCst);
        self.shared.epoch.send_modify(|e| *e += 1);
        let mine = self.cut_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let shared = Arc::clone(&self.shared);
        let generation = Arc::clone(&self.cut_generation);
        self.runtime.spawn(async move {
            tokio::time::sleep(for_).await;
            // Only the latest cut ends the refusal.
            if generation.load(Ordering::SeqCst) == mine {
                shared.refusing.store(false, Ordering::SeqCst);
            }
        });
    }

    /// While on, bytes are swallowed in both directions without closing anything.
    pub fn blackhole(&self, on: bool) {
        self.shared.blackhole.store(on, Ordering::SeqCst);
    }

    /// Ends a cut and a blackhole.
    pub fn restore(&self) {
        self.cut_generation.fetch_add(1, Ordering::SeqCst);
        self.shared.refusing.store(false, Ordering::SeqCst);
        self.shared.blackhole.store(false, Ordering::SeqCst);
    }

    /// How many connections the proxy has relayed to the upstream (refused ones, and ones
    /// whose upstream connect failed, do not count).
    pub fn accepted(&self) -> usize {
        self.shared.accepted.load(Ordering::SeqCst)
    }
}

impl Drop for ChaosProxy {
    fn drop(&mut self) {
        self.accept_task.abort();
        self.shared.epoch.send_modify(|e| *e += 1);
    }
}

async fn accept_loop(listener: TcpListener, upstream: SocketAddr, shared: Arc<Shared>) {
    while let Ok((client, _)) = listener.accept().await {
        // Subscribe before looking at `refusing`: a cut that lands in between is seen by
        // either the flag or the receiver.
        let epoch = shared.epoch.subscribe();
        if shared.refusing.load(Ordering::SeqCst) {
            drop(client);
            continue;
        }
        tokio::spawn(relay(client, upstream, Arc::clone(&shared), epoch));
    }
}

async fn relay(
    client: TcpStream,
    upstream: SocketAddr,
    shared: Arc<Shared>,
    mut epoch: watch::Receiver<u64>,
) {
    let Ok(server) = TcpStream::connect(upstream).await else {
        return;
    };
    shared.accepted.fetch_add(1, Ordering::SeqCst);
    let (client_read, client_write) = client.into_split();
    let (server_read, server_write) = server.into_split();
    let up = pump(client_read, server_write, Arc::clone(&shared));
    let down = pump(server_read, client_write, shared);
    tokio::select! {
        () = async { tokio::join!(up, down); } => {}
        _ = epoch.changed() => {}
    }
    // Dropping the halves closes both sockets.
}

/// Copies `from` to `to` until EOF or an error, swallowing the bytes while blackholed.
async fn pump<R, W>(mut from: R, mut to: W, shared: Arc<Shared>)
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = [0u8; 8 * 1024];
    while let Ok(n) = from.read(&mut buf).await {
        if n == 0 {
            let _ = to.shutdown().await;
            break;
        }
        if shared.blackhole.load(Ordering::SeqCst) {
            continue;
        }
        if to.write_all(&buf[..n]).await.is_err() {
            break;
        }
    }
}
