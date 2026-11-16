//! The PostgreSQL backend (`alg/13` SQL-SAFE-02 for PostgreSQL; M3b Task 9): libpg_query's
//! tree through `sql_oracle_pg::tree`, walked by the generated visitor
//! (`visit::pg_generated`, R-M3-2) with the statement table `data/pg/statements.toml`.
//!
//! **Combine rule** ([`analyse`]): libpg_query is the authority.
//! 1. The whole text is parsed once. A parse error (or the guard's refusal, or a protobuf
//!    decode-depth failure) makes every item `Unknown` with `ParseError`.
//! 2. libpg_query's statements must be our items: the same count, in order, and each item
//!    exactly its raw statement's first-to-last significant token. Otherwise every item is
//!    `Unknown` with
//!    `ParserDisagreement` (and `MultiStatement` when one unit was expected and the parser
//!    found several).
//! 3. Token-boundary agreement: our lexer's significant tokens (with the session's
//!    `LexModes` and the server version's profile) must have exactly the PostgreSQL scanner's
//!    boundaries; the one equivalence is a `:name` placeholder, one token to us and `:` plus a
//!    name to the scanner. With `standard_conforming_strings` off, a backslash in a plain
//!    string is a disagreement too (libpg_query parses with it on). Disagreement adds
//!    `ParserDisagreement` to every item.
//! 4. A server newer than the grammar (PostgreSQL 18) adds `SyntaxNewerThanAgent`.
//! 5. A statement that may change how later statements read (a search path that may not keep
//!    `pg_catalog` first, `set_config` or `UPDATE pg_settings`, a lexing-mode or role change,
//!    `RESET ALL`, `DISCARD ALL`, `pg_reload_conf`, `LOAD`, a routine call (`CALL`, `DO`), DDL
//!    (it may create a function, operator, cast or view a later name resolves to), or anything
//!    `UNKNOWN`, which may run code that does any of these) makes every later item `Unknown`
//!    (`EffectsUnknown`): they were read against the context before it.
//!
//! A limit (the guard's size and nesting caps, prost's decode depth of 100 messages) is
//! `TooComplex`, a syntax error `ParseError`. A leading byte-order mark is parsed as spaces
//! (our lexer reads it as whitespace; a server that does not strip it rejects the text).
//!
//! **Per statement** ([`Classifier`]): the statement row's kind and base effects, then a walk of
//! every message under it: nested statements add their rows' effects; `INTO` is DDL and a
//! write (temp: `DDL_TEMP`, `WRITE_TEMP`); locking clauses lock rows; a data change inside a
//! CTE is `DataModifyingCte`; every function call, operator and cast is judged by Task 3's data
//! (`lookup_fn`, `operator_known`, `cast_known`); `COPY`, `EXPLAIN`, `SET`, transactions and
//! `DECLARE` have their own rules; destructive patterns are found on statements that run now
//! (not inside routine bodies, rules, prepared statements or a plain `EXPLAIN`).
//!
//! **Fail closed.** Every message kind has a role ([`role`], an exhaustive match: a new
//! libpg_query node does not compile until it is placed). Kinds that only an analysed tree
//! holds (`FuncExpr`, `OpExpr`, `Query`, …) are `Unhandled`: met in a raw tree, they make the
//! statement `Unknown`. Every `Unknown` outcome carries `EffectSet::UNKNOWN` or a doubt reason
//! (the producer contract of Task 4's review).
//!
//! **Names.** The raw tree holds names as the server resolved them (unquoted ASCII folded, no
//! locale casing). A function name that was quoted in the text is never taken as a builtin (it
//! is `Unknown` unless denied: the deny list matches the bare name in any form). A qualified
//! user function, an unreviewed builtin and anything else Task 3's data does not know is
//! `Unknown` (`FnUnknown`), unless the profile's allowlist (`ClassifyCtx.fn_allowlist`, exact
//! names after folding) or a catalog (`ClassifyCtx.catalog`, M4) says otherwise; a volatile
//! catalog function sets `VOLATILE_FN` (`alg/13` S-04).

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use serde::Deserialize;
use smallvec::SmallVec;
use sql_dialect::{DialectId, lex_profile};
use sql_ident::{Ident, QualifiedName, parse_qualified};
use sql_lexer::{LexState, Lexed, TokenKind};
use sql_oracle_pg::tree::{self, ScanKind, pb};
use sql_text::{TextRange, TextSize};

use crate::backend::Backend;
use crate::classify::{ClassifyCtx, Expect};
use crate::destructive::{DestructiveKind, DestructiveReason};
use crate::effects::{EffectSet, RiskFlags, class_of};
use crate::functions::{DenyGroup, FnClass, FnLookup, cast_known, lookup_fn, operator_known};
use crate::reason::{Reason, ReasonCode};
use crate::report::{
    BackendVersion, Confidence, ObjectKind, ObjectRef, SetTarget, StmtKind, StmtReport,
    TouchedObjects,
};
use crate::visit::pg_generated::{self as g, MsgKind, PgRef, Visitor, Walk};

/// The backend as `SafetyReport.backends` records it.
pub(crate) const BACKEND: BackendVersion = BackendVersion {
    name: "libpg_query",
    version: "18.1.0",
};

/// The newest PostgreSQL major the vendored grammar knows.
const GRAMMAR_MAJOR: u16 = 18;

/// libpg_query's `CURSOR_OPT_HOLD` (`DECLARE … WITH HOLD`).
const CURSOR_OPT_HOLD: i32 = 0x0020;

/// GUCs that change how the server lexes later statements (`alg/13` SAFE-02 step 7).
const LEX_VARIABLES: [&str; 3] = [
    "standard_conforming_strings",
    "client_encoding",
    "backslash_quote",
];

/// GUCs that switch the session's role (`PrivilegeChange`).
const ROLE_VARIABLES: [&str; 2] = ["role", "session_authorization"];

/// Relations that show other sessions' activity or statement text (information disclosure,
/// T10's RR-I2 ruling): reading one is `Unknown`, so a read-only session denies it. Defence in
/// depth only: `pg_read_all_stats`/superuser is the real control. The functions behind them
/// (`pg_stat_get_activity`, `pg_stat_get_backend_activity`, `pg_stat_statements(…)`) are not
/// reviewed builtins in Task 3's data, so they are `Unknown` already. `pg_locks` alone shows
/// pids and lock modes, not text, and is left a read; joined with `pg_stat_activity` the join
/// is `Unknown` through the activity view.
const DISCLOSING_RELATIONS: [&str; 2] = ["pg_stat_activity", "pg_stat_statements"];

/// The PostgreSQL backend.
pub(crate) struct PgBackend;

// ## m3b-t14-orchestrator (named exception, T10 C1 carry): the orchestrator's lexing is the
// token-agreement check's.
impl Backend for PgBackend {
    fn analyse(
        &self,
        sql: &str,
        items: &[TextRange],
        ctx: &ClassifyCtx,
        lexed: Option<&Lexed>,
    ) -> Vec<StmtReport> {
        analyse_lexed(sql, items, ctx, lexed)
    }
}

// ---------------------------------------------------------------------------------------------
// The statement table

/// One row of `data/pg/statements.toml`.
#[derive(Debug, Clone)]
pub(crate) struct Row {
    kind: StmtKind,
    effects: EffectSet,
    flags: RiskFlags,
    reasons: Vec<ReasonCode>,
    deferred: bool,
}

/// The table, by statement message.
pub(crate) type Table = BTreeMap<MsgKind, Row>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    rows: Vec<RowFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RowFile {
    node: String,
    kind: String,
    effects: Vec<String>,
    #[serde(default)]
    flags: Vec<String>,
    #[serde(default)]
    reasons: Vec<String>,
    #[serde(default)]
    deferred: bool,
}

const STATEMENTS: &str = include_str!("../../data/pg/statements.toml");

/// The statement table; `None` when the data does not load (every statement is then `Unknown`,
/// and `the_statement_table_loads` keeps that from shipping).
pub(crate) fn table() -> Option<&'static Table> {
    static TABLE: OnceLock<Option<Table>> = OnceLock::new();
    TABLE.get_or_init(|| load(STATEMENTS).ok()).as_ref()
}

/// `kind` named in the table.
fn stmt_kind(name: &str) -> Option<StmtKind> {
    use StmtKind as K;
    Some(match name {
        "Select" => K::Select,
        "Values" => K::Values,
        "Table" => K::Table,
        "Insert" => K::Insert,
        "Update" => K::Update,
        "Delete" => K::Delete,
        "Merge" => K::Merge,
        "Copy" => K::Copy,
        "Call" => K::Call,
        "Do" => K::Do,
        "Explain" => K::Explain { analyze: false },
        "CreateTable" => K::CreateTable,
        "CreateOther" => K::CreateOther,
        "Alter" => K::Alter,
        "Drop" => K::Drop,
        "Truncate" => K::Truncate,
        "Grant" => K::Grant,
        "Revoke" => K::Revoke,
        "Set" => K::Set,
        "Show" => K::Show,
        "Begin" => K::Begin,
        "Commit" => K::Commit,
        "Rollback" => K::Rollback,
        "Savepoint" => K::Savepoint,
        "Lock" => K::Lock,
        "Prepare" => K::Prepare,
        "Execute" => K::Execute,
        "Listen" => K::Listen,
        "Notify" => K::Notify,
        "Other" => K::Other,
        _ => return None,
    })
}

/// `text` as the table; an unknown name, a duplicate row or a non-statement node is an error.
pub(crate) fn load(text: &str) -> Result<Table, String> {
    let file: File = toml::from_str(text).map_err(|e| e.to_string())?;
    let mut out = Table::new();
    for r in file.rows {
        let node = MsgKind::from_name(&r.node).ok_or_else(|| format!("node {}", r.node))?;
        if role(node) != Role::Stmt {
            return Err(format!("{} is not a statement", r.node));
        }
        let kind = stmt_kind(&r.kind).ok_or_else(|| format!("kind {}", r.kind))?;
        let mut effects = EffectSet::empty();
        for e in &r.effects {
            effects |= EffectSet::from_name(e).ok_or_else(|| format!("effect {e}"))?;
        }
        let mut flags = RiskFlags::empty();
        for f in &r.flags {
            flags |= RiskFlags::from_name(f).ok_or_else(|| format!("flag {f}"))?;
        }
        let reasons = r
            .reasons
            .iter()
            .map(|n| {
                ReasonCode::ALL
                    .into_iter()
                    .find(|c| format!("{c:?}") == *n)
                    .ok_or_else(|| format!("reason {n}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let row = Row {
            kind,
            effects,
            flags,
            reasons,
            deferred: r.deferred,
        };
        if out.insert(node, row).is_some() {
            return Err(format!("{} twice", r.node));
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Message roles

/// What the classifier does with a message kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    /// A statement: its row in the table, plus its own rules.
    Stmt,
    /// A message with a rule of its own (functions, operators, casts, `INTO`, locks, …).
    Rule,
    /// Structure or a builtin expression with no effect of its own; its children are judged.
    Neutral,
    /// Never part of a raw parse tree (an analysed-tree node, a scanner or summary message):
    /// met in a tree, it makes the statement `Unknown`.
    Unhandled,
}

/// The role of every message kind: an exhaustive match, so a new libpg_query message must be
/// placed before the crate compiles.
pub(crate) fn role(k: MsgKind) -> Role {
    use MsgKind as M;
    match k {
        M::InsertStmt
        | M::DeleteStmt
        | M::UpdateStmt
        | M::MergeStmt
        | M::SelectStmt
        | M::SetOperationStmt
        | M::ReturnStmt
        | M::PlAssignStmt
        | M::CreateSchemaStmt
        | M::AlterTableStmt
        | M::ReplicaIdentityStmt
        | M::AlterCollationStmt
        | M::AlterDomainStmt
        | M::GrantStmt
        | M::GrantRoleStmt
        | M::AlterDefaultPrivilegesStmt
        | M::CopyStmt
        | M::VariableSetStmt
        | M::VariableShowStmt
        | M::CreateStmt
        | M::CreateTableSpaceStmt
        | M::DropTableSpaceStmt
        | M::AlterTableSpaceOptionsStmt
        | M::AlterTableMoveAllStmt
        | M::CreateExtensionStmt
        | M::AlterExtensionStmt
        | M::AlterExtensionContentsStmt
        | M::CreateFdwStmt
        | M::AlterFdwStmt
        | M::CreateForeignServerStmt
        | M::AlterForeignServerStmt
        | M::CreateForeignTableStmt
        | M::CreateUserMappingStmt
        | M::AlterUserMappingStmt
        | M::DropUserMappingStmt
        | M::ImportForeignSchemaStmt
        | M::CreatePolicyStmt
        | M::AlterPolicyStmt
        | M::CreateAmStmt
        | M::CreateTrigStmt
        | M::CreateEventTrigStmt
        | M::AlterEventTrigStmt
        | M::CreatePLangStmt
        | M::CreateRoleStmt
        | M::AlterRoleStmt
        | M::AlterRoleSetStmt
        | M::DropRoleStmt
        | M::CreateSeqStmt
        | M::AlterSeqStmt
        | M::DefineStmt
        | M::CreateDomainStmt
        | M::CreateOpClassStmt
        | M::CreateOpFamilyStmt
        | M::AlterOpFamilyStmt
        | M::DropStmt
        | M::TruncateStmt
        | M::CommentStmt
        | M::SecLabelStmt
        | M::DeclareCursorStmt
        | M::ClosePortalStmt
        | M::FetchStmt
        | M::IndexStmt
        | M::CreateStatsStmt
        | M::AlterStatsStmt
        | M::CreateFunctionStmt
        | M::AlterFunctionStmt
        | M::DoStmt
        | M::CallStmt
        | M::RenameStmt
        | M::AlterObjectDependsStmt
        | M::AlterObjectSchemaStmt
        | M::AlterOwnerStmt
        | M::AlterOperatorStmt
        | M::AlterTypeStmt
        | M::RuleStmt
        | M::NotifyStmt
        | M::ListenStmt
        | M::UnlistenStmt
        | M::TransactionStmt
        | M::CompositeTypeStmt
        | M::CreateEnumStmt
        | M::CreateRangeStmt
        | M::AlterEnumStmt
        | M::ViewStmt
        | M::LoadStmt
        | M::CreatedbStmt
        | M::AlterDatabaseStmt
        | M::AlterDatabaseRefreshCollStmt
        | M::AlterDatabaseSetStmt
        | M::DropdbStmt
        | M::AlterSystemStmt
        | M::ClusterStmt
        | M::VacuumStmt
        | M::ExplainStmt
        | M::CreateTableAsStmt
        | M::RefreshMatViewStmt
        | M::CheckPointStmt
        | M::DiscardStmt
        | M::LockStmt
        | M::ConstraintsSetStmt
        | M::ReindexStmt
        | M::CreateConversionStmt
        | M::CreateCastStmt
        | M::CreateTransformStmt
        | M::PrepareStmt
        | M::ExecuteStmt
        | M::DeallocateStmt
        | M::DropOwnedStmt
        | M::ReassignOwnedStmt
        | M::AlterTsDictionaryStmt
        | M::AlterTsConfigurationStmt
        | M::CreatePublicationStmt
        | M::AlterPublicationStmt
        | M::CreateSubscriptionStmt
        | M::AlterSubscriptionStmt
        | M::DropSubscriptionStmt => Role::Stmt,

        M::FuncCall
        | M::AExpr
        | M::TypeCast
        | M::IntoClause
        | M::LockingClause
        | M::RangeVar
        | M::RangeTableSample
        | M::SubLink
        | M::SortBy
        | M::CommonTableExpr
        | M::AlterTableCmd
        // Fix round 1: coercions to a named type (I1) and implicit `=` lookups (M2).
        | M::JsonOutput
        | M::JsonTableColumn
        | M::RangeTableFuncCol
        | M::XmlSerialize
        | M::RangeFunction
        | M::ColumnDef
        | M::CaseExpr
        | M::JoinExpr => Role::Rule,

        // Raw-tree structure and builtin expressions.
        M::ParseResult
        | M::RawStmt
        | M::Node
        | M::Integer
        | M::Float
        | M::Boolean
        | M::String
        | M::BitString
        | M::List
        | M::OidList
        | M::IntList
        | M::AConst
        | M::Alias
        | M::GroupingFunc
        | M::MergeSupportFunc
        | M::NamedArgExpr
        | M::BoolExpr
        | M::CaseWhen
        | M::RowExpr
        | M::CoalesceExpr
        | M::MinMaxExpr
        | M::SqlValueFunction
        | M::XmlExpr
        | M::JsonFormat
        | M::JsonReturning
        | M::JsonValueExpr
        | M::JsonIsPredicate
        | M::JsonBehavior
        | M::NullTest
        | M::BooleanTest
        | M::SetToDefault
        | M::CurrentOfExpr
        | M::TypeName
        | M::ColumnRef
        | M::ParamRef
        | M::CollateClause
        | M::RoleSpec
        | M::AStar
        | M::AIndices
        | M::AIndirection
        | M::AArrayExpr
        | M::ResTarget
        | M::MultiAssignRef
        | M::WindowDef
        | M::RangeSubselect
        | M::RangeTableFunc
        | M::TableLikeClause
        | M::IndexElem
        | M::DefElem
        | M::PartitionElem
        | M::PartitionSpec
        | M::PartitionBoundSpec
        | M::PartitionRangeDatum
        | M::PartitionCmd
        | M::GroupingSet
        | M::WithClause
        | M::InferClause
        | M::OnConflictClause
        | M::CteSearchClause
        | M::CteCycleClause
        | M::MergeWhenClause
        | M::ReturningOption
        | M::ReturningClause
        | M::TriggerTransition
        | M::JsonArgument
        | M::JsonFuncExpr
        | M::JsonTablePathSpec
        | M::JsonTable
        | M::JsonKeyValue
        | M::JsonParseExpr
        | M::JsonScalarExpr
        | M::JsonSerializeExpr
        | M::JsonObjectConstructor
        | M::JsonArrayConstructor
        | M::JsonArrayQueryConstructor
        | M::JsonAggConstructor
        | M::JsonObjectAgg
        | M::JsonArrayAgg
        | M::AtAlterConstraint
        | M::ObjectWithArgs
        | M::AccessPriv
        | M::Constraint
        | M::CreateOpClassItem
        | M::StatsElem
        | M::FunctionParameter
        | M::VacuumRelation
        | M::PublicationTable
        | M::PublicationObjSpec => Role::Neutral,

        // Analysed-tree, scanner and summary messages: never in a raw parse tree. Their
        // functions and operators are OIDs the classifier cannot name.
        M::ScanResult
        | M::ScanToken
        | M::SummaryResult
        | M::TableFunc
        | M::Var
        | M::Param
        | M::Aggref
        | M::WindowFunc
        | M::WindowFuncRunCondition
        | M::SubscriptingRef
        | M::FuncExpr
        | M::OpExpr
        | M::DistinctExpr
        | M::NullIfExpr
        | M::ScalarArrayOpExpr
        | M::SubPlan
        | M::AlternativeSubPlan
        | M::FieldSelect
        | M::FieldStore
        | M::RelabelType
        | M::CoerceViaIo
        | M::ArrayCoerceExpr
        | M::ConvertRowtypeExpr
        | M::CollateExpr
        | M::CaseTestExpr
        | M::ArrayExpr
        | M::RowCompareExpr
        | M::JsonConstructorExpr
        | M::JsonExpr
        | M::JsonTablePath
        | M::JsonTablePathScan
        | M::JsonTableSiblingJoin
        | M::MergeAction
        | M::CoerceToDomain
        | M::CoerceToDomainValue
        | M::NextValueExpr
        | M::InferenceElem
        | M::ReturningExpr
        | M::TargetEntry
        | M::RangeTblRef
        | M::FromExpr
        | M::OnConflictExpr
        | M::Query
        | M::RangeTblEntry
        | M::RtePermissionInfo
        | M::RangeTblFunction
        | M::TableSampleClause
        | M::WithCheckOption
        | M::SortGroupClause
        | M::WindowClause
        | M::RowMarkClause
        | M::InlineCodeBlock
        | M::CallContext => Role::Unhandled,
    }
}

// ---------------------------------------------------------------------------------------------
// The combine rule

/// The reports of `items` (our splitter's statement ranges in `sql`), by the combine rule of
/// the module docs.
pub(crate) fn analyse(sql: &str, items: &[TextRange], ctx: &ClassifyCtx) -> Vec<StmtReport> {
    analyse_lexed(sql, items, ctx, None)
}

// ## m3b-t14-orchestrator (named exception, T10 C1 carry)
/// [`analyse`] with `sql`'s lexing when the caller has it (`Backend::analyse`'s contract).
pub(crate) fn analyse_lexed(
    sql: &str,
    items: &[TextRange],
    ctx: &ClassifyCtx,
    lexed: Option<&Lexed>,
) -> Vec<StmtReport> {
    if items.is_empty() {
        return Vec::new();
    }
    let failed = |codes: &[ReasonCode]| -> Vec<StmtReport> {
        items.iter().map(|r| unknown_report(*r, codes)).collect()
    };
    // A leading UTF-8 byte-order mark is whitespace to our lexer but a syntax error to
    // libpg_query: parse the text with the mark as three spaces (same offsets). A server that
    // does not strip it rejects the text, so nothing runs that the parse did not see.
    let blanked;
    let text = match sql.strip_prefix('\u{feff}') {
        Some(rest) => {
            blanked = format!("   {rest}");
            blanked.as_str()
        }
        None => sql,
    };
    let parsed = match tree::parse_tree(text) {
        Ok(t) => t,
        Err(e) if tree::over_a_limit(&e) => return failed(&[ReasonCode::TooComplex]),
        Err(_) => return failed(&[ReasonCode::ParseError]),
    };
    let toks = tree::scan_tokens(text).ok();
    let agree = toks
        .as_deref()
        .is_some_and(|t| statements_agree(&parsed, items, text, t));
    if !agree {
        let mut codes = vec![ReasonCode::ParserDisagreement];
        if ctx.expect == Expect::SingleStatement && parsed.stmts.len() > 1 {
            codes.push(ReasonCode::MultiStatement);
        }
        return failed(&codes);
    }
    let Some(table) = table() else {
        return failed(&[ReasonCode::EffectsUnknown]);
    };
    let mut doubts = Vec::new();
    if !toks
        .as_deref()
        .is_some_and(|t| tokens_agree(sql, ctx, t, lexed))
    {
        doubts.push(ReasonCode::ParserDisagreement);
    }
    if ctx.dialect.version.is_some_and(|v| v.major > GRAMMAR_MAJOR) {
        doubts.push(ReasonCode::SyntaxNewerThanAgent);
    }
    let toks = toks.unwrap_or_default();
    // An item that may change what later items mean (the search_path, the lexing modes, the
    // role, code the classifier cannot see into, a routine it calls, or DDL that may create a
    // function, operator, cast, type or view a later name resolves to) makes every later item
    // `Unknown`: they were classified against the context as it was before it (fix round 1,
    // I2a; round 2, R1-I1, R1-M2).
    let taints =
        EffectSet::UNKNOWN | EffectSet::ROUTINE_CALL | EffectSet::DDL | EffectSet::DDL_TEMP;
    let mut stale = false;
    items
        .iter()
        .zip(&parsed.stmts)
        .map(|(range, raw)| {
            let (mut r, changes_context) = statement(text, *range, raw, ctx, table, &toks);
            for code in &doubts {
                doubt(&mut r, *code);
            }
            if stale {
                doubt(&mut r, ReasonCode::EffectsUnknown);
            }
            stale |= changes_context || r.effects.intersects(taints);
            r
        })
        .collect()
}

/// A report that proves nothing: `Unknown`, `UNKNOWN`, with `codes`.
fn unknown_report(range: TextRange, codes: &[ReasonCode]) -> StmtReport {
    StmtReport {
        range,
        kind: StmtKind::Other,
        effects: EffectSet::UNKNOWN,
        flags: RiskFlags::empty(),
        class: class_of(EffectSet::UNKNOWN),
        touched: TouchedObjects::default(),
        reasons: codes
            .iter()
            .map(|c| Reason {
                code: *c,
                range: Some(range),
                arg: None,
            })
            .collect(),
        destructive: Vec::new(),
        confidence: Confidence::Conservative,
        set_targets: Vec::new(),
    }
}

/// Adds a doubt to `r`: the reason, and `UNKNOWN`.
fn doubt(r: &mut StmtReport, code: ReasonCode) {
    r.reasons.push(Reason {
        code,
        range: Some(r.range),
        arg: None,
    });
    r.effects |= EffectSet::UNKNOWN;
    r.class = class_of(r.effects);
}

/// Whether libpg_query's raw statements are `items`: as many, in order, and each item exactly
/// its raw statement's significant tokens (from the first to the last token inside the span
/// `stmt_location`..+`stmt_len`, 0 running to the end, comments and edge `;` aside).
fn statements_agree(
    parsed: &pb::ParseResult,
    items: &[TextRange],
    text: &str,
    toks: &[(TextRange, ScanKind)],
) -> bool {
    if parsed.stmts.len() != items.len() {
        return false;
    }
    let len = text.len();
    let semicolon =
        |r: &TextRange| text.get(usize::from(r.start())..usize::from(r.end())) == Some(";");
    let mut floor = 0usize;
    for (raw, item) in parsed.stmts.iter().zip(items) {
        let (Ok(start), Ok(n)) = (
            usize::try_from(raw.stmt_location),
            usize::try_from(raw.stmt_len),
        ) else {
            return false;
        };
        let end = if n == 0 { len } else { start.saturating_add(n) };
        if start < floor || end > len {
            return false;
        }
        let inside: Vec<TextRange> = toks
            .iter()
            .filter(|(r, k)| {
                *k != ScanKind::Comment
                    && usize::from(r.start()) >= start
                    && usize::from(r.end()) <= end
            })
            .map(|(r, _)| *r)
            .collect();
        let first = inside.iter().find(|r| !semicolon(r));
        let last = inside.iter().rev().find(|r| !semicolon(r));
        match (first, last) {
            (Some(f), Some(l)) if f.start() == item.start() && l.end() == item.end() => {}
            _ => return false,
        }
        floor = end;
    }
    true
}

/// Step 3 of the combine rule: our lexer and PostgreSQL's scanner cut the text alike.
// ## m3b-t14-orchestrator (named exception, T10 C1 carry): `pre` is `sql`'s lexing when the
// caller has it.
pub(crate) fn tokens_agree(
    sql: &str,
    ctx: &ClassifyCtx,
    pg: &[(TextRange, ScanKind)],
    pre: Option<&Lexed>,
) -> bool {
    let Ok(profile) = lex_profile(&ctx.dialect) else {
        return false;
    };
    let owned;
    let lexed = match pre {
        Some(l) => l,
        None => {
            owned = sql_lexer::lex(sql, profile, ctx.dialect.modes, &LexState::Initial);
            &owned
        }
    };
    if !lexed.errors().is_empty() {
        return false;
    }
    let theirs: Vec<TextRange> = pg
        .iter()
        .filter(|(_, k)| *k != ScanKind::Comment)
        .map(|(r, _)| *r)
        .collect();
    let text = |r: TextRange| sql.get(usize::from(r.start())..usize::from(r.end()));
    let mut j = 0;
    for i in (0..lexed.len()).filter(|&i| !lexed.is_trivia(i)) {
        let ours = lexed.range(i);
        let kind = lexed.kind(i);
        if !ctx.dialect.modes.standard_conforming_strings
            && kind == TokenKind::String
            && text(ours).is_some_and(|t| t.contains('\\') && !t.starts_with(['E', 'e']))
        {
            return false;
        }
        if theirs.get(j) == Some(&ours) {
            j += 1;
            continue;
        }
        let split_placeholder = kind == TokenKind::NamedParam
            && matches!(
                (theirs.get(j), theirs.get(j + 1)),
                (Some(a), Some(b)) if a.start() == ours.start()
                    && text(*a) == Some(":")
                    && a.end() == b.start()
                    && b.end() == ours.end()
            );
        if split_placeholder {
            j += 2;
            continue;
        }
        return false;
    }
    j == theirs.len()
}

// ---------------------------------------------------------------------------------------------
// One statement

/// The report of one raw statement, `range` being our item, and whether running it may change
/// how later statements must be classified (search_path, lexing modes, role).
fn statement(
    sql: &str,
    range: TextRange,
    raw: &pb::RawStmt,
    ctx: &ClassifyCtx,
    table: &Table,
    toks: &[(TextRange, ScanKind)],
) -> (StmtReport, bool) {
    let Some(root) = raw.stmt.as_deref().and_then(g::node_inner) else {
        return (unknown_report(range, &[ReasonCode::EffectsUnknown]), true);
    };
    let mut c = Classifier::new(sql, range, ctx, table, toks);
    g::walk(root, &mut c);
    let changes_context = c.changes_context;
    (c.finish(), changes_context)
}

/// Context flags of an entered message, undone on `leave`.
const F_STMT: u8 = 1;
const F_DEFERRED: u8 = 2;
const F_CTE: u8 = 4;
const F_ALTER: u8 = 8;
/// Inside a `RangeFunction`, whose column definitions coerce the function's result.
const F_COLDEF: u8 = 16;

/// The visitor that classifies one statement.
struct Classifier<'a, 's> {
    sql: &'s str,
    range: TextRange,
    ctx: &'s ClassifyCtx,
    table: &'s Table,
    toks: &'s [(TextRange, ScanKind)],
    dialect: DialectId,
    /// Whether an unqualified name reaches `pg_catalog` first: the session's known
    /// `search_path` leaves it out or names it first (P-M3b-T3-1; unknown: assumed, as the
    /// server does when `pg_catalog` is not listed).
    catalog_first: bool,
    kind: Option<StmtKind>,
    effects: EffectSet,
    flags: RiskFlags,
    reasons: Vec<Reason>,
    destructive: Vec<DestructiveReason>,
    touched: TouchedObjects,
    conservative: bool,
    set_targets: Vec<SetTarget>,
    /// Statements entered and not left.
    depth: u32,
    /// Enclosing statements whose sub-statements do not run now.
    deferred: u32,
    /// Enclosing CTEs.
    cte: u32,
    /// Enclosing `RangeFunction`s.
    coldef: u32,
    /// Running this statement may change how later statements must be classified.
    changes_context: bool,
    /// The relation of each enclosing `ALTER TABLE`.
    alter: Vec<Option<ObjectRef>>,
    /// The frames of entered messages (`F_*`), popped on `leave`.
    frames: Vec<u8>,
    /// Relations a statement writes (identity), so the walk does not count them as reads.
    write_targets: Vec<*const pb::RangeVar>,
    /// Where each written relation's name is written.
    target_spans: Vec<(ObjectRef, TextRange)>,
    _tree: std::marker::PhantomData<&'a ()>,
}

impl<'a, 's> Classifier<'a, 's> {
    fn new(
        sql: &'s str,
        range: TextRange,
        ctx: &'s ClassifyCtx,
        table: &'s Table,
        toks: &'s [(TextRange, ScanKind)],
    ) -> Self {
        Classifier {
            sql,
            range,
            ctx,
            table,
            toks,
            dialect: ctx.dialect.id,
            catalog_first: catalog_first(&ctx.search_path),
            kind: None,
            effects: EffectSet::empty(),
            flags: RiskFlags::empty(),
            reasons: Vec::new(),
            destructive: Vec::new(),
            touched: TouchedObjects::default(),
            conservative: false,
            set_targets: Vec::new(),
            depth: 0,
            deferred: 0,
            cte: 0,
            coldef: 0,
            changes_context: false,
            alter: Vec::new(),
            frames: Vec::new(),
            write_targets: Vec::new(),
            target_spans: Vec::new(),
            _tree: std::marker::PhantomData,
        }
    }

    fn finish(mut self) -> StmtReport {
        if self.effects.is_empty() {
            // Nothing was proved (a statement the rules gave no effect): fail closed.
            self.unknown(ReasonCode::EffectsUnknown, None, None);
        }
        let mut reasons: Vec<Reason> = Vec::with_capacity(self.reasons.len());
        for r in self.reasons {
            if !reasons.contains(&r) {
                reasons.push(r);
            }
        }
        dedup(&mut self.touched.reads);
        dedup(&mut self.touched.writes);
        dedup(&mut self.touched.calls);
        StmtReport {
            range: self.range,
            kind: self.kind.unwrap_or(StmtKind::Other),
            effects: self.effects,
            flags: self.flags,
            class: class_of(self.effects),
            touched: self.touched,
            reasons,
            destructive: self.destructive,
            confidence: if self.conservative {
                Confidence::Conservative
            } else {
                Confidence::Exact
            },
            set_targets: self.set_targets,
        }
    }

    /// The range of the scanner token at byte `location`, or the statement's.
    fn at(&self, location: i32) -> TextRange {
        usize::try_from(location)
            .ok()
            .and_then(|l| self.token_index(l))
            .map_or(self.range, |i| self.toks[i].0)
    }

    /// The scanner token that starts at byte `at`.
    fn token_index(&self, at: usize) -> Option<usize> {
        let at = TextSize::try_from(at).ok()?;
        self.toks.binary_search_by(|(r, _)| r.start().cmp(&at)).ok()
    }

    /// Whether the (possibly qualified) name that starts at `location` was written with a
    /// quoted part. A name with no place in the text counts as quoted unless `pg_catalog`
    /// qualifies it (the grammar's own names, `pg_catalog.extract`, …): fail closed.
    fn quoted_at(&self, location: i32, qualified_by_catalog: bool) -> bool {
        let Some(mut i) = usize::try_from(location)
            .ok()
            .and_then(|l| self.token_index(l))
        else {
            return !qualified_by_catalog;
        };
        // `part ( . part )*`, comments allowed between the tokens.
        let text = |i: usize| {
            self.toks.get(i).map(|(r, k)| {
                let t = self
                    .sql
                    .get(usize::from(r.start())..usize::from(r.end()))
                    .unwrap_or("");
                (t, *k)
            })
        };
        let skip_comments = |mut i: usize| {
            while matches!(text(i), Some((_, ScanKind::Comment))) {
                i += 1;
            }
            i
        };
        loop {
            match text(i) {
                Some((t, ScanKind::Ident))
                    if t.starts_with('"') || t.starts_with("U&") || t.starts_with("u&") =>
                {
                    return true;
                }
                Some((_, ScanKind::Ident | ScanKind::Keyword)) => {}
                _ => return false,
            }
            i = skip_comments(i + 1);
            match text(i) {
                Some((".", ScanKind::Other)) => i = skip_comments(i + 1),
                _ => return false,
            }
        }
    }

    fn reason(&mut self, code: ReasonCode, range: Option<TextRange>, arg: Option<&str>) {
        self.reasons.push(Reason {
            code,
            range: Some(range.unwrap_or(self.range)),
            arg: arg.map(Arc::from),
        });
    }

    /// `UNKNOWN` with `code`.
    fn unknown(&mut self, code: ReasonCode, range: Option<TextRange>, arg: Option<&str>) {
        self.effects |= EffectSet::UNKNOWN;
        self.reason(code, range, arg);
    }

    fn top(&self) -> bool {
        self.depth == 0
    }

    /// Records `rv` as written (and where its name is written), and returns its object.
    fn write_target(&mut self, rv: &pb::RangeVar, kind: ObjectKind) -> ObjectRef {
        self.write_targets.push(rv as *const pb::RangeVar);
        let o = object(range_var_name(rv), kind);
        self.touched.writes.push(o.clone());
        if let Some(span) = self.span_at(rv.location) {
            self.target_spans.push((o.clone(), span));
        }
        o
    }

    /// A destructive pattern; its range is where the target's name is written (fix round 1,
    /// M7: the type-to-confirm text is those bytes), else the statement's.
    fn destroy(&mut self, kind: DestructiveKind, target: Option<ObjectRef>) {
        if self.deferred > 0 {
            return;
        }
        let range = target
            .as_ref()
            .and_then(|t| {
                self.target_spans
                    .iter()
                    .find(|(o, _)| o == t)
                    .map(|(_, r)| *r)
                    .or_else(|| self.find_name(&t.name))
            })
            .unwrap_or(self.range);
        self.destructive.push(DestructiveReason {
            kind,
            range,
            target,
        });
    }

    /// Token `i`: its text, kind and range.
    fn tok(&self, i: usize) -> Option<(&'s str, ScanKind, TextRange)> {
        let (r, k) = *self.toks.get(i)?;
        let t = self.sql.get(usize::from(r.start())..usize::from(r.end()))?;
        Some((t, k, r))
    }

    /// The first token at or after `i` that is not a comment.
    fn skip_comments(&self, mut i: usize) -> usize {
        while matches!(self.tok(i), Some((_, ScanKind::Comment, _))) {
            i += 1;
        }
        i
    }

    /// The span of the name `part ( . part )*` that starts at token `i`.
    fn name_span_from(&self, i: usize) -> Option<TextRange> {
        let (_, k, first) = self.tok(i)?;
        if !matches!(k, ScanKind::Ident | ScanKind::Keyword) {
            return None;
        }
        let (mut end, mut j) = (first.end(), i);
        loop {
            let dot = self.skip_comments(j + 1);
            if !matches!(self.tok(dot), Some((".", ScanKind::Other, _))) {
                break;
            }
            let next = self.skip_comments(dot + 1);
            match self.tok(next) {
                Some((_, ScanKind::Ident | ScanKind::Keyword, r)) => {
                    end = r.end();
                    j = next;
                }
                _ => break,
            }
        }
        Some(TextRange::new(first.start(), end))
    }

    /// The span of the name written at byte `location`.
    fn span_at(&self, location: i32) -> Option<TextRange> {
        let at = usize::try_from(location).ok()?;
        self.name_span_from(self.token_index(at)?)
    }

    /// Where `name` is first written inside the statement: name tokens whose server reading
    /// (unquoted folded to ASCII lower case, quoted taken exactly) are its parts.
    fn find_name(&self, name: &QualifiedName) -> Option<TextRange> {
        let read = |t: &str, k: ScanKind| -> Option<String> {
            match k {
                ScanKind::Ident if t.starts_with('"') => t
                    .strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                    .map(|s| s.replace("\"\"", "\"")),
                ScanKind::Ident if t.starts_with("U&") || t.starts_with("u&") => None,
                ScanKind::Ident | ScanKind::Keyword => Some(t.to_ascii_lowercase()),
                _ => None,
            }
        };
        let in_range = |r: TextRange| self.range.contains_range(r);
        'start: for i in 0..self.toks.len() {
            let mut j = i;
            for (n, part) in name.parts.iter().enumerate() {
                if n > 0 {
                    let dot = self.skip_comments(j + 1);
                    if !matches!(self.tok(dot), Some((".", ScanKind::Other, _))) {
                        continue 'start;
                    }
                    j = self.skip_comments(dot + 1);
                }
                match self.tok(j) {
                    Some((t, k, r))
                        if in_range(r) && read(t, k).as_deref() == Some(&*part.text) => {}
                    _ => continue 'start,
                }
            }
            let start = self.toks[i].0.start();
            let end = self.toks[j].0.end();
            return Some(TextRange::new(start, end));
        }
        None
    }

    // -- statements -------------------------------------------------------------------------

    /// A statement message: its row, then its own rules. Returns the frame bits.
    fn statement(&mut self, r: PgRef<'a>) -> u8 {
        let top = self.top();
        let mut frame = F_STMT;
        let Some(row) = self.table.get(&r.kind()) else {
            self.unknown(ReasonCode::EffectsUnknown, None, Some(r.kind().name()));
            return frame;
        };
        self.effects |= row.effects;
        self.flags |= row.flags;
        for code in row.reasons.clone() {
            self.reason(code, None, None);
        }
        if top {
            self.kind = Some(row.kind);
        }
        if row.deferred {
            frame |= F_DEFERRED;
            self.deferred += 1;
        }
        match r {
            PgRef::SelectStmt(s) => {
                if top && s.into_clause.is_some() {
                    self.kind = Some(StmtKind::CreateTable);
                } else if top && !s.values_lists.is_empty() {
                    self.kind = Some(StmtKind::Values);
                }
            }
            PgRef::InsertStmt(s) => {
                self.dml(s.relation.as_ref());
            }
            PgRef::UpdateStmt(s) => {
                if s.relation
                    .as_ref()
                    .is_some_and(|rv| rv.relname == "pg_settings")
                {
                    // `pg_settings`'s update rule runs `set_config(name, setting, false)`: the
                    // same session change, in any schema (a user relation of that name is read
                    // the same way, conservatively; round 2, R1-I2).
                    self.effects |= EffectSet::SESSION_STATE | EffectSet::ADMIN;
                    self.reason(ReasonCode::SessionState, None, Some("pg_settings"));
                    self.reason(ReasonCode::AdminControl, None, None);
                    self.changes_context = true;
                }
                let target = self.dml(s.relation.as_ref());
                self.unbounded(
                    s.where_clause.as_deref(),
                    &s.from_clause,
                    s.relation.as_ref(),
                    target,
                );
            }
            PgRef::DeleteStmt(s) => {
                let target = self.dml(s.relation.as_ref());
                self.unbounded(
                    s.where_clause.as_deref(),
                    &s.using_clause,
                    s.relation.as_ref(),
                    target,
                );
            }
            PgRef::MergeStmt(s) => {
                let target = self.dml(s.relation.as_ref());
                let deletes = s.merge_when_clauses.iter().any(|w| {
                    matches!(&w.node, Some(pb::node::Node::MergeWhenClause(m))
                        if m.command_type == pb::CmdType::CmdDelete as i32)
                });
                if deletes {
                    self.destroy(DestructiveKind::MergeDelete, target);
                }
            }
            PgRef::TruncateStmt(s) => {
                for rel in &s.relations {
                    if let Some(pb::node::Node::RangeVar(rv)) = &rel.node {
                        let o = self.write_target(rv, ObjectKind::Table);
                        self.destroy(DestructiveKind::Truncate, Some(o));
                    }
                }
            }
            PgRef::CopyStmt(s) => self.copy(s),
            PgRef::ExplainStmt(s) => {
                let analyze = explain_analyze(&s.options);
                if analyze {
                    self.flags |= RiskFlags::EXPLAIN_ANALYZE;
                } else if frame & F_DEFERRED == 0 {
                    // A plain EXPLAIN runs nothing (one increment per frame, undone on leave).
                    frame |= F_DEFERRED;
                    self.deferred += 1;
                }
                if top {
                    self.kind = Some(StmtKind::Explain { analyze });
                }
            }
            PgRef::VariableSetStmt(s) if top => self.set(s),
            PgRef::TransactionStmt(s) => self.transaction(s, top),
            PgRef::GrantStmt(s) if top && !s.is_grant => self.kind = Some(StmtKind::Revoke),
            PgRef::GrantRoleStmt(s) if top && !s.is_grant => self.kind = Some(StmtKind::Revoke),
            PgRef::DropStmt(s) => {
                let cascade = s.behavior == pb::DropBehavior::DropCascade as i32;
                let kind = drop_object_kind(s.remove_type);
                if s.objects.is_empty() {
                    self.destroy(DestructiveKind::Drop { cascade }, None);
                }
                for o in &s.objects {
                    let target = drop_object_name(o).map(|n| object(n, kind));
                    self.destroy(DestructiveKind::Drop { cascade }, target);
                }
            }
            PgRef::DropOwnedStmt(s) => {
                let cascade = s.behavior == pb::DropBehavior::DropCascade as i32;
                self.destroy(DestructiveKind::Drop { cascade }, None);
            }
            PgRef::DropdbStmt(s) => {
                let target = object(simple_name(&s.dbname), ObjectKind::Database);
                self.destroy(DestructiveKind::Drop { cascade: false }, Some(target));
            }
            PgRef::DropRoleStmt(s) => {
                for role in &s.roles {
                    let target = match &role.node {
                        Some(pb::node::Node::RoleSpec(rs)) if !rs.rolename.is_empty() => {
                            Some(object(simple_name(&rs.rolename), ObjectKind::Role))
                        }
                        _ => None,
                    };
                    self.destroy(DestructiveKind::Drop { cascade: false }, target);
                }
            }
            PgRef::DropTableSpaceStmt(_)
            | PgRef::DropUserMappingStmt(_)
            | PgRef::DropSubscriptionStmt(_) => {
                self.destroy(DestructiveKind::Drop { cascade: false }, None);
            }
            PgRef::AlterTableStmt(s) => {
                let target = s
                    .relation
                    .as_ref()
                    .map(|rv| self.write_target(rv, ObjectKind::Table));
                self.alter.push(target);
                frame |= F_ALTER;
            }
            PgRef::CreateStmt(s) => {
                self.ddl_maybe_temp(s.relation.as_ref());
            }
            PgRef::CreateSeqStmt(s) => {
                self.ddl_maybe_temp(s.sequence.as_ref());
            }
            PgRef::ViewStmt(s) => {
                self.ddl_maybe_temp(s.view.as_ref());
            }
            PgRef::DeclareCursorStmt(s) if s.options & CURSOR_OPT_HOLD != 0 => {
                self.effects |= EffectSet::SESSION_STATE;
            }
            PgRef::CallStmt(s) => {
                // A procedure is never a reviewed builtin function, whatever its name: its
                // target is unknown, and it may change any setting (round 2, R1-I1).
                let name = s
                    .funccall
                    .as_ref()
                    .and_then(|f| strings(&f.funcname))
                    .map(|p| p.join("."));
                self.unknown(ReasonCode::FnUnknown, None, name.as_deref());
                self.changes_context = true;
            }
            PgRef::LoadStmt(_) => {
                // A loaded library's `_PG_init` may change any setting (round 2, R1-M1).
                self.changes_context = true;
            }
            PgRef::DiscardStmt(s) if s.target == pb::DiscardMode::DiscardAll as i32 => {
                // `DISCARD ALL` includes `SET SESSION AUTHORIZATION DEFAULT` and `RESET ALL`
                // (fix round 1, M5).
                self.session_reset();
            }
            PgRef::AlterDomainStmt(s) if s.subtype == "X" => {
                // `ALTER DOMAIN … DROP CONSTRAINT` (fix round 1, M6).
                let target =
                    strings(&s.type_name).map(|p| object(qualified(&p), ObjectKind::Other));
                self.destroy(DestructiveKind::DropConstraint, target);
            }
            _ => {}
        }
        frame
    }

    /// A data change's target: written; inside a CTE it is a data-modifying CTE.
    fn dml(&mut self, relation: Option<&pb::RangeVar>) -> Option<ObjectRef> {
        if self.cte > 0 {
            self.reason(ReasonCode::DataModifyingCte, None, None);
        }
        relation.map(|rv| self.write_target(rv, ObjectKind::Table))
    }

    /// `UPDATE`/`DELETE` that may touch every row (`alg/13` SAFE-07 steps 1-2).
    fn unbounded(
        &mut self,
        where_clause: Option<&pb::Node>,
        joined: &[pb::Node],
        relation: Option<&pb::RangeVar>,
        target: Option<ObjectRef>,
    ) {
        match where_clause {
            None => self.destroy(DestructiveKind::NoWhere, target),
            Some(w) if always_true(w) => self.destroy(DestructiveKind::AlwaysTrue, target),
            Some(w) if !joined.is_empty() && !relation.is_some_and(|rv| references(w, rv)) => {
                self.destroy(DestructiveKind::JoinWithoutOn, target);
            }
            Some(_) => {}
        }
    }

    /// `CREATE [TEMP] TABLE`/`SEQUENCE`/`VIEW`: temp objects are `DDL_TEMP`.
    fn ddl_maybe_temp(&mut self, rv: Option<&pb::RangeVar>) {
        match rv {
            Some(rv) if rv.relpersistence == "t" => {
                self.effects |= EffectSet::DDL_TEMP;
                self.write_target(rv, ObjectKind::Table);
            }
            Some(rv) => {
                self.effects |= EffectSet::DDL;
                self.write_target(rv, ObjectKind::Table);
            }
            None => self.unknown(ReasonCode::EffectsUnknown, None, None),
        }
    }

    /// `COPY` (`alg/13` PG table): from a source is a write, to a destination a read; a server
    /// file is external I/O, a program process execution; a query adds its own effects.
    fn copy(&mut self, s: &'a pb::CopyStmt) {
        if s.is_from {
            self.effects |= EffectSet::WRITE_DATA;
            if let Some(rv) = &s.relation {
                self.write_target(rv, ObjectKind::Table);
            }
        } else {
            self.effects |= EffectSet::READ;
        }
        if s.is_program {
            self.effects |= EffectSet::PROCESS_EXEC;
            self.reason(ReasonCode::ProcessExec, None, None);
        } else if !s.filename.is_empty() {
            self.effects |= EffectSet::EXTERNAL_IO;
            self.reason(ReasonCode::ExternalFile, None, None);
        }
    }

    /// A top-level `SET`/`RESET`: one target per variable with its value as written, one
    /// `SessionState` reason naming it; role switches and lexing changes on top.
    fn set(&mut self, s: &'a pb::VariableSetStmt) {
        use pb::VariableSetKind as V;
        let kind = V::try_from(s.kind).unwrap_or(V::Undefined);
        if kind == V::VarResetAll {
            self.reason(ReasonCode::SessionState, None, None);
            self.session_reset();
            return;
        }
        if s.name.is_empty() || kind == V::Undefined {
            self.unknown(ReasonCode::EffectsUnknown, None, None);
            self.changes_context = true;
            return;
        }
        let value = match kind {
            V::VarSetValue => s
                .args
                .first()
                .and_then(node_location)
                .and_then(|at| self.sql.get(at..usize::from(self.range.end())))
                .map(|v| Arc::from(v.trim())),
            _ => None,
        };
        let name: Arc<str> = Arc::from(s.name.as_str());
        self.variable(&name);
        // A search path that may not reach `pg_catalog` first changes how later names resolve
        // (I2a); the harmless list's own test decides which values keep it first.
        if name.eq_ignore_ascii_case("search_path")
            && !crate::functions::set_allowed(DialectId::Postgres, &name, value.as_deref())
        {
            self.changes_context = true;
        }
        self.reason(ReasonCode::SessionState, None, Some(&name));
        self.set_targets.push(SetTarget { name, value });
    }

    /// What setting `name` implies beyond session state (also for `set_config`): a role switch
    /// and a lexing change are flagged, and both change how later statements read.
    fn variable(&mut self, name: &str) {
        if ROLE_VARIABLES.iter().any(|v| v.eq_ignore_ascii_case(name)) {
            self.effects |= EffectSet::PRIVILEGE;
            self.reason(ReasonCode::PrivilegeChange, None, Some(name));
            self.changes_context = true;
        }
        if LEX_VARIABLES.iter().any(|v| v.eq_ignore_ascii_case(name)) {
            self.flags |= RiskFlags::LEX_MODE_CHANGE;
            self.reason(ReasonCode::LexModeChange, None, Some(name));
            self.changes_context = true;
        }
    }

    /// `RESET ALL`, `DISCARD ALL`: every setting returns to its default, the lexing ones and
    /// (for `DISCARD ALL`) the session authorization included, so both a lexing change and a
    /// role switch (fix round 1, M5); later statements read differently (I2a).
    fn session_reset(&mut self) {
        self.effects |= EffectSet::PRIVILEGE;
        self.reason(ReasonCode::PrivilegeChange, None, None);
        self.flags |= RiskFlags::LEX_MODE_CHANGE;
        self.reason(ReasonCode::LexModeChange, None, None);
        self.changes_context = true;
    }

    fn transaction(&mut self, s: &'a pb::TransactionStmt, top: bool) {
        use pb::TransactionStmtKind as T;
        let kind = T::try_from(s.kind).unwrap_or(T::Undefined);
        let stmt_kind = match kind {
            T::TransStmtBegin | T::TransStmtStart => StmtKind::Begin,
            T::TransStmtCommit | T::TransStmtCommitPrepared => StmtKind::Commit,
            T::TransStmtRollback | T::TransStmtRollbackPrepared | T::TransStmtPrepare => {
                StmtKind::Rollback
            }
            T::TransStmtSavepoint | T::TransStmtRelease | T::TransStmtRollbackTo => {
                StmtKind::Savepoint
            }
            T::Undefined => {
                self.unknown(ReasonCode::EffectsUnknown, None, None);
                StmtKind::Other
            }
        };
        if matches!(
            kind,
            T::TransStmtPrepare | T::TransStmtCommitPrepared | T::TransStmtRollbackPrepared
        ) {
            // Two-phase commit outlives the session and ends other sessions' work.
            self.effects |= EffectSet::WRITE_DATA | EffectSet::ADMIN;
            self.reason(ReasonCode::AdminControl, None, None);
        }
        if top {
            self.kind = Some(stmt_kind);
        }
    }

    // -- expressions ------------------------------------------------------------------------

    /// A function call by name (`alg/13` SAFE-03): deny list, then Task 3's builtins, then the
    /// profile's allowlist, then the catalog; else `Unknown`.
    fn call(&mut self, name: &[pb::Node], arity: usize, args: &[pb::Node], location: i32) {
        let Some(parts) = strings(name) else {
            self.unknown(ReasonCode::FnUnknown, Some(self.at(location)), None);
            return;
        };
        let dotted = parts.join(".");
        let qn = qualified(&parts);
        self.touched
            .calls
            .push(object(qn.clone(), ObjectKind::Function));
        let range = Some(self.at(location));
        let by_catalog = parts.len() == 2 && parts[0] == "pg_catalog";
        if parts
            .last()
            .is_some_and(|n| n.eq_ignore_ascii_case("pg_reload_conf"))
        {
            // A reload gives the session every setting it has not overridden itself, `ALTER
            // SYSTEM`'s new values included (round 2, R1-M1).
            self.changes_context = true;
        }
        match lookup_fn(self.dialect, &qn, self.ctx.dialect.version) {
            FnClass::Denied(group) => {
                self.reason(ReasonCode::FnDenylisted, range, Some(&dotted));
                self.denied(group, &parts, args, range);
            }
            FnClass::Allowed
                if (by_catalog || self.catalog_first) && !self.quoted_at(location, by_catalog) => {}
            FnClass::Allowed | FnClass::Unknown => self.not_builtin(&qn, &dotted, arity, range),
        }
    }

    /// A call Task 3's data does not prove pure.
    fn not_builtin(
        &mut self,
        qn: &QualifiedName,
        dotted: &str,
        arity: usize,
        range: Option<TextRange>,
    ) {
        if allowlisted(&self.ctx.fn_allowlist, qn) {
            self.conservative = true;
            return;
        }
        let looked_up = self
            .ctx
            .catalog
            .as_ref()
            .map(|c| c.lookup(qn, u16::try_from(arity).ok()));
        match looked_up {
            Some(FnLookup::Known {
                volatile: false,
                writes: false,
            }) => self.conservative = true,
            Some(FnLookup::Known {
                volatile: true,
                writes,
            }) => {
                self.conservative = true;
                if writes {
                    self.effects |= EffectSet::WRITE_DATA;
                }
                self.flags |= RiskFlags::VOLATILE_FN;
                self.unknown(ReasonCode::VolatileFunction, range, Some(dotted));
            }
            Some(FnLookup::Known { writes: true, .. }) => {
                self.conservative = true;
                self.effects |= EffectSet::WRITE_DATA;
                self.unknown(ReasonCode::FnUnknown, range, Some(dotted));
            }
            Some(FnLookup::Unknown) | None => {
                self.unknown(ReasonCode::FnUnknown, range, Some(dotted));
            }
        }
    }

    /// The effects of a denied function's group.
    fn denied(
        &mut self,
        group: DenyGroup,
        parts: &[&str],
        args: &[pb::Node],
        range: Option<TextRange>,
    ) {
        let bare = parts.last().copied().unwrap_or("").to_ascii_lowercase();
        let (effects, code) = match group {
            DenyGroup::Sleep => (EffectSet::READ, None),
            DenyGroup::FileIo => (EffectSet::EXTERNAL_IO, Some(ReasonCode::ExternalFile)),
            DenyGroup::Network => {
                if bare.starts_with("dblink") {
                    self.flags |= RiskFlags::DYNAMIC_SQL;
                }
                (EffectSet::EXTERNAL_IO, Some(ReasonCode::ExternalNetwork))
            }
            DenyGroup::ProcessExec => (EffectSet::PROCESS_EXEC, Some(ReasonCode::ProcessExec)),
            DenyGroup::AdminSignal | DenyGroup::Replication => {
                (EffectSet::ADMIN, Some(ReasonCode::AdminControl))
            }
            DenyGroup::LargeObject => (EffectSet::WRITE_DATA, Some(ReasonCode::WriteData)),
            DenyGroup::DynamicSql => {
                self.flags |= RiskFlags::DYNAMIC_SQL;
                (EffectSet::UNKNOWN, Some(ReasonCode::DynamicSql))
            }
            DenyGroup::Advisory => (EffectSet::ADVISORY_LOCK, Some(ReasonCode::AdvisoryLock)),
            DenyGroup::Sequence => (EffectSet::SEQUENCE, Some(ReasonCode::SequenceAdvance)),
            DenyGroup::XidConsume => (EffectSet::XID_CONSUME, Some(ReasonCode::XidConsume)),
            DenyGroup::SetConfig => {
                // `set_config(name, value, is_local)`: a session change that the RO pin does
                // not stop (case 8: Admin); the name decides role and lexing changes. It may set
                // any setting (the search path among them), so later statements read
                // differently (I2a).
                self.changes_context = true;
                match args.first().and_then(string_const) {
                    Some(name) => self.variable(name),
                    None => self.unknown(ReasonCode::EffectsUnknown, range, Some("set_config")),
                }
                (
                    EffectSet::SESSION_STATE | EffectSet::ADMIN,
                    Some(ReasonCode::SessionState),
                )
            }
        };
        self.effects |= effects;
        if let Some(code) = code {
            self.reason(code, range, None);
        }
    }

    /// An operator by name (`OPERATOR(schema.op)` included).
    fn operator(&mut self, name: &[pb::Node], location: i32) {
        let known = strings(name).is_some_and(|parts| match parts.as_slice() {
            [op] => self.catalog_first && operator_known(self.dialect, op),
            ["pg_catalog", op] => operator_known(self.dialect, op),
            _ => false,
        });
        if !known {
            let op = strings(name).map(|p| p.join("."));
            self.unknown(
                ReasonCode::OperatorUnknown,
                Some(self.at(location)),
                op.as_deref(),
            );
        }
    }

    fn a_expr(&mut self, e: &pb::AExpr) {
        use pb::AExprKind as A;
        match A::try_from(e.kind).unwrap_or(A::Undefined) {
            A::AexprBetween | A::AexprNotBetween | A::AexprBetweenSym | A::AexprNotBetweenSym => {
                if !(self.catalog_first
                    && operator_known(self.dialect, ">=")
                    && operator_known(self.dialect, "<="))
                {
                    self.unknown(ReasonCode::OperatorUnknown, Some(self.at(e.location)), None);
                }
            }
            A::Undefined => {
                self.unknown(ReasonCode::OperatorUnknown, Some(self.at(e.location)), None)
            }
            _ => self.operator(&e.name, e.location),
        }
    }

    /// A coercion to a named type: an explicit cast (`::`, `CAST`, `type 'literal'`) and every
    /// other place that names a target type (fix round 1, I1: SQL/JSON `RETURNING`,
    /// `JSON_TABLE`/`XMLTABLE` column types, `XMLSERIALIZE … AS`, a function's column
    /// definition list). The type must be a builtin written unquoted (`cast_known`); anything
    /// else, a domain or user type whose input, cast or CHECK may run code, is `Unknown`.
    fn coerce(&mut self, tn: Option<&pb::TypeName>, location: i32) {
        let Some(tn) = tn else {
            self.unknown(ReasonCode::CastUnknown, Some(self.at(location)), None);
            return;
        };
        let parts = strings(&tn.names);
        let ok = parts.as_ref().is_some_and(|parts| {
            let by_catalog = parts.len() == 2 && parts[0] == "pg_catalog";
            let mut text = parts.join(".");
            if !tn.array_bounds.is_empty() {
                text.push_str("[]");
            }
            // The single-byte `"char"` is spelled quoted (M9); any other quoted name is not a
            // builtin.
            let quoted_char = *parts == ["char"]
                && usize::try_from(tn.location)
                    .ok()
                    .and_then(|l| self.token_index(l))
                    .and_then(|i| self.tok(i))
                    .is_some_and(|(t, _, _)| t == "\"char\"");
            if quoted_char {
                text = "\"char\"".to_string();
                if !tn.array_bounds.is_empty() {
                    text.push_str("[]");
                }
            }
            !tn.pct_type
                && !tn.setof
                && parts
                    .iter()
                    .all(|p| !p.bytes().any(|b| b.is_ascii_uppercase()))
                && (by_catalog || self.catalog_first)
                && (quoted_char || !self.quoted_at(tn.location, by_catalog))
                && cast_known(self.dialect, &text)
        });
        if !ok {
            let text = parts.map(|p| p.join("."));
            self.unknown(
                ReasonCode::CastUnknown,
                Some(self.at(tn.location)),
                text.as_deref(),
            );
        }
    }

    /// An `=` the server looks up by name through the search path (`CASE x WHEN`, `JOIN …
    /// USING`, `NATURAL JOIN`; fix round 1, M2).
    fn implicit_equals(&mut self, location: i32) {
        if !(self.catalog_first && operator_known(self.dialect, "=")) {
            self.unknown(
                ReasonCode::OperatorUnknown,
                Some(self.at(location)),
                Some("="),
            );
        }
    }

    fn range_var(&mut self, rv: &pb::RangeVar) {
        if DISCLOSING_RELATIONS
            .iter()
            .any(|d| rv.relname.eq_ignore_ascii_case(d))
        {
            // Any schema and any spelling (a quoted `"PG_STAT_ACTIVITY"` too): conservative.
            let range = self.span_at(rv.location);
            self.unknown(ReasonCode::EffectsUnknown, range, Some(&rv.relname));
        }
        let ptr = rv as *const pb::RangeVar;
        if !self.write_targets.iter().any(|w| std::ptr::eq(*w, ptr)) {
            self.touched
                .reads
                .push(object(range_var_name(rv), ObjectKind::Table));
        }
    }

    fn alter_cmd(&mut self, cmd: &pb::AlterTableCmd) {
        use pb::AlterTableType as T;
        let kind = match T::try_from(cmd.subtype) {
            Ok(T::AtChangeOwner) => {
                // `ALTER TABLE … OWNER TO` changes privileges (fix round 1, M6).
                self.effects |= EffectSet::PRIVILEGE;
                return;
            }
            Ok(T::AtDropColumn) => DestructiveKind::DropColumn,
            Ok(T::AtDropConstraint) => DestructiveKind::DropConstraint,
            Ok(T::AtAlterColumnType) => DestructiveKind::AlterType,
            _ => return,
        };
        let target = self.alter.last().cloned().flatten();
        self.destroy(kind, target);
    }
}

impl<'a> Visitor<'a> for Classifier<'a, '_> {
    fn enter(&mut self, r: PgRef<'a>) -> Walk {
        let mut frame = 0u8;
        match role(r.kind()) {
            Role::Stmt => frame = self.statement(r),
            Role::Unhandled => {
                self.unknown(ReasonCode::EffectsUnknown, None, Some(r.kind().name()));
            }
            Role::Neutral => {}
            Role::Rule => match r {
                PgRef::FuncCall(f) => {
                    let args = &f.args;
                    self.call(&f.funcname, args.len(), args, f.location);
                }
                PgRef::RangeTableSample(t) => self.call(&t.method, t.args.len(), &[], t.location),
                PgRef::AExpr(e) => self.a_expr(e),
                PgRef::TypeCast(tc) => self.coerce(tc.type_name.as_ref(), tc.location),
                PgRef::JsonOutput(o) => {
                    if let Some(tn) = &o.type_name {
                        self.coerce(Some(tn), tn.location);
                    }
                }
                PgRef::JsonTableColumn(c) => {
                    if let Some(tn) = &c.type_name {
                        self.coerce(Some(tn), c.location);
                    }
                }
                PgRef::RangeTableFuncCol(c) => {
                    if let Some(tn) = &c.type_name {
                        self.coerce(Some(tn), c.location);
                    }
                }
                PgRef::XmlSerialize(x) => self.coerce(x.type_name.as_ref(), x.location),
                PgRef::RangeFunction(_) => {
                    frame |= F_COLDEF;
                    self.coldef += 1;
                }
                PgRef::ColumnDef(c) if self.coldef > 0 => {
                    self.coerce(c.type_name.as_ref(), c.location);
                }
                PgRef::CaseExpr(c) if c.arg.is_some() => self.implicit_equals(c.location),
                PgRef::JoinExpr(j) if j.is_natural || !j.using_clause.is_empty() => {
                    self.implicit_equals(-1);
                }
                PgRef::SubLink(s) if !s.oper_name.is_empty() => {
                    self.operator(&s.oper_name, s.location);
                }
                PgRef::SortBy(s) if !s.use_op.is_empty() => self.operator(&s.use_op, s.location),
                PgRef::IntoClause(i) => match &i.rel {
                    Some(rv) if rv.relpersistence == "t" => {
                        self.effects |= EffectSet::DDL_TEMP | EffectSet::WRITE_TEMP;
                        self.write_target(rv, ObjectKind::Table);
                    }
                    Some(rv) => {
                        self.effects |= EffectSet::DDL | EffectSet::WRITE_DATA;
                        self.write_target(rv, ObjectKind::Table);
                    }
                    None => self.unknown(ReasonCode::EffectsUnknown, None, None),
                },
                PgRef::LockingClause(_) => {
                    self.effects |= EffectSet::LOCK_ROW;
                    self.reason(ReasonCode::LockingClause, None, None);
                }
                PgRef::CommonTableExpr(_) => {
                    frame |= F_CTE;
                    self.cte += 1;
                }
                PgRef::RangeVar(rv) => self.range_var(rv),
                PgRef::AlterTableCmd(cmd) => self.alter_cmd(cmd),
                _ => {}
            },
        }
        if frame & F_STMT != 0 {
            self.depth += 1;
        }
        self.frames.push(frame);
        Walk::Children
    }

    fn leave(&mut self, _r: PgRef<'a>) {
        let frame = self.frames.pop().unwrap_or(0);
        if frame & F_STMT != 0 {
            self.depth -= 1;
        }
        if frame & F_DEFERRED != 0 {
            self.deferred -= 1;
        }
        if frame & F_CTE != 0 {
            self.cte -= 1;
        }
        if frame & F_ALTER != 0 {
            self.alter.pop();
        }
        if frame & F_COLDEF != 0 {
            self.coldef -= 1;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Helpers over the tree

/// Whether an unqualified name reaches `pg_catalog` before any other schema on a session
/// whose search path is `path` (P-M3b-T3-1).
///
/// **The contract of `ClassifyCtx.search_path`** (fix round 1, I2b): one element per schema,
/// each the schema's name exactly as the server stores it, i.e. folded (what
/// `current_schemas(true)` returns, or `SHOW search_path` split and folded); `$user` is kept
/// literally as `$user`. Empty means unknown: the server searches `pg_catalog` first when the
/// path does not name it, so an unknown path assumes it.
///
/// Fail closed: an element that cannot be such a name (empty, with a comma, a double quote, a
/// control character or edge whitespace, as a raw `SHOW` string has), or that is `pg_catalog`
/// in another case (an
/// unfolded spelling), makes the answer `false`. Otherwise `pg_catalog` must be absent or first.
fn catalog_first(path: &[Arc<str>]) -> bool {
    let mut first_named: Option<usize> = None;
    for (i, s) in path.iter().enumerate() {
        let parseable = !s.is_empty()
            && !s.contains([',', '"'])
            && !s.chars().any(char::is_control)
            && s.trim() == &**s
            && (**s == *"pg_catalog" || !s.eq_ignore_ascii_case("pg_catalog"));
        if !parseable {
            return false;
        }
        if **s == *"pg_catalog" && first_named.is_none() {
            first_named = Some(i);
        }
    }
    first_named.is_none_or(|i| i == 0)
}

/// The texts of a list of `String` nodes; `None` if any element is something else.
fn strings(list: &[pb::Node]) -> Option<Vec<&str>> {
    list.iter()
        .map(|n| match &n.node {
            Some(pb::node::Node::String(s)) => Some(s.sval.as_str()),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
        .filter(|v| !v.is_empty())
}

/// A name from tree parts. The raw tree holds names as the server resolved them, so each part
/// is taken exactly (`quoted`): folding again would make a quoted `"Lower"` read as `lower`.
fn qualified(parts: &[&str]) -> QualifiedName {
    QualifiedName {
        parts: parts
            .iter()
            .map(|p| Ident {
                text: Arc::from(*p),
                quoted: true,
            })
            .collect::<SmallVec<_>>(),
    }
}

fn simple_name(name: &str) -> QualifiedName {
    qualified(&[name])
}

fn object(name: QualifiedName, kind: ObjectKind) -> ObjectRef {
    ObjectRef { name, kind }
}

fn range_var_name(rv: &pb::RangeVar) -> QualifiedName {
    let parts: Vec<&str> = [&rv.catalogname, &rv.schemaname, &rv.relname]
        .into_iter()
        .filter(|p| !p.is_empty())
        .map(String::as_str)
        .collect();
    qualified(&parts)
}

fn dedup(objects: &mut Vec<ObjectRef>) {
    let mut seen: Vec<ObjectRef> = Vec::with_capacity(objects.len());
    for o in objects.drain(..) {
        if !seen.contains(&o) {
            seen.push(o);
        }
    }
    *objects = seen;
}

/// Whether the profile's allowlist names `qn` exactly (each entry read as a PostgreSQL name and
/// folded as the server folds it).
fn allowlisted(list: &[Arc<str>], qn: &QualifiedName) -> bool {
    list.iter().any(|entry| {
        parse_qualified(DialectId::Postgres, entry).is_some_and(|e| {
            e.parts.len() == qn.parts.len()
                && e.parts.iter().zip(&qn.parts).all(|(a, b)| {
                    *sql_ident::fold(DialectId::Postgres, &a.text, a.quoted) == *b.text
                })
        })
    })
}

/// A string constant's text.
fn string_const(n: &pb::Node) -> Option<&str> {
    match &n.node {
        Some(pb::node::Node::AConst(c)) => match &c.val {
            Some(pb::a_const::Val::Sval(s)) => Some(s.sval.as_str()),
            _ => None,
        },
        _ => None,
    }
}

/// Where an argument starts in the text, for the nodes a `SET` value is made of.
fn node_location(n: &pb::Node) -> Option<usize> {
    use pb::node::Node as N;
    let at = match n.node.as_ref()? {
        N::AConst(c) => c.location,
        N::TypeCast(t) => t.location,
        N::ColumnRef(c) => c.location,
        N::ParamRef(p) => p.location,
        N::FuncCall(f) => f.location,
        N::AExpr(e) => e.location,
        _ => return None,
    };
    usize::try_from(at).ok()
}

/// Whether an `EXPLAIN` option list asks for `ANALYZE`: any `analyze`/`analyse` option that is
/// not plainly false (`false`, `off`, `0`; PostgreSQL rejects other spellings, which count as
/// true here).
fn explain_analyze(options: &[pb::Node]) -> bool {
    options.iter().any(|o| match &o.node {
        Some(pb::node::Node::DefElem(d))
            if d.defname.eq_ignore_ascii_case("analyze")
                || d.defname.eq_ignore_ascii_case("analyse") =>
        {
            match d.arg.as_deref().and_then(|a| a.node.as_ref()) {
                None => true,
                Some(pb::node::Node::String(s)) => {
                    !(s.sval.eq_ignore_ascii_case("false") || s.sval.eq_ignore_ascii_case("off"))
                }
                Some(pb::node::Node::Integer(i)) => i.ival != 0,
                Some(pb::node::Node::Boolean(b)) => b.boolval,
                Some(_) => true,
            }
        }
        _ => false,
    })
}

fn drop_object_kind(remove_type: i32) -> ObjectKind {
    use pb::ObjectType as O;
    match O::try_from(remove_type) {
        Ok(O::ObjectTable | O::ObjectForeignTable) => ObjectKind::Table,
        Ok(O::ObjectView | O::ObjectMatview) => ObjectKind::View,
        Ok(O::ObjectSequence) => ObjectKind::Sequence,
        Ok(O::ObjectSchema) => ObjectKind::Schema,
        Ok(O::ObjectDatabase) => ObjectKind::Database,
        Ok(O::ObjectRole) => ObjectKind::Role,
        Ok(O::ObjectFunction | O::ObjectProcedure | O::ObjectRoutine | O::ObjectAggregate) => {
            ObjectKind::Function
        }
        _ => ObjectKind::Other,
    }
}

/// The name of a dropped object: a `List` of `String`s, a `String`, an `ObjectWithArgs` or a
/// `TypeName`.
fn drop_object_name(n: &pb::Node) -> Option<QualifiedName> {
    use pb::node::Node as N;
    let parts = match n.node.as_ref()? {
        N::List(l) => strings(&l.items)?,
        N::String(s) => vec![s.sval.as_str()],
        N::ObjectWithArgs(o) => strings(&o.objname)?,
        N::TypeName(t) => strings(&t.names)?,
        _ => return None,
    };
    Some(qualified(&parts))
}

// -- constant folding (`alg/13` SAFE-07 step 1) ------------------------------------------------

/// A constant value.
#[derive(Debug, Clone, PartialEq)]
enum Val {
    Bool(bool),
    Int(i64),
    Num(f64),
    Str(String),
    Null,
}

/// Whether `w` is true for every row: it folds to the constant `true` (`1=1`, `TRUE`,
/// `'a'='a'`, `NULL IS NULL`, `NOT FALSE`, `x OR 1=1`, …).
fn always_true(w: &pb::Node) -> bool {
    eval(w) == Some(Val::Bool(true))
}

fn eval(n: &pb::Node) -> Option<Val> {
    use pb::node::Node as N;
    match n.node.as_ref()? {
        N::AConst(c) => {
            if c.isnull {
                return Some(Val::Null);
            }
            match c.val.as_ref()? {
                pb::a_const::Val::Ival(i) => Some(Val::Int(i64::from(i.ival))),
                pb::a_const::Val::Fval(f) => f.fval.parse().ok().map(Val::Num),
                pb::a_const::Val::Boolval(b) => Some(Val::Bool(b.boolval)),
                pb::a_const::Val::Sval(s) => Some(Val::Str(s.sval.clone())),
                pb::a_const::Val::Bsval(_) => None,
            }
        }
        N::TypeCast(t) => {
            let names = strings(&t.type_name.as_ref()?.names)?;
            if !matches!(
                names.as_slice(),
                ["bool" | "boolean"] | ["pg_catalog", "bool"]
            ) {
                return None;
            }
            match eval(t.arg.as_deref()?)? {
                Val::Bool(b) => Some(Val::Bool(b)),
                Val::Null => Some(Val::Null),
                Val::Str(s) => parse_bool(&s).map(Val::Bool),
                Val::Int(_) | Val::Num(_) => None,
            }
        }
        N::BoolExpr(b) => {
            let args: Vec<Option<Val>> = b.args.iter().map(eval).collect();
            match pb::BoolExprType::try_from(b.boolop).ok()? {
                pb::BoolExprType::AndExpr => {
                    if args.iter().all(|a| *a == Some(Val::Bool(true))) {
                        Some(Val::Bool(true))
                    } else if args.contains(&Some(Val::Bool(false))) {
                        Some(Val::Bool(false))
                    } else {
                        None
                    }
                }
                pb::BoolExprType::OrExpr => {
                    if args.contains(&Some(Val::Bool(true))) {
                        Some(Val::Bool(true))
                    } else if args.iter().all(|a| *a == Some(Val::Bool(false))) {
                        Some(Val::Bool(false))
                    } else {
                        None
                    }
                }
                pb::BoolExprType::NotExpr => match args.as_slice() {
                    [Some(Val::Bool(v))] => Some(Val::Bool(!v)),
                    _ => None,
                },
                pb::BoolExprType::Undefined => None,
            }
        }
        N::AExpr(e) if e.kind == pb::AExprKind::AexprOp as i32 => {
            let op = match strings(&e.name)?.as_slice() {
                [op] => *op,
                _ => return None,
            };
            compare(op, eval(e.lexpr.as_deref()?)?, eval(e.rexpr.as_deref()?)?)
        }
        N::NullTest(t) => {
            let v = eval(t.arg.as_deref()?)?;
            match pb::NullTestType::try_from(t.nulltesttype).ok()? {
                pb::NullTestType::IsNull => Some(Val::Bool(v == Val::Null)),
                pb::NullTestType::IsNotNull => Some(Val::Bool(v != Val::Null)),
                pb::NullTestType::Undefined => None,
            }
        }
        N::BooleanTest(t) => {
            use pb::BoolTestType as B;
            let v = match eval(t.arg.as_deref()?)? {
                Val::Bool(b) => Some(b),
                Val::Null => None,
                _ => return None,
            };
            Some(Val::Bool(match B::try_from(t.booltesttype).ok()? {
                B::IsTrue => v == Some(true),
                B::IsNotTrue => v != Some(true),
                B::IsFalse => v == Some(false),
                B::IsNotFalse => v != Some(false),
                B::IsUnknown => v.is_none(),
                B::IsNotUnknown => v.is_some(),
                B::Undefined => return None,
            }))
        }
        _ => None,
    }
}

/// PostgreSQL's boolean input spellings.
fn parse_bool(s: &str) -> Option<bool> {
    let s = s.trim();
    let is = |w: &str| s.eq_ignore_ascii_case(w);
    if ["t", "true", "y", "yes", "on", "1"].into_iter().any(is) {
        Some(true)
    } else if ["f", "false", "n", "no", "off", "0"].into_iter().any(is) {
        Some(false)
    } else {
        None
    }
}

/// A comparison of two constants; `NULL` compares to `NULL` (unknown, never true).
fn compare(op: &str, a: Val, b: Val) -> Option<Val> {
    use std::cmp::Ordering;
    let ord: Option<Ordering> = match (&a, &b) {
        (Val::Null, _) | (_, Val::Null) => return Some(Val::Null),
        (Val::Int(x), Val::Int(y)) => Some(x.cmp(y)),
        (Val::Int(x), Val::Num(y)) => (*x as f64).partial_cmp(y),
        (Val::Num(x), Val::Int(y)) => x.partial_cmp(&(*y as f64)),
        (Val::Num(x), Val::Num(y)) => x.partial_cmp(y),
        (Val::Str(x), Val::Str(y)) => Some(x.cmp(y)),
        (Val::Bool(x), Val::Bool(y)) => Some(x.cmp(y)),
        _ => None,
    };
    let ord = ord?;
    let v = match op {
        "=" => ord == Ordering::Equal,
        "<>" | "!=" => ord != Ordering::Equal,
        "<" => ord == Ordering::Less,
        ">" => ord == Ordering::Greater,
        "<=" => ord != Ordering::Greater,
        ">=" => ord != Ordering::Less,
        _ => return None,
    };
    Some(Val::Bool(v))
}

/// Whether `w` may refer to the target `rv`: an unqualified column (it could be the target's),
/// a column qualified by the target's alias or name, or `CURRENT OF`.
fn references(w: &pb::Node, rv: &pb::RangeVar) -> bool {
    struct Refs<'n> {
        names: [&'n str; 2],
        found: bool,
    }
    impl<'a> Visitor<'a> for Refs<'_> {
        fn enter(&mut self, r: PgRef<'a>) -> Walk {
            match r {
                PgRef::ColumnRef(c) => {
                    let parts: Vec<Option<&str>> = c
                        .fields
                        .iter()
                        .map(|f| match &f.node {
                            Some(pb::node::Node::String(s)) => Some(s.sval.as_str()),
                            _ => None,
                        })
                        .collect();
                    let qualifier = parts.len().checked_sub(2).and_then(|i| parts[i]);
                    match qualifier {
                        None if parts.len() < 2 => self.found = true,
                        Some(q) if self.names.iter().any(|n| !n.is_empty() && *n == q) => {
                            self.found = true;
                        }
                        _ => {}
                    }
                }
                PgRef::CurrentOfExpr(_) => self.found = true,
                _ => {}
            }
            Walk::Children
        }
    }
    let alias = rv.alias.as_ref().map_or("", |a| a.aliasname.as_str());
    let mut refs = Refs {
        names: [alias, rv.relname.as_str()],
        found: false,
    };
    g::walk(PgRef::Node(w), &mut refs);
    refs.found
}

// ---------------------------------------------------------------------------------------------
// Test hooks

/// The PG backend's hooks for `tests/pg_backend.rs` and `tests/pg_coverage.rs` (re-exported,
/// hidden, from the crate root as `pg_backend_hooks`). No `pg_query` type crosses them.
pub mod hooks {
    use std::collections::{BTreeMap, BTreeSet};

    use smallvec::smallvec;
    use sql_dialect::lex_profile;
    use sql_lexer::LexState;
    use sql_split::SplitMode;
    use sql_text::TextRange;

    use super::{Role, role, table};
    use crate::classify::ClassifyCtx;
    use crate::fingerprint::classified_hash;
    use crate::reason::{Reason, ReasonCode};
    use crate::report::{SafetyReport, StmtReport, merge};
    use crate::visit::pg_generated::{self as g, MsgKind, PgRef, Visitor, Walk};

    /// `sql` split in execution mode, each statement through the PG backend, merged as the
    /// orchestrator merges (no L0 checks, no pre-pass: Task 14's).
    // ## m3b-t14-orchestrator (named exception, fix round 1, I2): the harness entries run on a
    // classifier stack, as `classify` does.
    pub fn classify_pg(sql: &str, ctx: &ClassifyCtx) -> SafetyReport {
        crate::classify::on_classifier_stack(
            || classify_pg_here(sql, ctx),
            || crate::classify::unknown_report(sql, ctx, ReasonCode::EffectsUnknown),
        )
    }

    fn classify_pg_here(sql: &str, ctx: &ClassifyCtx) -> SafetyReport {
        let items: Vec<TextRange> = match lex_profile(&ctx.dialect) {
            Ok(profile) => {
                let lexed = sql_lexer::lex(sql, profile, ctx.dialect.modes, &LexState::Initial);
                sql_split::split(sql, &lexed, &ctx.dialect, ctx.flavor, SplitMode::Execution)
                    .statements()
                    .map(|s| s.strict)
                    .collect()
            }
            Err(_) => Vec::new(),
        };
        let reports = analyse_pg(sql, &items, ctx);
        let (request_class, request_effects, flags) = merge(&reports);
        SafetyReport {
            pre_decision: reports
                .is_empty()
                .then(|| vec![Reason::new(ReasonCode::NoStatement)]),
            items: reports,
            request_class,
            request_effects,
            flags,
            fingerprint: sql_split::hv_norm1(sql, &ctx.dialect),
            backends: smallvec![super::BACKEND],
            classified_len: u32::try_from(sql.len()).unwrap_or(u32::MAX),
            classified_hash: classified_hash(sql),
            dialect: Some(ctx.dialect.id),
        }
    }

    /// The backend on explicit items.
    pub fn analyse_pg(sql: &str, items: &[TextRange], ctx: &ClassifyCtx) -> Vec<StmtReport> {
        crate::classify::on_classifier_stack(
            || super::analyse(sql, items, ctx),
            || {
                items
                    .iter()
                    .map(|r| super::unknown_report(*r, &[ReasonCode::EffectsUnknown]))
                    .collect()
            },
        )
    }

    /// How many `Node`s of each oneof variant (prost's name) the generated walk visits in the
    /// parse tree of `sql`.
    pub fn visit_counts(sql: &str) -> Result<BTreeMap<&'static str, usize>, String> {
        struct Count(BTreeMap<&'static str, usize>);
        impl<'a> Visitor<'a> for Count {
            fn enter(&mut self, r: PgRef<'a>) -> Walk {
                if let PgRef::Node(n) = r
                    && let Some(v) = g::node_variant(n)
                {
                    *self.0.entry(v).or_default() += 1;
                }
                Walk::Children
            }
        }
        let tree = sql_oracle_pg::tree::parse_tree(sql).map_err(|e| e.message)?;
        let mut c = Count(BTreeMap::new());
        g::walk(PgRef::ParseResult(&tree), &mut c);
        Ok(c.0)
    }

    /// How many messages of every kind the generated walk visits in the parse tree of `sql`,
    /// the root included (fix round 1, M8).
    pub fn visit_total(sql: &str) -> Result<usize, String> {
        struct Count(usize);
        impl<'a> Visitor<'a> for Count {
            fn enter(&mut self, _r: PgRef<'a>) -> Walk {
                self.0 += 1;
                Walk::Children
            }
        }
        let tree = sql_oracle_pg::tree::parse_tree(sql).map_err(|e| e.message)?;
        let mut c = Count(0);
        g::walk(PgRef::ParseResult(&tree), &mut c);
        Ok(c.0)
    }

    /// The message kinds in the parse tree of `sql` whose role is `Unhandled`.
    pub fn unhandled_kinds(sql: &str) -> Result<BTreeSet<&'static str>, String> {
        struct Seen(BTreeSet<&'static str>);
        impl<'a> Visitor<'a> for Seen {
            fn enter(&mut self, r: PgRef<'a>) -> Walk {
                if role(r.kind()) == Role::Unhandled {
                    self.0.insert(r.kind().name());
                }
                Walk::Children
            }
        }
        let tree = sql_oracle_pg::tree::parse_tree(sql).map_err(|e| e.message)?;
        let mut s = Seen(BTreeSet::new());
        g::walk(PgRef::ParseResult(&tree), &mut s);
        Ok(s.0)
    }

    /// The `Node` oneof variants (prost's names) whose message has the role `Unhandled`.
    pub fn unhandled_node_variants() -> BTreeSet<&'static str> {
        g::NODE_VARIANTS
            .iter()
            .filter(|(_, k)| role(*k) == Role::Unhandled)
            .map(|(v, _)| *v)
            .collect()
    }

    /// Every `Node` oneof variant (prost's names).
    pub fn node_variants() -> BTreeSet<&'static str> {
        g::NODE_VARIANTS.iter().map(|(v, _)| *v).collect()
    }

    /// The statement table's node names; `None` when it does not load.
    pub fn statement_rows() -> Option<BTreeSet<&'static str>> {
        table().map(|t| t.keys().map(|k| k.name()).collect())
    }

    /// The message kinds whose role is `Stmt`.
    pub fn statement_messages() -> BTreeSet<&'static str> {
        MsgKind::ALL
            .iter()
            .filter(|k| role(**k) == Role::Stmt)
            .map(|k| k.name())
            .collect()
    }

    /// Every message name of the schema.
    pub fn message_names() -> BTreeSet<&'static str> {
        MsgKind::ALL.iter().map(|k| k.name()).collect()
    }

    /// Whether `text` loads as a statement table (the loader's error otherwise).
    pub fn load_table(text: &str) -> Result<usize, String> {
        super::load(text).map(|t| t.len())
    }

    /// The first line of the generated visitor (`// proto-digest: xxh3-64:<hex>`).
    pub fn generated_digest_line() -> &'static str {
        include_str!("../visit/pg_generated.rs")
            .lines()
            .next()
            .unwrap_or("")
    }
}
