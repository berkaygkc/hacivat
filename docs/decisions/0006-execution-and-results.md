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
