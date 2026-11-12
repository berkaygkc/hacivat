//! The loopback forwarder (R18, P10): relays local connections into a stream factory.

use std::io;
use std::net::SocketAddr;
#[cfg(unix)]
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use hacivat_net::{
    BoxedStream, ForwarderAddr, ForwarderMode, LoopbackForwarder, StreamFactory, Target, direct,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

async fn echo_server() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut r, mut w) = s.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    addr
}

/// A factory that dials a loopback echo server.
async fn echo_factory() -> StreamFactory {
    let addr = echo_server().await;
    direct(
        Target::Tcp {
            host: "127.0.0.1".into(),
            port: addr.port(),
        },
        vec![addr],
        Duration::from_secs(2),
    )
}

async fn connect_to_result(addr: &ForwarderAddr) -> io::Result<BoxedStream> {
    match addr {
        ForwarderAddr::Tcp(a) => Ok(Box::new(tokio::net::TcpStream::connect(a).await?)),
        #[cfg(unix)]
        ForwarderAddr::Unix(p) => Ok(Box::new(tokio::net::UnixStream::connect(p).await?)),
        #[cfg(not(unix))]
        ForwarderAddr::Unix(_) => Err(io::ErrorKind::Unsupported.into()),
    }
}

async fn connect_to(addr: &ForwarderAddr) -> BoxedStream {
    connect_to_result(addr).await.unwrap()
}

async fn read4(s: &mut BoxedStream) -> [u8; 4] {
    let mut buf = [0u8; 4];
    s.read_exact(&mut buf).await.unwrap();
    buf
}

#[cfg(unix)]
fn mode(p: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).unwrap().permissions().mode()
}

#[tokio::test]
async fn forwarder_relays_counts_and_cleans_up() {
    let dir = tempdir();
    let (fwd, guard) = LoopbackForwarder::bind(
        echo_factory().await,
        dir.path(),
        4,
        ForwarderMode::UnixOrTcp,
    )
    .await
    .unwrap();
    let mut c = connect_to(fwd.addr()).await;
    c.write_all(b"ping").await.unwrap();
    assert_eq!(read4(&mut c).await, *b"ping");
    assert_eq!(fwd.accepted(), 1);
    #[cfg(unix)]
    {
        let ForwarderAddr::Unix(p) = fwd.addr() else {
            panic!("expected a Unix socket, got {:?}", fwd.addr())
        };
        assert_eq!(mode(p) & 0o777, 0o600);
        assert_eq!(mode(p.parent().unwrap()) & 0o777, 0o700);
        assert!(p.starts_with(dir.path()));
    }
    #[cfg(not(unix))]
    assert!(matches!(fwd.addr(), ForwarderAddr::Tcp(a) if a.ip().is_loopback()));
    guard.run().await;
    assert!(connect_to_result(fwd.addr()).await.is_err());
    #[cfg(unix)]
    if let ForwarderAddr::Unix(p) = fwd.addr() {
        assert!(
            !p.parent().unwrap().exists(),
            "the private directory is removed"
        );
    }
}

#[tokio::test]
async fn tcp_mode_forwarder_is_loopback_tcp_and_accepts_once() {
    let dir = tempdir();
    let (fwd, _guard) =
        LoopbackForwarder::bind(echo_factory().await, dir.path(), 1, ForwarderMode::Tcp)
            .await
            .unwrap();
    let ForwarderAddr::Tcp(a) = fwd.addr() else {
        panic!("expected TCP, got {:?}", fwd.addr())
    };
    assert!(a.ip().is_loopback());
    assert_eq!(a.ip().to_string(), "127.0.0.1");
    let mut c = connect_to(fwd.addr()).await;
    c.write_all(b"ping").await.unwrap();
    assert_eq!(read4(&mut c).await, *b"ping");
    assert!(connect_to_result(fwd.addr()).await.is_err());
    assert_eq!(fwd.accepted(), 1);
    // The relay still works after the listener closed.
    c.write_all(b"pong").await.unwrap();
    assert_eq!(read4(&mut c).await, *b"pong");
}

#[tokio::test]
async fn each_accepted_connection_gets_the_next_open_in_order() {
    // The factory hands out streams tagged 0, 1, 2 in call order.
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let factory = StreamFactory::new(move || {
        let n = counter.fetch_add(1, Ordering::SeqCst) as u8;
        Box::pin(async move {
            let (a, mut b) = tokio::io::duplex(64);
            b.write_all(&[n]).await?;
            tokio::spawn(async move {
                // Keep the far end open until the reader is done.
                let mut sink = [0u8; 1];
                let _ = b.read(&mut sink).await;
            });
            Ok(Box::new(a) as BoxedStream)
        })
    });
    let dir = tempdir();
    let (fwd, guard) = LoopbackForwarder::bind(factory, dir.path(), 3, ForwarderMode::Tcp)
        .await
        .unwrap();
    for expected in 0u8..3 {
        let mut c = connect_to(fwd.addr()).await;
        let mut tag = [0u8; 1];
        c.read_exact(&mut tag).await.unwrap();
        assert_eq!(tag[0], expected);
    }
    assert_eq!(fwd.accepted(), 3);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert!(connect_to_result(fwd.addr()).await.is_err());
    guard.run().await;
}

#[tokio::test]
async fn a_failed_open_closes_the_accepted_connection() {
    let factory = StreamFactory::new(|| {
        Box::pin(async { Err(io::Error::from(io::ErrorKind::ConnectionRefused)) })
    });
    let dir = tempdir();
    let (fwd, guard) = LoopbackForwarder::bind(factory, dir.path(), 1, ForwarderMode::Tcp)
        .await
        .unwrap();
    let mut c = connect_to(fwd.addr()).await;
    let mut buf = [0u8; 1];
    let n = tokio::time::timeout(Duration::from_secs(2), c.read(&mut buf))
        .await
        .expect("the forwarder closes the connection instead of hanging");
    assert!(matches!(n, Ok(0) | Err(_)));
    assert_eq!(fwd.accepted(), 1);
    guard.run().await;
}

#[tokio::test]
async fn the_guard_tears_down_live_relays() {
    let dir = tempdir();
    let (fwd, guard) =
        LoopbackForwarder::bind(echo_factory().await, dir.path(), 1, ForwarderMode::Tcp)
            .await
            .unwrap();
    let mut c = connect_to(fwd.addr()).await;
    c.write_all(b"ping").await.unwrap();
    assert_eq!(read4(&mut c).await, *b"ping");
    guard.run().await;
    let mut buf = [0u8; 4];
    let n = tokio::time::timeout(Duration::from_secs(2), c.read(&mut buf))
        .await
        .expect("the relay is gone after the guard runs");
    assert!(matches!(n, Ok(0) | Err(_)));
}

#[cfg(unix)]
#[tokio::test]
async fn unix_mode_creates_the_private_directory_when_missing() {
    let dir = tempdir();
    let private = dir.path().join("private");
    let (fwd, guard) =
        LoopbackForwarder::bind(echo_factory().await, &private, 1, ForwarderMode::UnixOrTcp)
            .await
            .unwrap();
    assert_eq!(mode(&private) & 0o777, 0o700);
    let mut c = connect_to(fwd.addr()).await;
    c.write_all(b"ping").await.unwrap();
    assert_eq!(read4(&mut c).await, *b"ping");
    // After its last accept the socket file is gone.
    let ForwarderAddr::Unix(p) = fwd.addr() else {
        panic!("expected a Unix socket")
    };
    assert!(!p.exists());
    guard.run().await;
}

/// Waits up to two seconds for `addr` to refuse connections.
async fn eventually_refuses(addr: &ForwarderAddr) -> bool {
    for _ in 0..40 {
        if connect_to_result(addr).await.is_err() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test]
async fn dropping_the_guard_without_running_it_still_tears_down() {
    for forwarder_mode in [ForwarderMode::Tcp, ForwarderMode::UnixOrTcp] {
        let dir = tempdir();
        let (fwd, guard) =
            LoopbackForwarder::bind(echo_factory().await, dir.path(), 4, forwarder_mode)
                .await
                .unwrap();
        let mut c = connect_to(fwd.addr()).await;
        c.write_all(b"ping").await.unwrap();
        assert_eq!(read4(&mut c).await, *b"ping");

        drop(guard); // a cancelled connect never runs its guards
        if let ForwarderAddr::Unix(p) = fwd.addr() {
            assert!(
                !p.parent().unwrap().exists(),
                "the private directory is removed"
            );
        }
        assert!(
            eventually_refuses(fwd.addr()).await,
            "{forwarder_mode:?}: still accepting after the guard was dropped"
        );
        let mut buf = [0u8; 4];
        let n = tokio::time::timeout(Duration::from_secs(2), c.read(&mut buf))
            .await
            .expect("the relay is gone after the guard was dropped");
        assert!(matches!(n, Ok(0) | Err(_)), "{forwarder_mode:?}");
        assert_eq!(fwd.accepted(), 1);
    }
}

#[tokio::test]
async fn a_client_that_resets_at_once_does_not_kill_the_forwarder() {
    let dir = tempdir();
    // Two slots: whether the reset connection is caught before it uses one (its peer is
    // already gone at accept) or after (the reset is still in flight) is up to the kernel.
    // Either way the forwarder must go on serving.
    let (fwd, guard) =
        LoopbackForwarder::bind(echo_factory().await, dir.path(), 2, ForwarderMode::Tcp)
            .await
            .unwrap();
    let ForwarderAddr::Tcp(a) = fwd.addr() else {
        panic!("expected TCP")
    };
    // Connect and reset without yielding, before the forwarder's first accept.
    let rude = std::net::TcpStream::connect(a).unwrap();
    rude.set_nonblocking(true).unwrap();
    let rude = tokio::net::TcpStream::from_std(rude).unwrap();
    rude.set_zero_linger().unwrap();
    drop(rude);

    let mut c = connect_to(fwd.addr()).await;
    c.write_all(b"ping").await.unwrap();
    let mut buf = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(2), c.read_exact(&mut buf))
        .await
        .expect("the real client is served")
        .unwrap();
    assert_eq!(buf, *b"ping");
    assert!((1..=2).contains(&fwd.accepted()), "{}", fwd.accepted());
    guard.run().await;
}
