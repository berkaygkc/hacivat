//! `ChaosProxy` against a loopback echo server (Task 16).

use std::net::SocketAddr;
use std::time::Duration;

use hacivat_testkit::chaos::ChaosProxy;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// An echo server; every connection echoes until EOF.
async fn echo() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 256];
                while let Ok(n) = s.read(&mut buf).await {
                    if n == 0 || s.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    addr
}

async fn connect(addr: SocketAddr) -> std::io::Result<TcpStream> {
    TcpStream::connect(addr).await
}

async fn roundtrip(c: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    c.write_all(b"ping").await?;
    let mut got = [0u8; 4];
    c.read_exact(&mut got).await?;
    Ok(got.to_vec())
}

async fn echo_via(addr: SocketAddr) -> Vec<u8> {
    let mut c = connect(addr).await.unwrap();
    timeout(Duration::from_secs(5), roundtrip(&mut c))
        .await
        .expect("echo timed out")
        .unwrap()
}

/// Whether the connection ends (EOF or error) soon.
async fn read_eof(mut c: TcpStream) -> bool {
    let mut b = [0u8; 8];
    matches!(
        timeout(Duration::from_secs(5), c.read(&mut b)).await,
        Ok(Ok(0) | Err(_))
    )
}

/// During a cut a new connection is refused, or closed without an echo.
async fn refused_soon(addr: SocketAddr) -> bool {
    match connect(addr).await {
        Err(_) => true,
        Ok(mut c) => roundtrip(&mut c).await.is_err(),
    }
}

#[tokio::test]
async fn chaos_proxy_relays_cuts_and_refuses() {
    let up = echo().await;
    let p = ChaosProxy::start(up).await;
    assert_eq!(echo_via(p.addr).await, b"ping");
    assert_eq!(p.accepted(), 1);
    let held = connect(p.addr).await.unwrap();
    p.cut(ms(600));
    assert!(read_eof(held).await);
    assert!(refused_soon(p.addr).await);
    tokio::time::sleep(ms(700)).await;
    assert_eq!(echo_via(p.addr).await, b"ping");
}

#[tokio::test]
async fn blackhole_stalls_without_closing() {
    let p = ChaosProxy::start(echo().await).await;
    let mut c = connect(p.addr).await.unwrap();
    assert_eq!(roundtrip(&mut c).await.unwrap(), b"ping");
    p.blackhole(true);
    assert!(timeout(ms(200), roundtrip(&mut c)).await.is_err());
    // Still open: once the blackhole ends, the same connection echoes again.
    p.blackhole(false);
    assert_eq!(roundtrip(&mut c).await.unwrap(), b"ping");
}

#[tokio::test]
async fn restore_ends_a_cut_early() {
    let p = ChaosProxy::start(echo().await).await;
    p.cut(Duration::from_secs(60));
    assert!(refused_soon(p.addr).await);
    p.restore();
    assert_eq!(echo_via(p.addr).await, b"ping");
}

#[tokio::test]
async fn accepted_counts_relayed_connections() {
    let p = ChaosProxy::start(echo().await).await;
    assert_eq!(p.accepted(), 0);
    echo_via(p.addr).await;
    echo_via(p.addr).await;
    assert_eq!(p.accepted(), 2);
}
