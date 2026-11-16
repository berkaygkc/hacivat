//! Properties (`alg/10` SQL-LEX-02 test 18 in PR form; the 10⁶-input fuzz target is M3b's):
//! the lexer is total and lossless for every profile and mode, its tokens are well formed and
//! its errors name every rejected token, resuming from an end state at a line start equals a
//! full lex, and the chunked and relex entry points equal `lex`.
//!
//! **The resume property is narrower than the plan's** (`a` and `b` arbitrary): it cuts only at
//! a line start (`a` ends with `\n`), because an end state is exact only there. A cut between
//! the quotes of `''`, after the backslash of `\'`, or between `*` and `/`, resumes into a
//! different reading; that is what a checkpoint (UI-EDIT-11, M3b) avoids. It also trims a
//! leading byte-order mark from `b`: the BOM rule depends on the absolute offset (a BOM at a
//! text's offset 0 is whitespace, `lex` docs), so a resumed slice never starts at one.

use std::borrow::Cow;
use std::ops::Range;

use proptest::prelude::*;
use sql_dialect::{DialectCtx, DialectId, LexDialect, LexModes, PgWireFlavor, keyword_lookup};
use sql_lexer::{LexErrorCode, LexState, Lexed, TokenFlags, TokenKind, lex, lex_chunks, relex};
use sql_testkit::lex::{concat_tokens, dialect_ctx, kinds_of, profile, texts_of};
use sql_text::{TextChunks, TextEdit, TextRange, TextSize};

/// Pieces that open, close and confuse every construct the lexer knows.
const FRAGMENTS: &[&str] = &[
    "SELECT", "select", "lımıt", "İ", "x", "_a1", "$", "$$", "$a$", "$A$", "$1", "1", "0x1F",
    "0b1", "1_000", "1e5", "1e", ".5", "1.", "..", "::", ":=", ":a", ":'v'", ":{?v}", "?", "?1",
    "@", "@@x", "@u", "#", "#c\n", "--", "-- c", "-", "+", "*", "/", "/*", "*/", "/*!", "/*!50000",
    "'*/'", "\"*/\"", "`*/`", "# */", "/*+ '", "/**/", "/*M!", "/*+", "'", "''", "\"", "`", "[",
    "]", "(", ")", "{", "}", ",", ";", ".", "E'", "e'", "U&'", "U&\"", "B'", "X'", "N'", "\\",
    "\\.", " ", "\t", "\n", "\r", "\r\n", "\u{b}", "\u{c}", "\0", "\u{feff}", "\u{a0}", "；", "😀",
    "ç", "|", "&", "~", "!", "<", ">", "=", "%", "^", "e", "E", "(x)", "$a::b(", "_utf8'",
];

fn any_sql() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => prop::collection::vec(prop::sample::select(FRAGMENTS), 0..48).prop_map(|v| v.concat()),
        1 => any::<String>(),
        1 => "[\\x00-\\x7f]{0,48}",
    ]
}

fn contexts() -> Vec<DialectCtx> {
    let mut all: Vec<DialectCtx> = [
        ("postgres", None),
        ("postgres", Some("15.0.0")),
        ("postgres", Some("14.0.0")),
        ("mysql", Some("8.0.36")),
        ("mysql", Some("8.4.0")),
        ("mariadb", Some("11.4.0")),
        ("sqlite", Some("3.51.0")),
        ("generic", None),
    ]
    .into_iter()
    .map(|(d, v)| dialect_ctx(d, v))
    .collect();
    let wire = DialectId::PgWire(PgWireFlavor::Redshift);
    all.push(DialectCtx {
        id: wire,
        version: None,
        modes: LexModes::default_for(wire),
    });
    all
}

fn profiles() -> impl Strategy<Value = &'static LexDialect> {
    let all: Vec<&'static LexDialect> = contexts().iter().map(profile).collect();
    prop::sample::select(all)
}

fn modes_strategy() -> impl Strategy<Value = LexModes> {
    prop::array::uniform6(any::<bool>()).prop_map(|b| LexModes {
        standard_conforming_strings: b[0],
        ansi_quotes: b[1],
        no_backslash_escapes: b[2],
        pipes_as_concat: b[3],
        high_not_precedence: b[4],
        mariadb_oracle_mode: b[5],
    })
}

fn starts() -> impl Strategy<Value = LexState> {
    prop_oneof![4 => Just(LexState::Initial), 1 => Just(LexState::InCopyData)]
}

/// The kinds of the tokens of `full` that end after byte `at`.
fn tail_kinds(full: &Lexed, at: usize) -> Vec<TokenKind> {
    tail(full, at).into_iter().map(|(k, _)| k).collect()
}

/// The kinds and ends (relative to `at`) of the tokens of `l` that end after byte `at`.
fn tail(l: &Lexed, at: usize) -> Vec<(TokenKind, usize)> {
    (0..l.len())
        .map(|i| (l.kind(i), usize::from(l.range(i).end())))
        .filter(|(_, end)| *end > at)
        .map(|(k, end)| (k, end - at))
        .collect()
}

/// Every structural promise a lexing of `s` keeps.
fn assert_well_formed(s: &str, p: &LexDialect, l: &Lexed) -> Result<(), TestCaseError> {
    prop_assert_eq!(concat_tokens(s, l), s);
    prop_assert!(l.len() <= s.len().max(1));
    let mut at = 0usize;
    for i in 0..l.len() {
        let r = l.range(i);
        prop_assert_eq!(
            usize::from(r.start()),
            at,
            "token {} starts where the last ended",
            i
        );
        prop_assert!(r.end() > r.start(), "token {} is empty", i);
        at = usize::from(r.end());
        prop_assert!(s.is_char_boundary(at));
        if l.flags(i).contains(TokenFlags::UNTERMINATED) {
            prop_assert_eq!(i, l.len() - 1, "only the last token runs unterminated");
        }
        let text = &s[usize::from(r.start())..at];
        let want = if l.kind(i) == TokenKind::Word {
            keyword_lookup(p.keywords, text)
        } else {
            None
        };
        prop_assert_eq!(l.keyword(i), want);
    }
    prop_assert_eq!(at, s.len());
    for e in l.errors() {
        let i = l.token_at(e.range.start()).unwrap();
        prop_assert_eq!(l.range(i), e.range, "an error covers one token");
    }
    // Errors name every rejected token (review minor 4): `JUNK` ⟺ `TrailingJunk`, `ERROR` ⟺
    // another non-unterminated code, `UNTERMINATED` ⟸ an unterminated code (except where the
    // server accepts the open construct).
    for i in 0..l.len() {
        let codes: Vec<LexErrorCode> = l
            .errors()
            .iter()
            .filter(|e| e.range == l.range(i))
            .map(|e| e.code)
            .collect();
        let flags = l.flags(i);
        prop_assert_eq!(
            flags.contains(TokenFlags::JUNK),
            codes.contains(&LexErrorCode::TrailingJunk),
            "token {}: {:?} {:?}",
            i,
            flags,
            codes
        );
        let rejected = codes
            .iter()
            .any(|c| !c.is_unterminated() && *c != LexErrorCode::TrailingJunk);
        prop_assert_eq!(
            flags.contains(TokenFlags::ERROR),
            rejected,
            "token {}: {:?}",
            i,
            codes
        );
        let open_ok = l.kind(i) == TokenKind::InlineData
            || (l.kind(i) == TokenKind::BlockComment && p.comments.unterminated_ok);
        if flags.contains(TokenFlags::UNTERMINATED) && !open_ok {
            prop_assert!(
                codes.iter().any(|c| c.is_unterminated()),
                "token {}: {:?}",
                i,
                codes
            );
        }
        if !flags.contains(TokenFlags::UNTERMINATED) {
            prop_assert!(
                !codes.iter().any(|c| c.is_unterminated()),
                "token {}: {:?}",
                i,
                codes
            );
        }
    }
    Ok(())
}

/// The flags of the tokens of `l` that end after byte `at`; the first one (a construct cut by
/// `at`) keeps only what a resumed piece can know: whether it runs unterminated and whether
/// its comment end is ambiguous (a prefix's own checks, `U&` or SQLite blobs, sit before `at`).
fn tail_flags(l: &Lexed, at: usize) -> Vec<TokenFlags> {
    let first = (0..l.len()).find(|i| usize::from(l.range(*i).end()) > at);
    let Some(first) = first else {
        return Vec::new();
    };
    (first..l.len())
        .map(|i| {
            let flags = l.flags(i);
            if i > first {
                return flags;
            }
            let ambiguous = l
                .errors()
                .iter()
                .any(|e| e.range == l.range(i) && e.code == LexErrorCode::AmbiguousCommentEnd);
            let kept = flags & TokenFlags::UNTERMINATED;
            if ambiguous {
                kept | TokenFlags::ERROR
            } else {
                kept
            }
        })
        .collect()
}

proptest! {
    #[test]
    fn lossless_for_every_profile_and_mode(s in any_sql(), p in profiles(), m in modes_strategy()) {
        let l = lex(&s, p, m, &LexState::Initial);
        prop_assert_eq!(concat_tokens(&s, &l), s.clone());
        prop_assert!(l.len() <= s.len().max(1));
        assert_well_formed(&s, p, &l)?;
        // From the initial state, lexing ends inside a construct exactly when the last token
        // runs unterminated.
        let open = !l.is_empty() && l.flags(l.len() - 1).contains(TokenFlags::UNTERMINATED);
        prop_assert_eq!(*l.end_state() != LexState::Initial, open);
    }

    #[test]
    fn resuming_from_an_end_state_equals_a_full_lex(
        a in any_sql(),
        b in any_sql(),
        p in profiles(),
        m in modes_strategy(),
        start in starts(),
    ) {
        // The cut is a line start (a checkpoint), and `b` never starts with a byte-order mark,
        // whose reading depends on the absolute offset (module docs).
        let b = b.trim_start_matches('\u{feff}');
        let a = a + "\n";
        let full = lex(&(a.clone() + b), p, m, &start);
        let left = lex(&a, p, m, &start);
        let right = lex(b, p, m, left.end_state());
        prop_assert_eq!(tail_kinds(&full, a.len()), kinds_of(&right));
        prop_assert_eq!(tail(&full, a.len()), tail(&right, 0));
        prop_assert_eq!(full.end_state(), right.end_state());
        // The flags carry over too, an ambiguous executable-comment or hint end included.
        let right_flags: Vec<TokenFlags> = tail_flags(&right, 0);
        prop_assert_eq!(tail_flags(&full, a.len()), right_flags);
        assert_well_formed(b, p, &right)?;
    }

    #[test]
    fn chunked_and_relexed_text_lexes_the_same(
        s in any_sql(),
        p in profiles(),
        m in modes_strategy(),
        cuts in prop::collection::vec(any::<prop::sample::Index>(), 0..4),
    ) {
        let whole = lex(&s, p, m, &LexState::Initial);
        let mut at: Vec<usize> = cuts.iter().map(|c| c.index(s.len() + 1)).collect();
        at.push(0);
        at.push(s.len());
        at.sort_unstable();
        at.dedup();
        let pieces: Vec<String> = at
            .windows(2)
            .map(|w| (floor_boundary(&s, w[0]), floor_boundary(&s, w[1])))
            .map(|(x, y)| s[x..y].to_owned())
            .collect();
        prop_assert_eq!(pieces.concat(), s.clone());
        prop_assert_eq!(&lex_chunks(&Pieces(pieces), p, m, &LexState::Initial), &whole);
        let mut relexed = lex("", p, m, &LexState::Initial);
        let edit = TextEdit { delete: TextRange::empty(TextSize::from(0)), insert: s.clone() };
        let delta = relex(&mut relexed, &s, &edit, p, m);
        prop_assert_eq!(&relexed, &whole);
        prop_assert_eq!(delta.changed, 0..whole.len());
        prop_assert_eq!(texts_of(&s, &relexed).concat(), s);
    }
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// A text in pieces.
struct Pieces(Vec<String>);

impl TextChunks for Pieces {
    fn len(&self) -> usize {
        self.0.iter().map(String::len).sum()
    }

    fn slice(&self, r: Range<usize>) -> Cow<'_, str> {
        Cow::Owned(self.0.concat()[r].to_owned())
    }

    fn chunks(&self) -> Box<dyn Iterator<Item = &str> + '_> {
        Box::new(self.0.iter().map(String::as_str))
    }
}
