//! The workspace shell: the window, its tiles and tabs, and the extension points later tasks
//! plug into (F3): [`Layers`], the [`TabContentRegistry`], the key-capture hook in
//! [`ShellHooks`] and the [`ActiveKeymap`].

pub mod hooks;
pub mod keymap_state;
pub mod layers;
pub mod persist;
pub mod tab_content;
pub mod tabs;
pub mod tiles;
pub mod workspace;

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use hacivat_l10n::{Catalogs, L10n, NoArgs, OsLocale, Text, ids};
use hacivat_settings::{LanguagePref, LoadOutcome, LocalPaths, Settings, SettingsDiag, ThemePref};
use hacivat_ui::actions::ActionRegistry;
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{App, Global, WindowHandle};

pub use hooks::{KeyCaptureHook, ShellHooks, set_key_capture};
pub use keymap_state::{ActiveKeymap, install_with as install_keymap_with, set_keymap_preset};
pub use layers::{Layer, Layers};
pub use tab_content::{TabContent, TabContentRegistry, TabKind};
pub use tiles::TileLayout;
pub use workspace::{DemoCapsule, Workspace, open_workspace};

use crate::l10n_global::{HostLocale, build_initial, install_l10n, tr};
use crate::theme_pref::apply_theme;
use persist::LatestWriter;

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
    /// Whether settings changes are saved to `settings.toml` (false under session overrides).
    pub persist_settings: bool,
}

/// Command-line overrides for one session (`--theme`, `--lang`): applied in memory only, and
/// while any is set no settings change is ever written.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionOverrides {
    pub theme: Option<ThemePref>,
    pub language: Option<String>,
}

impl SessionOverrides {
    pub fn is_empty(&self) -> bool {
        self.theme.is_none() && self.language.is_none()
    }
}

impl Startup {
    pub fn new(paths: LocalPaths, outcome: LoadOutcome, os_locale: OsLocale) -> Startup {
        Startup::with_overrides(paths, outcome, os_locale, &SessionOverrides::default())
    }

    /// Like [`Startup::new`], with `overrides` applied to the loaded settings in memory.
    pub fn with_overrides(
        paths: LocalPaths,
        mut outcome: LoadOutcome,
        os_locale: OsLocale,
        overrides: &SessionOverrides,
    ) -> Startup {
        if let Some(theme) = overrides.theme {
            outcome.settings.ui.theme = theme;
        }
        if let Some(tag) = &overrides.language {
            outcome.settings.ui.language = LanguagePref::Tag(tag.clone());
        }
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
            persist_settings: overrides.is_empty(),
        }
    }
}

/// Saves settings changes in order (only the latest state lands) and flushes on quit.
struct SettingsWriter {
    writer: LatestWriter<(PathBuf, Settings)>,
    enabled: bool,
}

impl Global for SettingsWriter {}

fn write_settings((path, settings): &(PathBuf, Settings)) {
    if let Err(err) = hacivat_settings::save_changes(path, settings) {
        // The file stays as it was; the change stays applied for this session.
        tracing::warn!(path = %path.display(), %err, "settings: saving failed");
    }
}

fn install_settings_writer(cx: &mut App, enabled: bool) {
    if let Some(w) = cx.try_global::<SettingsWriter>() {
        let writer = w.writer.clone();
        cx.set_global(SettingsWriter { writer, enabled });
        return;
    }
    cx.set_global(SettingsWriter {
        writer: LatestWriter::new(write_settings),
        enabled,
    });
    cx.on_app_quit(|cx| {
        if let Some(w) = cx.try_global::<SettingsWriter>() {
            w.writer.flush();
        }
        async {}
    })
    .detach();
}

/// Writes the pending settings and every workspace's pending layout now, on the calling thread.
/// `on_app_quit` does this at quit, but `App::restart` quits from inside an update, where GPUI
/// cannot run its quit handlers, so a restart calls this first.
pub fn flush_pending_writes(cx: &mut App) {
    if let Some(w) = cx.try_global::<SettingsWriter>() {
        w.writer.flush();
    }
    for window in cx.windows() {
        if let Some(workspace) = window.downcast::<Workspace>() {
            let _ = workspace.update(cx, |ws, _, _| ws.flush_layout());
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
    install_settings_writer(cx, startup.persist_settings);
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
/// the background executor, in order: of quick successive changes the last one lands. Nothing is
/// written under session overrides. Returns the new settings (the defaults without `AppState`).
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
    if let Some(w) = cx.try_global::<SettingsWriter>().filter(|w| w.enabled) {
        w.writer.submit((path, settings.clone()), None, cx).detach();
    }
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
