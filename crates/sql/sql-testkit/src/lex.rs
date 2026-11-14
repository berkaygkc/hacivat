//! The lexer corpus (`corpus/lex/{pg,mysql,sqlite}.toml`; Task 4) and the helpers lexer tests
//! share. One `[[case]]` per input, with the token kinds it lexes to and, optionally, the token
//! texts, flags and error codes. Expectations come from Hacivat's specs (`alg/10` SQL-LEX-01,
//! -02, -05; ADR 0004 step 1a), never from another product (ALG-TEST-01).

use std::path::{Path, PathBuf};

use serde::Deserialize;
use sql_dialect::{DialectCtx, DialectId, LexDialect, LexModes, ServerVersion, lex_profile};
use sql_lexer::{LexState, Lexed, TokenKind, lex};

/// One lexer case.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LexCase {
    pub id: String,
    /// Where the expectation comes from.
    pub source: String,
    /// `postgres`, `mysql`, `mariadb`, `sqlite` or `generic`.
    pub dialect: String,
    /// The server version, `major.minor.patch`; absent: unknown (the newest profile).
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub modes: ModeOverrides,
    /// `copy`: the input is psql `COPY … FROM STDIN` data (lexed from `InCopyData`).
    #[serde(default)]
    pub start: Option<String>,
    pub input: String,
    /// The token kinds, by their `Debug` names.
    pub kinds: Vec<String>,
    /// The token texts.
    #[serde(default)]
    pub texts: Option<Vec<String>>,
    /// The token flags by name, one entry per token (`UNTERMINATED|JUNK`; `""` for none).
    #[serde(default)]
    pub flags: Option<Vec<String>>,
    /// The error codes, by their `Debug` names, in order.
    #[serde(default)]
    pub errors: Option<Vec<String>>,
}

/// Session modes a case sets on top of its dialect's defaults (`LexModes::default_for`).
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModeOverrides {
    pub standard_conforming_strings: Option<bool>,
    pub ansi_quotes: Option<bool>,
    pub no_backslash_escapes: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpusFile {
    case: Vec<LexCase>,
}

impl LexCase {
    /// The case's dialect context.
    pub fn ctx(&self) -> DialectCtx {
        let mut ctx = dialect_ctx(&self.dialect, self.version.as_deref());
        let m = self.modes;
        let modes = &mut ctx.modes;
        modes.standard_conforming_strings = m
            .standard_conforming_strings
            .unwrap_or(modes.standard_conforming_strings);
        modes.ansi_quotes = m.ansi_quotes.unwrap_or(modes.ansi_quotes);
        modes.no_backslash_escapes = m.no_backslash_escapes.unwrap_or(modes.no_backslash_escapes);
        ctx
    }

    /// The state the case is lexed from.
    pub fn start_state(&self) -> LexState {
        match self.start.as_deref() {
            None => LexState::Initial,
            Some("copy") => LexState::InCopyData,
            Some(other) => panic!("{}: unknown start state {other:?}", self.id),
        }
    }
}

/// The path of corpus file `rel` (`lex/pg.toml`).
pub fn corpus_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("corpus")
        .join(rel)
}

/// The cases of corpus file `rel`.
///
/// # Panics
/// When the file is missing or malformed, naming it.
pub fn load(rel: &str) -> Vec<LexCase> {
    let path = corpus_path(rel);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let file: CorpusFile =
        toml::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    file.case
}

/// The context of `dialect` at `version` (`major.minor.patch`), with the dialect's default
/// modes.
///
/// # Panics
/// On an unknown dialect name or a malformed version.
pub fn dialect_ctx(dialect: &str, version: Option<&str>) -> DialectCtx {
    let id = match dialect {
        "postgres" => DialectId::Postgres,
        "mysql" => DialectId::MySql,
        "mariadb" => DialectId::MariaDb,
        "sqlite" => DialectId::Sqlite,
        "generic" => DialectId::Generic,
        other => panic!("unknown dialect {other:?}"),
    };
    let version = version.map(|v| {
        let parts: Vec<u16> = v
            .split('.')
            .map(|p| p.parse().unwrap_or_else(|e| panic!("version {v:?}: {e}")))
            .collect();
        match parts.as_slice() {
            [major, minor, patch] => ServerVersion {
                major: *major,
                minor: *minor,
                patch: *patch,
            },
            _ => panic!("version {v:?} is not major.minor.patch"),
        }
    });
    DialectCtx {
        id,
        version,
        modes: LexModes::default_for(id),
    }
}

/// The lexer profile of `ctx`.
///
/// # Panics
/// For a dialect without a profile in M3a.
pub fn profile(ctx: &DialectCtx) -> &'static LexDialect {
    lex_profile(ctx).unwrap_or_else(|e| panic!("no lexer profile: {e:?}"))
}

/// `text` lexed in `ctx` from the initial state.
pub fn lex_in(ctx: &DialectCtx, text: &str) -> Lexed {
    lex(text, profile(ctx), ctx.modes, &LexState::Initial)
}

/// The kinds of every token.
pub fn kinds_of(l: &Lexed) -> Vec<TokenKind> {
    (0..l.len()).map(|i| l.kind(i)).collect()
}

/// The text of every token.
pub fn texts_of<'a>(text: &'a str, l: &Lexed) -> Vec<&'a str> {
    (0..l.len())
        .map(|i| {
            let r = l.range(i);
            &text[usize::from(r.start())..usize::from(r.end())]
        })
        .collect()
}

/// The tokens' texts joined: the input again, when lexing is lossless.
pub fn concat_tokens(text: &str, l: &Lexed) -> String {
    texts_of(text, l).concat()
}
