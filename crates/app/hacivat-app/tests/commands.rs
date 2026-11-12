//! Commands (Task 16): the action registry, the UI-EDIT-13 key dispatcher, the command palette,
//! the localized menus, About and the info toast. Line 4 of the full test set runs it with
//! `scene-check` and `pseudo`.

mod support;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, ClipboardItem, Entity, EntityInputHandler as _, Focusable as _, KeyDownEvent, Keystroke,
    Menu, MenuItem, Modifiers, TestAppContext, VisualContext as _, VisualTestContext, point, px,
};
use hacivat_app::PendingChord;
#[cfg(debug_assertions)]
use hacivat_app::commands::debug::install_focus_move;
use hacivat_app::commands::dispatch::{DispatchOutcome, Dispatcher, PENDING_TIMEOUT};
use hacivat_app::commands::palette::{CommandEntry, open_palette_view, rank_commands};
use hacivat_app::commands::register_workspace_actions;
use hacivat_app::menus::{MENU_KEY_CONTEXT, build_menus, menu_model, popup_items};
use hacivat_app::shell::keymap_state::current_platform;
use hacivat_app::shell::set_keymap_preset;
use hacivat_l10n::{
    Catalogs, FormatInputs, L10n, MatchMode, NegotiationCtx, PrefSource, SearchOpts, build_runtime,
    negotiate,
};
use hacivat_settings::KeymapPreset;
use hacivat_ui::accesskey::AccessKeyLocale;
use hacivat_ui::actions::ActionRegistry;
use hacivat_ui::input::FocusedInput;
use hacivat_ui::keymaps::{PRESETS, load};
use hacivat_ui::menu::{MenuItemSpec, PopupMenuEvent};
use hacivat_ui::palette::{Palette, PaletteItem};
use hacivat_ui_core::gpui_surface::RunCommand;
use hacivat_ui_core::keymap::{ContextStack, KeyboardLayoutId, Keymap, KeymapPlatform};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use support::*;

// ---- helpers -----------------------------------------------------------------------------------

/// `cmd-<key>` on macOS, `ctrl-<key>` elsewhere.
fn primary(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("cmd-{key}")
    } else {
        format!("ctrl-{key}")
    }
}

const ALL_PLATFORMS: [KeymapPlatform; 3] = [
    KeymapPlatform::MacOs,
    KeymapPlatform::Windows,
    KeymapPlatform::Linux,
];

/// The open palette view.
fn palette(vcx: &VisualTestContext) -> Entity<Palette> {
    vcx.cx.read(open_palette_view).expect("the palette is open")
}

fn open_palette(vcx: &VisualTestContext) {
    assert!(run(vcx, "workspace.command_palette"));
}

fn palette_items(vcx: &VisualTestContext) -> Vec<PaletteItem> {
    vcx.run_until_parked();
    let p = palette(vcx);
    vcx.cx.read(|cx| p.read(cx).items().to_vec())
}

fn palette_item(vcx: &VisualTestContext, id: &str) -> PaletteItem {
    palette_items(vcx)
        .into_iter()
        .find(|i| i.id == id)
        .unwrap_or_else(|| panic!("no palette row `{id}`"))
}

fn palette_query(vcx: &VisualTestContext) -> String {
    let p = palette(vcx);
    vcx.cx.read(|cx| p.read(cx).query())
}

fn palette_input_focused(vcx: &VisualTestContext) -> bool {
    let p = palette(vcx);
    in_window(vcx, |window, cx| {
        p.read(cx).focus_handle(cx).is_focused(window)
    })
}

fn enabled_action_count(vcx: &VisualTestContext) -> usize {
    vcx.cx.read(|cx| {
        cx.global::<ActionRegistry>()
            .all()
            .iter()
            .filter(|d| (d.enabled)(cx))
            .count()
    })
}

/// Wraps every registered action so running it is recorded (by ID).
fn record_actions(vcx: &VisualTestContext) -> Rc<RefCell<Vec<String>>> {
    let ran: Rc<RefCell<Vec<String>>> = Rc::default();
    let sink = ran.clone();
    vcx.cx.update(|cx| {
        let old = cx.global::<ActionRegistry>().clone();
        let mut new = ActionRegistry::default();
        for def in old.all() {
            let mut def = def.clone();
            let (inner, sink, id) = (def.run.clone(), sink.clone(), def.id);
            def.run = Rc::new(move |window, cx| {
                sink.borrow_mut().push(id.to_string());
                inner(window, cx)
            });
            new.register(def).expect("unique IDs");
        }
        cx.set_global(new);
    });
    ran
}

/// Makes the shell's window the active one and repaints: a `TextInput` publishes itself as the
/// `FocusedInput` (which the dispatcher reads for `composing`) only in the active window.
fn activate(vcx: &VisualTestContext) {
    in_window(vcx, |window, cx| {
        window.activate_window();
        cx.refresh_windows();
    });
    vcx.run_until_parked();
}

/// Starts an IME composition of `text` in the palette's query field (`replace_and_mark`).
fn compose_in_palette(vcx: &VisualTestContext, text: &str) {
    activate(vcx);
    let input = vcx.cx.read(|cx| palette(vcx).read(cx).input().clone());
    in_window(vcx, |window, cx| {
        input.update(cx, |i, cx| {
            i.replace_and_mark_text_in_range(None, text, None, window, cx)
        })
    });
    vcx.run_until_parked();
}

fn commit_palette_composition(vcx: &VisualTestContext) {
    let input = vcx.cx.read(|cx| palette(vcx).read(cx).input().clone());
    in_window(vcx, |window, cx| {
        input.update(cx, |i, cx| i.unmark_text(window, cx))
    });
    vcx.run_until_parked();
}

/// Starts an IME composition of `text` in the test input focused by `handle`.
fn compose_in(vcx: &VisualTestContext, handle: &gpui::FocusHandle, text: &str) {
    activate(vcx);
    let input = test_input(vcx, handle);
    in_window(vcx, |window, cx| {
        input.update(cx, |i, cx| {
            i.replace_and_mark_text_in_range(None, text, None, window, cx)
        })
    });
    vcx.run_until_parked();
}

/// The registry `register_workspace_actions` builds.
fn registry() -> ActionRegistry {
    let mut reg = ActionRegistry::default();
    register_workspace_actions(&mut reg);
    reg
}

/// A runtime over the embedded catalogs in `lang`.
fn l10n_for(lang: &str) -> L10n {
    let catalogs = Catalogs::embedded();
    let ctx = NegotiationCtx {
        sources: vec![PrefSource::Setting(lang.to_string())],
        ..Default::default()
    };
    let neg = negotiate(&ctx, catalogs.locales());
    L10n::new(build_runtime(
        &catalogs,
        &neg,
        &FormatInputs::new(&neg, "UTC"),
        1,
    ))
}

/// Every registered action as a palette entry, titled in `lang`, unbound.
fn fixture_items(lang: &str) -> Vec<CommandEntry> {
    let l = l10n_for(lang);
    registry()
        .all()
        .iter()
        .map(|d| CommandEntry {
            id: d.id,
            title: l.t_dynamic(d.title_id, Vec::new()).into(),
            english_title: l.t_english(d.title_id).into(),
            keys: None,
        })
        .collect()
}

fn fixture_items_tr() -> Vec<CommandEntry> {
    fixture_items("tr")
}

fn fixture_items_en() -> Vec<CommandEntry> {
    fixture_items("en")
}

/// Turkish-tolerant fuzzy search.
fn tol() -> SearchOpts {
    SearchOpts {
        turkish_tolerant: true,
        mode: MatchMode::Fuzzy,
    }
}

/// Plain fuzzy search.
fn off() -> SearchOpts {
    SearchOpts {
        turkish_tolerant: false,
        mode: MatchMode::Fuzzy,
    }
}

/// The names of every action item of the application menus, in order.
fn action_item_names(menus: &[Menu]) -> Vec<String> {
    menus
        .iter()
        .flat_map(|m| m.items.iter())
        .filter_map(|item| match item {
            MenuItem::Action { name, .. } => Some(name.to_string()),
            _ => None,
        })
        .collect()
}

/// The labels of the in-window menu's items (separators skipped), in order.
fn popup_labels(items: &[MenuItemSpec], cx: &App) -> Vec<String> {
    items
        .iter()
        .filter(|i| !i.separator)
        .map(|i| (i.label)(cx).to_string())
        .collect()
}

// ---- the brief's tests -------------------------------------------------------------------------

#[gpui::test]
fn primary_k_opens_palette_with_every_action(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    vcx.simulate_keystrokes(&primary("k"));
    assert_eq!(layer_ids(vcx), ["palette"]);
    assert_eq!(palette_items(vcx).len(), enabled_action_count(vcx));
    assert!(palette_item(vcx, "workspace.open_settings").keys.is_some());
}

#[gpui::test]
fn altgr_at_sign_reaches_palette_input(cx: &mut TestAppContext) {
    let vcx = shell_with_keymap(
        cx,
        KeymapPlatform::Windows,
        "tr-q",
        Some(
            "[[binding]]\nchord=\"ctrl-alt-q\"\ncommand=\"workspace.toggle_theme\"\ncontext=\"Workspace\"",
        ),
    );
    open_palette(vcx);
    let ran = record_actions(vcx);
    vcx.simulate_event(KeyDownEvent {
        keystroke: Keystroke {
            modifiers: Modifiers {
                control: true,
                alt: true,
                ..Default::default()
            },
            key: "q".into(),
            key_char: Some("@".into()),
        },
        is_held: false,
        prefer_character_input: false,
    });
    vcx.simulate_input("@");
    vcx.run_until_parked();
    assert!(ran.take().is_empty());
    assert_eq!(vcx.read(|cx| Theme::get(cx).mode), ThemeMode::Dark);
    assert_eq!(palette_query(vcx), "@");
}

#[gpui::test]
fn bindings_do_not_fire_while_composing(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    run(vcx, "workspace.new_console_tab");
    assert_eq!(tab_count(vcx), 2);
    open_palette(vcx);
    compose_in_palette(vcx, "ş"); // replace_and_mark through the input handler
    assert!(vcx.read(|cx| cx.global::<FocusedInput>().composing));
    vcx.simulate_keystrokes(&primary("w"));
    assert_eq!(tab_count(vcx), 2);
    commit_palette_composition(vcx);
    vcx.simulate_keystrokes("escape");
    assert!(layer_ids(vcx).is_empty());
    vcx.simulate_keystrokes(&primary("w"));
    assert_eq!(tab_count(vcx), 1);
}

#[gpui::test]
fn switch_language_with_palette_open_keeps_query(cx: &mut TestAppContext) {
    let vcx = shell(cx, "tr");
    open_palette(vcx);
    vcx.simulate_input("ayar");
    switch_to(vcx, "en");
    vcx.run_until_parked();
    assert_eq!(palette_query(vcx), "ayar");
    assert_eq!(
        palette_item(vcx, "workspace.open_settings").title,
        "Open settings"
    );
}

#[gpui::test]
fn dispatcher_follows_active_keymap(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    vcx.update(|_, cx| {
        set_keymap_preset(cx, KeymapPreset::Datagrip);
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes(&primary("shift-a"));
    assert_eq!(layer_ids(vcx), ["palette"]);
}

#[test]
fn ranking_prefers_word_match_then_english_layer() {
    let r = rank_commands("ayar", &fixture_items_tr(), &[], &tol());
    assert_eq!(r[0].id, "workspace.open_settings");
    let r = rank_commands("settings", &fixture_items_tr(), &[], &tol());
    assert_eq!(r[0].id, "workspace.open_settings");
    let r = rank_commands("AYARLARI", &fixture_items_tr(), &[], &tol());
    assert_eq!(r[0].id, "workspace.open_settings");
}

#[test]
fn action_id_is_the_third_layer() {
    assert_eq!(
        rank_commands("open_settings", &fixture_items_tr(), &[], &tol())[0].id,
        "workspace.open_settings"
    );
    assert_eq!(
        rank_commands("workspace.open", &fixture_items_en(), &[], &off())[0].id,
        "workspace.open_settings"
    );
    let r = rank_commands("app", &fixture_items_en(), &[], &off()); // fixture titles: app.show_menu "Show app menu", app.about "About Hacivat"
    assert_eq!(r[0].id, "app.show_menu");
    assert!(r.iter().any(|i| i.id == "app.about")); // a title match outranks ID-only matches
}

#[test]
fn recent_breaks_ties() {
    let r = rank_commands("", &fixture_items_en(), &["app.about".into()], &off());
    assert_eq!(r[0].id, "app.about");
}

/// Review B1: recency outranks a better score inside the same match kind, not only exact ties.
/// For `tab` every tab title is an ExactWord match; "Next tab" scores best (earliest match).
#[test]
fn recent_leads_matches_of_the_same_kind() {
    let ids = |r: Vec<PaletteItem>| r.into_iter().map(|i| i.id.to_string()).collect::<Vec<_>>();
    let plain = ids(rank_commands("tab", &fixture_items_en(), &[], &off()));
    assert_eq!(plain[0], "workspace.next_tab");
    let at = |v: &[String], id: &str| v.iter().position(|x| x == id).unwrap();
    assert!(at(&plain, "workspace.close_tab") < at(&plain, "workspace.new_console_tab"));
    let r = ids(rank_commands(
        "tab",
        &fixture_items_en(),
        &["workspace.close_tab".into()],
        &off(),
    ));
    assert_eq!(r[0], "workspace.close_tab");
    assert!(at(&r, "workspace.close_tab") < at(&r, "workspace.new_console_tab"));
    let r = ids(rank_commands(
        "tab",
        &fixture_items_en(),
        &["workspace.new_console_tab".into()],
        &off(),
    ));
    assert_eq!(r[0], "workspace.new_console_tab");
}

/// Review B1: a better match kind still beats recency. For `set`, "Open settings" is a Prefix
/// title match; the recent "Switch between dark and light theme" only matches fuzzily.
#[test]
fn a_better_kind_beats_recency() {
    let r = rank_commands(
        "set",
        &fixture_items_en(),
        &["workspace.toggle_theme".into()],
        &off(),
    );
    assert_eq!(r[0].id, "workspace.open_settings");
    assert!(r.iter().any(|i| i.id == "workspace.toggle_theme"));
}

#[test]
fn every_action_title_exists_in_en() {
    let (reg, l) = (registry(), l10n_en());
    for d in reg.all() {
        assert!(!l.t_english(d.title_id).starts_with('⟦'), "{}", d.title_id);
    }
}

#[test]
fn keymap_commands_are_registered() {
    let reg = registry();
    for p in ALL_PLATFORMS {
        for preset in PRESETS {
            for b in load(p, preset, None).0.bindings() {
                assert!(reg.get(&b.command).is_some(), "{}", b.command);
            }
        }
    }
}

#[gpui::test]
fn menus_follow_language(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let names = |vcx: &mut VisualTestContext| {
        vcx.update(|_, cx| {
            build_menus(&menu_model(cx.global::<ActionRegistry>()), cx)
                .iter()
                .map(|m| m.name.to_string())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(names(vcx), ["Hacivat", "View", "Window", "Help"]);
    switch_to(vcx, "tr");
    vcx.run_until_parked();
    assert_eq!(names(vcx), ["Hacivat", "Görünüm", "Pencere", "Yardım"]);
}

#[gpui::test]
fn in_window_menu_matches_app_menu(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let (app_menu, popup) = vcx.update(|_, cx| {
        let m = menu_model(cx.global::<ActionRegistry>());
        (
            action_item_names(&build_menus(&m, cx)),
            popup_labels(&popup_items(&m, cx), cx),
        )
    });
    assert!(!popup.is_empty());
    assert_eq!(popup, app_menu); // same actions, same order, same labels
}

#[gpui::test]
fn about_dialog_is_localized(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    run(vcx, "app.about");
    assert_eq!(dialog_title(vcx), "About Hacivat");
    switch_to(vcx, "tr");
    vcx.run_until_parked();
    assert_eq!(dialog_title(vcx), "Hacivat Hakkında");
}

#[gpui::test]
fn reload_translations_shows_info_toast(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    run(vcx, "workspace.reload_translations");
    vcx.run_until_parked();
    assert_eq!(toast_texts(vcx), ["Translations reloaded"]);
}

#[gpui::test]
fn pending_chord_shows_in_status_bar(cx: &mut TestAppContext) {
    let vcx = shell_with_keymap(
        cx,
        current_platform(),
        "us",
        Some(
            "[[binding]]\nchord=\"ctrl-j ctrl-s\"\ncommand=\"workspace.toggle_theme\"\ncontext=\"Workspace\"",
        ),
    );
    vcx.simulate_keystrokes("ctrl-j");
    let want = if cfg!(target_os = "macos") {
        "⌃J"
    } else {
        "Ctrl+J"
    };
    assert_eq!(
        vcx.read(|cx| cx.global::<PendingChord>().0.clone())
            .as_deref(),
        Some(want)
    );
    assert!(status_bar_text(vcx).contains(want));
    vcx.executor().advance_clock(Duration::from_millis(1001));
    vcx.run_until_parked();
    assert_eq!(vcx.read(|cx| cx.global::<PendingChord>().0.clone()), None);
}

#[cfg(debug_assertions)]
#[gpui::test]
fn debug_focus_move_fires_after_first_composition(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    vcx.update(|_, cx| install_focus_move(cx, Duration::from_millis(50)));
    let input = focus_test_input(vcx);
    compose_in(vcx, &input, "か");
    vcx.executor().advance_clock(Duration::from_millis(60));
    vcx.run_until_parked();
    assert_eq!(layer_ids(vcx), ["palette"]);
    assert!(palette_input_focused(vcx));
}

// ---- wave-4 notes and rulings ------------------------------------------------------------------

/// Parked from Task 12: under a Turkic UI, typing `i` chooses the item whose access key is `İ`
/// and `ı` chooses `I`, through the shell's `open_popup_menu`.
#[gpui::test]
fn popup_menu_access_keys_fold_dotted_and_dotless_i(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    vcx.update(|_, cx| cx.set_global(AccessKeyLocale { turkic: true }));
    let open = |vcx: &VisualTestContext| {
        let items = vec![
            MenuItemSpec::new(Rc::new(|_: &App| "İçe aktar".into()), "test.dotted")
                .accesskey(Rc::new(|_: &App| Some('İ'))),
            MenuItemSpec::new(Rc::new(|_: &App| "Ilık".into()), "test.dotless")
                .accesskey(Rc::new(|_: &App| Some('I'))),
        ];
        let menu = with_ws(vcx, |w, window, cx| {
            w.open_popup_menu(point(px(10.0), px(50.0)), items, window, cx)
        });
        let chosen: Rc<RefCell<Vec<String>>> = Rc::default();
        let sink = chosen.clone();
        vcx.cx.update(|cx| {
            menu.update(cx, |m, _| m.set_access_keys(true));
            cx.subscribe(&menu, move |_, e: &PopupMenuEvent, _| {
                if let PopupMenuEvent::Chosen(a) = e {
                    sink.borrow_mut().push(a.to_string());
                }
            })
            .detach();
        });
        vcx.run_until_parked();
        chosen // the menu handle is dropped: only the shell keeps the menu
    };
    let chosen = open(vcx);
    vcx.simulate_keystrokes("i");
    assert_eq!(chosen.take(), ["test.dotted"]);
    assert!(layer_ids(vcx).is_empty());
    let chosen = open(vcx);
    vcx.simulate_keystrokes("ı");
    assert_eq!(chosen.take(), ["test.dotless"]);
}

#[gpui::test]
fn explorer_focus_filter_is_bound_and_shows_the_explorer(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let want = if cfg!(target_os = "macos") {
        "⌘F"
    } else {
        "Ctrl+F"
    };
    assert_eq!(
        vcx.read(|cx| {
            hacivat_app::shell::keymap_state::shortcut_label(cx, "explorer.focus_filter")
        })
        .as_deref(),
        Some(want)
    );
    run(vcx, "workspace.toggle_explorer");
    assert!(!layout(vcx).1.explorer_visible);
    vcx.simulate_keystrokes(&primary("f"));
    assert!(layout(vcx).1.explorer_visible);
}

/// T11 forward note: the menus bind no clipboard key equivalents, so Cmd/Ctrl-A/C/X/V still
/// reach the focused `TextInput` with the menus installed.
#[gpui::test]
fn clipboard_keys_reach_the_text_input_with_menus_installed(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    vcx.update(|_, cx| hacivat_app::menus::rebuild(cx));
    let bound: Vec<(bool, bool, String)> = vcx.read(|cx| {
        cx.key_bindings()
            .borrow()
            .bindings()
            .flat_map(|b| {
                b.keystrokes()
                    .iter()
                    .map(|k| {
                        let m = k.modifiers();
                        (m.platform, m.control, k.key().to_string())
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    });
    assert!(!bound.is_empty() || !cfg!(target_os = "macos"));
    for key in ["a", "c", "x", "v"] {
        let primary_key = (
            cfg!(target_os = "macos"),
            !cfg!(target_os = "macos"),
            key.to_string(),
        );
        assert!(!bound.contains(&primary_key), "{key} is bound: {bound:?}");
    }
    open_palette(vcx);
    vcx.simulate_input("abc");
    vcx.simulate_keystrokes(&format!("{} {}", primary("a"), primary("c")));
    assert_eq!(
        vcx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some("abc")
    );
    vcx.simulate_keystrokes(&format!("right {}", primary("v")));
    assert_eq!(palette_query(vcx), "abcabc");
    vcx.write_to_clipboard(ClipboardItem::new_string("z".into()));
    vcx.simulate_keystrokes(&format!("{} {}", primary("a"), primary("x")));
    assert_eq!(palette_query(vcx), "");
}

/// The menus' key equivalents are GPUI bindings under `HacivatMenu`, a context no element has,
/// so they only label the native menu items (R7).
#[cfg(target_os = "macos")]
#[gpui::test]
fn menu_chords_are_bound_under_the_menu_context(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let found = vcx.read(|cx| {
        cx.key_bindings()
            .borrow()
            .bindings()
            .filter(|b| {
                b.keystrokes().len() == 1
                    && b.keystrokes()[0].modifiers().platform
                    && b.keystrokes()[0].key() == "t"
            })
            .map(|b| {
                (
                    b.action().partial_eq(&RunCommand {
                        id: "workspace.new_console_tab".into(),
                    }),
                    b.predicate().map(|p| p.to_string()),
                )
            })
            .collect::<Vec<_>>()
    });
    assert_eq!(found, [(true, Some(MENU_KEY_CONTEXT.to_string()))]);
}

/// The popup menu's bounds and the window's viewport size, after a frame.
fn popup_bounds(
    vcx: &mut VisualTestContext,
) -> (gpui::Bounds<gpui::Pixels>, gpui::Size<gpui::Pixels>) {
    vcx.run_until_parked();
    let bounds = vcx
        .debug_bounds("popup-menu")
        .expect("the popup menu is drawn");
    let viewport = in_window(vcx, |window, _| window.viewport_size());
    (bounds, viewport)
}

fn assert_inside(bounds: gpui::Bounds<gpui::Pixels>, viewport: gpui::Size<gpui::Pixels>) {
    assert!(
        bounds.origin.x >= px(0.0) && bounds.origin.y >= px(0.0),
        "{bounds:?}"
    );
    assert!(
        bounds.right() <= viewport.width,
        "{bounds:?} in {viewport:?}"
    );
    assert!(
        bounds.bottom() <= viewport.height,
        "{bounds:?} in {viewport:?}"
    );
}

/// Review B3, end to end where the in-window menu exists: `app.show_menu` opens layer `menu`
/// under the title bar, inside the window.
#[cfg(not(target_os = "macos"))]
#[gpui::test]
fn show_menu_opens_the_in_window_menu_inside_the_window(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    assert!(run(vcx, "app.show_menu"));
    assert_eq!(layer_ids(vcx), ["menu"]);
    let (bounds, viewport) = popup_bounds(vcx);
    assert_inside(bounds, viewport);
    assert_eq!(bounds.origin.y, px(40.0));
}

/// Review B3 on every platform: the anchor `app.show_menu` uses keeps the whole menu (wider
/// than its 200 px minimum) inside the window, right-aligned under the title bar.
#[gpui::test]
fn the_in_window_menu_anchor_keeps_the_menu_inside_the_window(cx: &mut TestAppContext) {
    use hacivat_app::menus::in_window_menu_anchor;
    let vcx = shell(cx, "tr");
    with_ws(vcx, |w, window, cx| {
        let items = popup_items(&menu_model(cx.global::<ActionRegistry>()), cx);
        let anchor = in_window_menu_anchor(&items, window, cx);
        w.open_popup_menu(anchor, items, window, cx);
    });
    assert_eq!(layer_ids(vcx), ["menu"]);
    let (bounds, viewport) = popup_bounds(vcx);
    assert_inside(bounds, viewport);
    assert!(bounds.size.width > px(200.0), "{bounds:?}");
    assert!(
        viewport.width - bounds.right() <= px(40.0),
        "{bounds:?} in {viewport:?}"
    );
}

/// Review B2: `menus::rebuild` owns GPUI's key bindings; after it, every binding is a menu key
/// equivalent under `HacivatMenu`, so none can fire on an element.
#[gpui::test]
fn every_gpui_binding_after_rebuild_is_a_menu_equivalent(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    vcx.update(|_, cx| {
        cx.bind_keys([gpui::KeyBinding::new(
            "cmd-y",
            RunCommand {
                id: "app.about".into(),
            },
            None,
        )]);
        hacivat_app::menus::rebuild(cx);
    });
    let contexts: Vec<Option<String>> = vcx.read(|cx| {
        cx.key_bindings()
            .borrow()
            .bindings()
            .map(|b| b.predicate().map(|p| p.to_string()))
            .collect()
    });
    if cfg!(target_os = "macos") {
        assert!(!contexts.is_empty());
    }
    for context in &contexts {
        assert_eq!(context.as_deref(), Some(MENU_KEY_CONTEXT), "{contexts:?}");
    }
}

#[gpui::test]
fn run_command_from_a_menu_runs_the_action(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let window = vcx.window_handle();
    vcx.cx.dispatch_action(
        window,
        RunCommand {
            id: "workspace.new_console_tab".into(),
        },
    );
    vcx.run_until_parked();
    assert_eq!(tab_count(vcx), 2);
}

/// `app.restart` asks the platform to restart.
#[gpui::test]
fn restart_asks_the_platform_to_restart(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let mut restarted = vcx.cx.expect_restart();
    assert!(run(vcx, "app.restart"));
    assert!(matches!(restarted.try_recv(), Ok(Some(_))));
}

/// Ruling: a settings change and a layout change still inside their debounce are on disk by the
/// time `app.restart` asks the platform to restart (nothing is parked in between, so neither
/// background writer has run).
#[gpui::test]
fn restart_flushes_pending_writes_before_restarting(cx: &mut TestAppContext) {
    use hacivat_app::shell::update_settings;
    use hacivat_settings::ThemePref;
    let root = tempfile::tempdir().unwrap();
    let vcx = shell_at(cx, root.path(), "en");
    let mut restarted = vcx.cx.expect_restart();
    let settings = root.path().join("config/settings.toml");
    let layout_file = root.path().join("data/workspace.json");
    assert!(!layout_file.exists());
    let ws = workspace(vcx);
    let written_at_restart = in_window(vcx, |window, cx| {
        update_settings(cx, |s| s.ui.theme = ThemePref::Light);
        ws.update(cx, |w, cx| w.toggle_explorer(window, cx));
        assert!(ActionRegistry::run("app.restart", window, cx));
        (
            std::fs::read_to_string(&settings).unwrap_or_default(),
            std::fs::read_to_string(&layout_file).unwrap_or_default(),
        )
    });
    // The restart is deferred to the end of that update; read the files once it has run.
    assert!(matches!(restarted.try_recv(), Ok(Some(_))));
    assert_eq!(written_at_restart, (String::new(), String::new()));
    let saved_settings = std::fs::read_to_string(&settings).unwrap();
    assert!(
        saved_settings.contains("theme = \"light\""),
        "{saved_settings}"
    );
    let saved_layout = std::fs::read_to_string(&layout_file).unwrap();
    assert!(
        saved_layout.contains("\"explorer_visible\": false"),
        "{saved_layout}"
    );
}

/// T13 fix: a caller that subscribes to the dialog `open_dialog` returns (and keeps no handle)
/// still receives the pressed button.
#[gpui::test]
fn a_dialog_caller_receives_the_pressed_button(cx: &mut TestAppContext) {
    use hacivat_app::l10n_global::text_fn_id;
    use hacivat_ui::button::ButtonKind;
    use hacivat_ui::dialog::{DialogButton, DialogEvent, DialogSpec};
    let vcx = shell(cx, "en");
    let events: Rc<RefCell<Vec<DialogEvent>>> = Rc::default();
    let sink = events.clone();
    with_ws(vcx, |w, window, cx| {
        let button = |id| DialogButton {
            label: text_fn_id(id),
            kind: ButtonKind::Default,
            accesskey: None,
            english_accesskey: None,
        };
        let spec = DialogSpec {
            title: text_fn_id("hv-about-title"),
            body: text_fn_id("hv-menu-view"),
            buttons: vec![button("hui-dialog-close"), button("hv-menu-help")],
            cancel: 0,
            primary: 1,
        };
        let dialog = w.open_dialog(spec, window, cx);
        cx.subscribe(&dialog, move |_, _, e: &DialogEvent, _| {
            sink.borrow_mut().push(*e)
        })
        .detach();
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("enter");
    assert_eq!(events.take(), [DialogEvent::Pressed(1)]);
    assert!(layer_ids(vcx).is_empty());
}

/// T13 fix, menu side: the caller's subscription gets the chosen item.
#[gpui::test]
fn a_popup_menu_caller_receives_the_chosen_item(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    let events: Rc<RefCell<Vec<PopupMenuEvent>>> = Rc::default();
    let sink = events.clone();
    with_ws(vcx, |w, window, cx| {
        let items = vec![MenuItemSpec::new(
            Rc::new(|_: &App| "Item".into()),
            "test.item",
        )];
        let menu = w.open_popup_menu(point(px(10.0), px(50.0)), items, window, cx);
        cx.subscribe(&menu, move |_, _, e: &PopupMenuEvent, _| {
            sink.borrow_mut().push(e.clone())
        })
        .detach();
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("enter");
    assert_eq!(events.take(), [PopupMenuEvent::Chosen("test.item".into())]);
    assert!(layer_ids(vcx).is_empty());
}

#[gpui::test]
fn plain_escape_with_a_layer_open_reaches_the_layer(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    open_palette(vcx);
    let ran = record_actions(vcx);
    vcx.simulate_keystrokes("escape");
    assert!(layer_ids(vcx).is_empty());
    assert!(ran.take().is_empty());
}

#[gpui::test]
fn confirming_a_palette_row_runs_it_and_closes_the_palette(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    open_palette(vcx);
    vcx.simulate_input("new console");
    assert_eq!(palette_items(vcx)[0].id, "workspace.new_console_tab");
    vcx.simulate_keystrokes("enter");
    assert!(layer_ids(vcx).is_empty());
    assert_eq!(tab_count(vcx), 2);
    open_palette(vcx);
    assert_eq!(palette_items(vcx)[0].id, "workspace.new_console_tab"); // recent first
}

#[test]
fn every_title_id_follows_the_action_id() {
    for d in registry().all() {
        let want = format!("hv-cmd-{}", d.id.replace(['.', '_'], "-"));
        assert_eq!(d.title_id, want);
    }
}

#[test]
fn show_menu_is_the_only_platform_gated_action() {
    let ids: Vec<&str> = registry().all().iter().map(|d| d.id).collect();
    assert_eq!(
        ids,
        [
            "workspace.command_palette",
            "workspace.open_settings",
            "workspace.new_console_tab",
            "workspace.close_tab",
            "workspace.next_tab",
            "workspace.prev_tab",
            "workspace.split_right",
            "workspace.split_down",
            "workspace.focus_next_pane",
            "workspace.toggle_explorer",
            "workspace.toggle_theme",
            "workspace.next_language",
            "workspace.reload_translations",
            "app.about",
            "app.show_menu",
            "app.restart",
            "app.quit",
            "explorer.focus_filter",
        ]
    );
}

// ---- the dispatcher alone ----------------------------------------------------------------------

fn press(chord: &'static str) -> KeyDownEvent {
    KeyDownEvent {
        keystroke: Keystroke::parse(chord).expect("a keystroke"),
        is_held: false,
        prefer_character_input: false,
    }
}

fn dispatcher(user: &str) -> Dispatcher {
    let (km, diags) = Keymap::from_sources(KeymapPlatform::Linux, "", "", &[], Some(user));
    assert!(diags.is_empty(), "{diags:?}");
    Dispatcher::new(Arc::new(km), KeyboardLayoutId("us".to_string()))
}

use std::sync::Arc;
use std::time::Instant;

const TWO_KEYS: &str = "[[binding]]\nchord=\"ctrl-j ctrl-s\"\ncommand=\"t.two\"\ncontext=\"Workspace\"\n[[binding]]\nchord=\"ctrl-k\"\ncommand=\"t.one\"\ncontext=\"Workspace\"";

#[test]
fn a_two_key_chord_waits_then_runs() {
    let mut d = dispatcher(TWO_KEYS);
    let ctx = ContextStack(vec!["Workspace"]);
    let t0 = Instant::now();
    assert_eq!(
        d.on_key_down(&press("ctrl-j"), &ctx, false, t0),
        DispatchOutcome::Pending("Ctrl+J".into())
    );
    assert_eq!(
        d.on_key_down(
            &press("ctrl-s"),
            &ctx,
            false,
            t0 + Duration::from_millis(10)
        ),
        DispatchOutcome::Run("t.two".to_string())
    );
    assert!(d.pending.is_empty());
    assert_eq!(
        d.on_key_down(&press("ctrl-k"), &ctx, false, t0),
        DispatchOutcome::Run("t.one".to_string())
    );
}

#[test]
fn a_pending_chord_expires_after_one_second() {
    let mut d = dispatcher(TWO_KEYS);
    let ctx = ContextStack(vec!["Workspace"]);
    let t0 = Instant::now();
    d.on_key_down(&press("ctrl-j"), &ctx, false, t0);
    assert_eq!(
        d.on_key_down(&press("ctrl-s"), &ctx, false, t0 + PENDING_TIMEOUT),
        DispatchOutcome::PassThrough
    );
    assert!(d.pending.is_empty());
}

#[test]
fn set_keymap_clears_the_pending_chord() {
    let mut d = dispatcher(TWO_KEYS);
    let ctx = ContextStack(vec!["Workspace"]);
    d.on_key_down(&press("ctrl-j"), &ctx, false, Instant::now());
    assert_eq!(d.pending.len(), 1);
    let keymap = d.keymap.clone();
    d.set_keymap(keymap, KeyboardLayoutId("tr-q".to_string()));
    assert!(d.pending.is_empty());
    assert_eq!(d.layout, KeyboardLayoutId("tr-q".to_string()));
}

#[test]
fn chords_in_other_contexts_and_during_composition_pass_through() {
    let mut d = dispatcher(TWO_KEYS);
    assert_eq!(
        d.on_key_down(
            &press("ctrl-k"),
            &ContextStack(vec!["Other"]),
            false,
            Instant::now()
        ),
        DispatchOutcome::PassThrough
    );
    assert_eq!(
        d.on_key_down(
            &press("ctrl-k"),
            &ContextStack(vec!["Workspace"]),
            true,
            Instant::now()
        ),
        DispatchOutcome::PassThrough
    );
}
