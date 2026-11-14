//! The workspace and app commands (F14): every one is reachable from the palette and the menus
//! and can be bound to a key. Titles are `hv-cmd-<action id with . and _ as ->` (R5).

use std::rc::Rc;
use std::sync::Arc;

use hacivat_l10n::{Catalogs, Status, Text, ids};
use hacivat_settings::{LanguagePref, ThemePref};
use hacivat_ui::actions::{ActionDef, ActionFn, ActionRegistry};
use hacivat_ui::button::ButtonKind;
use hacivat_ui::dialog::{DialogButton, DialogSize, DialogSpec};
use hacivat_ui::panes::Axis;
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{App, AppContext as _, Context, Window};
use hacivat_ui_core::theme::{Theme, ThemeMode};

use crate::commands::palette;
use crate::l10n_global::{L10nGlobal, switch_locale, text_fn, text_fn_id, tr, ui_language};
use crate::menus;
use crate::shell::{AppState, Workspace, flush_pending_writes, update_settings};
// ## m3a-t01b-seams
use crate::shell::CloseDecision;
use crate::theme_pref::apply_theme;

/// The key context every command here belongs to.
const WORKSPACE: &str = "Workspace";

// ## m3a-t01b-seams
/// `app.quit`: asks the quit guard (`ShellHooks::quit_guard`) first and quits only on
/// `Proceed`; on `Defer` the guard calls `cx.quit()` itself after its prompt.
pub fn request_quit(cx: &mut App) -> CloseDecision {
    let decision = crate::shell::hooks::quit_decision(cx);
    if decision == CloseDecision::Proceed {
        #[cfg(feature = "test-support")]
        {
            cx.default_global::<QuitRequests>().0 += 1;
        }
        cx.quit();
    }
    decision
}

/// How often the app asked the platform to quit (test seam, feature `test-support`: the test
/// platform's quit does nothing observable).
#[cfg(feature = "test-support")]
#[derive(Debug, Clone, Copy, Default)]
pub struct QuitRequests(pub u32);

#[cfg(feature = "test-support")]
impl hacivat_ui_core::gpui_surface::Global for QuitRequests {}

/// How often the app asked the platform to quit (test seam, feature `test-support`).
#[cfg(feature = "test-support")]
pub fn quit_requests(cx: &App) -> u32 {
    cx.try_global::<QuitRequests>().map_or(0, |q| q.0)
}

/// The tab kind Task 17 registers for the settings.
const SETTINGS_TAB: &str = "settings";

/// Runs `f` on the window's workspace (nothing when the root view is not one).
fn on_workspace(
    f: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
) -> ActionFn {
    Rc::new(move |window, cx| {
        if let Some(ws) = window.root::<Workspace>().flatten() {
            ws.update(cx, |ws, cx| f(ws, window, cx));
        }
    })
}

fn always(_: &App) -> bool {
    true
}

/// `app.show_menu` exists where the title bar draws the app-menu button (F14); macOS has the
/// menu bar.
fn has_in_window_menu(_: &App) -> bool {
    !cfg!(target_os = "macos")
}

/// One command before registration: ID, title message, body, enabled check.
type Def = (&'static str, &'static str, ActionFn, fn(&App) -> bool);

/// Registers exactly the workspace and app commands, in palette order. An ID already registered
/// keeps its first definition.
pub fn register_workspace_actions(reg: &mut ActionRegistry) {
    let defs: Vec<Def> = vec![
        (
            "workspace.command_palette",
            ids::hv::CMD_WORKSPACE_COMMAND_PALETTE.id,
            Rc::new(palette::open_palette),
            always,
        ),
        (
            "workspace.open_settings",
            ids::hv::CMD_WORKSPACE_OPEN_SETTINGS.id,
            on_workspace(|ws, window, cx| ws.open_registered_tab(SETTINGS_TAB, window, cx)),
            always,
        ),
        (
            "workspace.new_console_tab",
            ids::hv::CMD_WORKSPACE_NEW_CONSOLE_TAB.id,
            on_workspace(Workspace::new_console_tab),
            always,
        ),
        (
            "workspace.close_tab",
            ids::hv::CMD_WORKSPACE_CLOSE_TAB.id,
            // ## m3a-t01b-seams: through the tab close guard (the open-transaction prompt).
            Rc::new(|window, cx| {
                if let Some(ws) = window.root::<Workspace>().flatten() {
                    Workspace::request_close_active_tab(&ws, window, cx);
                }
            }),
            always,
        ),
        (
            "workspace.next_tab",
            ids::hv::CMD_WORKSPACE_NEXT_TAB.id,
            on_workspace(Workspace::activate_next_tab),
            always,
        ),
        (
            "workspace.prev_tab",
            ids::hv::CMD_WORKSPACE_PREV_TAB.id,
            on_workspace(Workspace::activate_prev_tab),
            always,
        ),
        (
            "workspace.split_right",
            ids::hv::CMD_WORKSPACE_SPLIT_RIGHT.id,
            on_workspace(|ws, window, cx| ws.split_focused(Axis::Horizontal, window, cx)),
            always,
        ),
        (
            "workspace.split_down",
            ids::hv::CMD_WORKSPACE_SPLIT_DOWN.id,
            on_workspace(|ws, window, cx| ws.split_focused(Axis::Vertical, window, cx)),
            always,
        ),
        (
            "workspace.focus_next_pane",
            ids::hv::CMD_WORKSPACE_FOCUS_NEXT_PANE.id,
            on_workspace(Workspace::focus_next_pane),
            always,
        ),
        (
            "workspace.toggle_explorer",
            ids::hv::CMD_WORKSPACE_TOGGLE_EXPLORER.id,
            on_workspace(Workspace::toggle_explorer),
            always,
        ),
        (
            "workspace.toggle_theme",
            ids::hv::CMD_WORKSPACE_TOGGLE_THEME.id,
            Rc::new(|_, cx| toggle_theme(cx)),
            always,
        ),
        (
            "workspace.next_language",
            ids::hv::CMD_WORKSPACE_NEXT_LANGUAGE.id,
            Rc::new(|_, cx| next_language(cx)),
            always,
        ),
        (
            "workspace.reload_translations",
            ids::hv::CMD_WORKSPACE_RELOAD_TRANSLATIONS.id,
            Rc::new(|_, cx| reload_translations(cx)),
            always,
        ),
        (
            "app.about",
            ids::hv::CMD_APP_ABOUT.id,
            on_workspace(open_about),
            always,
        ),
        (
            "app.show_menu",
            ids::hv::CMD_APP_SHOW_MENU.id,
            Rc::new(show_menu),
            has_in_window_menu,
        ),
        (
            "app.restart",
            ids::hv::CMD_APP_RESTART.id,
            // Deferred, so nothing is being updated when it runs. The pending settings and
            // layout writes land first: a restart inside an update runs no quit hooks.
            Rc::new(|_, cx| {
                cx.defer(|cx| {
                    flush_pending_writes(cx);
                    cx.restart();
                })
            }),
            always,
        ),
        (
            "app.quit",
            ids::hv::CMD_APP_QUIT.id,
            // ## m3a-t01b-seams: through the quit guard (the open-transaction prompt).
            Rc::new(|_, cx| {
                request_quit(cx);
            }),
            always,
        ),
        (
            "explorer.focus_filter",
            ids::hv::CMD_EXPLORER_FOCUS_FILTER.id,
            on_workspace(focus_explorer_filter),
            always,
        ),
    ];
    for (id, title_id, run, enabled) in defs {
        // A duplicate keeps the first definition; registration order is the palette's order.
        let _ = reg.register(ActionDef {
            id,
            title_id,
            context: WORKSPACE,
            run,
            enabled,
        });
    }
}

/// Switches between the dark and the light theme and saves the choice.
fn toggle_theme(cx: &mut App) {
    let pref = match Theme::get(cx).mode {
        ThemeMode::Dark => ThemePref::Light,
        ThemeMode::Light => ThemePref::Dark,
    };
    update_settings(cx, |s| s.ui.theme = pref);
    apply_theme(cx, pref);
}

/// Switches to the next stable or beta catalog language (wrapping) and saves it.
fn next_language(cx: &mut App) {
    let Some(global) = cx.try_global::<L10nGlobal>() else {
        return;
    };
    let tags: Vec<(String, String)> = global
        .catalogs
        .locales()
        .iter()
        .filter(|l| l.status != Status::Experimental)
        .map(|l| (l.tag.to_string(), l.tag.language.as_str().to_string()))
        .collect();
    if tags.is_empty() {
        return;
    }
    let current = ui_language(cx).map(|l| l.language.as_str().to_string());
    let at = tags
        .iter()
        .position(|(_, lang)| Some(lang) == current.as_ref());
    let next = at.map_or(0, |i| (i + 1) % tags.len());
    let tag = tags[next].0.clone();
    let settings = update_settings(cx, |s| s.ui.language = LanguagePref::Tag(tag));
    switch_locale(cx, &settings).detach();
}

/// Reads the translation overlay again (on the background executor), installs the result for
/// the current language and then shows an info toast.
fn reload_translations(cx: &mut App) {
    let Some(global) = cx.try_global::<L10nGlobal>().cloned() else {
        return;
    };
    let overlay = cx
        .try_global::<AppState>()
        .map(|s| s.paths.locales_overlay_dir());
    let build = cx.background_spawn(async move {
        let embedded = Catalogs::embedded();
        match overlay {
            Some(dir) => embedded.with_overlay(&dir),
            None => embedded,
        }
    });
    cx.spawn(async move |cx| {
        let catalogs = build.await;
        let switched = cx.update(|cx| {
            cx.set_global(L10nGlobal {
                l10n: global.l10n.clone(),
                catalogs: Arc::new(catalogs),
            });
            let settings = cx
                .try_global::<AppState>()
                .map(|s| s.settings.clone())
                .unwrap_or_default();
            switch_locale(cx, &settings)
        });
        switched.await;
        cx.update(|cx| {
            ToastQueue::push(
                cx,
                Toast::new(
                    ToastKind::Info,
                    text_fn(ids::hv::TOAST_TRANSLATIONS_RELOADED),
                ),
            );
        });
    })
    .detach();
}

/// The About dialog: the product and its version, with one Close button (F14).
fn open_about(ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let version = env!("CARGO_PKG_VERSION");
    let spec = DialogSpec {
        title: text_fn(ids::hv::ABOUT_TITLE),
        body: Rc::new(move |cx: &App| {
            tr(
                cx,
                ids::hv::ABOUT_VERSION,
                ids::hv::HvAboutVersion {
                    version: Text(version.to_string()),
                },
            )
        }),
        buttons: vec![DialogButton {
            label: text_fn_id("hui-dialog-close"),
            kind: ButtonKind::Default,
            accesskey: None,
            english_accesskey: None,
        }],
        cancel: 0,
        primary: 0,
        size: DialogSize::Standard,
    };
    ws.open_dialog(spec, window, cx);
}

/// The Windows/Linux in-window menu: the same model as the macOS menu bar, under the title
/// bar's app-menu button at the right end of the title bar.
fn show_menu(window: &mut Window, cx: &mut App) {
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    let items = match cx.try_global::<ActionRegistry>() {
        Some(reg) => menus::popup_items(&menus::menu_model(reg), cx),
        None => return,
    };
    let anchor = menus::in_window_menu_anchor(&items, window, cx);
    let menu = ws.update(cx, |ws, cx| ws.open_popup_menu(anchor, items, window, cx));
    menu.update(cx, |m, _| m.label(text_fn_id("hui-menu-app")));
}

/// Shows the explorer when it is hidden, so its connection filter is on screen, and focuses
/// the filter when the connections explorer is there (M2a, Task 18; the M1 filter is a static
/// field with nothing to focus).
fn focus_explorer_filter(ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if !ws.layout().1.explorer_visible {
        ws.toggle_explorer(window, cx);
    }
    // ## m2a-t18-explorer: after this update, so the explorer view is free to read.
    window.defer(cx, crate::connections::explorer::focus_filter);
}
