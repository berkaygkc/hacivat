//! Command-line flags and the launch probes: `--smoke` (F18) prints the first-frame time and,
//! 5 s later, the idle resident set, then exits; `--print-window-id` prints what
//! `design/vf/capture.sh app` needs to capture the window.

use std::io::Write as _;
use std::time::{Duration, Instant};

use hacivat_app::l10n_global::WINDOW_TITLE;
use hacivat_ui_core::gpui_surface::{AnyWindowHandle, App};
use hacivat_ui_core::platform;

/// How long `--smoke` waits after the first frame before reading the resident set.
pub const IDLE_WAIT: Duration = Duration::from_secs(5);

/// The command-line flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Flags {
    /// Print `first-frame-ms=<n>`, then `idle-rss-mib=<n>` after [`IDLE_WAIT`], and exit.
    pub smoke: bool,
    /// Show the harman production capsule (visual-fidelity captures).
    pub vf_demo: bool,
    /// Print `WINDOW_ID=<n>` and `WINDOW_TITLE=<title>` once the window has drawn.
    pub print_window_id: bool,
}

/// Parses the flags after the program name; an unknown flag is an error.
pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Flags, String> {
    let mut flags = Flags::default();
    for arg in args {
        match arg.as_str() {
            "--smoke" => flags.smoke = true,
            "--vf-demo" => flags.vf_demo = true,
            "--print-window-id" => flags.print_window_id = true,
            other => return Err(format!("unknown flag `{other}`")),
        }
    }
    Ok(flags)
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

/// Hooks the probes the flags ask for onto `window`'s first rendered frame.
pub fn install(cx: &mut App, window: AnyWindowHandle, flags: Flags, started: Instant) {
    if !flags.smoke && !flags.print_window_id {
        return;
    }
    let _ = window.update(cx, move |_, window, _| {
        window.on_next_frame(move |window, cx| {
            let mut out = std::io::stdout().lock();
            if flags.print_window_id {
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
            if flags.smoke {
                let _ = writeln!(out, "{}", first_frame_line(started.elapsed()));
                cx.spawn(async move |cx| {
                    cx.background_executor().timer(IDLE_WAIT).await;
                    let (line, code) = idle_rss_report(platform::resident_set_mib());
                    if code == 0 {
                        println!("{line}");
                    } else {
                        eprintln!("{line}");
                    }
                    let _ = std::io::stdout().flush();
                    std::process::exit(code);
                })
                .detach();
            }
            let _ = out.flush();
        });
    });
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
            parse(args(&["--smoke", "--vf-demo", "--print-window-id"])),
            Ok(Flags {
                smoke: true,
                vf_demo: true,
                print_window_id: true
            })
        );
        assert!(parse(args(&["--nope"])).is_err());
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
