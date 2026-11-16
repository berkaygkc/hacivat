//! M3b Task 1b: every M3b/M3c command id is registered and bound as the plan's table lists; the
//! M3b stubs answer the safe value (Cancel, Roll back, Stop, no values) and draw nothing, so the
//! M3a shots hold; the `Results` key context joins the stack on the results tile's root; the six
//! new icons are bundled.

#![cfg_attr(not(unix), allow(dead_code, unused_imports, unused_variables))]

mod support;

use std::sync::Arc;
use std::time::Instant;

use gpui::{AppContext as _, AssetSource as _, Focusable as _, TestAppContext};
use hacivat_app::console::{
    ConsoleId, ConsoleState, Consoles, find_bar, idle_toast, tx_badge, tx_chip,
};
use hacivat_app::exec_ui::pending_core::{
    CommitAnswer, CommitQuestion, ConfirmAnswer, ConfirmPort as _, ConfirmQuestion, GateReason,
    ParamPrompt, RunChoice,
};
use hacivat_app::exec_ui::{
    AppConfirmPort, ErrorAsk, commit_prompt, confirm, error_bar, fetch_policy_for, params_prompt,
    policy_popover,
};
use hacivat_app::results::view::hosts;
use hacivat_app::results::{ConsoleResults, RESULTS_CONTEXT, count};
use hacivat_exec::{ResultTab, SessionLog, TabKey};
use hacivat_results::{
    CloseReason, Completeness, FetchControl, FetchPolicy, ResultId, ResultStore, StoreSource,
};
use hacivat_session::session::{ClientId, ExecError, ExecErrorKind, RunId, UnitId};
use hacivat_ui::actions::ActionRegistry;
use hacivat_ui::icon::IconName;
use hacivat_ui::keymaps::load;
use hacivat_ui_core::assets::BundledAssets;
use hacivat_ui_core::keymap::{Keymap, KeymapPlatform, chord_string};
use hacivat_values::{ColumnMeta, LogicalType, NativeType, ResultSchema, Value};
use support::*;

// ---- the keymap table ----------------------------------------------------------------------

/// The plan's table (Task 1b, P-M3b-21; P-M3b-T1b-2 for `select_all_occurrences`): id,
/// context, macOS chord, Windows/Linux chord. `None`: palette-only on that platform.
const BOUND: &[(&str, &str, Option<&str>, Option<&str>)] = &[
    (
        "editor.add_caret_above",
        "Editor",
        Some("cmd-alt-up"),
        Some("ctrl-alt-shift-up"),
    ),
    (
        "editor.add_caret_below",
        "Editor",
        Some("cmd-alt-down"),
        Some("ctrl-alt-shift-down"),
    ),
    (
        "editor.select_next_occurrence",
        "Editor",
        Some("ctrl-g"),
        Some("alt-j"),
    ),
    (
        "editor.select_all_occurrences",
        "Editor",
        Some("ctrl-cmd-g"),
        Some("ctrl-shift-l"),
    ),
    (
        "editor.undo_selection",
        "Editor",
        Some("cmd-u"),
        Some("ctrl-u"),
    ),
    ("editor.find", "Editor", Some("cmd-f"), Some("ctrl-f")),
    ("editor.find_next", "Editor", Some("cmd-g"), Some("f3")),
    (
        "editor.find_prev",
        "Editor",
        Some("cmd-shift-g"),
        Some("shift-f3"),
    ),
    (
        "editor.toggle_comment",
        "Editor",
        Some("cmd-/"),
        Some("ctrl-/"),
    ),
    (
        "editor.duplicate_line",
        "Editor",
        Some("cmd-d"),
        Some("ctrl-d"),
    ),
    (
        "editor.delete_line",
        "Editor",
        Some("cmd-shift-k"),
        Some("ctrl-y"),
    ),
    (
        "editor.move_line_up",
        "Editor",
        Some("alt-shift-up"),
        Some("alt-shift-up"),
    ),
    (
        "editor.move_line_down",
        "Editor",
        Some("alt-shift-down"),
        Some("alt-shift-down"),
    ),
    (
        "editor.history_prev",
        "Editor",
        Some("ctrl-alt-up"),
        Some("ctrl-alt-up"),
    ),
    (
        "editor.history_next",
        "Editor",
        Some("ctrl-alt-down"),
        Some("ctrl-alt-down"),
    ),
    (
        "grid.select_column",
        "Grid",
        Some("cmd-shift-space"),
        Some("ctrl-shift-space"),
    ),
    (
        "grid.grow_selection",
        "Grid",
        Some("alt-up"),
        Some("alt-shift-up"),
    ),
    (
        "grid.shrink_selection",
        "Grid",
        Some("alt-down"),
        Some("alt-shift-down"),
    ),
    ("grid.find", "Grid", Some("cmd-f"), Some("ctrl-f")),
    ("grid.goto_row", "Grid", Some("cmd-l"), Some("ctrl-g")),
    (
        "grid.open_value",
        "Grid",
        Some("shift-enter"),
        Some("shift-enter"),
    ),
    (
        "grid.context_menu",
        "Grid",
        Some("shift-f10"),
        Some("shift-f10"),
    ),
    (
        "results.next_tab",
        "Results",
        Some("ctrl-pagedown"),
        Some("ctrl-pagedown"),
    ),
    (
        "results.prev_tab",
        "Results",
        Some("ctrl-pageup"),
        Some("ctrl-pageup"),
    ),
    (
        "history.open",
        "Workspace",
        Some("cmd-shift-h"),
        Some("ctrl-shift-h"),
    ),
    // Palette-only.
    ("editor.split_selection_into_lines", "Editor", None, None),
    ("editor.redo_selection", "Editor", None, None),
    ("editor.fold", "Editor", None, None),
    ("editor.unfold", "Editor", None, None),
    ("editor.fold_all", "Editor", None, None),
    ("editor.unfold_all", "Editor", None, None),
    ("editor.find_close", "Editor", None, None),
    ("editor.normalize_line_endings", "Editor", None, None),
    ("grid.auto_fit", "Grid", None, None),
    ("grid.freeze", "Grid", None, None),
    ("grid.unfreeze", "Grid", None, None),
    ("grid.sort_asc", "Grid", None, None),
    ("grid.sort_desc", "Grid", None, None),
    ("grid.clear_sort", "Grid", None, None),
    ("grid.view_table", "Grid", None, None),
    ("grid.view_transposed", "Grid", None, None),
    ("grid.view_text", "Grid", None, None),
    ("grid.toggle_record", "Grid", None, None),
];

/// The datagrip preset's own rows: (id, context, platform, chord).
const DATAGRIP: &[(&str, &str, KeymapPlatform, &str)] = &[
    (
        "editor.delete_line",
        "Editor",
        KeymapPlatform::MacOs,
        "cmd-backspace",
    ),
    (
        "editor.select_all_occurrences",
        "Editor",
        KeymapPlatform::Windows,
        "ctrl-alt-shift-j",
    ),
    (
        "editor.select_all_occurrences",
        "Editor",
        KeymapPlatform::Linux,
        "ctrl-alt-shift-j",
    ),
    (
        "grid.grow_selection",
        "Grid",
        KeymapPlatform::Windows,
        "ctrl-w",
    ),
    (
        "grid.grow_selection",
        "Grid",
        KeymapPlatform::Linux,
        "ctrl-w",
    ),
    (
        "grid.shrink_selection",
        "Grid",
        KeymapPlatform::Windows,
        "ctrl-shift-w",
    ),
    (
        "grid.shrink_selection",
        "Grid",
        KeymapPlatform::Linux,
        "ctrl-shift-w",
    ),
];

/// `chord` in the keymap's canonical spelling (modifier order).
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

#[gpui::test]
fn every_m3b_command_id_is_registered_and_bound_as_listed(cx: &mut TestAppContext) {
    let reg = cx.update(|cx| {
        hacivat_app::init(cx);
        cx.global::<ActionRegistry>().clone()
    });
    for (id, context, mac, pc) in BOUND {
        let def = reg
            .get(id)
            .unwrap_or_else(|| panic!("{id} is not registered"));
        assert_eq!(def.context, *context, "{id}");
        assert_eq!(
            def.title_id,
            format!("hv-cmd-{}", id.replace(['.', '_'], "-")),
            "{id}"
        );
        for (platform, want) in [
            (KeymapPlatform::MacOs, mac),
            (KeymapPlatform::Windows, pc),
            (KeymapPlatform::Linux, pc),
        ] {
            let want: Vec<String> = want.iter().map(|c| canonical(platform, c)).collect();
            assert_eq!(
                chords(platform, "hacivat", id, context),
                want,
                "{id} {platform:?}"
            );
        }
    }
    for (id, context, platform, chord) in DATAGRIP {
        assert_eq!(
            chords(*platform, "datagrip", id, context),
            vec![canonical(*platform, chord)],
            "{id} {platform:?}"
        );
    }
    // The default `delete_to_line_start` chord on macOS goes to Delete line under datagrip.
    assert!(
        chords(
            KeymapPlatform::MacOs,
            "datagrip",
            "editor.delete_to_line_start",
            "Editor"
        )
        .is_empty()
    );
}

// ---- the stubs answer the safe value --------------------------------------------------------

fn confirm_question() -> ConfirmQuestion {
    ConfirmQuestion {
        run: RunId(1),
        unit: 0,
        index: 1,
        total: 1,
        sql_masked: "DELETE FROM t".into(),
        reasons: vec![GateReason {
            fluent_id: "exec-gate-no-where",
            args: vec![("table", "t".into())],
        }],
        type_to_confirm: None,
        irreversible: true,
        suppressible: false,
        warn_only: false,
        production: true,
        connection: "orders-prod".into(),
    }
}

fn commit_question() -> CommitQuestion {
    CommitQuestion {
        run: RunId(1),
        unit: UnitId {
            run: RunId(1),
            ordinal: 0,
        },
        affected: Some(3),
        deadline: Instant::now(),
        savepoint: true,
        production: true,
        connection: "orders-prod".into(),
    }
}

#[gpui::test]
fn confirm_stub_answers_cancel(cx: &mut TestAppContext) {
    let mut rx = cx.update(|cx| confirm::open(confirm_question(), cx));
    assert!(matches!(rx.try_recv(), Ok(ConfirmAnswer::Cancel)));
    let mut rx = AppConfirmPort.confirm(confirm_question());
    assert!(matches!(rx.try_recv(), Ok(ConfirmAnswer::Cancel)));
}

#[gpui::test]
fn commit_prompt_stub_answers_roll_back(cx: &mut TestAppContext) {
    let mut rx = cx.update(|cx| commit_prompt::open(commit_question(), cx));
    assert!(matches!(rx.try_recv(), Ok(CommitAnswer::RollBack)));
    let mut rx = AppConfirmPort.commit_decision(commit_question());
    assert!(matches!(rx.try_recv(), Ok(CommitAnswer::RollBack)));
}

#[gpui::test]
fn error_bar_and_params_stubs_answer_stop_and_cancel(cx: &mut TestAppContext) {
    let ask = ErrorAsk {
        unit: UnitId {
            run: RunId(1),
            ordinal: 0,
        },
        error: ExecError::of(ExecErrorKind::Internal),
        rollback_and_continue: false,
    };
    let mut rx = cx.update(|cx| error_bar::show(ConsoleId(1), ask, cx));
    assert!(matches!(rx.try_recv(), Ok(RunChoice::Stop)));
    let prompts = vec![ParamPrompt {
        name: "id".into(),
        occurrences: 1,
        remembered: None,
    }];
    let mut rx = cx.update(|cx| params_prompt::open(prompts, cx));
    assert!(matches!(rx.try_recv(), Ok(None)));
    // The run's fetch policy is M3a's until Task 25.
    let policy = cx.update(|cx| fetch_policy_for(ConsoleId(1), cx));
    assert_eq!(policy, FetchPolicy::default());
}

#[gpui::test]
fn new_slots_render_nothing_so_m3a_shots_hold(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let console = ConsoleId(1);
    vcx.cx.update(|cx| {
        assert!(tx_chip::render(console, cx).is_none());
        assert!(tx_badge::render(console, cx).is_none());
        assert!(idle_toast::render(console, cx).is_none());
        assert!(find_bar::render(console, cx).is_none());
        assert!(policy_popover::render(console, cx).is_none());
        assert!(count::render(console, cx).is_none());
    });
    // The history dialog: `history.open` is disabled (its chord passes through) and opens
    // nothing.
    assert!(!run(vcx, "history.open"));
    vcx.cx.update(hacivat_app::history::open);
    vcx.run_until_parked();
    assert!(layer_ids(vcx).is_empty());
    // The stub M3b commands are not listed in the palette.
    let listed: Vec<&str> = vcx
        .cx
        .read(hacivat_app::commands::palette::command_entries)
        .iter()
        .map(|e| e.id)
        .collect();
    for id in [
        "history.open",
        "results.next_tab",
        "grid.sort_asc",
        "grid.view_text",
    ] {
        assert!(!listed.contains(&id), "{id}");
    }
}

// ---- the `Results` context ------------------------------------------------------------------

struct NoControl;

impl FetchControl for NoControl {
    fn fetch_more(&self, _: ResultId, _: u64) {}
    fn close(&self, _: ResultId, _: CloseReason) {}
    fn fetch_all(&self, _: ResultId) {}
}

/// A finished one-column result of two rows.
fn one_result() -> ResultTab {
    let schema = Arc::new(ResultSchema {
        columns: Arc::from([ColumnMeta {
            name: "id".into(),
            logical: LogicalType::Int {
                bits: 64,
                signed: true,
            },
            native: NativeType::Pg {
                oid: 20,
                typmod: -1,
                name: "int8".into(),
            },
            origin: None,
        }]),
    });
    let store = ResultStore::from_rows(
        schema,
        vec![vec![Value::Int(1)], vec![Value::Int(2)]],
        Completeness::Complete,
    );
    ResultTab {
        key: TabKey {
            console: 1,
            unit_ordinal: 0,
            result_ordinal: 0,
        },
        title: "".into(),
        unit: UnitId {
            run: RunId(1),
            ordinal: 0,
        },
        source: StoreSource::new(store, Arc::new(NoControl), FetchPolicy::default()),
    }
}

#[cfg(unix)]
#[gpui::test]
fn results_context_is_active_on_the_results_root(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    let profile = add_profile(vcx, test_profile("orders-dev", None, "10.0.0.5"));
    let ws = workspace(vcx);
    let results = in_window(vcx, |window, cx| {
        let tab = ws
            .update(cx, |ws, cx| ws.open_console_tab(Some(profile), window, cx))
            .expect("a console tab opens");
        let log = SessionLog::new();
        let results = cx.new(|_| ConsoleResults::new(log.clone()));
        Consoles::register(
            cx,
            ConsoleState {
                id: ConsoleId(0),
                tab,
                n: 1,
                profile: Some(profile),
                client: ClientId::DEFAULT,
                session: None,
                editor: None,
                results: Some(results.clone()),
                sql: None,
                log,
                service: None,
            },
        );
        results
    });
    vcx.run_until_parked();
    in_window(vcx, |_, cx| {
        results.update(cx, |r, cx| {
            r.apply(&[one_result()], cx);
        })
    });
    in_window(vcx, |window, _| window.refresh());
    vcx.run_until_parked();
    assert!(!context_stack(vcx).0.contains(&RESULTS_CONTEXT));
    let host = vcx
        .cx
        .read(|cx| hosts(cx).into_iter().next())
        .expect("a results tile");
    in_window(vcx, |window, cx| {
        let handle = host.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    });
    in_window(vcx, |window, _| window.refresh());
    vcx.run_until_parked();
    let stack = context_stack(vcx);
    assert_eq!(stack.0.last(), Some(&RESULTS_CONTEXT), "{stack:?}");
    // The dispatcher resolves the result-tab chords there (disabled until Task 25).
    let keymap = active_keymap(vcx).keymap;
    assert!(
        keymap
            .bindings()
            .iter()
            .any(|b| b.command == "results.next_tab"
                && b.context == RESULTS_CONTEXT
                && chord_string(&b.chord) == canonical(keymap.platform(), "ctrl-pagedown"))
    );
}

// ---- icons ----------------------------------------------------------------------------------

/// The bundled bytes of `path` (`BundledAssets`, what `Application::with_assets` serves).
fn asset_bytes(path: &str) -> Option<Vec<u8>> {
    BundledAssets.load(path).ok().flatten().map(|b| b.to_vec())
}

#[test]
fn new_icons_are_embedded() {
    for n in [
        IconName::Play,
        IconName::Pin,
        IconName::Sigma,
        IconName::Filter,
        IconName::ArrowUpDown,
        IconName::Snowflake,
    ] {
        assert!(asset_bytes(n.path()).is_some(), "{n:?}");
    }
}

/// Controller ruling RC-4: every `IconName` (the list is checked complete in `hacivat-ui`)
/// resolves to a bundled SVG.
#[test]
fn every_icon_name_resolves_to_an_asset() {
    for n in IconName::ALL {
        let bytes = asset_bytes(n.path()).unwrap_or_else(|| panic!("{n:?}: no asset"));
        assert!(bytes.starts_with(b"<svg"), "{n:?}");
    }
}
