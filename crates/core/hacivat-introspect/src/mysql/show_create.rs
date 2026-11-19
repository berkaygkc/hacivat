// ## m4-t17-mysql
//! Server text (R-M4-24, R-M4-37): the `SHOW CREATE TABLE|VIEW|PROCEDURE|FUNCTION|TRIGGER|
//! EVENT` statement of one object, through the pack's `show-create` sections. The schema and
//! the name are the identifier macro, rendered only by `sql_ident::quote` (backticks doubled,
//! NUL refused, Task 4); the server's own statement comes back verbatim. The connection must
//! read `utf8mb4` on every character set (the T4 carry: under GBK a doubled backtick can be
//! swallowed by a lead byte), else nothing is sent.

use hacivat_meta::{Level, Name, ObjectKind};
use hacivat_session::meta::MetaError;

use super::pass::{SectionReader, text};
use super::rows::{self, opt_name};
use super::server::ServerCtx;
use crate::introspector::IntroError;
use crate::pack::{Mode, Params};

/// The `show-create` section of `kind`, its width and the position of the statement column;
/// `None` for a kind MySQL has no `SHOW CREATE` for.
pub fn section_of(kind: ObjectKind) -> Option<(&'static str, usize, usize)> {
    Some(match kind {
        ObjectKind::Table | ObjectKind::Sequence => ("ShowCreateTable", 2, 1),
        ObjectKind::View => ("ShowCreateView", 4, 1),
        ObjectKind::Procedure => ("ShowCreateProcedure", 6, 2),
        ObjectKind::Routine => ("ShowCreateFunction", 6, 2),
        ObjectKind::Trigger => ("ShowCreateTrigger", 7, 2),
        ObjectKind::Event => ("ShowCreateEvent", 7, 3),
        _ => return None,
    })
}

/// The `SHOW CREATE` statement of `schema`.`name` of `kind`, as the server prints it.
/// `Ok(None)`: the kind has none, or the server returned no row.
pub async fn show_create(
    r: &mut dyn SectionReader,
    srv: &ServerCtx,
    kind: ObjectKind,
    schema: &str,
    name: &str,
) -> Result<Option<Name>, IntroError> {
    let Some((section, width, at)) = section_of(kind) else {
        return Ok(None);
    };
    if !srv.utf8mb4 {
        return Err(IntroError::Meta(MetaError::ReadOnlyNotEnforced));
    }
    let params = Params {
        scalars: vec![
            (Name::from("schema"), text(schema)),
            (Name::from("name"), text(name)),
        ],
        lists: Vec::new(),
    };
    let rows = r
        .read(section, srv.pack_ctx(Mode::Full, Level::Sources, params))
        .await?;
    Ok(rows::checked(section, &rows, width)?
        .first()
        .and_then(|row| opt_name(row, at)))
}
