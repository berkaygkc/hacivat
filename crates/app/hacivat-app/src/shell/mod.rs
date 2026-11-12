//! The workspace shell: the window, its tiles and tabs, and the extension points later tasks
//! plug into (F3): [`Layers`], the [`TabContentRegistry`], the key-capture hook in
//! [`ShellHooks`] and the [`ActiveKeymap`].

pub mod hooks;
pub mod keymap_state;
pub mod layers;
pub mod tab_content;
pub mod tabs;
pub mod tiles;
pub mod workspace;

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use hacivat_l10n::{Catalogs, L10n, NoArgs, OsLocale, Text, ids};
use hacivat_settings::{LoadOutcome, LocalPaths, Settings, SettingsDiag};
use hacivat_ui::actions::ActionRegistry;
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{App, AppContext as _, Global, WindowHandle};

pub use hooks::{KeyCaptureHook, ShellHooks, set_key_capture};
pub use keymap_state::{ActiveKeymap, install_with as install_keymap_with, set_keymap_preset};
pub use layers::{Layer, Layers};
pub use tab_content::{TabContent, TabContentRegistry, TabKind};
pub use tiles::TileLayout;
pub use workspace::{DemoCapsule, Workspace, open_workspace};

use crate::l10n_global::{HostLocale, build_initial, install_l10n, tr};
use crate::theme_pref::apply_theme;

/// Where the app keeps its files and the settings in effect.
#[derive(Debug, Clone)]
pub struct AppState {
    pub paths: LocalPaths,
    pub settings: Settings,
}

impl Global for AppState {}

/// Everything prepared before GPUI starts: paths, loaded settings (with their diagnostics),
/// the OS locale, the catalogs (embedded plus the overlay in `<config>/locales`) and the first
/// localization runtime, built synchronously.
pub struct Startup {
    pub paths: LocalPaths,
    pub settings: Settings,
    pub diags: Vec<SettingsDiag>,
    pub quarantined: Option<PathBuf>,
    pub os_locale: OsLocale,
    pub catalogs: Arc<Catalogs>,
    pub l10n: Arc<L10n>,
}

impl Startup {
    pub fn new(paths: LocalPaths, outcome: LoadOutcome, os_locale: OsLocale) -> Startup {
        let catalogs = Catalogs::embedded().with_overlay(&paths.locales_overlay_dir());
        let l10n = build_initial(&catalogs, &outcome.settings, &os_locale);
        Startup {
            paths,
            settings: outcome.settings,
            diags: outcome.diags,
            quarantined: outcome.quarantined,
            os_locale,
            catalogs: Arc::new(catalogs),
            l10n: Arc::new(l10n),
        }
    }
}

/// Set once `crate::init` has run in this app.
struct Initialized;

impl Global for Initialized {}

/// Starts the app on `cx`: installs the settings, localization, theme and keymap, runs
/// [`crate::init`] (once per app), opens the workspace window and shows the startup
/// diagnostics as toasts (F24).
pub fn start(
    cx: &mut App,
    startup: Startup,
    demo: Option<DemoCapsule>,
) -> anyhow::Result<WindowHandle<Workspace>> {
    let theme = startup.settings.ui.theme;
    cx.set_global(AppState {
        paths: startup.paths,
        settings: startup.settings,
    });
    cx.set_global(HostLocale(startup.os_locale));
    install_l10n(cx, startup.l10n, startup.catalogs);
    if !cx.has_global::<Initialized>() {
        cx.set_global(Initialized);
        crate::init(cx);
    }
    apply_theme(cx, theme);
    keymap_state::install(cx);
    let window = open_workspace(cx, demo)?;
    show_startup_diags(cx, &startup.diags, startup.quarantined.as_deref());
    Ok(window)
}

/// Installs the shell's globals when missing (a later call keeps what is there).
pub(crate) fn install_globals(cx: &mut App) {
    cx.default_global::<TabContentRegistry>();
    cx.default_global::<ShellHooks>();
    cx.default_global::<ActionRegistry>();
    cx.default_global::<ToastQueue>();
}

/// Applies `change` to the settings in effect and saves the changed keys to `settings.toml` on
/// the background executor. Returns the new settings (the defaults when no `AppState` exists).
pub fn update_settings(cx: &mut App, change: impl FnOnce(&mut Settings)) -> Settings {
    let Some(state) = cx.try_global::<AppState>() else {
        let mut settings = Settings::default();
        change(&mut settings);
        return settings;
    };
    let mut settings = state.settings.clone();
    let path = state.paths.settings_toml();
    change(&mut settings);
    cx.global_mut::<AppState>().settings = settings.clone();
    let saved = settings.clone();
    cx.background_spawn(async move {
        // A failed save keeps the file as it was; the setting stays applied for this session.
        let _ = hacivat_settings::save_changes(&path, &saved);
    })
    .detach();
    settings
}

/// One toast per startup problem (F24): the quarantined settings file (error), the keys whose
/// values were invalid (one warning listing them) and a settings file from a newer version.
fn show_startup_diags(cx: &mut App, diags: &[SettingsDiag], quarantined: Option<&std::path::Path>) {
    if let Some(path) = quarantined {
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let text = Rc::new(move |cx: &App| {
            tr(
                cx,
                ids::hv::SETTINGS_FILE_QUARANTINED,
                ids::hv::HvSettingsFileQuarantined {
                    file: Text(file.clone()),
                },
            )
        });
        ToastQueue::push(cx, Toast::new(ToastKind::Error, text));
    }
    let keys: Vec<String> = diags
        .iter()
        .filter(|d| d.code == "settings-invalid-value")
        .map(|d| d.key.clone())
        .collect();
    if !keys.is_empty() {
        let text = Rc::new(move |cx: &App| {
            tr(
                cx,
                ids::hv::SETTINGS_INVALID_VALUE,
                ids::hv::HvSettingsInvalidValue { keys: keys.clone() },
            )
        });
        ToastQueue::push(cx, Toast::new(ToastKind::Error, text));
    }
    if diags.iter().any(|d| d.code == "settings-newer-version") {
        let text = Rc::new(|cx: &App| tr(cx, ids::hv::SETTINGS_NEWER_VERSION, NoArgs));
        ToastQueue::push(cx, Toast::new(ToastKind::Info, text));
    }
}
