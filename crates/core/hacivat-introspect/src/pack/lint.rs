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
//!
//! What the parser already refuses never reaches the lint: nesting deeper than `MAX_NESTING`,
//! MySQL executable comments, `#{ident:…}` outside a `show-create` section (fix round 1).

use hacivat_meta::Name;
use sql_dialect::DialectId;

use super::parse::{Family, Piece, Tok, scan_line, tokenize, walk_lines};
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

/// Words followed by `(` that are SQL syntax, not a function call (`joinable` rule).
const SYNTAX: [&str; 54] = [
    "in",
    "exists",
    "any",
    "all",
    "some",
    "array",
    "values",
    "as",
    "cast",
    "coalesce",
    "nullif",
    "greatest",
    "least",
    "row",
    "over",
    "filter",
    "join",
    "from",
    "using",
    "on",
    "and",
    "or",
    "not",
    "where",
    "select",
    "when",
    "then",
    "else",
    "between",
    "is",
    "case",
    "lateral",
    "by",
    "with",
    "recursive",
    "partition",
    "extract",
    "position",
    "substring",
    "trim",
    "overlay",
    "collation",
    "interval",
    "within",
    "table",
    "into",
    "returning",
    "union",
    "intersect",
    "except",
    "like",
    "distinct",
    "end",
    "having",
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
                        Tok::Word(&q[1..q.len() - 1])
                    } else {
                        Tok::Operand
                    };
                    toks.push((l.line, tok));
                }
                Piece::Comment(_) => {}
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
                            let syntax = SYNTAX.iter().any(|s| eq(w, s));
                            if known || (sec.joinable && !after_type && !syntax) {
                                push(line, LintKind::UnqualifiedPgCatalog(w.to_string()));
                            }
                        }
                        if eq(w, "age") && call && !age_in_between_shape(&toks, k, q.is_some()) {
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

/// Whether the `age(` call at `k` is one side of `age(x) BETWEEN 0 AND age(y)` (R-M4-8):
/// followed — after its arguments and any `::type` casts — by `BETWEEN`, or the upper bound
/// right after `BETWEEN 0 AND`. Any other position (a comparison, parentheses around it,
/// arithmetic, a cast then a comparison) is not.
fn age_in_between_shape(toks: &[(u32, Tok<'_>)], k: usize, qualified: bool) -> bool {
    let is = |j: Option<usize>, w: &str| matches!(j.and_then(|j| toks.get(j)), Some((_, Tok::Word(x))) if x.eq_ignore_ascii_case(w));
    // The upper bound: `BETWEEN 0 AND [pg_catalog.]age(`.
    let start = if qualified { k.checked_sub(2) } else { Some(k) };
    if let Some(s) = start
        && is(s.checked_sub(1), "and")
        && is(s.checked_sub(2), "0")
        && is(s.checked_sub(3), "between")
    {
        return true;
    }
    // The lower operand: `age(…)` [`::` type …] `BETWEEN`.
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
    // … `BETWEEN 0 AND [pg_catalog.]age(`: the upper bound is itself an `age(` call.
    let upper =
        if is(Some(j + 3), "pg_catalog") && matches!(toks.get(j + 4), Some((_, Tok::Op(".")))) {
            j + 5
        } else {
            j + 3
        };
    is(Some(j), "between")
        && is(Some(j + 1), "0")
        && is(Some(j + 2), "and")
        && is(Some(upper), "age")
        && matches!(toks.get(upper + 1), Some((_, Tok::Op("("))))
}
