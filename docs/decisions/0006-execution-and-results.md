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

## m3a-t31 — conformance

**What the matrix is.** `crates/core/hacivat-exec/tests/docker_m3a_matrix.rs` has nine Docker
tests, with their helpers in `tests/common/mod.rs` (`// ## m3a-t31-matrix`). Every case goes
the app's way:

1. a console document in the context the console read at bind (R-M3-39: the server version and
   the probe row);
2. `resolve_units` in execution mode (Execute3, the whole script);
3. `ExecService::run`;
4. a real session from a `ConnectionManager` with M2b's configuration and the app's production
   policy;
5. the typed driver.

An admin connection (the side channel) checks what reached the server. Statements reach the
session only through `ExecService::run`. The tests call `SessionHandle` directly only where the
app does, or for checks (fix round 1, review M8):
- Stop: `SessionHandle::cancel` of the run's request or of the fetch in flight. `ExecService`
  has no cancel of its own; the app cancels the same way.
- Closing a console: `SessionHandle::close`.
- Two internal, idempotent `Request::Exec` calls: the console's SQL-context probe (R-M3-39) and
  the backend id that the side channel checks against.
- The command-tag pass of `dml_and_ddl_report_update_counts_on_a_dev_console`. It sends the
  same resolved units through `SessionHandle::run` with its own observer, because the run service
  does not keep the tag (ruling 5).

**Lines (the M2b ruling: no line is skipped silently).**

- The server lines come from `targets(Some(engine))` for PostgreSQL, MySQL and MariaDB.
  `classify` sorts them by their properties, never by a list of names (fix round 1, review I2):
  - **a line:** one target per engine and major, at or above the floor of `EngineCatalog`;
  - **a twin:** a second target of the same engine and major whose id is its line's id plus a
    `-variant` suffix (`pg17-tls`, `pg17-auth`, `pg17-tr`, `mysql84-x509`). Twins are the lines
    of their own tests;
  - **below the floor:** pg13.
- Any other target fails every test until someone classifies it:
  - a second target of a major that is no twin;
  - a target of another engine;
  - a target that `targets(None)` has and the per-engine lists do not.

  A line added later (a `pg19`) joins the matrix by itself. The plain unit test
  `lines_are_classified_from_the_targets_never_from_a_list` pins all of this, with no Docker.
- Every test also runs on a SQLite file of its own.
- Under `full`, every major that the catalog's `tested` list names must have a line. In any set,
  PostgreSQL and the MySQL family must each have a line. The `pr` set has pg17 and mysql84 and no
  MariaDB line, so the rule is per family.
- `admin` first sweeps the `hv_t31_*` objects that another process's red run left behind (their
  names carry that process's id), so fixtures stay idempotent across runs (Q7; review M9).
- Each test records one row per line and case, asserts that every line of the active set ran,
  and writes the rows to the Docker artifact `t31-<test>.tsv`. The table below is built from
  those rows.

**Rulings.**

1. *The cancel budget.* The plan names `cancel_within_one_second_on_every_engine`, and the
   Docker rule keeps its wall-clock assert (with margin).
   - What is measured: from Stop (`SessionHandle::cancel` of the run's request, or of the fetch
     in flight) to the end of the run, or to the cancel's answer.
   - When: only after the statement has passed the overdue mark. Before it, R-M3-16 waits for the
     mark by design. The test reads the mark from the server where it can:
     - PostgreSQL: `pg_stat_activity.query_start`, 1.1 s ago;
     - MySQL family: `processlist.time ≥ 2`;
     - SQLite: the run status `Running { since }`, 1.1 s ago;
     - a fetch: no chunk for 1.1 s.
   - On what: a quiet matrix. The test holds `QUIET` for writing; every other test holds it for
     reading.
   - Measured over the three flake runs:

     | Engine | Time |
     |---|---|
     | PostgreSQL | 1–6 ms |
     | MariaDB | 4–37 ms |
     | SQLite | 0–3 ms |
     | MySQL | 6–65 ms |

     MySQL's kill goes over a new side connection with TLS.
2. *Engine semantics, pinned, not hidden.* MySQL's `KILL QUERY` ends a bare `SELECT SLEEP(60)`
   normally: `SLEEP()` returns 1 without an error, as the MySQL manual says. MariaDB's `DO`
   swallows the interruption (`DO SLEEP(60)`, `DO (SELECT COUNT(*) …)`). In these cases:
   - the unit succeeds and the cancel answers `AlreadyFinished`;
   - the stop still lands within the budget, the server is free and the connection is reused;
   - the console shows what the server said.

   `CancelOutcome::AlreadyFinished` is documented to match (review M7). The change is a fenced,
   doc-only named exception in `hacivat-session/src/session/cancel.rs`: "the request ended
   without the server reporting a cancel: it was not running, or the server ended normally a
   statement Hacivat's kill interrupted".

   MariaDB's fetch in a long step is a sequence-engine scan whose last row sleeps
   (`SELECT IF(seq <= 20000, seq, SLEEP(60)) FROM seq_1_to_20001`), because its `UNION ALL`
   materialises before the first row. `KILL QUERY` ends it with an error in 10–14 ms (review M6).

   MySQL 8.0 ends a killed `DO (SELECT COUNT(*) …)` or `SET @v = (…)` normally in some runs and
   with 1317 in others (one run in three cancelled, across the flake runs). Those statements are
   therefore not cases. The MySQL family's "not a read head" case is
   `INSERT INTO t … SELECT COUNT(*) FROM <10⁹-row cross join>`: the kill ends it with 1317, and
   the side channel finds no row.
   - MySQL takes the CTE form (`INSERT INTO t WITH d(x) AS (…) SELECT …`). The same count over
     derived tables took 77 ms to 1.2 s to honour the kill on mysql80 and mysql84, which is the
     server's plan, not the session.
   - MariaDB runs the derived-table form (5–14 ms).
3. *The error view's code on MySQL is the SQLSTATE* (Task 23's rule: the SQLSTATE when there is
   one). Read-only refusals therefore read `25006` (vendor 1792), and MariaDB's `XA START`
   reads `XAE09` (vendor 1400).
4. *Bypass rows through Run differ from run B where the run service splits.*
   - `SELECT 1; DROP TABLE t` is two units, and the server refuses the `DROP` with `25006`.
     Run B gets `42601`/`1064` from one multi-statement prepare.
   - `/*!50000 … */` is code to the MySQL lexer and is sent as written.
   - MariaDB's `LOCK TABLES` ends in a gap (the connection is discarded and reconnected), as in
     run B.
   - The rows run as the test account, which may write, so only the read-only enforcement stops
     them.
   - After every row, the side channel must be unchanged.
   - After every row, nothing may be held (review M3): the console's backend is neither idle in
     a transaction nor running, and the fixture can be locked at once from the side channel:
     - PostgreSQL: `LOCK … ACCESS EXCLUSIVE NOWAIT`;
     - the MySQL family: `SELECT … FOR UPDATE NOWAIT`;
     - SQLite: `BEGIN EXCLUSIVE`.
   - After every row, a control `SELECT count(*)` must run on the same console. A console that
     runs nothing fails the control.
5. *DDL counts.* PostgreSQL's command tag has no count, so DDL logs "Completed". MySQL's OK
   packet and SQLite's change counter say 0, so DDL logs "0 rows affected", as the `mysql` and
   `sqlite3` clients do. Command tags are PostgreSQL's own (`INSERT 0 3`, `UPDATE 2`) and the
   first keyword on MySQL, MariaDB and SQLite.
6. *FakeDriver fidelity.* The real outcomes were compared with what the FakeDriver tests of
   Tasks 18 and 23 assume, and no case was found where the fake is more lenient than a server.
   The compared cases:
   - stop between chunks: no server cancel;
   - overdue chunk: one cancel or kill;
   - a kill that races the statement's end: absorbed;
   - Hacivat's transaction rolled back on release;
   - production refusals;
   - the update counts.

   No fake was changed.
   *Rollback is proved on the server, not by the log* (fix round 1, review I1). On PostgreSQL
   the read head calls a `VOLATILE` function that inserts every row it reads. The side channel
   then counts the rows:
   - **0** after each early release: close cursor, another statement, the idle bound, stop, and
     a console closed with its cursor open;
   - **all of them** after a cursor that completes (12,000; 30,000 in the cursor test).

   The product mutation `Hold::OwnTx` ROLLBACK → COMMIT (`cursor.rs`) fails both tests, where
   the log ids alone passed it.
7. *The read-only startup option (Task 22).* `SHOW idle_in_transaction_session_timeout` answers
   `30s` through Run on a read-only console on pg14–pg18. A pooler that refuses startup options
   (pgbouncer without `ignore_startup_parameters`) is not in the matrix, so that case is not
   verified (M3b).

**The table** (`full`; green three times in a row before fix round 1 and twice after it;
versions as the servers report them):

| Line | Version | Golden table | Cursor: suspend 10,000, fetch more, close, fetch all | Stop fetching (rows kept) | Cancel (read head / not / fetch in a long step) | Read-only bypass rows | Production writes | DML/DDL counts and tags | No transaction left |
|---|---|---|---|---|---|---|---|---|---|
| pg14 | 14.25 | 3 × 18 ✓ | ✓, rolled back (server: 0 rows; 30,000 on completion) | ✓ | ✓ / ✓ / ✓ | 22/22 stopped (gap: `txid_current`) | 25006 ×3 | ✓ | 5 paths, 0 rows written each; 12,000 on completion; read-only ✓ |
| pg15 | 15.20 | 3 × 18 ✓ | ✓, rolled back (server: 0 rows; 30,000 on completion) | ✓ | ✓ / ✓ / ✓ | 22/22 | 25006 ×3 | ✓ | ✓ |
| pg16 | 16.16 | 3 × 18 ✓ | ✓, rolled back (server: 0 rows; 30,000 on completion) | ✓ | ✓ / ✓ / ✓ | 22/22 | 25006 ×3 | ✓ | ✓ |
| pg17 | 17.12 | 3 × 18 ✓ | ✓, rolled back (server: 0 rows; 30,000 on completion) | ✓ | ✓ / ✓ / ✓ | 22/22 | 25006 ×3 | ✓ | ✓ |
| pg18 | 18.7 | 3 × 18 ✓ | ✓, rolled back (server: 0 rows; 30,000 on completion) | ✓ | ✓ / ✓ / ✓ | 22/22 | 25006 ×3 | ✓ | ✓ |
| mysql80 | 8.0.46 | 3 × 14 ✓ | ✓ | ✓ | ✓ / ✓ (`INSERT … SELECT`, no row) / ✓; `SLEEP` completes | 14/14 | 25006 ×3 | ✓ | — |
| mysql84 | 8.4.13 | 3 × 14 ✓ | ✓ | ✓ | ✓ / ✓ (`INSERT … SELECT`, no row) / ✓; `SLEEP` completes | 14/14 | 25006 ×3 | ✓ | — |
| mariadb106 | 10.6.28 | 3 × 14 ✓ (JSON as text) | ✓ | ✓ | ✓ / ✓ (`INSERT … SELECT`, no row) / ✓ (sequence engine); `DO` completes | 14/14 (gap: `LOCK TABLES`) | 25006 ×3 | ✓ | — |
| mariadb1011 | 10.11.19 | 3 × 14 ✓ (JSON as text) | ✓ | ✓ | ✓ / ✓ (`INSERT … SELECT`, no row) / ✓ (sequence engine); `DO` completes | 14/14 (gap: `LOCK TABLES`) | 25006 ×3 | ✓ | — |
| mariadb114 | 11.4.13 | 3 × 14 ✓ (JSON as text) | ✓ | ✓ | ✓ / ✓ (`INSERT … SELECT`, no row) / ✓ (sequence engine); `DO` completes | 14/14 (gap: `LOCK TABLES`) | 25006 ×3 | ✓ | — |
| sqlite | 3.54.1 | 3 × 8 ✓ | ✓ | ✓ | ✓ / ✓ (`CREATE TABLE AS` leaves nothing) / ✓ | 10/10 (`23`, the authorizer) | 23 ×3 | ✓ | — |
| pg17-tr | 17.11 | Turkish text both ways, `tr_TR` order and case mapping, the server's Turkish message (`… nesnesi mevcut değil`), equal to the side channel's message for the same statement (review M4) | | | | | | | |

`pr` runs the same tests on pg17, mysql84 and SQLite. The Turkish test skips there, because
pg17-tr is a `full` line.

**Tests:**
- `lines_are_classified_from_the_targets_never_from_a_list` (no Docker)
- `typed_golden_tables_per_engine_and_version` (its post-condition is `assert_freed`, the
  server's view; review M5)
- `select_streams_suspends_fetches_more_and_closes`
- `stop_fetching_keeps_rows_and_frees_the_session`
- `cancel_within_one_second_on_every_engine`
- `read_only_bypass_rows_blocked_through_run_without_classifier`
- `production_console_rejects_insert_on_every_engine`
- `turkish_collation_text_round_trips_on_pg17_tr`
- `incomplete_cursor_leaves_no_transaction_in_pg_stat_activity`
- `dml_and_ddl_report_update_counts_on_a_dev_console`

## m3b-t01c — core seams

M3b Task 1c's run-service and result-store seams. Every stub fails closed (F31) and none is
wired into Run before Task 24 (R-M3-52).

- **Gate (Task 15).** `gate()` denies every unit: `exec-gate-stale-classification` when the
  unit's report does not match its `final_text` (P-M3b-2, `SafetyReport::matches_text`), else
  `exec-gate-not-implemented`. `ConnSafety`, `GateReason`, `GateDecision` (no form for
  `RunInTxThenConfirm`: the session picks it, P-M3b-1), `GateItem`, `GatePlan`,
  `ConfirmPort` (`Debug` supertrait), `ConfirmQuestion`/`ConfirmAnswer`/`CommitQuestion`.
  `explain_analyze_inner` answers `None`; `SuppressSet` works.
- **Params (Task 15).** `prepare_units` refuses the first unit holding a `:`, `$` or `?` byte
  (`ParamDiag::TooManyBinds`, at its document offset); every other unit is `Ready` as written
  with an identity `OffsetMap` (a working piecewise-linear map).
- **Classifier pool (Task 14).** `ClassifyPool::classify` answers `sql-safety`'s stub at once;
  as a `StatementClassifier` it denies every statement and knows no effects. Nothing installs
  it before Task 18.
- **Run service.** `RunRequest` gains `prepared`, `plan`, `policy`, `run_as_one`, `history`,
  `confirm` (P-M3b-22: every M3a literal sets `None`/`StopOnError`/`false`). A request with
  `prepared`, `plan` or `run_as_one` is refused before anything is sent (`never_started`,
  `Internal`) until Task 16: ignoring a plan could send what the gate denied. `policy` reaches
  the session. `with_history`, `with_classifier`; `count` answers `Err(Internal)` and
  `count_sql` `None` (Task 16). The units keep M3a's shape (`RunUnit::plain`).
- **Log.** `LogMsg` gains `Notice`, `ResultSetLimit`, `ImplicitCommit`, `SafetyDenied`,
  `ConfirmCommitted`, `ConfirmRolledBack`, `ConfirmStatementFailed`, `SkippedByUser`, `Retried`,
  `Evicted`, each with a literal Fluent ID; their strings come with the tasks that write them
  (none is written before). `SessionLog::lines_since(seq)` answers the lines from `seq` and
  whether some left the ring first. `DenyLayer::Classifier` → `exec-deny-read-only-classifier`;
  the new error kinds have titles (`status.rs`, named exception).
- **Auditor chain (Task 16).** `SyncAuditor`, `AuditorChain` (the synchronous half works; no
  sinks yet), `SINK_CAPACITY` = 10,000.
- **History (Task 17).** The types, and `HistoryStore::open` answering
  `Err(HistoryError::Disabled)` with nothing touched on disk. `rusqlite` (`bundled`) has FTS5
  (`fts5_is_compiled_in`).
- **Results (Task 8).** `CloseReason::Evicted`, `ResultSource::{cap_raised, set_visible}`
  (defaults `false` and nothing), `MemoryBudget::evict_for` (stub `0`), `EvictionRegistry`
  (weak stores and their visibility; nothing evicts yet).
- **Values (P-M3b-17).** `format_cell` dispatches `Purpose::Tooltip` to `valfmt/tooltip.rs`
  (Task 38) and `Purpose::A11y` to `valfmt/a11y_text.rs` (Task 40); both answer the cell's text.
- **Settings (P-M3b-9).** `[exec]`, `[results]`, `[history]`, `[grid]` are hand-loaded like
  `[editor]`; `results.max_value_bytes` is a `ValueLimit` clamped to 64 KiB..=64 MiB with a
  `settings-invalid-value` diagnostic; an M3a file loads with no diagnostic and saves unchanged.
- **Ruling I2.** The app takes no `sql-safety` dependency: `hacivat-exec` re-exports
  `ClassifyCtx`, `Expect`, `FnCatalog`, `GateClass`, `Limits`, `Reason`, `ReasonCode` and
  `SafetyReport`, and the session's `CommitAnswer`, `ErrorPolicy` and `RunChoice`.

## m3b-t08 — eviction

- **Shared tail.** A result's rows and state (the chunk list, starts, counts, completeness)
  move from the writer into the store, behind a mutex the writer takes per append or state
  change. Eviction is the second publisher: it cuts a result to its first chunk under the same
  mutex, so versions only grow and no second copy of the chunk list keeps evicted rows alive.
  Readers still never lock (one `ArcSwap` load).
- **Who is evicted (R-M3-68, HV-RES-07).** `admit` asks `MemoryBudget::evict_for(shortfall)`
  before it answers `Pause(GlobalBudget)`, and pauses only if that did not make room. The
  budget's `EvictionRegistry` (every `ResultStore` joins it in `ResultStore::new`) takes
  hidden results whose run has ended (`Complete`, `Closed` but not `Evicted`, `Cancelled`,
  `Truncated`) and that hold more than one chunk, longest out of sight first (a logical clock,
  no ties). A `Streaming` or `Suspended` result is never taken: its session would keep
  appending, and only the session closes a server cursor. The evicted result keeps chunk 0 and
  publishes `Closed { reason: Evicted }`; its writer's later appends and states are dropped,
  so it never reads as `Complete` again.
- **Visible by default.** A result joins the registry visible; it becomes a candidate only
  when the app calls `StoreSource::set_visible(false)` (Task 25: `true` for the active tab of
  every console, `false` for the others). A console that never reports visibility loses
  nothing — its results are simply never evicted (the run pauses as in M3a). Rejected: hidden
  by default, which would evict an on-screen result until Task 25 is wired.
- **Accounting (P-M3b-29).** `evict_for` answers the bytes that left `used()` during the call:
  a chunk a held snapshot still shows is released when that snapshot drops and is not
  counted, so the pass goes on to the next result. A held snapshot keeps every row it showed:
  a copy or a frame in progress never changes under it. The pass holds the registry lock, so a
  tab shown during it is either spared or finds the result evicted.
- **Not in M3b.** HV-RES-07's 30-second grace after a tab is hidden and the pinned-result
  exemption: the order (longest hidden first) stands in for the grace, and pins are Task 25's
  (a pinned tab can stay `set_visible(true)`). Spill to disk stays v1.x.
- **Coalescing (ledger 109 M4).** The ask ledger moves from `StoreSource` into the store, so
  every source over one store shares it: per snapshot version the asks add up to what the
  farthest reader needs, a need already asked asks nothing, and a farther need asks only for
  the rows past those asked. The session merges queued `FetchMore`s of one result into one
  fetch of the sum (`Ctl::coalesce`), so no row is fetched twice.
- **Sources.** `StoreSource::cap_raised` reads the store's flag (ledger 253 M7).
  `FetchPolicy::with_cell_cap` takes `results.max_value_bytes` (Task 25 passes it) clamped to
  `MIN_CELL_CAP` 64 KiB ..= `MAX_CELL_CAP` 64 MiB, the cap of the value viewer's "Re-run with a
  64 MiB value limit" (R-M3-82).
- **Review fix round 1.**
  - *Only what is needed (M1).* `admit` evicts until its chunk fits, and the room is re-checked
    under the registry lock before each result. The check counts bytes already cut but not yet
    freed (`releasing`) as free. Two runs over the budget at once cut one result, not two.
  - *Frees outside the lock (M3).* Under the lock the pass only cuts and collects. It drops
    the cut chunks, and the stores it upgraded, after releasing the lock, so `set_visible` on
    the UI thread never waits for a large free.
  - *Previous outcome (M4).* `StoreSnapshot::before_eviction()` keeps the state an evicted
    result had ended in, so the strip can still name it beside "Closed to free memory".
  - *Notification seam (M6).* `MemoryBudget::on_evicted(listener)` is called once per
    eviction with `evict::Eviction { result, kept_rows, previous }`. The call comes after the
    lock is released and the chunks are freed, on the evicting thread. Listeners must not
    block. The run service turns it into `LogMsg::Evicted` (Task 16).

## m3b-t17 — history

- **Store (R-M3-62, `alg/20` HV-EXEC-12).** `hacivat-exec::history`: SQLite + FTS5 at
  `LocalPaths::history_db()`, schema version 1 (`PRAGMA user_version`), WAL, a 5 s busy
  timeout and `BEGIN IMMEDIATE` writes, so several windows (or two processes) share the file.
  One writer task owns the connection and runs every call on the blocking pool; searches,
  recall and pins go through it, after the records already queued, so a search sees what was
  sent before it. A batch that fails (disk full, a lock past the timeout) is dropped. History
  is never an audit trail: it drops records when full, busy or broken.
- **Secrets (HV-EXEC-12 step 2).** The writer redacts before it hashes, fingerprints, caps or
  stores. No dialect rides on a record, so the scan reads the text with and without backslash
  escapes and masks the union. It masks every string after `PASSWORD`/`PASSWD`/`PWD`/
  `SECRET`/`IDENTIFIED` (or a word with such an `_` part, `MASTER_PASSWORD`) up to `;`, the
  bare value in value position (Oracle `IDENTIFIED BY tiger`, `REPLACE`, `SET PASSWORD FOR …
  =`), any string carrying `password`/`pwd=`/`secret` or a URI password, and psql
  meta-commands to the end of the line through `HvNorm1` (`\connect ?`). The values found
  are also removed from the error message (MySQL 1064 quotes the statement) and the
  parameter text. `redacted` is set when anything was masked.
- **Hash and fingerprint are the store's.** `text_hash` is SHA-256 of the whole redacted
  statement and `fingerprint` is `HvNorm1` (generic dialect) of it; the record's values are
  ignored. Rejected: storing the producer's hash of the sent text, a guessing oracle for the
  password it held.
- **Caps and retention (D-061).** Statements over `statement_cap` (64 KiB) keep a prefix ending
  on a character boundary and the hash of the whole; error messages 4 KiB. Dedup by
  `(data source, text_hash)` increments `run_count` and moves the entry to the front (latest
  start, outcome and metrics). Per data source the newest 10,000 unpinned entries stay, and
  unpinned entries last run more than 90 days ago go: after each batch for the touched
  sources, at open, and every 10 minutes, in batches of 1,000; `auto_vacuum = INCREMENTAL`
  gives the pages back. Pinned entries never expire and do not count against the cap.
- **Search.** FTS5 (`unicode61 remove_diacritics 2`) over two shadow columns, the folded
  statement (`search_key`) and the folded error message (`error_key`, an addition to the
  `alg/20` schema so Turkish error text is found the same way). The fold is a fixed table
  (`İ`, `I`, `ı` → `i`; `ş` `s`, `ğ` `g`, `ç` `c`, `ö` `o`, `ü` `u`; ASCII capitals; combining
  marks U+0300–U+036F dropped), never locale casing, applied to both sides. The user's text is
  never FTS5 syntax: its words (letters and digits) become quoted prefix phrases ANDed in one
  bound parameter; text with no word matches nothing. Recall is an exact prefix
  (`substr(…) = ?`, never `LIKE`). `HistoryQuery` gains `fingerprint` ("Similar").
- **File.** Created `0600` before SQLite opens it (SQLite gives `-wal`/`-shm` the same mode);
  a wider mode on an existing file is tightened. The directory is the app's (T1a review M7).
  `PRAGMA quick_check` at open: a file SQLite calls not a database or corrupt is renamed
  `history.sqlite.corrupt-<unix seconds>` with its `-wal`/`-shm` (an old log is never replayed
  into the new file) and a fresh file is made; `recovered_from()` tells the app, once. A busy
  file is an error, never a rotation.
- **Who is not recorded.** A private console never reaches the sink (Task 16); a profile with
  "do not record history" is dropped by the run service (Task 24) and by the writer too
  (`set_do_not_record`).
- **Shutdown.** `shutdown(grace)` writes what is queued, checkpoints the log and closes within
  the grace (2 s); later calls answer `Disabled`.
- **Review fix round 1 (redaction).**
  - *Comments follow the reading (I1).* The MySQL reading takes `#` as a line comment, `--`
    only before whitespace, block comments that do not nest, and the bodies of `/*!…*/` and
    `/*M!…*/` as code; the standard reading nests block comments. A comment's body is read as
    code by the same rules, and `password=` values and URI passwords in it are masked in place
    (m1). An apostrophe in `# Bob's` or in a nested PostgreSQL comment no longer hides what
    follows.
  - *Quoted names (I2).* `"password" '…'` and `` `passwd` = '…' `` arm like the bare word.
  - *Turkish keyboard (I3).* Keywords are compared through a fixed table (ASCII capitals;
    `İ`, `ı` → `I`), so `İDENTİFİED` and `ıdentıfıed` arm. No locale casing.
  - *Functions (m2).* Every string argument of `crypt`, `pgp_sym_encrypt`/`decrypt`
    (`_bytea`), `pgp_pub_decrypt` (`_bytea`), `encrypt`/`decrypt` (`_iv`),
    `AES_ENCRYPT`/`AES_DECRYPT` and `DES_ENCRYPT`/`DES_DECRYPT` is masked and scrubbed from
    errors; so is `set_config`'s value when its name has a secret part.
  - *Also.* The corrupt file's `-wal`/`-shm` are moved before the file itself, so a crash in
    between never leaves an old log beside a fresh file (m7); `temp_store = MEMORY` (m13);
    only FTS5 syntax errors read as "no match" (m9).
  - *Tested by* the review's hostile probe, kept as a table
    (`review_probe_every_case_is_caught_or_a_documented_gap`, 87 rows): each row is caught
    (statement, error, parameters, search and the file's bytes) or is a gap whose phrase the
    module docs must contain.
- **What redaction does not catch.** Known limits: a secret written this way can reach the
  history file. m3–m5 are on the PO list.
  - *Other secret words (m3):* `TOKEN`, `API_KEY`, `ACCESS_KEY`, `PRIVATE_KEY` and other
    names without a `password`/`passwd`/`pwd`/`secret` part (`OPTIONS (token '…')`);
    SQLCipher `PRAGMA key`/`rekey` and `ATTACH … KEY '…'`; MySQL user variables without a
    secret part in their name (`SET @pw = '…'`, then `IDENTIFIED BY @pw`).
  - *URIs (m4):* a password with an unencoded `/`, `#`, `?`, `'` or whitespace; a scheme-less
    `user:password@host`; shell command lines such as `COPY … PROGRAM 'curl -u u:pw …'`.
  - *Error echoes (m5):* only whole secret values are scrubbed from errors. An echo cut short
    (MySQL `near '%.80s'`) keeps the part it shows, and a string masked only because it follows
    a secret word (the second of two adjacent strings, `PASSWORD 'a' 'b'`) is not removed
    from the error.
  - *Bind values* quoted back by the server: the store never sees them; Task 16 scrubs them.
  - *Free text and other functions:* a secret in prose (`-- the pw is …`), in a function not
    listed (`hmac`, user-defined functions), or built at run time
    (`'… PASSWORD ' || quote_literal(v)`).
  - *Depth:* code quoted in code, and comments, are read to three levels.
  - *Over-masking* is accepted: every string after a secret word in its statement, the next
    non-keyword word after `PASSWORD`, whole strings and comment words near `password`/`secret`,
    and error text such as `password authentication failed for user …` (m6, not changed).

## m3b-t16 — run service

- **Two paths.** A `RunRequest` without `prepared`, `plan` and `run_as_one` is M3a's run,
  unchanged. With `prepared` and `plan` it is an M3b run: each unit is sent as
  `PreparedUnit::final_text` with its binds, the gate item's verdict and effects, its masked
  text and `max_result_sets: 100`; `run_as_one` sends `Protocol::Simple`, never suspended.
- **The plan is checked, never assumed.** It must hold one item per prepared unit in order,
  and the prepared units must be the request's units. `Run`, `RunInTxThenConfirm` and
  `ExplainAnalyzeRollback` are sent. A unit the user skipped is not sent: `apply_answer` turns
  its `Confirm`/`Warn` into `Deny(exec-skipped-by-user)`, logged once as `SkippedByUser { n }`
  and counted in the summary's skipped (R-M3-84). An unanswered `Confirm`/`Warn`, any other
  `Deny`, a short or empty plan, or `plan`/`prepared`/`run_as_one` alone sends nothing. An
  answer never turns a `Deny` into a run.
- **Questions on the ticket.** `RunTicket::choices` carries the pending `NeedsChoice` or
  `NeedsCommit` with the run's request number, and clears when the run moves on (`UnitStarted`,
  `UnitEnded`, `CommitDecided`, `RunEnded`). A question asked before the request number is
  known is published as soon as it is. A `NeedsCommit` also goes to `RunRequest::confirm`
  (none answers `RollBack`). The port's answer goes back with
  `SessionHandle::commit_decision(seq, unit, question, ..)`, for exactly that question.
  `CommitQuestion::production` is `true` (confirm-in-transaction exists only on production,
  P-M3b-30). `connection` is empty: the port belongs to its console and names it.
- **Tabs and titles (R-M3-66).** One tab per result set, `TabKey { console, unit, set }`. The
  title is the unit's directly preceding comment, without its markers, its last comment and
  first line (`ExecUnit::title`, set by `resolve_units` from `UnitSpan::title_comment`); else
  `Result N`, where N counts the run's units that opened a tab. Further sets add ` (k)`.
  `ResultTab::title` keeps the whole title (the tooltip). `short_title()` cuts it after 40
  graphemes with `…`. Clusters are approximated without a segmentation table, since the crate
  takes no new dependency: combining marks, variation selectors, emoji modifiers, tags, ZWJ
  joins and regional-indicator pairs. A prepared unit past 100 sets logs
  `ResultSetLimit { total, limit: 100 }`; M3a's units keep `MoreResultSets`.
- **Count rows (R-M3-76).** `count_sql` reads the unit's tokens. The first word must be
  `SELECT`/`WITH`/`VALUES`/`TABLE`. A locking clause, `INTO`, a second statement, an executable
  comment, a client command or a lexer error refuses it. A trailing `;` and trivia go. A
  top-level `ORDER BY` goes unless a top-level `LIMIT`/`FETCH`/`OFFSET` follows. The count is a
  separate query sent only by `ExecService::count`, never implicitly. The read-only check
  (`StatementClassifier::check_read_only`; the pool's from `with_classifier`) must allow it;
  without a check it is refused (fail closed). It runs on a pooled `Service` session when the
  console holds no transaction of the user's, else on the console (after its cursor is
  released, as for any run). It runs as a `Request::Run`, so a read-only or production
  session's guard applies unchanged (R-M3-52). It is cancelled after 30 s
  (`CountError::TimedOut`) and can be cancelled earlier (`CountTicket::cancel`).
- **Auditor chain (R-M3-63).** Every event goes first to the synchronous auditors in order,
  then is `try_send` to each sink. A full sink counts a drop and never waits. A closed sink is
  ignored. `RowsFetched` reaches the sinks at most once per 50 ms per result. The output log
  stays the console's bounded, non-blocking `SessionLog` ring, written by the observer as in
  M3a, so its order and timing are M3a's. The chain's drops end the run as one
  `Dropped { n }` line.
  The chain is an event pipeline, not an audit trail. What a sink drops is gone, and the
  history is never evidence of what ran.
- **History sink.** There is one record per `UnitEnded` of a unit that ran, sent when
  `RunRequest::history` is `Some` and not private. A profile that records no history gets
  `None` from Task 24, and the store's `set_do_not_record` drops its records as well. The
  record holds `PreparedUnit::masked` only, never the text sent. Its `error_message` has every
  bind value scrubbed out: longest first, and a value of one or two bytes only where it stands
  alone. `text_hash` and `fingerprint` are left zero, since the store computes both from its
  own redacted text (Task 17). A record is never `{:?}`-formatted.
- **Log lines.**
  - *Notices and MySQL warnings.* These are `Notice` lines. The server's text stays verbatim in
    its own field, and the code is a chip, not a sentence argument. The session reads MySQL
    warnings (`RawConn::warnings`) after a statement whose `Done` counted some, on read-write
    sessions only (T13 review M-8). Notices past the cap are `NoticesDropped { n }`, carried by
    `RunEvent::NoticesDropped`.
  - *Run as one.* A stop that closed it early is `ClosedEarly { ran, rest_not_run }`, from
    `take_close_report` via `RunEvent::ClosedEarly`. A PostgreSQL failure adds
    `RunAsOneRolledBack`, because it is one implicit transaction (T13 review M-2).
  - *Other events.* A retried statement logs `Retried`. An eviction of one of the run's results
    logs `Evicted` on its console, through `MemoryBudget::on_evicted`.
  - *DDL.* A DDL statement ends `exec-completed` on every engine (first word `CREATE`/`ALTER`/
    `DROP`/`TRUNCATE`/`RENAME`/`COMMENT`, or the gate's DDL effect; PO, ledger 535). The unit's
    status keeps the server's count.
  - *Command tag.* A line keeps its statement's command tag (`LogLine::tag`, copied as
    `tag=…`; ledger 340).
- **One duration rule (PO, ledger 535).** `duration_message` gives `exec-dur-ms` under a
  second, else `exec-dur-seconds` with two decimals, cut as the status bar's
  `hv-status-duration-*` cuts them. The app formats the hundredths through
  `UnitMsg::decimal`. The running clock keeps whole seconds and minutes.
  `exec-dur-lt-ms` and `exec-dur-s-ms` are deleted: no code names them any more.
- **Fix round 1 (review of e1bc91c).**
  - *Titles (I-1).* A comment title never carries what could re-order or restyle the chrome.
    - C0/C1 controls (a lone `\r`, ESC, NUL, tab), U+2028/2029, bidi marks, embeddings, overrides and isolates, and U+FEFF become white space.
    - Runs of white space become one space.
    - The stored title is cut after 256 graphemes with `…`; the tab shows 40.
    - An optimizer hint or executable comment (`/*+`, `/*!`, `/*M!`) is never a title.
  - *Two named exceptions to the one duration rule (m2; on the PO list).*
    - Configured spans (`LogArg::SpanMs`, the idle bound) read the way the setting reads: whole seconds under a minute ("20 s"), then minutes and seconds ("10 min").
    - The running clock shows whole seconds, then minutes and seconds. It repaints once a second.
  - *Server text in the log (m3, ruling).* Server error and notice text stays verbatim in the console's output log, even when it echoes a bind value.
    - The log is session-only and in memory, and nothing persists or traces it.
    - The value was typed by the same user in the same session, so the exposure equals that of a literal in the editor.
    - Scrubbing it would hide which value failed.
    - History, the only store that outlives the session, is scrubbed.
    - Anything that ever writes the log to disk (an export, a crash report) must apply the same scrub.
  - *Not-run units (m5).* A unit that ends without starting (a queued run cancelled, a full queue) still gets a history record: at the run's start, with duration 0 and no rows, masked text as always. `HistoryRecord` has no flag of its own for this.
  - *Count rows (m6–m8).*
    - `count_prepared` wraps a prepared unit's `final_text` and carries its binds (`CountRequest::binds`) in slot order. A top-level `ORDER BY` holding a placeholder is kept. `count_sql` refuses placeholders, since it has no values.
    - The cancel and the 30 s timeout cover the whole count: the read-only check and the pooled session's connect too.
    - Dropping the ticket, or keeping only `done`, never cancels the count.
  - *PostgreSQL Run as one (m9).* The line is chosen from the user's transaction status when the unit started (the session's at the run's start, then each `TxChanged`).
    - Outside a transaction, a failure or a cancel logs `exec-run-as-one-rolled-back`.
    - Inside one, nothing was rolled back yet, so it logs `exec-run-as-one-tx-failed`.
  - *Stopped early (m10).* `ClosedEarly` is logged only when more statements ran; the stop's own line covers the rest.
  - *Read-only sessions (m11).* MySQL warnings on a read-only session are not read, and their count is not logged either; the guard's `ROLLBACK` follows at once.
  - *The output log is not a chain sink (m12, ruling).* The observer writes the output log directly, keeping M3a's order and timing, instead of through an async chain sink. The M8 chain sinks (`hacivat-audit`) will consume the same `RunEvent`s through `AuditorChain::push_sink`.

## m3b-t15 — gate

- **Order (R-M3-55).** Per unit, the first rule that decides wins: a stale report (no report,
  a report count that differs from the unit count, or `matches_text(final_text)` false;
  P-M3b-2) is denied; the classifier stub's `NotImplemented` pre-decision is denied on every
  connection (F31); a read-only connection runs only `Read`, or `Session` on the harmless-`SET`
  list, and only when `decide(.., ReadOnlyPlusExplain, Human)` allows (R-M3-56); on
  production `decide(.., Admin, Human)` must not deny; then the D-062 wrap; then the class row;
  then type-to-confirm, the MySQL/MariaDB implicit commit and suppression. Production stays
  read-only until Task 26 because its consoles are read-only connections: the gate is gated as
  read-only first, so it unlocks nothing (R-M3-52).
- **The production AND-check.** `decide(Admin)` is the profile whose rules R-M3-55 does not
  contradict (`ReadWriteDml`/`Ddl` would deny what R-M3-55 runs or confirms). Its `Deny`
  denies (its first reason is the gate's reason: a parse error, a lexical-mode change, a role
  switch, an ambiguous boundary). Its `Confirm` on a class whose own `admin` row allows (`Read`,
  `Lock`, `Dml`, `DmlUnsafe`, `DdlOther`, `DdlDestructive`) means it saw evidence the gate
  class did not, so the unit becomes `Unknown` (Confirm). The request-level class, effects and
  flags also floor the class through `gate_class` of a synthetic statement, so they never lower
  it. Off production the AND-check does not run: R-M3-55's "everything else runs" and the
  server decides.
- **Type-to-confirm** comes from the destructive kinds, not the class (a `DROP` in the same
  unit hides `DmlUnsafe`): every unsafe-DML target and every `DROP DATABASE/SCHEMA` target.
  The name is the user's spelling (each part's bytes as written, quotes removed, never folded):
  the pattern's range when it reads as the target, else the first dotted name in the
  statement whose parts `ident_eq` the target's, else the classifier's spelling (PG targets
  are server-resolved, T9 carry). A pattern without a target, or two different targets, deny
  (`exec-unsafe-no-target`, `exec-unsafe-several-targets`): typing one name never approves
  another table.
- **Implicit commit.** `SessionEffects::DDL` is set for every DDL statement (temporary ones
  included on PostgreSQL; on MySQL/MariaDB/TiDB see fix round 1: temporary `CREATE`/`DROP`
  excluded) and, on MySQL/MariaDB/TiDB, for privilege statements. With a transaction open as the gate sees it (any status but idle,
  or a Manual session) the unit is `Confirm` with verdict `ImplicitCommit`, never suppressible;
  `irreversible` is set for every such DDL on those engines.
- **Verdicts.** `Read` only when the report reads only (`reads_only`: every statement's class
  field, effects and gate class `Read`, none opening a write, the request-level fields agreeing);
  `Routine` and `Unknown` are `Unknown` with every session effect but `DDL`, never `Write`.
  R-M3-23 is ANDed through `units::suspendable_for` (`apply_read_verdicts`), the same condition
  as the `Read` verdict: `RunUnit.suspendable = prepared.unit.suspendable && verdict == Read`.
- **D-062.** A lone `EXPLAIN ANALYZE` whose inner class is not `Read` is wrapped; an `Unknown`
  inner gets the wrap; `Routine`/`Admin` inners are refused, and so are DDL inners on
  MySQL/MariaDB/TiDB (their implicit commit would end the wrapping transaction). One inside a
  unit of several statements is refused (the rollback would undo the other statements).
- **Suppression.** `(console, fingerprint)` for a suppressible `Confirm` or `Warn`, production
  included (NOTES ruling 4); never with type-to-confirm or an implicit commit.
- **Parameters.** `prepare_units` refuses the first unit with any diagnostic but the
  informational `QuestionMarkOff`, or a placeholder without a rewrite (any unbindable
  placeholder: nothing is sent), and a bind plan with a slot no name fills (`$2` alone). Values
  come from the store; one prompt per distinct name of the run. `masked` masks every literal and
  comment of a text that mentions a password (`password`, `identified`, `passwd`, `pwd=`), the
  rest of the text after an unclean token; bind values never appear in the text.
- **Run as one.** `RefuseReason::RunAsOne` is gone: a `Verbatim` unit resolves like any other
  and runs as one (`protocol_for` → `Protocol::Simple`, `expect_for` → `Expect::Script`).
- **Fix round 1 (controller rulings on the review).**
  - *Not analysed (I1).* A pre-decided report, or one with no statement, is denied on
    production and read-only connections. Off production it never runs silently: when the
    unit's words hold a write or DDL keyword (`INSERT`, `UPDATE`, `DELETE`, `MERGE`, `REPLACE`,
    `DROP`, `TRUNCATE`, `ALTER`, `CREATE`, `GRANT`, `REVOKE`, `CALL`, `DO`, `LOAD`, `COPY`,
    `INTO`, `EXEC`, … including the words inside MySQL executable comments, and the rest of the
    text after a token the lexer could not read), it asks one calm confirmation
    (`exec-gate-not-analysed`, never suppressible); otherwise it runs, so dumps such as
    `/*!40101 SET NAMES utf8 */` keep working. A `Verbatim` unit whose only reason is
    `AmbiguousBoundary` follows the same rule (R-M3-60 on production is denied: PO list).
  - *D-062 on production (I2, M7).* An `Unknown` inner, a unit the AND-check made `Unknown`,
    and on MySQL/MariaDB/TiDB every wrapped inner ask before the wrap: `Confirm` with verdict
    `ExplainAnalyzeRollback`, never suppressible, naming what a rollback does not undo
    (sequence advances, session advisory locks, dblink and other file or network I/O, writes to
    non-transactional MyISAM tables; `exec-explain-analyze-not-undone`). The wrap itself is
    unchanged off production.
  - *DROP DATABASE/SCHEMA (I3).* The gate also reads `DROP DATABASE|SCHEMA [IF EXISTS] <name>`
    from the statement's words and asks for the typed name whatever kind the backend gave the
    target; no readable name denies (`exec-unsafe-no-target`). The interim backend (named
    exception in T10's `interim.rs`) now gives a `DROP` target its object kind (`Database`,
    `Schema`, `Table`, `View`, `Role`, `Sequence`, `Other`).
  - *Parameters (I4, M2, M5, D8 ruling).* Remembered values only pre-fill the prompt: a
    statement with parameters always returns `NeedsValues` first, and only this run's
    `ParamAnswers` (`prepare_units_with`) make it `Ready`. Ordinal and anonymous parameters
    (`?`, `?NNN`, `$n`) are never remembered (`ParamPrompt::rememberable`) and belong to their
    unit (`ParamPrompt.unit`): two units never share one prompt. A bind plan with a slot no
    placeholder fills (`$2` alone) is refused before any prompt.
  - *Implicit commits on MySQL/MariaDB/TiDB (I5, M3).* One list in
    `hacivat_exec::implicit_commit`, from MySQL 8.4 Reference Manual §15.3.3 "Statements That
    Cause an Implicit Commit" (8.0: §13.3.3) and the MariaDB Knowledge Base page "SQL statements
    That Cause an Implicit Commit": DDL except `CREATE`/`DROP TEMPORARY TABLE` (which do not
    commit), `GRANT`/`REVOKE`, `CREATE`/`ALTER`/`DROP`/`RENAME USER`, roles, `SET PASSWORD`,
    `BEGIN`/`START TRANSACTION`, `LOCK`/`UNLOCK TABLES`, `SET autocommit = 1`,
    `ANALYZE`/`CHECK`/`OPTIMIZE`/`REPAIR TABLE`, `FLUSH`, `RESET`, `CACHE INDEX`,
    `LOAD INDEX`, `LOAD DATA`/`XML`, `INSTALL`/`UNINSTALL`, replication start/stop/change.
    The gate sets `SessionEffects::DDL` for exactly these statements on those engines, which is
    how the session's T19 backstop (`TxError::ImplicitCommitUnconfirmed`) reuses the list.
    `irreversible` covers their DDL (temporary `CREATE`/`DROP` excluded) and privilege
    statements.
  - *One SessionEffects mapping (M8).* `gate::session_effects_of(report, text, engine)` is the
    single mapping; the classifier pool's `StatementClassifier::effects` (Task 14) delegates to
    it at the merge.
  - *Suppression (controller ruling, M4, M9).* On production "don't ask again" never covers
    `DdlDestructive`, `Admin`, `Routine` or `Unknown` units (`Routine` added because R-M3-55
    treats it as `Unknown`); a unit of several statements with a write is never suppressible;
    the key (`GateItem.fingerprint`) mixes the fingerprint with the class.
- **Fix round 2.**
  - *Two bits (R1-I1, controller ruling).* `SessionEffects::COMMITS_FIRST = 128` (named
    exception in `hacivat-session` `verdict.rs`) is set on MySQL/MariaDB/TiDB for exactly the
    statements of `implicit_commit::commits` (plus privilege statements and DDL other than
    `CREATE`/`DROP TEMPORARY TABLE`); the gate's implicit-commit rule and T19's backstop key on
    it. `DDL` means "a DDL statement" again on every engine, temporary-table DDL included, so
    T16's "schema change" reading holds (`LOAD DATA` keeps its row count). The round-1 bullet
    that put the list into `DDL` is superseded.
  - *Raw words (R1-M1).* An unanalysed report's scan reads every ASCII word of the text, strings
    and comments included, so a lexing-mode difference (`NO_BACKSLASH_ESCAPES`) cannot hide a
    write; over-counting asks more, never less. Added keywords (R1-M2): `COMMENT`, `NOTIFY`,
    `XA`, `BINLOG`, `RESTART`, `CLONE`, `ANALYZE`, and `SET GLOBAL`/`PERSIST`/`PERSIST_ONLY`/
    `@@global.`.
  - *Listed schemas (R1-M3).* `DROP SCHEMA a, b` read from the words names no single name, so it
    is denied on production (`exec-unsafe-no-target`).

## m3b-t21 — confirmation dialogs and the app's ConfirmPort

- **Closes answer the safe value, by construction.** The confirmation (`exec_ui::confirm`) and
  the confirm-in-transaction prompt (`exec_ui::commit_prompt`) own their `oneshot` reply and
  send it once. Escape, the close button and ⌘W (Ctrl+W) answer Cancel / Roll back; a dialog
  dropped without an answer (its window closed, the app quit) answers it from `Drop`; the quit
  guard answers every open one first (one named line in `close_prompt::quit_guard`, so a quit or
  a window close never leaves a question pending). Enter acts on key down on the focused button
  only, never in the type field; the initial focus is the safe answer.
- **The typed name is compared as bytes.** `name_matches` is `typed.as_bytes() == name.as_bytes()`
  with the name exactly as `GateItem` carries it: no case or Turkish İ/i folding, no trimming,
  no Unicode normalisation (an NFD spelling fails closed; the Copy button copies the exact bytes
  for a name the keyboard can't reproduce). Names, reason arguments and the SQL are drawn made
  visible (bidi and invisible characters as `\u{…}`), so a name cannot reorder the sentence it
  sits in.
- **"Don't ask again" is checked twice.** The gate's flag (which already refuses typed names,
  implicit commits and multi-statement units with a write) and the dialog's own rule
  (`offers_suppress`: never with a typed name, never for `exec-gate-not-analysed` or an implicit
  commit, and on production never for `DdlDestructive`, `Admin`, `Unknown`, `Routine` or an
  unknown class). `answer_plan` returns the item's class-mixed `fingerprint` only when the dialog
  offered it.
- **Answers change decisions only.** `answer_plan` goes through `apply_answer`, so an item's
  verdict (`ExplainAnalyzeRollback`, `ImplicitCommit`), effects, class and fingerprint reach the
  run unchanged. Execute all (RC-1) answers only later units that would ask without a typed name
  and never a `RunInTxThenConfirm` unit; Skip all skips every later unit that would ask.
- **The port reads the console at question time.** `AppConfirmPort::for_console` sends each
  question over an `mpsc` to one foreground task (the `AppUiPort` pattern); the task fills the
  connection name and the production flag from the console's live profile (T16 m13: the run
  service sends `production: true` and an empty name), or answers the safe value when the console
  is gone. `follow(choices, sql)` makes an open commit prompt close once its question is no longer
  the one the run waits on (`CommitDecided`, T1c M7); the outcome is the session's event.
- **The countdown runs on the executor's clock**: the time left is read once from the session's
  `Instant` deadline, then counted on `background_executor().now()` and repainted when its `m:ss`
  text changes (once a second). At 0:00 the prompt answers Roll back; the session's own deadline
  is what undoes the statement (D-014).
- **Fix round 1.**
  - *Arming (I1).* Every button that runs or keeps a change (Execute, Execute all, "don't ask
    again", Commit, Keep) answers only `ARM_DELAY` (400 ms, executor clock) after its dialog was
    shown, by click or by Enter, and never on a click whose `click_count > 1`. The safe answers
    are live at once. Chained dialogs of the same shape put Execute at the same pixel, so the
    second click of a double-click must never reach the next dialog.
  - *Runs, not slots (I2).* The port's book keys follows and losses by `RunId`
    (`follow(run, choices, status, sql)`, `connection_lost(run, n)`); a commit question attaches
    its own run's follow whenever it arrives and never another run's. Once its question stops
    waiting, the run's own `RunStatus` decides: a unit failed with a lost connection shows the
    lost form, an ended unit or run closes the prompt. An explicit loss shows the lost form in
    any order and supplies the statement count when it comes later.
  - *Stack-aware focus (I3).* `Workspace::remove_layer` gives the focus back only when the top
    layer closes; a layer removed from under another hands its restore target to the layer above
    (named exception in `shell/{workspace,layers}.rs`).
  - *Production is never downgraded (m2):* the port's production flag is
    `question.production || profile is production`.
  - *Untypeable names (m8):* a name with a control character (a tab or a line break in a quoted
    identifier) cannot be typed or pasted (the field turns controls into spaces), so it can never
    be confirmed. That stays fail-closed, and the dialog says why
    (`hv-confirm-type-untypeable`). PO list: whether such statements need another way to run.
  - *IME (m7):* Escape during a composition in the type field ends the composition (its text
    stays) and keeps the dialog; the next Escape cancels.

## m3b-t25 — results: tabs per result set, pinning, Count rows, notices, eviction in the app

- **Titles.** A result tab shows its statement's comment as the run service gave it (Task 16
  sanitised it), as data, else the console's `Result N` (N per statement, shared by its further
  sets). The app tells a comment from the service's own `Result n[ (k)]` by recomputing n over
  the run's tab list (`tabs::comment_of`); a comment that reads exactly so is shown as the
  console's `Result N`. A title an earlier tab already shows gets `(2)`, `(3)` …
  (`hv-results-tab-dup`): further sets and repeated comments alike. Titles cap at 190 px (120 px
  in a short strip, measured each frame; NOTES ruling 30), the whole title in the tooltip.
- **Pinning.** A pinned tab is never reused: `apply` reuses only an unpinned tab of the key
  (`tab_by_key` prefers it); a tab already showing the result is left alone, so a pinned tab and
  its successor may share a key. The strip facts are keyed by key and unit for the same reason.
  Re-run prefers the active tab (named exception in `exec_ui/orchestrator.rs`).
- **Visibility (Task 8 carries).** Every tab reports `ResultSource::set_visible`: true at once
  while it is the console's active tab, pinned, or claimed by another view
  (`set_view_visible`, for Task 38); false only after `HIDE_GRACE` (30 s, executor clock) with
  no claim. An evicted result, once shown again, moves its adapter to a new generation over the
  same result, which clears the grid's selection to its focus cell; a copy request reaching past
  the kept rows is refused with `hv-results-copy-evicted`. `cap_raised` is read from the source.
- **Count rows.** Only the button sends it: the tab's unit (the run UI's `tab_units`) is lexed
  alone under the console's known context, wrapped by `count_sql` (no parameters, so no binds
  yet), and handed to `ExecService::count` on the live session and its connection's pool.
  Anything uncountable says so and sends nothing. No spinner: the static amber marker,
  "Counting…" and Cancel (design ruling 21). A count past the per-result cap makes Fetch all ask
  first ("Fetch up to the memory limit" focused, the bounded choice).
- **Log.** The output view reads `SessionLog::lines_since` (a change costs its new lines), keeps
  as many lines as the ring, and heads the list with one "n events not shown" row for lines the
  ring dropped unread. Notices are one row each with the server's text verbatim at the row's
  end; a MySQL error chip shows `vendor · SQLSTATE` from the new `ErrorView::vendor_code`
  (`code` stays the SQLSTATE the bypass rows judge by). ↑/↓/Enter are element-level keys.
- **Untyped results.** A result whose every column is text of `NativeType::Unknown` is marked
  "untyped (run as one)": PostgreSQL's simple protocol; MySQL's text protocol still types its
  columns, so it is not marked; nor is a column of another logical type (a fake driver's).

## m3b-t28 — conformance

**What the matrix is.** `crates/core/hacivat-exec/tests/docker_m3b_matrix.rs` holds twelve
Docker tests (the E-rows of the M3b gate), with their helpers in `tests/common/mod.rs`
(`// ## m3b-t28-matrix`, module `t28`). Each case goes the app's M3b way:

1. a console document in the context the console read at bind;
2. `resolve_units` (Execute3), or one `Verbatim` unit sent as written (Run as one);
3. `prepare_units`, every prompt answered by name (`prepare_units_with`);
4. the real `ClassifyPool` (`classify_units`) over exactly the prepared bytes, in the
   session's own context read from the server (lexing-mode setting, PostgreSQL's search path or
   the MySQL family's `DATABASE()`, the server version; `guard_ctx_in`, modes known);
5. `gate` on the live connection (the session snapshot's transaction status, owner and mode),
   with `Confirm`/`Warn` answered through `apply_answer`;
6. `ExecService::run`, a real session from a `ConnectionManager` with `ManagerConfig::m2b()`, and
   the typed driver.

An admin side channel (M3a T31's `matrix::Side`) checks what reached the server after each
case.

- **Lines.** These come from `matrix::lines()`: the `targets(Some(engine))` properties, never a
  list. Under `full` they are PostgreSQL 14–18, MySQL 8.0 and 8.4, and MariaDB 10.6, 10.11 and
  11.4; under `pr` they are pg17 and mysql84; SQLite (a file per test) runs in every set.
  - Every test ends in `Table::finish`. It fails when a line of the active set recorded nothing,
    and it writes `t28-<test>.tsv`.
  - Per-engine differences are explicit expectations, never skips. Examples: no `CALL` on
    SQLite; PostgreSQL's Run-as-one cells are `Text` (the simple protocol names no type, T13);
    `EXPLAIN ANALYZE` is a syntax error on MariaDB and SQLite; no notices on SQLite.
- **Production writes.** A production console here is `ManagerConfig::m2b()`, which keeps
  production writable as Task 26 will. The production policy flag is never named (P-M3b-7). A
  production profile starts in Manual (`tx::default_mode`); the test asserts that and sets Auto
  for the own-form legs.
- **The 2 s deadline.** `ExecService::run` always sends D-014's 5 minutes, so the deadline legs
  use `Con::run_with_deadline`. It sends the same prepared units, with the gate's verdicts and
  effects, straight to the session with a 2 s `confirm_deadline` (the T31 precedent of a pass
  through `SessionHandle::run`). That reply is `Tx(ConfirmTimedOut)`, and the events carry
  `CommitDecided { RollBack, by_deadline: true }`.
- **Read-only, layer by layer.** The cases below run on a read-only console whose guard asks
  the real pool:
  - The gate denies `INSERT`, and the run service refuses a denied plan: nothing executed or
    failed, every unit `Skipped`, the log's "not started" (fix round 1, I2).
  - With the gate bypassed, the session's first-keyword input layer refuses it (DEMO-7, outcome
    `input`).
  - A write that starts like a read is denied by the gate. With the gate bypassed, step 0b
    denies it (`exec-deny-classifier`):
    - PG: a writable CTE;
    - MySQL: `WITH … DELETE` (fix round 1, M2: a real write, not a locking read);
    - MariaDB: `SET STATEMENT … FOR DELETE` (MariaDB has no `WITH … DELETE`);
    - SQLite: `WITH … INSERT`.
  - With a classifier that allows everything (`AllowAll`), the server's own layers stop the
    same statement:
    - PostgreSQL and the MySQL family answer `25006`, the read-only transaction (the MySQL
      family's 1792, by its SQLSTATE);
    - SQLite answers `23`, the read-only connection's authorizer.
  - On PostgreSQL, a read that the real classifier allows still writes through a view over a
    `VOLATILE` function. The server stops it with `25006`.
  - The side channel's row count is unchanged after every step. Every statement of these
    legs would change that count if the server ran it, so the count is evidence on every
    engine.
- **Mutation check (plan Step 2).** Undo's `ROLLBACK TO SAVEPOINT` was replaced by `RELEASE`
  only in `session/confirm.rs`. `confirm_in_tx_both_forms_per_engine` then went red on pg17
  ("Undo keeps the user's transaction": `Failed`, not `InTx`). The mutation was reverted.
- **Case 12** (`physical_memory_bytes_is_known`) is not in this file: `hacivat-exec` may not
  reach `hacivat-ui-core` (layering). `hacivat-ui-core`'s
  `physical_memory_is_known_on_this_platform` runs in `build-test` on all three OSes.
- **Case 11** runs both offline and on Docker. Task 15's
  `gate::gate_end_to_end_over_real_classify` is no longer `#[ignore]`d (a named exception in
  `tests/gate.rs`), and the Docker case runs the same shapes on live production, development and
  read-only consoles of every line. `DROP TABLE`/`TRUNCATE` on production ask without a typed
  name (R-M3-55 types only unsafe-DML targets and `DROP DATABASE`/`SCHEMA`, which the rows
  include).

**Fix round 1 (review I1, I2, M1–M5, M7).**

- **pg17-tr (I1).** `turkish_cases_on_pg17_tr` runs on the `pg17-tr` twin
  (`lc_messages = tr_TR.UTF-8`) through `require_target`, and fails under `full` when the target
  is missing (M3a's pattern). It covers:
  - Turkish text binds stored byte for byte (side channel) and found by a Turkish bind;
  - a notice in Turkish (`tablo "…" mevcut değil, atlanıyor`) that still names the table;
  - Ask decided by the SQLSTATE `42P01` under a Turkish message;
  - production's typed name for `DELETE FROM hv_t31_m3b_müşteri_…`, which equals the table's
    exact bytes;
  - a Turkish-identifier `UPDATE … WHERE` through confirm-in-transaction, visible only after
    Commit.
- **A denied plan sends nothing (I2).** `denied_plan_sends_nothing_on_production`: a production
  console in Auto runs as one `DELETE FROM a; DELETE FROM b`.
  - The gate denies it (`exec-unsafe-several-targets`). Nothing later would stop it: the session
    is writable and the server would run it.
  - The test asserts the refused shape (executed 0, failed 0, every unit `Skipped`, "not
    started"), and the side channel still sees both tables' rows.
  - Mutation: the run service sending a `Deny` unit (`service.rs`, `Planned`) turned this test
    and case 7 red on pg17, mysql84 and SQLite. The side channel went from (2, 2) to (0, 0). The
    mutation was reverted.
- **D-062 (M1).**
  - PG and MySQL run the savepoint form inside the user's transaction: the explained `DELETE` is
    undone, and the user's own `INSERT` and transaction stay (4 rows after the user's COMMIT).
  - MariaDB and SQLite have no `EXPLAIN ANALYZE`: the server answers a syntax error, so their
    "rows kept" cell proves nothing about the wrap ("n/a" below).
  - **MariaDB `ANALYZE <dml>`** really executes its statement. By the controller ruling, the
    interim backend (`sql-safety` `backend/interim.rs`, named exception) classifies
    `ANALYZE INSERT|UPDATE|DELETE|REPLACE` as the bare statement. `ANALYZE SELECT` and
    `ANALYZE FORMAT=…` stay `Unknown`. The gate then treats it exactly like the bare DML:
    `run` on development (`WHERE`), `tx-confirm` on production.
  - On Docker, production's `ANALYZE DELETE` asks. Roll back leaves 3 rows; Commit leaves 0,
    showing that it executes.
  - The T20 seed for `ANALYZE DELETE FROM t` (MariaDB) moved from admin `deny` to `allow`, a
    named exception in `classifier_seeds.toml`. Read-only stays `deny`. PO list: MariaDB
    `ANALYZE <dml>` and D-062.
- **A multi-table `DELETE` names every target (found in this round).**
  - Before: MySQL and MariaDB `DELETE a, b FROM a JOIN b …` and `DELETE FROM a, b USING …`
    reported a no-`WHERE` pattern on `a` only, so typing `a` approved deleting from `b` too.
  - Now the interim backend reports every listed table (named exception, unit test). The gate
    then denies the several targets on production.
- **Side channel in case 1 (M3).** Each hint's bound row equals its literal row as the server's
  own text, and the NULL bind is stored as NULL.
- **Typed names (M4).** Case 11 asserts the exact bytes of production's typed name: `müşteri`,
  `s` (`DROP SCHEMA`) and `d` (`DROP DATABASE`).
- **Artifacts (M5).** Every `t28-<test>.tsv` row now holds measured values: outcomes, codes,
  counts, end states. No row is a constant description.
- **E-row mapping for T30 (M7).**
  - E5's 100-result-set cap is not on Docker; T11 and T16 cover it offline.
  - E2's "DmlUnsafe, DDL and Admin send nothing before an answer" is not on Docker here, since
    every `Planned::answer` is Execute. T30 cites T26's Docker test for that clause.
  - E4, E5 (multi-result, Run as one, error policy) and E2's confirm-in-transaction clauses are
    this file.
- **Carried to T24's merge (M6, controller):** the classify context is still rebuilt by the
  helper. It should switch to T24's session-held source.

| Case (test) | pg14–18 | mysql80, mysql84 | mariadb106, 1011, 114 | SQLite |
|---|---|---|---|---|
| 1 `binds_round_trip_every_type_hint` | 14/14 hints stored equal + NULL | 11/11 + NULL | 11/11 + NULL | 9/9 + NULL |
| 1 `value_with_quote_is_bound_not_spliced` | stored as written | same | same | same |
| 2 `multi_result_call_and_run_as_one` | CALL 1 tab (INOUT); as one 3 text tabs | CALL 3 tabs; as one 2 | CALL 3 tabs; as one 2 | as one 2 |
| 3 `error_policy_ask_ignore_stop_and_rollback_and_continue` | Ignore, Stop, Roll back and continue (Auto, Manual) | Ignore, Stop, Ignore in tx (Auto, Manual) | same as MySQL | same as MySQL |
| 4 `confirm_in_tx_both_forms_per_engine` | own ×3, savepoint ×4, Manual keep+undo, Smart | same | same | same |
| 5 `smart_mode_opens_only_before_writes` | read: none; write: opened | same | same | same |
| 6 `explain_analyze_dml_is_rolled_back` | wrap, ran, rows kept; savepoint form in a user tx | same | n/a (no `EXPLAIN ANALYZE`: 42000); `ANALYZE DELETE` gated as bare DML, rollback 3 / commit 0 | n/a (no `EXPLAIN ANALYZE`: 1) |
| 7 `read_only_classifier_denies_and_server_layers_hold` | gate (refused shape), input, 0b, server 25006, view 25006 | gate, input, 0b, server 25006 (`WITH … DELETE`) | gate, input, 0b, server 25006 (`SET STATEMENT … FOR DELETE`) | gate, input, 0b, authorizer 23 |
| 8 `notices_and_warnings_reach_the_log` | 2 notices | 1 warning | 1 warning | none |
| 9 `count_rows_matches_count_star` | 20, 15 | 20, 15 | 20, 15 | 20, 15 |
| 10 `type_goldens_match` | 72 values (T7 file) | 31 values (T7 file) | 31 values (T7 file) | 8 values |
| 11 `gate_end_to_end_over_real_classify` | 9 × 3 consoles, typed `müşteri`, `s` | 9 × 3, typed `müşteri`, `d` | 9 × 3, typed `müşteri`, `d` | 5 × 3, typed `müşteri` |
| (I2) `denied_plan_sends_nothing_on_production` | refused shape, rows (2, 2) | same | same | same |

| pg17-tr | `turkish_cases_on_pg17_tr` |
|---|---|
| binds | 5 Turkish words stored byte for byte; found by `İstanbul` |
| notice | `tablo "hv_t31_m3b_yok_ğ_…" mevcut değil, atlanıyor` |
| Ask | unit 1, `42P01` |
| production | typed name = the table's bytes; `UPDATE … WHERE` own form, `İstanbul` → `Ömer` after Commit |

## m3b-t25 — results, fix round 1

- **Crowded strips (I1).** A result tab never shrinks: it is drawn whole or not at all. The
  strip is fitted to last frame's width: every tab whole, else every tab in its short form
  (ruling 30), else the short form of as many as fit, in order, the active tab always among
  them, and a "+N" button whose menu lists the rest by their whole titles. Every drawn tab,
  with its environment swatch, is therefore inside the strip. Text widths are cached per
  string (m12) and the titles computed in one pass.
- **Titles (m8, m9).** The comment comes from the unit (`ExecUnit::title`, the run UI's
  `tab_units`); the service's title is read back only for a result the run UI does not know. A
  repeat's `(k)` is its own label after the cut title (`hv-results-tab-repeat`); the whole
  title (`hv-results-tab-dup`) stays the accessible name.
- **Lifecycle (m4–m7).** An eviction the shown tab has not answered is answered after the
  tile's frame; a closed tab is hidden to eviction at once; a count stops when its console's
  results drop; the toolbar's Clear empties the view directly.

## m3b-t24 — execution wiring in the app

- **One pipeline, nothing sent before every step agreed** (`exec_ui::orchestrator`, section
  `m3b-t24-exec-ui`). Run's resolved units (as previewed) → `prepare_units` with a fresh
  `ParamAnswers` (the parameter prompt on `NeedsValues`; Cancel sends nothing) → the classifier
  pool's `classify_units` on exactly the prepared `final_text`s (P-M3b-2: the gate denies a
  stale report) → `gate` with the connection's live `ConnSafety` → the confirmations in unit
  order (`confirm::ask_plan_at`, R-M3-84) → one `ExecService::run` with the answered plan. Each
  early end logs a `LogMsg::RunUi` line (a refusal with its reason and arguments, or the user's
  answer), marks the refused units' gutter `Failed` (its tooltip the reason, its end-of-line
  words "not sent" or M3a's "not sent: read-only"), and says it once (toast and
  announcement). Every unit is either confirmed, sent, or refused visibly.
- **Off the UI thread.** Preparation and the gate run on the background executor; the
  classification on the pool's 16 MiB threads (the UI's foreground task awaits a `oneshot`). A
  Run being prepared holds the console like a run in flight (one at a time; its gutter shows
  nothing yet).
- **The real context: the session's own (plan amendment C2(b)/I1, T18 carries; fix round 1,
  I1/I2).** Run asks the session it will run on for its classification context first
  (`SessionHandle::classifier_context`, a queued `Request::ReadContext`): the lexing-mode
  setting, the search path (MySQL/MariaDB: the current database) and the server version, kept
  **per session**, so every console that shares a session gets the same one. A read-only
  session answers what its guard's classifier is told (T18); a read-write session reads T18's
  own constant text (`readonly::read_context_again`: PostgreSQL `current_setting(...)`, split
  by `schema::split_search_path`; MySQL/MariaDB `@@SESSION.sql_mode, DATABASE()`), a read
  that cannot fail in an open transaction, run outside the guard and not counted as a statement
  of the user's transaction (`observe(false)`). **Never stale:** the question is answered in
  queue order, and every request that may change the context (a user statement, a run with a
  unit not proved a read, a transaction command, a schema switch, a reset, a reconnect) marks
  it stale before the next request is taken; a stale one is read again before it is answered.
  **Fail-closed:** in a failed transaction, or while a suspended cursor holds the connection,
  nothing is read and the answer is unknown; a Run that gets no answer within 10 s classifies
  with unknown modes. The units were split in the console's context: when the session now reads
  text in other modes (the `standard_conforming_strings` example: `SELECT '\'\n' ; DELETE FROM
  t; --'`), nothing is sent (`exec-run-context-changed`) and the console reads its context
  again. The context is asked again after the prompt and the confirmations, just before the
  send: if another console on the same session changed it meanwhile, nothing is sent (same
  line). **The context's generation (fix round 2):** the session's answer carries a
  generation that moves with every stale-marking (marked before the request runs, so a request
  that never finishes still moves it); the run carries the generation of the last answer
  (`RunSpec::context_generation`), and the session refuses a run whose generation has moved,
  sending nothing (`ExecErrorKind::ContextChanged`, `exec-run-context-changed`): another
  console's request queued between that answer and the run cannot change the context the run
  was classified in. **Unproven engines (fix round 2):** on Redshift and Cockroach the read is
  not sent while a user transaction is open (not proven to succeed there); the answer is then
  unknown. PostgreSQL, Timescale and Yugabyte read. A function on the profile's
  `read_only_fn_allowlist` is a proved read and never marks the context stale: the list is the
  user's declaration that it only reads. **One context per run (M12):** the parameter scan and the classification both use it
  (`orchestrator::classify_ctx_in` → `classify_pool::run_ctx_in`, exactly `guard_ctx_in`'s
  rules). Count rows checks in the context of the session it runs on, read from that session.
  Within one run, a later unit is classified in the context a literal `USE db` (MySQL/MariaDB)
  or `SET search_path TO <literal list>` (PostgreSQL) of an earlier unit sets; any other
  context change leaves the later units `Unknown` (`sql-safety` `classify_units`, fix round 1,
  I5).
- **Unknown modes stay fail-closed for parameters too**: the units are prepared under every
  reading of the modes, and a run whose parameters read differently is refused
  (`exec-param-modes-unknown`).
- **Production stays read-only until Task 26 (R-M3-52).** `ConnSafety.read_only` is the
  session's read-only flag **or** a production profile: the gate denies every non-`Read` unit
  there, whatever the session underneath (the guard still applies). Task 26 deletes that one
  rule in `conn_safety`.
- **Run as one (R-M3-60).** A unit run as one is a `Verbatim` unit: a selection under the
  console's "As one" policy, or the uncertain chooser's "Run as one" row, whose region is the
  execution statements holding the uncertain boundaries (Run 3: the whole text; a selection: the
  selection), without outer white space, refused when it cuts a token or reaches a client command
  or `COPY` rows. The row is never preselected (`PopupMenu::clear_selection`, named exception);
  the band shows the region while it is highlighted; on a read-only session it is disabled with
  its reason. It is classified with `Expect::Script`; a unit run as one whose verdict would be
  `TxThenConfirm` or `ExplainAnalyzeRollback` is denied (`exec-gate-run-as-one-refused`, the T19
  carry: the session refuses them). MySQL/MariaDB: the gate does not model a Smart
  transaction an earlier unit of the run opens, so DML then DDL in one Smart run is refused by the
  session (`ImplicitCommitUnconfirmed`): fail-closed (T19 m8).
- **CHOOSE-06.** The console's `RunPolicy` (inside: Ask / Smallest / Largest / Whole script; a
  selection as separate statements or as one) is kept until the console closes and read by Run
  (`resolve_units_in`) and its band (`preview_with`, the service's `set_run_policy`) through one
  table, `hacivat_sql_editor::run_policy`, so the preview-truth rule holds under any policy
  (property `preview_equals_execution_under_the_console_policy`).
- **Parameters (R-M3-79).** Values are remembered per `(profile, name)` for the app session
  (`ParamMemory`, never persisted), only for named parameters; they pre-fill the prompt and
  never skip it. Values are read machine format first: a canonical integer gets an integer
  hint, `\x`+hex pairs bytes (malformed `\x` refused on every engine, the T13 M-6 carry), the
  rest text (decimals stay exact). A dot-grouped number where the format locale groups with a
  dot is ambiguous until edited or confirmed ("Use 1.234"). **Known limit (fix round 1, M11):**
  a value that begins with `\x` or `\X` is never sent as text (read as bytes, or refused as
  bad hex); a "send as text" choice is left for later.
- **The run's questions.** After `run`, the port follows the run (`follow(run, choices,
  status, masked)`), a `NeedsChoice` goes to the error bar (`error_bar::on_choice`, Task 23), a
  `NeedsCommit` disables the transaction badge (`set_choice_pending`) and marks its unit
  "waiting for Commit or Roll back" (or Keep or Undo). Every loss of a run that had such a
  question reaches the port (`connection_lost`, with the statements before the confirmed one in
  the savepoint form), whichever of the loss and the question's end the app sees first (fix
  round 1, I3: `LossLink`; the listener outlives the run by 30 s).
- **The observer's derived lines (T19 carry).** `PlannedUnit` carries the gate's verdict: an
  `ImplicitCommit` unit that ends the user's transaction logs `exec-implicit-commit` (instead of
  the generic "the transaction ended" line), and a `TxThenConfirm` unit that fails or is
  cancelled in an open transaction before any question logs `exec-confirm-statement-failed`.
- **History (T17 carries).** The store opens on the core runtime at boot (its integrity check
  never holds the window) in the private `data/` directory (`create_dir_all_private`); records
  go through a channel the run service writes from the start and are forwarded once the store
  answers; a store recovered from a corrupt file says so once; a profile with
  `record_history == false` gets no `HistoryMeta` and is marked do-not-record in the store too.
- **Tests' hook (T1c M7).** `ClassifyPool::observed(threads, hook)`: the hook sees each text and
  its context on the pool thread before the real classifier reads it (Task 24's
  `classifier_receives_final_text`, `classification_never_runs_on_the_ui_thread`).
- **Refusals say what happened to every unit (fix round 1).** When the gate refuses a unit,
  nothing of the run is sent, and each other unit gets its own mark and log line naming the
  refused statement (`exec-gate-run-refused`, I4). A selection run as one that holds a parameter
  is refused before any prompt (`exec-param-run-as-one`, M1: one simple-query message carries
  no bind values); the "As one" policy on a read-only session is refused at the gate
  (`exec-gate-run-as-one-read-only`, M2: the chooser's reason). A console whose session changed
  between the gate and the send says so (`exec-run-connection-changed`, M4). Run as one trims
  only SQL white space (M3). The gate reads the profile as it is at gate time (M5). A history
  store that cannot open is logged (M9).
- **A carried context switch must take effect (fix round 2, R1-C1).** When a unit before the
  last is a literal `USE` / `SET search_path` whose new context the later units were classified
  in (`sql_safety::carried_switches`), the run is sent with `ErrorPolicy::StopOnError` whatever
  the error-policy setting says (no Ignore, Skip or Retry past its failure), and skipping it at
  a confirmation cancels the run (nothing sent): otherwise a failed `USE app` would run
  `SELECT * FROM session` against the old database although it was classified as a read of
  `app.session`.

## m3b-po — PO answers to the M3 decision list (M3b-04, M3b-23, M3b-26)

- **`TRUNCATE` and `DROP TABLE` type the table's name on production (PO M3b-04; R-M3-55
  amended).** The gate's typed name (`typed_name`) now also takes the target of every
  `Truncate` pattern and of every `Drop` of a table: the pattern's target is a table, or the
  statement's own leading words read `DROP [TEMPORARY|TEMP|FOREIGN] TABLE` (so a backend that
  names the kind wrongly cannot drop the typed name, as for `DROP DATABASE`/`SCHEMA`);
  `ALTER TABLE … DROP PARTITION` keeps its row. It fails closed like the unsafe-DML rule: no
  table to name is `exec-unsafe-no-target`, and more than one name in one unit
  (`TRUNCATE a, b`, `DROP TABLE a, b`, `DELETE FROM a; TRUNCATE b`) is
  `exec-unsafe-several-targets` (typing one name must never approve another). Off production
  nothing changes. Views, indexes, functions, sequences and column drops keep a plain
  confirmation.
- **Limit of the typed name (fix round 1, review m4; narrowed by m3c-p2-07).** The typed name
  needs a target the classifier can see. A `TRUNCATE` or `DROP TABLE` hidden from it — inside a
  `DO` block, dynamic SQL (`EXECUTE '…'`), `dblink_exec(…)`, MySQL `PREPARE`/`EXECUTE`, or a
  routine a `CALL` runs — gets the plain production confirmation instead: never suppressible,
  nothing sent before the answer, but no name to type.
- **`DROP OWNED BY` types the role's name on production (## m3c-p2-07, PO P2-07, approved
  2026-11-18).** PostgreSQL's `DROP OWNED BY <role> [, …] [CASCADE | RESTRICT]` (which drops
  every object the role owns in the database, every table included) now names each role as its
  `Drop` pattern's target (`ObjectKind::Role`, where the role is written), and the gate's typed
  name takes it when the statement's own leading words read `DROP OWNED`. The rules are the
  `TRUNCATE`/`DROP TABLE` ones: production only (`EnvTag::Production`), never suppressible,
  off production nothing changes; the name is typed as written (quotes removed, letter case
  kept, compared byte for byte). `CURRENT_USER`, `CURRENT_ROLE`, `SESSION_USER` (and `PUBLIC`)
  are not resolved to a role name — Hacivat has no way to — so the keyword is typed as written.
  The multi-target rule is the one `DROP TABLE a, b` follows: one name per unit, so
  `DROP OWNED BY a, b` (or two `DROP OWNED` statements, or a `DROP OWNED` beside a `TRUNCATE`
  or unbounded `DELETE`) is denied, `exec-drop-owned-several-targets`; the same role twice is
  one name. A role and a table written alike, or a keyword and a quoted role spelled alike
  (`"current_user"`, `current_user`), are two objects, never one name. A `DROP OWNED` whose role
  the classifier could not name is denied, `exec-drop-owned-no-target`. The dialog's reason is
  `safety-destructive-drop-owned[-cascade]` ("Removes every object `r` owns in this database
  …"), never `safety-destructive-drop` ("Removes `r`.", which `DROP ROLE` keeps).
- **More secret forms are masked in history (PO M3b-23; narrowed in fix round 1).** Secret
  name parts are now also `BEARER`, a bare `TOKEN` or a `TOKEN` after a secret-ish qualifier
  (`access_token`, `refresh_token`, `api_token`, `auth_token`; never `token_type`,
  `token_count`), and a `KEY` after an allow-listed prefix (`API`, `ACCESS`, `SECRET`,
  `PRIVATE`, `ENCRYPTION`, `MASTER`, `SIGNING`, `AUTH`, `CLIENT`, `SERVICE`, `LICENSE`, `HMAC`,
  `SESSION`) — never surrogate, table or public keys (`customer_key`, `date_key`,
  `idempotency_key`, `public_key`, `PRIMARY KEY`). They apply to words, quoted names,
  `set_config` names, `name=value` and JSON pairs (`"access_token": "…"`, camelCase too)
  inside strings. SQLCipher's `PRAGMA key|rekey|hexkey|hexrekey|textkey` (also after
  `EXPLAIN [QUERY PLAN]`), `ATTACH … KEY` and SQLite URI keys (`?key=`, `&hexkey=`,
  `textkey=`) take a value; a MySQL user variable named like a secret (`@pw`, `@db_pass`,
  `@key`; never `@pass_count` or `@@…`) takes a value after `=` or `:=`, and
  `SELECT '…' INTO @pw` masks the strings before it; `Bearer <credential>` in a string is
  masked. A type name after a secret column in DDL is never its value.
- **The error scrub replaces whole secrets of at least 4 bytes only (fix round 1, review I1).**
  Before, a secret `1` or `int` turned `ERROR 1054 … at line 1` into `ERROR ***054 … at line
  ***`. A secret glued to other word characters in an echo, or shorter than 4 bytes, now stays
  in the error text (documented gap).
- **The Execute all scope line names the real environment (PO M3b-26).** Execute all is never
  offered on production, so the line's old "a production change still waits for your answer"
  clause could never apply; it is gone, and the line says which environment the connection is
  (`hv-conn-env-*`, "no environment" untagged).
