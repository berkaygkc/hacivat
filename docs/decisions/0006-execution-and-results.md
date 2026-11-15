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
- `COMMIT`/`END`/`ROLLBACK` destroy a suspended PostgreSQL portal. As on a real pg17 server
  (pinned by a Task 12 Docker test): `next_event` answers `Unsupported`; `fetch_more` answers
  34000 once, sets the status to `Failed` inside a transaction block, and leaves nothing open;
  `close_query` answers `Closed { rows: sent }`; a new `query` answers `Unsupported` until the
  dead run is closed (Task 12 review round 1); `ROLLBACK TO SAVEPOINT s` destroys a portal opened inside `s` (or
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
  since Task 14 round 1 the MySQL-family fake also answers `close_query(Now)` with
  `NeedsServerKill` and keeps the run open, gives `Completed` on `AfterKill` when nothing was
  unread (or `Err(Io(TimedOut))` with the `kill_ignored` knob), returns a drained-rest error as
  `Err(Db)` and a 1317 as `Closed`, and gives each connection its own non-zero thread id
  (ADR 0002 `m3a-t14`);
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

## m3a-t23 — run service

**What is sent.** `resolve_units` calls `exec_range::resolve` in execution mode and sends each
unit's strict range, byte for byte (`doc.text()[range]`: no terminator, no leading comments),
with the extended protocol and `StopOnError` (R-M3-26). `suspendable` is
`head::cursor_eligible` on that same strict range, so `(SELECT …)` is never eligible (fail
closed). Tests: `units_carry_strict_text_and_eligibility`,
`the_text_sent_is_the_strict_range_byte_for_byte`, `a_parenthesised_select_is_never_cursor_eligible`,
`the_text_sent_is_the_units_text_byte_for_byte`.

**Refusals before anything is sent.** `Ambiguous` → `Resolution::Uncertain`. A `COPY` that
streams through the client (`UnitKind::CopyIn`, or a `COPY` naming `STDIN`/`STDOUT` in the
document's own tokens) refuses the whole run (`Resolution::Refused { CopyStream }`,
`exec-copy-unsupported`), also inside a multi-unit run: under `StopOnError` the units before it
would run and the `COPY` then fail, so a script holding one runs nothing. Rejected: running the
units before it (a half-run script the user did not ask for). `UnitKind::Verbatim` (a selection
run as one) is `Refused { RunAsOne }` until M3b. The chooser drops every candidate Run would
refuse; with none left, the first refusal answers. Tests: `a_copy_stream_refuses_the_whole_run`,
`the_chooser_leaves_out_candidates_that_would_be_refused`, `a_verbatim_unit_is_refused_until_m3b`, `a_run_holding_copy_in_or_verbatim_is_refused_whole` (the controller's ruling from Task 24: Task 24's preview marks these units uncertain, so Run never sends the units around them; a `CopyIn` unit alone is refused too, M3b).

**The PostgreSQL oracle (R-M3-24, R-M3-35).** Only for a single caret, only on PostgreSQL and
Timescale (the same grammar; Cockroach, Yugabyte and Redshift parse differently), never for a
`ClientCommand` or `OverlappingUnits` boundary, and never for a selection: a selection that
starts inside a token or a `$$` body must not be rescued by a parser that reads the selected
text from a clean state. The region of a boundary is the execution statement whose strict range
holds it. A region deeper than `MAX_ORACLE_NESTING` parentheses (the lexer's count) or holding
`COPY` data is never sent. The oracle's ranges are relative to the region's text and trimmed,
so the region's start is added; they must cover the region exactly (first start and last end
equal the region's, only white space and `;` between, every range non-empty on character
boundaries). Run 1/2 then runs the oracle statement at the caret (a caret not in a region, or a
remembered "from the caret"/"whole script", stays uncertain); Run 3 runs every statement with
each region replaced by its oracle statements, every other statement certain in both splits,
and no unsupported client command. Rejected: comments between oracle statements (refused: a
separator check on raw text cannot tell `/* ; */` from code without re-lexing the region, and
re-lexing with the lexer that called it uncertain decides nothing). The oracle blocks its
caller; resolution runs off the UI thread (the app's background executor), and requests
coalesce latest-wins per `OracleLane`: a request superseded before its parser job runs is
skipped and answers `Uncertain`. Tests: `pg_ambiguity_resolved_by_the_oracle_when_it_covers_the_region`,
`an_oracle_split_that_does_not_cover_the_region_stays_uncertain`,
`deep_nesting_is_never_sent_to_the_oracle` (a counting oracle: 0 calls above the limit, 1 at
it), `mysql_ambiguity_stays_uncertain`, `a_selection_or_several_carets_never_ask_the_oracle`,
`a_superseded_oracle_request_is_skipped`.

**The observer.** `ExecService::run` submits one `Request::Run` and answers a `RunTicket`
(the request's `seq` for cancel). Its `RunObserver` creates a `ResultStore` per result on the
shared budget (process-unique `ResultId`s), keyed `TabKey { console, unit_ordinal, 0 }`; a
re-run into the same key gets the next generation. Tabs are titled `Result N` by their order in
the run. The status lives in its `watch` and is edited in place (`Arc::make_mut` copies the
unit list only while a reader holds a snapshot), so an event is O(1) unless the UI kept the
previous status; phase changes publish at once, progress inside a unit (result opened, rows)
at most every 50 ms. Rejected: rebuilding `Arc<[UnitStatusEntry]>` on every event (O(units) per
event, O(n²) for a whole-script run of n units). A result that stays suspended after its unit
shows `Suspended { rows }` and follows its result: `Succeeded` with `more: true` when it is
released, with a second "rows retrieved" line when it completes.

**What the log says, and what it does not.** One line per finished unit (rows retrieved, with
"more are available" while the cursor stays open; rows affected; completed; "Statement N
failed" with the error view; a read-only denial; "Cancelled" for the unit in flight only), a
released cursor's reason with its "Hacivat's transaction was rolled back" form (one message per
`CloseReason`, both forms: 20 IDs), more result sets, a user transaction ended by
`COMMIT`/`END` or `ROLLBACK`/`ABORT` (an implicit end, a MySQL DDL or a loss, is not announced:
better silent than wrong), and a summary for runs of more than one unit. A cursor released by a
failure while its unit is in flight (`ServerTerminated`, `ConnectionLost`, `EnforcementGap`)
says nothing of its own: the unit's error line does. The ring drops its oldest line when full;
lines that leave before any `lines()` call read them are counted, and `lines()` then starts with
one `Dropped { n }` line (`exec-events-dropped`) until the log is cleared.

**Later fetches (plan amendment).** A result source's commands go through the service's own
`FetchControl`, which submits `FetchMore`/`FetchAll`/`CloseCursor` and awaits the reply on the
runtime the observer ran on (the core runtime): an error with server text (or any but a cancel
or a silent loss) is logged as "Statement N failed" with the server's text, next to the
session's `CursorReleased` line. Test: `a_later_fetch_failure_is_logged_with_the_server_text`.

**A run that never started.** Its reply comes without `RunEnded` (queued behind a cancel, a
closed or lost session). The service awaits the reply on its runtime (`ExecService::new` takes
the current Tokio runtime when there is one; `with_runtime` sets it) and ends the run there:
every unit `Skipped` (`Cancelled` after a cancel), an aborted summary on `done`, and the
reply's error in the log. Without a runtime the observer's drop ends it the same way, without
the error. Test: `a_run_that_never_started_ends_with_its_reason`.

**Error views.** The title is by error kind (`exec-error-server`, `-connection-lost`,
`-connection-lost-unknown`, `-cancelled`, `-tx-failed` for `Tx(FailedNeedsRollback)`,
`-tx-in-progress`, `-tx-unsupported`, `-enforcement-gap`, `-not-connected`, `-internal`; a
read-only denial is its `exec-deny-read-only-*` sentence); the code chip is the SQLSTATE, else
the vendor code; the server's text is kept verbatim. The SQLite busy hint is the same rule and
Fluent ID as `hacivat_driver_sqlite::error_hint` (`SQLITE_BUSY` in the code's low byte, kind
`Db`, engine SQLite): `hacivat-exec` has no `hacivat-driver-sqlite` dependency and no task after
Task 1b edits a manifest, so the rule is restated here and pinned by
`sqlite_busy_gets_hacivats_hint`. `doc_offset` maps a 1-based server character cursor through
`LineIndex::char_to_byte` into the unit and then the document (`doc_offset_of`); no driver
reports a cursor yet (`ServerMessage` has no position field), so the service passes `None` and
the mapping is tested on its own (`failed_unit_maps_server_position_to_the_document`).

**Messages.** Every `LogMsg` ID is a literal in `fluent_id` (P24); durations and sizes are one
message each with the unit inside (`exec-dur-*`, `exec-size-*`, 1,024-based with one decimal);
a statement number is a raw string argument (never grouped). Each `AmbiguityReason` has an
`exec-uncertain-*` sentence (`OverlappingUnits` included). `copy_invariant` writes ISO 8601 UTC
timestamps with milliseconds, ASCII severities, message IDs, `name=value` arguments (a duration
as `<n>ms`) and the server's text verbatim with `\`, tab, CR and LF escaped, so one line stays
one line; nothing is localized or case-mapped.

**Console UX.** `console_ux` is pure: `forced_read_only` = the production policy on, the profile
tagged production and the session read-only; the transaction badge is the user's only
(`UserOpen` for `TxOwner::User` in a transaction, `Failed` when it failed), Hacivat's own around
a PostgreSQL cursor is `CursorSnapshot`, the guard's or an unclaimed one `Auto`; cancelling at
every rung; `running` only for a run that started and has not ended (a session `Busy` with
request 0, the idle release, is not a run). Test: `console_ux_table`.

**SQL context (R-M3-39).** `dialect_id` maps every engine (TiDB → MySQL; engines without a SQL
dialect → `Generic`); the probe is `SHOW standard_conforming_strings` on the PostgreSQL wire and
`SELECT @@SESSION.sql_mode` on the MySQL family; a PostgreSQL answer other than `on`/`off` keeps
the default; `needs_reprobe` is a unit whose first word is `SET`.

**Fix round 1 (review of Task 23).**
- *The oracle needs `standard_conforming_strings = on`.* libpg_query lexes as a server with
  the mode on; a session with it off (or not known to be on) never uses the oracle, because
  `'a\'; INSERT … --'` is one literal to that session's server and editor while the oracle
  would read an `INSERT` statement out of it. Test:
  `the_oracle_is_never_asked_without_standard_conforming_strings` (every caret, every action,
  0 oracle calls). No other PostgreSQL lexing mode moves a statement boundary.
- *Transaction lines wait for the unit's outcome.* A `TxChanged` to idle is remembered and
  announced when its unit ends: `COMMIT`/`END` that succeeded says committed; a failed
  `COMMIT`/`END`/`ROLLBACK`/`ABORT` that left the session idle says rolled back (PostgreSQL
  rolls back a COMMIT that fails at commit time: a deferred constraint, `40001`); anything
  else says nothing. Test: `observer::tests::a_failed_commit_is_never_announced_as_committed`
  (synthetic events: the FakeDriver cannot fail into idle).
- *A release by a failure fails the unit.* `CursorReleased` with `ServerTerminated`,
  `ConnectionLost` or `EnforcementGap` after the unit ended turns its `Suspended` status into
  `Failed` (title by reason; the server's text is on the fetch's log line). Releases the user
  or Hacivat chose keep `Succeeded { more: true }`. Test:
  `observer::tests::a_release_by_a_failure_fails_the_unit`.
- *Run 3's editor-split check* cannot be reached from real text (both splits list the same
  boundaries, R-M3-8), so `whole_script` is tested on hand-built splits
  (`units::tests::whole_script_refuses_a_statement_the_editor_split_finds_uncertain`); the
  client-command guard by `a_client_command_boundary_never_asks_the_oracle`.
- *Smaller rules.* A run that never started logs `exec-not-started` with the reason and no
  statement; a `NotConnected` reply to a cursor command is quiet; the console reprobes its SQL
  context after `SET`, `RESET`, `DISCARD`, `ROLLBACK` and `ABORT`; a size picks its unit after
  rounding (never "1024.0 KB"). `cursor-closed-server-terminated` says the server stopped the
  query with an error (the session stays connected).
