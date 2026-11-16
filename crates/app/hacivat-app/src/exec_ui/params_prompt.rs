//! The parameter prompt (SQL-PARAM-01; M3b Task 24, `design/m3b-safe` surface 5): one field per
//! named parameter, machine format first (a Turkish `1.234` flagged as ambiguous).
//!
//! ## m3b-t01b-seams: the seam (Task 1b). STUB: [`open`] draws nothing and answers `None`
//! (the run is cancelled: nothing is sent). Task 24 fills it.

use std::sync::Arc;

use tokio::sync::oneshot;

use hacivat_ui_core::gpui_surface::App;
use hacivat_values::BindValue;

use super::answered;
use hacivat_exec::ParamPrompt;

/// The values the user gave, by parameter name; `None` when the prompt was cancelled.
pub type ParamValues = Option<Vec<(Arc<str>, BindValue)>>;

/// Asks for `prompts`. STUB: answers `None` (cancelled) at once.
pub fn open(prompts: Vec<ParamPrompt>, cx: &mut App) -> oneshot::Receiver<ParamValues> {
    let _ = (prompts, cx);
    answered(None)
}
