//! The catalog-SQL lint (M4 Task 4; alg/02 HV-INTRO-14 rules 1–2, HV-INTRO-07, R-M4-8,
//! R-M4-46). Session-free. It reads every line of every branch, so a gated variant cannot
//! hide a finding; it runs in `tests/pack.rs` over the built-in packs (empty findings) and in
//! the introspectors' own tests.
//!
//! What it refuses:
//! - **Canonical catalog spelling** (rule 2): PostgreSQL writes `pg_catalog`,
//!   `information_schema` and every `pg_*` name in lower case, and every `pg_*` name (tables,
//!   views and functions) qualified as `pg_catalog.<name>` (a hostile `search_path` cannot
//!   shadow it); MySQL/MariaDB write `INFORMATION_SCHEMA.<UPPER>` and `mysql.`,
//!   `performance_schema.`, `sys.` in lower case; SQLite writes `sqlite_schema` (never
//!   `sqlite_master`) and `sqlite_*`/`pragma_*` names in lower case.
//! - **Unqualified PostgreSQL catalog functions** (R-M4-46, fix round 1 m1): the catalog
//!   functions without a `pg_` prefix that packs use (`format_type`, `obj_description`,
//!   `has_*_privilege`, `age`, …) carry `pg_catalog.` in every section; in a `joinable`
//!   section, where the `search_path` is not pinned, **every** function call does.
//! - **Case folding in catalog SQL** (rule 1): `lower(`, `upper(`, `lcase(`, `ucase(`,
//!   `casefold(`, `initcap(`, `ILIKE`, `COLLATE` and PostgreSQL's `~*`, `!~*`, `~~*`, `!~~*`
//!   anywhere in code (a pack compares exact names, bound; a Turkish collation breaks every
//!   folding trick).
//! - MySQL/MariaDB `UPDATE_TIME` and `GROUP_CONCAT` (HV-INTRO-07; key parts are grouped
//!   client-side).
//! - PostgreSQL: any `age(…)` call outside the R-M4-8 shape `age(x) BETWEEN 0 AND age($W::xid)`
//!   — through a cast, parentheses or arithmetic included (fix round 1, I4).
//! - Every dialect: `txid_current` and `pg_current_xact_id` (they assign an XID).
//! - Directive-looking text (`--#`, `---`) inside a literal or quoted identifier.
//! - A quote character inside a comment of a section (R-W3-9, Task 33a): the runtime guard
//!   would refuse the rendered text.
//!
//! What the parser already refuses never reaches the lint: nesting deeper than `MAX_NESTING`,
//! MySQL executable comments, `#{ident:…}` outside a `show-create` section (fix round 1).

use hacivat_meta::Name;
use sql_dialect::DialectId;

use super::parse::{Family, Piece, SYNTAX, Tok, scan_line, tokenize, walk_lines};
use super::{DraftPack, Section};

/// One lint finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LintError {
    pub file: &'static str,
    pub line: u32,
    pub section: Name,
    pub kind: LintKind,
}

/// The closed set of lint findings; a `String` holds the word as written in the pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LintKind {
    /// A catalog name not in its canonical case (or `sqlite_master`).
    NonCanonicalCatalogName(String),
    /// A PostgreSQL catalog name or function without `pg_catalog.`.
    UnqualifiedPgCatalog(String),
    /// A case-folding function, operator or `COLLATE`.
    CaseFoldingOnCatalogName(String),
    /// MySQL `UPDATE_TIME`.
    UpdateTime,
    /// MySQL `GROUP_CONCAT`.
    GroupConcat,
    /// A PostgreSQL `age(…)` outside `age(x) BETWEEN 0 AND age($W::xid)` (R-M4-8).
    OrderingOnAge,
    /// `txid_current` or `pg_current_xact_id`.
    XidAssigningFunction(String),
    /// `--#` or `---` inside a literal or quoted identifier.
    DirectiveInLiteral,
    // ## m4-t33a-hygiene (named exception in Task 4's file; R-W3-9)
    /// A quote character (`'`, `"` or a backtick) inside a comment of a section. The comment is
    /// rendered with the section, and the metadata connection's guard lexes it in every
    /// dialect context of the engine, where a quote can open a literal: the guard then refuses
    /// every read of the section (`RefusedText`, found by Task 18). File-header comments are
    /// outside every section and never rendered.
    QuoteInComment,
    /// In a `joinable` PostgreSQL section (whose `search_path` Hacivat cannot pin): an
    /// operator not written `OPERATOR(pg_catalog.op)`, or a construct that picks an operator
    /// by name through the path (`LIKE`, `BETWEEN`, `NULLIF`, `IN` without a list bind,
    /// `IS DISTINCT FROM`, a simple `CASE`, `JOIN … USING`, `NATURAL JOIN`; T16 verification
    /// H8, the CVE-2018-1058 class).
    PathResolvedOperator(String),
    /// In a `joinable` PostgreSQL section: a type name that resolves through the path, after
    /// `::` or the `AS` of a `CAST(`, or in a typed literal (`text 'a'`); every type is written
    /// `pg_catalog.<type>` (T33a review M2). The `String` is the word found where
    /// `pg_catalog` belongs.
    PathResolvedType(String),
    // ## m4-t33a-hygiene
}

fn eq(w: &str, s: &str) -> bool {
    w.eq_ignore_ascii_case(s)
}

fn has_upper(w: &str) -> bool {
    w.bytes().any(|b| b.is_ascii_uppercase())
}

fn has_lower(w: &str) -> bool {
    w.bytes().any(|b| b.is_ascii_lowercase())
}

fn starts_with_ci(w: &str, prefix: &str) -> bool {
    w.len() >= prefix.len()
        && w.is_char_boundary(prefix.len())
        && w[..prefix.len()].eq_ignore_ascii_case(prefix)
}

fn ends_with_ci(w: &str, suffix: &str) -> bool {
    w.len() >= suffix.len()
        && w.is_char_boundary(w.len() - suffix.len())
        && w[w.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

/// Case-folding functions (rule 1).
const FOLDS: [&str; 6] = ["lower", "upper", "lcase", "ucase", "casefold", "initcap"];

/// PostgreSQL catalog functions without a `pg_` prefix that every section qualifies.
const PG_UNPREFIXED: [&str; 22] = [
    "age",
    "format_type",
    "obj_description",
    "col_description",
    "shobj_description",
    "quote_ident",
    "quote_literal",
    "quote_nullable",
    "current_setting",
    "current_database",
    "current_schema",
    "current_schemas",
    "current_user",
    "session_user",
    "to_regclass",
    "to_regtype",
    "to_regproc",
    "to_regprocedure",
    "to_regnamespace",
    "array_to_string",
    "string_to_array",
    "unnest",
];

/// Lints every section of `pack` with `dialect`'s rules.
pub fn lint<P: AsRef<DraftPack>>(pack: &P, dialect: DialectId) -> Vec<LintError> {
    let pack = pack.as_ref();
    let Some(rules) = Family::of(dialect) else {
        return Vec::new();
    };
    let lexer = pack.inner.family;
    let mut out = Vec::new();
    for sec in &pack.inner.sections {
        lint_section(sec, lexer, rules, &mut out);
    }
    out
}

fn lint_section(sec: &Section, lexer: Family, rules: Family, out: &mut Vec<LintError>) {
    let mut push = |line: u32, kind: LintKind| {
        out.push(LintError {
            file: sec.file,
            line,
            section: sec.name.clone(),
            kind,
        });
    };
    let mut toks: Vec<(u32, Tok<'_>)> = Vec::new();
    // ## m4-t33a-hygiene (named exception; H8): the indices of quoted names, which are names
    // and never SQL syntax.
    let mut quoted: std::collections::HashSet<usize> = std::collections::HashSet::new();
    // ## m4-t33a-hygiene (fix round 1, M2): the indices of string literals (typed literals).
    let mut literals: std::collections::HashSet<usize> = std::collections::HashSet::new();
    // ## m4-t33a-hygiene
    walk_lines(&sec.body, &mut |l| {
        // The parser accepted the line, so it lexes again.
        let Ok(pieces) = scan_line(lexer, &l.raw) else {
            return;
        };
        for p in pieces {
            match p {
                Piece::Code(c) => tokenize(c, l.line, &mut toks),
                Piece::Quoted(q) => {
                    if q.contains("--#") || q.contains("---") {
                        push(l.line, LintKind::DirectiveInLiteral);
                    }
                    // A quoted identifier is still a name the rules read: `"lower"(x)` calls
                    // `lower`, `"pg_class"` is unqualified. (`"` quotes a string on MySQL
                    // without ANSI_QUOTES; reading it as a name there only adds findings.)
                    let ident = q.starts_with('"') || q.starts_with('`');
                    let tok = if ident {
                        quoted.insert(toks.len());
                        Tok::Word(&q[1..q.len() - 1])
                    } else {
                        literals.insert(toks.len());
                        Tok::Operand
                    };
                    toks.push((l.line, tok));
                }
                // ## m4-t33a-hygiene (named exception; R-W3-9)
                Piece::Comment(c) => {
                    if c.contains(['\'', '"', '`']) {
                        push(l.line, LintKind::QuoteInComment);
                    }
                }
                // ## m4-t33a-hygiene
                Piece::Bind(_) | Piece::Macro(_) => toks.push((l.line, Tok::Operand)),
                Piece::List(_) => toks.push((l.line, Tok::List)),
            }
        }
    });

    let word = |k: usize| match toks.get(k) {
        Some((_, Tok::Word(w))) => Some(*w),
        _ => None,
    };
    let op = |k: usize| match toks.get(k) {
        Some((_, Tok::Op(o))) => Some(*o),
        _ => None,
    };
    // The word two tokens back when the token before is `.`: the qualifier of `toks[k]`.
    let qualifier = |k: usize| {
        if k >= 2 && op(k - 1) == Some(".") {
            word(k - 2)
        } else {
            None
        }
    };

    // ## m4-t33a-hygiene (named exception; T16 verification H8)
    if rules == Family::Pg && sec.joinable {
        for (line, w) in path_resolved_operators(&toks, &quoted) {
            push(line, LintKind::PathResolvedOperator(w));
        }
        // ## m4-t33a-hygiene (fix round 1, M2)
        for (line, w) in path_resolved_types(&toks, &quoted, &literals) {
            push(line, LintKind::PathResolvedType(w));
        }
    }
    // ## m4-t33a-hygiene
    let valid_ages = between_age_calls(&toks);
    for (k, &(line, tok)) in toks.iter().enumerate() {
        match tok {
            Tok::Op(o @ ("~*" | "!~*" | "~~*" | "!~~*")) => {
                push(line, LintKind::CaseFoldingOnCatalogName(o.to_string()));
            }
            Tok::Op(_) | Tok::Operand | Tok::List => {}
            Tok::Word(w) => {
                let call = op(k + 1) == Some("(");
                if eq(w, "txid_current") || eq(w, "pg_current_xact_id") {
                    push(line, LintKind::XidAssigningFunction(w.to_string()));
                }
                if (call && FOLDS.iter().any(|f| eq(w, f))) || eq(w, "ilike") || eq(w, "collate") {
                    push(line, LintKind::CaseFoldingOnCatalogName(w.to_string()));
                }
                match rules {
                    Family::Pg => {
                        let q = qualifier(k);
                        let in_catalog = q.is_some_and(|q| eq(q, "pg_catalog"));
                        if eq(w, "pg_catalog")
                            || eq(w, "information_schema")
                            || starts_with_ci(w, "pg_")
                            || q.is_some_and(|q| eq(q, "information_schema"))
                        {
                            if has_upper(w) {
                                push(line, LintKind::NonCanonicalCatalogName(w.to_string()));
                            }
                            if starts_with_ci(w, "pg_") && !eq(w, "pg_catalog") && !in_catalog {
                                push(line, LintKind::UnqualifiedPgCatalog(w.to_string()));
                            }
                        } else if call && !in_catalog {
                            let known = PG_UNPREFIXED.iter().any(|f| eq(w, f))
                                || (starts_with_ci(w, "has_") && ends_with_ci(w, "_privilege"));
                            let after_type = k >= 1
                                && (op(k - 1) == Some("::")
                                    || word(k - 1).is_some_and(|p| eq(p, "as")));
                            // ## m4-t33a-hygiene (named exception; H8): a quoted `"operator"`
                            // is a name, so only unquoted words are SQL syntax.
                            let syntax = !quoted.contains(&k) && SYNTAX.iter().any(|s| eq(w, s));
                            if known || (sec.joinable && !after_type && !syntax) {
                                push(line, LintKind::UnqualifiedPgCatalog(w.to_string()));
                            }
                        }
                        if eq(w, "age") && call && !valid_ages.contains(&k) {
                            push(line, LintKind::OrderingOnAge);
                        }
                    }
                    Family::MySql => {
                        if eq(w, "information_schema") && w != "INFORMATION_SCHEMA" {
                            push(line, LintKind::NonCanonicalCatalogName(w.to_string()));
                        }
                        if qualifier(k).is_some_and(|q| eq(q, "information_schema")) && has_lower(w)
                        {
                            push(line, LintKind::NonCanonicalCatalogName(w.to_string()));
                        }
                        if ["mysql", "performance_schema", "sys"]
                            .iter()
                            .any(|s| eq(w, s))
                            && op(k + 1) == Some(".")
                            && has_upper(w)
                        {
                            push(line, LintKind::NonCanonicalCatalogName(w.to_string()));
                        }
                        if eq(w, "update_time") {
                            push(line, LintKind::UpdateTime);
                        }
                        if eq(w, "group_concat") {
                            push(line, LintKind::GroupConcat);
                        }
                    }
                    Family::Sqlite => {
                        // `sqlite_schema` is canonical (`sqlite_master` is its legacy alias),
                        // and SQLite's own names are lower case.
                        let legacy = eq(w, "sqlite_master") || eq(w, "sqlite_temp_master");
                        let ours = starts_with_ci(w, "sqlite_") || starts_with_ci(w, "pragma_");
                        if legacy || (ours && has_upper(w)) {
                            push(line, LintKind::NonCanonicalCatalogName(w.to_string()));
                        }
                    }
                }
            }
        }
    }
}

// ## m4-t33a-hygiene (T16 verification H8)
/// The operators of a `joinable` PostgreSQL section that resolve through the `search_path`:
/// every operator token outside `OPERATOR ( pg_catalog . <op> )` (structure — `(` `)` `,` `.`
/// `::` `[` `]` — and the `*` of `count(*)`, `SELECT *` or `t.*` excepted), `OPERATOR(` with
/// any other qualifier, and the unquoted words that pick an operator by name.
fn path_resolved_operators(
    toks: &[(u32, Tok<'_>)],
    quoted: &std::collections::HashSet<usize>,
) -> Vec<(u32, String)> {
    let word_is = |j: usize, w: &str| {
        !quoted.contains(&j) && matches!(toks.get(j), Some((_, Tok::Word(x))) if eq(x, w))
    };
    let op_is = |j: usize, o: &str| matches!(toks.get(j), Some((_, Tok::Op(x))) if *x == o);
    let mut out = Vec::new();
    let mut k = 0;
    while k < toks.len() {
        let (line, tok) = toks[k];
        match tok {
            Tok::Word(_) if quoted.contains(&k) => {}
            Tok::Word(w) => {
                if eq(w, "operator") && op_is(k + 1, "(") {
                    if word_is(k + 2, "pg_catalog") && op_is(k + 3, ".") {
                        let mut j = k + 4;
                        while matches!(toks.get(j), Some((_, Tok::Op(o))) if *o != ")") {
                            j += 1;
                        }
                        if j > k + 4 && op_is(j, ")") {
                            k = j + 1;
                            continue;
                        }
                    }
                    out.push((line, "OPERATOR".to_string()));
                    // One finding for the whole `OPERATOR(…)`.
                    let mut j = k + 2;
                    while toks.get(j).is_some_and(|t| t.1 != Tok::Op(")")) {
                        j += 1;
                    }
                    k = j + 1;
                    continue;
                } else if ["like", "ilike", "similar", "between", "nullif"]
                    .iter()
                    .any(|b| eq(w, b))
                {
                    out.push((line, w.to_ascii_uppercase()));
                } else if eq(w, "in") && !matches!(toks.get(k + 1), Some((_, Tok::List))) {
                    out.push((line, "IN".to_string()));
                } else if eq(w, "is")
                    && (word_is(k + 1, "distinct")
                        || (word_is(k + 1, "not") && word_is(k + 2, "distinct")))
                {
                    out.push((line, "IS DISTINCT FROM".to_string()));
                } else if eq(w, "case") && !word_is(k + 1, "when") {
                    out.push((line, "CASE".to_string()));
                } else if eq(w, "using") || eq(w, "natural") {
                    // Fix round 1 (review M1): `JOIN … USING (col)` and `NATURAL JOIN` build an
                    // unqualified `=` (`transformJoinUsingClause`); `ORDER BY … USING op` is
                    // refused here too, beside its operator.
                    out.push((line, w.to_ascii_uppercase()));
                }
            }
            Tok::Op("(" | ")" | "," | "." | "::" | "[" | "]") => {}
            Tok::Op("*")
                if k == 0
                    || op_is(k - 1, "(")
                    || op_is(k - 1, ".")
                    || op_is(k - 1, ",")
                    || word_is(k - 1, "select") => {}
            Tok::Op(o) => out.push((line, o.to_string())),
            Tok::Operand | Tok::List => {}
        }
        k += 1;
    }
    out
}
// ## m4-t33a-hygiene

// ## m4-t33a-hygiene (fix round 1, review M2)
/// Words after which a string literal is a plain operand, never a typed literal's value.
const BEFORE_LITERAL: [&str; 36] = [
    "select", "then", "else", "when", "and", "or", "not", "is", "by", "on", "where", "from",
    "having", "limit", "offset", "values", "return", "escape", "like", "ilike", "similar", "to",
    "between", "symmetric", "distinct", "in", "all", "any", "some", "case", "end", "as", "with",
    "union", "except", "intersect",
];

/// The type names of a `joinable` PostgreSQL section that resolve through the `search_path`:
/// the type after `::` and after the `AS` of a `CAST(` must be `pg_catalog.<type>` (an
/// unquoted `pg_catalog`), and a typed literal (`<type> '…'`) must name `pg_catalog.<type>`.
/// Answers the word found where `pg_catalog` belongs.
fn path_resolved_types(
    toks: &[(u32, Tok<'_>)],
    quoted: &std::collections::HashSet<usize>,
    literals: &std::collections::HashSet<usize>,
) -> Vec<(u32, String)> {
    let word_is = |j: usize, w: &str| {
        !quoted.contains(&j) && matches!(toks.get(j), Some((_, Tok::Word(x))) if eq(x, w))
    };
    let op_is = |j: usize, o: &str| matches!(toks.get(j), Some((_, Tok::Op(x))) if *x == o);
    let shown = |j: usize| match toks.get(j) {
        Some((_, Tok::Word(w))) => (*w).to_string(),
        Some((_, Tok::Op(o))) => (*o).to_string(),
        _ => "?".to_string(),
    };
    // The type starting at `j` is `pg_catalog.<word>`.
    let qualified = |j: usize| {
        word_is(j, "pg_catalog")
            && op_is(j + 1, ".")
            && matches!(toks.get(j + 2), Some((_, Tok::Word(_))))
    };
    let mut out = Vec::new();
    for (k, &(line, tok)) in toks.iter().enumerate() {
        // `x::type`.
        if tok == Tok::Op("::") && !qualified(k + 1) {
            out.push((line, shown(k + 1)));
        }
        // `CAST(x AS type)`: the `AS` at the call's own depth.
        if word_is(k, "cast") && op_is(k + 1, "(") {
            let mut depth = 0usize;
            let mut j = k + 1;
            while let Some((_, t)) = toks.get(j) {
                match t {
                    Tok::Op("(") => depth += 1,
                    Tok::Op(")") => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ if depth == 1 && word_is(j, "as") => {
                        if !qualified(j + 1) {
                            out.push((toks[j].0, shown(j + 1)));
                        }
                        break;
                    }
                    _ => {}
                }
                j += 1;
            }
        }
        // A typed literal: a type name right before a string literal.
        if let Tok::Word(w) = tok
            && literals.contains(&(k + 1))
            && (quoted.contains(&k) || !BEFORE_LITERAL.iter().any(|b| eq(w, b)))
        {
            let after_dot = k >= 1 && op_is(k - 1, ".");
            let in_catalog = after_dot && k >= 2 && word_is(k - 2, "pg_catalog");
            if !in_catalog {
                out.push((line, if after_dot { shown(k - 2) } else { w.to_string() }));
            }
        }
    }
    out
}
// ## m4-t33a-hygiene

/// The index just past the call whose name is at `k` (`name(` … `)`) and any `::type`
/// casts after it.
fn past_call(toks: &[(u32, Tok<'_>)], k: usize) -> usize {
    let mut depth = 0usize;
    let mut j = k + 1;
    while let Some((_, t)) = toks.get(j) {
        match t {
            Tok::Op("(") => depth += 1,
            Tok::Op(")") => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        j += 1;
    }
    j += 1;
    while matches!(toks.get(j), Some((_, Tok::Op("::")))) {
        j += 2;
        while matches!(toks.get(j), Some((_, Tok::Op("."))))
            && matches!(toks.get(j + 1), Some((_, Tok::Word(_))))
        {
            j += 2;
        }
    }
    j
}

/// The `age(` calls (indices of the name token) that form `age(x) BETWEEN 0 AND age(y)`
/// exactly (R-M4-8; fix round 2, m9): the lower operand is an `age(` call (casts allowed) with
/// a boolean boundary before it (`WHERE`, `AND`, `OR`, `NOT`, `ON`, `WHEN`, `(`, `,`, …), the
/// bounds are `BETWEEN 0 AND` and an `age(` call, and a boundary follows the upper bound
/// (`AND`, `OR`, `)`, `,`, a clause word or the end). So no arithmetic, comparison,
/// `NOT BETWEEN`, `SYMMETRIC`, other lower bound or non-`age` lower operand passes.
fn between_age_calls(toks: &[(u32, Tok<'_>)]) -> std::collections::HashSet<usize> {
    let is = |j: usize, w: &str| matches!(toks.get(j), Some((_, Tok::Word(x))) if x.eq_ignore_ascii_case(w));
    // `age(` at `j`, possibly `pg_catalog.`-qualified; answers (first token, name token).
    let age_at = |j: usize| -> Option<(usize, usize)> {
        let name = if is(j, "pg_catalog") && matches!(toks.get(j + 1), Some((_, Tok::Op(".")))) {
            j + 2
        } else {
            j
        };
        (is(name, "age") && matches!(toks.get(name + 1), Some((_, Tok::Op("(")))))
            .then_some((j, name))
    };
    let before_ok = |first: usize| match first.checked_sub(1).and_then(|b| toks.get(b)) {
        None => true,
        Some((_, Tok::Op("(" | ","))) => true,
        Some((_, Tok::Word(w))) => [
            "where", "and", "or", "not", "on", "when", "then", "else", "select",
        ]
        .iter()
        .any(|b| w.eq_ignore_ascii_case(b)),
        Some(_) => false,
    };
    let after_ok = |j: usize| match toks.get(j) {
        None => true,
        Some((_, Tok::Op(")" | ","))) => true,
        Some((_, Tok::Word(w))) => [
            "and", "or", "then", "else", "end", "order", "group", "limit", "having", "union",
            "from", "as",
        ]
        .iter()
        .any(|b| w.eq_ignore_ascii_case(b)),
        Some(_) => false,
    };
    let mut out = std::collections::HashSet::new();
    for first in 0..toks.len() {
        let Some((first, lower)) = age_at(first) else {
            continue;
        };
        // A qualified call matches at its `pg_catalog` token; seen from the bare `age` token it
        // has `.` before it, which is no boundary.
        let b = past_call(toks, lower);
        if !(before_ok(first) && is(b, "between") && is(b + 1, "0") && is(b + 2, "and")) {
            continue;
        }
        let Some((_, upper)) = age_at(b + 3) else {
            continue;
        };
        if after_ok(past_call(toks, upper)) {
            out.insert(lower);
            out.insert(upper);
        }
    }
    out
}
