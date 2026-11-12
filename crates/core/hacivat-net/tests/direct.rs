//! Direct transport (HV-CONN-01): resolution and Happy Eyeballs connects over loopback only.

use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use hacivat_net::{BoxedStream, NetError, Target, direct, resolve};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A loopback echo server; returns its address.
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

/// A loopback port nothing listens on.
fn closed_port() -> SocketAddr {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap()
}

async fn read4(s: &mut BoxedStream) -> [u8; 4] {
    let mut buf = [0u8; 4];
    s.read_exact(&mut buf).await.unwrap();
    buf
}

fn open_err(r: io::Result<BoxedStream>) -> io::Error {
    match r {
        Ok(_) => panic!("expected the connect to fail"),
        Err(e) => e,
    }
}

#[tokio::test]
async fn direct_connects_and_falls_back_to_the_next_address() {
    let (open, closed) = (echo_server().await, closed_port());
    let f = direct(
        Target::Tcp {
            host: "localhost".into(),
            port: open.port(),
        },
        vec![closed, open],
        Duration::from_secs(2),
    );
    let mut s = f.open().await.unwrap();
    s.write_all(b"ping").await.unwrap();
    assert_eq!(read4(&mut s).await, *b"ping");
}

#[tokio::test]
async fn refused_and_dns_errors() {
    assert!(matches!(
        resolve("nonexistent.invalid", 5432).await,
        Err(NetError::Dns { .. })
    ));
    let f = direct(
        Target::Tcp {
            host: "localhost".into(),
            port: 1,
        },
        vec![closed_port()],
        Duration::from_secs(1),
    );
    assert_eq!(
        open_err(f.open().await).kind(),
        io::ErrorKind::ConnectionRefused
    );
}

#[tokio::test]
async fn ip_literals_resolve_to_themselves() {
    assert_eq!(
        resolve("127.0.0.1", 5432).await.unwrap(),
        vec!["127.0.0.1:5432".parse::<SocketAddr>().unwrap()]
    );
    assert_eq!(
        resolve("::1", 3306).await.unwrap(),
        vec!["[::1]:3306".parse::<SocketAddr>().unwrap()]
    );
    assert_eq!(
        resolve("[::1]", 3306).await.unwrap(),
        vec!["[::1]:3306".parse::<SocketAddr>().unwrap()]
    );
    let dns = resolve("nonexistent.invalid", 1).await.unwrap_err();
    assert_eq!(
        dns,
        NetError::Dns {
            host: "nonexistent.invalid".into()
        }
    );
}

#[tokio::test]
async fn localhost_resolves_to_loopback_addresses_with_the_port() {
    let addrs = resolve("localhost", 5433).await.unwrap();
    assert!(!addrs.is_empty());
    assert!(
        addrs
            .iter()
            .all(|a| a.ip().is_loopback() && a.port() == 5433),
        "{addrs:?}"
    );
}

#[tokio::test]
async fn every_open_is_a_new_socket() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let f = direct(
        Target::Tcp {
            host: "127.0.0.1".into(),
            port: addr.port(),
        },
        vec![addr],
        Duration::from_secs(2),
    );
    let _a = f.open().await.unwrap();
    let (_, peer_a) = listener.accept().await.unwrap();
    let _b = f.open().await.unwrap();
    let (_, peer_b) = listener.accept().await.unwrap();
    assert_ne!(peer_a.port(), peer_b.port());
}

#[tokio::test]
async fn no_address_is_an_error_not_a_hang() {
    let f = direct(
        Target::Tcp {
            host: "db".into(),
            port: 5432,
        },
        vec![],
        Duration::from_secs(1),
    );
    assert_eq!(open_err(f.open().await).kind(), io::ErrorKind::InvalidInput);
}

#[cfg(unix)]
#[tokio::test]
async fn unix_targets_connect_to_the_socket_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pg.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        s.write_all(b"unix").await.unwrap();
    });
    let f = direct(Target::Unix { path }, vec![], Duration::from_secs(2));
    let mut s = f.open().await.unwrap();
    assert_eq!(read4(&mut s).await, *b"unix");

    let missing = direct(
        Target::Unix {
            path: dir.path().join("missing.sock"),
        },
        vec![],
        Duration::from_secs(2),
    );
    assert_eq!(
        open_err(missing.open().await).kind(),
        io::ErrorKind::NotFound
    );
}
