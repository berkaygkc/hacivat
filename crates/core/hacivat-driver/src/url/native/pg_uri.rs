//! PostgreSQL URIs (`postgres://`, `postgresql://`) and the JDBC short form
//! (`jdbc:postgresql:dbname`) (HV-DRV-04), as libpq reads them: a host list (the first host is
//! used), a host that is a directory is a Unix socket, and the query keys `host`, `port`,
//! `dbname` and `user` override the parts before the `?`.

use hacivat_proto::DbmsKind;

use super::{Builder, Pair, decode_plain, plain, query, split_uri};
use crate::url::FieldId;
use crate::url::parse::ParseOutcome;

/// What follows `postgres://` or `postgresql://`.
pub(super) fn parse(rest: &str) -> Option<ParseOutcome> {
    let u = split_uri(rest)?;
    let mut b = Builder::new(DbmsKind::Postgres, "default");
    let q = b.uri(u);
    params(&mut b, q);
    b.pg_socket();
    b.finish()
}

/// What follows `jdbc:postgresql:` when no `//` does: `dbname[?query]`, on `localhost`.
pub(super) fn parse_short(rest: &str) -> Option<ParseOutcome> {
    if rest.contains('#') || rest.chars().any(char::is_control) {
        return None;
    }
    let (db, q) = rest.split_once('?').unwrap_or((rest, ""));
    let mut b = Builder::new(DbmsKind::Postgres, "default");
    b.implied(FieldId::Host, "localhost");
    let db = decode_plain(db)?;
    if !db.is_empty() {
        b.field("dbname", FieldId::Database, db);
    }
    params(&mut b, query(q)?);
    b.pg_socket();
    b.finish()
}

/// The query of a PostgreSQL URI: libpq connection keys become fields, the rest parameters.
fn params(b: &mut Builder, q: Vec<Pair>) {
    for (k, v) in q {
        match k.to_ascii_lowercase().as_str() {
            "host" => b.host_list(&k, &plain(v)),
            "port" => b.port_list(&k, &plain(v)),
            "dbname" => b.field(&k, FieldId::Database, plain(v)),
            "user" => b.field(&k, FieldId::User, plain(v)),
            _ => b.param(&k, v),
        }
    }
}
