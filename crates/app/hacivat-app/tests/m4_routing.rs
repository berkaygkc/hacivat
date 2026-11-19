//! M4 Task 1b: every M4 command id is registered, titled in EN and TR, and bound as the plan's
//! table lists (VS Code's defaults where VS Code has one, PO-Q10; P-M4-9, -10); the M4 stubs are
//! disabled, so no palette row resolves to one and their chords pass through; M4 registers no
//! explorer contributor and no action provider, and the explorer model stub is empty. The DDL
//! view is a real tab kind, one tab per object (ruling R-W0-1).

mod support;

use std::rc::Rc;

use std::cell::RefCell;
use std::time::Duration;

use gpui::{TestAppContext, VisualTestContext};
use hacivat_app::ddl_view::{self, DDL_VIEW_CONTEXT, DdlViewError};
use hacivat_app::explorer::{
    self, ActionSlot, ActionSurface, ExplorerActionProvider, ExplorerModel, NodeRef,
    ProvidedAction, RevealOutcome, contributors, database_rows, open_data, provided_actions,
    register_action_provider, reveal, set_open_data_handler,
};
use hacivat_app::shell::TabKind;
use hacivat_meta::{ObjectKey, ObjectKind};
use hacivat_proto::ProfileId;
use hacivat_ui::actions::ActionRegistry;
use hacivat_ui::keymaps::{PRESETS, load};
use hacivat_ui::tree::{TreeKey, TreeModel as _};
use hacivat_ui_core::gpui_surface::App;
use hacivat_ui_core::keymap::{Keymap, KeymapPlatform, chord_string};
use support::*;

const ALL_PLATFORMS: [KeymapPlatform; 3] = [
    KeymapPlatform::MacOs,
    KeymapPlatform::Windows,
    KeymapPlatform::Linux,
];

/// The plan's table (Task 1b "Keymap rows"; P-M4-9): id, the context the command is registered
/// in, the context of its M4 row, the macOS chord, the Windows/Linux chord. `None`: no M4 row
/// on that platform (palette only, or an earlier row that stays).
type Row = (
    &'static str,
    &'static str,
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
);

const M4_ROWS: &[Row] = &[
    (
        "explorer.view_ddl",
        "Explorer",
        "Explorer",
        Some("f12"),
        Some("f12"),
    ),
    (
        "explorer.copy_qualified_name",
        "Explorer",
        "Explorer",
        Some("cmd-alt-c"),
        Some("shift-alt-c"),
    ),
    (
        "explorer.collapse_all",
        "Explorer",
        "Explorer",
        Some("cmd-left"),
        Some("ctrl-left"),
    ),
    // The M2a Workspace row (`cmd-f` / `ctrl-f`) stays; VS Code's `list.find` joins it on macOS
    // only (P-M4-9: `ctrl-alt-f` is AltGr+F, `@` on Turkish-F).
    (
        "explorer.focus_filter",
        "Workspace",
        "Explorer",
        Some("cmd-alt-f"),
        None,
    ),
    (
        "explorer.refresh",
        "Explorer",
        "Explorer",
        Some("f5"),
        Some("f5"),
    ),
    (
        "ddl_view.copy",
        "DdlView",
        "DdlView",
        Some("cmd-c"),
        Some("ctrl-c"),
    ),
    // Palette only. Enter on a table or view row is the tree's own activate (Open data); on a
    // profile row it stays `connection.toggle_connect`.
    ("explorer.open_data", "Explorer", "Explorer", None, None),
    (
        "explorer.expand_all_children",
        "Explorer",
        "Explorer",
        None,
        None,
    ),
    ("explorer.edit_scope", "Explorer", "Explorer", None, None),
    (
        "explorer.set_level.auto",
        "Explorer",
        "Explorer",
        None,
        None,
    ),
    (
        "explorer.set_level.names",
        "Explorer",
        "Explorer",
        None,
        None,
    ),
    (
        "explorer.set_level.structure",
        "Explorer",
        "Explorer",
        None,
        None,
    ),
    (
        "explorer.set_level.sources",
        "Explorer",
        "Explorer",
        None,
        None,
    ),
    ("metadata.clear_cache", "Explorer", "Explorer", None, None),
];

/// The command ids M4 registers (every M4 row except the M2a command it adds a chord to).
fn m4_ids() -> Vec<&'static str> {
    M4_ROWS
        .iter()
        .map(|r| r.0)
        .filter(|id| *id != "explorer.focus_filter")
        .collect()
}

/// `chord` in the keymap's canonical spelling.
fn canonical(platform: KeymapPlatform, chord: &str) -> String {
    let text = format!("[[binding]]\nchord = \"{chord}\"\ncommand = \"t.x\"\ncontext = \"T\"\n");
    let (km, diags) = Keymap::from_sources(platform, &text, "", &[], None);
    assert!(diags.is_empty(), "{chord}: {diags:?}");
    chord_string(&km.bindings()[0].chord)
}

/// The chords of `id` in `context` on `platform` with `preset`, layout overlays left out.
fn chords(platform: KeymapPlatform, preset: &str, id: &str, context: &str) -> Vec<String> {
    load(platform, preset, None)
        .0
        .bindings()
        .iter()
        .filter(|b| b.command == id && b.context == context && b.when_layout.is_none())
        .map(|b| chord_string(&b.chord))
        .collect()
}

fn registry(cx: &mut TestAppContext) -> ActionRegistry {
    cx.update(|cx| {
        hacivat_app::init(cx);
        cx.global::<ActionRegistry>().clone()
    })
}

const TR_UI: &str = include_str!("../../../../locales/tr/hacivat-ui.ftl");

#[gpui::test]
fn every_m4_command_has_a_binding_or_is_palette_only(cx: &mut TestAppContext) {
    let reg = registry(cx);
    let en = l10n_en();
    let tr_ids = message_ids_in(TR_UI);
    for (id, registered_in, row_context, mac, pc) in M4_ROWS {
        let def = reg
            .get(id)
            .unwrap_or_else(|| panic!("{id} is not registered"));
        assert_eq!(def.context, *registered_in, "{id}");
        assert_eq!(
            def.title_id,
            format!("hv-cmd-{}", id.replace(['.', '_'], "-")),
            "{id}"
        );
        assert!(
            !en.t_english(def.title_id).starts_with('⟦'),
            "{id}: no EN title"
        );
        assert!(
            tr_ids.iter().any(|t| t == def.title_id),
            "{id}: no TR title"
        );
        for preset in PRESETS {
            for (platform, want) in [
                (KeymapPlatform::MacOs, mac),
                (KeymapPlatform::Windows, pc),
                (KeymapPlatform::Linux, pc),
            ] {
                let want: Vec<String> = want.iter().map(|c| canonical(platform, c)).collect();
                assert_eq!(
                    chords(platform, preset, id, row_context),
                    want,
                    "{id} {platform:?} {preset}"
                );
            }
        }
    }
    // The table is complete: every command id under the M4 prefixes is in it.
    for d in reg.all() {
        let m4 = d.id.starts_with("ddl_view.")
            || d.id.starts_with("metadata.")
            || (d.id.starts_with("explorer.") && d.id != "explorer.focus_filter");
        if m4 {
            assert!(m4_ids().contains(&d.id), "{} is not in the M4 table", d.id);
        }
    }
    // The M2a filter row is kept on every platform.
    for (platform, chord) in [
        (KeymapPlatform::MacOs, "cmd-f"),
        (KeymapPlatform::Windows, "ctrl-f"),
        (KeymapPlatform::Linux, "ctrl-f"),
    ] {
        assert_eq!(
            chords(platform, "hacivat", "explorer.focus_filter", "Workspace"),
            vec![canonical(platform, chord)],
            "{platform:?}"
        );
    }
}

/// VS Code's default for each M4 command (PO-Q10), per platform, and where M4 differs, why.
/// `None` in the VS Code column: VS Code has no default for it.
struct VsCode {
    id: &'static str,
    context: &'static str,
    vscode: [Option<&'static str>; 3],
    /// The M4 row where it differs from VS Code's (`None` = no row).
    differs: &'static [(KeymapPlatform, Option<&'static str>)],
}

const VSCODE: &[VsCode] = &[
    // Go to Definition.
    VsCode {
        id: "explorer.view_ddl",
        context: "Explorer",
        vscode: [Some("f12"), Some("f12"), Some("f12")],
        differs: &[],
    },
    // Copy Path: VS Code's Linux chord (Ctrl+Shift+Alt+C) is AltGr+Shift+C, `Ć` on Polish; M4
    // takes VS Code's Windows chord there (the T2 review, carries-w0).
    VsCode {
        id: "explorer.copy_qualified_name",
        context: "Explorer",
        vscode: [
            Some("cmd-alt-c"),
            Some("shift-alt-c"),
            Some("ctrl-shift-alt-c"),
        ],
        differs: &[(KeymapPlatform::Linux, Some("shift-alt-c"))],
    },
    // `list.collapseAll`.
    VsCode {
        id: "explorer.collapse_all",
        context: "Explorer",
        vscode: [Some("cmd-left"), Some("ctrl-left"), Some("ctrl-left")],
        differs: &[],
    },
    // `list.find`: Ctrl+Alt+F is AltGr+F on Windows and Linux (P-M4-9): no row there.
    VsCode {
        id: "explorer.focus_filter",
        context: "Explorer",
        vscode: [Some("cmd-alt-f"), Some("ctrl-alt-f"), Some("ctrl-alt-f")],
        differs: &[
            (KeymapPlatform::Windows, None),
            (KeymapPlatform::Linux, None),
        ],
    },
    // Copy.
    VsCode {
        id: "ddl_view.copy",
        context: "DdlView",
        vscode: [Some("cmd-c"), Some("ctrl-c"), Some("ctrl-c")],
        differs: &[],
    },
    // VS Code has no refresh default; F5 is the file-manager and browser convention.
    VsCode {
        id: "explorer.refresh",
        context: "Explorer",
        vscode: [None, None, None],
        differs: &[
            (KeymapPlatform::MacOs, Some("f5")),
            (KeymapPlatform::Windows, Some("f5")),
            (KeymapPlatform::Linux, Some("f5")),
        ],
    },
];

/// Whether `chord` holds Ctrl and Alt together (AltGr on Windows and Linux).
fn is_altgr(platform: KeymapPlatform, chord: &str) -> bool {
    let canon = canonical(platform, chord);
    let parts: Vec<&str> = canon.split('-').collect();
    parts.contains(&"ctrl") && parts.contains(&"alt")
}

#[test]
fn m4_defaults_follow_the_vscode_table() {
    for row in VSCODE {
        for (i, platform) in ALL_PLATFORMS.into_iter().enumerate() {
            let differs = row.differs.iter().find(|(p, _)| *p == platform);
            let want = match differs {
                Some((_, ours)) => {
                    // A difference is never silent: where VS Code has a chord, ours differs only
                    // because VS Code's is an AltGr chord there.
                    if let Some(vs) = row.vscode[i] {
                        assert!(
                            platform != KeymapPlatform::MacOs && is_altgr(platform, vs),
                            "{} {platform:?}: {vs} is not an AltGr chord",
                            row.id
                        );
                    }
                    *ours
                }
                None => row.vscode[i],
            };
            let want: Vec<String> = want.iter().map(|c| canonical(platform, c)).collect();
            assert_eq!(
                chords(platform, "hacivat", row.id, row.context),
                want,
                "{} {platform:?}",
                row.id
            );
            // No M4 row takes M5's reserved reformat chord or starts with the palette's chord.
            let palette = if platform == KeymapPlatform::MacOs {
                "cmd-k"
            } else {
                "ctrl-k"
            };
            for chord in chords(platform, "hacivat", row.id, row.context) {
                for reserved in ["alt-shift-f", palette] {
                    let reserved = canonical(platform, reserved);
                    assert!(
                        chord != reserved && !chord.starts_with(&format!("{reserved} ")),
                        "{} {platform:?}: {chord}",
                        row.id
                    );
                }
            }
        }
    }
}

#[gpui::test]
fn keymap_commands_are_registered_including_host_only(cx: &mut TestAppContext) {
    let reg = registry(cx);
    // Every M4 row of every platform's keymap resolves to a registered command on this host,
    // whatever the host is: no M4 command is registered on one host only (`commands.rs`
    // `HOST_ONLY` gains nothing).
    let mut seen = 0;
    for platform in ALL_PLATFORMS {
        for preset in PRESETS {
            for b in load(platform, preset, None).0.bindings() {
                let m4_row = M4_ROWS.iter().any(|(id, _, context, _, _)| {
                    b.command == *id && b.context == *context && b.when_layout.is_none()
                }) && !(b.command == "explorer.focus_filter"
                    && b.context == "Workspace");
                if m4_row {
                    seen += 1;
                    assert!(reg.get(&b.command).is_some(), "{} {platform:?}", b.command);
                }
            }
        }
    }
    // macOS has six M4 rows, Windows and Linux five each (no filter row), in both presets.
    assert_eq!(seen, (6 + 5 + 5) * PRESETS.len());
    // The one host-only row is a keymap row, not a registration: `cmd-alt-f` exists only in the
    // macOS keymap, and its command is registered everywhere.
    for platform in [KeymapPlatform::Windows, KeymapPlatform::Linux] {
        assert!(chords(platform, "hacivat", "explorer.focus_filter", "Explorer").is_empty());
    }
    assert!(reg.get("explorer.focus_filter").is_some());
}

/// The M4 stubs (registered disabled until their task fills them). A task that fills one
/// removes it from its module's `STUB_COMMANDS` and lowers the count here (named exception).
#[gpui::test]
fn stub_count_is_n(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let mut stubs: Vec<&str> = explorer::STUB_COMMANDS.to_vec();
    stubs.extend(ddl_view::STUB_COMMANDS);
    assert_eq!(stubs.len(), 13);
    let (enabled, listed) = vcx.cx.read(|cx| {
        let reg = cx.global::<ActionRegistry>();
        let enabled: Vec<&'static str> = reg
            .all()
            .iter()
            .filter(|d| (d.enabled)(cx))
            .map(|d| d.id)
            .collect();
        let listed: Vec<&'static str> = hacivat_app::commands::palette::command_entries(cx)
            .iter()
            .map(|e| e.id)
            .collect();
        (enabled, listed)
    });
    for id in &stubs {
        assert!(!enabled.contains(id), "{id} is enabled");
        assert!(!listed.contains(id), "{id} is in the palette");
        // Running a stub does nothing: it is refused as disabled.
        assert!(!run(vcx, id), "{id} ran");
    }
    // The palette check is not vacuous: an enabled command is listed.
    assert!(listed.contains(&"workspace.open_settings"));
}

/// A provider for the seam test: one action per slot, out of slot order.
struct FakeProvider;

impl ExplorerActionProvider for FakeProvider {
    fn actions(&self, node: &NodeRef, surface: ActionSurface, _: &App) -> Vec<ProvidedAction> {
        if surface != ActionSurface::ExplorerMenu || node.kind != Some(ObjectKind::Table) {
            return Vec::new();
        }
        [
            (ActionSlot::Source, "fake.source"),
            (ActionSlot::Data, "fake.data"),
            (ActionSlot::Modify, "fake.modify"),
            (ActionSlot::Transfer, "fake.transfer"),
        ]
        .into_iter()
        .map(|(slot, id)| ProvidedAction {
            slot,
            id,
            label: id.into(),
            enabled: true,
        })
        .collect()
    }
}

#[gpui::test]
fn action_provider_stub_contributes_nothing(cx: &mut TestAppContext) {
    registry(cx);
    let node = NodeRef {
        profile: ProfileId::new(),
        key: TreeKey(7),
        object: Some(table("public", "orders")),
        kind: Some(ObjectKind::Table),
    };
    cx.update(|cx| {
        // M4 registers no provider and no contributor (R-M4-19).
        for surface in [ActionSurface::ExplorerMenu, ActionSurface::DdlViewToolbar] {
            assert!(provided_actions(&node, surface, cx).is_empty());
        }
        assert!(contributors(cx).is_empty());
        // `database_rows` (M5, M4b) lists nothing until Task 24.
        assert!(database_rows(node.profile, cx).is_empty());
        // The explorer model stub shows nothing and claims nothing about a key.
        let model = ExplorerModel::default();
        assert!(model.roots().is_empty());
        assert!(model.row(TreeKey(7)).is_none());
        assert_eq!(
            model.children(TreeKey(7)),
            hacivat_ui::tree::ChildState::NotLoaded
        );
        // The seam itself works for the milestones that use it (M6, M7): a registered
        // provider's actions come back grouped by slot, on the surface it answers for.
        register_action_provider(Rc::new(FakeProvider), cx);
        let ids: Vec<&str> = provided_actions(&node, ActionSurface::ExplorerMenu, cx)
            .iter()
            .map(|a| a.id)
            .collect();
        assert_eq!(
            ids,
            ["fake.data", "fake.transfer", "fake.modify", "fake.source"]
        );
        assert!(provided_actions(&node, ActionSurface::DdlViewToolbar, cx).is_empty());
        // A provider matches on the node's kind: a view gets nothing from it.
        let view = NodeRef {
            kind: Some(ObjectKind::View),
            ..node.clone()
        };
        assert!(provided_actions(&view, ActionSurface::ExplorerMenu, cx).is_empty());
    });
}

/// A PostgreSQL-style table key (`[schema, table]`).
fn table(schema: &str, name: &str) -> ObjectKey {
    ObjectKey {
        kind: ObjectKind::Table,
        path: [schema, name].into_iter().map(Into::into).collect(),
        server_id: None,
        overload_sig: None,
    }
}

/// The open-data handler, `reveal` and `open_ddl_view` are seams typed on `hacivat_meta`:
/// unset, unavailable and not yet until Tasks 24 and 25 (no stub claims a fact), and the handler
/// is replaceable (M6).
#[gpui::test]
fn meta_typed_seams_answer_nothing_until_their_tasks(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let profile = ProfileId::new();
    let key = table("public", "orders");
    let ws = workspace(vcx);
    let (opened, revealed, ran) = in_window(vcx, |window, cx| {
        (
            ddl_view::open_ddl_view(&ws, profile, key.clone(), window, cx),
            reveal(profile, &key, window, cx),
            open_data(profile, key.clone(), window, cx),
        )
    });
    assert_eq!(opened, Err(DdlViewError::NotYet));
    assert_eq!(tab_count(vcx), 1, "open_ddl_view opened a tab");
    assert_eq!(revealed, RevealOutcome::Unavailable);
    assert!(
        !ran,
        "no open-data handler is installed in M4 before Task 24"
    );
    // A handler, once set, receives the object; a second one replaces it.
    let seen: Rc<RefCell<Vec<(&str, ObjectKey)>>> = Rc::default();
    for name in ["first", "second"] {
        let sink = seen.clone();
        vcx.cx.update(|cx| {
            set_open_data_handler(
                Box::new(move |p, k, _, _| {
                    assert_eq!(p, profile);
                    sink.borrow_mut().push((name, k));
                }),
                cx,
            )
        });
    }
    let ran = in_window(vcx, |window, cx| {
        open_data(profile, key.clone(), window, cx)
    });
    assert!(ran);
    assert_eq!(seen.borrow().as_slice(), [("second", key)]);
}

/// Ruling R-W0-1: the DDL view is a `TabKind::DdlView` per object. Two objects open two tabs
/// titled "{name} · DDL" in the `DdlView` context; the same object focuses its tab; DDL view
/// tabs are not saved in `workspace.json`.
#[gpui::test]
fn ddl_view_tabs_are_one_per_object(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let vcx = shell_at(cx, root.path(), "en");
    let profile = ProfileId::new();
    let ws = workspace(vcx);
    let open = |vcx: &VisualTestContext, key: ObjectKey| {
        in_window(vcx, |_, cx| {
            let object = ddl_view::object_id(profile, &key, cx);
            ws.update(cx, |w, cx| w.open_ddl_view_tab(profile, object, cx))
                .expect("a tab")
        })
    };
    let before = tab_count(vcx);
    let orders = open(vcx, table("public", "orders"));
    vcx.run_until_parked();
    assert_eq!(tab_title(vcx), "orders · DDL");
    assert_eq!(context_stack(vcx).0.last(), Some(&DDL_VIEW_CONTEXT));
    let users = open(vcx, table("public", "users"));
    vcx.run_until_parked();
    assert_ne!(orders, users);
    assert_eq!(tab_count(vcx), before + 2);
    assert_eq!(tab_title(vcx), "users · DDL");
    // The same object (an equal key built anew) focuses its existing tab.
    let again = open(vcx, table("public", "orders"));
    vcx.run_until_parked();
    assert_eq!(again, orders);
    assert_eq!(tab_count(vcx), before + 2);
    assert_eq!(tab_title(vcx), "orders · DDL");
    let kind = vcx.cx.read(|cx| ws.read(cx).tab_kind(orders));
    assert!(matches!(kind, Some(TabKind::DdlView { profile: p, .. }) if p == profile));
    // The same name in another profile is another object.
    let other = in_window(vcx, |_, cx| {
        let object = ddl_view::object_id(ProfileId::new(), &table("public", "orders"), cx);
        let mine = ddl_view::object_id(profile, &table("public", "orders"), cx);
        object != mine
    });
    assert!(other);
    // Not saved: after the debounce, `workspace.json` holds no DDL view tab, and a restart
    // restores only the other tabs.
    vcx.executor().advance_clock(Duration::from_millis(600));
    vcx.run_until_parked();
    let saved = std::fs::read_to_string(root.path().join("data/workspace.json")).unwrap();
    assert!(!saved.to_ascii_lowercase().contains("ddl"), "{saved}");
    close_all_windows(vcx);
    let vcx2 = shell_at(cx, root.path(), "en");
    assert_eq!(tab_count(vcx2), before);
}
