//! Statement fingerprints (SQL-PARAM-03; Task 6): `HvNorm1` replaces literals and `IN (…)`
//! lists with placeholders, collapses whitespace and comments, ASCII-uppercases keywords, keeps
//! identifiers and quoted text byte-exact and hashes with XXH3-64 (P-M3b-18).

use std::sync::Arc;

use sql_dialect::{DialectCtx, DialectId, LexDialect, LexModes};
use sql_lexer::{LexState, Lexed, TokenKind, exec_comment, lex};
use twox_hash::XxHash3_64;

use crate::splitter::lexer_for;

/// Which normaliser produced a [`Fingerprint`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FingerprintAlgo {
    /// Hacivat's lexer-based normaliser (every dialect).
    HvNorm1,
    /// libpg_query's fingerprint (PostgreSQL).
    PgQuery,
}

/// A statement's fingerprint: equal for statements that differ only in literals, whitespace,
/// comments and keyword case.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Fingerprint {
    pub algo: FingerprintAlgo,
    pub hash: u64,
    pub normalized: Arc<str>,
    /// The normaliser's version; `0` is Task 1a's stub.
    pub version: u16,
}

// ## m3b-t06-params

/// `HvNorm1`'s version (recorded with every fingerprint; a change to the rules bumps it).
pub const HV_NORM1_VERSION: u16 = 1;

/// The placeholder a literal or parameter becomes.
const HOLE: &str = "?";
/// A parenthesised list of placeholders only.
const HOLE_LIST: &str = "?…";
/// An optimizer hint.
const HINT: &str = "/*+…*/";

/// The `HvNorm1` fingerprint of `text` in `ctx` (a dialect without a lexer profile is read
/// with the generic one).
///
/// The text is lexed from the initial state. Whitespace and comments are dropped; strings,
/// numbers, bit and hex strings, dollar bodies, `COPY` data and parameters become `?` (a MySQL
/// charset introducer such as `_utf8mb4` joins its string, also across whitespace for a known
/// character set); a psql meta-command keeps `\name` and its arguments become one `?` (no
/// secret survives: `\connect db u password=…` → `\connect ?`); a parenthesised list of `?` only
/// becomes `(?…)`, so `IN (1,2,3)` and `IN (9)` agree; an optimizer hint becomes `/*+…*/`; an
/// executable comment keeps its opener and its body is normalised as code; keywords (the
/// lexer's keyword table) are ASCII-uppercased; identifiers, quoted identifiers and every other
/// token stay byte-exact (no case folding, no Unicode normalisation: servers compare bytes).
/// Tokens are joined by one space, with none after `(`, `[` or `.` and none before `)`, `]`,
/// `,`, `.` or `;`. The hash is XXH3-64 of that text.
pub fn hv_norm1(text: &str, ctx: &DialectCtx) -> Fingerprint {
    let profile = lexer_for(ctx);
    let mysql = matches!(ctx.id, DialectId::MySql | DialectId::MariaDb);
    let lexed = lex(text, profile, ctx.modes, &LexState::Initial);
    let mut pieces: Vec<Piece<'_>> = Vec::new();
    normalise(
        text,
        &lexed,
        profile,
        ctx.modes,
        mysql,
        Depth::Top,
        &mut pieces,
    );
    let normalized = join(&collapse_lists(pieces));
    Fingerprint {
        algo: FingerprintAlgo::HvNorm1,
        hash: XxHash3_64::oneshot(normalized.as_bytes()),
        normalized: Arc::from(normalized),
        version: HV_NORM1_VERSION,
    }
}

/// One output token: its text and how it joins its neighbours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Piece<'t> {
    Open(&'t str),
    Close(&'t str),
    Comma,
    Dot,
    Semicolon,
    Hole,
    HoleList,
    Text(&'t str),
    Upper(&'t str),
}

/// Whether normalising an executable comment's body may read nested executable comments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Depth {
    Top,
    Body,
}

fn normalise<'t>(
    text: &'t str,
    lexed: &Lexed,
    profile: &LexDialect,
    modes: LexModes,
    mysql: bool,
    depth: Depth,
    out: &mut Vec<Piece<'t>>,
) {
    let tok = |i: usize| {
        let r = lexed.range(i);
        &text[usize::from(r.start())..usize::from(r.end())]
    };
    let mut next = 0;
    while next < lexed.len() {
        let i = next;
        next += 1;
        let kind = lexed.kind(i);
        // A psql meta-command (`\connect db user password=…`, `\set x 'v'`): the lexer reads
        // `\` as an unknown character and the rest as code. Keep `\name`, mask every argument
        // up to the end of the line with one `?`, so no secret survives normalisation.
        if (kind == TokenKind::Unknown && tok(i) == "\\") || kind == TokenKind::ClientCommand {
            let start = usize::from(lexed.range(i).start());
            let (name_end, args) = if kind == TokenKind::ClientCommand {
                let t = tok(i);
                let name = t.find(|c: char| c.is_ascii_whitespace()).unwrap_or(t.len());
                (start + name, !t[name..].trim().is_empty())
            } else {
                let mut name_end = usize::from(lexed.range(i).end());
                if next < lexed.len()
                    && lexed.kind(next) == TokenKind::Word
                    && usize::from(lexed.range(next).start()) == name_end
                {
                    name_end = usize::from(lexed.range(next).end());
                    next += 1;
                }
                let mut args = false;
                while next < lexed.len() && lexed.kind(next) != TokenKind::Newline {
                    args |= !lexed.is_trivia(next);
                    next += 1;
                }
                (name_end, args)
            };
            out.push(Piece::Text(&text[start..name_end]));
            if args {
                out.push(Piece::Hole);
            }
            continue;
        }
        let piece = match kind {
            TokenKind::Whitespace
            | TokenKind::Newline
            | TokenKind::LineComment
            | TokenKind::BlockComment => continue,
            TokenKind::HintComment => Piece::Text(HINT),
            TokenKind::ExecutableComment => {
                match (depth, exec_comment(text, lexed, i)) {
                    (Depth::Top, Some(ec)) => {
                        let (os, oe) =
                            (usize::from(ec.opener.start()), usize::from(ec.opener.end()));
                        let (bs, be) = (usize::from(ec.body.start()), usize::from(ec.body.end()));
                        out.push(Piece::Text(&text[os..oe]));
                        let body = &text[bs..be];
                        let sub = lex(body, profile, modes, &LexState::Initial);
                        normalise(body, &sub, profile, modes, mysql, Depth::Body, out);
                        let end = usize::from(lexed.range(i).end());
                        if end > be {
                            out.push(Piece::Text(&text[be..end]));
                        }
                    }
                    // A nested one (or one that does not parse) keeps nothing but its shape.
                    _ => out.push(Piece::Text(HINT)),
                }
                continue;
            }
            TokenKind::String
            | TokenKind::DollarString
            | TokenKind::Number
            | TokenKind::BitString
            | TokenKind::HexString
            | TokenKind::InlineData
            | TokenKind::PositionalParam
            | TokenKind::QuestionParam
            | TokenKind::NamedParam
            | TokenKind::ClientVar => Piece::Hole,
            TokenKind::Word => {
                let w = tok(i);
                if mysql && introducer(lexed, i, w) {
                    continue;
                }
                if lexed.keyword(i).is_some() {
                    Piece::Upper(w)
                } else {
                    Piece::Text(w)
                }
            }
            TokenKind::LParen | TokenKind::LBracket => Piece::Open(tok(i)),
            TokenKind::RParen | TokenKind::RBracket => Piece::Close(tok(i)),
            TokenKind::Comma => Piece::Comma,
            TokenKind::Dot => Piece::Dot,
            TokenKind::Semicolon => Piece::Semicolon,
            _ => Piece::Text(tok(i)),
        };
        out.push(piece);
    }
}

/// MySQL/MariaDB character sets, for introducers written apart from their string
/// (`_binary X'ff'`).
const CHARSETS: &[&str] = &[
    "armscii8", "ascii", "big5", "binary", "cp1250", "cp1251", "cp1256", "cp1257", "cp850",
    "cp852", "cp866", "cp932", "dec8", "eucjpms", "euckr", "gb18030", "gb2312", "gbk", "geostd8",
    "greek", "hebrew", "hp8", "keybcs2", "koi8r", "koi8u", "latin1", "latin2", "latin5", "latin7",
    "macce", "macroman", "sjis", "swe7", "tis620", "ucs2", "ujis", "utf16", "utf16le", "utf32",
    "utf8", "utf8mb3", "utf8mb4",
];

fn is_string(k: TokenKind) -> bool {
    matches!(
        k,
        TokenKind::String | TokenKind::HexString | TokenKind::BitString
    )
}

/// Word `i` (`w`) is a MySQL charset introducer: `_name` directly before a string
/// (`_utf8mb4'x'`), or a known character set's `_name` before one with only whitespace between
/// (`_binary X'ff'`). A `_word` that is no character set, apart from its string, stays a word
/// (`_x 'a'` is a column and its alias).
fn introducer(lexed: &Lexed, i: usize, w: &str) -> bool {
    let Some(name) = w.strip_prefix('_') else {
        return false;
    };
    if i + 1 < lexed.len() && is_string(lexed.kind(i + 1)) {
        return true;
    }
    let after = (i + 1..lexed.len())
        .find(|j| !matches!(lexed.kind(*j), TokenKind::Whitespace | TokenKind::Newline));
    after.is_some_and(|j| is_string(lexed.kind(j)))
        && CHARSETS.iter().any(|c| name.eq_ignore_ascii_case(c))
}

/// `( ? , ? , … )` → `( ?… )`: a parenthesised list of placeholders only.
fn collapse_lists(pieces: Vec<Piece<'_>>) -> Vec<Piece<'_>> {
    let mut out = Vec::with_capacity(pieces.len());
    let mut i = 0;
    while i < pieces.len() {
        if pieces[i] == Piece::Open("(") {
            let mut j = i + 1;
            let mut holes = 0;
            loop {
                match pieces.get(j) {
                    Some(Piece::Hole) => {
                        holes += 1;
                        j += 1;
                        match pieces.get(j) {
                            Some(Piece::Comma) => j += 1,
                            Some(Piece::Close(")")) => break,
                            _ => {
                                holes = 0;
                                break;
                            }
                        }
                    }
                    _ => {
                        holes = 0;
                        break;
                    }
                }
            }
            if holes > 0 && pieces.get(j) == Some(&Piece::Close(")")) {
                out.extend([Piece::Open("("), Piece::HoleList, Piece::Close(")")]);
                i = j + 1;
                continue;
            }
        }
        out.push(pieces[i]);
        i += 1;
    }
    out
}

fn join(pieces: &[Piece<'_>]) -> String {
    let mut s = String::new();
    let mut prev: Option<Piece<'_>> = None;
    for p in pieces {
        let glue_left = matches!(
            p,
            Piece::Close(_) | Piece::Comma | Piece::Dot | Piece::Semicolon
        );
        let glue_right = matches!(prev, Some(Piece::Open(_) | Piece::Dot));
        if prev.is_some() && !glue_left && !glue_right {
            s.push(' ');
        }
        match p {
            Piece::Open(t) | Piece::Close(t) | Piece::Text(t) => s.push_str(t),
            Piece::Comma => s.push(','),
            Piece::Dot => s.push('.'),
            Piece::Semicolon => s.push(';'),
            Piece::Hole => s.push_str(HOLE),
            Piece::HoleList => s.push_str(HOLE_LIST),
            Piece::Upper(w) => s.push_str(&w.to_ascii_uppercase()),
        }
        prev = Some(*p);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pg() -> DialectCtx {
        DialectCtx::new(DialectId::Postgres, None)
    }

    #[test]
    fn version_one_and_algo() {
        let a = hv_norm1("SELECT 1", &pg());
        assert_eq!(a.version, HV_NORM1_VERSION);
        assert_eq!(a.algo, FingerprintAlgo::HvNorm1);
        assert_eq!(&*a.normalized, "SELECT ?");
        assert_eq!(a.hash, hv_norm1("select   2", &pg()).hash);
    }

    #[test]
    fn qualified_names_and_brackets_join_tightly() {
        assert_eq!(
            &*hv_norm1("select s . t.c, a[1] from s.t;", &pg()).normalized,
            "SELECT s.t.c, a [?] FROM s.t;"
        );
    }

    #[test]
    fn only_pure_hole_lists_collapse() {
        assert_eq!(
            &*hv_norm1("select (1), (a), (1, a), ()", &pg()).normalized,
            "SELECT (?…), (a), (?, a), ()"
        );
    }
}
