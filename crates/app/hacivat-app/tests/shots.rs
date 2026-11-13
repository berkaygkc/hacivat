//! The screenshot harness (Task 18, feature `shots`): renders every M1 surface headlessly (R8:
//! GPUI's `HeadlessAppContext` with the platform's real text system and its headless renderer)
//! and writes `target/shots/<surface>-<theme>-<locale>-<w>x<h>@<s>x.png`.
//!
//! ```text
//! cargo test -p hacivat-app --features shots --test shots -- --ignored
//! ```
//!
//! The target has `harness = false`: on macOS the platform (and its text system) can only be
//! created on the main thread, and libtest runs every test on a spawned thread. `main` keeps
//! libtest's contract: the self-checks run unless `--ignored`, and `render_all_surfaces` (the
//! main check) runs only with `--ignored` or `--include-ignored`. A positional filter that
//! names no test selects shots by file name (substring, or whole name with `--exact`):
//! `-- --ignored palette-open` renders only those and keeps the other PNGs. `SHOTS_DIR`
//! overrides the output directory.
//!
//! Every surface must show in its frame: a label drawn by its own source file (palette, dialog,
//! toast, menu, settings page; the title bar for the shells, the capsule for the demo).
//!
//! The M2a surfaces (Task 21) render the connection flow through `ConnectionsBoot::for_tests`
//! with a `FakeDriver` registry and a memory secret store, from the fixtures Tasks 9, 17, 18 and
//! 20 own (R28, P19): the connection dialog (also at 1024x700), the test strip, the error
//! dialog, the production prompt, the explorer and its menu, the delete and import dialogs,
//! the live capsule states, the explanation popover and the sticky toast; plus the shell at its
//! 960x600 minimum with the explorer populated.
//!
//! Every frame is checked before it is written:
//! - (a) in the pseudo locales (`en-XA`, `tr-XT`) every `TextSource::Fluent` label is bracketed;
//! - (b) no label is clipped: wider than its box, or cut at its container's clip (a control
//!   row running past its card, a tile cutting its last row), wrapping paragraphs included;
//!   text partly scrolled out of a marked scroll viewport is not clipped (`text.rs`,
//!   `cut_by_clip`, has the policy and its limits);
//! - (c) every label is at least 11 px nominal (synthesized small caps record their nominal
//!   size; their 0.7x glyph runs are the one exception);
//! - (d) the One Filled Badge rule holds inside the title-bar capsule, expecting production
//!   only where the capsule shows it, and no badge-sized `cp_fill` quad sits anywhere else (the
//!   7 px production marks of dialogs and the explorer are not badges; T21 ruling);
//! - (e) (Task 22 fix round 2) the title bar's layout, read from `TitleBarProbe`: the trailing
//!   controls (EN/TR, theme, window controls) lie wholly inside the window and end at the
//!   bar's right padding, the palette field is at least `PALETTE_MIN_PX` wide and ends before
//!   them, and the capsule ends before the palette field. A pushed-off icon button draws no
//!   clipped text, so (b) alone cannot see it.
//!
//! Windows has no headless renderer at the pin: the test prints `shots: no headless renderer
//! (R8)` and passes.
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hacivat_app::connections::{
    ConnectionsBoot, capsule, delete, dialog, error_view, explorer, import, prod_prompt, test_strip,
};
use hacivat_app::l10n_global::text_fn_id;
use hacivat_app::menus;
use hacivat_app::shell::DemoCapsule;
use hacivat_app::shell::Workspace;
use hacivat_app::{Startup, start};
use hacivat_driver::DriverRegistry;
use hacivat_driver::fake::FakeDriver;
use hacivat_l10n::OsLocale;
use hacivat_proto::DbmsKind;
use hacivat_settings::{LanguagePref, LocalPaths, ThemePref};
use hacivat_ui::actions::ActionRegistry;
use hacivat_ui::capsule::CapsuleProbe;
use hacivat_ui::text::{RenderedText, TextRegistry, TextSource};
use hacivat_ui::titlebar::{PALETTE_MIN_PX, TitleBarProbe};
use hacivat_ui_core::app::{TextRendering, bootstrap};
use hacivat_ui_core::assets::BundledAssets;
use hacivat_ui_core::gpui_surface::{
    AnyWindowHandle, App, Bounds, HeadlessAppContext, Pixels, Window, current_headless_renderer,
    current_platform, point, px, size,
};
use hacivat_ui_core::named_rules::assert_one_filled_badge_in;
use hacivat_ui_core::platform::apply_text_rendering;
use hacivat_ui_core::theme::{Theme, ThemeMode};

/// The surfaces, in render order: M1's, then M2a's (Task 21).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Surface {
    Shell,
    ShellDemoProduction,
    ShellNarrow,
    PaletteOpen,
    SettingsGeneral,
    DialogAbout,
    ToastInfo,
    MenuApp,
    // ## m2a-t21-shots
    ConnDialogNew,
    ConnDialogPasted,
    ConnDialogEngineSwitch,
    ConnDialogSqlite,
    ConnTestRunning,
    ConnTestOk,
    ConnTestFailed,
    ConnErrorAuth,
    ConnErrorUnknownDb,
    ConnErrorUnsupported,
    ConnErrorTls,
    ConnProdPrompt,
    ConnProdPromptUnset,
    ExplorerEmpty,
    ExplorerList,
    ExplorerMenu,
    ConnDeleteConfirm,
    ConnImportPgService,
    CapsuleConnecting,
    CapsuleConnectedUnencrypted,
    CapsuleProductionRo,
    CapsuleFailed,
    CapsuleUnsupported,
    UnencryptedPopover,
    ToastStickyDemo,
    ShellMinimum,
    // ## m2b-t22-live (fix round 1, M1): every T22 state, at 1280x800 and the 960x600 minimum.
    LiveReconnecting,
    LiveIdle,
    LiveLostTx,
    LiveLostReconnected,
    LiveGap,
    LiveRoEnforced,
    LiveRoGuarded,
    LiveRoPg13,
    LiveSshRo,
    LiveSshRw,
}

const SURFACES: [Surface; 44] = [
    Surface::Shell,
    Surface::ShellDemoProduction,
    Surface::ShellNarrow,
    Surface::PaletteOpen,
    Surface::SettingsGeneral,
    Surface::DialogAbout,
    Surface::ToastInfo,
    Surface::MenuApp,
    // ## m2a-t21-shots
    Surface::ConnDialogNew,
    Surface::ConnDialogPasted,
    Surface::ConnDialogEngineSwitch,
    Surface::ConnDialogSqlite,
    Surface::ConnTestRunning,
    Surface::ConnTestOk,
    Surface::ConnTestFailed,
    Surface::ConnErrorAuth,
    Surface::ConnErrorUnknownDb,
    Surface::ConnErrorUnsupported,
    Surface::ConnErrorTls,
    Surface::ConnProdPrompt,
    Surface::ConnProdPromptUnset,
    Surface::ExplorerEmpty,
    Surface::ExplorerList,
    Surface::ExplorerMenu,
    Surface::ConnDeleteConfirm,
    Surface::ConnImportPgService,
    Surface::CapsuleConnecting,
    Surface::CapsuleConnectedUnencrypted,
    Surface::CapsuleProductionRo,
    Surface::CapsuleFailed,
    Surface::CapsuleUnsupported,
    Surface::UnencryptedPopover,
    Surface::ToastStickyDemo,
    Surface::ShellMinimum,
    // ## m2b-t22-live
    Surface::LiveReconnecting,
    Surface::LiveIdle,
    Surface::LiveLostTx,
    Surface::LiveLostReconnected,
    Surface::LiveGap,
    Surface::LiveRoEnforced,
    Surface::LiveRoGuarded,
    Surface::LiveRoPg13,
    Surface::LiveSshRo,
    Surface::LiveSshRw,
];
const LOCALES: [&str; 4] = ["en", "tr", "en-XA", "tr-XT"];
const THEMES: [ThemePref; 2] = [ThemePref::Dark, ThemePref::Light];
const PSEUDO: [&str; 2] = ["en-XA", "tr-XT"];

impl Surface {
    fn name(self) -> &'static str {
        match self {
            Surface::Shell => "shell",
            Surface::ShellDemoProduction => "shell-demo-production",
            Surface::ShellNarrow => "shell-narrow",
            Surface::PaletteOpen => "palette-open",
            Surface::SettingsGeneral => "settings-general",
            Surface::DialogAbout => "dialog-about",
            Surface::ToastInfo => "toast-info",
            Surface::MenuApp => "menu-app",
            Surface::ConnDialogNew => "conn-dialog-new",
            Surface::ConnDialogPasted => "conn-dialog-pasted",
            Surface::ConnDialogEngineSwitch => "conn-dialog-engine-switch",
            Surface::ConnDialogSqlite => "conn-dialog-sqlite",
            Surface::ConnTestRunning => "conn-test-running",
            Surface::ConnTestOk => "conn-test-ok",
            Surface::ConnTestFailed => "conn-test-failed",
            Surface::ConnErrorAuth => "conn-error-auth",
            Surface::ConnErrorUnknownDb => "conn-error-unknown-db",
            Surface::ConnErrorUnsupported => "conn-error-unsupported",
            Surface::ConnErrorTls => "conn-error-tls",
            Surface::ConnProdPrompt => "conn-prod-prompt",
            Surface::ConnProdPromptUnset => "conn-prod-prompt-unset",
            Surface::ExplorerEmpty => "explorer-empty",
            Surface::ExplorerList => "explorer-list",
            Surface::ExplorerMenu => "explorer-menu",
            Surface::ConnDeleteConfirm => "conn-delete-confirm",
            Surface::ConnImportPgService => "conn-import-pg-service",
            Surface::CapsuleConnecting => "capsule-connecting",
            Surface::CapsuleConnectedUnencrypted => "capsule-connected-unencrypted",
            Surface::CapsuleProductionRo => "capsule-production-ro",
            Surface::CapsuleFailed => "capsule-failed",
            Surface::CapsuleUnsupported => "capsule-unsupported",
            Surface::UnencryptedPopover => "unencrypted-popover",
            Surface::ToastStickyDemo => "toast-sticky-demo",
            Surface::ShellMinimum => "shell-minimum",
            // ## m2b-t22-live
            Surface::LiveReconnecting => "live-reconnecting",
            Surface::LiveIdle => "live-idle",
            Surface::LiveLostTx => "live-lost-tx",
            Surface::LiveLostReconnected => "live-lost-reconnected",
            Surface::LiveGap => "live-gap",
            Surface::LiveRoEnforced => "live-ro-enforced",
            Surface::LiveRoGuarded => "live-ro-guarded",
            Surface::LiveRoPg13 => "live-ro-pg13",
            Surface::LiveSshRo => "live-ssh-ro",
            Surface::LiveSshRw => "live-ssh-rw",
        }
    }

    // ## m2b-t22-live
    /// Whether the surface is one of Task 22's live states.
    fn is_live(self) -> bool {
        matches!(
            self,
            Surface::LiveReconnecting
                | Surface::LiveIdle
                | Surface::LiveLostTx
                | Surface::LiveLostReconnected
                | Surface::LiveGap
                | Surface::LiveRoEnforced
                | Surface::LiveRoGuarded
                | Surface::LiveRoPg13
                | Surface::LiveSshRo
                | Surface::LiveSshRw
        )
    }

    /// The logical window sizes of the matrix shots: 1280x800, plus 1024x700 for the
    /// connection dialog (the Overflow Rule); `shell-narrow` only at 1024x700 and
    /// `shell-minimum` only at the 960x600 minimum window (DESIGN.md).
    fn sizes(self) -> &'static [(f32, f32)] {
        if self.is_live() {
            // ## m2b-t22-live: the default window and the minimum one (ruling I1).
            return &[(1280.0, 800.0), (960.0, 600.0)];
        }
        match self {
            Surface::ShellNarrow => &[(1024.0, 700.0)],
            Surface::ShellMinimum => &[(960.0, 600.0)],
            Surface::ConnDialogNew
            | Surface::ConnDialogPasted
            | Surface::ConnDialogEngineSwitch
            | Surface::ConnDialogSqlite => &[(1280.0, 800.0), (1024.0, 700.0)],
            _ => &[(1280.0, 800.0)],
        }
    }

    fn is_demo(self) -> bool {
        self == Surface::ShellDemoProduction
    }

    /// Whether the title-bar capsule shows production (the one filled badge): the demo, and
    /// the capsule fixtures of a production connection (connected, or failed with PRODUCTION
    /// first, NOTES §7).
    fn shows_production(self) -> bool {
        matches!(
            self,
            Surface::ShellDemoProduction
                | Surface::CapsuleProductionRo
                | Surface::CapsuleFailed
                // ## m2b-t22-live
                | Surface::LiveReconnecting
                | Surface::LiveLostTx
                | Surface::LiveLostReconnected
                | Surface::LiveGap
                | Surface::LiveRoEnforced
                | Surface::LiveSshRo
        )
    }

    /// Whether the shell boots the M2a connections (a `FakeDriver` registry, a memory secret
    /// store): every M2a surface. M1's surfaces render the shell without them, as in M1.
    fn needs_connections(self) -> bool {
        !matches!(
            self,
            Surface::Shell
                | Surface::ShellDemoProduction
                | Surface::ShellNarrow
                | Surface::PaletteOpen
                | Surface::SettingsGeneral
                | Surface::DialogAbout
                | Surface::ToastInfo
                | Surface::MenuApp
        )
    }

    /// Proof that the surface is on screen: a label created in this source file (and, for the
    /// demo, with this text). `RenderedText::path` is the label's creation site.
    fn marker(self) -> (&'static str, Option<&'static str>) {
        match self {
            Surface::Shell | Surface::ShellNarrow => ("hacivat-ui/src/titlebar.rs:", None),
            Surface::ShellDemoProduction => ("hacivat-ui/src/capsule.rs:", Some("orders-prod")),
            Surface::PaletteOpen => ("hacivat-ui/src/palette.rs:", None),
            Surface::SettingsGeneral => ("hacivat-app/src/settings_ui/", None),
            Surface::DialogAbout => ("hacivat-ui/src/dialog.rs:", None),
            Surface::ToastInfo => ("hacivat-ui/src/toast.rs:", None),
            Surface::MenuApp => ("hacivat-ui/src/menu.rs:", None),
            Surface::ConnDialogNew | Surface::ConnDialogSqlite => {
                ("hacivat-app/src/connections/dialog/", None)
            }
            Surface::ConnDialogPasted | Surface::ConnDialogEngineSwitch => {
                ("hacivat-ui/src/notice.rs:", None)
            }
            Surface::ConnTestRunning | Surface::ConnTestOk => {
                ("hacivat-app/src/connections/test_strip.rs:", None)
            }
            Surface::ConnTestFailed
            | Surface::ConnErrorAuth
            | Surface::ConnErrorUnknownDb
            | Surface::ConnErrorUnsupported
            | Surface::ConnErrorTls => ("hacivat-app/src/connections/error_view.rs:", None),
            Surface::ConnProdPrompt | Surface::ConnProdPromptUnset => {
                ("hacivat-app/src/connections/prod_prompt.rs:", None)
            }
            Surface::ExplorerEmpty => ("hacivat-app/src/connections/explorer.rs:", None),
            Surface::ExplorerList | Surface::ShellMinimum => (
                "hacivat-app/src/connections/explorer.rs:",
                Some("orders-prod"),
            ),
            Surface::ExplorerMenu => ("hacivat-ui/src/menu.rs:", None),
            Surface::ConnDeleteConfirm => ("hacivat-ui/src/dialog.rs:", None),
            Surface::ConnImportPgService => ("hacivat-app/src/connections/import.rs:", None),
            Surface::CapsuleConnecting => ("hacivat-ui/src/capsule.rs:", Some("orders-staging")),
            Surface::CapsuleConnectedUnencrypted => {
                ("hacivat-ui/src/capsule.rs:", Some("orders-dev"))
            }
            Surface::CapsuleProductionRo | Surface::CapsuleFailed => {
                ("hacivat-ui/src/capsule.rs:", Some("orders-prod"))
            }
            Surface::CapsuleUnsupported => ("hacivat-ui/src/capsule.rs:", Some("legacy")),
            Surface::UnencryptedPopover => ("hacivat-ui/src/explain.rs:", None),
            Surface::ToastStickyDemo => ("hacivat-ui/src/toast.rs:", None),
            // ## m2b-t22-live
            Surface::LiveReconnecting | Surface::LiveSshRo => {
                ("hacivat-ui/src/capsule.rs:", Some("orders-prod"))
            }
            Surface::LiveSshRw => ("hacivat-ui/src/capsule.rs:", Some("orders-staging")),
            Surface::LiveIdle
            | Surface::LiveLostTx
            | Surface::LiveLostReconnected
            | Surface::LiveGap => ("hacivat-ui/src/toast.rs:", None),
            Surface::LiveRoEnforced | Surface::LiveRoGuarded | Surface::LiveRoPg13 => {
                ("hacivat-app/src/connections/access_popover.rs:", None)
            }
        }
    }

    /// A violation when the frame shows no label of this surface.
    fn missing_from(self, name: &str, frame: &[RenderedText]) -> Option<String> {
        let (file, text) = self.marker();
        let shown = frame
            .iter()
            .any(|r| r.path.contains(file) && text.is_none_or(|t| strip(&r.text) == t));
        (!shown).then(|| format!("{name}: the surface is missing: no label from {file} {text:?}"))
    }

    /// Brings the surface on screen in a freshly started shell.
    fn open(self, window: &mut Window, cx: &mut App) {
        if self.needs_connections() {
            open_m2a(self, window, cx);
            return;
        }
        let action = match self {
            Surface::Shell | Surface::ShellDemoProduction | Surface::ShellNarrow => return,
            Surface::PaletteOpen => "workspace.command_palette",
            Surface::SettingsGeneral => "workspace.open_settings",
            Surface::DialogAbout => "app.about",
            Surface::ToastInfo => "workspace.reload_translations",
            Surface::MenuApp => {
                open_app_menu(window, cx);
                return;
            }
            _ => unreachable!("an M2a surface opens through its fixture"),
        };
        assert!(
            ActionRegistry::run(action, window, cx),
            "{action} did not run"
        );
    }
}

// ## m2a-t21-shots

/// Opens an M2a surface through the fixture its task owns (R28, P19).
fn open_m2a(surface: Surface, window: &mut Window, cx: &mut App) {
    match surface {
        Surface::ConnDialogNew => dialog::new_fixture(window, cx),
        Surface::ConnDialogPasted => dialog::pasted_fixture(window, cx),
        Surface::ConnDialogEngineSwitch => dialog::engine_switch_fixture(window, cx),
        Surface::ConnDialogSqlite => dialog::sqlite_fixture(window, cx),
        Surface::ConnTestRunning => test_strip::running_fixture(window, cx),
        Surface::ConnTestOk => test_strip::ok_fixture(window, cx),
        Surface::ConnTestFailed => test_strip::failed_fixture(window, cx),
        Surface::ConnErrorAuth => error_view::auth_fixture(window, cx),
        Surface::ConnErrorUnknownDb => error_view::unknown_db_fixture(window, cx),
        Surface::ConnErrorUnsupported => error_view::unsupported_fixture(window, cx),
        Surface::ConnErrorTls => error_view::tls_fixture(window, cx),
        Surface::ConnProdPrompt => prod_prompt::preselected_fixture(window, cx),
        Surface::ConnProdPromptUnset => prod_prompt::unset_fixture(window, cx),
        Surface::ExplorerEmpty => explorer::empty_fixture(window, cx),
        // The menu opens in `after_paint`, under the selected row as Shift-F10 does: the
        // row's bounds exist only once the list has painted.
        Surface::ExplorerList | Surface::ShellMinimum | Surface::ExplorerMenu => {
            explorer::list_fixture(window, cx)
        }
        Surface::ConnDeleteConfirm => delete::confirm_fixture(window, cx),
        Surface::ConnImportPgService => import::pg_service_fixture(window, cx),
        Surface::CapsuleConnecting => capsule::connecting_fixture(window, cx),
        Surface::CapsuleConnectedUnencrypted => capsule::connected_unencrypted_fixture(window, cx),
        Surface::CapsuleProductionRo => capsule::production_ro_fixture(window, cx),
        Surface::CapsuleFailed => capsule::failed_fixture(window, cx),
        Surface::CapsuleUnsupported => capsule::unsupported_fixture(window, cx),
        Surface::UnencryptedPopover => capsule::unencrypted_popover_fixture(window, cx),
        // ## m2b-t22-live
        Surface::LiveReconnecting => capsule::reconnecting_fixture(window, cx),
        Surface::LiveIdle => capsule::idle_fixture(window, cx),
        Surface::LiveLostTx => capsule::lost_tx_fixture(window, cx),
        Surface::LiveLostReconnected => capsule::lost_reconnected_fixture(window, cx),
        Surface::LiveGap => capsule::gap_fixture(window, cx),
        Surface::LiveRoEnforced => capsule::ro_enforced_fixture(window, cx),
        Surface::LiveRoGuarded => capsule::ro_guarded_fixture(window, cx),
        Surface::LiveRoPg13 => capsule::ro_pg13_fixture(window, cx),
        Surface::LiveSshRo => capsule::ssh_ro_fixture(window, cx),
        Surface::LiveSshRw => capsule::ssh_rw_fixture(window, cx),
        // The widget owns no copy: the app's strings (the Copy as URL toast, the keyring
        // fallback as a long bullet that wraps, the toast's own Dismiss).
        Surface::ToastStickyDemo => hacivat_ui::toast::sticky_demo_fixture(
            text_fn_id("hv-conn-toast-copied"),
            vec![
                text_fn_id("hv-conn-toast-nopw"),
                text_fn_id("hv-conn-keyring-unavailable"),
            ],
            text_fn_id("hui-toast-dismiss"),
            window,
            cx,
        ),
        _ => unreachable!("an M1 surface opens through its action"),
    }
}

/// The step of a surface that needs one painted frame first.
fn after_paint(surface: Surface, window: &mut Window, cx: &mut App) {
    if surface == Surface::ExplorerMenu {
        explorer::open_context_menu(None, window, cx);
    }
}

/// The connections an M2a shot boots: `ConnectionsBoot::for_tests` (a memory secret store, no
/// credential files) over a `FakeDriver` per engine, so no shot touches the keyring, a socket
/// or DNS.
fn fake_connections(paths: &LocalPaths) -> ConnectionsBoot {
    let mut drivers = DriverRegistry::new();
    for kind in [
        DbmsKind::Postgres,
        DbmsKind::MySql,
        DbmsKind::MariaDb,
        DbmsKind::Sqlite,
    ] {
        drivers.register(Arc::new(FakeDriver::new(kind)));
    }
    ConnectionsBoot::for_tests(paths, drivers)
}

/// `app.show_menu`'s popup. The action exists only where the title bar draws the app-menu
/// button (Windows, Linux); on macOS the harness opens the same popup directly.
fn open_app_menu(window: &mut Window, cx: &mut App) {
    if cfg!(not(target_os = "macos")) {
        assert!(ActionRegistry::run("app.show_menu", window, cx));
        return;
    }
    let ws = window
        .root::<Workspace>()
        .flatten()
        .expect("the root view is the workspace");
    let items = menus::popup_items(&menus::menu_model(cx.global::<ActionRegistry>()), cx);
    let anchor = menus::in_window_menu_anchor(&items, window, cx);
    let menu = ws.update(cx, |ws, cx| ws.open_popup_menu(anchor, items, window, cx));
    menu.update(cx, |m, _| m.label(text_fn_id("hui-menu-app")));
}

/// One frame to render.
#[derive(Debug, Clone, Copy)]
struct Shot {
    surface: Surface,
    theme: ThemePref,
    locale: &'static str,
    size: (f32, f32),
}

impl Shot {
    fn theme_name(&self) -> &'static str {
        match self.theme {
            ThemePref::Light => "light",
            _ => "dark",
        }
    }

    fn file_name(&self, scale: f32) -> String {
        format!(
            "{}-{}-{}-{}x{}@{}x.png",
            self.surface.name(),
            self.theme_name(),
            self.locale,
            self.size.0,
            self.size.1,
            scale
        )
    }
}

/// Every shot: the surface matrix at each surface's sizes x locales x themes, then
/// `shell-demo-production` at 1440x900 x en/tr x dark/light for VF.
fn shots() -> Vec<Shot> {
    let mut out = Vec::new();
    for surface in SURFACES {
        for &size in surface.sizes() {
            for theme in THEMES {
                for locale in LOCALES {
                    out.push(Shot {
                        surface,
                        theme,
                        locale,
                        size,
                    });
                }
            }
        }
    }
    for theme in THEMES {
        for locale in ["en", "tr"] {
            out.push(Shot {
                surface: Surface::ShellDemoProduction,
                theme,
                locale,
                size: (1440.0, 900.0),
            });
        }
    }
    out
}

fn out_dir() -> PathBuf {
    std::env::var_os("SHOTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../target/shots")
                .canonicalize()
                .unwrap_or_else(|_| {
                    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/shots")
                })
        })
}

/// Without isolation marks (U+2068/U+2069), which wrap placeables.
fn strip(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

/// Assertions (a) to (c) on what the frame drew; one message per violation.
fn check_labels(name: &str, locale: &str, frame: &[RenderedText]) -> Vec<String> {
    let mut out = Vec::new();
    for r in frame {
        let text = strip(&r.text);
        if PSEUDO.contains(&locale)
            && r.source == TextSource::Fluent
            && !(text.starts_with('[') && text.ends_with(']'))
        {
            out.push(format!(
                "{name}: (a) Fluent label not bracketed: {:?} at {}",
                text, r.path
            ));
        }
        if r.clipped {
            out.push(format!(
                "{name}: (b) clipped: {:?} ({:.1} px painted) at {}",
                text, r.painted_px, r.path
            ));
        }
        if r.size_px < 11.0 {
            out.push(format!(
                "{name}: (c) {} px < 11: {:?} at {}",
                r.size_px, text, r.path
            ));
        }
    }
    out
}

// ## m2b-t22-live (fix round 2, R1-I1)
/// (e) The title bar's layout (a geometry check: an icon button pushed off the window draws no
/// clipped text): the trailing controls sit wholly inside the window and end where the bar's
/// right padding puts them, the palette field keeps `PALETTE_MIN_PX` and ends before them,
/// and the capsule ends before the palette field begins.
fn check_title_bar(width: f32, bar: TitleBarProbe, capsule: Option<Bounds<Pixels>>) -> Vec<String> {
    const SLACK: f32 = 0.5;
    let mut out = Vec::new();
    let Some(trailing) = bar.trailing else {
        return vec!["the title bar drew no trailing controls".to_string()];
    };
    let (left, right) = (f32::from(trailing.left()), f32::from(trailing.right()));
    let end = width - bar.right_padding;
    if left < -SLACK || right > width + SLACK {
        out.push(format!(
            "the trailing controls ({left:.1}..{right:.1}) leave the {width} px window"
        ));
    } else if (right - end).abs() > SLACK {
        out.push(format!(
            "the trailing controls end at {right:.1}, not at {end:.1}: something pushed them"
        ));
    }
    let Some(palette) = bar.palette else {
        out.push("the title bar drew no palette field".to_string());
        return out;
    };
    let (p_left, p_right) = (f32::from(palette.left()), f32::from(palette.right()));
    let palette_w = f32::from(palette.size.width);
    if palette_w < PALETTE_MIN_PX - SLACK {
        out.push(format!(
            "the palette field is {palette_w:.1} px, under its {PALETTE_MIN_PX} px minimum"
        ));
    }
    if p_left < -SLACK || p_right > left + SLACK {
        out.push(format!(
            "the palette field ({p_left:.1}..{p_right:.1}) runs into the trailing controls at \
             {left:.1}"
        ));
    }
    if let Some(capsule) = capsule {
        let c_right = f32::from(capsule.right());
        if c_right > p_left + SLACK {
            out.push(format!(
                "the capsule ends at {c_right:.1}, past the palette field's start at {p_left:.1}"
            ));
        }
    }
    out
}

/// Renders `shot` in a fresh app, checks the frame and writes the PNG. Returns the violations.
fn render(shot: Shot, dir: &Path) -> Vec<String> {
    let platform = current_platform(true);
    let mut cx = HeadlessAppContext::with_platform(
        platform.text_system(),
        Arc::new(BundledAssets),
        current_headless_renderer,
    );
    cx.update(|cx| {
        bootstrap(cx).expect("bundled fonts register and resolve");
        Theme::install(cx, ThemeMode::Dark);
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        });
        cx.set_global(CapsuleProbe::default());
        cx.set_global(TitleBarProbe::default());
    });
    let root = tempfile::tempdir().expect("temp dir");
    let paths = LocalPaths::under(root.path());
    paths.ensure_dirs().expect("test dirs");
    let mut outcome = hacivat_settings::load(&paths.settings_toml());
    outcome.settings.ui.language = LanguagePref::Tag(shot.locale.to_string());
    outcome.settings.ui.theme = shot.theme;
    let connections = shot
        .surface
        .needs_connections()
        .then(|| fake_connections(&paths));
    let mut startup = Startup::new(paths, outcome, OsLocale::default());
    startup.connections = connections;
    let demo = shot.surface.is_demo().then(DemoCapsule::harman);
    let handle: AnyWindowHandle = cx
        .update(|cx| start(cx, startup, demo))
        .expect("the workspace opens")
        .into();
    let (w, h) = shot.size;
    cx.update_window(handle, |_, window, cx| {
        window.resize(size(px(w), px(h)));
        window.bounds_changed(cx);
        // A real app window is key; focus rings and carets draw as they would there.
        window.activate_window();
    })
    .expect("the window is open");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| shot.surface.open(window, cx))
        .expect("the window is open");
    cx.run_until_parked();
    if shot.surface.needs_connections() {
        // Profile writes and the keyring probe run on the core runtime (P22); let them land.
        std::thread::sleep(std::time::Duration::from_millis(30));
        cx.run_until_parked();
    }
    cx.update(|cx| cx.refresh_windows());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        after_paint(shot.surface, window, cx)
    })
    .expect("the window is open");
    cx.run_until_parked();
    cx.update(|cx| cx.refresh_windows());
    cx.run_until_parked();

    let scale = cx
        .update_window(handle, |_, window, _| window.scale_factor())
        .expect("the window is open");
    let name = shot.file_name(scale);
    let frame = cx.update(|cx| cx.global::<TextRegistry>().frame.clone());
    let mut failures = check_labels(&name, shot.locale, &frame);
    if frame.is_empty() {
        failures.push(format!("{name}: the frame drew no label"));
    }
    failures.extend(shot.surface.missing_from(&name, &frame));
    let badge = cx
        .update_window(handle, |_, window, cx| {
            let cp_fill = Theme::get(cx).tokens.colors.cp_fill.to_hsla();
            let capsule = cx.global::<CapsuleProbe>().bounds;
            catch_unwind(AssertUnwindSafe(|| {
                assert_one_filled_badge_in(
                    window,
                    cp_fill,
                    shot.surface.shows_production(),
                    capsule,
                )
            }))
        })
        .expect("the window is open");
    if let Err(panic) = badge {
        let msg = panic
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_else(|| "One Filled Badge rule".to_string());
        failures.push(format!("{name}: (d) {msg}"));
    }
    let (bar, capsule) = cx.update(|cx| {
        (
            *cx.global::<TitleBarProbe>(),
            cx.global::<CapsuleProbe>().bounds,
        )
    });
    failures.extend(
        check_title_bar(w, bar, capsule)
            .into_iter()
            .map(|v| format!("{name}: (e) {v}")),
    );

    let image = cx.capture_screenshot(handle).expect("the frame renders");
    let want = ((w * scale).round() as u32, (h * scale).round() as u32);
    if image.dimensions() != want {
        failures.push(format!(
            "{name}: image is {:?}, expected {want:?}",
            image.dimensions()
        ));
    }
    image.save(dir.join(&name)).expect("write the PNG");
    println!("shot {name}: {} labels", frame.len());
    failures
}

/// The main check: renders the shots named in `names` (file names), then fails with every
/// violation found. Only a full run (`all`) removes the PNGs already in the directory.
fn render_all_surfaces(names: &[String], all: bool) {
    match current_headless_renderer() {
        Ok(Some(_)) => {}
        Ok(None) => {
            println!("shots: no headless renderer (R8)");
            return;
        }
        Err(err) => panic!("shots: the headless renderer failed to start: {err:#}"),
    }
    // GPUI reads the font-smoothing preference once, on the first glyph (R11).
    apply_text_rendering(TextRendering::Crisp);
    let dir = out_dir();
    std::fs::create_dir_all(&dir).expect("create the shots directory");
    // Stale PNGs never pass for fresh ones; a filtered run keeps the shots it does not render.
    if all {
        for entry in std::fs::read_dir(&dir)
            .expect("read the shots directory")
            .flatten()
        {
            if entry.path().extension().is_some_and(|e| e == "png") {
                std::fs::remove_file(entry.path()).expect("remove a stale shot");
            }
        }
    }
    let mut failures = Vec::new();
    let mut rendered = 0;
    for shot in shots() {
        if !names.contains(&shot.file_name(2.0)) {
            continue;
        }
        failures.extend(render(shot, &dir));
        rendered += 1;
    }
    println!("shots: {rendered} rendered into {}", dir.display());
    assert!(rendered > 0, "no shot selected");
    assert!(
        failures.is_empty(),
        "{} violations:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn the_matrix_covers_every_surface_locale_and_theme() {
    let all = shots();
    // M1: 8 surfaces x 8 + 4 VF; M2a: 26 surfaces x 8 + the 4 dialog surfaces again at 1024x700;
    // M2b T22: 10 surfaces x 8 at 1280x800 and again at 960x600.
    assert_eq!(all.len(), 8 * 4 * 2 + 4 + 26 * 8 + 4 * 8 + 10 * 8 * 2);
    let names: Vec<String> = all.iter().map(|s| s.file_name(2.0)).collect();
    assert!(names.contains(&"live-ssh-ro-dark-tr-XT-960x600@2x.png".to_string()));
    assert!(names.contains(&"shell-dark-en-1280x800@2x.png".to_string()));
    assert!(names.contains(&"shell-narrow-light-tr-XT-1024x700@2x.png".to_string()));
    assert!(names.contains(&"shell-demo-production-light-tr-1440x900@2x.png".to_string()));
    assert!(names.contains(&"menu-app-dark-en-XA-1280x800@2x.png".to_string()));
    assert!(names.contains(&"conn-dialog-new-light-tr-XT-1024x700@2x.png".to_string()));
    assert!(names.contains(&"capsule-production-ro-dark-tr-1280x800@2x.png".to_string()));
    assert!(names.contains(&"shell-minimum-dark-en-960x600@2x.png".to_string()));
    assert!(!names.contains(&"conn-error-auth-dark-en-1024x700@2x.png".to_string()));
    let unique: std::collections::BTreeSet<&String> = names.iter().collect();
    assert_eq!(unique.len(), names.len());
}

fn pseudo_labels_must_be_bracketed_and_unclipped_and_at_least_11_px() {
    let label = |text: &str, source, size_px, clipped| RenderedText {
        text: text.to_string().into(),
        source,
        size_px,
        painted_px: 10.0,
        clipped,
        truncated: false,
        path: "x.rs:1:1".into(),
    };
    let frame = [
        label("[Ŝéåŕçĥ]", TextSource::Fluent, 13.0, false),
        label("\u{2068}[Ŝéåŕçĥ]\u{2069}", TextSource::Fluent, 13.0, false),
        label("EN", TextSource::Data, 11.5, false),
        label("Search", TextSource::Fluent, 13.0, false),
        label("[ok]", TextSource::Fluent, 10.5, true),
    ];
    assert!(check_labels("s", "en", &frame[..3]).is_empty());
    let found = check_labels("s", "en-XA", &frame);
    assert_eq!(found.len(), 3, "{found:#?}");
    assert!(found[0].contains("(a)") && found[0].contains("Search"));
    assert!(found[1].contains("(b)"));
    assert!(found[2].contains("(c) 10.5 px"));
    // Outside the pseudo locales an unbracketed Fluent label is fine.
    assert!(check_labels("s", "tr", &frame[3..4]).is_empty());
}

/// The ignored test: it renders, so it runs only on request.
const RENDER_TEST: &str = "render_all_surfaces";

/// The command line, read the way libtest reads it.
#[derive(Debug, Default, PartialEq)]
struct Cli {
    /// Positional arguments: test names, or (for the render) shot file names.
    filters: Vec<String>,
    /// `--skip`: tests whose name contains one of these do not run.
    skips: Vec<String>,
    exact: bool,
    list: bool,
    /// Only the ignored tests (`--ignored`).
    ignored_only: bool,
    /// Ignored tests too (`--include-ignored`).
    include_ignored: bool,
    terse: bool,
}

impl Cli {
    fn parse(args: impl IntoIterator<Item = String>) -> Cli {
        let mut cli = Cli::default();
        let mut it = args.into_iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--exact" => cli.exact = true,
                "--list" => cli.list = true,
                "--ignored" => cli.ignored_only = true,
                "--include-ignored" => cli.include_ignored = true,
                "--skip" => cli.skips.extend(it.next()),
                "--format" => cli.terse |= it.next().as_deref() == Some("terse"),
                // Flags with a value that change nothing here.
                "--test-threads" | "--color" | "--logfile" | "-Z" => {
                    it.next();
                }
                s if s.starts_with("--format=") => cli.terse |= s == "--format=terse",
                s if s.starts_with('-') => {}
                _ => cli.filters.push(a),
            }
        }
        cli
    }

    /// Whether `pattern` selects `name` (substring, or equality with `--exact`).
    fn matches(&self, pattern: &str, name: &str) -> bool {
        if self.exact {
            name == pattern || name.strip_suffix(".png") == Some(pattern)
        } else {
            name.contains(pattern)
        }
    }

    fn skipped(&self, name: &str) -> bool {
        self.skips.iter().any(|s| name.contains(s.as_str()))
    }

    /// Whether test `name` is selected by the filters (and not skipped).
    fn selects_test(&self, name: &str) -> bool {
        !self.skipped(name)
            && (self.filters.is_empty() || self.filters.iter().any(|f| self.matches(f, name)))
    }

    /// The shots the render covers: all of them when the test is selected by its own name or
    /// by no filter, else the shots a filter names. `None` when the render is not selected.
    fn shots(&self, all: &[String]) -> Option<Vec<String>> {
        if self.skipped(RENDER_TEST) {
            return None;
        }
        if self.selects_test(RENDER_TEST) {
            return Some(all.to_vec());
        }
        let picked: Vec<String> = all
            .iter()
            .filter(|s| self.filters.iter().any(|f| self.matches(f, s)))
            .cloned()
            .collect();
        (!picked.is_empty()).then_some(picked)
    }

    fn runs_ignored(&self) -> bool {
        self.ignored_only || self.include_ignored
    }
}

/// libtest's contract without its threads. `--list` lists every test (`--ignored`: only the
/// ignored render). The self-checks run unless `--ignored`; the render runs with `--ignored` or
/// `--include-ignored`. A filter naming the render (or, without matching a test, shots) selects
/// what it renders. Asked for the render by its exact name without `--ignored` (how nextest runs
/// one test), the binary fails instead of passing without rendering.
fn main() {
    let cli = Cli::parse(std::env::args().skip(1));
    let checks: [(&str, fn()); 5] = [
        (
            "a_missing_surface_fails_its_frame",
            a_missing_surface_fails_its_frame,
        ),
        (
            "a_pushed_title_bar_fails_its_frame",
            a_pushed_title_bar_fails_its_frame,
        ),
        (
            "the_matrix_covers_every_surface_locale_and_theme",
            the_matrix_covers_every_surface_locale_and_theme,
        ),
        (
            "pseudo_labels_must_be_bracketed_and_unclipped_and_at_least_11_px",
            pseudo_labels_must_be_bracketed_and_unclipped_and_at_least_11_px,
        ),
        (
            "the_command_line_reads_like_libtest",
            the_command_line_reads_like_libtest,
        ),
    ];
    let all: Vec<String> = shots().iter().map(|s| s.file_name(2.0)).collect();
    let render = cli.shots(&all);
    if cli.list {
        let mut n = 0;
        if !cli.ignored_only {
            for (name, _) in checks.iter().filter(|(name, _)| cli.selects_test(name)) {
                println!("{name}: test");
                n += 1;
            }
        }
        if render.is_some() {
            println!("{RENDER_TEST}: test");
            n += 1;
        }
        if !cli.terse {
            println!("\n{n} tests, 0 benchmarks");
        }
        return;
    }
    let (mut passed, mut ignored) = (0, 0);
    if !cli.ignored_only {
        for (name, check) in checks.iter().filter(|(name, _)| cli.selects_test(name)) {
            check();
            println!("test {name} ... ok");
            passed += 1;
        }
    }
    match render {
        Some(names) if cli.runs_ignored() => {
            render_all_surfaces(&names, names.len() == all.len());
            println!("test {RENDER_TEST} ... ok");
            passed += 1;
        }
        Some(_) if cli.exact => {
            eprintln!("{RENDER_TEST} is ignored: run it with --ignored or --include-ignored");
            std::process::exit(101);
        }
        Some(_) => {
            println!("test {RENDER_TEST} ... ignored, renders PNGs (run with --ignored)");
            ignored += 1;
        }
        None => {}
    }
    println!("\ntest result: ok. {passed} passed; 0 failed; {ignored} ignored");
}

// ## m2b-t22-live (fix round 2, R1-I1)
fn a_pushed_title_bar_fails_its_frame() {
    let b = |x: f32, w: f32| Bounds::new(point(px(x), px(0.0)), size(px(w), px(40.0)));
    // A 960 px window, right padding 10: trailing controls 830..950, palette 400..620.
    let good = TitleBarProbe {
        palette: Some(b(400.0, 220.0)),
        trailing: Some(b(830.0, 120.0)),
        right_padding: 10.0,
    };
    assert!(check_title_bar(960.0, good, Some(b(100.0, 280.0))).is_empty());
    // Pushed right by a wide capsule: the theme button leaves the window.
    let pushed = TitleBarProbe {
        trailing: Some(b(865.0, 120.0)),
        ..good
    };
    let found = check_title_bar(960.0, pushed, None);
    assert!(found.iter().any(|v| v.contains("leave the")), "{found:?}");
    // Pushed by less than its padding: still inside, but no longer at its place.
    let nudged = TitleBarProbe {
        trailing: Some(b(834.0, 120.0)),
        ..good
    };
    let found = check_title_bar(960.0, nudged, None);
    assert!(
        found.iter().any(|v| v.contains("something pushed them")),
        "{found:?}"
    );
    // A squeezed palette field, and a capsule running under it.
    let squeezed = TitleBarProbe {
        palette: Some(b(500.0, 160.0)),
        ..good
    };
    let found = check_title_bar(960.0, squeezed, Some(b(100.0, 420.0)));
    assert!(found.iter().any(|v| v.contains("under its")), "{found:?}");
    assert!(
        found.iter().any(|v| v.contains("past the palette")),
        "{found:?}"
    );
    // No title bar drawn at all.
    assert!(!check_title_bar(960.0, TitleBarProbe::default(), None).is_empty());
}

fn a_missing_surface_fails_its_frame() {
    let at = |path: &str, text: &str| RenderedText {
        text: text.to_string().into(),
        source: TextSource::Fluent,
        size_px: 13.0,
        painted_px: 10.0,
        clipped: false,
        truncated: false,
        path: path.to_string().into(),
    };
    let shell = [at(
        "crates/ui/hacivat-ui/src/titlebar.rs:297:25#palette-placeholder",
        "Search",
    )];
    assert!(Surface::Shell.missing_from("s", &shell).is_none());
    for surface in [
        Surface::PaletteOpen,
        Surface::SettingsGeneral,
        Surface::DialogAbout,
        Surface::ToastInfo,
        Surface::MenuApp,
        Surface::ShellDemoProduction,
    ] {
        let found = surface
            .missing_from("s", &shell)
            .expect("a shell-only frame misses it");
        assert!(found.contains("the surface is missing"), "{found}");
    }
    // ## m2a-t21-shots: an M2a surface needs its own file's label too.
    for surface in [
        Surface::ConnDialogNew,
        Surface::ConnErrorAuth,
        Surface::ExplorerList,
        Surface::UnencryptedPopover,
        Surface::ToastStickyDemo,
    ] {
        assert!(surface.missing_from("s", &shell).is_some(), "{surface:?}");
    }
    let explorer = [at(
        "crates/app/hacivat-app/src/connections/explorer.rs:420:17",
        "orders-prod",
    )];
    assert!(Surface::ExplorerList.missing_from("s", &explorer).is_none());
    // The demo needs the harman capsule, not just any capsule label.
    let capsule = |text| [at("crates/ui/hacivat-ui/src/capsule.rs:111:21", text)];
    assert!(
        Surface::ShellDemoProduction
            .missing_from("s", &capsule("No connection"))
            .is_some()
    );
    assert!(
        Surface::ShellDemoProduction
            .missing_from("s", &capsule("orders-prod"))
            .is_none()
    );
}

fn the_command_line_reads_like_libtest() {
    let parse = |s: &str| Cli::parse(s.split_whitespace().map(str::to_string));
    let all: Vec<String> = shots().iter().map(|s| s.file_name(2.0)).collect();
    // No filter: everything; the render is ignored unless asked for.
    let cli = parse("--nocapture --test-threads 1");
    assert_eq!(cli.shots(&all).map(|s| s.len()), Some(468));
    assert!(!cli.runs_ignored());
    // nextest: `--list --format terse --ignored` lists only the render.
    let cli = parse("--list --format terse --ignored");
    assert!(cli.list && cli.terse && cli.ignored_only);
    // A filter naming shots renders only those; one naming the test renders all.
    assert_eq!(
        parse("--ignored palette-open-dark")
            .shots(&all)
            .map(|s| s.len()),
        Some(4)
    );
    assert_eq!(
        parse("--ignored render_all").shots(&all).map(|s| s.len()),
        Some(468)
    );
    // A dialog surface renders at both sizes.
    assert_eq!(
        parse("--ignored conn-dialog-new-dark")
            .shots(&all)
            .map(|s| s.len()),
        Some(8)
    );
    // `--exact` matches whole names (a shot with or without `.png`).
    assert_eq!(parse("--exact render_all").shots(&all), None);
    assert_eq!(
        parse("--exact --ignored shell-dark-en-1280x800@2x").shots(&all),
        Some(vec!["shell-dark-en-1280x800@2x.png".to_string()])
    );
    assert!(parse("--exact render_all_surfaces").selects_test(RENDER_TEST));
    assert!(
        !parse("--exact the_matrix")
            .selects_test("the_matrix_covers_every_surface_locale_and_theme")
    );
    // `--skip` drops tests; a filter matching nothing selects nothing.
    assert_eq!(parse("--skip render").shots(&all), None);
    assert_eq!(parse("no-such-shot").shots(&all), None);
}
