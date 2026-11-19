// ## m4-t01c-core (seam) — body: ## m4-t12-scope
//! Object filters (HV-INTRO-03, M4 Task 12): which objects of a namespace are read
//! ([`FilterRole::Introspect`], the scope filter) or shown ([`FilterRole::View`], the view
//! filter). Builds without the `session` feature (the fuzzed grammar).
//!
//! # Grammar
//!
//! ```text
//! filter := item ( '||' item )*
//! item   := ( kind ':' ( 'sig:' )? )? '-'? pattern
//! pattern:= glob | '{' regex '}'
//! ```
//!
//! - A **glob** is [`crate::glob`]'s: `*`, `?`, `[…]`, `\` escapes; `_`, `%` and a single `|`
//!   are literal. A `{…}` regex is brace-balanced (`\}` escapes a brace) and matches the whole
//!   name; `||` inside it belongs to the regex. Whitespace around an item is trimmed (escape a
//!   name's edge space as `\ `). An empty item (`a||||b`) is ignored with a warning.
//! - A **kind** is an English, locale-invariant token: an `ObjectKind` machine id
//!   (`table`, `view`, `materialized-view`, `sequence`, `index`, `trigger`, `event`, …; `_`
//!   may stand for `-`) or an alias: `routine` = `function` = `procedure` (functions,
//!   procedures and aggregates), `mview` = `materialized-view`; `table` also covers partitions.
//!   A token the dialect's kind schema does not have is an error (`intro-filter-unknown-kind`);
//!   so is any `word:` prefix that is not a kind (`tabel:x`; escape a literal colon `\:`).
//! - `routine:sig:` items match the signature form `name(argtypes)` instead of the name
//!   ([`CompiledFilter::matches_signature`]; edge 9). `matches` skips them.
//!
//! # Meaning (DataGrip-compatible)
//!
//! An object of kind `k` named `n` passes when, among the items that apply to `k` (typed for
//! `k` or untyped), there is no positive item or some positive item matches, and no negative
//! (`-`) item matches. Kinds no item applies to pass. Case follows the server's identifier
//! rule for the kind's name class (HV-INTRO-14, [`CompiledFilter::with_ident`]): exact on
//! PostgreSQL, ASCII-caseless where MySQL/MariaDB or SQLite fold; never Unicode or Turkish
//! folding. Every glob and regex is compiled once, at parse.
//!
//! The **view** filter only hides (its objects stay `Found` for resolution); the **scope**
//! filter leaves objects unread, so what it excludes is `Unknown(OutOfScope)`
//! ([`CompiledFilter::excludes`] feeds `hacivat_meta::view::ScopeFilter`). A compiled filter
//! is an immutable value: a load that started with one keeps it (edge 13).
//!
//! # Pushdown (scope filters only)
//!
//! [`CompiledFilter::pushdown`] turns the positive globs that apply to a kind into `LIKE`
//! patterns sent **as binds** ([`SqlPredicate`]); the SQL text is a fixed pack fragment and
//! never holds user text (R-M4-7). Never a negative item, a regex, a signature item or a
//! match-everything glob; never `ILIKE`, `lower()` or `upper()`; nothing when an applicable
//! positive item cannot be sent (the server must return a superset) or when more than
//! [`MAX_PUSHDOWN_ITEMS`] apply. The client matcher always runs after it.

use std::sync::Arc;

use hacivat_meta::name::{FoldRule, IdentCtx, NameClass};
use hacivat_meta::{Name, ObjectKind};
use hacivat_proto::DbmsKind;
use hacivat_values::{BindValue, LogicalType};
use regex::Regex;
use sql_dialect::{DialectId, PgWireFlavor};

use crate::glob::{Glob, GlobCase, full_match_pair};
use crate::scope::NsInfo;

/// Above this many applicable positive items nothing is pushed down (no giant SQL; alg/02
/// HV-INTRO-03 edge 12).
pub const MAX_PUSHDOWN_ITEMS: usize = 200;

/// What a filter decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FilterRole {
    /// Objects that are not read from the server at all (the scope filter).
    Introspect,
    /// Objects that are read but hidden in the explorer (the view filter).
    View,
}

/// A compiled object filter.
#[derive(Debug, Clone)]
pub struct CompiledFilter {
    text: String,
    dialect: DialectId,
    role: FilterRole,
    ident: IdentCtx,
    items: Vec<Item>,
    warnings: Vec<FilterWarning>,
}

impl PartialEq for CompiledFilter {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
            && self.dialect == other.dialect
            && self.role == other.role
            && self.ident == other.ident
    }
}

impl Eq for CompiledFilter {}

impl std::hash::Hash for CompiledFilter {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.text.hash(state);
        self.dialect.hash(state);
        self.role.hash(state);
        self.ident.hash(state);
    }
}

#[derive(Debug, Clone)]
struct Item {
    /// The kinds it applies to; empty: every kind.
    kinds: Vec<ObjectKind>,
    negative: bool,
    /// A `routine:sig:` item: matched against the signature form.
    sig: bool,
    pattern: Pattern,
}

#[derive(Debug, Clone)]
enum Pattern {
    Glob { exact: Glob, caseless: Glob },
    Regex { exact: Regex, caseless: Regex },
}

impl Pattern {
    fn is_match(&self, s: &str, case: GlobCase) -> bool {
        match (self, case) {
            (Pattern::Glob { exact, .. }, GlobCase::Exact) => exact.is_match(s),
            (Pattern::Glob { caseless, .. }, GlobCase::AsciiInsensitive) => caseless.is_match(s),
            (Pattern::Regex { exact, .. }, GlobCase::Exact) => exact.is_match(s),
            (Pattern::Regex { caseless, .. }, GlobCase::AsciiInsensitive) => caseless.is_match(s),
        }
    }
}

impl Item {
    fn applies(&self, kind: ObjectKind) -> bool {
        self.kinds.is_empty() || self.kinds.contains(&kind)
    }
}

/// A `WHERE` fragment a scope filter pushes down to a catalog query: fixed pack text plus one
/// `LIKE` pattern bind per positive glob (R-M4-3, R-M4-7, R-W1-7).
///
/// **How a pack section uses it** (Tasks 16–18). The section writes
/// `AND <name column> LIKE :name_like ESCAPE :like_escape` as a top-level `AND` conjunct
/// (this is [`SqlPredicate::sql_fragment`] after the column; it parses under Task 4's lexer —
/// whose literals may hold no backslash, so the escape character `\` travels as a bind too —
/// and composes with a `<col> IN :[x]` list bind). The positive items are OR'ed, so the
/// section runs **once per pattern**, each run binding [`SqlPredicate::runs`]'s scalars; the
/// union of the rows (de-duplicated by key) is the superset the client matcher then filters,
/// as list-bind chunks are unioned.
#[derive(Debug, Clone, PartialEq)]
pub struct SqlPredicate {
    pub sql_fragment: &'static str,
    /// One `LIKE` pattern per distinct positive glob, in item order (`ESCAPE '\'`).
    pub binds: Vec<BindValue>,
}

impl SqlPredicate {
    /// The pack bind that carries one pattern.
    pub const PATTERN_BIND: &'static str = "name_like";
    /// The pack bind that carries the escape character `\`.
    pub const ESCAPE_BIND: &'static str = "like_escape";
    /// The fragment: pack text, binds only (the same on every dialect: `:x` renders `$n` on
    /// PostgreSQL and `?` elsewhere).
    pub const FRAGMENT: &'static str = "LIKE :name_like ESCAPE :like_escape";

    /// The escape character's bind value (`\`).
    pub fn escape() -> BindValue {
        text_bind("\\")
    }

    /// The scalar binds of each run, one run per pattern: `name_like` and `like_escape`.
    pub fn runs(&self) -> impl Iterator<Item = Vec<(Name, BindValue)>> + '_ {
        self.binds.iter().map(|b| {
            vec![
                (Name::from(Self::PATTERN_BIND), b.clone()),
                (Name::from(Self::ESCAPE_BIND), Self::escape()),
            ]
        })
    }
}

fn text_bind(s: &str) -> BindValue {
    BindValue::Typed {
        hint: LogicalType::Text {
            max_chars: None,
            fixed: false,
        },
        text: Arc::from(s),
    }
}

/// A parse warning: the filter applies, but part of it is ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FilterWarning {
    /// The 1-based position, in Unicode scalar values.
    pub column: u32,
    pub kind: FilterWarningKind,
}

/// The closed set of filter warnings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FilterWarningKind {
    /// An empty item (`a||||b`, a trailing `||`): ignored (edge 5).
    EmptyItem,
}

impl CompiledFilter {
    /// The filter with the server's identifier rule (`lower_case_table_names`; from the
    /// session's `ServerFacts`). Until it is set the dialect's defaults apply
    /// ([`IdentCtx::new`]: `lower_case_table_names = 0`).
    pub fn with_ident(mut self, ident: IdentCtx) -> Self {
        self.ident = ident;
        self
    }

    /// Whether an object of `kind` named `name` passes the filter (`routine:sig:` items are
    /// skipped; see [`CompiledFilter::matches_signature`]).
    pub fn matches(&self, kind: ObjectKind, name: &Name) -> bool {
        self.decide(kind, name, None)
    }

    /// [`CompiledFilter::matches`], with `routine:sig:` items matched against `signature`
    /// (`name(argtypes)`, as the introspector formats it).
    pub fn matches_signature(&self, kind: ObjectKind, name: &str, signature: &str) -> bool {
        self.decide(kind, name, Some(signature))
    }

    fn decide(&self, kind: ObjectKind, name: &str, signature: Option<&str>) -> bool {
        let case = self.case(kind);
        let mut positives = false;
        let mut hit = false;
        for it in self.items.iter().filter(|it| it.applies(kind)) {
            let subject = match (it.sig, signature) {
                (false, _) => name,
                (true, Some(s)) => s,
                (true, None) => continue,
            };
            let m = it.pattern.is_match(subject, case);
            if it.negative {
                if m {
                    return false;
                }
            } else {
                positives = true;
                hit |= m;
            }
        }
        !positives || hit
    }

    fn dbms(&self) -> DbmsKind {
        dbms_of(self.dialect)
    }

    fn case(&self, kind: ObjectKind) -> GlobCase {
        let class = kind.name_class(self.dbms());
        match FoldRule::for_names(self.dialect, &self.ident, class) {
            Some(FoldRule::AsciiCaseless) => GlobCase::AsciiInsensitive,
            _ => GlobCase::Exact,
        }
    }

    /// Whether this scope filter may have left an object of `kind` (`None`: any kind) named
    /// `name` (`None`: any name; else the probe text as typed, quoted or not) unread — the
    /// answer a `hacivat_meta::view::ScopeFilter` gives. `true` when unsure; a view filter
    /// never leaves anything unread (`false`).
    pub fn excludes(&self, kind: Option<ObjectKind>, name: Option<&str>) -> bool {
        if self.role == FilterRole::View || self.items.is_empty() {
            return false;
        }
        let kinds: Vec<ObjectKind> = match kind {
            Some(k) => vec![k],
            // Every kind an item names, and one no typed item names (untyped items only).
            None => self
                .items
                .iter()
                .flat_map(|it| it.kinds.iter().copied())
                .chain([ObjectKind::Unknown])
                .collect(),
        };
        let Some(name) = name else {
            return kinds
                .iter()
                .any(|&k| self.items.iter().any(|it| it.applies(k)));
        };
        // The probe as typed (it may name a quoted object) and folded as PostgreSQL folds an
        // unquoted name; the caseless engines already compare caselessly.
        let probes = [name.to_owned(), name.to_ascii_lowercase()];
        kinds.iter().any(|&k| {
            self.items.iter().any(|it| it.applies(k) && it.sig)
                || probes.iter().any(|p| !self.decide(k, p, None))
        })
    }

    /// Whether the filter applies inside namespace `ns`: a scope filter never applies to a
    /// system namespace (`pg_catalog`, `information_schema`, `mysql`, …), whose objects come
    /// from the shipped snapshots for type and function resolution (edge 10).
    pub fn applies_to(&self, ns: &NsInfo) -> bool {
        !(self.role == FilterRole::Introspect && ns.system)
    }

    /// The server-side form of the positive globs that apply to `kind`, for a catalog query on
    /// `dialect` under `ident` (see the module docs and [`SqlPredicate`]). `None` — the client
    /// filters everything — for a view filter, a dialect without an eligibility rule, a kind
    /// whose catalog column compares differently from the client matcher (alg/02 HV-INTRO-03
    /// matrix), no applicable positive item, more than [`MAX_PUSHDOWN_ITEMS`], or any
    /// applicable positive item that cannot be sent as is.
    ///
    /// Eligibility (the per-DBMS rows of alg/02 HV-INTRO-03, MSSQL excluded per R-M4-4):
    /// - **PostgreSQL** (`name` columns compare in the `C` collation): only exact items (every
    ///   PostgreSQL name class is exact; a case-insensitive item is never pushed, and no
    ///   `ILIKE`/`lower()` is ever written). A non-ASCII pattern only on a UTF-8 server (a
    ///   character the server encoding lacks would fail the whole query).
    /// - **MySQL / MariaDB**: table and database names only (exact under
    ///   `lower_case_table_names = 0`, caseless under 1 and 2, as `INFORMATION_SCHEMA` compares
    ///   them); every other name class (columns, routines, triggers, …) client-side. No
    ///   character outside the BMP (`INFORMATION_SCHEMA` is `utf8mb3`).
    /// - **SQLite** (`LIKE` is ASCII-caseless, like the client rule): ASCII patterns only.
    /// - Every other dialect: none.
    ///
    /// A pattern holding NUL is never sent.
    pub fn pushdown(
        &self,
        kind: ObjectKind,
        dialect: DialectId,
        ident: &IdentCtx,
    ) -> Option<SqlPredicate> {
        if self.role != FilterRole::Introspect || !same_family(self.dialect, dialect) {
            return None;
        }
        let class = kind.name_class(dbms_of(dialect));
        let rule = FoldRule::for_names(dialect, ident, class)?;
        let utf8 = ident
            .encoding
            .as_deref()
            .is_some_and(|e| e.eq_ignore_ascii_case("UTF8") || e.eq_ignore_ascii_case("UTF-8"));
        let char_ok: fn(&str, bool) -> bool = match dialect {
            DialectId::Postgres if rule == FoldRule::Exact => |p, utf8| utf8 || p.is_ascii(),
            DialectId::MySql | DialectId::MariaDb
                if matches!(
                    class,
                    NameClass::Table | NameClass::Database | NameClass::Schema
                ) =>
            {
                |p, _| p.chars().all(|c| u32::from(c) <= 0xFFFF)
            }
            DialectId::Sqlite => |p, _| p.is_ascii(),
            _ => return None,
        };
        let mut positives = 0usize;
        let mut binds: Vec<BindValue> = Vec::new();
        for it in self
            .items
            .iter()
            .filter(|it| it.applies(kind) && !it.negative)
        {
            positives += 1;
            if positives > MAX_PUSHDOWN_ITEMS || it.sig {
                return None;
            }
            let Pattern::Glob { exact, .. } = &it.pattern else {
                return None;
            };
            if exact.matches_everything() {
                return None;
            }
            let like = exact.like_pattern();
            if like.contains('\0') || !char_ok(&like, utf8) {
                return None;
            }
            let b = text_bind(&like);
            if !binds.contains(&b) {
                binds.push(b);
            }
        }
        if binds.is_empty() {
            return None;
        }
        Some(SqlPredicate {
            sql_fragment: SqlPredicate::FRAGMENT,
            binds,
        })
    }

    /// The filter as typed.
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn role(&self) -> FilterRole {
        self.role
    }

    pub fn dialect(&self) -> DialectId {
        self.dialect
    }

    pub fn warnings(&self) -> &[FilterWarning] {
        &self.warnings
    }

    /// Whether the filter has no item (it passes everything).
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Why a filter does not parse (`intro-filter-*`, with the offending item as typed).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FilterError {
    UnknownKind {
        item: String,
    },
    /// Also an unclosed `{` or text after a regex's `}`.
    BadRegex {
        item: String,
    },
    BadGlob {
        item: String,
    },
}

/// Parses an object filter of `dialect` for `role` (see the module docs).
pub fn parse_filter(
    text: &str,
    dialect: DialectId,
    role: FilterRole,
) -> Result<CompiledFilter, FilterError> {
    let cs: Vec<char> = text.chars().collect();
    let dbms = dbms_of(dialect);
    let mut items = Vec::new();
    let mut warnings = Vec::new();
    let mut i = 0;
    if !cs.iter().all(|c| c.is_whitespace()) {
        loop {
            let end = item_end(&cs, i);
            let raw: String = cs[i..end].iter().collect();
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                warnings.push(FilterWarning {
                    column: u32::try_from(i + 1).unwrap_or(u32::MAX),
                    kind: FilterWarningKind::EmptyItem,
                });
            } else {
                items.push(parse_item(trimmed, dbms)?);
            }
            if end >= cs.len() {
                break;
            }
            i = end + 2;
        }
    }
    Ok(CompiledFilter {
        text: text.to_owned(),
        dialect,
        role,
        ident: IdentCtx::new(dialect),
        items,
        warnings,
    })
}

/// The index of the `||` that ends the item starting at `i` (or the text's end). A `{…}`
/// regex (an item's pattern that starts with `{`, after the kind and the `-`), a glob class
/// `[…]` and a `\`-escaped character never end an item; a brace inside a glob is literal.
fn item_end(cs: &[char], i: usize) -> usize {
    let mut j = i;
    while cs.get(j).is_some_and(|c| c.is_whitespace()) {
        j += 1;
    }
    if cs.get(j).is_some_and(char::is_ascii_alphabetic) {
        let mut e = j;
        while cs
            .get(e)
            .is_some_and(|&c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            e += 1;
        }
        if cs.get(e) == Some(&':') {
            j = e + 1;
            if cs[j..].starts_with(&['s', 'i', 'g', ':']) {
                j += 4;
            }
        }
    }
    if cs.get(j) == Some(&'-') {
        j += 1;
    }
    if cs.get(j) == Some(&'{') {
        // The regex runs to its balancing `}`; unclosed, the rest of the text is the item.
        let mut depth = 0usize;
        loop {
            match cs.get(j) {
                None => return cs.len(),
                Some('\\') => j += 1,
                Some('{') => depth += 1,
                Some('}') => {
                    depth -= 1;
                    if depth == 0 {
                        j += 1;
                        break;
                    }
                }
                Some(_) => {}
            }
            j += 1;
        }
    }
    let mut class = false;
    while j < cs.len() {
        match cs[j] {
            '\\' => {
                j += 2;
                continue;
            }
            '[' if !class => {
                class = true;
                // `]` right after `[`, `[!` or `[^` is literal.
                let mut k = j + 1;
                if matches!(cs.get(k), Some('!' | '^')) {
                    k += 1;
                }
                if cs.get(k) == Some(&']') {
                    j = k;
                }
            }
            ']' if class => class = false,
            '|' if !class && cs.get(j + 1) == Some(&'|') => return j,
            _ => {}
        }
        j += 1;
    }
    cs.len()
}

fn parse_item(raw: &str, dbms: DbmsKind) -> Result<Item, FilterError> {
    let item = || raw.to_owned();
    let mut rest = raw;
    let mut kinds = Vec::new();
    let mut sig = false;
    if let Some((token, after)) = kind_prefix(rest) {
        kinds = kinds_of(token, dbms).ok_or_else(|| FilterError::UnknownKind { item: item() })?;
        rest = after;
        if kinds.contains(&ObjectKind::Routine)
            && let Some(after) = rest.strip_prefix("sig:")
        {
            sig = true;
            rest = after;
        }
    }
    let negative = rest.starts_with('-');
    if negative {
        rest = &rest[1..];
    }
    let pattern = if let Some(body) = rest.strip_prefix('{') {
        let source = regex_body(body).ok_or_else(|| FilterError::BadRegex { item: item() })?;
        let (exact, caseless) =
            full_match_pair(source).map_err(|_| FilterError::BadRegex { item: item() })?;
        Pattern::Regex { exact, caseless }
    } else {
        if rest.is_empty() {
            return Err(FilterError::BadGlob { item: item() });
        }
        let bad = |_| FilterError::BadGlob { item: item() };
        Pattern::Glob {
            exact: Glob::new(rest, GlobCase::Exact).map_err(bad)?,
            caseless: Glob::new(rest, GlobCase::AsciiInsensitive).map_err(bad)?,
        }
    };
    Ok(Item {
        kinds,
        negative,
        sig,
        pattern,
    })
}

/// `body` is the text after a `{`: the regex up to its balancing `}`, which must end the item.
fn regex_body(body: &str) -> Option<&str> {
    let mut depth = 1usize;
    let mut it = body.char_indices();
    while let Some((at, c)) = it.next() {
        match c {
            '\\' => {
                it.next();
            }
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return body[at + 1..].is_empty().then_some(&body[..at]);
                }
            }
            _ => {}
        }
    }
    None
}

/// A leading `word:` (an ASCII letter, then letters, digits, `_` or `-`): the kind token and
/// the text after the colon.
fn kind_prefix(s: &str) -> Option<(&str, &str)> {
    let b = s.as_bytes();
    if !b.first()?.is_ascii_alphabetic() {
        return None;
    }
    let end = b
        .iter()
        .position(|&c| !(c.is_ascii_alphanumeric() || c == b'_' || c == b'-'))?;
    (b[end] == b':').then(|| (&s[..end], &s[end + 1..]))
}

/// The kinds a token names in `dbms`; `None` when the kind schema has none of them.
fn kinds_of(token: &str, dbms: DbmsKind) -> Option<Vec<ObjectKind>> {
    use ObjectKind::*;
    let t: String = token
        .chars()
        .map(|c| {
            if c == '_' {
                '-'
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect();
    let family: Vec<ObjectKind> = match t.as_str() {
        "table" => vec![Table, Partition, Subpartition],
        "routine" | "function" | "procedure" => vec![Routine, Procedure, Aggregate],
        "mview" => vec![MaterializedView],
        other => match ObjectKind::from_id(other)? {
            // Namespaces are the scope's, not a filter's; `unknown` names no kind.
            Root | Database | Schema | Unknown => return None,
            k => vec![k],
        },
    };
    let kinds: Vec<ObjectKind> = family.into_iter().filter(|k| k.exists_in(dbms)).collect();
    (!kinds.is_empty()).then_some(kinds)
}

/// The engine whose kind schema and name classes `d` uses.
fn dbms_of(d: DialectId) -> DbmsKind {
    match d {
        DialectId::Postgres | DialectId::PgWire(PgWireFlavor::Other) => DbmsKind::Postgres,
        DialectId::PgWire(PgWireFlavor::Redshift) => DbmsKind::Redshift,
        DialectId::PgWire(PgWireFlavor::Cockroach) => DbmsKind::Cockroach,
        DialectId::PgWire(PgWireFlavor::Yugabyte) => DbmsKind::Yugabyte,
        DialectId::PgWire(PgWireFlavor::Timescale) => DbmsKind::Timescale,
        DialectId::MySql => DbmsKind::MySql,
        DialectId::MariaDb => DbmsKind::MariaDb,
        DialectId::Sqlite => DbmsKind::Sqlite,
        DialectId::SqlServer => DbmsKind::MsSql,
        DialectId::ClickHouse => DbmsKind::ClickHouse,
        DialectId::Oracle => DbmsKind::Oracle,
        // `Generic` and any later dialect: no kind schema of its own.
        _ => DbmsKind::Unknown,
    }
}

/// Whether a filter parsed for `a` may push down to a catalog query of `b` (MySQL and MariaDB
/// share a kind schema and catalog columns).
fn same_family(a: DialectId, b: DialectId) -> bool {
    let my = |d| matches!(d, DialectId::MySql | DialectId::MariaDb);
    a == b || (my(a) && my(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_split_on_double_bars_outside_regexes_classes_and_escapes() {
        let ends = |s: &str| {
            let cs: Vec<char> = s.chars().collect();
            item_end(&cs, 0)
        };
        assert_eq!(ends("a||b"), 1);
        assert_eq!(ends("{a||b}||c"), 6);
        assert_eq!(ends("[|]|||c"), 3);
        assert_eq!(ends(r"a\|||b"), 3);
        assert_eq!(ends("[]||]x"), 6);
        assert_eq!(ends("a|b"), 3);
        // A brace is a regex only where a pattern starts; in a glob it is literal.
        assert_eq!(ends("a{b||c"), 3);
        assert_eq!(ends("table:-{x||y}||z"), 13);
        assert_eq!(ends(" routine:sig:{f(a||b)}||z"), 22);
        assert_eq!(ends("{unclosed||x"), 12);
    }

    #[test]
    fn kind_tokens_follow_the_dialect_kind_schema() {
        assert!(kinds_of("table", DbmsKind::Postgres).is_some());
        assert!(kinds_of("TABLE", DbmsKind::Postgres).is_some());
        assert!(kinds_of("materialized_view", DbmsKind::Postgres).is_some());
        assert!(kinds_of("mview", DbmsKind::MySql).is_none());
        assert!(kinds_of("event", DbmsKind::Postgres).is_none());
        assert!(kinds_of("event", DbmsKind::MySql).is_some());
        assert!(kinds_of("schema", DbmsKind::Postgres).is_none());
        assert_eq!(
            kinds_of("function", DbmsKind::MySql),
            Some(vec![ObjectKind::Routine, ObjectKind::Procedure])
        );
    }
}
