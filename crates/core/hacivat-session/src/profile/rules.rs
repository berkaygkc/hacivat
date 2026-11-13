//! Naming and environment rules (D-004, R22, R23, P7). Host and name matching folds ASCII case
//! only (`eq_ignore_ascii_case`, `to_ascii_lowercase`); other text is kept verbatim, never cased.

use hacivat_net::{AddressClass, classify_host};
use hacivat_proto::DbmsKind;

use super::{Profile, ProfileEndpoint};

/// Where a production hint was found (R22, P7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HintSource {
    Host,
    Name,
}

/// Host-name parts that only say "a database" and make poor names (the mock's `suggestName`).
const GENERIC_HOST_PARTS: [&str; 4] = ["db", "pg", "mysql", "sql"];

/// Whether the endpoint is on this machine: a loopback host (`localhost`, `*.localhost`,
/// `127.0.0.0/8`, `::1`, as `hacivat_net::classify_host` decides; an empty host means the
/// default local socket), a socket or a file.
pub fn is_local_endpoint(e: &ProfileEndpoint) -> bool {
    match e {
        ProfileEndpoint::Server { host, .. } => is_local_host(host),
        ProfileEndpoint::Socket { .. } | ProfileEndpoint::File { .. } => true,
    }
}

/// Whether `host` names this machine. One rule with the connect path (M2a final review M3):
/// `classify_host` without resolved addresses, so the dialog's automatic `local` tag, the
/// environment prompt and the plaintext rule never disagree.
fn is_local_host(host: &str) -> bool {
    let host = host.trim();
    host.is_empty() || classify_host(host, &[]) == AddressClass::Loopback
}

/// A name for an unnamed profile (the mock's `suggestName`):
/// - SQLite: the file name (`app.db`);
/// - a local host: `<database>-local` (`shop-local`, or `local-local` without a database);
/// - otherwise the first DNS label without generic parts (`db-prod-us` → `prod-us`), prefixed
///   with the database unless the database already starts with it (`orders-prod-us`, `orders`).
///
/// An empty host gives an empty name. Non-ASCII text is kept as typed.
pub fn suggest_name(p: &Profile) -> String {
    let host = match &p.endpoint {
        ProfileEndpoint::File { path, .. } => {
            let text = path.to_string_lossy();
            return text
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or_default()
                .to_string();
        }
        _ if p.engine == DbmsKind::Sqlite => return String::new(),
        ProfileEndpoint::Server { host, .. } => host.trim(),
        ProfileEndpoint::Socket { .. } => "",
    };
    if host.is_empty() && !matches!(p.endpoint, ProfileEndpoint::Socket { .. }) {
        return String::new();
    }
    let database = p.database.as_deref().filter(|d| !d.is_empty());
    if is_local_endpoint(&p.endpoint) {
        return format!("{}-local", database.unwrap_or("local"));
    }
    let label = host.split('.').next().unwrap_or_default();
    let kept: Vec<&str> = label
        .split('-')
        .filter(|part| {
            !GENERIC_HOST_PARTS
                .iter()
                .any(|g| part.eq_ignore_ascii_case(g))
        })
        .collect();
    let first = if kept.iter().all(|part| part.is_empty()) {
        label.to_string()
    } else {
        kept.join("-")
    };
    match database {
        None => first,
        Some(db) if starts_with_ignore_ascii_case(db, &first) => db.to_string(),
        Some(db) => format!("{db}-{first}"),
    }
}

/// `text` starts with `prefix`, ASCII case ignored (other characters compared exactly).
fn starts_with_ignore_ascii_case(text: &str, prefix: &str) -> bool {
    text.len() >= prefix.len()
        && text.is_char_boundary(prefix.len())
        && text[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// The production token in `host`, then `name`: whole tokens split on `[-._/:0-9]`, compared
/// with `eq_ignore_ascii_case` against `prod`, `prd`, `live` and `production` (reported as
/// `"prod"`); never a substring, so `delivery-db` gives `None` (R22, P7).
pub fn production_hint(host: &str, name: &str) -> Option<(&'static str, HintSource)> {
    hint_in(host)
        .map(|t| (t, HintSource::Host))
        .or_else(|| hint_in(name).map(|t| (t, HintSource::Name)))
}

/// The first production token of `text`.
fn hint_in(text: &str) -> Option<&'static str> {
    text.split(|c: char| matches!(c, '-' | '.' | '_' | '/' | ':') || c.is_ascii_digit())
        .find_map(|token| {
            if token.eq_ignore_ascii_case("production") || token.eq_ignore_ascii_case("prod") {
                Some("prod")
            } else if token.eq_ignore_ascii_case("prd") {
                Some("prd")
            } else if token.eq_ignore_ascii_case("live") {
                Some("live")
            } else {
                None
            }
        })
}

/// `name` made unique among `existing` by a numeric suffix: `orders` → `orders-2`, then
/// `orders-3` (names compare with ASCII case ignored).
pub fn duplicate_name(name: &str, existing: &[&str]) -> String {
    let taken = |candidate: &str| existing.iter().any(|e| e.eq_ignore_ascii_case(candidate));
    (2u32..)
        .map(|n| format!("{name}-{n}"))
        .find(|candidate| !taken(candidate))
        .unwrap_or_default()
}

/// Whether connecting should first ask for an environment tag: the profile has none and its
/// endpoint is not on this machine (a local one is tagged `local` without asking).
pub fn needs_environment_prompt(p: &Profile) -> bool {
    p.environment.is_none() && !is_local_endpoint(&p.endpoint)
}
