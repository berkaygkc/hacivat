//! Request routing (HV-SESS-02, D-035; Task 15): which session a request goes to.

use hacivat_proto::Department;

use crate::session::ClientId;

/// A profile's session mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoutingMode {
    Multiple,
    /// Every request through one session (`Profile::single_session`, SQLite).
    Single,
}

/// What a request needs from its session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RouteRequest {
    pub client: Option<ClientId>,
    pub department: Department,
    /// It must see the client's open transaction.
    pub needs_tx_context: bool,
    /// It must not share a session (a job).
    pub isolated: bool,
}

/// Where a request goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RouteTarget {
    Default,
    Client(ClientId),
    Pooled,
    NewJob,
    Mcp(ClientId),
    NewConsole,
}

/// The session of `req` in `mode`; `target_busy` says the preferred session is busy. Task 1
/// stub: the default session (Task 15).
pub fn route(_mode: RoutingMode, _req: &RouteRequest, _target_busy: bool) -> RouteTarget {
    RouteTarget::Default
}
