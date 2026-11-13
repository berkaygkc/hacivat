# ADR 0004 — Sessions and the read-only guard

- Status: Accepted
- Date: 2026-11-13
- Scope: M2b (HV-SESS-01 to -08, HV-CONN-04, -05, SQL-SAFE-05, MCP-09 step 1, D-035)

## Context

M2a holds one connection per profile and labels read-only connections without enforcing them
(R10). M2b turns connections into sessions:

- a session actor per connection, with a request queue, transaction state, a cancel ladder,
  environment replay and schema tracking;
- a `SessionManager` per profile that routes requests to the default session, client
  sessions, pools and jobs;
- liveness and reconnect with an honest account of the state a reconnect loses;
- a read-only guard in layers per engine (`hacivat-guard`, sans I/O), a role probe and an
  access label that never claims more than the server enforces.

These choices decide what a user's statement can do to their database, so every one is
recorded here by the task that makes it.

## Rule

Each M2b task that makes a choice in this area appends one section headed
`## m2b-tNN — <topic>` (append-only; a choice made in a fix round goes into the same task's
section before re-review). A section states the choice, the alternatives it rejected, how it
fails closed, and the tests (or bypass-suite results) that hold it. Topics, in the order of the
plan: the actor model, transaction state, the cancel ladder, the reconnect policy, the guard
layers per engine, the labels, the role probe and the bypass-suite results.

## m2b-t07 — session actor

**Actor model (HV-SESS-01).** One Tokio task per connection owns the `LiveConnection`. Requests
run one at a time from a FIFO `VecDeque`; while one runs the actor still reads commands (new
requests queue, cancels climb the ladder, a close waits for the request). The request future is
polled first (`biased` select), so its statement has started before any cancel is read. The
state is a `watch` of `SessionSnapshot` whose generation bumps on every change; events go out
on a `broadcast`.

- A panic in a driver call is caught (`AssertUnwindSafe` + `catch_unwind` on the request
  future): the session becomes `Broken(Internal)`, the connection is closed and discarded, and
  the panic message is logged, never the request's SQL. A second guard around the whole actor
  marks the snapshot `Broken(Internal)` if the actor itself panics. Each session has its own
  task, so other sessions are unaffected.
- A lost connection (`Io`, `Protocol`, `Timeout`, `Tls` during a request) breaks the session
  (`ConnectionLost { outcome_unknown }`); a broken session is never reused: every later request
  is `NotConnected` until Task 17's reconnect replaces it.
- A session is read-only when its spec **or** its profile says so (fail safe). Read-only
  statements go only through `readonly::run_guarded(.., replay)` with `replay` = the
  environment plan without the PostgreSQL client identity (Q15); the Task 1 stub refuses, so
  nothing runs unguarded before Task 18.
- Spawning runs neither the environment plan nor a schema probe (Q27). One exception, a
  safety one: a read-write MySQL session that starts Manual sends `SET autocommit = 0`, so the
  server never autocommits under a Manual badge.
- Rejected: one task per request (no single owner of the connection, harder cancel races);
  `JoinError`-based panic capture through `spawn` (would move the connection into a second
  task and lose it on panic anyway, with an extra hop per request).

**Transaction state (HV-SESS-03, B22, Q14).** The status always comes from
`RawConn::tx_status()` after every round trip, never from parsing SQL. Manual mode on
PostgreSQL and SQLite sends `begin_sql(..)` before a statement when the status is `Idle`
(not before `BEGIN`/`START`/`COMMIT`/`END`/`ROLLBACK`/`ABORT`); SQLite uses `BEGIN IMMEDIATE`
unless the first keyword is `SELECT` or `WITH`. MySQL switches `autocommit` at `SetMode`.
Manual → Auto with an open or failed transaction is `InProgress`; `SetIsolation` is refused
the same way and applies to the next `begin_sql` (MySQL's implicit Manual transactions keep the
server's level: a known limit). `Smart` needs the classifier (M3): `SetMode(Smart)` is
`NeedsClassifier`, and a profile that asks for Smart starts Manual (the safer mode) until then.
In `Failed`, any statement whose first keyword is not `ROLLBACK` is refused before it is sent
(advisory; the server refuses anyway). Read-only sessions are always Auto: `SetMode(Manual |
Smart)` and `Begin` are `Unsupported`.

**Cancel ladder (HV-SESS-05, B23, Q18, Q37).** `cancel(seq)` acts only on the running request
(the stale-cancel guard): anything else is `AlreadyFinished`. The first rung drops the queue
(`Cancelled`) and cancels through the connection's `CancelHandle`; MySQL has none and runs
`KILL QUERY <thread id>` on a side connection (`SessionHooks.side`), pre-opened once a
statement has run 500 ms so a cancel through SSH lands within about a second. Callers get the
outcome within 2 s (`CANCEL_WAIT`), else `StillRunning`; at 10 s the session reports
`SlowCancel`. `Terminate` runs `SELECT pg_terminate_backend(<pid>)` or `KILL CONNECTION <id>` on
a side connection; `Deactivate` drops the request and the connection and marks
`Broken(Cancelled)`. PostgreSQL `57014` and MySQL `1317` count as cancelled only once a cancel
was asked for (a statement timeout also answers `57014`). **Cancel race:** a cancel or kill
still in flight when its request ends is awaited (bounded by 2 s) before the next request
starts, so it cannot hit that request; the server-side race of a protocol cancel that arrives
after the statement ended remains the server's (PostgreSQL's CancelRequest is
fire-and-forget).

**`KILL` limits behind proxies (Q37).** `KILL QUERY`/`KILL CONNECTION` name a server thread ID
read at connect. Behind a connection-multiplexing proxy (ProxySQL, MaxScale, RDS Proxy,
Vitess) the side connection may reach a different backend, where the ID names another
session or none: the kill may fail (`Failed(class)`) or, in the worst case, hit an unrelated
session of the same user on that backend. Hacivat sends a kill only for the request it is
running, only by the ID its own connection reported, and never retries a failed kill. Users
behind such proxies should prefer `Deactivate`. The same holds for PostgreSQL behind
transaction-pooling PgBouncer (`pg_terminate_backend` by a pooled PID).

**Environment plan (HV-SESS-06, Q8, Q15, Q27).** In order: the client identity (PostgreSQL
`SET application_name`; MySQL's connect attribute carries it), the read-only setup of
`hacivat_guard::plan_for(kind).session_setup` for read-only sessions, no time zone until M3,
and the last schema only when one is known. Every literal or identifier goes through its own
quoting function: `quote_pg_ident` (`"` doubled), `quote_my_ident` (`` ` `` doubled),
`quote_pg_literal` (`'` doubled; an `E''` literal with doubled backslashes when the value has
one, so it reads the same whatever `standard_conforming_strings` is) and `quote_my_literal`
(`'` and `\` doubled). NUL is stripped from literals; a schema name with NUL (or an empty one)
is refused, never sent, because the server would cut its C string there.

**Schema tracking (HV-SESS-07, Q27).** The probe (`current_database()`, `current_schema()`,
`current_schemas(true)`, `current_user`; MySQL `DATABASE()`, `CURRENT_USER()`; SQLite
`PRAGMA database_list`) runs only after a user statement whose first keyword (ASCII-folded,
after whitespace and comments) is `SET`, `RESET`, `USE`, `BEGIN`, `START`, `COMMIT`, `END`,
`ROLLBACK`, `ABORT` or `DISCARD`, after a user commit or rollback, and after `SwitchSchema`;
`SchemaChanged` fires on a difference. Never after internal control statements or a reset.
Cost (accepted): the context can lag behind a change made by any other statement.

**Reset (HV-SESS-08, Q10, Q19).** `ROLLBACK` when not `Idle`, `RawConn::reset()`, then
PostgreSQL's verify probe `SELECT current_user = session_user`, which must answer true
(no row fails closed), then the full environment plan (identity included) and MySQL's
`autocommit = 0` for a Manual session; mode and isolation return to the session's start.
Any failure breaks the session and closes the connection.

**Tests.** `crates/core/hacivat-session/tests/{session_actor,tx,cancel,env_schema}.rs` (49
tests over `FakeDriver` through the real connect pipeline). The fake now latches a cancel sent
between `execute` and its first poll, and drops one sent while nothing runs (T1 review M10;
`hacivat-driver` `fake::tests::an_early_cancel_is_latched_and_an_idle_cancel_is_a_no_op`).
