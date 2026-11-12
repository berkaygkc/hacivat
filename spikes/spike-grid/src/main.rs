//! B1 grid spike: 10,000,000 rows x 50 columns, virtualized, with a fling bench.
//!
//! `spike-grid` runs interactively (trackpad and wheel, including momentum events).
//! `spike-grid --bench fling` drives a synthetic fling, prints `FrameStats` and writes
//! `target/bench/b1-fling-<os>-<ts>.json`.

mod bench;
mod element;
mod scroll;
mod source;

use std::cell::RefCell;
use std::process::ExitCode;
use std::rc::Rc;

use hacivat_ui_core::app::{open_window, run_app};
use hacivat_ui_core::gpui_surface::{
    AppContext, Context, InteractiveElement, IntoElement, ParentElement, Render, ScrollWheelEvent,
    Styled, Window, div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};

use element::{GridState, grid_canvas};
use scroll::{ScrollPos, scroll_by, viewport_rows};
use source::SyntheticSource;

const ROWS: u64 = 10_000_000;
const COLS: usize = 50;
/// DESIGN.md reference window.
const WINDOW: (f32, f32) = (1440.0, 900.0);

pub struct GridView {
    pub pos: ScrollPos,
    scroll_x: f32,
    pub state: Rc<RefCell<GridState>>,
}

impl GridView {
    fn new(tokens: &'static hacivat_ui_core::theme::ThemeTokens, start_row: u64) -> Self {
        let source = Rc::new(SyntheticSource::new(ROWS, COLS));
        Self {
            pos: ScrollPos {
                top_row: start_row,
                offset_px: 0.0,
            },
            scroll_x: 0.0,
            state: Rc::new(RefCell::new(GridState::new(source, tokens))),
        }
    }

    /// Scrolls by `dy` (positive moves towards later rows) and `dx` (towards later columns).
    pub fn scroll_px(&mut self, dy: f64, dx: f32) {
        let state = self.state.borrow();
        let (body_w, body_h) = state.body_size;
        let row_h = hacivat_ui_core::theme::DARK.spacing.grid_row_height;
        self.pos = scroll_by(
            self.pos,
            dy,
            row_h,
            state.row_count(),
            viewport_rows(body_h, row_h),
        );
        let max_x = (state.content_width() - body_w).max(0.0);
        self.scroll_x = (self.scroll_x + dx).clamp(0.0, max_x);
    }
}

impl Render for GridView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.state.borrow_mut().render_started = Some(std::time::Instant::now());
        let tokens = Theme::get(cx).tokens;
        let row_h = tokens.spacing.grid_row_height;
        div()
            .size_full()
            .bg(tokens.colors.ground.to_hsla())
            .on_scroll(cx.listener(move |this, e: &ScrollWheelEvent, _, cx| {
                // GPUI deltas are positive when content should move down (towards row 0).
                let d = e.delta.pixel_delta(px(row_h));
                this.scroll_px(-f64::from(f32::from(d.y)), -f32::from(d.x));
                cx.notify();
            }))
            .child(grid_canvas(
                self.state.clone(),
                self.pos,
                self.scroll_x,
                tokens,
            ))
    }
}

enum Mode {
    Interactive,
    BenchFling,
}

fn parse_args() -> Result<Mode, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => Ok(Mode::Interactive),
        ["--bench", "fling"] => Ok(Mode::BenchFling),
        other => Err(format!(
            "unknown arguments {other:?}; usage: spike-grid [--bench fling]"
        )),
    }
}

fn main() -> ExitCode {
    let mode = match parse_args() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    run_app(move |cx| {
        Theme::install(cx, ThemeMode::Dark);
        let tokens = Theme::get(cx).tokens;
        // The bench starts mid-table, where an absolute f32 pixel offset (~105M px) would
        // already have lost sub-pixel precision; the row-anchored model must not care.
        let start_row = match mode {
            Mode::Interactive => 0,
            Mode::BenchFling => ROWS / 2,
        };
        let handle = match open_window(cx, "Hacivat B1 grid spike", WINDOW, |_, cx| {
            cx.new(|_| GridView::new(tokens, start_row))
        }) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("failed to open window: {e:#}");
                cx.quit();
                return;
            }
        };
        cx.activate(true);
        if let Mode::BenchFling = mode {
            let _ = handle.update(cx, |_, window, cx| bench::start(cx.entity(), window));
        }
    });
    ExitCode::SUCCESS
}
