//! JDBC URLs (`jdbc:postgresql://...`, `jdbc:postgresql:dbname`, `jdbc:mysql://...`,
//! `jdbc:mariadb://...`, `jdbc:sqlite:...`) (HV-DRV-04): the `jdbc:` prefix is stripped and the
//! rest is routed to the same parsers as a pasted URI. Only these engines are taken.

use super::{pg_uri, server_uri, sqlite, strip_ci};
use crate::url::parse::ParseOutcome;

/// What follows `jdbc:`.
pub(super) fn parse(rest: &str) -> Option<ParseOutcome> {
    if let Some(claimed) = server_uri(rest) {
        return claimed;
    }
    if let Some(r) = strip_ci(rest, "postgresql:") {
        return pg_uri::parse_short(r);
    }
    if let Some(r) = strip_ci(rest, "sqlite:") {
        return sqlite::sqlite_scheme(r);
    }
    None
}
