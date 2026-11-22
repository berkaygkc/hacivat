//! The connection chooser (HV-SCRIPT-09, R-M4b-9, Q14; NOTES surface 7): after a pick in Go to
//! script, and from "Open under another connection…", it lists only the connections where the
//! file is visible (binding and environment scope), the active tab's connection first, then
//! the least severe environment; **production is never preselected** unless it is the active
//! tab's connection, and the connection a tab or row is under is never offered for it.
//!
//! ## m4b-t01b-seams: nothing yet (Task 15).
//!
//! ## m4b-t15-goto
//! - **Candidates** ([`candidates`], R-M4b-31): every place a script can open under
//!   (`projection::script_places`: the databases the explorer asked about plus each profile's
//!   configured one) whose binding includes the file's library, whose environment shows the
//!   file (`view(env)`, the tree's own rule, R-M4b-6), and where the script may run
//!   (`open::session_plan`, R-M4b-25), so a row never opens a refusal. The place a tab or row is
//!   under is left out (R-M4b-4). They are computed from the live snapshot and profiles each
//!   time the chooser is drawn up and again on every Enter (never a stale list).
//! - **Order** ([`order`]): the active tab's connection first (its own database first), then
//!   local, dev, test, staging, production (the reverse of `EnvTag::severity_rank`), then
//!   untagged, then the connection's name in the UI language's order, then the database.
//! - **The active row** ([`preselect`]): the first row that is not production, or whose
//!   connection is the active tab's. "The focused connection" is always the **active tab's**
//!   ([`focused_place`]), never a tree row's or the explorer's selection: with only production
//!   candidates and the active tab elsewhere no row is active, and Enter does nothing until the
//!   user moves onto one.
//! - **What a pick does** ([`decide`]): one candidate that is not production opens at once;
//!   production, or two or more, ask with the chooser; none offers "Open without a connection"
//!   (an unbound tab, R-M4b-30), only for a file an unbound tab can show.
//! - The chooser itself is drawn by the Go-to-script layer (`super::goto`), which is also what
//!   "Open under another connection…" opens, from a tree row (`scripts.open_under`) and from a
//!   script tab's notices (`open::set_open_under_another`).

use std::cmp::Ordering;
use std::rc::Rc;

use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_scripts::{EntryKind, LinkState};
use hacivat_ui::actions::{ActionDef, ActionRegistry};
use hacivat_ui_core::gpui_surface::App;

use super::projection::{self, Node};
use super::{DatabaseName, LibraryId, RelPath};
use crate::console::Consoles;
use crate::explorer::EXPLORER_TREE_CONTEXT;
use crate::explorer::rows::NameOrder;

/// One place the chooser offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub profile: ProfileId,
    pub database: DatabaseName,
    /// The connection's name, verbatim (shown escaped).
    pub name: String,
    pub env: Option<EnvTag>,
    pub engine: DbmsKind,
}

impl Candidate {
    pub fn is_production(&self) -> bool {
        self.env == Some(EnvTag::Production)
    }
}

/// The active tab's connection, and the database its script tab is bound to (`None` for a
/// console tab): "the focused connection" of R-M4b-9. `None` when the active tab runs nowhere.
pub fn focused_place(cx: &App) -> Option<(ProfileId, Option<DatabaseName>)> {
    let tab = cx.try_global::<crate::shell::workspace::ActiveTab>()?.0?;
    if let Some(doc) = super::tabs::doc_of_tab(tab, cx) {
        let spec = doc.read(cx).spec(cx)?;
        // A tab whose connection was deleted runs nowhere: no focused connection.
        super::open::profile_of(spec.profile?, cx)?;
        return Some((spec.profile?, spec.database));
    }
    let console = Consoles::by_tab(cx, tab)?;
    let profile = Consoles::get(cx, console)?.profile?;
    super::open::profile_of(profile, cx)?;
    Some((profile, None))
}

/// Whether `entry` is a script a tab can open (a file, not a link that leaves the library).
fn opens(entry: &hacivat_scripts::ScriptEntry) -> bool {
    entry.kind != EntryKind::Folder
        && matches!(entry.link, LinkState::Plain | LinkState::InsideLink)
}

/// Every place where file `rel` of `library` can be opened, except `exclude` (the place a tab
/// or tree row is under), unordered. Empty without a library service.
pub fn candidates(
    library: LibraryId,
    rel: &RelPath,
    exclude: Option<(ProfileId, &str)>,
    cx: &App,
) -> Vec<Candidate> {
    let Some(service) = super::open::service(cx) else {
        return Vec::new();
    };
    let snap = service.snapshot();
    let mut out: Vec<Candidate> = Vec::new();
    for place in projection::script_places(cx) {
        let p = &place.profile;
        if exclude == Some((p.id, place.database.as_str())) || !place.libraries.contains(&library) {
            continue;
        }
        let shown = snap
            .view(p.environment)
            .library(library)
            .and_then(|l| l.entry(rel))
            .is_some_and(opens);
        if !shown {
            continue;
        }
        let database = DatabaseName::new(&place.database);
        if super::open::session_plan(p, &database, cx).is_err() {
            continue;
        }
        if out
            .iter()
            .any(|c| c.profile == p.id && c.database == database)
        {
            continue;
        }
        out.push(Candidate {
            profile: p.id,
            database,
            name: p.name.clone(),
            env: p.environment,
            engine: p.engine,
        });
    }
    out
}

/// Whether an unbound tab (no connection, `None` environment) can show file `rel` of
/// `library`: "Open without a connection" is offered only then.
pub fn opens_unbound(library: LibraryId, rel: &RelPath, cx: &App) -> bool {
    super::open::service(cx).is_some_and(|s| {
        s.snapshot()
            .view(None)
            .library(library)
            .and_then(|l| l.entry(rel))
            .is_some_and(opens)
    })
}

/// Least severe first: local, dev, test, staging, production, then untagged.
fn env_rank(env: Option<EnvTag>) -> u8 {
    match env {
        Some(t) => 4 - t.severity_rank(),
        None => 5,
    }
}

/// How close `c` is to the focused place: its connection and database (0), its connection (1),
/// another one (2).
fn focus_rank(c: &Candidate, focused: Option<&(ProfileId, Option<DatabaseName>)>) -> u8 {
    match focused {
        Some((p, db)) if *p == c.profile && db.as_ref() == Some(&c.database) => 0,
        Some((p, _)) if *p == c.profile => 1,
        _ => 2,
    }
}

/// `rows` in the chooser's order (R-M4b-9).
pub fn order(
    mut rows: Vec<Candidate>,
    focused: Option<&(ProfileId, Option<DatabaseName>)>,
    names: &NameOrder,
) -> Vec<Candidate> {
    rows.sort_by(|a, b| {
        focus_rank(a, focused)
            .cmp(&focus_rank(b, focused))
            .then_with(|| env_rank(a.env).cmp(&env_rank(b.env)))
            .then_with(|| names.cmp(&a.name, &b.name))
            .then_with(|| a.profile.to_string().cmp(&b.profile.to_string()))
            .then_with(|| names.cmp(a.database.as_str(), b.database.as_str()))
            .then(Ordering::Equal)
    });
    rows
}

/// The row active when the chooser opens: the first that is not production or whose
/// connection is the focused one; `None` when every row is production elsewhere.
pub fn preselect(
    rows: &[Candidate],
    focused: Option<&(ProfileId, Option<DatabaseName>)>,
) -> Option<usize> {
    let focused = focused.map(|(p, _)| *p);
    rows.iter()
        .position(|c| !c.is_production() || Some(c.profile) == focused)
}

/// What a pick in Go to script does with its (ordered) candidates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// One candidate that is not production: open there at once.
    Open(Candidate),
    /// Production, or two or more: ask.
    Choose,
    /// No connection shows the file.
    Unbound,
}

pub fn decide(rows: &[Candidate]) -> Decision {
    match rows {
        [] => Decision::Unbound,
        [one] if !one.is_production() => Decision::Open(one.clone()),
        _ => Decision::Choose,
    }
}

/// The candidates for `rel` of `library` in the chooser's order, with the focused place they
/// were ordered for.
pub fn ordered(
    library: LibraryId,
    rel: &RelPath,
    exclude: Option<(ProfileId, &str)>,
    cx: &App,
) -> (Vec<Candidate>, Option<(ProfileId, Option<DatabaseName>)>) {
    let focused = focused_place(cx);
    let names = projection::name_order(cx);
    (
        order(
            candidates(library, rel, exclude, cx),
            focused.as_ref(),
            &names,
        ),
        focused,
    )
}

// ---- "Open under another connection…" ---------------------------------------------------------

/// The tree row's command (`ExplorerTree`, menu and palette only): the chooser for the cursor's
/// file, without the place the row is under.
pub const OPEN_UNDER: &str = "scripts.open_under";

/// The cursor's file row: its library, path and place.
fn cursor_file(cx: &App) -> Option<(LibraryId, RelPath, ProfileId, String)> {
    let (_, place) = projection::cursor_place(cx)?;
    let Node::Entry { library, entry, .. } = &place.node else {
        return None;
    };
    if !opens(entry) {
        return None;
    }
    let rel = RelPath::parse(entry.rel.as_str()).ok()?;
    Some((*library, rel, place.profile, place.database.clone()))
}

fn can_open_under(cx: &App) -> bool {
    cursor_file(cx).is_some_and(|(library, rel, profile, database)| {
        !candidates(library, &rel, Some((profile, &database)), cx).is_empty()
    })
}

fn open_under_command(window: &mut hacivat_ui_core::gpui_surface::Window, cx: &mut App) {
    if let Some((library, rel, profile, database)) = cursor_file(cx) {
        super::goto::open_chooser(library, rel, Some((profile, database)), window, cx);
    }
}

/// Registers `scripts.open_under` and fills the script tab's "Open under another connection…"
/// (`open::set_open_under_another`, Task 14's seam) with the chooser.
pub(crate) fn install(cx: &mut App) {
    let reg = cx.default_global::<ActionRegistry>();
    let _ = reg.register(ActionDef {
        id: OPEN_UNDER,
        title_id: "hv-cmd-scripts-open-under",
        context: EXPLORER_TREE_CONTEXT,
        run: Rc::new(open_under_command),
        enabled: can_open_under,
    });
    super::open::set_open_under_another(
        Rc::new(|tab, _places, cx| {
            // The chooser computes its own rows (the explorer's places too), without the tab's
            // own place; it never rebinds the tab.
            let exclude = tab
                .profile
                .zip(tab.database.as_ref().map(|d| d.as_str().to_owned()));
            let (library, rel) = (tab.library, tab.rel.clone());
            crate::exec_ui::in_workspace_window(cx, move |_, window, cx| {
                super::goto::open_chooser(library, rel, exclude, window, cx);
            });
        }),
        cx,
    );
}
