//! The UI port of a connect: password prompts from the core runtime, answered on the UI thread
//! through `tokio::sync` channels (Task 20).

use hacivat_session::pipeline::Interaction;
use hacivat_ui_core::gpui_surface::App;

/// How connects started from the UI may ask the user (Task 20). The stub is `Headless`.
pub fn interaction(_cx: &App) -> Interaction {
    Interaction::Headless
}
