//! The policy table (`alg/13` SQL-SAFE-04; the SAFE-07 model; Task 4): a report, a profile and
//! an actor give a verdict.
//!
//! [`decide`] fails closed. A pre-decided report, a report with no statement, or a policy
//! table that does not load is `Deny`. Otherwise every statement and the request-level fields
//! are judged, and the strictest rule wins (`Deny > Confirm > Allow`):
//!
//! - **Effects:** each effect bit selects a group of `data/profiles.toml` ([`GROUP_OF_BIT`]);
//!   an empty set or an unnamed bit is the `unknown` group, which denies in every built-in
//!   profile but `Admin` (Confirm, audit flag). A `SET` whose every target is on the
//!   dialect's harmless list ([`crate::set_allowed`], P-M3b-6) is a read; any other `SET`
//!   names `SetNotAllowlisted`.
//! - **Classes:** a statement's `class` field, the request's class and request-level effect
//!   bits the statements do not show each select their group too, so a producer cannot lower
//!   a decision by leaving a field behind.
//! - **Destructive patterns:** unsafe DML confirms under the writable profiles
//!   (`write_unbounded`), destructive DDL under `ddl` (`ddl_destructive`).
//! - **Flags and reasons:** doubt (lexing, splitting, parsing, limits), a lexical-mode change
//!   and a privilege change deny in every profile; unresolved functions, operators and casts,
//!   dynamic SQL and procedural code are `unknown`; an executable comment (D-058) or a
//!   denylisted function denies on the read-only profiles and for an AI agent; a `SET_VAR`
//!   hint denies unless its variable is harmless; `MAX_EXECUTION_TIME(0)` (or an unreadable
//!   limit) denies. Every other reason is evidence of its effect group.
//! - **Actors:** an AI agent is never allowed an executable comment, a denylisted function,
//!   `EXPLAIN ANALYZE` on a read-only profile, or an UPDATE/DELETE without a bounding WHERE
//!   (`alg/13` SAFE-04 step 5); `Automation` is decided like a human.

use std::sync::OnceLock;

use serde::Deserialize;
use sql_dialect::DialectId;
use sql_text::TextRange;

use crate::effects::{CoarseClass, EffectSet, RiskFlags, class_of, flags_are_named};
use crate::functions::set_allowed;
use crate::reason::{Reason, ReasonCode};
use crate::report::{SafetyReport, StmtKind, StmtReport};

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

/// The verdict for `r` under `p` for `a` (`alg/13` SQL-SAFE-04; see the module docs).
pub fn decide(r: &SafetyReport, p: Profile, a: Actor) -> Decision {
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
enum Group {
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
        Some(ReasonCode::PrivilegeChange),
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

/// What a reason code in a statement means to the policy.
enum ReasonRule {
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
    /// Nothing beyond the statement's effects (the destructive list is judged on its own).
    None,
}

/// The policy's reading of `code`: an exhaustive match, so a new code must be placed.
fn reason_rule(code: ReasonCode) -> ReasonRule {
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
        | R::EffectsUnknown => ReasonRule::Group(Group::Unknown),
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
        R::ExternalFile | R::ExternalNetwork | R::ProcessExec | R::AdminControl | R::ExternalIo => {
            ReasonRule::Group(Group::Admin)
        }
        R::WriteTemp => ReasonRule::Group(Group::WriteTemp),
        R::SchemaChange => ReasonRule::Group(Group::Ddl),
        R::RoutineCall => ReasonRule::Group(Group::Routine),
        R::Destructive => ReasonRule::None,
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

/// The decision being built: the strictest rule so far and the reasons for it.
struct Judge<'a> {
    table: &'a Table,
    profile: Profile,
    actor: Actor,
    deny: Vec<Reason>,
    confirm: Vec<Reason>,
    class: CoarseClass,
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

    fn group(&mut self, g: Group, reason: Reason) {
        if g == Group::Unknown {
            self.class = CoarseClass::Unknown;
        }
        let rule = self.table.row(g).rule(self.profile);
        self.push(rule, reason);
    }

    /// The groups of `effects`' bits; an empty set or an unnamed bit is `unknown`.
    /// `session_reason` replaces the `SESSION_STATE` reason (a `SET` names its targets).
    fn effects(
        &mut self,
        effects: EffectSet,
        range: Option<TextRange>,
        session: Option<&[Reason]>,
    ) {
        let at = |code| Reason {
            code,
            range,
            arg: None,
        };
        if effects.is_empty() {
            self.group(Group::Unknown, at(ReasonCode::EffectsUnknown));
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
                (_, _, Some(code)) => self.group(g, at(code)),
                (_, _, None) => self.group(g, at(ReasonCode::EffectsUnknown)),
            }
        }
        if named.bits() != effects.bits() {
            self.group(Group::Unknown, at(ReasonCode::EffectsUnknown));
        }
    }

    /// `EXPLAIN ANALYZE` over `effects` (the inner statement's).
    fn explain_analyze(&mut self, effects: EffectSet, range: Option<TextRange>) {
        let reason = Reason {
            code: ReasonCode::ExplainAnalyze,
            range,
            arg: None,
        };
        if self.ai() && self.profile.is_read_only() {
            self.deny(reason.clone());
        }
        if class_of(effects) == CoarseClass::Read {
            self.group(Group::ExplainAnalyzeRead, reason);
        }
        // Otherwise the inner statement's effects decide (judged with the statement).
    }

    fn flags(&mut self, flags: RiskFlags, effects: EffectSet, range: Option<TextRange>) {
        let at = |code| Reason {
            code,
            range,
            arg: None,
        };
        if !flags_are_named(flags) {
            self.group(Group::Unknown, at(ReasonCode::EffectsUnknown));
        }
        if flags.contains(RiskFlags::DYNAMIC_SQL) {
            self.group(Group::Unknown, at(ReasonCode::DynamicSql));
        }
        if flags.contains(RiskFlags::LEX_MODE_CHANGE) {
            self.deny(at(ReasonCode::LexModeChange));
        }
        if flags.contains(RiskFlags::EXEC_COMMENT) && (self.profile.is_read_only() || self.ai()) {
            self.deny(at(ReasonCode::ExecutableComment));
        }
        if flags.contains(RiskFlags::EXPLAIN_ANALYZE) {
            self.explain_analyze(effects, range);
        }
    }

    fn reason(
        &mut self,
        r: &Reason,
        s: &StmtReport,
        dialect: Option<DialectId>,
        allowed: &dyn Fn(DialectId, &str) -> bool,
        harmless_set: bool,
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
            ReasonRule::Group(Group::Session)
                if harmless_set && r.code == ReasonCode::SessionState => {}
            ReasonRule::Group(g) => self.group(g, r.clone()),
            ReasonRule::HintSetVar => {
                let ok = match (dialect, r.arg.as_deref()) {
                    (Some(d), Some(var)) => allowed(d, var),
                    _ => false,
                };
                if !ok {
                    self.deny(r.clone());
                }
            }
            ReasonRule::HintExecTime => {
                let ok = r
                    .arg
                    .as_deref()
                    .and_then(|n| n.trim().parse::<u64>().ok())
                    .is_some_and(|n| n > 0);
                if !ok {
                    self.deny(r.clone());
                }
            }
            ReasonRule::ExplainAnalyze => self.explain_analyze(s.effects, r.range),
            ReasonRule::None => {}
        }
    }

    fn statement(
        &mut self,
        s: &StmtReport,
        dialect: Option<DialectId>,
        allowed: &dyn Fn(DialectId, &str) -> bool,
    ) {
        let range = Some(s.range);
        let effects_class = class_of(s.effects);
        self.class = self.class.max(s.class).max(effects_class);
        // A class field more critical than the effects: that class's group.
        if s.class > effects_class {
            let (g, code) = group_of_class(s.class);
            self.group(
                g,
                Reason {
                    code,
                    range,
                    arg: None,
                },
            );
        }
        let harmless = harmless_set(s, dialect, allowed);
        if harmless {
            self.group(Group::Read, Reason::new(ReasonCode::SessionState));
        } else if s.kind == StmtKind::Set {
            let refused = set_refusals(s, dialect, allowed);
            self.effects(s.effects, range, Some(&refused));
        } else {
            self.effects(s.effects, range, None);
        }
        self.flags(s.flags, s.effects, range);
        for r in &s.reasons {
            self.reason(r, s, dialect, allowed, harmless);
        }
        for d in &s.destructive {
            let reason = Reason {
                code: ReasonCode::Destructive,
                range: Some(d.range),
                arg: None,
            };
            if d.kind.is_unsafe_dml() {
                self.class = self.class.max(CoarseClass::Write);
                if self.ai() && d.kind.denied_for_ai() {
                    self.deny(reason.clone());
                }
                self.group(Group::WriteUnbounded, reason);
            } else {
                self.class = self.class.max(CoarseClass::Ddl);
                self.group(Group::DdlDestructive, reason);
            }
        }
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

/// A `SET` the dialect's harmless list covers: only `SESSION_STATE`, no flag, no destructive
/// pattern, a known dialect, and at least one target, every reason naming a target
/// (`SessionState` with the variable as its argument) that `allowed` accepts.
fn harmless_set(
    s: &StmtReport,
    dialect: Option<DialectId>,
    allowed: &dyn Fn(DialectId, &str) -> bool,
) -> bool {
    let Some(d) = dialect else {
        return false;
    };
    s.kind == StmtKind::Set
        && s.effects == EffectSet::SESSION_STATE
        && s.flags.is_empty()
        && s.destructive.is_empty()
        && s.class <= CoarseClass::Session
        && !s.reasons.is_empty()
        && s.reasons.iter().all(|r| {
            r.code == ReasonCode::SessionState && r.arg.as_deref().is_some_and(|n| allowed(d, n))
        })
}

/// Why a `SET` is not harmless: `SetNotAllowlisted` for each target the list refuses (all of
/// them when the dialect is unknown), or once without an argument when it names none.
fn set_refusals(
    s: &StmtReport,
    dialect: Option<DialectId>,
    allowed: &dyn Fn(DialectId, &str) -> bool,
) -> Vec<Reason> {
    let mut out: Vec<Reason> = s
        .reasons
        .iter()
        .filter(|r| r.code == ReasonCode::SessionState)
        .filter_map(|r| r.arg.as_ref().map(|n| (r, n)))
        .filter(|(_, n)| !dialect.is_some_and(|d| allowed(d, n)))
        .map(|(r, n)| Reason {
            code: ReasonCode::SetNotAllowlisted,
            range: r.range.or(Some(s.range)),
            arg: Some(n.clone()),
        })
        .collect();
    if out.is_empty() {
        out.push(Reason {
            code: ReasonCode::SetNotAllowlisted,
            range: Some(s.range),
            arg: None,
        });
    }
    out
}

/// [`decide`] with the harmless-`SET` list injected (the unit tests use a fake one).
fn decide_with(
    r: &SafetyReport,
    p: Profile,
    a: Actor,
    allowed: &dyn Fn(DialectId, &str) -> bool,
) -> Decision {
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
    let request_class = r.request_class.max(class_of(r.request_effects));
    if request_class > j.class {
        let (g, code) = group_of_class(request_class);
        j.group(g, Reason::new(code));
    }
    j.class = j.class.max(request_class);
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

    fn set_report(d: Option<DialectId>, names: &[&str]) -> SafetyReport {
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

    /// The harmless-SET path with a fake list (Task 3 fills the real one in parallel).
    #[test]
    fn a_harmless_set_is_a_read_on_every_profile() {
        let list = |d: DialectId, n: &str| d == DialectId::Postgres && n == "search_path";
        let pg = Some(DialectId::Postgres);
        for p in [
            Profile::ReadOnly,
            Profile::ReadOnlyPlusExplain,
            Profile::ReadWriteDml,
            Profile::Ddl,
            Profile::Admin,
        ] {
            for a in [Actor::Human, Actor::AiAgent, Actor::Automation] {
                let d = decide_with(&set_report(pg, &["search_path"]), p, a, &list);
                assert_eq!(d.verdict, Verdict::Allow, "{p:?} {a:?}");
                assert_eq!(d.class, CoarseClass::Session);
            }
        }
        // A target off the list, another dialect, or no dialect: refused on read-only.
        for (d, names) in [
            (pg, &["search_path", "role"][..]),
            (Some(DialectId::MySql), &["search_path"][..]),
            (None, &["search_path"][..]),
            (pg, &[][..]),
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
            assert!(
                reasons
                    .iter()
                    .any(|r| r.code == ReasonCode::SetNotAllowlisted),
                "{reasons:?}"
            );
        }
        // Only the refused target is named.
        let Verdict::Deny(reasons) = decide_with(
            &set_report(pg, &["search_path", "role"]),
            Profile::ReadOnly,
            Actor::Human,
            &list,
        )
        .verdict
        else {
            panic!()
        };
        let named: Vec<_> = reasons
            .iter()
            .filter(|r| r.code == ReasonCode::SetNotAllowlisted)
            .map(|r| r.arg.as_deref())
            .collect();
        assert_eq!(named, [Some("role")]);
        // A harmless SET_VAR hint is accepted, any other refused.
        let mut hinted = set_report(pg, &["search_path"]);
        hinted.items[0].kind = StmtKind::Select;
        hinted.items[0].effects = EffectSet::READ;
        hinted.items[0].class = CoarseClass::Read;
        hinted.items[0].reasons[0].code = ReasonCode::HintSetVar;
        let (c, e, f) = merge(&hinted.items);
        (hinted.request_class, hinted.request_effects, hinted.flags) = (c, e, f);
        assert_eq!(
            decide_with(&hinted, Profile::ReadOnly, Actor::Human, &list).verdict,
            Verdict::Allow
        );
        hinted.items[0].reasons[0].arg = Some(Arc::from("transaction_read_only"));
        assert!(matches!(
            decide_with(&hinted, Profile::ReadOnly, Actor::Human, &list).verdict,
            Verdict::Deny(_)
        ));
    }
}
