//! The lexer pre-scan inventory that runs before any parse (Task 10; P-M3b-8, P-M3b-35).
//!
//! Everything here reads our own lexer's tokens, never `sqlparser`'s, so what it finds holds
//! whatever the parser makes of the text. The interim backend unions it with the parse
//! (`alg/13` SQL-SAFE-02: "both must agree, union effects, fail closed"):
//!
//! - **Inventory** ([`inventory`]): `Word(` calls whose name is on the deny list (any schema
//!   or quoting), `INTO OUTFILE|DUMPFILE|@var`, `FOR UPDATE|SHARE`, `LOCK IN SHARE MODE`,
//!   `:=`, `HANDLER`, `XA`, `LOAD DATA|XML`, and on MySQL and MariaDB the MariaDB-only forms
//!   `sqlparser` accepts or misreads: `NEXT|PREVIOUS VALUE FOR`, `NEXTVAL(`, `LASTVAL(`,
//!   `SETVAL(`, `seq.nextval` (SEQUENCE), `RETURNING` (WRITE_DATA), `FOR SYSTEM_TIME` (Unknown) and
//!   `BEGIN NOT ATOMIC` (procedural code). On SQLite: `ATTACH`, `DETACH`, `VACUUM … INTO`,
//!   `REINDEX`. Client commands (`DELIMITER`, `SOURCE`, `\…`) are flagged.
//! - **Hints** ([`hints`]): `SET_VAR(<var> = …)` and `MAX_EXECUTION_TIME(<n>)` in `/*+ … */`.
//! - **`SET`** ([`set_statement`]): the MySQL/MariaDB `SET` grammar, one target per
//!   assignment with its name and value as written, and the special forms (`NAMES`,
//!   `CHARACTER SET`, `PASSWORD`, `ROLE`, `TRANSACTION`, …).
//! - **`PRAGMA`** ([`pragma`]): SQLite's, read only for `sqlite_read_pragmas()`.
//! - **Narrow read grammars** ([`narrow_show`], [`narrow_describe`], [`narrow_explain`]) for
//!   the MySQL/MariaDB `SHOW`, `DESCRIBE` and `EXPLAIN` forms `sqlparser` rejects (P-M3b-35):
//!   no function call, no subquery, no variable, else nothing (the statement stays a parse
//!   error).
//! - **Running reading** ([`running_close`]) of an executable comment whose end is ambiguous
//!   (ledger 126/138).
//!
//! Keyword matching is ASCII case-insensitive on the token text (INV-3): `SELECT`'s Turkish
//! look-alikes (`SELECT` spelled with U+0130) are never keywords.

use std::sync::Arc;

use smallvec::SmallVec;
use sql_dialect::{DialectId, LexModes};
use sql_ident::{Ident, QualifiedName};
use sql_lexer::{Lexed, TokenKind};
use sql_text::{TextRange, TextSize};

use crate::classify::ClassifyCtx;
use crate::destructive::DestructiveReason;
use crate::effects::{EffectSet, RiskFlags};
use crate::functions::{DenyGroup, FnClass, FnLookup, lookup_fn};
use crate::reason::{Reason, ReasonCode};
use crate::report::{ObjectKind, ObjectRef, SetTarget, StmtKind, TouchedObjects};

/// The evidence gathered for one reading of one statement.
#[derive(Debug, Clone, Default)]
pub(crate) struct Acc {
    /// The statement's kind, when the parse or a grammar named it.
    pub(crate) kind: Option<StmtKind>,
    /// A kind the pre-scan alone saw (`LOAD DATA`, `HANDLER`, `DO`), used when no parse names
    /// one.
    pub(crate) kind_hint: Option<StmtKind>,
    pub(crate) effects: EffectSet,
    pub(crate) flags: RiskFlags,
    pub(crate) reasons: Vec<Reason>,
    pub(crate) destructive: Vec<DestructiveReason>,
    pub(crate) touched: TouchedObjects,
    /// `SET` targets with the range of the name (one target per name range, across views).
    pub(crate) set_targets: Vec<(SetTarget, TextRange)>,
    /// A catalog, an allowlist or an exec-comment reading was relied on.
    pub(crate) conservative: bool,
    // ## m4-t06-ddl-effects
    /// The catalog effects the parse of this reading names (`backend::ddl_effects::interim`);
    /// read by no verdict path.
    pub(crate) ddl: Vec<crate::report::DdlEffect>,
}

impl Acc {
    pub(crate) fn reason(
        &mut self,
        code: ReasonCode,
        range: Option<TextRange>,
        arg: Option<Arc<str>>,
    ) {
        self.reasons.push(Reason { code, range, arg });
    }

    /// Doubt: the effects cannot be proved. `UNKNOWN` and the code, never the class alone.
    pub(crate) fn doubt(&mut self, code: ReasonCode, range: Option<TextRange>) {
        self.effects |= EffectSet::UNKNOWN;
        self.reason(code, range, None);
    }

    pub(crate) fn add(&mut self, effects: EffectSet, code: ReasonCode, range: Option<TextRange>) {
        self.effects |= effects;
        self.reason(code, range, None);
    }

    /// Adds `other`'s evidence (another reading of the same statement).
    pub(crate) fn absorb(&mut self, other: Acc) {
        self.kind = self.kind.or(other.kind);
        self.kind_hint = self.kind_hint.or(other.kind_hint);
        self.effects |= other.effects;
        self.flags |= other.flags;
        self.reasons.extend(other.reasons);
        self.destructive.extend(other.destructive);
        self.touched.reads.extend(other.touched.reads);
        self.touched.writes.extend(other.touched.writes);
        self.touched.calls.extend(other.touched.calls);
        self.set_targets.extend(other.set_targets);
        self.conservative |= other.conservative;
        // ## m4-t06-ddl-effects
        self.ddl.extend(other.ddl);
    }
}

// ---------------------------------------------------------------------------------------------
// Tokens

/// The significant (non-trivia) tokens of one lexed text.
pub(crate) struct Toks<'a> {
    pub(crate) text: &'a str,
    pub(crate) lexed: &'a Lexed,
    /// Indices into `lexed` of the tokens that are not whitespace or comments.
    pub(crate) sig: Vec<usize>,
}

impl<'a> Toks<'a> {
    pub(crate) fn new(text: &'a str, lexed: &'a Lexed) -> Self {
        let sig = (0..lexed.len()).filter(|&i| !lexed.is_trivia(i)).collect();
        Toks { text, lexed, sig }
    }

    pub(crate) fn len(&self) -> usize {
        self.sig.len()
    }

    pub(crate) fn kind(&self, j: usize) -> Option<TokenKind> {
        self.sig.get(j).map(|&i| self.lexed.kind(i))
    }

    pub(crate) fn range(&self, j: usize) -> TextRange {
        self.lexed.range(self.sig[j])
    }

    pub(crate) fn start(&self, j: usize) -> usize {
        usize::from(self.range(j).start())
    }

    pub(crate) fn end(&self, j: usize) -> usize {
        usize::from(self.range(j).end())
    }

    pub(crate) fn text(&self, j: usize) -> &'a str {
        let r = self.range(j);
        &self.text[usize::from(r.start())..usize::from(r.end())]
    }

    /// Whether token `j` is the word `kw` (ASCII case-insensitive, whole).
    pub(crate) fn is(&self, j: usize, kw: &str) -> bool {
        self.kind(j) == Some(TokenKind::Word) && self.text(j).eq_ignore_ascii_case(kw)
    }

    /// Whether token `j` is one of `kws`.
    pub(crate) fn is_any(&self, j: usize, kws: &[&str]) -> bool {
        kws.iter().any(|k| self.is(j, k))
    }

    pub(crate) fn is_op(&self, j: usize, op: &str) -> bool {
        self.kind(j) == Some(TokenKind::Op) && self.text(j) == op
    }

    /// Whether token `j` is a name: a word or a quoted identifier.
    pub(crate) fn is_name(&self, j: usize) -> bool {
        matches!(
            self.kind(j),
            Some(TokenKind::Word | TokenKind::QuotedIdent | TokenKind::UnicodeQuotedIdent)
        )
    }

    /// Token `j`'s name text with its quotes removed (doubled quotes undoubled).
    pub(crate) fn name(&self, j: usize) -> (Arc<str>, bool) {
        let t = self.text(j);
        match self.kind(j) {
            Some(TokenKind::QuotedIdent) if t.len() >= 2 => {
                let open = t.as_bytes()[0];
                let close = if open == b'[' { b']' } else { open };
                let inner = &t[1..t.len() - 1];
                let doubled = [close as char, close as char].iter().collect::<String>();
                let single = (close as char).to_string();
                (Arc::from(inner.replace(&doubled, &single)), true)
            }
            _ => (Arc::from(t), false),
        }
    }

    /// The range from token `a`'s start to token `b`'s end.
    pub(crate) fn span(&self, a: usize, b: usize) -> TextRange {
        TextRange::new(self.range(a).start(), self.range(b).end())
    }

    /// The range of every significant token.
    pub(crate) fn whole(&self) -> Option<TextRange> {
        (!self.sig.is_empty()).then(|| self.span(0, self.sig.len() - 1))
    }
}

fn arc(s: &str) -> Arc<str> {
    Arc::from(s)
}

fn is_mysql_family(d: DialectId) -> bool {
    matches!(d, DialectId::MySql | DialectId::MariaDb)
}

// ---------------------------------------------------------------------------------------------
// Function calls (SAFE-03)

/// What a denied function group does (`alg/13` SAFE-03 deny catalogue): its effect bits, the
/// reason naming that group and any risk flag. `Sleep` has no bit in M3b (`RESOURCE_HEAVY`
/// is not modelled, ADR 0005 `m3b-t04`): the `FnDenylisted` reason carries it.
pub(crate) fn deny_effects(g: DenyGroup) -> (EffectSet, Option<ReasonCode>, RiskFlags) {
    use EffectSet as E;
    use ReasonCode as R;
    match g {
        DenyGroup::Sleep => (E::empty(), None, RiskFlags::empty()),
        DenyGroup::FileIo => (E::EXTERNAL_IO, Some(R::ExternalFile), RiskFlags::empty()),
        DenyGroup::Network => (E::EXTERNAL_IO, Some(R::ExternalNetwork), RiskFlags::empty()),
        DenyGroup::ProcessExec => (E::PROCESS_EXEC, Some(R::ProcessExec), RiskFlags::empty()),
        DenyGroup::AdminSignal | DenyGroup::Replication => {
            (E::ADMIN, Some(R::AdminControl), RiskFlags::empty())
        }
        DenyGroup::LargeObject => (
            E::WRITE_DATA | E::EXTERNAL_IO,
            Some(R::ExternalFile),
            RiskFlags::empty(),
        ),
        DenyGroup::DynamicSql => (E::UNKNOWN, Some(R::DynamicSql), RiskFlags::DYNAMIC_SQL),
        DenyGroup::Advisory => (E::ADVISORY_LOCK, Some(R::AdvisoryLock), RiskFlags::empty()),
        DenyGroup::Sequence => (E::SEQUENCE, Some(R::SequenceAdvance), RiskFlags::empty()),
        DenyGroup::XidConsume => (E::XID_CONSUME, Some(R::XidConsume), RiskFlags::empty()),
        DenyGroup::SetConfig => (E::SESSION_STATE, Some(R::SessionState), RiskFlags::empty()),
    }
}

/// `name` as written, parts joined by `.`.
pub(crate) fn shown(name: &QualifiedName) -> Arc<str> {
    let parts: Vec<&str> = name.parts.iter().map(|p| &*p.text).collect();
    Arc::from(parts.join("."))
}

/// Whether `name` is on the profile's read-only function allowlist (D-009, P-M3b-10): an
/// entry equal to the name as written (parts joined by `.`), ASCII case-insensitively (the
/// non-PostgreSQL engines resolve function names case-insensitively; INV-3).
fn allowlisted(ctx: &ClassifyCtx, name: &QualifiedName) -> bool {
    let full = shown(name);
    ctx.fn_allowlist
        .iter()
        .any(|e| e.eq_ignore_ascii_case(&full))
}

/// Assesses one call of `name` (SAFE-03): the deny list first (by bare name, any schema or
/// quoting), then the reviewed builtins, the profile's allowlist and the catalog. With
/// `deny_only`, only a denial is recorded (the pre-scan's `Word(` inventory: the parse
/// decides the rest).
pub(crate) fn assess_call(
    name: &QualifiedName,
    range: Option<TextRange>,
    arity: Option<u16>,
    ctx: &ClassifyCtx,
    deny_only: bool,
    acc: &mut Acc,
) {
    let label = || shown(name);
    match lookup_fn(ctx.dialect.id, name, ctx.dialect.version) {
        FnClass::Denied(g) => {
            let (effects, code, flags) = deny_effects(g);
            acc.effects |= effects;
            acc.flags |= flags;
            acc.reason(ReasonCode::FnDenylisted, range, Some(label()));
            if let Some(code) = code {
                acc.reason(code, range, Some(label()));
            }
        }
        _ if deny_only => return,
        FnClass::Allowed => {}
        FnClass::Unknown if allowlisted(ctx, name) => acc.conservative = true,
        FnClass::Unknown => match ctx.catalog.as_ref().map(|c| c.lookup(name, arity)) {
            Some(FnLookup::Known { writes: true, .. }) => {
                acc.doubt(ReasonCode::FnUnknown, range);
                if let Some(r) = acc.reasons.last_mut() {
                    r.arg = Some(label());
                }
            }
            Some(FnLookup::Known { volatile: true, .. }) => {
                // S-04: a volatile function no allowlist covers is Unknown.
                acc.flags |= RiskFlags::VOLATILE_FN;
                acc.effects |= EffectSet::UNKNOWN;
                acc.reason(ReasonCode::VolatileFunction, range, Some(label()));
            }
            Some(FnLookup::Known { .. }) => acc.conservative = true,
            Some(FnLookup::Unknown) | None => {
                acc.effects |= EffectSet::UNKNOWN;
                acc.reason(ReasonCode::FnUnknown, range, Some(label()));
            }
        },
    }
    acc.touched.calls.push(ObjectRef {
        name: name.clone(),
        kind: ObjectKind::Function,
    });
}

/// Whether token `j`'s bare name (quotes removed, ASCII lower case) is on any deny list (a
/// dialect without data of its own reads every list). A superset check before the full
/// `lookup_fn`, which decides.
fn may_be_denied(t: &Toks<'_>, j: usize) -> bool {
    use std::collections::HashSet;
    use std::sync::OnceLock;
    static ALL: OnceLock<HashSet<&'static str>> = OnceLock::new();
    let all =
        ALL.get_or_init(|| crate::functions::denied_function_names(DialectId::Generic).collect());
    let (name, _) = t.name(j);
    all.contains(name.to_ascii_lowercase().as_str())
}

/// The qualified name of a call whose name ends at token `j` (followed by `(`), with the
/// range of the name.
fn call_name(t: &Toks<'_>, j: usize) -> (QualifiedName, TextRange) {
    let mut first = j;
    while first >= 2 && t.kind(first - 1) == Some(TokenKind::Dot) && t.is_name(first - 2) {
        first -= 2;
    }
    let mut parts: SmallVec<[Ident; 3]> = SmallVec::new();
    let mut k = first;
    while k <= j {
        let (text, quoted) = t.name(k);
        parts.push(Ident { text, quoted });
        k += 2;
    }
    (QualifiedName { parts }, t.span(first, j))
}

// ---------------------------------------------------------------------------------------------
// Lexical doubt

/// MySQL client commands that may stand at the start of a statement (mysql(1) "mysql Client
/// Commands"); `USE`, `HELP` and `STATUS`-like words that are also SQL are left to the parse.
const MYSQL_CLIENT_WORDS: [&str; 8] = [
    "DELIMITER",
    "SOURCE",
    "SYSTEM",
    "CONNECT",
    "PROMPT",
    "TEE",
    "PAGER",
    "REHASH",
];

/// Doubt the lexer alone can see in a reading: unreadable tokens, client commands and
/// client variables. (A MariaDB `/*M!` comment on MySQL is read as an executable comment by
/// the backend's views, which set `EXEC_COMMENT`.)
pub(crate) fn lexical(t: &Toks<'_>, d: DialectId, acc: &mut Acc) {
    let lexed = t.lexed;
    for i in 0..lexed.len() {
        let r = lexed.range(i);
        match lexed.kind(i) {
            TokenKind::Unknown => acc.doubt(ReasonCode::LexError, Some(r)),
            TokenKind::ClientCommand | TokenKind::ClientVar => {
                acc.doubt(ReasonCode::ClientCommand, Some(r))
            }
            _ => {}
        }
    }
    if t.len() == 0 {
        return;
    }
    let head = t.text(0);
    let client = match d {
        DialectId::MySql | DialectId::MariaDb => {
            t.is_any(0, &MYSQL_CLIENT_WORDS) || head.starts_with('\\')
        }
        // sqlite3's dot commands stand at a line start.
        DialectId::Sqlite => t.kind(0) == Some(TokenKind::Dot) && line_start(t.text, t.start(0)),
        _ => false,
    };
    if client {
        acc.doubt(ReasonCode::ClientCommand, Some(t.range(0)));
    }
}

fn line_start(text: &str, at: usize) -> bool {
    text[..at]
        .bytes()
        .rev()
        .take_while(|b| *b != b'\n')
        .all(|b| b == b' ' || b == b'\t' || b == b'\r')
}

// ---------------------------------------------------------------------------------------------
// The inventory

/// A MySQL/MariaDB `SHOW` that discloses other sessions' activity, other accounts or the
/// server's replication, binary-log and engine state (controller rulings, fix rounds 1 and 2):
///
/// - `SHOW [FULL] PROCESSLIST`; `SHOW ENGINE …`;
/// - `SHOW {REPLICA|SLAVE|MASTER} ['<connection>'] … STATUS …`, `SHOW ALL {REPLICAS|SLAVES}
///   STATUS`, `SHOW BINARY LOG STATUS`, `SHOW REPLICAS`, `SHOW SLAVE HOSTS`;
/// - `SHOW {BINARY|MASTER} LOGS`, `SHOW {BINLOG|RELAYLOG} EVENTS`;
/// - MariaDB `SHOW {EXPLAIN|ANALYZE} [FORMAT=…] FOR <connection>` (every `SHOW EXPLAIN|ANALYZE`
///   form names another connection), `SHOW {USER|CLIENT}_STATISTICS`;
/// - `SHOW GRANTS FOR` and `SHOW CREATE USER` of another account (`CURRENT_USER` and
///   `CURRENT_USER()` are one's own).
///
/// Returns the form's name. Defence in depth: the server's PROCESS, SUPER and REPLICATION
/// privileges decide what it shows.
fn info_disclosure(t: &Toks<'_>) -> Option<&'static str> {
    if !t.is(0, "SHOW") {
        return None;
    }
    let j = if t.is(1, "FULL") { 2 } else { 1 };
    if t.is(j, "PROCESSLIST") {
        return Some("SHOW PROCESSLIST");
    }
    // `SHOW ENGINES` (the list of engines) is a plain read; `SHOW ENGINE x …` is not.
    if t.is(1, "ENGINE") {
        return Some("SHOW ENGINE");
    }
    // `SHOW SLAVE 'conn' STATUS`, `SHOW REPLICA STATUS FOR CHANNEL c`, `SHOW MASTER STATUS`.
    if t.is_any(1, &["REPLICA", "SLAVE", "MASTER"]) && (2..t.len()).any(|k| t.is(k, "STATUS")) {
        return Some("SHOW REPLICA STATUS");
    }
    if t.is(1, "ALL") && t.is_any(2, &["REPLICAS", "SLAVES"]) && t.is(3, "STATUS") {
        return Some("SHOW ALL REPLICAS STATUS");
    }
    if t.is(1, "BINARY") && t.is(2, "LOG") && t.is(3, "STATUS") {
        return Some("SHOW BINARY LOG STATUS");
    }
    if t.is_any(1, &["BINARY", "MASTER"]) && t.is(2, "LOGS") {
        return Some("SHOW BINARY LOGS");
    }
    if (t.is(1, "REPLICAS") && t.len() == 2) || (t.is(1, "SLAVE") && t.is(2, "HOSTS")) {
        return Some("SHOW REPLICAS");
    }
    if t.is_any(1, &["EXPLAIN", "ANALYZE"]) {
        return Some("SHOW EXPLAIN FOR");
    }
    if t.is_any(1, &["BINLOG", "RELAYLOG"]) && t.is(2, "EVENTS") {
        return Some("SHOW BINLOG EVENTS");
    }
    if t.is_any(1, &["USER_STATISTICS", "CLIENT_STATISTICS"]) {
        return Some("SHOW USER_STATISTICS");
    }
    if t.is(1, "GRANTS") && t.is(2, "FOR") && !own_account(t, 3) {
        return Some("SHOW GRANTS FOR");
    }
    if t.is(1, "CREATE") && t.is(2, "USER") && !own_account(t, 3) {
        return Some("SHOW CREATE USER");
    }
    None
}

/// Whether the account named from token `j` to the end is one's own: `CURRENT_USER` or
/// `CURRENT_USER()`.
fn own_account(t: &Toks<'_>, j: usize) -> bool {
    t.is(j, "CURRENT_USER")
        && (t.len() == j + 1
            || (t.len() == j + 3
                && t.kind(j + 1) == Some(TokenKind::LParen)
                && t.kind(j + 2) == Some(TokenKind::RParen)))
}

/// The MySQL/MariaDB tables and views that hold other sessions' activity and statement text
/// (controller ruling, fix round 2, RR-I2), as `(schema, name)`, ASCII lower case. Matched in
/// any quoting and letter case.
const DISCLOSURE_TABLES: [(&str, &str); 11] = [
    ("information_schema", "processlist"),
    ("information_schema", "user_statistics"),
    ("information_schema", "client_statistics"),
    ("performance_schema", "threads"),
    ("performance_schema", "events_statements_current"),
    ("performance_schema", "events_statements_history"),
    ("performance_schema", "events_statements_history_long"),
    ("sys", "processlist"),
    ("sys", "session"),
    ("sys", "x$processlist"),
    ("sys", "x$session"),
];

/// The names of [`DISCLOSURE_TABLES`] specific enough to match unqualified too (a default
/// database of `information_schema`, `performance_schema` or `sys` reaches them unqualified);
/// `threads` and `session` are common user table names and match only qualified.
const DISCLOSURE_UNQUALIFIED: [&str; 8] = [
    "processlist",
    "user_statistics",
    "client_statistics",
    "events_statements_current",
    "events_statements_history",
    "events_statements_history_long",
    "x$processlist",
    "x$session",
];

// ## m3b-t20-differential (named exception, T10 RR-I2 gap from the T14 re-check)
/// The [`DISCLOSURE_TABLES`] names that match unqualified only when the session's current
/// database is their schema (`ClassifyCtx.search_path[0]` on MySQL/MariaDB, `DATABASE()`), as
/// `(schema, name)`: they are common user-table names, so with another current database, or
/// none known, they stay user tables.
const DISCLOSURE_IN_CURRENT_DB: [(&str, &str); 2] =
    [("performance_schema", "threads"), ("sys", "session")];

/// The first reference in the reading to a [`DISCLOSURE_TABLES`] table: `schema.name` in any
/// quoting and case, or one of [`DISCLOSURE_UNQUALIFIED`] standing alone (not a qualifier, a
/// column of another table, or a call), or one of [`DISCLOSURE_IN_CURRENT_DB`] standing alone
/// when `current_db` is its schema.
fn disclosure_table(t: &Toks<'_>, current_db: Option<&str>) -> Option<(TextRange, String)> {
    let lower = |j: usize| t.name(j).0.to_ascii_lowercase();
    for j in 0..t.len() {
        if !t.is_name(j) {
            continue;
        }
        if t.kind(j + 1) == Some(TokenKind::Dot) && t.is_name(j + 2) {
            let (schema, name) = (lower(j), lower(j + 2));
            if DISCLOSURE_TABLES
                .iter()
                .any(|(s, n)| *s == schema && *n == name)
            {
                return Some((t.span(j, j + 2), format!("{schema}.{name}")));
            }
            continue;
        }
        let qualified_part = j > 0 && t.kind(j - 1) == Some(TokenKind::Dot);
        let call = t.kind(j + 1) == Some(TokenKind::LParen);
        if !qualified_part && !call {
            let name = lower(j);
            if DISCLOSURE_UNQUALIFIED.contains(&name.as_str()) {
                return Some((t.range(j), name));
            }
            // ## m3b-t20-differential
            if let Some(db) = current_db
                && let Some((schema, _)) = DISCLOSURE_IN_CURRENT_DB
                    .iter()
                    .find(|(s, n)| *n == name && db.eq_ignore_ascii_case(s))
            {
                return Some((t.range(j), format!("{schema}.{name}")));
            }
        }
    }
    None
}

/// The pre-scan inventory of one reading (see the module docs).
pub(crate) fn inventory(t: &Toks<'_>, ctx: &ClassifyCtx, acc: &mut Acc) {
    let d = ctx.dialect.id;
    let n = t.len();
    if n == 0 {
        return;
    }
    let is_set = t.is(0, "SET");
    // Heads.
    if is_mysql_family(d) {
        if let Some(what) = info_disclosure(t) {
            // Controller ruling (fix round 1, I3): other sessions' activity and the server's
            // replication and engine state are information disclosure, Unknown in M3b (no
            // INFO_DISCLOSURE bit), so read-only denies them.
            acc.doubt(ReasonCode::EffectsUnknown, t.whole());
            if let Some(r) = acc.reasons.last_mut() {
                r.arg = Some(arc(what));
            }
        }
        // The same data through tables (fix round 2, RR-I2).
        // ## m3b-t20-differential: the current database names the unqualified ones.
        let current_db = ctx.search_path.first().map(|s| &**s);
        if let Some((range, name)) = disclosure_table(t, current_db) {
            acc.doubt(ReasonCode::EffectsUnknown, Some(range));
            if let Some(r) = acc.reasons.last_mut() {
                r.arg = Some(arc(&name));
            }
        }
        if t.is(0, "HANDLER") {
            acc.kind_hint = Some(StmtKind::Other);
            acc.add(
                EffectSet::SESSION_STATE,
                ReasonCode::SessionState,
                t.whole(),
            );
        } else if t.is(0, "XA") {
            acc.kind_hint = Some(StmtKind::Other);
            acc.add(EffectSet::TXN_CONTROL, ReasonCode::TxnControl, t.whole());
            // Two-phase commit by XID can end other sessions' prepared work: opaque.
            acc.doubt(ReasonCode::EffectsUnknown, t.whole());
        } else if t.is(0, "LOAD") && t.is_any(1, &["DATA", "XML"]) {
            acc.kind_hint = Some(StmtKind::Insert);
            acc.add(
                EffectSet::WRITE_DATA | EffectSet::EXTERNAL_IO,
                ReasonCode::ExternalFile,
                t.whole(),
            );
        } else if t.is(0, "DO") {
            acc.kind_hint = Some(StmtKind::Do);
        }
    }
    if d == DialectId::Sqlite {
        if t.is(0, "ATTACH") {
            acc.kind_hint = Some(StmtKind::Other);
            acc.add(
                EffectSet::EXTERNAL_IO | EffectSet::SESSION_STATE,
                ReasonCode::ExternalFile,
                t.whole(),
            );
        } else if t.is(0, "DETACH") {
            acc.kind_hint = Some(StmtKind::Other);
            acc.add(
                EffectSet::SESSION_STATE,
                ReasonCode::SessionState,
                t.whole(),
            );
        } else if t.is(0, "REINDEX") {
            acc.kind_hint = Some(StmtKind::Other);
            acc.add(EffectSet::DDL, ReasonCode::SchemaChange, t.whole());
        } else if t.is(0, "VACUUM") {
            acc.kind_hint = Some(StmtKind::Other);
            acc.add(EffectSet::DDL, ReasonCode::SchemaChange, t.whole());
            if (1..n).any(|j| t.is(j, "INTO")) {
                acc.add(EffectSet::EXTERNAL_IO, ReasonCode::ExternalFile, t.whole());
            }
        } else if t.is(0, "CREATE") && t.is(1, "VIRTUAL") {
            acc.add(
                EffectSet::DDL | EffectSet::EXTERNAL_IO,
                ReasonCode::ExternalFile,
                t.whole(),
            );
        }
    }
    for j in 0..n {
        let r = Some(t.range(j));
        // `Word(` and `"quoted"(`: a call. Only a denial is recorded here (a cheap bare-name
        // check first; `lookup_fn` decides).
        if t.is_name(j) && t.kind(j + 1) == Some(TokenKind::LParen) && may_be_denied(t, j) {
            let (name, range) = call_name(t, j);
            assess_call(&name, Some(range), None, ctx, true, acc);
        }
        if t.kind(j) == Some(TokenKind::Op) && t.text(j) == ":=" && !is_set {
            acc.add(EffectSet::SESSION_STATE, ReasonCode::SessionState, r);
        }
        if t.is(j, "RETURNING") {
            acc.add(EffectSet::WRITE_DATA, ReasonCode::WriteData, r);
        }
        if !is_mysql_family(d) {
            continue;
        }
        if t.is(j, "INTO") {
            if t.is_any(j + 1, &["OUTFILE", "DUMPFILE"]) {
                acc.add(EffectSet::EXTERNAL_IO, ReasonCode::ExternalFile, r);
            } else if t.kind(j + 1) == Some(TokenKind::UserVar) {
                acc.add(EffectSet::SESSION_STATE, ReasonCode::SessionState, r);
            }
        } else if t.is(j, "FOR") {
            if t.is_any(j + 1, &["UPDATE", "SHARE"]) {
                acc.add(EffectSet::LOCK_ROW, ReasonCode::LockingClause, r);
            } else if t.is(j + 1, "SYSTEM_TIME") {
                // MariaDB system-versioned tables: `sqlparser` has no reading of it.
                acc.doubt(ReasonCode::ParserDisagreement, Some(t.span(j, j + 1)));
            }
        } else if t.is(j, "LOCK") && t.is(j + 1, "IN") && t.is(j + 2, "SHARE") {
            acc.add(EffectSet::LOCK_ROW, ReasonCode::LockingClause, r);
        } else if t.is_any(j, &["NEXT", "PREVIOUS"]) && t.is(j + 1, "VALUE") && t.is(j + 2, "FOR") {
            acc.add(
                EffectSet::SEQUENCE,
                ReasonCode::SequenceAdvance,
                Some(t.span(j, j + 2)),
            );
        } else if t.is_any(j, &["NEXTVAL", "LASTVAL", "SETVAL"])
            && t.kind(j + 1) == Some(TokenKind::LParen)
        {
            acc.add(EffectSet::SEQUENCE, ReasonCode::SequenceAdvance, r);
        } else if t.is_name(j)
            && t.name(j).0.eq_ignore_ascii_case("NEXTVAL")
            && j > 0
            && t.kind(j - 1) == Some(TokenKind::Dot)
        {
            // `seq.nextval` and ``seq.`nextval` `` (MariaDB's Oracle-style pseudo-column).
            acc.add(EffectSet::SEQUENCE, ReasonCode::SequenceAdvance, r);
        } else if t.is(j, "BEGIN") && t.is(j + 1, "NOT") && t.is(j + 2, "ATOMIC") {
            acc.doubt(ReasonCode::ProceduralCode, Some(t.span(j, j + 2)));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Optimizer hints

/// `SET_VAR(<var> = …)` and `MAX_EXECUTION_TIME(<n>)` in every `/*+ … */` of the reading
/// (MySQL and MariaDB; wherever it stands, the parse is not trusted to place it). The policy
/// denies a `SET_VAR` whose variable is not harmless and a limit that is not a positive number.
pub(crate) fn hints(t: &Toks<'_>, d: DialectId, acc: &mut Acc) {
    if !is_mysql_family(d) {
        return;
    }
    let lexed = t.lexed;
    for i in 0..lexed.len() {
        if lexed.kind(i) != TokenKind::HintComment {
            continue;
        }
        let r = lexed.range(i);
        let s = &t.text[usize::from(r.start())..usize::from(r.end())];
        let body = s.strip_prefix("/*+").unwrap_or(s);
        let body = body.strip_suffix("*/").unwrap_or(body);
        for (word, args) in hint_calls(body) {
            if word.eq_ignore_ascii_case("SET_VAR") {
                let var = args.split('=').next().map(str::trim).filter(|v| {
                    !v.is_empty() && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                });
                acc.reason(ReasonCode::HintSetVar, Some(r), var.map(arc));
            } else if word.eq_ignore_ascii_case("MAX_EXECUTION_TIME") {
                acc.reason(ReasonCode::HintExecTime, Some(r), Some(arc(args.trim())));
            }
        }
    }
}

/// The `word(args)` items of a hint body; an unclosed item ends the body.
fn hint_calls(body: &str) -> Vec<(&str, &str)> {
    let b = body.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_alphabetic() || b[i] == b'_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let word = &body[start..i];
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < b.len() && b[i] == b'(' {
                let open = i + 1;
                let mut depth = 1;
                i += 1;
                while i < b.len() && depth > 0 {
                    match b[i] {
                        b'(' => depth += 1,
                        b')' => depth -= 1,
                        _ => {}
                    }
                    i += 1;
                }
                let close = if depth == 0 { i - 1 } else { b.len() };
                out.push((word, &body[open..close]));
            }
        } else {
            i += 1;
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// SET (MySQL, MariaDB)

/// Variables whose change alters how the server lexes later statements (`alg/13` step 7).
const LEX_MODE_VARS: [&str; 4] = [
    "sql_mode",
    "character_set_client",
    "character_set_connection",
    "collation_connection",
];

const SCOPES: [&str; 5] = ["GLOBAL", "SESSION", "LOCAL", "PERSIST", "PERSIST_ONLY"];

/// The MySQL/MariaDB `SET` grammar over a reading that starts with `SET`. Each assignment is
/// one [`SetTarget`] with one `SessionState` reason naming it (the producer contract: the two
/// lists match exactly); `GLOBAL`/`PERSIST` scopes are `ADMIN`; a lexing variable is
/// `LEX_MODE_CHANGE`. The special forms carry no target. A `SET` the grammar cannot read
/// keeps the targets it read, is `Unknown` (`ParseError` and `UNKNOWN`), and is `ADMIN` when
/// any `GLOBAL`/`PERSIST`/`PERSIST_ONLY` word or `@@global.`/`@@persist.` scope appears (fix
/// round 1, I1).
pub(crate) fn set_statement(t: &Toks<'_>, acc: &mut Acc) {
    acc.kind = Some(StmtKind::Set);
    let whole = t.whole();
    let j = 1;
    let scope_word = t.is_any(j, &SCOPES) && !t.is_op(j + 1, "=") && !t.is_op(j + 1, ":=");
    let scoped = if scope_word { j + 1 } else { j };
    // The special forms.
    if t.is_any(j, &["NAMES", "CHARSET"]) || (t.is(j, "CHARACTER") && t.is(j + 1, "SET")) {
        acc.effects |= EffectSet::SESSION_STATE;
        acc.flags |= RiskFlags::LEX_MODE_CHANGE;
        acc.reason(ReasonCode::LexModeChange, whole, None);
        acc.reason(ReasonCode::SessionState, whole, None);
        return;
    }
    if t.is(j, "PASSWORD") || (t.is(j, "DEFAULT") && t.is(j + 1, "ROLE")) {
        acc.add(EffectSet::PRIVILEGE, ReasonCode::PrivilegeGrant, whole);
        return;
    }
    if t.is(j, "ROLE") {
        acc.add(EffectSet::PRIVILEGE, ReasonCode::PrivilegeChange, whole);
        return;
    }
    if t.is(j, "RESOURCE") && t.is(j + 1, "GROUP") {
        acc.add(EffectSet::ADMIN, ReasonCode::AdminControl, whole);
        return;
    }
    if t.is(scoped, "TRANSACTION") {
        acc.add(
            EffectSet::TXN_CONTROL | EffectSet::SESSION_STATE,
            ReasonCode::TxnControl,
            whole,
        );
        acc.reason(ReasonCode::SessionState, whole, None);
        if scope_word && t.is_any(j, &["GLOBAL", "PERSIST", "PERSIST_ONLY"]) {
            acc.add(EffectSet::ADMIN, ReasonCode::AdminControl, whole);
        }
        return;
    }
    let mut targets = Vec::new();
    let read = set_targets(t, &mut targets).is_some() && !targets.is_empty();
    // Every target read so far is named, also when the rest of the statement does not read
    // (fix round 1, I1: never a silent fall back to plain session state).
    for (name, value, name_range, global) in targets {
        if global {
            acc.add(EffectSet::ADMIN, ReasonCode::AdminControl, Some(name_range));
        } else {
            acc.effects |= EffectSet::SESSION_STATE;
        }
        if LEX_MODE_VARS.iter().any(|v| name.eq_ignore_ascii_case(v)) {
            acc.flags |= RiskFlags::LEX_MODE_CHANGE;
            acc.reason(
                ReasonCode::LexModeChange,
                Some(name_range),
                Some(name.clone()),
            );
        }
        acc.reason(
            ReasonCode::SessionState,
            Some(name_range),
            Some(name.clone()),
        );
        acc.set_targets.push((
            SetTarget {
                name,
                value: Some(value),
            },
            name_range,
        ));
    }
    if read {
        return;
    }
    // A `SET` the grammar cannot read is Unknown (a doubt reason and `UNKNOWN`), still a
    // session change, and `ADMIN` when any global or persisted scope appears in it.
    acc.effects |= EffectSet::SESSION_STATE;
    acc.doubt(ReasonCode::ParseError, whole);
    let global_scope = (1..t.len()).any(|j| {
        t.is_any(j, &["GLOBAL", "PERSIST", "PERSIST_ONLY"])
            || (t.kind(j) == Some(TokenKind::SystemVar) && {
                let var = &t.text(j)[2..];
                ["GLOBAL", "PERSIST", "PERSIST_ONLY"]
                    .iter()
                    .any(|s| var.eq_ignore_ascii_case(s))
            })
    });
    if global_scope {
        acc.add(EffectSet::ADMIN, ReasonCode::AdminControl, whole);
    }
}

/// One assignment: name as written, value as written, the name's range, whether the scope is
/// global (`GLOBAL`, `PERSIST`, `PERSIST_ONLY`, `@@global.`, `@@persist.`).
type Assignment = (Arc<str>, Arc<str>, TextRange, bool);

fn set_targets(t: &Toks<'_>, out: &mut Vec<Assignment>) -> Option<()> {
    let n = t.len();
    let mut j = 1;
    loop {
        let mut global = false;
        if t.is_any(j, &SCOPES) && !t.is_op(j + 1, "=") && !t.is_op(j + 1, ":=") {
            global = t.is_any(j, &["GLOBAL", "PERSIST", "PERSIST_ONLY"]);
            j += 1;
        }
        let (name, name_range) = match t.kind(j)? {
            TokenKind::SystemVar => {
                let var = &t.text(j)[2..];
                let scope = SCOPES.iter().any(|s| var.eq_ignore_ascii_case(s));
                if scope && t.kind(j + 1) == Some(TokenKind::Dot) && t.is_name(j + 2) {
                    global |= ["GLOBAL", "PERSIST", "PERSIST_ONLY"]
                        .iter()
                        .any(|s| var.eq_ignore_ascii_case(s));
                    // Component variables are dotted (`@@global.validate_password.length`).
                    let first = j + 2;
                    let mut parts = vec![t.name(first).0];
                    j += 3;
                    while t.kind(j) == Some(TokenKind::Dot) && t.is_name(j + 1) {
                        parts.push(t.name(j + 1).0);
                        j += 2;
                    }
                    let joined: Vec<&str> = parts.iter().map(|p| &**p).collect();
                    (arc(&joined.join(".")), t.span(first, j - 1))
                } else {
                    let r = t.range(j);
                    j += 1;
                    (arc(var), r)
                }
            }
            TokenKind::UserVar => {
                let r = t.range(j);
                j += 1;
                (arc(t.text(j - 1)), r)
            }
            TokenKind::Word | TokenKind::QuotedIdent => {
                // Plugin variables may be dotted (`validate_password.length`).
                let first = j;
                let mut parts = vec![t.name(j).0];
                j += 1;
                while t.kind(j) == Some(TokenKind::Dot) && t.is_name(j + 1) {
                    parts.push(t.name(j + 1).0);
                    j += 2;
                }
                let joined: Vec<&str> = parts.iter().map(|p| &**p).collect();
                (arc(&joined.join(".")), t.span(first, j - 1))
            }
            _ => return None,
        };
        if !(t.is_op(j, "=") || t.is_op(j, ":=")) {
            return None;
        }
        j += 1;
        let value_start = j;
        let mut depth = 0i32;
        while j < n {
            match t.kind(j) {
                Some(TokenKind::LParen) => depth += 1,
                Some(TokenKind::RParen) => depth -= 1,
                Some(TokenKind::Comma) if depth == 0 => break,
                _ => {}
            }
            j += 1;
        }
        if j == value_start {
            return None;
        }
        let value = &t.text[t.start(value_start)..t.end(j - 1)];
        out.push((name, arc(value.trim()), name_range, global));
        if j >= n {
            return Some(());
        }
        j += 1; // the comma
    }
}

// ---------------------------------------------------------------------------------------------
// PRAGMA (SQLite)

/// SQLite's `PRAGMA [schema.]name`, `PRAGMA [schema.]name(arg)` and `PRAGMA [schema.]name =
/// value`: a read only without an argument for a `sqlite_read_pragmas()` name, or with one
/// table or index name for an introspection pragma; any other pragma is `ADMIN`
/// (`AdminControl`); a form that does not read is `UNKNOWN` (`ParseError`).
pub(crate) fn pragma(t: &Toks<'_>, acc: &mut Acc) {
    acc.kind = Some(StmtKind::Pragma);
    let whole = t.whole();
    let n = t.len();
    let mut j = 1;
    if t.is_name(j) && t.kind(j + 1) == Some(TokenKind::Dot) {
        j += 2;
    }
    if !t.is_name(j) {
        acc.doubt(ReasonCode::ParseError, whole);
        return;
    }
    let (name, _) = t.name(j);
    let name_range = t.range(j);
    j += 1;
    let read = if j == n {
        crate::pragmas::pragma_read_without_arg(&name)
    } else if t.kind(j) == Some(TokenKind::LParen)
        && j + 3 == n
        && t.kind(j + 2) == Some(TokenKind::RParen)
    {
        let arg_is_name = matches!(
            t.kind(j + 1),
            Some(TokenKind::Word | TokenKind::QuotedIdent | TokenKind::String)
        );
        arg_is_name && crate::pragmas::pragma_read_with_name_arg(&name)
    } else if t.is_op(j, "=") && j + 1 < n {
        false
    } else {
        acc.doubt(ReasonCode::ParseError, whole);
        return;
    };
    if read {
        acc.effects |= EffectSet::READ;
    } else {
        acc.effects |= EffectSet::ADMIN;
        acc.reason(ReasonCode::AdminControl, Some(name_range), Some(name));
    }
}

// ---------------------------------------------------------------------------------------------
// Narrow read grammars (P-M3b-35)

/// Words that start a subquery or an assignment: never inside a narrow read form.
const NOT_IN_NARROW: [&str; 5] = ["SELECT", "WITH", "VALUES", "INTO", "TABLE"];

/// `SHOW <words> [LIKE '…' | WHERE <read expr>]` where `sqlparser` rejects the form: names,
/// strings, numbers, operators and `IN (…)` lists only; no call (`word(`), no subquery, no
/// variable, no assignment. `SHOW TABLE STATUS` keeps its `TABLE`.
pub(crate) fn narrow_show(t: &Toks<'_>) -> bool {
    if !t.is(0, "SHOW") {
        return false;
    }
    (1..t.len()).all(|j| match t.kind(j) {
        Some(TokenKind::Word) => {
            let table_status = t.is(j, "TABLE") && t.is(j + 1, "STATUS");
            (table_status || !t.is_any(j, &NOT_IN_NARROW))
                && t.kind(j + 1) != Some(TokenKind::LParen)
        }
        Some(TokenKind::QuotedIdent) => t.kind(j + 1) != Some(TokenKind::LParen),
        Some(
            TokenKind::String
            | TokenKind::Number
            | TokenKind::Dot
            | TokenKind::Comma
            | TokenKind::RParen,
        ) => true,
        Some(TokenKind::Op) => t.text(j) != ":=",
        Some(TokenKind::LParen) => t.is(j - 1, "IN"),
        _ => false,
    })
}

/// What a `DESCRIBE`/`DESC` that `sqlparser` rejects is.
pub(crate) enum Describe {
    /// `DESCRIBE <qualified name> [<column or pattern>]`: a read.
    Table,
    /// `DESCRIBE` used as `EXPLAIN`: the inner statement starts at this byte.
    Explain(usize),
    /// Neither.
    No,
}

/// `DESCRIBE <qualified name> [<column>]` or a `DESCRIBE` that is an `EXPLAIN`.
pub(crate) fn narrow_describe(t: &Toks<'_>) -> Describe {
    if !t.is_any(0, &["DESCRIBE", "DESC"]) {
        return Describe::No;
    }
    if let Some(at) = narrow_explain(t) {
        return Describe::Explain(at);
    }
    let n = t.len();
    let mut j = 1;
    if !t.is_name(j) || t.is_any(j, &NOT_IN_NARROW) {
        return Describe::No;
    }
    j += 1;
    while t.kind(j) == Some(TokenKind::Dot) && t.is_name(j + 1) {
        j += 2;
    }
    if j < n
        && matches!(
            t.kind(j),
            Some(TokenKind::Word | TokenKind::QuotedIdent | TokenKind::String)
        )
    {
        j += 1;
    }
    if j == n {
        Describe::Table
    } else {
        Describe::No
    }
}

/// Statement words an `EXPLAIN` may explain.
const EXPLAINABLE: [&str; 7] = [
    "SELECT", "WITH", "TABLE", "VALUES", "INSERT", "REPLACE", "UPDATE",
];

/// `EXPLAIN [FORMAT = <word or string>] <statement>` (or `DESCRIBE …` in that role) without
/// `ANALYZE`: the byte where the inner statement starts, which the caller classifies on its
/// own (P-M3b-35). `None` for any other form.
pub(crate) fn narrow_explain(t: &Toks<'_>) -> Option<usize> {
    if !t.is_any(0, &["EXPLAIN", "DESCRIBE", "DESC"]) {
        return None;
    }
    let mut j = 1;
    if t.is(j, "FORMAT") {
        if !t.is_op(j + 1, "=") {
            return None;
        }
        if !matches!(t.kind(j + 2), Some(TokenKind::Word | TokenKind::String)) {
            return None;
        }
        j += 3;
    }
    let explainable =
        t.is_any(j, &EXPLAINABLE) || t.is(j, "DELETE") || t.kind(j) == Some(TokenKind::LParen);
    explainable.then(|| t.start(j))
}

// ---------------------------------------------------------------------------------------------
// The running reading of an executable comment

/// Where a server that *runs* an executable comment ends it: it reads the body as code from
/// `from` (the first body byte), so a `*/` inside a quoted string or identifier, or inside a
/// `#`/`-- ` comment, does not close it; one nested `/* … */` is opaque to its first `*/`.
/// Returns the byte after the closing `*/`, or `None` when the body does not close before
/// `limit` (or meets an inner executable comment, which this reading does not follow).
pub(crate) fn running_close(
    text: &str,
    from: usize,
    limit: usize,
    modes: LexModes,
    mariadb: bool,
) -> Option<usize> {
    let b = text.as_bytes();
    let limit = limit.min(b.len());
    let mut i = from;
    while i < limit {
        match b[i] {
            b'*' if b.get(i + 1) == Some(&b'/') && i + 2 <= limit => return Some(i + 2),
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let inner =
                    b.get(i + 2) == Some(&b'!') || (mariadb && b.get(i + 2..i + 4) == Some(b"M!"));
                if inner {
                    return None;
                }
                let close = text[i + 2..limit].find("*/")?;
                i = i + 2 + close + 2;
            }
            q @ (b'\'' | b'"' | b'`') => {
                let backslash =
                    q != b'`' && !modes.no_backslash_escapes && !(q == b'"' && modes.ansi_quotes);
                i += 1;
                loop {
                    if i >= limit {
                        return None;
                    }
                    if backslash && b[i] == b'\\' {
                        i += 2;
                    } else if b[i] == q {
                        if b.get(i + 1) == Some(&q) {
                            i += 2;
                        } else {
                            i += 1;
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            b'#' => {
                i = text[i..limit].find('\n').map_or(limit, |p| i + p + 1);
            }
            b'-' if b.get(i + 1) == Some(&b'-')
                && b.get(i + 2)
                    .is_none_or(|c| c.is_ascii_whitespace() || c.is_ascii_control()) =>
            {
                i = text[i..limit].find('\n').map_or(limit, |p| i + p + 1);
            }
            _ => i += 1,
        }
    }
    None
}

/// `TextRange` of `a..b` (bytes).
pub(crate) fn range_of(a: usize, b: usize) -> TextRange {
    let size = |n: usize| TextSize::from(u32::try_from(n).unwrap_or(u32::MAX));
    TextRange::new(size(a), size(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_reading_skips_quotes_and_line_comments() {
        let m = LexModes::default();
        let s = "/*!50000 , '*/' */ x";
        assert_eq!(running_close(s, 8, s.len(), m, false), Some(18));
        let s = "/*!50000 # */\n */";
        assert_eq!(running_close(s, 8, s.len(), m, false), Some(s.len()));
        let s = "/*!50000 'a\\'*/' */";
        assert_eq!(running_close(s, 8, s.len(), m, false), Some(s.len()));
        let nbe = LexModes {
            no_backslash_escapes: true,
            ..m
        };
        assert_eq!(running_close(s, 8, s.len(), nbe, false), Some(15));
        let s = "/*!50000 /* a */ b */";
        assert_eq!(running_close(s, 8, s.len(), m, false), Some(s.len()));
        let s = "/*!50000 /*!40000 x */ */";
        assert_eq!(running_close(s, 8, s.len(), m, false), None);
        assert_eq!(running_close("/*!50000 'open", 8, 14, m, false), None);
    }

    #[test]
    fn hint_items_parse() {
        assert_eq!(
            hint_calls(" SET_VAR(sort_buffer_size = 16M) MAX_EXECUTION_TIME(1000) BKA(t1) "),
            vec![
                ("SET_VAR", "sort_buffer_size = 16M"),
                ("MAX_EXECUTION_TIME", "1000"),
                ("BKA", "t1")
            ]
        );
        assert_eq!(hint_calls("SET_VAR(x=(1)"), vec![("SET_VAR", "x=(1)")]);
    }
}
