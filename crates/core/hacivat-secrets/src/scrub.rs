//! Scrubbing: removes passwords from URLs and option lists before they reach a log, the
//! clipboard or a file (HV-SEC-05).
//!
//! A key is secret when `hacivat_proto::is_secret_param` (the one password-key rule the parser,
//! the import, the dialog and the store share) says so, or when its ASCII-lowercased form
//! contains `pass`, `pwd`, `secret`, `token` or `key`. The fragments are a deliberate superset:
//! a scrub errs on the side of removing too much. Matching is locale-invariant.
//!
//! Callers: the connect error's technical details and the copied reports scrub the server's
//! message (`hacivat-app` `connections::report`), so a server that echoes a connection string
//! never puts its password on the screen or the clipboard.

/// The key fragments that mark a secret-bearing option.
const SECRET_FRAGMENTS: [&str; 5] = ["pass", "pwd", "secret", "token", "key"];

/// What a scrubbed option value becomes in [`scrub_options`].
const REMOVED: &str = "<removed>";

/// Whether an option key names a secret: the shared password-key rule, or a secret fragment
/// (`(?i)pass|pwd|secret|token|key`, ASCII only).
fn is_secret_key(key: &str) -> bool {
    let key = percent_decode_ascii(key.trim());
    if hacivat_proto::is_secret_param(&key) {
        return true;
    }
    let key = key.to_ascii_lowercase();
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

/// For scheme-less userinfo (`^(//)?[^\s/@=]*:\S*@`), the text after the optional `//`.
///
/// A drive letter followed by `:\`, or text starting with `\\` (a UNC path), is a Windows path
/// (`C:\dir\a@b`, `\\host\share:x@y`), not credentials. Any other `\` is allowed in the user
/// (`DOMAIN\user:pw@h` is scrubbed).
fn bare_userinfo(s: &str) -> Option<&str> {
    let rest = s.strip_prefix("//").unwrap_or(s);
    let colon = rest.find(':')?;
    if rest[..colon]
        .bytes()
        .any(|b| b.is_ascii_whitespace() || matches!(b, b'/' | b'@' | b'='))
        || rest.starts_with("\\\\")
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

// ## m2a-finalfix (round 2, N4)
/// A server's message without a password: [`scrub_url`], unless that would leave nothing of a
/// message that had text. Free text is no connection string, and `scrub_url` can read a whole
/// sentence as one `key=value` segment (`duplicate key value … Key (id)=(1) already exists.`)
/// and drop it. Then only the values of secret keys (the scrubber's own rule: a password key,
/// or a `pass`/`pwd`/`secret`/`token`/`key` fragment) are removed (`password=<removed>`); every
/// other word, and the whitespace, stays as the server sent it. A value is found:
/// - in a whitespace-separated word, also after `;` (`user=u;password=<removed>`, R2-2);
/// - after a spaced `=` (`password = <removed>`, `password =<removed>`, `password= <removed>`);
/// - over the following words of a quoted value with spaces (`password='a b'`).
pub fn scrub_message(text: &str) -> String {
    let scrubbed = scrub_url(text);
    if !scrubbed.trim().is_empty() || text.trim().is_empty() {
        return scrubbed;
    }
    let mut out = String::with_capacity(text.len());
    let mut open = Open::Nothing;
    for word in text.split_inclusive(char::is_whitespace) {
        let body = word.trim_end_matches(char::is_whitespace);
        let space = &word[body.len()..];
        if body.is_empty() {
            out.push_str(word);
            continue;
        }
        open = match open {
            // The rest of a quoted value goes, with the space after it.
            Open::Quote(q) => {
                if body.ends_with(q) {
                    out.push_str(space);
                    Open::Nothing
                } else {
                    Open::Quote(q)
                }
            }
            Open::Value => {
                let next = removed_value(body, &mut out);
                if next == Open::Nothing {
                    out.push_str(space);
                }
                next
            }
            Open::Equals if body == "=" => {
                out.push_str(word);
                Open::Value
            }
            Open::Equals if body.starts_with('=') => {
                out.push('=');
                let next = removed_value(&body[1..], &mut out);
                if next == Open::Nothing {
                    out.push_str(space);
                }
                next
            }
            Open::Nothing | Open::Equals => {
                let next = scrub_word(body, &mut out);
                if !matches!(next, Open::Quote(_)) {
                    out.push_str(space);
                }
                next
            }
        };
    }
    out
}

/// What a word of [`scrub_message`]'s fallback leaves open for the next one.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Open {
    Nothing,
    /// A secret key ended the word: a `=` may follow.
    Equals,
    /// A secret key's `=` ended the word: its value follows.
    Value,
    /// A quoted secret value goes on until a word ends with this quote.
    Quote(char),
}

/// Writes `<removed>` for the secret value `value`; a quoted value that does not close in it
/// stays open.
fn removed_value(value: &str, out: &mut String) -> Open {
    out.push_str(REMOVED);
    match value.chars().next() {
        Some(q @ ('\'' | '"')) if value.len() == 1 || !value.ends_with(q) => Open::Quote(q),
        _ => Open::Nothing,
    }
}

/// Writes `body` (a whitespace-free word) with the value of each secret `key=value` among its
/// `;`-separated parts removed, and says what the word leaves open.
fn scrub_word(body: &str, out: &mut String) -> Open {
    let parts: Vec<&str> = body.split_inclusive(';').collect();
    let last = parts.len() - 1;
    for (i, part) in parts.into_iter().enumerate() {
        let (part, semi) = match part.strip_suffix(';') {
            Some(p) => (p, ";"),
            None => (part, ""),
        };
        let open_end = i == last && semi.is_empty();
        match part.split_once('=') {
            Some((key, value)) if !key_of(key).is_empty() && is_secret_key(key_of(key)) => {
                out.push_str(key);
                out.push('=');
                if value.is_empty() && open_end {
                    return Open::Value;
                }
                let open = removed_value(value, out);
                out.push_str(semi);
                if open_end {
                    return open;
                }
            }
            None if open_end && !key_of(part).is_empty() && is_secret_key(key_of(part)) => {
                out.push_str(part);
                return Open::Equals;
            }
            _ => {
                out.push_str(part);
                out.push_str(semi);
            }
        }
    }
    Open::Nothing
}

/// The option name at the end of `s` (`(password` → `password`, `"pwd` → `pwd`); closing
/// quotes after it are skipped (`"password"` → `password`; fix round 1, M12).
fn key_of(s: &str) -> &str {
    let s = s.trim_end_matches(['"', '\'', '`']);
    let start = s
        .char_indices()
        .rev()
        .find(|(_, c)| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')))
        .map_or(0, |(i, c)| i + c.len_utf8());
    &s[start..]
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
            // A domain user is userinfo; Windows paths are not.
            ("DOMAIN\\user:pw@h", "DOMAIN\\user@h"),
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

    // ## m2a-finalfix (round 2, N4)
    #[test]
    fn a_server_message_is_never_blanked_and_still_loses_its_password() {
        let duplicate = "duplicate key value violates unique constraint \"t_pkey\" \
                         DETAIL: Key (id)=(1) already exists.";
        assert_eq!(scrub_url(duplicate), "", "scrub_url alone blanks it");
        assert_eq!(scrub_message(duplicate), duplicate);
        assert_eq!(
            scrub_message("could not connect: key=value password=s3cret\n(pwd=x2) token=t user=u"),
            "could not connect: key=<removed> password=<removed>\n(pwd=<removed> token=<removed> user=u"
        );
        // What `scrub_url` handles keeps its result.
        assert_eq!(
            scrub_message("host=db user=u password=s3cret"),
            "host=db user=u"
        );
        assert_eq!(scrub_message("plain text"), "plain text");
        assert_eq!(scrub_message(""), "");
    }

    /// M2a re-review R2-2: the word-by-word fallback also hides a password after `;` in one
    /// word, around a spaced `=`, and in a quoted value with spaces.
    #[test]
    fn the_fallback_hides_semicolon_spaced_and_quoted_passwords() {
        let lead = "duplicate key value violates unique constraint \"t_pkey\" DETAIL: Key (id)=(1)";
        for (tail, want) in [
            ("user=u;password=s3cret", "user=u;password=<removed>"),
            ("password = s3cret user=u", "password = <removed> user=u"),
            ("password =s3cret user=u", "password =<removed> user=u"),
            ("password= s3cret user=u", "password= <removed> user=u"),
            ("password='s3 cret' user=u", "password=<removed> user=u"),
            ("pwd = \"s3 cr et\" user=u", "pwd = <removed> user=u"),
        ] {
            let message = format!("{lead} {tail}");
            assert_eq!(scrub_url(&message), "", "the fallback runs for {tail:?}");
            let scrubbed = scrub_message(&message);
            assert_eq!(scrubbed, format!("{lead} {want}"), "{tail:?}");
            assert!(!scrubbed.contains("s3"), "{scrubbed}");
        }
        // M12 (fix round 1): a quoted key loses its value too.
        for (tail, want) in [
            (
                "\"password\"=\"s3cret\" user=u",
                "\"password\"=<removed> user=u",
            ),
            ("'pwd'='s3cret' user=u", "'pwd'=<removed> user=u"),
        ] {
            let message = format!("{lead} {tail}");
            assert_eq!(scrub_url(&message), "", "the fallback runs for {tail:?}");
            assert_eq!(
                scrub_message(&message),
                format!("{lead} {want}"),
                "{tail:?}"
            );
        }
        // A secret-looking word without `=` after it stays (`key` here is prose).
        let prose = format!("{lead} the key is fine");
        assert_eq!(scrub_message(&prose), prose);
    }

    // ## m2a-finalfix
    /// Every key the shared rule calls a password is scrubbed (the scrub is a superset).
    #[test]
    fn every_shared_password_key_is_scrubbed() {
        for k in [
            "password",
            "--password",
            "pgpassword",
            "mysql_pwd",
            "sslkeypassword",
            "db_password",
            "x-pwd",
        ] {
            assert!(hacivat_proto::is_secret_param(k), "{k}");
            assert!(is_secret_key(k), "{k}");
        }
    }
}
