// ## m3b-t01c-core-seams
//! The exec gate (HV-EXEC-04, R-M3-55, R-M3-84; Task 15): what Run does with each prepared
//! unit given its classification and the connection: run it, warn, ask for a confirmation
//! (type-to-confirm for the destructive kinds), run it in a transaction and then ask
//! (confirm-in-transaction; the session picks the form, P-M3b-1), wrap an `EXPLAIN ANALYZE`
//! (D-062), or deny it. Pure and synchronous: it never talks to a session.
//!
//! Since Task 15 ([`gate`] lists the rules). The app calls the gate only from Task 24 on
//! (R-M3-52). ## m3b-t26-prod-unlock: production sessions are read-write since Task 26, so the
//! gate's production rows (rule 4 on) are what stands between a production console and its
//! data; a read-only connection is still gated as read-only whatever its environment.

use std::sync::Arc;
use std::time::Instant;

use hacivat_driver::TxStatus;
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_session::session::{
    CommitAnswer, RunId, SessionEffects, TxMode, TxOwner, UnitId, UnitVerdict,
};
// ## m3b-t15-gate
use sql_dialect::{DialectCtx, DialectId, lex_profile};
use sql_ident::{PG_MAX_IDENT_BYTES, QualifiedName, ident_eq, parse_qualified};
use sql_lexer::{LexState, TokenKind, lex};
use sql_safety::effects::class_of;
use sql_safety::{
    Actor, CoarseClass, Confidence, DestructiveKind, EffectSet, GateClass, ObjectKind, ObjectRef,
    Profile, Reason, ReasonCode, RiskFlags, SafetyReport, StmtKind, StmtReport, TouchedObjects,
    Verdict, decide, gate_class, opens_write,
};
use sql_text::TextRange;
use tokio::sync::oneshot;

use crate::explain_wrap::{explain_analyze_inner, wrap_allowed};
use crate::implicit_commit::{
    atoms, commits, engine_commits_implicitly, mentions_write, raw_words, temporary_table,
};
use crate::params::PreparedUnit;
use crate::sqlctx::dialect_id;
use crate::suppress::SuppressSet;

/// What the gate knows of the connection a run goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnSafety {
    pub engine: DbmsKind,
    pub env: Option<EnvTag>,
    pub read_only: bool,
    pub tx_status: TxStatus,
    pub tx_owner: TxOwner,
    pub tx_mode: TxMode,
    /// `exec.confirm_unsafe_where` (off production).
    pub confirm_unsafe_where: bool,
}

/// Why the gate decided as it did: a Fluent ID and its arguments (server text never).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GateReason {
    pub fluent_id: &'static str,
    pub args: Vec<(&'static str, Arc<str>)>,
}

impl GateReason {
    /// A reason without arguments.
    pub fn of(fluent_id: &'static str) -> GateReason {
        GateReason {
            fluent_id,
            args: Vec::new(),
        }
    }
}

/// What Run does with one unit.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GateDecision {
    Run,
    /// A dialog with Cancel focused (`Lock` on production).
    Warn(Vec<GateReason>),
    Confirm {
        reasons: Vec<GateReason>,
        /// The exact bytes the user types (no case folding), when asked.
        type_to_confirm: Option<Arc<str>>,
        /// MySQL/MariaDB DDL: "cannot be rolled back".
        irreversible: bool,
        /// "Execute and don't ask again" is offered (never with type-to-confirm).
        suppressible: bool,
    },
    /// Run in a transaction, then ask Commit/Roll back (the session picks the form; P-M3b-1).
    RunInTxThenConfirm {
        reasons: Vec<GateReason>,
    },
    /// D-062.
    ExplainAnalyzeRollback,
    Deny(GateReason),
}

/// The gate's answer for one unit.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GateItem {
    /// The unit's index in the run.
    pub unit: u32,
    pub decision: GateDecision,
    /// What the session is told (`RunUnit::verdict`).
    pub verdict: UnitVerdict,
    pub effects: SessionEffects,
    pub class: GateClass,
    /// The "don't ask again" key (`SuppressSet` keys on it): since Task 15 fix round 1 (M4) the
    /// classified text's fingerprint mixed with `class`, so the same shape at a higher class
    /// is asked again. The answer `ExecuteAndSuppress` records this value.
    pub fingerprint: u64,
}

/// The gate's answer for a run.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct GatePlan {
    pub items: Vec<GateItem>,
}

impl GatePlan {
    /// Whether any unit needs a dialog before the run is sent (`Confirm` or `Warn`; R-M3-84).
    pub fn needs_prompt(&self) -> bool {
        self.items.iter().any(|i| {
            matches!(
                i.decision,
                GateDecision::Confirm { .. } | GateDecision::Warn(_)
            )
        })
    }

    /// The first denied unit, if any (a run with one sends nothing).
    pub fn first_denied(&self) -> Option<&GateItem> {
        self.items
            .iter()
            .find(|i| matches!(i.decision, GateDecision::Deny(_)))
    }
}

// ## m3b-t15-gate

/// The Fluent IDs the gate names (`hacivat-core.ftl`, block `m3b-t15-gate`).
const STALE: &str = "exec-gate-stale-classification";
const NOT_CLASSIFIED: &str = "exec-gate-not-classified";
const READ_ONLY: &str = "exec-gate-read-only";
const POLICY_DENIED: &str = "exec-gate-policy-denied";
const IMPLICIT_COMMIT: &str = "exec-gate-implicit-commit";
const EXPLAIN_REFUSED: &str = "exec-explain-analyze-refused";
const NO_TARGET: &str = "exec-unsafe-no-target";
const SEVERAL_TARGETS: &str = "exec-unsafe-several-targets";
// ## m3c-p2-07 (PO P2-07): `DROP OWNED BY` on production.
const DROP_OWNED_NO_TARGET: &str = "exec-drop-owned-no-target";
const DROP_OWNED_SEVERAL_TARGETS: &str = "exec-drop-owned-several-targets";
const DROP_OWNED: &str = "safety-destructive-drop-owned";
const DROP_OWNED_CASCADE: &str = "safety-destructive-drop-owned-cascade";
// ## m3c-p2-07
// ## m3b-t15-gate (fix round 1)
/// I1: a statement Hacivat could not analyse that may change data (off production).
const NOT_ANALYSED: &str = "exec-gate-not-analysed";
/// I2: what the D-062 rollback does not undo.
const EXPLAIN_NOT_UNDONE: &str = "exec-explain-analyze-not-undone";
/// A destructive pattern whose target the classifier did not name (Task 4's id).
const DESTRUCTIVE: &str = "safety-reason-destructive";

impl GatePlan {
    /// Whether this plan has exactly one item per unit of a run of `units` units, in order.
    /// An empty plan covers nothing: it never means "allow all" (a caller that sends a unit
    /// this plan has no item for sends a unit nobody gated).
    pub fn covers(&self, units: usize) -> bool {
        units > 0
            && self.items.len() == units
            && self
                .items
                .iter()
                .enumerate()
                .all(|(i, item)| usize::try_from(item.unit).is_ok_and(|u| u == i))
    }
}

/// The gate (the module docs): one item per unit, `reports[i]` classifying
/// `units[i].final_text`; `suppressed` holds the console's "don't ask again" pairs.
///
/// Per unit, in this order (R-M3-55; the first rule that decides wins):
/// 1. **Stale** (P-M3b-2): no report, a report count that differs from the unit count, or a
///    report of other bytes than `final_text` → `Deny(exec-gate-stale-classification)`. The
///    executor sends exactly the bytes classified.
/// 2. **Not classified**: a report pre-decided as `NotImplemented` (the classifier stub) →
///    `Deny(exec-gate-not-classified)` on every connection (F31).
///    **Not analysed** (fix round 1, I1): any other pre-decided report, or one with no
///    statement, is denied on production and read-only connections (the classifier's first
///    reason, or `exec-gate-read-only`); off production it asks one calm, never suppressible
///    `Confirm` (`exec-gate-not-analysed`) when its words hold a write or DDL keyword (or it
///    commits an open MySQL transaction), and runs otherwise (a dump's `/*!40101 SET NAMES */`).
/// 3. **Read-only** connections: `Run` only when the unit's class is `Read` (or `Session` on
///    the harmless-`SET` list, P-M3b-6) **and** `decide(.., ReadOnlyPlusExplain, Human)` allows
///    it (R-M3-56); else `Deny(exec-gate-read-only)`, naming the class. `CALL`, `Unknown` and
///    `Admin` are denied.
/// 4. **Production, the AND-check**: `decide(report, Admin, Human)` must not deny (its first
///    reason is the denial), and a `Confirm` from it on a class whose own `admin` row allows
///    raises the class to `Unknown` (evidence the gate class did not see: request-level flags
///    and effects reach the gate only this way and through the request floor).
/// 5. **D-062**: a lone `EXPLAIN ANALYZE` of a non-read runs wrapped in a rollback
///    (`ExplainAnalyzeRollback`), refused for `Routine`/`Admin` inner classes and, on
///    MySQL/MariaDB/TiDB, for DDL inners (their implicit commit escapes the rollback); one
///    inside a unit of several statements is refused. On production every wrapped inner asks
///    first (## m3b-t26-prod-unlock fix round 1, B-M2; before: an `Unknown` inner and every
///    MySQL-family one): `Confirm` with verdict `ExplainAnalyzeRollback`, naming what a
///    rollback does not undo (`exec-explain-analyze-not-undone`; fix round 1, I2, M7).
///    **B-M6** (## m3b-gate-explain: after 4 and 5): a plain `EXPLAIN` of a code-free data
///    change runs as a read (`plain_explain_of_dml`).
/// 6. **The table**: production `Read`/`Session` → `Run`, `Lock` → `Warn`, `Dml` →
///    `RunInTxThenConfirm` (the session picks the form, P-M3b-1), every other class →
///    `Confirm`; off production `Run`, except a unit holding an unsafe-DML pattern, which asks
///    the calm `Confirm` while `confirm_unsafe_where` is on.
///    ## m3b-t26-prod-unlock (fix round 1, ruling on A-I2/B-I1, D-043): on production
///    MySQL/MariaDB/TiDB a `Dml` is a `Confirm` before it is sent (a MyISAM/MEMORY/Aria table
///    keeps a write no rollback undoes); PostgreSQL and SQLite keep the savepoint form. A
///    production `Session` statement that changes how later writes behave
///    (`session_replication_role`, `sql_log_bin`, `foreign_key_checks`, `unique_checks`,
///    `sql_safe_updates`, `gtid_next`, `pseudo_replica_mode`/`pseudo_slave_mode`,
///    `synchronous_commit`, quoted or not) or a `NOTIFY` asks too (B-M1; fix round 2, R3).
/// 7. **Stricter on top**: type-to-confirm on production (from the destructive kinds, not the
///    class: a DDL in the same unit hides `DmlUnsafe`; `DROP DATABASE/SCHEMA` also read from
///    the statement's words, I3), the MySQL/MariaDB/TiDB implicit commit of
///    [`crate::implicit_commit`]'s list while a transaction is open (`Confirm`, verdict
///    `ImplicitCommit`, never suppressible; I5), `irreversible` for their DDL and privilege
///    statements.
/// 8. **Suppression**: a suppressible `Confirm`/`Warn` whose `(console, key)` the user turned
///    off runs (`Run`); the key mixes the fingerprint with the class (M4). Never suppressible:
///    type-to-confirm, implicit commits, a unit of several statements with a write (M9), and
///    **every production item** (T26 fix round 2, N1/R1: production never consults the set, so
///    no "don't ask again" recorded anywhere applies there). Off production a code-like
///    definition (`CREATE [OR REPLACE]`/`ALTER` of a `FUNCTION`, `PROCEDURE`, `TRIGGER`,
///    `EVENT`, `RULE`, `POLICY` or `VIEW`; fix rounds 1 and 2) keys on a hash of its whole text
///    too (a new body asks again).
///
/// The verdict is `Read` only for a unit whose report reads only ([`reads_only`], R-M3-23);
/// `Routine` and `Unknown` are `Unknown` with unknown session effects, never `Write`.
pub fn gate(
    units: &[PreparedUnit],
    reports: &[SafetyReport],
    conn: &ConnSafety,
    suppressed: &SuppressSet,
    console: u64,
) -> GatePlan {
    // Reports pair with units by index: when the counts differ no pairing can be trusted.
    let paired = reports.len() == units.len();
    let items = units
        .iter()
        .enumerate()
        .map(|(i, unit)| {
            let report = if paired { reports.get(i) } else { None };
            gate_unit(index(i), unit, report, conn, suppressed, console)
        })
        .collect();
    GatePlan { items }
}

fn index(i: usize) -> u32 {
    u32::try_from(i).unwrap_or(u32::MAX)
}

/// Whether `report` reads only (R-M3-23): classified (not pre-decided), with statements, every
/// one of class `Read` by its field, its effects and its gate class, none opening a write, and
/// the request-level class, effects and flags agreeing. The gate's `Read` verdict and a cursor
/// (`units::suspendable_for`) both need it.
pub fn reads_only(report: &SafetyReport) -> bool {
    report.pre_decision.is_none()
        && !report.items.is_empty()
        && report.request_class == CoarseClass::Read
        && class_of(report.request_effects) == CoarseClass::Read
        && request_floor(report) == GateClass::Read
        && report.items.iter().all(|s| {
            s.class == CoarseClass::Read
                && class_of(s.effects) == CoarseClass::Read
                && !opens_write(s)
                && gate_class(s) == GateClass::Read
        })
}

/// The gate class of the request-level fields alone (class, effects, flags): a synthetic
/// statement of kind `Other` read by `gate_class`, so request-level evidence never lowers a
/// unit's class.
fn request_floor(report: &SafetyReport) -> GateClass {
    let synthetic = StmtReport {
        range: TextRange::default(),
        kind: StmtKind::Other,
        effects: report.request_effects,
        flags: report.flags,
        class: report.request_class,
        touched: TouchedObjects::default(),
        reasons: Vec::new(),
        destructive: Vec::new(),
        confidence: Confidence::Conservative,
        set_targets: Vec::new(),
    };
    gate_class(&synthetic)
}

/// MySQL, MariaDB and TiDB commit an open transaction before a DDL statement (and the rest of
/// [`crate::implicit_commit`]'s list).
fn commits_on_ddl(engine: DbmsKind) -> bool {
    engine_commits_implicitly(engine)
}

/// The dialect `report` was classified in, else `engine`'s.
fn dialect_for(report: &SafetyReport, engine: DbmsKind) -> DialectId {
    report.dialect.unwrap_or_else(|| dialect_id(engine))
}

/// The words of statement `s` (its range of `text`; the whole text when the range is not in it).
fn stmt_atoms(s: &StmtReport, text: &str, d: DialectId) -> Vec<String> {
    atoms(slice(text, s.range).unwrap_or(text), d)
}

/// A transaction is open as the gate sees it: any status but idle, or a Manual session (a
/// Manual MySQL session runs with `autocommit = 0`, so it always counts as open; P-M3b-1).
fn tx_open(conn: &ConnSafety) -> bool {
    conn.tx_status != TxStatus::Idle || conn.tx_mode == TxMode::Manual
}

// ## m3b-gate-explain
/// Whether `report` (of gate class `class`) is the B-M6 read: every statement a plain `EXPLAIN`
/// of a data change that calls no code. A plain `EXPLAIN` still plans, and planning runs code:
/// PostgreSQL pre-evaluates IMMUTABLE functions with constant arguments while planning (and
/// STABLE ones for estimates), and MySQL builds derived tables under a plain `EXPLAIN`, so a
/// stored function in a `FROM` subquery runs (MySQL bug #31468, closed as documented; ADR 0004
/// `m3b-gate-explain` has the sources). So the class must be exactly `Dml` (a routine class
/// below it is hidden by the maximum, hence the explicit checks), and any evidence of a call
/// or of `ANALYZE` keeps the class: a touched call, the `ROUTINE_CALL` bit on a statement or
/// on the request, a destructive pattern, an `EXPLAIN_ANALYZE` flag or reason, or a policy
/// that does not simply allow it under `Admin` (a routine reason, a session effect, doubt).
fn plain_explain_of_dml(report: &SafetyReport, class: GateClass) -> bool {
    class == GateClass::Dml
        && !report.items.is_empty()
        && !report.request_effects.contains(EffectSet::ROUTINE_CALL)
        && !report.flags.contains(RiskFlags::EXPLAIN_ANALYZE)
        && report.items.iter().all(|s| {
            s.kind == (StmtKind::Explain { analyze: false })
                && explain_analyze_inner(s).is_none()
                && !s.effects.contains(EffectSet::ROUTINE_CALL)
                && s.destructive.is_empty()
                && s.touched.calls.is_empty()
        })
        && decide(report, Profile::Admin, Actor::Human).allows()
}

/// Whether the `admin` profile's row for `class` allows (`data/profiles.toml`): the classes
/// whose `decide(.., Admin)` answer is `Allow` when nothing else is in the report. A `Confirm`
/// from `decide` on one of these means `decide` saw evidence the gate class did not.
fn admin_row_allows(class: GateClass) -> bool {
    match class {
        GateClass::Read
        | GateClass::Lock
        | GateClass::Dml
        | GateClass::DmlUnsafe
        | GateClass::DdlOther
        | GateClass::DdlDestructive => true,
        GateClass::Session
        | GateClass::Tx
        | GateClass::Routine
        | GateClass::Admin
        | GateClass::Unknown => false,
    }
}

/// The session-level effects a unit of unknown effects may have: every bit but `DDL` and
/// `COMMITS_FIRST` (which say what the statement is; the gate does not know).
fn unknown_effects() -> SessionEffects {
    SessionEffects::all().difference(SessionEffects::DDL | SessionEffects::COMMITS_FIRST)
}

/// Whether `s` is a DDL statement: its effects, kind, class field, gate class or destructive
/// patterns say so.
fn is_ddl(s: &StmtReport) -> bool {
    s.effects.intersects(EffectSet::DDL | EffectSet::DDL_TEMP)
        || matches!(
            s.kind,
            StmtKind::CreateTable
                | StmtKind::CreateOther
                | StmtKind::Alter
                | StmtKind::Drop
                | StmtKind::DropOwned
                | StmtKind::Truncate
        )
        || s.class == CoarseClass::Ddl
        || matches!(
            gate_class(s),
            GateClass::DdlOther | GateClass::DdlDestructive
        )
        || s.destructive.iter().any(|d| d.kind.is_destructive_ddl())
}

/// The session-level effects of `report` over `text` (the bytes it classified) on `engine`
/// (R-M3-83: the session schedules with them; the read-only guard never trusts them). The one
/// mapping from a report to `SessionEffects` (fix round 1, M8): the classifier pool's
/// `StatementClassifier::effects` (Task 14) delegates here.
///
/// `DDL` is set for every DDL statement on every engine (temporary-table DDL included).
/// `COMMITS_FIRST` (fix round 2, R1-I1) is set on MySQL/MariaDB/TiDB for exactly the statements
/// of [`crate::implicit_commit`]'s list (DDL but `CREATE`/`DROP TEMPORARY TABLE`, privileges,
/// `BEGIN`, `LOCK TABLES`, `SET autocommit = 1`, `FLUSH`, `LOAD DATA`, …), so the session's T19
/// backstop reuses that list through it. A report with no statements has every bit but those
/// two, plus `COMMITS_FIRST` when its words are on the list.
pub fn session_effects_of(report: &SafetyReport, text: &str, engine: DbmsKind) -> SessionEffects {
    let mysql = commits_on_ddl(engine);
    let d = dialect_for(report, engine);
    if report.items.is_empty() {
        let commits_first = mysql && commits(&atoms(text, d));
        return unknown_effects()
            | if commits_first {
                SessionEffects::COMMITS_FIRST
            } else {
                SessionEffects::empty()
            };
    }
    let mut out = SessionEffects::empty();
    for s in &report.items {
        let e = s.effects;
        let words = if mysql {
            stmt_atoms(s, text, d)
        } else {
            Vec::new()
        };
        if e.contains(EffectSet::SESSION_STATE) || matches!(s.kind, StmtKind::Set | StmtKind::Use) {
            out |= SessionEffects::SESSION_STATE;
        }
        if e.intersects(EffectSet::WRITE_TEMP | EffectSet::DDL_TEMP) {
            out |= SessionEffects::WRITE_TEMP;
        }
        if e.contains(EffectSet::ADVISORY_LOCK) {
            out |= SessionEffects::ADVISORY_LOCK;
        }
        let lock_tables = e.contains(EffectSet::LOCK_TABLE)
            || s.kind == StmtKind::Lock
            || (words.first().is_some_and(|w| w == "LOCK")
                && words.get(1).is_some_and(|w| w == "TABLE" || w == "TABLES"));
        if mysql && lock_tables {
            out |= SessionEffects::LOCK_TABLES;
        }
        if e.contains(EffectSet::PREPARE) || s.kind == StmtKind::Prepare {
            out |= SessionEffects::PREPARE;
        }
        if opens_write(s) {
            out |= SessionEffects::OPENS_WRITE;
        }
        let privilege = e.contains(EffectSet::PRIVILEGE)
            || matches!(s.kind, StmtKind::Grant | StmtKind::Revoke);
        if is_ddl(s) {
            out |= SessionEffects::DDL;
        }
        let commits_first = commits(&words) || privilege || (is_ddl(s) && !temporary_table(&words));
        if mysql && commits_first {
            out |= SessionEffects::COMMITS_FIRST;
        }
    }
    out
}

/// Whether a MySQL-family statement of `report` cannot be rolled back ("MySQL commits DDL at
/// once"): any DDL statement but `CREATE`/`DROP TEMPORARY TABLE`, and privilege statements.
fn irreversible_ddl(report: &SafetyReport, text: &str, engine: DbmsKind) -> bool {
    let d = dialect_for(report, engine);
    commits_on_ddl(engine)
        && report.items.iter().any(|s| {
            let privilege = s.effects.contains(EffectSet::PRIVILEGE)
                || matches!(s.kind, StmtKind::Grant | StmtKind::Revoke);
            privilege || (is_ddl(s) && !temporary_table(&stmt_atoms(s, text, d)))
        })
}

/// What the gate reads of one classified unit.
struct Facts {
    /// The most critical gate class of its statements and of the request-level fields.
    class: GateClass,
    /// It holds an unsafe-DML pattern (`DmlUnsafe` by kind, even under a DDL class).
    unsafe_dml: bool,
    effects: SessionEffects,
    /// The inner class of a lone `EXPLAIN ANALYZE` whose inner statement is not a read.
    explain: Option<GateClass>,
    /// An `EXPLAIN ANALYZE` of a non-read among several statements.
    explain_in_script: bool,
    read: bool,
    /// Pre-decided by the classifier stub (`NotImplemented`).
    not_classified: bool,
    // ## m3b-t15-gate (fix round 1)
    /// Not analysed: pre-decided, or no statement (I1).
    unanalysed: bool,
    /// A MySQL-family DDL statement that cannot be rolled back.
    irreversible: bool,
    /// Several statements, one of them a write (M9: never suppressible).
    mixed_write: bool,
}

impl Facts {
    fn of(report: &SafetyReport, text: &str, engine: DbmsKind) -> Facts {
        let class = if report.pre_decision.is_some() || report.items.is_empty() {
            GateClass::Unknown
        } else {
            report
                .items
                .iter()
                .map(gate_class)
                .max()
                .unwrap_or(GateClass::Unknown)
                .max(request_floor(report))
        };
        let inners: Vec<GateClass> = report
            .items
            .iter()
            .filter_map(explain_analyze_inner)
            .filter(|c| *c != GateClass::Read)
            .collect();
        let lone = report.items.len() == 1;
        Facts {
            class,
            unsafe_dml: report
                .items
                .iter()
                .flat_map(|s| &s.destructive)
                .any(|d| d.kind.is_unsafe_dml()),
            effects: session_effects_of(report, text, engine),
            explain: if lone { inners.first().copied() } else { None },
            explain_in_script: !lone && !inners.is_empty(),
            read: reads_only(report),
            not_classified: report
                .pre_decision
                .iter()
                .flatten()
                .any(|r| r.code == ReasonCode::NotImplemented),
            unanalysed: report.pre_decision.is_some() || report.items.is_empty(),
            irreversible: irreversible_ddl(report, text, engine),
            mixed_write: report.items.len() > 1 && report.items.iter().any(opens_write),
        }
    }
}

/// A denied unit.
fn denied(
    unit: u32,
    class: GateClass,
    effects: SessionEffects,
    fingerprint: u64,
    reason: GateReason,
) -> GateItem {
    GateItem {
        unit,
        decision: GateDecision::Deny(reason),
        verdict: UnitVerdict::Unknown,
        effects,
        class,
        fingerprint,
    }
}

/// `exec-gate-read-only` with the class named (`$class`, a Fluent selector).
fn read_only_reason(class: GateClass) -> GateReason {
    let key = match class {
        GateClass::Read => "read",
        GateClass::Session => "session",
        GateClass::Tx => "tx",
        GateClass::Lock => "lock",
        GateClass::Routine => "routine",
        GateClass::Dml | GateClass::DmlUnsafe => "dml",
        GateClass::DdlOther | GateClass::DdlDestructive => "ddl",
        GateClass::Admin => "admin",
        GateClass::Unknown => "unknown",
    };
    GateReason {
        fluent_id: READ_ONLY,
        args: vec![("class", Arc::from(key))],
    }
}

/// A classifier reason as the dialog names it (`$name`: the function or variable, as written).
fn reason_of(r: &Reason) -> GateReason {
    GateReason {
        fluent_id: r.code.fluent_id(),
        args: r
            .arg
            .as_ref()
            .map(|a| vec![("name", a.clone())])
            .unwrap_or_default(),
    }
}

fn push_unique(v: &mut Vec<GateReason>, r: GateReason) {
    if !v.contains(&r) {
        v.push(r);
    }
}

/// `parts` joined with `.`.
fn dotted<'a>(parts: impl Iterator<Item = &'a str>) -> Arc<str> {
    Arc::from(parts.collect::<Vec<_>>().join("."))
}

fn same_name(d: DialectId, a: &QualifiedName, b: &QualifiedName) -> bool {
    a.parts.len() == b.parts.len() && a.parts.iter().zip(&b.parts).all(|(x, y)| ident_eq(d, x, y))
}

fn slice(text: &str, r: TextRange) -> Option<&str> {
    text.get(usize::from(r.start())..usize::from(r.end()))
}

/// `target`'s name as the user wrote it in `text` (each part's exact bytes, quotes removed,
/// never case-folded): [`written_qualified`]'s reading, else the classifier's spelling (a
/// server-resolved name: still the object that changes).
fn written_name(
    target: &ObjectRef,
    hint: TextRange,
    stmt: TextRange,
    text: &str,
    dialect: Option<DialectId>,
) -> Arc<str> {
    written_qualified(target, hint, stmt, text, dialect)
        .map_or_else(|| as_typed(&target.name), |q| as_typed(&q))
}

// ## m3c-p2-07 (fix round 2, I1, M1)
/// `target`'s name as the user wrote it in `text`, each part's text and quoting exactly as
/// written (quotes removed; never case-folded, never cut at 63 bytes): the destructive pattern's
/// own range when it reads as that name, else the first dotted name in the statement's range
/// that names it (`ident_eq` per part); `None` when neither does.
fn written_qualified(
    target: &ObjectRef,
    hint: TextRange,
    stmt: TextRange,
    text: &str,
    dialect: Option<DialectId>,
) -> Option<QualifiedName> {
    let d = dialect?;
    let named = |s: &str| parse_qualified(d, s.trim()).filter(|q| same_name(d, q, &target.name));
    if let Some(q) = slice(text, hint).and_then(named) {
        return Some(q);
    }
    let body = slice(text, stmt)?;
    let ctx = DialectCtx::new(d, None);
    let profile = lex_profile(&ctx).ok()?;
    let lexed = lex(body, profile, ctx.modes, &LexState::Initial);
    let ident = |i: usize| matches!(lexed.kind(i), TokenKind::Word | TokenKind::QuotedIdent);
    let next = |mut i: usize| {
        i += 1;
        while i < lexed.len() && lexed.is_trivia(i) {
            i += 1;
        }
        (i < lexed.len()).then_some(i)
    };
    for start in (0..lexed.len()).filter(|&i| ident(i)) {
        let mut end = start;
        while let Some(dot) = next(end).filter(|&k| lexed.kind(k) == TokenKind::Dot) {
            match next(dot).filter(|&k| ident(k)) {
                Some(k) => end = k,
                None => break,
            }
        }
        let range = TextRange::new(lexed.range(start).start(), lexed.range(end).end());
        if let Some(q) = slice(body, range).and_then(named) {
            return Some(q);
        }
    }
    None
}
// ## m3c-p2-07

/// The name a production confirmation asks to type (R-M3-55): the target of every unsafe-DML
/// pattern, of a `DROP DATABASE`/`DROP SCHEMA` and (## m3b-po-typed-name, PO M3b-04) of a
/// `TRUNCATE` or `DROP TABLE`, and (## m3c-p2-07, PO P2-07) the role of a `DROP OWNED BY`, as
/// written. From the destructive kinds, not the class. `Err` when one has no target (nothing to
/// type) or two name different objects (typing one name must not approve another): since
/// m3c-p2-07 fix round 1 the objects are told apart by [`TypedKey`] (the engine's resolution of
/// each target, with its kind), never by the written text alone.
fn typed_name(
    report: &SafetyReport,
    text: &str,
    engine: DbmsKind,
) -> Result<Option<Arc<str>>, GateReason> {
    // ## m3c-p2-07 (fix round 2, I2): every name with its statement, before any merges.
    let mut found: Vec<(usize, Arc<str>, TypedKey)> = Vec::new();
    let mut every_stmt_names = true;
    let d = dialect_for(report, engine);
    for (si, s) in report.items.iter().enumerate() {
        let before = found.len();
        // Fix round 1 (I3): the statement's own words say `DROP DATABASE`/`DROP SCHEMA`, so a
        // backend that names the target's kind wrongly cannot drop the typed name.
        let dropped_db = drop_database_name(slice(text, s.range).unwrap_or(text), d);
        // ## m3b-po-typed-name (PO M3b-04): the statement's own leading words, so a backend
        // that names a dropped table's kind wrongly cannot drop the typed name either.
        let lead = leading_words(slice(text, s.range).unwrap_or(text), d, 3);
        let word = |k: usize, w: &str| lead.get(k).is_some_and(|x| x == w);
        let drops_table = word(0, "DROP")
            && (word(1, "TABLE")
                || (["TEMPORARY", "TEMP", "FOREIGN"].iter().any(|w| word(1, w))
                    && word(2, "TABLE")));
        let alters = word(0, "ALTER");
        // ## m3c-p2-07 (PO P2-07; fix round 2, M5): `DROP OWNED BY` names the roles whose
        // objects go. It is the classifier's own statement kind; when its words disagree, the
        // unit is denied (fail closed).
        let drops_owned = s.kind == StmtKind::DropOwned;
        if drops_owned != (word(0, "DROP") && word(1, "OWNED")) {
            return Err(GateReason::of(DROP_OWNED_NO_TARGET));
        }
        // ## m3c-p2-07 (fix round 1): what a schema-or-database drop removes, by its words.
        let db_group = |t: Option<&ObjectRef>| {
            if word(1, "DATABASE")
                || (!word(1, "SCHEMA") && t.is_some_and(|t| t.kind == ObjectKind::Database))
            {
                Group::Database
            } else {
                Group::Schema
            }
        };
        let mut named = false;
        for dr in &s.destructive {
            let schema_or_db = matches!(dr.kind, DestructiveKind::Drop { .. })
                && (dropped_db.is_some()
                    || dr.target.as_ref().is_some_and(|t| {
                        matches!(t.kind, ObjectKind::Schema | ObjectKind::Database)
                    }));
            // ## m3b-po-typed-name (PO M3b-04): `TRUNCATE` and `DROP TABLE` name the table
            // that loses its rows, as an unbounded `DELETE` does (fail closed: no table to
            // name is denied, two are several targets). `ALTER TABLE … DROP PARTITION` keeps
            // its row.
            let table_gone = match dr.kind {
                DestructiveKind::Truncate => true,
                DestructiveKind::Drop { .. } if !schema_or_db => {
                    drops_table
                        || (!alters
                            && dr
                                .target
                                .as_ref()
                                .is_some_and(|t| t.kind == ObjectKind::Table))
                }
                _ => false,
            };
            // ## m3c-p2-07 (PO P2-07): every `Drop` of a `DROP OWNED` names its role (fail
            // closed: no role to name is denied, two are several targets).
            let owned_gone = drops_owned && matches!(dr.kind, DestructiveKind::Drop { .. });
            if !dr.kind.is_unsafe_dml() && !schema_or_db && !table_gone && !owned_gone {
                continue;
            }
            if owned_gone {
                let Some(role) = dr.target.as_ref() else {
                    return Err(GateReason::of(DROP_OWNED_NO_TARGET));
                };
                let w = written_qualified(role, dr.range, s.range, text, report.dialect);
                let n = w.as_ref().map_or_else(|| as_typed(&role.name), as_typed);
                let key = TypedKey::of(d, Group::Role, &role.name, w.as_ref(), found.len());
                found.push((si, n, key));
                continue;
            }
            // ## m3c-p2-07
            // ## m3b-t26-prod-unlock (named exception, T26 carry MUST 1): a pattern without a
            // target of its own names every object its statement writes, never only the first
            // (`UPDATE a JOIN b … SET b.y = 2` writes `b`): typing one name must never approve
            // another, so two are several targets.
            let targets: Vec<&ObjectRef> = match dr.target.as_ref() {
                Some(t) => vec![t],
                None => s.touched.writes.iter().collect(),
            };
            if targets.is_empty() {
                if schema_or_db && let Some(Some(q)) = &dropped_db {
                    let key = TypedKey::of(d, db_group(None), q, Some(q), found.len());
                    found.push((si, as_typed(q), key));
                    named = true;
                    continue;
                }
                return Err(GateReason::of(NO_TARGET));
            }
            for target in targets {
                // ## m3c-p2-07 (fix round 1; fix round 2, I1, M1: keyed on the written name)
                let group = if schema_or_db {
                    db_group(Some(target))
                } else {
                    Group::Relation
                };
                let w = written_qualified(target, dr.range, s.range, text, report.dialect);
                let n = w.as_ref().map_or_else(|| as_typed(&target.name), as_typed);
                let key = TypedKey::of(d, group, &target.name, w.as_ref(), found.len());
                found.push((si, n, key));
            }
            named |= schema_or_db;
        }
        match dropped_db {
            Some(Some(q)) if !named => {
                let key = TypedKey::of(d, db_group(None), &q, Some(&q), found.len());
                found.push((si, as_typed(&q), key));
            }
            Some(None) if !named => return Err(GateReason::of(NO_TARGET)),
            _ => {}
        }
        every_stmt_names &= found.len() > before;
    }
    // ## m3c-p2-07 (fix round 2, I2): a statement that names no typed target may change what
    // an unqualified name resolves to (`SET search_path`, `USE`, a rename, `set_config`, …), so
    // unless every statement names one, an unqualified name merges only within its statement.
    let mut names: Vec<Typed> = Vec::new();
    for (si, n, mut key) in found {
        if !every_stmt_names && key.parts.len() == 1 {
            key.scope = Some(si);
        }
        push_name(&mut names, n, key);
    }
    match names.len() {
        0 => Ok(None),
        1 => Ok(names.pop().map(|(n, _)| n)),
        // ## m3c-p2-07 (PO P2-07)
        _ if names.iter().any(|(_, k)| k.group == Group::Role) => {
            Err(GateReason::of(DROP_OWNED_SEVERAL_TARGETS))
        }
        _ => Err(GateReason::of(SEVERAL_TARGETS)),
    }
}

// ## m3c-p2-07 (PO P2-07; fix round 1, controller ruling; fix round 2, I1, I2, M1)
/// The kind of object a typed name approves: a table (or any relation a write names), a
/// schema, a database, a role whose objects `DROP OWNED` drops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    Relation,
    Schema,
    Database,
    Role,
}

/// How a key part was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PartKind {
    /// ASCII, folded by the engine's case rule.
    Folded,
    /// Non-ASCII, written bare: never case-folded.
    BareNonAscii,
    /// Non-ASCII, written quoted.
    QuotedNonAscii,
    /// `CURRENT_USER`, `CURRENT_ROLE`, `SESSION_USER` or `PUBLIC` (a `DROP OWNED` role).
    Keyword,
}

/// The object a typed name approves, as far as the key can prove it: two targets with
/// different keys are two objects (several targets), whatever their written text.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TypedKey {
    group: Group,
    parts: Vec<(Arc<str>, PartKind)>,
    /// I2: the statement an unqualified name is resolved in, when another statement of the
    /// unit may change that resolution.
    scope: Option<usize>,
    /// I1: a classifier name that may be cut at 63 bytes and whose written spelling was not
    /// found: it never merges.
    unique: Option<usize>,
}

impl TypedKey {
    /// The key of the name the user wrote (`written`; else the classifier's `name`), part by
    /// part (a schema qualifier counts), in dialect `d`:
    /// - an ASCII part folds by the engine's case rule, never cut at 63 bytes (I1): PostgreSQL
    ///   and the PG-wire dialects lower an unquoted part and keep a quoted one; SQLite lowers
    ///   every part; MySQL, MariaDB and the others keep the exact text (a MySQL table name on
    ///   a case-sensitive file system is case-sensitive; `fold`'s docs), quoting aside;
    /// - a part with a non-ASCII byte is never folded, and its quoting counts (M1);
    /// - a `DROP OWNED` keyword role (the classifier's unquoted reading) is the keyword, never
    ///   the quoted role spelled alike.
    ///
    /// Without a written spelling, a part of 63 bytes or more may have been cut, so the key is
    /// unique (`seq`).
    fn of(
        d: DialectId,
        group: Group,
        name: &QualifiedName,
        written: Option<&QualifiedName>,
        seq: usize,
    ) -> TypedKey {
        let keyword = group == Group::Role && name.parts.iter().any(|p| !p.quoted);
        let (src, unique) = match written {
            Some(w) => (w, None),
            None => (
                name,
                name.parts
                    .iter()
                    .any(|p| p.text.len() >= PG_MAX_IDENT_BYTES)
                    .then_some(seq),
            ),
        };
        let pg = matches!(d, DialectId::Postgres | DialectId::PgWire(_));
        let parts = src
            .parts
            .iter()
            .map(|p| {
                if keyword {
                    (Arc::from(p.text.to_ascii_lowercase()), PartKind::Keyword)
                } else if !p.text.is_ascii() {
                    let kind = if p.quoted {
                        PartKind::QuotedNonAscii
                    } else {
                        PartKind::BareNonAscii
                    };
                    (p.text.clone(), kind)
                } else if d == DialectId::Sqlite || (pg && !p.quoted) {
                    (Arc::from(p.text.to_ascii_lowercase()), PartKind::Folded)
                } else {
                    (p.text.clone(), PartKind::Folded)
                }
            })
            .collect();
        TypedKey {
            group,
            parts,
            scope: None,
            unique,
        }
    }
}

/// A name to type (as written) and the object it approves.
type Typed = (Arc<str>, TypedKey);

/// Adds `n` unless its object is already named: one object written two ways asks once, for its
/// first spelling (the typed comparison stays byte for byte against that spelling).
fn push_name(names: &mut Vec<Typed>, n: Arc<str>, key: TypedKey) {
    if !names.iter().any(|(_, k)| *k == key) {
        names.push((n, key));
    }
}

/// `q`'s parts as written, quotes removed, joined with `.`.
fn as_typed(q: &QualifiedName) -> Arc<str> {
    dotted(q.parts.iter().map(|p| &*p.text))
}
// ## m3c-p2-07

/// When `stmt` (in dialect `d`) reads `DROP DATABASE|SCHEMA [IF EXISTS] <name>`: the name as
/// written (`Some(None)` when none can be read); `None` for any other statement.
/// ## m3c-p2-07 (fix round 1): the parsed name, so its key keeps its quoting.
fn drop_database_name(stmt: &str, d: DialectId) -> Option<Option<QualifiedName>> {
    let ctx = DialectCtx::new(d, None);
    let profile = lex_profile(&ctx).ok()?;
    let lexed = lex(stmt, profile, ctx.modes, &LexState::Initial);
    let code: Vec<usize> = (0..lexed.len())
        .filter(|&i| {
            !lexed.is_trivia(i)
                && !matches!(
                    lexed.kind(i),
                    TokenKind::LineComment | TokenKind::BlockComment | TokenKind::HintComment
                )
        })
        .collect();
    let word = |k: usize, w: &str| {
        code.get(k).is_some_and(|&i| {
            lexed.kind(i) == TokenKind::Word
                && slice(stmt, lexed.range(i)).is_some_and(|t| t.eq_ignore_ascii_case(w))
        })
    };
    if !word(0, "DROP") || !(word(1, "DATABASE") || word(1, "SCHEMA")) {
        return None;
    }
    let mut k = 2;
    if word(k, "IF") && word(k + 1, "EXISTS") {
        k += 2;
    }
    let Some(&first) = code.get(k) else {
        return Some(None);
    };
    // The name runs to the next token that is no part of a dotted name.
    let mut end = first;
    for &i in &code[k..] {
        if !matches!(
            lexed.kind(i),
            TokenKind::Word | TokenKind::QuotedIdent | TokenKind::Dot
        ) || (i != first && lexed.kind(i) != TokenKind::Dot && lexed.kind(end) != TokenKind::Dot)
        {
            break;
        }
        end = i;
    }
    // Fix round 2 (R1-M3): a list (`DROP SCHEMA a, b`) drops more than one name; typing one
    // must not approve the others, so no single name is read.
    let after = code
        .iter()
        .position(|&i| i == end)
        .and_then(|j| code.get(j + 1));
    if after.is_some_and(|&i| lexed.kind(i) == TokenKind::Comma) {
        return Some(None);
    }
    let range = TextRange::new(lexed.range(first).start(), lexed.range(end).end());
    Some(slice(stmt, range).and_then(|s| parse_qualified(d, s)))
}

// ## m3b-po-typed-name (PO M3b-04)
/// The first `n` words of `stmt`'s code (comments and trivia skipped), ASCII-uppercased; it
/// stops at the first token that is not a word.
fn leading_words(stmt: &str, d: DialectId, n: usize) -> Vec<String> {
    let ctx = DialectCtx::new(d, None);
    let Ok(profile) = lex_profile(&ctx) else {
        return Vec::new();
    };
    let lexed = lex(stmt, profile, ctx.modes, &LexState::Initial);
    let mut out = Vec::new();
    for i in 0..lexed.len() {
        if out.len() == n {
            break;
        }
        if lexed.is_trivia(i)
            || matches!(
                lexed.kind(i),
                TokenKind::LineComment | TokenKind::BlockComment | TokenKind::HintComment
            )
        {
            continue;
        }
        match (lexed.kind(i), slice(stmt, lexed.range(i))) {
            (TokenKind::Word, Some(w)) => out.push(w.to_ascii_uppercase()),
            _ => break,
        }
    }
    out
}

/// The reasons a dialog lists: one per destructive pattern (its target as written), then the
/// classifier's reasons, in order, without repeats.
fn reasons_of(report: &SafetyReport, text: &str) -> Vec<GateReason> {
    let mut out = Vec::new();
    for s in &report.items {
        // ## m3c-p2-07 (PO P2-07; fix round 2, M5): a `DROP OWNED` role loses what it owns,
        // not itself; known by the classifier's statement kind.
        let drops_owned = s.kind == StmtKind::DropOwned;
        for d in &s.destructive {
            let reason = match (&d.target, d.kind) {
                (Some(t), DestructiveKind::Drop { cascade }) if drops_owned => GateReason {
                    fluent_id: if cascade {
                        DROP_OWNED_CASCADE
                    } else {
                        DROP_OWNED
                    },
                    args: vec![(
                        "role",
                        written_name(t, d.range, s.range, text, report.dialect),
                    )],
                },
                // ## m3c-p2-07
                (Some(t), _) => GateReason {
                    fluent_id: d.kind.fluent_id(),
                    args: vec![(
                        "table",
                        written_name(t, d.range, s.range, text, report.dialect),
                    )],
                },
                (None, _) => GateReason::of(DESTRUCTIVE),
            };
            push_unique(&mut out, reason);
        }
    }
    for r in report
        .pre_decision
        .iter()
        .flatten()
        .chain(report.items.iter().flat_map(|s| &s.reasons))
    {
        push_unique(&mut out, reason_of(r));
    }
    out
}

/// The R-M3-55 row a class takes before the stricter rules.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Row {
    Run,
    Warn,
    RunInTxThenConfirm,
    Confirm,
}

// ## m3b-t26-prod-unlock (fix round 1, B-I2; fix round 2, R1)
/// Whether `words` (a statement's atoms) define or change code-like objects whose body the
/// fingerprint does not see: `CREATE [OR REPLACE] [DEFINER = …] … FUNCTION | PROCEDURE |
/// TRIGGER | EVENT | RULE | POLICY | VIEW`, or `ALTER` of one. A word among the first dozen is
/// enough (a column named `function` makes it stricter, never weaker).
fn defines_routine(words: &[String]) -> bool {
    matches!(words.first().map(String::as_str), Some("CREATE" | "ALTER"))
        && words.iter().skip(1).take(12).any(|w| {
            matches!(
                w.as_str(),
                "FUNCTION" | "PROCEDURE" | "TRIGGER" | "EVENT" | "RULE" | "POLICY" | "VIEW"
            )
        })
}

/// A hash of `text`, whole (the same text, the same value, in every run of the app).
fn body_hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

// ## m3b-t26-prod-unlock (fix round 1, B-M1; fix round 2, R3)
/// Whether a statement (its atoms `words`, and `raw`, every word of its text with quoted names,
/// strings and comments read too) sets what makes later writes skip their checks, their
/// durability or their replication, or is a `NOTIFY`. A variable is compared by its bare name:
/// `@@`, a scope (`SESSION.`, `@@LOCAL.`, …) and its quotes (`"…"`, `` `…` ``) dropped, in any
/// letter case. A string or comment naming one makes it stricter, never weaker.
fn weakens_later_writes(words: &[String], raw: &[String]) -> bool {
    const VARS: [&str; 9] = [
        "SESSION_REPLICATION_ROLE",
        "SQL_LOG_BIN",
        "FOREIGN_KEY_CHECKS",
        "UNIQUE_CHECKS",
        "SQL_SAFE_UPDATES",
        "GTID_NEXT",
        "PSEUDO_REPLICA_MODE",
        "PSEUDO_SLAVE_MODE",
        "SYNCHRONOUS_COMMIT",
    ];
    // A raw word keeps `@` and `.`: `@@GLOBAL.FOREIGN_KEY_CHECKS` is one; its last part counts.
    let bare = |w: &str| {
        w.rsplit('.')
            .next()
            .unwrap_or(w)
            .trim_start_matches('@')
            .to_string()
    };
    match words.first().map(String::as_str) {
        Some("NOTIFY") => true,
        Some("SET") => raw.iter().any(|w| VARS.contains(&bare(w).as_str())),
        _ => false,
    }
}

/// The "don't ask again" key of a unit (fix round 1, M4): the classified text's fingerprint
/// mixed with the unit's class, so a pair recorded for a statement of one class never silences
/// a statement of the same shape and a higher class.
fn suppression_key(fingerprint: u64, class: GateClass) -> u64 {
    // The 64-bit golden-ratio multiplier spreads the class over every bit.
    fingerprint ^ (class as u64 + 1).wrapping_mul(11_400_714_819_323_198_485)
}

fn gate_unit(
    unit: u32,
    prepared: &PreparedUnit,
    report: Option<&SafetyReport>,
    conn: &ConnSafety,
    suppressed: &SuppressSet,
    console: u64,
) -> GateItem {
    // 1. P-M3b-2: only a report of exactly the bytes sent.
    let Some(report) = report.filter(|r| r.matches_text(&prepared.final_text)) else {
        let reason = GateReason::of(STALE);
        return denied(unit, GateClass::Unknown, SessionEffects::empty(), 0, reason);
    };
    let text = &*prepared.final_text;
    // ## m3b-t26-prod-unlock (fix round 1, B-I2): a routine definition's key also holds its
    // whole text (the fingerprint drops constants, and a body is one).
    let dialect = dialect_for(report, conn.engine);
    let words: Vec<Vec<String>> = report
        .items
        .iter()
        .map(|s| stmt_atoms(s, text, dialect))
        .collect();
    let routine = words.iter().any(|w| defines_routine(w));
    let base = if routine {
        report.fingerprint.hash ^ body_hash(text)
    } else {
        report.fingerprint.hash
    };
    let key = |c: GateClass| suppression_key(base, c);
    let facts = Facts::of(report, text, conn.engine);
    let effects = facts.effects;
    let deny = |class, reason| denied(unit, class, effects, key(class), reason);
    // 2. The classifier stub never lets anything through (F31).
    if facts.not_classified {
        return deny(GateClass::Unknown, GateReason::of(NOT_CLASSIFIED));
    }
    let production = conn.env == Some(EnvTag::Production);
    let mysql = commits_on_ddl(conn.engine);
    let commits_first = mysql && effects.contains(SessionEffects::COMMITS_FIRST);
    let implicit_commit = commits_first && tx_open(conn);

    // 2b. Fix round 1 (I1): a report that analysed nothing (pre-decided, or no statement).
    // Denied on production and read-only connections; off production it never runs silently
    // when its words may write (one calm confirmation, never suppressible), else it runs.
    if facts.unanalysed {
        if conn.read_only {
            return deny(GateClass::Unknown, read_only_reason(GateClass::Unknown));
        }
        if production {
            let reason = report
                .pre_decision
                .iter()
                .flatten()
                .next()
                .map(reason_of)
                .unwrap_or_else(|| GateReason::of(NOT_ANALYSED));
            return deny(GateClass::Unknown, reason);
        }
        // Fix round 2 (R1-M1): the raw words of the whole text, strings and comments included,
        // so a lexing-mode difference (`NO_BACKSLASH_ESCAPES`, `ANSI_QUOTES`) cannot hide a
        // write inside what the default lexer reads as a string.
        let writes = mentions_write(&raw_words(text));
        let decision = if writes || implicit_commit {
            let mut reasons = vec![GateReason::of(NOT_ANALYSED)];
            if implicit_commit {
                push_unique(&mut reasons, GateReason::of(IMPLICIT_COMMIT));
            }
            for r in report.pre_decision.iter().flatten() {
                push_unique(&mut reasons, reason_of(r));
            }
            GateDecision::Confirm {
                reasons,
                type_to_confirm: None,
                irreversible: false,
                suppressible: false,
            }
        } else {
            GateDecision::Run
        };
        return GateItem {
            unit,
            decision,
            verdict: if implicit_commit {
                UnitVerdict::ImplicitCommit
            } else {
                UnitVerdict::Unknown
            },
            effects: effects | unknown_effects(),
            class: GateClass::Unknown,
            fingerprint: key(GateClass::Unknown),
        };
    }

    let mut class = facts.class;
    let explaining = facts.explain.is_some() || facts.explain_in_script;

    // 3. Read-only: reads and the harmless SET list only, and `decide` must agree.
    if conn.read_only {
        let allowed = matches!(class, GateClass::Read | GateClass::Session)
            && !explaining
            && decide(report, Profile::ReadOnlyPlusExplain, Actor::Human).allows();
        if !allowed {
            return deny(class, read_only_reason(class));
        }
        let verdict = if class == GateClass::Read && facts.read {
            UnitVerdict::Read
        } else {
            UnitVerdict::Write
        };
        return GateItem {
            unit,
            decision: GateDecision::Run,
            verdict,
            effects,
            class,
            fingerprint: key(class),
        };
    }

    let mut extra = Vec::new();
    // 4. Production: `decide(Admin)` must not deny, and its confirmations count.
    if production {
        match decide(report, Profile::Admin, Actor::Human).verdict {
            Verdict::Deny(reasons) => {
                let reason = reasons
                    .first()
                    .map(reason_of)
                    .unwrap_or_else(|| GateReason::of(POLICY_DENIED));
                return deny(class, reason);
            }
            Verdict::Confirm(reasons) => {
                if admin_row_allows(class) {
                    class = GateClass::Unknown;
                }
                extra = reasons.iter().map(reason_of).collect();
            }
            Verdict::Allow => {}
        }
    }

    // 5. D-062.
    if facts.explain_in_script {
        return deny(class, GateReason::of(EXPLAIN_REFUSED));
    }
    if let Some(inner) = facts.explain {
        if !wrap_allowed(inner, conn.engine) {
            return deny(class, GateReason::of(EXPLAIN_REFUSED));
        }
        // Fix round 1 (I2, M7): on production the rollback is not enough for an inner statement
        // Hacivat cannot classify (sequences, session advisory locks, dblink, file and network
        // I/O survive it), nor on MySQL/MariaDB/TiDB at all (non-transactional tables keep their
        // writes): it asks first, then runs wrapped. Never suppressible.
        // ## m3b-t26-prod-unlock (fix round 1, B-M2): on production every wrapped inner asks
        // first (row locks, bloat, triggers' side effects outlive the rollback).
        let asks = production;
        let decision = if asks {
            let mut reasons = reasons_of(report, text);
            push_unique(&mut reasons, GateReason::of(EXPLAIN_NOT_UNDONE));
            for r in extra {
                push_unique(&mut reasons, r);
            }
            GateDecision::Confirm {
                reasons,
                type_to_confirm: None,
                irreversible: false,
                suppressible: false,
            }
        } else {
            GateDecision::ExplainAnalyzeRollback
        };
        return GateItem {
            unit,
            decision,
            verdict: UnitVerdict::ExplainAnalyzeRollback,
            effects,
            class,
            fingerprint: key(class),
        };
    }

    // ## m3b-t26-prod-unlock (fix round 1, B-M6): a plain `EXPLAIN` (no `ANALYZE`) of a data
    // change plans it and runs nothing, so a read-write connection runs it as a read (no
    // Keep/Undo prompt over a plan). Only a lone DML class qualifies: a destructive pattern, a
    // call or anything the classifier doubts keeps its class (and asks). The read-only
    // profile's own rule (above) is unchanged.
    // ## m3b-gate-explain: after the production AND-check and D-062 (never before them), and
    // only for a plan of code-free statements (`plain_explain_of_dml`).
    if plain_explain_of_dml(report, class) {
        return GateItem {
            unit,
            decision: GateDecision::Run,
            verdict: UnitVerdict::Read,
            effects: SessionEffects::empty(),
            class: GateClass::Read,
            fingerprint: key(GateClass::Read),
        };
    }

    // 6. The class's row.
    let mut row = if production {
        match class {
            // ## m3b-t26-prod-unlock (fix round 1, B-M1)
            GateClass::Read | GateClass::Session
                if words.iter().zip(report.items.iter()).any(|(w, st)| {
                    weakens_later_writes(w, &raw_words(slice(text, st.range).unwrap_or(text)))
                }) =>
            {
                Row::Confirm
            }
            GateClass::Read | GateClass::Session => Row::Run,
            GateClass::Lock => Row::Warn,
            // ## m3b-t26-prod-unlock (fix round 1, ruling on A-I2/B-I1): never run-then-ask on
            // the MySQL family (a non-transactional table keeps the write).
            GateClass::Dml if mysql => Row::Confirm,
            GateClass::Dml => Row::RunInTxThenConfirm,
            GateClass::DmlUnsafe
            | GateClass::DdlOther
            | GateClass::DdlDestructive
            | GateClass::Routine
            | GateClass::Admin
            | GateClass::Unknown
            | GateClass::Tx => Row::Confirm,
        }
    } else if facts.unsafe_dml && conn.confirm_unsafe_where {
        Row::Confirm
    } else {
        Row::Run
    };

    // 7. Stricter rules on top.
    let type_to_confirm = if production {
        match typed_name(report, text, conn.engine) {
            Ok(name) => name,
            Err(reason) => return deny(class, reason),
        }
    } else {
        None
    };
    if type_to_confirm.is_some() || implicit_commit {
        row = Row::Confirm;
    }
    // Fix round 1 (controller ruling, M9): nowhere a unit of several statements with a write in
    // it. ## m3b-t26-prod-unlock (fix round 2, N1/R1/A R1-m2): on production never, for any
    // class: a production item never consults the set, so nothing recorded off production (or
    // on it, before this round) applies there.
    let never_suppressed = production;
    let suppressible =
        type_to_confirm.is_none() && !implicit_commit && !never_suppressed && !facts.mixed_write;

    let mut reasons = reasons_of(report, text);
    if implicit_commit {
        push_unique(&mut reasons, GateReason::of(IMPLICIT_COMMIT));
    }
    for r in extra {
        push_unique(&mut reasons, r);
    }

    // 8. "Don't ask again".
    let fingerprint = key(class);
    let asks = matches!(row, Row::Confirm | Row::Warn);
    if asks && suppressible && suppressed.contains(console, fingerprint) {
        row = Row::Run;
    }

    let decision = match row {
        Row::Run => GateDecision::Run,
        Row::Warn => GateDecision::Warn(reasons),
        Row::RunInTxThenConfirm => GateDecision::RunInTxThenConfirm { reasons },
        Row::Confirm => GateDecision::Confirm {
            reasons,
            type_to_confirm,
            irreversible: facts.irreversible,
            suppressible,
        },
    };
    let verdict = match row {
        Row::RunInTxThenConfirm => UnitVerdict::TxThenConfirm,
        _ if implicit_commit => UnitVerdict::ImplicitCommit,
        _ => match class {
            GateClass::Read if facts.read => UnitVerdict::Read,
            GateClass::Read | GateClass::Routine | GateClass::Unknown => UnitVerdict::Unknown,
            _ => UnitVerdict::Write,
        },
    };
    let effects = if verdict == UnitVerdict::Unknown {
        effects | unknown_effects()
    } else {
        effects
    };
    GateItem {
        unit,
        decision,
        verdict,
        effects,
        class,
        fingerprint,
    }
}

/// Where the app asks the user (Task 21's dialogs), from the run service (R-M3-84).
pub trait ConfirmPort: Send + Sync + std::fmt::Debug + 'static {
    /// A `Confirm` or `Warn` unit, before the run is sent. Any close of the dialog is
    /// `Cancel`.
    fn confirm(&self, q: ConfirmQuestion) -> oneshot::Receiver<ConfirmAnswer>;
    /// A confirm-in-transaction question (`RunEvent::NeedsCommit`). Any close is `RollBack`.
    fn commit_decision(&self, q: CommitQuestion) -> oneshot::Receiver<CommitAnswer>;
}

/// One confirmation ("statement `index` of `total`").
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConfirmQuestion {
    pub run: RunId,
    pub unit: u32,
    pub index: u32,
    pub total: u32,
    /// The statement with its bind values masked.
    pub sql_masked: Arc<str>,
    pub reasons: Vec<GateReason>,
    pub type_to_confirm: Option<Arc<str>>,
    pub irreversible: bool,
    pub suppressible: bool,
    pub warn_only: bool,
    pub production: bool,
    /// The connection's name.
    pub connection: Arc<str>,
}

/// The answer to a [`ConfirmQuestion`]; any close is `Cancel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConfirmAnswer {
    Execute,
    ExecuteAll,
    ExecuteAndSuppress,
    Skip,
    SkipAll,
    Cancel,
}

/// A confirm-in-transaction question, from `RunEvent::NeedsCommit`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CommitQuestion {
    pub run: RunId,
    pub unit: UnitId,
    /// `NeedsCommit::question`: the answer goes back with it (`commit_decision`; review I1).
    pub question: u64,
    pub affected: Option<u64>,
    pub deadline: Instant,
    /// The form the session chose (`NeedsCommit::savepoint`): Keep/Undo when true.
    pub savepoint: bool,
    pub production: bool,
    pub connection: Arc<str>,
}
