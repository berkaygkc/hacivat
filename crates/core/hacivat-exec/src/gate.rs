// ## m3b-t01c-core-seams
//! The exec gate (HV-EXEC-04, R-M3-55, R-M3-84; Task 15): what Run does with each prepared
//! unit given its classification and the connection: run it, warn, ask for a confirmation
//! (type-to-confirm for the destructive kinds), run it in a transaction and then ask
//! (confirm-in-transaction; the session picks the form, P-M3b-1), wrap an `EXPLAIN ANALYZE`
//! (D-062), or deny it. Pure and synchronous: it never talks to a session.
//!
//! Since Task 15 ([`gate`] lists the rules). The app calls the gate only from Task 24 on
//! (R-M3-52), and production stays read-only until Task 26: the gate unlocks nothing by
//! itself (a read-only connection is gated as read-only whatever its environment).

use std::sync::Arc;
use std::time::Instant;

use hacivat_driver::TxStatus;
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_session::session::{
    CommitAnswer, RunId, SessionEffects, TxMode, TxOwner, UnitId, UnitVerdict,
};
// ## m3b-t15-gate
use sql_dialect::{DialectCtx, DialectId, lex_profile};
use sql_ident::{QualifiedName, ident_eq, parse_qualified};
use sql_lexer::{LexState, TokenKind, lex};
use sql_safety::effects::class_of;
use sql_safety::{
    Actor, CoarseClass, Confidence, DestructiveKind, EffectSet, GateClass, ObjectKind, ObjectRef,
    Profile, Reason, ReasonCode, SafetyReport, StmtKind, StmtReport, TouchedObjects, Verdict,
    decide, gate_class, opens_write,
};
use sql_text::TextRange;
use tokio::sync::oneshot;

use crate::explain_wrap::{explain_analyze_inner, wrap_allowed};
use crate::params::PreparedUnit;
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
    /// The classified text's fingerprint (`SuppressSet` keys on it).
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
///    inside a unit of several statements is refused.
/// 6. **The table**: production `Read`/`Session` → `Run`, `Lock` → `Warn`, `Dml` →
///    `RunInTxThenConfirm` (the session picks the form, P-M3b-1), every other class →
///    `Confirm`; off production `Run`, except a unit holding an unsafe-DML pattern, which asks
///    the calm `Confirm` while `confirm_unsafe_where` is on.
/// 7. **Stricter on top**: type-to-confirm on production (from the destructive kinds, not the
///    class: a DDL in the same unit hides `DmlUnsafe`), the MySQL/MariaDB implicit commit
///    (`Confirm`, verdict `ImplicitCommit`, never suppressible), `irreversible` for their DDL.
/// 8. **Suppression**: a suppressible `Confirm`/`Warn` whose `(console, fingerprint)` the user
///    turned off runs (`Run`); type-to-confirm and implicit commits are never suppressible.
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

/// MySQL, MariaDB and TiDB commit an open transaction before a DDL statement.
fn commits_on_ddl(engine: DbmsKind) -> bool {
    matches!(engine, DbmsKind::MySql | DbmsKind::MariaDb | DbmsKind::TiDb)
}

/// A transaction is open as the gate sees it: any status but idle, or a Manual session (a
/// Manual MySQL session runs with `autocommit = 0`, so it always counts as open; P-M3b-1).
fn tx_open(conn: &ConnSafety) -> bool {
    conn.tx_status != TxStatus::Idle || conn.tx_mode == TxMode::Manual
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

/// The session-level effects a unit of unknown effects may have: every bit but `DDL` (which
/// says the unit is a DDL statement; the gate does not know that it is).
fn unknown_effects() -> SessionEffects {
    SessionEffects::all().difference(SessionEffects::DDL)
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
                | StmtKind::Truncate
        )
        || s.class == CoarseClass::Ddl
        || matches!(
            gate_class(s),
            GateClass::DdlOther | GateClass::DdlDestructive
        )
        || s.destructive.iter().any(|d| d.kind.is_destructive_ddl())
}

/// The session-level effects of `report` on `engine` (R-M3-83: the session schedules with
/// them; the read-only guard never trusts them). `DDL` is set for every DDL statement and, on
/// MySQL/MariaDB/TiDB, for privilege statements too (they commit an open transaction).
fn session_effects(report: &SafetyReport, engine: DbmsKind) -> SessionEffects {
    if report.items.is_empty() {
        return unknown_effects();
    }
    let mysql = commits_on_ddl(engine);
    let mut out = SessionEffects::empty();
    for s in &report.items {
        let e = s.effects;
        if e.contains(EffectSet::SESSION_STATE) || matches!(s.kind, StmtKind::Set | StmtKind::Use) {
            out |= SessionEffects::SESSION_STATE;
        }
        if e.intersects(EffectSet::WRITE_TEMP | EffectSet::DDL_TEMP) {
            out |= SessionEffects::WRITE_TEMP;
        }
        if e.contains(EffectSet::ADVISORY_LOCK) {
            out |= SessionEffects::ADVISORY_LOCK;
        }
        if mysql && (e.contains(EffectSet::LOCK_TABLE) || s.kind == StmtKind::Lock) {
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
        if is_ddl(s) || (mysql && privilege) {
            out |= SessionEffects::DDL;
        }
    }
    out
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
}

impl Facts {
    fn of(report: &SafetyReport, engine: DbmsKind) -> Facts {
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
            effects: session_effects(report, engine),
            explain: if lone { inners.first().copied() } else { None },
            explain_in_script: !lone && !inners.is_empty(),
            read: reads_only(report),
            not_classified: report
                .pre_decision
                .iter()
                .flatten()
                .any(|r| r.code == ReasonCode::NotImplemented),
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
/// never case-folded): the destructive pattern's own range when it reads as that name, else
/// the first dotted name in the statement's range that names it (`ident_eq` per part), else
/// the classifier's spelling (a server-resolved name: still the object that changes).
fn written_name(
    target: &ObjectRef,
    hint: TextRange,
    stmt: TextRange,
    text: &str,
    dialect: Option<DialectId>,
) -> Arc<str> {
    let fallback = || dotted(target.name.parts.iter().map(|p| &*p.text));
    let Some(d) = dialect else {
        return fallback();
    };
    let named = |s: &str| {
        parse_qualified(d, s.trim())
            .filter(|q| same_name(d, q, &target.name))
            .map(|q| dotted(q.parts.iter().map(|p| &*p.text)))
    };
    if let Some(n) = slice(text, hint).and_then(named) {
        return n;
    }
    let Some(body) = slice(text, stmt) else {
        return fallback();
    };
    let ctx = DialectCtx::new(d, None);
    let Ok(profile) = lex_profile(&ctx) else {
        return fallback();
    };
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
        if let Some(n) = slice(body, range).and_then(named) {
            return n;
        }
    }
    fallback()
}

/// The name a production confirmation asks to type (R-M3-55): the target of every unsafe-DML
/// pattern and of a `DROP DATABASE`/`DROP SCHEMA`, as written. From the destructive kinds, not
/// the class. `Err` when one has no target (nothing to type) or two name different objects
/// (typing one name must not approve another).
fn typed_name(report: &SafetyReport, text: &str) -> Result<Option<Arc<str>>, GateReason> {
    let mut names: Vec<Arc<str>> = Vec::new();
    for s in &report.items {
        for d in &s.destructive {
            let schema_or_db = matches!(d.kind, DestructiveKind::Drop { .. })
                && d.target
                    .as_ref()
                    .is_some_and(|t| matches!(t.kind, ObjectKind::Schema | ObjectKind::Database));
            if !d.kind.is_unsafe_dml() && !schema_or_db {
                continue;
            }
            let Some(target) = d.target.as_ref().or_else(|| s.touched.writes.first()) else {
                return Err(GateReason::of(NO_TARGET));
            };
            let name = written_name(target, d.range, s.range, text, report.dialect);
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    match names.len() {
        0 => Ok(None),
        1 => Ok(names.pop()),
        _ => Err(GateReason::of(SEVERAL_TARGETS)),
    }
}

/// The reasons a dialog lists: one per destructive pattern (its target as written), then the
/// classifier's reasons, in order, without repeats.
fn reasons_of(report: &SafetyReport, text: &str) -> Vec<GateReason> {
    let mut out = Vec::new();
    for s in &report.items {
        for d in &s.destructive {
            let reason = match &d.target {
                Some(t) => GateReason {
                    fluent_id: d.kind.fluent_id(),
                    args: vec![(
                        "table",
                        written_name(t, d.range, s.range, text, report.dialect),
                    )],
                },
                None => GateReason::of(DESTRUCTIVE),
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
    let fingerprint = report.fingerprint.hash;
    let facts = Facts::of(report, conn.engine);
    let effects = facts.effects;
    let deny = |class, reason| denied(unit, class, effects, fingerprint, reason);
    // 2. The classifier stub never lets anything through (F31).
    if facts.not_classified {
        return deny(GateClass::Unknown, GateReason::of(NOT_CLASSIFIED));
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
            fingerprint,
        };
    }

    let production = conn.env == Some(EnvTag::Production);
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
        return GateItem {
            unit,
            decision: GateDecision::ExplainAnalyzeRollback,
            verdict: UnitVerdict::ExplainAnalyzeRollback,
            effects,
            class,
            fingerprint,
        };
    }

    // 6. The class's row.
    let mut row = if production {
        match class {
            GateClass::Read | GateClass::Session => Row::Run,
            GateClass::Lock => Row::Warn,
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
    let text = &*prepared.final_text;
    let type_to_confirm = if production {
        match typed_name(report, text) {
            Ok(name) => name,
            Err(reason) => return deny(class, reason),
        }
    } else {
        None
    };
    let mysql_ddl = commits_on_ddl(conn.engine) && effects.contains(SessionEffects::DDL);
    let implicit_commit = mysql_ddl && tx_open(conn);
    if type_to_confirm.is_some() || implicit_commit {
        row = Row::Confirm;
    }
    let suppressible = type_to_confirm.is_none() && !implicit_commit;

    let mut reasons = reasons_of(report, text);
    if implicit_commit {
        push_unique(&mut reasons, GateReason::of(IMPLICIT_COMMIT));
    }
    for r in extra {
        push_unique(&mut reasons, r);
    }

    // 8. "Don't ask again".
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
            irreversible: mysql_ddl,
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
