//! Address classes: decide whether plaintext credentials may cross the network (HV-SEC-02).
//! Bodies in Task 5.

use std::net::{IpAddr, SocketAddr};

/// Where an endpoint lives, from the client's point of view (HV-SEC-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AddressClass {
    Loopback,
    Private,
    Public,
    UnixSocket,
}

/// The class of one address (Task 5). The stub answers `Public`, the strictest class.
pub fn classify_ip(_ip: IpAddr) -> AddressClass {
    AddressClass::Public
}

/// The class of a host: Public if any resolved address is public (Task 5). The stub answers
/// `Public`.
pub fn classify_host(_host: &str, _resolved: &[SocketAddr]) -> AddressClass {
    AddressClass::Public
}
