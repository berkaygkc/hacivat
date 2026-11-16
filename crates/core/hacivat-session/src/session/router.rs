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

/// The client every MCP request without its own client is routed to.
const MCP_UNATTACHED: ClientId = ClientId(u64::MAX);

/// The session of `req` in `mode`; `target_busy` says the preferred session is busy
/// (HV-SESS-02 steps 1–7):
/// 1. Single mode → `Default`;
/// 2. a client is attached → `Client`;
/// 3. background lookups (`Introspection`, `QuickDoc`, `CodeGeneration`, `TextSearch`,
///    `Refactoring`) → `Pooled`;
/// 4. `DataExport` and `DataImport` → `NewJob`;
/// 5. `Mcp` → `Mcp(client or ClientId(u64::MAX))`;
/// 6. anything else → `NewConsole`;
/// 7. an `isolated` request whose client's session is busy and that needs no transaction
///    context → `Pooled` (DataGrip's temporary client); one that needs it queues.
///
/// MCP is checked before step 1: an MCP client never shares a user's session, in either mode
/// (HV-SESS-02 edge case). Step 7 borrows only for a client's session: in Single mode the
/// request queues behind the shared session (it must see the same temporary objects), and a
/// session that is new is never busy.
pub fn route(mode: RoutingMode, req: &RouteRequest, target_busy: bool) -> RouteTarget {
    if req.department == Department::Mcp {
        return RouteTarget::Mcp(req.client.unwrap_or(MCP_UNATTACHED));
    }
    if mode == RoutingMode::Single {
        return RouteTarget::Default;
    }
    if let Some(client) = req.client {
        if req.isolated && target_busy && !req.needs_tx_context {
            return RouteTarget::Pooled;
        }
        return RouteTarget::Client(client);
    }
    match req.department {
        Department::Introspection
        | Department::QuickDoc
        | Department::CodeGeneration
        | Department::TextSearch
        | Department::Refactoring => RouteTarget::Pooled,
        Department::DataExport | Department::DataImport => RouteTarget::NewJob,
        _ => RouteTarget::NewConsole,
    }
}
