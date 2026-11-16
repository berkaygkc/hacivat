//! Command-line flags and the launch probes: `--smoke` (F18) prints the first-frame time and the
//! linked libpg_query (M3a), 5 s later the idle resident set, then exits; `--print-window-id`
//! prints what `design/vf/capture.sh app` needs to capture the window.

use std::cell::Cell;
use std::io::Write as _;
use std::rc::Rc;
use std::time::{Duration, Instant};

use hacivat_app::SessionOverrides;
use hacivat_app::l10n_global::WINDOW_TITLE;
use hacivat_settings::ThemePref;
use hacivat_ui_core::gpui_surface::{AnyWindowHandle, App, Global};
use hacivat_ui_core::platform;
use tracing_appender::non_blocking::WorkerGuard;

/// How long `--smoke` waits after the first frame before reading the resident set.
pub const IDLE_WAIT: Duration = Duration::from_secs(5);
/// How long `--smoke` waits for the first frame before giving up.
pub const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(30);

/// The command-line flags.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Flags {
    /// Print `first-frame-ms=<n>`, then `idle-rss-mib=<n>` after [`IDLE_WAIT`], and exit.
    pub smoke: bool,
    /// Show the harman production capsule (visual-fidelity captures).
    pub vf_demo: bool,
    /// Print `WINDOW_ID=<n>` and `WINDOW_TITLE=<title>` once the window has drawn.
    pub print_window_id: bool,
    /// `--theme <dark|light|system>` and `--lang <tag>`: this session only, never saved.
    pub overrides: SessionOverrides,
}

/// Parses the flags after the program name; an unknown flag or a bad value is an error.
pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Flags, String> {
    let mut flags = Flags::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--smoke" => flags.smoke = true,
            "--vf-demo" => flags.vf_demo = true,
            "--print-window-id" => flags.print_window_id = true,
            "--theme" => {
                flags.overrides.theme = Some(match args.next().as_deref() {
                    Some("dark") => ThemePref::Dark,
                    Some("light") => ThemePref::Light,
                    Some("system") => ThemePref::System,
                    other => return Err(format!("--theme dark|light|system, got {other:?}")),
                });
            }
            "--lang" => match args.next() {
                Some(tag) if is_language_tag(&tag) => flags.overrides.language = Some(tag),
                other => return Err(format!("--lang <language tag>, got {other:?}")),
            },
            other => return Err(format!("unknown flag `{other}`")),
        }
    }
    Ok(flags)
}

/// A plausible BCP 47 tag (`en`, `tr-TR`, `en-XA`); negotiation decides what it maps to.
fn is_language_tag(tag: &str) -> bool {
    tag.parse::<hacivat_l10n::LanguageIdentifier>().is_ok()
}

/// The `first-frame-ms` line.
pub fn first_frame_line(elapsed: Duration) -> String {
    format!("first-frame-ms={}", elapsed.as_millis())
}

/// The `idle-rss-mib` line and the exit code: 0 with a reading, 1 when the platform cannot
/// report the resident set.
pub fn idle_rss_report(rss_mib: Option<u64>) -> (String, i32) {
    match rss_mib {
        Some(mib) => (format!("idle-rss-mib={mib}"), 0),
        None => ("idle-rss-mib unavailable".to_string(), 1),
    }
}

// ## m3a-t01a-proofs
/// The `libpg-query` line: the libpg_query the binary links and how many statements it finds
/// in `SELECT 1`, so the launch smoke proves the C library in the shipped binary (R-M3-1).
pub fn pg_query_line() -> String {
    match sql_oracle_pg::statement_count("SELECT 1") {
        Ok(n) => format!("libpg-query={} statements={n}", sql_oracle_pg::version()),
        Err(e) => format!("libpg-query={} error={e}", sql_oracle_pg::version()),
    }
}

/// The log writer's guard, kept for the whole run and dropped (flushing the log) on every way
/// out: quit, and the smoke probe's exit.
pub struct LogGuard(pub Option<WorkerGuard>);

impl Global for LogGuard {}

/// Flushes the log, then ends the process with `code`.
pub fn exit(cx: &mut App, code: i32) -> ! {
    if cx.has_global::<LogGuard>() {
        drop(cx.global_mut::<LogGuard>().0.take());
    }
    let _ = std::io::stdout().flush();
    std::process::exit(code);
}

/// Hooks the probes the flags ask for onto `window`'s first rendered frame. With `--smoke`, a
/// watchdog ends the run with exit 1 when no frame is drawn within [`FIRST_FRAME_TIMEOUT`].
pub fn install(cx: &mut App, window: AnyWindowHandle, flags: &Flags, started: Instant) {
    if !flags.smoke && !flags.print_window_id {
        return;
    }
    let (smoke, print_window_id) = (flags.smoke, flags.print_window_id);
    let drawn = Rc::new(Cell::new(false));
    if smoke {
        let drawn = drawn.clone();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(FIRST_FRAME_TIMEOUT).await;
            if !drawn.get() {
                eprintln!("first-frame-ms unavailable");
                cx.update(|cx| exit(cx, 1));
            }
        })
        .detach();
    }
    let hooked = window.update(cx, move |_, window, _| {
        window.on_next_frame(move |window, cx| {
            drawn.set(true);
            let mut out = std::io::stdout().lock();
            if print_window_id {
                match platform::native_window_id(window) {
                    Some(id) => {
                        let _ = writeln!(out, "WINDOW_ID={id}");
                    }
                    None => {
                        let _ = writeln!(out, "WINDOW_ID=unavailable");
                    }
                }
                let _ = writeln!(out, "WINDOW_TITLE={WINDOW_TITLE}");
            }
            if smoke {
                let _ = writeln!(out, "{}", first_frame_line(started.elapsed()));
                let _ = writeln!(out, "{}", pg_query_line());
                cx.spawn(async move |cx| {
                    cx.background_executor().timer(IDLE_WAIT).await;
                    let (line, code) = idle_rss_report(platform::resident_set_mib());
                    if code == 0 {
                        println!("{line}");
                    } else {
                        eprintln!("{line}");
                    }
                    cx.update(|cx| exit(cx, code));
                })
                .detach();
            }
            let _ = out.flush();
        });
    });
    if let Err(err) = hooked {
        eprintln!("first-frame-ms unavailable: {err:#}");
        if smoke {
            exit(cx, 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn flags_parse_and_unknown_flags_fail() {
        assert_eq!(parse(args(&[])), Ok(Flags::default()));
        assert_eq!(
            parse(args(&[
                "--smoke",
                "--vf-demo",
                "--print-window-id",
                "--theme",
                "light",
                "--lang",
                "tr"
            ])),
            Ok(Flags {
                smoke: true,
                vf_demo: true,
                print_window_id: true,
                overrides: SessionOverrides {
                    theme: Some(ThemePref::Light),
                    language: Some("tr".to_string()),
                },
            })
        );
        assert!(parse(args(&["--nope"])).is_err());
        assert!(parse(args(&["--theme", "blue"])).is_err());
        assert!(parse(args(&["--theme"])).is_err());
        assert!(parse(args(&["--lang", "not a tag"])).is_err());
        assert!(parse(args(&["--lang"])).is_err());
    }

    // ## m3a-t01a-proofs
    #[test]
    fn pg_query_line_names_the_linked_library_and_one_statement() {
        assert_eq!(
            pg_query_line(),
            "libpg-query=pg_query 18.1.0 (PostgreSQL 18.6) statements=1"
        );
    }

    #[test]
    fn smoke_lines_have_the_ci_format() {
        assert_eq!(
            first_frame_line(Duration::from_millis(312)),
            "first-frame-ms=312"
        );
        assert_eq!(
            idle_rss_report(Some(143)),
            ("idle-rss-mib=143".to_string(), 0)
        );
        assert_eq!(idle_rss_report(None).1, 1);
    }
}
