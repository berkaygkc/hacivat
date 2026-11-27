//! M3b Task 4: the policy table (`alg/13` SQL-SAFE-04 :317-329), the SQL-SAFE-01 class cases,
//! and the rulings on top of them (D-058 executable comments, D-059 temp writes, P-M3b-6
//! harmless `SET`s, `Unknown` denied outside `Admin`).

mod common;

use common::{
    arb_parts, explain_analyze_read, reason, report, request, request_in, set_of, set_with, single,
    stmt_full, unknown, value_for,
};
use proptest::prelude::*;
use serde::Deserialize;
use sql_dialect::DialectId;
use sql_safety::effects::class_of;
use sql_safety::{
    Actor, CoarseClass, Decision, DestructiveKind, EffectSet, GateClass, Profile, Reason,
    ReasonCode, RiskFlags, SafetyReport, StmtKind, Verdict, decide, gate_class, set_allowed,
};

const PROFILES: [Profile; 5] = [
    Profile::ReadOnly,
    Profile::ReadOnlyPlusExplain,
    Profile::ReadWriteDml,
    Profile::Ddl,
    Profile::Admin,
];

const ACTORS: [Actor; 3] = [Actor::Human, Actor::AiAgent, Actor::Automation];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum V {
    A,
    C,
    D,
}

fn v(d: &Decision) -> V {
    match d.verdict {
        Verdict::Allow => V::A,
        Verdict::Confirm(_) => V::C,
        Verdict::Deny(_) => V::D,
    }
}

fn reasons(d: &Decision) -> &[Reason] {
    match &d.verdict {
        Verdict::Allow => &[],
        Verdict::Confirm(r) | Verdict::Deny(r) => r,
    }
}

fn codes(d: &Decision) -> Vec<ReasonCode> {
    reasons(d).iter().map(|r| r.code).collect()
}

fn human(r: &SafetyReport) -> [V; 5] {
    PROFILES.map(|p| v(&decide(r, p, Actor::Human)))
}

// ---------------------------------------------------------------------------------------------
// SQL-SAFE-01

#[derive(Deserialize)]
struct Safe01 {
    case: Vec<Safe01Case>,
}

#[derive(Deserialize)]
struct Safe01Case {
    effects: Vec<String>,
    class: String,
    gate: String,
}

#[test]
fn safe01_cases() {
    let data: Safe01 = toml::from_str(include_str!("data/safe01.toml")).unwrap();
    assert!(data.case.len() >= 20);
    for c in data.case {
        let mut e = EffectSet::empty();
        for n in &c.effects {
            e |= EffectSet::from_name(n).unwrap_or_else(|| panic!("flag {n}"));
        }
        let class = CoarseClass::ALL
            .into_iter()
            .find(|k| format!("{k:?}") == c.class)
            .unwrap_or_else(|| panic!("class {}", c.class));
        let gate = GateClass::ALL
            .into_iter()
            .find(|k| format!("{k:?}") == c.gate)
            .unwrap_or_else(|| panic!("gate {}", c.gate));
        assert_eq!(class_of(e), class, "{:?}", c.effects);
        assert_eq!(gate_class(&report(e, &[])), gate, "{:?}", c.effects);
        // The request's class is the statement's.
        assert_eq!(request(vec![report(e, &[])]).request_class, class);
    }
}

// ---------------------------------------------------------------------------------------------
// SQL-SAFE-04

/// Every (profile, effect group) cell of `alg/13` :317-329 for a human. Columns: read_only,
/// read_only_plus_explain, read_write_dml, ddl, admin. One deliberate tightening: unbounded
/// UPDATE/DELETE also confirms under `ddl` (the spec's `ddl` column is a superset of
/// `read_write_dml` and says only "Allow"); see ADR 0005 `m3b-t04`.
#[test]
fn safe04_profile_table() {
    use DestructiveKind as K;
    use V::{A, C, D};
    let session = [D, D, D, D, C];
    let lock = [D, D, A, A, A];
    let write = [D, D, A, A, A];
    let admin = [D, D, D, D, C];
    let unknown_row = [D, D, D, D, C];
    let rows: Vec<(&str, SafetyReport, [V; 5])> = vec![
        ("read", single(EffectSet::READ, &[]), [A, A, A, A, A]),
        (
            "explain analyze of a read",
            explain_analyze_read(),
            [D, A, A, A, A],
        ),
        ("txn", single(EffectSet::TXN_CONTROL, &[]), session),
        (
            "session state",
            single(EffectSet::SESSION_STATE, &[]),
            session,
        ),
        ("notify", single(EffectSet::NOTIFY, &[]), session),
        ("prepare", single(EffectSet::PREPARE, &[]), session),
        (
            "row lock",
            single(EffectSet::READ | EffectSet::LOCK_ROW, &[]),
            lock,
        ),
        ("table lock", single(EffectSet::LOCK_TABLE, &[]), lock),
        (
            "advisory lock",
            single(EffectSet::READ | EffectSet::ADVISORY_LOCK, &[]),
            lock,
        ),
        ("write", single(EffectSet::WRITE_DATA, &[]), write),
        (
            "sequence",
            single(EffectSet::READ | EffectSet::SEQUENCE, &[]),
            write,
        ),
        (
            "xid",
            single(EffectSet::READ | EffectSet::XID_CONSUME, &[]),
            write,
        ),
        (
            "unbounded delete",
            single(EffectSet::WRITE_DATA, &[K::NoWhere]),
            [D, D, C, C, A],
        ),
        (
            "always-true update",
            single(EffectSet::WRITE_DATA, &[K::AlwaysTrue]),
            [D, D, C, C, A],
        ),
        (
            "join without on",
            single(EffectSet::WRITE_DATA, &[K::JoinWithoutOn]),
            [D, D, C, C, A],
        ),
        (
            "write temp",
            single(EffectSet::WRITE_TEMP, &[]),
            [D, D, A, A, A],
        ),
        ("ddl", single(EffectSet::DDL, &[]), [D, D, D, A, A]),
        (
            "temp ddl",
            single(EffectSet::DDL_TEMP, &[]),
            [D, D, D, A, A],
        ),
        (
            "drop",
            single(EffectSet::DDL, &[K::Drop { cascade: false }]),
            [D, D, D, C, A],
        ),
        (
            "drop cascade",
            single(EffectSet::DDL, &[K::Drop { cascade: true }]),
            [D, D, D, C, A],
        ),
        (
            "truncate",
            single(EffectSet::WRITE_DATA | EffectSet::DDL, &[K::Truncate]),
            [D, D, D, C, A],
        ),
        ("privilege", single(EffectSet::PRIVILEGE, &[]), admin),
        ("admin control", single(EffectSet::ADMIN, &[]), admin),
        (
            "external io",
            single(EffectSet::READ | EffectSet::EXTERNAL_IO, &[]),
            admin,
        ),
        (
            "process exec",
            single(EffectSet::READ | EffectSet::PROCESS_EXEC, &[]),
            admin,
        ),
        (
            "routine call",
            single(EffectSet::ROUTINE_CALL, &[]),
            unknown_row,
        ),
        (
            "unknown",
            single(EffectSet::READ | EffectSet::UNKNOWN, &[]),
            unknown_row,
        ),
        (
            "empty effects",
            single(EffectSet::empty(), &[]),
            unknown_row,
        ),
        (
            "unnamed bit",
            single(EffectSet::READ | EffectSet::from_bits_retain(1 << 24), &[]),
            unknown_row,
        ),
    ];
    for (name, r, want) in rows {
        assert_eq!(human(&r), want, "{name}");
        for p in PROFILES {
            let d = decide(&r, p, Actor::Human);
            if v(&d) != V::A {
                assert!(
                    !reasons(&d).is_empty(),
                    "{name} {p:?}: a refusal names its reasons"
                );
            }
        }
    }
}

/// `alg/13` SQL-SAFE-04 "Test cases".
#[test]
fn safe04_spec_test_cases() {
    // SELECT 1 under read_only → Allow.
    assert_eq!(
        v(&decide(
            &single(EffectSet::READ, &[]),
            Profile::ReadOnly,
            Actor::Human
        )),
        V::A
    );
    // EXPLAIN ANALYZE SELECT under read_only → Deny, under read_only_plus_explain → Allow.
    let ea = explain_analyze_read();
    let d = decide(&ea, Profile::ReadOnly, Actor::Human);
    assert_eq!(v(&d), V::D);
    assert!(codes(&d).contains(&ReasonCode::ExplainAnalyze));
    assert_eq!(
        v(&decide(&ea, Profile::ReadOnlyPlusExplain, Actor::Human)),
        V::A
    );
    // DELETE FROM t under read_write_dml: human → Confirm, AI → Deny.
    let del = request(vec![stmt_full(
        StmtKind::Delete,
        EffectSet::WRITE_DATA,
        RiskFlags::empty(),
        &[DestructiveKind::NoWhere],
        Vec::new(),
    )]);
    let d = decide(&del, Profile::ReadWriteDml, Actor::Human);
    assert_eq!(v(&d), V::C);
    assert!(codes(&d).contains(&ReasonCode::Destructive));
    assert_eq!(
        v(&decide(&del, Profile::ReadWriteDml, Actor::AiAgent)),
        V::D
    );
    // DROP TABLE t under ddl → Confirm.
    let drop = request(vec![stmt_full(
        StmtKind::Drop,
        EffectSet::DDL,
        RiskFlags::empty(),
        &[DestructiveKind::Drop { cascade: false }],
        Vec::new(),
    )]);
    assert_eq!(v(&decide(&drop, Profile::Ddl, Actor::Human)), V::C);
    // SET ROLE admin under admin (no role-switch policy) → Deny.
    let role = request(vec![stmt_full(
        StmtKind::Set,
        EffectSet::SESSION_STATE | EffectSet::PRIVILEGE,
        RiskFlags::empty(),
        &[],
        vec![
            reason(ReasonCode::PrivilegeChange, Some("role")),
            reason(ReasonCode::SessionState, Some("role")),
        ],
    )]);
    let d = decide(&role, Profile::Admin, Actor::Human);
    assert_eq!(v(&d), V::D);
    assert!(codes(&d).contains(&ReasonCode::PrivilegeChange));
    // SELECT /*+ MAX_EXECUTION_TIME(0) */ 1 (MySQL) → Deny.
    let hint = request_in(
        Some(DialectId::MySql),
        vec![stmt_full(
            StmtKind::Select,
            EffectSet::READ,
            RiskFlags::empty(),
            &[],
            vec![reason(ReasonCode::HintExecTime, Some("0"))],
        )],
    );
    for p in PROFILES {
        assert_eq!(v(&decide(&hint, p, Actor::Human)), V::D, "{p:?}");
    }
    // A positive limit is a harmless hint.
    let hint_ok = request_in(
        Some(DialectId::MySql),
        vec![stmt_full(
            StmtKind::Select,
            EffectSet::READ,
            RiskFlags::empty(),
            &[],
            vec![reason(ReasonCode::HintExecTime, Some("1000"))],
        )],
    );
    assert_eq!(v(&decide(&hint_ok, Profile::ReadOnly, Actor::Human)), V::A);
}

#[test]
fn unknown_denies_in_every_profile_but_admin() {
    for p in [
        Profile::ReadOnly,
        Profile::ReadOnlyPlusExplain,
        Profile::ReadWriteDml,
        Profile::Ddl,
    ] {
        for a in ACTORS {
            let d = decide(&unknown(), p, a);
            assert!(matches!(d.verdict, Verdict::Deny(_)), "{p:?} {a:?}");
            assert_eq!(d.class, CoarseClass::Unknown);
        }
    }
    // Admin: Confirm (audit flag), never Allow.
    for a in ACTORS {
        assert!(
            matches!(
                decide(&unknown(), Profile::Admin, a).verdict,
                Verdict::Confirm(_)
            ),
            "{a:?}"
        );
    }
}

#[test]
fn pre_decided_and_empty_requests_deny_everywhere() {
    let mut pre = single(EffectSet::READ, &[]);
    pre.pre_decision = Some(vec![Reason::new(ReasonCode::MultiStatement)]);
    let empty = request(Vec::new());
    assert_eq!(empty.request_class, CoarseClass::Unknown);
    for p in PROFILES {
        for a in ACTORS {
            let d = decide(&pre, p, a);
            assert_eq!(v(&d), V::D, "{p:?} {a:?}");
            assert_eq!(codes(&d), [ReasonCode::MultiStatement]);
            assert_eq!(d.class, CoarseClass::Unknown);
            let d = decide(&empty, p, a);
            assert_eq!(v(&d), V::D, "{p:?} {a:?}");
            assert_eq!(codes(&d), [ReasonCode::NoStatement]);
        }
    }
    // Even an empty pre-decision list is a denial, with a reason.
    let mut pre_empty = single(EffectSet::READ, &[]);
    pre_empty.pre_decision = Some(Vec::new());
    let d = decide(&pre_empty, Profile::Admin, Actor::Human);
    assert_eq!(v(&d), V::D);
    assert!(!reasons(&d).is_empty());
}

#[test]
fn doubt_reasons_in_a_statement_deny_everywhere() {
    for code in [
        ReasonCode::NotImplemented,
        ReasonCode::LexError,
        ReasonCode::ClientCommand,
        ReasonCode::TemplateVariable,
        ReasonCode::NoStatement,
        ReasonCode::MultiStatement,
        ReasonCode::AmbiguousBoundary,
        ReasonCode::ParseError,
        ReasonCode::ParserDisagreement,
        ReasonCode::TooComplex,
        ReasonCode::SyntaxNewerThanAgent,
        ReasonCode::UnsupportedDialect,
        ReasonCode::LexModeChange,
        ReasonCode::PrivilegeChange,
    ] {
        let r = request(vec![stmt_full(
            StmtKind::Select,
            EffectSet::READ,
            RiskFlags::empty(),
            &[],
            vec![reason(code, None)],
        )]);
        for p in PROFILES {
            assert_eq!(v(&decide(&r, p, Actor::Human)), V::D, "{code:?} {p:?}");
        }
    }
    // Unresolved functions, operators, casts, dynamic SQL and procedural code are Unknown:
    // denied outside Admin, confirmed there.
    for code in [
        ReasonCode::FnUnknown,
        ReasonCode::OperatorUnknown,
        ReasonCode::CastUnknown,
        ReasonCode::DynamicSql,
        ReasonCode::ProceduralCode,
    ] {
        let r = request(vec![stmt_full(
            StmtKind::Select,
            EffectSet::READ,
            RiskFlags::empty(),
            &[],
            vec![reason(code, Some("x"))],
        )]);
        assert_eq!(human(&r), [V::D, V::D, V::D, V::D, V::C], "{code:?}");
        assert_eq!(
            decide(&r, Profile::ReadOnly, Actor::Human).class,
            CoarseClass::Unknown
        );
    }
}

#[test]
fn lex_mode_change_denies_everywhere() {
    let r = request_in(
        Some(DialectId::MySql),
        vec![stmt_full(
            StmtKind::Set,
            EffectSet::SESSION_STATE,
            RiskFlags::LEX_MODE_CHANGE,
            &[],
            vec![reason(ReasonCode::SessionState, Some("sql_mode"))],
        )],
    );
    for p in PROFILES {
        for a in ACTORS {
            let d = decide(&r, p, a);
            assert_eq!(v(&d), V::D, "{p:?} {a:?}");
            assert!(codes(&d).contains(&ReasonCode::LexModeChange));
        }
    }
}

#[test]
fn executable_comment_denies_on_read_only_and_for_ai() {
    for flagged in [true, false] {
        // Either the flag or the reason is enough.
        let (flags, why) = if flagged {
            (RiskFlags::EXEC_COMMENT, Vec::new())
        } else {
            (
                RiskFlags::empty(),
                vec![reason(ReasonCode::ExecutableComment, None)],
            )
        };
        let r = request_in(
            Some(DialectId::MySql),
            vec![stmt_full(
                StmtKind::Select,
                EffectSet::READ,
                flags,
                &[],
                why,
            )],
        );
        for p in [Profile::ReadOnly, Profile::ReadOnlyPlusExplain] {
            for a in ACTORS {
                let d = decide(&r, p, a);
                assert_eq!(v(&d), V::D, "{p:?} {a:?}");
                assert!(codes(&d).contains(&ReasonCode::ExecutableComment));
            }
        }
        for p in PROFILES {
            let d = decide(&r, p, Actor::AiAgent);
            assert_eq!(v(&d), V::D, "AI {p:?}");
            assert!(codes(&d).contains(&ReasonCode::ExecutableComment));
        }
        // A human on a writable profile is decided by the effects.
        for p in [Profile::ReadWriteDml, Profile::Ddl, Profile::Admin] {
            assert_eq!(v(&decide(&r, p, Actor::Human)), V::A, "{p:?}");
        }
    }
}

#[test]
fn temp_writes_deny_on_read_only() {
    for e in [
        EffectSet::WRITE_TEMP,
        EffectSet::READ | EffectSet::WRITE_TEMP,
    ] {
        let r = single(e, &[]);
        for p in [Profile::ReadOnly, Profile::ReadOnlyPlusExplain] {
            for a in ACTORS {
                let d = decide(&r, p, a);
                assert_eq!(v(&d), V::D, "{p:?} {a:?}");
                assert!(codes(&d).contains(&ReasonCode::WriteTemp));
                assert_eq!(d.class, CoarseClass::Write);
            }
        }
        assert_eq!(v(&decide(&r, Profile::ReadWriteDml, Actor::Human)), V::A);
    }
}

#[test]
fn explain_analyze_of_a_read_is_allowed_for_humans_only() {
    assert!(matches!(
        decide(
            &explain_analyze_read(),
            Profile::ReadOnlyPlusExplain,
            Actor::Human
        )
        .verdict,
        Verdict::Allow
    ));
    assert!(matches!(
        decide(&explain_analyze_read(), Profile::ReadOnly, Actor::AiAgent).verdict,
        Verdict::Deny(_)
    ));
    // An AI agent on read_only_plus_explain is allowed: that profile is the MCP path to
    // `explain_query analyze=true` (`alg/13` :331; fix round 1, I4).
    assert!(matches!(
        decide(
            &explain_analyze_read(),
            Profile::ReadOnlyPlusExplain,
            Actor::AiAgent
        )
        .verdict,
        Verdict::Allow
    ));
    // EXPLAIN ANALYZE of a write: the inner class decides, so read-only profiles deny.
    let ea_write = request(vec![stmt_full(
        StmtKind::Explain { analyze: true },
        EffectSet::READ | EffectSet::WRITE_DATA,
        RiskFlags::EXPLAIN_ANALYZE,
        &[],
        Vec::new(),
    )]);
    for p in [Profile::ReadOnly, Profile::ReadOnlyPlusExplain] {
        assert_eq!(v(&decide(&ea_write, p, Actor::Human)), V::D, "{p:?}");
    }
    assert_eq!(
        v(&decide(&ea_write, Profile::ReadWriteDml, Actor::Human)),
        V::A
    );
}

#[test]
fn read_only_denies_set_unless_allowlisted() {
    let pg = Some(DialectId::Postgres);
    for name in [
        "search_path",
        "STATEMENT_TIMEOUT",
        "work_mem",
        "role",
        "session authorization",
        "transaction_read_only",
        "default_transaction_read_only",
        "idle_in_transaction_session_timeout",
        "standard_conforming_strings",
    ] {
        let allowed = set_allowed(DialectId::Postgres, name, Some(value_for(name)));
        for p in [Profile::ReadOnly, Profile::ReadOnlyPlusExplain] {
            let d = decide(&set_of(pg, &[name]), p, Actor::Human);
            if allowed {
                assert_eq!(v(&d), V::A, "{name} {p:?}");
            } else {
                assert_eq!(v(&d), V::D, "{name} {p:?}");
                assert!(
                    reasons(&d)
                        .iter()
                        .any(|r| r.code == ReasonCode::SetNotAllowlisted
                            && r.arg.as_deref() == Some(name)),
                    "{name}: {:?}",
                    reasons(&d)
                );
            }
        }
    }
    // Never allowed on PG, whatever the list says (P-M3b-6).
    for name in [
        "role",
        "session authorization",
        "transaction_read_only",
        "default_transaction_read_only",
        "idle_in_transaction_session_timeout",
    ] {
        assert_eq!(
            v(&decide(
                &set_of(pg, &[name]),
                Profile::ReadOnly,
                Actor::Human
            )),
            V::D,
            "{name}"
        );
    }
    // MySQL's sql_mode is never harmless.
    assert_eq!(
        v(&decide(
            &set_of(Some(DialectId::MySql), &["sql_mode"]),
            Profile::ReadOnly,
            Actor::Human
        )),
        V::D
    );
    // One forbidden target in the list denies the whole SET.
    assert_eq!(
        v(&decide(
            &set_of(pg, &["search_path", "role"]),
            Profile::ReadOnly,
            Actor::Human
        )),
        V::D
    );
    // No named target (RESET ALL, a non-literal name), or no dialect: denied. With no target
    // the refusal is a session change; every SetNotAllowlisted names its variable (M8).
    let d = decide(&set_of(pg, &[]), Profile::ReadOnly, Actor::Human);
    assert_eq!(v(&d), V::D);
    assert!(codes(&d).contains(&ReasonCode::SessionState));
    assert!(
        reasons(&d)
            .iter()
            .all(|r| r.code != ReasonCode::SetNotAllowlisted || r.arg.is_some())
    );
    assert_eq!(
        v(&decide(
            &set_of(None, &["search_path"]),
            Profile::ReadOnly,
            Actor::Human
        )),
        V::D
    );
    // A SET that also does something else is not a harmless SET.
    let mixed = request(vec![stmt_full(
        StmtKind::Set,
        EffectSet::SESSION_STATE | EffectSet::READ,
        RiskFlags::empty(),
        &[],
        vec![reason(ReasonCode::SessionState, Some("search_path"))],
    )]);
    assert_eq!(v(&decide(&mixed, Profile::ReadOnly, Actor::Human)), V::D);
    // SESSION_STATE that is not a SET (USE, LISTEN, DISCARD) is never allowlisted.
    let listen = request(vec![stmt_full(
        StmtKind::Listen,
        EffectSet::SESSION_STATE,
        RiskFlags::empty(),
        &[],
        vec![reason(ReasonCode::SessionState, Some("search_path"))],
    )]);
    assert_eq!(v(&decide(&listen, Profile::ReadOnly, Actor::Human)), V::D);
    // A backend's own SetNotAllowlisted always denies on read-only.
    let flagged = request(vec![stmt_full(
        StmtKind::Set,
        EffectSet::SESSION_STATE,
        RiskFlags::empty(),
        &[],
        vec![reason(ReasonCode::SetNotAllowlisted, Some("search_path"))],
    )]);
    assert_eq!(v(&decide(&flagged, Profile::ReadOnly, Actor::Human)), V::D);
}

#[test]
fn hint_set_var_denies_unless_allowlisted() {
    let r = request_in(
        Some(DialectId::MySql),
        vec![stmt_full(
            StmtKind::Select,
            EffectSet::READ,
            RiskFlags::empty(),
            &[],
            vec![reason(
                ReasonCode::HintSetVar,
                Some("transaction_read_only"),
            )],
        )],
    );
    for p in PROFILES {
        let d = decide(&r, p, Actor::Human);
        assert_eq!(v(&d), V::D, "{p:?}");
        assert!(codes(&d).contains(&ReasonCode::HintSetVar));
    }
}

#[test]
fn denylisted_functions_deny_on_read_only_and_for_ai() {
    let r = request(vec![stmt_full(
        StmtKind::Select,
        EffectSet::READ | EffectSet::SEQUENCE,
        RiskFlags::empty(),
        &[],
        vec![reason(ReasonCode::FnDenylisted, Some("nextval"))],
    )]);
    assert_eq!(human(&r), [V::D, V::D, V::A, V::A, V::A]);
    for p in PROFILES {
        assert_eq!(v(&decide(&r, p, Actor::AiAgent)), V::D, "{p:?}");
    }
    // A denylisted function on a bare READ (a backend that mapped no effect) still denies on
    // read-only.
    let bare = request(vec![stmt_full(
        StmtKind::Select,
        EffectSet::READ,
        RiskFlags::empty(),
        &[],
        vec![reason(ReasonCode::FnDenylisted, Some("pg_sleep"))],
    )]);
    assert_eq!(v(&decide(&bare, Profile::ReadOnly, Actor::Human)), V::D);
}

#[test]
fn every_statement_and_the_request_fields_count() {
    // A write after a read in one request.
    let r = request(vec![
        report(EffectSet::READ, &[]),
        report(EffectSet::WRITE_DATA, &[]),
    ]);
    let d = decide(&r, Profile::ReadOnly, Actor::Human);
    assert_eq!(v(&d), V::D);
    assert_eq!(d.class, CoarseClass::Write);
    // Request-level effects the items do not show still count (fail closed).
    let mut r = single(EffectSet::READ, &[]);
    r.request_effects |= EffectSet::DDL;
    assert_eq!(v(&decide(&r, Profile::ReadOnly, Actor::Human)), V::D);
    // So does a more critical request class or statement class.
    let mut r = single(EffectSet::READ, &[]);
    r.request_class = CoarseClass::Unknown;
    assert_eq!(human(&r), [V::D, V::D, V::D, V::D, V::C]);
    let mut r = single(EffectSet::READ, &[]);
    r.items[0].class = CoarseClass::Admin;
    assert_eq!(human(&r), [V::D, V::D, V::D, V::D, V::C]);
    // Request-level flags count.
    let mut r = single(EffectSet::READ, &[]);
    r.flags |= RiskFlags::EXEC_COMMENT;
    assert_eq!(v(&decide(&r, Profile::ReadOnly, Actor::Human)), V::D);
}

#[test]
fn automation_is_decided_like_a_human() {
    for r in [
        single(EffectSet::READ, &[]),
        single(EffectSet::WRITE_DATA, &[DestructiveKind::NoWhere]),
        single(EffectSet::DDL, &[DestructiveKind::Truncate]),
        unknown(),
    ] {
        for p in PROFILES {
            assert_eq!(
                v(&decide(&r, p, Actor::Automation)),
                v(&decide(&r, p, Actor::Human)),
                "{p:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Fix round 1

/// I2: `VOLATILE_FN` is `Unknown` (`alg/13` S-04): denied outside Admin, confirmed there.
#[test]
fn volatile_functions_are_unknown() {
    let r = request(vec![stmt_full(
        StmtKind::Select,
        EffectSet::READ,
        RiskFlags::VOLATILE_FN,
        &[],
        Vec::new(),
    )]);
    for a in ACTORS {
        assert_eq!(
            PROFILES.map(|p| v(&decide(&r, p, a))),
            [V::D, V::D, V::D, V::D, V::C],
            "{a:?}"
        );
    }
    let d = decide(&r, Profile::ReadOnly, Actor::AiAgent);
    assert!(codes(&d).contains(&ReasonCode::VolatileFunction));
    assert_eq!(d.class, CoarseClass::Unknown);
    assert_eq!(gate_class(&r.items[0]), GateClass::Unknown);
}

/// I5 (H6): the kind is evidence: a READ-only report of a DELETE, DROP, GRANT, CALL, BEGIN,
/// SET … is refused on read-only and judged as its kind elsewhere.
#[test]
fn kinds_are_policy_evidence() {
    use StmtKind as K;
    use V::{A, C, D};
    for (k, want) in [
        (K::Insert, [D, D, A, A, A]),
        (K::Update, [D, D, A, A, A]),
        (K::Delete, [D, D, A, A, A]),
        (K::Merge, [D, D, A, A, A]),
        (K::Drop, [D, D, D, C, A]),
        (K::Truncate, [D, D, D, C, A]),
        (K::CreateTable, [D, D, D, A, A]),
        (K::Alter, [D, D, D, A, A]),
        (K::Grant, [D, D, D, D, C]),
        (K::Revoke, [D, D, D, D, C]),
        (K::Call, [D, D, D, D, C]),
        (K::Do, [D, D, D, D, C]),
        (K::Execute, [D, D, D, D, C]),
        (K::Begin, [D, D, D, D, C]),
        (K::Commit, [D, D, D, D, C]),
        (K::Rollback, [D, D, D, D, C]),
        (K::Savepoint, [D, D, D, D, C]),
        (K::Lock, [D, D, A, A, A]),
        (K::Use, [D, D, D, D, C]),
        (K::Listen, [D, D, D, D, C]),
        (K::Prepare, [D, D, D, D, C]),
        (K::Notify, [D, D, D, D, C]),
        (K::Set, [D, D, D, D, C]),
        (K::Explain { analyze: true }, [D, A, A, A, A]),
        (K::Select, [A, A, A, A, A]),
        (K::Copy, [A, A, A, A, A]),
        (K::Pragma, [A, A, A, A, A]),
    ] {
        let r = request(vec![stmt_full(
            k,
            EffectSet::READ,
            RiskFlags::empty(),
            &[],
            Vec::new(),
        )]);
        assert_eq!(human(&r), want, "{k:?}");
    }
    // A SET without its SESSION_STATE bit still goes through the harmless list: the refused
    // target is named.
    for name in ["role", "search_path"] {
        let mut set = set_of(Some(DialectId::Postgres), &[name]);
        set.items[0].effects = EffectSet::READ;
        let d = decide(&set, Profile::ReadOnly, Actor::Human);
        assert_eq!(v(&d), V::D, "{name}");
        if name == "role" {
            assert!(reasons(&d).iter().any(
                |r| r.code == ReasonCode::SetNotAllowlisted && r.arg.as_deref() == Some("role")
            ));
        }
    }
}

/// I3: a SET is never harmless outside the read-only profiles or for an AI agent; there it is
/// `alg/13`'s Session row (the real list is Task 3's; the harmless path itself is a unit test
/// in `policy.rs` with a fake list).
#[test]
fn a_set_off_the_read_only_profiles_is_the_session_row() {
    let r = set_of(Some(DialectId::Postgres), &["search_path"]);
    for a in ACTORS {
        assert_eq!(v(&decide(&r, Profile::ReadWriteDml, a)), V::D, "{a:?}");
        assert_eq!(v(&decide(&r, Profile::Ddl, a)), V::D, "{a:?}");
        assert_eq!(v(&decide(&r, Profile::Admin, a)), V::C, "{a:?}");
    }
    for a in [Actor::AiAgent, Actor::Automation] {
        for p in [Profile::ReadOnly, Profile::ReadOnlyPlusExplain] {
            assert_eq!(v(&decide(&r, p, a)), V::D, "{p:?} {a:?}");
        }
    }
}

/// P-M3b-T3-1: `SET search_path` is harmless only when `pg_catalog` is absent or first; the
/// value reaches Task 3's `set_allowed(d, name, value)` through `SetTarget`.
#[test]
fn search_path_value_reaches_the_list() {
    let pg = Some(DialectId::Postgres);
    let ro = |value| {
        decide(
            &set_with(pg, &[("search_path", value)]),
            Profile::ReadOnly,
            Actor::Human,
        )
    };
    assert!(ro(Some("public")).allows());
    assert!(ro(Some("pg_catalog, public")).allows());
    assert!(!ro(Some("evil, pg_catalog")).allows());
    assert!(!ro(None).allows());
}

/// M2: a GRANT's PRIVILEGE bit confirms under Admin; a role switch denies everywhere, and the
/// two reasons are distinct codes, so a GRANT's decision reasons fed back stay confirmable.
#[test]
fn grant_and_role_switch_are_distinct() {
    let grant = request(vec![stmt_full(
        StmtKind::Grant,
        EffectSet::PRIVILEGE,
        RiskFlags::empty(),
        &[],
        Vec::new(),
    )]);
    let d = decide(&grant, Profile::Admin, Actor::Human);
    assert_eq!(v(&d), V::C);
    assert_eq!(codes(&d), [ReasonCode::PrivilegeGrant]);
    // Its own reasons back in a report: still Confirm.
    let again = request(vec![stmt_full(
        StmtKind::Grant,
        EffectSet::PRIVILEGE,
        RiskFlags::empty(),
        &[],
        reasons(&d).to_vec(),
    )]);
    assert_eq!(v(&decide(&again, Profile::Admin, Actor::Human)), V::C);
    let role = request(vec![stmt_full(
        StmtKind::Set,
        EffectSet::SESSION_STATE,
        RiskFlags::empty(),
        &[],
        vec![reason(ReasonCode::PrivilegeChange, Some("role"))],
    )]);
    for p in PROFILES {
        assert_eq!(v(&decide(&role, p, Actor::Human)), V::D, "{p:?}");
    }
}

/// M3 (H5): a bare `Destructive` reason is destructive-DDL evidence.
#[test]
fn a_bare_destructive_reason_counts() {
    let r = request(vec![stmt_full(
        StmtKind::Other,
        EffectSet::READ,
        RiskFlags::empty(),
        &[],
        vec![reason(ReasonCode::Destructive, None)],
    )]);
    assert_eq!(human(&r), [V::D, V::D, V::D, V::C, V::A]);
    assert_eq!(gate_class(&r.items[0]), GateClass::DdlDestructive);
}

/// M1: `Decision.class` follows reason evidence.
#[test]
fn the_decision_class_follows_reason_evidence() {
    for (code, class) in [
        (ReasonCode::ExternalFile, CoarseClass::Admin),
        (ReasonCode::ProcessExec, CoarseClass::Admin),
        (ReasonCode::WriteData, CoarseClass::Write),
        (ReasonCode::LockingClause, CoarseClass::Write),
        (ReasonCode::SchemaChange, CoarseClass::Ddl),
        (ReasonCode::TxnControl, CoarseClass::Session),
        (ReasonCode::FnUnknown, CoarseClass::Unknown),
    ] {
        let r = request(vec![stmt_full(
            StmtKind::Select,
            EffectSet::READ,
            RiskFlags::empty(),
            &[],
            vec![reason(code, Some("x"))],
        )]);
        let d = decide(&r, Profile::ReadOnly, Actor::Human);
        assert_eq!(v(&d), V::D, "{code:?}");
        assert_eq!(d.class, class, "{code:?}");
    }
}

/// M5 (H7): an empty statement among reads makes the request Unknown, its effects included.
#[test]
fn an_empty_statement_is_unknown_in_the_request() {
    let r = request(vec![
        report(EffectSet::READ, &[]),
        report(EffectSet::empty(), &[]),
    ]);
    assert_eq!(r.request_class, CoarseClass::Unknown);
    assert!(r.request_effects.contains(EffectSet::UNKNOWN));
    assert_eq!(human(&r), [V::D, V::D, V::D, V::D, V::C]);
}

fn rank(x: V) -> u8 {
    match x {
        V::A => 0,
        V::C => 1,
        V::D => 2,
    }
}

fn arb_profile() -> impl Strategy<Value = Profile> {
    prop::sample::select(PROFILES.to_vec())
}

fn arb_actor() -> impl Strategy<Value = Actor> {
    prop::sample::select(ACTORS.to_vec())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(20_000))]

    /// Fix round 1, I6: adding a bit, flag, reason or destructive kind to a statement never
    /// loosens a decision or lowers its class, in any profile, for any actor.
    #[test]
    fn decide_is_monotone(a in arb_parts(), b in arb_parts(), p in arb_profile(), actor in arb_actor()) {
        // From a non-empty effect set: the empty set is `unknown` because nothing was proved.
        let a = common::non_empty(a);
        let base = request(vec![a.stmt()]);
        let more = request(vec![a.grown(&b).stmt()]);
        let (d0, d1) = (decide(&base, p, actor), decide(&more, p, actor));
        prop_assert!(rank(v(&d1)) >= rank(v(&d0)), "{a:?} + {b:?}: {d0:?} -> {d1:?}");
        prop_assert!(d1.class >= d0.class, "{a:?} + {b:?}");
    }

    /// The strictest statement wins, in either order, and merging never lowers a class.
    #[test]
    fn merge_takes_the_strictest(a in arb_parts(), b in arb_parts(), p in arb_profile(), actor in arb_actor()) {
        let (s, t) = (a.stmt(), b.stmt());
        let one = decide(&request(vec![s.clone()]), p, actor);
        let two = decide(&request(vec![t.clone()]), p, actor);
        let ab = decide(&request(vec![s.clone(), t.clone()]), p, actor);
        let ba = decide(&request(vec![t.clone(), s.clone()]), p, actor);
        let strictest = rank(v(&one)).max(rank(v(&two)));
        prop_assert_eq!(rank(v(&ab)), strictest, "{:?} ; {:?}", a, b);
        prop_assert_eq!(rank(v(&ba)), strictest, "{:?} ; {:?}", b, a);
        prop_assert!(ab.class >= one.class.max(two.class));
        let (c, e, _) = sql_safety::report::merge(&[s.clone(), t.clone()]);
        prop_assert_eq!(c, sql_safety::report::merge(&[s]).0.max(sql_safety::report::merge(&[t]).0));
        prop_assert!(e.contains(a.effects & EffectSet::all()));
    }

    /// Read-only allows nothing but reads; `Unknown` (bits, class field, or a doubt) is never
    /// allowed anywhere.
    #[test]
    fn read_only_allows_only_reads(p in arb_parts(), actor in arb_actor()) {
        let s = p.stmt();
        let r = request(vec![s.clone()]);
        for prof in [Profile::ReadOnly, Profile::ReadOnlyPlusExplain] {
            let d = decide(&r, prof, actor);
            if d.allows() {
                prop_assert_eq!(s.effects, EffectSet::READ, "{:?} {:?}", p, prof);
                prop_assert!(s.destructive.is_empty());
                prop_assert!(s.class == CoarseClass::Read);
                prop_assert!(gate_class(&s) == GateClass::Read);
            }
        }
        if class_of(s.effects) == CoarseClass::Unknown || s.class == CoarseClass::Unknown {
            for prof in PROFILES {
                prop_assert!(!decide(&r, prof, actor).allows(), "{p:?} {prof:?}");
            }
        }
    }
}
