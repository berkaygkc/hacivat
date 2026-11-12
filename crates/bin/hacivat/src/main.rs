//! The Hacivat executable: local paths, settings, logs and localization are prepared before
//! GPUI starts; then the app opens the workspace window.
//!
//! Flags: `--smoke` (launch probe for CI), `--vf-demo` (the harman production capsule, for
//! visual-fidelity captures), `--print-window-id` (for `design/vf/capture.sh app`), and
//! `--theme <dark|light|system>` / `--lang <tag>` (this session only; settings are not saved).

mod logging;
mod smoke;

use std::time::Instant;

use hacivat_app::Startup;
use hacivat_app::shell::DemoCapsule;
use hacivat_l10n::OsLocale;
use hacivat_settings::{LocalPaths, TextRenderingPref};
use hacivat_ui_core::app::{Boot, TextRendering, run_app};

use smoke::LogGuard;

/// Exit code for bad flags.
const USAGE_EXIT: i32 = 2;
/// Exit code when the local directories or the window cannot be created.
const STARTUP_EXIT: i32 = 1;

fn main() {
    let started = Instant::now();
    let flags = match smoke::parse(std::env::args().skip(1)) {
        Ok(flags) => flags,
        Err(err) => {
            eprintln!(
                "hacivat: {err} (flags: --smoke, --vf-demo, --print-window-id, \
                 --theme dark|light|system, --lang <tag>)"
            );
            std::process::exit(USAGE_EXIT);
        }
    };
    let paths = match LocalPaths::from_os().and_then(|p| p.ensure_dirs().map(|()| p)) {
        Ok(paths) => paths,
        Err(err) => {
            eprintln!("hacivat: cannot prepare the local directories: {err}");
            std::process::exit(STARTUP_EXIT);
        }
    };
    let outcome = hacivat_settings::load(&paths.settings_toml());
    let log_guard = logging::init(&paths.logs_dir());
    tracing::info!(
        diags = outcome.diags.len(),
        quarantined = outcome.quarantined.is_some(),
        overrides = !flags.overrides.is_empty(),
        "settings loaded"
    );
    let startup = Startup::with_overrides(paths, outcome, OsLocale::detect(), &flags.overrides);
    let boot = Boot {
        text_rendering: match startup.settings.ui.text_rendering {
            TextRenderingPref::Crisp => TextRendering::Crisp,
            TextRenderingPref::Standard => TextRendering::Standard,
        },
    };
    let demo = flags.vf_demo.then(DemoCapsule::harman);
    run_app(boot, move |cx| {
        cx.set_global(LogGuard(log_guard));
        // The log is flushed before the process ends on a normal quit.
        cx.on_app_quit(|cx| {
            if cx.has_global::<LogGuard>() {
                drop(cx.global_mut::<LogGuard>().0.take());
            }
            async {}
        })
        .detach();
        match hacivat_app::start(cx, startup, demo) {
            Ok(window) => smoke::install(cx, window.into(), &flags, started),
            Err(err) => {
                tracing::error!(error = %format!("{err:#}"), "opening the workspace window failed");
                eprintln!("hacivat: opening the window failed: {err:#}");
                smoke::exit(cx, STARTUP_EXIT);
            }
        }
        cx.activate(true);
    });
}
