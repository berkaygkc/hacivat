// ## m4b-t11-editor
//! The M4b shots (`design/m4b-scripts`), each at 1280x800 and the 960x600 minimum, in every
//! locale and theme, after every M4 shot. Task 11's two states of surface 9: the T2 read-only
//! preview's banner, and the 20–100 MB paste question. Later M4b tasks add theirs below their
//! own block headers.
//!
//! How a state is reached: [`M4b::open`] saves a development profile (an M2a fake: nothing
//! connects); [`M4b::steps`] opens a console over the fixture text and puts the editor in the
//! state, each step waiting until the state is shown.
#![cfg_attr(not(unix), allow(dead_code, unused_imports, unused_variables))]

use hacivat_app::connections::ConnectionsModel;
use hacivat_app::console::view::fixtures as console_fixtures;
use hacivat_app::shell::Workspace;
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_session::profile::{Profile, ProfileDraft, ProfileEndpoint};
use hacivat_ui::dialog::DialogSize;
use hacivat_ui_core::gpui_surface::{App, Focusable as _, Role, Window};
use hacivat_ui_editor::large_file::preview_cut;
use hacivat_ui_editor::{PreviewSpec, Tier, TierLimits, clipboard};

use super::m3a::{Step, editor_in, step};

/// The profile the states open their console under.
const DEV: &str = "orders-dev";

/// One M4b state (a shot surface), named after its `design/m4b-scripts` surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum M4b {
    // 9 · Large files
    /// A 148 MB dump open as a T2 preview of its first 2.5 MB: the banner above the text.
    T2Preview,
    /// "Paste 42.6 MB?": the 420 px question, Cancel focused.
    PasteConfirm,
    // ## m4b-t13-explorer: surfaces 1, 2 and 11.
    // 1 · Scripts node
    /// The library under orders-prod: "shared", the `@production` folder with its red mark,
    /// the git badges (M and N amber, C ink, the folder dot), counts.
    ScriptsProd,
    /// The same library under orders-staging: no `@production`, its own count.
    ScriptsStaging,
    // 2 · Context menus
    /// A file row's menu (Open, Open under another connection… (## m4b-t15-goto: enabled,
    /// orders-prod also shows the file), Rename F2, Move to Trash, Reveal, Copy path, Copy
    /// relative path).
    ScriptsFileMenu,
    // 11 · Library diagnostics popover
    /// The problems row's popover: an unknown `@prod` folder, said in plain words.
    ScriptsDiagnostics,
    // ## m4b-t13-explorer fix round 1 (review m7)
    /// The name field over a file row, with a refused name and its reason under it.
    ScriptsNameField,
    /// A Scripts row's tooltip (count, "shared") over a tree with a link pointing outside.
    ScriptsTooltip,
    /// MySQL (under the non-default database `kampanya`) and SQLite (above `main`).
    ScriptsEngines,
    /// The "Moved to Trash" toast.
    ScriptsTrashed,
    // ## m4b-t13-explorer
    // ## m4b-t14-tabs: surfaces 5, 6, 6b and the Run refusal (`design/m4b-scripts`).
    /// Surface 5: one file open under orders-prod and under orders-staging, two tabs (N1: they
    /// collapse at 960 without overlapping), the staging one active.
    ScriptTabs,
    /// Surface 6: a dirty tab whose file changed on disk ("Changed on disk": Reload, Keep mine).
    ScriptChanged,
    /// Surface 6: "Deleted on disk" (Save, Close tab).
    ScriptDeleted,
    /// Surface 6: "No longer listed under this connection", with "Open under another
    /// connection…" disabled and its reason (N2).
    ScriptUnlisted,
    /// Surface 5: the MySQL Run refusal naming both databases.
    ScriptRunRefused,
    /// Surface 5: the connection the tab was opened under was deleted (unbound, runs nothing).
    ScriptRemoved,
    /// Surface 6b: "Save changes to …?" (Don't save · Save · Cancel focused).
    ScriptUnsaved,
    /// Surface 5, connected (fix round 1, minor 12): a script tab under the production
    /// connection, connected: the capsule, the hairline and the status segment follow it.
    ScriptConnectedProd,
    // ## m4b-t14-tabs
    // ## m4b-t15-goto: surface 7 (`design/m4b-scripts`).
    /// Go to script's results for the empty query: names, folders (the `@production` one with
    /// its mark), libraries.
    GotoResults,
    /// The chooser for a file three connections show, no tab active: dev active, production
    /// last and never preselected ("Production is never chosen for you.").
    GotoChooser,
    /// The chooser for a `@production` file: production only, no row active.
    GotoProdOnly,
    /// The chooser with a production tab active: production first, "current", and active.
    GotoFocusedProd,
    /// The chooser for a file no connection shows: "Open without a connection".
    GotoNoCandidate,
    // ## m4b-t15-goto
}

/// Every M4b state, in shot order.
// ## m4b-t13-explorer: Task 13's four after Task 11's two.
// ## m4b-t13-explorer fix round 1 (review m7): and its four more.
// ## m4b-t14-tabs (named exception): Task 14's states after Task 11's.
// ## m4b-t15-goto (named exception): Task 15's five after them.
pub(super) const M4B_STATES: [M4b; 23] = [
    M4b::T2Preview,
    M4b::PasteConfirm,
    M4b::ScriptsProd,
    M4b::ScriptsStaging,
    M4b::ScriptsFileMenu,
    M4b::ScriptsDiagnostics,
    M4b::ScriptsNameField,
    M4b::ScriptsTooltip,
    M4b::ScriptsEngines,
    M4b::ScriptsTrashed,
    M4b::ScriptTabs,
    M4b::ScriptChanged,
    M4b::ScriptDeleted,
    M4b::ScriptUnlisted,
    M4b::ScriptRunRefused,
    M4b::ScriptRemoved,
    M4b::ScriptUnsaved,
    M4b::ScriptConnectedProd,
    // ## m4b-t15-goto
    M4b::GotoResults,
    M4b::GotoChooser,
    M4b::GotoProdOnly,
    M4b::GotoFocusedProd,
    M4b::GotoNoCandidate,
];

/// The preview's file: 148 MB (the mock's).
const T2_TOTAL: u64 = 148 << 20;
/// The paste the question is about (42.6 MB) and the document it would make (61.0 MB).
const PASTE_BYTES: u64 = 44_669_747;
const PASTE_TOTAL: u64 = 63_963_136;
/// The script the paste question is over.
const PASTED_INTO: &str =
    "-- müşteri raporu\nSELECT id, city, total\nFROM orders\nWHERE city = 'İzmir';\n";

impl M4b {
    pub(super) fn name(self) -> &'static str {
        match self {
            M4b::T2Preview => "m4b-editor-t2-preview",
            M4b::PasteConfirm => "m4b-editor-paste-confirm",
            // ## m4b-t13-explorer
            M4b::ScriptsProd => "m4b-scripts-tree-prod",
            M4b::ScriptsStaging => "m4b-scripts-tree-staging",
            M4b::ScriptsFileMenu => "m4b-scripts-file-menu",
            M4b::ScriptsDiagnostics => "m4b-scripts-diagnostics",
            // ## m4b-t13-explorer fix round 1
            M4b::ScriptsNameField => "m4b-scripts-name-field",
            M4b::ScriptsTooltip => "m4b-scripts-row-tooltip",
            M4b::ScriptsEngines => "m4b-scripts-mysql-sqlite",
            M4b::ScriptsTrashed => "m4b-scripts-trashed",
            // ## m4b-t14-tabs
            M4b::ScriptTabs => "m4b-script-tabs",
            M4b::ScriptChanged => "m4b-script-changed",
            M4b::ScriptDeleted => "m4b-script-deleted",
            M4b::ScriptUnlisted => "m4b-script-unlisted",
            M4b::ScriptRunRefused => "m4b-script-run-refused",
            M4b::ScriptRemoved => "m4b-script-removed",
            M4b::ScriptUnsaved => "m4b-script-unsaved",
            M4b::ScriptConnectedProd => "m4b-script-connected-prod",
            // ## m4b-t15-goto
            M4b::GotoResults => "m4b-goto-results",
            M4b::GotoChooser => "m4b-goto-chooser",
            M4b::GotoProdOnly => "m4b-goto-chooser-prod-only",
            M4b::GotoFocusedProd => "m4b-goto-chooser-prod-focused",
            M4b::GotoNoCandidate => "m4b-goto-chooser-no-candidate",
        }
    }

    /// Proof that the state is on screen (a label from this file, with this text).
    pub(super) fn marker(self) -> (&'static str, Option<&'static str>) {
        match self {
            M4b::T2Preview => ("#editor-t2-title", None),
            M4b::PasteConfirm => ("hacivat-ui/src/dialog.rs:", None),
            // ## m4b-t13-explorer
            M4b::ScriptsProd => (ROW, Some("drop_old.sql")),
            M4b::ScriptsStaging | M4b::ScriptsFileMenu => (ROW, Some("haftalık_rapor.sql")),
            M4b::ScriptsDiagnostics => ("hacivat-ui/src/explain.rs:", None),
            // ## m4b-t13-explorer fix round 1
            M4b::ScriptsNameField => ("hacivat-app/src/scripts/file_ops.rs:", None),
            M4b::ScriptsTooltip => ("hacivat-ui/src/popover.rs:", None),
            M4b::ScriptsEngines => (ROW, Some("kampanya_raporu.sql")),
            M4b::ScriptsTrashed => ("hacivat-ui/src/toast.rs:", None),
            // ## m4b-t14-tabs
            M4b::ScriptTabs | M4b::ScriptConnectedProd => ("#tab-title", None),
            M4b::ScriptUnsaved => ("hacivat-ui/src/dialog.rs:", None),
            M4b::ScriptChanged
            | M4b::ScriptDeleted
            | M4b::ScriptUnlisted
            | M4b::ScriptRunRefused
            | M4b::ScriptRemoved => ("hacivat-ui/src/notice.rs:", None),
            // ## m4b-t15-goto
            M4b::GotoResults => (GOTO, Some("daily_revenue.sql")),
            M4b::GotoChooser | M4b::GotoProdOnly | M4b::GotoFocusedProd => {
                (GOTO, Some("orders-prod"))
            }
            M4b::GotoNoCandidate => (GOTO, None),
        }
    }

    /// Labels (`#id`) that must be visible too.
    pub(super) fn also_shown(self) -> &'static [&'static str] {
        match self {
            M4b::T2Preview => &["#editor-t2-body"],
            M4b::PasteConfirm => &[],
            // ## m4b-t13-explorer
            M4b::ScriptsFileMenu => &["#menu-item-label"],
            M4b::ScriptsProd | M4b::ScriptsStaging | M4b::ScriptsDiagnostics => &[],
            M4b::ScriptsNameField
            | M4b::ScriptsTooltip
            | M4b::ScriptsEngines
            | M4b::ScriptsTrashed => &[],
            // ## m4b-t14-tabs
            M4b::ScriptChanged
            | M4b::ScriptDeleted
            | M4b::ScriptUnlisted
            | M4b::ScriptRunRefused
            | M4b::ScriptRemoved
            | M4b::ScriptTabs
            | M4b::ScriptConnectedProd
            | M4b::ScriptUnsaved => &[],
            // ## m4b-t15-goto
            M4b::GotoResults
            | M4b::GotoChooser
            | M4b::GotoProdOnly
            | M4b::GotoFocusedProd
            | M4b::GotoNoCandidate => &[],
        }
    }

    /// Brings the state's fixtures on screen in a freshly started shell.
    pub(super) fn open(self, window: &mut Window, cx: &mut App) {
        // ## m4b-t13-explorer
        if self == M4b::ScriptsEngines {
            return scripts::open_engines(window, cx);
        }
        if let Some(under) = self.scripts_under() {
            return scripts::open(self, under, window, cx);
        }
        save(cx, pg(DEV, EnvTag::Dev));
        // ## m4b-t14-tabs: the connections a script is open under.
        // ## m4b-t15-goto (named line): and the ones Go to script offers.
        if self.is_script() || self.is_goto() {
            let mut prod = pg(PROD, EnvTag::Production);
            if self.connects() {
                // An IP literal: nothing resolves (the M3a fakes dial themselves).
                prod.endpoint = ProfileEndpoint::Server {
                    host: "10.0.4.12".into(),
                    port: None,
                };
                prod.user = Some("deploy".into());
                prod.auth = hacivat_secrets::AuthProviderId::None;
            }
            save(cx, prod);
            save(cx, pg(STAGING, EnvTag::Staging));
            let mut api = pg(API, EnvTag::Dev);
            api.engine = DbmsKind::MySql;
            api.database = Some("api".into());
            save(cx, api);
        }
    }

    // ## m4b-t14-tabs
    /// A state that connects for real (over the M3a fakes), so its capsule shows.
    pub(super) fn connects(self) -> bool {
        self == M4b::ScriptConnectedProd
    }

    /// A script tab state (Task 14).
    fn is_script(self) -> bool {
        matches!(
            self,
            M4b::ScriptTabs
                | M4b::ScriptChanged
                | M4b::ScriptDeleted
                | M4b::ScriptUnlisted
                | M4b::ScriptRunRefused
                | M4b::ScriptRemoved
                | M4b::ScriptUnsaved
                | M4b::ScriptConnectedProd
        )
    }

    // ## m4b-t15-goto
    /// A Go to script state (Task 15).
    fn is_goto(self) -> bool {
        matches!(
            self,
            M4b::GotoResults
                | M4b::GotoChooser
                | M4b::GotoProdOnly
                | M4b::GotoFocusedProd
                | M4b::GotoNoCandidate
        )
    }

    // ## m4b-t13-explorer
    /// The connection a Scripts state shows its library under.
    fn scripts_under(self) -> Option<&'static str> {
        match self {
            M4b::ScriptsProd | M4b::ScriptsDiagnostics | M4b::ScriptsTooltip => Some(scripts::PROD),
            M4b::ScriptsStaging
            | M4b::ScriptsFileMenu
            | M4b::ScriptsNameField
            | M4b::ScriptsTrashed => Some(scripts::STAGING),
            M4b::ScriptsEngines => Some(scripts::ENGINES),
            M4b::T2Preview | M4b::PasteConfirm => None,
            // ## m4b-t14-tabs
            M4b::ScriptTabs
            | M4b::ScriptChanged
            | M4b::ScriptDeleted
            | M4b::ScriptUnlisted
            | M4b::ScriptRunRefused
            | M4b::ScriptRemoved
            | M4b::ScriptUnsaved
            | M4b::ScriptConnectedProd => None,
            // ## m4b-t15-goto
            M4b::GotoResults
            | M4b::GotoChooser
            | M4b::GotoProdOnly
            | M4b::GotoFocusedProd
            | M4b::GotoNoCandidate => None,
        }
    }

    /// The steps after [`M4b::open`].
    pub(super) fn steps(self) -> Vec<Step> {
        // ## m4b-t13-explorer
        if self == M4b::ScriptsEngines {
            return scripts::engine_steps();
        }
        if let Some(under) = self.scripts_under() {
            return scripts::steps(self, under);
        }
        // ## m4b-t14-tabs
        if self.is_script() {
            return script_steps(self);
        }
        // ## m4b-t15-goto
        if self.is_goto() {
            return goto_steps(self);
        }
        match self {
            M4b::T2Preview => vec![
                // The console over the file's first 2.5 MB (what the host reads for a preview).
                step(
                    |window, cx| {
                        let Some(id) = profile_named(cx, DEV) else {
                            return;
                        };
                        console_fixtures::open(id, &dump()[..shown()], window, cx);
                    },
                    |window, cx| {
                        editor_in(window, cx)
                            .is_some_and(|e| e.read(cx).snapshot().buffer.len_bytes() == shown())
                    },
                ),
                // Then the editor becomes its preview.
                step(
                    |window, cx| {
                        let Some(editor) = editor_in(window, cx) else {
                            return;
                        };
                        editor.update(cx, |e, cx| {
                            e.set_preview(
                                PreviewSpec {
                                    shown_bytes: shown() as u64,
                                    total_bytes: T2_TOTAL,
                                },
                                cx,
                            );
                        });
                        let focus = editor.read(cx).focus_handle(cx);
                        window.focus(&focus, cx);
                    },
                    |window, cx| {
                        editor_in(window, cx)
                            .is_some_and(|e| e.read(cx).status().tier == Tier::Preview)
                    },
                ),
            ],
            // ## m4b-t11-editor (fix round 1, review m8): the question over the editor being
            // pasted into (a console over the dump's first lines, focused), as the mock shows.
            M4b::PasteConfirm => vec![
                step(
                    |window, cx| {
                        let Some(id) = profile_named(cx, DEV) else {
                            return;
                        };
                        console_fixtures::open(id, PASTED_INTO, window, cx);
                    },
                    |window, cx| {
                        editor_in(window, cx).is_some_and(|e| {
                            e.read(cx).snapshot().buffer.len_bytes() == PASTED_INTO.len()
                        })
                    },
                ),
                step(
                    |window, cx| {
                        if let Some(editor) = editor_in(window, cx) {
                            let focus = editor.read(cx).focus_handle(cx);
                            window.focus(&focus, cx);
                        }
                        let Some(ws) = window.root::<Workspace>().flatten() else {
                            return;
                        };
                        let spec = clipboard::paste_confirm_dialog(PASTE_BYTES, PASTE_TOTAL);
                        assert_eq!(spec.size, DialogSize::Standard, "the 420 px card");
                        let dialog = ws.update(cx, |ws, cx| ws.open_dialog(spec, window, cx));
                        dialog.update(cx, |d, _| d.set_role(Role::AlertDialog));
                    },
                    |window, cx| {
                        window
                            .root::<Workspace>()
                            .flatten()
                            .is_some_and(|ws| ws.read(cx).layer_ids().contains(&"dialog"))
                    },
                ),
            ],
            // ## m4b-t13-explorer: reached above.
            M4b::ScriptsProd
            | M4b::ScriptsStaging
            | M4b::ScriptsFileMenu
            | M4b::ScriptsDiagnostics
            | M4b::ScriptsNameField
            | M4b::ScriptsTooltip
            | M4b::ScriptsEngines
            | M4b::ScriptsTrashed => Vec::new(),
            // ## m4b-t14-tabs: reached through `script_steps` above.
            M4b::ScriptTabs
            | M4b::ScriptChanged
            | M4b::ScriptDeleted
            | M4b::ScriptUnlisted
            | M4b::ScriptRunRefused
            | M4b::ScriptRemoved
            | M4b::ScriptUnsaved
            | M4b::ScriptConnectedProd => Vec::new(),
            // ## m4b-t15-goto: reached through `goto_steps` above.
            M4b::GotoResults
            | M4b::GotoChooser
            | M4b::GotoProdOnly
            | M4b::GotoFocusedProd
            | M4b::GotoNoCandidate => Vec::new(),
        }
    }
}

// ## m4b-t14-tabs ---------------------------------------------------------------------------------

/// The connections of the script states (surface 5).
const PROD: &str = "orders-prod";
const STAGING: &str = "orders-staging";
const API: &str = "api-dev";
/// The script the states open.
const REPORT: &str = "orders_report.sql";
const REPORT_TEXT: &str = "-- haftalık sipariş raporu\nSELECT city, sum(total)\nFROM orders\nWHERE city = 'İzmir'\nGROUP BY city;\n";

/// The shots' library: a folder beside the app's own (never inside them), attached to a
/// service whose watcher is a fake (nothing changes on disk during a shot).
#[derive(Clone, Copy)]
struct ShotLibrary(hacivat_scripts::LibraryId);

impl hacivat_ui_core::gpui_surface::Global for ShotLibrary {}

fn shot_library(cx: &mut App) -> Option<hacivat_scripts::LibraryId> {
    if let Some(lib) = cx.try_global::<ShotLibrary>() {
        return Some(lib.0);
    }
    let paths = cx.try_global::<hacivat_app::AppState>()?.paths.clone();
    let root = paths.config.parent()?.join("library");
    std::fs::create_dir_all(&root).ok()?;
    std::fs::write(root.join(REPORT), REPORT_TEXT).ok()?;
    std::fs::write(root.join("kampanya_özet.sql"), "SELECT 1;\n").ok()?;
    let root = std::fs::canonicalize(&root).ok()?;
    let handle = cx.global::<ConnectionsModel>().runtime();
    let (_, watch) = hacivat_scripts::testing::FakeEventSource::new();
    let service = std::sync::Arc::new(hacivat_scripts::service::LibraryService::open_with(
        &paths,
        &handle,
        watch.sources(),
        hacivat_scripts::service::ServiceOptions {
            home: None,
            ..Default::default()
        },
    ));
    hacivat_app::scripts::open::install_service(service.clone(), cx);
    let id = futures::executor::block_on(service.attach(
        root,
        Some(hacivat_scripts::LogicalDb::parse("orders").ok()?),
        false,
    ))
    .ok()?;
    cx.set_global(ShotLibrary(id));
    Some(id)
}

/// The active tab's script doc, once its file is shown.
fn ready_doc(
    window: &mut Window,
    cx: &App,
) -> Option<hacivat_ui_core::gpui_surface::Entity<hacivat_app::scripts::tabs::ScriptDoc>> {
    let tab = window
        .root::<Workspace>()
        .flatten()?
        .read(cx)
        .active_tab()?;
    let doc = hacivat_app::scripts::tabs::doc_of_tab(tab, cx)?;
    (*doc.read(cx).load() == hacivat_app::scripts::tabs::Load::Ready).then_some(doc)
}

/// Opens `file` under `profile` (`None`: unbound) once the library lists it, then waits for its
/// tab to show it.
fn open_step(file: &'static str, profile: Option<&'static str>, db: Option<&'static str>) -> Step {
    step(
        move |_, cx| {
            let Some(lib) = shot_library(cx) else {
                return;
            };
            let profile = profile.and_then(|name| profile_named(cx, name));
            hacivat_app::scripts::open::open_script(
                cx,
                lib,
                hacivat_scripts::RelPath::parse(file).expect("a path"),
                profile,
                db.map(hacivat_scripts::DatabaseName::new),
            );
        },
        move |window, cx| {
            ready_doc(window, cx)
                .is_some_and(|d| d.read(cx).spec(cx).is_some_and(|s| s.rel.as_str() == file))
        },
    )
}

/// Types into the active script tab (it becomes dirty).
fn edit_step() -> Step {
    step(
        |window, cx| {
            if let Some(editor) = editor_in(window, cx) {
                let text = format!("{REPORT_TEXT}-- bu hafta: İstanbul da\n");
                editor.update(cx, |e, cx| e.set_text(&text, cx));
                let focus = editor.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }
        },
        |window, cx| ready_doc(window, cx).is_some_and(|d| d.read(cx).is_dirty(cx)),
    )
}

/// Shows `notice` on the active script tab.
fn notice_step(notice: hacivat_app::scripts::tabs::Notice) -> Step {
    let shown = notice.clone();
    step(
        move |window, cx| {
            let Some(tab) = window
                .root::<Workspace>()
                .flatten()
                .and_then(|ws| ws.read(cx).active_tab())
            else {
                return;
            };
            hacivat_app::scripts::tabs::fixtures::show(tab, notice.clone(), cx);
        },
        move |window, cx| {
            ready_doc(window, cx).is_some_and(|d| d.read(cx).notices().contains(&shown))
        },
    )
}

fn script_steps(state: M4b) -> Vec<Step> {
    use hacivat_app::scripts::tabs::{Notice, RunRefused, Unlisted};
    let mut steps = Vec::new();
    match state {
        M4b::ScriptTabs => {
            steps.push(open_step(REPORT, Some(PROD), Some("orders")));
            steps.push(open_step(REPORT, Some(STAGING), Some("orders")));
        }
        M4b::ScriptChanged => {
            steps.push(open_step(REPORT, Some(STAGING), Some("orders")));
            steps.push(edit_step());
            steps.push(notice_step(Notice::Changed));
        }
        M4b::ScriptDeleted => {
            steps.push(open_step(REPORT, Some(STAGING), Some("orders")));
            steps.push(notice_step(Notice::Deleted));
        }
        M4b::ScriptUnlisted => {
            steps.push(open_step(REPORT, Some(STAGING), Some("orders")));
            steps.push(notice_step(Notice::Unlisted(Unlisted::Env {
                folder: "@production".into(),
            })));
        }
        M4b::ScriptRunRefused => {
            steps.push(open_step("kampanya_özet.sql", Some(API), Some("kampanya")));
            steps.push(notice_step(Notice::RunRefused(RunRefused::Mysql {
                bound: "kampanya".into(),
                current: "api".into(),
            })));
        }
        M4b::ScriptRemoved => {
            steps.push(open_step(REPORT, None, None));
            steps.push(notice_step(Notice::Removed));
        }
        M4b::ScriptUnsaved => {
            steps.push(open_step(REPORT, Some(STAGING), Some("orders")));
            steps.push(edit_step());
            steps.push(step(
                |window, cx| {
                    hacivat_ui::actions::ActionRegistry::run("workspace.close_tab", window, cx);
                },
                |window, cx| {
                    window
                        .root::<Workspace>()
                        .flatten()
                        .is_some_and(|ws| ws.read(cx).layer_ids().contains(&"dialog"))
                },
            ));
        }
        M4b::ScriptConnectedProd => {
            steps.push(step(
                |_, cx| {
                    if let Some(id) = profile_named(cx, PROD) {
                        ConnectionsModel::connect(cx, id, None);
                    }
                },
                |_, cx| {
                    profile_named(cx, PROD).is_some_and(|id| {
                        matches!(
                            cx.global::<ConnectionsModel>().live(id),
                            hacivat_session::LiveState::Connected(_)
                        )
                    })
                },
            ));
            steps.push(open_step(REPORT, Some(PROD), Some("orders")));
        }
        M4b::T2Preview | M4b::PasteConfirm => {}
        // Task 13's states: `scripts::steps`.
        M4b::ScriptsProd
        | M4b::ScriptsStaging
        | M4b::ScriptsFileMenu
        | M4b::ScriptsDiagnostics
        | M4b::ScriptsNameField
        | M4b::ScriptsTooltip
        | M4b::ScriptsEngines
        | M4b::ScriptsTrashed => {}
        // ## m4b-t15-goto: `goto_steps`.
        M4b::GotoResults
        | M4b::GotoChooser
        | M4b::GotoProdOnly
        | M4b::GotoFocusedProd
        | M4b::GotoNoCandidate => {}
    }
    steps
}
// ## m4b-t14-tabs

// ## m4b-t15-goto ---------------------------------------------------------------------------------
/// Go to script's labels are drawn here.
const GOTO: &str = "hacivat-app/src/scripts/goto.rs";

/// The Go-to-script states' libraries: `orders` (bound to the database of orders-prod,
/// orders-staging and orders-dev) and `misc` (bound to no connection), on a service whose
/// watcher is a fake.
#[derive(Clone, Copy)]
struct GotoLibraries(hacivat_scripts::LibraryId);

impl hacivat_ui_core::gpui_surface::Global for GotoLibraries {}

fn goto_libraries(cx: &mut App) -> Option<hacivat_scripts::LibraryId> {
    if let Some(lib) = cx.try_global::<GotoLibraries>() {
        return Some(lib.0);
    }
    let paths = cx.try_global::<hacivat_app::AppState>()?.paths.clone();
    let base = paths.config.parent()?.to_path_buf();
    // Without a manifest a library binds its folder's name.
    let orders = base.join("goto").join("orders");
    for (rel, text) in [
        (REPORT, REPORT_TEXT),
        ("reports/daily_revenue.sql", "SELECT 1;\n"),
        ("reports/müşteri_segment.sql", "SELECT 2;\n"),
        ("@production/purge_old.sql", "SELECT 3;\n"),
    ] {
        let p = orders.join(rel);
        std::fs::create_dir_all(p.parent()?).ok()?;
        std::fs::write(p, text).ok()?;
    }
    let misc = base.join("goto").join("misc");
    std::fs::create_dir_all(&misc).ok()?;
    std::fs::write(misc.join("todo.sql"), "SELECT 4;\n").ok()?;
    let (orders, misc) = (
        std::fs::canonicalize(&orders).ok()?,
        std::fs::canonicalize(&misc).ok()?,
    );
    let handle = cx.global::<ConnectionsModel>().runtime();
    let (_, watch) = hacivat_scripts::testing::FakeEventSource::new();
    let service = std::sync::Arc::new(hacivat_scripts::service::LibraryService::open_with(
        &paths,
        &handle,
        watch.sources(),
        hacivat_scripts::service::ServiceOptions {
            home: None,
            ..Default::default()
        },
    ));
    hacivat_app::scripts::open::install_service(service.clone(), cx);
    let id = futures::executor::block_on(service.attach(
        orders,
        Some(hacivat_scripts::LogicalDb::parse("orders").ok()?),
        false,
    ))
    .ok()?;
    futures::executor::block_on(service.attach(
        misc,
        Some(hacivat_scripts::LogicalDb::parse("misc").ok()?),
        false,
    ))
    .ok()?;
    cx.set_global(GotoLibraries(id));
    Some(id)
}

/// The open Go-to-script layer's view.
fn goto_host(
    cx: &App,
) -> Option<hacivat_ui_core::gpui_surface::Entity<hacivat_app::scripts::goto::GotoHost>> {
    hacivat_app::scripts::goto::open_host(cx)
}

/// Go to script's result row that starts with `path`, picked (Enter on it).
fn goto_pick_step(path: &'static str) -> Step {
    step(
        move |window, cx| {
            let Some(host) = goto_host(cx) else {
                return;
            };
            let ix = host
                .read(cx)
                .result_rows()
                .iter()
                .position(|r| r.starts_with(path));
            if let Some(ix) = ix {
                host.update(cx, |h, cx| h.confirm_result(ix, window, cx));
            }
        },
        |_, cx| goto_host(cx).is_some_and(|h| h.read(cx).is_choosing()),
    )
}

fn goto_steps(state: M4b) -> Vec<Step> {
    let mut steps = vec![step(
        |_, cx| {
            goto_libraries(cx);
        },
        |_, cx| {
            hacivat_app::scripts::open::service(cx).is_some_and(|s| {
                let snap = s.snapshot();
                snap.libraries.len() == 2
                    && snap.libraries.iter().all(|l| {
                        snap.view(Some(EnvTag::Production))
                            .library(l.id)
                            .is_some_and(|v| v.file_count() > 0)
                    })
            })
        },
    )];
    if state == M4b::GotoFocusedProd {
        // The user works on production: its script tab is the active one.
        steps.push(step(
            |_, cx| {
                let Some(lib) = goto_libraries(cx) else {
                    return;
                };
                let prod = profile_named(cx, PROD);
                hacivat_app::scripts::open::open_script(
                    cx,
                    lib,
                    hacivat_scripts::RelPath::parse(REPORT).expect("a path"),
                    prod,
                    Some(hacivat_scripts::DatabaseName::new("orders")),
                );
            },
            |window, cx| ready_doc(window, cx).is_some(),
        ));
    }
    // ⇧⌘O, then the results of the empty query.
    steps.push(step(
        |window, cx| {
            hacivat_ui::actions::ActionRegistry::run("scripts.goto", window, cx);
        },
        |_, cx| goto_host(cx).is_some_and(|h| h.read(cx).result_rows().len() >= 5),
    ));
    match state {
        M4b::GotoChooser | M4b::GotoFocusedProd => {
            steps.push(goto_pick_step("reports/daily_revenue.sql"));
        }
        M4b::GotoProdOnly => steps.push(goto_pick_step("@production/purge_old.sql")),
        M4b::GotoNoCandidate => steps.push(goto_pick_step("todo.sql")),
        _ => {}
    }
    steps
}
// ## m4b-t15-goto

/// Where the preview of [`dump`] ends (UI-EDIT-23 step 1).
fn shown() -> usize {
    preview_cut(dump(), TierLimits::default().preview_bytes)
}

/// A SQL dump a little past the preview's 2.5 MB, Turkish data in it.
fn dump() -> &'static str {
    static TEXT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TEXT.get_or_init(|| {
        let line = "INSERT INTO orders (id, city, total) VALUES (1204, 'İzmir', 1234567);\n";
        let n = usize::try_from(TierLimits::default().preview_bytes).unwrap_or(0) / line.len() + 8;
        format!("-- orders dump\n{}", line.repeat(n))
    })
}

fn pg(name: &str, env: EnvTag) -> Profile {
    Profile {
        name: name.to_string(),
        engine: DbmsKind::Postgres,
        environment: Some(env),
        database: Some("orders".into()),
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

// ## m4b-t13-explorer
/// The tree rows' labels are drawn here.
const ROW: &str = "hacivat-ui/src/tree/view/row.rs";

/// Task 13's Scripts states: one library folder `orders` (a git repository: one file modified,
/// two new, one conflicted) bound to the database `orders` of orders-prod and orders-staging,
/// both with a cached catalog, read through a library service over fake event sources.
mod scripts {
    use std::path::Path;
    use std::sync::Arc;

    use hacivat_app::connections::explorer;
    use hacivat_app::explorer::adapter;
    use hacivat_app::scripts::{file_ops, projection};
    use hacivat_proto::{DbmsKind, EnvTag};
    use hacivat_scripts::service::{FolderTrash, LibraryService, ServiceOptions};
    use hacivat_scripts::testing::FakeHandle;
    use hacivat_scripts::{GitSummary, LibraryState};
    use hacivat_ui::actions::ActionRegistry;
    use hacivat_ui_core::gpui_surface::{App, Window};

    use super::super::m3a::{Step, step};
    use super::super::support::catalogs::{catalog, schema, table};
    use super::{M4b, pg, profile_named, save};

    pub(super) const PROD: &str = "orders-prod";
    pub(super) const STAGING: &str = "orders-staging";
    /// The MySQL and SQLite state's marker for `scripts_under`.
    pub(super) const ENGINES: &str = "api-dev";

    /// The git CLI on the fixture repository this file made (never in product code).
    fn git(dir: &Path, args: &[&str]) -> bool {
        std::process::Command::new("git")
            .args([
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    fn write(dir: &Path, rel: &str, text: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().expect("a parent")).expect("the folder");
        std::fs::write(p, text).expect("the file");
    }

    /// The library folder: committed, then a file edited, two added, one in conflict.
    fn fixture(diagnostics: bool, outside: bool) -> std::path::PathBuf {
        let dir = super::super::support::temp_root().join("orders");
        std::fs::create_dir_all(&dir).expect("the library folder");
        for f in [
            "müşteri_segment.sql",
            "conflict.sql",
            "@production/drop_old.sql",
            "README.md",
        ] {
            write(&dir, f, "SELECT 1;\n");
        }
        git(&dir, &["init", "-q", "-b", "main"]);
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "fixture"]);
        // A conflict on conflict.sql.
        git(&dir, &["checkout", "-q", "-b", "other"]);
        write(&dir, "conflict.sql", "SELECT 2;\n");
        git(&dir, &["commit", "-q", "-am", "theirs"]);
        git(&dir, &["checkout", "-q", "main"]);
        write(&dir, "conflict.sql", "SELECT 3;\n");
        git(&dir, &["commit", "-q", "-am", "ours"]);
        let _ = git(&dir, &["merge", "-q", "other"]);
        write(&dir, "müşteri_segment.sql", "SELECT 4;\n");
        write(&dir, "reports/haftalık_rapor.sql", "SELECT 5;\n");
        write(&dir, "orders_report.sql", "SELECT 6;\n");
        if diagnostics {
            write(&dir, "@prod/typo.sql", "SELECT 7;\n");
        }
        // ## m4b-t13-explorer fix round 1: a link pointing outside the library (listed dim).
        #[cfg(unix)]
        if outside {
            let away = super::super::support::temp_root().join("secret.sql");
            std::fs::write(&away, "SELECT 8;\n").expect("the outside file");
            std::os::unix::fs::symlink(&away, dir.join("outside.sql")).expect("the link");
        }
        #[cfg(not(unix))]
        let _ = outside;
        std::fs::canonicalize(&dir).expect("canonical")
    }

    /// Saves both connections with their cached catalogs and starts the library service.
    pub(super) fn open(state: M4b, under: &'static str, window: &mut Window, cx: &mut App) {
        let diagnostics = state == M4b::ScriptsDiagnostics;
        let outside = state == M4b::ScriptsTooltip;
        let cat = catalog(
            DbmsKind::Postgres,
            vec![schema(
                DbmsKind::Postgres,
                &["orders", "public"],
                false,
                vec![table("orders", Some(16_400), &[("id", "bigint")])],
            )],
        );
        for (name, env) in [(PROD, EnvTag::Production), (STAGING, EnvTag::Staging)] {
            let id = save(cx, pg(name, env));
            adapter::set_catalog(id, Some(cat.clone()), None, cx);
        }
        let dir = fixture(diagnostics, outside);
        start_service(vec![dir], cx);
        let id = profile_named(cx, under);
        hacivat_app::connections::ConnectionsModel::select(cx, id);
        explorer::focus_list(window, cx);
    }

    /// Starts a library service over fake event sources (and a test trash, so Move to Trash
    /// never reaches the user's) and attaches `dirs`.
    fn start_service(dirs: Vec<std::path::PathBuf>, cx: &mut App) {
        let paths = cx.global::<hacivat_app::AppState>().paths.clone();
        let rt = cx
            .global::<hacivat_app::connections::ConnectionsModel>()
            .runtime();
        let trash = super::super::support::temp_root();
        let svc = Arc::new(LibraryService::open_with(
            &paths,
            &rt,
            FakeHandle::default().sources(),
            ServiceOptions {
                git_debounce: std::time::Duration::from_millis(40),
                home: None,
                trasher: Arc::new(FolderTrash(trash)),
                ..ServiceOptions::default()
            },
        ));
        projection::set_service(svc.clone(), cx);
        cx.spawn(async move |_| {
            for dir in dirs {
                let _ = svc.attach(dir, None, false).await;
            }
        })
        .detach();
    }

    /// The library is listed with its repository status.
    fn listed(cx: &App) -> bool {
        projection::service(cx).is_some_and(|s| {
            let snap = s.snapshot();
            snap.libraries.first().is_some_and(|l| {
                snap.view(None).library(l.id).is_some_and(|v| {
                    v.library().state == LibraryState::Watching && v.git() == GitSummary::Ready
                })
            })
        })
    }

    fn key(cx: &App, label: &str) -> Option<hacivat_ui::tree::TreeKey> {
        explorer::shown_rows(cx)
            .into_iter()
            .find(|r| r.status.is_none() && super::super::strip(&r.data.label) == label)
            .map(|r| r.key)
    }

    /// The Scripts node under `under`'s `orders` (its label is localized: found by key).
    fn scripts_node(cx: &App, under: &str) -> Option<hacivat_ui::tree::TreeKey> {
        let id = profile_named(cx, under)?;
        let k = projection::key_for(id, "orders", None, None);
        explorer::shown_rows(cx)
            .into_iter()
            .any(|r| r.key == k)
            .then_some(k)
    }

    fn expand_key(
        find: impl Fn(&App) -> Option<hacivat_ui::tree::TreeKey> + 'static,
        then: impl Fn(&App) -> bool + 'static,
    ) -> Step {
        step(
            move |_, cx| {
                if let (Some(k), Some(tree)) = (find(cx), explorer::tree(cx)) {
                    tree.update(cx, |t, cx| t.set_expanded(k, true, cx));
                }
            },
            move |_, cx| then(cx),
        )
    }

    fn expand(label: &'static str, then: &'static str) -> Step {
        expand_key(move |cx| key(cx, label), move |cx| key(cx, then).is_some())
    }

    fn has_layer(window: &Window, cx: &App, id: &str) -> bool {
        window
            .root::<hacivat_app::shell::Workspace>()
            .flatten()
            .is_some_and(|ws| ws.read(cx).layer_ids().contains(&id))
    }

    pub(super) fn steps(state: M4b, under: &'static str) -> Vec<Step> {
        let mut out = vec![
            step(|_, _| {}, |_, cx| listed(cx)),
            expand(under, "orders"),
            expand_key(
                |cx| key(cx, "orders"),
                move |cx| scripts_node(cx, under).is_some(),
            ),
            expand_key(
                move |cx| scripts_node(cx, under),
                |cx| key(cx, "reports").is_some(),
            ),
            expand("reports", "haftalık_rapor.sql"),
        ];
        if under == PROD {
            out.push(expand("@production", "drop_old.sql"));
        }
        match state {
            M4b::ScriptsFileMenu => out.push(step(
                |window, cx| {
                    let Some(k) = key(cx, "orders_report.sql") else {
                        return;
                    };
                    let at = explorer::tree(cx)
                        .and_then(|t| t.read(cx).row_bounds(k))
                        .map(|b| b.bottom_left())
                        .unwrap_or_default();
                    hacivat_app::explorer::open_node_menu(k, at, window, cx);
                },
                |window, cx| has_layer(window, cx, "menu"),
            )),
            M4b::ScriptsDiagnostics => out.push(step(
                |window, cx| {
                    let Some(k) = explorer::shown_rows(cx)
                        .into_iter()
                        .find(|r| r.data.icon == Some(hacivat_ui::icon::IconName::CircleAlert))
                        .map(|r| r.key)
                    else {
                        return;
                    };
                    if let Some(tree) = explorer::tree(cx) {
                        tree.update(cx, |t, cx| t.select(k, cx));
                    }
                    let owner = hacivat_app::explorer::contrib::contributors(cx)
                        .into_iter()
                        .next();
                    if let Some(owner) = owner {
                        owner.activate(k, window, cx);
                    }
                },
                |window, cx| {
                    has_layer(
                        window,
                        cx,
                        hacivat_app::scripts::diagnostics::DIAGNOSTICS_LAYER,
                    )
                },
            )),
            // ## m4b-t13-explorer fix round 1 (review m7)
            M4b::ScriptsNameField => {
                out.push(step(
                    |window, cx| {
                        select(cx, "orders_report.sql");
                        ActionRegistry::run("scripts.rename", window, cx);
                    },
                    |window, cx| file_ops::open_field(window, cx).is_some(),
                ));
                out.push(step(
                    |window, cx| {
                        if let Some(f) = file_ops::open_field(window, cx) {
                            f.update(cx, |f, cx| f.type_for_tests("../orders_report.sql", cx));
                        }
                    },
                    |window, cx| {
                        file_ops::open_field(window, cx)
                            .is_some_and(|f| f.read(cx).error().is_some())
                    },
                ));
            }
            M4b::ScriptsTooltip => out.push(step(
                |window, cx| {
                    let Some(k) = scripts_node(cx, PROD) else {
                        return;
                    };
                    let Some(b) = explorer::tree(cx).and_then(|t| t.read(cx).row_bounds(k)) else {
                        return;
                    };
                    use gpui::InputEvent as _;
                    window.dispatch_event(
                        gpui::MouseMoveEvent {
                            // Over the row's icon: away from the name, whose own tooltip (a cut
                            // name's full text) would take the hover in a long locale.
                            position: gpui::point(b.left() + gpui::px(58.0), b.center().y),
                            pressed_button: None,
                            modifiers: gpui::Modifiers::none(),
                        }
                        .to_platform_input(),
                        cx,
                    );
                },
                |_, cx| {
                    cx.try_global::<hacivat_ui::text::TextRegistry>()
                        .is_some_and(|r| {
                            r.frame
                                .iter()
                                .any(|t| t.path.contains("hacivat-ui/src/popover.rs:"))
                        })
                },
            )),
            M4b::ScriptsTrashed => out.push(step(
                |window, cx| {
                    select(cx, "orders_report.sql");
                    ActionRegistry::run("scripts.trash", window, cx);
                },
                |_, cx| !hacivat_ui::toast::ToastQueue::visible_ids(cx).is_empty(),
            )),
            M4b::ScriptsProd
            | M4b::ScriptsStaging
            | M4b::ScriptsEngines
            | M4b::T2Preview
            | M4b::PasteConfirm => {}
            // ## m4b-t14-tabs: Task 14's states (`script_steps`).
            M4b::ScriptTabs
            | M4b::ScriptChanged
            | M4b::ScriptDeleted
            | M4b::ScriptUnlisted
            | M4b::ScriptRunRefused
            | M4b::ScriptRemoved
            | M4b::ScriptUnsaved
            | M4b::ScriptConnectedProd => {}
            // ## m4b-t15-goto: Task 15's states (`goto_steps`).
            M4b::GotoResults
            | M4b::GotoChooser
            | M4b::GotoProdOnly
            | M4b::GotoFocusedProd
            | M4b::GotoNoCandidate => {}
        }
        out
    }

    /// Puts the tree's cursor on the row labelled `label`.
    fn select(cx: &mut App, label: &str) {
        if let (Some(k), Some(tree)) = (key(cx, label), explorer::tree(cx)) {
            tree.update(cx, |t, cx| t.select(k, cx));
        }
    }

    /// MySQL `api-dev` (databases `api` and `kampanya`; a library bound to `kampanya`) and
    /// SQLite `local-file` (`/x/orders.db`; a library bound to `orders`).
    pub(super) fn open_engines(window: &mut Window, cx: &mut App) {
        let mut my = pg(ENGINES, EnvTag::Dev);
        my.engine = DbmsKind::MySql;
        my.database = Some("api".into());
        let my_id = save(cx, my);
        adapter::set_catalog(
            my_id,
            Some(catalog(
                DbmsKind::MySql,
                ["api", "kampanya"]
                    .iter()
                    .map(|db| {
                        schema(
                            DbmsKind::MySql,
                            &[db],
                            false,
                            vec![table("t", None, &[("id", "int")])],
                        )
                    })
                    .collect(),
            )),
            None,
            cx,
        );
        let mut lite = pg("local-file", EnvTag::Local);
        lite.engine = DbmsKind::Sqlite;
        lite.database = None;
        lite.endpoint = hacivat_session::profile::ProfileEndpoint::File {
            path: "/x/orders.db".into(),
            create_if_missing: false,
        };
        let lite_id = save(cx, lite);
        adapter::set_catalog(
            lite_id,
            Some(catalog(
                DbmsKind::Sqlite,
                vec![schema(
                    DbmsKind::Sqlite,
                    &["main"],
                    false,
                    vec![table("t", None, &[("id", "integer")])],
                )],
            )),
            None,
            cx,
        );
        let root = super::super::support::temp_root();
        let mut dirs = Vec::new();
        for (name, file) in [
            ("kampanya", "kampanya_raporu.sql"),
            ("orders", "günlük.sql"),
        ] {
            let dir = root.join(name);
            write(&dir, file, "SELECT 1;\n");
            dirs.push(std::fs::canonicalize(&dir).expect("canonical"));
        }
        start_service(dirs, cx);
        hacivat_app::connections::ConnectionsModel::select(cx, Some(my_id));
        explorer::focus_list(window, cx);
    }

    /// Both libraries listed.
    fn both_listed(cx: &App) -> bool {
        projection::service(cx).is_some_and(|s| {
            let snap = s.snapshot();
            snap.libraries.len() == 2
                && snap.libraries.iter().all(|l| {
                    snap.view(None).library(l.id).is_some_and(|v| {
                        v.library().state == LibraryState::Watching
                            && v.git() != GitSummary::Pending
                    })
                })
        })
    }

    /// The Scripts node of `profile`'s `database`, when shown.
    fn node_of(cx: &App, profile: &str, database: &str) -> Option<hacivat_ui::tree::TreeKey> {
        let id = profile_named(cx, profile)?;
        let k = projection::key_for(id, database, None, None);
        explorer::shown_rows(cx)
            .into_iter()
            .any(|r| r.key == k)
            .then_some(k)
    }

    pub(super) fn engine_steps() -> Vec<Step> {
        vec![
            step(|_, _| {}, |_, cx| both_listed(cx)),
            expand(ENGINES, "kampanya"),
            expand_key(
                |cx| key(cx, "kampanya"),
                |cx| node_of(cx, ENGINES, "kampanya").is_some(),
            ),
            expand_key(
                |cx| node_of(cx, ENGINES, "kampanya"),
                |cx| key(cx, "kampanya_raporu.sql").is_some(),
            ),
            expand_key(
                |cx| key(cx, "local-file"),
                |cx| node_of(cx, "local-file", "main").is_some(),
            ),
            expand_key(
                |cx| node_of(cx, "local-file", "main"),
                |cx| key(cx, "günlük.sql").is_some(),
            ),
        ]
    }
}
// ## m4b-t13-explorer
