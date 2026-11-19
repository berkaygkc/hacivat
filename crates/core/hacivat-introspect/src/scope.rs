// ## m4-t01c-core (seam) — body: ## m4-t12-scope
//! Schema scope patterns (HV-INTRO-02, M4 Task 12): which namespaces a profile introspects.
//! Builds without the `session` feature (the fuzzed grammar, `fuzz/meta` `scope_pattern`).
//!
//! # Grammar
//!
//! ```text
//! pattern  := clause ( '|' clause )*             // empty text: nothing but @:@ (below)
//! clause   := '!'? dbterm ( ':' schlist )?        // PostgreSQL (database:schema)
//!           | '!'? ( dbterm ':' )? schlist         // MySQL, MariaDB, SQLite (one level)
//! schlist  := term ( ',' term )*
//! term     := '@' | '*' | '{' regex '}' | name
//! name     := bare | '"' ( '""' | [^"] )* '"'
//! ```
//!
//! A bare name holds no whitespace and none of `: , | { } ! " @ *` (a `*` inside a name is
//! refused rather than read as a literal star, so `app_*` is an error, not a strange name;
//! quote it: `"app_*"`). Whitespace around terms and separators is ignored. A regex is
//! brace-balanced and `\}` escapes a brace; it is a `regex` crate expression matched against
//! the **whole** name. Error positions count Unicode scalar values from 1, never bytes.
//!
//! # Meaning
//!
//! - `@` is the current database (database position) or schema (schema position) as detected
//!   at connect ([`CurrentCtx`]); unknown → it selects nothing (`intro-scope-current-unknown`,
//!   [`ScopePattern::mentions_current`]). `{t.*}:@` is the schema named like the current one in
//!   every database whose name fully matches `t.*`.
//! - `*` is every namespace **except system namespaces** ([`NsInfo::system`]): those come from
//!   the shipped snapshots and are never introspected live by default (R-M4-14, R-M4-6). A
//!   regex skips them too; a name or `@` selects one. In a **negative** clause `*` and a regex
//!   do reach them: removing is always safe (`@:pg_catalog|!*:*` selects nothing).
//! - `@` and the implicit `@:@` compare with the listed names by the server's rule, like a
//!   bare name (`DATABASE()` = `Shop` names the listed `shop` under MySQL
//!   `lower_case_table_names` 2), and select the listed key.
//! - A name compares by the dialect's identifier key ([`sql_ident::key::match_key`],
//!   HV-INTRO-14): on PostgreSQL bare `Sales` is `sales` and `"Sales"` is `Sales`; on MySQL and
//!   MariaDB under `lower_case_table_names` 1 or 2, and on SQLite, ASCII letters compare
//!   caselessly. Where the server's rule is not pinned (a non-ASCII cased letter on a caseless
//!   MySQL name, R-M4-41) only the exact text matches. Never the process locale.
//! - A regex is case-sensitive where the name class is exact and ASCII-caseless where it folds
//!   (never Unicode or Turkish folding); it is compiled once, at parse.
//! - A PostgreSQL clause with only a database term means `db:@`: that database and its schema
//!   named like the current one (DataGrip's "database only").
//! - On one-level dialects a `db:` prefix is ignored with a warning unless it is `*`
//!   (edge 11); the text keeps it.
//! - The result is the union of the positive clauses minus the union of the negative ones
//!   (order-independent; duplicates are idempotent), **plus the current namespace**, which is
//!   always kept so the session's default schema resolves (edge 9; [`ScopePattern::select`]).
//!   [`ScopePattern::select_explicit`] is the pattern alone (what the explorer shows).
//! - A namespace the server does not list is never selected (edge 13: no error either).
//!
//! The scope is never a security boundary (Karagöz enforces RBAC separately, HV-INTRO-17).

use std::collections::BTreeSet;

use hacivat_meta::name::{FoldRule, IdentCtx, NameClass, NamePart, match_key};
use hacivat_meta::{Name, NsKey, NsKind};
use regex::Regex;
use sql_dialect::DialectId;

use crate::glob::{GlobCase, full_match_pair};

/// A namespace the server lists (`Introspector::list_namespaces`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NsInfo {
    pub key: NsKey,
    /// A system schema (`pg_catalog`, `information_schema`, `mysql`, …): never introspected
    /// live by default; it comes from the snapshots (R-M4-6, R-M4-14).
    pub system: bool,
}

/// What `@` means — the session's current database and schema (MySQL/MariaDB: the database is
/// the schema; SQLite: `main`) — and the server's identifier rule (`lower_case_table_names`,
/// the encoding; `None`: the dialect's defaults, [`IdentCtx::new`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct CurrentCtx {
    pub database: Option<Name>,
    pub schema: Option<Name>,
    pub ident: Option<IdentCtx>,
}

/// A parsed scope pattern: its clauses, the canonical text and the parse warnings. Two patterns
/// are equal when their dialect and syntax tree are (the warnings point into the text as typed).
#[derive(Debug, Clone)]
pub struct ScopePattern {
    dialect: DialectId,
    clauses: Vec<Clause>,
    text: String,
    warnings: Vec<ScopeWarning>,
}

impl PartialEq for ScopePattern {
    fn eq(&self, other: &Self) -> bool {
        self.dialect == other.dialect && self.clauses == other.clauses
    }
}

impl Eq for ScopePattern {}

impl std::hash::Hash for ScopePattern {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.dialect.hash(state);
        self.clauses.hash(state);
    }
}

/// One `|`-separated clause.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Clause {
    /// Written with a leading `!`: it removes what it matches.
    pub negative: bool,
    /// The database term: always present on PostgreSQL; on one-level dialects only when the
    /// text had a `db:` prefix (ignored there).
    pub db: Option<Term>,
    /// The schema terms; empty for a PostgreSQL database-only clause (`db:@`).
    pub schemas: Vec<Term>,
}

/// One term.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Term {
    /// `@`
    Current,
    /// `*`
    All,
    /// `{…}`
    Regex(ScopeRegex),
    /// A bare or quoted name.
    Name(NamePart),
}

/// A `{…}` term: its text as written between the braces, compiled once for each case rule.
#[derive(Debug, Clone)]
pub struct ScopeRegex {
    source: String,
    exact: Regex,
    caseless: Regex,
}

impl ScopeRegex {
    /// The text between the braces, as written (`\}` escapes kept).
    pub fn source(&self) -> &str {
        &self.source
    }

    fn is_match(&self, name: &str, case: GlobCase) -> bool {
        match case {
            GlobCase::Exact => self.exact.is_match(name),
            GlobCase::AsciiInsensitive => self.caseless.is_match(name),
        }
    }
}

impl PartialEq for ScopeRegex {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}

impl Eq for ScopeRegex {}

impl std::hash::Hash for ScopeRegex {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.source.hash(state);
    }
}

/// A parse warning: the pattern applies, but part of it is ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeWarning {
    /// The 1-based position, in Unicode scalar values.
    pub column: u32,
    pub kind: ScopeWarningKind,
}

/// The closed set of scope warnings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScopeWarningKind {
    /// A `db:` prefix on a one-level dialect (MySQL, MariaDB, SQLite) other than `*`: ignored
    /// (alg/02 HV-INTRO-02 edge 11).
    DbTermIgnored,
}

/// Which slot a term stands in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Database,
    Schema,
}

impl ScopePattern {
    /// The namespaces of `infos` the scope covers: [`ScopePattern::select_explicit`] plus the
    /// current namespace when the server lists it (the implicit `@:@`, kept so the session's
    /// default schema resolves even when a negative clause removes it; edge 9).
    ///
    /// The current namespace is the **listed** key that names it under the server's rule
    /// (MySQL/MariaDB `lower_case_table_names` 1 or 2, SQLite: ASCII-caseless), never the
    /// session's text (fix round 1, I3).
    pub fn select(&self, infos: &[NsInfo], ctx: &CurrentCtx) -> BTreeSet<NsKey> {
        let mut out = self.select_explicit(infos, ctx);
        let ident = self.ident_of(ctx);
        let m = self.matcher(&ident, ctx);
        out.extend(
            infos
                .iter()
                .filter(|i| m.is_current(i))
                .map(|i| i.key.clone()),
        );
        out
    }

    fn ident_of(&self, ctx: &CurrentCtx) -> IdentCtx {
        ctx.ident
            .clone()
            .unwrap_or_else(|| IdentCtx::new(self.dialect))
    }

    fn matcher<'a>(&self, ident: &'a IdentCtx, ctx: &'a CurrentCtx) -> Matcher<'a> {
        Matcher {
            dialect: self.dialect,
            ident,
            ctx,
        }
    }

    /// The namespaces of `infos` the pattern itself selects: the union of its positive clauses
    /// minus the union of its negative ones. On PostgreSQL a listed database namespace is
    /// selected with any of its schemas, or by a database-only clause.
    pub fn select_explicit(&self, infos: &[NsInfo], ctx: &CurrentCtx) -> BTreeSet<NsKey> {
        let ident = self.ident_of(ctx);
        let m = self.matcher(&ident, ctx);
        let mut out: BTreeSet<NsKey> = infos
            .iter()
            .filter(|i| {
                let hit = |neg: bool| {
                    self.clauses
                        .iter()
                        .filter(|c| c.negative == neg)
                        .any(|c| m.clause(c, i))
                };
                hit(false) && !hit(true)
            })
            .map(|i| i.key.clone())
            .collect();
        if two_level(self.dialect) {
            let dbs: Vec<NsKey> = infos
                .iter()
                .filter(|i| i.key.kind == NsKind::Database)
                .filter(|i| {
                    out.iter().any(|k| {
                        k.kind == NsKind::Schema && k.path.len() == 2 && k.path[0] == i.key.path[0]
                    })
                })
                .map(|i| i.key.clone())
                .collect();
            out.extend(dbs);
        }
        out
    }

    // ## m4-t15-engine (named exception, review I-2)
    /// Whether some positive clause may select a schema of PostgreSQL database `db` (its
    /// database term matches `db`); negative clauses are ignored, so this is a superset. The
    /// engine lists the schemas of every such database on a connection to it (one
    /// `pg_namespace` per database, alg/02 HV-INTRO-02 edge 10). `false` on one-level dialects.
    #[cfg(feature = "session")]
    pub(crate) fn may_select_database(&self, db: &Name, system: bool, ctx: &CurrentCtx) -> bool {
        if !two_level(self.dialect) {
            return false;
        }
        let ident = self.ident_of(ctx);
        let m = self.matcher(&ident, ctx);
        self.clauses.iter().filter(|c| !c.negative).any(|c| {
            c.db.as_ref()
                .is_some_and(|t| m.term(t, db, Slot::Database, system))
        })
    }
    // ## m4-t15-engine

    /// Whether a clause writes `@` (when the current namespace is unknown the caller shows
    /// `intro-scope-current-unknown`).
    pub fn mentions_current(&self) -> bool {
        self.clauses
            .iter()
            .flat_map(|c| c.db.iter().chain(&c.schemas))
            .any(|t| *t == Term::Current)
            || self
                .clauses
                .iter()
                .any(|c| c.schemas.is_empty() && two_level(self.dialect))
    }

    /// The pattern as canonical text, rebuilt from the syntax tree: no whitespace, names bare
    /// when they were written bare and quoted (with `""`) when they were quoted, regexes as
    /// written. `parse_scope(p.to_text())` gives `p` back.
    pub fn to_text(&self) -> &str {
        &self.text
    }

    pub fn dialect(&self) -> DialectId {
        self.dialect
    }

    pub fn clauses(&self) -> &[Clause] {
        &self.clauses
    }

    pub fn warnings(&self) -> &[ScopeWarning] {
        &self.warnings
    }
}

/// Whether `d` has databases holding schemas (`database:schema` clauses).
fn two_level(d: DialectId) -> bool {
    matches!(
        d,
        DialectId::Postgres | DialectId::PgWire(_) | DialectId::SqlServer
    )
}

struct Matcher<'a> {
    dialect: DialectId,
    ident: &'a IdentCtx,
    ctx: &'a CurrentCtx,
}

impl Matcher<'_> {
    fn clause(&self, c: &Clause, info: &NsInfo) -> bool {
        let path = &info.key.path;
        // A wildcard never *selects* a system namespace, but a negative wildcard removes one
        // (removing is always safe; fix round 1, M4).
        let system = info.system && !c.negative;
        if two_level(self.dialect) {
            let Some(db_term) = &c.db else {
                return false;
            };
            match (info.key.kind, path.len()) {
                (NsKind::Schema, 2) => {
                    let (db, s) = (&path[0], &path[1]);
                    // The namespace's own level is the schema: `*` skips a system schema.
                    if !self.term(db_term, db, Slot::Database, false) {
                        return false;
                    }
                    if c.schemas.is_empty() {
                        return self.term(&Term::Current, s, Slot::Schema, system);
                    }
                    c.schemas
                        .iter()
                        .any(|t| self.term(t, s, Slot::Schema, system))
                }
                (NsKind::Database, 1) => {
                    c.schemas.is_empty() && self.term(db_term, &path[0], Slot::Database, system)
                }
                _ => false,
            }
        } else {
            match (info.key.kind, path.len()) {
                (NsKind::Schema, 1) => c
                    .schemas
                    .iter()
                    .any(|t| self.term(t, &path[0], Slot::Schema, system)),
                _ => false,
            }
        }
    }

    /// Whether `info` is the session's current namespace (`@:@`), compared by the server's rule
    /// like any name (fix round 1, I3): `USE Shop` under MySQL `lower_case_table_names` 2
    /// leaves `DATABASE()` = `Shop` while the server lists `shop`.
    fn is_current(&self, info: &NsInfo) -> bool {
        let path = &info.key.path;
        match (info.key.kind, path.len(), two_level(self.dialect)) {
            (NsKind::Schema, 2, true) => {
                self.term(&Term::Current, &path[0], Slot::Database, false)
                    && self.term(&Term::Current, &path[1], Slot::Schema, false)
            }
            (NsKind::Schema, 1, false) => self.term(&Term::Current, &path[0], Slot::Schema, false),
            _ => false,
        }
    }

    /// Whether the listed name `listed` and the session's `current` text name one namespace
    /// under the server's rule for `class` (exact where no rule is pinned).
    fn same_name(&self, listed: &str, current: &str, class: NameClass) -> bool {
        match FoldRule::for_names(self.dialect, self.ident, class) {
            Some(rule) => rule.matches(listed, current),
            None => listed == current,
        }
    }

    /// Whether `t` in `slot` matches `name`; `system` is the namespace's own flag when `slot`
    /// is its own level (a wildcard never selects a system namespace).
    fn term(&self, t: &Term, name: &Name, slot: Slot, system: bool) -> bool {
        let class = match slot {
            Slot::Database => NameClass::Database,
            Slot::Schema => NameClass::Schema,
        };
        match t {
            Term::Current => {
                let cur = match slot {
                    Slot::Database => self.ctx.database.as_ref(),
                    Slot::Schema if two_level(self.dialect) => self.ctx.schema.as_ref(),
                    Slot::Schema => self.ctx.schema.as_ref().or(self.ctx.database.as_ref()),
                };
                cur.is_some_and(|c| self.same_name(name, c, class))
            }
            Term::All => !system,
            Term::Regex(r) => !system && r.is_match(name, self.case(class)),
            Term::Name(part) => {
                match match_key(self.dialect, self.ident, &part.text, part.quoted, class) {
                    Ok(key) => match FoldRule::for_names(self.dialect, self.ident, class) {
                        Some(rule) => rule.matches(name, key.as_str()),
                        None => **name == *key.as_str(),
                    },
                    // The server's rule for this text is not pinned (R-M4-41): exact text only.
                    Err(_) => *part.text == **name,
                }
            }
        }
    }

    fn case(&self, class: NameClass) -> GlobCase {
        match FoldRule::for_names(self.dialect, self.ident, class) {
            Some(FoldRule::AsciiCaseless) => GlobCase::AsciiInsensitive,
            _ => GlobCase::Exact,
        }
    }
}

/// Why a scope pattern does not parse.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScopeError {
    /// The 1-based position, in Unicode scalar values (never bytes).
    pub column: u32,
    pub kind: ScopeErrorKind,
}

/// The closed set of scope errors (`intro-scope-err-*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScopeErrorKind {
    /// A `{` without its `}` (reported at the `{`).
    UnclosedBrace,
    /// The text between the braces is not a valid regular expression (reported at the `{`).
    BadRegex,
    /// Any other syntax error (`intro-scope-syntax-error`).
    Syntax,
}

/// Parses a scope pattern of `dialect` (see the module docs).
pub fn parse_scope(text: &str, dialect: DialectId) -> Result<ScopePattern, ScopeError> {
    let mut p = Parser {
        cs: text.chars().collect(),
        i: 0,
        two: two_level(dialect),
        warnings: Vec::new(),
    };
    let mut clauses = Vec::new();
    p.ws();
    if !p.eof() {
        loop {
            clauses.push(p.clause()?);
            p.ws();
            match p.peek() {
                None => break,
                Some('|') => {
                    p.i += 1;
                    p.ws();
                }
                Some(_) => return Err(p.err(ScopeErrorKind::Syntax)),
            }
        }
    }
    let text = render(&clauses);
    Ok(ScopePattern {
        dialect,
        clauses,
        text,
        warnings: p.warnings,
    })
}

struct Parser {
    cs: Vec<char>,
    i: usize,
    two: bool,
    warnings: Vec<ScopeWarning>,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.cs.get(self.i).copied()
    }

    fn eof(&self) -> bool {
        self.i >= self.cs.len()
    }

    fn ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.i += 1;
        }
    }

    fn column(at: usize) -> u32 {
        u32::try_from(at + 1).unwrap_or(u32::MAX)
    }

    /// An error at the current character. A `{` that is out of place and never closes is
    /// reported as the unclosed brace it is (`db{1:x`: character 3).
    fn err(&self, kind: ScopeErrorKind) -> ScopeError {
        let kind = match kind {
            ScopeErrorKind::Syntax
                if self.peek() == Some('{') && self.brace_end(self.i).is_none() =>
            {
                ScopeErrorKind::UnclosedBrace
            }
            k => k,
        };
        ScopeError {
            column: Self::column(self.i),
            kind,
        }
    }

    /// The index of the `}` balancing the `{` at `open` (`\` escapes the next character).
    fn brace_end(&self, open: usize) -> Option<usize> {
        let mut depth = 0usize;
        let mut j = open;
        loop {
            match self.cs.get(j)? {
                '\\' => j += 2,
                '{' => {
                    depth += 1;
                    j += 1;
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(j);
                    }
                    j += 1;
                }
                _ => j += 1,
            }
        }
    }

    fn clause(&mut self) -> Result<Clause, ScopeError> {
        let negative = self.peek() == Some('!');
        if negative {
            self.i += 1;
            self.ws();
        }
        let first_at = self.i;
        let first = self.term()?;
        self.ws();
        if self.two {
            let schemas = if self.peek() == Some(':') {
                self.i += 1;
                self.ws();
                self.schlist()?
            } else {
                Vec::new()
            };
            return Ok(Clause {
                negative,
                db: Some(first),
                schemas,
            });
        }
        if self.peek() == Some(':') {
            if first != Term::All {
                self.warnings.push(ScopeWarning {
                    column: Self::column(first_at),
                    kind: ScopeWarningKind::DbTermIgnored,
                });
            }
            self.i += 1;
            self.ws();
            let schemas = self.schlist()?;
            return Ok(Clause {
                negative,
                db: Some(first),
                schemas,
            });
        }
        let mut schemas = vec![first];
        self.more_terms(&mut schemas)?;
        Ok(Clause {
            negative,
            db: None,
            schemas,
        })
    }

    fn schlist(&mut self) -> Result<Vec<Term>, ScopeError> {
        let mut out = vec![self.term()?];
        self.ws();
        self.more_terms(&mut out)?;
        Ok(out)
    }

    fn more_terms(&mut self, out: &mut Vec<Term>) -> Result<(), ScopeError> {
        while self.peek() == Some(',') {
            self.i += 1;
            self.ws();
            out.push(self.term()?);
            self.ws();
        }
        Ok(())
    }

    fn term(&mut self) -> Result<Term, ScopeError> {
        match self.peek() {
            Some('@') => {
                self.i += 1;
                Ok(Term::Current)
            }
            Some('*') => {
                self.i += 1;
                Ok(Term::All)
            }
            Some('{') => self.regex(),
            Some('"') => self.quoted(),
            Some(c) if bare(c) => {
                let start = self.i;
                while self.peek().is_some_and(bare) {
                    self.i += 1;
                }
                Ok(Term::Name(NamePart {
                    text: self.cs[start..self.i].iter().collect::<String>().into(),
                    quoted: false,
                }))
            }
            _ => Err(self.err(ScopeErrorKind::Syntax)),
        }
    }

    fn quoted(&mut self) -> Result<Term, ScopeError> {
        let open = self.i;
        self.i += 1;
        let mut text = String::new();
        loop {
            match self.peek() {
                None => {
                    return Err(ScopeError {
                        column: Self::column(open),
                        kind: ScopeErrorKind::Syntax,
                    });
                }
                Some('"') if self.cs.get(self.i + 1) == Some(&'"') => {
                    text.push('"');
                    self.i += 2;
                }
                Some('"') => {
                    self.i += 1;
                    break;
                }
                Some(c) => {
                    text.push(c);
                    self.i += 1;
                }
            }
        }
        if text.is_empty() {
            return Err(ScopeError {
                column: Self::column(open),
                kind: ScopeErrorKind::Syntax,
            });
        }
        Ok(Term::Name(NamePart {
            text: text.into(),
            quoted: true,
        }))
    }

    fn regex(&mut self) -> Result<Term, ScopeError> {
        let open = self.i;
        let at = |kind| ScopeError {
            column: Self::column(open),
            kind,
        };
        let j = self
            .brace_end(open)
            .ok_or_else(|| at(ScopeErrorKind::UnclosedBrace))?;
        let source: String = self.cs[open + 1..j].iter().collect();
        self.i = j + 1;
        let (exact, caseless) =
            full_match_pair(&source).map_err(|_| at(ScopeErrorKind::BadRegex))?;
        Ok(Term::Regex(ScopeRegex {
            source,
            exact,
            caseless,
        }))
    }
}

/// A character a bare name may hold.
fn bare(c: char) -> bool {
    !(c.is_whitespace() || matches!(c, ':' | ',' | '|' | '{' | '}' | '!' | '"' | '@' | '*'))
}

fn render(clauses: &[Clause]) -> String {
    let mut out = String::new();
    for (n, c) in clauses.iter().enumerate() {
        if n > 0 {
            out.push('|');
        }
        if c.negative {
            out.push('!');
        }
        if let Some(db) = &c.db {
            render_term(&mut out, db);
            if !c.schemas.is_empty() {
                out.push(':');
            }
        }
        for (k, t) in c.schemas.iter().enumerate() {
            if k > 0 {
                out.push(',');
            }
            render_term(&mut out, t);
        }
    }
    out
}

fn render_term(out: &mut String, t: &Term) {
    match t {
        Term::Current => out.push('@'),
        Term::All => out.push('*'),
        Term::Regex(r) => {
            out.push('{');
            out.push_str(&r.source);
            out.push('}');
        }
        Term::Name(p) if p.quoted => {
            out.push('"');
            out.push_str(&p.text.replace('"', "\"\""));
            out.push('"');
        }
        Term::Name(p) => out.push_str(&p.text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(path: &[&str]) -> NsInfo {
        NsInfo {
            key: NsKey::schema(path.iter().copied()),
            system: false,
        }
    }

    fn ctx(db: Option<&str>, schema: Option<&str>) -> CurrentCtx {
        CurrentCtx {
            database: db.map(Name::from),
            schema: schema.map(Name::from),
            ident: None,
        }
    }

    #[test]
    fn one_level_star_selects_every_listed_schema() {
        let infos = [info(&["shop"]), info(&["orders"])];
        let p = parse_scope("*", DialectId::MySql).unwrap();
        let got = p.select(&infos, &ctx(Some("shop"), Some("shop")));
        assert_eq!(got.len(), 2);
        assert_eq!(p.to_text(), "*");
    }

    #[test]
    fn the_current_namespace_is_never_invented() {
        let infos = [info(&["db", "public"])];
        let p = parse_scope("", DialectId::Postgres).unwrap();
        assert!(p.select(&infos, &ctx(Some("db"), None)).is_empty());
        assert!(p.select(&infos, &ctx(Some("db"), Some("app"))).is_empty());
        assert_eq!(
            p.select(&infos, &ctx(Some("db"), Some("public"))),
            BTreeSet::from([NsKey::schema(["db", "public"])])
        );
    }

    #[test]
    fn regex_depth_skips_escaped_braces() {
        let p = parse_scope(r"{a\}b}", DialectId::MySql).unwrap();
        assert_eq!(p.to_text(), r"{a\}b}");
        let got = p.select(&[info(&["a}b"]), info(&["ab"])], &CurrentCtx::default());
        assert_eq!(got, BTreeSet::from([NsKey::schema(["a}b"])]));
    }
}
