//! The introspection level pill and its submenu (Task 20; mock 6): blue when set here, grey when
//! inherited or automatic; Automatic, Names only, Structure, Structure and sources.
//!
//! ## m4-t20-levels
//!
//! **Where a level comes from** (datagrip/04 §3.1, D-038). A database or schema row's level is
//! - **set here** (blue): an entry for the node in `ProfileUserData.level_overrides`
//!   ([`super::userdata`]);
//! - **inherited** (grey): a PostgreSQL schema without its own entry under a database that has
//!   one;
//! - **automatic** (grey): neither; the pill then shows the level the namespace was last read
//!   at and the number of its major objects, so a schema that has not been read yet shows no
//!   pill (nothing is claimed about it). A database row shows its pill only when it is set
//!   there: its automatic level is the engine's per-namespace choice (Task 15's
//!   `LevelPolicy`), not a fact the explorer holds.
//!
//! Until a profile's data file has been read the explorer draws no pill for it at all: an
//! automatic pill could hide a level the user set.
//!
//! **The list.** A click on the pill and "Introspection level ›" (Task 24's node menu,
//! [`menu_items`]) open the same four rows; each runs its `explorer.set_level.*` command on the
//! tree's cursor row (a click on a pill first moves the cursor to the pill's row). Picking one
//! writes the profile's data file; Task 24 forwards the change to the engine
//! (`IntroHandle::set_level_override`), which re-reads the namespace.

use std::rc::Rc;
use std::sync::Arc;

use hacivat_l10n::ArgValue;
use hacivat_meta::userdata::ProfileUserData;
use hacivat_meta::{Level, Name, NamespaceSnapshot, NsKey, NsKind, ObjectKey};
use hacivat_proto::ProfileId;
use hacivat_ui::menu::MenuItemSpec;
use hacivat_ui::text::TextFn;
use hacivat_ui::tree::{RowPill, TreeKey};
use hacivat_ui_core::gpui_surface::{App, Pixels, Point, SharedString, Window};

use super::adapter::{self, NodeInfo};
use super::userdata;
use crate::l10n_global::L10nGlobal;
use crate::shell::Workspace;

/// The level commands, in the list's order: Automatic, Names only, Structure, Structure and
/// sources.
pub const LEVEL_COMMANDS: [(&str, Option<Level>); 4] = [
    ("explorer.set_level.auto", None),
    ("explorer.set_level.names", Some(Level::Names)),
    ("explorer.set_level.structure", Some(Level::Structure)),
    ("explorer.set_level.sources", Some(Level::Sources)),
];

/// Where a row's level comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PillSource {
    /// Set on this node (blue).
    Explicit,
    /// From the database above (its name, verbatim).
    Inherited { parent: String },
    /// Read automatically, for this many major objects.
    Auto { objects: usize },
}

/// The level code a Fluent selector takes (`$level`).
pub fn level_code(level: Level) -> &'static str {
    match level {
        Level::Names => "names",
        Level::Structure => "structure",
        Level::Sources => "sources",
    }
}

/// The node key a namespace root's level is kept under (by path: a namespace keeps its key
/// whether or not it was read).
pub fn namespace_object_key(ns: &NsKey) -> ObjectKey {
    ObjectKey {
        kind: ns.kind.root_kind(),
        path: ns.path.iter().cloned().collect(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    }
}

/// The number of major objects (those in a kind group) a namespace holds.
pub fn major_objects(ns: &NamespaceSnapshot) -> usize {
    let dbms = ns.dbms();
    ns.objs()
        .iter()
        .skip(1)
        .filter(|o| o.kind().group(dbms).is_some())
        .count()
}

/// A database row's level: only when it is set there.
pub fn database_level(data: &ProfileUserData, db: &Name) -> Option<(Level, PillSource)> {
    let key = namespace_object_key(&NsKey::database(db.clone()));
    data.level_overrides
        .get(&key)
        .map(|l| (*l, PillSource::Explicit))
}

/// A schema row's level (a MySQL/MariaDB database, a SQLite schema): set here, inherited from
/// its database (PostgreSQL), or the level `ns` was read at.
pub fn schema_level(
    data: &ProfileUserData,
    key: &NsKey,
    ns: Option<&NamespaceSnapshot>,
) -> Option<(Level, PillSource)> {
    if let Some(l) = data.level_overrides.get(&namespace_object_key(key)) {
        return Some((*l, PillSource::Explicit));
    }
    if key.kind == NsKind::Schema && key.path.len() == 2 {
        let db = key.path[0].clone();
        if let Some(l) = data
            .level_overrides
            .get(&namespace_object_key(&NsKey::database(db.clone())))
        {
            return Some((
                *l,
                PillSource::Inherited {
                    parent: db.to_string(),
                },
            ));
        }
    }
    let ns = ns?;
    Some((
        ns.meta().level,
        PillSource::Auto {
            objects: major_objects(ns),
        },
    ))
}

/// Renders a Fluent message with its arguments.
pub type Translate<'a> = &'a dyn Fn(&str, Vec<(&'static str, ArgValue)>) -> SharedString;

/// The pill of a row at `level` from `source`; `t` renders a Fluent message with arguments.
pub fn row_pill(level: Level, source: &PillSource, t: Translate<'_>) -> RowPill {
    let text = t(
        match level {
            Level::Names => "hv-explorer-pill-names",
            Level::Structure => "hv-explorer-pill-structure",
            Level::Sources => "hv-explorer-pill-sources",
        },
        Vec::new(),
    );
    let code = ArgValue::Text(level_code(level).to_string());
    let tooltip = match source {
        PillSource::Explicit => t("hv-explorer-pill-tip-explicit", vec![("level", code)]),
        PillSource::Inherited { parent } => t(
            "hv-explorer-pill-tip-inherited",
            vec![("parent", ArgValue::Text(parent.clone())), ("level", code)],
        ),
        PillSource::Auto { objects } => t(
            "hv-explorer-pill-tip-auto",
            vec![
                (
                    "n",
                    ArgValue::Count(u64::try_from(*objects).unwrap_or(u64::MAX)),
                ),
                ("level", code),
            ],
        ),
    };
    RowPill {
        text,
        explicit: *source == PillSource::Explicit,
        tooltip,
    }
}

// ---- the list ---------------------------------------------------------------------------------

/// A database or schema row the level commands act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelTarget {
    pub profile: ProfileId,
    /// The node's key in `level_overrides`.
    pub key: ObjectKey,
    /// The level set on the node (`None`: Automatic).
    pub set: Option<Level>,
    /// The level in effect, when known (the Automatic row's "now …" hint).
    pub effective: Option<Level>,
}

/// The level target of tree row `key`: a database or schema row of a profile whose data has
/// been read, not out of scope (`None` otherwise).
pub fn target_of(key: TreeKey, cx: &App) -> Option<LevelTarget> {
    let tree = crate::connections::explorer::tree(cx)?;
    let info = tree.read(cx).model().info(key)?;
    let (profile, ns) = match info {
        NodeInfo::Database { profile, name } => (profile, NsKey::database(name)),
        NodeInfo::Schema { profile, ns } => (profile, ns),
        _ => return None,
    };
    let data = userdata::get(profile, cx)?;
    let catalog = adapter::catalog_of(profile, cx);
    let state = catalog.as_ref().and_then(|c| {
        c.states
            .get(&hacivat_meta::snapshot::ContainerKey::Namespace(ns.clone()))
            .cloned()
    });
    if matches!(state, Some(hacivat_meta::LoadState::OutOfScope)) {
        return None;
    }
    let level = match ns.kind {
        NsKind::Database => database_level(&data, &ns.path[0]),
        _ => {
            let loaded: Option<Arc<NamespaceSnapshot>> = catalog
                .as_ref()
                .and_then(|c| c.namespaces.get(&ns).cloned());
            schema_level(&data, &ns, loaded.as_deref())
        }
    };
    let key = namespace_object_key(&ns);
    Some(LevelTarget {
        profile,
        set: data.level_overrides.get(&key).copied(),
        key,
        effective: level.map(|(l, _)| l),
    })
}

/// The target of the tree's cursor row.
pub fn cursor_target(cx: &App) -> Option<LevelTarget> {
    let tree = crate::connections::explorer::tree(cx)?;
    let cursor = tree.read(cx).cursor()?;
    target_of(cursor, cx)
}

fn t_dynamic(cx: &App, id: &str, args: Vec<(&'static str, ArgValue)>) -> SharedString {
    match cx.try_global::<L10nGlobal>() {
        Some(g) => g.l10n.t_dynamic(id, args).into(),
        None => SharedString::from(id.to_string()),
    }
}

fn text(id: &'static str) -> TextFn {
    Rc::new(move |cx: &App| t_dynamic(cx, id, Vec::new()))
}

/// The four rows of the level list for `target` (the pill's list, and Task 24's "Introspection
/// level ›" submenu): the current setting checked, each with its hint.
pub fn menu_items(target: &LevelTarget) -> Vec<MenuItemSpec> {
    LEVEL_COMMANDS
        .iter()
        .map(|(action, level)| {
            let (label, hint): (TextFn, Option<TextFn>) = match level {
                None => (
                    text("hv-explorer-level-auto"),
                    target.effective.map(|now| -> TextFn {
                        Rc::new(move |cx: &App| {
                            t_dynamic(
                                cx,
                                "hv-explorer-level-auto-hint",
                                vec![("level", ArgValue::Text(level_code(now).to_string()))],
                            )
                        })
                    }),
                ),
                Some(Level::Names) => (
                    text("hv-explorer-level-names"),
                    Some(text("hv-explorer-level-names-hint")),
                ),
                Some(Level::Structure) => (
                    text("hv-explorer-level-structure"),
                    Some(text("hv-explorer-level-structure-hint")),
                ),
                Some(Level::Sources) => (
                    text("hv-explorer-level-sources"),
                    Some(text("hv-explorer-level-sources-hint")),
                ),
            };
            let mut item = MenuItemSpec::new(label, *action).checked(target.set == *level);
            if let Some(hint) = hint {
                item = item.hint(hint);
            }
            item
        })
        .collect()
}

/// A click on row `key`'s pill: the row becomes the cursor (the commands act on it) and the
/// level list opens at `at` (the pill's bottom-left).
pub fn open_from_pill(key: TreeKey, at: Point<Pixels>, window: &mut Window, cx: &mut App) {
    let Some(target) = target_of(key, cx) else {
        return;
    };
    if let Some(tree) = crate::connections::explorer::tree(cx) {
        tree.update(cx, |t, cx| t.select(key, cx));
    }
    let items = menu_items(&target);
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    ws.update(cx, |ws, cx| {
        ws.open_popup_menu(at, items, window, cx);
    });
}

/// Whether the level commands can run: the cursor is on a database or schema row whose
/// profile's data is known.
pub fn can_set_level(cx: &App) -> bool {
    cursor_target(cx).is_some()
}

/// Sets (`Some`) or clears (`None`: Automatic) the level of the cursor's row.
pub fn set_level(level: Option<Level>, cx: &mut App) {
    let Some(target) = cursor_target(cx) else {
        return;
    };
    if target.set == level {
        return;
    }
    userdata::set_level_override(target.profile, target.key, level, cx).detach();
}
