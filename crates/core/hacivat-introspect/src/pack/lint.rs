//! The catalog-SQL lint (M4 Task 4; alg/02 HV-INTRO-14 rules 1–2, HV-INTRO-07, R-M4-8,
//! R-M4-37). Session-free. It reads every line of every branch, so a gated variant cannot
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
//! - **Case folding in catalog SQL** (rule 1): `lower(`, `upper(`, `lcase(`, `ucase(`,
//!   `ILIKE`, `COLLATE` and PostgreSQL's `~*`, `!~*`, `~~*`, `!~~*` anywhere in code (a pack
//!   compares exact names, bound; a Turkish collation breaks every folding trick).
//! - MySQL/MariaDB `UPDATE_TIME` and `GROUP_CONCAT` (HV-INTRO-07; key parts are grouped
//!   client-side), and MySQL executable comments `/*!` and `/*M!` (ver/01 `#mysql-comments`).
//! - PostgreSQL: an ordering comparison (`<`, `<=`, `>`, `>=`) on an `age(…)` call (R-M4-8:
//!   the change predicate is `age(x) BETWEEN 0 AND age($W::xid)`).
//! - Every dialect: `txid_current` and `pg_current_xact_id` (they assign an XID).
//! - `#{ident:…}` outside a `show-create` section (R-M4-37).
//! - Directive-looking text (`--#`, `---`) inside a literal or quoted identifier.
//!
//! Nesting deeper than `MAX_NESTING` never reaches the lint: the parser refuses it.

use hacivat_meta::Name;
use sql_dialect::DialectId;

use super::parse::{Family, Piece, scan_line, walk_lines};
use super::{Pack, Section};

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
    /// A PostgreSQL `pg_*` name without `pg_catalog.`.
    UnqualifiedPgCatalog(String),
    /// A case-folding function, operator or `COLLATE`.
    CaseFoldingOnCatalogName(String),
    /// MySQL `UPDATE_TIME`.
    UpdateTime,
    /// MySQL `GROUP_CONCAT`.
    GroupConcat,
    /// A MySQL/MariaDB executable comment.
    ExecutableComment,
    /// `<`, `<=`, `>` or `>=` on a PostgreSQL `age(…)`.
    OrderingOnAge,
    /// `txid_current` or `pg_current_xact_id`.
    XidAssigningFunction(String),
    /// `#{ident:…}` outside a `show-create` section.
    IdentMacroOutsideShowCreate,
    /// `--#` or `---` inside a literal or quoted identifier.
    DirectiveInLiteral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok<'a> {
    Word(&'a str),
    Op(&'a str),
    /// A literal, quoted identifier, bind or macro: an operand the rules do not read.
    Operand,
}

const OPS: [&str; 9] = ["!~~*", "!~*", "~~*", "~*", "<=", ">=", "<>", "!=", "::"];

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}

fn tokenize<'a>(code: &'a str, line: u32, out: &mut Vec<(u32, Tok<'a>)>) {
    let b = code.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if is_word_byte(c) {
            let mut e = i;
            while e < b.len() && is_word_byte(b[e]) {
                e += 1;
            }
            out.push((line, Tok::Word(&code[i..e])));
            i = e;
        } else if let Some(op) = OPS.iter().find(|op| code[i..].starts_with(**op)) {
            out.push((line, Tok::Op(op)));
            i += op.len();
        } else {
            out.push((line, Tok::Op(&code[i..i + 1])));
            i += 1;
        }
    }
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

fn is_ordering(t: Option<&Tok<'_>>) -> bool {
    matches!(t, Some(Tok::Op("<" | "<=" | ">" | ">=")))
}

/// Lints every section of `pack` with `dialect`'s rules.
pub fn lint(pack: &Pack, dialect: DialectId) -> Vec<LintError> {
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
                Piece::Comment(c) => {
                    if rules == Family::MySql && (c.starts_with("/*!") || c.starts_with("/*M!")) {
                        push(l.line, LintKind::ExecutableComment);
                    }
                }
                Piece::Bind(_) | Piece::List(_) => toks.push((l.line, Tok::Operand)),
                Piece::Macro(m) => {
                    if m.starts_with("ident:") && !sec.show_create {
                        push(l.line, LintKind::IdentMacroOutsideShowCreate);
                    }
                    toks.push((l.line, Tok::Operand));
                }
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
            Tok::Op(_) | Tok::Operand => {}
            Tok::Word(w) => {
                let call = op(k + 1) == Some("(");
                if eq(w, "txid_current") || eq(w, "pg_current_xact_id") {
                    push(line, LintKind::XidAssigningFunction(w.to_string()));
                }
                if (call
                    && ["lower", "upper", "lcase", "ucase"]
                        .iter()
                        .any(|f| eq(w, f)))
                    || eq(w, "ilike")
                    || eq(w, "collate")
                {
                    push(line, LintKind::CaseFoldingOnCatalogName(w.to_string()));
                }
                match rules {
                    Family::Pg => {
                        let q = qualifier(k);
                        if eq(w, "pg_catalog")
                            || eq(w, "information_schema")
                            || starts_with_ci(w, "pg_")
                            || q.is_some_and(|q| eq(q, "information_schema"))
                        {
                            if has_upper(w) {
                                push(line, LintKind::NonCanonicalCatalogName(w.to_string()));
                            }
                            if starts_with_ci(w, "pg_")
                                && !eq(w, "pg_catalog")
                                && !q.is_some_and(|q| eq(q, "pg_catalog"))
                            {
                                push(line, LintKind::UnqualifiedPgCatalog(w.to_string()));
                            }
                        }
                        if eq(w, "age") && call {
                            let before = if q.is_some() {
                                k.checked_sub(3)
                            } else {
                                k.checked_sub(1)
                            };
                            let mut depth = 0usize;
                            let mut close = None;
                            for (m, (_, t)) in toks.iter().enumerate().skip(k + 1) {
                                match t {
                                    Tok::Op("(") => depth += 1,
                                    Tok::Op(")") => {
                                        depth -= 1;
                                        if depth == 0 {
                                            close = Some(m);
                                            break;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            let after = close.and_then(|m| toks.get(m + 1)).map(|t| &t.1);
                            let prev = before.and_then(|b| toks.get(b)).map(|t| &t.1);
                            if is_ordering(after) || is_ordering(prev) {
                                push(line, LintKind::OrderingOnAge);
                            }
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
