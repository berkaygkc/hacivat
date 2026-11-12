//! B3 editor spike: a 50,000-line SQL editor with typing and completion latency benches.
//!
//! `spike-editor --file <path.sql> [--bench typing|typing-long-line|completion]`

mod bench;
mod buffer;
mod completion;
mod element;
mod input;

use std::path::PathBuf;
use std::process::ExitCode;

use hacivat_ui_core::app::{open_window, run_app};
use hacivat_ui_core::gpui_surface::AppContext;
use hacivat_ui_core::theme::{Theme, ThemeMode};

use crate::bench::BenchMode;
use crate::buffer::Buffer;
use crate::element::EditorView;

struct Args {
    file: PathBuf,
    bench: Option<BenchMode>,
}

fn parse_args() -> Result<Args, String> {
    let (mut file, mut bench) = (None, None);
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--file" => file = Some(PathBuf::from(it.next().ok_or("--file needs a path")?)),
            "--bench" => {
                let mode = it.next().ok_or("--bench needs a mode")?;
                bench = Some(BenchMode::parse(&mode).ok_or(format!(
                    "unknown bench {mode:?} (typing, typing-long-line, completion)"
                ))?);
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(Args {
        file: file.ok_or("missing --file <path.sql>")?,
        bench,
    })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("spike-editor: {e}");
            return ExitCode::from(2);
        }
    };
    let text = match std::fs::read_to_string(&args.file) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("spike-editor: {}: {e}", args.file.display());
            return ExitCode::FAILURE;
        }
    };
    let buffer = Buffer::from(text.as_str());
    drop(text);
    let bench_mode = args.bench;

    run_app(move |cx| {
        Theme::install(cx, ThemeMode::Dark);
        let window = open_window(cx, "Hacivat B3 editor", (1280., 800.), |window, cx| {
            cx.new(|cx| EditorView::new(buffer, window, cx))
        });
        let started = window.and_then(|window| {
            window.update(cx, |view, window, cx| {
                window.focus(&view.focus_handle, cx);
                match bench_mode {
                    Some(mode) => bench::start(mode, view, window, cx),
                    None => Ok(()),
                }
            })?
        });
        if let Err(e) = started {
            eprintln!("spike-editor: {e:#}");
            cx.quit();
        }
    });
    ExitCode::SUCCESS
}
