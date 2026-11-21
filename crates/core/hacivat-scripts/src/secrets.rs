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
//!    (`:name`, `$1`, `?`, `@name`), a template (`${…}`, `{{…}}`, `<…>`), `NULL`, or a bare
//!    word that calls a function (`now()`);
//! 2. the token formats of `data/secret_patterns.toml` (vendor facts, our own patterns);
//! 3. minus the findings on a line whose comment (`--`, `#` or `/*` before the marker, on the
//!    same line) contains `hacivat:allow-secret`.
//!
//! A finding holds a range, a zero-based line and a kind, never the value; nothing here logs.
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
/// docs), sorted by position; overlapping findings of one kind are one finding.
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
    // The latest finding of each kind: a later one of that kind overlapping it is merged.
    let mut last: [Option<usize>; KINDS] = [None; KINDS];
    for (range, kind) in found {
        let slot = &mut last[kind as usize];
        if let Some(f) = slot.map(|i| &mut out[i])
            && (range.start < f.range.end || range.start == f.range.start)
        {
            f.range.end = f.range.end.max(range.end);
            continue;
        }
        *slot = Some(out.len());
        out.push(SecretFinding {
            line: lines.line(range.start),
            range,
            kind,
        });
    }
    out
}

/// The number of [`SecretKind`]s (an index per kind when merging).
const KINDS: usize = SecretKind::Unscanned as usize + 1;

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
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Check {
    None,
    /// The first `.`-segment decodes (base64url) to a JSON object naming `"alg"`.
    JwtHeader,
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
        if row.id.is_empty()
            || ids.contains(&row.id.as_str())
            || !row.source.starts_with("alg/28 S")
        {
            return Err(TableError);
        }
        ids.push(&row.id);
        let kind = match row.kind.as_str() {
            "access_token" => SecretKind::AccessToken,
            "test_key" => SecretKind::TestKey,
            "private_key" => SecretKind::PrivateKey,
            "jwt" => SecretKind::Jwt,
            "connection_string" => SecretKind::ConnectionString,
            _ => return Err(TableError),
        };
        let check = match row.check.as_deref() {
            None => Check::None,
            Some("jwt_header") => Check::JwtHeader,
            Some(_) => return Err(TableError),
        };
        let regex = Regex::new(&row.regex).map_err(|_| TableError)?;
        let value_group = regex.capture_names().any(|n| n == Some("value"));
        out.push(Pattern {
            kind,
            regex,
            value_group,
            check,
        });
    }
    Ok(out)
}

/// The matches of `p` in `text`, added to `found`.
fn tokens(p: &Pattern, text: &str, found: &mut Vec<(Range<usize>, SecretKind)>) {
    if p.value_group {
        for caps in p.regex.captures_iter(text) {
            if let Some(v) = caps.name("value")
                && !placeholder(v.as_str())
            {
                found.push((v.range(), p.kind));
            }
        }
        return;
    }
    for m in p.regex.find_iter(text) {
        if p.check == Check::JwtHeader && !jwt_header(m.as_str()) {
            continue;
        }
        found.push((m.range(), p.kind));
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
    t.is_empty()
        || t.bytes().all(|c| c == b'*')
        || wrapped("${", "}")
        || wrapped("{{", "}}")
        || wrapped("<", ">")
        || (t.starts_with(':') && t.len() > 1 && t[1..].bytes().all(is_name_byte))
        || t == "?"
        || (t.starts_with('$') && t.len() > 1 && t[1..].bytes().all(|c| c.is_ascii_digit()))
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

/// The lines whose comment holds [`ALLOW`], ascending: the marker follows a `--`, `#` or `/*`
/// on its own line.
fn allowed_lines(text: &str) -> Vec<u32> {
    let mut out = Vec::new();
    if !text.contains(ALLOW) {
        return out;
    }
    for (n, line) in text.split('\n').enumerate() {
        let Some(at) = line.find(ALLOW) else {
            continue;
        };
        let before = &line[..at];
        if before.contains("--") || before.contains('#') || before.contains("/*") {
            out.push(u32::try_from(n).unwrap_or(u32::MAX));
        }
    }
    out
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
        assert_eq!(
            base64url("eyJhbGciOiJub25lIn0").unwrap(),
            br#"{"alg":"none"}"#
        );
        assert!(base64url("a+b").is_none());
        assert!(jwt_header("eyJhbGciOiJub25lIn0.x."));
        assert!(!jwt_header("eyJzdWIiOiIxIn0.x.y"));
    }

    #[test]
    fn placeholders() {
        for p in [
            "",
            "  ",
            "***",
            "${A}",
            "{{a}}",
            "<password>",
            ":pw",
            "?",
            "$1",
        ] {
            assert!(placeholder(p), "{p}");
        }
        for v in [
            "x", "changeme", "${a}b", "<a><b>", "{{a}}}", "$", ":", "*x*",
        ] {
            assert!(!placeholder(v), "{v}");
        }
    }
}
