# ADR 0006 — Execution and results

- Status: Accepted
- Date: 2026-11-14
- Scope: M3a (HV-EXEC-01, -05, -11; HV-RES-01 to -06; HV-VAL; R-M3-16 to -18, -23, -25, -31)

## Context

M3a runs a query and shows its rows. The pieces this ADR covers:

- the value model (`hacivat-values`): logical and native types, `Value`, columnar `Chunk`s and
  the display rules;
- the typed streaming surface of `RawConn` (`hacivat-driver::typed`): result sets as events,
  a suspendable query (a console cursor) that stops at a read-ahead point and resumes or closes;
- the session's `Run` request and its cursor requests (`hacivat-session`), with the read-only
  guard around a typed query;
- the result store (`hacivat-results`): one writer, lock-free snapshots, a memory budget, the
  fetch window;
- the run service (`hacivat-exec`): execution units, one session request per run, the run
  status, result tabs, the output log and the console UX state.

These choices decide what a statement can do while its rows are on screen (an open cursor
holds a snapshot and a transaction) and how much memory a result can take, so each is recorded
here by the task that makes it.

## Rule

Each M3a task that makes a choice in this area appends one section headed
`## m3a-tNN — <topic>` (append-only; a choice made in a fix round goes into the same task's
section before re-review). A section states the choice, the alternatives it rejected, how it
fails closed, and the tests that hold it. Sections, in the order of the plan: Task 1b (seams),
Task 23 (run service), Task 31 (conformance matrix).

## m3a-t01b — seams

**`Value` layout (P8).** `Value` is at most 32 bytes (`value_is_at_most_32_bytes`). The wide
payloads sit behind a pointer: `Bytes(Arc<[u8]>)`, `Lossy(Arc<[u8]>)`, `Opaque(Arc<OpaqueVal>)`,
`Error(Arc<ErrorVal>)`, `Truncated(Box<TruncatedVal>)`; text-like values are `Arc<str>`. The
largest inline payloads are `TimestampTz`, `Decimal` and `Interval`. Rejected: `bytes::Bytes`
for binary values (four words, which makes `Value` 40 bytes) with the test relaxed to 40. Bulk
bytes travel in a `Chunk`'s column buffers, so `Value` is the exception path (single cells,
binds, the display cache), and a clone of a wide value is a reference-count bump.
`ValueRef<'a>` borrows a cell out of a `Chunk` without allocating.

**One open query per connection, with the PostgreSQL exception (P5).** The `RawConn` typed
methods (`typed_caps`, `query`, `next_event`, `fetch_more`, `close_query`) default to
`RawErrorKind::Unsupported`, so a driver that has not implemented them fails closed. The rule,
in `hacivat-driver/src/typed.rs`: one open query per connection, except on PostgreSQL, where
`execute`/`batch` may run while a query is `Suspended`, in the same transaction, and the named
portal survives (the read-only guard's canary, R-M3-17, depends on it). MySQL and SQLite answer
`Unsupported` to a statement while a query is open; every engine answers `Unsupported` to a
second `query`. On PostgreSQL `suspendable: true` requires `tx_status() != Idle`, else `query`
answers `Unsupported` before it sends anything: a Sync outside a transaction block would end the
portal. The session opens `BEGIN` or a `SAVEPOINT` (Task 18), or the guard its read-only `BEGIN`
(Task 22), first. A statement that returns no rows reports one result with an empty schema
(`ResultStart` of zero columns, then `ResultEnd` with the affected rows), so every unit has a
result to attach its status to.

**What `FakeDriver` models, and what it does not** (fix round 1 corrected an overclaim here).
It enforces these rules, each with a test in `hacivat-session/tests/m3a_interfaces.rs`:
- one open query per connection, except a suspended PostgreSQL portal
  (`one_open_query_per_connection_except_a_suspended_pg_portal`);
- a suspendable PostgreSQL query outside a transaction is refused
  (`fake_pg_refuses_a_suspendable_query_outside_a_transaction`);
- `COMMIT`/`END`/`ROLLBACK` destroy a suspended PostgreSQL portal (later `fetch_more` or
  `next_event` answer 34000); `ROLLBACK TO SAVEPOINT s` destroys a portal opened inside `s` (or
  a later savepoint) and keeps one opened before `s`; `RELEASE SAVEPOINT` keeps it (round 2;
  `a_suspended_pg_portal_does_not_survive_commit`,
  `rollback_destroys_a_suspended_pg_portal_and_rollback_to_keeps_it`,
  `a_portal_opened_inside_a_savepoint_does_not_survive_rollback_to_it`, R-M3-23's
  `SAVEPOINT hv_cursor`);
- a failed PostgreSQL transaction refuses `query` and every statement except
  `ROLLBACK`/`ABORT`/`COMMIT`/`END` and `ROLLBACK TO SAVEPOINT`, with 25P02
  (`a_pg_query_in_a_failed_transaction_is_refused`,
  `a_failed_pg_transaction_takes_only_its_end_or_a_rollback_to`). PostgreSQL accepts `COMMIT` in a
  failed transaction and answers `ROLLBACK`, so the fake accepts it too;
- a cancel or a failed statement inside a PostgreSQL transaction fails it (P16);
- a MySQL drain of more than one read asks for a server-side kill whenever the caps say
  `close_needs_kill` (`a_mysql_drain_of_a_large_rest_asks_for_a_kill_by_its_caps`);
- `Protocol::Simple` is refused on every engine (`a_simple_protocol_query_is_refused`).

It is still a model, not the engine. Its transaction status and savepoint names follow statement
keywords only with `.tracks_transactions()` (without it, a script sets the status and every
suspendable PostgreSQL query is refused); it knows no locks, isolation or server-side time-outs;
its cancel is a latch. The Docker legs (Tasks 18, 22 and 31) are the authority for engine
behaviour; a session test that passes only against the fake proves the session's logic, not the
engine's.

**The guard around a typed query: a phase API.** `readonly::guarded_query` returns a
`GuardedCursor` whose `after_first_chunk` and `finish` run the guard's later steps (the canary
after the first chunk, the replay and the transaction end when the cursor closes); each step
fails with a `GuardFailure { error, discard }`. Rejected: one function that returns when the
whole statement is done, because a console cursor outlives the request that opened it (a
`FetchMore` or `CloseCursor` request finishes it), and Task 22 edits only `readonly.rs` while
Task 18 owns the call sites in `run.rs` and `cursor.rs`. Task 1b's stubs answer
`ExecError::Internal`, so nothing runs until Task 22 fills them.

**Fail-closed stubs.** Until their tasks land: the session's `Run`, `FetchMore`, `FetchAll` and
`CloseCursor` answer `ExecError::Internal` and send nothing (`run_fails_closed_until_task_18`);
`resolve_units` answers `NothingToRun`; `ExecService::run` sends nothing and answers an aborted
summary; the console's Run shows "Running queries isn't available in this console yet." and
sends nothing. Working since Task 1b because later tasks build on them: `SessionLog` (a bounded
ring), `MemoryBudget` (`min(RAM / 4, 4 GiB)`, else 1 GiB; 256 MiB per result), `FetchPolicy`'s
defaults, `Chunk::from_values`, `effective_read_only(profile, production_read_only)` with
`SessionPolicy` (production profiles are read-only in the app since this commit,
`connections/boot.rs`).
