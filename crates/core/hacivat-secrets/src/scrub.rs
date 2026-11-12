//! Scrubbing: removes passwords from URLs and option lists before they reach a log, the
//! clipboard or a file (HV-SEC-05).
//!
//! A key is secret when its ASCII-lowercased form contains `pass`, `pwd`, `secret`, `token` or
//! `key`. Matching is locale-invariant and errs on the side of removing too much.

/// The key fragments that mark a secret-bearing option.
const SECRET_FRAGMENTS: [&str; 5] = ["pass", "pwd", "secret", "token", "key"];

/// What a scrubbed option value becomes in [`scrub_options`].
const REMOVED: &str = "<removed>";

/// Whether an option key names a secret (`(?i)pass|pwd|secret|token|key`, ASCII only).
fn is_secret_key(key: &str) -> bool {
    let key = percent_decode_ascii(key.trim()).to_ascii_lowercase();
    SECRET_FRAGMENTS.iter().any(|f| key.contains(f))
}

/// `url` without its password, wherever it appears:
/// - a URL's userinfo password (`postgres://u:p@h/db` → `postgres://u@h/db`);
/// - secret query parameters (`?user=u&password=p` → `?user=u`) and secret `;key=value`
///   parameters after the authority (`jdbc:sqlserver://h;password=p`);
/// - scheme-less userinfo (`root:pw@localhost:3306/db`, the Go DSN `u:pw@tcp(h)/db`,
///   `//u:p@h/db`);
/// - libpq keyword/value strings (`host=h password='a b'` → `host=h`);
/// - semicolon connection strings (`Server=h;Password=x;` → `Server=h;`).
///
/// Other text is returned unchanged.
pub fn scrub_url(url: &str) -> String {
    // A URL only when the text before `://` is a scheme (`jdbc:postgresql` included), so a
    // `sslcrl=http://c` value inside a keyword string does not make the string a URL.
    if let Some(scheme_end) = url.find("://")
        && is_scheme(&url[..scheme_end])
    {
        let (scheme, rest) = url.split_at(scheme_end + 3);
        return format!("{scheme}{}", scrub_after_scheme(rest));
    }
    if let Some(rest) = bare_userinfo(url) {
        let prefix = &url[..url.len() - rest.len()];
        return format!("{prefix}{}", scrub_after_scheme(rest));
    }
    if !url.contains('=') {
        return url.to_string();
    }
    // Both shapes in turn; removal only shrinks the string. The semicolon pass goes first, so
    // `Password=x;Url=h` loses only its first entry; the libpq pass then drops a pair whose
    // unquoted value held a `;` (`password=a;b`). A `;` in a key means it is not libpq.
    let semicolon_scrubbed = scrub_semicolon_list(url);
    match keyword_pairs(&semicolon_scrubbed) {
        Some(pairs) if !pairs.iter().any(|p| p.key.contains(';')) => pairs
            .iter()
            .filter(|p| !is_secret_key(p.key))
            .map(|p| p.text)
            .collect::<Vec<_>>()
            .join(" "),
        _ => semicolon_scrubbed,
    }
}

/// Whether `s` is a URL scheme: `^[A-Za-z][A-Za-z0-9+.\-:]*$` (`:` for `jdbc:postgresql`).
fn is_scheme(s: &str) -> bool {
    let mut bytes = s.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_alphabetic())
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-' | b':'))
}

/// For scheme-less userinfo (`^(//)?[^\s/@=\\]*:\S*@`), the text after the optional `//`.
///
/// A `\` before the colon, or a drive letter followed by `:\`, is a Windows path
/// (`C:\dir\a@b`, `\\host\share:x@y`), not credentials.
fn bare_userinfo(s: &str) -> Option<&str> {
    let rest = s.strip_prefix("//").unwrap_or(s);
    let colon = rest.find(':')?;
    if rest[..colon]
        .bytes()
        .any(|b| b.is_ascii_whitespace() || matches!(b, b'/' | b'@' | b'=' | b'\\'))
    {
        return None;
    }
    if colon == 1 && rest.as_bytes()[0].is_ascii_alphabetic() && rest[2..].starts_with('\\') {
        return None;
    }
    let after_colon = &rest[colon..];
    let token = after_colon
        .split(|c: char| c.is_ascii_whitespace())
        .next()
        .unwrap_or("");
    token.contains('@').then_some(rest)
}

/// `opts` with the value of every secret-bearing key replaced by `<removed>`; the keys and
/// their order stay.
pub fn scrub_options(opts: &[(String, String)]) -> Vec<(String, String)> {
    opts.iter()
        .map(|(k, v)| {
            let value = if is_secret_key(k) {
                REMOVED.to_string()
            } else {
                v.clone()
            };
            (k.clone(), value)
        })
        .collect()
}

/// Scrubs what follows `scheme://`: the userinfo password, `;` parameters and the query.
fn scrub_after_scheme(rest: &str) -> String {
    let rest = without_userinfo_password(rest);
    let rest = rest.as_str();
    let (before_fragment, fragment) = match rest.find('#') {
        Some(i) => rest.split_at(i),
        None => (rest, ""),
    };
    let (before_query, query) = match before_fragment.find('?') {
        Some(i) => (&before_fragment[..i], Some(&before_fragment[i + 1..])),
        None => (before_fragment, None),
    };

    let mut out = before_query.to_string();
    if let Some(semi) = out.find(';') {
        let tail = scrub_semicolon_list(&out[semi + 1..]);
        out.truncate(semi + 1);
        out.push_str(&tail);
        if out.ends_with(';') && tail.is_empty() {
            out.pop();
        }
    }
    if let Some(query) = query {
        let kept: Vec<&str> = query
            .split('&')
            .filter(|param| {
                let key = param.split_once('=').map_or(*param, |(k, _)| k);
                !param.is_empty() && !is_secret_key(key)
            })
            .collect();
        if !kept.is_empty() {
            out.push('?');
            out.push_str(&kept.join("&"));
        }
    }
    out.push_str(fragment);
    out
}

/// `u:p@h/db` → `u@h/db`, for the text after `scheme://`.
///
/// The authority ends at the first `/`, `?` or `#`; its userinfo ends at the authority's last
/// `@`, so an unescaped `@` in the password is removed too.
///
/// A raw `/`, `?` or `#` in a password ends the authority early (`u:p#ss@h`, `u:12/34@h`).
/// When the authority has no `@`, does not start with `[` (IPv6) and has a `:`, the userinfo
/// runs to the next `@` if no `=` or `&` comes before that `@` (it would be a query, as in
/// `?user=u@srv`) and one of these holds:
/// - the text after the last `:` is not a numeric port;
/// - the authority ends at `#`;
/// - no further `/` comes before that `@`.
fn without_userinfo_password(rest: &str) -> String {
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if let Some(at) = authority.rfind('@') {
        if let Some(colon) = authority[..at].find(':') {
            return format!("{}{}", &rest[..colon], &rest[at..]);
        }
        return rest.to_string();
    }
    if authority.starts_with('[') {
        return rest.to_string();
    }
    if let (Some(colon), Some(last_colon)) = (authority.find(':'), authority.rfind(':'))
        && let Some(at) = rest[authority_end..].find('@')
    {
        let at = authority_end + at;
        let before_at = &rest[authority_end..at];
        let port = &authority[last_colon + 1..];
        let is_port = !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit());
        let ends_at_hash = rest[authority_end..].starts_with('#');
        let no_further_slash = !before_at.get(1..).unwrap_or("").contains('/');
        let not_a_query = !before_at.contains(['=', '&']);
        if not_a_query && (!is_port || ends_at_hash || no_further_slash) {
            return format!("{}{}", &rest[..colon], &rest[at..]);
        }
    }
    rest.to_string()
}

/// `Server=h;Password=x;` → `Server=h;`. Values may be braced (`{a;b}`, ODBC) or quoted
/// (`'a;b'`, `"a;b"`), and their `;` does not split.
fn scrub_semicolon_list(s: &str) -> String {
    split_semicolons(s)
        .into_iter()
        .filter(|segment| {
            let key = segment.split_once('=').map_or("", |(k, _)| k);
            key.is_empty() || !is_secret_key(key)
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// Splits on `;` outside braces and quotes. A segment with `=` keeps any trailing empty
/// segment, so `a=1;` stays `["a=1", ""]`.
fn split_semicolons(s: &str) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut segments = Vec::new();
    let mut start = 0;
    let mut i = 0;
    let mut at_value_start = false;
    while i < bytes.len() {
        match bytes[i] {
            b';' => {
                segments.push(&s[start..i]);
                start = i + 1;
                at_value_start = false;
            }
            b'=' => at_value_start = true,
            open @ (b'{' | b'\'' | b'"') if at_value_start => {
                let close = if open == b'{' { b'}' } else { open };
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == close {
                        // A doubled closer is an escaped one.
                        if bytes.get(i + 1) == Some(&close) {
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    i += 1;
                }
                at_value_start = false;
            }
            b' ' | b'\t' => {}
            _ => at_value_start = false,
        }
        i += 1;
    }
    segments.push(&s[start.min(s.len())..]);
    segments
}

/// One libpq `key = value` pair.
struct Pair<'a> {
    key: &'a str,
    /// The whole pair as written.
    text: &'a str,
}

/// Parses a libpq keyword/value string; `None` when it is not one (a key without `=`).
fn keyword_pairs(s: &str) -> Option<Vec<Pair<'_>>> {
    let bytes = s.as_bytes();
    let mut pairs = Vec::new();
    let mut i = 0;
    let skip_ws = |mut i: usize| {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    };
    loop {
        i = skip_ws(i);
        if i >= bytes.len() {
            return Some(pairs);
        }
        let start = i;
        while i < bytes.len() && bytes[i] != b'=' && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let key = &s[start..i];
        i = skip_ws(i);
        if key.is_empty() || bytes.get(i) != Some(&b'=') {
            return None;
        }
        i = skip_ws(i + 1);
        if bytes.get(i) == Some(&b'\'') {
            i += 1;
            while i < bytes.len() && bytes[i] != b'\'' {
                i += if bytes[i] == b'\\' { 2 } else { 1 };
            }
            i = (i + 1).min(bytes.len());
        } else {
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += if bytes[i] == b'\\' { 2 } else { 1 };
            }
            i = i.min(bytes.len());
        }
        pairs.push(Pair {
            key,
            text: &s[start..i],
        });
    }
}

/// Decodes `%XX` escapes of ASCII bytes, so `pass%77ord` is matched; other text is kept.
fn percent_decode_ascii(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = s.get(i + 1..i + 3)
            && let Ok(b) = u8::from_str_radix(hex, 16)
            && b.is_ascii()
        {
            out.push(char::from(b));
            i += 3;
            continue;
        }
        let ch = s[i..].chars().next().unwrap_or_default();
        out.push(ch);
        i += ch.len_utf8().max(1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harder_shapes_never_keep_the_password() {
        // Failures name the case number only, never the input or output.
        let cases = [
            (
                "postgres://u:p%40ss@h/db?sslmode=require",
                "postgres://u@h/db?sslmode=require",
            ),
            ("postgres://u:a/b@h/db", "postgres://u@h/db"),
            ("postgres://u:p#ss@h:5432/db", "postgres://u@h:5432/db"),
            ("postgres://u:a?b@h/db", "postgres://u@h/db"),
            ("postgres://user@srv:pw@h/db", "postgres://user@srv@h/db"),
            ("postgres://u@h/db", "postgres://u@h/db"),
            ("postgres://[::1]:5432/db", "postgres://[::1]:5432/db"),
            (
                "postgres://h:5432/db?user=u@srv&password=p",
                "postgres://h:5432/db?user=u@srv",
            ),
            (
                "postgres://h/db?Pass%77ord=p&x=1#f",
                "postgres://h/db?x=1#f",
            ),
            (
                "jdbc:sqlserver://h:1433;user=u;password=p",
                "jdbc:sqlserver://h:1433;user=u",
            ),
            ("host=h password = 'a\\' b' user=u", "host=h user=u"),
            ("host=h password='a;b' user=u", "host=h user=u"),
            ("Server=my host;Password=x", "Server=my host"),
            ("Driver={X};PWD={a;b}};c};Server=h", "Driver={X};Server=h"),
            ("Data Source=h;Password='a;b';", "Data Source=h;"),
            ("no secrets here", "no secrets here"),
        ];
        for (n, (input, want)) in cases.into_iter().enumerate() {
            assert!(scrub_url(input) == want, "scrub case {n} failed");
        }
    }

    #[test]
    fn review_round_1_vectors() {
        // Failures name the case number only, never the input or output.
        let cases = [
            // 1. `://` inside a value does not make a keyword or semicolon string a URL.
            (
                "host=h password=x sslcrl=http://c",
                "host=h sslcrl=http://c",
            ),
            ("Password=x;Url=http://h", "Url=http://h"),
            (
                "jdbc:postgresql://h/db?password=p",
                "jdbc:postgresql://h/db",
            ),
            // 2. An unquoted `;` in a libpq password value.
            ("host=h password=a;b", "host=h"),
            ("host=h password=a;b user=u", "host=h user=u"),
            // 3. Scheme-less userinfo.
            ("root:pw@localhost:3306/db", "root@localhost:3306/db"),
            ("u:pw@tcp(h)/db", "u@tcp(h)/db"),
            ("//u:p@h/db", "//u@h/db"),
            (
                "u:pw@tcp(h)/db?parseTime=true",
                "u@tcp(h)/db?parseTime=true",
            ),
            ("note: see a@b", "note: see a@b"),
            // Windows paths are not userinfo.
            ("C:\\dir\\a@b", "C:\\dir\\a@b"),
            ("\\\\host\\share:x@y", "\\\\host\\share:x@y"),
            // 6. A numeric password with `#`, `/` or `?`.
            ("postgres://u:12#34@h/db", "postgres://u@h/db"),
            ("postgres://u:12/34@h/db", "postgres://u@h/db"),
            ("postgres://u:12?34@h/db", "postgres://u@h/db"),
            // 7. IPv6 hosts are never taken for userinfo.
            (
                "postgres://[::1]/db?user=u@srv",
                "postgres://[::1]/db?user=u@srv",
            ),
            (
                "postgres://[::1]:5432/db?user=u@srv",
                "postgres://[::1]:5432/db?user=u@srv",
            ),
        ];
        for (n, (input, want)) in cases.into_iter().enumerate() {
            assert!(scrub_url(input) == want, "round-1 scrub case {n} failed");
        }
    }

    #[test]
    fn never_panics_on_short_punctuation_strings() {
        // Every string of up to four of these characters, alone and after `x://`; slicing on
        // multi-byte `İ` and `%` at the end are the risky cases.
        let alphabet = [
            ':', '@', '/', '?', '#', '=', ';', '\'', '{', '}', ' ', '[', '%', 'İ', 'a',
            // 2-byte (`ş`), 3-byte (`€`) and a backslash.
            'ş', '€', '\\',
        ];
        let mut s = String::new();
        let mut checked = 0;
        for n in 0..alphabet.len().pow(4) {
            s.clear();
            let mut k = n;
            for _ in 0..4 {
                s.push(alphabet[k % alphabet.len()]);
                k /= alphabet.len();
            }
            for input in [s.as_str(), &format!("x://{s}")] {
                let out = scrub_url(input);
                assert!(out.len() <= input.len(), "scrubbing never grows the text");
                checked += 1;
            }
        }
        assert_eq!(checked, 2 * alphabet.len().pow(4));
    }

    #[test]
    fn never_panics_on_longer_multibyte_strings() {
        // A deterministic LCG picks 5 to 24 characters from an alphabet with 1-, 2-, 3- and
        // 4-byte characters, alone and after `x://`; 20 000 strings.
        let alphabet = [
            ':',
            '@',
            '/',
            '?',
            '#',
            '=',
            ';',
            '\'',
            '{',
            '}',
            ' ',
            '[',
            '%',
            '\\',
            'a',
            'ş',
            'İ',
            '€',
            '\u{1F600}',
        ];
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as usize
        };
        for _ in 0..20_000 {
            let len = 5 + next() % 20;
            let s: String = (0..len)
                .map(|_| alphabet[next() % alphabet.len()])
                .collect();
            for input in [s.clone(), format!("x://{s}")] {
                let out = scrub_url(&input);
                // The keyword pass re-joins pairs with one space, so a string whose quoted
                // value runs straight into the next key (`k='v'k2=w`) may gain a byte per pair
                // (a pair is at least three bytes); it never grows by more than that.
                assert!(
                    out.len() <= input.len() + input.len() / 3,
                    "scrubbing grew the text: {input:?} -> {out:?}"
                );
            }
        }
    }

    #[test]
    fn secret_keys_are_matched_ascii_insensitively() {
        for k in [
            "PASSWORD",
            "sslpassword",
            "PWD",
            "client_secret",
            "access_token",
            "sslkey",
        ] {
            assert!(is_secret_key(k), "{k}");
        }
        for k in ["host", "sslmode", "user", "dbname"] {
            assert!(!is_secret_key(k), "{k}");
        }
    }
}
