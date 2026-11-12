//! Native connection-text forms, one module each (HV-DRV-04), and what they share: the router,
//! the URI splitter, the query reader, strict percent-decoding, parameter mapping and the
//! outcome builder.
//!
//! Every value is decoded exactly once: here for the native forms, by the matcher for a
//! template match (whose values `from_template` takes as they are). `sanitize` (HV-DRV-05)
//! relies on that and rejects any `%XX` left in a value as a second encoding. Keys are
//! compared with ASCII case folding only.
//!
//! A password, from the user info or from a password-like parameter (`password`, `pass`, `pwd`,
//! `passwd`, any ASCII case, after decoding, so `pass%77ord` counts), goes to the secret, and an
//! `sslpassword` to the TLS key passphrase; neither ever lands in the fields or the extras. They
//! are held in wiped buffers until they are `SecretString`s.
//!
//! A text a native form claims (by its scheme, `jdbc:`, `file:`, `sqlite:`, `:memory:`, or as a
//! DSN or SQLite path it accepts) is final: if the form refuses it, no template is tried, so a
//! looser template can never swallow a refused URL's password into a field.

pub mod jdbc;
pub mod libpq_dsn;
pub mod mysql_uri;
pub mod pg_uri;
pub mod sqlite;

use std::collections::{BTreeMap, HashSet};
use std::mem::discriminant;

use hacivat_net::TlsMode;
use hacivat_proto::DbmsKind;
use percent_encoding::percent_decode_str;
use secrecy::zeroize::Zeroizing;
use secrecy::{ExposeSecret, SecretString};

use super::matcher::MAX_LIST_ITEMS;
use super::parse::{MappedParam, MappedTo, ParseOutcome, ParseWarning};
use super::render::is_secret_param;
use super::{FieldId, FieldMap};

/// Recognises `text` (trimmed, at most 64 KiB) as one of the native forms, in order: `jdbc:`
/// (stripped and re-routed), the server URIs, SQLite URIs and `:memory:`, the libpq DSN, and a
/// SQLite path. A prefix claims the text: its form's verdict is final.
pub(super) fn parse(text: &str) -> Claim {
    if let Some(rest) = strip_ci(text, "jdbc:") {
        return Claim::Claimed(jdbc::parse(rest));
    }
    if let Some(claimed) = server_uri(text) {
        return Claim::Claimed(claimed);
    }
    if let Some(rest) = strip_ci(text, "file:") {
        return Claim::Claimed(sqlite::file_uri(rest));
    }
    if let Some(rest) = strip_ci(text, "sqlite:") {
        return Claim::Claimed(sqlite::sqlite_scheme(rest));
    }
    if text == sqlite::MEMORY {
        return Claim::Claimed(sqlite::memory());
    }
    match libpq_dsn::parse(text).or_else(|| sqlite::plain_path(text)) {
        Some(o) => Claim::Claimed(Some(o)),
        None => Claim::Unclaimed,
    }
}

/// What the native recognizers made of a text.
pub(super) enum Claim {
    /// No native form claims the text; only then may the template fallback try it.
    Unclaimed,
    /// A native form claimed the text; `None` when it refused it. Final either way.
    Claimed(Option<ParseOutcome>),
}

/// A `postgres://`, `postgresql://`, `mysql://` or `mariadb://` URI: `None` when `text` has none
/// of these schemes, else what its parser made of it.
pub(super) fn server_uri(text: &str) -> Option<Option<ParseOutcome>> {
    if let Some(rest) = strip_ci(text, "postgresql://").or_else(|| strip_ci(text, "postgres://")) {
        return Some(pg_uri::parse(rest));
    }
    if let Some(rest) = strip_ci(text, "mysql://") {
        return Some(mysql_uri::parse(rest, DbmsKind::MySql));
    }
    if let Some(rest) = strip_ci(text, "mariadb://") {
        return Some(mysql_uri::parse(rest, DbmsKind::MariaDb));
    }
    None
}

/// `text` after `prefix`, compared ASCII-case-insensitively.
pub(in crate::url) fn strip_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

/// `raw` percent-decoded exactly once, in a buffer that is wiped on drop. `None` for a `%` not
/// followed by two hex digits, or for bytes that are not UTF-8.
pub(super) fn decode(raw: &str) -> Option<Zeroizing<String>> {
    let b = raw.as_bytes();
    let mut from = 0;
    while let Some(off) = b[from..].iter().position(|&c| c == b'%') {
        let at = from + off;
        let hex = |i: usize| b.get(i).is_some_and(u8::is_ascii_hexdigit);
        if !(hex(at + 1) && hex(at + 2)) {
            return None;
        }
        from = at + 3;
    }
    let text = percent_decode_str(raw).decode_utf8().ok()?;
    Some(Zeroizing::new(text.into_owned()))
}

/// [`decode`], for a value that is not a secret.
pub(super) fn decode_plain(raw: &str) -> Option<String> {
    decode(raw).map(plain)
}

/// The text of a value that is not a secret, moved out of its wiped buffer.
pub(super) fn plain(mut v: Zeroizing<String>) -> String {
    std::mem::take(&mut *v)
}

/// A decoded `key=value` pair; the value may be a secret.
pub(super) type Pair = (String, Zeroizing<String>);

/// A URI query: `key=value` pairs joined by `&`, each side decoded once; the value may hold `=`.
/// An empty query has no pairs. An empty pair, a pair without `=` or with an empty key, or more
/// than `MAX_LIST_ITEMS` pairs (the matcher's cap) is refused.
pub(super) fn query(q: &str) -> Option<Vec<Pair>> {
    let mut out = Vec::new();
    if q.is_empty() {
        return Some(out);
    }
    for part in q.split('&') {
        if out.len() == MAX_LIST_ITEMS {
            return None;
        }
        let (k, v) = part.split_once('=')?;
        if k.is_empty() {
            return None;
        }
        out.push((decode_plain(k)?, decode(v)?));
    }
    Some(out)
}

/// A server URI after `scheme://`, decoded.
pub(super) struct Uri {
    user: Option<String>,
    password: Option<Zeroizing<String>>,
    /// Each host with its port digits as typed (`05432` stays `05432`).
    pub(super) hosts: Vec<(String, Option<String>)>,
    /// The text after the first `/`, if there is one.
    path: Option<String>,
    pub(super) query: Vec<Pair>,
}

/// Splits `[user[:password]@]host[:port][,host[:port]…][/path][?query]`. A host may be a
/// bracketed IPv6 literal. The user info ends at the last `@` of the authority and the password
/// starts after its first `:`. Refused: a `#` (a fragment is not part of these URIs), control
/// characters, whitespace in the authority, a bad port or a bad escape.
pub(super) fn split_uri(rest: &str) -> Option<Uri> {
    if rest.contains('#') || rest.chars().any(char::is_control) {
        return None;
    }
    let (head, q) = rest.split_once('?').unwrap_or((rest, ""));
    let (authority, path) = match head.split_once('/') {
        Some((a, p)) => (a, Some(p)),
        None => (head, None),
    };
    // A raw `@` after the first `/` means the user info held an unencoded `/` (`u:12/34@h`):
    // the split would put part of the password in the path.
    if authority.chars().any(char::is_whitespace) || path.is_some_and(|p| p.contains('@')) {
        return None;
    }
    let (userinfo, hostspec) = match authority.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, authority),
    };
    let (user, password) = match userinfo.map(|u| u.split_once(':').ok_or(u)) {
        None => (None, None),
        Some(Ok((name, pw))) => (Some(decode_plain(name)?), Some(decode(pw)?)),
        Some(Err(name)) => (Some(decode_plain(name)?), None),
    };
    let mut hosts = Vec::new();
    for entry in hostspec.split(',') {
        if hosts.len() == MAX_LIST_ITEMS {
            return None;
        }
        hosts.push(host_port(entry)?);
    }
    let path = match path {
        Some(p) => Some(decode_plain(p)?),
        None => None,
    };
    Some(Uri {
        user,
        password,
        hosts,
        path,
        query: query(q)?,
    })
}

/// `host[:port]` or `[ipv6][:port]`; an empty port is no port.
fn host_port(entry: &str) -> Option<(String, Option<String>)> {
    let (host, port) = if let Some(inner) = entry.strip_prefix('[') {
        let (h, after) = inner.split_once(']')?;
        if h.is_empty() {
            return None;
        }
        let port = match after {
            "" => None,
            a => Some(a.strip_prefix(':')?),
        };
        (h, port)
    } else {
        if entry.contains(['[', ']']) {
            return None;
        }
        match entry.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (entry, None),
        }
    };
    let port = port.filter(|p| !p.is_empty());
    if port.is_some_and(|p| !valid_port(p)) {
        return None;
    }
    Some((decode_plain(host)?, port.map(str::to_string)))
}

/// One to five ASCII digits naming a port from 1 to 65535; a leading zero is allowed.
fn valid_port(p: &str) -> bool {
    (1..=5).contains(&p.len())
        && p.bytes().all(|b| b.is_ascii_digit())
        && p.parse::<u16>().is_ok_and(|n| n != 0)
}

/// The one secret-shape check (T7 fix rounds 1 to 3): whether `v`, as written or
/// percent-decoded once, holds `key=` for a password-like `key` (`password`, `sslpassword`,
/// `pass`, `pwd`, `passwd`; ASCII case ignored; the key itself also percent-decoded, up to 8
/// times). The shape of a query, a DSN or an ADO string swallowed into one value.
///
/// The key rule is an allowlist: at each `=`, skip back over any Unicode whitespace
/// (`char::is_whitespace`, so NBSP and U+3000 count) and closing quotes (`"`, `'`, `` ` ``),
/// then take the longest run of `[A-Za-z0-9_%-]` before it. Every other character ends the
/// key, so no list of separators can miss one (`'`, `{`, `|`, `:`, `/`, `+`, …). Linear in `v`:
/// neither backward scan crosses the previous `=`.
pub(super) fn carries_secret(v: &str) -> bool {
    names_secret_key(v)
        || (v.contains('%') && names_secret_key(&percent_decode_str(v).decode_utf8_lossy()))
}

fn names_secret_key(s: &str) -> bool {
    let key_byte = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'%' | b'-');
    let b = s.as_bytes();
    s.match_indices('=').any(|(eq, _)| {
        // `trim_end_matches` cuts on char boundaries; the key run is ASCII, so `start` is one too.
        let mut end = s[..eq]
            .trim_end_matches(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`'))
            .len();
        // A backslash right before a closing quote (`\"password\"=X`) is escaping, not key.
        if end > 0 && b[end - 1] == b'\\' && matches!(b[end], b'"' | b'\'' | b'`') {
            end -= 1;
        }
        let mut start = end;
        while start > 0 && key_byte(b[start - 1]) {
            start -= 1;
        }
        start < end && secret_key(&s[start..end])
    })
}

/// `key` names a secret, as written or after up to 8 percent-decodings.
fn secret_key(key: &str) -> bool {
    let mut k = key.to_string();
    for _ in 0..8 {
        if is_secret_param(&k) {
            return true;
        }
        let decoded = percent_decode_str(&k).decode_utf8_lossy().into_owned();
        if decoded == k {
            return false;
        }
        k = decoded;
    }
    false
}

/// A path that names another machine or the NT object namespace: it starts with two
/// separators (`//host`, `\\host`, `\\?\UNC\`, `\\.\`, or a mix), or with `\??\` (any
/// separators). Opening one on Windows can send the user's credentials (an NTLM hash) to a
/// host, so no pasted text may become one. Callers check decoded text.
pub(super) fn is_network_path(p: &str) -> bool {
    let sep = |c: Option<char>| matches!(c, Some('/' | '\\'));
    let mut c = p.chars();
    let (a, b, x, d) = (c.next(), c.next(), c.next(), c.next());
    (sep(a) && sep(b)) || (sep(a) && b == Some('?') && x == Some('?') && sep(d))
}

/// The outcome invariant, checked before `parse` returns (T7 fix rounds 2 and 3): no value the
/// outcome keeps in the clear (fields, hosts, extras as `key=value`, mapped values) carries a
/// password-like `key =`, and neither the file nor the socket is a network path. Whatever recognizer produced the
/// outcome, a secret is moved by a parser that owns its key or the text is refused; it is never
/// left inside another value.
pub(in crate::url) fn leaks_a_secret(o: &ParseOutcome) -> bool {
    // A file or a socket on another machine (round 3: sockets too; a MySQL `\\host\pipe\…` or
    // a PostgreSQL `//host/share` socket directory would reach out over SMB).
    let remote = |f: FieldId| {
        o.fields.values.get(&f).is_some_and(|p| {
            is_network_path(p) || is_network_path(&percent_decode_str(p).decode_utf8_lossy())
        })
    };
    remote(FieldId::File)
        || remote(FieldId::Socket)
        || o.fields.values.values().any(|v| carries_secret(v))
        || o.fields.hosts.iter().any(|(h, _)| carries_secret(h))
        || o.extra
            .iter()
            .any(|(k, v)| carries_secret(&format!("{k}={v}")))
        || o.mapped.iter().any(|m| carries_secret(&m.value))
}

/// The setting a parameter maps to, for `kind`; `folded` is the ASCII-lowercased key. A value
/// Hacivat has no setting for (`sslmode=allow`, `connect_timeout=soon`) maps to nothing and
/// stays an extra.
fn map_param(kind: DbmsKind, folded: &str, value: &str) -> Option<MappedTo> {
    if kind == DbmsKind::Sqlite {
        return (folded == "mode" && value == "ro").then_some(MappedTo::ReadOnly(true));
    }
    match folded {
        "sslmode" | "ssl-mode" => tls_mode(value).map(MappedTo::TlsMode),
        "usessl" => {
            if value.eq_ignore_ascii_case("true") {
                Some(MappedTo::TlsMode(TlsMode::Require))
            } else if value.eq_ignore_ascii_case("false") {
                Some(MappedTo::TlsMode(TlsMode::Disable))
            } else {
                None
            }
        }
        "application_name" => Some(MappedTo::ApplicationName),
        "connect_timeout" if value.bytes().all(|b| b.is_ascii_digit()) => {
            value.parse().ok().map(MappedTo::ConnectTimeout)
        }
        "options" if kind == DbmsKind::Postgres => Some(MappedTo::PgOptions),
        _ => None,
    }
}

/// A libpq `sslmode` or a MySQL `ssl-mode`/`sslMode` value (ASCII case and `_`/`-` ignored).
fn tls_mode(value: &str) -> Option<TlsMode> {
    let v = value.to_ascii_lowercase().replace('_', "-");
    Some(match v.as_str() {
        "prefer" | "preferred" => TlsMode::Auto,
        "disable" | "disabled" => TlsMode::Disable,
        "require" | "required" => TlsMode::Require,
        "verify-ca" => TlsMode::VerifyCa,
        "verify-full" | "verify-identity" => TlsMode::VerifyFull,
        _ => return None,
    })
}

/// Builds a [`ParseOutcome`] from what a form found, in order. A key seen twice (ASCII case
/// ignored; a field counts once whatever its key) is reported once as `DuplicateKey`: a field,
/// the password or a setting keeps the last value, an extra keeps both.
pub(super) struct Builder {
    kind: DbmsKind,
    conn_type: String,
    fields: BTreeMap<FieldId, String>,
    password: Option<SecretString>,
    tls_key_passphrase: Option<SecretString>,
    extra: Vec<(String, String)>,
    mapped: Vec<MappedParam>,
    warnings: Vec<ParseWarning>,
    seen: HashSet<String>,
    warned: HashSet<String>,
}

impl Builder {
    pub(super) fn new(kind: DbmsKind, conn_type: &str) -> Self {
        Builder {
            kind,
            conn_type: conn_type.to_string(),
            fields: BTreeMap::new(),
            password: None,
            tls_key_passphrase: None,
            extra: Vec::new(),
            mapped: Vec::new(),
            warnings: Vec::new(),
            seen: HashSet::new(),
            warned: HashSet::new(),
        }
    }

    /// Records the key `id`, shown as `shown` if it repeats.
    fn note(&mut self, id: &str, shown: &str) {
        let id = id.to_ascii_lowercase();
        if !self.seen.insert(id.clone()) && self.warned.insert(id) {
            self.warnings
                .push(ParseWarning::DuplicateKey(shown.to_string()));
        }
    }

    /// Sets `f` from the key `key`; an empty value clears it.
    pub(super) fn field(&mut self, key: &str, f: FieldId, value: String) {
        self.note(f.id(), key);
        if value.is_empty() {
            self.fields.remove(&f);
        } else {
            self.fields.insert(f, value);
        }
    }

    /// Sets `f` to a value the form implies (JDBC's `localhost`), which a key may override
    /// without a warning.
    pub(super) fn implied(&mut self, f: FieldId, value: &str) {
        self.fields.insert(f, value.to_string());
    }

    pub(super) fn clear(&mut self, f: FieldId) {
        self.fields.remove(&f);
    }

    pub(super) fn conn_type(&mut self, id: &str) {
        self.conn_type = id.to_string();
    }

    /// Sets the password from the key `key`; an empty one clears it.
    pub(super) fn password(&mut self, key: &str, value: SecretString) {
        self.note("password", key);
        self.password = (!value.expose_secret().is_empty()).then_some(value);
    }

    /// The first of `hosts` sets the host and the port; the others are dropped with a warning.
    pub(super) fn hosts(&mut self, key: &str, hosts: Vec<(String, Option<String>)>) {
        let dropped = hosts.len().saturating_sub(1);
        let Some((host, port)) = hosts.into_iter().next() else {
            return;
        };
        if dropped > 0 {
            self.warnings.push(ParseWarning::MultipleHosts {
                used: host.clone(),
                dropped,
            });
        }
        if !host.is_empty() {
            self.field(key, FieldId::Host, host);
        }
        if let Some(port) = port {
            self.field("port", FieldId::Port, port);
        }
    }

    /// A libpq `host` value, a comma list: the first host is used, the others dropped.
    pub(super) fn host_list(&mut self, key: &str, value: &str) {
        let mut hosts = value.split(',');
        let first = hosts.next().unwrap_or_default();
        let dropped = hosts.count();
        if dropped > 0 {
            self.warnings.push(ParseWarning::MultipleHosts {
                used: first.to_string(),
                dropped,
            });
        }
        self.field(key, FieldId::Host, first.to_string());
    }

    /// A libpq `port` value, a comma list (one port per host): the first is used.
    pub(super) fn port_list(&mut self, key: &str, value: &str) {
        let first = value.split(',').next().unwrap_or_default();
        self.field(key, FieldId::Port, first.to_string());
    }

    /// The user, password, first host and database of a URI; returns its query.
    pub(super) fn uri(&mut self, u: Uri) -> Vec<Pair> {
        if let Some(user) = u.user.filter(|s| !s.is_empty()) {
            self.field("user", FieldId::User, user);
        }
        if let Some(pw) = u.password {
            self.password("password", SecretString::from(pw.as_str()));
        }
        self.hosts("host", u.hosts);
        if let Some(db) = u.path.filter(|s| !s.is_empty()) {
            self.field("dbname", FieldId::Database, db);
        }
        u.query
    }

    /// PostgreSQL: a host that is a directory is a Unix socket (libpq).
    pub(super) fn pg_socket(&mut self) {
        if self
            .fields
            .get(&FieldId::Host)
            .is_some_and(|h| h.starts_with('/'))
            && let Some(dir) = self.fields.remove(&FieldId::Host)
        {
            self.fields.insert(FieldId::Socket, dir);
            self.conn_type("unix_socket");
        }
    }

    /// A parameter that is not a field of the form: a password moves to the secret, a known
    /// setting is mapped (PostgreSQL `options` is also kept as an extra), anything else is kept
    /// as an extra. `sslpassword` becomes the TLS key passphrase (the last one wins; an empty one
    /// clears it).
    pub(super) fn param(&mut self, key: &str, value: Zeroizing<String>) {
        if key.eq_ignore_ascii_case("sslpassword") {
            self.note(key, key);
            self.tls_key_passphrase =
                (!value.is_empty()).then(|| SecretString::from(value.as_str()));
            return;
        }
        if is_secret_param(key) {
            self.password(key, SecretString::from(value.as_str()));
            return;
        }
        let folded = key.to_ascii_lowercase();
        self.note(&folded, key);
        let value = plain(value);
        let Some(to) = map_param(self.kind, &folded, &value) else {
            self.extra.push((key.to_string(), value));
            return;
        };
        if to == MappedTo::PgOptions {
            self.extra.retain(|(k, _)| !k.eq_ignore_ascii_case(key));
            self.extra.push((key.to_string(), value.clone()));
        }
        let m = MappedParam {
            key: key.to_string(),
            value,
            to,
        };
        match self
            .mapped
            .iter_mut()
            .find(|x| discriminant(&x.to) == discriminant(&m.to))
        {
            Some(slot) => *slot = m,
            None => self.mapped.push(m),
        }
    }

    /// The outcome; `None` when the port is not a port.
    pub(super) fn finish(mut self) -> Option<ParseOutcome> {
        if self
            .fields
            .get(&FieldId::Port)
            .is_some_and(|p| !valid_port(p))
        {
            return None;
        }
        let mut secrets = Vec::new();
        if let Some(pw) = self.password.take() {
            secrets.push((FieldId::Password, pw));
            self.warnings.push(ParseWarning::PasswordMoved);
        }
        Some(ParseOutcome {
            kind: self.kind,
            conn_type: self.conn_type,
            fields: FieldMap {
                values: self.fields,
                hosts: Vec::new(),
            },
            extra: self.extra,
            secrets,
            tls_key_passphrase: self.tls_key_passphrase,
            mapped: self.mapped,
            warnings: self.warnings,
        })
    }
}

/// The outcome of a template match (the fallback recognizer): the matcher has decoded every
/// value once; extras go through the same password, mapping and duplicate rules.
///
/// Refused, as defence in depth behind the claim rule: a field value that holds a
/// password-like `key=` (a query or DSN tail swallowed by a loose placeholder), and a file that
/// is a network path.
pub(super) fn from_template(
    kind: DbmsKind,
    conn_type: &str,
    map: FieldMap,
    extra: Vec<(String, String)>,
    password: Option<SecretString>,
) -> Option<ParseOutcome> {
    if map.values.values().any(|v| carries_secret(v))
        || map
            .values
            .get(&FieldId::File)
            .is_some_and(|f| is_network_path(f))
    {
        return None;
    }
    let mut b = Builder::new(kind, conn_type);
    if let Some(pw) = password {
        b.password("password", pw);
    }
    for (f, v) in map.values {
        b.field(f.id(), f, v);
    }
    if map.hosts.len() > 1
        && let Some((used, _)) = map.hosts.first()
    {
        b.warnings.push(ParseWarning::MultipleHosts {
            used: used.clone(),
            dropped: map.hosts.len() - 1,
        });
    }
    for (k, v) in extra {
        b.param(&k, Zeroizing::new(v));
    }
    b.finish().filter(|o| !leaks_a_secret(o))
}
