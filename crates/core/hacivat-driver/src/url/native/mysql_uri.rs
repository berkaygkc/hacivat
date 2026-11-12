//! MySQL and MariaDB URIs (`mysql://`, `mariadb://`) (HV-DRV-04). A `socket` parameter on a
//! local host with no port makes a Unix-socket connection (the `localhost` is implied); with a
//! remote host or a port it is kept as an extra. The query key `user` sets the user.

use hacivat_proto::DbmsKind;

use super::{Builder, plain, split_uri};
use crate::url::FieldId;
use crate::url::parse::ParseOutcome;

/// What follows `mysql://` or `mariadb://`, for `kind`.
pub(super) fn parse(rest: &str, kind: DbmsKind) -> Option<ParseOutcome> {
    let u = split_uri(rest)?;
    let local = match u.hosts.as_slice() {
        [(host, None)] => host.is_empty() || host.eq_ignore_ascii_case("localhost"),
        _ => false,
    };
    let socket = local
        && u.query
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("socket") && !v.is_empty());
    let mut b = Builder::new(kind, if socket { "unix_socket" } else { "default" });
    let q = b.uri(u);
    if socket {
        b.clear(FieldId::Host);
    }
    for (k, v) in q {
        if k.eq_ignore_ascii_case("user") {
            b.field(&k, FieldId::User, plain(v));
        } else if socket && k.eq_ignore_ascii_case("socket") {
            b.field(&k, FieldId::Socket, plain(v));
        } else {
            b.param(&k, v);
        }
    }
    b.finish()
}
