// ## m4-t01c-core (seam) — body: ## m4-t12-scope
//! Glob patterns of the object filter grammar (HV-INTRO-03, M4 Task 12) and the regex helpers
//! the scope and filter grammars share.
//!
//! A glob has `*` (any run of characters), `?` (one character), `[…]` (one character of a
//! set: `[abc]`, `[a-z]`, negated `[!abc]` or `[^abc]`, `]` first is literal) and `\`, which
//! makes the next character literal. Everything else is literal: `_`, `%` and `|` match only
//! themselves (DBeaver's pitfall, alg/02 HV-INTRO-03). A glob matches the **whole** name.
//!
//! **Case** follows the server's identifier rule (HV-INTRO-14), never the process locale:
//! [`GlobCase::Exact`] compares characters, [`GlobCase::AsciiInsensitive`] lets an ASCII letter
//! match its other case and nothing else (no Unicode or Turkish folding; INV-3).
//!
//! **Compiled once.** [`Glob::new`] picks a matcher once: a plain compare for a literal, a
//! prefix, a suffix or an infix, and an anchored regular expression otherwise. Nothing is
//! compiled per object.
//!
//! **Server form.** [`Glob::like_pattern`] is the glob as a `LIKE … ESCAPE '\'` pattern
//! (`*` → `%`, `?` and `[…]` → `_`, a literal `%`, `_` or `\` escaped with `\`). It selects a
//! superset of the glob's matches (a class becomes any one character), so the client matcher
//! always runs after it; [`crate::filter`] decides when it may be sent.

use regex::{Regex, RegexBuilder};

/// The most memory one compiled pattern may take (the `regex` crate's size limits). User text
/// that needs more is refused as an invalid pattern, not compiled slowly.
pub(crate) const REGEX_SIZE_LIMIT: usize = 1 << 20;

/// How a glob compares letters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlobCase {
    /// Character for character (PostgreSQL names, MySQL table names under
    /// `lower_case_table_names = 0`).
    Exact,
    /// ASCII letters match either case; every other character exactly (the server's rule;
    /// never Unicode or locale folding).
    AsciiInsensitive,
}

/// One compiled glob.
#[derive(Debug, Clone)]
pub struct Glob {
    pattern: String,
    case: GlobCase,
    tokens: Vec<Tok>,
    matcher: Matcher,
}

impl PartialEq for Glob {
    fn eq(&self, other: &Self) -> bool {
        self.pattern == other.pattern && self.case == other.case
    }
}

impl Eq for Glob {}

impl std::hash::Hash for Glob {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.pattern.hash(state);
        self.case.hash(state);
    }
}

/// Why a glob does not compile (`intro-filter-bad-glob`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlobError {
    /// A `\` with nothing after it.
    TrailingEscape,
    /// A `[` without its `]`.
    UnclosedClass,
    /// A range whose end comes before its start (`[z-a]`).
    BadRange,
    /// The pattern is too large to compile.
    TooLarge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Lit(char),
    /// `*`
    Many,
    /// `?`
    One,
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
}

#[derive(Debug, Clone)]
enum Matcher {
    /// `*`
    All,
    Literal(String),
    /// `abc*`
    Prefix(String),
    /// `*abc`
    Suffix(String),
    /// `*abc*`
    Infix(String),
    Regex(Regex),
}

impl Glob {
    /// Compiles `pattern` (see the module docs for the syntax).
    pub fn new(pattern: &str, case: GlobCase) -> Result<Glob, GlobError> {
        let tokens = tokenize(pattern)?;
        let matcher = matcher_of(&tokens, case)?;
        Ok(Glob {
            pattern: pattern.to_owned(),
            case,
            tokens,
            matcher,
        })
    }

    /// The pattern as written.
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    pub fn case(&self) -> GlobCase {
        self.case
    }

    /// Whether the pattern matches every name (`*`).
    pub fn matches_everything(&self) -> bool {
        matches!(self.matcher, Matcher::All)
    }

    /// Whether `name`, as a whole, matches.
    pub fn is_match(&self, name: &str) -> bool {
        let eq = |a: &[u8], b: &[u8]| match self.case {
            GlobCase::Exact => a == b,
            GlobCase::AsciiInsensitive => a.eq_ignore_ascii_case(b),
        };
        let n = name.as_bytes();
        match &self.matcher {
            Matcher::All => true,
            Matcher::Literal(l) => eq(n, l.as_bytes()),
            Matcher::Prefix(l) => n.len() >= l.len() && eq(&n[..l.len()], l.as_bytes()),
            Matcher::Suffix(l) => n.len() >= l.len() && eq(&n[n.len() - l.len()..], l.as_bytes()),
            // A window that compares equal starts on a character boundary: `l` starts with an
            // ASCII byte or a UTF-8 lead byte, and only ASCII bytes compare caselessly.
            Matcher::Infix(l) => n.windows(l.len()).any(|w| eq(w, l.as_bytes())),
            Matcher::Regex(r) => r.is_match(name),
        }
    }

    /// The glob as a `LIKE` pattern with `\` as its escape character: `*` → `%`, `?` and a
    /// class → `_`, a literal `%`, `_` or `\` → `\%`, `\_`, `\\`. A superset of the glob's
    /// matches (a class becomes any one character); the case is the server column's.
    pub fn like_pattern(&self) -> String {
        let mut out = String::with_capacity(self.pattern.len() + 4);
        for t in &self.tokens {
            match t {
                Tok::Lit(c) => {
                    if matches!(c, '%' | '_' | '\\') {
                        out.push('\\');
                    }
                    out.push(*c);
                }
                Tok::Many => out.push('%'),
                Tok::One | Tok::Class { .. } => out.push('_'),
            }
        }
        out
    }
}

fn tokenize(pattern: &str) -> Result<Vec<Tok>, GlobError> {
    let mut out = Vec::new();
    let mut it = pattern.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\\' => out.push(Tok::Lit(it.next().ok_or(GlobError::TrailingEscape)?)),
            '*' => {
                if out.last() != Some(&Tok::Many) {
                    out.push(Tok::Many);
                }
            }
            '?' => out.push(Tok::One),
            '[' => {
                let negated = it.next_if(|&c| c == '!' || c == '^').is_some();
                let mut ranges = Vec::new();
                let mut first = true;
                loop {
                    let mut lo = it.next().ok_or(GlobError::UnclosedClass)?;
                    if lo == ']' && !first {
                        break;
                    }
                    first = false;
                    if lo == '\\' {
                        lo = it.next().ok_or(GlobError::UnclosedClass)?;
                    }
                    let mut hi = lo;
                    if it.peek() == Some(&'-') {
                        let mut ahead = it.clone();
                        ahead.next();
                        if matches!(ahead.peek(), Some(&c) if c != ']') {
                            it.next();
                            hi = it.next().ok_or(GlobError::UnclosedClass)?;
                            if hi == '\\' {
                                hi = it.next().ok_or(GlobError::UnclosedClass)?;
                            }
                            if hi < lo {
                                return Err(GlobError::BadRange);
                            }
                        }
                    }
                    ranges.push((lo, hi));
                }
                out.push(Tok::Class { negated, ranges });
            }
            c => out.push(Tok::Lit(c)),
        }
    }
    Ok(out)
}

fn literal_run(toks: &[Tok]) -> Option<String> {
    toks.iter()
        .map(|t| match t {
            Tok::Lit(c) => Some(*c),
            _ => None,
        })
        .collect()
}

fn matcher_of(toks: &[Tok], case: GlobCase) -> Result<Matcher, GlobError> {
    let n = toks.len();
    let many_at = |i: usize| toks.get(i) == Some(&Tok::Many);
    let fast = match toks {
        [Tok::Many] => Some(Matcher::All),
        _ if n >= 3 && many_at(0) && many_at(n - 1) => {
            literal_run(&toks[1..n - 1]).map(Matcher::Infix)
        }
        _ if n >= 2 && many_at(0) => literal_run(&toks[1..]).map(Matcher::Suffix),
        _ if n >= 2 && many_at(n - 1) => literal_run(&toks[..n - 1]).map(Matcher::Prefix),
        _ => literal_run(toks).map(Matcher::Literal),
    };
    if let Some(m) = fast {
        return Ok(m);
    }
    let mut re = String::from(r"\A(?s:");
    for t in toks {
        match t {
            Tok::Lit(c) => push_lit(&mut re, *c, case),
            Tok::Many => re.push_str(".*"),
            Tok::One => re.push('.'),
            Tok::Class { negated, ranges } => {
                re.push('[');
                if *negated {
                    re.push('^');
                }
                for &(lo, hi) in ranges {
                    push_range(&mut re, lo, hi, case);
                }
                re.push(']');
            }
        }
    }
    re.push_str(r")\z");
    build(&re)
        .map(Matcher::Regex)
        .map_err(|_| GlobError::TooLarge)
}

/// A literal character in a regex: an ASCII letter as `[aA]` when caseless, every other
/// character escaped.
fn push_lit(re: &mut String, c: char, case: GlobCase) {
    if case == GlobCase::AsciiInsensitive && c.is_ascii_alphabetic() {
        re.push('[');
        re.push(c.to_ascii_lowercase());
        re.push(c.to_ascii_uppercase());
        re.push(']');
    } else {
        re.push_str(&regex::escape(c.encode_utf8(&mut [0; 4])));
    }
}

/// One range of a regex class, its ends as `\x{…}` escapes (never read as class syntax), plus
/// the other case of every ASCII letter it holds when caseless.
fn push_range(re: &mut String, lo: char, hi: char, case: GlobCase) {
    use std::fmt::Write as _;
    let _ = write!(re, r"\x{{{:X}}}-\x{{{:X}}}", u32::from(lo), u32::from(hi));
    if case == GlobCase::AsciiInsensitive {
        for c in other_cases(lo, hi) {
            let _ = write!(re, r"\x{{{:X}}}", u32::from(c));
        }
    }
}

/// The other case of every ASCII letter in `lo..=hi`.
fn other_cases(lo: char, hi: char) -> impl Iterator<Item = char> {
    ('A'..='Z')
        .chain('a'..='z')
        .filter(move |c| (lo..=hi).contains(c))
        .map(|c| {
            if c.is_ascii_uppercase() {
                c.to_ascii_lowercase()
            } else {
                c.to_ascii_uppercase()
            }
        })
}

/// Compiles `pattern` with the shared size limits.
pub(crate) fn build(pattern: &str) -> Result<Regex, regex::Error> {
    RegexBuilder::new(pattern)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_SIZE_LIMIT)
        .build()
}

/// A user's regular expression (`{…}` in the scope and filter grammars) compiled to match a
/// **whole** name, once per case rule: `(exact, ascii_caseless)`. `Err` when the text is not a
/// valid `regex` crate expression (or too large).
///
/// The text is first compiled alone, so its groups are balanced and `\A(?:…)\z` cannot be
/// escaped from. A verbose-mode (`(?x)`) comment at the end would swallow the closing `)\z`;
/// that wrapped form does not compile, and the retry ends the comment with a newline first
/// (whitespace in verbose mode).
pub(crate) fn full_match_pair(source: &str) -> Result<(Regex, Regex), regex::Error> {
    build(source)?;
    let exact = anchored(source, false)?;
    let caseless = match ascii_caseless(source).map(|s| anchored(&s, false)) {
        Some(Ok(r)) => r,
        // Text this reader does not follow (a verbose-mode comment holding `[`, a range
        // written with escapes that then fails): the regex crate's own case-insensitive mode,
        // Unicode simple case folding — still locale-free, a superset of the ASCII rule.
        _ => anchored(source, true)?,
    };
    Ok((exact, caseless))
}

/// `body` (valid alone) anchored at both ends; `unicode_caseless` sets the `regex` crate's
/// own case-insensitive mode.
fn anchored(body: &str, unicode_caseless: bool) -> Result<Regex, regex::Error> {
    let b = |p: &str| {
        RegexBuilder::new(p)
            .size_limit(REGEX_SIZE_LIMIT)
            .dfa_size_limit(REGEX_SIZE_LIMIT)
            .case_insensitive(unicode_caseless)
            .build()
    };
    b(&format!(r"\A(?:{body})\z")).or_else(|_| b(&format!("\\A(?:{body}\n)\\z")))
}

/// Rewrites a regular expression so that each literal ASCII letter also matches its other
/// case, and nothing else does: `a` → `[aA]`, a class item `x` → `[xX]`, a class range
/// `a-f` → `[a-f` plus `A`…`F` `]` (as a nested class, so no new range can form). The `regex`
/// crate's own `(?i)` would fold Unicode too (`k` ↔ U+212A KELVIN SIGN, `s` ↔ U+017F); the
/// server rule this follows is ASCII only (HV-INTRO-14).
///
/// Left as written: escapes (`\d`, `\p{Lu}`, `\x{41}`, an escaped letter is never a letter
/// literal), group heads and names (`(?P<name>`, `(?i:`), counted repetitions (`{2,3}`),
/// ASCII classes (`[[:alpha:]]`), and ranges with an escaped end. `None` when the text leaves
/// what this reader follows (an unclosed class or brace).
pub(crate) fn ascii_caseless(src: &str) -> Option<String> {
    let cs: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len() * 2);
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        match c {
            '\\' => i = copy_escape(&cs, i, &mut out)?,
            '[' => i = class(&cs, i, &mut out)?,
            '(' if cs.get(i + 1) == Some(&'?') => i = copy_group_head(&cs, i, &mut out)?,
            '{' => {
                let j = find(&cs, i, '}')?;
                out.extend(&cs[i..=j]);
                i = j + 1;
            }
            c if c.is_ascii_alphabetic() => {
                push_both(&mut out, c);
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    Some(out)
}

fn push_both(out: &mut String, c: char) {
    out.push('[');
    out.push(c.to_ascii_lowercase());
    out.push(c.to_ascii_uppercase());
    out.push(']');
}

fn find(cs: &[char], from: usize, c: char) -> Option<usize> {
    cs[from..].iter().position(|&d| d == c).map(|p| from + p)
}

/// Copies the escape at `cs[i]` (a `\`) and returns the index after it.
fn copy_escape(cs: &[char], i: usize, out: &mut String) -> Option<usize> {
    let e = *cs.get(i + 1)?;
    out.push('\\');
    out.push(e);
    let mut j = i + 2;
    let hex_digits = match e {
        'x' => 2,
        'u' => 4,
        'U' => 8,
        'p' | 'P' => {
            if cs.get(j) == Some(&'{') {
                let end = find(cs, j, '}')?;
                out.extend(&cs[j..=end]);
                return Some(end + 1);
            }
            out.push(*cs.get(j)?);
            return Some(j + 1);
        }
        _ => return Some(j),
    };
    if cs.get(j) == Some(&'{') {
        let end = find(cs, j, '}')?;
        out.extend(&cs[j..=end]);
        return Some(end + 1);
    }
    while j < cs.len() && j < i + 2 + hex_digits && cs[j].is_ascii_hexdigit() {
        out.push(cs[j]);
        j += 1;
    }
    Some(j)
}

/// Copies a group head `(?…` up to and including its `:`, `)` or `>` and returns the index
/// after it.
fn copy_group_head(cs: &[char], i: usize, out: &mut String) -> Option<usize> {
    out.push_str("(?");
    let mut j = i + 2;
    let named = match cs.get(j) {
        Some('<') => true,
        Some('P') if cs.get(j + 1) == Some(&'<') => true,
        _ => false,
    };
    if named {
        let end = find(cs, j, '>')?;
        out.extend(&cs[j..=end]);
        return Some(end + 1);
    }
    while let Some(&c) = cs.get(j) {
        out.push(c);
        j += 1;
        if c == ':' || c == ')' {
            return Some(j);
        }
    }
    None
}

/// Copies the class at `cs[i]` (a `[`) with the other case of every literal ASCII letter, and
/// returns the index after its `]`.
fn class(cs: &[char], mut i: usize, out: &mut String) -> Option<usize> {
    out.push('[');
    i += 1;
    if cs.get(i) == Some(&'^') {
        out.push('^');
        i += 1;
    }
    let mut first = true;
    loop {
        let c = *cs.get(i)?;
        let next = cs.get(i + 1).copied();
        match c {
            ']' if !first => {
                out.push(']');
                return Some(i + 1);
            }
            '[' if next == Some(':') => {
                // An ASCII class `[:alpha:]` / `[:^alpha:]`.
                let mut j = i + 2;
                loop {
                    if *cs.get(j)? == ':' && cs.get(j + 1) == Some(&']') {
                        break;
                    }
                    j += 1;
                }
                out.extend(&cs[i..j + 2]);
                i = j + 2;
            }
            '[' => i = class(cs, i, out)?,
            '&' | '-' | '~' if next == Some(c) => {
                out.push(c);
                out.push(c);
                i += 2;
            }
            _ => {
                // One item: a literal (perhaps escaped) or a range.
                let (lo, after_lo) = atom(cs, i)?;
                let is_range = cs.get(after_lo) == Some(&'-')
                    && !matches!(cs.get(after_lo + 1), None | Some(']' | '-'));
                if is_range {
                    let (hi, after_hi) = atom(cs, after_lo + 1)?;
                    match (lo, hi) {
                        (Atom::Plain(lo), Atom::Plain(hi)) if lo <= hi => {
                            out.push('[');
                            push_range(out, lo, hi, GlobCase::AsciiInsensitive);
                            out.push(']');
                        }
                        _ => out.extend(&cs[i..after_hi]),
                    }
                    i = after_hi;
                } else {
                    match lo {
                        Atom::Plain(c) if c.is_ascii_alphabetic() => push_both(out, c),
                        _ => out.extend(&cs[i..after_lo]),
                    }
                    i = after_lo;
                }
            }
        }
        first = false;
    }
}

enum Atom {
    Plain(char),
    Escaped,
}

/// The class atom at `cs[i]` and the index after it.
fn atom(cs: &[char], i: usize) -> Option<(Atom, usize)> {
    if *cs.get(i)? == '\\' {
        let mut scratch = String::new();
        let j = copy_escape(cs, i, &mut scratch)?;
        return Some((Atom::Escaped, j));
    }
    Some((Atom::Plain(cs[i]), i + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(p: &str) -> Glob {
        Glob::new(p, GlobCase::Exact).unwrap()
    }

    fn gi(p: &str) -> Glob {
        Glob::new(p, GlobCase::AsciiInsensitive).unwrap()
    }

    fn full_match(source: &str, case: GlobCase) -> Result<Regex, regex::Error> {
        let (exact, caseless) = full_match_pair(source)?;
        Ok(match case {
            GlobCase::Exact => exact,
            GlobCase::AsciiInsensitive => caseless,
        })
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(2_000))]

        /// On ASCII names the rewrite agrees with the `regex` crate's own `(?i)` (the two
        /// differ only on non-ASCII letters, where the rewrite stays exact).
        #[test]
        fn caseless_rewrite_matches_regex_case_insensitivity_on_ascii(
            pat in r"([abAB_.|*+?()^\-]|\[\^?[a-cA-C_-]{1,3}\]|\\d|\{1,2\}){1,8}",
            names in proptest::collection::vec("[abcABC_1-]{0,5}", 8),
        ) {
            let Ok((_, caseless)) = full_match_pair(&pat) else {
                return Ok(());
            };
            let unicode = anchored(&pat, true).unwrap();
            for n in &names {
                proptest::prop_assert_eq!(caseless.is_match(n), unicode.is_match(n), "{} ~ {}", pat, n);
            }
        }

        /// A glob and its regex fallback agree with the fast paths.
        #[test]
        fn glob_fast_paths_agree_with_the_regex_form(
            pat in r"[\*]?[abAB_%]{0,4}[\*]?",
            names in proptest::collection::vec("[abAB_%x]{0,6}", 8),
        ) {
            for case in [GlobCase::Exact, GlobCase::AsciiInsensitive] {
                let fast = Glob::new(&pat, case).unwrap();
                // One-character classes force the regex path without changing the meaning.
                let slow_pat: String = pat
                    .chars()
                    .map(|c| if c == '*' { "*".to_owned() } else { format!("[{c}]") })
                    .collect();
                let slow = Glob::new(&slow_pat, case).unwrap();
                for n in &names {
                    proptest::prop_assert_eq!(fast.is_match(n), slow.is_match(n), "{} ~ {}", pat, n);
                }
            }
        }
    }

    #[test]
    fn fast_paths_and_the_regex_path_agree() {
        for (p, yes, no) in [
            ("abc", "abc", "abcd"),
            ("ab*", "abzz", "zab"),
            ("*ab", "zzab", "abz"),
            ("*ab*", "zabz", "azb"),
            ("a?c", "abc", "ac"),
            ("a*c?", "axxcd", "axxc"),
            ("[ab]x", "bx", "cx"),
            ("[!ab]x", "cx", "ax"),
            ("[a-c]*", "b1", "d1"),
        ] {
            assert!(g(p).is_match(yes), "{p} ~ {yes}");
            assert!(!g(p).is_match(no), "{p} !~ {no}");
        }
        assert!(g("*").matches_everything());
        assert!(g("*").is_match(""));
    }

    #[test]
    fn literal_specials_and_escapes() {
        assert!(g("my_table").is_match("my_table"));
        assert!(!g("my_table").is_match("myXtable"));
        assert!(g(r"my\*table").is_match("my*table"));
        assert!(!g(r"my\*table").is_match("myXtable"));
        assert!(g("a|b").is_match("a|b"));
        assert!(g("100%").is_match("100%"));
        assert!(!g("100%").is_match("1000"));
        assert!(g(r"[\]]").is_match("]"));
        assert!(g("[]]").is_match("]"));
        assert!(g("[a-]").is_match("-"));
        assert_eq!(
            Glob::new("a\\", GlobCase::Exact),
            Err(GlobError::TrailingEscape)
        );
        assert_eq!(
            Glob::new("a[b", GlobCase::Exact),
            Err(GlobError::UnclosedClass)
        );
        assert_eq!(
            Glob::new("[z-a]", GlobCase::Exact),
            Err(GlobError::BadRange)
        );
    }

    #[test]
    fn question_mark_is_one_character_not_one_byte() {
        assert!(g("f?ş").is_match("fiş"));
        assert!(g("?").is_match("ş"));
        assert!(!g("?").is_match("şş"));
        assert!(g("a*b").is_match("a\nb"));
    }

    #[test]
    fn ascii_insensitive_folds_only_ascii_letters() {
        assert!(gi("items").is_match("ITEMS"));
        assert!(gi("it*").is_match("IT_x"));
        assert!(gi("*ems").is_match("ItEMS"));
        assert!(gi("*te*").is_match("xTEx"));
        assert!(gi("i?ems").is_match("ITEMS"));
        assert!(gi("[a-c]x").is_match("BX"));
        assert!(!gi("[!a]x").is_match("AX"));
        // Never Unicode or Turkish folding.
        assert!(!gi("fiş").is_match("FİŞ"));
        assert!(!gi("fiş").is_match("FIŞ"));
        assert!(gi("fiş").is_match("FIş"));
        assert!(!gi("k").is_match("\u{212A}"));
        assert!(!gi("ı?").is_match("Ix"));
        assert!(!g("items").is_match("ITEMS"));
    }

    #[test]
    fn like_pattern_escapes_specials_and_widens_classes() {
        assert_eq!(g("ord*").like_pattern(), "ord%");
        assert_eq!(g("a_b").like_pattern(), r"a\_b");
        assert_eq!(g("50%?").like_pattern(), r"50\%_");
        assert_eq!(g(r"a\\b").like_pattern(), r"a\\b");
        assert_eq!(g("[ab]*").like_pattern(), "_%");
        assert_eq!(g(r"x\*").like_pattern(), "x*");
    }

    #[test]
    fn caseless_rewrite_leaves_syntax_alone() {
        let r = |s: &str| ascii_caseless(s).unwrap();
        assert_eq!(r("ab"), "[aA][bB]");
        assert_eq!(r(r"\d+x"), r"\d+[xX]");
        assert_eq!(r(r"\p{Lu}\pL\x41\x{42}"), r"\p{Lu}\pL\x41\x{42}");
        assert_eq!(r("(?P<nm>a)(?i:b)"), "(?P<nm>[aA])(?i:[bB])");
        assert_eq!(r("a{2,3}"), "[aA]{2,3}");
        assert_eq!(r("[[:alpha:]]"), "[[:alpha:]]");
        assert_eq!(r("[^a]"), "[^[aA]]");
        assert!(ascii_caseless("[ab").is_none());
        let re = full_match("t[a-c]+_x", GlobCase::AsciiInsensitive).unwrap();
        assert!(re.is_match("TaBc_X"));
        assert!(!re.is_match("TaBc_X1"));
        let k = full_match("k", GlobCase::AsciiInsensitive).unwrap();
        assert!(k.is_match("K") && !k.is_match("\u{212A}"));
    }

    #[test]
    fn full_match_cannot_be_escaped_and_survives_verbose_comments() {
        assert!(full_match("a)|(b", GlobCase::Exact).is_err());
        let r = full_match("a|ab", GlobCase::Exact).unwrap();
        assert!(r.is_match("ab"));
        assert!(!r.is_match("abc"));
        let v = full_match("(?x) a b # trailing comment", GlobCase::Exact).unwrap();
        assert!(v.is_match("ab"));
        assert!(!v.is_match("abx"));
    }
}
