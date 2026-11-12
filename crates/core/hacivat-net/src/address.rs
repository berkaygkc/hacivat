//! Address classes: decide whether plaintext credentials may cross the network (HV-SEC-02).

use std::net::{IpAddr, SocketAddr};

/// Where an endpoint lives, from the client's point of view (HV-SEC-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AddressClass {
    Loopback,
    Private,
    Public,
    UnixSocket,
}

/// The class of one address. Private means RFC 1918, RFC 4193 (`fc00::/7`) and link-local
/// (`169.254/16`, `fe80::/10`). CGNAT (`100.64/10`) is Public: it is the carrier's network,
/// not the user's. IPv4-mapped IPv6 addresses are classified as their IPv4 address.
pub fn classify_ip(ip: IpAddr) -> AddressClass {
    match ip.to_canonical() {
        IpAddr::V4(v4) if v4.is_loopback() => AddressClass::Loopback,
        IpAddr::V4(v4) if v4.is_private() || v4.is_link_local() => AddressClass::Private,
        IpAddr::V6(v6) if v6.is_loopback() => AddressClass::Loopback,
        IpAddr::V6(v6) if v6.is_unique_local() || v6.is_unicast_link_local() => {
            AddressClass::Private
        }
        _ => AddressClass::Public,
    }
}

/// The class of a host. With resolved addresses it is the least private of them, so a host is
/// Public if any address is public. Without any, an IP literal is classified as itself,
/// `localhost` (and `*.localhost`, RFC 6761) is Loopback, and any other name is Public.
pub fn classify_host(host: &str, resolved: &[SocketAddr]) -> AddressClass {
    if resolved.is_empty() {
        return match parse_ip_literal(host) {
            Some(ip) => classify_ip(ip),
            None if is_localhost_name(host) => AddressClass::Loopback,
            None => AddressClass::Public,
        };
    }
    resolved
        .iter()
        .map(|a| classify_ip(a.ip()))
        .max_by_key(|c| exposure(*c))
        .unwrap_or(AddressClass::Public)
}

/// How far an address class reaches; a host takes its widest address.
fn exposure(class: AddressClass) -> u8 {
    match class {
        AddressClass::UnixSocket | AddressClass::Loopback => 0,
        AddressClass::Private => 1,
        AddressClass::Public => 2,
    }
}

fn is_localhost_name(host: &str) -> bool {
    let host = host.strip_suffix('.').unwrap_or(host);
    let suffix = ".localhost";
    host.eq_ignore_ascii_case("localhost")
        || (host.len() > suffix.len()
            && host.is_char_boundary(host.len() - suffix.len())
            && host[host.len() - suffix.len()..].eq_ignore_ascii_case(suffix))
}

/// An IP literal, with or without IPv6 brackets (`[::1]`).
pub(crate) fn parse_ip_literal(host: &str) -> Option<IpAddr> {
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    bare.parse().ok()
}
