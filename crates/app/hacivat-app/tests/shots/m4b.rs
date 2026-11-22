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
    /// A file row's menu (Open, Open under another connection… disabled, Rename F2, Move to
    /// Trash, Reveal, Copy path, Copy relative path).
    ScriptsFileMenu,
    // 11 · Library diagnostics popover
    /// The problems row's popover: an unknown `@prod` folder, said in plain words.
    ScriptsDiagnostics,
    // ## m4b-t13-explorer
}

/// Every M4b state, in shot order.
// ## m4b-t13-explorer: Task 13's four after Task 11's two.
pub(super) const M4B_STATES: [M4b; 6] = [
    M4b::T2Preview,
    M4b::PasteConfirm,
    M4b::ScriptsProd,
    M4b::ScriptsStaging,
    M4b::ScriptsFileMenu,
    M4b::ScriptsDiagnostics,
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
        }
    }

    /// Brings the state's fixtures on screen in a freshly started shell.
    pub(super) fn open(self, window: &mut Window, cx: &mut App) {
        // ## m4b-t13-explorer
        if let Some(under) = self.scripts_under() {
            return scripts::open(under, self == M4b::ScriptsDiagnostics, window, cx);
        }
        save(cx, pg(DEV, EnvTag::Dev));
    }

    // ## m4b-t13-explorer
    /// The connection a Scripts state shows its library under.
    fn scripts_under(self) -> Option<&'static str> {
        match self {
            M4b::ScriptsProd | M4b::ScriptsDiagnostics => Some(scripts::PROD),
            M4b::ScriptsStaging | M4b::ScriptsFileMenu => Some(scripts::STAGING),
            M4b::T2Preview | M4b::PasteConfirm => None,
        }
    }

    /// The steps after [`M4b::open`].
    pub(super) fn steps(self) -> Vec<Step> {
        // ## m4b-t13-explorer
        if let Some(under) = self.scripts_under() {
            return scripts::steps(self, under);
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
            | M4b::ScriptsDiagnostics => Vec::new(),
        }
    }
}

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
    use hacivat_app::scripts::projection;
    use hacivat_proto::{DbmsKind, EnvTag};
    use hacivat_scripts::service::{LibraryService, ServiceOptions};
    use hacivat_scripts::testing::FakeHandle;
    use hacivat_scripts::{GitSummary, LibraryState};
    use hacivat_ui_core::gpui_surface::{App, Window};

    use super::super::m3a::{Step, step};
    use super::super::support::catalogs::{catalog, schema, table};
    use super::{M4b, pg, profile_named, save};

    pub(super) const PROD: &str = "orders-prod";
    pub(super) const STAGING: &str = "orders-staging";

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
    fn fixture(diagnostics: bool) -> std::path::PathBuf {
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
        std::fs::canonicalize(&dir).expect("canonical")
    }

    /// Saves both connections with their cached catalogs and starts the library service.
    pub(super) fn open(under: &'static str, diagnostics: bool, window: &mut Window, cx: &mut App) {
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
        let dir = fixture(diagnostics);
        let paths = cx.global::<hacivat_app::AppState>().paths.clone();
        let rt = cx
            .global::<hacivat_app::connections::ConnectionsModel>()
            .runtime();
        let svc = Arc::new(LibraryService::open_with(
            &paths,
            &rt,
            FakeHandle::default().sources(),
            ServiceOptions {
                git_debounce: std::time::Duration::from_millis(40),
                home: None,
                ..ServiceOptions::default()
            },
        ));
        projection::set_service(svc.clone(), cx);
        cx.spawn(async move |_| {
            let _ = svc.attach(dir, None, false).await;
        })
        .detach();
        let id = profile_named(cx, under);
        hacivat_app::connections::ConnectionsModel::select(cx, id);
        explorer::focus_list(window, cx);
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
            M4b::ScriptsProd | M4b::ScriptsStaging | M4b::T2Preview | M4b::PasteConfirm => {}
        }
        out
    }
}
// ## m4b-t13-explorer
