//! M2a Task 18: the explorer list of saved connections, its keyboard paths and menu, the
//! delete confirmation, the pg_service import, the connection commands and workspace Tab
//! traversal (NOTES surface 8; R19, R20, R24, R31).

mod support;

use std::time::Duration;

use gpui::{TestAppContext, VisualTestContext};
use hacivat_app::connections::ConnectionsModel;
use hacivat_app::connections::explorer;
use hacivat_app::connections::import::{self, ImportSources};
use hacivat_app::menus::menu_model;
use hacivat_proto::{EnvTag, ImportSource, Origin, ProfileId};
use hacivat_secrets::{Retention, SecretKey, SecretSlot, SecretString};
use hacivat_session::profile::ProfileDraft;
use hacivat_ui::actions::ActionRegistry;
use hacivat_ui::capsule::CapsuleModel;
use hacivat_ui::text::TextSource;
use hacivat_ui::toast::{Toast, ToastAction, ToastKind, ToastQueue};
use support::*;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn primary(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("cmd-{key}")
    } else {
        format!("ctrl-{key}")
    }
}

fn primary_shift(key: &str) -> String {
    format!("{}-shift-{key}", primary("").trim_end_matches('-'))
}

/// A connections shell over the fake PostgreSQL driver, in `lang`, with `profiles` saved.
fn with_profiles_lang<const N: usize>(
    cx: &mut TestAppContext,
    lang: &str,
    profiles: [(&str, Option<EnvTag>); N],
) -> &'static mut VisualTestContext {
    let vcx = shell_with_fake(
        cx,
        lang,
        hacivat_driver::fake::FakeDriver::new(hacivat_proto::DbmsKind::Postgres),
    );
    for (name, env) in profiles {
        add_profile(vcx, test_profile(name, env, "127.0.0.1"));
    }
    vcx
}

fn with_profiles<const N: usize>(
    cx: &mut TestAppContext,
    profiles: [(&str, Option<EnvTag>); N],
) -> &'static mut VisualTestContext {
    with_profiles_lang(cx, "en", profiles)
}

/// The explorer's texts from its panel title to the connection panel's, without the title,
/// the filter placeholder and the shortcut labels (Fluent texts only).
fn explorer_texts(vcx: &VisualTestContext) -> Vec<String> {
    let frame = registry_frame(vcx);
    let start = frame
        .iter()
        .position(|r| r.path.ends_with("#panel-explorer-title"))
        .expect("the explorer is drawn");
    frame[start + 1..]
        .iter()
        .take_while(|r| !r.path.ends_with("#panel-connection-title"))
        .filter(|r| r.source == TextSource::Fluent)
        .filter(|r| {
            !r.path.contains("hacivat-ui/src/input/") && !r.path.ends_with("#explorer-filter")
        })
        .map(|r| r.text.to_string())
        .collect()
}

/// The group headers with their counts (`PRODUCTION 1`), top down.
fn group_headers(vcx: &VisualTestContext) -> Vec<String> {
    (0..)
        .map_while(|g| {
            let title = text_of(vcx, &format!("explorer-group-{g}"))?;
            let count = text_of(vcx, &format!("explorer-group-count-{g}"))?;
            Some(format!("{title} {count}"))
        })
        .collect()
}

/// The visible rows' names, top down.
fn row_names(vcx: &VisualTestContext) -> Vec<String> {
    (0..)
        .map_while(|i| text_of(vcx, &format!("explorer-row-name-{i}")))
        .collect()
}

fn focus_list(vcx: &VisualTestContext) {
    in_window(vcx, explorer::focus_list);
    vcx.run_until_parked();
}

fn focus_filter(vcx: &VisualTestContext) {
    in_window(vcx, explorer::focus_filter);
    vcx.run_until_parked();
}

fn filter(vcx: &mut VisualTestContext, text: &str) {
    focus_filter(vcx);
    vcx.simulate_input(text);
    vcx.run_until_parked();
}

/// The profiles a connect was started for (any state but Disconnected), in list order.
fn connect_calls(vcx: &VisualTestContext) -> Vec<String> {
    vcx.cx.read(|cx| {
        let m = cx.global::<ConnectionsModel>();
        m.profiles()
            .iter()
            .filter(|p| m.manager().state(p.id) != hacivat_session::LiveState::Disconnected)
            .map(|p| p.name.clone())
            .collect()
    })
}

fn selected_id(vcx: &VisualTestContext) -> Option<ProfileId> {
    vcx.cx.read(|cx| cx.global::<ConnectionsModel>().selected())
}

/// The open connection menu's rows (`—` for a separator); each row is drawn where the menu has
/// an item and nowhere else.
fn menu_items(vcx: &mut VisualTestContext) -> Vec<String> {
    assert_eq!(layer_ids(vcx), ["menu"]);
    let id = selected_id(vcx).expect("a selection");
    let specs = vcx.cx.read(|cx| explorer::menu_items(cx, id));
    specs
        .iter()
        .enumerate()
        .map(|(i, spec)| {
            let drawn = vcx
                .debug_bounds(Box::leak(format!("menu-item-{i}").into_boxed_str()))
                .is_some();
            assert_eq!(drawn, !spec.separator, "row {i}");
            if spec.separator {
                "—".to_string()
            } else {
                vcx.cx.read(|cx| (spec.label)(cx).to_string())
            }
        })
        .collect()
}

fn clipboard(vcx: &VisualTestContext) -> String {
    vcx.cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default()
}

fn focused_button_label(vcx: &VisualTestContext) -> String {
    let dialog = vcx
        .cx
        .read(hacivat_app::connections::delete::open_dialog)
        .expect("the delete confirmation is open");
    let index = in_window(vcx, |window, cx| dialog.read(cx).focused_button(window))
        .expect("a button has the focus");
    ["Cancel", "Delete"][index].to_string()
}

fn db_key(id: ProfileId) -> SecretKey {
    SecretKey {
        profile: id,
        slot: SecretSlot::DbPassword,
    }
}

/// A connections shell with profile `name` saved with a password kept Forever.
fn with_saved_secret(
    cx: &mut TestAppContext,
    name: &str,
) -> (&'static mut VisualTestContext, ProfileId) {
    let vcx = with_profiles(cx, []);
    let mut profile = test_profile(name, Some(EnvTag::Dev), "127.0.0.1");
    profile.auth = hacivat_secrets::AuthProviderId::Password;
    profile.retention = Retention::Forever;
    let draft = ProfileDraft {
        password: Some(SecretString::from("hv_test_pw".to_string())),
        is_new: true,
        ..ProfileDraft::edit(profile)
    };
    let id = save_draft(vcx, draft);
    settle(vcx, ms(100));
    (vcx, id)
}

fn keyring_entry(vcx: &VisualTestContext, id: ProfileId) -> Option<SecretString> {
    secret_backend(vcx)
        .get(&db_key(id))
        .expect("the memory backend answers")
}

/// A connections shell whose import reads `text` as its only service file.
fn with_pg_service(cx: &mut TestAppContext, text: &str) -> &'static mut VisualTestContext {
    let vcx = with_profiles(cx, []);
    let path = paths(vcx).data.join("pg_service.conf");
    write(&path, text);
    vcx.cx.update(|cx| cx.set_global(ImportSources(vec![path])));
    vcx
}

fn import_view(vcx: &VisualTestContext) -> gpui::Entity<import::ImportView> {
    vcx.cx
        .read(import::open_view)
        .expect("the import dialog is open")
}

fn import_rows(vcx: &VisualTestContext) -> Vec<String> {
    settle(vcx, ms(100));
    let view = import_view(vcx);
    vcx.cx.read(|cx| view.read(cx).row_names())
}

fn import_env(vcx: &VisualTestContext, row: usize) -> Option<EnvTag> {
    let view = import_view(vcx);
    vcx.cx.read(|cx| view.read(cx).env(row))
}

fn set_import_env(vcx: &VisualTestContext, row: usize, env: Option<EnvTag>) {
    let view = import_view(vcx);
    vcx.cx
        .update(|cx| view.update(cx, |v, cx| v.set_env(row, env, cx)));
    vcx.run_until_parked();
}

fn import_button_disabled(vcx: &VisualTestContext) -> bool {
    let view = import_view(vcx);
    !vcx.cx.read(|cx| view.read(cx).can_import())
}

fn profile_origins(vcx: &VisualTestContext) -> Vec<Origin> {
    vcx.cx.read(|cx| {
        cx.global::<ConnectionsModel>()
            .profiles()
            .iter()
            .map(|p| p.origin.clone())
            .collect()
    })
}

fn menu_bar_action_ids(vcx: &VisualTestContext) -> Vec<String> {
    vcx.cx.read(|cx| {
        menu_model(cx.global::<ActionRegistry>())
            .into_iter()
            .flat_map(|g| g.items)
            .filter_map(|e| match e {
                hacivat_app::menus::MenuEntry::Action(id) => Some(id.to_string()),
                hacivat_app::menus::MenuEntry::Separator => None,
            })
            .collect()
    })
}

#[gpui::test]
fn m1_shell_without_connections_is_unchanged(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en"); // no ConnectionsBoot: M1 elements (P17)
    assert!(explorer_texts(vcx).contains(&"No connections yet".to_string()));
    assert!(matches!(
        capsule_model(vcx),
        CapsuleModel::Placeholder { .. }
    ));
    assert!(!vcx.update(|_, cx| cx.has_global::<ConnectionsModel>()));
    assert!(status_bar_text(vcx).starts_with("No connection"));
}

#[gpui::test]
fn empty_state_offers_three_entry_points(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    assert_eq!(
        explorer_texts(vcx),
        [
            "No connections yet",
            "Add a PostgreSQL, MySQL, MariaDB or SQLite database. If you paste a connection URL, Hacivat fills the form for you.",
            "New connection",
            "Paste URL",
            "Import pg_service.conf…"
        ]
    );
    // The status bar and the capsule agree: nothing is connected.
    assert!(status_bar_text(vcx).starts_with("No connection"));
    assert!(matches!(
        capsule_model(vcx),
        CapsuleModel::Placeholder { .. }
    ));
}

#[gpui::test]
fn rows_grouped_by_severity(cx: &mut TestAppContext) {
    let vcx = with_profiles(
        cx,
        [
            ("a", Some(EnvTag::Local)),
            ("b", Some(EnvTag::Production)),
            ("c", None),
            ("d", Some(EnvTag::Staging)),
        ],
    );
    assert_eq!(
        group_headers(vcx),
        ["PRODUCTION 1", "STAGING 1", "LOCAL 1", "NO ENVIRONMENT 1"]
    );
    assert_eq!(row_names(vcx), ["b", "d", "a", "c"]);
}

#[gpui::test]
fn keyboard_paths(cx: &mut TestAppContext) {
    let vcx = with_profiles(cx, [("a", Some(EnvTag::Dev)), ("b", Some(EnvTag::Dev))]);
    focus_list(vcx);
    vcx.simulate_keystrokes("down enter");
    assert_eq!(connect_calls(vcx), ["b"]);
    vcx.simulate_keystrokes(&primary("d"));
    settle(vcx, ms(50));
    assert_eq!(row_names(vcx), ["a", "b", "b-2"]);
    vcx.simulate_keystrokes(&primary_shift("c"));
    assert!(
        clipboard(vcx).starts_with("postgresql://"),
        "{}",
        clipboard(vcx)
    );
    // ⌘E opening the edit dialog needs Task 17's dialog; Task 21 tests it.
}

#[gpui::test]
fn backspace_in_filter_does_not_delete(cx: &mut TestAppContext) {
    let vcx = with_profiles(cx, [("a", Some(EnvTag::Dev))]);
    focus_filter(vcx);
    vcx.simulate_input("ab");
    assert!(row_names(vcx).is_empty());
    vcx.simulate_keystrokes("backspace backspace backspace");
    assert!(layer_ids(vcx).is_empty());
    assert_eq!(row_names(vcx), ["a"]);
}

#[gpui::test]
fn escape_in_the_filter_runs_the_escape_hook(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(
        cx,
        "en",
        hacivat_driver::fake::FakeDriver::new(hacivat_proto::DbmsKind::Postgres)
            .delay(Duration::from_secs(5)),
    );
    add_profile(vcx, test_profile("a", Some(EnvTag::Dev), "127.0.0.1"));
    connect(vcx, "a");
    focus_filter(vcx);
    vcx.simulate_keystrokes("escape"); // the field swallows Escape and reports it (P16)
    settle(vcx, ms(50));
    assert_eq!(live(vcx, "a"), hacivat_session::LiveState::Disconnected);
    assert_eq!(toast_texts(vcx), ["Connecting to a was cancelled."]);
}

#[gpui::test]
fn context_menu_items(cx: &mut TestAppContext) {
    let vcx = with_profiles(cx, [("a", Some(EnvTag::Dev))]);
    focus_list(vcx);
    vcx.simulate_keystrokes("shift-f10");
    assert_eq!(
        menu_items(vcx),
        [
            "Connect",
            "Edit…",
            "Duplicate",
            "Copy as URL",
            "—",
            "Delete…"
        ]
    );
}

#[gpui::test]
fn delete_confirms_with_cancel_focused_and_removes_secrets(cx: &mut TestAppContext) {
    let (vcx, id) = with_saved_secret(cx, "a");
    assert!(keyring_entry(vcx, id).is_some());
    focus_list(vcx);
    vcx.simulate_keystrokes("backspace");
    assert_eq!(dialog_title(vcx), "Delete a?");
    assert_eq!(focused_button_label(vcx), "Cancel");
    vcx.simulate_keystrokes("tab enter");
    settle(vcx, ms(100));
    assert!(row_names(vcx).is_empty());
    assert!(keyring_entry(vcx, id).is_none());
    assert_eq!(toast_texts(vcx), ["Deleted a"]);
}

#[gpui::test]
fn filter_is_turkish_tolerant_for_tr(cx: &mut TestAppContext) {
    let vcx = with_profiles_lang(
        cx,
        "tr",
        [
            ("Müşteri-prod", Some(EnvTag::Production)),
            ("orders", Some(EnvTag::Dev)),
        ],
    );
    filter(vcx, "musteri");
    assert_eq!(row_names(vcx), ["Müşteri-prod"]);
}

#[gpui::test]
fn pg_service_import_requires_env_for_remote(cx: &mut TestAppContext) {
    let vcx = with_pg_service(
        cx,
        "[orders-prod]\nhost=db-prod\ndbname=orders\n[local]\nhost=localhost\n[orders-prod]\nhost=ignored\n",
    );
    run(vcx, "connection.import_pg_service");
    assert_eq!(import_rows(vcx), ["orders-prod", "local"]);
    assert_eq!(layer_ids(vcx), ["import"]);
    assert_eq!(import_env(vcx, 0), Some(EnvTag::Production));
    assert_eq!(import_env(vcx, 1), Some(EnvTag::Local));
    set_import_env(vcx, 0, None);
    assert!(import_button_disabled(vcx));
    set_import_env(vcx, 0, Some(EnvTag::Staging));
    assert!(!import_button_disabled(vcx));
    click(vcx, "button-import-confirm");
    settle(vcx, ms(100));
    assert!(layer_ids(vcx).is_empty());
    assert_eq!(
        profile_origins(vcx),
        vec![
            Origin::Imported {
                source: ImportSource::PgService
            };
            2
        ]
    );
    assert_eq!(toast_texts(vcx), ["Imported 2 connections"]);
}

#[gpui::test]
fn an_imported_name_that_is_taken_gets_a_suffix(cx: &mut TestAppContext) {
    let vcx = with_pg_service(cx, "[orders]\nhost=localhost\n");
    add_profile(vcx, test_profile("orders", Some(EnvTag::Dev), "127.0.0.1"));
    run(vcx, "connection.import_pg_service");
    assert_eq!(import_rows(vcx), ["orders-2"]);
}

#[gpui::test]
fn only_connection_new_is_in_the_menu_bar(cx: &mut TestAppContext) {
    let ids = menu_bar_action_ids(shell_with_connections(cx, "en"));
    assert!(ids.contains(&"connection.new".to_string()));
    assert_eq!(
        ids.iter().filter(|i| i.starts_with("connection.")).count(),
        1
    );
}

#[gpui::test]
fn tab_moves_between_the_filter_and_the_list(cx: &mut TestAppContext) {
    let vcx = with_profiles(cx, [("a", Some(EnvTag::Dev))]);
    focus_filter(vcx);
    assert!(!context_stack(vcx).0.contains(&"Explorer"));
    vcx.simulate_keystrokes("tab");
    assert_eq!(context_stack(vcx).0.last(), Some(&"Explorer"));
    // Focusing the list selected its first row.
    assert_eq!(selected_id(vcx), Some(profile_id(vcx, "a")));
    vcx.simulate_keystrokes("shift-tab");
    assert!(!context_stack(vcx).0.contains(&"Explorer"));
    vcx.simulate_input("zz");
    assert!(row_names(vcx).is_empty(), "the filter has the focus again");
}

#[gpui::test]
fn a_dialog_keeps_tab_inside_it(cx: &mut TestAppContext) {
    let (vcx, _) = with_saved_secret(cx, "a");
    focus_list(vcx);
    vcx.simulate_keystrokes("backspace tab tab");
    // Cancel → Delete → Cancel: the workspace never moved the focus out of the dialog.
    assert_eq!(layer_ids(vcx), ["dialog"]);
    assert_eq!(focused_button_label(vcx), "Cancel");
}

#[gpui::test]
fn f6_moves_the_focus_to_the_newest_sticky_toast(cx: &mut TestAppContext) {
    let vcx = with_profiles(cx, []);
    vcx.cx.update(|cx| {
        let mut toast = Toast::new(
            ToastKind::Info,
            hacivat_app::l10n_global::text_fn_id("hv-explorer-copied"),
        );
        toast.sticky = true;
        toast.actions = vec![ToastAction {
            label: hacivat_app::l10n_global::text_fn_id("hv-capsule-retry"),
            run: std::rc::Rc::new(|_, _| {}),
        }];
        ToastQueue::push(cx, toast);
    });
    vcx.run_until_parked();
    let before = focused(vcx);
    vcx.simulate_keystrokes("f6");
    assert_ne!(focused(vcx), before);
    vcx.simulate_keystrokes("escape"); // a focused toast dismisses on Escape
    assert!(vcx.cx.read(ToastQueue::visible_ids).is_empty());
}

/// Whether the registered command `id` is enabled now.
fn enabled(vcx: &VisualTestContext, id: &str) -> bool {
    vcx.cx.read(|cx| {
        cx.global::<ActionRegistry>()
            .get(id)
            .is_some_and(|d| (d.enabled)(cx))
    })
}

/// Review fix 1: a filter that hides the selection moves it to the first visible row (or
/// clears it), so Enter, Backspace and ⌘D never act on a hidden profile.
#[gpui::test]
fn explorer_commands_never_act_on_a_hidden_row(cx: &mut TestAppContext) {
    let vcx = with_profiles(
        cx,
        [
            ("orders-prod", Some(EnvTag::Production)),
            ("orders-dev", Some(EnvTag::Dev)),
        ],
    );
    focus_list(vcx); // selects the first row, orders-prod
    assert_eq!(selected_id(vcx), Some(profile_id(vcx, "orders-prod")));
    filter(vcx, "dev");
    assert_eq!(row_names(vcx), ["orders-dev"]);
    assert_eq!(selected_id(vcx), Some(profile_id(vcx, "orders-dev")));
    focus_list(vcx);
    vcx.simulate_keystrokes("enter");
    assert_eq!(connect_calls(vcx), ["orders-dev"]);
    // Nothing visible: nothing selected, and the explorer commands are disabled.
    focus_filter(vcx);
    vcx.simulate_input("zzz");
    assert!(row_names(vcx).is_empty());
    assert_eq!(selected_id(vcx), None);
    for id in [
        "connection.toggle_connect",
        "connection.delete",
        "connection.duplicate",
        "connection.copy_url",
    ] {
        assert!(!enabled(vcx, id), "{id}");
    }
    // A selection the filter hides (set from elsewhere) is not acted on either.
    let prod = profile_id(vcx, "orders-prod");
    vcx.cx.update(|cx| ConnectionsModel::select(cx, Some(prod)));
    assert!(!run(vcx, "connection.delete"));
    assert!(!run(vcx, "connection.duplicate"));
    assert!(layer_ids(vcx).is_empty());
    assert_eq!(row_names(vcx), Vec::<String>::new());
    assert_eq!(connect_calls(vcx), ["orders-dev"]);
}

/// Review fix 4: a read-only store (a newer file) disables every mutating command; the menu
/// shows them disabled, and connecting still works.
#[gpui::test]
fn a_read_only_store_disables_the_mutating_commands(cx: &mut TestAppContext) {
    let vcx = shell_with_connections_file(
        cx,
        "en",
        "version = 2\n[[connection]]\nid = \"6f9619ff-8b86-d011-b42d-00c04fc964ff\"\nname = \"a\"\nengine = \"postgres\"\nhost = \"127.0.0.1\"\nenvironment = \"dev\"\n",
    );
    focus_list(vcx);
    assert_eq!(row_names(vcx), ["a"]);
    for id in [
        "connection.new",
        "connection.paste_url",
        "connection.import_pg_service",
        "connection.edit",
        "connection.duplicate",
        "connection.delete",
    ] {
        assert!(!enabled(vcx, id), "{id}");
    }
    assert!(enabled(vcx, "connection.toggle_connect"));
    assert!(enabled(vcx, "connection.copy_url"));
    vcx.simulate_keystrokes("backspace");
    assert!(layer_ids(vcx).is_empty(), "no delete confirmation");
    let id = selected_id(vcx).expect("selected");
    let disabled: Vec<bool> = vcx.cx.read(|cx| {
        explorer::menu_items(cx, id)
            .iter()
            .filter(|i| !i.separator)
            .map(|i| i.disabled)
            .collect()
    });
    // Connect, Edit…, Duplicate, Copy as URL, Delete…
    assert_eq!(disabled, [false, true, true, false, true]);
}

/// Review minor: the home entry points are real buttons (tab stops) that run their command.
#[gpui::test]
fn home_entry_points_are_buttons(cx: &mut TestAppContext) {
    let vcx = with_profiles(cx, [("a", Some(EnvTag::Dev))]);
    for id in [
        "button-home-new",
        "button-home-paste",
        "button-home-connect",
        "button-home-palette",
    ] {
        assert!(vcx.debug_bounds(id).is_some(), "{id}");
    }
    focus_list(vcx); // selects a
    click(vcx, "button-home-connect");
    assert_eq!(connect_calls(vcx), ["a"]);
    click(vcx, "button-home-palette");
    assert_eq!(layer_ids(vcx), ["palette"]);
}

// ## m2a-t21-shots

#[gpui::test]
fn only_delete_is_red_in_the_connection_menu(cx: &mut TestAppContext) {
    let vcx = with_profiles(cx, [("a", Some(EnvTag::Dev))]);
    let id = profile_id(vcx, "a");
    let danger: Vec<(String, bool)> = vcx.cx.read(|cx| {
        explorer::menu_items(cx, id)
            .iter()
            .filter(|s| !s.separator)
            .map(|s| ((s.label)(cx).to_string(), s.danger))
            .collect()
    });
    assert_eq!(
        danger,
        [
            ("Connect".to_string(), false),
            ("Edit…".to_string(), false),
            ("Duplicate".to_string(), false),
            ("Copy as URL".to_string(), false),
            ("Delete…".to_string(), true),
        ]
    );
}
