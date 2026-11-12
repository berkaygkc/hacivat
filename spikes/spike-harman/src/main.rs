//! `spike-harman [--theme dark|light] [--lang en|tr] [--scale 1|1.5|2] [--print-window-id]`
//! Reproduces the approved harman workspace (1440 × 900 logical) for visual-fidelity checks.

mod screen;

use std::io::Write as _;

use anyhow::{Result, bail};
use hacivat_ui_core::fonts::font_for;
use hacivat_ui_core::gpui_surface::*;
use hacivat_ui_core::shell::{
    display_sizes, native_window_id, open_unified_window, run_app_with_assets, verify_bundled_fonts,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use screen::data::Lang;
use screen::ui::Ui;

struct Args {
    mode: ThemeMode,
    lang: Lang,
    scale: f32,
    print_window_id: bool,
    display: Option<usize>,
}

fn parse_args() -> Result<Args> {
    let mut a = Args { mode: ThemeMode::Dark, lang: Lang::En, scale: 2.0, print_window_id: false, display: None };
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
            other => bail!("unknown flag {other}"),
        }
    }
    Ok(a)
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
    run_app_with_assets(move |cx| {
        if let Err(e) = verify_bundled_fonts(cx) {
            eprintln!("spike-harman: bundled fonts did not resolve: {e:#}");
            std::process::exit(3);
        }
        Theme::install(cx, args.mode);
        let (lang, scale, print_window_id) = (args.lang, args.scale, args.print_window_id);
        // Native traffic lights replace the mock's drawn ones.
        let lights = (14.0, 13.0);
        // Default display: the first one small enough to be a laptop panel (a Retina display
        // reports 1.7k-2.1k logical px); GPUI has no per-display scale query. Override with --display.
        let sizes = display_sizes(cx);
        let display = args.display.or_else(|| sizes.iter().position(|s| s.0 <= 2200.0 && s.0 >= 1440.0));
        eprintln!("spike-harman: displays {sizes:?}, using {display:?}");
        let opened = open_unified_window(cx, "Hacivat", screen::WINDOW, lights, display, move |_, cx| {
            cx.new(|_| Harman { lang, scale, print_window_id, reported: false })
        });
        if let Err(e) = opened {
            eprintln!("spike-harman: cannot open window: {e:#}");
            std::process::exit(1);
        }
        cx.activate(true);
    });
}
