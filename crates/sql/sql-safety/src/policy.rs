//! The policy table (`alg/13` SQL-SAFE-04; the SAFE-07 model; Task 4): a report, a profile and
//! an actor give a verdict.
//!
//! [`decide`] fails closed. A pre-decided report, a report with no statement, or a policy
//! table that does not load is `Deny`. Otherwise every statement and the request-level fields
//! are judged, and the strictest rule wins (`Deny > Confirm > Allow`):
//!
//! - **Effects:** each effect bit selects a group of `data/profiles.toml` ([`GROUP_OF_BIT`]);
//!   an empty set or an unnamed bit is the `unknown` group, which denies in every built-in
//!   profile but `Admin` (Confirm, audit flag).
//! - **Evidence beyond the effects** selects groups too, so a producer cannot lower a decision
//!   by leaving a field behind: the statement's `class` field, its kind ([`kind_rule`]: a
//!   `DELETE` is a write whatever its bits say), its risk flags ([`FLAG_RULES`]), its reasons
//!   ([`reason_rule`]: a reason is evidence of its group), its destructive patterns, and the
//!   request's class, effects and flags.
//! - **Harmless `SET` (P-M3b-6):** on the read-only profiles, for a human, a `SET` whose every
//!   target is on the dialect's harmless list ([`crate::set_allowed`]) is a read. Everywhere
//!   else a `SET` is the `session` row (`alg/13`: Deny in `read_write_dml` and `ddl`, Confirm
//!   in `admin`); a refused target is named by `SetNotAllowlisted`.
//! - **Doubt** (lexing, splitting, parsing, limits), a lexical-mode change and a role switch
//!   deny in every profile. An executable comment (D-058) or a denylisted function denies on
//!   the read-only profiles and for an AI agent. A `SET_VAR` hint denies unless its variable
//!   is harmless; `MAX_EXECUTION_TIME(0)` (or an unreadable limit) denies.
//! - **Actors:** an AI agent is never allowed an executable comment, a denylisted function or
//!   an UPDATE/DELETE without a bounding WHERE (`alg/13` SAFE-04 step 5); `Automation` is
//!   decided like a human except that the harmless-`SET` relaxation is a human's only.
//!
//! [`crate::gate_class`] reads the same evidence through [`reason_gate_floor`],
//! [`FLAG_RULES`] and [`kind_rule`], so the gate never calls `Read` what `decide` refuses on a
//! read-only profile (fix round 1, I1).

use std::sync::OnceLock;

use serde::Deserialize;
use sql_dialect::DialectId;
use sql_text::TextRange;

use crate::effects::{CoarseClass, EffectSet, RiskFlags, class_of, flags_are_named};
use crate::functions::set_allowed;
use crate::gate_class::GateClass;
use crate::reason::{Reason, ReasonCode};
use crate::report::{SafetyReport, SetTarget, StmtKind, StmtReport};

/// What a connection allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Profile {
    ReadOnly,
    ReadOnlyPlusExplain,
    ReadWriteDml,
    Ddl,
    Admin,
}

impl Profile {
    fn is_read_only(self) -> bool {
        matches!(self, Profile::ReadOnly | Profile::ReadOnlyPlusExplain)
    }
}

/// Who sends the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Actor {
    Human,
    AiAgent,
    Automation,
}

/// The policy's answer.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Verdict {
    Allow,
    Confirm(Vec<Reason>),
    Deny(Vec<Reason>),
}

/// A verdict with the class it was decided for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Decision {
    pub verdict: Verdict,
    pub class: CoarseClass,
}

impl Decision {
    /// Whether the verdict is `Allow`.
    pub fn allows(&self) -> bool {
        self.verdict == Verdict::Allow
    }
}

/// The verdict for `r` under `p` for `a` (`alg/13` SQL-SAFE-04; see the module docs).
pub fn decide(r: &SafetyReport, p: Profile, a: Actor) -> Decision {
    // Each SET target's value reaches the list (P-M3b-T3-1: PG `search_path`).
    decide_with(r, p, a, &set_allowed)
}

// ---------------------------------------------------------------------------------------------
// The table

/// A cell of the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Rule {
    Allow,
    Confirm,
    Deny,
}

/// One effect group's rules, one per built-in profile.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct Row {
    read_only: Rule,
    read_only_plus_explain: Rule,
    read_write_dml: Rule,
    ddl: Rule,
    admin: Rule,
}

impl Row {
    fn rule(&self, p: Profile) -> Rule {
        match p {
            Profile::ReadOnly => self.read_only,
            Profile::ReadOnlyPlusExplain => self.read_only_plus_explain,
            Profile::ReadWriteDml => self.read_write_dml,
            Profile::Ddl => self.ddl,
            Profile::Admin => self.admin,
        }
    }
}

/// `data/profiles.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Table {
    version: u32,
    read: Row,
    explain_analyze_read: Row,
    session: Row,
    lock: Row,
    write: Row,
    write_unbounded: Row,
    write_temp: Row,
    ddl: Row,
    ddl_destructive: Row,
    admin: Row,
    routine: Row,
    unknown: Row,
}

/// An effect group: a table of `data/profiles.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Group {
    Read,
    ExplainAnalyzeRead,
    Session,
    Lock,
    Write,
    WriteUnbounded,
    WriteTemp,
    Ddl,
    DdlDestructive,
    Admin,
    Routine,
    Unknown,
}

impl Group {
    /// The gate class this group stands for (`Session` covers transaction control too; the
    /// callers that know a statement is transaction control use `Tx`).
    pub(crate) fn gate(self) -> GateClass {
        match self {
            Group::Read | Group::ExplainAnalyzeRead => GateClass::Read,
            Group::Session => GateClass::Session,
            Group::Lock => GateClass::Lock,
            Group::Write | Group::WriteTemp => GateClass::Dml,
            Group::WriteUnbounded => GateClass::DmlUnsafe,
            Group::Ddl => GateClass::DdlOther,
            Group::DdlDestructive => GateClass::DdlDestructive,
            Group::Admin => GateClass::Admin,
            Group::Routine => GateClass::Routine,
            Group::Unknown => GateClass::Unknown,
        }
    }

    /// The coarse class this group stands for (`Decision.class` evidence, M1).
    fn coarse(self) -> CoarseClass {
        match self {
            Group::Read | Group::ExplainAnalyzeRead => CoarseClass::Read,
            Group::Session => CoarseClass::Session,
            Group::Lock | Group::Write | Group::WriteUnbounded | Group::WriteTemp => {
                CoarseClass::Write
            }
            Group::Ddl | Group::DdlDestructive => CoarseClass::Ddl,
            Group::Admin => CoarseClass::Admin,
            Group::Routine | Group::Unknown => CoarseClass::Unknown,
        }
    }
}

impl Table {
    fn row(&self, g: Group) -> &Row {
        match g {
            Group::Read => &self.read,
            Group::ExplainAnalyzeRead => &self.explain_analyze_read,
            Group::Session => &self.session,
            Group::Lock => &self.lock,
            Group::Write => &self.write,
            Group::WriteUnbounded => &self.write_unbounded,
            Group::WriteTemp => &self.write_temp,
            Group::Ddl => &self.ddl,
            Group::DdlDestructive => &self.ddl_destructive,
            Group::Admin => &self.admin,
            Group::Routine => &self.routine,
            Group::Unknown => &self.unknown,
        }
    }
}

const PROFILES_TOML: &str = include_str!("../data/profiles.toml");

/// The table, parsed once; `None` when it does not load (then `decide` denies everything).
fn table() -> Option<&'static Table> {
    static TABLE: OnceLock<Option<Table>> = OnceLock::new();
    TABLE
        .get_or_init(|| {
            toml::from_str::<Table>(PROFILES_TOML)
                .ok()
                .filter(|t| t.version == 1)
        })
        .as_ref()
}

// ---------------------------------------------------------------------------------------------
// Evidence: effect bits, flags, kinds, reasons

/// The group and the policy's reason for every named effect bit. `READ` names no reason.
const GROUP_OF_BIT: [(EffectSet, Group, Option<ReasonCode>); 20] = [
    (EffectSet::READ, Group::Read, None),
    (
        EffectSet::WRITE_DATA,
        Group::Write,
        Some(ReasonCode::WriteData),
    ),
    (
        EffectSet::WRITE_TEMP,
        Group::WriteTemp,
        Some(ReasonCode::WriteTemp),
    ),
    (EffectSet::DDL, Group::Ddl, Some(ReasonCode::SchemaChange)),
    (
        EffectSet::DDL_TEMP,
        Group::Ddl,
        Some(ReasonCode::SchemaChange),
    ),
    (
        EffectSet::TXN_CONTROL,
        Group::Session,
        Some(ReasonCode::TxnControl),
    ),
    (
        EffectSet::SESSION_STATE,
        Group::Session,
        Some(ReasonCode::SessionState),
    ),
    (
        EffectSet::LOCK_ROW,
        Group::Lock,
        Some(ReasonCode::LockingClause),
    ),
    (
        EffectSet::LOCK_TABLE,
        Group::Lock,
        Some(ReasonCode::TableLock),
    ),
    (
        EffectSet::ADVISORY_LOCK,
        Group::Lock,
        Some(ReasonCode::AdvisoryLock),
    ),
    (
        EffectSet::PRIVILEGE,
        Group::Admin,
        Some(ReasonCode::PrivilegeGrant),
    ),
    (
        EffectSet::ADMIN,
        Group::Admin,
        Some(ReasonCode::AdminControl),
    ),
    (
        EffectSet::EXTERNAL_IO,
        Group::Admin,
        Some(ReasonCode::ExternalIo),
    ),
    (
        EffectSet::PROCESS_EXEC,
        Group::Admin,
        Some(ReasonCode::ProcessExec),
    ),
    (
        EffectSet::SEQUENCE,
        Group::Write,
        Some(ReasonCode::SequenceAdvance),
    ),
    (
        EffectSet::XID_CONSUME,
        Group::Write,
        Some(ReasonCode::XidConsume),
    ),
    (
        EffectSet::PREPARE,
        Group::Session,
        Some(ReasonCode::PreparedStatement),
    ),
    (
        EffectSet::ROUTINE_CALL,
        Group::Routine,
        Some(ReasonCode::RoutineCall),
    ),
    (EffectSet::NOTIFY, Group::Session, Some(ReasonCode::Notify)),
    (
        EffectSet::UNKNOWN,
        Group::Unknown,
        Some(ReasonCode::EffectsUnknown),
    ),
];

/// What a risk flag means (fix round 1, I2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FlagRule {
    /// The `unknown` group (`DYNAMIC_SQL`; `VOLATILE_FN`, `alg/13` S-04).
    Unknown(ReasonCode),
    /// Deny in every profile (`LEX_MODE_CHANGE`, `alg/13` SAFE-04 step 3).
    DenyAlways(ReasonCode),
    /// Deny on the read-only profiles and for an AI agent (`EXEC_COMMENT`, D-058).
    DenyReadOnlyOrAi(ReasonCode),
    /// The `EXPLAIN ANALYZE` rule over the statement's (inner) effects.
    ExplainAnalyze,
}

/// Every named risk flag with its policy rule and its gate-class floor (a coverage test checks
/// the table against `RiskFlags::all()`, so a new flag cannot be ignored silently).
///
/// `VOLATILE_FN` is set by the producers only for a volatile function no allowlist covers (a
/// user function, or a builtin without a `reviewed` allow row; Task 3's volatile-but-pure hand
/// rows and `ClassifyCtx.fn_allowlist` entries never set it). `EXEC_COMMENT` is `Unknown` for
/// the gate: the executable views are unioned into the effects (`alg/13` SAFE-02 step 4), but
/// a production console confirms before running hidden code.
pub(crate) const FLAG_RULES: [(RiskFlags, FlagRule, GateClass); 5] = [
    (
        RiskFlags::EXEC_COMMENT,
        FlagRule::DenyReadOnlyOrAi(ReasonCode::ExecutableComment),
        GateClass::Unknown,
    ),
    (
        RiskFlags::DYNAMIC_SQL,
        FlagRule::Unknown(ReasonCode::DynamicSql),
        GateClass::Unknown,
    ),
    (
        RiskFlags::LEX_MODE_CHANGE,
        FlagRule::DenyAlways(ReasonCode::LexModeChange),
        GateClass::Unknown,
    ),
    (
        RiskFlags::EXPLAIN_ANALYZE,
        FlagRule::ExplainAnalyze,
        GateClass::Read,
    ),
    (
        RiskFlags::VOLATILE_FN,
        FlagRule::Unknown(ReasonCode::VolatileFunction),
        GateClass::Unknown,
    ),
];

/// What a statement kind says about the statement, whatever its effect bits (fix round 1, I5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KindRule {
    /// The effects decide (`SELECT`, `VALUES`, `TABLE`, `SHOW`, `COPY`, `PRAGMA`, `Other`,
    /// `EXPLAIN` without `ANALYZE`).
    Effects,
    /// At least this group and gate class, named by this reason.
    Floor(Group, GateClass, ReasonCode),
    /// `SET`: the `session` group through the harmless list.
    Set,
    /// `EXPLAIN ANALYZE`: the class of its inner statement (its effects), or `Unknown`.
    ExplainAnalyze,
}

/// The minimum a statement kind implies (the controller's floors for round 1, extended to the
/// plan's remaining unambiguous kinds; an exhaustive match, so a new kind must be placed).
pub(crate) fn kind_rule(k: StmtKind) -> KindRule {
    use GateClass as G;
    use ReasonCode as R;
    use StmtKind as K;
    match k {
        K::Select
        | K::Values
        | K::Table
        | K::Show
        | K::Copy
        | K::Pragma
        | K::Other
        | K::Explain { analyze: false } => KindRule::Effects,
        K::Explain { analyze: true } => KindRule::ExplainAnalyze,
        K::Set => KindRule::Set,
        K::Insert | K::Update | K::Delete | K::Merge => {
            KindRule::Floor(Group::Write, G::Dml, R::WriteData)
        }
        K::Drop | K::DropOwned | K::Truncate => {
            KindRule::Floor(Group::DdlDestructive, G::DdlDestructive, R::Destructive)
        }
        K::CreateTable | K::CreateOther | K::Alter => {
            KindRule::Floor(Group::Ddl, G::DdlOther, R::SchemaChange)
        }
        K::Grant | K::Revoke => KindRule::Floor(Group::Admin, G::Admin, R::PrivilegeGrant),
        K::Call | K::Do => KindRule::Floor(Group::Routine, G::Routine, R::RoutineCall),
        K::Execute => KindRule::Floor(Group::Unknown, G::Unknown, R::DynamicSql),
        K::Begin | K::Commit | K::Rollback | K::Savepoint => {
            KindRule::Floor(Group::Session, G::Tx, R::TxnControl)
        }
        K::Lock => KindRule::Floor(Group::Lock, G::Lock, R::TableLock),
        K::Use | K::Listen => KindRule::Floor(Group::Session, G::Session, R::SessionState),
        K::Prepare => KindRule::Floor(Group::Session, G::Session, R::PreparedStatement),
        K::Notify => KindRule::Floor(Group::Session, G::Session, R::Notify),
    }
}

/// What a reason code in a statement means to the policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReasonRule {
    /// Deny in every profile, for every actor.
    DenyAlways,
    /// Deny on the read-only profiles and for an AI agent.
    DenyReadOnlyOrAi,
    /// Deny on the read-only profiles.
    DenyReadOnly,
    /// Judge as this effect group.
    Group(Group),
    /// `SET_VAR(<var> = …)`: deny unless the variable is harmless on the dialect.
    HintSetVar,
    /// `MAX_EXECUTION_TIME(<n>)`: deny unless `n` is a positive whole number.
    HintExecTime,
    /// The group of the statement's `EXPLAIN ANALYZE` (the flag's rule).
    ExplainAnalyze,
}

/// The policy's reading of `code`: an exhaustive match, so a new code must be placed.
///
/// `PrivilegeChange` is a role or authorization switch (`SET ROLE`, `SET SESSION
/// AUTHORIZATION`), denied everywhere (`alg/13` SAFE-04 step 3); a `GRANT`'s `PRIVILEGE` bit
/// names `PrivilegeGrant`, the `admin` group (fix round 1, M2). A bare `Destructive` reason is
/// destructive DDL evidence (M3).
pub(crate) fn reason_rule(code: ReasonCode) -> ReasonRule {
    use ReasonCode as R;
    match code {
        R::NotImplemented
        | R::LexError
        | R::ClientCommand
        | R::TemplateVariable
        | R::NoStatement
        | R::MultiStatement
        | R::AmbiguousBoundary
        | R::ParseError
        | R::ParserDisagreement
        | R::TooComplex
        | R::SyntaxNewerThanAgent
        | R::UnsupportedDialect
        | R::LexModeChange
        | R::PrivilegeChange => ReasonRule::DenyAlways,
        R::ExecutableComment | R::FnDenylisted => ReasonRule::DenyReadOnlyOrAi,
        R::SetNotAllowlisted => ReasonRule::DenyReadOnly,
        R::FnUnknown
        | R::OperatorUnknown
        | R::CastUnknown
        | R::DynamicSql
        | R::ProceduralCode
        | R::EffectsUnknown
        | R::VolatileFunction => ReasonRule::Group(Group::Unknown),
        R::HintSetVar => ReasonRule::HintSetVar,
        R::HintExecTime => ReasonRule::HintExecTime,
        R::ExplainAnalyze => ReasonRule::ExplainAnalyze,
        R::DataModifyingCte | R::XidConsume | R::SequenceAdvance | R::WriteData => {
            ReasonRule::Group(Group::Write)
        }
        R::LockingClause | R::TableLock | R::AdvisoryLock => ReasonRule::Group(Group::Lock),
        R::TxnControl | R::SessionState | R::PreparedStatement | R::Notify => {
            ReasonRule::Group(Group::Session)
        }
        R::ExternalFile
        | R::ExternalNetwork
        | R::ProcessExec
        | R::AdminControl
        | R::ExternalIo
        | R::PrivilegeGrant => ReasonRule::Group(Group::Admin),
        R::WriteTemp => ReasonRule::Group(Group::WriteTemp),
        R::SchemaChange => ReasonRule::Group(Group::Ddl),
        R::RoutineCall => ReasonRule::Group(Group::Routine),
        R::Destructive => ReasonRule::Group(Group::DdlDestructive),
    }
}

/// Whether a `MAX_EXECUTION_TIME` hint's limit is a positive whole number of milliseconds.
/// `decide` has no ceilings input, so a positive limit is accepted whatever its size (`alg/13`
/// SAFE-04 step 3's ceiling is not in the plan; ADR 0005 `m3b-t04`, fix round 1, M6).
fn hint_exec_time_ok(r: &Reason) -> bool {
    r.arg
        .as_deref()
        .and_then(|n| n.trim().parse::<u64>().ok())
        .is_some_and(|n| n > 0)
}

/// The least gate class a reason in a statement implies (fix round 1, I1). Doubt and every
/// rule that denies regardless of the effects is `Unknown`, a role switch `Admin`, a reason of
/// a group that group's class (transaction control `Tx`), a hint `Session`.
pub(crate) fn reason_gate_floor(r: &Reason) -> GateClass {
    match reason_rule(r.code) {
        ReasonRule::DenyAlways if r.code == ReasonCode::PrivilegeChange => GateClass::Admin,
        ReasonRule::DenyAlways | ReasonRule::DenyReadOnlyOrAi => GateClass::Unknown,
        ReasonRule::DenyReadOnly | ReasonRule::HintSetVar => GateClass::Session,
        ReasonRule::Group(_) if r.code == ReasonCode::TxnControl => GateClass::Tx,
        ReasonRule::Group(g) => g.gate(),
        ReasonRule::HintExecTime if hint_exec_time_ok(r) => GateClass::Read,
        ReasonRule::HintExecTime => GateClass::Session,
        ReasonRule::ExplainAnalyze => GateClass::Read,
    }
}

/// The group that stands for a coarse class (a class field or the request's class that is
/// more critical than the effects show).
fn group_of_class(c: CoarseClass) -> (Group, ReasonCode) {
    match c {
        CoarseClass::Read => (Group::Read, ReasonCode::EffectsUnknown),
        CoarseClass::Session => (Group::Session, ReasonCode::SessionState),
        CoarseClass::Write => (Group::Write, ReasonCode::WriteData),
        CoarseClass::Ddl => (Group::Ddl, ReasonCode::SchemaChange),
        CoarseClass::Admin => (Group::Admin, ReasonCode::AdminControl),
        CoarseClass::Unknown => (Group::Unknown, ReasonCode::EffectsUnknown),
    }
}

// ---------------------------------------------------------------------------------------------
// The judgement

/// The harmless-`SET` list `decide` consults: dialect, variable name, value (`None` when the
/// statement sets none or the backend could not read it; fail-closed in the list).
type Allowed<'a> = &'a dyn Fn(DialectId, &str, Option<&str>) -> bool;

/// The decision being built: the strictest rule so far and the reasons for it.
struct Judge<'a> {
    table: &'a Table,
    profile: Profile,
    actor: Actor,
    deny: Vec<Reason>,
    confirm: Vec<Reason>,
    class: CoarseClass,
    /// The coarse classes of the groups applied to the current statement.
    applied: Vec<CoarseClass>,
    /// The coarse classes of the groups applied to any statement.
    all_applied: Vec<CoarseClass>,
}

fn at(code: ReasonCode, range: Option<TextRange>) -> Reason {
    Reason {
        code,
        range,
        arg: None,
    }
}

impl Judge<'_> {
    fn ai(&self) -> bool {
        self.actor == Actor::AiAgent
    }

    fn push(&mut self, rule: Rule, reason: Reason) {
        let list = match rule {
            Rule::Allow => return,
            Rule::Confirm => &mut self.confirm,
            Rule::Deny => &mut self.deny,
        };
        if !list.contains(&reason) {
            list.push(reason);
        }
    }

    fn deny(&mut self, reason: Reason) {
        self.push(Rule::Deny, reason);
    }

    /// The group of a class field (a statement's or the request's) unless a group of the same
    /// coarse class was already applied: groups of one coarse class have the same row or a
    /// stricter one (`write_unbounded`, `ddl_destructive`), so skipping only drops a duplicate
    /// reason. Applied whatever the effects say: a `Session` field over `WRITE_DATA` still
    /// takes the `session` row, which is stricter than `write` (fix round 1; found by
    /// `decide_is_monotone`).
    fn class_floor(&mut self, c: CoarseClass, range: Option<TextRange>, request: bool) {
        let applied = if request {
            &self.all_applied
        } else {
            &self.applied
        };
        if c == CoarseClass::Read || applied.contains(&c) {
            return;
        }
        let (g, code) = group_of_class(c);
        self.group(g, at(code, range));
    }

    /// Records that a group of coarse class `c` was applied (or, for a harmless `SET`, that its
    /// `Session` class was judged by the harmless list).
    fn mark(&mut self, c: CoarseClass) {
        for list in [&mut self.applied, &mut self.all_applied] {
            if !list.contains(&c) {
                list.push(c);
            }
        }
    }

    fn group(&mut self, g: Group, reason: Reason) {
        self.class = self.class.max(g.coarse());
        self.mark(g.coarse());
        let rule = self.table.row(g).rule(self.profile);
        self.push(rule, reason);
    }

    /// The groups of `effects`' bits; an empty set or an unnamed bit is `unknown`.
    /// `session` replaces the `SESSION_STATE` reason (a `SET` names its refused targets).
    fn effects(
        &mut self,
        effects: EffectSet,
        range: Option<TextRange>,
        session: Option<&[Reason]>,
    ) {
        if effects.is_empty() {
            self.group(Group::Unknown, at(ReasonCode::EffectsUnknown, range));
            return;
        }
        let mut named = EffectSet::empty();
        for (bit, g, code) in GROUP_OF_BIT {
            if !effects.intersects(bit) {
                continue;
            }
            named |= bit;
            match (bit == EffectSet::SESSION_STATE, session, code) {
                (true, Some(reasons), _) => {
                    for r in reasons {
                        self.group(g, r.clone());
                    }
                }
                (_, _, Some(code)) => self.group(g, at(code, range)),
                (_, _, None) => self.group(g, at(ReasonCode::EffectsUnknown, range)),
            }
        }
        if named.bits() != effects.bits() {
            self.group(Group::Unknown, at(ReasonCode::EffectsUnknown, range));
        }
    }

    /// `EXPLAIN ANALYZE` over `effects` (the inner statement's): the `explain_analyze_read`
    /// row when the inner statement is a read; otherwise the inner effects decide (judged with
    /// the statement), and an empty inner set is `unknown` there.
    fn explain_analyze(&mut self, effects: EffectSet, range: Option<TextRange>) {
        if class_of(effects) == CoarseClass::Read {
            self.group(
                Group::ExplainAnalyzeRead,
                at(ReasonCode::ExplainAnalyze, range),
            );
        }
    }

    fn flags(&mut self, flags: RiskFlags, effects: EffectSet, range: Option<TextRange>) {
        if !flags_are_named(flags) {
            self.group(Group::Unknown, at(ReasonCode::EffectsUnknown, range));
        }
        for (flag, rule, _) in FLAG_RULES {
            if !flags.contains(flag) {
                continue;
            }
            match rule {
                FlagRule::Unknown(code) => self.group(Group::Unknown, at(code, range)),
                FlagRule::DenyAlways(code) => self.deny(at(code, range)),
                FlagRule::DenyReadOnlyOrAi(code) => {
                    if self.profile.is_read_only() || self.ai() {
                        self.deny(at(code, range));
                    }
                }
                FlagRule::ExplainAnalyze => self.explain_analyze(effects, range),
            }
        }
    }

    fn reason(
        &mut self,
        r: &Reason,
        s: &StmtReport,
        dialect: Option<DialectId>,
        allowed: Allowed<'_>,
    ) {
        match reason_rule(r.code) {
            ReasonRule::DenyAlways => self.deny(r.clone()),
            ReasonRule::DenyReadOnlyOrAi => {
                if self.profile.is_read_only() || self.ai() {
                    self.deny(r.clone());
                }
            }
            ReasonRule::DenyReadOnly => {
                if self.profile.is_read_only() {
                    self.deny(r.clone());
                }
            }
            ReasonRule::Group(g) => self.group(g, r.clone()),
            ReasonRule::HintSetVar => {
                // The hint's value is not carried: `None` (fail-closed in the list).
                let ok = match (dialect, r.arg.as_deref()) {
                    (Some(d), Some(var)) => allowed(d, var, None),
                    _ => false,
                };
                if !ok {
                    self.deny(r.clone());
                }
            }
            ReasonRule::HintExecTime => {
                if !hint_exec_time_ok(r) {
                    self.deny(r.clone());
                }
            }
            ReasonRule::ExplainAnalyze => self.explain_analyze(s.effects, r.range),
        }
    }

    fn statement(&mut self, s: &StmtReport, dialect: Option<DialectId>, allowed: Allowed<'_>) {
        let range = Some(s.range);
        self.applied.clear();
        self.class = self.class.max(s.class).max(class_of(s.effects));
        let harmless = self.profile.is_read_only()
            && self.actor == Actor::Human
            && harmless_set(s, dialect, allowed);
        match kind_rule(s.kind) {
            KindRule::Effects => self.effects(s.effects, range, None),
            KindRule::Floor(g, _, code) => {
                self.group(g, at(code, range));
                self.effects(s.effects, range, None);
            }
            KindRule::ExplainAnalyze => {
                self.explain_analyze(s.effects, range);
                self.effects(s.effects, range, None);
            }
            KindRule::Set if harmless => {
                self.class = self.class.max(CoarseClass::Session);
                self.mark(CoarseClass::Session);
                self.group(Group::Read, at(ReasonCode::SessionState, range));
            }
            KindRule::Set => {
                let refused = set_refusals(s, dialect, allowed);
                for r in &refused {
                    self.group(Group::Session, r.clone());
                }
                self.effects(s.effects, range, Some(&refused));
            }
        }
        self.flags(s.flags, s.effects, range);
        for r in &s.reasons {
            if harmless && r.code == ReasonCode::SessionState {
                continue;
            }
            self.reason(r, s, dialect, allowed);
        }
        for d in &s.destructive {
            let reason = at(ReasonCode::Destructive, Some(d.range));
            if d.kind.is_unsafe_dml() {
                if self.ai() && d.kind.denied_for_ai() {
                    self.deny(reason.clone());
                }
                self.group(Group::WriteUnbounded, reason);
            } else {
                self.group(Group::DdlDestructive, reason);
            }
        }
        // The class field, after the evidence that names its group more precisely.
        self.class_floor(s.class, range, false);
    }

    fn finish(self) -> Decision {
        let verdict = if !self.deny.is_empty() {
            Verdict::Deny(self.deny)
        } else if !self.confirm.is_empty() {
            Verdict::Confirm(self.confirm)
        } else {
            Verdict::Allow
        };
        Decision {
            verdict,
            class: self.class,
        }
    }
}

/// Whether the harmless list accepts `t` on `dialect` (no dialect: never).
fn target_allowed(t: &SetTarget, dialect: Option<DialectId>, allowed: Allowed<'_>) -> bool {
    dialect.is_some_and(|d| allowed(d, &t.name, t.value.as_deref()))
}

/// A `SET` the dialect's harmless list covers: only `SESSION_STATE`, no flag, no destructive
/// pattern, a class no higher than `Session`, a known dialect, at least one target in
/// `set_targets` and every one accepted by `allowed` with its value, and no reason but
/// `SessionState`, whose arguments name exactly the targets ([`targets_match_reasons`]). The
/// caller applies it on the read-only profiles for a human only (P-M3b-6; fix round 1, I3).
fn harmless_set(s: &StmtReport, dialect: Option<DialectId>, allowed: Allowed<'_>) -> bool {
    s.kind == StmtKind::Set
        && s.effects == EffectSet::SESSION_STATE
        && s.flags.is_empty()
        && s.destructive.is_empty()
        && s.class <= CoarseClass::Session
        && !s.set_targets.is_empty()
        && s.set_targets
            .iter()
            .all(|t| target_allowed(t, dialect, allowed))
        && s.reasons.iter().all(|r| r.code == ReasonCode::SessionState)
        && targets_match_reasons(s)
}

/// Whether the `SessionState` reasons' arguments and the `set_targets` names are the same
/// list (as multisets, byte-exact; a `SessionState` reason without an argument never
/// matches). The producer contract emits one of each per target; a report where the two
/// disagree names a variable the harmless check did not see, so it is not harmless (re-review
/// R1-M3: the "field left behind" pattern).
fn targets_match_reasons(s: &StmtReport) -> bool {
    let mut named: Vec<Option<&str>> = s
        .reasons
        .iter()
        .filter(|r| r.code == ReasonCode::SessionState)
        .map(|r| r.arg.as_deref())
        .collect();
    let mut targets: Vec<Option<&str>> = s.set_targets.iter().map(|t| Some(&*t.name)).collect();
    named.sort_unstable();
    targets.sort_unstable();
    named == targets
}

/// Why a `SET` is refused: `SetNotAllowlisted` naming each target the list refuses (all of
/// them when the dialect is unknown); a `SET` that names no target (`RESET ALL`, a computed
/// name) is `SessionState` without an argument, so every `SetNotAllowlisted` names its
/// variable (fix round 1, M8). A `SessionState` reason naming a variable that is not among
/// the targets is refused by that name too (re-review R1-M3).
fn set_refusals(s: &StmtReport, dialect: Option<DialectId>, allowed: Allowed<'_>) -> Vec<Reason> {
    let refused = |name: &std::sync::Arc<str>| Reason {
        code: ReasonCode::SetNotAllowlisted,
        range: Some(s.range),
        arg: Some(name.clone()),
    };
    let mut out: Vec<Reason> = s
        .set_targets
        .iter()
        .filter(|t| !target_allowed(t, dialect, allowed))
        .map(|t| refused(&t.name))
        .collect();
    for r in &s.reasons {
        if r.code != ReasonCode::SessionState {
            continue;
        }
        if let Some(name) = &r.arg
            && !s.set_targets.iter().any(|t| t.name == *name)
        {
            let reason = refused(name);
            if !out.contains(&reason) {
                out.push(reason);
            }
        }
    }
    if out.is_empty() {
        out.push(at(ReasonCode::SessionState, Some(s.range)));
    }
    out
}

/// [`decide`] with the harmless-`SET` list injected (the unit tests use a fake one).
fn decide_with(r: &SafetyReport, p: Profile, a: Actor, allowed: Allowed<'_>) -> Decision {
    let refuse = |reasons: Vec<Reason>| Decision {
        verdict: Verdict::Deny(reasons),
        class: CoarseClass::Unknown,
    };
    if let Some(pre) = &r.pre_decision {
        return refuse(if pre.is_empty() {
            vec![Reason::new(ReasonCode::NotImplemented)]
        } else {
            pre.clone()
        });
    }
    if r.items.is_empty() {
        return refuse(vec![Reason::new(ReasonCode::NoStatement)]);
    }
    let Some(table) = table() else {
        return refuse(vec![Reason::new(ReasonCode::NotImplemented)]);
    };
    let mut j = Judge {
        table,
        profile: p,
        actor: a,
        deny: Vec::new(),
        confirm: Vec::new(),
        class: CoarseClass::Read,
        applied: Vec::new(),
        all_applied: Vec::new(),
    };
    let mut shown = EffectSet::empty();
    for s in &r.items {
        j.statement(s, r.dialect, allowed);
        shown |= s.effects;
    }
    // Request-level fields the statements do not show.
    let extra = EffectSet::from_bits_retain(r.request_effects.bits() & !shown.bits());
    if !extra.is_empty() {
        j.effects(extra, None, None);
    }
    j.class_floor(r.request_class, None, true);
    j.class_floor(class_of(r.request_effects), None, true);
    j.class = j
        .class
        .max(r.request_class)
        .max(class_of(r.request_effects));
    j.flags(r.flags, r.request_effects, None);
    j.finish()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use smallvec::SmallVec;
    use sql_dialect::DialectCtx;
    use sql_text::TextRange;

    use super::*;
    use crate::fingerprint::classified_hash;
    use crate::report::{Confidence, TouchedObjects, merge};

    #[test]
    fn the_profile_table_loads() {
        let t = table().expect("data/profiles.toml");
        assert_eq!(t.version, 1);
        // Unknown never allows; admin confirms it (audit flag).
        for p in [
            Profile::ReadOnly,
            Profile::ReadOnlyPlusExplain,
            Profile::ReadWriteDml,
            Profile::Ddl,
        ] {
            assert_eq!(t.row(Group::Unknown).rule(p), Rule::Deny, "{p:?}");
        }
        assert_eq!(t.row(Group::Unknown).rule(Profile::Admin), Rule::Confirm);
    }

    #[test]
    fn a_table_with_a_missing_cell_does_not_load() {
        let broken = PROFILES_TOML.replacen("admin = \"confirm\"", "", 1);
        assert!(toml::from_str::<Table>(&broken).is_err());
        let extra = format!("{PROFILES_TOML}\n[surprise]\nread_only = \"allow\"\n");
        assert!(toml::from_str::<Table>(&extra).is_err());
    }

    #[test]
    fn every_named_bit_has_a_group() {
        let mut union = EffectSet::empty();
        for (bit, _, code) in GROUP_OF_BIT {
            assert!(!union.intersects(bit), "{bit:?} twice");
            union |= bit;
            assert_eq!(code.is_none(), bit == EffectSet::READ, "{bit:?}");
        }
        assert_eq!(union, EffectSet::all());
    }

    /// Fix round 1, I2: a new risk flag must be placed in `FLAG_RULES`.
    #[test]
    fn every_named_flag_has_a_rule() {
        let mut union = RiskFlags::empty();
        for (flag, _, _) in FLAG_RULES {
            assert_eq!(flag.bits().count_ones(), 1, "{flag:?}");
            assert!(!union.intersects(flag), "{flag:?} twice");
            union |= flag;
        }
        assert_eq!(union, RiskFlags::all());
    }

    /// A floor's gate class and its group agree (transaction control is the one `Tx`).
    #[test]
    fn kind_floors_agree_with_their_groups() {
        use StmtKind as K;
        for k in [
            K::Insert,
            K::Update,
            K::Delete,
            K::Merge,
            K::Drop,
            K::DropOwned,
            K::Truncate,
            K::CreateTable,
            K::CreateOther,
            K::Alter,
            K::Grant,
            K::Revoke,
            K::Call,
            K::Do,
            K::Execute,
            K::Begin,
            K::Commit,
            K::Rollback,
            K::Savepoint,
            K::Lock,
            K::Use,
            K::Listen,
            K::Prepare,
            K::Notify,
        ] {
            let KindRule::Floor(g, gate, code) = kind_rule(k) else {
                panic!("{k:?} has no floor");
            };
            if gate == GateClass::Tx {
                assert_eq!(g, Group::Session, "{k:?}");
            } else {
                assert_eq!(g.gate(), gate, "{k:?}");
            }
            assert_eq!(
                reason_gate_floor(&Reason::new(code)),
                gate,
                "{k:?}: its reason implies its floor"
            );
        }
    }

    fn set_report(d: Option<DialectId>, names: &[&str]) -> SafetyReport {
        let targets: Vec<(&str, Option<&str>)> = names.iter().map(|n| (*n, Some("x"))).collect();
        set_report_with(d, &targets)
    }

    /// A `SET` of `targets` (name, value), one `SessionState` reason per target.
    fn set_report_with(d: Option<DialectId>, targets: &[(&str, Option<&str>)]) -> SafetyReport {
        let names: Vec<&str> = targets.iter().map(|(n, _)| *n).collect();
        let s = StmtReport {
            range: TextRange::default(),
            kind: StmtKind::Set,
            effects: EffectSet::SESSION_STATE,
            flags: RiskFlags::empty(),
            class: CoarseClass::Session,
            touched: TouchedObjects::default(),
            reasons: names
                .iter()
                .map(|n| Reason {
                    code: ReasonCode::SessionState,
                    range: None,
                    arg: Some(Arc::from(*n)),
                })
                .collect(),
            destructive: Vec::new(),
            confidence: Confidence::Exact,
            set_targets: targets
                .iter()
                .map(|(n, v)| SetTarget {
                    name: Arc::from(*n),
                    value: v.map(Arc::from),
                })
                .collect(),
            // ## m4-t01c-core
            ddl: Vec::new(),
        };
        let items = vec![s];
        let (request_class, request_effects, flags) = merge(&items);
        let ctx = DialectCtx::new(DialectId::Postgres, None);
        SafetyReport {
            items,
            request_class,
            request_effects,
            flags,
            fingerprint: sql_split::hv_norm1("x", &ctx),
            backends: SmallVec::new(),
            pre_decision: None,
            classified_len: 1,
            classified_hash: classified_hash("x"),
            dialect: d,
        }
    }

    /// A fake list: PG `search_path` only, and only with a value whose `pg_catalog` (if any)
    /// comes first (the shape of P-M3b-T3-1); no value is refused.
    fn list(d: DialectId, n: &str, value: Option<&str>) -> bool {
        let Some(v) = value else {
            return false;
        };
        let first = v.split(',').next().unwrap_or_default().trim();
        d == DialectId::Postgres
            && n == "search_path"
            && (!v.contains("pg_catalog") || first == "pg_catalog")
    }

    /// Re-review R1-M3: the `SessionState` reasons and `set_targets` must name the same
    /// variables; a report where they disagree is not harmless, and the unseen name is refused.
    #[test]
    fn reasons_and_targets_must_name_the_same_variables() {
        let pg = Some(DialectId::Postgres);
        let ro = |r: &SafetyReport| decide_with(r, Profile::ReadOnly, Actor::Human, &list);
        // The consistent report is harmless.
        assert!(ro(&set_report_with(pg, &[("search_path", Some("public"))])).allows());
        // A reason names `role`, the target says `search_path = public`.
        let mut swapped = set_report_with(pg, &[("search_path", Some("public"))]);
        swapped.items[0].reasons[0].arg = Some(Arc::from("role"));
        let d = ro(&swapped);
        let Verdict::Deny(reasons) = d.verdict else {
            panic!("{d:?}")
        };
        assert!(
            reasons.iter().any(
                |r| r.code == ReasonCode::SetNotAllowlisted && r.arg.as_deref() == Some("role")
            )
        );
        // An extra reason naming another variable.
        let mut extra = set_report_with(pg, &[("search_path", Some("public"))]);
        extra.items[0].reasons.push(Reason {
            code: ReasonCode::SessionState,
            range: None,
            arg: Some(Arc::from("role")),
        });
        assert!(!ro(&extra).allows());
        // A target no reason names.
        let mut unnamed = set_report_with(pg, &[("search_path", Some("public"))]);
        unnamed.items[0].reasons.clear();
        assert!(!ro(&unnamed).allows());
        // A SessionState reason without an argument.
        let mut bare = set_report_with(pg, &[("search_path", Some("public"))]);
        bare.items[0].reasons[0].arg = None;
        assert!(!ro(&bare).allows());
        // The same name twice in the reasons but once in the targets.
        let mut twice = set_report_with(pg, &[("search_path", Some("public"))]);
        let again = twice.items[0].reasons[0].clone();
        twice.items[0].reasons.push(again);
        assert!(!ro(&twice).allows());
    }

    /// Re-review R1-M1: `class_floor` skips a class field's group when a group of the same
    /// coarse class was already applied. That is sound only while every group of coarse
    /// class `c` is at least as strict as `group_of_class(c)` in every profile.
    #[test]
    fn groups_of_one_coarse_class_are_at_least_as_strict_as_its_floor_group() {
        const GROUPS: [Group; 12] = [
            Group::Read,
            Group::ExplainAnalyzeRead,
            Group::Session,
            Group::Lock,
            Group::Write,
            Group::WriteUnbounded,
            Group::WriteTemp,
            Group::Ddl,
            Group::DdlDestructive,
            Group::Admin,
            Group::Routine,
            Group::Unknown,
        ];
        let t = table().expect("data/profiles.toml");
        for c in CoarseClass::ALL {
            if c == CoarseClass::Read {
                continue; // `class_floor` never applies a Read field.
            }
            let (floor, _) = group_of_class(c);
            assert_eq!(floor.coarse(), c, "{c:?}");
            for g in GROUPS.into_iter().filter(|g| g.coarse() == c) {
                for p in [
                    Profile::ReadOnly,
                    Profile::ReadOnlyPlusExplain,
                    Profile::ReadWriteDml,
                    Profile::Ddl,
                    Profile::Admin,
                ] {
                    assert!(
                        t.row(g).rule(p) >= t.row(floor).rule(p),
                        "{g:?} is looser than {floor:?} (the {c:?} floor) under {p:?}"
                    );
                }
            }
        }
    }

    /// The SET's value reaches the list (P-M3b-T3-1 plumbing; fix round 1).
    #[test]
    fn the_set_value_reaches_the_list() {
        let pg = Some(DialectId::Postgres);
        let ro = |targets: &[(&str, Option<&str>)]| {
            decide_with(
                &set_report_with(pg, targets),
                Profile::ReadOnly,
                Actor::Human,
                &list,
            )
        };
        assert!(ro(&[("search_path", Some("public"))]).allows());
        assert!(ro(&[("search_path", Some("pg_catalog, public"))]).allows());
        let d = ro(&[("search_path", Some("evil, pg_catalog"))]);
        assert!(!d.allows());
        let Verdict::Deny(reasons) = d.verdict else {
            panic!()
        };
        assert!(
            reasons
                .iter()
                .any(|r| r.code == ReasonCode::SetNotAllowlisted
                    && r.arg.as_deref() == Some("search_path"))
        );
        // No value (RESET, unreadable): fail-closed.
        assert!(!ro(&[("search_path", None)]).allows());
    }

    /// The harmless-SET path with a fake list (Task 3 fills the real one in parallel): a read
    /// on the read-only profiles for a human only (P-M3b-6; fix round 1, I3).
    #[test]
    fn a_harmless_set_is_a_read_on_read_only_for_a_human() {
        let pg = Some(DialectId::Postgres);
        let r = set_report(pg, &["search_path"]);
        for p in [Profile::ReadOnly, Profile::ReadOnlyPlusExplain] {
            let d = decide_with(&r, p, Actor::Human, &list);
            assert_eq!(d.verdict, Verdict::Allow, "{p:?}");
            assert_eq!(d.class, CoarseClass::Session);
            for a in [Actor::AiAgent, Actor::Automation] {
                assert!(
                    matches!(decide_with(&r, p, a, &list).verdict, Verdict::Deny(_)),
                    "{p:?} {a:?}"
                );
            }
        }
        // Elsewhere: the `session` row (alg/13), whatever the actor.
        for a in [Actor::Human, Actor::AiAgent, Actor::Automation] {
            for p in [Profile::ReadWriteDml, Profile::Ddl] {
                assert!(
                    matches!(decide_with(&r, p, a, &list).verdict, Verdict::Deny(_)),
                    "{p:?} {a:?}"
                );
            }
            assert!(
                matches!(
                    decide_with(&r, Profile::Admin, a, &list).verdict,
                    Verdict::Confirm(_)
                ),
                "{a:?}"
            );
        }
        // A target off the list, another dialect, or no dialect: refused on read-only, the
        // refused variable named.
        for (d, names, refused) in [
            (pg, &["search_path", "role"][..], &["role"][..]),
            (
                Some(DialectId::MySql),
                &["search_path"][..],
                &["search_path"][..],
            ),
            (None, &["search_path"][..], &["search_path"][..]),
        ] {
            let dec = decide_with(
                &set_report(d, names),
                Profile::ReadOnly,
                Actor::Human,
                &list,
            );
            let Verdict::Deny(reasons) = dec.verdict else {
                panic!("{d:?} {names:?}: {dec:?}");
            };
            let named: Vec<_> = reasons
                .iter()
                .filter(|r| r.code == ReasonCode::SetNotAllowlisted)
                .map(|r| r.arg.as_deref().unwrap())
                .collect();
            assert_eq!(named, refused, "{d:?} {names:?}");
        }
        // No target: refused as a session change, never a nameless SetNotAllowlisted.
        let Verdict::Deny(reasons) =
            decide_with(&set_report(pg, &[]), Profile::ReadOnly, Actor::Human, &list).verdict
        else {
            panic!()
        };
        assert!(reasons.iter().any(|r| r.code == ReasonCode::SessionState));
        assert!(
            reasons
                .iter()
                .all(|r| r.code != ReasonCode::SetNotAllowlisted || r.arg.is_some())
        );
        // A SET_VAR hint reaches the list by name, without a value (`None`): accepted only
        // where the list accepts a name alone; this fake list never does.
        let mut hinted = set_report(pg, &["search_path"]);
        hinted.items[0].kind = StmtKind::Select;
        hinted.items[0].effects = EffectSet::READ;
        hinted.items[0].class = CoarseClass::Read;
        hinted.items[0].set_targets.clear();
        hinted.items[0].reasons[0].code = ReasonCode::HintSetVar;
        let (c, e, f) = merge(&hinted.items);
        (hinted.request_class, hinted.request_effects, hinted.flags) = (c, e, f);
        assert!(!decide_with(&hinted, Profile::ReadOnly, Actor::Human, &list).allows());
        let name_only = |d: DialectId, n: &str, _: Option<&str>| {
            d == DialectId::Postgres && n == "statement_timeout"
        };
        hinted.items[0].reasons[0].arg = Some(Arc::from("statement_timeout"));
        assert!(decide_with(&hinted, Profile::ReadOnly, Actor::Human, &name_only).allows());
        hinted.items[0].reasons[0].arg = Some(Arc::from("transaction_read_only"));
        assert!(!decide_with(&hinted, Profile::ReadOnly, Actor::Human, &name_only).allows());
    }

    /// I1's invariant on the harmless path the integration tests cannot reach before Task 3:
    /// the gate calls a harmless SET `Session`, never `Read`.
    #[test]
    fn a_harmless_set_is_session_for_the_gate() {
        let r = set_report(Some(DialectId::Postgres), &["search_path"]);
        assert_eq!(crate::gate_class(&r.items[0]), GateClass::Session);
        assert!(decide_with(&r, Profile::ReadOnlyPlusExplain, Actor::Human, &list).allows());
    }
}
