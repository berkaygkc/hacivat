//! `spike-harman [--theme dark|light] [--lang en|tr] [--scale 1|1.5|2] [--print-window-id]
//!               [--check-update <manifest-url>]`
//! Reproduces the approved harman workspace (1440 × 900 logical) for visual-fidelity checks.

mod screen;
mod update;

use std::io::Write as _;

use anyhow::{Result, bail};
use hacivat_ui_core::fonts::font_for;
use hacivat_ui_core::gpui_surface::*;
use hacivat_ui_core::app::{Boot, run_app};
use hacivat_ui_core::shell::{display_sizes, native_window_id, open_unified_window};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use screen::data::Lang;
use screen::ui::Ui;

struct Args {
    mode: ThemeMode,
    lang: Lang,
    scale: f32,
    print_window_id: bool,
    display: Option<usize>,
    check_update: Option<String>,
}

fn parse_args() -> Result<Args> {
    let mut a = Args { mode: ThemeMode::Dark, lang: Lang::En, scale: 2.0, print_window_id: false, display: None, check_update: None };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        match k.as_str() {
            "--theme" => {
                a.mode = match it.next().as_deref() {
                    Some("dark") => ThemeMode::Dark,
                    Some("light") => ThemeMode::Light,
                    other => bail!("--theme dark|light, got {other:?}"),
                }
            }
            "--lang" => {
                a.lang = match it.next().as_deref() {
                    Some("en") => Lang::En,
                    Some("tr") => Lang::Tr,
                    other => bail!("--lang en|tr, got {other:?}"),
                }
            }
            "--scale" => a.scale = it.next().unwrap_or_default().parse()?,
            "--print-window-id" => a.print_window_id = true,
            "--display" => a.display = Some(it.next().unwrap_or_default().parse()?),
            "--check-update" => a.check_update = Some(it.next().unwrap_or_default()),
            other => bail!("unknown flag {other}"),
        }
    }
    Ok(a)
}

/// The window title (also how `design/vf/capture.sh` finds the window on Linux).
fn window_title() -> String {
    format!("Hacivat Spike {}", update::VERSION)
}

/// Printed with `--print-window-id` so capture scripts can find the window by title.
fn window_title_line() -> String {
    format!("WINDOW_TITLE={}", window_title())
}

struct Harman {
    lang: Lang,
    scale: f32,
    print_window_id: bool,
    reported: bool,
}

impl Render for Harman {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = Ui::new(cx, self.lang);
        if !self.reported {
            self.reported = true;
            let actual = window.scale_factor();
            if (actual - self.scale).abs() > 0.01 {
                eprintln!(
                    "spike-harman: requested scale {} but this display renders at {actual}; \
                     capture at that scale needs a matching display (Windows 150% host for 1.5x)",
                    self.scale
                );
                std::process::exit(4);
            }
            if self.print_window_id {
                match native_window_id(window) {
                    Some(id) => println!("WINDOW_ID={id}"),
                    None => println!("WINDOW_ID=unavailable"),
                }
                let _ = std::io::stdout().flush();
            }
        }
        let mono = font_for(&ui.mono(12.5));
        let ts = window.text_system();
        let advance = ts
            .advance(ts.resolve_font(&mono), px(12.5), 'm')
            .map(|s| f32::from(s.width))
            .unwrap_or(7.5);
        screen::workspace(&ui, advance)
    }
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("spike-harman: {e:#}");
            std::process::exit(2);
        }
    };
    if let Some(url) = &args.check_update {
        update::check_and_install(url);
    }
    // run_app registers and verifies the bundled fonts (exit 3 on failure) and installs the icons.
    run_app(Boot::default(), move |cx| {
        Theme::install(cx, args.mode);
        let (lang, scale, print_window_id) = (args.lang, args.scale, args.print_window_id);
        // Native traffic lights replace the mock's drawn ones.
        let lights = (14.0, 13.0);
        // Default display: the first one small enough to be a laptop panel (a Retina display
        // reports 1.7k-2.1k logical px); GPUI has no per-display scale query. Override with --display.
        let sizes = display_sizes(cx);
        let display = args.display.or_else(|| sizes.iter().position(|s| s.0 <= 2200.0 && s.0 >= 1440.0));
        eprintln!("spike-harman: displays {sizes:?}, using {display:?}");
        if print_window_id {
            println!("{}", window_title_line());
        }
        let opened = open_unified_window(cx, &window_title(), screen::WINDOW, lights, display, move |_, cx| {
            cx.new(|_| Harman { lang, scale, print_window_id, reported: false })
        });
        if let Err(e) = opened {
            eprintln!("spike-harman: cannot open window: {e:#}");
            std::process::exit(1);
        }
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_title_line_names_the_window_capture_sh_looks_for() {
        assert_eq!(window_title(), format!("Hacivat Spike {}", update::VERSION));
        assert_eq!(window_title_line(), format!("WINDOW_TITLE={}", window_title()));
    }
}
