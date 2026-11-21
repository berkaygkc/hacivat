//! M4b Task 1b: every M4b command id is registered, titled in EN and TR, disabled while it is a
//! stub and bound as R-M4b-37 lists; a console's source defaults to a console of its own; script
//! tabs are a tab kind saved in `workspace.json` version 2 (R-M4b-29); the M4b stubs add nothing
//! (no Scripts node, no menu item, no reload, an empty Go to script); and the three M4 explorer
//! seam gaps are closed (R-M4b-34), each driven end to end with fake contributors.

mod support;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{TestAppContext, VisualTestContext};
use hacivat_app::connections::explorer::{self as conn_explorer, ShownRow};
use hacivat_app::console::{ConsoleId, ConsoleSource, ConsoleState, Consoles};
use hacivat_app::explorer::adapter::{self, NodeInfo};
use hacivat_app::explorer::contrib::{
    self, ContributedChildren, ContributedNode, ExplorerContributor, contributors,
    register_contributor,
};
use hacivat_app::explorer::node_menu;
use hacivat_app::scripts::{
    self, DatabaseName, RelPath, SCRIPT_TAB_CONTEXT, ScriptTab, ScriptsContributor, goto, tabs,
};
use hacivat_app::shell::TabKind;
use hacivat_driver::fake::FakeDriver;
use hacivat_meta::{CatalogSnapshot, ObjectKind};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_session::profile::{Profile, ProfileEndpoint};
use hacivat_ui::actions::ActionRegistry;
use hacivat_ui::icon::IconName;
use hacivat_ui::keymaps::{PRESETS, load};
use hacivat_ui::menu::MenuItemSpec;
use hacivat_ui::text::TextSource;
use hacivat_ui::tree::{ChildState, TreeKey, TreeRowData};
use hacivat_ui_core::gpui_surface::App;
use hacivat_ui_core::keymap::{Keymap, KeymapPlatform, chord_string};
use support::catalogs::*;
use support::*;

const ALL_PLATFORMS: [KeymapPlatform; 3] = [
    KeymapPlatform::MacOs,
    KeymapPlatform::Windows,
    KeymapPlatform::Linux,
];

// ---- commands and keys ------------------------------------------------------------------------

/// R-M4b-37 and controller ruling R-T2-1 (c): id, the context it is registered in, the context
/// of its row, the macOS chord, the Windows/Linux chord (`None`: palette and menu only).
type Row = (
    &'static str,
    &'static str,
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
);

const M4B_ROWS: &[Row] = &[
    (
        "scripts.goto",
        "Workspace",
        "Workspace",
        Some("cmd-shift-o"),
        Some("ctrl-shift-o"),
    ),
    (
        "file.save",
        "ScriptTab",
        "ScriptTab",
        Some("cmd-s"),
        Some("ctrl-s"),
    ),
    // `ctrl-alt-s` is AltGr+S (`ś` on Polish), Ctrl+Shift+S is VS Code's Save As, Ctrl+K S starts
    // with the palette chord: no Windows/Linux row (Q22).
    (
        "file.save_all",
        "Workspace",
        "Workspace",
        Some("cmd-alt-s"),
        None,
    ),
    ("scripts.reveal", "Workspace", "Workspace", None, None),
    ("scripts.attach", "Workspace", "Workspace", None, None),
    // Script file and folder rows only; Move to Trash has no chord (ruling c).
    (
        "scripts.rename",
        "ExplorerTree",
        "ExplorerTree",
        Some("f2"),
        Some("f2"),
    ),
];

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

const EN_UI: &str = include_str!("../../../../locales/en/hacivat-ui.ftl");
const TR_UI: &str = include_str!("../../../../locales/tr/hacivat-ui.ftl");
const EN_HUI: &str = include_str!("../../../../locales/en/hui.ftl");

#[gpui::test]
fn every_m4b_command_id_is_registered_and_bound_as_listed(cx: &mut TestAppContext) {
    let reg = registry(cx);
    let en = l10n_en();
    let tr_ids = message_ids_in(TR_UI);
    for (id, registered_in, row_context, mac, pc) in M4B_ROWS {
        let def = reg
            .get(id)
            .unwrap_or_else(|| panic!("{id} is not registered"));
        assert_eq!(def.context, *registered_in, "{id}");
        // R5: the title follows the command id.
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
                // No other context binds it either (one row per command).
                let anywhere = load(platform, preset, None)
                    .0
                    .bindings()
                    .iter()
                    .filter(|b| b.command == *id && b.when_layout.is_none())
                    .count();
                assert_eq!(anywhere, want.len(), "{id} {platform:?} {preset}");
            }
        }
    }
    // No `file.save_all` row on Windows or Linux, in any preset.
    for platform in [KeymapPlatform::Windows, KeymapPlatform::Linux] {
        for preset in PRESETS {
            assert!(
                load(platform, preset, None)
                    .0
                    .bindings()
                    .iter()
                    .all(|b| b.command != "file.save_all"),
                "{platform:?} {preset}"
            );
        }
    }
    // The table is complete: every command under the M4b prefixes is in it.
    for d in reg.all() {
        if d.id.starts_with("scripts.") || d.id.starts_with("file.") {
            assert!(
                M4B_ROWS.iter().any(|r| r.0 == d.id),
                "{} is not in the M4b table",
                d.id
            );
        }
    }
}

/// Whether `chord` holds Ctrl and Alt together (AltGr on Windows and Linux).
fn is_altgr(platform: KeymapPlatform, chord: &str) -> bool {
    let canon = canonical(platform, chord);
    let parts: Vec<&str> = canon.split('-').collect();
    parts.contains(&"ctrl") && parts.contains(&"alt")
}

#[test]
fn m4b_chords_avoid_altgr_the_palette_chord_and_reformat_and_take_no_ones_chord() {
    for platform in ALL_PLATFORMS {
        let palette = if platform == KeymapPlatform::MacOs {
            "cmd-k"
        } else {
            "ctrl-k"
        };
        for preset in PRESETS {
            let km = load(platform, preset, None).0;
            for (id, _, context, mac, pc) in M4B_ROWS {
                let mine = if platform == KeymapPlatform::MacOs {
                    mac
                } else {
                    pc
                };
                let Some(chord) = mine else { continue };
                let canon = canonical(platform, chord);
                if platform != KeymapPlatform::MacOs {
                    assert!(!is_altgr(platform, chord), "{id} {platform:?}: {chord}");
                }
                for reserved in ["alt-shift-f", palette] {
                    let reserved = canonical(platform, reserved);
                    assert!(
                        canon != reserved && !canon.starts_with(&format!("{reserved} ")),
                        "{id} {platform:?}: {canon}"
                    );
                }
                // Nothing else is bound to the same chord in the same context.
                let others: Vec<&str> = km
                    .bindings()
                    .iter()
                    .filter(|b| {
                        b.context == *context
                            && chord_string(&b.chord) == canon
                            && b.command != *id
                            && b.when_layout.is_none()
                    })
                    .map(|b| b.command.as_str())
                    .collect();
                assert!(others.is_empty(), "{id} {platform:?} {preset}: {others:?}");
            }
        }
    }
}

/// The M4b stubs: registered disabled until their tasks fill them. A task that fills one removes
/// it from `scripts::STUB_COMMANDS` and lowers the count here (named exception).
#[gpui::test]
fn m4b_stub_commands_are_disabled_and_out_of_the_palette(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let stubs: Vec<&str> = scripts::STUB_COMMANDS.to_vec();
    assert_eq!(stubs.len(), 6);
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
        assert!(!run(vcx, id), "{id} ran");
    }
    // The palette check is not vacuous: an enabled command is listed.
    assert!(listed.contains(&"workspace.open_settings"));
    // The stub contributor is not registered (Task 13 registers the real one), so M4's own
    // check that M4 registers none still holds.
    assert!(vcx.cx.read(|cx| contributors(cx).is_empty()));
}

#[test]
fn every_m4b_title_id_exists_in_en_and_tr() {
    let block = |ftl: &str| -> Vec<String> {
        let (_, after) = ftl
            .split_once("\n## m4b-t01b-seams\n")
            .expect("the M4b Task 1b block");
        // The block runs to the next block header (none yet) or the end.
        let block = after.split("\n## ").next().unwrap_or(after);
        message_ids_in(block)
    };
    let en = block(EN_UI);
    let tr = block(TR_UI);
    assert_eq!(en.len(), 206, "the mock's 205 messages and the F2 title");
    assert_eq!(en, tr, "EN and TR hold the same ids, in the same order");
    let en_catalog = l10n_en();
    for id in &en {
        assert!(!en_catalog.t_english(id).starts_with('⟦'), "{id}");
    }
    // The copy table's prefixes; the editor's T2 and paste ids belong to Task 11's `hui.ftl`
    // block (carries-w1 "T1b").
    for id in &en {
        assert!(
            id.starts_with("hv-scripts-")
                || id.starts_with("hv-cmd-scripts-")
                || id.starts_with("hv-cmd-file-")
                || id.starts_with("hv-explorer-menu-attach-library")
                || id.starts_with("hv-explorer-menu-map-library")
                || id.starts_with("hv-settings-nav-scripts")
                || id.starts_with("hv-settings-section-scripts"),
            "{id}"
        );
    }
    for id in message_ids_in(EN_HUI) {
        assert!(
            !id.starts_with("hui-editor-t2-") && !id.starts_with("hui-editor-paste-confirm-"),
            "{id} is Task 11's"
        );
    }
    // Every M4b command's title is in the block.
    for (id, ..) in M4B_ROWS {
        let title = format!("hv-cmd-{}", id.replace(['.', '_'], "-"));
        assert!(en.contains(&title), "{title}");
    }
}

#[test]
fn the_m4b_icons_are_bundled() {
    use gpui::AssetSource as _;
    for icon in [
        IconName::Folder,
        IconName::FileText,
        IconName::Download,
        IconName::FileDiff,
        IconName::ScrollText,
        IconName::Link,
    ] {
        let bytes = hacivat_ui_core::assets::BundledAssets
            .load(icon.path())
            .expect("loads")
            .unwrap_or_else(|| panic!("{icon:?} is bundled"));
        let text = std::str::from_utf8(&bytes).expect("utf-8");
        assert!(text.contains("stroke-width=\"1.6\""), "{icon:?}");
    }
}

// ---- consoles and script tabs ------------------------------------------------------------------

#[gpui::test]
fn console_source_defaults_to_console_so_m3_restore_holds(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let ws = workspace(vcx);
    let profile = ProfileId::new();
    let register = |vcx: &VisualTestContext| {
        in_window(vcx, |window, cx| {
            let tab = ws
                .update(cx, |w, cx| w.open_console_tab(Some(profile), window, cx))
                .expect("a console tab");
            let log = hacivat_exec::SessionLog::new();
            Consoles::register(
                cx,
                ConsoleState {
                    id: ConsoleId(0),
                    tab,
                    n: 2,
                    profile: Some(profile),
                    client: hacivat_session::session::ClientId::DEFAULT,
                    session: None,
                    editor: None,
                    results: None,
                    sql: None,
                    log,
                    service: None,
                },
            )
        })
    };
    let console = register(vcx);
    // Every console M3 registers (its restore included) is a console of its own.
    let sources: Vec<ConsoleSource> = vcx.cx.read(|cx| {
        Consoles::ids(cx)
            .into_iter()
            .map(|id| Consoles::source(cx, id))
            .collect()
    });
    assert!(!sources.is_empty());
    assert!(sources.iter().all(|s| *s == ConsoleSource::Console));
    // A script source is kept beside the state, and dropped with the console.
    let script = ConsoleSource::Script {
        library: scripts::parse_library_id("01234567-89ab-cdef-0123-456789abcdef").unwrap(),
        rel: RelPath::parse("reports/orders.sql").unwrap(),
        database: Some(DatabaseName::new("orders")),
    };
    vcx.cx
        .update(|cx| Consoles::set_source(cx, console, script.clone()));
    assert_eq!(vcx.cx.read(|cx| Consoles::source(cx, console)), script);
    vcx.cx.update(|cx| Consoles::remove(cx, console));
    assert_eq!(
        vcx.cx.read(|cx| Consoles::source(cx, console)),
        ConsoleSource::Console
    );
    // A console that is gone takes no source.
    vcx.cx
        .update(|cx| Consoles::set_source(cx, console, script.clone()));
    assert_eq!(
        vcx.cx.read(|cx| Consoles::source(cx, console)),
        ConsoleSource::Console
    );
}

/// A layout with a console and one more tab of `kind_json` (a `"kind": …` object body), at
/// `version`.
fn two_tab_workspace(version: u32, kind_json: &str) -> String {
    format!(
        r#"{{"version": {version},
            "tiles": {{"explorer_w": 260.0, "conn_panel_h": 180.0, "editor_h": 420.0,
                      "inspector_w": 300.0, "explorer_visible": true}},
            "panes": {{"root": {{"Leaf": 0}}, "panes": {{"0": {{"tabs": [0, 1], "active": 1}}}},
                      "focused": 0}},
            "tabs": [{{"id": 0, "kind": "console", "n": 1}}, {{"id": 1, {kind_json}}}]}}"#
    )
}

const LIBRARY: &str = "01234567-89ab-cdef-0123-456789abcdef";

fn script_json(profile: &str) -> String {
    format!(
        r#""kind": "script", "library": "{LIBRARY}", "rel_path": "reports/orders.sql",
            "profile": {profile}, "database": "orders""#
    )
}

fn saved_json(root: &std::path::Path) -> serde_json::Value {
    let text = std::fs::read_to_string(root.join("data").join("workspace.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn set_aside(root: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(root.join("data"))
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.contains("corrupt") || n.contains("newer"))
        .collect()
}

#[gpui::test]
fn workspace_json_v1_loads_with_no_script_tabs(cx: &mut TestAppContext) {
    let root = temp_root();
    std::fs::create_dir_all(root.join("data")).unwrap();
    // A version-1 file restores its consoles; a script-kind tab in it (no version-1 build wrote
    // one) closes like a tab of a kind nobody registered, and the file is not set aside.
    let profile = ProfileId::new();
    write(
        root.join("data").join("workspace.json"),
        &two_tab_workspace(1, &script_json(&format!("\"{profile}\""))),
    );
    let vcx = shell_at(cx, root.as_path(), "en");
    assert_eq!(tab_count(vcx), 1);
    let ws = workspace(vcx);
    let (panes, _) = layout(vcx);
    let kinds: Vec<TabKind> = vcx.cx.read(|cx| {
        panes
            .panes
            .values()
            .flat_map(|p| p.tabs.clone())
            .filter_map(|t| ws.read(cx).tab_kind(t))
            .collect()
    });
    assert!(
        matches!(kinds.as_slice(), [TabKind::Console { n: 1, .. }]),
        "{kinds:?}"
    );
    assert!(set_aside(&root).is_empty(), "{:?}", set_aside(&root));
    // The saved file says version 2.
    assert!(run(vcx, "workspace.new_console_tab"));
    vcx.executor().advance_clock(Duration::from_millis(600));
    vcx.run_until_parked();
    let saved = saved_json(&root);
    assert_eq!(saved["version"], 2);
    let tabs = saved["tabs"].as_array().unwrap();
    assert!(tabs.iter().all(|t| t["kind"] == "console"), "{tabs:?}");
}

#[gpui::test]
fn script_tab_kind_round_trips_through_workspace_json(cx: &mut TestAppContext) {
    let root = temp_root();
    let vcx = shell_at(cx, root.as_path(), "en");
    let ws = workspace(vcx);
    let profile = ProfileId::new();
    let spec = ScriptTab {
        library: scripts::parse_library_id(LIBRARY).unwrap(),
        rel: RelPath::parse("reports/orders.sql").unwrap(),
        profile: Some(profile),
        database: Some(DatabaseName::new("orders")),
    };
    let open = |vcx: &VisualTestContext, spec: ScriptTab| {
        in_window(vcx, |_, cx| {
            let script = tabs::intern(spec, cx);
            ws.update(cx, |w, cx| w.open_script_tab(script, cx))
                .expect("a tab")
        })
    };
    let before = tab_count(vcx);
    let tab = open(vcx, spec.clone());
    vcx.run_until_parked();
    assert_eq!(tab_count(vcx), before + 1);
    // The profile is unknown to this app: the title says the tab has no connection (and the
    // tab is never bound to another one).
    assert_eq!(tab_title(vcx), "orders.sql · no connection");
    assert!(context_stack(vcx).0.contains(&SCRIPT_TAB_CONTEXT));
    // The same (file, connection, database) focuses its tab: one tab per file and connection.
    assert!(run(vcx, "workspace.new_console_tab"));
    assert_eq!(open(vcx, spec.clone()), tab);
    assert_eq!(tab_count(vcx), before + 2);
    // Under another connection it is another tab.
    let other = open(
        vcx,
        ScriptTab {
            profile: Some(ProfileId::new()),
            ..spec.clone()
        },
    );
    assert_ne!(other, tab);
    assert!(run(vcx, "workspace.close_tab"), "closes the other one");
    // Saved as its own kind with the binding itself (R-M4b-22).
    vcx.executor().advance_clock(Duration::from_millis(600));
    vcx.run_until_parked();
    let saved = saved_json(&root);
    assert_eq!(saved["version"], 2);
    let tabs = saved["tabs"].as_array().unwrap();
    let script = tabs
        .iter()
        .find(|t| t["kind"] == "script")
        .unwrap_or_else(|| panic!("a script tab in {tabs:?}"));
    assert_eq!(script["id"], tab.0);
    assert_eq!(script["library"], LIBRARY);
    assert_eq!(script["rel_path"], "reports/orders.sql");
    assert_eq!(script["profile"], profile.to_string());
    assert_eq!(script["database"], "orders");
    let mut keys: Vec<&str> = script
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["database", "id", "kind", "library", "profile", "rel_path"]
    );
    // A restart restores it with the same binding, in the same place.
    let ids_kinds = |vcx: &VisualTestContext| {
        let ws = workspace(vcx);
        let (panes, _) = layout(vcx);
        vcx.cx.read(|cx| {
            panes
                .panes
                .values()
                .flat_map(|p| p.tabs.clone())
                .filter_map(|t| ws.read(cx).tab_kind(t).map(|k| (t, k)))
                .collect::<Vec<_>>()
        })
    };
    let before_restart = ids_kinds(vcx).len();
    close_all_windows(vcx);
    let vcx2 = shell_at(cx, root.as_path(), "en");
    assert!(set_aside(&root).is_empty(), "{:?}", set_aside(&root));
    let restored = ids_kinds(vcx2);
    assert_eq!(restored.len(), before_restart);
    let (_, kind) = restored
        .iter()
        .find(|(t, _)| *t == tab)
        .expect("the script tab is restored");
    let TabKind::Script { script } = kind else {
        panic!("{kind:?}");
    };
    assert_eq!(vcx2.cx.read(|cx| tabs::spec(*script, cx)), Some(spec));
}

#[gpui::test]
fn a_saved_script_tab_with_a_bad_binding_closes_and_one_without_a_profile_stays_unbound(
    cx: &mut TestAppContext,
) {
    for (kind, kept) in [
        // A path that leaves the library, a library that is not an id: the tab closes.
        (
            format!(
                r#""kind": "script", "library": "{LIBRARY}", "rel_path": "../x.sql", "profile": null, "database": null"#
            ),
            false,
        ),
        (
            r#""kind": "script", "library": "lib", "rel_path": "x.sql", "profile": null, "database": null"#
                .to_string(),
            false,
        ),
        // An unreadable profile restores the tab unbound (as a console's), never bound.
        (script_json("\"not-a-uuid\""), true),
    ] {
        let root = temp_root();
        std::fs::create_dir_all(root.join("data")).unwrap();
        write(
            root.join("data").join("workspace.json"),
            &two_tab_workspace(2, &kind),
        );
        let vcx = shell_at(cx, root.as_path(), "en");
        assert!(set_aside(&root).is_empty(), "{kind}");
        assert_eq!(tab_count(vcx), if kept { 2 } else { 1 }, "{kind}");
        if kept {
            let ws = workspace(vcx);
            let spec = vcx.cx.read(|cx| match ws.read(cx).tab_kind(hacivat_ui::panes::TabId(1)) {
                Some(TabKind::Script { script }) => tabs::spec(script, cx),
                _ => None,
            });
            assert_eq!(spec.and_then(|s| s.profile), None, "{kind}");
        }
        close_all_windows(vcx);
    }
}

// ---- stubs ------------------------------------------------------------------------------------

fn strip(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

fn rows(vcx: &VisualTestContext) -> Vec<ShownRow> {
    vcx.run_until_parked();
    vcx.cx.read(conn_explorer::shown_rows)
}

/// The shown rows as `indent label`.
fn lines(vcx: &VisualTestContext) -> Vec<String> {
    rows(vcx)
        .into_iter()
        .map(|r| format!("{}{}", "  ".repeat(r.depth as usize), strip(&r.data.label)))
        .collect()
}

fn key_of(vcx: &VisualTestContext, label: &str) -> TreeKey {
    rows(vcx)
        .into_iter()
        .find(|r| strip(&r.data.label) == label && r.status.is_none())
        .unwrap_or_else(|| panic!("`{label}` is shown in {:#?}", lines(vcx)))
        .key
}

fn open_rows(vcx: &VisualTestContext, labels: &[&str]) {
    for l in labels {
        let k = key_of(vcx, l);
        vcx.cx.update(|cx| {
            let tree = conn_explorer::tree(cx).expect("the tree");
            tree.update(cx, |t, cx| t.set_expanded(k, true, cx));
        });
        vcx.run_until_parked();
    }
}

fn put_cursor(vcx: &VisualTestContext, key: TreeKey) {
    vcx.cx.update(|cx| {
        let tree = conn_explorer::tree(cx).expect("the tree");
        tree.update(cx, |t, cx| t.select(key, cx));
    });
    vcx.run_until_parked();
}

fn show(vcx: &VisualTestContext, profile: ProfileId, cat: Arc<CatalogSnapshot>) {
    vcx.cx
        .update(|cx| adapter::set_catalog(profile, Some(cat), None, cx));
    vcx.run_until_parked();
}

fn pg(name: &str) -> Profile {
    let mut p = test_profile(name, Some(EnvTag::Dev), "127.0.0.1");
    p.database = Some("app".to_string());
    p
}

fn pg_app() -> Arc<CatalogSnapshot> {
    catalog(
        DbmsKind::Postgres,
        vec![schema(
            DbmsKind::Postgres,
            &["app", "public"],
            false,
            vec![table("orders", Some(16_400), &[("id", "bigint")])],
        )],
    )
}

/// A shell with the PostgreSQL profile `shop` (database `app`) and its catalog shown.
fn pg_shell(cx: &mut TestAppContext) -> (&'static mut VisualTestContext, ProfileId) {
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    let id = add_profile(vcx, pg("shop"));
    show(vcx, id, pg_app());
    (vcx, id)
}

#[gpui::test]
fn scripts_contributor_stub_adds_no_node_so_m4_shots_hold(cx: &mut TestAppContext) {
    let (vcx, id) = pg_shell(cx);
    open_rows(vcx, &["shop", "app"]);
    let before = lines(vcx);
    assert_eq!(
        before,
        ["DEV", "shop", "  app", "    public"].map(String::from),
        "{before:?}"
    );
    let stub = ScriptsContributor;
    vcx.cx.update(|cx| {
        assert!(stub.database_children(id, "app", cx).is_empty());
        assert_eq!(stub.children(TreeKey(1), cx), ChildState::Leaf);
        assert!(stub.child_node(TreeKey(1), cx).is_none());
        assert!(stub.context_menu(TreeKey(1), cx).is_empty());
    });
    assert_eq!(stub.generation(), 0);
    vcx.cx.update(|cx| {
        register_contributor(Rc::new(ScriptsContributor), cx);
        contrib::contributors_changed(cx);
    });
    assert_eq!(lines(vcx), before);
    // Its activation opens nothing.
    let tabs_before = tab_count(vcx);
    in_window(vcx, |window, cx| {
        ScriptsContributor.activate(TreeKey(1), window, cx)
    });
    vcx.run_until_parked();
    assert_eq!(tab_count(vcx), tabs_before);
    // `open_script` opens nothing either, bound or not.
    let opened = vcx.cx.update(|cx| {
        scripts::open_script(
            cx,
            scripts::parse_library_id(LIBRARY).unwrap(),
            RelPath::parse("a.sql").unwrap(),
            Some(id),
            Some(DatabaseName::new("app")),
        )
    });
    assert_eq!(opened, None);
    assert_eq!(tab_count(vcx), tabs_before);
    // And the menu seam adds nothing: the database row's menu is M4's alone.
    let menu = vcx
        .cx
        .read(|cx| node_menu(key_of(vcx, "app"), cx))
        .expect("a menu");
    assert!(menu.last().is_some_and(|m| !m.separator));
    assert!(menu.iter().all(|m| !m.action.starts_with("scripts.")));
}

#[gpui::test]
fn goto_script_stub_lists_nothing(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    in_window(vcx, goto::open);
    vcx.run_until_parked();
    assert_eq!(layer_ids(vcx), [goto::GOTO_LAYER]);
    let host = vcx.cx.read(goto::open_host).expect("the layer's host");
    let palette = vcx.cx.read(|cx| host.read(cx).palette().clone());
    assert!(vcx.cx.read(|cx| palette.read(cx).items().is_empty()));
    // Typing finds nothing.
    vcx.simulate_input("orders");
    vcx.run_until_parked();
    assert!(vcx.cx.read(|cx| palette.read(cx).items().is_empty()));
    // A second open keeps the one layer.
    in_window(vcx, goto::open);
    vcx.run_until_parked();
    assert_eq!(layer_ids(vcx), [goto::GOTO_LAYER]);
    // Escape closes it.
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(layer_ids(vcx).is_empty());
}

// ---- the M4 seam gaps (R-M4b-34), with fake contributors ---------------------------------------

/// A contributor that puts "<label>" (lazy) under database `app`, with one leaf
/// "<label>.sql", and records what the explorer asks of it.
struct Fake {
    base: u128,
    label: RefCell<String>,
    generation: Cell<u64>,
    activated: RefCell<Vec<TreeKey>>,
    menus: RefCell<Vec<TreeKey>>,
}

impl Fake {
    fn new(base: u128, label: &str) -> Rc<Fake> {
        Rc::new(Fake {
            base,
            label: RefCell::new(label.to_string()),
            generation: Cell::new(1),
            activated: RefCell::default(),
            menus: RefCell::default(),
        })
    }

    fn node_key(&self) -> TreeKey {
        TreeKey(self.base + 1)
    }

    fn leaf_key(&self) -> TreeKey {
        TreeKey(self.base + 2)
    }

    fn node(&self, key: TreeKey, label: String, children: ContributedChildren) -> ContributedNode {
        let mut row = TreeRowData::new(label.clone(), TextSource::Data);
        row.icon = Some(IconName::ScrollText);
        ContributedNode {
            key,
            row,
            children,
            filter_text: Some(label),
        }
    }
}

impl ExplorerContributor for Fake {
    fn database_children(&self, _: ProfileId, database: &str, _: &App) -> Vec<ContributedNode> {
        if database != "app" {
            return Vec::new();
        }
        let leaf = self.leaf_key();
        vec![self.node(
            self.node_key(),
            self.label.borrow().clone(),
            ContributedChildren::Lazy(Arc::new(move |_| ChildState::Loaded(Arc::from([leaf])))),
        )]
    }
    fn children(&self, _: TreeKey, _: &App) -> ChildState {
        ChildState::Leaf
    }
    fn child_node(&self, key: TreeKey, _: &App) -> Option<ContributedNode> {
        (key == self.leaf_key()).then(|| {
            let label = format!("{}.sql", self.label.borrow());
            self.node(key, label, ContributedChildren::Leaf)
        })
    }
    fn activate(&self, key: TreeKey, _: &mut gpui::Window, _: &mut App) {
        self.activated.borrow_mut().push(key);
    }
    fn context_menu(&self, key: TreeKey, _: &App) -> Vec<MenuItemSpec> {
        self.menus.borrow_mut().push(key);
        let label = self.label.borrow().clone();
        vec![MenuItemSpec::new(
            Rc::new(move |_: &App| format!("{label} item").into()),
            format!("fake.{}", self.base),
        )]
    }
    fn generation(&self) -> u64 {
        self.generation.get()
    }
}

/// The PostgreSQL shell with two fakes registered under `app`, both expanded.
fn shell_with_fakes(
    cx: &mut TestAppContext,
) -> (&'static mut VisualTestContext, Rc<Fake>, Rc<Fake>) {
    let (vcx, _) = pg_shell(cx);
    let a = Fake::new(1_000, "Scripts");
    let b = Fake::new(2_000, "Other");
    vcx.cx.update(|cx| {
        register_contributor(a.clone(), cx);
        register_contributor(b.clone(), cx);
    });
    open_rows(vcx, &["shop", "app", "Scripts", "Other"]);
    assert_eq!(
        lines(vcx),
        [
            "DEV",
            "shop",
            "  app",
            "    Scripts",
            "      Scripts.sql",
            "    Other",
            "      Other.sql",
            "    public",
        ]
        .map(String::from)
    );
    (vcx, a, b)
}

#[gpui::test]
fn activating_a_contributed_leaf_calls_its_contributor(cx: &mut TestAppContext) {
    let (vcx, a, b) = shell_with_fakes(cx);
    in_window(vcx, |window, _| window.activate_window());
    in_window(vcx, conn_explorer::focus_list);
    put_cursor(vcx, a.leaf_key());
    press_key(vcx, "enter");
    assert_eq!(*a.activated.borrow(), [a.leaf_key()]);
    assert!(b.activated.borrow().is_empty(), "only the owner is asked");
    // Another contributor's leaf goes to that contributor only.
    put_cursor(vcx, b.leaf_key());
    press_key(vcx, "enter");
    assert_eq!(*a.activated.borrow(), [a.leaf_key()]);
    assert_eq!(*b.activated.borrow(), [b.leaf_key()]);
    // An expandable contributed node still toggles, and is not activated.
    put_cursor(vcx, a.node_key());
    press_key(vcx, "enter");
    assert!(!lines(vcx).contains(&"      Scripts.sql".to_string()));
    assert_eq!(*a.activated.borrow(), [a.leaf_key()]);
    press_key(vcx, "enter");
    assert!(lines(vcx).contains(&"      Scripts.sql".to_string()));
    assert_eq!(*a.activated.borrow(), [a.leaf_key()]);
}

#[gpui::test]
fn a_contributed_node_shows_its_contributors_menu(cx: &mut TestAppContext) {
    let (vcx, a, b) = shell_with_fakes(cx);
    let actions = |vcx: &VisualTestContext, key: TreeKey| -> Option<Vec<String>> {
        vcx.cx
            .read(|cx| node_menu(key, cx).map(|m| m.iter().map(|i| i.action.to_string()).collect()))
    };
    assert_eq!(
        actions(vcx, a.leaf_key()),
        Some(vec!["fake.1000".to_string()])
    );
    assert_eq!(
        actions(vcx, a.node_key()),
        Some(vec!["fake.1000".to_string()])
    );
    assert_eq!(
        actions(vcx, b.leaf_key()),
        Some(vec!["fake.2000".to_string()])
    );
    assert_eq!(*a.menus.borrow(), [a.leaf_key(), a.node_key()]);
    assert_eq!(*b.menus.borrow(), [b.leaf_key()], "only the owner is asked");
    // Opening the menu on the row shows the contributor's items.
    let at = gpui::Point::default();
    in_window(vcx, |window, cx| {
        hacivat_app::explorer::open_node_menu(a.leaf_key(), at, window, cx)
    });
    vcx.run_until_parked();
    enable_registry(vcx);
    assert!(
        full_frame(vcx)
            .iter()
            .any(|t| t.text.as_ref() == "Scripts item"),
        "the contributor's item is drawn"
    );
}

#[gpui::test]
fn database_and_schema_menus_end_with_the_scripts_items(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    let pg_id = add_profile(vcx, pg("shop"));
    show(vcx, pg_id, pg_app());
    let mut my = test_profile("api-dev", Some(EnvTag::Dev), "127.0.0.1");
    my.engine = DbmsKind::MySql;
    let my_id = add_profile(vcx, my);
    show(
        vcx,
        my_id,
        catalog(
            DbmsKind::MySql,
            vec![schema(
                DbmsKind::MySql,
                &["api"],
                false,
                vec![table("orders", None, &[("id", "int")])],
            )],
        ),
    );
    let mut lite = test_profile("local-file", Some(EnvTag::Local), "127.0.0.1");
    lite.engine = DbmsKind::Sqlite;
    lite.endpoint = ProfileEndpoint::File {
        path: "/data/orders.db".into(),
        create_if_missing: false,
    };
    let lite_id = add_profile(vcx, lite);
    open_rows(vcx, &["shop", "api-dev"]);
    let pg_db = key_of(vcx, "app");
    let my_schema = key_of(vcx, "api");
    let menu = |vcx: &VisualTestContext, key: TreeKey| -> Vec<MenuItemSpec> {
        vcx.cx.read(|cx| node_menu(key, cx)).expect("a menu")
    };
    let root_menu = |vcx: &VisualTestContext, id: ProfileId| -> Vec<MenuItemSpec> {
        vcx.cx.read(|cx| conn_explorer::menu_items(cx, id))
    };
    let actions = |items: &[MenuItemSpec]| -> Vec<String> {
        items
            .iter()
            .map(|i| {
                if i.separator {
                    "—".to_string()
                } else {
                    i.action.to_string()
                }
            })
            .collect()
    };
    // The stub: M4's menus, unchanged.
    let m4_pg = actions(&menu(vcx, pg_db));
    let m4_my = actions(&menu(vcx, my_schema));
    let m2a_lite = actions(&root_menu(vcx, lite_id));
    let m2a_pg_root = actions(&root_menu(vcx, pg_id));
    assert_eq!(
        m4_pg.last().map(String::as_str),
        Some("explorer.collapse_children")
    );
    assert_eq!(
        m4_my.last().map(String::as_str),
        Some("explorer.collapse_children")
    );
    assert_eq!(
        m2a_lite.last().map(String::as_str),
        Some("connection.delete")
    );
    assert_eq!(m2a_lite, m2a_pg_root);
    // A test hook makes the stub answer one item and records the node it was asked for.
    let asked: Rc<RefCell<Vec<NodeInfo>>> = Rc::default();
    let seen = asked.clone();
    vcx.cx.update(|cx| {
        scripts::set_database_menu_items_for_tests(
            move |node, _| {
                seen.borrow_mut().push(node.clone());
                vec![MenuItemSpec::new(
                    Rc::new(|_: &App| "Attach".into()),
                    "scripts.attach",
                )]
            },
            cx,
        )
    });
    let with = |m4: &[String]| -> Vec<String> {
        let mut v = m4.to_vec();
        v.extend(["—".to_string(), "scripts.attach".to_string()]);
        v
    };
    assert_eq!(actions(&menu(vcx, pg_db)), with(&m4_pg));
    assert_eq!(actions(&menu(vcx, my_schema)), with(&m4_my));
    // The SQLite root: after the other items and a separator, before the last Delete….
    let mut lite_want = m2a_lite.clone();
    let delete = lite_want.pop().unwrap();
    lite_want.extend([
        "—".to_string(),
        "scripts.attach".to_string(),
        "—".to_string(),
        delete,
    ]);
    let lite_menu = root_menu(vcx, lite_id);
    assert_eq!(actions(&lite_menu), lite_want);
    assert!(
        lite_menu.last().is_some_and(|m| m.danger),
        "Delete… stays last and red"
    );
    // A PostgreSQL profile root is no database node: its menu is unchanged.
    assert_eq!(actions(&root_menu(vcx, pg_id)), m2a_pg_root);
    // The hook saw the PG database, the MySQL schema and the SQLite root.
    let asked = asked.borrow();
    assert!(
        matches!(asked[0], NodeInfo::Database { profile, ref name } if profile == pg_id && name.as_ref() == "app"),
        "{:?}",
        asked[0]
    );
    assert!(
        matches!(asked[1], NodeInfo::Schema { profile, .. } if profile == my_id),
        "{:?}",
        asked[1]
    );
    assert_eq!(asked[2], NodeInfo::Profile(lite_id));
    assert_eq!(asked.len(), 3);
}

#[gpui::test]
fn contributors_changed_redraws_the_explorer(cx: &mut TestAppContext) {
    let (vcx, _) = pg_shell(cx);
    let a = Fake::new(1_000, "Scripts");
    vcx.cx.update(|cx| register_contributor(a.clone(), cx));
    open_rows(vcx, &["shop", "app"]);
    enable_registry(vcx);
    redraw_full(vcx);
    let drawn = |vcx: &VisualTestContext, text: &str| {
        registry_frame(vcx).iter().any(|t| t.text.as_ref() == text)
    };
    assert!(drawn(vcx, "Scripts"));
    // The fake's answers change; it says so once, and nothing else happens.
    *a.label.borrow_mut() = "Betikler".to_string();
    a.generation.set(2);
    vcx.cx.update(contrib::contributors_changed);
    vcx.run_until_parked();
    // The model shows the new row, and the window drew it by itself: the last frame (no
    // refresh asked for) has it.
    assert!(
        lines(vcx).contains(&"    Betikler".to_string()),
        "{:?}",
        lines(vcx)
    );
    assert!(drawn(vcx, "Betikler"), "the explorer redrew on its own");
    assert!(!drawn(vcx, "Scripts"));
    // What the window shows now agrees.
    assert!(
        full_frame(vcx)
            .iter()
            .any(|t| t.text.as_ref() == "Betikler")
    );
}

#[gpui::test]
fn a_key_no_contributor_listed_has_no_owner_menu_or_activation(cx: &mut TestAppContext) {
    let (vcx, a, b) = shell_with_fakes(cx);
    // A key outside both fakes' domains: no menu from anyone.
    let stray = TreeKey(a.base + 50);
    assert!(vcx.cx.read(|cx| node_menu(stray, cx)).is_none());
    assert!(a.menus.borrow().is_empty() && b.menus.borrow().is_empty());
    // The objects of the catalog keep M4's menus (the seam takes only contributed nodes).
    let orders_menu = {
        open_rows(vcx, &["public"]);
        let tables = rows(vcx)
            .into_iter()
            .find(|r| strip(&r.data.label).starts_with("TABLES"))
            .expect("the tables group")
            .key;
        vcx.cx.update(|cx| {
            let tree = conn_explorer::tree(cx).expect("the tree");
            tree.update(cx, |t, cx| t.set_expanded(tables, true, cx));
        });
        vcx.run_until_parked();
        let orders = key_of(vcx, "orders");
        vcx.cx.read(|cx| node_menu(orders, cx))
    };
    let orders_menu = orders_menu.expect("an object menu");
    assert!(orders_menu.iter().any(|m| m.action == "explorer.copy_name"));
    let _ = ObjectKind::Table;
}
