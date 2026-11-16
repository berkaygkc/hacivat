// ## m3b-t01c-core-seams
//! What the history never keeps (Task 17): bind values (masked once in `prepare_units`),
//! secrets in statements (`PASSWORD '…'`, `IDENTIFIED BY …`), error messages past 4 KiB.
// ## m3b-t17-history
//!
//! The writer redacts every statement before it hashes, fingerprints, caps or stores it
//! (`alg/20` HV-EXEC-12 step 2). The scan knows no dialect (a record carries none), so it reads
//! the text twice, once with backslash escapes inside quotes (MySQL) and once without
//! (standard SQL), and masks the union of what either reading finds: a string the two readings
//! disagree about is masked whichever reading is right.
//!
//! - **Secret keywords.** A word `PASSWORD`, `PASSWD`, `PWD`, `SECRET` or `IDENTIFIED`, or a
//!   word with such an `_`-separated part (`MASTER_PASSWORD`, `client_secret`), arms the rest
//!   of its statement (up to a `;`, or a psql meta-command): every quoted string after it
//!   (`'…'`, `"…"`, `$tag$…$tag$`) is masked. The value right after the keyword is masked even
//!   when it is a bare word (Oracle `IDENTIFIED BY tiger`): after `PASSWORD [=|(]`, after
//!   `IDENTIFIED [WITH plugin] BY|AS [PASSWORD|RANDOM]`, after `REPLACE`, and after
//!   `SET PASSWORD FOR <user> =`.
//! - **Connection strings.** A quoted string with `password`, `passwd`, `pwd=` or `secret` in
//!   it (ASCII case-insensitive), or a URI with a password in its user part
//!   (`postgresql://u:pw@h`), is masked wherever it stands (`CREATE SUBSCRIPTION … CONNECTION`,
//!   `dblink_connect(…)`).
//! - **psql meta-commands.** From a `\` outside quotes to the end of its line, the text becomes
//!   its `HvNorm1` form (Task 6): `\name` and one `?` for every argument
//!   (`\connect db u password=…` → `\connect ?`).
//!
//! A masked string becomes `'***'`, a masked word `***`. The values found in value positions
//! (and the `password=` values inside connection strings) are the record's *secrets*: the
//! writer also removes them from the error message and the parameter text, since servers quote
//! the failing statement in their errors (MySQL 1064: `near 'BY '…''`).

use std::ops::Range;

use sql_dialect::{DialectCtx, DialectId};
use sql_split::hv_norm1;

/// What a masked bare word becomes.
pub(crate) const MASK: &str = "***";
/// What a masked quoted string becomes.
const MASKED_STRING: &str = "'***'";

/// A redacted text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Redacted {
    pub text: String,
    /// Something was masked.
    pub changed: bool,
    /// The secret values found (for [`scrub`]).
    pub secrets: Vec<String>,
}

/// How deep the scan reads code quoted inside code for secret values
/// (`DO $$ … EXECUTE 'ALTER ROLE r PASSWORD ''…''' … $$`).
const MAX_DEPTH: u8 = 3;

/// Masks the secrets in `text` (the module docs).
pub(crate) fn redact(text: &str) -> Redacted {
    redact_at(text, 0)
}

fn redact_at(text: &str, depth: u8) -> Redacted {
    let mut masks = Vec::new();
    let mut secrets = Vec::new();
    for backslash in [false, true] {
        Scan::new(text, backslash, depth, &mut masks, &mut secrets).run();
    }
    secrets.retain(|s: &String| !s.trim().is_empty() && s != MASK);
    secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    secrets.dedup();
    if masks.is_empty() {
        return Redacted {
            text: text.to_owned(),
            changed: false,
            secrets,
        };
    }
    masks.sort_by_key(|m| (m.range.start, std::cmp::Reverse(m.range.end)));
    let mut merged: Vec<Mask> = Vec::with_capacity(masks.len());
    for m in masks {
        match merged.last_mut() {
            Some(last) if m.range.start < last.range.end => {
                if m.range.end > last.range.end || m.kind != last.kind {
                    last.range.end = last.range.end.max(m.range.end);
                    if m.kind != last.kind {
                        last.kind = Kind::Mixed;
                    }
                }
            }
            _ => merged.push(m),
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for m in &merged {
        out.push_str(&text[at..m.range.start]);
        let piece = &text[m.range.clone()];
        match m.kind {
            Kind::Meta => out.push_str(&hv_norm1(piece, &psql()).normalized),
            Kind::Word => out.push_str(MASK),
            Kind::String => out.push_str(MASKED_STRING),
            Kind::Mixed if piece.starts_with(['\'', '"', '$']) => out.push_str(MASKED_STRING),
            Kind::Mixed => out.push_str(MASK),
        }
        at = m.range.end;
    }
    out.push_str(&text[at..]);
    Redacted {
        text: out,
        changed: true,
        secrets,
    }
}

/// `text` (an error message, the parameter text) redacted, with every one of `secrets`
/// replaced by `***`. Answers whether anything changed.
pub(crate) fn scrub(text: &str, secrets: &[String]) -> (String, bool) {
    let r = redact(text);
    let mut out = r.text;
    let mut changed = r.changed;
    for s in secrets.iter().chain(&r.secrets) {
        if !s.is_empty() && out.contains(s.as_str()) {
            out = out.replace(s.as_str(), MASK);
            changed = true;
        }
    }
    (out, changed)
}

fn psql() -> DialectCtx {
    DialectCtx::new(DialectId::Postgres, None)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    String,
    Word,
    Meta,
    Mixed,
}

#[derive(Debug, Clone)]
struct Mask {
    range: Range<usize>,
    kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    Space,
    Comment,
    /// A quoted string (`'…'`, `"…"`) or a dollar body; `inner` is its content.
    String {
        inner: (usize, usize),
    },
    /// A backquoted identifier.
    Ident,
    Word,
    Number,
    /// `\` outside quotes to the end of the line.
    Meta,
    Semicolon,
    Punct(u8),
}

/// What the scan expects after a secret keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    Nothing,
    /// The value after `PASSWORD`-like words: `=` and `(` may come first; a keyword ends it.
    Value,
    /// The value after `IDENTIFIED … BY`, `AS` or `REPLACE`: any word but `PASSWORD`/`RANDOM`.
    StrictValue,
    /// After `IDENTIFIED`: `WITH plugin`, then `BY` or `AS`.
    Identified,
    /// After `IDENTIFIED WITH`: the plugin's name.
    Plugin,
    /// After `PASSWORD FOR`: the user, up to `=`.
    UserThenEq,
}

struct Scan<'a> {
    text: &'a str,
    bytes: &'a [u8],
    backslash: bool,
    depth: u8,
    at: usize,
    masks: &'a mut Vec<Mask>,
    secrets: &'a mut Vec<String>,
    armed: bool,
    identified: bool,
    expect: Expect,
}

impl<'a> Scan<'a> {
    fn new(
        text: &'a str,
        backslash: bool,
        depth: u8,
        masks: &'a mut Vec<Mask>,
        secrets: &'a mut Vec<String>,
    ) -> Self {
        Scan {
            text,
            bytes: text.as_bytes(),
            backslash,
            depth,
            at: 0,
            masks,
            secrets,
            armed: false,
            identified: false,
            expect: Expect::Nothing,
        }
    }

    fn run(&mut self) {
        while let Some((tok, range)) = self.next() {
            self.step(tok, range);
        }
    }

    fn reset(&mut self) {
        self.armed = false;
        self.identified = false;
        self.expect = Expect::Nothing;
    }

    fn mask(&mut self, range: Range<usize>, kind: Kind) {
        self.masks.push(Mask { range, kind });
    }

    fn secret(&mut self, s: &str) {
        self.secrets.push(s.to_owned());
    }

    fn step(&mut self, tok: Tok, range: Range<usize>) {
        let piece = &self.text[range.clone()];
        match tok {
            Tok::Space | Tok::Comment => {}
            Tok::Semicolon => self.reset(),
            Tok::Meta => {
                self.mask(range.clone(), Kind::Meta);
                for v in key_values(piece) {
                    self.secret(v);
                }
                // `\set pw …`, `\c "conninfo"`: every argument of three or more characters.
                for arg in piece.split_whitespace().skip(1) {
                    let arg = arg.trim_matches(|c| c == '\'' || c == '"');
                    if arg.len() >= 3 {
                        self.secret(arg);
                    }
                }
                self.reset();
            }
            Tok::String { inner } => {
                let content = &self.text[inner.0..inner.1];
                let value = matches!(self.expect, Expect::Value | Expect::StrictValue);
                if value {
                    self.secret(content);
                }
                let conninfo = looks_like_conninfo(content);
                if conninfo {
                    for v in key_values(content) {
                        self.secret(v);
                    }
                    if let Some(pw) = uri_password(content) {
                        self.secret(pw);
                    }
                    // Code quoted in code: its own secret values, quotes unescaped.
                    if self.depth < MAX_DEPTH {
                        // A dollar body is code as it stands; a quoted string doubles its quote.
                        let unquoted = match self.bytes[range.start] {
                            b'\'' => content.replace("''", "'"),
                            b'"' => content.replace("\"\"", "\""),
                            _ => content.to_owned(),
                        };
                        let inner = redact_at(&unquoted, self.depth + 1);
                        self.secrets.extend(inner.secrets);
                    }
                }
                if self.armed || value || conninfo {
                    self.mask(range, Kind::String);
                }
                self.after_value();
            }
            Tok::Ident | Tok::Number => {
                if matches!(self.expect, Expect::Value | Expect::StrictValue) {
                    self.secret(piece);
                    self.mask(range, Kind::Word);
                }
                self.after_value();
            }
            Tok::Punct(b) => match (self.expect, b) {
                (Expect::Value | Expect::StrictValue, b'=' | b'(') => {}
                // `U&'…'`: the `&` of the Unicode string prefix.
                (Expect::Value | Expect::StrictValue, b'&')
                    if matches!(self.bytes.get(range.end), Some(b'\'' | b'"')) => {}
                (Expect::UserThenEq, b'=') => self.expect = Expect::Value,
                (Expect::UserThenEq | Expect::Plugin, _) => {}
                _ => self.expect = Expect::Nothing,
            },
            Tok::Word => self.word(piece, range),
        }
    }

    /// After a value: `IDENTIFIED BY 'x' REPLACE 'y'` keeps the `IDENTIFIED` chain alive.
    fn after_value(&mut self) {
        self.expect = match self.expect {
            Expect::Plugin => Expect::Identified,
            Expect::UserThenEq => Expect::UserThenEq,
            _ => Expect::Nothing,
        };
    }

    fn word(&mut self, w: &str, range: Range<usize>) {
        // A string prefix (`E'…'`, `N'…'`, `_utf8mb4'…'`): the string is the value.
        let upper = w.to_ascii_uppercase();
        let next = self.bytes.get(range.end);
        let unicode = upper == "U"
            && next == Some(&b'&')
            && matches!(self.bytes.get(range.end + 1), Some(b'\'' | b'"'));
        if unicode
            || matches!(next, Some(b'\'' | b'"'))
                && (matches!(
                    upper.as_str(),
                    "E" | "N" | "B" | "X" | "U" | "R" | "BR" | "RB"
                ) || upper.starts_with('_'))
        {
            return;
        }
        match self.expect {
            Expect::StrictValue => {
                if !matches!(upper.as_str(), "PASSWORD" | "RANDOM") {
                    self.secret(w);
                    self.mask(range, Kind::Word);
                    self.expect = Expect::Nothing;
                }
                return;
            }
            Expect::Plugin => {
                self.expect = Expect::Identified;
                return;
            }
            Expect::Identified => {
                self.expect = match upper.as_str() {
                    "BY" | "AS" => Expect::StrictValue,
                    "WITH" => Expect::Plugin,
                    _ => Expect::Nothing,
                };
                return;
            }
            Expect::UserThenEq => return,
            Expect::Value => {
                if upper == "FOR" {
                    self.expect = Expect::UserThenEq;
                    return;
                }
                if !is_secret_word(&upper) && !KEYWORDS.contains(&upper.as_str()) {
                    self.secret(w);
                    self.mask(range, Kind::Word);
                    self.expect = Expect::Nothing;
                    return;
                }
                self.expect = Expect::Nothing;
            }
            Expect::Nothing => {}
        }
        if upper == "IDENTIFIED" {
            self.armed = true;
            self.identified = true;
            self.expect = Expect::Identified;
        } else if upper == "REPLACE" && self.identified {
            self.expect = Expect::StrictValue;
        } else if is_secret_word(&upper) {
            self.armed = true;
            self.expect = Expect::Value;
        }
    }

    /// The next token and its byte range.
    fn next(&mut self) -> Option<(Tok, Range<usize>)> {
        let b = self.bytes;
        let start = self.at;
        let c = *b.get(start)?;
        let tok = match c {
            b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c => {
                self.at = self.skip_while(start, |c| c.is_ascii_whitespace());
                Tok::Space
            }
            b'-' if b.get(start + 1) == Some(&b'-') => {
                self.at = self.line_end(start);
                Tok::Comment
            }
            b'/' if b.get(start + 1) == Some(&b'*') => {
                self.at = find(b, start + 2, b"*/").map_or(b.len(), |e| e + 2);
                Tok::Comment
            }
            b'\'' | b'"' => {
                let end = self.quoted(start, c);
                self.at = end;
                let close = usize::from(end > start + 1 && b[end - 1] == c && end - 1 > start);
                Tok::String {
                    inner: (start + 1, end - close),
                }
            }
            b'`' => {
                self.at = self.quoted(start, c);
                Tok::Ident
            }
            b'$' => match self.dollar_tag(start) {
                Some(tag_end) => {
                    let tag = &b[start..tag_end];
                    let (inner_end, end) = match find(b, tag_end, tag) {
                        Some(e) => (e, e + tag.len()),
                        None => (b.len(), b.len()),
                    };
                    self.at = end;
                    Tok::String {
                        inner: (tag_end, inner_end),
                    }
                }
                None => {
                    // `$1`: a parameter, read like a number.
                    self.at =
                        self.skip_while(start + 1, |c| c.is_ascii_alphanumeric() || c == b'_');
                    Tok::Number
                }
            },
            b'\\' => {
                self.at = self.line_end(start);
                Tok::Meta
            }
            b';' => {
                self.at = start + 1;
                Tok::Semicolon
            }
            c if c.is_ascii_digit() => {
                self.at = self.skip_while(start, |c| {
                    c.is_ascii_alphanumeric() || c == b'_' || c == b'.'
                });
                Tok::Number
            }
            c if is_word_byte(c) => {
                self.at = self.skip_while(start, |c| {
                    is_word_byte(c) || c.is_ascii_digit() || c == b'$'
                });
                Tok::Word
            }
            c => {
                // One character (a whole UTF-8 sequence for a non-ASCII one: those are words).
                self.at = start + 1;
                Tok::Punct(c)
            }
        };
        Some((tok, start..self.at))
    }

    fn skip_while(&self, mut i: usize, f: impl Fn(u8) -> bool) -> usize {
        while i < self.bytes.len() && f(self.bytes[i]) {
            i += 1;
        }
        i
    }

    fn line_end(&self, i: usize) -> usize {
        self.skip_while(i, |c| c != b'\n')
    }

    /// The end of the string opened by `q` at `start` (past its closing quote; the end of the
    /// text when it never closes). A doubled quote is one quote; a backslash escapes the next
    /// byte when this reading takes backslash escapes.
    fn quoted(&self, start: usize, q: u8) -> usize {
        let b = self.bytes;
        let mut i = start + 1;
        while i < b.len() {
            match b[i] {
                c if c == q => {
                    if b.get(i + 1) == Some(&q) {
                        i += 2;
                        continue;
                    }
                    return i + 1;
                }
                b'\\' if self.backslash && q != b'`' => i += 2,
                _ => i += 1,
            }
        }
        b.len()
    }

    /// `$$` or `$tag$` at `start`: the end of the opening tag.
    fn dollar_tag(&self, start: usize) -> Option<usize> {
        let b = self.bytes;
        let mut i = start + 1;
        if b.get(i).is_some_and(|c| c.is_ascii_digit()) {
            return None;
        }
        while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] >= 0x80) {
            i += 1;
        }
        (b.get(i) == Some(&b'$')).then_some(i + 1)
    }
}

/// A byte that starts or continues a word: ASCII letters, `_`, and every byte of a non-ASCII
/// character (identifiers may be Turkish).
fn is_word_byte(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

fn find(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if from > hay.len() || needle.is_empty() {
        return None;
    }
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// A word naming a secret: `PASSWORD`, `PASSWD`, `PWD`, `SECRET`, or one with such an
/// `_`-separated part (`upper` is ASCII-uppercased).
fn is_secret_word(upper: &str) -> bool {
    upper
        .split('_')
        .any(|p| matches!(p, "PASSWORD" | "PASSWD" | "PWD" | "SECRET"))
}

/// A string that looks like it carries a credential (a connection string, a URI with a
/// password, a key=value list naming one).
fn looks_like_conninfo(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    ["password", "passwd", "pwd=", "pwd =", "secret"]
        .iter()
        .any(|k| lower.contains(k))
        || uri_password(s).is_some()
}

/// The password of a URI's user part (`scheme://user:PASSWORD@host`).
fn uri_password(s: &str) -> Option<&str> {
    let mut rest = s;
    while let Some(p) = rest.find("://") {
        let auth_start = p + 3;
        let auth = &rest[auth_start..];
        let end = auth
            .find(|c: char| c == '/' || c == '?' || c == '#' || c.is_whitespace() || c == '\'')
            .unwrap_or(auth.len());
        let authority = &auth[..end];
        if let Some(at) = authority.rfind('@') {
            let user = &authority[..at];
            if let Some(colon) = user.find(':') {
                return Some(&user[colon + 1..]);
            }
        }
        rest = &rest[auth_start..];
    }
    None
}

/// The values of `password=`, `passwd=`, `pwd=` and `secret=` pairs in `s` (ASCII
/// case-insensitive keys, optional spaces around `=`, quoted or bare values).
fn key_values(s: &str) -> Vec<&str> {
    let lower = s.to_ascii_lowercase();
    let mut out = Vec::new();
    for key in ["password", "passwd", "pwd", "secret"] {
        let mut from = 0;
        while let Some(p) = lower[from..].find(key) {
            let mut i = from + p + key.len();
            from = i;
            let b = s.as_bytes();
            while i < b.len() && b[i] == b' ' {
                i += 1;
            }
            if b.get(i) != Some(&b'=') {
                continue;
            }
            i += 1;
            while i < b.len() && b[i] == b' ' {
                i += 1;
            }
            let (start, end) = match b.get(i) {
                Some(&q @ (b'\'' | b'"')) => {
                    let e = s[i + 1..].find(q as char).map_or(s.len(), |e| e + i + 1);
                    (i + 1, e)
                }
                _ => {
                    let e = s[i..]
                        .find(|c: char| {
                            c.is_whitespace() || matches!(c, '\'' | '"' | ';' | '&' | ')')
                        })
                        .map_or(s.len(), |e| e + i);
                    (i, e)
                }
            };
            if end > start {
                out.push(&s[start..end]);
            }
        }
    }
    out
}

/// Words that end the wait for a `PASSWORD` value: a column named `password` in a query is not
/// followed by a secret (`SELECT password FROM users`).
const KEYWORDS: &[&str] = &[
    "AND",
    "AS",
    "ASC",
    "BY",
    "CASE",
    "CROSS",
    "CURRENT",
    "DAY",
    "DEFAULT",
    "DELETE",
    "DESC",
    "DISCARD",
    "ELSE",
    "ENCRYPTED",
    "END",
    "EXPIRE",
    "FROM",
    "FULL",
    "GROUP",
    "HAVING",
    "HISTORY",
    "IN",
    "INNER",
    "INSERT",
    "INTERVAL",
    "INTO",
    "IS",
    "JOIN",
    "LEFT",
    "LIKE",
    "LIMIT",
    "LOCK",
    "NEVER",
    "NOT",
    "NULL",
    "OLD",
    "ON",
    "OPTIONAL",
    "OR",
    "ORDER",
    "OUTER",
    "RANDOM",
    "REQUIRE",
    "RETAIN",
    "REUSE",
    "RIGHT",
    "SELECT",
    "SET",
    "THEN",
    "TO",
    "UNENCRYPTED",
    "UNION",
    "UNLOCK",
    "UPDATE",
    "USING",
    "VALID",
    "VALUES",
    "WHEN",
    "WHERE",
    "WITH",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn r(s: &str) -> String {
        redact(s).text
    }

    #[test]
    fn the_alg_20_case() {
        assert_eq!(
            r("ALTER ROLE app PASSWORD 'x'"),
            "ALTER ROLE app PASSWORD '***'"
        );
    }

    #[test]
    fn bare_values_are_masked() {
        assert_eq!(
            r("CREATE USER u IDENTIFIED BY tiger"),
            "CREATE USER u IDENTIFIED BY ***"
        );
        assert_eq!(
            r("ALTER USER u IDENTIFIED BY tiger REPLACE lion"),
            "ALTER USER u IDENTIFIED BY *** REPLACE ***"
        );
        assert_eq!(
            r("CREATE USER u IDENTIFIED BY RANDOM PASSWORD"),
            "CREATE USER u IDENTIFIED BY RANDOM PASSWORD"
        );
    }

    #[test]
    fn a_password_column_in_a_query_is_left_alone() {
        let q = "SELECT password FROM users WHERE id = 1";
        assert_eq!(r(q), q);
        assert!(!redact(q).changed);
    }

    #[test]
    fn psql_meta_commands_keep_only_their_name() {
        assert_eq!(r("\\connect db u password=x"), "\\connect ?");
        assert_eq!(
            r("SELECT 1;\n\\c host=h password=x\nSELECT 2"),
            "SELECT 1;\n\\c ?\nSELECT 2"
        );
        assert_eq!(r("\\password"), "\\password");
    }

    #[test]
    fn secrets_are_the_values() {
        let s = redact("SET PASSWORD FOR 'u'@'h' = 'pw1'").secrets;
        assert_eq!(s, ["pw1"]);
        let s = redact("SELECT dblink_connect('host=h password=pw2 user=u')").secrets;
        assert!(s.contains(&"pw2".to_owned()));
        let s = redact("SELECT f('postgresql://u:pw3@h/db')").secrets;
        assert!(s.contains(&"pw3".to_owned()));
    }

    #[test]
    fn scrub_removes_secrets_from_other_text() {
        let s = redact("CREATE USER u IDENTIFIED BY 'pw4'").secrets;
        let (out, changed) = scrub("near 'BY 'pw4'' at line 1", &s);
        assert!(changed);
        assert!(!out.contains("pw4"), "{out}");
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(2_000))]
        /// Byte offsets never cut a character, whatever the text.
        #[test]
        fn redaction_never_panics(s in "\\PC{0,80}") {
            let _ = scrub(&s, &redact(&s).secrets);
        }

        /// SQL-shaped text with quotes, escapes, dollars, meta-commands and Turkish letters.
        #[test]
        fn sql_shaped_text_never_panics(
            s in "(PASSWORD|IDENTIFIED|BY|WITH|FOR|REPLACE|U&|E|\\$\\$|\\$t\\$|'|\"|`|\\\\|;|=|\\(|\\)|/\\*|\\*/|--|\n| |ş|İ|x|1|password=|://u:p@h){0,40}"
        ) {
            let r = redact(&s);
            let _ = scrub(&s, &r.secrets);
        }
    }
}
