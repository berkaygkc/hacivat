//! Request routing by department (HV-SESS-02 steps 1–7; Task 15).

use hacivat_proto::Department;
use hacivat_session::session::{ClientId, RouteRequest, RouteTarget, RoutingMode, route};

#[test]
fn route_table() {
    use RouteTarget::*;
    let r = |m, c, d, tx, iso, busy| {
        route(
            m,
            &RouteRequest {
                client: c,
                department: d,
                needs_tx_context: tx,
                isolated: iso,
            },
            busy,
        )
    };
    assert_eq!(
        r(
            RoutingMode::Single,
            None,
            Department::Introspection,
            false,
            false,
            false
        ),
        Default
    );
    assert_eq!(
        r(
            RoutingMode::Multiple,
            Some(ClientId(3)),
            Department::Console,
            false,
            false,
            false
        ),
        Client(ClientId(3))
    );
    assert_eq!(
        r(
            RoutingMode::Multiple,
            None,
            Department::Introspection,
            false,
            false,
            false
        ),
        Pooled
    );
    assert_eq!(
        r(
            RoutingMode::Multiple,
            None,
            Department::DataExport,
            false,
            false,
            false
        ),
        NewJob
    );
    assert_eq!(
        r(
            RoutingMode::Multiple,
            Some(ClientId(9)),
            Department::Mcp,
            false,
            false,
            false
        ),
        Mcp(ClientId(9))
    );
    assert_eq!(
        r(
            RoutingMode::Multiple,
            Some(ClientId(3)),
            Department::Console,
            false,
            true,
            true
        ),
        Pooled
    );
    assert_eq!(
        r(
            RoutingMode::Multiple,
            Some(ClientId(3)),
            Department::Console,
            true,
            true,
            true
        ),
        Client(ClientId(3))
    );
}

#[test]
fn every_background_department_is_pooled_and_every_job_department_is_new() {
    let r = |d| {
        route(
            RoutingMode::Multiple,
            &RouteRequest {
                client: None,
                department: d,
                needs_tx_context: false,
                isolated: false,
            },
            false,
        )
    };
    for d in [
        Department::Introspection,
        Department::QuickDoc,
        Department::CodeGeneration,
        Department::TextSearch,
        Department::Refactoring,
    ] {
        assert_eq!(r(d), RouteTarget::Pooled, "{d:?}");
    }
    for d in [Department::DataExport, Department::DataImport] {
        assert_eq!(r(d), RouteTarget::NewJob, "{d:?}");
    }
    for d in [
        Department::Console,
        Department::DataEditor,
        Department::Executor,
        Department::Unknown,
    ] {
        assert_eq!(r(d), RouteTarget::NewConsole, "{d:?}");
    }
}

#[test]
fn an_mcp_request_without_a_client_gets_the_reserved_client() {
    let target = route(
        RoutingMode::Multiple,
        &RouteRequest {
            client: None,
            department: Department::Mcp,
            needs_tx_context: false,
            isolated: false,
        },
        false,
    );
    assert_eq!(target, RouteTarget::Mcp(ClientId(u64::MAX)));
}

#[test]
fn an_attached_client_wins_over_its_department() {
    // Step 2 before step 3: a console's own introspection sees its transaction.
    let target = route(
        RoutingMode::Multiple,
        &RouteRequest {
            client: Some(ClientId(4)),
            department: Department::Introspection,
            needs_tx_context: false,
            isolated: false,
        },
        false,
    );
    assert_eq!(target, RouteTarget::Client(ClientId(4)));
}

#[test]
fn an_isolated_request_waits_for_an_idle_client() {
    // Step 7 needs the target busy: an idle client serves its own isolated request.
    let target = route(
        RoutingMode::Multiple,
        &RouteRequest {
            client: Some(ClientId(3)),
            department: Department::Console,
            needs_tx_context: false,
            isolated: true,
        },
        false,
    );
    assert_eq!(target, RouteTarget::Client(ClientId(3)));
}

#[test]
fn single_mode_never_borrows_a_pooled_session_but_mcp_never_shares() {
    let single = |c, d, iso, busy| {
        route(
            RoutingMode::Single,
            &RouteRequest {
                client: c,
                department: d,
                needs_tx_context: false,
                isolated: iso,
            },
            busy,
        )
    };
    // Single mode: everything through one session, even an isolated request on a busy one
    // (it queues behind the console's, HV-SESS-02).
    assert_eq!(
        single(Some(ClientId(3)), Department::Console, true, true),
        RouteTarget::Default
    );
    // MCP sessions never share with a user's console, in either mode (HV-SESS-02 edge case).
    assert_eq!(
        single(Some(ClientId(9)), Department::Mcp, false, false),
        RouteTarget::Mcp(ClientId(9))
    );
}
