//! Typed streaming on SQLite (HV-RES-04 SQLite; Task 15): a typed query is one job on the
//! connection's blocking thread, stepping a prepared statement into `hacivat_values::Chunk`s
//! through [`crate::decode`], with the declared column types (`column_decltype`) as the native
//! type. The grammar and the session's rules are `hacivat_driver::typed`'s.
//!
//! **The job** (`Job::run`). `Statement<'conn>` lives on the job's stack, so nothing borrows
//! across threads:
//! 1. prepare: on a read-only profile the authorizer judges every action here, then
//!    `sqlite3_stmt_readonly` must hold (`readonly-required`, the journal-mode query exempt),
//!    exactly as `execute` (`exec.rs`); a statement with a parameter is `Unsupported` (binds
//!    are M3b; an unbound parameter would run as NULL); the cancel latch is read;
//! 2. `Started`: `query` returns here, so a statement SQLite refuses fails `query` itself;
//! 3. step rows into a `ChunkBuilder` (`with_schema`), sealing a chunk at `first_rows` rows (the
//!    first one) or `chunk_rows` (later ones), at `chunk_bytes`, or when the builder is full, and
//!    send each one on a channel of depth `EVENT_DEPTH`: a consumer that stops reading stops
//!    the stepping (the memory bound);
//! 4. a suspendable query, at the read-ahead point (`first_rows` rows, then each
//!    `fetch_more(n)` batch of `n`), steps one row more: none left → the result ends (no cursor
//!    and no lock left behind); a row → it is held as the next batch's first, `Suspended` goes
//!    out, and the job blocks on its command channel for `FetchMore(n)` or `Close`;
//! 5. at the end: `ResultEnd` (rows, `sqlite3_changes` for a statement that writes, the first
//!    keyword as the tag), the statement is finalized, then `Done` with the transaction status.
//!
//! A statement without columns (DML, DDL) is one result with an empty schema. The job ends when
//! its command channel closes (the connection was dropped: the statement is finalized and the
//! thread moves on), when the consumer drops the event channel, or with the cancel latch, which
//! covers the whole sub-loop: the progress handler and `sqlite3_interrupt` stop a step, the job
//! reads the latch after prepare and when a batch resumes.
//!
//! **Kinds decided by the first value.** The buffer of a column whose declared type does not
//! fix it (BLOB affinity, expressions, NUMERIC) follows the first non-NULL value of the first
//! chunk ([`crate::decode`]). Until every such column has one, rows are kept as whole values;
//! once all are known (usually at the first row), they are replayed into the builder and the
//! rest are pushed directly. `ResultStart` goes out once the schema is known.
//!
//! **Closing.** `Close(Now)` (and `AfterKill`, which SQLite never needs) finalizes at once.
//! `Close(Drain(d))` steps on, counting, until the result ends (`Completed`) or `d` passes
//! (`Closed`); the latch's deadline makes the progress handler stop a long step at `d` too. The
//! job answers a `Close` after it finalized, so the lock is gone when `close_query` returns. A
//! `Close` that arrives while rows stream is seen at the next row, so a `Close(Now)` during one
//! long step waits for it: the session cancels first when it cannot wait (the progress handler
//! and the interrupt end the step). A deadline past what `Instant` holds is no deadline.
//!
//! **Idle bound** (ER6, R4). A live statement holds a SHARED lock in a rollback-journal mode,
//! so no other process can commit: the cursor's bound is 30 s (`ROLLBACK_IDLE_BOUND`). In WAL
//! it only keeps a checkpoint from completing: `None` (the session's own bound). The bound is
//! cached per connection: read at the open and on `probe`, and re-read after a statement whose
//! text names `PRAGMA`, `ATTACH` or `DETACH` (`may_change_journal`), the only statements that
//! change a journal mode or the list of databases; never on the first-chunk path. Another
//! process can only switch a file into WAL while this connection is open (out of WAL needs the
//! file to itself), so a stale bound is the cautious 30 s until the next `probe`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use hacivat_driver::{
    CloseHow, Closed, QueryDone, QueryEvent, QuerySpec, RawError, RawErrorKind, ResultEnd,
    TxStatus, TypedCaps,
};
use hacivat_values::{BufKind, ChunkBuilder, ColumnMeta, ResultSchema, Value};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, Rows};
use tokio::sync::mpsc;

use crate::decode::{self, ColKind, Column, Rule};
use crate::exec::{SqliteCancel, first_keyword, interrupted, readonly_required, tx_status};
use crate::harden::AuthTrail;
use crate::{gone, raw_error};

/// The idle bound of a cursor in a rollback-journal mode (ER6).
pub(crate) const ROLLBACK_IDLE_BOUND: Duration = Duration::from_secs(30);

/// The depth of a typed query's event channel: chunks in flight at most.
pub(crate) const EVENT_DEPTH: usize = 4;

/// What the SQLite typed path can do: suspend, no binds yet (M3b), closes by finalizing (no
/// server-side kill), and the journal mode's idle bound.
pub(crate) fn caps(idle_bound: Option<Duration>) -> TypedCaps {
    TypedCaps {
        suspend: true,
        binds: false,
        close_needs_kill: false,
        idle_bound,
    }
}

pub(crate) fn unsupported() -> RawError {
    RawError {
        kind: RawErrorKind::Unsupported,
        server: None,
    }
}

// ## T15 review round 1 (finding 4)
/// Whether `sql` may change a journal mode or the list of databases: it names `PRAGMA`,
/// `ATTACH` or `DETACH` anywhere (ASCII case folding; a word inside a literal re-reads for
/// nothing, which is cheap), so the cached idle bound is re-read after it.
pub(crate) fn may_change_journal(sql: &str) -> bool {
    ["PRAGMA", "ATTACH", "DETACH"].iter().any(|w| {
        sql.as_bytes()
            .windows(w.len())
            .any(|b| b.eq_ignore_ascii_case(w.as_bytes()))
    })
}

/// The idle bound of a cursor on `conn`: none when every database with a file is in WAL mode
/// (a database without a file, `:memory:` or `temp`, holds no lock another process sees), else
/// [`ROLLBACK_IDLE_BOUND`]; also when the modes cannot be read (fail closed: a bound).
pub(crate) fn idle_bound(conn: &Connection) -> Option<Duration> {
    match every_file_in_wal(conn) {
        Ok(true) => None,
        Ok(false) | Err(_) => Some(ROLLBACK_IDLE_BOUND),
    }
}

fn every_file_in_wal(conn: &Connection) -> rusqlite::Result<bool> {
    let mut schemas = Vec::new();
    conn.pragma_query(None, "database_list", |row| {
        let has_file = match row.get_ref(2)? {
            ValueRef::Null => false,
            ValueRef::Text(t) | ValueRef::Blob(t) => !t.is_empty(),
            ValueRef::Integer(_) | ValueRef::Real(_) => true,
        };
        if has_file {
            schemas.push(row.get::<_, String>(1)?);
        }
        Ok(())
    })?;
    for schema in &schemas {
        let mode: String = conn.pragma_query_value(Some(schema), "journal_mode", |r| r.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Ok(false);
        }
    }
    Ok(true)
}

/// What a job's last message carries, read after the statement is finalized: the transaction
/// status, and the idle bound when the statement may have changed it.
#[derive(Debug, Clone, Copy)]
struct After {
    tx: TxStatus,
    idle_bound: Option<Option<Duration>>,
}

impl After {
    fn apply(self, tx: &mut TxStatus, idle_bound: &mut Option<Duration>) {
        *tx = self.tx;
        if let Some(bound) = self.idle_bound {
            *idle_bound = bound;
        }
    }
}

/// From the job to the async side.
#[derive(Debug)]
enum Msg {
    /// Prepared and checked: the query runs.
    Started,
    Event(QueryEvent),
    /// The result ended and the statement is finalized.
    Done(QueryDone, After),
    /// The query failed; the statement is finalized.
    Failed(RawError, After),
    /// The answer to `Close`, after the statement is finalized: `Some(rows)` when a drain
    /// completed the result.
    Closed(Option<u64>, After),
}

/// From the async side to a job waiting at its read-ahead point (or streaming, for `Close`).
#[derive(Debug, Clone, Copy)]
enum Cmd {
    FetchMore(u32),
    Close(CloseHow),
}

/// How far a job has got: rows stepped, and whether it waits for room on its event channel.
#[derive(Debug, Default)]
pub(crate) struct Pace {
    pub(crate) rows: AtomicU64,
    pub(crate) parked: AtomicBool,
}

/// The spec's limits, normalized (a zero row count means one chunk's worth).
#[derive(Debug, Clone, Copy)]
struct Limits {
    first_rows: u32,
    chunk_rows: u32,
    chunk_bytes: u64,
    cell_cap: u32,
}

impl Limits {
    fn of(spec: &QuerySpec<'_>) -> Limits {
        let chunk_rows = spec.chunk_rows.max(1);
        Limits {
            first_rows: match spec.first_rows {
                0 => chunk_rows,
                n => n,
            },
            chunk_rows,
            chunk_bytes: spec.chunk_bytes,
            cell_cap: spec.cell_cap,
        }
    }
}

/// A typed query's work on the connection's thread.
pub(crate) struct Job {
    sql: String,
    suspendable: bool,
    limits: Limits,
    trail: Option<Arc<AuthTrail>>,
    cancel: Arc<SqliteCancel>,
    ticket: u64,
    events: mpsc::Sender<Msg>,
    cmds: mpsc::UnboundedReceiver<Cmd>,
    pace: Arc<Pace>,
}

/// How a job's statement ended.
enum Outcome {
    /// `ResultEnd` went out: `Done` follows.
    Ended,
    Failed(RawError),
    /// A `Close`; `Some(rows)` when a drain completed the result.
    Closed(Option<u64>),
    /// Nobody listens any more (the connection or the query was dropped).
    Gone,
}

/// What reading the next row gave, between the result's rows.
enum Step {
    Row,
    End,
    Stop(Outcome),
}

/// A typed query's job and its async side. `ticket` is the cancel latch's ticket, taken before
/// the job is sent.
pub(crate) fn start(
    spec: &QuerySpec<'_>,
    trail: Option<Arc<AuthTrail>>,
    cancel: Arc<SqliteCancel>,
    ticket: u64,
) -> (Job, Open) {
    let (events_tx, events) = mpsc::channel(EVENT_DEPTH);
    let (cmds, cmds_rx) = mpsc::unbounded_channel();
    let pace = Arc::new(Pace::default());
    let open = Open {
        events,
        cmds,
        stage: Stage::Streaming,
        delivered: 0,
        completed: None,
        #[cfg(test)]
        pace: Arc::clone(&pace),
    };
    let job = Job {
        sql: spec.sql.to_string(),
        suspendable: spec.suspendable,
        limits: Limits::of(spec),
        trail,
        cancel,
        ticket,
        events: events_tx,
        cmds: cmds_rx,
        pace,
    };
    (job, open)
}

impl Job {
    /// Runs the query on the connection's thread (module docs).
    pub(crate) fn run(mut self, conn: &Connection) {
        self.cancel.start(self.ticket);
        let outcome = self.query(conn);
        // The statement is finalized by now; a later cancel is not for this job.
        self.cancel.set_deadline(None);
        let tx = tx_status(conn);
        let after = After {
            tx,
            idle_bound: may_change_journal(&self.sql).then(|| idle_bound(conn)),
        };
        self.cancel.end(self.ticket);
        let last = match outcome {
            Outcome::Ended => Msg::Done(
                QueryDone {
                    tx,
                    warnings: 0,
                    skipped_result_sets: 0,
                },
                after,
            ),
            Outcome::Failed(err) => Msg::Failed(err, after),
            Outcome::Closed(completed) => Msg::Closed(completed, after),
            Outcome::Gone => return,
        };
        self.send(last);
    }

    /// Sends `msg`; `false` when nobody listens. Blocks while the channel is full.
    fn send(&self, msg: Msg) -> bool {
        if self.events.capacity() == 0 {
            self.pace.parked.store(true, Ordering::SeqCst);
        }
        let sent = self.events.blocking_send(msg).is_ok();
        self.pace.parked.store(false, Ordering::SeqCst);
        sent
    }

    /// Prepares, checks and streams the statement; it is finalized when this returns.
    fn query(&mut self, conn: &Connection) -> Outcome {
        if let Some(trail) = &self.trail {
            trail.clear();
        }
        let mut stmt = match conn.prepare(&self.sql) {
            Ok(s) => s,
            Err(e) => return Outcome::Failed(raw_error(e)),
        };
        // Text of only whitespace, comments and `;` prepares to no statement (`exec.rs`).
        let empty = stmt.expanded_sql().is_none();
        if empty && first_keyword(&self.sql).is_some() {
            return Outcome::Failed(raw_error(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_NOMEM),
                None,
            )));
        }
        if !empty {
            if let Some(trail) = &self.trail
                && !stmt.readonly()
                && !trail.only_journal_mode_query()
            {
                return Outcome::Failed(readonly_required());
            }
            if stmt.parameter_count() > 0 {
                return Outcome::Failed(unsupported());
            }
        }
        if self.cancel.cancelled(self.ticket) {
            return Outcome::Failed(interrupted());
        }
        if !self.send(Msg::Started) {
            return Outcome::Gone;
        }
        if empty {
            let schema = Arc::new(ResultSchema {
                columns: Arc::from([]),
            });
            let end = ResultEnd {
                rows: 0,
                kept: 0,
                rows_affected: None,
                command_tag: None,
            };
            return self.finish_empty(schema, end);
        }
        let columns = decode::columns(&stmt);
        let writes = !stmt.readonly();
        let before = conn.total_changes();
        let tag = first_keyword(&self.sql);
        let mut set = ResultSet::new(columns, self.limits, self.suspendable);
        let mut rows = stmt.raw_query();
        let stepped = self.stream(&mut set, &mut rows);
        drop(rows);
        if let Err(outcome) = stepped {
            return outcome;
        }
        let rows_affected = writes.then(|| {
            if conn.total_changes() > before {
                conn.changes()
            } else {
                0
            }
        });
        let end = ResultEnd {
            rows: set.rows,
            kept: set.delivered,
            rows_affected,
            command_tag: tag,
        };
        if !self.send(Msg::Event(QueryEvent::ResultEnd(end))) {
            return Outcome::Gone;
        }
        Outcome::Ended
    }

    fn finish_empty(&mut self, schema: Arc<ResultSchema>, end: ResultEnd) -> Outcome {
        if self.send(Msg::Event(QueryEvent::ResultStart(schema)))
            && self.send(Msg::Event(QueryEvent::ResultEnd(end)))
        {
            Outcome::Ended
        } else {
            Outcome::Gone
        }
    }

    /// Sends what `set` made ready: the schema once known, then sealed chunks.
    fn flush(&mut self, set: &mut ResultSet, force: bool) -> Result<(), Outcome> {
        if let Some(schema) = set.take_unsent_schema()
            && !self.send(Msg::Event(QueryEvent::ResultStart(schema)))
        {
            return Err(Outcome::Gone);
        }
        if let Some(chunk) = set.take_chunk(force)
            && !self.send(Msg::Event(QueryEvent::Rows(chunk)))
        {
            return Err(Outcome::Gone);
        }
        Ok(())
    }

    /// Steps the rows of `set` to the end of the result, suspending at each read-ahead point.
    /// `Err` is how the job ends early.
    fn stream(&mut self, set: &mut ResultSet, rows: &mut Rows<'_>) -> Result<(), Outcome> {
        if set.decided() {
            self.flush(set, false)?;
        }
        loop {
            if set.batch_left == Some(0) {
                // The read-ahead point: the batch's rows go out, then one more row decides.
                set.decide_all();
                self.flush(set, true)?;
                match self.step(set, rows) {
                    Step::End => break,
                    Step::Stop(outcome) => return Err(outcome),
                    Step::Row => {}
                }
                if !self.send(Msg::Event(QueryEvent::Suspended)) {
                    return Err(Outcome::Gone);
                }
                match self.cmds.blocking_recv() {
                    None => return Err(Outcome::Gone),
                    Some(Cmd::Close(how)) => return Err(self.close(how, set, rows)),
                    Some(Cmd::FetchMore(n)) => {
                        if self.cancel.cancelled(self.ticket) {
                            return Err(Outcome::Failed(interrupted()));
                        }
                        let n = if n == 0 { set.limits.chunk_rows } else { n };
                        // The held row is this batch's first.
                        set.batch_left = Some(u64::from(n) - 1);
                    }
                }
            } else {
                match self.cmds.try_recv() {
                    Ok(Cmd::Close(how)) => return Err(self.close(how, set, rows)),
                    // Only a suspended query is resumed; the async side never sends this here.
                    Ok(Cmd::FetchMore(_)) | Err(mpsc::error::TryRecvError::Empty) => {}
                    Err(mpsc::error::TryRecvError::Disconnected) => return Err(Outcome::Gone),
                }
                match self.step(set, rows) {
                    Step::End => break,
                    Step::Stop(outcome) => return Err(outcome),
                    Step::Row => {}
                }
            }
            if set.sealable() {
                set.decide_all();
            }
            self.flush(set, false)?;
        }
        set.decide_all();
        self.flush(set, true)
    }

    /// Steps one row into `set`.
    fn step(&mut self, set: &mut ResultSet, rows: &mut Rows<'_>) -> Step {
        match rows.next() {
            Ok(Some(row)) => {
                set.push(row);
                self.pace.rows.store(set.rows, Ordering::Relaxed);
                Step::Row
            }
            Ok(None) => Step::End,
            Err(e) => Step::Stop(Outcome::Failed(raw_error(e))),
        }
    }

    /// Ends the query by `how` (module docs); the caller finalizes the statement.
    fn close(&mut self, how: CloseHow, set: &mut ResultSet, rows: &mut Rows<'_>) -> Outcome {
        let CloseHow::Drain(d) = how else {
            return Outcome::Closed(None);
        };
        // A deadline past what `Instant` holds is no deadline (review round 1, finding 2).
        let at = Instant::now().checked_add(d);
        self.cancel.set_deadline(at);
        let mut total = set.rows;
        let completed = loop {
            if at.is_some_and(|at| Instant::now() >= at) {
                break false;
            }
            match rows.next() {
                Ok(Some(_)) => total += 1,
                Ok(None) => break true,
                // The deadline's interrupt, a cancel, or an error: closed all the same.
                Err(_) => break false,
            }
        };
        self.cancel.set_deadline(None);
        Outcome::Closed(completed.then_some(total))
    }
}

/// The state of one result while it streams.
struct ResultSet {
    limits: Limits,
    columns: Vec<Column>,
    rules: Vec<Rule>,
    /// Each column's kind, once known.
    kinds: Vec<Option<ColKind>>,
    natives: Vec<hacivat_values::NativeType>,
    schema: Option<Arc<ResultSchema>>,
    schema_sent: bool,
    /// Each column's kind and buffer, once every kind is known.
    settled: Vec<ColKind>,
    plan: Vec<BufKind>,
    builder: Option<ChunkBuilder>,
    /// Rows read while some kind was unknown, as whole values.
    held: Vec<Vec<Value>>,
    /// About the bytes of `held` (the payloads and a `Value` per cell).
    held_bytes: u64,
    /// Rows stepped in this result.
    rows: u64,
    /// Rows sent in chunks.
    delivered: u64,
    chunks: u64,
    /// Rows left in this batch (`None`: not suspendable).
    batch_left: Option<u64>,
}

impl ResultSet {
    fn new(columns: Vec<Column>, limits: Limits, suspendable: bool) -> ResultSet {
        let (natives, rules): (Vec<_>, Vec<_>) =
            columns.iter().map(Column::native_and_rule).unzip();
        let kinds = rules
            .iter()
            .map(|r| match r {
                Rule::Fixed(k) => Some(*k),
                Rule::FirstValue { .. } => None,
            })
            .collect();
        let mut set = ResultSet {
            limits,
            columns,
            rules,
            kinds,
            natives,
            schema: None,
            schema_sent: false,
            settled: Vec::new(),
            plan: Vec::new(),
            builder: None,
            held: Vec::new(),
            held_bytes: 0,
            rows: 0,
            delivered: 0,
            chunks: 0,
            batch_left: suspendable.then_some(u64::from(limits.first_rows)),
        };
        set.settle();
        set
    }

    fn decided(&self) -> bool {
        self.schema.is_some()
    }

    /// Builds the schema, the plan and the builder once every kind is known, and replays the
    /// rows held until then.
    fn settle(&mut self) {
        if self.schema.is_some() || self.kinds.iter().any(Option::is_none) {
            return;
        }
        let kinds: Vec<ColKind> = self.kinds.iter().flatten().copied().collect();
        let columns: Vec<ColumnMeta> = self
            .columns
            .iter()
            .zip(&self.natives)
            .zip(&kinds)
            .map(|((c, native), kind)| ColumnMeta {
                name: Arc::clone(&c.name),
                logical: kind.logical(),
                native: native.clone(),
                origin: c.origin.clone(),
            })
            .collect();
        self.plan = kinds.iter().map(|k| k.buf()).collect();
        self.schema = Some(Arc::new(ResultSchema {
            columns: columns.into(),
        }));
        self.settled = kinds;
        for row in std::mem::take(&mut self.held) {
            let (b, kinds) = self.builder();
            for (col, (v, kind)) in row.into_iter().zip(kinds).enumerate() {
                decode::push_owned(b, col as u32, *kind, v);
            }
            b.end_row();
        }
        self.held_bytes = 0;
    }

    /// Settles every kind still unknown with its fallback (the first chunk is complete).
    fn decide_all(&mut self) {
        for (kind, rule) in self.kinds.iter_mut().zip(&self.rules) {
            if kind.is_none() {
                *kind = Some(rule.kind(None));
            }
        }
        self.settle();
    }

    /// The current chunk's builder and the columns' kinds; only once every kind is known
    /// (`settle`), which every caller checks first.
    fn builder(&mut self) -> (&mut ChunkBuilder, &[ColKind]) {
        let limit = self.row_limit();
        let ResultSet {
            builder,
            plan,
            schema,
            settled,
            ..
        } = self;
        let schema = schema
            .as_ref()
            .expect("the builder is made once the schema is known");
        let b = builder.get_or_insert_with(|| ChunkBuilder::with_schema(plan, schema, limit));
        (b, settled)
    }

    /// The current chunk's row limit: `first_rows` for the first chunk, `chunk_rows` after.
    fn row_limit(&self) -> u32 {
        if self.chunks == 0 {
            self.limits.first_rows
        } else {
            self.limits.chunk_rows
        }
    }

    fn push(&mut self, row: &rusqlite::Row<'_>) {
        self.rows += 1;
        if let Some(left) = self.batch_left.as_mut() {
            *left = left.saturating_sub(1);
        }
        let cap = self.limits.cell_cap;
        if self.decided() {
            let (b, kinds) = self.builder();
            for (col, kind) in kinds.iter().enumerate() {
                // `col` is below the statement's column count, so this never fails.
                let v = row.get_ref(col).unwrap_or(ValueRef::Null);
                decode::push(b, col as u32, *kind, v, cap);
            }
            b.end_row();
            return;
        }
        let mut values = Vec::with_capacity(self.kinds.len());
        for (col, kind) in self.kinds.iter_mut().enumerate() {
            let v = row.get_ref(col).unwrap_or(ValueRef::Null);
            if kind.is_none() {
                *kind = ColKind::of_class(v.data_type());
            }
            let v = decode::value_of(v, cap);
            self.held_bytes += held_size(&v);
            values.push(v);
        }
        self.held.push(values);
        self.settle();
    }

    /// Whether the rows read so far fill a chunk (or the batch).
    fn sealable(&self) -> bool {
        let (rows, bytes, full) = match &self.builder {
            Some(b) => (b.rows(), b.bytes(), b.is_full()),
            None => (self.held.len() as u32, self.held_bytes, false),
        };
        rows > 0
            && (rows >= self.row_limit()
                || bytes >= self.limits.chunk_bytes
                || full
                || self.batch_left == Some(0))
    }

    /// The schema, the first time it is asked for after it became known.
    fn take_unsent_schema(&mut self) -> Option<Arc<ResultSchema>> {
        if self.schema_sent {
            return None;
        }
        let schema = self.schema.clone()?;
        self.schema_sent = true;
        Some(schema)
    }

    /// The current chunk, sealed, when it is due (or has any row, with `force`).
    fn take_chunk(&mut self, force: bool) -> Option<hacivat_values::Chunk> {
        if !self.decided() || !(force || self.sealable()) {
            return None;
        }
        let b = self.builder.take()?;
        if b.rows() == 0 {
            return None;
        }
        let chunk = b.seal();
        self.delivered += u64::from(chunk.rows());
        self.chunks += 1;
        Some(chunk)
    }
}

/// About the bytes a held value takes: a `Value`, plus its text or bytes.
fn held_size(v: &Value) -> u64 {
    let payload = match v {
        Value::Text(t) => t.len(),
        Value::Bytes(b) | Value::Lossy(b) => b.len(),
        Value::Truncated(t) => match &t.prefix {
            Value::Text(s) => s.len(),
            Value::Bytes(b) | Value::Lossy(b) => b.len(),
            _ => 0,
        },
        _ => 0,
    };
    (size_of::<Value>() + payload) as u64
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Events flow: `next_event` reads them.
    Streaming,
    /// At a read-ahead point: `fetch_more` or `close_query`.
    Suspended,
}

/// The async side of an open typed query.
pub(crate) struct Open {
    events: mpsc::Receiver<Msg>,
    cmds: mpsc::UnboundedSender<Cmd>,
    stage: Stage,
    /// Rows handed out by `next_event`.
    delivered: u64,
    /// The result ended (`ResultEnd` handed out) with this many rows.
    completed: Option<u64>,
    #[cfg(test)]
    pace: Arc<Pace>,
}

impl Open {
    /// `query`'s answer: the job's first message, `Started` when the query runs; else its
    /// error (with the transaction status and idle bound it left).
    pub(crate) async fn started(
        &mut self,
        tx: &mut TxStatus,
        idle_bound: &mut Option<Duration>,
    ) -> Result<(), RawError> {
        match self.events.recv().await {
            Some(Msg::Started) => Ok(()),
            Some(Msg::Failed(err, after)) => {
                after.apply(tx, idle_bound);
                Err(err)
            }
            // The job sends nothing else first; without a message it is gone.
            Some(_) | None => Err(gone()),
        }
    }

    /// How far the job has got (tests).
    #[cfg(test)]
    pub(crate) fn pace(&self) -> &Pace {
        &self.pace
    }
}

/// `RawConn::next_event`.
pub(crate) async fn next_event(
    open: &mut Option<Open>,
    tx: &mut TxStatus,
    idle_bound: &mut Option<Duration>,
) -> Result<QueryEvent, RawError> {
    let Some(q) = open.as_mut() else {
        return Err(unsupported());
    };
    if q.stage == Stage::Suspended {
        return Err(unsupported());
    }
    loop {
        return match q.events.recv().await {
            Some(Msg::Event(event)) => {
                match &event {
                    QueryEvent::Rows(chunk) => q.delivered += u64::from(chunk.rows()),
                    QueryEvent::Suspended => q.stage = Stage::Suspended,
                    QueryEvent::ResultEnd(end) => q.completed = Some(end.rows),
                    QueryEvent::ResultStart(_) | QueryEvent::Done(_) => {}
                }
                Ok(event)
            }
            // Left by a `query` whose future was dropped before it read it.
            Some(Msg::Started) => continue,
            Some(Msg::Done(done, after)) => {
                after.apply(tx, idle_bound);
                *open = None;
                Ok(QueryEvent::Done(done))
            }
            Some(Msg::Failed(err, after)) => {
                after.apply(tx, idle_bound);
                *open = None;
                Err(err)
            }
            // `Closed` answers only `close_query`; the job is gone.
            Some(Msg::Closed(..)) | None => {
                *open = None;
                Err(gone())
            }
        };
    }
}

/// `RawConn::fetch_more`.
pub(crate) fn fetch_more(open: &mut Option<Open>, rows: u32) -> Result<(), RawError> {
    let Some(q) = open.as_mut() else {
        return Err(unsupported());
    };
    if q.stage != Stage::Suspended {
        return Err(unsupported());
    }
    if q.cmds.send(Cmd::FetchMore(rows)).is_err() {
        *open = None;
        return Err(gone());
    }
    q.stage = Stage::Streaming;
    Ok(())
}

/// `RawConn::close_query`: the job ends by `how`; events still in flight are dropped. The query
/// stays the connection's open one until the job answered, so a connection dropped meanwhile
/// still stops it (`Drop for SqliteConn`).
pub(crate) async fn close_query(
    open: &mut Option<Open>,
    tx: &mut TxStatus,
    idle_bound: &mut Option<Duration>,
    how: CloseHow,
) -> Result<Closed, RawError> {
    let Some(q) = open.as_mut() else {
        return Ok(Closed::Closed { rows: 0 });
    };
    // The job may have ended already (its last message is queued); then nobody reads this.
    let _ = q.cmds.send(Cmd::Close(how));
    let mut completed = q.completed;
    loop {
        match q.events.recv().await {
            Some(Msg::Event(QueryEvent::ResultEnd(end))) => completed = Some(end.rows),
            Some(Msg::Event(_) | Msg::Started) => {}
            Some(Msg::Done(_, after)) => {
                after.apply(tx, idle_bound);
                break;
            }
            Some(Msg::Closed(drained, after)) => {
                after.apply(tx, idle_bound);
                completed = completed.or(drained);
                break;
            }
            // An error while closing (a cancel, the drain's interrupt): closed all the same.
            Some(Msg::Failed(_, after)) => {
                after.apply(tx, idle_bound);
                break;
            }
            None => {
                *open = None;
                return Err(gone());
            }
        }
    }
    let delivered = q.delivered;
    *open = None;
    Ok(match (how, completed) {
        (CloseHow::Drain(_), Some(rows)) => Closed::Completed { rows },
        _ => Closed::Closed { rows: delivered },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_never_ask_for_zero_rows() {
        let spec = |first_rows, chunk_rows| QuerySpec {
            sql: "",
            binds: &[],
            protocol: hacivat_driver::Protocol::Extended,
            suspendable: true,
            first_rows,
            chunk_rows,
            chunk_bytes: 1,
            cell_cap: 1,
            max_result_sets: 1,
        };
        let l = Limits::of(&spec(0, 0));
        assert_eq!((l.first_rows, l.chunk_rows), (1, 1));
        let l = Limits::of(&spec(0, 50));
        assert_eq!((l.first_rows, l.chunk_rows), (50, 50));
        let l = Limits::of(&spec(501, 4_096));
        assert_eq!((l.first_rows, l.chunk_rows), (501, 4_096));
    }

    #[test]
    fn only_statements_that_name_a_journal_or_database_change_re_read_the_bound() {
        for sql in [
            "PRAGMA journal_mode = WAL",
            "pragma main.journal_mode",
            "ATTACH 'x.db' AS x",
            "  detach x",
            "BEGIN; PRAGMA journal_mode = DELETE; COMMIT",
        ] {
            assert!(may_change_journal(sql), "{sql}");
        }
        for sql in [
            "SELECT 1",
            "INSERT INTO t VALUES (1)",
            "",
            "CREATE TABLE prag(a)",
        ] {
            assert!(!may_change_journal(sql), "{sql}");
        }
    }

    #[test]
    fn caps_suspend_without_binds_or_kills() {
        let c = caps(Some(ROLLBACK_IDLE_BOUND));
        assert!(c.suspend && !c.binds && !c.close_needs_kill);
        assert_eq!(c.idle_bound, Some(Duration::from_secs(30)));
        assert_eq!(caps(None).idle_bound, None);
    }
}
