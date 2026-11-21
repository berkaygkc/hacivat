//! Opening a script (HV-SCRIPT-06, R-M4b-4, R-M4b-25, R-M4b-27): context from location. The
//! tab is bound to the (connection, database) it is opened under and keeps it for its life.
//!
//! ## m4b-t01b-seams: the stub opens nothing. Task 14 fills it: it focuses the tab already open
//! for the same (library, file, connection, database), otherwise reads the file off the UI
//! thread, registers a console with [`ConsoleSource::Script`](crate::console::ConsoleSource) in
//! a [`TabKind::Script`](crate::shell::TabKind) tab and binds its session by the C-M4-5 rules.

use hacivat_proto::ProfileId;
use hacivat_ui_core::gpui_surface::App;

use crate::console::ConsoleId;
use crate::scripts::{DatabaseName, LibraryId, RelPath};

/// Opens `rel` of `library` under `profile` and `database` (`None`: unbound, "Open without a
/// connection"); the console of its tab. Stub: opens nothing and answers `None`.
pub fn open_script(
    cx: &mut App,
    library: LibraryId,
    rel: RelPath,
    profile: Option<ProfileId>,
    database: Option<DatabaseName>,
) -> Option<ConsoleId> {
    let _ = cx;
    tracing::info!(
        %library,
        rel = rel.as_str(),
        bound = profile.is_some(),
        database = database.as_ref().map(DatabaseName::as_str),
        "scripts: open_script is not implemented yet (Task 14); nothing opened"
    );
    None
}
