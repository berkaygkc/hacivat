# DataGrip 2024.3.5 — sessions, connections, transactions and the execution engine

> Covers: the session/connection/client model and its state machines, routing (single vs multiple sessions), departments, transaction modes and isolation, commit/rollback/savepoints, cancellation and its escalation, keep-alive and auto-disconnect timers, read-only enforcement (driver-level + client-side statement classification), startup scripts, session templates, schema/search-path switching, time zones, DBMS_OUTPUT, the statement execution layer and pre-execution validators.
> Out of scope (other doc set): result-grid editing UX, query-history UI, explain plans. Conventions: `00-method-and-legal.md`.

---

## 1. Scope and sources

| Source | Version | Licence |
|---|---|---|
| Shipped help: `managing-connection-sessions` (title "Sessions"), `configuring-database-connections` ("Configure connection"), `data-sources-and-drivers-dialog`, `run-a-query`, `query-execution`, `schemas`, `services-tool-window`, `submitting-and-reverting-changes` | DataGrip 2024.3.5 offline help (2024-10) | Proprietary docs |
| `messages/DatabaseBundle.properties` (transaction, session, cancel, read-only, department keys) | 2024.3.5 | Proprietary |
| Class-name inventory: `console/session/DatabaseSessionManager$*`, `console/JdbcEngine$*`, `dataSource/connection/statements/*`, `datagrid/DataRequest$*`, `console/DatabaseAccessController$*`, `console/KeepAliveActionDescriptor`, `console/AutoDisconnectActionDescriptor`, `dataSource/connection/DGDepartment$*`, `*TimeZoneManager` | 2024.3.5 | names only |
| Descriptors: `DatabaseRegistryKeys.xml`, `sqlEffectAnalyzer` and `queryValidator` registrations, `executionEnvironmentHelper` | 2024.3.5 | Proprietary |
| `DataGrip/redis-jdbc-driver@v1.5` (blocking commands, cluster restrictions) | GitHub | Apache-2.0 |

---

## 2. Executive summary

1. **Session** = "a wrapper over a single connection" that stores its state (active or not, transaction mode, other settings); connections inside a session open and close as needed; sessions have **clients** (files/consoles, data editors) `[Observed: help:managing-connection-sessions]`.
2. Since **2024.1 sessions are managed automatically**: opening a console, a table, or attaching a file creates a session; consoles connect lazily on first DB action, tables connect immediately `[Observed: help:managing-connection-sessions]`.
3. Two routing modes: **multiple sessions** (default; "each new query console creates a new connection") and **single session mode** ("the data source and all consoles use one and the same session" — temp objects visible in the tree, one transaction across consoles; used e.g. when every connection requires MFA) `[Observed: help]`. Internally `SingleSessionRouter` / `MultipleSessionsRouter` `[Observed: classes]`.
4. Every request carries a **department** (console, data editor, procedure, function, executor, run configuration runner, data import, introspection, quick documentation, code generation, refactoring, data export, text search, debugger, test runner, unknown) `[Observed: bundle:department.*]` — used for routing, logging and auditing (`SessionAuditService`, `EngineAuditService`).
5. **Transaction control**: `Auto` (auto-commit) or `Manual`, plus isolation (Database Default, Read Uncommitted, Read Committed, Repeatable Read, Serializable), per data source default and per session switch (Tx widget) `[Observed: help, bundle:transaction.mode.*]`. Closing a connection with an open manual transaction asks *"Current connection will be closed. Commit current transaction?"* `[Observed: bundle:JdbcConsole.transaction.message]`.
6. **Cancellation**: "Cancel running and queued statements"; a red square in the gutter while cancelling; pressing Cancel **twice** offers to *deactivate* (terminate the remote process / close all connections) or keep waiting; a warning also appears after **10 seconds** of idle cancelling; *Deactivate* stops the query client-side but "the query will continue its execution in the database"; connection establishment itself is cancellable (*Connection canceled*) `[Observed: help:run-a-query; bundle:deactivate.data.source.*]`.
7. **Keep-alive** ("Run keep-alive query each N seconds"; per-driver custom keep-alive query) and **auto-disconnect** ("Disconnect … after the specified number of seconds") are per-data-source timers (`KeepAliveActionDescriptor`, `AutoDisconnectActionDescriptor`) `[Observed]`.
8. **Read-only** has two layers: the driver read-only flag ("Data modifications might be possible in the query console if the driver does not support the read-only status") and a **client-side PSI-based classifier** that blocks DML, DDL, routine calls with side effects, and other side-effecting statements with distinct messages `[Observed: help; bundle:database.access.{dml,ddl,routine,common}; classes: DatabaseAccessController$PsiBasedReadOnly]`.
9. **Startup script** runs on every new connection (supports IDE macros); **session templates** override auth, read-only, startup script, driver and URL parameters per session `[Observed: help; classes: DataSourceSessionTemplatePanel$*Widget]`.
10. **Schema switching**: Automatic / Manual / Disable; PostgreSQL/Redshift support an ordered **search path** chooser and Automatic restores it across restarts `[Observed: help:schemas, data-sources-and-drivers-dialog]`.
11. **Time zone** per data source applied via per-DBMS `TimeZoneManager`s (PG, MySQL-base, Oracle, Exasol, HSQL, Vertica) `[Observed: classes]`.
12. The execution core (`JdbcEngine`) distinguishes query, update (with an *unexpected updates guard*), call, transaction (commit/rollback/rollback-to-savepoint), *silent* (internal) and *raw* operations, *disposal rollback* when a session dies, *atomic* and *potentially transparent* operations; statements are "smart" wrappers (one-shot vs reusable; simple/parameterized/callable; batch inserts) `[Observed: classes]`.

---

## 3. Detailed findings

### 3.1 Object model: data source → session → connection → client

```
DataSource (config)                                   [1]
  └── Session (named, e.g. "console", "console_2")    [0..n]   (single-session mode: exactly 1 default)
        ├── state: disconnected | connecting | connected | cancelling | broken
        ├── tx: mode (Auto/Manual), isolation, in-transaction flag
        ├── env: current database/schema/search path, time zone, read-only, DBMS_OUTPUT
        ├── Connection (physical, via driver process)  [0..1 active at a time]
        └── Clients: console file, SQL file, data editor (grid), run-config, routine runner
```
Evidence: help text (sessions wrap a single connection; clients are files and data editors; green dot = connected session; sessions persist across IDE restarts *but empty* — "all your sessions will be available in the Services tool window, but they will be empty") `[Observed: help:managing-connection-sessions]`; classes `DatabaseSessionManager$SessionType`, `$SingleConnection`, `$MultiConnection`, `$TemporaryClient`, `console/client/{DatabaseSessionClient, DatabaseSessionClientWithFile, VisibleDatabaseSessionClient, ScriptingSessionClient}`, `DataGridSessionClient` `[Observed]`; `GlobalSessionStateListener$SavedSession` + `$SessionSearchStrategy{CONSOLE,REGULAR}` → sessions are saved and re-found by strategy at startup `[Observed names; behaviour Inferred]`.

User actions on sessions `[Observed: help + bundle]`: New (implicit), Switch Session (for a file), Attach Session / Detach Session, Rename Session (Shift+F6), Close Session ("detached from its session clients, … disappear"), Close All Sessions, Open in New Tab, Deactivate (Ctrl+F2 = close connections of the data source). Confirmations: *"Detach '{0}' from '{1}' session?"*, *"Detach {0} clients from … sessions?"*, *"Close '{0}' session?"*; notification *"session changed from '{0}' to '{1}'"*.

`MultiConnection` vs `SingleConnection` session types `[Observed names]` → `[Inferred]` a session normally owns one connection, but tooling (e.g. the data editor reading metadata while a console runs) may borrow additional connections; `DataRequest$IsolatedQueryRequest` and `$CrossSessionOwner` `[Observed]` indicate requests that deliberately bypass the client's session (e.g. quick-doc lookups must not block on a long-running console statement).

### 3.2 Session state machine

`[Inferred from help + bundle + class names; DataGrip does not publish a formal machine]`

```
            open client / first action
 ┌───────────┐  ───────────────────────────►  ┌────────────┐
 │Disconnected│                               │ Connecting │──cancel──► Disconnected ("Connection canceled")
 └───────────┘ ◄──── connect error ─────────── └────────────┘
      ▲   ▲                                         │ success (+startup script, env)
      │   │                                         ▼
      │   │  auto-disconnect timer / Deactivate ┌────────────┐  execute ┌─────────┐
      │   └──────────────(tx? ask commit)──────│ Connected  │─────────►│ Running │
      │                                         │ idle       │◄─────────│         │
      │                                         └────────────┘  done    └─────────┘
      │                                              ▲    │               │ cancel
      │                 keep-alive ok                │    │ keep-alive    ▼
      │                                              │    │ fails     ┌────────────┐
      │                                              │    ▼           │ Cancelling │──ok──► Connected
      │                                         ┌────────────┐        └────────────┘
      └─────────────── close/cleanup ───────────│  Broken    │◄── 2nd cancel → Deactivate (escalation)
                                                └────────────┘    or remote "session closed"
```
- *Broken → Connecting* happens lazily on the next request (reconnect) and re-runs startup script & environment; `JdbcEngine$ReconnectErrorBuilder`, `UnusableConnectionException` `[Observed]` → reconnect is explicit and reported.
- Transaction sub-state while Connected: `Idle(no tx)` → `InTx` (manual mode after first statement) → commit/rollback → `Idle`; `Failed` when the DBMS aborted the tx (PG "current transaction is aborted") `[Inferred]`. `DisposalRollbackRequest` → if a session is disposed with an open tx the engine rolls back `[Observed name]`.

### 3.3 Routing and departments

- Router chooses the target connection for a request from `(data source, session, department, client)` `[Observed: classes DatabaseSessionManager$Facade$ConnectionsRouter, $Routers, $SingleSessionRouter, $MultipleSessionsRouter, $ConnectionReference, $ConnectionInfo, $Lockpick, $Runner]`.
- `Lockpick` `[Observed name]` → `[Inferred]` the mechanism to obtain a connection that is currently busy/locked by a long operation (e.g. to run a cancel or a quick metadata query), or to break a stuck lock on user request.
- `ConnectionRequestor` (+ `Anonymous`) and `DatabaseDepartment`/`DGDepartment` (`Console`, `DataEditor`, `QUICK_DOCUMENTATION`, `REFACTORING`, `RoutineRunner`, `RunConfigurationRunner`) `[Observed]`. Introspection uses its own department ("introspection") and **its own connection** unless in single-session mode `[Inferred: help says single-session mode makes temporary objects visible in the database tree — which only works if introspection uses the shared session]`.
- Single session mode is enabled per DS; applying it asks whether to disconnect existing sessions (No: keep; Yes: disconnect; Yes for All) `[Observed: help]`.

### 3.4 Transactions

- Modes `Auto` / `Manual`; isolation `Database Default`, `Read Uncommitted`, `Read Committed`, `Repeatable Read`, `Serializable` `[Observed: bundle:transaction.mode.*]`. Help: "In the Tx:Auto mode, DataGrip commits all the changes automatically once you click Submit"; in Manual, changes accumulate until *Submit and Commit*, Commit, or Roll Back; buttons hidden in Auto `[Observed: help:configuring-database-connections]`.
- DS default applies to **new** consoles; the Tx switch in toolbar/Services changes the current session `[Observed]`. Notifications: "{mode} transaction mode ON", "transaction mode can not be changed", "transaction isolation level switched to {0}", "…can not be switched", "transaction committed/rolled back" `[Observed: bundle]`.
- Guards: *"Failed to {0}. The transaction is in progress "{1}". Commit or roll back it first and try again"* (e.g. switching mode or isolation mid-transaction) `[Observed: bundle:dialog.message.failed.to.transaction.in.progress…]`; *"Locked by transaction: {0}"*; *"Failed to roll back to a savepoint."*; *"Roll back the current transaction to undo changes."*; *"Cannot undo/redo database modification"* `[Observed]`.
- Data editor in Manual mode submits DML into the session transaction; Auto commits each submit `[Observed: help]`. Option "Submit changes immediately" (Data Editor settings) `[Observed: help]`.
- `UnsafeUpdateRolledBackErrorInfo` + `UpdateOperation$UnexpectedUpdatesGuard` `[Observed]` → `[Inferred]` when a grid UPDATE/DELETE affects a different number of rows than expected (e.g. table without PK, duplicates), the engine rolls back that change and reports it.
- Oracle drivers default `auto-commit=false` (opt in driver XML) `[Observed]`.
- Savepoints: `RemoteSavepoint`, `JdbcEngine$OperationBase$RollbackToSavepoint` `[Observed]` → `[Inferred]` used internally to make multi-statement grid submits atomic inside a user transaction (rollback to savepoint on failure without losing earlier user work).

### 3.5 Cancellation

`[Observed: help:run-a-query; bundle; classes]`

| Step | Trigger | Behaviour |
|---|---|---|
| 1 | *Cancel Running Statements* (Ctrl+F2 in Services/console) | "Cancel running and queued statements" — queued statements of that session are dropped; running statement gets a driver-level cancel (`Statement.cancel()` → PG CancelRequest, MSSQL attention, MySQL `KILL QUERY`, Oracle break) `[Inferred mechanisms]`; gutter shows red square; status "Cancelling…" |
| 2 | 10 s without completion | notification *"Cancelling of running statements takes too long. You can keep waiting or deactivate the data source with all the connections."* with *Deactivate* link |
| 3 | Second click on Cancel | dialog *"Cancelling of Running Statements Failed. Deactivate the Data Source?"* buttons Deactivate / **Keep Waiting** |
| 4 | Deactivate | closes all connections of the data source (and can terminate the driver process) — the server-side query may continue |
| — | Cancel during connect | aborts connection creation → *"Connection canceled"* |

Notification group "Database cancellation of running statements" `[Observed: DatabaseConnectivity.xml]`; progress title *"Cancel running statements in {0} [{1}]"*; `CancellingDelegate`, `DataRequest$Cancel` `[Observed]`. Redis driver marks blocking commands (`BLPOP`, `BRPOP`, `BLMOVE`, `BLMPOP`, `BRPOPLPUSH`, `BZMPOP`, `BZPOPMIN`, `BZPOPMAX`) and executes them with a separate blocking socket timeout (`blockingSocketTimeout`, default 0 = infinite) `[Observed: oss-redis Analyzer.java, README@v1.5]` → cancelling those relies on closing the connection.

### 3.6 Keep-alive and auto-disconnect

- Per DS: "Run keep-alive query each N" (seconds), "Auto-disconnect after N" (seconds) `[Observed: help + bundle:data.source.keep.alive.label, data.source.auto.disconnect.label]`.
- Keep-alive query: default per DBMS (e.g. `SELECT 1` family) with driver override field *Keep-alive query* "for unsupported databases"; Tibero ships one in its driver XML; Test Connection reports keep-alive failures separately `[Observed]`.
- Implementation names: `KeepAliveActionDescriptor$KeepAliveAction`, `AutoDisconnectActionDescriptor$AutoDisconnectAction` `[Observed]` → `[Inferred]` both are scheduled "action descriptors" in a shared per-connection timer system.
- `[Inferred]` Auto-disconnect timing counts from the last activity; with an open manual transaction the user is asked to commit (same message as §3.4).
- SSH keep-alive is separate (SSH config "Send keep-alive messages every N seconds") `[Observed]`.
- `[Unknown]` default values (UI defaults not in resources).

### 3.7 Read-only enforcement

`[Observed: help:data-sources-and-drivers-dialog; bundle:database.access.*; classes]`

| Layer | Mechanism | Coverage |
|---|---|---|
| Driver/server | `Connection.setReadOnly(true)` → PG `SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY`, MySQL `SET SESSION TRANSACTION READ ONLY`, Oracle `SET TRANSACTION READ ONLY` (per tx), SQL Server: none (ApplicationIntent=ReadOnly is routing only) `[Inferred: JDBC driver behaviour]` | Depends on driver; docs warn modifications might still be possible |
| Grid | Data editor refuses edits: "Table is read-only. {0}", "Connection is read-only" `[Observed]`; `ReadOnlyDatabaseGridDataHookUp` | All |
| Console (client-side) | `DatabaseAccessController$PsiBasedReadOnly` parses each statement (SQL PSI / MongoJS / Redis) and classifies; messages: "…to execute a statement that leads to modification of data during a read-only connection" (DML), "…a DDL statement…" (DDL), "…a routine that may cause side effects…" (routine), "…a statement that may cause side effects…" (other) | Language-aware; Mongo uses `[pure]` signatures; Redis uses its DDL/upsert statement classes |
| MySQL schema switch | allowed in read-only (`connection-bounds in-read-only="true"`) | |

The `%s` prefix of the messages suggests a configurable verb ("Unable"/"Confirm") `[Inferred]`; `DatabaseAccessController$Token` `[Observed]` → an access token granting one-shot permission (e.g. user confirmed) `[Inferred]`.

### 3.8 Pre-execution validators

`queryValidator` EP with `SqlWithoutWhereInspection` (warning for `UPDATE`/`DELETE` without `WHERE`, user can *Execute* anyway — help "Show warning before running potentially unsafe queries"), `SqlJoinWithoutOnInspection`, `SqlTypeInspection` `[Observed: plugin.xml; help:query-execution]`.

### 3.9 Session environment: startup script, templates, schema, time zone, DBMS_OUTPUT

- **Startup script**: SQL run "each time you establish a connection", with *Insert Macros* (IDE macros) `[Observed: help]`. Because consoles get their own connections, it runs per connection.
- **Session templates**: named overrides of Authentication, Read-only, Startup script (init script), Driver, URL parameter overrides; selected via *Use session template* `[Observed: help; classes DataSourceSessionTemplatePanel$AuthWidget/InitScriptWidget/ReadonlyWidget/UrlOverridesWidget/UrlParamOverrideWidget/NameWidget]`. Use case: same server, "read-only analyst" vs "admin" sessions.
- **Schema switching** (`schema-control` driver option; UI *Switch schema*): Automatic (switch when console context changes; restore previous search path/current schema between restarts), Manual, Disable `[Observed]`. Per-console context selector `<schema>`; PG/Redshift multi-schema **search path** editor (Space to add/remove, Alt+Up/Down reorder) `[Observed: help:schemas]`. Classes `SearchPathSender`/`SearchPathReceiver`, `DataRequest$SchemaSwitchRequest`, `$CustomSearchPathOwner` `[Observed]` → `[Inferred]` the engine tracks the *server-side* current schema (receiver: parses `SET search_path`/`USE` executed by the user) and re-sends the intended path when needed (sender).
- Registry `database.query.generation.omit.schema.qualifier` — "Remove schema qualification from generated queries. Leads to additional search path requests" `[Observed]`.
- **Single database mode**: only the configured database is visible/accessible (PgBouncer-style setups) `[Observed]`.
- **Time zone**: DS option *Time zone*; per-DBMS managers PG/MySQL-base/Oracle/Exasol/HSQL/Vertica `[Observed]` → `[Inferred]` PG `SET TIME ZONE`, MySQL `SET time_zone`, Oracle `ALTER SESSION SET TIME_ZONE`; display time zone for values is a separate Data Views setting ("Display temporal data in time zone") `[Observed: bundle:time.format.time.zone]`.
- **DBMS_OUTPUT** (Oracle, Db2 LUW): enabled by default for new sessions if the option is set; console toggle *Enable DBMS_OUTPUT* `[Observed]`.
- ClickHouse **session_id** auto-assignment so `USE`/`SET` persist across HTTP calls `[Observed: bundle:automatically.assign.session.id.desc]`.

### 3.10 Execution layer

`[Observed: classes; registry keys]`

| Concept | Names | Meaning |
|---|---|---|
| Operations | `QueryExecutionOperation`, `UpdateOperation(+UnexpectedUpdatesGuard)`, `CallOperation`, `TransactionOperation`, `OperationBase$CommitOrRollback/Rollback/RollbackToSavepoint/Inactive`, `SilentOperation`, `RawOperation`, `AtomicOperation`, `PotentiallyAtomicOperation`, `PotentiallyTransparentOperation`, `DisposalRollbackRequest` | Typed units of work; *silent* = internal queries not shown in console log; *transparent* = does not change tx state (e.g. SELECT in auto mode) `[Inferred]` |
| Executors | `SimpleExecutor`, `ParameterizedExecutor`, `RegularExecutor`, `CallExecutor` | |
| Paging | `LastPageHandler$RowsCyclicBuffer` | keep only the last N rows (fetch-to-end without unbounded memory) `[Inferred]` |
| Statements | `SmartStatement` (+`OneShot`, `Reusable*`, `Simple`, `Parameterized`, `Callable`, `BatchInsertsStatement`, `NoisyStatement`, `BarrenStatement`), `ExecutionMode`, `StagedException$Stage` | statement wrappers that remember purpose/department (`SmartStatements$ConnectionWithDepartment`, `$PurposedConnection`) |
| Requests | `DataRequest$QueryRequest/RawQueryRequest/IsolatedQueryRequest/ConditionQueryRequest/CallRequest/DmlRequest/InsertRequest/UpdateRequest/DeleteRequest/TxRequest/TxCommand/TxMarker/SchemaSwitchRequest/Disconnect/Cancel/CompositeRequest` | grid ↔ engine protocol |
| Printing | `StatementPrinter$PlaceholdersExtractor` | renders statements with parameter values for the log |

Registry defaults `[Observed: DatabaseRegistryKeys.xml]`: `database.scrollable.result.sets=true` ("if JDBC driver reports it supports them"), `database.result.set.limit=100` (max result sets per query), `database.grid.paging.offset=true` (LIMIT/OFFSET paging), `database.batch.size=2000`, `database.disable.escape.processing=true` (JDBC `{fn}` escapes off → SQL sent verbatim), `database.generic_dialect.limit=true`.

Execution scope UX (detailed in the editor doc set): caret outside statement → Nothing/Whole script/Everything below; selection executed "as one statement" or "exactly as separate statements"; per-statement results tabs `[Observed: help:query-execution]`.

Auditing: `SessionAuditService`, `JdbcEngine$EngineAuditService`, `DatabaseSessionLogView` (per-session log in Services) `[Observed]`.

---

## 4. Algorithm specifications

### ALG-SESS-01: Session routing

- **Problem:** Map a request `(ds, client, department)` to a connection, honouring single/multiple session modes, isolation of background work, and lazy connect.
- **Inputs / outputs:** `Request{ds_id, client_id?, department, needs_tx_context: bool}` → `ConnHandle` (actor mailbox).
- **Algorithm:**
  1. If `ds.single_session` → `session = ds.default_session` (create if absent). Else if `client_id` attached → its session. Else if department ∈ {introspection, quick_doc, code_gen, text_search} → `ds.service_session(department)` (hidden, reusable, never shares tx with consoles). Else create a new visible session named after the client (`console`, `console_2`, …).
  2. If `request.isolated` (must not wait behind a running console statement) and the session is busy → borrow a temporary connection (`TemporaryClient`) with the same env but **no** tx context; forbidden if `needs_tx_context` (then queue).
  3. If session state is `Disconnected|Broken` → connect (ALG-CONN-01) then apply env (ALG-SESS-05).
  4. Enqueue the request into the session's FIFO; the session actor executes one statement at a time.
- **Data structures:** `Session { id, name, state, tx: TxState, env: SessionEnv, queue: VecDeque<Req>, clients: HashSet<ClientId>, conn: Option<ConnActor> }`; `Ds { sessions: IndexMap<SessionId, Session>, single_session: bool }`.
- **Complexity:** O(1) per request.
- **Edge cases:** single-session toggle while sessions exist (ask: disconnect or keep); closing a session with attached clients (detach first); IDE restart (restore sessions empty, reconnect lazily); client switching session mid-transaction (warn: tx stays in old session); department-based isolation must respect temp tables (introspection in single-session mode must use the shared session to see temp objects); MFA-per-connection users (single-session is the recommended mode per docs).
- **Test cases:** two consoles, multi mode → two physical connections; single mode → one; introspection while console runs a 60 s query in multi mode → runs on service session, not blocked; in single mode → queued behind (documented trade-off).
- **Prior art and evidence:** `[Observed: help:managing-connection-sessions; classes routers/departments]`.
- **Used by:** Hacivat; Karagöz (a session = audit unit; P2P channel multiplexes sessions).

### ALG-SESS-02: Cancellation escalation

- **Problem:** Stop running work quickly and safely, escalating only with user consent.
- **Inputs / outputs:** `cancel(session)` → `CancelOutcome{Cancelled, StillRunning, Deactivated}`.
- **Algorithm:**
  1. Drop all **queued** requests of the session (complete them with `Cancelled`).
  2. Send a protocol-level cancel for the running statement on a **side channel** (PG: CancelRequest with backend pid + secret on a new socket through the same tunnel/TLS; MySQL/MariaDB: second connection `KILL QUERY <thread_id>`; MSSQL: TDS attention packet; Oracle: OCI break / driver cancel; SQLite/DuckDB: interrupt handle; ClickHouse: `KILL QUERY WHERE query_id=…`; MongoDB: `killOp`/`killOperations` by `opId` or `comment`; Redis: `CLIENT KILL ID <id>` from a second connection / `SCRIPT KILL`/`FUNCTION KILL` for Lua).
  3. Mark session `Cancelling`, start a 10 s timer (DataGrip's threshold).
  4. If the statement finishes (with cancel error or not) → `Connected` (in manual tx: PG tx becomes aborted → require rollback; MySQL statement-level rollback only).
  5. On timer expiry → non-modal notification "taking too long — Keep waiting / Deactivate".
  6. On second cancel request → modal: Deactivate / Keep Waiting.
  7. Deactivate → close the physical connection (socket drop), mark `Broken`, warn that the server may continue (offer server-side kill if a privileged side connection exists: PG `pg_terminate_backend`, MySQL `KILL CONNECTION`, MSSQL `KILL spid`).
  8. Cancel during connect → abort handshake, close tunnel if refcount 0, state `Disconnected`, message "Connection canceled".
- **Edge cases:** cancel races with completion (ignore late cancel ack; PG cancel may hit the *next* statement — guard by waiting for ReadyForQuery before sending next request, or by query sequence check); cancel through SSH needs the forward to still exist; pooled proxies (PgBouncer) route cancel by key — OK in transaction pooling only if cancel key forwarded; Redis blocking commands: only connection close works; driver that ignores cancel; cancel of a COPY/LOAD stream; cancel during result fetch (stop fetching, close cursor; server may still compute).
- **Test cases:** PG `SELECT pg_sleep(60)` → cancel → error `57014` within < 1 s, session usable; MySQL `SELECT SLEEP(60)` → KILL QUERY → returns 1/interrupted; Redis `BLPOP k 0` → close connection, reconnect lazily; second cancel within 10 s → dialog shown.
- **Prior art:** `[Observed: help:run-a-query; bundle:deactivate.data.source.*]`; protocol mechanisms from vendor docs.
- **Used by:** Hacivat; Karagöz (must forward cancel over P2P and execute the side-channel cancel locally next to the DB — the remote side may not have a second DB connection otherwise).

### ALG-SESS-03: Keep-alive and auto-disconnect timers

- **Problem:** Keep idle connections alive through NAT/firewalls/proxies and close idle ones to save server resources, without interfering with running work or open transactions.
- **Inputs / outputs:** per connection `last_activity`, `keepalive_every: Option<Duration>`, `autodisconnect_after: Option<Duration>`, `in_tx: bool`, `busy: bool`.
- **Algorithm:**
  1. On any request completion: `last_activity = now`.
  2. Timer wheel tick (1 s): for each connected session —
     a. if `busy` → skip.
     b. if `autodisconnect_after` and `now - last_activity ≥ after`: if `in_tx` → if interactive, ask "Commit current transaction?" (Commit / Rollback / Keep connected), else (headless) keep connection and log; else disconnect (state `Disconnected`, clients stay attached).
     c. else if `keepalive_every` and `now - last_keepalive ≥ every` and `now - last_activity ≥ every` → run keep-alive (`SILENT` op; driver/DBMS-specific: `SELECT 1`, Oracle `SELECT 1 FROM DUAL`, Redis `PING`, Mongo `{ping:1}`, custom per driver). On failure → state `Broken` (lazy reconnect) and notify once.
  3. Keep-alive does **not** update `last_activity` (otherwise auto-disconnect would never fire).
- **Edge cases:** keep-alive inside an aborted PG transaction fails (`25P02`) → do not mark broken; use protocol-level ping (`Sync`/empty query) or skip when `in_tx`; MySQL `wait_timeout` lower than keep-alive interval; SSH keep-alive vs DB keep-alive independence; laptop sleep (huge `now - last` jump → treat as broken and reconnect on next use, don't fire 1,000 keep-alives); keep-alive must not change `last_insert_id()`/`@@ROWCOUNT`-style session state (use statements without side effects).
- **Test cases:** every=60, after=300, idle 301 s, no tx → disconnected at ~300 s with keep-alives at 60/120/180/240; with open tx → prompt; keep-alive during running query → skipped.
- **Prior art:** `[Observed: help; KeepAlive/AutoDisconnect action descriptors]`.
- **Used by:** Hacivat; Karagöz (server-side idle reaping of P2P sessions should use the same semantics).

### ALG-RO-01: Read-only statement classification

- **Problem:** Decide whether a statement may modify data/schema/state, per dialect, with conservative defaults.
- **Inputs / outputs:** `(dialect, statement_text)` → `Class{ReadOnly, Dml, Ddl, RoutineCall, SideEffect, Unknown}` + reason span.
- **Algorithm (SQL):**
  1. Parse (dialect parser; on parse failure → `Unknown` ⇒ treat as `SideEffect` in read-only mode).
  2. Leading keyword dispatch: `SELECT`/`VALUES`/`TABLE`/`SHOW`/`DESCRIBE`/`EXPLAIN` (without `ANALYZE`!) → candidate ReadOnly; `INSERT/UPDATE/DELETE/MERGE/UPSERT/REPLACE/COPY … FROM/LOAD DATA/TRUNCATE` → `Dml` (TRUNCATE is DDL in some DBMSs — classify `Ddl` for display but block either way); `CREATE/ALTER/DROP/RENAME/COMMENT/GRANT/REVOKE` → `Ddl`; `CALL/EXEC/EXECUTE/DO` → `RoutineCall`; `SET`, `USE`, `BEGIN/COMMIT/ROLLBACK/SAVEPOINT`, `LISTEN`, `PREPARE`, `DEALLOCATE`, `DISCARD` → allowed session statements (allow-list).
  3. Deep checks for candidate ReadOnly: CTEs with data-modifying sub-statements (`WITH x AS (DELETE … RETURNING …)`) → `Dml`; `SELECT … INTO new_table` (PG/MSSQL) → `Ddl`; `SELECT … FOR UPDATE/SHARE` → allowed (locks only) but warn; function calls in SELECT: if function is known volatile/side-effecting (PG `nextval`, `setval`, `pg_terminate_backend`, `dblink_exec`, `lo_unlink`; Oracle autonomous functions; MySQL `GET_LOCK`, `SLEEP` harmless) → `SideEffect` using a per-dialect deny-list + model knowledge (routine volatility from introspection: PG `provolatile='v'`); `EXPLAIN ANALYZE <dml>` → `Dml`.
  4. Unknown statement kinds → `SideEffect` (fail-closed).
- **Algorithm (Redis):** look up the command in a command table (from `COMMAND DOCS`/`COMMAND INFO` flags: `write`, `admin`, `noscript`, `pubsub`); `write` → `Dml`; admin/config (`FLUSHALL`, `CONFIG SET`, `DEBUG`, `SHUTDOWN`) → `Ddl/SideEffect`; `EVAL`/`FCALL` → `SideEffect` unless `EVAL_RO`/`FCALL_RO`.
- **Algorithm (Mongo):** method signature table with `pure` flag (DataGrip's `[pure]` annotations); aggregate with `$out`/`$merge` stage → `Dml` even though `aggregate` is pure otherwise; `runCommand` → inspect command name.
- **Data structures:** per-dialect keyword trie; function deny-list; command tables.
- **Edge cases:** multi-statement strings (classify each, max wins); comments/hints before keyword; dynamic SQL (`EXECUTE 'DELETE…'` in PG DO blocks) → `RoutineCall/SideEffect`; T-SQL `GO` batches; MySQL conditional comments `/*!50000 DROP … */` (must be parsed as code!); PL/SQL anonymous blocks → SideEffect; `COPY … TO STDOUT` → ReadOnly; `VACUUM`/`ANALYZE` → SideEffect; `REFRESH MATERIALIZED VIEW` → Ddl/SideEffect; `LOCK TABLE` → SideEffect; `CALL` of a procedure known read-only (no DML in body) still `RoutineCall` (DataGrip blocks with "may cause side effects").
- **Test cases:** `WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d` → Dml; `SELECT nextval('s')` → SideEffect; `/*!40101 DROP TABLE t */` (MySQL) → Ddl; `EXPLAIN SELECT 1` → ReadOnly; `EXPLAIN ANALYZE DELETE FROM t` → Dml; Redis `GET k` → ReadOnly, `SET k v` → Dml, `EVAL_RO …` → ReadOnly; Mongo `db.c.aggregate([{$out:"x"}])` → Dml.
- **Prior art:** `[Observed: bundle:database.access.{dml,ddl,routine,common}; DatabaseAccessController$PsiBasedReadOnly; Mongo [pure] signatures; Redis Ddl/Upsert statement PSI]`.
- **Used by:** Hacivat (UX guard) and **Karagöz (authoritative enforcement — must be implemented server-side because P2P traffic bypasses Perde)**; combine with DB-level read-only (`default_transaction_read_only`, read-only DB roles) for defence in depth.

### ALG-SESS-04: Transaction mode switching

- **Problem:** Change auto/manual and isolation safely.
- **Algorithm:** if `in_tx` → refuse with "transaction is in progress, commit or roll back first" (DataGrip behaviour); else Auto→Manual: `setAutoCommit(false)` (PG/MySQL: no server statement until first statement; MSSQL: `SET IMPLICIT_TRANSACTIONS ON`); Manual→Auto: driver commits pending (JDBC semantics) — we must instead require explicit commit/rollback; isolation: issue `SET TRANSACTION ISOLATION LEVEL …` at next tx begin (PG: `BEGIN ISOLATION LEVEL …`; Oracle supports only READ COMMITTED/SERIALIZABLE → disable others; SQLite only SERIALIZABLE (+ read_uncommitted pragma for shared cache)).
- **Edge cases:** detect tx state from protocol (PG ReadyForQuery `I/T/E`, MySQL `SERVER_STATUS_IN_TRANS`, MSSQL `@@TRANCOUNT` or ENVCHANGE tokens) instead of trusting client bookkeeping — user may run `BEGIN` manually in Auto mode; DDL auto-commits in MySQL/Oracle (tx ends implicitly).
- **Test cases:** user types `BEGIN;` in Auto PG console → UI must show "in transaction" (status `T`); MySQL `CREATE TABLE` inside manual tx → tx state becomes Idle.
- **Prior art:** `[Observed: bundle tx messages]`.
- **Used by:** Hacivat; Karagöz (audit tx boundaries).

### ALG-SESS-05: Session environment application

- **Problem:** Make every (re)connected session match the user's intended environment.
- **Algorithm (ordered):** 1) DBMS detection & version; 2) read-only flag (driver + server statement); 3) time zone (if set); 4) current database (connection-bounds) and schema/search path (Automatic mode: restore last recorded path; Manual: user choice; Disable: none); 5) DBMS_OUTPUT enable (Oracle/Db2); 6) ClickHouse session_id; 7) **startup script** (user SQL, macro-expanded) — executed last so it can override; 8) record server-reported state (current schema, search path, tx status) as the baseline for the `SearchPathReceiver` logic.
- **Edge cases:** startup script errors (fail connect? DataGrip: `[Unknown]`; recommend: show error, keep session connected but flagged); startup script containing `BEGIN` (leaves tx open — warn); script that changes schema conflicts with Automatic mode (startup script wins, record result); reconnect must replay identically (idempotent statements recommended).
- **Test cases:** reconnect after keep-alive failure reapplies `SET search_path = a,b` and startup script; Disable mode issues no schema statements.
- **Prior art:** `[Observed: help Options tab; executionEnvironmentHelper, TimeZoneManager classes]`.
- **Used by:** Hacivat; Karagöz may inject mandatory environment (e.g. `SET ROLE readonly_role`, `statement_timeout`) **before** the user's startup script.

### ALG-SESS-06: Search-path tracking (receiver/sender)

- **Problem:** Keep the UI's "current schema" correct even when the user changes it by SQL, and restore it for new connections.
- **Algorithm:** after each executed statement, if it syntactically changes schema (`USE x`, `SET search_path`, `SET SCHEMA`, `ALTER SESSION SET CURRENT_SCHEMA`) or after any error/rollback that may revert it (PG `SET` inside rolled-back tx is reverted; `SET LOCAL` is tx-scoped) → re-query server state (`current_schema()`, `SHOW search_path`, `DATABASE()`, `SYS_CONTEXT('USERENV','CURRENT_SCHEMA')`, `SCHEMA_NAME()`) and update the session env; UI selector changes send the equivalent statement (sender).
- **Edge cases:** `SET LOCAL`; function-level `SET search_path` (no effect after call); PgBouncer transaction pooling (session state not preserved! warn); quoted/case-sensitive schema names; `$user` in search path.
- **Test cases:** `BEGIN; SET search_path=x; ROLLBACK;` → current schema reverts to previous.
- **Prior art:** `[Observed: classes SearchPathSender/Receiver; registry omit.schema.qualifier]`.
- **Used by:** Hacivat.

---

## 5. Implications for Hacivat / Karagöz / Perde

1. Implement **Session = actor** owning one connection (+ optional borrowed temp connection), a FIFO queue, tx state derived from the protocol, env, and a cancel side channel. Clients attach/detach. Provide both routing modes; default to multiple sessions (DataGrip default) but offer single-session for MFA/PgBouncer scenarios.
2. Adopt DataGrip's **department** list verbatim as an enum (stable ids) — it doubles as Karagöz audit `source` and as a policy dimension (e.g. "introspection allowed, data export denied").
3. Cancellation UX: copy the escalation (10 s hint, second-click deactivate) but add server-side kill when privileges allow; over P2P, Karagöz executes the cancel side channel locally.
4. Read-only: enforce in three places — DB role/session (`default_transaction_read_only`), Hacivat classifier (UX), **Karagöz classifier (authoritative)** using ALG-RO-01 with fail-closed semantics.
5. Keep-alive/auto-disconnect/startup script/session templates are cheap wins; session templates map naturally to **Perde roles** (template = role-specific auth + read-only + startup script, e.g. `SET ROLE analyst`).
6. Show tx state from protocol, not from client bookkeeping (improvement over a JDBC-based client).

## 6. i18n considerations

- Session names (`console`, `console_2`) are user data but default names come from a message key; Turkish users should get `konsol`, `konsol_2` only if they opt in — recommend keeping default session names English-neutral ("console") because they appear in audit logs across locales, while the *labels* around them are translated.
- Transaction/isolation names: keep SQL keywords (READ COMMITTED) untranslated in SQL, but translate descriptions ("Current transaction detects only committed changes").
- Read-only blocking messages have a verb placeholder (`%s to execute …`) — a word-order trap for Turkish (verb-final). Use whole-sentence keys per variant instead of composing.
- Durations in keep-alive/auto-disconnect fields: localized number formatting, but store seconds as integers.

## 7. Risks, pitfalls, open questions

- `[Unknown]` Default keep-alive/auto-disconnect values; whether keep-alive runs inside open transactions.
- `[Unknown]` Behaviour when the startup script fails.
- `[Unknown]` Exact semantics of `Lockpick` and `PotentiallyTransparentOperation`.
- `[Inferred]` Driver-level cancel mechanisms are JDBC-driver behaviours, not DataGrip code.
- Pitfall: PG cancel requests can cancel the *wrong* statement if sent after the target finished and a new one started — Hacivat must serialize per session and drop stale cancels.
- Pitfall: auto-commit toggling via JDBC implicitly commits; a Rust client should never commit implicitly.
