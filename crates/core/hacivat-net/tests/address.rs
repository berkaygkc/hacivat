//! Address classes (HV-SEC-02): which endpoints may see plaintext credentials.

use std::net::SocketAddr;

use hacivat_net::AddressClass::{Loopback, Private, Public};
use hacivat_net::{classify_host, classify_ip};

fn sa(s: &str) -> SocketAddr {
    s.parse().unwrap()
}

#[test]
fn address_classes() {
    for (ip, c) in [
        ("127.0.0.1", Loopback),
        ("::1", Loopback),
        ("10.1.2.3", Private),
        ("172.16.0.1", Private),
        ("172.32.0.1", Public),
        ("192.168.1.1", Private),
        ("fd00::1", Private),
        ("169.254.1.1", Private),
        ("100.64.0.1", Public),
        ("8.8.8.8", Public),
    ] {
        assert_eq!(classify_ip(ip.parse().unwrap()), c, "{ip}");
    }
    assert_eq!(classify_host("localhost", &[]), Loopback);
    assert_eq!(
        classify_host("db", &[sa("10.0.0.5:5432"), sa("8.8.8.8:5432")]),
        Public
    );
}

#[test]
fn v6_link_local_mapped_v4_and_global_v6() {
    for (ip, c) in [
        ("fe80::1", Private),
        ("fc00::5", Private),
        ("::ffff:127.0.0.1", Loopback),
        ("::ffff:10.0.0.1", Private),
        ("::ffff:8.8.8.8", Public),
        ("2001:4860:4860::8888", Public),
        ("127.255.255.254", Loopback),
        ("172.31.255.255", Private),
    ] {
        assert_eq!(classify_ip(ip.parse().unwrap()), c, "{ip}");
    }
}

#[test]
fn a_host_takes_its_least_private_address() {
    assert_eq!(
        classify_host("db", &[sa("127.0.0.1:5432"), sa("[::1]:5432")]),
        Loopback
    );
    assert_eq!(
        classify_host("db", &[sa("127.0.0.1:5432"), sa("192.168.0.9:5432")]),
        Private
    );
    // `localhost` is classified by its addresses once it has any.
    assert_eq!(classify_host("localhost", &[sa("8.8.8.8:5432")]), Public);
}

#[test]
fn an_unresolved_host_is_classified_by_its_name() {
    assert_eq!(classify_host("LOCALHOST", &[]), Loopback);
    assert_eq!(classify_host("app.localhost", &[]), Loopback);
    assert_eq!(classify_host("10.0.0.5", &[]), Private);
    assert_eq!(classify_host("[::1]", &[]), Loopback);
    assert_eq!(classify_host("db.acme.io", &[]), Public);
    assert_eq!(classify_host("notlocalhost", &[]), Public);
}
