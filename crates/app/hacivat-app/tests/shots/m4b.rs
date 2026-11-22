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
}

/// Every M4b state, in shot order.
pub(super) const M4B_STATES: [M4b; 2] = [M4b::T2Preview, M4b::PasteConfirm];

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
        }
    }

    /// Proof that the state is on screen (a label from this file, with this text).
    pub(super) fn marker(self) -> (&'static str, Option<&'static str>) {
        match self {
            M4b::T2Preview => ("#editor-t2-title", None),
            M4b::PasteConfirm => ("hacivat-ui/src/dialog.rs:", None),
        }
    }

    /// Labels (`#id`) that must be visible too.
    pub(super) fn also_shown(self) -> &'static [&'static str] {
        match self {
            M4b::T2Preview => &["#editor-t2-body"],
            M4b::PasteConfirm => &[],
        }
    }

    /// Brings the state's fixtures on screen in a freshly started shell.
    pub(super) fn open(self, _window: &mut Window, cx: &mut App) {
        save(cx, pg(DEV, EnvTag::Dev));
    }

    /// The steps after [`M4b::open`].
    pub(super) fn steps(self) -> Vec<Step> {
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
