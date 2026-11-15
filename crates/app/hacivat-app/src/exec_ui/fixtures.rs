// ## m3a-t35-shots: the run UI's shots fixtures (feature `shots`; routed from Task 33, ledger I7).
//! Shots fixtures for NOTES surface 5 (the gutter's execution marks): a console's marks in
//! given unit states, with the labels the run UI gives them (`marks::mark_of`, `short_of`), so
//! a shot shows every state without a server that can be slow, cancelled or failing on cue.
//!
//! The close prompt (surface 13) and the chooser (surface 4) need no fixture: the shots reach
//! them through the product's own guard and `editor.run` on a live console.
//!
//! **One difference from a live run, for stable PNGs:** a running mark's clock is the fixed
//! duration given here, where the live mark reads the executor clock on every repaint.

use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use hacivat_exec::UnitStatus;
use hacivat_l10n::ArgValue;
use hacivat_session::session::RunId;
use hacivat_ui::text::TextFn;
use hacivat_ui_core::gpui_surface::App;

use crate::console::{ConsoleId, Consoles};
use crate::exec_ui::marks::{duration_text, mark_key, message};

/// The run every fixture mark belongs to (far from the runs a console makes).
const FIXTURE_RUN: RunId = RunId(0xF_FFFF);

/// Gives console `console`'s editor one mark per `(range, status, elapsed)`, as the run UI
/// sets them for a run whose unit `i` is in `status` after `elapsed`. Ranges are byte ranges
/// of the editor's current text. Returns how many marks were placed.
pub fn show_marks(
    console: ConsoleId,
    units: Vec<(Range<usize>, UnitStatus, Duration)>,
    cx: &mut App,
) -> usize {
    let Some(editor) = Consoles::get(cx, console).and_then(|s| s.editor.clone()) else {
        return 0;
    };
    let now = cx.background_executor().now();
    let version = editor.read(cx).snapshot().version;
    let keys: Vec<_> = (0..units.len()).map(|i| mark_key(FIXTURE_RUN, i)).collect();
    editor.update(cx, |e, cx| {
        e.add_exec_marks(
            version,
            keys.iter()
                .zip(&units)
                .map(|(k, (r, _, _))| (*k, r.clone()))
                .collect(),
            cx,
        );
    });
    for (i, ((_, status, elapsed), key)) in units.into_iter().zip(keys).enumerate() {
        let n = u32::try_from(i + 1).unwrap_or(u32::MAX);
        let started = now.checked_sub(elapsed).unwrap_or(now);
        let (state, mut label, mut eol) =
            crate::exec_ui::marks::mark_of(&status, n, started, elapsed, false);
        let mut short = crate::exec_ui::marks::short_of(&status, started, elapsed, false);
        if matches!(
            status,
            UnitStatus::Running { .. } | UnitStatus::Fetching { .. }
        ) {
            let ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
            let fixed = |id: &'static str| -> TextFn {
                Rc::new(move |cx: &App| {
                    let d = duration_text(cx, ms);
                    message(cx, id, vec![("duration", ArgValue::Text(d))])
                })
            };
            eol = fixed("hv-editor-mark-running");
            label = eol.clone();
            short = Some(fixed("hv-editor-mark-short-running"));
        }
        editor.update(cx, |e, cx| {
            e.set_exec_state(key, state, Some(label), Some(eol), cx);
            e.set_exec_short(key, short, cx);
        });
    }
    editor.read(cx).marks().len()
}
