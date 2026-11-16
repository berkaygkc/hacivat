//! The lexer's cases (Task 4): the `alg/10` SQL-LEX-02 corpus for PostgreSQL, MySQL/MariaDB
//! and SQLite (`sql-testkit/corpus/lex/*.toml`), the plan's named cases, the generic profile,
//! keywords through the lexer, end states, token classes, `lex_chunks` and `relex`.

use std::borrow::Cow;
use std::ops::Range;

use smallvec::SmallVec;
use sql_dialect::{
    DialectCtx, DialectId, KeywordId, KwCategory, KwEntry, LexDialect, LexModes, keyword_lookup, kw,
};
use sql_lexer::TokenKind::*;
use sql_lexer::{
    BodyReading, LexState, Lexed, StringRule, TokenClass, TokenFlags, TokenKind, lex, lex_chunks,
    relex, token_class,
};
use sql_testkit::lex::{
    LexCase, concat_tokens, dialect_ctx, kinds_of, lex_in, load, profile, texts_of,
};
use sql_text::{TextChunks, TextEdit, TextRange, TextSize};

fn corpus(rel: &str) -> impl Iterator<Item = LexCase> {
    load(rel).into_iter()
}

fn parse_flags(id: &str, names: &str) -> TokenFlags {
    names
        .split('|')
        .filter(|n| !n.is_empty())
        .map(|n| TokenFlags::from_name(n).unwrap_or_else(|| panic!("{id}: unknown flag {n}")))
        .fold(TokenFlags::empty(), |all, f| all | f)
}

/// What differs between case `c` and its lexing, if anything.
fn mismatch(c: &LexCase) -> Option<std::string::String> {
    let ctx = c.ctx();
    let l = lex(&c.input, profile(&ctx), ctx.modes, &c.start_state());
    // `String` is `TokenKind::String` in this file.
    let kinds: Vec<std::string::String> = kinds_of(&l).iter().map(|k| format!("{k:?}")).collect();
    let texts = texts_of(&c.input, &l);
    let flags: Vec<TokenFlags> = (0..l.len()).map(|i| l.flags(i)).collect();
    let errors: Vec<std::string::String> =
        l.errors().iter().map(|e| format!("{:?}", e.code)).collect();
    let found = format!("kinds {kinds:?} texts {texts:?} flags {flags:?} errors {errors:?}");
    let wrong = kinds != c.kinds
        || c.texts.as_ref().is_some_and(|t| texts != *t)
        || c.flags
            .as_ref()
            .is_some_and(|f| flags != f.iter().map(|f| parse_flags(&c.id, f)).collect::<Vec<_>>())
        || c.errors.as_ref().is_some_and(|e| errors != *e)
        || concat_tokens(&c.input, &l) != c.input;
    wrong.then(|| format!("{} ({:?}): {found}", c.id, c.input))
}

fn assert_tokens(c: &LexCase) {
    if let Some(m) = mismatch(c) {
        panic!("{m}");
    }
}

const CORPORA: [&str; 3] = ["lex/pg.toml", "lex/mysql.toml", "lex/sqlite.toml"];

#[test]
fn lex02_cases_1_to_18() {
    let cases: Vec<LexCase> = corpus("lex/pg.toml")
        .chain(corpus("lex/mysql.toml"))
        .chain(corpus("lex/sqlite.toml"))
        .collect();
    assert!(
        cases.len() >= 150,
        "the corpus shrank to {} cases",
        cases.len()
    );
    let wrong: Vec<std::string::String> = cases.iter().filter_map(mismatch).collect();
    assert!(
        wrong.is_empty(),
        "{} cases differ:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
    cases.iter().for_each(assert_tokens);
}

/// `alg/10` SQL-LEX-02 tests 1–17 each have a case, or a reason they have none here; test 18
/// is the property test (`props.rs`).
#[test]
fn lex02_ids_are_covered() {
    const SKIPPED: [(u32, &str); 3] = [
        (9, "T-SQL: no SQL Server profile in M3a"),
        (11, "ClickHouse: no profile in M3a"),
        (12, "Oracle: no profile in M3a"),
    ];
    let ids: Vec<std::string::String> = CORPORA
        .iter()
        .flat_map(|rel| load(rel))
        .map(|c| c.id)
        .collect();
    for n in 1..=17u32 {
        let present = ids
            .iter()
            .any(|id| id.starts_with(&format!("LEX02-{n:02}")));
        let skipped = SKIPPED.iter().any(|(s, _)| *s == n);
        assert!(
            present != skipped,
            "LEX-02 test {n}: present {present}, skipped {skipped}"
        );
    }
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), ids.len(), "case ids are unique");
}

fn pg() -> DialectCtx {
    dialect_ctx("postgres", None)
}

fn mysql() -> DialectCtx {
    dialect_ctx("mysql", Some("8.4.0"))
}

fn kinds(ctx: DialectCtx, text: &str) -> Vec<TokenKind> {
    kinds_of(&lex_in(&ctx, text))
}

fn lex_pg(text: &str) -> Lexed {
    lex_in(&pg(), text)
}

#[test]
fn mysql_double_dash_needs_whitespace() {
    assert_eq!(kinds(mysql(), "1--1"), [Number, Op, Op, Number]);
    assert_eq!(kinds(mysql(), "1-- x"), [Number, LineComment]);
    // PostgreSQL needs nothing after the dashes.
    assert_eq!(kinds(pg(), "1--1"), [Number, LineComment]);
}

#[test]
fn dollar_tags_are_case_sensitive() {
    assert_eq!(kinds(pg(), "$a$x$A$y$a$"), [DollarString]);
    let l = lex_pg("$a$x$A$y$A");
    assert_eq!(kinds_of(&l), [DollarString]);
    assert!(l.flags(0).contains(TokenFlags::UNTERMINATED));
}

#[test]
fn positional_param_is_not_a_tag() {
    assert_eq!(kinds(pg(), "$1$"), [PositionalParam, Unknown]);
}

#[test]
fn unterminated_string_is_one_flagged_token() {
    let l = lex_pg("SELECT 'abc");
    assert!(l.flags(l.len() - 1).contains(TokenFlags::UNTERMINATED));
    assert_eq!(l.kind(l.len() - 1), String);
    assert_eq!(l.range(l.len() - 1), range(7, 11));
    assert_eq!(l.errors().len(), 1);
    assert_eq!(l.errors()[0].range, range(7, 11));
}

#[test]
fn max_paren_depth_counts_nesting() {
    let l = lex_pg("SELECT ((1), (2))");
    assert_eq!(l.max_paren_depth(0..l.len()), 2);
    // Parentheses inside strings and comments are not nesting.
    let l = lex_pg("SELECT '((((' /* ((( */ (1)");
    assert_eq!(l.max_paren_depth(0..l.len()), 1);
}

#[test]
fn generic_profile_is_ansi() {
    let ctx = dialect_ctx("generic", None);
    let text = "\"a\" 'b''c\\' x::int ? :n -- c\n/* /* */ $ #";
    assert_eq!(
        kinds(ctx, text),
        [
            QuotedIdent,
            Whitespace,
            String,
            Whitespace,
            Word,
            Op,
            Word,
            Whitespace,
            QuestionParam,
            Whitespace,
            NamedParam,
            Whitespace,
            LineComment,
            Newline,
            BlockComment,
            Whitespace,
            Unknown,
            Whitespace,
            Unknown,
        ]
    );
    assert_eq!(profile(&ctx).name(), "generic");
}

#[test]
fn pg_wire_servers_lex_as_postgres() {
    use sql_dialect::PgWireFlavor;
    let ctx = DialectCtx {
        id: DialectId::PgWire(PgWireFlavor::Cockroach),
        version: None,
        modes: LexModes::default_for(DialectId::PgWire(PgWireFlavor::Cockroach)),
    };
    assert_eq!(
        kinds(ctx, "SELECT $$a;b$$;"),
        [Word, Whitespace, DollarString, Semicolon]
    );
}

#[test]
fn words_carry_their_keyword_lookup() {
    for (dialect, text) in [
        ("postgres", "SELECT name, type FROM t WHERE x LIMIT 1"),
        ("mysql", "select `select`, 1e5x FROM dual"),
        ("sqlite", "SELECT $a FROM t"),
        ("generic", "select 1"),
    ] {
        let ctx = dialect_ctx(dialect, None);
        let l = lex_in(&ctx, text);
        let words = texts_of(text, &l);
        for (i, word) in words.iter().enumerate() {
            let want = if l.kind(i) == Word {
                keyword_lookup(profile(&ctx).keywords, word)
            } else {
                None
            };
            assert_eq!(l.keyword(i), want, "{dialect}: {word:?}");
        }
    }
    // With Task 3's tables the lookups are real: the lexer stores the keyword's id, and an
    // identifier (or a quoted `select`, or a digit-leading word) stores none.
    let l = lex_pg("SELECT name FROM t LIMIT 1");
    let ids: Vec<Option<KeywordId>> = (0..l.len()).map(|i| l.keyword(i).map(|k| k.id)).collect();
    assert_eq!(ids[0], Some(kw::SELECT));
    assert_eq!(ids[4], Some(kw::FROM));
    assert_eq!(ids[8], Some(kw::LIMIT));
    assert_eq!(ids[6], None, "t is an identifier");
    let l = lex_in(&mysql(), "select `select`, 1e5x");
    assert_eq!(l.keyword(0).map(|k| k.id), Some(kw::SELECT));
    assert_eq!(l.keyword(2), None, "a quoted identifier");
    assert_eq!(l.keyword(5), None, "a digit-leading word");
}

#[test]
fn turkish_letters_never_become_keywords() {
    let text = "SELECT lımıt, lİmİt, SЕLECT FROM t";
    let l = lex_pg(text);
    let words: Vec<(&str, Option<KwEntry>)> = texts_of(text, &l)
        .into_iter()
        .zip((0..l.len()).map(|i| l.keyword(i)))
        .filter(|(w, _)| w.chars().any(char::is_alphabetic))
        .collect();
    // Each non-ASCII word is one word, and none of them is a keyword.
    assert_eq!(words.len(), 6, "{words:?}");
    for (word, kw) in &words[1..4] {
        assert_eq!(*kw, None, "{word}");
    }
    assert_eq!(words[0].1, keyword_lookup(DialectId::Postgres, "SELECT"));
    // The ASCII spellings are keywords (Task 3's tables); the Turkish and Cyrillic ones are not.
    assert_eq!(words[0].1.map(|k| k.id), Some(kw::SELECT));
    assert_eq!(words[4].1.map(|k| k.id), Some(kw::FROM));
    let l = lex_pg("SELECT 1 LIMIT 2 lImIt 3");
    assert_eq!(l.keyword(4).map(|k| k.id), Some(kw::LIMIT));
    assert_eq!(l.keyword(8).map(|k| k.id), Some(kw::LIMIT));
}

#[test]
fn end_states_name_the_open_construct() {
    let cases: [(DialectCtx, &str, LexState); 14] = [
        (pg(), "SELECT 1", LexState::Initial),
        (pg(), "'abc", in_string(StringRule::Standard, b'\'', String)),
        (pg(), "E'a\\'", in_string(StringRule::Escape, b'\'', String)),
        (pg(), "B'01", in_string(StringRule::Raw, b'\'', BitString)),
        (pg(), "/* /* x", LexState::InBlockComment { depth: 2 }),
        (
            pg(),
            "$fn$ x",
            LexState::InDollar {
                tag: SmallVec::from_slice(b"fn"),
            },
        ),
        (
            pg(),
            "U&\"x",
            LexState::InQuotedIdent {
                close: b'"',
                kind: UnicodeQuotedIdent,
            },
        ),
        (
            mysql(),
            "\"a",
            in_string(StringRule::Backslash, b'"', String),
        ),
        (mysql(), "/* /* x", LexState::InBlockComment { depth: 1 }),
        (
            mysql(),
            "/*!99999 /* x",
            LexState::InExecutableComment {
                depth: 2,
                body: BodyReading::Nested,
            },
        ),
        (
            mysql(),
            "/*! 'x",
            LexState::InExecutableComment {
                depth: 1,
                body: BodyReading::Quoted { quote: b'\'' },
            },
        ),
        (
            mysql(),
            "/*! # x",
            LexState::InExecutableComment {
                depth: 1,
                body: BodyReading::LineComment,
            },
        ),
        (
            mysql(),
            "/*+ x",
            LexState::InHintComment {
                body: BodyReading::Code,
            },
        ),
        (
            dialect_ctx("sqlite", None),
            "[x",
            LexState::InQuotedIdent {
                close: b']',
                kind: QuotedIdent,
            },
        ),
    ];
    for (ctx, text, want) in cases {
        assert_eq!(*lex_in(&ctx, text).end_state(), want, "{text:?}");
    }
}

fn in_string(rule: StringRule, quote: u8, kind: TokenKind) -> LexState {
    LexState::InString { rule, quote, kind }
}

#[test]
fn lexing_from_a_state_continues_its_construct() {
    let p = profile(&pg());
    let m = pg().modes;
    let l = lex("x */ y", p, m, &LexState::InBlockComment { depth: 1 });
    assert_eq!(kinds_of(&l), [BlockComment, Whitespace, Word]);
    let l = lex("a */ b */ c", p, m, &LexState::InBlockComment { depth: 2 });
    assert_eq!(kinds_of(&l), [BlockComment, Whitespace, Word]);
    let l = lex(
        "x$fn$;",
        p,
        m,
        &LexState::InDollar {
            tag: SmallVec::from_slice(b"fn"),
        },
    );
    assert_eq!(kinds_of(&l), [DollarString, Semicolon]);
    let l = lex(
        "b' c",
        p,
        m,
        &in_string(StringRule::Standard, b'\'', String),
    );
    assert_eq!(kinds_of(&l), [String, Whitespace, Word]);
    // Still inside: the whole text continues the construct, and the state carries on.
    let l = lex(
        "no end",
        p,
        m,
        &in_string(StringRule::Standard, b'\'', String),
    );
    assert_eq!(kinds_of(&l), [String]);
    assert_eq!(
        *l.end_state(),
        in_string(StringRule::Standard, b'\'', String)
    );
    // An empty text stays in the state it started in.
    let l = lex("", p, m, &LexState::InCopyData);
    assert!(l.is_empty());
    assert_eq!(*l.end_state(), LexState::InCopyData);
}

#[test]
fn token_classes_follow_kinds_flags_and_keywords() {
    let kw = Some(KwEntry {
        id: KeywordId(1),
        category: KwCategory::Reserved,
    });
    let none = TokenFlags::empty();
    let cases = [
        (Word, none, kw, TokenClass::Keyword),
        (Word, none, None, TokenClass::Identifier),
        (QuotedIdent, none, None, TokenClass::QuotedIdentifier),
        (UnicodeQuotedIdent, none, None, TokenClass::QuotedIdentifier),
        (String, none, None, TokenClass::String),
        (DollarString, none, None, TokenClass::String),
        (HexString, none, None, TokenClass::String),
        (Number, none, None, TokenClass::Number),
        (LineComment, none, None, TokenClass::Comment),
        (
            BlockComment,
            TokenFlags::UNTERMINATED,
            None,
            TokenClass::Comment,
        ),
        (HintComment, none, None, TokenClass::OptimizerHint),
        (ExecutableComment, none, None, TokenClass::ExecutableComment),
        (Op, none, None, TokenClass::Operator),
        (Semicolon, none, None, TokenClass::Punctuation),
        (LParen, none, None, TokenClass::Punctuation),
        (PositionalParam, none, None, TokenClass::Parameter),
        (UserVar, none, None, TokenClass::Parameter),
        (ClientVar, none, None, TokenClass::Parameter),
        (ClientCommand, none, None, TokenClass::ClientCommand),
        (Whitespace, TokenFlags::BOM, None, TokenClass::Whitespace),
        (Newline, none, None, TokenClass::Whitespace),
        (Unknown, TokenFlags::ERROR, None, TokenClass::Error),
        (Number, TokenFlags::JUNK, None, TokenClass::Error),
        (Op, TokenFlags::ERROR, None, TokenClass::Error),
        (String, TokenFlags::UNTERMINATED, None, TokenClass::String),
    ];
    for (kind, flags, kw, want) in cases {
        assert_eq!(token_class(kind, flags, kw), want, "{kind:?} {flags:?}");
    }
}

/// A text in several chunks, split at char boundaries.
struct Chunked(Vec<std::string::String>);

impl TextChunks for Chunked {
    fn len(&self) -> usize {
        self.0.iter().map(std::string::String::len).sum()
    }

    fn slice(&self, r: Range<usize>) -> Cow<'_, str> {
        Cow::Owned(self.0.concat()[r].to_owned())
    }

    fn chunks(&self) -> Box<dyn Iterator<Item = &str> + '_> {
        Box::new(self.0.iter().map(std::string::String::as_str))
    }
}

#[test]
fn lex_chunks_equals_lex() {
    let ctx = pg();
    let p = profile(&ctx);
    let text = "SELECT 'çğ;' /* a ; */ $$x$$; -- İ\nSELECT 2";
    let whole = lex(text, p, ctx.modes, &LexState::Initial);
    for at in [1, 9, 10, 20, 30] {
        let split = (0..=at).rev().find(|i| text.is_char_boundary(*i)).unwrap();
        let chunked = Chunked(vec![text[..split].to_owned(), text[split..].to_owned()]);
        assert_eq!(
            lex_chunks(&chunked, p, ctx.modes, &LexState::Initial),
            whole
        );
    }
    let one: &dyn TextChunks = &text;
    assert_eq!(lex_chunks(one, p, ctx.modes, &LexState::Initial), whole);
    let empty = Chunked(Vec::new());
    assert!(lex_chunks(&empty, p, ctx.modes, &LexState::Initial).is_empty());
}

#[test]
fn relex_is_a_full_relex() {
    let ctx = pg();
    let p = profile(&ctx);
    let mut l = lex("SELECT 1", p, ctx.modes, &LexState::Initial);
    let new_text = "SELECT '1";
    let edit = TextEdit {
        delete: range(7, 7),
        insert: "'".into(),
    };
    let delta = relex(&mut l, &new_text, &edit, p, ctx.modes);
    assert_eq!(l, lex(new_text, p, ctx.modes, &LexState::Initial));
    assert!(delta.full);
    assert_eq!(delta.changed, 0..l.len());
}

#[test]
fn token_at_and_tokens_in_find_their_tokens() {
    let text = "SELECT 1; SELECT 'x'";
    let l = lex_pg(text);
    assert_eq!(l.token_at(TextSize::from(0)), Some(0));
    assert_eq!(l.token_at(TextSize::from(5)), Some(0));
    assert_eq!(l.token_at(TextSize::from(6)), Some(1));
    assert_eq!(l.token_at(TextSize::from(19)), Some(l.len() - 1));
    assert_eq!(l.token_at(TextSize::from(20)), Some(l.len() - 1));
    assert_eq!(l.token_at(TextSize::from(21)), None);
    assert_eq!(l.tokens_in(range(7, 9)), 2..4);
    assert_eq!(l.tokens_in(range(3, 3)), 0..1);
    assert_eq!(l.tokens_in(range(6, 6)), 1..1);
    assert!(lex_pg("").token_at(TextSize::from(0)).is_none());
}

#[test]
fn profiles_are_static_data() {
    let a: &LexDialect = profile(&pg());
    let b: &LexDialect = profile(&dialect_ctx("postgres", Some("17.0.0")));
    assert!(std::ptr::eq(a, b));
    assert_eq!(a.name(), "pg-16");
}

fn range(start: u32, end: u32) -> TextRange {
    TextRange::new(TextSize::from(start), TextSize::from(end))
}
