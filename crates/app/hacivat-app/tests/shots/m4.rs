// ## m4-t32-shots
//! The M4 shots (Task 32): `design/m4-explorer` states, each at 1280x800 and the 960x600
//! minimum, in every locale and theme, after every M3c shot; plus the PO's R-W5-14 surfaces
//! (Settings › General › Time zone; the clear-history question).
//!
//! How a state is reached: [`M4::open`] saves the state's profiles (the M2a fakes: nothing
//! connects, so nothing starts on the core runtime but what boot starts, and the live-thread
//! guard holds) and hands the explorer a fixture catalog (`explorer::adapter::set_catalog`,
//! the cached tree a profile shows before it connects, surface 3); [`M4::steps`] then expands
//! rows by their labels, each step waiting until the row it opens is shown. Catalogs are
//! data built through the model's public API (`support::catalogs`), so every shot is the same
//! on every machine.
#![cfg_attr(not(unix), allow(dead_code, unused_imports, unused_variables))]

use std::sync::Arc;

use hacivat_app::connections::ConnectionsModel;
use hacivat_app::connections::explorer;
use hacivat_app::explorer::adapter::{self, NodeInfo};
use hacivat_app::explorer::rows::DisplayGroup;
use hacivat_app::settings_ui::{self, Section};
use hacivat_meta::snapshot::IntroErrorCode;
use hacivat_meta::{CatalogSnapshot, Level, LoadState, NsKey, Watermark};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_session::profile::{Profile, ProfileDraft, ProfileEndpoint};
use hacivat_ui::actions::ActionRegistry;
use hacivat_ui_core::gpui_surface::{App, Window};

use super::m3a::{Step, instant, step};
use super::support::catalogs::{catalog_gen, schema, table, wide_pg_schema};

/// The tree rows' labels are drawn here.
const ROW: &str = "hacivat-ui/src/tree/view/row.rs";

/// One M4 state (a shot surface), named after its `design/m4-explorer` surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum M4 {
    // 1 · Explorer tree
    /// A PostgreSQL connection's cached tree: database, schemas, kind groups, a table's
    /// columns, Turkish names.
    TreePg,
    // 2 · Loading states
    /// A schema still being read ("Loading…" under it).
    TreeLoading,
    /// A schema whose read failed ("Couldn't read audit", Retry).
    TreeFailed,
    // 11 · Stale object
    /// A schema marked stale (changes on the server not read yet).
    TreeStale,
    // 12 · Invisible characters
    /// A table whose name has a zero-width joiner and one with a control character: the
    /// badge, and the character drawn visibly on the single-line row.
    TreeInvisible,
    // 1 · the 15,000-table schema (the perf scale)
    TreeLarge,
    // 5 · Context menus
    /// A table row's menu (Open data, View DDL, Copy…, Refresh, Hide objects like this).
    NodeMenu,
    /// A connection's menu with "Clear query history…" (R-W5-14 b).
    ConnMenu,
    // 13 · Settings › Metadata
    SettingsMetadata,
    // R-W5-14 a · Settings › General › Time zone
    SettingsTimeZone,
    // R-W5-14 b · "Clear query history…": the question (Cancel is the default)
    ClearHistoryQuestion,
}

/// Every M4 state, in shot order.
pub(super) const M4_STATES: [M4; 11] = [
    M4::TreePg,
    M4::TreeLoading,
    M4::TreeFailed,
    M4::TreeStale,
    M4::TreeInvisible,
    M4::TreeLarge,
    M4::NodeMenu,
    M4::ConnMenu,
    M4::SettingsMetadata,
    M4::SettingsTimeZone,
    M4::ClearHistoryQuestion,
];

impl M4 {
    pub(super) fn name(self) -> &'static str {
        match self {
            M4::TreePg => "m4-tree-pg",
            M4::TreeLoading => "m4-tree-loading",
            M4::TreeFailed => "m4-tree-failed",
            M4::TreeStale => "m4-tree-stale",
            M4::TreeInvisible => "m4-tree-invisible",
            M4::TreeLarge => "m4-tree-15000",
            M4::NodeMenu => "m4-node-menu",
            M4::ConnMenu => "m4-conn-menu-clear-history",
            M4::SettingsMetadata => "m4-settings-metadata",
            M4::SettingsTimeZone => "m4-settings-time-zone",
            M4::ClearHistoryQuestion => "m4-clear-history-question",
        }
    }

    /// Proof that the state is on screen (a label from this file, with this text).
    pub(super) fn marker(self) -> (&'static str, Option<&'static str>) {
        match self {
            M4::TreePg | M4::NodeMenu => (ROW, Some("müşteri_siparişleri")),
            M4::ConnMenu => (ROW, Some("shop")),
            M4::TreeLoading | M4::TreeFailed | M4::TreeStale => (ROW, Some("audit")),
            M4::TreeInvisible => (ROW, Some("line\u{2407}items")),
            M4::TreeLarge => (ROW, Some("t00000")),
            M4::SettingsMetadata => ("#settings-row-title-metadata.refresh_after_ddl", None),
            M4::SettingsTimeZone => ("#settings-row-title-ui.time_zone", None),
            M4::ClearHistoryQuestion => ("hacivat-ui/src/dialog.rs:", None),
        }
    }

    /// Labels (`#id`) that must be visible too.
    pub(super) fn also_shown(self) -> &'static [&'static str] {
        match self {
            M4::TreeLoading | M4::TreeFailed => &["#hv-tree-status-label"],
            M4::NodeMenu | M4::ConnMenu => &["#menu-item-label"],
            M4::SettingsTimeZone => &["#settings-row-note-ui.time_zone"],
            _ => &[],
        }
    }

    /// Brings the state's fixtures on screen in a freshly started shell.
    pub(super) fn open(self, window: &mut Window, cx: &mut App) {
        match self {
            M4::SettingsMetadata | M4::SettingsTimeZone => {
                ActionRegistry::run("workspace.open_settings", window, cx);
            }
            M4::ClearHistoryQuestion => {
                save(cx, pg("orders-dev", EnvTag::Dev));
            }
            _ => {
                let id = save(cx, pg("shop", EnvTag::Dev));
                adapter::set_catalog(id, Some(self.catalog()), None, cx);
                ConnectionsModel::select(cx, Some(id));
                explorer::focus_list(window, cx);
            }
        }
    }

    /// The fixture catalog of a tree state.
    fn catalog(self) -> Arc<CatalogSnapshot> {
        let pg = DbmsKind::Postgres;
        let orders = table(
            "orders",
            Some(16_400),
            &[
                ("id", "bigint"),
                ("customer_id", "bigint"),
                ("total", "numeric(12,2)"),
                ("created_at", "timestamp with time zone"),
            ],
        );
        let turkish = table(
            "müşteri_siparişleri",
            Some(16_410),
            &[("sipariş_no", "integer"), ("İl", "text")],
        );
        let customers = table("customers", Some(16_420), &[("id", "bigint")]);
        let public = |extra: Vec<hacivat_meta::namespace::Subtree>| {
            let mut objects = vec![customers.clone(), orders.clone(), turkish.clone()];
            objects.extend(extra);
            schema(pg, &["app", "public"], false, objects)
        };
        let audit = schema(
            pg,
            &["app", "audit"],
            false,
            vec![table("events", Some(16_500), &[("at", "timestamptz")])],
        );
        let stamp = std::time::SystemTime::UNIX_EPOCH;
        match self {
            M4::TreeLoading => catalog_gen(
                pg,
                2,
                vec![public(Vec::new())],
                vec![(
                    NsKey::schema(["app", "audit"]),
                    LoadState::Loading {
                        generation: 2,
                        level: Level::Structure,
                    },
                )],
            ),
            M4::TreeFailed => catalog_gen(
                pg,
                2,
                vec![public(Vec::new())],
                vec![(
                    NsKey::schema(["app", "audit"]),
                    LoadState::Failed {
                        at: stamp,
                        error: IntroErrorCode::PermissionDenied,
                        retry_after: std::time::Duration::ZERO,
                    },
                )],
            ),
            M4::TreeStale => catalog_gen(
                pg,
                2,
                vec![public(Vec::new()), audit],
                vec![(
                    NsKey::schema(["app", "audit"]),
                    LoadState::Loaded {
                        level: Level::Structure,
                        at: stamp,
                        watermark: Watermark::default(),
                        stale: true,
                    },
                )],
            ),
            M4::TreeInvisible => catalog_gen(
                pg,
                2,
                vec![public(vec![
                    table("invoice\u{200d}s", Some(16_430), &[]),
                    table("line\u{0007}items", Some(16_440), &[]),
                ])],
                Vec::new(),
            ),
            M4::TreeLarge => catalog_gen(
                pg,
                2,
                vec![wide_pg_schema("app", "public", 15_000, 2)],
                Vec::new(),
            ),
            _ => catalog_gen(pg, 2, vec![public(Vec::new()), audit], Vec::new()),
        }
    }

    /// The steps after [`M4::open`].
    pub(super) fn steps(self) -> Vec<Step> {
        match self {
            M4::TreePg => expand(
                &["shop", "app", "public", TABLES, "orders", COLUMNS],
                "created_at",
            ),
            M4::NodeMenu => {
                let mut s = expand(&["shop", "app", "public", TABLES], "orders");
                s.push(step(
                    |window, cx| open_node_menu("orders", window, cx),
                    |window, cx| super::m3a::menu_open(window, cx),
                ));
                s
            }
            M4::ConnMenu => vec![step(
                |window, cx| {
                    select("shop", cx);
                    explorer::open_context_menu(None, window, cx);
                },
                |window, cx| super::m3a::menu_open(window, cx),
            )],
            M4::TreeLoading | M4::TreeFailed | M4::TreeStale => {
                let mut s = expand(&["shop", "app"], "audit");
                if self != M4::TreeStale {
                    s.extend(expand(&["audit"], ""));
                }
                s
            }
            M4::TreeInvisible => expand(&["shop", "app", "public", TABLES], "orders"),
            M4::TreeLarge => expand(&["shop", "app", "public", TABLES], "t00000"),
            M4::SettingsMetadata => vec![instant(|_, cx| {
                settings_ui::fixtures::show_section(Section::Metadata, cx);
            })],
            M4::SettingsTimeZone => vec![instant(|_, cx| {
                settings_ui::fixtures::show_section(Section::General, cx);
            })],
            M4::ClearHistoryQuestion => vec![
                step(
                    |_, _| {},
                    |_, cx| {
                        hacivat_app::history::store(cx).is_some()
                            && profile_named(cx, "orders-dev").is_some()
                    },
                ),
                step(
                    |_, cx| {
                        if let Some(id) = profile_named(cx, "orders-dev") {
                            hacivat_app::history::clear::ask(id, cx);
                        }
                    },
                    |_, cx| hacivat_app::history::clear::question(cx).is_some(),
                ),
            ],
        }
    }
}

// ---- fixtures -------------------------------------------------------------------------------

fn pg(name: &str, env: EnvTag) -> Profile {
    Profile {
        name: name.to_string(),
        engine: DbmsKind::Postgres,
        environment: Some(env),
        database: Some("app".into()),
        endpoint: ProfileEndpoint::Server {
            host: format!("{name}.db.internal"),
            port: None,
        },
        ..Profile::default()
    }
}

fn save(cx: &mut App, profile: Profile) -> ProfileId {
    let name = profile.name.clone();
    ConnectionsModel::save(
        cx,
        ProfileDraft {
            is_new: true,
            ..ProfileDraft::edit(profile)
        },
    )
    .detach();
    profile_named(cx, &name).expect("the profile is in the model")
}

fn profile_named(cx: &App, name: &str) -> Option<ProfileId> {
    cx.global::<ConnectionsModel>()
        .profiles()
        .iter()
        .find(|p| p.name == name)
        .map(|p| p.id)
}

fn strip(s: &str) -> String {
    super::strip(s)
}

/// A kind-group header, named by its group (its label is localized).
const TABLES: &str = "#tables";
const COLUMNS: &str = "#columns";

/// Whether shown row `r` is the one `label` names: a group header for [`TABLES`] and
/// [`COLUMNS`], else the row whose label is `label`.
fn is(r: &explorer::ShownRow, label: &str, cx: &App) -> bool {
    let group = match label {
        TABLES => Some(DisplayGroup::Tables),
        COLUMNS => Some(DisplayGroup::Columns),
        _ => None,
    };
    match group {
        Some(g) => explorer::tree(cx).is_some_and(|t| {
            matches!(
                t.read(cx).model().info(r.key),
                Some(NodeInfo::Group { group, .. }) if group == g
            )
        }),
        None => strip(&r.data.label) == label,
    }
}

/// The key of the shown node row `label` names.
fn key_of(cx: &App, label: &str) -> Option<hacivat_ui::tree::TreeKey> {
    explorer::shown_rows(cx)
        .into_iter()
        .find(|r| r.status.is_none() && is(r, label, cx))
        .map(|r| r.key)
}

/// Whether a row labelled `label` is shown (a status row when `label` is empty: loading or
/// failed children).
fn shown(cx: &App, label: &str) -> bool {
    explorer::shown_rows(cx).into_iter().any(|r| {
        if label.is_empty() {
            r.status.is_some()
        } else {
            r.status.is_none() && is(&r, label, cx)
        }
    })
}

/// Expands the rows labelled `labels` in order, each step waiting until the next label is
/// shown; the last waits for `then`.
fn expand(labels: &'static [&'static str], then: &'static str) -> Vec<Step> {
    labels
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let next = labels.get(i + 1).copied().unwrap_or(then);
            step(
                move |_, cx| {
                    let Some(key) = key_of(cx, label) else {
                        return;
                    };
                    if let Some(tree) = explorer::tree(cx) {
                        tree.update(cx, |t, cx| t.set_expanded(key, true, cx));
                    }
                },
                move |_, cx| shown(cx, next),
            )
        })
        .collect()
}

/// Opens the node menu of the row labelled `label` under it (as Shift-F10 there does).
fn open_node_menu(label: &str, window: &mut Window, cx: &mut App) {
    select(label, cx);
    let Some(key) = key_of(cx, label) else {
        return;
    };
    let Some(items) = hacivat_app::explorer::actions::node_menu(key, cx) else {
        return;
    };
    let anchor = explorer::tree(cx)
        .and_then(|t| t.read(cx).row_bounds(key))
        .map(|b| b.bottom_left())
        .unwrap_or_default();
    if let Some(ws) = window.root::<hacivat_app::shell::Workspace>().flatten() {
        ws.update(cx, |ws, cx| {
            ws.open_popup_menu(anchor, items, window, cx);
        });
    }
}

/// Puts the tree's cursor on the row labelled `label`.
fn select(label: &str, cx: &mut App) {
    let Some(key) = key_of(cx, label) else {
        return;
    };
    if let Some(tree) = explorer::tree(cx) {
        tree.update(cx, |t, cx| t.select(key, cx));
    }
}
