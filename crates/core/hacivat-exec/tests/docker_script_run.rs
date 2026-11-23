//! M4b Task 19 (gate E8's Docker row; carries-w1 T19, T12 review I6): "Run as script" on real
//! servers, through the runner a script tab uses (`ScriptRunner`, R-M4b-47 / R-T12-1's two
//! passes), a real file read through one handle (`common::t12::FileScript`), and a console
//! session of a real `ConnectionManager` (M2b's configuration):
//!
//! - **The 150 MB dump** (`mysql_dump_of_150_mb_runs_in_constant_memory_on_a_real_server`):
//!   a generated MySQL dump of extended INSERTs runs to its end on mysql84; the server holds
//!   every row; the process's resident set is sampled through both passes and its growth stays
//!   under the INSERT-dump budget (64 MiB, carries-w1 T12 / T8 m-fr2-4). The figures go out as
//!   `HV-FACT t19 e8 …` lines.
//! - **R-M4b-47 refusals on a real server**: a stray `END` (`R1-my-stray-end`) on every MySQL
//!   and MariaDB line of the active set, and a statement the gate denies late in the file on
//!   every server line: the file is refused whole, and the side channel sees that nothing ran
//!   (no row inserted, no row deleted, no routine created).
//! - **The premise, without Docker** (`the_late_statement_is_denied_on_production_by_a_fake`):
//!   the late statement the Docker test relies on is a gate denial in pass 1 on a production
//!   session of each engine, so the real-server test tests what it says.
//!
//! Run with `<main>/target/docker/run-docker-tests.sh --set pr|full -p hacivat-exec --test
//! docker_script_run` (CI's `docker-pr`/`docker-full`). Fixture tables are `hvm4b_t19_*`, made
//! unique to the process and dropped by a guard, also when a test fails.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use common::matrix::{Console, Line, Opts, Side, docker_enabled, line_of, must_exist, unique};
use common::t12::{Answers, FileScript, dialect_of, rig, script};
use hacivat_driver::fake::FakeDriver;
use hacivat_exec::{
    ClassifyPool, ExecService, ScriptChunks, ScriptGate, ScriptPhase, ScriptProgress,
    ScriptRefuseReason, ScriptRunOutcome, ScriptRunRequest, ScriptRunner, SuppressSet, run_file,
};
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_results::MemoryBudget;
use hacivat_session::session::{Request, TxCommand, TxMode};
use hacivat_testkit::require_target;
use tokio::sync::watch;

// ---------------------------------------------------------------------------------------------
// Support

/// The budget of an INSERT dump's resident-set growth (carries-w1 T12: the plan's "RSS growth
/// < 64 MiB" covers INSERT dumps; T8 m-fr2-4).
const DUMP_RSS_BUDGET_MIB: u64 = 64;
/// The dump's size: 150 MiB of text.
const DUMP_BYTES: usize = 150 << 20;
/// Rows per extended INSERT (about 15 KiB per statement, the shape of a `mysqldump` with a
/// small `--net-buffer-length`).
const ROWS_PER_INSERT: usize = 160;
/// Statements before the late one: enough text for four of the runner's 64 KiB chunks.
const EARLY_INSERTS: usize = 8_000;

/// A fixture table on `line`, dropped when the guard goes (also on a failed assertion): the
/// drop runs on a runtime of its own on another thread, so it works while unwinding.
struct Table {
    line: Line,
    name: String,
    routine: Option<String>,
}

impl Table {
    async fn create(line: &Line, slug: &str, cols: &str) -> (Table, Side) {
        let name = unique(&format!("hvm4b_t19_{slug}"));
        let mut a = Side::open(line, None).await;
        let ddl = if line.is_mysql_family() {
            format!("CREATE TABLE {name} ({cols}) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4")
        } else {
            format!("CREATE TABLE {name} ({cols})")
        };
        a.batch(&ddl).await;
        (
            Table {
                line: line.clone(),
                name,
                routine: None,
            },
            a,
        )
    }
}

impl Drop for Table {
    fn drop(&mut self) {
        let line = self.line.clone();
        let mut drops = vec![format!("DROP TABLE IF EXISTS {}", self.name)];
        if let Some(p) = &self.routine {
            drops.push(format!("DROP PROCEDURE IF EXISTS {p}"));
        }
        let joined = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a runtime for the drop");
            rt.block_on(async {
                let mut a = Side::open(&line, None).await;
                for sql in drops {
                    let _ = a.try_batch(&sql).await;
                }
                a.close().await;
            });
        })
        .join();
        if joined.is_err() {
            eprintln!(
                "t19: dropping {} failed; the next run's names differ",
                self.name
            );
        }
    }
}

/// A console on `line` (M2b's configuration: production writable) in autocommit, so a unit
/// sent by mistake commits at once and the side channel sees it.
async fn console(line: &Line, env: EnvTag) -> Console {
    let c = Console::open(
        line,
        Opts {
            env: Some(env),
            ..Opts::default()
        },
    )
    .await;
    let r =
        c.s.submit(Request::Tx(TxCommand::SetMode(TxMode::Auto)))
            .reply
            .await
            .expect("the actor replies");
    assert!(r.is_ok(), "{}: SetMode(Auto): {r:?}", line.id);
    c
}

/// A runner as the app builds one: the run service and a classifier pool that keeps nothing.
fn runner() -> ScriptRunner {
    ScriptRunner::new(
        Arc::new(ExecService::new(MemoryBudget::new(1 << 30, 256 << 20))),
        ClassifyPool::new(2),
    )
}

/// The run of `source` on `c`'s session as a script tab under `env` sends it: the tab's
/// context is the one the console read at bind (known modes), every question answered by
/// `port`.
struct Run {
    env: watch::Sender<Option<EnvTag>>,
    cancel: watch::Sender<bool>,
}

impl Run {
    fn new() -> Run {
        Run {
            env: watch::channel(None).0,
            cancel: watch::channel(false).0,
        }
    }

    fn request(
        &self,
        c: &Console,
        source: Box<dyn ScriptChunks>,
        env: EnvTag,
        port: Arc<Answers>,
    ) -> ScriptRunRequest {
        let (_, flavor) = dialect_of(c.line.engine);
        self.env.send_replace(Some(env));
        ScriptRunRequest {
            session: c.s.clone(),
            source,
            console: 19,
            profile: c.profile.id,
            engine: c.line.engine,
            dialect: c.ctx,
            modes_known: true,
            flavor,
            gate: ScriptGate {
                env: self.env.subscribe(),
                confirm_unsafe_where: true,
                allowlist: Arc::from(Vec::new()),
                suppressed: SuppressSet::default(),
                connection: Arc::from(c.profile.name.as_str()),
            },
            confirm: port,
            log: c.log.clone(),
            cancel: self.cancel.subscribe(),
            history: None,
            refresh: None,
        }
    }
}

/// A script file in a temporary folder of its own, removed with the guard.
struct ScriptFile {
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
}

/// Writes `text` (made by `write`) to a fresh file. A large file goes under the Docker
/// artifacts folder (`hacivat_testkit::artifact`), never `/tmp` (user rule: nothing over
/// 100 MB there).
fn script_file(large: bool, write: impl FnOnce(&mut dyn std::io::Write)) -> ScriptFile {
    let dir = if large {
        let base = hacivat_testkit::artifact("hvm4b_t19_dump.sql");
        let parent = base.parent().expect("the artifacts folder").to_path_buf();
        std::fs::create_dir_all(&parent).expect("the artifacts folder");
        tempfile::Builder::new()
            .prefix("hvm4b_t19_")
            .tempdir_in(parent)
            .expect("a folder for the dump")
    } else {
        tempfile::tempdir().expect("a temporary folder")
    };
    let path = dir.path().join("script.sql");
    let mut f = std::io::BufWriter::new(std::fs::File::create(&path).expect("the script file"));
    write(&mut f);
    std::io::Write::flush(&mut f).expect("the script is written");
    drop(f);
    ScriptFile { _dir: dir, path }
}

/// This process's resident set, in KiB (`ps`, as Task 12's perf test reads it).
fn rss_kib() -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

/// Samples [`rss_kib`] every 100 ms on a thread of its own until stopped; the peak.
struct Sampler {
    stop: Arc<AtomicBool>,
    peak: Arc<AtomicU64>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Sampler {
    fn start() -> Sampler {
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU64::new(rss_kib()));
        let (s, p) = (stop.clone(), peak.clone());
        let thread = std::thread::spawn(move || {
            while !s.load(Ordering::SeqCst) {
                p.fetch_max(rss_kib(), Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        Sampler {
            stop,
            peak,
            thread: Some(thread),
        }
    }

    fn finish(mut self) -> u64 {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        self.peak.load(Ordering::SeqCst).max(rss_kib())
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The refusal reason of `out` and the statement it names, or a panic naming the outcome.
fn refusal(id: &str, out: &ScriptRunOutcome) -> (ScriptRefuseReason, Option<u64>) {
    match out {
        ScriptRunOutcome::Refused(r) => (r.reason.clone(), r.at.map(|p| p.unit)),
        other => panic!("{id}: expected the file to be refused, got {other:?}"),
    }
}

/// The stray-`END` file of `R1-my-stray-end` (`sql-testkit/corpus/split/mysql.toml`), after a
/// write of its own into `t`: the eager stream gives out the INSERT and `DELETE FROM t` before
/// its stop; `p` is the procedure it would create.
fn stray_end_text(t: &str, p: &str) -> String {
    format!(
        "INSERT INTO {t} VALUES (3);\nCREATE PROCEDURE {p}() BEGIN SELECT 1; END; DELETE FROM {t}; END;\nSELECT 2;\n"
    )
}

/// The server lines of the active set of `engines`: the matrix's lines (from the targets'
/// properties, never a list; one per supported major, no twins, nothing below an engine's
/// floor; Task 31's `matrix::server_lines`).
fn server_lines(engines: &[DbmsKind]) -> Vec<Line> {
    common::matrix::server_lines()
        .into_iter()
        .filter(|l| engines.contains(&l.engine))
        .collect()
}

/// The early statements of the late-denial file: `n` single-row INSERTs into `table` (the
/// rows a mistaken send would leave behind).
fn early_inserts(table: &str, n: usize) -> String {
    (0..n)
        .map(|i| format!("INSERT INTO {table} (a) VALUES ({i});\n"))
        .collect()
}

/// The statement the gate denies on a production session of every engine here, whatever the
/// statements before it: it switches the role the connection acts as, which the policy denies
/// in every profile (`sql-safety` policy: "a lexical-mode change and a role switch deny in
/// every profile"). Valid on PostgreSQL, MySQL 8 and MariaDB, so a mistaken send would also run.
/// The premise is checked without Docker below.
const LATE_DENIED: &str = "SET ROLE NONE;\n";

// ---------------------------------------------------------------------------------------------
// The premise, without Docker

#[tokio::test]
async fn the_late_statement_is_denied_on_production_by_a_fake() {
    for engine in [DbmsKind::Postgres, DbmsKind::MySql, DbmsKind::MariaDb] {
        let fake = match engine {
            DbmsKind::Postgres => common::fake_pg(),
            other => FakeDriver::new(other),
        };
        let r = rig(fake, false).await;
        let text = format!(
            "{}{}",
            early_inserts("hvm4b_t19_deny", EARLY_INSERTS),
            LATE_DENIED
        );
        assert!(text.len() > 3 * hacivat_exec::SCRIPT_CHUNK_BYTES);
        let (out, progress) = r
            .run(r.request(
                script(&text).boxed(),
                Some(EnvTag::Production),
                Answers::new(&[]),
            ))
            .await;
        let (reason, unit) = refusal(&format!("{engine:?}"), &out);
        assert!(
            matches!(reason, ScriptRefuseReason::Denied(_)),
            "{engine:?}: {reason:?}"
        );
        // The denial is the late statement's, not an early INSERT's.
        assert_eq!(unit, Some(EARLY_INSERTS as u64 + 1), "{engine:?}");
        assert_eq!(progress.units_run, 0, "{engine:?}");
        assert!(
            r.all_sent().iter().all(|q| !q.contains("hvm4b_t19_deny")),
            "{engine:?}: an INSERT was sent"
        );
    }
}

#[tokio::test]
async fn the_stray_end_file_is_refused_by_a_fake() {
    for engine in [DbmsKind::MySql, DbmsKind::MariaDb] {
        let r = rig(FakeDriver::new(engine), false).await;
        let text = stray_end_text("hvm4b_t19_stray", "hvm4b_t19_stray_p");
        let (out, progress) = r
            .run(r.request(script(&text).boxed(), Some(EnvTag::Dev), Answers::new(&[])))
            .await;
        let (reason, _) = refusal(&format!("{engine:?}"), &out);
        assert!(
            matches!(reason, ScriptRefuseReason::Uncertain(_)),
            "{engine:?}: {reason:?}"
        );
        assert_eq!(progress.units_run, 0, "{engine:?}");
        assert!(
            r.all_sent().iter().all(|q| !q.contains("hvm4b_t19_stray")),
            "{engine:?}: a statement of the file was sent"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// The 150 MB dump

/// One extended INSERT of `ROWS_PER_INSERT` rows from `first`.
fn dump_insert(table: &str, first: usize) -> String {
    let mut s = format!("INSERT INTO `{table}` VALUES ");
    for i in first..first + ROWS_PER_INSERT {
        if i > first {
            s.push(',');
        }
        s.push_str(&format!("({i},'{:0>80}')", i));
    }
    s.push_str(";\n");
    s
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn mysql_dump_of_150_mb_runs_in_constant_memory_on_a_real_server() {
    if !docker_enabled() {
        return;
    }
    let Some(target) = require_target("mysql84") else {
        return;
    };
    let line = line_of(target);
    let (table, mut side) =
        Table::create(&line, "dump", "id INT PRIMARY KEY, v VARCHAR(100)").await;
    let mut units = 0usize;
    let mut bytes = 0usize;
    let file = script_file(true, |w| {
        let header = format!(
            "-- A generated MySQL dump (M4b Task 19)\n--\n-- Table structure for `{}`: made by the test\n\n",
            table.name
        );
        w.write_all(header.as_bytes()).expect("the dump is written");
        bytes += header.len();
        while bytes < DUMP_BYTES {
            let stmt = dump_insert(&table.name, units * ROWS_PER_INSERT);
            w.write_all(stmt.as_bytes()).expect("the dump is written");
            bytes += stmt.len();
            units += 1;
        }
    });
    let c = console(&line, EnvTag::Dev).await;
    let run = Run::new();
    let port = Answers::new(&[]);
    let req = run.request(
        &c,
        Box::new(FileScript::open(file.path.clone())),
        EnvTag::Dev,
        port.clone(),
    );
    let before = rss_kib();
    let sampler = Sampler::start();
    let (tx, mut rx) = watch::channel(ScriptProgress::default());
    let started = Instant::now();
    let pass1 = async {
        let _ = rx.wait_for(|p| p.phase != ScriptPhase::Checking).await;
        rss_kib()
    };
    let runner = runner();
    let (out, after_pass1) = tokio::join!(run_file(&runner, req, tx), pass1);
    let secs = started.elapsed().as_secs();
    let peak = sampler.finish();
    let rows = side
        .count(&format!("SELECT COUNT(*) FROM {}", table.name))
        .await;
    let summary = match &out {
        ScriptRunOutcome::Finished(s) => s.clone(),
        other => panic!("mysql84: the dump did not finish: {other:?}"),
    };
    let growth_mib = peak.saturating_sub(before) / 1024;
    let pass1_mib = after_pass1.saturating_sub(before) / 1024;
    println!(
        "HV-FACT t19 e8 target=mysql84 bytes={bytes} units={} units_run={} rows={rows} \
         rss_before_mib={} rss_peak_mib={} rss_growth_mib={growth_mib} pass1_growth_mib={pass1_mib} \
         peak_held={} secs={secs}",
        units,
        summary.units_run,
        before / 1024,
        peak / 1024,
        summary.peak_held,
    );
    assert!(
        port.asked_units().is_empty(),
        "a dump on dev asked {:?}",
        port.asked_units()
    );
    assert_eq!(summary.units_run, units as u64);
    assert_eq!(rows, i64::try_from(units * ROWS_PER_INSERT).unwrap());
    assert!(summary.peak_held <= 256 << 10, "held {}", summary.peak_held);
    assert!(
        growth_mib < DUMP_RSS_BUDGET_MIB,
        "the resident set grew {growth_mib} MiB over the dump (budget {DUMP_RSS_BUDGET_MIB})"
    );
    side.close().await;
}

// ---------------------------------------------------------------------------------------------
// R-M4b-47 refusals on a real server

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn stray_end_file_executes_nothing_on_a_real_server() {
    if !docker_enabled() {
        return;
    }
    let lines = server_lines(&[DbmsKind::MySql, DbmsKind::MariaDb]);
    assert!(
        !lines.is_empty(),
        "the active set has no MySQL or MariaDB line"
    );
    if must_exist() {
        assert!(
            lines.iter().any(|l| l.engine == DbmsKind::MariaDb),
            "the full set has no MariaDB line"
        );
    }
    for line in &lines {
        let id = &line.id;
        let (mut table, mut side) = Table::create(line, "stray", "a INT").await;
        let t = table.name.clone();
        let p = format!("{t}_p");
        table.routine = Some(p.clone());
        side.batch(&format!("INSERT INTO {t} VALUES (1), (2)"))
            .await;
        let text = stray_end_text(&t, &p);
        let file = script_file(false, |w| w.write_all(text.as_bytes()).expect("written"));
        let c = console(line, EnvTag::Dev).await;
        let run = Run::new();
        let (tx, rx) = watch::channel(ScriptProgress::default());
        let req = run.request(
            &c,
            Box::new(FileScript::open(file.path.clone())),
            EnvTag::Dev,
            Answers::new(&[]),
        );
        let out = run_file(&runner(), req, tx).await;
        let (reason, _) = refusal(id, &out);
        let rows = side.count(&format!("SELECT COUNT(*) FROM {t}")).await;
        let routines = side
            .count(&format!(
                "SELECT COUNT(*) FROM information_schema.routines \
                 WHERE routine_schema = DATABASE() AND routine_name = '{p}'"
            ))
            .await;
        println!(
            "HV-FACT t19 e8 stray_end target={id} refused={reason:?} units_run={} rows={rows} routines={routines}",
            rx.borrow().units_run
        );
        assert!(
            matches!(reason, ScriptRefuseReason::Uncertain(_)),
            "{id}: {reason:?}"
        );
        assert_eq!(rx.borrow().units_run, 0, "{id}");
        assert_eq!(rows, 2, "{id}: the table changed");
        assert_eq!(routines, 0, "{id}: the routine was created");
        side.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn a_denied_statement_late_in_a_file_executes_nothing_on_a_real_server() {
    if !docker_enabled() {
        return;
    }
    let lines = server_lines(&[DbmsKind::Postgres, DbmsKind::MySql, DbmsKind::MariaDb]);
    assert!(
        lines.iter().any(Line::is_pg) && lines.iter().any(Line::is_mysql_family),
        "the active set needs a PostgreSQL and a MySQL-family line"
    );
    for line in &lines {
        let id = &line.id;
        let (table, mut side) = Table::create(line, "deny", "a INT").await;
        let t = table.name.clone();
        let text = format!("{}{}", early_inserts(&t, EARLY_INSERTS), LATE_DENIED);
        let file = script_file(false, |w| w.write_all(text.as_bytes()).expect("written"));
        // Production, writable (M2b's configuration) and in autocommit: every INSERT the runner
        // sent would commit at once.
        let c = console(line, EnvTag::Production).await;
        let run = Run::new();
        let port = Answers::new(&[]);
        let (tx, rx) = watch::channel(ScriptProgress::default());
        let req = run.request(
            &c,
            Box::new(FileScript::open(file.path.clone())),
            EnvTag::Production,
            port.clone(),
        );
        let out = run_file(&runner(), req, tx).await;
        let (reason, unit) = refusal(id, &out);
        let rows = side.count(&format!("SELECT COUNT(*) FROM {t}")).await;
        println!(
            "HV-FACT t19 e8 late_denial target={id} refused={reason:?} at_unit={unit:?} units_run={} rows={rows} asked={}",
            rx.borrow().units_run,
            port.asked_units().len()
        );
        assert!(
            matches!(reason, ScriptRefuseReason::Denied(_)),
            "{id}: {reason:?}"
        );
        assert_eq!(
            unit,
            Some(EARLY_INSERTS as u64 + 1),
            "{id}: not the late statement"
        );
        assert_eq!(rx.borrow().units_run, 0, "{id}");
        assert!(port.asked_units().is_empty(), "{id}: a refused file asked");
        assert_eq!(rows, 0, "{id}: an early INSERT reached the server");
        side.close().await;
    }
}
