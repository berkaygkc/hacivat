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
    // ## m4b-t14-tabs
}

/// Every M4b state, in shot order.
// ## m4b-t14-tabs (named exception): Task 14's states after Task 11's.
pub(super) const M4B_STATES: [M4b; 9] = [
    M4b::T2Preview,
    M4b::PasteConfirm,
    M4b::ScriptTabs,
    M4b::ScriptChanged,
    M4b::ScriptDeleted,
    M4b::ScriptUnlisted,
    M4b::ScriptRunRefused,
    M4b::ScriptRemoved,
    M4b::ScriptUnsaved,
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
            // ## m4b-t14-tabs
            M4b::ScriptTabs => "m4b-script-tabs",
            M4b::ScriptChanged => "m4b-script-changed",
            M4b::ScriptDeleted => "m4b-script-deleted",
            M4b::ScriptUnlisted => "m4b-script-unlisted",
            M4b::ScriptRunRefused => "m4b-script-run-refused",
            M4b::ScriptRemoved => "m4b-script-removed",
            M4b::ScriptUnsaved => "m4b-script-unsaved",
        }
    }

    /// Proof that the state is on screen (a label from this file, with this text).
    pub(super) fn marker(self) -> (&'static str, Option<&'static str>) {
        match self {
            M4b::T2Preview => ("#editor-t2-title", None),
            M4b::PasteConfirm => ("hacivat-ui/src/dialog.rs:", None),
            // ## m4b-t14-tabs
            M4b::ScriptTabs => ("#tab-title", None),
            M4b::ScriptUnsaved => ("hacivat-ui/src/dialog.rs:", None),
            M4b::ScriptChanged
            | M4b::ScriptDeleted
            | M4b::ScriptUnlisted
            | M4b::ScriptRunRefused
            | M4b::ScriptRemoved => ("hacivat-ui/src/notice.rs:", None),
        }
    }

    /// Labels (`#id`) that must be visible too.
    pub(super) fn also_shown(self) -> &'static [&'static str] {
        match self {
            M4b::T2Preview => &["#editor-t2-body"],
            M4b::PasteConfirm => &[],
            // ## m4b-t14-tabs
            M4b::ScriptChanged
            | M4b::ScriptDeleted
            | M4b::ScriptUnlisted
            | M4b::ScriptRunRefused
            | M4b::ScriptRemoved
            | M4b::ScriptTabs
            | M4b::ScriptUnsaved => &[],
        }
    }

    /// Brings the state's fixtures on screen in a freshly started shell.
    pub(super) fn open(self, _window: &mut Window, cx: &mut App) {
        save(cx, pg(DEV, EnvTag::Dev));
        // ## m4b-t14-tabs: the connections a script is open under.
        if self.is_script() {
            save(cx, pg(PROD, EnvTag::Production));
            save(cx, pg(STAGING, EnvTag::Staging));
            let mut api = pg(API, EnvTag::Dev);
            api.engine = DbmsKind::MySql;
            api.database = Some("api".into());
            save(cx, api);
        }
    }

    // ## m4b-t14-tabs
    /// A script tab state (Task 14).
    fn is_script(self) -> bool {
        !matches!(self, M4b::T2Preview | M4b::PasteConfirm)
    }

    /// The steps after [`M4b::open`].
    pub(super) fn steps(self) -> Vec<Step> {
        // ## m4b-t14-tabs
        if self.is_script() {
            return script_steps(self);
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
            // ## m4b-t14-tabs: reached through `script_steps` above.
            _ => Vec::new(),
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
        M4b::T2Preview | M4b::PasteConfirm => {}
    }
    steps
}
// ## m4b-t14-tabs

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
