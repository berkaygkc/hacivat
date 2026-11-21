//! Secret-literal detection for the save warning (HV-SCRIPT-10; R-M4b-17, -28; alg/28 C9).
//!
//! ## m4b-t01a-seams
//! Task 1a: the types and a fail-closed [`scan_secrets`] stub that answered one finding over
//! the whole text, so a save asks instead of silently passing.
//!
//! This crate never depends on `hacivat-exec` (R-M4b-28): the app passes the history
//! redactor's spans, `hacivat_exec::history::secret_spans(text)`, mapping each
//! `hacivat_exec::history::SecretSpanKind` onto [`SpanKind`] one to one
//! (`Value` → `Password`, `ConnectionValue` → `ConnectionString`, `Armed` → `Other`).
//!
//! ## m4b-t06-secrets
//! Task 6: the scanner. [`scan_secrets`] reports
//! 1. the history spans of kind [`SpanKind::Password`] and [`SpanKind::ConnectionString`]
//!    (value positions only, C9), unless the literal is empty, all `*`, a bind parameter
//!    (`:name`, `$1`, `?`, `@name`), a template (`${…}`, `{{…}}`, `${{ … }}`, `{name}`, `<…>`,
//!    `%s`, `%(name)s`, `$NAME`, `[REDACTED]`), a bare `NULL`, or a bare word that calls a
//!    function (`now()`);
//! 2. the token formats of `data/secret_patterns.toml` (vendor facts, our own patterns);
//! 3. minus the findings on a line whose comment contains `hacivat:allow-secret`: the marker
//!    counts only when a small lexer, run once with backslash escapes and `#` comments (the
//!    MySQL reading) and once without (standard SQL), puts it inside a comment that starts on
//!    that line in **both** readings (fix round 1, m1).
//!
//! Overlapping findings are one finding, of the most specific kind (fix round 1, m3). A finding
//! holds a range, a zero-based line and a kind, never the value; nothing here logs.
//! The regexes run on the `regex` crate (linear in the text, no backtracking), compiled once.
//! Casing is ASCII only (`eq_ignore_ascii_case`), never locale casing. A pattern table that
//! fails to load answers one [`SecretKind::Unscanned`] finding over the whole text (fail
//! closed), and so does a span that is out of range or cuts a character (it is reported, its
//! range clamped to the text). The app runs the scan off the UI thread.

use std::ops::Range;
use std::sync::OnceLock;

use regex::Regex;
use serde::Deserialize;

/// The kind of a history-redaction span, mirrored from `hacivat_exec::history::SecretSpanKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpanKind {
    /// A value right after a secret keyword (`PASSWORD 'x'`, `IDENTIFIED BY x`): reported.
    Password,
    /// A connection-string or URI password value (`password=…`, `user:pass@`): reported.
    ConnectionString,
    /// Another string of a statement a secret word armed (`WHERE password_hash = 'x'`): masked
    /// in history, never reported (C9).
    Other,
}

/// What a finding is. Never the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SecretKind {
    Password,
    ConnectionString,
    /// A vendor token format (AWS, GitHub, Slack, Stripe live, Google API key).
    AccessToken,
    /// A test-mode key (`sk_test_`/`rk_test_`): lower severity.
    TestKey,
    PrivateKey,
    Jwt,
    /// The text was not scanned (the pattern table failed to load): a save asks.
    Unscanned,
}

/// One finding: where and what, never the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretFinding {
    /// The byte range of the literal in the scanned text.
    pub range: Range<usize>,
    /// Zero-based line of the range's start.
    pub line: u32,
    pub kind: SecretKind,
}

// ## m4b-t06-secrets
/// The secret-like literals of `text`, given the history redactor's spans for it (module
/// docs), sorted by position; overlapping findings are one finding.
pub fn scan_secrets(text: &str, history_spans: &[(Range<usize>, SpanKind)]) -> Vec<SecretFinding> {
    let Ok(patterns) = patterns() else {
        return vec![SecretFinding {
            range: 0..text.len(),
            line: 0,
            kind: SecretKind::Unscanned,
        }];
    };
    let mut found: Vec<(Range<usize>, SecretKind)> = Vec::new();
    for (range, kind) in history_spans {
        let kind = match kind {
            SpanKind::Password => SecretKind::Password,
            SpanKind::ConnectionString => SecretKind::ConnectionString,
            SpanKind::Other => continue,
        };
        match text.get(range.clone()) {
            Some(lit) if range.start <= range.end => {
                if !not_a_secret(lit, &text[range.end..]) {
                    found.push((range.clone(), kind));
                }
            }
            // A span this text cannot hold: reported, never dropped (fail closed).
            _ => found.push((clamp(text, range), kind)),
        }
    }
    for p in patterns {
        tokens(p, text, &mut found);
    }
    let lines = LineIndex::new(text);
    let allowed = allowed_lines(text);
    found.retain(|(r, _)| allowed.binary_search(&lines.line(r.start)).is_err());
    found.sort_by_key(|(r, k)| (r.start, r.end, *k as u8));
    let mut out: Vec<SecretFinding> = Vec::with_capacity(found.len());
    // ## m4b-t06-secrets (fix round 1, m3): one finding per overlapping run, whatever the
    // kinds (`…?password=s3cret` is a history value and a URI password): the dialog lists a
    // line once. The run keeps its most specific kind.
    for (range, kind) in found {
        if let Some(f) = out.last_mut()
            && (range.start < f.range.end || range.start == f.range.start)
        {
            f.range.end = f.range.end.max(range.end);
            if rank(kind) > rank(f.kind) {
                f.kind = kind;
            }
            continue;
        }
        out.push(SecretFinding {
            line: lines.line(range.start),
            range,
            kind,
        });
    }
    out
}

// ## m4b-t06-secrets (fix round 1, m3)
/// How specific a kind is: a token format says more than "a password goes here".
fn rank(kind: SecretKind) -> u8 {
    match kind {
        SecretKind::Unscanned => 7,
        SecretKind::PrivateKey => 6,
        SecretKind::AccessToken => 5,
        SecretKind::Jwt => 4,
        SecretKind::TestKey => 3,
        SecretKind::ConnectionString => 2,
        SecretKind::Password => 1,
    }
}

// ## m4b-t06-secrets (fix round 1, m11)
/// The alg/28 sources a pattern row may cite (§0.2: the vendor and RFC documents).
const SOURCES: &[u32] = &[9, 10, 11, 12, 13, 14, 15, 16, 21, 22];

/// The comment marker that silences its line (R-M4b-17).
const ALLOW: &str = "hacivat:allow-secret";

/// One row of `data/secret_patterns.toml`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Row {
    id: String,
    kind: String,
    regex: String,
    source: String,
    #[serde(default)]
    check: Option<String>,
    // ## m4b-t06-secrets (fix round 1, m5)
    #[serde(default)]
    min_len: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Table {
    pattern: Vec<Row>,
}

/// A compiled token format.
struct Pattern {
    kind: SecretKind,
    regex: Regex,
    /// The regex has a group named `value`: that group is the literal.
    value_group: bool,
    check: Check,
    /// The shortest match reported, in bytes.
    min_len: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Check {
    None,
    /// The first `.`-segment decodes (base64url) to a JSON object naming `"alg"`.
    JwtHeader,
    // ## m4b-t06-secrets (fix round 1, m4)
    /// The characters after the 4-letter prefix hold a digit.
    AwsShape,
}

/// Why the pattern table did not load (never shown: the scan answers `Unscanned`).
#[derive(Debug)]
struct TableError;

fn patterns() -> Result<&'static [Pattern], &'static TableError> {
    static TABLE: OnceLock<Result<Vec<Pattern>, TableError>> = OnceLock::new();
    TABLE
        .get_or_init(|| load(include_str!("../data/secret_patterns.toml")))
        .as_deref()
}

fn load(toml_text: &str) -> Result<Vec<Pattern>, TableError> {
    let table: Table = toml::from_str(toml_text).map_err(|_| TableError)?;
    if table.pattern.is_empty() {
        return Err(TableError);
    }
    let mut ids: Vec<&str> = Vec::new();
    let mut out = Vec::with_capacity(table.pattern.len());
    for row in &table.pattern {
        if row.id.is_empty() || ids.contains(&row.id.as_str()) || !cited(&row.source) {
            return Err(TableError);
        }
        ids.push(&row.id);
        let kind = match row.kind.as_str() {
            "access_token" => SecretKind::AccessToken,
            "test_key" => SecretKind::TestKey,
            "private_key" => SecretKind::PrivateKey,
            "jwt" => SecretKind::Jwt,
            "connection_string" => SecretKind::ConnectionString,
            "password" => SecretKind::Password,
            _ => return Err(TableError),
        };
        let check = match row.check.as_deref() {
            None => Check::None,
            Some("jwt_header") => Check::JwtHeader,
            Some("aws_shape") => Check::AwsShape,
            Some(_) => return Err(TableError),
        };
        let regex = Regex::new(&row.regex).map_err(|_| TableError)?;
        let value_group = regex.capture_names().any(|n| n == Some("value"));
        out.push(Pattern {
            kind,
            regex,
            value_group,
            check,
            min_len: row.min_len.unwrap_or(0),
        });
    }
    Ok(out)
}

// ## m4b-t06-secrets (fix round 1, m11)
/// `source` names one of [`SOURCES`]: `alg/28 S<n> <document>`.
fn cited(source: &str) -> bool {
    let Some(rest) = source.strip_prefix("alg/28 S") else {
        return false;
    };
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    rest.as_bytes().get(digits) == Some(&b' ')
        && rest[..digits]
            .parse::<u32>()
            .is_ok_and(|n| SOURCES.contains(&n))
}

/// The matches of `p` in `text`, added to `found`.
fn tokens(p: &Pattern, text: &str, found: &mut Vec<(Range<usize>, SecretKind)>) {
    if p.value_group {
        for caps in p.regex.captures_iter(text) {
            if let Some(v) = caps.name("value")
                && caps.get(0).is_some_and(|m| m.len() >= p.min_len)
                && !placeholder(unquote(v.as_str()).unwrap_or(v.as_str()))
            {
                found.push((v.range(), p.kind));
            }
        }
        return;
    }
    for m in p.regex.find_iter(text) {
        let shaped = match p.check {
            Check::None => true,
            Check::JwtHeader => jwt_header(m.as_str()),
            Check::AwsShape => m.as_str().bytes().skip(4).any(|c| c.is_ascii_digit()),
        };
        if shaped && m.len() >= p.min_len {
            found.push((m.range(), p.kind));
        }
    }
}

/// `lit` (a span's text, followed in the scanned text by `after`) is no secret literal.
fn not_a_secret(lit: &str, after: &str) -> bool {
    let quoted = unquote(lit);
    let inner = quoted.unwrap_or(lit);
    if placeholder(inner) {
        return true;
    }
    if quoted.is_some() {
        return false;
    }
    // A bare word: `NULL`, a parameter, or a function's name (`passwd_changed = now()`).
    lit.eq_ignore_ascii_case("NULL")
        || lit.starts_with(['?', ':', '@'])
        || (lit.starts_with('$') && lit[1..].bytes().all(|c| c.is_ascii_digit()))
        || after.trim_start().starts_with('(')
}

/// A literal that stands for a value instead of being one: empty, all `*`, a template.
fn placeholder(inner: &str) -> bool {
    let t = inner.trim();
    let wrapped = |open: &str, close: &str| {
        t.len() > open.len() + close.len()
            && t.starts_with(open)
            && t.ends_with(close)
            && !t[open.len()..t.len() - close.len()].contains(['{', '}', '<', '>'])
    };
    // ## m4b-t06-secrets (fix round 1, m6): `%s`, `%(name)s`, `$NAME`, `{name}`,
    // `${{ secrets.PW }}`, `[REDACTED]`.
    let printf = t == "%s"
        || (t.starts_with("%(")
            && t.ends_with(")s")
            && t.len() > 4
            && t[2..t.len() - 2].bytes().all(is_name_byte));
    let env_var = t.len() > 1
        && t.starts_with('$')
        && t.as_bytes()[1].is_ascii_uppercase()
        && t[1..]
            .bytes()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_');
    let redacted = t.len() > 2
        && t.starts_with('[')
        && t.ends_with(']')
        && t[1..t.len() - 1]
            .bytes()
            .all(|c| c.is_ascii_uppercase() || c == b'_' || c == b' ');
    t.is_empty()
        || t.bytes().all(|c| c == b'*')
        || wrapped("${{", "}}")
        || wrapped("${", "}")
        || wrapped("{{", "}}")
        || wrapped("{", "}")
        || wrapped("<", ">")
        || (t.starts_with(':') && t.len() > 1 && t[1..].bytes().all(is_name_byte))
        || t == "?"
        || (t.starts_with('$') && t.len() > 1 && t[1..].bytes().all(|c| c.is_ascii_digit()))
        || printf
        || env_var
        || redacted
}

fn is_name_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// The content of a quoted literal (`'…'`, `"…"`, `$tag$…$tag$`); `None` for a bare word.
fn unquote(lit: &str) -> Option<&str> {
    let b = lit.as_bytes();
    match b.first()? {
        &q @ (b'\'' | b'"') => {
            let end = if b.len() >= 2 && b[b.len() - 1] == q {
                b.len() - 1
            } else {
                b.len()
            };
            Some(&lit[1..end])
        }
        b'$' => {
            let tag_len = lit[1..].find('$')? + 2;
            let tag = &lit[..tag_len];
            if lit.len() >= 2 * tag_len && lit.ends_with(tag) {
                Some(&lit[tag_len..lit.len() - tag_len])
            } else {
                Some(&lit[tag_len..])
            }
        }
        _ => None,
    }
}

/// `range` clamped into `text` on character boundaries (start ≤ end).
fn clamp(text: &str, range: &Range<usize>) -> Range<usize> {
    let mut start = range.start.min(text.len());
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = range.end.max(start).min(text.len());
    while !text.is_char_boundary(end) {
        end += 1;
    }
    start..end
}

/// A JWT's header (its first `.`-segment, base64url) decodes to a JSON object naming `"alg"`
/// (RFC 7519 §3, RFC 7515 §4.1.1).
fn jwt_header(token: &str) -> bool {
    let Some(head) = token.split('.').next() else {
        return false;
    };
    let Some(json) = base64url(head) else {
        return false;
    };
    json.starts_with(b"{\"") && json.windows(5).any(|w| w == b"\"alg\"")
}

/// Unpadded base64url (RFC 4648 §5); `None` on a character outside the alphabet.
fn base64url(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Zero-based line numbers by byte offset: a line ends at `\n`, so `\r\n` counts once.
struct LineIndex {
    newlines: Vec<usize>,
}

impl LineIndex {
    fn new(text: &str) -> Self {
        LineIndex {
            newlines: text
                .bytes()
                .enumerate()
                .filter(|&(_, c)| c == b'\n')
                .map(|(i, _)| i)
                .collect(),
        }
    }

    fn line(&self, at: usize) -> u32 {
        u32::try_from(self.newlines.partition_point(|&n| n < at)).unwrap_or(u32::MAX)
    }
}

// ## m4b-t06-secrets (fix round 1, m1)
/// The lines whose comment holds [`ALLOW`], ascending: each marker counts only when both
/// readings of [`comments`] put it wholly inside a comment that starts on the marker's line.
/// A quote, `#>` (PostgreSQL) or a `#` inside a password never makes one (fail towards a
/// warning).
fn allowed_lines(text: &str) -> Vec<u32> {
    let mut out = Vec::new();
    if !text.contains(ALLOW) {
        return out;
    }
    let lines = LineIndex::new(text);
    let readings = [comments(text, false), comments(text, true)];
    for (at, _) in text.match_indices(ALLOW) {
        let line = lines.line(at);
        let inside = readings.iter().all(|cs| {
            let i = cs.partition_point(|c| c.start <= at);
            i > 0 && {
                let c = &cs[i - 1];
                at + ALLOW.len() <= c.end && lines.line(c.start) == line
            }
        });
        if inside && out.last() != Some(&line) {
            out.push(line);
        }
    }
    out
}

// ## m4b-t06-secrets (fix round 1, m1)
/// The comments of `text`, in order, as one reading sees them. The MySQL reading (`mysql`)
/// takes backslash escapes in quotes, backquoted names, `#` line comments, `--` only before
/// white space or the end, block comments that do not nest, and `/*!…`/`/*M!…` bodies as code;
/// the standard reading takes `--` always, nesting block comments and `$tag$` bodies. One pass,
/// linear in the text.
fn comments(text: &str, mysql: bool) -> Vec<Range<usize>> {
    let b = text.as_bytes();
    let line_end = |i: usize| {
        b[i..]
            .iter()
            .position(|&c| c == b'\n')
            .map_or(b.len(), |p| i + p)
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            q @ (b'\'' | b'"') => i = quoted_end(b, i, q, mysql),
            b'`' if mysql => i = quoted_end(b, i, b'`', false),
            b'-' if b.get(i + 1) == Some(&b'-')
                && (!mysql
                    || b.get(i + 2)
                        .is_none_or(|c| c.is_ascii_whitespace() || *c < 0x20)) =>
            {
                let end = line_end(i);
                out.push(i..end);
                i = end;
            }
            b'#' if mysql => {
                let end = line_end(i);
                out.push(i..end);
                i = end;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                if mysql && (b.get(i + 2) == Some(&b'!') || b[i + 2..].starts_with(b"M!")) {
                    // MySQL executable comment: its body is code.
                    i += 3;
                    continue;
                }
                let end = block_end(b, i, !mysql);
                out.push(i..end);
                i = end;
            }
            b'$' if !mysql && (i == 0 || !is_ident_byte(b[i - 1])) => {
                i = match dollar_tag(b, i) {
                    Some(tag_end) => {
                        let tag = &b[i..tag_end];
                        b[tag_end..]
                            .windows(tag.len())
                            .position(|w| w == tag)
                            .map_or(b.len(), |p| tag_end + p + tag.len())
                    }
                    None => i + 1,
                }
            }
            _ => i += 1,
        }
    }
    out
}

/// Past the string opened by `q` at `start` (a doubled quote is one quote; a backslash escapes
/// the next byte when `backslash`); the end of the text when it never closes.
fn quoted_end(b: &[u8], start: usize, q: u8, backslash: bool) -> usize {
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
            b'\\' if backslash => i += 2,
            _ => i += 1,
        }
    }
    b.len()
}

/// Past the block comment opened at `start`, nesting when `nest`; the end of the text when it
/// never closes.
fn block_end(b: &[u8], start: usize, nest: bool) -> usize {
    let mut depth = 1u32;
    let mut i = start + 2;
    while i + 1 < b.len() {
        match (b[i], b[i + 1]) {
            (b'/', b'*') if nest => {
                depth += 1;
                i += 2;
            }
            (b'*', b'/') => {
                depth -= 1;
                i += 2;
                if depth == 0 {
                    return i;
                }
            }
            _ => i += 1,
        }
    }
    b.len()
}

/// `$$` or `$tag$` at `start`: the end of the opening tag.
fn dollar_tag(b: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 1;
    if b.get(i).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    while i < b.len() && is_ident_byte(b[i]) {
        i += 1;
    }
    (b.get(i) == Some(&b'$')).then_some(i + 1)
}

fn is_ident_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}
// ## m4b-t06-secrets

#[cfg(test)]
mod tests {
    // ## m4b-t06-secrets
    use super::*;

    #[test]
    fn a_broken_table_fails_closed() {
        for bad in [
            "",
            "pattern = []",
            "[[pattern]]\nid = \"x\"\nkind = \"access_token\"\nregex = \"(\"\nsource = \"alg/28 S9 x\"",
            "[[pattern]]\nid = \"x\"\nkind = \"nope\"\nregex = \"a\"\nsource = \"alg/28 S9 x\"",
            "[[pattern]]\nid = \"x\"\nkind = \"jwt\"\nregex = \"a\"\nsource = \"somewhere\"",
            "[[pattern]]\nid = \"x\"\nkind = \"jwt\"\nregex = \"a\"\nsource = \"alg/28 S9 x\"\ncheck = \"nope\"",
            "[[pattern]]\nid = \"x\"\nkind = \"jwt\"\nregex = \"a\"\nsource = \"alg/28 S9 x\"\nextra = 1",
            "[[pattern]]\nid = \"x\"\nkind = \"jwt\"\nregex = \"a\"\nsource = \"alg/28 S9 x\"\n[[pattern]]\nid = \"x\"\nkind = \"jwt\"\nregex = \"b\"\nsource = \"alg/28 S9 x\"",
        ] {
            assert!(load(bad).is_err(), "{bad}");
        }
        assert!(patterns().is_ok());
    }

    #[test]
    fn base64url_and_jwt_header() {
        // m10: JWT-shaped fixtures are joined from parts.
        let head = concat!("eyJ", "hbGciOiJub25lIn0");
        assert_eq!(base64url(head).unwrap(), br#"{"alg":"none"}"#);
        assert!(base64url("a+b").is_none());
        assert!(jwt_header(&format!("{head}.x.")));
        assert!(!jwt_header(concat!("eyJ", "zdWIiOiIxIn0.x.y")));
    }

    #[test]
    fn placeholders() {
        for p in [
            "",
            "  ",
            "***",
            "${A}",
            "{{a}}",
            "{{ secrets.PW }}",
            "${{ secrets.PW }}",
            "{pw}",
            "<password>",
            ":pw",
            "?",
            "$1",
            "%s",
            "%(pw)s",
            "$PW",
            "$APP_PW2",
            "[REDACTED]",
        ] {
            assert!(placeholder(p), "{p}");
        }
        for v in [
            "x", "changeme", "${a}b", "<a><b>", "{{a}}}", "$", ":", "*x*", "{}", "$pw", "%x",
            "[x]", "%()s",
        ] {
            assert!(!placeholder(v), "{v}");
        }
    }

    #[test]
    fn sources_are_pinned() {
        for ok in [
            "alg/28 S9 AWS",
            "alg/28 S16 RFC",
            "alg/28 S21 RFC",
            "alg/28 S22 MS",
        ] {
            assert!(cited(ok), "{ok}");
        }
        for bad in [
            "alg/28 S1 x",
            "alg/28 S17 x",
            "alg/28 S9",
            "alg/28 Sx y",
            "S9 x",
            "alg/28 S99 x",
        ] {
            assert!(!cited(bad), "{bad}");
        }
    }

    #[test]
    fn the_comment_lexer_reads_both_ways() {
        let t = "SELECT '--x', `#y` -- a\n# b\n/* c /* d */ e */ $$ -- f $$ /*!80000 g */";
        let std: Vec<&str> = comments(t, false).iter().map(|r| &t[r.clone()]).collect();
        assert_eq!(std, ["-- a", "/* c /* d */ e */", "/*!80000 g */"], "{t}");
        let my: Vec<&str> = comments(t, true).iter().map(|r| &t[r.clone()]).collect();
        assert_eq!(
            my,
            ["-- a", "# b", "/* c /* d */", "-- f $$ /*!80000 g */"],
            "{t}"
        );
    }
}
