//! libpq keyword/value connection strings (`host=... dbname=...`) (HV-DRV-04), tokenized as
//! libpq does: `key = value` pairs separated by whitespace (C `isspace`), whitespace allowed
//! around `=`, and a value either single-quoted or ending at whitespace, with `\` escaping the
//! next character in both. There is no percent-decoding: unquoting is the one decode.
//!
//! A text is a DSN only when it names a connection key (`host`, `hostaddr`, `port`, `dbname`
//! or `user`, ASCII case ignored), so `a=b` is not taken for one. Keys fold ASCII case only:
//! `HOST` is `host`, `İSER` is not `user`.

use hacivat_proto::DbmsKind;
use secrecy::zeroize::Zeroizing;

use super::{Builder, Pair, plain};
use crate::url::FieldId;
use crate::url::matcher::MAX_LIST_ITEMS;
use crate::url::parse::ParseOutcome;

/// The keys that make a text a DSN.
const CONNECTION_KEYS: [&str; 5] = ["host", "hostaddr", "port", "dbname", "user"];

/// A libpq DSN.
pub(super) fn parse(text: &str) -> Option<ParseOutcome> {
    let pairs = tokens(text)?;
    if !pairs
        .iter()
        .any(|(k, _)| CONNECTION_KEYS.iter().any(|c| k.eq_ignore_ascii_case(c)))
    {
        return None;
    }
    let mut b = Builder::new(DbmsKind::Postgres, "default");
    for (k, v) in pairs {
        match k.to_ascii_lowercase().as_str() {
            "host" => b.host_list(&k, &plain(v)),
            "port" => b.port_list(&k, &plain(v)),
            "dbname" => b.field(&k, FieldId::Database, plain(v)),
            "user" => b.field(&k, FieldId::User, plain(v)),
            _ => b.param(&k, v),
        }
    }
    b.pg_socket();
    b.finish()
}

/// C `isspace`: space, tab, newline, vertical tab, form feed, carriage return.
fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r') || c == char::from(11) || c == char::from(12)
}

fn skip_space(s: &str, i: usize) -> usize {
    s[i..]
        .find(|c: char| !is_space(c))
        .map_or(s.len(), |off| i + off)
}

/// The pairs of `s`; `None` for a key without `=`, an empty key, an unterminated quote, a
/// dangling `\`, a control character, or more than `MAX_LIST_ITEMS` pairs.
fn tokens(s: &str) -> Option<Vec<Pair>> {
    let mut out = Vec::new();
    let mut i = skip_space(s, 0);
    while i < s.len() {
        if out.len() == MAX_LIST_ITEMS {
            return None;
        }
        let key_end = s[i..]
            .find(|c: char| is_space(c) || c == '=')
            .map_or(s.len(), |off| i + off);
        let key = &s[i..key_end];
        if key.is_empty() || key.chars().any(char::is_control) {
            return None;
        }
        let eq = skip_space(s, key_end);
        if !s[eq..].starts_with('=') {
            return None;
        }
        let (value, next) = value_at(s, skip_space(s, eq + 1))?;
        out.push((key.to_string(), value));
        i = skip_space(s, next);
    }
    Some(out)
}

/// The value starting at `start` and where the text after it starts. The end is found first, so
/// the value is built in one buffer of its final capacity (a password is never left behind in a
/// grown-out allocation) that is wiped on drop.
fn value_at(s: &str, start: usize) -> Option<(Zeroizing<String>, usize)> {
    let quoted = s[start..].starts_with('\'');
    let body = if quoted { start + 1 } else { start };
    let mut end = None;
    let mut chars = s[body..].char_indices();
    while let Some((j, c)) = chars.next() {
        if c == '\\' {
            chars.next()?;
        } else if (quoted && c == '\'') || (!quoted && is_space(c)) {
            end = Some(body + j);
            break;
        }
    }
    let end = match end {
        Some(e) => e,
        None if quoted => return None,
        None => s.len(),
    };
    let mut value = Zeroizing::new(String::with_capacity(end - body));
    let mut chars = s[body..end].chars();
    while let Some(c) = chars.next() {
        let c = if c == '\\' { chars.next()? } else { c };
        if c.is_control() {
            return None;
        }
        value.push(c);
    }
    Some((value, if quoted { end + 1 } else { end }))
}
